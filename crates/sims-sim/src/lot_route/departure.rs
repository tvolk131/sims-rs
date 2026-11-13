//! 0x4832d4 seated departure selection, followed by state-1 Stand dispatch.
//! Clearance callbacks are ordinary serializable native floor continuations.
use super::candidate::{Check, Input};
use crate::{Point, Simulation, Step, lot_acceptance, routing::Departure};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, RouteRequest, ThreadId, Vm};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Probe {
    pub position: [i32; 2],
    pub direction: u8,
    relative: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scan {
    origin: Point,
    target: Point,
    allowed: u16,
    next: u8,
    pending: Option<Probe>,
    results: Vec<(Probe, u8)>,
    selected: Departure,
    best: Option<i32>,
}
impl Scan {
    pub fn new(origin: Point, target: Point, allowed: u16) -> Self {
        Self {
            origin,
            target,
            allowed,
            next: 0,
            pending: None,
            results: vec![],
            selected: Departure {
                position: origin,
                flags: 0,
            },
            best: None,
        }
    }
    /// Native code re-reads the captured seat's facing before every probe.
    pub fn next(&mut self, facing: i16) -> Option<Probe> {
        if self.pending.is_some() {
            return self.pending;
        }
        while self.next < 8 {
            if self.allowed & (1 << self.next) != 0 {
                let direction = ((i32::from(facing) + i32::from(self.next)) & 7) as u8;
                let (dx, dy) = crate::routing::DIRECTION_OFFSETS[usize::from(direction)];
                let probe = Probe {
                    position: [
                        self.origin.x.wrapping_add(dx),
                        self.origin.y.wrapping_add(dy),
                    ],
                    direction,
                    relative: self.next,
                };
                self.pending = Some(probe);
                return Some(probe);
            }
            self.next += 1;
        }
        None
    }
    pub fn resume(&mut self, probe: Probe, classification: u8) -> Result<(), String> {
        if self.pending != Some(probe) || classification > 6 {
            return Err("departure response mismatch".into());
        }
        if matches!(classification, 5 | 6) {
            let dx = self.target.x.wrapping_sub(probe.position[0]);
            let dy = self.target.y.wrapping_sub(probe.position[1]);
            let distance = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
            if self.best.is_none_or(|previous| distance < previous) {
                self.best = Some(distance);
                self.selected = Departure {
                    position: Point::new(probe.position[0], probe.position[1]),
                    flags: 1 << probe.relative,
                };
            }
        }
        self.results.push((probe, classification));
        self.pending = None;
        self.next += 1;
        Ok(())
    }
    pub fn outcome(&self) -> Option<Departure> {
        (self.next == 8 && self.pending.is_none()).then_some(self.selected)
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut rebuilt = Self::new(self.origin, self.target, self.allowed);
        for &(probe, classification) in &self.results {
            if rebuilt.next(i16::from(probe.direction) - i16::from(probe.relative)) != Some(probe) {
                return Err("invalid departure probe sequence".into());
            }
            rebuilt.resume(probe, classification)?;
        }
        if let Some(probe) = self.pending {
            if rebuilt.next(i16::from(probe.direction) - i16::from(probe.relative)) != Some(probe) {
                return Err("invalid pending departure probe".into());
            }
        } else if self.next == 8 {
            rebuilt.next(0);
        }
        if rebuilt != *self {
            return Err("invalid departure scan state".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Execution {
    caller: ObjectId,
    seat: ObjectId,
    input: Input,
    scan: Scan,
    clearance: Option<Check>,
    floor: Option<lot_acceptance::live::Execution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    initial_blocked_object: Option<i16>,
    pub blocked_object: i16,
}
impl Execution {
    pub fn begin(s: &Simulation, p: &Program, r: &RouteRequest) -> Result<Option<Self>, String> {
        let control = s
            .route_control
            .get(&r.id)
            .and_then(|control| control.original.as_ref())
            .ok_or("departure route context absent")?;
        Self::begin_potential(
            s,
            p,
            r,
            (control.ignored != 0).then_some(control.ignored),
            control.blocked_object,
        )
    }
    pub(super) fn begin_potential(
        simulation: &Simulation,
        program: &Program,
        request: &RouteRequest,
        ignored_person: Option<ObjectId>,
        blocked_object: i16,
    ) -> Result<Option<Self>, String> {
        let mut root = request.caller;
        let mut seen = std::collections::BTreeSet::new();
        let seat = loop {
            if !seen.insert(root) {
                return Err("cyclic zero-slot predecessor chain".into());
            }
            let object = simulation
                .vm
                .world
                .objects
                .get(&root)
                .ok_or("departure object absent")?;
            let physical = *object
                .data
                .get(&14)
                .ok_or("departure physical slot absent")?;
            let predecessor = *object.data.get(&26).ok_or("departure predecessor absent")?;
            let parent = (predecessor != 0
                && simulation.vm.world.objects.contains_key(&predecessor))
            .then_some(predecessor);
            if physical != 0 || parent.is_none() {
                break parent;
            }
            root = parent.unwrap();
        };
        let Some(seat) = seat else {
            return Ok(None);
        };
        let definition = simulation.vm.world.objects[&seat]
            .definition
            .as_ref()
            .ok_or("departure seat definition absent")?;
        let module = program
            .modules
            .get(&definition.source)
            .ok_or("departure seat resource absent")?;
        if module
            .object_functions
            .get(&definition.object)
            .and_then(|functions| functions.get(26))
            .ok_or("departure seat function table absent")?
            .action
            == 0
        {
            return Ok(None);
        }
        let allowed = *module
            .object_definition_words
            .get(&definition.object)
            .and_then(|words| words.get(0x84 / 2))
            .ok_or("departure seat mask absent")? as u16;
        simulation.vm.world.objects[&seat]
            .data
            .get(&1)
            .ok_or("departure seat facing absent")?;
        let actor = &simulation.vm.world.positions[&request.caller];
        let target = &simulation.vm.world.positions[&request.target];
        let (slot, _) = super::live::descriptor(&request.destination)?;
        Ok(Some(Self {
            caller: request.caller,
            seat,
            input: Input {
                size: super::active_size(
                    &simulation.vm.world,
                    simulation.native_lot.as_ref().unwrap().placement.links.size,
                )?,
                position: [actor.x, actor.y],
                target: [target.x, target.y],
                level: actor.level,
                room: simulation.vm.world.objects[&request.caller].data[&29],
                flags: slot.flags,
                direction: None,
                ignored: ignored_person,
            },
            scan: Scan::new(
                Point::new(actor.x, actor.y),
                Point::new(target.x, target.y),
                allowed,
            ),
            clearance: None,
            floor: None,
            initial_blocked_object: Some(blocked_object),
            blocked_object,
        }))
    }
    pub(super) fn validate_context(
        &self,
        flags: u32,
        ignored_person: Option<ObjectId>,
        blocked_seed: Option<i16>,
    ) -> Result<(), String> {
        if self.input.flags != flags
            || self.input.ignored != ignored_person
            || blocked_seed.is_some() && blocked_seed != self.initial_blocked_object
        {
            return Err("departure potential context mismatch".into());
        }
        Ok(())
    }
    pub fn outcome(&self) -> Option<Departure> {
        self.scan.outcome()
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.floor
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub fn validate(&self, s: &Simulation, p: &Program, caller: ObjectId) -> Result<(), String> {
        self.scan.validate()?;
        let definition =
            s.vm.world
                .objects
                .get(&self.seat)
                .and_then(|seat| seat.definition.as_ref())
                .ok_or("departure seat definition absent")?;
        let module = p
            .modules
            .get(&definition.source)
            .ok_or("departure seat resource absent")?;
        if !s.vm.world.objects[&self.seat].data.contains_key(&1)
            || module
                .object_functions
                .get(&definition.object)
                .and_then(|functions| functions.get(26))
                .is_none()
            || module
                .object_definition_words
                .get(&definition.object)
                .and_then(|words| words.get(0x84 / 2))
                .is_none()
        {
            return Err("departure seat resources absent".into());
        }
        if self.caller != caller
            || self.input.position != [self.scan.origin.x, self.scan.origin.y]
            || self.input.target != [self.scan.target.x, self.scan.target.y]
            || self.input.size
                != super::active_size(
                    &s.vm.world,
                    s.native_lot.as_ref().unwrap().placement.links.size,
                )?
            || !(1..=2).contains(&self.input.level)
            || self.input.direction.is_some()
            || self.scan.pending.is_some() != self.clearance.is_some()
            || !s.vm.world.objects.contains_key(&self.seat)
            || self.outcome().is_some() && (self.floor.is_some() || self.clearance.is_some())
        {
            return Err("invalid departure binding".into());
        }
        if let Some(check) = &self.clearance {
            let probe = self
                .scan
                .pending
                .ok_or("departure clearance without probe")?;
            let mut expected = self.input.clone();
            expected.position = probe.position;
            expected.direction = Some(probe.direction);
            if check.input() != &expected {
                return Err("departure clearance mismatch".into());
            }
        }
        if let Some(floor) = &self.floor {
            floor.validate(&s.vm, p)?;
            let check = self
                .clearance
                .as_ref()
                .ok_or("departure floor without clearance")?;
            if floor.binding() != (caller, check.input().position, self.input.level)
                || check.pending()
                    != Some(super::candidate::Request::Placement {
                        position: check.input().position,
                        level: self.input.level,
                    })
            {
                return Err("departure floor mismatch".into());
            }
        }
        Ok(())
    }
}
impl Simulation {
    pub(crate) fn drive_native_departure(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            if e.clearance.is_none() {
                let facing = *self
                    .vm
                    .world
                    .objects
                    .get(&e.seat)
                    .and_then(|o| o.data.get(&1))
                    .ok_or("departure seat disappeared")?;
                let Some(probe) = e.scan.next(facing) else {
                    return Ok(true);
                };
                let mut input = e.input.clone();
                input.position = probe.position;
                input.direction = Some(probe.direction);
                e.clearance = Some(Check::new(
                    input,
                    self.native_lot.as_ref().unwrap().diagnostics,
                    e.blocked_object,
                )?);
            }
            if !self.drive_candidate_clearance(
                p,
                e.caller,
                e.clearance.as_mut().unwrap(),
                &mut e.floor,
                remaining,
                step,
            )? {
                return Ok(false);
            }
            let outcome = e.clearance.as_ref().unwrap().outcome().unwrap();
            e.blocked_object = outcome.blocked_object;
            e.scan
                .resume(e.scan.pending.unwrap(), outcome.classification)?;
            e.clearance = None;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    NotSeated,
    Keep,
    Failed,
    Started,
}
pub trait Services {
    fn posture(&self) -> i16;
    fn reuse(&mut self) -> Result<bool, String>;
    fn container(&mut self) -> Result<Option<ObjectId>, String>;
    fn function(&mut self, seat: ObjectId, number: usize) -> Result<u16, String>;
    fn flags(&self) -> u16;
    fn push(&mut self, seat: ObjectId, bhav: u16, flags: u16) -> Result<bool, String>;
    fn publish(&mut self, flags: u16) -> Result<(), String>;
}
/// State 1 after 0x48235a: current-seat reuse precedes container/Sit/Stand checks.
pub fn dispatch(host: &mut impl Services) -> Result<Dispatch, String> {
    if host.posture() != 1 {
        return Ok(Dispatch::NotSeated);
    }
    if host.reuse()? {
        return Ok(Dispatch::Keep);
    }
    if let Some(seat) = host.container()? {
        if host.function(seat, 26)? != 0 {
            let stand = host.function(seat, 27)?;
            if stand != 0 && host.push(seat, stand, host.flags())? {
                host.publish(host.flags())?;
                return Ok(Dispatch::Started);
            }
        }
    }
    Ok(Dispatch::Failed)
}
/// State 5 restores ignored-person state before testing Stand's result/posture.
pub fn returned(success: bool, posture: i16) -> bool {
    success && posture == 0
}

struct Host<'a> {
    s: &'a mut Simulation,
    p: &'a Program,
    thread: ThreadId,
    r: &'a RouteRequest,
    candidates: &'a [crate::routing::Candidate],
    flags: u16,
}
impl Services for Host<'_> {
    fn posture(&self) -> i16 {
        self.s.vm.world.objects[&self.r.caller]
            .person
            .as_ref()
            .unwrap()[&0]
    }
    fn reuse(&mut self) -> Result<bool, String> {
        for candidate in self.candidates {
            if candidate.seat != 0
                && self.s.vm.world.objects.contains_key(&candidate.seat)
                && self.function(candidate.seat, 26)? != 0
                && self.container()? == Some(candidate.seat)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn container(&mut self) -> Result<Option<ObjectId>, String> {
        self.s
            .native_lot
            .as_ref()
            .unwrap()
            .placement
            .links
            .capture(&mut self.s.vm.world, self.r.caller)
            .map(|d| d.container)
    }
    fn function(&mut self, seat: ObjectId, number: usize) -> Result<u16, String> {
        let d = self.s.vm.world.objects[&seat]
            .definition
            .as_ref()
            .ok_or("Stand seat definition absent")?;
        Ok(self
            .p
            .modules
            .get(&d.source)
            .and_then(|m| m.object_functions.get(&d.object))
            .and_then(|f| f.get(number))
            .ok_or("Stand seat function table absent")?
            .action)
    }
    fn flags(&self) -> u16 {
        self.flags
    }
    fn push(&mut self, seat: ObjectId, bhav: u16, flags: u16) -> Result<bool, String> {
        let owner = self.s.vm.world.objects[&seat]
            .definition
            .as_ref()
            .unwrap()
            .source
            .clone();
        self.s.vm.push_service_call(
            self.p,
            self.thread,
            sims_vm::Start {
                owner: &owner,
                bhav,
                caller: self.r.caller,
                callee: seat,
                stack_object: seat,
                args: vec![flags as i16, 0, 0, 0],
                check: false,
            },
        )?;
        self.s
            .route_control
            .get_mut(&self.r.id)
            .unwrap()
            .original
            .as_mut()
            .unwrap()
            .standing = Some(seat);
        Ok(true)
    }
    fn publish(&mut self, flags: u16) -> Result<(), String> {
        self.s.person(self.r.caller, 37, flags as i16)?;
        self.s.person(self.r.caller, 72, 0)
    }
}
impl Simulation {
    pub(crate) fn service_original_stand(
        &mut self,
        p: &Program,
        thread: ThreadId,
        request: u64,
        candidates: &[crate::routing::Candidate],
        flags: u16,
        events: &mut Vec<crate::Event>,
    ) -> Result<bool, String> {
        let r = crate::contention::pending_route(&self.vm.threads[&thread], request)
            .ok_or("Stand route absent")?
            .clone();
        let result = dispatch(&mut Host {
            s: self,
            p,
            thread,
            r: &r,
            candidates,
            flags,
        })?;
        match result {
            Dispatch::NotSeated => Ok(false),
            Dispatch::Keep => {
                self.begin_native_route_completion(thread, &r, true, 0)?;
                self.event(
                    events,
                    thread,
                    "original-lot route reused current seat".into(),
                );
                Ok(true)
            }
            Dispatch::Failed => {
                // Native state 7 cleanup, without state-16 failure feedback.
                let result = self.route_control[&request].controller.result;
                self.begin_native_route_completion(thread, &r, false, i32::from(result))?;
                Ok(true)
            }
            Dispatch::Started => {
                let seat = self.route_control[&request]
                    .original
                    .as_ref()
                    .unwrap()
                    .standing
                    .unwrap();
                self.event(
                    events,
                    thread,
                    format!("original-lot route invokes Stand on #{seat}; departure flags {flags}"),
                );
                Ok(true)
            }
        }
    }
}
