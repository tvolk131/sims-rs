//! Original 590430 base-layer selection, shared by ordinary updates, movement
//! settling and Animate Sim completion. Asset decoding remains a host service.
use super::{
    animator::{Animator, Settings, Slot},
    ordinary,
};
use sims_formats::{animation::Animation, avatar::Skeleton};

#[derive(Debug, Clone, Copy)]
pub struct Input {
    pub posture: i16,
    pub species: i16,
    pub definition_flags: u8,
    pub tick: i32,
}

fn index(posture: i16) -> Option<u32> {
    match posture {
        0 => Some(3),
        1 => Some(1),
        2 => Some(2),
        _ => None,
    }
}

/// Read-only prediction of the resource request, accounting for the preceding
/// stale-layer clear. This discovers assets without acknowledging a service.
pub fn selection(animator: &Animator, state: &ordinary::State, input: Input) -> Option<u32> {
    let retained =
        state.base_posture == i32::from(input.posture) && animator.slot(Slot::Base).is_some();
    if input.definition_flags & 4 == 0 && (!retained || input.species > 1) {
        index(input.posture)
    } else {
        None
    }
}

/// Execute the clear/gate/cache-write prefix. The cache commits before resource
/// lookup, including an unsupported posture or an actually absent resource.
pub fn prepare(
    animator: &mut Animator,
    state: &mut ordinary::State,
    input: Input,
    settings: Settings,
) -> Result<Option<u32>, String> {
    if state.base_posture != i32::from(input.posture) {
        animator.clear(
            Slot::Base,
            ordinary::clock(input.tick, state.clock_mode),
            settings,
        )?;
    }
    if input.definition_flags & 4 == 0 && (animator.slot(Slot::Base).is_none() || input.species > 1)
    {
        state.base_posture = i32::from(input.posture);
        Ok(index(input.posture))
    } else {
        Ok(None)
    }
}

/// Complete a selected lookup. None is native resource absence, not an asset
/// awaiting decoding. A present clip whose start leaves no layer is an error.
pub fn selected(
    animator: &mut Animator,
    state: &ordinary::State,
    tick: i32,
    skeleton: &Skeleton,
    clip: Option<&Animation>,
    ready: bool,
    settings: Settings,
) -> Result<(), String> {
    if let Some(clip) = clip {
        if ready {
            animator.start(
                Slot::Base,
                skeleton,
                Some(clip),
                false,
                ordinary::clock(tick, state.clock_mode),
                settings,
            )?;
        }
        let id = animator
            .slot(Slot::Base)
            .ok_or("native base start left a null layer")?;
        animator.repeat(id)?;
        animator.set_priority(id, -10)?;
    }
    Ok(())
}

/// Synchronous execution. Hosts stage mutations while resolving an unloaded
/// asset so a retry cannot clear/fade the old layer a second time.
pub fn update<'a>(
    animator: &mut Animator,
    state: &mut ordinary::State,
    input: Input,
    skeleton: &Skeleton,
    settings: Settings,
    mut lookup: impl FnMut(u32) -> Result<Option<&'a Animation>, String>,
) -> Result<(), String> {
    if let Some(index) = prepare(animator, state, input, settings)? {
        selected(
            animator,
            state,
            input.tick,
            skeleton,
            lookup(index)?,
            true,
            settings,
        )?;
    }
    Ok(())
}
