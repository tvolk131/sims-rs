//! Original saved animation descriptors, resource binding and read-tail entry.
//! The caller supplies an initialized owner and services the queued native tail.
use crate::{Index, assets};
use anyhow::{Context, Result, ensure};
use sims_formats::{avatar::Skeleton, saved_objects::Person};
use sims_sim::pose::{
    animator::{Animator, Settings, Slot},
    layer::{Descriptor, Layer},
};
use sims_vm::Program;

/// Read descriptors and saved animator words into a genuinely initialized
/// model, then transfer its owners and enter 5918ba's tail. The returned model
/// cache must accompany the resident through geometry services and snapshots.
/// Person motives/words and post-load Reset have not been read/run here.
pub fn restore_initialized(
    index: &Index,
    program: &mut Program,
    person: &Person,
    mut initialized: sims_sim::pose::initialize::Initialization,
    censor: sims_sim::pose::refresh::Censor,
    evaluated_tick: u64,
    context: sims_sim::pose::properties::Context,
    now: i32,
) -> Result<(sims_sim::character::Resident, crate::character_model::State)> {
    ensure!(
        initialized.is_complete() && initialized.initialized,
        "saved animator requires completed native initialization"
    );
    ensure!(
        person.floats.iter().all(|v| v.is_finite()),
        "nonfinite saved animator state"
    );
    restore(
        index,
        program,
        &initialized.source,
        person,
        &mut initialized.bound.skeleton.skin.animator,
        now,
        initialized.settings,
    )?;
    initialized.ordinary.mode = match person.routing_state {
        0..=3 => person.routing_state,
        _ => 3,
    };
    initialized
        .scalars
        .head_angles
        .copy_from_slice(&person.floats[..3]);
    initialized.scalars.head_target_point = Some(person.floats[3..6].try_into()?);
    initialized
        .scalars
        .placement
        .offset
        .copy_from_slice(&person.floats[6..9]);
    let (mut resident, model) = crate::character_model::State::handoff(
        initialized,
        censor,
        evaluated_tick,
        person.animation_events,
        person.engaged,
        context,
    )?;
    resident
        .character
        .begin_loaded_tail(true)
        .map_err(anyhow::Error::msg)?;
    Ok((resident, model))
}

/// 5918ba's represented saved animator state, followed by its actual read tail.
/// The owner must already have gone through model/rig initialization and saved
/// appearance binding. In particular this never constructs a standing-pose
/// replacement for a missing initialized owner. The caller must finish every
/// returned Character service before running post-load Reset callbacks.
///
/// `now` is the descriptor construction clock. Character::clock and the head
/// clock retain their initialized values until the native tail changes them.
/// Native person storage is read after this animator subrecord; the tail host
/// must use the person state at that read boundary, not post-load callback state.
pub fn restore_character(
    index: &Index,
    program: &mut Program,
    skeleton: &Skeleton,
    person: &Person,
    resident: &mut sims_sim::character::Resident,
    now: i32,
) -> Result<()> {
    ensure!(
        !resident.character.busy() && !resident.controls.movement.busy(),
        "saved character restoration during a pending update"
    );
    ensure!(
        person.floats.iter().all(|v| v.is_finite()),
        "nonfinite saved animator state"
    );
    ensure!(
        skeleton.bones.len() == resident.character.skin.bones.len()
            && skeleton.bones.iter().map(|b| b.parent).eq(resident
                .character
                .hierarchy
                .parents
                .iter()
                .copied()),
        "saved character rig does not match initialized owner"
    );
    resident.controls.animation_events = person.animation_events;
    resident.controls.engaged = person.engaged;
    restore(
        index,
        program,
        skeleton,
        person,
        &mut resident.character.skin.animator,
        now,
        resident.controls.settings,
    )?;
    resident.controls.ordinary.mode = match person.routing_state {
        0..=3 => person.routing_state,
        _ => 3,
    };
    let head = &mut resident.character.head;
    head.basis_heading = person.floats[0];
    head.target_heading = person.floats[1];
    head.turn = person.floats[2];
    head.target_point = Some(person.floats[3..6].try_into()?);
    resident
        .character
        .placement
        .offset
        .copy_from_slice(&person.floats[6..9]);
    resident
        .character
        .begin_loaded_tail(true)
        .map_err(anyhow::Error::msg)
}

/// Native 5918ba restores foreground, carry (+0x13c), then base. Empty or
/// incomplete descriptors clear their slots. Missing referenced assets surface
/// an error here so an imported home cannot silently acquire a substitute pose.
/// This returns independent layers. Use restore() to install into an existing
/// owner with native foreground/carry/base replacement and fade semantics.
pub fn load(
    index: &Index,
    program: &mut Program,
    skeleton: &Skeleton,
    person: &Person,
    now: i32,
    sampling_config: u32,
) -> Result<[Option<Layer>; 3]> {
    let mut layers = [None, None, None];
    for (slot, text) in person.animations.iter().enumerate() {
        layers[slot] = load_one(index, program, skeleton, text, now, sampling_config)?;
    }
    Ok(layers)
}

fn load_one(
    index: &Index,
    program: &mut Program,
    skeleton: &Skeleton,
    text: &str,
    now: i32,
    sampling_config: u32,
) -> Result<Option<Layer>> {
    let Some(descriptor) = Descriptor::parse(text) else {
        return Ok(None);
    };
    if !program.animations.contains_key(&descriptor.name) {
        assets::load_named_animations(index, program, &[descriptor.name.clone()].into())?;
    }
    let asset = program
        .animations
        .get(&descriptor.name)
        .with_context(|| format!("saved animation {} is absent", descriptor.name))?;
    Ok(Some(
        descriptor
            .bind(skeleton, &asset.clip, now, sampling_config)
            .map_err(anyhow::Error::msg)?,
    ))
}

/// Native 5918ba's descriptor order, with 591eec clearing before lookup even
/// when the new descriptor is incomplete or missing its resource. This restores
/// represented descriptor state, not omitted original cursors or fade history.
pub fn restore(
    index: &Index,
    program: &mut Program,
    skeleton: &Skeleton,
    person: &Person,
    animator: &mut Animator,
    now: i32,
    settings: Settings,
) -> Result<()> {
    for (slot, text) in [Slot::Foreground, Slot::Carry, Slot::Base]
        .into_iter()
        .zip(&person.animations)
    {
        animator
            .clear(slot, now, settings)
            .map_err(anyhow::Error::msg)?;
        let layer = load_one(
            index,
            program,
            skeleton,
            text,
            now,
            settings.sampling_config,
        )?;
        animator
            .restore_layer(slot, layer, now, settings)
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}
