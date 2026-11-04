//! Original standing Look Towards animation on the supplied adult profile.
//! The animator runs after the person's VM visit. Presentation only samples it.
use crate::{Event, Simulation};
use serde::{Deserialize, Serialize};
use sims_vm::{Program, ThreadId, TurnRequest, look};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    Starting,
    PathReady,
    Turning,
    Complete,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Turn {
    pub request: u64,
    pub thread: ThreadId,
    pub path: [[i32; 2]; 3],
    pub started_tick: u64,
    pub last_tick: Option<u64>,
    pub phase: Phase,
    pub from: f32,
    pub target: f32,
    pub base: f32,
    pub heading: f32,
    pub clip: Option<String>,
    pub progress: f32,
    pub next_event: usize,
}
impl Turn {
    fn new(
        program: &Program,
        request: &TurnRequest,
        thread: ThreadId,
        path: [[i32; 2]; 3],
        heading: f32,
        tick: u64,
    ) -> Result<Self, String> {
        let delta = std::array::from_fn(|i| path[2][i].wrapping_sub(path[1][i]));
        let target = look::target_heading(delta);
        let change = look::heading_delta(heading, target);
        let choice = look::standing_turn(heading, change)?;
        let clip = if change.abs() < 0.25 {
            None
        } else {
            let name = program
                .modules
                .get(&program.globals)
                .and_then(|m| m.strings.get(&150))
                .and_then(|s| s.get(usize::from(choice.clip)))
                .ok_or("missing standard adult standing turn name")?
                .to_ascii_lowercase();
            let asset = program
                .animations
                .get(&name)
                .ok_or_else(|| format!("missing standing turn clip {name}"))?;
            if !asset.clip.duration_ms.is_finite()
                || !(1. ..=3_600_000.).contains(&asset.clip.duration_ms)
                || asset.clip.frames == 0
            {
                return Err("invalid standing turn duration".into());
            }
            // Footsteps are presentation events. Reject other properties until their engine semantics are supplied.
            if asset.clip.events.iter().any(|e| {
                e.pairs
                    .iter()
                    .any(|(k, v)| k != "footstep" || v.parse::<i16>().is_err())
            }) {
                return Err(format!("unsupported standing turn timed property: {name}"));
            }
            Some(name)
        };
        Ok(Self {
            request: request.id,
            thread,
            path,
            started_tick: tick,
            last_tick: None,
            phase: Phase::Starting,
            from: heading,
            target,
            base: choice.base_heading,
            heading,
            clip,
            progress: 0.,
            next_event: 0,
        })
    }
    fn update(&mut self, program: &Program, tick: u64) -> Result<(), String> {
        if self.last_tick == Some(tick) {
            return Ok(());
        }
        if self
            .last_tick
            .is_some_and(|last| last.checked_add(1) != Some(tick))
            || (self.last_tick.is_none() && tick != self.started_tick)
        {
            return Err("standing turn clock skipped a tick".into());
        }
        match self.phase {
            Phase::Starting => self.phase = Phase::PathReady,
            Phase::PathReady => {
                if self.clip.is_some() {
                    self.phase = Phase::Turning;
                } else {
                    self.heading = self.target;
                    self.phase = Phase::Complete;
                }
            }
            Phase::Turning => {
                self.heading = look::pose_heading(self.from, self.base, self.target, self.progress);
                if self.progress >= 1. {
                    self.phase = Phase::Complete;
                } else {
                    let duration = program.animations[self.clip.as_ref().unwrap()]
                        .clip
                        .duration_ms;
                    let elapsed = look::animation_time(tick as i32, false)
                        .wrapping_sub(look::animation_time(self.last_tick.unwrap() as i32, false));
                    self.progress = look::advance_progress(self.progress, elapsed, duration).0;
                }
            }
            Phase::Complete => (),
        }
        if matches!(self.phase, Phase::Turning | Phase::Complete)
            && let Some(name) = &self.clip
        {
            let clip = &program.animations[name].clip;
            let elapsed_ms =
                (f64::from(self.progress) * f64::from(clip.duration_ms)).trunc() as u32;
            self.next_event = clip
                .events
                .partition_point(|e| i64::from(e.time_ms) <= i64::from(elapsed_ms));
        }
        self.last_tick = Some(tick);
        Ok(())
    }
    /// Both the source root rotation and native heading correction contribute.
    /// The stored correction precedes this tick's clip advancement in the engine.
    pub fn sample(&self, program: &Program, fraction: f32) -> Option<(&str, f32, f32)> {
        if self.phase != Phase::Turning {
            return None;
        }
        let clip = self.clip.as_ref()?;
        let asset = &program.animations[clip];
        let progress =
            (self.progress + fraction.clamp(0., 1.) * 50. / asset.clip.duration_ms).min(1.);
        let heading = self.heading
            + fraction.clamp(0., 1.)
                // While the source turn pose remains visible, its root still
                // contributes nominal rotation. The full target heading belongs
                // to the following standing pose, after Phase::Complete.
                * (look::pose_heading(self.from, self.base, self.base, self.progress)
                    - self.heading);
        Some((
            clip,
            progress * asset.clip.frames as f32,
            heading * std::f32::consts::FRAC_PI_4,
        ))
    }
}
impl Simulation {
    pub(crate) fn service_turn(
        &mut self,
        program: &Program,
        thread: ThreadId,
        request: &TurnRequest,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        if let Some(turn) = self.turns.get(&request.caller) {
            if turn.thread != thread || turn.request != request.id {
                return Err("standing turn owner changed".into());
            }
            if turn.phase != Phase::Complete {
                return Ok(false);
            }
            self.vm.finish_turn(program, thread, request.id, true)?;
            self.turns.remove(&request.caller);
            self.event(events, thread, "original standing turn completed".into());
            return Ok(true);
        }
        let actor = &self.vm.world.objects[&request.caller];
        let person = actor.person.as_ref().ok_or("turn actor is not a person")?;
        let age = *person.get(&58).ok_or("turn actor age is missing")?;
        if age < 18 || person.get(&65).copied().unwrap_or(0) > 1 {
            return Err("standing turn needs the loaded adult profile".into());
        }
        // Native 0x58f4ee selects the same standard turn with occupied hands.
        // 0x5926d7 replaces only the foreground; the independent carry layer
        // continues to pose the arm and held object above it.
        let body = self.body(request.caller)?;
        let position = [body.position.y, body.position.x];
        let target = self
            .bodies
            .get(&request.target)
            .map(|b| [b.position.y, b.position.x]);
        let look::Preparation::Start { path, clear_data9 } = look::prepare(
            request.mode,
            0,
            0,
            position,
            target,
            self.vm.world.camera_rotation,
        )?
        else {
            unreachable!()
        };
        let turn = Turn::new(
            program,
            request,
            thread,
            path,
            body.facing.trailing_zeros() as f32,
            self.vm.tick,
        )?;
        if clear_data9 {
            *self
                .vm
                .world
                .objects
                .get_mut(&request.caller)
                .unwrap()
                .data
                .get_mut(&9)
                .ok_or("turn data9 missing")? = 0;
        }
        self.turns.insert(request.caller, turn);
        self.event(events, thread, "started original standing turn path".into());
        Ok(false)
    }
    pub(crate) fn update_turn(
        &mut self,
        program: &Program,
        thread: ThreadId,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let actor = self
            .turns
            .iter()
            .find_map(|(&actor, t)| (t.thread == thread).then_some(actor));
        if let Some(actor) = actor {
            let turn = self.turns.get_mut(&actor).unwrap();
            let phase = turn.phase;
            let advancing = turn.last_tick != Some(self.vm.tick);
            let before = turn.next_event;
            turn.update(program, self.vm.tick)?;
            let facing = 1 << look::quantized_heading(turn.heading);
            if let Some(name) = &turn.clip {
                for event in &program.animations[name].clip.events[before..turn.next_event] {
                    events.push(Event {
                        tick: self.vm.tick,
                        thread,
                        detail: format!(
                            "standing turn {name} at {} ms: {:?}",
                            event.time_ms, event.pairs
                        ),
                    });
                }
            }
            if advancing && phase == Phase::PathReady {
                self.animations.remove(&actor);
            }
            self.face(actor, facing)?;
        }
        Ok(())
    }
    pub(crate) fn validate_turns(&self, program: &Program) -> Result<(), String> {
        if self.vm.world.camera_rotation > 3 || self.turns.len() > self.bodies.len() {
            return Err("invalid turn/camera state".into());
        }
        for (&actor, turn) in &self.turns {
            let t = self
                .vm
                .threads
                .get(&turn.thread)
                .ok_or("orphaned standing turn")?;
            let sims_vm::Status::AwaitingTurn(request) = &t.status else {
                return Err("standing turn without waiting frame".into());
            };
            let body = self.body(actor)?;
            if actor != request.caller
                || turn.request != request.id
                || turn.path[0] != turn.path[1]
                || turn.path[0] != [body.position.y, body.position.x]
                || !turn.from.is_finite()
                || !(0. ..=7.).contains(&turn.from)
                || turn.started_tick > self.vm.tick
                || turn
                    .last_tick
                    .is_some_and(|t| t > self.vm.tick || t < turn.started_tick)
                || turn.last_tick.is_none() && turn.started_tick != self.vm.tick
                || self.routes.contains_key(&turn.thread)
            {
                return Err("invalid standing turn binding".into());
            }
            let mut expected = Turn::new(
                program,
                request,
                turn.thread,
                turn.path,
                turn.from,
                turn.started_tick,
            )?;
            if let Some(last) = turn.last_tick {
                if last - turn.started_tick > 75_000 {
                    return Err("oversized standing turn duration".into());
                }
                for tick in turn.started_tick..=last {
                    expected.update(program, tick)?;
                }
            }
            if expected != *turn || body.facing != 1 << look::quantized_heading(turn.heading) {
                return Err("standing turn cursor disagrees with its original timeline".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
