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

use sims_render::Camera;
use sims_sim::World;
use sims_sim::animator::UNSET;

/// How high above its tile an object is taken to reach, in storeys.
const RISE: f32 = 0.75;

/// The object a press at `point` (canvas pixels) lands on, if any.
/// People are left out: they are picked on their own.
pub fn object(world: &World, camera: &Camera, point: [i32; 2]) -> Option<u16> {
    let width = camera.zoom.tile_width();
    let height = camera.zoom.tile_height();
    let rise = (camera.zoom.floor_height() as f32 * RISE) as i32;
    // (inside the diamond, how near the viewer, ID): the largest wins.
    let mut best: Option<(bool, i32, u16)> = None;
    for (&id, object) in &world.objects {
        let [x, y, level] = object.position;
        if object.person.is_some() || x < 0 || y < 0 || level < 1 || level > camera.level {
            continue;
        }
        let tile = camera.rotate_tile([x >> 4, y >> 4]);
        let [left_x, top] = camera.project(tile, level, 0);
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

/// The person a press at `point` lands on, if any: the frontmost whose
/// standing box (half a tile wide, most of a storey tall, over where it
/// stands) holds the point.
pub fn person(world: &World, camera: &Camera, point: [i32; 2]) -> Option<u16> {
    let half = camera.zoom.tile_width() / 4;
    let tall = (camera.zoom.floor_height() as f32 * 0.8) as i32;
    let mut best: Option<(i32, u16)> = None;
    for (&id, object) in &world.objects {
        if object.person.is_none() {
            continue;
        }
        let Some(animator) = world.animators.get(&id) else {
            continue;
        };
        if animator.position.contains(&UNSET) {
            continue;
        }
        let [x, y, _] = animator.position.map(|v| v.to_f32());
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
            continue;
        }
        let level = object.position[2].max(1);
        if level > camera.level {
            continue;
        }
        let fine = [x as i32, y as i32];
        let view = camera.rotate_fine(fine);
        let [sx, sy] = camera.project_fine(view, level, 0);
        let (dx, dy) = (point[0] - sx, sy - point[1]);
        if dx.abs() > half || dy < -half / 2 || dy > tall {
            continue;
        }
        let near = view[0] + view[1] + 2048 * level;
        if best.is_none_or(|b| near > b.0) {
            best = Some((near, id));
        }
    }
    best.map(|(_, id)| id)
}
