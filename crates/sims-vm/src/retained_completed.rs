//! Owned assembly of a separately supplied completed-tick projection.
//!
//! This is not the narrow C0/ordinary importer, a native observer, or a loader.
//! The caller supplies the complete World, every VM/Thread owner, actual
//! lifecycle fields and explicit adapter associations. Source keys describe
//! external provenance; this module cannot authenticate that provenance. Raw
//! observations which do not establish these inputs must remain a first demand.
//! No constructors, callbacks, Init, Main, RNG or scheduler execution occurs.
//! Explicit unknown RNG values remain unknown. Admission as an authenticated
//! Complete Collection native baseline separately requires observed global,
//! lot and CRT streams/ownership; structural assembly grants no such credit.
use crate::{
    ObjectId, Program, Snapshot, Status, ThreadId, Vm, World,
    lifecycle::Lifecycle,
    retained_epoch::{FreshUnpublishedRequestEpoch, VmProjection},
    retained_frames::{RetainedStack, ThreadControls},
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

fn required<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}

/// One caller-declared source owner and its unpublished replacement handle.
/// Native object IDs, action serials and World lifetimes are separate domains.
/// The input is already projected consistently; assembly does not rewrite IDs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentityMapping {
    pub source_owner: String,
    pub adapter: u64,
}

/// All replacement identities are still private. Counter gaps are permitted;
/// only retained/consumed owners need mappings, not an invented native namespace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompletedAdapterEpoch {
    pub fresh: FreshUnpublishedRequestEpoch,
    pub threads: Vec<IdentityMapping>,
    pub requests: Vec<IdentityMapping>,
    pub next_thread: u64,
    pub next_request: u64,
}

/// Complete observed stack and controls, with explicit association/absence.
/// RetainedStack must first pass its raw-arena/context validation constructor.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletedThreadInput {
    pub source_owner: String,
    pub adapter_thread: ThreadId,
    pub object: Option<ObjectId>,
    pub stack: RetainedStack,
    pub controls: ThreadControls,
}

/// Every World object needs one entry, including known absence. In particular,
/// None is not an omitted Main or a request to derive constructor history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ObjectThreadBinding {
    pub object: ObjectId,
    #[serde(deserialize_with = "required")]
    pub thread: Option<ThreadId>,
    #[serde(deserialize_with = "required")]
    pub lifecycle: Option<Lifecycle>,
}

fn key(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 4096
}

fn identities(
    values: Vec<IdentityMapping>,
    next: u64,
    domain: &str,
) -> Result<BTreeMap<u64, String>, String> {
    if next == 0 {
        return Err(format!("completed {domain} counter is zero"));
    }
    let mut ids = BTreeMap::new();
    let mut sources = BTreeSet::new();
    for value in values {
        if !key(&value.source_owner)
            || value.adapter == 0
            || value.adapter >= next
            || !sources.insert(value.source_owner.clone())
            || ids.insert(value.adapter, value.source_owner).is_some()
        {
            return Err(format!(
                "duplicate/invalid completed {domain} identity mapping"
            ));
        }
    }
    Ok(ids)
}

/// Request ownership includes suspended parents as well as the current status.
/// Zero is the explicitly supported old accessory protocol, not an allocation.
fn request(status: &Status) -> Option<u64> {
    match status {
        Status::AwaitingRoute(r) => Some(r.id),
        Status::AwaitingSnap(r) => Some(r.route.id),
        Status::AwaitingAnimation(r) => Some(r.id),
        Status::AwaitingAccessory { request, .. } => (*request != 0).then_some(*request),
        Status::AwaitingTurn(r) => Some(r.id),
        Status::AwaitingReach(r) => Some(r.id),
        Status::AwaitingCreation(r) => Some(r.id),
        Status::AwaitingImmediate(r) => Some(r.id),
        Status::AwaitingTransfer(r) => Some(r.id),
        Status::AwaitingPlacement(r) => Some(r.id),
        Status::AwaitingRemoval(r) => Some(r.id),
        Status::AwaitingBurn(r) => Some(r.id),
        Status::AwaitingDialog(r) => Some(r.id()),
        Status::AwaitingUiEffect(r) => Some(r.request().id),
        Status::Ready
        | Status::Sleeping { .. }
        | Status::NativeSuspended(_)
        | Status::RegistrySuspended(_)
        | Status::NativeRegistryReentry(_)
        | Status::Returned(_)
        | Status::Faulted(_) => None,
    }
}

fn validate_callback_owners(controls: &ThreadControls) -> Result<(), String> {
    for callback in controls.callback.iter().flat_map(|b| b.chain()) {
        // Snapshot validates these parents at their retained frame/depth, not
        // just at the current leaf. Other request kinds need an explicit
        // semantic adapter before this assembly path can admit them.
        let unsupported = match &callback.resume_status {
            Status::AwaitingRoute(_) => Some("AwaitingRoute"),
            Status::AwaitingSnap(_) => Some("AwaitingSnap"),
            Status::AwaitingAnimation(_) => Some("AwaitingAnimation"),
            Status::AwaitingAccessory { .. } => Some("AwaitingAccessory"),
            Status::AwaitingReach(_) => Some("AwaitingReach"),
            Status::AwaitingCreation(_) => Some("AwaitingCreation"),
            Status::AwaitingImmediate(_) => Some("AwaitingImmediate"),
            Status::AwaitingTransfer(_) => Some("AwaitingTransfer"),
            Status::AwaitingPlacement(_) => Some("AwaitingPlacement"),
            Status::AwaitingRemoval(_) => Some("AwaitingRemoval"),
            Status::Ready
            | Status::Sleeping { .. }
            | Status::NativeSuspended(_)
            | Status::RegistrySuspended(_)
            | Status::NativeRegistryReentry(_)
            | Status::Returned(_)
            | Status::Faulted(_)
            | Status::AwaitingTurn(_)
            | Status::AwaitingDialog(_)
            | Status::AwaitingUiEffect(_)
            | Status::AwaitingBurn(_) => None,
        };
        if let Some(kind) = unsupported {
            return Err(format!(
                "unsupported completed callback-parent {kind}: full nested request semantics require an explicit validated adapter"
            ));
        }
    }
    Ok(())
}

impl Vm {
    /// Read replacement identity counters without guessing a native allocator
    /// namespace. Exhaustive decomposition keeps completed-epoch audits aware
    /// of every VM root field; counters remain validated by Snapshot restore.
    pub fn completed_delivery_counters(&self) -> (u64, u64) {
        let Self {
            terminal_observer: _,
            scheduled_object_visit: _,
            world: _,
            player_menu: _,
            native_accessory_host: _,
            tick: _,
            rng_state: _,
            lot_rng_state: _,
            lot_rng_scope: _,
            crt_rng_state: _,
            threads: _,
            object_threads: _,
            lifecycles: _,
            named_calls: _,
            sounds: _,
            effects: _,
            next_thread,
            next_request,
        } = self;
        (*next_thread, *next_request)
    }
    /// Allocate supplied replacement handles into a new unpublished VM, then
    /// run the unchanged complete Snapshot validator before returning ownership.
    /// Inputs are consumed; no externally existing VM/World is partly mutated.
    /// Unsupported host-in-flight owners fail through the existing validator.
    pub fn from_retained_completed_epoch(
        program: &Program,
        world: World,
        projection: VmProjection,
        entries: Vec<CompletedThreadInput>,
        bindings: Vec<ObjectThreadBinding>,
        epoch: CompletedAdapterEpoch,
    ) -> Result<Self, String> {
        if !key(&epoch.fresh.source_key)
            || epoch.fresh.accepted_commands != 0
            || epoch.fresh.published_thread_handles != 0
            || epoch.fresh.published_request_handles != 0
        {
            return Err("completed import needs a fresh unpublished adapter domain".into());
        }
        if projection.scheduled_object_visit.is_some() || projection.lot_rng_scope.is_some() {
            return Err("completed import overlaps an active scheduler/RNG-scope owner".into());
        }
        world
            .scheduler_storage
            .as_ref()
            .ok_or("completed scheduler storage is absent")?
            .require_complete_backing(&world)?;
        world.validate_registry_order()?;
        let thread_ids = identities(epoch.threads, epoch.next_thread, "Thread")?;
        let request_ids = identities(epoch.requests, epoch.next_request, "request")?;
        let mut inputs = BTreeMap::new();
        for input in entries {
            if thread_ids.get(&input.adapter_thread) != Some(&input.source_owner)
                || inputs.insert(input.adapter_thread, input).is_some()
            {
                return Err("duplicate/unknown completed Thread input owner".into());
            }
        }
        if inputs.keys().ne(thread_ids.keys()) {
            return Err("completed Thread identity mapping has a missing input owner".into());
        }
        let mut object_bindings = BTreeMap::new();
        for binding in bindings {
            if binding.object <= 0 || object_bindings.insert(binding.object, binding).is_some() {
                return Err("duplicate/invalid completed object binding".into());
            }
        }
        if object_bindings.keys().ne(world.objects.keys()) {
            return Err("completed object binding coverage differs from World".into());
        }
        let mut object_threads = BTreeMap::new();
        let mut lifecycles = BTreeMap::new();
        for (object, binding) in object_bindings {
            if let Some(id) = binding.thread {
                if inputs
                    .get(&id)
                    .is_none_or(|input| input.object != Some(object))
                {
                    return Err(
                        "completed object association has an unknown/different Thread owner".into(),
                    );
                }
                object_threads.insert(object, id);
            }
            if let Some(lifecycle) = binding.lifecycle {
                if binding.thread != Some(lifecycle.thread) {
                    return Err(
                        "completed lifecycle differs from supplied Thread association".into(),
                    );
                }
                lifecycles.insert(object, lifecycle);
            }
        }
        let mut threads = BTreeMap::new();
        let mut consumed_requests = BTreeSet::new();
        for (id, input) in inputs {
            if input
                .object
                .is_some_and(|object| object_threads.get(&object) != Some(&id))
            {
                return Err("completed Thread has an unknown/missing object association".into());
            }
            validate_callback_owners(&input.controls)?;
            let thread = input.stack.into_thread(program, input.controls)?;
            crate::registry_suspension::validate_epoch(
                &thread,
                &epoch.fresh.source_key,
                projection.tick,
            )?;
            crate::native_registry_reentry::validate_epoch(
                &thread,
                &epoch.fresh.source_key,
                projection.tick,
            )?;
            crate::retained_frames::validate_world(program, &world, &thread)?;
            for status in std::iter::once(&thread.status)
                .chain(
                    thread
                        .callback
                        .iter()
                        .flat_map(|b| b.chain())
                        .map(|b| &b.resume_status),
                )
                .chain(
                    thread
                        .service
                        .iter()
                        .flat_map(|b| b.chain())
                        .map(|b| &b.resume_status),
                )
            {
                consumed_requests.extend(request(status));
            }
            threads.insert(id, thread);
        }
        consumed_requests.extend(world.dialogs.records.iter().map(|r| r.id));
        consumed_requests.extend(world.dialogs.modal);
        if consumed_requests.iter().ne(request_ids.keys()) {
            return Err("completed request mapping differs from retained request owners".into());
        }
        Snapshot::validate_owned(
            program,
            Self {
                terminal_observer: Default::default(),
                world,
                scheduled_object_visit: projection.scheduled_object_visit,
                player_menu: projection.player_menu,
                native_accessory_host: projection.native_accessory_host,
                tick: projection.tick,
                rng_state: projection.rng_state,
                lot_rng_state: projection.lot_rng_state,
                lot_rng_scope: projection.lot_rng_scope,
                crt_rng_state: projection.crt_rng_state,
                threads,
                object_threads,
                lifecycles,
                named_calls: projection.named_calls,
                sounds: projection.sounds,
                effects: projection.effects,
                next_thread: epoch.next_thread,
                next_request: epoch.next_request,
            },
        )
    }
}

#[cfg(test)]
mod tests;
