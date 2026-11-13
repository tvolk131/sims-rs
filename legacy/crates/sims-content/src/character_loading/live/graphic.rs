//! Live lot services for retained ordinary graphics. Scheduling/selection owns
//! when an update begins; this adapter must not advance the simulation clock.
use super::*;
use crate::{
    graphic_materials::Backend,
    graphic_update::{Environment, Request, Step},
};
use sims_sim::pose::model::Matrix;

/// Services owned by the presentation/caller, separate from object positions,
/// terrain and clock read from the live simulation on each advancement.
pub struct View {
    pub viewport: [i32; 4],
    pub outside: bool,
    pub visibility_enabled: bool,
    pub camera: Matrix,
    pub materials: Backend,
    pub normal_material_mode: u32,
}
impl<P: Presentation> Host<P> {
    /// The same pre-unlink raw service used by the load Reader. General live
    /// retained-frame cadence remains gated by update_contexts until supported.
    pub fn service_retained_removal_graphic(
        &mut self,
        simulation: &mut Simulation,
        program: &Program,
        id: ObjectId,
    ) -> Result<()> {
        crate::graphic_loading::retained::refresh::service(
            &mut self.ordinary,
            simulation,
            program,
            id,
        )
    }
    pub(super) fn service_graphic_retirement(
        &mut self,
        simulation: &mut Simulation,
        id: ObjectId,
    ) -> Result<()> {
        crate::graphic_retirement::service(
            &self.index,
            &mut self.ordinary,
            &mut self.mesh_pool,
            self.initializer_settings,
            &mut self.presentation,
            simulation,
            id,
        )
    }
    pub(super) fn validate_selection_factory_graphics(
        &self,
        simulation: &Simulation,
    ) -> Result<()> {
        let pending = simulation.pending_native_selection();
        if let Some((sims_sim::lot_selection::Request::FactoryGraphic(id), started)) = pending {
            ensure!(
                started == (self.ordinary.pending(id) || self.ordinary.graphics.contains_key(&id)),
                "selection factory lost its asset continuation"
            );
        }
        for id in self.ordinary.pending_objects() {
            ensure!(
                pending == Some((sims_sim::lot_selection::Request::FactoryGraphic(id), true)),
                "ordinary graphic constructor lost its live factory owner"
            );
        }
        Ok(())
    }
    /// One fresh ordinary-Animator service while the selection pool's original
    /// object factory is suspended before its date stamp, Reset and Init.
    pub fn service_selection_factory_graphic(
        &mut self,
        simulation: &mut Simulation,
        program: &Program,
        id: ObjectId,
        viewport: [i32; 4],
    ) -> Result<()> {
        let request = sims_sim::lot_selection::Request::FactoryGraphic(id);
        let (pending, started) = simulation
            .pending_native_selection()
            .context("selection factory owner absent")?;
        ensure!(
            pending == request,
            "selection factory graphic request differs"
        );
        if !started {
            self.ordinary.begin(&simulation.vm, program, id)?;
            simulation
                .start_selection_graphic(request)
                .map_err(anyhow::Error::msg)?;
        }
        let object = simulation
            .vm
            .world
            .objects
            .get(&id)
            .context("star graphic object absent")?;
        let position = simulation
            .vm
            .world
            .positions
            .get(&id)
            .context("star graphic position absent")?;
        let lot = simulation
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("star graphic terrain absent")?;
        let floor_vector = simulation
            .native_character_settings
            .context("star graphic view absent")?
            .floor_vector;
        let mut crt = simulation
            .vm
            .crt_rng_state
            .context("star graphic CRT stream absent")?;
        let result = if self.ordinary.graphics.contains_key(&id) {
            // Asset handoff may have completed before a later lifecycle error.
            Ok(true)
        } else {
            self.ordinary.advance(
                &self.index,
                &simulation.vm,
                program,
                id,
                &mut crt,
                self.initializer_settings,
                &mut self.mesh_pool,
                crate::graphic_initialization::Environment {
                    owner_appearance_index: *object
                        .data
                        .get(&68)
                        .context("star appearance index absent")?,
                    source: sims_sim::pose::graphic_transform::Source {
                        position: [position.x, position.y],
                        facing: *object.data.get(&1).context("star facing absent")?,
                        level: position.level,
                    },
                    view: transform::View {
                        rotation: simulation.vm.world.camera_rotation,
                        floor_vector,
                    },
                    terrain: transform::Terrain {
                        size: lot.size,
                        corners: &lot.heights,
                    },
                    viewport,
                },
            )
        };
        // Retain any constructor draws alongside its partial asset state, even
        // when the next original asset service reports an error.
        simulation.vm.crt_rng_state = Some(crt);
        if result? {
            simulation
                .complete_selection_factory_graphic(program, id)
                .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    /// Complete one retained service. Pending property/blend callbacks require
    /// their actual owner; this method never substitutes a person's handler.
    /// The caller must keep the ordinary scheduler suspended until Complete.
    pub fn service_graphic(
        &mut self,
        simulation: &mut Simulation,
        program: &Program,
        id: ObjectId,
        view: View,
    ) -> Result<Step> {
        advance_retained(
            &self.index,
            &mut self.ordinary,
            &mut self.mesh_pool,
            simulation,
            program,
            id,
            view,
        )
    }
}

/// Borrowed owners for a retained ordinary update. Shared by the full live host
/// and source-qualified completed-state replay; no constructor is involved.
pub(super) fn advance_retained(
    index: &Index,
    ordinary: &mut crate::graphic_loading::Registry,
    mesh_pool: &mut MeshPool,
    simulation: &mut Simulation,
    program: &Program,
    id: ObjectId,
    view: View,
) -> Result<Step> {
    ordinary.validate_with_retired(&simulation.vm, &simulation.retired_graphics)?;
    let object = simulation
        .vm
        .world
        .objects
        .get(&id)
        .context("ordinary graphic object absent")?;
    let definition = object
        .definition
        .as_ref()
        .context("ordinary graphic definition absent")?;
    let words =
        sims_vm::construction::definition_words(program, definition).map_err(anyhow::Error::msg)?;
    let kind = words[9];
    let position = simulation
        .vm
        .world
        .positions
        .get(&id)
        .context("ordinary graphic position absent")?;
    let lot = simulation
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .context("ordinary graphic original terrain absent")?;
    let floor_vector = simulation
        .native_character_settings
        .context("ordinary graphic view settings absent")?
        .floor_vector;
    let graphic = ordinary
        .graphics
        .get_mut(&id)
        .context("ordinary graphic owner absent; constructor replay is not a restore")?;
    let step = graphic.advance_update(
        index,
        program,
        mesh_pool,
        Environment {
            placement: crate::graphic_initialization::Environment {
                owner_appearance_index: *object
                    .data
                    .get(&68)
                    .context("ordinary appearance index absent")?,
                source: sims_sim::pose::graphic_transform::Source {
                    position: [position.x, position.y],
                    facing: *object.data.get(&1).context("ordinary facing absent")?,
                    level: position.level,
                },
                view: transform::View {
                    rotation: simulation.vm.world.camera_rotation,
                    floor_vector,
                },
                terrain: transform::Terrain {
                    size: lot.size,
                    corners: &lot.heights,
                },
                viewport: view.viewport,
            },
            tick: simulation.vm.tick as i32,
            outside: view.outside,
            visibility_enabled: view.visibility_enabled,
            camera: view.camera,
            materials: view.materials,
            normal_material_mode: view.normal_material_mode,
        },
    )?;
    if let Step::Pending(request) = &step {
        match request {
            Request::Invalidate { object } => {
                // Actual 434101(owner, 0): refresh the containment root and
                // descendants, including original dynamic/expiry fields.
                simulation
                    .vm
                    .refresh_object(*object, *object, 0, 0)
                    .map_err(anyhow::Error::msg)?;
            }
            Request::Position { .. } if kind == 4 => {
                // cXObject virtual +78 is 51d372, an actual no-op. In
                // particular this does NOT teleport the physical marker.
            }
            _ => return Ok(step),
        }
        graphic.acknowledge_update(request)?;
        return Ok(Step::Progress);
    }
    Ok(step)
}
