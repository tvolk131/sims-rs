//! Owned pre-Reset services inside 440eeb. Time cannot pass while one of these
//! synchronous calls is suspended. UI selection uses the same real controller
//! as player selection, with a distinct saved owner inside the deletion stack.
use crate::{Simulation, Step, lot_selection, person_reset};
use serde::{Deserialize, Serialize};
use sims_vm::{
    ObjectId, Program, ThreadId, Vm,
    deletion_entry::{Entry, Request, View},
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    controller: Entry,
    pub(crate) selection: Option<lot_selection::Execution>,
    pub(crate) references: Option<person_reset::Execution>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deletion_notifies_live_person_order_and_saves_inside_cancellation() {
        let (mut p, mut initial) = crate::lot_runtime::tests::fixture();
        let m = p.modules.get_mut("objects").unwrap();
        let mut callback = m.routines[&4096].clone();
        callback.body.instructions = vec![callback.body.instructions[0].clone(); 3];
        callback.body.instructions[0].on_true = 1;
        callback.body.instructions[1].on_true = 2;
        m.routines.insert(4100, callback);
        m.object_functions.get_mut(&1).unwrap()[4].action = 4100;
        let mut words = m.object_definition_words[&1].clone();
        words[9] = 2;
        words[14] = 2;
        m.object_definition_words.insert(2, words);
        m.object_functions.insert(2, m.object_functions[&1].clone());
        for object in initial.vm.world.objects.values_mut() {
            object.native_flags = Some(0);
        }
        initial.vm.world.globals.insert(20, 0);
        initial.vm.world.refresh = Some(sims_vm::refresh::RefreshState {
            draw: (1..=4).map(|id| (id, Default::default())).collect(),
            ..Default::default()
        });
        initial.vm.world.person.as_mut().unwrap().person_order = vec![3, 1];
        for id in [3, 1] {
            let o = initial.vm.world.objects.get_mut(&id).unwrap();
            o.person = Some((0..256).map(|i| (i, 0)).collect());
            let d = o.definition.as_mut().unwrap();
            d.object = 2;
            d.guid = 2;
            initial.vm.start_object_lifecycle(&p, id).unwrap();
        }
        initial.vm.world.actions = Some(sims_vm::action::WorldState {
            serial: 3,
            actors: [1, 3]
                .map(|id| {
                    (
                        id,
                        sims_vm::action::Actor {
                            queue: sims_vm::action::Queue {
                                pending: [sims_vm::action::Action {
                                    id: id as u32,
                                    target: 4,
                                    icon_owner: 4,
                                    ..Default::default()
                                }]
                                .into(),
                            },
                            ..Default::default()
                        },
                    )
                })
                .into(),
            ..Default::default()
        });
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut s = initial.clone();
            s.deletion.push(crate::removal::Deletion::new(4));
            let mut step = Step {
                stop: crate::StepStop::Budget,
                trace: vec![],
                events: vec![],
            };
            let mut observed = vec![];
            let mut done = false;
            for _ in 0..100 {
                done = s
                    .drive_deletion_entry(&p, &mut budget.clone(), &mut step)
                    .unwrap();
                if let Some(c) = &s.cancellation {
                    if observed.last() != Some(&c.callback.stack_object) {
                        observed.push(c.callback.stack_object);
                    }
                }
                s.deletion
                    .last()
                    .unwrap()
                    .entry
                    .as_ref()
                    .unwrap()
                    .validate(&s, &p, 4)
                    .unwrap();
                let restored: Simulation =
                    serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
                assert_eq!(restored, s);
                s = restored;
                if done {
                    break;
                }
            }
            assert!(done);
            if budget == 1 {
                assert_eq!(observed, [3, 1]);
            }
            assert!(
                s.vm.world
                    .actions
                    .as_ref()
                    .unwrap()
                    .actors
                    .values()
                    .all(|a| a.queue.pending.is_empty())
            );
            assert!(s.vm.world.objects.contains_key(&4));
            assert_eq!(s.vm.world.objects[&4].native_flags, Some(0x40));
            assert_eq!(s.vm.tick, initial.vm.tick);
            outcomes.push((s, step));
        }
        assert_eq!(outcomes[0].0, outcomes[1].0);
        assert_eq!(outcomes[0].1.trace, outcomes[1].1.trace);
    }
}
impl Execution {
    fn new(id: ObjectId) -> Self {
        Self {
            controller: Entry::new(id),
            selection: None,
            references: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.controller.complete() && self.selection.is_none() && self.references.is_none()
    }
    pub(crate) fn retired(&self, id: ObjectId) -> bool {
        self.complete() && self.controller.object() == id && self.controller.validate().is_ok()
    }
    pub(crate) fn owns_callback(
        &self,
        vm: &Vm,
        p: &Program,
        thread: ThreadId,
        depth: usize,
    ) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .references
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program, id: ObjectId) -> Result<(), String> {
        self.controller.validate()?;
        if self.controller.object() != id || self.selection.is_some() && self.references.is_some() {
            return Err("deletion entry lost its object or synchronous caller".into());
        }
        if let Some(e) = &self.selection {
            e.validate(&s.vm, p)?;
            if !matches!(self.controller.pending(), Some(Request::ClearSelection) if e.is_clear())
                && !matches!(self.controller.pending(), Some(Request::ReleaseIndicator) if e.is_release(id))
            {
                return Err("deletion selection has no waiting caller".into());
            }
        }
        if let Some(e) = &self.references {
            if self.controller.pending() != Some(Request::References) || !e.broadcasts(id) {
                return Err("deletion reference broadcast has no waiting caller".into());
            }
            e.validate(s, p)?;
        }
        Ok(())
    }
}
impl View for Simulation {
    fn flags(&self, object: ObjectId) -> Result<u32, String> {
        self.vm
            .world
            .objects
            .get(&object)
            .and_then(|o| o.native_flags)
            .ok_or_else(|| "deletion native flags absent".into())
    }
    fn write_flags(&mut self, object: ObjectId, flags: u32) -> Result<(), String> {
        *self
            .vm
            .world
            .objects
            .get_mut(&object)
            .and_then(|o| o.native_flags.as_mut())
            .ok_or("deletion native flags absent")? = flags;
        Ok(())
    }
    fn ui_owner(&self) -> Option<ObjectId> {
        self.vm.world.ui_owner.owner
    }
    fn selected(&self) -> Result<Option<ObjectId>, String> {
        self.native_selected_person()
    }
}
impl Simulation {
    pub fn deletion_entry_pending(&self) -> bool {
        self.deletion
            .last()
            .and_then(|d| d.entry.as_ref())
            .is_some_and(|e| !e.complete())
    }
    pub(crate) fn drive_deletion_entry(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let d = self
            .deletion
            .last_mut()
            .ok_or("deletion entry owner absent")?;
        if self.vm.world.ui_owner.dialog_owners.contains(&d.target) {
            return Err("deletion with live UI dialogs requires dialog disposal".into());
        }
        let mut e = d.entry.take().unwrap_or_else(|| Execution::new(d.target));
        let result = self.advance_deletion_entry(p, &mut e, remaining, step);
        self.deletion
            .last_mut()
            .ok_or("deletion entry owner changed")?
            .entry = Some(e);
        result
    }
    fn advance_deletion_entry(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let id = e.controller.object();
        while *remaining > 0 {
            let Some(request) = e.controller.advance(self)? else {
                return Ok(true);
            };
            match request {
                Request::Signal => self
                    .native_lot
                    .as_mut()
                    .ok_or("deletion lot absent")?
                    .observers
                    .push(crate::lot_placement::Request::Signal {
                        event: 0x87,
                        value: id,
                    }),
                Request::ReleaseUi => {
                    if self.vm.world.ui_owner.dialog_owners.contains(&id) {
                        return Err("deletion with live UI dialogs requires dialog disposal".into());
                    }
                    self.vm.world.ui_owner.release_if_owner(id)?;
                }
                Request::ClearSelection | Request::ReleaseIndicator => {
                    let selection = e.selection.get_or_insert_with(|| {
                        if request == Request::ClearSelection {
                            lot_selection::Execution::clear()
                        } else {
                            lot_selection::Execution::release(id)
                        }
                    });
                    // The OBJD's original Animator flag requires a retained
                    // ordinary graphic. The host errors if that owner is absent;
                    // it must not silently skip an owed refresh.
                    let mut ordinary = BTreeSet::new();
                    for (&object, o) in &self.vm.world.objects {
                        if o.person.is_none() {
                            if let Some(d) = &o.definition {
                                if sims_vm::construction::definition_words(p, d)?
                                    .get(102)
                                    //431759 zero-extends legacy OBJD tails.
                                    //Cursors.iff is version136/160 bytes and
                                    //therefore has no Animator flag at+cc.
                                    .is_some_and(|word| word & 1 != 0)
                                {
                                    ordinary.insert(object);
                                }
                            }
                        }
                    }
                    let graphics = ordinary
                        .iter()
                        .chain(self.characters.keys())
                        .copied()
                        .collect();
                    if !selection.advance(self, p, &graphics, &ordinary, remaining, step)? {
                        return Ok(false);
                    }
                    e.selection = None;
                }
                Request::Invalidate => self.vm.refresh_object(id, id, 0, 0)?,
                Request::References => {
                    let broadcast = e
                        .references
                        .get_or_insert_with(|| person_reset::Execution::broadcast(id));
                    if !self.drive_person_reset(p, broadcast, remaining, step)? {
                        return Ok(false);
                    }
                    e.references = None;
                }
            }
            e.controller.acknowledge(request)?;
            *remaining = remaining.saturating_sub(1);
        }
        Ok(e.complete())
    }
}
