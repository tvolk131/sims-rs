//! Explicit driver for authored primitive/animation fixtures, not a household
//! save importer. Object selection uses the same original `scheduler::Pass`
//! kernel as gameplay. The synthetic registry supplies enabled, non-idling
//! actors; its Update service runs the fixture's bare check/action threads.
//! It does not claim original construction, pause controls, or RNG ownership.
//! Real households reject this driver, including in restored snapshots.
use crate::{Simulation, Status, Step};
use serde::{Deserialize, Serialize};
use sims_vm::{
    ObjectId, Program, ThreadId,
    scheduler::{Inputs, Pass, Registry, Reply, Request},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    cursor: i32,
    pass: Option<Pass>,
    tick: Option<u64>,
    owners: BTreeMap<ThreadId, ObjectId>,
}
impl State {
    pub fn validate(&self, s: &Simulation) -> Result<(), String> {
        if s.native_scheduler.is_some()
            || self.cursor < 0
            || self.pass.is_some() != self.tick.is_some()
            || self.tick.is_some_and(|tick| tick != s.vm.tick)
            || self.owners.values().any(|id| *id <= 0)
        {
            return Err("invalid synthetic fixture pass owner".into());
        }
        if let Some(pass) = &self.pass {
            pass.validate()?;
        }
        if let Some((thread, actor)) = s.visit {
            if actor != self.owners.get(&thread).copied()
                || self.pass.as_ref().and_then(Pass::request)
                    != actor
                        .map(|object| Request::Tick {
                            object: object as u32,
                            tick: s.vm.tick as u32,
                        })
                        .as_ref()
            {
                return Err("fixture Update lost its registry request".into());
            }
        }
        Ok(())
    }
}
struct Actors<'a>(&'a BTreeMap<ThreadId, ObjectId>);
impl Registry for Actors<'_> {
    fn len(&self) -> usize {
        self.0.values().copied().max().unwrap_or(0) as usize
    }
    fn object(&self, slot: i32) -> Result<Option<u32>, String> {
        Ok(self
            .0
            .values()
            .any(|id| i32::from(*id) == slot)
            .then_some(slot as u32))
    }
    fn word(&self, _: i32) -> Result<u32, String> {
        Ok(1 << 16)
    }
    fn set_word(&mut self, _: i32, _: u32) -> Result<(), String> {
        Err("synthetic fixture registry has no countdown or error flags".into())
    }
    fn error(&self, _: u32) -> Result<i16, String> {
        Ok(0)
    }
}
impl Simulation {
    /// Explicitly run an authored laboratory fixture. Never a fallback from
    /// `run_current_tick`, and never enabled by loading a household snapshot.
    pub fn run_fixture_tick(&mut self, program: &Program, budget: usize) -> Result<Step, String> {
        if self.native_scheduler.is_some() {
            return Err("fixture driver cannot run an original household".into());
        }
        self.run_scheduled_slice(program, budget, Self::next_fixture_thread)
    }
    pub(crate) fn prepare_fixture_pass(&mut self) -> Result<(), String> {
        self.fixture_driver.get_or_insert_with(Default::default);
        self.fixture_driver.as_ref().unwrap().validate(self)?;
        if self
            .fixture_driver
            .as_ref()
            .unwrap()
            .pass
            .as_ref()
            .is_some_and(|p| p.result().is_some())
        {
            let state = self.fixture_driver.as_mut().unwrap();
            state.pass = None;
            state.tick = None;
        }
        Ok(())
    }
    pub(crate) fn next_fixture_thread(&mut self) -> Result<Option<ThreadId>, String> {
        if let Some((thread, _)) = self.visit {
            return Ok(Some(thread));
        }
        self.prepare_fixture_pass()?;
        let state = self.fixture_driver.as_mut().unwrap();
        state
            .owners
            .retain(|thread, _| self.vm.threads.contains_key(thread));
        for (&id, thread) in &self.vm.threads {
            if let Some(actor) = thread
                .stack
                .first()
                .map(|f| f.caller)
                .or_else(|| self.pending_actions.get(&id).map(|a| a.actor))
                .or_else(|| {
                    self.vm
                        .object_threads
                        .iter()
                        .find_map(|(&actor, &t)| (t == id).then_some(actor))
                })
            {
                if actor <= 0 {
                    return Err("fixture registry requires positive actor IDs".into());
                }
                state.owners.insert(id, actor);
            }
        }
        let input = Inputs {
            paused: 0,
            mode: 0,
            lot_number: 0,
            tick: self.vm.tick as u32,
        };
        if state.pass.is_none() {
            state.pass = Some(Pass::begin(
                input,
                &Actors(&state.owners),
                &mut state.cursor,
            )?);
            state.tick = Some(self.vm.tick);
        }
        loop {
            let pass = state.pass.as_mut().unwrap();
            let Some(request) =
                pass.advance(&mut Actors(&state.owners), &mut state.cursor, input)?
            else {
                return Ok(None);
            };
            let Request::Tick { object, .. } = request else {
                return Err("synthetic fixture produced an object error service".into());
            };
            // Multiple bare scripts are the fixture's explicit Update service.
            // They are not extra globally scheduled objects. Newly spawned
            // action/check continuations finish before this Update returns.
            if let Some((&thread, _)) = state.owners.iter().find(|(id, actor)| {
                **actor as u32 == object
                    && (!self.visited.contains(id) || self.vm.threads[id].status == Status::Ready)
            }) {
                return Ok(Some(thread));
            }
            pass.acknowledge(&request, Reply::Tick(1))?;
        }
    }
    /// Advance the explicit fixture clock after all its serviced work yields.
    /// Gameplay uses `finish_registry_pass` instead, including partial passes.
    pub fn advance_fixture_tick(&mut self) -> Result<(), String> {
        if self.native_scheduler.is_some() {
            return Err("original scheduler requires finish_registry_pass".into());
        }
        self.validate_pass_boundary()?;
        if let Some(state) = &self.fixture_driver {
            state.validate(self)?;
            if state
                .pass
                .as_ref()
                .is_some_and(|p| p.result() != Some(true))
            {
                return Err("fixture registry pass has not returned".into());
            }
        }
        if self
            .pending_actions
            .keys()
            .any(|id| matches!(self.vm.threads[id].status, Status::Returned(_)))
            || self.vm.threads.iter().any(|(id, t)| match &t.status {
                Status::AwaitingAnimation(r) => {
                    if let Some(character) = self.characters.get(&r.caller) {
                        character.animation_yield != Some((*id, r.id, self.vm.tick))
                    } else {
                        self.animations.get(&r.caller).is_none_or(|p| {
                            p.last_yield_tick != Some(self.vm.tick)
                                && (p.ended || !p.pending.is_empty())
                        }) || r.operands[..2] == [0, 0]
                    }
                }
                Status::AwaitingSnap(_)
                | Status::AwaitingCreation(_)
                | Status::AwaitingImmediate(_)
                | Status::AwaitingRemoval(_)
                | Status::AwaitingBurn(_)
                | Status::AwaitingDialog(_)
                | Status::AwaitingUiEffect(_)
                | Status::AwaitingPlacement(_)
                | Status::AwaitingTransfer(_) => true,
                Status::AwaitingTurn(r) => {
                    if let Some(character) = self.characters.get(&r.caller) {
                        character.native_turn.as_ref().is_none_or(|turn| {
                            turn.thread != *id
                                || turn.request != *r
                                || turn.yielded_tick != Some(self.vm.tick)
                                || turn.poll.is_some()
                                || turn.physical.is_some()
                                || character.evaluated_tick != self.vm.tick
                        })
                    } else {
                        !self.turns.contains_key(&r.caller)
                    }
                }
                Status::AwaitingReach(r) => {
                    if self.native_lot.is_some() {
                        self.native_reaches
                            .get(&r.id)
                            .is_none_or(|e| !e.yielded(*id, r.id, self.vm.tick))
                    } else {
                        self.animations
                            .get(&r.caller)
                            .is_none_or(|a| a.last_yield_tick != Some(self.vm.tick))
                    }
                }
                Status::AwaitingRoute(request) => {
                    if let Some(character) = self.characters.get(&request.caller) {
                        character.native_walk.as_ref().is_none_or(|walk| {
                            walk.thread != *id
                                || walk.request != request.id
                                || walk.yielded_tick != self.vm.tick
                                || character.evaluated_tick != self.vm.tick
                        })
                    } else {
                        self.routes
                            .get(id)
                            .is_none_or(|r| r.last_tick < self.vm.tick)
                    }
                }
                _ => false,
            })
        {
            return Err("host work remains in the current tick".into());
        }
        self.vm.advance_tick()?;
        self.advance_household_clock(true);
        if let Some(state) = &mut self.fixture_driver {
            state.pass = None;
            state.tick = None;
        }
        Ok(())
    }
}
