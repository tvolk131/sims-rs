//! Draws a lot the way the original does: terrain, floors, walls and the
//! objects' pre-rendered sprites, at the three zooms and four rotations.
//!
//! It reads the lot and its objects from a [`sims_sim::World`] and never
//! changes it, and the game's art through `sims-formats` and
//! `sims-content`. Everything is drawn on the CPU into a [`Canvas`], an
//! image with a depth buffer, so tests run without a window or GPU.
//!
//! What was compared with the original, and what is not yet, is in
//! `docs/decisions/`.

pub mod assets;
pub mod build_preview;
pub mod cache;
pub mod camera;
pub mod canvas;
pub mod caps;
pub mod censor;
pub mod cutaway;
pub mod depth;
pub mod grass;
pub mod light;
pub mod lot;
pub mod neighborhood;
pub mod open;
pub mod png;
pub mod pools;
pub mod scene;
pub mod shadows;

pub use assets::{Assets, BaseAssets};
pub use cache::{CacheHit, LotCache};
pub use camera::{Camera, Walls, Zoom};
pub use canvas::{Canvas, EMPTY_DEPTH, ID_NONE, Ids, Image, clamp_depth, pack_rgb, unpack_rgb};
pub use censor::{Censored, draw_censor};
pub use cutaway::{Focus, Selected, screen_centre_pick, unproject_terrain};
pub use light::Light;
pub use lot::Lot;
pub use open::{Opened, open_house};
pub use scene::{
    PersonAttach, RenderOptions, Stats, draw, draw_held, draw_held_by, draw_held_except,
    draw_held_with_attach, draw_held_with_options, draw_lot, draw_lot_in_viewport,
    draw_lot_in_viewport_with_options, draw_lot_with_preview, draw_lot_with_preview_and_options,
    held_by, holds_person,
};

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
