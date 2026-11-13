//! Standalone placement shoo, 45bd5e. Unlike corridor shoo, this has no
//! potential-path query or route-control link. All physical work uses the
//! imported lot's existing factory, placement, cancellation and destructor.
use crate::{Simulation, Step, lot_links, lot_placement, removal};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, ThreadId, action::Action};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Factory,
    Place,
    Enqueue,
    Publish,
    Detach,
    Destroy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct Execution {
    blocker: ObjectId,
    priority: i32,
    object: ObjectId,
    phase: Phase,
    placement: Option<lot_placement::live::Execution>,
    queued: Option<(u32, bool)>,
    after_delete_room: Option<i16>,
}

/// 45bec1/45bf03: active action first, then the pending queue backwards.
/// The route caller's separate "already asking" link is not consulted here.
pub(super) fn has_move(s: &Simulation, blocker: ObjectId) -> Result<bool, String> {
    let actor =
        s.vm.world
            .actions
            .as_ref()
            .and_then(|a| a.actors.get(&blocker))
            .ok_or("placement shoo action owner absent")?;
    Ok(actor
        .active
        .iter()
        .chain(actor.queue.pending.iter().rev())
        .any(|a| {
            a.menu_id == 3
                && s.vm
                    .world
                    .objects
                    .get(&a.target)
                    .and_then(|o| o.definition.as_ref())
                    .is_some_and(|d| d.guid == 0x7c4)
        }))
}

impl Execution {
    pub(super) fn begin(
        s: &mut Simulation,
        p: &Program,
        blocker: ObjectId,
        priority: i32,
    ) -> Result<Option<Self>, String> {
        if has_move(s, blocker)? {
            return Ok(None);
        }
        let (module, number) = p
            .modules
            .values()
            .find_map(|m| {
                m.object_definition_words
                    .iter()
                    .find(|(_, w)| w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16) == 0x7c4)
                    .map(|(&n, _)| (m, n))
            })
            .ok_or("native placement shoo GoHere GUID 000007C4 is not loaded")?;
        let Some(object) = (1..=i16::MAX).find(|id| !s.vm.world.objects.contains_key(id)) else {
            return Ok(None);
        };
        s.initialize_object_factory(p, object, &module.source, number, false)?;
        Ok(Some(Self {
            blocker,
            priority,
            object,
            phase: Phase::Factory,
            placement: None,
            queued: None,
            after_delete_room: None,
        }))
    }
    pub(super) fn owns_callback(
        &self,
        vm: &sims_vm::Vm,
        p: &Program,
        thread: ThreadId,
        depth: usize,
    ) -> bool {
        self.placement
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub(super) fn owns_deletion(&self, id: ObjectId) -> bool {
        self.phase == Phase::Destroy && self.object == id
    }
    pub(super) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if s.vm
            .world
            .objects
            .get(&self.blocker)
            .is_none_or(|o| o.person.is_none())
            || s.vm
                .world
                .objects
                .get(&self.object)
                .and_then(|o| o.definition.as_ref())
                .is_none_or(|d| d.guid != 0x7c4)
            || self.placement.is_some()
                != matches!(self.phase, Phase::Place | Phase::Detach | Phase::Destroy)
            || self.queued.is_some()
                != matches!(self.phase, Phase::Publish | Phase::Detach | Phase::Destroy)
            || matches!(self.phase, Phase::Detach | Phase::Destroy)
                && self.queued.is_none_or(|(_, accepted)| accepted)
        {
            return Err("invalid standalone shoo continuation".into());
        }
        if let Some(e) = &self.placement {
            e.validate(&s.vm, p, self.object)?;
            match self.phase {
                Phase::Place => {
                    let (object, destination) = e.operation.binding();
                    if object != self.object
                        || destination
                            .is_none_or(|d| !d.on_lot || d.container.is_some() || d.slot != 0)
                    {
                        return Err("standalone shoo placement target changed".into());
                    }
                }
                Phase::Detach | Phase::Destroy => {
                    if e.operation.binding() != (self.object, None) {
                        return Err("standalone shoo removal target changed".into());
                    }
                }
                _ => unreachable!(),
            }
        }
        if self.phase == Phase::Destroy
            && s.deletion.first().is_none_or(|d| d.target != self.object)
        {
            return Err("standalone shoo rejection lost destructor".into());
        }
        Ok(())
    }
}

impl Simulation {
    pub(super) fn drive_snap_shoo(
        &mut self,
        p: &Program,
        e: &mut Execution,
        thread: ThreadId,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Option<bool>, String> {
        loop {
            match e.phase {
                Phase::Factory => {
                    if !self.drive_object_factory(p, e.object, remaining, step)? {
                        return Ok(None);
                    }
                    self.native_lot.as_mut().unwrap().observers.push(
                        lot_placement::Request::Signal {
                            event: 0xe2,
                            value: e.object,
                        },
                    );
                    let at = self.vm.world.positions[&e.blocker];
                    e.placement = Some(lot_placement::live::Execution::new(
                        lot_placement::Operation::set(
                            e.object,
                            lot_links::Descriptor {
                                on_lot: true,
                                position: [at.x, at.y],
                                level: at.level,
                                container: None,
                                slot: 0,
                            },
                            true,
                        ),
                    ));
                    e.phase = Phase::Place;
                }
                Phase::Place => {
                    if !self.drive_live_placement(
                        p,
                        e.placement.as_mut().unwrap(),
                        remaining,
                        step,
                    )? {
                        return Ok(None);
                    }
                    e.placement = None;
                    e.phase = Phase::Enqueue;
                }
                Phase::Enqueue => {
                    self.vm
                        .world
                        .objects
                        .get_mut(&e.object)
                        .unwrap()
                        .routing_slots = Some(vec![crate::contention::placement_shoo_descriptor()]);
                    let menu = self
                        .vm
                        .action_menu(p, e.object)?
                        .iter()
                        .find(|m| m.id == 3)
                        .ok_or("GoHere Move menu missing")?;
                    e.queued = Some(self.vm.enqueue_action(
                        p,
                        e.blocker,
                        Action {
                            target: e.object,
                            icon_owner: e.object,
                            menu_id: 3,
                            priority: e.priority,
                            routine: menu.action,
                            flags: u32::from(menu.flags & 0x100 != 0),
                            ..Default::default()
                        },
                    )?);
                    e.phase = Phase::Publish;
                }
                Phase::Publish => {
                    if !self.drive_cancellation(p, remaining, step)? {
                        return Ok(None);
                    }
                    let (serial, accepted) = e.queued.unwrap();
                    self.event(&mut step.events, thread, format!("original placement shoo Move #{serial} for #{} object #{} priority={} accepted={accepted}", e.blocker, e.object, e.priority));
                    if accepted {
                        return Ok(Some(true));
                    }
                    self.native_removal_tree(p, e.object)?;
                    e.after_delete_room =
                        crate::lot_removal::immediate_room(&self.vm.world, e.object)?;
                    e.placement = Some(lot_placement::live::Execution::new(
                        lot_placement::Operation::remove(e.object, true),
                    ));
                    e.phase = Phase::Detach;
                }
                Phase::Detach => {
                    if !self.drive_live_placement(
                        p,
                        e.placement.as_mut().unwrap(),
                        remaining,
                        step,
                    )? {
                        return Ok(None);
                    }
                    if !self.deletion.is_empty() {
                        return Err("nested standalone shoo destruction requires a destructor stack boundary".into());
                    }
                    let mut deletion = removal::Deletion::new(e.object);
                    deletion.after_delete_room = e.after_delete_room;
                    self.deletion.push(deletion);
                    e.phase = Phase::Destroy;
                }
                Phase::Destroy => {
                    if !self.drive_deletion_stack(p, remaining, step, false)? {
                        return Ok(None);
                    }
                    return Ok(Some(false));
                }
            }
        }
    }
}
