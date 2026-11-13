//! Base Animator Reset 584d76. No person movement, head or carry-state reset.
use super::{animator::Settings, graphic_transform::Placement, model::Bound};

pub fn apply(
    bound: &mut Bound,
    placement: &mut Placement,
    flags: &mut [u8; 4],
    engaged: &mut i32,
    now: i32,
    settings: Settings,
) -> Result<(), String> {
    if bound.skeleton.skin.busy() {
        return Err("base Animator reset during a skin pass".into());
    }
    if settings.fade_enabled && settings.fade_duration < 0 {
        return Err("negative base Animator fade duration is unsupported".into());
    }
    placement.invalidate();
    placement.offset = [0.; 3];
    *flags = [1; 4];
    bound.skeleton.geometry.generation = bound.skeleton.geometry.generation.wrapping_add(1);
    bound.skeleton.skin.animator.reset_graphic(now, settings)?;
    *engaged = 0;
    Ok(())
}
