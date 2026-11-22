//! Playable Neighborhood ↔ Family Bin ↔ Lot screen state machine.
//!
//! Joins the decoupled library pillars into a single headless-testable shell:
//! - [`sims_render::neighborhood::NeighborhoodMap`] for Complete Collection 1.91
//!   map rendering and pixel-exact lot silhouette hit-testing,
//! - [`sims_ui::neighborhood::NeighborhoodChrome`] and
//!   [`sims_ui::family_bin::FamilyBin`] for top-bar controls, 9-slice House Info
//!   hover popups, and the Select a Family picker,
//! - [`sims_sim::neighborhood_store::NeighborhoodStore`] and
//!   [`sims_vm::lifecycle`] for atomic REDB persistence (never mutating
//!   `SIMS_INSTALL`) across lot entry, Save & Leave, Move-In, Evict, and
//!   Bulldoze, and
//! - [`sims_audio::music::MusicMode`] transitions for streaming mode music.

use crate::autosave_worker::{
    AutosaveWorker, SaveCompletion, build_save_manager_snapshot, format_save_label, parse_slot_id,
    validate_save_payload,
};
use crate::caf_screen::{CafOutcome, CafSession};
use crate::coop::default_player_name;
use crate::game::{Game, InstallAssets, SpecialLotEntry};
use crate::hud::{DIALOG_FADE_DURATION_MS, DialogKey};
use crate::sound::{self, Input};
use sims_audio::SoundEvent;
use sims_audio::music::MusicMode;
use sims_content::ui_graphics::UiGraphics;
use sims_content::{ArchivePool, Catalog, Index, Install};
use sims_core::CrtRng;
use sims_formats::bitmap::Image;
use sims_formats::family_file::FamilyFile;
use sims_formats::iff::{ChunkData, Iff, write as write_iff};
use sims_formats::strings::StringTable;
use sims_render::camera::{Camera, Walls, Zoom};
use sims_render::neighborhood::{
    COLOR_KEY_565, CommunityFilter, HoverFadeState, LotStatus, LotZoning, MapLot, NeighborhoodKind,
    NeighborhoodMap, RenderOptions, Surface565, pack_565,
};
use sims_render::{Canvas, Focus};
use sims_sim::album_store::{AlbumFamilyToken, AlbumImage, NewAlbumPhoto};
use sims_sim::autosave::{SavePayloadKind, SaveSlot, SaveTarget};
use sims_sim::load::Context;
use sims_sim::migration::StoreMigrationReport;
use sims_sim::neighborhood_store::{
    DEFAULT_ARCH_VALUE_MULTIPLIER, NeighborhoodStore, StoredLotThumbnail, StoredNeighborhood,
    ThumbnailSurface565, family_net_worth, lot_total_value,
};
use sims_ui::album::{
    AlbumCommand, AlbumEntry, AlbumKey, AlbumSnapshot, AlbumStrings, PhotoAlbumDialog, PhotoId,
};
use sims_ui::art::Art;
use sims_ui::book::{
    BookCommand, BookEntry, BookPage, BookStrings, ReferenceBookDialog, ReferenceBookSnapshot,
};
use sims_ui::command::Command;
use sims_ui::coop::{CoopCommand, CoopLobbyDialog, CoopSnapshot, LobbyTab};
use sims_ui::dialog::{self, DialogFadeState, DialogState, FrameStyle, Labels, Message};
use sims_ui::family_bin::{
    FamilyBin, FamilyBinCommand, FamilyBinMode, FamilyBinStrings, FamilyBinView, FamilyEntry,
    FamilyMemberPortrait,
};
use sims_ui::frame::{Frame, Rect};
use sims_ui::household::Faces;
use sims_ui::loading::{LoadingAudioCue, LoadingKind, LoadingScreen, LoadingState, LoadingStrings};
use sims_ui::neighborhood::{
    LotInfo, LotKind, LotOccupant, MAX_NEIGHBORHOOD, MIN_NEIGHBORHOOD, NeighborhoodChrome,
    NeighborhoodCommand, NeighborhoodMode, NeighborhoodState, NeighborhoodStrings,
    format_simoleons,
};
use sims_ui::options::{OptionsState, OptionsTexts};
use sims_ui::saves::{SaveManagerCommand, SaveManagerDialog, SaveManagerSnapshot};
use sims_ui::shop::{
    CurrencyKind, ShopCommand, ShopItem, ShopKind, ShopPickerDialog, ShopSnapshot, ShopStrings,
    Wallet, WardrobeCategory,
};
use sims_ui::travel::{
    Destination, LotFilter, TravelBarMode, TravelChrome, TravelCommand, TravelDialog,
    TravelDialogCommand, TravelDialogLayout, TravelDialogState, TravelLayout, TravelState,
    TravelStrings, TravelTransport as UiTravelTransport,
};
use sims_vm::boot::Frames;
use sims_vm::boot::save::Saved;
use sims_vm::caf::{CafContext, encode_fami, import_family_fam};
use sims_vm::lifecycle::{self, BulldozeReport, EvictReport, MoveInReport, SaveReport};
use sims_vm::travel::{
    self as vm_travel, ReturnReport, TravelDestination, TravelRequest, TravelSession,
    TravelTransport as VmTravelTransport, ValidatedTravelPlan,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const US_ENGLISH: u8 = 1;
const NEIGHBORHOOD_DIALOG_STRINGS: u16 = 131;
const MOVE_IN_DIALOG_STRINGS: u16 = 132;
const IMPORT_DIALOG_STRINGS: u16 = 143;
const REZONE_DIALOG_STRINGS: u16 = 250;
const ZONING_TYPES_STRINGS: u16 = 251;
const STREET_NAMES_TABLE: u16 = 2000;
const LOT_ASSIGNMENTS_TABLE: u16 = 2001;
const LOT_DESC_BASE_TABLE: u16 = 2000;
/// Water wave animation cadence on the neighborhood map (200 ms per step, 5 Hz).
pub const WATER_STEP_US: u64 = 200_000;
/// Steady-hover debounce delay before popping the neighborhood House Info card (275 ms).
pub const HOVER_CARD_DELAY_US: u64 = 275_000;
/// `Global.far` `FCNS #4` tuning chunk ID.
const NEIGHBORHOOD_TUNING_CHUNK: u16 = 4;

/// High-level screen currently shown by [`NeighborhoodShell`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    Loading,
    Neighborhood,
    FamilyBin,
    CreateFamily,
    Lot,
}

/// Host-visible side effects from a [`NeighborhoodShell`] input event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShellOutcome {
    pub redraw: bool,
    pub quit: bool,
    pub photo: bool,
}

impl ShellOutcome {
    fn redraw() -> Self {
        Self {
            redraw: true,
            quit: false,
            photo: false,
        }
    }
}

/// REDB neighborhood ID (`"UserData"` for slot 1, `"UserData2"`..=`"UserData8"`).
pub fn neighborhood_id_for_slot(slot: u8) -> String {
    let slot = slot.clamp(MIN_NEIGHBORHOOD, MAX_NEIGHBORHOOD);
    if slot == 1 {
        "UserData".to_owned()
    } else {
        format!("UserData{slot}")
    }
}

/// Streaming [`MusicMode`] for a neighborhood map screen.
pub fn music_mode_for_kind(kind: NeighborhoodKind) -> MusicMode {
    match kind {
        NeighborhoodKind::Unleashed | NeighborhoodKind::Classic => MusicMode::Neighborhood,
        NeighborhoodKind::Downtown => MusicMode::Downtown,
        NeighborhoodKind::Vacation => MusicMode::Vacation,
        NeighborhoodKind::StudioTown => MusicMode::Superstar,
        NeighborhoodKind::MagicTown => MusicMode::MagicTown,
    }
}

/// Modal confirmation/notice dialog active over the Neighborhood or Family Bin screen.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ShellDialogKind {
    ConfirmQuit,
    ConfirmMoveIn {
        family_id: u16,
        lot_id: u16,
    },
    ConfirmEvict {
        lot_id: u16,
    },
    ConfirmBulldoze {
        lot_id: u16,
    },
    ConfirmDeleteFamily {
        family_id: u16,
    },
    ConfirmResumeOrAbortTrip {
        home_house: u32,
        destination_house: u32,
    },
    ConfirmRezone {
        lot_id: u16,
        target_zone: i32,
        expected_instance_id: u128,
        expected_revision: u64,
        expected_occupant: Option<u16>,
    },
    Notice,
}

/// Discovered `.FAM` family archive eligible for importing into the Family Bin.
#[derive(Debug, Clone)]
struct ImportableFamily {
    path: PathBuf,
    family_name: String,
    member_count: usize,
    budget: i32,
    source_label: String,
}

/// Optional desktop host hook called immediately before blocking lot/travel
/// transitions with the rendered native [`LoadingScreen`] packed `0x00RRGGBB`
/// pixel buffer, canvas size, and drained audio queue (starting [`MusicMode::Load`]).
pub type LoadingPresenter = Box<dyn FnMut(&[u32], [i32; 2], Vec<Input>) + Send>;

/// Conservative invariant key for steady-state Neighborhood hover-fade frames
/// where only dynamic lot rects changed on the cached map layer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NeighborhoodSteadyKey {
    size: [i32; 2],
    slot: u8,
    kind: NeighborhoodKind,
    water_frame: usize,
    pointer: Option<[i32; 2]>,
    nbhd_state: NeighborhoodState,
    travel_state: TravelState,
    chrome_hovered: Option<sims_ui::neighborhood::BarButton>,
    chrome_pressed: Option<sims_ui::neighborhood::BarButton>,
    travel_hovered: Option<sims_ui::travel::TravelControl>,
    travel_pressed: Option<sims_ui::travel::TravelControl>,
    pressed_lot_id: Option<u16>,
    magic_family_bin_hovered: bool,
    magic_family_bin_pressed: bool,
}

/// Maximum number of full-resolution album images kept hydrated around the
/// currently selected page (well under `PhotoAlbumStore`'s 32 MiB bulk limit).
const ALBUM_IMAGE_WINDOW: usize = 12;

struct ShellDialog {
    kind: ShellDialogKind,
    message: Message,
    state: DialogState,
    layout: Option<dialog::Layout>,
}

/// Cached per-lot display and valuation data for the active neighborhood.
#[derive(Debug, Clone, Default)]
struct LotMeta {
    house_cost: i32,
    has_house: bool,
    from_saved_lot: bool,
    lot_name: String,
    lot_description: String,
    filter_mask: Option<u16>,
}

/// Cached custom thumbnail cells captured after Save & Leave or Bulldoze.
#[derive(Debug, Clone)]
struct CustomThumb {
    normal: Surface565,
    roofs_off: Option<Surface565>,
}

/// Parsed `StreetNames.iff` street templates and lot assignments.
#[derive(Debug, Clone, Default)]
pub(crate) struct StreetNames {
    streets: Vec<String>,
    assignments: Vec<Option<usize>>,
}

impl StreetNames {
    pub(crate) fn load(install: &Install, user_dir: &Path) -> Self {
        let candidate = resolve_child_case_insensitive(user_dir, "StreetNames.iff")
            .or_else(|| install.file("TemplateUserData/StreetNames.iff").ok());
        let Some(path) = candidate else {
            return Self::default();
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        Self::parse_iff(bytes)
    }

    fn parse_iff(bytes: Vec<u8>) -> Self {
        let Ok(iff) = Iff::parse(bytes) else {
            return Self::default();
        };
        let read_vec = |id: u16| -> Vec<String> {
            let Some(chunk) = iff.get(b"STR#", id) else {
                return Vec::new();
            };
            let Ok(table) = StringTable::parse(iff.data(chunk)) else {
                return Vec::new();
            };
            (0..table.entries.len())
                .map(|i| {
                    table
                        .get(i, US_ENGLISH)
                        .or_else(|| table.get(i, 1))
                        .unwrap_or("")
                        .to_owned()
                })
                .collect()
        };
        let streets = read_vec(STREET_NAMES_TABLE);
        let assignments = read_vec(LOT_ASSIGNMENTS_TABLE)
            .into_iter()
            .map(|s| {
                s.trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|v| v.checked_sub(1))
            })
            .collect();
        Self {
            streets,
            assignments,
        }
    }

    /// Formats the street address for 1-based `lot_number` if `StreetNames.iff`
    /// assigns a street to it.
    pub(crate) fn address_for(&self, lot_number: u16) -> Option<String> {
        let idx = usize::from(lot_number).checked_sub(1)?;
        let street_idx = (*self.assignments.get(idx)?)?;
        let template = self.streets.get(street_idx).filter(|s| !s.is_empty())?;
        let num = lot_number.to_string();
        Some(format_template(template, &[&num]))
    }
}

/// Parsed `Houses/*Desc.iff` lot names and descriptions (`0x50e14b..0x50e253`),
/// matching `sims_vm::boot::read_lot_info`: reads `STR# (2000 + lot_id)`
/// entries 0 (name) and 1 (description) from `DTDesc.iff`, `VIDesc.iff`,
/// `STDesc.iff`, `MTDesc.iff`, or `NeighborhoodDesc.iff`.
///
/// Each existing `*Desc.iff` file is read and parsed at most once on load and
/// cached by file and table ID, eliminating 40+ redundant disk reads during
/// [`build_lot_meta`].
#[derive(Debug, Clone, Default)]
pub(crate) struct NeighborhoodDescriptions {
    by_file: BTreeMap<&'static str, BTreeMap<u16, (String, String)>>,
}

impl NeighborhoodDescriptions {
    const DESC_FILES: [&'static str; 5] = [
        "NeighborhoodDesc.iff",
        "DTDesc.iff",
        "VIDesc.iff",
        "STDesc.iff",
        "MTDesc.iff",
    ];

    pub(crate) fn load(install: &Install, user_dir: &Path) -> Self {
        let mut houses_dirs = Vec::new();
        for dir in [
            Some(user_dir.to_path_buf()),
            install.file("UserData").ok(),
            install.file("TemplateUserData").ok(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(houses) = resolve_dir_case_insensitive(&dir, "Houses")
                && !houses_dirs.contains(&houses)
            {
                houses_dirs.push(houses);
            }
        }
        let mut by_file = BTreeMap::new();
        for &desc_file in &Self::DESC_FILES {
            let mut entries: BTreeMap<u16, (String, String)> = BTreeMap::new();
            for houses_dir in &houses_dirs {
                let Some(path) = resolve_child_case_insensitive(houses_dir, desc_file) else {
                    continue;
                };
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(scanned) = Iff::scan_types(&bytes, &[*b"STR#"]) else {
                    continue;
                };
                for chunk in scanned.all(b"STR#") {
                    // Match `Iff::get(b"STR#", table_id)` (high flag byte 0).
                    if chunk.flags >> 8 != 0 || chunk.id < LOT_DESC_BASE_TABLE {
                        continue;
                    }
                    if entries.contains_key(&chunk.id) {
                        continue;
                    }
                    let Ok(table) = StringTable::parse(&chunk.data) else {
                        continue;
                    };
                    let name = table
                        .get(0, US_ENGLISH)
                        .or_else(|| table.get(0, 1))
                        .unwrap_or_default()
                        .to_owned();
                    let desc = table
                        .get(1, US_ENGLISH)
                        .or_else(|| table.get(1, 1))
                        .unwrap_or_default()
                        .to_owned();
                    if !name.is_empty() || !desc.is_empty() {
                        entries.insert(chunk.id, (name, desc));
                    }
                }
            }
            by_file.insert(desc_file, entries);
        }
        Self { by_file }
    }

    pub(crate) fn lot_info(&self, lot_number: u16) -> (String, String) {
        let desc_file = match lot_number {
            21..=30 => "DTDesc.iff",
            40..=48 => "VIDesc.iff",
            81..=89 => "STDesc.iff",
            90..=99 => "MTDesc.iff",
            _ => "NeighborhoodDesc.iff",
        };
        let Some(table_id) = LOT_DESC_BASE_TABLE.checked_add(lot_number) else {
            return (String::new(), String::new());
        };
        self.by_file
            .get(desc_file)
            .and_then(|map| map.get(&table_id))
            .cloned()
            .unwrap_or_default()
    }
}

/// Localized dialog strings loaded dynamically from `GameData/UIText.iff`.
#[derive(Debug, Clone, Default)]
struct DialogStrings {
    /// `STR# 131`: Bulldoze / Evict confirmation messages.
    bulldoze: Vec<String>,
    /// `STR# 132`: Move-In confirmation / notice messages.
    move_in: Vec<String>,
    /// `STR# 143`: Import Family strings.
    import_strs: Vec<String>,
    /// `STR# 250`: Rezone Mode confirmation strings.
    rezone: Vec<String>,
    /// `STR# 251`: Zoning type display names (`[0]` Residential, `[1]` Community).
    zoning_types: Vec<String>,
    /// `STR# 128`: Family Bin strings (Delete Family confirmation at 6..=7).
    family_bin: FamilyBinStrings,
    /// `STR# 153`: Options / Quit confirmation strings.
    options: OptionsTexts,
    /// `STR# 142` & `STR# 161`: OK/Cancel/Yes/No labels + hotkeys.
    labels: Labels,
}

impl DialogStrings {
    fn load(install: &Install) -> Self {
        let mut out = Self {
            bulldoze: Vec::new(),
            move_in: Vec::new(),
            import_strs: Vec::new(),
            rezone: Vec::new(),
            zoning_types: Vec::new(),
            family_bin: FamilyBinStrings::load(install, US_ENGLISH).unwrap_or_default(),
            options: OptionsTexts::load(install).unwrap_or_default(),
            labels: Labels::load(install).unwrap_or_default(),
        };
        if let Ok(path) = install.file("GameData/UIText.iff")
            && let Ok(bytes) = std::fs::read(&path)
            && let Ok(iff) = Iff::parse(bytes)
        {
            let read = |id: u16| -> Vec<String> {
                let Some(chunk) = iff.get(b"STR#", id) else {
                    return Vec::new();
                };
                let Ok(table) = StringTable::parse(iff.data(chunk)) else {
                    return Vec::new();
                };
                (0..table.entries.len())
                    .map(|i| {
                        table
                            .get(i, US_ENGLISH)
                            .or_else(|| table.get(i, 1))
                            .unwrap_or("")
                            .to_owned()
                    })
                    .collect()
            };
            out.bulldoze = read(NEIGHBORHOOD_DIALOG_STRINGS);
            out.move_in = read(MOVE_IN_DIALOG_STRINGS);
            out.import_strs = read(IMPORT_DIALOG_STRINGS);
            out.rezone = read(REZONE_DIALOG_STRINGS);
            out.zoning_types = read(ZONING_TYPES_STRINGS);
        }
        out
    }

    fn get<'a>(list: &'a [String], idx: usize, fallback: &'a str) -> &'a str {
        list.get(idx)
            .filter(|s| !s.is_empty())
            .map_or(fallback, String::as_str)
    }

    fn ok(&self) -> String {
        self.labels.get(1).to_owned()
    }

    fn yes(&self) -> String {
        self.labels.get(3).to_owned()
    }

    fn no(&self) -> String {
        self.labels.get(4).to_owned()
    }
}

/// Ephemeral REDB staging store kept alive for a lot booted from a Save Manager
/// slot so subsequent saves run the full `sims_vm::lifecycle::save_and_leave_lot`
/// pipeline (`people::save_to_record`, `update_lot_values`, `deletion_epoch`)
/// before committing atomically to the real store with strict fencing.
struct StagingStoreHandle {
    _dir: TempStagingDir,
    store: NeighborhoodStore,
}

/// Active live lot session launched from the Neighborhood map.
struct LotSession {
    game: Game,
    neighborhood_id: String,
    neighborhood_instance_id: u128,
    /// Expected REDB neighborhood revision for optimistic write fencing.
    neighborhood_revision: u64,
    /// Present when the lot was booted from an ephemeral reconciled staging store
    /// (e.g. Save Manager slot load or corrupt-save recovery).
    staging: Option<StagingStoreHandle>,
    house: u32,
    /// Last observed VM `days_played` (`world.globals[28]`) for exact midnight
    /// rotating autosave detection (`daily::turn_day`).
    last_days_played: i16,
    /// Present when the household is actively visiting a travel destination
    /// (`depart_to_destination` / `resume_active_trip`), so leaving returns
    /// home cleanly via `return_from_destination` without overwriting the
    /// destination community lot.
    active_trip: Option<TravelSession>,
    /// True when the lot was opened as an unoccupied destination community lot
    /// in Build/Buy edit mode (`open_destination_for_edit`).
    is_destination_edit: bool,
}

enum ActiveScreen {
    Loading(LoadingState),
    Neighborhood,
    FamilyBin,
    CreateFamily(Box<CafSession>),
    Lot(Box<LotSession>),
}

/// Pending travel departure plan awaiting confirmation in [`TravelDialog`].
struct PendingTravel {
    request: TravelRequest,
    plan: ValidatedTravelPlan,
}

/// Full playable Neighborhood ↔ Family Bin ↔ Lot application shell.
pub struct NeighborhoodShell {
    install_root: PathBuf,
    install: Install,
    /// Shared FAR archive handle pool reused across Neighborhood UI loading,
    /// neighborhood portrait reloads, and cross-lot transitions.
    pool: Arc<ArchivePool>,
    /// Cached base install resource index (without neighborhood-specific
    /// `Characters`), shared across neighborhood switches and lot entries.
    base_index: Option<Arc<Index>>,
    /// Cached base install catalog (when built without neighborhood characters).
    base_catalog: Option<Arc<Catalog>>,
    /// Lazily populated immutable install-wide assets (`Index`, `Catalog`,
    /// `ArchivePool`, surface plug-ins, `HudStatic`, `HudShared`,
    /// `ContentAssets`, `PeopleLibrary`) reused across lot entries & travel.
    install_assets: Option<Arc<InstallAssets>>,
    saves_path: PathBuf,
    store: NeighborhoodStore,
    autosave_worker: AutosaveWorker,
    migration_report: Option<StoreMigrationReport>,
    migration_error: Option<String>,
    options_file: PathBuf,
    options_state: OptionsState,
    saved_optn: Vec<u8>,
    play: bool,
    /// When `true` (`--debug`), draws the top diagnostic/status banners on
    /// Neighborhood, CAF, and Lot screens and logs diagnostic notes to stderr.
    debug: bool,
    answers: Vec<u8>,
    frames: Frames,
    arch_multiplier: f32,
    magic_lot_91_cost: i32,
    magic_lot_other_cost: i32,

    screen: ActiveScreen,
    /// Monotonic generation bumped whenever a lot is entered or restored in
    /// place so the host app resets error counters and lot-scoped audio.
    lot_generation: u64,
    size: [i32; 2],
    canvas: Canvas,
    pointer: Option<[i32; 2]>,
    clock_us: u64,
    water_frame: usize,

    // Shared UI artwork & strings loaded once from the install.
    chrome_art: Art,
    chrome_strings: NeighborhoodStrings,
    family_bin_art: Art,
    dialog_art: Art,
    save_manager_art: Art,
    loading_art: Art,
    loading_strings: LoadingStrings,
    album_art: Art,
    travel_art: Art,
    travel_strings: TravelStrings,
    dialogs: DialogStrings,

    // Native Loading Screen state (`sims_ui::loading::LoadingScreen`).
    loading_screen: LoadingScreen,
    loading_caption_seq: usize,

    // Active neighborhood state.
    slot: u8,
    neighborhood_id: String,
    kind: NeighborhoodKind,
    map: NeighborhoodMap,
    stored: StoredNeighborhood,
    family_names: BTreeMap<u16, String>,
    street_names: StreetNames,
    descriptions: NeighborhoodDescriptions,
    faces: Faces,
    lot_meta: BTreeMap<u16, LotMeta>,
    custom_thumbs: BTreeMap<(String, u16), CustomThumb>,

    // Neighborhood screen interactive state.
    chrome: NeighborhoodChrome,
    nbhd_state: NeighborhoodState,
    hovered_lot_id: Option<u16>,
    /// `self.clock_us` timestamp when `hovered_lot_id` became the hovered lot.
    hovered_lot_since_us: Option<u64>,
    /// Lot ID whose House Info card is currently popped in `nbhd_state.hovered_lot`.
    hover_card_shown_for: Option<u16>,
    /// 300 ms native per-lot roof cross-fade state (`cWinLot`, `0x54ea7f..0x54eb3d`).
    hover_fade: HoverFadeState,
    /// Tracks whether any roof cross-fade was actively animating on the last tick
    /// so the final landed frame is always drawn before idle wakes stop.
    hover_fade_was_animating: bool,
    pressed_lot_id: Option<u16>,
    move_in_family_id: Option<u16>,

    // Travel destination sub-neighborhood chrome & confirmation dialog.
    travel_chrome: TravelChrome,
    travel_state: TravelState,
    travel_dialog: TravelDialog,
    travel_dialog_state: Option<TravelDialogState>,
    pending_travel: Option<PendingTravel>,
    magic_family_bin_hovered: bool,
    magic_family_bin_pressed: bool,
    loading_presenter: Option<LoadingPresenter>,

    // Family Bin screen interactive state.
    family_bin: FamilyBin,
    family_bin_entries: Vec<FamilyEntry>,

    // Modal confirmation / notice dialog over Neighborhood or Family Bin.
    dialog: Option<ShellDialog>,
    /// Opacity fade-in state for shell confirmation / notice dialogs (`cTSWinFade`).
    dialog_fade: DialogFadeState,
    dialog_fade_was_animating: bool,
    // Household Photo Album / Storybook modal (overlay over Lot or Neighborhood).
    album_dialog: PhotoAlbumDialog,
    album_snapshot: Option<AlbumSnapshot>,
    album_token: Option<AlbumFamilyToken>,
    album_neighborhood_id: Option<String>,
    album_has_image: BTreeSet<PhotoId>,
    album_strip_scroll: usize,
    album_strip_pressed: Option<i32>,
    // Crash-safe Save & Rotating Autosave Manager dialog (overlay over Lot/Neighborhood).
    save_manager: SaveManagerDialog,
    save_manager_snapshot: Option<SaveManagerSnapshot>,
    save_manager_target_lot: Option<u32>,
    last_save_completions: Vec<SaveCompletion>,
    // Co-op lobby modal over Neighborhood screen (delegates to Game when in a Lot).
    coop_art: Art,
    coop_lobby: CoopLobbyDialog,
    coop_error: Option<String>,
    coop_player_name: String,
    coop_iroh_config: sims_net::IrohConfig,
    // Neighborhood Family Import picker modal (`ImportFamily` top-bar button).
    import_search_dirs: Vec<PathBuf>,
    import_picker: ShopPickerDialog,
    import_picker_snapshot: Option<ShopSnapshot>,
    import_picker_entries: BTreeMap<String, ImportableFamily>,
    import_picker_strings: ShopStrings,
    import_caf_ctx: Option<CafContext>,
    import_rng_seed: u32,
    // Neighborhood Credits/About paginated reference modal (`OpenCredits`).
    credits_dialog: ReferenceBookDialog,
    credits_snapshot: Option<ReferenceBookSnapshot>,
    book_strings: BookStrings,
    note: Option<String>,

    // Pending sound inputs to be drained by the host.
    audio_queue: Vec<Input>,

    // Safe partial-damage tracking for steady-state Neighborhood hover fades.
    prev_nb_steady_key: Option<NeighborhoodSteadyKey>,
    prev_nb_dynamic_rects: Vec<sims_render::canvas::Rect>,
    present_damage: Option<Vec<sims_render::canvas::Rect>>,
}

impl NeighborhoodShell {
    /// Opens the Neighborhood shell for `install_root`, storing all play state
    /// inside the REDB file `saves_path` (creating/importing neighborhood `slot`
    /// on first run without mutating `install_root`).
    pub fn open(
        install_root: PathBuf,
        saves_path: PathBuf,
        slot: u8,
        play: bool,
        answers: Vec<u8>,
        frames: Frames,
    ) -> Result<Self, String> {
        let install = Install::detect(&install_root).map_err(|e| e.to_string())?;
        let store = NeighborhoodStore::open(&saves_path).map_err(|e| e.to_string())?;
        // Non-destructive startup migration: surface any schema/corruption error
        // clearly without crashing or overwriting existing saves.
        let (migration_report, migration_error, startup_note) = match store.migrate_all() {
            Ok(report) => (Some(report), None, None),
            Err(err) => {
                let msg =
                    format!("Save migration warning (existing saves preserved untouched): {err}");
                (None, Some(err.to_string()), Some(msg))
            }
        };
        let autosave_worker = AutosaveWorker::spawn(store.autosaves());
        let options_file = saves_path.with_extension("options.iff");
        let slot = slot.clamp(MIN_NEIGHBORHOOD, MAX_NEIGHBORHOOD);
        let kind = default_map_kind(&install, slot);
        let neighborhood_id = neighborhood_id_for_slot(slot);
        let stored = ensure_imported(&install, &store, &neighborhood_id)?;
        let _ = store.set_active_neighborhood(&neighborhood_id);
        let user_dir = resolve_user_dir(&install, slot)?;
        let pool = Arc::new(ArchivePool::new());

        // Parallelize independent UI art packs, string/tuning tables,
        // neighborhood map, descriptions/street names, and base Index/Faces
        // while sharing a single ArchivePool.
        let (art_res, strings_tuple, map_res, text_meta, faces_tuple, custom_thumbs, family_names) =
            std::thread::scope(|scope| {
                let art_handle = scope.spawn(|| -> Result<_, String> {
                    let ui = UiGraphics::open_with_pool(&install, Arc::clone(&pool))
                        .map_err(|e| e.to_string())?;
                    let chrome_art = sims_ui::neighborhood::load_art(&ui, US_ENGLISH)
                        .map_err(|e| e.to_string())?;
                    let family_bin_art = sims_ui::family_bin::load_art(&ui, US_ENGLISH)
                        .map_err(|e| e.to_string())?;
                    let dialog_art = Art::load(&ui, US_ENGLISH, &dialog::RESOURCES)
                        .map_err(|e| e.to_string())?;
                    let save_manager_art = Art::load_with_optional(
                        &ui,
                        US_ENGLISH,
                        &dialog::RESOURCES,
                        &[sims_ui::neighborhood::art_id::POPUP_TILER],
                    )
                    .map_err(|e| e.to_string())?;
                    let loading_art = sims_ui::loading::load_art(&ui, US_ENGLISH)
                        .or_else(|_| {
                            Art::load_with_optional(
                                &ui,
                                US_ENGLISH,
                                &[],
                                &sims_ui::loading::RESOURCES,
                            )
                        })
                        .map_err(|e| e.to_string())?;
                    let album_art = sims_ui::album::load_art(&ui, US_ENGLISH)
                        .or_else(|_| {
                            Art::load_with_optional(
                                &ui,
                                US_ENGLISH,
                                &dialog::RESOURCES,
                                &sims_ui::album::RESOURCES,
                            )
                        })
                        .map_err(|e| e.to_string())?;
                    let travel_art = sims_ui::travel::load_art(&ui, US_ENGLISH)
                        .or_else(|_| {
                            Art::load_with_optional(
                                &ui,
                                US_ENGLISH,
                                &[],
                                &sims_ui::travel::RESOURCES,
                            )
                        })
                        .map_err(|e| e.to_string())?;
                    let coop_art = sims_ui::coop::load_art(&ui, US_ENGLISH)
                        .or_else(|_| {
                            Art::load_with_optional(&ui, US_ENGLISH, &[], &sims_ui::coop::RESOURCES)
                        })
                        .unwrap_or_default();
                    Ok((
                        chrome_art,
                        family_bin_art,
                        dialog_art,
                        save_manager_art,
                        loading_art,
                        album_art,
                        travel_art,
                        coop_art,
                    ))
                });
                let strings_handle = scope.spawn(|| {
                    let options_state = if options_file.is_file() {
                        OptionsState::load_iff(&options_file)
                            .or_else(|_| OptionsState::load_install(&install))
                            .unwrap_or_default()
                    } else {
                        OptionsState::load_install(&install).unwrap_or_default()
                    };
                    let chrome_strings =
                        NeighborhoodStrings::load(&install, US_ENGLISH).unwrap_or_default();
                    let loading_strings =
                        LoadingStrings::load(&install, US_ENGLISH).unwrap_or_default();
                    let album_strings =
                        AlbumStrings::load_from_install(&install, US_ENGLISH).unwrap_or_default();
                    let travel_strings =
                        TravelStrings::load(&install, US_ENGLISH).unwrap_or_default();
                    let dialogs = DialogStrings::load(&install);
                    let arch_multiplier = load_arch_multiplier(&install);
                    let magic_lot_91_cost = lifecycle::magic_town_lot_cost(Some(&install_root), 91);
                    let magic_lot_other_cost =
                        lifecycle::magic_town_lot_cost(Some(&install_root), 90);
                    (
                        options_state,
                        chrome_strings,
                        loading_strings,
                        album_strings,
                        travel_strings,
                        dialogs,
                        arch_multiplier,
                        magic_lot_91_cost,
                        magic_lot_other_cost,
                    )
                });
                let map_handle = scope.spawn(|| load_map_for_slot(&install, slot, kind));
                let meta_handle = scope.spawn(|| {
                    let street_names = StreetNames::load(&install, &user_dir);
                    let descriptions = NeighborhoodDescriptions::load(&install, &user_dir);
                    (street_names, descriptions)
                });
                let faces_handle = scope.spawn(|| {
                    let base_index = Index::build(&install).ok().map(Arc::new);
                    let (faces, base_catalog) = load_neighborhood_faces_cached(
                        &install,
                        &store,
                        &neighborhood_id,
                        &stored,
                        &pool,
                        base_index.as_ref(),
                        None,
                    );
                    (base_index, base_catalog, faces)
                });
                let family_names = load_family_names_with_caf(
                    &user_dir,
                    &saves_path,
                    &neighborhood_id,
                    stored.instance_id,
                    &stored,
                );
                let custom_thumbs = load_persisted_custom_thumbs(&store, &neighborhood_id);
                (
                    art_handle.join().expect("neighborhood art worker panicked"),
                    strings_handle
                        .join()
                        .expect("neighborhood strings worker panicked"),
                    map_handle.join().expect("neighborhood map worker panicked"),
                    meta_handle
                        .join()
                        .expect("neighborhood descriptions worker panicked"),
                    faces_handle
                        .join()
                        .expect("neighborhood faces worker panicked"),
                    custom_thumbs,
                    family_names,
                )
            });

        let (
            chrome_art,
            family_bin_art,
            dialog_art,
            save_manager_art,
            loading_art,
            album_art,
            travel_art,
            coop_art,
        ) = art_res?;
        let (
            options_state,
            chrome_strings,
            loading_strings,
            album_strings,
            travel_strings,
            dialogs,
            arch_multiplier,
            magic_lot_91_cost,
            magic_lot_other_cost,
        ) = strings_tuple;
        let saved_optn = options_state.to_optn();
        let mut map = map_res?;
        let (street_names, descriptions) = text_meta;
        let (base_index, base_catalog, faces) = faces_tuple;
        let lot_meta = build_lot_meta(
            &store,
            &neighborhood_id,
            &map,
            &descriptions,
            arch_multiplier,
        );
        sync_map_with_store(
            &mut map,
            &stored,
            &lot_meta,
            &neighborhood_id,
            &custom_thumbs,
        );
        let family_bin_entries = build_family_bin_entries(&stored, &family_names, &faces);

        let nbhd_state = NeighborhoodState {
            neighborhood_number: slot,
            mode: NeighborhoodMode::Normal,
            expansions: sims_ui::neighborhood::ExpansionFlags::ALL,
            has_importable_family: false,
            show_credits_button: destination_for_kind(kind).is_none(),
            hovered_lot: None,
        };
        let import_picker_strings = ShopStrings::load(&install, US_ENGLISH).unwrap_or_default();
        let book_strings = BookStrings::load(&install, US_ENGLISH).unwrap_or_default();
        let travel_state = TravelState::default();

        let vols = [
            options_state.sound.fx,
            options_state.sound.music,
            options_state.sound.vox,
        ];
        let mut loading_screen = LoadingScreen::new();
        let mut audio_queue = vec![
            Input::FxVolume(vols[0]),
            Input::MusicVolume(vols[1]),
            Input::VoxVolume(vols[2]),
        ];
        if let Some(LoadingAudioCue::StartLoadLoop { .. }) = loading_screen.begin(0) {
            audio_queue.push(Input::MusicMode(MusicMode::Load));
        }
        if let Some(LoadingAudioCue::StopLoadLoop) = loading_screen.finish() {
            audio_queue.push(Input::MusicMode(music_mode_for_kind(kind)));
        } else {
            audio_queue.push(Input::MusicMode(music_mode_for_kind(kind)));
        }

        let mut shell = Self {
            install_root,
            install,
            pool,
            base_index,
            base_catalog,
            install_assets: None,
            saves_path,
            store,
            autosave_worker,
            migration_report,
            migration_error,
            options_file,
            options_state,
            saved_optn,
            play,
            debug: false,
            answers,
            frames,
            arch_multiplier,
            magic_lot_91_cost,
            magic_lot_other_cost,
            screen: ActiveScreen::Neighborhood,
            lot_generation: 0,
            size: [800, 600],
            canvas: Canvas::new(800, 600, [0, 0, 0]),
            pointer: None,
            clock_us: 0,
            water_frame: 0,
            chrome_art,
            chrome_strings,
            family_bin_art,
            dialog_art,
            save_manager_art,
            loading_art,
            loading_strings,
            album_art,
            travel_art,
            travel_strings,
            dialogs,
            loading_screen,
            loading_caption_seq: 1,
            slot,
            neighborhood_id,
            kind,
            map,
            stored,
            family_names,
            street_names,
            descriptions,
            faces,
            lot_meta,
            custom_thumbs,
            chrome: NeighborhoodChrome::new(),
            nbhd_state,
            hovered_lot_id: None,
            hovered_lot_since_us: None,
            hover_card_shown_for: None,
            hover_fade: HoverFadeState::new(),
            hover_fade_was_animating: false,
            pressed_lot_id: None,
            move_in_family_id: None,
            travel_chrome: TravelChrome::new(),
            travel_state,
            travel_dialog: TravelDialog::new(),
            travel_dialog_state: None,
            pending_travel: None,
            magic_family_bin_hovered: false,
            magic_family_bin_pressed: false,
            loading_presenter: None,
            family_bin: FamilyBin::new(),
            family_bin_entries,
            dialog: None,
            dialog_fade: DialogFadeState::hidden(),
            dialog_fade_was_animating: false,
            album_dialog: PhotoAlbumDialog::new(album_strings),
            album_snapshot: None,
            album_token: None,
            album_neighborhood_id: None,
            album_has_image: BTreeSet::new(),
            album_strip_scroll: 0,
            album_strip_pressed: None,
            save_manager: SaveManagerDialog::new(),
            save_manager_snapshot: None,
            save_manager_target_lot: None,
            last_save_completions: Vec::new(),
            coop_art,
            coop_lobby: CoopLobbyDialog::new(),
            coop_error: None,
            coop_player_name: default_player_name("Player"),
            coop_iroh_config: sims_net::IrohConfig::P2pDefault,
            import_search_dirs: Vec::new(),
            import_picker: ShopPickerDialog::new(),
            import_picker_snapshot: None,
            import_picker_entries: BTreeMap::new(),
            import_picker_strings,
            import_caf_ctx: None,
            import_rng_seed: 1,
            credits_dialog: ReferenceBookDialog::new(),
            credits_snapshot: None,
            book_strings,
            note: startup_note,
            audio_queue,
            prev_nb_steady_key: None,
            prev_nb_dynamic_rects: Vec::new(),
            present_damage: None,
        };
        shell.refresh_importable_families();
        let active_trips = shell.inspect_any_active_trips();
        if !active_trips.is_empty() && shell.note.is_none() {
            let (home_house, trip) = &active_trips[0];
            shell.note = Some(format!(
                "Lot {home_house} has an interrupted trip to lot {}. Click it to Resume or Abort.",
                trip.destination_house
            ));
        }
        Ok(shell)
    }

    pub fn screen(&self) -> ScreenKind {
        match self.screen {
            ActiveScreen::Loading(_) => ScreenKind::Loading,
            ActiveScreen::Neighborhood => ScreenKind::Neighborhood,
            ActiveScreen::FamilyBin => ScreenKind::FamilyBin,
            ActiveScreen::CreateFamily(_) => ScreenKind::CreateFamily,
            ActiveScreen::Lot(_) => ScreenKind::Lot,
        }
    }

    /// Monotonic counter incremented on every lot entry or Save Manager / F9
    /// restore so the window host resets error indices and lot-scoped audio.
    pub fn lot_generation(&self) -> u64 {
        self.lot_generation
    }

    pub fn migration_report(&self) -> Option<&StoreMigrationReport> {
        self.migration_report.as_ref()
    }

    pub fn migration_error(&self) -> Option<&str> {
        self.migration_error.as_deref()
    }

    pub fn autosave_worker(&self) -> &AutosaveWorker {
        &self.autosave_worker
    }

    pub fn pending_background_saves(&self) -> usize {
        self.autosave_worker.pending_count()
    }

    pub fn save_manager_open(&self) -> bool {
        self.save_manager.is_open()
    }

    pub fn save_manager(&self) -> &SaveManagerDialog {
        &self.save_manager
    }

    pub fn save_manager_snapshot(&self) -> Option<&SaveManagerSnapshot> {
        self.save_manager_snapshot.as_ref()
    }

    pub fn loading_screen(&self) -> &LoadingScreen {
        &self.loading_screen
    }

    pub fn loading_strings(&self) -> &LoadingStrings {
        &self.loading_strings
    }

    pub fn loading_state(&self) -> Option<&LoadingState> {
        match &self.screen {
            ActiveScreen::Loading(state) => Some(state),
            _ => None,
        }
    }

    /// Enters the interactive native [`LoadingScreen`] (`ScreenKind::Loading`),
    /// emitting [`Input::MusicMode`]`(`[`MusicMode::Load`]`)` when
    /// [`LoadingAudioCue::StartLoadLoop`] fires.
    pub fn begin_loading(
        &mut self,
        kind: LoadingKind,
        destination_label: Option<String>,
        progress: f32,
    ) {
        self.start_load_audio();
        self.screen = ActiveScreen::Loading(LoadingState {
            kind,
            progress,
            destination_label,
            caption_override: None,
        });
    }

    /// Updates the progress ratio and optional caption override while on
    /// [`ScreenKind::Loading`].
    pub fn update_loading(&mut self, progress: f32, caption_override: Option<String>) -> bool {
        if let ActiveScreen::Loading(state) = &mut self.screen {
            state.progress = progress;
            state.caption_override = caption_override;
            true
        } else {
            false
        }
    }

    /// Advances the active loading screen's rotating caption timer by `delta_ms`.
    pub fn tick_loading(&mut self, delta_ms: u64) {
        self.loading_screen.tick(delta_ms);
    }

    /// Finishes the active [`LoadingScreen`], emitting
    /// [`LoadingAudioCue::StopLoadLoop`] -> destination music mode and returning
    /// to the Neighborhood screen if still on [`ScreenKind::Loading`].
    pub fn finish_loading(&mut self) {
        if matches!(self.screen, ActiveScreen::Loading(_)) {
            self.screen = ActiveScreen::Neighborhood;
        }
        let target_music = self.desired_music_mode();
        self.stop_load_audio(target_music);
    }

    /// Renders a standalone [`LoadingScreen`] frame at `size` using the shell's
    /// loaded splash artwork and `UIText.iff` caption strings.
    pub fn render_loading_frame(&self, state: &LoadingState, size: [u32; 2]) -> Canvas {
        let w = size[0].max(1);
        let h = size[1].max(1);
        let mut frame = Frame::new(w, h, [0, 0, 0]);
        self.loading_screen
            .draw(&mut frame, state, &self.loading_art, &self.loading_strings);
        let mut canvas = Canvas::new(w as usize, h as usize, [0, 0, 0]);
        canvas.color = frame.into_packed();
        canvas
    }

    /// Installs a desktop host presenter invoked immediately before blocking
    /// lot/travel transitions so the native [`LoadingScreen`] splash, progress
    /// bar, rotating caption, and [`MusicMode::Load`] are presented live while
    /// work executes.
    pub fn set_loading_presenter(&mut self, presenter: Option<LoadingPresenter>) {
        self.loading_presenter = presenter;
    }

    /// Enables or disables `--debug` mode (top diagnostic/status banners and
    /// diagnostic stderr logging), propagating to any active lot [`Game`].
    pub fn set_debug(&mut self, debug: bool) {
        self.debug = debug;
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.set_debug(debug);
        }
    }

    /// Whether `--debug` diagnostic banner & stderr logging mode is active.
    pub fn debug(&self) -> bool {
        self.debug
    }

    /// Sets a programmatic/diagnostic status note on the active Lot or
    /// Neighborhood screen. Shown visually in the top strip only when `--debug`
    /// is enabled; always readable via [`Self::note`] / [`Self::status_note`].
    pub fn set_note(&mut self, note: impl Into<String>) {
        let text = note.into();
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.set_note(text);
        } else {
            if self.debug {
                eprintln!("sims-game [debug]: {text}");
            }
            self.note = Some(text);
        }
    }

    /// Surfaces a genuine user-facing error or notice via a native Sims 1 modal
    /// message dialog (`cTSWinMsgBox` / `ShellDialogKind::Notice`), while also
    /// recording `body` in the programmatic status note getter.
    pub fn show_notice(&mut self, title: impl Into<String>, body: impl Into<String>) {
        let title = title.into();
        let body = body.into();
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.show_notice(title, body);
        } else {
            if self.debug {
                eprintln!("sims-game [debug]: {body}");
            }
            self.note = Some(body.clone());
            self.open_dialog(
                ShellDialogKind::Notice,
                Message::notice(title, body, self.dialogs.ok()).with_frame(FrameStyle::Generic),
            );
        }
    }

    /// Returns the active status note on the current Lot or Neighborhood screen, if any.
    pub fn note(&self) -> Option<&str> {
        if let ActiveScreen::Lot(session) = &self.screen {
            session.game.note()
        } else {
            self.note.as_deref()
        }
    }

    fn present_loading_step(
        &mut self,
        kind: LoadingKind,
        destination_label: Option<String>,
        progress: f32,
    ) {
        if self.loading_presenter.is_none() {
            return;
        }
        let state = LoadingState {
            kind,
            progress: progress.clamp(0.0, 1.0),
            destination_label,
            caption_override: None,
        };
        let frame_size = self.frame_size_u32();
        let canvas = self.render_loading_frame(&state, frame_size);
        let audio = std::mem::take(&mut self.audio_queue);
        if let Some(mut presenter) = self.loading_presenter.take() {
            presenter(&canvas.color, self.size, audio);
            self.loading_presenter = Some(presenter);
        }
    }

    fn start_load_audio(&mut self) {
        let seq = self.loading_caption_seq;
        self.loading_caption_seq = self.loading_caption_seq.wrapping_add(1);
        if let Some(LoadingAudioCue::StartLoadLoop { .. }) = self.loading_screen.begin(seq) {
            self.audio_queue.push(Input::MusicMode(MusicMode::Load));
        }
    }

    fn stop_load_audio(&mut self, next_music: MusicMode) {
        if let Some(LoadingAudioCue::StopLoadLoop) = self.loading_screen.finish() {
            self.audio_queue.push(Input::MusicMode(next_music));
        }
    }

    pub fn album_open(&self) -> bool {
        self.album_dialog.is_open()
    }

    pub fn album_dialog(&self) -> &PhotoAlbumDialog {
        &self.album_dialog
    }

    pub fn album_snapshot(&self) -> Option<&AlbumSnapshot> {
        self.album_snapshot.as_ref()
    }

    pub fn travel_state(&self) -> &TravelState {
        &self.travel_state
    }

    pub fn travel_dialog_open(&self) -> bool {
        self.travel_dialog_state.is_some()
    }

    pub fn travel_dialog_state(&self) -> Option<&TravelDialogState> {
        self.travel_dialog_state.as_ref()
    }

    /// Active travel session when the household is currently visiting a
    /// destination lot (`depart_to_destination` / `resume_active_trip`).
    pub fn active_trip(&self) -> Option<&TravelSession> {
        match &self.screen {
            ActiveScreen::Lot(session) => session.active_trip.as_ref(),
            _ => None,
        }
    }

    /// Whether the active lot is an unoccupied destination lot opened for
    /// Build/Buy editing (`open_destination_for_edit`).
    pub fn is_destination_edit(&self) -> bool {
        match &self.screen {
            ActiveScreen::Lot(session) => session.is_destination_edit,
            _ => false,
        }
    }

    pub fn caf_session(&self) -> Option<&CafSession> {
        match &self.screen {
            ActiveScreen::CreateFamily(caf) => Some(caf),
            _ => None,
        }
    }

    pub fn caf_session_mut(&mut self) -> Option<&mut CafSession> {
        match &mut self.screen {
            ActiveScreen::CreateFamily(caf) => Some(caf),
            _ => None,
        }
    }

    pub fn neighborhood_slot(&self) -> u8 {
        self.slot
    }

    pub fn neighborhood_id(&self) -> &str {
        &self.neighborhood_id
    }

    pub fn neighborhood_kind(&self) -> NeighborhoodKind {
        self.kind
    }

    pub fn neighborhood_state(&self) -> &NeighborhoodState {
        &self.nbhd_state
    }

    pub fn map(&self) -> &NeighborhoodMap {
        &self.map
    }

    pub fn store(&self) -> &NeighborhoodStore {
        &self.store
    }

    pub fn stored_neighborhood(&self) -> &StoredNeighborhood {
        &self.stored
    }

    pub fn family_bin(&self) -> &FamilyBin {
        &self.family_bin
    }

    pub fn family_bin_entries(&self) -> &[FamilyEntry] {
        &self.family_bin_entries
    }

    pub fn game(&self) -> Option<&Game> {
        match &self.screen {
            ActiveScreen::Lot(session) => Some(&session.game),
            _ => None,
        }
    }

    pub fn game_mut(&mut self) -> Option<&mut Game> {
        match &mut self.screen {
            ActiveScreen::Lot(session) => Some(&mut session.game),
            _ => None,
        }
    }

    pub fn active_lot_house(&self) -> Option<u32> {
        match &self.screen {
            ActiveScreen::Lot(session) => Some(session.house),
            _ => None,
        }
    }

    pub fn hovered_lot_id(&self) -> Option<u16> {
        self.hovered_lot_id
    }

    fn now_ms(&self) -> u64 {
        self.clock_us / 1_000
    }

    fn update_hover_fade(&mut self, prev_lot: Option<u16>, hovered: Option<u16>) {
        if self.clock_us > 0 {
            let now_ms = self.clock_us / 1_000;
            // If `prev_lot` was hovered at `clock_us == 0` (rendered settled at
            // 1.0 without starting a hidden ramp), seed it as settled before
            // transitioning so moving away fades its roof back in smoothly.
            if self.hover_fade.hovered().is_none()
                && let Some(prev) = prev_lot
                && prev_lot != hovered
            {
                let seed_ms =
                    now_ms.saturating_sub(sims_render::neighborhood::HOVER_FADE_DURATION_MS);
                self.hover_fade.set_hovered(Some(prev), seed_ms);
            }
            self.hover_fade.set_hovered(hovered, now_ms);
            self.hover_fade_was_animating = self.hover_fade.is_animating(now_ms);
        } else {
            self.hover_fade.reset();
            self.hover_fade_was_animating = false;
        }
    }

    /// Whether any Neighborhood lot roof cross-fade (300 ms `cWinLot` ramp) is
    /// actively transitioning at the current presentation clock.
    pub fn has_active_fades(&self) -> bool {
        self.clock_us > 0
            && matches!(self.screen, ActiveScreen::Neighborhood)
            && self.hover_fade.is_animating(self.now_ms())
    }

    /// Current Neighborhood hover roof cross-fade tracker (`HoverFadeState`).
    pub fn hover_fade_state(&self) -> &HoverFadeState {
        &self.hover_fade
    }

    /// Current shell modal dialog fade state (`DialogFadeState`).
    pub fn dialog_fade_state(&self) -> &DialogFadeState {
        &self.dialog_fade
    }

    /// Current water ripple animation frame index on the Neighborhood map.
    pub fn water_frame(&self) -> usize {
        self.water_frame
    }

    /// Whether hovering a lot on the current map should draw the synthetic
    /// yellow highlight outline (`RenderOptions::highlight_hovered`).
    ///
    /// Disabled during plain idle residential/destination browsing (matching
    /// Complete Collection 1.91, which lifts the roof on hover without a yellow
    /// outline), and enabled only during Move-In, Bulldoze, or active Community
    /// Filter modes.
    pub fn should_highlight_hovered_lot(&self) -> bool {
        let in_action_mode = matches!(
            self.nbhd_state.mode,
            NeighborhoodMode::MoveIn { .. } | NeighborhoodMode::Bulldoze | NeighborhoodMode::Rezone
        ) || self.travel_state.mode == TravelBarMode::Bulldoze;
        let has_active_filter = destination_for_kind(self.kind).is_some()
            && (self.travel_state.active_filter.is_some()
                || (self.kind == NeighborhoodKind::Vacation
                    && self.travel_state.vacation_lodging_filter.is_some()));
        in_action_mode || has_active_filter
    }

    pub fn has_dialog(&self) -> bool {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
            || self.coop_lobby.is_open()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open()
        {
            return true;
        }
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.has_dialog() || session.game.shop_open(),
            ActiveScreen::CreateFamily(caf) => caf.controller().confirm().is_some(),
            _ => self.dialog.is_some(),
        }
    }

    /// Whether the Neighborhood Family Import picker modal is currently open.
    pub fn import_picker_open(&self) -> bool {
        self.import_picker.is_open()
    }

    /// Active snapshot for the Family Import picker modal, if open.
    pub fn import_picker_snapshot(&self) -> Option<&ShopSnapshot> {
        self.import_picker_snapshot.as_ref()
    }

    /// Whether any importable `.FAM` household was found for the top-bar Import button.
    pub fn has_importable_family(&self) -> bool {
        self.nbhd_state.has_importable_family
    }

    /// Whether the Neighborhood Credits/About reference modal is currently open.
    pub fn credits_open(&self) -> bool {
        self.credits_dialog.is_open()
    }

    /// Active [`ReferenceBookDialog`] for the Credits/About modal.
    pub fn credits_dialog(&self) -> &ReferenceBookDialog {
        &self.credits_dialog
    }

    /// Active [`ReferenceBookSnapshot`] for the Credits/About modal, if open.
    pub fn credits_snapshot(&self) -> Option<&ReferenceBookSnapshot> {
        self.credits_snapshot.as_ref()
    }

    /// Whether any paginated reference book modal (Neighborhood Credits or
    /// in-lot Spellbook / Cookbook) is currently open.
    pub fn book_open(&self) -> bool {
        self.credits_dialog.is_open()
            || matches!(&self.screen, ActiveScreen::Lot(s) if s.game.book_open())
    }

    pub fn dialog_layout(&self) -> Option<&dialog::Layout> {
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.dialog(),
            _ => self.dialog.as_ref().and_then(|d| d.layout.as_ref()),
        }
    }

    pub fn status_note(&self) -> Option<&str> {
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.note().or(self.note.as_deref()),
            ActiveScreen::CreateFamily(caf) => caf.note().or(self.note.as_deref()),
            _ => self.note.as_deref(),
        }
    }

    pub fn volumes(&self) -> [u8; 3] {
        if let ActiveScreen::Lot(session) = &self.screen {
            session.game.volumes()
        } else {
            [
                self.options_state.sound.fx,
                self.options_state.sound.music,
                self.options_state.sound.vox,
            ]
        }
    }

    pub fn desired_music_mode(&self) -> MusicMode {
        match &self.screen {
            ActiveScreen::Loading(_) => MusicMode::Load,
            ActiveScreen::Neighborhood => music_mode_for_kind(self.kind),
            ActiveScreen::FamilyBin | ActiveScreen::CreateFamily(_) => MusicMode::FamilyBin,
            ActiveScreen::Lot(session) => {
                sound::lot_music_mode(session.game.mode(), session.game.lot().lot_id())
            }
        }
    }

    /// Drains queued mixer inputs generated by screen transitions.
    pub fn take_audio_inputs(&mut self) -> Vec<Input> {
        std::mem::take(&mut self.audio_queue)
    }

    /// Whether the active screen has an interactive tool, drag, or dialog open
    /// that benefits from full monitor refresh rate.
    pub fn is_actively_interacting(&self) -> bool {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
            || self.coop_lobby.is_open()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open()
        {
            return true;
        }
        match &self.screen {
            ActiveScreen::Loading(_) => true,
            ActiveScreen::Lot(session) => session.game.is_actively_interacting(),
            ActiveScreen::Neighborhood => {
                self.dialog.is_some()
                    || self.has_active_fades()
                    || (self.clock_us > 0 && self.dialog_fade.is_animating(self.now_ms() as u32))
                    || !matches!(self.nbhd_state.mode, NeighborhoodMode::Normal)
                    || self.pressed_lot_id.is_some()
            }
            ActiveScreen::FamilyBin => self.dialog.is_some(),
            ActiveScreen::CreateFamily(_) => true,
        }
    }

    /// Window title string appropriate for the current screen.
    pub fn window_title(&self) -> String {
        match &self.screen {
            ActiveScreen::Loading(state) => match &state.destination_label {
                Some(label) => format!("sims-game: Loading {label}..."),
                None => "sims-game: Loading...".to_owned(),
            },
            ActiveScreen::Neighborhood => {
                if let Some(dest) = destination_for_kind(self.kind) {
                    format!(
                        "sims-game: Neighborhood {} — {}",
                        self.slot,
                        dest.short_label()
                    )
                } else {
                    format!("sims-game: Neighborhood {}", self.slot)
                }
            }
            ActiveScreen::FamilyBin => {
                format!("sims-game: Neighborhood {} — Select a Family", self.slot)
            }
            ActiveScreen::CreateFamily(_) => {
                format!("sims-game: Neighborhood {} — Create a Family", self.slot)
            }
            ActiveScreen::Lot(session) => {
                let suffix = if session.game.stopped().is_some() {
                    ", stopped"
                } else if session.game.paused() {
                    ", paused"
                } else {
                    ""
                };
                format!("sims-game: tick {}{suffix}", session.game.lot().ticks())
            }
        }
    }

    /// Persists user Options (`Optn` 128) if modified in the live lot.
    pub fn persist_options(&mut self) {
        if let ActiveScreen::Lot(session) = &self.screen {
            let current = session.game.options().to_optn();
            if current != self.saved_optn {
                self.options_state = *session.game.options();
                match session.game.save_options(&self.options_file) {
                    Ok(()) => self.saved_optn = current,
                    Err(e) => eprintln!("sims-game: {}: {e}", self.options_file.display()),
                }
            }
        }
    }

    pub fn resize(&mut self, size: [i32; 2]) -> bool {
        let size = [size[0].max(1), size[1].max(1)];
        let changed = self.size != size;
        self.size = size;
        if let ActiveScreen::Lot(session) = &mut self.screen {
            return session.game.resize(size) || changed;
        }
        if changed {
            self.relayout_dialog();
        }
        changed
    }

    /// Advances shell-level animation timers (e.g. loading captions, neighborhood
    /// water waves, Photo Album caret blink, CAF turntable & caret blink) and
    /// polls background save completions. Returns `true` when the active screen
    /// should be redrawn.
    pub fn tick_shell(&mut self, delta_us: u64) -> bool {
        self.clock_us = self.clock_us.saturating_add(delta_us);
        let mut redraw = self.poll_background_saves();
        if self.album_dialog.is_open() && self.album_dialog.sync_blink(delta_us) {
            redraw = true;
        }
        if self.coop_lobby.is_open() && self.coop_lobby.sync_blink(self.clock_us) {
            redraw = true;
        }
        if let Some(dlg) = &mut self.dialog {
            if dlg.message.entry.is_some() && dlg.state.sync_blink(self.clock_us) {
                redraw = true;
            }
            let now_ms = self.now_ms() as u32;
            let anim = self.clock_us > 0 && self.dialog_fade.is_animating(now_ms);
            if anim || self.dialog_fade_was_animating {
                redraw = true;
            }
            self.dialog_fade_was_animating = anim;
        } else {
            self.dialog_fade_was_animating = false;
        }
        match &mut self.screen {
            ActiveScreen::Loading(_) => {
                let ms = (delta_us / 1_000).max(u64::from(delta_us > 0));
                self.loading_screen.tick(ms);
                redraw = true;
            }
            ActiveScreen::Neighborhood => {
                let now_ms = self.clock_us / 1_000;
                let fade_anim_now = self.hover_fade.is_animating(now_ms);
                if fade_anim_now || self.hover_fade_was_animating {
                    redraw = true;
                }
                self.hover_fade_was_animating = fade_anim_now;
                self.hover_fade.prune(now_ms);

                let next_wave = (self.clock_us / WATER_STEP_US) as usize;
                if next_wave != self.water_frame {
                    self.water_frame = next_wave;
                    let has_waves = self.map.waves.iter().any(Option::is_some);
                    let has_filter_anim = destination_for_kind(self.kind).is_some()
                        && self.travel_state.active_filter.is_some();
                    redraw |= has_waves || has_filter_anim;
                }
                let modal_open = self.dialog.is_some()
                    || self.save_manager.is_open()
                    || self.album_dialog.is_open()
                    || self.travel_dialog_state.is_some()
                    || self.coop_lobby.is_open()
                    || self.import_picker.is_open()
                    || self.credits_dialog.is_open();
                if !modal_open
                    && let (Some(lot_id), Some(since_us)) =
                        (self.hovered_lot_id, self.hovered_lot_since_us)
                    && self.hover_card_shown_for != Some(lot_id)
                    && self.clock_us.saturating_sub(since_us) >= HOVER_CARD_DELAY_US
                {
                    self.hover_card_shown_for = Some(lot_id);
                    self.nbhd_state.hovered_lot = self.build_lot_info(lot_id);
                    if self.nbhd_state.hovered_lot.is_some() {
                        redraw = true;
                    }
                }
            }
            ActiveScreen::CreateFamily(caf) => {
                redraw |= caf.tick(delta_us);
                self.drain_caf_voice_previews();
            }
            _ => {}
        }
        redraw
    }

    /// Ensures immutable install-wide assets are loaded once and cached on this
    /// shell for fast cross-lot transitions.
    fn ensure_install_assets(&mut self) -> Result<Arc<InstallAssets>, String> {
        if let Some(cached) = &self.install_assets {
            return Ok(Arc::clone(cached));
        }
        let loaded = Arc::new(InstallAssets::load(
            &self.install,
            Arc::clone(&self.pool),
            self.base_index.clone(),
            self.base_catalog.clone(),
        )?);
        self.base_index = Some(Arc::clone(&loaded.base_index));
        self.base_catalog = Some(Arc::clone(&loaded.base_catalog));
        self.install_assets = Some(Arc::clone(&loaded));
        Ok(loaded)
    }

    /// Exact microseconds until this shell next needs an event-loop wakeup, or
    /// `None` when completely idle (allowing `ControlFlow::Wait`).
    pub fn next_wake_us(&self, dragging: bool, anim_interval_us: u64) -> Option<u64> {
        self.next_wake_us_with_window(dragging, anim_interval_us, true, false)
    }

    /// Like [`NeighborhoodShell::next_wake_us`], gating pure presentation
    /// wakeups on `can_draw` and letting the redraw pacer own already-dirty
    /// frame deadlines so a zero-sized window or deferred redraw never spins.
    pub fn next_wake_us_with_window(
        &self,
        dragging: bool,
        anim_interval_us: u64,
        can_draw: bool,
        frame_dirty: bool,
    ) -> Option<u64> {
        let mut best: Option<u64> = None;
        let mut consider = |us: u64| {
            best = Some(best.map_or(us, |prev| prev.min(us)));
        };
        if self.autosave_worker.has_pending_or_unread() {
            consider(25_000);
        }
        if can_draw {
            if let Some(us) = self.album_dialog.next_blink_us() {
                consider(us);
            }
            if let Some(us) = self.coop_lobby.next_blink_us(self.clock_us) {
                consider(us);
            }
            if let Some(dialog) = &self.dialog {
                let now_ms = self.now_ms() as u32;
                if (self.clock_us > 0 && self.dialog_fade.is_animating(now_ms))
                    || self.dialog_fade_was_animating
                {
                    consider(anim_interval_us.max(1_000));
                }
                if dialog.message.entry.is_some()
                    && let Some(us) = dialog.state.next_blink_us(self.clock_us)
                {
                    consider(us);
                }
            }
        }
        let modal_open = self.dialog.is_some()
            || self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
            || self.coop_lobby.is_open()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open();
        match &self.screen {
            ActiveScreen::Loading(_) => {
                if can_draw {
                    consider(anim_interval_us.max(1_000));
                }
            }
            ActiveScreen::Neighborhood => {
                if can_draw {
                    let now_ms = self.clock_us / 1_000;
                    if self.hover_fade.is_animating(now_ms) || self.hover_fade_was_animating {
                        consider(anim_interval_us.max(1_000));
                    }
                    let has_waves = self.map.waves.iter().any(Option::is_some);
                    let has_filter_anim = destination_for_kind(self.kind).is_some()
                        && self.travel_state.active_filter.is_some();
                    if has_waves || has_filter_anim {
                        let rem = WATER_STEP_US - (self.clock_us % WATER_STEP_US);
                        consider(rem.max(1_000));
                    }
                    if !modal_open
                        && let (Some(lot_id), Some(since_us)) =
                            (self.hovered_lot_id, self.hovered_lot_since_us)
                        && self.hover_card_shown_for != Some(lot_id)
                    {
                        let elapsed = self.clock_us.saturating_sub(since_us);
                        let rem = HOVER_CARD_DELAY_US.saturating_sub(elapsed);
                        if rem > 0 {
                            consider(rem);
                        } else if !frame_dirty {
                            consider(anim_interval_us.max(1_000));
                        }
                    }
                }
            }
            ActiveScreen::FamilyBin => {}
            ActiveScreen::CreateFamily(caf) => {
                if can_draw {
                    consider(caf.next_wake_us());
                }
            }
            ActiveScreen::Lot(session) => {
                let shell_modal_open = self.save_manager.is_open()
                    || self.album_dialog.is_open()
                    || self.travel_dialog_state.is_some();
                if shell_modal_open {
                    if session.game.coop_active() || session.game.pending_coop_restage().is_some() {
                        consider(5_000);
                    }
                } else if let Some(us) = session.game.next_wake_us_with_window(
                    dragging,
                    anim_interval_us,
                    can_draw,
                    frame_dirty,
                ) {
                    consider(us);
                }
            }
        }
        best
    }

    pub fn canvas(&self) -> &Canvas {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
        {
            return &self.canvas;
        }
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.canvas(),
            _ => &self.canvas,
        }
    }

    /// Canvas-space dirty rects accumulated since the last successful window
    /// present, or `None` when a full blit/present is required.
    pub fn peek_present_damage(&self) -> Option<&[sims_render::canvas::Rect]> {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
        {
            return None;
        }
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.peek_present_damage(),
            ActiveScreen::Neighborhood => self.present_damage.as_deref(),
            _ => None,
        }
    }

    /// Marks the active screen's current canvas as successfully presented to
    /// the window backbuffer.
    pub fn on_presented(&mut self) {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
        {
            self.present_damage = None;
            if let ActiveScreen::Lot(session) = &mut self.screen {
                session.game.invalidate_present_damage();
            }
            return;
        }
        match &mut self.screen {
            ActiveScreen::Lot(session) => {
                session.game.on_presented();
                self.present_damage = None;
            }
            ActiveScreen::Neighborhood => {
                self.present_damage = Some(Vec::new());
            }
            _ => {
                self.present_damage = None;
            }
        }
    }

    pub fn render(&mut self) {
        if let ActiveScreen::Lot(session) = &mut self.screen {
            self.prev_nb_steady_key = None;
            self.prev_nb_dynamic_rects.clear();
            self.present_damage = None;
            session.game.render();
            let has_overlay = self.save_manager.is_open()
                || self.album_dialog.is_open()
                || self.travel_dialog_state.is_some();
            if has_overlay {
                session.game.invalidate_present_damage();
                let lot_canvas = session.game.canvas();
                let w = lot_canvas.width;
                let h = lot_canvas.height;
                if let Ok(mut frame) =
                    Frame::from_packed(w as u32, h as u32, lot_canvas.color.clone())
                {
                    if self.album_dialog.is_open()
                        && let Some(snapshot) = &self.album_snapshot
                    {
                        self.album_dialog
                            .draw(&mut frame, snapshot, &self.album_art);
                    }
                    if let Some(td_state) = &self.travel_dialog_state {
                        self.travel_dialog.draw(
                            &mut frame,
                            td_state,
                            &self.travel_art,
                            &self.travel_strings,
                        );
                    }
                    if self.save_manager.is_open()
                        && let Some(snapshot) = &self.save_manager_snapshot
                    {
                        self.save_manager
                            .draw(&mut frame, snapshot, &self.save_manager_art);
                    }
                    let mut composed = Canvas::new(w, h, [0, 0, 0]);
                    composed.color = frame.into_packed();
                    self.canvas = composed;
                }
            }
            return;
        }
        let w = self.size[0].max(1) as usize;
        let h = self.size[1].max(1) as usize;
        if self.canvas.width != w || self.canvas.height != h {
            self.canvas = Canvas::new(w, h, [0, 0, 0]);
        }

        match &mut self.screen {
            ActiveScreen::Loading(state) => {
                self.prev_nb_steady_key = None;
                self.prev_nb_dynamic_rects.clear();
                self.present_damage = None;
                let packed = std::mem::take(&mut self.canvas.color);
                if let Ok(mut frame) = Frame::from_packed(w as u32, h as u32, packed) {
                    self.loading_screen.draw(
                        &mut frame,
                        state,
                        &self.loading_art,
                        &self.loading_strings,
                    );
                    self.canvas.color = frame.into_packed();
                } else {
                    self.canvas.color = vec![0u32; w * h];
                }
            }
            ActiveScreen::Neighborhood => {
                let is_travel_map = destination_for_kind(self.kind).is_some();
                let highlighted_lots = if is_travel_map {
                    self.travel_filtered_lot_ids()
                } else {
                    BTreeSet::new()
                };
                let active_filter = if is_travel_map
                    && !(self.kind == NeighborhoodKind::Vacation
                        && self.travel_state.vacation_lodging_filter.is_some())
                {
                    self.travel_state
                        .active_filter
                        .map(community_filter_for_lot_filter)
                } else {
                    None
                };
                let mut opts = RenderOptions {
                    viewport: [w as u32, h as u32],
                    hovered_lot: self.hovered_lot_id,
                    hover_roofs_off: true,
                    highlight_hovered: self.should_highlight_hovered_lot(),
                    highlighted_lots,
                    water_frame: Some(self.water_frame),
                    active_filter,
                    filter_frame: self.water_frame,
                    show_occupancy_markers: matches!(
                        self.nbhd_state.mode,
                        NeighborhoodMode::MoveIn { .. }
                    ),
                    ..RenderOptions::default()
                };
                if self.clock_us > 0 {
                    let now_ms = self.clock_us / 1_000;
                    self.hover_fade.prune(now_ms);
                    self.hover_fade.apply_to(&mut opts, now_ms);
                    // Preserve a zero-clock hover as already settled at 1.0 if
                    // the clock starts advancing before the pointer moves.
                    if let Some(lot) = self.hovered_lot_id
                        && self.hover_fade.hovered() != Some(lot)
                    {
                        opts.hovered_lot = Some(lot);
                        opts.hover_roof_fade = 1.0;
                        opts.lot_roof_fades.insert(lot, 1.0);
                    }
                }
                let map_damage = self.map.render_into_with_damage(&mut self.canvas, &opts);
                let packed = std::mem::take(&mut self.canvas.color);
                if let Ok(mut frame) = Frame::from_packed(w as u32, h as u32, packed) {
                    if is_travel_map {
                        self.travel_chrome.draw(
                            &mut frame,
                            &self.travel_state,
                            &self.travel_art,
                            &self.travel_strings,
                        );
                        if self.kind == NeighborhoodKind::MagicTown {
                            self.draw_magic_family_bin_button(&mut frame);
                        }
                    } else {
                        self.chrome.draw(
                            &mut frame,
                            &self.nbhd_state,
                            &self.chrome_art,
                            &self.chrome_strings,
                        );
                    }
                    self.draw_overlay_note_and_dialog(&mut frame);
                    if self.album_dialog.is_open()
                        && let Some(snapshot) = &self.album_snapshot
                    {
                        self.album_dialog
                            .draw(&mut frame, snapshot, &self.album_art);
                    }
                    if let Some(td_state) = &self.travel_dialog_state {
                        self.travel_dialog.draw(
                            &mut frame,
                            td_state,
                            &self.travel_art,
                            &self.travel_strings,
                        );
                    }
                    if self.save_manager.is_open()
                        && let Some(snapshot) = &self.save_manager_snapshot
                    {
                        self.save_manager
                            .draw(&mut frame, snapshot, &self.save_manager_art);
                    }
                    if self.import_picker.is_open()
                        && let Some(snapshot) = &self.import_picker_snapshot
                    {
                        self.import_picker.draw(
                            &mut frame,
                            snapshot,
                            &self.import_picker_strings,
                            &self.save_manager_art,
                        );
                    }
                    if self.credits_dialog.is_open()
                        && let Some(snapshot) = &self.credits_snapshot
                    {
                        self.credits_dialog.draw(
                            &mut frame,
                            snapshot,
                            &self.book_strings,
                            &self.save_manager_art,
                        );
                    }
                    self.canvas.color = frame.into_packed();
                } else {
                    self.canvas.color = vec![0u32; w * h];
                }

                let steady_eligible = map_damage.is_some()
                    && !self.debug
                    && self.dialog.is_none()
                    && !self.album_dialog.is_open()
                    && self.travel_dialog_state.is_none()
                    && !self.save_manager.is_open()
                    && !self.coop_lobby.is_open()
                    && !self.import_picker.is_open()
                    && !self.credits_dialog.is_open()
                    && opts.active_filter.is_none()
                    && !opts.show_occupancy_markers;
                let cur_key = steady_eligible.then(|| NeighborhoodSteadyKey {
                    size: self.size,
                    slot: self.slot,
                    kind: self.kind,
                    water_frame: self.water_frame,
                    pointer: self.pointer,
                    nbhd_state: self.nbhd_state.clone(),
                    travel_state: self.travel_state.clone(),
                    chrome_hovered: self.chrome.hovered(),
                    chrome_pressed: self.chrome.pressed(),
                    travel_hovered: self.travel_chrome.hovered(),
                    travel_pressed: self.travel_chrome.pressed(),
                    pressed_lot_id: self.pressed_lot_id,
                    magic_family_bin_hovered: self.magic_family_bin_hovered,
                    magic_family_bin_pressed: self.magic_family_bin_pressed,
                });
                let cur_rects = map_damage.unwrap_or_default();
                if cur_key.is_some() && self.prev_nb_steady_key == cur_key {
                    if let Some(dmg) = &mut self.present_damage {
                        dmg.extend_from_slice(&self.prev_nb_dynamic_rects);
                        dmg.extend_from_slice(&cur_rects);
                        if dmg.len() > 32 {
                            self.present_damage = None;
                        }
                    }
                } else {
                    self.present_damage = None;
                }
                self.prev_nb_dynamic_rects = cur_rects;
                self.prev_nb_steady_key = cur_key;
            }
            ActiveScreen::FamilyBin => {
                self.prev_nb_steady_key = None;
                self.prev_nb_dynamic_rects.clear();
                self.present_damage = None;
                let packed = std::mem::take(&mut self.canvas.color);
                if let Ok(mut frame) = Frame::from_packed(w as u32, h as u32, packed) {
                    let view = self.family_bin_view();
                    self.family_bin.draw(
                        &mut frame,
                        &view,
                        &self.family_bin_art,
                        &self.dialogs.family_bin,
                    );
                    self.draw_overlay_note_and_dialog(&mut frame);
                    self.canvas.color = frame.into_packed();
                } else {
                    self.canvas.color = vec![0u32; w * h];
                }
            }
            ActiveScreen::CreateFamily(caf) => {
                self.prev_nb_steady_key = None;
                self.prev_nb_dynamic_rects.clear();
                self.present_damage = None;
                let packed = std::mem::take(&mut self.canvas.color);
                if let Ok(mut frame) = Frame::from_packed(w as u32, h as u32, packed) {
                    caf.draw(&mut frame);
                    if self.debug
                        && let Some(note) = caf.note().or(self.note.as_deref())
                        && let Some(font) = self
                            .chrome_art
                            .font(12)
                            .or_else(|| self.chrome_art.font(10))
                    {
                        let bar_h = font.line_height + 6;
                        frame.fill(Rect::new(0, 0, frame.width() as i32, bar_h), [96, 24, 24]);
                        font.draw(&mut frame, note, [8, 3], [255, 255, 255]);
                    }
                    self.canvas.color = frame.into_packed();
                } else {
                    self.canvas.color = vec![0u32; w * h];
                }
            }
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    fn draw_overlay_note_and_dialog(&mut self, frame: &mut Frame) {
        if self.debug
            && let Some(note) = &self.note
            && let Some(font) = self
                .chrome_art
                .font(12)
                .or_else(|| self.chrome_art.font(10))
        {
            let y = sims_ui::neighborhood::BAR_HEIGHT + 4;
            let bar_h = font.line_height + 6;
            let w = frame.width() as i32;
            frame.fill(Rect::new(0, y, w, bar_h), [16, 24, 68]);
            font.draw(frame, note, [8, y + 3], [240, 245, 255]);
        }
        if let Some(dlg) = &mut self.dialog {
            if dlg.layout.is_none() {
                dlg.layout = dialog::layout(self.size, &dlg.message, &self.dialog_art);
            }
            if let Some(layout) = &dlg.layout {
                let alpha = if self.clock_us == 0 {
                    255
                } else {
                    self.dialog_fade.alpha((self.clock_us / 1_000) as u32)
                };
                dialog::draw_with_state_alpha(
                    frame,
                    &self.dialog_art,
                    &dlg.message,
                    layout,
                    self.pointer,
                    &dlg.state,
                    alpha,
                );
            }
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            self.coop_lobby.draw(frame, &snap, &self.coop_art);
        }
    }

    fn family_bin_view(&self) -> FamilyBinView<'_> {
        let mode = if self.kind == NeighborhoodKind::MagicTown {
            FamilyBinMode::MagicTown
        } else {
            FamilyBinMode::Standard
        };
        FamilyBinView {
            mode,
            families: &self.family_bin_entries,
        }
    }

    fn frame_size_u32(&self) -> [u32; 2] {
        [self.size[0].max(1) as u32, self.size[1].max(1) as u32]
    }

    pub fn hover(&mut self, point: [i32; 2]) -> bool {
        self.pointer = Some(point);
        let frame_size = self.frame_size_u32();
        if self.save_manager.is_open() {
            if let Some(snapshot) = &self.save_manager_snapshot {
                self.save_manager
                    .pointer_move(frame_size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if self.import_picker.is_open() {
            if let Some(snapshot) = &self.import_picker_snapshot {
                self.import_picker
                    .pointer_move(frame_size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if self.credits_dialog.is_open() {
            if let Some(snapshot) = &self.credits_snapshot {
                self.credits_dialog
                    .pointer_move(frame_size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            self.coop_lobby
                .pointer_move(frame_size, &snap, point[0], point[1]);
            return true;
        }
        if self.travel_dialog_state.is_some() {
            return true;
        }
        if self.album_dialog.is_open() {
            if let Some(snapshot) = &self.album_snapshot {
                self.album_dialog
                    .handle_pointer_move(point, self.size, snapshot);
            }
            return true;
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            return session.game.hover(point);
        }
        if self.dialog.is_some() {
            return true;
        }
        match &mut self.screen {
            ActiveScreen::Loading(_) => false,
            ActiveScreen::Neighborhood => {
                let prev_lot = self.hovered_lot_id;
                let (chrome_changed, over_bar) = if destination_for_kind(self.kind).is_some() {
                    let prev_btn = self.travel_chrome.hovered();
                    let prev_fb = self.magic_family_bin_hovered;
                    let layout =
                        TravelLayout::compute(frame_size[0], frame_size[1], &self.travel_state);
                    let over_magic_fb = self
                        .magic_family_bin_rect(&layout)
                        .is_some_and(|r| r.contains(point[0], point[1]));
                    self.magic_family_bin_hovered = over_magic_fb;
                    self.travel_chrome.pointer_move(point[0], point[1], &layout);
                    let cur_btn = self.travel_chrome.hovered();
                    let over_chrome = cur_btn.is_some()
                        || over_magic_fb
                        || layout.bar_rect.contains(point[0], point[1])
                        || layout.filter_strip_rect.contains(point[0], point[1]);
                    (cur_btn != prev_btn || over_magic_fb != prev_fb, over_chrome)
                } else {
                    let prev_btn = self.chrome.hovered();
                    let btn = self.chrome.hover(
                        point[0],
                        point[1],
                        frame_size,
                        &self.nbhd_state,
                        &self.chrome_art,
                    );
                    (
                        btn != prev_btn,
                        btn.is_some() || point[1] < sims_ui::neighborhood::BAR_HEIGHT,
                    )
                };
                let next_lot = if over_bar {
                    None
                } else {
                    self.map
                        .hit_test_with_state(frame_size, point, self.hovered_lot_id, true)
                };
                self.hovered_lot_id = next_lot;
                if next_lot != prev_lot {
                    self.hovered_lot_since_us = next_lot.map(|_| self.clock_us);
                    self.hover_card_shown_for = None;
                    self.nbhd_state.hovered_lot = None;
                    self.update_hover_fade(prev_lot, next_lot);
                }
                chrome_changed || next_lot != prev_lot
            }
            ActiveScreen::FamilyBin => {
                let view = FamilyBinView {
                    mode: if self.kind == NeighborhoodKind::MagicTown {
                        FamilyBinMode::MagicTown
                    } else {
                        FamilyBinMode::Standard
                    },
                    families: &self.family_bin_entries,
                };
                self.family_bin.hover(point[0], point[1], frame_size, &view);
                true
            }
            ActiveScreen::CreateFamily(caf) => {
                let _ = caf.pointer_move(point, frame_size);
                self.drain_caf_voice_previews();
                true
            }
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn leave(&mut self) -> bool {
        self.pointer = None;
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
            || self.coop_lobby.is_open()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open()
        {
            return true;
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            return session.game.leave();
        }
        match self.screen {
            ActiveScreen::Loading(_) => false,
            ActiveScreen::Neighborhood => {
                let had_state = self.chrome.hovered().is_some()
                    || self.chrome.pressed().is_some()
                    || self.travel_chrome.hovered().is_some()
                    || self.hovered_lot_id.is_some()
                    || self.pressed_lot_id.is_some();
                self.chrome.clear_pointer();
                self.magic_family_bin_hovered = false;
                self.magic_family_bin_pressed = false;
                let prev_lot = self.hovered_lot_id.take();
                self.hovered_lot_since_us = None;
                self.hover_card_shown_for = None;
                self.pressed_lot_id = None;
                self.nbhd_state.hovered_lot = None;
                self.update_hover_fade(prev_lot, None);
                had_state
            }
            ActiveScreen::FamilyBin => {
                self.family_bin.clear_pointer();
                true
            }
            ActiveScreen::CreateFamily(_) => false,
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn wheel(&mut self, delta_y: f64) -> bool {
        if self.save_manager.is_open() {
            let delta = if delta_y > 0.0 {
                -1
            } else if delta_y < 0.0 {
                1
            } else {
                return false;
            };
            let frame_size = self.frame_size_u32();
            if let Some(snapshot) = &self.save_manager_snapshot {
                return self.save_manager.scroll(delta, frame_size, snapshot);
            }
            return false;
        }
        if self.import_picker.is_open() {
            let delta = if delta_y > 0.0 {
                -1
            } else if delta_y < 0.0 {
                1
            } else {
                return false;
            };
            let frame_size = self.frame_size_u32();
            if let Some(snapshot) = &self.import_picker_snapshot {
                return self.import_picker.scroll(delta, frame_size, snapshot);
            }
            return false;
        }
        if self.credits_dialog.is_open() {
            let delta = if delta_y > 0.0 {
                -1
            } else if delta_y < 0.0 {
                1
            } else {
                return false;
            };
            let frame_size = self.frame_size_u32();
            if let Some(snapshot) = &self.credits_snapshot {
                if self.credits_dialog.scroll_vertical(
                    delta * 3,
                    frame_size,
                    snapshot,
                    &self.save_manager_art,
                ) {
                    return true;
                }
                return self.credits_dialog.scroll(delta, snapshot).is_some();
            }
            return false;
        }
        if self.album_dialog.is_open() {
            let delta = if delta_y > 0.0 {
                -1
            } else if delta_y < 0.0 {
                1
            } else {
                return false;
            };
            if let Some(snapshot) = &self.album_snapshot {
                let layout = self.album_dialog.layout(self.size[0], self.size[1]);
                let in_caption = self
                    .pointer
                    .is_some_and(|p| layout.caption_box.contains(p[0], p[1]));
                self.album_dialog
                    .handle_scroll(delta, self.size, snapshot, &self.album_art);
                if !in_caption {
                    let max_scroll = snapshot
                        .entries
                        .len()
                        .saturating_sub(layout.thumb_slots.len());
                    if delta > 0 {
                        self.album_strip_scroll =
                            (self.album_strip_scroll + delta as usize).min(max_scroll);
                    } else {
                        self.album_strip_scroll = self
                            .album_strip_scroll
                            .saturating_sub(delta.unsigned_abs() as usize);
                    }
                }
                self.hydrate_album_images();
                return true;
            }
            return false;
        }
        if self.travel_dialog_state.is_some() || self.dialog.is_some() {
            return false;
        }
        match &mut self.screen {
            ActiveScreen::Lot(session) if session.game.shop_open() => {
                session.game.shop_wheel(delta_y)
            }
            ActiveScreen::FamilyBin => {
                let delta = if delta_y > 0.0 {
                    -1
                } else if delta_y < 0.0 {
                    1
                } else {
                    return false;
                };
                let before = self.family_bin.scroll_offset();
                self.family_bin.scroll(
                    delta,
                    self.family_bin_entries.len(),
                    sims_ui::family_bin::VISIBLE_ROWS,
                );
                self.family_bin.scroll_offset() != before
            }
            _ => false,
        }
    }

    pub fn press(&mut self, point: [i32; 2]) -> ShellOutcome {
        self.pointer = Some(point);
        let frame_size = self.frame_size_u32();
        if self.save_manager.is_open() {
            if let Some(snapshot) = &self.save_manager_snapshot {
                let changed = self
                    .save_manager
                    .pointer_down(frame_size, snapshot, point[0], point[1]);
                return ShellOutcome {
                    redraw: changed,
                    ..ShellOutcome::default()
                };
            }
            return ShellOutcome::default();
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            let changed = self
                .coop_lobby
                .pointer_down(frame_size, &snap, point[0], point[1]);
            return ShellOutcome {
                redraw: changed,
                ..ShellOutcome::default()
            };
        }
        if self.import_picker.is_open() {
            if let Some(snapshot) = &self.import_picker_snapshot {
                let changed = self
                    .import_picker
                    .pointer_down(frame_size, snapshot, point[0], point[1]);
                return ShellOutcome {
                    redraw: changed,
                    ..ShellOutcome::default()
                };
            }
            return ShellOutcome::default();
        }
        if self.credits_dialog.is_open() {
            if let Some(snapshot) = &self.credits_snapshot {
                let changed = self
                    .credits_dialog
                    .pointer_down(frame_size, snapshot, point[0], point[1]);
                return ShellOutcome {
                    redraw: changed,
                    ..ShellOutcome::default()
                };
            }
            return ShellOutcome::default();
        }
        if let Some(td_state) = &self.travel_dialog_state {
            let layout = TravelDialogLayout::compute(frame_size[0], frame_size[1]);
            let changed = self
                .travel_dialog
                .pointer_down(point[0], point[1], &layout, td_state);
            return ShellOutcome {
                redraw: changed,
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            if let Some(snapshot) = &self.album_snapshot {
                let layout = self.album_dialog.layout(self.size[0], self.size[1]);
                self.album_strip_pressed = if layout.strip_prev.contains(point[0], point[1]) {
                    Some(-1)
                } else if layout.strip_next.contains(point[0], point[1]) {
                    Some(1)
                } else {
                    None
                };
                let changed = self
                    .album_dialog
                    .handle_pointer_down(point, self.size, snapshot);
                return ShellOutcome {
                    redraw: changed,
                    ..ShellOutcome::default()
                };
            }
            return ShellOutcome::default();
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            let out = session.game.press(point);
            return self.handle_lot_outcome(out);
        }
        if let Some(dlg) = &mut self.dialog {
            if let Some(layout) = &dlg.layout
                && let Some(Command::AnswerDialog { button, .. }) =
                    dialog::press_with_state(layout, point, &dlg.state)
            {
                dlg.state.pressed_button = Some(usize::from(button));
                return ShellOutcome::redraw();
            }
            return ShellOutcome::default();
        }
        self.note = None;
        match &mut self.screen {
            ActiveScreen::Loading(_) => ShellOutcome::default(),
            ActiveScreen::Neighborhood => {
                if destination_for_kind(self.kind).is_some() {
                    let layout =
                        TravelLayout::compute(frame_size[0], frame_size[1], &self.travel_state);
                    if self
                        .magic_family_bin_rect(&layout)
                        .is_some_and(|r| r.contains(point[0], point[1]))
                    {
                        self.magic_family_bin_pressed = true;
                        self.pressed_lot_id = None;
                        return ShellOutcome::redraw();
                    }
                    self.magic_family_bin_pressed = false;
                    if self.travel_chrome.pointer_down(point[0], point[1], &layout) {
                        self.pressed_lot_id = None;
                        return ShellOutcome::redraw();
                    }
                    let over_chrome = layout.bar_rect.contains(point[0], point[1])
                        || layout.filter_strip_rect.contains(point[0], point[1]);
                    let hit = if over_chrome {
                        None
                    } else {
                        self.map
                            .hit_test_with_state(frame_size, point, self.hovered_lot_id, true)
                    };
                    self.pressed_lot_id = hit;
                    if hit != self.hovered_lot_id {
                        let prev_lot = self.hovered_lot_id;
                        self.hovered_lot_id = hit;
                        self.hovered_lot_since_us = hit.map(|_| self.clock_us);
                        self.hover_card_shown_for = None;
                        self.nbhd_state.hovered_lot = None;
                        self.update_hover_fade(prev_lot, hit);
                    }
                    return ShellOutcome {
                        redraw: hit.is_some(),
                        ..ShellOutcome::default()
                    };
                }
                if self.chrome.press(
                    point[0],
                    point[1],
                    frame_size,
                    &self.nbhd_state,
                    &self.chrome_art,
                ) {
                    self.pressed_lot_id = None;
                    return ShellOutcome::redraw();
                }
                let hit =
                    self.map
                        .hit_test_with_state(frame_size, point, self.hovered_lot_id, true);
                self.pressed_lot_id = hit;
                if hit != self.hovered_lot_id {
                    let prev_lot = self.hovered_lot_id;
                    self.hovered_lot_id = hit;
                    self.hovered_lot_since_us = hit.map(|_| self.clock_us);
                    self.hover_card_shown_for = None;
                    self.nbhd_state.hovered_lot = None;
                    self.update_hover_fade(prev_lot, hit);
                }
                ShellOutcome {
                    redraw: hit.is_some(),
                    ..ShellOutcome::default()
                }
            }
            ActiveScreen::FamilyBin => {
                let view = FamilyBinView {
                    mode: if self.kind == NeighborhoodKind::MagicTown {
                        FamilyBinMode::MagicTown
                    } else {
                        FamilyBinMode::Standard
                    },
                    families: &self.family_bin_entries,
                };
                let cmd = self.family_bin.press(point[0], point[1], frame_size, &view);
                if let Some(cmd) = cmd {
                    self.handle_family_bin_command(cmd)
                } else {
                    ShellOutcome::redraw()
                }
            }
            ActiveScreen::CreateFamily(caf) => {
                let outcome = caf.pointer_down(point, frame_size);
                self.drain_caf_voice_previews();
                self.handle_caf_outcome(outcome)
            }
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn double_click(&mut self, point: [i32; 2]) -> ShellOutcome {
        self.pointer = Some(point);
        if self.save_manager.is_open()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open()
            || self.dialog.is_some()
            || !matches!(self.screen, ActiveScreen::FamilyBin)
        {
            return self.press(point);
        }
        let frame_size = self.frame_size_u32();
        let view = FamilyBinView {
            mode: if self.kind == NeighborhoodKind::MagicTown {
                FamilyBinMode::MagicTown
            } else {
                FamilyBinMode::Standard
            },
            families: &self.family_bin_entries,
        };
        if let Some(cmd) = self
            .family_bin
            .double_click(point[0], point[1], frame_size, &view)
        {
            return self.handle_family_bin_command(cmd);
        }
        // Rapid repeat clicks on non-row controls (scrollbar arrows, action
        // buttons) fall through to `press` so their release activates normally.
        self.press(point)
    }

    pub fn release(&mut self, point: [i32; 2]) -> ShellOutcome {
        self.pointer = Some(point);
        let frame_size = self.frame_size_u32();
        if self.save_manager.is_open() {
            let cmd = self.save_manager_snapshot.as_ref().and_then(|snap| {
                self.save_manager
                    .pointer_up(frame_size, snap, point[0], point[1])
            });
            return match cmd {
                Some(cmd) => self.handle_save_manager_command(cmd),
                None => ShellOutcome::redraw(),
            };
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            let cmd = self
                .coop_lobby
                .pointer_up(frame_size, &snap, point[0], point[1]);
            return match cmd {
                Some(cmd) => self.handle_shell_coop_command(cmd),
                None => ShellOutcome::redraw(),
            };
        }
        if self.import_picker.is_open() {
            let cmd = self.import_picker_snapshot.as_ref().and_then(|snap| {
                self.import_picker
                    .pointer_up(frame_size, snap, point[0], point[1])
            });
            return match cmd {
                Some(cmd) => self.handle_import_picker_command(cmd),
                None => ShellOutcome::redraw(),
            };
        }
        if self.credits_dialog.is_open() {
            let cmd = self.credits_snapshot.as_ref().and_then(|snap| {
                self.credits_dialog
                    .pointer_up(frame_size, snap, point[0], point[1])
            });
            return match cmd {
                Some(BookCommand::Close) => {
                    self.close_credits_modal();
                    ShellOutcome::redraw()
                }
                Some(BookCommand::ScrollLines(delta)) => {
                    if let Some(snap) = &self.credits_snapshot {
                        let _ = self.credits_dialog.scroll_vertical(
                            delta,
                            frame_size,
                            snap,
                            &self.save_manager_art,
                        );
                    }
                    ShellOutcome::redraw()
                }
                Some(BookCommand::PageChanged(_)) | None => ShellOutcome::redraw(),
            };
        }
        if let Some(td_state) = self.travel_dialog_state.clone() {
            let layout = TravelDialogLayout::compute(frame_size[0], frame_size[1]);
            let cmd = self
                .travel_dialog
                .pointer_up(point[0], point[1], &layout, &td_state);
            return match cmd {
                Some(cmd) => self.handle_travel_dialog_command(cmd),
                None => ShellOutcome::redraw(),
            };
        }
        if self.album_dialog.is_open() {
            let strip_pressed = self.album_strip_pressed.take();
            let layout = self.album_dialog.layout(self.size[0], self.size[1]);
            let cmds = self
                .album_snapshot
                .clone()
                .map(|snap| self.album_dialog.handle_pointer_up(point, self.size, &snap))
                .unwrap_or_default();
            if let Some(snap) = &self.album_snapshot {
                let max_scroll = snap.entries.len().saturating_sub(layout.thumb_slots.len());
                if strip_pressed == Some(-1) && layout.strip_prev.contains(point[0], point[1]) {
                    self.album_strip_scroll = self.album_strip_scroll.saturating_sub(1);
                } else if strip_pressed == Some(1) && layout.strip_next.contains(point[0], point[1])
                {
                    self.album_strip_scroll = (self.album_strip_scroll + 1).min(max_scroll);
                }
            }
            let out = self.execute_album_commands(cmds);
            self.hydrate_album_images();
            return out;
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            let out = session.game.release(point);
            return self.handle_lot_outcome(out);
        }
        if let Some(dlg) = &mut self.dialog {
            let pressed = dlg.state.pressed_button.take();
            if let (Some(pressed_idx), Some(layout)) = (pressed, &dlg.layout)
                && let Some(Command::AnswerDialog { button, .. }) =
                    dialog::press_with_state(layout, point, &dlg.state)
                && usize::from(button) == pressed_idx
            {
                return self.answer_dialog(button);
            }
            return ShellOutcome {
                redraw: pressed.is_some(),
                ..ShellOutcome::default()
            };
        }
        match &mut self.screen {
            ActiveScreen::Loading(_) => ShellOutcome::default(),
            ActiveScreen::Neighborhood => {
                if destination_for_kind(self.kind).is_some() {
                    let layout =
                        TravelLayout::compute(frame_size[0], frame_size[1], &self.travel_state);
                    let was_magic_fb = std::mem::take(&mut self.magic_family_bin_pressed);
                    if was_magic_fb
                        && self
                            .magic_family_bin_rect(&layout)
                            .is_some_and(|r| r.contains(point[0], point[1]))
                    {
                        self.pressed_lot_id = None;
                        self.open_family_bin();
                        return ShellOutcome::redraw();
                    }
                    if let Some(cmd) = self.travel_chrome.pointer_up(
                        point[0],
                        point[1],
                        &layout,
                        &self.travel_state,
                    ) {
                        self.pressed_lot_id = None;
                        return self.handle_travel_command(cmd);
                    }
                    let pressed_lot = self.pressed_lot_id.take();
                    let over_chrome = layout.bar_rect.contains(point[0], point[1])
                        || layout.filter_strip_rect.contains(point[0], point[1]);
                    let hit = if over_chrome {
                        None
                    } else {
                        self.map
                            .hit_test_with_state(frame_size, point, self.hovered_lot_id, true)
                    };
                    if let Some(lot_id) = pressed_lot
                        && Some(lot_id) == hit
                    {
                        return self.activate_lot(lot_id);
                    }
                    return ShellOutcome::redraw();
                }
                if let Some(cmd) = self.chrome.release(
                    point[0],
                    point[1],
                    frame_size,
                    &self.nbhd_state,
                    &self.chrome_art,
                ) {
                    self.pressed_lot_id = None;
                    return self.handle_neighborhood_command(cmd);
                }
                let pressed_lot = self.pressed_lot_id.take();
                let hit = if point[1] < sims_ui::neighborhood::BAR_HEIGHT {
                    None
                } else {
                    self.map
                        .hit_test_with_state(frame_size, point, self.hovered_lot_id, true)
                };
                if let Some(lot_id) = pressed_lot
                    && Some(lot_id) == hit
                {
                    return self.activate_lot(lot_id);
                }
                ShellOutcome::redraw()
            }
            ActiveScreen::FamilyBin => {
                let view = FamilyBinView {
                    mode: if self.kind == NeighborhoodKind::MagicTown {
                        FamilyBinMode::MagicTown
                    } else {
                        FamilyBinMode::Standard
                    },
                    families: &self.family_bin_entries,
                };
                if let Some(cmd) = self
                    .family_bin
                    .release(point[0], point[1], frame_size, &view)
                {
                    self.handle_family_bin_command(cmd)
                } else {
                    ShellOutcome::redraw()
                }
            }
            ActiveScreen::CreateFamily(caf) => {
                let outcome = caf.pointer_up();
                self.drain_caf_voice_previews();
                self.handle_caf_outcome(outcome)
            }
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn release_outside(&mut self) -> ShellOutcome {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open()
        {
            return ShellOutcome::redraw();
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            let out = session.game.release_outside();
            return self.handle_lot_outcome(out);
        }
        if let Some(dlg) = &mut self.dialog {
            let had = dlg.state.pressed_button.take().is_some();
            return ShellOutcome {
                redraw: had,
                ..ShellOutcome::default()
            };
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.pointer_up();
            self.drain_caf_voice_previews();
            return self.handle_caf_outcome(outcome);
        }
        self.pressed_lot_id = None;
        self.chrome.clear_pointer();
        self.family_bin.clear_pointer();
        ShellOutcome::redraw()
    }

    pub fn key_dialog(&mut self, key: DialogKey) -> ShellOutcome {
        if let ActiveScreen::Lot(session) = &mut self.screen {
            let out = session.game.dialog_confirm_key(key);
            return self.handle_lot_outcome(out);
        }
        let Some(dlg) = &self.dialog else {
            return ShellOutcome::default();
        };
        let count = dlg.message.buttons.len();
        let choice = match key {
            DialogKey::Enter => (count > 0).then_some(0u8),
            DialogKey::Escape => match count {
                0 => None,
                1 => Some(0),
                n => Some((n - 1) as u8),
            },
            DialogKey::Char(ch) if count >= 2 => {
                let lower = ch.to_ascii_lowercase();
                if lower == self.dialogs.labels.yes_key().to_ascii_lowercase() {
                    Some(0)
                } else if lower == self.dialogs.labels.no_key().to_ascii_lowercase() {
                    Some(1)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(button) = choice {
            self.answer_dialog(button)
        } else {
            ShellOutcome::default()
        }
    }

    pub fn escape(&mut self) -> ShellOutcome {
        if self.save_manager.is_open() {
            if let Some(cmd) = self.save_manager.escape() {
                return self.handle_save_manager_command(cmd);
            }
            return ShellOutcome::redraw();
        }
        if self.import_picker.is_open() {
            if let Some(cmd) = self.import_picker.escape() {
                return self.handle_import_picker_command(cmd);
            }
            return ShellOutcome::redraw();
        }
        if self.credits_dialog.is_open() {
            self.close_credits_modal();
            return ShellOutcome::redraw();
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            if let Some(cmd) = self.coop_lobby.escape(&snap) {
                return self.handle_shell_coop_command(cmd);
            }
            return ShellOutcome::redraw();
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            let out = session.game.coop_lobby_escape();
            return self.handle_lot_outcome(out);
        }
        if let Some(td_state) = self.travel_dialog_state.clone() {
            if let Some(cmd) = self.travel_dialog.key_press("Escape", &td_state) {
                return self.handle_travel_dialog_command(cmd);
            }
            self.close_travel_dialog();
            return ShellOutcome::redraw();
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Escape);
        }
        if self.dialog.is_some() {
            return self.key_dialog(DialogKey::Escape);
        }
        match &mut self.screen {
            ActiveScreen::Loading(_) => ShellOutcome::default(),
            ActiveScreen::Neighborhood => {
                if !matches!(self.nbhd_state.mode, NeighborhoodMode::Normal)
                    || self.travel_state.mode != TravelBarMode::Browse
                {
                    self.nbhd_state.mode = NeighborhoodMode::Normal;
                    self.travel_state.mode = TravelBarMode::Browse;
                    self.move_in_family_id = None;
                    self.refresh_hovered_lot();
                    return ShellOutcome::redraw();
                }
                if destination_for_kind(self.kind).is_some() {
                    let home_kind = default_map_kind(&self.install, self.slot);
                    let _ = self.switch_kind(home_kind);
                    return ShellOutcome::redraw();
                }
                self.open_quit_dialog();
                ShellOutcome::redraw()
            }
            ActiveScreen::FamilyBin => {
                let cmd = self.family_bin.cancel();
                self.handle_family_bin_command(cmd)
            }
            ActiveScreen::CreateFamily(caf) => {
                let outcome = caf.key_escape();
                self.drain_caf_voice_previews();
                self.handle_caf_outcome(outcome)
            }
            ActiveScreen::Lot(session) if session.game.shop_open() => {
                let out = session.game.shop_escape();
                self.handle_lot_outcome(out)
            }
            ActiveScreen::Lot(session) if session.game.book_open() => {
                let out = session.game.book_close();
                self.handle_lot_outcome(out)
            }
            ActiveScreen::Lot(session) if session.game.camera_strip_open() => {
                session.game.close_camera_strip();
                ShellOutcome::redraw()
            }
            ActiveScreen::Lot(_) => ShellOutcome::default(),
        }
    }

    pub fn key_char(&mut self, ch: char) -> ShellOutcome {
        if self.coop_lobby.is_open() {
            return ShellOutcome {
                redraw: self.coop_lobby.insert_char(ch),
                ..ShellOutcome::default()
            };
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            return ShellOutcome {
                redraw: session.game.coop_lobby_char(ch),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Char(ch));
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_char(ch);
            self.drain_caf_voice_previews();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_backspace(&mut self) -> ShellOutcome {
        if self.coop_lobby.is_open() {
            return ShellOutcome {
                redraw: self.coop_lobby.backspace(),
                ..ShellOutcome::default()
            };
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            return ShellOutcome {
                redraw: session.game.coop_lobby_backspace(),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Backspace);
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_backspace();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_delete(&mut self) -> ShellOutcome {
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Delete);
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_delete();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_left(&mut self) -> ShellOutcome {
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            return ShellOutcome {
                redraw: self.credits_dialog.prev_page(snapshot).is_some(),
                ..ShellOutcome::default()
            };
        }
        if self.travel_dialog_state.is_some() {
            return ShellOutcome {
                redraw: self.cycle_travel_destination(-1),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Left);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.book_open() {
                return ShellOutcome {
                    redraw: session.game.book_prev_page(),
                    ..ShellOutcome::default()
                };
            }
            if session.game.shop_open() {
                let out = session.game.shop_rotate_preview(-45);
                return self.handle_lot_outcome(out);
            }
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_left();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_right(&mut self) -> ShellOutcome {
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            return ShellOutcome {
                redraw: self.credits_dialog.next_page(snapshot).is_some(),
                ..ShellOutcome::default()
            };
        }
        if self.travel_dialog_state.is_some() {
            return ShellOutcome {
                redraw: self.cycle_travel_destination(1),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Right);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.book_open() {
                return ShellOutcome {
                    redraw: session.game.book_next_page(),
                    ..ShellOutcome::default()
                };
            }
            if session.game.shop_open() {
                let out = session.game.shop_rotate_preview(45);
                return self.handle_lot_outcome(out);
            }
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_right();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_up(&mut self) -> ShellOutcome {
        if self.save_manager.is_open()
            && let Some(snapshot) = &self.save_manager_snapshot
        {
            return ShellOutcome {
                redraw: self.save_manager.select_prev(snapshot),
                ..ShellOutcome::default()
            };
        }
        if self.import_picker.is_open()
            && let Some(snapshot) = &self.import_picker_snapshot
        {
            return ShellOutcome {
                redraw: self.import_picker.select_prev(snapshot),
                ..ShellOutcome::default()
            };
        }
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            let frame_size = self.frame_size_u32();
            return ShellOutcome {
                redraw: self.credits_dialog.scroll_vertical(
                    -3,
                    frame_size,
                    snapshot,
                    &self.save_manager_art,
                ),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Up);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.book_open() {
                return ShellOutcome {
                    redraw: session.game.book_scroll_lines(-3),
                    ..ShellOutcome::default()
                };
            }
            if session.game.shop_open() {
                return ShellOutcome {
                    redraw: session.game.shop_select_prev(),
                    ..ShellOutcome::default()
                };
            }
        }
        ShellOutcome::default()
    }

    pub fn key_down(&mut self) -> ShellOutcome {
        if self.save_manager.is_open()
            && let Some(snapshot) = &self.save_manager_snapshot
        {
            return ShellOutcome {
                redraw: self.save_manager.select_next(snapshot),
                ..ShellOutcome::default()
            };
        }
        if self.import_picker.is_open()
            && let Some(snapshot) = &self.import_picker_snapshot
        {
            return ShellOutcome {
                redraw: self.import_picker.select_next(snapshot),
                ..ShellOutcome::default()
            };
        }
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            let frame_size = self.frame_size_u32();
            return ShellOutcome {
                redraw: self.credits_dialog.scroll_vertical(
                    3,
                    frame_size,
                    snapshot,
                    &self.save_manager_art,
                ),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Down);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.book_open() {
                return ShellOutcome {
                    redraw: session.game.book_scroll_lines(3),
                    ..ShellOutcome::default()
                };
            }
            if session.game.shop_open() {
                return ShellOutcome {
                    redraw: session.game.shop_select_next(),
                    ..ShellOutcome::default()
                };
            }
        }
        ShellOutcome::default()
    }

    pub fn key_home(&mut self) -> ShellOutcome {
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Home);
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_home();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_end(&mut self) -> ShellOutcome {
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::End);
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_end();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_tab(&mut self) -> ShellOutcome {
        if self.coop_lobby.is_open() {
            let next = match self.coop_lobby.tab() {
                LobbyTab::Host => LobbyTab::Join,
                LobbyTab::Join => LobbyTab::Host,
            };
            self.coop_lobby.set_tab(next);
            return ShellOutcome::redraw();
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            return ShellOutcome {
                redraw: session.game.coop_lobby_toggle_tab(),
                ..ShellOutcome::default()
            };
        }
        if self.travel_dialog_state.is_some() {
            return ShellOutcome {
                redraw: self.cycle_travel_destination(1),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Tab);
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_tab();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_page_up(&mut self) -> ShellOutcome {
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            return ShellOutcome {
                redraw: self.credits_dialog.prev_page(snapshot).is_some(),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::PageUp);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.book_open()
        {
            return ShellOutcome {
                redraw: session.game.book_prev_page(),
                ..ShellOutcome::default()
            };
        }
        ShellOutcome::default()
    }

    pub fn key_page_down(&mut self) -> ShellOutcome {
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            return ShellOutcome {
                redraw: self.credits_dialog.next_page(snapshot).is_some(),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::PageDown);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.book_open()
        {
            return ShellOutcome {
                redraw: session.game.book_next_page(),
                ..ShellOutcome::default()
            };
        }
        ShellOutcome::default()
    }

    pub fn key_save_shortcut(&mut self) -> ShellOutcome {
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::SaveShortcut);
        }
        ShellOutcome::default()
    }

    pub fn key_enter(&mut self) -> ShellOutcome {
        if self.save_manager.is_open() {
            let cmd = self
                .save_manager_snapshot
                .as_ref()
                .and_then(|snap| self.save_manager.submit(snap));
            return match cmd {
                Some(cmd) => self.handle_save_manager_command(cmd),
                None => ShellOutcome::default(),
            };
        }
        if self.import_picker.is_open() {
            let cmd = self
                .import_picker_snapshot
                .as_ref()
                .and_then(|snap| self.import_picker.submit(snap));
            return match cmd {
                Some(cmd) => self.handle_import_picker_command(cmd),
                None => ShellOutcome::default(),
            };
        }
        if self.credits_dialog.is_open() {
            self.close_credits_modal();
            return ShellOutcome::redraw();
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            return match self.coop_lobby.submit(&snap) {
                Some(cmd) => self.handle_shell_coop_command(cmd),
                None => ShellOutcome::default(),
            };
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            let out = session.game.coop_lobby_submit();
            return self.handle_lot_outcome(out);
        }
        if let Some(td_state) = self.travel_dialog_state.clone() {
            if let Some(cmd) = self.travel_dialog.key_press("Enter", &td_state) {
                return self.handle_travel_dialog_command(cmd);
            }
            return ShellOutcome::default();
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Enter);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.shop_open() {
                let out = session.game.shop_submit();
                return self.handle_lot_outcome(out);
            }
            if session.game.book_open() {
                let out = session.game.book_close();
                return self.handle_lot_outcome(out);
            }
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_enter();
            self.drain_caf_voice_previews();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    /// Handles a [`crate::game::Outcome`] produced by an input to the active lot
    /// (including Save, Save Manager, Photo Album, Save & Leave to Neighborhood,
    /// Quit, and Photo).
    pub fn handle_lot_outcome(&mut self, outcome: crate::game::Outcome) -> ShellOutcome {
        self.persist_options();
        let mut out = ShellOutcome {
            redraw: outcome.redraw,
            quit: false,
            photo: outcome.photo,
        };
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.asking().is_none()
            && session.game.asking_special_panel().is_none()
            && let Some((request, dest_lot)) = session.game.take_pending_travel_lot()
        {
            self.handle_special_panel_travel(request, dest_lot);
            out.redraw = true;
            return out;
        }
        if outcome.open_save_manager {
            self.open_save_manager(None);
            out.redraw = true;
            return out;
        }
        if outcome.open_coop_lobby {
            if let ActiveScreen::Lot(session) = &mut self.screen {
                session.game.open_coop_lobby(None);
            }
            out.redraw = true;
            return out;
        }
        if outcome.open_photo_album {
            if let Err(err) = self.open_photo_album()
                && let ActiveScreen::Lot(session) = &mut self.screen
            {
                session
                    .game
                    .show_notice("Photo Album", format!("Photo Album unavailable: {err}"));
            }
            out.redraw = true;
            return out;
        }
        if outcome.neighborhood {
            match self.leave_lot(outcome.save) {
                Ok(()) => {
                    out.redraw = true;
                }
                Err(e) => {
                    if let ActiveScreen::Lot(session) = &mut self.screen {
                        session.game.after_save_failed();
                        session.game.show_notice(
                            "Neighborhood",
                            format!("Could not return to Neighborhood: {e}"),
                        );
                    }
                    out.redraw = true;
                }
            }
            return out;
        }
        let save_err = if outcome.save {
            self.save_current_lot().err()
        } else {
            None
        };
        let saved_ok = save_err.is_none();
        if let Some(err) = save_err
            && let ActiveScreen::Lot(session) = &mut self.screen
        {
            session.game.after_save_failed_with_notice(
                "Save Failed",
                format!("The lot was not saved: {err}"),
            );
            out.redraw = true;
        }
        if outcome.quit && saved_ok {
            self.leave_coop_if_active();
            self.flush_background_saves();
            out.quit = true;
        }
        out
    }

    /// Enqueues a non-blocking CoW [`sims_sim::World`] snapshot onto the
    /// background [`AutosaveWorker`] with `expected_instance_id` fencing.
    pub fn enqueue_background_world_save(&mut self, target: SaveTarget) -> Result<(), String> {
        let ActiveScreen::Lot(session) = &self.screen else {
            return Err("no lot is currently open".into());
        };
        if session.game.coop_blocks_local_save() {
            return Err("Only the co-op host can save the lot.".into());
        }
        if session.active_trip.is_some() {
            return Err(
                "cannot save directly while visiting a travel destination; return home first"
                    .into(),
            );
        }
        let world = session.game.lot().world().clone();
        let label = self.active_lot_save_label(session);
        self.autosave_worker.enqueue_world(
            session.neighborhood_id.clone(),
            session.neighborhood_instance_id,
            session.house,
            target,
            u64::from(session.game.lot().ticks()),
            label,
            world,
        )
    }

    /// Enqueues a non-blocking CoW [`Saved`] lot snapshot onto the background
    /// [`AutosaveWorker`] (encoding runs off-thread).
    pub fn enqueue_background_lot_save(&mut self, target: SaveTarget) -> Result<(), String> {
        let ActiveScreen::Lot(session) = &self.screen else {
            return Err("no lot is currently open".into());
        };
        if session.game.coop_blocks_local_save() {
            return Err("Only the co-op host can save the lot.".into());
        }
        if session.active_trip.is_some() {
            return Err(
                "cannot save directly while visiting a travel destination; return home first"
                    .into(),
            );
        }
        let saved = session.game.save()?;
        let label = self.active_lot_save_label(session);
        self.autosave_worker.enqueue_saved_lot(
            session.neighborhood_id.clone(),
            session.neighborhood_instance_id,
            session.house,
            target,
            u64::from(session.game.lot().ticks()),
            label,
            saved,
        )
    }

    fn active_lot_save_label(&self, session: &LotSession) -> String {
        let world = session.game.lot().world();
        let hour = session.game.hour().min(23) as u8;
        let minute = world.globals.get(5).copied().unwrap_or(0).clamp(0, 59) as u8;
        let sim_day = (world.globals.get(28).copied().unwrap_or(0).max(0) as u32) + 1;
        let fam_name = self
            .stored
            .family_in(session.house)
            .and_then(|(id, _)| self.family_names.get(&id))
            .map(String::as_str);
        format_save_label(fam_name, sim_day, hour, minute)
    }

    /// Polls background save completions and triggers a periodic rotating
    /// autosave whenever the active lot's VM `days_played` (`world.globals[28]`)
    /// advances across midnight.
    /// Returns `true` if status feedback changed and the screen should redraw.
    pub fn poll_background_saves(&mut self) -> bool {
        let mut redraw = false;
        let mut refresh_manager = false;
        let completions = self.autosave_worker.poll_completions();
        for completion in completions {
            if self.save_manager.is_open()
                && self.save_manager_target_lot == Some(completion.lot_id)
                && completion.neighborhood_id == self.neighborhood_id
            {
                refresh_manager = true;
            }
            match &completion.result {
                Ok(meta) => {
                    if completion.target == SaveTarget::Autosave
                        && let ActiveScreen::Lot(session) = &mut self.screen
                        && session.house == completion.lot_id
                        && session.game.note().is_none()
                    {
                        session.game.set_note(format!(
                            "Autosaved ({}) at tick {}.",
                            meta.slot, meta.sim_tick
                        ));
                        redraw = true;
                    }
                }
                Err(err) => {
                    let msg = format!("Background save warning (existing saves preserved): {err}");
                    if let ActiveScreen::Lot(session) = &mut self.screen {
                        session.game.set_note(msg);
                    } else {
                        self.note = Some(msg);
                    }
                    redraw = true;
                }
            }
            if self.last_save_completions.len() >= 64 {
                self.last_save_completions.remove(0);
            }
            self.last_save_completions.push(completion);
        }

        if refresh_manager && let Some(lot_id) = self.save_manager_target_lot {
            let active_err = self
                .save_manager_snapshot
                .as_ref()
                .and_then(|s| s.active_error.clone());
            let label = self.format_lot_label_for_saves(lot_id);
            self.save_manager_snapshot = Some(build_save_manager_snapshot(
                &self.store.autosaves(),
                &self.neighborhood_id,
                self.slot,
                lot_id,
                label,
                active_err,
            ));
            redraw = true;
        }

        // Periodic midnight rotating autosave (triggered strictly when VM
        // `daily::turn_day` increments `world.globals[28]`). Skip during active
        // travel visits so destination community templates are never overwritten.
        let trigger_midnight = if let ActiveScreen::Lot(session) = &mut self.screen {
            let days_played = session
                .game
                .lot()
                .world()
                .globals
                .get(28)
                .copied()
                .unwrap_or(0);
            let turned = session.active_trip.is_none()
                && !session.game.coop_blocks_local_save()
                && days_played > session.last_days_played;
            session.last_days_played = days_played;
            turned
        } else {
            false
        };
        if trigger_midnight && let Err(err) = self.enqueue_background_lot_save(SaveTarget::Autosave)
        {
            if let ActiveScreen::Lot(session) = &mut self.screen {
                session.game.set_note(format!("Autosave skipped: {err}"));
            }
            redraw = true;
        }
        redraw
    }

    /// Flushes all pending background save/autosave jobs and drains completions.
    pub fn flush_background_saves(&mut self) {
        self.autosave_worker.flush();
        let _ = self.poll_background_saves();
    }

    /// Returns completed background save results observed since the last drain.
    pub fn take_save_completions(&mut self) -> Vec<SaveCompletion> {
        std::mem::take(&mut self.last_save_completions)
    }

    /// Opens the crash-safe [`SaveManagerDialog`] for the currently active lot
    /// (or hovered/last lot), flushing pending background saves first so every
    /// manual and rotating autosave slot is shown.
    pub fn open_save_manager(&mut self, active_error: Option<String>) -> bool {
        self.flush_background_saves();
        let lot_id = match &self.screen {
            ActiveScreen::Lot(session) => {
                if let Some(trip) = &session.active_trip {
                    Some(trip.home_house)
                } else {
                    Some(session.house)
                }
            }
            _ => self
                .save_manager_target_lot
                .or_else(|| self.hovered_lot_id.map(u32::from)),
        };
        let Some(lot_id) = lot_id else {
            self.note = Some("Enter a lot to manage its saves.".into());
            return false;
        };
        self.save_manager_target_lot = Some(lot_id);
        let lot_label = self.format_lot_label_for_saves(lot_id);
        let snapshot = build_save_manager_snapshot(
            &self.store.autosaves(),
            &self.neighborhood_id,
            self.slot,
            lot_id,
            lot_label,
            active_error,
        );
        self.clear_hover_state();
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.close_camera_strip();
        }
        self.save_manager_snapshot = Some(snapshot);
        self.save_manager.open();
        true
    }

    /// Closes the [`SaveManagerDialog`] without modifying any save slot.
    pub fn close_save_manager(&mut self) {
        self.save_manager.close();
        self.save_manager_snapshot = None;
    }

    fn format_lot_label_for_saves(&self, lot_id: u32) -> String {
        let fam = self
            .stored
            .family_in(lot_id)
            .and_then(|(id, _)| self.family_names.get(&id))
            .cloned();
        match fam {
            Some(name) => format!("Neighborhood {} · Lot {lot_id} ({name})", self.slot),
            None => format!("Neighborhood {} · Lot {lot_id}", self.slot),
        }
    }

    /// Executes a [`SaveManagerCommand`] emitted by [`SaveManagerDialog`]
    /// (`LoadSlot`, `FallbackToNewestIntact`, `DeleteSlot`, or `Cancel`).
    pub fn handle_save_manager_command(&mut self, cmd: SaveManagerCommand) -> ShellOutcome {
        // Always target the lot displayed in Save Manager (e.g. the home house
        // during a live travel visit), never overriding it with a destination lot.
        let Some(lot_id) = self
            .save_manager_target_lot
            .or_else(|| self.active_lot_house())
        else {
            self.close_save_manager();
            return ShellOutcome::redraw();
        };
        match cmd {
            SaveManagerCommand::Cancel => {
                self.close_save_manager();
                ShellOutcome::redraw()
            }
            SaveManagerCommand::DeleteSlot { slot_id } => {
                let err = match parse_slot_id(&slot_id) {
                    Some(slot) => self
                        .store
                        .autosaves()
                        .delete_slot(&self.neighborhood_id, lot_id, slot)
                        .err()
                        .map(|e| format!("Could not delete save slot: {e}")),
                    None => Some(format!("Unknown save slot identifier: {slot_id}")),
                };
                let label = self.format_lot_label_for_saves(lot_id);
                self.save_manager_snapshot = Some(build_save_manager_snapshot(
                    &self.store.autosaves(),
                    &self.neighborhood_id,
                    self.slot,
                    lot_id,
                    label,
                    err,
                ));
                ShellOutcome::redraw()
            }
            SaveManagerCommand::LoadSlot { slot_id } => {
                let result = if self.active_trip().is_some() {
                    Err(
                        "Cannot load a save slot while visiting a travel destination; return home first."
                            .to_owned(),
                    )
                } else {
                    parse_slot_id(&slot_id)
                        .ok_or_else(|| format!("unknown save slot {slot_id:?}"))
                        .and_then(|slot| self.load_autosave_slot_into_lot(lot_id, slot))
                };
                match result {
                    Ok(tick) => {
                        self.close_save_manager();
                        if let ActiveScreen::Lot(session) = &mut self.screen {
                            session
                                .game
                                .set_note(format!("Loaded save slot {slot_id} (tick {tick})."));
                        }
                    }
                    Err(err) => {
                        // Non-destructive: keep dialog open with warning banner & fallback.
                        let label = self.format_lot_label_for_saves(lot_id);
                        self.save_manager_snapshot = Some(build_save_manager_snapshot(
                            &self.store.autosaves(),
                            &self.neighborhood_id,
                            self.slot,
                            lot_id,
                            label,
                            Some(err),
                        ));
                    }
                }
                ShellOutcome::redraw()
            }
            SaveManagerCommand::FallbackToNewestIntact { slot_id } => {
                let result = if self.active_trip().is_some() {
                    Err(
                        "Cannot load a save slot while visiting a travel destination; return home first."
                            .to_owned(),
                    )
                } else {
                    // Always consult current REDB metadata first and validate
                    // candidates in descending `(sim_tick, saved_at_unix_secs, slot)`
                    // order so a newer save committed after the UI snapshot is
                    // never bypassed in favor of an older slot.
                    let mut candidates: Vec<sims_sim::autosave::SaveMetadata> = self
                        .store
                        .autosaves()
                        .list_saves_metadata(&self.neighborhood_id, lot_id)
                        .map_err(|e| e.to_string())
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|entry| match entry {
                            sims_sim::autosave::SaveSlotEntry::Intact(meta) => Some(meta),
                            sims_sim::autosave::SaveSlotEntry::Corrupt { .. } => None,
                        })
                        .collect();
                    candidates.sort_by(|a, b| {
                        let key_a = match a.slot {
                            SaveSlot::Manual => (1u8, 0u64),
                            SaveSlot::Autosave(seq) => (0u8, seq),
                        };
                        let key_b = match b.slot {
                            SaveSlot::Manual => (1u8, 0u64),
                            SaveSlot::Autosave(seq) => (0u8, seq),
                        };
                        b.sim_tick
                            .cmp(&a.sim_tick)
                            .then_with(|| b.saved_at_unix_secs.cmp(&a.saved_at_unix_secs))
                            .then_with(|| key_b.cmp(&key_a))
                    });
                    let mut ordered_slots: Vec<SaveSlot> =
                        candidates.into_iter().map(|m| m.slot).collect();
                    if let Some(hint_slot) = parse_slot_id(&slot_id)
                        && !ordered_slots.contains(&hint_slot)
                    {
                        ordered_slots.push(hint_slot);
                    }
                    let mut recovered_tick = None;
                    let mut last_err = format!("no intact save slots available for lot {lot_id}");
                    for slot in ordered_slots {
                        match self.load_autosave_slot_into_lot(lot_id, slot) {
                            Ok(tick) => {
                                recovered_tick = Some(tick);
                                break;
                            }
                            Err(err) => {
                                last_err = err;
                            }
                        }
                    }
                    recovered_tick.ok_or(last_err)
                };
                match result {
                    Ok(tick) => {
                        self.close_save_manager();
                        if let ActiveScreen::Lot(session) = &mut self.screen {
                            session.game.set_note(format!(
                                "Recovered newest intact save (tick {tick}); corrupt saves preserved."
                            ));
                        }
                    }
                    Err(err) => {
                        let label = self.format_lot_label_for_saves(lot_id);
                        self.save_manager_snapshot = Some(build_save_manager_snapshot(
                            &self.store.autosaves(),
                            &self.neighborhood_id,
                            self.slot,
                            lot_id,
                            label,
                            Some(format!("Recovery failed: {err}")),
                        ));
                    }
                }
                ShellOutcome::redraw()
            }
        }
    }

    fn load_autosave_slot_into_lot(
        &mut self,
        lot_id: u32,
        slot: sims_sim::autosave::SaveSlot,
    ) -> Result<u64, String> {
        let loaded = self
            .store
            .autosaves()
            .load_save_validated(&self.neighborhood_id, lot_id, slot, validate_save_payload)
            .map_err(|e| e.to_string())?;
        self.apply_loaded_save_into_lot(lot_id, &loaded)
    }

    fn apply_loaded_save_into_lot(
        &mut self,
        lot_id: u32,
        loaded: &sims_sim::autosave::LoadedSave,
    ) -> Result<u64, String> {
        if matches!(&self.screen, ActiveScreen::Lot(s) if s.game.coop_active()) {
            return Err("Leave the co-op session before loading a local save slot.".into());
        }
        if self.active_trip().is_some() {
            return Err(
                "cannot restore a save slot while visiting a travel destination; return home first"
                    .into(),
            );
        }
        let snapshot = self
            .store
            .load_lot_snapshot(&self.neighborhood_id, lot_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))?;
        let has_trvl_envelope = snapshot
            .saved_lot
            .as_deref()
            .is_some_and(|bytes| bytes.len() >= 4 && &bytes[0..4] == b"TRVL");
        let secondary_dest = snapshot
            .neighborhood
            .family_in(lot_id)
            .map(|(_, f)| f.secondary_house())
            .filter(|&d| d != 0);
        if has_trvl_envelope || secondary_dest.is_some() {
            let dest_detail = match secondary_dest {
                Some(dest) => format!(" to lot {dest}"),
                None => String::new(),
            };
            return Err(format!(
                "lot {lot_id} has an active travel trip{dest_detail}; resume or abort the trip before restoring a save slot"
            ));
        }
        let real_instance_id = snapshot.neighborhood.instance_id;
        let real_revision = snapshot.neighborhood.revision;

        // Boot through an ephemeral staging NeighborhoodStore populated with the
        // CURRENT StoredNeighborhood (all newer CAF families, NBRS, NGBH, TATT,
        // made_characters, and deletion_epoch tombstones) + the selected validated
        // save slot. This avoids retrying a corrupt SAVED_LOTS record and runs
        // `sims_vm::lifecycle::enter_lot`'s full neighborhood reconciliation
        // before replacing the live session.
        let staging_dir = TempStagingDir::new("slot-boot")?;
        let staging_store = NeighborhoodStore::open(&staging_dir.path().join("stage.redb"))
            .map_err(|e| e.to_string())?;
        for (file_name, bytes) in &snapshot.character_files {
            staging_store
                .save_character_file(&self.neighborhood_id, file_name, bytes)
                .map_err(|e| e.to_string())?;
        }
        if let Some(base_house) = &snapshot.base_house {
            staging_store
                .save_base_house(&self.neighborhood_id, lot_id, base_house)
                .map_err(|e| e.to_string())?;
        }

        let install_assets = self.ensure_install_assets()?;
        let (staged_saved_bytes, restored_tick) = match &loaded.metadata.payload_kind {
            SavePayloadKind::SavedLot => {
                let _ = Saved::decode(&loaded.payload).map_err(|e| e.to_string())?;
                (loaded.payload.clone(), None)
            }
            SavePayloadKind::WorldSnapshot => {
                let world = sims_sim::World::decode(&loaded.payload).map_err(|e| e.to_string())?;
                // Obtain a compatible base `Saved` envelope without touching a
                // potentially corrupt SAVED_LOTS entry: reuse the open lot if it
                // matches `lot_id`, otherwise boot the clean base house in staging.
                let base_saved = match &self.screen {
                    ActiveScreen::Lot(session) if session.house == lot_id => session.game.save()?,
                    _ => {
                        staging_store
                            .save_neighborhood(&self.neighborhood_id, &snapshot.neighborhood)
                            .map_err(|e| e.to_string())?;
                        let base_game = Game::open_from_store_cached(
                            &self.install_root,
                            &staging_store,
                            &self.neighborhood_id,
                            lot_id,
                            &self.answers,
                            self.play,
                            self.frames.clone(),
                            Some(&install_assets),
                        )?;
                        base_game.save()?
                    }
                };
                let spliced = splice_world_into_saved_bytes(&base_saved, &world)?;
                (spliced, Some(loaded.metadata.sim_tick))
            }
            SavePayloadKind::Custom(tag) => {
                return Err(format!("unsupported custom payload kind {tag:?}"));
            }
        };

        // Keep `snapshot.neighborhood` unmodified so restoring an older slot
        // reconciles against the live neighborhood's current FAMI budgets, NBRS,
        // made_characters, and deletion_epoch without rolling any of them back.
        staging_store
            .save_lot_and_neighborhood(
                &self.neighborhood_id,
                lot_id,
                &staged_saved_bytes,
                &snapshot.neighborhood,
            )
            .map_err(|e| e.to_string())?;

        let mut game = Game::open_from_store_cached(
            &self.install_root,
            &staging_store,
            &self.neighborhood_id,
            lot_id,
            &self.answers,
            self.play,
            self.frames.clone(),
            Some(&install_assets),
        )?;
        if self.options_file.is_file() {
            let _ = game.load_options(&self.options_file);
        }
        game.set_album_button_enabled(true);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let ticks = restored_tick.unwrap_or_else(|| u64::from(game.lot().ticks()));
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());

        // Replace live session only after the reconciled boot succeeded.
        self.stored = snapshot.neighborhood;
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: self.neighborhood_id.clone(),
            neighborhood_instance_id: real_instance_id,
            neighborhood_revision: real_revision,
            staging: Some(StagingStoreHandle {
                _dir: staging_dir,
                store: staging_store,
            }),
            house: lot_id,
            last_days_played: initial_days_played,
            active_trip: None,
            is_destination_edit: false,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.dialog = None;
        self.save_manager_target_lot = Some(lot_id);
        self.note = None;
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
        self.pressed_lot_id = None;
        self.nbhd_state.hovered_lot = None;
        self.audio_queue
            .push(Input::Events(vec![SoundEvent::StopAll]));
        self.audio_queue.push(Input::MusicMode(next_music));
        Ok(ticks)
    }

    /// Saves the currently open lot to REDB (F5 or Options → Save): performs the
    /// strictly fenced `SAVED_LOTS` + neighborhood writeback commit first, then
    /// mirrors the exact committed `Saved` bytes to the background
    /// [`AutosaveWorker`] manual slot and updates the cached neighborhood map
    /// thumbnail immediately.
    pub fn save_current_lot(&mut self) -> Result<SaveReport, String> {
        let (report, committed_bytes, nh_id, expected_instance_id, house, ticks, label) = {
            let ActiveScreen::Lot(session) = &mut self.screen else {
                return Err("no lot is currently open".into());
            };
            if session.game.coop_blocks_local_save() {
                let msg = "Only the co-op host can save the lot.";
                session.game.set_note(msg);
                return Err(msg.into());
            }
            if session.active_trip.is_some() {
                let msg =
                    "Cannot save directly while visiting a travel destination; return home first.";
                session.game.set_note(msg);
                return Err(msg.into());
            }
            let (report, lot_bytes) = if session.game.world_mutated_via_restore() {
                save_cheat_mutated_lot_to_store(
                    &self.install_root,
                    &self.store,
                    &session.neighborhood_id,
                    session.neighborhood_instance_id,
                    session.neighborhood_revision,
                    session.house,
                    self.arch_multiplier,
                    &self.answers,
                    self.frames.clone(),
                    &mut session.game,
                )?
            } else if let Some(staging) = &session.staging {
                save_staged_lot_to_store(
                    &self.install_root,
                    &self.store,
                    &staging.store,
                    &session.neighborhood_id,
                    session.neighborhood_instance_id,
                    session.neighborhood_revision,
                    session.house,
                    &mut session.game,
                )?
            } else {
                // Propagate fencing / stale-revision / replacement errors directly
                // instead of bypassing them with an unfenced fallback.
                let report = session.game.save_to_neighborhood(
                    &self.install_root,
                    &self.store,
                    &session.neighborhood_id,
                )?;
                let bytes = self
                    .store
                    .load_saved_lot(&session.neighborhood_id, session.house)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "committed saved lot missing after save".to_owned())?;
                (report, bytes)
            };
            session.neighborhood_revision = report.revision;
            let nh_id = session.neighborhood_id.clone();
            let inst = session.neighborhood_instance_id;
            let house = session.house;
            let ticks = u64::from(session.game.lot().ticks());
            let world = session.game.lot().world();
            let hour = session.game.hour().min(23) as u8;
            let minute = world.globals.get(5).copied().unwrap_or(0).clamp(0, 59) as u8;
            let sim_day = (world.globals.get(28).copied().unwrap_or(0).max(0) as u32) + 1;
            let fam_name = self
                .stored
                .family_in(house)
                .and_then(|(id, _)| self.family_names.get(&id))
                .map(String::as_str);
            let label = format_save_label(fam_name, sim_day, hour, minute);
            (report, lot_bytes, nh_id, inst, house, ticks, label)
        };
        if let Ok(Some(reloaded)) = self.store.load_neighborhood(&nh_id) {
            self.stored = reloaded;
        } else {
            self.stored.revision = report.revision;
        }
        // Mirror the exact committed `Saved` bytes into the crash-safe manual slot
        // on the background worker so F9 and Save Manager share one source of truth.
        if let Err(err) = self.autosave_worker.enqueue_saved_lot_bytes(
            nh_id,
            expected_instance_id,
            house,
            SaveTarget::Manual,
            ticks,
            label,
            committed_bytes,
        ) && let ActiveScreen::Lot(session) = &mut self.screen
        {
            session
                .game
                .set_note(format!("Saved lot; manual slot mirror warning: {err}"));
        }
        self.capture_active_lot_thumbnail();
        Ok(report)
    }

    fn capture_active_lot_thumbnail(&mut self) {
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.active_trip.is_none()
            && let Ok(lot_id) = u16::try_from(session.house)
        {
            let target_size_and_cutaway = self
                .map
                .lot(lot_id)
                .map(|m| ([m.normal.width, m.normal.height], m.roofs_off.is_some()))
                .or_else(|| {
                    let kind = map_kind_for_house(&self.install, self.slot, session.house);
                    load_map_for_slot(&self.install, self.slot, kind)
                        .ok()
                        .and_then(|m| {
                            m.lot(lot_id)
                                .map(|l| ([l.normal.width, l.normal.height], l.roofs_off.is_some()))
                        })
                });
            if let Some((normal_size, has_cutaway)) = target_size_and_cutaway {
                let (world, lot_shape, assets) = session.game.render_parts_mut();
                let thumb =
                    capture_lot_thumbnail(world, lot_shape, assets, normal_size, has_cutaway);
                let stored_thumb = StoredLotThumbnail {
                    normal: to_stored_thumb_surface(&thumb.normal),
                    roofs_off: thumb.roofs_off.as_ref().map(to_stored_thumb_surface),
                };
                match self.store.save_lot_thumbnail_checked(
                    &session.neighborhood_id,
                    session.neighborhood_instance_id,
                    session.house,
                    &stored_thumb,
                ) {
                    Ok(Ok(())) => {
                        self.custom_thumbs
                            .insert((session.neighborhood_id.clone(), lot_id), thumb);
                    }
                    Ok(Err(commit_err)) => {
                        session.game.set_note(format!(
                            "Saved lot; map thumbnail was not updated ({commit_err:?})."
                        ));
                    }
                    Err(err) => {
                        session
                            .game
                            .set_note(format!("Saved lot; map thumbnail update warning: {err}"));
                    }
                }
            }
        }
    }

    /// Reloads the currently open lot from its latest REDB manual save (F9),
    /// preferring the primary committed `SAVED_LOTS` record whenever it is
    /// intact (so a newer F5 commit is never shadowed by a stale manual mirror)
    /// and falling back non-destructively to [`SaveManagerDialog`] if either
    /// the manual slot or `SAVED_LOTS` is corrupt.
    pub fn restore_current_lot(&mut self) -> Result<(), String> {
        self.flush_background_saves();
        let (neighborhood_id, house) = match &mut self.screen {
            ActiveScreen::Lot(session) => {
                if session.game.coop_active() {
                    let msg = "Leave the co-op session before restoring a local save.";
                    session.game.set_note(msg);
                    return Err(msg.into());
                }
                if session.active_trip.is_some() {
                    let msg = "Cannot restore directly while visiting a travel destination; return home first.";
                    session.game.set_note(msg);
                    return Err(msg.into());
                }
                (session.neighborhood_id.clone(), session.house)
            }
            _ => return Err("no lot is currently open".into()),
        };
        let manual_slot = self.store.autosaves().load_save_validated(
            &neighborhood_id,
            house,
            sims_sim::autosave::SaveSlot::Manual,
            validate_save_payload,
        );
        let manual_loaded = match manual_slot {
            Ok(loaded) => Some(loaded),
            Err(sims_sim::autosave::SaveError::Missing { .. }) => None,
            Err(corrupt_err) => {
                let msg = format!(
                    "Manual save could not be loaded ({corrupt_err}). Choose an intact autosave below."
                );
                self.open_save_manager(Some(msg.clone()));
                return Err(msg);
            }
        };
        let saved_lot_bytes = self
            .store
            .load_saved_lot(&neighborhood_id, house)
            .map_err(|e| e.to_string())?;
        if let Some(bytes) = saved_lot_bytes {
            if Saved::decode(&bytes).is_ok() {
                match self.enter_lot(house) {
                    Ok(()) => {
                        if let ActiveScreen::Lot(session) = &mut self.screen {
                            let ticks = session.game.lot().ticks();
                            session
                                .game
                                .set_note(format!("Back to the save after tick {ticks}."));
                        }
                        return Ok(());
                    }
                    Err(err) => {
                        self.open_save_manager(Some(format!(
                            "Saved lot could not be restored ({err}). Choose an intact autosave below."
                        )));
                        return Err(err);
                    }
                }
            } else if let Some(loaded) = manual_loaded {
                // Primary SAVED_LOTS is unreadable, but the validated Manual slot is intact.
                return match self.apply_loaded_save_into_lot(house, &loaded) {
                    Ok(ticks) => {
                        if let ActiveScreen::Lot(session) = &mut self.screen {
                            session
                                .game
                                .set_note(format!("Back to the save after tick {ticks}."));
                        }
                        Ok(())
                    }
                    Err(err) => {
                        self.open_save_manager(Some(format!(
                            "Manual save could not be restored ({err}). Choose an intact autosave below."
                        )));
                        Err(err)
                    }
                };
            } else {
                let msg =
                    "Saved lot is corrupt and no intact manual slot is available. Choose an intact autosave below."
                        .to_owned();
                self.open_save_manager(Some(msg.clone()));
                return Err(msg);
            }
        }
        if let Some(loaded) = manual_loaded {
            return match self.apply_loaded_save_into_lot(house, &loaded) {
                Ok(ticks) => {
                    if let ActiveScreen::Lot(session) = &mut self.screen {
                        session
                            .game
                            .set_note(format!("Back to the save after tick {ticks}."));
                    }
                    Ok(())
                }
                Err(err) => {
                    self.open_save_manager(Some(format!(
                        "Manual save could not be restored ({err}). Choose an intact autosave below."
                    )));
                    Err(err)
                }
            };
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session
                .game
                .set_note("The save was not restored: nothing is saved for this house");
        }
        Err("nothing is saved for this house".into())
    }

    /// Switches to neighborhood slot `1..=8`, importing it into REDB on first visit.
    pub fn switch_neighborhood(&mut self, slot: u8) -> Result<(), String> {
        let slot = slot.clamp(MIN_NEIGHBORHOOD, MAX_NEIGHBORHOOD);
        let kind = default_map_kind(&self.install, slot);
        let id = neighborhood_id_for_slot(slot);
        let stored = ensure_imported(&self.install, &self.store, &id)?;
        let _ = self.store.set_active_neighborhood(&id);
        let mut map = load_map_for_slot(&self.install, slot, kind)?;
        let user_dir = resolve_user_dir(&self.install, slot)?;
        let family_names = load_family_names_with_caf(
            &user_dir,
            &self.saves_path,
            &id,
            stored.instance_id,
            &stored,
        );
        let street_names = StreetNames::load(&self.install, &user_dir);
        let descriptions = NeighborhoodDescriptions::load(&self.install, &user_dir);
        let (faces, base_cat) = load_neighborhood_faces_cached(
            &self.install,
            &self.store,
            &id,
            &stored,
            &self.pool,
            self.base_index.as_ref(),
            self.base_catalog.as_ref(),
        );
        if self.base_catalog.is_none() {
            self.base_catalog = base_cat;
        }
        let lot_meta = build_lot_meta(&self.store, &id, &map, &descriptions, self.arch_multiplier);
        let custom_thumbs = load_persisted_custom_thumbs(&self.store, &id);
        sync_map_with_store(&mut map, &stored, &lot_meta, &id, &custom_thumbs);
        let family_bin_entries = build_family_bin_entries(&stored, &family_names, &faces);

        self.slot = slot;
        self.neighborhood_id = id;
        self.kind = kind;
        self.map = map;
        self.stored = stored;
        self.family_names = family_names;
        self.street_names = street_names;
        self.descriptions = descriptions;
        self.faces = faces;
        self.lot_meta = lot_meta;
        self.custom_thumbs = custom_thumbs;
        self.family_bin_entries = family_bin_entries;
        self.family_bin
            .sync(&self.family_bin_entries, sims_ui::family_bin::VISIBLE_ROWS);
        self.nbhd_state.neighborhood_number = slot;
        self.nbhd_state.mode = NeighborhoodMode::Normal;
        self.nbhd_state.hovered_lot = None;
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
        self.pressed_lot_id = None;
        self.move_in_family_id = None;
        self.dialog = None;
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = None;
        self.close_import_family_picker();
        self.close_credits_modal();
        self.nbhd_state.show_credits_button = destination_for_kind(kind).is_none();
        self.refresh_importable_families();
        self.note = None;
        self.screen = ActiveScreen::Neighborhood;
        self.sync_travel_state();
        self.audio_queue
            .push(Input::MusicMode(music_mode_for_kind(kind)));
        Ok(())
    }

    /// Switches between the main neighborhood map and expansion sub-neighborhoods
    /// (Downtown, Vacation Island, Studio Town, Magic Town).
    pub fn switch_kind(&mut self, kind: NeighborhoodKind) -> Result<(), String> {
        let mut map = load_map_for_slot(&self.install, self.slot, kind)?;
        let lot_meta = build_lot_meta(
            &self.store,
            &self.neighborhood_id,
            &map,
            &self.descriptions,
            self.arch_multiplier,
        );
        self.custom_thumbs = load_persisted_custom_thumbs(&self.store, &self.neighborhood_id);
        sync_map_with_store(
            &mut map,
            &self.stored,
            &lot_meta,
            &self.neighborhood_id,
            &self.custom_thumbs,
        );
        self.kind = kind;
        self.map = map;
        self.lot_meta = lot_meta;
        self.nbhd_state.mode = NeighborhoodMode::Normal;
        self.nbhd_state.show_credits_button = destination_for_kind(kind).is_none();
        self.nbhd_state.hovered_lot = None;
        self.close_import_family_picker();
        self.close_credits_modal();
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
        self.pressed_lot_id = None;
        self.move_in_family_id = None;
        self.travel_state.active_filter = None;
        self.travel_state.vacation_lodging_filter = None;
        self.travel_state.mode = TravelBarMode::Browse;
        self.sync_travel_state();
        self.audio_queue
            .push(Input::MusicMode(music_mode_for_kind(kind)));
        Ok(())
    }

    /// Opens the Family Bin ("Select a Family") overlay screen.
    pub fn open_family_bin(&mut self) {
        self.family_bin_entries =
            build_family_bin_entries(&self.stored, &self.family_names, &self.faces);
        self.family_bin.clear_pointer();
        self.family_bin
            .sync(&self.family_bin_entries, sims_ui::family_bin::VISIBLE_ROWS);
        self.dialog = None;
        self.note = None;
        self.screen = ActiveScreen::FamilyBin;
        self.audio_queue
            .push(Input::MusicMode(MusicMode::FamilyBin));
    }

    /// Closes the Family Bin and returns to the Neighborhood map.
    pub fn close_family_bin(&mut self) {
        self.dialog = None;
        self.screen = ActiveScreen::Neighborhood;
        self.audio_queue
            .push(Input::MusicMode(music_mode_for_kind(self.kind)));
    }

    /// Enters Move-In placement mode on the Neighborhood map for bin household `family_id`.
    pub fn start_move_in(&mut self, family_id: u16) -> Result<(), String> {
        let fam = self
            .stored
            .families
            .get(&family_id)
            .ok_or_else(|| format!("family {family_id} not found"))?;
        if fam.house != 0 || fam.secondary_house() != 0 || fam.members.is_empty() {
            return Err(format!("family {family_id} is not in the Family Bin"));
        }
        let family_name = self
            .family_names
            .get(&family_id)
            .cloned()
            .unwrap_or_else(|| format!("Family {family_id}"));
        let family_magic_coins = lifecycle::family_magic_coins(&self.stored, family_id);
        self.move_in_family_id = Some(family_id);
        self.nbhd_state.mode = NeighborhoodMode::MoveIn {
            family_name,
            family_funds: fam.budget,
            family_magic_coins,
        };
        self.screen = ActiveScreen::Neighborhood;
        self.dialog = None;
        self.refresh_hovered_lot();
        self.audio_queue
            .push(Input::MusicMode(music_mode_for_kind(self.kind)));
        Ok(())
    }

    /// Opens the interactive Create-a-Family (`CAF`) screen.
    pub fn open_create_family(&mut self) -> Result<(), String> {
        let ui = UiGraphics::open(&self.install).map_err(|e| e.to_string())?;
        let caf = CafSession::open(&self.install_root, &self.install, &ui)?;
        self.dialog = None;
        self.note = None;
        self.screen = ActiveScreen::CreateFamily(Box::new(caf));
        self.audio_queue
            .push(Input::MusicMode(MusicMode::FamilyBin));
        Ok(())
    }

    fn handle_caf_outcome(&mut self, outcome: CafOutcome) -> ShellOutcome {
        match outcome {
            CafOutcome::Stay { redraw } => ShellOutcome {
                redraw,
                ..ShellOutcome::default()
            },
            CafOutcome::Cancelled => {
                self.open_family_bin();
                ShellOutcome::redraw()
            }
            CafOutcome::Commit(draft) => {
                let commit_res = match &mut self.screen {
                    ActiveScreen::CreateFamily(caf) => {
                        caf.commit_family_draft(&self.store, &self.neighborhood_id, &draft)
                    }
                    _ => Err("Create-a-Family screen is not active".into()),
                };
                match commit_res {
                    Ok(created) => {
                        self.family_names
                            .insert(created.family_id, created.family_name.clone());
                        save_persisted_caf_name(
                            &self.saves_path,
                            &self.neighborhood_id,
                            self.stored.instance_id,
                            created.family_id,
                            Some(&created.family_name),
                        );
                        if let Err(err) = self.refresh_from_store() {
                            self.note = Some(format!(
                                "Created family {} but refresh failed: {err}",
                                created.family_name
                            ));
                        }
                        self.open_family_bin();
                        // Pre-select the newly created household so the player can
                        // immediately click Move In.
                        if let Some(idx) = self
                            .family_bin_entries
                            .iter()
                            .position(|e| e.id == u32::from(created.family_id))
                        {
                            self.family_bin.select(Some(idx), &self.family_bin_entries);
                        }
                        self.note = Some(format!(
                            "Created family {} ({} Sim{}). Ready to Move In!",
                            created.family_name,
                            created.members.len(),
                            if created.members.len() == 1 { "" } else { "s" }
                        ));
                        ShellOutcome::redraw()
                    }
                    Err(err) => {
                        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
                            caf.set_note(format!("Could not create family: {err}"));
                        } else {
                            self.note = Some(format!("Could not create family: {err}"));
                        }
                        ShellOutcome::redraw()
                    }
                }
            }
        }
    }

    /// Enters `house` in the active neighborhood via [`lifecycle::enter_lot`],
    /// automatically resuming an active travel trip if crash-recovery detects a
    /// persisted `TRVL` envelope on `house`, and driving [`LoadingScreen`] audio
    /// cues across the transition.
    pub fn enter_lot(&mut self, house: u32) -> Result<(), String> {
        // Crash recovery: if `house` has an active persisted travel trip,
        // resume it into the destination lot instead of treating the `TRVL`
        // envelope as a raw `Saved` lot.
        if let Ok(Some(_)) =
            vm_travel::inspect_active_trip(&self.store, &self.neighborhood_id, house)
        {
            self.resume_active_trip(house)?;
            return Ok(());
        }
        self.start_load_audio();
        self.present_loading_step(LoadingKind::Lot, Some(format!("Lot {house}")), 0.2);
        // Capture the neighborhood version BEFORE booting the VM and confirm it
        // did not advance concurrently so `LotSession` never records a newer
        // `(instance_id, revision)` than the snapshot that actually booted.
        let before_nh = match self
            .store
            .load_neighborhood(&self.neighborhood_id)
            .map_err(|e| e.to_string())
            .and_then(|opt| {
                opt.ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))
            }) {
            Ok(nh) => nh,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        let install_assets = match self.ensure_install_assets() {
            Ok(assets) => assets,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        let mut game = match Game::open_from_store_cached(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            house,
            &self.answers,
            self.play,
            self.frames.clone(),
            Some(&install_assets),
        ) {
            Ok(game) => game,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        let after_nh = match self
            .store
            .load_neighborhood(&self.neighborhood_id)
            .map_err(|e| e.to_string())
            .and_then(|opt| {
                opt.ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))
            }) {
            Ok(nh) => nh,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        if after_nh.instance_id != before_nh.instance_id || after_nh.revision != before_nh.revision
        {
            let fallback = self.desired_music_mode();
            self.stop_load_audio(fallback);
            return Err(format!(
                "neighborhood {:?} changed while opening lot {house}; please try again",
                self.neighborhood_id
            ));
        }
        self.stored = before_nh;
        if self.options_file.is_file() {
            let _ = game.load_options(&self.options_file);
        }
        game.coop_set_player_name(self.coop_player_name.clone());
        game.coop_set_iroh_config(self.coop_iroh_config);
        game.set_album_button_enabled(true);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: self.neighborhood_id.clone(),
            neighborhood_instance_id: self.stored.instance_id,
            neighborhood_revision: self.stored.revision,
            staging: None,
            house,
            last_days_played: initial_days_played,
            active_trip: None,
            is_destination_edit: false,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.dialog = None;
        self.coop_lobby.close();
        self.coop_error = None;
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = Some(house);
        self.note = None;
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
        self.pressed_lot_id = None;
        self.nbhd_state.hovered_lot = None;
        self.stop_load_audio(next_music);
        Ok(())
    }

    /// Leaves the currently open lot back to the Neighborhood map, optionally
    /// saving through [`lifecycle::save_and_leave_lot`] + rotating background
    /// autosave first, cleanly draining the background worker, and refreshing
    /// map thumbnails, funds, occupancy, and the Family Bin.
    ///
    /// If the household is currently on an active travel destination visit,
    /// returns them cleanly via [`Self::return_from_destination`] first so
    /// community lot templates are never overwritten and the home lot is
    /// reconciled before returning to the neighborhood map.
    pub fn leave_lot(&mut self, save_first: bool) -> Result<(), String> {
        let mut effective_save_first = save_first;
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.coop_blocks_local_save() {
                // Peers (and disconnected guest worlds) never overwrite local REDB slots.
                effective_save_first = false;
            }
            if session.game.coop_active() {
                session.game.leave_coop();
            }
        }
        if self.active_trip().is_some() {
            self.return_from_destination()?;
        }
        self.start_load_audio();
        self.present_loading_step(LoadingKind::Neighborhood, None, 0.2);
        let mut autosave_warning: Option<String> = None;
        if effective_save_first {
            if let Err(err) = self.save_current_lot() {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
            if let Err(err) = self.enqueue_background_lot_save(SaveTarget::Autosave) {
                autosave_warning = Some(format!(
                    "Rotating autosave could not be queued (existing saves preserved): {err}"
                ));
            }
        }
        // Drain any prior completion noise before flushing this leave's jobs so
        // we accurately report failures from leaving the lot.
        let _ = self.take_save_completions();
        self.flush_background_saves();
        for completion in self.take_save_completions() {
            if let Err(err) = completion.result {
                autosave_warning = Some(format!(
                    "Background save warning (existing saves preserved): {err}"
                ));
            }
        }
        self.persist_options();
        self.screen = ActiveScreen::Neighborhood;
        self.nbhd_state.mode = NeighborhoodMode::Normal;
        self.nbhd_state.hovered_lot = None;
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
        self.pressed_lot_id = None;
        self.move_in_family_id = None;
        self.dialog = None;
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.note = autosave_warning;
        let refresh_res = self.refresh_from_store();
        self.audio_queue
            .push(Input::Events(vec![SoundEvent::StopAll]));
        self.stop_load_audio(music_mode_for_kind(self.kind));
        refresh_res?;
        Ok(())
    }

    /// Executes Move-In for `family_id` onto `house`, refreshes REDB state, and
    /// immediately enters the newly occupied lot. If opening the lot fails,
    /// rolls back the neighborhood commit so the household is not left charged
    /// and moved out of the bin.
    pub fn execute_move_in(&mut self, family_id: u16, house: u32) -> Result<MoveInReport, String> {
        let before = self
            .store
            .load_neighborhood(&self.neighborhood_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))?;
        let report = lifecycle::move_in(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            family_id,
            house,
        )
        .map_err(|e| e.to_string())?;
        match self.enter_lot(house) {
            Ok(()) => {
                self.nbhd_state.mode = NeighborhoodMode::Normal;
                self.move_in_family_id = None;
                let _ = self.refresh_from_store();
                Ok(report)
            }
            Err(enter_err) => {
                let rollback = self
                    .store
                    .save_neighborhood_checked(
                        &self.neighborhood_id,
                        report.committed_instance_id,
                        report.committed_revision,
                        &before,
                    )
                    .map_err(|e| e.to_string())
                    .and_then(|res| res.map(|_| ()).map_err(|e| format!("{e:?}")));
                match rollback {
                    Ok(()) => {
                        let _ = self.refresh_from_store();
                        Err(format!(
                            "could not open lot {house} (move-in rolled back): {enter_err}"
                        ))
                    }
                    Err(rb_err) => {
                        self.nbhd_state.mode = NeighborhoodMode::Normal;
                        self.move_in_family_id = None;
                        let _ = self.refresh_from_store();
                        Err(format!(
                            "family moved into lot {house}, but opening the lot failed \
                             (rollback failed: {rb_err}): {enter_err}"
                        ))
                    }
                }
            }
        }
    }

    /// Executes Evict for the household occupying `house`, returning them to
    /// the Family Bin and refreshing map occupancy and Family Bin immediately.
    pub fn execute_evict(&mut self, house: u32) -> Result<EvictReport, String> {
        let report = lifecycle::evict(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            house,
        )
        .map_err(|e| e.to_string())?;
        self.refresh_from_store()?;
        Ok(report)
    }

    /// Executes Bulldoze on `house`, deleting any persisted custom REDB map
    /// thumbnail and refreshing map occupancy, vacant lot thumbnail, and Family
    /// Bin immediately.
    pub fn execute_bulldoze(&mut self, house: u32) -> Result<BulldozeReport, String> {
        let report = lifecycle::bulldoze(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            house,
        )
        .map_err(|e| e.to_string())?;
        let thumb_warning = match self.store.delete_lot_thumbnail_checked(
            &self.neighborhood_id,
            self.stored.instance_id,
            house,
        ) {
            Ok(_) => None,
            Err(err) => Some(format!("Lot bulldozed; thumbnail cleanup warning: {err}")),
        };
        if let Ok(lot_id) = u16::try_from(house) {
            self.custom_thumbs
                .remove(&(self.neighborhood_id.clone(), lot_id));
        }
        self.refresh_from_store()?;
        if let Some(warn) = thumb_warning {
            self.note = Some(warn);
        }
        Ok(report)
    }

    /// Permanently removes a Family Bin household from the active neighborhood
    /// in REDB via [`sims_vm::caf::delete_bin_family`] (vacating `NBRS` slots,
    /// scrubbing reciprocal relationships, and recording deletion tombstones)
    /// and refreshes the Family Bin list.
    pub fn execute_delete_family(&mut self, family_id: u16) -> Result<(), String> {
        sims_vm::caf::delete_bin_family(&self.store, &self.neighborhood_id, family_id)
            .map_err(|e| e.to_string())?;
        self.family_names.remove(&family_id);
        save_persisted_caf_name(
            &self.saves_path,
            &self.neighborhood_id,
            self.stored.instance_id,
            family_id,
            None,
        );
        self.refresh_from_store()?;
        Ok(())
    }

    /// Reloads [`StoredNeighborhood`], lot metadata, map occupancy, and Family
    /// Bin entries from REDB.
    pub fn refresh_from_store(&mut self) -> Result<(), String> {
        self.stored = self
            .store
            .load_neighborhood(&self.neighborhood_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))?;
        let (faces, base_cat) = load_neighborhood_faces_cached(
            &self.install,
            &self.store,
            &self.neighborhood_id,
            &self.stored,
            &self.pool,
            self.base_index.as_ref(),
            self.base_catalog.as_ref(),
        );
        self.faces = faces;
        if self.base_catalog.is_none() {
            self.base_catalog = base_cat;
        }
        self.lot_meta = build_lot_meta(
            &self.store,
            &self.neighborhood_id,
            &self.map,
            &self.descriptions,
            self.arch_multiplier,
        );
        self.custom_thumbs = load_persisted_custom_thumbs(&self.store, &self.neighborhood_id);
        sync_map_with_store(
            &mut self.map,
            &self.stored,
            &self.lot_meta,
            &self.neighborhood_id,
            &self.custom_thumbs,
        );
        self.family_bin_entries =
            build_family_bin_entries(&self.stored, &self.family_names, &self.faces);
        self.family_bin
            .sync(&self.family_bin_entries, sims_ui::family_bin::VISIBLE_ROWS);
        self.refresh_importable_families();
        self.sync_travel_state();
        self.refresh_hovered_lot();
        Ok(())
    }

    fn refresh_hovered_lot(&mut self) {
        self.nbhd_state.hovered_lot = self
            .hovered_lot_id
            .filter(|&id| self.hover_card_shown_for == Some(id))
            .and_then(|id| self.build_lot_info(id));
    }

    fn magic_cost_for_lot(&self, lot_id: u16) -> i32 {
        match lot_id {
            91 => self.magic_lot_91_cost,
            90 | 92 => self.magic_lot_other_cost,
            _ => 0,
        }
    }

    fn build_lot_info(&self, lot_id: u16) -> Option<LotInfo> {
        let map_lot = self.map.lot(lot_id)?;
        let origin = self.map.map_origin_for(self.frame_size_u32());
        let anchor = [
            origin[0].saturating_add(map_lot.anchor[0]),
            origin[1].saturating_add(map_lot.anchor[1]),
        ];
        let meta = self.lot_meta.get(&lot_id);
        let is_magic_res = (90..=92).contains(&lot_id);
        let is_magic_comm = (93..=98).contains(&lot_id);
        let kind = if is_magic_res {
            LotKind::MagicResidential
        } else if is_magic_comm {
            LotKind::MagicCommunity
        } else if map_lot.zoning == LotZoning::Community {
            LotKind::Community
        } else {
            LotKind::Residential
        };

        let occupant = self
            .stored
            .family_in(u32::from(lot_id))
            .map(|(fam_id, fam)| {
                let family_name = self
                    .family_names
                    .get(&fam_id)
                    .cloned()
                    .unwrap_or_else(|| format!("Family {fam_id}"));
                let portraits = fam
                    .members
                    .iter()
                    .filter_map(|guid| self.faces.face(*guid).cloned())
                    .collect();
                let magic_coins = lifecycle::family_magic_coins(&self.stored, fam_id);
                LotOccupant {
                    family_name,
                    funds: fam.budget,
                    house_value: fam.architecture_value,
                    net_worth: family_net_worth(fam),
                    friends: fam.friends,
                    magic_coins,
                    portraits,
                }
            });

        Some(LotInfo {
            lot_number: lot_id,
            address: self.street_names.address_for(lot_id),
            house_name: meta.map_or_else(String::new, |m| m.lot_name.clone()),
            house_description: meta.map_or_else(String::new, |m| m.lot_description.clone()),
            kind,
            has_house: meta.is_some_and(|m| m.has_house),
            house_cost: meta.map_or(0, |m| m.house_cost),
            magic_cost: self.magic_cost_for_lot(lot_id),
            occupant,
            anchor,
        })
    }

    fn handle_neighborhood_command(&mut self, cmd: NeighborhoodCommand) -> ShellOutcome {
        match cmd {
            NeighborhoodCommand::PreviousNeighborhood => {
                let prev = self.nbhd_state.previous_neighborhood();
                if let Err(e) = self.switch_neighborhood(prev) {
                    self.note = Some(format!("Could not switch neighborhood: {e}"));
                }
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::NextNeighborhood => {
                let next = self.nbhd_state.next_neighborhood();
                if let Err(e) = self.switch_neighborhood(next) {
                    self.note = Some(format!("Could not switch neighborhood: {e}"));
                }
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenFamilyBin => {
                self.open_family_bin();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::CancelMoveIn => {
                self.nbhd_state.mode = NeighborhoodMode::Normal;
                self.move_in_family_id = None;
                self.refresh_hovered_lot();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::ToggleBulldoze => {
                self.nbhd_state.mode = if matches!(self.nbhd_state.mode, NeighborhoodMode::Bulldoze)
                {
                    NeighborhoodMode::Normal
                } else {
                    self.move_in_family_id = None;
                    NeighborhoodMode::Bulldoze
                };
                self.refresh_hovered_lot();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenDowntown => {
                self.toggle_sub_neighborhood(NeighborhoodKind::Downtown);
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenVacationIsland => {
                self.toggle_sub_neighborhood(NeighborhoodKind::Vacation);
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenStudioTown => {
                self.toggle_sub_neighborhood(NeighborhoodKind::StudioTown);
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenMagicTown => {
                self.toggle_sub_neighborhood(NeighborhoodKind::MagicTown);
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::Quit => {
                self.open_quit_dialog();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::ToggleRezone => {
                self.nbhd_state.mode = if matches!(self.nbhd_state.mode, NeighborhoodMode::Rezone) {
                    NeighborhoodMode::Normal
                } else {
                    self.move_in_family_id = None;
                    NeighborhoodMode::Rezone
                };
                self.refresh_hovered_lot();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::ImportFamily => {
                self.open_import_family_picker();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenWeb => {
                let title = self
                    .chrome_strings
                    .bar
                    .get(sims_ui::neighborhood::BarButton::Web.tooltip_index())
                    .filter(|s| !s.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| "Web".into());
                self.open_dialog(
                    ShellDialogKind::Notice,
                    Message::notice(
                        title,
                        "This offline engine does not launch external web browsers.",
                        self.dialogs.ok(),
                    )
                    .with_frame(FrameStyle::Generic),
                );
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenCredits => {
                self.open_credits_modal();
                ShellOutcome::redraw()
            }
        }
    }

    fn toggle_sub_neighborhood(&mut self, target: NeighborhoodKind) {
        let next = if self.kind == target {
            default_map_kind(&self.install, self.slot)
        } else {
            target
        };
        if let Err(e) = self.switch_kind(next) {
            self.note = Some(format!("Could not open map view: {e}"));
        }
    }

    fn handle_family_bin_command(&mut self, cmd: FamilyBinCommand) -> ShellOutcome {
        match cmd {
            FamilyBinCommand::SelectFamily(_) => ShellOutcome::redraw(),
            FamilyBinCommand::Cancel => {
                self.close_family_bin();
                ShellOutcome::redraw()
            }
            FamilyBinCommand::MoveInFamily(id) => {
                if let Ok(fam_id) = u16::try_from(id)
                    && let Err(e) = self.start_move_in(fam_id)
                {
                    self.note = Some(e);
                }
                ShellOutcome::redraw()
            }
            FamilyBinCommand::DeleteFamily(id) => {
                if let Ok(fam_id) = u16::try_from(id) {
                    let name = self
                        .family_names
                        .get(&fam_id)
                        .cloned()
                        .unwrap_or_else(|| format!("Family {fam_id}"));
                    let title = DialogStrings::get(&self.dialogs.family_bin.entries, 6, "Delete");
                    let tmpl = DialogStrings::get(&self.dialogs.family_bin.entries, 7, "%s");
                    let body = format_template(tmpl, &[&name]);
                    self.open_dialog(
                        ShellDialogKind::ConfirmDeleteFamily { family_id: fam_id },
                        Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                            .with_frame(FrameStyle::Generic),
                    );
                }
                ShellOutcome::redraw()
            }
            FamilyBinCommand::CreateNewFamily => {
                if let Err(err) = self.open_create_family() {
                    self.note = Some(format!("Could not open Create-a-Family: {err}"));
                }
                ShellOutcome::redraw()
            }
        }
    }

    fn activate_lot(&mut self, lot_id: u16) -> ShellOutcome {
        match self.nbhd_state.mode.clone() {
            NeighborhoodMode::Rezone => self.prompt_rezone_lot(lot_id),
            NeighborhoodMode::Normal | NeighborhoodMode::Visit => {
                let house = u32::from(lot_id);
                if let Ok(Some(trip)) =
                    vm_travel::inspect_active_trip(&self.store, &self.neighborhood_id, house)
                {
                    let title = format!("Active Travel Trip (Lot {house})");
                    let body = format!(
                        "This household has an interrupted trip at destination lot {}. Resume the trip or abort and restore the frozen home lot?",
                        trip.destination_house
                    );
                    self.open_dialog(
                        ShellDialogKind::ConfirmResumeOrAbortTrip {
                            home_house: house,
                            destination_house: trip.destination_house,
                        },
                        Message::confirm(
                            title,
                            body,
                            vec!["Resume Trip".into(), "Abort Trip".into(), "Cancel".into()],
                        )
                        .with_frame(FrameStyle::Generic),
                    );
                    return ShellOutcome::redraw();
                }
                let is_unoccupied_destination = destination_for_kind(self.kind).is_some()
                    && TravelDestination::classify(house, &self.stored.zoning).is_some()
                    && self.stored.family_in(house).is_none();
                let res = if is_unoccupied_destination {
                    self.open_destination_for_edit(house)
                } else {
                    self.enter_lot(house)
                };
                if let Err(e) = res {
                    // Non-destructive corrupt-save fallback: if intact autosaves
                    // exist for this lot, open the Save Manager with the error banner.
                    if self
                        .store
                        .autosaves()
                        .list_saves_metadata(&self.neighborhood_id, house)
                        .is_ok_and(|slots| !slots.is_empty())
                    {
                        self.save_manager_target_lot = Some(house);
                        self.open_save_manager(Some(format!("Could not open lot {lot_id}: {e}")));
                    } else {
                        self.note = Some(format!("Could not enter lot {lot_id}: {e}"));
                    }
                }
                ShellOutcome::redraw()
            }
            NeighborhoodMode::MoveIn {
                family_funds,
                family_magic_coins,
                ..
            } => {
                let Some(family_id) = self.move_in_family_id else {
                    self.nbhd_state.mode = NeighborhoodMode::Normal;
                    return ShellOutcome::redraw();
                };
                let is_res = self.stored.is_residential(u32::from(lot_id));
                if !is_res {
                    let title = DialogStrings::get(&self.dialogs.move_in, 12, "Community");
                    let body = DialogStrings::get(&self.dialogs.move_in, 13, "Not residential.");
                    self.open_dialog(
                        ShellDialogKind::Notice,
                        Message::notice(title, body, self.dialogs.ok())
                            .with_frame(FrameStyle::Generic),
                    );
                    return ShellOutcome::redraw();
                }
                if self.stored.family_in(u32::from(lot_id)).is_some() {
                    let title = DialogStrings::get(&self.dialogs.move_in, 14, "Occupied");
                    let body = DialogStrings::get(&self.dialogs.move_in, 15, "Lot is occupied.");
                    self.open_dialog(
                        ShellDialogKind::Notice,
                        Message::notice(title, body, self.dialogs.ok())
                            .with_frame(FrameStyle::Generic),
                    );
                    return ShellOutcome::redraw();
                }
                let meta = self.lot_meta.get(&lot_id);
                let cost = meta.map_or(0, |m| m.house_cost);
                let magic_cost = self.magic_cost_for_lot(lot_id);
                let has_house = meta.is_some_and(|m| m.has_house);
                if family_funds < cost || (magic_cost > 0 && family_magic_coins < magic_cost) {
                    // Hover card already explains insufficient Simoleons or MagiCoins.
                    return ShellOutcome::redraw();
                }
                let (title_idx, body_idx) = if has_house { (0, 1) } else { (6, 7) };
                let title = DialogStrings::get(&self.dialogs.move_in, title_idx, "Move In");
                let body = DialogStrings::get(&self.dialogs.move_in, body_idx, "Move in?");
                self.open_dialog(
                    ShellDialogKind::ConfirmMoveIn { family_id, lot_id },
                    Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                        .with_frame(FrameStyle::Generic),
                );
                ShellOutcome::redraw()
            }
            NeighborhoodMode::Bulldoze => {
                if let Some((fam_id, fam)) = self.stored.family_in(u32::from(lot_id)) {
                    let name = self
                        .family_names
                        .get(&fam_id)
                        .cloned()
                        .unwrap_or_else(|| format!("Family {fam_id}"));
                    let worth = format_simoleons(family_net_worth(fam));
                    let title = DialogStrings::get(&self.dialogs.bulldoze, 2, "Evict");
                    let tmpl = DialogStrings::get(&self.dialogs.bulldoze, 3, "%s (%s)");
                    let body = format_template(tmpl, &[&name, &worth]);
                    self.open_dialog(
                        ShellDialogKind::ConfirmEvict { lot_id },
                        Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                            .with_frame(FrameStyle::Generic),
                    );
                    return ShellOutcome::redraw();
                }
                let has_house = self.lot_meta.get(&lot_id).is_some_and(|m| m.has_house);
                if !has_house {
                    let title = DialogStrings::get(&self.dialogs.bulldoze, 4, "Bulldoze");
                    let body = DialogStrings::get(&self.dialogs.bulldoze, 5, "Empty lot.");
                    self.open_dialog(
                        ShellDialogKind::Notice,
                        Message::notice(title, body, self.dialogs.ok())
                            .with_frame(FrameStyle::Generic),
                    );
                    return ShellOutcome::redraw();
                }
                let title = DialogStrings::get(&self.dialogs.bulldoze, 0, "Bulldoze");
                let body = DialogStrings::get(&self.dialogs.bulldoze, 1, "Bulldoze?");
                self.open_dialog(
                    ShellDialogKind::ConfirmBulldoze { lot_id },
                    Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                        .with_frame(FrameStyle::Generic),
                );
                ShellOutcome::redraw()
            }
        }
    }

    fn open_quit_dialog(&mut self) {
        let (title, body) = (
            self.dialogs.options.confirm_quit.0.clone(),
            self.dialogs.options.confirm_quit.1.clone(),
        );
        self.open_dialog(
            ShellDialogKind::ConfirmQuit,
            Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                .with_frame(FrameStyle::Generic),
        );
    }

    fn clear_hover_state(&mut self) {
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.nbhd_state.hovered_lot = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
    }

    fn open_dialog(&mut self, kind: ShellDialogKind, message: Message) {
        self.clear_hover_state();
        let state = DialogState::for_message(&message);
        let layout = dialog::layout(self.size, &message, &self.dialog_art);
        self.dialog = Some(ShellDialog {
            kind,
            message,
            state,
            layout,
        });
        if self.clock_us > 0 {
            let now_ms = (self.clock_us / 1_000) as u32;
            self.dialog_fade = DialogFadeState::hidden();
            self.dialog_fade
                .fade_to(1.0, now_ms, DIALOG_FADE_DURATION_MS);
            self.dialog_fade_was_animating = true;
        } else {
            self.dialog_fade = DialogFadeState::opaque();
            self.dialog_fade_was_animating = false;
        }
    }

    fn relayout_dialog(&mut self) {
        if let Some(dlg) = &mut self.dialog {
            dlg.layout = dialog::layout(self.size, &dlg.message, &self.dialog_art);
        }
    }

    fn answer_dialog(&mut self, button: u8) -> ShellOutcome {
        let Some(dlg) = self.dialog.take() else {
            return ShellOutcome::default();
        };
        self.dialog_fade = DialogFadeState::hidden();
        self.dialog_fade_was_animating = false;
        if let ShellDialogKind::ConfirmResumeOrAbortTrip {
            home_house,
            destination_house,
        } = dlg.kind
        {
            match button {
                0 => {
                    if let Err(e) = self.resume_active_trip(home_house) {
                        self.note = Some(format!(
                            "Could not resume trip to lot {destination_house}: {e}"
                        ));
                    }
                }
                1 => match self.abort_active_trip(home_house) {
                    Ok(restored) => {
                        self.note = Some(format!(
                            "Aborted interrupted trip and restored home lot {restored}."
                        ));
                    }
                    Err(e) => {
                        self.note = Some(format!("Could not abort interrupted trip: {e}"));
                    }
                },
                _ => {}
            }
            return ShellOutcome::redraw();
        }
        if button != 0 {
            return ShellOutcome::redraw();
        }
        match dlg.kind {
            ShellDialogKind::ConfirmQuit => ShellOutcome {
                redraw: true,
                quit: true,
                photo: false,
            },
            ShellDialogKind::ConfirmMoveIn { family_id, lot_id } => {
                if let Err(e) = self.execute_move_in(family_id, u32::from(lot_id)) {
                    self.note = Some(format!("Move-In failed: {e}"));
                }
                ShellOutcome::redraw()
            }
            ShellDialogKind::ConfirmEvict { lot_id } => {
                if let Err(e) = self.execute_evict(u32::from(lot_id)) {
                    self.note = Some(format!("Evict failed: {e}"));
                }
                ShellOutcome::redraw()
            }
            ShellDialogKind::ConfirmBulldoze { lot_id } => {
                if let Err(e) = self.execute_bulldoze(u32::from(lot_id)) {
                    self.note = Some(format!("Bulldoze failed: {e}"));
                }
                ShellOutcome::redraw()
            }
            ShellDialogKind::ConfirmDeleteFamily { family_id } => {
                if let Err(e) = self.execute_delete_family(family_id) {
                    self.note = Some(format!("Delete family failed: {e}"));
                }
                ShellOutcome::redraw()
            }
            ShellDialogKind::ConfirmRezone {
                lot_id,
                target_zone,
                expected_instance_id,
                expected_revision,
                expected_occupant,
            } => {
                match self.execute_rezone_checked(
                    lot_id,
                    target_zone,
                    Some((expected_instance_id, expected_revision, expected_occupant)),
                ) {
                    Ok(_) => {}
                    Err(e) => {
                        let _ = self.refresh_from_store();
                        self.open_dialog(
                            ShellDialogKind::Notice,
                            Message::notice(
                                DialogStrings::get(&self.dialogs.rezone, 0, "Rezone"),
                                format!("Could not rezone lot {lot_id}: {e}"),
                                self.dialogs.ok(),
                            )
                            .with_frame(FrameStyle::Generic),
                        );
                    }
                }
                ShellOutcome::redraw()
            }
            ShellDialogKind::ConfirmResumeOrAbortTrip { .. } => unreachable!(),
            ShellDialogKind::Notice => ShellOutcome::redraw(),
        }
    }

    // =========================================================================
    // Neighborhood Top-Bar Secondary Actions: Rezone, Import Family, Credits
    // =========================================================================

    fn is_lot_rezonable(&self, lot_id: u16) -> bool {
        destination_for_kind(self.kind).is_none()
            && ((1..=10).contains(&lot_id) || (50..=80).contains(&lot_id))
    }

    fn zoning_label(&self, zone: i32) -> &str {
        if zone == 2 {
            DialogStrings::get(&self.dialogs.zoning_types, 1, "Community")
        } else {
            DialogStrings::get(&self.dialogs.zoning_types, 0, "Residential")
        }
    }

    fn active_trip_blocking_rezone(
        &self,
        stored: &StoredNeighborhood,
        house: u32,
        saved_lot: Option<&[u8]>,
    ) -> Option<String> {
        let house_i32 = i32::try_from(house).unwrap_or(-1);
        for fam in stored.families.values() {
            let sec = fam.secondary_house();
            if fam.house == house_i32 && sec != 0 {
                return Some(format!(
                    "Lot {house} has an active travel trip to lot {sec}; resume or abort the trip before rezoning."
                ));
            }
            if sec == house_i32 && sec != 0 {
                return Some(format!(
                    "Lot {house} is the destination of an active travel trip from lot {}; resume or abort the trip before rezoning.",
                    fam.house
                ));
            }
        }
        if saved_lot.is_some_and(|bytes| bytes.len() >= 4 && &bytes[0..4] == b"TRVL") {
            return Some(format!(
                "Lot {house} has an active travel trip in progress; resume or abort the trip before rezoning."
            ));
        }
        None
    }

    fn prompt_rezone_lot(&mut self, lot_id: u16) -> ShellOutcome {
        let title = DialogStrings::get(&self.dialogs.rezone, 0, "Rezone").to_owned();
        if !self.is_lot_rezonable(lot_id) {
            self.open_dialog(
                ShellDialogKind::Notice,
                Message::notice(
                    title,
                    format!("Lot {lot_id} cannot be rezoned."),
                    self.dialogs.ok(),
                )
                .with_frame(FrameStyle::Generic),
            );
            return ShellOutcome::redraw();
        }
        let _ = self.refresh_from_store();
        let house = u32::from(lot_id);
        let saved_lot = self
            .store
            .load_saved_lot(&self.neighborhood_id, house)
            .ok()
            .flatten();
        if let Some(reason) =
            self.active_trip_blocking_rezone(&self.stored, house, saved_lot.as_deref())
        {
            self.open_dialog(
                ShellDialogKind::Notice,
                Message::notice(title, reason, self.dialogs.ok()).with_frame(FrameStyle::Generic),
            );
            return ShellOutcome::redraw();
        }
        let current_zone = self
            .stored
            .zoning
            .get(&(lot_id as i16))
            .copied()
            .unwrap_or(1);
        let target_zone = if current_zone == 2 { 1 } else { 2 };
        let current_label = self.zoning_label(current_zone).to_owned();
        let target_label = self.zoning_label(target_zone).to_owned();
        let base_tmpl = DialogStrings::get(&self.dialogs.rezone, 1, "Rezone %s to %s?");
        let mut body = format_template(base_tmpl, &[&current_label, &target_label]);
        let expected_occupant = self
            .stored
            .families
            .iter()
            .find(|(_, f)| f.house == i32::from(lot_id))
            .map(|(&id, _)| id);
        if let Some(fam_id) = expected_occupant {
            let fam_name = self
                .family_names
                .get(&fam_id)
                .cloned()
                .unwrap_or_else(|| format!("Family {fam_id}"));
            body.push_str(&format!(
                "\n\n{fam_name} will be moved back to the Family Bin before rezoning."
            ));
        }
        self.open_dialog(
            ShellDialogKind::ConfirmRezone {
                lot_id,
                target_zone,
                expected_instance_id: self.stored.instance_id,
                expected_revision: self.stored.revision,
                expected_occupant,
            },
            Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                .with_frame(FrameStyle::Generic),
        );
        ShellOutcome::redraw()
    }

    /// Toggles `lot_id` zoning in REDB (`1` = Residential, `2` = Community),
    /// atomically staging any household eviction and zoning update together
    /// under strict `(instance_id, revision)` fencing, and refreshes map
    /// metadata and hover overlays immediately.
    pub fn execute_rezone(&mut self, lot_id: u16, target_zone: i32) -> Result<i32, String> {
        self.execute_rezone_checked(lot_id, target_zone, None)
    }

    fn execute_rezone_checked(
        &mut self,
        lot_id: u16,
        target_zone: i32,
        expected: Option<(u128, u64, Option<u16>)>,
    ) -> Result<i32, String> {
        if !self.is_lot_rezonable(lot_id) {
            return Err(format!("lot {lot_id} is not eligible for rezoning"));
        }
        if target_zone != 1 && target_zone != 2 {
            return Err(format!("unsupported zoning code {target_zone}"));
        }
        let house = u32::from(lot_id);
        let snapshot = self
            .store
            .load_lot_snapshot(&self.neighborhood_id, house)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))?;

        if let Some(reason) = self.active_trip_blocking_rezone(
            &snapshot.neighborhood,
            house,
            snapshot.saved_lot.as_deref(),
        ) {
            return Err(reason);
        }

        let live_occupant = snapshot
            .neighborhood
            .families
            .iter()
            .find(|(_, f)| f.house == i32::from(lot_id))
            .map(|(&id, _)| id);

        let (fence_instance_id, fence_revision) =
            if let Some((exp_inst, exp_rev, exp_occ)) = expected {
                if snapshot.neighborhood.instance_id != exp_inst
                    || snapshot.neighborhood.revision != exp_rev
                    || live_occupant != exp_occ
                {
                    return Err(
                    "neighborhood state or lot occupancy changed since confirmation; please retry"
                        .into(),
                );
                }
                (exp_inst, exp_rev)
            } else {
                (
                    snapshot.neighborhood.instance_id,
                    snapshot.neighborhood.revision,
                )
            };

        if live_occupant.is_some() {
            // Stage the eviction in an ephemeral NeighborhoodStore so the
            // evicted lot blob and updated zoning commit together in one
            // fenced REDB transaction (never leaving a partial eviction behind).
            let staging_dir = TempStagingDir::new("rezone-evict")?;
            let staging_store = NeighborhoodStore::open(&staging_dir.path().join("stage.redb"))
                .map_err(|e| e.to_string())?;
            for (file_name, bytes) in &snapshot.character_files {
                staging_store
                    .save_character_file(&self.neighborhood_id, file_name, bytes)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(base_house) = &snapshot.base_house {
                staging_store
                    .save_base_house(&self.neighborhood_id, house, base_house)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(saved_lot) = &snapshot.saved_lot {
                staging_store
                    .save_lot_and_neighborhood(
                        &self.neighborhood_id,
                        house,
                        saved_lot,
                        &snapshot.neighborhood,
                    )
                    .map_err(|e| e.to_string())?;
            } else {
                staging_store
                    .save_neighborhood(&self.neighborhood_id, &snapshot.neighborhood)
                    .map_err(|e| e.to_string())?;
            }
            lifecycle::evict(
                &self.install_root,
                &staging_store,
                &self.neighborhood_id,
                house,
            )
            .map_err(|e| e.to_string())?;
            let evicted_lot_bytes = staging_store
                .load_saved_lot(&self.neighborhood_id, house)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "staged eviction lot missing after evict".to_owned())?;
            let mut evicted_nh = staging_store
                .load_neighborhood(&self.neighborhood_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "staged neighborhood missing after evict".to_owned())?;
            evicted_nh.zoning.insert(lot_id as i16, target_zone);
            self.store
                .save_lot_and_neighborhood_checked(
                    &self.neighborhood_id,
                    house,
                    Some((fence_instance_id, fence_revision)),
                    &evicted_lot_bytes,
                    &evicted_nh,
                    &[],
                )
                .map_err(|e| e.to_string())?
                .map_err(|conflict| format!("neighborhood modified concurrently ({conflict:?})"))?;
        } else {
            let mut updated = snapshot.neighborhood;
            updated.zoning.insert(lot_id as i16, target_zone);
            updated.revision = updated.revision.saturating_add(1);
            self.store
                .save_neighborhood_checked(
                    &self.neighborhood_id,
                    fence_instance_id,
                    fence_revision,
                    &updated,
                )
                .map_err(|e| e.to_string())?
                .map_err(|conflict| format!("neighborhood modified concurrently ({conflict:?})"))?;
        }

        self.refresh_from_store()?;
        self.note = Some(format!(
            "Rezoned lot {lot_id} to {}.",
            self.zoning_label(target_zone)
        ));
        Ok(target_zone)
    }

    /// Adds an extra directory scanned for `.FAM` files by `ImportFamily`
    /// (prepended ahead of the install's `UserData*/Import` and `Export` folders).
    pub fn add_import_search_dir(&mut self, dir: PathBuf) {
        if !self.import_search_dirs.contains(&dir) {
            self.import_search_dirs.insert(0, dir);
        }
        self.refresh_importable_families();
    }

    fn scan_importable_families(&self) -> Vec<ImportableFamily> {
        let mut dirs: Vec<(PathBuf, String)> = Vec::new();
        for extra in &self.import_search_dirs {
            let label = extra
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Import")
                .to_owned();
            dirs.push((extra.clone(), label));
        }
        if let Ok(user_dir) = resolve_user_dir(&self.install, self.slot) {
            if let Some(imp) = resolve_dir_case_insensitive(&user_dir, "Import") {
                dirs.push((imp, format!("{}/Import", self.neighborhood_id)));
            }
            if let Some(exp) = resolve_dir_case_insensitive(&user_dir, "Export") {
                dirs.push((exp, format!("{}/Export", self.neighborhood_id)));
            }
        }
        if let Ok(ud1) = self.install.file("UserData") {
            if let Some(imp) = resolve_dir_case_insensitive(&ud1, "Import")
                && !dirs.iter().any(|(d, _)| d == &imp)
            {
                dirs.push((imp, "UserData/Import".into()));
            }
            if let Some(exp) = resolve_dir_case_insensitive(&ud1, "Export")
                && !dirs.iter().any(|(d, _)| d == &exp)
            {
                dirs.push((exp, "UserData/Export".into()));
            }
        }

        let mut seen_paths = BTreeSet::new();
        let mut out = Vec::new();
        for (dir, source_label) in dirs {
            let Ok(read_dir) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut entries: Vec<PathBuf> = read_dir
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("fam"))
                })
                .collect();
            entries.sort();
            for path in entries {
                let canon = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if !seen_paths.insert(canon) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(parsed) = FamilyFile::parse(&bytes) else {
                    continue;
                };
                // Skip system NPC/stray households (`Strays_4000.FAM`).
                if parsed.export_info.family_id >= 4000 {
                    continue;
                }
                let member_count = parsed
                    .export_info
                    .member_ids
                    .into_iter()
                    .take_while(|&id| id != 0)
                    .count();
                if member_count == 0 {
                    continue;
                }
                let family_name = parsed
                    .iff
                    .get(b"CTSS", 1000)
                    .and_then(|c| StringTable::parse(parsed.iff.data(c)).ok())
                    .and_then(|t| {
                        t.get(0, US_ENGLISH)
                            .or_else(|| t.get(0, 1))
                            .map(str::to_owned)
                    })
                    .or_else(|| {
                        parsed
                            .name
                            .get(0, US_ENGLISH)
                            .or_else(|| parsed.name.get(0, 1))
                            .map(str::to_owned)
                    })
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| {
                        path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Family")
                            .to_owned()
                    });
                out.push(ImportableFamily {
                    path,
                    family_name,
                    member_count,
                    budget: parsed.family.budget,
                    source_label: source_label.clone(),
                });
            }
        }
        out
    }

    /// Rescans configured `Import/` and `Export/` folders for `.FAM` archives
    /// and updates the top-bar Import button enabled state. Returns `true` if
    /// availability changed.
    pub fn refresh_importable_families(&mut self) -> bool {
        let next = !self.scan_importable_families().is_empty();
        if next != self.nbhd_state.has_importable_family {
            self.nbhd_state.has_importable_family = next;
            true
        } else {
            false
        }
    }

    /// Scans installed `Import/` and `Export/` `.FAM` archives and opens the
    /// native Family Import picker modal.
    pub fn open_import_family_picker(&mut self) -> bool {
        let candidates = self.scan_importable_families();
        self.nbhd_state.has_importable_family = !candidates.is_empty();
        let title = DialogStrings::get(&self.dialogs.import_strs, 8, "Import Family").to_owned();
        if candidates.is_empty() {
            self.open_dialog(
                ShellDialogKind::Notice,
                Message::notice(
                    title,
                    "No importable .FAM family files were found in UserData/Import.",
                    self.dialogs.ok(),
                )
                .with_frame(FrameStyle::Generic),
            );
            return false;
        }
        let mut items = Vec::with_capacity(candidates.len());
        let mut entries = BTreeMap::new();
        for (idx, fam) in candidates.into_iter().enumerate() {
            let id = format!("import_fam:{idx}");
            let file_name = fam
                .path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("family.FAM");
            let members_word = if fam.member_count == 1 { "Sim" } else { "Sims" };
            let description = format!(
                "{} {} | {} | {}/{}",
                fam.member_count,
                members_word,
                format_simoleons(fam.budget),
                fam.source_label,
                file_name,
            );
            items.push(ShopItem {
                id: id.clone(),
                name: fam.family_name.clone(),
                description,
                price: 0,
                currency: CurrencyKind::Simoleons,
                icon: None,
                supports_try_on: false,
                owned: false,
                available: true,
            });
            entries.insert(id, fam);
        }
        self.clear_hover_state();
        self.import_picker_entries = entries;
        self.import_picker_snapshot = Some(ShopSnapshot {
            kind: ShopKind::DowntownBoutique(WardrobeCategory::Everyday),
            title_override: Some(title),
            buy_label_override: Some("Import".into()),
            shopper_name: None,
            wallet: Wallet::default(),
            items,
            tried_on_item_id: None,
            preview_image: None,
            preview_label: None,
        });
        self.import_picker.open();
        true
    }

    pub fn close_import_family_picker(&mut self) {
        self.import_picker.close();
        self.import_picker_snapshot = None;
        self.import_picker_entries.clear();
    }

    fn handle_import_picker_command(&mut self, cmd: ShopCommand) -> ShellOutcome {
        match cmd {
            ShopCommand::Cancel => {
                self.close_import_family_picker();
                ShellOutcome::redraw()
            }
            ShopCommand::TryOn { .. } | ShopCommand::RotatePreview(_) => ShellOutcome::redraw(),
            ShopCommand::Buy { item_id, .. } => {
                let Some(entry) = self.import_picker_entries.get(&item_id).cloned() else {
                    return ShellOutcome::redraw();
                };
                self.close_import_family_picker();
                let title =
                    DialogStrings::get(&self.dialogs.import_strs, 8, "Import Family").to_owned();
                match self.execute_import_family(&entry) {
                    Ok(created) => {
                        let body = format!(
                            "{} imported into the Family Bin ({} members).",
                            created.family_name,
                            created.members.len()
                        );
                        self.note = Some(body.clone());
                        self.open_dialog(
                            ShellDialogKind::Notice,
                            Message::notice(title, body, self.dialogs.ok())
                                .with_frame(FrameStyle::Generic),
                        );
                    }
                    Err(e) => {
                        let body = format!("Could not import {}: {e}", entry.family_name);
                        self.note = Some(body.clone());
                        self.open_dialog(
                            ShellDialogKind::Notice,
                            Message::notice(title, body, self.dialogs.ok())
                                .with_frame(FrameStyle::Generic),
                        );
                    }
                }
                ShellOutcome::redraw()
            }
        }
    }

    fn execute_import_family(
        &mut self,
        entry: &ImportableFamily,
    ) -> Result<sims_vm::caf::CreatedHousehold, String> {
        let raw_bytes = std::fs::read(&entry.path).map_err(|e| e.to_string())?;
        let parsed = FamilyFile::parse(&raw_bytes).map_err(|e| e.to_string())?;
        let fam_bytes = if parsed.family.house == 0 && parsed.family.secondary_house() == 0 {
            raw_bytes
        } else {
            // Normalize a house-exported `.FAM` into a Family Bin archive (`house = 0`,
            // `secondary_house = 0`) so `import_family_fam` places it cleanly in the Family Bin.
            let mut bin_family = parsed.family.clone();
            bin_family.house = 0;
            if bin_family.version >= 8 && bin_family.tail.len() >= 4 {
                bin_family.tail[0..4].fill(0);
            }
            let fami_id = parsed.export_info.family_id;
            let fami_bytes = encode_fami(&bin_family);
            // Rewrite ONLY the IFF's effective resources in `resource_order`
            // (preserving native `rsmp` lookup semantics and omitting stale,
            // unlisted physical chunks so reparse never exposes shadowed data).
            let tags: BTreeSet<[u8; 4]> = parsed
                .iff
                .chunks
                .iter()
                .map(|c| c.tag)
                .filter(|tag| tag != b"rsmp" && tag != b"XXXX")
                .collect();
            let mut chunks: Vec<ChunkData> = Vec::with_capacity(parsed.iff.chunks.len());
            for tag in tags {
                let effective: Vec<&sims_formats::iff::Chunk> =
                    match parsed.iff.resource_order(&tag) {
                        Ok(list) => list,
                        Err(_) => parsed.iff.all(&tag).collect(),
                    };
                for chunk in effective {
                    let eff_flags = parsed.iff.resource_flags(chunk);
                    // Clear `DELETED` (`4`) when emitting a mapless IFF so an
                    // `rsmp`-listed live resource is never skipped by the
                    // physical chunk scanner on reparse.
                    let write_flags = eff_flags & !sims_formats::iff::DELETED;
                    let data =
                        if &chunk.tag == b"FAMI" && chunk.id == fami_id && (eff_flags >> 8) == 0 {
                            fami_bytes.clone()
                        } else {
                            parsed.iff.data(chunk).to_vec()
                        };
                    chunks.push(ChunkData {
                        tag: chunk.tag,
                        id: chunk.id,
                        flags: write_flags,
                        label: chunk.label.clone(),
                        data,
                    });
                }
            }
            write_iff(&chunks).map_err(|e| e.to_string())?
        };

        if self.import_caf_ctx.is_none() {
            self.import_caf_ctx =
                Some(CafContext::from_install(&self.install_root).map_err(|e| e.to_string())?);
        }
        let ctx = self.import_caf_ctx.as_ref().expect("import_caf_ctx");
        let seed = self
            .import_rng_seed
            .wrapping_add(self.clock_us as u32)
            .max(1);
        self.import_rng_seed = self.import_rng_seed.wrapping_add(1).max(1);
        let mut rng = CrtRng::from_state(seed);
        let created = match import_family_fam(
            ctx,
            &self.store,
            &self.neighborhood_id,
            &fam_bytes,
            US_ENGLISH,
            &mut rng,
        ) {
            Ok(created) => created,
            Err(sims_vm::caf::CafError::Content(sims_content::Error::Unsupported(_)))
                if ctx.wardrobe.is_some() =>
            {
                // Some base-game / pre-Hot-Date `.FAM` archives hit unported
                // legacy outfit replacement generators in `refresh_outfits`;
                // fall back to preserving the archive's original body strings.
                let fallback_ctx = CafContext {
                    template_iff: ctx.template_iff.clone(),
                    catalog_guids: ctx.catalog_guids.clone(),
                    wardrobe: None,
                    starting_funds: ctx.starting_funds,
                };
                let mut fallback_rng = CrtRng::from_state(seed);
                import_family_fam(
                    &fallback_ctx,
                    &self.store,
                    &self.neighborhood_id,
                    &fam_bytes,
                    US_ENGLISH,
                    &mut fallback_rng,
                )
                .map_err(|e| e.to_string())?
            }
            Err(err) => return Err(err.to_string()),
        };
        self.family_names
            .insert(created.family_id, created.family_name.clone());
        save_persisted_caf_name(
            &self.saves_path,
            &self.neighborhood_id,
            self.stored.instance_id,
            created.family_id,
            Some(&created.family_name),
        );
        self.refresh_from_store()?;
        Ok(created)
    }

    /// Opens the native paginated Credits/About modal loaded from
    /// `GameData/Credits.iff` (with fallback to `GameData/UIText.iff` `STR# 163`).
    pub fn open_credits_modal(&mut self) {
        let title = self
            .chrome_strings
            .bar
            .get(sims_ui::neighborhood::BarButton::Credits.tooltip_index())
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| "Credits".into());
        let mut pages =
            sims_ui::book::load_credits_pages(&self.install, US_ENGLISH).unwrap_or_default();
        if pages.is_empty() {
            pages.push(BookPage {
                heading: None,
                entries: vec![BookEntry {
                    title: title.clone(),
                    subtitle: None,
                    body: "Credits data is unavailable in this install.".into(),
                    badges: Vec::new(),
                    highlight: false,
                }],
            });
        }
        self.clear_hover_state();
        self.credits_snapshot = Some(ReferenceBookSnapshot {
            title,
            subtitle: None,
            held_summary: Vec::new(),
            pages,
        });
        self.credits_dialog.open();
    }

    pub fn close_credits_modal(&mut self) {
        self.credits_dialog.close();
        self.credits_snapshot = None;
    }

    // =========================================================================
    // CAF Simlish Voice Preview draining
    // =========================================================================

    fn drain_caf_voice_previews(&mut self) {
        let previews = match &mut self.screen {
            ActiveScreen::CreateFamily(caf) => caf.take_voice_previews(),
            _ => Vec::new(),
        };
        for preview in previews {
            self.audio_queue.push(Input::PlayVoice {
                voice: preview.voice,
                category: preview.category,
                pitch: preview.pitch,
                seed: preview.seed,
                object: None,
            });
        }
    }

    // =========================================================================
    // Photo Album / Family Storybook Modal (`sims_ui::album` + `PhotoAlbumStore`)
    // =========================================================================

    /// Opens the Photo Album modal for the active lot's household (or hovered /
    /// first neighborhood household when on the Neighborhood map).
    pub fn open_photo_album(&mut self) -> Result<(), String> {
        let family_id = match &self.screen {
            ActiveScreen::Lot(session) => {
                if let Some(trip) = &session.active_trip {
                    Some(trip.home_family)
                } else {
                    self.stored
                        .family_in(session.house)
                        .map(|(id, _)| id)
                        .or_else(|| {
                            u16::try_from(
                                session
                                    .game
                                    .lot()
                                    .world()
                                    .globals
                                    .get(9)
                                    .copied()
                                    .unwrap_or(0),
                            )
                            .ok()
                            .filter(|&id| id != 0)
                        })
                }
            }
            _ => self
                .hovered_lot_id
                .and_then(|lot| self.stored.family_in(u32::from(lot)).map(|(id, _)| id))
                .or_else(|| self.stored.families.keys().next().copied()),
        }
        .ok_or_else(|| "no household available for Photo Album".to_owned())?;
        self.open_photo_album_for_family(family_id)
    }

    /// Opens the Photo Album modal for a specific `family_id` in the active
    /// neighborhood, acquiring a fenced [`AlbumFamilyToken`] from REDB.
    pub fn open_photo_album_for_family(&mut self, family_id: u16) -> Result<(), String> {
        let nh_id = match &self.screen {
            ActiveScreen::Lot(session) => session.neighborhood_id.clone(),
            _ => self.neighborhood_id.clone(),
        };
        let token = self
            .store
            .albums()
            .family_token(&nh_id, family_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("family {family_id} not found in neighborhood {nh_id:?}"))?;
        let (snapshot, has_image) = self.build_album_snapshot(&nh_id, family_id, None)?;
        self.clear_hover_state();
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.close_camera_strip();
        }
        self.album_token = Some(token);
        self.album_neighborhood_id = Some(nh_id);
        self.album_has_image = has_image;
        self.album_strip_scroll = 0;
        self.album_strip_pressed = None;
        self.album_dialog.open(&snapshot);
        self.album_snapshot = Some(snapshot);
        Ok(())
    }

    /// Closes the Photo Album modal cleanly.
    pub fn close_photo_album(&mut self) {
        self.album_dialog.close();
        self.album_snapshot = None;
        self.album_token = None;
        self.album_neighborhood_id = None;
        self.album_has_image.clear();
        self.album_strip_scroll = 0;
        self.album_strip_pressed = None;
    }

    fn build_album_snapshot(
        &self,
        nh_id: &str,
        family_id: u16,
        selected_hint: Option<PhotoId>,
    ) -> Result<(AlbumSnapshot, BTreeSet<PhotoId>), String> {
        let summaries = self
            .store
            .albums()
            .list_summaries(nh_id, family_id)
            .map_err(|e| e.to_string())?;
        let family_name = self
            .family_names
            .get(&family_id)
            .cloned()
            .unwrap_or_else(|| format!("Family {family_id}"));
        let focus_idx = selected_hint
            .or_else(|| self.album_dialog.selected_id())
            .and_then(|sel| summaries.iter().position(|s| s.id == sel.0))
            .unwrap_or(0);
        let (win_start, win_end) = album_hydration_window(summaries.len(), focus_idx);
        let visible_slots = self
            .album_dialog
            .layout(self.size[0], self.size[1])
            .thumb_slots
            .len();
        let strip_start = self
            .album_strip_scroll
            .min(summaries.len().saturating_sub(visible_slots));
        let strip_end = (strip_start + visible_slots + 2).min(summaries.len());
        let mut entries = Vec::with_capacity(summaries.len());
        let mut has_image = BTreeSet::new();
        for (idx, s) in summaries.into_iter().enumerate() {
            let pid = PhotoId(s.id);
            let entry_has_img = s.image_dimensions.is_some();
            if entry_has_img {
                has_image.insert(pid);
            }
            let in_sel = idx >= win_start && idx < win_end;
            let in_strip = idx >= strip_start && idx < strip_end;
            let (image, thumbnail) = if entry_has_img && (in_sel || in_strip) {
                let loaded = self
                    .store
                    .albums()
                    .get(nh_id, family_id, s.id)
                    .map_err(|e| e.to_string())?
                    .and_then(|p| p.image)
                    .map(|img| Image {
                        width: img.width,
                        height: img.height,
                        rgba: img.to_rgba(),
                    });
                let thumb = loaded
                    .as_ref()
                    .map(|img| downscale_album_thumbnail(img, 78, 60));
                if in_sel {
                    (loaded, thumb)
                } else {
                    (None, thumb)
                }
            } else {
                (None, None)
            };
            entries.push(AlbumEntry {
                id: pid,
                title: s.title,
                caption: s.caption,
                image,
                thumbnail,
                sim_day: s.sim_day,
            });
        }
        Ok((
            AlbumSnapshot {
                family_name,
                entries,
                can_capture_snapshot: matches!(self.screen, ActiveScreen::Lot(_)),
            },
            has_image,
        ))
    }

    fn sync_album_strip_scroll_to_selection(&mut self) {
        let Some(selected) = self.album_dialog.selected_id() else {
            return;
        };
        let Some(snapshot) = &self.album_snapshot else {
            return;
        };
        let total = snapshot.entries.len();
        let visible_slots = self
            .album_dialog
            .layout(self.size[0], self.size[1])
            .thumb_slots
            .len();
        if visible_slots == 0 || total == 0 {
            self.album_strip_scroll = 0;
            return;
        }
        let max_scroll = total.saturating_sub(visible_slots);
        if let Some(idx) = snapshot.entries.iter().position(|e| e.id == selected) {
            if idx < self.album_strip_scroll {
                self.album_strip_scroll = idx;
            } else if idx >= self.album_strip_scroll + visible_slots {
                self.album_strip_scroll = idx + 1 - visible_slots;
            }
        }
        self.album_strip_scroll = self.album_strip_scroll.min(max_scroll);
    }

    fn hydrate_album_images(&mut self) {
        let (Some(nh_id), Some(token)) = (self.album_neighborhood_id.clone(), self.album_token)
        else {
            return;
        };
        let visible_slots = self
            .album_dialog
            .layout(self.size[0], self.size[1])
            .thumb_slots
            .len();
        let selected = self.album_dialog.selected_id();
        let Some(snapshot) = self.album_snapshot.as_mut() else {
            return;
        };
        let total = snapshot.entries.len();
        if total == 0 {
            self.album_strip_scroll = 0;
            return;
        }
        let focus_idx = selected
            .and_then(|sel| snapshot.entries.iter().position(|e| e.id == sel))
            .unwrap_or(0);
        let (win_start, win_end) = album_hydration_window(total, focus_idx);
        let max_scroll = total.saturating_sub(visible_slots);
        self.album_strip_scroll = self.album_strip_scroll.min(max_scroll);
        let strip_start = self.album_strip_scroll.saturating_sub(1);
        let strip_end = (self.album_strip_scroll + visible_slots + 2).min(total);
        for (idx, entry) in snapshot.entries.iter_mut().enumerate() {
            let in_sel = idx >= win_start && idx < win_end;
            let in_strip = idx >= strip_start && idx < strip_end;
            let has_img = self.album_has_image.contains(&entry.id);
            if in_sel {
                if has_img
                    && entry.image.is_none()
                    && let Ok(Some(photo)) =
                        self.store.albums().get(&nh_id, token.family_id, entry.id.0)
                    && let Some(img) = photo.image
                {
                    let full = Image {
                        width: img.width,
                        height: img.height,
                        rgba: img.to_rgba(),
                    };
                    if entry.thumbnail.is_none() {
                        entry.thumbnail = Some(downscale_album_thumbnail(&full, 78, 60));
                    }
                    entry.image = Some(full);
                }
            } else {
                // Keep full-resolution memory bounded even for large albums.
                entry.image = None;
                if in_strip
                    && has_img
                    && entry.thumbnail.is_none()
                    && let Ok(Some(photo)) =
                        self.store.albums().get(&nh_id, token.family_id, entry.id.0)
                    && let Some(img) = photo.image
                {
                    let full = Image {
                        width: img.width,
                        height: img.height,
                        rgba: img.to_rgba(),
                    };
                    entry.thumbnail = Some(downscale_album_thumbnail(&full, 78, 60));
                }
            }
        }
    }

    fn reload_album_snapshot(&mut self, select_id: Option<PhotoId>) -> Result<(), String> {
        let (Some(nh_id), Some(token)) = (self.album_neighborhood_id.clone(), self.album_token)
        else {
            return Err("Photo Album is not open".into());
        };
        let (snapshot, has_image) =
            self.build_album_snapshot(&nh_id, token.family_id, select_id)?;
        self.album_has_image = has_image;
        self.album_dialog.sync_snapshot(&snapshot);
        if let Some(id) = select_id {
            self.album_dialog.select_entry(&snapshot, id);
        }
        self.album_snapshot = Some(snapshot);
        self.sync_album_strip_scroll_to_selection();
        self.hydrate_album_images();
        Ok(())
    }

    fn send_album_key(&mut self, key: AlbumKey) -> ShellOutcome {
        let cmds = self
            .album_snapshot
            .clone()
            .map(|snap| {
                self.album_dialog
                    .handle_key(key, self.size, &snap, &self.album_art)
            })
            .unwrap_or_default();
        let out = self.execute_album_commands(cmds);
        self.sync_album_strip_scroll_to_selection();
        self.hydrate_album_images();
        out
    }

    fn execute_album_commands(&mut self, cmds: Vec<AlbumCommand>) -> ShellOutcome {
        let mut out = ShellOutcome::redraw();
        for cmd in cmds {
            let step = self.execute_album_command(cmd);
            out.redraw |= step.redraw;
            out.quit |= step.quit;
            out.photo |= step.photo;
        }
        out
    }

    /// Executes a single [`AlbumCommand`] emitted by [`PhotoAlbumDialog`] against
    /// REDB's fenced [`sims_sim::album_store::PhotoAlbumStore`].
    pub fn execute_album_command(&mut self, cmd: AlbumCommand) -> ShellOutcome {
        match cmd {
            AlbumCommand::Close => {
                self.close_photo_album();
                ShellOutcome::redraw()
            }
            AlbumCommand::SelectEntry(id) => {
                if let Some(snap) = &self.album_snapshot {
                    self.album_dialog.select_entry(snap, id);
                }
                self.sync_album_strip_scroll_to_selection();
                self.hydrate_album_images();
                ShellOutcome::redraw()
            }
            AlbumCommand::CaptureSnapshot => {
                let (Some(nh_id), Some(token)) =
                    (self.album_neighborhood_id.clone(), self.album_token)
                else {
                    return ShellOutcome::redraw();
                };
                let capture = if let ActiveScreen::Lot(session) = &mut self.screen {
                    session.game.render();
                    let canvas = session.game.canvas();
                    let sim_day = u32::try_from(
                        session
                            .game
                            .lot()
                            .world()
                            .globals
                            .get(28)
                            .copied()
                            .unwrap_or(0)
                            .max(0),
                    )
                    .ok()
                    .map(|d| d + 1);
                    let lot_title = {
                        let name = session.game.lot().lot_name().trim();
                        if !name.is_empty() {
                            name.to_owned()
                        } else {
                            format!("Lot {}", session.house)
                        }
                    };
                    capture_canvas_to_album_image(canvas).map(|img| (lot_title, img, sim_day))
                } else {
                    Err("snapshot capture requires an open lot".to_owned())
                };
                match capture {
                    Ok((title, img, sim_day)) => {
                        let mut entry = NewAlbumPhoto::with_image(title, "", img);
                        if let Some(day) = sim_day {
                            entry = entry.with_sim_day(day);
                        }
                        match self.store.albums().add(&nh_id, token, entry) {
                            Ok(photo) => {
                                if let Err(err) =
                                    self.reload_album_snapshot(Some(PhotoId(photo.id)))
                                {
                                    self.set_note(format!(
                                        "Saved snapshot; album reload error: {err}"
                                    ));
                                }
                            }
                            Err(err) => {
                                self.set_note(format!("Could not save snapshot: {err}"));
                            }
                        }
                    }
                    Err(err) => {
                        self.set_note(format!("Snapshot capture failed: {err}"));
                    }
                }
                ShellOutcome::redraw()
            }
            AlbumCommand::AddBlankPage => {
                if let (Some(nh_id), Some(token)) =
                    (self.album_neighborhood_id.clone(), self.album_token)
                {
                    let page = NewAlbumPhoto::blank("", "");
                    match self.store.albums().add(&nh_id, token, page) {
                        Ok(photo) => {
                            if let Err(err) = self.reload_album_snapshot(Some(PhotoId(photo.id))) {
                                self.set_note(format!("Added page; album reload error: {err}"));
                            }
                        }
                        Err(err) => {
                            self.set_note(format!("Could not add album page: {err}"));
                        }
                    }
                }
                ShellOutcome::redraw()
            }
            AlbumCommand::UpdateEntryText { id, title, caption } => {
                if let (Some(nh_id), Some(token)) =
                    (self.album_neighborhood_id.clone(), self.album_token)
                {
                    match self
                        .store
                        .albums()
                        .update_text(&nh_id, token, id.0, &title, &caption)
                    {
                        Ok(_) => {
                            if let Err(err) = self.reload_album_snapshot(Some(id)) {
                                self.set_note(format!("Updated entry; album reload error: {err}"));
                            }
                        }
                        Err(err) => {
                            self.set_note(format!("Could not update album text: {err}"));
                        }
                    }
                }
                ShellOutcome::redraw()
            }
            AlbumCommand::MoveEntry { id, new_index } => {
                if let (Some(nh_id), Some(token)) =
                    (self.album_neighborhood_id.clone(), self.album_token)
                {
                    match self
                        .store
                        .albums()
                        .move_entry(&nh_id, token, id.0, new_index)
                    {
                        Ok(_) => {
                            if let Err(err) = self.reload_album_snapshot(Some(id)) {
                                self.set_note(format!(
                                    "Reordered entry; album reload error: {err}"
                                ));
                            }
                        }
                        Err(err) => {
                            self.set_note(format!("Could not reorder album entry: {err}"));
                        }
                    }
                }
                ShellOutcome::redraw()
            }
            AlbumCommand::DeleteEntry(id) => {
                if let (Some(nh_id), Some(token)) =
                    (self.album_neighborhood_id.clone(), self.album_token)
                {
                    match self.store.albums().delete(&nh_id, token, id.0) {
                        Ok(_) => {
                            if let Err(err) = self.reload_album_snapshot(None) {
                                self.set_note(format!("Deleted entry; album reload error: {err}"));
                            }
                        }
                        Err(err) => {
                            self.set_note(format!("Could not delete album entry: {err}"));
                        }
                    }
                }
                ShellOutcome::redraw()
            }
        }
    }

    // =========================================================================
    // Travel Destinations Shell (`TravelChrome`, `TravelDialog`, `sims_vm::travel`)
    // =========================================================================

    fn sync_travel_state(&mut self) {
        if let Some(dest) = destination_for_kind(self.kind) {
            self.travel_state.destination = dest;
        }
        self.travel_state.mode = match self.nbhd_state.mode {
            NeighborhoodMode::Bulldoze => TravelBarMode::Bulldoze,
            NeighborhoodMode::Visit => TravelBarMode::Visit,
            _ => TravelBarMode::Browse,
        };
        self.travel_state.expansions = self.nbhd_state.expansions;
        self.travel_state.vacation_has_no_vacancy = self
            .map
            .lots
            .iter()
            .any(|l| (40..=48).contains(&l.id) && l.has_vacation_occupancy);
        // Show the first occupied household's funds & MagiCoins as a quick status badge.
        if let Some((&fam_id, fam)) = self.stored.families.iter().find(|(_, f)| f.house != 0) {
            self.travel_state.simoleons = fam.budget;
            self.travel_state.magic_coins = lifecycle::family_magic_coins(&self.stored, fam_id);
        }
    }

    fn magic_family_bin_rect(&self, layout: &TravelLayout) -> Option<Rect> {
        if self.kind != NeighborhoodKind::MagicTown {
            return None;
        }
        // On native viewports (>= 800px), place the Family Bin button in the open
        // 78px left margin of `TravelChrome`'s top bar. On compact viewports
        // (< 800px), top bar controls start at x=4, so dock the compact button on
        // the open right side of `filter_strip_rect` (Magic Town only has 2 filter
        // chips on the left) so Family Bin stays reachable at every viewport width.
        let rect = if layout.compact {
            let btn_w = 54.min((layout.filter_strip_rect.width - 8).max(0));
            let btn_h = (layout.filter_strip_rect.height - 4).clamp(0, 26);
            let x = layout.filter_strip_rect.x + layout.filter_strip_rect.width - btn_w - 4;
            let y = layout.filter_strip_rect.y + (layout.filter_strip_rect.height - btn_h) / 2;
            if btn_w <= 0 || btn_h <= 0 {
                return None;
            }
            Rect::new(x, y, btn_w, btn_h)
        } else {
            let btn_w = 53;
            let btn_h = (layout.bar_rect.height - 6).clamp(0, 56);
            let x = layout.bar_rect.x + 12;
            let y = (layout.bar_rect.height - btn_h) / 2;
            if btn_w <= 0 || btn_h <= 0 {
                return None;
            }
            Rect::new(x, y, btn_w, btn_h)
        };
        // Never overlap any TravelChrome control.
        if layout.controls.iter().any(|c| {
            rect.x < c.rect.x + c.rect.width
                && rect.x + rect.width > c.rect.x
                && rect.y < c.rect.y + c.rect.height
                && rect.y + rect.height > c.rect.y
        }) {
            return None;
        }
        Some(rect)
    }

    fn draw_magic_family_bin_button(&self, frame: &mut Frame) {
        let layout = TravelLayout::compute(frame.width(), frame.height(), &self.travel_state);
        let Some(rect) = self.magic_family_bin_rect(&layout) else {
            return;
        };
        let state = if self.magic_family_bin_pressed {
            2
        } else if self.magic_family_bin_hovered {
            1
        } else {
            0
        };
        if !layout.compact
            && let Some(img) = self
                .chrome_art
                .image(sims_ui::neighborhood::art_id::SELECT_FAMILY)
            && img.width >= 4
            && img.height > 0
            && rect.height >= img.height as i32
        {
            let sw = (img.width / 4) as i32;
            let sh = img.height as i32;
            let src = Rect::new(state * sw, 0, sw, sh);
            let dx = rect.x + (rect.width - sw) / 2;
            let dy = rect.y + (rect.height - sh) / 2;
            frame.draw(img, src, [dx, dy]);
        } else {
            let fill = if self.magic_family_bin_pressed {
                [56, 74, 114]
            } else if self.magic_family_bin_hovered {
                [84, 108, 158]
            } else {
                [66, 86, 130]
            };
            frame.fill(rect, fill);
            if let Some(font) = self.chrome_art.font(10) {
                let text_y = rect.y + ((rect.height - 10) / 2).max(2);
                font.draw(frame, "Fam", [rect.x + 12, text_y], [245, 245, 255]);
            }
        }
    }

    fn travel_filtered_lot_ids(&self) -> BTreeSet<u16> {
        let filter_mask = self
            .travel_state
            .active_filter
            .map(|f| community_filter_for_lot_filter(f) as u16);
        let lodging = if self.kind == NeighborhoodKind::Vacation {
            self.travel_state.vacation_lodging_filter
        } else {
            None
        };
        if filter_mask.is_none() && lodging.is_none() {
            return BTreeSet::new();
        }
        self.map
            .lots
            .iter()
            .filter(|lot| {
                let matches_filter = filter_mask.is_none_or(|mask| (lot.filter_mask & mask) != 0);
                let matches_lodging = lodging.is_none_or(|l| vacation_lot_matches_lodging(lot, l));
                matches_filter && matches_lodging
            })
            .map(|lot| lot.id)
            .collect()
    }

    /// Handles a [`TravelCommand`] emitted by [`TravelChrome`] on expansion
    /// sub-neighborhood maps.
    pub fn handle_travel_command(&mut self, cmd: TravelCommand) -> ShellOutcome {
        match cmd {
            TravelCommand::ReturnToNeighborhood => {
                let home = default_map_kind(&self.install, self.slot);
                if let Err(e) = self.switch_kind(home) {
                    self.note = Some(format!("Could not return to neighborhood map: {e}"));
                }
                ShellOutcome::redraw()
            }
            TravelCommand::ToggleBulldoze => {
                if self.travel_state.mode == TravelBarMode::Bulldoze {
                    self.travel_state.mode = TravelBarMode::Browse;
                    self.nbhd_state.mode = NeighborhoodMode::Normal;
                } else {
                    self.travel_state.mode = TravelBarMode::Bulldoze;
                    self.nbhd_state.mode = NeighborhoodMode::Bulldoze;
                }
                self.refresh_hovered_lot();
                ShellOutcome::redraw()
            }
            TravelCommand::ImportLot => {
                self.note = Some("Community lot import is not ported yet.".into());
                ShellOutcome::redraw()
            }
            TravelCommand::SwitchDestination(dest) => {
                if let Err(e) = self.switch_kind(kind_for_destination(dest)) {
                    self.note = Some(format!("Could not switch destination map: {e}"));
                }
                ShellOutcome::redraw()
            }
            TravelCommand::SetFilter(filter) => {
                self.travel_state.active_filter = filter;
                ShellOutcome::redraw()
            }
            TravelCommand::SetVacationLodging(lodging) => {
                self.travel_state.vacation_lodging_filter = lodging;
                ShellOutcome::redraw()
            }
        }
    }

    /// Opens an unoccupied travel destination community lot in Build/Buy edit
    /// mode via [`vm_travel::open_destination_for_edit`]. Edits persist via
    /// [`Self::save_current_lot`] / [`Self::leave_lot`].
    pub fn open_destination_for_edit(&mut self, destination_house: u32) -> Result<(), String> {
        self.start_load_audio();
        self.present_loading_step(
            loading_kind_for_house(destination_house),
            Some(format!("Destination Lot {destination_house}")),
            0.2,
        );
        let snapshot = match self
            .store
            .load_lot_snapshot(&self.neighborhood_id, destination_house)
            .map_err(|e| e.to_string())
            .and_then(|opt| {
                opt.ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))
            }) {
            Ok(s) => s,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        let lot = match vm_travel::open_destination_for_edit(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            destination_house,
            &self.answers,
            self.frames.clone(),
        ) {
            Ok(lot) => lot,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err.to_string());
            }
        };
        let install_assets = self.ensure_install_assets().ok();
        let mut game = match Game::from_loaded_lot_cached(
            &self.install_root,
            lot,
            Vec::new(),
            &snapshot.character_files,
            snapshot.base_house.as_deref(),
            self.play,
            install_assets.as_ref(),
        ) {
            Ok(game) => game,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        if self.options_file.is_file() {
            let _ = game.load_options(&self.options_file);
        }
        game.coop_set_player_name(self.coop_player_name.clone());
        game.coop_set_iroh_config(self.coop_iroh_config);
        game.coop_set_travel_blocked(true);
        game.set_album_button_enabled(true);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());
        self.stored = snapshot.neighborhood;
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: self.neighborhood_id.clone(),
            neighborhood_instance_id: self.stored.instance_id,
            neighborhood_revision: self.stored.revision,
            staging: None,
            house: destination_house,
            last_days_played: initial_days_played,
            active_trip: None,
            is_destination_edit: true,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.dialog = None;
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = Some(destination_house);
        self.note = None;
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
        self.pressed_lot_id = None;
        self.nbhd_state.hovered_lot = None;
        self.stop_load_audio(next_music);
        Ok(())
    }

    /// Opens [`TravelDialog`] from the currently open home lot using the first
    /// eligible Complete Collection travel destination (defaulting to Downtown,
    /// falling back across Vacation Island, Old Town Community, Studio Town,
    /// and Magic Town). Desktop users can press `Left`/`Right`/`Tab` inside the
    /// open dialog to cycle destinations before confirming.
    pub fn open_default_travel_dialog(&mut self) -> Result<ValidatedTravelPlan, String> {
        let mut last_err = "no travel destinations available".to_owned();
        for dest in [
            TravelDestination::Downtown,
            TravelDestination::VacationIsland,
            TravelDestination::OldTown,
            TravelDestination::StudioTown,
            TravelDestination::MagicTown,
        ] {
            match self.open_travel_dialog_for_destination(dest) {
                Ok(plan) => return Ok(plan),
                Err(err) => last_err = err,
            }
        }
        Err(last_err)
    }

    /// Opens [`TravelDialog`] for `destination` from the currently open home lot,
    /// selecting the first available community lot for `destination` and the
    /// appropriate traveling party (selected Sim or whole household).
    pub fn open_travel_dialog_for_destination(
        &mut self,
        destination: TravelDestination,
    ) -> Result<ValidatedTravelPlan, String> {
        let dest_house = self
            .default_house_for_destination(destination)
            .ok_or_else(|| format!("no community lot found for {destination:?}"))?;
        let leader_guid = {
            let ActiveScreen::Lot(session) = &self.screen else {
                return Err("enter a home lot before traveling".into());
            };
            let world = session.game.lot().world();
            let home_family_id = self
                .stored
                .family_in(session.house)
                .map(|(id, _)| id)
                .or_else(|| {
                    u16::try_from(world.globals.get(9).copied().unwrap_or(0))
                        .ok()
                        .filter(|&id| id != 0)
                })
                .ok_or_else(|| format!("lot {} has no resident household", session.house))?;
            let fam_members: Vec<u32> = self
                .stored
                .families
                .get(&home_family_id)
                .map(|f| f.members.clone())
                .unwrap_or_default();
            let selected_guid = world
                .selected()
                .and_then(|sel| world.objects.get(&sel))
                .map(|o| o.guid)
                .filter(|g| fam_members.contains(g));
            selected_guid
                .or_else(|| fam_members.first().copied())
                .ok_or_else(|| format!("household {home_family_id} has no living members"))?
        };
        let party = match destination {
            TravelDestination::VacationIsland
            | TravelDestination::OldTown
            | TravelDestination::MagicTown => vm_travel::TravelParty::WholeFamily { leader_guid },
            TravelDestination::Downtown | TravelDestination::StudioTown => {
                vm_travel::TravelParty::Single {
                    sim_guid: leader_guid,
                }
            }
        };
        let request = TravelRequest {
            destination_house: dest_house,
            party,
            transport: None,
            accommodation: vm_travel::VacationAccommodation::None,
            fare_override: None,
            answers: self.answers.clone(),
            frames: self.frames.clone(),
            view_centre: None,
        };
        self.open_travel_dialog(request)
    }

    /// Cycles the open [`TravelDialog`] to the next (`delta > 0`) or previous
    /// (`delta < 0`) eligible destination without leaving the dialog.
    pub fn cycle_travel_destination(&mut self, delta: i32) -> bool {
        let Some(current) = self.pending_travel.as_ref().map(|p| p.plan.destination) else {
            return false;
        };
        const ORDER: [TravelDestination; 5] = [
            TravelDestination::Downtown,
            TravelDestination::VacationIsland,
            TravelDestination::OldTown,
            TravelDestination::StudioTown,
            TravelDestination::MagicTown,
        ];
        let start = ORDER.iter().position(|&d| d == current).unwrap_or(0);
        let step: isize = if delta >= 0 { 1 } else { -1 };
        for offset in 1..ORDER.len() {
            let idx =
                (start as isize + step * offset as isize).rem_euclid(ORDER.len() as isize) as usize;
            if self.open_travel_dialog_for_destination(ORDER[idx]).is_ok() {
                return true;
            }
        }
        false
    }

    fn default_house_for_destination(&self, dest: TravelDestination) -> Option<u32> {
        let candidates: &[u32] = match dest {
            TravelDestination::Downtown => &[21, 22, 23, 24, 25, 26, 27, 28, 29, 30],
            TravelDestination::VacationIsland => &[40, 41, 42, 43, 44, 45, 46, 47, 48],
            TravelDestination::OldTown => &[58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68],
            TravelDestination::StudioTown => &[81, 82, 83, 84, 85, 86, 87, 88, 89],
            TravelDestination::MagicTown => &[93, 94, 95, 96, 97, 98],
        };
        candidates.iter().copied().find(|&house| {
            TravelDestination::classify(house, &self.stored.zoning) == Some(dest)
                && self
                    .store
                    .load_lot_snapshot(&self.neighborhood_id, house)
                    .ok()
                    .flatten()
                    .is_some_and(|s| s.base_house.is_some() || s.saved_lot.is_some())
        })
    }

    /// Validates `request` against the currently open home lot via
    /// [`vm_travel::validate_travel_plan`] and opens [`TravelDialog`] for
    /// player confirmation.
    pub fn open_travel_dialog(
        &mut self,
        request: TravelRequest,
    ) -> Result<ValidatedTravelPlan, String> {
        let (plan, travelers, simoleons, magic_coins) = {
            let ActiveScreen::Lot(session) = &self.screen else {
                return Err("enter a home lot before traveling".into());
            };
            if session.game.coop_active() {
                return Err("Leave the co-op session before traveling to a destination.".into());
            }
            if session.active_trip.is_some() {
                return Err("household is already visiting a travel destination".into());
            }
            let world = session.game.lot().world();
            let home_family_id = self
                .stored
                .family_in(session.house)
                .map(|(id, _)| id)
                .or_else(|| {
                    u16::try_from(world.globals.get(9).copied().unwrap_or(0))
                        .ok()
                        .filter(|&id| id != 0)
                })
                .ok_or_else(|| format!("lot {} has no resident household", session.house))?;
            let plan = vm_travel::validate_travel_plan(
                world,
                session.house,
                home_family_id,
                u16::from(self.install.expansions.0),
                &self.stored,
                &request,
            )
            .map_err(|e| e.to_string())?;
            let travelers: Vec<String> = plan
                .members
                .iter()
                .map(|t| {
                    self.faces
                        .name(t.guid)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("Sim 0x{:08X}", t.guid))
                })
                .collect();
            let simoleons = sims_vm::available_funds(world).max(0);
            let members: Vec<u32> = self
                .stored
                .families
                .get(&home_family_id)
                .map(|f| f.members.clone())
                .unwrap_or_default();
            let magic_coins = vm_travel::family_magic_coins(world, &members).max(0);
            (plan, travelers, simoleons, magic_coins)
        };
        let ui_dest = ui_destination_for_vm(plan.destination);
        let mut td_state = TravelDialogState::for_destination(ui_dest, travelers);
        td_state.transport = ui_transport_for_vm(plan.transport);
        td_state.fare_simoleons = plan.fare;
        td_state.household_simoleons = simoleons;
        td_state.household_magic_coins = magic_coins;
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.close_camera_strip();
        }
        self.travel_dialog = TravelDialog::new();
        self.travel_dialog_state = Some(td_state);
        self.pending_travel = Some(PendingTravel {
            request,
            plan: plan.clone(),
        });
        Ok(plan)
    }

    /// Closes [`TravelDialog`] without departing.
    pub fn close_travel_dialog(&mut self) {
        self.travel_dialog_state = None;
        self.pending_travel = None;
    }

    fn handle_travel_dialog_command(&mut self, cmd: TravelDialogCommand) -> ShellOutcome {
        match cmd {
            TravelDialogCommand::Cancel => {
                self.close_travel_dialog();
                ShellOutcome::redraw()
            }
            TravelDialogCommand::Confirm { .. } => {
                let pending = self.pending_travel.take();
                self.travel_dialog_state = None;
                if let Some(pending) = pending {
                    let _ = &pending.plan;
                    if let Err(err) = self.depart_to_destination(pending.request) {
                        if let ActiveScreen::Lot(session) = &mut self.screen {
                            session.game.set_note(format!("Travel failed: {err}"));
                        } else if self.note.is_none() {
                            self.note = Some(format!("Travel failed: {err}"));
                        }
                    }
                }
                ShellOutcome::redraw()
            }
        }
    }

    /// Builds real neighborhood / destination lot entries (names, street
    /// addresses, descriptions, and zoning) from the live neighborhood store
    /// and installed `*Desc.iff` / `StreetNames.iff` tables for VM special lot
    /// pickers (`Dialog` kinds `5, 7, 8, 11, 13`).
    fn build_special_lot_catalog(&self) -> Vec<SpecialLotEntry> {
        let mut lot_ids: BTreeSet<u16> = BTreeSet::new();
        // Single key-scan over stored houses (no character-file copying), so
        // only lots that actually exist in the neighborhood store are offered
        // (never unbacked map metadata entries).
        if let Ok(stored_houses) = self.store.list_houses(&self.neighborhood_id) {
            for house in stored_houses {
                if let Ok(id) = u16::try_from(house) {
                    lot_ids.insert(id);
                }
            }
        }
        lot_ids
            .into_iter()
            .map(|lot_id| {
                let (name, description) = if let Some(meta) = self.lot_meta.get(&lot_id) {
                    (meta.lot_name.clone(), meta.lot_description.clone())
                } else {
                    self.descriptions.lot_info(lot_id)
                };
                let address = self.street_names.address_for(lot_id);
                let zoning =
                    self.stored
                        .zoning
                        .get(&(lot_id as i16))
                        .copied()
                        .unwrap_or(match lot_id {
                            21..=30 | 40..=48 | 81..=89 | 93..=98 => 2,
                            _ => 1,
                        }) as i16;
                SpecialLotEntry {
                    lot_id,
                    name,
                    address,
                    description,
                    zoning,
                }
            })
            .collect()
    }

    /// Executes a destination travel transition requested by answering a VM
    /// special lot picker (`Dialog` kinds `5, 7, 8, 11, 13`). Never halts or
    /// strands: when departure is ineligible (e.g., co-op active, already away,
    /// or missing destination snapshot), stays cleanly on the current lot and
    /// surfaces a native notice dialog without overwriting active VM dialogs.
    fn handle_special_panel_travel(
        &mut self,
        request: sims_vm::SpecialPanelRequest,
        dest_house: u32,
    ) {
        let report_ineligible = |screen: &mut ActiveScreen, msg: String| {
            if let ActiveScreen::Lot(session) = screen {
                if !session.game.has_dialog() {
                    session.game.show_notice("Travel", msg);
                } else {
                    session.game.set_note(msg);
                }
            }
        };
        let Some(destination) = TravelDestination::classify(dest_house, &self.stored.zoning) else {
            report_ineligible(
                &mut self.screen,
                format!("Lot {dest_house} is not a travel destination."),
            );
            return;
        };
        let leader_guid = {
            let ActiveScreen::Lot(session) = &self.screen else {
                return;
            };
            if session.game.coop_active() {
                report_ineligible(
                    &mut self.screen,
                    "Leave the co-op session before traveling to a destination.".into(),
                );
                return;
            }
            if session.active_trip.is_some() {
                report_ineligible(
                    &mut self.screen,
                    "Return home before starting another destination trip.".into(),
                );
                return;
            }
            if session.is_destination_edit {
                report_ineligible(
                    &mut self.screen,
                    "Travel is unavailable while editing a destination lot.".into(),
                );
                return;
            }
            let world = session.game.lot().world();
            let home_family_id = self
                .stored
                .family_in(session.house)
                .map(|(id, _)| id)
                .or_else(|| {
                    u16::try_from(world.globals.get(9).copied().unwrap_or(0))
                        .ok()
                        .filter(|&id| id != 0)
                });
            let Some(home_family_id) = home_family_id else {
                report_ineligible(
                    &mut self.screen,
                    "No resident household is active on this lot for travel.".into(),
                );
                return;
            };
            let fam_members: Vec<u32> = self
                .stored
                .families
                .get(&home_family_id)
                .map(|f| f.members.clone())
                .unwrap_or_default();
            let selected_guid = world
                .selected()
                .and_then(|sel| world.objects.get(&sel))
                .map(|o| o.guid)
                .filter(|g| fam_members.contains(g));
            let Some(guid) = selected_guid.or_else(|| fam_members.first().copied()) else {
                report_ineligible(
                    &mut self.screen,
                    "No household member is available to travel.".into(),
                );
                return;
            };
            guid
        };
        let party = match destination {
            TravelDestination::VacationIsland
            | TravelDestination::OldTown
            | TravelDestination::MagicTown => vm_travel::TravelParty::WholeFamily { leader_guid },
            TravelDestination::Downtown | TravelDestination::StudioTown => {
                vm_travel::TravelParty::Single {
                    sim_guid: leader_guid,
                }
            }
        };
        let transport = match request {
            sims_vm::SpecialPanelRequest::MagicTownLotPicker { via_blimp: false } => {
                Some(vm_travel::TravelTransport::MagicHole)
            }
            sims_vm::SpecialPanelRequest::MagicTownLotPicker { via_blimp: true } => {
                Some(vm_travel::TravelTransport::MagicBlimp)
            }
            sims_vm::SpecialPanelRequest::StudioTownLotPicker { star_limo: true } => {
                Some(vm_travel::TravelTransport::StudioLimo)
            }
            _ => None,
        };
        let travel_req = TravelRequest {
            destination_house: dest_house,
            party,
            transport,
            accommodation: vm_travel::VacationAccommodation::None,
            fare_override: None,
            answers: self.answers.clone(),
            frames: self.frames.clone(),
            view_centre: None,
        };
        if let Err(err) = self.depart_to_destination(travel_req) {
            report_ineligible(
                &mut self.screen,
                format!("Could not travel to lot {dest_house}: {err}"),
            );
        }
    }

    /// Departs from the currently open home lot to a travel destination via
    /// [`vm_travel::depart_to_destination`], driving [`LoadingScreen`] audio
    /// cues and wrapping the booted destination lot into the active session.
    pub fn depart_to_destination(
        &mut self,
        request: TravelRequest,
    ) -> Result<TravelSession, String> {
        self.note = None;
        self.flush_background_saves();
        let nh_id = match &self.screen {
            ActiveScreen::Lot(session) => {
                if session.game.coop_active() {
                    return Err("Leave the co-op session before traveling to a destination.".into());
                }
                if session.active_trip.is_some() {
                    return Err("already on an active travel trip".into());
                }
                session.neighborhood_id.clone()
            }
            _ => return Err("no home lot is open for travel departure".into()),
        };
        // Pre-validate snapshot availability BEFORE committing `TRVL` to REDB.
        let _ = self
            .store
            .load_lot_snapshot(&nh_id, request.destination_house)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {nh_id:?} not found before departure"))?;

        self.start_load_audio();
        let load_kind = loading_kind_for_travel(request.destination_house, request.transport);
        self.present_loading_step(
            load_kind,
            Some(format!("Destination Lot {}", request.destination_house)),
            0.2,
        );
        let departure = {
            let ActiveScreen::Lot(session) = &mut self.screen else {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err("no home lot is open for travel departure".into());
            };
            match vm_travel::depart_to_destination(
                &self.install_root,
                &self.store,
                &nh_id,
                session.game.lot_mut(),
                &request,
            ) {
                Ok(dep) => dep,
                Err(err) => {
                    let fallback = self.desired_music_mode();
                    self.stop_load_audio(fallback);
                    return Err(err.to_string());
                }
            }
        };
        self.present_loading_step(
            loading_kind_for_travel(departure.session.destination_house, request.transport),
            Some(format!(
                "Destination Lot {}",
                departure.session.destination_house
            )),
            0.75,
        );
        let trip = departure.session;
        let dest_house = trip.destination_house;
        let install_assets = self.ensure_install_assets().ok();
        let post_commit_res: Result<(Game, StoredNeighborhood), String> = (|| {
            let snapshot = self
                .store
                .load_lot_snapshot(&nh_id, dest_house)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("neighborhood {nh_id:?} not found after departure"))?;
            let members = snapshot
                .neighborhood
                .families
                .get(&trip.home_family)
                .map(|f| f.members.clone())
                .unwrap_or_default();
            let game = Game::from_loaded_lot_cached(
                &self.install_root,
                departure.destination_lot,
                members,
                &snapshot.character_files,
                snapshot.base_house.as_deref(),
                self.play,
                install_assets.as_ref(),
            )?;
            Ok((game, snapshot.neighborhood))
        })();
        let (mut game, next_stored) = match post_commit_res {
            Ok(ok) => ok,
            Err(err) => {
                // Departure already committed `TRVL` to REDB: never leave the
                // frozen home lot on screen with `active_trip == None`. Return
                // cleanly to the Neighborhood map so clicking `trip.home_house`
                // offers Resume or Abort.
                self.screen = ActiveScreen::Neighborhood;
                self.lot_generation = self.lot_generation.wrapping_add(1);
                self.close_photo_album();
                self.close_travel_dialog();
                self.save_manager.close();
                self.save_manager_snapshot = None;
                self.save_manager_target_lot = None;
                let _ = self.refresh_from_store();
                self.note = Some(format!(
                    "Trip departed (saved in REDB), but destination lot {dest_house} could not open: {err}. Click lot {} to Resume or Abort.",
                    trip.home_house
                ));
                self.audio_queue
                    .push(Input::Events(vec![SoundEvent::StopAll]));
                self.stop_load_audio(music_mode_for_kind(self.kind));
                return Err(err);
            }
        };
        if self.options_file.is_file() {
            let _ = game.load_options(&self.options_file);
        }
        game.coop_set_player_name(self.coop_player_name.clone());
        game.coop_set_iroh_config(self.coop_iroh_config);
        game.coop_set_travel_blocked(true);
        game.set_album_button_enabled(true);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());
        self.stored = next_stored;
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: nh_id,
            neighborhood_instance_id: self.stored.instance_id,
            neighborhood_revision: self.stored.revision,
            staging: None,
            house: dest_house,
            last_days_played: initial_days_played,
            active_trip: Some(trip.clone()),
            is_destination_edit: false,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = None;
        self.note = None;
        self.audio_queue
            .push(Input::Events(vec![SoundEvent::StopAll]));
        self.stop_load_audio(next_music);
        Ok(trip)
    }

    /// Returns the traveling household from the active destination lot back to
    /// their reconciled home lot via [`vm_travel::return_from_destination`].
    pub fn return_from_destination(&mut self) -> Result<ReturnReport, String> {
        self.note = None;
        let (nh_id, trip) = match &self.screen {
            ActiveScreen::Lot(session) => {
                let Some(trip) = session.active_trip.clone() else {
                    return Err("current lot is not an active travel trip".into());
                };
                (session.neighborhood_id.clone(), trip)
            }
            _ => return Err("no active travel destination lot is open".into()),
        };
        // Pre-validate home snapshot availability BEFORE committing return.
        let _ = self
            .store
            .load_lot_snapshot(&nh_id, trip.home_house)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {nh_id:?} not found before return"))?;

        self.start_load_audio();
        self.present_loading_step(
            LoadingKind::Lot,
            Some(format!("Home Lot {}", trip.home_house)),
            0.2,
        );
        let ret = {
            let ActiveScreen::Lot(session) = &mut self.screen else {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err("no active travel destination lot is open".into());
            };
            match vm_travel::return_from_destination(
                &self.install_root,
                &self.store,
                &trip,
                session.game.lot_mut(),
                &self.answers,
                self.frames.clone(),
            ) {
                Ok(ret) => ret,
                Err(err) => {
                    let fallback = self.desired_music_mode();
                    self.stop_load_audio(fallback);
                    return Err(err.to_string());
                }
            }
        };
        let home_house = ret.report.home_house;
        self.present_loading_step(
            LoadingKind::Lot,
            Some(format!("Home Lot {home_house}")),
            0.75,
        );
        let install_assets = self.ensure_install_assets().ok();
        let post_commit_res: Result<(Game, StoredNeighborhood), String> = (|| {
            let snapshot = self
                .store
                .load_lot_snapshot(&nh_id, home_house)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("neighborhood {nh_id:?} not found after return"))?;
            let members = snapshot
                .neighborhood
                .family_in(home_house)
                .map(|(_, f)| f.members.clone())
                .unwrap_or_default();
            let game = Game::from_loaded_lot_cached(
                &self.install_root,
                ret.home_lot,
                members,
                &snapshot.character_files,
                snapshot.base_house.as_deref(),
                self.play,
                install_assets.as_ref(),
            )?;
            Ok((game, snapshot.neighborhood))
        })();
        let (mut game, next_stored) = match post_commit_res {
            Ok(ok) => ok,
            Err(err) => {
                // Return already reconciled the home lot and consumed `TRVL` in
                // REDB: never leave the consumed destination lot on screen with
                // a stale `active_trip`. Transition cleanly to Neighborhood so
                // the player can immediately enter their saved home lot.
                self.screen = ActiveScreen::Neighborhood;
                self.lot_generation = self.lot_generation.wrapping_add(1);
                self.close_photo_album();
                self.close_travel_dialog();
                self.save_manager.close();
                self.save_manager_snapshot = None;
                self.save_manager_target_lot = Some(home_house);
                let _ = self.refresh_from_store();
                self.note = Some(format!(
                    "Returned home to lot {home_house} (saved in REDB), but failed to reopen home lot: {err}. Click lot {home_house} to enter."
                ));
                self.audio_queue
                    .push(Input::Events(vec![SoundEvent::StopAll]));
                self.stop_load_audio(music_mode_for_kind(self.kind));
                return Err(err);
            }
        };
        if self.options_file.is_file() {
            let _ = game.load_options(&self.options_file);
        }
        game.set_album_button_enabled(true);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());
        self.stored = next_stored;
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: nh_id,
            neighborhood_instance_id: self.stored.instance_id,
            neighborhood_revision: self.stored.revision,
            staging: None,
            house: home_house,
            last_days_played: initial_days_played,
            active_trip: None,
            is_destination_edit: false,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = Some(home_house);
        self.note = None;
        self.audio_queue
            .push(Input::Events(vec![SoundEvent::StopAll]));
        self.stop_load_audio(next_music);
        Ok(ret.report)
    }

    /// Inspects whether `home_house` in the active neighborhood has an active
    /// crash-persisted travel trip envelope (`TRVL`) in REDB.
    pub fn inspect_active_trip(&self, home_house: u32) -> Result<Option<TravelSession>, String> {
        vm_travel::inspect_active_trip(&self.store, &self.neighborhood_id, home_house)
            .map_err(|e| e.to_string())
    }

    /// Scans occupied residential lots in the active neighborhood for any
    /// crash-interrupted travel trips persisted in REDB.
    pub fn inspect_any_active_trips(&self) -> Vec<(u32, TravelSession)> {
        let mut out = Vec::new();
        for fam in self.stored.families.values() {
            if fam.house > 0
                && let Ok(house) = u32::try_from(fam.house)
                && let Ok(Some(session)) =
                    vm_travel::inspect_active_trip(&self.store, &self.neighborhood_id, house)
            {
                out.push((house, session));
            }
        }
        out
    }

    /// Resumes an interrupted active travel trip for `home_house` directly into
    /// its destination lot via [`vm_travel::resume_active_trip`].
    pub fn resume_active_trip(&mut self, home_house: u32) -> Result<TravelSession, String> {
        self.start_load_audio();
        self.present_loading_step(
            LoadingKind::Lot,
            Some(format!("Resuming Trip (Lot {home_house})")),
            0.2,
        );
        let resumed = match vm_travel::resume_active_trip(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            home_house,
            &self.answers,
            self.frames.clone(),
        ) {
            Ok(dep) => dep,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err.to_string());
            }
        };
        let trip = resumed.session;
        let dest_house = trip.destination_house;
        let snapshot = match self
            .store
            .load_lot_snapshot(&self.neighborhood_id, dest_house)
            .map_err(|e| e.to_string())
            .and_then(|opt| {
                opt.ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))
            }) {
            Ok(s) => s,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        let members = snapshot
            .neighborhood
            .families
            .get(&trip.home_family)
            .map(|f| f.members.clone())
            .unwrap_or_default();
        let install_assets = self.ensure_install_assets().ok();
        let mut game = match Game::from_loaded_lot_cached(
            &self.install_root,
            resumed.destination_lot,
            members,
            &snapshot.character_files,
            snapshot.base_house.as_deref(),
            self.play,
            install_assets.as_ref(),
        ) {
            Ok(game) => game,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        if self.options_file.is_file() {
            let _ = game.load_options(&self.options_file);
        }
        game.coop_set_player_name(self.coop_player_name.clone());
        game.coop_set_iroh_config(self.coop_iroh_config);
        game.coop_set_travel_blocked(true);
        game.set_album_button_enabled(true);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());
        self.stored = snapshot.neighborhood;
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: self.neighborhood_id.clone(),
            neighborhood_instance_id: self.stored.instance_id,
            neighborhood_revision: self.stored.revision,
            staging: None,
            house: dest_house,
            last_days_played: initial_days_played,
            active_trip: Some(trip.clone()),
            is_destination_edit: false,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.dialog = None;
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = None;
        self.note = None;
        self.stop_load_audio(next_music);
        Ok(trip)
    }

    /// Aborts a crash-interrupted travel trip for `home_house` via
    /// [`vm_travel::abort_active_trip`], restoring the frozen home lot bytes in
    /// REDB and refreshing the neighborhood view.
    pub fn abort_active_trip(&mut self, home_house: u32) -> Result<u32, String> {
        let restored = vm_travel::abort_active_trip(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            home_house,
            &self.answers,
            self.frames.clone(),
        )
        .map_err(|e| e.to_string())?;
        self.refresh_from_store()?;
        Ok(restored.report.home_house)
    }

    // -------------------------------------------------------------------------
    // Co-op Multiplayer Lobby & Session Helpers
    // -------------------------------------------------------------------------

    /// Whether the co-op lobby modal is currently open (either on the active
    /// lot or over the Neighborhood shell).
    pub fn coop_lobby_open(&self) -> bool {
        if self.coop_lobby.is_open() {
            return true;
        }
        matches!(&self.screen, ActiveScreen::Lot(session) if session.game.coop_lobby_open())
    }

    /// Sets the local player display name used when hosting or joining co-op.
    pub fn coop_set_player_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            self.coop_player_name = trimmed.to_owned();
            if let ActiveScreen::Lot(session) = &mut self.screen {
                session
                    .game
                    .coop_set_player_name(self.coop_player_name.clone());
            }
        }
    }

    /// Configures the `iroh` binding mode (`P2pDefault` or `Loopback`) for
    /// future co-op sessions.
    pub fn coop_set_iroh_config(&mut self, config: sims_net::IrohConfig) {
        self.coop_iroh_config = config;
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.coop_set_iroh_config(config);
        }
    }

    /// Opens the co-op lobby dialog on the active lot or over the Neighborhood
    /// screen.
    pub fn open_coop_lobby(&mut self, tab: Option<LobbyTab>) {
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.close_camera_strip();
            session.game.open_coop_lobby(tab);
            return;
        }
        self.clear_hover_state();
        self.coop_error = None;
        self.coop_lobby.open(tab.unwrap_or(LobbyTab::Join));
    }

    /// Toggles the co-op lobby dialog open or closed (`F6` / `Ctrl+M`).
    pub fn toggle_coop_lobby(&mut self) -> bool {
        if let ActiveScreen::Lot(session) = &mut self.screen {
            return session.game.toggle_coop_lobby();
        }
        if self.coop_lobby.is_open() {
            self.coop_lobby.close();
        } else {
            self.coop_error = None;
            self.coop_lobby.open(LobbyTab::Join);
        }
        true
    }

    /// Pastes a ticket into the open co-op lobby dialog (`Ctrl/Cmd+V`), using
    /// the OS clipboard when `text` is `None`.
    pub fn coop_lobby_paste(&mut self, text: Option<&str>) -> bool {
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            return session.game.coop_lobby_paste(text);
        }
        if !self.coop_lobby.is_open() {
            return false;
        }
        let pasted = match text {
            Some(s) => s.to_owned(),
            None => crate::coop::clipboard_paste().unwrap_or_default(),
        };
        if pasted.is_empty() {
            return false;
        }
        self.coop_lobby.paste_text(&pasted)
    }

    /// Cleanly leaves any active co-op session and shuts down its background
    /// transport runtime. Safe to call when offline.
    pub fn leave_coop_if_active(&mut self) {
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_active()
        {
            session.game.leave_coop();
        }
    }

    /// Picks a default residential home lot in the current neighborhood for
    /// CLI/shell co-op host or join bootstrapping (prefers lot 5 / Goth house,
    /// then first occupied home lot `1..=10`, then lot 5).
    fn default_coop_home_lot(&self) -> u32 {
        if self.stored.family_in(5).is_some() {
            return 5;
        }
        for house in 1..=10 {
            if self.stored.family_in(house).is_some() {
                return house;
            }
        }
        5
    }

    /// Hosts a co-op session on the currently open home lot, or enters the
    /// default residential lot first when called from Neighborhood view.
    pub fn coop_host_current_or_default(&mut self) -> Result<(), String> {
        if !matches!(self.screen, ActiveScreen::Lot(_)) {
            let house = self.default_coop_home_lot();
            self.enter_lot(house)?;
        }
        let ActiveScreen::Lot(session) = &mut self.screen else {
            return Err("could not open a home lot to host co-op".into());
        };
        session.game.open_coop_lobby(Some(LobbyTab::Host));
        session.game.coop_start_host_async(None)
    }

    /// Validates `ticket`, enters a local host-compatible lot shell if on the
    /// Neighborhood screen, and starts joining the remote co-op host via
    /// non-blocking `IrohTransport` + `"SCOP"` v2 snapshot restore.
    pub fn coop_join_from_shell(&mut self, ticket: String) -> Result<(), String> {
        let trimmed = ticket.trim().to_owned();
        if trimmed.is_empty() {
            return Err("Paste a simscoop1... invite ticket first.".into());
        }
        sims_net::InviteTicket::decode_string(&trimmed)
            .map_err(|e| format!("Invalid invite ticket: {e}"))?;
        if !matches!(self.screen, ActiveScreen::Lot(_)) {
            let house = self.default_coop_home_lot();
            self.enter_lot(house)?;
        }
        let ActiveScreen::Lot(session) = &mut self.screen else {
            return Err("could not open lot shell to join co-op".into());
        };
        session.game.open_coop_lobby(Some(LobbyTab::Join));
        session.game.coop_start_join_async(trimmed, None)
    }

    /// Checks if the active lot's co-op session parked a verified `"SCOP"` v2
    /// snapshot for a different `(host_house, host_family)` than the currently
    /// loaded guest lot, re-stages the matching local lot seamlessly, and
    /// completes verified restore + host ACK. Returns `true` if handled.
    pub fn pump_coop_restage(&mut self) -> bool {
        let Some((host_house, host_family)) = (match &self.screen {
            ActiveScreen::Lot(session) => session.game.pending_coop_restage(),
            _ => None,
        }) else {
            return false;
        };
        let (mut coop, lobby_open) = {
            let ActiveScreen::Lot(session) = &mut self.screen else {
                return false;
            };
            let was_open = session.game.coop_lobby_open();
            (session.game.take_coop_controller(), was_open)
        };

        // Try ordered candidate sources (REDB store first, then installed
        // `HouseNN.iff`) so an `OtherLoad` mismatch on the store copy falls
        // back seamlessly to the matching installed base house before failing.
        let mut candidates: Vec<Game> = Vec::new();
        let store_matches = self
            .stored
            .family_in(host_house)
            .is_some_and(|(fid, _)| u32::from(fid) == host_family);
        let install_assets = self.ensure_install_assets().ok();
        if store_matches
            && let Ok(game) = Game::open_from_store_cached(
                &self.install_root,
                &self.store,
                &self.neighborhood_id,
                host_house,
                &self.answers,
                self.play,
                self.frames.clone(),
                install_assets.as_ref(),
            )
        {
            candidates.push(game);
        }
        let base_house = self
            .install_root
            .join("UserData")
            .join("Houses")
            .join(format!("House{host_house:02}.iff"));
        if base_house.is_file()
            && let Ok(game) = Game::open(&crate::game::Options {
                install: self.install_root.clone(),
                house: base_house,
                answers: self.answers.clone(),
                play: self.play,
                frames: self.frames.clone(),
            })
        {
            candidates.push(game);
        }

        let mut last_err = format!(
            "Host is playing Lot {host_house} (family {host_family}), which is not available \
             in this neighborhood or install."
        );
        for mut new_game in candidates {
            if self.options_file.is_file() {
                let _ = new_game.load_options(&self.options_file);
            }
            new_game.coop_set_player_name(self.coop_player_name.clone());
            new_game.coop_set_iroh_config(self.coop_iroh_config);
            new_game.set_album_button_enabled(true);
            new_game.set_special_lot_catalog(self.build_special_lot_catalog());
            new_game.set_debug(self.debug);
            new_game.resize(self.size);
            new_game.install_coop_controller(coop, true);
            if lobby_open {
                new_game.open_coop_lobby(Some(LobbyTab::Join));
            }
            match new_game.try_complete_pending_coop_restage() {
                Ok(()) => {
                    let initial_days_played =
                        new_game.lot().world().globals.get(28).copied().unwrap_or(0);
                    if let ActiveScreen::Lot(session) = &mut self.screen {
                        session.game = new_game;
                        session.house = host_house;
                        session.last_days_played = initial_days_played;
                    }
                    self.lot_generation = self.lot_generation.wrapping_add(1);
                    return true;
                }
                Err(crate::coop::RestageAttemptError::Mismatch(err)) => {
                    // Snapshot remains parked; take controller back and try next candidate.
                    coop = new_game.take_coop_controller();
                    last_err = err;
                }
                Err(crate::coop::RestageAttemptError::Fatal(err)) => {
                    let failed_coop = new_game.take_coop_controller();
                    if let ActiveScreen::Lot(session) = &mut self.screen {
                        session.game.install_coop_controller(failed_coop, false);
                        session.game.set_note(err);
                        if lobby_open {
                            session.game.open_coop_lobby(Some(LobbyTab::Join));
                        }
                    }
                    return true;
                }
            }
        }

        if let ActiveScreen::Lot(session) = &mut self.screen {
            coop.fail_pending_restage(session.game.lot_mut(), last_err.clone());
            session.game.install_coop_controller(coop, false);
            session.game.set_note(last_err);
            if lobby_open {
                session.game.open_coop_lobby(Some(LobbyTab::Join));
            }
        }
        true
    }

    fn shell_coop_snapshot(&self) -> CoopSnapshot {
        CoopSnapshot {
            role: sims_ui::coop::CoopRole::Offline,
            connection: match &self.coop_error {
                Some(err) => sims_ui::coop::ConnectionState::Error(err.clone()),
                None => sims_ui::coop::ConnectionState::Disconnected,
            },
            players: Vec::new(),
            host_only_time_control: false,
            pause_reasons: Vec::new(),
            time_toast: None,
            resync: None,
        }
    }

    fn handle_shell_coop_command(&mut self, cmd: CoopCommand) -> ShellOutcome {
        match cmd {
            CoopCommand::CloseDialog => {
                self.coop_lobby.close();
                self.coop_error = None;
                ShellOutcome::redraw()
            }
            CoopCommand::StartHosting => {
                if let Err(err) = self.coop_host_current_or_default() {
                    self.coop_error = Some(err);
                } else {
                    self.coop_lobby.close();
                    self.coop_error = None;
                }
                ShellOutcome::redraw()
            }
            CoopCommand::JoinSession { ticket } => {
                if let Err(err) = self.coop_join_from_shell(ticket) {
                    self.coop_error = Some(err);
                } else {
                    self.coop_lobby.close();
                    self.coop_error = None;
                }
                ShellOutcome::redraw()
            }
            CoopCommand::CopyInviteTicket(ticket) => {
                let _ = crate::coop::clipboard_copy(&ticket);
                self.note = Some("Copied co-op invite ticket to clipboard.".into());
                ShellOutcome::redraw()
            }
            CoopCommand::Disconnect | CoopCommand::CancelConnect => {
                self.coop_error = None;
                ShellOutcome::redraw()
            }
            CoopCommand::SetHostOnlyTimeControl(_) => ShellOutcome::redraw(),
        }
    }
}

fn loading_kind_for_house(house: u32) -> LoadingKind {
    loading_kind_for_travel(house, None)
}

fn loading_kind_for_travel(
    house: u32,
    transport: Option<vm_travel::TravelTransport>,
) -> LoadingKind {
    match house {
        21..=30 => LoadingKind::Downtown,
        40..=48 => LoadingKind::Vacation { alternate: false },
        81..=89 => LoadingKind::StudioTown { fan_bus: false },
        90..=98 => LoadingKind::MagicTown {
            via_hole: matches!(transport, Some(vm_travel::TravelTransport::MagicHole)),
        },
        _ => LoadingKind::Lot,
    }
}

fn album_hydration_window(total: usize, focus_idx: usize) -> (usize, usize) {
    if total <= ALBUM_IMAGE_WINDOW {
        return (0, total);
    }
    let half = ALBUM_IMAGE_WINDOW / 2;
    let start = focus_idx
        .saturating_sub(half)
        .min(total - ALBUM_IMAGE_WINDOW);
    (start, start + ALBUM_IMAGE_WINDOW)
}

fn downscale_album_thumbnail(src: &Image, max_w: u32, max_h: u32) -> Image {
    if src.width == 0 || src.height == 0 || (src.width <= max_w && src.height <= max_h) {
        return src.clone();
    }
    let scale = (max_w as f32 / src.width as f32).min(max_h as f32 / src.height as f32);
    let dst_w = ((src.width as f32 * scale).round() as u32).clamp(1, max_w);
    let dst_h = ((src.height as f32 * scale).round() as u32).clamp(1, max_h);
    let mut rgba = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    for dy in 0..dst_h {
        let sy = ((u64::from(dy) * u64::from(src.height)) / u64::from(dst_h))
            .min(u64::from(src.height.saturating_sub(1))) as usize;
        for dx in 0..dst_w {
            let sx = ((u64::from(dx) * u64::from(src.width)) / u64::from(dst_w))
                .min(u64::from(src.width.saturating_sub(1))) as usize;
            let s = (sy * src.width as usize + sx) * 4;
            let d = (dy as usize * dst_w as usize + dx as usize) * 4;
            rgba[d..d + 4].copy_from_slice(&src.rgba[s..s + 4]);
        }
    }
    Image {
        width: dst_w,
        height: dst_h,
        rgba,
    }
}

fn vacation_lot_matches_lodging(lot: &MapLot, lodging: sims_ui::travel::VacationLodging) -> bool {
    let has_lodging_bit = (lot.filter_mask & (CommunityFilter::Lodging as u16)) != 0;
    let by_zone = match lodging {
        sims_ui::travel::VacationLodging::Hotel => matches!(lot.id, 40 | 42 | 46),
        sims_ui::travel::VacationLodging::CabinRental => matches!(lot.id, 44 | 48),
        sims_ui::travel::VacationLodging::Campground => matches!(lot.id, 41 | 43 | 45 | 47),
    };
    by_zone && (has_lodging_bit || (40..=48).contains(&lot.id))
}

fn load_persisted_custom_thumbs(
    store: &NeighborhoodStore,
    neighborhood_id: &str,
) -> BTreeMap<(String, u16), CustomThumb> {
    let mut out = BTreeMap::new();
    if let Ok(stored) = store.load_lot_thumbnails(neighborhood_id) {
        for (house, thumb) in stored {
            if let Ok(lot_id) = u16::try_from(house) {
                let normal = from_stored_thumb_surface(&thumb.normal);
                if normal.is_valid() {
                    out.insert(
                        (neighborhood_id.to_owned(), lot_id),
                        CustomThumb {
                            normal,
                            roofs_off: thumb.roofs_off.as_ref().map(from_stored_thumb_surface),
                        },
                    );
                }
            }
        }
    }
    out
}

fn to_stored_thumb_surface(s: &Surface565) -> ThumbnailSurface565 {
    ThumbnailSurface565 {
        width: s.width as u32,
        height: s.height as u32,
        pixels: s.pixels.clone(),
    }
}

fn from_stored_thumb_surface(s: &ThumbnailSurface565) -> Surface565 {
    Surface565 {
        width: s.width as usize,
        height: s.height as usize,
        pixels: s.pixels.clone(),
    }
}

fn destination_for_kind(kind: NeighborhoodKind) -> Option<Destination> {
    match kind {
        NeighborhoodKind::Downtown => Some(Destination::Downtown),
        NeighborhoodKind::Vacation => Some(Destination::VacationIsland),
        NeighborhoodKind::StudioTown => Some(Destination::StudioTown),
        NeighborhoodKind::MagicTown => Some(Destination::MagicTown),
        NeighborhoodKind::Unleashed | NeighborhoodKind::Classic => None,
    }
}

fn kind_for_destination(dest: Destination) -> NeighborhoodKind {
    match dest {
        Destination::Downtown => NeighborhoodKind::Downtown,
        Destination::VacationIsland => NeighborhoodKind::Vacation,
        Destination::StudioTown => NeighborhoodKind::StudioTown,
        Destination::MagicTown => NeighborhoodKind::MagicTown,
    }
}

fn map_kind_for_house(install: &Install, slot: u8, house: u32) -> NeighborhoodKind {
    match house {
        21..=30 => NeighborhoodKind::Downtown,
        40..=48 => NeighborhoodKind::Vacation,
        81..=89 => NeighborhoodKind::StudioTown,
        90..=98 => NeighborhoodKind::MagicTown,
        _ => default_map_kind(install, slot),
    }
}

fn ui_destination_for_vm(dest: TravelDestination) -> Destination {
    match dest {
        TravelDestination::Downtown => Destination::Downtown,
        TravelDestination::VacationIsland => Destination::VacationIsland,
        TravelDestination::StudioTown => Destination::StudioTown,
        TravelDestination::MagicTown => Destination::MagicTown,
        TravelDestination::OldTown => Destination::Downtown,
    }
}

fn ui_transport_for_vm(transport: VmTravelTransport) -> UiTravelTransport {
    match transport {
        VmTravelTransport::DowntownCab => UiTravelTransport::Taxi,
        VmTravelTransport::VacationWagon
        | VmTravelTransport::OldTownTrolley
        | VmTravelTransport::StudioShuttle
        | VmTravelTransport::StudioLimo => UiTravelTransport::Shuttle,
        VmTravelTransport::MagicHole | VmTravelTransport::MagicBlimp => {
            UiTravelTransport::MagicHole
        }
    }
}

fn community_filter_for_lot_filter(filter: LotFilter) -> CommunityFilter {
    match filter {
        LotFilter::Lodging => CommunityFilter::Lodging,
        LotFilter::Food => CommunityFilter::Food,
        LotFilter::Gardening => CommunityFilter::Gardening,
        LotFilter::Shopping => CommunityFilter::Shopping,
        LotFilter::DogCat => CommunityFilter::Pets,
        LotFilter::SmallAnimal => CommunityFilter::SmallAnimals,
        LotFilter::Recreation => CommunityFilter::Park,
        LotFilter::Spa => CommunityFilter::Spa,
        LotFilter::Movie => CommunityFilter::Movie,
        LotFilter::Model => CommunityFilter::Model,
        LotFilter::Music => CommunityFilter::Music,
        LotFilter::Magic => CommunityFilter::Magic,
        LotFilter::Rides => CommunityFilter::Rides,
    }
}

/// Converts a live lot [`Canvas`] into a bounded [`AlbumImage`] (clamped to
/// `<= 640×480` so snapshots stay crisp and compact inside REDB).
fn capture_canvas_to_album_image(canvas: &Canvas) -> Result<AlbumImage, String> {
    if canvas.width == 0 || canvas.height == 0 {
        return Err("lot framebuffer is empty".into());
    }
    let max_w = 640usize;
    let max_h = 480usize;
    let scale = (max_w as f32 / canvas.width as f32)
        .min(max_h as f32 / canvas.height as f32)
        .min(1.0);
    let tw = ((canvas.width as f32 * scale).round() as usize).clamp(1, max_w);
    let th = ((canvas.height as f32 * scale).round() as usize).clamp(1, max_h);
    let mut rgba = Vec::with_capacity(tw * th * 4);
    for y in 0..th {
        let sy = (y * canvas.height / th).min(canvas.height - 1);
        for x in 0..tw {
            let sx = (x * canvas.width / tw).min(canvas.width - 1);
            let [r, g, b] = sims_render::unpack_rgb(canvas.color[sy * canvas.width + sx]);
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    AlbumImage::from_rgba(tw as u32, th as u32, &rgba).map_err(|e| e.to_string())
}

/// Saves a lot booted from an ephemeral reconciled staging store through the
/// full `sims_vm::lifecycle::save_and_leave_lot` pipeline (flushing live person
/// fields via `people::save_to_record`, recomputing lot values/family flags,
/// and stamping `saved.deletion_epoch = stored.deletion_epoch`), then commits
/// the exact produced `(lot_bytes, stored)` atomically to the real store under
/// strict `(expected_instance_id, expected_revision)` fencing.
#[allow(clippy::too_many_arguments)]
fn save_staged_lot_to_store(
    install_root: &Path,
    real_store: &NeighborhoodStore,
    staging_store: &NeighborhoodStore,
    neighborhood_id: &str,
    expected_instance_id: u128,
    expected_revision: u64,
    house: u32,
    game: &mut Game,
) -> Result<(SaveReport, Vec<u8>), String> {
    let real_nh = real_store
        .load_neighborhood(neighborhood_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("neighborhood {neighborhood_id:?} not found"))?;
    if real_nh.instance_id != expected_instance_id {
        return Err(format!(
            "neighborhood {neighborhood_id:?} was replaced (expected instance {expected_instance_id:032x}, found {:032x})",
            real_nh.instance_id
        ));
    }
    if real_nh.revision != expected_revision {
        return Err(format!(
            "stale neighborhood {neighborhood_id:?} revision (expected {expected_revision}, found {})",
            real_nh.revision
        ));
    }
    let mut report = game.save_to_neighborhood(install_root, staging_store, neighborhood_id)?;
    let lot_bytes = staging_store
        .load_saved_lot(neighborhood_id, house)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "staged saved lot missing after save".to_owned())?;
    let staged_nh = staging_store
        .load_neighborhood(neighborhood_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "staged neighborhood missing after save".to_owned())?;
    let committed = real_store
        .save_lot_and_neighborhood_checked(
            neighborhood_id,
            house,
            Some((expected_instance_id, expected_revision)),
            &lot_bytes,
            &staged_nh,
            &[],
        )
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("checked neighborhood commit failed: {e:?}"))?;
    report.revision = committed.1;
    Ok((report, lot_bytes))
}

/// Flushes every live person's persistent `data` words (`0x423827`, revision 9)
/// into its `NBRS` slot on `world`, mirroring `sims_vm::people::save_to_record`
/// (`0x425a69`).
fn flush_live_people_to_neighbors(world: &mut sims_sim::World) -> Result<(), String> {
    const NEIGHBOR_WORD: usize = 31;
    const SAVED_REVISION: i32 = 9;
    // Exact `0x423827` table through revision 9 (`sims_vm::people::persistent_fields(9)`).
    const PERSISTENT_FIELDS_REV9: &[usize] = &[
        2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 16, 20, 15, 26, 17, 18, 46, 47, 48, 49, 50, 51, 52, 53,
        13, 14, 58, 60, 61, 56, 57, 63, 65, 67, 68, 69, 74, 54, 55, 75, 76, 80, 81, 82,
    ];
    for person_id in world.people.clone() {
        let Some(person) = world
            .objects
            .get(&person_id)
            .and_then(|o| o.person.as_ref())
        else {
            continue;
        };
        let mut values = Vec::with_capacity(PERSISTENT_FIELDS_REV9.len());
        for &word in PERSISTENT_FIELDS_REV9 {
            values.push(
                *person
                    .data
                    .get(word)
                    .ok_or_else(|| format!("person {person_id} missing data word {word}"))?,
            );
        }
        let neighbor_id = person.data.get(NEIGHBOR_WORD).copied().unwrap_or(0);
        let Some(record) = world.neighbors.get_mut(neighbor_id) else {
            continue;
        };
        record.revision = SAVED_REVISION;
        for (&word, value) in PERSISTENT_FIELDS_REV9.iter().zip(values) {
            if let Some(slot) = record.data.get_mut(word) {
                *slot = value;
            }
        }
    }
    Ok(())
}

/// Persists a lot whose `neighborhood_origin` was cleared by a world-mutating
/// cheat (`Lot::restore`) by flushing live person fields and running the full
/// `sims_vm::lifecycle::save_and_leave_lot` pipeline inside an ephemeral
/// staging store (so `people::save_to_record`, `update_lot_values`,
/// `family_flags`, and `saved.deletion_epoch` match normal saves exactly), then
/// committing atomically to `store` under strict `(expected_instance_id,
/// expected_revision)` fencing.
#[allow(clippy::too_many_arguments)]
fn save_cheat_mutated_lot_to_store(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    expected_instance_id: u128,
    expected_revision: u64,
    house: u32,
    arch_multiplier: f32,
    answers: &[u8],
    frames: Frames,
    game: &mut Game,
) -> Result<(SaveReport, Vec<u8>), String> {
    let snapshot = store
        .load_lot_snapshot(neighborhood_id, house)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("neighborhood {neighborhood_id:?} not found"))?;
    if snapshot.neighborhood.instance_id != expected_instance_id {
        return Err(format!(
            "neighborhood {neighborhood_id:?} was replaced (expected instance {expected_instance_id:032x}, found {:032x})",
            snapshot.neighborhood.instance_id
        ));
    }
    if snapshot.neighborhood.revision != expected_revision {
        return Err(format!(
            "stale neighborhood {neighborhood_id:?} revision (expected {expected_revision}, found {})",
            snapshot.neighborhood.revision
        ));
    }
    let raw_saved = game.save()?;
    let mut flushed_world = raw_saved.world().clone();
    flush_live_people_to_neighbors(&mut flushed_world)?;
    let flushed_saved_bytes = splice_world_into_saved_bytes(&raw_saved, &flushed_world)?;

    let mut staging_nh = snapshot.neighborhood.clone();
    let fam_id = staging_nh.family_in(house).map_or(0, |(id, _)| id);
    let b = &flushed_world.budget;
    let total_val = lot_total_value(
        b.lot_value,
        b.objects_value,
        b.architecture_value,
        arch_multiplier,
    );
    staging_nh.write_back_world(&flushed_world, house, fam_id, total_val, None, 50);

    let staging_dir = TempStagingDir::new("cheat-save")?;
    let staging_store = NeighborhoodStore::open(&staging_dir.path().join("stage.redb"))
        .map_err(|e| e.to_string())?;
    for (file_name, bytes) in &snapshot.character_files {
        staging_store
            .save_character_file(neighborhood_id, file_name, bytes)
            .map_err(|e| e.to_string())?;
    }
    if let Some(base_house) = &snapshot.base_house {
        staging_store
            .save_base_house(neighborhood_id, house, base_house)
            .map_err(|e| e.to_string())?;
    }
    staging_store
        .save_lot_and_neighborhood(neighborhood_id, house, &flushed_saved_bytes, &staging_nh)
        .map_err(|e| e.to_string())?;

    let mut staged_lot = lifecycle::enter_lot(
        install_root,
        &staging_store,
        neighborhood_id,
        house,
        answers,
        frames,
    )
    .map_err(|e| format!("staging cheat save boot failed: {e}"))?;
    let mut report = lifecycle::save_and_leave_lot(
        install_root,
        &staging_store,
        neighborhood_id,
        &mut staged_lot,
    )
    .map_err(|e| format!("staging cheat save writeback failed: {e}"))?;
    let lot_bytes = staging_store
        .load_saved_lot(neighborhood_id, house)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "staged cheat save bytes missing".to_owned())?;
    let final_nh = staging_store
        .load_neighborhood(neighborhood_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "staged cheat save neighborhood missing".to_owned())?;

    let committed = store
        .save_lot_and_neighborhood_checked(
            neighborhood_id,
            house,
            Some((expected_instance_id, expected_revision)),
            &lot_bytes,
            &final_nh,
            &[],
        )
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("checked neighborhood commit failed: {e:?}"))?;
    let ticks = game.lot().ticks();
    game.mark_saved_clean(format!("Saved after tick {ticks}."));
    report.revision = committed.1;
    Ok((report, lot_bytes))
}

struct TempStagingDir(PathBuf);

impl TempStagingDir {
    fn new(tag: &str) -> Result<Self, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sims-game-stage-{tag}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("create staging dir: {e}"))?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempStagingDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn splice_world_into_saved_bytes(
    base_saved: &Saved,
    world: &sims_sim::World,
) -> Result<Vec<u8>, String> {
    let orig_world_body = base_saved.world().encode();
    let encoded = base_saved.encode();
    let saved_body = encoded
        .get(8..)
        .ok_or_else(|| "invalid base Saved header".to_owned())?;
    let tail = saved_body
        .strip_prefix(orig_world_body.as_slice())
        .ok_or_else(|| "base Saved world prefix mismatch".to_owned())?;
    let new_world_body = world.encode();
    let mut rebuilt = Vec::with_capacity(8 + new_world_body.len() + tail.len());
    rebuilt.extend_from_slice(&encoded[..8]);
    rebuilt.extend_from_slice(&new_world_body);
    rebuilt.extend_from_slice(tail);
    let _ = Saved::decode(&rebuilt).map_err(|e| e.to_string())?;
    Ok(rebuilt)
}

fn caf_names_sidecar_path(saves_path: &Path) -> PathBuf {
    saves_path.with_extension("caf-names.tsv")
}

fn load_family_names_with_caf(
    user_dir: &Path,
    saves_path: &Path,
    neighborhood_id: &str,
    instance_id: u128,
    stored: &StoredNeighborhood,
) -> BTreeMap<u16, String> {
    let mut names = load_family_names(user_dir);
    let sidecar = caf_names_sidecar_path(saves_path);
    let Ok(text) = std::fs::read_to_string(&sidecar) else {
        return names;
    };
    let inst_hex = format!("{instance_id:032x}");
    for line in text.lines() {
        let mut parts = line.splitn(4, '\t');
        let (Some(nh), Some(inst), Some(fam_str), Some(surname)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if nh == neighborhood_id
            && inst == inst_hex
            && let Ok(fam_id) = fam_str.parse::<u16>()
            && stored.families.contains_key(&fam_id)
            && !surname.trim().is_empty()
        {
            names.insert(fam_id, surname.to_owned());
        }
    }
    names
}

fn save_persisted_caf_name(
    saves_path: &Path,
    neighborhood_id: &str,
    instance_id: u128,
    family_id: u16,
    surname: Option<&str>,
) {
    let sidecar = caf_names_sidecar_path(saves_path);
    let inst_hex = format!("{instance_id:032x}");
    let mut entries: BTreeMap<(String, String, u16), String> = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(&sidecar) {
        for line in text.lines() {
            let mut parts = line.splitn(4, '\t');
            if let (Some(nh), Some(inst), Some(fam_str), Some(name)) =
                (parts.next(), parts.next(), parts.next(), parts.next())
                && let Ok(fid) = fam_str.parse::<u16>()
                && !name.is_empty()
            {
                entries.insert((nh.to_owned(), inst.to_owned(), fid), name.to_owned());
            }
        }
    }
    let key = (neighborhood_id.to_owned(), inst_hex, family_id);
    match surname.map(|s| s.replace(['\t', '\r', '\n'], " ")) {
        Some(clean) if !clean.trim().is_empty() => {
            entries.insert(key, clean.trim().to_owned());
        }
        _ => {
            entries.remove(&key);
        }
    }
    let mut out = String::new();
    for ((nh, inst, fid), name) in entries {
        out.push_str(&nh);
        out.push('\t');
        out.push_str(&inst);
        out.push('\t');
        out.push_str(&fid.to_string());
        out.push('\t');
        out.push_str(&name);
        out.push('\n');
    }
    let _ = std::fs::write(&sidecar, out);
}

fn resolve_dir_case_insensitive(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_dir() {
        return Some(direct);
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
            && entry.path().is_dir()
        {
            return Some(entry.path());
        }
    }
    None
}

fn resolve_child_case_insensitive(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
        {
            return Some(entry.path());
        }
    }
    None
}

fn resolve_user_dir(install: &Install, slot: u8) -> Result<PathBuf, String> {
    let name = neighborhood_id_for_slot(slot);
    install
        .file(&name)
        .or_else(|_| install.file("TemplateUserData"))
        .or_else(|_| install.file("UserData"))
        .map_err(|e| e.to_string())
}

fn ensure_imported(
    install: &Install,
    store: &NeighborhoodStore,
    id: &str,
) -> Result<StoredNeighborhood, String> {
    if let Some(existing) = store.load_neighborhood(id).map_err(|e| e.to_string())? {
        return Ok(existing);
    }
    let dir = install
        .file(id)
        .or_else(|_| install.file("TemplateUserData"))
        .or_else(|_| install.file("UserData"))
        .map_err(|e| e.to_string())?;
    store
        .import_directory(id, &dir, Context::COMPLETE_COLLECTION.expansions)
        .map_err(|e| e.to_string())?;
    store
        .load_neighborhood(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("imported neighborhood {id:?} missing from store"))
}

fn default_map_kind(install: &Install, slot: u8) -> NeighborhoodKind {
    if NeighborhoodMap::load(install, slot, NeighborhoodKind::Unleashed, false).is_ok() {
        NeighborhoodKind::Unleashed
    } else {
        NeighborhoodKind::Classic
    }
}

fn load_map_for_slot(
    install: &Install,
    slot: u8,
    kind: NeighborhoodKind,
) -> Result<NeighborhoodMap, String> {
    NeighborhoodMap::load(install, slot, kind, false).or_else(|err| {
        let fallback_dir = resolve_user_dir(install, slot)?;
        NeighborhoodMap::load_from_dir(install, &fallback_dir, slot, kind, false)
            .map_err(|_| err.to_string())
    })
}

fn load_arch_multiplier(install: &Install) -> f32 {
    let Ok(index) = Index::build(install) else {
        return DEFAULT_ARCH_VALUE_MULTIPLIER;
    };
    let Some((_, res)) = index.named("Global.iff").next() else {
        return DEFAULT_ARCH_VALUE_MULTIPLIER;
    };
    let Ok(bytes) = res.read() else {
        return DEFAULT_ARCH_VALUE_MULTIPLIER;
    };
    let Ok(iff) = Iff::parse(bytes) else {
        return DEFAULT_ARCH_VALUE_MULTIPLIER;
    };
    let Some(chunk) = iff.get(b"FCNS", NEIGHBORHOOD_TUNING_CHUNK) else {
        return DEFAULT_ARCH_VALUE_MULTIPLIER;
    };
    let Ok(fcns) = sims_formats::tuning::Constants::parse(iff.data(chunk)) else {
        return DEFAULT_ARCH_VALUE_MULTIPLIER;
    };
    fcns.entries
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case("architecture value multiplier"))
        .map_or(DEFAULT_ARCH_VALUE_MULTIPLIER, |e| e.value)
}

fn load_family_names(user_dir: &Path) -> BTreeMap<u16, String> {
    let mut names = BTreeMap::new();
    let Some(path) = resolve_child_case_insensitive(user_dir, "Neighborhood.iff") else {
        return names;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return names;
    };
    let Ok(iff) = Iff::parse(bytes) else {
        return names;
    };
    for chunk in iff.all(b"FAMs") {
        if let Ok(table) = StringTable::parse(iff.data(chunk))
            && let Some(surname) = table
                .get(0, US_ENGLISH)
                .or_else(|| table.get(0, 1))
                .filter(|s| !s.is_empty())
        {
            names.insert(chunk.id, surname.to_owned());
        }
    }
    names
}

fn load_neighborhood_faces_cached(
    install: &Install,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    stored: &StoredNeighborhood,
    pool: &ArchivePool,
    base_index: Option<&Arc<Index>>,
    base_catalog: Option<&Arc<Catalog>>,
) -> (Faces, Option<Arc<Catalog>>) {
    let mut guids = Vec::new();
    for fam in stored.families.values() {
        for &guid in &fam.members {
            if !guids.contains(&guid) {
                guids.push(guid);
            }
        }
    }
    if guids.is_empty() {
        return (Faces::default(), base_catalog.cloned());
    }
    let chars = store
        .load_character_files(neighborhood_id)
        .unwrap_or_default();
    if chars.is_empty()
        && let (Some(idx), Some(cat)) = (base_index, base_catalog)
    {
        return (
            Faces::load_with_pool(idx, cat, &guids, pool).unwrap_or_default(),
            Some(Arc::clone(cat)),
        );
    }
    let mut index = match base_index {
        Some(idx) => (**idx).clone(),
        None => {
            let Ok(idx) = Index::build(install) else {
                return (Faces::default(), base_catalog.cloned());
            };
            idx
        }
    };
    if !chars.is_empty() {
        let _ = index.add_character_files(&chars);
    }
    let Ok(catalog) = Catalog::build_with_pool(&index, pool) else {
        return (Faces::default(), base_catalog.cloned());
    };
    let faces = Faces::load_with_pool(&index, &catalog, &guids, pool).unwrap_or_default();
    let new_base_cat = if chars.is_empty() {
        Some(Arc::new(catalog))
    } else {
        base_catalog.cloned()
    };
    (faces, new_base_cat)
}

fn build_lot_meta(
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    map: &NeighborhoodMap,
    descriptions: &NeighborhoodDescriptions,
    arch_multiplier: f32,
) -> BTreeMap<u16, LotMeta> {
    let mut out = BTreeMap::new();
    for lot in &map.lots {
        let house = u32::from(lot.id);
        let (default_name, default_desc) = descriptions.lot_info(lot.id);
        if let Ok(Some(saved_bytes)) = store.load_saved_lot(neighborhood_id, house)
            && let Ok(saved) = Saved::decode(&saved_bytes)
        {
            let world = saved.world();
            let b = &world.budget;
            let house_cost = lot_total_value(
                b.lot_value,
                b.objects_value,
                b.architecture_value,
                arch_multiplier,
            );
            let built_flag = world.globals.get(22).copied().unwrap_or(0) != 0;
            let has_house = built_flag || b.architecture_value > 0 || b.objects_value > 0;
            let filter_mask = world.globals.get(37).copied().map(|v| v as u16);
            let lot_name = if saved.lot_name().is_empty() {
                default_name
            } else {
                saved.lot_name().to_owned()
            };
            let lot_description = if saved.lot_description().is_empty() {
                default_desc
            } else {
                saved.lot_description().to_owned()
            };
            out.insert(
                lot.id,
                LotMeta {
                    house_cost,
                    has_house,
                    from_saved_lot: true,
                    lot_name,
                    lot_description,
                    filter_mask,
                },
            );
            continue;
        }
        let mut house_cost = 0;
        let mut has_house = false;
        if let Ok(Some(base_bytes)) = store.load_base_house(neighborhood_id, house)
            && let Ok(Some(simi_bytes)) = Iff::find_first_physical(&base_bytes, b"SIMI")
            && let Ok(sim) = sims_formats::lot::Simulator::parse(&simi_bytes)
        {
            house_cost = lot_total_value(
                sim.lot_value,
                sim.objects_value,
                sim.architecture_value,
                arch_multiplier,
            );
            let built_flag = sim.globals.get(22).copied().unwrap_or(0) != 0;
            has_house = built_flag || sim.architecture_value > 0 || sim.objects_value > 0;
        }
        out.insert(
            lot.id,
            LotMeta {
                house_cost,
                has_house,
                from_saved_lot: false,
                lot_name: default_name,
                lot_description: default_desc,
                filter_mask: None,
            },
        );
    }
    out
}

fn sync_map_with_store(
    map: &mut NeighborhoodMap,
    stored: &StoredNeighborhood,
    lot_meta: &BTreeMap<u16, LotMeta>,
    neighborhood_id: &str,
    custom_thumbs: &BTreeMap<(String, u16), CustomThumb>,
) {
    for lot in &mut map.lots {
        if let Some(&zone) = stored.zoning.get(&(lot.id as i16)) {
            lot.zoning = match zone {
                1 => LotZoning::Residential,
                2 => LotZoning::Community,
                _ => lot.zoning,
            };
        }
        let house = u32::from(lot.id);
        let occ = stored.family_in(house);
        lot.status = match (lot.zoning, occ) {
            (LotZoning::Community, Some((fam_id, _))) => LotStatus::Community {
                occupied_by_family: Some(i32::from(fam_id)),
            },
            (LotZoning::Community, None) => LotStatus::Community {
                occupied_by_family: None,
            },
            (_, Some((fam_id, fam))) => {
                let vac = u16::try_from(fam.secondary_house())
                    .ok()
                    .filter(|&v| v != 0);
                LotStatus::Occupied {
                    family_id: i32::from(fam_id),
                    vacation_lot: vac,
                }
            }
            (_, None) => LotStatus::Vacant,
        };
        lot.has_primary_vacation_stay = stored
            .families
            .values()
            .any(|f| u32::try_from(f.house).ok() == Some(house) && f.secondary_house() != 0);
        lot.has_vacation_occupancy = occ.is_some();
        if let Some(meta) = lot_meta.get(&lot.id)
            && let Some(mask) = meta.filter_mask
        {
            lot.filter_mask = mask;
        }
        if let Some(meta) = lot_meta.get(&lot.id)
            && meta.from_saved_lot
            && !meta.has_house
        {
            lot.normal = vacant_lot_thumbnail(lot.normal.width, lot.normal.height);
            lot.roofs_off = None;
        } else if let Some(thumb) = custom_thumbs.get(&(neighborhood_id.to_owned(), lot.id)) {
            apply_custom_thumb(lot, thumb);
        }
    }
}

fn apply_custom_thumb(lot: &mut MapLot, thumb: &CustomThumb) {
    if thumb.normal.is_valid() {
        lot.normal = thumb.normal.clone();
        lot.roofs_off = thumb.roofs_off.clone();
    }
}

fn build_family_bin_entries(
    stored: &StoredNeighborhood,
    family_names: &BTreeMap<u16, String>,
    faces: &Faces,
) -> Vec<FamilyEntry> {
    stored
        .family_bin()
        .into_iter()
        .map(|(fam_id, fam)| {
            let name = family_names
                .get(&fam_id)
                .cloned()
                .unwrap_or_else(|| format!("Family {fam_id}"));
            let members = fam
                .members
                .iter()
                .map(|&guid| FamilyMemberPortrait {
                    name: faces.name(guid).unwrap_or("").to_owned(),
                    portrait: faces.face(guid).cloned(),
                })
                .collect();
            let magic_coins = lifecycle::family_magic_coins(stored, fam_id);
            FamilyEntry {
                id: u32::from(fam_id),
                name,
                funds: family_net_worth(fam),
                friends: fam.friends,
                magic_coins,
                members,
            }
        })
        .collect()
}

/// Captures updated neighborhood map thumbnail cells (`normal` + optional
/// `roofs_off`) by rendering the live lot at far zoom and downsampling into the
/// lot's map sprite dimensions with `COLOR_KEY_565` sky transparency.
fn capture_lot_thumbnail(
    world: &sims_sim::World,
    lot_shape: &sims_render::Lot,
    assets: &mut sims_render::Assets,
    target_size: [usize; 2],
    has_cutaway: bool,
) -> CustomThumb {
    let tw = target_size[0].max(1);
    let th = target_size[1].max(1);
    let render_w = (tw * 2).clamp(160, 512);
    let render_h = (th * 2).clamp(120, 384);
    let mut render_cell = |walls: Walls| -> Surface565 {
        let mut camera = Camera {
            zoom: Zoom::Far,
            rotation: 0,
            level: 1,
            origin: [0, 0],
            walls,
            focus: Focus::default(),
        };
        camera.center_on(
            lot_shape.built_center(),
            [render_w as i32 / 2, render_h as i32 / 2],
        );
        let mut canvas = Canvas::new(render_w, render_h, [0, 0, 0]);
        sims_render::draw_lot(world, lot_shape, assets, &camera, &mut canvas);
        let surf = downsample_canvas_to_565(&canvas, tw, th);
        if surf.pixels.iter().any(|&p| p != COLOR_KEY_565) {
            surf
        } else {
            vacant_lot_thumbnail(tw, th)
        }
    };
    let normal = render_cell(Walls::Up);
    let roofs_off = has_cutaway.then(|| render_cell(Walls::Cutaway));
    CustomThumb { normal, roofs_off }
}

fn downsample_canvas_to_565(canvas: &Canvas, tw: usize, th: usize) -> Surface565 {
    let mut surf = Surface565::filled(tw, th, COLOR_KEY_565);
    if canvas.width == 0 || canvas.height == 0 {
        return surf;
    }
    for y in 0..th {
        let sy = (y * canvas.height / th).min(canvas.height - 1);
        for x in 0..tw {
            let sx = (x * canvas.width / tw).min(canvas.width - 1);
            let idx = sy * canvas.width + sx;
            if canvas.depth[idx] == sims_render::EMPTY_DEPTH {
                continue;
            }
            let px = pack_565(sims_render::unpack_rgb(canvas.color[idx]));
            surf.pixels[y * tw + x] = if px == COLOR_KEY_565 { px ^ 1 } else { px };
        }
    }
    surf
}

/// Generates a clean isometric grass diamond thumbnail for a bulldozed vacant lot.
fn vacant_lot_thumbnail(width: usize, height: usize) -> Surface565 {
    let w = width.max(1);
    let h = height.max(1);
    let mut surf = Surface565::filled(w, h, COLOR_KEY_565);
    let grass = pack_565([88, 136, 56]);
    let border = pack_565([64, 104, 40]);
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.55;
    let rx = (w as f32 * 0.44).max(1.0);
    let ry = (h as f32 * 0.34).max(1.0);
    for y in 0..h {
        let dy = ((y as f32 + 0.5) - cy).abs() / ry;
        for x in 0..w {
            let dx = ((x as f32 + 0.5) - cx).abs() / rx;
            let d = dx + dy;
            if d <= 1.0 {
                surf.pixels[y * w + x] = if d >= 0.92 { border } else { grass };
            }
        }
    }
    surf
}

fn format_template(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut chars = template.chars().peekable();
    let mut idx = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            continue;
        }
        if ch == '%'
            && let Some(&next) = chars.peek()
        {
            match next {
                's' | 'd' | 'i' | 'u' => {
                    chars.next();
                    if let Some(val) = args.get(idx) {
                        out.push_str(val);
                    }
                    idx += 1;
                    continue;
                }
                '%' => {
                    chars.next();
                    out.push('%');
                    continue;
                }
                _ => {}
            }
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighborhood_id_maps_slots_one_through_eight() {
        assert_eq!(neighborhood_id_for_slot(0), "UserData");
        assert_eq!(neighborhood_id_for_slot(1), "UserData");
        assert_eq!(neighborhood_id_for_slot(2), "UserData2");
        assert_eq!(neighborhood_id_for_slot(8), "UserData8");
        assert_eq!(neighborhood_id_for_slot(99), "UserData8");
    }

    #[test]
    fn street_names_formats_assigned_lot_addresses() {
        let streets = StreetNames {
            streets: vec!["%s Sim Lane".into(), "%s Ridge Road".into()],
            assignments: vec![Some(0), Some(1), None],
        };
        assert_eq!(streets.address_for(1).as_deref(), Some("1 Sim Lane"));
        assert_eq!(streets.address_for(2).as_deref(), Some("2 Ridge Road"));
        assert_eq!(streets.address_for(3), None);
        assert_eq!(streets.address_for(0), None);
    }

    #[test]
    fn vacant_lot_thumbnail_draws_non_key_grass_diamond() {
        let thumb = vacant_lot_thumbnail(64, 48);
        assert_eq!((thumb.width, thumb.height), (64, 48));
        assert!(thumb.pixels.iter().any(|&p| p != COLOR_KEY_565));
        assert_eq!(thumb.pixels[0], COLOR_KEY_565);
    }
}
