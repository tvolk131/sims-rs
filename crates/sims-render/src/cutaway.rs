//! Which walls the cutaway cuts, chosen as the original chooses.
//!
//! The original does not cut every wall in front of a room. It keeps a
//! mask of view tiles (view+0x208, 64 rows of 64 bits) built from the
//! rooms it is looking at, and cuts a wall only where the mask holds the
//! tiles on both sides of it (0x54059a). The rooms are:
//!
//! - the last three rooms the pointer passed over, newest first
//!   (view+0x1fc, a list of three that never holds a room twice:
//!   0x49e923). Each pointer move adds the room under the pointer and the
//!   first other room straight below it on screen (0x4ab7ee). The list is
//!   emptied when the floor changes (0x4a80c7);
//! - in live mode, the selected Sim's room, if the Sim is on the floor
//!   shown (0x4ab992).
//!
//! A room's part of the mask is its own tiles, plus every tile below one
//! of them on screen whose walls, standing, would overlap it (0x468203).
//! An outdoor room adds no tiles of its own. Instead, the selected Sim's
//! outdoor tile adds a square of up to 4 by 4 tiles towards the camera.
//! And if any of the rooms is outdoors, the pointer adds a 4 by 4 square
//! of tiles around it (0x4abdda).
//!
//! When the pointer leaves the lot, the original empties the mask
//! (0x4ac07b) but keeps the list: the pointer's rooms count again once it
//! is back over the lot.
//!
//! A room's part is kept, not built afresh for each mask: each room holds
//! its own (0x468203, at the room's +0x80), and the view's mask only
//! combines them (0x4ab992, at 0x4abafe). A room's part is built at the
//! zoom of the moment, for every room on the level shown when the view
//! turns (0x4a7f28, 0x4a8006) or the floor changes (0x4a8116), and for
//! the rooms of each level a room update counts, at its end (0x466c3c,
//! 0x466f0c). A change of zoom (0x4a7c9f) builds none, so until one of
//! those, the rooms keep the zoom they were built at
//! ([`Focus::masks_built_at`]). The squares are worked out at the zoom
//! shown.
//!
//! The old prototype ported these in
//! `legacy/crates/sims-sim/src/lot_presenter/{room_masks,visibility,hover,
//! inverse_projection}.rs` and `walls/cutaway.rs`; this follows them.
//! Where the pointer and the selected Sim are is input, given in
//! [`Focus`]: the renderer only reads the world.

use crate::camera::{ARRAY_SIZE, Camera, Zoom};
use crate::lot::{self, Lot};

/// How many pointer rooms the original remembers (the list's capacity, set
/// in the view's constructor at 0x4a6d96).
pub const ROOMS: usize = 3;

/// What the cutaway looks at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Focus {
    /// The last rooms the pointer passed over, newest first.
    pub rooms: [Option<i16>; ROOMS],
    /// The pointer left the lot: the rooms are kept but not looked at.
    pub away: bool,
    /// The pointer's screen point, if known.
    pub pointer: Option<[i32; 2]>,
    /// The selected Sim, in live mode.
    pub selected: Option<Selected>,
    /// The zoom the rooms' parts of the mask were last built at, if not
    /// the zoom shown: a change of zoom keeps them, and turning the view,
    /// changing the floor or a room update builds them again at the zoom
    /// shown. None for the zoom shown. Built at another zoom, a room's
    /// part can differ by a row of tiles: the overlap test takes 2 pixels
    /// off the room tile's lower edge at every zoom (0x46831c), so the next
    /// row down the screen just touches it at far zoom and overlaps it by 2
    /// pixels at medium and 6 at near.
    pub masks_built_at: Option<Zoom>,
}

/// Where the selected Sim is, as the simulation keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selected {
    /// Its lot tile.
    pub tile: [i32; 2],
    /// Its level, 1 or 2.
    pub level: i32,
    /// Its room. A Sim on a tile a diagonal wall splits is in one of the
    /// two rooms, so this is not the tile's room.
    pub room: i16,
}

impl Focus {
    /// Remembers a room the pointer passed over, as 0x49e923 does: a room
    /// already in the list stays where it is, and a new one goes first,
    /// pushing the oldest out.
    pub fn visit(&mut self, room: i16) {
        if self.rooms.contains(&Some(room)) {
            return;
        }
        self.rooms.rotate_right(1);
        self.rooms[0] = Some(room);
    }

    /// Forgets the pointer's rooms, as a change of floor does.
    pub fn forget(&mut self) {
        self.rooms = [None; ROOMS];
    }

    /// The pointer moved to screen point `at` (0x4ab7ee). Remembers the
    /// room under it and the first other room below it on screen, within
    /// half a storey, looked for every half tile. Off the lot, the rooms
    /// are kept but no longer looked at.
    pub fn point(&mut self, lot: &Lot, camera: &Camera, at: [i32; 2]) {
        // Points this far off are off the lot; the projection's arithmetic
        // is not meant for them.
        if !near(camera, at) {
            self.pointer = None;
            self.away = true;
            return;
        }
        self.pointer = Some(at);
        let view = View::new(lot, camera);
        // The room under a screen point: none off the lot's inner tiles,
        // and none (but on the lot) on a tile a diagonal splits.
        let room = |y: i32| -> Option<Option<i16>> {
            let fine = view.fine([at[0], y])?;
            let tile = fine.map(|f| f >> 4);
            view.interior(tile).then(|| {
                let [x, y] = camera.unrotate_tile(tile);
                room_of(lot, view.level, [x, y])
            })
        };
        let Some(first) = room(at[1]) else {
            self.away = true;
            return;
        };
        self.away = false;
        let step = camera.zoom.tile_height() / 2;
        let mut below = first;
        let mut offset = step;
        while offset < camera.zoom.floor_height() / 2 {
            // A point off the lot keeps the room found so far.
            if let Some(found) = room(at[1] + offset) {
                below = found;
            }
            if below != first {
                break;
            }
            offset += step;
        }
        for room in [first, below].into_iter().flatten() {
            self.visit(room);
        }
    }
}

/// Whether screen point `at` is near enough the lot's origin to be on it,
/// and both small enough that the projection's arithmetic cannot overflow.
fn near(camera: &Camera, at: [i32; 2]) -> bool {
    const SMALL: i32 = 1 << 28;
    const FAR: i32 = 1 << 24;
    (0..2).all(|i| {
        let [a, o] = [at[i], camera.origin[i]];
        (-SMALL..SMALL).contains(&a)
            && (-SMALL..SMALL).contains(&o)
            && (-FAR..FAR).contains(&(a - o))
    })
}

/// The room of a whole tile, as 0x4899d5 gives it: none for a tile a
/// diagonal wall splits.
fn room_of(lot: &Lot, level: usize, [x, y]: [i32; 2]) -> Option<i16> {
    if !lot.contains(x, y) {
        return None;
    }
    let room = lot.levels[level].rooms.as_ref()?[Lot::cell(x as usize, y as usize)];
    (room >= 0).then_some(room)
}

/// The slope from screen point `b` to `a`, rise over run in 16.16, as the
/// original divides (0x4a955d). None where the run is zero.
fn ratio(a: [i32; 2], b: [i32; 2]) -> Option<i32> {
    (a[1] - b[1]).wrapping_shl(16).checked_div(a[0] - b[0])
}

/// The view tiles the cutaway may cut walls between.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    rows: [u64; ARRAY_SIZE as usize],
}

/// The view's tile bounds, inclusive: the lot's tiles turned into the view.
fn bounds(lot: &Lot, camera: &Camera) -> [i32; 4] {
    let last = lot.size as i32 - 1;
    let a = camera.rotate_tile([0, 0]);
    let b = camera.rotate_tile([last, last]);
    [
        a[0].min(b[0]),
        a[0].max(b[0]),
        a[1].min(b[1]),
        a[1].max(b[1]),
    ]
}

struct View<'a> {
    lot: &'a Lot,
    camera: &'a Camera,
    level: usize,
    bounds: [i32; 4],
}

impl<'a> View<'a> {
    fn new(lot: &'a Lot, camera: &'a Camera) -> Self {
        Self {
            lot,
            camera,
            level: (camera.level.clamp(1, 2) - 1) as usize,
            bounds: bounds(lot, camera),
        }
    }
}

impl View<'_> {
    /// The level shown, 1 or 2.
    fn storey(&self) -> i32 {
        self.level as i32 + 1
    }

    fn inside(&self, [x, y]: [i32; 2]) -> bool {
        let b = self.bounds;
        x >= b[0] && x <= b[1] && y >= b[2] && y <= b[3]
    }

    fn interior(&self, [x, y]: [i32; 2]) -> bool {
        let b = self.bounds;
        x > b[0] && x < b[1] && y > b[2] && y < b[3]
    }

    /// The heights of a view tile's corners, in terrain steps, in screen
    /// order: top, left... as the view turns them (0x479567).
    fn corners(&self, view: [i32; 2]) -> [i32; 4] {
        let [x, y] = self.camera.unrotate_tile(view);
        if !self.lot.contains(x, y) {
            return [0; 4];
        }
        let h = self.lot.heights[Lot::cell(x as usize, y as usize)];
        let rotation = usize::from(self.camera.rotation & 3);
        std::array::from_fn(|n| i32::from(h[(n + 4 - rotation) & 3]))
    }

    /// The height of a view tile's top corner, in terrain steps.
    fn top(&self, view: [i32; 2]) -> i32 {
        self.corners(view)[1]
    }

    /// A view tile's top corner on screen on `level`, raised by the
    /// terrain on the lot's inner tiles (0x4a9c50).
    fn corner(&self, p: [i32; 2], level: i32) -> [i32; 2] {
        let height = if self.interior(p) { self.top(p) } else { 0 };
        self.camera.project(p, level, height)
    }

    /// The view tile under screen point `at` on the level shown, by the
    /// original's search over the tiles' raised corners (0x4a955d). None
    /// far off the lot, where the search would not end.
    fn coarse(&self, at: [i32; 2]) -> Option<[i32; 2]> {
        let camera = self.camera;
        let level = self.storey();
        let x = (at[0] - camera.origin[0]) / 2;
        let y = at[1] - camera.origin[1];
        let shift = camera.zoom.level() + 3;
        let mut best = [(y + x) >> shift, (y - x) >> shift];
        // The first guess ignores height, which can lift a tile by up to
        // 255 quarter tiles plus a storey: allow for that much.
        if best
            .iter()
            .any(|v| !(-2 * ARRAY_SIZE..3 * ARRAY_SIZE).contains(v))
        {
            return None;
        }
        let project = |p: [i32; 2]| self.corner(p, level);
        let mut parity = usize::from(project(best)[0] >= at[0]);
        let mut scan = best;
        let mut budget = 4096;
        let mut spend = || {
            budget -= 1;
            (budget > 0).then_some(())
        };
        while scan[0] < 0 || scan[1] < 0 {
            spend()?;
            scan[parity & 1] += 1;
            parity += 1;
        }
        let upper = [self.bounds[1] + 1, self.bounds[3] + 1];
        if scan[0] < upper[0] && scan[1] < upper[1] {
            loop {
                spend()?;
                let axis = parity & 1;
                parity += 1;
                scan[axis] += 1;
                if scan[axis] >= upper[axis] {
                    break;
                }
                if project(scan)[1] < at[1] {
                    best = scan;
                }
            }
        }
        loop {
            spend()?;
            let p = project(best);
            if at[0] < p[0] {
                let edge = ratio(p, project([best[0], best[1] + 1]))?;
                if ratio(p, at)? <= edge {
                    break;
                }
                best[0] -= 1;
            } else if at[0] == p[0] {
                break;
            } else {
                let edge = ratio(p, project([best[0] + 1, best[1]]))?;
                if edge <= ratio(p, at)? {
                    break;
                }
                best[1] -= 1;
            }
        }
        Some(best)
    }

    /// The fine view position (sixteenths of a tile) under screen point
    /// `at` on the level shown: the tile from [`View::coarse`], then the
    /// spot within it on the terrain's slope (0x4a97c3 with flag one).
    fn fine(&self, at: [i32; 2]) -> Option<[i32; 2]> {
        let tile = self.coarse(at)?;
        let whole = tile.map(|t| t << 4);
        if !self.interior(tile) {
            return Some(whole);
        }
        let zoom = self.camera.zoom;
        let h = self.corners(tile);
        let center = self.corner(tile, 1);
        let left = at[0] < center[0];
        let width = zoom.tile_width() / 2;
        let side_x = center[0] + if left { -width } else { width };
        let raise = |steps: i32| steps << (zoom.level() + 1);
        let side_y = center[1] + raise(h[1] - h[if left { 0 } else { 2 }] + 2);
        let bottom_y = center[1] + raise(h[1] - h[3] + 4);
        let denominator = f64::from(center[0] - side_x);
        let first = f64::from(center[1] - side_y) / denominator;
        let second = f64::from(bottom_y - side_y) / denominator;
        let shifted_y = f64::from(at[1] + zoom.floor_height() * (self.storey() - 1));
        let mouse_x = f64::from(at[0]);
        let [side_x, side_y] = [side_x, side_y].map(f64::from);
        // The original keeps one result in a float and one in a double.
        let (stored, retained) = if first == second {
            (mouse_x, mouse_x)
        } else {
            let delta = second - first;
            (
                f64::from(
                    ((side_y - shifted_y - side_x * first + mouse_x * second) / delta) as f32,
                ),
                (shifted_y - side_y - mouse_x * first + side_x * second) / delta,
            )
        };
        let center_x = f64::from(center[0]);
        let width = f64::from(width);
        let (a, b) = if left {
            ((center_x - stored) / width, (retained - side_x) / width)
        } else {
            ((side_x - retained) / width, (stored - center_x) / width)
        };
        let part = |v: f64| {
            let v = v * 16.0;
            v.is_finite().then_some((v as i64 & 15) as i32)
        };
        Some([whole[0] | part(b)?, whole[1] | part(a)?])
    }

    /// A view tile's top corner on screen, without its height.
    fn project(&self, [x, y]: [i32; 2]) -> [i32; 2] {
        let shift = self.camera.zoom.level();
        [(x - y) * (8 << shift), (x + y) * (4 << shift)]
    }

    fn rise(&self, steps: i32) -> i32 {
        steps << (self.camera.zoom.level() + 1)
    }

    /// The box a standing wall on a view tile covers (0x4ac25d).
    fn standing(&self, p: [i32; 2]) -> [i32; 4] {
        let zoom = self.camera.zoom;
        let [x, y] = self.project(p);
        let y = y - self.rise(self.top(p));
        let half = zoom.tile_width() / 2;
        [
            x - half - 1,
            y - zoom.floor_height(),
            x + half - 1,
            y + zoom.tile_height(),
        ]
    }

    /// Marks a room's tiles and the tiles in front of them (0x468203).
    fn room(&self, id: i16, rows: &mut [u64; ARRAY_SIZE as usize]) {
        let Some(rooms) = self.lot.levels[self.level].rooms.as_ref() else {
            return;
        };
        if self.lot.outdoor_rooms.contains(&id) {
            return;
        }
        let size = self.lot.size as i32;
        let half = self.camera.zoom.tile_width() / 2;
        // A diagonal that stands upright on screen halves the tile's box.
        let upright = if self.camera.rotation & 1 == 0 {
            lot::ASCENDING
        } else {
            lot::DESCENDING
        };
        for x in 0..size {
            for y in 0..size {
                let cell = Lot::cell(x as usize, y as usize);
                let key = [self.level as u8 + 1, x as u8, y as u8];
                let triangle = self
                    .lot
                    .halves
                    .get(&key)
                    .and_then(|h| h.iter().find(|(_, r)| *r == id))
                    .map(|&(t, _)| t);
                if rooms[cell] != id && triangle.is_none() {
                    continue;
                }
                let tile = self.camera.rotate_tile([x, y]);
                let [sx, sy] = self.project(tile);
                let sy = if self.interior(tile) {
                    sy - self.rise(self.top(tile))
                } else {
                    sy
                };
                let mut own = [sx - half + 1, sy + 1, sx + half - 2, sy + half - 2];
                if self.lot.levels[self.level].walls[cell].segments & upright != 0
                    && let Some(t) = triangle
                {
                    // The triangle by the corner it holds, turned into the
                    // view: 1 is the left half on screen, 3 the right.
                    match ((t.wrapping_sub(self.camera.rotation).wrapping_sub(1)) & 3) + 1 {
                        1 => own[2] -= half,
                        3 => own[0] += half,
                        _ => {}
                    }
                }
                mark(rows, tile);
                // Walk down the screen: three tiles a row, until all three
                // are outside.
                let mut p = [tile[0], tile[1] + 1];
                loop {
                    let mut outside = 0;
                    for step in [[0, 0], [1, -1], [0, 1]] {
                        p = [p[0] + step[0], p[1] + step[1]];
                        if !self.interior(p) {
                            outside += 1;
                        } else if !contains(rows, p) && overlap(own, self.standing(p)) {
                            mark(rows, p);
                        }
                    }
                    if outside == 3 {
                        break;
                    }
                    p[1] += 1;
                }
            }
        }
    }
}

fn mark(rows: &mut [u64; ARRAY_SIZE as usize], [x, y]: [i32; 2]) {
    if (0..ARRAY_SIZE).contains(&x) && (0..ARRAY_SIZE).contains(&y) {
        rows[y as usize] |= 1 << x;
    }
}

fn contains(rows: &[u64; ARRAY_SIZE as usize], [x, y]: [i32; 2]) -> bool {
    (0..ARRAY_SIZE).contains(&x) && (0..ARRAY_SIZE).contains(&y) && rows[y as usize] >> x & 1 != 0
}

/// Whether two boxes overlap by more than an edge.
fn overlap(a: [i32; 4], b: [i32; 4]) -> bool {
    a[3] > b[1] && b[3] > a[1] && b[2] > a[0] && a[2] > b[0]
}

impl Mask {
    /// The mask for the floor the camera shows (0x4ab992).
    pub fn new(lot: &Lot, camera: &Camera) -> Self {
        let mut rows = [0u64; ARRAY_SIZE as usize];
        let view = View::new(lot, camera);
        let focus = camera.focus;
        // The rooms' parts as they were last built (see the module docs).
        let built = Camera {
            zoom: focus.masks_built_at.unwrap_or(camera.zoom),
            ..*camera
        };
        let rooms = View::new(lot, &built);
        let mut outdoors = false;
        let looked_at = if focus.away {
            &[][..]
        } else {
            &focus.rooms[..]
        };
        for &room in looked_at.iter().flatten() {
            rooms.room(room, &mut rows);
            outdoors |= lot.outdoor_rooms.contains(&room);
        }
        // The original skips the room numbers that mark split tiles.
        if let Some(s) = focus
            .selected
            .filter(|s| s.level == view.storey() && s.room >= 0)
        {
            rooms.room(s.room, &mut rows);
            let size = lot.size as i32;
            let [x, y] = s.tile;
            if lot.outdoor_rooms.contains(&s.room) {
                outdoors = true;
                if x > 0 && x < size - 1 && y > 0 && y < size - 1 {
                    let start = camera.rotate_tile([x, y]);
                    let end = (0..=4)
                        .rev()
                        .map(|d| [start[0] + d, start[1] + d])
                        .find(|&e| view.inside(e))
                        .unwrap_or(start);
                    square(&view, &mut rows, start, end);
                }
            }
        }
        // The pointer's square starts at the tile four tiles' height below
        // it on screen and reaches four tiles back up, clipped to the lot.
        if outdoors
            && let Some(at) = focus.pointer.filter(|&at| near(camera, at))
            && let Some(fine) = view.fine([at[0], at[1] + 4 * camera.zoom.tile_height()])
        {
            let mut start = fine.map(|f| f >> 4);
            let mut end = [start[0] - 4, start[1] - 4];
            while !view.inside(start) && start != end {
                start = [start[0] - 1, start[1] - 1];
            }
            while !view.inside(end) && end != start {
                end = [end[0] + 1, end[1] + 1];
            }
            if view.inside(end) {
                square(&view, &mut rows, start, end);
            }
        }
        Self { rows }
    }

    /// Whether the mask holds a view tile.
    pub fn contains(&self, view: [i32; 2]) -> bool {
        contains(&self.rows, view)
    }

    /// Whether the cutaway cuts the wall with segment bit `bit` of lot
    /// cell (x, y), by the original's test (0x54059a) in the view.
    pub fn cuts(&self, camera: &Camera, [x, y]: [i32; 2], bit: u8) -> bool {
        if !(0..ARRAY_SIZE).contains(&x) || !(0..ARRAY_SIZE).contains(&y) {
            return false;
        }
        let view = |t: [i32; 2]| camera.rotate_tile(t);
        let has = |t: [i32; 2]| self.contains(t);
        match bit {
            lot::WEST => has(view([x, y])) && has(view([x - 1, y])),
            lot::NORTH => has(view([x, y])) && has(view([x, y - 1])),
            _ => {
                let v = view([x, y]);
                // The diagonal runs down the screen ("\" in the view) when
                // it ascends at an even rotation or descends at an odd one.
                let ascending = bit == lot::ASCENDING;
                if ascending == (camera.rotation & 1 == 0) {
                    has(v) && has([v[0], v[1] - 2]) && has([v[0] - 2, v[1]])
                } else {
                    has([v[0], v[1] + 1]) && (has([v[0] - 1, v[1]]) || has([v[0], v[1] - 1]))
                }
            }
        }
    }
}

/// The flag in an object's data word 8 that makes it hide with the walls.
/// The objects' own scripts set it when the lot loads, so a house read
/// without running them lacks it.
const HIDES_WITH_WALLS: u16 = 0x400;

/// Whether an object hides with the walls, by its data words.
pub fn hides_with_walls(data: &[i16]) -> bool {
    data.get(8)
        .is_some_and(|&w| w as u16 & HIDES_WITH_WALLS != 0)
}

impl Mask {
    /// Whether the cutaway hides an object that hides with the walls
    /// (0x4aa446): the mask holds its tile, and the tile its facing side
    /// looks onto. `data` is the object's data words; its facing side is
    /// its placement (word 13) turned by its facing (word 1).
    pub fn hides(&self, camera: &Camera, tile: [i32; 2], data: &[i16]) -> bool {
        if !tile.iter().all(|v| (0..ARRAY_SIZE).contains(v)) {
            return false;
        }
        let word = |n: usize| data.get(n).copied().unwrap_or(0);
        let view = camera.rotate_tile(tile);
        let bit: u16 = match word(13) as u16 & 5 {
            1 => 2,
            4 => 8,
            _ => 0,
        };
        let turn = (i32::from(word(1)) / 2 + i32::from(camera.rotation & 3)).rem_euclid(4) as u32;
        let side = match ((bit << turn) | (bit >> (4 - turn))) & 15 {
            1 => [-1, 0],
            2 => [0, -1],
            4 => [1, 0],
            8 => [0, 1],
            _ => [0, 0],
        };
        self.contains(view) && self.contains([view[0] + side[0], view[1] + side[1]])
    }
}

/// Marks the view tiles from `a` up to but not including `b`, in both
/// directions, that are on the lot.
fn square(view: &View<'_>, rows: &mut [u64; ARRAY_SIZE as usize], a: [i32; 2], b: [i32; 2]) {
    for y in a[1].min(b[1])..a[1].max(b[1]) {
        for x in a[0].min(b[0])..a[0].max(b[0]) {
            if view.inside([x, y]) {
                mark(rows, [x, y]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Walls, Zoom};

    const SIZE: usize = 20;
    const ROOM: i16 = 2;
    const OUTSIDE: i16 = 1;

    /// A flat 20-tile lot, outdoors but for room 2 on tiles 8 to 11 each
    /// way, walled all round.
    fn house() -> Lot {
        let full = lot::FULL * lot::FULL;
        let mut rooms = vec![OUTSIDE; full];
        let mut walls = vec![lot::WallCell::default(); full];
        for x in 8..12 {
            for y in 8..12 {
                rooms[Lot::cell(x, y)] = ROOM;
            }
        }
        for i in 8..12 {
            walls[Lot::cell(8, i)].segments |= lot::WEST;
            walls[Lot::cell(12, i)].segments |= lot::WEST;
            walls[Lot::cell(i, 8)].segments |= lot::NORTH;
            walls[Lot::cell(i, 12)].segments |= lot::NORTH;
        }
        let level = |rooms: Option<Vec<i16>>, walls| lot::Level {
            floors: vec![0; full],
            walls,
            rooms,
        };
        Lot {
            size: SIZE,
            heights: vec![[0; 4]; full],
            grass: vec![0; full],
            levels: [
                level(Some(rooms), walls),
                level(None, vec![lot::WallCell::default(); full]),
            ],
            outdoor_rooms: [OUTSIDE].into(),
            room_lights: Default::default(),
            room_fractions: Default::default(),
            outdoor_light: None,
            time_of_day: 0,
            halves: Default::default(),
            lamps: Default::default(),
        }
    }

    fn camera(rotation: u8) -> Camera {
        Camera {
            zoom: Zoom::Far,
            rotation,
            level: 1,
            origin: [400, -100],
            walls: Walls::Cutaway,
            focus: Focus::default(),
        }
    }

    /// A wall: its lot cell, segment bit, and the tiles inside and outside.
    type Wall = ([i32; 2], u8, [i32; 2], [i32; 2]);

    /// The room's walls.
    fn perimeter() -> Vec<Wall> {
        let mut walls = Vec::new();
        for i in 8..12 {
            walls.push(([8, i], lot::WEST, [8, i], [7, i]));
            walls.push(([12, i], lot::WEST, [11, i], [12, i]));
            walls.push(([i, 8], lot::NORTH, [i, 8], [i, 7]));
            walls.push(([i, 12], lot::NORTH, [i, 11], [i, 12]));
        }
        walls
    }

    #[test]
    fn the_pointer_remembers_three_rooms_newest_first() {
        let mut focus = Focus::default();
        for room in [1, 2, 3, 2, 4] {
            focus.visit(room);
        }
        assert_eq!(focus.rooms, [Some(4), Some(3), Some(2)]);
        focus.forget();
        assert_eq!(focus.rooms, [None; ROOMS]);
    }

    #[test]
    fn nothing_is_cut_without_a_room_to_look_at() {
        let lot = house();
        for rotation in 0..4 {
            let c = camera(rotation);
            let mask = Mask::new(&lot, &c);
            for (cell, bit, _, _) in perimeter() {
                assert!(!mask.cuts(&c, cell, bit));
            }
        }
    }

    #[test]
    fn a_looked_at_room_loses_the_walls_in_front_of_it() {
        let lot = house();
        for rotation in 0..4 {
            let mut c = camera(rotation);
            c.focus.visit(ROOM);
            let mask = Mask::new(&lot, &c);
            for (cell, bit, inside, outside) in perimeter() {
                let [i, o] = [inside, outside].map(|t| c.rotate_tile(t));
                // The camera looks down the view's x and y: the walls
                // whose outside is nearer the camera are the front ones.
                let front = o[0] + o[1] > i[0] + i[1];
                assert_eq!(
                    mask.cuts(&c, cell, bit),
                    front,
                    "rotation {rotation}, wall {cell:?} {bit}"
                );
            }
        }
    }

    #[test]
    fn a_change_of_zoom_keeps_the_rooms_parts_as_built() {
        let lot = house();
        let count = |rows: &[u64]| rows.iter().map(|r| r.count_ones()).sum::<u32>();
        for rotation in 0..4 {
            let mut far = camera(rotation);
            far.focus.visit(ROOM);
            let built_far = Mask::new(&lot, &far).rows;
            for zoom in [Zoom::Medium, Zoom::Near] {
                let shown = Camera { zoom, ..far };
                let kept = Camera {
                    focus: Focus {
                        masks_built_at: Some(Zoom::Far),
                        ..far.focus
                    },
                    ..shown
                };
                assert_eq!(
                    Mask::new(&lot, &kept).rows,
                    built_far,
                    "{rotation} {zoom:?}"
                );
                // Built at the zoom shown, the room's part also takes the
                // next row of tiles down the screen, which overlaps the
                // room's tiles there (0x46831c).
                let fresh = Mask::new(&lot, &shown).rows;
                assert!(
                    fresh.iter().zip(&built_far).all(|(f, b)| f & b == *b),
                    "{rotation} {zoom:?}"
                );
                assert!(count(&fresh) > count(&built_far), "{rotation} {zoom:?}");
            }
        }
    }

    /// The screen point at the middle of a tile raised `height` steps.
    fn middle(c: &Camera, tile: [i32; 2], height: i32) -> [i32; 2] {
        let [x, y] = c.project(tile, 1, height);
        [x, y + c.zoom.tile_height() / 2]
    }

    #[test]
    fn the_pointer_finds_the_spot_it_is_over() {
        let lot = house();
        let c = camera(0);
        let view = View::new(&lot, &c);
        assert_eq!(view.fine(middle(&c, [9, 9], 0)), Some([152, 152]));
        // Down and right along the tile's x edge is along x only; down and
        // left is along y.
        let [x, y] = middle(&c, [9, 9], 0);
        assert_eq!(view.fine([x + 4, y + 2]), Some([156, 152]));
        assert_eq!(view.fine([x - 6, y + 3]), Some([152, 158]));
    }

    #[test]
    fn the_pointer_looks_at_its_room_and_the_room_below_it() {
        let lot = house();
        let c = camera(0);
        let mut focus = Focus::default();
        focus.point(&lot, &c, middle(&c, [9, 9], 0));
        assert_eq!(focus.rooms, [Some(ROOM), None, None]);
        assert_eq!(focus.pointer, Some(middle(&c, [9, 9], 0)));
        assert!(!focus.away);
        // From the room's front corner, outdoors is half a tile down.
        let mut focus = Focus::default();
        focus.point(&lot, &c, middle(&c, [11, 11], 0));
        assert_eq!(focus.rooms, [Some(OUTSIDE), Some(ROOM), None]);
        // Off the lot, or on its outer ring of tiles, the rooms are kept
        // but not looked at, until the pointer comes back.
        for off in [[-5000, -5000], middle(&c, [0, 9], 0)] {
            let mut c = c;
            c.focus = focus;
            c.focus.point(&lot, &camera(0), off);
            assert!(c.focus.away);
            assert_eq!(c.focus.rooms, focus.rooms);
            assert_eq!(Mask::new(&lot, &c), Mask::new(&lot, &camera(0)));
            c.focus.point(&lot, &camera(0), middle(&c, [9, 9], 0));
            assert!(!c.focus.away);
            assert_ne!(Mask::new(&lot, &c), Mask::new(&lot, &camera(0)));
        }
    }

    #[test]
    fn the_pointer_picks_raised_tiles_where_they_are_drawn() {
        let mut lot = house();
        for x in 8..12 {
            for y in 8..12 {
                lot.heights[Lot::cell(x, y)] = [4; 4];
            }
        }
        let c = camera(0);
        // The room's back tile, raised a tile's height: the ground there
        // would be outdoors.
        let at = middle(&c, [8, 8], 4);
        assert_eq!(View::new(&lot, &c).fine(at), Some([136, 136]));
        let mut focus = Focus::default();
        focus.point(&lot, &c, at);
        assert_eq!(focus.rooms, [Some(ROOM), None, None]);
    }

    #[test]
    fn the_pointer_finds_tiles_raised_as_high_as_terrain_goes_upstairs() {
        let mut lot = house();
        for x in 1..SIZE - 1 {
            for y in 1..SIZE - 1 {
                lot.heights[Lot::cell(x, y)] = [255; 4];
            }
        }
        for rotation in 0..4 {
            let mut c = camera(rotation);
            c.level = 2;
            let tile = c.rotate_tile([1, 1]);
            let [x, y] = c.project(tile, 2, 255);
            let at = [x, y + c.zoom.tile_height() / 2];
            let fine = View::new(&lot, &c).fine(at).map(|f| f.map(|v| v >> 4));
            assert_eq!(fine, Some(tile), "rotation {rotation}");
        }
    }

    #[test]
    fn an_outdoor_sim_and_the_pointer_mark_squares_of_tiles() {
        let lot = house();
        let mut c = camera(0);
        let outdoors = Selected {
            tile: [4, 4],
            level: 1,
            room: OUTSIDE,
        };
        c.focus.selected = Some(outdoors);
        let mask = Mask::new(&lot, &c);
        assert!(mask.contains([4, 4]) && mask.contains([7, 7]));
        assert!(!mask.contains([8, 8]) && !mask.contains([3, 3]));
        // A Sim on the other floor marks nothing.
        c.focus.selected = Some(Selected {
            level: 2,
            ..outdoors
        });
        assert!(!Mask::new(&lot, &c).contains([4, 4]));
        // The pointer adds a square once a room is outdoors: from the tile
        // four tiles' height below it, four tiles back.
        c.focus.selected = None;
        let [x, y] = middle(&c, [15, 15], 0);
        c.focus.pointer = Some([x, y - 4 * c.zoom.tile_height()]);
        assert!(!Mask::new(&lot, &c).contains([12, 12]));
        c.focus.visit(OUTSIDE);
        let mask = Mask::new(&lot, &c);
        assert!(mask.contains([11, 11]) && mask.contains([14, 14]));
        assert!(!mask.contains([15, 15]) && !mask.contains([10, 10]));
        // An outdoor selected Sim turns the square on too: both set the
        // flag the square tests (0x4aba79, 0x4abb32; tested at 0x4abe87).
        c.focus.forget();
        c.focus.selected = Some(outdoors);
        assert!(Mask::new(&lot, &c).contains([12, 12]));
    }

    #[test]
    fn the_selected_sim_counts_in_its_own_room() {
        // The Sim's room is the simulation's, not its tile's: on a split
        // tile they differ.
        let lot = house();
        let mut c = camera(0);
        c.focus.selected = Some(Selected {
            tile: [7, 9],
            level: 1,
            room: ROOM,
        });
        let mask = Mask::new(&lot, &c);
        assert!(mask.contains([9, 9]) && !mask.contains([5, 5]));
        assert!(mask.cuts(&c, [12, 9], lot::WEST));
        // Split markers count for nothing.
        c.focus.selected = Some(Selected {
            tile: [7, 9],
            level: 1,
            room: -5,
        });
        assert_eq!(Mask::new(&lot, &c), Mask::new(&lot, &camera(0)));
    }

    #[test]
    fn objects_hide_where_the_mask_holds_them_and_the_side_they_face() {
        let lot = house();
        let mut c = camera(0);
        c.focus.visit(ROOM);
        let mask = Mask::new(&lot, &c);
        let mut data = vec![0i16; 16];
        data[8] = 0x400;
        assert!(hides_with_walls(&data));
        // In the front wall, facing out of the room: hidden.
        data[13] = 4;
        assert!(mask.hides(&c, [10, 11], &data));
        // In the back wall, facing out: the tile behind is not looked at.
        data[13] = 1;
        assert!(!mask.hides(&c, [10, 8], &data));
        data[8] = 0;
        assert!(!hides_with_walls(&data));
    }

    #[test]
    fn far_off_pointers_and_tiles_do_not_panic() {
        let lot = house();
        let mut c = camera(3);
        c.focus.visit(ROOM);
        c.focus.visit(OUTSIDE);
        c.focus.pointer = Some([i32::MAX / 2, i32::MIN / 2]);
        c.focus.selected = Some(Selected {
            tile: [-1, 70],
            level: 1,
            room: OUTSIDE,
        });
        let mask = Mask::new(&lot, &c);
        assert!(!mask.contains([-1, 0]) && !mask.contains([64, 64]));
        assert!(!mask.cuts(&c, [i32::MIN, i32::MAX], lot::ASCENDING));
        let mut focus = Focus::default();
        focus.point(&lot, &c, [i32::MAX, i32::MIN]);
        assert!(focus.away && focus.pointer.is_none());
        c.origin = [i32::MAX, i32::MIN];
        focus.point(&lot, &c, [i32::MAX, i32::MIN]);
        c.focus.pointer = Some([i32::MAX, i32::MIN]);
        Mask::new(&lot, &c);
        // Every screen point near the lot finds a spot or none, quickly.
        let c = camera(1);
        let view = View::new(&lot, &c);
        for x in (-1200..2000).step_by(37) {
            for y in (-800..1200).step_by(29) {
                view.fine([x, y]);
            }
        }
    }
}
