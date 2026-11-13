//! Resources for the sealed collection prefix, stopping before import666238.
//! This module needs the parent-owned factory-origin and live-image handoff
//! described below; it does not complete a factory or enable any alpha gate.
//!
//! Parent wiring (not supplied by this two-file change):
//! - declare this module, Capability::{SystemCollection, SystemCollectionSection}
//!   and memory::Kind::SystemList; declare native_system::collection (Worker A).
//! - implement system_factory::require_collection_origin(services, request).
//!   First admission must prove the actual issuing factory capability, heap,
//!   both trees, first Base72 return and exact Collection frontier5d5879 after
//!   store5d5875. After binding, validate that same retained factory invocation
//!   and call Binding::require_live instead of checking stale factory images.
//! - after EVERY synchronize/allocate_list/initialize_section invocation, on
//!   Ok AND Err and before any successful reply can be lost, use current_images to
//!   refresh the existing first SystemBase72 binding's parent_image and
//!   caller_image (and the factory's shared current-image record). This is an
//!   infallible, exclusively borrowed handoff of authenticated retained images.
//!   An allocation quota error may follow successful source synchronization.
//!   Do not call
//!   the old factory synchronizer or acknowledge an old child as an image reset.
//! - route section/debug lookups for SystemCollectionSection to its methods,
//!   passing the complete services, not just Memory. Adapt collection::Host to
//!   the three functions below, retaining and checking its Request identity.
//!
//! All source/API tests below are authored only and remain UNRUN. The retained
//! locks are Rust Mutex resources, without a Win32 lock or runtime claim.
use super::{BootstrapServices, Capability, unsupported};
use crate::native_bootstrap::memory::{Identity, Kind, Memory};
use sims_sim::{
    native_application::{InitializedSection, SectionRequest},
    native_system::{
        collection::{Checkpoint, ListRequest, Request, SectionCall, SectionSite},
        factory::TreeSite,
    },
    native_world_children::Owner,
    saved_matrix::{Allocated, MAX_ALLOCATION_BYTES, Stop},
};
use std::sync::{Arc, Mutex};

const SECTION_OFFSETS: [usize; 3] = [136, 180, 224];

/// One owning source invocation. Full current images include writes performed
/// by this host before an interpreter has received/consumed its retained reply.
pub(super) struct Binding {
    capability: u32,
    request: Request,
    heap: Identity,
    initial_parent: Owner,
    initial_caller: Owner,
    parent: Owner,
    caller: Owner,
    initial_list: Option<Owner>,
    list: Option<Owner>,
    list_call: Option<ListRequest>,
    section_calls: [Option<SectionCall>; 3],
    sections: [Option<InitializedSection>; 3],
    section_caps: [Option<u32>; 3],
    section_locks: [Option<Arc<Mutex<()>>>; 3],
    admitted: Option<Checkpoint>,
    event: Option<super::system_collection_event::Link>,
}

/// A role can only be minted from an actual sealed SectionCall. A generic or
/// shifted SectionRequest cannot acquire this capability, even with equal bytes.
pub(super) struct SectionBinding {
    root: u32,
    heap: Identity,
    call: SectionCall,
    returned: InitializedSection,
    lock: Arc<Mutex<()>>,
}

fn site_index(site: SectionSite) -> usize {
    match site {
        SectionSite::Collection => 0,
        SectionSite::List => 1,
        SectionSite::Resource => 2,
    }
}
fn span(owner: &Owner) -> Result<(u64, u64), Stop> {
    let end = owner.token.checked_add(owner.data.len() as u64);
    if owner.token == 0
        || owner.data.is_empty()
        || owner.data.len() > MAX_ALLOCATION_BYTES
        || end.is_none_or(|end| end > 1_u64 << 32)
    {
        return Err(unsupported(
            "collection requires complete bounded native-width owners",
        ));
    }
    Ok((owner.token, end.unwrap()))
}
fn separate(a: &Owner, b: &Owner) -> Result<(), Stop> {
    let (a, ae) = span(a)?;
    let (b, be) = span(b)?;
    if a < be && b < ae {
        return Err(unsupported(
            "collection retained complete allocations alias",
        ));
    }
    Ok(())
}
fn image(memory: &Memory, owner: &Owner, kind: Kind) -> Result<(), Stop> {
    span(owner)?;
    if memory.allocation_kind(owner.token)? != kind
        || memory.allocation_bytes(owner.token)? != owner.data
    {
        return Err(unsupported(
            "collection lost an exact live allocation role or full image",
        ));
    }
    Ok(())
}
fn copy_owner(owner: &Owner) -> Result<Owner, Stop> {
    let mut data = Vec::new();
    data.try_reserve_exact(owner.data.len()).map_err(|_| {
        Stop::Unavailable("collection full-image storage could not be reserved".into())
    })?;
    data.extend_from_slice(&owner.data);
    Ok(Owner {
        token: owner.token,
        data,
    })
}
fn key(services: &BootstrapServices<'_>, request: &Request) -> Result<Option<u32>, Stop> {
    let mut found = None;
    for (token, cap) in &services.capabilities {
        if let Capability::SystemCollection(held) = cap {
            if held.request.same_call(request) {
                if found.replace(*token).is_some() {
                    return Err(unsupported(
                        "collection invocation has duplicate owning capabilities",
                    ));
                }
            } else if held.initial_parent.token == request.factory().parent().token
                || held.initial_caller.token == request.factory().caller().token
            {
                return Err(unsupported(
                    "collection owners already belong to another invocation",
                ));
            }
        }
    }
    Ok(found)
}
fn binding<'s>(services: &'s BootstrapServices<'_>, token: u32) -> Result<&'s Binding, Stop> {
    match services.capabilities.get(&token) {
        Some(Capability::SystemCollection(binding)) => Ok(binding),
        _ => Err(unsupported(
            "collection owning capability is missing or has another role",
        )),
    }
}
fn binding_mut<'s>(services: &'s mut BootstrapServices<'_>, token: u32) -> &'s mut Binding {
    let Some(Capability::SystemCollection(binding)) = services.capabilities.get_mut(&token) else {
        unreachable!("exclusively borrowed prechecked collection binding")
    };
    binding
}

impl Binding {
    pub(super) fn capability(&self) -> u32 {
        self.capability
    }
    pub(super) fn event(&self) -> Option<&super::system_collection_event::Link> {
        self.event.as_ref()
    }
    pub(super) fn admit_event(
        &self,
        call: &sims_sim::native_system::collection::event::Call,
    ) -> Result<(), Stop> {
        call.validate()?;
        let cut = call.prefix();
        if self.event.is_some()
            || !cut.request().same_call(&self.request)
            || self
                .admitted
                .as_ref()
                .is_none_or(|actual| !same_checkpoint(actual, cut))
            || cut.source_ordinal() != 39
            || cut.parent() != &self.parent
            || cut.caller_frame() != &self.caller
            || cut.initial_list() != self.initial_list.as_ref()
            || cut.list() != self.list.as_ref()
            || cut.sections() != &self.sections
        {
            return Err(unsupported(
                "event entry is not the actual synchronized collection resource cut",
            ));
        }
        Ok(())
    }
    pub(super) fn install_event(&mut self, link: super::system_collection_event::Link) {
        assert!(
            self.event.is_none(),
            "prechecked event slot remains vacant in exclusive services"
        );
        self.event = Some(link);
    }
    pub(super) fn publish_event(
        &mut self,
        parent: Owner,
        cut: sims_sim::native_system::collection::event::Checkpoint,
    ) {
        self.parent = parent;
        self.event.as_mut().expect("prechecked event link").admitted = Some(cut);
    }
    pub(super) fn request(&self) -> &Request {
        &self.request
    }
    pub(super) fn parent(&self) -> &Owner {
        &self.parent
    }
    pub(super) fn caller_frame(&self) -> &Owner {
        &self.caller
    }

    /// No factory callback here: the parent origin check may call this method.
    /// It must separately retain/check its own issuing factory capability.
    pub(super) fn require_live(&self, services: &BootstrapServices<'_>) -> Result<(), Stop> {
        if !services.memory.has_identity(&self.heap) {
            return Err(unsupported("collection belongs to another heap lifetime"));
        }
        self.request.validate()?;
        let Some(root) = key(services, &self.request)? else {
            return Err(unsupported("collection lost its owning capability"));
        };
        if root != self.capability || !std::ptr::eq(binding(services, root)?, self) {
            return Err(unsupported("collection owning capability was swapped"));
        }
        image(&services.memory, &self.parent, Kind::Owner)?;
        image(&services.memory, &self.caller, Kind::Owner)?;
        if self.parent.data.len() != 1240
            || self.initial_parent != *self.request.factory().parent()
            || self.initial_caller != *self.request.factory().caller()
            || self.parent.token != self.initial_parent.token
            || self.caller.token != self.initial_caller.token
            || self.caller.data.len() != self.initial_caller.data.len()
        {
            return Err(unsupported(
                "collection changed its initial complete factory owners",
            ));
        }
        if self.initial_list.is_some() != self.list.is_some()
            || self.list.is_some() != self.list_call.is_some()
        {
            return Err(unsupported(
                "collection lost its retained list allocation receipt",
            ));
        }
        if let Some(list) = &self.list {
            let initial = self.initial_list.as_ref().unwrap();
            if initial.token != list.token || initial.data.len() != 24 || list.data.len() != 24 {
                return Err(unsupported(
                    "collection changed its complete list allocation",
                ));
            }
            let call = self.list_call.as_ref().unwrap();
            call.validate()?;
            if !call.checkpoint().request().same_call(&self.request) {
                return Err(unsupported("collection list has another source invocation"));
            }
            image(&services.memory, list, Kind::SystemList)?;
        }
        let factory = self.request.factory();
        let trees = [
            factory.tree(TreeSite::Refcount),
            factory.tree(TreeSite::Derived),
        ];
        for tree in trees.iter().flatten() {
            image(&services.memory, tree, Kind::SystemTree)?;
        }
        let prefix = factory
            .base72()
            .ok_or_else(|| unsupported("collection lost first Base72"))?;
        let first = prefix.section();
        let cap = u32::try_from(first.capability)
            .map_err(|_| unsupported("collection first section capability exceeds native width"))?;
        let Some(Capability::SystemBase72(section)) = services.capabilities.get(&cap) else {
            return Err(unsupported("collection lost its first Base72 capability"));
        };
        // Parent-owned refresh must keep the existing first-child lifetime
        // check intact as the shared complete parent/caller images advance.
        section.require_live(&services.memory)?;
        if !section.request.same_call(prefix.request())
            || section.returned != *first
            || self.parent.data[60..84] != first.region.data
        {
            return Err(unsupported("collection changed its retained first section"));
        }
        let first_debug = first.debug_info.as_ref();
        if let Some(debug) = first_debug {
            image(&services.memory, debug, Kind::SystemSectionDebug)?;
        }
        let mut debugs: [Option<&Owner>; 3] = [None, None, None];
        for at in 0..3 {
            match (
                &self.section_calls[at],
                &self.sections[at],
                self.section_caps[at],
                &self.section_locks[at],
            ) {
                (None, None, None, None) => {}
                (Some(call), Some(returned), Some(token), Some(lock)) => {
                    call.validate()?;
                    if site_index(call.site()) != at
                        || !call.checkpoint().request().same_call(&self.request)
                        || returned.capability != u64::from(token)
                    {
                        return Err(unsupported(
                            "collection section receipt has another source role",
                        ));
                    }
                    let request = checked_section(call)?;
                    returned.validate(&request)?;
                    let Some(Capability::SystemCollectionSection(section)) =
                        services.capabilities.get(&token)
                    else {
                        return Err(unsupported(
                            "collection section capability was lost or replaced",
                        ));
                    };
                    if section.root != root
                        || !services.memory.has_identity(&section.heap)
                        || !section.call.same_call(call)
                        || section.returned != *returned
                        || !Arc::ptr_eq(lock, &section.lock)
                    {
                        return Err(unsupported("collection section capabilities were swapped"));
                    }
                    let offset = SECTION_OFFSETS[at];
                    if self.parent.data[offset..offset + 24] != returned.region.data {
                        return Err(unsupported("collection changed its live embedded section"));
                    }
                    let debug = returned.debug_info.as_ref().ok_or_else(|| {
                        unsupported("collection lost actual section debug storage")
                    })?;
                    if debug.data.len() != 32 {
                        return Err(unsupported(
                            "collection debug allocation has another extent",
                        ));
                    }
                    image(&services.memory, debug, Kind::SystemSectionDebug)?;
                    debugs[at] = Some(debug);
                }
                _ => {
                    return Err(unsupported(
                        "collection lost a staged section call or return",
                    ));
                }
            }
        }
        let owners = [
            Some(&self.parent),
            Some(&self.caller),
            self.list.as_ref(),
            trees[0],
            trees[1],
            first_debug,
            debugs[0],
            debugs[1],
            debugs[2],
        ];
        for (at, owner) in owners.iter().enumerate() {
            if let Some(owner) = owner {
                for other in owners[..at].iter().flatten() {
                    separate(owner, other)?;
                }
            }
        }
        super::system_collection_event::require_for_root(services, self)?;
        Ok(())
    }
}

/// Parent-only access for the infallible shared-image refresh immediately after
/// a successful service operation. These are retained authenticated images,
/// never an adoption API. Public resource access still performs full checks.
pub(super) fn current_images<'s>(
    services: &'s BootstrapServices<'_>,
    request: &Request,
) -> Option<(&'s Owner, &'s Owner)> {
    let token = key(services, request).ok()??;
    let held = binding(services, token).ok()?;
    Some((held.parent(), held.caller_frame()))
}

fn same_checkpoint(a: &Checkpoint, b: &Checkpoint) -> bool {
    a.request().same_call(b.request())
        && a.source_ordinal() == b.source_ordinal()
        && a.parent() == b.parent()
        && a.caller_frame() == b.caller_frame()
        && a.list() == b.list()
        && a.initial_list() == b.initial_list()
        && a.sections() == b.sections()
        && a.writes() == b.writes()
}

fn admit(services: &mut BootstrapServices<'_>, request: &Request) -> Result<u32, Stop> {
    request.validate()?;
    // MUST precede even the first equal-image lookup/binding. Byte equality
    // cannot authenticate a foreign heap's first call.
    super::system_factory::require_collection_origin(services, request)?;
    if let Some(token) = key(services, request)? {
        binding(services, token)?.require_live(services)?;
        return Ok(token);
    }
    if services.capabilities.values().any(|cap| {
        matches!(cap, Capability::SystemCollectionSection(section)
            if section.call.checkpoint().request().same_call(request))
    }) {
        return Err(unsupported(
            "collection lost the owning capability of retained section effects",
        ));
    }
    let factory = request.factory();
    image(&services.memory, factory.parent(), Kind::Owner)?;
    image(&services.memory, factory.caller(), Kind::Owner)?;
    let token = services.next_capability()?;
    let initial_parent = copy_owner(factory.parent())?;
    let initial_caller = copy_owner(factory.caller())?;
    let parent = copy_owner(&initial_parent)?;
    let caller = copy_owner(&initial_caller)?;
    let held = request.clone();
    let heap = services.memory.identity();
    services.commit(
        token,
        Capability::SystemCollection(Binding {
            capability: token,
            request: held,
            heap,
            initial_parent,
            initial_caller,
            parent,
            caller,
            initial_list: None,
            list: None,
            list_call: None,
            section_calls: [None, None, None],
            sections: [None, None, None],
            section_caps: [None, None, None],
            section_locks: [None, None, None],
            admitted: None,
            event: None,
        }),
    );
    Ok(token)
}

pub(super) fn synchronize(
    services: &mut BootstrapServices<'_>,
    cut: &Checkpoint,
) -> Result<(), Stop> {
    cut.validate()?;
    let token = admit(services, cut.request())?;
    let held = binding(services, token)?;
    // Even the identical-prefix fast path must never publish after the event
    // owns a later source domain or has an undelivered retained creation.
    if held.event.is_some() {
        return Err(unsupported(
            "collection prefix cannot synchronize after event takeover",
        ));
    }
    if let Some(previous) = &held.admitted {
        if cut.source_ordinal() < previous.source_ordinal() {
            return Err(unsupported(
                "historical collection checkpoint cannot rewind live source",
            ));
        }
        if same_checkpoint(cut, previous) {
            // An effect's reply may not yet have reached the cursor. Retain the
            // NEWER host images/allocation and allow exactly its old sync seam.
            return Ok(());
        }
        if cut.source_ordinal() == previous.source_ordinal() {
            return Err(unsupported(
                "collection source ordinal changed images or staged receipts",
            ));
        }
    }
    if cut.initial_list() != held.initial_list.as_ref() || cut.sections() != &held.sections {
        return Err(unsupported(
            "collection checkpoint substituted, swapped or lost actual returns",
        ));
    }
    if cut.parent().token != held.parent.token
        || cut.parent().data.len() != 1240
        || cut.caller_frame().token != held.caller.token
        || cut.caller_frame().data.len() != held.caller.data.len()
        || cut.list().map(|o| (o.token, o.data.len()))
            != held.list.as_ref().map(|o| (o.token, o.data.len()))
    {
        return Err(unsupported(
            "collection checkpoint changed complete live owner extents",
        ));
    }
    if cut.parent().data[60..84] != held.parent.data[60..84] {
        return Err(unsupported(
            "collection source changed the retained first Base72 section",
        ));
    }
    for (at, section) in held.sections.iter().enumerate() {
        if let Some(section) = section {
            let offset = SECTION_OFFSETS[at];
            if cut.parent().data[offset..offset + 24] != section.region.data {
                return Err(unsupported(
                    "collection source changed an initialized live section",
                ));
            }
        }
    }
    // Replay is sealed by Worker A; every actual return and CURRENT live image
    // has been checked before any write or replacement of retained snapshots.
    let parent = copy_owner(cut.parent())?;
    let caller = copy_owner(cut.caller_frame())?;
    let list = cut.list().map(copy_owner).transpose()?;
    let admitted = cut.clone();
    services
        .memory
        .synchronize_owner(&parent)
        .expect("prechecked live complete parent");
    services
        .memory
        .synchronize_owner(&caller)
        .expect("prechecked live complete caller");
    if let Some(list) = &list {
        services
            .memory
            .synchronize_owner(list)
            .expect("prechecked actual complete list");
    }
    let held = binding_mut(services, token);
    held.parent = parent;
    held.caller = caller;
    held.list = list;
    held.admitted = Some(admitted);
    Ok(())
}

pub(super) fn allocate_list(
    services: &mut BootstrapServices<'_>,
    call: &ListRequest,
) -> Result<Allocated, Stop> {
    call.validate()?;
    if call.instruction() != 0x6017c8 || call.bytes() != 24 {
        return Err(unsupported(
            "collection list call has another source instruction or extent",
        ));
    }
    let token = admit(services, call.checkpoint().request())?;
    let held = binding(services, token)?;
    if let Some(previous) = &held.list_call {
        if !previous.same_call(call) {
            return Err(unsupported(
                "collection list role belongs to another sealed call",
            ));
        }
        let owner = held.initial_list.as_ref().unwrap();
        return Ok(Allocated::Bytes {
            token: owner.token,
            data: owner.data.clone(),
        });
    }
    synchronize(services, call.checkpoint())?;
    let retained = call.clone();
    // The owning capability already exists; allocation itself is transactional
    // on quota failure. No Result::Err follows a successful real allocation.
    let Allocated::Bytes {
        token: address,
        data,
    } = services.memory.allocate(Kind::SystemList, 24)?
    else {
        unreachable!("logical allocator never fabricates a native null")
    };
    let owner = Owner {
        token: address,
        data,
    };
    let returned = Allocated::Bytes {
        token: address,
        data: owner.data.clone(),
    };
    let held = binding_mut(services, token);
    held.initial_list = Some(owner.clone());
    held.list = Some(owner);
    held.list_call = Some(retained);
    Ok(returned)
}

fn checked_section(call: &SectionCall) -> Result<SectionRequest, Stop> {
    call.validate()?;
    let request = call.section_request();
    let parent = call.checkpoint().parent();
    let offset = SECTION_OFFSETS[site_index(call.site())];
    if parent.data.len() != 1240
        || request.instruction != 0x5d91bc
        || request.import_slot != 0x666114
        || parent.token.checked_add(offset as u64) != Some(request.region.token)
        || request.region.data.len() != 24
        || request.region.data != parent.data[offset..offset + 24]
        || request.region.data != [0; 24]
    {
        return Err(unsupported(
            "collection section is not its sealed source role and zeroed input",
        ));
    }
    Ok(request)
}

pub(super) fn initialize_section(
    services: &mut BootstrapServices<'_>,
    call: &SectionCall,
) -> Result<InitializedSection, Stop> {
    let request = checked_section(call)?;
    let token = admit(services, call.checkpoint().request())?;
    let at = site_index(call.site());
    let held = binding(services, token)?;
    if let Some(previous) = &held.section_calls[at] {
        if !previous.same_call(call) {
            return Err(unsupported(
                "collection section role belongs to another sealed call",
            ));
        }
        return Ok(held.sections[at].as_ref().unwrap().clone());
    }
    // Reject an occupied address regardless of capability role. In particular,
    // generic Application/manager sections cannot be relabeled by equal bytes.
    for cap in services.capabilities.values() {
        let address = match cap {
            Capability::Section(s) => Some(s.address),
            Capability::SystemBase72(s) => Some(s.returned.region.token),
            Capability::SystemCollectionSection(s) => Some(s.returned.region.token),
            _ => None,
        };
        if address == Some(request.region.token) {
            return Err(unsupported(
                "collection section address already has an issued role",
            ));
        }
    }
    // Reserve capacity before source synchronization as well as allocation.
    let cap = services.next_capability()?;
    synchronize(services, call.checkpoint())?;
    let held_call = call.clone();
    let root_call = call.clone();
    let mut parent = copy_owner(binding(services, token)?.parent())?;
    let heap = services.memory.identity();
    let lock = Arc::new(Mutex::new(()));
    let retained_lock = lock.clone();
    let Allocated::Bytes {
        token: address,
        data,
    } = services.memory.allocate(Kind::SystemSectionDebug, 32)?
    else {
        unreachable!("logical allocator never fabricates a native null")
    };
    // No Result::Err after this effect. Same-heap monotonic allocation proves
    // fresh32-byte debug storage is disjoint from every retained complete owner.
    let debug_input = Owner {
        token: address,
        data,
    };
    let returned =
        InitializedSection::from_debug_info(&request, Some(&debug_input), u64::from(cap))
            .expect("checked source role and fresh bounded debug allocation");
    let offset = SECTION_OFFSETS[at];
    parent.data[offset..offset + 24].copy_from_slice(&returned.region.data);
    services
        .memory
        .synchronize_owner(returned.debug_info.as_ref().unwrap())
        .expect("fresh complete debug allocation");
    services
        .memory
        .synchronize_owner(&parent)
        .expect("prechecked live complete parent");
    let result = returned.clone();
    let root_returned = returned.clone();
    services.commit(
        cap,
        Capability::SystemCollectionSection(SectionBinding {
            root: token,
            heap,
            call: held_call,
            returned,
            lock,
        }),
    );
    let held = binding_mut(services, token);
    held.parent = parent;
    held.section_calls[at] = Some(root_call);
    held.sections[at] = Some(root_returned);
    held.section_caps[at] = Some(cap);
    held.section_locks[at] = Some(retained_lock);
    Ok(result)
}

impl SectionBinding {
    fn require_live(&self, services: &BootstrapServices<'_>) -> Result<(), Stop> {
        let held = binding(services, self.root)?;
        if !services.memory.has_identity(&self.heap)
            || !held.request.same_call(self.call.checkpoint().request())
        {
            return Err(unsupported(
                "collection section belongs to another heap or source invocation",
            ));
        }
        super::system_factory::require_collection_origin(services, &held.request)?;
        held.require_live(services)?;
        let token = u32::try_from(self.returned.capability)
            .map_err(|_| unsupported("collection capability exceeds native width"))?;
        let Some(Capability::SystemCollectionSection(actual)) = services.capabilities.get(&token)
        else {
            return Err(unsupported("collection section lost its capability role"));
        };
        if !std::ptr::eq(self, actual) {
            return Err(unsupported("collection section capability was swapped"));
        }
        Ok(())
    }
    pub(super) fn section<'s>(
        &'s self,
        services: &BootstrapServices<'_>,
        address: u64,
        bytes: u32,
    ) -> Result<&'s Arc<Mutex<()>>, Stop> {
        if address != self.returned.region.token || bytes != 24 {
            return Err(unsupported(
                "collection section lookup has another address or extent",
            ));
        }
        self.require_live(services)?;
        Ok(&self.lock)
    }
    pub(super) fn debug_info<'s>(
        &'s self,
        services: &BootstrapServices<'_>,
        address: u64,
        bytes: u32,
    ) -> Result<Option<&'s Owner>, Stop> {
        self.section(services, address, bytes)?;
        Ok(self.returned.debug_info.as_ref())
    }
}

#[cfg(test)]
mod tests;
