//! Finds a game install on disk and indexes its content.
//!
//! [`Install::detect`] recognizes an install from its files and works out
//! which expansion packs it has. [`Index::build`] then lists every file and
//! FAR archive member under the directories the original scans for
//! objects, in the order it finds them. [`Catalog::build`] registers every object
//! definition by GUID and resolves semi-global links. [`Neighborhood::open`]
//! reads a neighborhood's families and finds its houses, and [`House::load`]
//! reads a saved lot against the catalog. [`Animations`] holds the clips a
//! person's animator plays.

mod animations;
pub mod assets;
mod catalog;
mod error;
pub mod family_import;
mod file_table;
mod import_wardrobe;
mod index;
mod install;
mod neighborhood;
mod people;
mod surfaces;
pub mod ui_graphics;
mod wardrobe;

pub use animations::{Animations, Channel, Clip, Event, Track};
pub use assets::Assets;
pub use catalog::{Catalog, Object};
pub use error::Error;
pub use file_table::SuitTable;
pub use import_wardrobe::ImportWardrobe;
pub use index::{Index, Member, Resource, far_members};
pub use install::{Expansions, Install, Layout, Mount};
pub use neighborhood::{House, Neighborhood, PENDING_IMPORT};
pub use people::{AvatarRegistry, Bone, People, Profile, Skeleton, skeleton_name, skeleton_scales};
pub use surfaces::{SurfaceCatalog, SurfaceFile, SurfaceKind, SurfacePlugin, Surfaces, atoi};
pub use wardrobe::Wardrobe;

pub type Result<T> = std::result::Result<T, Error>;
