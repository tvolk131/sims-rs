//! Which person or object the pointer is on: the original's pointer pick
//! (0x48c9be) on the frame last drawn.
//!
//! 1. **A person or the plumb bob drawn at the pixel.** The original's
//!    3D graphics note themselves while they are drawn: a person's draw
//!    (0x590ebd to 0x590f84) and the bob's (0x5863eb to 0x586455) read the
//!    depth buffer at the pointer before and after drawing the mesh, and
//!    the one that changed it is the graphic under the pointer, the last
//!    drawn winning. The pick takes it when it stands on the floor shown
//!    (+0x110 against 0x49295d). Here, the person or bob whose pixel the
//!    frame shows there ([`Canvas::id_at`]: `sims_people` names each
//!    person's pixels and the bob's): the pixel itself, with no reach
//!    around it.
//! 2. **Otherwise the sprite pick** (0x4aa713, [`Lot::sprite_pick`]):
//!    from the tile the lot's depth at the pixel stands for, the tiles
//!    toward the viewer, and the first whose objects' screen rectangles
//!    hold the point ([`Canvas::drawn`], widened to whole groups of four
//!    columns as the original keeps them, 0x438f71).
//!
//! The depth the original reads at the pointer is the lot's alone: a
//! dump with Sims on screen shows them in the colour buffer only. So the
//! sprite pick unprojects the depth the lot left before its people were
//! drawn ([`Depth::unproject`]). Each tile of a multi-tile object is its
//! own object and is picked as itself, not as its group's lead: that is
//! the object the pie menu is listed on, and the simulation sends the
//! interactions of a tile on to the tile that takes them, as the
//! original does (`crate::menu::target`, 0x435286).

use sims_render::depth::{Depth, EMPTY};
use sims_render::{Camera, Canvas};
use sims_sim::World;
use sims_vm::boot::Lot;

/// The person or object the pointer at `point` (canvas pixels) is on in
/// `canvas`, the frame last drawn of `lot` through `camera`, whose lot
/// left `lot_depth` (a depth per pixel, in rows of the canvas's width)
/// before its people were drawn: the person or bob drawn at the pixel
/// ([`noted`]), else the sprite pick's object.
pub fn at(
    lot: &Lot,
    camera: &Camera,
    canvas: &Canvas,
    lot_depth: &[i32],
    point: [i32; 2],
) -> Option<u16> {
    noted(lot.world(), camera, canvas, point)
        .or_else(|| sprite(lot, camera, canvas, lot_depth, point))
}

/// The person or plumb bob drawn at `point` in `canvas`, if it stands on
/// the floor `camera` shows.
pub fn noted(world: &World, camera: &Camera, canvas: &Canvas, [x, y]: [i32; 2]) -> Option<u16> {
    let id = canvas.id_at(x, y)?;
    let object = world.objects.get(&id)?;
    let three_d = object.person.is_some() || object.guid == sims_people::bob::STANDARD;
    (three_d && object.position[2] == camera.level).then_some(id)
}

/// The sprite pick's object at `point` ([`Lot::sprite_pick`]).
fn sprite(
    lot: &Lot,
    camera: &Camera,
    canvas: &Canvas,
    lot_depth: &[i32],
    point: [i32; 2],
) -> Option<u16> {
    let [x, y] = point;
    let on_canvas = (0..canvas.width as i32).contains(&x) && y >= 0;
    let depth = on_canvas
        .then(|| lot_depth.get(y as usize * canvas.width + x as usize))
        .flatten()
        .copied()
        .filter(|&d| d != EMPTY)
        .map(|d| place(camera, point, d));
    let level = camera.level;
    let lifted = [x, y + camera.zoom.floor_height() * (level - 1)];
    let ground = camera.unproject_fine(lifted);
    let level = i8::try_from(level).ok()?;
    lot.sprite_pick(depth, ground, level, point, |id| {
        canvas.drawn(id).map(widen)
    })
}

/// Where a pixel of depth `depth` at `point` is: its lot position
/// (sixteenths of a tile, rounded half away from zero as 0x454fb9 rounds)
/// and the level its height is on. The level is one more for each
/// floor's height the point stands above level-1 ground, as the
/// original's 0x4b07b3 counts it against the floor height (which here
/// leaves out the terrain's height under the point: **unverified** on a
/// sloped lot).
fn place(camera: &Camera, point: [i32; 2], depth: i32) -> ([i32; 2], i32) {
    let (view, height) = Depth::new(camera).unproject(camera, point, depth);
    let span = f64::from(sims_render::camera::ARRAY_SIZE * 16);
    let [vx, vy] = view;
    let lot = match camera.rotation & 3 {
        1 => [vy, span - vx],
        2 => [span - vx, span - vy],
        3 => [span - vy, vx],
        _ => [vx, vy],
    };
    let level = 1 + (height / f64::from(camera.zoom.floor_height())).floor() as i32;
    (lot.map(round), level)
}

/// Rounds half away from zero (0x636ac9, 0x636b98).
fn round(v: f64) -> i32 {
    let whole = if v >= 0.0 {
        (v + 0.5).floor()
    } else {
        -(-v + 0.5).floor()
    };
    whole as i32
}

/// A drawn rectangle as the original keeps an object's (0x438f71): its
/// left edge moved left and its right edge right to a multiple of four.
fn widen([left, top, right, bottom]: [i32; 4]) -> [i32; 4] {
    [
        left - left.rem_euclid(4),
        top,
        right + (4 - right.rem_euclid(4)) % 4,
        bottom,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_is_half_away_from_zero() {
        assert_eq!(round(415.5), 416);
        assert_eq!(round(415.49), 415);
        assert_eq!(round(-0.5), -1);
        assert_eq!(round(-0.49), 0);
    }

    #[test]
    fn a_rectangle_widens_to_groups_of_four_columns() {
        assert_eq!(widen([465, 250, 482, 308]), [464, 250, 484, 308]);
        assert_eq!(widen([464, 1, 484, 2]), [464, 1, 484, 2]);
        assert_eq!(widen([-3, 0, -1, 0]), [-4, 0, 0, 0]);
    }
}
