//! Finds a game install on disk and indexes its content.
//!
//! [`Install::detect`] recognizes an install from its files and works out
//! which expansion packs it has. [`Index::build`] then lists every file and
//! FAR archive member under the directories the original scans for
//! objects, in the order it finds them. [`Catalog::build`] registers every object
//! definition by GUID and resolves semi-global links. [`Neighborhood::open`]
//! reads a neighborhood's families and finds its houses, and [`House::load`]
//! reads a saved lot against the catalog.

mod catalog;
mod error;
mod file_table;
mod index;
mod install;
mod neighborhood;

pub use catalog::{Catalog, Object};
pub use error::Error;
pub use index::{Index, Member, Resource};
pub use install::{Expansions, Install, Layout, Mount};
pub use neighborhood::{House, Neighborhood};

pub type Result<T> = std::result::Result<T, Error>;
