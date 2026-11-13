//! Explicit synchronous draw/portrait appearance preparation. Native590c05
//! invokes58a178(force0) even when profile+d8 is clear; portrait4e77c6 forces it.
//! Rendering itself only reads the completed retained bindings.
use super::*;
use sims_sim::pose::{
    appearance::{Execution, Input},
    appearance_selection::Request as SelectionRequest,
};

impl<P: Presentation + Clone> Host<P> {
    /// Source590ca9..590cbd, before geometry/drawing. No tick, RNG draw or
    /// animation sampling; cached selectors remain authoritative until this
    /// explicit operation successfully installs every requested binding.
    pub fn prepare_draw_appearance(
        &mut self,
        simulation: &mut Simulation,
        program: &Program,
        id: ObjectId,
    ) -> Result<()> {
        self.prepare_appearance(simulation, program, id, false)
            .map(|_| ())
    }
    /// Prepare the actual resident's current appearance for portrait drawing.
    /// This native call does not yield or tick. Work on cloned owners so a
    /// missing asset/unsupported presentation does not install a partial
    /// selection. The presentation clone must own its transaction state (as
    /// the software View does); external irreversible callbacks do not belong
    /// in this synchronous preparation host.
    pub(super) fn prepare_appearance(
        &mut self,
        simulation: &mut Simulation,
        program: &Program,
        id: ObjectId,
        portrait: bool,
    ) -> Result<Vec<(usize, usize)>> {
        ensure!(
            !self.fresh.contains_key(&id)
                && !self.reloads.contains_key(&id)
                && !self.accessories.contains_key(&id)
                && !self.retirements.contains_key(&id),
            "portrait preparation overlaps a character asset operation"
        );
        let assets = self
            .characters
            .get(&id)
            .context("appearance assets absent")?;
        let resident = simulation
            .characters
            .get(&id)
            .context("appearance resident absent")?;
        ensure!(
            portrait
                || (!resident.character.busy()
                    && resident.ordinary_update.is_none()
                    && resident.person_update.is_none()
                    && resident.skill_profile.is_none()),
            "draw preparation overlaps an animator continuation"
        );
        ensure!(
            assets.appearance_owner.model_present,
            "portrait model absent"
        );
        // 590cb8 still calls selection with +d8 clear. Evaluate that call
        // before copying mesh buffers/world owners; an ordinary cache hit has
        // no binding or notification work. Retain the actual selector writes.
        let mut prepared = None;
        if portrait || !resident.profile_dirty {
            let object = simulation
                .vm
                .world
                .objects
                .get(&id)
                .context("appearance person absent")?;
            let definition = object
                .definition
                .as_ref()
                .context("appearance definition absent")?;
            let words = program
                .modules
                .get(&definition.source)
                .and_then(|m| m.object_definition_words.get(&definition.object))
                .context("appearance definition words absent")?;
            let flags = *words
                .get(0xcc / 2)
                .context("appearance definition flags absent")? as u16;
            let mut person = object
                .person
                .clone()
                .context("appearance person data absent")?;
            let mut profiles = simulation.vm.world.body_profiles.clone();
            let mut career = object.career_appearance.clone();
            let mut state = assets.appearance_owner.selection.clone();
            state.dirty = resident.character.placement.dirty;
            let selection = crate::clothing_profile::select_appearance_with_career(
                program,
                &mut profiles,
                definition,
                &mut state,
                &mut person,
                flags,
                portrait,
                &mut career,
                &mut crate::clothing_catalog::Installed {
                    index: &self.index,
                    cache: &mut self.clothing,
                },
            )?;
            if selection.is_complete() && !portrait {
                self.characters
                    .get_mut(&id)
                    .unwrap()
                    .appearance_owner
                    .selection = state.clone();
                simulation
                    .characters
                    .get_mut(&id)
                    .unwrap()
                    .character
                    .placement
                    .dirty = state.dirty;
                simulation.vm.world.objects.get_mut(&id).unwrap().person = Some(person);
                simulation
                    .vm
                    .world
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .career_appearance = career;
                simulation.vm.world.body_profiles = profiles;
                return Ok(vec![]);
            }
            prepared = Some((state, person, profiles, selection, career));
        }
        let mut assets = assets.clone();
        let mut resident = resident.clone();
        let mut vm = simulation.vm.clone();
        let mut pool = self.mesh_pool.clone();
        let mut presentation = self.presentation.clone();
        if !portrait && resident.profile_dirty {
            // 590ca9 clears +d8 before its virtual5888d3 reload. This call is
            // synchronous; failed assets leave the retained owners unchanged.
            resident.profile_dirty = false;
            let mut service =
                character_appearance::Service::begin(id, assets.appearance_owner.clone());
            let mut placement = resident.character.placement.clone();
            loop {
                let pending = assets.model.with_bound(&mut resident, |bound| {
                    service.advance_bound_with_clothing(
                        &self.index,
                        &mut vm,
                        program,
                        &mut assets.avatar,
                        &mut assets.geometry,
                        bound,
                        &mut placement,
                        &mut pool,
                        &mut crate::clothing_catalog::Installed {
                            index: &self.index,
                            cache: &mut self.clothing,
                        },
                    )
                })?;
                if let Some(request) = pending {
                    character_appearance::notify_images(&mut vm, id, &request)?;
                    presentation.appearance(id, &request)?;
                    service.acknowledge(&mut assets.avatar.records)?;
                } else {
                    break;
                }
            }
            resident.character.placement = placement;
            assets.appearance_owner = service.owner;
            resident.character.censor = Censor {
                binding: assets.appearance_owner.attachment,
                mask: assets.appearance_owner.attachment_metadata as i32,
            };
            vm.world.objects.get_mut(&id).unwrap().accessories.clear();
        }
        let object = vm
            .world
            .objects
            .get_mut(&id)
            .context("portrait person absent")?;
        let definition = object
            .definition
            .as_ref()
            .context("portrait definition absent")?;
        let words = program
            .modules
            .get(&definition.source)
            .and_then(|m| m.object_definition_words.get(&definition.object))
            .context("portrait definition words absent")?;
        let flags = *words
            .get(0xcc / 2)
            .context("portrait definition flags absent")? as u16;
        let person = object
            .person
            .as_mut()
            .context("portrait person data absent")?;
        // +d9 is shared with placement. Unlike profile reload, this call does
        // not clear records, reset hand fields, or reload identity/accessories.
        let mut selection = if let Some((state, fields, profiles, selection, career)) = prepared {
            assets.appearance_owner.selection = state;
            *person = fields;
            object.career_appearance = career;
            vm.world.body_profiles = profiles;
            selection
        } else {
            assets.appearance_owner.selection.dirty = resident.character.placement.dirty;
            crate::clothing_profile::select_appearance_with_career(
                program,
                &mut vm.world.body_profiles,
                definition,
                &mut assets.appearance_owner.selection,
                person,
                flags,
                portrait,
                &mut object.career_appearance,
                &mut crate::clothing_catalog::Installed {
                    index: &self.index,
                    cache: &mut self.clothing,
                },
            )?
        };
        resident.character.placement.dirty = assets.appearance_owner.selection.dirty;
        while let Some(request) =
            selection.next(&assets.appearance_owner.selection, &assets.avatar.records)
        {
            match request {
                SelectionRequest::Bind { record, descriptor } => {
                    assets.model.with_bound(&mut resident, |bound| {
                        assets.geometry.apply_record(
                            &self.index,
                            &mut assets.avatar,
                            record,
                            Execution::bind(Input::Replace {
                                name: descriptor.name,
                                texture: descriptor.texture,
                            }),
                            bound,
                            &mut pool,
                        )
                    })?;
                }
                other => {
                    let request = character_appearance::Request::Selection(other);
                    character_appearance::notify_images(&mut vm, id, &request)?;
                    presentation.appearance(id, &request)?;
                }
            }
            selection.acknowledge().map_err(anyhow::Error::msg)?;
        }
        // Selection may have replaced a head. Collect only after every binding
        // callback and tint notification has completed, in the retained order.
        let bindings = if portrait {
            assets.geometry.portrait_bindings(&assets.avatar)?
        } else {
            vec![]
        };
        self.characters.insert(id, assets);
        simulation.characters.insert(id, resident);
        simulation.vm = vm;
        self.mesh_pool = pool;
        self.presentation = presentation;
        Ok(bindings)
    }
}
