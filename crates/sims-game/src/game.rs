//! The game without its window: a booted lot, stepped as the player's
//! speed asks, drawn with its people and the live-mode panel, and the
//! player's presses turned into what they ask for.
//!
//! The simulation runs in strict mode ([`Lot::step`]) unless
//! [`Options::play`] asks for play mode. In strict mode the first step
//! that fails stops the lot, which stays on screen as the failure left
//! it, with the failure written across the top. In play mode
//! ([`Lot::step_play`]) an object whose tick fails, usually on something
//! the simulation does not do yet, is reset as the original resets an
//! object after a SimAntics error, and the lot goes on; each new error
//! caught is logged and shown across the top for a while, and only a
//! failure play mode does not catch stops the lot.

use crate::coop::{
    CoopController, CoopPollOutput, CoopPumpStats, CoopStartOptions, CoopTransport, clipboard_copy,
    clipboard_paste, default_player_name, player_badge_color,
};
use crate::destination_picker::{
    DestinationMapPicker, DestinationPickerAction, DestinationPickerAssets,
};
use crate::hud::{DialogKey, Hud, HudShared, HudStatic, OptionsDialog};
use crate::menu::{self, Listing, Menu};
use crate::pace::{self, Pace};
use crate::people::People;
use crate::pick;
use crate::sound;
use crate::view::{Move, View};
use sims_content::assets::Assets as ContentAssets;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{
    Animations, ArchivePool, AvatarRegistry, Catalog, House, Index, Install, Neighborhood,
    People as ContentPeople, SurfacePlugin, Surfaces, Wardrobe,
};
use sims_net::{
    BuyBuildEdit, CameraRotation, Command as NetCommand, DialogId as NetDialogId, IrohConfig,
    ManifestDigest, PeerAddress, PlayerId as NetPlayerId, PlayerMode as NetPlayerMode,
    SimSpeed as NetSimSpeed,
};
use sims_people::{Cast, Library as PeopleLibrary, PreloadedCast};
use sims_render::build_preview::{
    self as render_build, RoofCatalog, RoofPitch as RenderRoofPitch, RoofPreviewConfig, RoofStyle,
    TerrainOverlay, TerrainTarget, TerrainTool as RenderTerrainTool, WaterAction, WaterOverlay,
    WaterTool as RenderWaterTool,
};
use sims_render::{BaseAssets, Camera, Canvas, Selected, Walls};
use sims_ui::art::Art;
use sims_ui::balloon::BalloonFadeState;
use sims_ui::book::{
    BookBadge, BookCommand, BookEntry, BookPage, BookStrings, ReferenceBookDialog,
    ReferenceBookSnapshot,
};
use sims_ui::build::{BuildSubtoolAction, RoofPitchPreset, TerrainSubtool, WaterSubtool};
use sims_ui::camera::{
    CameraButton, CameraCommand, CameraLayout, CameraPanel, CameraState, CameraStrings,
};
use sims_ui::command::{
    BuildItemId, BuildTool, Camera as UiCamera, Command, Mode, OptionsAction, OptionsToggle, Speed,
    VolumeChannel, Walls as UiWalls,
};
use sims_ui::coop::{
    ConnectionState as CoopConnectionState, CoopCommand, CoopHudOverlay, CoopLobbyDialog,
    CoopSnapshot, LobbyTab,
};
use sims_ui::dialog::DialogFadeState;
use sims_ui::options::{LotVisibility, OptionsState};
use sims_ui::phonebook::{
    PhoneBookCommand, PhoneBookDialog, PhoneBookEntry, PhoneBookSnapshot, PhoneBookStrings,
};
use sims_ui::queue::QueueSlideState;
use sims_ui::shop::{
    CurrencyKind, ShopCommand, ShopItem, ShopKind, ShopPickerDialog, ShopSnapshot, ShopStrings,
    Wallet, WardrobeCategory,
};
use sims_vm::Program;
use sims_vm::ShownSpecialPanel;
use sims_vm::boot::save::Saved;
use sims_vm::boot::{
    AskingSpecialPanel, Frames, Lot, PreloadedBoot, StepError, default_character_name,
    load_ordered_wardrobe,
};
use sims_vm::play::{Recovery, ScriptError};
use sims_vm::{
    BuyCursor, FloorDragMode, FloorPlan, PanelMode, PoolMode, PoolPlan, RoofPitch as VmRoofPitch,
    RoofState, SkippedFrame, SoundEvent, SpecialPanelRequest, SpecialPanelResponse, TerrainMode,
    TerrainPlan, UiEffectEvent, WALL_ASCENDING, WALL_DESCENDING, WALL_EAST, WALL_NORTH, WALL_SOUTH,
    WALL_WEST, WallCursor, WallDragMode, WallFace, WallPlan, WallStyle, WallpaperDragMode,
    WallpaperPlan, WaterMode, WaterPlan,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The room word of an object in no room.
const NO_ROOM: i16 = -5;

/// How many ticks an error caught stays across the top: 10 seconds at
/// normal speed.
const SHOWN_TICKS: u32 = 200;

/// The most errors kept in the log; later ones are only counted.
const LOGGED: usize = 256;

/// The most different errors remembered, to show each once. Errors
/// after that are only counted.
const SEEN: usize = 4096;

/// The most sound requests kept for the host to take; the oldest go
/// first. A host that plays sound takes them every wake.
const SOUNDS_KEPT: usize = 4096;

/// What the game is started with.
#[derive(Debug, Clone)]
pub struct Options {
    /// The install's root directory.
    pub install: PathBuf,
    /// A house file inside a neighborhood's `Houses` directory.
    pub house: PathBuf,
    /// Answers to the dialogs that offer a choice, in order, as with
    /// `sims boot --answers` (the Goths' house takes `3`, No to the Help
    /// System). A dialog shown once they run out waits for the player
    /// (`Lot::wait_for_answers`).
    pub answers: Vec<u8>,
    /// Step in play mode ([`Lot::step_play`]), which resets an object
    /// after its error and goes on, instead of strict mode ([`Lot::step`]),
    /// which stops the lot at the first error.
    pub play: bool,
    /// After which ticks the house view's frame is drawn, as far as it
    /// writes the simulation's state (`sims_vm::frame`). When the
    /// original draws one is up to its main loop, not the simulation; the
    /// default is the schedule of the captures of the Goth house without
    /// input. A replay of a recording takes the recording's.
    pub frames: Frames,
}

/// What the host should do after an input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The picture changed.
    pub redraw: bool,
    /// The player asked for a photo of the view.
    pub photo: bool,
    /// The player accepted a Save action from the Options panel (`0x51687e -> 0x4f26cf`).
    pub save: bool,
    /// The player accepted Quit from the Options panel (`0x516866..0x51686c`).
    pub quit: bool,
    /// The player accepted returning to the Neighborhood screen (`0x5170f0..0x51713c`).
    pub neighborhood: bool,
    /// The player clicked the Options → Saves ("Load / Manage Saves") button.
    pub open_save_manager: bool,
    /// The player clicked the Options → Co-op button.
    pub open_coop_lobby: bool,
    /// The player clicked the Camera-panel Photo Album button (`CamViewAlbum`).
    pub open_photo_album: bool,
}

impl Outcome {
    fn redraw() -> Self {
        Self {
            redraw: true,
            ..Self::default()
        }
    }
}

/// How often the window draws the plumb bob's spin when nothing else
/// changes: 20 frames a second, this crate's choice (the original's frame
/// rate is not ported).
pub const SPIN_FRAME_US: u64 = 50_000;

/// Duration of the smooth camera pan glide when double-clicking / re-clicking a
/// Sim's portrait to centre the view (`0x4c9a27`), in microseconds (350 ms).
pub const CAMERA_PAN_DURATION_US: u64 = 350_000;

/// Active smooth camera pan interpolation from `start_origin` to `target_origin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CameraPan {
    start_origin: [i32; 2],
    target_origin: [i32; 2],
    start_us: u64,
    duration_us: u64,
}

impl CameraPan {
    fn sample(&self, now_us: u64) -> ([i32; 2], bool) {
        let elapsed = now_us.saturating_sub(self.start_us);
        if self.duration_us == 0 || elapsed >= self.duration_us {
            return (self.target_origin, true);
        }
        let t = (elapsed as f64) / (self.duration_us as f64);
        let inv = 1.0 - t;
        // Cubic ease-out: fast responsive departure, gentle deceleration onto the Sim.
        let eased = 1.0 - inv * inv * inv;
        let lerp = |a: i32, b: i32| -> i32 {
            let delta = f64::from(b) - f64::from(a);
            (f64::from(a) + delta * eased).round() as i32
        };
        (
            [
                lerp(self.start_origin[0], self.target_origin[0]),
                lerp(self.start_origin[1], self.target_origin[1]),
            ],
            false,
        )
    }
}

/// Real neighborhood / destination lot metadata supplied to destination lot
/// pickers (`Dialog` kinds `5, 7, 8, 11, 13`) so picker rows show real installed
/// lot names, street addresses, and descriptions with zero hardcoded game prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialLotEntry {
    pub lot_id: u16,
    pub name: String,
    pub address: Option<String>,
    pub description: String,
    pub zoning: i16,
}

/// Outfit / skin / pet target resolved for live 3D "Try On" turntable preview rendering.
#[derive(Debug, Clone)]
struct ShopPreviewTarget {
    person: u16,
    suit: String,
    row: Option<String>,
    /// For a pet adoption preview, the pet's skeleton (its type's body
    /// string 0, `STR# 200[0]`); `None` renders a human with a head.
    pet_skeleton: Option<String>,
}

impl ShopPreviewTarget {
    fn human(person: u16, suit: String, row: Option<String>) -> Self {
        Self {
            person,
            suit,
            row,
            pet_skeleton: None,
        }
    }
}

/// Active VM special panel (`Dialog` kinds `5..=15`) currently presented in UI.
#[derive(Debug, Clone)]
struct ActiveSpecialPanel {
    tick: u32,
    request: SpecialPanelRequest,
    coop_dialog_id: Option<NetDialogId>,
    responses: BTreeMap<String, SpecialPanelResponse>,
    preview_targets: BTreeMap<String, ShopPreviewTarget>,
    /// Where the panel came from: `true` for [`Game::preview_special_panel`]
    /// (nothing answers the VM, and [`Game::sync_special_panel`] keeps it
    /// open), `false` for a VM request. Set on every open or replacement.
    preview: bool,
}

impl ActiveSpecialPanel {
    /// Whether this panel already presents the VM's `shown` request. A
    /// preview never does, even one with the same request at tick 0: the
    /// real request must replace it, or a pick would close the preview and
    /// leave the VM waiting.
    fn shows(&self, shown: &ShownSpecialPanel, coop_dialog_id: Option<NetDialogId>) -> bool {
        !self.preview
            && self.tick == shown.tick
            && self.request == shown.request
            && self.coop_dialog_id == coop_dialog_id
    }
}

/// Conservative invariant key for steady-state Tier-2 lot frames where all
/// non-people pixels (lot surface, HUD, overlays) are guaranteed unchanged and
/// pixel differences are strictly bounded by `prev_dynamic_rects ∪ cur_dynamic_rects`.
#[derive(Debug, Clone, PartialEq)]
struct SteadyFrameKey {
    width: usize,
    height: usize,
    viewport: (u32, u32),
    camera: Camera,
    ticks: u32,
    restore_generation: u64,
    world_sound_rev: u64,
    controls: sims_ui::hud::Controls,
    selected: Option<u16>,
    funds: i32,
    paused: bool,
    speed: i16,
    return_speed: i16,
    clock_nonzero: bool,
    queue: sims_ui::queue::Queue,
    hud_items: Vec<sims_ui::hud::Item>,
}

pub struct Game {
    lot: Lot,
    lot_shape: sims_render::Lot,
    assets: sims_render::Assets,
    pub view: View,
    people: People,
    hud: Hud,
    pub pace: Pace,
    play: bool,
    /// Why the lot stopped, once it has.
    stopped: Option<String>,
    errors: Errors,
    /// The steps' sound requests, in order, until the host takes them.
    sounds: VecDeque<SoundEvent>,
    /// UI Effect highlight requests from steps not yet taken by the host.
    ui_effects: VecDeque<UiEffectEvent>,
    /// Active UI Effect highlights (opcode 34, `0x476485`/`0x47690d`/`0x4769c3`).
    highlighted_buttons: BTreeSet<i16>,
    highlighted_portraits: BTreeSet<u32>,
    highlighted_relationships: BTreeSet<u32>,
    /// A short message about the last press, until the next one.
    note: Option<String>,
    /// Where the pointer is on the canvas, while it is over the window.
    pointer: Option<[i32; 2]>,
    /// The object the pointer last reached on the lot, if it is still on
    /// it.
    reached: Option<u16>,
    /// The pie menu listed when the pointer reached an object: for whom,
    /// on what, and the listing, until a click opens it.
    listed: Option<(u16, u16, Result<Listing, String>)>,
    /// The entry of the open pie menu the pointer last highlighted
    /// ([`Game::highlight`]).
    highlighted: Option<usize>,
    /// The Buy Mode catalog item currently armed for placement (`0x486f5a`).
    buy_selected: Option<u32>,
    /// The active Buy Mode `"Tile Cursor"` and held catalog object, once the
    /// pointer has entered the house view (`0x487d3d`).
    buy_cursor: Option<BuyCursor>,
    /// Whether the pointer is currently inside the house view in Buy Mode
    /// (`[tool + 0x97]`, `0x487d3d` / `0x487d4e`), so re-entering after
    /// leaving onto the catalog recreates the preview via `0x486f5a`.
    buy_in_house: bool,
    /// Last Buy Mode catalog GUID instantiated by `0x486f5a` (`tool[0x12]`).
    buy_last_guid: Option<u32>,
    /// Tool-stored Buy Mode facing (`tool[0x10]`, initialized by `0x486f5a`
    /// when switching catalog GUIDs and updated on placement commit
    /// `0x486ada`).
    buy_facing: Option<i16>,
    /// The screen anchor of an active Buy Mode left-button press over the lot
    /// (`0x485f98`), for drag-to-rotate (`dx^2 + dy^2 > 36`, `0x485a2e`) and
    /// commit on release (`0x4864d0`).
    buy_dragging: Option<[i32; 2]>,
    /// Screen-pixel offset from the picked-up object's projected origin to the
    /// pickup click point (`[tool + 0x50]`, `[tool + 0x54]`, `0x486960`),
    /// subtracted before terrain unprojection in `0x485a2e`. Zero for new
    /// catalog purchases (`0x486f5a`).
    buy_grab_offset: [i32; 2],
    /// Native `[tool + 0x81]`: whether releasing the left mouse button commits
    /// the held Buy Mode object (`true` when pressing with an already-active
    /// cursor; `false` on initial Hand Tool pickup until dragged ≥ 2 px,
    /// `0x485f98` / `0x4864d0`).
    buy_commit_on_release: bool,
    /// Saved Live-mode control panel expanded state (`0x4cea5d`), restored
    /// when returning from Buy/Build/Options back to Live Mode.
    live_panel_expanded: bool,
    /// Whether Shift is currently held (`VK_SHIFT`, `0x4864f9`), which rearms
    /// any Buy Mode item for repeat purchase on placement (`0x48680f`), and
    /// switches Build Mode Wall/Floor/Wallpaper tools to Room-Box / Room-Fill.
    shift_held: bool,
    /// Whether Ctrl is currently held (`VK_CONTROL`), which switches Build Mode
    /// Wall/Floor/Wallpaper tools to Demolish / Remove / Strip.
    ctrl_held: bool,
    /// Active Build Mode `"Wall Cursor"` (`0x00000439`) when the Wall or
    /// Wallpaper tool is active.
    wall_cursor: Option<WallCursor>,
    /// Active Build Mode drag operation (Wall, Floor, Wallpaper, Terrain, Water, or Pool).
    build_drag: Option<BuildDrag>,
    /// Temporary rendered lot shape showing live Build Mode drag/Shift preview.
    build_preview_shape: Option<sims_render::Lot>,
    /// Baseline roof state parsed from the house's `HOUS 0` chunk (`RoofState::from_house_view`).
    base_roof: RoofState,
    /// Lazily loaded `GameData/Roofs/` texture catalog for isometric roof previews.
    roof_catalog: Option<RoofCatalog>,
    /// Whether the Camera Roof preview toggle (`UiCamera::Roof`) is active.
    roof_view: bool,
    /// Active Terrain tool hover/drag overlay rendered by `sims_render::build_preview`.
    terrain_hover_overlay: Option<TerrainOverlay>,
    /// Active Water / Pool tool hover/drag overlay rendered by `sims_render::build_preview`.
    water_hover_overlay: Option<WaterOverlay>,
    /// Travel / community-lot Shop, Wardrobe, Pet Adoption & Magic Vendor picker.
    shop_dialog: ShopPickerDialog,
    shop_art: Option<Art>,
    shop_strings: Option<ShopStrings>,
    shop_snapshot: Option<ShopSnapshot>,
    shop_preview_assets: Option<sims_people::Assets>,
    shop_preview_yaw_deg: i16,
    shop_preview_body: Option<(String, sims_people::appearance::Body)>,
    shop_preview_targets: BTreeMap<String, ShopPreviewTarget>,
    purchased_shop_items: BTreeSet<String>,
    /// Native paginated Spellbook (kind 12) & Cookbook (kind 15) reference modal.
    book_dialog: ReferenceBookDialog,
    book_art: Option<Art>,
    book_strings: Option<BookStrings>,
    book_snapshot: Option<ReferenceBookSnapshot>,
    /// Native `cWinPhoneBook` (`phonebookbkg.bmp` `80`, `phoneicon.bmp` `81`) modal
    /// for `Dialog` kind 10 (`CallNeighbor`).
    phonebook_dialog: PhoneBookDialog,
    phonebook_art: Option<Art>,
    phonebook_strings: Option<PhoneBookStrings>,
    phonebook_snapshot: Option<PhoneBookSnapshot>,
    /// Real graphical destination map picker for VM special lot pickers
    /// (`Dialog` kinds `5, 7, 8, 11, 13`: Downtown, Vacation, Old Town,
    /// Studio Town, Magic Town).
    destination_picker: Option<DestinationMapPicker>,
    destination_picker_assets: Option<DestinationPickerAssets>,
    /// Neighborhood slot (`1..=8`) owning this lot, used to load the matching
    /// graphical destination map when a special lot picker opens.
    neighborhood_slot: u8,
    /// Currently presented VM special panel (`Dialog` kinds `5..=15`), if any.
    active_special_panel: Option<ActiveSpecialPanel>,
    /// Co-op special-panel dialog ID already answered locally plus pump count
    /// elapsed since submission; cleared when the lockstep commit resolves or
    /// rejects the panel, or after a bounded retry wait if host ingress drops
    /// the command.
    submitted_coop_special_panel: Option<(NetDialogId, u32)>,
    /// Lazily cached native ordered wardrobe (`ImportWardrobe`) matching
    /// `sims-vm` pet/human skin indices for special pickers (kinds 6, 9, 14).
    ordered_wardrobe: Option<sims_content::ImportWardrobe>,
    /// Destination lot chosen via a VM special lot picker (`5, 7, 8, 11, 13`)
    /// awaiting `NeighborhoodShell` travel transition (`depart_to_destination`).
    pending_travel_lot: Option<(SpecialPanelRequest, u32)>,
    /// Neighborhood / destination lot metadata passed by `NeighborhoodShell`
    /// (populated lazily from `Install` on standalone launches).
    special_lot_catalog: Vec<SpecialLotEntry>,
    /// Tracks whether temporal UI animations were running on the previous tick
    /// so crossing settlement always emits one final landed redraw frame.
    ui_animating_prev: bool,
    /// Pending Simlish voice requests to be drained by `sound::Feed::update`.
    voice_inputs: VecDeque<sound::Input>,
    /// Last `clock_us` when `UI_moneyback` (`0x492ad4`) or `UI_archtool_delete`
    /// (`0x492b05`) played, enforcing the native 400 ms sound cooldown.
    buy_moneyback_us: Option<u64>,
    buy_delete_us: Option<u64>,
    /// The time the window has passed in, in microseconds: the clock
    /// that spins the plumb bob and drives the pause button's 500 ms blink.
    clock_us: u64,
    /// `clock_us` when the frame was last drawn.
    drawn_us: u64,
    /// Whether the Live-mode pause button's blink phase flipped since the
    /// last frame was drawn.
    blink_dirty: bool,
    /// Whether the lot has unsaved changes (`[app + 0xf7]`, checked by Quit at
    /// `0x516809` and Neighborhood at `0x5170ff`).
    dirty: bool,
    /// When the player chooses No on an edit-mode Save Card during Quit
    /// (`0x51682c..0x516852`), `dirty` is cleared temporarily for the follow-up
    /// `ConfirmQuit` prompt and restored if the player declines to quit.
    restore_dirty_on_quit_cancel: bool,
    /// Status note to append after a follow-up host save succeeds (used by
    /// Neighborhood and Reset Tutorial save-before-action flows so `save_to`
    /// does not overwrite the unported-action note).
    pending_after_save_note: Option<&'static str>,
    /// Optional follow-up Options dialog opened automatically once an active
    /// `OptionsDialog::Notice` is dismissed (used to chain a Save & Quit write
    /// failure notice into `OptionsDialog::ConfirmSaveBeforeQuit`).
    pending_after_notice: Option<OptionsDialog>,
    /// Whether the lot is open in neighborhood edit mode (`[lot + 0x50] == 1`,
    /// `0x4f1b5f..0x4f1be5`, `0x4f24b5`, `0x4f24d9`, `0x517c21`). Enabled when
    /// opening any vacant zoned lot or a lot with `global[32] == 1`.
    edit_mode: bool,
    /// Exact fixed-point sub-pixel remainder in `1 / 16_000_000_000` px units
    /// carried across `edge_scroll` ticks so straight-edge (`562.5` px/s,
    /// `0x4d499d`, `0x4d4a06`, `0x4d4839`, `0x4d4946`) and corner-capped
    /// (`0x4d4ab5`) scroll speeds are completely independent of event-loop
    /// wakeup splitting.
    edge_scroll_rem: [i128; 2],
    canvas: Canvas,
    lot_cache: sims_render::LotCache,
    /// The camera the frame last drawn was drawn through, the depth its
    /// lot left before anything else was drawn, and its IDs once its
    /// people were drawn, before the objects holding them: what the
    /// pointer picks from ([`Game::under`]).
    drawn_camera: Camera,
    /// Fallback lot depth buffer used only when `lot_depth_from_cache` is
    /// false (uncached/zero-area canvas or after `clear_lot_cache()`). On
    /// normal cached draws, [`Game::under`] borrows `&[u16]` directly from
    /// [`LotCache::last_lot_depth`] with zero per-frame copy.
    lot_depth: Vec<u16>,
    lot_depth_from_cache: bool,
    people_ids: sims_render::Ids,
    /// Reusable per-frame tight bounding rects `[x0, y0, x1, y1)` written by
    /// `People::draw_in_viewport_tracked` (used for sparse `people_ids` capture
    /// and safe partial-damage presentation).
    people_rects: Vec<sims_render::canvas::Rect>,
    /// Dynamic rects (`people_rects` + any `draw_held` rects) from the last
    /// completed `Game::draw`.
    prev_dynamic_rects: Vec<sims_render::canvas::Rect>,
    /// Key describing all non-people inputs to `Game::draw` for proving that a
    /// steady-state Tier-2 frame only changed pixels inside
    /// `prev_dynamic_rects ∪ cur_dynamic_rects`.
    prev_steady_key: Option<SteadyFrameKey>,
    /// Accumulated canvas-space damage rects since the last successful window
    /// present, or `None` when a full blit/present is required.
    present_damage: Option<Vec<sims_render::canvas::Rect>>,
    /// Explicit override enabling 400 ms pie-menu animations even at `clock_us == 0`.
    /// Also enabled automatically whenever `clock_us > 0` (live window / `advance`).
    pie_animations: bool,
    /// When `true` (`--debug`), draws the maroon top diagnostic/status banner
    /// and logs diagnostic notes to stderr. Defaults to `false`.
    debug: bool,
    /// Last diagnostic note logged to stderr while `debug` is enabled.
    last_debug_logged_note: Option<String>,
    /// Explicit override enabling smooth camera pan interpolation on portrait
    /// centring even at `clock_us == 0`. Enabled automatically when `clock_us > 0`.
    smooth_camera_pan: bool,
    /// Active smooth camera pan toward a Sim's centred origin, if any.
    camera_pan: Option<CameraPan>,
    /// Set whenever `step_camera_pan` advances the camera origin, consumed by
    /// the desktop host loop so the final landing frame always redraws.
    camera_pan_moved: bool,
    /// Classic cheat console toggles (`map_edit`, `autonomy`, `sim_speed`).
    /// (`move_objects` is stored natively on `self.lot`.)
    cheat_map_edit: bool,
    cheat_autonomy: Option<u8>,
    cheat_sim_speed: Option<i16>,
    /// True once `Lot::restore` cleared `neighborhood_origin` (after loading a save
    /// slot or applying a world-mutating cheat), signaling `NeighborhoodShell` to
    /// write back via fenced `StoredNeighborhood::write_back_world`.
    world_mutated_via_restore: bool,
    /// Monotonic counter bumped whenever `Game::restore` resets `self.errors`
    /// and live lot state, so `App` resets its error-log index and audio feed.
    restore_generation: u64,
    /// Authoritative frame draw schedule configured when this lot booted.
    frames: Frames,
    /// Live co-op multiplayer session controller (`Idle` in single-player).
    coop: CoopController,
    /// Host / Join co-op lobby modal dialog (`sims_ui::coop::CoopLobbyDialog`).
    coop_lobby: CoopLobbyDialog,
    /// Lazily loaded 9-slice / button art for the co-op lobby and HUD overlay.
    coop_art: Option<Art>,
    /// Local player display name used when hosting or joining co-op.
    coop_player_name: String,
    /// `iroh` binding mode for co-op sessions (`P2pDefault` or `Loopback`).
    coop_iroh_config: IrohConfig,
    /// Set by `NeighborhoodShell` while on an away-from-home travel visit or
    /// destination edit lot so v1 home-lot co-op refuses cleanly.
    coop_travel_blocked: bool,
    /// Latched `true` once this `Game` joins as a co-op peer or restores a
    /// remote host's `"SCOP"` v2 snapshot. Stays `true` even after disconnect
    /// so a remote host's world can NEVER overwrite local saves or autosaves.
    coop_remote_world: bool,
    /// Compact quick-action buttons rendered beneath `CoopHudOverlay` (`Co-op` / `Copy` / `Leave`).
    coop_overlay_buttons: CoopOverlayButtons,
    /// Local-only Buy/Move placement preview in co-op mode (never mutates `self.lot.world()`).
    coop_buy_preview: Option<CoopBuyPreview>,
    /// Monotonic counter bumped on world-mutating operations so
    /// [`sound::Feed::update`] can dirty-gate `Snapshot` rebuilds in O(1).
    world_sound_rev: u64,
    /// `true` when running a clean-room standalone `open-pack/` lot with zero
    /// commercial EA/Maxis assets (uses 2D sprite Sim rendering and procedural
    /// UI art instead of commercial 3D `.skn`/`.cmx` meshes).
    open_pack_mode: bool,
    /// Keeps the compiled `open-pack/` temporary staging directory alive for
    /// the lifetime of this [`Game`].
    _open_pack_staging: Option<crate::open_pack::OpenPackStagingDir>,
    /// Synthesized PCM WAV sound bank from `open-pack/` (for `Mixer::open_pack`).
    open_pack_wavs: Option<BTreeMap<String, Vec<u8>>>,
    /// Stable save slot key derived from the source `open-pack/` directory and
    /// lot manifest (never from the temporary staging directory).
    open_pack_save_slot: Option<String>,
}

/// Immutable install-wide assets cached on [`crate::neighborhood::NeighborhoodShell`]
/// across lot entries and travel transitions so switching or re-entering lots
/// avoids re-scanning `SIMS_INSTALL` (`Index`, `Catalog`, `ArchivePool`,
/// surface plug-ins, `HudStatic`, `HudShared`, `ContentAssets`, and
/// `PeopleLibrary`).
pub struct InstallAssets {
    pub(crate) install: Install,
    pub(crate) pool: Arc<ArchivePool>,
    pub(crate) base_index: Arc<Index>,
    pub(crate) base_catalog: Arc<Catalog>,
    pub(crate) floor_plugins: BTreeMap<u16, SurfacePlugin>,
    pub(crate) wall_plugins: BTreeMap<u16, SurfacePlugin>,
    pub(crate) hud_static: HudStatic,
    pub(crate) hud_shared: HudShared,
    pub(crate) content_assets: ContentAssets,
    pub(crate) people_library: PeopleLibrary,
    pub(crate) avatar_registry: Option<Arc<AvatarRegistry>>,
    pub(crate) base_preloaded_cast: Option<PreloadedCast>,
    pub(crate) vm_cache: Arc<sims_vm::lifecycle::VmInstallCache>,
}

impl InstallAssets {
    /// Returns the warm install-wide VM cache reused across lot entries.
    pub fn vm_cache(&self) -> &Arc<sims_vm::lifecycle::VmInstallCache> {
        &self.vm_cache
    }

    /// Loads all immutable install-wide assets in parallel, optionally reusing
    /// an already-opened [`ArchivePool`] and base [`Index`] (e.g. from
    /// [`crate::neighborhood::NeighborhoodShell::open`]).
    pub fn load(
        install: &Install,
        pool: Arc<ArchivePool>,
        existing_base_index: Option<Arc<Index>>,
        existing_base_catalog: Option<Arc<Catalog>>,
    ) -> Result<Self, String> {
        let (idx_cat_shared_res, plugins, hud_static_res, people_res, registry_res, vm_cache_res) =
            std::thread::scope(|scope| {
                let plugins_handle = scope.spawn(|| Surfaces::plugins_with_pool(install, &pool));
                let hud_handle = scope.spawn(|| HudStatic::load(install, Arc::clone(&pool)));
                let people_handle = scope.spawn(|| -> Result<_, String> {
                    let content_assets = ContentAssets::scan(&install.root)?;
                    let people_library = PeopleLibrary::scan_with_pool(&content_assets, &pool)?;
                    Ok((content_assets, people_library))
                });
                let registry_handle =
                    scope.spawn(|| AvatarRegistry::load_with_pool(install, &pool));
                let vm_cache_handle = scope.spawn(|| {
                    sims_vm::lifecycle::VmInstallCache::load(&install.root)
                        .map(Arc::new)
                        .map_err(|e| e.to_string())
                });
                let idx_cat_shared_res = (|| -> Result<_, String> {
                    let base_index = match existing_base_index {
                        Some(idx) => idx,
                        None => Arc::new(Index::build(install).map_err(|e| e.to_string())?),
                    };
                    let base_catalog = match existing_base_catalog {
                        Some(cat) => cat,
                        None => Arc::new(
                            Catalog::build_with_pool(&base_index, &pool)
                                .map_err(|e| e.to_string())?,
                        ),
                    };
                    let hud_shared = HudShared::load(&base_index, &base_catalog, &pool)?;
                    Ok((base_index, base_catalog, hud_shared))
                })();
                (
                    idx_cat_shared_res,
                    plugins_handle
                        .join()
                        .expect("install assets surface plugins worker panicked"),
                    hud_handle
                        .join()
                        .expect("install assets hud static worker panicked"),
                    people_handle
                        .join()
                        .expect("install assets people worker panicked"),
                    registry_handle
                        .join()
                        .expect("install assets avatar registry worker panicked"),
                    vm_cache_handle
                        .join()
                        .expect("install assets vm cache worker panicked"),
                )
            });
        let (base_index, base_catalog, hud_shared) = idx_cat_shared_res?;
        let (floor_plugins, wall_plugins) = plugins;
        let hud_static = hud_static_res?;
        let (content_assets, people_library) = people_res?;
        let vm_cache = vm_cache_res?;
        let avatar_registry = registry_res.ok().map(Arc::new);
        let base_preloaded_cast = avatar_registry.as_ref().and_then(|registry| {
            let person_types = base_catalog
                .objects
                .iter()
                .filter(|o| o.kind == 2)
                .map(|o| o.guid);
            ContentPeople::load_each_with_registry(
                &base_index,
                &base_catalog,
                person_types,
                registry,
                &pool,
            )
            .ok()
            .map(|(people_types, unresolved_people)| PreloadedCast {
                skeleton_names: people_types
                    .skeletons
                    .into_iter()
                    .map(|(guid, skel)| (guid, skel.name))
                    .collect(),
                unresolved: unresolved_people,
            })
        });
        Ok(Self {
            install: install.clone(),
            pool,
            base_index,
            base_catalog,
            floor_plugins,
            wall_plugins,
            hud_static,
            hud_shared,
            content_assets,
            people_library,
            avatar_registry,
            base_preloaded_cast,
            vm_cache,
        })
    }

    /// Returns a [`PreloadedCast`] matching `(index, catalog)`. When
    /// neighborhood character files are mounted, reuses cached base entries
    /// only when both the winning `OBJD` definition and its resolved
    /// semi-global file are unchanged, re-resolving all inline or affected
    /// person types against the cached [`AvatarRegistry`].
    pub(crate) fn preloaded_cast_for(
        &self,
        index: &Index,
        catalog: &Catalog,
        has_character_files: bool,
    ) -> Option<PreloadedCast> {
        let mut preloaded = self.base_preloaded_cast.clone()?;
        if !has_character_files {
            return Some(preloaded);
        }
        let registry = self.avatar_registry.as_ref()?;
        let semi_global_loc = |idx: &Index, cat: &Catalog, resource: usize| {
            cat.semi_globals
                .get(&resource)
                .map(|opt| opt.map(|target| idx.resources[target].location(&idx.root)))
        };
        let mut to_resolve = Vec::new();
        for (&guid, &obj_idx) in &catalog.by_guid {
            let obj = &catalog.objects[obj_idx];
            if obj.kind != 2 {
                preloaded.skeleton_names.remove(&guid);
                preloaded.unresolved.remove(&guid);
                continue;
            }
            let unchanged = !index.resources[obj.resource].is_inline()
                && self.base_catalog.object(guid).is_some_and(|base_obj| {
                    base_obj.kind == 2
                        && base_obj.chunk == obj.chunk
                        && index.resources[obj.resource].location(&index.root)
                            == self.base_index.resources[base_obj.resource]
                                .location(&self.base_index.root)
                        && semi_global_loc(index, catalog, obj.resource)
                            == semi_global_loc(
                                &self.base_index,
                                &self.base_catalog,
                                base_obj.resource,
                            )
                });
            if !unchanged {
                to_resolve.push(guid);
            }
        }
        if !to_resolve.is_empty() {
            let (extra, unresolved) = ContentPeople::load_each_with_registry(
                index, catalog, to_resolve, registry, &self.pool,
            )
            .ok()?;
            for (guid, skel) in extra.skeletons {
                preloaded.unresolved.remove(&guid);
                preloaded.skeleton_names.insert(guid, skel.name);
            }
            for (guid, reason) in unresolved {
                preloaded.skeleton_names.remove(&guid);
                preloaded.unresolved.insert(guid, reason);
            }
        }
        Some(preloaded)
    }
}

impl Game {
    /// Boots the house and reads everything drawing it needs.
    pub fn open(options: &Options) -> Result<Self, String> {
        let house = &options.house;
        let install = Install::detect(&options.install).map_err(|e| e.to_string())?;
        let pool = Arc::new(ArchivePool::new());

        // Stage A: load all independent install/neighborhood inputs in parallel
        // once, sharing a single ArchivePool across every FAR read. Error
        // checks are deferred to match the exact original sequential order:
        // `open_house` -> `Lot::load` -> `Hud::open` -> `People::open`.
        let (
            index_catalog_res,
            people_assets_res,
            ordered_wardrobe,
            registry_res,
            surfaces_res,
            hud_static_res,
            base_assets_res,
            icon_frames_res,
            default_name,
        ) = std::thread::scope(|scope| {
            let idx_handle = scope.spawn(|| -> Result<_, String> {
                let mut index = Index::build(&install).map_err(|e| e.to_string())?;
                let neighborhood_dir = house
                    .parent()
                    .and_then(Path::parent)
                    .ok_or("the house file is not in a Houses folder")?;
                let neighborhood =
                    Neighborhood::open(neighborhood_dir).map_err(|e| e.to_string())?;
                let characters = neighborhood.characters().map_err(|e| e.to_string())?;
                if let Some(ref chars) = characters {
                    index.add_characters(chars).map_err(|e| e.to_string())?;
                }
                let catalog = Catalog::build_with_pool(&index, &pool).map_err(|e| e.to_string())?;
                let house_file = House::load(house, &catalog).map_err(|e| e.to_string())?;
                // Match `sims_render::open_house` house validation and initial
                // screen-centre pick before consuming `BaseAssets` or VM boot results.
                let mut world = sims_sim::World::from_house(&house_file, &catalog)
                    .map_err(|e| e.to_string())?;
                let version = house_file
                    .version()
                    .ok_or_else(|| "the house file has no HOUS 1 chunk".to_owned())?;
                if let Some(&requested) = world.globals.get(23) {
                    let requested = if requested == 0 {
                        sims_sim::FULL_SIZE
                    } else {
                        requested
                    };
                    if (4..=sims_sim::FULL_SIZE).contains(&requested) {
                        world.size = requested;
                    }
                }
                for id in sims_sim::load::FLOORS
                    .into_iter()
                    .chain(sims_sim::load::WALLS)
                {
                    world.arrays.remove(&id);
                }
                world
                    .read_arrays(&house_file.arrays, version)
                    .map_err(|e| e.to_string())?;
                world.rebuild_rooms().map_err(|e| e.to_string())?;
                let opening_rotation = house_file
                    .view
                    .as_ref()
                    .map_or(0, |v| (v.camera_direction & 3) as u8);
                let initial_centre = sims_render::Lot::from_world(&world).ok().and_then(|shape| {
                    let shape = shape.with_floor_rotation(opening_rotation);
                    let view = View::new([800, 600], shape.built_center(), opening_rotation);
                    let mut camera = view.camera;
                    camera.origin[0] = camera.origin[0].saturating_add(pick::ORIGIN_COLUMN);
                    sims_render::screen_centre_pick(&shape, &camera, [800, 600])
                        .map(|[x, y]| [(x >> 4) as i8, (y >> 4) as i8])
                });
                Ok((
                    neighborhood,
                    characters,
                    index,
                    catalog,
                    house_file,
                    initial_centre,
                ))
            });
            let people_handle = scope.spawn(|| match ContentAssets::scan(&install.root) {
                Ok(content_assets) => {
                    let (wardrobe, library_res) = std::thread::scope(|inner| {
                        let lib_handle =
                            inner.spawn(|| PeopleLibrary::scan_with_pool(&content_assets, &pool));
                        let wardrobe = Wardrobe::load_with_pool(&content_assets, &pool);
                        (
                            wardrobe,
                            lib_handle.join().expect("people library worker panicked"),
                        )
                    });
                    (
                        wardrobe,
                        library_res.map(|people_library| (content_assets, people_library)),
                    )
                }
                Err(error) => (Wardrobe::unread(error.clone()), Err(error)),
            });
            let ordered_ward_handle = scope.spawn(|| load_ordered_wardrobe(&install, &pool));
            let reg_handle = scope.spawn(|| AvatarRegistry::load_with_pool(&install, &pool));
            let surf_handle =
                scope.spawn(|| Surfaces::build_with_plugins_and_pool(&install, &pool));
            let hud_handle = scope.spawn(|| HudStatic::load(&install, Arc::clone(&pool)));
            let base_assets_res = BaseAssets::load(&install).map_err(|e| e.to_string());
            let icon_frames_res = sims_vm::icon_frames(&install);
            let default_name = default_character_name(&install);
            (
                idx_handle.join().expect("index/catalog worker panicked"),
                people_handle.join().expect("people assets worker panicked"),
                ordered_ward_handle
                    .join()
                    .expect("ordered wardrobe worker panicked"),
                reg_handle.join().expect("avatar registry worker panicked"),
                surf_handle.join().expect("surfaces worker panicked"),
                hud_handle.join().expect("hud static worker panicked"),
                base_assets_res,
                icon_frames_res,
                default_name,
            )
        });

        let (neighborhood, characters, index, catalog, house_file, initial_centre) =
            index_catalog_res?;
        let base_roof = house_file
            .view
            .as_ref()
            .map(RoofState::from_house_view)
            .unwrap_or_default();
        let index = Arc::new(index);
        let catalog = Arc::new(catalog);
        let (surfaces_res, floor_plugins, wall_plugins) = match surfaces_res {
            Ok((surfaces, floor_plugins, wall_plugins)) => {
                (Ok(surfaces), floor_plugins, wall_plugins)
            }
            Err(error) => (
                Err(error),
                std::collections::BTreeMap::new(),
                std::collections::BTreeMap::new(),
            ),
        };
        let assets = sims_render::Assets::from_base(
            base_assets_res?,
            floor_plugins,
            wall_plugins,
            Arc::clone(&index),
            Arc::clone(&catalog),
            Arc::clone(&pool),
        );

        // Stage B: link Program, load Animations, resolve person types, and load
        // Index/Catalog-dependent HUD tables in parallel.
        let (program_res, animations_res, people_res, hud_shared_res) =
            std::thread::scope(|scope| {
                let prog_handle = scope.spawn(|| {
                    Program::link_with_pool(
                        &index,
                        &catalog,
                        catalog.objects.iter().map(|o| o.guid),
                        &pool,
                    )
                });
                let anim_handle =
                    scope.spawn(|| Animations::load_with_pool(&install, &index, &pool));
                let hud_shared_handle = scope.spawn(|| HudShared::load(&index, &catalog, &pool));
                let person_types = catalog
                    .objects
                    .iter()
                    .filter(|o| o.kind == 2)
                    .map(|o| o.guid);
                let people_res = ContentPeople::load_each_with_registry_result(
                    &index,
                    &catalog,
                    person_types,
                    registry_res,
                    &pool,
                );
                (
                    prog_handle.join().expect("program link worker panicked"),
                    anim_handle.join().expect("animations worker panicked"),
                    people_res,
                    hud_shared_handle
                        .join()
                        .expect("hud shared worker panicked"),
                )
            });

        let preloaded_cast = people_res
            .as_ref()
            .ok()
            .map(|(people_types, unresolved_people)| PreloadedCast {
                skeleton_names: people_types
                    .skeletons
                    .iter()
                    .map(|(&guid, skeleton)| (guid, skeleton.name.clone()))
                    .collect(),
                unresolved: unresolved_people.clone(),
            });
        // `Lot::boot` keeps a loaded `Wardrobe` even if `PeopleLibrary::scan`
        // fails, and records a `ContentAssets::scan` error in `Wardrobe::unread`;
        // `People::open` reports either error later after `Hud::open`.
        let (wardrobe, people_assets_for_cast) = people_assets_res;

        // Stage C: run the deterministic lot boot from preloaded content, then
        // assemble HUD portraits and 3D people in parallel against the booted world.
        let mut lot = Lot::load_preloaded(
            house,
            &options.answers,
            options.frames.clone(),
            PreloadedBoot {
                install: &install,
                neighborhood: &neighborhood,
                characters: characters.as_deref(),
                index: &index,
                catalog: Arc::clone(&catalog),
                pool: &pool,
                file: Some(house_file),
                program: program_res,
                animations: animations_res,
                people: people_res,
                icon_frames: icon_frames_res,
                wardrobe,
                ordered_wardrobe,
                surfaces: surfaces_res,
                default_character_name: default_name,
                view_centre: initial_centre,
            },
        )
        .map_err(|e| format!("{}: {e}", house.display()))?;
        // The player answers a dialog when it is shown.
        lot.wait_for_answers();
        let stopped = lot
            .load_report()
            .stopped
            .as_ref()
            .map(|s| format!("The load stopped, so the lot cannot run: {s}"));
        // The booted world, not the file as read: some objects (doors,
        // windows) set how they are drawn in their own init scripts.
        let lot_shape = sims_render::Lot::from_world(lot.world())
            .map_err(|e| e.to_string())?
            .with_floor_rotation(lot.floor_rotation());
        let friendship = lot.program().autonomy_constants().friendship_threshold;
        let world = lot.world();
        let (hud_res, people_open_res) = std::thread::scope(|scope| {
            let people_handle = scope.spawn(|| -> Result<People, String> {
                let (content_assets, people_library) = people_assets_for_cast?;
                let cast = Cast::from_shared(
                    install.clone(),
                    Arc::clone(&index),
                    Arc::clone(&catalog),
                    preloaded_cast,
                    Arc::clone(&pool),
                )?;
                let mut people = People::from_inner(sims_people::People::new(
                    content_assets,
                    people_library,
                    cast,
                ));
                people.refresh(world);
                Ok(people)
            });
            let hud_res = hud_static_res.and_then(|hud_static| {
                Hud::from_preloaded(
                    &install,
                    house,
                    &neighborhood,
                    &assets,
                    world,
                    friendship,
                    hud_static,
                    hud_shared_res,
                    &pool,
                )
            });
            (
                hud_res,
                people_handle
                    .join()
                    .expect("people refresh worker panicked"),
            )
        });
        let mut hud = hud_res?;
        hud.sync_clock_blink(lot.world(), 0);
        // `0x4e6704 -> 0x432220`: apply persisted `UserData/Options.iff` Free
        // Will preference to lot global 30 when it differs from the lot file.
        let free_will = hud.controls.options.play.free_will;
        if lot.world().globals.get(30).copied().unwrap_or(1) != i16::from(free_will) {
            let _ = lot.set_free_will(free_will);
        }
        let mut people = people_open_res?;
        // In case `set_free_will` changed anything (it only sets global 30),
        // refreshing is an instant no-op when people are unchanged.
        people.refresh(lot.world());
        let view = View::new([800, 600], lot_shape.built_center(), lot.view_rotation());
        let drawn_camera = view.camera;
        let has_family = lot.world().globals.get(9).copied().unwrap_or(0) != 0;
        // `0x4f1b5f..0x4f1b76`: any vacant lot with nonzero zoning (residential
        // zoning 1 such as `House02.iff` as well as zoning 2) or in the special
        // neighborhood ranges enters edit mode (`global[32] = 1`).
        let edit_mode = lot.world().globals.get(32).copied().unwrap_or(0) == 1
            || (!has_family
                && (lot.lot_zoning().unwrap_or(0) != 0
                    || matches!(lot.lot_id(), 21..=30 | 40..=48 | 81..=99)));
        let mut game = Self {
            lot,
            lot_shape,
            assets,
            view,
            people,
            hud,
            pace: Pace::default(),
            play: options.play,
            stopped,
            errors: Errors::default(),
            sounds: VecDeque::new(),
            ui_effects: VecDeque::new(),
            highlighted_buttons: BTreeSet::new(),
            highlighted_portraits: BTreeSet::new(),
            highlighted_relationships: BTreeSet::new(),
            pointer: None,
            reached: None,
            listed: None,
            highlighted: None,
            buy_selected: None,
            buy_cursor: None,
            buy_in_house: false,
            buy_last_guid: None,
            buy_facing: None,
            buy_dragging: None,
            buy_grab_offset: [0, 0],
            buy_commit_on_release: false,
            live_panel_expanded: true,
            shift_held: false,
            ctrl_held: false,
            wall_cursor: None,
            build_drag: None,
            build_preview_shape: None,
            base_roof,
            roof_catalog: None,
            roof_view: false,
            terrain_hover_overlay: None,
            water_hover_overlay: None,
            shop_dialog: ShopPickerDialog::new(),
            shop_art: None,
            shop_strings: None,
            shop_snapshot: None,
            shop_preview_assets: None,
            shop_preview_yaw_deg: 0,
            shop_preview_body: None,
            shop_preview_targets: BTreeMap::new(),
            purchased_shop_items: BTreeSet::new(),
            book_dialog: ReferenceBookDialog::new(),
            book_art: None,
            book_strings: None,
            book_snapshot: None,
            phonebook_dialog: PhoneBookDialog::new(),
            phonebook_art: None,
            phonebook_strings: None,
            phonebook_snapshot: None,
            destination_picker: None,
            destination_picker_assets: None,
            neighborhood_slot: 1,
            active_special_panel: None,
            submitted_coop_special_panel: None,
            ordered_wardrobe: None,
            pending_travel_lot: None,
            special_lot_catalog: Vec::new(),
            ui_animating_prev: false,
            voice_inputs: VecDeque::new(),
            buy_moneyback_us: None,
            buy_delete_us: None,
            note: None,
            clock_us: 0,
            drawn_us: 0,
            blink_dirty: false,
            dirty: false,
            restore_dirty_on_quit_cancel: false,
            pending_after_save_note: None,
            pending_after_notice: None,
            edit_mode,
            edge_scroll_rem: [0, 0],
            canvas: Canvas::new(1, 1, [0, 0, 0]),
            lot_cache: sims_render::LotCache::new(),
            drawn_camera,
            lot_depth: Vec::new(),
            lot_depth_from_cache: false,
            people_ids: sims_render::Ids::default(),
            people_rects: Vec::new(),
            prev_dynamic_rects: Vec::new(),
            prev_steady_key: None,
            present_damage: None,
            pie_animations: false,
            debug: false,
            last_debug_logged_note: None,
            smooth_camera_pan: false,
            camera_pan: None,
            camera_pan_moved: false,
            cheat_map_edit: false,
            cheat_autonomy: None,
            cheat_sim_speed: None,
            world_mutated_via_restore: false,
            restore_generation: 0,
            frames: options.frames.clone(),
            coop: CoopController::new(),
            coop_lobby: CoopLobbyDialog::new(),
            coop_art: None,
            coop_player_name: default_player_name("Player"),
            coop_iroh_config: IrohConfig::P2pDefault,
            coop_travel_blocked: false,
            coop_remote_world: false,
            coop_overlay_buttons: CoopOverlayButtons::default(),
            coop_buy_preview: None,
            world_sound_rev: 0,
            open_pack_mode: false,
            _open_pack_staging: None,
            open_pack_wavs: None,
            open_pack_save_slot: None,
        };
        game.sync_view();
        if edit_mode && game.stopped.is_none() {
            // `0x4f1bd9..0x4f1be5`, `0x4f2510..0x4f2520`, `0x46d8c9`:
            // unoccupied edit-mode lots enable budget mode (global 32 = 1 ->
            // §999,999) and enter Buy mode out of Live mode.
            game.lot.set_edit_budget(true);
            game.set_mode(Mode::Buy);
        }
        Ok(game)
    }

    /// Boots `house` from a REDB [`sims_sim::neighborhood_store::NeighborhoodStore`]
    /// via [`sims_vm::lifecycle::enter_lot`], without reading or mutating any
    /// `UserData` house files on disk.
    #[allow(clippy::too_many_arguments)]
    pub fn open_from_store(
        install_root: &Path,
        store: &sims_sim::neighborhood_store::NeighborhoodStore,
        neighborhood_id: &str,
        house: u32,
        answers: &[u8],
        play: bool,
        frames: Frames,
    ) -> Result<Self, String> {
        Self::open_from_store_cached(
            install_root,
            store,
            neighborhood_id,
            house,
            answers,
            play,
            frames,
            None,
        )
    }

    /// Like [`Game::open_from_store`], reusing preloaded install-wide
    /// [`InstallAssets`] across lot transitions when `cached` is `Some`.
    #[allow(clippy::too_many_arguments)]
    pub fn open_from_store_cached(
        install_root: &Path,
        store: &sims_sim::neighborhood_store::NeighborhoodStore,
        neighborhood_id: &str,
        house: u32,
        answers: &[u8],
        play: bool,
        frames: Frames,
        cached: Option<&Arc<InstallAssets>>,
    ) -> Result<Self, String> {
        let snapshot = store
            .load_lot_snapshot(neighborhood_id, house)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {neighborhood_id:?} not found in store"))?;
        let members = snapshot
            .neighborhood
            .family_in(house)
            .map(|(_, f)| f.members.clone())
            .unwrap_or_default();
        let pool = cached.map_or_else(|| Arc::new(ArchivePool::new()), |c| Arc::clone(&c.pool));
        let character_files = &snapshot.character_files;
        let (lot_res, render_and_index_res, hud_static_res, people_assets_res) =
            std::thread::scope(|scope| {
                let assets_handle = scope.spawn(|| -> Result<_, String> {
                    if let Some(shared) = cached {
                        let (idx_cat_res, base_res) = std::thread::scope(|inner| {
                            let base_handle = inner.spawn(|| {
                                BaseAssets::load(&shared.install).map_err(|e| e.to_string())
                            });
                            let idx_cat_res = if character_files.is_empty() {
                                Ok((
                                    Arc::clone(&shared.base_index),
                                    Arc::clone(&shared.base_catalog),
                                ))
                            } else {
                                (|| -> Result<_, String> {
                                    let mut index = (*shared.base_index).clone();
                                    index
                                        .add_character_files(character_files)
                                        .map_err(|e| e.to_string())?;
                                    let catalog = Catalog::build_with_pool(&index, &shared.pool)
                                        .map_err(|e| e.to_string())?;
                                    Ok((Arc::new(index), Arc::new(catalog)))
                                })()
                            };
                            (
                                idx_cat_res,
                                base_handle.join().expect("base assets worker panicked"),
                            )
                        });
                        let (index, catalog) = idx_cat_res?;
                        let base = base_res?;
                        return Ok((
                            shared.install.clone(),
                            index,
                            catalog,
                            Ok(shared.hud_shared.clone()),
                            base,
                            shared.floor_plugins.clone(),
                            shared.wall_plugins.clone(),
                        ));
                    }
                    let install = Install::detect(install_root).map_err(|e| e.to_string())?;
                    let (idx_cat_res, base_res, (floor_plugins, wall_plugins)) =
                        std::thread::scope(|inner| {
                            let plugins_handle =
                                inner.spawn(|| Surfaces::plugins_with_pool(&install, &pool));
                            let base_handle = inner
                                .spawn(|| BaseAssets::load(&install).map_err(|e| e.to_string()));
                            let idx_cat_res = (|| -> Result<_, String> {
                                let mut index =
                                    Index::build(&install).map_err(|e| e.to_string())?;
                                if !character_files.is_empty() {
                                    index
                                        .add_character_files(character_files)
                                        .map_err(|e| e.to_string())?;
                                }
                                let catalog = Catalog::build_with_pool(&index, &pool)
                                    .map_err(|e| e.to_string())?;
                                let index = Arc::new(index);
                                let catalog = Arc::new(catalog);
                                let hud_shared_res = HudShared::load(&index, &catalog, &pool);
                                Ok((index, catalog, hud_shared_res))
                            })();
                            (
                                idx_cat_res,
                                base_handle.join().expect("base assets worker panicked"),
                                plugins_handle
                                    .join()
                                    .expect("surface plugins worker panicked"),
                            )
                        });
                    let (index, catalog, hud_shared_res) = idx_cat_res?;
                    let base = base_res?;
                    Ok((
                        install,
                        index,
                        catalog,
                        hud_shared_res,
                        base,
                        floor_plugins,
                        wall_plugins,
                    ))
                });
                let hud_handle = scope.spawn(|| -> Result<HudStatic, String> {
                    if let Some(shared) = cached {
                        return Ok(shared.hud_static.clone());
                    }
                    let install = Install::detect(install_root).map_err(|e| e.to_string())?;
                    HudStatic::load(&install, Arc::clone(&pool))
                });
                let people_handle = scope.spawn(|| -> Result<_, String> {
                    if let Some(shared) = cached {
                        return Ok((shared.content_assets.clone(), shared.people_library.clone()));
                    }
                    let content_assets = ContentAssets::scan(install_root)?;
                    let people_library = PeopleLibrary::scan_with_pool(&content_assets, &pool)?;
                    Ok((content_assets, people_library))
                });
                let lot_res = match cached {
                    Some(shared) => sims_vm::lifecycle::enter_lot_with_cache(
                        &shared.vm_cache,
                        store,
                        neighborhood_id,
                        house,
                        answers,
                        frames.clone(),
                    ),
                    None => sims_vm::lifecycle::enter_lot(
                        install_root,
                        store,
                        neighborhood_id,
                        house,
                        answers,
                        frames.clone(),
                    ),
                }
                .map_err(|e| format!("{neighborhood_id} House{house:02}: {e}"));
                (
                    lot_res,
                    assets_handle.join().expect("store assets worker panicked"),
                    hud_handle.join().expect("hud static worker panicked"),
                    people_handle.join().expect("people assets worker panicked"),
                )
            });
        let mut lot = lot_res?;
        lot.wait_for_answers();
        let stopped = lot
            .load_report()
            .stopped
            .as_ref()
            .map(|s| format!("The load stopped, so the lot cannot run: {s}"));
        let (install, index, catalog, hud_shared_res, base_assets, floor_plugins, wall_plugins) =
            render_and_index_res?;
        let assets = sims_render::Assets::from_base(
            base_assets,
            floor_plugins,
            wall_plugins,
            Arc::clone(&index),
            Arc::clone(&catalog),
            Arc::clone(&pool),
        );
        let lot_shape = sims_render::Lot::from_world(lot.world())
            .map_err(|e| e.to_string())?
            .with_floor_rotation(lot.floor_rotation());
        let friendship = lot.program().autonomy_constants().friendship_threshold;
        let world = lot.world();
        let (hud_res, people_open_res) = std::thread::scope(|scope| {
            let people_handle = scope.spawn(|| -> Result<People, String> {
                let (content_assets, people_library) = people_assets_res?;
                let preloaded_cast = cached.and_then(|shared| {
                    shared.preloaded_cast_for(&index, &catalog, !character_files.is_empty())
                });
                let cast = Cast::from_shared(
                    install.clone(),
                    Arc::clone(&index),
                    Arc::clone(&catalog),
                    preloaded_cast,
                    Arc::clone(&pool),
                )?;
                let mut people = People::from_inner(sims_people::People::new(
                    content_assets,
                    people_library,
                    cast,
                ));
                people.refresh(world);
                Ok(people)
            });
            let hud_res = hud_static_res.and_then(|hud_static| {
                Hud::from_preloaded_with_members(
                    &install,
                    members,
                    &assets,
                    world,
                    friendship,
                    hud_static,
                    hud_shared_res,
                    &pool,
                )
            });
            (
                hud_res,
                people_handle
                    .join()
                    .expect("people refresh worker panicked"),
            )
        });
        let mut hud = hud_res?;
        hud.sync_clock_blink(lot.world(), 0);
        let free_will = hud.controls.options.play.free_will;
        if lot.world().globals.get(30).copied().unwrap_or(1) != i16::from(free_will) {
            let _ = lot.set_free_will(free_will);
        }
        let people = people_open_res?;
        let view = View::new([800, 600], lot_shape.built_center(), lot.view_rotation());
        let drawn_camera = view.camera;
        let has_family = lot.world().globals.get(9).copied().unwrap_or(0) != 0;
        let is_residential = lot.lot_zoning().unwrap_or(1) == 1
            && !matches!(lot.lot_id(), 21..=30 | 40..=48 | 81..=89 | 93..=98);
        let edit_mode = if has_family && is_residential {
            false
        } else {
            lot.world().globals.get(32).copied().unwrap_or(0) == 1
                || (!has_family
                    && (lot.lot_zoning().unwrap_or(0) != 0
                        || matches!(lot.lot_id(), 21..=30 | 40..=48 | 81..=99)))
        };
        let mut pace = Pace::default();
        if let Some(speed) = pace::shown(lot.world()) {
            pace.speed = speed;
        }
        let base_roof = snapshot
            .base_house
            .as_deref()
            .and_then(parse_base_roof_from_house_bytes)
            .unwrap_or_default();
        let mut game = Self {
            lot,
            lot_shape,
            assets,
            view,
            people,
            hud,
            pace,
            play,
            stopped,
            errors: Errors::default(),
            sounds: VecDeque::new(),
            ui_effects: VecDeque::new(),
            highlighted_buttons: BTreeSet::new(),
            highlighted_portraits: BTreeSet::new(),
            highlighted_relationships: BTreeSet::new(),
            pointer: None,
            reached: None,
            listed: None,
            highlighted: None,
            buy_selected: None,
            buy_cursor: None,
            buy_in_house: false,
            buy_last_guid: None,
            buy_facing: None,
            buy_dragging: None,
            buy_grab_offset: [0, 0],
            buy_commit_on_release: false,
            live_panel_expanded: true,
            shift_held: false,
            ctrl_held: false,
            wall_cursor: None,
            build_drag: None,
            build_preview_shape: None,
            base_roof,
            roof_catalog: None,
            roof_view: false,
            terrain_hover_overlay: None,
            water_hover_overlay: None,
            shop_dialog: ShopPickerDialog::new(),
            shop_art: None,
            shop_strings: None,
            shop_snapshot: None,
            shop_preview_assets: None,
            shop_preview_yaw_deg: 0,
            shop_preview_body: None,
            shop_preview_targets: BTreeMap::new(),
            purchased_shop_items: BTreeSet::new(),
            book_dialog: ReferenceBookDialog::new(),
            book_art: None,
            book_strings: None,
            book_snapshot: None,
            phonebook_dialog: PhoneBookDialog::new(),
            phonebook_art: None,
            phonebook_strings: None,
            phonebook_snapshot: None,
            destination_picker: None,
            destination_picker_assets: None,
            neighborhood_slot: 1,
            active_special_panel: None,
            submitted_coop_special_panel: None,
            ordered_wardrobe: None,
            pending_travel_lot: None,
            special_lot_catalog: Vec::new(),
            ui_animating_prev: false,
            voice_inputs: VecDeque::new(),
            buy_moneyback_us: None,
            buy_delete_us: None,
            note: None,
            clock_us: 0,
            drawn_us: 0,
            blink_dirty: false,
            dirty: false,
            restore_dirty_on_quit_cancel: false,
            pending_after_save_note: None,
            pending_after_notice: None,
            edit_mode,
            edge_scroll_rem: [0, 0],
            canvas: Canvas::new(1, 1, [0, 0, 0]),
            lot_cache: sims_render::LotCache::new(),
            drawn_camera,
            lot_depth: Vec::new(),
            lot_depth_from_cache: false,
            people_ids: sims_render::Ids::default(),
            people_rects: Vec::new(),
            prev_dynamic_rects: Vec::new(),
            prev_steady_key: None,
            present_damage: None,
            pie_animations: false,
            debug: false,
            last_debug_logged_note: None,
            smooth_camera_pan: false,
            camera_pan: None,
            camera_pan_moved: false,
            cheat_map_edit: false,
            cheat_autonomy: None,
            cheat_sim_speed: None,
            world_mutated_via_restore: false,
            restore_generation: 0,
            frames,
            coop: CoopController::new(),
            coop_lobby: CoopLobbyDialog::new(),
            coop_art: None,
            coop_player_name: default_player_name("Player"),
            coop_iroh_config: IrohConfig::P2pDefault,
            coop_travel_blocked: false,
            coop_remote_world: false,
            coop_overlay_buttons: CoopOverlayButtons::default(),
            coop_buy_preview: None,
            world_sound_rev: 0,
            open_pack_mode: false,
            _open_pack_staging: None,
            open_pack_wavs: None,
            open_pack_save_slot: None,
        };
        game.sync_view();
        if edit_mode && game.stopped.is_none() {
            game.lot.set_edit_budget(true);
            game.set_mode(Mode::Buy);
        }
        Ok(game)
    }

    /// Wraps an already-booted [`Lot`] (e.g. from `sims_vm::travel::depart_to_destination`,
    /// `resume_active_trip`, `return_from_destination`, or `open_destination_for_edit`)
    /// into a live [`Game`] session without re-booting the VM.
    pub fn from_loaded_lot(
        install_root: &Path,
        lot: Lot,
        members: Vec<u32>,
        character_files: &BTreeMap<String, Vec<u8>>,
        base_house_bytes: Option<&[u8]>,
        play: bool,
    ) -> Result<Self, String> {
        Self::from_loaded_lot_cached(
            install_root,
            lot,
            members,
            character_files,
            base_house_bytes,
            play,
            None,
        )
    }

    /// Like [`Game::from_loaded_lot`], reusing preloaded install-wide
    /// [`InstallAssets`] when `cached` is `Some`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_loaded_lot_cached(
        install_root: &Path,
        mut lot: Lot,
        members: Vec<u32>,
        character_files: &BTreeMap<String, Vec<u8>>,
        base_house_bytes: Option<&[u8]>,
        play: bool,
        cached: Option<&Arc<InstallAssets>>,
    ) -> Result<Self, String> {
        lot.wait_for_answers();
        let stopped = lot
            .load_report()
            .stopped
            .as_ref()
            .map(|s| format!("The load stopped, so the lot cannot run: {s}"));
        let pool = cached.map_or_else(|| Arc::new(ArchivePool::new()), |c| Arc::clone(&c.pool));
        let (render_and_index_res, hud_static_res, people_assets_res) =
            std::thread::scope(|scope| {
                let assets_handle = scope.spawn(|| -> Result<_, String> {
                    if let Some(shared) = cached {
                        let (idx_cat_res, base_res) = std::thread::scope(|inner| {
                            let base_handle = inner.spawn(|| {
                                BaseAssets::load(&shared.install).map_err(|e| e.to_string())
                            });
                            let idx_cat_res = if character_files.is_empty() {
                                Ok((
                                    Arc::clone(&shared.base_index),
                                    Arc::clone(&shared.base_catalog),
                                ))
                            } else {
                                (|| -> Result<_, String> {
                                    let mut index = (*shared.base_index).clone();
                                    index
                                        .add_character_files(character_files)
                                        .map_err(|e| e.to_string())?;
                                    let catalog = Catalog::build_with_pool(&index, &shared.pool)
                                        .map_err(|e| e.to_string())?;
                                    Ok((Arc::new(index), Arc::new(catalog)))
                                })()
                            };
                            (
                                idx_cat_res,
                                base_handle.join().expect("base assets worker panicked"),
                            )
                        });
                        let (index, catalog) = idx_cat_res?;
                        let base = base_res?;
                        return Ok((
                            shared.install.clone(),
                            index,
                            catalog,
                            Ok(shared.hud_shared.clone()),
                            base,
                            shared.floor_plugins.clone(),
                            shared.wall_plugins.clone(),
                        ));
                    }
                    let install = Install::detect(install_root).map_err(|e| e.to_string())?;
                    let (idx_cat_res, base_res, (floor_plugins, wall_plugins)) =
                        std::thread::scope(|inner| {
                            let plugins_handle =
                                inner.spawn(|| Surfaces::plugins_with_pool(&install, &pool));
                            let base_handle = inner
                                .spawn(|| BaseAssets::load(&install).map_err(|e| e.to_string()));
                            let idx_cat_res = (|| -> Result<_, String> {
                                let mut index =
                                    Index::build(&install).map_err(|e| e.to_string())?;
                                if !character_files.is_empty() {
                                    index
                                        .add_character_files(character_files)
                                        .map_err(|e| e.to_string())?;
                                }
                                let catalog = Catalog::build_with_pool(&index, &pool)
                                    .map_err(|e| e.to_string())?;
                                let index = Arc::new(index);
                                let catalog = Arc::new(catalog);
                                let hud_shared_res = HudShared::load(&index, &catalog, &pool);
                                Ok((index, catalog, hud_shared_res))
                            })();
                            (
                                idx_cat_res,
                                base_handle.join().expect("base assets worker panicked"),
                                plugins_handle
                                    .join()
                                    .expect("surface plugins worker panicked"),
                            )
                        });
                    let (index, catalog, hud_shared_res) = idx_cat_res?;
                    let base = base_res?;
                    Ok((
                        install,
                        index,
                        catalog,
                        hud_shared_res,
                        base,
                        floor_plugins,
                        wall_plugins,
                    ))
                });
                let hud_handle = scope.spawn(|| -> Result<HudStatic, String> {
                    if let Some(shared) = cached {
                        return Ok(shared.hud_static.clone());
                    }
                    let install = Install::detect(install_root).map_err(|e| e.to_string())?;
                    HudStatic::load(&install, Arc::clone(&pool))
                });
                let people_handle = scope.spawn(|| -> Result<_, String> {
                    if let Some(shared) = cached {
                        return Ok((shared.content_assets.clone(), shared.people_library.clone()));
                    }
                    let content_assets = ContentAssets::scan(install_root)?;
                    let people_library = PeopleLibrary::scan_with_pool(&content_assets, &pool)?;
                    Ok((content_assets, people_library))
                });
                (
                    assets_handle
                        .join()
                        .expect("loaded lot assets worker panicked"),
                    hud_handle.join().expect("hud static worker panicked"),
                    people_handle.join().expect("people assets worker panicked"),
                )
            });
        let (install, index, catalog, hud_shared_res, base_assets, floor_plugins, wall_plugins) =
            render_and_index_res?;
        let assets = sims_render::Assets::from_base(
            base_assets,
            floor_plugins,
            wall_plugins,
            Arc::clone(&index),
            Arc::clone(&catalog),
            Arc::clone(&pool),
        );
        let lot_shape = sims_render::Lot::from_world(lot.world())
            .map_err(|e| e.to_string())?
            .with_floor_rotation(lot.floor_rotation());
        let friendship = lot.program().autonomy_constants().friendship_threshold;
        let world = lot.world();
        let (hud_res, people_open_res) = std::thread::scope(|scope| {
            let people_handle = scope.spawn(|| -> Result<People, String> {
                let (content_assets, people_library) = people_assets_res?;
                let preloaded_cast = cached.and_then(|shared| {
                    shared.preloaded_cast_for(&index, &catalog, !character_files.is_empty())
                });
                let cast = Cast::from_shared(
                    install.clone(),
                    Arc::clone(&index),
                    Arc::clone(&catalog),
                    preloaded_cast,
                    Arc::clone(&pool),
                )?;
                let mut people = People::from_inner(sims_people::People::new(
                    content_assets,
                    people_library,
                    cast,
                ));
                people.refresh(world);
                Ok(people)
            });
            let hud_res = hud_static_res.and_then(|hud_static| {
                Hud::from_preloaded_with_members(
                    &install,
                    members,
                    &assets,
                    world,
                    friendship,
                    hud_static,
                    hud_shared_res,
                    &pool,
                )
            });
            (
                hud_res,
                people_handle
                    .join()
                    .expect("people refresh worker panicked"),
            )
        });
        let mut hud = hud_res?;
        hud.sync_clock_blink(lot.world(), 0);
        let free_will = hud.controls.options.play.free_will;
        if lot.world().globals.get(30).copied().unwrap_or(1) != i16::from(free_will) {
            let _ = lot.set_free_will(free_will);
        }
        let people = people_open_res?;
        let view = View::new([800, 600], lot_shape.built_center(), lot.view_rotation());
        let drawn_camera = view.camera;
        let has_family = lot.world().globals.get(9).copied().unwrap_or(0) != 0;
        let edit_mode = lot.world().globals.get(32).copied().unwrap_or(0) == 1
            || (!has_family
                && (lot.lot_zoning().unwrap_or(0) != 0
                    || matches!(lot.lot_id(), 21..=30 | 40..=48 | 81..=99)));
        let mut pace = Pace::default();
        if let Some(speed) = pace::shown(lot.world()) {
            pace.speed = speed;
        }
        let base_roof = base_house_bytes
            .and_then(parse_base_roof_from_house_bytes)
            .unwrap_or_default();
        let mut game = Self {
            lot,
            lot_shape,
            assets,
            view,
            people,
            hud,
            pace,
            play,
            stopped,
            errors: Errors::default(),
            sounds: VecDeque::new(),
            ui_effects: VecDeque::new(),
            highlighted_buttons: BTreeSet::new(),
            highlighted_portraits: BTreeSet::new(),
            highlighted_relationships: BTreeSet::new(),
            pointer: None,
            reached: None,
            listed: None,
            highlighted: None,
            buy_selected: None,
            buy_cursor: None,
            buy_in_house: false,
            buy_last_guid: None,
            buy_facing: None,
            buy_dragging: None,
            buy_grab_offset: [0, 0],
            buy_commit_on_release: false,
            live_panel_expanded: true,
            shift_held: false,
            ctrl_held: false,
            wall_cursor: None,
            build_drag: None,
            build_preview_shape: None,
            base_roof,
            roof_catalog: None,
            roof_view: false,
            terrain_hover_overlay: None,
            water_hover_overlay: None,
            shop_dialog: ShopPickerDialog::new(),
            shop_art: None,
            shop_strings: None,
            shop_snapshot: None,
            shop_preview_assets: None,
            shop_preview_yaw_deg: 0,
            shop_preview_body: None,
            shop_preview_targets: BTreeMap::new(),
            purchased_shop_items: BTreeSet::new(),
            book_dialog: ReferenceBookDialog::new(),
            book_art: None,
            book_strings: None,
            book_snapshot: None,
            phonebook_dialog: PhoneBookDialog::new(),
            phonebook_art: None,
            phonebook_strings: None,
            phonebook_snapshot: None,
            destination_picker: None,
            destination_picker_assets: None,
            neighborhood_slot: 1,
            active_special_panel: None,
            submitted_coop_special_panel: None,
            ordered_wardrobe: None,
            pending_travel_lot: None,
            special_lot_catalog: Vec::new(),
            ui_animating_prev: false,
            voice_inputs: VecDeque::new(),
            buy_moneyback_us: None,
            buy_delete_us: None,
            note: None,
            clock_us: 0,
            drawn_us: 0,
            blink_dirty: false,
            dirty: false,
            restore_dirty_on_quit_cancel: false,
            pending_after_save_note: None,
            pending_after_notice: None,
            edit_mode,
            edge_scroll_rem: [0, 0],
            canvas: Canvas::new(1, 1, [0, 0, 0]),
            lot_cache: sims_render::LotCache::new(),
            drawn_camera,
            lot_depth: Vec::new(),
            lot_depth_from_cache: false,
            people_ids: sims_render::Ids::default(),
            people_rects: Vec::new(),
            prev_dynamic_rects: Vec::new(),
            prev_steady_key: None,
            present_damage: None,
            pie_animations: false,
            debug: false,
            last_debug_logged_note: None,
            smooth_camera_pan: false,
            camera_pan: None,
            camera_pan_moved: false,
            cheat_map_edit: false,
            cheat_autonomy: None,
            cheat_sim_speed: None,
            world_mutated_via_restore: false,
            restore_generation: 0,
            frames: Frames::default(),
            coop: CoopController::new(),
            coop_lobby: CoopLobbyDialog::new(),
            coop_art: None,
            coop_player_name: default_player_name("Player"),
            coop_iroh_config: IrohConfig::P2pDefault,
            coop_travel_blocked: false,
            coop_remote_world: false,
            coop_overlay_buttons: CoopOverlayButtons::default(),
            coop_buy_preview: None,
            world_sound_rev: 0,
            open_pack_mode: false,
            _open_pack_staging: None,
            open_pack_wavs: None,
            open_pack_save_slot: None,
        };
        game.sync_view();
        if edit_mode && game.stopped.is_none() {
            game.lot.set_edit_budget(true);
            game.set_mode(Mode::Buy);
        }
        Ok(game)
    }

    /// Compiles and boots a clean-room standalone `open-pack/` directory with
    /// zero commercial EA/Maxis assets required.
    ///
    /// Uses `sims_antics::compile_standalone_pack_dir` to build all 20 objects,
    /// 4 `.flr` floors, 4 `.wll` walls, `House01.iff`, `Neighborhood.iff`,
    /// starter Sims, and synthesized PCM WAV bank, paired with
    /// [`sims_ui::art::Art::clean_room_procedural`] and 2D clean-room Sim
    /// sprite rendering.
    pub fn open_open_pack(
        pack_dir: &Path,
        answers: &[u8],
        play: bool,
        frames: Frames,
    ) -> Result<Self, String> {
        let staged = crate::open_pack::stage_open_pack(pack_dir)?;
        let install_root = staged.staging_dir.path();
        let house = &staged.house_path;
        let neighborhood_dir = house
            .parent()
            .and_then(Path::parent)
            .ok_or("staged open-pack house is not in a Houses folder")?;
        let install = Install::detect(install_root).map_err(|e| e.to_string())?;
        let neighborhood = Neighborhood::open(neighborhood_dir).map_err(|e| e.to_string())?;
        let pool = Arc::new(ArchivePool::new());
        let mut index = Index::build(&install).map_err(|e| e.to_string())?;
        if let Some(characters) = neighborhood.characters().map_err(|e| e.to_string())? {
            index
                .add_characters(&characters)
                .map_err(|e| e.to_string())?;
        }
        let catalog = Arc::new(Catalog::build_with_pool(&index, &pool).map_err(|e| e.to_string())?);
        let index = Arc::new(index);
        let base_assets = BaseAssets::load(&install).map_err(|e| e.to_string())?;
        let (floor_plugins, wall_plugins) = Surfaces::plugins_with_pool(&install, &pool);

        let mut lot = Lot::load_open_pack(install_root, house, answers, frames.clone())
            .map_err(|e| format!("{}: {e}", house.display()))?;
        lot.wait_for_answers();
        let stopped = lot
            .load_report()
            .stopped
            .as_ref()
            .map(|s| format!("The load stopped, so the lot cannot run: {s}"));

        let assets = sims_render::Assets::from_base(
            base_assets,
            floor_plugins,
            wall_plugins,
            Arc::clone(&index),
            Arc::clone(&catalog),
            Arc::clone(&pool),
        );
        let lot_shape = sims_render::Lot::from_world(lot.world())
            .map_err(|e| e.to_string())?
            .with_floor_rotation(lot.floor_rotation());
        let friendship = lot.program().autonomy_constants().friendship_threshold;
        let members: Vec<u32> = staged.starter_sims.iter().map(|s| s.guid).collect();
        let mut hud = Hud::open_open_pack(
            &install,
            members,
            &staged.starter_sims,
            &assets,
            lot.world(),
            friendship,
            &pool,
        )?;
        hud.sync_clock_blink(lot.world(), 0);
        let content_assets = ContentAssets::scan(install_root)?;
        let people_library = PeopleLibrary::scan_with_pool(&content_assets, &pool)?;
        let cast = Cast::from_shared(
            install,
            Arc::clone(&index),
            Arc::clone(&catalog),
            None,
            Arc::clone(&pool),
        )?;
        let people = People::from_inner(sims_people::People::new(
            content_assets,
            people_library,
            cast,
        ));
        let view = View::new([800, 600], lot_shape.built_center(), lot.view_rotation());
        let drawn_camera = view.camera;

        let mut game = Self {
            lot,
            lot_shape,
            assets,
            view,
            people,
            hud,
            pace: Pace::default(),
            play,
            stopped,
            errors: Errors::default(),
            sounds: VecDeque::new(),
            ui_effects: VecDeque::new(),
            highlighted_buttons: BTreeSet::new(),
            highlighted_portraits: BTreeSet::new(),
            highlighted_relationships: BTreeSet::new(),
            pointer: None,
            reached: None,
            listed: None,
            highlighted: None,
            buy_selected: None,
            buy_cursor: None,
            buy_in_house: false,
            buy_last_guid: None,
            buy_facing: None,
            buy_dragging: None,
            buy_grab_offset: [0, 0],
            buy_commit_on_release: false,
            live_panel_expanded: true,
            shift_held: false,
            ctrl_held: false,
            wall_cursor: None,
            build_drag: None,
            build_preview_shape: None,
            base_roof: RoofState::default(),
            roof_catalog: None,
            roof_view: false,
            terrain_hover_overlay: None,
            water_hover_overlay: None,
            shop_dialog: ShopPickerDialog::new(),
            shop_art: None,
            shop_strings: None,
            shop_snapshot: None,
            shop_preview_assets: None,
            shop_preview_yaw_deg: 0,
            shop_preview_body: None,
            shop_preview_targets: BTreeMap::new(),
            purchased_shop_items: BTreeSet::new(),
            book_dialog: ReferenceBookDialog::new(),
            book_art: None,
            book_strings: None,
            book_snapshot: None,
            phonebook_dialog: PhoneBookDialog::new(),
            phonebook_art: None,
            phonebook_strings: None,
            phonebook_snapshot: None,
            destination_picker: None,
            destination_picker_assets: None,
            neighborhood_slot: 1,
            active_special_panel: None,
            submitted_coop_special_panel: None,
            ordered_wardrobe: None,
            pending_travel_lot: None,
            special_lot_catalog: Vec::new(),
            ui_animating_prev: false,
            voice_inputs: VecDeque::new(),
            buy_moneyback_us: None,
            buy_delete_us: None,
            note: None,
            clock_us: 0,
            drawn_us: 0,
            blink_dirty: false,
            dirty: false,
            restore_dirty_on_quit_cancel: false,
            pending_after_save_note: None,
            pending_after_notice: None,
            edit_mode: false,
            edge_scroll_rem: [0, 0],
            canvas: Canvas::new(1, 1, [0, 0, 0]),
            lot_cache: sims_render::LotCache::new(),
            drawn_camera,
            lot_depth: Vec::new(),
            lot_depth_from_cache: false,
            people_ids: sims_render::Ids::default(),
            people_rects: Vec::new(),
            prev_dynamic_rects: Vec::new(),
            prev_steady_key: None,
            present_damage: None,
            pie_animations: false,
            debug: false,
            last_debug_logged_note: None,
            smooth_camera_pan: false,
            camera_pan: None,
            camera_pan_moved: false,
            cheat_map_edit: false,
            cheat_autonomy: None,
            cheat_sim_speed: None,
            world_mutated_via_restore: false,
            restore_generation: 0,
            frames,
            coop: CoopController::new(),
            coop_lobby: CoopLobbyDialog::new(),
            coop_art: None,
            coop_player_name: default_player_name("Player"),
            coop_iroh_config: IrohConfig::P2pDefault,
            coop_travel_blocked: false,
            coop_remote_world: false,
            coop_overlay_buttons: CoopOverlayButtons::default(),
            coop_buy_preview: None,
            world_sound_rev: 0,
            open_pack_mode: true,
            _open_pack_staging: Some(staged.staging_dir),
            open_pack_wavs: Some(staged.sounds_wav),
            open_pack_save_slot: Some(staged.save_slot),
        };
        game.sync_view();
        Ok(game)
    }

    /// Whether this game is running in standalone clean-room `open-pack/` mode.
    pub fn is_open_pack(&self) -> bool {
        self.open_pack_mode
    }

    /// Stable save slot key for standalone clean-room `open-pack/` mode,
    /// derived from the source pack directory and lot manifest.
    pub fn open_pack_save_slot(&self) -> Option<&str> {
        self.open_pack_save_slot.as_deref()
    }

    /// Synthesized PCM WAV sound bank loaded from `open-pack/`, when running
    /// in standalone clean-room mode.
    pub fn open_pack_sounds(&self) -> Option<&BTreeMap<String, Vec<u8>>> {
        self.open_pack_wavs.as_ref()
    }

    /// Saves the active lot back to its REDB [`sims_sim::neighborhood_store::NeighborhoodStore`]
    /// via [`sims_vm::lifecycle::save_and_leave_lot`] (stripping any unpaid
    /// Buy Mode placement cursor first).
    pub fn save_to_neighborhood(
        &mut self,
        install_root: &Path,
        store: &sims_sim::neighborhood_store::NeighborhoodStore,
        neighborhood_id: &str,
    ) -> Result<sims_vm::lifecycle::SaveReport, String> {
        if self.coop_blocks_local_save() {
            let msg = "Only the co-op host can save a local lot (remote guest lots are read-only).";
            self.note = Some(msg.into());
            return Err(msg.into());
        }
        let result = match self.buy_cursor.as_ref() {
            Some(cursor) => sims_vm::lifecycle::save_and_leave_lot_without_buy_cursor(
                install_root,
                store,
                neighborhood_id,
                &mut self.lot,
                cursor,
            ),
            None => sims_vm::lifecycle::save_and_leave_lot(
                install_root,
                store,
                neighborhood_id,
                &mut self.lot,
            ),
        }
        .map_err(|e| e.to_string());
        let followup = self.pending_after_save_note.take();
        match &result {
            Ok(_) => {
                self.dirty = false;
                self.note = Some(match followup {
                    Some(extra) => format!("Saved after tick {}. {extra}", self.lot.ticks()),
                    None => format!("Saved after tick {}.", self.lot.ticks()),
                });
            }
            Err(e) => {
                self.note = Some(format!("The lot was not saved: {e}"));
            }
        }
        result
    }

    /// Whether the player is currently holding an interactive tool, drag,
    /// slider, or modal dialog that benefits from display-refresh redraws.
    pub fn is_actively_interacting(&self) -> bool {
        self.buy_cursor.is_some()
            || self.coop_buy_preview.is_some()
            || self.build_drag.is_some()
            || self.wall_cursor.is_some()
            || self.shop_dialog.is_open()
            || self.phonebook_dialog.is_open()
            || self.coop_lobby.is_open()
            || self.hud.dragging_slider()
            || self.hud.dragging_dialog_entry()
            || self.has_dialog()
            || self.pie_animating()
            || self.camera_panning()
            || self.ui_animating()
            || self.cheat_console_open()
    }

    /// Whether any temporal UI animation (250 ms help-balloon fade, modal
    /// dialog fade-in, or 500 ms action-queue icon slide) is currently in flight.
    /// Always `false` when `clock_us == 0` (static/headless frames).
    pub fn ui_animating(&self) -> bool {
        self.hud.is_ui_animating(self.clock_us)
    }

    /// Current Live help-balloon fade state (`BalloonFadeState`).
    pub fn balloon_fade(&self) -> &BalloonFadeState {
        self.hud.balloon_fade()
    }

    /// Current modal dialog fade state (`DialogFadeState`).
    pub fn dialog_fade(&self) -> &DialogFadeState {
        self.hud.dialog_fade()
    }

    /// Current action-queue icon slide state (`QueueSlideState`).
    pub fn queue_slide(&self) -> &QueueSlideState {
        self.hud.queue_slide()
    }

    /// Enables or disables smooth camera pan interpolation on portrait
    /// double-click / re-click (also enabled automatically once `clock_us > 0`).
    pub fn set_smooth_camera_pan(&mut self, enabled: bool) {
        self.smooth_camera_pan = enabled;
        if !enabled {
            self.camera_pan = None;
        }
    }

    fn smooth_pan_active(&self) -> bool {
        self.smooth_camera_pan || self.clock_us > 0
    }

    /// Whether a smooth camera pan interpolation is currently in flight.
    pub fn camera_panning(&self) -> bool {
        self.camera_pan.is_some()
    }

    /// Returns and clears whether the camera origin moved during the last
    /// smooth camera pan step.
    pub fn take_camera_pan_moved(&mut self) -> bool {
        std::mem::take(&mut self.camera_pan_moved)
    }

    fn step_camera_pan(&mut self) -> bool {
        let Some(pan) = self.camera_pan else {
            return false;
        };
        let (origin, finished) = pan.sample(self.clock_us);
        let moved = self.view.set_origin(origin);
        if finished {
            self.camera_pan = None;
            self.sync_view();
        }
        if moved {
            self.camera_pan_moved = true;
        }
        moved || finished
    }

    /// Enables or disables 400 ms pie-menu pop-in / submenu glide / direction
    /// ease-out animations explicitly (also enabled automatically once `clock_us > 0`).
    pub fn set_pie_animations(&mut self, enabled: bool) {
        self.pie_animations = enabled;
    }

    fn pie_animations_active(&self) -> bool {
        self.pie_animations || self.clock_us > 0
    }

    /// Whether an open pie menu currently has an active 400 ms animation
    /// (backdrop pop-in, submenu glide, or head direction ease-out).
    pub fn pie_animating(&self) -> bool {
        let now_ms = (self.clock_us / 1_000) as u32;
        self.hud
            .menu
            .as_ref()
            .is_some_and(|m| m.is_animating(now_ms))
    }

    /// Whether the lot's live `World` was replaced via `Lot::restore` (e.g.
    /// Save Manager load or world-mutating cheat), clearing `neighborhood_origin`.
    pub fn world_mutated_via_restore(&self) -> bool {
        self.world_mutated_via_restore
    }

    /// Monotonic counter bumped on each `Game::restore` that resets `errors()`.
    pub fn restore_generation(&self) -> u64 {
        self.restore_generation
    }

    /// Marks the lot clean after an external host/REDB save succeeds.
    pub fn mark_saved_clean(&mut self, note: impl Into<String>) {
        self.dirty = false;
        self.note = Some(note.into());
    }

    /// Whether `move_objects on` is currently enabled on the live lot.
    pub fn cheat_move_objects(&self) -> bool {
        self.lot.move_objects()
    }

    /// Whether `map_edit on` is currently enabled via the cheat console.
    pub fn cheat_map_edit(&self) -> bool {
        self.cheat_map_edit
    }

    /// Last `autonomy <0..=100>` level applied via the cheat console, if any.
    pub fn cheat_autonomy(&self) -> Option<u8> {
        self.cheat_autonomy
    }

    /// Last `sim_speed <-1000..=1000>` value applied via the cheat console, if any.
    pub fn cheat_sim_speed(&self) -> Option<i16> {
        self.cheat_sim_speed
    }

    /// Screen rectangle of the Options-strip "Saves" button when visible.
    pub fn options_saves_button_rect(&self) -> Option<sims_ui::frame::Rect> {
        self.hud.options_saves_button_rect()
    }

    /// Screen rectangle of the Options-strip "Co-op" button when visible.
    pub fn options_coop_button_rect(&self) -> Option<sims_ui::frame::Rect> {
        self.hud.options_coop_button_rect()
    }

    /// Read-only access to the classic top-left cheat console (`Ctrl+Shift+C`).
    pub fn cheat_console(&self) -> &sims_ui::cheat_console::CheatConsole {
        &self.hud.cheat_console
    }

    /// Whether the classic cheat console overlay is currently open.
    pub fn cheat_console_open(&self) -> bool {
        self.hud.cheat_console.is_open()
    }

    /// Toggles the classic cheat console (`Ctrl+Shift+C` / `Cmd+Shift+C`).
    pub fn toggle_cheat_console(&mut self) -> bool {
        self.hud.cheat_console.toggle()
    }

    /// Closes the cheat console if open; returns `true` if it was open.
    pub fn close_cheat_console(&mut self) -> bool {
        if self.hud.cheat_console.is_open() {
            self.hud.cheat_console.close();
            true
        } else {
            false
        }
    }

    pub fn cheat_console_char(&mut self, ch: char) -> bool {
        self.hud.cheat_console.insert_char(ch)
    }

    pub fn cheat_console_backspace(&mut self) -> bool {
        self.hud.cheat_console.backspace()
    }

    pub fn cheat_console_delete(&mut self) -> bool {
        self.hud.cheat_console.delete_forward()
    }

    pub fn cheat_console_left(&mut self) -> bool {
        self.hud.cheat_console.move_left()
    }

    pub fn cheat_console_right(&mut self) -> bool {
        self.hud.cheat_console.move_right()
    }

    pub fn cheat_console_home(&mut self) -> bool {
        self.hud.cheat_console.move_home()
    }

    pub fn cheat_console_end(&mut self) -> bool {
        self.hud.cheat_console.move_end()
    }

    pub fn cheat_console_history_up(&mut self) -> bool {
        self.hud.cheat_console.history_up()
    }

    pub fn cheat_console_history_down(&mut self) -> bool {
        self.hud.cheat_console.history_down()
    }

    /// Submits the current cheat console input line and executes every emitted
    /// [`sims_ui::cheat_console::CheatCommand`] against the active lot.
    pub fn cheat_console_submit(
        &mut self,
    ) -> Result<Vec<sims_ui::cheat_console::CheatCommand>, sims_ui::cheat_console::CheatParseError>
    {
        let commands = self.hud.cheat_console.submit()?;
        self.execute_cheat_commands(&commands);
        Ok(commands)
    }

    /// Executes a batch of parsed [`sims_ui::cheat_console::CheatCommand`]s
    /// cleanly against `self` and updates console feedback.
    pub fn execute_cheat_commands(&mut self, commands: &[sims_ui::cheat_console::CheatCommand]) {
        use sims_ui::cheat_console::{CheatCommand, FeedbackKind};

        if self.coop.is_active_or_starting() {
            self.hud.cheat_console.set_feedback(
                FeedbackKind::Error,
                "Cheats are disabled during co-op multiplayer sessions.",
            );
            return;
        }

        let mut total_funds_delta: i32 = 0;
        let mut target_hour: Option<u8> = None;
        let mut messages: Vec<String> = Vec::new();
        let mut saw_help = false;

        for cmd in commands {
            match *cmd {
                CheatCommand::AddFunds(amount) => {
                    total_funds_delta = total_funds_delta.saturating_add(amount);
                }
                CheatCommand::SetHour(hour) => {
                    target_hour = Some(hour);
                }
                CheatCommand::MoveObjects(on) => {
                    self.lot.set_move_objects(on);
                    messages.push(format!("move_objects {}", if on { "on" } else { "off" }));
                }
                CheatCommand::MapEdit(on) => {
                    self.cheat_map_edit = on;
                    self.lot.set_edit_budget(on);
                    messages.push(format!("map_edit {}", if on { "on" } else { "off" }));
                }
                CheatCommand::WaterTool => {
                    self.set_mode(Mode::Build);
                    self.select_build_tool(BuildTool::Water);
                    messages.push("Water tool selected".into());
                }
                CheatCommand::Autonomy(level) => {
                    self.cheat_autonomy = Some(level);
                    let free_will = level > 0;
                    let _ = self.lot.set_free_will(free_will);
                    self.hud.controls.options.play.free_will = free_will;
                    messages.push(format!("Autonomy set to {level}"));
                }
                CheatCommand::SimSpeed(val) => {
                    self.cheat_sim_speed = Some(val);
                    let speed = if val > 600 {
                        Speed::Ultra
                    } else if val > 200 {
                        Speed::Fast
                    } else {
                        Speed::Normal
                    };
                    self.set_speed(speed);
                    messages.push(format!("sim_speed {val}"));
                }
                CheatCommand::Interests => {
                    self.set_mode(Mode::Live);
                    self.hud.controls.expanded = true;
                    self.hud.controls.panel = Some(sims_ui::command::Panel::Interests);
                    messages.push("Interests panel opened".into());
                }
                CheatCommand::PrepareLot => {
                    self.buy_cursor = None;
                    self.buy_selected = None;
                    self.wall_cursor = None;
                    self.build_drag = None;
                    self.build_preview_shape = None;
                    self.refresh_world();
                    self.clear_lot_cache();
                    messages.push("Lot prepared".into());
                }
                CheatCommand::Help => {
                    saw_help = true;
                }
            }
        }

        if total_funds_delta != 0 {
            let edit_budget = self.lot.world().globals.get(32).copied().unwrap_or(0) != 0;
            match self.lot.add_funds(total_funds_delta) {
                Ok(_) => {
                    if edit_budget {
                        messages.push("Funds unchanged in edit-budget mode".into());
                    } else {
                        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                        self.dirty = true;
                        messages.push(format!("Added ${total_funds_delta}"));
                    }
                }
                Err(err) => {
                    self.hud
                        .cheat_console
                        .set_feedback(FeedbackKind::Error, format!("Cheat failed: {err}"));
                    return;
                }
            }
        }

        if let Some(hour) = target_hour {
            match self.lot.set_hour(hour) {
                Ok(()) => {
                    self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                    self.dirty = true;
                    self.clear_lot_cache();
                    let world = self.lot.world();
                    if let Ok(shape) = sims_render::Lot::from_world(world) {
                        self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation());
                    }
                    self.hud.sync_clock_blink(self.lot.world(), self.clock_us);
                    messages.push(format!("Hour set to {hour:02}:00"));
                }
                Err(err) => {
                    self.hud
                        .cheat_console
                        .set_feedback(FeedbackKind::Error, format!("Cheat failed: {err}"));
                    return;
                }
            }
        }

        if !messages.is_empty() {
            self.hud
                .cheat_console
                .set_feedback(FeedbackKind::Ok, messages.join("; "));
        } else if !saw_help {
            self.hud
                .cheat_console
                .set_feedback(FeedbackKind::Info, "Done.");
        }
    }

    /// Releases any active Buy/Build preview cursors or drags so a subsequent
    /// `Lot::restore` never leaves stale handles in `Game`.
    fn cancel_active_buy_build_tools(&mut self) {
        if let Some(cursor) = self.buy_cursor.take() {
            if cursor.is_existing() {
                let _ = self.lot.buy_snap_back(&cursor);
            } else {
                let _ = self.lot.buy_cancel(&cursor);
            }
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        }
        self.buy_selected = None;
        self.buy_in_house = false;
        self.buy_dragging = None;
        self.buy_commit_on_release = false;
        if let Some(cursor) = self.wall_cursor.take() {
            let _ = self.lot.build_cursor_close(&cursor);
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        }
        self.build_drag = None;
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
    }

    /// Applies an in-place mutation to the live `World` via lossless `Saved`
    /// round-tripping while preserving UI mode, camera, and open cheat console.
    fn mutate_world_via_saved(
        &mut self,
        mutator: impl FnOnce(&mut sims_sim::World),
    ) -> Result<(), String> {
        self.cancel_active_buy_build_tools();
        let saved = self.save()?;
        let orig_world_body = saved.world().encode();
        let encoded = saved.encode();
        let saved_body = encoded
            .get(8..)
            .ok_or_else(|| "invalid Saved snapshot header".to_owned())?;
        let tail = saved_body
            .strip_prefix(orig_world_body.as_slice())
            .ok_or_else(|| "Saved snapshot world prefix mismatch".to_owned())?;
        let mut world = saved.world().clone();
        mutator(&mut world);
        let new_world_body = world.encode();
        let mut rebuilt = Vec::with_capacity(8 + new_world_body.len() + tail.len());
        rebuilt.extend_from_slice(&encoded[..8]);
        rebuilt.extend_from_slice(&new_world_body);
        rebuilt.extend_from_slice(tail);
        let patched = Saved::decode(&rebuilt).map_err(|e| e.to_string())?;
        self.lot.restore(patched).map_err(|e| e.to_string())?;
        self.world_mutated_via_restore = true;
        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        self.dirty = true;
        self.clear_lot_cache();
        let world = self.lot.world();
        if !self.open_pack_mode {
            self.people.refresh(world);
        }
        if let Ok(shape) = sims_render::Lot::from_world(world) {
            self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation());
        }
        self.hud.sync_clock_blink(self.lot.world(), self.clock_us);
        Ok(())
    }

    /// Current lot geometry and render assets for capturing map thumbnails on save.
    pub fn render_parts_mut(
        &mut self,
    ) -> (
        &sims_sim::World,
        &sims_render::Lot,
        &mut sims_render::Assets,
    ) {
        (self.lot.world(), &self.lot_shape, &mut self.assets)
    }

    /// The lot, for reading.
    pub fn lot(&self) -> &Lot {
        &self.lot
    }

    /// Mutable access to the active [`Lot`] (used by travel departure/return).
    pub fn lot_mut(&mut self) -> &mut Lot {
        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        &mut self.lot
    }

    /// Enables or disables the Camera subpanel's Photo Album action (`CamViewAlbum`, `1105`).
    pub fn set_album_button_enabled(&mut self, enabled: bool) {
        self.hud.set_album_button_enabled(enabled);
    }

    /// Current native Camera Mode settings (`SnapshotSize`, `SnapshotQuality`,
    /// `album_enabled`, `quick_tips`).
    pub fn camera_state(&self) -> CameraState {
        self.hud.camera_state()
    }

    /// Active native `cWinCamera` panel controller.
    pub fn camera_panel(&self) -> &CameraPanel {
        self.hud.camera_panel()
    }

    /// Loaded `STR# 140` Camera Mode strings (button coordinates, tooltips, Quick Tips).
    pub fn camera_strings(&self) -> &CameraStrings {
        self.hud.camera_strings()
    }

    /// Last laid-out native [`CameraLayout`] when Camera Mode is open.
    pub fn camera_layout(&self) -> Option<&CameraLayout> {
        self.hud.camera_layout()
    }

    /// Screen rectangle of a specific native `cWinCamera` button when Camera Mode is open.
    pub fn camera_button_rect(&self, button: CameraButton) -> Option<sims_ui::frame::Rect> {
        self.hud.camera_button_rect(button)
    }

    /// Whether the bottom native Camera subpanel (`cWinCamera`) is currently open.
    pub fn camera_strip_open(&self) -> bool {
        self.hud.camera_strip_open()
    }

    /// Closes the bottom Camera subpanel if open. Returns `true` if it was open.
    pub fn close_camera_strip(&mut self) -> bool {
        self.hud.close_camera_strip()
    }

    /// Bounding rectangle of the bottom Camera subpanel when open.
    pub fn camera_strip_rect(&self) -> Option<sims_ui::frame::Rect> {
        self.hud.camera_strip_rect()
    }

    /// Screen rectangle of the Camera subpanel's "Take Snapshot" button (`CamSnap`),
    /// when open and wide enough to display beside `CamViewAlbum`.
    pub fn camera_snapshot_button_rect(&self) -> Option<sims_ui::frame::Rect> {
        self.hud.camera_snapshot_button_rect()
    }

    /// Screen rectangle of the native `CamViewAlbum` (`1105`) button, if open and enabled.
    pub fn album_button_rect(&self) -> Option<sims_ui::frame::Rect> {
        self.hud.album_button_rect()
    }

    /// Supplies real installed neighborhood/destination lot metadata (names,
    /// addresses, zoning) to destination lot pickers (`Dialog` kinds `5, 7, 8, 11, 13`).
    pub fn set_special_lot_catalog(&mut self, lots: Vec<SpecialLotEntry>) {
        self.special_lot_catalog = lots;
    }

    /// Sets the owning neighborhood slot (`1..=8`) so graphical destination
    /// map pickers load the matching `UserData*` map.
    pub fn set_neighborhood_slot(&mut self, slot: u8) {
        self.neighborhood_slot = slot.clamp(1, 8);
    }

    /// Whether the graphical destination map picker (`Dialog` kinds `5, 7, 8, 11, 13`)
    /// is currently open.
    pub fn destination_picker_open(&self) -> bool {
        self.destination_picker.is_some()
    }

    /// Active graphical [`DestinationMapPicker`], when a destination lot-picker
    /// special panel is open.
    pub fn destination_picker(&self) -> Option<&DestinationMapPicker> {
        self.destination_picker.as_ref()
    }

    /// Cancels the open graphical destination map picker (`Esc` or Return button),
    /// delivering `SpecialPanelResponse::Lot(None)` to the VM.
    pub fn destination_picker_escape(&mut self) -> Outcome {
        if self.destination_picker.is_none() {
            return Outcome::default();
        }
        if self.answer_special_panel(SpecialPanelResponse::Lot(None)) {
            self.hud.play(crate::hud::CLICK);
        } else {
            self.hud.play(crate::hud::ERROR);
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    /// Confirms the currently hovered visitable lot on the graphical
    /// destination map picker (`Enter`), if any.
    pub fn destination_picker_submit(&mut self) -> Outcome {
        let Some(lot_id) = self
            .destination_picker
            .as_ref()
            .and_then(DestinationMapPicker::hovered_lot)
        else {
            return Outcome::default();
        };
        if self.answer_special_panel(SpecialPanelResponse::Lot(Some(lot_id as i16))) {
            self.hud.play(crate::hud::CLICK);
        } else {
            self.hud.play(crate::hud::ERROR);
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    /// Takes any destination lot chosen in a VM special lot picker (`5, 7, 8, 11, 13`)
    /// so `NeighborhoodShell` can cleanly transition via `depart_to_destination`.
    pub fn take_pending_travel_lot(&mut self) -> Option<(SpecialPanelRequest, u32)> {
        self.pending_travel_lot.take()
    }

    /// Enables or disables `--debug` mode (which shows the maroon top
    /// diagnostic/status banner and logs diagnostic notes to stderr).
    pub fn set_debug(&mut self, debug: bool) {
        self.debug = debug;
        if !debug {
            self.last_debug_logged_note = None;
        }
    }

    /// Whether `--debug` diagnostic banner & stderr logging mode is active.
    pub fn debug(&self) -> bool {
        self.debug
    }

    /// Surfaces a genuine user-facing error or confirmation via a native Sims 1
    /// modal message dialog (`cTSWinMsgBox`, OK button), while also recording
    /// `body` in the programmatic [`Game::note`] getter (and logging to stderr
    /// when `--debug` is enabled).
    pub fn show_notice(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.pending_after_notice = None;
        let body = body.into();
        self.set_note(body.clone());
        self.hud.open_notice_dialog(title, body, self.view.size);
    }

    fn log_debug_note_if_new(&mut self) {
        if !self.debug {
            return;
        }
        let current = self.note.as_deref();
        if current != self.last_debug_logged_note.as_deref() {
            if let Some(msg) = current {
                eprintln!("sims-game [debug]: {msg}");
            }
            self.last_debug_logged_note = current.map(str::to_owned);
        }
    }

    /// House baseline roof state read from `HOUS 0`.
    pub fn base_roof(&self) -> &RoofState {
        &self.base_roof
    }

    /// Active roof state (live `ROOF_META_ARRAY` edit or `HOUS 0` baseline).
    pub fn current_roof_state(&self) -> RoofState {
        self.lot.roof_state_with_base(&self.base_roof)
    }

    /// Laid-out HUD items from the last rendered frame.
    pub fn hud_items(&self) -> &[sims_ui::hud::Item] {
        self.hud.items()
    }

    /// Whether the Camera Roof preview toggle (`Camera::Roof`) is active.
    pub fn roof_view(&self) -> bool {
        self.roof_view
    }

    /// Sets the Camera Roof view (`Camera::Roof`, native 4-way radio state
    /// `[ui + 0x48] == 8`). Turning Roof view on also raises walls to
    /// [`Walls::Up`] so the roof never floats over lowered walls.
    pub fn set_roof_view(&mut self, on: bool) {
        if on {
            self.view.camera.walls = Walls::Up;
        }
        self.roof_view = on;
        self.hud.set_roof_view(on);
    }

    /// Queues a Simlish voice clip for live person `sim_id` (using its person
    /// data age/gender/pet flags) to be mixed on the next [`sound::Feed::update`].
    pub fn play_sim_voice(&mut self, sim_id: u16, category: Option<&str>, seed: u64) -> bool {
        let Some(person) = self
            .lot
            .world()
            .objects
            .get(&sim_id)
            .and_then(|o| o.person.as_ref())
        else {
            return false;
        };
        let age = person.data.get(58).copied().unwrap_or(27);
        let gender = person.data.get(65).copied().unwrap_or(0);
        let flags = person.data.get(74).copied().unwrap_or(0);
        let voice = sound::voice_for_person(age, gender, flags);
        self.voice_inputs.push_back(sound::Input::PlayVoice {
            voice,
            category: category.map(str::to_owned),
            pitch: sims_audio::voices::VoicePitch::NATIVE,
            seed,
            object: Some(u32::from(sim_id)),
        });
        true
    }

    /// Drains queued Simlish voice mixer inputs (called by [`sound::Feed::update`]).
    pub fn take_voice_inputs(&mut self) -> Vec<sound::Input> {
        self.voice_inputs.drain(..).collect()
    }

    /// The hour of the game's clock (global 0), 0 to 23.
    pub fn hour(&self) -> u32 {
        let hour = self.lot.world().globals.first().copied().unwrap_or(0);
        u32::try_from(hour.rem_euclid(24)).unwrap_or(0)
    }

    /// The ambience's camera value (`sims_audio::ambience::view`): the
    /// share, in percent, of the tiles on the screen that are outdoors,
    /// on the floor shown. The original works it out each frame from its
    /// camera (0x570c3e).
    pub fn ambience_view(&self) -> i32 {
        let camera = &self.view.camera;
        // The camera's arithmetic overflows for an origin this far off;
        // no view the controls make comes near it.
        if camera.origin.iter().any(|n| n.unsigned_abs() >= 1 << 26) {
            return 0;
        }
        let corner = camera
            .rotate_fine(camera.unproject_fine([0, 0]))
            .map(|n| n >> 4);
        let level = u8::try_from(camera.level).unwrap_or(0);
        let world = self.lot.world();
        sims_audio::ambience::view(self.view.size, camera.zoom.level(), corner, |tile| {
            let tile = camera.unrotate_tile(tile.map(i32::from));
            world.outside(level, tile).unwrap_or(false)
        })
    }

    /// What of the camera sends the sound's kind 33 when it changes
    /// ([`sound::Shot`]).
    pub fn shot(&self) -> sound::Shot {
        sound::Shot::of(&self.view.camera, self.view.size)
    }

    /// What the sound scripts may ask about the lot's objects, with where
    /// each is on the screen as the camera shows it now
    /// ([`sound::sources`], [`sound::screen`]).
    pub fn sound_sources(&self) -> sims_audio::sources::Snapshot {
        let world = self.lot.world();
        let mut snapshot = sound::sources(world);
        snapshot.screen = sound::screen(world, &self.lot_shape, &self.view.camera, self.view.size);
        snapshot
    }

    /// Screen-space sound projections alone, used by [`sound::Feed`] on pure
    /// camera pans to avoid rebuilding `objects` and `positions`.
    pub(crate) fn sound_screen(&self) -> BTreeMap<i32, sims_audio::placement::Projection> {
        sound::screen(
            self.lot.world(),
            &self.lot_shape,
            &self.view.camera,
            self.view.size,
        )
    }

    /// Lightweight O(1) dirty-tracking key for [`sound::Feed::update`].
    pub(crate) fn sound_world_key(&self) -> sound::SoundWorldKey {
        let world = self.lot.world();
        sound::SoundWorldKey {
            ticks: self.lot.ticks(),
            restore_gen: self.restore_generation,
            sound_rev: self.world_sound_rev,
            error_count: self.errors.lines.len(),
            object_count: world.objects.len(),
            hour: world.globals.first().copied().unwrap_or(0),
            lot_id: world.globals.get(10).copied().unwrap_or(0),
            buy_held_pos: self
                .buy_cursor
                .as_ref()
                .and_then(|c| world.objects.get(&c.held).map(|o| (c.held, o.position))),
        }
    }

    /// The Options panel's current state (active tab, graphics, sound, and play settings).
    pub fn options(&self) -> &OptionsState {
        &self.hud.controls.options
    }

    /// Whether the lot has unsaved changes (`[app + 0xf7]`, checked by Quit at
    /// `0x516809` and Neighborhood at `0x5170ff`).
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Whether a UI-owned modal confirmation dialog is currently open.
    pub fn has_ui_dialog(&self) -> bool {
        self.hud.has_ui_dialog()
    }

    /// Whether any modal dialog (UI confirmation, simulation prompt, or special
    /// panel `Dialog` kinds `5..=15`) is open.
    pub fn has_dialog(&self) -> bool {
        self.hud.has_dialog()
            || self.book_dialog.is_open()
            || self.phonebook_dialog.is_open()
            || self.destination_picker.is_some()
            || self.lot.asking().is_some()
            || self.lot.asking_special_panel().is_some()
            || self.active_special_panel.is_some()
    }

    /// Loads persisted Options preferences from an `Options.iff` (`Optn` 128)
    /// file (`0x4e6505..0x4e685e`), applying Free Will to the lot's global 30.
    pub fn load_options(&mut self, path: &Path) -> Result<(), String> {
        let mut loaded = OptionsState::load_iff(path).map_err(|e| e.to_string())?;
        loaded.tab = self.hud.controls.options.tab;
        loaded.help = self.hud.controls.options.help;
        let free_will = loaded.play.free_will;
        self.hud.controls.options = loaded;
        if self.lot.world().globals.get(30).copied().unwrap_or(1) != i16::from(free_will) {
            let _ = self.lot.set_free_will(free_will);
        }
        Ok(())
    }

    /// Saves the current Options preferences to `path` as an `Options.iff`
    /// (`Optn` 128) file (`0x4e6505..0x4e685e`).
    pub fn save_options(&self, path: &Path) -> Result<(), String> {
        self.hud
            .controls
            .options
            .save_iff(path)
            .map_err(|e| e.to_string())
    }

    /// The lot's editable name shown in the edit-mode Save Card (`cWinEditLotInfo`,
    /// `0x50e1f8..0x50e42c`, persisted in [`Saved`]).
    pub fn lot_name(&self) -> &str {
        self.lot.lot_name()
    }

    /// The lot's editable description shown in the edit-mode Save Card
    /// (`cWinEditLotInfo`, `0x50e1f8..0x50e42c`, persisted in [`Saved`]).
    pub fn lot_description(&self) -> &str {
        self.lot.lot_description()
    }

    /// The text currently in the active modal dialog's secondary
    /// `cTSWinTextEdit2` field (Lot Description on the edit-mode Save Card).
    pub fn dialog_secondary_entry(&self) -> Option<&str> {
        self.hud.active_dialog_secondary_entry()
    }

    /// Switches keyboard focus between Lot Name and Lot Description on an open
    /// edit-mode Save Card (`Tab`).
    pub fn dialog_switch_field(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.switch_dialog_field()
    }

    /// Whether pressing Return/Enter in the active dialog inserts a newline
    /// into a multiline field (Lot Description on the edit-mode Save Card,
    /// `0x50db98..0x50dba0`, `0x5f28b6..0x5f28f9`) instead of submitting.
    pub fn dialog_enter_is_newline(&self) -> bool {
        !self.ui_modal_in_front() && self.hud.is_multiline_entry()
    }

    /// Handles a native modal confirmation key (`Enter`, `Escape`, or localized
    /// Yes/No from `STR# 161`, `0x5e5b3b..0x5e5bfd`, `0x6d7530`).
    pub fn dialog_confirm_key(&mut self, key: DialogKey) -> Outcome {
        if self.phonebook_dialog.is_open() {
            return match key {
                DialogKey::Enter => self.phonebook_submit(),
                DialogKey::Escape => self.phonebook_escape(),
                _ => Outcome::default(),
            };
        }
        if self.book_dialog.is_open() {
            return match key {
                DialogKey::Enter | DialogKey::Escape => self.book_close(),
                _ => Outcome::default(),
            };
        }
        // Special-panel pickers sit in front of any VM prompt: their keys
        // never reach it.
        if self.shop_dialog.is_open() {
            return match key {
                DialogKey::Enter => self.shop_submit(),
                DialogKey::Escape => self.shop_escape(),
                _ => Outcome::default(),
            };
        }
        if self.active_special_panel.is_some() || self.destination_picker.is_some() {
            return Outcome::default();
        }
        if key == DialogKey::Tab {
            return Outcome {
                redraw: self.hud.switch_dialog_field(),
                ..Outcome::default()
            };
        }
        if key == DialogKey::Enter && self.hud.is_multiline_entry() {
            return Outcome {
                redraw: self.hud.type_dialog_char('\n'),
                ..Outcome::default()
            };
        }
        let Some(command) = self.hud.confirm_key(key) else {
            return Outcome::default();
        };
        let outcome = self.command(command, None);
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        outcome
    }

    /// Called by the window host when a save requested after edit-mode
    /// `SaveCardQuit` failed: if `ConfirmQuit` was queued while `dirty` is
    /// still set, upgrades it to `ConfirmSaveBeforeQuit` (`0x516821 ->
    /// 0x516866 -> 0x4f290d` with `dirty == true`).
    pub fn after_save_failed(&mut self) {
        if self.dirty && self.hud.ui_dialog_kind() == Some(OptionsDialog::ConfirmQuit) {
            let name = self.lot.lot_name().to_owned();
            let desc = self.lot.lot_description().to_owned();
            self.hud.open_options_dialog(
                OptionsDialog::ConfirmSaveBeforeQuit,
                self.view.size,
                &name,
                &desc,
            );
        }
    }

    /// Called by the window host when a save failed and a user-facing notice
    /// should explain the write failure first. When `ConfirmQuit` was queued
    /// after edit-mode `SaveCardQuit` while `dirty` is still set, chains
    /// `OptionsDialog::Notice` -> `OptionsDialog::ConfirmSaveBeforeQuit` on
    /// dismissal so the player sees the exact error before retrying or quitting
    /// without saving.
    pub fn after_save_failed_with_notice(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) {
        if self.dirty && self.hud.ui_dialog_kind() == Some(OptionsDialog::ConfirmQuit) {
            self.show_notice(title, body);
            self.pending_after_notice = Some(OptionsDialog::ConfirmSaveBeforeQuit);
        } else if !self.hud.has_ui_dialog() {
            self.show_notice(title, body);
        }
    }

    /// Whether a Sound volume slider drag is currently captured (`0x5f6245`).
    pub fn dragging_slider(&self) -> bool {
        self.hud.dragging_slider()
    }

    /// Updates an active Sound volume slider drag with a canvas x-coordinate
    /// (including out-of-window coordinates while captured, `0x5f6245..0x5f6336`).
    pub fn drag_slider(&mut self, x: i32) -> bool {
        self.hud.drag_slider(x)
    }

    /// Whether a `cTSWinTextEdit2` mouse-drag text selection is currently captured
    /// (`[esi + 0x157]` at `0x5f27a6..0x5f2887`).
    pub fn dragging_dialog_entry(&self) -> bool {
        self.hud.dragging_dialog_entry()
    }

    /// Updates a captured `cTSWinTextEdit2` mouse-drag text selection with canvas
    /// coordinates (including out-of-window coordinates while captured,
    /// `0x5f27ae..0x5f2887`).
    pub fn drag_dialog_entry(&mut self, point: [i32; 2]) -> bool {
        self.hud.drag_dialog_entry(point)
    }

    /// Displays a status note across the top of the frame, clearing any
    /// pending post-save follow-up note so a failed host store open never
    /// leaks a stale Neighborhood or Reset Tutorial note onto a later save.
    pub fn set_note(&mut self, note: impl Into<String>) {
        self.pending_after_save_note = None;
        self.note = Some(note.into());
        self.log_debug_note_if_new();
    }

    /// The status note currently displayed across the top of the frame, if any.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Whether the active modal dialog has an editable `cTSWinTextEdit2` field.
    pub fn has_dialog_entry(&self) -> bool {
        !self.ui_modal_in_front() && self.hud.active_dialog_entry().is_some()
    }

    /// Whether a Game-drawn modal (phone book, book, shop picker, special
    /// panel or destination picker) sits in front of any HUD or VM dialog.
    /// Keys then go to it, never to a text field behind it.
    fn ui_modal_in_front(&self) -> bool {
        self.phonebook_dialog.is_open()
            || self.book_dialog.is_open()
            || self.shop_dialog.is_open()
            || self.active_special_panel.is_some()
            || self.destination_picker.is_some()
    }

    /// The text currently in the active modal dialog's `cTSWinTextEdit2` field,
    /// if any.
    pub fn dialog_entry(&self) -> Option<&str> {
        self.hud.active_dialog_entry()
    }

    /// The text entered in the last answered VM kind 3 text-entry dialog
    /// (`0x451a6b..0x451a85` -> global `[0x6e4ac0]`).
    pub fn last_dialog_text(&self) -> &str {
        self.lot.last_dialog_text()
    }

    /// Whether the lot is open in edit mode (`[lot + 0x50] == 1`, `0x4f24b5`,
    /// `0x4f24d9`, `0x517c21`).
    pub fn edit_mode(&self) -> bool {
        self.edit_mode
    }

    /// Sets whether the lot is open in neighborhood edit mode (`[lot + 0x50] == 1`).
    pub fn set_edit_mode(&mut self, edit: bool) {
        self.edit_mode = edit;
        self.lot.set_edit_budget(edit);
    }

    /// Per-lot Options visibility (`0x4f2490..0x4f24e7`, `0x5177aa..0x517833`).
    pub fn lot_visibility(&self) -> LotVisibility {
        let world = self.lot.world();
        let has_family = world.globals.get(9).copied().unwrap_or(0) != 0;
        LotVisibility::for_lot(
            self.lot.lot_id(),
            self.lot.lot_zoning(),
            has_family,
            self.edit_mode,
        )
    }

    /// Current `[fx, music, vox]` Sound slider steps (`0..=10`), wired to
    /// `sims_audio::player::Player` via [`crate::sound::Feed`].
    pub fn volumes(&self) -> [u8; 3] {
        self.hud.controls.options.sound.steps()
    }

    /// Sets one Sound volume slider step (`0..=10`, clamped).
    pub fn set_volume(&mut self, channel: VolumeChannel, step: u8) -> bool {
        self.hud.controls.options.sound.set(channel, step)
    }

    /// Types `ch` into the active modal dialog's `cTSWinTextEdit2` input field.
    pub fn type_dialog_char(&mut self, ch: char) -> bool {
        !self.ui_modal_in_front() && self.hud.type_dialog_char(ch)
    }

    /// Deletes the character before the caret in the active dialog's input field.
    pub fn dialog_backspace(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_backspace()
    }

    /// Deletes the character at the caret in the active dialog's input field.
    pub fn dialog_delete(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_delete()
    }

    /// Moves the active dialog's text-entry caret one character left.
    pub fn dialog_move_left(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_move_left()
    }

    /// Moves the active dialog's text-entry caret one character right.
    pub fn dialog_move_right(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_move_right()
    }

    /// Moves the active dialog's text-entry caret one visual row up (multiline
    /// fields, `0x5f29f2..0x5f2a90`).
    pub fn dialog_move_up(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_move_up()
    }

    /// Moves the active dialog's text-entry caret one visual row down (multiline
    /// fields, `0x5f2a90..0x5f2b40`).
    pub fn dialog_move_down(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_move_down()
    }

    /// Moves the active dialog's text-entry caret to the start of the current
    /// line (or field, `0x5f2c13`).
    pub fn dialog_home(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_home()
    }

    /// Moves the active dialog's text-entry caret to the end of the field.
    pub fn dialog_end(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_end()
    }

    /// Submits the active modal dialog via Enter (activates button 0 if enabled).
    pub fn dialog_submit(&mut self) -> Outcome {
        // The foreground modal takes Enter first, so it never reaches a VM
        // prompt waiting behind it.
        if self.phonebook_dialog.is_open()
            || self.book_dialog.is_open()
            || self.shop_dialog.is_open()
        {
            return self.dialog_confirm_key(DialogKey::Enter);
        }
        if self.active_special_panel.is_some() || self.destination_picker.is_some() {
            return Outcome::default();
        }
        let Some(command) = self.hud.submit_dialog() else {
            return Outcome::default();
        };
        let outcome = self.command(command, None);
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        outcome
    }

    /// Scrolls the camera when `Edge Scrolling` is enabled (`0x4e6246..0x4e6286`,
    /// `0x4d25b6..0x4d25c7` -> `0x4d45f3` -> `0x4d4709..0x4d4ade`) and the
    /// pointer rests within the 32-pixel windowed edge band of the house
    /// viewport in any mode with no modal dialog or pie menu open. Returns
    /// whether the camera moved.
    pub fn edge_scroll(&mut self, elapsed_us: u64) -> bool {
        if !self.hud.controls.options.play.edge_scrolling
            || self.hud.has_ui_dialog()
            || self.lot.asking().is_some()
            || self.lot.asking_special_panel().is_some()
            || self.shop_dialog.is_open()
            || self.book_dialog.is_open()
            || self.phonebook_dialog.is_open()
            || self.hud.menu.is_some()
            || self.cheat_console_open()
        {
            self.edge_scroll_rem = [0, 0];
            return false;
        }
        let Some([px, py]) = self.pointer else {
            self.edge_scroll_rem = [0, 0];
            return false;
        };
        // `0x4d45c8` -> `0x4d4843`: subtract the expanded HUD strip height so
        // bottom-edge scrolling triggers at the house viewport bottom and does
        // not pan when hovering over the control panel strip.
        let (vw, vh) = self.house_viewport_size();
        let (w, h) = (vw as i32, vh as i32);
        // `0x4d4586..0x4d45b1`, `0x4d4843`: windowed mode uses a 32-pixel
        // (`0x20`) edge scroll band inside the house viewport.
        const EDGE: i32 = 32;
        if w <= EDGE * 2 || h <= EDGE * 2 || px < 0 || py < 0 || px >= w || py >= h {
            self.edge_scroll_rem = [0, 0];
            return false;
        }
        let dir_x: i128 = if px < EDGE {
            1
        } else if px >= w - EDGE {
            -1
        } else {
            0
        };
        let dir_y: i128 = if py < EDGE {
            1
        } else if py >= h - EDGE {
            -1
        } else {
            0
        };
        if dir_x == 0 || dir_x * self.edge_scroll_rem[0] < 0 {
            self.edge_scroll_rem[0] = 0;
        }
        if dir_y == 0 || dir_y * self.edge_scroll_rem[1] < 0 {
            self.edge_scroll_rem[1] = 0;
        }
        // `0x4d4440`: `[esi + 0x60] = -562.5` px/s and `[esi + 0x64] = 562.5` px/s.
        // Straight edges (`0x4d4839`, `0x4d4946`, `0x4d499d`, `0x4d4a06`) scroll
        // at 562.5 px/s = 9 / 16_000 px/us exactly. Corners (`0x4d4893`,
        // `0x4d48f9`) start at raw `(±1125.0, ±562.5)` and are normalized by
        // `0x4d4ab0..0x4d4ad8` (`0x4d4ab5`) to magnitude 562.5 px/s
        // (`(1125 / sqrt(5), 562.5 / sqrt(5))` px/s). `0x4d50ef..0x4d5160` caps
        // each step at 100 ms and skips movement when `dt == 0`.
        if elapsed_us == 0 || (dir_x == 0 && dir_y == 0) {
            return false;
        }
        const SCALE: i128 = 16_000_000_000;
        const STRAIGHT_PER_US: i128 = 9_000_000; // 562.5 px/s exactly
        const CORNER_X_PER_US: i128 = 8_049_845; // 1125 / sqrt(5) px/s
        const CORNER_Y_PER_US: i128 = 4_024_922; // 562.5 / sqrt(5) px/s
        let (rate_x, rate_y) = if dir_x != 0 && dir_y != 0 {
            (CORNER_X_PER_US, CORNER_Y_PER_US)
        } else {
            (STRAIGHT_PER_US, STRAIGHT_PER_US)
        };
        let dt_us = i128::from(elapsed_us.min(100_000));
        let acc_x = self.edge_scroll_rem[0] + dir_x * dt_us * rate_x;
        let acc_y = self.edge_scroll_rem[1] + dir_y * dt_us * rate_y;
        let dx = (acc_x / SCALE) as i32;
        let dy = (acc_y / SCALE) as i32;
        self.edge_scroll_rem = [acc_x % SCALE, acc_y % SCALE];
        if dx == 0 && dy == 0 {
            return false;
        }
        let moved = self.apply(Move::Scroll([dx, dy]));
        if moved && let Some(point) = self.pointer {
            let _ = self.refresh_mode_hover(point);
        }
        moved
    }

    /// Refreshes Buy/Build hover previews (including held furniture in Build
    /// mode) or Live pointer following after pointer movement or edge scrolling.
    fn refresh_mode_hover(&mut self, point: [i32; 2]) -> bool {
        if self.hud.controls.mode == Mode::Buy {
            if !self.hud.covers(point) || self.buy_dragging.is_some() {
                self.hover_buy(point)
            } else {
                self.hide_buy_cursor()
            }
        } else if self.hud.controls.mode == Mode::Build {
            if self.buy_selected.is_some()
                || self.buy_cursor.is_some()
                || self.coop_buy_preview.is_some()
            {
                if !self.hud.covers(point) || self.buy_dragging.is_some() {
                    self.hover_buy(point)
                } else {
                    self.hide_buy_cursor()
                }
            } else if !self.hud.covers(point) {
                self.hover_build(point)
            } else if self.build_drag.is_none() {
                self.hide_build_cursor()
            } else {
                false
            }
        } else {
            self.follow()
        }
    }

    /// Why the lot stopped, if it has.
    pub fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// The errors play mode caught and recovered from, one line each, in
    /// order: the first `LOGGED` (256) different ones (an error again at a
    /// later tick is not listed again), then a line saying more were
    /// caught.
    pub fn errors(&self) -> &[String] {
        &self.errors.lines
    }

    /// The sound requests of the steps run since the last call, in
    /// order, for the sound system ([`crate::sound::event`]).
    pub fn take_sounds(&mut self) -> Vec<SoundEvent> {
        self.sounds.drain(..).collect()
    }

    /// The HUD highlight requests emitted by UI Effect (opcode 34, `0x44acff`)
    /// since the last call, in order.
    pub fn take_ui_effects(&mut self) -> Vec<UiEffectEvent> {
        self.ui_effects.drain(..).collect()
    }

    /// Control IDs currently highlighted by UI Effect mode 0 (`0x476485`).
    pub fn highlighted_buttons(&self) -> &BTreeSet<i16> {
        &self.highlighted_buttons
    }

    /// Person GUIDs whose portrait button is currently highlighted by UI Effect
    /// mode 1 (`0x47690d`).
    pub fn highlighted_portraits(&self) -> &BTreeSet<u32> {
        &self.highlighted_portraits
    }

    /// Neighbor GUIDs whose relationship control is currently highlighted by
    /// UI Effect mode 2 (`0x4769c3`).
    pub fn highlighted_relationships(&self) -> &BTreeSet<u32> {
        &self.highlighted_relationships
    }

    /// How many errors play mode caught.
    pub fn caught(&self) -> u64 {
        self.errors.count
    }

    /// Problems drawing the people, one line each.
    pub fn problems(&self) -> &[String] {
        &self.people.problems
    }

    /// Sets the game's speed, as the panel's speed buttons do in live
    /// mode (`0x53a62a`, `0x53a6ad`, `0x53a728`): a paused game runs
    /// again (the original applies it after the speed; both restart the
    /// same clocks, so the order does not matter), and the simulation's speed ([`Lot::set_speed`]) and the
    /// time between ticks with it are set, unless the speed is already
    /// the one asked for (`0x4ce588`), when the time owed carries on. A
    /// lot that cannot take it keeps its speed, and the note across the
    /// top says why.
    pub fn set_speed(&mut self, speed: Speed) {
        let level = pace::level(speed);
        if self.coop.is_active() {
            if self.coop.session().is_some_and(|s| s.time.explicit_pause)
                && let Err(e) = self
                    .coop
                    .submit_command(NetCommand::SetPaused { paused: false })
            {
                self.note = Some(format!("The pause was not changed: {e}"));
                return;
            }
            if let Err(e) = self.coop.submit_command(NetCommand::SetSpeed {
                speed: NetSimSpeed::new(level),
            }) {
                self.note = Some(format!("The speed was not changed: {e}"));
            } else if self.hud.controls.mode == sims_ui::command::Mode::Live {
                self.hud.set_idle_speed(false);
            }
            return;
        }
        let result = if self.paused() {
            self.lot.unpause().map(|_| self.pace.restart())
        } else {
            Ok(())
        };
        let result = result.and_then(|()| {
            if self.lot.world().speed() == level {
                self.pace.speed = speed;
                return Ok(());
            }
            self.lot
                .set_speed(i32::from(level))
                .map(|_| self.pace.set_speed(speed))
        });
        match result {
            Ok(()) => {
                // 0x53a5e3, 0x53a661..0x53a666, 0x53a6dc..0x53a6e1: manual
                // speed presses in Live mode clear `[control_panel + 0x1ec]`.
                if self.hud.controls.mode == sims_ui::command::Mode::Live {
                    self.hud.set_idle_speed(false);
                }
            }
            Err(e) => {
                self.note = Some(format!("The speed was not changed: {e}"));
            }
        }
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
    }

    /// Whether the game is paused (global 17).
    pub fn paused(&self) -> bool {
        self.lot.world().paused()
    }

    /// Pauses the game, or runs it again, as the panel's pause button
    /// does. A paused lot goes on ticking, with its clock standing
    /// ([`Lot::pause`]).
    pub fn toggle_pause(&mut self) {
        if self.coop.is_active() {
            let next_paused = !self.coop.session().is_some_and(|s| s.time.explicit_pause);
            if let Err(e) = self.coop.submit_command(NetCommand::SetPaused {
                paused: next_paused,
            }) {
                self.note = Some(format!("The pause was not changed: {e}"));
            }
            return;
        }
        let result = if self.paused() {
            self.lot.unpause().map(|_| self.pace.restart())
        } else {
            self.lot.pause().map(|_| ())
        };
        if let Err(e) = result {
            self.note = Some(format!("The pause was not changed: {e}"));
        }
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
    }

    /// Whether the pointer is currently inside an active edge-scrolling band
    /// (without mutating accumulator state), used to schedule display-refresh
    /// wakeups.
    pub fn is_edge_scrolling(&self, dragging: bool) -> bool {
        if dragging
            || !self.hud.controls.options.play.edge_scrolling
            || self.hud.has_ui_dialog()
            || self.lot.asking().is_some()
            || self.lot.asking_special_panel().is_some()
            || self.shop_dialog.is_open()
            || self.book_dialog.is_open()
            || self.phonebook_dialog.is_open()
            || self.hud.menu.is_some()
            || self.cheat_console_open()
        {
            return false;
        }
        let Some([px, py]) = self.pointer else {
            return false;
        };
        let (vw, vh) = self.house_viewport_size();
        let (w, h) = (vw as i32, vh as i32);
        const EDGE: i32 = 32;
        if w <= EDGE * 2 || h <= EDGE * 2 || px < 0 || py < 0 || px >= w || py >= h {
            return false;
        }
        px < EDGE || px >= w - EDGE || py < EDGE || py >= h - EDGE
    }

    /// Exact microseconds until the lot next needs an event-loop wakeup, or
    /// `None` when completely idle (allowing `ControlFlow::Wait`).
    pub fn next_wake_us(&self, dragging: bool, anim_interval_us: u64) -> Option<u64> {
        self.next_wake_us_with_window(dragging, anim_interval_us, true, false)
    }

    /// Like [`Game::next_wake_us`], gating pure presentation wakeups on
    /// `can_draw` and yielding to the redraw pacer when `frame_dirty` is
    /// already set so a zero-sized window or deferred redraw never spins.
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
        if self.coop.is_active_or_starting() || self.pending_coop_restage().is_some() {
            consider(5_000);
        }
        let ticks_active = self.hud.controls.mode == Mode::Live
            && !self.hud.has_ui_dialog()
            && !self.book_dialog.is_open()
            && !self.phonebook_dialog.is_open()
            && self.destination_picker.is_none()
            && self.lot.asking().is_none()
            && self.lot.asking_special_panel().is_none()
            && self.stopped.is_none();
        if ticks_active {
            consider(self.pace.remaining_us(self.lot.world().speed()));
        }
        if can_draw {
            if let Some(picker) = &self.destination_picker {
                if let Some(us) = picker.next_wake_us(self.clock_us, anim_interval_us) {
                    consider(us);
                }
                if let Some(us) = self.coop_lobby.next_blink_us(self.clock_us) {
                    consider(us);
                }
                return best;
            }
            if self.is_edge_scrolling(dragging)
                || self.pie_animating()
                || self.ui_animating()
                || self.ui_animating_prev
                || self.camera_panning()
            {
                consider(anim_interval_us.max(1_000));
            }
            if self
                .lot
                .world()
                .objects
                .values()
                .any(|o| o.guid == sims_people::bob::STANDARD)
            {
                let elapsed = self.clock_us.saturating_sub(self.drawn_us);
                let rem = SPIN_FRAME_US.saturating_sub(elapsed);
                if rem > 0 {
                    consider(rem);
                } else if !frame_dirty {
                    // Once `frame_dirty` is true, `redraw_pacer` owns the next
                    // presentation deadline; never return 0 us and busy-spin.
                    consider(anim_interval_us.max(1_000));
                }
            }
            if let Some(us) = self.hud.next_blink_us(self.clock_us) {
                consider(us);
            }
            if let Some(us) = self.coop_lobby.next_blink_us(self.clock_us) {
                consider(us);
            }
        }
        best
    }

    /// Whether the pause button's 500 ms blink phase changed since the
    /// frame was last drawn.
    pub fn blink_due(&self) -> bool {
        self.blink_dirty
    }

    /// Whether the plumb bob has spun enough since the frame was drawn to
    /// draw it again, though no tick ran (as while a dialog waits): a
    /// plumb bob is on the lot and [`SPIN_FRAME_US`] have passed.
    pub fn spin_due(&self) -> bool {
        self.clock_us.saturating_sub(self.drawn_us) >= SPIN_FRAME_US
            && self
                .lot
                .world()
                .objects
                .values()
                .any(|o| o.guid == sims_people::bob::STANDARD)
    }

    /// Runs the ticks owed after `elapsed_us` more microseconds at the
    /// simulation's speed. Returns how many ran.
    ///
    /// While a dialog waits for the player, no time is owed: the game
    /// waits, as the original does, and the idle loop's speed is not set
    /// (the lot takes nothing between ticks then; **unverified** whether
    /// the original's idle loop sets it while its dialog is up).
    ///
    /// Each call is a pass of the original's idle loop, so it first sets
    /// the idle loop's own speed ([`Lot::idle_speed`]), whether a tick is
    /// owed or not, paused or not: a speed the player picks after that
    /// stands. The same check runs again before each tick owed. Whenever
    /// it calls the setter, even for the speed the game already has, the
    /// time owed starts again and no more ticks run in this call, as the
    /// setter restarts the original's clocks. The original may then run
    /// an update at once, when its second clock (`0x4732de`) is ready
    /// (`0x46da1b`); here the next tick waits a period of the new speed
    /// (**unverified**, and the window's timing, not the simulation's).
    pub fn advance(&mut self, elapsed_us: u64) -> u32 {
        self.advance_while(elapsed_us, elapsed_us, |_| true).0
    }

    /// Advances the presentation clock by `clock_elapsed_us` and runs the
    /// ticks owed for `step_elapsed_us` while `keep_going(ran)` returns
    /// `true`. Returns `(ran, capped)`, where `capped` is `true` if
    /// `keep_going` stopped the batch before all owed ticks ran (in which case
    /// [`Pace::restart`] drops the leftover fraction).
    ///
    /// Keeping `clock_elapsed_us` separate from `step_elapsed_us` lets the
    /// interactive window loop drop capped simulation time from scheduler
    /// pacing without slowing the plumb bob spin or pause-button blink;
    /// headless callers use [`Game::advance`].
    pub fn advance_while(
        &mut self,
        clock_elapsed_us: u64,
        step_elapsed_us: u64,
        mut keep_going: impl FnMut(u32) -> bool,
    ) -> (u32, bool) {
        self.clock_us = self.clock_us.saturating_add(clock_elapsed_us);
        self.hud.set_clock_us(self.clock_us);
        if self.step_camera_pan() {
            self.blink_dirty = true;
        }
        let ui_anim_now = self.hud.is_ui_animating(self.clock_us);
        if ui_anim_now || self.ui_animating_prev {
            self.blink_dirty = true;
        }
        self.ui_animating_prev = ui_anim_now;
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us)
            || self.hud.sync_dialog_blink(self.clock_us)
            || self.coop_lobby.sync_blink(self.clock_us)
        {
            self.blink_dirty = true;
        }
        if let Some(picker) = &mut self.destination_picker
            && picker.tick(self.clock_us)
        {
            self.blink_dirty = true;
        }
        if self.coop.is_active_or_starting() {
            return self.advance_coop_while(clock_elapsed_us, step_elapsed_us, keep_going);
        }
        if self.hud.controls.mode != Mode::Live
            || self.hud.has_ui_dialog()
            || self.lot.asking().is_some()
            || self.lot.asking_special_panel().is_some()
        {
            self.pace.restart();
            return (0, false);
        }
        if self.idle_speed() {
            return (0, false);
        }
        let due = self.pace.due(step_elapsed_us, self.lot.world().speed());
        let (ran, capped) = self.run_while(due, true, &mut keep_going);
        if capped {
            self.pace.restart();
        }
        if ran > 0 {
            self.dirty = true;
        }
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
        (ran, capped)
    }

    /// Sets the idle loop's own speed between ticks ([`Lot::idle_speed`]).
    /// True if it called the setter, which starts the time owed again.
    fn idle_speed(&mut self) -> bool {
        // A lot that cannot take it (halted, or not loaded) has stopped,
        // and the banner says why.
        let set = self.lot.idle_speed().unwrap_or(false);
        if set {
            self.pace.restart();
            // 0x46d7b5 -> 0x109(1) when entering idle speed, 0x46d803 ->
            // 0x109(0) when leaving (`return_speed` reset to -1).
            self.hud
                .set_idle_speed(self.lot.world().return_speed() >= 0);
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
        }
        set
    }

    /// The dialog the lot waits on, if it waits for the player.
    pub fn asking(&self) -> Option<&sims_vm::boot::Asking> {
        self.lot.asking()
    }

    /// The special panel (`Dialog` kinds `5..=15`) the lot waits on, if it waits
    /// for the player.
    pub fn asking_special_panel(&self) -> Option<&AskingSpecialPanel> {
        self.lot.asking_special_panel()
    }

    /// The active [`SpecialPanelRequest`] currently presented in the UI, if any.
    pub fn active_special_panel_request(&self) -> Option<&SpecialPanelRequest> {
        self.active_special_panel.as_ref().map(|a| &a.request)
    }

    /// Where the dialog shown in the last frame drawn is, and its buttons.
    pub fn dialog(&self) -> Option<&sims_ui::dialog::Layout> {
        self.hud.dialog()
    }

    /// Runs up to `ticks` ticks, stopping at the first that fails or that
    /// shows a dialog the player has to answer.
    pub fn run(&mut self, ticks: u32) -> u32 {
        let ran = self.run_while(ticks, false, |_| true).0;
        if ran > 0 {
            self.dirty = true;
        }
        ran
    }

    /// Runs up to `ticks` ticks, stopping at the first that fails, before
    /// `keep_going(ran)` returns `false`, and, if `idle`, before a tick the
    /// idle loop's speed sets the speed for ([`Game::idle_speed`]) or after a
    /// tick that changes the simulation's speed or return speed (such as
    /// Special Effect resetting to normal, `0x4f684c` → `0x46d697`). Any
    /// mid-tick speed or return-speed change updates the panel's speed and
    /// restarts the time owed, whether `idle` is set or not.
    fn run_while(
        &mut self,
        ticks: u32,
        idle: bool,
        mut keep_going: impl FnMut(u32) -> bool,
    ) -> (u32, bool) {
        let mut ran = 0;
        let mut capped = false;
        while ran < ticks && self.stopped.is_none() {
            if idle && self.idle_speed() {
                break;
            }
            if !keep_going(ran) {
                capped = true;
                break;
            }
            let speed_before = (self.lot.world().speed(), self.lot.world().return_speed());
            let step = if self.play {
                self.lot.step_play()
            } else {
                self.lot.step()
            };
            match step {
                Ok(update) => {
                    ran += 1;
                    let at = self.lot.ticks();
                    for line in in_order(at, &update.errors, &update.frame_errors) {
                        self.errors.push(at, line);
                    }
                    self.view.masks_rebuilt(update.masks_rebuilt);
                    self.keep_sounds(update.sounds);
                    self.keep_ui_effects(update.ui_effects);
                    for (req, resp) in update.special_panels_answered {
                        if matches!(
                            req,
                            SpecialPanelRequest::DowntownLotPicker
                                | SpecialPanelRequest::VacationLotPicker { .. }
                                | SpecialPanelRequest::OldTownLotPicker { .. }
                                | SpecialPanelRequest::StudioTownLotPicker { .. }
                                | SpecialPanelRequest::MagicTownLotPicker { .. }
                        ) && let SpecialPanelResponse::Lot(Some(lot_id)) = resp
                            && lot_id > 0
                        {
                            self.pending_travel_lot = Some((req, lot_id as u32));
                        }
                    }
                }
                // The tick shows a dialog or special panel: it runs once it is
                // answered, and no time is owed from before the wait.
                Err(StepError::Asking { .. }) => {
                    self.pace.restart();
                    self.sync_special_panel();
                    break;
                }
                Err(error) => {
                    match &error {
                        // A strict step whose frame failed after its tick
                        // ran still made that tick's sounds.
                        StepError::Frame {
                            update: Some(update),
                            ..
                        } => {
                            self.view.masks_rebuilt(update.masks_rebuilt);
                            self.keep_sounds(update.sounds.clone());
                            self.keep_ui_effects(update.ui_effects.clone());
                        }
                        // A play step whose tick failed hands back what
                        // it caught and made before the failure.
                        StepError::Tick {
                            tick,
                            update: Some(update),
                            ..
                        } => {
                            for line in in_order(*tick, &update.errors, &update.frame_errors) {
                                self.errors.push(*tick, line);
                            }
                            self.view.masks_rebuilt(update.masks_rebuilt);
                            self.keep_sounds(update.sounds.clone());
                            self.keep_ui_effects(update.ui_effects.clone());
                        }
                        _ => {}
                    }
                    self.stop(&error);
                }
            }
            // `Lot::step` runs `World::idle_speed` before the tick (0x46d9d6).
            // Detect `0x109(1)` / `0x109(0)` transitions directly from
            // `return_speed`'s sign change so even a same-tick Special Effect
            // `0x10c` reset (`(NORMAL, -1) -> (IDLE, NORMAL) -> (NORMAL, NORMAL)`)
            // arms `[control_panel + 0x1ec]` under both `Game::run` and
            // `Game::advance`.
            let return_after = self.lot.world().return_speed();
            if speed_before.1 < 0 && return_after >= 0 {
                self.hud.set_idle_speed(true);
            } else if speed_before.1 >= 0 && return_after < 0 {
                self.hud.set_idle_speed(false);
            }
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
            if sync_speed_after_step(&mut self.pace, speed_before, self.lot.world()) && idle {
                break;
            }
        }
        if ticks > 0 && self.stopped.is_none() {
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        }
        if ran > 0 || self.stopped.is_some() {
            self.hud.sync_character_names(self.lot.character_names());
            let world = self.lot.world();
            self.people.refresh(world);
            // The room lights change as lamps and the time of day do.
            match sims_render::Lot::from_world(world) {
                Ok(shape) => self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation()),
                Err(e) => {
                    self.people
                        .note(vec![format!("the lot is not redrawn: {e}")]);
                }
            }
        }
        (ran, capped)
    }

    /// Keeps a step's sound requests for the host, at most
    /// [`SOUNDS_KEPT`], the oldest dropped first.
    fn keep_sounds(&mut self, sounds: Vec<SoundEvent>) {
        keep(&mut self.sounds, sounds, SOUNDS_KEPT);
    }

    /// Applies and queues a step's UI Effect highlight requests.
    fn keep_ui_effects(&mut self, effects: Vec<UiEffectEvent>) {
        for &effect in &effects {
            match effect {
                UiEffectEvent::Button { id, highlight } => {
                    if highlight {
                        self.highlighted_buttons.insert(id);
                    } else {
                        self.highlighted_buttons.remove(&id);
                    }
                }
                UiEffectEvent::Portrait { guid, highlight } => {
                    if highlight {
                        self.highlighted_portraits.insert(guid);
                    } else {
                        self.highlighted_portraits.remove(&guid);
                    }
                }
                UiEffectEvent::Relationship { guid, highlight } => {
                    // `0x4769ca` resolves the first `cWinRelationship` control
                    // without comparing neighbor identity, so a clear for any
                    // valid neighbor clears the single active highlight.
                    self.highlighted_relationships.clear();
                    if highlight {
                        self.highlighted_relationships.insert(guid);
                    }
                }
            }
        }
        keep(&mut self.ui_effects, effects, SOUNDS_KEPT);
    }

    fn stop(&mut self, error: &StepError) {
        let at = self.lot.ticks();
        self.stopped = Some(match error {
            StepError::Incomplete => "The load did not complete, so the lot cannot run.".into(),
            _ => format!("The lot stopped after tick {at}: {error}"),
        });
    }

    /// The status line across the top: why the lot stopped, else the last
    /// error caught for a while after it, then the last press's note.
    fn status(&self) -> Option<String> {
        let stopped = self
            .stopped
            .clone()
            .or_else(|| self.errors.recent(self.lot.ticks()));
        match (&stopped, &self.note) {
            (Some(stop), Some(note)) => Some(format!("{stop}  —  {note}")),
            (Some(line), None) | (None, Some(line)) => Some(line.clone()),
            (None, None) => None,
        }
    }

    /// Draws the frame: the lot, its people, then the panel. When the
    /// frame shows a new object under the pointer (the object moved, or
    /// the view did), its pie menu is listed, as when the pointer moves
    /// onto it, and the frame drawn again if the listing changed the lot.
    pub fn render(&mut self) -> &Canvas {
        self.draw();
        if self.follow() {
            self.prev_steady_key = None;
            self.present_damage = None;
            self.draw();
        }
        &self.canvas
    }

    /// Clears `lot_cache` while preserving the last drawn lot depth in
    /// `self.lot_depth` so `Game::under` stays exact until the next `draw()`.
    fn clear_lot_cache(&mut self) {
        if self.lot_depth_from_cache {
            if let Some(cached) = self.lot_cache.last_lot_depth() {
                self.lot_depth.clear();
                self.lot_depth.extend_from_slice(cached);
            }
            self.lot_depth_from_cache = false;
        }
        self.lot_cache.clear();
        self.prev_steady_key = None;
        self.present_damage = None;
    }

    /// Read-only view of the lot depth buffer for the frame last drawn
    /// (borrowing directly from [`LotCache::last_lot_depth`] on cached draws
    /// with zero per-frame copy, or falling back to `self.lot_depth`).
    pub fn lot_depth_slice(&self) -> &[u16] {
        if self.lot_depth_from_cache
            && let Some(cached) = self
                .lot_cache
                .last_lot_depth()
                .filter(|d| d.len() == self.canvas.width.saturating_mul(self.canvas.height))
        {
            return cached;
        }
        &self.lot_depth
    }

    /// Whether `Game::under` is currently borrowing lot depth directly from
    /// [`LotCache`] without a per-frame clone.
    pub fn lot_depth_from_cache(&self) -> bool {
        self.lot_depth_from_cache
    }

    /// Number of pixels currently stored in the sparse `people_ids` buffer.
    pub fn people_ids_stored_pixels(&self) -> usize {
        self.people_ids.stored_pixels()
    }

    /// Canvas-space dirty rects accumulated since the last successful window
    /// present, or `None` when a full blit/present is required.
    pub fn peek_present_damage(&self) -> Option<&[sims_render::canvas::Rect]> {
        self.present_damage.as_deref()
    }

    /// Marks the current canvas as successfully presented to the window
    /// backbuffer so subsequent steady-state frames can accumulate bounded
    /// partial damage relative to it.
    pub fn on_presented(&mut self) {
        self.present_damage = Some(Vec::new());
    }

    /// Forces the next present of this lot to perform a full blit (e.g. when a
    /// shell-level modal overlay was composited over the lot canvas).
    pub fn invalidate_present_damage(&mut self) {
        self.present_damage = None;
    }

    /// Draws the frame.
    fn draw(&mut self) {
        let [w, h] = self.view.size.map(|n| n.max(1) as usize);
        if (self.canvas.width, self.canvas.height) != (w, h) {
            self.canvas = Canvas::new(w, h, [0, 0, 0]);
        }
        let sell = self.buy_sell_info();
        let world = self.lot.world();
        // The cutaway also opens the room the local player's selected Sim is in.
        self.view.camera.focus.selected = if self.coop.is_active() {
            self.local_selected_sim()
                .and_then(|id| world.objects.get(&id))
                .map(|o| Selected {
                    tile: [o.position[0].div_euclid(16), o.position[1].div_euclid(16)],
                    level: o.position[2],
                    room: o.data.get(29).copied().unwrap_or(NO_ROOM),
                })
        } else {
            world.selected_whereabouts().map(|w| Selected {
                tile: [i32::from(w.tile[0]), i32::from(w.tile[1])],
                level: w.level,
                room: w.room.unwrap_or(NO_ROOM),
            })
        };
        let viewport = self.house_viewport_size();
        let render_options = sims_render::RenderOptions {
            shadows: self.hud.controls.options.graphics.shadows,
            lighting: self.hud.controls.options.graphics.lighting,
        };
        // The lot, then the people, then the objects that hold someone
        // (a bench a Sim sits on), as the original's frame does.
        let preview = self.buy_cursor.as_ref().map(|c| (c.cursor, c.held));
        let draw_shape = self.build_preview_shape.as_ref().unwrap_or(&self.lot_shape);
        self.lot_cache.draw_lot_with_preview_and_options(
            world,
            draw_shape,
            &mut self.assets,
            &self.view.camera,
            viewport,
            preview,
            render_options,
            &mut self.canvas,
        );
        let lot_hit = self.lot_cache.last_hit();
        self.drawn_camera = self.view.camera;
        if self
            .lot_cache
            .last_lot_depth()
            .is_some_and(|d| d.len() == self.canvas.width.saturating_mul(self.canvas.height))
        {
            self.lot_depth_from_cache = true;
        } else {
            self.lot_depth.clone_from(&self.canvas.depth);
            self.lot_depth_from_cache = false;
        }
        // The plumb bob spins by the game's clock in milliseconds (the
        // original's 0x6e54e0); here the time passed in, which runs on
        // while the lot is paused or a dialog waits (**unverified**
        // whether the original's does).
        self.drawn_us = self.clock_us;
        let milliseconds = (self.clock_us / 1000) as u32;
        self.people.set_clock(milliseconds);
        self.people_rects.clear();
        if self.open_pack_mode {
            crate::open_pack::draw_open_pack_sims(
                world,
                draw_shape,
                &self.view.camera,
                viewport,
                &mut self.canvas,
                &mut self.people_rects,
            );
        } else {
            self.people.draw_in_viewport_tracked(
                world,
                &self.view.camera,
                viewport,
                &mut self.canvas,
                &mut self.people_rects,
            );
        }
        for (&id, obj) in &world.objects {
            if (obj.person.is_some() || obj.guid == sims_people::bob::STANDARD)
                && let Some(rect) = self.canvas.drawn(id)
            {
                self.people_rects.push(rect);
            }
        }
        // Who the pick notes: the people as drawn, before a bench they sit
        // on draws over them, captured only within their tight drawn rects.
        self.people_ids.take_rects(&self.canvas, &self.people_rects);
        sims_render::draw_held_with_options(
            world,
            draw_shape,
            &mut self.assets,
            &self.view.camera,
            render_options,
            &mut self.canvas,
        );
        let mut cur_dynamic_rects = self.people_rects.clone();
        for id in sims_render::holds_person(world) {
            if let Some(rect) = self.canvas.drawn(id) {
                cur_dynamic_rects.push(rect);
            }
        }
        // Build Mode Part 2 previews (Terrain overlay, Water/Pool digging preview
        // with coping border, and isometric Roof preview). Never drawn during static
        // Live Mode frames when `roof_view` is false, keeping ratchet frames bit-exact.
        if self.hud.controls.mode == Mode::Build {
            if let Some(overlay) = self.terrain_hover_overlay {
                let _ = render_build::draw_terrain_preview(
                    &self.lot_shape,
                    Some(world),
                    &self.view.camera,
                    &overlay,
                    &mut self.canvas,
                );
            }
            if let Some(overlay) = self.water_hover_overlay {
                let _ = render_build::draw_water_preview(
                    &self.lot_shape,
                    Some(world),
                    &self.view.camera,
                    &overlay,
                    &mut self.canvas,
                );
            }
        }
        if self.roof_view
            || (self.hud.controls.mode == Mode::Build
                && self.hud.build_state.tool == Some(BuildTool::Roof))
        {
            self.draw_roof_preview();
        }
        if self.coop.is_active() {
            self.draw_coop_remote_indicators(viewport);
            self.draw_coop_buy_preview(viewport);
        }
        let selected_override = if self.coop.is_active() {
            let local_sel = self.local_selected_sim();
            (local_sel != self.lot.world().selected()).then_some(local_sel)
        } else {
            None
        };
        self.log_debug_note_if_new();
        self.hud.set_clock_us(self.clock_us);
        self.sync_special_panel();
        let status = self.debug.then(|| self.status()).flatten();
        let world = self.lot.world();
        // A dialog the lot waits on is in the lot as the tick left it when
        // it was shown. Read it before the pie menu so a newly opened dialog
        // suppresses the pie's dark backdrop and head on the frame it closes
        // the menu.
        let dialogs = self.lot.asking().map_or(world, |a| &a.world);
        self.hud.read_dialog(dialogs, &self.assets);
        // The pie menu shades the house viewport behind it (0x4a94ec →
        // 0x4ac8ff) and renders the Sim's 3D head tracking the pointer
        // (0x4a953e → 0x462290 → 0x462985) directly into the house viewport
        // before the 2D HUD draws, so oversized headwear is clipped only by
        // the house viewport (not the 201×201 face bitmap), the bottom control
        // strip hides anything below the viewport, and pie labels draw on top.
        if !self.hud.will_show_dialog(self.view.size) {
            self.hud.resize_menu(self.view.size);
            let pointer = self.hud.pointer();
            if let Some(menu) = &mut self.hud.menu {
                if let Some(backdrop) = menu.backdrop_at(milliseconds) {
                    crate::people::shade_pie_face_with_radius(
                        &mut self.canvas,
                        backdrop.center,
                        backdrop.radius,
                        viewport,
                    );
                }
                if let Some(center) = menu.main_center()
                    && !self.open_pack_mode
                {
                    let rendered = if let Some(pose) = menu.head_pose_at(pointer, milliseconds) {
                        self.people.head_in_viewport_pose(
                            world,
                            menu.person,
                            viewport,
                            center,
                            self.view.camera.zoom,
                            pose,
                        )
                    } else {
                        let look = menu.head_look(pointer);
                        self.people.head_in_viewport(
                            world,
                            menu.person,
                            viewport,
                            center,
                            self.view.camera.zoom,
                            look,
                        )
                    };
                    match rendered {
                        Ok((origin, head, depth)) => {
                            if head.width > 0 && head.height > 0 {
                                crate::people::composite_pie_head(
                                    &mut self.canvas,
                                    origin,
                                    &head,
                                    &depth,
                                    viewport,
                                );
                            }
                            menu.head_origin = origin;
                            menu.head = Some(head);
                            menu.head_depth = depth;
                        }
                        Err(e) => {
                            self.people
                                .note(vec![format!("the pie menu has no head: {e}")]);
                            menu.head_origin = [0, 0];
                            menu.head = None;
                            menu.head_depth.clear();
                        }
                    }
                }
            }
        }
        let was_ui_animating = self.ui_animating_prev;
        let blink_changed = self.hud.sync_clock_blink(world, self.clock_us);
        self.hud.sync_character_names(self.lot.character_names());
        self.blink_dirty = false;
        if self.hud.controls.mode == Mode::Build && self.hud.controls.expanded {
            self.hud.layout_build(
                self.view.size,
                world,
                &self.assets,
                self.lot.can_undo(),
                self.lot.can_redo(),
            );
            self.hud.ensure_build_swatches(&mut self.assets);
        }
        self.hud.draw_for_lot_at_with_selection(
            world,
            self.lot.residential(),
            self.lot.lot_zoning(),
            self.lot_visibility(),
            &self.assets,
            &self.view,
            &self.pace,
            status.as_deref(),
            sell.as_ref(),
            self.clock_us,
            selected_override,
            &mut self.canvas,
        );
        self.ui_animating_prev = self.hud.is_ui_animating(self.clock_us);
        draw_ui_effect_highlights(
            self.hud.items(),
            &self.highlighted_buttons,
            &self.highlighted_portraits,
            !self.highlighted_relationships.is_empty(),
            &mut self.canvas,
        );
        self.draw_build_cost_label();
        self.draw_shop_dialog();
        self.draw_book_dialog();
        self.draw_phonebook_dialog();
        self.draw_destination_picker();
        self.draw_coop_hud_and_lobby();
        // The help balloon closed by the selection's going.
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        let problems = self.hud.take_problems();
        self.people.note(problems);

        let steady_eligible = lot_hit == sims_render::CacheHit::Full
            && self.hud.controls.mode == Mode::Live
            && self.hud.controls.help.is_none()
            && !blink_changed
            && !was_ui_animating
            && !self.ui_animating_prev
            && !self.debug
            && !self.roof_view
            && self.terrain_hover_overlay.is_none()
            && self.water_hover_overlay.is_none()
            && self.build_preview_shape.is_none()
            && self.buy_cursor.is_none()
            && self.buy_selected.is_none()
            && !self.coop.is_active_or_starting()
            && !self.coop_lobby.is_open()
            && self.hud.menu.is_none()
            && self.hud.dialog().is_none()
            && !self.hud.has_ui_dialog()
            && self.lot.asking().is_none()
            && self.lot.asking_special_panel().is_none()
            && !self.hud.camera_strip_open()
            && !self.cheat_console_open()
            && !self.shop_dialog.is_open()
            && !self.book_dialog.is_open()
            && !self.phonebook_dialog.is_open()
            && self.destination_picker.is_none()
            && self.highlighted_buttons.is_empty()
            && self.highlighted_portraits.is_empty()
            && self.highlighted_relationships.is_empty()
            && sell.is_none();
        let cur_steady_key = if steady_eligible {
            let world = self.lot.world();
            Some(SteadyFrameKey {
                width: self.canvas.width,
                height: self.canvas.height,
                viewport,
                camera: self.view.camera,
                ticks: self.lot.ticks(),
                restore_generation: self.restore_generation,
                world_sound_rev: self.world_sound_rev,
                controls: self.hud.controls,
                selected: world.selected(),
                funds: world.budget.funds,
                paused: world.paused(),
                speed: world.speed(),
                return_speed: world.return_speed(),
                clock_nonzero: self.clock_us > 0,
                queue: self.hud.queue().clone(),
                hud_items: self.hud.items().to_vec(),
            })
        } else {
            None
        };
        if cur_steady_key.is_some() && self.prev_steady_key == cur_steady_key {
            if let Some(dmg) = &mut self.present_damage {
                dmg.extend_from_slice(&self.prev_dynamic_rects);
                dmg.extend_from_slice(&cur_dynamic_rects);
                if dmg.len() > 32 {
                    self.present_damage = None;
                }
            }
        } else {
            self.present_damage = None;
        }
        self.prev_dynamic_rects = cur_dynamic_rects;
        self.prev_steady_key = cur_steady_key;
    }

    /// The canvas last drawn.
    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// Passes the house view's screen-centre pick (`0x4a7b58` over the active
    /// house viewport [`Game::house_viewport_size`], at the original's origin
    /// column `camera.origin[0] + pick::ORIGIN_COLUMN`, cut to a tile via
    /// `0x433067`) into the simulation.
    fn sync_view_centre(&mut self) {
        if self.coop.is_active() {
            return;
        }
        let (w, h) = self.house_viewport_size();
        let mut camera = self.view.camera;
        camera.origin[0] = camera.origin[0].saturating_add(pick::ORIGIN_COLUMN);
        let centre =
            sims_render::screen_centre_pick(&self.lot_shape, &camera, [w as i32, h as i32])
                .map(|[x, y]| [(x >> 4) as i8, (y >> 4) as i8]);
        self.lot.set_view_centre(centre);
    }

    /// Passes the house view's rotation and screen-centre pick into the
    /// simulation.
    fn sync_view(&mut self) {
        if self.coop.is_active() {
            return;
        }
        self.lot.set_view_rotation(self.view.camera.rotation);
        self.sync_view_centre();
    }

    /// Resizes the canvas to `size` of the original's pixels.
    pub fn resize(&mut self, size: [i32; 2]) -> bool {
        if size == self.view.size {
            return false;
        }
        self.camera_pan = None;
        self.view.resize(size);
        self.sync_view_centre();
        self.hud.resize_menu(size);
        self.hud.relayout_dialog(size);
        self.hud.layout_camera_strip(size);
        if matches!(
            self.hud.controls.mode,
            Mode::Buy | Mode::Build | Mode::Options
        ) {
            let sell = self.buy_sell_info();
            self.hud.relayout_panel(
                size,
                self.lot.world(),
                self.lot.residential(),
                self.lot.lot_zoning(),
                self.lot_visibility(),
                &self.assets,
                sell.as_ref(),
            );
            if self.hud.controls.mode == Mode::Build {
                self.layout_build();
            }
        }
        true
    }

    /// Moves the camera. The view's rotation and screen-centre pick are the
    /// lot's inputs too ([`Lot::set_view_rotation`], [`Lot::set_view_centre`]),
    /// synced only when the picker's camera geometry actually changes so
    /// no-op or wall-mode commands preserve a restored pick.
    pub fn apply(&mut self, command: Move) -> bool {
        let cancelled_pan = self.camera_pan.take().is_some();
        let cleared_roof = matches!(command, Move::Walls(_)) && self.roof_view;
        if cleared_roof {
            self.roof_view = false;
            self.hud.set_roof_view(false);
        }
        let before = self.view.camera;
        let (w, h) = self.house_viewport_size();
        let viewport = [w as i32, h as i32];
        let moved = match command {
            Move::ZoomIn | Move::ZoomOut | Move::Rotate(_) if self.lot_shape.size > 0 => {
                let changed = native_zoom_or_rotate(
                    &self.lot_shape,
                    &mut self.view.camera,
                    viewport,
                    command,
                );
                self.view.bound();
                changed || self.view.camera != before
            }
            _ => self.view.apply_in(command, viewport, pick::ORIGIN_COLUMN),
        };
        let after = self.view.camera;
        if after.rotation != before.rotation && self.coop.is_active() {
            let _ = self.coop.submit_command(NetCommand::SetCameraRotation {
                rotation: CameraRotation::new(after.rotation),
            });
        }
        if cancelled_pan
            || after.origin != before.origin
            || after.zoom != before.zoom
            || after.rotation != before.rotation
            || after.level != before.level
        {
            self.sync_view();
        }
        moved || cleared_roof
    }

    /// Whether the panel takes presses at `point`.
    pub fn over_panel(&self, point: [i32; 2]) -> bool {
        self.coop_lobby.is_open()
            || self.coop_overlay_buttons.contains(point)
            || self.has_dialog()
            || self.hud.covers(point)
    }

    /// Whether `point` lies inside the canvas (`[0..width, 0..height)`).
    fn in_view(&self, point: [i32; 2]) -> bool {
        point[0] >= 0
            && point[1] >= 0
            && point[0] < self.view.size[0]
            && point[1] < self.view.size[1]
    }

    /// Moves the pointer to `point`. When it first reaches an object on
    /// the lot in Live mode, the selected Sim's pie menu on it is listed, as
    /// the original lists it then (0x48c834); listing changes the lot. In Buy
    /// mode, hovering over the house view activates and moves (or drag-rotates)
    /// the placement cursor (`0x487d3d`, `0x485a2e`), while moving onto the
    /// panel hides it (`0x487d4e`). A point outside the canvas is treated as
    /// [`Game::leave`].
    pub fn hover(&mut self, point: [i32; 2]) -> bool {
        if !self.in_view(point) {
            return self.leave();
        }
        self.pointer = Some(point);
        if self.coop_lobby.is_open() {
            let snap = self.coop_snapshot();
            let size = [
                self.view.size[0].max(1) as u32,
                self.view.size[1].max(1) as u32,
            ];
            self.coop_lobby
                .pointer_move(size, &snap, point[0], point[1]);
            return true;
        }
        if self.shop_dialog.is_open() {
            if let Some(snapshot) = &self.shop_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.shop_dialog
                    .pointer_move(size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if self.phonebook_dialog.is_open() {
            if let Some(snapshot) = &self.phonebook_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.phonebook_dialog
                    .pointer_move(size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if self.book_dialog.is_open() {
            if let Some(snapshot) = &self.book_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.book_dialog
                    .pointer_move(size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if let Some(picker) = &mut self.destination_picker {
            return picker.hover(self.view.size, point, self.clock_us);
        }
        let mut redraw = self.hud.hover(point);
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        redraw |= self.highlight(point);
        redraw |= self.refresh_mode_hover(point);
        // The cutaway follows the rooms the pointer passes over, while
        // the cutaway is on and the pointer is over the lot.
        if !self.hud.covers(point) && self.view.camera.walls == Walls::Cutaway {
            let camera = self.view.camera;
            self.view
                .camera
                .focus
                .point(&self.lot_shape, &camera, point);
            redraw |= self.view.camera.focus != camera.focus;
        }
        redraw
    }

    /// A left press at `point`, which is over the panel or the lot.
    ///
    /// While a pie menu is open (with no dialog over it), a press only
    /// updates which entry is under `point` and marks the button held
    /// (0x5eb3d1 with `+0x170` clear); the entry is chosen, or the menu
    /// closed, on the matching [`Game::release`] (0x5eb433).
    pub fn press(&mut self, point: [i32; 2]) -> Outcome {
        if self.coop_lobby.is_open() {
            self.pointer = Some(point);
            let snap = self.coop_snapshot();
            let size = [
                self.view.size[0].max(1) as u32,
                self.view.size[1].max(1) as u32,
            ];
            let changed = self
                .coop_lobby
                .pointer_down(size, &snap, point[0], point[1]);
            return Outcome {
                redraw: changed,
                ..Outcome::default()
            };
        }
        if self.coop_overlay_buttons.press(point) {
            self.pointer = Some(point);
            return Outcome::redraw();
        }
        if self.shop_dialog.is_open() {
            self.pointer = Some(point);
            if let Some(snapshot) = &self.shop_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                let changed = self
                    .shop_dialog
                    .pointer_down(size, snapshot, point[0], point[1]);
                return Outcome {
                    redraw: changed,
                    ..Outcome::default()
                };
            }
            return Outcome::default();
        }
        if self.phonebook_dialog.is_open() {
            self.pointer = Some(point);
            if let Some(snapshot) = &self.phonebook_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                let changed = self
                    .phonebook_dialog
                    .pointer_down(size, snapshot, point[0], point[1]);
                return Outcome {
                    redraw: changed,
                    ..Outcome::default()
                };
            }
            return Outcome::default();
        }
        if self.book_dialog.is_open() {
            self.pointer = Some(point);
            if let Some(snapshot) = &self.book_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                let changed = self
                    .book_dialog
                    .pointer_down(size, snapshot, point[0], point[1]);
                return Outcome {
                    redraw: changed,
                    ..Outcome::default()
                };
            }
            return Outcome::default();
        }
        if let Some(picker) = &mut self.destination_picker {
            self.pointer = Some(point);
            let changed = picker.press(self.view.size, point, self.clock_us);
            return Outcome {
                redraw: changed,
                ..Outcome::default()
            };
        }
        if !self.has_dialog() && self.hud.menu.is_some() {
            self.pointer = Some(point);
            let mut redraw = self.hud.hover(point);
            redraw |= self.highlight(point);
            if let Some(menu) = &mut self.hud.menu {
                redraw |= menu.press();
            }
            return Outcome {
                redraw,
                ..Outcome::default()
            };
        }
        let cleared = self.note.take().is_some();
        if self.has_dialog() && !self.hud.has_dialog() {
            // A newly arrived simulation dialog not yet drawn still blocks
            // clicks from reaching controls behind it.
            return Outcome::redraw();
        }
        if self.hud.covers(point) {
            // Native `0x50d04d` → `0x4cf97a` → `0x487e18`: clicking the Buy
            // catalog strip while holding a picked-up existing object sells
            // it; clicking empty strip background while holding an unpaid
            // preview cancels it.
            let over_camera_strip = self
                .hud
                .camera_strip_rect()
                .is_some_and(|r| r.contains(point[0], point[1]));
            if !self.has_dialog()
                && !over_camera_strip
                && self.hud.controls.mode == Mode::Buy
                && self.hud.over_buy_catalog(self.view.size, point)
                && self.buy_cursor.is_some()
                && (self.buy_cursor.as_ref().is_some_and(BuyCursor::is_existing)
                    || self.hud.buy_layout().press(point).is_none())
            {
                self.sell_held();
                return Outcome::redraw();
            }
            if !self.has_dialog()
                && !over_camera_strip
                && self.hud.controls.mode == Mode::Build
                && let Some(action) = self.hud.press_build_subtool(point)
            {
                self.apply_build_subtool(action);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                return Outcome::redraw();
            }
            // The menu, whose entries a choice is an index into.
            let menu = self.hud.menu.clone();
            let outcome = match self
                .hud
                .press(point, self.lot.world(), self.lot.residential())
            {
                Some(command) => self.command(command, menu.as_ref()),
                None => Outcome::redraw(),
            };
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return outcome;
        }
        // 0x51f3c5..0x51f3e5, 0x51d312, 0x51d354: pressing outside the live
        // control panel dismisses any open help balloon with `UI_whoosh`.
        let dismissed = self.hud.dismiss_help();
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        // Native `cWinCamera`: clicking inside the house viewport while Camera
        // Mode is open closes the Camera subpanel and captures a clean snapshot.
        if self.hud.camera_strip_open() {
            self.hud.close_camera_strip();
            self.hud.play(crate::hud::CLICK);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return Outcome {
                redraw: true,
                photo: true,
                ..Outcome::default()
            };
        }
        if self.hud.controls.mode == Mode::Buy {
            if self.stopped.is_some() {
                self.note = Some("The lot has stopped, so it takes no orders.".into());
                return Outcome::redraw();
            }
            if self.buy_selected.is_some()
                || self.buy_cursor.is_some()
                || self.coop_buy_preview.is_some()
            {
                self.pointer = Some(point);
                self.hover_buy(point);
                self.buy_dragging = Some(point);
                self.buy_commit_on_release = true;
                let valid = if self.coop.is_active() {
                    self.coop_buy_preview.is_some()
                } else {
                    self.buy_cursor.as_ref().is_some_and(|c| c.valid)
                };
                self.hud.play(if valid {
                    "UI_object_move_place"
                } else {
                    crate::hud::ERROR
                });
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                return Outcome::redraw();
            }
            // Hand Tool pickup of an existing lot object (`0x485f98` →
            // `0x486960` → `0x4215e6`).
            if let Some(target) = self.under(point) {
                if self.coop.is_active() {
                    return self.coop_pick_up_preview(target, point);
                }
                match self.lot.buy_pick_up(target) {
                    Ok(Some(cursor)) => {
                        let screen = lot_point(&self.view.camera, &self.lot_shape, cursor.position);
                        self.buy_grab_offset = [
                            point[0] - (screen[0] + pick::ORIGIN_COLUMN),
                            point[1] - screen[1],
                        ];
                        self.buy_cursor = Some(cursor);
                        self.buy_in_house = true;
                        self.buy_dragging = Some(point);
                        self.buy_commit_on_release = false;
                        self.pointer = Some(point);
                        self.hud.play("UI_object_move");
                        let sounds = self.hud.take_sounds();
                        self.keep_sounds(sounds);
                        self.refresh_world();
                        self.layout_buy();
                        return Outcome::redraw();
                    }
                    Ok(None) => {
                        self.hud.play(crate::hud::ERROR);
                        let sounds = self.hud.take_sounds();
                        self.keep_sounds(sounds);
                        return Outcome::redraw();
                    }
                    Err(e) => {
                        self.note = Some(format!("Object {target} was not picked up: {e}"));
                        self.hud.play(crate::hud::ERROR);
                        let sounds = self.hud.take_sounds();
                        self.keep_sounds(sounds);
                        return Outcome::redraw();
                    }
                }
            }
            return Outcome {
                redraw: cleared || dismissed,
                ..Outcome::default()
            };
        }
        if self.hud.controls.mode == Mode::Build {
            if self.stopped.is_some() {
                self.note = Some("The lot has stopped, so it takes no orders.".into());
                return Outcome::redraw();
            }
            if self.buy_selected.is_some()
                || self.buy_cursor.is_some()
                || self.coop_buy_preview.is_some()
            {
                self.pointer = Some(point);
                self.hover_buy(point);
                self.buy_dragging = Some(point);
                self.buy_commit_on_release = true;
                let valid = if self.coop.is_active() {
                    self.coop_buy_preview.is_some()
                } else {
                    self.buy_cursor.as_ref().is_some_and(|c| c.valid)
                };
                self.hud.play(if valid {
                    "UI_object_move_place"
                } else {
                    crate::hud::ERROR
                });
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                return Outcome::redraw();
            }
            if self.hud.build_state.tool == Some(BuildTool::Hand) {
                if let Some(target) = self.under(point) {
                    if self.coop.is_active() {
                        return self.coop_pick_up_preview(target, point);
                    }
                    match self.lot.buy_pick_up(target) {
                        Ok(Some(cursor)) => {
                            let screen =
                                lot_point(&self.view.camera, &self.lot_shape, cursor.position);
                            self.buy_grab_offset = [
                                point[0] - (screen[0] + pick::ORIGIN_COLUMN),
                                point[1] - screen[1],
                            ];
                            self.buy_cursor = Some(cursor);
                            self.buy_in_house = true;
                            self.buy_dragging = Some(point);
                            self.buy_commit_on_release = false;
                            self.pointer = Some(point);
                            self.hud.play("UI_object_move");
                            let sounds = self.hud.take_sounds();
                            self.keep_sounds(sounds);
                            self.refresh_world();
                            self.layout_build();
                            return Outcome::redraw();
                        }
                        Ok(None) => {
                            self.hud.play(crate::hud::ERROR);
                            let sounds = self.hud.take_sounds();
                            self.keep_sounds(sounds);
                            return Outcome::redraw();
                        }
                        Err(e) => {
                            self.note = Some(format!("Object {target} was not picked up: {e}"));
                            self.hud.play(crate::hud::ERROR);
                            let sounds = self.hud.take_sounds();
                            self.keep_sounds(sounds);
                            return Outcome::redraw();
                        }
                    }
                }
                return Outcome {
                    redraw: cleared || dismissed,
                    ..Outcome::default()
                };
            }
            return self.press_build(point);
        }
        if self.hud.controls.mode != Mode::Live {
            return Outcome {
                redraw: cleared || dismissed,
                ..Outcome::default()
            };
        }
        let Some(target) = self.under(point) else {
            return Outcome {
                redraw: cleared || dismissed,
                ..Outcome::default()
            };
        };
        self.open_menu(target, point);
        Outcome::redraw()
    }

    /// A left release at `point` (`0x5eb433` in Live mode, `0x4864d0` in Buy
    /// mode, `0x5e4599` for modal dialog buttons, `0x5f62ba` for Sound sliders).
    ///
    /// - Evaluates and ends any active Sound slider drag (`0x5f62ba..0x5f62de`).
    /// - Releases any held modal dialog button (`0x5e4599..0x5e45fc`), activating
    ///   it only when `point` is still inside that button.
    /// - Commits a Buy Mode drag or updates/chooses an open pie menu entry.
    pub fn release(&mut self, point: [i32; 2]) -> Outcome {
        if self.coop_lobby.is_open() {
            self.pointer = Some(point);
            let snap = self.coop_snapshot();
            let size = [
                self.view.size[0].max(1) as u32,
                self.view.size[1].max(1) as u32,
            ];
            let cmd = self.coop_lobby.pointer_up(size, &snap, point[0], point[1]);
            return match cmd {
                Some(cmd) => self.execute_coop_ui_command(cmd),
                None => Outcome::redraw(),
            };
        }
        if let Some(action) = self.coop_overlay_buttons.release(Some(point)) {
            self.hud.play(crate::hud::CLICK);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return self.execute_coop_overlay_action(action);
        }
        if self.shop_dialog.is_open() {
            self.pointer = Some(point);
            let cmd = self.shop_snapshot.as_ref().and_then(|snap| {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.shop_dialog.pointer_up(size, snap, point[0], point[1])
            });
            return match cmd {
                Some(cmd) => self.execute_shop_command(cmd),
                None => Outcome::redraw(),
            };
        }
        if self.phonebook_dialog.is_open() {
            self.pointer = Some(point);
            let cmd = self.phonebook_snapshot.as_ref().and_then(|snap| {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.phonebook_dialog
                    .pointer_up(size, snap, point[0], point[1])
            });
            return match cmd {
                Some(cmd) => self.execute_phonebook_command(cmd),
                None => Outcome::redraw(),
            };
        }
        if self.book_dialog.is_open() {
            self.pointer = Some(point);
            let cmd = self.book_snapshot.as_ref().and_then(|snap| {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.book_dialog.pointer_up(size, snap, point[0], point[1])
            });
            return match cmd {
                Some(BookCommand::PageChanged(_)) => {
                    self.hud.play(crate::hud::CLICK);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    Outcome::redraw()
                }
                Some(BookCommand::ScrollLines(delta)) => {
                    if self.book_scroll_lines(delta) {
                        self.hud.play(crate::hud::CLICK);
                        let sounds = self.hud.take_sounds();
                        self.keep_sounds(sounds);
                    }
                    Outcome::redraw()
                }
                Some(BookCommand::Close) => self.book_close(),
                None => Outcome::redraw(),
            };
        }
        if let Some(picker) = &mut self.destination_picker {
            self.pointer = Some(point);
            let action = picker.release(self.view.size, point, self.clock_us);
            return match action {
                Some(DestinationPickerAction::SelectLot(lot_id)) => {
                    if self.answer_special_panel(SpecialPanelResponse::Lot(Some(lot_id as i16))) {
                        self.hud.play(crate::hud::CLICK);
                    } else {
                        self.hud.play(crate::hud::ERROR);
                    }
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    Outcome::redraw()
                }
                Some(DestinationPickerAction::Cancel) => self.destination_picker_escape(),
                Some(DestinationPickerAction::InvalidLot) => {
                    self.hud.play(crate::hud::ERROR);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    Outcome::redraw()
                }
                Some(DestinationPickerAction::Clicked) => {
                    self.hud.play(crate::hud::CLICK);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    Outcome::redraw()
                }
                Some(DestinationPickerAction::Redraw) | None => Outcome::redraw(),
            };
        }
        let released_slider = self.hud.release_slider(Some(point));
        let (released_saves, open_saves) = self.hud.release_saves_button(Some(point));
        if open_saves {
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return Outcome {
                redraw: true,
                open_save_manager: true,
                ..Outcome::default()
            };
        }
        let (released_coop, open_coop) = self.hud.release_coop_button(Some(point));
        if open_coop {
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            self.open_coop_lobby(None);
            return Outcome {
                redraw: true,
                open_coop_lobby: true,
                ..Outcome::default()
            };
        }
        let (released_camera, camera_cmd) = self
            .hud
            .release_camera_strip((!self.has_dialog()).then_some(point));
        if let Some(cmd) = camera_cmd {
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return match cmd {
                CameraCommand::TakeSnapshot => Outcome {
                    redraw: true,
                    photo: true,
                    ..Outcome::default()
                },
                CameraCommand::OpenPhotoAlbum => Outcome {
                    redraw: true,
                    open_photo_album: true,
                    ..Outcome::default()
                },
                CameraCommand::SetSize(_)
                | CameraCommand::SetQuality(_)
                | CameraCommand::Help(_) => Outcome::redraw(),
            };
        }
        let (released_action, action_cmd) = self.hud.release_options_action(Some(point));
        if let Some(command) = action_cmd {
            let outcome = self.command(command, None);
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return outcome;
        }
        let (released_dialog, dialog_cmd) = self.hud.release_dialog(Some(point));
        if let Some(command) = dialog_cmd {
            let outcome = self.command(command, None);
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return outcome;
        }
        if !self.in_view(point) {
            let mut out = self.release_outside();
            out.redraw |= released_slider
                || released_saves
                || released_coop
                || released_camera
                || released_action
                || released_dialog;
            return out;
        }
        if matches!(self.hud.controls.mode, Mode::Buy | Mode::Build)
            && let Some(anchor) = self.buy_dragging
        {
            // Dropping a held object onto an open Camera sub-strip cancels the
            // drag safely without committing a placement behind the HUD.
            if self
                .hud
                .camera_strip_rect()
                .is_some_and(|r| r.contains(point[0], point[1]))
            {
                self.buy_dragging = None;
                self.buy_commit_on_release = false;
                return Outcome::redraw();
            }
            // Dropping a held object onto the Buy catalog strip sells it
            // (`0x50d04d` → `0x4cf97a` → `0x487e18`).
            if self.hud.controls.mode == Mode::Buy
                && !self.hud.camera_strip_open()
                && self.hud.over_buy_catalog(self.view.size, point)
                && (self.buy_cursor.is_some() || self.coop_buy_preview.is_some())
            {
                self.buy_dragging = None;
                self.buy_commit_on_release = false;
                self.sell_held();
                return Outcome::redraw();
            }
            // Native `0x4864f0` invokes `OnMouseMove` with the release
            // coordinates before evaluating the drag threshold (`dx^2 + dy^2
            // >= 4`) and `[tool + 0x81]` commit flag.
            self.pointer = Some(point);
            self.hover_buy(point);
            self.buy_dragging = None;
            let dx = point[0] - anchor[0];
            let dy = point[1] - anchor[1];
            let dragged = dx * dx + dy * dy >= 4;
            let should_commit = self.buy_commit_on_release || dragged;
            self.buy_commit_on_release = false;
            if should_commit {
                return self.commit_buy(point);
            }
            return Outcome::redraw();
        }
        if self.hud.controls.mode == Mode::Build && self.build_drag.is_some() {
            return self.release_build(point);
        }
        if self.has_dialog() || self.hud.menu.is_none() {
            return Outcome {
                redraw: released_slider
                    || released_saves
                    || released_coop
                    || released_camera
                    || released_action
                    || released_dialog,
                ..Outcome::default()
            };
        }
        self.pointer = Some(point);
        let mut redraw = self.hud.hover(point);
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        redraw |= self.highlight(point);
        let now_ms = (self.clock_us / 1_000) as u32;
        let will_choose = self.hud.menu.as_ref().is_some_and(|m| m.will_choose(point));
        if !will_choose {
            let was_held = self.hud.menu.as_ref().is_some_and(|m| m.held());
            let (release, command) = self.hud.release_menu_at(point, now_ms);
            debug_assert_eq!((release, command), (menu::Release::Settled, None));
            return Outcome {
                redraw: redraw || was_held,
                ..Outcome::default()
            };
        }
        let _ = self.note.take();
        // Choosing an entry checks it again first (0x5eb433 -> 0x5eb62a
        // -> event 0xe -> 0x48df28).
        if let Some(entry) = self.hud.menu.as_ref().and_then(|m| m.highlighted(point)) {
            self.check_entry(entry);
        }
        // A failed check's note stays in front of what the choice says.
        let failed = self.note.take();
        let menu = self.hud.menu.clone();
        let (release, command) = self.hud.release_menu_at(point, now_ms);
        if release == menu::Release::Stays
            && let Some(warped) = self.hud.menu.as_ref().and_then(Menu::pending_pointer)
        {
            self.pointer = Some(warped);
        }
        self.highlighted = None;
        let outcome = match command {
            Some(command) => {
                if let Command::ChooseInteraction { sim, .. } = &command {
                    let origin = menu.as_ref().and_then(Menu::main_center).unwrap_or(point);
                    self.hud.set_queue_origin(Some(*sim), origin);
                }
                self.command(command, menu.as_ref())
            }
            None => Outcome::redraw(),
        };
        if let Some(failed) = failed {
            self.note = Some(match self.note.take() {
                Some(note) => format!("{failed} {note}"),
                None => failed,
            });
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        outcome
    }

    /// A left release while the pointer is outside the window (after
    /// [`Game::leave`]): ends slider/dialog capture without activating, cancels
    /// a Buy Mode drag, settles an opening press on a moved menu, or closes the
    /// pie without checking or choosing any entry.
    pub fn release_outside(&mut self) -> Outcome {
        let _ = self.coop_overlay_buttons.release(None);
        if let Some(picker) = &mut self.destination_picker {
            let redraw = picker.leave(self.clock_us);
            return Outcome {
                redraw,
                ..Outcome::default()
            };
        }
        let released_slider = self.hud.release_slider(None);
        let (released_saves, _) = self.hud.release_saves_button(None);
        let (released_coop, _) = self.hud.release_coop_button(None);
        let (released_camera, _) = self.hud.release_camera_strip(None);
        let (released_action, _) = self.hud.release_options_action(None);
        let (released_dialog, _) = self.hud.release_dialog(None);
        if self.buy_dragging.take().is_some() {
            self.buy_commit_on_release = false;
            let redraw = self.hide_buy_cursor()
                || released_slider
                || released_saves
                || released_coop
                || released_camera
                || released_action
                || released_dialog;
            return Outcome {
                redraw,
                ..Outcome::default()
            };
        }
        if self.build_drag.take().is_some() {
            self.build_preview_shape = None;
            let _ = self.hide_build_cursor();
            return Outcome::redraw();
        }
        if self.has_dialog() || self.hud.menu.is_none() {
            return Outcome {
                redraw: released_slider
                    || released_saves
                    || released_coop
                    || released_camera
                    || released_action
                    || released_dialog,
                ..Outcome::default()
            };
        }
        self.pointer = None;
        self.highlighted = None;
        let redraw = self.hud.leave();
        let was_held = self.hud.menu.as_ref().is_some_and(|m| m.held());
        match self.hud.release_menu_outside() {
            menu::Release::Settled => Outcome {
                redraw: redraw || was_held,
                ..Outcome::default()
            },
            menu::Release::Closed => {
                let _ = self.note.take();
                Outcome::redraw()
            }
            menu::Release::Chose(_) | menu::Release::Stays => unreachable!(),
        }
    }

    /// The pointer left the window: it is on no object, button, portrait
    /// or pie entry, so coming back onto an entry checks it again. True
    /// if the frame changes.
    pub fn leave(&mut self) -> bool {
        self.pointer = None;
        self.highlighted = None;
        if let Some(picker) = &mut self.destination_picker {
            return picker.leave(self.clock_us);
        }
        let mut redraw = self.hud.leave();
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        if self.hud.controls.mode == Mode::Buy {
            if self.buy_dragging.is_none() {
                redraw |= self.hide_buy_cursor();
            }
            redraw
        } else if self.hud.controls.mode == Mode::Build {
            if self.buy_dragging.is_none() {
                redraw |= self.hide_buy_cursor();
            }
            if self.build_drag.is_none() {
                redraw |= self.hide_build_cursor();
            }
            redraw
        } else {
            self.follow() || redraw
        }
    }

    /// Follows the object under the pointer in the frame last drawn: when
    /// it changes to an object, lists the selected Sim's pie menu on it.
    /// Nothing is listed while the pointer is over the panel, a dialog or
    /// a menu, while a modal is up (the lot behind a modal takes no
    /// pointer, and listing runs check trees that change the world), or
    /// once the lot has stopped. True if a listing may have changed the lot.
    fn follow(&mut self) -> bool {
        if self.hud.controls.mode != Mode::Live || self.has_dialog() || self.shop_dialog.is_open() {
            self.reached = None;
            self.listed = None;
            return false;
        }
        let under = match self.pointer {
            Some(point) if self.stopped.is_none() && !self.hud.covers(point) => self.under(point),
            _ => None,
        };

        if under == self.reached {
            return false;
        }
        self.reached = under;
        self.listed = None;
        under.is_some_and(|object| self.list(object))
    }

    /// For whom and on what the pie menu was listed when the pointer
    /// reached an object, until a click opens it or the pointer leaves.
    pub fn listed(&self) -> Option<(u16, u16)> {
        self.listed
            .as_ref()
            .map(|(person, object, _)| (*person, *object))
    }

    /// Checks again the action of the open pie menu's entry the pointer
    /// at `point` has reached, as the original does each time the pointer
    /// reaches an entry (0x48df28, `Lot::highlight`), which changes the
    /// lot. The action the check leaves is the one choosing the entry
    /// queues. True if the check ran.
    fn highlight(&mut self, point: [i32; 2]) -> bool {
        let Some(menu) = &self.hud.menu else {
            self.highlighted = None;
            return false;
        };
        let entry = menu.highlighted(point);
        if entry == self.highlighted {
            return false;
        }
        self.highlighted = entry;
        entry.is_some_and(|i| self.check_entry(i))
    }

    /// Checks again the action of the open pie menu's entry `i`
    /// (0x48df28, `Lot::highlight`), keeping the action the check leaves
    /// for choosing it. True if the check ran.
    fn check_entry(&mut self, i: usize) -> bool {
        if self.coop.is_active() {
            // Co-op never mutates the shared simulation on local pie hover;
            // `Lot::apply_coop_batch` re-validates on a throwaway probe when
            // `SimActionPick` executes deterministically on every peer.
            return false;
        }
        let Some(menu) = &self.hud.menu else {
            return false;
        };
        let Some(action) = menu.entries.get(i).and_then(|e| e.action.clone()) else {
            return false;
        };
        if self.stopped.is_some() {
            return false;
        }
        let person = menu.person;
        match self.lot.highlight(person, action) {
            Ok(checked) => {
                self.keep_sounds(checked.sounds);
                if let Some(entry) = self.hud.menu.as_mut().and_then(|m| m.entries.get_mut(i)) {
                    entry.action = Some(checked.action);
                }
            }
            Err(e) => self.note = Some(format!("The entry's check did not run: {e}")),
        }
        true
    }

    /// The person or object the pointer at `point` is on in the frame
    /// last drawn, by the original's pointer pick ([`pick::at`]).
    pub fn under(&self, point: [i32; 2]) -> Option<u16> {
        pick::at(
            &self.lot,
            &self.drawn_camera,
            &self.canvas,
            self.lot_depth_slice(),
            &self.people_ids,
            point,
        )
    }

    /// Lists the selected Sim's pie menu on `object` (`Lot::pie_menu`,
    /// which changes the lot), keeping its sounds for the host. True if
    /// the simulation listed it, and so may have changed the lot.
    fn list(&mut self, object: u16) -> bool {
        if self.coop.is_active() {
            let Some(player_id) = self.coop.local_player_id() else {
                return false;
            };
            let Some(person) = self.local_selected_sim() else {
                return false;
            };
            let listing = menu::coop_entries(&self.lot, &self.assets, player_id, object);
            if let Ok(listing) = &listing {
                self.keep_sounds(listing.sounds.clone());
            }
            self.listed = Some((person, object, listing));
            // Throwaway co-op pie probe never mutates the shared lot.
            return false;
        }
        let Some(person) = self.lot.world().selected() else {
            return false;
        };
        let listing = menu::entries(&mut self.lot, &self.assets, person, object);
        let changed = listing.as_ref().is_ok_and(|l| l.unchecked.is_none());
        if !listing.as_ref().is_ok_and(|l| l.unchecked.is_some()) {
            // `Lot::pie_menu` commits check-tree side effects before `menu::named`
            // runs, so invalidate steady-state damage even if naming returns `Err`.
            self.prev_steady_key = None;
            self.present_damage = None;
        }
        if let Ok(listing) = &listing {
            self.keep_sounds(listing.sounds.clone());
        }
        self.listed = Some((person, object, listing));
        changed
    }

    /// Opens the pie menu of `object` at `point` for the selected Sim:
    /// the one listed when the pointer reached it, or, when it was not
    /// listed then (the pointer did not move onto it first), one listed
    /// now.
    fn open_menu(&mut self, object: u16, point: [i32; 2]) {
        if self.stopped.is_some() {
            self.note = Some("The lot has stopped, so it takes no orders.".into());
            return;
        }
        let Some(person) = self.local_selected_sim() else {
            self.note = Some("Nobody is selected to do anything.".into());
            return;
        };
        // The press lists the menu again: the live mode's press handler
        // (0x48cbec) first sets the object under the pointer to none
        // (0x48c834 with 0), so the press's own pick lists it anew.
        self.list(object);
        self.reached = Some(object);
        let Some((_, _, listing)) = self.listed.take() else {
            return;
        };
        match listing {
            Ok(listing) => {
                // Where the simulation cannot list the menu yet, the note
                // says so and why.
                let why = listing
                    .unchecked
                    .map(|reason| format!("the simulation cannot list this menu yet: {reason}"));
                if listing.entries.is_empty() {
                    let line = format!("Object {object} offers nothing to do");
                    self.note = Some(match why {
                        Some(why) => format!("{line} ({why})."),
                        None => format!("{line}."),
                    });
                } else {
                    self.note = why.map(|why| format!("Every entry is shown, as {why}"));
                    self.highlighted = None;
                    let menu = if self.pie_animations_active() {
                        Menu::new_animated(
                            person,
                            object,
                            listing.entries,
                            point,
                            &self.hud.art,
                            self.view.size,
                            (self.clock_us / 1_000) as u32,
                        )
                    } else {
                        Menu::new(
                            person,
                            object,
                            listing.entries,
                            point,
                            &self.hud.art,
                            self.view.size,
                        )
                    };
                    // Native menu setup reads the current cursor (0x5ebd05)
                    // after any on-screen clamp warp (0x5ebab2) and updates
                    // hit state (0x5ebd29), so the opening frame aims the head
                    // at the press or warped center rather than a stale hover.
                    let effective = menu.pending_pointer().unwrap_or(point);
                    self.pointer = Some(effective);
                    self.hud.set_pointer(effective);
                    self.hud.menu = Some(menu);
                }
            }
            Err(e) => self.note = Some(format!("Object {object}'s menu is not listed: {e}")),
        }
    }

    /// The control panel as it was last laid out, from the world as it
    /// was when the frame was drawn ([`Game::render`]).
    pub fn panel(&self) -> &[sims_ui::hud::Item] {
        self.hud.items()
    }

    /// What the control panel shows of `world`: [`Game::render`] lays
    /// the panel out from this for the lot's world.
    pub fn snapshot(&self, world: &sims_sim::World) -> sims_ui::household::Snapshot {
        self.hud.snapshot(world, self.lot.residential())
    }

    /// The first name the panel shows for people of type `guid`.
    pub fn name(&self, guid: u32) -> Option<&str> {
        self.hud.name(guid)
    }

    /// The pie menu, while one is open. Its labels are placed when the
    /// game is rendered.
    pub fn menu(&self) -> Option<&Menu> {
        self.hud.menu.as_ref()
    }

    /// Where the window is to move the pointer (canvas pixels), once:
    /// the original moves it to the middle of a pie menu that opened
    /// elsewhere than asked, and of a submenu that opens.
    pub fn take_pointer(&mut self) -> Option<[i32; 2]> {
        self.hud.take_pointer()
    }

    /// Closes the pie menu; false if none was open.
    pub fn close_menu(&mut self) -> bool {
        let closed = self.hud.menu.take().is_some();
        if closed {
            self.highlighted = None;
        }
        closed
    }

    /// Handles Escape in Options mode (`cWinOptions::OnKeyDown`,
    /// `0x517904..0x517954`): closes any open Options subpanel tab (and help
    /// balloon) and consumes Escape so pressing Escape in Options never exits
    /// the game directly. Returns `false` outside Options mode.
    pub fn escape_options(&mut self) -> bool {
        if self.hud.controls.mode != Mode::Options {
            return false;
        }
        if self.hud.controls.options.tab.take().is_some() {
            // `0x5175ab..0x5175bb -> 0x50fb7f`: closing a tab also dismisses
            // any open Options help balloon with `UI_whoosh` before `UI_click`.
            if self.hud.controls.options.help.take().is_some() {
                self.hud.play(crate::hud::WHOOSH);
            }
            self.hud.play(crate::hud::CLICK);
            self.hud.relayout_panel(
                self.view.size,
                self.lot.world(),
                self.lot.residential(),
                self.lot.lot_zoning(),
                self.lot_visibility(),
                &self.assets,
                None,
            );
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
        } else if self.hud.controls.options.help.take().is_some() {
            self.hud.play(crate::hud::WHOOSH);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
        }
        true
    }

    /// Queues `entry` of the pie menu on `object` for `person`, as
    /// choosing it does: the action the simulation offered
    /// (`Lot::pick`, on the action's caller), or for a fallback entry the
    /// entry's menu number (`Lot::queue_interaction`).
    fn order(&mut self, person: u16, object: u16, entry: &menu::Entry) {
        let label = &entry.label;
        // A picked action is queued on its caller: `person`, but for the
        // offer to join `person`'s running interaction.
        let on = entry
            .action
            .as_ref()
            .and_then(|a| u16::try_from(a.caller).ok())
            .unwrap_or(person);
        if self.coop.is_active() {
            let cmd = match &entry.action {
                Some(action) => NetCommand::SimActionPick {
                    sim_id: action.caller as u16,
                    target_id: action.target as u16,
                    menu_index: action.table_index as i16,
                    joining: action.join != 0,
                    args: action.args,
                },
                None => NetCommand::SimActionPick {
                    sim_id: person,
                    target_id: object,
                    menu_index: entry.menu as i16,
                    joining: false,
                    args: [0; 4],
                },
            };
            self.note = Some(match self.coop.submit_command(cmd) {
                Ok(()) => format!("Sent \"{label}\" for person {on}."),
                Err(e) => format!("\"{label}\" was not sent: {e}"),
            });
            return;
        }
        let queued = match &entry.action {
            Some(action) => self.lot.pick(person, action.clone()),
            None => self.lot.queue_interaction(person, object, entry.menu),
        };
        self.note = Some(match queued {
            Ok(Some(_)) => {
                self.dirty = true;
                format!("Queued \"{label}\" for person {on}.")
            }
            // A full queue, an entry gone from the table, or a check
            // that refused an action run at once.
            Ok(None) => format!("\"{label}\" was not queued for person {on}."),
            Err(e) => format!("\"{label}\" was not queued: {e}"),
        });
    }

    /// Cancels `sim`'s action numbered `action`, as a press on its icon
    /// does (`Lot::cancel`, 0x4ea3e3), keeping the sounds the trees made,
    /// then plays `UI_queue_delete` if the press counts and `UI_error` if
    /// not (0x4cb16e). A cancel the lot refuses, or a stopped lot, leaves
    /// the icon to take another press.
    fn cancel(&mut self, sim: u16, action: i32) {
        if self.stopped.is_some() {
            self.note = Some("The lot has stopped, so it takes no orders.".into());
            self.hud.release(action);
            return;
        }
        if self.coop.is_active() {
            let cmd = NetCommand::SimActionCancel {
                sim_id: sim,
                interaction_uid: sims_vm::coop::cancel_uid_to_wire(action),
            };
            match self.coop.submit_command(cmd) {
                Ok(()) => self.hud.play(crate::hud::QUEUE_DELETE),
                Err(e) => {
                    self.hud.release(action);
                    self.note = Some(format!("The action cancel was not sent: {e}"));
                }
            }
            return;
        }
        match self.lot.cancel(sim, action) {
            Ok(cancelled) => {
                if cancelled.counts {
                    self.dirty = true;
                }
                self.keep_sounds(cancelled.sounds);
                self.hud.play(if cancelled.counts {
                    crate::hud::QUEUE_DELETE
                } else {
                    crate::hud::ERROR
                });
            }
            Err(e) => {
                // The lot is as it was, so the icon takes another press.
                self.hud.release(action);
                self.note = Some(format!("The action was not cancelled: {e}"));
            }
        }
    }

    /// The selected Sim's actions as the frame last drawn showed them.
    pub fn queue(&self) -> &sims_ui::queue::Queue {
        self.hud.queue()
    }

    /// Answers the dialog `caller` shows with its button `button` (0 the
    /// first): the answer the button gives (`Shown::answers`) goes to the
    /// lot, and the tick that showed it runs again at once.
    pub fn answer(&mut self, caller: u16, button: u8) {
        let Some(asking) = self.lot.asking() else {
            self.note = Some("No dialog waits for an answer.".into());
            return;
        };
        if asking.shown.object != caller {
            self.note = Some(format!(
                "The dialog shown is object {}'s, not object {caller}'s.",
                asking.shown.object
            ));
            return;
        }
        let Some(&answer) = asking.shown.answers.get(usize::from(button)) else {
            self.note = Some(format!("The dialog has no button {button}."));
            return;
        };
        let text = self.hud.active_dialog_entry().map(str::to_owned);
        if self.coop.is_active() {
            let dialog_id = self.lot.coop().and_then(|c| c.active_dialog).or_else(|| {
                self.coop
                    .session()
                    .and_then(|s| s.open_dialogs.keys().next().copied())
            });
            let Some(dialog_id) = dialog_id else {
                self.note = Some("No co-op blocking dialog is active.".into());
                return;
            };
            let cmd = NetCommand::AnswerBlockingDialog {
                dialog_id,
                answer: sims_net::DialogAnswer {
                    status: answer,
                    bitmap_id: Some(sims_ui::dialog::BUTTON as i16),
                    text,
                },
            };
            if let Err(e) = self.coop.submit_command(cmd) {
                self.note = Some(format!("The answer was not sent: {e}"));
            } else {
                self.hud.clear_vm_dialog();
            }
            return;
        }
        if let Err(e) = self.lot.answer(sims_vm::Answer {
            status: answer,
            bitmap_id: Some(sims_ui::dialog::BUTTON as i16),
            text,
        }) {
            self.note = Some(format!("The answer was not taken: {e}"));
            return;
        }
        // `0x523825..0x523887`: clear the cached VM dialog before replaying the
        // tick so a consecutive prompt with the same caller & operands starts
        // with a freshly initialized entry field.
        self.hud.clear_vm_dialog();
        // The tick that showed the dialog runs again now, with the answer,
        // so nothing is listed or ordered on the lot as it was before it.
        self.run(1);
    }

    /// A press on the portrait of the Sim `sim` (0x4cf9c4). On another
    /// Sim's portrait it selects `sim` (`Lot::select`, through 0x4e989b
    /// and 0x445449), keeping the sounds that made, and drops a menu
    /// listed for the Sim selected before. On the selected Sim's own
    /// portrait it centres the view on the Sim instead
    /// ([`Game::centre_on`]). Either way it then plays `UI_click`
    /// (0x4cfa51).
    ///
    /// On a residential lot 0x4e989b selects the person only when person
    /// word 32 is 0 (it lives here), and nobody otherwise; the portraits
    /// show only the household's people, whose word is 0, so that is not
    /// checked here. Not ported: a person the household's table names
    /// but who is not on the lot (0x4e9652), and the people panel's call
    /// on a press on the selected Sim's portrait (0x51ea7a, behind a
    /// check of global 0x6f6bac) whose effect has not been read.
    pub fn select(&mut self, sim: u16) {
        let world = self.lot.world();
        let current_selected = self.local_selected_sim();
        let same_neighbor = current_selected == Some(sim)
            || current_selected
                .and_then(|id| world.objects.get(&id))
                .zip(world.objects.get(&sim))
                .is_some_and(|(selected, target)| selected.guid == target.guid);
        if same_neighbor {
            // 0x4cfa0d..0x4cfa47: centres on the first live person of that
            // neighbor in person-registry order (`0x4e9608`).
            let target = world
                .objects
                .get(&sim)
                .and_then(|o| sims_ui::household::first_person_for_guid(world, o.guid))
                .map_or(sim, |o| o.id);
            let smooth = self.smooth_pan_active();
            self.centre_on_with_pan(target, smooth);
            self.hud.play(crate::hud::CLICK);
            return;
        }
        if self.coop.is_active() {
            match self
                .coop
                .submit_command(NetCommand::SelectPerson { sim_id: Some(sim) })
            {
                Ok(()) => {
                    self.close_menu();
                    self.reached = None;
                    self.listed = None;
                    self.highlighted = None;
                    self.hud.play(crate::hud::CLICK);
                }
                Err(e) => {
                    self.note = Some(format!("Person {sim} was not selected: {e}"));
                }
            }
            return;
        }
        match self.lot.select(sim) {
            Ok(sounds) => {
                self.dirty = true;
                self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                self.keep_sounds(sounds);
                self.hud.play(crate::hud::CLICK);
            }
            Err(e) => self.note = Some(format!("Person {sim} was not selected: {e}")),
        }
        self.listed = None;
        self.reached = None;
    }

    /// Centres the view on the object `id` as 0x434ced does with 1, but
    /// at once: the original slides the view there over a while
    /// (0x4c9a27, the time at 0x6c7788), which is left out.
    ///
    /// Nothing happens unless object data word 34 is 0 and word 29 is not
    /// negative. For a multi-tile object, `0x4071a3` walks the group from
    /// its lead tile through `next_parts` and averages the parts' `x` and
    /// `y` positions before rotating and projecting. The original's house
    /// view sits above the 100-pixel live-mode strip
    /// (`0x49ae5e` → `0x5cf1b6`, `0x4b136b`), so its middle is
    /// `[width / 2, height / 2]` over [`Game::house_viewport_size`] (full
    /// canvas while the strip is collapsed, `height - 100` while expanded).
    /// A person on the floor shown is
    /// centred a little above its feet: its screen point on the lot's
    /// terrain ([`sound::object_point_on_lot`]) raised by half of 2 √6
    /// times 2.0 times 2 to the power of the zoom plus one, cut to a whole
    /// number (19, 39 and 78 pixels from far to near), comes to the house
    /// view's middle (0x4a7777). Anything else, or a person on another
    /// floor, shows its floor (0x4a80c7) and, unless it stands off the
    /// lot's rotated tile bounds (0x4a7af0), its screen point on that
    /// floor comes to the middle (0x4a7996). Both paths then clamp the
    /// origin to the scroll limits (0x4a9fa9) and align its x coordinate
    /// with `(x + 1) & !3` (0x4a81cc).
    pub fn centre_on(&mut self, id: u16) {
        self.centre_on_with_pan(id, false);
    }

    fn centre_on_with_pan(&mut self, id: u16, smooth: bool) {
        let world = self.lot.world();
        let Some(object) = world.objects.get(&id) else {
            return;
        };
        let word = |i: usize| object.data.get(i).copied().unwrap_or(0);
        if word(34) != 0 || word(29) < 0 {
            return;
        }
        let is_person = object.person.is_some();
        let Some(position) = centre_position(world, id) else {
            return;
        };
        let level = position[2];
        let (w, h) = self.house_viewport_size();
        let view_size = [w as i32, h as i32];
        if is_person && level == self.view.camera.level {
            let mut at = lot_point(&self.view.camera, &self.lot_shape, position);
            at[1] -= person_rise(self.view.camera.zoom.level());
            self.centre_screen_point_with_pan(at, view_size, smooth);
            return;
        }
        if self.view.apply(Move::Floor(level)) {
            self.sync_view();
        }
        if !self.on_lot(position) {
            return;
        }
        let at = lot_point(&self.view.camera, &self.lot_shape, position);
        self.centre_screen_point_with_pan(at, view_size, smooth);
    }

    /// The active 3D house viewport size (`0x49ae5e`, `0x4f4b43`, `0x4d363b`,
    /// `0x53b2eb` → `0x4b136b` → `0x49b0bf` → `0x5cf1b6`): full canvas
    /// `(width, height)` while the bottom strip is closed, and
    /// `(width, height - STRIP)` (`height - 150 + 50` from the shared
    /// 150-pixel panel container at `0x4f4b43`) while the strip is open in
    /// Live, Build, Buy, or Options mode.
    pub fn house_viewport_size(&self) -> (u32, u32) {
        viewport_for_controls(self.view.size, &self.hud.controls)
    }

    /// Whether `position` lies on the active lot's rotated tile bounds
    /// (`0x4a7af0`: rotated fine position shifted right by 4, narrowed to
    /// signed bytes, checked against `lot + 0x70..+0x7c`).
    fn on_lot(&self, position: [i32; 3]) -> bool {
        if self.lot_shape.size == 0 {
            return false;
        }
        let rotated = sound::rotate_object_fine(&self.view.camera, [position[0], position[1]]);
        let tx = i32::from((rotated[0] >> 4) as i8);
        let ty = i32::from((rotated[1] >> 4) as i8);
        let b = sound::lot_bounds(&self.lot_shape, &self.view.camera);
        tx >= b[0] && tx <= b[1] && ty >= b[2] && ty <= b[3]
    }

    /// Moves the camera origin so screen point `at` comes to the middle
    /// of `view_size`, clamped to the original's scroll limits
    /// (`0x4a9fa9`) and x-aligned via `(x + 1) & !3` (`0x4a81cc`).
    ///
    /// `sims-render` draws walls and floors at `camera.origin[0]` and
    /// objects/people at `camera.origin[0] + 1`, whereas the original
    /// draws walls and floors one column left of `DAT_006f00d0`
    /// (`0x4ac14b`, `0x4ac25d`) and objects/people at it. So `0x4a9fa9`
    /// and `0x4a81cc` compute the original's `origin_x`, and we store
    /// `origin_x - pick::ORIGIN_COLUMN` so both walls and objects land on
    /// the original's exact screen columns.
    fn centre_screen_point_with_pan(&mut self, at: [i32; 2], view_size: [i32; 2], smooth: bool) {
        let middle = [view_size[0] / 2, view_size[1] / 2];
        let origin = self.view.camera.origin;
        let target = [
            origin[0].saturating_add(middle[0].saturating_sub(at[0])),
            origin[1].saturating_add(middle[1].saturating_sub(at[1])),
        ];
        let ([min_x, max_x], [min_y, max_y]) =
            scroll_limits(&self.lot_shape, &self.view.camera, view_size);
        let native_x = target[0].max(min_x).min(max_x).wrapping_add(1) & !3;
        let clamped_x = native_x.saturating_sub(pick::ORIGIN_COLUMN);
        let clamped_y = target[1].max(min_y).min(max_y);
        let goal = [clamped_x, clamped_y];
        if smooth && origin != goal {
            if self.camera_pan.is_some_and(|pan| pan.target_origin == goal) {
                return;
            }
            self.camera_pan = Some(CameraPan {
                start_origin: origin,
                target_origin: goal,
                start_us: self.clock_us,
                duration_us: CAMERA_PAN_DURATION_US,
            });
            self.camera_pan_moved = true;
            return;
        }
        self.camera_pan = None;
        if self.view.set_origin(goal) {
            self.sync_view();
        }
    }

    /// The lot as it stands, to go back to later ([`Game::restore`]).
    /// While a dialog waits, the save is of the lot before the tick that
    /// showed it, so the restored lot shows it again. While an unpaid Buy
    /// Mode preview is held, the preview and its `"Tile Cursor"` are stripped
    /// from the saved snapshot without disturbing the live placement.
    pub fn save(&self) -> Result<Saved, String> {
        match &self.buy_cursor {
            Some(cursor) => self
                .lot
                .save_without_buy_cursor(cursor)
                .map_err(|e| e.to_string()),
            None => self.lot.save().map_err(|e| e.to_string()),
        }
    }

    /// Keeps the lot in `store` under `slot`, saying across the top how
    /// that went. Returns `true` if the save succeeded (`0x4f26cf`, checked by
    /// `0x4f29cc` before exiting).
    pub fn save_to(&mut self, store: &sims_sim::Snapshots, slot: &str) -> bool {
        if self.coop_blocks_local_save() {
            self.note = Some("Cannot save a remote host's co-op lot to a local save slot.".into());
            return false;
        }
        let saved = self.save().and_then(|saved| {
            store
                .save_lot(slot, &saved.encode())
                .map_err(|e| e.to_string())
        });
        let ok = saved.is_ok();
        let followup = self.pending_after_save_note.take();
        self.note = Some(match saved {
            Ok(()) => {
                self.dirty = false;
                match followup {
                    Some(extra) => format!("Saved after tick {}. {extra}", self.lot.ticks()),
                    None => format!("Saved after tick {}.", self.lot.ticks()),
                }
            }
            Err(e) => format!("The lot was not saved: {e}"),
        });
        ok
    }

    /// Goes back to the lot saved in `store` under `slot`, saying across
    /// the top how that went. False if nothing changed.
    pub fn restore_from(&mut self, store: &sims_sim::Snapshots, slot: &str) -> bool {
        if self.coop.is_active_or_starting() {
            self.note = Some("Leave the co-op session before restoring a local save.".into());
            return false;
        }
        let saved = match store.load_lot(slot) {
            Ok(Some(bytes)) => Saved::decode(&bytes).map_err(|e| e.to_string()),
            Ok(None) => Err("nothing is saved for this house".into()),
            Err(e) => Err(e.to_string()),
        };
        let restored = saved.and_then(|saved| self.restore(saved));
        self.note = Some(match &restored {
            Ok(()) => format!("Back to the save after tick {}.", self.lot.ticks()),
            Err(e) => format!("The save was not restored: {e}"),
        });
        restored.is_ok()
    }

    /// Goes back to `saved`, a save of this house ([`Lot::restore`]). The
    /// lot then steps as the saved one would have. What the window kept
    /// of the lot before is dropped: why it stopped, the errors caught,
    /// the sounds not yet taken, an open or listed pie menu, the icons
    /// pressed and the dialog read. The panel's speed follows the saved
    /// lot's ([`pace::shown`]), the view turns to the saved lot's
    /// rotation, which the lot's code reads, and its people and rooms are
    /// read again. Sounds already playing are not stopped (**unverified** what a long one does once
    /// its object is gone).
    pub fn restore(&mut self, saved: Saved) -> Result<(), String> {
        self.lot.restore(saved).map_err(|e| e.to_string())?;
        self.world_mutated_via_restore = true;
        self.restore_generation = self.restore_generation.wrapping_add(1);
        self.clear_lot_cache();
        self.stopped = None;
        self.errors = Errors::default();
        self.sounds.clear();
        self.ui_effects.clear();
        self.highlighted_buttons.clear();
        self.highlighted_portraits.clear();
        self.highlighted_relationships.clear();
        self.reached = None;
        self.listed = None;
        self.highlighted = None;
        self.buy_selected = None;
        self.buy_cursor = None;
        self.buy_in_house = false;
        self.buy_last_guid = None;
        self.buy_facing = None;
        self.buy_dragging = None;
        self.buy_grab_offset = [0, 0];
        self.buy_commit_on_release = false;
        self.wall_cursor = None;
        self.build_drag = None;
        self.build_preview_shape = None;
        self.live_panel_expanded = true;
        self.submitted_coop_special_panel = None;
        self.close_special_panel_ui();
        self.pending_travel_lot = None;
        self.ui_animating_prev = false;
        self.hud.forget();
        self.hud.controls.mode = ui_mode(self.lot.mode());
        self.blink_dirty = false;
        self.dirty = false;
        self.restore_dirty_on_quit_cancel = false;
        self.pending_after_save_note = None;
        self.edge_scroll_rem = [0, 0];
        self.camera_pan = None;
        self.camera_pan_moved = false;
        // `0x517c21`, `0x46d8c9`: edit mode and the §999,999 edit budget both
        // read `[lot + 0x50]` (`global[32]`), so keep `edit_mode` synced with
        // the restored snapshot's global 32.
        self.edit_mode = self.lot.world().globals.get(32).copied().unwrap_or(0) == 1;
        // `0x4e66eb..0x4e6704 -> 0x432220`: restoring a lot keeps the player's
        // persisted Free Will preference and applies it to the restored lot.
        let free_will = self.hud.controls.options.play.free_will;
        if self.lot.world().globals.get(30).copied().unwrap_or(1) != i16::from(free_will) {
            let _ = self.lot.set_free_will(free_will);
        }
        self.hud.reset_character_names(self.lot.character_names());
        let world = self.lot.world();
        self.hud.sync_clock_blink(world, self.clock_us);
        if let Some(speed) = pace::shown(world) {
            self.pace.speed = speed;
        }
        // The view turns as the turn keys do, keeping the middle of the
        // screen, until it shows the lot as the saved lot was turned.
        let (w, h) = self.house_viewport_size();
        let viewport = [w as i32, h as i32];
        let rotation = self.lot.view_rotation();
        for _ in 0..4 {
            if self.view.camera.rotation == rotation {
                break;
            }
            if self.lot_shape.size > 0 {
                native_zoom_or_rotate(
                    &self.lot_shape,
                    &mut self.view.camera,
                    viewport,
                    Move::Rotate(true),
                );
                self.view.bound();
            } else {
                self.view
                    .apply_in(Move::Rotate(true), viewport, pick::ORIGIN_COLUMN);
            }
        }
        self.pace.restart();
        if !self.open_pack_mode {
            self.people.refresh(world);
        }
        match sims_render::Lot::from_world(world) {
            Ok(shape) => self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation()),
            Err(e) => self
                .people
                .note(vec![format!("the lot is not redrawn: {e}")]),
        }
        self.hud.relayout_panel(
            self.view.size,
            self.lot.world(),
            self.lot.residential(),
            self.lot.lot_zoning(),
            self.lot_visibility(),
            &self.assets,
            None,
        );
        if self.hud.controls.mode == Mode::Build {
            self.layout_build();
        }
        Ok(())
    }

    /// Carries out a panel command. `menu` is the pie menu that was open
    /// when it was pressed.
    fn command(&mut self, command: Command, menu: Option<&Menu>) -> Outcome {
        match command {
            Command::SetSpeed(speed) => {
                self.set_speed(speed);
                Outcome::redraw()
            }
            Command::TogglePause => {
                self.toggle_pause();
                Outcome::redraw()
            }
            Command::Camera(camera) => match camera {
                UiCamera::ZoomIn => {
                    self.apply(Move::ZoomIn);
                    Outcome::redraw()
                }
                UiCamera::ZoomOut => {
                    self.apply(Move::ZoomOut);
                    Outcome::redraw()
                }
                // Which way the original turns for each is unverified.
                UiCamera::RotateRight => {
                    self.apply(Move::Rotate(true));
                    Outcome::redraw()
                }
                UiCamera::RotateLeft => {
                    self.apply(Move::Rotate(false));
                    Outcome::redraw()
                }
                UiCamera::FloorUp => {
                    self.apply(Move::LevelUp);
                    Outcome::redraw()
                }
                UiCamera::FloorDown => {
                    self.apply(Move::LevelDown);
                    Outcome::redraw()
                }
                UiCamera::Walls(w) => {
                    self.apply(Move::Walls(match w {
                        UiWalls::Up => Walls::Up,
                        UiWalls::Cutaway => Walls::Cutaway,
                        UiWalls::Down => Walls::Down,
                    }));
                    Outcome::redraw()
                }
                UiCamera::Photo => {
                    self.toggle_camera_strip();
                    Outcome::redraw()
                }
                UiCamera::Roof => {
                    // Native `Sims.exe` `0x53a34d -> 0x4cef10(8, 1)`: 4-way
                    // mutually exclusive radio button (`ROOF` / `WALLS_UP` /
                    // `CUTAWAY` / `WALLS_DOWN`). Selecting Roof raises walls
                    // and enables roof rendering, unlighting all 3 wall buttons.
                    self.set_roof_view(true);
                    self.hud.play(crate::hud::CLICK);
                    Outcome::redraw()
                }
            },
            Command::ChooseInteraction {
                sim,
                object,
                interaction,
            } => {
                // `interaction` is the entry's index in the open menu.
                match menu.and_then(|m| m.entries.get(usize::from(interaction))) {
                    Some(entry) => self.order(sim, object, entry),
                    None => self.note = Some(format!("No menu entry {interaction} to choose.")),
                }
                Outcome::redraw()
            }
            Command::Select { sim } => {
                self.select(sim);
                Outcome::redraw()
            }
            Command::CancelAction { sim, action } => {
                self.cancel(sim, action);
                Outcome::redraw()
            }
            Command::AnswerDialog { caller, button } => {
                if caller == 0 && self.hud.has_ui_dialog() {
                    if let Some((kind, choice, entry, secondary)) =
                        self.hud.take_ui_dialog_answer(button)
                    {
                        self.hud.play(crate::hud::CLICK);
                        let apply_save_card = |lot: &mut Lot, dirty: &mut bool| {
                            let name = entry.unwrap_or_else(|| lot.lot_name().to_owned());
                            let desc =
                                secondary.unwrap_or_else(|| lot.lot_description().to_owned());
                            if name != lot.lot_name() || desc != lot.lot_description() {
                                *dirty = true;
                            }
                            lot.set_lot_info(name, desc);
                        };
                        match (kind, choice) {
                            (OptionsDialog::SaveCardSave, 0) => {
                                apply_save_card(&mut self.lot, &mut self.dirty);
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmQuit, 0) => {
                                self.restore_dirty_on_quit_cancel = false;
                                return Outcome {
                                    redraw: true,
                                    quit: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmQuit, 1) => {
                                if std::mem::take(&mut self.restore_dirty_on_quit_cancel) {
                                    self.dirty = true;
                                }
                                self.note = None;
                            }
                            // `0x51681a..0x51686f`: edit-mode Save Card during
                            // Quit saves on Yes (`0x4f26cf`) and then enters
                            // the regular Quit confirmation (`0x4f290d`); on No
                            // it clears `dirty` temporarily for `ConfirmQuit`
                            // and restores `dirty` if the player declines.
                            (OptionsDialog::SaveCardQuit, 0) => {
                                apply_save_card(&mut self.lot, &mut self.dirty);
                                self.restore_dirty_on_quit_cancel = false;
                                self.hud.open_options_dialog(
                                    OptionsDialog::ConfirmQuit,
                                    self.view.size,
                                    self.lot.lot_name(),
                                    self.lot.lot_description(),
                                );
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::SaveCardQuit, 1) => {
                                self.restore_dirty_on_quit_cancel = self.dirty;
                                self.dirty = false;
                                self.hud.open_options_dialog(
                                    OptionsDialog::ConfirmQuit,
                                    self.view.size,
                                    self.lot.lot_name(),
                                    self.lot.lot_description(),
                                );
                            }
                            (OptionsDialog::ConfirmSaveBeforeQuit, 0) => {
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    quit: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmSaveBeforeQuit, 1) => {
                                return Outcome {
                                    redraw: true,
                                    quit: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::SaveCardNeighborhood, 0) => {
                                apply_save_card(&mut self.lot, &mut self.dirty);
                                let note = "Returning to Neighborhood.";
                                self.pending_after_save_note = Some(note);
                                self.note = Some(note.into());
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    neighborhood: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmSaveBeforeNeighborhood, 0) => {
                                let note = "Returning to Neighborhood.";
                                self.pending_after_save_note = Some(note);
                                self.note = Some(note.into());
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    neighborhood: true,
                                    ..Outcome::default()
                                };
                            }
                            (
                                OptionsDialog::SaveCardNeighborhood
                                | OptionsDialog::ConfirmSaveBeforeNeighborhood,
                                1,
                            ) => {
                                self.pending_after_save_note = None;
                                self.note = None;
                                return Outcome {
                                    redraw: true,
                                    neighborhood: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmResetTutorial, 0) => {
                                // `0x4f25ea -> 0x4f1a11..0x4f1ac6`: confirming
                                // Reset Tutorial on a dirty savable lot runs
                                // the native save confirmation before reset.
                                if self.edit_mode && self.dirty {
                                    self.hud.open_options_dialog(
                                        OptionsDialog::SaveCardResetTutorial,
                                        self.view.size,
                                        self.lot.lot_name(),
                                        self.lot.lot_description(),
                                    );
                                } else if self.dirty && self.lot_visibility().can_save {
                                    self.hud.open_options_dialog(
                                        OptionsDialog::ConfirmSaveBeforeResetTutorial,
                                        self.view.size,
                                        self.lot.lot_name(),
                                        self.lot.lot_description(),
                                    );
                                } else {
                                    self.note = Some("Tutorial reset is not ported yet.".into());
                                }
                            }
                            (OptionsDialog::SaveCardResetTutorial, 0) => {
                                apply_save_card(&mut self.lot, &mut self.dirty);
                                let note = "Tutorial reset is not ported yet.";
                                self.pending_after_save_note = Some(note);
                                self.note = Some(note.into());
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmSaveBeforeResetTutorial, 0) => {
                                let note = "Tutorial reset is not ported yet.";
                                self.pending_after_save_note = Some(note);
                                self.note = Some(note.into());
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    ..Outcome::default()
                                };
                            }
                            (
                                OptionsDialog::SaveCardResetTutorial
                                | OptionsDialog::ConfirmSaveBeforeResetTutorial,
                                1,
                            ) => {
                                self.pending_after_save_note = None;
                                self.note = Some("Tutorial reset is not ported yet.".into());
                            }
                            (OptionsDialog::SpecialBook, _) => {
                                if !self.answer_special_panel(SpecialPanelResponse::Dismissed) {
                                    // Restore the book modal if co-op submission failed so
                                    // the blocking panel never becomes invisible.
                                    self.active_special_panel = None;
                                    self.sync_special_panel();
                                }
                            }
                            (OptionsDialog::Notice, _) => {
                                self.note = None;
                                if let Some(next) = self.pending_after_notice.take() {
                                    let name = self.lot.lot_name().to_owned();
                                    let desc = self.lot.lot_description().to_owned();
                                    self.hud.open_options_dialog(
                                        next,
                                        self.view.size,
                                        &name,
                                        &desc,
                                    );
                                }
                            }
                            _ => {
                                self.note = None;
                            }
                        }
                    }
                } else {
                    self.answer(caller, button);
                }
                Outcome::redraw()
            }
            Command::OptionsAction(action) => {
                if self.coop.is_peer()
                    && matches!(action, OptionsAction::Save | OptionsAction::Neighborhood)
                {
                    self.note = Some(
                        "Only the host can save or return to Neighborhood; use Co-op -> Leave to disconnect."
                            .into(),
                    );
                    return Outcome::redraw();
                }
                let can_save = self.lot_visibility().can_save;
                match action {
                    // `0x516874`: calls `0x517c03(0)` (shows the save card only
                    // when `[lot + 0x50] == 1` edit mode at `0x517c21`) and
                    // otherwise proceeds directly to `0x4f26cf` (save).
                    OptionsAction::Save if !self.edit_mode => {
                        self.pending_after_save_note = None;
                        Outcome {
                            redraw: true,
                            save: true,
                            ..Outcome::default()
                        }
                    }
                    OptionsAction::Save => {
                        self.hud.open_options_dialog(
                            OptionsDialog::SaveCardSave,
                            self.view.size,
                            self.lot.lot_name(),
                            self.lot.lot_description(),
                        );
                        Outcome::redraw()
                    }
                    // `0x516809..0x51686f` -> `0x4f290d`:
                    // - edit mode & dirty -> save card (`0x517c03(1)`)
                    // - ordinary dirty & savable -> 3-button `STR# 153[5..6]` (`0x4f298e`)
                    // - clean or non-savable -> 2-button `STR# 153[1..2]` (`0x4f29f1`/`0x4f2a46`)
                    OptionsAction::Quit => {
                        let dialog = if self.edit_mode && self.dirty {
                            OptionsDialog::SaveCardQuit
                        } else if self.dirty && can_save {
                            OptionsDialog::ConfirmSaveBeforeQuit
                        } else {
                            OptionsDialog::ConfirmQuit
                        };
                        self.hud.open_options_dialog(
                            dialog,
                            self.view.size,
                            self.lot.lot_name(),
                            self.lot.lot_description(),
                        );
                        Outcome::redraw()
                    }
                    // `0x5170f0..0x51713c`: prompts only when `dirty && can_save`;
                    // edit mode uses the save card (`0x517c03(1)`), ordinary lots
                    // fall back to `0x4f1a11` (`STR# 153[3..4]`).
                    OptionsAction::Neighborhood if !self.dirty || !can_save => Outcome {
                        redraw: true,
                        neighborhood: true,
                        ..Outcome::default()
                    },
                    OptionsAction::Neighborhood => {
                        let dialog = if self.edit_mode {
                            OptionsDialog::SaveCardNeighborhood
                        } else {
                            OptionsDialog::ConfirmSaveBeforeNeighborhood
                        };
                        self.hud.open_options_dialog(
                            dialog,
                            self.view.size,
                            self.lot.lot_name(),
                            self.lot.lot_description(),
                        );
                        Outcome::redraw()
                    }
                    // `0x4f25b4`: 2-button `STR# 153[7..8]` confirmation.
                    OptionsAction::TutorialReset => {
                        self.hud.open_options_dialog(
                            OptionsDialog::ConfirmResetTutorial,
                            self.view.size,
                            self.lot.lot_name(),
                            self.lot.lot_description(),
                        );
                        Outcome::redraw()
                    }
                }
            }
            Command::ToggleOption(toggle) => {
                if toggle == OptionsToggle::FreeWill {
                    // `0x432220..0x432242` -> `0x46d9c4(0x1e, al)`: write global 30.
                    let enabled = self.hud.controls.options.play.free_will;
                    if self.coop.is_active() {
                        if let Err(e) = self
                            .coop
                            .submit_command(NetCommand::SetFreeWill { enabled })
                        {
                            self.note = Some(format!("Free Will was not changed: {e}"));
                        }
                    } else if let Err(e) = self.lot.set_free_will(enabled) {
                        self.note = Some(format!("Free Will was not changed: {e}"));
                    } else {
                        self.dirty = true;
                    }
                }
                Outcome::redraw()
            }
            Command::OpenHelp => {
                self.note = Some("The Help System dialog is not ported yet.".into());
                Outcome::redraw()
            }
            Command::SetMode(mode) => {
                self.set_mode(mode);
                Outcome::redraw()
            }
            Command::BuyCategory(_) | Command::BuySubcategory(_) | Command::BuyBack => {
                self.cancel_buy();
                self.layout_buy();
                Outcome::redraw()
            }
            Command::BuyToggleSort => {
                self.toggle_buy_sort();
                Outcome::redraw()
            }
            Command::BuyPage(_) => {
                self.layout_buy();
                Outcome::redraw()
            }
            Command::BuyItem(guid) => {
                self.select_buy_item(guid);
                Outcome::redraw()
            }
            Command::BuildTool(tool) => {
                self.select_build_tool(tool);
                Outcome::redraw()
            }
            Command::BuildPage(delta) => {
                self.page_build(delta);
                Outcome::redraw()
            }
            Command::BuildItem(item) => {
                self.select_build_item(item);
                Outcome::redraw()
            }
            Command::Undo => {
                self.undo_buy();
                Outcome::redraw()
            }
            Command::Redo => {
                self.redo_buy();
                Outcome::redraw()
            }
            Command::TogglePanel => {
                self.hud.close_camera_strip();
                if self.hud.controls.mode == Mode::Live {
                    self.live_panel_expanded = self.hud.controls.expanded;
                }
                self.sync_view_centre();
                if self.hud.controls.mode == Mode::Buy {
                    self.layout_buy();
                } else if self.hud.controls.mode == Mode::Build {
                    self.layout_build();
                }
                Outcome::redraw()
            }
            Command::OpenOptionsTab(_) | Command::OptionsHelp(_) => {
                self.hud.relayout_panel(
                    self.view.size,
                    self.lot.world(),
                    self.lot.residential(),
                    self.lot.lot_zoning(),
                    self.lot_visibility(),
                    &self.assets,
                    None,
                );
                Outcome::redraw()
            }
            Command::OpenPanel(_)
            | Command::Refused { .. }
            | Command::ShowRelations(_)
            | Command::RelationsPage(_)
            | Command::InterestsPage(_)
            | Command::ShowInventory(_)
            | Command::InventoryPage(_)
            | Command::SelectInventoryItem(_)
            | Command::Help(_)
            | Command::SetDetail { .. }
            | Command::SetVolume { .. } => Outcome::redraw(),
        }
    }

    /// The active control-panel mode (`Mode::Live`, `Mode::Buy`, etc.).
    pub fn mode(&self) -> Mode {
        self.hud.controls.mode
    }

    /// The active simulation speed (`Speed::Normal`, `Speed::Fast`, `Speed::Ultra`).
    pub fn speed(&self) -> Speed {
        self.pace.speed
    }

    /// Whether the bottom control panel strip is currently expanded.
    pub fn panel_expanded(&self) -> bool {
        self.hud.controls.expanded
    }

    /// Active wall display mode (`Walls::Up`, `Walls::Cutaway`, `Walls::Down`).
    pub fn walls(&self) -> Walls {
        self.view.camera.walls
    }

    /// Active displayed floor level (`1` = ground, `2` = upper).
    pub fn level(&self) -> i32 {
        self.view.camera.level
    }

    fn flush_hud_sounds(&mut self) {
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
    }

    /// Toggles the bottom control panel strip collapsed/expanded (`F10` or
    /// pressing `F1` while already in Live Mode, `0x4cfc75`). Closes the Camera
    /// strip if open, clears any open help balloons (`Command::TogglePanel`),
    /// and re-centers the active house viewport. Defers while a Buy or Build
    /// pointer drag is in progress so expanding the strip underneath a held
    /// object cannot turn a move release into an unintended sale.
    pub fn toggle_panel(&mut self) -> bool {
        if self.buy_dragging.is_some() || self.buy_commit_on_release || self.build_drag.is_some() {
            return false;
        }
        self.hud.close_camera_strip();
        let had_balloon =
            self.hud.controls.help.is_some() || self.hud.controls.options.help.is_some();
        self.hud.controls.apply(&Command::TogglePanel);
        if self.hud.controls.mode == Mode::Live {
            self.live_panel_expanded = self.hud.controls.expanded;
        }
        self.sync_view_centre();
        if self.hud.controls.mode == Mode::Buy {
            self.layout_buy();
        } else if self.hud.controls.mode == Mode::Build {
            self.layout_build();
        }
        if had_balloon {
            self.hud.play(crate::hud::WHOOSH);
        }
        self.hud.play(crate::hud::CLICK);
        self.flush_hud_sounds();
        true
    }

    /// Toggles the Camera / Photo Mode strip (`F4`, `0x4d1712` / `0x53a543`).
    /// Defers while a Buy or Build pointer drag is in progress.
    pub fn toggle_camera_strip(&mut self) -> bool {
        if self.buy_dragging.is_some() || self.buy_commit_on_release || self.build_drag.is_some() {
            return false;
        }
        let was_expanded = self.hud.controls.expanded;
        self.hud.toggle_camera_strip(self.view.size);
        if self.hud.controls.expanded != was_expanded {
            if self.hud.controls.mode == Mode::Live {
                self.live_panel_expanded = self.hud.controls.expanded;
            }
            self.sync_view_centre();
            if self.hud.controls.mode == Mode::Buy {
                self.layout_buy();
            } else if self.hud.controls.mode == Mode::Build {
                self.layout_build();
            }
        }
        self.hud.play(crate::hud::CLICK);
        self.flush_hud_sounds();
        true
    }

    /// Authentic `Sims.exe` 1.91 `F1`/`F2`/`F3`/`F5` mode hotkey dispatch
    /// (`0x4d16e2..0x4d1869`):
    /// - `F1` (`Mode::Live`, `0x4d1826`): enters Live Mode, closes Camera strip
    ///   if open, or toggles panel collapse/expand when already in Live Mode.
    /// - `F2` (`Mode::Buy`, `0x4d1787`): expands/enters Buy Mode, or toggles
    ///   `ByFunction` ↔ `ByRoom` sort when already in Buy Mode (`0x4cf3af`).
    /// - `F3` (`Mode::Build`, `0x4d1750`): expands and enters Build Mode.
    /// - `F5` (`Mode::Options`, `0x4d16e2`): expands and enters Options Mode.
    pub fn hotkey_mode(&mut self, mode: Mode) {
        match mode {
            Mode::Live => {
                if self.hud.controls.mode == Mode::Live {
                    if self.hud.camera_strip_open() {
                        self.hud.close_camera_strip();
                        self.hud.play(crate::hud::CLICK);
                        self.flush_hud_sounds();
                    } else {
                        self.toggle_panel();
                    }
                } else {
                    self.set_mode(Mode::Live);
                }
            }
            Mode::Buy => {
                if self.hud.controls.mode == Mode::Buy && !self.hud.camera_strip_open() {
                    if !self.hud.controls.expanded {
                        self.toggle_panel();
                    } else if self.toggle_buy_sort() {
                        self.hud.play(crate::hud::CLICK);
                        self.flush_hud_sounds();
                    }
                } else {
                    self.set_mode(Mode::Buy);
                }
            }
            Mode::Build => {
                if self.hud.controls.mode == Mode::Build && !self.hud.camera_strip_open() {
                    if !self.hud.controls.expanded {
                        self.toggle_panel();
                    }
                } else {
                    self.set_mode(Mode::Build);
                }
            }
            Mode::Options => {
                self.set_mode(Mode::Options);
            }
        }
    }

    /// Cycles selection to the next Sim in the household roster, wrapping
    /// around (`Space` in Live Mode, `cWinLive::OnKeyDown` at `0x51ba2d..0x51ba7c`).
    /// Only active in Live Mode so Buy/Build/Options editing never switches
    /// household selection. Closes any open pie menu and plays `UI_click`. If
    /// only one Sim is in the household, centres the camera on that Sim.
    pub fn select_next_sim(&mut self) -> bool {
        if self.hud.controls.mode != Mode::Live {
            return false;
        }
        let residents = self.hud.resident_object_ids(self.lot.world());
        if residents.is_empty() {
            return false;
        }
        let world = self.lot.world();
        let current = self.local_selected_sim();
        let current_guid = current
            .and_then(|id| world.objects.get(&id))
            .map(|o| o.guid);
        let current_idx = residents.iter().position(|&id| {
            current == Some(id)
                || current_guid.is_some() && world.objects.get(&id).map(|o| o.guid) == current_guid
        });
        let next = match current_idx {
            Some(idx) => residents[(idx + 1) % residents.len()],
            None => residents[0],
        };
        self.close_menu();
        self.select(next);
        self.flush_hud_sounds();
        true
    }

    /// Centres the camera on the currently selected Sim (`Enter` in
    /// `OnKeyDown`, `0x4d1330 -> 0x4d14d2..0x4d1507`).
    pub fn centre_on_selected(&mut self) -> bool {
        let Some(sim) = self.local_selected_sim() else {
            return false;
        };
        let world = self.lot.world();
        let target = world
            .objects
            .get(&sim)
            .and_then(|o| sims_ui::household::first_person_for_guid(world, o.guid))
            .map_or(sim, |o| o.id);
        let smooth = self.smooth_pan_active();
        self.centre_on_with_pan(target, smooth);
        true
    }

    /// Steps wall display mode upward (`Home`: `Down → Cutaway → Up`, `0x4d135b`)
    /// or downward (`End`: `Up → Cutaway → Down`, `0x4d1393`), playing
    /// `UI_error` at either boundary. When Roof view (`[ui + 0x48] == 8`) is
    /// active, `Home`/`End` do not match `1`/`2`/`4` in `Sims.exe` 1.91 and
    /// leave Roof view unchanged without playing an error.
    pub fn step_walls(&mut self, up: bool) -> bool {
        if self.roof_view {
            return false;
        }
        let next = match (self.view.camera.walls, up) {
            (Walls::Down, true) => Some(Walls::Cutaway),
            (Walls::Cutaway, true) => Some(Walls::Up),
            (Walls::Up, true) => None,
            (Walls::Up, false) => Some(Walls::Cutaway),
            (Walls::Cutaway, false) => Some(Walls::Down),
            (Walls::Down, false) => None,
        };
        match next {
            Some(walls) => self.apply(Move::Walls(walls)),
            None => {
                self.hud.play(crate::hud::ERROR);
                self.flush_hud_sounds();
                false
            }
        }
    }

    /// Toggles between ground floor (`1`) and upper floor (`2`) (`Tab` in
    /// `OnKeyDown`, `0x4d1324 -> 0x4d150c`).
    pub fn toggle_floor(&mut self) -> bool {
        if self.view.camera.level <= 1 {
            self.apply(Move::LevelUp)
        } else {
            self.apply(Move::LevelDown)
        }
    }

    /// Steps the active Buy or Build catalog grid page by `delta` (`+1` for
    /// `X`, `-1` for `Z`, `0x4d11b5` / `0x4d1224`).
    pub fn page_catalog(&mut self, delta: i32) -> bool {
        match self.hud.controls.mode {
            Mode::Buy => self.page_buy(delta),
            Mode::Build => self.page_build(delta),
            _ => false,
        }
    }

    /// The Buy Mode catalog master GUID currently armed for placement, if any.
    pub fn buy_selected(&self) -> Option<u32> {
        self.buy_selected
    }

    /// The active Buy Mode placement cursor (once the pointer has entered
    /// the house view or picked up an existing object), if any.
    pub fn buy_cursor(&self) -> Option<&BuyCursor> {
        self.buy_cursor.as_ref()
    }

    /// The Buy Mode catalog strip state (sort, category, subcategory, page,
    /// selected GUID).
    pub fn buy_state(&self) -> sims_ui::buy::BuyState {
        self.hud.buy_state
    }

    /// The Buy Mode catalog strip layout as last built.
    pub fn buy_layout(&self) -> &sims_ui::buy::BuyLayout {
        self.hud.buy_layout()
    }

    /// Computes sell prompt info (`0x487d7a` → `0x4cf90b` → `0x50a343`) when
    /// the Hand Tool is currently holding a picked-up existing lot object.
    fn buy_sell_info(&mut self) -> Option<sims_ui::buy::SellInfo> {
        let cursor = self.buy_cursor.as_ref()?;
        if !cursor.is_existing() {
            return None;
        }
        let held = cursor.held;
        let guid = cursor.guid;
        let (icon_guid, name) = self
            .hud
            .ensure_buy_catalog(&self.assets)
            .display_for_object(guid);
        let refund = self.lot.resale_value(held).unwrap_or(0);
        let can_delete = self.lot.can_delete(held).unwrap_or(false);
        Some(sims_ui::buy::SellInfo {
            guid: icon_guid,
            name,
            refund,
            can_delete,
        })
    }

    /// Rebuilds the cached Buy Mode strip layout (catalog or active Hand Tool
    /// sell prompt) for the current viewport size and world state.
    fn layout_buy(&mut self) {
        let sell = self.buy_sell_info();
        self.hud.layout_buy_for_lot(
            self.view.size,
            self.lot.world(),
            self.lot.lot_zoning(),
            &self.assets,
            sell.as_ref(),
        );
    }

    /// Toggles the Buy Mode catalog sort order between `ByFunction` and
    /// `ByRoom` (`0x53a2c3` → `0x4cf3af`), cancelling any active cursor and
    /// returning to the top-level categories.
    pub fn toggle_buy_sort(&mut self) -> bool {
        if self.hud.controls.mode != Mode::Buy {
            return false;
        }
        self.cancel_buy();
        if self.buy_cursor.is_some() {
            return false;
        }
        let changed = self.hud.buy_state.apply(&Command::BuyToggleSort);
        self.layout_buy();
        changed
    }

    /// Steps the Buy Mode catalog grid page by `delta` (`+1` for `x`, `-1` for
    /// `z`, `0x4d0ef0`). Returns `true` if the displayed page changed.
    pub fn page_buy(&mut self, delta: i32) -> bool {
        if self.hud.controls.mode != Mode::Buy
            || !self.hud.controls.expanded
            || self.hud.buy_state.category.is_none()
        {
            return false;
        }
        let before = self.hud.buy_state.page;
        if !self.hud.buy_state.apply(&Command::BuyPage(delta)) {
            return false;
        }
        self.layout_buy();
        self.hud.buy_state.page != before
    }

    /// Sells or deletes the currently held Buy Mode object (`Delete` /
    /// `Backspace` key `0x486b5b` or clicking/dropping onto the catalog strip
    /// `0x50d04d` → `0x4cf97a` → `0x487e18`). Returns `true` if a Buy cursor
    /// was active and handled.
    pub fn sell_held(&mut self) -> bool {
        if !matches!(self.hud.controls.mode, Mode::Buy | Mode::Build) || self.stopped.is_some() {
            return false;
        }
        if self.coop.is_active() {
            let Some(preview) = self.coop_buy_preview.take() else {
                return false;
            };
            self.buy_dragging = None;
            self.buy_commit_on_release = false;
            self.buy_in_house = false;
            self.buy_selected = None;
            self.hud.buy_state.selected = None;
            if let Some(object_id) = preview.existing_object_id {
                if let Err(e) = self
                    .coop
                    .submit_command(NetCommand::BuyBuild(BuyBuildEdit::SellObject { object_id }))
                {
                    self.note = Some(format!("Could not send Sell command: {e}"));
                    self.hud.play(crate::hud::ERROR);
                } else {
                    self.hud.play(crate::hud::MONEYBACK);
                }
            }
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return true;
        }
        let Some(cursor) = self.buy_cursor.take() else {
            return false;
        };
        let was_existing = cursor.is_existing();
        let refund = if was_existing {
            self.lot.resale_value(cursor.held).unwrap_or(0)
        } else {
            0
        };
        match self.lot.buy_sell(&cursor) {
            Ok(true) => {
                if was_existing {
                    self.dirty = true;
                    // Native `0x487e18` -> `0x492ad4` (`UI_moneyback`) /
                    // `0x492b05` (`UI_archtool_delete`) with 400 ms cooldown,
                    // keyed off `resale_value` even when unlimited budget
                    // (`global[32] != 0`) keeps household funds flat.
                    if refund != 0 {
                        if self
                            .buy_moneyback_us
                            .is_none_or(|t| self.clock_us.saturating_sub(t) > 400_000)
                        {
                            self.hud.play(crate::hud::MONEYBACK);
                            self.buy_moneyback_us = Some(self.clock_us);
                        }
                    } else if self
                        .buy_delete_us
                        .is_none_or(|t| self.clock_us.saturating_sub(t) > 400_000)
                    {
                        self.hud.play(crate::hud::ARCHTOOL_DELETE);
                        self.buy_delete_us = Some(self.clock_us);
                    }
                }
                self.buy_dragging = None;
                self.buy_grab_offset = [0, 0];
                self.buy_commit_on_release = false;
                self.buy_in_house = false;
                self.buy_selected = None;
                self.hud.buy_state.selected = None;
                self.refresh_world();
                self.layout_buy();
            }
            Ok(false) => {
                self.buy_cursor = Some(cursor);
                self.hud.play(crate::hud::ERROR);
            }
            Err(e) => {
                self.buy_cursor = Some(cursor);
                self.note = Some(format!("The object was not sold: {e}"));
                self.hud.play(crate::hud::ERROR);
            }
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        true
    }

    /// Whether a committed Buy or Build Mode action can be undone (`0x4cf074`).
    pub fn can_undo(&self) -> bool {
        match self.hud.controls.mode {
            Mode::Buy => self.lot.can_buy_undo(),
            Mode::Build => self.lot.can_undo(),
            _ => false,
        }
    }

    /// Whether an undone Buy or Build Mode action can be redone (`0x4cf08c`).
    pub fn can_redo(&self) -> bool {
        match self.hud.controls.mode {
            Mode::Buy => self.lot.can_buy_redo(),
            Mode::Build => self.lot.can_redo(),
            _ => false,
        }
    }

    /// Runs a Buy/Build Mode Undo (`undo = true`) or Redo (`undo = false`). If the
    /// Hand Tool currently holds a picked-up existing lot object, snaps it
    /// back and clears cursor ownership first (without clearing the redo
    /// stack) so the Hand Tool never conflicts with or double-refunds an
    /// undone/redone object; unpaid new catalog previews stay armed
    /// (`0x4a8545`, `0x4a856a`).
    fn apply_buy_history(&mut self, undo: bool) -> bool {
        if self.coop.is_active() {
            self.note = Some("Undo/Redo is disabled during co-op multiplayer sessions.".into());
            return false;
        }
        let allowed = if undo {
            self.can_undo()
        } else {
            self.can_redo()
        };
        if !allowed || self.stopped.is_some() {
            return false;
        }
        self.build_drag = None;
        self.build_preview_shape = None;
        let snapped_existing = if self.buy_cursor.as_ref().is_some_and(BuyCursor::is_existing) {
            let cursor = self.buy_cursor.take().unwrap();
            match self.lot.buy_snap_back(&cursor) {
                Ok(()) => {
                    self.buy_dragging = None;
                    self.buy_grab_offset = [0, 0];
                    self.buy_commit_on_release = false;
                    self.buy_in_house = false;
                    true
                }
                Err(e) => {
                    self.buy_cursor = Some(cursor);
                    self.note = Some(format!("The held object could not be released: {e}"));
                    return false;
                }
            }
        } else {
            false
        };
        let result = if self.hud.controls.mode == Mode::Build {
            if undo {
                self.lot.undo()
            } else {
                self.lot.redo()
            }
        } else if undo {
            self.lot.buy_undo()
        } else {
            self.lot.buy_redo()
        };
        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        if let Some(cursor) = self.buy_cursor.as_mut() {
            let held_on_lot = self
                .lot
                .world()
                .objects
                .get(&cursor.held)
                .is_some_and(|o| o.position[0] >= 0);
            let affordable =
                cursor.price <= 0 || cursor.price <= sims_vm::available_funds(self.lot.world());
            cursor.valid = held_on_lot && affordable;
        }
        match result {
            Ok(true) => {
                self.dirty = true;
                self.refresh_world();
                if self.hud.controls.mode == Mode::Build {
                    self.sync_build_roof_ui();
                    self.layout_build();
                } else {
                    self.layout_buy();
                }
                true
            }
            Ok(false) => {
                self.refresh_world();
                if self.hud.controls.mode == Mode::Build {
                    self.layout_build();
                } else if snapped_existing {
                    self.layout_buy();
                }
                false
            }
            Err(e) => {
                let verb = if undo { "undone" } else { "redone" };
                self.note = Some(format!("The action was not {verb}: {e}"));
                self.refresh_world();
                if self.hud.controls.mode == Mode::Build {
                    self.layout_build();
                } else if snapped_existing {
                    self.layout_buy();
                }
                false
            }
        }
    }

    /// Undoes the most recent committed Buy/Build Mode action (`0x4cf2e0` /
    /// event `0xec` → `0x4932fa`). Returns `true` if an action was undone.
    pub fn undo_buy(&mut self) -> bool {
        self.apply_buy_history(true)
    }

    /// Redoes the most recently undone Buy/Build Mode action (`0x4cf317` /
    /// event `0xed` → `0x493338`). Returns `true` if an action was redone.
    pub fn redo_buy(&mut self) -> bool {
        self.apply_buy_history(false)
    }

    /// Sets whether the Shift modifier is held (`VK_SHIFT`, `0x4864f9`),
    /// which keeps the Buy tool armed for repeat purchases after placing any
    /// affordable item (`0x48680f`) and toggles Build Mode Room-Box / Room-Fill.
    pub fn set_shift(&mut self, held: bool) {
        if self.shift_held == held {
            return;
        }
        self.shift_held = held;
        if self.hud.controls.mode == Mode::Build
            && let Some(pt) = self.pointer
            && !self.hud.covers(pt)
        {
            self.hover_build(pt);
        }
    }

    /// Sets whether the Ctrl modifier is held (`VK_CONTROL`), which switches
    /// Build Mode Wall/Floor/Wallpaper tools to Demolish / Remove / Strip.
    pub fn set_ctrl(&mut self, held: bool) {
        if self.ctrl_held == held {
            return;
        }
        self.ctrl_held = held;
        if self.hud.controls.mode == Mode::Build
            && let Some(pt) = self.pointer
            && !self.hud.covers(pt)
        {
            self.hover_build(pt);
        }
    }

    /// Switches between Live, Buy, Build, and Options modes (`0x4cea5d`,
    /// `0x4ce44f`). Cancels any unplaced Buy Mode item, closes open pie menus,
    /// updates `global[19]` (calling `go_live` `0x416534` when returning to
    /// Live mode), and refreshes the control panel and Buy catalog strip.
    pub fn set_mode(&mut self, mode: Mode) {
        self.hud.close_camera_strip();
        // `0x4f2482..0x4f248b` -> `0x53b233..0x53b254`: edit-mode lots lock
        // out Live mode while `[lot + 0x50] == 1`.
        if mode == Mode::Live && self.edit_mode {
            return;
        }
        // Native `0x4cea5d`: pressing a mode shortcut for the already-active
        // mode (e.g. F2 in Buy Mode, F1 in Live Mode) is a no-op and preserves
        // the open catalog state and any active Hand Tool pickup; in Options
        // mode, re-clicking while collapsed expands the strip (`0x53a51b`).
        if self.hud.controls.mode == mode && (mode != Mode::Options || self.hud.controls.expanded) {
            return;
        }
        // Native `0x4cea5d` runs `0x416534` (`go_live`), `0x40726a`
        // (`update_comeandsee`), and `0x4933dc` (`buy_purge_detached`) BEFORE
        // tearing down the Buy tool's active cursor (`0x4cec3a` -> `0x4cef9b`
        // -> `0x487191` / `0x487105`). Execute all four steps atomically so
        // marker IDs match native and any snapback refusal leaves the mode and
        // held cursor intact.
        if self.coop.is_active() {
            if let Some(net_mode) = match mode {
                Mode::Live => Some(NetPlayerMode::Live),
                Mode::Buy => Some(NetPlayerMode::Buy),
                Mode::Build => Some(NetPlayerMode::Build),
                Mode::Options => None,
            } && let Err(e) = self
                .coop
                .submit_command(NetCommand::SetMode { mode: net_mode })
            {
                self.note = Some(format!("The mode was not changed: {e}"));
                return;
            }
            self.coop_buy_preview = None;
        } else if let Err(e) = self
            .lot
            .set_mode_releasing_buy_cursor(panel_mode_for(mode), self.buy_cursor.as_ref())
        {
            self.note = Some(format!("The mode was not changed: {e}"));
            return;
        }
        self.buy_cursor = None;
        self.buy_dragging = None;
        self.buy_grab_offset = [0, 0];
        self.buy_commit_on_release = false;
        self.buy_in_house = false;
        self.buy_selected = None;
        self.wall_cursor = None;
        self.build_drag = None;
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
        self.hud.buy_state.selected = None;
        self.close_menu();
        self.reached = None;
        self.listed = None;
        self.highlighted = None;
        if self.hud.controls.mode == Mode::Live && mode != Mode::Live {
            self.live_panel_expanded = self.hud.controls.expanded;
        }
        let prev_mode = self.hud.controls.mode;
        let was_expanded = self.hud.controls.expanded;
        let next_expanded = if mode == Mode::Live {
            self.live_panel_expanded
        } else {
            true
        };
        if mode == Mode::Options && (prev_mode != Mode::Options || !was_expanded) {
            self.hud.controls.options.tab = None;
        }
        self.hud.controls.mode = mode;
        self.hud.controls.expanded = next_expanded;
        if was_expanded != next_expanded {
            self.sync_view_centre();
        }
        self.hud.controls.help = None;
        self.hud.controls.options.help = None;
        self.hud.buy_state.reset();
        self.hud.build_state.reset();
        // Native `0x487191` clears the active selection/cursor while retaining
        // `buy_last_guid` (`tool[0x12]`) and `buy_facing` (`tool[0x10]`).
        self.buy_in_house = false;
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
        self.refresh_world();
        self.hud.relayout_panel(
            self.view.size,
            self.lot.world(),
            self.lot.residential(),
            self.lot.lot_zoning(),
            self.lot_visibility(),
            &self.assets,
            None,
        );
        if mode == Mode::Build {
            self.sync_build_roof_ui();
            self.layout_build();
        }
    }

    /// Arms catalog master `guid` on the Buy Mode placement tool (`0x5065c8`).
    /// Does not create the object until the pointer enters the house view
    /// (`0x487d3d`). Plays `UI_error` if the household cannot afford it.
    fn select_buy_item(&mut self, guid: u32) {
        self.cancel_buy();
        if self.buy_cursor.is_some() {
            return;
        }
        let price = self
            .hud
            .ensure_buy_catalog(&self.assets)
            .get(guid)
            .map(|item| item.price);
        let funds = sims_vm::available_funds(self.lot.world());
        match price {
            Some(price) if price <= 0 || price <= funds => {
                self.buy_selected = Some(guid);
                self.hud.buy_state.selected = Some(guid);
            }
            Some(_) => {
                self.hud.play(crate::hud::ERROR);
                self.buy_selected = None;
                self.hud.buy_state.selected = None;
            }
            None => {
                self.buy_selected = None;
                self.hud.buy_state.selected = None;
            }
        }
        self.layout_buy();
    }

    /// Cancels any active Buy Mode cursor or armed catalog selection
    /// (`0x487105`), deleting the unplaced object and `"Tile Cursor"` without
    /// charging funds. Returns `true` if anything was cancelled. If deleting
    /// the preview fails, the cursor handle is kept so it can be retried.
    pub fn cancel_buy(&mut self) -> bool {
        self.buy_dragging = None;
        self.buy_grab_offset = [0, 0];
        self.buy_commit_on_release = false;
        self.buy_in_house = false;
        let had_coop_preview = self.coop_buy_preview.take().is_some();
        let had_cursor = if let Some(cursor) = self.buy_cursor.take() {
            match self.lot.buy_cancel(&cursor) {
                Ok(()) => {
                    self.refresh_world();
                    true
                }
                Err(e) => {
                    self.buy_cursor = Some(cursor);
                    self.note = Some(format!("The Buy cursor was not cancelled: {e}"));
                    return false;
                }
            }
        } else {
            false
        };
        let had_build_drag = self.build_drag.take().is_some();
        if had_build_drag {
            self.build_preview_shape = None;
        }
        let had_selection = self.buy_selected.take().is_some();
        let had_hud = self.hud.buy_state.selected.take().is_some();
        if had_selection || had_hud || had_cursor || had_build_drag || had_coop_preview {
            if self.hud.controls.mode == Mode::Buy {
                self.layout_buy();
            } else if self.hud.controls.mode == Mode::Build {
                self.layout_build();
            }
            true
        } else {
            false
        }
    }

    /// Rotates the held Buy Mode object by one signed turn step (`0x486cb8` →
    /// `0x421be4(±1)`), returning `true` if a Buy cursor was active.
    pub fn rotate_held(&mut self, clockwise: bool) -> bool {
        if self.coop.is_active() {
            let Some(preview) = self.coop_buy_preview.as_mut() else {
                return false;
            };
            let delta = if clockwise { 2 } else { -2 };
            preview.facing = (preview.facing + delta).rem_euclid(8);
            self.buy_facing = Some(preview.facing);
            return true;
        }
        let Some(mut cursor) = self.buy_cursor.take() else {
            return false;
        };
        let steps = if clockwise { 1 } else { -1 };
        if let Err(e) = self.lot.buy_turn(&mut cursor, steps) {
            self.note = Some(format!("The object was not rotated: {e}"));
        } else {
            self.refresh_world();
        }
        self.buy_cursor = Some(cursor);
        true
    }

    /// Handles pointer movement over the house view in Buy Mode (`0x487d3d`,
    /// `0x485a2e`): on mouse entry (`!self.buy_in_house`), recreates the
    /// armed catalog preview at the tool's stored facing (`0x487d3d` →
    /// `0x486f5a`); then rechecks the previous cursor position (`0x485bda`),
    /// rotates the held object in place while dragging past the 6 px
    /// threshold (`dx^2 + dy^2 > 36`), and otherwise moves the cursor to the
    /// terrain-unprojected tile (including within the 6 px drag radius).
    fn hover_buy(&mut self, point: [i32; 2]) -> bool {
        if self.stopped.is_some() {
            return false;
        }
        if self.coop.is_active() {
            return self.hover_buy_coop(point);
        }
        if !self.buy_in_house {
            self.buy_in_house = true;
            if let Some(guid) = self.buy_selected {
                // Native `0x487d3d` → `0x486f5a`: destroy any hidden previous
                // preview (`0x487105`) and instantiate a fresh one using the
                // tool's stored facing (`tool[0x10]`), resetting it to
                // `default_buy_direction` only when switching catalog items.
                if let Some(old) = self.buy_cursor.take()
                    && let Err(e) = self.lot.buy_cancel(&old)
                {
                    self.buy_cursor = Some(old);
                    self.buy_in_house = false;
                    self.note = Some(format!("Could not reset Buy preview: {e}"));
                    return true;
                }
                if self.buy_last_guid != Some(guid) {
                    self.buy_facing =
                        Some(sims_vm::default_buy_direction(self.view.camera.rotation));
                    self.buy_last_guid = Some(guid);
                }
                let dir = self
                    .buy_facing
                    .unwrap_or_else(|| sims_vm::default_buy_direction(self.view.camera.rotation));
                match self.lot.buy_start(guid, self.view.camera.level, dir) {
                    Ok(Some(cursor)) => self.buy_cursor = Some(cursor),
                    Ok(None) => {
                        self.hud.play(crate::hud::ERROR);
                        self.buy_selected = None;
                        self.hud.buy_state.selected = None;
                        self.layout_buy();
                        return true;
                    }
                    Err(e) => {
                        self.note = Some(format!("Could not start placing 0x{guid:08X}: {e}"));
                        return true;
                    }
                }
            }
        }
        let Some(mut cursor) = self.buy_cursor.take() else {
            return false;
        };
        // Native `0x485b4c..0x485bda`: ALWAYS recheck the `"Tile Cursor"`'s
        // physical on-lot position on the currently displayed floor
        // (`0x485ba2`) BEFORE checking the drag threshold.
        let level = self.view.camera.level;
        if let Err(e) = self.lot.buy_recheck(&mut cursor, level) {
            self.note = Some(format!("Could not recheck Buy placement: {e}"));
        }
        let rotated = if let Some(anchor) = self.buy_dragging {
            let dx = point[0] - anchor[0];
            let dy = point[1] - anchor[1];
            if dx * dx + dy * dy > 36 {
                // Native `0x485d5d..0x485dad`: `iVar6 = dy`, `iVar8 = dx`.
                // Upper-left -> 6, upper-right -> 4, lower-left -> 0,
                // lower-right -> 2. Compare `desired` against post-`0x485bda`
                // current facing FIRST (`0x485da9`), and only if they differ
                // subtract the OBJD `+0x7a` facing offset (`0x434f8d`) before
                // turning.
                let quad = if dy < 0 {
                    if dx < 0 { 6 } else { 4 }
                } else if dx < 0 {
                    0
                } else {
                    2
                };
                let desired = (quad - 2 * i16::from(self.view.camera.rotation & 3)).rem_euclid(8);
                let current = self
                    .lot
                    .world()
                    .objects
                    .get(&cursor.held)
                    .and_then(|o| o.data.get(1).copied())
                    .unwrap_or(0)
                    .rem_euclid(8);
                if current != desired {
                    let target = (desired - cursor.facing_offset).rem_euclid(8);
                    if let Err(e) = self.lot.buy_rotate(&mut cursor, target) {
                        self.note = Some(format!("The object was not rotated: {e}"));
                    }
                }
                // Native `0x485e59` → `0x485e72`: test Tile Cursor fit on the
                // existing preview in place on the displayed floor (without an
                // extra TakeOff) and place if it fits, including when the
                // facing was unchanged.
                if let Err(e) = self.lot.buy_put_if_fits(&mut cursor, level) {
                    self.note = Some(format!("Could not finalize Buy placement: {e}"));
                }
                true
            } else {
                false
            }
        } else {
            false
        };
        if !rotated {
            let grab = if cursor.is_existing() {
                self.buy_grab_offset
            } else {
                [0, 0]
            };
            let at = [point[0] - grab[0] - pick::ORIGIN_COLUMN, point[1] - grab[1]];
            let fine = sims_render::unproject_terrain(&self.lot_shape, &self.view.camera, at)
                .unwrap_or_else(|| self.view.camera.unproject_fine(at));
            if let Err(e) = self.lot.buy_move(&mut cursor, fine, self.view.camera.level) {
                self.note = Some(format!("Could not move Buy cursor: {e}"));
            }
        }
        self.buy_cursor = Some(cursor);
        self.refresh_world();
        true
    }

    /// Hides the active Buy Mode cursor off the lot on mouse exit (`0x487d4e`).
    fn hide_buy_cursor(&mut self) -> bool {
        let was_in = std::mem::replace(&mut self.buy_in_house, false);
        if !was_in {
            return false;
        }
        let Some(mut cursor) = self.buy_cursor.take() else {
            return false;
        };
        if let Err(e) = self.lot.buy_hide(&mut cursor) {
            self.note = Some(format!("Could not hide Buy cursor: {e}"));
        }
        self.buy_cursor = Some(cursor);
        self.refresh_world();
        true
    }

    /// Commits a Buy Mode placement on mouse release (`0x4864d0` → `0x486ada`
    /// → `0x421a0e`, `0x4929fb`). For repeat-buy items (`cursor.keep_buying`,
    /// OBJD word 96 == 1 outside tutorial), immediately starts another copy
    /// with the placed item's facing (`0x4864d0` → `0x486f5a`) if affordable.
    fn commit_buy(&mut self, point: [i32; 2]) -> Outcome {
        if self.coop.is_active() {
            return self.commit_buy_coop();
        }
        let Some(cursor) = self.buy_cursor.take() else {
            return Outcome::default();
        };
        if !cursor.valid {
            self.buy_cursor = Some(cursor);
            self.hud.play(crate::hud::ERROR);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return Outcome::redraw();
        }
        let last_facing = self
            .lot
            .world()
            .objects
            .get(&cursor.held)
            .and_then(|o| o.data.get(1).copied())
            .unwrap_or_else(|| sims_vm::default_buy_direction(self.view.camera.rotation));
        match self.lot.buy_commit(&cursor) {
            Ok(_) => {
                self.buy_grab_offset = [0, 0];
                self.buy_commit_on_release = false;
                self.dirty = true;
                // Native `0x486ada` line `0x486af6`: remember the committed
                // facing on the tool (`tool[0x10]`).
                self.buy_facing = Some(last_facing);
                self.buy_last_guid = Some(cursor.guid);
                // Native `0x4864d0` line `0x4867e8`: repeat-buy (`keep_buying`
                // or `VK_SHIFT`) runs only for newly bought catalog objects
                // (`+0x75 != 0`), never when dropping a moved existing object.
                if !cursor.is_existing() && (cursor.keep_buying || self.shift_held) {
                    let funds = sims_vm::available_funds(self.lot.world());
                    if cursor.price <= 0 || cursor.price <= funds {
                        self.buy_selected = Some(cursor.guid);
                        self.hud.buy_state.selected = Some(cursor.guid);
                        match self
                            .lot
                            .buy_start(cursor.guid, self.view.camera.level, last_facing)
                        {
                            Ok(Some(next)) => {
                                self.buy_cursor = Some(next);
                                self.buy_in_house = !self.hud.covers(point);
                                self.refresh_world();
                                if !self.hud.covers(point) {
                                    self.hover_buy(point);
                                }
                            }
                            Ok(None) => {
                                self.hud.play(crate::hud::ERROR);
                                self.buy_selected = None;
                                self.hud.buy_state.selected = None;
                                self.refresh_world();
                            }
                            Err(e) => {
                                self.note = Some(format!(
                                    "Could not start repeat purchase 0x{:08X}: {e}",
                                    cursor.guid
                                ));
                                self.buy_selected = None;
                                self.hud.buy_state.selected = None;
                                self.refresh_world();
                            }
                        }
                    } else {
                        self.hud.play(crate::hud::ERROR);
                        self.buy_selected = None;
                        self.hud.buy_state.selected = None;
                        self.refresh_world();
                    }
                } else {
                    self.buy_selected = None;
                    self.hud.buy_state.selected = None;
                    self.refresh_world();
                }
                if self.hud.controls.mode == Mode::Build {
                    self.layout_build();
                } else {
                    self.layout_buy();
                }
            }
            Err(e) => {
                // `Lot::buy_commit` rolled back the world; keep the cursor
                // handle so the preview objects remain manageable.
                self.buy_cursor = Some(cursor);
                self.note = Some(format!("The item was not placed: {e}"));
                self.hud.play(crate::hud::ERROR);
            }
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    /// The Build Mode panel state (active tool, page, selected item).
    pub fn build_state(&self) -> sims_ui::build::BuildState {
        self.hud.build_state
    }

    /// The Build Mode panel layout as last built.
    pub fn build_layout(&self) -> &sims_ui::build::BuildLayout {
        self.hud.build_layout()
    }

    /// Rebuilds the cached Build Mode panel layout for the current viewport and
    /// Undo/Redo availability.
    fn layout_build(&mut self) {
        let can_undo = self.lot.can_undo();
        let can_redo = self.lot.can_redo();
        self.hud.layout_build(
            self.view.size,
            self.lot.world(),
            &self.assets,
            can_undo,
            can_redo,
        );
    }

    /// Selects one of the 12 Build Mode tools (`cWinArch`, `0x501197`),
    /// managing `"Wall Cursor"` (`0x00000439`) activation/deactivation
    /// (`0x48b68b`, `0x48a66a`, `0x48a6e5`).
    pub fn select_build_tool(&mut self, tool: BuildTool) {
        self.cancel_buy();
        self.build_drag = None;
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
        let catalog = self.hud.ensure_build_catalog(&self.assets).clone();
        self.hud.build_state.select_tool(tool, &catalog);
        if self.coop.is_active() {
            // Never open or close the VM `"Wall Cursor"` object out-of-band in
            // co-op mode; Build previews remain purely local until lockstep commit.
            if matches!(
                tool,
                BuildTool::Terrain | BuildTool::Water | BuildTool::Roof
            ) {
                self.note = Some(
                    "Terrain, Water/Pool, and Roof tools are not yet supported in co-op mode."
                        .into(),
                );
            } else if tool == BuildTool::Roof {
                self.sync_build_roof_ui();
            }
            self.layout_build();
            return;
        }
        match tool {
            BuildTool::Wall => {
                let mode = self.current_wall_drag_mode().cursor_mode();
                self.open_wall_cursor(mode);
            }
            BuildTool::Wallpaper => {
                self.open_wall_cursor(2);
            }
            BuildTool::Roof => {
                self.close_wall_cursor();
                self.sync_build_roof_ui();
            }
            _ => {
                self.close_wall_cursor();
            }
        }
        // 1.91 opens every object catalog tool with its first item picked up
        // (oracle `build-doors/windows/stairs/fireplace/plants.png` all show
        // slot 0 in the selected state). Skip it when the item is not
        // affordable so a tool click never plays the error sound.
        if matches!(
            tool,
            BuildTool::Doors
                | BuildTool::Windows
                | BuildTool::Stairs
                | BuildTool::Fireplace
                | BuildTool::Plants
        ) && self.hud.build_state.selected.is_none()
            && let Some(first) = catalog.items_for(tool).first()
            && (first.price <= 0 || first.price <= sims_vm::available_funds(self.lot.world()))
        {
            self.select_build_item(first.id);
        }
        self.layout_build();
        self.refresh_world();
    }

    /// Applies a Build Mode Part 2 subtool button or roof pattern/pitch selection
    /// (`Terrain`, `Water`/`Pool`, `RoofPitch`, `RoofPattern`, or narrow-viewport `ToolPage`).
    pub fn apply_build_subtool(&mut self, action: BuildSubtoolAction) -> bool {
        if self.hud.controls.mode != Mode::Build {
            return false;
        }
        let catalog = self.hud.ensure_build_catalog(&self.assets).clone();
        if self.coop.is_active()
            && matches!(
                action,
                BuildSubtoolAction::RoofPitch(_)
                    | BuildSubtoolAction::RoofPattern(_)
                    | BuildSubtoolAction::Terrain(_)
                    | BuildSubtoolAction::Water(_)
            )
        {
            self.note = Some(
                "Terrain, Water/Pool, and Roof tools are not yet supported in co-op mode.".into(),
            );
            self.hud.play(crate::hud::ERROR);
            self.layout_build();
            return false;
        }
        let state_changed = self.hud.build_state.apply_subtool(action, &catalog);
        match action {
            BuildSubtoolAction::Terrain(_) | BuildSubtoolAction::Water(_) => {
                self.cancel_buy();
                self.build_drag = None;
                self.build_preview_shape = None;
                self.terrain_hover_overlay = None;
                self.water_hover_overlay = None;
                self.close_wall_cursor();
                self.hud.play(crate::hud::CLICK);
            }
            BuildSubtoolAction::RoofPitch(preset) => {
                self.cancel_buy();
                self.build_drag = None;
                self.build_preview_shape = None;
                self.terrain_hover_overlay = None;
                self.water_hover_overlay = None;
                self.close_wall_cursor();
                let plan = sims_vm::plan_roof_pitch_with_base(
                    self.lot.world(),
                    &self.base_roof,
                    vm_roof_pitch(preset),
                );
                if plan.valid {
                    match self.lot.build_commit_roof(&plan) {
                        Ok(true) => {
                            self.dirty = true;
                            self.hud.play(crate::hud::CLICK);
                        }
                        Ok(false) => {}
                        Err(e) => {
                            self.note = Some(format!("Roof pitch change failed: {e}"));
                            self.hud.play(crate::hud::ERROR);
                        }
                    }
                } else {
                    self.hud.play(crate::hud::CLICK);
                }
                self.sync_build_roof_ui();
            }
            BuildSubtoolAction::RoofPattern(idx) => {
                self.cancel_buy();
                self.build_drag = None;
                self.build_preview_shape = None;
                self.terrain_hover_overlay = None;
                self.water_hover_overlay = None;
                self.close_wall_cursor();
                if let Some(pat) = catalog.roof_patterns().get(idx) {
                    let plan = sims_vm::plan_roof_pattern_with_base(
                        self.lot.world(),
                        &self.base_roof,
                        &pat.stem,
                    );
                    if plan.valid {
                        match self.lot.build_commit_roof(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                self.hud.play(crate::hud::CLICK);
                            }
                            Ok(false) => {}
                            Err(e) => {
                                self.note = Some(format!("Roof pattern change failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else {
                        self.hud.play(crate::hud::CLICK);
                    }
                }
                self.sync_build_roof_ui();
            }
            BuildSubtoolAction::ToolPage(_) => {
                self.hud.play(crate::hud::CLICK);
            }
        }
        self.layout_build();
        state_changed
    }

    /// Synchronizes `hud.build_state`'s Roof pitch & pattern index from the lot's
    /// live roof state (or `HOUS 0` baseline) so the UI and Undo/Redo always match.
    fn sync_build_roof_ui(&mut self) {
        let state = self.lot.roof_state_with_base(&self.base_roof);
        self.hud.build_state.roof_pitch = ui_roof_pitch_preset(state.pitch);
        let catalog = self.hud.ensure_build_catalog(&self.assets);
        let target = state.pattern.trim().to_ascii_lowercase();
        let target_stem = target.strip_suffix(".bmp").unwrap_or(target.as_str());
        if let Some(idx) = catalog.roof_patterns().iter().position(|p| {
            let stem = p.stem.trim().to_ascii_lowercase();
            let s = stem.strip_suffix(".bmp").unwrap_or(stem.as_str());
            s == target_stem
        }) {
            self.hud.build_state.roof_pattern = idx;
        }
    }

    fn draw_roof_preview(&mut self) {
        if self.roof_catalog.is_none() {
            if let Ok(cat) = RoofCatalog::load(self.hud.install()) {
                self.roof_catalog = Some(cat);
            } else {
                return;
            }
        }
        let Some(catalog) = self.roof_catalog.as_ref() else {
            return;
        };
        let state = self.lot.roof_state_with_base(&self.base_roof);
        let config = RoofPreviewConfig {
            pattern_name: state.pattern,
            style: RoofStyle::Hip,
            pitch: render_roof_pitch(state.pitch),
            custom_slope: None,
            level: None,
            footprint: None,
        };
        let _ = render_build::draw_roof_preview_from_catalog(
            &self.lot_shape,
            &self.view.camera,
            catalog,
            &config,
            &mut self.canvas,
        );
    }

    fn current_terrain_mode(&self) -> TerrainMode {
        match self.hud.build_state.terrain_subtool {
            TerrainSubtool::Raise => {
                if self.ctrl_held {
                    TerrainMode::Lower
                } else {
                    TerrainMode::Raise
                }
            }
            TerrainSubtool::Lower => {
                if self.ctrl_held {
                    TerrainMode::Raise
                } else {
                    TerrainMode::Lower
                }
            }
            TerrainSubtool::Level => {
                if self.shift_held {
                    TerrainMode::LevelAverage
                } else {
                    TerrainMode::Level
                }
            }
            TerrainSubtool::Grass => {
                if self.ctrl_held {
                    TerrainMode::RemoveGrass
                } else {
                    TerrainMode::RestoreGrass
                }
            }
        }
    }

    fn current_water_mode(&self) -> WaterMode {
        if self.ctrl_held {
            WaterMode::Remove
        } else {
            WaterMode::Paint
        }
    }

    fn current_pool_mode(&self) -> PoolMode {
        if self.ctrl_held {
            PoolMode::Demolish
        } else {
            PoolMode::Dig
        }
    }

    /// Steps the Build Mode catalog grid page by `delta`.
    pub fn page_build(&mut self, delta: i32) -> bool {
        if self.hud.controls.mode != Mode::Build || !self.hud.controls.expanded {
            return false;
        }
        let changed = self.hud.build_state.step_page(delta);
        self.layout_build();
        changed
    }

    /// Selects a Build Mode catalog entry (wall/fence style, floor, wallpaper,
    /// or door/window/column/stair/fireplace/plant object).
    pub fn select_build_item(&mut self, item: BuildItemId) {
        self.cancel_buy();
        self.build_drag = None;
        self.build_preview_shape = None;
        self.hud.build_state.select_item(item);
        match item {
            BuildItemId::WallStyle(_) => {
                let mode = self.current_wall_drag_mode().cursor_mode();
                self.open_wall_cursor(mode);
            }
            BuildItemId::Wallpaper(_) => {
                self.open_wall_cursor(2);
            }
            BuildItemId::Floor(_) => {
                self.close_wall_cursor();
            }
            BuildItemId::Object(guid) => {
                self.close_wall_cursor();
                let price = self
                    .hud
                    .ensure_build_catalog(&self.assets)
                    .get(item)
                    .map(|e| e.price);
                let funds = sims_vm::available_funds(self.lot.world());
                match price {
                    Some(p) if p <= 0 || p <= funds => {
                        self.buy_selected = Some(guid);
                    }
                    Some(_) => {
                        self.hud.play(crate::hud::ERROR);
                        self.buy_selected = None;
                        self.hud.build_state.selected = None;
                    }
                    None => {
                        self.buy_selected = None;
                    }
                }
            }
        }
        self.layout_build();
        self.refresh_world();
    }

    fn open_wall_cursor(&mut self, mode: i16) {
        if self.coop.is_active_or_starting() {
            return;
        }
        if let Some(mut cur) = self.wall_cursor.take() {
            if cur.mode != mode {
                let pos = cur.position;
                let _ = self.lot.build_cursor_move(&mut cur, pos, None, Some(mode));
                self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
            }
            self.wall_cursor = Some(cur);
            return;
        }
        match self.lot.build_cursor_open(mode) {
            Ok(cursor) => {
                self.wall_cursor = Some(cursor);
                self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
            }
            Err(e) => self.note = Some(format!("Could not open Wall Cursor: {e}")),
        }
    }

    fn close_wall_cursor(&mut self) {
        if self.coop.is_active() {
            self.wall_cursor = None;
            return;
        }
        if let Some(cursor) = self.wall_cursor.take() {
            if let Err(e) = self.lot.build_cursor_close(&cursor) {
                self.note = Some(format!("Could not close Wall Cursor: {e}"));
            }
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        } else {
            let _ = self.lot.build_cursor_close_active();
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        }
    }

    fn current_wall_drag_mode(&self) -> WallDragMode {
        match (self.shift_held, self.ctrl_held) {
            (false, false) => WallDragMode::Build,
            (false, true) => WallDragMode::Demolish,
            (true, false) => WallDragMode::RoomBox,
            (true, true) => WallDragMode::RoomBoxDemolish,
        }
    }

    fn current_floor_drag_mode(&self) -> FloorDragMode {
        match (self.shift_held, self.ctrl_held) {
            (false, false) => FloorDragMode::Lay,
            (false, true) => FloorDragMode::Remove,
            (true, false) => FloorDragMode::RoomFillLay,
            (true, true) => FloorDragMode::RoomFillRemove,
        }
    }

    fn current_wallpaper_drag_mode(&self) -> WallpaperDragMode {
        match (self.shift_held, self.ctrl_held) {
            (false, false) => WallpaperDragMode::Apply,
            (false, true) => WallpaperDragMode::Strip,
            (true, false) => WallpaperDragMode::RoomFillApply,
            (true, true) => WallpaperDragMode::RoomFillStrip,
        }
    }

    fn unproject_build_fine(&self, point: [i32; 2]) -> [i32; 2] {
        let at = [point[0] - pick::ORIGIN_COLUMN, point[1]];
        sims_render::unproject_terrain(&self.lot_shape, &self.view.camera, at)
            .unwrap_or_else(|| self.view.camera.unproject_fine(at))
    }

    /// Computes which diagonal half (`0` or `1`) of tile `(tx, ty)` the fine
    /// coordinates `(fx, fy)` fall into for a split diagonal floor tile.
    fn split_half_at(&self, fine: [i32; 2], tx: i32, ty: i32, level: u8) -> Option<u8> {
        let level_idx = usize::from(level.saturating_sub(1)).min(1);
        let size = self.lot_shape.size;
        if tx < 0 || ty < 0 || tx as usize >= size || ty as usize >= size {
            return None;
        }
        let segs = self.lot_shape.levels[level_idx].walls
            [sims_render::Lot::cell(tx as usize, ty as usize)]
        .segments;
        let lx = fine[0] - tx * 16;
        let ly = fine[1] - ty * 16;
        if (segs & WALL_DESCENDING) != 0 {
            Some(u8::from(lx + ly >= 16))
        } else if (segs & WALL_ASCENDING) != 0 {
            Some(u8::from(lx > ly))
        } else {
            None
        }
    }

    /// Screen-space front-facing wall quad hit-test for the Wallpaper tool
    /// (`cNewWallpaperTool::HitTest` `0x48a709`), accurate across all 3 zooms
    /// and 4 camera rotations.
    fn pick_wall_face(&self, point: [i32; 2]) -> Option<WallFace> {
        let level = u8::try_from(self.view.camera.level)
            .unwrap_or(1)
            .clamp(1, 2);
        let level_idx = usize::from(level - 1);
        let active = self.lot_shape.size;
        if active < 2 {
            return None;
        }
        let wall_h = match (self.view.camera.walls, self.view.camera.zoom) {
            (Walls::Down, sims_render::Zoom::Far) => 6,
            (Walls::Down, sims_render::Zoom::Medium) => 12,
            (Walls::Down, sims_render::Zoom::Near) => 23,
            (_, sims_render::Zoom::Far) => 29,
            (_, sims_render::Zoom::Medium) => 58,
            (_, sims_render::Zoom::Near) => 116,
        };
        let walls = &self.lot_shape.levels[level_idx].walls;
        let lvl = i32::from(level);
        // Gather candidate tiles around the ground unprojections at heights
        // `0..=wall_h` so clicks anywhere on a standing wall are included.
        let mut candidates = BTreeSet::new();
        for dy in [0, wall_h / 3, (wall_h * 2) / 3, wall_h] {
            let fine = self.unproject_build_fine([point[0], point[1] + dy]);
            let cx = fine[0] >> 4;
            let cy = fine[1] >> 4;
            for ox in -2..=2 {
                for oy in -2..=2 {
                    let tx = cx + ox;
                    let ty = cy + oy;
                    if tx >= 1 && ty >= 1 && (tx as usize) < active && (ty as usize) < active {
                        candidates.insert((tx as usize, ty as usize));
                    }
                }
            }
        }

        let px = point[0] as f32;
        let py = point[1] as f32;
        let mut best: Option<(f32, WallFace)> = None;

        let mut test_quad = |c0: [i32; 2],
                             c1: [i32; 2],
                             normal: [i32; 2],
                             face: WallFace,
                             _seg_tile: [i32; 2],
                             _seg_bit: u8| {
            let quad_h = wall_h;
            let mid = [(c0[0] + c1[0]) / 2, (c0[1] + c1[1]) / 2];
            let p_mid = lot_point(&self.view.camera, &self.lot_shape, [mid[0], mid[1], lvl]);
            let p_out = lot_point(
                &self.view.camera,
                &self.lot_shape,
                [mid[0] + normal[0], mid[1] + normal[1], lvl],
            );
            // Front-facing iff outward normal projects downward toward camera (`delta_y > 0`).
            if p_out[1] <= p_mid[1] {
                return;
            }
            let p0 = lot_point(&self.view.camera, &self.lot_shape, [c0[0], c0[1], lvl]);
            let p1 = lot_point(&self.view.camera, &self.lot_shape, [c1[0], c1[1], lvl]);
            let x0 = (p0[0] + pick::ORIGIN_COLUMN) as f32;
            let y0 = p0[1] as f32;
            let x1 = (p1[0] + pick::ORIGIN_COLUMN) as f32;
            let y1 = p1[1] as f32;
            let min_x = x0.min(x1) - 2.0;
            let max_x = x0.max(x1) + 2.0;
            if px < min_x || px > max_x {
                return;
            }
            let span = (x1 - x0).abs();
            let t = if span < 1.0 {
                0.5
            } else {
                ((px - x0) / (x1 - x0)).clamp(0.0, 1.0)
            };
            let y_bot = y0 + t * (y1 - y0);
            if py >= y_bot - quad_h as f32 - 2.0 && py <= y_bot + 3.0 {
                let score = y_bot;
                if best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, face));
                }
            }
        };

        for (x, y) in candidates {
            let segs = walls[sims_render::Lot::cell(x, y)].segments;
            let fx = (x as i32) * 16;
            let fy = (y as i32) * 16;
            let seg_tile = [x as i32, y as i32];
            if (segs & WALL_WEST) != 0 {
                test_quad(
                    [fx, fy],
                    [fx, fy + 16],
                    [4, 0],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_WEST,
                        side: 0,
                    },
                    seg_tile,
                    WALL_WEST,
                );
                if x > 0 {
                    test_quad(
                        [fx, fy],
                        [fx, fy + 16],
                        [-4, 0],
                        WallFace {
                            tile: [(x - 1) as u8, y as u8],
                            bit: WALL_EAST,
                            side: 0,
                        },
                        seg_tile,
                        WALL_WEST,
                    );
                }
            }
            if (segs & WALL_NORTH) != 0 {
                test_quad(
                    [fx, fy],
                    [fx + 16, fy],
                    [0, 4],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_NORTH,
                        side: 0,
                    },
                    seg_tile,
                    WALL_NORTH,
                );
                if y > 0 {
                    test_quad(
                        [fx, fy],
                        [fx + 16, fy],
                        [0, -4],
                        WallFace {
                            tile: [x as u8, (y - 1) as u8],
                            bit: WALL_SOUTH,
                            side: 0,
                        },
                        seg_tile,
                        WALL_NORTH,
                    );
                }
            }
            if (segs & WALL_DESCENDING) != 0 {
                test_quad(
                    [fx + 16, fy],
                    [fx, fy + 16],
                    [-4, -4],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_DESCENDING,
                        side: 0,
                    },
                    seg_tile,
                    WALL_DESCENDING,
                );
                test_quad(
                    [fx + 16, fy],
                    [fx, fy + 16],
                    [4, 4],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_DESCENDING,
                        side: 1,
                    },
                    seg_tile,
                    WALL_DESCENDING,
                );
            }
            if (segs & WALL_ASCENDING) != 0 {
                test_quad(
                    [fx, fy],
                    [fx + 16, fy + 16],
                    [-4, 4],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_ASCENDING,
                        side: 0,
                    },
                    seg_tile,
                    WALL_ASCENDING,
                );
                test_quad(
                    [fx, fy],
                    [fx + 16, fy + 16],
                    [4, -4],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_ASCENDING,
                        side: 1,
                    },
                    seg_tile,
                    WALL_ASCENDING,
                );
            }
        }
        best.map(|(_, f)| f)
    }

    fn hover_build(&mut self, point: [i32; 2]) -> bool {
        if self.stopped.is_some() {
            return false;
        }
        let level = u8::try_from(self.view.camera.level)
            .unwrap_or(1)
            .clamp(1, 2);
        let active = self.lot_shape.size;
        let tool = self.hud.build_state.tool;

        // 1. Update active drag if one is in progress.
        if let Some(mut drag) = self.build_drag.take() {
            let prices = self.hud.build_prices(&self.assets);
            let tool_prices = self.hud.tool_prices(&self.assets);
            match &mut drag {
                BuildDrag::Wall {
                    start_corner,
                    end_corner,
                    style,
                    mode,
                    plan,
                } => {
                    self.terrain_hover_overlay = None;
                    self.water_hover_overlay = None;
                    let fine = self.unproject_build_fine(point);
                    let corner = sims_vm::snap_wall_corner(fine, active);
                    *end_corner = corner;
                    *mode = self.current_wall_drag_mode();
                    if !self.coop.is_active()
                        && let Some(mut cur) = self.wall_cursor.take()
                    {
                        let cfine = sims_vm::wall_cursor_fine(corner, level);
                        let _ = self.lot.build_cursor_move(
                            &mut cur,
                            cfine,
                            Some(0),
                            Some(mode.cursor_mode()),
                        );
                        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                        self.wall_cursor = Some(cur);
                    }
                    let computed = sims_vm::plan_wall_drag(
                        self.lot.world(),
                        *start_corner,
                        *end_corner,
                        level,
                        *style,
                        *mode,
                        &prices,
                    );
                    if computed.valid {
                        let mut preview_world = self.lot.world().clone();
                        sims_vm::preview_wall_on_world(&mut preview_world, &computed);
                        self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                            .ok()
                            .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                    } else {
                        self.build_preview_shape = None;
                    }
                    *plan = Some(computed);
                }
                BuildDrag::Floor {
                    start_tile,
                    end_tile,
                    floor_id,
                    mode,
                    split_half,
                    plan,
                } => {
                    self.terrain_hover_overlay = None;
                    self.water_hover_overlay = None;
                    let fine = self.unproject_build_fine(point);
                    let tile = [fine[0] >> 4, fine[1] >> 4];
                    *end_tile = tile;
                    *mode = self.current_floor_drag_mode();
                    *split_half = self.split_half_at(fine, tile[0], tile[1], level);
                    let computed = sims_vm::plan_floor_edit(
                        self.lot.world(),
                        *start_tile,
                        *end_tile,
                        level,
                        *floor_id,
                        *mode,
                        &prices,
                        *split_half,
                    );
                    if computed.valid {
                        let mut preview_world = self.lot.world().clone();
                        sims_vm::preview_floor_on_world(&mut preview_world, &computed);
                        self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                            .ok()
                            .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                    } else {
                        self.build_preview_shape = None;
                    }
                    *plan = Some(computed);
                }
                BuildDrag::Wallpaper {
                    faces,
                    pattern_id,
                    mode,
                    plan,
                } => {
                    self.terrain_hover_overlay = None;
                    self.water_hover_overlay = None;
                    *mode = self.current_wallpaper_drag_mode();
                    if let Some(face) = self.pick_wall_face(point) {
                        if !self.coop.is_active()
                            && let Some(mut cur) = self.wall_cursor.take()
                        {
                            let cfine = sims_vm::wallpaper_cursor_fine(face, level);
                            let facing =
                                sims_vm::wallpaper_cursor_facing(face, self.view.camera.rotation);
                            let _ = self.lot.build_cursor_move(&mut cur, cfine, facing, Some(2));
                            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                            self.wall_cursor = Some(cur);
                        }
                        if !faces.contains(&face) {
                            faces.push(face);
                        }
                    }
                    let computed = sims_vm::plan_wallpaper_edit(
                        self.lot.world(),
                        faces,
                        level,
                        *pattern_id,
                        *mode,
                        &prices,
                    );
                    if computed.valid {
                        let mut preview_world = self.lot.world().clone();
                        sims_vm::preview_wallpaper_on_world(&mut preview_world, &computed);
                        self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                            .ok()
                            .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                    } else {
                        self.build_preview_shape = None;
                    }
                    *plan = Some(computed);
                }
                BuildDrag::Terrain {
                    start,
                    end,
                    mode,
                    plan,
                } => {
                    self.water_hover_overlay = None;
                    self.build_preview_shape = None;
                    *mode = self.current_terrain_mode();
                    let fine = self.unproject_build_fine(point);
                    *end = terrain_coord_for_mode(fine, active, *mode);
                    let computed = sims_vm::plan_terrain_edit(
                        self.lot.world(),
                        *start,
                        *end,
                        *mode,
                        &tool_prices,
                    );
                    self.terrain_hover_overlay =
                        Some(terrain_overlay_for_drag(*start, *end, *mode));
                    *plan = Some(computed);
                }
                BuildDrag::Water {
                    start_tile,
                    end_tile,
                    mode,
                    plan,
                } => {
                    self.terrain_hover_overlay = None;
                    self.build_preview_shape = None;
                    *mode = self.current_water_mode();
                    let fine = self.unproject_build_fine(point);
                    let tile = [fine[0] >> 4, fine[1] >> 4];
                    *end_tile = tile;
                    let computed = sims_vm::plan_water_edit(
                        self.lot.world(),
                        *start_tile,
                        *end_tile,
                        *mode,
                        &tool_prices,
                    );
                    self.water_hover_overlay = Some(WaterOverlay {
                        tool: RenderWaterTool::Pond,
                        action: if *mode == WaterMode::Remove {
                            WaterAction::Remove
                        } else {
                            WaterAction::Dig
                        },
                        start: *start_tile,
                        end: *end_tile,
                    });
                    *plan = Some(computed);
                }
                BuildDrag::Pool {
                    start_tile,
                    end_tile,
                    mode,
                    plan,
                } => {
                    self.terrain_hover_overlay = None;
                    self.build_preview_shape = None;
                    *mode = self.current_pool_mode();
                    let fine = self.unproject_build_fine(point);
                    let tile = [fine[0] >> 4, fine[1] >> 4];
                    *end_tile = tile;
                    let computed = sims_vm::plan_pool_edit(
                        self.lot.world(),
                        *start_tile,
                        *end_tile,
                        *mode,
                        &tool_prices,
                    );
                    self.water_hover_overlay = Some(WaterOverlay {
                        tool: RenderWaterTool::Pool,
                        action: if *mode == PoolMode::Demolish {
                            WaterAction::Remove
                        } else {
                            WaterAction::Dig
                        },
                        start: *start_tile,
                        end: *end_tile,
                    });
                    *plan = Some(computed);
                }
            }
            self.build_drag = Some(drag);
            return true;
        }

        // 2. Non-dragging hover: move `"Wall Cursor"`, Terrain/Water overlays, and Shift room-fills.
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
        match tool {
            Some(BuildTool::Wall) => {
                let fine = self.unproject_build_fine(point);
                let corner = sims_vm::snap_wall_corner(fine, active);
                let mode = self.current_wall_drag_mode().cursor_mode();
                if !self.coop.is_active()
                    && let Some(mut cur) = self.wall_cursor.take()
                {
                    let cfine = sims_vm::wall_cursor_fine(corner, level);
                    let _ = self
                        .lot
                        .build_cursor_move(&mut cur, cfine, Some(0), Some(mode));
                    self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                    self.wall_cursor = Some(cur);
                }
                true
            }
            Some(BuildTool::Wallpaper) => {
                if let Some(face) = self.pick_wall_face(point) {
                    if !self.coop.is_active()
                        && let Some(mut cur) = self.wall_cursor.take()
                    {
                        let cfine = sims_vm::wallpaper_cursor_fine(face, level);
                        let facing =
                            sims_vm::wallpaper_cursor_facing(face, self.view.camera.rotation);
                        let _ = self.lot.build_cursor_move(&mut cur, cfine, facing, Some(2));
                        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                        self.wall_cursor = Some(cur);
                    }
                    if self.shift_held {
                        let pat = self.hud.build_state.last_wallpaper.unwrap_or(1);
                        let prices = self.hud.build_prices(&self.assets);
                        let mode = self.current_wallpaper_drag_mode();
                        let plan = sims_vm::plan_wallpaper_edit(
                            self.lot.world(),
                            &[face],
                            level,
                            pat,
                            mode,
                            &prices,
                        );
                        if plan.valid {
                            let mut preview_world = self.lot.world().clone();
                            sims_vm::preview_wallpaper_on_world(&mut preview_world, &plan);
                            self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                                .ok()
                                .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                        }
                    }
                    true
                } else {
                    self.hide_build_cursor()
                }
            }
            Some(BuildTool::Floor) if self.shift_held => {
                let fine = self.unproject_build_fine(point);
                let tile = [fine[0] >> 4, fine[1] >> 4];
                let split_half = self.split_half_at(fine, tile[0], tile[1], level);
                let fid = self.hud.build_state.last_floor.unwrap_or(1);
                let prices = self.hud.build_prices(&self.assets);
                let mode = self.current_floor_drag_mode();
                let plan = sims_vm::plan_floor_edit(
                    self.lot.world(),
                    tile,
                    tile,
                    level,
                    fid,
                    mode,
                    &prices,
                    split_half,
                );
                if plan.valid {
                    let mut preview_world = self.lot.world().clone();
                    sims_vm::preview_floor_on_world(&mut preview_world, &plan);
                    self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                        .ok()
                        .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                    return true;
                }
                false
            }
            Some(BuildTool::Terrain) => {
                let mode = self.current_terrain_mode();
                let fine = self.unproject_build_fine(point);
                let coord = terrain_coord_for_mode(fine, active, mode);
                self.terrain_hover_overlay = Some(terrain_overlay_for_drag(coord, coord, mode));
                true
            }
            Some(BuildTool::Water) => {
                let fine = self.unproject_build_fine(point);
                let tile = [fine[0] >> 4, fine[1] >> 4];
                let (wtool, action) = match self.hud.build_state.water_subtool {
                    WaterSubtool::Pool => (
                        RenderWaterTool::Pool,
                        if self.ctrl_held {
                            WaterAction::Remove
                        } else {
                            WaterAction::Dig
                        },
                    ),
                    WaterSubtool::Water => (
                        RenderWaterTool::Pond,
                        if self.ctrl_held {
                            WaterAction::Remove
                        } else {
                            WaterAction::Dig
                        },
                    ),
                };
                self.water_hover_overlay = Some(WaterOverlay {
                    tool: wtool,
                    action,
                    start: tile,
                    end: tile,
                });
                true
            }
            _ => false,
        }
    }

    fn hide_build_cursor(&mut self) -> bool {
        let had_overlay = self.build_preview_shape.take().is_some()
            || self.terrain_hover_overlay.take().is_some()
            || self.water_hover_overlay.take().is_some();
        if self.coop.is_active() {
            self.wall_cursor = None;
            return had_overlay;
        }
        let Some(mut cur) = self.wall_cursor.take() else {
            return had_overlay;
        };
        let was_on_lot = cur.position[0] >= 0;
        if was_on_lot {
            let _ = self.lot.build_cursor_hide(&mut cur);
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        }
        self.wall_cursor = Some(cur);
        was_on_lot || had_overlay
    }

    fn press_build(&mut self, point: [i32; 2]) -> Outcome {
        let level = u8::try_from(self.view.camera.level)
            .unwrap_or(1)
            .clamp(1, 2);
        let active = self.lot_shape.size;
        self.pointer = Some(point);
        match self.hud.build_state.tool {
            Some(BuildTool::Wall) => {
                let fine = self.unproject_build_fine(point);
                let corner = sims_vm::snap_wall_corner(fine, active);
                let style = WallStyle::from_style_id(self.hud.build_state.last_wall_style)
                    .unwrap_or(WallStyle::Standard);
                let mode = self.current_wall_drag_mode();
                if !self.coop.is_active()
                    && let Some(mut cur) = self.wall_cursor.take()
                {
                    let cfine = sims_vm::wall_cursor_fine(corner, level);
                    let _ = self.lot.build_cursor_move(
                        &mut cur,
                        cfine,
                        Some(0),
                        Some(mode.cursor_mode()),
                    );
                    self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                    self.wall_cursor = Some(cur);
                }
                self.build_drag = Some(BuildDrag::Wall {
                    start_corner: corner,
                    end_corner: corner,
                    style,
                    mode,
                    plan: None,
                });
                self.hud.play(crate::hud::BLD_DRAG_DOWN);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            Some(BuildTool::Floor) => {
                let fine = self.unproject_build_fine(point);
                let tile = [fine[0] >> 4, fine[1] >> 4];
                let split_half = self.split_half_at(fine, tile[0], tile[1], level);
                let floor_id = self
                    .hud
                    .build_state
                    .last_floor
                    .or_else(|| self.hud.ensure_build_catalog(&self.assets).default_floor())
                    .unwrap_or(1);
                let mode = self.current_floor_drag_mode();
                let prices = self.hud.build_prices(&self.assets);
                let plan = sims_vm::plan_floor_edit(
                    self.lot.world(),
                    tile,
                    tile,
                    level,
                    floor_id,
                    mode,
                    &prices,
                    split_half,
                );
                if plan.valid {
                    let mut preview_world = self.lot.world().clone();
                    sims_vm::preview_floor_on_world(&mut preview_world, &plan);
                    self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                        .ok()
                        .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                }
                self.build_drag = Some(BuildDrag::Floor {
                    start_tile: tile,
                    end_tile: tile,
                    floor_id,
                    mode,
                    split_half,
                    plan: Some(plan),
                });
                self.hud.play(crate::hud::BLD_DRAG_DOWN);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            Some(BuildTool::Wallpaper) => {
                let Some(face) = self.pick_wall_face(point) else {
                    self.hud.play(crate::hud::ERROR);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    return Outcome::redraw();
                };
                if !self.coop.is_active()
                    && let Some(mut cur) = self.wall_cursor.take()
                {
                    let cfine = sims_vm::wallpaper_cursor_fine(face, level);
                    let facing = sims_vm::wallpaper_cursor_facing(face, self.view.camera.rotation);
                    let _ = self.lot.build_cursor_move(&mut cur, cfine, facing, Some(2));
                    self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                    self.wall_cursor = Some(cur);
                }
                let pattern_id = self
                    .hud
                    .build_state
                    .last_wallpaper
                    .or_else(|| {
                        self.hud
                            .ensure_build_catalog(&self.assets)
                            .default_wallpaper()
                    })
                    .unwrap_or(1);
                let mode = self.current_wallpaper_drag_mode();
                let prices = self.hud.build_prices(&self.assets);
                let plan = sims_vm::plan_wallpaper_edit(
                    self.lot.world(),
                    &[face],
                    level,
                    pattern_id,
                    mode,
                    &prices,
                );
                if plan.valid {
                    let mut preview_world = self.lot.world().clone();
                    sims_vm::preview_wallpaper_on_world(&mut preview_world, &plan);
                    self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                        .ok()
                        .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                }
                self.build_drag = Some(BuildDrag::Wallpaper {
                    faces: vec![face],
                    pattern_id,
                    mode,
                    plan: Some(plan),
                });
                self.hud.play(crate::hud::BLD_DRAG_DOWN);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            Some(BuildTool::Terrain) => {
                let mode = self.current_terrain_mode();
                let fine = self.unproject_build_fine(point);
                let coord = terrain_coord_for_mode(fine, active, mode);
                let tool_prices = self.hud.tool_prices(&self.assets);
                let plan =
                    sims_vm::plan_terrain_edit(self.lot.world(), coord, coord, mode, &tool_prices);
                self.terrain_hover_overlay = Some(terrain_overlay_for_drag(coord, coord, mode));
                self.build_drag = Some(BuildDrag::Terrain {
                    start: coord,
                    end: coord,
                    mode,
                    plan: Some(plan),
                });
                self.hud.play(crate::hud::BLD_DRAG_DOWN);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            Some(BuildTool::Water) => {
                let fine = self.unproject_build_fine(point);
                let tile = [fine[0] >> 4, fine[1] >> 4];
                let tool_prices = self.hud.tool_prices(&self.assets);
                match self.hud.build_state.water_subtool {
                    WaterSubtool::Pool => {
                        let mode = self.current_pool_mode();
                        let plan = sims_vm::plan_pool_edit(
                            self.lot.world(),
                            tile,
                            tile,
                            mode,
                            &tool_prices,
                        );
                        self.water_hover_overlay = Some(WaterOverlay {
                            tool: RenderWaterTool::Pool,
                            action: if mode == PoolMode::Demolish {
                                WaterAction::Remove
                            } else {
                                WaterAction::Dig
                            },
                            start: tile,
                            end: tile,
                        });
                        self.build_drag = Some(BuildDrag::Pool {
                            start_tile: tile,
                            end_tile: tile,
                            mode,
                            plan: Some(plan),
                        });
                    }
                    WaterSubtool::Water => {
                        let mode = self.current_water_mode();
                        let plan = sims_vm::plan_water_edit(
                            self.lot.world(),
                            tile,
                            tile,
                            mode,
                            &tool_prices,
                        );
                        self.water_hover_overlay = Some(WaterOverlay {
                            tool: RenderWaterTool::Pond,
                            action: if mode == WaterMode::Remove {
                                WaterAction::Remove
                            } else {
                                WaterAction::Dig
                            },
                            start: tile,
                            end: tile,
                        });
                        self.build_drag = Some(BuildDrag::Water {
                            start_tile: tile,
                            end_tile: tile,
                            mode,
                            plan: Some(plan),
                        });
                    }
                }
                self.hud.play(crate::hud::BLD_DRAG_DOWN);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            _ => Outcome::default(),
        }
    }

    fn release_build(&mut self, point: [i32; 2]) -> Outcome {
        if self.hud.covers(point) {
            self.build_drag = None;
            self.build_preview_shape = None;
            self.terrain_hover_overlay = None;
            self.water_hover_overlay = None;
            self.hide_build_cursor();
            return Outcome::redraw();
        }
        self.pointer = Some(point);
        self.hover_build(point);
        let Some(drag) = self.build_drag.take() else {
            return Outcome::default();
        };
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
        if self.coop.is_active() {
            return self.commit_build_coop(drag);
        }
        match drag {
            BuildDrag::Wall {
                start_corner,
                end_corner,
                mode,
                plan,
                ..
            } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_wall(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                if mode.is_demolish() {
                                    self.hud.play(if plan.net_cost < 0 {
                                        crate::hud::MONEYBACK
                                    } else {
                                        crate::hud::ARCHTOOL_DELETE
                                    });
                                } else {
                                    self.hud.play(crate::hud::BLD_DRAG_UP);
                                }
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Wall edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if start_corner != end_corner {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
            BuildDrag::Floor { mode, plan, .. } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_floor(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                if mode.is_remove() {
                                    self.hud.play(if plan.net_cost < 0 {
                                        crate::hud::MONEYBACK
                                    } else {
                                        crate::hud::ARCHTOOL_DELETE
                                    });
                                } else {
                                    self.hud.play(crate::hud::BLD_DRAG_UP);
                                }
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Floor edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if !plan.affordable {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
            BuildDrag::Wallpaper { mode, plan, .. } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_wallpaper(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                if mode.is_strip() {
                                    self.hud.play(if plan.net_cost < 0 {
                                        crate::hud::MONEYBACK
                                    } else {
                                        crate::hud::ARCHTOOL_DELETE
                                    });
                                } else {
                                    self.hud.play(crate::hud::BLD_DRAG_UP);
                                }
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Wallpaper edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if !plan.affordable {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
            BuildDrag::Terrain { plan, .. } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_terrain(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                self.hud.play(crate::hud::BLD_DRAG_UP);
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Terrain edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if !plan.affordable {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
            BuildDrag::Water { mode, plan, .. } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_water(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                if mode == WaterMode::Remove {
                                    self.hud.play(if plan.net_cost < 0 {
                                        crate::hud::MONEYBACK
                                    } else {
                                        crate::hud::ARCHTOOL_DELETE
                                    });
                                } else {
                                    self.hud.play(crate::hud::BLD_DRAG_UP);
                                }
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Water edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if !plan.affordable {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
            BuildDrag::Pool { mode, plan, .. } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_pool(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                if mode == PoolMode::Demolish {
                                    self.hud.play(if plan.net_cost < 0 {
                                        crate::hud::MONEYBACK
                                    } else {
                                        crate::hud::ARCHTOOL_DELETE
                                    });
                                } else {
                                    self.hud.play(crate::hud::BLD_DRAG_UP);
                                }
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Pool edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if !plan.affordable {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
        }
        self.refresh_world();
        self.layout_build();
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    /// Draws the floating Architecture cost/refund tag next to the cursor while
    /// dragging a wall, floor, wallpaper, terrain, water, or pool edit.
    fn draw_build_cost_label(&mut self) {
        let Some([px, py]) = self.pointer else {
            return;
        };
        let (cost, affordable) = match &self.build_drag {
            Some(BuildDrag::Wall { plan: Some(p), .. }) if p.valid => (p.net_cost, p.affordable),
            Some(BuildDrag::Floor { plan: Some(p), .. }) if p.valid => (p.net_cost, p.affordable),
            Some(BuildDrag::Wallpaper { plan: Some(p), .. }) if p.valid => {
                (p.net_cost, p.affordable)
            }
            Some(BuildDrag::Terrain { plan: Some(p), .. }) if p.valid => (p.net_cost, p.affordable),
            Some(BuildDrag::Water { plan: Some(p), .. }) if p.valid => (p.net_cost, p.affordable),
            Some(BuildDrag::Pool { plan: Some(p), .. }) if p.valid => (p.net_cost, p.affordable),
            _ => return,
        };
        if cost == 0 {
            return;
        }
        let Some(font) = self.hud.art.font(10) else {
            return;
        };
        let label = if cost > 0 {
            format!("${cost}")
        } else {
            format!("+${}", -cost)
        };
        let (w, h) = (
            u32::try_from(self.canvas.width).unwrap_or(0),
            u32::try_from(self.canvas.height).unwrap_or(0),
        );
        let Ok(mut frame) =
            sims_ui::frame::Frame::from_packed(w, h, std::mem::take(&mut self.canvas.color))
        else {
            return;
        };
        let text_w = font.width(&label).max(20);
        let box_w = text_w + 10;
        let box_h = font.line_height + 6;
        let bx = (px + 14).clamp(4, (w as i32 - box_w - 4).max(4));
        let by = (py - box_h - 6).clamp(4, (h as i32 - box_h - 4).max(4));
        let bg = if affordable {
            [20, 32, 92]
        } else {
            [110, 24, 24]
        };
        let fg = if cost < 0 {
            [152, 240, 168]
        } else if affordable {
            [248, 240, 180]
        } else {
            [255, 160, 160]
        };
        frame.fill(sims_ui::frame::Rect::new(bx, by, box_w, box_h), bg);
        font.draw(&mut frame, &label, [bx + 5, by + 3], fg);
        self.canvas.color = frame.into_packed();
    }

    /// Refreshes the cached render lot shape and people from `self.lot.world()`.
    fn refresh_world(&mut self) {
        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        let world = self.lot.world();
        if !self.open_pack_mode {
            self.people.refresh(world);
        }
        match sims_render::Lot::from_world(world) {
            Ok(shape) => self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation()),
            Err(e) => self
                .people
                .note(vec![format!("the lot is not redrawn: {e}")]),
        }
    }

    /// Whether the travel / community-lot [`ShopPickerDialog`] is open.
    pub fn shop_open(&self) -> bool {
        self.shop_dialog.is_open()
    }

    /// Active [`ShopPickerDialog`] controller.
    pub fn shop_dialog(&self) -> &ShopPickerDialog {
        &self.shop_dialog
    }

    /// Active [`ShopSnapshot`], when the shop picker is open.
    pub fn shop_snapshot(&self) -> Option<&ShopSnapshot> {
        self.shop_snapshot.as_ref()
    }

    /// Default [`ShopKind`] matching the active lot's expansion / community zoning.
    ///
    /// Returns `None` on residential lots (including Magic Town residential lots
    /// 90–92) and during unlimited-budget community edit mode (`edit_mode` /
    /// `globals[32] != 0`) or when no live household (`globals[9]`) is active.
    pub fn shop_kind_for_lot(&self) -> Option<ShopKind> {
        let world = self.lot.world();
        let has_household = world.globals.get(9).copied().unwrap_or(0) != 0;
        let edit_budget = world.globals.get(32).copied().unwrap_or(0) != 0;
        if self.edit_mode || edit_budget || !has_household || self.lot.residential() {
            return None;
        }
        let lot_id = self.lot.lot_id();
        match lot_id {
            21..=30 => Some(ShopKind::DowntownBoutique(WardrobeCategory::Formal)),
            40..=48 => Some(ShopKind::VacationKiosk),
            81..=89 => Some(ShopKind::StudioWardrobe),
            90..=98 => Some(ShopKind::MagicVendor),
            _ => Some(ShopKind::PetAdoption),
        }
    }

    /// Opens the community-lot [`ShopPickerDialog`] for `kind` with curated
    /// destination items and the active household's live Simoleons & MagiCoins wallet.
    pub fn open_shop(&mut self, kind: ShopKind) -> bool {
        let items = build_default_shop_items(kind, &self.purchased_shop_items);
        self.open_shop_with_items(kind, items)
    }

    /// Opens [`ShopPickerDialog`] with explicit `items` and the live household wallet.
    pub fn open_shop_with_items(&mut self, kind: ShopKind, mut items: Vec<ShopItem>) -> bool {
        let world = self.lot.world();
        let has_household = world.globals.get(9).copied().unwrap_or(0) != 0;
        let edit_budget = world.globals.get(32).copied().unwrap_or(0) != 0;
        if self.edit_mode || edit_budget || !has_household {
            self.note = Some("Shop is only available during a live household visit.".into());
            return false;
        }
        self.ensure_shop_assets();
        for item in &mut items {
            if self.purchased_shop_items.contains(&item.id) {
                item.owned = true;
            }
        }
        let wallet = self.current_shop_wallet();
        let shopper_name = self.lot.world().selected_whereabouts().and_then(|_| {
            let sel = self.lot.world().globals.get(1).copied().unwrap_or(0) as u16;
            self.lot
                .world()
                .objects
                .get(&sel)
                .and_then(|o| self.hud.name(o.guid))
                .map(str::to_owned)
        });
        let preview_targets = self.build_boutique_preview_targets(kind, &items);
        self.shop_snapshot = Some(ShopSnapshot {
            kind,
            title_override: None,
            buy_label_override: None,
            shopper_name,
            wallet,
            items,
            tried_on_item_id: None,
            preview_image: None,
            preview_label: None,
            hide_wallet_and_prices: false,
        });
        self.shop_preview_yaw_deg = 0;
        self.shop_preview_body = None;
        self.shop_preview_targets = preview_targets;
        self.shop_dialog.open();
        self.close_menu();
        true
    }

    /// Closes the open [`ShopPickerDialog`].
    pub fn close_shop(&mut self) {
        self.shop_dialog.close();
        self.shop_snapshot = None;
        self.shop_preview_yaw_deg = 0;
        self.shop_preview_body = None;
        self.shop_preview_targets.clear();
    }

    /// Current horizontal rotation angle (in degrees) of the live 3D Shop Try-On preview.
    pub fn shop_preview_yaw_deg(&self) -> i16 {
        self.shop_preview_yaw_deg
    }

    /// Triggers Try-On for the currently selected item in [`ShopPickerDialog`].
    pub fn shop_try_on_selected(&mut self) -> Outcome {
        let Some(item_id) = self.shop_snapshot.as_ref().and_then(|snap| {
            self.shop_dialog
                .can_try_on_selected(snap)
                .then(|| self.shop_dialog.selected_item(snap).map(|i| i.id.clone()))
                .flatten()
        }) else {
            return Outcome::default();
        };
        self.execute_shop_command(ShopCommand::TryOn { item_id })
    }

    /// Rotates the live 3D Shop Try-On preview by `delta_deg` degrees.
    pub fn shop_rotate_preview(&mut self, delta_deg: i16) -> Outcome {
        if !self.shop_dialog.is_open()
            || self
                .shop_snapshot
                .as_ref()
                .is_none_or(|s| s.preview_image.is_none())
        {
            return Outcome::default();
        }
        self.execute_shop_command(ShopCommand::RotatePreview(delta_deg))
    }

    pub fn shop_select_prev(&mut self) -> bool {
        self.shop_snapshot
            .as_ref()
            .is_some_and(|snap| self.shop_dialog.select_prev(snap))
    }

    pub fn shop_select_next(&mut self) -> bool {
        self.shop_snapshot
            .as_ref()
            .is_some_and(|snap| self.shop_dialog.select_next(snap))
    }

    pub fn shop_wheel(&mut self, delta_y: f64) -> bool {
        let delta = if delta_y > 0.0 {
            -1
        } else if delta_y < 0.0 {
            1
        } else {
            return false;
        };
        let size = [
            self.view.size[0].max(1) as u32,
            self.view.size[1].max(1) as u32,
        ];
        self.shop_snapshot
            .as_ref()
            .is_some_and(|snap| self.shop_dialog.scroll(delta, size, snap))
    }

    pub fn shop_submit(&mut self) -> Outcome {
        let cmd = self
            .shop_snapshot
            .as_ref()
            .and_then(|snap| self.shop_dialog.submit(snap));
        match cmd {
            Some(cmd) => self.execute_shop_command(cmd),
            None => Outcome::default(),
        }
    }

    pub fn shop_escape(&mut self) -> Outcome {
        match self.shop_dialog.escape() {
            Some(cmd) => self.execute_shop_command(cmd),
            None => Outcome::default(),
        }
    }

    /// Executes a [`ShopCommand`] emitted by [`ShopPickerDialog`], either
    /// answering an active VM special panel (`Dialog` kinds `5..=15`) or
    /// deducting Simoleons / MagiCoins atomically for an interactive community shop.
    pub fn execute_shop_command(&mut self, cmd: ShopCommand) -> Outcome {
        if let Some(active) = self.active_special_panel.clone() {
            return self.execute_special_panel_shop_command(active, cmd);
        }
        match cmd {
            ShopCommand::Cancel => {
                self.close_shop();
                self.hud.play(crate::hud::CLICK);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            ShopCommand::TryOn { item_id } => {
                let target = self.shop_preview_targets.get(&item_id).cloned();
                self.apply_shop_try_on(&item_id, target.as_ref());
                self.hud.play(crate::hud::CLICK);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            ShopCommand::RotatePreview(delta_deg) => {
                self.apply_shop_rotate_preview(delta_deg);
                self.hud.play(crate::hud::CLICK);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            ShopCommand::Buy {
                item_id,
                currency,
                price,
            } => {
                if price < 0 {
                    self.hud.play(crate::hud::ERROR);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    return Outcome::redraw();
                }
                let wallet = self.current_shop_wallet();
                if !wallet.can_afford(price, currency) {
                    self.note = Some(format!(
                        "Insufficient funds for purchase ({}).",
                        currency.format_price(price)
                    ));
                    self.hud.play(crate::hud::ERROR);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    return Outcome::redraw();
                }
                let deduct_res = self.deduct_shop_currency(currency, price);
                match deduct_res {
                    Ok(()) => {
                        self.purchased_shop_items.insert(item_id.clone());
                        let next_wallet = self.current_shop_wallet();
                        let mut bought_name = item_id.clone();
                        if let Some(snap) = &mut self.shop_snapshot {
                            snap.wallet = next_wallet;
                            if let Some(item) = snap.items.iter_mut().find(|i| i.id == item_id) {
                                item.owned = true;
                                bought_name = item.name.clone();
                            }
                        }
                        self.note = Some(format!(
                            "Purchased {bought_name} for {}.",
                            currency.format_price(price)
                        ));
                        self.hud.play(crate::hud::MONEYBACK);
                    }
                    Err(e) => {
                        self.note = Some(format!("Purchase failed: {e}"));
                        self.hud.play(crate::hud::ERROR);
                    }
                }
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
        }
    }

    fn current_shop_wallet(&self) -> Wallet {
        let world = self.lot.world();
        let simoleons = sims_vm::available_funds(world).max(0);
        let fam_id = i32::from(world.globals.get(9).copied().unwrap_or(0));
        let members: Vec<u32> = world
            .families
            .iter()
            .find(|f| f.id == fam_id)
            .map(|f| f.members.clone())
            .unwrap_or_default();
        let magic_coins = sims_vm::travel::family_magic_coins(world, &members).max(0);
        Wallet {
            simoleons,
            magic_coins,
        }
    }

    fn deduct_shop_currency(&mut self, currency: CurrencyKind, price: i32) -> Result<(), String> {
        if price == 0 {
            return Ok(());
        }
        self.mutate_world_via_saved(|world| {
            let fam_id = i32::from(world.globals.get(9).copied().unwrap_or(0));
            match currency {
                CurrencyKind::Simoleons => {
                    let next_funds = world.budget.funds.saturating_sub(price).max(0);
                    let _ = world.set_funds(next_funds);
                }
                CurrencyKind::MagiCoins => {
                    let members: Vec<u32> = world
                        .families
                        .iter()
                        .find(|f| f.id == fam_id)
                        .map(|f| f.members.clone())
                        .unwrap_or_default();
                    let mut remaining = price.max(0);
                    for guid in members {
                        if remaining == 0 {
                            break;
                        }
                        let Some(nid) = world
                            .neighbors
                            .slots
                            .iter()
                            .flatten()
                            .find(|r| r.guid == guid)
                            .map(|r| r.id)
                        else {
                            continue;
                        };
                        let have = i32::from(sims_vm::travel::neighbor_token_count(
                            world,
                            nid,
                            sims_vm::travel::MAGICOIN_TOKEN_KIND,
                            sims_vm::travel::MAGICOIN_TOKEN_GUID,
                        ));
                        if have > 0 {
                            let take = have.min(remaining);
                            let next = u16::try_from((have - take).max(0)).unwrap_or(0);
                            sims_vm::travel::set_neighbor_token_count(
                                world,
                                nid,
                                sims_vm::travel::MAGICOIN_TOKEN_KIND,
                                sims_vm::travel::MAGICOIN_TOKEN_GUID,
                                next,
                            );
                            remaining -= take;
                        }
                    }
                }
            }
        })
    }

    fn ensure_shop_assets(&mut self) {
        if self.shop_strings.is_none() {
            let strings = ShopStrings::load(self.hud.install(), US_ENGLISH).unwrap_or_default();
            self.shop_strings = Some(strings);
        }
        if self.shop_art.is_none()
            && let Ok(ui) = UiGraphics::open(self.hud.install())
            && let Ok(art) = sims_ui::shop::load_art(&ui, US_ENGLISH)
        {
            self.shop_art = Some(art);
        }
    }

    fn draw_shop_dialog(&mut self) {
        if !self.shop_dialog.is_open() {
            return;
        }
        self.ensure_shop_assets();
        let Some(snapshot) = self.shop_snapshot.as_ref() else {
            return;
        };
        let strings = self.shop_strings.clone().unwrap_or_default();
        let art = self.shop_art.as_ref().unwrap_or(&self.hud.art);
        let (w, h) = (
            u32::try_from(self.canvas.width).unwrap_or(0),
            u32::try_from(self.canvas.height).unwrap_or(0),
        );
        let Ok(mut frame) =
            sims_ui::frame::Frame::from_packed(w, h, std::mem::take(&mut self.canvas.color))
        else {
            return;
        };
        self.shop_dialog.draw(&mut frame, snapshot, &strings, art);
        self.canvas.color = frame.into_packed();
    }

    fn draw_destination_picker(&mut self) {
        if self.destination_picker.is_none() {
            return;
        }
        if self.destination_picker_assets.is_none() {
            self.destination_picker_assets =
                Some(DestinationPickerAssets::load(self.hud.install()));
        }
        let Some(assets) = self.destination_picker_assets.as_ref() else {
            return;
        };
        if let Some(picker) = self.destination_picker.as_mut() {
            picker.draw(&mut self.canvas, assets, self.clock_us);
        }
    }

    // -------------------------------------------------------------------------
    // Paginated Spellbook & Cookbook Reference Modal (`sims_ui::book`)
    // -------------------------------------------------------------------------

    /// Whether the paginated Spellbook / Cookbook [`ReferenceBookDialog`] is open.
    pub fn book_open(&self) -> bool {
        self.book_dialog.is_open()
    }

    /// Active [`ReferenceBookDialog`] controller.
    pub fn book_dialog(&self) -> &ReferenceBookDialog {
        &self.book_dialog
    }

    /// Active [`ReferenceBookSnapshot`], when the reference book modal is open.
    pub fn book_snapshot(&self) -> Option<&ReferenceBookSnapshot> {
        self.book_snapshot.as_ref()
    }

    /// Navigates to the previous page of the open reference book.
    pub fn book_prev_page(&mut self) -> bool {
        let changed = self
            .book_snapshot
            .as_ref()
            .and_then(|snap| self.book_dialog.prev_page(snap))
            .is_some();
        if changed {
            self.hud.play(crate::hud::CLICK);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
        }
        changed
    }

    /// Navigates to the next page of the open reference book.
    pub fn book_next_page(&mut self) -> bool {
        let changed = self
            .book_snapshot
            .as_ref()
            .and_then(|snap| self.book_dialog.next_page(snap))
            .is_some();
        if changed {
            self.hud.play(crate::hud::CLICK);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
        }
        changed
    }

    /// Scrolls vertically within the active page of the open reference book
    /// (clamped to the measured wrapped line overflow for the current viewport).
    pub fn book_scroll_lines(&mut self, delta_lines: i32) -> bool {
        if !self.book_dialog.is_open() {
            return false;
        }
        self.ensure_book_assets();
        let size = [
            self.view.size[0].max(1) as u32,
            self.view.size[1].max(1) as u32,
        ];
        let art = self.book_art.as_ref().unwrap_or(&self.hud.art);
        self.book_snapshot.as_ref().is_some_and(|snap| {
            self.book_dialog
                .scroll_vertical(delta_lines, size, snap, art)
        })
    }

    /// Scrolls within a tall reference book page via mouse wheel, turning pages
    /// once at the top/bottom of the current page.
    pub fn book_wheel(&mut self, delta_y: f64) -> bool {
        let dir = if delta_y > 0.0 {
            -1
        } else if delta_y < 0.0 {
            1
        } else {
            return false;
        };
        if self.book_scroll_lines(dir * 3) {
            return true;
        }
        let changed = self
            .book_snapshot
            .as_ref()
            .and_then(|snap| self.book_dialog.scroll(dir, snap))
            .is_some();
        if changed {
            self.hud.play(crate::hud::CLICK);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
        }
        changed
    }

    /// Opens the paginated Spellbook (`is_cookbook == false`) or Cookbook
    /// (`is_cookbook == true`) reference modal directly for `caller`.
    pub fn open_reference_book(&mut self, caller: Option<u16>, is_cookbook: bool) {
        self.ensure_book_assets();
        let snapshot = self.build_reference_book_snapshot(self.lot.world(), caller, is_cookbook);
        self.book_snapshot = Some(snapshot);
        self.book_dialog.open();
        self.close_menu();
        self.hud.close_camera_strip();
    }

    /// Closes the open reference book modal and delivers
    /// [`SpecialPanelResponse::Dismissed`] to the VM when backing a special panel.
    pub fn book_close(&mut self) -> Outcome {
        let is_book_panel = self.active_special_panel.as_ref().is_some_and(|p| {
            matches!(
                p.request,
                SpecialPanelRequest::SpellBook { .. } | SpecialPanelRequest::CookBook { .. }
            )
        });
        if !self.book_dialog.is_open() && !is_book_panel {
            return Outcome::default();
        }
        self.book_dialog.close();
        if is_book_panel {
            if self.answer_special_panel(SpecialPanelResponse::Dismissed) {
                self.hud.play(crate::hud::CLICK);
            } else {
                self.hud.play(crate::hud::ERROR);
                if self.book_snapshot.is_some() {
                    self.book_dialog.open();
                } else {
                    self.active_special_panel = None;
                    self.sync_special_panel();
                }
            }
        } else {
            self.book_snapshot = None;
            self.hud.play(crate::hud::CLICK);
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    fn ensure_book_assets(&mut self) {
        if self.book_strings.is_none() {
            let strings = BookStrings::load(self.hud.install(), US_ENGLISH).unwrap_or_default();
            self.book_strings = Some(strings);
        }
        if self.book_art.is_none()
            && let Ok(ui) = UiGraphics::open(self.hud.install())
            && let Ok(art) = sims_ui::book::load_art(&ui, US_ENGLISH)
        {
            self.book_art = Some(art);
        }
    }

    fn draw_book_dialog(&mut self) {
        if !self.book_dialog.is_open() {
            return;
        }
        self.ensure_book_assets();
        let Some(snapshot) = self.book_snapshot.as_ref() else {
            return;
        };
        let strings = self.book_strings.clone().unwrap_or_default();
        let art = self.book_art.as_ref().unwrap_or(&self.hud.art);
        let (w, h) = (
            u32::try_from(self.canvas.width).unwrap_or(0),
            u32::try_from(self.canvas.height).unwrap_or(0),
        );
        let Ok(mut frame) =
            sims_ui::frame::Frame::from_packed(w, h, std::mem::take(&mut self.canvas.color))
        else {
            return;
        };
        self.book_dialog.draw(&mut frame, snapshot, &strings, art);
        self.canvas.color = frame.into_packed();
    }

    // -------------------------------------------------------------------------
    // Live 3D Sim Preview for Wardrobe / Charm Try-On (`sims_people::caf_preview`)
    // -------------------------------------------------------------------------

    fn ensure_shop_preview_assets(&mut self) -> Option<&sims_people::Assets> {
        if self.shop_preview_assets.is_none() {
            self.shop_preview_assets = sims_people::Assets::scan(&self.hud.install().root).ok();
        }
        self.shop_preview_assets.as_ref()
    }

    fn build_boutique_preview_targets(
        &mut self,
        kind: ShopKind,
        items: &[ShopItem],
    ) -> BTreeMap<String, ShopPreviewTarget> {
        let mut targets = BTreeMap::new();
        let category_prefix: Option<&str> = match kind {
            ShopKind::DowntownBoutique(WardrobeCategory::Everyday) => Some("b"),
            ShopKind::DowntownBoutique(WardrobeCategory::Swimwear) => Some("s"),
            ShopKind::DowntownBoutique(WardrobeCategory::Formal) => Some("f"),
            ShopKind::DowntownBoutique(WardrobeCategory::Pajamas) => Some("l"),
            ShopKind::DowntownBoutique(WardrobeCategory::Winterwear) => Some("w"),
            ShopKind::StudioWardrobe => Some("h"),
            _ => None,
        };
        let Some(prefix) = category_prefix else {
            return targets;
        };
        let try_on_items: Vec<&ShopItem> = items.iter().filter(|i| i.supports_try_on).collect();
        if try_on_items.is_empty() {
            return targets;
        }
        let world = self.lot.world();
        let Some(person) = world.selected().or_else(|| {
            world
                .objects
                .iter()
                .find(|(_, o)| o.person.is_some())
                .map(|(&id, _)| id)
        }) else {
            return targets;
        };
        let (female, child) = world
            .objects
            .get(&person)
            .and_then(|o| o.person.as_ref())
            .map(|p| {
                let kind_word = p.data.get(sims_sim::neighbors::KIND).copied().unwrap_or(0);
                let age = p.data.get(58).copied().unwrap_or(27);
                ((kind_word & 1) == 1, age < 18)
            })
            .unwrap_or((false, false));
        let skin = self.person_skin_tone(world, person);

        if prefix == "b" {
            let mut everyday_rows: Vec<(String, String)> = Vec::new();
            if let Some(wardrobe) = self.ensure_ordered_wardrobe() {
                let count = wardrobe.count(false, female, child, &skin).min(32);
                for idx in 0..count {
                    if let Some((suit, row)) = wardrobe.pick(false, female, child, &skin, idx) {
                        everyday_rows.push((suit, row));
                    }
                }
            }
            for (idx, item) in try_on_items.iter().enumerate() {
                if let Some((suit, row)) = everyday_rows.get(idx) {
                    targets.insert(
                        item.id.clone(),
                        ShopPreviewTarget::human(person, suit.clone(), Some(row.clone())),
                    );
                }
            }
            return targets;
        }

        let gender_age = match (female, child) {
            (true, false) => "fa",
            (false, false) => "ma",
            (true, true) => "fc",
            (false, true) => "mc",
        };
        let mut category_suits: Vec<String> = Vec::new();
        if let Some(assets) = self.ensure_shop_preview_assets() {
            let mut matched: Vec<String> = sims_people::appearance::appearance_names(assets)
                .filter(|name| {
                    let lower = name.to_ascii_lowercase();
                    lower.starts_with(prefix)
                        && lower.len() >= 6
                        && lower.as_bytes()[1..4].iter().all(u8::is_ascii_digit)
                        && lower.as_bytes().get(4..6) == Some(gender_age.as_bytes())
                })
                .map(str::to_owned)
                .collect();
            matched.sort();
            matched.dedup();
            category_suits = matched;
        }
        for (idx, item) in try_on_items.iter().enumerate() {
            if let Some(suit) = category_suits.get(idx) {
                targets.insert(
                    item.id.clone(),
                    ShopPreviewTarget::human(person, suit.clone(), None),
                );
            }
        }
        targets
    }

    fn apply_shop_try_on(&mut self, item_id: &str, target: Option<&ShopPreviewTarget>) {
        let mut tried_name = item_id.to_owned();
        let is_new_item = self
            .shop_snapshot
            .as_ref()
            .and_then(|s| s.tried_on_item_id.as_deref())
            != Some(item_id);
        if is_new_item {
            self.shop_preview_yaw_deg = 0;
            self.shop_preview_body = None;
        }
        if let Some(snap) = &self.shop_snapshot
            && let Some(item) = snap.items.iter().find(|i| i.id == item_id)
        {
            tried_name = item.name.clone();
        }
        let image = self.render_shop_try_on_image(item_id, target);
        if let Some(snap) = &mut self.shop_snapshot {
            snap.tried_on_item_id = Some(item_id.to_owned());
            if image.is_some() {
                snap.preview_image = image;
                snap.preview_label = Some(tried_name.clone());
            } else {
                snap.preview_image = None;
                snap.preview_label = Some("Preview unavailable".into());
            }
        }
        self.note = Some(format!("Previewing {tried_name}."));
    }

    fn apply_shop_rotate_preview(&mut self, delta_deg: i16) {
        self.shop_preview_yaw_deg =
            (i32::from(self.shop_preview_yaw_deg) + i32::from(delta_deg)).rem_euclid(360) as i16;
        let yaw = sims_people::caf_preview::TURNTABLE_BASE_YAW
            + f32::from(self.shop_preview_yaw_deg).to_radians();
        let params = sims_people::caf_preview::TurntableParams {
            yaw,
            ..sims_people::caf_preview::TurntableParams::default()
        };
        let rendered = self
            .shop_preview_body
            .as_ref()
            .and_then(|(_, body)| {
                sims_people::caf_preview::render_turntable(body, None, &params).ok()
            })
            .map(|canvas| sims_formats::bitmap::Image {
                width: canvas.width,
                height: canvas.height,
                rgba: canvas.rgba,
            });
        if let (Some(img), Some(snap)) = (rendered, &mut self.shop_snapshot) {
            snap.preview_image = Some(img);
        }
    }

    fn person_head_binding(
        &self,
        world: &sims_sim::World,
        person: u16,
    ) -> Option<(String, String)> {
        let obj = world.objects.get(&person)?;
        if let Some(outfits) = world.made_outfits.get(&obj.guid)
            && let Some(raw) = outfits.get(&2)
        {
            let (app, tex) = sims_people::appearance::body_string(raw);
            if !app.is_empty() {
                return Some((app, tex));
            }
        }
        let catalog = self.assets.catalog();
        if let Some(entry) = catalog
            .by_guid
            .get(&obj.guid)
            .and_then(|&idx| catalog.objects.get(idx))
            && let Some(res) = self.assets.index().resources.get(entry.resource)
            && let Ok(bytes) = res.read()
            && let Ok(iff) = sims_formats::iff::Iff::parse(bytes)
            && let Some(chunk) = iff.get(b"STR#", 200)
            && let Ok(table) = sims_formats::strings::StringTable::parse(iff.data(chunk))
            && let Some(raw) = table.get(2, US_ENGLISH).or_else(|| table.get(2, 1))
        {
            let (app, tex) = sims_people::appearance::body_string(raw);
            if !app.is_empty() {
                return Some((app, tex));
            }
        }
        None
    }

    fn render_shop_try_on_image(
        &mut self,
        item_id: &str,
        target: Option<&ShopPreviewTarget>,
    ) -> Option<sims_formats::bitmap::Image> {
        let target = target?;
        if let Some(skeleton) = target.pet_skeleton.clone() {
            return self.render_pet_try_on_image(item_id, target, &skeleton);
        }
        let world = self
            .lot
            .asking_special_panel()
            .map(|a| &a.world)
            .unwrap_or_else(|| self.lot.world());
        let person = target.person;
        let (female, child) = world
            .objects
            .get(&person)
            .and_then(|o| o.person.as_ref())
            .map(|p| {
                let kind = p.data.get(sims_sim::neighbors::KIND).copied().unwrap_or(0);
                let age = p.data.get(58).copied().unwrap_or(27);
                ((kind & 1) == 1, age < 18)
            })
            .unwrap_or((false, false));
        let skin_code = self.person_skin_tone(world, person);
        let head_override = self.person_head_binding(world, person);
        let p_age = if child {
            sims_people::caf_preview::Age::Child
        } else {
            sims_people::caf_preview::Age::Adult
        };
        let p_gender = if female {
            sims_people::caf_preview::Gender::Female
        } else {
            sims_people::caf_preview::Gender::Male
        };
        let p_skin = match skin_code.as_str() {
            "med" => sims_people::caf_preview::SkinTone::Medium,
            "drk" => sims_people::caf_preview::SkinTone::Dark,
            _ => sims_people::caf_preview::SkinTone::Light,
        };

        if self
            .shop_preview_body
            .as_ref()
            .is_none_or(|(id, _)| id != item_id)
        {
            let assets = self.ensure_shop_preview_assets()?;
            let wardrobe = sims_people::suit::Wardrobe::of(assets);
            let catalog = sims_people::caf_preview::enumerate_catalog_from_wardrobe(
                &wardrobe, p_age, p_gender, p_skin,
            );
            let head = head_override
                .filter(|(app, _)| sims_people::appearance::load_appearance(assets, app).is_ok())
                .or_else(|| {
                    catalog
                        .heads
                        .first()
                        .map(|h| (h.appearance.clone(), h.texture_binding.clone()))
                })?;

            let mut candidate_bodies: Vec<(String, String)> = Vec::new();
            if let Some(row) = &target.row {
                let (app, tex) = sims_people::appearance::body_string(row);
                if !app.is_empty() {
                    candidate_bodies.push((app, tex));
                }
            }
            if !target.suit.is_empty() {
                if sims_people::appearance::load_appearance(assets, &target.suit).is_ok() {
                    candidate_bodies.push((target.suit.clone(), String::new()));
                } else {
                    // `scan_install_skin_names` may return a `.bmp` texture stem
                    // such as `s001fafitlgt_bikini`; match its appearance prefix.
                    let lower_suit = target.suit.to_ascii_lowercase();
                    let prefix_len = lower_suit.len().min(9);
                    let matched_app = sims_people::appearance::appearance_names(assets)
                        .find(|name| {
                            let ln = name.to_ascii_lowercase();
                            ln == lower_suit
                                || (prefix_len >= 6
                                    && ln.as_bytes().get(..prefix_len)
                                        == lower_suit.as_bytes().get(..prefix_len))
                        })
                        .map(str::to_owned);
                    if let Some(app) = matched_app {
                        candidate_bodies.push((app, format!("BODY={}", target.suit)));
                    }
                }
            }

            let mut loaded_body = None;
            for (body_app, body_tex) in candidate_bodies {
                let cfg = sims_people::caf_preview::SimConfig {
                    age: p_age,
                    gender: p_gender,
                    skin_tone: p_skin,
                    head: head.clone(),
                    body: (body_app, body_tex),
                    hands: None,
                };
                if let Ok(body) = cfg.load_body(assets) {
                    loaded_body = Some(body);
                    break;
                }
            }
            let body = loaded_body?;
            self.shop_preview_body = Some((item_id.to_owned(), body));
        }

        let (_, body) = self.shop_preview_body.as_ref()?;
        let yaw = sims_people::caf_preview::TURNTABLE_BASE_YAW
            + f32::from(self.shop_preview_yaw_deg).to_radians();
        let params = sims_people::caf_preview::TurntableParams {
            yaw,
            ..sims_people::caf_preview::TurntableParams::default()
        };
        let canvas = sims_people::caf_preview::render_turntable(body, None, &params).ok()?;
        Some(sims_formats::bitmap::Image {
            width: canvas.width,
            height: canvas.height,
            rgba: canvas.rgba,
        })
    }

    /// Renders a pet adoption candidate on the pet's own skeleton: the
    /// picked body (`BODY=` row) alone, as pets wear no head or hands.
    fn render_pet_try_on_image(
        &mut self,
        item_id: &str,
        target: &ShopPreviewTarget,
        skeleton: &str,
    ) -> Option<sims_formats::bitmap::Image> {
        if self
            .shop_preview_body
            .as_ref()
            .is_none_or(|(id, _)| id != item_id)
        {
            let row = target.row.as_deref()?;
            let (app, tex) = sims_people::appearance::body_string(row);
            let app = if app.is_empty() {
                target.suit.clone()
            } else {
                app
            };
            let assets = self.ensure_shop_preview_assets()?;
            let outfit = sims_people::appearance::Outfit {
                parts: vec![(app, tex)],
            };
            let body = load_pet_body(assets, skeleton, &outfit).ok()?;
            self.shop_preview_body = Some((item_id.to_owned(), body));
        }
        let (_, body) = self.shop_preview_body.as_ref()?;
        let yaw = sims_people::caf_preview::TURNTABLE_BASE_YAW
            + f32::from(self.shop_preview_yaw_deg).to_radians();
        let params = sims_people::caf_preview::TurntableParams {
            yaw,
            ..sims_people::caf_preview::TurntableParams::default()
        };
        let canvas = sims_people::caf_preview::render_turntable(body, None, &params).ok()?;
        Some(sims_formats::bitmap::Image {
            width: canvas.width,
            height: canvas.height,
            rgba: canvas.rgba,
        })
    }

    // -------------------------------------------------------------------------
    // Special Dialog Panels (`Dialog` kinds `5..=15`, single-player & co-op)
    // -------------------------------------------------------------------------

    /// Opens `request`'s panel over `world` without the VM asking, so
    /// headless snapshot tests can draw it. Nothing answers the VM: picks
    /// are refused as unasked, and Escape closes the panel.
    #[doc(hidden)]
    pub fn preview_special_panel(&mut self, request: SpecialPanelRequest, world: &sims_sim::World) {
        self.close_special_panel_ui();
        self.open_special_panel_ui(0, request, world, None, true);
    }

    /// Whether the open special panel is a synthetic preview rather than a
    /// VM request.
    fn previewing_special_panel(&self) -> bool {
        self.active_special_panel
            .as_ref()
            .is_some_and(|p| p.preview)
    }

    /// Closes any local UI modal opened for a VM special panel without sending
    /// an answer to the VM (used after answering, on co-op peer dismissal, or on
    /// restore).
    fn close_special_panel_ui(&mut self) {
        if let Some(panel) = self.active_special_panel.take() {
            if panel.preview {
                // The pointer last moved over the preview, not the lot:
                // listing resumes only on fresh pointer input, so dismissing
                // never runs a check tree for whatever was under it.
                self.pointer = None;
                self.reached = None;
                self.listed = None;
            }
            self.shop_dialog.close();
            self.shop_snapshot = None;
            self.shop_preview_yaw_deg = 0;
            self.shop_preview_body = None;
            self.shop_preview_targets.clear();
            self.book_dialog.close();
            self.book_snapshot = None;
            self.phonebook_dialog.close();
            self.phonebook_snapshot = None;
            self.destination_picker = None;
            self.hud.close_special_book_dialog();
        }
    }

    /// Synchronizes local modal UI with `Lot::asking_special_panel()` so all
    /// eleven special `Dialog` kinds (`5..=15`) — destination lot pickers,
    /// interactive wardrobe/pet/neighbor/charm pickers, and inline reference
    /// books (Spellbook 12 & Cookbook 15) — present clean native modals in both
    /// single-player and co-op sessions.
    pub fn sync_special_panel(&mut self) {
        // Borrow first to avoid cloning the paused VM world on unchanged frames.
        let coop_dialog_id = self
            .lot
            .coop()
            .and_then(|c| c.active_special_panel.as_ref().map(|(id, _)| *id));
        let unchanged = {
            let Some(asking) = self.lot.asking_special_panel() else {
                if self.previewing_special_panel() {
                    return;
                }
                self.submitted_coop_special_panel = None;
                self.close_special_panel_ui();
                return;
            };
            if coop_dialog_id.is_some()
                && self.submitted_coop_special_panel.map(|(id, _)| id) == coop_dialog_id
            {
                return;
            }
            self.active_special_panel
                .as_ref()
                .is_some_and(|a| a.shows(&asking.shown, coop_dialog_id))
        };
        if unchanged {
            return;
        }
        self.submitted_coop_special_panel = None;
        let Some(asking) = self.lot.asking_special_panel().cloned() else {
            return;
        };
        let tick = asking.shown.tick;
        let request = asking.shown.request.clone();
        self.open_special_panel_ui(tick, request, &asking.world, coop_dialog_id, false);
    }

    /// Opens the local modal for `request`, as the VM asked it at `tick`
    /// with `world` as the asking tick left it.
    fn open_special_panel_ui(
        &mut self,
        tick: u32,
        request: SpecialPanelRequest,
        world: &sims_sim::World,
        coop_dialog_id: Option<NetDialogId>,
        preview: bool,
    ) {
        self.close_menu();
        self.hud.close_camera_strip();
        self.hud.close_special_book_dialog();
        self.book_dialog.close();
        self.book_snapshot = None;
        self.phonebook_dialog.close();
        self.phonebook_snapshot = None;
        self.destination_picker = None;
        self.shop_dialog.close();
        self.shop_snapshot = None;
        self.shop_preview_yaw_deg = 0;
        self.shop_preview_body = None;

        match &request {
            SpecialPanelRequest::SpellBook { caller }
            | SpecialPanelRequest::CookBook { caller } => {
                let is_cookbook = matches!(request, SpecialPanelRequest::CookBook { .. });
                self.ensure_book_assets();
                let snapshot = self.build_reference_book_snapshot(world, *caller, is_cookbook);
                self.book_snapshot = Some(snapshot);
                self.book_dialog.open();
                self.active_special_panel = Some(ActiveSpecialPanel {
                    tick,
                    request,
                    coop_dialog_id,
                    responses: BTreeMap::new(),
                    preview_targets: BTreeMap::new(),
                    preview,
                });
            }
            SpecialPanelRequest::DowntownLotPicker
            | SpecialPanelRequest::VacationLotPicker { .. }
            | SpecialPanelRequest::OldTownLotPicker { .. }
            | SpecialPanelRequest::StudioTownLotPicker { .. }
            | SpecialPanelRequest::MagicTownLotPicker { .. } => {
                let entries = self.resolve_special_lot_entries();
                let wallet = self.current_shop_wallet();
                let fame_half_stars = world
                    .selected()
                    .and_then(|sel| world.objects.get(&sel))
                    .and_then(|o| o.person.as_ref())
                    .and_then(|p| p.data.get(81).copied())
                    .map(|level| level.clamp(0, 10) as u8)
                    .unwrap_or(0);
                if let Ok(picker) = DestinationMapPicker::open(
                    self.hud.install(),
                    self.neighborhood_slot,
                    request.clone(),
                    entries,
                    wallet.simoleons,
                    wallet.magic_coins,
                    fame_half_stars,
                ) {
                    if let Some(err) = picker.map_error() {
                        self.note = Some(format!(
                            "Could not load destination map for special panel kind {}: {err}",
                            request.kind()
                        ));
                    }
                    self.destination_picker = Some(picker);
                    self.active_special_panel = Some(ActiveSpecialPanel {
                        tick,
                        request,
                        coop_dialog_id,
                        responses: BTreeMap::new(),
                        preview_targets: BTreeMap::new(),
                        preview,
                    });
                }
            }
            SpecialPanelRequest::WardrobeShop { person, mode } => {
                let category = match *mode {
                    0 => WardrobeCategory::Everyday,
                    1 => WardrobeCategory::Swimwear,
                    2 => WardrobeCategory::Formal,
                    3 => WardrobeCategory::Pajamas,
                    4 => WardrobeCategory::Winterwear,
                    _ => WardrobeCategory::Formal,
                };
                let shop_kind = if *mode == 5 {
                    ShopKind::StudioWardrobe
                } else {
                    ShopKind::DowntownBoutique(category)
                };
                let shopper = self.person_display_name(world, *person);
                let (items, responses, preview_targets) =
                    self.build_wardrobe_picker_items(world, *person, *mode);
                self.open_special_picker(
                    tick,
                    request,
                    coop_dialog_id,
                    shop_kind,
                    None,
                    shopper,
                    items,
                    responses,
                    preview_targets,
                    preview,
                );
            }
            SpecialPanelRequest::PetPicker { pet } => {
                let shopper = self.person_display_name(world, *pet);
                let (items, responses, preview_targets) = self.build_pet_picker_items(world, *pet);
                self.open_special_picker(
                    tick,
                    request,
                    coop_dialog_id,
                    ShopKind::PetAdoption,
                    None,
                    shopper,
                    items,
                    responses,
                    preview_targets,
                    preview,
                );
            }
            SpecialPanelRequest::CallNeighbor { caller_neighbor } => {
                // Native `cWinPhoneBook` (`0x5221d0`) on `phonebookbkg.bmp`.
                self.ensure_phonebook_assets();
                let snapshot = self.build_phonebook_snapshot(world, *caller_neighbor);
                self.phonebook_snapshot = Some(snapshot);
                self.phonebook_dialog.open();
                self.active_special_panel = Some(ActiveSpecialPanel {
                    tick,
                    request,
                    coop_dialog_id,
                    responses: BTreeMap::new(),
                    preview_targets: BTreeMap::new(),
                    preview,
                });
            }
            SpecialPanelRequest::CharmTransform { person } => {
                let shopper = self.person_display_name(world, *person);
                let (items, responses, preview_targets) =
                    self.build_charm_transform_items(world, *person);
                self.open_special_picker(
                    tick,
                    request,
                    coop_dialog_id,
                    ShopKind::MagicVendor,
                    None,
                    shopper,
                    items,
                    responses,
                    preview_targets,
                    preview,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn open_special_picker(
        &mut self,
        tick: u32,
        request: SpecialPanelRequest,
        coop_dialog_id: Option<NetDialogId>,
        kind: ShopKind,
        title_override: Option<String>,
        shopper_name: Option<String>,
        items: Vec<ShopItem>,
        responses: BTreeMap<String, SpecialPanelResponse>,
        preview_targets: BTreeMap<String, ShopPreviewTarget>,
        preview: bool,
    ) {
        self.ensure_shop_assets();
        let wallet = self.current_shop_wallet();
        self.shop_snapshot = Some(ShopSnapshot {
            kind,
            title_override,
            buy_label_override: None,
            shopper_name,
            wallet,
            items,
            tried_on_item_id: None,
            preview_image: None,
            preview_label: None,
            // Special VM pickers carry no price: the calling object's own
            // tree charges (if anything), so never show a bogus `$0` wallet.
            hide_wallet_and_prices: true,
        });
        self.shop_preview_yaw_deg = 0;
        self.shop_preview_body = None;
        self.shop_dialog.open();
        self.active_special_panel = Some(ActiveSpecialPanel {
            tick,
            request,
            coop_dialog_id,
            responses,
            preview_targets,
            preview,
        });
    }

    /// Delivers `response` to the currently waiting VM special panel (`Dialog`
    /// kinds `5..=15`) in either single-player (`Lot::answer_special_panel` +
    /// tick replay) or co-op lockstep (`sims_vm::coop::special_panel_answer_command`).
    pub fn answer_special_panel(&mut self, response: SpecialPanelResponse) -> bool {
        if self.previewing_special_panel() {
            // A synthetic preview panel never answers a real VM request,
            // even one that happens to be waiting.
            self.close_special_panel_ui();
            return false;
        }
        let Some(asking) = self.lot.asking_special_panel() else {
            self.close_special_panel_ui();
            self.note = Some("No special panel waits for a response.".into());
            return false;
        };
        let request = asking.shown.request.clone();
        if !request.accepts(&response) {
            self.note = Some(format!(
                "Special panel kind {} does not accept {response:?}.",
                request.kind()
            ));
            return false;
        }
        if self.coop.is_active() {
            let dialog_id = self
                .lot
                .coop()
                .and_then(|c| c.active_special_panel.as_ref().map(|(id, _)| *id))
                .or_else(|| {
                    self.active_special_panel
                        .as_ref()
                        .and_then(|a| a.coop_dialog_id)
                })
                .or_else(|| {
                    self.coop
                        .session()
                        .and_then(|s| s.open_dialogs.keys().next().copied())
                });
            let Some(dialog_id) = dialog_id else {
                self.note = Some("No co-op blocking special panel is active.".into());
                return false;
            };
            let cmd = sims_vm::coop::special_panel_answer_command(dialog_id, &response);
            if let Err(e) = self.coop.submit_command(cmd) {
                self.note = Some(format!("Special panel response was not sent: {e}"));
                return false;
            }
            self.submitted_coop_special_panel = Some((dialog_id, 0));
            self.close_special_panel_ui();
            return true;
        }
        if let Err(e) = self.lot.answer_special_panel(response) {
            self.note = Some(format!("Special panel response was not taken: {e}"));
            return false;
        }
        self.close_special_panel_ui();
        // Replay the paused tick immediately with the chosen response, just
        // like `Game::answer` does for ordinary dialogs. Committed destination
        // lot picks are recorded into `pending_travel_lot` inside `run_while`
        // from `Update::special_panels_answered` only when the tick commits.
        self.run(1);
        self.sync_special_panel();
        true
    }

    fn execute_special_panel_shop_command(
        &mut self,
        active: ActiveSpecialPanel,
        cmd: ShopCommand,
    ) -> Outcome {
        match cmd {
            ShopCommand::Cancel => {
                let cancel = special_panel_cancel_response(&active.request);
                if self.answer_special_panel(cancel) {
                    self.hud.play(crate::hud::CLICK);
                } else {
                    self.hud.play(crate::hud::ERROR);
                    // Re-open the picker if co-op submission failed so the
                    // blocking special panel never stays hidden.
                    if self.shop_snapshot.is_some() {
                        self.shop_dialog.open();
                    } else {
                        self.active_special_panel = None;
                        self.sync_special_panel();
                    }
                }
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            ShopCommand::TryOn { item_id } => {
                let target = active.preview_targets.get(&item_id);
                self.apply_shop_try_on(&item_id, target);
                self.hud.play(crate::hud::CLICK);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            ShopCommand::RotatePreview(delta_deg) => {
                self.apply_shop_rotate_preview(delta_deg);
                self.hud.play(crate::hud::CLICK);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            ShopCommand::Buy { item_id, .. } => {
                if let Some(resp) = active.responses.get(&item_id).cloned() {
                    if self.answer_special_panel(resp) {
                        self.hud.play(crate::hud::CLICK);
                    } else {
                        self.hud.play(crate::hud::ERROR);
                        if self.shop_snapshot.is_some() {
                            self.shop_dialog.open();
                        } else {
                            self.active_special_panel = None;
                            self.sync_special_panel();
                        }
                    }
                } else {
                    self.hud.play(crate::hud::ERROR);
                }
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
        }
    }

    fn person_display_name(&self, world: &sims_sim::World, object_id: u16) -> Option<String> {
        let obj = world.objects.get(&object_id)?;
        self.lot
            .character_names()
            .get(&obj.guid)
            .cloned()
            .or_else(|| self.hud.name(obj.guid).map(str::to_owned))
    }

    fn build_reference_book_snapshot(
        &self,
        world: &sims_sim::World,
        caller: Option<u16>,
        is_cookbook: bool,
    ) -> ReferenceBookSnapshot {
        let strings = self.book_strings.clone().unwrap_or_default();
        // `magic.iff STR# 1[0..1]` is `"%s Family Spellbook"` / `"%s Family
        // Cookbook"`: the subject is the caller's family surname (`FAMs` of the
        // family `person.data[61]` names), e.g. `"Goth Family Spellbook"`.
        let surname = caller
            .and_then(|id| world.objects.get(&id))
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.data.get(sims_sim::neighbors::FAMILY).copied())
            .filter(|&fam| fam > 0)
            .and_then(|fam| self.hud.family_surname(i32::from(fam)))
            .map(str::to_owned);
        let title = strings.format_title(is_cookbook, surname.as_deref());

        let neighbor_id = caller
            .and_then(|id| world.objects.get(&id))
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.data.get(31).copied())
            .filter(|&nid| nid > 0);
        let mut held_tokens: Vec<(String, u16)> = Vec::new();
        if let Some(nid) = neighbor_id
            && let Some(inv) = world
                .neighbors
                .inventories
                .iter()
                .find(|i| i.neighbor == nid)
        {
            for item in &inv.items {
                if item.count == 0 {
                    continue;
                }
                // Kind 7 holds magic tokens/spells/ingredients; kind 8/9 hold recipes/charms.
                let relevant = if is_cookbook {
                    matches!(item.kind, 7..=9)
                } else {
                    matches!(item.kind, 7 | 8)
                };
                if !relevant {
                    continue;
                }
                let label = self
                    .catalog_object_name(item.guid)
                    .unwrap_or_else(|| format!("0x{:08X}", item.guid));
                held_tokens.push((label, item.count));
            }
        }

        let held_summary: Vec<BookBadge> = held_tokens
            .iter()
            .map(|(name, count)| BookBadge {
                label: if *count > 1 {
                    format!("{name} (x{count})")
                } else {
                    name.clone()
                },
                owned: true,
            })
            .collect();

        let catalog =
            sims_ui::book::load_magic_book_entries(self.hud.install(), is_cookbook, US_ENGLISH)
                .unwrap_or_default();

        let mut pages: Vec<BookPage> = Vec::new();
        for entry in catalog {
            let haystack = format!(
                "{} {}",
                entry.title.to_ascii_lowercase(),
                entry.description.to_ascii_lowercase()
            );
            let mut matched_badges = Vec::new();
            for (token_name, count) in &held_tokens {
                let needle = token_name.to_ascii_lowercase();
                if needle.len() >= 3 && haystack.contains(&needle) {
                    matched_badges.push(BookBadge {
                        label: if *count > 1 {
                            format!("{token_name} (x{count})")
                        } else {
                            token_name.clone()
                        },
                        owned: true,
                    });
                }
            }
            let highlight = !matched_badges.is_empty();
            pages.push(BookPage {
                heading: None,
                category: Some(entry.category),
                entries: vec![BookEntry {
                    title: entry.title,
                    subtitle: None,
                    body: entry.description,
                    badges: matched_badges,
                    highlight,
                }],
            });
        }

        if pages.is_empty() {
            let body = if held_summary.is_empty() {
                "—".to_owned()
            } else {
                held_summary
                    .iter()
                    .map(|b| b.label.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            pages.push(BookPage {
                heading: None,
                category: None,
                entries: vec![BookEntry {
                    title: title.clone(),
                    subtitle: None,
                    body,
                    badges: Vec::new(),
                    highlight: !held_summary.is_empty(),
                }],
            });
        }

        ReferenceBookSnapshot {
            title,
            subtitle: None,
            held_summary,
            pages,
            use_magic_book_chrome: true,
        }
    }

    fn catalog_object_name(&self, guid: u32) -> Option<String> {
        let catalog = self.assets.catalog();
        let obj = catalog
            .by_guid
            .get(&guid)
            .and_then(|&idx| catalog.objects.get(idx))?;
        let bytes = self
            .assets
            .index()
            .resources
            .get(obj.resource)?
            .read()
            .ok()?;
        let iff = sims_formats::iff::Iff::parse(bytes).ok()?;
        if let Some(chunk) = iff.get(b"OBJD", obj.chunk) {
            if let Ok(def) = sims_formats::behavior::ObjectDefinition::parse(iff.data(chunk)) {
                let ctss_id = def.catalog;
                if ctss_id > 0
                    && let Some(ctss) = iff.get(b"CTSS", ctss_id)
                    && let Ok(table) = sims_formats::strings::StringTable::parse(iff.data(ctss))
                    && let Some(name) = table
                        .get(0, US_ENGLISH)
                        .or_else(|| table.get(0, 1))
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                {
                    return Some(name.to_owned());
                }
            }
            let chunk_label = chunk.label.trim();
            if !chunk_label.is_empty() {
                return Some(chunk_label.to_owned());
            }
        }
        None
    }

    fn resolve_special_lot_entries(&self) -> Vec<SpecialLotEntry> {
        if !self.special_lot_catalog.is_empty() {
            return self.special_lot_catalog.clone();
        }
        let install = self.hud.install();
        let user_dir = install
            .file("UserData")
            .or_else(|_| install.file("TemplateUserData"))
            .unwrap_or_else(|_| install.root.clone());
        let street_names = crate::neighborhood::StreetNames::load(install, &user_dir);
        let descriptions = crate::neighborhood::NeighborhoodDescriptions::load(install, &user_dir);
        let mut zoning: BTreeMap<u16, i16> = BTreeMap::new();
        let mut discovered: BTreeSet<u16> = BTreeSet::new();
        let expansions = u16::from(install.expansions.0);
        if let Ok(nb) = Neighborhood::open(&user_dir) {
            if let Ok(houses) = nb.houses() {
                for (num, _) in houses {
                    if let Ok(id) = u16::try_from(num) {
                        discovered.insert(id);
                    }
                }
            }
            if let Ok(nb_zoning) = nb.zoning(expansions) {
                for (lot, zone) in nb_zoning {
                    if let Ok(id) = u16::try_from(lot) {
                        zoning.insert(id, zone as i16);
                    }
                }
            }
        }
        if let Ok(tpl_dir) = install.file("TemplateUserData")
            && let Ok(nb) = Neighborhood::open(&tpl_dir)
            && let Ok(houses) = nb.houses()
        {
            for (num, _) in houses {
                if let Ok(id) = u16::try_from(num) {
                    discovered.insert(id);
                }
            }
            if let Ok(nb_zoning) = nb.zoning(expansions) {
                for (lot, zone) in nb_zoning {
                    if let Ok(id) = u16::try_from(lot) {
                        zoning.entry(id).or_insert(zone as i16);
                    }
                }
            }
        }
        discovered
            .into_iter()
            .map(|lot_id| {
                let (name, description) = descriptions.lot_info(lot_id);
                let address = street_names.address_for(lot_id);
                let z = zoning.get(&lot_id).copied().unwrap_or(match lot_id {
                    21..=30 | 40..=48 | 81..=89 | 93..=98 => 2,
                    _ => 1,
                });
                SpecialLotEntry {
                    lot_id,
                    name,
                    address,
                    description,
                    zoning: z,
                }
            })
            .collect()
    }

    fn person_skin_tone(&self, world: &sims_sim::World, person: u16) -> String {
        let Some(obj) = world.objects.get(&person) else {
            return "lgt".into();
        };
        let normalize = |raw: &str| -> Option<String> {
            let prefix: String = raw.trim().chars().take(3).collect();
            if prefix.eq_ignore_ascii_case("lgt")
                || prefix.eq_ignore_ascii_case("med")
                || prefix.eq_ignore_ascii_case("drk")
            {
                Some(prefix.to_ascii_lowercase())
            } else {
                None
            }
        };
        if let Some(outfits) = world.made_outfits.get(&obj.guid)
            && let Some(raw) = outfits.get(&14)
            && let Some(tone) = normalize(raw)
        {
            return tone;
        }
        let catalog = self.assets.catalog();
        if let Some(entry) = catalog
            .by_guid
            .get(&obj.guid)
            .and_then(|&idx| catalog.objects.get(idx))
            && let Some(res) = self.assets.index().resources.get(entry.resource)
            && let Ok(bytes) = res.read()
            && let Ok(iff) = sims_formats::iff::Iff::parse(bytes)
            && let Some(chunk) = iff.get(b"STR#", 200)
            && let Ok(table) = sims_formats::strings::StringTable::parse(iff.data(chunk))
            && let Some(raw) = table.get(14, US_ENGLISH).or_else(|| table.get(14, 1))
            && let Some(tone) = normalize(raw)
        {
            return tone;
        }
        "lgt".into()
    }

    fn ensure_ordered_wardrobe(&mut self) -> Option<&sims_content::ImportWardrobe> {
        if self.ordered_wardrobe.is_none()
            && let Ok((wardrobe, _)) =
                sims_content::ImportWardrobe::load_lenient(self.hud.install())
        {
            self.ordered_wardrobe = Some(wardrobe);
        }
        self.ordered_wardrobe.as_ref()
    }

    /// The caller's `(female, child)` from `person.data[65]` bit 0 and age
    /// word 58.
    fn person_sex_and_age(world: &sims_sim::World, person: u16) -> (bool, bool) {
        world
            .objects
            .get(&person)
            .and_then(|o| o.person.as_ref())
            .map(|p| {
                let kind = p.data.get(sims_sim::neighbors::KIND).copied().unwrap_or(0);
                let age = p.data.get(58).copied().unwrap_or(27);
                ((kind & 1) == 1, age < 18)
            })
            .unwrap_or((false, false))
    }

    /// `Dialog` kind 6 (`0x508f22`): every outfit of the mode's category the
    /// caller can wear — matching sex (or unisex), age and skin tone — in
    /// native wardrobe order, uncapped. Item `i` answers `Selection { index: i }`.
    fn build_wardrobe_picker_items(
        &mut self,
        world: &sims_sim::World,
        person: u16,
        mode: i16,
    ) -> (
        Vec<ShopItem>,
        BTreeMap<String, SpecialPanelResponse>,
        BTreeMap<String, ShopPreviewTarget>,
    ) {
        let (female, child) = Self::person_sex_and_age(world, person);
        let skin = self.person_skin_tone(world, person);
        let outfits = self
            .ensure_ordered_wardrobe()
            .map(|w| category_outfits(w, wardrobe_mode_category(mode), female, child, &skin))
            .unwrap_or_default();
        Self::outfit_picker_items("special_wardrobe", person, outfits)
    }

    /// Picker rows for human outfits: clean labels, no price, try-on enabled.
    fn outfit_picker_items(
        id_prefix: &str,
        person: u16,
        outfits: Vec<(String, String)>,
    ) -> (
        Vec<ShopItem>,
        BTreeMap<String, SpecialPanelResponse>,
        BTreeMap<String, ShopPreviewTarget>,
    ) {
        let mut items = Vec::with_capacity(outfits.len());
        let mut responses = BTreeMap::new();
        let mut preview_targets = BTreeMap::new();
        for (idx, (suit, row)) in outfits.into_iter().enumerate() {
            let Ok(index) = i16::try_from(idx) else {
                break;
            };
            let id = format!("{id_prefix}:{idx}");
            items.push(ShopItem {
                id: id.clone(),
                name: outfit_display_label(&suit, &row),
                description: String::new(),
                price: 0,
                currency: CurrencyKind::Simoleons,
                icon: None,
                supports_try_on: true,
                owned: false,
                available: true,
            });
            responses.insert(
                id.clone(),
                SpecialPanelResponse::Selection {
                    accepted: true,
                    index,
                },
            );
            preview_targets.insert(id, ShopPreviewTarget::human(person, suit, Some(row)));
        }
        (items, responses, preview_targets)
    }

    /// `Dialog` kind 9 (`0x521788`): the pet bodies of the pen pet's species
    /// in the exact `ImportWardrobe::pick_pet` order `Make New Character`
    /// reads the answer with (`0x44b064`), uncapped, with breed and sex labels
    /// and a 3D turntable preview on the pet's own skeleton.
    fn build_pet_picker_items(
        &mut self,
        world: &sims_sim::World,
        pet: u16,
    ) -> (
        Vec<ShopItem>,
        BTreeMap<String, SpecialPanelResponse>,
        BTreeMap<String, ShopPreviewTarget>,
    ) {
        let kind = world
            .objects
            .get(&pet)
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.data.get(sims_sim::neighbors::KIND).copied())
            .unwrap_or(0);
        let species = sims_ui::household::PetKind::from_kind(kind);
        let is_cat = species == Some(sims_ui::household::PetKind::Cat);
        let female = kind & 1 == 1;
        let skeleton = world
            .objects
            .get(&pet)
            .and_then(|o| self.person_body_string(o.guid, 0))
            .filter(|s| !s.trim().is_empty());
        let sex_species = match (female, is_cat) {
            (false, false) => "Male Dog",
            (true, false) => "Female Dog",
            (false, true) => "Male Cat",
            (true, true) => "Female Cat",
        };
        let rows: Vec<(String, String)> = self
            .ensure_ordered_wardrobe()
            .map(|w| {
                (0..w.count_pet(is_cat))
                    .map_while(|idx| w.pick_pet(is_cat, idx))
                    .collect()
            })
            .unwrap_or_default();
        let mut items = Vec::with_capacity(rows.len());
        let mut responses = BTreeMap::new();
        let mut preview_targets = BTreeMap::new();
        for (idx, (suit, row)) in rows.into_iter().enumerate() {
            let Ok(index) = i16::try_from(idx) else {
                break;
            };
            let id = format!("special_pet:{idx}");
            items.push(ShopItem {
                id: id.clone(),
                name: pet_breed_label(&row),
                description: sex_species.to_owned(),
                price: 0,
                currency: CurrencyKind::Simoleons,
                icon: None,
                supports_try_on: skeleton.is_some(),
                owned: false,
                available: true,
            });
            responses.insert(
                id.clone(),
                SpecialPanelResponse::Selection {
                    accepted: true,
                    index,
                },
            );
            if let Some(skeleton) = &skeleton {
                preview_targets.insert(
                    id,
                    ShopPreviewTarget {
                        person: pet,
                        suit,
                        row: Some(row),
                        pet_skeleton: Some(skeleton.clone()),
                    },
                );
            }
        }
        (items, responses, preview_targets)
    }

    /// Body string `index` (`STR# 200`) of the person type `guid`, preferring
    /// strings Make New Character wrote.
    fn person_body_string(&self, guid: u32, index: usize) -> Option<String> {
        let catalog = self.assets.catalog();
        let entry = catalog
            .by_guid
            .get(&guid)
            .and_then(|&idx| catalog.objects.get(idx))?;
        let bytes = self
            .assets
            .index()
            .resources
            .get(entry.resource)?
            .read()
            .ok()?;
        let iff = sims_formats::iff::Iff::parse(bytes).ok()?;
        let chunk = iff.get(b"STR#", 200)?;
        let table = sims_formats::strings::StringTable::parse(iff.data(chunk)).ok()?;
        table
            .get(index, US_ENGLISH)
            .or_else(|| table.get(index, 1))
            .map(|s| s.trim().to_owned())
    }

    /// `Dialog` kind 14 (`0x536a6b`): the caller's own Create-a-Sim body list
    /// (everyday `b` bodies matching sex, age and skin tone, native order,
    /// uncapped). No MagiCoin price: the charm, not the picker, pays.
    fn build_charm_transform_items(
        &mut self,
        world: &sims_sim::World,
        person: u16,
    ) -> (
        Vec<ShopItem>,
        BTreeMap<String, SpecialPanelResponse>,
        BTreeMap<String, ShopPreviewTarget>,
    ) {
        let (female, child) = Self::person_sex_and_age(world, person);
        let skin = self.person_skin_tone(world, person);
        let outfits = self
            .ensure_ordered_wardrobe()
            .map(|w| category_outfits(w, b'b', female, child, &skin))
            .unwrap_or_default();
        Self::outfit_picker_items("special_charm", person, outfits)
    }

    /// Builds the call-eligible neighbor list for the native Phonebook
    /// (`cWinPhoneBook`, `Dialog` kind 10). A neighbor is listed when the
    /// caller has a relationship row naming them (`"They can only call people
    /// they've already met"`, `UIText.iff STR# 166[59]`), and they are not the
    /// caller, not in the caller's family (`person.data[61]`), not a pet
    /// (`person.data[65] > 1`, `0x427c69`), not familyless (service NPC), and
    /// not already on this lot. Rows keep the caller's relationship order.
    fn build_phonebook_snapshot(
        &self,
        world: &sims_sim::World,
        caller_neighbor: i16,
    ) -> PhoneBookSnapshot {
        let strings = self.phonebook_strings.clone().unwrap_or_default();
        let records: BTreeMap<i16, &sims_sim::neighbors::Neighbor> = world
            .neighbors
            .slots
            .iter()
            .flatten()
            .map(|r| (r.id, r))
            .collect();
        let caller = records.get(&caller_neighbor).copied();
        let caller_name = caller.and_then(|r| self.neighbor_display_name(r.guid));
        let caller_family = caller
            .and_then(|r| r.data.get(sims_sim::neighbors::FAMILY).copied())
            .unwrap_or(0);
        let on_lot: std::collections::BTreeSet<i16> = world
            .objects
            .values()
            .filter_map(|o| o.person.as_ref())
            .filter_map(|p| p.data.get(PERSON_NEIGHBOR_WORD).copied())
            .filter(|&id| id > 0)
            .collect();
        let entries = caller
            .map(|c| c.relationships.as_slice())
            .unwrap_or_default()
            .iter()
            .filter_map(|row| {
                let id = i16::try_from(row.target).ok()?;
                let record = records.get(&id).copied()?;
                phonebook_eligible(record, caller_neighbor, caller_family, &on_lot).then(|| {
                    let daily = row.values.first().copied().unwrap_or(0);
                    let lifetime = row.values.get(1).copied().unwrap_or(0);
                    let clamp = |v: i32| v.clamp(-100, 100) as i16;
                    PhoneBookEntry {
                        neighbor_id: id,
                        name: self
                            .neighbor_display_name(record.guid)
                            .unwrap_or_else(|| format!("#{id}")),
                        daily: clamp(daily),
                        lifetime: clamp(lifetime),
                        portrait: self.hud.face(record.guid).cloned(),
                        is_friend: daily >= 50 || lifetime >= 50,
                    }
                })
            })
            .collect();
        let empty_message = match caller_name.as_deref() {
            Some(name) => format!("{name} {}", strings.knows_nobody),
            None => strings.knows_nobody.clone(),
        };
        PhoneBookSnapshot {
            title: strings.title.clone(),
            caller_name,
            entries,
            cancel_label: strings.cancel.clone(),
            prev_label: "<".into(),
            next_label: ">".into(),
            empty_message,
        }
    }

    fn neighbor_display_name(&self, guid: u32) -> Option<String> {
        self.lot
            .character_names()
            .get(&guid)
            .cloned()
            .or_else(|| self.hud.name(guid).map(str::to_owned))
    }

    fn ensure_phonebook_assets(&mut self) {
        if self.phonebook_strings.is_none() {
            let strings =
                PhoneBookStrings::load(self.hud.install(), US_ENGLISH).unwrap_or_default();
            self.phonebook_strings = Some(strings);
        }
        if self.phonebook_art.is_none()
            && let Ok(ui) = UiGraphics::open(self.hud.install())
            && let Ok(art) = sims_ui::phonebook::load_art(&ui, US_ENGLISH)
        {
            self.phonebook_art = Some(art);
        }
    }

    fn draw_phonebook_dialog(&mut self) {
        if !self.phonebook_dialog.is_open() {
            return;
        }
        self.ensure_phonebook_assets();
        let Some(snapshot) = self.phonebook_snapshot.as_ref() else {
            return;
        };
        let art = self.phonebook_art.as_ref().unwrap_or(&self.hud.art);
        let (w, h) = (
            u32::try_from(self.canvas.width).unwrap_or(0),
            u32::try_from(self.canvas.height).unwrap_or(0),
        );
        let Ok(mut frame) =
            sims_ui::frame::Frame::from_packed(w, h, std::mem::take(&mut self.canvas.color))
        else {
            return;
        };
        self.phonebook_dialog.draw(&mut frame, snapshot, art);
        self.canvas.color = frame.into_packed();
    }

    /// The open Phonebook's view-model, if any (for tests and the harness).
    pub fn phonebook_snapshot(&self) -> Option<&PhoneBookSnapshot> {
        self.phonebook_dialog
            .is_open()
            .then_some(self.phonebook_snapshot.as_ref())
            .flatten()
    }

    /// Calls the Phonebook's selected neighbor (`Enter`).
    pub fn phonebook_submit(&mut self) -> Outcome {
        let cmd = self
            .phonebook_snapshot
            .as_ref()
            .and_then(|snap| self.phonebook_dialog.submit(snap));
        match cmd {
            Some(cmd) => self.execute_phonebook_command(cmd),
            None => Outcome::default(),
        }
    }

    /// Cancels the Phonebook (`Escape`).
    pub fn phonebook_escape(&mut self) -> Outcome {
        match self.phonebook_dialog.escape() {
            Some(cmd) => self.execute_phonebook_command(cmd),
            None => Outcome::default(),
        }
    }

    /// Answers the waiting `Dialog` kind 10 with the Phonebook's choice:
    /// `Neighbor(Some(id))` for a call, `Neighbor(None)` for Cancel (stored as
    /// `-9999` by `end_of_update`, `0x4413e1`).
    pub fn execute_phonebook_command(&mut self, cmd: PhoneBookCommand) -> Outcome {
        let response = match cmd {
            PhoneBookCommand::Call(id) => SpecialPanelResponse::Neighbor(Some(id)),
            PhoneBookCommand::Cancel => SpecialPanelResponse::Neighbor(None),
        };
        self.phonebook_dialog.close();
        if self.answer_special_panel(response) {
            self.hud.play(crate::hud::CLICK);
        } else {
            self.hud.play(crate::hud::ERROR);
            if self.phonebook_snapshot.is_some() && self.active_special_panel.is_some() {
                self.phonebook_dialog.open();
            } else {
                self.phonebook_snapshot = None;
            }
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    /// Crops the rendered lot canvas to the active Camera Mode `SnapshotSize`
    /// centered on the frame (clamped to canvas bounds) so selecting Small,
    /// Medium, Large, or Custom changes the saved PNG snapshot dimensions.
    pub fn capture_snapshot_canvas(&self) -> Canvas {
        let state = self.hud.camera_state();
        let [req_w, req_h] = state.size.dimensions(state.custom_dimensions);
        let src_w = self.canvas.width;
        let src_h = self.canvas.height;
        if src_w == 0 || src_h == 0 || req_w == 0 || req_h == 0 {
            return self.canvas.clone();
        }
        let crop_w = (req_w as usize).min(src_w);
        let crop_h = (req_h as usize).min(src_h);
        if crop_w == src_w && crop_h == src_h {
            return self.canvas.clone();
        }
        let x0 = (src_w - crop_w) / 2;
        let y0 = (src_h - crop_h) / 2;
        let mut out = Canvas::new(crop_w, crop_h, [0, 0, 0]);
        for y in 0..crop_h {
            let src_row = (y0 + y) * src_w + x0;
            let dst_row = y * crop_w;
            out.color[dst_row..dst_row + crop_w]
                .copy_from_slice(&self.canvas.color[src_row..src_row + crop_w]);
        }
        out
    }

    // -------------------------------------------------------------------------
    // Co-op Multiplayer Live Shell (Wave 3)
    // -------------------------------------------------------------------------

    /// Whether the Host / Join Co-op Lobby modal dialog is currently open.
    pub fn coop_lobby_open(&self) -> bool {
        self.coop_lobby.is_open()
    }

    /// Opens the Host / Join Co-op Lobby dialog on `tab` (defaulting to Host
    /// if offline/hosting or Join if currently a client).
    pub fn open_coop_lobby(&mut self, tab: Option<LobbyTab>) {
        let target_tab = tab.unwrap_or(if self.coop.is_peer() {
            LobbyTab::Join
        } else {
            LobbyTab::Host
        });
        self.coop_lobby.open(target_tab);
    }

    /// Closes the Host / Join Co-op Lobby dialog.
    pub fn close_coop_lobby(&mut self) {
        self.coop_lobby.close();
    }

    /// Toggles the Host / Join Co-op Lobby dialog (`F6` / `Ctrl+M`).
    pub fn toggle_coop_lobby(&mut self) -> bool {
        if self.coop_lobby.is_open() {
            self.coop_lobby.close();
        } else {
            self.open_coop_lobby(None);
        }
        true
    }

    /// Switches between Host and Join tabs in the open Co-op Lobby dialog.
    pub fn coop_lobby_switch_tab(&mut self) -> bool {
        if !self.coop_lobby.is_open() {
            return false;
        }
        let next = match self.coop_lobby.tab() {
            LobbyTab::Host => LobbyTab::Join,
            LobbyTab::Join => LobbyTab::Host,
        };
        self.coop_lobby.set_tab(next);
        true
    }

    /// Types a character into the Co-op Lobby Join ticket box.
    pub fn coop_lobby_char(&mut self, ch: char) -> bool {
        self.coop_lobby.insert_char(ch)
    }

    /// Pastes text (or system clipboard when `text` is `None`) into the Co-op
    /// Lobby Join ticket box.
    pub fn coop_lobby_paste(&mut self, text: Option<&str>) -> bool {
        match text {
            Some(s) => self.coop_lobby.paste_text(s),
            None => clipboard_paste().is_some_and(|s| self.coop_lobby.paste_text(&s)),
        }
    }

    /// Deletes the last character from the Co-op Lobby Join ticket box.
    pub fn coop_lobby_backspace(&mut self) -> bool {
        self.coop_lobby.backspace()
    }

    /// Switches between the Host and Join tabs in the open Co-op Lobby (`Tab`).
    pub fn coop_lobby_toggle_tab(&mut self) -> bool {
        if !self.coop_lobby.is_open() {
            return false;
        }
        let next = match self.coop_lobby.tab() {
            LobbyTab::Host => LobbyTab::Join,
            LobbyTab::Join => LobbyTab::Host,
        };
        self.coop_lobby.set_tab(next);
        true
    }

    /// Submits the active Co-op Lobby tab (`Enter`).
    pub fn coop_lobby_submit(&mut self) -> Outcome {
        let snap = self.coop_snapshot();
        match self.coop_lobby.submit(&snap) {
            Some(cmd) => self.execute_coop_ui_command(cmd),
            None => Outcome::default(),
        }
    }

    /// Handles `Escape` while the Co-op Lobby dialog is open.
    pub fn coop_lobby_escape(&mut self) -> Outcome {
        let snap = self.coop_snapshot();
        match self.coop_lobby.escape(&snap) {
            Some(cmd) => self.execute_coop_ui_command(cmd),
            None => Outcome::default(),
        }
    }

    /// Whether a co-op session is active or asynchronously starting up.
    pub fn coop_active(&self) -> bool {
        self.coop.is_active_or_starting()
    }

    /// Whether the local player is currently the authoritative co-op Host.
    pub fn coop_is_host(&self) -> bool {
        self.coop.is_host()
    }

    /// Whether the local player is currently a joined/connecting co-op Peer.
    pub fn coop_is_peer(&self) -> bool {
        self.coop.is_peer()
    }

    /// Whether local saves/autosaves must be blocked because this `Game` is
    /// currently connected as a peer OR holds a remote host's restored world
    /// (persists even after disconnect so a host's world never overwrites a
    /// guest's local REDB/standalone save slots).
    pub fn coop_blocks_local_save(&self) -> bool {
        self.coop.is_peer() || self.coop_remote_world
    }

    /// Returns `(host_house, host_family)` if a verified `"SCOP"` v2 snapshot
    /// is parked awaiting guest shell restaging onto the host's matching lot.
    pub fn pending_coop_restage(&self) -> Option<(u32, u32)> {
        self.coop.pending_restage_target()
    }

    /// Detaches and returns the live [`CoopController`] (used when the shell
    /// re-stages a fresh `Game` for a remote host's lot before completing a
    /// parked snapshot restore).
    pub fn take_coop_controller(&mut self) -> CoopController {
        std::mem::take(&mut self.coop)
    }

    /// Installs a transferred [`CoopController`] onto a freshly staged `Game`.
    pub fn install_coop_controller(&mut self, coop: CoopController, mark_remote: bool) {
        self.coop = coop;
        if mark_remote {
            self.coop_remote_world = true;
        }
    }

    /// Attempts to complete a parked host-lot snapshot restore onto this
    /// candidate `Game`, preserving the parked snapshot on `OtherHouse` /
    /// `OtherLoad` mismatches so the shell can try fallback local candidates.
    pub fn try_complete_pending_coop_restage(
        &mut self,
    ) -> Result<(), crate::coop::RestageAttemptError> {
        let note = self.coop.try_complete_pending_restage(&mut self.lot)?;
        self.coop_remote_world = true;
        self.sync_after_coop_snapshot_restore();
        self.note = Some(note);
        Ok(())
    }

    /// Completes a parked host-lot snapshot restore onto this newly staged
    /// `Game`, verifying the snapshot transactionally BEFORE sending the
    /// completion ACK to the host.
    pub fn complete_pending_coop_restage(&mut self) -> Result<(), String> {
        let note = self.coop.complete_pending_restage(&mut self.lot)?;
        self.coop_remote_world = true;
        self.sync_after_coop_snapshot_restore();
        self.note = Some(note);
        Ok(())
    }

    /// Fails and cleans up a parked host-lot restage with an actionable message.
    pub fn fail_pending_coop_restage(&mut self, reason: String) {
        self.coop
            .fail_pending_restage(&mut self.lot, reason.clone());
        self.note = Some(reason);
    }

    /// Local assigned [`sims_net::PlayerId`], once hosting or joined.
    pub fn coop_local_player_id(&self) -> Option<NetPlayerId> {
        self.coop.local_player_id()
    }

    /// Currently selected Sim for the local player (their per-player selection
    /// in co-op, or `lot.world().selected()` in single-player).
    pub fn local_selected_sim(&self) -> Option<u16> {
        if self.coop.is_active() {
            self.coop.local_selected_sim()
        } else {
            self.lot.world().selected()
        }
    }

    /// Shareable `simscoop1...` invite ticket when hosting over `iroh`.
    pub fn coop_invite_ticket(&self) -> Option<&str> {
        self.coop.invite_ticket()
    }

    /// Read-only access to the live [`CoopController`].
    pub fn coop_controller(&self) -> &CoopController {
        &self.coop
    }

    /// Sets the local player display name for subsequent co-op sessions.
    pub fn coop_set_player_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        if !name.trim().is_empty() {
            self.coop_player_name = name;
        }
    }

    /// Configures `iroh` binding mode (`P2pDefault` or `Loopback`) for async
    /// co-op startup.
    pub fn coop_set_iroh_config(&mut self, config: IrohConfig) {
        self.coop_iroh_config = config;
    }

    /// Marks whether this lot is an away-from-home travel visit or community
    /// destination edit lot (refused for v1 home-lot co-op).
    pub fn coop_set_travel_blocked(&mut self, blocked: bool) {
        self.coop_travel_blocked = blocked;
    }

    /// Checks v1 home-lot co-op eligibility per `docs/vision.md`:
    /// refuses tutorial lots (`Global 26 != 0`) and away-from-home travel /
    /// community destination lots cleanly with an explicit message.
    pub fn coop_preflight_check(&self) -> Result<(), String> {
        if self.stopped.is_some() {
            return Err("cannot start co-op while the lot is stopped".into());
        }
        if self.lot.world().globals.get(26).copied().unwrap_or(0) != 0 {
            return Err(
                "Tutorial lots are single-player only and cannot be played in co-op mode.".into(),
            );
        }
        if self.coop_travel_blocked || !self.lot.residential() {
            return Err(
                "Co-op multiplayer v1 is only available on home lots (return home from travel first)."
                    .into(),
            );
        }
        Ok(())
    }

    /// Starts hosting a co-op session over real `iroh` asynchronously without
    /// blocking the UI thread.
    pub fn coop_start_host_async(
        &mut self,
        options: Option<CoopStartOptions>,
    ) -> Result<(), String> {
        if let Err(err) = self.coop_preflight_check() {
            self.coop.set_error(err.clone());
            self.note = Some(err.clone());
            return Err(err);
        }
        self.cancel_active_buy_build_tools();
        self.close_menu();
        let opts = options.unwrap_or_else(|| CoopStartOptions {
            player_name: self.coop_player_name.clone(),
            iroh_config: self.coop_iroh_config,
            ..CoopStartOptions::default()
        });
        let install_root = self.hud.install().root.clone();
        if let Err(err) = self.coop.start_host_iroh_async(install_root, opts) {
            self.coop.set_error(err.clone());
            self.note = Some(err.clone());
            return Err(err);
        }
        self.note = Some("Starting co-op host over iroh...".into());
        Ok(())
    }

    /// Validates `ticket` and joins a co-op session over real `iroh`
    /// asynchronously without blocking the UI thread.
    pub fn coop_start_join_async(
        &mut self,
        ticket: String,
        options: Option<CoopStartOptions>,
    ) -> Result<(), String> {
        if let Err(err) = self.coop_preflight_check() {
            self.coop.set_error(err.clone());
            self.note = Some(err.clone());
            return Err(err);
        }
        self.cancel_active_buy_build_tools();
        self.close_menu();
        let opts = options.unwrap_or_else(|| CoopStartOptions {
            player_name: self.coop_player_name.clone(),
            iroh_config: self.coop_iroh_config,
            ..CoopStartOptions::default()
        });
        let install_root = self.hud.install().root.clone();
        if let Err(err) = self.coop.start_join_iroh_async(install_root, ticket, opts) {
            self.note = Some(err.clone());
            return Err(err);
        }
        self.note = Some("Connecting to co-op host...".into());
        Ok(())
    }

    /// Synchronously starts hosting on this lot over `transport`
    /// (`CoopTransport::Memory` or pre-bound `CoopTransport::Iroh`).
    pub fn coop_start_host_with_transport(
        &mut self,
        transport: CoopTransport,
        manifest: ManifestDigest,
        options: CoopStartOptions,
    ) -> Result<(), String> {
        if let Err(err) = self.coop_preflight_check() {
            self.coop.set_error(err.clone());
            self.note = Some(err.clone());
            return Err(err);
        }
        self.cancel_active_buy_build_tools();
        self.close_menu();
        let install = self.hud.install().clone();
        let frames = self.frames.clone();
        self.coop.start_host_with_transport(
            &mut self.lot,
            &install,
            self.play,
            frames,
            transport,
            manifest,
            options,
        )?;
        self.refresh_world();
        Ok(())
    }

    /// Synchronously starts joining a host at `host_addr` over `transport`
    /// (`CoopTransport::Memory` or pre-bound `CoopTransport::Iroh`).
    pub fn coop_start_join_with_transport(
        &mut self,
        transport: CoopTransport,
        host_addr: PeerAddress,
        manifest: ManifestDigest,
        player_name: impl Into<String>,
    ) -> Result<(), String> {
        if let Err(err) = self.coop_preflight_check() {
            self.coop.set_error(err.clone());
            self.note = Some(err.clone());
            return Err(err);
        }
        self.cancel_active_buy_build_tools();
        self.close_menu();
        self.coop
            .start_join_with_transport(transport, host_addr, manifest, player_name)?;
        Ok(())
    }

    /// Submits a raw [`sims_net::Command`] through the active co-op
    /// [`LockstepNode`](sims_net::LockstepNode).
    pub fn coop_submit_command(&mut self, command: NetCommand) -> Result<(), String> {
        self.coop.submit_command(command)
    }

    /// Submits a [`sims_net::BuyBuildEdit`] through the active co-op lockstep
    /// session.
    pub fn coop_submit_buy_build(&mut self, edit: BuyBuildEdit) -> Result<(), String> {
        self.coop.submit_command(NetCommand::BuyBuild(edit))
    }

    /// Polls pending async co-op startup and network/snapshot events.
    pub fn coop_poll(&mut self) -> CoopPollOutput {
        // Ensure no lingering VM Buy/Build cursors (e.g. Wall Cursor) survive
        // into the moment async startup finishes and attaches co-op to `Lot`.
        if self.coop.is_starting() && (self.wall_cursor.is_some() || self.buy_cursor.is_some()) {
            self.cancel_active_buy_build_tools();
        }
        let install = self.hud.install().clone();
        let frames = self.frames.clone();
        let out = self
            .coop
            .poll_and_handle(&mut self.lot, &install, self.play, frames);
        if out.snapshot_restored {
            self.sync_after_coop_snapshot_restore();
        }
        if let Some(note) = &out.note {
            self.note = Some(note.clone());
        }
        out
    }

    /// Deterministic headless co-op pump step (polls network/snapshots,
    /// optionally schedules one host tick, and steps all ready lockstep
    /// batches). Used by integration tests for both `MemoryMesh` and real
    /// `IrohTransport` loopback.
    pub fn coop_pump(
        &mut self,
        schedule_host_tick: bool,
        flush_peer: bool,
    ) -> Result<CoopPumpStats, String> {
        let mut stats = CoopPumpStats::default();
        let poll = self.coop_poll();
        if poll.snapshot_restored {
            stats.snapshots_restored += 1;
        }
        stats.drift_events += poll.drift_events as u32;

        if schedule_host_tick
            && self.coop.is_host()
            && let Some(sims_net::HostStepResult::Scheduled(_)) = self.coop.advance_host_tick()?
        {
            stats.host_scheduled = true;
        }

        let flush = self.coop.is_host() || flush_peer;
        while let Some(report) = self.coop.step_next_batch(&mut self.lot, flush)? {
            stats.batches_applied += 1;
            if report.stepped {
                stats.sim_ticks_stepped += 1;
            }
            self.apply_coop_batch_side_effects(&report);
        }

        let poll2 = self.coop_poll();
        if poll2.snapshot_restored {
            stats.snapshots_restored += 1;
        }
        stats.drift_events += poll2.drift_events as u32;
        self.tick_coop_special_panel_retry();
        Ok(stats)
    }

    /// Bounded retry guard shared by both headless (`coop_pump`) and live
    /// (`advance_coop_while`) co-op loops: if a peer's special-panel answer was
    /// dropped by full host ingress while the same dialog ID remains active,
    /// reopen the modal so the client can retry instead of waiting forever.
    fn tick_coop_special_panel_retry(&mut self) -> bool {
        const COOP_SPECIAL_PANEL_RETRY_PUMPS: u32 = 120;
        let Some((submitted_id, ref mut pumps)) = self.submitted_coop_special_panel else {
            return false;
        };
        let still_active = self
            .lot
            .coop()
            .and_then(|c| c.active_special_panel.as_ref().map(|(id, _)| *id))
            == Some(submitted_id);
        if still_active {
            *pumps = pumps.saturating_add(1);
            if *pumps >= COOP_SPECIAL_PANEL_RETRY_PUMPS {
                self.submitted_coop_special_panel = None;
                self.sync_special_panel();
                return true;
            }
        } else {
            self.submitted_coop_special_panel = None;
        }
        false
    }

    /// Gracefully leaves any active co-op session, detaching `Lot::coop` and
    /// closing any `iroh` endpoint and background Tokio runtime cleanly.
    pub fn leave_coop(&mut self) {
        let was_remote = self.coop_remote_world;
        self.coop_buy_preview = None;
        self.coop_overlay_buttons = CoopOverlayButtons::default();
        self.submitted_coop_special_panel = None;
        self.coop.leave_and_shutdown(&mut self.lot);
        self.note = Some(if was_remote {
            "Disconnected from co-op session (read-only remote lot; return to Neighborhood for local saves)."
                .into()
        } else {
            "Disconnected from co-op session.".into()
        });
    }

    /// Non-blocking interactive Leave/Cancel for UI buttons so endpoint drain
    /// and runtime shutdown never stall the `winit` UI thread.
    pub fn leave_coop_nonblocking(&mut self) {
        let was_remote = self.coop_remote_world;
        self.coop_buy_preview = None;
        self.coop_overlay_buttons = CoopOverlayButtons::default();
        self.submitted_coop_special_panel = None;
        self.coop.leave_nonblocking(&mut self.lot);
        self.note = Some(if was_remote {
            "Disconnected from co-op session (read-only remote lot; return to Neighborhood for local saves)."
                .into()
        } else {
            "Disconnected from co-op session.".into()
        });
    }

    /// Builds a read-only [`CoopSnapshot`] for the Co-op Lobby & HUD Overlay.
    pub fn coop_snapshot(&self) -> CoopSnapshot {
        let lot = &self.lot;
        let hud = &self.hud;
        self.coop.ui_snapshot(lot, |sim_id| {
            let obj = lot.world().objects.get(&sim_id)?;
            lot.character_names()
                .get(&obj.guid)
                .cloned()
                .or_else(|| hud.name(obj.guid).map(str::to_owned))
                .or_else(|| Some(format!("Sim #{sim_id}")))
        })
    }

    fn execute_coop_ui_command(&mut self, cmd: CoopCommand) -> Outcome {
        self.hud.play(crate::hud::CLICK);
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        match cmd {
            CoopCommand::StartHosting => {
                let _ = self.coop_start_host_async(None);
            }
            CoopCommand::Disconnect | CoopCommand::CancelConnect => {
                self.leave_coop_nonblocking();
            }
            CoopCommand::CopyInviteTicket(ticket) => match clipboard_copy(&ticket) {
                Ok(()) => {
                    self.note = Some("Invite ticket copied to clipboard.".into());
                }
                Err(err) => {
                    self.note = Some(format!("Could not copy invite ticket: {err}"));
                }
            },
            CoopCommand::SetHostOnlyTimeControl(locked) => {
                if let Err(err) = self
                    .coop
                    .submit_command(NetCommand::SetHostSpeedLock { locked })
                {
                    self.note = Some(format!("Could not toggle host time lock: {err}"));
                }
            }
            CoopCommand::JoinSession { ticket } => {
                let _ = self.coop_start_join_async(ticket, None);
            }
            CoopCommand::CloseDialog => {
                self.coop_lobby.close();
            }
        }
        Outcome::redraw()
    }

    fn execute_coop_overlay_action(&mut self, action: CoopOverlayAction) -> Outcome {
        match action {
            CoopOverlayAction::OpenLobby => {
                self.open_coop_lobby(None);
            }
            CoopOverlayAction::CopyTicket => {
                if let Some(ticket) = self.coop.invite_ticket().map(str::to_owned) {
                    match clipboard_copy(&ticket) {
                        Ok(()) => {
                            self.coop_overlay_buttons.copied_flash = true;
                            self.note = Some("Invite ticket copied to clipboard.".into());
                        }
                        Err(err) => {
                            self.note = Some(format!("Could not copy invite ticket: {err}"));
                        }
                    }
                }
            }
            CoopOverlayAction::Leave => {
                self.leave_coop_nonblocking();
            }
        }
        Outcome::redraw()
    }

    fn advance_coop_while(
        &mut self,
        clock_elapsed_us: u64,
        step_elapsed_us: u64,
        mut keep_going: impl FnMut(u32) -> bool,
    ) -> (u32, bool) {
        let toast_expired = self
            .coop
            .tick_toasts((clock_elapsed_us / 1_000).min(u64::from(u32::MAX)) as u32);
        if toast_expired {
            self.blink_dirty = true;
        }
        let poll = self.coop_poll();
        if poll.redraw {
            self.blink_dirty = true;
        }
        if !self.coop.is_active() {
            return (0, false);
        }

        let is_host = self.coop.is_host();
        if is_host {
            self.coop.accumulate_host_elapsed(step_elapsed_us, 8);
        }

        let mut ran = 0u32;
        let mut capped = false;
        let mut any_batch = false;

        // Interleave host tick scheduling and stepping 1-for-1 inside the budget
        // loop so the host never yields with scheduled-but-unapplied batches
        // that would create a tick gap for a mid-game joiner.
        while self.stopped.is_none() {
            if !keep_going(ran) {
                capped = true;
                break;
            }
            match self.coop.step_next_batch(&mut self.lot, is_host) {
                Ok(Some(report)) => {
                    any_batch = true;
                    if report.stepped {
                        ran += 1;
                    }
                    self.apply_coop_batch_side_effects(&report);
                    continue;
                }
                Ok(None) => {}
                Err(err) => {
                    self.note = Some(err);
                    break;
                }
            }
            if is_host {
                match self.coop.schedule_one_host_tick_if_due() {
                    Ok(true) => match self.coop.step_next_batch(&mut self.lot, true) {
                        Ok(Some(report)) => {
                            any_batch = true;
                            if report.stepped {
                                ran += 1;
                            }
                            self.apply_coop_batch_side_effects(&report);
                            continue;
                        }
                        Ok(None) => break,
                        Err(err) => {
                            self.note = Some(err);
                            break;
                        }
                    },
                    Ok(false) => break,
                    Err(err) => {
                        self.note = Some(err);
                        break;
                    }
                }
            } else {
                break;
            }
        }

        if self.tick_coop_special_panel_retry() {
            self.blink_dirty = true;
        }
        if any_batch || poll.snapshot_restored {
            self.dirty = true;
            self.blink_dirty = true;
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
        }
        (ran, capped)
    }

    fn apply_coop_batch_side_effects(&mut self, report: &sims_vm::coop::CoopBatchReport) {
        if let Some(update) = &report.update {
            let at = self.lot.ticks();
            for line in in_order(at, &update.errors, &update.frame_errors) {
                self.errors.push(at, line);
            }
            self.view.masks_rebuilt(update.masks_rebuilt);
            self.keep_sounds(update.sounds.clone());
            self.keep_ui_effects(update.ui_effects.clone());
        }
        if report.opened_dialog.is_some() {
            self.hud.clear_vm_dialog();
        }
        if report.opened_special_panel.is_some() {
            self.submitted_coop_special_panel = None;
            self.close_special_panel_ui();
            self.sync_special_panel();
        }
        let mut visual_changed = report.stepped
            || report.opened_dialog.is_some()
            || report.opened_special_panel.is_some();
        for (stamped, status) in &report.command_results {
            match status {
                sims_vm::coop::CoopCommandStatus::BuyBuildApplied
                | sims_vm::coop::CoopCommandStatus::QueuedInteraction { .. }
                | sims_vm::coop::CoopCommandStatus::CancelledInteraction { .. }
                | sims_vm::coop::CoopCommandStatus::DialogAnswered { .. }
                | sims_vm::coop::CoopCommandStatus::Applied => {
                    visual_changed = true;
                }
                sims_vm::coop::CoopCommandStatus::Refused(reason)
                    if Some(stamped.issuer) == self.coop.local_player_id() =>
                {
                    self.submitted_coop_special_panel = None;
                    visual_changed = true;
                    self.note = Some(format!("Co-op command refused: {reason}"));
                }
                sims_vm::coop::CoopCommandStatus::RejectedBySession(reason)
                    if Some(stamped.issuer) == self.coop.local_player_id() =>
                {
                    self.submitted_coop_special_panel = None;
                    visual_changed = true;
                    self.note = Some(format!("Co-op command rejected: {reason:?}"));
                }
                _ => {}
            }
        }
        if visual_changed {
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
            if self.lot.asking_special_panel().is_none() {
                self.submitted_coop_special_panel = None;
                self.close_special_panel_ui();
            } else {
                self.sync_special_panel();
            }
            self.hud.sync_character_names(self.lot.character_names());
            let world = self.lot.world();
            self.people.refresh(world);
            if let Ok(shape) = sims_render::Lot::from_world(world) {
                self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation());
            }
            if self.hud.controls.mode == Mode::Buy {
                self.layout_buy();
            } else if self.hud.controls.mode == Mode::Build {
                self.layout_build();
            }
        }
    }

    fn sync_after_coop_snapshot_restore(&mut self) {
        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        if self.coop.is_peer() {
            self.coop_remote_world = true;
        }
        self.stopped = None;
        self.errors = Errors::default();
        self.buy_cursor = None;
        self.coop_buy_preview = None;
        self.buy_selected = None;
        self.buy_dragging = None;
        self.buy_commit_on_release = false;
        self.wall_cursor = None;
        self.build_drag = None;
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
        self.close_menu();
        self.reached = None;
        self.listed = None;
        self.highlighted = None;
        self.submitted_coop_special_panel = None;
        self.close_special_panel_ui();
        self.pending_travel_lot = None;
        self.hud.clear_vm_dialog();
        self.sync_special_panel();
        self.hud.sync_character_names(self.lot.character_names());
        self.people.refresh(self.lot.world());
        if let Ok(shape) = sims_render::Lot::from_world(self.lot.world()) {
            self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation());
        }
        self.clear_lot_cache();
        self.restore_generation = self.restore_generation.wrapping_add(1);
        self.blink_dirty = true;
    }

    /// Presentation-only Hand Tool pickup preview for Buy/Build modes in co-op
    /// (never calls `Lot::buy_pick_up`, keeping the shared world immutable until
    /// `BuyBuildEdit::MoveObject` / `SellObject` commits via lockstep).
    fn coop_pick_up_preview(&mut self, target: u16, point: [i32; 2]) -> Outcome {
        if let Some(obj) = self.lot.world().objects.get(&target)
            && obj.person.is_none()
        {
            self.coop_buy_preview = Some(CoopBuyPreview {
                guid: obj.guid,
                existing_object_id: Some(target),
                position: obj.position,
                facing: obj.data.get(1).copied().unwrap_or(0).rem_euclid(8),
            });
            self.buy_in_house = true;
            self.buy_dragging = Some(point);
            self.buy_commit_on_release = false;
            self.pointer = Some(point);
            self.hud.play("UI_object_move");
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return Outcome::redraw();
        }
        self.hud.play(crate::hud::ERROR);
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    fn hover_buy_coop(&mut self, point: [i32; 2]) -> bool {
        let at = [point[0] - pick::ORIGIN_COLUMN, point[1]];
        let fine = sims_render::unproject_terrain(&self.lot_shape, &self.view.camera, at)
            .unwrap_or_else(|| self.view.camera.unproject_fine(at));
        let snapped = [
            (fine[0] & !15) + 8,
            (fine[1] & !15) + 8,
            self.view.camera.level,
        ];
        if let Some(preview) = self.coop_buy_preview.as_mut() {
            if let Some(anchor) = self.buy_dragging {
                let dx = point[0] - anchor[0];
                let dy = point[1] - anchor[1];
                if dx * dx + dy * dy > 36 {
                    let quad = if dy < 0 {
                        if dx < 0 { 6 } else { 4 }
                    } else if dx < 0 {
                        0
                    } else {
                        2
                    };
                    let desired =
                        (quad - 2 * i16::from(self.view.camera.rotation & 3)).rem_euclid(8);
                    preview.facing = desired;
                    self.buy_facing = Some(desired);
                    return true;
                }
            }
            preview.position = snapped;
            return true;
        }
        if let Some(guid) = self.buy_selected {
            let facing = self
                .buy_facing
                .unwrap_or_else(|| sims_vm::default_buy_direction(self.view.camera.rotation));
            self.coop_buy_preview = Some(CoopBuyPreview {
                guid,
                existing_object_id: None,
                position: snapped,
                facing,
            });
            self.buy_in_house = true;
            return true;
        }
        false
    }

    fn commit_buy_coop(&mut self) -> Outcome {
        let Some(preview) = self.coop_buy_preview.take() else {
            return Outcome::default();
        };
        let edit = match preview.existing_object_id {
            Some(object_id) => BuyBuildEdit::MoveObject {
                object_id,
                position: preview.position,
                facing: preview.facing,
                container: None,
            },
            None => BuyBuildEdit::BuyObject {
                guid: preview.guid,
                position: preview.position,
                facing: preview.facing,
                container: None,
            },
        };
        match self.coop.submit_command(NetCommand::BuyBuild(edit)) {
            Ok(()) => {
                self.buy_facing = Some(preview.facing);
                self.buy_last_guid = Some(preview.guid);
                if preview.existing_object_id.is_some() || !self.shift_held {
                    self.buy_selected = None;
                    self.hud.buy_state.selected = None;
                }
                self.hud.play("UI_object_move_place");
            }
            Err(e) => {
                self.note = Some(format!("Co-op placement not sent: {e}"));
                self.hud.play(crate::hud::ERROR);
            }
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    fn commit_build_coop(&mut self, drag: BuildDrag) -> Outcome {
        // Stay strictly below `LockstepNode`'s 64-command per-player queue cap
        // so a single Build drag never silently drops trailing commands.
        const MAX_COOP_BUILD_DRAG_COMMANDS: usize = 48;
        // `sims_vm::build::cell_index(x, y) = x * 64 + y` (column-major stride 64).
        const FULL_GRID_STRIDE: usize = 64;
        let level = self.view.camera.level.clamp(1, 2) as u8;
        let mut sent = 0usize;
        let mut err_msg: Option<String> = None;
        match drag {
            BuildDrag::Wall {
                start_corner,
                end_corner,
                style,
                mode,
                plan,
            } => {
                if let Some(plan) = plan
                    && plan.valid
                    && plan.affordable
                {
                    let (x0, x1) = (
                        start_corner[0].min(end_corner[0]),
                        start_corner[0].max(end_corner[0]),
                    );
                    let y = start_corner[1] as i16;
                    let count = usize::try_from(x1.saturating_sub(x0)).unwrap_or(0);
                    if count > MAX_COOP_BUILD_DRAG_COMMANDS {
                        err_msg = Some(format!(
                            "Co-op wall drag ({count} segments) exceeds {MAX_COOP_BUILD_DRAG_COMMANDS}-segment limit; drag a smaller section."
                        ));
                    } else if x1 > x0 {
                        for x in x0..x1 {
                            let edit = if mode.is_demolish() {
                                BuyBuildEdit::RemoveWall {
                                    x: x as i16,
                                    y,
                                    level,
                                }
                            } else {
                                BuyBuildEdit::PlaceWall {
                                    x: x as i16,
                                    y,
                                    level,
                                    style: style.style_id(),
                                    pattern: style.default_pattern(),
                                }
                            };
                            if let Err(e) = self.coop.submit_command(NetCommand::BuyBuild(edit)) {
                                err_msg = Some(e);
                                break;
                            }
                            sent += 1;
                        }
                    } else {
                        err_msg = Some(
                            "Co-op wall edits currently support horizontal wall segments.".into(),
                        );
                    }
                }
            }
            BuildDrag::Floor {
                floor_id,
                mode,
                plan,
                ..
            } => {
                if let Some(plan) = plan
                    && plan.valid
                    && plan.affordable
                {
                    if plan.floor_changes.len() > MAX_COOP_BUILD_DRAG_COMMANDS {
                        err_msg = Some(format!(
                            "Co-op floor drag ({} tiles) exceeds {MAX_COOP_BUILD_DRAG_COMMANDS}-tile limit; drag a smaller area.",
                            plan.floor_changes.len()
                        ));
                    } else {
                        let pattern = if mode.is_remove() { 0 } else { floor_id };
                        for &(idx, _before, _after) in &plan.floor_changes {
                            let x = (idx / FULL_GRID_STRIDE) as i16;
                            let y = (idx % FULL_GRID_STRIDE) as i16;
                            let edit = BuyBuildEdit::SetFloorTile {
                                x,
                                y,
                                level,
                                pattern,
                            };
                            if let Err(e) = self.coop.submit_command(NetCommand::BuyBuild(edit)) {
                                err_msg = Some(e);
                                break;
                            }
                            sent += 1;
                        }
                    }
                }
            }
            BuildDrag::Wallpaper {
                faces,
                pattern_id,
                mode,
                plan,
            } => {
                if let Some(plan) = plan
                    && plan.valid
                    && plan.affordable
                {
                    if faces.len() > MAX_COOP_BUILD_DRAG_COMMANDS {
                        err_msg = Some(format!(
                            "Co-op wallpaper drag ({} faces) exceeds {MAX_COOP_BUILD_DRAG_COMMANDS}-face limit; drag a smaller section.",
                            faces.len()
                        ));
                    } else {
                        let pattern = if mode.is_strip() { 0 } else { pattern_id };
                        for face in faces {
                            let side = match face.bit {
                                WALL_NORTH => 0,
                                WALL_EAST => 1,
                                WALL_SOUTH => 2,
                                WALL_WEST => 3,
                                _ => continue,
                            };
                            let edit = BuyBuildEdit::SetWallpaper {
                                x: i16::from(face.tile[0]),
                                y: i16::from(face.tile[1]),
                                level,
                                side,
                                pattern,
                            };
                            if let Err(e) = self.coop.submit_command(NetCommand::BuyBuild(edit)) {
                                err_msg = Some(e);
                                break;
                            }
                            sent += 1;
                        }
                    }
                }
            }
            BuildDrag::Terrain { .. } | BuildDrag::Water { .. } | BuildDrag::Pool { .. } => {
                err_msg = Some(
                    "Terrain, Water, and Pool tools are not yet supported in co-op mode.".into(),
                );
            }
        }
        if let Some(msg) = err_msg {
            self.note = Some(msg);
            self.hud.play(crate::hud::ERROR);
        } else if sent > 0 {
            self.hud.play(crate::hud::BLD_DRAG_UP);
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    fn ensure_coop_art(&mut self) {
        if self.coop_art.is_none()
            && let Ok(ui) = UiGraphics::open(self.hud.install())
            && let Ok(art) = sims_ui::coop::load_art(&ui, US_ENGLISH)
        {
            self.coop_art = Some(art);
        }
    }

    /// Draws distinct colored faceted diamond plumbob indicators above Sims
    /// selected by remote teammates (and above the local peer's own selected
    /// Sim when it differs from the host's native plumbob). Never runs in
    /// single-player (`coop == None`).
    fn draw_coop_remote_indicators(&mut self, viewport: (u32, u32)) {
        let Some(session) = self.coop.session() else {
            return;
        };
        let host_selected = session
            .players
            .get(&session.host_id)
            .and_then(|p| p.selected_sim);
        let local_id = self.coop.local_player_id();
        let world = self.lot.world();
        let vp_w = i32::try_from(viewport.0)
            .unwrap_or(i32::MAX)
            .min(self.canvas.width as i32);
        let vp_h = i32::try_from(viewport.1)
            .unwrap_or(i32::MAX)
            .min(self.canvas.height as i32);
        if vp_w <= 0 || vp_h <= 0 {
            return;
        }

        // Group players by selected Sim so co-selected Sims fan out badges cleanly.
        type RemotePlumbobEntry = (NetPlayerId, String, [u8; 3], bool);
        let mut by_sim: BTreeMap<u16, Vec<RemotePlumbobEntry>> = BTreeMap::new();
        for player in session.players.values() {
            let Some(sim_id) = player.selected_sim else {
                continue;
            };
            // The host's selected Sim already has the native 3D plumbob when we
            // are the host (or when it is the host's own Sim); draw colored
            // accents for every remote player AND for the local peer when they
            // select a different Sim from the host.
            let is_local = Some(player.player_id) == local_id;
            if is_local && Some(sim_id) == host_selected && player.player_id == session.host_id {
                continue;
            }
            by_sim.entry(sim_id).or_default().push((
                player.player_id,
                player.name.clone(),
                player_badge_color(player.player_id),
                is_local,
            ));
        }
        if by_sim.is_empty() {
            return;
        }

        let (head_lift, half_w, half_h) = match self.view.camera.zoom {
            sims_render::Zoom::Far => (34i32, 4i32, 7i32),
            sims_render::Zoom::Medium => (68i32, 6i32, 11i32),
            sims_render::Zoom::Near => (132i32, 9i32, 16i32),
        };
        // Gentle vertical bob driven by `clock_us` (pure presentation).
        let phase = ((self.clock_us / 120_000) % 4) as i32;
        let bob = match phase {
            1 => -2,
            2 => -1,
            3 => 1,
            _ => 0,
        };

        for (sim_id, markers) in by_sim {
            let Some(obj) = world.objects.get(&sim_id) else {
                continue;
            };
            if obj.position[2] > self.view.camera.level
                || (obj.position[0] == 0 && obj.position[1] == 0)
            {
                continue;
            }
            let base = lot_point(&self.view.camera, &self.lot_shape, obj.position);
            let base_x = base[0] + pick::ORIGIN_COLUMN;
            let base_y = base[1] - head_lift + bob;
            let spacing = half_w * 2 + 6;
            let start_x = base_x - ((markers.len() as i32 - 1) * spacing) / 2;
            for (idx, (pid, _name, color, is_local)) in markers.iter().enumerate() {
                let cx = start_x + (idx as i32) * spacing;
                let cy = base_y;
                draw_coop_plumbob_diamond(
                    &mut self.canvas,
                    cx,
                    cy,
                    half_w,
                    half_h,
                    *color,
                    *is_local,
                    (vp_w, vp_h),
                );
                let _ = pid;
            }
        }
    }

    fn draw_coop_buy_preview(&mut self, viewport: (u32, u32)) {
        let Some(preview) = &self.coop_buy_preview else {
            return;
        };
        let vp_w = i32::try_from(viewport.0)
            .unwrap_or(i32::MAX)
            .min(self.canvas.width as i32);
        let vp_h = i32::try_from(viewport.1)
            .unwrap_or(i32::MAX)
            .min(self.canvas.height as i32);
        let pt = lot_point(&self.view.camera, &self.lot_shape, preview.position);
        let cx = pt[0] + pick::ORIGIN_COLUMN;
        let cy = pt[1];
        draw_coop_plumbob_diamond(
            &mut self.canvas,
            cx,
            cy,
            10,
            6,
            [64, 224, 255],
            true,
            (vp_w, vp_h),
        );
    }

    fn draw_coop_hud_and_lobby(&mut self) {
        let show_overlay = self.coop.is_active_or_starting();
        let show_lobby = self.coop_lobby.is_open();
        if !show_overlay && !show_lobby {
            self.coop_overlay_buttons.clear_rects();
            return;
        }
        self.ensure_coop_art();
        let snapshot = self.coop_snapshot();
        let art = self.coop_art.as_ref().unwrap_or(&self.hud.art);
        let (w, h) = (
            u32::try_from(self.canvas.width).unwrap_or(0),
            u32::try_from(self.canvas.height).unwrap_or(0),
        );
        let Ok(mut frame) =
            sims_ui::frame::Frame::from_packed(w, h, std::mem::take(&mut self.canvas.color))
        else {
            return;
        };
        if show_overlay {
            CoopHudOverlay::draw(&mut frame, &snapshot, art);
            let overlay_layout = CoopHudOverlay::layout([w, h], &snapshot, art);
            if let Some(panel) = overlay_layout.status_panel {
                let hosting = matches!(snapshot.connection, CoopConnectionState::Hosting { .. });
                self.coop_overlay_buttons.layout_and_draw(
                    &mut frame,
                    art,
                    panel,
                    hosting,
                    self.pointer,
                );
            } else {
                self.coop_overlay_buttons.clear_rects();
            }
        } else {
            self.coop_overlay_buttons.clear_rects();
        }
        if show_lobby {
            self.coop_lobby.draw(&mut frame, &snapshot, art);
        }
        self.canvas.color = frame.into_packed();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoopOverlayAction {
    OpenLobby,
    CopyTicket,
    Leave,
}

#[derive(Debug, Default)]
struct CoopOverlayButtons {
    lobby_rect: Option<sims_ui::frame::Rect>,
    copy_rect: Option<sims_ui::frame::Rect>,
    leave_rect: Option<sims_ui::frame::Rect>,
    pressed: Option<CoopOverlayAction>,
    copied_flash: bool,
}

impl CoopOverlayButtons {
    fn clear_rects(&mut self) {
        self.lobby_rect = None;
        self.copy_rect = None;
        self.leave_rect = None;
        self.pressed = None;
    }

    fn hit(&self, point: [i32; 2]) -> Option<CoopOverlayAction> {
        if self
            .lobby_rect
            .is_some_and(|r| r.contains(point[0], point[1]))
        {
            return Some(CoopOverlayAction::OpenLobby);
        }
        if self
            .copy_rect
            .is_some_and(|r| r.contains(point[0], point[1]))
        {
            return Some(CoopOverlayAction::CopyTicket);
        }
        if self
            .leave_rect
            .is_some_and(|r| r.contains(point[0], point[1]))
        {
            return Some(CoopOverlayAction::Leave);
        }
        None
    }

    fn contains(&self, point: [i32; 2]) -> bool {
        self.hit(point).is_some()
    }

    fn press(&mut self, point: [i32; 2]) -> bool {
        let hit = self.hit(point);
        self.pressed = hit;
        hit.is_some()
    }

    fn release(&mut self, point: Option<[i32; 2]>) -> Option<CoopOverlayAction> {
        let pressed = self.pressed.take()?;
        let hit = point.and_then(|p| self.hit(p));
        (hit == Some(pressed)).then_some(pressed)
    }

    fn layout_and_draw(
        &mut self,
        frame: &mut sims_ui::frame::Frame,
        art: &Art,
        panel: sims_ui::frame::Rect,
        hosting: bool,
        pointer: Option<[i32; 2]>,
    ) {
        let bar_y = panel.y + panel.height + 4;
        let btn_h = 20;
        let gap = 4;
        let fh = i32::try_from(frame.height()).unwrap_or(600);
        if bar_y + btn_h >= fh - 4 {
            self.clear_rects();
            return;
        }
        if hosting {
            let btn_w = ((panel.width - gap * 2) / 3).max(40);
            self.lobby_rect = Some(sims_ui::frame::Rect::new(panel.x, bar_y, btn_w, btn_h));
            self.copy_rect = Some(sims_ui::frame::Rect::new(
                panel.x + btn_w + gap,
                bar_y,
                btn_w,
                btn_h,
            ));
            let leave_x = panel.x + (btn_w + gap) * 2;
            let leave_w = (panel.x + panel.width - leave_x).max(40);
            self.leave_rect = Some(sims_ui::frame::Rect::new(leave_x, bar_y, leave_w, btn_h));
        } else {
            let btn_w = ((panel.width - gap) / 2).max(48);
            self.lobby_rect = Some(sims_ui::frame::Rect::new(panel.x, bar_y, btn_w, btn_h));
            self.copy_rect = None;
            let leave_x = panel.x + btn_w + gap;
            let leave_w = (panel.x + panel.width - leave_x).max(48);
            self.leave_rect = Some(sims_ui::frame::Rect::new(leave_x, bar_y, leave_w, btn_h));
        }

        let draw_btn = |frame: &mut sims_ui::frame::Frame,
                        rect: sims_ui::frame::Rect,
                        label: &str,
                        action: CoopOverlayAction,
                        danger: bool| {
            let hovered = pointer.is_some_and(|[px, py]| rect.contains(px, py));
            let active = self.pressed == Some(action) && hovered;
            let (bg, border, fg) = if danger {
                if active {
                    ([138, 36, 48], [255, 160, 160], [255, 255, 255])
                } else if hovered {
                    ([110, 30, 42], [230, 120, 130], [255, 232, 232])
                } else {
                    ([74, 22, 34], [176, 86, 98], [235, 200, 205])
                }
            } else if active {
                ([30, 88, 176], [0, 255, 255], [0, 255, 255])
            } else if hovered {
                ([26, 62, 146], [136, 178, 232], [255, 255, 255])
            } else {
                ([16, 36, 108], [88, 120, 204], [195, 205, 205])
            };
            frame.fill(rect, bg);
            frame.fill(
                sims_ui::frame::Rect::new(rect.x, rect.y, rect.width, 1),
                border,
            );
            frame.fill(
                sims_ui::frame::Rect::new(rect.x, rect.y + rect.height - 1, rect.width, 1),
                border,
            );
            frame.fill(
                sims_ui::frame::Rect::new(rect.x, rect.y, 1, rect.height),
                border,
            );
            frame.fill(
                sims_ui::frame::Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height),
                border,
            );
            if let Some(font) = art.font(10).or_else(|| art.font(12)) {
                let tw = font.width(label);
                let tx = rect.x + (rect.width - tw) / 2;
                let ty = rect.y + (rect.height - font.line_height) / 2;
                font.draw(frame, label, [tx, ty], fg);
            }
        };

        if let Some(r) = self.lobby_rect {
            draw_btn(frame, r, "Lobby", CoopOverlayAction::OpenLobby, false);
        }
        if let Some(r) = self.copy_rect {
            let label = if self.copied_flash { "Copied" } else { "Copy" };
            draw_btn(frame, r, label, CoopOverlayAction::CopyTicket, false);
        }
        if let Some(r) = self.leave_rect {
            draw_btn(frame, r, "Leave", CoopOverlayAction::Leave, true);
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CoopBuyPreview {
    guid: u32,
    existing_object_id: Option<u16>,
    position: [i32; 3],
    facing: i16,
}

#[allow(clippy::too_many_arguments)]
fn draw_coop_plumbob_diamond(
    canvas: &mut Canvas,
    cx: i32,
    cy: i32,
    half_w: i32,
    half_h: i32,
    color: [u8; 3],
    bright_ring: bool,
    viewport: (i32, i32),
) {
    if half_w <= 0 || half_h <= 0 {
        return;
    }
    let width = canvas.width as i32;
    let light = [
        color[0].saturating_add(55),
        color[1].saturating_add(55),
        color[2].saturating_add(55),
    ];
    let shadow = [
        (u16::from(color[0]) * 3 / 5) as u8,
        (u16::from(color[1]) * 3 / 5) as u8,
        (u16::from(color[2]) * 3 / 5) as u8,
    ];
    let outline = if bright_ring {
        [255, 255, 255]
    } else {
        [14, 22, 34]
    };

    for dy in -half_h..=half_h {
        let py = cy + dy;
        if py < 0 || py >= viewport.1 {
            continue;
        }
        let span = ((half_h - dy.abs()) * half_w) / half_h;
        let row = py as usize * width as usize;
        for dx in -span..=span {
            let px = cx + dx;
            if px < 0 || px >= viewport.0 {
                continue;
            }
            let is_edge = dx == -span || dx == span || dy.abs() == half_h;
            let rgb = if is_edge {
                outline
            } else if dx < 0 && dy < 0 {
                light
            } else if dx > 0 && dy > 0 {
                shadow
            } else {
                color
            };
            canvas.color[row + px as usize] = sims_render::pack_rgb(rgb);
        }
    }
}

#[derive(Debug, Clone)]
enum BuildDrag {
    Wall {
        start_corner: [i32; 2],
        end_corner: [i32; 2],
        style: WallStyle,
        mode: WallDragMode,
        plan: Option<WallPlan>,
    },
    Floor {
        start_tile: [i32; 2],
        end_tile: [i32; 2],
        floor_id: u16,
        mode: FloorDragMode,
        split_half: Option<u8>,
        plan: Option<FloorPlan>,
    },
    Wallpaper {
        faces: Vec<WallFace>,
        pattern_id: u16,
        mode: WallpaperDragMode,
        plan: Option<WallpaperPlan>,
    },
    Terrain {
        start: [i32; 2],
        end: [i32; 2],
        mode: TerrainMode,
        plan: Option<TerrainPlan>,
    },
    Water {
        start_tile: [i32; 2],
        end_tile: [i32; 2],
        mode: WaterMode,
        plan: Option<WaterPlan>,
    },
    Pool {
        start_tile: [i32; 2],
        end_tile: [i32; 2],
        mode: PoolMode,
        plan: Option<PoolPlan>,
    },
}

fn parse_base_roof_from_house_bytes(bytes: &[u8]) -> Option<RoofState> {
    let iff = sims_formats::iff::Iff::parse(bytes.to_vec()).ok()?;
    let chunk = iff.get(b"HOUS", 0)?;
    let view = sims_formats::lot::HouseView::parse(iff.data(chunk)).ok()?;
    Some(RoofState::from_house_view(&view))
}

const fn vm_roof_pitch(preset: RoofPitchPreset) -> VmRoofPitch {
    match preset {
        RoofPitchPreset::Flat => VmRoofPitch::Flat,
        RoofPitchPreset::Shallow => VmRoofPitch::Shallow,
        RoofPitchPreset::Medium => VmRoofPitch::Medium,
        RoofPitchPreset::Steep => VmRoofPitch::Steep,
    }
}

const fn ui_roof_pitch_preset(pitch: VmRoofPitch) -> RoofPitchPreset {
    match pitch {
        VmRoofPitch::Flat => RoofPitchPreset::Flat,
        VmRoofPitch::Shallow => RoofPitchPreset::Shallow,
        VmRoofPitch::Medium => RoofPitchPreset::Medium,
        VmRoofPitch::Steep => RoofPitchPreset::Steep,
    }
}

const fn render_roof_pitch(pitch: VmRoofPitch) -> RenderRoofPitch {
    match pitch {
        VmRoofPitch::Flat => RenderRoofPitch::Flat,
        VmRoofPitch::Shallow => RenderRoofPitch::Shallow,
        VmRoofPitch::Medium => RenderRoofPitch::Medium,
        VmRoofPitch::Steep => RenderRoofPitch::Steep,
    }
}

fn terrain_coord_for_mode(fine: [i32; 2], active: usize, mode: TerrainMode) -> [i32; 2] {
    match mode {
        TerrainMode::RestoreGrass | TerrainMode::RemoveGrass => [fine[0] >> 4, fine[1] >> 4],
        TerrainMode::Raise
        | TerrainMode::Lower
        | TerrainMode::Level
        | TerrainMode::LevelAverage => sims_vm::snap_wall_corner(fine, active),
    }
}

fn terrain_overlay_for_drag(start: [i32; 2], end: [i32; 2], mode: TerrainMode) -> TerrainOverlay {
    let vertex_drag_target = || {
        if start == end {
            TerrainTarget::Vertex(end)
        } else {
            let min_x = start[0].min(end[0]);
            let max_x = start[0].max(end[0]);
            let min_y = start[1].min(end[1]);
            let max_y = start[1].max(end[1]);
            TerrainTarget::Rect {
                start: [min_x.saturating_sub(1), min_y.saturating_sub(1)],
                end: [max_x, max_y],
            }
        }
    };
    match mode {
        TerrainMode::Raise => TerrainOverlay {
            tool: RenderTerrainTool::Raise,
            target: vertex_drag_target(),
        },
        TerrainMode::Lower => TerrainOverlay {
            tool: RenderTerrainTool::Lower,
            target: vertex_drag_target(),
        },
        TerrainMode::Level | TerrainMode::LevelAverage => TerrainOverlay {
            tool: RenderTerrainTool::Level,
            target: TerrainTarget::Rect { start, end },
        },
        TerrainMode::RestoreGrass => TerrainOverlay {
            tool: RenderTerrainTool::Grass,
            target: if start == end {
                TerrainTarget::Brush {
                    center: end,
                    radius: 0,
                }
            } else {
                TerrainTarget::Rect { start, end }
            },
        },
        TerrainMode::RemoveGrass => TerrainOverlay {
            tool: RenderTerrainTool::Soil,
            target: if start == end {
                TerrainTarget::Brush {
                    center: end,
                    radius: 0,
                }
            } else {
                TerrainTarget::Rect { start, end }
            },
        },
    }
}

fn special_panel_cancel_response(request: &SpecialPanelRequest) -> SpecialPanelResponse {
    match request {
        SpecialPanelRequest::DowntownLotPicker
        | SpecialPanelRequest::VacationLotPicker { .. }
        | SpecialPanelRequest::OldTownLotPicker { .. }
        | SpecialPanelRequest::StudioTownLotPicker { .. }
        | SpecialPanelRequest::MagicTownLotPicker { .. } => SpecialPanelResponse::Lot(None),
        SpecialPanelRequest::CallNeighbor { .. } => SpecialPanelResponse::Neighbor(None),
        SpecialPanelRequest::WardrobeShop { .. }
        | SpecialPanelRequest::PetPicker { .. }
        | SpecialPanelRequest::CharmTransform { .. } => SpecialPanelResponse::Selection {
            accepted: false,
            index: 0,
        },
        SpecialPanelRequest::SpellBook { .. } | SpecialPanelRequest::CookBook { .. } => {
            SpecialPanelResponse::Dismissed
        }
    }
}

fn build_default_shop_items(kind: ShopKind, purchased: &BTreeSet<String>) -> Vec<ShopItem> {
    let make = |id: &str,
                name: &str,
                desc: &str,
                price: i32,
                currency: CurrencyKind,
                supports_try_on: bool| ShopItem {
        id: id.to_owned(),
        name: name.to_owned(),
        description: desc.to_owned(),
        price,
        currency,
        icon: None,
        supports_try_on,
        owned: purchased.contains(id),
        available: true,
    };
    match kind {
        ShopKind::DowntownBoutique(WardrobeCategory::Formal) => vec![
            make(
                "dt_formal_1",
                "Evening Suit",
                "Formal outfit",
                250,
                CurrencyKind::Simoleons,
                true,
            ),
            make(
                "dt_formal_2",
                "Gala Gown",
                "Formal outfit",
                350,
                CurrencyKind::Simoleons,
                true,
            ),
            make(
                "dt_formal_3",
                "Classic Tuxedo",
                "Formal outfit",
                425,
                CurrencyKind::Simoleons,
                true,
            ),
        ],
        ShopKind::DowntownBoutique(WardrobeCategory::Swimwear) => vec![
            make(
                "dt_swim_1",
                "Poolside Swimsuit",
                "Swimwear",
                120,
                CurrencyKind::Simoleons,
                true,
            ),
            make(
                "dt_swim_2",
                "Resort Trunks",
                "Swimwear",
                150,
                CurrencyKind::Simoleons,
                true,
            ),
        ],
        ShopKind::DowntownBoutique(WardrobeCategory::Pajamas) => vec![
            make(
                "dt_pj_1",
                "Silk Sleepwear",
                "Pajamas",
                140,
                CurrencyKind::Simoleons,
                true,
            ),
            make(
                "dt_pj_2",
                "Lounge Set",
                "Pajamas",
                180,
                CurrencyKind::Simoleons,
                true,
            ),
        ],
        ShopKind::DowntownBoutique(WardrobeCategory::Winterwear) => vec![
            make(
                "dt_winter_1",
                "Alpine Parka",
                "Winterwear",
                220,
                CurrencyKind::Simoleons,
                true,
            ),
            make(
                "dt_winter_2",
                "Wool Overcoat",
                "Winterwear",
                275,
                CurrencyKind::Simoleons,
                true,
            ),
        ],
        ShopKind::DowntownBoutique(WardrobeCategory::Everyday) => vec![
            make(
                "dt_day_1",
                "Casual Ensemble",
                "Everyday outfit",
                160,
                CurrencyKind::Simoleons,
                true,
            ),
            make(
                "dt_day_2",
                "City Jacket",
                "Everyday outfit",
                210,
                CurrencyKind::Simoleons,
                true,
            ),
        ],
        ShopKind::VacationKiosk => vec![
            make(
                "vac_souvenir_1",
                "Island Tiki",
                "Vacation souvenir",
                75,
                CurrencyKind::Simoleons,
                false,
            ),
            make(
                "vac_souvenir_2",
                "Snow Globe",
                "Vacation souvenir",
                95,
                CurrencyKind::Simoleons,
                false,
            ),
            make(
                "vac_rental_1",
                "Volleyball Set",
                "Resort rental",
                125,
                CurrencyKind::Simoleons,
                false,
            ),
        ],
        ShopKind::PetAdoption => vec![
            make(
                "pet_dog_1",
                "Adopt Dog",
                "Household dog",
                399,
                CurrencyKind::Simoleons,
                false,
            ),
            make(
                "pet_cat_1",
                "Adopt Cat",
                "Household cat",
                399,
                CurrencyKind::Simoleons,
                false,
            ),
            make(
                "pet_collar_1",
                "Pet Collar",
                "Pet accessory",
                50,
                CurrencyKind::Simoleons,
                false,
            ),
        ],
        ShopKind::StudioWardrobe => vec![
            make(
                "st_wardrobe_1",
                "Red Carpet Look",
                "Studio wardrobe",
                400,
                CurrencyKind::Simoleons,
                true,
            ),
            make(
                "st_wardrobe_2",
                "Stage Outfit",
                "Studio wardrobe",
                550,
                CurrencyKind::Simoleons,
                true,
            ),
        ],
        ShopKind::MagicVendor => vec![
            make(
                "mt_ing_1",
                "Spell Reagent I",
                "Magic ingredient",
                21,
                CurrencyKind::MagiCoins,
                false,
            ),
            make(
                "mt_ing_2",
                "Spell Reagent II",
                "Magic ingredient",
                31,
                CurrencyKind::MagiCoins,
                false,
            ),
            make(
                "mt_ing_3",
                "Spell Reagent III",
                "Magic ingredient",
                27,
                CurrencyKind::MagiCoins,
                false,
            ),
            make(
                "mt_ing_4",
                "Spell Reagent IV",
                "Magic ingredient",
                21,
                CurrencyKind::MagiCoins,
                false,
            ),
        ],
    }
}

fn panel_mode_for(mode: Mode) -> PanelMode {
    match mode {
        Mode::Live => PanelMode::Live,
        Mode::Buy => PanelMode::Buy,
        Mode::Build => PanelMode::Build,
        Mode::Options => PanelMode::Options,
    }
}

fn ui_mode(mode: PanelMode) -> Mode {
    match mode {
        PanelMode::Live => Mode::Live,
        PanelMode::Build => Mode::Build,
        PanelMode::Buy => Mode::Buy,
        PanelMode::Options | PanelMode::Camera => Mode::Options,
    }
}

/// The errors play mode caught.
#[derive(Debug, Default)]
struct Errors {
    /// One line each for the first [`LOGGED`] different errors, then one
    /// saying more came. An error that differs from one listed only in
    /// its tick is counted, not listed again.
    lines: Vec<String>,
    /// The different errors seen, without their ticks, the first
    /// [`SEEN`].
    seen: BTreeSet<String>,
    /// How many were caught, listed or not.
    count: u64,
    /// The last new error and the tick count after the step that caught
    /// it. Repeats are not shown again.
    last: Option<(u32, String)>,
}

impl Errors {
    fn push(&mut self, at: u32, line: String) {
        self.count += 1;
        // A line starts with its tick: "tick 95647, object 5: ...".
        let what = line
            .strip_prefix("tick ")
            .and_then(|rest| rest.split_once(", "))
            .map_or(line.as_str(), |(_, what)| what);
        // Once SEEN different errors are remembered, a new one cannot be
        // told from its repeats, so it is only counted.
        if self.seen.contains(what) || self.seen.len() >= SEEN {
            return;
        }
        self.seen.insert(what.to_owned());
        if self.lines.len() < LOGGED {
            self.lines.push(line.clone());
        } else if self.lines.len() == LOGGED {
            self.lines
                .push("further errors are counted, not listed".into());
        }
        self.last = Some((at, line));
    }

    /// The line across the top at tick count `now`: the last error, for
    /// [`SHOWN_TICKS`] after it was caught.
    fn recent(&self, now: u32) -> Option<String> {
        let (at, line) = self.last.as_ref()?;
        if now.saturating_sub(*at) >= SHOWN_TICKS {
            return None;
        }
        let count = match self.count {
            1 => String::new(),
            n => format!(" ({n} so far)"),
        };
        Some(format!("Error caught{count}: {line}"))
    }
}

/// Highlight colour drawn around HUD controls targeted by UI Effect (opcode 34,
/// `0x476485` / `0x47690d` / `0x4769c3` → `0x5de13f`).
const UI_EFFECT_HIGHLIGHT_RGB: [u8; 3] = [255, 224, 64];

/// Outlines any laid-out HUD items currently highlighted by UI Effect
/// (opcode 34, `0x44acff`). Returns immediately when no highlight is active so
/// ordinary frames and ratchet captures are untouched.
fn draw_ui_effect_highlights(
    items: &[sims_ui::hud::Item],
    highlighted_buttons: &BTreeSet<i16>,
    highlighted_portraits: &BTreeSet<u32>,
    highlight_relationship: bool,
    canvas: &mut Canvas,
) {
    if highlighted_buttons.is_empty() && highlighted_portraits.is_empty() && !highlight_relationship
    {
        return;
    }
    let mut relationship_drawn = false;
    for item in items {
        let matched = match &item.kind {
            sims_ui::hud::Kind::Button { id, .. }
            | sims_ui::hud::Kind::Toggle { id, .. }
            | sims_ui::hud::Kind::Cell { id, .. } => {
                i16::try_from(*id).is_ok_and(|k| highlighted_buttons.contains(&k))
            }
            sims_ui::hud::Kind::Pause { .. } => i16::try_from(sims_ui::hud::id::PAUSE)
                .is_ok_and(|k| highlighted_buttons.contains(&k)),
            sims_ui::hud::Kind::Portrait { guid, .. } => highlighted_portraits.contains(guid),
            sims_ui::hud::Kind::Relation { .. }
                if highlight_relationship && !relationship_drawn =>
            {
                relationship_drawn = true;
                true
            }
            _ => false,
        };
        if matched {
            draw_highlight_rect(canvas, item.rect, UI_EFFECT_HIGHLIGHT_RGB);
        }
    }
}

/// Draws a 2-pixel rectangular outline around `rect` clipped to `canvas`.
fn draw_highlight_rect(canvas: &mut Canvas, rect: sims_ui::frame::Rect, rgb: [u8; 3]) {
    if rect.width <= 0 || rect.height <= 0 || canvas.width == 0 || canvas.height == 0 {
        return;
    }
    let cw = canvas.width as i32;
    let ch = canvas.height as i32;
    let x0 = rect.x;
    let y0 = rect.y;
    let x1 = rect.x.saturating_add(rect.width).saturating_sub(1);
    let y1 = rect.y.saturating_add(rect.height).saturating_sub(1);
    let packed = sims_render::pack_rgb(rgb);
    let mut put = |x: i32, y: i32| {
        if x >= 0 && x < cw && y >= 0 && y < ch {
            let idx = y as usize * canvas.width + x as usize;
            if let Some(px) = canvas.color.get_mut(idx) {
                *px = packed;
            }
        }
    };
    for inset in 0..2 {
        let lx = x0.saturating_add(inset);
        let rx = x1.saturating_sub(inset);
        let ty = y0.saturating_add(inset);
        let by = y1.saturating_sub(inset);
        if lx > rx || ty > by {
            break;
        }
        for x in lx.max(0)..=rx.min(cw - 1) {
            put(x, ty);
            put(x, by);
        }
        for y in ty.max(0)..=by.min(ch - 1) {
            put(lx, y);
            put(rx, y);
        }
    }
}

/// Adds `items` to the requests the host has yet to take, keeping at
/// most `kept`: the oldest go first.
fn keep<T>(queue: &mut VecDeque<T>, items: Vec<T>, kept: usize) {
    queue.extend(items);
    let over = queue.len().saturating_sub(kept);
    queue.drain(..over);
}

/// The lines for the caught errors of the step that runs the lot's tick
/// `tick` ([`Lot::ticks`]), in the order they happened: the frame drawn
/// before the lot's first tick, the objects' errors in the tick, then the
/// frame drawn after it. A frame's line says which of the two it was,
/// and not the tick: an object's line starts with the clock's tick count,
/// which is another count, and the error log takes the same failure at
/// later ticks for a repeat ([`Errors::push`]).
fn in_order(tick: u32, errors: &[ScriptError], frames: &[SkippedFrame]) -> Vec<String> {
    let frame = |frame: &SkippedFrame| match frame.after {
        0 => format!(
            "the frame before the first tick was skipped: {}",
            frame.error
        ),
        _ => format!("the frame after the tick was skipped: {}", frame.error),
    };
    let before = frames.iter().filter(|frame| frame.after < tick);
    let after = frames.iter().filter(|frame| frame.after >= tick);
    before
        .map(frame)
        .chain(errors.iter().map(describe))
        .chain(after.map(frame))
        .collect()
}

/// One line for an error play mode caught: when, what failed, and what
/// was done about it.
fn describe(error: &ScriptError) -> String {
    let place = if error.in_thread {
        ""
    } else {
        " outside its thread"
    };
    let then = error
        .then
        .as_ref()
        .map(|then| format!("; then, outside its thread, {then}"))
        .unwrap_or_default();
    let recovery = match &error.recovery {
        Recovery::Reset => "it was reset".to_string(),
        Recovery::Restarted(fault) => {
            format!("its reset failed ({fault}), so its thread restarted at Main")
        }
        Recovery::Parked(None) => "it failed again after a reset, so it stops".to_string(),
        Recovery::Parked(Some(fault)) => {
            format!("restarting its thread failed ({fault}), so it stops")
        }
    };
    format!(
        "tick {}, object {}{place}: {}{then}; {recovery}",
        error.tick, error.object, error.fault
    )
}

/// The Goths' house in an install at `root`, the house the boot goal
/// checks against the original.
pub fn goth_house(root: &Path) -> PathBuf {
    root.join("UserData/Houses/House05.iff")
}

/// The fine `[x, y, level]` position `0x434ced` centres on: for a
/// multi-tile object (`0x4071a3`), starts at the lead tile (`parent`, or
/// `id` itself when `parent == 0`), walks `world.next_parts`, and averages
/// the parts' `x` and `y` coordinates by signed integer division toward
/// zero while keeping `object.position[2]` as the level.
fn centre_position(world: &sims_sim::World, id: u16) -> Option<[i32; 3]> {
    let object = world.objects.get(&id)?;
    let level = object.position[2];
    let Some(part) = &object.multipart else {
        return Some(object.position);
    };
    let lead = u16::try_from(part.parent)
        .ok()
        .filter(|&p| p != 0 && world.objects.contains_key(&p))
        .unwrap_or(id);
    let mut sum_x = 0i64;
    let mut sum_y = 0i64;
    let mut count = 0i64;
    let mut visited = Vec::new();
    let mut current = Some(lead);
    while let Some(part_id) = current {
        if visited.contains(&part_id) {
            break;
        }
        let Some(tile) = world.objects.get(&part_id) else {
            break;
        };
        visited.push(part_id);
        sum_x += i64::from(tile.position[0]);
        sum_y += i64::from(tile.position[1]);
        count += 1;
        current = world.next_parts.get(&part_id).copied();
    }
    if count == 0 {
        return Some(object.position);
    }
    Some([(sum_x / count) as i32, (sum_y / count) as i32, level])
}

/// The screen point of a fine lot position (`0x4a9dc7` → `0x4a9c50`),
/// narrowing rotated tile coordinates to signed bytes (`0x4a9c5c..0x4a9c71`)
/// while preserving the low 4-bit sub-tile offset, worked out from the lot's
/// own corner and then moved by the origin.
fn lot_point(camera: &Camera, lot: &sims_render::Lot, position: [i32; 3]) -> [i32; 2] {
    let rotated = sound::rotate_object_fine(camera, [position[0], position[1]]);
    let tile = [
        i32::from((rotated[0] >> 4) as i8),
        i32::from((rotated[1] >> 4) as i8),
    ];
    let narrowed = [
        (tile[0] << 4) | (rotated[0] & 15),
        (tile[1] << 4) | (rotated[1] & 15),
    ];
    let corners = sound::tile_corners(lot, camera, tile);
    let at = sound::fine_point_with_heights(
        &Camera {
            origin: [0, 0],
            ..*camera
        },
        narrowed,
        position[2],
        corners,
    );
    [
        at[0].saturating_add(camera.origin[0]),
        at[1].saturating_add(camera.origin[1]),
    ]
}

/// The house view's camera origin scroll limits `([min_x, max_x], [min_y, max_y])`
/// for `lot`, `camera`, and viewport `view_size` (`0x4a9fa9`, using the
/// diagonal height extrema `0x4b0615` and rotated lot bounds `0x4791a6`).
pub(crate) fn scroll_limits(
    lot: &sims_render::Lot,
    camera: &Camera,
    view_size: [i32; 2],
) -> ([i32; 2], [i32; 2]) {
    let size = lot.size as i32;
    let zoom = camera.zoom.level();
    let tile_w = camera.zoom.tile_width();
    let half_w = tile_w / 2;
    let tile_h = camera.zoom.tile_height();
    let half_h = tile_h / 2;
    let floor_h = camera.zoom.floor_height();
    let [view_w, view_h] = view_size;

    let inner_tiles = (size - 2).max(0);
    let span_x = inner_tiles * tile_w;
    let (mut min_x, mut max_x) = if view_w < span_x {
        ((span_x / -2 - half_w) + view_w, span_x / 2 + tile_w)
    } else {
        (view_w / 2, view_w / 2)
    };

    // Diagonal height extrema across the active lot (0x4b0615): indexed by
    // rotated `rx + ry` in `0..128`.
    let b = sound::lot_bounds(lot, camera);
    let (min_rx, min_ry) = (b[0], b[2]);
    let mut max_h = [0u8; 128];
    let mut min_h = [0x40u8; 128];
    for y in 0..size {
        for x in 0..size {
            let rx = x + min_rx;
            let ry = y + min_ry;
            let diag = (rx + ry) as usize;
            if diag < 128 {
                for h in sound::raw_tile_corners(lot, camera, [rx, ry]) {
                    max_h[diag] = max_h[diag].max(h);
                    min_h[diag] = min_h[diag].min(h);
                }
            }
        }
    }

    let mut top_adjust = 0i32;
    if size > 2 {
        let (mut rx, mut ry) = (1 + min_rx, 1 + min_ry);
        for step in 1..(size - 1) {
            let diag = (rx + ry) as usize;
            let val = (i32::from(max_h[diag]) << (zoom + 1)) - half_h * step;
            top_adjust = top_adjust.max(val);
            rx += 1;
            ry += 1;
        }
    }
    let mut max_y = top_adjust + floor_h * 2;

    let mut min_y = (view_h - tile_h * 2) - inner_tiles * tile_h;
    if size > 2 {
        let mut bottom_adjust = i32::MAX;
        let (mut rx, mut ry) = (size - 2 + min_rx, size - 2 + min_ry);
        for step in 1..(size - 1) {
            let diag = (rx + ry) as usize;
            let val = half_h * step - (i32::from(min_h[diag]) << (zoom + 1));
            bottom_adjust = bottom_adjust.min(val);
            rx -= 1;
            ry -= 1;
        }
        min_y -= bottom_adjust;
    }

    let shift_x = (min_rx - min_ry) * half_w;
    let shift_y = (min_ry + min_rx) * half_h;
    min_x -= shift_x;
    max_x -= shift_x;
    min_y -= shift_y;
    max_y -= shift_y;
    ([min_x, max_x], [min_y, max_y])
}

/// How far above its feet a centred person is (0x434d64 to 0x434da7):
/// 2 √6 (single precision, 0x434df6) times 2.0 (1/3 times 6, single)
/// times 2 to the power of the zoom plus one, cut toward zero, halved.
fn person_rise(zoom: u32) -> i32 {
    let rise = (2.0 * 6f64.sqrt()) as f32 * 2.0 * (1u32 << (zoom + 1)) as f32;
    rise as i32 / 2
}

/// Syncs the panel's speed and restarts the time owed when a tick changes
/// the simulation's speed or return speed (such as Special Effect resetting
/// to normal, `0x4f684c` → `0x46d697`, or `World::idle_speed` clearing
/// global 18 when someone wakes during manual stepping, `0x46d7db` →
/// `0x46d697`). Returns whether either changed.
fn sync_speed_after_step(
    pace: &mut Pace,
    speed_before: (i16, i16),
    world: &sims_sim::World,
) -> bool {
    if (world.speed(), world.return_speed()) == speed_before {
        return false;
    }
    if let Some(speed) = pace::shown(world) {
        pace.speed = speed;
    }
    pace.restart();
    true
}

/// Rotates a fine lot/view coordinate about `1023` (`0x47bc1f` with `0x400`).
fn rotate_fine_1023([x, y]: [i32; 2], turns: u8) -> [i32; 2] {
    let m: i32 = 1023;
    match turns & 3 {
        1 => [m.wrapping_sub(y), x],
        2 => [m.wrapping_sub(x), m.wrapping_sub(y)],
        3 => [y, m.wrapping_sub(x)],
        _ => [x, y],
    }
}

/// Applies `Move::ZoomIn`, `Move::ZoomOut`, or `Move::Rotate` using the
/// original's terrain-aware screen-centre anchor on the displayed floor
/// (`0x4a7c1f..0x4a7d2e` for zoom, `0x4a7e89..0x4a7f94` for rotation):
///
/// 1. Picks the view-rotated fine ground point at the active house viewport's
///    midpoint (`0x4a7b58`) using the native origin column (`origin[0] + 1`).
/// 2. Cuts it to a signed-byte tile (`0x433067`) and checks the rotated lot's
///    interior bounds (`0x434cb7`); if outside, falls back to the tile
///    `[(size / 4) + min_x, (size / 4) + min_y]` converted to its fine centre
///    `((t << 4) | 8)` (`0x4a7dec`, `0x434c98`).
/// 3. Updates zoom (or un-rotates by `(-old) & 3`, updates rotation, and
///    re-rotates by `new` via `0x47bc1f` about `1023`).
/// 4. Recenters via `0x4a7996` (`on_lot` check `0x4a7af0`, terrain-aware
///    projection on `camera.level` via `fine_point_with_heights`, clamping to
///    `scroll_limits` and `(x + 1) & !3` alignment at `0x4a81cc`).
fn native_zoom_or_rotate(
    lot: &sims_render::Lot,
    camera: &mut Camera,
    viewport: [i32; 2],
    command: Move,
) -> bool {
    let before = *camera;
    let next_zoom = match command {
        Move::ZoomIn => {
            let next = camera.zoom.zoom_in();
            if next == camera.zoom {
                return false;
            }
            Some(next)
        }
        Move::ZoomOut => {
            let next = camera.zoom.zoom_out();
            if next == camera.zoom {
                return false;
            }
            Some(next)
        }
        Move::Rotate(_) => None,
        _ => return false,
    };

    let mut native = *camera;
    native.origin[0] = native.origin[0].saturating_add(pick::ORIGIN_COLUMN);
    let mut fine = match sims_render::screen_centre_pick(lot, &native, viewport) {
        Some(fine) => fine,
        None => return false,
    };
    let [tx, ty] = [
        i32::from((fine[0] >> 4) as i8),
        i32::from((fine[1] >> 4) as i8),
    ];
    let b = sound::lot_bounds(lot, &native);
    let inside = tx > b[0] && tx < b[1] && ty > b[2] && ty < b[3];
    if !inside {
        let quarter = (lot.size as i32 / 4) as i8;
        let fx = i32::from(quarter.wrapping_add(b[0] as i8));
        let fy = i32::from(quarter.wrapping_add(b[2] as i8));
        fine = [(fx << 4) | 8, (fy << 4) | 8];
    }

    if let Some(zoom) = next_zoom {
        native.zoom = zoom;
    } else if let Move::Rotate(clockwise) = command {
        let unrotated = rotate_fine_1023(fine, (4 - (native.rotation & 3)) & 3);
        native.rotation = (native.rotation + if clockwise { 1 } else { 3 }) & 3;
        fine = rotate_fine_1023(unrotated, native.rotation);
    }

    // 0x4a7af0 narrows the rotated tile coordinates to signed bytes (`movsx al/dl`)
    // before checking lot bounds, and 0x4a9dc7 → 0x4a9c50 projects the narrowed tile
    // plus the low 4-bit fractional offset.
    let b = sound::lot_bounds(lot, &native);
    let [rx, ry] = [
        i32::from((fine[0] >> 4) as i8),
        i32::from((fine[1] >> 4) as i8),
    ];
    if rx >= b[0] && rx <= b[1] && ry >= b[2] && ry <= b[3] {
        let narrowed_fine = [(rx << 4) | (fine[0] & 15), (ry << 4) | (fine[1] & 15)];
        let corners = sound::tile_corners(lot, &native, [rx, ry]);
        // 0x4a79fd projects using the current native origin and sign-extends the
        // 16-bit screen coordinates before `0x4a7a23..0x4a7a47` computes
        // `current_origin + (middle - screen_point)`.
        let at = sound::fine_point_with_heights(&native, narrowed_fine, native.level, corners);
        let middle = [viewport[0] / 2, viewport[1] / 2];
        let target = [
            native.origin[0].saturating_add(middle[0].saturating_sub(at[0])),
            native.origin[1].saturating_add(middle[1].saturating_sub(at[1])),
        ];
        let ([min_x, max_x], [min_y, max_y]) = scroll_limits(lot, &native, viewport);
        let native_x = target[0].max(min_x).min(max_x).wrapping_add(1) & !3;
        let clamped_y = target[1].max(min_y).min(max_y);
        native.origin = [native_x, clamped_y];
    }

    camera.zoom = native.zoom;
    camera.rotation = native.rotation;
    camera.origin = [
        native.origin[0].saturating_sub(pick::ORIGIN_COLUMN),
        native.origin[1],
    ];
    if camera.zoom != before.zoom {
        let built = before.focus.masks_built_at.unwrap_or(before.zoom);
        camera.focus.masks_built_at = (built != camera.zoom).then_some(built);
    }
    if camera.rotation != before.rotation {
        camera.focus.masks_built_at = None;
    }
    *camera != before
}

/// The active 3D house viewport size (`0x49ae5e`, `0x4f4b43`, `0x4d363b`,
/// `0x53b2eb` → `0x4b136b` → `0x49b0bf` → `0x5cf1b6`): full canvas when the
/// bottom strip is closed, and `height - STRIP` (`height - 150 + 50` from the
/// shared 150-pixel panel container at `0x4f4b43`) when the strip is open in
/// Live, Build, Buy, or Options mode.
fn viewport_for_controls(size: [i32; 2], controls: &sims_ui::hud::Controls) -> (u32, u32) {
    let [w, h] = size.map(|n| n.max(1));
    let strip = if controls.expanded {
        sims_ui::hud::STRIP
    } else {
        0
    };
    (w as u32, (h - strip).max(1) as u32)
}

/// Loads a pet's body for the pet picker's turntable. Pets' skeletons are
/// `dog_skeleton.cmx.bcf` and `kat_skeleton.cmx.bcf` (`ExpansionShared.far`
/// `Skeletons\`), an underscore where people's `<name>-skeleton.cmx.bcf`
/// has a hyphen, so [`sims_people::Body::load`] cannot find them; this
/// tries the hyphen first, then the underscore, and binds the outfit the
/// same way.
fn load_pet_body(
    assets: &sims_people::Assets,
    skeleton: &str,
    outfit: &sims_people::Outfit,
) -> Result<sims_people::Body, String> {
    if let Ok(body) = sims_people::Body::load(assets, skeleton, outfit) {
        return Ok(body);
    }
    use sims_people::appearance::{load_appearance, load_mesh, load_texture, texture_for};
    use sims_people::rig::bone;
    let file = format!("{skeleton}_skeleton.cmx.bcf");
    let catalog =
        sims_formats::avatar::read_bcf(&assets.read(&file)?).map_err(|e| format!("{file}: {e}"))?;
    let skeleton = catalog
        .skeletons
        .into_iter()
        .next()
        .ok_or_else(|| format!("{file} holds no skeleton"))?;
    let mut parts = Vec::new();
    for (name, textures) in &outfit.parts {
        let appearance = load_appearance(assets, name)?;
        for binding in &appearance.bindings {
            let at = bone(&skeleton, &binding.bone)
                .ok_or_else(|| format!("{name:?} binds the missing bone {:?}", binding.bone))?;
            let mesh = load_mesh(assets, &binding.mesh)?;
            let bones = mesh
                .bones
                .iter()
                .map(|b| bone(&skeleton, b).ok_or_else(|| format!("missing bone {b:?}")))
                .collect::<Result<Vec<_>, _>>()?;
            let texture_name = texture_for(&binding.mesh, textures)
                .or_else(|| (!mesh.texture.is_empty()).then(|| mesh.texture.clone()));
            let texture = match &texture_name {
                Some(t) => load_texture(assets, t)?,
                None => sims_people::texture::Texture::solid([200, 200, 200, 255]),
            };
            parts.push(sims_people::Part {
                appearance: name.clone(),
                bone: at,
                bones,
                mesh,
                texture_name,
                texture,
                censor: binding.censor,
            });
        }
    }
    Ok(sims_people::Body { skeleton, parts })
}

/// `person.data[31]`: the person's neighbor record ID (`PERSON_NEIGHBOR`).
const PERSON_NEIGHBOR_WORD: usize = 31;

/// Whether `record` may appear in `caller`'s Phonebook (see
/// [`Game::build_phonebook_snapshot`]).
fn phonebook_eligible(
    record: &sims_sim::neighbors::Neighbor,
    caller: i16,
    caller_family: i16,
    on_lot: &std::collections::BTreeSet<i16>,
) -> bool {
    let family = record
        .data
        .get(sims_sim::neighbors::FAMILY)
        .copied()
        .unwrap_or(0);
    let kind = record
        .data
        .get(sims_sim::neighbors::KIND)
        .copied()
        .unwrap_or(0);
    record.id > 0
        && record.id != caller
        && kind <= 1
        && family > 0
        && (caller_family <= 0 || family != caller_family)
        && !on_lot.contains(&record.id)
}

/// The suit category letter of `Dialog` kind 6 mode `0..=5` (`0x5097c9`):
/// everyday `b`, swimwear `s`, formal `f`, pajamas `l`, winter `w`, and
/// Superstar high fashion `h`.
fn wardrobe_mode_category(mode: i16) -> u8 {
    match mode {
        1 => b's',
        2 => b'f',
        3 => b'l',
        4 => b'w',
        5 => b'h',
        _ => b'b',
    }
}

/// Every `(suit, "suit,BODY=texture")` of category `first` that a person
/// of the given sex, age and skin tone can wear, in native wardrobe order:
/// the filter of `ImportWardrobe::pick` (`0x409663`) — a 9-byte suit prefix
/// `<first>NNN<sex|u><age>xxx` and textures `<prefix><skin>…` — with the
/// category letter in place of `b`.
fn category_outfits(
    wardrobe: &sims_content::ImportWardrobe,
    first: u8,
    female: bool,
    child: bool,
    skin: &str,
) -> Vec<(String, String)> {
    let Some(skin_bytes) = sims_formats::far::encode_name(&skin.to_ascii_lowercase()) else {
        return Vec::new();
    };
    let sex = if female { b'f' } else { b'm' };
    let age = if child { b'c' } else { b'a' };
    let mut out = Vec::new();
    for suit in wardrobe.suits() {
        if suit.len() < 9
            || !suit[0].eq_ignore_ascii_case(&first)
            || !(suit[4].eq_ignore_ascii_case(&sex) || suit[4].eq_ignore_ascii_case(&b'u'))
            || !suit[5].eq_ignore_ascii_case(&age)
        {
            continue;
        }
        let mut prefix = suit[..9].to_ascii_lowercase();
        prefix.extend_from_slice(&skin_bytes);
        for tex in wardrobe.textures() {
            if tex.len() >= prefix.len() && tex[..prefix.len()].eq_ignore_ascii_case(&prefix) {
                let suit_str: String = sims_formats::strings::decode(suit, 1)
                    .to_ascii_lowercase()
                    .chars()
                    .take(63)
                    .collect();
                let tex_str = sims_formats::strings::decode(tex, 1);
                let desc: String = format!("BODY={tex_str}").chars().take(63).collect();
                out.push((suit_str.clone(), format!("{suit_str},{desc}")));
            }
        }
    }
    out
}

/// A readable outfit name from a wardrobe row: the texture's name after its
/// first `_` (`"b002fafitmed_mom02"` -> `"Mom02"`, `"h551mafitlgt_TuxedoBasic"`
/// -> `"Tuxedo Basic"`), falling back to the suit's.
fn outfit_display_label(suit: &str, row: &str) -> String {
    let texture = row
        .split_once(',')
        .and_then(|(_, desc)| desc.split_once('='))
        .map(|(_, tex)| tex.trim())
        .unwrap_or("");
    let tail = |name: &str| name.split_once('_').map(|(_, t)| t.to_owned());
    let raw = tail(texture)
        .or_else(|| tail(suit))
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| suit.to_owned());
    humanize_identifier(&raw)
}

/// `"TuxedoBasic"` -> `"Tuxedo Basic"`, `"mom02"` -> `"Mom02"`,
/// `"red_dress"` -> `"Red Dress"`.
fn humanize_identifier(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 4);
    let mut prev: Option<char> = None;
    for c in raw.chars() {
        if c == '_' || c == '-' {
            if !out.ends_with(' ') && !out.is_empty() {
                out.push(' ');
            }
            prev = Some(' ');
            continue;
        }
        let boundary = prev.is_some_and(|p| p.is_ascii_lowercase() && c.is_ascii_uppercase());
        if boundary {
            out.push(' ');
        }
        let start = prev.is_none_or(|p| p == ' ');
        if start || boundary {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        prev = Some(c);
    }
    out.trim().to_owned()
}

/// A pet body row's breed (`"b000dog,BODY=B000dog_lab"` -> `"Labrador
/// Retriever"`), from the Unleashed texture names, title-casing any
/// unlisted one.
fn pet_breed_label(row: &str) -> String {
    let texture = row
        .split_once(',')
        .and_then(|(_, desc)| desc.split_once('='))
        .map(|(_, tex)| tex.trim())
        .unwrap_or(row);
    let key = texture
        .split_once('_')
        .map_or(texture, |(_, t)| t)
        .to_ascii_lowercase();
    let known = match key.as_str() {
        "lab" => "Labrador Retriever",
        "aussie" => "Australian Shepherd",
        "dalmation" | "dalmatian" => "Dalmatian",
        "cairn" => "Cairn Terrier",
        "poodle" => "Poodle",
        "pinkpoodle" => "Pink Poodle",
        "jack" => "Jack Russell Terrier",
        "springer" => "Springer Spaniel",
        "setter" => "Irish Setter",
        "greyhound" => "Greyhound",
        "beagle" => "Beagle",
        "rott" => "Rottweiler",
        "bulldog" => "Bulldog",
        "scottish" => "Scottish Terrier",
        "shep" => "German Shepherd",
        "chihuahua" => "Chihuahua",
        "tabby" => "Tabby",
        "tabbybrown" => "Brown Tabby",
        "silver" => "Silver Tabby",
        "calico" => "Calico",
        "calicoblk" => "Black Calico",
        "calicored" => "Red Calico",
        "persian" => "Persian",
        "bluemanx" => "Blue Manx",
        "ragdoll" => "Ragdoll",
        "siamese" => "Siamese",
        "tortoise" => "Tortoiseshell",
        "bombay" => "Bombay",
        "spotted" => "Spotted",
        "sphinx" => "Sphynx",
        "albino" => "Albino",
        _ => return humanize_identifier(&key),
    };
    known.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_never_stands_in_for_the_vm_request() {
        let request = SpecialPanelRequest::CallNeighbor { caller_neighbor: 7 };
        let shown = ShownSpecialPanel {
            tick: 0,
            request: request.clone(),
        };
        let mut panel = ActiveSpecialPanel {
            tick: 0,
            request,
            coop_dialog_id: None,
            responses: BTreeMap::new(),
            preview_targets: BTreeMap::new(),
            preview: true,
        };
        assert!(
            !panel.shows(&shown, None),
            "a tick-0 preview of the same request is replaced by the real one"
        );
        panel.preview = false;
        assert!(panel.shows(&shown, None), "the real panel stays as it is");
    }
    use sims_vm::{Fault, Location, Trap};

    fn fault(object: u16) -> Fault {
        Fault {
            object,
            at: Some(Location {
                tree: 0x1002,
                node: 7,
            }),
            trap: Trap::Unimplemented { opcode: 0x2a },
        }
    }

    #[test]
    fn a_centred_person_is_raised_by_half_its_zooms_height() {
        // trunc(2 sqrt 6 * 2.0 * 2^(zoom + 1)) / 2 for far, medium, near.
        assert_eq!([1, 2, 3].map(super::person_rise), [19, 39, 78]);
    }

    #[test]
    fn scroll_limits_match_0x4a9fa9_across_rotations_view_widths_and_terrain() {
        use sims_render::camera::{Walls, Zoom};
        let mut world = sims_sim::World::empty();
        world.size = 52;
        let full = sims_render::lot::FULL * sims_render::lot::FULL;
        for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
            world.arrays.insert(
                id,
                sims_formats::lot::Array {
                    width: sims_render::lot::FULL,
                    height: sims_render::lot::FULL,
                    stride,
                    encoding: 0,
                    data: vec![0; full * stride],
                },
            );
        }
        let flat = sims_render::Lot::from_world(&world).unwrap();
        let cam = |zoom: Zoom, rotation: u8| Camera {
            zoom,
            rotation,
            level: 1,
            origin: [0, 0],
            walls: Walls::Up,
            focus: sims_render::Focus::default(),
        };

        // 52x52 flat lot at far zoom, 800x500 house view:
        // span_x = 50 * 32 = 1600, min_x = (-800 - 16) + 800 = -16,
        // max_x = 800 + 32 = 832;
        // max_y = 0 + 58 * 2 = 116;
        // min_y = (500 - 32) - 50 * 16 - 8 = -340.
        assert_eq!(
            scroll_limits(&flat, &cam(Zoom::Far, 0), [800, 500]),
            ([-16, 832], [-340, 116])
        );
        // Rotation 1 on a 52x52 lot shifts min_rx = 12, min_ry = 0:
        // shift_x = 12 * 16 = 192, shift_y = 12 * 8 = 96.
        assert_eq!(
            scroll_limits(&flat, &cam(Zoom::Far, 1), [800, 500]),
            ([-16 - 192, 832 - 192], [-340 - 96, 116 - 96])
        );
        // Rotation 2 shifts min_rx = 12, min_ry = 12:
        // shift_x = 0, shift_y = 24 * 8 = 192.
        assert_eq!(
            scroll_limits(&flat, &cam(Zoom::Far, 2), [800, 500]),
            ([-16, 832], [-340 - 192, 116 - 192])
        );
        // Viewport wider than span_x (1600) centres x at view_w / 2.
        let ([min_x, max_x], _) = scroll_limits(&flat, &cam(Zoom::Far, 0), [2000, 500]);
        assert_eq!((min_x, max_x), (1000, 1000));

        // Raising the top corner tile (1, 1) to height 10 raises max_y by
        // max(0, (10 << 2) - 8 * 1) = 32 -> 116 + 32 = 148.
        let cell = |x: usize, y: usize| (x * sims_render::lot::FULL + y) * 4;
        world.arrays.get_mut(&0).unwrap().data[cell(1, 1)..cell(1, 1) + 4]
            .copy_from_slice(&[10, 10, 10, 10]);
        let raised = sims_render::Lot::from_world(&world).unwrap();
        let (_, [min_y, max_y]) = scroll_limits(&raised, &cam(Zoom::Far, 0), [800, 500]);
        assert_eq!((min_y, max_y), (-340, 148));
    }

    #[test]
    fn centre_position_averages_linked_multipart_tiles() {
        let mut world = sims_sim::World::empty();
        let make = |id: u16, position: [i32; 3], parent: Option<i16>| sims_sim::Object {
            id,
            guid: 0,
            footprint: [0; 4],
            position,
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
            multipart: parent.map(|p| sims_formats::saved_objects::Multipart {
                offset: [0; 3],
                group: [0; 3],
                parent: p,
            }),
            portal: None,
            person: None,
            registry_word: None,
            last_result: false,
            removal_skips_resets: false,
            selected: false,
        };
        world.objects.insert(1, make(1, [100, 200, 1], None));
        world.objects.insert(10, make(10, [384, 416, 1], Some(0)));
        world.objects.insert(11, make(11, [400, 416, 1], Some(10)));
        world.objects.insert(12, make(12, [416, 449, 2], Some(10)));
        world.next_parts.insert(10, 11);
        world.next_parts.insert(11, 12);
        assert_eq!(centre_position(&world, 1), Some([100, 200, 1]));
        // (384+400+416)/3 = 400, (416+416+449)/3 = 1281/3 = 427; level comes
        // from the queried object (0x434ced reads param_1 + 0x110).
        assert_eq!(centre_position(&world, 10), Some([400, 427, 1]));
        assert_eq!(centre_position(&world, 11), Some([400, 427, 1]));
        assert_eq!(centre_position(&world, 12), Some([400, 427, 2]));
    }
    #[test]
    fn an_error_says_what_failed_and_what_was_done() {
        let mut error = ScriptError {
            tick: 900,
            object: 12,
            fault: fault(12),
            in_thread: true,
            then: None,
            recovery: Recovery::Reset,
        };
        let line = describe(&error);
        assert!(line.starts_with("tick 900, object 12: object 12, tree 0x1002 node 7"));
        assert!(line.ends_with("; it was reset"), "{line}");
        // A caught thread fault, then one in the rest of the tick.
        error.then = Some(fault(13));
        error.recovery = Recovery::Restarted(fault(12));
        let line = describe(&error);
        assert!(
            line.starts_with("tick 900, object 12: object 12,"),
            "{line}"
        );
        assert!(line.contains("; then, outside its thread, object 13"));
        assert!(line.ends_with("its thread restarted at Main"), "{line}");
        error.then = None;
        error.in_thread = false;
        error.recovery = Recovery::Parked(None);
        let line = describe(&error);
        assert!(line.starts_with("tick 900, object 12 outside its thread:"));
        assert!(line.ends_with("it failed again after a reset, so it stops"));
        error.recovery = Recovery::Parked(Some(fault(12)));
        let line = describe(&error);
        assert!(
            line.contains("restarting its thread failed (object 12,"),
            "{line}"
        );
        assert!(line.ends_with("so it stops"));
    }

    #[test]
    fn an_updates_errors_are_logged_in_the_order_they_happened() {
        let error = ScriptError {
            tick: 1,
            object: 12,
            fault: fault(12),
            in_thread: true,
            then: None,
            recovery: Recovery::Reset,
        };
        let frames = |skipped: &[(u32, &str)]| -> Vec<SkippedFrame> {
            skipped
                .iter()
                .map(|&(after, error)| SkippedFrame {
                    after,
                    error: error.to_string(),
                })
                .collect()
        };
        let starts = |lines: Vec<String>| -> Vec<String> {
            lines.into_iter().map(|l| l[..13].to_string()).collect()
        };
        let errors = std::slice::from_ref(&error);
        // The first step: the frame before the tick, the tick, the frame
        // after it.
        let first = in_order(1, errors, &frames(&[(1, "a"), (0, "b")]));
        assert_eq!(
            starts(first.clone()),
            ["the frame bef", "tick 1, objec", "the frame aft"]
        );
        assert_eq!(first[0], "the frame before the first tick was skipped: b");
        assert_eq!(first[2], "the frame after the tick was skipped: a");
        // A single frame on the first step says which one it was.
        assert_eq!(
            starts(in_order(1, errors, &frames(&[(1, "a")]))),
            ["tick 1, objec", "the frame aft"]
        );
        assert_eq!(
            starts(in_order(1, errors, &frames(&[(0, "b")]))),
            ["the frame bef", "tick 1, objec"]
        );
        // A step that failed hands back the frame before its tick only.
        assert_eq!(
            in_order(1, &[], &frames(&[(0, "b")])),
            ["the frame before the first tick was skipped: b"]
        );
        // Later steps draw only after the tick.
        assert_eq!(
            in_order(34, &[], &frames(&[(34, "a")])),
            ["the frame after the tick was skipped: a"]
        );
        assert!(in_order(2, &[], &[]).is_empty());
    }

    #[test]
    fn errors_show_for_a_while_and_the_log_is_bounded() {
        let mut errors = Errors::default();
        assert_eq!(errors.recent(0), None);
        errors.push(100, "first".into());
        assert_eq!(errors.recent(100).as_deref(), Some("Error caught: first"));
        errors.push(110, "second".into());
        assert_eq!(
            errors.recent(110 + SHOWN_TICKS - 1).as_deref(),
            Some("Error caught (2 so far): second")
        );
        assert_eq!(errors.recent(110 + SHOWN_TICKS), None);
        // The same error at another tick is counted, not listed or shown
        // again.
        errors.push(300, "tick 7, object 2: x".into());
        errors.push(310 + SHOWN_TICKS, "tick 8, object 2: x".into());
        assert_eq!(errors.lines.len(), 3);
        assert_eq!(errors.lines[2], "tick 7, object 2: x");
        assert_eq!(errors.count, 4);
        assert_eq!(errors.recent(310 + SHOWN_TICKS), None);
        for n in 0..LOGGED * 2 {
            errors.push(200, n.to_string());
        }
        assert_eq!(errors.lines.len(), LOGGED + 1);
        assert_eq!(errors.count, LOGGED as u64 * 2 + 4);
        assert_eq!(
            errors.lines.last().unwrap(),
            "further errors are counted, not listed"
        );
        // Once SEEN different errors are remembered, a new one is only
        // counted, so one recurring every tick does not hold the line open.
        for n in errors.seen.len()..SEEN {
            errors.push(400, format!("tick 9, filler {n}"));
        }
        let count = errors.count;
        errors.push(400 + SHOWN_TICKS, "tick 10, object 3: new".into());
        assert_eq!(errors.count, count + 1);
        assert_eq!(errors.recent(400 + SHOWN_TICKS), None);
        assert_eq!(errors.lines.len(), LOGGED + 1);
    }

    #[test]
    fn the_sound_queue_keeps_the_newest_and_hands_them_over_once() {
        let play = |object: i16| SoundEvent::Play {
            object,
            name: format!("sound {object}"),
        };
        let mut queue = VecDeque::new();
        keep(&mut queue, (0..3).map(play).collect(), 4);
        keep(&mut queue, (3..6).map(play).collect(), 4);
        // The two oldest are dropped; the rest stay in order.
        assert_eq!(
            Vec::from(queue.clone()),
            (2..6).map(play).collect::<Vec<_>>()
        );
        keep(&mut queue, Vec::new(), 4);
        assert_eq!(queue.len(), 4);
        // Taking them empties the queue, as `Game::take_sounds` does.
        let taken: Vec<_> = queue.drain(..).collect();
        assert_eq!(taken.len(), 4);
        assert!(queue.is_empty());
    }

    #[test]
    fn a_mid_tick_speed_reset_updates_the_panel_and_drops_owed_time() {
        use sims_sim::speed::{FAST, IDLE, NORMAL};

        let mut world = sims_sim::World::empty();
        world.globals[18] = -1;
        world.set_speed(i32::from(FAST));

        let mut pace = Pace::default();
        pace.set_speed(Speed::Fast);
        // Unchanged speed keeps both the panel selection and accrued credit.
        assert_eq!(pace.due(13_000, FAST), 0);
        assert!(!sync_speed_after_step(&mut pace, (FAST, -1), &world));
        assert_eq!(pace.speed, Speed::Fast);
        assert_eq!(pace.due(1_000, FAST), 1);

        // Accrue 13 ms at Fast, then drop speed to Normal inside a step:
        // the panel switches to Normal and the 13 ms credit is dropped so
        // the next Normal tick waits a full 42 ms.
        assert_eq!(pace.due(13_000, FAST), 0);
        world.set_speed(i32::from(NORMAL));
        assert!(sync_speed_after_step(&mut pace, (FAST, -1), &world));
        assert_eq!(pace.speed, Speed::Normal);
        assert_eq!(pace.due(29_000, NORMAL), 0);
        assert_eq!(pace.due(12_999, NORMAL), 0);
        assert_eq!(pace.due(1, NORMAL), 1);

        // Idle speed (1000, returning to Fast) dropping to Normal (760, 760)
        // also switches the panel to Normal and drops accrued credit.
        pace.set_speed(Speed::Fast);
        world.globals[16] = IDLE;
        world.globals[18] = FAST;
        assert_eq!(pace.due(500, IDLE), 0);
        world.set_speed(i32::from(NORMAL));
        assert_eq!((world.speed(), world.return_speed()), (NORMAL, NORMAL));
        assert!(sync_speed_after_step(&mut pace, (IDLE, FAST), &world));
        assert_eq!(pace.speed, Speed::Normal);
        assert_eq!(pace.due(41_999, NORMAL), 0);
        assert_eq!(pace.due(1, NORMAL), 1);

        // Wake-up clearing global 18 from (760, 760) to (760, -1) inside a
        // manual step still calls 0x46d697 and must drop accrued credit.
        assert_eq!(pace.due(41_000, NORMAL), 0);
        world.globals[18] = -1;
        assert!(sync_speed_after_step(&mut pace, (NORMAL, NORMAL), &world));
        assert_eq!(pace.speed, Speed::Normal);
        assert_eq!(pace.due(1_000, NORMAL), 0);
        assert_eq!(pace.due(40_999, NORMAL), 0);
        assert_eq!(pace.due(1, NORMAL), 1);
    }

    #[test]
    fn house_viewport_size_follows_the_control_panel_strip() {
        use sims_ui::command::Mode;
        use sims_ui::hud::Controls;

        let mut controls = Controls::default();
        assert!(!controls.expanded);
        assert_eq!(viewport_for_controls([800, 600], &controls), (800, 600));

        controls.expanded = true;
        controls.mode = Mode::Live;
        assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

        controls.mode = Mode::Build;
        assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

        controls.mode = Mode::Options;
        assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

        controls.mode = Mode::Buy;
        assert_eq!(viewport_for_controls([800, 600], &controls), (800, 500));

        // Short windows clamp to at least 1 pixel rather than wrapping.
        assert_eq!(viewport_for_controls([800, 80], &controls), (800, 1));
    }

    #[test]
    fn screen_centre_pick_uses_active_viewport_and_native_origin_column() {
        use sims_render::camera::{Walls, Zoom};
        use sims_ui::hud::Controls;

        let mut world = sims_sim::World::empty();
        world.size = 64;
        let full = sims_render::lot::FULL * sims_render::lot::FULL;
        for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
            world.arrays.insert(
                id,
                sims_formats::lot::Array {
                    width: sims_render::lot::FULL,
                    height: sims_render::lot::FULL,
                    stride,
                    encoding: 0,
                    data: vec![0; full * stride],
                },
            );
        }
        let lot = sims_render::Lot::from_world(&world).unwrap();
        let pick_tile = |game_origin: [i32; 2], expanded: bool| {
            let controls = Controls {
                expanded,
                ..Controls::default()
            };
            let (w, h) = viewport_for_controls([800, 600], &controls);
            let camera = Camera {
                origin: [
                    game_origin[0].saturating_add(pick::ORIGIN_COLUMN),
                    game_origin[1],
                ],
                zoom: Zoom::Far,
                rotation: 0,
                level: 1,
                walls: Walls::Cutaway,
                focus: sims_render::Focus::default(),
            };
            sims_render::screen_centre_pick(&lot, &camera, [w as i32, h as i32])
                .map(|[x, y]| [(x >> 4) as i8, (y >> 4) as i8])
        };
        // Native origin [400, 0] (game origin [399, 0]): expanded 800x500 picks
        // [15, 15], collapsed 800x600 picks [18, 18].
        assert_eq!(pick_tile([399, 0], true), Some([15, 15]));
        assert_eq!(pick_tile([399, 0], false), Some([18, 18]));
        // Expanded 800x500: game origin [398, -6] (native [399, -6]) picks
        // [16, 15], while game origin [399, -6] (native [400, -6]) picks [16, 16].
        assert_eq!(pick_tile([398, -6], true), Some([16, 15]));
        assert_eq!(pick_tile([399, -6], true), Some([16, 16]));

        // Zooming from Far to Medium with the panel expanded (800x500 active
        // viewport) uses native 0x4a7b58 + 0x434cb7 + 0x4a7996 anchoring:
        // - from native [400, -6] (inside lot): stays on [16, 16];
        // - from native [400, 240] (tile [0, 0], outside interior): falls back
        //   to quarter-lot tile [16, 16] (0x4a7c56..0x4a7c96) instead of [32, 32].
        let controls = Controls {
            expanded: true,
            ..Controls::default()
        };
        let (w, h) = viewport_for_controls([800, 600], &controls);
        let viewport = [w as i32, h as i32];
        let pick_unrotated = |camera: Camera| {
            let mut native = camera;
            native.origin[0] = native.origin[0].saturating_add(pick::ORIGIN_COLUMN);
            let tile = sims_render::screen_centre_pick(&lot, &native, viewport)
                .map(|[x, y]| [i32::from((x >> 4) as i8), i32::from((y >> 4) as i8)])?;
            Some(native.unrotate_tile(tile))
        };

        for start_origin in [[399, -6], [399, 240]] {
            let mut cam = Camera {
                origin: start_origin,
                zoom: Zoom::Far,
                rotation: 0,
                level: 1,
                walls: Walls::Cutaway,
                focus: sims_render::Focus::default(),
            };
            assert!(native_zoom_or_rotate(
                &lot,
                &mut cam,
                viewport,
                Move::ZoomIn
            ));
            assert_eq!(
                pick_unrotated(cam),
                Some([16, 16]),
                "zoom from game origin {start_origin:?}"
            );
        }

        // On floor 2 at native origin [400, -6] (game [399, -6]), clockwise
        // rotation anchors on the displayed floor's pick ([19, 19]) and keeps
        // the unrotated search start at [19, 19].
        let mut floor2 = Camera {
            origin: [399, -6],
            zoom: Zoom::Far,
            rotation: 0,
            level: 2,
            walls: Walls::Cutaway,
            focus: sims_render::Focus::default(),
        };
        assert_eq!(pick_unrotated(floor2), Some([19, 19]));
        assert!(native_zoom_or_rotate(
            &lot,
            &mut floor2,
            viewport,
            Move::Rotate(true)
        ));
        assert_eq!(pick_unrotated(floor2), Some([19, 19]));

        // Off-lot wrapped pick: on a flat 64x64 lot at 800x600 (collapsed),
        // Far -> Medium from native origin [381, -3997] (game [380, -3997])
        // wraps tile [269, 268] to signed bytes [13, 12] at 0x4a7af0 and
        // recenters to native origin [392, -117] (game [391, -117]).
        let mut wrapped = Camera {
            origin: [380, -3997],
            zoom: Zoom::Far,
            rotation: 0,
            level: 1,
            walls: Walls::Cutaway,
            focus: sims_render::Focus::default(),
        };
        assert!(native_zoom_or_rotate(
            &lot,
            &mut wrapped,
            [800, 600],
            Move::ZoomIn
        ));
        assert_eq!(
            [wrapped.origin[0] + pick::ORIGIN_COLUMN, wrapped.origin[1]],
            [392, -117]
        );

        // 16-bit screen wrap during zoom recentring (0x4a79fd..0x4a7a47): from
        // game origin [380, 61539] (whose low 16 bits wrap to -3997), Far ->
        // Medium projects with the current origin and clamps to [391, 232].
        let mut wrapped_high = Camera {
            origin: [380, 61539],
            zoom: Zoom::Far,
            rotation: 0,
            level: 1,
            walls: Walls::Cutaway,
            focus: sims_render::Focus::default(),
        };
        assert!(native_zoom_or_rotate(
            &lot,
            &mut wrapped_high,
            [800, 600],
            Move::ZoomIn
        ));
        assert_eq!(wrapped_high.origin, [391, 232]);

        // Wrapped lot position projection in `lot_point`: [424, 392, 1] and
        // [4520, 392, 1] (differing by 256 tiles = 4096 fine units) project
        // identically after signed-byte tile narrowing (0x4a9c5c..0x4a9c71).
        let near = Camera {
            origin: [0, 0],
            zoom: Zoom::Near,
            rotation: 0,
            level: 1,
            walls: Walls::Cutaway,
            focus: sims_render::Focus::default(),
        };
        assert_eq!(
            lot_point(&near, &lot, [424, 392, 1]),
            lot_point(&near, &lot, [4520, 392, 1])
        );
    }

    #[test]
    fn ui_effect_highlights_outline_buttons_portraits_and_first_relationship_control() {
        let relation = |guid: u32, id: i16| sims_ui::household::Relation {
            guid,
            id,
            daily: 10,
            lifetime: 5,
            friends: false,
            pink_heart: false,
            red_heart: false,
            star: false,
            family: false,
        };
        let items = vec![
            sims_ui::hud::Item {
                rect: sims_ui::frame::Rect::new(2, 2, 10, 10),
                kind: sims_ui::hud::Kind::Button {
                    id: sims_ui::hud::id::LIVE,
                    cells: 4,
                    selected: false,
                },
                command: None,
            },
            sims_ui::hud::Item {
                rect: sims_ui::frame::Rect::new(16, 2, 10, 10),
                kind: sims_ui::hud::Kind::Portrait {
                    guid: 0x1111_2222,
                    selected: false,
                },
                command: None,
            },
            sims_ui::hud::Item {
                rect: sims_ui::frame::Rect::new(30, 2, 10, 10),
                kind: sims_ui::hud::Kind::Relation {
                    relation: relation(0xAAAA_0001, 1),
                    lit: false,
                },
                command: None,
            },
            sims_ui::hud::Item {
                rect: sims_ui::frame::Rect::new(44, 2, 10, 10),
                kind: sims_ui::hud::Kind::Relation {
                    relation: relation(0xAAAA_0002, 2),
                    lit: false,
                },
                command: None,
            },
        ];

        // Empty highlight sets leave the canvas untouched.
        let mut canvas = Canvas::new(64, 20, [0, 0, 0]);
        draw_ui_effect_highlights(
            &items,
            &BTreeSet::new(),
            &BTreeSet::new(),
            false,
            &mut canvas,
        );
        assert!(canvas.color.iter().all(|&px| px == 0));

        // Active highlights draw the outline on the matching button, portrait,
        // and only the first relationship control (matching 0x4769ca).
        let buttons = BTreeSet::from([sims_ui::hud::id::LIVE as i16]);
        let portraits = BTreeSet::from([0x1111_2222u32]);
        draw_ui_effect_highlights(&items, &buttons, &portraits, true, &mut canvas);
        let pixel = |c: &Canvas, x: usize, y: usize| c.pixel_rgb(x, y);
        assert_eq!(pixel(&canvas, 2, 2), UI_EFFECT_HIGHLIGHT_RGB);
        assert_eq!(pixel(&canvas, 16, 2), UI_EFFECT_HIGHLIGHT_RGB);
        assert_eq!(pixel(&canvas, 30, 2), UI_EFFECT_HIGHLIGHT_RGB);
        assert_eq!(pixel(&canvas, 44, 2), [0, 0, 0]);
        // Interior pixels stay untouched.
        assert_eq!(pixel(&canvas, 6, 6), [0, 0, 0]);
    }
}
