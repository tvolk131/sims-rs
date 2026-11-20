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

/// The sprite pick's object at `point` ([`Lot::sprite_pick`]). Nothing
/// before the first frame is drawn, or off the canvas.
fn sprite(
    lot: &Lot,
    camera: &Camera,
    canvas: &Canvas,
    lot_depth: &[i32],
    point: [i32; 2],
) -> Option<u16> {
    let [x, y] = point;
    let column = usize::try_from(x).ok().filter(|&x| x < canvas.width)?;
    let row = usize::try_from(y).ok().filter(|&y| y < canvas.height)?;
    let depth = *lot_depth.get(row * canvas.width + column)?;
    let world = lot.world();
    let depth = (depth != EMPTY).then(|| place(world, camera, point, depth));
    let level = camera.level;
    let lifted = [x, y + camera.zoom.floor_height() * (level - 1)];
    let ground = camera.unproject_fine(lifted);
    let level = i8::try_from(level).ok()?;
    lot.sprite_pick(depth, ground, level, camera.rotation, point, |id| {
        canvas.drawn(id).map(widen)
    })
}

/// Where a pixel of depth `depth` at `point` is: its lot position
/// (sixteenths of a tile, rounded half away from zero as 0x454fb9 rounds)
/// and the level it is on. The original's 0x4b07b3 classifies a point by
/// its height above the terrain under it (the mean of its tile's four
/// corners): two floors' height or more is level 3, one or more level 2,
/// anything lower level 1. Our depth gives a position only for a level
/// taken, so each level is tried, and the answer is the highest whose
/// position and height classify as that level (only an upper floor's
/// depth carries its level's offset). With none, it is the level the
/// first level's position classifies as, at that level's position.
fn place(world: &World, camera: &Camera, point: [i32; 2], depth: i32) -> ([i32; 2], i32) {
    let depths = Depth::new(camera);
    let floor = f64::from(camera.zoom.floor_height());
    let step = f64::from(2i32 << camera.zoom.level());
    let on = |level: i32| {
        let (view, height) = depths.unproject(camera, point, depth, level);
        let fine = to_lot(camera, view).map(round);
        let above = floor * f64::from(level - 1) + height - terrain(world, fine) * step;
        (fine, classify(above, floor))
    };
    (1..=3)
        .rev()
        .map(|level| (level, on(level)))
        .find(|&(level, (_, classified))| level == classified)
        .map_or_else(
            || {
                let (_, level) = on(1);
                (on(level).0, level)
            },
            |(_, place)| place,
        )
}

/// The level of a point `above` pixels above the terrain, with floors
/// `floor` pixels high (0x4b07b3).
fn classify(above: f64, floor: f64) -> i32 {
    if above >= 2.0 * floor {
        3
    } else if above >= floor {
        2
    } else {
        1
    }
}

/// A fine view position turned back to the lot's, as the camera turns it.
fn to_lot(camera: &Camera, [x, y]: [f64; 2]) -> [f64; 2] {
    let span = f64::from(sims_render::camera::ARRAY_SIZE * 16);
    match camera.rotation & 3 {
        1 => [y, span - x],
        2 => [span - x, span - y],
        3 => [span - y, x],
        _ => [x, y],
    }
}

/// The terrain's height under lot position `fine`, in terrain steps: the
/// mean of its tile's four corner heights (the lot's array 0), 0 off it.
fn terrain(world: &World, fine: [i32; 2]) -> f64 {
    let full = sims_render::lot::FULL;
    let tile = fine.map(|n| usize::try_from(n >> 4).ok().filter(|&n| n < full));
    let ([Some(x), Some(y)], Some(heights)) = (tile, world.arrays.get(&0)) else {
        return 0.0;
    };
    let at = sims_render::Lot::cell(x, y) * 4;
    heights
        .data
        .get(at..at + 4)
        .filter(|_| heights.stride == 4)
        .map_or(0.0, |c| c.iter().map(|&h| f64::from(h)).sum::<f64>() / 4.0)
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
        left.saturating_sub(left.rem_euclid(4)),
        top,
        right.saturating_add((4 - right.rem_euclid(4)) % 4),
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
    fn a_point_below_the_first_floor_height_is_on_level_one() {
        assert_eq!(classify(-9.6, 58.0), 1);
        assert_eq!(classify(57.9, 58.0), 1);
        assert_eq!(classify(58.0, 58.0), 2);
        assert_eq!(classify(116.0, 58.0), 3);
        assert_eq!(classify(400.0, 58.0), 3);
    }

    #[test]
    fn a_rectangle_widens_to_groups_of_four_columns() {
        assert_eq!(widen([465, 250, 482, 308]), [464, 250, 484, 308]);
        assert_eq!(widen([464, 1, 484, 2]), [464, 1, 484, 2]);
        assert_eq!(widen([-3, 0, -1, 0]), [-4, 0, 0, 0]);
        assert_eq!(
            widen([i32::MIN, 0, i32::MAX, 0]),
            [i32::MIN, 0, i32::MAX, 0]
        );
    }
}
