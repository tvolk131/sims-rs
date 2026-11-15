//! Which object a click on the lot lands on.
//!
//! `sims-render` does not say which object drew a pixel, so this guesses
//! from where each object stands: every object is drawn from one tile
//! (a multi-tile object is one object per tile), and its sprite covers
//! that tile's diamond and rises above it. A click inside a tile's
//! diamond picks the frontmost object on that tile; otherwise the
//! frontmost object whose tile the click is above, within [`RISE`] of a
//! storey, is picked. A short object in front of a tall one can take a
//! click meant for the tall one. Drawing an object ID per pixel in
//! `sims-render` would make this exact.

use sims_render::{Camera, Lot};
use sims_sim::World;

/// How high above its tile an object is taken to reach, in storeys.
const RISE: f32 = 0.75;

/// Object data word 34: nonzero hides the object, and `sims-render`
/// does not draw it.
const HIDDEN: usize = 34;

/// The object a press at `point` (canvas pixels) lands on, if any.
/// People are left out: [`crate::people::People::at`] finds them.
pub fn object(world: &World, lot: &Lot, camera: &Camera, point: [i32; 2]) -> Option<u16> {
    let width = camera.zoom.tile_width();
    let height = camera.zoom.tile_height();
    let rise = (camera.zoom.floor_height() as f32 * RISE) as i32;
    // (inside the diamond, how near the viewer, ID): the largest wins.
    let mut best: Option<(bool, i32, u16)> = None;
    for (&id, object) in &world.objects {
        let [x, y, level] = object.position;
        if object.person.is_some()
            || object.data.get(HIDDEN).is_some_and(|&w| w != 0)
            || x < 0
            || y < 0
            || level < 1
            || level > camera.level
        {
            continue;
        }
        let tile = camera.rotate_tile([x >> 4, y >> 4]);
        // Raised by the terrain at the tile's middle, as the lot draws it.
        let ground = terrain(lot, [(x & !15) as f32 + 8.0, (y & !15) as f32 + 8.0]) as i32;
        let [left_x, top] = camera.project(tile, level, 0);
        let top = top - ground / camera.zoom.near_scale();
        // The tile's diamond: its top corner at `top`, `width` across and
        // `height` down.
        let dx = point[0] - left_x;
        let dy = point[1] - top;
        if dx.abs() > width / 2 || dy > height || dy < -rise {
            continue;
        }
        let inside = dy >= 0 && 2 * dx.abs() * height <= width * dy.min(height - dy) * 2;
        let near = tile[0] + tile[1] + 128 * level;
        let candidate = (inside, near, id);
        if best.is_none_or(|b| (candidate.0, candidate.1) > (b.0, b.1)) {
            best = Some(candidate);
        }
    }
    best.map(|(_, _, id)| id)
}

/// The terrain's height under the fine position `fine`, in near-zoom
/// pixels (16 a terrain step, as `sims-render` raises the ground):
/// the tile's four corners, blended by where in the tile the position
/// is. Off the lot it is 0.
pub fn terrain(lot: &Lot, fine: [f32; 2]) -> f32 {
    let [x, y] = fine.map(|v| v / 16.0);
    let (tx, ty) = (x.floor(), y.floor());
    let size = lot.size as f32;
    if !(0.0..size).contains(&tx) || !(0.0..size).contains(&ty) {
        return 0.0;
    }
    let (fx, fy) = (x - tx, y - ty);
    // A tile's heights are its corners (0, 1), (0, 0), (1, 0) and (1, 1),
    // in that order (`sims_render::scene`).
    let h = lot.heights[Lot::cell(tx as usize, ty as usize)].map(f32::from);
    let height = h[1] * (1.0 - fx) * (1.0 - fy)
        + h[2] * fx * (1.0 - fy)
        + h[0] * (1.0 - fx) * fy
        + h[3] * fx * fy;
    height * 16.0
}
