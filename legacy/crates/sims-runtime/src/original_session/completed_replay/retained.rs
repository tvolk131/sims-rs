//! Re-execute explicitly supplied retained components, without a Session.
//!
//! This diagnostic driver does not supply an elapsed/application caller, input
//! queue, renderer cadence or source qualification. Callers must keep those
//! obligations separate from equality of the owners represented by `Retained`.
use crate::original_session::completed_assets::checkpoints::Retained;
use anyhow::{Context, Result, ensure};
use sims_content::{
    Index,
    character_loading::{Presentation, live::dependent, live::graphic},
    graphic_loading, graphic_materials,
    saved_avatar::BoundAvatar,
};
use sims_sim::pose::refresh::{Request, Response};
use sims_vm::{ObjectId, Program, Trace};
use std::collections::BTreeMap;

pub struct Runtime<'a> {
    index: &'a Index,
    simulation: sims_sim::Simulation,
    characters: BTreeMap<ObjectId, sims_content::character_loading::live::SavedAssets>,
    meshes: sims_content::character_geometry::MeshPool,
    native_meshes: Vec<u32>,
    dependent: BTreeMap<ObjectId, dependent::Job>,
    presentation: crate::original_session::completed_presentation::Bindings,
    floors: Option<BTreeMap<u16, sims_content::character_loading::floors::Binding>>,
    ordinary: graphic_loading::Registry,
    avatars: BTreeMap<ObjectId, BoundAvatar>,
}

/// Explicit service inputs, including the backend fields which have not yet
/// been independently qualified for the current Goth diagnostic.
#[derive(Clone, Copy)]
pub struct Services {
    pub materials: graphic_materials::Backend,
    pub normal_material_mode: u32,
}

impl<'a> Runtime<'a> {
    pub fn restore(index: &'a Index, program: &Program, cut: Retained) -> Result<Self> {
        cut.validate(index, program)?;
        let simulation = cut
            .simulation
            .restore(program)
            .map_err(anyhow::Error::msg)?;
        let ordinary = graphic_loading::Registry::restore(index, cut.ordinary.clone())?;
        let avatars = cut
            .characters
            .iter()
            .map(|(&id, a)| {
                Ok((
                    id,
                    BoundAvatar::restore(index, &a.skeleton, id as u64, &a.binding)?,
                ))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            index,
            simulation,
            characters: cut.characters,
            meshes: cut.meshes,
            native_meshes: cut.native_meshes,
            dependent: cut.dependent,
            presentation: cut.presentation,
            floors: cut.floors,
            ordinary,
            avatars,
        })
    }

    pub fn tick(&self) -> u64 {
        self.simulation.vm.tick
    }

    pub fn pending_services(&self) -> Vec<(ObjectId, Request)> {
        self.simulation.pending_character_services()
    }

    pub fn snapshot(&self, program: &Program) -> Retained {
        Retained {
            simulation: sims_sim::Snapshot::capture(program, &self.simulation),
            characters: self.characters.clone(),
            ordinary: self.ordinary.snapshot(),
            meshes: self.meshes.clone(),
            native_meshes: self.native_meshes.clone(),
            dependent: self.dependent.clone(),
            presentation: self.presentation.clone(),
            floors: self.floors.clone(),
        }
    }

    /// An explicit caller-owned publication, never inferred from a tick or
    /// missing animation. Only the independently supported null-bounds upload
    /// acknowledgement is available here.
    pub fn publish_cpu(
        &mut self,
        program: &Program,
        actors: &[ObjectId],
        control: u16,
    ) -> Result<()> {
        let before = sims_sim::Snapshot::capture(program, &self.simulation);
        for &actor in actors {
            let a = self
                .characters
                .get_mut(&actor)
                .context("publication asset owner absent")?;
            let resident = self
                .simulation
                .characters
                .get_mut(&actor)
                .context("publication resident absent")?;
            ensure!(
                resident.character.skin.animator.fp_control() == control,
                "publication precision differs"
            );
            let uploads = a.model.with_bound(resident, |bound| {
                a.geometry.update_loaded_vertices_with_control(
                    &self.avatars[&actor],
                    bound,
                    false,
                    control,
                )
            })?;
            for binding in uploads {
                a.geometry
                    .vertices
                    .get_mut(&binding)
                    .context("notified buffer absent")?
                    .acknowledge_null_bounds_upload(false)?;
            }
        }
        ensure!(
            sims_sim::Snapshot::capture(program, &self.simulation) == before,
            "CPU publication changed simulation/VM/RNG/continuations"
        );
        Ok(())
    }

    /// Finish the current registry pass using the same production character
    /// services as the retained research producer. `before_service` can save a
    /// suspended cut and its trace-prefix length before any reply is supplied.
    /// This method grants no authority to schedule a new elapsed-clock child.
    pub fn finish_update(
        &mut self,
        program: &Program,
        services: Services,
        mut before_service: impl FnMut(&Self, usize) -> Result<()>,
    ) -> Result<Vec<Trace>> {
        let tick = self.tick();
        let mut trace = Vec::new();
        let mut polls = Vec::new();
        for _ in 0..256 {
            let start = polls.len();
            let step = self
                .simulation
                .run_current_tick_with_host_polls(program, 100_000, &mut polls)
                .map_err(anyhow::Error::msg)?;
            for position in 0..=step.trace.len() {
                for poll in &polls[start..] {
                    if poll.before_instruction == position {
                        trace.push(poll.instruction.clone());
                    }
                }
                if let Some(instruction) = step.trace.get(position) {
                    trace.push(instruction.clone());
                }
            }
            ensure!(
                step.events.is_empty(),
                "retained diagnostic requires an unbound host event"
            );
            if step.stop != sims_sim::StepStop::Budget {
                ensure!(
                    step.stop == sims_sim::StepStop::Waiting
                        && self.simulation.pending_character_services().is_empty()
                        && self.simulation.pending_person_services().is_empty()
                        && self.simulation.pending_character_updates().is_empty(),
                    "retained diagnostic stopped with unsupported pending work"
                );
                ensure!(
                    self.simulation
                        .finish_registry_pass()
                        .map_err(anyhow::Error::msg)?,
                    "retained diagnostic did not complete game time"
                );
                // 467094 belongs to the explicit4a8895 view caller. A burst
                // may finish several updates before that caller runs.
                ensure!(
                    self.tick() == tick + 1,
                    "retained update crossed more than one boundary"
                );
                return Ok(trace);
            }
            before_service(self, trace.len())?;
            ensure!(
                self.service(program, services)?,
                "retained diagnostic has an unsupported service"
            );
        }
        anyhow::bail!("retained diagnostic service work bound exceeded")
    }

    fn service(&mut self, program: &Program, services: Services) -> Result<bool> {
        let mut completed = false;
        for (actor, request) in self.simulation.pending_character_services() {
            if request == Request::OrdinaryCarriedObject {
                let child = self.simulation.vm.world.objects[&actor].slots[1]
                    .context("dependent child absent")?;
                let outside = !self
                    .simulation
                    .native_lot
                    .as_ref()
                    .context("dependent lot absent")?
                    .placement
                    .links
                    .capture_raw(&self.simulation.vm.world, child)
                    .map_err(anyhow::Error::msg)?
                    .on_lot;
                let view = graphic::View {
                    viewport: self.presentation.viewport,
                    outside,
                    visibility_enabled: self.presentation.visibility.enabled,
                    camera: self.presentation.visibility.camera,
                    materials: services.materials,
                    normal_material_mode: services.normal_material_mode,
                };
                dependent::Owners {
                    index: self.index,
                    ordinary: &mut self.ordinary,
                    mesh_pool: &mut self.meshes,
                    jobs: &mut self.dependent,
                }
                .service_dependent(&mut self.simulation, program, actor, view)?;
                completed = true;
                continue;
            }
            let a = self
                .characters
                .get_mut(&actor)
                .context("retained asset owner absent")?;
            if a.geometry
                .service_resident(
                    &mut a.model,
                    self.simulation
                        .characters
                        .get_mut(&actor)
                        .context("retained resident absent")?,
                    &self.avatars[&actor],
                    &mut self.meshes,
                )?
                .is_some()
            {
                completed = true;
                continue;
            }
            let response = match request {
                Request::Property(sims_sim::pose::properties::Request::FootstepSurface {
                    ..
                }) if self.floors.is_some() => Response::PropertySurface(
                    self.simulation
                        .original_footstep_surface(
                            actor,
                            &sims_content::character_loading::floors::categories(
                                self.floors.as_ref().unwrap(),
                            ),
                        )
                        .map_err(anyhow::Error::msg)?,
                ),
                Request::Property(
                    ref property @ sims_sim::pose::properties::Request::Sound { .. },
                ) if self.presentation.disabled_sound.is_some() => {
                    self.presentation.property(actor, property)?
                }
                Request::Viewport => Response::Viewport(self.presentation.viewport),
                Request::Visibility { viewport } => {
                    Response::Visible(self.presentation.visibility.visible(
                        &self.simulation.characters[&actor],
                        &a.model,
                        viewport,
                    )?)
                }
                _ => continue,
            };
            self.simulation
                .complete_character_service(actor, &request, response)
                .map_err(anyhow::Error::msg)?;
            completed = true;
        }
        Ok(completed)
    }
}

#[cfg(test)]
pub(crate) mod verification;
