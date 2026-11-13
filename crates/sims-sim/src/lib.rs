//! The simulation state of one lot.
//!
//! The original keeps one lot for as long as it runs and loads each house
//! into it, and [`World`] is that lot. [`World::empty`] is the lot before
//! any load and [`World::read_simulator`] reads a house's simulator state
//! into it. [`World::from_house`] reads a whole saved house at once: every
//! object with its attributes, data and script stack, the scheduler cursor,
//! both random streams, the budget and the lot arrays. [`World::hash`]
//! fingerprints the state, and [`Snapshots`] stores it in a REDB file.
//! [`load`] holds the load steps that change state without running scripts.
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
pub use world::{Budget, FULL_SIZE, MagicHistory, Object, TypeAttributes, World};

pub type Result<T> = std::result::Result<T, Error>;
