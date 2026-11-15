//! Draws a lot: terrain, floors, walls and objects, level by level.

use crate::assets::{Assets, Definition};
use crate::camera::{self, Camera, Walls, Zoom};
use crate::canvas::{Canvas, Image};
use crate::caps::{self, Cap, Corners, Line, solid};
use crate::cutaway::{self, Mask};
use crate::depth::{Anchor, Depth};
use crate::grass::{self, Grass};
use crate::light::Light;
use crate::lot::{self, Lot, WallCell};
use crate::pools::{self, Pools};
use sims_sim::{Object, World};
use std::collections::{BTreeMap, BTreeSet};

/// Rows between a tile's left corner and the top of its first grass
/// column (matched against the original's screen).
const GRASS_ROW: i32 = 0;
const GRASS_COLUMN: i32 = 1;

/// Object sprites sit one pixel right of the point the camera projects
/// them to, measured against the walls and floors around them. The camera
/// origin is fitted to the tile art, and the original places the two
/// kinds of art one pixel apart; at every compared camera (far and
/// medium zoom) the gap is this same pixel. Near zoom is unverified (see
/// the decisions log, 2026-11-14).
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
        depth: Depth::new(camera),
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
        let corners = painter.corners(level, &cut);
        painter.walls(level, &cut, &corners);
        painter.trims(level, &corners);
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

/// A level's walls and corners, and each wall's lot cell and segment bit.
struct Level {
    corners: Corners,
    faces: Vec<([i32; 2], u8)>,
    /// Each wall's index, by its cell and segment bit.
    index: BTreeMap<([i32; 2], u8), usize>,
}

impl Level {
    /// A wall's flags (see `caps::Corners`), by its cell and segment bit.
    fn flags(&self, face: ([i32; 2], u8)) -> u8 {
        self.index.get(&face).map_or(0, |&i| self.corners.flags[i])
    }
}

/// A wall segment by its two lot corners, the lower first.
type Edge = [[i32; 2]; 2];

fn edge(a: [i32; 2], b: [i32; 2]) -> Edge {
    if a <= b { [a, b] } else { [b, a] }
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
    depth: Depth,
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

    /// The lamps whose pools reach tile (`x`, `y`) of a level, as view
    /// offsets from the lamp to the tile: those of the tile's record
    /// within the pool map's reach each way (0x4c8049).
    fn lamps(&self, level: usize, [x, y]: [i32; 2]) -> [Option<[i32; 2]>; 2] {
        let record = usize::try_from(x)
            .ok()
            .zip(usize::try_from(y).ok())
            .and_then(|(x, y)| self.lot.lamps[level].get(Lot::cell(x, y)).copied())
            .unwrap_or_default();
        record.0.map(|lamp| {
            let offset = lamp?.map(i32::from);
            offset
                .iter()
                .all(|n| n.abs() <= pools::REACH)
                .then(|| self.rotate_direction(offset))
        })
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
    fn rotate_direction(&self, direction: [i32; 2]) -> [i32; 2] {
        rotate_direction(self.camera.rotation, direction)
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
                let p = self.corner_screen(top, level);
                let (view, height) = (self.rotate_corner(top), self.corner_elevation(top) / 16);
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
                    let byte = self
                        .lot
                        .light_at(index, [x * 16 + inside[0], y * 16 + inside[1]]);
                    let lamps = self.lamps(index, [x, y]);
                    let depth = &self.depth;
                    let depth = |_, y, _| Some(depth.floor(view, height, level, at[1], y));
                    match (Light::drawn_byte(byte), self.assets.pools()) {
                        // A room in full light shows no pools (0x4c83d4).
                        (Some(base), Some(pools)) if lamps.iter().any(Option::is_some) => {
                            let tile = self.camera.zoom.tile_width();
                            let origins = lamps.map(|lamp| lamp.map(|a| Pools::origin(tile, a)));
                            let light = |ix: usize, iy: usize| {
                                let pool = |o: [i32; 2]| {
                                    u32::from(pools.at(zoom, o[0] + ix as i32, o[1] + iy as i32))
                                };
                                Light::from_sum(
                                    base + origins.iter().flatten().map(|&o| pool(o)).sum::<u32>(),
                                )
                            };
                            self.canvas.draw_shaded(&image, at, light, depth);
                        }
                        _ => self
                            .canvas
                            .draw_lit(&image, at, false, Light::from_byte(byte), depth),
                    }
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
        // The whole tile takes one depth, from its top corner.
        let depth = self.depth.grass(view, i32::from(heights[1]));
        let left = corner[0] + GRASS_COLUMN;
        for column in grass::columns(left, corner[1] + GRASS_ROW, heights, shades, colors, zoom) {
            for (row, pixels) in column.rows() {
                for (i, &p) in pixels.iter().enumerate() {
                    self.canvas
                        .plot(column.x + i as i32, row, depth, lit.apply(grass::widen(p)));
                }
            }
            let index = ((column.x - left) / 4) as usize;
            for (bx, foot, height, p) in self.grass.blades(view, &column, index) {
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
    fn walls(&mut self, level: i32, cut: &Cut, corners: &Level) {
        let index = (level - 1) as usize;
        let zoom = self.camera.zoom.index();
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
                    let opening = openings.get(&key);
                    let pattern = patterns[front];
                    let ends = [s.from, s.to].map(|c| self.corner_screen(c, level));
                    let heights = [s.from, s.to].map(|c| self.corner_elevation(c) / 16);
                    let frame = wall_frame(ends);
                    // A cut wall marked for a cap at one end stands full
                    // height there: the original draws it in shape 7 or 8,
                    // then lays a strip of trim-coloured pattern (252 or
                    // 253, shape 9 or 10) down that end (0x53ea55 to
                    // 0x53ebb5). Door-like walls are drawn as usual.
                    let flags = corners.flags(([x, y], s.bit));
                    let capped = (flags & 6 != 0 && !caps::door_like(style))
                        .then_some(if flags & 2 != 0 { 8 } else { 7 });
                    // Fences have no cut shape and stay up.
                    let image = match capped {
                        Some(shape) => {
                            wall_image(self.assets, pattern, shape, None, false, zoom, frame)
                        }
                        None => wall_image(self.assets, pattern, style, opening, cut, zoom, frame)
                            .or_else(|| {
                                wall_image(self.assets, pattern, style, opening, false, zoom, frame)
                            }),
                    };
                    let Some(image) = image else {
                        self.stats.missing_walls += 1;
                        continue;
                    };
                    // The side that shows takes the light of the room on
                    // that side: sampled just beside the wall's middle, so
                    // a split tile gives the triangle touching the wall.
                    let light = self.light(index, wall_side(&s, front));
                    self.place_wall(&image, frame, ends, heights, level, light, None);
                    if let Some(shape) = capped {
                        let (trim, mask) = if shape == 7 { (252, 9) } else { (253, 10) };
                        if let Some(strip) =
                            wall_image(self.assets, trim, mask, None, false, zoom, frame)
                        {
                            // At full light, like the trim along the top.
                            self.place_wall(&strip, frame, ends, heights, level, Light::FULL, None);
                        }
                    }
                    self.stats.walls += 1;
                }
            }
        }
    }

    /// Draws a wall face on `level` standing between the screen points
    /// `a` and `b` of its ends, on terrain `heights` steps up at each,
    /// only within screen columns `columns` (from, to) when given.
    ///
    /// The wall is placed at its end lower on the screen (the left one of
    /// a wall level on screen), and a straight wall's columns run up or
    /// down its column table from there; both were fitted to the
    /// original's depth buffer at medium zoom (2026-11-14).
    #[allow(clippy::too_many_arguments)]
    fn place_wall(
        &mut self,
        image: &Image,
        frame: usize,
        [a, b]: [[i32; 2]; 2],
        heights: [i32; 2],
        level: i32,
        light: Light,
        columns: Option<[i32; 2]>,
    ) {
        let rise = 4 << self.camera.zoom.level();
        let left = if a[0] <= b[0] { a } else { b };
        let (w, h) = (image.width as i32, image.height as i32);
        // The sprite's bottom row sits one pixel above the corner's
        // projected point (matched against the original's screen,
        // 2026-11-14).
        let at = match frame {
            0 | 2 => [left[0], left[1] - h],
            1 => [left[0], left[1] - (h - rise)],
            _ => {
                let bottom = if a[1] > b[1] { a } else { b };
                [bottom[0] - w / 2, bottom[1] - h]
            }
        };
        let anchor = match (frame, a[1].cmp(&b[1])) {
            (2, _) if a[0] <= b[0] => self.depth.anchor(a[1], heights[0] as f32, level),
            (2, _) => self.depth.anchor(b[1], heights[1] as f32, level),
            // An edge-on diagonal is anchored halfway between its ends:
            // with the original's bias (see `Depth::diagonal`) that gives
            // its depth on every row compared (2026-11-15). Its height
            // there, the ends' mean, is unverified.
            (3, _) => {
                let height = (heights[0] + heights[1]) as f32 / 2.0;
                self.depth.anchor((a[1] + b[1]) / 2, height, level)
            }
            (_, std::cmp::Ordering::Less) => self.depth.anchor(b[1], heights[1] as f32, level),
            _ => self.depth.anchor(a[1], heights[0] as f32, level),
        };
        let sprite_left = at[0];
        let clipped;
        let (image, at) = match columns {
            Some(columns) => {
                let Some((part, skip)) = within(image, at[0], columns) else {
                    return;
                };
                clipped = part;
                (&clipped, [at[0].saturating_add(skip), at[1]])
            }
            None => (image, at),
        };
        let depth = &self.depth;
        let first = depth.wall_start(frame == 0);
        self.canvas
            .draw_wall(image, at, light, |sx, sy| match frame {
                0 | 1 => depth.wall(anchor, sy, first, sx.saturating_sub(sprite_left)),
                _ => depth.diagonal(anchor, sy, frame == 3),
            });
    }

    /// A level's walls as the original's corner pass sees them (see
    /// `caps`).
    fn corners(&self, level: i32, cut: &Cut) -> Level {
        let index = (level - 1) as usize;
        let size = self.lot.size as i32;
        let mut walls = Vec::new();
        let mut faces = Vec::new();
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
                    walls.push(caps::Wall {
                        lot: [s.from, s.to],
                        view: [s.from, s.to].map(|c| self.rotate_corner(c)),
                        style,
                        cut: cut.cuts(self.camera, [x, y], s.bit, style),
                    });
                    faces.push(([x, y], s.bit));
                }
            }
        }
        Level {
            corners: Corners::new(walls),
            index: faces.iter().enumerate().map(|(i, &f)| (f, i)).collect(),
            faces,
        }
    }

    /// The wood trim along the tops of a level's walls and the caps on
    /// their exposed ends (0x53c227). Each corner a wall touches takes one
    /// trim sprite whose frame shows the walls meeting there: standing
    /// walls' trim at the storey's height, cut walls' at the cut height.
    /// Which walls count, and the cap, follow the original's corner
    /// records (see `caps`). The original draws the trim at full light,
    /// whatever the room's light.
    fn trims(&mut self, level: i32, corners: &Level) {
        let Level { corners, faces, .. } = corners;
        let zoom = self.camera.zoom;
        let half = zoom.tile_width() / 2;
        let cut_height = [6, 12, 23][zoom.index()];
        for (&corner, c) in &corners.corners {
            let at = self.corner_screen(corner, level);
            let view = self.rotate_corner(corner);
            let ground = self.corner_elevation(corner) / 16;
            for (bits, is_cut, height) in [
                (c.cut, true, cut_height),
                (c.standing, false, zoom.floor_height()),
            ] {
                let Some(frame) = trim_frame(bits) else {
                    continue;
                };
                // Cut trim running along a view diagonal through the
                // corner's top or bottom (bits 0x10 and 0x40) comes from the
                // standing walls' set; lone 0x10 sits a little higher when
                // zoomed in.
                let diagonal = is_cut && bits & 0x50 != 0;
                let lift = if is_cut && bits == 0x10 {
                    [0, 2, 5][zoom.index()]
                } else {
                    0
                };
                let set = is_cut && !diagonal;
                // The original subtracts one more pixel from x; its corner
                // point sits one to the right of ours (fitted to the
                // screenshot).
                let Some(x) = at[0].checked_sub(half) else {
                    continue;
                };
                let mut pieces = vec![(set, lift, None)];
                // With 0x10, the cut set's frame is laid again over the
                // part of the corner where a wall leaves along +x (bit 8,
                // right of a point just past the corner) or +y (bit 4,
                // left of a point just short of it), at the plain cut
                // height (0x53c509 to 0x53c640).
                if diagonal && bits & 0x10 != 0 {
                    let z = zoom.index() as i32 + 1;
                    let (step, nudge) = (8 << z, 8 >> (3 - z));
                    if bits & 8 != 0 {
                        pieces.push((true, 0, Some([x + step + nudge, i32::MAX])));
                    }
                    if bits & 4 != 0 {
                        pieces.push((true, 0, Some([i32::MIN, x + step - nudge])));
                    }
                }
                for (set, lift, columns) in pieces {
                    let Some(image) = self.assets.trim(set, zoom).get(frame).cloned() else {
                        continue;
                    };
                    let (image, x) = match columns {
                        Some(columns) => match within(&image, x, columns) {
                            Some((part, skip)) => (part, x.saturating_add(skip)),
                            None => continue,
                        },
                        None => (image, x),
                    };
                    let rise = zoom.tile_height() / 2 + height + lift;
                    let Some(y) = at[1].checked_sub(rise) else {
                        continue;
                    };
                    // The trim lies on the wall tops, one depth a row
                    // (fitted to the original's depth buffer, 2026-11-14).
                    let depth = &self.depth;
                    self.canvas
                        .draw_lit(&image, [x, y], false, Light::FULL, |_, sy, _| {
                            Some(depth.flat(view, ground, level, y, sy))
                        });
                }
            }
            if let Some((cap, low)) = c.cap {
                self.cap(level, corners, faces, corner, c, cap, low);
            }
        }
    }

    /// Draws a corner's cap (0x53c6b7): a narrow upright slice of a wall
    /// face on the cap's line, in the plain wall shape and the pattern of
    /// the wall in the cap's slot, standing in the tile the cap names and
    /// cut to a column beside the corner.
    #[allow(clippy::too_many_arguments)]
    fn cap(
        &mut self,
        level: i32,
        corners: &Corners,
        faces: &[([i32; 2], u8)],
        corner: [i32; 2],
        c: &caps::Corner,
        cap: Cap,
        low: bool,
    ) {
        let Some(owner) = c.slots[cap.slot] else {
            return;
        };
        let index = (level - 1) as usize;
        let face = |i: usize| {
            let ([x, y], bit) = faces[i];
            let s = segments(x, y).into_iter().find(|s| s.bit == bit)?;
            let n = self.rotate_direction(s.normal);
            Some(([x, y], s, usize::from(n[0] + n[1] < 0)))
        };
        let Some(([x, y], s, front)) = face(owner) else {
            return;
        };
        let pattern = self.sides(index, x, y, s.bit)[front];
        // The light of the first wall at the corner on the cap's line,
        // else of the slot's wall (0x53c97f).
        let lit = c
            .slots
            .iter()
            .flatten()
            .copied()
            .find(|&i| corners.walls[i].line() == cap.line)
            .unwrap_or(owner);
        let light = match face(lit) {
            Some((_, s, front)) => self.light(index, wall_side(&s, front)),
            None => return,
        };
        let [tx, ty] = [c.view[0] + cap.offset[0], c.view[1] + cap.offset[1]];
        let ends = match cap.line {
            Line::Y => [[tx, ty], [tx, ty + 1]],
            Line::X => [[tx, ty], [tx + 1, ty]],
            Line::Rising => [[tx, ty], [tx + 1, ty + 1]],
            Line::Falling => [[tx + 1, ty], [tx, ty + 1]],
        };
        // The phantom face stays within the lot's corners.
        let full = lot::FULL as i32;
        if ends.iter().flatten().any(|v| !(0..=full).contains(v)) {
            return;
        }
        // It stands on the corner's own ground (0x53c7e2 projects the
        // corner's position).
        let ground = self.corner_elevation(corner);
        let ends = ends.map(|[x, y]| self.camera.project_fine([x * 16, y * 16], level, ground));
        let zoom = self.camera.zoom.index();
        let frame = wall_frame(ends);
        // The plain wall's shape, or style 255's for a wall of that style
        // (0x53c8c1).
        let style = if corners.walls[owner].style == 255 {
            255
        } else {
            PLAIN_STYLE
        };
        let Some(image) = wall_image(self.assets, pattern, style, None, low, zoom, frame) else {
            return;
        };
        // The column beside the corner, in the original's corner point
        // (one pixel right of ours).
        let cx = self.corner_screen(corner, level)[0].saturating_add(1);
        let width = cap.width << zoom;
        let columns = if cap.right {
            [
                cx.saturating_sub(1),
                cx.saturating_sub(1).saturating_add(width),
            ]
        } else {
            [cx.saturating_sub(width), cx]
        };
        let step = ground / 16;
        self.place_wall(&image, frame, ends, [step; 2], level, light, Some(columns));
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
            let assets = &mut *self.assets;
            let slot = |guid, i: usize| assets.definition(guid)?.slots.get(i).copied();
            let Some(([dx, dy], lift)) = held(&self.world.objects, slot, id) else {
                continue;
            };
            let [x, y] = [x + dx, y + dy];
            let view = self.camera.rotate_fine([x, y]);
            // The ground under the object: the mean of its tile's corners.
            let ground = (0..4)
                .map(|c| self.corner_elevation([tile[0] + (c & 1), tile[1] + (c >> 1)]))
                .sum::<i32>()
                / 4;
            let height = ground as f32 / 16.0 + lift;
            let anchor = place(&self.depth, self.camera, view, level, height);
            list.push((anchor, id, view, ground, [x, y], lift));
        }
        // Far objects first, so soft edges blend over what is behind them.
        list.sort_by_key(|&(a, id, ..)| (std::cmp::Reverse(a.depth), id));
        for (anchor, id, view, ground, spot, lift) in list {
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
            let base = ground as f32 / 16.0;
            // An object takes the light of the room at its position (the
            // original samples the tile light map there, 0x467482). One
            // that gives light (data word 51), other than a window, shows
            // in full light (0x437f7d, 0x4b455f).
            let glows = o.data.get(LIGHT).is_some_and(|&w| w != 0) && definition.kind != WINDOW;
            let light = if glows {
                Light::FULL
            } else {
                Light::from_byte(
                    self.lot
                        .light_at((level - 1) as usize, [o.position[0], o.position[1]]),
                )
            };
            // Each pixel the object draws names it, for a click
            // (`Canvas::id_at`).
            self.canvas.set_id(Some(id));
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
                    let depth = &self.depth;
                    part_anchor(depth, self.camera, spot, facing, object, level, base)
                };
                let depth = &self.depth;
                // A frame without a depth channel stands flat at its
                // anchor's depth. None of the base game's objects has one,
                // and this is not read from the original: it keeps such a
                // frame drawn rather than lost to byte 255.
                let flat = part.image.depth.is_none();
                self.canvas
                    .draw_lit(&part.image, at, part.flip, light, |_, _, byte| {
                        if flat {
                            Some(anchor.depth)
                        } else {
                            depth.sprite(anchor, byte)
                        }
                    });
            }
            self.canvas.set_id(None);
            self.stats.objects += 1;
        }
    }
}

/// Rotates a lot direction into the view at `rotation`, as
/// [`Camera::rotate_fine`] turns positions.
fn rotate_direction(rotation: u8, [x, y]: [i32; 2]) -> [i32; 2] {
    match rotation & 3 {
        1 => [-y, x],
        2 => [-x, -y],
        3 => [y, -x],
        _ => [x, y],
    }
}

/// An object's data word that hides it when not zero (the simulation's
/// `HIDDEN`).
const HIDDEN: usize = 34;

/// An object's data word holding the light it gives.
const LIGHT: usize = 51;

/// The object type of a window (OBJD word 9).
const WINDOW: i16 = 8;

/// The data words naming the object that holds this one and which of
/// its slots it is in.
const CONTAINER: usize = 2;
const CONTAINER_SLOT: usize = 3;

/// How far a held object stands from its own position: a fine x and y
/// offset and a lift in height units. An object another holds (a lamp on
/// an end table, a sink in a counter) stands at its holder's place plus
/// the slot's offset, turned by the holder's facing, and up by the slot's
/// height; the original adds these while it draws the holder's contents
/// (0x437fdd, read by the old prototype in
/// `legacy/crates/sims-render/src/lib.rs`). The game stores a held object
/// at its holder's place, so the offset is added to the object's own
/// position and level, which the caller has checked. Objects a Sim holds
/// get no offset. None when a holder is hidden. A chain of holders that
/// loops gives no offset. Each part is clamped to a lot's width, so
/// nonsense in a save cannot overflow anything. `slot` gives a holder's
/// slot by its GUID and the slot's index.
fn held(
    objects: &BTreeMap<u16, Object>,
    mut slot: impl FnMut(u32, usize) -> Option<[f32; 3]>,
    id: u16,
) -> Option<([i32; 2], f32)> {
    let mut cursor = objects.get(&id)?;
    let mut seen = BTreeSet::from([id]);
    let mut offset = [0.0f32; 3];
    loop {
        let container = cursor.data.get(CONTAINER).copied().unwrap_or(0);
        let Some((holder_id, holder)) = u16::try_from(container)
            .ok()
            .filter(|&c| c != 0)
            .and_then(|c| Some((c, objects.get(&c)?)))
            .filter(|(_, h)| h.person.is_none())
        else {
            break;
        };
        if !seen.insert(holder_id) {
            return Some(([0, 0], 0.0));
        }
        if holder.data.get(HIDDEN).is_some_and(|&w| w != 0) {
            return None;
        }
        let at = usize::try_from(cursor.data.get(CONTAINER_SLOT).copied().unwrap_or(-1))
            .ok()
            .and_then(|i| slot(holder.guid, i))
            .unwrap_or_default();
        let [mut x, mut y, z] = sprite_offset(at);
        for _ in 0..(i32::from(holder.data.get(1).copied().unwrap_or(0)) / 2).rem_euclid(4) {
            (x, y) = (-y, x);
        }
        offset = sprite_offset([offset[0] + x, offset[1] + y, offset[2] + z]);
        cursor = holder;
    }
    Some((
        [offset[0].round() as i32, offset[1].round() as i32],
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

/// Places a point at fine view position `view` on `level`, `height`
/// terrain steps up (16 near-zoom pixels each): the original projects it
/// at its height rounded to a whole step, then takes its depth anchor
/// (0x4b97ea to 0x4c9ceb).
fn place(depth: &Depth, camera: &Camera, view: [i32; 2], level: i32, height: f32) -> Anchor {
    let steps = (height + 0.5).trunc() as i32;
    let rise = steps.saturating_mul(2 << camera.zoom.level());
    let y = camera.project_fine(view, level, 0)[1].saturating_sub(rise);
    depth.anchor(y, height, level)
}

/// The depth anchor of a draw group sprite that sits `object` away from
/// an object at fine position `position` facing `facing` (data word 1),
/// on `level`, on ground `base` terrain steps up. The offset's x and y are
/// sixteenths of a tile in the object's own frame, and turn with it a
/// quarter turn per two steps of facing, as the old prototype turned them;
/// z is in height units of 16 near-zoom pixels (2 << 3, as
/// [`sprite_rise`]), the same as a terrain step. The old prototype read
/// the same units (`legacy/crates/sims-render/src/lib.rs` divides x and y
/// by 16 to get tiles).
fn part_anchor(
    depth: &Depth,
    camera: &Camera,
    position: [i32; 2],
    facing: i16,
    object: [f32; 3],
    level: i32,
    base: f32,
) -> Anchor {
    let [mut x, mut y, z] = sprite_offset(object);
    for _ in 0..(i32::from(facing) / 2).rem_euclid(4) {
        (x, y) = (-y, x);
    }
    let fine = [
        position[0] + x.round() as i32,
        position[1] + y.round() as i32,
    ];
    place(depth, camera, camera.rotate_fine(fine), level, base + z)
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

/// The frame of a wall by its direction on screen, from the screen points
/// of its two ends.
fn wall_frame([a, b]: [[i32; 2]; 2]) -> usize {
    let (left, right) = if a[0] <= b[0] { (a, b) } else { (b, a) };
    if left[0] == right[0] {
        3
    } else if left[1] == right[1] {
        2
    } else if left[1] > right[1] {
        0
    } else {
        1
    }
}

/// The part of an image drawn with its left edge at screen column `x`
/// that falls within screen columns `from` (inclusive) to `to`
/// (exclusive), and how many columns were cut off its left; None if
/// nothing is left.
fn within(image: &Image, x: i32, [from, to]: [i32; 2]) -> Option<(Image, i32)> {
    let width = i64::try_from(image.width).ok()?;
    let left = (i64::from(from) - i64::from(x)).clamp(0, width);
    let right = (i64::from(to) - i64::from(x)).clamp(left, width);
    if left == right {
        return None;
    }
    let (left, right) = (usize::try_from(left).ok()?, usize::try_from(right).ok()?);
    let w = right - left;
    let mut rgba = Vec::with_capacity(w * image.height * 4);
    let mut depth = image
        .depth
        .as_ref()
        .map(|_| Vec::with_capacity(w * image.height));
    for row in 0..image.height {
        let start = row * image.width;
        rgba.extend_from_slice(image.rgba.get((start + left) * 4..(start + right) * 4)?);
        if let (Some(out), Some(d)) = (depth.as_mut(), image.depth.as_ref()) {
            out.extend_from_slice(d.get(start + left..start + right)?);
        }
    }
    let part = Image {
        width: w,
        height: image.height,
        rgba,
        depth,
    };
    Some((part, i32::try_from(left).ok()?))
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
            let (a, b) = (caps::bit(d), caps::bit([-d[0], -d[1]]));
            assert_ne!(a, 0);
            // Both ends of a wall, straight through a corner, have a frame.
            assert!(trim_frame(a | b).is_some(), "{d:?}");
            assert_eq!(a.count_ones() + b.count_ones(), (a | b).count_ones());
        }
        assert_eq!(caps::bit([2, 0]), 0);
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

    /// A lamp's offset to a tile turns as the camera turns their
    /// positions, so its pool stays centred on it in every view, the
    /// off-axis tiles' gradients too.
    #[test]
    fn lamp_offsets_turn_as_the_camera_turns_positions() {
        for rotation in 0..4 {
            let camera = camera(rotation);
            for (lamp, offset) in [([10, 20], [3, 0]), ([10, 20], [-2, 1]), ([40, 5], [1, -3])] {
                let tile = [lamp[0] + offset[0], lamp[1] + offset[1]];
                let [a, b] = [tile, lamp].map(|t| camera.rotate_fine(t.map(|n| n * 16)));
                let turned = rotate_direction(rotation, offset).map(|n| n * 16);
                assert_eq!([a[0] - b[0], a[1] - b[1]], turned, "{rotation} {offset:?}");
            }
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

    fn object(id: u16, guid: u32, position: [i32; 3], words: &[(usize, i16)]) -> Object {
        let mut data = vec![0; 64];
        for &(i, v) in words {
            data[i] = v;
        }
        Object {
            id,
            guid,
            footprint: [0; 4],
            position,
            unknown_short: 0,
            attributes: Vec::new(),
            temps: Vec::new(),
            data,
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: Vec::new(),
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: None,
            registry_word: None,
            last_result: false,
            removal_skips_resets: false,
            selected: false,
        }
    }

    #[test]
    fn held_objects_stand_on_their_holders_slot() {
        const TABLE: u32 = 7;
        let slot = |guid: u32, i: usize| (guid == TABLE && i == 1).then_some([4.0, 0.0, 4.0]);
        let mut objects = BTreeMap::new();
        // A table facing 2 (one quarter turn) holds a lamp in its slot 1.
        objects.insert(1, object(1, TABLE, [100, 200, 1], &[(1, 2)]));
        objects.insert(
            2,
            object(2, 9, [100, 200, 1], &[(CONTAINER, 1), (CONTAINER_SLOT, 1)]),
        );
        // The slot's x turns with the table into y; its height lifts.
        assert_eq!(held(&objects, slot, 2), Some(([0, 4], 4.0)));
        assert_eq!(held(&objects, slot, 1), Some(([0, 0], 0.0)));
        // A holder without that slot puts it at its own place.
        objects.get_mut(&2).unwrap().data[CONTAINER_SLOT] = 5;
        assert_eq!(held(&objects, slot, 2), Some(([0, 0], 0.0)));
        objects.get_mut(&2).unwrap().data[CONTAINER_SLOT] = 1;
        // A hidden holder hides what it holds.
        objects.get_mut(&1).unwrap().data[HIDDEN] = 1;
        assert_eq!(held(&objects, slot, 2), None);
        objects.get_mut(&1).unwrap().data[HIDDEN] = 0;
        // Holders that hold each other, or an object that holds itself,
        // give no offset, however many other objects there are.
        objects.get_mut(&1).unwrap().data[CONTAINER] = 2;
        assert_eq!(held(&objects, slot, 2), Some(([0, 0], 0.0)));
        objects.insert(
            3,
            object(3, TABLE, [0, 0, 1], &[(CONTAINER, 3), (CONTAINER_SLOT, 1)]),
        );
        assert_eq!(held(&objects, slot, 3), Some(([0, 0], 0.0)));
    }

    #[test]
    fn a_long_chain_of_huge_slots_stays_within_a_lot() {
        let huge = |_: u32, _: usize| Some([f32::MAX, -1e30, f32::INFINITY]);
        let mut objects = BTreeMap::new();
        objects.insert(1, object(1, 1, [0, 0, 1], &[]));
        for id in 2..200u16 {
            let below = (id - 1) as i16;
            objects.insert(
                id,
                object(id, 1, [0, 0, 1], &[(CONTAINER, below), (CONTAINER_SLOT, 0)]),
            );
        }
        let limit = camera::ARRAY_SIZE * 16;
        let ([x, y], lift) = held(&objects, huge, 199).unwrap();
        assert!(x.abs() <= limit && y.abs() <= limit);
        assert!(lift.abs() <= limit as f32);
    }

    #[test]
    fn slot_heights_follow_the_category() {
        use crate::assets::slot_height;
        assert_eq!(slot_height(1, 3.0), 0.0);
        assert_eq!(slot_height(2, 0.0), 2.5);
        assert_eq!(slot_height(3, 0.0), 4.0);
        assert_eq!(slot_height(8, 5.0), 4.0);
        assert_eq!(slot_height(4, 0.0).to_bits(), 0x4081_999a);
        assert_eq!(slot_height(0, 5.0), 5.0);
        assert_eq!(slot_height(5, 4.0), 4.0);
    }

    #[test]
    fn sprite_world_offsets_move_depth_with_the_object_and_camera() {
        let position = [100, 200];
        let base = 2.0;
        let d = Depth::new(&camera(0));
        let at =
            |c: u8, fine, height| place(&d, &camera(c), camera(c).rotate_fine(fine), 1, height);
        let part =
            |c: u8, facing, object| part_anchor(&d, &camera(c), position, facing, object, 1, base);
        assert_eq!(part(0, 0, [0.0; 3]), at(0, [100, 200], base));
        // One height unit up is one terrain step, and nearer.
        assert_eq!(part(0, 0, [0.0, 0.0, 1.0]), at(0, [100, 200], base + 1.0));
        assert!(part(0, 0, [0.0, 0.0, 1.0]).depth < part(0, 0, [0.0; 3]).depth);
        // Sixteen units along x is one tile, at facing 0; at facing 2 (a
        // quarter turn) x becomes y.
        assert_eq!(part(0, 0, [16.0, 0.0, 0.0]), at(0, [116, 200], base));
        assert_eq!(part(0, 2, [16.0, 0.0, 0.0]), at(0, [100, 216], base));
        // The camera's rotation turns the moved point like any other.
        assert_eq!(part(1, 2, [16.0, 0.0, 0.0]), at(1, [100, 216], base));
        // Nonsense in a file stays bounded.
        for bad in [f32::NAN, f32::INFINITY, f32::MAX, f32::MIN] {
            let _ = part_anchor(&d, &camera(3), position, i16::MIN, [bad, bad, bad], 1, base);
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
    fn within_keeps_only_the_columns_asked_for() {
        let image = Image {
            width: 4,
            height: 2,
            rgba: (0..32).collect(),
            depth: Some((0..8).collect()),
        };
        let (part, skip) = within(&image, 10, [11, 13]).unwrap();
        assert_eq!((part.width, part.height, skip), (2, 2, 1));
        assert_eq!(part.rgba[..8], image.rgba[4..12]);
        assert_eq!(part.depth, Some(vec![1, 2, 5, 6]));
        // Nothing left, and bounds at the integer limits.
        assert_eq!(within(&image, 10, [0, 10]), None);
        assert_eq!(within(&image, i32::MAX, [i32::MIN, i32::MAX]), None);
        let (all, skip) = within(&image, i32::MIN, [i32::MIN, i32::MAX]).unwrap();
        assert_eq!((all.width, skip), (4, 0));
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
