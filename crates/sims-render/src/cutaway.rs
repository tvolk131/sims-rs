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
//! And if any of the rooms is outdoors, the pointer adds the 4 by 4 square
//! behind it (0x4abdda).
//!
//! The old prototype ported these in
//! `legacy/crates/sims-sim/src/lot_presenter/{room_masks,visibility}.rs`
//! and `walls/cutaway.rs`; this follows them. Where the pointer and the
//! selected Sim are is input, given in [`Focus`]: the renderer only reads
//! the world.

use crate::camera::{ARRAY_SIZE, Camera};
use crate::lot::{self, Lot};

/// How many pointer rooms the original remembers (the list's capacity, set
/// in the view's constructor at 0x4a6d96).
pub const ROOMS: usize = 3;

/// What the cutaway looks at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Focus {
    /// The last rooms the pointer passed over, newest first.
    pub rooms: [Option<i16>; ROOMS],
    /// The lot tile under the pointer, if it is over the lot.
    pub pointer: Option<[i32; 2]>,
    /// The selected Sim's lot tile and level (1 or 2), in live mode.
    pub selected: Option<[i32; 3]>,
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
    /// half a storey, looked for every half tile.
    pub fn point(&mut self, lot: &Lot, camera: &Camera, at: [i32; 2]) {
        // Points this far off are off the lot; the projection's arithmetic
        // is not meant for them.
        let off = |i: usize| (i64::from(at[i]) - i64::from(camera.origin[i])).unsigned_abs();
        if off(0) > FAR || off(1) > FAR {
            self.pointer = None;
            return;
        }
        let level = camera.level.clamp(1, 2);
        let room = |y: i32| -> Option<Option<i16>> {
            let tile = ground_tile(camera, [at[0], y.saturating_add(lift(camera))]);
            lot.contains(tile[0], tile[1])
                .then(|| room_of(lot, (level - 1) as usize, tile))
        };
        let Some(first) = room(at[1]) else {
            self.pointer = None;
            return;
        };
        self.pointer = Some(ground_tile(
            camera,
            [at[0], at[1].saturating_add(lift(camera))],
        ));
        let step = camera.zoom.tile_height() / 2;
        let mut below = first;
        let mut offset = step;
        while offset < camera.zoom.floor_height() / 2 {
            // A point off the lot keeps the room found so far.
            if let Some(found) = room(at[1].saturating_add(offset)) {
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

/// Screen points this far from the origin are treated as off the lot.
const FAR: u64 = 1 << 24;

/// Screen rows a level's floor sits above the ground's.
fn lift(camera: &Camera) -> i32 {
    camera.zoom.floor_height() * (camera.level.clamp(1, 2) - 1)
}

fn ground_tile(camera: &Camera, at: [i32; 2]) -> [i32; 2] {
    camera.unproject_fine(at).map(|f| f.div_euclid(16))
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

impl View<'_> {
    fn inside(&self, [x, y]: [i32; 2]) -> bool {
        let b = self.bounds;
        x >= b[0] && x <= b[1] && y >= b[2] && y <= b[3]
    }

    fn interior(&self, [x, y]: [i32; 2]) -> bool {
        let b = self.bounds;
        x > b[0] && x < b[1] && y > b[2] && y < b[3]
    }

    /// The height of a view tile's top corner, in terrain steps.
    fn top(&self, view: [i32; 2]) -> i32 {
        let [x, y] = self.camera.unrotate_tile(view);
        if !self.lot.contains(x, y) {
            return 0;
        }
        let h = self.lot.heights[Lot::cell(x as usize, y as usize)];
        // Corner 1 on screen is the stored corner (1 - rotation) mod 4
        // (0x479567).
        i32::from(h[(5 - usize::from(self.camera.rotation & 3)) & 3])
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
        let level = camera.level.clamp(1, 2);
        let view = View {
            lot,
            camera,
            level: (level - 1) as usize,
            bounds: bounds(lot, camera),
        };
        let focus = camera.focus;
        let mut outdoors = false;
        for room in focus.rooms.into_iter().flatten() {
            view.room(room, &mut rows);
            outdoors |= lot.outdoor_rooms.contains(&room);
        }
        if let Some([x, y, _]) = focus.selected.filter(|s| s[2] == level)
            && let Some(room) = room_of(lot, view.level, [x, y])
        {
            view.room(room, &mut rows);
            let size = lot.size as i32;
            if lot.outdoor_rooms.contains(&room) {
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
        if outdoors && let Some(pointer) = focus.pointer.filter(|p| lot.contains(p[0], p[1])) {
            let mut start = camera.rotate_tile(pointer);
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
            halves: Default::default(),
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
    fn the_pointer_looks_at_its_room_and_the_room_below_it() {
        let lot = house();
        let c = camera(0);
        let screen = |tile: [i32; 2]| {
            let [x, y] = c.project(tile, 1, 0);
            [x, y + c.zoom.tile_height() / 2]
        };
        let mut focus = Focus::default();
        focus.point(&lot, &c, screen([9, 9]));
        assert_eq!(focus.rooms, [Some(ROOM), None, None]);
        assert_eq!(focus.pointer, Some([9, 9]));
        // From the room's front corner, outdoors is half a tile down.
        let mut focus = Focus::default();
        focus.point(&lot, &c, screen([11, 11]));
        assert_eq!(focus.rooms, [Some(OUTSIDE), Some(ROOM), None]);
        // Off the lot nothing is remembered.
        let mut focus = Focus::default();
        focus.point(&lot, &c, [-5000, -5000]);
        assert_eq!(focus, Focus::default());
    }

    #[test]
    fn an_outdoor_sim_and_the_pointer_mark_squares_of_tiles() {
        let lot = house();
        let mut c = camera(0);
        c.focus.selected = Some([4, 4, 1]);
        let mask = Mask::new(&lot, &c);
        assert!(mask.contains([4, 4]) && mask.contains([7, 7]));
        assert!(!mask.contains([8, 8]) && !mask.contains([3, 3]));
        // A Sim on the other floor marks nothing.
        c.focus.selected = Some([4, 4, 2]);
        assert!(!Mask::new(&lot, &c).contains([4, 4]));
        // The pointer adds the square behind it once a room is outdoors.
        c.focus.selected = None;
        c.focus.pointer = Some([15, 15]);
        assert!(!Mask::new(&lot, &c).contains([12, 12]));
        c.focus.visit(OUTSIDE);
        let mask = Mask::new(&lot, &c);
        assert!(mask.contains([11, 11]) && mask.contains([14, 14]));
        assert!(!mask.contains([15, 15]));
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
        c.focus.selected = Some([-1, 70, 1]);
        let mask = Mask::new(&lot, &c);
        assert!(!mask.contains([-1, 0]) && !mask.contains([64, 64]));
        assert!(!mask.cuts(&c, [i32::MIN, i32::MAX], lot::ASCENDING));
        let mut focus = Focus::default();
        focus.point(&lot, &c, [i32::MAX, i32::MIN]);
    }
}
