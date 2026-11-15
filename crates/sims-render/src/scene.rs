//! Draws a lot: terrain, floors, walls and objects, level by level.

use crate::assets::{Assets, Definition};
use crate::camera::{self, Camera, Walls, Zoom};
use crate::canvas::{Canvas, Image};
use crate::cutaway::{self, Mask};
use crate::depth;
use crate::grass::{self, Grass};
use crate::light::Light;
use crate::lot::{self, Lot, WallCell};
use sims_sim::World;
use std::collections::BTreeMap;

/// Rows between a tile's left corner and the top of its first grass
/// column (matched against the original's screen).
const GRASS_ROW: i32 = 0;
const GRASS_COLUMN: i32 = 1;

/// Object sprites sit one pixel right of the point the camera projects
/// them to, measured against the walls and floors around them. The camera
/// origin is fitted to the tile art, and the original places the two
/// kinds of art one pixel apart; at every compared camera (far and
/// medium zoom) the gap is this same pixel. Near zoom is unverified (see
/// the decisions log, 2026-11-15).
const OBJECT_COLUMN: i32 = 1;

/// What the window shows outside the lot: the original's grey (5-6-5
/// red 14, green 28, blue 14), widened to 8 bits a channel.
pub const BACKGROUND: [u8; 3] = [115, 113, 115];

/// Counts of what one frame drew, for tests and the viewer's status line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub floors: usize,
    pub walls: usize,
    pub objects: usize,
    /// Wall faces whose shape or pattern the game files lack.
    pub missing_walls: usize,
    /// Objects with no sprites for this view.
    pub missing_objects: usize,
}

/// Draws the lot as the camera sees it. `world` is only read.
pub fn draw(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    canvas: &mut Canvas,
) -> Stats {
    canvas.clear(BACKGROUND);
    let mut painter = Painter {
        world,
        lot,
        camera,
        assets,
        canvas,
        grass: Grass::new(lot, camera),
        stats: Stats::default(),
    };
    let top = camera.level.clamp(1, 2);
    for level in 1..=top {
        painter.floors(level);
        // Only the top floor shown is cut away; the floors below it stand
        // (0x4a8895, the old prototype's `wall_view.rs`).
        let cut = if level < top {
            Cut::None
        } else {
            match camera.walls {
                Walls::Up => Cut::None,
                Walls::Down => Cut::All,
                Walls::Cutaway => Cut::Mask(Box::new(Mask::new(lot, camera))),
            }
        };
        painter.walls(level, &cut);
        painter.trims(level, &cut);
        painter.objects(level, &cut);
    }
    painter.stats
}

/// Which walls of a level are cut down, and which objects hide with them.
enum Cut {
    None,
    All,
    Mask(Box<Mask>),
}

impl Cut {
    /// Whether the wall with segment bit `bit` and style `style` of lot
    /// cell `cell` is cut down. Fences never are.
    fn cuts(&self, camera: &Camera, cell: [i32; 2], bit: u8, style: u16) -> bool {
        solid(style)
            && match self {
                Cut::None => false,
                Cut::All => true,
                Cut::Mask(mask) => mask.cuts(camera, cell, bit),
            }
    }
}

/// The trim frame for the walls meeting at a corner, by the directions
/// they leave it in (the original's table at 0x6cd468, less one): bits 1,
/// 2, 4 and 8 are view -y, -x, +y and +x, and 0x10 to 0x80 the diagonals
/// between them, from (-1, -1) round to (1, -1). Combinations the game
/// has no frame for are None.
fn trim_frame(mask: u8) -> Option<usize> {
    const DIAGONALS: [(u8, usize); 30] = [
        (0x14, 16),
        (0x18, 17),
        (0x1c, 18),
        (0x20, 19),
        (0x21, 20),
        (0x28, 21),
        (0x29, 22),
        (0x30, 23),
        (0x38, 24),
        (0x40, 25),
        (0x41, 26),
        (0x42, 27),
        (0x43, 28),
        (0x50, 29),
        (0x60, 30),
        (0x61, 31),
        (0x70, 32),
        (0x80, 33),
        (0x82, 34),
        (0x84, 35),
        (0x86, 36),
        (0x90, 37),
        (0x94, 38),
        (0xa0, 39),
        (0xb0, 40),
        (0xc0, 41),
        (0xc2, 42),
        (0xd0, 43),
        (0xe0, 44),
        (0xf0, 45),
    ];
    match mask {
        0 => None,
        1..=0x10 => Some(usize::from(mask) - 1),
        _ => DIAGONALS
            .iter()
            .find(|&&(m, _)| m == mask)
            .map(|&(_, frame)| frame),
    }
}

/// The trim bit for a wall leaving a view corner in view direction `d`.
fn trim_bit(d: [i32; 2]) -> u8 {
    match d {
        [0, -1] => 1,
        [-1, 0] => 2,
        [0, 1] => 4,
        [1, 0] => 8,
        [-1, -1] => 0x10,
        [-1, 1] => 0x20,
        [1, 1] => 0x40,
        [1, -1] => 0x80,
        _ => 0,
    }
}

/// The walls meeting at one corner, for its trim.
#[derive(Default)]
struct Meeting {
    /// Bits of the standing walls.
    up: u8,
    /// Bits of the cut walls that take trim.
    cut: u8,
}

/// A wall segment by its two lot corners, the lower first.
type Edge = [[i32; 2]; 2];

fn edge(a: [i32; 2], b: [i32; 2]) -> Edge {
    if a <= b { [a, b] } else { [b, a] }
}

/// The elevation of a level's floor above the ground level, in near-zoom
/// pixels.
fn level_elevation(level: i32) -> i32 {
    Zoom::Near.floor_height() * (level - 1)
}

/// One wall of a cell: its corners and the direction its first side faces.
struct Segment {
    bit: u8,
    from: [i32; 2],
    to: [i32; 2],
    normal: [i32; 2],
}

fn segments(x: i32, y: i32) -> [Segment; 4] {
    let segment = |bit, from, to, normal| Segment {
        bit,
        from,
        to,
        normal,
    };
    [
        segment(lot::WEST, [x, y], [x, y + 1], [1, 0]),
        segment(lot::NORTH, [x, y], [x + 1, y], [0, 1]),
        segment(lot::DESCENDING, [x + 1, y], [x, y + 1], [1, 1]),
        segment(lot::ASCENDING, [x, y], [x + 1, y + 1], [1, -1]),
    ]
}

/// A fine position just beside a wall segment's middle, on its normal's
/// side (side 0) or the other (side 1).
fn wall_side(s: &Segment, side: usize) -> [i32; 2] {
    let sign = if side == 0 { 2 } else { -2 };
    [0, 1].map(|i| (s.from[i] + s.to[i]) * 8 + sign * s.normal[i])
}

struct Painter<'a> {
    world: &'a World,
    lot: &'a Lot,
    camera: &'a Camera,
    assets: &'a mut Assets,
    canvas: &'a mut Canvas,
    grass: Grass,
    stats: Stats,
}

impl Painter<'_> {
    /// The elevation of a lot corner in near-zoom pixels: 16 per terrain
    /// step.
    fn corner_elevation(&self, [cx, cy]: [i32; 2]) -> i32 {
        // A tile's four heights are its corners (0, 1), (0, 0), (1, 0) and
        // (1, 1), in that order (`legacy/crates/sims-render/src/walls.rs`).
        let last = self.lot.size as i32 - 1;
        let (tx, ty) = (cx.clamp(0, last), cy.clamp(0, last));
        let h = self.lot.heights[Lot::cell(tx as usize, ty as usize)];
        let corner = match (cx - tx, cy - ty) {
            (0, 1) => 0,
            (0, 0) => 1,
            (1, 0) => 2,
            _ => 3,
        };
        i32::from(h[corner]) * 16
    }

    /// The light at a fine position on a level (0 or 1).
    fn light(&self, level: usize, fine: [i32; 2]) -> Light {
        Light::from_byte(self.lot.light_at(level, fine))
    }

    /// Rotates a lot corner (0 to 64) into the view.
    fn rotate_corner(&self, [x, y]: [i32; 2]) -> [i32; 2] {
        let m = lot::FULL as i32;
        match self.camera.rotation & 3 {
            1 => [m - y, x],
            2 => [m - x, m - y],
            3 => [y, m - x],
            _ => [x, y],
        }
    }

    /// Rotates a lot direction into the view.
    fn rotate_direction(&self, [x, y]: [i32; 2]) -> [i32; 2] {
        match self.camera.rotation & 3 {
            1 => [-y, x],
            2 => [-x, -y],
            3 => [y, -x],
            _ => [x, y],
        }
    }

    /// The screen point of a lot corner on a level, at its terrain height.
    fn corner_screen(&self, corner: [i32; 2], level: i32) -> [i32; 2] {
        let view = self.rotate_corner(corner);
        self.camera.project_fine(
            [view[0] * 16, view[1] * 16],
            level,
            self.corner_elevation(corner),
        )
    }

    fn floors(&mut self, level: i32) {
        // The ground shows wherever a floor leaves it uncovered, such as
        // half of a split tile. All of it goes first: a grass column can
        // reach a pixel or two into the next tile, and floors win there.
        // The original draws no ground on the lot's outermost tiles
        // (0x49cecc).
        if level == 1 {
            let inner = 1..self.lot.size as i32 - 1;
            for x in inner.clone() {
                for y in inner.clone() {
                    self.terrain([x, y]);
                }
            }
        }
        let index = (level - 1) as usize;
        let zoom = self.camera.zoom.index();
        let base = level_elevation(level);
        for x in 0..self.lot.size as i32 {
            for y in 0..self.lot.size as i32 {
                let cell = Lot::cell(x as usize, y as usize);
                let floor = self.lot.levels[index].floors[cell];
                if floor == 0 {
                    continue;
                }
                // The tile's top corner in the view.
                let top = [[x, y], [x + 1, y], [x + 1, y + 1], [x, y + 1]]
                    .into_iter()
                    .min_by_key(|&c| {
                        let v = self.rotate_corner(c);
                        v[0] + v[1]
                    })
                    .unwrap_or([x, y]);
                let elevation = base + self.corner_elevation(top);
                let p = self.corner_screen(top, level);
                let wall = self.lot.levels[index].walls[cell];
                let halves = if floor == lot::SPLIT_FLOOR {
                    wall.split_floors().map(Some)
                } else {
                    [Some(floor), None]
                };
                for (half, id) in halves.into_iter().enumerate() {
                    let Some(id) = id.filter(|&id| id != 0) else {
                        continue;
                    };
                    let Some(image) = self.assets.floor(id, zoom) else {
                        continue;
                    };
                    let at = [p[0] - image.width as i32 / 2, p[1]];
                    let image = if floor == lot::SPLIT_FLOOR {
                        split_half(self.camera, image, wall.segments, half)
                    } else {
                        image.clone()
                    };
                    // A point inside the half, for its room's light.
                    let inside = match (floor == lot::SPLIT_FLOOR, wall.segments, half) {
                        (false, _, _) => [8, 8],
                        (true, s, 0) if s & lot::DESCENDING != 0 => [4, 4],
                        (true, s, _) if s & lot::DESCENDING != 0 => [12, 12],
                        (true, _, 0) => [4, 12],
                        (true, _, _) => [12, 4],
                    };
                    let light = self.light(index, [x * 16 + inside[0], y * 16 + inside[1]]);
                    let camera = self.camera;
                    self.canvas.draw_lit(&image, at, false, light, |_, y, _| {
                        depth::ground(camera, y, elevation)
                    });
                    self.stats.floors += 1;
                }
            }
        }
    }

    /// Draws a grass tile in the original's four-pixel columns (see
    /// [`crate::grass`]).
    fn terrain(&mut self, [x, y]: [i32; 2]) {
        let view = self.camera.rotate_tile([x, y]);
        let Some((shades, colors)) = self.grass.tile(view) else {
            return;
        };
        let rotation = usize::from(self.camera.rotation & 3);
        let h = self.lot.heights[Lot::cell(x as usize, y as usize)];
        let heights: [u8; 4] = std::array::from_fn(|n| h[(n + 4 - rotation) & 3]);
        // The left corner, at height 0; columns start a row above it.
        let corner = self.camera.project([view[0], view[1] + 1], 1, 0);
        let zoom = self.camera.zoom.level();
        let width = 16 << zoom;
        // Skip a tile wholly off the canvas, even on the highest ground.
        let reach = width + (255 << (zoom + 1));
        let size = [self.canvas.width, self.canvas.height].map(|n| n as i32);
        if (0..2).any(|i| corner[i] < -reach || corner[i] > size[i].saturating_add(reach)) {
            return;
        }
        let lit = self.light(0, [x * 16 + 8, y * 16 + 8]);
        // The whole tile takes the depth of its lowest corner. Following
        // the slope would be truer, but floors take their top corner's
        // elevation, and where sloped ground meets a floor the grass would
        // then cover the floor's edge, which the original does not.
        let elevation = (0..4)
            .map(|c| self.corner_elevation([x + (c & 1), y + (c >> 1)]))
            .min()
            .unwrap_or(0);
        let camera = self.camera;
        let left = corner[0] + GRASS_COLUMN;
        for column in grass::columns(left, corner[1] + GRASS_ROW, heights, shades, colors, zoom) {
            for (row, pixels) in column.rows() {
                let depth = depth::ground(camera, row, elevation);
                for (i, &p) in pixels.iter().enumerate() {
                    self.canvas
                        .plot(column.x + i as i32, row, depth, lit.apply(grass::widen(p)));
                }
            }
            let index = ((column.x - left) / 4) as usize;
            for (bx, foot, height, p) in self.grass.blades(view, &column, index) {
                let depth = depth::ground(camera, foot, elevation);
                let color = lit.apply(grass::widen(p));
                for row in foot.saturating_sub(height - 1)..=foot {
                    self.canvas.plot(bx, row, depth, color);
                }
            }
        }
    }

    /// The wallpaper on each side of a wall, first along its normal (the
    /// old prototype's `faces` in
    /// `legacy/crates/sims-render/src/walls.rs`, checked there against the
    /// original's wall records).
    fn sides(&self, level: usize, x: i32, y: i32, bit: u8) -> [u16; 2] {
        let walls = &self.lot.levels[level].walls;
        let cell = |x: i32, y: i32| -> WallCell {
            if self.lot.contains(x, y) {
                walls[Lot::cell(x as usize, y as usize)]
            } else {
                WallCell::default()
            }
        };
        let t = cell(x, y);
        match bit {
            lot::WEST => [t.patterns[0], cell(x - 1, y).patterns[3]],
            lot::NORTH => [t.patterns[1], cell(x, y - 1).patterns[2]],
            _ => [t.patterns[3], t.patterns[2]],
        }
    }

    /// The objects that cut their own shape into a wall on a level, by the
    /// wall they stand in (the old prototype's `object_apertures` in
    /// `legacy/crates/sims-render/src/walls.rs`).
    fn wall_objects(&mut self, level: i32) -> BTreeMap<Edge, Definition> {
        let mut found = BTreeMap::new();
        for o in self.world.objects.values() {
            if o.position[2] != level || o.position[0] < 0 || o.position[1] < 0 {
                continue;
            }
            let Some(definition) = self.assets.definition(o.guid) else {
                continue;
            };
            if let Some(e) = object_edge(o.position, &o.data).filter(|_| definition.shapes_wall()) {
                found.insert(e, definition);
            }
        }
        found
    }

    /// Draws a level's walls.
    fn walls(&mut self, level: i32, cut: &Cut) {
        let index = (level - 1) as usize;
        let zoom = self.camera.zoom.index();
        let base = level_elevation(level);
        let rise = 4 << self.camera.zoom.level();
        let openings = self.wall_objects(level);
        for x in 0..self.lot.size as i32 {
            for y in 0..self.lot.size as i32 {
                let wall = self.lot.levels[index].walls[Lot::cell(x as usize, y as usize)];
                for s in segments(x, y) {
                    if wall.segments & s.bit == 0 {
                        continue;
                    }
                    let style = if s.bit == lot::WEST {
                        wall.styles[0]
                    } else {
                        wall.styles[1]
                    };
                    let patterns = self.sides(index, x, y, s.bit);
                    let n = self.rotate_direction(s.normal);
                    let front = usize::from(n[0] + n[1] < 0);
                    let cut = cut.cuts(self.camera, [x, y], s.bit, style);
                    let key = edge(s.from, s.to);
                    // The frame from the wall's direction on screen.
                    let a = self.corner_screen(s.from, level);
                    let b = self.corner_screen(s.to, level);
                    let (left, right) = if a[0] <= b[0] { (a, b) } else { (b, a) };
                    let frame = if left[0] == right[0] {
                        3
                    } else if left[1] == right[1] {
                        2
                    } else if left[1] > right[1] {
                        0
                    } else {
                        1
                    };
                    let opening = openings.get(&key);
                    let pattern = patterns[front];
                    // Fences have no cut shape and stay up.
                    let image = wall_image(self.assets, pattern, style, opening, cut, zoom, frame)
                        .or_else(|| {
                            wall_image(self.assets, pattern, style, opening, false, zoom, frame)
                        });
                    let Some(image) = image else {
                        self.stats.missing_walls += 1;
                        continue;
                    };
                    let (w, h) = (image.width as i32, image.height as i32);
                    // The sprite's bottom row sits one pixel above the
                    // corner's projected point (matched against the
                    // original's screen, 2026-11-14).
                    let (foot, at) = match frame {
                        0 | 2 => (left, [left[0], left[1] - h]),
                        1 => (left, [left[0], left[1] - (h - rise)]),
                        _ => {
                            let bottom = if a[1] > b[1] { a } else { b };
                            (bottom, [bottom[0] - w / 2, bottom[1] - h])
                        }
                    };
                    let elevation = base + self.corner_elevation(s.from);
                    // The side that shows takes the light of the room on
                    // that side: sampled just beside the wall's middle, so
                    // a split tile gives the triangle touching the wall.
                    let light = self.light(index, wall_side(&s, front));
                    let camera = self.camera;
                    self.canvas.draw_lit(&image, at, false, light, |sx, sy, _| {
                        let row = match frame {
                            0 => foot[1] - (sx - foot[0]) / 2,
                            1 => foot[1] + (sx - foot[0]) / 2,
                            _ => foot[1],
                        };
                        depth::upright(camera, sy, row, elevation)
                    });
                    self.stats.walls += 1;
                }
            }
        }
    }

    /// The wood trim along the tops of a level's walls (0x53c227). Each
    /// corner a wall touches takes one sprite whose frame shows the walls
    /// meeting there: standing walls' trim at the storey's height, cut
    /// walls' at the cut height. A cut wall that meets a standing one ends
    /// in a cap instead, which is not drawn yet (0x53c6b7 draws a slice of
    /// the wall's own pattern there). Cut walls in a door-like style (3, 5,
    /// 6, 15, 23) and fences take no trim. The original draws the trim at
    /// full light, whatever the room's light.
    fn trims(&mut self, level: i32, cut: &Cut) {
        let index = (level - 1) as usize;
        let mut corners: BTreeMap<[i32; 2], Meeting> = BTreeMap::new();
        let size = self.lot.size as i32;
        for x in 0..size {
            for y in 0..size {
                let wall = self.lot.levels[index].walls[Lot::cell(x as usize, y as usize)];
                for s in segments(x, y) {
                    if wall.segments & s.bit == 0 {
                        continue;
                    }
                    let style = if s.bit == lot::WEST {
                        wall.styles[0]
                    } else {
                        wall.styles[1]
                    };
                    if !solid(style) {
                        continue;
                    }
                    let is_cut = cut.cuts(self.camera, [x, y], s.bit, style);
                    let [a, b] = [s.from, s.to].map(|c| self.rotate_corner(c));
                    for (at, other, corner) in [(a, b, s.from), (b, a, s.to)] {
                        let bit = trim_bit([other[0] - at[0], other[1] - at[1]]);
                        let m = corners.entry(corner).or_default();
                        if !is_cut {
                            m.up |= bit;
                        } else if !matches!(style, 3 | 5 | 6 | 15 | 23) {
                            m.cut |= bit;
                        }
                    }
                }
            }
        }
        let zoom = self.camera.zoom;
        let half = zoom.tile_width() / 2;
        let cut_height = [6, 12, 23][zoom.index()];
        let base = level_elevation(level);
        for (corner, m) in corners {
            // A cut wall meeting a standing one ends in a cap.
            let cut_bits = if m.up != 0 { 0 } else { m.cut };
            let at = self.corner_screen(corner, level);
            let elevation = base + self.corner_elevation(corner);
            for (bits, is_cut, height) in [
                (cut_bits, true, cut_height),
                (m.up, false, zoom.floor_height()),
            ] {
                let Some(frame) = trim_frame(bits) else {
                    continue;
                };
                // Cut trim running along a view diagonal through the
                // corner's top or bottom (bits 0x10 and 0x40) comes from the
                // standing walls' set; lone 0x10 sits a little higher when
                // zoomed in. The extra pieces the original adds for bits 8
                // and 4 there are not drawn yet.
                let diagonal = is_cut && bits & 0x50 != 0;
                let lift = if is_cut && bits == 0x10 {
                    [0, 2, 5][zoom.index()]
                } else {
                    0
                };
                let set = is_cut && !diagonal;
                let Some(image) = self.assets.trim(set, zoom).get(frame).cloned() else {
                    continue;
                };
                // The original subtracts one more pixel from x; its corner
                // point sits one to the right of ours (fitted to the
                // screenshot).
                let rise = zoom.tile_height() / 2 + height + lift;
                let (Some(x), Some(y)) = (at[0].checked_sub(half), at[1].checked_sub(rise)) else {
                    continue;
                };
                let top = [x, y];
                let camera = self.camera;
                // The trim lies on the wall tops: each pixel as deep as
                // the wall under it.
                self.canvas
                    .draw_lit(&image, top, false, Light::FULL, |_, sy, _| {
                        depth::upright(camera, sy, sy + height, elevation)
                    });
            }
        }
    }

    fn objects(&mut self, level: i32, cut: &Cut) {
        let zoom = self.camera.zoom.index();
        let mut list = Vec::new();
        for (&id, o) in &self.world.objects {
            let [x, y, z] = o.position;
            if o.person.is_some() || z != level || x < 0 || y < 0 {
                continue;
            }
            // A script can hide an object (data word 34); the original
            // skips it when drawing, as the old prototype did
            // (`legacy/crates/sims-render/src/lib.rs`).
            if o.data.get(HIDDEN).is_some_and(|&w| w != 0) {
                continue;
            }
            let tile = [x >> 4, y >> 4];
            if !self.lot.contains(tile[0], tile[1]) {
                continue;
            }
            let Some(([x, y], lift)) = held(self.world, self.assets, id) else {
                continue;
            };
            let view = self.camera.rotate_fine([x, y]);
            // The ground under the object: the mean of its tile's corners.
            let ground = (0..4)
                .map(|c| self.corner_elevation([tile[0] + (c & 1), tile[1] + (c >> 1)]))
                .sum::<i32>()
                / 4;
            let up = (lift * 16.0).round() as i32;
            let anchor = depth::point(view, level_elevation(level) + ground + up);
            list.push((anchor, id, view, ground, [x, y], lift));
        }
        // Far objects first, so soft edges blend over what is behind them.
        list.sort_by_key(|&(d, id, ..)| (std::cmp::Reverse(d), id));
        for (anchor, id, view, ground, place, lift) in list {
            let o = &self.world.objects[&id];
            let Some(definition) = self.assets.definition(o.guid) else {
                self.stats.missing_objects += 1;
                continue;
            };
            // Doors, windows and the like hide where the walls are cut
            // (0x4aa446). Their scripts mark them when the lot loads; in a
            // house read without running those, the objects that shape a
            // wall still hide.
            let tile = [o.position[0] >> 4, o.position[1] >> 4];
            let walled = cutaway::hides_with_walls(&o.data) || definition.shapes_wall();
            let hidden = walled
                && match cut {
                    Cut::None => false,
                    Cut::All => true,
                    Cut::Mask(mask) => mask.hides(self.camera, tile, &o.data),
                };
            if hidden {
                continue;
            }
            let Some(group) = u16::try_from(o.data.first().copied().unwrap_or(0))
                .ok()
                .filter(|&g| g < definition.graphics)
                .and_then(|g| definition.graphic.checked_add(g))
            else {
                continue;
            };
            let direction = view_direction(o.data.get(1).copied().unwrap_or(0), self.camera);
            let screen = self.camera.project_fine(view, level, ground);
            let parts = self.assets.group(&definition, group, direction, zoom);
            if parts.is_empty() {
                self.stats.missing_objects += 1;
                continue;
            }
            let facing = o.data.get(1).copied().unwrap_or(0);
            let base = level_elevation(level) + ground;
            // An object takes the light of the room at its position (the
            // original samples the tile light map there, 0x467482).
            let light = Light::from_byte(
                self.lot
                    .light_at((level - 1) as usize, [o.position[0], o.position[1]]),
            );
            for part in parts {
                if !sprite_visible(part.sprite, definition.dynamic_base, &o.dynamic_sprites) {
                    continue;
                }
                // Offsets come from the files and may be anything; the
                // canvas clips whatever this gives.
                let height = i32::try_from(part.image.height).unwrap_or(i32::MAX);
                // A held object stands on its holder's slot: the slot's
                // height adds to the sprite's own.
                let object = [part.object[0], part.object[1], part.object[2] + lift];
                let at = [
                    screen[0]
                        .saturating_add(part.offset[0])
                        .saturating_add(OBJECT_COLUMN),
                    screen[1]
                        .saturating_add(part.offset[1])
                        .saturating_sub(height)
                        .saturating_sub(sprite_rise(object, self.camera.zoom)),
                ];
                let anchor = if part.object == [0.0; 3] {
                    anchor
                } else {
                    part_anchor(self.camera, [place[0], place[1], 0], facing, object, base)
                };
                self.canvas
                    .draw_lit(&part.image, at, part.flip, light, |_, _, byte| {
                        depth::sprite(anchor, byte)
                    });
            }
            self.stats.objects += 1;
        }
    }
}

/// An object's data word that hides it when not zero (the simulation's
/// `HIDDEN`).
const HIDDEN: usize = 34;

/// The data words naming the object that holds this one and which of
/// its slots it is in.
const CONTAINER: usize = 2;
const CONTAINER_SLOT: usize = 3;

/// Where an object stands: its fine position and how far it is lifted,
/// in height units. An object another holds (a lamp on an end table, a
/// sink in a counter) stands at its holder's place plus the slot's
/// offset, turned by the holder's facing, and up by the slot's height;
/// the original adds these while it draws the holder's contents
/// (0x437fdd, read by the old prototype in
/// `legacy/crates/sims-render/src/lib.rs`). None when a holder is hidden.
/// Objects a Sim holds stay at their own position.
fn held(world: &World, assets: &mut Assets, id: u16) -> Option<([i32; 2], f32)> {
    let mut cursor = world.objects.get(&id)?;
    let mut offset = [0.0f32; 3];
    // A chain longer than the lot has objects is a loop.
    for _ in 0..world.objects.len() {
        let container = cursor.data.get(CONTAINER).copied().unwrap_or(0);
        let Some(holder) = u16::try_from(container)
            .ok()
            .filter(|&c| c != 0)
            .and_then(|c| world.objects.get(&c))
            .filter(|h| h.person.is_none())
        else {
            break;
        };
        if holder.data.get(HIDDEN).is_some_and(|&w| w != 0) {
            return None;
        }
        let slot = usize::try_from(cursor.data.get(CONTAINER_SLOT).copied().unwrap_or(-1))
            .ok()
            .and_then(|i| assets.definition(holder.guid)?.slots.get(i).copied())
            .unwrap_or_default();
        let [mut x, mut y, z] = sprite_offset(slot);
        for _ in 0..(i32::from(holder.data.get(1).copied().unwrap_or(0)) / 2).rem_euclid(4) {
            (x, y) = (-y, x);
        }
        offset = [offset[0] + x, offset[1] + y, offset[2] + z];
        cursor = holder;
    }
    let [x, y, _] = cursor.position;
    Some((
        [x + offset[0].round() as i32, y + offset[1].round() as i32],
        offset[2],
    ))
}

/// A draw group sprite's world offset (`object`), made safe: x and y in
/// sixteenths of a tile, z in height units, finite and clamped to a lot's
/// width so nonsense in a file cannot overflow anything.
fn sprite_offset(object: [f32; 3]) -> [f32; 3] {
    let limit = (camera::ARRAY_SIZE * 16) as f32;
    object.map(|v| {
        if v.is_finite() {
            v.clamp(-limit, limit)
        } else {
            0.0
        }
    })
}

/// How many screen pixels a sprite rises for its world offset's height
/// `z`: `z * (2 << zoom)`, rounded as the original's `__ftol` does (add a
/// half, then truncate toward zero). This is the original's catalog
/// preview (0x438ae4 to 0x438b58, read by the old prototype in
/// `legacy/crates/sims-render/src/object_preview/software.rs`); that the
/// lot view places sprites the same way is **unverified**.
fn sprite_rise(object: [f32; 3], zoom: Zoom) -> i32 {
    let z = sprite_offset(object)[2];
    if z == 0.0 {
        return 0;
    }
    (z * (2 << zoom.level()) as f32 + 0.5).trunc() as i32
}

/// The depth anchor of a draw group sprite that sits `object` away from
/// an object at `position` (fine units) facing `facing` (data word 1), on
/// ground `base` near-zoom pixels up. The offset's x and y are sixteenths
/// of a tile in the object's own frame, and turn with it a quarter turn per
/// two steps of facing, as the old prototype turned them; z is in height
/// units of 16 near-zoom pixels (2 << 3, as [`sprite_rise`]). The old
/// prototype read the same units (`legacy/crates/sims-render/src/lib.rs`
/// divides x and y by 16 to get tiles).
fn part_anchor(
    camera: &Camera,
    position: [i32; 3],
    facing: i16,
    object: [f32; 3],
    base: i32,
) -> i32 {
    let [mut x, mut y, z] = sprite_offset(object);
    for _ in 0..(i32::from(facing) / 2).rem_euclid(4) {
        (x, y) = (-y, x);
    }
    let fine = [
        position[0] + x.round() as i32,
        position[1] + y.round() as i32,
    ];
    let up = (z * 16.0).round() as i32;
    depth::point(camera.rotate_fine(fine), base + up)
}

/// The draw group view (one bit of 1, 4, 16 and 64) for an object facing
/// `facing` (data word 1: 0, 2, 4 or 6), turned by the camera's rotation
/// (the old prototype's reading in `legacy/crates/sims-render/src/lib.rs`).
pub fn view_direction(facing: i16, camera: &Camera) -> u32 {
    let bits = (1u32 << (facing & 7)).rotate_left(u32::from(camera.rotation & 3) * 2);
    (bits & 255) | (bits >> 8)
}

/// The wall an object in a wall stands in. A door or window faces along
/// its placement bit (data word 13) turned by its facing (data word 1),
/// as the original's 0x4358c3 does (the old prototype's `normal` in
/// `legacy/crates/sims-sim/src/lot_presenter/object_cutaway.rs`).
fn object_edge(position: [i32; 3], data: &[i16]) -> Option<Edge> {
    let placement = data.get(13).copied().unwrap_or(0) as u16;
    let facing = data.get(1).copied().unwrap_or(0);
    let bit: u16 = match placement & 5 {
        1 => 2,
        4 => 8,
        _ => return None,
    };
    let turn = (i32::from(facing) / 2).rem_euclid(4) as u32;
    let normal = ((bit << turn) | (bit >> (4 - turn))) & 15;
    let [x, y] = [position[0] >> 4, position[1] >> 4];
    let (a, b) = match normal {
        1 => ([x, y], [x, y + 1]),
        2 => ([x, y], [x + 1, y]),
        4 => ([x + 1, y], [x + 1, y + 1]),
        8 => ([x, y + 1], [x + 1, y + 1]),
        _ => return None,
    };
    Some(edge(a, b))
}

/// Whether a wall style can be cut down: not a fence, railing or the
/// like (styles 2, 12, 13, 14 and 255; 0x54059a).
fn solid(style: u16) -> bool {
    !matches!(style, 2 | 12 | 13 | 14 | 255)
}

/// The wall style whose cut-down shape a cut wall with a door or window in
/// it takes.
const PLAIN_STYLE: u16 = 1;

/// A wall face: the pattern's frame cut to the shape's frame. A wall with
/// style 22 or 23 takes its shape from the door or window in it when it is
/// up. Cut down, a wall of style 3, 5, 6, 15, 22 or 23 (the styles the
/// original marks with bit 8 when it cuts them, 0x54059a; doors stand in
/// style 3) is a plain wall's stub: the openings' own cut sprites are one
/// small outline, and the original shows the same stub under a window or
/// door as beside it (screenshot comparison, 2026-11-14; which style's
/// stub it takes is unverified).
fn wall_image(
    assets: &mut Assets,
    pattern: u16,
    style: u16,
    opening: Option<&Definition>,
    cut: bool,
    zoom: usize,
    frame: usize,
) -> Option<Image> {
    let shape = match opening {
        _ if cut && matches!(style, 3 | 5 | 6 | 15 | 22 | 23) => {
            assets.shape(PLAIN_STYLE, true, zoom)?[frame].clone()
        }
        Some(definition) if matches!(style, 22 | 23) => {
            assets.object_shape(definition, false, zoom)?[frame].clone()
        }
        _ => assets.shape(style, cut, zoom)?[frame].clone(),
    };
    assets.pattern(pattern, zoom)?[frame].masked(&shape)
}

/// One half of a floor image for a tile a diagonal wall splits: half 0
/// holds corner (0, 0) under a descending wall, or corner (0, 1) under an
/// ascending one.
fn split_half(camera: &Camera, image: &Image, segments: u8, half: usize) -> Image {
    let half_w = 8 << camera.zoom.level();
    let half_h = 4 << camera.zoom.level();
    let mut out = image.clone();
    for py in 0..image.height {
        for px in 0..image.width {
            // The pixel's view position inside the tile, in sixteenths,
            // from its offset below the top corner.
            let s = py as i32 * 16 / half_h;
            let d = (px as i32 - image.width as i32 / 2) * 16 / half_w;
            let [x, y] = unrotate_local(camera, [(s + d) / 2, (s - d) / 2]);
            let first = if segments & lot::DESCENDING != 0 {
                x + y < 16
            } else {
                x < y
            };
            if first != (half == 0) {
                out.rgba[(py * image.width + px) * 4 + 3] = 0;
            }
        }
    }
    out
}

/// A position inside a tile (sixteenths), from view to lot axes.
fn unrotate_local(camera: &Camera, [x, y]: [i32; 2]) -> [i32; 2] {
    match camera.rotation & 3 {
        1 => [y, 16 - x],
        2 => [16 - x, 16 - y],
        3 => [16 - y, x],
        _ => [x, y],
    }
}

/// Whether a dynamic sprite shows: an object keeps one word per dynamic
/// sprite, counted from its definition's first dynamic sprite ID, and a
/// sprite outside that range always shows (the original's 0x437f99, read
/// by `legacy/crates/sims-render/src/lib.rs`).
pub fn sprite_visible(sprite: u16, base: i16, flags: &[i16]) -> bool {
    let index = i32::from(sprite) - i32::from(base);
    match usize::try_from(index).ok().and_then(|i| flags.get(i)) {
        Some(&flag) => flag != 0,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(rotation: u8) -> Camera {
        Camera {
            zoom: Zoom::Near,
            rotation,
            level: 1,
            origin: [0, 0],
            walls: Walls::Up,
            focus: crate::Focus::default(),
        }
    }

    #[test]
    fn trim_frames_follow_the_original_table() {
        assert_eq!(trim_frame(0), None);
        assert_eq!(trim_frame(1), Some(0));
        // A straight wall through a corner, each way.
        assert_eq!(trim_frame(1 | 4), Some(4));
        assert_eq!(trim_frame(2 | 8), Some(9));
        assert_eq!(trim_frame(0x10), Some(15));
        assert_eq!(trim_frame(0x14), Some(16));
        assert_eq!(trim_frame(0x10 | 0x40), Some(29));
        assert_eq!(trim_frame(0xf0), Some(45));
        // No frame for a diagonal with a wall at right angles to it.
        assert_eq!(trim_frame(0x11), None);
        assert_eq!(trim_frame(0xff), None);
    }

    #[test]
    fn a_wall_marks_opposite_bits_at_its_two_ends() {
        for d in [[0, -1], [-1, 0], [-1, -1], [-1, 1]] {
            let (a, b) = (trim_bit(d), trim_bit([-d[0], -d[1]]));
            assert_ne!(a, 0);
            // Both ends of a wall, straight through a corner, have a frame.
            assert!(trim_frame(a | b).is_some(), "{d:?}");
            assert_eq!(a.count_ones() + b.count_ones(), (a | b).count_ones());
        }
        assert_eq!(trim_bit([2, 0]), 0);
    }

    #[test]
    fn a_wall_takes_light_from_the_triangle_touching_it() {
        // Sample points land inside the tile, in the triangle that holds
        // the wall's side (1 SW, 2 SE, 3 NE, 4 NW; see `Lot::room_at`).
        let [west, north, descending, ascending] = segments(3, 5);
        assert_eq!(wall_side(&west, 0), [50, 88]);
        assert_eq!(wall_side(&west, 1), [46, 88]);
        assert_eq!(wall_side(&north, 0), [56, 82]);
        assert_eq!(wall_side(&descending, 0), [58, 90]);
        assert_eq!(wall_side(&descending, 1), [54, 86]);
        assert_eq!(wall_side(&ascending, 0), [58, 86]);
        assert_eq!(wall_side(&ascending, 1), [54, 90]);
    }

    #[test]
    fn view_directions_turn_with_the_camera() {
        assert_eq!(view_direction(0, &camera(0)), 1);
        assert_eq!(view_direction(2, &camera(0)), 4);
        assert_eq!(view_direction(0, &camera(1)), 4);
        assert_eq!(view_direction(6, &camera(1)), 1);
        assert_eq!(view_direction(4, &camera(3)), 4);
    }

    #[test]
    fn sprite_world_offsets_move_depth_with_the_object_and_camera() {
        let position = [100, 200, 1];
        let base = 32;
        let anchor = depth::point(camera(0).rotate_fine([100, 200]), base);
        assert_eq!(part_anchor(&camera(0), position, 0, [0.0; 3], base), anchor);
        // One height unit up is 16 near-zoom pixels, and nearer.
        assert_eq!(
            part_anchor(&camera(0), position, 0, [0.0, 0.0, 1.0], base),
            depth::point([100, 200], base + 16)
        );
        // Sixteen units along x is one tile, at facing 0; at facing 2 (a
        // quarter turn) x becomes y.
        assert_eq!(
            part_anchor(&camera(0), position, 0, [16.0, 0.0, 0.0], base),
            depth::point([116, 200], base)
        );
        assert_eq!(
            part_anchor(&camera(0), position, 2, [16.0, 0.0, 0.0], base),
            depth::point([100, 216], base)
        );
        // The camera's rotation turns the moved point like any other.
        assert_eq!(
            part_anchor(&camera(1), position, 2, [16.0, 0.0, 0.0], base),
            depth::point(camera(1).rotate_fine([100, 216]), base)
        );
        // Nonsense in a file stays bounded.
        for bad in [f32::NAN, f32::INFINITY, f32::MAX, f32::MIN] {
            let _ = part_anchor(&camera(3), position, i16::MIN, [bad, bad, bad], base);
            let _ = sprite_rise([bad, bad, bad], Zoom::Near);
        }
    }

    #[test]
    fn a_sprite_rises_by_its_height_at_each_zoom() {
        // 2 << zoom pixels per height unit, rounded half up toward zero.
        assert_eq!(sprite_rise([0.0, 0.0, 0.25], Zoom::Near), 4);
        assert_eq!(sprite_rise([0.0, 0.0, 0.25], Zoom::Medium), 2);
        assert_eq!(sprite_rise([0.0, 0.0, 0.25], Zoom::Far), 1);
        assert_eq!(sprite_rise([0.0, 0.0, 0.4082483], Zoom::Near), 7);
        assert_eq!(sprite_rise([5.0, -3.0, 0.0], Zoom::Near), 0);
        // __ftol truncates after adding a half: -0.25 at the near zoom is
        // -4 + 0.5 = -3.5, which truncates to -3.
        assert_eq!(sprite_rise([0.0, 0.0, -0.25], Zoom::Near), -3);
    }

    #[test]
    fn dynamic_sprites_hide_only_when_their_word_is_zero() {
        assert!(sprite_visible(10, 20, &[0, 0]));
        assert!(!sprite_visible(20, 20, &[0, 1]));
        assert!(sprite_visible(21, 20, &[0, 1]));
        assert!(sprite_visible(22, 20, &[0, 1]));
        assert!(sprite_visible(20, 20, &[]));
    }

    #[test]
    fn doors_stand_in_the_wall_their_placement_names() {
        let at = [5 * 16 + 8, 7 * 16 + 8, 1];
        let mut data = vec![0; 14];
        data[13] = 1;
        assert_eq!(object_edge(at, &data), Some([[5, 7], [6, 7]]));
        data[1] = 2;
        assert_eq!(object_edge(at, &data), Some([[6, 7], [6, 8]]));
        data[13] = 4;
        data[1] = 0;
        assert_eq!(object_edge(at, &data), Some([[5, 8], [6, 8]]));
        data[13] = 0;
        assert_eq!(object_edge(at, &data), None);
    }

    #[test]
    fn split_floors_keep_complementary_halves() {
        let image = Image {
            width: 127,
            height: 64,
            rgba: [9, 9, 9, 255].repeat(127 * 64),
            depth: None,
        };
        for rotation in 0..4 {
            for segments in [lot::DESCENDING, lot::ASCENDING] {
                let c = camera(rotation);
                let a = split_half(&c, &image, segments, 0);
                let b = split_half(&c, &image, segments, 1);
                let opaque = |i: &Image| {
                    (0..i.rgba.len() / 4)
                        .filter(|&n| i.rgba[n * 4 + 3] != 0)
                        .count()
                };
                assert_eq!(opaque(&a) + opaque(&b), 127 * 64);
                assert!(opaque(&a) > 127 * 16 && opaque(&b) > 127 * 16);
            }
        }
    }
}
