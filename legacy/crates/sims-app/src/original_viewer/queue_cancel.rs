//! Revalidate a queued UI intent at the exclusive household-owner boundary.
//! A publication can remain on screen while a prior request completes.
use super::*;

pub(super) fn apply(viewer: &mut Viewer, id: u64) -> Result<()> {
    let native_id = u32::try_from(id).context("invalid native action ID")?;
    let session = viewer.session.as_mut().context("household still loading")?;
    let simulation = &mut session.simulation;
    // Preserve the authored posture adapter's existing retry behavior. Its
    // retained seated/blocked-exit state can outlive the native queue record.
    let retained_posture = simulation
        .interactions
        .get(&viewer.resident)
        .and_then(|actor| actor.active.as_ref())
        .is_some_and(|active| active.item.id == id);
    if simulation.native_lot.is_some()
        && !retained_posture
        && simulation.has_interaction_actor(viewer.resident)
        && let Some(actor) = simulation
            .vm
            .world
            .actions
            .as_ref()
            .and_then(|world| world.actors.get(&viewer.resident))
    {
        match actor
            .active
            .iter()
            .chain(actor.queue.pending.iter())
            .find(|action| action.id == native_id)
        {
            Some(action) if action.flags & sims_vm::action::CANCELLED != 0 => {
                viewer.status = "Cancellation already requested.".into();
                return Ok(());
            }
            None => {
                // Includes a duplicate click after the first request removed
                // a queued action, or an action that completed in the interim.
                viewer.status = "That interaction is no longer queued.".into();
                return Ok(());
            }
            Some(_) => {}
        }
    }
    // Missing actor/services and genuine engine failures keep their previous
    // error path. Only an authoritative already-requested/absent queue record
    // is treated as a harmless stale UI intent.
    simulation
        .cancel_interaction(&session.program, viewer.resident, id)
        .map_err(anyhow::Error::msg)?;
    viewer.status = "Cancellation requested.".into();
    Ok(())
}
