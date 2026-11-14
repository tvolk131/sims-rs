//! The simulation state of one lot.
//!
//! The original keeps one lot for as long as it runs and loads each house
//! into it, and [`World`] is that lot. [`World::empty`] is the lot before
//! any load, and [`World::read_simulator`] and [`World::read_arrays`] read
//! parts of a house file into it. The saved objects are read by `sims-vm`,
//! which holds the object definitions the original constructs them from.
//! [`World::from_house`] reads a whole saved house at once: every
//! object with its attributes, data and script stack, the scheduler cursor,
//! both random streams, the budget and the lot arrays. [`World::hash`]
//! fingerprints the state, and [`Snapshots`] stores it in a REDB file.
//! [`load`] holds the load steps that change state without running scripts,
//! and [`rooms`] works out the lot's rooms from its walls and counts
//! them: their size, what stands in them, their light and their score.
//! [`motives`] decays a person's motives and works out the mood.
//!
//! The state holds every field the readers decode. Bytes the readers keep
//! but do not understand (format tails, raw records) are left out: they are
//! not state anything here can act on.

mod error;
pub mod load;
pub mod motives;
pub mod rooms;
mod snapshot;
mod world;

pub use error::Error;
pub use snapshot::Snapshots;
pub use world::{
    Budget, Dialog, Dialogs, FULL_SIZE, FrameStorage, MagicHistory, Object, PersonTick,
    TypeAttributes, World,
};

pub type Result<T> = std::result::Result<T, Error>;
