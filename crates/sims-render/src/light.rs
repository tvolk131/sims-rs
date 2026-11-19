//! How a room's light darkens what is drawn in it.
//!
//! The original stores a light byte for each tile: its room's light times
//! 255 ([`crate::lot::light_byte`]). Drawing reads the byte back as a
//! fraction and keeps it between 0.5 and the brightest a lit tile may be
//! (the setter 0x4b4525, and the old prototype's reading of it,
//! `legacy/crates/sims-vm/src/scheduler/clock/draw/context/light.rs`):
//! even an unlit room shows at half brightness. The brightest is 1 by day
//! and 0.8 otherwise ([`brightest`]).
//!
//! How that fraction darkens a pixel is read from the original's code.
//! The original draws in 16-bit color (5 bits red, 6 green, 5 blue). A
//! fraction of exactly 1 draws the colors as they are (0x4c7c59). Any
//! other fraction becomes a light byte, `fraction * 255 + 0.5` truncated
//! (0x4b76a8), and its top five bits pick one of 32 levels `l`. Each
//! channel is then looked up in a table the game fills at start-up
//! (0x49eea8, read through 0x438e6d), every entry truncated after adding
//! the constant shown:
//!
//! ```text
//! red   = l * red / 31 + 0.5
//! green = 2 * l * green / 63 + 0.5
//! blue  = (max(l, 1) - 1) * blue / 31 + 1.5
//! ```
//!
//! Blue takes the level below and one step more, so black under any light
//! shows a little blue.

/// The brightest a lit tile may be drawn, by the time of day (global 4: 0
/// day, 1 dusk, 2 night, 3 dawn): 1 by day and the float 0.8 otherwise.
/// The lot sets it on every change of the time of day and at the end of a
/// load (signal 0xdd, 0x466ba3, at 0x6c741c), and only the drawing's light
/// setter reads it (0x4b4525).
pub fn brightest(time_of_day: i16) -> f32 {
    if time_of_day == 0 {
        1.0
    } else {
        f32::from_bits(0x3f4c_cccd)
    }
}

/// A light to draw with: one of the original's 32 light levels, or full
/// light.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Light(Option<u8>);

impl Light {
    /// Full light: colors are drawn as they are.
    pub const FULL: Light = Light(None);

    /// The light for a tile's light byte, drawn no brighter than
    /// `brightest` ([`brightest`]).
    pub fn from_byte(byte: u8, brightest: f32) -> Self {
        Self::drawn_byte(byte, brightest).map_or(Self::FULL, Self::from_sum)
    }

    /// The light byte a tile's light byte is drawn with, or None for full
    /// light: the byte as a fraction, as [`Light::drawn_fraction`] draws it.
    pub fn drawn_byte(byte: u8, brightest: f32) -> Option<u32> {
        Self::drawn_fraction(f32::from(byte) / 255.0, brightest)
    }

    /// The light for a light given as a fraction, as the original passes a
    /// room's light straight to the drawing context (the wall-top trim,
    /// 0x53e2f5).
    pub fn from_fraction(light: f32, brightest: f32) -> Self {
        Self::drawn_fraction(light, brightest).map_or(Self::FULL, Self::from_sum)
    }

    /// The light byte a fraction is drawn with, or None for full light.
    /// The drawing context keeps the fraction between 0.5 and `brightest`
    /// (0x4b4525: a fraction below 0.5, or not a number, becomes 0.5; then
    /// it becomes `brightest` unless `brightest` is at least it). A
    /// fraction within a millionth of 1 draws in full light (0x4c7c59,
    /// the float 0x358637bd); any other becomes `fraction * 255 + 0.5`,
    /// truncated (0x4b76a8). After dark the cap keeps the drawing context
    /// out of full light.
    fn drawn_fraction(light: f32, brightest: f32) -> Option<u32> {
        let fraction = if light >= 0.5 { light } else { 0.5 };
        let fraction = if brightest >= fraction {
            fraction
        } else {
            brightest
        };
        let tolerance = f64::from(f32::from_bits(0x3586_37bd));
        if (f64::from(fraction) - 1.0).abs() <= tolerance {
            return None;
        }
        Some((f64::from(fraction) * 255.0 + 0.5) as u32)
    }

    /// The light for a drawn light byte plus whatever a lamp's pool adds,
    /// kept to 255 (0x4b6c17): never full light, even at 255.
    pub fn from_sum(sum: u32) -> Self {
        Light(Some((sum.min(255) >> 3) as u8))
    }

    pub fn is_full(self) -> bool {
        self == Self::FULL
    }

    /// A color under this light. Full light leaves it as it is; any other
    /// light gives a 16-bit color, widened back to 8 bits a channel.
    pub fn apply(self, [r, g, b]: [u8; 3]) -> [u8; 3] {
        let Some(level) = self.0 else {
            return [r, g, b];
        };
        let l = f64::from(level);
        let entry = |x: f64| x as u8;
        let r = entry(l * f64::from(r >> 3) * f64::from(1.0f32 / 31.0) + 0.5);
        let g = entry(2.0 * l * f64::from(g >> 2) * f64::from(1.0f32 / 63.0) + 0.5);
        let b = entry((l.max(1.0) - 1.0) * f64::from(b >> 3) * f64::from(1.0f32 / 31.0) + 1.5);
        let (r, g, b) = (r.min(31), g.min(63), b.min(31));
        [
            (r << 3) | (r >> 2),
            (g << 2) | (g >> 4),
            (b << 3) | (b >> 2),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fraction_short_of_one_is_not_full_light() {
        assert_eq!(Light::from_fraction(1.0, 1.0), Light::FULL);
        assert_eq!(Light::from_fraction(2.0, 1.0), Light::FULL);
        assert_eq!(Light::from_fraction(0.999, 1.0), Light(Some(31)));
        assert_eq!(Light::from_fraction(0.2, 1.0), Light(Some(16)));
        assert_eq!(Light::from_fraction(f32::NAN, 1.0), Light(Some(16)));
        assert_eq!(Light::from_byte(255, 1.0), Light::FULL);
    }

    #[test]
    fn after_dark_a_lit_tile_is_drawn_at_most_at_eight_tenths() {
        let night = brightest(2);
        assert_eq!(brightest(0), 1.0);
        assert_eq!(brightest(1), night);
        assert_eq!(brightest(3), night);
        // 0.8 as a float: 0.8 * 255 + 0.5 truncates to 204, level 25.
        assert_eq!(Light::from_byte(255, night), Light(Some(25)));
        assert_eq!(Light::from_fraction(1.0, night), Light(Some(25)));
        assert_eq!(Light::drawn_byte(255, night), Some(204));
        // Below the cap a light is drawn as by day; the floor is still 0.5.
        assert_eq!(Light::from_byte(153, night), Light::from_byte(153, 1.0));
        assert_eq!(Light::from_byte(0, night), Light(Some(16)));
        assert_eq!(Light::from_fraction(f32::NAN, night), Light(Some(16)));
    }

    #[test]
    fn a_light_within_a_millionth_of_one_is_full() {
        let below = 1.0 - 5e-7f32;
        assert_eq!(Light::from_fraction(below, 1.0), Light::FULL);
        let further = 1.0 - 2e-6f32;
        assert_eq!(Light::from_fraction(further, 1.0), Light(Some(31)));
    }

    #[test]
    fn light_is_kept_between_half_and_full() {
        assert_eq!(Light::from_byte(0, 1.0), Light(Some(16)));
        assert_eq!(Light::from_byte(127, 1.0), Light(Some(16)));
        assert_eq!(Light::from_byte(255, 1.0), Light::FULL);
        // 204 / 255 * 255 + 0.5 truncates to 204, level 25.
        assert_eq!(Light::from_byte(204, 1.0), Light(Some(25)));
        assert_eq!(Light::from_byte(254, 1.0), Light(Some(31)));
    }

    #[test]
    fn a_pool_adds_to_the_drawn_byte_up_to_255() {
        assert_eq!(Light::drawn_byte(255, 1.0), None);
        assert_eq!(Light::drawn_byte(162, 1.0), Some(162));
        assert_eq!(Light::drawn_byte(0, 1.0), Some(128));
        assert_eq!(Light::from_sum(162 + 40), Light(Some(25)));
        assert_eq!(Light::from_sum(162 + 255), Light(Some(31)));
    }

    #[test]
    fn half_light_matches_the_original_tables() {
        let half = Light::from_byte(0, 1.0);
        // Red 21 of 31, green 40 of 63, blue 20 of 31.
        let c = [21 << 3, 40 << 2, 20 << 3];
        let [r, g, b] = half.apply(c);
        assert_eq!((r >> 3, g >> 2, b >> 3), (11, 20, 11));
        assert_eq!(Light::FULL.apply([1, 2, 3]), [1, 2, 3]);
        // Blue takes one step more than red's rule would give: blues 0, 2
        // and 4 become 1, 2 and 3 (red's rule: 0, 1 and 2).
        let blue = |v: u8| half.apply([0, 0, v << 3])[2] >> 3;
        assert_eq!(
            [blue(0), blue(1), blue(2), blue(3), blue(4)],
            [1, 1, 2, 2, 3]
        );
        // White, expanded back to eight bits by repeating the top bits.
        assert_eq!(half.apply([255, 255, 255]), [132, 130, 132]);
    }

    #[test]
    fn nearly_full_light_still_shifts_blue() {
        // Level 31 keeps red, takes green down a step (63 to 62), and
        // gives blue level 30, whose extra 1.5 rounds it back to 31.
        let light = Light::from_byte(254, 1.0);
        assert_eq!(light.apply([255, 255, 255]), [255, 251, 255]);
        assert_eq!(light.apply([0, 0, 0])[2] >> 3, 1);
    }
}
