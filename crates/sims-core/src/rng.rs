//! The original game's random number generators.
//!
//! Every random draw in the simulation comes from a stream owned by the
//! simulation state, so a saved state replays exactly.

use serde::{Deserialize, Serialize};

/// The game's main generator, a linear congruential generator. The original
/// runs two independent streams of it, one for the current lot and one
/// global, so callers keep two instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GameRng {
    state: u32,
}

impl GameRng {
    pub const fn from_state(state: u32) -> Self {
        Self { state }
    }

    pub const fn state(self) -> u32 {
        self.state
    }

    /// Advances the stream. The state keeps the low 32 bits of
    /// `state * 0x41c64e6d + 0x3039`; the output is bits 16..48 of the full
    /// 64-bit product. Unlike the C library's `rand`, the output isn't masked
    /// to 15 bits.
    pub fn next_u32(&mut self) -> u32 {
        let product = u64::from(self.state) * 0x41c6_4e6d + 0x3039;
        self.state = product as u32;
        (product >> 16) as u32
    }
}

/// The C runtime's `rand`, which the original uses in a few places (for
/// example, default personality traits). It has its own state, separate from
/// [`GameRng`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CrtRng {
    state: u32,
}

impl CrtRng {
    pub const fn from_state(state: u32) -> Self {
        Self { state }
    }

    pub const fn state(self) -> u32 {
        self.state
    }

    /// Returns a value in `0..=0x7fff`.
    pub fn next_u15(&mut self) -> u16 {
        self.state = self
            .state
            .wrapping_mul(0x0003_43fd)
            .wrapping_add(0x0026_9ec3);
        ((self.state >> 16) & 0x7fff) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_stream_matches_original_outputs() {
        let mut rng = GameRng::from_state(1);
        for (output, state) in [
            (16838, 1_103_527_590),
            (1_507_104_382, 2_524_885_223),
            (3_204_589_441, 662_824_084),
            (2_509_948_011, 3_295_386_429),
        ] {
            assert_eq!(rng.next_u32(), output);
            assert_eq!(rng.state(), state);
        }
    }

    #[test]
    fn crt_stream_matches_msvc_rand() {
        // MSVC `rand()` after `srand(1)`.
        let mut rng = CrtRng::from_state(1);
        let outputs: Vec<u16> = (0..5).map(|_| rng.next_u15()).collect();
        assert_eq!(outputs, [41, 18467, 6334, 26500, 19169]);
    }
}
