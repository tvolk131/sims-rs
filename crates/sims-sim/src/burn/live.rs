//! Synchronous Burn services in the imported lot. Ordinary scheduling is held
//! until the controller returns; factory/placement callbacks retain their own
//! continuations and instruction budgets.
use super::{
    Execution as Controller, Input, Position, Request, Response,
    world::{Live, Resource},
};
use crate::{Simulation, Step, lot_acceptance, lot_links::Descriptor, lot_placement, removal};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, Vm};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Cleanup {
    object: ObjectId,
    placement: lot_placement::live::Execution,
    after_room: Option<i16>,
    destroying: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::behavior::Instruction;

    #[test]
    fn live_fire_factory_placement_and_failure_cleanup_resume_without_repeating_callbacks() {
        for reject in [false, true] {
            let (mut p, mut initial) = crate::lot_runtime::tests::fixture();
            let m = p.modules.get_mut("objects").unwrap();
            let template = m.routines[&4096].clone();
            let expr = |dest, op, scope, value, next| Instruction {
                opcode: 2,
                on_true: next,
                on_false: 255,
                operands: [dest, 0, value, 0, 0, op, scope, 7],
            };
            for (id, instructions) in [
                (
                    4096,
                    vec![Instruction {
                        opcode: 9,
                        on_true: 254,
                        on_false: 255,
                        operands: [1, 1, 0, 0, 0, 0, 0, 0],
                    }],
                ),
                (4097, vec![expr(0, 5, 8, 0, 254)]),
                (
                    4098,
                    vec![
                        expr(4, 5, 3, 1, 1),
                        expr(42, 5, 3, if reject { 0 } else { 3 }, 2),
                        expr(34, 3, 6, 1, 3),
                        Instruction {
                            opcode: 1,
                            on_true: 4,
                            on_false: 255,
                            operands: [10, 0, 0, 0, 0, 0, 0, 0],
                        },
                        Instruction {
                            opcode: 1,
                            on_true: 254,
                            on_false: 255,
                            operands: [10, 0, 0, 0, 0, 0, 0, 0],
                        },
                    ],
                ),
                (4099, vec![expr(34, 3, 6, 10, 254)]),
                (4100, vec![expr(34, 3, 6, 100, 1), expr(0, 5, 8, 0, 254)]),
                (4101, vec![expr(36, 3, 6, 1, 1), expr(0, 5, 8, 0, 254)]),
            ] {
                let mut routine = template.clone();
                routine.body.instructions = instructions;
                m.routines.insert(id, routine);
            }
            let mut words = m.object_definition_words[&1].clone();
            words[14] = super::super::FIRE_GUID as i16;
            words[15] = (super::super::FIRE_GUID >> 16) as i16;
            m.object_definition_words.insert(2, words.clone());
            words[14] = 3;
            words[15] = 0;
            m.object_definition_words.insert(3, words);
            let mut functions = m.object_functions[&1].clone();
            functions[0].action = 4098;
            functions[1].action = 4099;
            functions[9].action = 4100;
            functions[10].action = 4101;
            m.object_functions.insert(2, functions);
            let mut functions = m.object_functions[&1].clone();
            functions[1].action = 4097;
            m.object_functions.insert(3, functions);
            for id in [1, 5, 6] {
                m.named_constants.insert(
                    id,
                    sims_formats::tuning::Constants {
                        version: 1,
                        entries: vec![],
                    },
                );
            }
            for id in [501, 502, 503, 504] {
                m.strings.insert(id, vec![String::new(); 9]);
            }
            for id in 1..=3 {
                let d = initial
                    .vm
                    .world
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .definition
                    .as_mut()
                    .unwrap();
                d.object = 3;
                d.guid = 3;
                initial.vm.start_object_lifecycle(&p, id).unwrap();
            }
            initial
                .vm
                .world
                .globals
                .extend([(15, 0), (32, 0), (33, 0), (34, 0), (36, 0)]);
            initial.vm.world.refresh = Some(Default::default());
            let a = initial.architecture.as_mut().unwrap();
            a.original.as_mut().unwrap().pool.fill(255);
            a.bind_footprints(&p).unwrap();
            a.publish_room_context(&mut initial.vm.world);
            initial
                .enable_household(
                    &p,
                    crate::clock::LotClock {
                        hour: 12,
                        day: 1,
                        period: 0,
                        minute: 0,
                        second: 0,
                        month: 1,
                        year: 1997,
                        elapsed_days: 0,
                        month_days: 30,
                        ticks: 0,
                        statistics: [[0; 8]; 6],
                    },
                )
                .unwrap();
            let caller = initial.vm.object_threads[&4];
            initial
                .vm
                .threads
                .get_mut(&caller)
                .unwrap()
                .stack
                .last_mut()
                .unwrap()
                .stack_object = 1;
            let mut outcomes = vec![];
            for budget in [1, 1000] {
                let mut s = initial.clone();
                let mut saved = 0;
                let mut callbacks = 0;
                for checkpoint in 0..100 {
                    let step = s.run_fixture_tick(&p, budget).unwrap_or_else(|error| {
                        panic!("Burn {reject}/{budget}/{checkpoint}: {error}")
                    });
                    assert_ne!(
                        step.stop,
                        crate::StepStop::Faulted,
                        "Burn {reject}/{budget}/{checkpoint}: {:?}",
                        s.vm.threads
                            .values()
                            .filter_map(|t| if let Status::Faulted(f) = &t.status {
                                Some(&f.message)
                            } else {
                                None
                            })
                            .collect::<Vec<_>>()
                    );
                    if s.native_burn.is_some() {
                        saved += 1;
                        callbacks +=
                            usize::from(s.vm.threads.values().any(|t| t.callback.is_some()));
                        assert!(s.advance_fixture_tick().is_err());
                        let mut bad = crate::Snapshot::capture(&p, &s);
                        bad.state.native_burn = None;
                        assert!(bad.restore(&p).is_err(), "lost Burn continuation accepted");
                    }
                    let bytes = serde_json::to_vec(&crate::Snapshot::capture(&p, &s)).unwrap();
                    s = serde_json::from_slice::<crate::Snapshot>(&bytes)
                        .unwrap()
                        .restore(&p)
                        .unwrap_or_else(|error| {
                            panic!("Burn snapshot {reject}/{budget}/{checkpoint}: {error}")
                        });
                    assert_eq!(s.vm.tick, 0);
                    if step.stop != crate::StepStop::Budget {
                        break;
                    }
                    assert!(checkpoint < 99);
                }
                assert_eq!(saved > 0, budget == 1);
                assert_eq!(callbacks > 0, budget == 1);
                assert!(
                    matches!(s.vm.threads[&caller].status, Status::Returned(value) if value == !reject)
                );
                assert_eq!(s.vm.world.globals[&34], if reject { 11 } else { 111 });
                assert_eq!(s.vm.world.objects.contains_key(&5), !reject);
                assert_eq!(
                    s.vm.world.fires.registered,
                    if reject { vec![] } else { vec![5, 5] }
                );
                assert_eq!(
                    s.vm.world.fires.activity_changes,
                    if reject {
                        vec![true, false]
                    } else {
                        vec![true]
                    }
                );
                if !reject {
                    let at = s.vm.world.positions[&5];
                    assert_eq!([at.x, at.y, at.level], [24, 40, 1]);
                    assert_eq!(
                        s.native_lot
                            .as_ref()
                            .unwrap()
                            .placement
                            .links
                            .head(&s.vm.world, [2, 1, 1]),
                        Some(5)
                    );
                }
                assert!(s.deletion.is_empty());
                assert!(s.native_burn.is_none());
                outcomes.push(s);
            }
            assert_eq!(outcomes[0], outcomes[1]);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    thread: ThreadId,
    request: sims_vm::burn::Request,
    controller: Controller,
    fire: Option<Resource>,
    factory: Option<ObjectId>,
    acceptance: Option<lot_acceptance::live::Execution>,
    placement: Option<lot_placement::live::Execution>,
    cleanup: Option<Cleanup>,
    #[serde(default)]
    pub(crate) reset: Option<crate::person_reset::Execution>,
}

fn destination(at: Position) -> Descriptor {
    Descriptor {
        on_lot: true,
        position: at.xy,
        level: at.level,
        container: None,
        slot: 0,
    }
}
impl Execution {
    pub(crate) fn request(&self) -> &sims_vm::burn::Request {
        &self.request
    }
    fn new(thread: ThreadId, request: sims_vm::burn::Request) -> Self {
        Self {
            thread,
            controller: Controller::new(Input {
                target: request.target,
                mode: request.operands[0],
                flags: request.operands[1],
            }),
            request,
            fire: None,
            factory: None,
            acceptance: None,
            placement: None,
            cleanup: None,
            reset: None,
        }
    }
    pub(crate) fn owns_deletion(&self, target: ObjectId) -> bool {
        self.cleanup
            .as_ref()
            .is_some_and(|e| e.destroying && e.object == target)
    }
    pub fn resetting_person(&self) -> Option<ObjectId> {
        self.reset
            .as_ref()
            .map(crate::person_reset::Execution::object)
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.acceptance
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .placement
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .cleanup
                .as_ref()
                .is_some_and(|e| e.placement.owns_callback(vm, p, thread, depth))
            || self
                .reset
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub(crate) fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        self.controller.validate()?;
        let t =
            s.vm.threads
                .get(&self.thread)
                .ok_or("Burn owner thread absent")?;
        if s.native_lot.is_none()
            || !self.request.started
            || !std::iter::once(&t.status)
                .chain(
                    t.callback
                        .iter()
                        .flat_map(|b| b.chain())
                        .map(|b| &b.resume_status),
                )
                .chain(
                    t.service
                        .iter()
                        .flat_map(|b| b.chain())
                        .map(|b| &b.resume_status),
                )
                .any(|status| matches!(status, Status::AwaitingBurn(r) if r == &self.request))
            || self.controller.input()
                != (Input {
                    target: self.request.target,
                    mode: self.request.operands[0],
                    flags: self.request.operands[1],
                })
            || self.fire.as_ref().is_some_and(|r| {
                Resource::loaded_fire(p)
                    .as_ref()
                    .ok()
                    .and_then(|q| q.as_ref())
                    != Some(r)
            })
        {
            return Err("invalid live Burn request binding".into());
        }
        let active = usize::from(self.factory.is_some())
            + usize::from(self.acceptance.is_some())
            + usize::from(self.placement.is_some())
            + usize::from(self.cleanup.is_some())
            + usize::from(self.reset.is_some());
        if active > 1 || active != 0 && self.fire.is_none() {
            return Err("overlapping or unbound Burn services".into());
        }
        if matches!(self.controller.pending(), Some(Request::Reset(_)))
            && self.reset.as_ref().is_none_or(|e| e.complete())
        {
            return Err("Burn lost its active person Reset continuation".into());
        }
        if let Some(e) = &self.reset {
            if self.controller.pending() != Some(&Request::Reset(e.object())) {
                return Err("unbound Burn person Reset".into());
            }
            e.validate(s, p)?;
        }
        if let Some(id) = self.factory {
            let object =
                s.vm.world
                    .objects
                    .get(&id)
                    .ok_or("Burn factory object absent")?;
            let resource = self.fire.as_ref().unwrap();
            if self.controller.pending() != Some(&Request::Factory)
                || !s.unplaced.contains(&id)
                || object
                    .definition
                    .as_ref()
                    .is_none_or(|d| d.source != resource.source || d.object != resource.definition)
                || !s.vm.lifecycles.contains_key(&id)
            {
                return Err("unbound Burn factory".into());
            }
        }
        if let Some(e) = &self.acceptance {
            e.validate(&s.vm, p)?;
            if !matches!(self.controller.pending(), Some(Request::Check(id, at)) if e.binding() == (*id, at.xy, at.level))
                || e.container_binding().is_some()
            {
                return Err("unbound Burn acceptance".into());
            }
        }
        if let Some(e) = &self.placement {
            let Some(Request::Place(id, at)) = self.controller.pending() else {
                return Err("unbound Burn placement".into());
            };
            e.validate(&s.vm, p, *id)?;
            if e.operation.binding() != (*id, Some(destination(*at))) {
                return Err("Burn placement destination mismatch".into());
            }
        }
        if let Some(e) = &self.cleanup {
            if self.controller.pending() != Some(&Request::Delete(e.object))
                || e.placement.operation.binding() != (e.object, None)
                || e.destroying
                    && (!e.placement.complete
                        || s.deletion.first().map(|d| d.target) != Some(e.object))
            {
                return Err("unbound Burn failure cleanup".into());
            }
            e.placement.validate(&s.vm, p, e.object)?;
        }
        Ok(())
    }
}
impl Simulation {
    pub fn pending_burn_resource(&self) -> bool {
        self.native_burn
            .as_ref()
            .is_some_and(|e| e.controller.pending() == Some(&Request::Resource) && e.fire.is_none())
    }
    pub(crate) fn service_burn(
        &mut self,
        p: &Program,
        thread: ThreadId,
        mut request: sims_vm::burn::Request,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if self.native_lot.is_none() {
            return Err("Burn requires the original physical-lot host".into());
        }
        if self.native_burn.is_some() {
            return Err("nested Burn requires original shared candidate-vector semantics".into());
        }
        if request.started {
            return Err("orphan live Burn continuation".into());
        }
        if !matches!(&self.vm.threads.get(&thread).ok_or("Burn caller absent")?.status, Status::AwaitingBurn(r) if *r == request)
        {
            return Err("stale Burn request".into());
        }
        request.started = true;
        self.vm.threads.get_mut(&thread).unwrap().status = Status::AwaitingBurn(request.clone());
        self.native_burn = Some(Execution::new(thread, request));
        self.drive_native_burn(p, remaining, step)
    }
    pub(crate) fn drive_native_burn(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let Some(mut e) = self.native_burn.take() else {
            return Ok(true);
        };
        let result = self.drive_burn_inner(p, &mut e, remaining, step);
        if !matches!(result, Ok(true)) {
            self.native_burn = Some(e);
        }
        result
    }
    fn drive_burn_inner(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            let request = e
                .controller
                .next(&mut Live::new(p, self, e.fire.as_ref())?)?;
            let Some(request) = request else {
                self.vm.finish_burn(
                    p,
                    e.thread,
                    &e.request,
                    e.controller.result().ok_or("Burn result absent")?,
                )?;
                return Ok(true);
            };
            let response = match request {
                Request::Resource => {
                    let Some(resource) = Resource::loaded_fire(p)? else {
                        return Ok(false);
                    };
                    e.fire = Some(resource);
                    Response::Resource(true)
                }
                Request::Factory => {
                    if e.factory.is_none() {
                        let resource = e.fire.as_ref().ok_or("Burn fire resource absent")?;
                        let words = &p.modules[&resource.source].object_definition_words
                            [&resource.definition];
                        if words.len() < 68 || [2, 8, 9].contains(&words[9]) || words[10] != 0 {
                            return Err("Burn fire needs an ordinary single-part factory".into());
                        }
                        let Some(id) =
                            (1..=i16::MAX).find(|id| !self.vm.world.objects.contains_key(id))
                        else {
                            e.controller.respond(Response::Created(None))?;
                            continue;
                        };
                        self.initialize_object_factory(
                            p,
                            id,
                            &resource.source,
                            resource.definition,
                            false,
                        )?;
                        e.factory = Some(id);
                    }
                    let id = e.factory.unwrap();
                    if !self.drive_object_factory(p, id, remaining, step)? {
                        return Ok(false);
                    }
                    self.native_lot.as_mut().unwrap().observers.push(
                        lot_placement::Request::Signal {
                            event: 0xe2,
                            value: id,
                        },
                    );
                    e.factory = None;
                    Response::Created(Some(id))
                }
                Request::Check(id, at) => {
                    if e.acceptance.is_none() {
                        e.acceptance = Some(lot_acceptance::live::Execution::new(
                            self, id, at.xy, at.level,
                        )?);
                    }
                    let check = e.acceptance.as_mut().unwrap();
                    if !self.drive_native_acceptance(p, check, remaining, step)? {
                        return Ok(false);
                    }
                    let result = check
                        .outcome()
                        .ok_or("Burn acceptance result absent")?
                        .accepted;
                    e.acceptance = None;
                    Response::Accepted(result)
                }
                Request::Place(id, at) => {
                    if e.placement.is_none() {
                        let operation = lot_placement::VirtualOperation::set(
                            &self.vm.world,
                            &self.native_lot.as_ref().unwrap().placement.links,
                            id,
                            destination(at),
                            true,
                        )?;
                        e.placement = Some(lot_placement::live::Execution::new_virtual(operation));
                    }
                    if !self.drive_live_placement(
                        p,
                        e.placement.as_mut().unwrap(),
                        remaining,
                        step,
                    )? {
                        return Ok(false);
                    }
                    e.placement = None;
                    Response::Done
                }
                Request::Delete(id) => {
                    if e.cleanup.is_none() {
                        self.native_removal_tree(p, id)?;
                        let after_room = crate::lot_removal::immediate_room(&self.vm.world, id)?;
                        let operation = lot_placement::VirtualOperation::remove(
                            &self.vm.world,
                            &self.native_lot.as_ref().unwrap().placement.links,
                            id,
                            true,
                        )?;
                        e.cleanup = Some(Cleanup {
                            object: id,
                            placement: lot_placement::live::Execution::new_virtual(operation),
                            after_room,
                            destroying: false,
                        });
                    }
                    let cleanup = e.cleanup.as_mut().unwrap();
                    if !cleanup.destroying {
                        if !self.drive_live_placement(p, &mut cleanup.placement, remaining, step)? {
                            return Ok(false);
                        }
                        if !self.deletion.is_empty() {
                            return Err("nested Burn cleanup requires a destructor boundary".into());
                        }
                        let mut d = removal::Deletion::new(id);
                        d.after_delete_room = cleanup.after_room;
                        self.deletion.push(d);
                        cleanup.destroying = true;
                    }
                    if !self.drive_deletion_stack(p, remaining, step, false)? {
                        return Ok(false);
                    }
                    e.cleanup = None;
                    Response::Done
                }
                Request::Reset(id) => {
                    if e.reset.is_none() {
                        e.reset = Some(crate::person_reset::Execution::new(self, id)?);
                    }
                    if !self.drive_person_reset(p, e.reset.as_mut().unwrap(), remaining, step)? {
                        return Ok(false);
                    }
                    e.reset = None;
                    Response::Done
                }
                Request::Refresh(id) => {
                    // Person virtual +0x74 is 45e519, the same cache/profile
                    // refresh used after Find Location, not an animator tick.
                    self.refresh_find_location_appearance(id)?;
                    Response::Done
                }
            };
            e.controller.respond(response)?;
        }
    }
}
