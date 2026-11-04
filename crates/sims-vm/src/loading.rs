//! Native post-load decisions. These are separate from object construction:
//! a compatible stack is retained, and persons skip the ordinary Load callback.
use crate::{
    Frame, Location, ObjectId, Program, Status, Thread, ThreadId, Vm, frame_words,
    save_frame_storage,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GridHeadRepair {
    pub tile: [usize; 2],
    pub object: u16,
    /// None means the registry had no object for the signed saved ID.
    pub location: Option<[i8; 3]>,
}

/// Native +0xfc/+0xfd/+0xfe bytes from 0x4341c7. Grid column is physical Y,
/// row is physical X; arithmetic shift and byte narrowing are intentional.
pub fn coarse_position(position: crate::autonomy::Placement) -> [i8; 3] {
    [
        (position.y >> 4) as i8,
        (position.x >> 4) as i8,
        position.level as i8,
    ]
}

/// Mutating pass of 0x443cb3: only ground-floor heads are repaired. The later
/// root/containment diagnostic pass does not rebuild grids or tile chains.
pub fn repair_ground_object_heads(
    size: usize,
    heads: &mut [u16],
    world: &crate::World,
) -> Result<Vec<GridHeadRepair>, String> {
    repair_ground_object_heads_in_active(size, size, heads, world)
}

///443cb3 scans world+10, while the saved array retains its physical stride.
///Inactive backing cells are not inspected, repaired or reconstructed.
pub fn repair_ground_object_heads_in_active(
    size: usize,
    active: usize,
    heads: &mut [u16],
    world: &crate::World,
) -> Result<Vec<GridHeadRepair>, String> {
    if !(1..=64).contains(&size) || heads.len() != size * size {
        return Err("invalid original object grid dimensions".into());
    }
    if !(1..=size).contains(&active) {
        return Err("invalid active object grid dimensions".into());
    }
    let mut repairs = vec![];
    for (cell, head) in heads.iter().enumerate() {
        if cell / size >= active || cell % size >= active {
            continue;
        }
        if *head == 0 {
            continue;
        }
        let id = *head as i16;
        let location = if id > 0 && world.objects.contains_key(&id) {
            Some(coarse_position(
                *world
                    .positions
                    .get(&id)
                    .ok_or("registered object has no physical position")?,
            ))
        } else {
            None
        };
        let tile = [cell / size, cell % size];
        if location != Some([tile[0] as i8, tile[1] as i8, 1]) {
            repairs.push(GridHeadRepair {
                tile,
                object: *head,
                location,
            });
        }
    }
    for repair in &repairs {
        heads[repair.tile[0] * size + repair.tile[1]] = 0;
    }
    Ok(repairs)
}

/// Values supplied by the native game context rather than the SIMI stream.
/// Keeping them explicit also permits deterministic headless load profiles.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimulatorContext {
    pub game_mode: i8,
    pub retained_global25: i16,
    pub global30: u8,
}

/// The enclosing house loader (0x415eff) preserves installation state across
/// SIMI decoding. These values belong to the running engine, not to the save.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct HouseContext {
    pub simulator: SimulatorContext,
    pub installed_expansions: u16,
    /// Hardware tier selected by 0x413acc, exposed as global 33.
    pub performance_class: i16,
}

pub fn house_globals(saved: &[i16], context: HouseContext) -> Result<BTreeMap<i16, i16>, String> {
    let mut globals = house_resize_globals(saved, context)?;
    globals.insert(9, 0);
    Ok(globals)
}

///415eff before415e20. The saved family word remains visible to temporary
///controller Init/Main; global9 is cleared only after resize returns.
pub fn house_resize_globals(
    saved: &[i16],
    context: HouseContext,
) -> Result<BTreeMap<i16, i16>, String> {
    let mut globals = simulator_globals(saved, context.simulator)?;
    globals.insert(20, context.installed_expansions as i16);
    globals.insert(33, context.performance_class);
    globals.insert(32, 0);
    Ok(globals)
}

/// 0x46cdc6/0x46ceb9 initialize all 64 globals; 0x46cfb3 overlays the
/// versioned SIMI words, retains word 25 and refreshes the runtime fields.
pub fn simulator_globals(
    saved: &[i16],
    context: SimulatorContext,
) -> Result<BTreeMap<i16, i16>, String> {
    if !matches!(saved.len(), 32 | 64) {
        return Err("unsupported original simulator global count".into());
    }
    let mut globals: BTreeMap<_, _> = (0..64).map(|i| (i, 0)).collect();
    globals.extend(saved.iter().enumerate().map(|(i, &v)| (i as i16, v)));
    globals.insert(15, i16::from(context.game_mode));
    globals.insert(25, context.retained_global25);
    globals.insert(29, 30);
    globals.insert(30, i16::from(context.global30));
    globals.insert(16, globals[&16].clamp(0, 1000));
    if globals[&18] >= 0 {
        globals.insert(18, globals[&16]);
    }
    Ok(globals)
}

impl crate::World {
    /// House's 0x46d2c2 listener for the 0xe4 selection notification. The
    /// broadcaster's payload is narrowed directly into global 3; validation,
    /// selecting a replacement Sim and indicators belong to the sender.
    pub fn notify_person_selection(&mut self, value: i32) -> Result<(), String> {
        *self
            .globals
            .get_mut(&3)
            .ok_or("house selection global absent")? = value as i16;
        Ok(())
    }

    /// Existing-family branch of 0x428041 followed by 0x46d8dc. Moving a
    /// family to another lot has an additional property-value calculation.
    pub fn select_loaded_family(
        &mut self,
        family: i16,
        house: i16,
        funds: i32,
    ) -> Result<(), String> {
        let mode = *self.globals.get(&32).ok_or("missing loaded budget mode")?;
        let budget = self
            .budget
            .as_mut()
            .ok_or("missing loaded budget storage")?;
        if mode == 0 && budget.notifications.len() >= 4096 {
            return Err("loaded budget notification queue is full".into());
        }
        // 0x4282a2 publishes manager +0x10 before the globals/funds setters.
        // Neighbor scope 0x18 and the autonomy gate read that family handle.
        if let Some(people) = self.person.as_mut() {
            people.controlled_family = family;
        }
        self.globals.insert(9, family);
        self.globals.insert(10, house);
        self.publish_family_funds(if family == 0 { 0 } else { funds })
    }

    ///46d8dc: publish the family's retained balance, including the DA notice.
    ///Both428041 and the ordinary residential4f1ae9 tail call this setter.
    pub fn publish_family_funds(&mut self, funds: i32) -> Result<(), String> {
        let mode = *self.globals.get(&32).ok_or("missing loaded budget mode")?;
        let budget = self
            .budget
            .as_mut()
            .ok_or("missing loaded budget storage")?;
        if mode == 0 && budget.notifications.len() >= 4096 {
            return Err("loaded budget notification queue is full".into());
        }
        if mode == 0 {
            budget.balance = funds;
            self.globals.insert(14, (budget.balance / 10000) as i16);
            self.globals.insert(13, (budget.balance % 10000) as i16);
            budget.notifications.push(budget.balance);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostLoad {
    pub load_callback: bool,
    pub reset: bool,
    pub init_callback: bool,
}

/// Native neighborhood/person synchronization table, 0x423827. The neighbor's
/// person-data revision is distinct from the enclosing NBRS format version.
pub fn persistent_person_fields(revision: i32) -> Vec<i16> {
    let mut fields: Vec<_> = [
        2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 16, 20, 15, 26, 17, 18, 46, 47, 48, 49, 50, 51, 52, 53,
        13, 14, 58, 60, 61,
    ]
    .into();
    for (required, extra) in [
        (2, &[56, 57, 63][..]),
        (3, &[65][..]),
        (4, &[67, 68][..]),
        (5, &[69][..]),
        (6, &[74][..]),
        (7, &[54, 55, 75, 76][..]),
        (8, &[80, 81][..]),
        (9, &[82][..]),
        (10, &[83, 84, 85, 86][..]),
    ] {
        if revision >= required {
            fields.extend_from_slice(extra);
        }
    }
    if revision < 1 {
        fields.clear();
    }
    fields
}

pub fn attach_neighbor(
    person: &mut BTreeMap<i16, i16>,
    neighbor: &BTreeMap<i16, i16>,
    id: i16,
    revision: i32,
) -> Result<(), String> {
    let mut updates = Vec::new();
    for field in persistent_person_fields(revision) {
        updates.push((
            field,
            *neighbor
                .get(&field)
                .ok_or("missing persistent neighbor word")?,
        ));
    }
    // Family membership is copied even when the neighbor has no persistent
    // person-data revision. Native writes neighbor ID before the merge.
    updates.push((61, *neighbor.get(&61).ok_or("missing neighbor family")?));
    person.insert(31, id);
    person.extend(updates);
    Ok(())
}

#[derive(Debug, Clone)]
pub struct SavedFrame {
    pub owner: String,
    pub tree: u16,
    pub node: u16,
    pub stack_object: ObjectId,
    pub args: Vec<i16>,
    pub locals: Vec<i16>,
    pub primitive_state: u32,
}

/// Person use-list record read by 0x4592f5. Depth is one-based and identifies
/// the frame whose removal releases the use, not the current top frame.
#[derive(Debug, Clone, Copy)]
pub struct SavedUse {
    pub target: ObjectId,
    pub depth: usize,
    pub icon: bool,
}

/// In-flight base virtual reset's nonzero-function-3 branch (0x431fbe).
/// Keep this token while servicing the synchronous callback. It does not cover
/// person reset or the absent-function path's detach/reconstruct/reposition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaseCallbackReset {
    object: ObjectId,
    thread: ThreadId,
    depth: usize,
}
impl BaseCallbackReset {
    pub fn thread(&self) -> ThreadId {
        self.thread
    }
    pub fn object(&self) -> ObjectId {
        self.object
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        if self.thread != thread
            || self.depth != depth
            || vm.object_threads.get(&self.object) != Some(&thread)
        {
            return false;
        }
        let Some(t) = vm.threads.get(&thread) else {
            return false;
        };
        let Some(f) = t.stack.get(depth) else {
            return false;
        };
        let Some(d) = vm
            .world
            .objects
            .get(&self.object)
            .and_then(|o| o.definition.as_ref())
        else {
            return false;
        };
        let routine = p
            .modules
            .get(&d.source)
            .and_then(|m| m.object_functions.get(&d.object))
            .and_then(|v| v.get(3))
            .map(|f| f.action);
        t.callback
            .iter()
            .flat_map(|b| b.chain())
            .any(|b| b.external && b.base_depth == depth)
            && f.caller == self.object
            && f.callee == self.object
            && f.code_owner == d.source
            && routine == Some(f.location.bhav)
    }
    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        if !self.owns_callback(vm, p, self.thread, self.depth) {
            return Err("unbound base Reset callback".into());
        }
        Ok(())
    }

    /// No ordinary Main instructions may run between callback return and this
    /// tail. This loader stage is not an independently playable snapshot.
    pub fn finish(self, vm: &mut Vm, program: &Program) -> Result<(), String> {
        let thread = vm
            .threads
            .get(&self.thread)
            .ok_or("reset callback thread absent")?;
        if vm.object_threads.get(&self.object) != Some(&self.thread)
            || thread.callback.is_some()
            || !frozen_ordinary_status(&thread.status)
        {
            return Err("base reset callback is not ready for completion".into());
        }
        crate::native_suspension::validate(program, thread)?;
        vm.native_replacement_preflight(program, self.object, Some(self.thread))?;
        let object = vm
            .world
            .objects
            .get_mut(&self.object)
            .ok_or("reset object absent")?;
        *object.data.get_mut(&8).ok_or("reset flags absent")? &= !0x20;
        *object.data.get_mut(&62).ok_or("reset word 62 absent")? = 0;
        vm.restart_loaded_main(program, self.object)?;
        vm.finish_loaded_reset(self.object)
    }
}
impl Vm {
    /// Storage Reset (+0x40) during frozen lot activation, after virtual detach.
    /// Other saved stacks can still refer to this object; their notifications
    /// are handled by the enclosing loader. This does not resume those stacks.
    pub fn reset_loaded_storage(
        &mut self,
        p: &Program,
        object: ObjectId,
        now: crate::construction::ObjectDate,
    ) -> Result<ThreadId, String> {
        let previous = *self
            .object_threads
            .get(&object)
            .ok_or("loaded reset thread absent")?;
        self.native_replacement_preflight(p, object, Some(previous))?;
        let t = &self.threads[&previous];
        if t.callback.is_some() || t.service.is_some() || !frozen_ordinary_status(&t.status) {
            return Err("loaded storage reset requires frozen ordinary execution".into());
        }
        let q = self
            .world
            .positions
            .get(&object)
            .ok_or("loaded reset position absent")?;
        let old = self
            .world
            .objects
            .get(&object)
            .ok_or("loaded reset object absent")?
            .clone();
        if q.x != -16
            || q.y != -16
            || old.container.is_some()
            || old.data.get(&14) != Some(&-1)
            || old.data.get(&26) != Some(&0)
        {
            return Err("loaded storage reset requires completed virtual detach".into());
        }
        let last_result = t.last_result;
        let mut reset = old.clone();
        let init = reset.reset_storage(p, object, now)?;
        // This is a new explicit immediate object visit, even if this multipart
        // member was visited earlier during another member's post-load reset.
        reset.last_update = None;
        self.world.objects.insert(object, reset);
        match self.install_lifecycle(p, object, init, Some(previous)) {
            Ok(thread) => {
                self.threads.get_mut(&thread).unwrap().last_result = last_result;
                Ok(thread)
            }
            Err(e) => {
                self.world.objects.insert(object, old);
                Err(e)
            }
        }
    }
    /// Tail of base virtual reset: discard the passive scheduler countdown and
    /// release this object's UI ownership. Main itself is not restarted here.
    pub fn finish_loaded_reset(&mut self, object: ObjectId) -> Result<(), String> {
        self.validate_native_suspension_reset(object)?;
        self.world
            .objects
            .get_mut(&object)
            .ok_or("reset object absent")?
            .idle
            .as_mut()
            .ok_or("reset scheduler state absent")?
            .countdown = -1;
        // NativeSuspended has no clock deadline to repair; preserve its owner.
        if let Some(&thread) = self.object_threads.get(&object)
            && let Status::Sleeping { wake_tick } =
                &mut self.threads.get_mut(&thread).unwrap().status
        {
            *wake_tick = self.tick.checked_add(1).ok_or("reset tick exhausted")?;
        }
        self.world.ui_owner.release_if_owner(object)?;
        self.world.apply_dialog_closes()
    }
    /// Begin only the base virtual reset branch with a configured Reset tree.
    /// The caller must separately dispatch subclasses and the placement branch.
    pub fn begin_loaded_base_callback_reset(
        &mut self,
        program: &Program,
        object: ObjectId,
    ) -> Result<BaseCallbackReset, String> {
        let o = self
            .world
            .objects
            .get(&object)
            .ok_or("reset object absent")?;
        let d = o.definition.as_ref().ok_or("reset definition absent")?;
        let module = program
            .modules
            .get(&d.source)
            .ok_or("reset module absent")?;
        let kind = module
            .object_definition_words
            .get(&d.object)
            .and_then(|w| w.get(9))
            .ok_or("reset definition kind absent")?;
        if *kind == 2 || o.person.is_some() {
            return Err("person virtual reset requires its own ordering".into());
        }
        if module
            .object_functions
            .get(&d.object)
            .and_then(|t| t.get(3))
            .is_none_or(|f| f.action == 0)
        {
            return Err("reset without function 3 requires placement reconstruction".into());
        }
        if !o.data.contains_key(&8) || !o.data.contains_key(&62) || o.idle.is_none() {
            return Err("base reset storage/scheduler state absent".into());
        }
        let thread = *self
            .object_threads
            .get(&object)
            .ok_or("reset thread absent")?;
        let t = self
            .threads
            .get(&thread)
            .ok_or("reset thread storage absent")?;
        crate::native_suspension::validate(program, t)?;
        if !frozen_ordinary_status(&t.status) || t.callback.is_some() || t.service.is_some() {
            return Err("loaded base reset requires idle pre-activation execution".into());
        }
        let depth = t.stack.len();
        self.begin_loaded_function(program, object, 3)?;
        Ok(BaseCallbackReset {
            object,
            thread,
            depth,
        })
    }
    /// 0x422d46 with stack-object zero and the object's temporary argument
    /// buffer. Run this synchronously before proceeding to the next load phase;
    /// native Sleep/animation/routing yields are errors inside the callback.
    /// None represents the native false return for an absent function/tree.
    /// A missing nonzero tree still writes/pops the native sentinel; ordinary
    /// frames, execution status and the previous branch result are preserved.
    pub fn begin_loaded_function(
        &mut self,
        p: &Program,
        object: ObjectId,
        function: usize,
    ) -> Result<Option<ThreadId>, String> {
        self.begin_object_function(p, object, function, 0)
    }
    /// Native 0x422d46 with a supplied stack object and a null argument pointer
    /// (the object's temporary buffer). Collision uses function 5 with the
    /// other object as Stack Object; placement/load callbacks normally use 0.
    pub fn begin_object_function(
        &mut self,
        p: &Program,
        object: ObjectId,
        function: usize,
        stack_object: ObjectId,
    ) -> Result<Option<ThreadId>, String> {
        self.begin_object_function_with_args(p, object, function, stack_object, None)
    }
    /// Non-null argument buffers in 0x422d46 are independent of object Temps.
    /// Dynamic-update function 8 supplies four words with the neighbor mask.
    pub fn begin_object_function_with_args(
        &mut self,
        p: &Program,
        object: ObjectId,
        function: usize,
        stack_object: ObjectId,
        arguments: Option<&[i16]>,
    ) -> Result<Option<ThreadId>, String> {
        let o = self
            .world
            .objects
            .get(&object)
            .ok_or("loaded callback object absent")?;
        let d = o
            .definition
            .as_ref()
            .ok_or("loaded callback definition absent")?;
        let owner = d.source.clone();
        let table = p
            .modules
            .get(&owner)
            .and_then(|m| m.object_functions.get(&d.object))
            .ok_or("loaded callback table absent")?;
        let bhav = table.get(function).map_or(0, |f| f.action);
        if bhav == 0 {
            return Ok(None);
        }
        let thread = *self
            .object_threads
            .get(&object)
            .ok_or("loaded callback thread absent")?;
        if self.threads[&thread].callback.is_some() {
            return Err("loaded callback already active".into());
        }
        let args = arguments.unwrap_or(&o.temps).to_vec();
        if p.lookup_routine(&owner, bhav)?.is_none() {
            crate::lifecycle::write_callback_sentinel(self.threads.get_mut(&thread).unwrap())?;
            return Ok(None);
        }
        self.begin_immediate_callback(
            p,
            thread,
            crate::Start {
                owner: &owner,
                bhav,
                caller: object,
                callee: object,
                stack_object,
                args,
                check: true,
            },
        )?;
        Ok(Some(thread))
    }
    /// Restore serialized ordinary frames for load callbacks. No frame runs
    /// until the caller advances the VM. Changed signatures may have stale PCs
    /// and dimensions: native reset must replace those before live execution.
    /// Person action frames use `restore_object_stack_with_uses` instead.
    pub fn restore_object_stack(
        &mut self,
        p: &Program,
        object: ObjectId,
        frames: &[SavedFrame],
        arena_bytes: usize,
        changed: bool,
    ) -> Result<ThreadId, String> {
        self.restore_object_stack_with_uses(p, object, frames, arena_bytes, changed, &[])
    }

    /// Deserialization only: restore use ownership without dispatching actions,
    /// incrementing target counts or clearing continuous motive changes. Those
    /// fields have already been read by the native object/person codecs.
    pub fn restore_object_stack_with_uses(
        &mut self,
        p: &Program,
        object: ObjectId,
        frames: &[SavedFrame],
        arena_bytes: usize,
        changed: bool,
        uses: &[SavedUse],
    ) -> Result<ThreadId, String> {
        if self.object_threads.contains_key(&object)
            || frames.is_empty()
            || frames.len() > crate::MAX_STACK
            || !(4..=1_048_576).contains(&arena_bytes)
            || arena_bytes % 2 != 0
        {
            return Err("invalid original object frame arena".into());
        }
        let def = self
            .world
            .objects
            .get(&object)
            .and_then(|o| o.definition.as_ref())
            .ok_or("loaded stack has no object definition")?
            .clone();
        let callbacks = p
            .modules
            .get(&def.source)
            .and_then(|m| m.object_functions.get(&def.object))
            .ok_or("loaded stack callback table missing")?;
        let mut stack = Vec::new();
        let mut offset = 0;
        let mut use_by_depth = BTreeMap::new();
        let mut previous_depth = 0;
        for record in uses {
            if record.depth < 2
                || record.depth > frames.len()
                || record.depth <= previous_depth
                || !self.world.objects.contains_key(&record.target)
                || self.world.objects[&object].person.is_none()
            {
                return Err("invalid original action-use frame record".into());
            }
            use_by_depth.insert(record.depth, record);
            previous_depth = record.depth;
        }
        let mut callee = object;
        for (index, f) in frames.iter().enumerate() {
            if f.tree == 0xffff || f.args.len() > 255 || f.locals.len() > 255 {
                return Err(
                    "original sentinel/primitive continuation requires a dedicated restore service"
                        .into(),
                );
            }
            if f.primitive_state != 0 {
                // Native 0x474694 retains the continuation word verbatim. At
                // present only suspended Idle For Input over an action child
                // has a complete import contract; other states stay explicit.
                let (_, routine) = p.resolve(&f.owner, f.tree)?;
                if f.primitive_state != 1
                    || routine
                        .body
                        .instructions
                        .get(usize::from(f.node))
                        .is_none_or(|i| i.opcode != 17)
                    || !use_by_depth.contains_key(&(index + 2))
                {
                    return Err(
                        "original primitive continuation requires a dedicated restore service"
                            .into(),
                    );
                }
            }
            let source = if changed {
                // The very reason for a signature reset can be a removed tree.
                // Its saved frame is only storage beneath Load's sentinel; it
                // must be replaced before ordinary execution is permitted.
                if f.tree < 4096 {
                    p.globals.as_str()
                } else if f.tree < 8192 {
                    f.owner.as_str()
                } else {
                    p.modules
                        .get(&f.owner)
                        .and_then(|m| m.semiglobal.as_deref())
                        .ok_or("saved semiglobal context missing")?
                }
            } else {
                let (source, routine) = p.resolve(&f.owner, f.tree)?;
                if crate::dimensions(routine)? != (f.args.len(), f.locals.len())
                    || usize::from(f.node) >= routine.body.instructions.len()
                {
                    return Err(format!(
                        "compatible original frame dimensions/PC disagree: {}:{}",
                        f.owner, f.tree
                    ));
                }
                source
            };
            let action_use = use_by_depth.get(&(index + 1));
            if let Some(record) = action_use {
                callee = record.target;
            }
            stack.push(Frame {
                continuation: f.primitive_state,
                action_target: action_use.map(|record| record.target),
                action_icon: action_use.is_some_and(|record| record.icon),
                code_owner: f.owner.clone(),
                location: Location {
                    source: source.into(),
                    bhav: f.tree,
                    instruction: f.node.into(),
                },
                caller: object,
                callee,
                stack_object: f.stack_object,
                args: f.args.clone(),
                locals: f.locals.clone(),
                opaque_locals: Default::default(),
                storage_offset: offset,
            });
            offset += frame_words(f.args.len(), f.locals.len());
        }
        if offset > arena_bytes / 2 {
            return Err("original frames exceed saved arena allocation".into());
        }
        // 0x474fc3 initializes only the first dword before deserializing frames.
        // Unserialized heap words must not become fabricated zero-valued locals.
        let mut t = Thread {
            service: None,
            dispatch: None,
            autonomy: None,
            find_best: vec![],
            completed_actions: vec![],
            last_result: false,
            callback: None,
            callback_result: None,
            stack,
            frame_memory: vec![None; arena_bytes / 2],
            retained_frame_arena: None,
            check: false,
            interrupted: false,
            status: Status::Ready,
        };
        t.frame_memory[..2].fill(Some(0));
        save_frame_storage(&mut t)?;
        let id = self.next_thread;
        self.next_thread = id.checked_add(1).ok_or("thread ID exhausted")?;
        self.threads.insert(id, t);
        self.object_threads.insert(object, id);
        self.lifecycles.insert(
            object,
            crate::lifecycle::Lifecycle {
                owner: def.source,
                definition: def.object,
                init: callbacks.first().map_or(0, |f| f.action),
                main: callbacks.get(1).map_or(0, |f| f.action),
                phase: crate::lifecycle::Phase::Main,
                thread: id,
                run_init: false,
            },
        );
        Ok(id)
    }
    /// Virtual reset's Main replacement. Retains object/person storage and the
    /// native frame arena; synchronous Reset/Init callbacks are separate steps.
    pub fn restart_loaded_main(
        &mut self,
        p: &Program,
        object: ObjectId,
    ) -> Result<ThreadId, String> {
        let previous = self
            .object_threads
            .get(&object)
            .copied()
            .ok_or("loaded object thread missing")?;
        // 0x47480c replaces frames and clears +0x28, but retains the result
        // byte (+0x20), which a preceding Reset callback may have changed.
        self.native_replacement_preflight(p, object, Some(previous))?;
        let result = self.threads[&previous].last_result;
        let id = self.install_lifecycle(p, object, false, Some(previous))?;
        self.threads.get_mut(&id).unwrap().last_result = result;
        Ok(id)
    }
}
/// A loader reset supersedes an ordinary scheduled/finished Main as well as
/// a deserialized Ready stack. Native reset does not wait for its countdown:
/// multipart resets and newly constructed controllers can already be sleeping.
/// A host primitive, callback or fault is not an ordinary frozen boundary.
fn frozen_ordinary_status(status: &Status) -> bool {
    matches!(
        status,
        Status::Ready
            | Status::Sleeping { .. }
            | Status::NativeSuspended(_)
            | Status::RegistrySuspended(_)
        | Status::NativeRegistryReentry(_)
            | Status::Returned(_)
    )
}
impl PostLoad {
    /// Original cXObject virtual +0x44 (0x431e2e), after placement/version
    /// conversion. The reset step first notifies other persons, then invokes
    /// virtual +0x38(0). That is not a direct call to storage Reset (+0x40).
    /// `reset_changed` is native DAT_006c2982 (enabled in our capture).
    pub fn new(kind: u16, changed: bool, reset_changed: bool, placeholder: bool) -> Self {
        let changed = changed && reset_changed;
        Self {
            load_callback: kind != 2,
            reset: changed || placeholder,
            init_callback: changed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Definition, Module, Object, Routine, World};
    use sims_formats::{
        behavior::{Behavior, Instruction},
        execution::ObjectFunction,
    };
    fn fixture() -> (Program, Vm) {
        let module = Module {
            source: "object".into(),
            object_functions: BTreeMap::from([(
                1,
                vec![
                    ObjectFunction {
                        action: 0,
                        condition: 0,
                    },
                    ObjectFunction {
                        action: 4097,
                        condition: 0,
                    },
                ],
            )]),
            routines: BTreeMap::from([(
                4097,
                Routine {
                    name: "replacement Main".into(),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 0,
                        locals: 4,
                        instructions: vec![Instruction {
                            opcode: 2,
                            on_true: 254,
                            on_false: 255,
                            operands: [0, 0, 0, 0, 0, 0, 7, 7],
                        }],
                    },
                },
            )]),
            ..Default::default()
        };
        let program = Program {
            globals: "object".into(),
            modules: BTreeMap::from([("object".into(), module)]),
            ..Default::default()
        };
        let world = World {
            objects: BTreeMap::from([(
                1,
                Object {
                    definition: Some(Definition {
                        source: "object".into(),
                        object: 1,
                        guid: 1,
                        slots: 0,
                    }),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        (program, Vm::new(world, 1))
    }
    fn old_frame() -> SavedFrame {
        SavedFrame {
            owner: "object".into(),
            tree: 4096,
            node: 99,
            stack_object: 1,
            args: vec![10, 11, 12, 13],
            locals: vec![],
            primitive_state: 0,
        }
    }
    #[test]
    fn changed_saved_tree_can_be_removed_but_its_frame_arena_survives_main_replacement() {
        let (p, mut vm) = fixture();
        let id = vm
            .restore_object_stack(&p, 1, &[old_frame()], 512, true)
            .unwrap();
        assert_eq!(
            &vm.threads[&id].frame_memory[8..12],
            &[Some(10), Some(11), Some(12), Some(13)]
        );
        assert!(
            vm.threads[&id].frame_memory[12..]
                .iter()
                .all(Option::is_none)
        );
        vm.threads.get_mut(&id).unwrap().last_result = true;
        assert_eq!(vm.restart_loaded_main(&p, 1).unwrap(), id);
        let t = &vm.threads[&id];
        assert_eq!(t.stack.len(), 1);
        assert_eq!(t.stack[0].location.bhav, 4097);
        assert_eq!(t.stack[0].location.instruction, 0);
        assert_eq!(t.stack[0].locals, [10, 11, 12, 13]);
        assert!(t.last_result);
    }
    #[test]
    fn unsupported_saved_continuations_and_invalid_compatible_frames_do_not_register() {
        let (p, mut vm) = fixture();
        assert!(
            vm.restore_object_stack(&p, 1, &[old_frame()], 512, false)
                .is_err()
        );
        let mut f = old_frame();
        f.tree = 4097;
        f.node = 0;
        f.args.clear();
        f.locals = vec![1, 2, 3, 4];
        f.primitive_state = 1;
        assert!(
            vm.restore_object_stack(&p, 1, &[f.clone()], 512, true)
                .is_err()
        );
        f.primitive_state = 0;
        assert!(
            vm.restore_object_stack(&p, 1, &[f.clone()], 16, false)
                .is_err()
        );
        assert!(vm.threads.is_empty() && vm.object_threads.is_empty());
        f.node = 100;
        assert!(
            vm.restore_object_stack(&p, 1, &[f.clone()], 512, false)
                .is_err()
        );
        f.node = 0;
        vm.restore_object_stack(&p, 1, &[f], 512, false).unwrap();
    }

    #[test]
    fn saved_action_use_restores_ownership_and_idle_without_dispatching() {
        let (mut p, mut vm) = fixture();
        let module = p.modules.get_mut("object").unwrap();
        let mut idle = module.routines[&4097].clone();
        idle.body.instructions[0].opcode = 17;
        module.routines.insert(4096, idle);
        let mut action = module.routines[&4097].clone();
        action.body.instructions[0].opcode = 4098;
        module.routines.insert(4099, action);
        module.routines.insert(4098, module.routines[&4097].clone());
        let target = vm.world.objects[&1].clone();
        vm.world.objects.insert(2, target);
        vm.world.objects.get_mut(&2).unwrap().data.insert(62, 7);
        vm.world.objects.get_mut(&1).unwrap().person = Some(Default::default());
        let frames: Vec<_> = [4096, 4099, 4098]
            .into_iter()
            .map(|tree| SavedFrame {
                owner: "object".into(),
                tree,
                node: 0,
                stack_object: 2,
                args: vec![],
                locals: vec![11, 12, 13, 14],
                primitive_state: u32::from(tree == 4096),
            })
            .collect();
        let before = vm.world.clone();
        let uses = [SavedUse {
            target: 2,
            depth: 2,
            icon: false,
        }];
        let thread = vm
            .restore_object_stack_with_uses(&p, 1, &frames, 512, false, &uses)
            .unwrap();
        assert_eq!(
            vm.world, before,
            "import must not dispatch or increment use counts"
        );
        let stack = &vm.threads[&thread].stack;
        assert_eq!(
            stack.iter().map(|f| f.callee).collect::<Vec<_>>(),
            [1, 2, 2]
        );
        assert_eq!(
            stack.iter().map(|f| f.action_target).collect::<Vec<_>>(),
            [None, Some(2), None]
        );
        assert_eq!(stack[0].continuation, 1);
        assert_eq!(vm.threads[&thread].frame_memory[4], Some(1));
        let copy: Vm = serde_json::from_slice(&serde_json::to_vec(&vm).unwrap()).unwrap();
        assert_eq!(copy, vm);
    }

    #[test]
    fn invalid_use_depth_is_rejected_before_registering_any_thread() {
        for depth in [0, 1, 3] {
            let (p, mut vm) = fixture();
            vm.world.objects.get_mut(&1).unwrap().person = Some(Default::default());
            let mut f = old_frame();
            f.tree = 4097;
            f.node = 0;
            f.args.clear();
            f.locals = vec![0; 4];
            let before = vm.clone();
            assert!(
                vm.restore_object_stack_with_uses(
                    &p,
                    1,
                    &[f],
                    512,
                    false,
                    &[SavedUse {
                        target: 1,
                        depth,
                        icon: false
                    }]
                )
                .is_err()
            );
            assert_eq!(vm, before);
        }
    }
}
