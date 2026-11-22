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
/// dialog answer's bitmap ID ([`crate::Answer`]); layout 3 kept the
/// pre-resolved house view centre; layout 4 keeps the house file's saved
/// floor rotation baseline (`HOUS 0` `camera_direction & 3`); layout 5
/// keeps the view-rotated screen-centre tile pick (`0x4a7b58` cut via
/// `0x433067`) and any frozen dialog-replay view; layout 6 keeps each
/// pending dialog answer's text-entry string ([`crate::Answer::text`]),
/// last kind 3 dialog text, Make New Character catalog names, and Save Card
/// lot name & description; layout 7 keeps choice-only follow-up answers
/// (`Answers::choices_after`) separately from explicit dialog replay queues
/// and records the neighborhood's `deletion_epoch` at save time so reusing a
/// deleted character GUID never revives stale instances on older saved lots;
/// layout 8 keeps the unencoded auxiliary portrait, outfit, lazy-index,
/// and scratch-pointer tables omitted from [`World::encode`].
pub const FORMAT: u32 = 8;

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
    /// Format 6 on: last kind 3 dialog text (`[0x6e4ac0]`), Make New Character
    /// catalog names (`0x44b0e8`), and Save Card lot name & description
    /// (`0x50e1f8..0x50e42c`).
    pub(crate) last_dialog_text: String,
    pub(crate) character_names: BTreeMap<u32, String>,
    pub(crate) lot_name: String,
    pub(crate) lot_description: String,
    /// Format 7 on: neighborhood `deletion_epoch` at the time this lot was
    /// saved (`0` for non-REDB saves).
    pub(crate) deletion_epoch: u64,
    /// Format 8 on: generated `BMP_` 2002 writebacks (`0x4e9000`),
    /// runtime-made character `"HEAD"` binding outcomes (`0x4e7824`),
    /// ambiguous `0x44b126` pet renders, runtime-loaded `.ndx` skeletons and
    /// suits (`0x5a0384`, `0x5a044d`), picked Make New Character outfits, live
    /// person attachment slots, and scratch destination pointer states, which
    /// [`World::encode`] omits to keep snapshot format 32's wire bytes
    /// unchanged.
    generated_portrait_2002: BTreeSet<PortraitFile>,
    made_portrait_head: BTreeMap<u32, Option<bool>>,
    ambiguous_portraits: BTreeSet<u32>,
    loaded_indexed_skeletons: BTreeMap<String, Option<String>>,
    loaded_indexed_suits: BTreeMap<String, Option<bool>>,
    loaded_indexed_suit_bones: BTreeMap<String, Vec<Vec<String>>>,
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

    /// Save Card lot name (`0x50e1f8..0x50e42c`).
    pub fn lot_name(&self) -> &str {
        &self.lot_name
    }

    /// Save Card lot description (`0x50e1f8..0x50e42c`).
    pub fn lot_description(&self) -> &str {
        &self.lot_description
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
        saved.world.generated_portrait_2002 = saved.generated_portrait_2002.clone();
        saved.world.made_portrait_head = saved.made_portrait_head.clone();
        saved.world.ambiguous_portraits = saved.ambiguous_portraits.clone();
        saved.world.loaded_indexed_skeletons = saved.loaded_indexed_skeletons.clone();
        saved.world.loaded_indexed_suits = saved.loaded_indexed_suits.clone();
        saved.world.loaded_indexed_suit_bones = saved.loaded_indexed_suit_bones.clone();
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
        let cursor = cursor
            .filter(|c| self.is_active_buy_cursor(c))
            .or_else(|| self.active_buy_cursor.as_ref().map(|a| &a.cursor));
        let mut world = self.world.clone();
        let has_history = !self.buy_redo.is_empty()
            || !self.buy_undo.is_empty()
            || !self.buy_orphans.is_empty()
            || !self.build_undo.is_empty()
            || !self.build_redo.is_empty();
        let has_wall_cursor = self.active_wall_cursor.is_some();
        if cursor.is_some() || has_history || has_wall_cursor {
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
            // Strip any active Buy preview (or snap back any held existing
            // furniture) first so a saved lot never serializes an unpaid item,
            // a `"Tile Cursor"`, or a held/hidden paid object stranded off-lot,
            // and never double-deletes a preview linked in a detached surface.
            if let Some(cursor) = cursor {
                let live = self.strip_detached_origin_children(cursor);
                interpreter.buy_cancel(&mut world, &live).map_err(|e| {
                    SaveError::Format(format!("clearing Buy Mode preview for save: {e}"))
                })?;
            }
            if let Some(wall_cur) = self.active_wall_cursor.as_ref() {
                interpreter
                    .build_cursor_close(&mut world, wall_cur)
                    .map_err(|e| {
                        SaveError::Format(format!("clearing Build Mode cursor for save: {e}"))
                    })?;
            }
            if has_history {
                let empty = std::collections::BTreeSet::new();
                let mut build_detached = Vec::new();
                for entry in self.build_undo.iter().chain(self.build_redo.iter()) {
                    build_detached.extend_from_slice(entry.detached_objects());
                }
                interpreter
                    .buy_purge_detached(&mut world, &self.buy_redo, &empty)
                    .and_then(|()| {
                        interpreter.buy_purge_detached(&mut world, &self.buy_undo, &empty)
                    })
                    .and_then(|()| {
                        interpreter.buy_purge_detached(&mut world, &self.buy_orphans, &empty)
                    })
                    .and_then(|()| {
                        interpreter.buy_purge_detached(&mut world, &build_detached, &empty)
                    })
                    .map_err(|e| {
                        SaveError::Format(format!("clearing detached Buy Mode redo objects: {e}"))
                    })?;
            }
        }
        let generated_portrait_2002 = world.generated_portrait_2002.clone();
        let made_portrait_head = world.made_portrait_head.clone();
        let ambiguous_portraits = world.ambiguous_portraits.clone();
        let loaded_indexed_skeletons = world.loaded_indexed_skeletons.clone();
        let loaded_indexed_suits = world.loaded_indexed_suits.clone();
        let loaded_indexed_suit_bones = world.loaded_indexed_suit_bones.clone();
        let made_outfits = world.made_outfits.clone();
        let person_outfits = world.person_outfits.clone();
        let scratch_ptr_set = world.scratch_ptr_set.clone();
        let scratch_ptr_set_to_next = world.scratch_ptr_set_to_next.clone();
        let scratch_ptr_expression = world.scratch_ptr_expression.clone();
        let scratch_ptr_arg_slot = world.scratch_ptr_arg_slot.clone();
        let runtime_added_suits = world.runtime_added_suits.clone();
        let suit_scan_cache = world.suit_scan_cache.clone();
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
            last_dialog_text: self.last_dialog_text.clone(),
            character_names: self.character_names.clone(),
            lot_name: self.lot_name.clone(),
            lot_description: self.lot_description.clone(),
            deletion_epoch: 0,
            generated_portrait_2002,
            made_portrait_head,
            ambiguous_portraits,
            loaded_indexed_skeletons,
            loaded_indexed_suits,
            loaded_indexed_suit_bones,
            made_outfits,
            person_outfits,
            scratch_ptr_set,
            scratch_ptr_set_to_next,
            scratch_ptr_expression,
            scratch_ptr_arg_slot,
            runtime_added_suits,
            suit_scan_cache,
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
        self.last_dialog_text = saved.last_dialog_text;
        self.character_names = saved.character_names;
        self.lot_name = saved.lot_name;
        self.lot_description = saved.lot_description;
        self.halted = false;
        self.asking = None;
        self.answered = saved.answered;
        self.buy_undo.clear();
        self.buy_redo.clear();
        self.buy_orphans.clear();
        self.active_buy_cursor = None;
        self.build_undo.clear();
        self.build_redo.clear();
        self.undo_order.clear();
        self.redo_order.clear();
        self.active_wall_cursor = None;
        // Restoring an external snapshot replaces `self.world` (including its
        // cached `families` and `neighbors`), so it is no longer a live
        // continuation of the store revision this `Lot` was opened from.
        self.neighborhood_origin = None;
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
        world
            .loaded_indexed_suit_bones
            .insert("suit_b".into(), vec![vec!["HEAD".into()]]);
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
            generated_portrait_2002: world.generated_portrait_2002.clone(),
            made_portrait_head: world.made_portrait_head.clone(),
            ambiguous_portraits: world.ambiguous_portraits.clone(),
            loaded_indexed_skeletons: world.loaded_indexed_skeletons.clone(),
            loaded_indexed_suits: world.loaded_indexed_suits.clone(),
            loaded_indexed_suit_bones: world.loaded_indexed_suit_bones.clone(),
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
            floor_rotation: 0,
            replay_view: None,
            last_dialog_text: String::new(),
            character_names: BTreeMap::new(),
            lot_name: String::new(),
            lot_description: String::new(),
            deletion_epoch: 0,
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
            decoded.world().loaded_indexed_suit_bones,
            BTreeMap::from([("suit_b".to_owned(), vec![vec!["HEAD".to_owned()]])])
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
