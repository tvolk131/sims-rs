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
use crate::{Answer, Answers, SpecialPanelResponse, SpecialPanels};
use serde::{Deserialize, Serialize};
use sims_sim::{
    MigrationError, MigrationRegistry, MigrationReport, MigrationStep, PersonOutfit, PortraitFile,
    World, decode_postcard_exact,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Earliest [`Saved::encode`] layout version supported by the explicit forward
/// schema migration chain ([`SAVED_MIGRATIONS`]: `6 -> 7 -> 8 -> 9`).
pub const MIN_FORMAT: u32 = 6;

/// The version of [`Saved::encode`]'s layout: the fields of [`Saved`],
/// their order and their types (postcard is not self-describing). Bump it
/// in the commit that changes any of them. The world inside is versioned
/// by [`sims_sim::SNAPSHOT_FORMAT`], which is written too. Formats in
/// [`MIN_FORMAT`]..=[`FORMAT`] (`6..=9`) with matching [`sims_sim::SNAPSHOT_FORMAT`]
/// are upgraded deterministically through [`SAVED_MIGRATIONS`]; older (`< 6`),
/// future (`> 9`), or mismatched world-format saves are refused cleanly. Layout 2
/// keeps each pending dialog answer's bitmap ID ([`crate::Answer`]); layout 3
/// kept the pre-resolved house view centre; layout 4 keeps the house file's saved
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
/// and scratch-pointer tables omitted from [`World::encode`]; layout 9 keeps
/// registry special-panel slots ([`SpecialPanels`]) and queued
/// [`SpecialPanelResponse`]s for `Dialog` kinds `5..=15`.
pub const FORMAT: u32 = 9;

/// Historical `Answers` layout in [`Saved`] format 6 (before `choices_after`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnswersV6 {
    queue: VecDeque<Answer>,
    include_single_button: bool,
}

/// Historical [`Saved`] layout 6 (before `Answers::choices_after`,
/// `deletion_epoch`, and the format-8 auxiliary tables).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedV6 {
    world: World,
    ticks: u32,
    start: u32,
    answers: AnswersV6,
    fresh: BTreeSet<u16>,
    view_rotation: u8,
    answered: bool,
    house: u32,
    family: u16,
    loaded: [u8; 32],
    view_centre: Option<[i8; 2]>,
    floor_rotation: u8,
    replay_view: Option<(u8, Option<[i8; 2]>)>,
    last_dialog_text: String,
    character_names: BTreeMap<u32, String>,
    lot_name: String,
    lot_description: String,
}

/// Historical [`Saved`] layout 7 (added `Answers::choices_after` and
/// `deletion_epoch`, before the format-8 auxiliary tables).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedV7 {
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
    view_centre: Option<[i8; 2]>,
    floor_rotation: u8,
    replay_view: Option<(u8, Option<[i8; 2]>)>,
    last_dialog_text: String,
    character_names: BTreeMap<u32, String>,
    lot_name: String,
    lot_description: String,
    deletion_epoch: u64,
}

fn migrate_v6_to_v7(record: &str, body: &[u8]) -> Result<Vec<u8>, MigrationError> {
    let mut v6: SavedV6 = decode_postcard_exact(record, 6, body)?;
    let answers = if v6.answers.include_single_button {
        Answers::replay(v6.answers.queue.make_contiguous())
    } else {
        let mut a = Answers::default();
        a.queue = v6.answers.queue;
        a
    };
    let v7 = SavedV7 {
        world: v6.world,
        ticks: v6.ticks,
        start: v6.start,
        answers,
        fresh: v6.fresh,
        view_rotation: v6.view_rotation,
        answered: v6.answered,
        house: v6.house,
        family: v6.family,
        loaded: v6.loaded,
        view_centre: v6.view_centre,
        floor_rotation: v6.floor_rotation,
        replay_view: v6.replay_view,
        last_dialog_text: v6.last_dialog_text,
        character_names: v6.character_names,
        lot_name: v6.lot_name,
        lot_description: v6.lot_description,
        deletion_epoch: 0,
    };
    postcard::to_allocvec(&v7).map_err(|error| MigrationError::StepFailed {
        record: record.to_owned(),
        from: 6,
        to: 7,
        detail: error.to_string(),
    })
}

/// Historical [`Saved`] layout 8 (before `special_panels` and
/// `special_panel_responses`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedV8 {
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
    view_centre: Option<[i8; 2]>,
    floor_rotation: u8,
    replay_view: Option<(u8, Option<[i8; 2]>)>,
    last_dialog_text: String,
    character_names: BTreeMap<u32, String>,
    lot_name: String,
    lot_description: String,
    deletion_epoch: u64,
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

fn migrate_v7_to_v8(record: &str, body: &[u8]) -> Result<Vec<u8>, MigrationError> {
    let v7: SavedV7 = decode_postcard_exact(record, 7, body)?;
    let v8 = SavedV8 {
        world: v7.world,
        ticks: v7.ticks,
        start: v7.start,
        answers: v7.answers,
        fresh: v7.fresh,
        view_rotation: v7.view_rotation,
        answered: v7.answered,
        house: v7.house,
        family: v7.family,
        loaded: v7.loaded,
        view_centre: v7.view_centre,
        floor_rotation: v7.floor_rotation,
        replay_view: v7.replay_view,
        last_dialog_text: v7.last_dialog_text,
        character_names: v7.character_names,
        lot_name: v7.lot_name,
        lot_description: v7.lot_description,
        deletion_epoch: v7.deletion_epoch,
        generated_portrait_2002: BTreeSet::new(),
        made_portrait_head: BTreeMap::new(),
        ambiguous_portraits: BTreeSet::new(),
        loaded_indexed_skeletons: BTreeMap::new(),
        loaded_indexed_suits: BTreeMap::new(),
        loaded_indexed_suit_bones: BTreeMap::new(),
        made_outfits: BTreeMap::new(),
        person_outfits: BTreeMap::new(),
        scratch_ptr_set: BTreeSet::new(),
        scratch_ptr_set_to_next: BTreeSet::new(),
        scratch_ptr_expression: BTreeSet::new(),
        scratch_ptr_arg_slot: BTreeSet::new(),
        runtime_added_suits: Vec::new(),
        suit_scan_cache: None,
    };
    postcard::to_allocvec(&v8).map_err(|error| MigrationError::StepFailed {
        record: record.to_owned(),
        from: 7,
        to: 8,
        detail: error.to_string(),
    })
}

fn migrate_v8_to_v9(record: &str, body: &[u8]) -> Result<Vec<u8>, MigrationError> {
    let v8: SavedV8 = decode_postcard_exact(record, 8, body)?;
    let v9 = Saved {
        world: v8.world,
        ticks: v8.ticks,
        start: v8.start,
        answers: v8.answers,
        fresh: v8.fresh,
        view_rotation: v8.view_rotation,
        answered: v8.answered,
        house: v8.house,
        family: v8.family,
        loaded: v8.loaded,
        view_centre: v8.view_centre,
        floor_rotation: v8.floor_rotation,
        replay_view: v8.replay_view,
        last_dialog_text: v8.last_dialog_text,
        character_names: v8.character_names,
        lot_name: v8.lot_name,
        lot_description: v8.lot_description,
        deletion_epoch: v8.deletion_epoch,
        generated_portrait_2002: v8.generated_portrait_2002,
        made_portrait_head: v8.made_portrait_head,
        ambiguous_portraits: v8.ambiguous_portraits,
        loaded_indexed_skeletons: v8.loaded_indexed_skeletons,
        loaded_indexed_suits: v8.loaded_indexed_suits,
        loaded_indexed_suit_bones: v8.loaded_indexed_suit_bones,
        made_outfits: v8.made_outfits,
        person_outfits: v8.person_outfits,
        scratch_ptr_set: v8.scratch_ptr_set,
        scratch_ptr_set_to_next: v8.scratch_ptr_set_to_next,
        scratch_ptr_expression: v8.scratch_ptr_expression,
        scratch_ptr_arg_slot: v8.scratch_ptr_arg_slot,
        runtime_added_suits: v8.runtime_added_suits,
        suit_scan_cache: v8.suit_scan_cache,
        special_panels: SpecialPanels::default(),
        special_panel_responses: VecDeque::new(),
    };
    postcard::to_allocvec(&v9).map_err(|error| MigrationError::StepFailed {
        record: record.to_owned(),
        from: 8,
        to: 9,
        detail: error.to_string(),
    })
}

fn validate_saved_current(record: &str, body: &[u8]) -> Result<(), MigrationError> {
    let _: Saved = decode_postcard_exact(record, FORMAT, body)?;
    Ok(())
}

static SAVED_STEPS: &[MigrationStep] = &[
    MigrationStep {
        from: 6,
        to: 7,
        description: "add Answers::choices_after (None) and deletion_epoch (0)",
        apply: migrate_v6_to_v7,
    },
    MigrationStep {
        from: 7,
        to: 8,
        description: "initialize unencoded format-8 auxiliary portrait/outfit/index/scratch tables",
        apply: migrate_v7_to_v8,
    },
    MigrationStep {
        from: 8,
        to: 9,
        description: "add Dialog kinds 5..=15 SpecialPanels state and queued responses",
        apply: migrate_v8_to_v9,
    },
];

/// Registered forward schema migration chain for [`Saved`] (`v6 -> v7 -> v8 -> v9`).
///
/// Expects bytes framed with `[layout: u32 LE][postcard body]` (i.e. after
/// verifying the 4-byte `world_format == sims_sim::SNAPSHOT_FORMAT` header).
pub const SAVED_MIGRATIONS: MigrationRegistry = MigrationRegistry {
    min_version: MIN_FORMAT,
    current_version: FORMAT,
    steps: SAVED_STEPS,
    validate_current: validate_saved_current,
};

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
    /// Format 9 on: registry special-panel slots (`cObjectRegistry +0xe8..=+0x10c`)
    /// and queued special-panel responses for `Dialog` kinds `5..=15`.
    pub(crate) special_panels: SpecialPanels,
    pub(crate) special_panel_responses: VecDeque<SpecialPanelResponse>,
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
        let mut out = Vec::with_capacity(8 + 64 * 1024);
        out.extend_from_slice(&FORMAT.to_le_bytes());
        out.extend_from_slice(&sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
        postcard::serialize_with_flavor(self, postcard::ser_flavors::ExtendFlavor::new(out))
            .expect("encoding to memory cannot fail")
    }

    fn hydrate_world_aux(&mut self) {
        self.world.generated_portrait_2002 = self.generated_portrait_2002.clone();
        self.world.made_portrait_head = self.made_portrait_head.clone();
        self.world.ambiguous_portraits = self.ambiguous_portraits.clone();
        self.world.loaded_indexed_skeletons = self.loaded_indexed_skeletons.clone();
        self.world.loaded_indexed_suits = self.loaded_indexed_suits.clone();
        self.world.loaded_indexed_suit_bones = self.loaded_indexed_suit_bones.clone();
        self.world.made_outfits = self.made_outfits.clone();
        self.world.person_outfits = self.person_outfits.clone();
        self.world.scratch_ptrs = sims_sim::ScratchPtrs::from_btreesets(
            &self.scratch_ptr_set,
            &self.scratch_ptr_set_to_next,
            &self.scratch_ptr_expression,
            &self.scratch_ptr_arg_slot,
        );
        self.world.runtime_added_suits = self.runtime_added_suits.clone();
        self.world.suit_scan_cache = self.suit_scan_cache.clone();
        self.world.mark_all_frame_storage_dirty();
    }

    /// Reads [`Saved::encode`]'s bytes, deterministically migrating supported
    /// older layouts ([`MIN_FORMAT`]..=[`FORMAT`], i.e. `6 -> 7 -> 8 -> 9`) forward
    /// through [`SAVED_MIGRATIONS`]. Truncated payloads, trailing bytes,
    /// unsupported layout versions, and mismatched world formats are refused.
    pub fn decode(bytes: &[u8]) -> Result<Self, SaveError> {
        Self::decode_with_report(bytes).map(|(saved, _)| saved)
    }

    /// Reads [`Saved::encode`]'s bytes from any supported layout version
    /// ([`MIN_FORMAT`]..=[`FORMAT`]) and returns both the decoded [`Saved`]
    /// and its [`MigrationReport`].
    pub fn decode_with_report(bytes: &[u8]) -> Result<(Self, MigrationReport), SaveError> {
        let format = SaveError::Format;
        let (header, body) = bytes
            .split_first_chunk::<8>()
            .ok_or_else(|| format("shorter than its header".into()))?;
        let layout = u32::from_le_bytes(header[..4].try_into().unwrap());
        let world = u32::from_le_bytes(header[4..].try_into().unwrap());
        if world != sims_sim::SNAPSHOT_FORMAT {
            return Err(format(format!(
                "save format {layout} with world format {world}, expected {MIN_FORMAT}..={FORMAT} with {}",
                sims_sim::SNAPSHOT_FORMAT
            )));
        }

        let (mut saved, report) = if layout == FORMAT {
            let decoded: Self = decode_postcard_exact("saved-lot", FORMAT, body)
                .map_err(|e| format(e.to_string()))?;
            (
                decoded,
                MigrationReport {
                    original_version: FORMAT,
                    final_version: FORMAT,
                    steps_applied: 0,
                },
            )
        } else {
            let mut versioned = Vec::with_capacity(4 + body.len());
            versioned.extend_from_slice(&layout.to_le_bytes());
            versioned.extend_from_slice(body);
            let migrated = SAVED_MIGRATIONS
                .migrate_bytes("saved-lot", &versioned)
                .map_err(|error| match error {
                    MigrationError::UnsupportedVersion {
                        found,
                        min_supported,
                        max_supported,
                        ..
                    } => format(format!(
                        "save format {found} with world format {world}, supported \
                         {min_supported}..={max_supported} with {}",
                        sims_sim::SNAPSHOT_FORMAT
                    )),
                    other => format(other.to_string()),
                })?;
            let (_, migrated_body) = migrated
                .encoded
                .split_first_chunk::<4>()
                .expect("migrated bytes always carry 4-byte header");
            let decoded: Self =
                decode_postcard_exact("saved-lot", migrated.final_version, migrated_body)
                    .map_err(|e| format(e.to_string()))?;
            (
                decoded,
                MigrationReport {
                    original_version: migrated.original_version,
                    final_version: migrated.final_version,
                    steps_applied: migrated.steps_applied,
                },
            )
        };
        validate_world_object_ids(&saved.world)?;
        saved.hydrate_world_aux();
        Ok((saved, report))
    }

    /// Reads [`Saved::encode`]'s bytes strictly at [`FORMAT`] without running
    /// older-version forward migrations, still rejecting truncated payloads and
    /// trailing bytes.
    pub fn decode_strict(bytes: &[u8]) -> Result<Self, SaveError> {
        let format = SaveError::Format;
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
        let mut saved: Self =
            decode_postcard_exact("saved-lot", FORMAT, body).map_err(|e| format(e.to_string()))?;
        validate_world_object_ids(&saved.world)?;
        saved.hydrate_world_aux();
        Ok(saved)
    }
}

/// Rejects saves whose world contains the reserved sentinel object ID
/// (`u16::MAX` / `65535`), which native signed-`i16` object registries never
/// emit and `sims_render::Canvas` reserves as `ID_NONE`.
fn validate_world_object_ids(world: &World) -> Result<(), SaveError> {
    if world.objects.contains_key(&u16::MAX)
        || world.objects.values().any(|o| o.id == u16::MAX)
        || world.people.contains(&u16::MAX)
        || world.selected() == Some(u16::MAX)
    {
        return Err(SaveError::Format(
            "saved world contains reserved object ID 65535".into(),
        ));
    }
    Ok(())
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
        let scratch_ptr_set = world.scratch_ptrs.to_btreeset(sims_sim::SCRATCH_PTR_SET);
        let scratch_ptr_set_to_next = world
            .scratch_ptrs
            .to_btreeset(sims_sim::SCRATCH_PTR_SET_TO_NEXT);
        let scratch_ptr_expression = world
            .scratch_ptrs
            .to_btreeset(sims_sim::SCRATCH_PTR_EXPRESSION);
        let scratch_ptr_arg_slot = world
            .scratch_ptrs
            .to_btreeset(sims_sim::SCRATCH_PTR_ARG_SLOT);
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
            special_panels: self.special_panels.clone(),
            special_panel_responses: self.special_panel_responses.clone(),
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
        validate_world_object_ids(&saved.world)?;
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
        self.asking_special_panel = None;
        self.special_panels = saved.special_panels;
        self.special_panel_responses = saved.special_panel_responses;
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
        self.coop = None;
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
        world.scratch_ptrs.set(
            42,
            sims_sim::SCRATCH_PTR_SET | sims_sim::SCRATCH_PTR_EXPRESSION,
        );
        world
            .scratch_ptrs
            .set(43, sims_sim::SCRATCH_PTR_SET_TO_NEXT);
        world.scratch_ptrs.set(44, sims_sim::SCRATCH_PTR_ARG_SLOT);
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
            scratch_ptr_set: world.scratch_ptrs.to_btreeset(sims_sim::SCRATCH_PTR_SET),
            scratch_ptr_set_to_next: world
                .scratch_ptrs
                .to_btreeset(sims_sim::SCRATCH_PTR_SET_TO_NEXT),
            scratch_ptr_expression: world
                .scratch_ptrs
                .to_btreeset(sims_sim::SCRATCH_PTR_EXPRESSION),
            scratch_ptr_arg_slot: world
                .scratch_ptrs
                .to_btreeset(sims_sim::SCRATCH_PTR_ARG_SLOT),
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
            special_panels: SpecialPanels::default(),
            special_panel_responses: VecDeque::new(),
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
        assert_eq!(
            decoded
                .world()
                .scratch_ptrs
                .to_btreeset(sims_sim::SCRATCH_PTR_SET),
            BTreeSet::from([42])
        );
        assert_eq!(
            decoded
                .world()
                .scratch_ptrs
                .to_btreeset(sims_sim::SCRATCH_PTR_SET_TO_NEXT),
            BTreeSet::from([43])
        );
        assert_eq!(
            decoded
                .world()
                .scratch_ptrs
                .to_btreeset(sims_sim::SCRATCH_PTR_EXPRESSION),
            BTreeSet::from([42])
        );
        assert_eq!(
            decoded
                .world()
                .scratch_ptrs
                .to_btreeset(sims_sim::SCRATCH_PTR_ARG_SLOT),
            BTreeSet::from([44])
        );
        assert_eq!(
            decoded.world().runtime_added_suits,
            vec!["late_suit".to_owned()]
        );
        assert_eq!(
            decoded.world().suit_scan_cache,
            Some(vec!["suit_a".to_owned(), "late_suit".to_owned()])
        );
    }

    fn encode_v6(v6: &SavedV6) -> Vec<u8> {
        let mut out = 6u32.to_le_bytes().to_vec();
        out.extend(sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
        out.extend(postcard::to_allocvec(v6).unwrap());
        out
    }

    fn encode_v7(v7: &SavedV7) -> Vec<u8> {
        let mut out = 7u32.to_le_bytes().to_vec();
        out.extend(sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
        out.extend(postcard::to_allocvec(v7).unwrap());
        out
    }

    #[test]
    fn saved_forward_migrations_v6_and_v7_to_v8_preserve_state_and_reject_corruption() {
        let mut v6_replay_queue = VecDeque::new();
        v6_replay_queue.push_back(Answer {
            status: 3,
            bitmap_id: Some(42),
            text: Some("Goth Manor".into()),
        });
        let v6_replay = SavedV6 {
            world: World::empty(),
            ticks: 123,
            start: 10,
            answers: AnswersV6 {
                queue: v6_replay_queue.clone(),
                include_single_button: true,
            },
            fresh: BTreeSet::from([7, 9]),
            view_rotation: 2,
            answered: true,
            house: 5,
            family: 1,
            loaded: [0x5a; 32],
            view_centre: Some([12, 34]),
            floor_rotation: 1,
            replay_view: Some((2, Some([12, 34]))),
            last_dialog_text: "Hello!".into(),
            character_names: BTreeMap::from([(0x1111_2222, "Mortimer".into())]),
            lot_name: "5 Sim Lane".into(),
            lot_description: "Spooky house".into(),
        };
        let bytes_v6_replay = encode_v6(&v6_replay);

        // Strict decode refuses format 6.
        assert!(matches!(
            Saved::decode_strict(&bytes_v6_replay),
            Err(SaveError::Format(_))
        ));

        // Migrating decode upgrades 6 -> 7 -> 8 -> 9 deterministically.
        let (migrated_v6, report_v6) = Saved::decode_with_report(&bytes_v6_replay).unwrap();
        assert_eq!(report_v6.original_version, 6);
        assert_eq!(report_v6.final_version, FORMAT);
        assert_eq!(report_v6.steps_applied, 3);
        assert!(report_v6.was_migrated());
        assert_eq!(migrated_v6.ticks, 123);
        assert_eq!(migrated_v6.start, 10);
        assert_eq!(
            migrated_v6.answers,
            Answers::replay(&[Answer {
                status: 3,
                bitmap_id: Some(42),
                text: Some("Goth Manor".into()),
            }])
        );
        assert_eq!(migrated_v6.fresh, BTreeSet::from([7, 9]));
        assert_eq!(migrated_v6.view_rotation, 2);
        assert!(migrated_v6.answered);
        assert_eq!(migrated_v6.house, 5);
        assert_eq!(migrated_v6.family, 1);
        assert_eq!(migrated_v6.loaded, [0x5a; 32]);
        assert_eq!(migrated_v6.view_centre, Some([12, 34]));
        assert_eq!(migrated_v6.floor_rotation, 1);
        assert_eq!(migrated_v6.replay_view, Some((2, Some([12, 34]))));
        assert_eq!(migrated_v6.last_dialog_text, "Hello!");
        assert_eq!(
            migrated_v6
                .character_names
                .get(&0x1111_2222)
                .map(String::as_str),
            Some("Mortimer")
        );
        assert_eq!(migrated_v6.lot_name(), "5 Sim Lane");
        assert_eq!(migrated_v6.lot_description(), "Spooky house");
        assert_eq!(migrated_v6.deletion_epoch, 0);

        // Also test v6 with include_single_button = false (choices mode).
        let mut v6_choices = v6_replay;
        v6_choices.answers.include_single_button = false;
        let bytes_v6_choices = encode_v6(&v6_choices);
        let (migrated_choices, _) = Saved::decode_with_report(&bytes_v6_choices).unwrap();
        let mut expected_choices = Answers::default();
        expected_choices.queue = v6_replay_queue;
        assert_eq!(migrated_choices.answers, expected_choices);

        // Test v7 -> v8 preserving `choices_after` and non-zero `deletion_epoch`.
        let mut v7_answers = Answers::replay(&[Answer::from(2)]);
        v7_answers.extend_choices_after(&[3, 4]);
        let v7 = SavedV7 {
            world: World::empty(),
            ticks: 456,
            start: 12,
            answers: v7_answers.clone(),
            fresh: BTreeSet::from([3]),
            view_rotation: 1,
            answered: false,
            house: 7,
            family: 2,
            loaded: [0xaa; 32],
            view_centre: None,
            floor_rotation: 3,
            replay_view: None,
            last_dialog_text: String::new(),
            character_names: BTreeMap::new(),
            lot_name: "7 Sim Lane".into(),
            lot_description: "Newbie home".into(),
            deletion_epoch: 19,
        };
        let bytes_v7 = encode_v7(&v7);
        assert!(matches!(
            Saved::decode_strict(&bytes_v7),
            Err(SaveError::Format(_))
        ));
        let (migrated_v7, report_v7) = Saved::decode_with_report(&bytes_v7).unwrap();
        assert_eq!(report_v7.original_version, 7);
        assert_eq!(report_v7.final_version, FORMAT);
        assert_eq!(report_v7.steps_applied, 2);
        assert_eq!(migrated_v7.answers, v7_answers);
        assert_eq!(migrated_v7.deletion_epoch, 19);
        assert_eq!(migrated_v7.lot_name(), "7 Sim Lane");

        // Re-encoding a migrated save emits canonical format 8 accepted by `decode_strict`.
        let canonical_v8 = migrated_v7.encode();
        let (redecoded, report_v8) = Saved::decode_with_report(&canonical_v8).unwrap();
        assert_eq!(report_v8.original_version, FORMAT);
        assert_eq!(report_v8.steps_applied, 0);
        assert_eq!(redecoded.deletion_epoch, 19);
        assert!(Saved::decode_strict(&canonical_v8).is_ok());

        // Reject trailing bytes and truncated payloads across v6, v7, and v8.
        for valid in [&bytes_v6_replay, &bytes_v7, &canonical_v8] {
            let mut with_trailing = valid.clone();
            with_trailing.push(0x00);
            assert!(
                matches!(Saved::decode(&with_trailing), Err(SaveError::Format(_))),
                "trailing byte must be rejected"
            );
            let truncated = &valid[..valid.len() - 1];
            assert!(
                matches!(Saved::decode(truncated), Err(SaveError::Format(_))),
                "truncated body must be rejected"
            );
        }

        // Direct SAVED_MIGRATIONS check on v6 -> v9, v9 -> v9, v5, v10.
        let mut raw_v6 = 6u32.to_le_bytes().to_vec();
        raw_v6.extend_from_slice(&bytes_v6_replay[8..]);
        let direct_v6 = SAVED_MIGRATIONS.migrate_bytes("test", &raw_v6).unwrap();
        assert_eq!(direct_v6.original_version, 6);
        assert_eq!(direct_v6.final_version, FORMAT);
        assert_eq!(direct_v6.steps_applied, 3);

        let mut raw_v8 = FORMAT.to_le_bytes().to_vec();
        raw_v8.extend_from_slice(&canonical_v8[8..]);
        let direct_v8 = SAVED_MIGRATIONS.migrate_bytes("test", &raw_v8).unwrap();
        assert_eq!(direct_v8.steps_applied, 0);
        assert_eq!(&direct_v8.encoded[4..], &canonical_v8[8..]);

        assert!(matches!(
            SAVED_MIGRATIONS.migrate_bytes("test", &5u32.to_le_bytes()),
            Err(MigrationError::UnsupportedVersion { found: 5, .. })
        ));
        assert!(matches!(
            SAVED_MIGRATIONS.migrate_bytes("test", &(FORMAT + 1).to_le_bytes()),
            Err(MigrationError::UnsupportedVersion { found, .. }) if found == FORMAT + 1
        ));
    }

    #[test]
    fn goth_house_migrated_from_v6_and_v7_steps_identically_to_live_lot() {
        let Some(root) = std::env::var_os("SIMS_INSTALL") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let house = root.join("UserData/Houses/House05.iff");
        let mut live = Lot::load(&root, &house, &[3], super::super::Frames::default()).unwrap();
        for _ in 0..500 {
            live.step().unwrap();
        }
        let saved_v8 = live.save().unwrap();

        // Build authentic Format 6 and Format 7 byte representations of tick 500.
        let v6_bytes = encode_v6(&SavedV6 {
            world: saved_v8.world.clone(),
            ticks: saved_v8.ticks,
            start: saved_v8.start,
            answers: AnswersV6 {
                queue: saved_v8.answers.queue.clone(),
                include_single_button: false,
            },
            fresh: saved_v8.fresh.clone(),
            view_rotation: saved_v8.view_rotation,
            answered: saved_v8.answered,
            house: saved_v8.house,
            family: saved_v8.family,
            loaded: saved_v8.loaded,
            view_centre: saved_v8.view_centre,
            floor_rotation: saved_v8.floor_rotation,
            replay_view: saved_v8.replay_view,
            last_dialog_text: saved_v8.last_dialog_text.clone(),
            character_names: saved_v8.character_names.clone(),
            lot_name: saved_v8.lot_name.clone(),
            lot_description: saved_v8.lot_description.clone(),
        });
        let v7_bytes = encode_v7(&SavedV7 {
            world: saved_v8.world.clone(),
            ticks: saved_v8.ticks,
            start: saved_v8.start,
            answers: saved_v8.answers.clone(),
            fresh: saved_v8.fresh.clone(),
            view_rotation: saved_v8.view_rotation,
            answered: saved_v8.answered,
            house: saved_v8.house,
            family: saved_v8.family,
            loaded: saved_v8.loaded,
            view_centre: saved_v8.view_centre,
            floor_rotation: saved_v8.floor_rotation,
            replay_view: saved_v8.replay_view,
            last_dialog_text: saved_v8.last_dialog_text.clone(),
            character_names: saved_v8.character_names.clone(),
            lot_name: saved_v8.lot_name.clone(),
            lot_description: saved_v8.lot_description.clone(),
            deletion_epoch: saved_v8.deletion_epoch,
        });

        let (dec_v6, rep_v6) = Saved::decode_with_report(&v6_bytes).unwrap();
        let (dec_v7, rep_v7) = Saved::decode_with_report(&v7_bytes).unwrap();
        assert_eq!(rep_v6.steps_applied, 3);
        assert_eq!(rep_v7.steps_applied, 2);

        let mut from_v6 = Lot::load(&root, &house, &[], super::super::Frames::default()).unwrap();
        let mut from_v7 = Lot::load(&root, &house, &[], super::super::Frames::default()).unwrap();
        from_v6.restore(dec_v6).unwrap();
        from_v7.restore(dec_v7).unwrap();

        for tick in 501..=700 {
            live.step().unwrap();
            from_v6.step().unwrap();
            from_v7.step().unwrap();
            assert_eq!(
                from_v6.world().hash(),
                live.world().hash(),
                "v6 tick {tick}"
            );
            assert_eq!(
                from_v7.world().hash(),
                live.world().hash(),
                "v7 tick {tick}"
            );
        }
    }

    /// Independent raw-postcard wire-byte golden test derived directly from the
    /// historical field declarations of commits `2ddea688` (FORMAT 6) and
    /// `50106efb` (FORMAT 7), without using `SavedV6` or `SavedV7` to build the
    /// input bytes. Catches any field-order or type drift between the historical
    /// commits and our migration structs.
    #[test]
    fn golden_raw_wire_bytes_from_2ddea688_and_50106efb_migrate_losslessly() {
        fn push_varint_u64(out: &mut Vec<u8>, mut v: u64) {
            loop {
                let mut byte = (v & 0x7f) as u8;
                v >>= 7;
                if v != 0 {
                    byte |= 0x80;
                }
                out.push(byte);
                if v == 0 {
                    break;
                }
            }
        }
        fn push_zigzag_i16(out: &mut Vec<u8>, v: i16) {
            let zz = ((v << 1) ^ (v >> 15)) as u16;
            push_varint_u64(out, u64::from(zz));
        }
        fn push_str(out: &mut Vec<u8>, s: &str) {
            push_varint_u64(out, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
        }

        // `World::encode()` is the canonical `postcard` encoding of `World` at `SNAPSHOT_FORMAT`.
        let world_postcard = World::empty().encode();
        let loaded_fingerprint: [u8; 32] =
            std::array::from_fn(|i| (i as u8).wrapping_mul(7) ^ 0xa5);

        // --- Golden FORMAT 6 bytes (commit `2ddea688`) ---
        assert_eq!(sims_sim::SNAPSHOT_FORMAT, 32);
        let mut raw_2ddea688 = Vec::new();
        raw_2ddea688.extend_from_slice(&6u32.to_le_bytes()); // FORMAT = 6
        raw_2ddea688.extend_from_slice(&32u32.to_le_bytes()); // SNAPSHOT_FORMAT = 32
        raw_2ddea688.extend_from_slice(&world_postcard); // 1. world: World
        push_varint_u64(&mut raw_2ddea688, 777); // 2. ticks: u32
        push_varint_u64(&mut raw_2ddea688, 33); // 3. start: u32
        // 4. answers: Answers (2ddea688: { queue: VecDeque<Answer>, include_single_button: bool })
        push_varint_u64(&mut raw_2ddea688, 1); // queue.len = 1
        raw_2ddea688.push(3); // Answer.status = 3
        raw_2ddea688.push(1); // Answer.bitmap_id = Some(-17)
        push_zigzag_i16(&mut raw_2ddea688, -17);
        raw_2ddea688.push(1); // Answer.text = Some("Bella")
        push_str(&mut raw_2ddea688, "Bella");
        raw_2ddea688.push(1); // include_single_button = true
        // 5. fresh: BTreeSet<u16> = {4, 19}
        push_varint_u64(&mut raw_2ddea688, 2);
        push_varint_u64(&mut raw_2ddea688, 4);
        push_varint_u64(&mut raw_2ddea688, 19);
        raw_2ddea688.push(3); // 6. view_rotation: u8
        raw_2ddea688.push(1); // 7. answered: bool = true
        push_varint_u64(&mut raw_2ddea688, 5); // 8. house: u32
        push_varint_u64(&mut raw_2ddea688, 1); // 9. family: u16
        raw_2ddea688.extend_from_slice(&loaded_fingerprint); // 10. loaded: [u8; 32]
        raw_2ddea688.extend_from_slice(&[1, 21, 42]); // 11. view_centre: Some([21, 42])
        raw_2ddea688.push(2); // 12. floor_rotation: u8
        raw_2ddea688.extend_from_slice(&[1, 3, 1, 21, 42]); // 13. replay_view: Some((3, Some([21, 42])))
        push_str(&mut raw_2ddea688, "Last prompt text"); // 14. last_dialog_text
        push_varint_u64(&mut raw_2ddea688, 1); // 15. character_names len = 1
        push_varint_u64(&mut raw_2ddea688, 0x1234_5678);
        push_str(&mut raw_2ddea688, "Cassandra");
        push_str(&mut raw_2ddea688, "5 Sim Lane"); // 16. lot_name
        push_str(&mut raw_2ddea688, "Historic Goth estate"); // 17. lot_description

        let (dec6, rep6) = Saved::decode_with_report(&raw_2ddea688).unwrap();
        assert_eq!(rep6.original_version, 6);
        assert_eq!(rep6.final_version, FORMAT);
        assert_eq!(rep6.steps_applied, 3);
        assert_eq!(dec6.ticks, 777);
        assert_eq!(dec6.start, 33);
        assert_eq!(
            dec6.answers,
            Answers::replay(&[Answer {
                status: 3,
                bitmap_id: Some(-17),
                text: Some("Bella".into()),
            }])
        );
        assert_eq!(dec6.fresh, BTreeSet::from([4, 19]));
        assert_eq!(dec6.view_rotation, 3);
        assert!(dec6.answered);
        assert_eq!(dec6.house, 5);
        assert_eq!(dec6.family, 1);
        assert_eq!(dec6.loaded, loaded_fingerprint);
        assert_eq!(dec6.view_centre, Some([21, 42]));
        assert_eq!(dec6.floor_rotation, 2);
        assert_eq!(dec6.replay_view, Some((3, Some([21, 42]))));
        assert_eq!(dec6.last_dialog_text, "Last prompt text");
        assert_eq!(
            dec6.character_names.get(&0x1234_5678).map(String::as_str),
            Some("Cassandra")
        );
        assert_eq!(dec6.lot_name(), "5 Sim Lane");
        assert_eq!(dec6.lot_description(), "Historic Goth estate");
        assert_eq!(dec6.deletion_epoch, 0);

        // --- Golden FORMAT 7 bytes (commit `50106efb`) ---
        let mut raw_50106efb = Vec::new();
        raw_50106efb.extend_from_slice(&7u32.to_le_bytes()); // FORMAT = 7
        raw_50106efb.extend_from_slice(&32u32.to_le_bytes()); // SNAPSHOT_FORMAT = 32
        raw_50106efb.extend_from_slice(&world_postcard); // 1. world: World
        push_varint_u64(&mut raw_50106efb, 888); // 2. ticks: u32
        push_varint_u64(&mut raw_50106efb, 33); // 3. start: u32
        // 4. answers: Answers (50106efb: adds `choices_after: Option<VecDeque<Answer>>`)
        push_varint_u64(&mut raw_50106efb, 1); // queue.len = 1
        raw_50106efb.push(2); // Answer.status = 2
        raw_50106efb.push(0); // bitmap_id = None
        raw_50106efb.push(0); // text = None
        raw_50106efb.push(1); // include_single_button = true
        raw_50106efb.push(1); // choices_after = Some(...)
        push_varint_u64(&mut raw_50106efb, 2); // 2 follow-up choices
        raw_50106efb.extend_from_slice(&[3, 0, 0]);
        raw_50106efb.extend_from_slice(&[4, 0, 0]);
        push_varint_u64(&mut raw_50106efb, 1); // 5. fresh = {11}
        push_varint_u64(&mut raw_50106efb, 11);
        raw_50106efb.push(1); // 6. view_rotation
        raw_50106efb.push(0); // 7. answered = false
        push_varint_u64(&mut raw_50106efb, 7); // 8. house
        push_varint_u64(&mut raw_50106efb, 2); // 9. family
        raw_50106efb.extend_from_slice(&loaded_fingerprint); // 10. loaded
        raw_50106efb.push(0); // 11. view_centre = None
        raw_50106efb.push(0); // 12. floor_rotation
        raw_50106efb.push(0); // 13. replay_view = None
        push_str(&mut raw_50106efb, "Dialog v7"); // 14. last_dialog_text
        push_varint_u64(&mut raw_50106efb, 0); // 15. character_names = {}
        push_str(&mut raw_50106efb, "7 Sim Lane"); // 16. lot_name
        push_str(&mut raw_50106efb, "Newbie house"); // 17. lot_description
        push_varint_u64(&mut raw_50106efb, 42); // 18. deletion_epoch = 42

        let (dec7, rep7) = Saved::decode_with_report(&raw_50106efb).unwrap();
        assert_eq!(rep7.original_version, 7);
        assert_eq!(rep7.final_version, FORMAT);
        assert_eq!(rep7.steps_applied, 2);
        let mut expected_ans7 = Answers::replay(&[Answer::from(2)]);
        expected_ans7.extend_choices_after(&[3, 4]);
        assert_eq!(dec7.ticks, 888);
        assert_eq!(dec7.answers, expected_ans7);
        assert_eq!(dec7.fresh, BTreeSet::from([11]));
        assert_eq!(dec7.house, 7);
        assert_eq!(dec7.family, 2);
        assert_eq!(dec7.loaded, loaded_fingerprint);
        assert_eq!(dec7.last_dialog_text, "Dialog v7");
        assert_eq!(dec7.lot_name(), "7 Sim Lane");
        assert_eq!(dec7.lot_description(), "Newbie house");
        assert_eq!(dec7.deletion_epoch, 42);

        // --- Format 8 -> 9 migration + v9 SpecialPanels lossless roundtrip ---
        let mut raw_v8 = Vec::new();
        raw_v8.extend_from_slice(&8u32.to_le_bytes());
        raw_v8.extend_from_slice(&sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
        let v8 = SavedV8 {
            world: dec7.world.clone(),
            ticks: 999,
            start: 33,
            answers: Answers::replay(&[Answer::from(3)]),
            fresh: BTreeSet::from([7]),
            view_rotation: 2,
            answered: false,
            house: 5,
            family: 1,
            loaded: loaded_fingerprint,
            view_centre: Some([12, 34]),
            floor_rotation: 1,
            replay_view: None,
            last_dialog_text: "v8 text".into(),
            character_names: BTreeMap::new(),
            lot_name: "5 Sim Lane".into(),
            lot_description: "Goth v8".into(),
            deletion_epoch: 77,
            generated_portrait_2002: BTreeSet::new(),
            made_portrait_head: BTreeMap::new(),
            ambiguous_portraits: BTreeSet::new(),
            loaded_indexed_skeletons: BTreeMap::new(),
            loaded_indexed_suits: BTreeMap::new(),
            loaded_indexed_suit_bones: BTreeMap::new(),
            made_outfits: BTreeMap::new(),
            person_outfits: BTreeMap::new(),
            scratch_ptr_set: BTreeSet::new(),
            scratch_ptr_set_to_next: BTreeSet::new(),
            scratch_ptr_expression: BTreeSet::new(),
            scratch_ptr_arg_slot: BTreeSet::new(),
            runtime_added_suits: Vec::new(),
            suit_scan_cache: None,
        };
        raw_v8.extend_from_slice(&postcard::to_allocvec(&v8).unwrap());
        let (mut dec8, rep8) = Saved::decode_with_report(&raw_v8).unwrap();
        assert_eq!(rep8.original_version, 8);
        assert_eq!(rep8.final_version, FORMAT);
        assert_eq!(rep8.steps_applied, 1);
        assert_eq!(dec8.ticks, 999);
        assert_eq!(dec8.deletion_epoch, 77);
        assert_eq!(dec8.special_panels, SpecialPanels::default());
        assert!(dec8.special_panel_responses.is_empty());

        // Populated v9 special panels & queued responses roundtrip losslessly.
        dec8.special_panels.magic_lot = 93;
        dec8.special_panels.wardrobe_index = 2;
        dec8.special_panels.studio_caller = Some(21);
        dec8.special_panel_responses
            .push_back(SpecialPanelResponse::Lot(Some(93)));
        let v9_bytes = dec8.encode();
        let (dec9, rep9) = Saved::decode_with_report(&v9_bytes).unwrap();
        assert_eq!(rep9.original_version, FORMAT);
        assert_eq!(rep9.steps_applied, 0);
        assert_eq!(dec9.special_panels.magic_lot, 93);
        assert_eq!(dec9.special_panels.wardrobe_index, 2);
        assert_eq!(dec9.special_panels.studio_caller, Some(21));
        assert_eq!(
            dec9.special_panel_responses,
            VecDeque::from([SpecialPanelResponse::Lot(Some(93))])
        );
    }

    #[test]
    fn decode_rejects_reserved_u16_max_object_id() {
        let mut world = World::empty();
        world.objects.insert(
            u16::MAX,
            sims_sim::Object {
                id: u16::MAX,
                guid: 0x1234_5678,
                footprint: [0; 4],
                position: [0; 3],
                unknown_short: 0,
                attributes: Vec::new(),
                temps: Vec::new(),
                data: Vec::new(),
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: Vec::new(),
                relationship_flag: 0,
                relationships: Vec::new(),
                slots: Vec::new(),
                dynamic_sprites: Vec::new(),
                multipart: None,
                portal: None,
                person: None,
                registry_word: None,
                last_result: false,
                removal_skips_resets: false,
                selected: false,
            },
        );
        let bad = Saved {
            generated_portrait_2002: BTreeSet::new(),
            made_portrait_head: BTreeMap::new(),
            ambiguous_portraits: BTreeSet::new(),
            loaded_indexed_skeletons: BTreeMap::new(),
            loaded_indexed_suits: BTreeMap::new(),
            loaded_indexed_suit_bones: BTreeMap::new(),
            made_outfits: BTreeMap::new(),
            person_outfits: BTreeMap::new(),
            scratch_ptr_set: BTreeSet::new(),
            scratch_ptr_set_to_next: BTreeSet::new(),
            scratch_ptr_expression: BTreeSet::new(),
            scratch_ptr_arg_slot: BTreeSet::new(),
            runtime_added_suits: Vec::new(),
            suit_scan_cache: None,
            world,
            ticks: 0,
            start: 0,
            answers: Answers::default(),
            fresh: BTreeSet::new(),
            view_rotation: 0,
            answered: false,
            house: 5,
            family: 1,
            loaded: [0; 32],
            view_centre: None,
            floor_rotation: 0,
            replay_view: None,
            last_dialog_text: String::new(),
            character_names: BTreeMap::new(),
            lot_name: String::new(),
            lot_description: String::new(),
            special_panels: SpecialPanels::default(),
            special_panel_responses: VecDeque::new(),
            deletion_epoch: 0,
        };
        let bytes = bad.encode();
        assert!(matches!(Saved::decode(&bytes), Err(SaveError::Format(_))));
        assert!(matches!(
            Saved::decode_strict(&bytes),
            Err(SaveError::Format(_))
        ));
    }
}
