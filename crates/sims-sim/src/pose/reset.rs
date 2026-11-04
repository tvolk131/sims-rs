//! Shared represented state of native animator Reset (589220). Appearance
//! reload is a separate subsequent service. Geometry invalidation and the route
//! diagnostic write are returned to the host, not silently acknowledged here.
use super::{animator::Settings, movement::Movement, ordinary, refresh::Character};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[must_use = "the character host must invalidate geometry and consume the route diagnostic"]
pub struct Effects {
    /// Increment native skin +ac, even if no layer was present.
    pub geometry_generation_delta: u32,
    /// Native 589e2d writes global 6f6d68 only in its immediate-stop branch.
    pub route_diagnostic: Option<i32>,
    /// Rig movement anchors +168/+16c are cleared. Retained measurements and
    /// the named head/hand/pelvis anchors are not reset.
    pub clear_movement_anchors: bool,
}

pub fn apply(
    character: &mut Character,
    control: &mut ordinary::State,
    movement: &mut Movement,
    now: i32,
    settings: Settings,
) -> Result<Effects, String> {
    if character.busy() {
        return Err("native animator reset during a pending update".into());
    }
    apply_storage(
        &mut character.skin,
        &mut character.placement,
        control,
        movement,
        now,
        settings,
    )
}

/// The same reset is used before a new character has a defined head target.
pub(super) fn apply_storage(
    skin: &mut super::live::Live,
    placement: &mut super::transform::Placement,
    control: &mut ordinary::State,
    movement: &mut Movement,
    now: i32,
    settings: Settings,
) -> Result<Effects, String> {
    if skin.busy() || movement.busy() {
        return Err("native animator reset during a pending update".into());
    }
    if settings.fade_enabled && settings.fade_duration < 0 {
        return Err("negative native animator fade duration is not supported".into());
    }
    placement.invalidate();
    placement.offset = [0.; 3];
    control.base_posture = -1;
    control.carry.selection = -1;
    control.carry.mode = 0;
    control.mode = 3;
    let status = movement.state.status;
    movement.state.flags = [0; 2];
    movement.state.reason = 0;
    let immediate = status != 0 && !matches!(status, -1 | 1);
    if let Some(path) = &mut movement.path {
        path.index = -1;
        path.deadline = 0;
        path.guard = 0;
        if immediate {
            path.stop_mode = 1;
        }
        if matches!(status, -1 | 1) {
            path.interrupt = 1;
        }
    }
    if immediate {
        placement.previous = placement.base;
    }
    movement.state.status = 0;
    skin.animator.reset(now, settings)?;
    // Head clock, angles, target, local bones, model placement, retained rate,
    // movement samples and phase/distance measurements survive this routine.
    Ok(Effects {
        geometry_generation_delta: 1,
        route_diagnostic: immediate.then_some(7),
        clear_movement_anchors: true,
    })
}
