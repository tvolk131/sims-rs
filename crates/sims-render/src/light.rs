//! How a room's light darkens what is drawn in it.
//!
//! The original stores a light byte for each tile: its room's light times
//! 255 ([`crate::lot::light_byte`]). Drawing reads the byte back as a
//! fraction and keeps it between 0.5 and 1 (the old prototype's reading of
//! the drawing context, `legacy/crates/sims-vm/src/scheduler/clock/draw/
//! context/light.rs`): even an unlit room shows at half brightness.
//!
//! How that fraction darkens a pixel was fitted to a screenshot of the
//! original, not read from its code. The original draws in 16-bit color
//! (5 bits red, 6 green, 5 blue). Each channel is multiplied by a factor in
//! 256ths and rounded up; blue's factor is one 256th higher than red's and
//! green's. At half light this gives 69% of the indoor pixels of the
//! Goth house exactly (see `docs/decisions.md`). Other light levels are
//! **unverified**.

/// A light to draw with: a factor per channel, in 256ths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Light([u32; 3]);

impl Light {
    /// Full light: colors are drawn as they are.
    pub const FULL: Light = Light([256; 3]);

    /// The light for a tile's light byte.
    pub fn from_byte(byte: u8) -> Self {
        // The drawing context keeps the fraction between 0.5 and 1.
        let level = (f32::from(byte) / 255.0).clamp(0.5, 1.0);
        let k = (level * 256.0).round() as u32;
        if k >= 256 {
            Self::FULL
        } else {
            Light([k, k, k + 1])
        }
    }

    pub fn is_full(self) -> bool {
        self == Self::FULL
    }

    /// A color under this light. Full light leaves it as it is; any other
    /// light gives a 16-bit color, widened back to 8 bits a channel.
    pub fn apply(self, [r, g, b]: [u8; 3]) -> [u8; 3] {
        if self.is_full() {
            return [r, g, b];
        }
        let scale = |v: u8, k: u32| ((u32::from(v) * k + 255) >> 8).min(255) as u8;
        let (r, g, b) = (
            scale(r >> 3, self.0[0]),
            scale(g >> 2, self.0[1]),
            scale(b >> 3, self.0[2]),
        );
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
        assert_eq!(Light::from_byte(0), Light([128, 128, 129]));
        assert_eq!(Light::from_byte(127), Light([128, 128, 129]));
        assert_eq!(Light::from_byte(255), Light::FULL);
        assert_eq!(Light::from_byte(204), Light([205, 205, 206]));
    }

    #[test]
    fn half_light_halves_each_channel_rounding_up_blue_a_little_more() {
        let half = Light::from_byte(0);
        // Red 21 of 31, green 40 of 63, blue 20 of 31.
        let c = [21 << 3, 40 << 2, 20 << 3];
        let [r, g, b] = half.apply(c);
        assert_eq!((r >> 3, g >> 2, b >> 3), (11, 20, 11));
        assert_eq!(Light::FULL.apply([1, 2, 3]), [1, 2, 3]);
        assert_eq!(half.apply([0, 0, 0]), [0, 0, 0]);
        let [r, g, b] = half.apply([255, 255, 255]);
        assert_eq!((r >> 3, g >> 2, b >> 3), (16, 32, 16));
    }
}
