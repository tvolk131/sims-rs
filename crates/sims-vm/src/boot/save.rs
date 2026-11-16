//! Saving a lot as it stands between two steps, and going on from a save.
//!
//! A [`Saved`] holds what the next step depends on: the world, and the
//! lot's own state outside it. That is the ticks run since the load (the
//! frames follow that count, [`Frames::after`](super::Frames::after)),
//! the tick count the load left, the answers still to give, play mode's
//! objects reset this tick, and the house view's rotation, which some
//! primitives read. It is the state a step that waits for the player sets
//! aside before its tick, and whether a dialog was answered and the tick
//! that showed it has not run again (the lot then takes no input). So a
//! lot restored from a save steps, and takes input, exactly as the saved
//! lot would have.
//!
//! The program is not saved. A save goes back into a lot loaded from the
//! same install and house: [`Lot::restore`] checks the house and family
//! the load selected, and the world's hash as the load left it, which
//! the install, the neighborhood and the house file decide. Saving is this reimplementation's, not the
//! original's: the original saves the house file and the neighborhood,
//! which is not ported.

use super::Lot;
use serde::{Deserialize, Serialize};
use sims_sim::World;
use std::collections::{BTreeSet, VecDeque};

/// The version of [`Saved::encode`]'s layout, bumped when it changes. The
/// world inside keeps [`sims_sim::SNAPSHOT_FORMAT`], which is written too.
pub const FORMAT: u32 = 1;

/// A lot as it stood between two steps ([`Lot::save`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Saved {
    world: World,
    ticks: u32,
    start: u32,
    answers: VecDeque<u8>,
    fresh: BTreeSet<u16>,
    view_rotation: u8,
    answered: bool,
    house: u32,
    family: u16,
    loaded: [u8; 32],
}

/// Why a lot could not be saved or restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The load did not finish, so the lot cannot step.
    Incomplete,
    /// A step failed partway, so the world is not between two steps.
    Halted,
    /// The save is of another house or family than the lot was loaded
    /// with.
    OtherHouse {
        saved: (u32, u16),
        loaded: (u32, u16),
    },
    /// The save is of the same house number and family, but of a lot the
    /// load left otherwise: another neighborhood, install or house file.
    OtherLoad,
    /// The bytes are not a save this version reads.
    Format(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Incomplete => write!(f, "the lot's load did not finish"),
            Self::Halted => write!(f, "the lot stopped partway through a step"),
            Self::OtherHouse { saved, loaded } => write!(
                f,
                "the save is of house {} (family {}), the lot of house {} (family {})",
                saved.0, saved.1, loaded.0, loaded.1
            ),
            Self::OtherLoad => write!(
                f,
                "the save is of a lot loaded from other files (another neighborhood or install)"
            ),
            Self::Format(reason) => write!(f, "not a readable save: {reason}"),
        }
    }
}

impl std::error::Error for SaveError {}

impl Saved {
    /// The saved world.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// The bytes a save store keeps: [`FORMAT`] and the world's snapshot
    /// format (each a little-endian u32), then the canonical encoding
    /// (postcard, as [`World::encode`]).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = FORMAT.to_le_bytes().to_vec();
        out.extend(sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
        out.extend(postcard::to_allocvec(self).expect("encoding to memory cannot fail"));
        out
    }

    /// Reads [`Saved::encode`]'s bytes. A save of another layout or world
    /// format is refused rather than misread.
    pub fn decode(bytes: &[u8]) -> Result<Self, SaveError> {
        let format = |reason: String| SaveError::Format(reason);
        let (header, body) = bytes
            .split_first_chunk::<8>()
            .ok_or_else(|| format("shorter than its header".into()))?;
        let layout = u32::from_le_bytes(header[..4].try_into().unwrap());
        let world = u32::from_le_bytes(header[4..].try_into().unwrap());
        if layout != FORMAT || world != sims_sim::SNAPSHOT_FORMAT {
            return Err(format(format!(
                "save format {layout} with world format {world}, expected {FORMAT} with {}",
                sims_sim::SNAPSHOT_FORMAT
            )));
        }
        postcard::from_bytes(body).map_err(|e| format(e.to_string()))
    }
}

impl Lot {
    /// The lot as it stands, to go on from later ([`Lot::restore`]). Call
    /// it between steps. While a dialog waits for the player
    /// ([`Lot::asking`]) the world is as it was before the tick that
    /// showed it, so a restored lot runs that tick again and asks again.
    /// Once answered, until that tick runs again, the save keeps the
    /// answer, and the restored lot takes no input before its next step.
    pub fn save(&self) -> Result<Saved, SaveError> {
        if self.load.finished.is_none() {
            return Err(SaveError::Incomplete);
        }
        if self.halted {
            return Err(SaveError::Halted);
        }
        Ok(Saved {
            world: self.world.clone(),
            ticks: self.ticks,
            start: self.start,
            answers: self.player.queue.clone(),
            fresh: self.play.fresh.clone(),
            view_rotation: self.view_rotation,
            answered: self.answered,
            house: self.load.house,
            family: self.load.family,
            loaded: self.loaded,
        })
    }

    /// Goes on from `saved`, a save of a lot loaded from the same install
    /// and house: the lot then steps exactly as the saved lot would have.
    /// A dialog this lot waited on, or a failed step, is forgotten; a
    /// dialog the saved lot had answered, whose tick had not run again,
    /// is answered here too.
    /// The frames drawn and whether the lot waits for the player's answers
    /// are this lot's, given at its load.
    pub fn restore(&mut self, saved: Saved) -> Result<(), SaveError> {
        if self.load.finished.is_none() {
            return Err(SaveError::Incomplete);
        }
        let loaded = (self.load.house, self.load.family);
        if (saved.house, saved.family) != loaded {
            return Err(SaveError::OtherHouse {
                saved: (saved.house, saved.family),
                loaded,
            });
        }
        if saved.loaded != self.loaded {
            return Err(SaveError::OtherLoad);
        }
        self.world = saved.world;
        self.ticks = saved.ticks;
        self.start = saved.start;
        self.player.queue = saved.answers;
        self.play.fresh = saved.fresh;
        self.view_rotation = saved.view_rotation;
        self.halted = false;
        self.asking = None;
        self.answered = saved.answered;
        Ok(())
    }
}
