//! Where a track's objects are, as its volume and pan: the original's
//! 3D placement (0x5730b6) and the sum over a track's objects
//! (0x57473b).
//!
//! When kind 1 starts a `kArgsIdVolPan` track, and whenever kind 30
//! turns a set on, the original records the object as an instance of the
//! track and then works out one volume and one pan from every object the
//! track has, which it writes to the track's `extvol` (19) and `pan`
//! (20) after starting it (0x573795 to 0x5737a4, 0x573d5d to 0x573d6e).
//!
//! For each object, the placement:
//!
//! - gives full volume (1024) and the centre (512) for an ID that is
//!   not above 0;
//! - leaves the volume and pan as they were for an ID that names no
//!   object, and for an object whose first or second coordinate is 0
//!   (0x5730f8 to 0x573100). The coordinates are the object's +0xf8,
//!   +0xf4 and +0x110 (0x570dc9), which are
//!   `sims_sim::Object::position` 0, 1 and 2;
//! - otherwise gives full volume and the centre when the track's `Spl`
//!   (register 57) is 0, and when it is not, works them out from where
//!   the object is on the screen ([`project`]).
//!
//! A track's `Spl` starts at 20, `spl_normal` (the default bank at
//! 0x6f6898, `crate::player::DEFAULTS`), and no script in the install
//! writes it, so every object on the lot is placed from where it is on
//! the screen: the host gives a [`Projection`] for each. At the far zoom
//! a sound loses 560 of its 1024 and at the medium 280; the pan follows
//! how far across the screen the object is, and the volume fades for an
//! object off the middle of the screen and drops to three fifths for one
//! on a floor the view does not show.
//!
//! The sum (0x57473b) takes the loudest volume, and the pan of each
//! object weighted by its volume, in 32-bit arithmetic that wraps. The
//! volume and pan an object leaves as they were are the previous
//! object's; for the first, the pan is 0 and the volume is whatever a
//! local held before (0x57475f: a word of the instance lookup's result,
//! a heap address). Here that volume is [`STALE_VOL`], **unverified**. A
//! track with no instance gets volume 0 and the centre.
//!
//! The volume and pan are worked out when the event comes; here, at the
//! next block, when the player has the game's [`Sources`], from the
//! objects and `Spl` as they were when the event came.
//!
//! Not ported: the original places every track again (0x57447f, each
//! track through 0x5746d9, not read) on kind 33, which its game sends
//! when the camera moves. The game here sends kind 33 every frame
//! (`Player::set_ambience_view`), where placing again would undo a
//! fading `extvol` each frame; it waits for the game to send it as the
//! original does.
//!
//! The original keeps the instances in a hash multimap and goes through
//! a track's in its order; here they go in object ID order, which only
//! matters when an object is left as it was. **Unverified.**

use crate::player::Sources;

/// Full volume.
pub const FULL: i32 = 1024;
/// The pan's centre.
pub const CENTRE: i32 = 512;
/// The volume the sum starts from before its first object is placed
/// (see the module notes). **Unverified.**
pub const STALE_VOL: i32 = 1024;

/// Where an object is on the screen, as the placement reads it when a
/// track's `Spl` is not 0 (which it starts as).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Projection {
    /// The object's position on the screen, in pixels (0x570e09, through
    /// the view's projection).
    pub at: [i32; 2],
    /// The screen's width and height (0x570c23).
    pub screen: [i32; 2],
    /// The zoom (0x570c11): 1 far, 2 medium, 3 near.
    pub zoom: i32,
    /// The level the view shows (0x570c39).
    pub shown_level: i32,
}

/// One object's volume and pan, as 0x5730b6 gives them for a track whose
/// `Spl` is `spl`. `Ok(None)` leaves the previous values. An error when
/// `Spl` is not 0 and the host gives no [`Projection`], or the screen is
/// one the original would divide by 0 for.
pub fn place(object: i32, spl: i32, sources: &dyn Sources) -> Result<Option<(i32, i32)>, String> {
    if object <= 0 {
        return Ok(Some((FULL, CENTRE)));
    }
    let Some([x, y, level]) = sources.position(object) else {
        return Ok(None);
    };
    if x == 0 || y == 0 {
        return Ok(None);
    }
    if spl == 0 {
        return Ok(Some((FULL, CENTRE)));
    }
    let Some(view) = sources.projection(object) else {
        return Err(format!(
            "object {object}: its track's Spl is {spl}, and the host gives no screen position"
        ));
    };
    project(spl, level, &view).map(Some)
}

/// The placement's volume and pan for an object on `level` at `view`,
/// for a track whose `Spl` is not 0 (0x573154 to 0x57328c).
pub fn project(spl: i32, level: i32, view: &Projection) -> Result<(i32, i32), String> {
    let [x, y] = view.at;
    let [width, height] = view.screen;
    let divide = |a: i32, b: i32| {
        a.checked_div(b)
            .ok_or_else(|| format!("the placement divides {a} by {b}"))
    };
    // The pan: how far right of the middle, against half the width.
    let middle = width / 2;
    let right = x.wrapping_sub(middle).wrapping_mul(3).wrapping_shl(10);
    let pan = divide(right, width.wrapping_mul(100) / 2)?.wrapping_add(CENTRE);
    let pan = pan.clamp(0, FULL);
    // The volume: less the farther the zoom, but not below the floor.
    let floor = if spl == 10_001 { 800 } else { 0 };
    let drop = 3i32
        .wrapping_sub(view.zoom)
        .wrapping_mul(spl)
        .wrapping_mul(14);
    let near = FULL.wrapping_sub(drop).max(0).max(floor);
    // How far outside the middle of the screen: past a fifth of the
    // width from either side, below the bottom, or above three tenths of
    // the height.
    let edge = divide(width.wrapping_mul(20), 100)?;
    let across = if x < edge {
        edge.wrapping_sub(x)
    } else {
        edge.wrapping_sub(width).wrapping_add(x)
    };
    let below = y.wrapping_sub(height);
    let down = if below > 0 {
        below
    } else {
        divide(height.wrapping_mul(30), 100)?.wrapping_sub(y).max(0)
    };
    let out = across.max(down);
    let mut vol = near;
    if out != 0 {
        vol = if out >= 600 {
            floor
        } else {
            let faded = near.wrapping_sub(floor).wrapping_mul(600 - out) / 600;
            faded.wrapping_add(floor).clamp(floor, FULL)
        };
    }
    if level != view.shown_level {
        vol = vol.wrapping_mul(3) / 5;
    }
    Ok((vol, pan))
}

/// The volume and pan of a track whose objects are `objects`, in order,
/// and whose `Spl` is `spl` (0x57473b).
pub fn combine(objects: &[u32], spl: i32, sources: &dyn Sources) -> Result<(i32, i32), String> {
    if objects.is_empty() {
        return Ok((0, CENTRE));
    }
    let (mut vol, mut pan) = (STALE_VOL, 0);
    let (mut loudest, mut total, mut weighted) = (0i32, 0i32, 0i32);
    for &object in objects {
        if let Some((v, p)) = place(object as i32, spl, sources)? {
            (vol, pan) = (v, p);
        }
        loudest = loudest.max(vol);
        total = total.wrapping_add(vol);
        weighted = weighted.wrapping_add(pan.wrapping_sub(CENTRE).wrapping_mul(vol));
    }
    let mean = if total == 0 {
        0
    } else {
        weighted
            .checked_div(total)
            .ok_or_else(|| format!("the sum divides {weighted} by {total}"))?
    };
    Ok((loudest, mean.wrapping_add(CENTRE)))
}
