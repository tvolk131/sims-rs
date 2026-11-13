//! Finds a game install on disk and indexes its content.
//!
//! [`Install::detect`] recognizes an install from its files and works out
//! which expansion packs it has. [`Index::build`] then lists every file and
//! FAR archive member under the directories the original mounts, in the
//! original's mount order. [`Catalog::build`] registers every object
//! definition by GUID and resolves semi-global links.

mod catalog;
mod error;
mod index;
mod install;

pub use catalog::{Catalog, Object};
pub use error::Error;
pub use index::{Index, Member, Resource};
pub use install::{Expansions, Install, Layout, Mount};

pub type Result<T> = std::result::Result<T, Error>;
