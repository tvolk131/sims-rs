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
    ChoiceDialog, CurrencyKind, ShopCommand, ShopItem, ShopKind, ShopPickerDialog, ShopSnapshot,
    ShopStrings, Wallet, WardrobeCategory,
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

/// What a choice dialog's view opens on.
enum ChoiceOpening {
    /// The person as they are, with the first item equal to their current
    /// row chosen (`0x56d3d9`, then `0x56d852` on OK), or none.
    Current(Option<ShopPreviewTarget>),
    /// The first item: the Charm's preset 0.
    First,
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
    /// Where the last Camera Mode lot click was (`0x4ab80d` records the
    /// pointer at `+0x4c4`/`+0x4c8`), for [`Game::take_photo`]'s frame.
    photo_point: Option<[i32; 2]>,
    /// What the last synthetic special-panel preview would have sent the VM
    /// ([`Game::preview_special_panel`]), for tests.
    preview_reply: Option<SpecialPanelResponse>,
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
    /// Destination lot chosen via a VM special lot picker (`5, 7, 8, 11, 13`).
    /// The pick does not travel: the script asks for the trip with Generic
    /// Sims Call 17 ([`Game::trip_request`]), and the shell reads the picker
    /// for how the trip goes.
    pending_travel_lot: Option<(SpecialPanelRequest, u32)>,
    /// The trip a script asked for ([`sims_sim::World::trip_request`]) that
    /// the host declined in co-op ([`Game::decline_trip`]). Co-op cannot drop
    /// the request from one peer's world without the lockstep, so the lot
    /// keeps it and ticks go on while the pending request is this one.
    declined_trip: Option<sims_sim::TripRequest>,
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

/// Whether a household is living on the lot: global 9 names its family, or
/// the family numbered 0 holds a solo traveler. A solo arrival leaves global
/// 9 at 0 and makes family 0 the traveler alone (0x4f1c87..0x4f1cec, with
/// 0x410a41 erasing then adding the member).
fn live_household(world: &sims_sim::World) -> bool {
    world.globals.get(9).copied().unwrap_or(0) != 0
        || world
            .families
            .iter()
            .any(|f| f.id == 0 && !f.members.is_empty())
}

mod book_dialog;
mod build_mode;
mod buy_mode;
mod commands;
mod coop_shell;
mod dialogs;
mod drawing;
mod opening;
mod pointer;
mod shell_state;
mod shop_dialog;
mod special_panels;
mod ticking;
mod try_on;

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

/// Whether `record` may appear in `caller`'s Phonebook (see
/// [`Game::build_phonebook_snapshot`]).
fn phonebook_eligible(
    record: &sims_sim::neighbors::Neighbor,
    caller: i16,
    caller_family: i16,
    families: &[sims_sim::neighbors::Family],
    row: &sims_sim::neighbors::Row,
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
    // `0x522ea1`: not the caller's family; below 2000 a family needs an
    // ID and a house (`[family + 0x110] >= 1`). Native reaches a neighbor
    // only as a member GUID of a family in the neighborhood's list.
    let family_ok = family != caller_family
        && families.iter().any(|f| {
            f.id == i32::from(family)
                && f.members.contains(&record.guid)
                && (family >= 2000 || (family != 0 && f.house >= 1))
        });
    // `0x522f17`: kind at most 1 and a relationship row with values
    // (`0x464f74` gives the first row's value count).
    record.id > 0 && record.id != caller && kind <= 1 && family_ok && !row.values.is_empty()
}

/// The suit category letter of `Dialog` kind 6 mode `0..=5`. `0x5097c9`
/// turns the mode into the view kind `0, 4, 2, 5, 0x11, 0x12` (any other
/// mode is kind 0), whose lists filter by the letters at `0x6c0fac`
/// (`0x408bf5` `B`, `0x4098f1` `F`, `0x409b08` `S`, `0x409d1f` `L`, then
/// `W` and `H`): everyday `b`, formal `f`, swimwear `s`, pajamas `l`,
/// winter `w`, and Superstar high fashion `h`.
fn wardrobe_mode_category(mode: i16) -> u8 {
    match mode {
        1 => b'f',
        2 => b's',
        3 => b'l',
        4 => b'w',
        5 => b'h',
        _ => b'b',
    }
}

/// The person's body string row of `Dialog` kind 6 mode `0..=5`: the view
/// kind's resolver (`0x40d0a4`, `0x40d52f`, `0x40d847`, `0x40db5f`,
/// `0x40de77`, `0x40e18f`) reads 1-based slot `2, 0x1f, 0x20, 0x21, 0x22,
/// 0x23` through `0x472ec4`: everyday 1, formal 30, swim 31, pajamas 32,
/// winter 33, high fashion 34.
fn wardrobe_mode_row(mode: i16) -> usize {
    match mode {
        1 => 30,
        2 => 31,
        3 => 32,
        4 => 33,
        5 => 34,
        _ => 1,
    }
}

/// The `(suit, texture)` an outfit row compares by (`0x5874ca`): the part
/// before any `;`, split at the first `,`, each kept to the 63 characters
/// of its 64-byte buffer and lowercased, since `0x472f92` compares both
/// with `0x46f874`, which ignores ASCII case.
fn outfit_key(row: &str) -> (String, String) {
    let half = row.split(';').next().unwrap_or("");
    let (suit, texture) = half.split_once(',').unwrap_or((half, ""));
    let fit = |s: &str| -> String { s.chars().take(63).collect::<String>().to_ascii_lowercase() };
    (fit(suit), fit(texture))
}

/// A definition's `CATS` bytes and its `CTSS` table, both numbered by
/// definition word 41, read as the loader reads them:
///
/// - in the byte order the resource's flags give: without flag 0x10 it is
///   big-endian (`0x470351`);
/// - in a table of length-prefixed strings, bytes 0xa1 become `o` and
///   0xd5 become `'` (`0x470f45..0x470f5b`).
fn catalog_strings(
    iff: &sims_formats::iff::Iff,
    table: u16,
) -> (Option<&[u8]>, Option<sims_formats::strings::StringTable>) {
    use sims_formats::strings::{Format, StringTable, decode};
    let cats = iff.get(b"CATS", table).map(|c| iff.data(c));
    let ctss = iff.get(b"CTSS", table).and_then(|c| {
        let mut table = StringTable::parse_chunk(iff.data(c), iff.resource_flags(c)).ok()?;
        if matches!(table.format, Format::Pascal | Format::MacResource) {
            for entry in &mut table.entries {
                let bytes: Vec<u8> = entry
                    .raw_text
                    .iter()
                    .map(|&b| match b {
                        0xa1 => b'o',
                        0xd5 => b'\'',
                        b => b,
                    })
                    .collect();
                entry.text = decode(&bytes, entry.language);
            }
        }
        Some(table)
    });
    (cats, ctss)
}

/// A definition's catalog name (`0x457d3b`, which fills it by
/// `0x4061e3`):
///
/// - with a `CATS` resource, its second string (`0x40631f..0x406352`);
/// - else `CTSS` slot 1 in the game's language, US English
///   (`0x470ca7(0, 0)`, `0x471411(1, -1)`). For a person whose name is
///   empty (`0x4062c1`), `0x47145f(1, 0)` probes language codes 1 to 20
///   in order (indices 0 to 19, each a code less one, `0x470e32`) and
///   takes slot 1 of the first that has one, even when its text is empty
///   (`0x471465..0x471479`);
/// - a name still empty becomes `*` and the definition's own name, its
///   `OBJD` label (`0x457db4..0x457de2`, `"*"` at `0x6c12fc`), so it is
///   never empty.
///
/// A language's slot 1 is the first entry of its last run in the table,
/// as the loader inserts each run in front (`program::loaded_order` in
/// `sims-vm`).
fn definition_catalog_name(
    cats: Option<&[u8]>,
    ctss: Option<&sims_formats::strings::StringTable>,
    person: bool,
    label: &str,
) -> String {
    let slot_one = |language: u8| {
        let entries = &ctss?.entries;
        let mut first = None;
        let mut run = None;
        for entry in entries {
            if run != Some(entry.language) {
                run = Some(entry.language);
                if entry.language == language {
                    first = Some(entry.text.clone());
                }
            }
        }
        first
    };
    let name = match cats {
        Some(raw) => raw
            .split(|&b| b == 0)
            .nth(1)
            .map(|raw| sims_formats::strings::decode(raw, US_ENGLISH))
            .unwrap_or_default(),
        None => {
            let name = slot_one(US_ENGLISH).unwrap_or_default();
            if name.is_empty() && person {
                // The probe walks indices 0..=19, and the loader files a
                // string under its code less one (`0x470e32`), so codes 1
                // to 20 (Korean). The reader gives code 0 language 1.
                (1..=20).find_map(slot_one).unwrap_or_default()
            } else {
                name
            }
        }
    };
    if name.is_empty() {
        format!("*{label}")
    } else {
        name
    }
}

/// Every `(suit, "suit,BODY=texture")` of category `first` that a person
/// of the given sex, age and skin tone can wear, in native wardrobe order:
/// the filter of `ImportWardrobe::pick` (`0x409663`) — a 9-byte suit prefix
/// `<first>NNN<sex|u><age>xxx` and textures `<prefix><skin>…` — with the
/// category letter in place of `b`.
/// A wardrobe suit's body shape as the clothing dialog's arrows read it:
/// up to three characters after the first six (`0x40d1f0..0x40d1fc`,
/// `b004mafit_01` → `fit`). The port's suit names are those of its
/// wardrobe list ([`category_outfits`]).
fn body_shape(suit: &str) -> &str {
    let bytes = suit.as_bytes();
    let start = bytes.len().min(6);
    let end = bytes.len().min(9);
    suit.get(start..end).unwrap_or("")
}

/// The Charm Transform's suit for `preset` (`0x5374d3`): children
/// alternate one costume with the skeleton (60), adults pick from an
/// eight-entry table by sex (jump tables `0x537595` and `0x5375b5`).
fn charm_suit(preset: usize, female: bool, child: bool) -> i16 {
    const MALE: [i16; 8] = [48, 58, 44, 45, 39, 46, 47, 40];
    const FEMALE: [i16; 8] = [48, 43, 44, 49, 39, 50, 51, 52];
    match (child, preset.is_multiple_of(2)) {
        (true, true) if female => 54,
        (true, true) => 53,
        (true, false) => 60,
        (false, _) if female => FEMALE[preset % 8],
        (false, _) => MALE[preset % 8],
    }
}

/// A name for a Charm costume, from its body in the VM's suit table
/// (`sims_vm` `costume_spec`, `0x58a948..0x58d475`). Native shows no name;
/// the port shows it while the view cannot draw the costume.
fn charm_costume_label(suit: i16) -> &'static str {
    match suit {
        39 => "Blue Genie",
        40 => "Minotaur",
        43 | 58 => "Vampire",
        44 => "Werewolf",
        45 => "Headless",
        46 => "Troll",
        47 => "Hunchback",
        48 => "Sorcerer",
        49 => "Witch",
        50 => "Ogress",
        51 => "Crone",
        52 => "Nymph",
        53 => "Goblin",
        54 => "Wizard Girl",
        60 => "Skeleton",
        _ => "Costume",
    }
}

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

/// Whether two trip requests are the same trip: the same house, party and
/// caller. `saved` is left out, because a later Generic Sims Call 17 adds
/// to a pending request's `saved` without changing the trip.
fn same_trip(a: &sims_sim::TripRequest, b: &sims_sim::TripRequest) -> bool {
    a.house == b.house
        && a.leader_guid == b.leader_guid
        && a.family == b.family
        && a.caller == b.caller
}

#[cfg(test)]
mod tests;
