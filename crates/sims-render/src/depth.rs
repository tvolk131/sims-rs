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
//! An object or wall is placed first (0x4c9ceb): its anchor is a screen
//! row `w9` and a depth `w10` taken from `R` and `H` at its position. Then
//!
//! ```text
//! sprite pixel = w10 + T[byte]                             drawn if <= what is there
//! wall pixel   = w10 + L[w9 + (8 << zoom) - y] + C[column] drawn if <= what is there + 1
//! ground tile  = R[top corner row] + H[top corner height]
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
//! against the game's depth buffer (`docs/decisions.md`, 2026-11-14).

use crate::camera::Camera;

/// Depth of a pixel nothing was drawn on.
pub const EMPTY: i32 = 65535;

/// The camera scale the original builds its tables from, the same at
/// every zoom (f32 bits 0x43AD9D3F).
const SCALE: f32 = f32::from_bits(0x43AD_9D3F);

/// What each level above the first adds to an anchor's depth, at every
/// zoom (depth owner words 33 to 35).
const LEVEL_DEPTH: i32 = -967;

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
        Self {
            zoom,
            origin_y: camera.origin[1],
            floor_height: camera.zoom.floor_height(),
            lookup,
            columns,
            curve,
            bytes,
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

    /// How much the ground table changes from screen row `first` to `y`.
    fn follow(&self, first: i32, y: i32) -> i32 {
        self.row(y.saturating_sub(self.origin_y))
            .saturating_sub(self.row(first.saturating_sub(self.origin_y)))
    }

    /// The depth of row `y` of a sprite drawn at one depth a row from the
    /// top row `top` (0x49f2a3), standing at view corner `corner` at
    /// terrain height `height` on `level`: the trim along wall tops (the
    /// original's side pieces, 0x53e1f7, read by the old prototype in
    /// `legacy/.../paint/row/geometry_side.rs`). The rows follow the
    /// ground table from the sprite's top.
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

    /// The upright part of a wall pixel on screen row `y`, for a wall
    /// placed at `anchor`.
    fn stand(&self, anchor: Anchor, y: i32) -> i32 {
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
    /// column table is all zero).
    pub fn diagonal(&self, anchor: Anchor, y: i32) -> i32 {
        self.stand(anchor, y)
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
