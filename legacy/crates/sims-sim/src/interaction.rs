//! Native action dispatch with a bounded chair-room posture/presentation host.
//! All cursors and cancellation requests are ordinary saved simulation state.
use crate::{Event, Simulation};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Start, Status, Stop, ThreadId, program::MenuEntry};
use std::collections::{BTreeSet, VecDeque};

pub const MAX_QUEUE: usize = sims_vm::action::CAPACITY;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Interaction {
    pub id: u64,
    pub target: ObjectId,
    pub menu_index: usize,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    Checking,
    Acting,
    Seated,
    Exiting,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Active {
    pub item: Interaction,
    pub thread: ThreadId,
    pub phase: Phase,
    pub cancel_requested: bool,
    pub exit_blocked: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ActorQueue {
    pub main: Option<ThreadId>,
    /// Presentation/posture state. Pending actions live only in the native VM queue.
    pub active: Option<Active>,
    pub last_result: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuChoice {
    pub index: usize,
    pub label: String,
    pub available: bool,
}
impl Simulation {
    /// Loaded people already own native queues and persistent Main threads.
    /// They do not need the authored room's posture/presentation adapter.
    pub fn has_interaction_actor(&self, actor: ObjectId) -> bool {
        self.interactions.contains_key(&actor)
            || (self.native_lot.is_some()
                && self
                    .vm
                    .world
                    .objects
                    .get(&actor)
                    .is_some_and(|o| o.person.is_some())
                && self.vm.object_threads.contains_key(&actor)
                && self
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .is_some_and(|a| a.actors.contains_key(&actor)))
    }
    fn menu_entry<'a>(
        &self,
        program: &'a Program,
        target: ObjectId,
        index: usize,
    ) -> Result<(&'a str, &'a MenuEntry), String> {
        let def = self
            .vm
            .world
            .objects
            .get(&target)
            .and_then(|o| o.definition.as_ref())
            .ok_or("interaction target has no definition")?;
        let module = program
            .modules
            .get(&def.source)
            .ok_or("missing interaction module")?;
        let entry = module
            .object_interactions
            .get(&def.object)
            .and_then(|m| m.get(index))
            .ok_or("missing interaction table entry")?;
        program.resolve(&def.source, entry.action)?;
        if entry.check != 0 {
            program.resolve(&def.source, entry.check)?;
        }
        Ok((&module.source, entry))
    }
    /// Evaluate original eligibility in a disposable VM so opening a menu has no world/RNG effects.
    /// Eligibility is checked again when the queued action actually starts.
    pub fn interaction_menu(
        &self,
        program: &Program,
        actor: ObjectId,
        target: ObjectId,
    ) -> Result<Vec<MenuChoice>, String> {
        if !self.has_interaction_actor(actor) {
            return Err("actor has no interaction queue".into());
        }
        let def = self
            .vm
            .world
            .objects
            .get(&target)
            .and_then(|o| o.definition.as_ref())
            .ok_or("target has no definition")?;
        let entries = program
            .modules
            .get(&def.source)
            .and_then(|m| m.object_interactions.get(&def.object))
            .ok_or("target has no interaction menu")?;
        (0..entries.len())
            .map(|index| self.interaction_choice(program, actor, target, index))
            .collect()
    }
    /// Evaluate one original table entry without evaluating unrelated debug or expansion entries.
    /// This is eligibility only; the client's supported-action filter is not an engine menu policy.
    pub fn interaction_choice(
        &self,
        program: &Program,
        actor: ObjectId,
        target: ObjectId,
        index: usize,
    ) -> Result<MenuChoice, String> {
        if !self.has_interaction_actor(actor) {
            return Err("actor has no interaction queue".into());
        }
        let (source, entry) = self.menu_entry(program, target, index)?;
        let available = if entry.check == 0 {
            true
        } else {
            let mut vm = self.vm.clone();
            let id = vm.spawn(
                program,
                Start {
                    owner: source,
                    bhav: entry.check,
                    caller: actor,
                    callee: target,
                    stack_object: target,
                    args: vec![0],
                    check: true,
                },
            )?;
            match vm.run_slice(program, id, 1000)?.stop {
                Stop::Returned(v) => v,
                other => {
                    return Err(format!("interaction eligibility did not return: {other:?}"));
                }
            }
        };
        Ok(MenuChoice {
            index,
            label: entry.label.clone(),
            available,
        })
    }
    pub fn pending_interactions(
        &self,
        actor: ObjectId,
    ) -> Option<&VecDeque<sims_vm::action::Action>> {
        Some(
            &self
                .vm
                .world
                .actions
                .as_ref()?
                .actors
                .get(&actor)?
                .queue
                .pending,
        )
    }
    fn display_item(
        &self,
        program: &Program,
        action: &sims_vm::action::Action,
    ) -> Result<Interaction, String> {
        let menu_index = self
            .vm
            .action_menu(program, action.target)?
            .iter()
            .position(|e| e.id == action.menu_id)
            .ok_or("missing queued menu identifier")?;
        Ok(Interaction {
            id: u64::from(action.id),
            target: action.target,
            menu_index,
        })
    }
    pub fn enqueue(
        &mut self,
        program: &Program,
        actor: ObjectId,
        target: ObjectId,
        menu_index: usize,
    ) -> Result<u64, String> {
        if !self.has_interaction_actor(actor) {
            return Err("actor has no interaction host".into());
        }
        let (_, entry) = self.menu_entry(program, target, menu_index)?;
        let immediate = entry.flags & 4 != 0;
        let action = sims_vm::action::Action {
            target,
            icon_owner: target,
            menu_id: entry.id,
            routine: entry.action,
            priority: 100,
            ..Default::default()
        };
        if immediate {
            let before = self.clone();
            let mut step = crate::Step {
                stop: crate::StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            let result = (|| {
                Ok::<_, String>(
                    if let Some(v) = self.begin_immediate(program, actor, action, None)? {
                        Some(v)
                    } else {
                        self.drive_immediate(program, &mut 100000, &mut step)?
                    },
                )
            })();
            if !matches!(result, Ok(Some(true))) {
                *self = before;
                return Err(result.err().unwrap_or_else(|| {
                    "immediate interaction unavailable or exceeded command budget".into()
                }));
            }
            self.sync_spatial_world();
            return Ok(u64::from(self.vm.world.actions.as_ref().unwrap().serial));
        }
        let (id, accepted) = self.vm.enqueue_action(program, actor, action)?;
        self.flush_action_callbacks(program)?;
        if !accepted {
            return Err("interaction queue is full".into());
        }
        Ok(u64::from(id))
    }
    /// Native cancellation applies while a VM action runs. A completed seated
    /// interaction instead requests the original stand tree through this room host.
    pub fn cancel_interaction(
        &mut self,
        program: &Program,
        actor: ObjectId,
        id: u64,
    ) -> Result<(), String> {
        let native_id = u32::try_from(id).map_err(|_| "invalid native action ID")?;
        if self.native_lot.is_some() {
            if !self.has_interaction_actor(actor) {
                return Err("actor has no interaction host".into());
            }
            let cancelled = self.vm.cancel_action(program, actor, native_id)?;
            self.flush_action_callbacks(program)?;
            return if cancelled {
                Ok(())
            } else {
                Err("interaction is no longer queued".into())
            };
        }
        let q = self
            .interactions
            .get_mut(&actor)
            .ok_or("actor has no interaction host")?;
        let displayed = if let Some(a) = q.active.as_mut().filter(|a| a.item.id == id) {
            a.cancel_requested = true;
            a.exit_blocked = false;
            true
        } else {
            false
        };
        let cancelled = self.vm.cancel_action(program, actor, native_id)?;
        self.flush_action_callbacks(program)?;
        if !displayed && !cancelled {
            return Err("interaction is no longer queued".into());
        }
        if !displayed {
            self.interactions.get_mut(&actor).unwrap().last_result =
                Some("Queued interaction cancelled.".into());
        }
        Ok(())
    }
    pub(crate) fn interaction_cancelled(&self, thread: ThreadId) -> bool {
        self.interactions.values().any(|q| {
            q.active.as_ref().is_some_and(|a| {
                a.thread == thread && a.cancel_requested && a.phase == Phase::Acting
            })
        })
    }
    fn start_exit(
        &mut self,
        program: &Program,
        actor: ObjectId,
        target: ObjectId,
    ) -> Result<ThreadId, String> {
        let def = self
            .vm
            .world
            .objects
            .get(&target)
            .and_then(|o| o.definition.as_ref())
            .ok_or("seat has no definition")?;
        let entry = program
            .modules
            .get(&def.source)
            .and_then(|m| m.object_stand_functions.get(&def.object))
            .copied()
            .filter(|id| *id != 0)
            .ok_or("seat has no stand entry")?;
        let owner = def.source.clone();
        // This room's user-directed exit policy prefers the chair's local front.
        // The original stand tree reads RouteEntryFlags (person 37) as its first
        // exit choice, then checks the remaining sides itself. Leaving the old
        // approach flags here makes a clear chair exit sideways or backwards.
        self.person(actor, 37, 1)?;
        let thread = self.interactions[&actor]
            .main
            .ok_or("person Main missing")?;
        if let Some(call) = self.vm.threads.get_mut(&thread).unwrap().service.take() {
            if call.result.is_none() {
                return Err("stand service still running".into());
            }
            self.vm.threads.get_mut(&thread).unwrap().status = call.resume_status;
        }
        self.vm.push_service_call(
            program,
            thread,
            Start {
                owner: &owner,
                bhav: entry,
                caller: actor,
                callee: target,
                stack_object: target,
                args: vec![0],
                check: false,
            },
        )?;
        Ok(thread)
    }
    pub(crate) fn service_interactions(
        &mut self,
        program: &Program,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        let mut changed = false;
        for actor in self.interactions.keys().copied().collect::<Vec<_>>() {
            let mut q = self.interactions[&actor].clone();
            let thread = q.main.ok_or("person Main is not installed")?;
            if let Some(active) = &mut q.active
                && active.phase == Phase::Exiting
                && let Some(success) = self.vm.threads[&thread]
                    .service
                    .as_ref()
                    .and_then(|c| c.result)
            {
                changed = true;
                if success {
                    self.vm.threads.get_mut(&thread).unwrap().service.take();
                    self.release(thread);
                    if self.vm.world.objects[&actor].container.is_some()
                        || self.vm.world.objects[&actor].person.as_ref().unwrap()[&0] != 0
                    {
                        return Err("stand returned without clearing containment/posture".into());
                    }
                    self.animations.remove(&actor);
                    q.last_result = Some(
                        if active.cancel_requested {
                            "Interaction cancelled."
                        } else {
                            "Interaction completed."
                        }
                        .into(),
                    );
                    q.active = None;
                } else {
                    active.phase = Phase::Seated;
                    active.exit_blocked = true;
                    q.last_result = Some("No room to stand. Clear an exit, then retry.".into());
                    // Explicit laboratory retry policy; retain Main's exact continuation.
                    self.vm.threads.get_mut(&thread).unwrap().status = Status::Sleeping {
                        wake_tick: u64::MAX,
                    };
                }
            }
            let t = &self.vm.threads[&thread];
            let native = &self.vm.world.actions.as_ref().unwrap().actors[&actor];
            let selected = t
                .dispatch
                .as_ref()
                .map(|d| &d.action)
                .or(native.active.as_ref())
                .cloned();
            if q.active
                .as_ref()
                .is_none_or(|a| a.phase != Phase::Exiting && !a.exit_blocked)
            {
                if let Some(action) = selected {
                    let item = self.display_item(program, &action)?;
                    if q.active.as_ref().is_some_and(|a| a.item.id != item.id) {
                        // Main may finish one action and enter the next eligibility
                        // callback in the same tick. Its old action reservation ends
                        // here; physical seat containment remains authoritative.
                        self.routes.remove(&thread);
                        self.reservations.retain(|_, owner| *owner != thread);
                    }
                    let cancel_requested = q
                        .active
                        .as_ref()
                        .is_some_and(|a| a.item.id == item.id && a.cancel_requested);
                    let phase = if t.dispatch.is_some() {
                        Phase::Checking
                    } else {
                        Phase::Acting
                    };
                    changed |= q
                        .active
                        .as_ref()
                        .is_none_or(|a| a.item != item || a.phase != phase);
                    q.active = Some(Active {
                        item,
                        thread,
                        phase,
                        cancel_requested,
                        exit_blocked: false,
                    });
                    if cancel_requested
                        && native
                            .active
                            .as_ref()
                            .is_some_and(|a| a.flags & sims_vm::action::CANCELLED == 0)
                    {
                        self.vm.cancel_action(program, actor, action.id)?;
                    }
                } else if let Some(active) = &mut q.active {
                    if active.phase == Phase::Acting {
                        changed = true;
                        self.release(thread);
                        if self.vm.world.objects[&actor].container == Some((active.item.target, 0))
                        {
                            active.phase = Phase::Seated;
                        } else {
                            q.last_result = Some(
                                if active.cancel_requested {
                                    "Interaction cancelled."
                                } else {
                                    "Interaction completed."
                                }
                                .into(),
                            );
                            q.active = None;
                        }
                    } else if active.phase == Phase::Checking
                        && !self
                            .pending_interactions(actor)
                            .unwrap()
                            .iter()
                            .any(|a| u64::from(a.id) == active.item.id)
                    {
                        q.last_result = Some("Interaction ended without sitting.".into());
                        q.active = None;
                        changed = true;
                    }
                }
            }
            if let Some(active) = &mut q.active
                && active.phase == Phase::Seated
                && !active.exit_blocked
            {
                // Moving to a queued destination unwinds posture inside the route
                // primitive. Only the demo's explicit Stand command enters here.
                let exit = active.cancel_requested;
                let t = &self.vm.threads[&thread];
                if exit
                    && t.dispatch.is_none()
                    && self.vm.world.actions.as_ref().unwrap().actors[&actor]
                        .active
                        .is_none()
                    && matches!(t.status, Status::Ready | Status::Sleeping { .. })
                {
                    self.start_exit(program, actor, active.item.target)?;
                    active.phase = Phase::Exiting;
                    changed = true;
                    self.event(
                        events,
                        thread,
                        "original stand entry on the persistent person stack".into(),
                    );
                }
            }
            if q.active.is_none()
                && let Some(action) = self.pending_interactions(actor).and_then(|q| q.front())
            {
                q.active = Some(Active {
                    item: self.display_item(program, action)?,
                    thread,
                    phase: Phase::Checking,
                    cancel_requested: false,
                    exit_blocked: false,
                });
                q.last_result = None;
                changed = true;
            }
            self.interactions.insert(actor, q);
        }
        Ok(changed)
    }
    pub(crate) fn retire_interaction_threads(&mut self) {
        if self.interactions.is_empty() {
            return;
        }
        let keep: BTreeSet<_> = self
            .interactions
            .values()
            .filter_map(|q| q.active.as_ref().map(|a| a.thread))
            .chain(self.animations.values().map(|p| p.thread))
            .chain(self.vm.object_threads.values().copied())
            .chain(self.pending_actions.keys().copied())
            .collect();
        self.vm
            .threads
            .retain(|id, t| !matches!(t.status, Status::Returned(_)) || keep.contains(id));
    }
    pub(crate) fn validate_interactions(&self, program: &Program) -> Result<(), String> {
        let mut owners = BTreeSet::new();
        for (&actor, q) in &self.interactions {
            let thread = q.main.ok_or("saved person Main missing")?;
            let t = self
                .vm
                .threads
                .get(&thread)
                .ok_or("saved person thread missing")?;
            let object = self
                .vm
                .world
                .objects
                .get(&actor)
                .ok_or("saved actor missing")?;
            let lifecycle = self
                .vm
                .lifecycles
                .get(&actor)
                .ok_or("saved person lifecycle missing")?;
            if !owners.insert(thread)
                || lifecycle.thread != thread
                || object.person.is_none()
                || q.last_result.as_ref().is_some_and(|s| s.len() > 256)
            {
                return Err("invalid saved person Main ownership".into());
            }
            let native = self
                .vm
                .world
                .actions
                .as_ref()
                .and_then(|w| w.actors.get(&actor))
                .ok_or("saved actor action state missing")?;
            let mut ids = BTreeSet::new();
            for action in native.queue.pending.iter().chain(native.active.iter()) {
                if !ids.insert(action.id) {
                    return Err("duplicate saved room action ID".into());
                }
            }
            let selected = t
                .dispatch
                .as_ref()
                .map(|d| &d.action)
                .or(native.active.as_ref());
            if let Some(action) = selected {
                let expected = self.display_item(program, action)?;
                if q.active.as_ref().is_none_or(|a| {
                    a.item != expected || (a.phase != Phase::Checking && a.phase != Phase::Acting)
                }) {
                    return Err("display action differs from original dispatch".into());
                }
            }
            if let Some(a) = &q.active {
                if matches!(a.phase, Phase::Seated | Phase::Exiting)
                    && ids.contains(&(a.item.id as u32))
                {
                    return Err("completed seated action duplicates a pending action".into());
                }
                self.menu_entry(program, a.item.target, a.item.menu_index)?;
                if a.thread != thread
                    || a.item.id > u64::from(u32::MAX)
                    || (a.exit_blocked && a.phase != Phase::Seated)
                {
                    return Err("invalid saved interaction phase".into());
                }
                if a.phase == Phase::Seated && object.container != Some((a.item.target, 0)) {
                    return Err("seated display disagrees with containment".into());
                }
                if a.phase == Phase::Exiting || a.exit_blocked {
                    let call = t.service.as_ref().ok_or("stand service boundary missing")?;
                    let def = self.vm.world.objects[&a.item.target]
                        .definition
                        .as_ref()
                        .unwrap();
                    if call.owner != def.source
                        || call.caller != actor
                        || call.callee != a.item.target
                        || program.modules[&def.source]
                            .object_stand_functions
                            .get(&def.object)
                            != Some(&call.bhav)
                        || (a.exit_blocked && call.result != Some(false))
                    {
                        return Err("invalid original stand service binding".into());
                    }
                } else if t
                    .service
                    .as_ref()
                    .is_some_and(|s| !matches!(s.resume_status, Status::AwaitingRoute(_)))
                {
                    return Err("unexpected person service call".into());
                }
            } else if t.service.is_some() {
                return Err("orphaned person service call".into());
            }
        }
        Ok(())
    }
}
