//! Draws the Sims: their skeletons, appearances, meshes and skins, posed
//! from animation clips and skinned on the CPU, and drawn offscreen by a
//! small software rasterizer.
//!
//! This is presentation. It reads the simulation's state and never
//! changes it, and the simulation never depends on it, so its math is
//! ordinary `f32`. [`assets`] finds the files, [`texture`] decodes skins,
//! [`rig`] poses a skeleton, [`appearance`] resolves what a person wears,
//! [`skin`] blends the mesh vertices over the posed bones, and [`raster`]
//! draws the result.

// Drawing never feeds simulation state (see the workspace lint table).
#![allow(clippy::disallowed_methods)]

pub mod appearance;
pub mod assets;
pub mod house;
pub mod math;
pub mod raster;
pub mod rig;
pub mod skin;
pub mod texture;

pub use appearance::{Body, Outfit, Part};
pub use assets::Assets;
pub use house::{Resident, residents};
pub use skin::skin;
pub use texture::{Texture, read_bmp};
