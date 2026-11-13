//! Deterministic building blocks shared by every simulation crate.
//!
//! Nothing here reads the clock, iterates hash maps, or calls platform math.
//! See docs/vision.md, "Simulation".

pub mod float;
pub mod rng;

pub use float::{Ext, F32};
pub use rng::{CrtRng, GameRng};
