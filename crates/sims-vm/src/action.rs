//! Action records and queue decisions recovered from Sims 1.91.
//! The eight-slot queue has observable truncation rules; it is not a sorted FIFO.
use crate::{
    Object, ObjectId, Program, Thread, ThreadId, Vm, observations::ActionTerminalReason,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

pub const CAPACITY: usize = 8;
pub const PUSH_FRONT: u32 = 2;
pub const CHECKED: u32 = 4;
pub const AVAILABLE: u32 = 8;
pub const CANCELLED: u32 = 0x100;

/// 0x458319's stack buffer passed to the non-yielding eligibility tree.
/// Frame construction may copy a fifth word: the check routine ID stored next
/// to the intended four arguments. Later words contain native pointer state.
pub fn eligibility_arguments(autonomous: bool, kind: u32, third: i16, check: u16) -> Vec<i16> {
    vec![
        i16::from(autonomous),
        i16::from(kind == 1),
        if kind == 1 { third } else { 0 },
        0,
        check as i16,
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Action {
    /// Native globally allocated action serial, separate from a VM thread ID.
    pub id: u32,
    pub target: ObjectId,
    pub icon_owner: ObjectId,
    /// TTAB identifier (the string index), not the entry's position in its table.
    pub menu_id: u32,
    pub priority: i32,
    pub flags: u32,
    pub args: [i16; 4],
    pub routine: u16,
    /// Serialized native +0x24 float, retained bit-for-bit through save/resume.
    /// It is distinct from the candidate scorer's resolved attenuation.
    #[serde(default)]
    pub attenuation_bits: u32,
    /// Cached or explicit text in the native action string at +0x28. The
    /// native getter fills an empty string from TTAB and clears flag 0x80;
    /// the setter copies text and sets 0x80.
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Queue {
    pub pending: VecDeque<Action>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplaySlot {
    Active,
    Queued(usize),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Actor {
    pub queue: Queue,
    pub active: Option<Action>,
    pub previous: Option<Action>,
    /// Native +0xa54 can be written by use-frame unwinding even when the
    /// active record has a null target. Person Reset notifications still read
    /// that icon pointer. Installing the empty action clears it.
    #[serde(default)]
    pub empty_action_icon: ObjectId,
}
impl Actor {
    pub fn icon_owner(&self) -> ObjectId {
        self.active
            .as_ref()
            .map_or(self.empty_action_icon, |a| a.icon_owner)
    }
    pub fn set_icon_owner(&mut self, id: ObjectId) {
        if let Some(active) = &mut self.active {
            active.icon_owner = id;
        } else {
            self.empty_action_icon = id;
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Callback {
    pub object: ObjectId,
    pub function: u16,
    pub stack_object: ObjectId,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorldState {
    pub serial: u32,
    pub immediate: Option<(ObjectId, Action)>,
    /// Supplied lot classification until original neighborhoods are loaded.
    pub live_family: bool,
    pub actors: BTreeMap<ObjectId, Actor>,
    /// Required callbacks; the host must service these before the next instruction.
    pub callbacks: VecDeque<Callback>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dispatch {
    pub actor: ObjectId,
    pub action: Action,
    pub owner: String,
    pub check: u16,
    pub parent_depth: usize,
    pub saved_temps: [i16; 8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insertion {
    pub accepted: bool,
    /// These targets receive function 4, in this order, with the actor as stack object.
    pub cancelled: Vec<Action>,
    /// Native push-front overflow discards these without cancellation callbacks.
    pub discarded: Vec<Action>,
}
impl Queue {
    /// Caller assigns a serial before this operation, even when insertion fails.
    pub fn insert(&mut self, mut action: Action) -> Result<Insertion, String> {
        if self.pending.len() > CAPACITY {
            return Err("invalid native action queue capacity".into());
        }
        let mut result = Insertion {
            accepted: true,
            cancelled: vec![],
            discarded: vec![],
        };
        if action.flags & PUSH_FRONT != 0 {
            if action.flags & 64 != 0 {
                // Native freezes this action's display label before copying it.
                action.flags |= 128;
            }
            // 0x45d3c1 copies seven records from a temporary queue when full:
            // the new action plus only six old records. Preserve this native edge.
            if self.pending.len() == CAPACITY {
                result.discarded = self.pending.split_off(CAPACITY - 2).into();
            }
            self.pending.push_front(action);
        } else {
            let mut keep = self.pending.len();
            while keep > 0 && self.pending[keep - 1].priority < action.priority {
                keep -= 1;
            }
            result.cancelled = self.pending.split_off(keep).into();
            if self.pending.len() == CAPACITY {
                result.accepted = false;
            } else {
                self.pending.push_back(action);
            }
        }
        Ok(result)
    }

    /// Native cancellation removes every matching pending serial in queue order.
    pub fn cancel(&mut self, id: u32) -> Vec<Action> {
        let mut removed = vec![];
        self.pending.retain(|a| {
            if a.id == id {
                removed.push(a.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    /// The original person interrupt predicate (0x45d7c0). `live_family` is
    /// the host-supplied result of the original lot/family classification query.
    pub fn should_interrupt(&self, person: &Object, live_family: bool) -> Result<bool, String> {
        let data = person
            .person
            .as_ref()
            .ok_or("Idle For Input requires a person")?;
        let read = |index| {
            data.get(&index)
                .copied()
                .ok_or_else(|| format!("missing person field {index}"))
        };
        if read(71)? != 0 {
            return Ok(false);
        }
        if self.pending.front().is_some_and(|a| {
            a.priority > i32::from(data.get(&33).copied().unwrap_or(0)) || a.flags & PUSH_FRONT != 0
        }) {
            // Require storage even when a push-front flag would suffice, as native
            // always reads the current priority before testing that flag.
            read(33)?;
            return Ok(true);
        }
        if !self.pending.is_empty() {
            read(33)?;
        }
        if live_family && read(32)? < 2 {
            let m = person
                .motives
                .as_ref()
                .ok_or("missing native motive storage")?;
            // Ordered x87 comparisons treat NaN like a value below the threshold.
            if !(f32::from_bits(m[7]) >= -99.0) || !(f32::from_bits(m[9]) >= -99.0) {
                return Ok(true);
            }
            if !(f32::from_bits(m[5]) >= -99.0) && read(65)? <= 1 {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleDecision {
    Complete,
    Yield,
    Dispatch,
}

/// The primitive's decision before resource checks and frame dispatch. Negative
/// timers wrap as i16. A continuing dispatched frame only admits push-front work.
pub fn idle_decision(
    queue: &Queue,
    interrupt: bool,
    counter: &mut i16,
    flags: &mut i16,
    dispatch: i16,
    continuation: u32,
) -> IdleDecision {
    if !interrupt {
        if *counter != 0 {
            *counter = counter.wrapping_sub(1);
            return IdleDecision::Yield;
        }
        *flags &= !0x40;
    } else {
        *flags |= 0x40;
        if dispatch != 0
            && queue
                .pending
                .front()
                .is_some_and(|a| continuation != 1 || a.flags & PUSH_FRONT != 0)
        {
            return IdleDecision::Dispatch;
        }
    }
    IdleDecision::Complete
}

impl Vm {
    /// A missing function-4 tree is the native no-op. Nonempty trees need a
    /// separately scheduled synchronous callback and remain an explicit boundary.
    pub fn service_empty_action_callbacks(&mut self, program: &Program) -> Result<(), String> {
        if self.next_action_callback(program)?.is_some() {
            return Err("nonempty action-cancellation callback is unsupported".into());
        }
        Ok(())
    }
    /// Drain missing function trees and expose the next synchronous callback.
    pub fn next_action_callback(
        &mut self,
        program: &Program,
    ) -> Result<Option<(Callback, String, u16)>, String> {
        loop {
            let Some(callback) = self
                .world
                .actions
                .as_ref()
                .and_then(|s| s.callbacks.front())
                .cloned()
            else {
                return Ok(None);
            };
            let definition = self
                .world
                .objects
                .get(&callback.object)
                .and_then(|o| o.definition.as_ref())
                .ok_or("callback target definition unavailable")?;
            let table = program
                .modules
                .get(&definition.source)
                .and_then(|m| m.object_functions.get(&definition.object))
                .ok_or("callback table unavailable")?;
            if let Some(f) = table
                .get(usize::from(callback.function))
                .filter(|f| f.action != 0)
            {
                return Ok(Some((callback, definition.source.clone(), f.action)));
            }
            self.world.actions.as_mut().unwrap().callbacks.pop_front();
        }
    }
    pub fn cancel_action(
        &mut self,
        program: &Program,
        actor: ObjectId,
        id: u32,
    ) -> Result<bool, String> {
        let state = self
            .world
            .actions
            .as_mut()
            .ok_or("native action services unavailable")?
            .actors
            .get_mut(&actor)
            .ok_or("missing actor action state")?;
        if let Some(active) = state.active.as_mut().filter(|a| a.id == id) {
            active.flags |= CANCELLED;
            self.world
                .objects
                .get_mut(&actor)
                .ok_or("missing cancellation actor")?
                .person
                .as_mut()
                .ok_or("cancellation requires person")?
                .insert(33, 0);
            let world = self.world.actions.as_ref().unwrap();
            if !world.actors[&actor]
                .queue
                .should_interrupt(&self.world.objects[&actor], world.live_family)?
            {
                if let Some(entry) = self
                    .action_menu(program, actor)?
                    .iter()
                    .find(|e| e.id == 33)
                {
                    if entry.flags & 4 != 0 {
                        return Err("immediate cancellation fallback is unsupported".into());
                    }
                    let mut action = self.menu_action(actor, actor, entry, 50)?;
                    action.icon_owner = 0;
                    self.enqueue_action(program, actor, action)?;
                }
            }
            Ok(true)
        } else {
            let removed = state.queue.cancel(id);
            let result = !removed.is_empty();
            for action in removed {
                if action.target != 0 {
                    self.world
                        .actions
                        .as_mut()
                        .unwrap()
                        .callbacks
                        .push_back(Callback {
                            object: action.target,
                            function: 4,
                            stack_object: actor,
                        });
                }
            }
            Ok(result)
        }
    }

    pub(crate) fn idle_for_input(
        &mut self,
        program: &Program,
        thread: ThreadId,
        t: &mut Thread,
        bytes: &[u8; 8],
        yes: u8,
    ) -> Result<String, String> {
        if t.dispatch.is_some() && t.callback.is_none() {
            let mut dispatch = t.dispatch.take().unwrap();
            let result = t
                .callback_result
                .take()
                .ok_or("dispatch check has not returned")?;
            self.world
                .objects
                .get_mut(&dispatch.actor)
                .ok_or("missing check actor")?
                .temps = dispatch.saved_temps;
            if result {
                dispatch.action.flags |= AVAILABLE;
            }
            return self.finish_dispatch(program, thread, t, dispatch);
        }
        let f = t.stack.last().ok_or("missing idle frame")?;
        let actor = f.caller;
        let state = self
            .world
            .actions
            .as_ref()
            .ok_or("native action services unavailable")?;
        let queue = &state
            .actors
            .get(&actor)
            .ok_or("missing actor action state")?
            .queue;
        let interrupt = queue.should_interrupt(&self.world.objects[&actor], state.live_family)?;
        let index = usize::try_from(crate::word(bytes, 0)).unwrap_or(usize::MAX);
        let counter = *f
            .args
            .get(index)
            .ok_or("native error 0x08: invalid idle parameter")?;
        let continuation = f.continuation;
        let flags = *self.world.objects[&actor]
            .data
            .get(&8)
            .ok_or("missing object flags for idle")?;
        let (mut counter, mut flags) = (counter, flags);
        let decision = idle_decision(
            queue,
            interrupt,
            &mut counter,
            &mut flags,
            crate::word(bytes, 2),
            continuation,
        );
        t.stack.last_mut().unwrap().args[index] = counter;
        self.world
            .objects
            .get_mut(&actor)
            .unwrap()
            .data
            .insert(8, flags);
        match decision {
            IdleDecision::Complete => {
                t.stack.last_mut().unwrap().continuation = 0;
                t.last_result = true;
                self.branch(program, t, yes)?;
                Ok("Idle For Input completed".into())
            }
            IdleDecision::Yield => self.yield_action(t),
            IdleDecision::Dispatch => {
                if t.check {
                    return Err("dispatch inside synchronous check is unsupported".into());
                }
                let mut action = self
                    .world
                    .actions
                    .as_mut()
                    .unwrap()
                    .actors
                    .get_mut(&actor)
                    .unwrap()
                    .queue
                    .pending
                    .pop_front()
                    .unwrap();
                if action.target == 0 {
                    return self.yield_action(t);
                }
                let entry = self
                    .action_menu(program, action.target)?
                    .iter()
                    .find(|e| e.id == action.menu_id)
                    .ok_or("queued interaction table entry disappeared")?
                    .clone();
                let owner = self.world.objects[&action.target]
                    .definition
                    .as_ref()
                    .unwrap()
                    .source
                    .clone();
                action.flags = (action.flags & !AVAILABLE) | CHECKED;
                let allowed = self.action_eligibility_gate(actor, action.target, entry.flags)?;
                let mut dispatch = Dispatch {
                    actor,
                    action,
                    owner,
                    check: entry.check,
                    parent_depth: t.stack.len(),
                    saved_temps: self.world.objects[&actor].temps,
                };
                if !allowed {
                    return self.finish_dispatch(program, thread, t, dispatch);
                }
                if entry.check == 0 {
                    dispatch.action.flags |= AVAILABLE;
                    return self.finish_dispatch(program, thread, t, dispatch);
                }
                if !self.try_push_callback_start(
                    program,
                    t,
                    crate::Start {
                        owner: &dispatch.owner,
                        bhav: entry.check,
                        caller: actor,
                        callee: dispatch.action.target,
                        stack_object: dispatch.action.target,
                        args: eligibility_arguments(false, 0, 0, entry.check),
                        check: true,
                    },
                )? {
                    return self.finish_dispatch(program, thread, t, dispatch);
                }
                t.dispatch = Some(dispatch);
                Ok("Idle For Input entered original eligibility callback".into())
            }
        }
    }

    fn yield_action(&self, t: &mut Thread) -> Result<String, String> {
        if t.callback.is_some() {
            return Err("synchronous callback yielded (native error 0x44f)".into());
        }
        t.status = crate::Status::Sleeping {
            wake_tick: self.tick.checked_add(1).ok_or("tick exhausted")?,
        };
        Ok("Idle For Input yielded".into())
    }

    pub(crate) fn action_eligibility_gate(
        &mut self,
        actor: ObjectId,
        target: ObjectId,
        flags: u32,
    ) -> Result<bool, String> {
        let a = &self.world.objects[&actor];
        let person = a.person.as_ref().ok_or("missing person profile")?;
        let read = |index| {
            person
                .get(&index)
                .copied()
                .ok_or_else(|| format!("missing person field {index}"))
        };
        let object = &self.world.objects[&target];
        let object_flags = *object.data.get(&8).ok_or("missing target flags")? as u16;
        let mut allowed = object_flags & 0x200 == 0;
        if allowed
            && flags & 1 == 0
            && read(32)? != 0
            && (self.world.actions.as_ref().unwrap().live_family || flags & 32 == 0)
        {
            let category = *object.data.get(&59).ok_or("missing target category")?;
            if category == 4 || category == 7 {
                return Err("visitor action permission service is unsupported".into());
            }
            allowed = false;
        }
        let age = read(58)?;
        let species = read(65)?;
        allowed &= if age != 0 && age < 18 {
            flags & (1 << 4) == 0
        } else if species & 2 != 0 {
            flags & (1 << 10) != 0
        } else if species & 4 != 0 {
            flags & (1 << 9) != 0
        } else {
            flags & (1 << 6) == 0
        };
        self.world
            .objects
            .get_mut(&actor)
            .unwrap()
            .data
            .insert(50, 0);
        Ok(allowed)
    }

    /// Original 41f00e constructor shared by direct and queued interactions.
    /// Preserve imported/queued records at dispatch; resolve these fields only
    /// when constructing a new action from its menu entry.
    fn menu_action(
        &self,
        actor: ObjectId,
        target: ObjectId,
        entry: &crate::program::MenuEntry,
        priority: i32,
    ) -> Result<Action, String> {
        let attenuation = match entry.autonomy.attenuation_code {
            0 => entry.autonomy.attenuation,
            1 => 0.,
            code @ 2..=4 => crate::autonomy::process_attenuation(
                &self.world,
                code,
                entry.autonomy.attenuation,
                self.world.objects[&actor]
                    .person
                    .as_ref()
                    .and_then(|p| p.get(&32))
                    .copied()
                    .ok_or("missing action visitor classification")?
                    != 0,
            )?,
            _ => 0.02,
        };
        Ok(Action {
            target,
            icon_owner: target,
            menu_id: entry.id,
            priority,
            flags: u32::from(entry.flags & 256 != 0),
            routine: entry.action,
            attenuation_bits: attenuation.to_bits(),
            ..Action::default()
        })
    }

    pub(crate) fn gosub_found_action(
        &mut self,
        program: &Program,
        thread: ThreadId,
        t: &mut Thread,
        yes: u8,
        no: u8,
    ) -> Result<String, String> {
        let frame = t.stack.last().ok_or("missing found-action frame")?;
        let actor = frame.caller;
        let target = frame.stack_object;
        if target == 0 {
            return Err("Gosub Found Action has zero target (native error 0x15)".into());
        }
        let entry = if self.world.objects.contains_key(&target) {
            if let Ok(menu) = self.action_menu(program, target) {
                let id = i32::from(
                    *self.world.objects[&actor]
                        .data
                        .get(&20)
                        .ok_or("missing found action index")?,
                ) as u32;
                Some(
                    menu.iter()
                        .find(|e| e.id == id)
                        .cloned()
                        .ok_or("missing found action entry (native error 0x16)")?,
                )
            } else {
                None
            }
        } else {
            None
        };
        let Some(entry) = entry else {
            t.last_result = false;
            self.branch(program, t, no)?;
            return Ok("Gosub Found Action target/table unavailable".into());
        };
        if t.check {
            return Err("Gosub Found Action in synchronous check is unsupported".into());
        }
        let owner = self.world.objects[&target]
            .definition
            .as_ref()
            .unwrap()
            .source
            .clone();
        if entry.action == 0 {
            return Err("Gosub Found Action cannot create its action frame".into());
        }
        let _ = yes; // The ordinary parent instruction branches when the child returns.
        self.finish_dispatch(
            program,
            thread,
            t,
            Dispatch {
                actor,
                action: self.menu_action(actor, target, &entry, 2)?,
                owner,
                check: 0,
                parent_depth: t.stack.len(),
                saved_temps: self.world.objects[&actor].temps,
            },
        )
    }

    fn finish_dispatch(
        &mut self,
        program: &Program,
        thread: ThreadId,
        t: &mut Thread,
        mut dispatch: Dispatch,
    ) -> Result<String, String> {
        let actor = dispatch.actor;
        let target = dispatch.action.target;
        let found = crate::current(program, t)?.opcode == 30;
        if !found {
            if *self.world.objects[&actor].data.get(&50).unwrap_or(&0) != 0 {
                dispatch.action.flags |= 16;
            } else {
                dispatch.action.flags &= !16;
            }
        }
        if (!found && dispatch.action.flags & AVAILABLE == 0) || dispatch.action.routine == 0 {
            self.world
                .actions
                .as_mut()
                .unwrap()
                .callbacks
                .push_back(Callback {
                    object: target,
                    function: 4,
                    stack_object: actor,
                });
            return self.yield_action(t);
        }
        let (source, routine) = program.resolve(&dispatch.owner, dispatch.action.routine)?;
        let (argc, locals) = crate::dimensions(routine)?;
        let args = dispatch
            .action
            .args
            .get(..argc)
            .ok_or("action arguments beyond four words are unsupported")?
            .to_vec();
        let parent = t.stack.last().ok_or("missing idle parent")?;
        let index = if !found {
            Some(
                usize::try_from(crate::word(&crate::current(program, t)?.operands, 0))
                    .map_err(|_| "invalid idle parameter")?,
            )
        } else {
            None
        };
        let offset =
            parent.storage_offset + crate::frame_words(parent.args.len(), parent.locals.len());
        if t.stack.len() >= crate::MAX_STACK {
            return Err("call stack limit reached".into());
        }
        let use_count = self.world.objects[&target]
            .data
            .get(&62)
            .copied()
            .ok_or("missing target use count")?;
        let context = self.retained_entry_context(t, target, &dispatch.owner)?;
        crate::retained_frames::ensure_capacity(t, offset + crate::frame_words(argc, locals))?;
        crate::save_frame_storage(t)?;
        t.frame_memory.resize(
            t.frame_memory
                .len()
                .max(offset + crate::frame_words(argc, locals)),
            Some(0),
        );
        let opaque_locals = (0..locals)
            .filter(|i| t.frame_memory[offset + 8 + argc + i].is_none())
            .collect();
        let values = (0..locals)
            .map(|i| t.frame_memory[offset + 8 + argc + i].unwrap_or(0))
            .collect();
        if let Some(index) = index {
            t.stack.last_mut().unwrap().args[index] = 0;
            t.stack.last_mut().unwrap().continuation = 1;
        }
        crate::retained_frames::finish_entry(
            t,
            crate::Frame {
                continuation: 0,
                action_target: Some(target),
                action_icon: false,
                code_owner: dispatch.owner,
                location: crate::Location {
                    source: source.into(),
                    bhav: dispatch.action.routine,
                    instruction: crate::entry_pc(routine, dispatch.action.args[0])?,
                },
                caller: actor,
                callee: target,
                stack_object: target,
                args,
                locals: values,
                opaque_locals,
                storage_offset: offset,
            },
            context,
        )?;
        self.world
            .objects
            .get_mut(&target)
            .unwrap()
            .data
            .insert(62, use_count.wrapping_add(1));
        let actions = self
            .world
            .actions
            .as_mut()
            .ok_or("native action services unavailable")?;
        if dispatch.action.id == 0 {
            actions.serial = actions.serial.wrapping_add(1);
            dispatch.action.id = actions.serial;
        }
        let state = actions
            .actors
            .get_mut(&actor)
            .ok_or("missing actor action state")?;
        // 45e6cd only replaces history when the old active target is present.
        // Completion may already have moved that record into previous.
        if let Some(mut previous) = state.active.take() {
            self.terminal_observer.retire(
                actor,
                &previous,
                self.tick,
                ActionTerminalReason::Replacement,
            );
            previous.flags = (previous.flags & !32) | u32::from(t.last_result) * 32;
            state.previous = Some(previous);
        }
        let priority = dispatch.action.priority as i16;
        self.terminal_observer
            .bind(actor, &dispatch.action, thread, &t.stack);
        state.active = Some(dispatch.action);
        state.empty_action_icon = 0;
        let object = self.world.objects.get_mut(&actor).unwrap();
        // Native 0x45e6cd clears all continuous motive changes when installing
        // an active action, even if its predecessor used the same target.
        object.motive_changes.clear();
        let p = object.person.as_mut().unwrap();
        p.insert(33, priority);
        for i in [38, 39, 71, 72, 73] {
            p.insert(i, 0);
        }
        if !found {
            *object.data.get_mut(&8).unwrap() |= 0x40;
        }
        Ok(if found {
            "Gosub Found Action entered native action frame"
        } else {
            "Idle For Input dispatched native action frame"
        }
        .into())
    }

    pub(crate) fn finish_action_frames(&mut self, t: &mut Thread) -> Result<(), String> {
        self.finish_action_frames_with_reason(t, ActionTerminalReason::NormalReturn)
    }

    fn finish_action_frames_with_reason(
        &mut self,
        t: &mut Thread,
        reason: ActionTerminalReason,
    ) -> Result<(), String> {
        if t.completed_actions.is_empty() {
            return Ok(());
        }
        let actor = t.completed_actions[0].0;
        let last_result = t.completed_actions.last().unwrap().2;
        while !t.completed_actions.is_empty() {
            let (_, target, _, _, depth) = t.completed_actions[0];
            if t.stack.len() + 1 == depth {
                let icon = t
                    .completed_actions
                    .iter()
                    .find(|record| record.3)
                    .map(|record| record.1)
                    .or_else(|| {
                        t.stack
                            .iter()
                            .rev()
                            .find(|f| f.action_icon)
                            .and_then(|f| f.action_target)
                    });
                if let Some(icon) = icon {
                    if let Some(a) = self
                        .world
                        .actions
                        .as_mut()
                        .and_then(|s| s.actors.get_mut(&actor))
                    {
                        a.set_icon_owner(icon);
                    }
                }
            }
            t.completed_actions.remove(0);
            let count = self
                .world
                .objects
                .get_mut(&target)
                .ok_or("action target disappeared")?
                .data
                .get_mut(&62)
                .ok_or("missing target use count")?;
            *count = count.wrapping_sub(1);
        }
        if !t.stack.iter().any(|f| f.action_target.is_some()) {
            self.clear_person_action(actor, last_result, reason)?;
        }
        Ok(())
    }

    /// 0x4752f6 followed by person virtual +8 (0x45e5fd). Reset removes a
    /// frame without executing its return branch or changing interpreter
    /// last_result. The frame arena, including the suspended PC, survives.
    pub(crate) fn pop_person_reset_frame(&mut self, t: &mut Thread) -> Result<(), String> {
        crate::save_frame_storage(t)?;
        let frame = t.stack.pop().ok_or("person Reset popped an empty stack")?;
        crate::retained_frames::remove_frame(t, frame.storage_offset);
        if let Some(target) = frame.action_target {
            t.completed_actions.push((
                frame.caller,
                target,
                t.last_result,
                frame.action_icon,
                t.stack.len() + 1,
            ));
        }
        self.finish_action_frames_with_reason(t, ActionTerminalReason::Reset)
    }

    /// Empty-action branch of 0x45e6cd. An absent active record preserves
    /// the previous record, including its original result bit. Returns whether
    /// clearing person[73] requires the synchronous footprint host service.
    pub(crate) fn clear_person_action(
        &mut self,
        actor: ObjectId,
        last_result: bool,
        reason: ActionTerminalReason,
    ) -> Result<bool, String> {
        let state = self
            .world
            .actions
            .as_mut()
            .ok_or("missing action state")?
            .actors
            .get_mut(&actor)
            .ok_or("missing actor action state")?;
        if let Some(mut previous) = state.active.take() {
            self.terminal_observer
                .retire(actor, &previous, self.tick, reason);
            previous.flags = (previous.flags & !32) | u32::from(last_result) * 32;
            state.previous = Some(previous);
        }
        state.empty_action_icon = 0;
        let object = self.world.objects.get_mut(&actor).unwrap();
        object.motive_changes.clear();
        let person = object.person.as_mut().unwrap();
        let footprint = person.get(&73).is_some_and(|v| *v != 0);
        for i in [33, 71, 72, 73] {
            person.insert(i, 0);
        }
        Ok(footprint)
    }
    pub fn action_menu<'a>(
        &self,
        program: &'a Program,
        object: ObjectId,
    ) -> Result<&'a [crate::program::MenuEntry], String> {
        let definition = self
            .world
            .objects
            .get(&object)
            .ok_or("native error 0x17: missing interaction target")?
            .definition
            .as_ref()
            .ok_or("interaction target definition unavailable")?;
        program
            .modules
            .get(&definition.source)
            .and_then(|m| m.object_interactions.get(&definition.object))
            .map(Vec::as_slice)
            .ok_or_else(|| "native error 0x1f: interaction table unavailable".into())
    }

    /// 0x41f167: retain nonempty cached text; otherwise read the TTAB label.
    /// The star is a TTAB flag, independent of the action's own flag 0x80.
    fn resolve_action_label(&self, p: &Program, action: &mut Action) -> Result<String, String> {
        if action.label.as_deref().is_none_or(str::is_empty) {
            let entry = if action.target == 0 {
                None
            } else {
                self.action_menu(p, action.target)?
                    .iter()
                    .find(|entry| entry.id == action.menu_id)
            };
            action.label = Some(entry.map_or_else(String::new, |entry| {
                format!(
                    "{}{}",
                    if entry.flags & 128 != 0 { "*" } else { "" },
                    entry.label
                )
            }));
        }
        action.flags &= !128;
        Ok(action.label.clone().unwrap_or_default())
    }
    ///41f167 for an actual retained action. Empty text is resolved lazily and
    /// dirty bit0x80 is cleared on that same record; no queue is reconstructed.
    pub fn displayed_action_label(
        &mut self,
        p: &Program,
        actor: ObjectId,
        slot: DisplaySlot,
    ) -> Result<String, String> {
        let owner = self
            .world
            .actions
            .as_ref()
            .and_then(|s| s.actors.get(&actor))
            .ok_or("display label actor absent")?;
        let mut action = match slot {
            DisplaySlot::Active => owner.active.clone().unwrap_or_default(),
            DisplaySlot::Queued(index) => owner
                .queue
                .pending
                .get(index)
                .cloned()
                .ok_or("display label queue index absent")?,
        };
        let label = self.resolve_action_label(p, &mut action)?;
        let owner = self
            .world
            .actions
            .as_mut()
            .unwrap()
            .actors
            .get_mut(&actor)
            .unwrap();
        let target = match slot {
            DisplaySlot::Active => owner.active.as_mut(),
            DisplaySlot::Queued(index) => owner.queue.pending.get_mut(index),
        };
        if let Some(target) = target {
            target.label = action.label;
            target.flags = action.flags;
        }
        Ok(label)
    }

    pub fn enqueue_action(
        &mut self,
        program: &Program,
        actor: ObjectId,
        mut action: Action,
    ) -> Result<(u32, bool), String> {
        let state = self
            .world
            .actions
            .as_mut()
            .ok_or("native action services unavailable")?;
        state.serial = state.serial.wrapping_add(1);
        action.id = state.serial;
        // Front pushes inherit the active record, even during an immediate
        // callback. Ordinary pushes inherit only a matching immediate owner.
        if action.flags & 64 != 0 {
            let front = action.flags & PUSH_FRONT != 0;
            let source = if front {
                Some(
                    state
                        .actors
                        .get(&actor)
                        .ok_or("missing actor action state")?
                        .active
                        .clone()
                        .unwrap_or_default(),
                )
            } else {
                state
                    .immediate
                    .as_ref()
                    .filter(|(owner, _)| *owner == actor)
                    .map(|(_, action)| action.clone())
            };
            if let Some(mut source) = source {
                action.label = Some(self.resolve_action_label(program, &mut source)?);
                action.flags |= 128;
                if front
                    && let Some(active) = self
                        .world
                        .actions
                        .as_mut()
                        .unwrap()
                        .actors
                        .get_mut(&actor)
                        .unwrap()
                        .active
                        .as_mut()
                {
                    *active = source;
                }
            }
        }
        let state = self.world.actions.as_mut().unwrap();
        if action.flags & PUSH_FRONT == 0
            && let Some((owner, immediate)) = &mut state.immediate
            && *owner == actor
        {
            *immediate = action.clone();
        }
        let queue = &mut state
            .actors
            .get_mut(&actor)
            .ok_or("missing actor action state")?
            .queue;
        let result = queue.insert(action)?;
        for cancelled in result.cancelled {
            if cancelled.target != 0 {
                state.callbacks.push_back(Callback {
                    object: cancelled.target,
                    function: 4,
                    stack_object: actor,
                });
            }
        }
        Ok((state.serial, result.accepted))
    }

    pub(crate) fn push_interaction(
        &mut self,
        program: &Program,
        t: &mut Thread,
        bytes: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let f = t.stack.last().ok_or("missing Push Interaction frame")?;
        let index = usize::from(bytes[1]);
        let from_local = bytes[3] & 2 != 0;
        let values = if from_local { &f.locals } else { &f.args };
        // This first operand has the original > bound, unlike the icon operand's >=.
        if index > values.len() {
            return Err(if from_local {
                "native error 0x33: invalid push local"
            } else {
                "native error 0x08: invalid push parameter"
            }
            .into());
        }
        let target = if from_local {
            if f.opaque_locals.contains(&index) {
                return Err("Push Interaction reads opaque native local".into());
            }
            values.get(index).copied().or_else(|| {
                t.frame_memory
                    .get(f.storage_offset + 8 + f.args.len() + index)
                    .copied()
                    .flatten()
            })
        } else if index == f.args.len() {
            if f.opaque_locals.contains(&0) {
                return Err("Push Interaction reads opaque native local".into());
            }
            f.locals.first().copied().or_else(|| {
                t.frame_memory
                    .get(f.storage_offset + 8 + index)
                    .copied()
                    .flatten()
            })
        } else {
            values.get(index).copied()
        }
        .ok_or("Push Interaction reads unavailable native frame storage")?;
        if !self.world.objects.contains_key(&target) {
            return Err("native error 0x17: missing interaction target".into());
        }
        let menu = self.action_menu(program, target)?;
        let entry = menu
            .iter()
            .find(|e| e.id == u32::from(bytes[0]))
            .ok_or("native error 0x16: missing interaction identifier")?;
        let actor = f.stack_object;
        let person = self
            .world
            .objects
            .get(&actor)
            .ok_or("native error 0x15: missing push stack object")?;
        if person.person.is_none() {
            return Err("native error 0x1c: push recipient is not a person".into());
        }
        let caller = &self.world.objects[&f.caller];
        let priority = match bytes[2] {
            0 => {
                if let Some(p) = &caller.person {
                    i32::from(*p.get(&33).ok_or("missing current interaction priority")?)
                } else {
                    1
                }
            }
            1 => 100,
            2 => 2,
            3 => 50,
            _ => return Err("native error 0x2d: invalid push priority".into()),
        };
        let icon_owner = if bytes[3] & 1 != 0 {
            let index = usize::from(bytes[4]);
            let value = *f
                .locals
                .get(index)
                .ok_or("native error 0x33: invalid push icon local")?;
            if f.opaque_locals.contains(&index) {
                return Err("Push Interaction reads opaque native icon local".into());
            }
            if self.world.objects.contains_key(&value) {
                value
            } else {
                0
            }
        } else {
            target
        };
        let immediate = entry.flags & 4 != 0;
        let mut action = self.menu_action(actor, target, entry, priority)?;
        action.icon_owner = icon_owner;
        action.flags |=
            u32::from(bytes[3] & 8 != 0) * 64 | u32::from(bytes[3] & 4 != 0) * PUSH_FRONT;
        if immediate {
            t.status = crate::Status::AwaitingImmediate(crate::immediate::Request {
                id: self.next_request,
                origin: t.stack.last().unwrap().location.clone(),
                actor,
                action,
            });
            self.next_request = self
                .next_request
                .checked_add(1)
                .ok_or("request ID overflow")?;
            return Ok((false, "awaiting original immediate TTAB action".into()));
        }
        let (id, accepted) = self.enqueue_action(program, actor, action)?;
        Ok((
            accepted,
            format!("native Push Interaction #{id} on actor #{actor}: accepted={accepted}"),
        ))
    }
}

#[cfg(test)]
mod functional_return_tests {
    use super::*;
    use crate::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn native_normal_return_and_unwind_restore_icons_differently() {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/functional-return.json")).unwrap();
        let routine = Routine {
            name: "synthetic".into(),
            body: sims_formats::behavior::Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                instructions: vec![sims_formats::behavior::Instruction {
                    opcode: 0,
                    on_true: 254,
                    on_false: 255,
                    operands: [0; 8],
                }],
            },
        };
        let p = Program {
            globals: "test".into(),
            modules: BTreeMap::from([(
                "test".into(),
                Module {
                    source: "test".into(),
                    routines: BTreeMap::from([(4096, routine)]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let n = &row["original"];
            let depth = c["depth"].as_u64().unwrap() as usize;
            let w = World {
                objects: BTreeMap::from([
                    (
                        1,
                        Object {
                            person: Some(BTreeMap::new()),
                            data: BTreeMap::from([(62, 1)]),
                            ..Default::default()
                        },
                    ),
                    (
                        2,
                        Object {
                            data: BTreeMap::from([(62, 1)]),
                            ..Default::default()
                        },
                    ),
                ]),
                actions: Some(WorldState {
                    actors: BTreeMap::from([(
                        1,
                        Actor {
                            active: Some(Action::default()),
                            ..Default::default()
                        },
                    )]),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let mut vm = Vm::new(w, 1);
            let id = vm
                .spawn(
                    &p,
                    Start {
                        owner: "test",
                        bhav: 4096,
                        caller: 1,
                        callee: 1,
                        stack_object: 1,
                        args: vec![],
                        check: false,
                    },
                )
                .unwrap();
            let mut t = vm.threads.remove(&id).unwrap();
            let f = t.stack[0].clone();
            t.stack.resize(depth, f);
            t.stack[0].action_target = Some(1);
            t.stack[0].action_icon = c["outer"] == 1;
            t.completed_actions.push((1, 2, true, c["inner"] == 1, 4));
            vm.finish_action_frames(&mut t).unwrap();
            assert_eq!(
                serde_json::json!(
                    vm.world.actions.as_ref().unwrap().actors[&1]
                        .active
                        .as_ref()
                        .unwrap()
                        .icon_owner
                ),
                n["icon"],
                "{c}"
            );
            assert_eq!(
                serde_json::json!(vm.world.objects[&2].data[&62]),
                n["use_count"]
            );
        }
    }
}
