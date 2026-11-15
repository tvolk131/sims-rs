//! Draws a lot the way the original does: terrain, floors, walls and the
//! objects' pre-rendered sprites, at the three zooms and four rotations.
//!
//! It reads the lot and its objects from a [`sims_sim::World`] and never
//! changes it, and the game's art through `sims-formats` and
//! `sims-content`. Everything is drawn on the CPU into a [`Canvas`], an
//! image with a depth buffer, so tests run without a window or GPU.
//!
//! What was compared with the original, and what is not yet, is in
//! `docs/decisions.md`.

pub mod assets;
pub mod camera;
pub mod canvas;
pub mod caps;
pub mod cutaway;
pub mod depth;
pub mod grass;
pub mod light;
pub mod lot;
pub mod open;
pub mod png;
pub mod pools;
pub mod scene;

pub use assets::Assets;
pub use camera::{Camera, Walls, Zoom};
pub use canvas::{Canvas, Image};
pub use cutaway::{Focus, Selected};
pub use light::Light;
pub use lot::Lot;
pub use open::{Opened, open_house};
pub use scene::{Stats, draw};

/// An error with a message saying what failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<sims_content::Error> for Error {
    fn from(e: sims_content::Error) -> Self {
        Self(e.to_string())
    }
}

impl From<sims_formats::Error> for Error {
    fn from(e: sims_formats::Error) -> Self {
        Self(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
