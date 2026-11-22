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
use sims_sim::{PersonOutfit, PortraitFile, World};
use std::collections::{BTreeMap, BTreeSet};

/// The version of [`Saved::encode`]'s layout: the fields of [`Saved`],
/// their order and their types (postcard is not self-describing). Bump it
/// in the commit that changes any of them. The world inside is versioned
/// by [`sims_sim::SNAPSHOT_FORMAT`], which is written too. A save of
/// another version is refused, not migrated. Layout 2 keeps each pending
/// dialog answer's bitmap ID ([`crate::Answer`]); layout 3 keeps the
/// house view's centre; layout 8 keeps [`World::dirty_portraits`],
/// [`World::generated_portrait_2002`] keyed by [`PortraitFile`],
/// [`World::made_portrait_head`], [`World::ambiguous_portraits`],
/// [`World::loaded_indexed_skeletons`], and [`World::made_outfits`]
/// (superseding pre-merge layouts 4–7).
pub const FORMAT: u32 = 8;

/// A lot as it stood between two steps ([`Lot::save`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Saved {
    world: World,
    ticks: u32,
    start: u32,
    answers: Answers,
    fresh: BTreeSet<u16>,
    view_rotation: u8,
    answered: bool,
    house: u32,
    family: u16,
    loaded: [u8; 32],
    /// Format 3 on.
    view_centre: Option<[i8; 2]>,
    /// Format 8 on: portrait dirty flags (`0x4e895e`), generated `BMP_` 2002
    /// writebacks (`0x4e9000`), runtime-made character `"HEAD"` binding
    /// outcomes (`0x4e7824`), ambiguous `0x44b126` pet renders, runtime-loaded
    /// `.ndx` skeletons (`0x5a0384`), and picked Make New Character outfits,
    /// which [`World::encode`] omits to keep snapshot format 31's wire bytes
    /// unchanged.
    dirty_portraits: BTreeSet<u32>,
    generated_portrait_2002: BTreeSet<PortraitFile>,
    made_portrait_head: BTreeMap<u32, Option<bool>>,
    ambiguous_portraits: BTreeSet<u32>,
    loaded_indexed_skeletons: BTreeMap<String, Option<String>>,
    loaded_indexed_suits: BTreeMap<String, Option<bool>>,
    made_outfits: BTreeMap<u32, BTreeMap<usize, String>>,
    person_outfits: BTreeMap<u16, PersonOutfit>,
    scratch_ptr_set: BTreeSet<u16>,
    scratch_ptr_set_to_next: BTreeSet<u16>,
    scratch_ptr_expression: BTreeSet<u16>,
    scratch_ptr_arg_slot: BTreeSet<u16>,
    runtime_added_suits: Vec<String>,
    suit_scan_cache: Option<Vec<String>>,
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
        let mut saved: Self = postcard::from_bytes(body).map_err(|e| format(e.to_string()))?;
        saved.world.dirty_portraits = saved.dirty_portraits.clone();
        saved.world.generated_portrait_2002 = saved.generated_portrait_2002.clone();
        saved.world.made_portrait_head = saved.made_portrait_head.clone();
        saved.world.ambiguous_portraits = saved.ambiguous_portraits.clone();
        saved.world.loaded_indexed_skeletons = saved.loaded_indexed_skeletons.clone();
        saved.world.loaded_indexed_suits = saved.loaded_indexed_suits.clone();
        saved.world.made_outfits = saved.made_outfits.clone();
        saved.world.person_outfits = saved.person_outfits.clone();
        saved.world.scratch_ptr_set = saved.scratch_ptr_set.clone();
        saved.world.scratch_ptr_set_to_next = saved.scratch_ptr_set_to_next.clone();
        saved.world.scratch_ptr_expression = saved.scratch_ptr_expression.clone();
        saved.world.scratch_ptr_arg_slot = saved.scratch_ptr_arg_slot.clone();
        saved.world.runtime_added_suits = saved.runtime_added_suits.clone();
        saved.world.suit_scan_cache = saved.suit_scan_cache.clone();
        Ok(saved)
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
            answers: self.player.clone(),
            fresh: self.play.fresh.clone(),
            view_rotation: self.view_rotation,
            answered: self.answered,
            house: self.load.house,
            family: self.load.family,
            loaded: self.loaded,
            view_centre: self.view_centre,
            dirty_portraits: self.world.dirty_portraits.clone(),
            generated_portrait_2002: self.world.generated_portrait_2002.clone(),
            made_portrait_head: self.world.made_portrait_head.clone(),
            ambiguous_portraits: self.world.ambiguous_portraits.clone(),
            loaded_indexed_skeletons: self.world.loaded_indexed_skeletons.clone(),
            loaded_indexed_suits: self.world.loaded_indexed_suits.clone(),
            made_outfits: self.world.made_outfits.clone(),
            person_outfits: self.world.person_outfits.clone(),
            scratch_ptr_set: self.world.scratch_ptr_set.clone(),
            scratch_ptr_set_to_next: self.world.scratch_ptr_set_to_next.clone(),
            scratch_ptr_expression: self.world.scratch_ptr_expression.clone(),
            scratch_ptr_arg_slot: self.world.scratch_ptr_arg_slot.clone(),
            runtime_added_suits: self.world.runtime_added_suits.clone(),
            suit_scan_cache: self.world.suit_scan_cache.clone(),
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
        self.player = saved.answers;
        self.play.fresh = saved.fresh;
        self.view_rotation = saved.view_rotation;
        self.view_centre = saved.view_centre;
        self.halted = false;
        self.asking = None;
        self.answered = saved.answered;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_round_trip_preserves_unencoded_portrait_sets() {
        let mut world = World::empty();
        world.dirty_portraits.insert(0x1111_2222);
        world
            .generated_portrait_2002
            .insert(PortraitFile::Resource(7));
        world.generated_portrait_2002.insert(PortraitFile::Made(3));
        world.made_portrait_head.insert(0x5555_6666, Some(true));
        world.ambiguous_portraits.insert(0x7777_8888);
        world
            .loaded_indexed_skeletons
            .insert("beta".into(), Some("alpha.bcf".into()));
        world
            .loaded_indexed_suits
            .insert("suit_b".into(), Some(true));
        world.made_outfits.insert(
            0x5555_6666,
            BTreeMap::from([
                (1, "b001,BODY=b001".into()),
                (2, "c001,HEAD-HEAD=c001".into()),
            ]),
        );
        world.scratch_ptr_set.insert(42);
        world.scratch_ptr_set_to_next.insert(43);
        world.scratch_ptr_expression.insert(42);
        world.scratch_ptr_arg_slot.insert(44);
        world.runtime_added_suits.push("late_suit".into());
        world.suit_scan_cache = Some(vec!["suit_a".into(), "late_suit".into()]);
        let saved = Saved {
            dirty_portraits: world.dirty_portraits.clone(),
            generated_portrait_2002: world.generated_portrait_2002.clone(),
            made_portrait_head: world.made_portrait_head.clone(),
            ambiguous_portraits: world.ambiguous_portraits.clone(),
            loaded_indexed_skeletons: world.loaded_indexed_skeletons.clone(),
            loaded_indexed_suits: world.loaded_indexed_suits.clone(),
            made_outfits: world.made_outfits.clone(),
            person_outfits: world.person_outfits.clone(),
            scratch_ptr_set: world.scratch_ptr_set.clone(),
            scratch_ptr_set_to_next: world.scratch_ptr_set_to_next.clone(),
            scratch_ptr_expression: world.scratch_ptr_expression.clone(),
            scratch_ptr_arg_slot: world.scratch_ptr_arg_slot.clone(),
            runtime_added_suits: world.runtime_added_suits.clone(),
            suit_scan_cache: world.suit_scan_cache.clone(),
            world,
            ticks: 1,
            start: 0,
            answers: Answers::default(),
            fresh: BTreeSet::new(),
            view_rotation: 0,
            answered: false,
            house: 5,
            family: 1,
            loaded: [0; 32],
            view_centre: Some([32, 32]),
        };
        let bytes = saved.encode();
        let decoded = Saved::decode(&bytes).unwrap();
        assert_eq!(
            decoded.world().dirty_portraits,
            BTreeSet::from([0x1111_2222])
        );
        assert_eq!(
            decoded.world().generated_portrait_2002,
            BTreeSet::from([PortraitFile::Resource(7), PortraitFile::Made(3)])
        );
        assert_eq!(
            decoded.world().made_portrait_head,
            BTreeMap::from([(0x5555_6666, Some(true))])
        );
        assert_eq!(
            decoded.world().ambiguous_portraits,
            BTreeSet::from([0x7777_8888])
        );
        assert_eq!(
            decoded.world().loaded_indexed_skeletons,
            BTreeMap::from([("beta".to_owned(), Some("alpha.bcf".to_owned()))])
        );
        assert_eq!(
            decoded.world().loaded_indexed_suits,
            BTreeMap::from([("suit_b".to_owned(), Some(true))])
        );
        assert_eq!(
            decoded.world().made_outfits,
            BTreeMap::from([(
                0x5555_6666,
                BTreeMap::from([
                    (1, "b001,BODY=b001".to_owned()),
                    (2, "c001,HEAD-HEAD=c001".to_owned()),
                ])
            )])
        );
        assert_eq!(decoded.world().scratch_ptr_set, BTreeSet::from([42]));
        assert_eq!(
            decoded.world().scratch_ptr_set_to_next,
            BTreeSet::from([43])
        );
        assert_eq!(decoded.world().scratch_ptr_expression, BTreeSet::from([42]));
        assert_eq!(decoded.world().scratch_ptr_arg_slot, BTreeSet::from([44]));
        assert_eq!(
            decoded.world().runtime_added_suits,
            vec!["late_suit".to_owned()]
        );
        assert_eq!(
            decoded.world().suit_scan_cache,
            Some(vec!["suit_a".to_owned(), "late_suit".to_owned()])
        );
    }
}
