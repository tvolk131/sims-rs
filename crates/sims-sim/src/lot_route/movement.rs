//! Imported-lot walking commits. Placement and room-cache ordering follow
//! person virtual +0x88 (0x45e4e9); the locomotion clock is the existing tuned
//! presentation model, not a recovered implementation of the native animator.
use crate::{
    Point, Route, Simulation, Step, locomotion::Locomotion, lot_acceptance, lot_links::Descriptor,
    lot_placement,
};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, RouteRequest, ThreadId, Vm};

/// After a rejected animator step, +0x188 is 3. Native 0x589d47 returns
/// failure (0), and 0x589e2d can stop that path immediately: state 3 -> 10 -> 6.
/// Cancellation or an empty retained path selects 11 -> 7 instead. The
/// placement diagnostic is separate from the existing route result.
fn rejected_walk_return(interrupted: bool, has_path: bool, result: i16) -> (u8, i16) {
    if interrupted {
        (7, 4)
    } else if !has_path {
        (7, result)
    } else {
        (6, result)
    }
}

/// Animator 0x58eeb5..0x58eee8 passes null container, slot zero. The person
/// override forces notify=1, then updates room membership after base SetObject.
pub fn commit_operation(
    actor: ObjectId,
    position: [i32; 2],
    level: i32,
) -> lot_placement::Operation {
    lot_placement::Operation::set(
        actor,
        Descriptor {
            on_lot: true,
            position,
            level,
            container: None,
            slot: 0,
        },
        true,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    pub thread: ThreadId,
    request: u64,
    caller: ObjectId,
    tick: u64,
    pub locomotion: Locomotion,
    pub acceptance: lot_acceptance::live::Execution,
    pub placement: Option<lot_placement::live::Execution>,
}
impl Execution {
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.acceptance.owns_callback(vm, p, thread, depth)
            || self
                .placement
                .as_ref()
                .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }
    pub fn validate(&self, s: &Simulation, p: &Program) -> Result<(), String> {
        let r = s
            .routes
            .get(&self.thread)
            .ok_or("native movement route absent")?;
        let mut next = r.locomotion.clone();
        next.elapsed = next
            .elapsed
            .checked_add(1)
            .ok_or("native movement clock overflow")?;
        let q = next.anchor(&r.path);
        if self.request != r.request
            || self.caller != r.caller
            || self.tick != s.vm.tick
            || r.last_tick.checked_add(1) != Some(self.tick)
            || next != self.locomotion
            || next.elapsed > next.duration()
            || s.visit.map(|v| v.0) != Some(self.thread)
            || self.acceptance.binding() != (self.caller, [q.y, q.x], s.body(self.caller)?.level)
            || s.native_lot.as_ref().is_none_or(|n| {
                n.route.is_some() || n.routing_policy != super::path::Policy::InferredRectangles
            })
        {
            return Err("invalid native movement continuation".into());
        }
        self.acceptance.validate(&s.vm, p)?;
        if let Some(e) = &self.placement {
            e.validate(&s.vm, p, self.caller)?;
            if !self.acceptance.outcome().is_some_and(|o| o.accepted)
                || e.operation.binding().1
                    != commit_operation(self.caller, [q.y, q.x], s.body(self.caller)?.level)
                        .binding()
                        .1
            {
                return Err("native movement placed before acceptance".into());
            }
        }
        Ok(())
    }
}
impl Simulation {
    pub(crate) fn install_original_path(
        &mut self,
        p: &Program,
        e: &super::live::Execution,
        step: &mut Step,
    ) -> Result<(), String> {
        if self.native_character_settings.is_some() {
            return self.start_original_animator_path(e, step);
        }
        if e.policy == super::path::Policy::Strict {
            return Err("strict original routing requires the native character animator".into());
        }
        let input = e.path.as_ref().ok_or("original path absent")?;
        let original_path = input.points.clone().ok_or("original path incomplete")?;
        let mut path = original_path.clone();
        path.dedup();
        let caller = self.body(e.caller)?;
        if caller.position != input.start
            || caller.level != input.level
            || self.routes.contains_key(&e.thread)
        {
            return Err("original path start changed during callbacks".into());
        }
        let goal = e.goal.clone().ok_or("original path destination absent")?;
        let locomotion = Locomotion::plan(p, &path, caller.facing, goal.facing)?;
        self.routes.insert(
            e.thread,
            Route {
                request: e.request,
                caller: e.caller,
                target: e.target,
                target_body: self.body(e.target)?.clone(),
                goal,
                path,
                original_path: Some(original_path),
                next: locomotion.next(),
                started_tick: self.vm.tick,
                last_tick: self.vm.tick,
                locomotion,
            },
        );
        // Route state 12 -> 3 shares 0x45b962 with the portal yield. Its
        // per-request ignored object stays in context +0x94 while person72 is
        // cleared; state 3 restores it when the animator finishes.
        self.person(e.caller, 72, 0)?;
        self.event(&mut step.events,e.thread,"original-lot experimental path started; geometry/search UNVERIFIED, locomotion timing tuned".into());
        Ok(())
    }
    pub(crate) fn drive_pending_native_movement(
        &mut self,
        p: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let Some(mut e) = self.native_lot.as_mut().and_then(|n| n.movement.take()) else {
            return Ok(true);
        };
        let result = self.drive_native_movement(p, &mut e, remaining, step);
        if !result.as_ref().is_ok_and(|done| *done) {
            self.native_lot.as_mut().unwrap().movement = Some(e);
        }
        result
    }
    fn drive_native_movement(
        &mut self,
        p: &Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if !self.drive_native_acceptance(p, &mut e.acceptance, remaining, step)? {
            return Ok(false);
        }
        let outcome = e
            .acceptance
            .outcome()
            .ok_or("native movement acceptance incomplete")?;
        if !outcome.accepted {
            let request = crate::contention::pending_route(&self.vm.threads[&e.thread], e.request)
                .ok_or("rejected walking request absent")?
                .clone();
            let interrupted = self.route_interrupted(e.thread, e.caller)?;
            let has_path = self
                .native_lot
                .as_ref()
                .unwrap()
                .route_paths
                .get(&e.caller)
                .is_some_and(|p| !p.is_empty());
            let prior = self
                .route_control
                .get(&e.request)
                .ok_or("rejected walking context absent")?
                .controller
                .result;
            let (state, result) = rejected_walk_return(interrupted, has_path, prior);
            // Stopping the tuned movement track represents 0x589e2d. Keep
            // physical position, retained path, context blocker and person72;
            // do not submit the rejected coordinate or stop a separate BHAV clip.
            self.release(e.thread);
            if state == 6 {
                let control = self.route_control.get_mut(&e.request).unwrap();
                control.controller.phase = crate::contention::Phase::Retry;
                control.deciding = true;
            } else {
                self.begin_native_route_completion(e.thread, &request, false, i32::from(result))?;
            }
            self.event(&mut step.events, e.thread, format!(
                "original-lot rejected walking step: placement {}; native return state {state}, route result {result}",
                outcome.error));
            return Ok(true);
        }
        let (_, to, level) = e.acceptance.binding();
        if e.placement.is_none() {
            e.placement = Some(lot_placement::live::Execution::new(commit_operation(
                e.caller, to, level,
            )));
        }
        if !self.drive_live_placement(p, e.placement.as_mut().unwrap(), remaining, step)? {
            return Ok(false);
        }
        // 0x45e4e9 invokes this even when base placement itself changed no tile.
        self.update_original_person_room(e.caller)?;
        if self.body(e.caller)?.position != Point::new(to[1], to[0])
            || self.body(e.caller)?.level != level
        {
            return Err("original movement placement callback changed its destination".into());
        }
        let r = self
            .routes
            .get_mut(&e.thread)
            .ok_or("native movement route disappeared")?;
        r.locomotion = e.locomotion.clone();
        r.next = e.locomotion.next();
        r.last_tick = e.tick;
        Ok(true)
    }
    /// True means the request completed; false is a normal tick wait, or a
    /// diagnostic budget split when native_lot.movement remains present.
    pub(crate) fn service_original_movement(
        &mut self,
        p: &Program,
        id: ThreadId,
        request: &RouteRequest,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if self
            .route_control
            .get(&request.id)
            .and_then(|c| c.original.as_ref())
            .is_some_and(|c| !c.cache_valid)
        {
            return Err("house refresh requires the original animator's stop/resume adapter; tuned movement cannot consume it".into());
        }
        let r = self
            .routes
            .get(&id)
            .ok_or("native movement route absent")?
            .clone();
        if r.request != request.id {
            return Err("native movement request changed".into());
        }
        if self.route_interrupted(id, request.caller)? {
            return Err("original-lot interrupted-walk adapter required".into());
        }
        let carried_target = request.target == request.caller
            || self.vm.world.objects[&request.target]
                .container
                .is_some_and(|(parent, _)| parent == request.caller);
        if !carried_target && self.body(request.target)? != &r.target_body {
            return Err("original-lot moving-target adapter required".into());
        }
        if r.locomotion.elapsed < r.locomotion.duration() {
            if self.vm.tick == r.last_tick {
                return Ok(false);
            }
            if self.vm.tick != r.last_tick + 1 {
                return Err("original movement skipped a simulation tick".into());
            }
            let mut locomotion = r.locomotion.clone();
            locomotion.elapsed += 1;
            let to = locomotion.anchor(&r.path);
            self.face(request.caller, locomotion.facing())?;
            let e = Execution {
                thread: id,
                request: request.id,
                caller: request.caller,
                tick: self.vm.tick,
                locomotion,
                acceptance: lot_acceptance::live::Execution::new(
                    self,
                    request.caller,
                    [to.y, to.x],
                    self.body(request.caller)?.level,
                )?,
                placement: None,
            };
            self.native_lot.as_mut().unwrap().movement = Some(e);
            if !self.drive_pending_native_movement(p, remaining, step)? {
                return Ok(false);
            }
            if !self.routes.contains_key(&id) {
                return Ok(true);
            }
            if self.routes[&id].locomotion.elapsed < self.routes[&id].locomotion.duration() {
                return Ok(false);
            }
        }
        let ignored = self
            .route_control
            .get(&request.id)
            .and_then(|c| c.original.as_ref())
            .ok_or("native movement context absent")?
            .ignored;
        self.person(request.caller, 72, ignored)?;
        self.face(request.caller, r.goal.facing)?;
        self.begin_native_route_completion(id, request, true, 0)?;
        self.drive_native_route_completion(p, remaining, step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[cfg(feature = "private-fixtures")]
    #[test]
    fn rejected_steps_match_native_animator_and_route_handoff() {
        let report: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/blocked-walking.json"))
                .unwrap();
        assert_eq!(report["steps"].as_array().unwrap().len(), 16);
        for row in report["steps"].as_array().unwrap() {
            let c = &row["input"];
            let n = &row["original"];
            let accepted = c["accepted"] == true;
            let events = n["events"].as_array().unwrap();
            assert_eq!(events.len(), if accepted { 2 } else { 1 });
            assert_eq!(events[0]["kind"], "check");
            let destination = commit_operation(1, [41, 58], c["level"].as_i64().unwrap() as i32)
                .binding()
                .1
                .unwrap();
            for event in events {
                assert_eq!(event["position"], json!(destination.position));
                assert_eq!(event["level"], json!(destination.level));
                assert_eq!(event["container"], 0);
                assert_eq!(event["slot"], 0);
            }
            if accepted {
                assert_eq!(events[1]["kind"], "commit");
                assert_eq!(n["animator"]["0x80"], json!(41.25f32.to_bits()));
                assert_eq!(n["animator"]["0x84"], json!(58.5f32.to_bits()));
            } else {
                assert_eq!(n["animator"]["0x188"], 3);
                assert_eq!(n["animator"]["0x80"], json!(40.25f32.to_bits()));
                assert_eq!(n["animator"]["0x84"], json!(56.5f32.to_bits()));
            }
        }
        let mut cases = 0;
        for row in report["returns"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["input"]["animator"] == 3)
        {
            let c = &row["input"];
            let n = &row["original"];
            let (state, result) = rejected_walk_return(
                c["interrupted"] == true && c["interruptible"] == true,
                c["path"] == true,
                13,
            );
            assert_eq!(json!(state), n["state"], "{c}");
            assert_eq!(json!(result), n["result"], "{c}");
            assert_eq!(n["yielded"], false);
            assert_eq!(n["animator"]["0x188"], 0);
            assert_eq!(n["animator"]["0x180"], json!(u32::MAX));
            assert_eq!(n["animator"]["0x1c0"], 0);
            assert_eq!(n["animator"]["0x18c"], 1);
            assert_eq!(n["animator"]["0x190"], 3);
            for (to, from) in [("0x1c4", "0x80"), ("0x1c8", "0x84"), ("0x1cc", "0x88")] {
                assert_eq!(n["animator"][to], n["animator"][from]);
            }
            assert_eq!(n["person72"], 0);
            assert_eq!(n["data9"], 7);
            assert_eq!(n["blocked_object"], 21);
            assert_eq!(n["stage"], 2);
            cases += 1;
        }
        assert_eq!(cases, 8);
    }
}
