//! How a room's light darkens what is drawn in it.
//!
//! The original stores a light byte for each tile: its room's light times
//! 255 ([`crate::lot::light_byte`]). Drawing reads the byte back as a
//! fraction and keeps it between 0.5 and 1 (the old prototype's reading of
//! the drawing context, `legacy/crates/sims-vm/src/scheduler/clock/draw/
//! context/light.rs`): even an unlit room shows at half brightness.
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

/// A light to draw with: one of the original's 32 light levels, or full
/// light.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Light(Option<u8>);

impl Light {
    /// Full light: colors are drawn as they are.
    pub const FULL: Light = Light(None);

    /// The light for a tile's light byte.
    pub fn from_byte(byte: u8) -> Self {
        Self::drawn_byte(byte).map_or(Self::FULL, Self::from_sum)
    }

    /// The light byte a tile's light byte is drawn with, or None for full
    /// light. The drawing context keeps the fraction between 0.5 and 1,
    /// and a fraction of 1 draws in full light (0x4c7c59); any other
    /// becomes `fraction * 255 + 0.5`, truncated (0x4b76a8).
    pub fn drawn_byte(byte: u8) -> Option<u32> {
        let fraction = (f32::from(byte) / 255.0).clamp(0.5, 1.0);
        if (fraction - 1.0).abs() < 1e-6 {
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
    fn light_is_kept_between_half_and_full() {
        assert_eq!(Light::from_byte(0), Light(Some(16)));
        assert_eq!(Light::from_byte(127), Light(Some(16)));
        assert_eq!(Light::from_byte(255), Light::FULL);
        // 204 / 255 * 255 + 0.5 truncates to 204, level 25.
        assert_eq!(Light::from_byte(204), Light(Some(25)));
        assert_eq!(Light::from_byte(254), Light(Some(31)));
    }

    #[test]
    fn a_pool_adds_to_the_drawn_byte_up_to_255() {
        assert_eq!(Light::drawn_byte(255), None);
        assert_eq!(Light::drawn_byte(162), Some(162));
        assert_eq!(Light::drawn_byte(0), Some(128));
        assert_eq!(Light::from_sum(162 + 40), Light(Some(25)));
        assert_eq!(Light::from_sum(162 + 255), Light(Some(31)));
    }

    #[test]
    fn half_light_matches_the_original_tables() {
        let half = Light::from_byte(0);
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
        let light = Light::from_byte(254);
        assert_eq!(light.apply([255, 255, 255]), [255, 251, 255]);
        assert_eq!(light.apply([0, 0, 0])[2] >> 3, 1);
    }
}
