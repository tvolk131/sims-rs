//! Depth: the original's own depth space, value for value.
//!
//! The original keeps a 16-bit depth per screen pixel, cleared to 65535;
//! smaller is nearer. It builds five tables per zoom from one camera scale
//! (0x4ca071 to 0x4ca5af), and every surface takes its depth from them:
//!
//! - `R`, the ground rows: the depth of level-1 ground at height 0, by
//!   screen row below the camera origin;
//! - `H`, the height curve: what a terrain height adds;
//! - `L`, the upright rows: what a pixel adds for each row it stands above
//!   its anchor;
//! - `C`, the wall columns: what a straight wall's pixel adds for each
//!   column along it;
//! - `T`, the sprite bytes: what a sprite pixel's depth byte adds.
//!
//! With `C` it keeps its peak, the depth of one tile's diagonal, which
//! the cut walls' trim adds in steps (see [`Depth::trim`]).
//!
//! An object or wall is placed first (0x4c9ceb): its anchor is a screen
//! row `w9` and a depth `w10` taken from `R` and `H` at its position. Then
//!
//! ```text
//! sprite pixel = w10 + T[byte]                             drawn if <= what is there
//! wall pixel   = w10 + L[w9 + (8 << zoom) - y] + C[column] drawn if <= what is there + 1
//! ground tile  = R[top corner row] + H[top corner height]
//! cut trim     = w10 + L[w9 + (8 << zoom) - y] + layer(byte, column)
//!                                                          drawn if <= what is there + 1
//! ```
//!
//! The sprite blitter does look `L` up (0x49a5d3), but at the sprite
//! context's own row, so it adds `L[8 << zoom]`, which is zero.
//!
//! The tables were dumped from the running game (800x600, all three
//! zooms) and the generators here reproduce every entry; the height curve
//! needs one fix-up, at 116, where the original's matrix maths comes out
//! one lower than the formula. Negative heights are not in the dump. The
//! formulas for the anchors, walls, ground and sprites were checked
//! against the game's depth buffer (`docs/decisions/`, 2026-11-14).

use crate::camera::Camera;

/// Depth of a pixel nothing was drawn on.
pub const EMPTY: i32 = 65535;

/// The camera scale the original builds its tables from, the same at
/// every zoom (f32 bits 0x43AD9D3F).
const SCALE: f32 = f32::from_bits(0x43AD_9D3F);

/// What each level above the first adds to an anchor's depth, at every
/// zoom (depth owner words 33 to 35).
const LEVEL_DEPTH: i32 = -967;

/// What a diagonal wall seen edge-on adds to its depth, at every zoom
/// (0x4c7be2, for wall kind 0x20 only).
const EDGE_ON_DEPTH: i32 = -300;

/// The largest terrain height the curve covers, either way.
const CURVE: i32 = 255;

/// A placed object's or wall's anchor: the original's context words 9
/// (a screen row) and 10 (a depth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub row: i32,
    pub depth: i32,
}

/// The original's depth tables for one camera.
#[derive(Debug, Clone)]
pub struct Depth {
    zoom: u32,
    origin_y: i32,
    floor_height: i32,
    lookup: Vec<i32>,
    columns: Vec<i32>,
    curve: Vec<i32>,
    bytes: Vec<i32>,
    peak: i32,
    row_step: f64,
}

/// One depth unit per world distance: `(6 / sqrt 6) / scale * 65535`.
fn unit() -> f64 {
    6.0 / 6f64.sqrt() / f64::from(SCALE) * 65535.0
}

/// The original's float to integer conversion: toward zero.
fn ftol(v: f64) -> i32 {
    v.trunc() as i32
}

impl Depth {
    pub fn new(camera: &Camera) -> Self {
        let zoom = camera.zoom.level();
        let x = unit();
        let e = 8i32 << zoom;
        // L: falls by one unit per 16 << zoom rows, from its middle at
        // 8 << zoom (0x4ca1e0). Entry 0 is 65 higher.
        let n = (404 >> (3 - zoom)) * 2;
        let mut lookup: Vec<i32> = (0..n)
            .map(|i| ftol(-f64::from(i - e) * x / f64::from(16 << zoom) + 0.5))
            .collect();
        lookup[0] += 65;
        // C: two tiles of a rising then falling ramp, one unit per
        // 8 << zoom columns (0x4ca071), with the original's nudges.
        let half: Vec<i32> = (0..=e)
            .map(|k| ftol(f64::from(k) / f64::from(e) * x))
            .collect();
        let mut columns = vec![0; (4 * e + 2) as usize];
        for j in 0..=e {
            columns[(e - j) as usize] = half[(e - j) as usize];
            columns[(e + j) as usize] = half[(e - j) as usize];
        }
        let e = e as usize;
        columns.copy_within(0..2 * e, 2 * e);
        columns[4 * e] = columns[4 * e - 1];
        columns[4 * e + 1] = columns[4 * e - 1];
        let nudges: &[usize] = match zoom {
            2 => &[2 * e - 1, 4 * e - 1],
            3 => &[2 * e - 2, 2 * e - 1, 4 * e - 2, 4 * e - 1],
            _ => &[],
        };
        for &i in nudges {
            columns[i] += 1;
        }
        // H: one unit for every eight terrain steps (0x4ca427).
        // The original computes it through its view matrix, which comes out
        // one lower at 116; the dumped value is taken there.
        let curve = (-CURVE..=CURVE)
            .map(|h| ftol(-f64::from(h) * x / 8.0 + 0.5) - i32::from(h == 116))
            .collect();
        // T: the sprite depth bytes (0x4ca5af), in the original's mix of
        // single and double precision.
        let r = 1.0 / f64::from(SCALE);
        let a = f64::from((r * 12.0) as f32);
        let root2 = f64::from(2f64.sqrt() as f32);
        let root3 = f64::from(3f64.sqrt() as f32);
        let d = (3.0 * root2 * r * root3 + a) * 16383.75;
        let step = f64::from((d * f64::from((1.0f64 / 127.0) as f32)) as f32);
        let base = a * -16383.75 - d;
        let bytes = (0..255)
            .map(|i| ftol(step * f64::from(i) + base + 0.5))
            .collect();
        // R: the ground falls sqrt 3 / 2 of a tile diagonal step per row
        // (0x4ca2f8).
        let row_step =
            3f64.sqrt() * f64::from((2f64.sqrt() * 3.0 / f64::from(8 << zoom)) as f32) * 0.5;
        // C's peak, rounded rather than truncated: the depth owner's word
        // 18 - zoom, stored as the column table is built (0x4ca10a).
        let peak = ftol(x + 0.5);
        Self {
            zoom,
            origin_y: camera.origin[1],
            floor_height: camera.zoom.floor_height(),
            lookup,
            columns,
            curve,
            bytes,
            peak,
            row_step,
        }
    }

    /// R: the depth of level-1 ground at height 0, `i` screen rows below
    /// the camera origin. The original keeps this as a table from
    /// `-(8 << zoom) * 256 - 1` to 5999; the formula is the same anywhere.
    fn row(&self, i: i32) -> i32 {
        let top = f64::from(SCALE - 12.5);
        ftol((top - f64::from(i) * self.row_step) * 65535.0 / f64::from(SCALE))
    }

    /// H: what terrain height `h` adds, held to the curve's span.
    fn height(&self, h: i32) -> i32 {
        self.curve[(h.clamp(-CURVE, CURVE) + CURVE) as usize]
    }

    /// L at index `i`. The original reads past its ends without a check,
    /// into other memory; this holds `i` to the table instead.
    fn upright(&self, i: i32) -> i32 {
        let last = self.lookup.len() as i32 - 1;
        self.lookup[i.clamp(0, last) as usize]
    }

    /// The depth of a ground tile whose top corner, after rotation, is
    /// view corner `top` at terrain height `height` (0x4c9e38).
    pub fn ground(&self, top: [i32; 2], height: i32) -> i32 {
        let y = (top[0] + top[1]).saturating_mul(4 << self.zoom);
        self.row(y).saturating_add(self.height(height))
    }

    /// The depth of a grass tile whose top corner, after rotation, is view
    /// corner `top` at terrain height `height`: the ground's, held to the
    /// buffer's range, and 80 farther (the ground pass, 0x4c9e38 with its
    /// clamp, then `+ 0x50` in its caller; the old prototype's
    /// `legacy/crates/sims-vm/src/scheduler/clock/draw/ground.rs`).
    pub fn grass(&self, top: [i32; 2], height: i32) -> i32 {
        self.ground(top, height).clamp(0, EMPTY) + 80
    }

    /// The depth of row `y` of a floor tile on `level` whose top corner,
    /// after rotation, is view corner `top` at terrain height `height`,
    /// drawn from screen row `first` down: the ground's depth there, 65
    /// farther, then following the ground table row by row (the old
    /// prototype's `sprite_depth` in
    /// `legacy/.../paint/row/placement.rs`). **Unverified** above level 1.
    pub fn floor(&self, top: [i32; 2], height: i32, level: i32, first: i32, y: i32) -> i32 {
        self.ground(top, height)
            .saturating_add(LEVEL_DEPTH.saturating_mul(level - 1))
            .saturating_add(65)
            .saturating_add(self.follow(first, y))
    }

    /// The depth at or past which a floor shadow darkens row `y` of the
    /// tile [`Depth::floor`] describes: the floor's own depth without its
    /// 65 (0x4b10c4), so the shadow falls on the floor and not on what
    /// stands in front of it.
    pub fn shadow(&self, top: [i32; 2], height: i32, level: i32, first: i32, y: i32) -> i32 {
        self.floor(top, height, level, first, y).saturating_sub(65)
    }

    /// How much the ground table changes from screen row `first` to `y`.
    fn follow(&self, first: i32, y: i32) -> i32 {
        self.row(y.saturating_sub(self.origin_y))
            .saturating_sub(self.row(first.saturating_sub(self.origin_y)))
    }

    /// The depth of row `y` of a sprite drawn at one depth a row from the
    /// top row `top` (0x49f2a3), standing at view corner `corner` at
    /// terrain height `height` on `level`: the trim along standing wall
    /// tops (the original's side pieces, 0x53e1f7, read by the old
    /// prototype in `legacy/.../paint/row/geometry_side.rs`). The rows
    /// follow the ground table from the sprite's top. A cut wall's trim
    /// has a depth sprite instead ([`Depth::trim`]).
    pub fn flat(&self, corner: [i32; 2], height: i32, level: i32, top: i32, y: i32) -> i32 {
        let at = self.origin_y.saturating_neg();
        let correction = self
            .row(at)
            .saturating_sub(self.row(at.saturating_add((4 << self.zoom) + 1)));
        let base = self
            .ground(corner, height)
            .saturating_add((LEVEL_DEPTH + 65).saturating_mul(level))
            .saturating_add(correction)
            .min(EMPTY);
        self.follow(top, y).saturating_add(base)
    }

    /// Places a point (0x4c9ceb): `y` is its screen row, raised by its
    /// height and level, and `height` its height in terrain steps.
    pub fn anchor(&self, y: i32, height: f32, level: i32) -> Anchor {
        let z = self.zoom;
        let scale = (2i32 << z) as f32;
        let rise = ftol(f64::from(scale * height + 0.5));
        let mut row = y.saturating_add(rise);
        let ground = row.saturating_add(self.floor_height.saturating_mul(level - 1));
        let mut depth = self
            .row(ground.saturating_sub(self.origin_y))
            .saturating_add(LEVEL_DEPTH.saturating_mul(level - 1));
        // The height enters through the curve, between its whole steps,
        // and lifts the row back up (x87 at single precision).
        let root = 6f32.sqrt() * 2.0;
        let alt = 1.0 / root * height * 3.0;
        if alt.abs() > f32::from_bits(0x3586_37bd) {
            let scaled = alt * f32::from_bits(0x3eaa_aaab) * root;
            let k = scaled.floor() as i32;
            let (a, b) = (self.height(k), self.height(k.saturating_add(1)));
            let between = ftol(f64::from((b - a) as f32 * (scaled - k as f32) + 0.5));
            depth = depth.saturating_add(between + a);
            row = row.saturating_sub(ftol(f64::from(scale * scaled + 0.5)));
        }
        Anchor { row, depth }
    }

    /// The depth of a point between pixels, for what the original draws
    /// over the finished lot (a person's mesh): fine view position `fine`
    /// (sixteenths of a tile, after rotation) on `level`, raised
    /// `elevation` near-zoom pixels. It is [`Depth::anchor`]'s depth without
    /// its rounding: the ground rows `R` at the point's level-1 ground row,
    /// the level's offset, and the height curve `H` for `elevation / 16`
    /// terrain steps (a step is 16 near-zoom pixels), held to the curve's
    /// span as `anchor` holds it.
    ///
    /// The original keeps people out of the lot's depth buffer: a dump
    /// with Sims on screen shows them in the colour buffer only, over a
    /// depth buffer without them (and without the bench they sit on). How
    /// it gives their meshes a depth to test against that buffer is
    /// **unverified**; this places them in the same space as everything
    /// else.
    pub fn point(&self, [x, y]: [f32; 2], elevation: f32, level: i32) -> f32 {
        let rows = f64::from(x + y) * f64::from(4i32 << self.zoom) / 16.0;
        let top = f64::from(SCALE - 12.5);
        let ground = (top - rows * self.row_step) * 65535.0 / f64::from(SCALE);
        let steps = (f64::from(elevation) / 16.0).clamp(-f64::from(CURVE), f64::from(CURVE));
        let height = -steps * unit() / 8.0;
        (ground + f64::from(LEVEL_DEPTH) * f64::from(level - 1) + height) as f32
    }

    /// Where a pixel of depth `depth` at screen point `at` is, taken as on
    /// `level` (the house view's point to tile, 0x4a97c3, in its depth
    /// mode): the inverse of [`Depth::point`] and of the upright table
    /// `L`, which both make the depth an affine function of the point's
    /// level-1 ground row and of its height above its level's floor, with
    /// each level above the first adding its own depth offset. Gives the
    /// point's fine view position (sixteenths of a tile, unrounded) and
    /// its height above `level`'s floor at terrain height 0, in pixels of
    /// this zoom. The original unprojects through its inverse view matrix
    /// instead; that the two agree to the rounding is **unverified**.
    pub fn unproject(
        &self,
        camera: &Camera,
        at: [i32; 2],
        depth: i32,
        level: i32,
    ) -> ([f64; 2], f64) {
        let scale = 65535.0 / f64::from(SCALE);
        let a = f64::from(SCALE - 12.5) * scale;
        let b = self.row_step * scale;
        // A pixel up something upright is this much nearer (L, and H per
        // terrain step of 2 << zoom pixels).
        let k = unit() / f64::from(16i32 << self.zoom);
        let above = f64::from(level - 1);
        let row = f64::from(at[1] - self.origin_y) + f64::from(self.floor_height) * above;
        let depth = f64::from(depth) - f64::from(LEVEL_DEPTH) * above;
        let height = (a - b * row - depth) / (b + k);
        let ground = row + height;
        let sum = ground * 16.0 / f64::from(4i32 << self.zoom);
        let difference = f64::from(at[0] - camera.origin[0]) * 16.0 / f64::from(8i32 << self.zoom);
        ([(sum + difference) / 2.0, (sum - difference) / 2.0], height)
    }

    /// The depth of a pixel on screen row `y` of something standing
    /// upright at `anchor`, without its own depth bytes or columns: the
    /// anchor's depth plus `L` for the rows `y` stands above the anchor's
    /// row. Walls take their depth this way; anything else drawn upright
    /// from an anchor (a person's mesh) can too.
    pub fn stand(&self, anchor: Anchor, y: i32) -> i32 {
        let i = anchor.row.saturating_add(8 << self.zoom).saturating_sub(y);
        anchor.depth.saturating_add(self.upright(i))
    }

    /// The depth of a sprite pixel with depth byte `byte`, for a sprite
    /// placed at `anchor`. The original reads the upright table at the
    /// sprite context's own row (0x49a5d3 is passed the context's +0x24,
    /// the row it subtracts), so the upright part is always zero: the
    /// depth bytes alone carry the depth up the sprite. None for byte 255,
    /// which the original never draws (0x4ca62e).
    pub fn sprite(&self, anchor: Anchor, byte: u8) -> Option<i32> {
        let t = *self.bytes.get(usize::from(byte))?;
        Some(anchor.depth.saturating_add(t))
    }

    /// The depth of a straight wall's pixel on row `y`, `column` columns
    /// along the wall's column table from its start `first` (0x4c4907).
    pub fn wall(&self, anchor: Anchor, y: i32, first: i32, column: i32) -> i32 {
        let last = self.columns.len() as i32 - 1;
        let i = first.saturating_add(column).clamp(0, last);
        self.stand(anchor, y)
            .saturating_add(self.columns[i as usize])
    }

    /// The depth of a diagonal wall's pixel on row `y` (0x4c7bce: its
    /// column table is all zero). A diagonal seen edge-on (`edge_on`)
    /// stands [`EDGE_ON_DEPTH`] nearer: 0x4c7be2 passes it to the row
    /// loop (0x4c4907), which adds it to every row's depth.
    pub fn diagonal(&self, anchor: Anchor, y: i32, edge_on: bool) -> i32 {
        let depth = self.stand(anchor, y);
        if edge_on {
            depth.saturating_add(EDGE_ON_DEPTH)
        } else {
            depth
        }
    }

    /// The depth of a cut wall's trim pixel on row `y`, `column` columns
    /// from the trim sprite's left, whose depth sprite holds `byte` there,
    /// for a trim placed at `anchor` (0x49ecbc, called by 0x4c8b1f).
    /// None for byte 255, which is not drawn.
    ///
    /// The trim stands upright like a wall ([`Depth::stand`]), and its
    /// depth byte picks one of eight layers (table 0x668440): a ramp along
    /// the wall columns `C` from one of three starts, or a step of the
    /// columns' peak or half of it.
    pub fn trim(&self, anchor: Anchor, y: i32, column: i32, byte: u8) -> Option<i32> {
        // The original's starts are its column pointer for this zoom
        // (C plus 16 << zoom entries), less 8 << zoom and less one more.
        let e = 8i32 << self.zoom;
        let (start, step) = match byte {
            0..=62 | 89..=113 => (None, self.peak),
            63..=88 | 114..=139 => (None, self.peak / 2),
            140..=190 => (Some(2 * e), 0),
            191..=215 => (Some(e - 1), self.peak),
            216..=254 => (Some(e), self.peak),
            255 => return None,
        };
        let ramp = start.map_or(0, |start| {
            // The original reads past the table's ends without a check.
            let last = self.columns.len() as i32 - 1;
            self.columns[start.saturating_add(column).clamp(0, last) as usize]
        });
        Some(
            self.stand(anchor, y)
                .saturating_add(ramp)
                .saturating_add(step),
        )
    }

    /// Where a straight wall's columns start: the rising ramp for a wall
    /// running up to the right on screen, the falling one for a wall
    /// running down.
    pub fn wall_start(&self, rising: bool) -> i32 {
        if rising { 0 } else { 8 << self.zoom }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Walls, Zoom};

    fn camera(zoom: Zoom, origin: [i32; 2]) -> Camera {
        Camera {
            zoom,
            rotation: 0,
            level: 1,
            origin,
            walls: Walls::Up,
            focus: crate::Focus::default(),
        }
    }

    fn digest(values: impl IntoIterator<Item = i32>) -> u64 {
        // FNV-1a over the little-endian values.
        values.into_iter().fold(0xcbf2_9ce4_8422_2325, |h, v| {
            v.to_le_bytes()
                .iter()
                .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
        })
    }

    #[test]
    fn tables_match_the_ones_dumped_from_the_game() {
        // Entries and digests read from the running game's depth owner
        // (2026-11-14). R is checked over the dumped span from its origin.
        let d = Depth::new(&camera(Zoom::Far, [0, 0]));
        assert_eq!(d.bytes[0], -1478);
        assert_eq!(&d.lookup[..6], &[296, 217, 202, 188, 173, 159]);
        let m = Depth::new(&camera(Zoom::Medium, [0, 0]));
        assert_eq!(&m.lookup[..6], &[296, 224, 217, 209, 202, 195]);
        assert_eq!(m.columns[32], 462);
        assert_eq!(m.columns[63], 15);
        assert_eq!([m.row(0), m.row(1), m.row(2)], [63175, 63154, 63132]);
        assert_eq!(
            [m.row(-8193), m.row(-8192), m.row(5999)],
            [240724, 240702, -66827]
        );
        let n = Depth::new(&camera(Zoom::Near, [0, 0]));
        assert_eq!(&n.lookup[..6], &[296, 228, 224, 220, 217, 213]);
        assert_eq!(m.height(6), -346);
        assert_eq!(m.height(116), -6703);
        // C's peak, as the owner's word 18 - zoom holds it.
        assert_eq!([d.peak, m.peak, n.peak], [462; 3]);
    }

    #[test]
    fn a_cut_trims_depth_byte_picks_a_layer() {
        let m = Depth::new(&camera(Zoom::Medium, [335, -560]));
        let a = m.anchor(240, 6.0, 1);
        let stand = m.stand(a, 230);
        let at = |column, byte| m.trim(a, 230, column, byte).map(|d| d - stand);
        // Steps: the peak or half of it, the same in every column.
        for column in [0, 17, 63] {
            assert_eq!(at(column, 0), Some(462));
            assert_eq!(at(column, 70), Some(231));
            assert_eq!(at(column, 100), Some(462));
            assert_eq!(at(column, 120), Some(231));
        }
        // Ramps: rising to the peak half a tile in (32 columns at medium
        // zoom), or falling from it with the peak added; 191 to 215 starts
        // a column earlier than 216 and up.
        assert_eq!([at(0, 150), at(32, 180)], [Some(0), Some(462)]);
        assert_eq!([at(0, 230), at(32, 230)], [Some(462 + 462), Some(462)]);
        assert_eq!(at(1, 200), at(0, 230));
        assert_eq!(at(5, 255), None);
        // Columns past the table's end hold to its last entry.
        assert_eq!(at(i32::MAX, 150), at(1000, 150));
    }

    #[test]
    fn a_wall_anchor_matches_the_games_depth_buffer() {
        // A wall end at screen row 240 on terrain six steps up, medium
        // zoom, origin row -560: the game's w9 and w10 for that wall.
        let m = Depth::new(&camera(Zoom::Medium, [335, -560]));
        let a = m.anchor(240, 6.0, 1);
        assert_eq!(
            a,
            Anchor {
                row: 240,
                depth: 44452
            }
        );
        // Its pixel at column 31, row 150, as the game drew it.
        assert_eq!(m.wall(a, 150, m.wall_start(true), 31), 44250);
    }

    #[test]
    fn points_agree_with_anchors_at_whole_positions() {
        for zoom in Zoom::ALL {
            for origin in [[0, 0], [37, -411], [-120, 260]] {
                let c = camera(zoom, origin);
                let m = Depth::new(&c);
                for level in 1..=3 {
                    for (x, y) in [(0, 0), (64, 32), (300, 500), (512, 512), (1020, 4)] {
                        for steps in [0, 1, 3, 10, 255, 300] {
                            let elevation = steps * 16;
                            let row = c.project_fine([x, y], level, elevation)[1];
                            let a = m.anchor(row, steps as f32, level);
                            let p = m.point([x as f32, y as f32], elevation as f32, level);
                            assert!(
                                (p - a.depth as f32).abs() <= 2.0,
                                "{zoom:?} {origin:?} level {level} ({x}, {y}) {steps}: {p} vs {}",
                                a.depth
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn unprojecting_a_point_gives_back_its_position_and_height() {
        for zoom in Zoom::ALL {
            for origin in [[0, 0], [37, -411], [-120, 260]] {
                let c = camera(zoom, origin);
                let m = Depth::new(&c);
                let near = zoom.near_scale();
                for level in 1..=3 {
                    for (x, y) in [(0, 0), (64, 32), (300, 500), (512, 512), (1020, 4)] {
                        for steps in [0, 1, 3, 10] {
                            let elevation = steps * 16;
                            let at = c.project_fine([x, y], level, elevation);
                            let depth = m.point([x as f32, y as f32], elevation as f32, level);
                            let (fine, height) = m.unproject(&c, at, depth.round() as i32, level);
                            let case =
                                format!("{zoom:?} {origin:?} level {level} ({x}, {y}) {steps}");
                            assert!(
                                (fine[0] - f64::from(x)).abs() < 0.5
                                    && (fine[1] - f64::from(y)).abs() < 0.5,
                                "{case}: {fine:?}"
                            );
                            let rise = f64::from(elevation / near);
                            assert!((height - rise).abs() < 0.5, "{case}: {height} vs {rise}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn points_are_nearer_forward_and_up() {
        let m = Depth::new(&camera(Zoom::Medium, [0, 0]));
        let p = |x: f32, y: f32, e: f32, l: i32| m.point([x, y], e, l);
        assert!(p(100.5, 100.25, 0.0, 1) < p(100.0, 100.0, 0.0, 1));
        assert!(p(100.0, 100.0, 7.5, 1) < p(100.0, 100.0, 0.0, 1));
        assert!(p(100.0, 100.0, 0.0, 2) < p(100.0, 100.0, 0.0, 1));
        // Along a screen row the depth does not change.
        assert_eq!(p(110.0, 90.0, 0.0, 1), p(90.0, 110.0, 0.0, 1));
        // Between whole positions and heights it is the straight line
        // between them (nothing is rounded).
        for zoom in Zoom::ALL {
            let m = Depth::new(&camera(zoom, [13, -70]));
            let p = |x: f32, y: f32, e: f32| m.point([x, y], e, 2);
            let mid = p(100.5, 200.25, 4.5);
            let ends = (p(100.0, 200.0, 0.0) + p(101.0, 200.5, 9.0)) / 2.0;
            assert!((mid - ends).abs() < 0.01, "{zoom:?}: {mid} vs {ends}");
            assert!(p(100.25, 200.0, 0.0) < p(100.0, 200.0, 0.0));
            assert!(p(100.0, 200.0, 0.5) < p(100.0, 200.0, 0.0));
        }
        // Past the height curve's span the height stops counting, as for
        // an anchor.
        let top = 255.0 * 16.0;
        assert_eq!(p(100.0, 100.0, top, 1), p(100.0, 100.0, top * 2.0, 1));
        assert_eq!(p(100.0, 100.0, -top, 1), p(100.0, 100.0, -top * 2.0, 1));
    }

    #[test]
    fn nearer_and_higher_is_smaller() {
        let m = Depth::new(&camera(Zoom::Medium, [0, 0]));
        assert!(m.ground([10, 10], 0) < m.ground([9, 10], 0));
        assert!(m.ground([10, 10], 4) < m.ground([10, 10], 0));
        let a = m.anchor(300, 0.0, 1);
        assert!(m.sprite(a, 100) < m.sprite(a, 101));
        assert_eq!(m.sprite(a, 255), None);
        assert!(m.anchor(300, 0.0, 2).depth < a.depth);
    }

    #[test]
    fn every_entry_matches_the_dumped_tables() {
        // FNV-1a digests of the game's own tables, dumped 2026-11-14: L,
        // C, H from 0 to 255, T, and R over the whole table the game
        // keeps, from `-(8 << zoom) * 256 - 1` to 5999, at each zoom.
        let spans = [4097, 8193, 16385];
        let expected = [
            0x2922_e8cb_a6b9_3749,
            0xb411_5c6c_5794_35a6,
            0x632c_8758_6ce8_74fe,
        ];
        for (i, z) in Zoom::ALL.into_iter().enumerate() {
            let d = Depth::new(&camera(z, [0, 0]));
            let values = d
                .lookup
                .iter()
                .chain(&d.columns)
                .chain(&d.curve[CURVE as usize..])
                .chain(&d.bytes)
                .copied()
                .chain((-spans[i]..6000).map(|r| d.row(r)));
            assert_eq!(digest(values), expected[i], "{z:?}");
        }
    }
}
