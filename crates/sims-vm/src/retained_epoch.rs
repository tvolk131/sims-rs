//! Atomic publication of a new, explicitly unpublished ordinary import epoch.
//! This consumes supplied owners; it neither authenticates a capture nor fills
//! missing World/native caller state. Existing managed Vm restores do not use it.
use crate::{
    ObjectId, Program, Snapshot, SoundEvent, SpecialEffect, Status, Vm, World,
    lifecycle::{Lifecycle, Phase},
    retained_frames::{RetainedStack, ThreadControls},
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

/// Caller declaration about replacement-adapter identities, not native bytes.
/// No handles/commands may have been published or accepted in this domain.
/// A nonempty source key identifies the caller's separately retained provenance;
/// this module does not verify the authenticity of that external evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FreshUnpublishedRequestEpoch {
    pub source_key: String,
    pub accepted_commands: u64,
    pub published_thread_handles: u64,
    pub published_request_handles: u64,
}
impl FreshUnpublishedRequestEpoch {
    fn validate(&self) -> Result<(), String> {
        if self.source_key.trim().is_empty()
            || self.source_key.len() > 4096
            || self.accepted_commands != 0
            || self.published_thread_handles != 0
            || self.published_request_handles != 0
        {
            return Err("retained ordinary import needs a fresh unpublished adapter domain".into());
        }
        Ok(())
    }
}

fn required<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}

/// Every field is an explicit input, including known absence. The supported
/// boundary has no active visit/query/named/RNG-scope continuation; supplying
/// one fails rather than clearing it. Presentation vectors retain their order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VmProjection {
    pub tick: u64,
    pub rng_state: u32,
    #[serde(deserialize_with = "required")]
    pub lot_rng_state: Option<u32>,
    #[serde(deserialize_with = "required")]
    pub crt_rng_state: Option<u32>,
    #[serde(deserialize_with = "required")]
    pub scheduled_object_visit: Option<crate::scheduler::visit::Visit>,
    #[serde(deserialize_with = "required")]
    pub player_menu: Option<crate::player_menu::Query>,
    #[serde(deserialize_with = "required")]
    pub lot_rng_scope: Option<crate::rng::LotScope>,
    pub named_calls: Vec<crate::named::Call>,
    pub native_accessory_host: bool,
    pub sounds: Vec<SoundEvent>,
    pub effects: Vec<SpecialEffect>,
}
impl VmProjection {
    fn validate_boundary(&self) -> Result<(), String> {
        if self.scheduled_object_visit.is_some()
            || self.player_menu.is_some()
            || self.lot_rng_scope.is_some()
            || !self.named_calls.is_empty()
        {
            return Err("retained ordinary epoch overlaps an unsupported VM continuation".into());
        }
        Ok(())
    }
}

/// Supplied bytes, frames and all eleven controls, before any Thread identity is
/// allocated. A Vec at the API boundary permits rejecting duplicate object keys.
#[derive(Debug, Clone, PartialEq)]
pub struct OrdinaryThreadInput {
    pub object: ObjectId,
    pub stack: RetainedStack,
    pub controls: ThreadControls,
}

fn validate_controls(object: ObjectId, controls: &ThreadControls) -> Result<(), String> {
    if controls.service.is_some()
        || controls.dispatch.is_some()
        || controls.autonomy.is_some()
        || !controls.find_best.is_empty()
        || !controls.completed_actions.is_empty()
        || controls.callback.is_some()
        || controls.callback_result.is_some()
        || controls.check
        || controls.interrupted
        || !matches!(&controls.status, Status::NativeSuspended(owner) if owner.object() == object)
    {
        return Err("retained ordinary epoch has an unsupported Thread owner/control".into());
    }
    Ok(())
}

impl Vm {
    /// Consume one supplied World and one fresh replacement-adapter domain.
    /// World.scheduler_storage is the sole canonical lifetime mapping. Thread
    /// IDs are deterministic adapter allocations, not native pointers/counters.
    ///
    /// No input is published until complete existing Snapshot validation passes.
    /// On error this owned constructor drops its uncommitted inputs; it is not
    /// a resumable native loader and does not mutate an existing Vm through alias.
    pub fn from_retained_ordinary_epoch(
        program: &Program,
        world: World,
        projection: VmProjection,
        entries: Vec<OrdinaryThreadInput>,
        epoch: FreshUnpublishedRequestEpoch,
    ) -> Result<Self, String> {
        epoch.validate()?;
        projection.validate_boundary()?;
        world
            .scheduler_storage
            .as_ref()
            .ok_or("retained ordinary epoch scheduler storage is absent")?
            .require_complete_backing(&world)?;
        world.validate_registry_order()?;
        if world.objects.is_empty() || world.objects.len() > i16::MAX as usize {
            return Err("retained ordinary epoch has invalid object extent".into());
        }
        // Dialog records retain request IDs even when their presentation is no
        // longer modal. A fresh next_request=1 cannot reuse that namespace.
        if !world.dialogs.records.is_empty() {
            return Err("retained ordinary epoch has an existing Dialog request domain".into());
        }
        let mut inputs = BTreeMap::new();
        for entry in entries {
            if entry.object <= 0 || inputs.insert(entry.object, entry).is_some() {
                return Err("retained ordinary epoch has duplicate/invalid object entries".into());
            }
        }
        if inputs.keys().ne(world.objects.keys()) {
            return Err("retained ordinary epoch Thread entries differ from World objects".into());
        }
        let mut threads = BTreeMap::new();
        let mut object_threads = BTreeMap::new();
        let mut lifecycles = BTreeMap::new();
        let mut next_thread = 1u64;
        for (object, input) in inputs {
            let state = &world.objects[&object];
            let definition = state
                .definition
                .as_ref()
                .ok_or("retained ordinary epoch object definition is absent")?;
            let words = crate::construction::definition_words(program, definition)?;
            // run_init=false describes this new adapter lifecycle, not native
            // constructor history. Group/cursor/factory Init ownership is outside
            // this narrow importer and must not be hidden behind that value.
            if state.person.is_some()
                || state.multitile.is_some()
                || state.cursor.is_some()
                || words.get(9) != Some(&4)
                || words.get(10) != Some(&0)
            {
                return Err("retained ordinary epoch requires non-group ordinary objects".into());
            }
            validate_controls(object, &input.controls)?;
            let table = program
                .modules
                .get(&definition.source)
                .and_then(|m| m.object_functions.get(&definition.object))
                .ok_or("retained ordinary epoch callback table is absent")?;
            let init = table.first().map_or(0, |f| f.action);
            let main = table.get(1).map_or(0, |f| f.action);
            if main == 0 {
                return Err("retained ordinary epoch Main is absent".into());
            }
            let thread = input.stack.into_thread(program, input.controls)?;
            crate::retained_frames::validate_world(program, &world, &thread)?;
            if thread.stack.first().is_none_or(|f| {
                f.caller != object
                    || f.callee != object
                    || f.code_owner != definition.source
                    || f.location.bhav != main
            }) {
                return Err("retained ordinary epoch Main binding differs".into());
            }
            let id = next_thread;
            next_thread = id.checked_add(1).ok_or("retained Thread IDs exhausted")?;
            threads.insert(id, thread);
            object_threads.insert(object, id);
            lifecycles.insert(
                object,
                Lifecycle {
                    owner: definition.source.clone(),
                    definition: definition.object,
                    init,
                    main,
                    phase: Phase::Main,
                    thread: id,
                    run_init: false,
                },
            );
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
                next_thread,
                next_request: 1,
            },
        )
    }
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
#[rustfmt::skip]
mod tests;
