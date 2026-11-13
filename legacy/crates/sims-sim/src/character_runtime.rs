//! Household services for native person and ordinary animator continuations.
//! Unimplemented world/presentation services stay pending and prevent ticks
//! from advancing. Hosts complete them against the original world/resources.
use crate::{
    Simulation,
    character_update::{AnimatorRequest, PersonPass},
    person_update,
    pose::{movement, ordinary, posture, refresh},
};
use sims_vm::{ObjectId, Program, ThreadId};

#[cfg(test)]
#[path = "character_runtime_tests.rs"]
mod tests;

impl Simulation {
    pub fn pending_character_updates(&self) -> Vec<(ObjectId, AnimatorRequest)> {
        self.characters
            .iter()
            .filter_map(|(&id, r)| Some((id, r.ordinary_update.as_ref()?.pending_request(r)?)))
            .collect()
    }
    pub fn pending_person_services(&self) -> Vec<(ObjectId, person_update::Request)> {
        self.characters
            .iter()
            .filter_map(|(&id, r)| {
                Some((
                    id,
                    r.person_update
                        .as_ref()?
                        .execution
                        .pending_request()?
                        .clone(),
                ))
            })
            .collect()
    }
    fn native_person_state(&self, object: ObjectId) -> Result<person_update::State, String> {
        Ok(person_update::State {
            room: *self
                .native_lot
                .as_ref()
                .and_then(|n| n.membership.0.get(&object))
                .ok_or("native person room cache absent")?,
            displayed_animation: self
                .characters
                .get(&object)
                .ok_or("native person animator absent")?
                .displayed_animation
                .clone(),
        })
    }
    pub fn complete_person_service(
        &mut self,
        object: ObjectId,
        expected: &person_update::Request,
        response: person_update::Response,
    ) -> Result<(), String> {
        if matches!(
            expected,
            person_update::Request::ObjectUpdate { .. }
                | person_update::Request::Animator { .. }
                | person_update::Request::Decay
                | person_update::Request::Scale { .. }
                | person_update::Request::RoomMembership
                | person_update::Request::HasActionUse
                | person_update::Request::AnimationName
                | person_update::Request::NotifyPerson { .. }
                | person_update::Request::NotifyAnimationName { .. }
        ) {
            return Err("person service belongs to the simulation host".into());
        }
        self.acknowledge_person(object, expected, response)
    }
    pub fn begin_person_notification(
        &mut self,
        object: ObjectId,
        expected: &person_update::Request,
    ) -> Result<i32, String> {
        if !matches!(
            expected,
            person_update::Request::NotifyPerson { .. }
                | person_update::Request::NotifyAnimationName { .. }
        ) {
            return Err("not a person notification".into());
        }
        let pass = self
            .characters
            .get_mut(&object)
            .and_then(|r| r.person_update.as_mut())
            .ok_or("person notification owner absent")?;
        if pass.notification_started
            || pass.execution.pending_request() != Some(expected)
            || pass.execution.tick()
                != if pass.constructor {
                    0
                } else {
                    self.vm.tick as i32
                }
        {
            return Err("stale or already owned person notification".into());
        }
        pass.notification_started = true;
        Ok(pass.execution.tick())
    }
    pub fn complete_person_notification(
        &mut self,
        object: ObjectId,
        expected: &person_update::Request,
    ) -> Result<(), String> {
        if !matches!(
            expected,
            person_update::Request::NotifyPerson { .. }
                | person_update::Request::NotifyAnimationName { .. }
        ) || !self
            .characters
            .get(&object)
            .and_then(|r| r.person_update.as_ref())
            .is_some_and(|p| p.notification_started)
        {
            return Err("person notification continuation absent".into());
        }
        self.acknowledge_person(object, expected, person_update::Response::Done)?;
        self.characters
            .get_mut(&object)
            .unwrap()
            .person_update
            .as_mut()
            .unwrap()
            .notification_started = false;
        Ok(())
    }
    fn acknowledge_person(
        &mut self,
        object: ObjectId,
        expected: &person_update::Request,
        response: person_update::Response,
    ) -> Result<(), String> {
        let mut state = self.native_person_state(object)?;
        let resident = self.characters.get_mut(&object).unwrap();
        let pass = resident
            .person_update
            .as_mut()
            .ok_or("native person update absent")?;
        if pass.execution.tick()
            != if pass.constructor {
                0
            } else {
                self.vm.tick as i32
            }
            || pass.execution.pending_request() != Some(expected)
        {
            return Err("stale native person update response".into());
        }
        pass.execution.acknowledge(
            self.vm
                .world
                .objects
                .get_mut(&object)
                .ok_or("native person object absent")?,
            &mut state,
            response,
        )?;
        resident.displayed_animation = state.displayed_animation;
        Ok(())
    }
    pub(crate) fn native_update_asset_names(
        &self,
        program: &Program,
    ) -> Result<std::collections::BTreeSet<String>, String> {
        let mut names = std::collections::BTreeSet::new();
        for (id, request) in self.pending_character_updates() {
            if self.characters[&id].profile_dirty || self.characters[&id].skill_profile.is_some() {
                continue;
            }
            let index = match request {
                AnimatorRequest::Prefix(ordinary::Request::Animation { index, .. }) => Some(index),
                AnimatorRequest::Movement(
                    movement::Request::Foreground { index }
                    | movement::Request::StopForeground { index }
                    | movement::Request::RootDisplacement { index },
                ) => Some(index as u32),
                AnimatorRequest::Movement(movement::Request::BasePosture) => {
                    let resident = &self.characters[&id];
                    posture::selection(
                        &resident.character.skin.animator,
                        &resident.controls.ordinary,
                        self.base_posture_input(program, id)?,
                    )
                }
                _ => None,
            };
            if let Some(index) = index {
                if let Some(name) = self.native_posture_name(program, id, index)? {
                    if !program.animations.contains_key(&name) {
                        names.insert(name);
                    }
                }
            }
        }
        Ok(names)
    }
    fn base_posture_input(
        &self,
        program: &Program,
        object: ObjectId,
    ) -> Result<posture::Input, String> {
        let person = self.vm.world.objects[&object]
            .person
            .as_ref()
            .ok_or("base posture person absent")?;
        Ok(posture::Input {
            posture: *person.get(&0).ok_or("base posture person field 0 absent")?,
            species: *person
                .get(&65)
                .ok_or("base posture person field 65 absent")?,
            definition_flags: self.definition_flags(program, object)?,
            tick: self.characters[&object]
                .ordinary_update
                .as_ref()
                .ok_or("base posture ordinary update absent")?
                .tick as i32,
        })
    }
    fn definition_flags(&self, program: &Program, object: ObjectId) -> Result<u8, String> {
        let d = self
            .vm
            .world
            .objects
            .get(&object)
            .and_then(|o| o.definition.as_ref())
            .ok_or("native person definition absent")?;
        Ok(*program
            .modules
            .get(&d.source)
            .and_then(|m| m.object_definition_words.get(&d.object))
            .and_then(|w| w.get(0xcc / 2))
            .ok_or("native person definition flags absent")? as u8)
    }
    pub(crate) fn drive_character_ordinary(
        &mut self,
        program: &Program,
        object: ObjectId,
        only_visible: bool,
        remaining: &mut usize,
        step: &mut crate::Step,
    ) -> Result<bool, String> {
        let settings = self
            .native_character_settings
            .ok_or("native live character settings absent")?;
        if self.characters[&object].ordinary_update.is_none() {
            self.characters
                .get_mut(&object)
                .unwrap()
                .begin_ordinary_visit(
                    &self.vm.world.objects[&object],
                    self.vm.tick,
                    only_visible,
                    settings.root_motion,
                    self.native_scheduler.is_some(),
                )?;
        }
        loop {
            if *remaining == 0 {
                return Ok(false);
            }
            if self.characters[&object].skill_profile.is_some() {
                return Ok(false);
            }
            let flags = self.definition_flags(program, object)?;
            let level = self
                .vm
                .world
                .positions
                .get(&object)
                .ok_or("native person physical position absent")?
                .level;
            let request = self
                .characters
                .get_mut(&object)
                .unwrap()
                .poll_ordinary_update(
                    self.vm.world.objects.get_mut(&object).unwrap(),
                    program,
                    level,
                    flags,
                    settings.movement,
                )?;
            self.sync_script_directions()?;
            let Some(request) = request else {
                self.characters
                    .get_mut(&object)
                    .unwrap()
                    .finish_ordinary_update()?;
                return Ok(true);
            };
            if self.characters[&object].profile_dirty {
                let lookup = match &request {
                    AnimatorRequest::Prefix(ordinary::Request::Animation { .. })
                    | AnimatorRequest::Movement(
                        movement::Request::Foreground { .. }
                        | movement::Request::StopForeground { .. }
                        | movement::Request::RootDisplacement { .. },
                    ) => true,
                    AnimatorRequest::Movement(movement::Request::BasePosture) => {
                        let r = &self.characters[&object];
                        posture::selection(
                            &r.character.skin.animator,
                            &r.controls.ordinary,
                            self.base_posture_input(program, object)?,
                        )
                        .is_some()
                    }
                    _ => false,
                };
                if lookup {
                    self.defer_skill_profile(
                        object,
                        crate::character_profile::Origin::Ordinary(request),
                    )?;
                    return Ok(false);
                }
            }
            match request {
                AnimatorRequest::Movement(
                    request @ (movement::Request::AcceptStep { .. }
                    | movement::Request::MoveStep { .. }),
                ) => {
                    if !self
                        .drive_character_physical_step(program, object, request, remaining, step)?
                    {
                        return Ok(false);
                    }
                }
                AnimatorRequest::Prefix(expected) => {
                    let response = match &expected {
                        ordinary::Request::CarriedSlot => ordinary::Response::CarriedSlot(
                            self.vm.world.objects[&object]
                                .slots
                                // 589fce reads physical slot 1. Object::slots
                                // excludes the physical slot-zero sentinel.
                                .first()
                                .ok_or("native automatic carry slot absent")?
                                .is_some(),
                        ),
                        ordinary::Request::OutsideLot => {
                            let p = self.vm.world.positions[&object];
                            ordinary::Response::OutsideLot(
                                !self
                                    .native_lot
                                    .as_ref()
                                    .ok_or("native character lot absent")?
                                    .placement
                                    .links
                                    .on_lot([p.x, p.y]),
                            )
                        }
                        ordinary::Request::Animation { index, .. } => {
                            let name = self.native_posture_name(program, object, *index)?;
                            if name
                                .as_ref()
                                .is_some_and(|n| !program.animations.contains_key(n))
                            {
                                return Err("native ordinary animation asset needs loading".into());
                            }
                            ordinary::Response::Animation { name, ready: true }
                        }
                    };
                    self.characters
                        .get_mut(&object)
                        .unwrap()
                        .complete_ordinary_prefix(program, &expected, response)?;
                }
                AnimatorRequest::Movement(
                    request @ (movement::Request::Foreground { index }
                    | movement::Request::StopForeground { index }),
                ) => {
                    let name = self.native_posture_name(program, object, index as u32)?;
                    if name
                        .as_ref()
                        .is_some_and(|n| !program.animations.contains_key(n))
                    {
                        return Err("native movement animation asset needs loading".into());
                    }
                    let resident = self.characters.get_mut(&object).unwrap();
                    // StopSelect has its own native continuation. Ordinary
                    // selection cannot acknowledge it, even for the same clip.
                    let select = if matches!(request, movement::Request::StopForeground { .. }) {
                        movement::Movement::stop_selected
                    } else {
                        movement::Movement::selected
                    };
                    select(
                        &mut resident.controls.movement,
                        &mut resident.character.skin.animator,
                        &resident.controls.ordinary,
                        resident
                            .source
                            .as_ref()
                            .ok_or("native movement skeleton absent")?,
                        name.as_ref().map(|n| &program.animations[n].clip),
                        resident.controls.settings,
                    )?;
                }
                AnimatorRequest::Movement(movement::Request::Placement) => {
                    let position = self.vm.world.positions[&object];
                    let o = &self.vm.world.objects[&object];
                    let fields = o.person.as_ref().ok_or("moving person fields absent")?;
                    let lot = self
                        .architecture
                        .as_ref()
                        .and_then(|a| a.original.as_ref())
                        .ok_or("moving person terrain absent")?;
                    let resident = self.characters.get_mut(&object).unwrap();
                    resident.controls.movement.place_with_scale(
                        &mut resident.character,
                        crate::pose::transform::Source {
                            position: [position.x, position.y],
                            facing: o.data[&1],
                            level: position.level,
                            flags: [fields[&84] as u8, fields[&68] as u8],
                        },
                        crate::pose::transform::View {
                            rotation: self.vm.world.camera_rotation,
                            floor_vector: settings.floor_vector,
                        },
                        crate::pose::transform::Terrain {
                            size: lot.size,
                            corners: &lot.heights,
                        },
                        settings.terrain_scale,
                    )?;
                }
                AnimatorRequest::Movement(movement::Request::BasePosture) => {
                    let input = self.base_posture_input(program, object)?;
                    let resident = &self.characters[&object];
                    let skeleton = resident
                        .source
                        .as_ref()
                        .ok_or("native base posture skeleton absent")?;
                    // A missing decoded clip leaves the entire native service
                    // pending. Stage its clear/cache/start and movement settle
                    // together so retrying cannot fade the same layer twice.
                    let mut animator = resident.character.skin.animator.clone();
                    let mut controls = resident.controls.clone();
                    posture::update(
                        &mut animator,
                        &mut controls.ordinary,
                        input,
                        skeleton,
                        controls.settings,
                        |index| {
                            self.native_posture_name(program, object, index)?
                                .map(|name| {
                                    program
                                        .animations
                                        .get(&name)
                                        .map(|asset| &asset.clip)
                                        .ok_or_else(|| {
                                            "native base posture animation asset needs loading"
                                                .to_owned()
                                        })
                                })
                                .transpose()
                        },
                    )?;
                    controls
                        .movement
                        .serviced(movement::Request::BasePosture, &mut animator)?;
                    let resident = self.characters.get_mut(&object).unwrap();
                    resident.character.skin.animator = animator;
                    resident.controls = controls;
                }
                AnimatorRequest::Tail(refresh::Request::Placement) => {
                    self.place_character(object, settings.floor_vector)?
                }
                AnimatorRequest::Tail(refresh::Request::Invalidate) => {
                    self.native_lot.as_mut().unwrap().observers.push(
                        crate::lot_placement::Request::Invalidate {
                            object,
                            children_only: false,
                        },
                    );
                    self.characters
                        .get_mut(&object)
                        .unwrap()
                        .character
                        .acknowledge(refresh::Response::Done)?;
                }
                AnimatorRequest::Tail(refresh::Request::HeadTarget { object: target }) => {
                    let target = self.character_head_target(target)?;
                    self.characters
                        .get_mut(&object)
                        .unwrap()
                        .character
                        .acknowledge(refresh::Response::HeadTarget(target))?;
                }
                AnimatorRequest::Tail(refresh::Request::RootMotionCorrection)
                    if self.characters[&object]
                        .ordinary_update
                        .as_ref()
                        .unwrap()
                        .entry
                        .root_motion
                        != 1 =>
                {
                    // The original first gate bypasses all remaining world reads.
                    self.characters
                        .get_mut(&object)
                        .unwrap()
                        .character
                        .correct_root(crate::pose::root_motion::Source {
                            enabled: 0,
                            movement_status: 0,
                            excluded_person: false,
                            world_word_30: 0,
                            movement_mode: 0,
                        })?;
                }
                AnimatorRequest::Tail(refresh::Request::OrdinaryCarriedObject)
                    if self.vm.world.objects[&object]
                        .slots
                        .get(1)
                        .is_none_or(Option::is_none) =>
                {
                    self.characters
                        .get_mut(&object)
                        .unwrap()
                        .character
                        .acknowledge(refresh::Response::Done)?;
                }
                // Actual geometry, appearance, sound, dependent objects, path
                // installation and physical callbacks require their host.
                _ => return Ok(false),
            }
            // Physical callbacks share this budget and can consume its last
            // instruction while completing the current animator service.
            *remaining = remaining.saturating_sub(1);
        }
    }
    pub(crate) fn drive_native_person_update(
        &mut self,
        program: &Program,
        thread: ThreadId,
        remaining: &mut usize,
        step: &mut crate::Step,
    ) -> Result<bool, String> {
        let Some((&object, _)) = self.vm.object_threads.iter().find(|(_, id)| **id == thread)
        else {
            return Ok(true);
        };
        if !self.characters.contains_key(&object) {
            return Ok(true);
        }
        let Some(update) = self.household.as_ref().and_then(|h| h.people.get(&object)) else {
            return Ok(true);
        };
        if update.started != Some(self.vm.tick) {
            return Ok(true);
        }
        if self.characters[&object].person_update.is_none() {
            if self.vm.world.objects[&object].last_update != Some(self.vm.tick) {
                return Err("native person tail precedes its object update".into());
            }
            self.characters.get_mut(&object).unwrap().person_update = Some(PersonPass {
                thread,
                execution: person_update::Execution::new(self.vm.tick as i32),
                constructor: false,
                notification_started: false,
            });
        }
        self.drive_native_person_tail(program, object, thread, remaining, step)
    }

    pub(crate) fn drive_native_person_tail(
        &mut self,
        program: &Program,
        object: ObjectId,
        thread: ThreadId,
        remaining: &mut usize,
        step: &mut crate::Step,
    ) -> Result<bool, String> {
        let settings = self
            .native_character_settings
            .ok_or("native live character settings absent")?;
        if self.characters[&object]
            .person_update
            .as_ref()
            .is_none_or(|p| p.thread != thread)
        {
            return Err("native person tail owner mismatch".into());
        }
        loop {
            if *remaining == 0 {
                return Ok(false);
            }
            let mut state = self.native_person_state(object)?;
            let request = {
                let resident = self.characters.get_mut(&object).unwrap();
                resident.person_update.as_mut().unwrap().execution.next(
                    self.vm.world.objects.get_mut(&object).unwrap(),
                    &mut state,
                    settings.person,
                )?
            };
            let Some(request) = request else {
                self.characters.get_mut(&object).unwrap().person_update = None;
                let update = self
                    .household
                    .as_mut()
                    .unwrap()
                    .people
                    .get_mut(&object)
                    .unwrap();
                update.started = None;
                update.finished = Some(self.vm.tick);
                return Ok(true);
            };
            match request {
                person_update::Request::ObjectUpdate { .. } => self.acknowledge_person(
                    object,
                    &request,
                    person_update::Response::ObjectUpdated(1),
                )?,
                person_update::Request::Scale { bits } => {
                    self.characters
                        .get_mut(&object)
                        .unwrap()
                        .character
                        .model
                        .set_scale([f32::from_bits(bits); 3])?;
                    self.acknowledge_person(object, &request, person_update::Response::Done)?;
                }
                person_update::Request::Decay => {
                    let household = self.household.as_ref().unwrap();
                    self.characters
                        .get_mut(&object)
                        .unwrap()
                        .person_update
                        .as_mut()
                        .unwrap()
                        .execution
                        .decay(
                            self.vm.world.objects.get_mut(&object).unwrap(),
                            &mut state,
                            household.clock.hour,
                            &household.tuning,
                        )?;
                }
                person_update::Request::Animator { only_visible } => {
                    if !self.drive_character_ordinary(
                        program,
                        object,
                        only_visible,
                        remaining,
                        step,
                    )? {
                        return Ok(false);
                    }
                    self.acknowledge_person(object, &request, person_update::Response::Done)?;
                }
                person_update::Request::RoomMembership => {
                    self.update_original_person_room(object)?;
                    self.acknowledge_person(object, &request, person_update::Response::Done)?;
                }
                person_update::Request::Environment { room } if self.room_environment.is_some() => {
                    let bits = self
                        .room_environment
                        .as_ref()
                        .unwrap()
                        .manager
                        .environment(room);
                    self.acknowledge_person(
                        object,
                        &request,
                        person_update::Response::Environment(bits),
                    )?;
                }
                person_update::Request::HasActionUse => {
                    let used = self
                        .vm
                        .threads
                        .get(&thread)
                        .ok_or("native person thread absent")?
                        .stack
                        .iter()
                        .any(|f| f.action_target.is_some());
                    self.acknowledge_person(object, &request, person_update::Response::Bool(used))?;
                }
                person_update::Request::AllowNegativeOutdoorEnvironment => {
                    let lot = *self
                        .vm
                        .world
                        .globals
                        .get(&10)
                        .ok_or("native current house absent")?;
                    let zoning = &self
                        .vm
                        .world
                        .person
                        .as_ref()
                        .ok_or("native zoning context absent")?
                        .lot_zoning;
                    let residential = sims_vm::zoning::is_residential(zoning, lot);
                    self.acknowledge_person(
                        object,
                        &request,
                        person_update::Response::Bool(residential),
                    )?;
                }
                person_update::Request::Statistics { has_action }
                    if self
                        .household
                        .as_ref()
                        .is_some_and(|h| h.house_statistics.is_some()) =>
                {
                    self.household
                        .as_mut()
                        .unwrap()
                        .house_statistics
                        .as_mut()
                        .unwrap()
                        .sample(has_action);
                    self.acknowledge_person(object, &request, person_update::Response::Done)?;
                }
                person_update::Request::AnimationName => {
                    let animator = &self.characters[&object].character.skin.animator;
                    // 593825 copies the foreground clip's name, or the literal
                    // "None" at 6d06a8 when the foreground pointer is null.
                    let name = animator
                        .slot(crate::pose::animator::Slot::Foreground)
                        .and_then(|id| animator.layer(id))
                        .map_or_else(|| b"None".to_vec(), |layer| layer.name.as_bytes().to_vec());
                    self.acknowledge_person(
                        object,
                        &request,
                        person_update::Response::AnimationName(name),
                    )?;
                }
                _ => return Ok(false),
            }
            *remaining = remaining.saturating_sub(1);
        }
    }
}
