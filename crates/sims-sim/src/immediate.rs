//! Synchronous action callbacks stay on the recipient's existing frame arena.
//! A diagnostic budget may interrupt the work; lot time may not advance.
use crate::{Simulation, Step};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Start, Status, ThreadId, action::Action};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    Check,
    Action,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    pub actor: ObjectId,
    pub thread: ThreadId,
    pub action: Action,
    pub owner: String,
    pub requester: Option<(ThreadId, u64)>,
    pub phase: Phase,
    pub saved_temps: Option<[i16; 8]>,
    pub priority: i16,
}
impl Simulation {
    pub(crate) fn validate_immediate(&self, p: &Program) -> Result<(), String> {
        let external: Vec<_> =
            self.vm
                .threads
                .iter()
                .flat_map(|(&id, t)| {
                    t.callback
                        .iter()
                        .flat_map(|b| b.chain())
                        .filter(move |b| {
                            b.external
                                && !self
                                    .vm
                                    .player_menu
                                    .as_ref()
                                    .is_some_and(|q| q.owns_callback(id, b.base_depth))
                                && !self
                                    .ui_wait_callback
                                    .as_ref()
                                    .is_some_and(|c| c.thread == id && c.depth == b.base_depth)
                                && !self
                                    .portrait_person
                                    .as_ref()
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .synchronous_removal
                                    .as_ref()
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .native_selection
                                    .as_ref()
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .ground_placement
                                    .as_ref()
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .controller_reconciliation
                                    .as_ref()
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .object_placement
                                    .as_ref()
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .object_pickup
                                    .as_ref()
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .cursor_factory
                                    .as_ref()
                                    .is_some_and(|f| f.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .cursor_rotation
                                    .as_ref()
                                    .is_some_and(|r| r.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .cursor_cleanup
                                    .as_ref()
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self.vm.owns_named_callback(id, b.base_depth)
                                && !self
                                    .cursor_release
                                    .as_ref()
                                    .is_some_and(|r| r.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self.characters.values().any(|r| {
                                    r.physical_movement.as_ref().is_some_and(|e| {
                                        e.owns_callback(&self.vm, p, id, b.base_depth)
                                    }) || r.native_walk.as_ref().is_some_and(|e| {
                                        e.owns_callback(&self.vm, p, id, b.base_depth)
                                    }) || r.native_turn.as_ref().is_some_and(|e| {
                                        e.owns_callback(&self.vm, p, id, b.base_depth)
                                    })
                                })
                                && !self
                                    .native_transfers
                                    .values()
                                    .any(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .native_reaches
                                    .values()
                                    .any(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .native_creations
                                    .values()
                                    .any(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self.deletion.iter().any(|d| {
                                    d.entry.as_ref().is_some_and(|e| {
                                        e.owns_callback(&self.vm, p, id, b.base_depth)
                                    }) || d.person_reset.as_ref().is_some_and(|e| {
                                        e.owns_callback(&self.vm, p, id, b.base_depth)
                                    }) || d.cursor.as_ref().is_some_and(|e| {
                                        e.owns_callback(&self.vm, p, id, b.base_depth)
                                    }) || d.base_reset.as_ref().is_some_and(|e| {
                                        e.owns_callback(&self.vm, p, id, b.base_depth)
                                    })
                                })
                                && !self
                                    .native_burn
                                    .as_ref()
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .native_lot
                                    .as_ref()
                                    .and_then(|n| n.removal.as_ref())
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .native_lot
                                    .as_ref()
                                    .and_then(|n| n.find_location.as_ref())
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .native_lot
                                    .as_ref()
                                    .and_then(|n| n.snap.as_ref())
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .native_lot
                                    .as_ref()
                                    .and_then(|n| n.completion.as_ref())
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self.person_factories.values().any(|s| {
                                    s.rejection_cleanup.as_ref().is_some_and(|c| {
                                        c.owns_callback(&self.vm, p, id, b.base_depth)
                                    })
                                })
                                && !self.person_factories.values().any(|s| {
                                    s.visitor.as_ref().is_some_and(|v| {
                                        v.owns_callback(&self.vm, id, b.base_depth)
                                    })
                                })
                                && !self.person_factories.values().any(|s| {
                                    s.acceptance.as_ref().is_some_and(|e| {
                                        e.owns_callback(&self.vm, p, id, b.base_depth)
                                    })
                                })
                                && !self
                                    .native_lot
                                    .as_ref()
                                    .and_then(|n| n.route.as_ref())
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .native_lot
                                    .as_ref()
                                    .and_then(|n| n.shoo.as_ref())
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .native_lot
                                    .as_ref()
                                    .and_then(|n| n.movement.as_ref())
                                    .is_some_and(|e| e.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self
                                    .person_factories
                                    .values()
                                    .any(|s| s.factory.owns_callback(&self.vm, p, id, b.base_depth))
                                && !self.person_factories.values().any(|s| {
                                    s.placement.as_ref().is_some_and(|e| {
                                        e.owns_callback(&self.vm, p, id, b.base_depth)
                                    })
                                })
                                && !self
                                    .cancellation
                                    .as_ref()
                                    .is_some_and(|e| e.thread == id && b.base_depth == e.depth)
                                && !self
                                    .reach_callback
                                    .as_ref()
                                    .is_some_and(|e| e.thread == id && b.base_depth == e.depth)
                        })
                        .map(move |b| (id, b))
                })
                .collect();
        let record = self
            .vm
            .world
            .actions
            .as_ref()
            .and_then(|a| a.immediate.as_ref());
        let Some(e) = &self.immediate else {
            if !external.is_empty() || record.is_some() {
                return Err("unbound immediate execution".into());
            }
            return Ok(());
        };
        let t = self
            .vm
            .threads
            .get(&e.thread)
            .ok_or("immediate thread missing")?;
        let o = self
            .vm
            .world
            .objects
            .get(&e.actor)
            .ok_or("immediate actor missing")?;
        let entry = self
            .vm
            .action_menu(p, e.action.target)?
            .iter()
            .find(|m| m.id == e.action.menu_id)
            .ok_or("immediate menu missing")?;
        let def = self.vm.world.objects[&e.action.target]
            .definition
            .as_ref()
            .ok_or("immediate target definition missing")?;
        if self.vm.object_threads.get(&e.actor) != Some(&e.thread)
            || o.person.is_none()
            || entry.flags & 4 == 0
            || entry.action != e.action.routine
            || e.owner != def.source
            || e.saved_temps.is_some() != (entry.check != 0)
        {
            return Err("immediate binding mismatch".into());
        }
        if e.phase == Phase::Check && (entry.check == 0 || record.is_some()) {
            return Err("invalid immediate eligibility phase".into());
        }
        if e.phase == Phase::Action
            && (record.is_none_or(|(actor, _)| *actor != e.actor)
                || o.person.as_ref().unwrap()[&33] != e.action.priority as i16)
        {
            return Err("invalid immediate action phase".into());
        }
        if let Some((actor, action)) = record {
            let m = self
                .vm
                .action_menu(p, action.target)?
                .iter()
                .find(|m| m.id == action.menu_id)
                .ok_or("immediate copied menu missing")?;
            if *actor != e.actor || m.action != action.routine {
                return Err("invalid immediate global record".into());
            }
        }
        if external.len() != 1 || external[0].0 != e.thread {
            return Err("immediate callback boundary missing".into());
        }
        let b = external[0].1;
        let f = t
            .stack
            .get(b.base_depth)
            .ok_or("immediate callback frame missing")?;
        if f.caller != e.actor
            || f.callee != e.action.target
            || f.code_owner != e.owner
            || f.location.bhav
                != if e.phase == Phase::Check {
                    entry.check
                } else {
                    e.action.routine
                }
        {
            return Err("invalid immediate callback frame".into());
        }
        if let Some((thread, request)) = e.requester {
            let requester = self
                .vm
                .threads
                .get(&thread)
                .ok_or("immediate requester missing")?;
            let status = if thread == e.thread {
                &b.resume_status
            } else {
                &requester.status
            };
            if !matches!(status,Status::AwaitingImmediate(r) if r.id==request && r.actor==e.actor && r.action==e.action)
            {
                return Err("immediate requester binding mismatch".into());
            }
        }
        Ok(())
    }
    pub(crate) fn begin_immediate(
        &mut self,
        p: &Program,
        actor: ObjectId,
        action: Action,
        requester: Option<(ThreadId, u64)>,
    ) -> Result<Option<bool>, String> {
        if self.immediate.is_some() {
            return Err("nested immediate eligibility callback is not yet supported".into());
        }
        let entry = self
            .vm
            .action_menu(p, action.target)?
            .iter()
            .find(|e| e.id == action.menu_id)
            .ok_or("immediate menu missing")?
            .clone();
        if entry.flags & 4 == 0 {
            return Err("non-immediate action at synchronous boundary".into());
        }
        if !self
            .vm
            .immediate_eligibility(actor, action.target, entry.flags)?
        {
            return Ok(Some(false));
        }
        let owner = self.vm.world.objects[&action.target]
            .definition
            .as_ref()
            .unwrap()
            .source
            .clone();
        let thread = *self
            .vm
            .object_threads
            .get(&actor)
            .ok_or("immediate actor has no Main")?;
        let saved_temps = (entry.check != 0).then_some(self.vm.world.objects[&actor].temps);
        let priority = self.vm.world.objects[&actor].person.as_ref().unwrap()[&33];
        if entry.check == 0 && self.vm.world.actions.as_ref().unwrap().immediate.is_some() {
            return Ok(Some(false));
        }
        let phase = if entry.check != 0 {
            Phase::Check
        } else {
            Phase::Action
        };
        if phase == Phase::Action {
            self.vm
                .world
                .objects
                .get_mut(&actor)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
                .insert(33, action.priority as i16);
            self.vm.world.actions.as_mut().unwrap().immediate = Some((actor, action.clone()));
        }
        self.vm.begin_immediate_callback(
            p,
            thread,
            Start {
                owner: &owner,
                bhav: if entry.check != 0 {
                    entry.check
                } else {
                    action.routine
                },
                caller: actor,
                callee: action.target,
                stack_object: action.target,
                args: if entry.check != 0 {
                    vec![0; 4]
                } else {
                    action.args.to_vec()
                },
                check: true,
            },
        )?;
        self.immediate = Some(Execution {
            actor,
            thread,
            action,
            owner,
            requester,
            phase,
            saved_temps,
            priority,
        });
        Ok(None)
    }
    pub(crate) fn drive_immediate(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Option<bool>, String> {
        loop {
            if !self.drive_cancellation(p, remaining, step)? {
                return Ok(None);
            }
            let e = self
                .immediate
                .clone()
                .ok_or("missing immediate execution")?;
            let id = e.thread;
            self.sync_spatial_world();
            if self.vm.threads[&id].callback.is_none() {
                let success = self.vm.threads[&id]
                    .callback_result
                    .ok_or("immediate callback has no result")?;
                if e.phase == Phase::Check
                    && success
                    && self.vm.world.actions.as_ref().unwrap().immediate.is_none()
                {
                    self.vm
                        .world
                        .objects
                        .get_mut(&e.actor)
                        .unwrap()
                        .person
                        .as_mut()
                        .unwrap()
                        .insert(33, e.action.priority as i16);
                    self.vm.world.actions.as_mut().unwrap().immediate =
                        Some((e.actor, e.action.clone()));
                    self.vm.begin_immediate_callback(
                        p,
                        id,
                        Start {
                            owner: &e.owner,
                            bhav: e.action.routine,
                            caller: e.actor,
                            callee: e.action.target,
                            stack_object: e.action.target,
                            args: e.action.args.to_vec(),
                            check: true,
                        },
                    )?;
                    self.immediate.as_mut().unwrap().phase = Phase::Action;
                    continue;
                }
                let accepted = e.phase == Phase::Action;
                if let Some(temps) = e.saved_temps {
                    self.vm.world.objects.get_mut(&e.actor).unwrap().temps = temps;
                }
                self.vm
                    .world
                    .objects
                    .get_mut(&e.actor)
                    .unwrap()
                    .person
                    .as_mut()
                    .unwrap()
                    .insert(33, e.priority);
                let a = self.vm.world.actions.as_mut().unwrap();
                if accepted {
                    let (_, action) = a
                        .immediate
                        .take()
                        .ok_or("missing native immediate record")?;
                    if action.id == 0 {
                        a.serial = a.serial.wrapping_add(1);
                    }
                }
                self.immediate = None;
                if let Some((requester, request)) = e.requester {
                    self.vm.finish_immediate(p, requester, request, accepted)?;
                }
                return Ok(Some(accepted));
            }
            if *remaining == 0 {
                return Ok(None);
            }
            match self.vm.threads[&id].status.clone() {
                Status::Ready => {
                    let run = self.vm.run_slice(p, id, *remaining)?;
                    *remaining -= run.trace.len();
                    step.trace.extend(run.trace);
                    self.publish_budget_statistics();
                }
                Status::AwaitingCreation(r) => {
                    if !self.service_creation(p, id, r, remaining, step)? {
                        return Ok(None);
                    }
                }
                Status::AwaitingPlacement(r) => {
                    self.service_find_location(p, id, &r, &mut step.events)?;
                }
                Status::AwaitingRemoval(r) => {
                    if !self.service_removal(p, id, &r, remaining, step)? {
                        return Ok(None);
                    }
                }
                Status::AwaitingTransfer(r) => {
                    self.service_transfer(p, id, &r, &mut step.events)?;
                    self.sync_carry(p)?;
                }
                Status::AwaitingImmediate(_) => {
                    return Err("nested immediate action requires native reentry semantics".into());
                }
                Status::Faulted(f) => {
                    return Err(format!("immediate callback failed: {}", f.message));
                }
                s => {
                    return Err(format!(
                        "immediate callback yielded (native error 0x44f): {s:?}"
                    ));
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::behavior::{Behavior, Instruction};
    use sims_vm::*;
    use std::collections::BTreeMap;
    fn expr(scope: u8, index: i16, value: i16, next: u8) -> Instruction {
        let mut b = [0; 8];
        b[..2].copy_from_slice(&index.to_le_bytes());
        b[2..4].copy_from_slice(&value.to_le_bytes());
        b[5..].copy_from_slice(&[5, scope, 7]);
        Instruction {
            opcode: 2,
            on_true: next,
            on_false: 253,
            operands: b,
        }
    }
    fn routine(ins: Vec<Instruction>) -> Routine {
        Routine {
            name: "fixture".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 4,
                locals: 0,
                instructions: ins,
            },
        }
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_immediate_actions_preserve_temps_priority_and_serials() {
        let report: serde_json::Value = serde_json::from_str(include_str!(
            "../../sims-vm/tests/fixtures/home-immediate.json"
        ))
        .unwrap();
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let e = &row["original"];
            for budget in [1, 1000] {
                let mut m = Module {
                    source: "test".into(),
                    ..Default::default()
                };
                m.routines = BTreeMap::from([
                    (4096, routine(vec![expr(0, 0, 0, 254)])),
                    (4097, routine(vec![expr(8, 0, 777, 255)])),
                    (
                        4098,
                        routine(vec![
                            expr(8, 0, 99, 1),
                            expr(0, 0, 321, c["check_return"].as_u64().unwrap_or(254) as u8),
                        ]),
                    ),
                ]);
                m.object_interactions.insert(
                    2,
                    vec![sims_vm::program::MenuEntry {
                        id: 0,
                        flags: 4,
                        action: 4097,
                        check: if c["check_return"].is_null() { 0 } else { 4098 },
                        ..Default::default()
                    }],
                );
                let p = Program {
                    modules: BTreeMap::from([("test".into(), m)]),
                    ..Default::default()
                };
                let mut world = World::default();
                for id in [1, 2] {
                    let mut o = Object {
                        definition: Some(Definition {
                            source: "test".into(),
                            guid: id as u32,
                            object: id as u16,
                            slots: 0,
                        }),
                        person: Some((0..256).map(|i| (i, 0)).collect()),
                        data: (0..72).map(|i| (i, 0)).collect(),
                        attributes: vec![100, 101, 102, 103],
                        temps: [1, 2, 3, 0, 7, -1, 8, 255],
                        ..Default::default()
                    };
                    o.person.as_mut().unwrap().insert(58, 30);
                    o.person.as_mut().unwrap().insert(33, 37);
                    world.objects.insert(id, o);
                }
                world.actions = Some(action::WorldState {
                    serial: 100,
                    actors: BTreeMap::from([(1, Default::default())]),
                    immediate: if c["busy"] == true {
                        Some((2, Action::default()))
                    } else {
                        None
                    },
                    ..Default::default()
                });
                let mut vm = Vm::new(world, 1);
                let thread = vm
                    .spawn(
                        &p,
                        Start {
                            owner: "test",
                            bhav: 4096,
                            caller: 1,
                            callee: 1,
                            stack_object: 1,
                            args: vec![0; 4],
                            check: false,
                        },
                    )
                    .unwrap();
                vm.object_threads.insert(1, thread);
                let mut s = Simulation::new(
                    vm,
                    crate::Room {
                        width: 64,
                        height: 64,
                        obstacles: vec![],
                    },
                    [1, 2]
                        .into_iter()
                        .map(|id| {
                            (
                                id,
                                crate::Body {
                                    level: 1,
                                    position: crate::Point::new(16 * id as i32, 16),
                                    facing: 1,
                                    radius: 0,
                                    elevation: 0,
                                },
                            )
                        })
                        .collect(),
                )
                .unwrap();
                let a = Action {
                    priority: c["priority"].as_i64().unwrap() as i32,
                    id: c["serial"].as_u64().unwrap() as u32,
                    target: 2,
                    routine: 4097,
                    ..Default::default()
                };
                let mut result = s.begin_immediate(&p, 1, a, None).unwrap();
                while result.is_none() {
                    let mut step = Step {
                        stop: crate::StepStop::Waiting,
                        trace: vec![],
                        events: vec![],
                    };
                    result = s
                        .drive_immediate(&p, &mut budget.clone(), &mut step)
                        .unwrap();
                }
                assert_eq!(result, Some(e["accepted"] == true), "{c}");
                assert_eq!(
                    serde_json::json!(s.vm.world.objects[&1].temps),
                    e["temps"],
                    "{c}"
                );
                assert_eq!(
                    serde_json::json!(s.vm.world.objects[&1].person.as_ref().unwrap()[&33]),
                    e["priority"],
                    "{c}"
                );
                assert_eq!(
                    serde_json::json!(s.vm.world.actions.as_ref().unwrap().serial),
                    e["next_serial"],
                    "{c}"
                );
                assert!(
                    s.vm.world.actions.as_ref().unwrap().actors[&1]
                        .queue
                        .pending
                        .is_empty()
                );
            }
        }
    }
}
