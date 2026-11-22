//! Draws the Sims: their skeletons, appearances, meshes and skins, posed
//! from animation clips and skinned on the CPU, and drawn offscreen by a
//! small software rasterizer.
//!
//! This is presentation. It reads the simulation's state and never
//! changes it, and the simulation never depends on it, so its math is
//! ordinary `f32`. [`assets`] finds the files, [`texture`] decodes skins,
//! [`rig`] poses a skeleton, [`appearance`] resolves what a person wears
//! and [`suit`] what their suit changes it to,
//! [`animation`] poses it from a clip, [`person`] places it and picks the
//! clips from the simulation's animator, [`skin()`] blends the mesh vertices
//! over the posed bones, and [`raster`] draws the result.

// Drawing never feeds simulation state (see the workspace lint table).
#![allow(clippy::disallowed_methods)]

pub mod animation;
pub mod appearance;
/// The person files of an install, found by [`sims_content::assets`].
pub use sims_content::assets;
pub mod bob;
pub mod caf_preview;
pub mod house;
pub mod lot;
pub mod math;
pub mod person;
pub mod pie_head;
pub mod raster;
pub mod rig;
pub mod skin;
pub mod suit;
pub mod texture;

pub use animation::{Clip, Library};
pub use appearance::{Body, Outfit, Part};
pub use assets::Assets;
pub use house::{Cast, PreloadedCast, Resident, residents, residents_of};
pub use lot::{People, View};
pub use person::{Player, base_clip, heading, placement, walk_clip};
pub use skin::skin;
pub use texture::{Texture, read_bmp};
