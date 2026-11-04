//! Cooperative route obstruction decisions recovered from Sims 1.91 0x45ad63.
//! Named trees and shoo placement remain explicit services of the host.
use crate::{Point, routing};
use serde::{Deserialize, Serialize};

/// A synchronous callback can temporarily hide the route at any nesting depth.
pub(crate) fn pending_route(thread: &sims_vm::Thread, id: u64) -> Option<&sims_vm::RouteRequest> {
    std::iter::once(&thread.status)
        .chain(
            thread
                .service
                .iter()
                .flat_map(|s| s.chain())
                .map(|s| &s.resume_status),
        )
        .chain(
            thread
                .callback
                .iter()
                .flat_map(|s| s.chain())
                .map(|s| &s.resume_status),
        )
        .find_map(|status| match status {
            sims_vm::Status::AwaitingRoute(r) if r.id == id => Some(r),
            _ => None,
        })
}

/// SAnimator +0x30, original 0x589ead. Clip playback modes consult the clip;
/// idle/standing modes permit interruption and transition modes forbid it.
pub fn animator_can_interrupt(mode: u32, clip_property: Option<i32>) -> bool {
    match mode {
        0 | 9 | 10 => clip_property.unwrap_or(1) != 0,
        1..=3 => true,
        _ => false,
    }
}

/// Original 0x4817e9 distinguishes ignored objects, nearby people and objects
/// requiring function 5. Horizontal coordinates use sixteenths of a tile;
/// native inter-level distance contributes twenty tiles per level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObstacleGate {
    Ignore,
    Block,
    ObjectFunction,
}
pub fn obstacle_gate(
    contained: bool,
    flags: u16,
    weight: i16,
    person: bool,
    explicitly_ignored: bool,
    delta: Point,
    level_delta: i32,
) -> ObstacleGate {
    let nonblocking = if flags & 4 == 0 {
        flags & 16 != 0
    } else {
        flags & 2 == 0
    };
    if contained || nonblocking && weight == 0 {
        return ObstacleGate::Ignore;
    }
    if !person {
        return ObstacleGate::ObjectFunction;
    }
    let level = i64::from(level_delta) * 20 * 16;
    let distance_squared = i64::from(delta.x).pow(2) + i64::from(delta.y).pow(2) + level.pow(2);
    if explicitly_ignored || distance_squared > 48 * 48 {
        ObstacleGate::Ignore
    } else {
        ObstacleGate::Block
    }
}

/// Direction of travel through the blocker's footprint, sampled by the original
/// shoo helper at max(|dx|,|dy|)+1 increments along each segment. The first point
/// outside after entering determines which perpendicular directions to offer.
pub fn shoo_direction(points: &[Point], position: Point, bounds: crate::Rect) -> Option<u8> {
    if points.len() < 2 {
        return None;
    }
    let mut entered = None;
    let mut exited = None;
    'segments: for segment in points.windows(2) {
        let a = segment[0];
        let b = segment[1];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let steps = dx.abs().max(dy.abs()) + 1;
        let mut fraction = 0.;
        while fraction < 1. {
            let p = Point::new(
                (a.x as f64 + dx as f64 * fraction) as i32,
                (a.y as f64 + dy as f64 * fraction) as i32,
            );
            let inside = bounds.min.x <= p.x
                && p.x < bounds.max.x
                && bounds.min.y <= p.y
                && p.y < bounds.max.y;
            if inside {
                entered.get_or_insert(p);
            } else if entered.is_some() {
                exited = Some(p);
                break 'segments;
            }
            fraction += 1. / steps as f64;
        }
    }
    let (from, to) = match entered {
        Some(a) => (a, exited.unwrap_or(*points.last().unwrap())),
        None => (points[0], position),
    };
    if from == to {
        return None;
    }
    Some(sims_vm::routing::direction([from.y, from.x], [to.y, to.x]) as u8)
}
pub fn shoo_descriptor(direction: u8, side_path: bool) -> sims_vm::routing::Descriptor {
    let flags = 0x2100
        | (1 << ((direction + 2) & 7))
        | (1 << ((direction + 6) & 7))
        | if side_path { 0 } else { 1 << direction };
    sims_vm::routing::Descriptor([
        0,
        0,
        0,
        u32::MAX,
        1,
        0,
        0,
        flags,
        u32::MAX,
        16,
        80,
        if side_path { 16 } else { 80 },
        0.1875f32.to_bits(),
        (-3i32) as u32,
        8,
    ])
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    Retry,
    Waiting,
    Asking,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Callback {
    Wait,
    Ask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Link {
    pub blocker: sims_vm::ObjectId,
    pub object: sims_vm::ObjectId,
    pub action: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    pub thread: sims_vm::ThreadId,
    pub caller: sims_vm::ObjectId,
    pub controller: Controller,
    pub link: Option<Link>,
    pub blocker_success: bool,
    pub callback: Option<Callback>,
    pub deciding: bool,
    /// Result of the synchronous factory/queue operation across budget slices.
    pub shoo_result: Option<bool>,
    pub last_portal: Option<sims_vm::ObjectId>,
    pub original: Option<crate::lot_route::portal::Control>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CreationOrigin {
    Route {
        request: u64,
        direction: u8,
    },
    Snap {
        thread: sims_vm::ThreadId,
        request: sims_vm::SnapRequest,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Creation {
    pub origin: CreationOrigin,
    pub blocker: sims_vm::ObjectId,
    pub object: sims_vm::ObjectId,
}
/// Native standalone placement shoo (0x45bd5e), which offers all four cardinal
/// directions at tile resolution, unlike the route corridor's half-tile search.
pub fn placement_shoo_descriptor() -> sims_vm::routing::Descriptor {
    let mut descriptor = shoo_descriptor(0, true);
    descriptor.0[7] = 0x2155;
    descriptor.0[14] = 16;
    descriptor
}
/// Snap 0x454976 reports placement failure even when it queued a shoo. The
/// script consumes field 52 to decide whether to wait and retry.
pub fn snap_shoo_allowed(caller_person: bool, flags: u8, error: i32, blocker_person: bool) -> bool {
    caller_person && flags & 2 != 0 && error == 11 && blocker_person
}
pub fn snap_failure_result(error: i32, shoo_accepted: bool) -> i16 {
    if shoo_accepted {
        1
    } else if error == 1 {
        2
    } else {
        0
    }
}

impl Callback {
    pub fn name(self) -> &'static str {
        match self {
            Self::Wait => "route waiting animation",
            Self::Ask => "route ask to move animation",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Replan,
    Callback(Callback),
    Fail { feedback: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Controller {
    pub phase: Phase,
    /// Native signed short at context +0x58; progress resets the retry sequence.
    pub stage: i16,
    pub previous: Point,
    /// The original 32-bit simulation clock, with zero used as the unset value.
    pub started: u32,
    pub result: i16,
}
impl Controller {
    pub fn new(position: Point) -> Self {
        Self {
            phase: Phase::Retry,
            stage: 0,
            previous: position,
            started: 0,
            result: 0,
        }
    }
    /// Run until candidate generation, a pushed original callback, or failure.
    /// Closure results represent actual host service outcomes, never timeouts.
    pub fn decide(
        &mut self,
        current: Point,
        clock: u32,
        interrupted: bool,
        mut blocker_active: bool,
        blocker_success: bool,
        mut shoo: impl FnMut() -> bool,
        mut callback: impl FnMut(Callback) -> bool,
    ) -> Decision {
        loop {
            match self.phase {
                Phase::Retry => {
                    if self.stage == 0 {
                        let progressed = routing::made_route_progress(self.previous, current);
                        self.previous = current;
                        if progressed {
                            return self.replan(interrupted);
                        }
                        self.stage = 1;
                    }
                    match self.stage {
                        1 => {
                            self.stage = 2;
                            if !blocker_active && shoo() {
                                blocker_active = true;
                                self.phase = Phase::Asking;
                            } else {
                                return Decision::Fail { feedback: true };
                            }
                        }
                        2 => {
                            self.stage = 3;
                            return self.replan(interrupted);
                        }
                        n => return Decision::Fail { feedback: n == 3 },
                    }
                }
                Phase::Waiting | Phase::Asking => {
                    if interrupted {
                        self.result = 4;
                        return Decision::Fail { feedback: false };
                    }
                    if self.phase == Phase::Waiting {
                        let started = if self.started == 0 {
                            clock
                        } else {
                            self.started
                        };
                        if (clock.wrapping_sub(started) as i32) < 15 && callback(Callback::Wait) {
                            self.started = started;
                            return Decision::Callback(Callback::Wait);
                        }
                        self.started = 0;
                        return Decision::Replan;
                    }
                    if !blocker_active {
                        self.started = 0;
                        if blocker_success {
                            self.stage = 0;
                        }
                        return Decision::Replan;
                    }
                    if self.started == 0 {
                        if callback(Callback::Ask) {
                            self.started = clock;
                            return Decision::Callback(Callback::Ask);
                        }
                    } else {
                        if (clock.wrapping_sub(self.started) as i32) < 100
                            && callback(Callback::Wait)
                        {
                            return Decision::Callback(Callback::Wait);
                        }
                        self.started = 0;
                    }
                    self.phase = Phase::Retry;
                }
            }
        }
    }
    fn replan(&mut self, interrupted: bool) -> Decision {
        if interrupted {
            self.result = 4;
            Decision::Fail { feedback: false }
        } else {
            Decision::Replan
        }
    }
}

impl crate::Simulation {
    /// 0x45ad63 consults 0x45d7c0 as well as the animator's +0x30 gate.
    /// Live native characters consult the actual ordinary mode and foreground
    /// property. The older playback adapter retains its separate approximation.
    pub(crate) fn route_interrupted(
        &self,
        thread: sims_vm::ThreadId,
        caller: sims_vm::ObjectId,
    ) -> Result<bool, String> {
        if let Some(resident) = self.characters.get(&caller) {
            // Native UI cancellation already updates this queue/person state.
            // Presentation flags and the legacy VM interrupt flag cannot bypass
            // 45d7c0's person-71 suppression or its priority/motive rules.
            let actions = self
                .vm
                .world
                .actions
                .as_ref()
                .ok_or("native route action world absent")?;
            let actor = actions
                .actors
                .get(&caller)
                .ok_or("native route action owner absent")?;
            let requested = actor
                .queue
                .should_interrupt(&self.vm.world.objects[&caller], actions.live_family)?;
            if !requested {
                return Ok(false);
            }
            let animator = &resident.character.skin.animator;
            let clip = animator
                .slot(crate::pose::animator::Slot::Foreground)
                .map(|id| animator.layer(id).ok_or("native route foreground absent"))
                .transpose()?;
            return Ok(animator_can_interrupt(
                resident.controls.ordinary.mode as u32,
                clip.map(|l| l.interruptible),
            ));
        }
        if self.vm.threads[&thread].interrupted || self.interaction_cancelled(thread) {
            return Ok(true);
        }
        if self.architecture.is_none() {
            return Ok(false);
        }
        let clip = self.animations.get(&caller).filter(|a| !a.ended);
        if !animator_can_interrupt(
            if clip.is_some() { 0 } else { 3 },
            clip.map(|a| i32::from(a.interruptible)),
        ) {
            return Ok(false);
        }
        let Some(actions) = &self.vm.world.actions else {
            return Ok(false);
        };
        let Some(actor) = actions.actors.get(&caller) else {
            return Ok(false);
        };
        actor
            .queue
            .should_interrupt(&self.vm.world.objects[&caller], actions.live_family)
    }
    pub(crate) fn validate_contention(&self, p: &sims_vm::Program) -> Result<(), String> {
        use sims_vm::Status;
        let mut blockers = std::collections::BTreeSet::new();
        for (&id, state) in &self.route_control {
            let thread = self
                .vm
                .threads
                .get(&state.thread)
                .ok_or("orphaned route controller")?;
            let call = thread
                .service
                .iter()
                .flat_map(|s| s.chain())
                .find(|s| matches!(&s.resume_status, Status::AwaitingRoute(r) if r.id == id));
            let request = pending_route(thread, id)
                .or_else(|| self.reset_route_request(state.thread, id))
                .ok_or("route controller without continuation")?;
            if state.original.is_some() != self.native_lot.is_some() {
                return Err("route controller host mismatch".into());
            }
            if let Some(original) = &state.original {
                if original
                    .seated
                    .is_some_and(|seat| call.is_none_or(|c| c.callee != seat))
                {
                    return Err("original Sit service continuation absent".into());
                }
                if original
                    .standing
                    .is_some_and(|seat| call.is_none_or(|c| c.callee != seat))
                    || original.standing.is_some() && original.seated.is_some()
                {
                    return Err("original Stand service continuation absent".into());
                }
                if let Some(call) = call
                    && !self.route_feedback.contains_key(&id)
                    && state.callback.is_none()
                {
                    let definition = self.vm.world.objects[&call.callee]
                        .definition
                        .as_ref()
                        .ok_or("original portal definition absent")?;
                    let seated = original.seated == Some(call.callee);
                    let standing = original.standing == Some(call.callee);
                    if (!seated
                        && !standing
                        && (!original.crossing || state.last_portal != Some(call.callee)))
                        || call.caller != state.caller
                        || p.modules[&definition.source]
                            .object_functions
                            .get(&definition.object)
                            .and_then(|f| {
                                f.get(if seated {
                                    26
                                } else if standing {
                                    27
                                } else {
                                    15
                                })
                            })
                            .is_none_or(|f| f.action == 0 || f.action != call.bhav)
                    {
                        return Err("original portal service binding mismatch".into());
                    }
                }
            }
            if self.architecture.is_none()
                || request.caller != state.caller
                || !self.bodies.contains_key(&state.caller)
                || !(0..=3).contains(&state.controller.stage)
                || !(0..=15).contains(&state.controller.result)
                || !self
                    .room
                    .contains(self.collision_bounds(state.caller, state.controller.previous))
                || state.shoo_result == Some(true) && state.link.is_none()
            {
                return Err("invalid native route controller".into());
            }
            if let Some(cb) = state.callback {
                let call = call.ok_or("route callback without service")?;
                let named = self.vm.resolve_named_tree(p, state.caller, cb.name())?;
                if !state.deciding
                    || call.caller != state.caller
                    || call.callee != state.caller
                    || named != Some((call.owner.clone(), call.bhav))
                {
                    return Err("route callback binding mismatch".into());
                }
            }
            if let Some(link) = &state.link {
                let actor = self
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .and_then(|a| a.actors.get(&link.blocker))
                    .ok_or("shoo linked to missing person")?;
                if link.blocker == state.caller
                    || link.action == 0
                    || !blockers.insert(link.blocker)
                {
                    return Err("invalid shoo action link".into());
                }
                // A cancelled action may disappear before its waiter times out.
                // If it is still queued or running, its identity must match.
                if let Some(action) = actor
                    .active
                    .iter()
                    .chain(&actor.queue.pending)
                    .find(|a| a.id == link.action)
                    && (action.target != link.object
                        || action.menu_id != 3
                        || self
                            .vm
                            .world
                            .objects
                            .get(&link.object)
                            .and_then(|o| o.definition.as_ref())
                            .is_none_or(|d| d.guid != 0x7c4))
                {
                    return Err("shoo action identity mismatch".into());
                }
            }
        }
        if let Some(c) = &self.shoo_creation {
            if !self.unplaced.contains(&c.object)
                || !self.bodies.contains_key(&c.blocker)
                || self
                    .vm
                    .world
                    .objects
                    .get(&c.object)
                    .and_then(|o| o.definition.as_ref())
                    .is_none_or(|d| d.guid != 0x7c4)
                || match &c.origin {
                    CreationOrigin::Route { request, direction } => {
                        *direction > 7
                            || self.route_control.get(request).is_none_or(|s| {
                                !s.deciding || s.shoo_result.is_some() || s.callback.is_some()
                            })
                    }
                    CreationOrigin::Snap { thread, request } => {
                        self.vm.threads.get(thread).is_none_or(
                            |t| !matches!(&t.status,sims_vm::Status::AwaitingSnap(r) if r==request),
                        )
                    }
                }
            {
                return Err("invalid pending shoo factory".into());
            }
        }
        Ok(())
    }
    /// Native callbacks and route cleanup identify a shoo by both person and
    /// action serial. A different later action must never release its waiter.
    pub(crate) fn complete_route(
        &mut self,
        p: &sims_vm::Program,
        thread: sims_vm::ThreadId,
        request: &sims_vm::RouteRequest,
        success: bool,
    ) -> Result<(), String> {
        let serial = self
            .vm
            .world
            .actions
            .as_ref()
            .and_then(|a| a.actors.get(&request.caller))
            .and_then(|a| a.active.as_ref())
            .map(|a| a.id);
        for state in self.route_control.values_mut() {
            if state
                .link
                .as_ref()
                .is_some_and(|l| l.blocker == request.caller && Some(l.action) == serial)
            {
                state.link = None;
                state.blocker_success = success;
            }
        }
        if let Some(state) = self.route_control.remove(&request.id)
            && let Some(link) = state.link
        {
            self.vm.cancel_action(p, link.blocker, link.action)?;
        }
        self.vm.finish_route(p, thread, request.id, success)
    }
    pub(crate) fn route_contention(
        &mut self,
        p: &sims_vm::Program,
        thread: sims_vm::ThreadId,
        request: &sims_vm::RouteRequest,
        events: &mut Vec<crate::Event>,
    ) -> Result<bool, String> {
        let mut state = self
            .route_control
            .remove(&request.id)
            .ok_or("route controller missing")?;
        let before = state.controller.clone();
        let mut want_shoo = false;
        let mut callbacks = std::collections::BTreeMap::new();
        for cb in [Callback::Wait, Callback::Ask] {
            if let Some(named) = self.vm.resolve_named_tree(p, request.caller, cb.name())? {
                callbacks.insert(cb.name(), named);
            }
        }
        let interrupted = self.route_interrupted(thread, request.caller)?;
        let decision = state.controller.decide(
            self.bodies[&request.caller].position,
            self.vm.tick as u32,
            interrupted,
            state.link.is_some() && state.shoo_result.is_none(),
            state.blocker_success,
            || {
                want_shoo = true;
                state.shoo_result.unwrap_or(true)
            },
            |cb| callbacks.contains_key(cb.name()),
        );
        if want_shoo && state.shoo_result.is_none() {
            state.controller = before;
            self.route_control.insert(request.id, state);
            if !self.start_shoo(p, request, events)? {
                self.route_control.get_mut(&request.id).unwrap().shoo_result = Some(false);
            }
            return Ok(true);
        }
        state.shoo_result = None;
        match decision {
            Decision::Replan => {
                state.deciding = false;
                self.route_control.insert(request.id, state);
                self.event(events, thread, "native route controller replans".into());
                Ok(true)
            }
            Decision::Callback(cb) => {
                let (owner, bhav) = callbacks.remove(cb.name()).unwrap();
                self.vm
                    .push_route_service(p, thread, owner, bhav, request.caller, None)?;
                if cb == Callback::Ask {
                    self.vm
                        .threads
                        .get_mut(&thread)
                        .unwrap()
                        .stack
                        .last_mut()
                        .unwrap()
                        .stack_object = state
                        .link
                        .as_ref()
                        .ok_or("ask callback without blocker")?
                        .blocker;
                }
                state.callback = Some(cb);
                self.route_control.insert(request.id, state);
                self.event(events, thread, format!("native {}", cb.name()));
                Ok(true)
            }
            Decision::Fail { feedback } => {
                let result = state.controller.result;
                let blocker = state.original.as_ref().map_or_else(
                    || state.link.as_ref().map_or(0, |l| l.blocker),
                    |c| c.blocked_object,
                );
                self.route_control.insert(request.id, state);
                if feedback {
                    self.fail_route_blocked(
                        p,
                        thread,
                        request,
                        result,
                        blocker,
                        "native retry sequence exhausted",
                        events,
                    )
                } else {
                    self.person(request.caller, 62, if result == 0 { 1 } else { result })?;
                    self.release(thread);
                    self.vm.threads.get_mut(&thread).unwrap().interrupted = false;
                    if self.native_lot.is_some() {
                        self.begin_native_route_completion(
                            thread,
                            request,
                            false,
                            i32::from(result),
                        )?;
                    } else {
                        self.complete_route(p, thread, request, false)?;
                    }
                    Ok(true)
                }
            }
        }
    }
    fn potential_route(
        &mut self,
        p: &sims_vm::Program,
        request: &sims_vm::RouteRequest,
        ignore: Option<i16>,
        ignore_people: bool,
    ) -> Result<Option<Vec<Point>>, String> {
        let caller = self.body(request.caller)?.clone();
        let target = self.body(request.target)?.clone();
        let cross_room = match &request.destination {
            sims_vm::routing::Destination::Slot { slot, .. } => slot.flags & 0x800 != 0,
            sims_vm::routing::Destination::Relative(r) => r.flags & 0x800 != 0,
            _ => false,
        };
        let clear = |s: &Self, pos: Point| {
            s.clear_with_person_policy(request.caller, pos, ignore, ignore_people)
                && (cross_room
                    || s.architecture.as_ref().is_none_or(|a| {
                        a.room_at(caller.level, pos) == a.room_at(caller.level, caller.position)
                    }))
        };
        let goals = if let sims_vm::routing::Destination::Slot { slot, version, .. } =
            &request.destination
            && slot.flags & 255 != 0
        {
            let mut context = routing::standing_slot(slot, *version, &target, &caller)?;
            if let Some(a) = &self.architecture {
                context.walls = a.wall_mask(
                    target.level,
                    target.position,
                    target.facing.trailing_zeros() as u8,
                );
            }
            let mut rng = self.vm.rng_state;
            let (mut candidates, _) = routing::posture_candidates(
                &context,
                slot.sitting,
                &mut rng,
                |point| self.seat_at(p, request.caller, point),
                |point, _| {
                    if clear(self, Point::new(point.y, point.x)) {
                        6
                    } else {
                        0
                    }
                },
            )?;
            routing::limit_candidates(
                &mut candidates,
                self.route_candidate_limit(request.caller),
                &mut rng,
            );
            self.vm.rng_state = rng;
            candidates
                .candidates
                .iter()
                .map(|c| {
                    (
                        routing::candidate_destination(&context, c),
                        ((candidates.maximum - c.score).max(0) * 16000) as u32,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            routing::destinations(&request.destination, &target, &caller)?
                .into_iter()
                .map(|g| (g, 0))
                .collect()
        };
        if self.architecture.is_some() {
            return Ok(self
                .native_path(p, request.caller, &goals, ignore, ignore_people, cross_room)?
                .map(|(_, path)| path));
        }
        Ok(
            crate::geometry::pathfind_scored(&self.room, caller.position, &goals, |point| {
                clear(self, point)
            })
            .map(|(_, path)| path),
        )
    }
    fn start_shoo(
        &mut self,
        p: &sims_vm::Program,
        r: &sims_vm::RouteRequest,
        events: &mut Vec<crate::Event>,
    ) -> Result<bool, String> {
        if self.native_lot.is_some() {
            return self.start_original_shoo(r);
        }
        if self.vm.world.objects[&r.caller]
            .person
            .as_ref()
            .is_none_or(|p| p.get(&0) != Some(&0))
        {
            return Ok(false);
        }
        let people = self
            .vm
            .world
            .person
            .as_ref()
            .ok_or("shoo needs person registry")?
            .person_order
            .clone();
        let people = people
            .into_iter()
            .filter(|&id| {
                id != r.caller
                    && self.bodies.contains_key(&id)
                    && {
                        let o = &self.vm.world.objects[&id];
                        let a = self.bodies[&r.caller].position;
                        let b = self.bodies[&id].position;
                        obstacle_gate(
                            o.container.is_some(),
                            o.data[&8] as u16,
                            o.data[&10],
                            true,
                            false,
                            Point::new(b.x - a.x, b.y - a.y),
                            self.bodies[&id].level - self.bodies[&r.caller].level,
                        ) == ObstacleGate::Block
                    }
                    && !self
                        .route_control
                        .values()
                        .any(|s| s.link.as_ref().is_some_and(|l| l.blocker == id))
            })
            .collect::<Vec<_>>();
        let mut selected = None;
        for &blocker in &people {
            if self.has_move_action(blocker) {
                continue;
            }
            if let Some(path) = self.potential_route(p, r, Some(blocker), false)?
                && let Some(direction) = shoo_direction(
                    &path,
                    self.bodies[&blocker].position,
                    self.collision_bounds(blocker, self.bodies[&blocker].position),
                )
            {
                selected = Some((blocker, direction));
                break;
            }
        }
        if selected.is_none()
            && !people.is_empty()
            && let Some(path) = self.potential_route(p, r, None, true)?
        {
            'path: for point in &path {
                for &blocker in &people {
                    let bounds = self.collision_bounds(blocker, self.bodies[&blocker].position);
                    if bounds.min.x <= point.x
                        && point.x < bounds.max.x
                        && bounds.min.y <= point.y
                        && point.y < bounds.max.y
                    {
                        if !self.has_move_action(blocker) {
                            selected =
                                shoo_direction(&path, self.bodies[&blocker].position, bounds)
                                    .map(|d| (blocker, d));
                        }
                        break 'path;
                    }
                }
            }
        }
        let Some((blocker, direction)) = selected else {
            return Ok(false);
        };
        let started = self.begin_shoo_factory(
            p,
            blocker,
            CreationOrigin::Route {
                request: r.id,
                direction,
            },
        )?;
        if started {
            self.event(
                events,
                self.route_control[&r.id].thread,
                format!("native route shoo begins for blocker #{blocker}"),
            );
        }
        Ok(started)
    }
    pub(crate) fn begin_shoo_factory(
        &mut self,
        p: &sims_vm::Program,
        blocker: i16,
        origin: CreationOrigin,
    ) -> Result<bool, String> {
        if self.has_move_action(blocker) {
            return Ok(false);
        }
        if self.shoo_creation.is_some() {
            return Err("overlapping synchronous shoo factories".into());
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
            .ok_or("native shoo GoHere GUID 000007C4 is not loaded")?;
        let Some(object) = (1..=i16::MAX).find(|id| !self.vm.world.objects.contains_key(id)) else {
            return Ok(false);
        };
        self.initialize_object_factory(p, object, &module.source, number, false)?;
        self.shoo_creation = Some(Creation {
            origin,
            blocker,
            object,
        });
        Ok(true)
    }
    pub(crate) fn has_move_action(&self, blocker: i16) -> bool {
        if self
            .route_control
            .values()
            .any(|s| s.caller == blocker && s.link.is_some())
        {
            return true;
        }
        self.vm
            .world
            .actions
            .as_ref()
            .and_then(|a| a.actors.get(&blocker))
            .is_none_or(|a| {
                a.active.iter().chain(&a.queue.pending).any(|a| {
                    a.menu_id == 3
                        && self
                            .vm
                            .world
                            .objects
                            .get(&a.target)
                            .and_then(|o| o.definition.as_ref())
                            .is_some_and(|d| d.guid == 0x7c4)
                })
            })
    }
    pub(crate) fn drive_shoo_creation(
        &mut self,
        p: &sims_vm::Program,
        remaining: &mut usize,
        step: &mut crate::Step,
    ) -> Result<bool, String> {
        let c = self.shoo_creation.clone().ok_or("missing shoo factory")?;
        if !self.drive_object_factory(p, c.object, remaining, step)? {
            return Ok(false);
        }
        let position = self.bodies[&c.blocker].position;
        let room = self.vm.world.objects[&c.blocker].data[&29];
        self.bodies.insert(
            c.object,
            crate::Body {
                level: self.bodies[&c.blocker].level,
                position,
                facing: 1,
                radius: 0,
                elevation: 0,
            },
        );
        self.unplaced.remove(&c.object);
        self.vm
            .world
            .objects
            .get_mut(&c.object)
            .unwrap()
            .data
            .insert(29, room);
        let (thread, caller) = match &c.origin {
            CreationOrigin::Route { request, .. } => {
                let state = &self.route_control[request];
                (state.thread, state.caller)
            }
            CreationOrigin::Snap { thread, request } => (*thread, request.route.caller),
        };
        let descriptor = match &c.origin {
            CreationOrigin::Route { direction, .. } => {
                let mut descriptor = shoo_descriptor(*direction, true);
                let def = self.vm.world.objects[&c.object]
                    .definition
                    .as_ref()
                    .unwrap();
                let request = sims_vm::RouteRequest {
                    id: 0,
                    origin: self.vm.threads[&thread]
                        .stack
                        .last()
                        .unwrap()
                        .location
                        .clone(),
                    caller: c.blocker,
                    target: c.object,
                    destination: sims_vm::routing::Destination::Slot {
                        source: def.source.clone(),
                        chunk: def.slots,
                        index: 0,
                        version: 10,
                        slot: descriptor.slot(),
                    },
                    no_failure_trees: true,
                };
                if self.potential_route(p, &request, None, false)?.is_none() {
                    descriptor = shoo_descriptor(*direction, false);
                }
                descriptor
            }
            CreationOrigin::Snap { .. } => placement_shoo_descriptor(),
        };
        self.vm
            .world
            .objects
            .get_mut(&c.object)
            .unwrap()
            .routing_slots = Some(vec![descriptor]);
        let menu = self
            .vm
            .action_menu(p, c.object)?
            .iter()
            .find(|m| m.id == 3)
            .ok_or("GoHere Move menu missing")?;
        let priority = match &c.origin {
            CreationOrigin::Route { .. } => i32::from(
                self.vm.world.objects[&caller]
                    .person
                    .as_ref()
                    .ok_or("shoo asker is not person")?[&33],
            ),
            // Native Snap passes the full active action priority (+0xa64), not
            // its possibly truncated person-data mirror (+0x5ce).
            CreationOrigin::Snap { .. } => self
                .vm
                .world
                .actions
                .as_ref()
                .and_then(|a| a.actors.get(&caller))
                .and_then(|a| a.active.as_ref())
                .map_or(0, |a| a.priority),
        };
        let action = sims_vm::action::Action {
            target: c.object,
            icon_owner: c.object,
            menu_id: 3,
            priority,
            routine: menu.action,
            flags: u32::from(menu.flags & 0x100 != 0),
            ..Default::default()
        };
        let (serial, accepted) = self.vm.enqueue_action(p, c.blocker, action)?;
        match &c.origin {
            CreationOrigin::Route { request, .. } => {
                let state = self.route_control.get_mut(request).unwrap();
                state.shoo_result = Some(accepted);
                if accepted {
                    state.blocker_success = false;
                    state.link = Some(Link {
                        blocker: c.blocker,
                        object: c.object,
                        action: serial,
                    });
                }
            }
            CreationOrigin::Snap { thread, request } => {
                self.vm
                    .world
                    .objects
                    .get_mut(&caller)
                    .unwrap()
                    .data
                    .insert(52, snap_failure_result(11, accepted));
                self.snap_failed(
                    p,
                    *thread,
                    request,
                    c.blocker,
                    "person blocks placement",
                    &mut step.events,
                )?;
            }
        }
        if !accepted {
            self.detach_tree(c.object)?;
            self.discard_unplaced(c.object)?;
        }
        self.shoo_creation = None;
        self.event(
            &mut step.events,
            thread,
            format!(
                "native shoo Move #{serial} for #{} accepted={accepted}",
                c.blocker
            ),
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_animator_interruption_gates() {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/route-interrupt-gate.json"))
                .unwrap();
        for c in report["cases"].as_array().unwrap() {
            assert_eq!(
                super::animator_can_interrupt(
                    c["mode"].as_u64().unwrap() as u32,
                    c["clip"].as_i64().map(|v| v as i32)
                ),
                c["result"] != 0,
                "{c}"
            );
        }
    }
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_obstacle_vectors() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/route-obstacles.json")).unwrap();
        for row in doc["cases"].as_array().unwrap() {
            let c = &row["input"];
            let n = &row["original"];
            assert!(n["error"].is_null(), "{row}");
            let gate = obstacle_gate(
                c["contained"].as_bool().unwrap(),
                c["flags"].as_u64().unwrap() as u16,
                c["weight"].as_i64().unwrap() as i16,
                c["kind"] == 2,
                c["ignored"].as_bool().unwrap(),
                Point::new(
                    c["position"][0].as_i64().unwrap() as i32,
                    c["position"][1].as_i64().unwrap() as i32,
                ),
                c["level"].as_i64().unwrap() as i32 - 1,
            );
            let ignored = gate == ObstacleGate::Ignore
                || gate == ObstacleGate::ObjectFunction && c["callback"].as_bool().unwrap();
            assert_eq!(ignored, n["ignored"].as_bool().unwrap(), "{row}");
            assert_eq!(
                usize::from(gate == ObstacleGate::ObjectFunction),
                n["callbacks"].as_array().unwrap().len(),
                "{row}"
            );
        }
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_placement_shoo_and_snap_failure_vectors() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/snap-shoo.json")).unwrap();
        for row in doc["descriptors"].as_array().unwrap() {
            assert!(row["original"]["error"].is_null());
            assert_eq!(
                serde_json::json!(placement_shoo_descriptor().0),
                row["original"]["descriptor"]
            );
            assert_eq!(row["original"]["returned"], row["input"]["enqueue"]);
            assert_eq!(
                row["original"]["calls"][3],
                serde_json::json!(["action", true, true, 3, row["input"]["priority"]])
            );
        }
        for row in doc["failures"].as_array().unwrap() {
            let c = &row["input"];
            let n = &row["original"];
            let error = c["placement_error"].as_i64().unwrap() as i32;
            let shoo = snap_shoo_allowed(
                true,
                c["flags"].as_u64().unwrap() as u8,
                error,
                c["blocker_kind"] == 2,
            );
            assert_eq!(shoo, !n["shoo"].is_null(), "{row}");
            assert_eq!(
                snap_failure_result(error, shoo && c["shoo_accepted"] == true),
                n["result"].as_i64().unwrap() as i16,
                "{row}"
            );
            assert_eq!(n["blocker"], 2);
            assert_eq!(n["returned"], 0);
        }
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_shoo_descriptor_vectors() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/shoo-descriptor.json")).unwrap();
        for row in doc["cases"].as_array().unwrap() {
            let c = &row["input"];
            let n = &row["original"];
            assert!(n["error"].is_null(), "{row}");
            let point = |v: &serde_json::Value| {
                Point::new(v[1].as_i64().unwrap() as i32, v[0].as_i64().unwrap() as i32)
            };
            let position = point(&c["position"]);
            let radius = c["radius"].as_i64().unwrap_or(3) as i32;
            let path = c["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(point)
                .collect::<Vec<_>>();
            let bounds = crate::Rect {
                min: Point::new(position.x - radius, position.y - radius),
                max: Point::new(position.x + radius, position.y + radius),
            };
            if n["descriptor"][4] == 1 {
                let direction = shoo_direction(&path, position, bounds).unwrap();
                assert_eq!(
                    serde_json::json!(shoo_descriptor(direction, c["path"] == true).0),
                    n["descriptor"],
                    "{row}"
                );
                assert_eq!(n["linked"], c["enqueue"], "{row}");
            } else if path.len() >= 2
                && c.get("already_queued").is_none()
                && c.get("already_asking").is_none()
                && c.get("resource").is_none()
                && c.get("allocate").is_none()
            {
                assert_eq!(shoo_direction(&path, position, bounds), None);
                let calls = n["calls"].as_array().unwrap();
                assert_eq!(
                    calls.len(),
                    3,
                    "zero direction still constructs/places, without cleanup: {row}"
                );
                assert_eq!(calls[0][0], "resource");
                assert_eq!(calls[1][0], "factory");
                assert_eq!(calls[2][0], "place");
                assert_eq!(n["returned"], false);
            }
        }
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_controller_vectors() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/route-contention.json")).unwrap();
        for row in doc["cases"].as_array().unwrap() {
            let c = &row["input"];
            let n = &row["original"];
            let point = |name: &str| {
                Point::new(
                    c[name][0].as_i64().unwrap() as i32,
                    c[name][1].as_i64().unwrap() as i32,
                )
            };
            let mut controller = Controller {
                phase: match c["state"].as_u64().unwrap() {
                    6 => Phase::Retry,
                    13 => Phase::Waiting,
                    14 => Phase::Asking,
                    _ => unreachable!(),
                },
                stage: c["stage"].as_i64().unwrap() as i16,
                previous: point("previous"),
                started: c["started"].as_u64().unwrap() as u32,
                result: 3,
            };
            let mut asked = false;
            let mut names = vec![];
            let mut calls = 0;
            let decision = controller.decide(
                point("current"),
                c["clock"].as_u64().unwrap() as u32,
                c["interrupted"] == true,
                c["blocker_active"] == true,
                c["blocker_success"] == true,
                || {
                    asked = true;
                    c["shoo_accepted"] == true
                },
                |cb| {
                    names.push(cb.name());
                    if c["present"] == true {
                        calls += 1;
                        c["accepted"] == true
                    } else {
                        false
                    }
                },
            );
            assert!(n["error"].is_null(), "{row}");
            assert_eq!(
                serde_json::json!(match decision {
                    Decision::Replan => "replan",
                    Decision::Callback(_) => "callback",
                    Decision::Fail { .. } => "fail",
                }),
                n["decision"],
                "{row}"
            );
            assert_eq!(serde_json::json!(names), n["names"], "{row}");
            assert_eq!(serde_json::json!(asked), n["asked"], "{row}");
            assert_eq!(serde_json::json!(calls), n["callback_count"], "{row}");
            assert_eq!(serde_json::json!(controller.stage), n["stage"], "{row}");
            assert_eq!(serde_json::json!(controller.started), n["started"], "{row}");
            assert_eq!(
                serde_json::json!(controller.result),
                n["route_result"],
                "{row}"
            );
            if matches!(decision, Decision::Callback(_)) {
                assert_eq!(
                    n["state"],
                    if controller.phase == Phase::Waiting {
                        13
                    } else {
                        14
                    },
                    "{row}"
                );
                assert_eq!(
                    n["stack_object"],
                    if decision == Decision::Callback(Callback::Ask) {
                        2
                    } else {
                        0
                    },
                    "{row}"
                );
            }
        }
    }
}
