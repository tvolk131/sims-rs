//! Finds a game install on disk and indexes its content.
//!
//! [`Install::detect`] recognizes an install from its files and works out
//! which expansion packs it has. [`Index::build`] then lists every file and
//! FAR archive member under the directories the original mounts, in the
//! original's mount order.

mod error;
mod index;
mod install;

pub use error::Error;
pub use index::{Index, Member, Resource};
pub use install::{Expansions, Install, Layout, Mount};

pub type Result<T> = std::result::Result<T, Error>;
