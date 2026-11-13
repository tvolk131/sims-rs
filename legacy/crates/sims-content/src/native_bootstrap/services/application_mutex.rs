//! Issuing-heap/source-image binding around the explicit shared authority.
use super::*;
use sims_sim::native_application::{Construction, mutex};

struct IssuedWrapper {
    heap: Identity,
    application: u64,
    version: platform::VersionReturn,
}
#[derive(Default)]
pub(super) struct NamedObjects {
    client: Option<sims_kernel::Client>,
    wrappers: BTreeMap<u64, IssuedWrapper>,
}
pub(super) struct Binding {
    pub(super) heap: Identity,
    pub(super) request: platform::MutexRequest,
    pending: mutex::Pending,
    pub(super) returned: Option<mutex::Return>,
    pub(super) application_image: Owner,
    pub(super) wrapper_image: Owner,
}

impl BootstrapServices<'_> {
    /// Inject an explicitly authenticated authority once. No default endpoint,
    /// service discovery, reconnection or successful local fallback exists.
    pub fn configure_named_object_authority(
        &mut self,
        client: sims_kernel::Client,
    ) -> Result<(), Stop> {
        if self.named_objects.client.is_some() {
            return Err(unsupported("named-object authority is already configured"));
        }
        self.named_objects.client = Some(client);
        Ok(())
    }
    pub(super) fn require_mutex_owner(
        &self,
        owner: &Owner,
        kind: Kind,
        bytes: usize,
    ) -> Result<(), Stop> {
        if owner.data.len() != bytes
            || self.memory.allocation_kind(owner.token)? != kind
            || self.memory.allocation_bytes(owner.token)?.len() != bytes
        {
            return Err(unsupported(
                "mutex owner has another allocation role or extent",
            ));
        }
        Ok(())
    }
    pub(super) fn allocate_application_mutex_wrapper(
        &mut self,
        request: &platform::WrapperRequest,
    ) -> Result<Allocated, Stop> {
        request.validate()?;
        if self.memory.allocation_kind(request.application())? != Kind::Owner
            || self.memory.allocation_bytes(request.application())?.len() != 324
        {
            return Err(unsupported(
                "mutex wrapper has another containing application extent",
            ));
        }
        if self.named_objects.wrappers.len() >= MAX_CAPABILITIES as usize {
            return Err(Stop::Unavailable(
                "mutex wrapper issuance ledger is exhausted".into(),
            ));
        }
        let allocated = self.memory.allocate(Kind::ApplicationMutexWrapper, 12)?;
        if let Allocated::Bytes { token, .. } = &allocated {
            self.named_objects.wrappers.insert(
                *token,
                IssuedWrapper {
                    heap: self.memory.identity(),
                    application: request.application(),
                    version: request.version.clone(),
                },
            );
        }
        Ok(allocated)
    }
    fn admit_mutex_request(&self, request: &platform::MutexRequest) -> Result<(), Stop> {
        request.validate()?;
        self.require_mutex_owner(request.application_image(), Kind::Owner, 324)?;
        self.require_mutex_owner(request.wrapper_image(), Kind::ApplicationMutexWrapper, 12)?;
        let issued = self
            .named_objects
            .wrappers
            .get(&request.wrapper())
            .ok_or_else(|| unsupported("mutex wrapper was not issued by these services"))?;
        if !self.memory.has_identity(&issued.heap)
            || issued.application != request.application_image().token
            || issued.version != *request.version()
        {
            return Err(unsupported(
                "mutex wrapper belongs to another heap or source invocation",
            ));
        }
        // The recovered caller ignores HRESULT. Authenticate the issued COM
        // history, without requiring that a failed return has a live apartment.
        if !self.capabilities.values().any(|cap| matches!(cap, Capability::Apartment { heap, application, returned } if self.memory.has_identity(heap) && *application == request.application_image().token && returned == request.com_return())) {
            return Err(unsupported("mutex request lost its issuing COM return or heap"));
        }
        Ok(())
    }
    pub(super) fn create_application_mutex(
        &mut self,
        request: &platform::MutexRequest,
    ) -> Result<mutex::Return, Stop> {
        self.admit_mutex_request(request)?;
        let existing = self.capabilities.iter().find_map(|(token, cap)| match cap {
            Capability::NamedMutex(binding) if binding.request.wrapper() == request.wrapper() => {
                Some(*token)
            }
            _ => None,
        });
        let token = if let Some(token) = existing {
            let Capability::NamedMutex(binding) = &self.capabilities[&token] else {
                unreachable!()
            };
            if !self.memory.has_identity(&binding.heap) || binding.request != *request {
                return Err(unsupported("pending mutex belongs to another invocation"));
            }
            token
        } else {
            let token = self.next_capability()?;
            let client = self.named_objects.client.as_ref().ok_or_else(|| {
                Stop::Unavailable("no named-object authority was explicitly configured".into())
            })?;
            let pending = mutex::Pending::begin(request, client, token)?; // reserves; no create sent
            // Both complete extents and issuing identities were checked before
            // either copy. Only sealed cursor images can reach this operation.
            self.memory.synchronize_owner(request.application_image())?;
            self.memory.synchronize_owner(request.wrapper_image())?;
            self.commit(
                token,
                Capability::NamedMutex(Binding {
                    heap: self.memory.identity(),
                    request: request.clone(),
                    pending,
                    returned: None,
                    application_image: request.application_image().clone(),
                    wrapper_image: request.wrapper_image().clone(),
                }),
            );
            token
        };
        let Some(Capability::NamedMutex(binding)) = self.capabilities.get_mut(&token) else {
            unreachable!()
        };
        let returned = binding.pending.resolve()?;
        binding.returned = Some(returned.clone()); // retain before validation
        returned.validate(request)?;
        Ok(returned)
    }
    /// Validate the current source cursor and the live issuing heap before
    /// synchronizing its post-return stores and asking the actual authority.
    /// Historical cursor replay never calls this method or sends IPC.
    pub fn named_mutex(&mut self, construction: &Construction) -> Result<&mutex::Return, Stop> {
        let token = self.admit_application_mutex_current(construction)?;
        let Some(Capability::NamedMutex(binding)) = self.capabilities.get(&token) else {
            unreachable!()
        };
        let held = binding.returned.as_ref().unwrap();
        held.check_live()?;
        Ok(held)
    }
}
