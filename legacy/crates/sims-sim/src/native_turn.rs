//! 45a98e Look Towards over the resident's shared native animator. The primitive
//! starts the same retained path as walking, polls 589d47 (including physical
//! acceptance/placement), promotes failure to success, then calls 589e2d.
use crate::{Simulation, Step, lot_route::animator::Physical, pose::movement};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, TurnRequest, Vm, look};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub thread: ThreadId,
    pub request: TurnRequest,
    pub started_tick: u64,
    pub yielded_tick: Option<u64>,
    pub path: [[i32; 2]; 3],
    pub poll: Option<movement::route::Poll>,
    pub physical: Option<Physical>,
}

pub(crate) enum Outcome {
    Ready,
    Waiting,
    Budget,
}

impl Turn {
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.physical
            .as_ref()
            .is_some_and(|e| e.owns_callback(vm, p, thread, depth))
    }

    pub fn validate(&self, s: &Simulation, p: &Program, actor: ObjectId) -> Result<(), String> {
        let thread =
            s.vm.threads
                .get(&self.thread)
                .ok_or("native turn thread absent")?;
        // A synchronous placement callback can temporarily cover AwaitingTurn.
        let waiting = std::iter::once(&thread.status)
            .chain(
                thread
                    .callback
                    .iter()
                    .flat_map(|c| c.chain())
                    .map(|c| &c.resume_status),
            )
            .chain(
                thread
                    .service
                    .iter()
                    .flat_map(|c| c.chain())
                    .map(|c| &c.resume_status),
            )
            .any(|status| matches!(status, Status::AwaitingTurn(r) if r == &self.request));
        let resident = &s.characters[&actor];
        if !waiting
            || self.request.caller != actor
            || self.started_tick > s.vm.tick
            || self
                .yielded_tick
                .is_some_and(|t| t < self.started_tick || t > s.vm.tick)
            || self.path[0] != self.path[1]
            || resident.native_walk.is_some()
            || s.turns.contains_key(&actor)
            || s.routes.contains_key(&self.thread)
            || s.native_character_settings.is_none()
            || s.native_lot
                .as_ref()
                .and_then(|n| n.route_paths.get(&actor))
                .map(Vec::as_slice)
                != Some(self.path.as_slice())
            || resident
                .controls
                .movement
                .path
                .as_ref()
                .map(|p| p.points.as_slice())
                != Some(self.path.as_slice())
        {
            return Err("invalid native standing turn binding".into());
        }
        if let Some(physical) = &self.physical {
            if self.poll.as_ref().and_then(|p| p.request()) != Some(physical.request()) {
                return Err("native turn lost its physical continuation".into());
            }
            physical.validate_execution(s, p, actor)?;
        }
        Ok(())
    }
}

impl Simulation {
    pub(crate) fn drive_pending_native_turn(
        &mut self,
        program: &Program,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        let pending = self.characters.values().find_map(|r| {
            let turn = r.native_turn.as_ref()?;
            turn.poll.as_ref()?.request()?;
            Some((turn.thread, turn.request.clone()))
        });
        if let Some((thread, request)) = pending {
            return Ok(!matches!(
                self.service_native_turn(program, thread, &request, remaining, step)?,
                Outcome::Budget
            ));
        }
        Ok(true)
    }

    pub(crate) fn service_native_turn(
        &mut self,
        program: &Program,
        thread: ThreadId,
        request: &TurnRequest,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Outcome, String> {
        let actor = request.caller;
        if self.characters[&actor].native_turn.is_none() {
            let position = self
                .vm
                .world
                .positions
                .get(&actor)
                .ok_or("native turn physical position absent")?;
            let target = self
                .vm
                .world
                .positions
                .get(&request.target)
                .map(|p| [p.x, p.y]);
            let look::Preparation::Start { path, clear_data9 } = look::prepare(
                request.mode,
                0,
                0,
                [position.x, position.y],
                target,
                self.vm.world.camera_rotation,
            )?
            else {
                unreachable!()
            };
            let object = &self.vm.world.objects[&actor];
            let facing = *object.data.get(&1).ok_or("native turn facing absent")?;
            if !object.data.contains_key(&22) || (clear_data9 && !object.data.contains_key(&9)) {
                return Err("native turn object fields absent".into());
            }
            let resident = &self.characters[&actor];
            if resident.native_walk.is_some()
                || resident.ordinary_update.is_some()
                || resident.character.busy()
                || self.turns.contains_key(&actor)
                || self.routes.contains_key(&thread)
            {
                return Err("native turn overlaps unfinished movement or character work".into());
            }
            // Stage start before publishing it; failure must not leave a partial path.
            let mut movement = resident.controls.movement.clone();
            let mut placement = resident.character.placement.clone();
            let mut ordinary = resident.controls.ordinary.clone();
            movement
                .path
                .as_mut()
                .ok_or("native turn calibration absent")?
                .points = path.to_vec();
            let scalar = movement.start_route(&mut placement, &mut ordinary, path[0], facing)?;
            self.native_lot
                .as_mut()
                .ok_or("native turn lot absent")?
                .route_paths
                .insert(actor, path.to_vec());
            let resident = self.characters.get_mut(&actor).unwrap();
            resident.controls.movement = movement;
            resident.character.placement = placement;
            resident.controls.ordinary = ordinary;
            resident.native_turn = Some(Turn {
                thread,
                request: request.clone(),
                started_tick: self.vm.tick,
                yielded_tick: None,
                path,
                poll: None,
                physical: None,
            });
            let object = self.vm.world.objects.get_mut(&actor).unwrap();
            if clear_data9 {
                *object.data.get_mut(&9).unwrap() = 0;
            }
            *object.data.get_mut(&22).unwrap() = scalar;
            self.event(
                &mut step.events,
                thread,
                "native Look Towards path started".into(),
            );
        }
        let mut turn = self
            .characters
            .get_mut(&actor)
            .unwrap()
            .native_turn
            .take()
            .unwrap();
        let outcome = (|| {
            if turn.thread != thread || turn.request != *request {
                return Err("native turn request changed".into());
            }
            if turn.poll.is_none() {
                let position = self.vm.world.positions[&actor];
                let facing = self
                    .vm
                    .world
                    .objects
                    .get_mut(&actor)
                    .unwrap()
                    .data
                    .get_mut(&1)
                    .ok_or("native turn facing absent")?;
                let resident = self.characters.get_mut(&actor).unwrap();
                turn.poll = Some(movement::route::Poll::begin(
                    &resident.controls.movement,
                    &mut resident.character.placement,
                    facing,
                    [position.x, position.y],
                    position.level,
                )?);
                self.sync_script_directions()?;
            }
            while let Some(request) = turn.poll.as_ref().and_then(|p| p.request()) {
                if turn.physical.is_none() {
                    turn.physical = Some(Physical::new(self, actor, request)?);
                }
                match turn.physical.as_mut().unwrap() {
                    Physical::Accept(e) => {
                        if !self.drive_native_acceptance(program, e, remaining, step)? {
                            return Ok(Outcome::Budget);
                        }
                        let accepted = e
                            .outcome()
                            .ok_or("native turn acceptance unfinished")?
                            .accepted;
                        turn.poll.as_mut().unwrap().accept(
                            &mut self.characters.get_mut(&actor).unwrap().character.placement,
                            accepted,
                        )?;
                    }
                    Physical::Move { execution, .. } => {
                        if !self.drive_live_placement(program, execution, remaining, step)? {
                            return Ok(Outcome::Budget);
                        }
                        self.update_original_person_room(actor)?;
                        turn.poll.as_mut().unwrap().moved()?;
                    }
                }
                turn.physical = None;
            }
            let result = turn
                .poll
                .as_ref()
                .and_then(|p| p.result())
                .ok_or("native turn poll result absent")?;
            let finished = if result != 2 {
                let r = self.characters.get_mut(&actor).unwrap();
                r.controls
                    .movement
                    .stop_route(&mut r.character.placement, &mut r.controls.ordinary)?
            } else {
                false
            };
            if look::complete(result, finished) == 2 {
                // Unlike walking states 9/10/11, 45a98e repolls after yielding.
                turn.poll = None;
                turn.yielded_tick = Some(self.vm.tick);
                return Ok(Outcome::Waiting);
            }
            self.vm.finish_turn(program, thread, request.id, true)?;
            self.event(
                &mut step.events,
                thread,
                "native Look Towards completed".into(),
            );
            Ok(Outcome::Ready)
        })();
        if !matches!(outcome, Ok(Outcome::Ready)) {
            self.characters.get_mut(&actor).unwrap().native_turn = Some(turn);
        }
        outcome
    }
}
