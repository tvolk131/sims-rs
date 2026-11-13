//! The simulation state of one lot.
//!
//! [`World::from_house`] builds the state a saved house describes: every
//! object with its attributes, data and script stack, the scheduler cursor,
//! both random streams, the budget and the lot arrays. [`World::hash`]
//! fingerprints it, and [`Snapshots`] stores it in a REDB file. [`load`]
//! holds the load steps that change state without running scripts.
//!
//! The state holds every field the readers decode. Bytes the readers keep
//! but do not understand (format tails, raw records) are left out: they are
//! not state anything here can act on.

mod error;
pub mod load;
mod snapshot;
mod world;

pub use error::Error;
pub use snapshot::Snapshots;
pub use world::{Budget, Object, World};

pub type Result<T> = std::result::Result<T, Error>;
