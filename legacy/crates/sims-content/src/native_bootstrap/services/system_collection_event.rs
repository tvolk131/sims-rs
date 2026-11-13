//! Actual anonymous Rust event retained across source-return delivery and
//! replay. No native handle interoperability or constructor counter producer.
use super::{BootstrapServices, Capability, unsupported};
use crate::native_bootstrap::memory::Identity;
use sims_kernel::event::{Event, Identity as EventIdentity, ResetMode};
use sims_sim::{
    native_system::collection::{
        self,
        event::{Call, Checkpoint, Return},
    },
    native_world_children::Owner,
    saved_matrix::Stop,
};

/// Sticky root ownership exists immediately after creation, even before any
/// event source checkpoint has been acknowledged. Weak identity is independent
/// of the capability entry so replacement cannot recreate the same resource.
pub(super) struct Link {
    pub(super) call: Call,
    pub(super) returned: Return,
    pub(super) capability: u32,
    pub(super) identity: EventIdentity,
    pub(super) admitted: Option<Checkpoint>,
}
pub(super) struct Binding {
    pub(super) root: u32,
    pub(super) capability: u32,
    pub(super) heap: Identity,
    pub(super) call: Call,
    pub(super) returned: Return,
    pub(super) event: Event,
}

fn root<'s>(
    services: &'s BootstrapServices<'_>,
    request: &collection::Request,
) -> Result<&'s super::system_collection::Binding, Stop> {
    let mut found = None;
    for cap in services.capabilities.values() {
        if let Capability::SystemCollection(held) = cap {
            if held.request().same_call(request) {
                if found.replace(held).is_some() {
                    return Err(unsupported("event has duplicate issuing collection owners"));
                }
            }
        }
    }
    found.ok_or_else(|| unsupported("event lost its actual issuing collection capability"))
}
fn root_mut<'s>(
    services: &'s mut BootstrapServices<'_>,
    token: u32,
) -> &'s mut super::system_collection::Binding {
    let Some(Capability::SystemCollection(held)) = services.capabilities.get_mut(&token) else {
        unreachable!("prechecked exclusively borrowed collection root");
    };
    held
}

/// No root/factory callback here. The root calls this only after validating its
/// complete allocations, trees, sections and first-child dependencies.
pub(super) fn require_for_root(
    services: &BootstrapServices<'_>,
    held: &super::system_collection::Binding,
) -> Result<(), Stop> {
    let Some(link) = held.event() else {
        return Ok(());
    };
    link.call.validate()?;
    link.returned.validate(&link.call)?;
    let Some(Capability::SystemCollectionEvent(actual)) =
        services.capabilities.get(&link.capability)
    else {
        return Err(unsupported(
            "collection lost its retained anonymous event capability",
        ));
    };
    if actual.root != held.capability()
        || actual.capability != link.capability
        || link.capability == 0
        || link.returned.handle() != link.capability
        || !services.memory.has_identity(&actual.heap)
        || !actual.call.same_call(&link.call)
        || !actual.returned.same_receipt(&link.returned)
        || !actual.event.has_identity(&link.identity)
        || !link.call.prefix().request().same_call(held.request())
    {
        return Err(unsupported(
            "anonymous event capability, source return or lifetime was replaced",
        ));
    }
    actual
        .event
        .check_live()
        .map_err(|_| unsupported("anonymous event is retired or poisoned"))?;
    let expected = match &link.admitted {
        None => link.call.prefix().parent(),
        Some(cut) => {
            cut.validate()?;
            if !cut.call().same_call(&link.call)
                || cut
                    .returned()
                    .is_none_or(|r| !r.same_receipt(&link.returned))
            {
                return Err(unsupported(
                    "event source checkpoint lost its actual return",
                ));
            }
            cut.parent()
        }
    };
    if held.parent() != expected || held.caller_frame() != link.call.prefix().caller_frame() {
        return Err(unsupported(
            "anonymous event lost its exact complete current images",
        ));
    }
    Ok(())
}

pub(super) fn create(services: &mut BootstrapServices<'_>, call: &Call) -> Result<Return, Stop> {
    call.validate()?;
    super::system_factory::require_collection_origin(services, call.prefix().request())?;
    let held = root(services, call.prefix().request())?;
    held.require_live(services)?;
    if let Some(link) = held.event() {
        if !link.call.same_call(call) {
            return Err(unsupported(
                "collection event role belongs to another source invocation",
            ));
        }
        // No state poll/reset and no reallocation, even after source stores.
        return Ok(link.returned.clone());
    }
    held.admit_event(call)?;
    let root = held.capability();
    let capability = services.next_capability()?;
    let returned = Return::from_host(call, u64::from(capability))?;
    let heap = services.memory.identity();
    let held_call = call.clone();
    let returned_for_root = returned.clone();
    // Everything fallible precedes the real resource. No typed error follows
    // creation/commit: exclusive ownership preserves the prevalidated slots.
    let event = Event::new(ResetMode::Auto, false);
    let identity = event.identity();
    services.commit(
        capability,
        Capability::SystemCollectionEvent(Binding {
            root,
            capability,
            heap,
            call: held_call.clone(),
            returned: returned.clone(),
            event,
        }),
    );
    root_mut(services, root).install_event(Link {
        call: held_call,
        returned: returned_for_root,
        capability,
        identity,
        admitted: None,
    });
    Ok(returned)
}

fn copy_owner(owner: &Owner) -> Result<Owner, Stop> {
    let mut data = Vec::new();
    data.try_reserve_exact(owner.data.len())
        .map_err(|_| Stop::Unavailable("event synchronization image quota unavailable".into()))?;
    data.extend_from_slice(&owner.data);
    Ok(Owner {
        token: owner.token,
        data,
    })
}
pub(super) fn synchronize(
    services: &mut BootstrapServices<'_>,
    cut: &Checkpoint,
) -> Result<(), Stop> {
    cut.validate()?;
    super::system_factory::require_collection_origin(services, cut.call().prefix().request())?;
    let held = root(services, cut.call().prefix().request())?;
    held.require_live(services)?;
    let link = held
        .event()
        .ok_or_else(|| unsupported("event source has no actual creation receipt"))?;
    if !link.call.same_call(cut.call())
        || cut
            .returned()
            .is_none_or(|r| !r.same_receipt(&link.returned))
    {
        return Err(unsupported("event source substituted its actual return"));
    }
    if let Some(previous) = &link.admitted {
        if cut.source_ordinal() < previous.source_ordinal() {
            return Err(unsupported(
                "historical event checkpoint cannot rewind source images",
            ));
        }
        if cut.source_ordinal() == previous.source_ordinal() {
            if cut.parent() != previous.parent() || cut.writes() != previous.writes() {
                return Err(unsupported(
                    "event ordinal changed its complete source images",
                ));
            }
            return Ok(());
        }
    }
    if cut.parent().token != held.parent().token
        || cut.parent().data.len() != 1240
        || cut.caller_frame() != held.caller_frame()
    {
        return Err(unsupported("event source changed its complete owner role"));
    }
    let token = held.capability();
    let parent = copy_owner(cut.parent())?;
    let saved = cut.clone();
    services
        .memory
        .synchronize_owner(&parent)
        .expect("prechecked complete live event parent");
    root_mut(services, token).publish_event(parent, saved);
    Ok(())
}

pub(super) fn access<'s>(
    services: &'s BootstrapServices<'_>,
    returned: &Return,
) -> Result<&'s Event, Stop> {
    returned.validate(returned.call())?;
    super::system_factory::require_collection_origin(services, returned.call().prefix().request())?;
    let held = root(services, returned.call().prefix().request())?;
    held.require_live(services)?;
    let link = held
        .event()
        .ok_or_else(|| unsupported("event return has no retained resource"))?;
    if !returned.same_receipt(&link.returned) {
        return Err(unsupported(
            "event access has another actual return receipt",
        ));
    }
    let Some(Capability::SystemCollectionEvent(actual)) =
        services.capabilities.get(&link.capability)
    else {
        unreachable!("root lifetime validated actual event");
    };
    Ok(&actual.event)
}

pub struct Host<'s, 'a> {
    pub(super) services: &'s mut BootstrapServices<'a>,
    pub(super) request: collection::Request,
}
impl Host<'_, '_> {
    fn preflight(&self, call: &Call) -> Result<(), Stop> {
        if !self.request.same_call(call.prefix().request()) {
            return Err(unsupported("event host has another collection invocation"));
        }
        super::system_factory::require_collection_origin(self.services, &self.request)
    }
}
impl sims_sim::native_system::collection::event::Host for Host<'_, '_> {
    fn create_event(&mut self, call: &Call) -> Result<Return, Stop> {
        self.preflight(call)?;
        let result = create(self.services, call);
        super::system_factory::refresh_collection_images(self.services, &self.request);
        result
    }
    fn synchronize_event(&mut self, cut: &Checkpoint) -> Result<(), Stop> {
        self.preflight(cut.call())?;
        let result = synchronize(self.services, cut);
        super::system_factory::refresh_collection_images(self.services, &self.request);
        result
    }
}
