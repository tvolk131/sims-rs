//! Household ownership of native character state and synchronous placement
//! refreshes. The caller must install an actually initialized character; this
//! adapter does not fabricate a rig or reconstruct it from legacy cursors.
use crate::{Simulation, head_look, lot_placement, pose};
use pose::{
    properties,
    refresh::{Character, Request, Response},
    transform,
};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program};

/// Retained animator controls needed by load/reset and ordinary updates. These
/// are supplied by actual initialization; there is deliberately no Default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Controls {
    pub ordinary: pose::ordinary::State,
    pub movement: pose::movement::Movement,
    pub settings: pose::animator::Settings,
    pub movement_anchors: [Option<usize>; 2],
    pub geometry_generation: u32,
    /// Saved +a4 and +a0, distinct from the timed-property event queue.
    pub animation_events: i32,
    pub engaged: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resident {
    pub character: Character,
    pub controls: Controls,
    /// Last completed load/ordinary-update tick. A forced refresh may run in
    /// next tick's ObjectUpdate before that tick's ordinary animator pass.
    /// Legacy Playback cannot stand in for missed native person updates.
    pub evaluated_tick: u64,
    /// Actual sound/view/appearance service inputs, provided by the host.
    pub context: properties::Context,
    /// Immutable source metadata for binding newly requested native layers.
    /// Older diagnostic characters can omit it until they execute commands.
    #[serde(default)]
    pub source: Option<sims_formats::avatar::Skeleton>,
    #[serde(default)]
    pub animation_yield: Option<(sims_vm::ThreadId, u64, u64)>,
    /// Native animator +d8. Completed initialization has already consumed its
    /// first profile reload; Reset sets this for the next skill lookup.
    #[serde(default)]
    pub profile_dirty: bool,
    #[serde(default)]
    pub skill_profile: Option<crate::character_profile::Pending>,
    #[serde(default)]
    pub ordinary_update: Option<crate::character_update::AnimatorUpdate>,
    #[serde(default)]
    pub person_update: Option<crate::character_update::PersonPass>,
    #[serde(default)]
    pub displayed_animation: Vec<u8>,
    #[serde(default)]
    pub native_walk: Option<crate::lot_route::animator::Walk>,
    #[serde(default)]
    pub native_turn: Option<crate::native_turn::Turn>,
    #[serde(default)]
    pub physical_movement: Option<crate::lot_route::animator::Physical>,
    pub(crate) refresh_tick: Option<u64>,
}
impl Resident {
    pub fn new(
        character: Character,
        evaluated_tick: u64,
        controls: Controls,
        context: properties::Context,
    ) -> Self {
        Self {
            character,
            evaluated_tick,
            controls,
            context,
            source: None,
            animation_yield: None,
            profile_dirty: false,
            skill_profile: None,
            ordinary_update: None,
            person_update: None,
            displayed_animation: vec![],
            native_walk: None,
            native_turn: None,
            physical_movement: None,
            refresh_tick: None,
        }
    }
    /// 589220's represented live state, shared by loading and deletion. This
    /// does not perform the separate appearance reload that follows Reset.
    pub fn reset(&mut self, now: i32) -> Result<Option<i32>, String> {
        if self.refresh_tick.is_some()
            || self.ordinary_update.is_some()
            || self.skill_profile.is_some()
        {
            return Err("native character reset during a pending refresh".into());
        }
        let mut next = self.clone();
        let effects = pose::reset::apply(
            &mut next.character,
            &mut next.controls.ordinary,
            &mut next.controls.movement,
            now,
            next.controls.settings,
        )?;
        next.controls.geometry_generation = next
            .controls
            .geometry_generation
            .wrapping_add(effects.geometry_generation_delta);
        if effects.clear_movement_anchors {
            next.controls.movement_anchors = [None; 2];
        }
        next.controls.engaged = 0;
        next.animation_yield = None;
        next.profile_dirty = true;
        *self = next;
        Ok(effects.route_diagnostic)
    }
}

impl Simulation {
    pub(crate) fn reset_character(&mut self, object: ObjectId) -> Result<(), String> {
        let state = self
            .characters
            .get_mut(&object)
            .ok_or("native character absent")?;
        let native = self
            .native_lot
            .as_mut()
            .ok_or("native character lot absent")?;
        let now = pose::ordinary::clock(self.vm.tick as i32, state.controls.ordinary.clock_mode);
        if let Some(error) = state.reset(now)? {
            native.diagnostics.error = error;
        }
        Ok(())
    }
    /// Required host work remains pending across budget yields and snapshots.
    /// In particular geometry, censor and dependent carried-object refreshes
    /// are not treated as completed simply because no window is being drawn.
    pub fn pending_character_services(&self) -> Vec<(ObjectId, Request)> {
        self.characters
            .iter()
            .filter_map(|(&id, state)| {
                if state.refresh_tick.is_none() && state.ordinary_update.is_none() {
                    return None;
                }
                Some((id, state.character.pending_request()?.clone()))
            })
            .collect()
    }

    /// Complete exactly the service observed by the host. A stale/incorrect
    /// reply does not change the character or the physical continuation.
    pub fn complete_character_service(
        &mut self,
        object: ObjectId,
        expected: &Request,
        response: Response,
    ) -> Result<(), String> {
        let state = self
            .characters
            .get_mut(&object)
            .ok_or("native character absent")?;
        if (state.refresh_tick != Some(self.vm.tick)
            && state
                .ordinary_update
                .as_ref()
                .is_none_or(|u| u.tick != self.vm.tick))
            || state.character.pending_request() != Some(expected)
        {
            return Err("native character service no longer pending".into());
        }
        // Placement and head targets use the actual household services below.
        if matches!(
            expected,
            Request::Placement | Request::HeadTarget { .. } | Request::Invalidate
        ) {
            return Err("native character service belongs to the physical host".into());
        }
        // These acknowledgments only assign visibility/pass fields. A mismatched
        // response is rejected before any write, and neither matching arm can
        // fail after a write. Keep the transaction copy for other services.
        if matches!(expected, Request::Viewport | Request::Visibility { .. }) {
            return state.character.acknowledge(response);
        }
        let mut character = state.character.clone();
        character.acknowledge(response)?;
        state.character = character;
        Ok(())
    }

    /// Complete 592132 using original terrain and current physical storage.
    /// The camera's floor vector remains an explicit profile input.
    pub fn place_character(
        &mut self,
        object: ObjectId,
        floor_vector: [f32; 3],
    ) -> Result<(), String> {
        let state = self
            .characters
            .get_mut(&object)
            .ok_or("native character absent")?;
        if (state.refresh_tick != Some(self.vm.tick)
            && state
                .ordinary_update
                .as_ref()
                .is_none_or(|u| u.tick != self.vm.tick))
            || state.character.pending_request() != Some(&Request::Placement)
        {
            return Err("native character placement is not pending".into());
        }
        let o = self
            .vm
            .world
            .objects
            .get(&object)
            .ok_or("native character object absent")?;
        let fields = o.person.as_ref().ok_or("native character person absent")?;
        let position = self
            .vm
            .world
            .positions
            .get(&object)
            .ok_or("native character position absent")?;
        let lot = self
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .ok_or("native character terrain absent")?;
        let mut character = state.character.clone();
        character.place_with_scale(
            transform::Source {
                position: [position.x, position.y],
                facing: *o.data.get(&1).ok_or("native character facing absent")?,
                level: position.level,
                flags: [
                    *fields.get(&84).ok_or("native character flag 84 absent")? as u8,
                    *fields.get(&68).ok_or("native character flag 68 absent")? as u8,
                ],
            },
            transform::View {
                rotation: self.vm.world.camera_rotation,
                floor_vector,
            },
            transform::Terrain {
                size: lot.size,
                corners: &lot.heights,
            },
            self.native_character_settings.and_then(|s| s.terrain_scale),
        )?;
        state.character = character;
        Ok(())
    }

    /// Snap's successful primitive tail calls animator +2c after the physical
    /// wrapper has reset cached XY/heading and the VM result word is cleared.
    pub(crate) fn apply_snap_character_offsets(
        &mut self,
        object: ObjectId,
        offsets: [u32; 3],
    ) -> Result<(), String> {
        let Some(state) = self.characters.get_mut(&object) else {
            return Ok(()); // Synthetic/legacy fixtures have no native animator.
        };
        let settings = self
            .native_character_settings
            .ok_or("Snap character view settings absent")?;
        let o = self
            .vm
            .world
            .objects
            .get(&object)
            .ok_or("Snap character absent")?;
        let fields = o
            .person
            .as_ref()
            .ok_or("Snap character person fields absent")?;
        let position = self
            .vm
            .world
            .positions
            .get(&object)
            .ok_or("Snap character position absent")?;
        let lot = self
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .ok_or("Snap character terrain absent")?;
        state.character.offset_placement_with_scale(
            offsets.map(f32::from_bits),
            transform::Source {
                position: [position.x, position.y],
                facing: *o.data.get(&1).ok_or("Snap character facing absent")?,
                level: position.level,
                flags: [
                    *fields.get(&84).ok_or("Snap character flag 84 absent")? as u8,
                    *fields.get(&68).ok_or("Snap character flag 68 absent")? as u8,
                ],
            },
            transform::View {
                rotation: self.vm.world.camera_rotation,
                floor_vector: settings.floor_vector,
            },
            transform::Terrain {
                size: lot.size,
                corners: &lot.heights,
            },
            self.native_character_settings.and_then(|s| s.terrain_scale),
        )?;
        Ok(())
    }

    pub(crate) fn character_head_target(
        &self,
        target: ObjectId,
    ) -> Result<Option<head_look::Target>, String> {
        let Some(o) = self.vm.world.objects.get(&target) else {
            return Ok(None);
        };
        let p = self
            .vm
            .world
            .positions
            .get(&target)
            .ok_or("native head target position absent")?;
        let point = if o.person.is_some() {
            let state = self
                .characters
                .get(&target)
                .ok_or("native head target person animator has not been activated")?;
            state
                .character
                .rig
                .head
                .map(|anchor| {
                    state
                        .character
                        .hierarchy
                        .absolute
                        .get(anchor.bone)
                        .ok_or("native head target bone absent")?
                        .anchor_point(anchor.kind, state.character.matrix)
                })
                .transpose()?
        } else {
            Some(head_look::ordinary_target_point([p.x, p.y]))
        };
        Ok(Some(head_look::Target {
            level: p.level,
            point,
        }))
    }

    /// Called with the physical operation still suspended at RefreshAnimator.
    /// Returning false retains that operation; removal/adjacency callbacks do
    /// not run again when the character's next service completes.
    pub(crate) fn drive_detached_character(
        &mut self,
        program: &Program,
        object: ObjectId,
        remaining: &mut usize,
    ) -> Result<bool, String> {
        self.drive_character_refresh(program, object, remaining, true)
    }
    /// Explicit viewer+54(0) call. The application camera owner must remain
    /// suspended through asset services; this runs no ordinary person tick.
    pub fn refresh_view_person(
        &mut self,
        program: &Program,
        object: ObjectId,
        mut budget: usize,
    ) -> Result<bool, String> {
        if self.selection_work_pending() {
            return Err("view refresh overlaps selection".into());
        }
        self.drive_character_refresh(program, object, &mut budget, false)
    }
    pub fn refresh_selection_notification_view_person(
        &mut self,
        program: &Program,
        object: ObjectId,
        token: crate::lot_selection::NotificationViewRefresh,
        mut budget: usize,
    ) -> Result<bool, String> {
        self.validate_selection_notification_view_refresh(token)?;
        let execution = self.native_selection.as_ref().unwrap();
        if execution
            .notification_view_person
            .is_some_and(|pending| pending != object)
        {
            return Err("selection notification view person differs".into());
        }
        if !self.characters.contains_key(&object) {
            return Err("native character absent".into());
        }
        if budget == 0 {
            return Ok(false);
        }
        self.native_selection.as_mut().unwrap().notification_view_person = Some(object);
        let complete = self.drive_character_refresh(program, object, &mut budget, false)?;
        if complete {
            self.native_selection.as_mut().unwrap().notification_view_person = None;
        }
        Ok(complete)
    }
    pub(crate) fn drive_character_refresh(
        &mut self,
        program: &Program,
        object: ObjectId,
        remaining: &mut usize,
        only_visible: bool,
    ) -> Result<bool, String> {
        if *remaining == 0 {
            return Ok(false);
        }
        let state = self
            .characters
            .get_mut(&object)
            .ok_or("native character absent")?;
        if state.ordinary_update.is_some() {
            return Err("placement refresh during a pending ordinary character update".into());
        }
        if state.evaluated_tick > self.vm.tick
            || (self.native_scheduler.is_none()
                && state.evaluated_tick.saturating_add(1) < self.vm.tick)
        {
            return Err("native ordinary person update required before placement refresh".into());
        }
        if state.refresh_tick.is_none() {
            state.character.begin(
                pose::ordinary::clock(self.vm.tick as i32, state.controls.ordinary.clock_mode),
                only_visible,
                state.character.visible,
                true,
            )?;
            state.refresh_tick = Some(self.vm.tick);
        }
        loop {
            if *remaining == 0 {
                return Ok(false);
            }
            let state = self.characters.get_mut(&object).unwrap();
            let fields = self
                .vm
                .world
                .objects
                .get_mut(&object)
                .and_then(|o| o.person.as_mut())
                .ok_or("native character person absent")?;
            let level = self
                .vm
                .world
                .positions
                .get(&object)
                .ok_or("native character position absent")?
                .level;
            let request = state
                .character
                .next(program, fields, state.context, level)?;
            let Some(request) = request else {
                state.refresh_tick = None;
                self.native_lot
                    .as_mut()
                    .ok_or("native character lot absent")?
                    .observers
                    .push(lot_placement::Request::RefreshAnimator(object));
                return Ok(true);
            };
            match request {
                Request::Placement if self.native_character_settings.is_some() => {
                    self.place_character(
                        object,
                        self.native_character_settings.unwrap().floor_vector,
                    )?;
                }
                Request::Invalidate => {
                    self.native_lot
                        .as_mut()
                        .ok_or("native character lot absent")?
                        .observers
                        .push(lot_placement::Request::Invalidate {
                            object,
                            children_only: false,
                        });
                    self.characters
                        .get_mut(&object)
                        .unwrap()
                        .character
                        .acknowledge(Response::Done)?;
                }
                Request::HeadTarget { object: target } => {
                    let target = self.character_head_target(target)?;
                    self.characters
                        .get_mut(&object)
                        .unwrap()
                        .character
                        .acknowledge(Response::HeadTarget(target))?;
                }
                Request::CarriedObject
                    if self.vm.world.objects[&object]
                        .slots
                        .get(2)
                        .is_none_or(Option::is_none) =>
                {
                    self.characters
                        .get_mut(&object)
                        .unwrap()
                        .character
                        .acknowledge(Response::Done)?;
                }
                _ => return Ok(false),
            }
            *remaining -= 1;
        }
    }

    pub(crate) fn validate_characters(&self, program: &Program) -> Result<(), String> {
        self.validate_skill_profiles()?;
        for (&id, state) in &self.characters {
            if self.animations.contains_key(&id)
                || self.backgrounds.contains_key(&id)
                || self.carry.contains_key(&id)
                || self.turns.contains_key(&id)
            {
                return Err("native character also owns legacy animation state".into());
            }
            if self.native_lot.is_none()
                || self
                    .vm
                    .world
                    .objects
                    .get(&id)
                    .is_none_or(|o| o.person.is_none())
                || state.context.actor != id
                || !state.context.position_x.is_finite()
                || !state.controls.ordinary.rate.is_finite()
                || !state.controls.ordinary.target_rate.is_finite()
                || (state.controls.movement.busy() && state.ordinary_update.is_none())
                || state
                    .controls
                    .movement_anchors
                    .iter()
                    .flatten()
                    .any(|&b| b >= state.character.skin.bones.len())
                || state.evaluated_tick > self.vm.tick
                || state.refresh_tick.is_some_and(|tick| {
                    tick != self.vm.tick || (self.native_scheduler.is_none() && state.evaluated_tick.saturating_add(1) < tick)
                })
                || state.refresh_tick.is_none()
                    && state.ordinary_update.is_none()
                    && state.character.busy()
                || state
                    .ordinary_update
                    .as_ref()
                    .is_some_and(|u| u.tick != self.vm.tick)
                || state.person_update.as_ref().is_some_and(|p| {
                    p.execution.tick() != if p.constructor { 0 } else { self.vm.tick as i32 }
                        || p.notification_started && !matches!(p.execution.pending_request(),
                            Some(crate::person_update::Request::NotifyPerson { .. } | crate::person_update::Request::NotifyAnimationName { .. }))
                        || self.native_character_settings.is_none()
                        || self.vm.object_threads.get(&id) != Some(&p.thread)
                        || if p.constructor {
                            self.person_factories.get(&id).is_none_or(|f| {
                                f.factory.pending_request() != Some(sims_vm::person_factory::registered::Request::Reset(
                                    sims_vm::person_factory::ConstructionResetRequest::ImmediatePersonUpdate { object: id, tick: 0 }))
                            })
                        } else { self.visit != Some((p.thread, Some(id))) }
                })
            {
                return Err("invalid household native character binding".into());
            }
            state.character.validate(program)?;
            if let Some(walk) = &state.native_walk {
                walk.validate(self, program, id)?;
            }
            if let Some(turn) = &state.native_turn {
                turn.validate(self, program, id)?;
            }
            if let Some(physical) = &state.physical_movement {
                physical.validate(self, program, id)?;
            }
        }
        Ok(())
    }
}
