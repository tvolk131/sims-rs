//! The global application-time owner at006e54e0, updated by473509.
//!
//! The frontend supplies the external clock-service sample. This owner clamps
//! elapsed advances and retains its offset; its `now` then feeds lot clocks and
//! UI timing. It is distinct from the lot timer/elapsed caller.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub now: u32,
    pub delta: u32,
    pub last_raw_sample: u32,
    pub generation: u32,
    pub offset: u32,
    pub maximum_delta: u32,
}
impl Fields {
    pub const fn from_words(words: [u32; 6]) -> Self {
        let [
            now,
            delta,
            last_raw_sample,
            generation,
            offset,
            maximum_delta,
        ] = words;
        Self {
            now,
            delta,
            last_raw_sample,
            generation,
            offset,
            maximum_delta,
        }
    }
    pub const fn words(self) -> [u32; 6] {
        [
            self.now,
            self.delta,
            self.last_raw_sample,
            self.generation,
            self.offset,
            self.maximum_delta,
        ]
    }
    /// Original unsigned32-bit arithmetic, including wrap and the strict JA
    /// clamp. No host clock is sampled and no lot/VM state is advanced here.
    pub fn update(&mut self, raw_sample: u32) {
        let prospective = self.offset.wrapping_add(raw_sample);
        self.last_raw_sample = raw_sample;
        self.delta = prospective.wrapping_sub(self.now);
        if self.delta > self.maximum_delta {
            self.delta = self.maximum_delta;
            self.now = self.now.wrapping_add(self.maximum_delta);
            self.offset = self.now.wrapping_sub(raw_sample);
        } else {
            self.now = prospective;
        }
        self.generation = self.generation.wrapping_add(1);
    }
}
