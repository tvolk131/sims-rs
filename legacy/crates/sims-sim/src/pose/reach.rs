//! Native Reach animator helpers (589ef3, 58a000, 58a00d).
//!
//! The host prepares the runtime slot height before selection and completes any
//! requested profile reload before resolving the selected STR# entry. Transfer
//! checks, placement and CT - Grab/Put callbacks belong to the primitive driver.
use super::{
    animator::{Animator, Slot},
    ordinary,
};
use crate::character::Controls;
use serde::{Deserialize, Serialize};
use sims_formats::{animation::Animation, avatar::Skeleton};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub strings: u16,
    /// Zero-based entry; native StringSet lookup receives index + 1.
    pub index: i16,
    pub reload_profile: bool,
}
impl Selection {
    /// 5888d3 can rewrite person[58]. Keep the already-selected height index,
    /// but 59dcd4's table choice observes the completed profile reload.
    pub fn reread_age(&mut self, age: i16) {
        self.strings = if age <= 17 { 155 } else { 154 };
    }
}

/// 589ef3 through the 59dcd4 lookup boundary. The carried-object query does not
/// affect 59265c's result. An invalid height selection leaves +d8 untouched;
/// otherwise +d8 is cleared BEFORE the profile reload service is called.
pub fn prepare(height: f32, posture: i16, age: i16, profile_dirty: &mut bool) -> Option<Selection> {
    let index = sims_vm::reach::animation_index(height, posture);
    if index == -1 {
        return None;
    }
    Some(Selection {
        strings: if age <= 17 { 155 } else { 154 },
        index,
        reload_profile: std::mem::take(profile_dirty),
    })
}

/// An unresolved host resource must suspend the host operation, not become
/// Absent. NotReady represents a real negative native skill-readiness result.
pub enum Resource<'a> {
    Absent,
    NotReady,
    Ready(&'a Animation),
}

/// Remaining 589ef3 after lookup. An absent skill preserves queued events; a
/// present skill clears them even if native layer creation cannot start it.
/// Foreground replacement owns its usual fade. Carry and base remain attached
/// here; the subsequent ordinary animator pass observes pickup = 1.
pub fn start_selected(
    animator: &mut Animator,
    controls: &mut Controls,
    skeleton: &Skeleton,
    resource: Resource<'_>,
    forward: u8,
    tick: i32,
) -> Result<bool, String> {
    let clip = match resource {
        Resource::Absent => return Ok(false),
        Resource::NotReady => None,
        Resource::Ready(clip) => Some(clip),
    };
    animator.clear_numbered_events()?;
    if !animator.start(
        Slot::Foreground,
        skeleton,
        clip,
        forward == 0,
        ordinary::clock(tick, controls.ordinary.clock_mode),
        controls.settings,
    )? {
        return Ok(false);
    }
    animator.set_selection(-1)?;
    controls.ordinary.mode = 0;
    controls.ordinary.pickup = 1;
    Ok(true)
}

/// 58a000 does not consume events or advance the animation clock.
pub fn finished(animator: &Animator) -> Result<bool, String> {
    animator.finished(Slot::Foreground)
}

/// 58a00d clears the foreground, pickup flag and event FIFO, then restores only
/// the ordinary mode from posture. It does not synchronously rebuild the base.
pub fn finish(
    animator: &mut Animator,
    controls: &mut Controls,
    posture: i16,
    tick: i32,
) -> Result<(), String> {
    animator.clear_foreground(
        ordinary::clock(tick, controls.ordinary.clock_mode),
        controls.settings,
    )?;
    controls.ordinary.pickup = 0;
    animator.clear_numbered_events()?;
    controls.ordinary.mode =
        sims_vm::animation::posture_mode(posture, controls.ordinary.mode as u32) as i32;
    Ok(())
}
