//! Shared load/live raw Refresh service, before RemoveFromLot may unlink.
use super::*;
use sims_sim::Simulation;
use sims_vm::Program;

#[cfg(test)]
mod tests;

fn environment(
    simulation: &Simulation,
    context: &RefreshContext,
    id: ObjectId,
) -> Result<frame::Environment> {
    let object = simulation
        .vm
        .world
        .objects
        .get(&id)
        .context("retained Refresh object absent")?;
    let position = simulation
        .vm
        .world
        .positions
        .get(&id)
        .context("retained Refresh position absent")?;
    if let Some(settings) = simulation.native_character_settings {
        ensure!(
            settings.floor_vector.map(f32::to_bits) == context.floor_vector_bits,
            "retained Refresh shared floor vector differs"
        );
    }
    let terrain = simulation
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref());
    Ok(frame::Environment {
        tick: simulation.vm.tick as i32,
        physical: frame::Physical {
            position: [position.x, position.y],
            facing: *object
                .data
                .get(&1)
                .context("retained Refresh facing absent")?,
            level: position.level,
        },
        camera_rotation: simulation.vm.world.camera_rotation,
        floor_vector_bits: context.floor_vector_bits,
        terrain_extent: terrain.map(|a| a.size),
        terrain_corners: terrain.map(|a| a.heights.clone()),
        fp_control: context.fp_control,
        origin_pose_bits: context.origin_pose_bits,
    })
}

/// Execute one owned service against the original Simulation. Each call joins
/// its saved physical/removal continuation with this same graphic/lifetime.
/// Invalidation performs the actual VM service; it is never supplied as Done.
pub fn service(
    ordinary: &mut crate::graphic_loading::Registry,
    simulation: &mut Simulation,
    program: &Program,
    id: ObjectId,
) -> Result<()> {
    let (_, started) = simulation
        .pending_removal_graphic()
        .context("retained Refresh placement absent")?;
    simulation
        .validate_removal_graphic_owner(program, id, started)
        .map_err(anyhow::Error::msg)?;
    ordinary.validate_with_retired(&simulation.vm, &simulation.retired_graphics)?;
    let registry = &mut ordinary.retained;
    let mut graphic = registry
        .graphics
        .get(&id)
        .context("retained Refresh graphic absent")?
        .clone();
    ensure!(
        graphic.refresh_started == started && !graphic.started,
        "retained Refresh lost its saved placement service"
    );
    let frame = graphic.owner.frame_mut()?;
    frame.validate_vm(&simulation.vm)?;
    if !started {
        frame.begin_refresh(true)?;
        // Exact public-owner preflight above makes this acknowledgement
        // infallible without interleaving; no partial asset commit precedes it.
        simulation
            .start_removal_graphic(id)
            .map_err(anyhow::Error::msg)?;
        graphic.refresh_started = true;
    } else {
        let context = registry
            .refresh_context
            .as_ref()
            .context("retained Refresh process context absent")?;
        let environment = environment(simulation, context, id)?;
        if frame.advance(&environment)? == Some(sims_sim::pose::graphic_frame::Request::Invalidate)
        {
            frame.perform_invalidation(&mut simulation.vm)?;
        }
        if frame.complete() {
            ensure!(
                frame.pending().is_none(),
                "retained Refresh completed with an external service"
            );
            simulation
                .complete_removal_graphic(id)
                .map_err(anyhow::Error::msg)?;
            graphic.refresh_started = false;
        }
    }
    registry.graphics.insert(id, graphic);
    Ok(())
}
