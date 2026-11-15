//! The shadows objects cast on the floor under them, read from the
//! original's code.
//!
//! The original darkens each tile an object stands on, after drawing the
//! tile's floor or grass and before any object (0x4ae844, in the tile
//! draw 0x4ad8ac). How dark each pixel goes comes from a *mask* the size
//! of a floor tile, one byte a pixel: 255 leaves the pixel alone, lower
//! is darker. Which mask a tile takes is the object's *shadow code*, a
//! byte saying which of the tile's eight neighbours also lie in the
//! shadow (bits 0 to 3 the four sides, 4 to 7 the four corners, see
//! [`side`]).
//!
//! - **The code** (0x4573e7, kept by the object at +0x184). A one-tile
//!   object's code is its definition's shadow word (OBJD word 57, byte
//!   0x72), and -1 means no shadow. A tile of a multi-tile object with a
//!   shadow word other than -1 builds its code from the other tiles of
//!   its group (same OBJD word 10, the group): bit 0 is set if the tile
//!   at (x, y - 1) in the group's own grid (OBJD word 11, x high byte, y
//!   low) has a shadow, 1 (x - 1, y), 2 (x, y + 1), 3 (x + 1, y), 4
//!   (x - 1, y - 1), 5 (x - 1, y + 1), 6 (x + 1, y + 1) and 7 (x + 1,
//!   y - 1), counting only tiles whose OBJD word 56 is 0 (0x43ac5c). A
//!   code the masks cannot draw ([`valid`]) becomes 0.
//! - **Turning it.** The code is turned by the object's facing (data word
//!   1, halved) and then by the view's rotation, each step moving every
//!   side and corner bit to the one before it ([`rotate`], the tables
//!   0x53d2fc fills). Code 16 never turns.
//! - **The masks.** Codes 0 and 16 are drawn by hand: SPR2 `0x300` and
//!   `0x310` in `floors.iff` plus `0x20` a zoom, whose palette indices are
//!   the mask, the rows' outer transparent runs left at 255 (0x4c90c1,
//!   0x4c92af). Every other code's mask is worked out (0x4a3cd3), over
//!   floor 0's tile (SPR2 0, `0x100`, `0x200`): see [`generate`].
//! - **Drawing** (0x4b10c4). Each mask byte plus the object's *strength*
//!   gives `s`; the pixel is left alone if `s` is 248 or more, and
//!   otherwise drawn in light `s` (kept to 0 and up) by the lighting's
//!   tables. The strength is OBJD word 79 (byte 0x9e) plus 255 less the
//!   tile's drawn light byte (255 in full light), so a shadow in a dim
//!   room is fainter. Only pixels at or behind the tile's ground depth
//!   (the floor's, without its 65) are touched.
//! - **Which object.** The first object of the tile's object grid cell
//!   (0x49296a), or the next on the tile after it (slot 0, 0x432823),
//!   that is not hidden (data word 34), has no zero-extent flag (data
//!   word 8, value 4), is not of OBJD type 2 (a person) or 9 and has a
//!   shadow (0x434e23).
//!
//! Not ported: the option that turns shadows off (the "shadows" cheat,
//! global 0x6f0188) and the drawing context's flags that allow them
//! (0x4b444f, 0x4b4456); they are taken as on. The shadow drawn under an
//! object the player is placing (0x438a48) is not drawn.

/// The hand-drawn codes: none of the eight neighbours in the shadow, and
/// the second drawing (SPR2 `0x310`), which no other tile's code
/// reaches.
pub const DRAWN: [u8; 2] = [0, 16];

/// A mask byte at or past which no pixel changes (0x4b11f5).
const UNSHADED: i32 = 0xf8;

/// The curve a side of the mask follows, 0 to 7: bit 0 is the side's own
/// neighbour, bits 1 and 2 the corners at its two ends (0x4a3914).
pub fn side(code: u8, i: usize) -> u8 {
    let bit = |n: u8| (code >> n) & 1;
    let (edge, end, start) = match i & 3 {
        0 => (bit(0), bit(7), bit(4)),
        1 => (bit(1), bit(5), bit(4)),
        2 => (bit(2), bit(6), bit(5)),
        _ => (bit(3), bit(6), bit(7)),
    };
    edge + 2 * end + 4 * start
}

/// The curve of selector `k` at `t`, 0 to 1 along a side (0x4a3a07,
/// 0x4a3a10, 0x4a3a50, 0x4a3a96, 0x5fe4a5): none for a corner without
/// its side, which the original leaves without a curve.
fn curve(k: u8, t: f32) -> Option<f64> {
    let pi = f64::from(std::f32::consts::PI);
    let lift = |x: f64| {
        let s = libm::sin(x);
        libm::pow(if s >= 0.0 { s } else { 0.0 }, 0.6)
    };
    let half = |t: f32| lift(pi * f64::from(t) * 0.5);
    match k {
        0 => Some(0.0),
        1 => Some(lift(pi * f64::from(t))),
        3 => Some(half(t)),
        5 => Some(half(1.0 - t)),
        7 => Some(1.0),
        _ => None,
    }
}

/// The four sides' curves of a code, if each has one.
fn curves(code: u8) -> Option<[u8; 4]> {
    let sides = [0, 1, 2, 3].map(|i| side(code, i));
    sides
        .iter()
        .all(|&k| curve(k, 0.0).is_some())
        .then_some(sides)
}

/// Whether a code's mask can be worked out: every side has a curve and
/// neighbouring sides meet at the corners (0x4a3993, 0x4a3f45 with
/// 0x4a3fb8).
pub fn valid(code: u8) -> bool {
    let Some(k) = curves(code) else {
        return false;
    };
    let f = |i: usize, t: f32| curve(k[i], t).unwrap_or(0.0);
    let meet = |a: f64, b: f64| (b - a).abs() <= f64::from(1e-6f32);
    meet(f(1, 0.0), f(0, 0.0))
        && meet(f(2, 0.0), f(1, 1.0))
        && meet(f(3, 1.0), f(2, 1.0))
        && meet(f(0, 1.0), f(3, 0.0))
}

/// A code turned `turns` quarter turns: each side and corner bit moves
/// to the one `turns` before it (0x53d2fc). Code 16 stays as it is
/// (0x4a39ee).
pub fn rotate(code: u8, turns: u8) -> u8 {
    let turns = turns & 3;
    if turns == 0 || code == 16 {
        return code;
    }
    let spin = |nibble: u8| ((nibble >> turns) | (nibble << (4 - turns))) & 0xf;
    spin(code & 0xf) | (spin(code >> 4) << 4)
}

/// The shadow code of one tile of a multi-tile object at `(x, y)` in
/// its group's grid, given which grid places hold a tile with a shadow
/// (0x4573fb).
pub fn group_code(x: i32, y: i32, shaded: impl Fn(i32, i32) -> bool) -> u8 {
    let around = [
        (0, -1),
        (-1, 0),
        (0, 1),
        (1, 0),
        (-1, -1),
        (-1, 1),
        (1, 1),
        (1, -1),
    ];
    let code = around
        .iter()
        .enumerate()
        .filter(|&(_, &(dx, dy))| shaded(x + dx, y + dy))
        .fold(0u8, |code, (bit, _)| code | (1 << bit));
    if valid(code) { code } else { 0 }
}

/// A floor tile's shadow mask: `width` by `height` bytes, 255 where the
/// tile leaves the pixel alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub values: Vec<u8>,
}

impl Mask {
    /// The mask byte at a pixel of the tile.
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.values[y * self.width + x]
    }

    /// How a pixel of the tile is drawn with `strength`: None if it is
    /// left alone, else the light byte to draw it in (0x4b11ee).
    pub fn shade(&self, x: usize, y: usize, strength: i32) -> Option<u32> {
        let s = i32::from(self.at(x, y)) + strength;
        (s < UNSHADED).then(|| s.max(0) as u32)
    }
}

/// Works out a code's mask over a floor tile's shape `tile` (255 off the
/// tile), or None for a code [`valid`] refuses (0x4a3cd3). Each pixel's
/// place on the tile is `u` along the upper right side (0 at the top
/// corner, 1 at the right) and `v` along the upper left (1 at the left
/// corner). Each side's curve is weighted by how near the pixel is to
/// that side, and the sum `r` gives `255 * (1 - r / 2)`, rounded.
pub fn generate(tile: &Mask, code: u8) -> Option<Mask> {
    let k = curves(code)?;
    if !valid(code) {
        return None;
    }
    let (w, h) = (tile.width, tile.height);
    let even = (w + (w & 1)) as f64;
    let scale = 2.0 / even;
    let f = |i: usize, t: f32| curve(k[i], t).unwrap_or(0.0);
    let clamp = |x: f64| x.clamp(0.0, 1.0);
    let mut values = vec![255; w * h];
    for y in 0..h {
        for x in 0..w {
            if tile.at(x, y) == 255 {
                continue;
            }
            let a = (x as f64 - even * 0.5) * 0.5;
            let b = y as f64 - h as f64 * 0.5;
            let u = clamp((b + a) * scale + 0.5) as f32;
            let v = clamp((b - a) * scale + 0.5) as f32;
            // 0x4a3eb1: the sum kept as a float between its terms.
            let (uf, vf) = (f64::from(u), f64::from(v));
            let mut sum = (f(1, v) * (1.0 - uf)) as f32;
            sum = (f(0, u) * (1.0 - vf) + f64::from(sum)) as f32;
            sum = (f(3, v) * uf + f64::from(sum)) as f32;
            let r = f(2, u) * vf + f64::from(sum);
            let value = clamp(1.0 - r * 0.5);
            values[y * w + x] = (value * 255.0 + 0.5) as u8;
        }
    }
    Some(Mask {
        width: w,
        height: h,
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_read_their_own_bit_and_the_corners_at_their_ends() {
        assert_eq!(side(0, 0), 0);
        assert_eq!(side(0b0000_0001, 0), 1);
        assert_eq!(side(0b1000_0001, 0), 3);
        assert_eq!(side(0b0001_0001, 0), 5);
        assert_eq!(side(0b1001_0001, 0), 7);
        // Each corner ends two sides: corner 4 starts sides 0 and 1.
        assert_eq!(side(0b0001_0000, 1), 4);
        assert_eq!(side(0b0010_0000, 1), 2);
        assert_eq!(side(0b0010_0000, 2), 4);
        assert_eq!(side(0b0100_0000, 2), 2);
        assert_eq!(side(0b0100_0000, 3), 2);
        assert_eq!(side(0b1000_0000, 3), 4);
    }

    #[test]
    fn a_corner_counts_only_with_both_sides_beside_it() {
        assert!(valid(0));
        assert!(valid(0b0000_1000));
        assert!(valid(0b0000_0101));
        // Corner 4 lies between sides 0 and 1.
        assert!(!valid(0b0001_0000));
        assert!(!valid(0b0001_0001));
        assert!(valid(0b0001_0011));
        assert!(valid(0xff));
        assert_eq!((0..=255u8).filter(|&c| valid(c)).count(), 47);
    }

    #[test]
    fn turning_moves_each_bit_back_and_four_turns_come_home() {
        assert_eq!(rotate(0b0000_0001, 1), 0b0000_1000);
        assert_eq!(rotate(0b0000_0010, 1), 0b0000_0001);
        assert_eq!(rotate(0b0001_0001, 1), 0b1000_1000);
        assert_eq!(rotate(0b0010_0000, 1), 0b0001_0000);
        assert_eq!(rotate(0b0000_0001, 2), 0b0000_0100);
        assert_eq!(rotate(0b0000_0001, 3), 0b0000_0010);
        // Code 16 is the second hand-drawn mask, not corner 4 alone: it
        // never turns, and no drawable code turns into it.
        assert_eq!(rotate(16, 1), 16);
        for code in (0..=255u8).filter(|&c| valid(c)) {
            assert_eq!(rotate(rotate(code, 1), 3), code);
            assert!(valid(rotate(code, 1)));
        }
    }

    #[test]
    fn group_tiles_see_each_other_across_their_shared_side() {
        // A two-tile bench along x: each part sees the other.
        let bench = |x: i32, y: i32| y == 0 && (0..2).contains(&x);
        assert_eq!(group_code(0, 0, bench), 0b1000);
        assert_eq!(group_code(1, 0, bench), 0b0010);
        // A corner with no side beside it is not drawable: code 0.
        let diagonal = |x: i32, y: i32| (x, y) == (0, 0) || (x, y) == (1, 1);
        assert_eq!(group_code(0, 0, diagonal), 0);
    }

    fn diamond(width: usize, height: usize) -> Mask {
        let half = (width / 2) as i32;
        let values = (0..height)
            .flat_map(|y| {
                (0..width).map(move |x| {
                    let reach = if y < height / 2 {
                        2 * y
                    } else {
                        2 * (height - 1 - y)
                    };
                    if ((x as i32) - half).abs() <= reach as i32 {
                        0
                    } else {
                        255
                    }
                })
            })
            .collect();
        Mask {
            width,
            height,
            values,
        }
    }

    #[test]
    fn a_mask_darkens_toward_the_sides_whose_neighbours_are_shaded() {
        let tile = diamond(31, 16);
        // No neighbours: nothing changes anywhere.
        let none = generate(&tile, 0).unwrap();
        assert!(none.values.iter().all(|&v| v == 255));
        // Side 3 (the lower right): darkest at its middle, untouched at
        // the far corner.
        let mask = generate(&tile, 0b1000).unwrap();
        assert_eq!(mask.at(15, 0), 255);
        assert!(mask.at(19, 11) < mask.at(15, 8));
        assert!(mask.at(15, 8) < 255);
        // Shaded all round: dark everywhere on the tile.
        let all = generate(&tile, 0xff).unwrap();
        assert!(
            all.values
                .iter()
                .zip(&tile.values)
                .all(|(&v, &t)| t == 255 || v == 0)
        );
        assert_eq!(generate(&tile, 0b0001_0000), None);
    }

    #[test]
    fn a_shade_is_kept_to_zero_and_left_out_from_248() {
        let mask = Mask {
            width: 3,
            height: 1,
            values: vec![0, 200, 255],
        };
        assert_eq!(mask.shade(0, 0, -10), Some(0));
        assert_eq!(mask.shade(1, 0, 10), Some(210));
        assert_eq!(mask.shade(1, 0, 47), Some(247));
        assert_eq!(mask.shade(1, 0, 48), None);
        assert_eq!(mask.shade(2, 0, 0), None);
    }
}
