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
    validate_save_payload_cached_with_sha256,
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
    TravelDialogCommand, TravelDialogLayout, TravelDialogState, TravelLayout, TravelPress,
    TravelState, TravelStrings, TravelTransport as UiTravelTransport,
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
    /// The neighborhood record ([`StoredNeighborhood::instance_id`] and
    /// [`StoredNeighborhood::revision`]) last written as `Neighborhood.iff`
    /// ([`NeighborhoodShell::neighborhood_iff_path`]), or the one opened.
    neighborhood_iff_synced: Option<(String, u128, u64)>,
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
    /// The cursor when the card was popped: the original's tooltip places a
    /// card once, from the cursor at that moment (`0x5da0ed`, `0x5ddbda`).
    hover_card_cursor: Option<[i32; 2]>,
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
    /// Each household album's current photo. Native keeps it on the
    /// household's album object (`+0x10`), which outlives the dialog
    /// (`0x4108e4`) and is freed with the family (`0x41010b`). The key is
    /// the album token, whose neighborhood instance and family epoch change
    /// when the family is freed, so a later family with the same ID starts
    /// again at 0, the index a missing entry means.
    album_index: BTreeMap<AlbumFamilyToken, usize>,
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

mod commands;
mod coop_shell;
mod drawing;
mod keys;
mod lot_saves;
mod lots_and_families;
mod opening;
mod photo_album;
mod pointer;
mod rezone_and_import;
mod shell_state;
mod travel_shell;

/// What a departure from the home lot carries out
/// ([`NeighborhoodShell::depart_to_destination`],
/// [`NeighborhoodShell::pump_trip_request`]).
enum Departure {
    /// A trip the shell plans: it writes the travelers' tokens and charges
    /// the fare ([`vm_travel::depart_to_destination`]).
    Request(TravelRequest),
    /// The trip a script asked for with Generic Sims Call 17, to `house`,
    /// whose tokens and fare the script already took care of
    /// ([`vm_travel::depart_from_request_with`]). `transport` is the way the
    /// lot picker showed: the arrival places the travelers by it, and it
    /// picks the loading screen.
    Trip {
        house: u32,
        transport: Option<vm_travel::TravelTransport>,
    },
}

/// How the lot picker a travel script asked with goes: the Magic Hole or
/// the blimp to Magic Town, or the star limo to Studio Town.
fn picker_transport(picker: &sims_vm::SpecialPanelRequest) -> Option<vm_travel::TravelTransport> {
    match picker {
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
    }
}

/// Tells the player why a trip did not depart: a notice, or the note when
/// a dialog is already up, so it never covers the lot's own dialog.
fn report_travel_refusal(screen: &mut ActiveScreen, msg: String) {
    if let ActiveScreen::Lot(session) = screen {
        if !session.game.has_dialog() {
            session.game.show_notice("Travel", msg);
        } else {
            session.game.set_note(msg);
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
        TravelDestination::OldTown => Destination::OldTown,
        TravelDestination::StudioTown => Destination::StudioTown,
        TravelDestination::MagicTown => Destination::MagicTown,
    }
}

fn ui_transport_for_vm(transport: VmTravelTransport) -> UiTravelTransport {
    match transport {
        VmTravelTransport::DowntownCab => UiTravelTransport::Taxi,
        VmTravelTransport::OldTownTrolley => UiTravelTransport::Trolley,
        VmTravelTransport::VacationWagon
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
    vm_cache: Option<&sims_vm::lifecycle::VmInstallCache>,
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

    let mut staged_lot = match vm_cache {
        Some(cache) => lifecycle::enter_lot_with_cache(
            cache,
            &staging_store,
            neighborhood_id,
            house,
            answers,
            frames,
        ),
        None => lifecycle::enter_lot(
            install_root,
            &staging_store,
            neighborhood_id,
            house,
            answers,
            frames,
        ),
    }
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

/// One-time atomic read-only migration from any pre-v4 `<saves>.caf-names.tsv`
/// sidecar into native REDB v4 `StoredNeighborhood.family_surnames`.
///
/// Leaves the sidecar file untouched on disk, commits both the migrated
/// surnames and the `NEIGHBORHOOD_META` completion marker in a single REDB
/// transaction, and updates `stored` in memory only after that commit succeeds.
fn migrate_legacy_caf_sidecar_if_needed(
    store: &NeighborhoodStore,
    saves_path: &Path,
    neighborhood_id: &str,
    stored: &mut StoredNeighborhood,
) {
    let sidecar = caf_names_sidecar_path(saves_path);
    if !sidecar.is_file() {
        return;
    }
    let Ok(text) = std::fs::read_to_string(&sidecar) else {
        return;
    };
    let inst_hex = format!("{:032x}", stored.instance_id);
    let mut legacy_surnames = BTreeMap::new();
    for line in text.lines() {
        let mut parts = line.splitn(4, '\t');
        let (Some(nh), Some(inst), Some(fam_str), Some(surname)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let trimmed = surname.trim();
        if nh == neighborhood_id
            && inst == inst_hex
            && let Ok(fam_id) = fam_str.parse::<u16>()
            && !trimmed.is_empty()
        {
            legacy_surnames.insert(fam_id, trimmed.to_owned());
        }
    }
    if let Ok(Ok(Some(updated))) = store.migrate_legacy_caf_surnames_checked(
        neighborhood_id,
        stored.instance_id,
        stored.revision,
        &legacy_surnames,
    ) {
        *stored = updated;
    }
}

/// Loads household surnames for `stored`, overlaying native REDB v4
/// `StoredNeighborhood.family_surnames` over base `Neighborhood.iff` names.
fn load_family_names_with_caf(
    user_dir: &Path,
    stored: &StoredNeighborhood,
) -> BTreeMap<u16, String> {
    let mut names = load_family_names(user_dir);
    for (&fam_id, surname) in &stored.family_surnames {
        let trimmed = surname.trim();
        if stored.families.contains_key(&fam_id) && !trimmed.is_empty() {
            names.insert(fam_id, trimmed.to_owned());
        }
    }
    names
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
mod tests;
