//! Owning allocator provenance and source-authenticated parent/caller images.
//! A Base72 call cannot choose an equal-image foreign heap: its sealed factory
//! invocation must already own the actual allocation in these same services.
use super::*;
use sims_sim::native_system::{
    base72, collection,
    factory::{self, Base72Call, Checkpoint, TreeRequest, TreeSite},
};

pub(super) struct Binding {
    request: factory::Request,
    heap: Identity,
    initial_parent: Owner,
    parent: Owner,
    caller: Owner,
    initial_trees: [Option<Owner>; 2],
    trees: [Option<Owner>; 2],
    tree_requests: [Option<TreeRequest>; 2],
    prepared: Option<Checkpoint>,
    call: Option<Base72Call>,
    section_capability: Option<u32>,
    base72_acknowledged: bool,
    admitted_ordinal: usize,
    collection_request: Option<collection::Request>,
}
fn index(site: TreeSite) -> usize {
    match site {
        TreeSite::Refcount => 0,
        TreeSite::Derived => 1,
    }
}
fn image(memory: &Memory, owner: &Owner, kind: Kind) -> Result<(), Stop> {
    if memory.allocation_kind(owner.token)? != kind
        || memory.allocation_bytes(owner.token)? != owner.data
    {
        return Err(unsupported(
            "system factory lost a complete issuing allocation image",
        ));
    }
    Ok(())
}
impl Binding {
    fn live(&self, memory: &Memory) -> Result<(), Stop> {
        if !memory.has_identity(&self.heap) {
            return Err(unsupported(
                "system factory belongs to another heap lifetime",
            ));
        }
        image(memory, &self.parent, Kind::Owner)?;
        image(memory, &self.caller, Kind::Owner)?;
        for owner in self.trees.iter().flatten() {
            image(memory, owner, Kind::SystemTree)?;
        }
        Ok(())
    }
}
fn token(services: &BootstrapServices<'_>, request: &factory::Request) -> Result<u32, Stop> {
    services
        .capabilities
        .iter()
        .find_map(|(token, cap)| match cap {
            Capability::SystemFactory(binding) if binding.request.same_call(request) => {
                Some(*token)
            }
            _ => None,
        })
        .ok_or_else(|| unsupported("system source call has no allocation issued by this heap"))
}
fn binding<'b>(services: &'b BootstrapServices<'_>, token: u32) -> &'b Binding {
    let Capability::SystemFactory(binding) = &services.capabilities[&token] else {
        unreachable!("checked system factory capability")
    };
    binding
}
fn binding_mut<'b>(services: &'b mut BootstrapServices<'_>, token: u32) -> &'b mut Binding {
    let Capability::SystemFactory(binding) = services.capabilities.get_mut(&token).unwrap() else {
        unreachable!("checked system factory capability")
    };
    binding
}
fn live(services: &BootstrapServices<'_>, issued: &Binding) -> Result<(), Stop> {
    if let Some(child) = collection_binding(services, &issued.request)? {
        check_collection_link(services, issued, child.request())?;
        return child.require_live(services);
    }
    if issued.collection_request.is_some() {
        return Err(unsupported(
            "system factory lost its admitted collection owner",
        ));
    }
    issued.live(&services.memory)?;
    if let Some(token) = issued.section_capability {
        let Some(Capability::SystemBase72(section)) = services.capabilities.get(&token) else {
            return Err(unsupported(
                "system factory lost its issued child section capability",
            ));
        };
        if !issued
            .call
            .as_ref()
            .is_some_and(|call| call.request().same_call(&section.request))
        {
            return Err(unsupported(
                "system factory child section has another invocation",
            ));
        }
        section.require_live(&services.memory)?;
    }
    Ok(())
}
pub(super) fn allocate(
    services: &mut BootstrapServices<'_>,
    request: &factory::Request,
) -> Result<Allocated, Stop> {
    request.validate()?;
    if services.memory.system_origin() != request.incoming().origin {
        return Err(unsupported(
            "system factory first call belongs to another heap origin",
        ));
    }
    if let Ok(token) = token(services, request) {
        let binding = binding(services, token);
        live(services, binding)?;
        return Ok(Allocated::Bytes {
            token: binding.initial_parent.token,
            data: binding.initial_parent.data.clone(),
        });
    }
    image(&services.memory, &request.incoming().caller, Kind::Owner)?;
    if services.capabilities.values().any(|cap|matches!(cap,Capability::SystemFactory(binding) if binding.caller.token==request.incoming().caller.token)) {
        return Err(unsupported("system factory caller is already retained by another invocation"));
    }
    let cap = services.next_capability()?;
    let held = request.clone();
    let caller = request.incoming().caller.clone();
    let heap = services.memory.identity();
    let parent = services.memory.allocate_owner(1240)?;
    let returned = Allocated::Bytes {
        token: parent.token,
        data: parent.data.clone(),
    };
    services.commit(
        cap,
        Capability::SystemFactory(Binding {
            request: held,
            heap,
            initial_parent: parent.clone(),
            parent,
            caller,
            initial_trees: [None, None],
            trees: [None, None],
            tree_requests: [None, None],
            prepared: None,
            call: None,
            section_capability: None,
            base72_acknowledged: false,
            admitted_ordinal: 0,
            collection_request: None,
        }),
    );
    Ok(returned)
}
fn check_checkpoint(
    services: &BootstrapServices<'_>,
    checkpoint: &Checkpoint,
) -> Result<u32, Stop> {
    checkpoint.validate()?;
    let token = token(services, checkpoint.request())?;
    let issued = binding(services, token);
    live(services, issued)?;
    if issued.collection_request.is_some()
        || collection_binding(services, &issued.request)?.is_some()
    {
        return Err(unsupported(
            "factory checkpoints cannot publish after collection takeover",
        ));
    }
    if checkpoint.source_ordinal() < issued.admitted_ordinal {
        return Err(unsupported(
            "historical system source cut cannot rewind admitted images",
        ));
    }
    if checkpoint.initial_parent() != &issued.initial_parent {
        return Err(unsupported(
            "system cut replaced its actual factory allocation return",
        ));
    }
    for site in [TreeSite::Refcount, TreeSite::Derived] {
        if checkpoint.initial_tree(site) != issued.initial_trees[index(site)].as_ref() {
            return Err(unsupported("system cut replaced a tree allocator return"));
        }
    }
    match (
        checkpoint.base72_call(),
        checkpoint.base72(),
        issued.call.as_ref(),
    ) {
        (None, None, None) => {}
        (Some(call), Some(prefix), Some(actual))
            if call.request().same_call(actual.request())
                && prefix.request().same_call(actual.request()) =>
        {
            let cap = u32::try_from(prefix.section().capability)
                .map_err(|_| unsupported("system section capability exceeds native width"))?;
            let Some(Capability::SystemBase72(section)) = services.capabilities.get(&cap) else {
                return Err(unsupported("system cut lost its actual section resource"));
            };
            section.require_live(&services.memory)?;
            if section.returned != *prefix.section() {
                return Err(unsupported("system cut replaced its actual section return"));
            }
        }
        _ => {
            return Err(unsupported(
                "system cut lost its actual Base72 source invocation",
            ));
        }
    }
    Ok(token)
}
// All validation precedes copying. These images are authenticated by the
// sealed root interpreter AND actual allocator/call receipts above.
fn copy_checkpoint(services: &mut BootstrapServices<'_>, token: u32, checkpoint: &Checkpoint) {
    services
        .memory
        .synchronize_owner(checkpoint.parent())
        .expect("checked complete system parent");
    services
        .memory
        .synchronize_owner(checkpoint.caller())
        .expect("checked complete system caller");
    for site in [TreeSite::Refcount, TreeSite::Derived] {
        if let Some(owner) = checkpoint.tree(site) {
            services
                .memory
                .synchronize_owner(owner)
                .expect("checked actual system tree");
        }
    }
    if let Some(prefix) = checkpoint.base72() {
        let cap = prefix.section().capability as u32;
        let Some(Capability::SystemBase72(section)) = services.capabilities.get_mut(&cap) else {
            unreachable!("prechecked system section")
        };
        section.parent_image = checkpoint.parent().clone();
        section.caller_image = checkpoint.caller().clone();
    }
    let binding = binding_mut(services, token);
    binding.parent = checkpoint.parent().clone();
    binding.caller = checkpoint.caller().clone();
    binding.admitted_ordinal = checkpoint.source_ordinal();
    for site in [TreeSite::Refcount, TreeSite::Derived] {
        binding.trees[index(site)] = checkpoint.tree(site).cloned();
    }
}
pub(super) fn synchronize(
    services: &mut BootstrapServices<'_>,
    checkpoint: &Checkpoint,
) -> Result<(), Stop> {
    let token = check_checkpoint(services, checkpoint)?;
    // The root stores after Base72 preserve its initialized section. Check it
    // explicitly before updating either shared full-image record.
    if let Some(prefix) = checkpoint.base72() {
        if checkpoint.parent().data[60..84] != prefix.section().region.data {
            return Err(unsupported(
                "system source cut changed its live section bytes",
            ));
        }
    }
    copy_checkpoint(services, token, checkpoint);
    Ok(())
}
pub(super) fn allocate_tree(
    services: &mut BootstrapServices<'_>,
    request: &TreeRequest,
) -> Result<Allocated, Stop> {
    request.validate()?;
    let token = token(services, request.checkpoint().request())?;
    let at = index(request.site());
    if let Some(previous) = &binding(services, token).tree_requests[at] {
        if !previous.same_call(request) {
            return Err(unsupported(
                "system tree site belongs to another source call",
            ));
        }
        let issued = binding(services, token);
        live(services, issued)?;
        let owner = issued.initial_trees[at].as_ref().unwrap();
        return Ok(Allocated::Bytes {
            token: owner.token,
            data: owner.data.clone(),
        });
    }
    let token = check_checkpoint(services, request.checkpoint())?;
    if let Some(prefix) = request.checkpoint().base72() {
        if request.checkpoint().parent().data[60..84] != prefix.section().region.data {
            return Err(unsupported("system tree source changed section bytes"));
        }
    }
    let held = request.clone();
    let Allocated::Bytes {
        token: address,
        data,
    } = services.memory.allocate(Kind::SystemTree, 24)?
    else {
        unreachable!("explicit logical allocator never returns native null")
    };
    // No Err follows the actual allocation; all target owners/roles are live
    // in this exclusively borrowed heap and every full source cut was checked.
    let owner = Owner {
        token: address,
        data,
    };
    let returned = Allocated::Bytes {
        token: address,
        data: owner.data.clone(),
    };
    copy_checkpoint(services, token, request.checkpoint());
    let issued = binding_mut(services, token);
    issued.initial_trees[at] = Some(owner.clone());
    issued.trees[at] = Some(owner);
    issued.tree_requests[at] = Some(held);
    Ok(returned)
}
pub(super) fn prepare(
    services: &mut BootstrapServices<'_>,
    checkpoint: &Checkpoint,
) -> Result<(), Stop> {
    let token = check_checkpoint(services, checkpoint)?;
    if checkpoint.base72().is_some() {
        return Err(unsupported("system Base72 site was already completed"));
    }
    let held = checkpoint.clone();
    copy_checkpoint(services, token, checkpoint);
    binding_mut(services, token).prepared = Some(held);
    Ok(())
}
fn check_call(services: &BootstrapServices<'_>, call: &Base72Call) -> Result<u32, Stop> {
    call.validate()?;
    let token = token(services, call.checkpoint().request())?;
    let issued = binding(services, token);
    live(services, issued)?;
    let prepared = issued
        .prepared
        .as_ref()
        .ok_or_else(|| unsupported("system child has no actual prepared factory cut"))?;
    if prepared.initial_parent() != call.checkpoint().initial_parent()
        || prepared.parent() != call.checkpoint().parent()
        || prepared.caller() != call.checkpoint().caller()
    {
        return Err(unsupported(
            "system Base72 origin differs from its prepared factory images",
        ));
    }
    if issued
        .call
        .as_ref()
        .is_some_and(|prior| !prior.request().same_call(call.request()))
    {
        return Err(unsupported(
            "system Base72 site belongs to another child invocation",
        ));
    }
    Ok(token)
}
/// Called by the private section module on EVERY initialization path, including
/// retries. The request alone cannot select or register an origin heap.
pub(super) fn require_origin(
    services: &BootstrapServices<'_>,
    request: &base72::Request,
) -> Result<(), Stop> {
    let call = services
        .capabilities
        .values()
        .find_map(|cap| match cap {
            Capability::SystemFactory(binding) => binding
                .call
                .as_ref()
                .filter(|call| call.request().same_call(request)),
            _ => None,
        })
        .ok_or_else(|| {
            unsupported("system Base72 first admission lacks its issuing factory origin")
        })?;
    check_call(services, call)?;
    Ok(())
}

fn collection_binding<'s>(
    services: &'s BootstrapServices<'_>,
    request: &factory::Request,
) -> Result<Option<&'s super::system_collection::Binding>, Stop> {
    let mut found = None;
    for cap in services.capabilities.values() {
        if let Capability::SystemCollection(child) = cap {
            if child.request().factory().request().same_call(request) {
                if found.replace(child).is_some() {
                    return Err(unsupported(
                        "system factory has duplicate collection owners",
                    ));
                }
            }
        }
    }
    Ok(found)
}

// Immutable factory receipts remain authoritative after its complete live
// images have advanced into the collection. Their ordinals are separate.
fn check_collection_link(
    services: &BootstrapServices<'_>,
    issued: &Binding,
    request: &collection::Request,
) -> Result<(), Stop> {
    request.validate()?;
    let cut = request.factory();
    if !services.memory.has_identity(&issued.heap)
        || services.memory.system_origin() != cut.request().incoming().origin
        || !issued.request.same_call(cut.request())
        || issued.initial_parent != *cut.initial_parent()
        || issued.admitted_ordinal != cut.source_ordinal()
        || !issued.base72_acknowledged
        || issued
            .collection_request
            .as_ref()
            .is_some_and(|held| !held.same_call(request))
    {
        return Err(unsupported(
            "collection lost its exact issuing factory frontier or heap",
        ));
    }
    for site in [TreeSite::Refcount, TreeSite::Derived] {
        if issued.initial_trees[index(site)].as_ref() != cut.initial_tree(site)
            || issued.trees[index(site)].as_ref() != cut.tree(site)
        {
            return Err(unsupported("collection changed its factory tree receipts"));
        }
        image(
            &services.memory,
            cut.tree(site)
                .ok_or_else(|| unsupported("collection factory tree absent"))?,
            Kind::SystemTree,
        )?;
    }
    let prefix = cut
        .base72()
        .ok_or_else(|| unsupported("collection factory Base72 absent"))?;
    let call = cut
        .base72_call()
        .ok_or_else(|| unsupported("collection factory child call absent"))?;
    if !issued
        .call
        .as_ref()
        .is_some_and(|held| held.request().same_call(call.request()))
        || !call.request().same_call(prefix.request())
        || issued.section_capability.map(u64::from) != Some(prefix.section().capability)
    {
        return Err(unsupported(
            "collection lost the acknowledged factory child identity",
        ));
    }
    let token = u32::try_from(prefix.section().capability)
        .map_err(|_| unsupported("collection factory section token exceeds native width"))?;
    let Some(Capability::SystemBase72(first)) = services.capabilities.get(&token) else {
        return Err(unsupported("collection factory section capability absent"));
    };
    if !first.request.same_call(prefix.request()) || first.returned != *prefix.section() {
        return Err(unsupported(
            "collection substituted its first section return",
        ));
    }
    first.require_live(&services.memory)?;
    Ok(())
}

pub(super) fn require_collection_origin(
    services: &BootstrapServices<'_>,
    request: &collection::Request,
) -> Result<(), Stop> {
    request.validate()?;
    let token = token(services, request.factory().request())?;
    let issued = binding(services, token);
    check_collection_link(services, issued, request)?;
    if let Some(child) = collection_binding(services, &issued.request)? {
        if !child.request().same_call(request) {
            return Err(unsupported(
                "system factory collection belongs to another source invocation",
            ));
        }
        return child.require_live(services);
    }
    if issued.collection_request.is_some() {
        return Err(unsupported(
            "system factory lost its retained collection capability",
        ));
    }
    // First admission checks the actual current complete factory owners. An
    // equal-address, equal-image foreign heap already failed the lifetime test.
    issued.live(&services.memory)?;
    if issued.parent != *request.factory().parent() || issued.caller != *request.factory().caller()
    {
        return Err(unsupported(
            "collection entry is not the current issuing factory cut",
        ));
    }
    Ok(())
}

// Called only after an origin preflight and an exclusively borrowed collection
// operation. Both Ok and Err may retain new source images. These records must
// follow them without writing memory again or returning a post-effect error.
pub(super) fn refresh_collection_images(
    services: &mut BootstrapServices<'_>,
    request: &collection::Request,
) {
    let Some((parent, caller)) = super::system_collection::current_images(services, request) else {
        return;
    };
    let parent = parent.clone();
    let caller = caller.clone();
    let factory_token =
        token(services, request.factory().request()).expect("preflight retained issuing factory");
    let first_token = u32::try_from(
        request
            .factory()
            .base72()
            .expect("validated factory child")
            .section()
            .capability,
    )
    .expect("preflight native-width section capability");
    let Some(Capability::SystemBase72(first)) = services.capabilities.get_mut(&first_token) else {
        unreachable!("preflight retained first section; collection never removes it")
    };
    first.parent_image = parent.clone();
    first.caller_image = caller.clone();
    let issued = binding_mut(services, factory_token);
    issued.parent = parent;
    issued.caller = caller;
    issued.collection_request = Some(request.clone());
}

pub struct CollectionHost<'s, 'a> {
    pub(super) services: &'s mut BootstrapServices<'a>,
    pub(super) request: collection::Request,
}
impl CollectionHost<'_, '_> {
    fn preflight(&self, request: &collection::Request) -> Result<(), Stop> {
        if !self.request.same_call(request) {
            return Err(unsupported("collection host has another source invocation"));
        }
        // Do not repair a tampered dependency after a failed preflight.
        require_collection_origin(self.services, request)
    }
}
impl collection::Host for CollectionHost<'_, '_> {
    fn synchronize_checkpoint(&mut self, cut: &collection::Checkpoint) -> Result<(), Stop> {
        self.preflight(cut.request())?;
        let result = super::system_collection::synchronize(self.services, cut);
        refresh_collection_images(self.services, &self.request);
        result
    }
    fn allocate_list(&mut self, call: &collection::ListRequest) -> Result<Allocated, Stop> {
        self.preflight(call.checkpoint().request())?;
        let result = super::system_collection::allocate_list(self.services, call);
        refresh_collection_images(self.services, &self.request);
        result
    }
    fn initialize_section(
        &mut self,
        call: &collection::SectionCall,
    ) -> Result<sims_sim::native_application::InitializedSection, Stop> {
        self.preflight(call.checkpoint().request())?;
        let result = super::system_collection::initialize_section(self.services, call);
        refresh_collection_images(self.services, &self.request);
        result
    }
}

pub(super) fn initialize_base72(
    services: &mut BootstrapServices<'_>,
    call: &Base72Call,
) -> Result<sims_sim::native_application::InitializedSection, Stop> {
    let token = check_call(services, call)?;
    if binding(services, token).call.is_none() {
        binding_mut(services, token).call = Some(call.clone());
    }
    let reply = super::system_base72::initialize(services, call.request())?;
    let Capability::SystemBase72(section) = &services.capabilities[&(reply.capability as u32)]
    else {
        unreachable!("actual staged system section capability")
    };
    let parent = section.parent_image.clone();
    let caller = section.caller_image.clone();
    let issued = binding_mut(services, token);
    issued.parent = parent;
    issued.caller = caller;
    issued.section_capability = Some(reply.capability as u32);
    Ok(reply)
}
pub(super) fn acknowledge(
    services: &mut BootstrapServices<'_>,
    call: &Base72Call,
    prefix: &base72::Prefix,
) -> Result<(), Stop> {
    let token = check_call(services, call)?;
    if !prefix.request().same_call(call.request()) {
        return Err(unsupported(
            "system acknowledgement has another child invocation",
        ));
    }
    super::system_base72::acknowledge(services, prefix)?;
    let issued = binding_mut(services, token);
    if !issued.base72_acknowledged {
        issued.parent = prefix.parent().clone();
        issued.caller = prefix.caller_frame().clone();
        issued.base72_acknowledged = true;
        issued.admitted_ordinal = call.checkpoint().source_ordinal() + 1;
    }
    Ok(())
}

pub struct Host<'s, 'a> {
    pub(super) services: &'s mut BootstrapServices<'a>,
    pub(super) request: factory::Request,
}
impl factory::Host for Host<'_, '_> {
    fn allocate_provider(&mut self, request: &factory::Request) -> Result<Allocated, Stop> {
        if !self.request.same_call(request) {
            return Err(unsupported("system host has another factory invocation"));
        }
        allocate(self.services, request)
    }
    fn allocate_tree(&mut self, request: &TreeRequest) -> Result<Allocated, Stop> {
        if !self.request.same_call(request.checkpoint().request()) {
            return Err(unsupported(
                "system tree host has another factory invocation",
            ));
        }
        allocate_tree(self.services, request)
    }
    fn prepare_base72(&mut self, checkpoint: &Checkpoint) -> Result<(), Stop> {
        if !self.request.same_call(checkpoint.request()) {
            return Err(unsupported("system prepare has another factory invocation"));
        }
        prepare(self.services, checkpoint)
    }
    fn initialize_base72(
        &mut self,
        call: &Base72Call,
    ) -> Result<sims_sim::native_application::InitializedSection, Stop> {
        if !self.request.same_call(call.checkpoint().request()) {
            return Err(unsupported(
                "system child host has another factory invocation",
            ));
        }
        initialize_base72(self.services, call)
    }
    fn acknowledge_base72(
        &mut self,
        call: &Base72Call,
        prefix: &base72::Prefix,
    ) -> Result<(), Stop> {
        if !self.request.same_call(call.checkpoint().request()) {
            return Err(unsupported(
                "system ack host has another factory invocation",
            ));
        }
        acknowledge(self.services, call, prefix)
    }
    fn synchronize_checkpoint(&mut self, checkpoint: &Checkpoint) -> Result<(), Stop> {
        if !self.request.same_call(checkpoint.request()) {
            return Err(unsupported(
                "system image host has another factory invocation",
            ));
        }
        synchronize(self.services, checkpoint)
    }
}
