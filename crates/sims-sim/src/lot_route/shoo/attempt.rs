//! 0x45bf2e after segment sampling: factory, physical placement, lateral
//! feasibility, Move insertion and (on rejection) synchronous destruction.
use crate::{Simulation, Step, lot_links, lot_placement, removal};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, RouteRequest, ThreadId, action::Action};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Factory,
    Place,
    Query,
    Enqueue,
    Publish,
    Detach,
    Destroy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    pub blocker: ObjectId,
    object: ObjectId,
    direction: Option<u8>,
    phase: Phase,
    placement: Option<lot_placement::live::Execution>,
    route: Option<RouteRequest>,
    query: Option<super::super::live::Execution>,
    side_path: Option<bool>,
    queued: Option<(u32, bool)>,
    after_delete_room: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ignored_word: Option<i16>,
}
impl Execution {
    pub(super) fn begin(
        s: &mut Simulation,
        p: &Program,
        blocker: ObjectId,
        direction: Option<u8>,
    ) -> Result<Option<Self>, String> {
        let Some((module, number)) = p.modules.values().find_map(|m| {
            m.object_definition_words
                .iter()
                .find(|(_, w)| w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16) == 0x7c4)
                .map(|(&n, _)| (m, n))
        }) else {
            return Err("native shoo GoHere GUID 000007C4 is not loaded".into());
        };
        let Some(object) = (1..=i16::MAX).find(|id| !s.vm.world.objects.contains_key(id)) else {
            return Ok(None);
        };
        s.initialize_object_factory(p, object, &module.source, number, false)?;
        Ok(Some(Self {
            blocker,
            object,
            direction,
            phase: Phase::Factory,
            placement: None,
            route: None,
            query: None,
            side_path: None,
            queued: None,
            after_delete_room: None,
            ignored_word: None,
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
            || self
                .query
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub(super) fn owns_deletion(&self, id: ObjectId) -> bool {
        self.phase == Phase::Destroy && self.object == id
    }
    pub(super) fn validate(
        &self,
        s: &Simulation,
        p: &Program,
        parent: &RouteRequest,
        thread: ThreadId,
    ) -> Result<(), String> {
        if self.direction.is_some_and(|d| d > 7)
            || self.direction.is_none() && !matches!(self.phase, Phase::Factory | Phase::Place)
            || s.vm
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
            || self.query.is_some() != (self.phase == Phase::Query)
            || self.side_path.is_some()
                != matches!(
                    self.phase,
                    Phase::Enqueue | Phase::Publish | Phase::Detach | Phase::Destroy
                )
            || self.queued.is_some()
                != matches!(self.phase, Phase::Publish | Phase::Detach | Phase::Destroy)
            || self.route.is_some() != !matches!(self.phase, Phase::Factory | Phase::Place)
            || self.ignored_word.is_some() != self.route.is_some()
        {
            return Err("invalid native shoo factory phase".into());
        }
        if let Some(e) = &self.placement {
            e.validate(&s.vm, p, self.object)?;
        }
        if let Some(r) = &self.route {
            let def = s.vm.world.objects[&self.object]
                .definition
                .as_ref()
                .unwrap();
            if (r.id, r.caller, r.target) != (parent.id, self.blocker, self.object)
                || r.origin != parent.origin
                || !r.no_failure_trees
                || r.destination
                    != (sims_vm::routing::Destination::Slot {
                        source: def.source.clone(),
                        chunk: def.slots,
                        index: 0,
                        version: 10,
                        slot: crate::contention::shoo_descriptor(self.direction.unwrap(), true)
                            .slot(),
                    })
            {
                return Err("invalid native shoo lateral request".into());
            }
            if let Some(q) = &self.query {
                if q.thread != thread
                    || q.potential.as_ref().is_none_or(|q| {
                        q.ignored_person.is_some()
                            || q.ignore_people
                            || q.blocked_object != 0
                            || q.retained_result != 0
                            || Some(q.ignored_word) != self.ignored_word
                    })
                {
                    return Err("invalid native shoo lateral context".into());
                }
                q.validate_request(s, p, r)?;
            }
        }
        if self.phase == Phase::Destroy
            && s.deletion.first().is_none_or(|d| d.target != self.object)
        {
            return Err("native shoo rejection lost destructor".into());
        }
        Ok(())
    }
}
impl Simulation {
    pub(super) fn drive_native_shoo_attempt(
        &mut self,
        p: &Program,
        e: &mut Execution,
        thread: ThreadId,
        parent: &RouteRequest,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Option<bool>, String> {
        e.validate(self, p, parent, thread)?;
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
                    // Original allocates and places before rejecting a zero
                    // direction. This branch leaves the GoHere object alive.
                    let Some(direction) = e.direction else {
                        return Ok(Some(false));
                    };
                    let def = self.vm.world.objects[&e.object]
                        .definition
                        .as_ref()
                        .unwrap();
                    let r = RouteRequest {
                        id: parent.id,
                        origin: parent.origin.clone(),
                        caller: e.blocker,
                        target: e.object,
                        destination: sims_vm::routing::Destination::Slot {
                            source: def.source.clone(),
                            chunk: def.slots,
                            index: 0,
                            version: 10,
                            slot: crate::contention::shoo_descriptor(direction, true).slot(),
                        },
                        no_failure_trees: true,
                    };
                    let ignored_word = self.vm.world.objects[&e.blocker]
                        .person
                        .as_ref()
                        .and_then(|person| person.get(&72))
                        .copied()
                        .ok_or("native shoo ignored word absent")?;
                    let q = super::super::live::Execution::begin_potential(
                        self,
                        p,
                        thread,
                        &r,
                        super::super::live::Potential {
                            ignored_person: None,
                            ignore_people: false,
                            ignored_word,
                            blocked_object: 0,
                            retained_result: 0,
                        },
                    )?;
                    e.ignored_word = q.potential.as_ref().map(|settings| settings.ignored_word);
                    e.route = Some(r);
                    e.query = Some(q);
                    e.placement = None;
                    e.phase = Phase::Query;
                }
                Phase::Query => {
                    let q = e.query.as_mut().unwrap();
                    if !self.drive_potential_preparation(
                        p,
                        q,
                        e.route.as_ref().unwrap(),
                        remaining,
                        step,
                    )? {
                        return Ok(None);
                    }
                    let actor = &self.vm.world.objects[&q.caller];
                    let person = actor.person.as_ref().unwrap();
                    let same_room = actor.data[&29] == self.vm.world.objects[&q.target].data[&29]
                        || [68, 84].into_iter().any(|i| person[&i] & 1 != 0);
                    if !q.candidates.as_ref().unwrap().candidates.is_empty() && same_room {
                        if !self.drive_native_path_prefix(p, q, remaining, step)? {
                            return Ok(None);
                        }
                        if q.policy == super::super::path::Policy::Strict {
                            return Err("original shoo lateral path crosses UNVERIFIED full routing wrapper boundary".into());
                        }
                    }
                    e.side_path = Some(q.path.as_ref().is_some_and(|i| i.points.is_some()));
                    e.query = None;
                    e.phase = Phase::Enqueue;
                }
                Phase::Enqueue => {
                    self.vm
                        .world
                        .objects
                        .get_mut(&e.object)
                        .unwrap()
                        .routing_slots = Some(vec![crate::contention::shoo_descriptor(
                        e.direction.unwrap(),
                        e.side_path.unwrap(),
                    )]);
                    let menu = self
                        .vm
                        .action_menu(p, e.object)?
                        .iter()
                        .find(|m| m.id == 3)
                        .ok_or("GoHere Move menu missing")?;
                    let priority = i32::from(
                        self.vm.world.objects[&parent.caller]
                            .person
                            .as_ref()
                            .unwrap()[&33],
                    );
                    let (serial, accepted) = self.vm.enqueue_action(
                        p,
                        e.blocker,
                        Action {
                            target: e.object,
                            icon_owner: e.object,
                            menu_id: 3,
                            priority,
                            routine: menu.action,
                            flags: u32::from(menu.flags & 0x100 != 0),
                            ..Default::default()
                        },
                    )?;
                    e.queued = Some((serial, accepted));
                    e.phase = Phase::Publish;
                }
                Phase::Publish => {
                    // Queue replacement invokes cancelled targets' function 4
                    // before 0x45bf2e publishes the link or destroys GoHere.
                    if !self.drive_cancellation(p, remaining, step)? {
                        return Ok(None);
                    }
                    let (serial, accepted) = e.queued.unwrap();
                    self.event(&mut step.events, thread, format!("original shoo Move #{serial} for #{} object #{} lateral={} accepted={accepted}", e.blocker, e.object, e.side_path.unwrap()));
                    if accepted {
                        let c = self.route_control.get_mut(&parent.id).unwrap();
                        c.blocker_success = false;
                        c.link = Some(crate::contention::Link {
                            blocker: e.blocker,
                            object: e.object,
                            action: serial,
                        });
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
                        return Err("nested synchronous shoo destruction requires a destructor stack boundary".into());
                    }
                    let mut d = removal::Deletion::new(e.object);
                    d.after_delete_room = e.after_delete_room;
                    self.deletion.push(d);
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
