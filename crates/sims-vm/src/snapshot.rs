use crate::{MAX_STACK, MAX_VARIABLES, Program, Status, Vm, current, dimensions};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

// One existing wire version, shared by captured snapshots and owned-state
// validation. The latter does not serialize, clone or migrate its input.
const CURRENT_VERSION: u32 = 25;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub version: u32,
    pub program_hash: String,
    pub state: Vm,
}
impl Snapshot {
    /// Validate an uncommitted owned VM through the complete current snapshot
    /// checks without capture's clone or a saved-frame/serde reconstruction.
    pub(crate) fn validate_owned(program: &Program, state: Vm) -> Result<Vm, String> {
        Self {
            version: CURRENT_VERSION,
            program_hash: program.fingerprint(),
            state,
        }
        .restore(program)
    }
    pub fn capture(program: &Program, state: &Vm) -> Self {
        Self {
            version: CURRENT_VERSION,
            program_hash: program.fingerprint(),
            state: state.clone(),
        }
    }
    pub fn restore(self, program: &Program) -> Result<Vm, String> {
        self.restore_with_person_factories(program, &[])
    }
    /// The native person factory registers an object before creating its Init/
    /// Main thread. A host snapshot must supply its saved factory continuation;
    /// standalone VM snapshots cannot claim ownership of that intermediate state.
    pub fn restore_with_person_factories(
        self,
        program: &Program,
        factories: &[&crate::person_factory::registered::Factory],
    ) -> Result<Vm, String> {
        self.restore_with_direct_person_factories(program, factories, &[])
    }
    /// Direct engine callers also own synchronous person construction, without
    /// an AwaitingCreation instruction. Their host must retain and validate the
    /// direct owner. Every supplied factory still validates against the VM;
    /// duplicate ownership is rejected, including across the two caller kinds.
    pub fn restore_with_direct_person_factories(
        self,
        program: &Program,
        factories: &[&crate::person_factory::registered::Factory],
        direct: &[&crate::person_factory::registered::Factory],
    ) -> Result<Vm, String> {
        self.restore_with_retired_creations(program, factories, direct, &[])
    }
    /// A rejected multipart factory destroys its head before its remaining
    /// parts. The simulation host must prove ownership of each still-waiting
    /// (thread, request, destroyed head) tuple with its saved destructor stack.
    /// Standalone VM restores never infer or accept these absent objects.
    pub fn restore_with_retired_creations(
        self,
        program: &Program,
        factories: &[&crate::person_factory::registered::Factory],
        direct: &[&crate::person_factory::registered::Factory],
        retired_creations: &[(crate::ThreadId, u64, crate::ObjectId)],
    ) -> Result<Vm, String> {
        self.restore_with_cursor_cleanup(program, factories, direct, retired_creations, &[])
    }
    ///42186b clears cursor pointers only after44123f returns. The host must
    ///validate its saved deletion caller before supplying these exact dangling
    ///(cursor ID, old lifetime) pairs. Standalone restores accept none.
    pub fn restore_with_cursor_cleanup(
        self,
        program: &Program,
        factories: &[&crate::person_factory::registered::Factory],
        direct: &[&crate::person_factory::registered::Factory],
        retired_creations: &[(crate::ThreadId, u64, crate::ObjectId)],
        retired_cursors: &[(crate::ObjectId, u32)],
    ) -> Result<Vm, String> {
        self.restore_with_group_construction(
            program,
            factories,
            direct,
            retired_creations,
            retired_cursors,
            &[],
        )
    }
    ///Only an exact saved440b2d construction owner permits a part whose raw
    ///origin is still zero before422872 returns or whose offsets await Reset.
    pub fn restore_with_group_construction(
        self,
        program: &Program,
        factories: &[&crate::person_factory::registered::Factory],
        direct: &[&crate::person_factory::registered::Factory],
        retired_creations: &[(crate::ThreadId, u64, crate::ObjectId)],
        retired_cursors: &[(crate::ObjectId, u32)],
        groups: &[&crate::group_factory::Construction],
    ) -> Result<Vm, String> {
        if self.version != CURRENT_VERSION {
            return Err(format!("unsupported snapshot version {}", self.version));
        }
        if self.program_hash != program.fingerprint() {
            return Err("snapshot program/content fingerprint mismatch".into());
        }
        let mut constructing_parts = BTreeSet::new();
        for group in groups {
            group.validate(&self.state, program)?;
            for &id in group.parts() {
                if !constructing_parts.insert(id) {
                    return Err("group part has duplicate construction owners".into());
                }
            }
        }
        if let Some(q) = &self.state.player_menu {
            q.validate(&self.state, program)?;
        }
        let mut factory_ids = BTreeSet::new();
        let mut before_thread = BTreeSet::new();
        let mut bound_factories = BTreeSet::new();
        let mut retired: BTreeSet<_> = retired_creations.iter().copied().collect();
        if retired.len() != retired_creations.len()
            || retired.iter().any(|&(_, _, id)| {
                id <= 0
                    || self.state.world.objects.contains_key(&id)
                    || self.state.lifecycles.contains_key(&id)
                    || self.state.object_threads.contains_key(&id)
            })
        {
            return Err("invalid retired creation host ownership".into());
        }
        for (factory, engine_call) in factories
            .iter()
            .map(|f| (f, false))
            .chain(direct.iter().map(|f| (f, true)))
        {
            factory.validate(&self.state, program)?;
            let id = factory.object();
            if !factory_ids.insert(id) {
                return Err("duplicate saved person factory owner".into());
            }
            if !self.state.lifecycles.contains_key(&id) {
                if factory.activated()
                    || factory.is_complete()
                    || self.state.object_threads.contains_key(&id)
                {
                    return Err("invalid person factory before thread creation".into());
                }
                before_thread.insert(id);
                if engine_call {
                    bound_factories.insert(id);
                }
            }
        }
        if self.state.threads.len() > 65536 || self.state.world.objects.len() > 32767 {
            return Err("snapshot world exceeds limits".into());
        }
        if self.state.next_thread == 0 || self.state.next_request == 0 {
            return Err("invalid snapshot ID counters".into());
        }
        if self.state.world.camera_rotation > 3 {
            return Err("invalid native camera rotation".into());
        }
        self.state.world.validate_scheduler_storage()?;
        self.state.world.validate_registry_order()?;
        if let Some(visit) = &self.state.scheduled_object_visit {
            visit.validate(&self.state)?;
        }
        self.state.validate_dialogs(program)?;
        if self
            .state
            .world
            .fires
            .registered
            .iter()
            .any(|id| *id <= 0 || !self.state.world.objects.contains_key(id))
        {
            return Err("saved fire registry contains an absent object".into());
        }
        self.state.world.resource_storage.validate(program)?;
        self.state.world.image_cache.validate()?;
        self.state.world.validate_indicator_queues()?;
        self.state.world.body_profiles.validate(program)?;
        for object in self.state.world.objects.values() {
            if let Some(cache) = &object.career_appearance {
                if object.person.is_none() {
                    return Err("career appearance belongs to a non-person".into());
                }
                cache.validate()?;
            }
        }
        let ui = &self.state.world.ui_owner;
        if ui.owner.is_some_and(|id| !self.state.world.objects.contains_key(&id))
            || ui.dialog_owners.len() > 4096 || ui.effects.len() > 4096
            || ui.dialog_owners.iter().any(|id| !self.state.world.objects.contains_key(id))
            || ui.effects.iter().any(|e| matches!(e, crate::ui_owner::Effect::CloseDialog(i) if *i >= ui.dialog_owners.len()))
        {
            return Err("invalid UI-event ownership state".into());
        }
        if self
            .state
            .world
            .budget
            .as_ref()
            .is_some_and(|b| b.notifications.len() > 4096)
        {
            return Err("oversized budget notification queue".into());
        }
        if let Some(constants) = &self.state.world.autonomy_constants {
            constants.validate()?;
        }
        if let Some(person) = &self.state.world.person {
            person.validate_neighbor_slots()?;
            person.validate_family_order()?;
            if person
                .nearby_houses
                .as_ref()
                .is_some_and(|h| h.len() > 32767)
            {
                return Err("oversized saved nearby-house vector".into());
            }
            let objects = &self.state.world.objects;
            let mut family_members = BTreeMap::<i16, BTreeSet<i16>>::new();
            if person
                .family_homes
                .keys()
                .any(|id| !person.families.contains_key(id))
            {
                return Err("saved home references missing family".into());
            }
            if person.families.len() > 32767
                || person.families.iter().any(|(&id, members)| {
                    id < 0
                        || members.len() > 32767
                        || members
                            .iter()
                            .any(|member| !person.neighbors.contains_key(member))
                })
            {
                return Err("invalid saved family membership".into());
            }
            // Family GUID lists and writable neighbor family words are distinct
            // native storage. 42601e clears the word while erasing only one GUID,
            // so duplicates/overlapping membership can legitimately survive.
            for (&family, members) in &person.families {
                for &member in members {
                    family_members.entry(member).or_default().insert(family);
                }
            }
            if family_members.iter().any(|(member, families)| {
                (families.len() > 1 && person.family_order.is_none())
                    || person.neighbors[member]
                        .person
                        .get(&61)
                        .is_none_or(|family| *family != 0 && !families.contains(family))
            }) {
                return Err("invalid saved family membership".into());
            }
            if !person.family_metadata.is_empty()
                && person.family_metadata.keys().ne(person.families.keys())
            {
                return Err("invalid saved family metadata ownership".into());
            }
            if person
                .family_metadata
                .values()
                .any(|m| m.member_guids.as_ref().is_some_and(|g| g.len() > 32767))
            {
                return Err("oversized saved native family GUID owner".into());
            }
            if person.neighbors.len() > 32767
                || person.tile_chains.len() > objects.len()
                || person.neighbors.iter().any(|(&id, n)| {
                    id <= 0
                        || n.person.keys().any(|&i| !(0..256).contains(&i))
                        || n.inventory.len() > 32767
                        || !crate::relationship::valid(&n.relationships)
                })
            {
                return Err("invalid saved neighborhood storage".into());
            }
            if let Some(context) = &person.autonomy {
                if context
                    .placements
                    .keys()
                    .chain(context.advertised.iter().flatten())
                    .chain(context.full_scan.iter().flatten())
                    .any(|id| !objects.contains_key(id))
                    || context
                        .selected_person
                        .is_some_and(|id| objects.get(&id).is_none_or(|o| o.person.is_none()))
                {
                    return Err("invalid saved autonomy context".into());
                }
            }
            for (&guid, order) in &person.resource_instances {
                let expected: std::collections::BTreeSet<_> = objects
                    .iter()
                    .filter_map(|(&id, o)| {
                        o.definition.as_ref().filter(|d| d.guid == guid).map(|_| id)
                    })
                    .collect();
                if order.len() != expected.len()
                    || order
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                        != expected
                {
                    return Err("invalid saved resource instance index".into());
                }
            }
            for (order, people_only) in
                [(&person.object_order, false), (&person.person_order, true)]
            {
                let mut seen = std::collections::BTreeSet::new();
                if order.iter().any(|id| {
                    !seen.insert(*id)
                        || objects
                            .get(id)
                            .is_none_or(|o| people_only && o.person.is_none())
                }) {
                    return Err("invalid saved object enumeration".into());
                }
            }
            let mut seen = std::collections::BTreeSet::new();
            if person.tile_chains.iter().any(|chain| {
                chain.is_empty()
                    || chain
                        .iter()
                        .any(|id| !objects.contains_key(id) || !seen.insert(*id))
            }) {
                return Err("invalid saved tile enumeration".into());
            }
        }
        if let Some(actions) = &self.state.world.actions {
            if actions.callbacks.len() > 65536 {
                return Err("oversized action callback queue".into());
            }
            for (&actor, state) in &actions.actors {
                if self
                    .state
                    .world
                    .objects
                    .get(&actor)
                    .is_none_or(|o| o.person.is_none())
                    || state.queue.pending.len() > crate::action::CAPACITY
                    || state.empty_action_icon != 0
                        && (state.active.is_some()
                            || !self
                                .state
                                .world
                                .objects
                                .contains_key(&state.empty_action_icon))
                {
                    return Err("invalid actor action queue".into());
                }
                for action in state
                    .queue
                    .pending
                    .iter()
                    .chain(state.active.iter())
                    .chain(state.previous.iter())
                {
                    let entry = self
                        .state
                        .action_menu(program, action.target)?
                        .iter()
                        .find(|e| e.id == action.menu_id)
                        .ok_or("saved action entry missing")?;
                    if entry.action != action.routine
                        || (action.icon_owner != 0
                            && !self.state.world.objects.contains_key(&action.icon_owner))
                    {
                        return Err("invalid saved action binding".into());
                    }
                }
            }
            for callback in &actions.callbacks {
                if callback.function != 4
                    || !self.state.world.objects.contains_key(&callback.object)
                    || !actions.actors.contains_key(&callback.stack_object)
                {
                    return Err("invalid action cancellation callback".into());
                }
            }
        }
        crate::multitile::validate_with_construction(
            &self.state.world,
            program,
            retired_cursors,
            &constructing_parts,
        )?;
        for (&id, object) in &self.state.world.objects {
            if let Some(slots) = &object.native_slot_geometry {
                if slots.len() != object.slots.len() {
                    return Err("retained containment geometry/allocation mismatch".into());
                }
                for slot in slots { slot.validate()?; }
            }
            if let Some(scores) = &object.native_motive_scores {
                if object.person.is_none() {
                    return Err("native personal score cache belongs to a nonperson".into());
                }
                scores.values()?;
            }
            if let Some(slots) = &object.routing_slots {
                if slots.len() > 256 || object.definition.is_none() {
                    return Err("invalid saved object routing slots".into());
                }
                for slot in slots {
                    slot.validate()?;
                }
            }
            if !crate::relationship::valid(&object.relationships) {
                return Err("invalid saved object relationships".into());
            }
            if object.relationship_order.is_some() {
                object.relationship_keys()?;
            }
            let mut accessories = BTreeSet::new();
            if object.accessories.len() > 256
                || (!object.accessories.is_empty() && object.person.is_none())
                || object.accessories.iter().any(|name| {
                    !accessories.insert(name) || !program.accessories.contains_key(name)
                })
            {
                return Err("invalid saved accessory appearances".into());
            }
            if let Some(h) = &object.headline {
                h.validate_resource(program)?;
                if !(0..=2).contains(&h.priority)
                    || !(0..12).contains(&h.ticks)
                    || h.last_update.is_some_and(|tick| tick > self.state.tick)
                {
                    return Err("invalid saved headline state".into());
                }
            }
            if object
                .last_update
                .is_some_and(|tick| tick > self.state.tick)
            {
                return Err("object update is in the future".into());
            }
            if id <= 0
                || object.attributes.len() > MAX_VARIABLES
                || object.slots.len() > MAX_VARIABLES
                || object.dynamic_sprite_flags.len() > 32767
            {
                return Err("invalid object storage".into());
            }
            if object
                .motives
                .is_some_and(|m| m.iter().any(|b| !f32::from_bits(*b).is_finite()))
            {
                return Err("invalid motive storage".into());
            }
            if object.motive_changes.len() > 4096
                || (!object.motive_changes.is_empty()
                    && (object.person.is_none() || object.motives.is_none()))
                || object.motive_changes.iter().any(|c| {
                    !matches!(c.motive,5..=9|14..=15)
                        || !f32::from_bits(c.delta).is_finite()
                        || !f32::from_bits(c.target).is_finite()
                })
            {
                return Err("invalid native motive changes".into());
            }
            if object.motive_update.as_ref().is_some_and(|u| {
                object.person.is_none()
                    || object.motives.is_none()
                    || u.previous.iter().any(|v| !f32::from_bits(*v).is_finite())
            }) {
                return Err("invalid person motive update storage".into());
            }
            if object
                .idle
                .as_ref()
                .is_some_and(|s| s.last_run.is_some_and(|tick| tick > self.state.tick))
            {
                return Err("idle scheduler timestamp is in the future".into());
            }
            for (slot, occupant) in object.slots.iter().enumerate() {
                if let Some(occupant) = occupant
                    && self
                        .state
                        .world
                        .objects
                        .get(occupant)
                        .and_then(|o| o.container)
                        != Some((id, slot as u16))
                {
                    return Err("dangling or inconsistent containment slot".into());
                }
            }
            if let Some((parent, slot)) = object.container
                && self
                    .state
                    .world
                    .objects
                    .get(&parent)
                    .and_then(|o| o.slots.get(slot as usize))
                    != Some(&Some(id))
            {
                return Err("inconsistent container reference".into());
            }
        }
        if let Some(refresh) = &self.state.world.refresh {
            if refresh
                .draw
                .keys()
                .chain(refresh.invalidated.iter())
                .any(|id| !self.state.world.objects.contains_key(id))
                || refresh.rooms.values().any(|r| r.category > 3)
                || refresh.rooms.len() > 65536
                || refresh
                    .redraw_rooms
                    .iter()
                    .any(|id| !refresh.rooms.contains_key(id))
            {
                return Err("invalid Refresh context".into());
            }
        }
        let mut requests: BTreeSet<_> = self
            .state
            .world
            .dialogs
            .records
            .iter()
            .map(|r| r.id)
            .collect();
        if self.state.effects.len() > 4096
            || self.state.effects.iter().any(|e| {
                e.tick > self.state.tick
                    || e.duration_ms % 1000 != 0
                    || !(-32768000..=32767000).contains(&e.duration_ms)
            })
        {
            return Err("invalid special-effect presentation queue".into());
        }
        if self.state.sounds.len() > 4096 {
            return Err("oversized sound queue".into());
        }
        for sound in &self.state.sounds {
            if sound.tick > self.state.tick
                || (!sound.stop && sound.name.is_empty())
                || (sound.stop
                    && (!sound.name.is_empty()
                        || sound.flags != 0
                        || sound.sample_rate != 0
                        || sound.volume != 0))
                || sound.name.len() > 1024
            {
                return Err("invalid queued sound event".into());
            }
        }
        for (&object, id) in &self.state.object_threads {
            let t = self
                .state
                .threads
                .get(id)
                .ok_or("registered object thread is missing")?;
            if !self.state.world.objects.contains_key(&object)
                || (t.check
                    && t.autonomy.is_none()
                    && t.find_best.is_empty()
                    && t.dispatch.is_none()
                    && !t.callback.as_ref().is_some_and(|b| {
                        b.chain().any(|b| {
                            b.external || self.state.local_named_callback(program, t, b.base_depth)
                        })
                    })
                    && self.state.lifecycles.get(&object).is_none_or(|l| {
                        l.thread != *id || l.phase != crate::lifecycle::Phase::Init
                    }))
                || t.stack.first().is_some_and(|f| f.caller != object)
            {
                return Err("invalid object thread registration".into());
            }
        }
        let mut lifecycle_threads = BTreeSet::new();
        for (&object, l) in &self.state.lifecycles {
            let definition = self
                .state
                .world
                .objects
                .get(&object)
                .and_then(|o| o.definition.as_ref())
                .ok_or("lifecycle object definition is missing")?;
            let table = program
                .modules
                .get(&l.owner)
                .and_then(|m| m.object_functions.get(&l.definition))
                .ok_or("lifecycle callback table is missing")?;
            let t = self
                .state
                .threads
                .get(&l.thread)
                .ok_or("lifecycle thread is missing")?;
            let expected_root = l.main;
            if l.phase == crate::lifecycle::Phase::Init {
                let b = t
                    .callback
                    .iter()
                    .flat_map(|b| b.chain())
                    .last()
                    .ok_or("Init callback boundary missing")?;
                if !l.run_init
                    || l.init == 0
                    || b.base_depth != 1
                    || b.resume_check
                    || b.resume_status != Status::Ready
                    || t.stack.get(1).is_none_or(|f| {
                        f.location.bhav != l.init
                            || f.code_owner != l.owner
                            || f.caller != object
                            || f.callee != object
                    })
                {
                    return Err("invalid Init callback boundary".into());
                }
            } else if t.callback.as_ref().is_some_and(|b| {
                !b.chain().any(|b| {
                    b.external || self.state.local_named_callback(program, t, b.base_depth)
                })
            }) && t.dispatch.is_none()
                && t.autonomy.is_none()
                && t.find_best.is_empty()
            {
                return Err("unexpected lifecycle callback".into());
            }
            if definition.source != l.owner
                || definition.object != l.definition
                || table.first().map_or(0, |e| e.action) != l.init
                || table.get(1).map_or(0, |e| e.action) != l.main
                || self.state.object_threads.get(&object) != Some(&l.thread)
                || !lifecycle_threads.insert(l.thread)
                || (l.phase == crate::lifecycle::Phase::Finished)
                    != matches!(
                        t.callback
                            .iter()
                            .flat_map(|b| b.chain())
                            .last()
                            .map_or(&t.status, |b| &b.resume_status),
                        Status::Returned(_)
                    )
                || (t.dispatch.is_none()
                    && t.autonomy.is_none()
                    && t.find_best.is_empty()
                    && !t.callback.as_ref().is_some_and(|b| {
                        b.chain().any(|b| {
                            b.external || self.state.local_named_callback(program, t, b.base_depth)
                        })
                    })
                    && (l.phase == crate::lifecycle::Phase::Init) != t.check)
                || !t
                    .callback
                    .iter()
                    .flat_map(|b| b.chain())
                    .any(|b| b.base_depth == 0)
                    && t.stack.first().is_some_and(|f| {
                        f.caller != object
                            || f.callee != object
                            || f.code_owner != l.owner
                            || f.location.bhav != expected_root
                    })
            {
                return Err("invalid saved lifecycle binding or phase".into());
            }
        }
        for (&id, t) in &self.state.threads {
            crate::retained_frames::validate_world(program, &self.state.world, t)?;
            if id == 0 || id >= self.state.next_thread || t.stack.len() > MAX_STACK {
                return Err("invalid thread/stack size".into());
            }
            if t.stack.is_empty() != matches!(t.status, Status::Returned(_)) {
                return Err("thread status/stack mismatch".into());
            }
            if t.frame_memory.len() > MAX_STACK * 520 {
                return Err("saved frame arena exceeds limit".into());
            }
            if !t.completed_actions.is_empty() {
                return Err("unserviced action frame returns".into());
            }
            if t.callback.is_some()
                && !lifecycle_threads.contains(&id)
                && !t.callback.as_ref().is_some_and(|b| {
                    b.chain().any(|b| {
                        b.external || self.state.local_named_callback(program, t, b.base_depth)
                    })
                })
                && t.dispatch.is_none()
                && t.autonomy.is_none()
                && t.find_best.is_empty()
            {
                return Err("unbound saved callback".into());
            }
            let mut previous_depth = 0;
            for q in &t.find_best {
                if q.depth <= previous_depth {
                    return Err("unordered Find Best continuations".into());
                }
                previous_depth = q.depth;
                q.validate(&self.state, program, t)?;
            }
            if let Some(q) = &t.autonomy {
                q.validate(&self.state, program, t)?;
            }
            if let Some(d) = &t.dispatch {
                let parent = t
                    .stack
                    .get(
                        d.parent_depth
                            .checked_sub(1)
                            .ok_or("invalid dispatch depth")?,
                    )
                    .ok_or("missing dispatch parent")?;
                let (_, routine) = program.resolve(&parent.code_owner, parent.location.bhav)?;
                let entry = self
                    .state
                    .action_menu(program, d.action.target)?
                    .iter()
                    .find(|e| e.id == d.action.menu_id)
                    .ok_or("dispatch entry missing")?;
                let target = &self.state.world.objects[&d.action.target];
                if d.actor != parent.caller
                    || routine
                        .body
                        .instructions
                        .get(parent.location.instruction)
                        .is_none_or(|i| i.opcode != 17)
                    || self
                        .state
                        .world
                        .actions
                        .as_ref()
                        .and_then(|a| a.actors.get(&d.actor))
                        .is_none()
                    || entry.check != d.check
                    || entry.action != d.action.routine
                    || target
                        .definition
                        .as_ref()
                        .is_none_or(|def| def.source != d.owner)
                {
                    return Err("invalid dispatch callback binding".into());
                }
                if let Some(b) = &t.callback {
                    let check = t
                        .stack
                        .get(d.parent_depth)
                        .ok_or("missing eligibility frame")?;
                    if b.base_depth != d.parent_depth
                        || b.resume_check
                        || b.resume_status != Status::Ready
                        || !t.check
                        || check.code_owner != d.owner
                        || check.location.bhav != d.check
                        || check.caller != d.actor
                        || check.callee != d.action.target
                    {
                        return Err("invalid eligibility callback boundary".into());
                    }
                } else if t.stack.len() != d.parent_depth || t.callback_result.is_none() || t.check
                {
                    return Err("invalid completed eligibility callback".into());
                }
            }
            let mut offset = 0;
            let mut boundary_depth = t.stack.len();
            for b in t.callback.iter().flat_map(|b| b.chain()) {
                // Placement/removal callbacks can run on a registered object
                // after its Main has returned. There is then a sentinel and
                // callback frame, but no suspended ordinary frame beneath it.
                // The spatial host validates the exact function and owner.
                let finished_object_callback = b.external
                    && b.parent.is_none()
                    && !b.resume_check
                    && matches!(b.resume_status, Status::Returned(_))
                    && t.stack.first().is_some_and(|f| {
                        f.caller == f.callee
                            && self.state.object_threads.get(&f.caller) == Some(&id)
                            && self.state.lifecycles.get(&f.caller).is_some_and(|l| {
                                l.phase == crate::lifecycle::Phase::Finished
                                    && l.owner == f.code_owner
                                    && program.modules[&l.owner].object_functions[&l.definition]
                                        .iter()
                                        .any(|function| function.action == f.location.bhav)
                            })
                    });
                if b.base_depth == 0
                    && !self.state.owns_named_callback(id, 0)
                    && !finished_object_callback
                    || b.base_depth >= boundary_depth
                    || !t.check
                {
                    return Err("invalid nested callback boundary".into());
                }
                boundary_depth = b.base_depth;
                if b.parent.is_some()
                    && b.external
                    && !self.state.owns_named_callback(id, b.base_depth)
                {
                    return Err("nested external callback is unsupported".into());
                }
                if b.parent.is_some()
                    && !b.external
                    && !t.find_best.iter().any(|q| q.depth == b.base_depth)
                {
                    let parent = &t.stack[b.base_depth - 1];
                    let (_, r) = program.resolve(&parent.code_owner, parent.location.bhav)?;
                    let ins = &r.body.instructions[parent.location.instruction];
                    if ins.opcode != 28
                        || ![0, 1].contains(&ins.operands[5])
                        || parent.continuation != 2
                    {
                        return Err("unbound synchronous named callback".into());
                    }
                    let Some((owner, bhav)) =
                        self.state
                            .named_tree_binding(program, parent, &ins.operands)?
                    else {
                        return Err("missing named callback resource".into());
                    };
                    let child = &t.stack[b.base_depth];
                    if child.code_owner != owner
                        || child.location.bhav != bhav
                        || child.caller != parent.caller
                        || child.callee != parent.stack_object
                    {
                        return Err("invalid named callback binding".into());
                    }
                }
            }
            let mut service_depth = t.stack.len() + 1;
            for service in t.service.iter().flat_map(|s| s.chain()) {
                if service.base_depth >= service_depth {
                    return Err("unordered service call boundaries".into());
                }
                service_depth = service.base_depth;

                if service.base_depth == 0
                    || service.base_depth > t.stack.len()
                    || !matches!(
                        service.resume_status,
                        Status::Ready | Status::Sleeping { .. } | Status::AwaitingRoute(_)
                    )
                    || (t.check && t.callback.is_none())
                    || t.dispatch.is_some()
                {
                    return Err("invalid service call boundary".into());
                }
                if service.result.is_some() {
                    if service.base_depth != t.stack.len() {
                        return Err("completed service has child frames".into());
                    }
                } else {
                    let child = t
                        .stack
                        .get(service.base_depth)
                        .ok_or("service frame missing")?;
                    if child.code_owner != service.owner
                        || child.location.bhav != service.bhav
                        || child.caller != service.caller
                        || child.callee != service.callee
                        || child
                            .action_target
                            .is_some_and(|target| target != service.callee)
                    {
                        return Err("invalid service frame binding".into());
                    }
                    if let Status::AwaitingRoute(request) = &service.resume_status {
                        let mut parent = t.clone();
                        parent.stack.truncate(service.base_depth);
                        let ins = current(program, &parent)?;
                        if request.id == 0
                            || request.id >= self.state.next_request
                            || !matches!(ins.opcode, 0x1b | 0x2d)
                            || !requests.insert(request.id)
                            || self.state.describe_route(
                                program,
                                &parent,
                                &ins.operands,
                                request.id,
                            )? != *request
                        {
                            return Err("invalid suspended route service request".into());
                        }
                    }
                }
            }
            for (depth, f) in t.stack.iter().enumerate() {
                if t.callback
                    .as_ref()
                    .is_some_and(|b| b.chain().any(|b| b.base_depth == depth))
                {
                    if t.frame_memory.get(offset..offset + 8)
                        != Some(&[
                            Some(-1),
                            Some(0),
                            Some(0),
                            Some(0),
                            Some(1),
                            Some(0),
                            Some(0),
                            Some(0),
                        ])
                    {
                        return Err("invalid callback sentinel storage".into());
                    }
                    offset += 8;
                }
                let (source, routine) = program.resolve(&f.code_owner, f.location.bhav)?;
                let (argc, locals) = dimensions(routine)?;
                if source != f.location.source
                    || f.location.instruction >= routine.body.instructions.len()
                    || f.args.len() != argc
                    || f.locals.len() != locals
                    || f.opaque_locals
                        .iter()
                        .any(|i| *i >= locals || f.locals[*i] != 0)
                    || f.location.instruction > 0xfa
                    || f.storage_offset != offset
                    || f.storage_offset + super::frame_words(argc, locals) > t.frame_memory.len()
                    || (depth == 0 && f.action_target.is_some())
                {
                    return Err("invalid saved frame".into());
                }
                offset += super::frame_words(argc, locals);
                let walking_route = matches!(routine.body.instructions[f.location.instruction].opcode, 27 | 45)
                    && matches!(f.continuation, 3 | 9 | 10 | 11 | 15)
                    && ((depth + 1 == t.stack.len()
                        && matches!(&t.status, Status::AwaitingRoute(r) if r.origin == f.location))
                        || t.callback.iter().flat_map(|b| b.chain()).any(|b| {
                            b.base_depth == depth + 1 && !b.resume_check
                                && matches!(&b.resume_status, Status::AwaitingRoute(r) if r.origin == f.location)
                        })
                        || t.service.iter().flat_map(|s| s.chain()).any(|s| {
                            s.base_depth == depth + 1
                                && matches!(&s.resume_status, Status::AwaitingRoute(r) if r.origin == f.location)
                        }));
                // Original 0x45ad63 state 2 retains the outer route while its
                // yielding portal tree runs. A raw continuation alone is not
                // authority: a suspended service or the returned leaf's
                // matching request must still own the route.
                let portal_route = matches!(routine.body.instructions[f.location.instruction].opcode, 27 | 45)
                    && f.continuation == 2
                    && ((depth + 1 == t.stack.len()
                        && matches!(&t.status, Status::AwaitingRoute(r) if r.origin == f.location))
                        || t.service.iter().flat_map(|s| s.chain()).any(|s| {
                        s.base_depth == depth + 1
                            && matches!(&s.resume_status, Status::AwaitingRoute(r) if r.origin == f.location)
                    }));
                if f.continuation != 0
                    && !walking_route
                    && !portal_route
                    && !(routine.body.instructions[f.location.instruction].opcode == 47
                        && f.continuation <= 3)
                    && !(routine.body.instructions[f.location.instruction].opcode == 35
                        && [1, 2].contains(&f.continuation))
                    && routine.body.instructions[f.location.instruction].opcode != 17
                    && !(routine.body.instructions[f.location.instruction].opcode == 28
                        && [1, 2, 3].contains(&f.continuation))
                    && !(routine.body.instructions[f.location.instruction].opcode == 28
                        && f.continuation == 1)
                    && !(routine.body.instructions[f.location.instruction].opcode == 22
                        && f.continuation == 1
                        && ((depth + 1 == t.stack.len()
                            && matches!(t.status, Status::AwaitingTurn(_)))
                            || t.callback.iter().flat_map(|b| b.chain()).any(|b| {
                                b.base_depth == depth + 1
                                    && !b.resume_check
                                    && matches!(b.resume_status, Status::AwaitingTurn(_))
                            })))
                {
                    return Err("continuation outside a supported resumable primitive".into());
                }
                if f.action_icon
                    && (f.action_target.is_none() || depth == 0 || {
                        let parent = &t.stack[depth - 1];
                        let (_, r) = program.resolve(&parent.code_owner, parent.location.bhav)?;
                        let ins = &r.body.instructions[parent.location.instruction];
                        ins.opcode != 20 || ins.operands[2] & 1 == 0
                    })
                {
                    return Err("action icon outside a functional-tree frame".into());
                }
                if !self.state.world.objects.contains_key(&f.caller)
                    || !self.state.world.objects.contains_key(&f.callee)
                {
                    return Err("saved caller/callee is missing".into());
                }
            }
            for (depth, pair) in t.stack.windows(2).enumerate() {
                if t.service
                    .as_ref()
                    .is_some_and(|s| s.chain().any(|s| s.base_depth == depth + 1))
                {
                    continue;
                }
                if t.callback
                    .as_ref()
                    .is_some_and(|b| b.chain().any(|b| b.base_depth == depth + 1))
                {
                    continue;
                }
                let (_, parent) = program.resolve(&pair[0].code_owner, pair[0].location.bhav)?;
                let opcode = parent.body.instructions[pair[0].location.instruction].opcode;
                if opcode == 20 {
                    let ins = &parent.body.instructions[pair[0].location.instruction];
                    let target = pair[0].stack_object;
                    let object = self
                        .state
                        .world
                        .objects
                        .get(&target)
                        .ok_or("missing functional target")?;
                    let def = object
                        .definition
                        .as_ref()
                        .ok_or("missing functional definition")?;
                    let species = self.state.world.objects[&pair[0].caller]
                        .person
                        .as_ref()
                        .and_then(|p| p.get(&65))
                        .copied();
                    let bound = crate::functional::function(
                        u16::from_le_bytes([ins.operands[0], ins.operands[1]]),
                        species,
                    )
                    .and_then(|i| {
                        program
                            .modules
                            .get(&def.source)?
                            .object_functions
                            .get(&def.object)?
                            .get(i)
                    })
                    .map(|e| e.action);
                    if bound != Some(pair[1].location.bhav)
                        || pair[1].action_icon
                            != (ins.operands[2] & 1 != 0 && pair[1].action_target.is_some())
                        || pair[0].continuation != 0
                        || pair[1].code_owner != def.source
                        || pair[1].caller != pair[0].caller
                        || pair[1].callee != target
                        || pair[1].action_target
                            != self.state.world.objects[&pair[0].caller]
                                .person
                                .as_ref()
                                .map(|_| target)
                    {
                        return Err("invalid saved functional-tree dispatch".into());
                    }
                    continue;
                }
                if opcode == 28 && pair[0].continuation == 1 {
                    let ins = &parent.body.instructions[pair[0].location.instruction];
                    let binding =
                        self.state
                            .named_tree_binding(program, &pair[0], &ins.operands)?;
                    if ins.operands[5] != 2
                        || binding != Some((pair[1].code_owner.clone(), pair[1].location.bhav))
                        || pair[1].caller != pair[0].caller
                        || pair[1].callee != pair[0].stack_object
                        || pair[1].action_target
                            != self.state.world.objects[&pair[0].caller]
                                .person
                                .as_ref()
                                .map(|_| pair[0].stack_object)
                    {
                        return Err("invalid saved named-tree dispatch".into());
                    }
                    continue;
                }
                if let Some(target) = pair[1].action_target {
                    if !((opcode == 17 && pair[0].continuation == 1)
                        || (opcode == 30 && pair[0].continuation == 0))
                        || pair[1].callee != target
                        || pair[1].caller != pair[0].caller
                        || self
                            .state
                            .world
                            .actions
                            .as_ref()
                            .and_then(|s| s.actors.get(&pair[1].caller))
                            .is_none()
                        || self
                            .state
                            .world
                            .objects
                            .get(&target)
                            .and_then(|o| o.definition.as_ref())
                            .is_none_or(|d| d.source != pair[1].code_owner)
                        || !self
                            .state
                            .action_menu(program, target)?
                            .iter()
                            .any(|entry| entry.action == pair[1].location.bhav)
                    {
                        return Err("invalid dispatched action frame".into());
                    }
                    continue;
                }
                if opcode != pair[1].location.bhav
                    || pair[0].code_owner != pair[1].code_owner
                    || pair[0].caller != pair[1].caller
                    || pair[0].callee != pair[1].callee
                {
                    return Err("saved call stack does not match call instructions".into());
                }
            }
            if matches!(t.status, Status::Sleeping { .. })
                && ![0, 17, 35].contains(&current(program, t)?.opcode)
                && !(current(program, t)?.opcode == 18
                    && current(program, t)?.operands[2] & 1 == 0
                    && crate::removal::selection(
                        crate::word(&current(program, t)?.operands, 0),
                        t.stack.last().unwrap().caller,
                        t.stack.last().unwrap().stack_object,
                    )? == t.stack.last().unwrap().caller)
                && !(current(program, t)?.opcode == 0x24
                    && current(program, t)?.operands[7] & 1 == 0
                    && crate::dialog::presentation::supported(current(program, t)?.operands[5]))
                && !t.service.as_ref().is_some_and(|s| s.result.is_some())
            {
                return Err("sleep status does not match instruction".into());
            }
            if let Status::AwaitingRoute(request) = &t.status {
                let ins = current(program, t)?;
                if request.id == 0
                    || request.id >= self.state.next_request
                    || !matches!(ins.opcode, 0x1b | 0x2d)
                    || !requests.insert(request.id)
                    || t.check
                    || self
                        .state
                        .describe_route(program, t, &ins.operands, request.id)?
                        != *request
                {
                    return Err("invalid saved route request".into());
                }
            }
            if let Status::AwaitingSnap(request) = &t.status {
                let ins = current(program, t)?;
                if request.route.id == 0
                    || request.route.id >= self.state.next_request
                    || ins.opcode != 0x2e
                    || !requests.insert(request.route.id)
                    || self
                        .state
                        .describe_snap(program, t, &ins.operands, request.route.id)?
                        != *request
                {
                    return Err("invalid saved snap request".into());
                }
            }
            if let Status::AwaitingAccessory {
                request,
                origin,
                caller,
                name,
            } = &t.status
            {
                let ins = current(program, t)?;
                let f = t.stack.last().ok_or("missing accessory frame")?;
                if ins.opcode != 6
                    || ins.operands[1] == 1
                    || (!self.state.native_accessory_host && ins.operands[2] & 1 != 0)
                    || if self.state.native_accessory_host {
                        *request == 0
                            || *request >= self.state.next_request
                            || !requests.insert(*request)
                    } else {
                        *request != 0
                    }
                    || *origin != f.location
                    || *caller != f.caller
                    || self.state.accessory_label(program, t, &ins.operands)? != *name
                {
                    return Err("invalid saved accessory request".into());
                }
            }
            if let Status::AwaitingAnimation(request) = &t.status {
                let ins = current(program, t)?;
                let frame = t.stack.last().ok_or("missing animation frame")?;
                if t.check
                    || request.id == 0
                    || request.id >= self.state.next_request
                    || !requests.insert(request.id)
                    || ins.opcode != 0x2c
                    || request.origin != frame.location
                    || request.caller != frame.caller
                    || request.operands != ins.operands
                {
                    return Err("invalid saved animation request".into());
                }
            }
            if let Status::AwaitingReach(r) = &t.status {
                let ins = current(program, t)?;
                let f = t.stack.last().ok_or("missing Reach frame")?;
                if t.check
                    || r.id == 0
                    || r.id >= self.state.next_request
                    || !requests.insert(r.id)
                    || ins.opcode != 47
                    || r.origin != f.location
                    || r.caller != f.caller
                    || r.target != f.stack_object
                    || r.operands != ins.operands
                {
                    return Err("invalid saved Reach request".into());
                }
            }
            if let Status::AwaitingTransfer(request) = &t.status {
                let ins = current(program, t)?;
                let f = t.stack.last().ok_or("missing transfer frame")?;
                if request.id == 0
                    || request.id >= self.state.next_request
                    || !requests.insert(request.id)
                    || ins.opcode
                        != if request.onto.is_some() {
                            43
                        } else if request.grab {
                            4
                        } else {
                            5
                        }
                    || request.onto.is_some() && request.grab
                    || request.onto.is_some_and(|slots| {
                        crate::transfer::onto_slots(&ins.operands, &f.args) != Ok(slots)
                    })
                    || request.origin != f.location
                    || request.caller != f.caller
                    || request.target != f.stack_object
                {
                    return Err("invalid saved Grab/Drop request".into());
                }
            }
            for (status, depth) in std::iter::once((&t.status, t.stack.len()))
                .chain(
                    t.callback
                        .iter()
                        .flat_map(|b| b.chain())
                        .map(|b| (&b.resume_status, b.base_depth)),
                )
                .chain(
                    t.service
                        .iter()
                        .flat_map(|b| b.chain())
                        .map(|b| (&b.resume_status, b.base_depth)),
                )
            {
                if let Status::AwaitingDialog(r) = status {
                    let f = depth
                        .checked_sub(1)
                        .and_then(|i| t.stack.get(i))
                        .ok_or("missing saved dialog frame")?;
                    r.validate(&self.state, program, f)?;
                    if !requests.insert(r.id()) {
                        return Err("duplicate dialog construction request".into());
                    }
                }
                if let Status::AwaitingUiEffect(r) = status {
                    let f = depth
                        .checked_sub(1)
                        .and_then(|i| t.stack.get(i))
                        .ok_or("missing saved UI-effect frame")?;
                    r.validate(&self.state, program, f)?;
                    if !requests.insert(r.request().id) {
                        return Err("duplicate UI-effect request".into());
                    }
                }
                if let Status::AwaitingBurn(r) = status {
                    let f = depth
                        .checked_sub(1)
                        .and_then(|i| t.stack.get(i))
                        .ok_or("missing saved Burn frame")?;
                    let ins = program
                        .modules
                        .get(&f.location.source)
                        .and_then(|m| m.routines.get(&f.location.bhav))
                        .and_then(|r| r.body.instructions.get(f.location.instruction))
                        .ok_or("saved Burn instruction absent")?;
                    if r.id == 0
                        || r.id >= self.state.next_request
                        || !requests.insert(r.id)
                        || ins.opcode != 9
                        || r.origin != f.location
                        || r.caller != f.caller
                        || r.target != f.stack_object
                        || r.operands != ins.operands
                    {
                        return Err("invalid saved Burn request".into());
                    }
                }
            }
            if let Status::AwaitingRemoval(r) = &t.status {
                let ins = current(program, t)?;
                let f = t.stack.last().ok_or("missing removal frame")?;
                if r.id == 0
                    || r.id >= self.state.next_request
                    || !requests.insert(r.id)
                    || ins.opcode != 0x12
                    || r.origin != f.location
                    || r.caller != f.caller
                    || r.target
                        != crate::removal::selection(
                            crate::word(&ins.operands, 0),
                            f.caller,
                            f.stack_object,
                        )?
                    || !self.state.world.objects.contains_key(&r.target)
                    || r.flags != ins.operands[2]
                {
                    return Err("invalid saved removal request".into());
                }
            }
            if let Status::AwaitingPlacement(request) = &t.status {
                let ins = current(program, t)?;
                if t.check
                    || request.id == 0
                    || request.id >= self.state.next_request
                    || !requests.insert(request.id)
                    || ins.opcode != 0x10
                    || self
                        .state
                        .describe_find_location(t, &ins.operands, request.id)?
                        != *request
                {
                    return Err("invalid saved Find Location request".into());
                }
            }
            for (status, depth) in std::iter::once((&t.status, t.stack.len())).chain(
                t.callback
                    .iter()
                    .flat_map(|b| b.chain())
                    .map(|b| (&b.resume_status, b.base_depth)),
            ) {
                if let Status::AwaitingImmediate(request) = status {
                    let mut parent = t.clone();
                    parent.stack.truncate(depth);
                    parent.status = Status::Ready;
                    let ins = current(program, &parent)?;
                    if request.id == 0
                        || request.id >= self.state.next_request
                        || !requests.insert(request.id)
                        || ins.opcode != 13
                    {
                        return Err("invalid saved immediate request".into());
                    }
                    let bytes = ins.operands;
                    let mut vm = self.state.clone();
                    vm.next_request = request.id;
                    vm.push_interaction(program, &mut parent, &bytes)?;
                    if parent.status != *status {
                        return Err("immediate request operand binding mismatch".into());
                    }
                }
            }
            if let Status::AwaitingCreation(request) = &t.status {
                let ins = current(program, t)?;
                let frame = t.stack.last().ok_or("missing creation frame")?;
                if request.id == 0
                    || request.id >= self.state.next_request
                    || !requests.insert(request.id)
                    || ins.opcode != 0x2a
                    || request.origin != frame.location
                    || request.caller != frame.caller
                    || request.target != frame.stack_object
                    || request.operands != ins.operands
                    || request.created.is_some_and(|object| {
                        object == request.caller
                            || (!retired.remove(&(id, request.id, object))
                                && !self.state.lifecycles.contains_key(&object)
                                && (!before_thread.contains(&object)
                                    || !bound_factories.insert(object)))
                    })
                {
                    return Err("invalid saved creation request".into());
                }
            }
            for (status, depth, check) in std::iter::once((&t.status, t.stack.len(), t.check))
                .chain(
                    t.callback
                        .iter()
                        .flat_map(|b| b.chain())
                        .map(|b| (&b.resume_status, b.base_depth, b.resume_check)),
                )
            {
                let Status::AwaitingTurn(request) = status else {
                    continue;
                };
                let frame = depth
                    .checked_sub(1)
                    .and_then(|i| t.stack.get(i))
                    .ok_or("missing turn frame")?;
                let (_, routine) = program.resolve(&frame.code_owner, frame.location.bhav)?;
                let ins = &routine.body.instructions[frame.location.instruction];
                if check
                    || request.id == 0
                    || request.id >= self.state.next_request
                    || !requests.insert(request.id)
                    || ins.opcode != 0x16
                    || request.origin != frame.location
                    || request.caller != frame.caller
                    || request.target != frame.stack_object
                    || !(1..=3).contains(&request.mode)
                    || request.mode != i16::from_le_bytes(ins.operands[..2].try_into().unwrap())
                    || frame.continuation != 1
                {
                    return Err("invalid saved turn request".into());
                }
            }
        }
        if !retired.is_empty() {
            return Err("unbound retired creation host ownership".into());
        }
        if bound_factories != before_thread {
            return Err("unbound saved person factory before thread creation".into());
        }
        self.state.validate_named_calls(program)?;
        crate::registry_suspension::validate_vm(program, &self.state)?;
        crate::native_registry_reentry::validate_vm(program, &self.state)?;
        Ok(self.state)
    }
    pub fn from_json(bytes: &[u8], program: &Program) -> Result<Vm, String> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("snapshot exceeds 16 MiB limit".into());
        }
        serde_json::from_slice::<Self>(bytes)
            .map_err(|e| e.to_string())?
            .restore(program)
    }
}
