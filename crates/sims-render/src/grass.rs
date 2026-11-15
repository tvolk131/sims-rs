//! The original's grass: a shaded color per tile corner, filled in
//! four-pixel columns from a dithered palette.
//!
//! Read from the original's code (addresses are in the game's recovered
//! image; see `docs/decisions.md`):
//!
//! - **Palette** (0x49c8d3, 0x49e6cf). 32 grass colors run from brown to
//!   green by the tile's grass value. Each has 128 shades running from a
//!   light color to a dark one. A shade is a 4 by 2 block of pixels: the
//!   shade's own color, with two pixels taken 8 and 6 shades lighter
//!   (the "dither offsets" 0, 8, 0, 6), and the second row mirrored.
//! - **Shading** (0x4a4994, 0x4a4783, 0x4c47f7). A sun lights the terrain.
//!   Each vertex takes the mean of the squared light on the six triangles
//!   around it, becomes a shade from 0 (lit) to 127, and is jittered by up
//!   to two shades from the original's random generator, seeded the same
//!   each time. A tile with a wall on its far edge in the view is 64 shades
//!   darker along that edge. The sun stays where it is when the camera
//!   turns: it always stands toward the view's +x, π/10 from the zenith.
//! - **Grass color** (0x49cd43). Each corner averages the four tiles' grass
//!   values around it and scales them to 0 to 31.
//! - **Columns** (0x49dacd, 0x49da21). A tile is drawn in columns four
//!   pixels wide. The shade and the grass color are interpolated down each
//!   column between the tile's edges, in 256ths.
//!
//! - **Blades** (0x49dfbd, 0x49d79d, 0x49d8fb, 0x49cf9f). 4800 blades are
//!   scattered over a tile's shape once per zoom, each with a rank, from a
//!   fixed seed. A tile draws the blades whose rank falls in a window: as
//!   many as its grass value asks for (500 to 1200, a quarter at the far
//!   zoom), starting at an offset from a random map the lot's size on a
//!   side. A blade is one to four pixels tall and up to 30 shades either
//!   side of 30 shades darker than the ground under it.
//!
//! The vertex jitter and the blade map both depend on the lot's size (52
//! for the Goth house, not the arrays' 64): each draws from its stream
//! once per vertex or tile of the lot.

use crate::camera::{ARRAY_SIZE, Camera};
use crate::lot::{self, Lot};
use std::sync::OnceLock;

/// Grass colors: brown to green.
pub const COLORS: usize = 32;
/// Shades of each color: lit to dark.
pub const SHADES: usize = 128;
const MAX_SHADE: i32 = SHADES as i32 - 1;

const LIGHT_GREEN: u32 = 0x50743b;
const DARK_GREEN: u32 = 0x0e3508;
const LIGHT_BROWN: u32 = 0x9d7541;
const DARK_BROWN: u32 = 0x513c12;
/// How many shades lighter each of a block's four columns is.
const DITHER: [i32; 4] = [0, 8, 0, 6];
/// The jitter's seed and reach (`fJitterRandSeed`, `fJitterEnvelope`).
const JITTER_SEED: u32 = 0x40de9;
const JITTER: u32 = 3;
/// The sun's angle from the zenith, as the grass's constructor stores it
/// (0x49c187): about π/10.
const LATITUDE_BITS: u32 = 0x3ea0_d973;
/// Shades added along a wall.
const WALL_SHADE: i32 = 64;

/// Blades scattered over a tile shape, and the thresholds that set how
/// many a tile draws.
const BLADES: i32 = 4800;
const BLADE_SEED: u32 = 0x4aa4_7b44;
/// Grass values at and below which the fewest blades grow, and at and
/// above which the most do; brown grass's blades are a pixel shorter.
const SPARSE: i32 = -73;
const DENSE: i32 = 85;
const FEWEST: i32 = 500;
const MOST: i32 = 1200;
/// Tallest blade at each zoom (index 1 far to 3 near), in pixels.
const BLADE_HEIGHT: [i32; 4] = [0, 1, 1, 4];
/// Blade shades relative to the ground: 30 more, plus or minus up to 30.
const BLADE_SHADE: i32 = 30;
const BLADE_SPREAD: u32 = 31;

/// A 4 by 2 block of 16-bit (5-6-5) pixels: the first row, then the
/// second.
pub type Block = [u16; 8];

/// The dithered palette, a block per grass color and shade.
pub fn palette() -> &'static [[Block; SHADES]; COLORS] {
    static PALETTE: OnceLock<Box<[[Block; SHADES]; COLORS]>> = OnceLock::new();
    PALETTE.get_or_init(|| {
        let mut p = Box::new([[[0; 8]; SHADES]; COLORS]);
        for (i, row) in p.iter_mut().enumerate() {
            let t = i as f32 / (COLORS - 1) as f32;
            let light = blend(LIGHT_BROWN, LIGHT_GREEN, t);
            let dark = blend(DARK_BROWN, DARK_GREEN, t);
            let shade = |j: i32| rgb565(blend(light, dark, j as f32 / SHADES as f32));
            for (j, block) in row.iter_mut().enumerate() {
                let j = j as i32;
                *block = [shade(j); 8];
                for (k, &offset) in DITHER.iter().enumerate() {
                    if offset != 0 {
                        block[k] = shade(j - offset);
                        block[7 - k] = shade(j - offset);
                    }
                }
            }
        }
        p
    })
}

/// Blends two 0xRRGGBB colors, truncating each channel and keeping it
/// within 0 to 255 (0x49cc39).
fn blend(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let (a, b) = (((a >> shift) & 255) as i32, ((b >> shift) & 255) as i32);
        let v = (f64::from(b - a) * f64::from(t) + f64::from(a)).trunc();
        v.clamp(0.0, 255.0) as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

/// 0xRRGGBB to 5-6-5, dropping the low bits (0x49f0ae).
fn rgb565(c: u32) -> u16 {
    (((c >> 8) & 0xf800) | ((c >> 5) & 0x07e0) | ((c >> 3) & 0x1f)) as u16
}

/// A 5-6-5 pixel widened to 8 bits a channel.
pub fn widen(p: u16) -> [u8; 3] {
    let (r, g, b) = ((p >> 11) as u8, ((p >> 5) & 63) as u8, (p & 31) as u8);
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

/// The original's random generator (0x46f457): a 32-bit linear
/// congruential step returning bits 16 to 47 of the product.
struct Random(u32);

impl Random {
    fn next(&mut self) -> u32 {
        let product = u64::from(self.0) * 0x41c6_4e6d + 0x3039;
        self.0 = product as u32;
        (product >> 16) as u32
    }

    /// A small signed jitter: the smaller of two draws below `reach`,
    /// negated half the time (0x46f49b).
    fn jitter(&mut self, reach: u32) -> i32 {
        let a = self.next() % reach;
        let b = self.next() % reach;
        let m = a.min(b) as i32;
        if self.next() & 1 != 0 { -m } else { m }
    }
}

/// Light on a slope, for height differences -5 to 5 along each axis
/// (0x4c47f7): the squared cosine between the sun and the slope's normal.
fn light_table() -> [[f32; 11]; 11] {
    // The sun's angle from the zenith: π/10, set by the grass's constructor
    // (0x49c187, the float 0x3ea0d973). The base class has a setter that
    // would tilt it by the camera's rotation (0x4a4551: 1, 2, 3 and 2
    // tenths of π), but only a command handler reaches it (message 0xdd at
    // 0x4c41bf), and turning the camera does not: at rotation 3 the
    // original's grass matches π/10 and not 2π/10. The constant is 9 ulps
    // below π/10 rounded to f32, and a slope can tell the difference.
    light_table_at(f32::from_bits(LATITUDE_BITS))
}

/// The light table for a sun `latitude` radians from the zenith.
fn light_table_at(latitude: f32) -> [[f32; 11]; 11] {
    // libm stands in for the x87's sine and cosine; both are within an
    // ulp of a double, far finer than the f32 they are rounded to.
    let (sin, cos) = (
        libm::sin(f64::from(latitude)),
        libm::cos(f64::from(latitude)),
    );
    // Longitude 0: the sun stands toward +x of the view.
    let v = [sin as f32, -0.0f32, cos as f32];
    let length = v
        .iter()
        .map(|&c| f64::from(c) * f64::from(c))
        .sum::<f64>()
        .sqrt();
    let inverse = 1.0 / length;
    let sun = v.map(|c| (f64::from(c) * inverse) as f32);
    // A tile's width in height units: twice the square root of 6
    // (0x434df6), so a height step of a units is a slope of a / run.
    let run = (2.0 * 6.0f64.sqrt()) as f32;
    let mut table = [[0.0; 11]; 11];
    for (i, row) in table.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            let (a, b) = (i as i32 - 5, j as i32 - 5);
            let dot = (f64::from(a) * f64::from(sun[0])
                + f64::from(b) * f64::from(sun[1])
                + f64::from(run) * f64::from(sun[2])) as f32;
            let length = (f64::from(a * a + b * b) + f64::from(run) * f64::from(run)).sqrt();
            let c = f64::from(dot) / length;
            *cell = (c * c) as f32;
        }
    }
    table
}

fn lookup(table: &[[f32; 11]; 11], a: i32, b: i32) -> f32 {
    table[(a + 5).clamp(0, 10) as usize][(b + 5).clamp(0, 10) as usize]
}

/// The truncated product of a nonnegative f64 and 127, exactly as the x87
/// computes it in 64-bit precision.
fn times_127(x: f64) -> i32 {
    if !(x.is_finite() && x > 0.0) {
        return 0;
    }
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let mantissa = (bits & ((1 << 52) - 1)) | if exponent == 0 { 0 } else { 1 << 52 };
    let shift = exponent.max(1) - 1075;
    let product = u128::from(mantissa) * 127;
    let whole = if shift >= 0 {
        product.checked_shl(shift as u32).unwrap_or(u128::MAX)
    } else if shift > -128 {
        product >> (-shift)
    } else {
        0
    };
    whole.min(i32::MAX as u128) as i32
}

/// The grass of a lot as one camera rotation sees it: per view tile, the
/// shade and the grass color at its corners, in the original's corner
/// order (0, 1), (0, 0), (1, 0), (1, 1).
pub struct Grass {
    pub shades: Vec<[u8; 4]>,
    pub colors: Vec<[u8; 4]>,
    /// How many blades each view tile draws, and the rank they start at.
    pub blades: Vec<(i32, i32)>,
    /// Tiles brown enough that their blades are a pixel shorter.
    short: Vec<bool>,
    /// The blades of each column of a tile at this zoom, by rank.
    scatter: Vec<Vec<Blade>>,
}

/// One blade of the scatter: its rank, height, column offset (0 to 3),
/// depth down the column (0 to 1) and shade offset.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Blade {
    rank: i32,
    height: i32,
    dx: i32,
    down: f32,
    shade: i32,
}

/// The blades scattered over one tile shape at a zoom (0x49dfbd).
fn scatter(zoom: u32) -> Vec<Vec<Blade>> {
    let zoom = zoom.clamp(1, 3);
    let columns = (8i32 << zoom) / 4;
    // Pixels in half a tile: columns of 8, 24, 40, ... pixels.
    let half = (columns - 1) * columns * 16 / 2 + columns * 8;
    let mut lists = vec![Vec::new(); (2 * columns) as usize];
    let mut random = Random(BLADE_SEED);
    let tallest = BLADE_HEIGHT[zoom as usize];
    for rank in 0..(MOST * 4).max(FEWEST) {
        let r = random.next() % (2 * half) as u32;
        let left = (r as i32) < half;
        let mut rest = r as i32 % half;
        let mut column = 0;
        if rest > 0 {
            let mut size = 8;
            while rest >= size {
                rest -= size;
                column += 1;
                size += 16;
                if rest <= 0 {
                    break;
                }
            }
        }
        if !left {
            column = 2 * columns - column - 1;
        }
        let dx = (random.next() & 3) as i32;
        let down = (f64::from(random.next() % 4001) * 0.000_249_937_515_621_094_75) as f32;
        let mut shade = (random.next() % BLADE_SPREAD) as i32;
        if random.next() & 1 != 0 {
            shade = -shade;
        }
        let mut height = tallest;
        if height > 1 {
            height -= (random.next() % (height / 2) as u32) as i32;
        }
        if let Some(list) = lists.get_mut(column as usize) {
            list.push(Blade {
                rank,
                height,
                dx,
                down,
                shade: BLADE_SHADE + shade,
            });
        }
    }
    lists
}

/// The last blade ranked below `threshold`, searching from `start` (the
/// original's galloping search at 0x49d79d).
fn select(blades: &[Blade], threshold: i32, start: usize) -> usize {
    let count = blades.len();
    let mut step = count / 2;
    let mut at = start;
    while step != 0 {
        let i = step + at;
        if i < count && blades[i].rank < threshold {
            at = i;
        } else {
            step >>= 1;
        }
    }
    at
}

const N: usize = ARRAY_SIZE as usize;

impl Grass {
    pub fn new(lot: &Lot, camera: &Camera) -> Self {
        let rotation = camera.rotation & 3;
        let inside = |v: i32| v.clamp(0, ARRAY_SIZE - 1);
        // A view tile's terrain heights, corners in view order
        // (0x479567).
        let heights = |x: i32, y: i32| -> [i32; 4] {
            let [lx, ly] = camera.unrotate_tile([inside(x), inside(y)]);
            let h = lot.heights[Lot::cell(lx as usize, ly as usize)];
            std::array::from_fn(|n| i32::from(h[(n + 4 - usize::from(rotation)) & 3]))
        };
        let table = light_table();
        let sw = |c: [i32; 4]| lookup(&table, c[0] - c[3], c[1] - c[0]);
        let ne = |c: [i32; 4]| lookup(&table, c[1] - c[2], c[3] - c[2]);
        // The vertices run 1 to the lot's size less one, x outer, each
        // taking its jitter from one stream (0x4a4a43). They count from
        // the lot's least corner in the view, as the blade map does below;
        // on a turned lot smaller than the arrays that is not 0 (which
        // corner the original counts from there is unverified).
        let side = lot.size.clamp(1, N) as i32;
        let last = side - 1;
        let ends = [camera.rotate_tile([0, 0]), camera.rotate_tile([last, last])];
        let least = [ends[0][0].min(ends[1][0]), ends[0][1].min(ends[1][1])];
        let mut random = Random(JITTER_SEED);
        let mut vertex = vec![0u8; N * N];
        for x in least[0] + 1..least[0] + side {
            for y in least[1] + 1..least[1] + side {
                let p0 = heights(x, y);
                let p1 = heights(x - 1, y);
                let p2 = heights(x - 1, y - 1);
                let p3 = heights(x, y - 1);
                let mut sum = sw(p3);
                for t in [ne(p2), sw(p2), ne(p1), ne(p0), sw(p0)] {
                    sum += t;
                }
                let mean = f64::from(sum) * f64::from(1.0f32 / 6.0);
                let mut v = MAX_SHADE - times_127(mean);
                v += random.jitter(JITTER);
                vertex[x as usize * N + y as usize] = v.clamp(0, MAX_SHADE) as u8;
            }
        }
        let at = |x: i32, y: i32| vertex[inside(x) as usize * N + inside(y) as usize];
        let mut shades = vec![[0u8; 4]; N * N];
        for x in 0..ARRAY_SIZE {
            for y in 0..ARRAY_SIZE {
                let mut s = [at(x, y + 1), at(x, y), at(x + 1, y), at(x + 1, y + 1)];
                // A wall along the tile's far edge in the view darkens the
                // two corners on it (0x4a4b9e).
                let [lx, ly] = camera.unrotate_tile([x, y]);
                let bits = lot.levels[0].walls[Lot::cell(lx as usize, ly as usize)].segments;
                if rotate_bits(bits, rotation) & lot::NORTH != 0 {
                    for c in &mut s[1..3] {
                        *c = (i32::from(*c) + WALL_SHADE).min(MAX_SHADE) as u8;
                    }
                }
                shades[x as usize * N + y as usize] = s;
            }
        }
        // Grass color at a view corner: the four tiles around it
        // (0x49cd43).
        let grass = |x: i32, y: i32| -> i32 {
            let [lx, ly] = camera.unrotate_tile([inside(x), inside(y)]);
            i32::from(lot.grass[Lot::cell(lx as usize, ly as usize)])
        };
        let corner = |x: i32, y: i32| -> u8 {
            let sum = grass(x, y) + grass(x - 1, y) + grass(x - 1, y - 1) + grass(x, y - 1);
            let v = (sum / 4 + 128) * COLORS as i32 / 256;
            v.clamp(0, COLORS as i32 - 1) as u8
        };
        let mut colors = vec![[0u8; 4]; N * N];
        for x in 0..ARRAY_SIZE {
            for y in 0..ARRAY_SIZE {
                colors[x as usize * N + y as usize] = [
                    corner(x, y + 1),
                    corner(x, y),
                    corner(x + 1, y),
                    corner(x + 1, y + 1),
                ];
            }
        }
        // Each tile's random offset into the blades: a square map the
        // lot's size on a side (0x49c9c7). A tile reads it by its place
        // from the lot's least view corner, the coordinates this pass runs
        // in (the pool test at 0x47c16d turns them back to the lot's).
        let side = side as usize;
        let mut random = Random(BLADE_SEED);
        let variation: Vec<u32> = (0..side * side).map(|_| random.next() % 0xffff).collect();
        let zoom = camera.zoom.level();
        let mut blades = vec![(0, 0); N * N];
        let mut short = vec![false; N * N];
        for x in 0..ARRAY_SIZE {
            for y in 0..ARRAY_SIZE {
                let i = x as usize * N + y as usize;
                let [lx, ly] = camera.unrotate_tile([x, y]);
                let cell = Lot::cell(lx as usize, ly as usize);
                // A floor grows none (0x49d109). The original also gives a
                // pool tile none, and a floor tile a diagonal wall splits
                // keeps the count of the tile drawn before it; here a split
                // tile counts by its own grass and pools are not looked at.
                let level = &lot.levels[0];
                if level.floors[cell] != 0 && level.walls[cell].segments & 0x30 == 0 {
                    continue;
                }
                let value = grass(x, y);
                let count = if value >= DENSE {
                    MOST
                } else if value <= SPARSE {
                    short[i] = true;
                    FEWEST
                } else {
                    (MOST - FEWEST) * (value - SPARSE) / (DENSE - SPARSE) + FEWEST
                };
                let count = if zoom == 1 { count / 4 } else { count };
                if count == 0 {
                    continue;
                }
                let (vx, vy) = (x - least[0], y - least[1]);
                if !(0..side as i32).contains(&vx) || !(0..side as i32).contains(&vy) {
                    continue;
                }
                let start = variation[vx as usize * side + vy as usize] % (BLADES - count) as u32;
                blades[i] = (count, start as i32);
            }
        }
        Self {
            shades,
            colors,
            blades,
            short,
            scatter: scatter(zoom),
        }
    }

    fn index(view: [i32; 2]) -> Option<usize> {
        let inside = |v: i32| (0..ARRAY_SIZE).contains(&v);
        (inside(view[0]) && inside(view[1])).then(|| view[0] as usize * N + view[1] as usize)
    }

    /// The shades and grass colors of a view tile.
    pub fn tile(&self, view: [i32; 2]) -> Option<([u8; 4], [u8; 4])> {
        Self::index(view).map(|i| (self.shades[i], self.colors[i]))
    }

    /// The blades a view tile draws in one of its columns (`index` 0 at
    /// the left): each blade's screen x, its foot row, its height and its
    /// 5-6-5 color.
    pub fn blades(
        &self,
        view: [i32; 2],
        column: &Column,
        index: usize,
    ) -> Vec<(i32, i32, i32, u16)> {
        let Some(i) = Self::index(view) else {
            return Vec::new();
        };
        let (count, start) = self.blades[i];
        let Some(list) = self.scatter.get(index) else {
            return Vec::new();
        };
        if column.height < 1 || count == 0 || list.is_empty() {
            return Vec::new();
        }
        let lo = select(list, start, 0);
        let hi = select(list, start + count, lo);
        let palette = palette();
        list[lo..hi]
            .iter()
            .map(|b| {
                let dy = (f64::from(column.height) * f64::from(b.down)).trunc() as i32;
                let mut height = b.height;
                if self.short[i] {
                    height = (height - 1).max(1);
                }
                let t = (step(column.shade, dy) >> 8) + i64::from(b.shade);
                let s = step(column.color, dy) >> 8;
                let t = t.clamp(0, i64::from(MAX_SHADE)) as usize;
                let s = s.clamp(0, COLORS as i64 - 1) as usize;
                let (x, foot) = (column.x.saturating_add(b.dx), column.y.saturating_add(dy));
                (x, foot, height, palette[s][t][0])
            })
            .collect()
    }
}

/// Wall bits as the view sees them (the original's table at 0x6f4670):
/// the four edges turn with the camera; the diagonals swap on odd turns.
fn rotate_bits(bits: u8, rotation: u8) -> u8 {
    let r = u32::from(rotation & 3);
    let edges = bits & 15;
    let edges = ((u32::from(edges) << r) | (u32::from(edges) >> (4 - r))) as u8 & 15;
    let diagonals = bits & 0x30;
    let diagonals = if r & 1 == 0 || diagonals == 0 || diagonals == 0x30 {
        diagonals
    } else {
        diagonals ^ 0x30
    };
    edges | diagonals
}

/// One four-pixel column of a tile: its screen position, its row count,
/// and the shade and grass color at its top with their steps per row, in
/// 256ths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column {
    pub x: i32,
    pub y: i32,
    pub height: i32,
    pub shade: [i32; 2],
    pub color: [i32; 2],
}

/// The columns of a tile (0x49dacd, without clipping). `left` is the
/// screen x of its left corner and `bottom` the row its columns start
/// from at height 0; `heights` are its corners' terrain heights, `shades`
/// and `colors` its corner values, all in the view corner order.
pub fn columns(
    left: i32,
    bottom: i32,
    heights: [u8; 4],
    shades: [u8; 4],
    colors: [u8; 4],
    zoom: u32,
) -> Vec<Column> {
    // Nothing this far off any screen is drawn; skipping it keeps the
    // arithmetic below in range.
    const FAR: i32 = 1 << 24;
    if !(-FAR..FAR).contains(&left) || !(-FAR..FAR).contains(&bottom) {
        return Vec::new();
    }
    let zoom = zoom.clamp(1, 3);
    let h = heights.map(i32::from);
    let t = shades.map(i32::from);
    let a = colors.map(i32::from);
    let width = 16i32 << zoom;
    let half = width / 2;
    let shift = zoom + 1;
    let step = |x: i32, z: i32| ((x - z) * 256) >> shift;
    let gradient = |top: i32, bottom: i32, n: i32| [top, (bottom - top) / n];
    let mut out = Vec::with_capacity((width / 4) as usize);
    let (mut shade, mut color) = ([0, 0], [0, 0]);
    // Left half, from the left corner toward the top and bottom ones.
    let slope = h[1] - h[3] + 4;
    let dy = h[1] - h[0] + 2;
    let mut height = h[0] - h[3] + 2;
    let mut y = bottom - (h[0] << shift);
    let (tu, tl, au, al) = (
        step(t[1], t[0]),
        step(t[3], t[0]),
        step(a[1], a[0]),
        step(a[3], a[0]),
    );
    let mut tv = [t[0] * 256, t[0] * 256];
    let mut av = [a[0] * 256, a[0] * 256];
    let mut x = 0;
    while x < half {
        if height != 0 {
            shade = gradient(tv[0], tv[1], height);
            color = gradient(av[0], av[1], height);
        }
        out.push(Column {
            x: left + x,
            y,
            height,
            shade,
            color,
        });
        x += 4;
        y -= dy;
        height += slope;
        tv = [tv[0] + tu, tv[1] + tl];
        av = [av[0] + au, av[1] + al];
    }
    // Right half, from the right corner back toward the middle.
    let dy = h[1] - h[2] + 2;
    let mut height = h[2] - h[3] + 2;
    let mut y = bottom - (h[2] << shift);
    let (tu, tl, au, al) = (
        step(t[1], t[2]),
        step(t[3], t[2]),
        step(a[1], a[2]),
        step(a[3], a[2]),
    );
    let mut tv = [t[2] * 256, t[2] * 256];
    let mut av = [a[2] * 256, a[2] * 256];
    let mut x = width - 4;
    while x >= half {
        tv = [tv[0] + tu, tv[1] + tl];
        av = [av[0] + au, av[1] + al];
        if height != 0 {
            shade = gradient(tv[0], tv[1], height);
            color = gradient(av[0], av[1], height);
        }
        out.push(Column {
            x: left + x,
            y,
            height,
            shade,
            color,
        });
        x -= 4;
        y -= dy;
        height += slope;
    }
    out
}

impl Column {
    /// The column's pixels, top to bottom: each row's screen y and its
    /// four 5-6-5 pixels.
    pub fn rows(&self) -> impl Iterator<Item = (i32, [u16; 4])> + '_ {
        let palette = palette();
        let end = i64::from(self.y) + i64::from(self.height);
        (0..self.height.max(0)).map(move |row| {
            let t = step(self.shade, row) >> 8;
            let s = step(self.color, row) >> 8;
            let t = t.clamp(0, i64::from(MAX_SHADE)) as usize;
            let s = s.clamp(0, COLORS as i64 - 1) as usize;
            let parity = ((end + i64::from(row)) & 1) as usize;
            let block = &palette[s][t];
            let mut px = [0; 4];
            px.copy_from_slice(&block[parity * 4..parity * 4 + 4]);
            (self.y.saturating_add(row), px)
        })
    }
}

/// A value and its step per row, `rows` rows on, without overflow.
fn step([value, per_row]: [i32; 2], rows: i32) -> i64 {
    i64::from(value) + i64::from(per_row) * i64::from(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_runs_from_light_to_dark_with_lighter_dither() {
        let p = palette();
        // Shade 0 of the greenest color: light green, the dither columns
        // extrapolated lighter.
        let b = p[31][0];
        assert_eq!(b[0], rgb565(LIGHT_GREEN));
        assert_eq!(b[2], b[0]);
        assert_eq!(b[1], b[6]);
        assert_eq!(b[3], b[4]);
        // The last shade sits one 128th short of dark.
        let last = p[0][127][0];
        assert_eq!(last, rgb565(blend(LIGHT_BROWN, DARK_BROWN, 127.0 / 128.0)));
    }

    #[test]
    fn the_sun_angle_is_the_constructors_exact_float() {
        // π/10 rounded to f32 is 0x3ea0d97c; the original stores 9 ulps
        // less. A vertex ringed by these slopes is shade 87 with the
        // original's float and would be 88 with the rounded one.
        let shade = |table: [[f32; 11]; 11]| {
            let slopes = [(-5, -1), (-3, 1), (-2, -2), (-2, -3), (-5, -3), (-5, 3)];
            let sum: f32 = slopes.iter().map(|&(a, b)| lookup(&table, a, b)).sum();
            MAX_SHADE - times_127(f64::from(sum) * f64::from(1.0f32 / 6.0))
        };
        let rounded = (f64::from(std::f32::consts::PI) * 0.1) as f32;
        assert_eq!(rounded.to_bits(), 0x3ea0_d97c);
        assert_eq!(shade(light_table()), 87);
        assert_eq!(shade(light_table_at(rounded)), 88);
    }

    #[test]
    fn random_matches_the_original_generator() {
        let mut r = Random(1);
        let a = r.next();
        assert_eq!(r.0, 0x41c6_4e6d + 0x3039);
        assert_eq!(a, ((0x41c6_4e6du64 + 0x3039) >> 16) as u32);
        // Bits above 32 of the product reach the result.
        let mut r = Random(0xffff_ffff);
        let p = 0xffff_ffffu64 * 0x41c6_4e6d + 0x3039;
        assert_eq!(r.next(), (p >> 16) as u32);
    }

    #[test]
    fn flat_ground_is_evenly_lit_and_a_flat_tile_has_diamond_columns() {
        let table = light_table();
        let flat = lookup(&table, 0, 0);
        // cos² of the sun's angle from the zenith, π/10.
        let cos = libm::cos(std::f64::consts::PI / 10.0);
        assert!((f64::from(flat) - cos * cos).abs() < 1e-6);
        assert_eq!(times_127(0.5), 63);
        assert_eq!(times_127(1.0), 127);
        let c = columns(0, 100, [0; 4], [10; 4], [31; 4], 1);
        assert_eq!(c.len(), 8);
        assert_eq!(
            c.iter().map(|c| (c.x, c.y, c.height)).collect::<Vec<_>>(),
            [
                (0, 100, 2),
                (4, 98, 6),
                (8, 96, 10),
                (12, 94, 14),
                (28, 100, 2),
                (24, 98, 6),
                (20, 96, 10),
                (16, 94, 14)
            ]
        );
        assert!(c.iter().all(|c| c.shade == [10 * 256, 0]));
    }

    fn lot(size: usize, grass: i8) -> Lot {
        let full = lot::FULL * lot::FULL;
        let level = || lot::Level {
            floors: vec![0; full],
            walls: vec![lot::WallCell::default(); full],
            rooms: None,
        };
        Lot {
            size,
            heights: vec![[0; 4]; full],
            grass: vec![grass; full],
            levels: [level(), level()],
            outdoor_rooms: Default::default(),
            room_lights: Default::default(),
            halves: Default::default(),
        }
    }

    fn camera(zoom: crate::Zoom) -> Camera {
        Camera {
            zoom,
            rotation: 0,
            level: 1,
            origin: [0, 0],
            walls: crate::Walls::Up,
            focus: crate::Focus::default(),
        }
    }

    #[test]
    fn blades_are_scattered_over_a_tile_in_rank_order() {
        for zoom in 1..=3 {
            let lists = scatter(zoom);
            assert_eq!(lists.len(), (16 << zoom) / 4);
            assert_eq!(lists.iter().map(Vec::len).sum::<usize>(), 4800);
            for list in &lists {
                assert!(list.windows(2).all(|w| w[0].rank < w[1].rank));
                assert!(
                    list.iter()
                        .all(|b| (0..4).contains(&b.dx) && (0.0..=1.0).contains(&b.down))
                );
                assert!(list.iter().all(|b| (0..=60).contains(&b.shade)));
            }
            let tallest = lists.iter().flatten().map(|b| b.height).max();
            assert_eq!(tallest, Some(if zoom == 3 { 4 } else { 1 }));
        }
        // The window search lands on the last rank below the threshold.
        let list = &scatter(1)[0];
        let at = select(list, 1000, 0);
        assert!(list[at].rank < 1000 && list[at + 1].rank >= 1000);
    }

    #[test]
    fn a_tile_grows_blades_by_its_grass_and_the_lot_size_moves_them() {
        // Green grass grows the most blades, a quarter of them at far zoom.
        let far = Grass::new(&lot(64, 100), &camera(crate::Zoom::Far));
        let near = Grass::new(&lot(64, 100), &camera(crate::Zoom::Near));
        assert_eq!(far.blades[10 * N + 10].0, 300);
        assert_eq!(near.blades[10 * N + 10].0, 1200);
        // Brown grass the fewest, and midway between them midway.
        let brown = Grass::new(&lot(64, -100), &camera(crate::Zoom::Near));
        assert_eq!(brown.blades[10 * N + 10].0, 500);
        assert!(brown.short[10 * N + 10]);
        let middle = Grass::new(&lot(64, 6), &camera(crate::Zoom::Near));
        assert_eq!(middle.blades[10 * N + 10].0, 850);
        // The random map is the lot's size on a side: on a smaller lot a
        // tile past the first row reads another entry, and tiles off the
        // lot grow none.
        let small = Grass::new(&lot(52, 100), &camera(crate::Zoom::Near));
        assert_eq!(small.blades[0], near.blades[0]);
        assert_eq!(small.blades[51], near.blades[51]);
        assert_ne!(small.blades[N + 1], near.blades[N + 1]);
        assert_eq!(small.blades[N + 1].1, near.blades[52 + 1].1);
        assert_eq!(small.blades[52 * N].0, 0);
        // A floor grows none.
        let mut floored = lot(64, 100);
        floored.levels[0].floors[Lot::cell(10, 10)] = 1;
        let floored = Grass::new(&floored, &camera(crate::Zoom::Near));
        assert_eq!(floored.blades[10 * N + 10].0, 0);
    }

    #[test]
    fn a_turned_small_lot_shades_every_tile_it_draws() {
        let small = lot(52, 0);
        for rotation in 0..4 {
            let camera = Camera {
                rotation,
                ..camera(crate::Zoom::Far)
            };
            let grass = Grass::new(&small, &camera);
            for x in 1..51 {
                for y in 1..51 {
                    let view = camera.rotate_tile([x, y]);
                    let (shades, _) = grass.tile(view).unwrap_or_default();
                    // Flat ground: 127 less 127 cos² of the sun's angle
                    // from the zenith, within two shades of jitter. The sun
                    // does not move when the camera turns.
                    let flat = 13;
                    assert!(
                        shades.iter().all(|&s| (i32::from(s) - flat).abs() <= 2),
                        "{rotation} {view:?} {shades:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn columns_far_off_screen_or_at_the_edge_of_i32_do_not_panic() {
        assert!(columns(0, i32::MAX - 10, [0; 4], [10; 4], [31; 4], 1).is_empty());
        assert!(columns(i32::MIN, 0, [255; 4], [10; 4], [31; 4], 99).is_empty());
        let wild = Column {
            x: i32::MAX,
            y: i32::MAX - 1,
            height: 40,
            shade: [i32::MAX; 2],
            color: [i32::MIN; 2],
        };
        assert_eq!(wild.rows().count(), 40);
        assert!(wild.rows().all(|(y, _)| y == i32::MAX || y == i32::MAX - 1));
        let grass = Grass::new(&lot(64, 100), &camera(crate::Zoom::Near));
        for (index, column) in [wild].iter().cycle().take(16).enumerate() {
            let _ = grass.blades([10, 10], column, index);
        }
    }

    #[test]
    fn walls_turn_with_the_camera() {
        assert_eq!(rotate_bits(lot::WEST, 1), lot::NORTH);
        assert_eq!(rotate_bits(lot::SOUTH, 1), lot::WEST);
        assert_eq!(rotate_bits(lot::DESCENDING, 1), lot::ASCENDING);
        assert_eq!(rotate_bits(lot::DESCENDING, 2), lot::DESCENDING);
        assert_eq!(rotate_bits(lot::EAST, 3), lot::NORTH);
        assert_eq!(rotate_bits(0x30, 1), 0x30);
    }
}
