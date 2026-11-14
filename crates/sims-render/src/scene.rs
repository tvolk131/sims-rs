//! Draws a lot: terrain, floors, walls and objects, level by level.

use crate::assets::{Assets, Definition};
use crate::camera::{self, Camera, Walls, Zoom};
use crate::canvas::{Canvas, Image};
use crate::depth;
use crate::lot::{self, Lot, WallCell};
use sims_sim::World;
use std::collections::{BTreeMap, BTreeSet};

/// Grass colours: the original blends from brown to green in 32 steps by
/// each tile's grass value, a light and a dark shade (the old prototype's
/// `legacy/crates/sims-render/src/grass.rs`). The original draws blades
/// from a seeded pattern; this draws a fixed scatter of the two shades.
const LIGHT_BROWN: [u8; 3] = [157, 117, 65];
const LIGHT_GREEN: [u8; 3] = [80, 116, 59];
const DARK_BROWN: [u8; 3] = [81, 60, 18];
const DARK_GREEN: [u8; 3] = [14, 53, 8];

/// What the window shows outside the lot.
pub const BACKGROUND: [u8; 3] = [0, 0, 0];

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
        stats: Stats::default(),
    };
    for level in 1..=camera.level.clamp(1, 2) {
        painter.floors(level);
        let cut = painter.walls(level);
        painter.objects(level, &cut);
    }
    painter.stats
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

struct Painter<'a> {
    world: &'a World,
    lot: &'a Lot,
    camera: &'a Camera,
    assets: &'a mut Assets,
    canvas: &'a mut Canvas,
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
        let index = (level - 1) as usize;
        let zoom = self.camera.zoom.index();
        let base = level_elevation(level);
        for x in 0..self.lot.size as i32 {
            for y in 0..self.lot.size as i32 {
                let cell = Lot::cell(x as usize, y as usize);
                // The ground shows wherever a floor leaves it uncovered,
                // such as half of a split tile.
                if level == 1 {
                    self.terrain([x, y]);
                }
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
                    let camera = self.camera;
                    self.canvas.draw(&image, at, false, |_, y, _| {
                        depth::ground(camera, y, elevation)
                    });
                    self.stats.floors += 1;
                }
            }
        }
    }

    /// Fills a grass tile as two triangles between its corners.
    fn terrain(&mut self, [x, y]: [i32; 2]) {
        let g = i32::from(self.lot.grass[Lot::cell(x as usize, y as usize)]).clamp(0, 31);
        let light = lerp(LIGHT_BROWN, LIGHT_GREEN, g);
        let dark = lerp(DARK_BROWN, DARK_GREEN, g);
        let points = [[x, y], [x + 1, y], [x + 1, y + 1], [x, y + 1]].map(|c| {
            let v = self.rotate_corner(c);
            (
                self.corner_screen(c, 1),
                depth::point([v[0] * 16, v[1] * 16], self.corner_elevation(c)),
            )
        });
        let origin = self.camera.origin;
        for [a, b, c] in [[0, 1, 2], [0, 2, 3]] {
            fill_triangle(self.canvas, points[a], points[b], points[c], |x, y| {
                if speckle(x - origin[0], y - origin[1]) {
                    dark
                } else {
                    light
                }
            });
        }
    }

    /// The wallpaper on each side of a wall (first along its normal), and
    /// the tile on each side (the old prototype's `faces` in
    /// `legacy/crates/sims-render/src/walls.rs`, checked there against the
    /// original's wall records).
    fn sides(&self, level: usize, x: i32, y: i32, bit: u8) -> ([u16; 2], [[i32; 2]; 2]) {
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
            lot::WEST => (
                [t.patterns[0], cell(x - 1, y).patterns[3]],
                [[x, y], [x - 1, y]],
            ),
            lot::NORTH => (
                [t.patterns[1], cell(x, y - 1).patterns[2]],
                [[x, y], [x, y - 1]],
            ),
            _ => ([t.patterns[3], t.patterns[2]], [[x + 1, y], [x - 1, y]]),
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

    /// Draws a level's walls and returns the ones drawn cut down.
    fn walls(&mut self, level: i32) -> BTreeSet<Edge> {
        let index = (level - 1) as usize;
        let zoom = self.camera.zoom.index();
        let base = level_elevation(level);
        let rise = 4 << self.camera.zoom.level();
        let mut cut_edges = BTreeSet::new();
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
                    let (patterns, tiles) = self.sides(index, x, y, s.bit);
                    let n = self.rotate_direction(s.normal);
                    let front = usize::from(n[0] + n[1] < 0);
                    let cut = match self.camera.walls {
                        Walls::Up => false,
                        Walls::Down => true,
                        // Up only if it is the back wall of a room: indoors on
                        // the camera's side and outdoors behind.
                        Walls::Cutaway => {
                            let [near, far] = [tiles[front], tiles[1 - front]];
                            self.lot.outdoors(index, near[0], near[1])
                                || !self.lot.outdoors(index, far[0], far[1])
                        }
                    };
                    let key = edge(s.from, s.to);
                    if cut {
                        cut_edges.insert(key);
                    }
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
                    let camera = self.camera;
                    self.canvas.draw(&image, at, false, |sx, sy, _| {
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
        cut_edges
    }

    fn objects(&mut self, level: i32, cut: &BTreeSet<Edge>) {
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
            let view = self.camera.rotate_fine([x, y]);
            // The ground under the object: the mean of its tile's corners.
            let ground = (0..4)
                .map(|c| self.corner_elevation([tile[0] + (c & 1), tile[1] + (c >> 1)]))
                .sum::<i32>()
                / 4;
            let anchor = depth::point(view, level_elevation(level) + ground);
            list.push((anchor, id, view, ground));
        }
        // Far objects first, so soft edges blend over what is behind them.
        list.sort_by_key(|&(d, id, _, _)| (std::cmp::Reverse(d), id));
        for (anchor, id, view, ground) in list {
            let o = &self.world.objects[&id];
            let Some(definition) = self.assets.definition(o.guid) else {
                self.stats.missing_objects += 1;
                continue;
            };
            // Doors and windows hide with their wall when it is cut down
            // (simplified from the original's 0x4aa446; see the decisions
            // log).
            if definition.shapes_wall()
                && object_edge(o.position, &o.data).is_some_and(|e| cut.contains(&e))
            {
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
            for part in parts {
                if !sprite_visible(part.sprite, definition.dynamic_base, &o.dynamic_sprites) {
                    continue;
                }
                // Offsets come from the files and may be anything; the
                // canvas clips whatever this gives.
                let height = i32::try_from(part.image.height).unwrap_or(i32::MAX);
                let at = [
                    screen[0].saturating_add(part.offset[0]),
                    screen[1]
                        .saturating_add(part.offset[1])
                        .saturating_sub(height),
                ];
                let anchor = if part.object == [0.0; 3] {
                    anchor
                } else {
                    part_anchor(self.camera, o.position, facing, part.object, base)
                };
                self.canvas.draw(&part.image, at, part.flip, |_, _, byte| {
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

/// Screen pixels at the near zoom per tile of height: a tile's side seen
/// from the original's camera, `32 * sqrt(6)` (the old prototype's
/// projection in `legacy/crates/sims-render/src/lib.rs`).
const NEAR_PIXELS_PER_TILE_UP: f32 = 78.383_67;

/// The depth anchor of a draw group sprite that sits `object` (tiles: x
/// and y in the object's frame, z up) away from an object at `position`
/// (fine units) facing `facing` (data word 1), on ground `base` near-zoom
/// pixels up. The offset turns with the object, a quarter turn per two
/// steps of facing, as the old prototype turned it. Offsets are clamped to
/// a lot's width, so nonsense in a file cannot overflow the depth.
fn part_anchor(
    camera: &Camera,
    position: [i32; 3],
    facing: i16,
    object: [f32; 3],
    base: i32,
) -> i32 {
    let limit = camera::ARRAY_SIZE as f32;
    let [mut x, mut y, z] = object.map(|v| {
        if v.is_finite() {
            v.clamp(-limit, limit)
        } else {
            0.0
        }
    });
    for _ in 0..(i32::from(facing) / 2).rem_euclid(4) {
        (x, y) = (-y, x);
    }
    let fine = [
        position[0] + (x * 16.0).round() as i32,
        position[1] + (y * 16.0).round() as i32,
    ];
    let up = (z * NEAR_PIXELS_PER_TILE_UP).round() as i32;
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

/// The wall style whose cut-down shape a cut wall with a door or window in
/// it takes.
const PLAIN_STYLE: u16 = 1;

/// A wall face: the pattern's frame cut to the shape's frame. A wall with
/// style 22 or 23 takes its shape from the door or window in it when it is
/// up. Cut down, it is a plain wall's stub: the opening's own cut sprites
/// are one small outline shared by every door and window, and the
/// original shows the same stub under a window as beside it (screenshot
/// comparison, 2026-11-14; which style's stub it takes is unverified).
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
        Some(_) if cut && matches!(style, 22 | 23) => {
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

fn lerp(a: [u8; 3], b: [u8; 3], t: i32) -> [u8; 3] {
    std::array::from_fn(|i| (i32::from(a[i]) + (i32::from(b[i]) - i32::from(a[i])) * t / 31) as u8)
}

/// A fixed scatter of dark grass pixels, about one in four, tied to the
/// lot so it moves with it.
fn speckle(x: i32, y: i32) -> bool {
    let mut n = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    n ^= n >> 15;
    n = n.wrapping_mul(0x2C1B_3C6D);
    n ^= n >> 12;
    n & 3 == 0
}

/// Fills a triangle, interpolating depth, with a colour per pixel. Pixels
/// on an edge two triangles share are filled by both; the depth test keeps
/// that harmless.
fn fill_triangle(
    canvas: &mut Canvas,
    (a, da): ([i32; 2], i32),
    (b, db): ([i32; 2], i32),
    (c, dc): ([i32; 2], i32),
    color: impl Fn(i32, i32) -> [u8; 3],
) {
    let area = i64::from((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]));
    if area == 0 {
        return;
    }
    let x0 = a[0].min(b[0]).min(c[0]).max(0);
    let x1 = a[0].max(b[0]).max(c[0]).min(canvas.width as i32 - 1);
    let y0 = a[1].min(b[1]).min(c[1]).max(0);
    let y1 = a[1].max(b[1]).max(c[1]).min(canvas.height as i32 - 1);
    let side = |p: [i32; 2], q: [i32; 2], x: i32, y: i32| {
        i64::from((q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0]))
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let w = [side(b, c, x, y), side(c, a, x, y), side(a, b, x, y)];
            let inside = if area > 0 {
                w.iter().all(|&w| w >= 0)
            } else {
                w.iter().all(|&w| w <= 0)
            };
            if inside {
                let d = (w[0] * i64::from(da) + w[1] * i64::from(db) + w[2] * i64::from(dc)) / area;
                canvas.plot(x, y, d as i32, color(x, y));
            }
        }
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
        }
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
        // Up is nearer: one tile up is 78 near-zoom pixels.
        assert_eq!(
            part_anchor(&camera(0), position, 0, [0.0, 0.0, 1.0], base),
            depth::point([100, 200], base + 78)
        );
        // An offset along x, facing 0 and facing 2 (a quarter turn: x
        // becomes y).
        assert_eq!(
            part_anchor(&camera(0), position, 0, [1.0, 0.0, 0.0], base),
            depth::point([116, 200], base)
        );
        assert_eq!(
            part_anchor(&camera(0), position, 2, [1.0, 0.0, 0.0], base),
            depth::point([100, 216], base)
        );
        // The camera's rotation turns the moved point like any other.
        assert_eq!(
            part_anchor(&camera(1), position, 2, [1.0, 0.0, 0.0], base),
            depth::point(camera(1).rotate_fine([100, 216]), base)
        );
        // Nonsense in a file stays bounded.
        for bad in [f32::NAN, f32::INFINITY, f32::MAX, f32::MIN] {
            let _ = part_anchor(&camera(3), position, i16::MIN, [bad, bad, bad], base);
        }
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

    #[test]
    fn triangles_fill_inside_with_interpolated_depth() {
        let mut canvas = Canvas::new(10, 10, [0, 0, 0]);
        fill_triangle(
            &mut canvas,
            ([0, 0], 0),
            ([9, 0], 90),
            ([0, 9], 0),
            |_, _| [1, 1, 1],
        );
        assert_eq!(canvas.color[0], [1, 1, 1]);
        assert_eq!(canvas.depth[5], 50);
        assert_eq!(canvas.color[99], [0, 0, 0]);
    }
}
