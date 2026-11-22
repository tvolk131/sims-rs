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

use crate::game::Game;
use crate::hud::DialogKey;
use crate::sound::{self, Input};
use sims_audio::SoundEvent;
use sims_audio::music::MusicMode;
use sims_content::ui_graphics::UiGraphics;
use sims_content::{Catalog, Index, Install};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_render::camera::{Camera, Walls, Zoom};
use sims_render::neighborhood::{
    COLOR_KEY_565, LotStatus, LotZoning, MapLot, NeighborhoodKind, NeighborhoodMap, RenderOptions,
    Surface565, pack_565,
};
use sims_render::{Canvas, Focus};
use sims_sim::load::Context;
use sims_sim::neighborhood_store::{
    DEFAULT_ARCH_VALUE_MULTIPLIER, NeighborhoodStore, StoredNeighborhood, family_net_worth,
    lot_total_value,
};
use sims_ui::art::Art;
use sims_ui::command::Command;
use sims_ui::dialog::{self, DialogState, FrameStyle, Labels, Message};
use sims_ui::family_bin::{
    FamilyBin, FamilyBinCommand, FamilyBinMode, FamilyBinStrings, FamilyBinView, FamilyEntry,
    FamilyMemberPortrait,
};
use sims_ui::frame::{Frame, Rect};
use sims_ui::household::Faces;
use sims_ui::neighborhood::{
    LotInfo, LotKind, LotOccupant, MAX_NEIGHBORHOOD, MIN_NEIGHBORHOOD, NeighborhoodChrome,
    NeighborhoodCommand, NeighborhoodMode, NeighborhoodState, NeighborhoodStrings,
    format_simoleons,
};
use sims_ui::options::{OptionsState, OptionsTexts};
use sims_vm::boot::Frames;
use sims_vm::boot::save::Saved;
use sims_vm::lifecycle::{self, BulldozeReport, EvictReport, MoveInReport, SaveReport};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const US_ENGLISH: u8 = 1;
const NEIGHBORHOOD_DIALOG_STRINGS: u16 = 131;
const MOVE_IN_DIALOG_STRINGS: u16 = 132;
const STREET_NAMES_TABLE: u16 = 2000;
const LOT_ASSIGNMENTS_TABLE: u16 = 2001;
const DESC_NAMES_TABLE: u16 = 2002;
const DESC_TEXT_TABLE: u16 = 2003;
/// Water wave animation cadence on the neighborhood map (250 ms per step).
const WATER_STEP_US: u64 = 250_000;
/// `Global.far` `FCNS #4` tuning chunk ID.
const NEIGHBORHOOD_TUNING_CHUNK: u16 = 4;

/// High-level screen currently shown by [`NeighborhoodShell`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    Neighborhood,
    FamilyBin,
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
    ConfirmMoveIn { family_id: u16, lot_id: u16 },
    ConfirmEvict { lot_id: u16 },
    ConfirmBulldoze { lot_id: u16 },
    ConfirmDeleteFamily { family_id: u16 },
    Notice,
}

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
struct StreetNames {
    streets: Vec<String>,
    assignments: Vec<Option<usize>>,
}

impl StreetNames {
    fn load(install: &Install, user_dir: &Path) -> Self {
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
    fn address_for(&self, lot_number: u16) -> Option<String> {
        let idx = usize::from(lot_number).checked_sub(1)?;
        let street_idx = (*self.assignments.get(idx)?)?;
        let template = self.streets.get(street_idx).filter(|s| !s.is_empty())?;
        let num = lot_number.to_string();
        Some(format_template(template, &[&num]))
    }
}

/// Parsed `NeighborhoodDesc.iff` lot names and descriptions.
#[derive(Debug, Clone, Default)]
struct NeighborhoodDescriptions {
    names: Vec<String>,
    descriptions: Vec<String>,
}

impl NeighborhoodDescriptions {
    fn load(user_dir: &Path) -> Self {
        let Some(path) = resolve_child_case_insensitive(user_dir, "NeighborhoodDesc.iff") else {
            return Self::default();
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
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
        Self {
            names: read_vec(DESC_NAMES_TABLE),
            descriptions: read_vec(DESC_TEXT_TABLE),
        }
    }

    fn lot_info(&self, lot_number: u16) -> (String, String) {
        let Some(idx) = usize::from(lot_number).checked_sub(1) else {
            return (String::new(), String::new());
        };
        let name = self.names.get(idx).cloned().unwrap_or_default();
        let desc = self.descriptions.get(idx).cloned().unwrap_or_default();
        (name, desc)
    }
}

/// Localized dialog strings loaded dynamically from `GameData/UIText.iff`.
#[derive(Debug, Clone, Default)]
struct DialogStrings {
    /// `STR# 131`: Bulldoze / Evict confirmation messages.
    bulldoze: Vec<String>,
    /// `STR# 132`: Move-In confirmation / notice messages.
    move_in: Vec<String>,
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

/// Active live lot session launched from the Neighborhood map.
struct LotSession {
    game: Game,
    neighborhood_id: String,
    house: u32,
}

enum ActiveScreen {
    Neighborhood,
    FamilyBin,
    Lot(Box<LotSession>),
}

/// Full playable Neighborhood ↔ Family Bin ↔ Lot application shell.
pub struct NeighborhoodShell {
    install_root: PathBuf,
    install: Install,
    store: NeighborhoodStore,
    options_file: PathBuf,
    options_state: OptionsState,
    saved_optn: Vec<u8>,
    play: bool,
    answers: Vec<u8>,
    frames: Frames,
    arch_multiplier: f32,

    screen: ActiveScreen,
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
    dialogs: DialogStrings,

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
    pressed_lot_id: Option<u16>,
    move_in_family_id: Option<u16>,

    // Family Bin screen interactive state.
    family_bin: FamilyBin,
    family_bin_entries: Vec<FamilyEntry>,

    // Modal confirmation / notice dialog over Neighborhood or Family Bin.
    dialog: Option<ShellDialog>,
    note: Option<String>,

    // Pending sound inputs to be drained by the host.
    audio_queue: Vec<Input>,
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
        let options_file = saves_path.with_extension("options.iff");
        let options_state = if options_file.is_file() {
            OptionsState::load_iff(&options_file)
                .or_else(|_| OptionsState::load_install(&install))
                .unwrap_or_default()
        } else {
            OptionsState::load_install(&install).unwrap_or_default()
        };
        let saved_optn = options_state.to_optn();

        let ui = UiGraphics::open(&install).map_err(|e| e.to_string())?;
        let chrome_art =
            sims_ui::neighborhood::load_art(&ui, US_ENGLISH).map_err(|e| e.to_string())?;
        let chrome_strings = NeighborhoodStrings::load(&install, US_ENGLISH).unwrap_or_default();
        let family_bin_art =
            sims_ui::family_bin::load_art(&ui, US_ENGLISH).map_err(|e| e.to_string())?;
        let dialog_art =
            Art::load(&ui, US_ENGLISH, &dialog::RESOURCES).map_err(|e| e.to_string())?;
        let dialogs = DialogStrings::load(&install);
        let arch_multiplier = load_arch_multiplier(&install);

        let slot = slot.clamp(MIN_NEIGHBORHOOD, MAX_NEIGHBORHOOD);
        let kind = default_map_kind(&install, slot);
        let neighborhood_id = neighborhood_id_for_slot(slot);
        let stored = ensure_imported(&install, &store, &neighborhood_id)?;
        let _ = store.set_active_neighborhood(&neighborhood_id);
        let mut map = load_map_for_slot(&install, slot, kind)?;

        let user_dir = resolve_user_dir(&install, slot)?;
        let family_names = load_family_names(&user_dir);
        let street_names = StreetNames::load(&install, &user_dir);
        let descriptions = NeighborhoodDescriptions::load(&user_dir);
        let faces = load_neighborhood_faces(&install, &store, &neighborhood_id, &stored);
        let lot_meta = build_lot_meta(
            &store,
            &neighborhood_id,
            &map,
            &descriptions,
            arch_multiplier,
        );
        let custom_thumbs = BTreeMap::new();
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
            show_credits_button: true,
            hovered_lot: None,
        };

        let vols = [
            options_state.sound.fx,
            options_state.sound.music,
            options_state.sound.vox,
        ];
        let audio_queue = vec![
            Input::FxVolume(vols[0]),
            Input::MusicVolume(vols[1]),
            Input::VoxVolume(vols[2]),
            Input::MusicMode(music_mode_for_kind(kind)),
        ];

        Ok(Self {
            install_root,
            install,
            store,
            options_file,
            options_state,
            saved_optn,
            play,
            answers,
            frames,
            arch_multiplier,
            screen: ActiveScreen::Neighborhood,
            size: [800, 600],
            canvas: Canvas::new(800, 600, [0, 0, 0]),
            pointer: None,
            clock_us: 0,
            water_frame: 0,
            chrome_art,
            chrome_strings,
            family_bin_art,
            dialog_art,
            dialogs,
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
            pressed_lot_id: None,
            move_in_family_id: None,
            family_bin: FamilyBin::new(),
            family_bin_entries,
            dialog: None,
            note: None,
            audio_queue,
        })
    }

    pub fn screen(&self) -> ScreenKind {
        match self.screen {
            ActiveScreen::Neighborhood => ScreenKind::Neighborhood,
            ActiveScreen::FamilyBin => ScreenKind::FamilyBin,
            ActiveScreen::Lot(_) => ScreenKind::Lot,
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

    pub fn has_dialog(&self) -> bool {
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.has_dialog(),
            _ => self.dialog.is_some(),
        }
    }

    pub fn dialog_layout(&self) -> Option<&dialog::Layout> {
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.dialog(),
            _ => self.dialog.as_ref().and_then(|d| d.layout.as_ref()),
        }
    }

    pub fn status_note(&self) -> Option<&str> {
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.note(),
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
            ActiveScreen::Neighborhood => music_mode_for_kind(self.kind),
            ActiveScreen::FamilyBin => MusicMode::FamilyBin,
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
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.is_actively_interacting(),
            ActiveScreen::Neighborhood => {
                self.dialog.is_some()
                    || !matches!(self.nbhd_state.mode, NeighborhoodMode::Normal)
                    || self.pressed_lot_id.is_some()
            }
            ActiveScreen::FamilyBin => self.dialog.is_some(),
        }
    }

    /// Window title string appropriate for the current screen.
    pub fn window_title(&self) -> String {
        match &self.screen {
            ActiveScreen::Neighborhood => {
                format!("sims-game: Neighborhood {}", self.slot)
            }
            ActiveScreen::FamilyBin => {
                format!("sims-game: Neighborhood {} — Select a Family", self.slot)
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

    /// Advances shell-level animation timers (e.g. neighborhood water waves).
    /// Returns `true` when the Neighborhood map should be redrawn.
    pub fn tick_shell(&mut self, delta_us: u64) -> bool {
        self.clock_us = self.clock_us.saturating_add(delta_us);
        if !matches!(self.screen, ActiveScreen::Neighborhood) {
            return false;
        }
        let next_wave = (self.clock_us / WATER_STEP_US) as usize;
        if next_wave != self.water_frame {
            self.water_frame = next_wave;
            return !self.map.waves.is_empty();
        }
        false
    }

    pub fn canvas(&self) -> &Canvas {
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.canvas(),
            _ => &self.canvas,
        }
    }

    pub fn render(&mut self) {
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.render();
            return;
        }
        let w = self.size[0].max(1) as usize;
        let h = self.size[1].max(1) as usize;
        if self.canvas.width != w || self.canvas.height != h {
            self.canvas = Canvas::new(w, h, [0, 0, 0]);
        }

        match self.screen {
            ActiveScreen::Neighborhood => {
                let opts = RenderOptions {
                    viewport: [w as u32, h as u32],
                    hovered_lot: self.hovered_lot_id,
                    hover_roofs_off: true,
                    highlight_hovered: true,
                    water_frame: Some(self.water_frame),
                    show_occupancy_markers: matches!(
                        self.nbhd_state.mode,
                        NeighborhoodMode::MoveIn { .. }
                    ),
                    ..RenderOptions::default()
                };
                self.map.render_into(&mut self.canvas, &opts);
                let rgb = std::mem::take(&mut self.canvas.color);
                if let Ok(mut frame) = Frame::from_rgb(w as u32, h as u32, rgb) {
                    self.chrome.draw(
                        &mut frame,
                        &self.nbhd_state,
                        &self.chrome_art,
                        &self.chrome_strings,
                    );
                    self.draw_overlay_note_and_dialog(&mut frame);
                    self.canvas.color = frame.into_rgb();
                } else {
                    self.canvas.color = vec![[0, 0, 0]; w * h];
                }
            }
            ActiveScreen::FamilyBin => {
                let rgb = std::mem::take(&mut self.canvas.color);
                if let Ok(mut frame) = Frame::from_rgb(w as u32, h as u32, rgb) {
                    let view = self.family_bin_view();
                    self.family_bin.draw(
                        &mut frame,
                        &view,
                        &self.family_bin_art,
                        &self.dialogs.family_bin,
                    );
                    self.draw_overlay_note_and_dialog(&mut frame);
                    self.canvas.color = frame.into_rgb();
                } else {
                    self.canvas.color = vec![[0, 0, 0]; w * h];
                }
            }
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    fn draw_overlay_note_and_dialog(&mut self, frame: &mut Frame) {
        if let Some(note) = &self.note
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
                dialog::draw_with_state(
                    frame,
                    &self.dialog_art,
                    &dlg.message,
                    layout,
                    self.pointer,
                    &dlg.state,
                );
            }
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
        if let ActiveScreen::Lot(session) = &mut self.screen {
            return session.game.hover(point);
        }
        if self.dialog.is_some() {
            return true;
        }
        let frame_size = self.frame_size_u32();
        match self.screen {
            ActiveScreen::Neighborhood => {
                let prev_btn = self.chrome.hovered();
                let prev_lot = self.hovered_lot_id;
                let btn = self.chrome.hover(
                    point[0],
                    point[1],
                    frame_size,
                    &self.nbhd_state,
                    &self.chrome_art,
                );
                let over_bar = btn.is_some() || point[1] < sims_ui::neighborhood::BAR_HEIGHT;
                let next_lot = if over_bar {
                    None
                } else {
                    self.map
                        .hit_test_with_state(frame_size, point, self.hovered_lot_id, true)
                };
                self.hovered_lot_id = next_lot;
                if next_lot != prev_lot {
                    self.nbhd_state.hovered_lot = next_lot.and_then(|id| self.build_lot_info(id));
                }
                btn != prev_btn || next_lot != prev_lot
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
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn leave(&mut self) -> bool {
        self.pointer = None;
        if let ActiveScreen::Lot(session) = &mut self.screen {
            return session.game.leave();
        }
        match self.screen {
            ActiveScreen::Neighborhood => {
                let had_state = self.chrome.hovered().is_some()
                    || self.chrome.pressed().is_some()
                    || self.hovered_lot_id.is_some()
                    || self.pressed_lot_id.is_some();
                self.chrome.clear_pointer();
                self.hovered_lot_id = None;
                self.pressed_lot_id = None;
                self.nbhd_state.hovered_lot = None;
                had_state
            }
            ActiveScreen::FamilyBin => {
                self.family_bin.clear_pointer();
                true
            }
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn wheel(&mut self, delta_y: f64) -> bool {
        if self.dialog.is_some() {
            return false;
        }
        match &mut self.screen {
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
        let frame_size = self.frame_size_u32();
        match self.screen {
            ActiveScreen::Neighborhood => {
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
                    self.hovered_lot_id = hit;
                    self.nbhd_state.hovered_lot = hit.and_then(|id| self.build_lot_info(id));
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
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn double_click(&mut self, point: [i32; 2]) -> ShellOutcome {
        self.pointer = Some(point);
        if self.dialog.is_some() {
            return ShellOutcome::default();
        }
        if !matches!(self.screen, ActiveScreen::FamilyBin) {
            return ShellOutcome::default();
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
        ShellOutcome::default()
    }

    pub fn release(&mut self, point: [i32; 2]) -> ShellOutcome {
        self.pointer = Some(point);
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
        let frame_size = self.frame_size_u32();
        match self.screen {
            ActiveScreen::Neighborhood => {
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
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn release_outside(&mut self) -> ShellOutcome {
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
        if self.dialog.is_some() {
            return self.key_dialog(DialogKey::Escape);
        }
        match self.screen {
            ActiveScreen::Neighborhood => {
                if !matches!(self.nbhd_state.mode, NeighborhoodMode::Normal) {
                    self.nbhd_state.mode = NeighborhoodMode::Normal;
                    self.move_in_family_id = None;
                    self.refresh_hovered_lot();
                    return ShellOutcome::redraw();
                }
                self.open_quit_dialog();
                ShellOutcome::redraw()
            }
            ActiveScreen::FamilyBin => {
                let cmd = self.family_bin.cancel();
                self.handle_family_bin_command(cmd)
            }
            ActiveScreen::Lot(_) => ShellOutcome::default(),
        }
    }

    /// Handles a [`crate::game::Outcome`] produced by an input to the active lot
    /// (including Save, Save & Leave to Neighborhood, Quit, and Photo).
    pub fn handle_lot_outcome(&mut self, outcome: crate::game::Outcome) -> ShellOutcome {
        self.persist_options();
        let mut out = ShellOutcome {
            redraw: outcome.redraw,
            quit: false,
            photo: outcome.photo,
        };
        if outcome.neighborhood {
            match self.leave_lot(outcome.save) {
                Ok(()) => {
                    out.redraw = true;
                }
                Err(e) => {
                    if let ActiveScreen::Lot(session) = &mut self.screen {
                        session.game.after_save_failed();
                        session
                            .game
                            .set_note(format!("Could not return to Neighborhood: {e}"));
                    }
                    out.redraw = true;
                }
            }
            return out;
        }
        let saved_ok = if outcome.save {
            self.save_current_lot().is_ok()
        } else {
            true
        };
        if outcome.save
            && !saved_ok
            && let ActiveScreen::Lot(session) = &mut self.screen
        {
            session.game.after_save_failed();
            out.redraw = true;
        }
        if outcome.quit && saved_ok {
            out.quit = true;
        }
        out
    }

    /// Saves the currently open lot to REDB (F5 or Options → Save).
    pub fn save_current_lot(&mut self) -> Result<SaveReport, String> {
        let ActiveScreen::Lot(session) = &mut self.screen else {
            return Err("no lot is currently open".into());
        };
        let report = session.game.save_to_neighborhood(
            &self.install_root,
            &self.store,
            &session.neighborhood_id,
        )?;
        Ok(report)
    }

    /// Reloads the currently open lot from its latest REDB save (F9).
    pub fn restore_current_lot(&mut self) -> Result<(), String> {
        let (neighborhood_id, house) = match &self.screen {
            ActiveScreen::Lot(session) => (session.neighborhood_id.clone(), session.house),
            _ => return Err("no lot is currently open".into()),
        };
        if self
            .store
            .load_saved_lot(&neighborhood_id, house)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            if let ActiveScreen::Lot(session) = &mut self.screen {
                session
                    .game
                    .set_note("The save was not restored: nothing is saved for this house");
            }
            return Err("nothing is saved for this house".into());
        }
        self.enter_lot(house)?;
        if let ActiveScreen::Lot(session) = &mut self.screen {
            let ticks = session.game.lot().ticks();
            session
                .game
                .set_note(format!("Back to the save after tick {ticks}."));
        }
        Ok(())
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
        let family_names = load_family_names(&user_dir);
        let street_names = StreetNames::load(&self.install, &user_dir);
        let descriptions = NeighborhoodDescriptions::load(&user_dir);
        let faces = load_neighborhood_faces(&self.install, &self.store, &id, &stored);
        let lot_meta = build_lot_meta(&self.store, &id, &map, &descriptions, self.arch_multiplier);
        sync_map_with_store(&mut map, &stored, &lot_meta, &id, &self.custom_thumbs);
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
        self.family_bin_entries = family_bin_entries;
        self.family_bin
            .sync(&self.family_bin_entries, sims_ui::family_bin::VISIBLE_ROWS);
        self.nbhd_state.neighborhood_number = slot;
        self.nbhd_state.mode = NeighborhoodMode::Normal;
        self.nbhd_state.hovered_lot = None;
        self.hovered_lot_id = None;
        self.pressed_lot_id = None;
        self.move_in_family_id = None;
        self.dialog = None;
        self.note = None;
        self.screen = ActiveScreen::Neighborhood;
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
        self.nbhd_state.hovered_lot = None;
        self.hovered_lot_id = None;
        self.pressed_lot_id = None;
        self.move_in_family_id = None;
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
        let family_magic_coins = fam
            .magic_history()
            .ok()
            .flatten()
            .map_or(0, |h| i32::from(h[0]));
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

    /// Enters `house` in the active neighborhood via [`lifecycle::enter_lot`].
    pub fn enter_lot(&mut self, house: u32) -> Result<(), String> {
        let mut game = Game::open_from_store(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            house,
            &self.answers,
            self.play,
            self.frames.clone(),
        )?;
        if self.options_file.is_file() {
            let _ = game.load_options(&self.options_file);
        }
        game.resize(self.size);
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: self.neighborhood_id.clone(),
            house,
        }));
        self.dialog = None;
        self.note = None;
        self.hovered_lot_id = None;
        self.pressed_lot_id = None;
        self.nbhd_state.hovered_lot = None;
        self.audio_queue.push(Input::MusicMode(next_music));
        Ok(())
    }

    /// Leaves the currently open lot back to the Neighborhood map, optionally
    /// saving through [`lifecycle::save_and_leave_lot`] first and refreshing
    /// map thumbnails, funds, occupancy, and the Family Bin.
    pub fn leave_lot(&mut self, save_first: bool) -> Result<(), String> {
        if save_first {
            let _ = self.save_current_lot()?;
            if let ActiveScreen::Lot(session) = &mut self.screen
                && let Ok(lot_id) = u16::try_from(session.house)
                && let Some(map_lot) = self.map.lot(lot_id)
            {
                let normal_size = [map_lot.normal.width, map_lot.normal.height];
                let has_cutaway = map_lot.roofs_off.is_some();
                let (world, lot_shape, assets) = session.game.render_parts_mut();
                let thumb =
                    capture_lot_thumbnail(world, lot_shape, assets, normal_size, has_cutaway);
                self.custom_thumbs
                    .insert((session.neighborhood_id.clone(), lot_id), thumb);
            }
        }
        self.persist_options();
        self.screen = ActiveScreen::Neighborhood;
        self.nbhd_state.mode = NeighborhoodMode::Normal;
        self.nbhd_state.hovered_lot = None;
        self.hovered_lot_id = None;
        self.pressed_lot_id = None;
        self.move_in_family_id = None;
        self.dialog = None;
        self.note = None;
        self.refresh_from_store()?;
        self.audio_queue
            .push(Input::Events(vec![SoundEvent::StopAll]));
        self.audio_queue
            .push(Input::MusicMode(music_mode_for_kind(self.kind)));
        Ok(())
    }

    /// Executes Move-In for `family_id` onto `house`, refreshes REDB state, and
    /// immediately enters the newly occupied lot.
    pub fn execute_move_in(&mut self, family_id: u16, house: u32) -> Result<MoveInReport, String> {
        let report = lifecycle::move_in(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            family_id,
            house,
        )
        .map_err(|e| e.to_string())?;
        self.nbhd_state.mode = NeighborhoodMode::Normal;
        self.move_in_family_id = None;
        self.refresh_from_store()?;
        self.enter_lot(house)?;
        Ok(report)
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

    /// Executes Bulldoze on `house`, refreshing map occupancy, vacant lot
    /// thumbnail, and Family Bin immediately.
    pub fn execute_bulldoze(&mut self, house: u32) -> Result<BulldozeReport, String> {
        let report = lifecycle::bulldoze(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            house,
        )
        .map_err(|e| e.to_string())?;
        if let Ok(lot_id) = u16::try_from(house)
            && let Some(map_lot) = self.map.lot(lot_id)
        {
            let grass = vacant_lot_thumbnail(map_lot.normal.width, map_lot.normal.height);
            self.custom_thumbs.insert(
                (self.neighborhood_id.clone(), lot_id),
                CustomThumb {
                    normal: grass,
                    roofs_off: None,
                },
            );
        }
        self.refresh_from_store()?;
        Ok(report)
    }

    /// Permanently removes a Family Bin household from the active neighborhood
    /// in REDB and refreshes the Family Bin list.
    pub fn execute_delete_family(&mut self, family_id: u16) -> Result<(), String> {
        let mut stored = self
            .store
            .load_neighborhood(&self.neighborhood_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))?;
        let origin = (stored.instance_id, stored.revision);
        let Some(fam) = stored.families.get(&family_id) else {
            return Err(format!("family {family_id} not found"));
        };
        if fam.house != 0 || fam.secondary_house() != 0 {
            return Err(format!("family {family_id} is not in the Family Bin"));
        }
        stored.families.remove(&family_id);
        match self
            .store
            .save_neighborhood_checked(&self.neighborhood_id, origin.0, origin.1, &stored)
            .map_err(|e| e.to_string())?
        {
            Ok(_) => {}
            Err(e) => return Err(format!("could not delete family: {e:?}")),
        }
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
        self.faces = load_neighborhood_faces(
            &self.install,
            &self.store,
            &self.neighborhood_id,
            &self.stored,
        );
        self.lot_meta = build_lot_meta(
            &self.store,
            &self.neighborhood_id,
            &self.map,
            &self.descriptions,
            self.arch_multiplier,
        );
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
        self.refresh_hovered_lot();
        Ok(())
    }

    fn refresh_hovered_lot(&mut self) {
        self.nbhd_state.hovered_lot = self.hovered_lot_id.and_then(|id| self.build_lot_info(id));
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
                let magic_coins = fam
                    .magic_history()
                    .ok()
                    .flatten()
                    .map_or(0, |h| i32::from(h[0]));
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
            magic_cost: 0,
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
            NeighborhoodCommand::ToggleRezone
            | NeighborhoodCommand::ImportFamily
            | NeighborhoodCommand::OpenWeb
            | NeighborhoodCommand::OpenCredits => ShellOutcome::redraw(),
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
                self.note = Some("Create-a-Family is not wired into this screen yet.".into());
                ShellOutcome::redraw()
            }
        }
    }

    fn activate_lot(&mut self, lot_id: u16) -> ShellOutcome {
        match self.nbhd_state.mode.clone() {
            NeighborhoodMode::Normal | NeighborhoodMode::Visit | NeighborhoodMode::Rezone => {
                if let Err(e) = self.enter_lot(u32::from(lot_id)) {
                    self.note = Some(format!("Could not enter lot {lot_id}: {e}"));
                }
                ShellOutcome::redraw()
            }
            NeighborhoodMode::MoveIn { family_funds, .. } => {
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
                let has_house = meta.is_some_and(|m| m.has_house);
                if family_funds < cost {
                    // Hover card already explains insufficient funds.
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

    fn open_dialog(&mut self, kind: ShellDialogKind, message: Message) {
        let state = DialogState::for_message(&message);
        let layout = dialog::layout(self.size, &message, &self.dialog_art);
        self.dialog = Some(ShellDialog {
            kind,
            message,
            state,
            layout,
        });
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
            ShellDialogKind::Notice => ShellOutcome::redraw(),
        }
    }
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

fn load_neighborhood_faces(
    install: &Install,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    stored: &StoredNeighborhood,
) -> Faces {
    let Ok(mut index) = Index::build(install) else {
        return Faces::default();
    };
    if let Ok(chars) = store.load_character_files(neighborhood_id)
        && !chars.is_empty()
    {
        let _ = index.add_character_files(&chars);
    }
    let Ok(catalog) = Catalog::build(&index) else {
        return Faces::default();
    };
    let mut guids = Vec::new();
    for fam in stored.families.values() {
        for &guid in &fam.members {
            if !guids.contains(&guid) {
                guids.push(guid);
            }
        }
    }
    Faces::load(&index, &catalog, &guids).unwrap_or_default()
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
            out.insert(
                lot.id,
                LotMeta {
                    house_cost,
                    has_house,
                    from_saved_lot: true,
                    lot_name: default_name,
                    lot_description: default_desc,
                    filter_mask,
                },
            );
            continue;
        }
        let mut house_cost = 0;
        let mut has_house = false;
        if let Ok(Some(base_bytes)) = store.load_base_house(neighborhood_id, house)
            && let Ok(iff) = Iff::parse(base_bytes)
            && let Some(chunk) = iff.chunks.iter().find(|c| &c.tag == b"SIMI")
            && let Ok(sim) = sims_formats::lot::Simulator::parse(iff.data(chunk))
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
        if let Some(thumb) = custom_thumbs.get(&(neighborhood_id.to_owned(), lot.id)) {
            apply_custom_thumb(lot, thumb);
        } else if let Some(meta) = lot_meta.get(&lot.id)
            && meta.from_saved_lot
            && !meta.has_house
        {
            lot.normal = vacant_lot_thumbnail(lot.normal.width, lot.normal.height);
            lot.roofs_off = None;
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
            let magic_coins = fam
                .magic_history()
                .ok()
                .flatten()
                .map_or(0, |h| i32::from(h[0]));
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
            if canvas.depth[idx] == sims_render::depth::EMPTY {
                continue;
            }
            let px = pack_565(canvas.color[idx]);
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
