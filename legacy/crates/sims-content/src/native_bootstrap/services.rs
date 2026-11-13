use super::{
    files::{BoundFile, FileRole, FileView, MountedFiles},
    locks::Section,
    memory::{Identity, Kind, Memory, Purpose, StringScope},
};
use crate::{Index, avatar_catalog::Directories};
use sims_sim::{
    native_application::{com, fpu, platform},
    native_game, native_matrix, native_world,
    native_world::resources::{file, handle, manager, names, opening, path, population, strings},
    native_world_children::{self, Owner, vector},
    saved_matrix::{self, Allocated, AllocationRequest, Lookup, Stop},
    saved_world,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const CAP_START: u32 = 0x0800_0000;
const MAX_CAPABILITIES: u32 = 4096;
mod application_fpu;
mod application_fpu_images;
mod application_mutex;
mod program_bindings;
pub mod program_modules;
pub mod program_message_box;
pub mod program_ole32;
pub mod program_registry;
pub mod program_registry_store;
pub mod program_raster;
pub mod program_raster_context;
pub mod program_winmm;
pub use program_bindings::ProgramAccess;
pub use program_modules::Termination as ProgramTermination;
mod system_base72;
mod system_collection;
mod system_collection_event;
mod system_factory;
pub use system_factory::{CollectionHost as SystemCollectionHost, Host as SystemProviderHost};
pub use system_collection_event::Host as SystemCollectionEventHost;
#[cfg(test)]
#[path = "services/system_base72/tests.rs"]
mod system_base72_tests;

/// Explicit Rust locale backend. This policy is not evidence of an original
/// process's locale or encoding-cache initialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalePolicy {
    C,
}

enum Capability {
    File(BoundFile),
    Section(Section),
    Apartment {
        heap: Identity,
        application: u64,
        returned: com::Return,
    },
    NamedMutex(application_mutex::Binding),
    FpuRead(application_fpu::ReadBinding),
    FpuLoad(application_fpu::LoadBinding),
    SystemBase72(system_base72::Binding),
    SystemFactory(system_factory::Binding),
    SystemCollection(system_collection::Binding),
    SystemCollectionSection(system_collection::SectionBinding),
    SystemCollectionEvent(system_collection_event::Binding),
    ProgramModule(program_modules::ModuleReference),
    ProgramImport(program_modules::ProcedureLease),
    ProgramProcedure(program_modules::ProcedureObservation),
    ProgramRegistryKey(program_registry::KeyLease),
    ProgramRasterContext(program_raster_context::ContextLease),
}

/// Concrete host services, retained with their World and loaded-file owner.
/// No Clone/Deserialize or standalone produce_world(Index) convenience exists.
pub struct BootstrapServices<'a> {
    memory: Memory,
    mounted: MountedFiles<'a>,
    capabilities: BTreeMap<u32, Capability>,
    next: u32,
    locale: LocalePolicy,
    application_version: Option<platform::VersionPolicy>,
    named_objects: application_mutex::NamedObjects,
    application_fpu: application_fpu::Environment,
    program: Option<program_bindings::Program>,
}

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}
fn scope(value: path::Scope) -> StringScope {
    match value {
        path::Scope::World => StringScope::World,
        path::Scope::Path => StringScope::Path,
    }
}
fn string_purpose(value: strings::Request) -> Purpose {
    match value {
        strings::Request::Construct(_) => Purpose::Construct,
        strings::Request::Grow(_) => Purpose::Grow,
    }
}
fn wide_purpose(value: opening::Request) -> Purpose {
    match value {
        opening::Request::Construct(_) => Purpose::Construct,
        opening::Request::Grow(_) => Purpose::Grow,
    }
}

impl<'a> BootstrapServices<'a> {
    pub fn new(
        memory: Memory,
        index: &'a Index,
        directories: &'a Directories,
        locale: LocalePolicy,
    ) -> Self {
        Self {
            memory,
            mounted: MountedFiles::new(index, directories),
            capabilities: BTreeMap::new(),
            next: CAP_START,
            locale,
            application_version: None,
            named_objects: Default::default(),
            application_fpu: Default::default(),
            program: None,
        }
    }
    pub fn memory(&self) -> &Memory {
        &self.memory
    }
    /// Explicit Rust-port input; no silent Windows version default or native
    /// GetVersion observation is supplied by this host.
    pub fn with_application_version_policy(mut self, policy: platform::VersionPolicy) -> Self {
        self.application_version = Some(policy);
        self
    }
    pub fn memory_mut(&mut self) -> &mut Memory {
        &mut self.memory
    }
    pub fn system_provider_host(&mut self, request: &sims_sim::native_system::factory::Request) -> SystemProviderHost<'_, 'a> {
        system_factory::Host { services: self, request: request.clone() }
    }
    pub fn system_collection_host(&mut self, request: &sims_sim::native_system::collection::Request) -> SystemCollectionHost<'_, 'a> {
        system_factory::CollectionHost { services: self, request: request.clone() }
    }
    pub fn system_collection_event_host(&mut self, request: &sims_sim::native_system::collection::Request) -> SystemCollectionEventHost<'_, 'a> {
        system_collection_event::Host { services:self, request:request.clone() }
    }
    /// Borrow the actual anonymous event issued for this exact source return.
    /// No integer/native-handle adoption or event-state polling is performed.
    pub fn collection_event(&self, returned: &sims_sim::native_system::collection::event::Return) -> Result<&sims_kernel::event::Event, Stop> {
        system_collection_event::access(self,returned)
    }
    pub fn live_capabilities(&self) -> usize {
        self.capabilities.len()
    }

    fn next_capability(&self) -> Result<u32, Stop> {
        if self
            .next
            .checked_sub(CAP_START)
            .is_none_or(|n| n >= MAX_CAPABILITIES)
            || self.next == u32::MAX
            || self.capabilities.contains_key(&self.next)
        {
            return Err(Stop::Unavailable(
                "bootstrap capability profile exhausted".into(),
            ));
        }
        Ok(self.next)
    }

    fn commit(&mut self, token: u32, value: Capability) {
        self.capabilities.insert(token, value);
        self.next += 1;
    }

    fn open(&mut self, filename: &[u8], role: FileRole) -> Result<Option<u32>, Stop> {
        let token = self.next_capability()?;
        let Some(file) = self.mounted.resolve(filename, role)? else {
            return Ok(None);
        };
        // All fallible work precedes publication. There is no Result::Err after
        // an issued capability, allocation or release has been committed.
        self.commit(token, Capability::File(file));
        Ok(Some(token))
    }

    pub fn file(&self, token: u32) -> Result<FileView<'_>, Stop> {
        match self.capabilities.get(&token) {
            Some(Capability::File(file)) => Ok(FileView {
                role: file.role,
                identity: &file.identity,
                bytes: &file.bytes,
                position: file.position,
            }),
            Some(_) => Err(unsupported("bootstrap capability is not a file")),
            None => Err(Stop::Unavailable(
                "bootstrap file capability is not live".into(),
            )),
        }
    }

    fn resource_file(&self, token: u32) -> Result<&BoundFile, Stop> {
        match self.capabilities.get(&token) {
            Some(Capability::File(file)) if file.role == FileRole::ResourceReadonly => Ok(file),
            Some(_) => Err(unsupported(
                "bootstrap capability is not a readonly resource file",
            )),
            None => Err(Stop::Unavailable(
                "bootstrap resource capability is not live".into(),
            )),
        }
    }

    /// Rust byte-stream read, separate from any unmodeled native ReadFile call.
    pub fn read(&mut self, token: u32, output: &mut [u8]) -> Result<usize, Stop> {
        self.resource_file(token)?;
        let Some(Capability::File(file)) = self.capabilities.get_mut(&token) else {
            unreachable!()
        };
        let count = output.len().min(file.bytes.len() - file.position);
        output[..count].copy_from_slice(&file.bytes[file.position..file.position + count]);
        file.position += count;
        Ok(count)
    }

    fn section_entry(&self, token: u64, address: u64, bytes: u32) -> Result<&Section, Stop> {
        let token = u32::try_from(token)
            .map_err(|_| unsupported("section capability exceeds native width"))?;
        match self.capabilities.get(&token) {
            Some(Capability::Section(section))
                if section.address == address && section.bytes == bytes =>
            {
                if let Some((heap, parent, bytes)) = &section.parent {
                    if !self.memory.has_identity(heap) {
                        return Err(unsupported("application section belongs to another heap"));
                    }
                    self.memory.require_owner_view(*parent, *bytes)?;
                    if self.memory.allocation_bytes(*parent)?.len() != *bytes as usize {
                        return Err(unsupported(
                            "application section has another containing owner extent",
                        ));
                    }
                }
                if let Some(owner) = &section.debug_info {
                    if self.memory.allocation_kind(owner.token)? != Kind::ApplicationSectionDebug
                        || self.memory.allocation_bytes(owner.token)? != owner.data
                    {
                        return Err(unsupported(
                            "section debug storage differs from its retained initialization image",
                        ));
                    }
                }
                Ok(section)
            }
            Some(_) => Err(unsupported(
                "section capability has another role or owner extent",
            )),
            None => Err(Stop::Unavailable("section capability is not live".into())),
        }
    }

    pub fn section(&self, token: u64, address: u64, bytes: u32) -> Result<&Arc<Mutex<()>>, Stop> {
        if let Ok(key)=u32::try_from(token) {
            if let Some(Capability::SystemCollectionSection(binding))=self.capabilities.get(&key) {
                return binding.section(self,address,bytes);
            }
            if let Some(Capability::SystemBase72(binding))=self.capabilities.get(&key) {
                return binding.section(&self.memory,address,bytes);
            }
        }
        Ok(&self.section_entry(token, address, bytes)?.lock)
    }

    pub fn section_debug_info(
        &self,
        token: u64,
        address: u64,
        bytes: u32,
    ) -> Result<Option<&Owner>, Stop> {
        if let Ok(key)=u32::try_from(token) {
            if let Some(Capability::SystemCollectionSection(binding))=self.capabilities.get(&key) {
                return binding.debug_info(self,address,bytes);
            }
            if let Some(Capability::SystemBase72(binding))=self.capabilities.get(&key) {
                return binding.debug_info(&self.memory,address,bytes);
            }
        }
        Ok(self
            .section_entry(token, address, bytes)?
            .debug_info
            .as_ref())
    }

    /// Access only an apartment return issued by these retained services and
    /// still attached to its live containing application. This is the explicit
    /// Rust-port backend, without native COM interoperability/class activation.
    pub fn apartment(&self, returned: &com::Return) -> Result<com::ApartmentAccess<'_>, Stop> {
        self.apartment_return(returned)?.current_apartment()
    }

    /// Select the actual retained Return by shared issuance identity. Numeric
    /// application/thread/apartment observations cannot stand in for it.
    fn apartment_return(&self, returned: &com::Return) -> Result<&com::Return, Stop> {
        for capability in self.capabilities.values() {
            if let Capability::Apartment {
                heap,
                application,
                returned: held,
            } = capability
            {
                if held == returned {
                    if !self.memory.has_identity(heap) || held.application() != *application {
                        return Err(unsupported("apartment belongs to another application heap"));
                    }
                    self.memory.require_owner_view(*application, 324)?;
                    if self.memory.allocation_kind(*application)? != Kind::Owner
                        || self.memory.allocation_bytes(*application)?.len() != 324
                    {
                        return Err(unsupported(
                            "apartment has another containing application extent",
                        ));
                    }
                    held.current_apartment()?.check()?;
                    return Ok(held);
                }
            }
        }
        Err(unsupported(
            "apartment return was not issued by these services",
        ))
    }

    fn allocate_string(
        &mut self,
        selected: path::Scope,
        request: strings::Request,
    ) -> Result<Allocated, Stop> {
        self.memory.allocate(
            Kind::ScopedString {
                scope: scope(selected),
                purpose: string_purpose(request),
            },
            request.bytes(),
        )
    }

    fn string_kind(&self, token: u64, selected: path::Scope) -> Result<Kind, Stop> {
        let kind = self.memory.allocation_kind(token)?;
        if !matches!(kind, Kind::ScopedString { scope: actual, .. } if actual == scope(selected)) {
            return Err(unsupported(
                "bootstrap string release has a different scope or allocation role",
            ));
        }
        Ok(kind)
    }

    fn free_string(&mut self, selected: path::Scope, owner: &Owner) -> Result<(), Stop> {
        let kind = self.string_kind(owner.token, selected)?;
        self.memory.release_owned(owner, kind)
    }

    pub fn scoped_strings(&mut self, selected: path::Scope) -> ScopedStrings<'_, 'a> {
        ScopedStrings {
            services: self,
            selected,
        }
    }

    pub fn application_host(&mut self) -> ApplicationHost<'_, 'a> {
        ApplicationHost { services: self }
    }
}

/// The recovered initializer writes its complete logical32-bit section and
/// debug-record images into fresh storage. The retained Rust lock is a backend
/// resource; initialization does not supply native enter/leave or process epochs.
pub struct ApplicationHost<'s, 'a> {
    services: &'s mut BootstrapServices<'a>,
}
impl strings::Host for ApplicationHost<'_, '_> {
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        self.services.allocate_string(path::Scope::Path, request)
    }
    fn free_string(&mut self, token: u64) -> Result<(), Stop> {
        let kind = self.services.string_kind(token, path::Scope::Path)?;
        self.services.memory.release(token, kind)
    }
}
impl sims_sim::native_application::Host for ApplicationHost<'_, '_> {
    fn read_control(&mut self, request: &fpu::ReadRequest) -> Result<fpu::ReadReturn, Stop> {
        self.services.read_application_control(request)
    }
    fn load_control(&mut self, request: &fpu::LoadRequest) -> Result<fpu::LoadReturn, Stop> {
        self.services.load_application_control(request)
    }
    fn query_version(
        &mut self,
        request: &platform::VersionRequest,
    ) -> Result<platform::VersionReturn, Stop> {
        request.validate()?;
        self.services
            .memory
            .require_owner_view(request.application, 324)?;
        if self
            .services
            .memory
            .allocation_bytes(request.application)?
            .len()
            != 324
        {
            return Err(unsupported(
                "version query has another containing application extent",
            ));
        }
        self.services
            .application_version
            .ok_or_else(|| {
                Stop::Unavailable("application version policy was not explicitly supplied".into())
            })?
            .reply(request)
    }
    fn allocate_mutex_wrapper(
        &mut self,
        request: &platform::WrapperRequest,
    ) -> Result<Allocated, Stop> {
        self.services.allocate_application_mutex_wrapper(request)
    }
    fn create_mutex(
        &mut self,
        request: &platform::MutexRequest,
    ) -> Result<sims_sim::native_application::mutex::Return, Stop> {
        self.services.create_application_mutex(request)
    }
    fn initialize_com(&mut self, request: &com::Request) -> Result<com::Return, Stop> {
        request.validate()?;
        self.services
            .memory
            .require_owner_view(request.application, 324)?;
        if self
            .services
            .memory
            .allocation_bytes(request.application)?
            .len()
            != 324
        {
            return Err(unsupported(
                "CoInitialize has another containing application extent",
            ));
        }
        let token = self.services.next_capability()?;
        let returned = com::initialize_rust_port(request)?;
        // This backend cannot return Err after acquiring the apartment lease.
        // Every direct invocation is a distinct call; the cursor stages its
        // successful return before replay, so no address-based dedup is used.
        self.services.commit(
            token,
            Capability::Apartment {
                heap: self.services.memory.identity(),
                application: request.application,
                returned: returned.clone(),
            },
        );
        Ok(returned)
    }
    fn initialize_section(
        &mut self,
        request: &sims_sim::native_application::SectionRequest,
    ) -> Result<sims_sim::native_application::InitializedSection, Stop> {
        let base = request
            .region
            .token
            .checked_sub(24)
            .ok_or_else(|| unsupported("application section has no containing owner"))?;
        if request.instruction != 0x5d91bc
            || request.import_slot != 0x666114
            || request.region.data.len() != 24
            || self.services.memory.allocation_bytes(base)?.len() != 324
        {
            return Err(unsupported(
                "application section request differs from the recovered caller",
            ));
        }
        self.services.memory.require_owner_view(base, 324)?;
        if self.services.capabilities.values().any(|cap|matches!(cap,Capability::Section(section) if section.address==request.region.token)) {
            return Err(unsupported("application section is already initialized"));
        }
        let token = self.services.next_capability()?;
        // A host quota/allocation error stops before publication. It is not
        // manufactured into the native routine's nullable-heap branch.
        let Allocated::Bytes {
            token: debug_token,
            data,
        } = self
            .services
            .memory
            .allocate(Kind::ApplicationSectionDebug, 32)?
        else {
            unreachable!("this heap never manufactures null allocation replies")
        };
        let debug_input = Owner {
            token: debug_token,
            data,
        };
        // These invariants follow from the checked containing allocation and
        // the same bounded monotonic heap. No Result::Err follows that effect.
        let reply = sims_sim::native_application::InitializedSection::from_debug_info(
            request,
            Some(&debug_input),
            u64::from(token),
        )
        .expect("fresh same-heap32-byte debug owner satisfies initialization extents");
        self.services
            .memory
            .synchronize_owner(reply.debug_info.as_ref().unwrap())
            .expect("the complete fresh debug allocation is still live in this exclusive heap");
        let section = Section::application(&reply, base, self.services.memory.identity());
        self.services.commit(token, Capability::Section(section));
        Ok(reply)
    }
    fn allocate_tree(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        if bytes != 24 {
            return Err(unsupported("application tree request has another extent"));
        }
        self.services.memory.allocate(Kind::ApplicationTree, bytes)
    }
}

pub struct ScopedStrings<'s, 'a> {
    services: &'s mut BootstrapServices<'a>,
    selected: path::Scope,
}

impl strings::Host for ScopedStrings<'_, '_> {
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        self.services.allocate_string(self.selected, request)
    }
    fn free_string(&mut self, token: u64) -> Result<(), Stop> {
        let kind = self.services.string_kind(token, self.selected)?;
        self.services.memory.release(token, kind)
    }
}

impl path::Host for BootstrapServices<'_> {
    fn allocate_string(
        &mut self,
        selected: path::Scope,
        request: strings::Request,
    ) -> Result<Allocated, Stop> {
        self.allocate_string(selected, request)
    }
    fn free_string(&mut self, selected: path::Scope, owner: &Owner) -> Result<(), Stop> {
        self.free_string(selected, owner)
    }
    fn open_scenario(&mut self, filename: &[u8], mode: &[u8]) -> Result<path::Opened, Stop> {
        if mode != b"rb" {
            return Err(unsupported(
                "Scenario probe requires exact readonly binary mode",
            ));
        }
        Ok(self
            .open(filename, FileRole::ScenarioProbe)?
            .map_or(path::Opened::Missing, |token| {
                path::Opened::File(u64::from(token))
            }))
    }
    fn close_scenario(&mut self, token: u64) -> Result<(), Stop> {
        let token = u32::try_from(token)
            .map_err(|_| unsupported("Scenario capability exceeds native width"))?;
        match self.capabilities.get(&token) {
            Some(Capability::File(file)) if file.role == FileRole::ScenarioProbe => (),
            Some(_) => {
                return Err(unsupported(
                    "Scenario close names a different capability role",
                ));
            }
            None => return Err(Stop::Unavailable("Scenario capability is not live".into())),
        }
        self.capabilities.remove(&token);
        Ok(())
    }
}

impl names::Host for BootstrapServices<'_> {
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        self.allocate_string(path::Scope::World, request)
    }
    fn free_string(&mut self, owner: &Owner) -> Result<(), Stop> {
        self.free_string(path::Scope::World, owner)
    }
}

impl file::Host for BootstrapServices<'_> {
    fn allocate_file(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        if bytes != 128 {
            return Err(unsupported("unqualified resource File object extent"));
        }
        self.memory.allocate(Kind::FileObject, bytes)
    }
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        self.allocate_string(path::Scope::World, request)
    }
    fn free_string(&mut self, owner: &Owner) -> Result<(), Stop> {
        self.free_string(path::Scope::World, owner)
    }
}

impl handle::Host for BootstrapServices<'_> {
    fn allocate_handle(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        if bytes != 36 {
            return Err(unsupported("unqualified resource handle extent"));
        }
        self.memory.allocate(Kind::ResourceHandle, bytes)
    }
    fn file_size(&mut self, token: u32) -> Result<handle::FileSize, Stop> {
        Ok(handle::FileSize::Bytes(
            self.resource_file(token)?.bytes.len() as u64,
        ))
    }
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        self.allocate_string(path::Scope::World, request)
    }
    fn free_string(&mut self, owner: &Owner) -> Result<(), Stop> {
        self.free_string(path::Scope::World, owner)
    }
}

impl opening::Host for BootstrapServices<'_> {
    fn allocate_wide(&mut self, request: opening::Request) -> Result<Allocated, Stop> {
        if request.bytes() == 0 || request.bytes() % 2 != 0 {
            return Err(unsupported(
                "wide allocation requires a nonzero even byte extent",
            ));
        }
        self.memory
            .allocate(Kind::Wide(wide_purpose(request)), request.bytes())
    }
    fn free_wide(&mut self, owner: &Owner) -> Result<(), Stop> {
        let kind = self.memory.allocation_kind(owner.token)?;
        if !matches!(kind, Kind::Wide(_)) {
            return Err(unsupported(
                "wide release names a different allocation role",
            ));
        }
        self.memory.release_owned(owner, kind)
    }
    fn locale(&mut self, _: opening::LocaleRequest) -> Result<opening::LocaleReply, Stop> {
        match self.locale {
            LocalePolicy::C => Ok(opening::LocaleReply::Value(b"C".to_vec())),
        }
    }
    fn open_readonly(&mut self, request: &opening::OpenRequest) -> Result<opening::Opened, Stop> {
        if request.access != 0x80000000
            || request.sharing != 1
            || request.security != 0
            || request.disposition != 3
            || request.flags != 0
            || request.template != 0
            || !(8..=256).contains(&request.filename.len())
            || request
                .filename
                .iter()
                .any(|unit| *unit == 0 || *unit > 127)
        {
            return Err(unsupported(
                "readonly open differs from the recovered ASCII request profile",
            ));
        }
        let filename: Vec<_> = request.filename.iter().map(|unit| *unit as u8).collect();
        Ok(self
            .open(&filename, FileRole::ResourceReadonly)?
            .map_or(opening::Opened::Failed, opening::Opened::File))
    }
    fn seek_current(&mut self, token: u32) -> Result<opening::Seeked, Stop> {
        Ok(opening::Seeked::Position(
            self.resource_file(token)?.position as u64,
        ))
    }
}

impl manager::Host for BootstrapServices<'_> {
    fn allocate_buckets(&mut self, map: u8, bytes: u32) -> Result<Allocated, Stop> {
        if map >= 2 || bytes != manager::bucket_count(100) * 4 {
            return Err(unsupported("unqualified resource manager bucket request"));
        }
        self.memory.allocate(Kind::Buckets(map), bytes)
    }
    fn initialize_section(&mut self, address: u64, bytes: u32) -> Result<u64, Stop> {
        if self
            .capabilities
            .values()
            .any(|cap| matches!(cap, Capability::Section(section) if section.address == address))
        {
            return Err(unsupported(
                "resource manager section is already initialized",
            ));
        }
        let token = self.next_capability()?;
        let section = Section::create(address, bytes)?;
        self.commit(token, Capability::Section(section));
        Ok(u64::from(token))
    }
}

impl native_game::Host for BootstrapServices<'_> {
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        native_game::Host::allocate_name(&mut self.memory, bytes)
    }
}
impl native_world::Host for BootstrapServices<'_> {
    fn allocate(&mut self, request: native_world::Request) -> Result<Allocated, Stop> {
        native_world::Host::allocate(&mut self.memory, request)
    }
}
impl native_world_children::Host for BootstrapServices<'_> {
    fn allocate(&mut self, request: native_world_children::Request) -> Result<Allocated, Stop> {
        native_world_children::Host::allocate(&mut self.memory, request)
    }
}
impl vector::Host for BootstrapServices<'_> {
    fn allocate_vector_elements(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        vector::Host::allocate_vector_elements(&mut self.memory, bytes)
    }
}
impl native_world::resources::Host for BootstrapServices<'_> {
    fn allocate_resource_vector(&mut self, vector: u8, bytes: u32) -> Result<Allocated, Stop> {
        native_world::resources::Host::allocate_resource_vector(&mut self.memory, vector, bytes)
    }
}
impl population::Host for BootstrapServices<'_> {
    fn allocate_population(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        population::Host::allocate_population(&mut self.memory, bytes)
    }
    fn free_null(&mut self) -> Result<(), Stop> {
        population::Host::free_null(&mut self.memory)
    }
}

impl saved_matrix::Host for BootstrapServices<'_> {
    fn lookup(&mut self, _: u32, _: u32, _: u32) -> Result<Lookup, Stop> {
        Err(Stop::Unavailable(
            "saved resource lookup requires the retained IFF source owner".into(),
        ))
    }
    fn source(&mut self, _: u64) -> Result<Vec<u8>, Stop> {
        Err(Stop::Unavailable(
            "saved resource bytes require the retained IFF source owner".into(),
        ))
    }
    fn release(&mut self, _: u64) -> Result<(), Stop> {
        Err(Stop::Unavailable(
            "saved resource lease requires the retained IFF source owner".into(),
        ))
    }
    fn free(&mut self, token: u64) -> Result<(), Stop> {
        self.memory.release_matrix(token)
    }
    fn allocate(&mut self, request: AllocationRequest) -> Result<Allocated, Stop> {
        self.memory.allocate_matrix(request)
    }
}
impl native_matrix::Host for BootstrapServices<'_> {
    fn require_owner(&self, owner: u64, bytes: u32) -> Result<(), Stop> {
        self.memory.require_owner_view(owner, bytes)
    }
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        self.memory.allocate(Kind::Name, bytes)
    }
    fn release_name(&mut self, token: u64) -> Result<(), Stop> {
        self.memory.release(token, Kind::Name)
    }
}
impl native_world::arrays::Host for BootstrapServices<'_> {
    fn allocate_array_root(&mut self, _: u8, bytes: u32) -> Result<Allocated, Stop> {
        self.memory.allocate(Kind::Owner, bytes)
    }
}
impl saved_world::Host for BootstrapServices<'_> {
    fn construct_temporary(
        &mut self,
        _: saved_world::Legacy,
    ) -> Result<saved_world::Temporary, Stop> {
        Err(Stop::Unavailable(
            "legacy temporary constructor semantics are not supplied".into(),
        ))
    }
    fn destroy_temporary(&mut self, _: &saved_world::Temporary) -> Result<(), Stop> {
        Err(Stop::Unavailable(
            "legacy temporary destructor semantics are not supplied".into(),
        ))
    }
    fn name_arrays(&mut self) -> Result<(), Stop> {
        Err(unsupported(
            "raw World names require the owning reader and retained naming child",
        ))
    }
}

#[cfg(test)]
mod tests;
