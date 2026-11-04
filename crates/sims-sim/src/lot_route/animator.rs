//! Imported route ownership for the native animator, separate from tuned
//! locomotion. The ordinary person update advances the movement clock.
use crate::{Body, Simulation, Step};
use crate::{character_update::AnimatorRequest, lot_acceptance, lot_placement, pose::movement};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, RouteRequest, ThreadId, Vm};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Physical {
    Accept(lot_acceptance::live::Execution),
    Move {
        position: [i32; 2],
        level: i32,
        execution: lot_placement::live::Execution,
    },
}
impl Physical {
    pub(crate) fn new(
        s: &Simulation,
        actor: ObjectId,
        request: movement::Request,
    ) -> Result<Self, String> {
        match request {
            movement::Request::AcceptStep { position, level } => Ok(Self::Accept(
                lot_acceptance::live::Execution::new(s, actor, position, level)?,
            )),
            movement::Request::MoveStep { position, level } => Ok(Self::Move {
                position,
                level,
                execution: lot_placement::live::Execution::new(super::movement::commit_operation(
                    actor, position, level,
                )),
            }),
            _ => Err("nonphysical animator movement request".into()),
        }
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        match self {
            Self::Accept(e) => e.owns_callback(vm, p, thread, depth),
            Self::Move { execution, .. } => execution.owns_callback(vm, p, thread, depth),
        }
    }
    pub(crate) fn request(&self) -> movement::Request {
        match self {
            Self::Accept(e) => {
                let (_, position, level) = e.binding();
                movement::Request::AcceptStep { position, level }
            }
            Self::Move {
                position, level, ..
            } => movement::Request::MoveStep {
                position: *position,
                level: *level,
            },
        }
    }
    pub fn validate(&self, s: &Simulation, p: &Program, actor: ObjectId) -> Result<(), String> {
        let resident = &s.characters[&actor];
        if resident
            .ordinary_update
            .as_ref()
            .and_then(|u| u.pending_request(resident))
            != Some(AnimatorRequest::Movement(self.request()))
        {
            return Err("native physical movement lost its animator continuation".into());
        }
        self.validate_execution(s, p, actor)
    }
    pub(crate) fn validate_execution(
        &self,
        s: &Simulation,
        p: &Program,
        actor: ObjectId,
    ) -> Result<(), String> {
        match self {
            Self::Accept(e) => {
                if e.binding().0 != actor {
                    return Err("native physical acceptance owner changed".into());
                }
                e.validate(&s.vm, p)
            }
            Self::Move {
                position,
                level,
                execution,
            } => {
                if execution.operation.binding().1
                    != super::movement::commit_operation(actor, *position, *level)
                        .binding()
                        .1
                {
                    return Err("native physical movement destination changed".into());
                }
                execution.validate(&s.vm, p, actor)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetainedOrigin {
    pub completed_tick: u64,
    pub source_readset_sha256: String,
    /// New unpublished host epoch, not an original route allocator/history.
    pub adapter_epoch: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Walk {
    pub thread: ThreadId,
    pub request: u64,
    pub target: ObjectId,
    pub target_body: Body,
    pub started_tick: u64,
    pub yielded_tick: u64,
    /// When present, started_tick begins this Rust ownership epoch. The native
    /// route predates it; its path/progress/clock remain in the Resident.
    #[serde(default)]
    pub retained_origin: Option<RetainedOrigin>,
    #[serde(default)]
    pub poll: Option<movement::route::Poll>,
    #[serde(default)]
    pub physical: Option<Physical>,
    #[serde(default)]
    pub stopping: Option<movement::route::Stop>,
}
impl Walk {
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.physical
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub fn validate(&self, s: &Simulation, p: &Program, actor: ObjectId) -> Result<(), String> {
        if let Some(origin) = &self.retained_origin {
            if origin.completed_tick != self.started_tick
                || origin.source_readset_sha256.len() != 64
                || !origin.source_readset_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || origin.adapter_epoch.trim().is_empty()
                || origin.adapter_epoch.len() > 4096
                || origin.adapter_epoch.chars().any(char::is_control)
            {
                return Err("invalid retained native walk origin".into());
            }
        }
        let pending =
            s.vm.threads
                .get(&self.thread)
                .and_then(|t| crate::contention::pending_route(t, self.request));
        let native = s.native_lot.as_ref().ok_or("native walk lot absent")?;
        let path = s.characters[&actor]
            .controls
            .movement
            .path
            .as_ref()
            .ok_or("native walk calibrated path absent")?;
        if pending.is_none_or(|r| r.caller != actor || r.target != self.target)
            || self.started_tick > s.vm.tick
            || self.yielded_tick < self.started_tick
            || self.yielded_tick > s.vm.tick
            || s.routes.contains_key(&self.thread)
            || s.native_character_settings.is_none()
            || !native.routing_policy.builds_path(&s.vm.world)
            || native.route_paths.get(&actor) != Some(&path.points)
            || path.points.len() < 3
            || s.route_control
                .get(&self.request)
                .is_none_or(|c| c.original.is_none())
        {
            return Err("invalid native animator route binding".into());
        }
        if let Some(physical) = &self.physical {
            if self.poll.as_ref().and_then(|p| p.request()) != Some(physical.request()) {
                return Err("native route result lost its physical continuation".into());
            }
            physical.validate_execution(s, p, actor)?;
        }
        if let Some(stopping) = self.stopping {
            match stopping {
                movement::route::Stop::Failure | movement::route::Stop::Refresh
                    if self.poll.is_none() =>
                {
                    ()
                }
                movement::route::Stop::Success | movement::route::Stop::Retry
                    if self
                        .poll
                        .as_ref()
                        .and_then(|poll| poll.result())
                        .map(movement::route::Stop::after_poll)
                        .transpose()?
                        .flatten()
                        == Some(stopping) =>
                {
                    ()
                }
                _ => return Err("native walk lost its selected stop outcome".into()),
            }
        }
        Ok(())
    }
}

impl Simulation {
    pub(crate) fn drive_character_physical_step(
        &mut self,
        p: &Program,
        actor: ObjectId,
        request: movement::Request,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if self.characters[&actor].physical_movement.is_none() {
            let execution = Physical::new(self, actor, request)?;
            self.characters.get_mut(&actor).unwrap().physical_movement = Some(execution);
        }
        let mut execution = self
            .characters
            .get_mut(&actor)
            .unwrap()
            .physical_movement
            .take()
            .unwrap();
        let result = (|| {
            if execution.request() != request {
                return Err("native physical movement request changed".into());
            }
            match &mut execution {
                Physical::Accept(e) => {
                    if !self.drive_native_acceptance(p, e, remaining, step)? {
                        return Ok(false);
                    }
                    let accepted = e
                        .outcome()
                        .ok_or("native movement acceptance unfinished")?
                        .accepted;
                    self.characters
                        .get_mut(&actor)
                        .unwrap()
                        .controls
                        .movement
                        .accept_step(accepted)?;
                }
                Physical::Move { execution: e, .. } => {
                    if !self.drive_live_placement(p, e, remaining, step)? {
                        return Ok(false);
                    }
                    self.update_original_person_room(actor)?;
                    let r = self.characters.get_mut(&actor).unwrap();
                    r.controls.movement.moved(
                        &mut r.character.skin.animator,
                        &mut r.character.placement,
                        &mut r.controls.ordinary,
                    )?;
                }
            }
            Ok(true)
        })();
        if !result.as_ref().is_ok_and(|done| *done) {
            self.characters.get_mut(&actor).unwrap().physical_movement = Some(execution);
        }
        result
    }
    pub(crate) fn start_original_animator_path(
        &mut self,
        e: &super::live::Execution,
        step: &mut Step,
    ) -> Result<(), String> {
        let input = e.path.as_ref().ok_or("native route input absent")?;
        if self.body(e.caller)?.position != input.start || self.body(e.caller)?.level != input.level
        {
            return Err("native route start changed during callbacks".into());
        }
        let points = self
            .native_lot
            .as_ref()
            .unwrap()
            .route_paths
            .get(&e.caller)
            .ok_or("native retained path absent")?
            .clone();
        let mut resident = self
            .characters
            .get(&e.caller)
            .ok_or("native route actor absent")?
            .clone();
        if resident.native_walk.is_some()
            || resident.native_turn.is_some()
            || resident.ordinary_update.is_some()
            || resident.character.busy()
            || self.routes.contains_key(&e.thread)
        {
            return Err("native route overlaps an unfinished movement or character update".into());
        }
        resident
            .controls
            .movement
            .path
            .as_mut()
            .ok_or("native route calibration absent")?
            .points = points;
        let physical = self.vm.world.positions[&e.caller];
        let data22 = resident.controls.movement.start_route(
            &mut resident.character.placement,
            &mut resident.controls.ordinary,
            [physical.x, physical.y],
            self.vm.world.objects[&e.caller].data[&1],
        )?;
        resident.native_walk = Some(Walk {
            thread: e.thread,
            request: e.request,
            target: e.target,
            target_body: self.body(e.target)?.clone(),
            started_tick: self.vm.tick,
            yielded_tick: self.vm.tick,
            retained_origin: None,
            poll: None,
            physical: None,
            stopping: None,
        });
        *self
            .vm
            .world
            .objects
            .get_mut(&e.caller)
            .unwrap()
            .data
            .get_mut(&22)
            .ok_or("native movement object data 22 absent")? = data22;
        self.characters.insert(e.caller, resident);
        // Original 45ad63 state 12 starts the animator, publishes state 3 in
        // the route primitive's frame, then yields. This is VM state as well
        // as the host Walk record, and must survive a mid-walk save.
        self.vm
            .threads
            .get_mut(&e.thread)
            .unwrap()
            .stack
            .last_mut()
            .unwrap()
            .continuation = 3;
        self.event(
            &mut step.events,
            e.thread,
            "original-lot native animator route started".into(),
        );
        Ok(())
    }

    pub(crate) fn service_original_animator_path(
        &mut self,
        p: &Program,
        thread: ThreadId,
        request: &RouteRequest,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let actor = request.caller;
        let walk = self
            .characters
            .get(&actor)
            .ok_or("native walking actor absent")?
            .native_walk
            .as_ref()
            .ok_or("native walking route absent")?;
        if walk.thread != thread || walk.request != request.id {
            return Err("native walking route changed".into());
        }
        let mut walk = self
            .characters
            .get_mut(&actor)
            .unwrap()
            .native_walk
            .take()
            .unwrap();
        let outcome = (|| {
            // A physical subcall is still inside this invocation. Otherwise
            // consume the invalidation at wrapper entry, before cancellation
            // or the normal animator poll. State15 retains the prior result.
            if walk.physical.is_none() && walk.poll.as_ref().is_none_or(|p| p.request().is_none()) {
                let state = walk.stopping.map_or(3, |s| s.frame_continuation());
                let control = self
                    .route_control
                    .get_mut(&request.id)
                    .ok_or("native walking control absent")?;
                let original = control
                    .original
                    .as_mut()
                    .ok_or("native walking original context absent")?;
                if super::refresh::resume(&mut original.cache_valid, true, state)
                    == super::refresh::Resume::StopWalking
                {
                    walk.stopping = Some(movement::route::Stop::Refresh);
                }
            }
            if walk.poll.is_none() && walk.stopping.is_none() {
                let interrupted = self.route_interrupted(thread, actor)?;
                let has_path = self
                    .native_lot
                    .as_ref()
                    .unwrap()
                    .route_paths
                    .get(&actor)
                    .is_some_and(|p| !p.is_empty());
                let control = self
                    .route_control
                    .get_mut(&request.id)
                    .ok_or("native walking control absent")?;
                walk.stopping = movement::route::Stop::before_poll(
                    interrupted,
                    has_path,
                    &mut control.controller.result,
                );
            }
            if walk.poll.is_none() && walk.stopping.is_none() {
                let r = self.characters.get_mut(&actor).unwrap();
                let position = self.vm.world.positions[&actor];
                let facing = self
                    .vm
                    .world
                    .objects
                    .get_mut(&actor)
                    .unwrap()
                    .data
                    .get_mut(&1)
                    .ok_or("native walking physical facing absent")?;
                walk.poll = Some(movement::route::Poll::begin(
                    &r.controls.movement,
                    &mut r.character.placement,
                    facing,
                    [position.x, position.y],
                    position.level,
                )?);
                self.sync_script_directions()?;
            }
            while let Some(request) = walk.poll.as_ref().and_then(|p| p.request()) {
                if walk.physical.is_none() {
                    walk.physical = Some(Physical::new(self, actor, request)?);
                }
                match walk.physical.as_mut().unwrap() {
                    Physical::Accept(e) => {
                        if !self.drive_native_acceptance(p, e, remaining, step)? {
                            return Ok(false);
                        }
                        let accepted = e
                            .outcome()
                            .ok_or("native route result acceptance unfinished")?
                            .accepted;
                        walk.poll.as_mut().unwrap().accept(
                            &mut self.characters.get_mut(&actor).unwrap().character.placement,
                            accepted,
                        )?;
                    }
                    Physical::Move { execution, .. } => {
                        if !self.drive_live_placement(p, execution, remaining, step)? {
                            return Ok(false);
                        }
                        self.update_original_person_room(actor)?;
                        walk.poll.as_mut().unwrap().moved()?;
                    }
                }
                walk.physical = None;
            }
            if walk.stopping.is_none() {
                walk.stopping = movement::route::Stop::after_poll(
                    walk.poll
                        .as_ref()
                        .and_then(|p| p.result())
                        .ok_or("native route result absent")?,
                )?;
            }
            if walk.stopping.is_none() {
                // State 3 yields; only the scheduled person update advances
                // the animator. Re-poll its resulting status next tick.
                walk.poll = None;
                walk.yielded_tick = self.vm.tick;
                return Ok(false);
            }
            let r = self.characters.get_mut(&actor).unwrap();
            if !r
                .controls
                .movement
                .stop_route(&mut r.character.placement, &mut r.controls.ordinary)?
            {
                // States 9/10/11 retain their chosen result while stopping.
                self.vm
                    .threads
                    .get_mut(&thread)
                    .unwrap()
                    .stack
                    .last_mut()
                    .unwrap()
                    .continuation = walk.stopping.unwrap().frame_continuation();
                walk.yielded_tick = self.vm.tick;
                return Ok(false);
            }
            self.event(
                &mut step.events,
                thread,
                "original-lot native animator end_route".into(),
            );
            match walk.stopping.unwrap() {
                movement::route::Stop::Success => {
                    self.begin_native_route_completion(thread, request, true, 0)?
                }
                movement::route::Stop::Failure => {
                    let result = i32::from(self.route_control[&request.id].controller.result);
                    self.begin_native_route_completion(thread, request, false, result)?;
                }
                movement::route::Stop::Retry => {
                    let control = self
                        .route_control
                        .get_mut(&request.id)
                        .ok_or("native walk retry context absent")?;
                    control.controller.phase = crate::contention::Phase::Retry;
                    control.deciding = true;
                }
                movement::route::Stop::Refresh => {
                    // State15 goes directly to state1. The normal route
                    // dispatcher now rebuilds departure/target candidates.
                    // +58 retry stage, result, previous portal and path stay.
                }
            }
            Ok(true)
        })();
        if !outcome.as_ref().is_ok_and(|done| *done) {
            self.characters.get_mut(&actor).unwrap().native_walk = Some(walk);
        }
        outcome
    }
}
