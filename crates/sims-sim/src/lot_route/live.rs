//! Native imported-lot destination preparation. This owns synchronous clearance
//! work. Strict path construction requires an explicitly retained native map.
use super::candidate::{Check, Input, Reply, Request};
use crate::{
    Simulation, Step, lot_acceptance,
    routing::{Candidate, CandidateSet, PointCandidate, PostureScan, StandingScan},
};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, RouteRequest, ThreadId, Vm};

/// Temporary 0x48163e context used by shoo. The ignored person pointer is
/// distinct from the constructor's captured person[72] used by clearance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Potential {
    pub ignored_person: Option<ObjectId>,
    pub ignore_people: bool,
    pub ignored_word: i16,
    pub blocked_object: i16,
    pub retained_result: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    pub potential: Option<Potential>,
    pub thread: ThreadId,
    pub request: u64,
    pub caller: ObjectId,
    pub target: ObjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    potential_request: Option<RouteRequest>,
    target_position: [i32; 2],
    target_facing: u8,
    descriptor_facing: i32,
    saved_facing: u8,
    candidate_limit: usize,
    subset_rng: Option<u32>,
    retained_result: u8,
    departure: Option<Box<super::departure::Execution>>,
    prepared: bool,
    /// State 1 crosses this decision once, before resumable path callbacks.
    posture_checked: bool,
    pub point: Option<PointCandidate>,
    pub standing: Option<StandingScan>,
    pub posture: Option<PostureScan>,
    input: Input,
    clearance: Option<Check>,
    blocked_object: i16,
    pub goal: Option<crate::Destination>,
    pub selected_seat: Option<Candidate>,
    floor: Option<lot_acceptance::live::Execution>,
    pub candidates: Option<CandidateSet>,
    pub obstacles: Option<super::obstacle::Prefix>,
    pub policy: super::path::Policy,
    pub path: Option<super::path::Inputs>,
    pub failed: bool,
}
impl Execution {
    fn path_result(&self) -> u8 {
        self.path
            .as_ref()
            .and_then(|p| p.failure_result)
            .unwrap_or_else(|| {
                self.candidates
                    .as_ref()
                    .map_or(self.retained_result, |s| s.result)
            })
    }
    pub(crate) fn potential_result(&self) -> (i16, u8) {
        (self.blocked_object, self.path_result())
    }
    pub(crate) fn mark_potential_prepared(&mut self) {
        self.posture_checked = true;
    }
    pub fn begin(
        s: &mut Simulation,
        p: &Program,
        thread: ThreadId,
        r: &RouteRequest,
    ) -> Result<Self, String> {
        let mut execution = Self::prepare(s, thread, r, None, None)?;
        if let Some(departure) = super::departure::Execution::begin(s, p, r)? {
            execution = Self::prepare(s, thread, r, Some(Box::new(departure)), None)?;
        }
        s.native_lot
            .as_ref()
            .unwrap()
            .placement
            .links
            .capture(&mut s.vm.world, r.caller)?;
        execution.install_preparation(s)?;
        Ok(execution)
    }
    pub fn begin_potential(
        s: &mut Simulation,
        p: &Program,
        thread: ThreadId,
        r: &RouteRequest,
        settings: Potential,
    ) -> Result<Self, String> {
        if settings
            .ignored_person
            .is_some_and(|person| person == 0 || !s.vm.world.objects.contains_key(&person))
        {
            return Err("invalid potential ignored-person pointer".into());
        }
        let posture =
            s.vm.world
                .objects
                .get(&r.caller)
                .and_then(|actor| actor.person.as_ref())
                .and_then(|person| person.get(&0))
                .ok_or("potential route caller posture absent")?;
        if !(0..=1).contains(posture) {
            return Err(format!(
                "unsupported potential route caller posture {posture}"
            ));
        }
        let actor = &s.vm.world.objects[&r.caller];
        if [4, 8, 30, 42]
            .into_iter()
            .any(|field| !actor.data.contains_key(&field))
            || [68, 72, 73, 84]
                .into_iter()
                .any(|field| !actor.person.as_ref().unwrap().contains_key(&field))
        {
            return Err("potential route caller clearance fields absent".into());
        }
        let mut execution = Self::prepare(s, thread, r, None, Some(settings.clone()))?;
        if let Some(departure) = super::departure::Execution::begin_potential(
            s,
            p,
            r,
            settings.ignored_person,
            settings.blocked_object,
        )? {
            execution = Self::prepare(s, thread, r, Some(Box::new(departure)), Some(settings))?;
        }
        s.native_lot
            .as_ref()
            .unwrap()
            .placement
            .links
            .capture(&mut s.vm.world, r.caller)?;
        execution.install_preparation(s)?;
        Ok(execution)
    }
    /// Reprepare the same temporary semantic context.48235a can lower+98,
    /// but only481693 constructs its initial48; a small room does not reset it.
    pub(super) fn begin_retained_potential(
        s: &mut Simulation,
        p: &Program,
        thread: ThreadId,
        r: &RouteRequest,
        settings: Potential,
        retained_limit: usize,
    ) -> Result<Self, String> {
        if ![8, 48].contains(&retained_limit) {
            return Err("invalid retained potential candidate limit".into());
        }
        let mut execution = Self::begin_potential(s, p, thread, r, settings)?;
        execution.candidate_limit = execution.candidate_limit.min(retained_limit);
        Ok(execution)
    }
    pub(crate) fn potential_candidate_limit(&self) -> usize {
        self.candidate_limit
    }
    fn prepare(
        s: &Simulation,
        thread: ThreadId,
        r: &RouteRequest,
        departure: Option<Box<super::departure::Execution>>,
        potential: Option<Potential>,
    ) -> Result<Self, String> {
        let preparing = departure.as_ref().is_some_and(|d| d.outcome().is_none());
        let native = s.native_lot.as_ref().ok_or("native route host absent")?;
        let (slot, version) = descriptor(&r.destination)?;
        if !(4..=10).contains(&version)
            || slot.kind != 3
            || slot.offset[2] != 0.
            || slot.height != 0
            || slot.snap_target >= 0
        {
            return Err("unsupported original point route SLOT".into());
        }
        let actor =
            s.vm.world
                .objects
                .get(&r.caller)
                .ok_or("native route caller absent")?;
        if actor.person.is_none() {
            return Err("native point route requires a person".into());
        }
        let actor_position =
            s.vm.world
                .positions
                .get(&r.caller)
                .ok_or("native route caller position absent")?;
        if !(1..=2).contains(&actor_position.level) {
            return Err("unsupported native route caller level".into());
        }
        let target =
            s.vm.world
                .positions
                .get(&r.target)
                .ok_or("native route target absent")?;
        let target_position = [target.x, target.y];
        let target_object =
            s.vm.world
                .objects
                .get(&r.target)
                .ok_or("native route target object absent")?;
        let facing = *target_object
            .data
            .get(&1)
            .ok_or("native route target facing absent")?;
        if !(0..=7).contains(&facing) {
            return Err("invalid native route target facing".into());
        }
        let target_facing = facing as u8;
        let saved_facing = u8::try_from(*actor.data.get(&1).ok_or("native actor facing absent")?)
            .map_err(|_| "invalid native actor facing")?;
        if saved_facing > 7 {
            return Err("invalid native actor facing".into());
        }
        let actor_room = *actor.data.get(&29).ok_or("native actor room absent")?;
        let target_room = *target_object
            .data
            .get(&29)
            .ok_or("native target room absent")?;
        if potential.is_none()
            && crate::routing::needs_portal(
                actor_room,
                target_room,
                *actor
                    .person
                    .as_ref()
                    .unwrap()
                    .get(&68)
                    .ok_or("native person field 68 absent")?,
            )
        {
            return Err(format!(
                "original-lot inter-room route adapter required: caller #{} room {}, target #{} room {}; native portal selection precedes destination candidates",
                r.caller, actor_room, r.target, target_room
            ));
        }
        if s.vm.world.positions[&r.caller].level != target.level {
            return Err("original-lot cross-floor route adapter required".into());
        }
        let candidate_limit = if preparing && potential.is_some() {
            48
        } else {
            s.route_candidate_limit(r.caller)
        };
        let (retained_result, blocked_object, ignored) = if let Some(settings) = &potential {
            (
                settings.retained_result,
                departure
                    .as_ref()
                    .filter(|departure| departure.outcome().is_some())
                    .map_or(settings.blocked_object, |departure| {
                        departure.blocked_object
                    }),
                settings.ignored_word,
            )
        } else {
            let control = s
                .route_control
                .get(&r.id)
                .ok_or("native route controller absent")?;
            let context = control
                .original
                .as_ref()
                .ok_or("native route context absent")?;
            (
                u8::try_from(control.controller.result)
                    .map_err(|_| "invalid native route result")?,
                context.blocked_object,
                context.ignored,
            )
        };
        let point = (!preparing && slot.flags & 255 == 0)
            .then(|| {
                PointCandidate::prepare(
                    target_position,
                    [slot.offset[0], slot.offset[1]],
                    target_facing,
                    slot.flags,
                )
            })
            .transpose()?;
        let mut posture = None;
        let standing = if !preparing && point.is_none() {
            let mut request = crate::routing::standing_slot(
                &slot,
                version,
                s.body(r.target)?,
                s.body(r.caller)?,
            )?;
            if let Some(selected) = departure.as_ref().and_then(|d| d.outcome()) {
                request.preferred = [selected.position.x, selected.position.y];
            }
            // Native 0x4359ea on the target's cached tile, rotated into SLOT axes.
            if slot.flags & 0x800 == 0 {
                let at = [
                    (target.y >> 4) as i8,
                    (target.x >> 4) as i8,
                    target.level as i8,
                ];
                let rotation = if slot.flags & 0x200 != 0 {
                    0
                } else {
                    target_facing
                };
                let Reply::Walls(walls) = Check::world_reply(
                    Request::Walls { tile: at, rotation },
                    &s.vm.world,
                    &native.placement.links,
                    s.architecture.as_ref().unwrap().original.as_ref().unwrap(),
                )?
                .unwrap() else {
                    unreachable!()
                };
                request.walls = walls;
            }
            if slot.sitting != 0 {
                posture = Some(PostureScan::new(request, slot.sitting, blocked_object)?);
                None
            } else {
                Some(StandingScan::new(request)?)
            }
        } else {
            None
        };
        let input = Input {
            size: super::active_size(&s.vm.world, native.placement.links.size)?,
            position: point.as_ref().map_or(target_position, |p| p.position),
            target: target_position,
            level: s.vm.world.positions[&r.caller].level,
            room: *actor
                .data
                .get(&29)
                .ok_or("native route actor room absent")?,
            flags: slot.flags,
            direction: point.as_ref().and_then(|p| p.direction),
            //4815a8 compares context+7c, not the person72 footprint word at+94.
            ignored: potential
                .as_ref()
                .map_or_else(|| (ignored != 0).then_some(ignored), |q| q.ignored_person),
        };
        let clearance = point
            .as_ref()
            .map(|_| Check::new(input.clone(), native.diagnostics, blocked_object))
            .transpose()?;
        let policy = native.routing_policy;
        Ok(Self {
            potential_request: potential.as_ref().map(|_| r.clone()),
            potential,
            thread,
            request: r.id,
            caller: r.caller,
            target: r.target,
            target_position,
            target_facing,
            descriptor_facing: slot.facing,
            saved_facing,
            candidate_limit,
            subset_rng: None,
            retained_result,
            departure,
            prepared: !preparing,
            posture_checked: false,
            point,
            standing,
            posture,
            input,
            clearance,
            blocked_object,
            goal: None,
            selected_seat: None,
            floor: None,
            candidates: None,
            obstacles: None,
            policy,
            path: None,
            failed: false,
        })
    }
    fn install_preparation(&self, s: &mut Simulation) -> Result<(), String> {
        if self.prepared {
            let ignored = self.potential.as_ref().map_or_else(
                || {
                    s.route_control[&self.request]
                        .original
                        .as_ref()
                        .unwrap()
                        .ignored
                },
                |settings| settings.ignored_word,
            );
            let actor = s.vm.world.objects.get_mut(&self.caller).unwrap();
            actor.person.as_mut().unwrap().insert(73, ignored);
            if let Some(point) = &self.point {
                actor.data.insert(1, i16::from(point.actor_facing));
                s.sync_script_directions()?;
            }
        }
        Ok(())
    }
    fn finish_candidates(&mut self, s: &mut Simulation, set: CandidateSet) -> Result<(), String> {
        // The route context retains the last occupant/collision ID even when
        // the scan produced a usable candidate. A later Sit failure reads it.
        if self.potential.is_none() {
            s.route_control
                .get_mut(&self.request)
                .ok_or("native candidate route control absent")?
                .original
                .as_mut()
                .ok_or("native candidate route context absent")?
                .blocked_object = self.blocked_object;
        }
        self.subset_rng = Some(s.vm.rng_state);
        self.candidates = Some(crate::routing::finish_candidate_scan(
            set,
            self.candidate_limit,
            &mut s.vm.rng_state,
            self.saved_facing,
            self.retained_result,
        ));
        let actor = s.vm.world.objects.get_mut(&self.caller).unwrap();
        actor.data.insert(1, i16::from(self.saved_facing));
        actor.person.as_mut().unwrap().insert(73, 0);
        s.sync_script_directions()
    }
    fn validate_finished_candidates(&self, set: &CandidateSet) -> Result<(), String> {
        let mut seed = self.subset_rng.ok_or("native subset RNG absent")?;
        let expected = crate::routing::finish_candidate_scan(
            set.clone(),
            self.candidate_limit,
            &mut seed,
            self.saved_facing,
            self.retained_result,
        );
        if self.candidates.as_ref() != Some(&expected) {
            return Err("native candidate subset mismatch".into());
        }
        Ok(())
    }
    pub(crate) fn destination(
        &self,
        c: &Candidate,
        path: &[crate::Point],
    ) -> Result<Option<crate::Destination>, String> {
        if c.seat != 0 {
            return Err(format!(
                "native selected-seat dispatch required: seat #{} flags {}",
                c.seat, c.flags
            ));
        }
        let Some(look) = self.endpoint_look(path)? else {
            return Ok(None);
        };
        Ok(Some(crate::Destination {
            position: crate::Point::new(c.position[1], c.position[0]),
            // The native look point is retained by the descriptor/path. The
            // current tuned locomotion timeline quantizes its final pose.
            facing: 1 << sims_vm::routing::direction(c.position, look),
            entry_flags: c.flags as i16,
        }))
    }

    pub(crate) fn endpoint_look(&self, path: &[crate::Point]) -> Result<Option<[i32; 2]>, String> {
        let points: Vec<_> = path.iter().map(|p| [p.y, p.x]).collect();
        crate::routing::endpoint_look(
            &points,
            self.target_position,
            self.target_facing,
            self.input.flags,
            self.descriptor_facing,
            self.saved_facing,
        )
    }

    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.departure
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .floor
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
            || self
                .obstacles
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        if self.potential.is_some() || self.potential_request.is_some() {
            return Err("temporary potential cannot use ordinary route owner".into());
        }
        let t =
            s.vm.threads
                .get(&self.thread)
                .ok_or("native route thread absent")?;
        let r = crate::contention::pending_route(t, self.request)
            .ok_or("native route request absent")?;
        self.validate_request(s, p, r)
    }
    /// A synchronous temporary query can belong to a different actor from
    /// the suspended route. Its owner supplies and validates that binding.
    pub(crate) fn validate_request(
        &self,
        s: &Simulation,
        p: &Program,
        r: &RouteRequest,
    ) -> Result<(), String> {
        let (slot, version) = descriptor(&r.destination)?;
        let (ignored, retained) = if let Some(q) = &self.potential {
            (q.ignored_word, q.retained_result)
        } else {
            let control = s
                .route_control
                .get(&self.request)
                .ok_or("native route controller absent")?;
            let context = control
                .original
                .as_ref()
                .ok_or("native route context absent")?;
            (context.ignored, control.controller.result as u8)
        };
        if (r.id, r.caller, r.target) != (self.request, self.caller, self.target)
            || s.vm
                .world
                .objects
                .get(&self.caller)
                .is_none_or(|actor| actor.person.is_none())
            || !s.vm.world.positions.contains_key(&self.caller)
            || !s.vm.world.objects.contains_key(&self.target)
            || !s.vm.world.positions.contains_key(&self.target)
            || !s.vm.threads.contains_key(&self.thread)
            || !(1..=2).contains(&self.input.level)
            || self.target_facing > 7
            || self.input.size
                != super::active_size(
                    &s.vm.world,
                    s.native_lot.as_ref().unwrap().placement.links.size,
                )?
            || self.input.target != self.target_position
            || self.input.flags != slot.flags
            || self.descriptor_facing != slot.facing
            || self.input.ignored
                != self
                    .potential
                    .as_ref()
                    .map_or_else(|| (ignored != 0).then_some(ignored), |q| q.ignored_person)
            || self.saved_facing > 7
            || self.retained_result != retained
            || self.potential_request.as_ref() != self.potential.as_ref().map(|_| r)
            || self.potential.is_some() && (self.goal.is_some() || self.selected_seat.is_some())
            || self.failed
            || ![8, 48].contains(&self.candidate_limit)
            || self.candidates.is_some() != self.subset_rng.is_some()
            || self.candidates.is_some() && self.floor.is_some()
            || self.posture_checked && (!self.prepared || self.candidates.is_none())
            || self.obstacles.is_some() && !self.posture_checked
            || s.visit.map(|(id, _)| id) != Some(self.thread)
            || s.native_lot
                .as_ref()
                .is_none_or(|n| n.routing_policy != self.policy)
        {
            return Err("invalid native route prefix binding".into());
        }
        if let Some(d) = &self.departure {
            d.validate(s, p, self.caller)?;
            d.validate_context(
                slot.flags,
                self.input.ignored,
                self.potential
                    .as_ref()
                    .map(|settings| settings.blocked_object),
            )?;
            if !self.prepared {
                if self.point.is_some()
                    || self.standing.is_some()
                    || self.posture.is_some()
                    || self.clearance.is_some()
                    || self.floor.is_some()
                    || self.candidates.is_some()
                    || self.obstacles.is_some()
                    || self.path.is_some()
                    || self.goal.is_some()
                    || self.selected_seat.is_some()
                {
                    return Err("destination preparation ran before departure".into());
                }
                return Ok(());
            }
        }
        if !self.prepared {
            return Err("route preparation continuation absent".into());
        }
        if self
            .departure
            .as_ref()
            .is_some_and(|d| d.outcome().is_none())
        {
            return Err("destination preparation before departure completion".into());
        }
        match (&self.point, &self.standing, &self.posture) {
            (Some(point), None, None) => {
                if *point
                    != PointCandidate::prepare(
                        self.target_position,
                        [slot.offset[0], slot.offset[1]],
                        self.target_facing,
                        slot.flags,
                    )?
                    || self.clearance.as_ref().is_none_or(|c| {
                        c.input().position != point.position
                            || c.input().direction != point.direction
                    })
                {
                    return Err("native point preparation mismatch".into());
                }
                if self.candidates.is_some() {
                    let outcome = self
                        .clearance
                        .as_ref()
                        .and_then(|c| c.outcome())
                        .ok_or("point clearance absent")?;
                    self.validate_finished_candidates(&point.finish(outcome.classification, 0)?)?;
                }
            }
            (None, Some(scan), None) => {
                scan.validate()?;
                let mut target = s.body(self.target)?.clone();
                target.position =
                    crate::Point::new(self.target_position[1], self.target_position[0]);
                target.facing = 1 << self.target_facing;
                let mut actor = s.body(self.caller)?.clone();
                actor.position =
                    crate::Point::new(scan.request().preferred[1], scan.request().preferred[0]);
                actor.facing = 1 << scan.request().actor_facing;
                let mut expected = crate::routing::standing_slot(&slot, version, &target, &actor)?;
                expected.walls = scan.request().walls;
                if expected != *scan.request()
                    || slot.sitting != 0
                    || self.candidates.is_some() != scan.outcome().is_some()
                    || self.clearance.as_ref().is_some_and(|c| {
                        scan.pending().is_none_or(|q| {
                            c.input().position != q.position
                                || c.input().direction != Some(q.direction)
                        })
                    })
                {
                    return Err("native standing preparation mismatch".into());
                }
                if let Some(set) = scan.outcome() {
                    self.validate_finished_candidates(set)?;
                }
            }
            (None, None, Some(scan)) => {
                scan.validate()?;
                let mut target = s.body(self.target)?.clone();
                target.position =
                    crate::Point::new(self.target_position[1], self.target_position[0]);
                target.facing = 1 << self.target_facing;
                let mut actor = s.body(self.caller)?.clone();
                actor.position =
                    crate::Point::new(scan.request().preferred[1], scan.request().preferred[0]);
                actor.facing = 1 << scan.request().actor_facing;
                let mut expected = crate::routing::standing_slot(&slot, version, &target, &actor)?;
                expected.walls = scan.request().walls;
                if expected != *scan.request()
                    || slot.sitting != scan.sitting()
                    || self.candidates.is_some() != scan.outcome().is_some()
                    || self.clearance.as_ref().is_some_and(|c| {
                        scan.pending().is_none_or(|q| {
                            c.input().position != q.position
                                || c.input().direction != Some(q.direction)
                        })
                    })
                {
                    return Err("native seated preparation mismatch".into());
                }
                if let Some(set) = scan.outcome() {
                    self.validate_finished_candidates(set)?;
                }
            }
            _ => return Err("ambiguous native destination preparation".into()),
        }
        if let Some(c) = &self.clearance {
            let input = c.input();
            if input.target != self.input.target
                || input.flags != self.input.flags
                || input.room != self.input.room
                || input.level != self.input.level
                || input.size != self.input.size
                || input.ignored != self.input.ignored
            {
                return Err("native clearance context mismatch".into());
            }
        }
        if let Some(e) = &self.floor {
            e.validate(&s.vm, p)?;
            let c = self
                .clearance
                .as_ref()
                .ok_or("native floor without clearance")?;
            if e.binding() != (self.caller, c.input().position, c.input().level)
                || c.pending()
                    != Some(Request::Placement {
                        position: c.input().position,
                        level: c.input().level,
                    })
            {
                return Err("unbound native route floor query".into());
            }
        }
        if let Some(goal) = &self.goal {
            let path = self
                .path
                .as_ref()
                .and_then(|p| p.points.as_ref())
                .ok_or("native route goal without path")?;
            if self.candidates.as_ref().is_none_or(|set| {
                !set.candidates.iter().any(|c| {
                    self.destination(c, path)
                        .is_ok_and(|d| d.as_ref() == Some(goal))
                })
            }) || self
                .path
                .as_ref()
                .and_then(|p| p.points.as_ref())
                .and_then(|p| p.last())
                != Some(&goal.position)
            {
                return Err("native route goal lacks prepared candidate".into());
            }
        }
        if let Some(candidate) = &self.selected_seat {
            if candidate.seat == 0
                || self.goal.is_some()
                || self
                    .candidates
                    .as_ref()
                    .is_none_or(|s| !s.candidates.contains(candidate))
                || self
                    .path
                    .as_ref()
                    .and_then(|p| p.points.as_ref())
                    .and_then(|p| p.last())
                    != Some(&crate::Point::new(
                        candidate.position[1],
                        candidate.position[0],
                    ))
            {
                return Err("native selected seat lacks prepared candidate/path".into());
            }
        }
        if let Some(scan) = &self.obstacles {
            if scan.person_filter()
                != self
                    .potential
                    .as_ref()
                    .map_or((None, false), |q| (q.ignored_person, q.ignore_people))
            {
                return Err("native potential obstacle filter mismatch".into());
            }
            if self
                .candidates
                .as_ref()
                .is_none_or(|c| c.candidates.is_empty())
                || self.floor.is_some()
            {
                return Err("native obstacles started before destination clearance".into());
            }
            scan.validate(s, p, self.caller)?;
        }
        if self.path.is_some()
            && (!self.policy.builds_path(&s.vm.world) || self.obstacles.is_none())
        {
            return Err("native path lacks its strict map owner or obstacle continuation".into());
        }
        if self.path.as_ref().is_some_and(|p| {
            p.failure_result.is_some_and(|r| {
                r != 3
                    || !self.failed
                    || p.points.is_some()
                    || self.goal.is_some()
                    || self.selected_seat.is_some()
            })
        }) {
            return Err("invalid native search failure".into());
        }
        Ok(())
    }
}

impl Simulation {
    pub(crate) fn validate_native_routing(&self, p: &Program) -> Result<(), String> {
        if let Some(e) = self.native_lot.as_ref().and_then(|n| n.shoo.as_ref()) {
            e.validate(self, p)?;
        }
        if let Some(e) = self.native_lot.as_ref().and_then(|n| n.route.as_ref()) {
            e.validate(self, p)?;
        }
        if let Some(e) = self.native_lot.as_ref().and_then(|n| n.movement.as_ref()) {
            e.validate(self, p)?;
        }
        Ok(())
    }
    /// Called before any ordinary update when a query owns callback execution.
    pub(crate) fn drive_native_route_prefix(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if self
            .native_lot
            .as_ref()
            .and_then(|native| native.route.as_ref())
            .is_some_and(|execution| {
                execution.potential.is_some() || execution.potential_request.is_some()
            })
        {
            return Err("temporary potential cannot use ordinary route owner".into());
        }
        let Some(mut e) = self.native_lot.as_mut().and_then(|n| n.route.take()) else {
            return Ok(true);
        };
        let mut departure_dispatched = false;
        let result = (|| {
            if !e.prepared {
                if !self.drive_native_departure(
                    p,
                    e.departure.as_mut().unwrap(),
                    remaining,
                    step,
                )? {
                    return Ok(false);
                }
                let departure = e.departure.as_ref().unwrap().clone();
                self.route_control
                    .get_mut(&e.request)
                    .unwrap()
                    .original
                    .as_mut()
                    .unwrap()
                    .blocked_object = departure.blocked_object;
                let r = crate::contention::pending_route(&self.vm.threads[&e.thread], e.request)
                    .ok_or("departure route absent")?
                    .clone();
                // Destination candidates capture facing/target after departure callbacks.
                e = Execution::prepare(self, e.thread, &r, Some(departure), None)?;
                e.install_preparation(self)?;
            }
            if !self.drive_point_clearance(p, &mut e, remaining, step)? {
                return Ok(false);
            }
            if !e.posture_checked {
                let flags = e
                    .departure
                    .as_ref()
                    .and_then(|d| d.outcome())
                    .map_or(0, |d| d.flags);
                if self.service_original_stand(
                    p,
                    e.thread,
                    e.request,
                    &e.candidates.as_ref().unwrap().candidates,
                    flags,
                    &mut step.events,
                )? {
                    departure_dispatched = true;
                    return Ok(true);
                }
                e.posture_checked = true;
            }
            self.drive_native_path_prefix(p, &mut e, remaining, step)
        })();
        if departure_dispatched {
            return result;
        }
        let complete = result.as_ref().is_ok_and(|done| *done);
        if complete && e.selected_seat.is_some() {
            match self.service_original_seat(p, &e, step) {
                Ok(super::seat::Dispatch::Failed) => {
                    let request =
                        crate::contention::pending_route(&self.vm.threads[&e.thread], e.request)
                            .ok_or("selected seat route absent")?
                            .clone();
                    return self.begin_route_retry(p, e.thread, &request, 5, &mut step.events);
                }
                Ok(_) => return Ok(true),
                Err(error) => {
                    self.native_lot.as_mut().unwrap().route = Some(e);
                    return Err(error);
                }
            }
        }
        if complete && e.goal.is_some() {
            if let Err(error) = self.install_original_path(p, &e, step) {
                self.native_lot.as_mut().unwrap().route = Some(e);
                return Err(error);
            }
            return Ok(true);
        }
        if complete
            && (e.failed
                || e.candidates
                    .as_ref()
                    .is_some_and(|s| s.candidates.is_empty()))
        {
            let request = crate::contention::pending_route(&self.vm.threads[&e.thread], e.request)
                .ok_or("native route retry request absent")?
                .clone();
            self.route_control
                .get_mut(&e.request)
                .unwrap()
                .original
                .as_mut()
                .unwrap()
                .blocked_object = e.blocked_object;
            self.event(
                &mut step.events,
                e.thread,
                format!(
                    "original-lot route enters native retry; result {}, blocker #{}",
                    e.path_result(),
                    e.blocked_object
                ),
            );
            return self.begin_route_retry(
                p,
                e.thread,
                &request,
                i16::from(e.path_result()),
                &mut step.events,
            );
        }
        let boundary = complete.then(|| format!("original-lot path construction required: thread {}, request {}, candidates {:?}, dynamic geometry {:?}; strict routing requires the retained native map owner",e.thread,e.request,e.candidates,e.obstacles.as_ref().and_then(|o|o.geometry.as_ref())));
        self.native_lot.as_mut().unwrap().route = Some(e);
        match boundary {
            Some(error) => Err(error),
            None => result,
        }
    }

    pub(crate) fn drive_native_path_prefix(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if !self.drive_point_clearance(p, e, remaining, step)? {
            return Ok(false);
        }
        if e.candidates.as_ref().unwrap().candidates.is_empty() {
            return Ok(true);
        }
        if e.obstacles.is_none() {
            e.obstacles = Some(super::obstacle::Prefix::begin(self, e.caller)?);
            if let Some(q) = &e.potential {
                e.obstacles
                    .as_mut()
                    .unwrap()
                    .set_person_filter(q.ignored_person, q.ignore_people);
            }
        }
        if e.policy.builds_path(&self.vm.world) {
            self.drive_original_path(p, e, remaining, step)
        } else {
            self.drive_native_obstacle_prefix(p, e.obstacles.as_mut().unwrap(), remaining, step)
        }
    }
    pub(crate) fn drive_potential_preparation(
        &mut self,
        program: &Program,
        execution: &mut Execution,
        request: &RouteRequest,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if execution.potential.is_none() {
            return Err("potential preparation without temporary context".into());
        }
        execution.validate_request(self, program, request)?;
        if !execution.prepared {
            if !self.drive_native_departure(
                program,
                execution
                    .departure
                    .as_mut()
                    .ok_or("potential departure absent")?,
                remaining,
                step,
            )? {
                return Ok(false);
            }
            let mut prepared = Execution::prepare(
                self,
                execution.thread,
                request,
                execution.departure.clone(),
                execution.potential.clone(),
            )?;
            prepared.candidate_limit = prepared.candidate_limit.min(execution.candidate_limit);
            prepared.policy = execution.policy;
            prepared.install_preparation(self)?;
            *execution = prepared;
        }
        if !self.drive_point_clearance(program, execution, remaining, step)? {
            return Ok(false);
        }
        execution.mark_potential_prepared();
        Ok(true)
    }
    pub(crate) fn drive_point_clearance(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            if e.candidates.is_some() {
                return Ok(true);
            }
            if e.clearance.is_none() {
                let (next, facing, set) = if let Some(scan) = e.standing.as_mut() {
                    let next = scan.next(&mut self.vm.rng_state)?;
                    (next, scan.actor_facing(), scan.outcome().cloned())
                } else if let Some(scan) = e.posture.as_mut() {
                    scan.observe_blocker(e.blocked_object);
                    let mut rng = self.vm.rng_state;
                    let next = scan.next(&mut rng, |point, facing| {
                        self.vm
                            .world
                            .objects
                            .get_mut(&e.caller)
                            .unwrap()
                            .data
                            .insert(1, i16::from(facing));
                        self.sync_script_directions()?;
                        self.original_seat_at(p, e.caller, point, e.input.level)
                    });
                    self.vm.rng_state = rng;
                    e.blocked_object = scan.blocked_object();
                    (next?, scan.actor_facing(), scan.outcome().cloned())
                } else {
                    return Err("native route clearance absent".into());
                };
                self.vm
                    .world
                    .objects
                    .get_mut(&e.caller)
                    .unwrap()
                    .data
                    .insert(1, i16::from(facing));
                self.sync_script_directions()?;
                let Some(check) = next else {
                    e.finish_candidates(self, set.ok_or("candidate scan unfinished")?)?;
                    return Ok(true);
                };
                let mut input = e.input.clone();
                input.position = check.position;
                input.direction = Some(check.direction);
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
            let outcome = e
                .clearance
                .as_ref()
                .unwrap()
                .outcome()
                .ok_or("native route clearance result absent")?;
            self.native_lot.as_mut().unwrap().diagnostics = outcome.diagnostics;
            e.blocked_object = outcome.blocked_object;
            if let Some(scan) = e.standing.as_mut() {
                scan.resume(
                    scan.pending().ok_or("standing request absent")?,
                    outcome.classification,
                )?;
                let facing = self.vm.world.objects[&e.caller].data[&1];
                scan.observe_actor_facing(
                    u8::try_from(facing).map_err(|_| "invalid callback facing")?,
                )?;
                e.clearance = None;
                continue;
            }
            if let Some(scan) = e.posture.as_mut() {
                scan.observe_blocker(e.blocked_object);
                scan.resume(
                    scan.pending().ok_or("posture request absent")?,
                    outcome.classification,
                )?;
                let facing = self.vm.world.objects[&e.caller].data[&1];
                scan.observe_actor_facing(
                    u8::try_from(facing).map_err(|_| "invalid callback facing")?,
                )?;
                e.clearance = None;
                continue;
            }
            let set = e
                .point
                .as_ref()
                .unwrap()
                .finish(outcome.classification, 0)?;
            e.finish_candidates(self, set)?;
            return Ok(true);
        }
    }
}

/// Relative wrappers build the same native 15-word descriptor; their distances
/// are already in subtile units, equivalent to an in-memory version-10 SLOT.
pub(crate) fn descriptor(
    destination: &sims_vm::routing::Destination,
) -> Result<(std::borrow::Cow<'_, sims_formats::execution::Slot>, u32), String> {
    use std::borrow::Cow;
    match destination {
        sims_vm::routing::Destination::Slot { slot, version, .. } => {
            Ok((Cow::Borrowed(slot), *version))
        }
        sims_vm::routing::Destination::Relative(relative) => Ok((
            Cow::Owned(sims_vm::routing::Descriptor(relative.words()).slot()),
            10,
        )),
        _ => Err("native route requires SLOT or relative descriptor".into()),
    }
}

#[cfg(test)]
#[path = "live/ignored_owner.rs"]
mod ignored_owner;

#[cfg(test)]
#[path = "live/potential_departure.rs"]
mod potential_departure;
