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
//! the load selected, and the world's hash as the load left it. That hash
//! tells apart another house file or neighborhood, and anything else the
//! load puts in the world. It does not cover the install's code and
//! constants, which the world does not hold: a save restored into a lot
//! of an install whose code or tuning differs is not refused, and is not
//! known to step as the saved lot would have. A save trusts that it goes
//! back onto the same install. Saving is this reimplementation's, not the
//! original's: the original saves the house file and the neighborhood,
//! which is not ported.

use super::Lot;
use crate::Answers;
use serde::{Deserialize, Serialize};
use sims_sim::World;
use std::collections::BTreeSet;

/// The version of [`Saved::encode`]'s layout: the fields of [`Saved`],
/// their order and their types (postcard is not self-describing). Bump it
/// in the commit that changes any of them. The world inside is versioned
/// by [`sims_sim::SNAPSHOT_FORMAT`], which is written too. A save of
/// another version is refused, not migrated. Layout 2 keeps each pending
/// dialog answer's bitmap ID ([`crate::Answer`]); layout 3 kept the
/// pre-resolved house view centre; layout 4 keeps the house file's saved
/// floor rotation baseline (`HOUS 0` `camera_direction & 3`); layout 5
/// keeps the view-rotated screen-centre tile pick (`0x4a7b58` cut via
/// `0x433067`) and any frozen dialog-replay view; layout 6 keeps choice-only
/// follow-up answers (`Answers::choices_after`) separately from explicit
/// dialog replay queues and records the neighborhood's `deletion_epoch` at save
/// time so reusing a deleted character GUID never revives stale instances on
/// older saved lots.
pub const FORMAT: u32 = 6;

/// A lot as it stood between two steps ([`Lot::save`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Saved {
    pub(crate) world: World,
    pub(crate) ticks: u32,
    pub(crate) start: u32,
    pub(crate) answers: Answers,
    pub(crate) fresh: BTreeSet<u16>,
    pub(crate) view_rotation: u8,
    pub(crate) answered: bool,
    pub(crate) house: u32,
    pub(crate) family: u16,
    pub(crate) loaded: [u8; 32],
    /// Format 3 on (view-rotated screen-centre tile pick from format 5).
    pub(crate) view_centre: Option<[i8; 2]>,
    /// Format 4 on: the saved house's opening camera rotation (`0..=3`)
    /// paired with its level-1 road-dash floor IDs (`0x4a87b1`–`0x4a87dc`).
    pub(crate) floor_rotation: u8,
    /// Format 5 on: frozen `(view_rotation, view_centre)` while a dialog tick
    /// awaits replay.
    pub(crate) replay_view: Option<(u8, Option<[i8; 2]>)>,
    /// Format 6 on: neighborhood `deletion_epoch` at the time this lot was
    /// saved (`0` for non-REDB saves).
    pub(crate) deletion_epoch: u64,
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
    /// load left otherwise: another neighborhood or house file, or a
    /// version of this program that loads the house differently. A change
    /// to the install's code or tuning alone is not seen.
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
                "the save is of a lot loaded otherwise (another neighborhood or house \
                 file, or a version that loads the house differently)"
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
        self.save_inner(None)
    }

    /// Like [`Lot::save`], but strips an active unpaid Buy Mode preview
    /// (`cursor.held` and `cursor.cursor`) from the saved world copy before
    /// purging detached Buy history so restoring the save never orphans an
    /// unpaid item or `"Tile Cursor"` and never double-deletes a preview
    /// still linked inside a detached surface container.
    pub fn save_without_buy_cursor(&self, cursor: &crate::BuyCursor) -> Result<Saved, SaveError> {
        self.save_inner(Some(cursor))
    }

    fn save_inner(&self, cursor: Option<&crate::BuyCursor>) -> Result<Saved, SaveError> {
        if self.load.finished.is_none() {
            return Err(SaveError::Incomplete);
        }
        if self.halted {
            return Err(SaveError::Halted);
        }
        let mut world = self.world.clone();
        let has_history =
            !self.buy_redo.is_empty() || !self.buy_undo.is_empty() || !self.buy_orphans.is_empty();
        if cursor.is_some() || has_history {
            let sounds = std::cell::RefCell::new(Vec::new());
            let interpreter = crate::interpreter::Interpreter {
                sounds: Some(&sounds),
                view_rotation: self.view_rotation,
                view_centre: self.view_centre,
                ..crate::interpreter::Interpreter::new(
                    &self.program,
                    &self.catalog,
                    crate::interpreter::Stream::Global,
                )
            };
            // Strip any unpaid Buy preview first so an item linked into an
            // undone surface container's slot is cleanly unlinked and deleted
            // before detached history objects are purged.
            if let Some(cursor) = cursor {
                interpreter.buy_cancel(&mut world, cursor).map_err(|e| {
                    SaveError::Format(format!("clearing Buy Mode preview for save: {e}"))
                })?;
            }
            if has_history {
                interpreter
                    .buy_purge_detached(&mut world, &self.buy_redo)
                    .and_then(|()| interpreter.buy_purge_detached(&mut world, &self.buy_undo))
                    .and_then(|()| interpreter.buy_purge_detached(&mut world, &self.buy_orphans))
                    .map_err(|e| {
                        SaveError::Format(format!("clearing detached Buy Mode redo objects: {e}"))
                    })?;
            }
        }
        Ok(Saved {
            world,
            ticks: self.ticks,
            start: self.start,
            answers: self.player.clone(),
            fresh: self.play.fresh.clone(),
            view_rotation: self.view_rotation,
            answered: self.answered,
            house: self.load.house,
            family: self.load.family,
            loaded: self.loaded,
            view_centre: self.view_centre,
            floor_rotation: self.floor_rotation,
            replay_view: self.replay_view,
            deletion_epoch: 0,
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
        if saved.loaded != self.loaded || saved.floor_rotation != self.floor_rotation {
            return Err(SaveError::OtherLoad);
        }
        self.world = saved.world;
        self.ticks = saved.ticks;
        self.start = saved.start;
        self.player = saved.answers;
        self.play.fresh = saved.fresh;
        self.view_rotation = saved.view_rotation;
        self.view_centre = saved.view_centre;
        self.replay_view = saved.replay_view;
        self.halted = false;
        self.asking = None;
        self.answered = saved.answered;
        self.buy_undo.clear();
        self.buy_redo.clear();
        self.buy_orphans.clear();
        // Restoring an external snapshot replaces `self.world` (including its
        // cached `families` and `neighbors`), so it is no longer a live
        // continuation of the store revision this `Lot` was opened from.
        self.neighborhood_origin = None;
        Ok(())
    }
}
