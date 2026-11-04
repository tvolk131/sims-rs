//! Live ordinary animator continuation. The resident retains one copy of its
//! controls; suspended prefix control flow carries no stale mode/clock words.
use crate::{
    character::Resident,
    pose::{movement, ordinary, refresh},
};
use serde::{Deserialize, Serialize};
use sims_vm::{Object, Program};

/// Live world/view configuration, supplied by the host rather than inferred
/// from a save's character fields. Native movement constants are explicit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub movement: movement::Settings,
    pub root_motion: u8,
    pub person: crate::person_update::Context,
    pub floor_vector: [f32; 3],
    /// Observed initialized 703f2c process cache, not recomputed under a new FPU context.
    #[serde(default)]
    pub terrain_scale: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonPass {
    pub thread: sims_vm::ThreadId,
    pub execution: crate::person_update::Execution,
    /// Reset(1) invokes Person::Update(0) synchronously inside its parent
    /// factory. Animator clock queries still read the live lot clock.
    #[serde(default)]
    pub constructor: bool,
    /// The presentation host owns a saved notification continuation.
    #[serde(default)]
    pub notification_started: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Phase {
    Prefix(ordinary::Checkpoint),
    Movement,
    Path {
        now: i32,
        step: u32,
        entry: ordinary::Entry,
    },
    Tail,
    Done,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimatorUpdate {
    pub tick: u64,
    pub entry: ordinary::Entry,
    only_visible: bool,
    phase: Phase,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnimatorRequest {
    Prefix(ordinary::Request),
    Movement(movement::Request),
    /// The path must be installed with all native progress/calibration words.
    /// An empty or reconstructed presentation route is not a substitute.
    Path,
    Tail(refresh::Request),
}
fn field(object: &Object, key: i16) -> Result<i16, String> {
    object
        .person
        .as_ref()
        .and_then(|p| p.get(&key))
        .copied()
        .ok_or_else(|| format!("ordinary animator person field {key} absent"))
}
fn data(object: &Object, key: i16) -> Result<i16, String> {
    object
        .data
        .get(&key)
        .copied()
        .ok_or_else(|| format!("ordinary animator object field {key} absent"))
}
impl AnimatorUpdate {
    fn tail(
        &mut self,
        resident: &mut Resident,
        now: i32,
        entry: ordinary::Entry,
    ) -> Result<(), String> {
        // Native animator +38 is the unscaled placement input. The skeleton's
        // published scale*placement matrix must not become the next input.
        resident.character.begin_ordinary_tail(
            now,
            self.only_visible,
            true,
            resident.character.model.placement,
        )?;
        self.entry = entry;
        self.phase = Phase::Tail;
        Ok(())
    }
    fn next(
        &mut self,
        resident: &mut Resident,
        object: &mut Object,
        program: &Program,
        level: i32,
        definition_flags: u8,
        settings: movement::Settings,
    ) -> Result<Option<AnimatorRequest>, String> {
        loop {
            match &self.phase {
                Phase::Prefix(checkpoint) => {
                    let mut prefix = ordinary::Prefix::resume(
                        resident.controls.ordinary.clone(),
                        checkpoint.clone(),
                    );
                    let result = prefix.next(
                        &mut resident.character,
                        ordinary::Source {
                            posture: field(object, 0)?,
                            species: field(object, 65)?,
                            definition_flags,
                            walk_style: data(object, 17)?,
                        },
                        resident.controls.settings,
                        settings.rate,
                    );
                    if result.as_ref().is_ok_and(Option::is_none) {
                        let continuation = prefix.finish()?;
                        resident.controls.ordinary = prefix.state;
                        match continuation {
                            ordinary::Continuation::Tail => {
                                self.tail(resident, resident.character.clock, self.entry)?
                            }
                            movement @ ordinary::Continuation::Movement { .. } => {
                                resident
                                    .controls
                                    .movement
                                    .begin(self.tick as i32, movement)?;
                                self.phase = Phase::Movement;
                            }
                        }
                        continue;
                    }
                    let (state, checkpoint) = prefix.suspend()?;
                    resident.controls.ordinary = state;
                    self.phase = Phase::Prefix(checkpoint);
                    return result.map(|r| r.map(AnimatorRequest::Prefix));
                }
                Phase::Movement => {
                    let source = movement::Source {
                        species: field(object, 65)?,
                        person78: field(object, 78)?,
                        walk_style: data(object, 17)?,
                        object9: data(object, 9)?,
                        level,
                    };
                    let facing = object
                        .data
                        .get_mut(&1)
                        .ok_or("ordinary animator facing absent")?;
                    if let Some(request) = resident.controls.movement.next(
                        &mut resident.character.skin.animator,
                        &mut resident.character.placement,
                        &mut resident.controls.ordinary,
                        facing,
                        source,
                        settings,
                        resident.controls.settings,
                        program,
                    )? {
                        return Ok(Some(AnimatorRequest::Movement(request)));
                    }
                    match resident.controls.movement.finish()? {
                        movement::Continuation::Tail { now, entry } => {
                            self.tail(resident, now, entry)?
                        }
                        movement::Continuation::Path { now, step, entry } => {
                            self.phase = Phase::Path { now, step, entry }
                        }
                    }
                }
                Phase::Path { .. } => return Ok(Some(AnimatorRequest::Path)),
                Phase::Tail => {
                    if let Some(request) = resident.character.next(
                        program,
                        object
                            .person
                            .as_mut()
                            .ok_or("ordinary animator person absent")?,
                        resident.context,
                        level,
                    )? {
                        return Ok(Some(AnimatorRequest::Tail(request)));
                    }
                    self.phase = Phase::Done;
                }
                Phase::Done => return Ok(None),
            }
        }
    }
    pub fn pending_request(&self, resident: &Resident) -> Option<AnimatorRequest> {
        match &self.phase {
            Phase::Prefix(checkpoint) => checkpoint
                .pending_request()
                .cloned()
                .map(AnimatorRequest::Prefix),
            Phase::Movement => resident
                .controls
                .movement
                .pending_request()
                .map(AnimatorRequest::Movement),
            Phase::Path { .. } => Some(AnimatorRequest::Path),
            Phase::Tail => resident
                .character
                .pending_request()
                .cloned()
                .map(AnimatorRequest::Tail),
            Phase::Done => None,
        }
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Done
    }
}

impl Resident {
    pub fn begin_ordinary_update(
        &mut self,
        object: &Object,
        tick: u64,
        only_visible: bool,
        root_motion: u8,
    ) -> Result<(), String> {
        self.begin_ordinary_visit(object, tick, only_visible, root_motion, false)
    }
    pub(crate) fn begin_ordinary_visit(
        &mut self,
        object: &Object,
        tick: u64,
        only_visible: bool,
        root_motion: u8,
        registry_scheduled: bool,
    ) -> Result<(), String> {
        if self.ordinary_update.is_some()
            || self.refresh_tick.is_some()
            || self.character.busy()
            || self.controls.movement.busy()
        {
            return Err("native ordinary update overlaps an unfinished character service".into());
        }
        if self.evaluated_tick > tick
            || (!registry_scheduled && self.evaluated_tick.saturating_add(1) < tick)
        {
            return Err("native ordinary animator clock skipped a tick".into());
        }
        let entry = ordinary::Entry {
            root_motion,
            movement_input: field(object, 64)?,
            walk_style: data(object, 17)?,
        };
        let mut prefix = ordinary::Prefix::new(self.controls.ordinary.clone());
        prefix.begin(&mut self.character, tick as i32, entry)?;
        let (state, checkpoint) = prefix.suspend()?;
        self.controls.ordinary = state;
        self.ordinary_update = Some(AnimatorUpdate {
            tick,
            entry,
            only_visible,
            phase: Phase::Prefix(checkpoint),
        });
        Ok(())
    }
    pub fn poll_ordinary_update(
        &mut self,
        object: &mut Object,
        program: &Program,
        level: i32,
        definition_flags: u8,
        settings: movement::Settings,
    ) -> Result<Option<AnimatorRequest>, String> {
        let mut update = self
            .ordinary_update
            .take()
            .ok_or("native ordinary update absent")?;
        let result = update.next(self, object, program, level, definition_flags, settings);
        self.ordinary_update = Some(update);
        result
    }
    pub fn complete_ordinary_prefix(
        &mut self,
        program: &Program,
        expected: &ordinary::Request,
        response: ordinary::Response,
    ) -> Result<(), String> {
        let update = self
            .ordinary_update
            .as_ref()
            .ok_or("native ordinary update absent")?;
        let Phase::Prefix(checkpoint) = &update.phase else {
            return Err("native ordinary prefix is not pending".into());
        };
        if checkpoint.pending_request() != Some(expected) {
            return Err("stale native ordinary prefix response".into());
        }
        let source = self
            .source
            .as_ref()
            .ok_or("native ordinary source skeleton absent")?;
        let mut prefix =
            ordinary::Prefix::resume(self.controls.ordinary.clone(), checkpoint.clone());
        // A malformed/unloaded asset reply must preserve the continuation.
        let mut character = self.character.clone();
        prefix.acknowledge(
            &mut character,
            program,
            source,
            self.controls.settings,
            response,
        )?;
        let (state, checkpoint) = prefix.suspend()?;
        self.character = character;
        self.controls.ordinary = state;
        self.ordinary_update.as_mut().unwrap().phase = Phase::Prefix(checkpoint);
        Ok(())
    }
    pub fn install_ordinary_path(&mut self, path: movement::path::Path) -> Result<(), String> {
        let update = self
            .ordinary_update
            .as_ref()
            .ok_or("native ordinary update absent")?;
        let Phase::Path { now, step, entry } = update.phase else {
            return Err("native path installation is not pending".into());
        };
        self.controls.movement.begin(
            update.tick as i32,
            ordinary::Continuation::Movement { now, step, entry },
        )?;
        self.controls.movement.path = Some(path);
        self.ordinary_update.as_mut().unwrap().phase = Phase::Movement;
        Ok(())
    }
    pub fn finish_ordinary_update(&mut self) -> Result<(), String> {
        let update = self
            .ordinary_update
            .as_ref()
            .ok_or("native ordinary update absent")?;
        if !update.complete() || self.character.busy() || self.controls.movement.busy() {
            return Err("native ordinary update is unfinished".into());
        }
        self.evaluated_tick = update.tick;
        self.ordinary_update = None;
        Ok(())
    }
}
