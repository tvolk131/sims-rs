//! Animate Sim over the loaded native character. Resource loading retries stage
//! the whole command so an unresolved clip cannot consume events or change layers.
use crate::{Event, Simulation, character::Resident, pose::command};
use sims_vm::{AnimationOutcome, AnimationRequest, Program, Status, ThreadId};
use std::collections::BTreeMap;

#[cfg(test)]
#[path = "character_animation_tests.rs"]
pub(crate) mod tests;

enum Prepared {
    Asset(String),
    Profile,
    State {
        resident: Box<Resident>,
        person: BTreeMap<i16, i16>,
        result: Result<command::Outcome, String>,
        missing: Vec<crate::pose::animator::Slot>,
    },
}

impl Simulation {
    pub(crate) fn native_posture_name(
        &self,
        program: &Program,
        actor: sims_vm::ObjectId,
        index: u32,
    ) -> Result<Option<String>, String> {
        let object = self
            .vm
            .world
            .objects
            .get(&actor)
            .ok_or("animation actor absent")?;
        let person = object
            .person
            .as_ref()
            .ok_or("animation actor is not a person")?;
        let definition = object
            .definition
            .as_ref()
            .ok_or("animation actor definition absent")?;
        let field = |key| {
            person
                .get(&key)
                .copied()
                .ok_or_else(|| format!("animation person field {key} absent"))
        };
        Ok(sims_vm::animation::posture_name(
            field(58)?,
            field(64)?,
            field(65)?,
            index as usize,
            |table, index| {
                let (source, table) = match table {
                    sims_vm::animation::NameTable::Private(t) => (definition.source.as_str(), t),
                    sims_vm::animation::NameTable::Global(t) => (program.globals.as_str(), t),
                };
                program
                    .modules
                    .get(source)?
                    .strings
                    .get(&table)?
                    .get(index)
                    .map(String::as_str)
            },
        )
        .map(str::to_ascii_lowercase))
    }
    fn command_name(
        &self,
        program: &Program,
        id: ThreadId,
        request: &AnimationRequest,
        number: i16,
        lookup: command::Lookup,
    ) -> Result<Option<String>, String> {
        if let command::Lookup::Posture(index) = lookup {
            return self.native_posture_name(program, request.caller, index);
        }
        let actor = self
            .vm
            .world
            .objects
            .get(&request.caller)
            .ok_or("animation actor absent")?;
        let person = actor
            .person
            .as_ref()
            .ok_or("animation actor is not a person")?;
        let field = |key| {
            person
                .get(&key)
                .copied()
                .ok_or_else(|| format!("animation person field {key} absent"))
        };
        let string = |source: &str, table, index| {
            program
                .modules
                .get(source)?
                .strings
                .get(&table)?
                .get(index)
                .map(String::as_str)
        };
        let age = field(58)?;
        let thread = &self.vm.threads[&id];
        let frame = thread.stack.last().ok_or("animation frame absent")?;
        let bytes = request.operands;
        let (source, table) = match bytes[4] {
            0 => {
                // 5894e2 uses the most recent action-use record; indirect
                // operands instead use the frame's current stack object.
                let owner = if bytes[5] & 4 != 0 {
                    frame.stack_object
                } else {
                    thread
                        .stack
                        .iter()
                        .rev()
                        .find_map(|f| f.action_target)
                        .unwrap_or(request.caller)
                };
                let definition = self
                    .vm
                    .world
                    .objects
                    .get(&owner)
                    .and_then(|o| o.definition.as_ref())
                    .ok_or("animation owner definition absent")?;
                let foreground = sims_vm::animation::mode(bytes[5]) == 0;
                let active = self
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .and_then(|a| a.actors.get(&request.caller))
                    .and_then(|a| a.active.as_ref());
                let matches = active.is_some_and(|a| a.target == owner);
                let icon_age = if foreground && matches {
                    active
                        .and_then(|a| self.vm.world.objects.get(&a.icon_owner))
                        .filter(|o| {
                            o.definition
                                .as_ref()
                                .and_then(|d| {
                                    program
                                        .modules
                                        .get(&d.source)?
                                        .object_definition_words
                                        .get(&d.object)?
                                        .get(9)
                                })
                                .copied()
                                == Some(2)
                        })
                        .and_then(|o| o.person.as_ref())
                        .map(|p| p.get(&58).copied().ok_or("animation partner age absent"))
                        .transpose()?
                } else {
                    None
                };
                (
                    definition.source.as_str(),
                    sims_vm::animation::object_action_table(age, foreground, matches, icon_age),
                )
            }
            1 => (program.globals.as_str(), 128),
            2 => (program.globals.as_str(), 130),
            3 => (
                program.globals.as_str(),
                sims_vm::animation::global_action_table(age),
            ),
            _ => return Ok(None),
        };
        Ok(usize::try_from(number)
            .ok()
            .and_then(|n| string(source, table, n))
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase))
    }

    fn prepare_native_command(
        &self,
        program: &Program,
        id: ThreadId,
        request: &AnimationRequest,
    ) -> Result<Prepared, String> {
        let thread = self.vm.threads.get(&id).ok_or("animation thread absent")?;
        if !matches!(&thread.status, Status::AwaitingAnimation(wait) if wait == request) {
            return Err("stale native animation request".into());
        }
        let frame = thread.stack.last().ok_or("animation frame absent")?;
        let actor = self
            .vm
            .world
            .objects
            .get(&request.caller)
            .ok_or("animation actor absent")?;
        let mut person = actor
            .person
            .clone()
            .ok_or("animation actor is not a person")?;
        let mut resident = Box::new(
            self.characters
                .get(&request.caller)
                .ok_or("native character absent")?
                .clone(),
        );
        let yielded = Some((id, request.id, self.vm.tick));
        if resident.animation_yield == yielded
            || !(0..=3).contains(&resident.controls.ordinary.mode)
        {
            resident.animation_yield = yielded;
            return Ok(Prepared::State {
                resident,
                person,
                result: Ok(command::Outcome::Waiting),
                missing: vec![],
            });
        }
        if resident.refresh_tick.is_some()
            || resident.ordinary_update.is_some()
            || resident.character.busy()
        {
            return Err("Animate Sim during a pending character service".into());
        }
        let bytes = request.operands;
        let operand = i16::from_le_bytes([bytes[0], bytes[1]]);
        let number = if bytes[5] & 4 != 0 {
            usize::try_from(operand)
                .ok()
                .and_then(|i| frame.args.get(i))
                .copied()
                .ok_or("animation index parameter out of bounds")?
        } else {
            operand
        };
        let definition = actor
            .definition
            .as_ref()
            .ok_or("animation actor definition absent")?;
        let definition_flags = *program
            .modules
            .get(&definition.source)
            .and_then(|m| m.object_definition_words.get(&definition.object))
            .and_then(|w| w.get(0xcc / 2))
            .ok_or("animation definition flags absent")? as u8;
        let source = resident
            .source
            .as_ref()
            .ok_or("native animation source skeleton absent")?;
        let mut asset = None;
        let mut profile = false;
        let mut missing = Vec::new();
        let result = command::invoke(
            &mut resident.character.skin.animator,
            &mut resident.controls,
            &mut person,
            source,
            command::Input {
                number,
                flags: bytes[5],
                event_count: bytes[6] as i8,
                tick: self.vm.tick as i32,
                definition_flags,
            },
            |lookup| {
                if resident.profile_dirty {
                    profile = true;
                    return Err("native animation profile needs reload".into());
                }
                let Some(name) = self.command_name(program, id, request, number, lookup)? else {
                    return Ok(None);
                };
                if let Some(loaded) = program.animations.get(&name) {
                    return Ok(Some(&loaded.clip));
                }
                asset = Some(name);
                Err("native animation asset needs loading".into())
            },
            |slot| {
                missing.push(slot);
                Ok(())
            },
            || {
                let actions = self
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .ok_or("interruptible animation needs action storage")?;
                actions
                    .actors
                    .get(&request.caller)
                    .ok_or("animation actor queue absent")?
                    .queue
                    .should_interrupt(actor, actions.live_family)
            },
            |_| {
                if bytes[5] & 32 != 0 {
                    if usize::from(bytes[2]) >= frame.locals.len() {
                        return Err("animation event local out of bounds".into());
                    }
                } else if frame.args.is_empty() {
                    return Err("animation event parameter out of bounds".into());
                }
                Ok(())
            },
        );
        if profile {
            return Ok(Prepared::Profile);
        }
        if let Some(asset) = asset {
            return Ok(Prepared::Asset(asset));
        }
        resident.animation_yield = if matches!(result, Ok(command::Outcome::Waiting)) {
            yielded
        } else {
            None
        };
        Ok(Prepared::State {
            resident,
            person,
            result,
            missing,
        })
    }

    pub(crate) fn native_command_asset(
        &self,
        program: &Program,
        id: ThreadId,
        request: &AnimationRequest,
    ) -> Result<Option<String>, String> {
        if self.characters[&request.caller].skill_profile.is_some() {
            return Ok(None);
        }
        match self.prepare_native_command(program, id, request)? {
            Prepared::Asset(name) => Ok(Some(name)),
            // Primitive faults belong to service, where partial native state is
            // committed. An asset query must not consume their effects.
            Prepared::State { .. } | Prepared::Profile => Ok(None),
        }
    }

    pub(crate) fn native_command_profile(
        &self,
        program: &Program,
        id: ThreadId,
        request: &AnimationRequest,
    ) -> Result<bool, String> {
        if self.characters[&request.caller].skill_profile.is_some() {
            return Err("native animation profile remains pending".into());
        }
        if !self.characters[&request.caller].profile_dirty {
            return Ok(false);
        }
        Ok(matches!(
            self.prepare_native_command(program, id, request)?,
            Prepared::Profile
        ))
    }

    pub(crate) fn service_native_animation(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: &AnimationRequest,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        let Prepared::State {
            resident,
            person,
            result,
            missing,
        } = self.prepare_native_command(program, id, request)?
        else {
            return Err("native animation asset needs loading".into());
        };
        self.characters.insert(request.caller, *resident);
        self.vm
            .world
            .objects
            .get_mut(&request.caller)
            .unwrap()
            .person = Some(person);
        for slot in missing {
            self.event(
                events,
                id,
                format!("missing native {slot:?} animation tree"),
            );
        }
        let result = match result? {
            command::Outcome::Waiting => return Ok(false),
            command::Outcome::Completed => AnimationOutcome::Completed,
            command::Outcome::Event(value) => AnimationOutcome::Event(value),
        };
        self.vm.finish_animation(program, id, request.id, result)?;
        Ok(true)
    }
}
