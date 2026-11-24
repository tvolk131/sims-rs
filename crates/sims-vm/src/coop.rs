//! Co-op multiplayer VM session bridge (`docs/vision.md`, "Multiplayer (v1)").
//!
//! Wires [`sims_net::SessionState`] and [`sims_net::Command`] batches into
//! [`crate::Interpreter`] and [`crate::boot::Lot`] while keeping single-player
//! execution (`coop == None`) 100% bit-for-bit identical:
//!
//! - **Global 3 (`Selected Person ID`) multiplayer policy:**
//!   - *"Am I the selected Sim?"*: when a Sim's own script reads Global 3 and
//!     **any** player in [`SessionState`] has that Sim selected, it resolves to
//!     that Sim's own ID.
//!   - *"Who is the player's Sim?"*: when a non-Sim or unselected activity
//!     script reads Global 3, it resolves to the most relevant player's selected
//!     Sim (the player whose command/party started the activity, or whose
//!     selected Sim is the stack object), falling back to the host's selected
//!     Sim.
//!   - Script-driven Global 3 writes (e.g. *Magic – Kid To Adult Backfire*)
//!     update the player(s) who had the affected Sim selected, falling back to
//!     the activity owner or host, and reconcile `world.selected()` immediately.
//! - **Per-player `camera_rotation` & isolated local camera panning:**
//!   - `Look Towards`, `Generic Sims Call 12` (tile depth), `Find Location`
//!     mode 5, and head model placement resolve the camera rotation of the
//!     player selecting `me` (for person scripts, strictly falling back to the
//!     host) or the player selecting `stack_object` / owning the activity (for
//!     non-person scripts).
//!   - Local camera panning (`Lot::view_centre`) NEVER feeds the co-op
//!     simulation; portrait search uses an authoritative lot-coordinate
//!     `portrait_anchor` (or tile-independent search when `None`).
//! - **Authoritative co-op `Frames` schedule & Tutorial/Help suppression:**
//!   - One authoritative [`Frames`] schedule is installed on attach, serialized
//!     in co-op snapshots, and hashed in [`Lot::coop_world_hash`].
//!   - Tutorial lots (`Global 26 != 0`) are refused on co-op attach, and
//!     Tutorial / Help System controllers (libraries using opcode 10 or 34) are
//!     deterministically parked so single-player help never pauses a co-op lot.
//! - **Sequential command execution, frozen dialog replay & atomic Buy/Build:**
//!   - Commands in a batch execute sequentially in exact host-stamped order,
//!     validating selections and dialog button statuses before mutating session
//!     state and enforcing mode gates at each command's position.
//!   - Interrupted ticks freeze their pre-tick `(session, activity_owners,
//!     free_will)` across all dialogs in the tick and through answered-but-paused
//!     snapshots, replaying identically on live peers and snapshot joiners.
//!   - `SimActionPick` resolves against a throwaway issuer-specific `pie_menu`
//!     probe and commits staged `world` + ownership only when queued/executed.
//!   - `BuyBuild` edits execute atomically on a staged `(World, CoopVmContext)`
//!     using catalog [`BuildPrices`], enforcing exact facing/container/style/
//!     pattern and purging all detached history before committing.

use crate::boot::save::{SaveError, Saved};
use crate::boot::{AnswerError, Frames, Lot, Menu, OrderError, SpecialPanelAnswerError, StepError};
use crate::build::{
    BuildPrices, FloorDragMode, SPLIT_FLOOR_ID, WALL_EAST, WALL_NORTH, WALL_SOUTH, WALL_WEST,
    WallDragMode, WallFace, WallStyle, WallpaperDragMode, is_buildable_tile, plan_floor_edit,
    plan_wall_drag, plan_wallpaper_edit,
};
use crate::buy::{BuyPurchase, snap_to_tile_centre};
use crate::interpreter::{Interpreter, Stream};
use crate::orders::MenuEntry;
use crate::{
    Answer, Answers, PanelMode, Shown, ShownSpecialPanel, SpecialPanelRequest,
    SpecialPanelResponse, Update,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_content::ui_graphics::US_ENGLISH;
use sims_content::{Install, Surfaces, atoi};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_net::{
    AppliedBatch, BuyBuildEdit, CameraRotation, Command, CommandIgnoreReason, CommandRejectReason,
    DialogAnswer, DialogId, InviteTicket, PlayerId, PlayerMode, PlayerRole, SessionError,
    SessionState, SimSpeed, StampedCommand, StateFingerprint, Tick, TickBatch,
};
use sims_sim::World;
use sims_sim::load::{ExpandedWall, WALLS};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Global 3: selected person ID (`0x445449`).
const GLOBAL_SELECTED: usize = 3;
/// Global 15: host simulation language.
const GLOBAL_LANGUAGE: usize = 15;
/// Global 17: simulation paused flag (`0x46d813`).
const GLOBAL_PAUSED: usize = 17;
/// Global 26: tutorial flag (`0x4878f3`, single-player only).
const GLOBAL_TUTORIAL: usize = 26;
/// Global 30: Free Will (`0x432220`).
const GLOBAL_FREE_WILL: usize = 30;
/// Global 33: Machine Level (fixed per session for determinism).
const GLOBAL_MACHINE_LEVEL: usize = 33;

/// Object data word 1: facing (`0..=7`).
const DATA_FACING: usize = 1;

/// SimAntics opcode 10 (`Tutorial` / `own_events`) and opcode 34 (`UI Effect`),
/// used exclusively by the single-player Tutorial and Help System controllers.
const OPCODE_TUTORIAL: u16 = 10;
const OPCODE_UI_EFFECT: u16 = 34;
/// Scheduler flag 1 (`READY`): cleared to park single-player Help System
/// controllers in co-op mode.
const SCHEDULER_READY: u16 = 1;

/// Domain separator for [`Lot::coop_world_hash`].
const COOP_WORLD_HASH_DOMAIN: &[u8] = b"sims-vm/coop-world/v1\0";
/// Magic prefix distinguishing [`CoopSnapshotEnvelope`] from raw [`Saved`] bytes.
const COOP_SNAPSHOT_MAGIC: &[u8; 4] = b"SCOP";
/// Wire version of [`CoopSnapshotEnvelope`].
const COOP_SNAPSHOT_VERSION: u32 = 3;

/// Tagged `DialogAnswer::status` values carrying [`SpecialPanelResponse`] over
/// [`Command::AnswerBlockingDialog`] (disjoint from ordinary statuses `2..=4`).
const SPECIAL_PANEL_STATUS_LOT_CANCEL: u8 = 0x81;
const SPECIAL_PANEL_STATUS_LOT_PICK: u8 = 0x82;
const SPECIAL_PANEL_STATUS_NEIGHBOR_CANCEL: u8 = 0x83;
const SPECIAL_PANEL_STATUS_NEIGHBOR_PICK: u8 = 0x84;
const SPECIAL_PANEL_STATUS_SELECTION_CANCEL: u8 = 0x85;
const SPECIAL_PANEL_STATUS_SELECTION_ACCEPT: u8 = 0x86;
const SPECIAL_PANEL_STATUS_DISMISSED: u8 = 0x87;

/// Encodes a [`SpecialPanelResponse`] into a [`DialogAnswer`] suitable for
/// lockstep [`Command::AnswerBlockingDialog`].
#[must_use]
pub fn special_panel_response_to_dialog_answer(response: &SpecialPanelResponse) -> DialogAnswer {
    match *response {
        SpecialPanelResponse::Lot(None) => DialogAnswer {
            status: SPECIAL_PANEL_STATUS_LOT_CANCEL,
            bitmap_id: None,
            text: None,
        },
        SpecialPanelResponse::Lot(Some(lot)) => DialogAnswer {
            status: SPECIAL_PANEL_STATUS_LOT_PICK,
            bitmap_id: Some(lot),
            text: None,
        },
        SpecialPanelResponse::Neighbor(None) => DialogAnswer {
            status: SPECIAL_PANEL_STATUS_NEIGHBOR_CANCEL,
            bitmap_id: None,
            text: None,
        },
        SpecialPanelResponse::Neighbor(Some(neighbor)) => DialogAnswer {
            status: SPECIAL_PANEL_STATUS_NEIGHBOR_PICK,
            bitmap_id: Some(neighbor),
            text: None,
        },
        SpecialPanelResponse::Selection {
            accepted: false,
            index,
        } => DialogAnswer {
            status: SPECIAL_PANEL_STATUS_SELECTION_CANCEL,
            bitmap_id: Some(index),
            text: None,
        },
        SpecialPanelResponse::Selection {
            accepted: true,
            index,
        } => DialogAnswer {
            status: SPECIAL_PANEL_STATUS_SELECTION_ACCEPT,
            bitmap_id: Some(index),
            text: None,
        },
        SpecialPanelResponse::Dismissed => DialogAnswer {
            status: SPECIAL_PANEL_STATUS_DISMISSED,
            bitmap_id: None,
            text: None,
        },
    }
}

/// Decodes a [`DialogAnswer`] produced by [`special_panel_response_to_dialog_answer`]
/// back into a typed [`SpecialPanelResponse`].
pub fn dialog_answer_to_special_panel_response(
    answer: &DialogAnswer,
) -> Result<SpecialPanelResponse, String> {
    if answer.text.is_some() {
        return Err("special panel response must not carry text payload".into());
    }
    match (answer.status, answer.bitmap_id) {
        (SPECIAL_PANEL_STATUS_LOT_CANCEL, None) => Ok(SpecialPanelResponse::Lot(None)),
        (SPECIAL_PANEL_STATUS_LOT_PICK, Some(lot)) => Ok(SpecialPanelResponse::Lot(Some(lot))),
        (SPECIAL_PANEL_STATUS_NEIGHBOR_CANCEL, None) => Ok(SpecialPanelResponse::Neighbor(None)),
        (SPECIAL_PANEL_STATUS_NEIGHBOR_PICK, Some(neighbor)) => {
            Ok(SpecialPanelResponse::Neighbor(Some(neighbor)))
        }
        (SPECIAL_PANEL_STATUS_SELECTION_CANCEL, Some(index)) => {
            Ok(SpecialPanelResponse::Selection {
                accepted: false,
                index,
            })
        }
        (SPECIAL_PANEL_STATUS_SELECTION_ACCEPT, Some(index)) => {
            Ok(SpecialPanelResponse::Selection {
                accepted: true,
                index,
            })
        }
        (SPECIAL_PANEL_STATUS_DISMISSED, None) => Ok(SpecialPanelResponse::Dismissed),
        (status, bitmap_id) => Err(format!(
            "invalid special panel dialog answer (status={status:#x}, bitmap_id={bitmap_id:?})"
        )),
    }
}

/// Builds a lockstep [`Command::AnswerBlockingDialog`] answering an active
/// co-op special panel (`CoopBatchReport::opened_special_panel`).
#[must_use]
pub fn special_panel_answer_command(
    dialog_id: DialogId,
    response: &SpecialPanelResponse,
) -> Command {
    Command::AnswerBlockingDialog {
        dialog_id,
        answer: special_panel_response_to_dialog_answer(response),
    }
}

/// `GameData/Build.iff` `STR#` chunk IDs (`0x4cd453`, `0x4cd696`, `0x4cd71b`).
const STR_WALLS: u16 = 129;
const STR_FLOORS: u16 = 130;
const STR_WALLPAPERS: u16 = 131;
const STR_PLUGIN_FLOOR: u16 = 134;
const STR_PLUGIN_WALL: u16 = 135;

/// Built-in wall/fence style table matching `cWinArch` (`0x4cd464..0x4cd608`):
/// `(style_id, str_entry_1based, default_price)`.
const BUILTIN_WALL_STYLES: [(u16, usize, i32); 5] = [
    (1, 1, 70),
    (2, 2, 10),
    (12, 4, 35),
    (13, 3, 45),
    (14, 5, 45),
];

/// Maps [`sims_net::PlayerMode`] onto the simulator's [`PanelMode`].
///
/// Note: in `Sims.exe` 1.91, `0x4cea5d` uses UI panel enum values (`Build = 0`,
/// `Buy = 1`, `Live = 2`), whereas `0x4ce44f` writes Global 19 as `0` Live,
/// `1` Build, `2` Buy (`crate::buy::panel_mode`, `tick.rs:574`, `pick.rs:89`).
/// Mapping through [`PanelMode`] guarantees Global 19 uses the exact simulator
/// encoding.
pub const fn player_mode_to_panel_mode(mode: PlayerMode) -> PanelMode {
    match mode {
        PlayerMode::Live => PanelMode::Live,
        PlayerMode::Build => PanelMode::Build,
        PlayerMode::Buy => PanelMode::Buy,
    }
}

/// Converts a [`MenuEntry`] listed by [`Lot::coop_pie_menu`] into the exact
/// lockstep [`Command::SimActionPick`] payload (preserving dynamic arguments
/// and join actions).
pub fn menu_entry_to_pick_command(entry: &MenuEntry) -> Command {
    Command::SimActionPick {
        sim_id: entry.action.caller as u16,
        target_id: entry.action.target as u16,
        menu_index: entry.action.table_index as i16,
        joining: entry.action.join != 0,
        args: entry.action.args,
    }
}

/// Encodes a 32-bit interaction UID returned by [`CoopCommandStatus::QueuedInteraction`]
/// into the `i16` wire field of [`Command::SimActionCancel`].
///
/// [`Lot::apply_coop_batch`] resolves the wire `i16` back to the exact `i32`
/// UID on the target Sim's active/queued interaction list (at most 9 entries),
/// supporting serials beyond `32767` cleanly while deterministically refusing
/// any ambiguous 16-bit wrap collision.
pub const fn cancel_uid_to_wire(uid: i32) -> i16 {
    uid as i16
}

/// Builds the full catalog [`BuildPrices`] table (wall style overrides,
/// built-in floors/wallpapers from `GameData/Build.iff`, and plug-in
/// `.flr`/`.wll` prices from `surfaces`) matching `cArchCatalog`
/// (`0x4cd149..0x4cd9a5`).
pub fn build_prices_from_install(install: &Install, surfaces: &Surfaces) -> BuildPrices {
    let build_iff = install
        .file("GameData/Build.iff")
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| Iff::parse(bytes).ok());

    let str_price = |chunk_id: u16, entry_1based: usize| -> Option<i32> {
        let iff = build_iff.as_ref()?;
        let chunk = iff.get(b"STR#", chunk_id)?;
        let table = StringTable::parse_chunk(iff.data(chunk), iff.resource_flags(chunk)).ok()?;
        let base = entry_1based.checked_sub(1)? * 3;
        let name = table.entry(base, US_ENGLISH)?.text.as_str();
        if name.is_empty() {
            return None;
        }
        let price_entry = table.entry(base + 1, US_ENGLISH)?;
        Some(atoi(&price_entry.raw_text))
    };

    let mut prices = BuildPrices::default();

    // 1. Wall & fence styles (`STR# 129`).
    for (style_id, entry_1based, fallback_price) in BUILTIN_WALL_STYLES {
        let price = str_price(STR_WALLS, entry_1based).unwrap_or(fallback_price);
        prices.styles.insert(style_id, price);
    }
    let std_price = prices.styles.get(&1).copied().unwrap_or(70);
    prices.styles.insert(22, std_price);
    prices.styles.insert(23, std_price);

    // 2. Built-in & plug-in floors (`STR# 130` & `STR# 134`).
    let def_floor_price = str_price(STR_PLUGIN_FLOOR, 1).unwrap_or(20);
    for key in 1u16..=29 {
        if (9..=13).contains(&key) {
            continue;
        }
        if let Some(price) = str_price(STR_FLOORS, usize::from(key)) {
            prices.floors.insert(key, price);
        }
    }
    for file in &surfaces.floors.files {
        let price = file.price.unwrap_or(def_floor_price);
        for &key in file.keys.values() {
            prices.floors.insert(key, price);
        }
    }

    // 3. Built-in & plug-in wallpapers (`STR# 131` & `STR# 135`).
    let def_wall_price = str_price(STR_PLUGIN_WALL, 1).unwrap_or(5);
    for key in 1u16..=30 {
        if let Some(price) = str_price(STR_WALLPAPERS, usize::from(key)) {
            prices.walls.insert(key, price);
        }
    }
    for file in &surfaces.walls.files {
        let price = file.price.unwrap_or(def_wall_price);
        for &key in file.keys.values() {
            if (248..=255).contains(&key) {
                continue;
            }
            prices.walls.insert(key, price);
        }
    }

    prices
}

/// Builds a fallback [`BuildPrices`] from `surfaces` alone when no `Install`
/// handle is available (e.g. synthetic unit tests or default attach).
fn default_build_prices_from_surfaces(surfaces: &Surfaces) -> BuildPrices {
    let mut prices = BuildPrices::default();
    for (style_id, _, fallback_price) in BUILTIN_WALL_STYLES {
        prices.styles.insert(style_id, fallback_price);
    }
    prices.styles.insert(22, 70);
    prices.styles.insert(23, 70);
    for file in &surfaces.floors.files {
        let price = file.price.unwrap_or(20);
        for &key in file.keys.values() {
            prices.floors.insert(key, price);
        }
    }
    for file in &surfaces.walls.files {
        let price = file.price.unwrap_or(5);
        for &key in file.keys.values() {
            if (248..=255).contains(&key) {
                continue;
            }
            prices.walls.insert(key, price);
        }
    }
    prices
}

/// Complete frozen pre-tick co-op inputs preserved across every blocking modal
/// dialog in an interrupted tick (and through answered-but-paused states and
/// snapshots) so live replay and snapshot restore always execute the exact
/// same interrupted tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FrozenDialogTick {
    pub session: SessionState,
    pub activity_owners: BTreeMap<u16, PlayerId>,
    pub free_will: bool,
    /// Players who issued an accepted `Command::SelectPerson` while this
    /// interrupted tick remained frozen; post-replay merge never overwrites
    /// their explicit choice even if they reselected the original Sim.
    pub intervening_selections: BTreeSet<PlayerId>,
}

impl FrozenDialogTick {
    fn digest(&self) -> [u8; 32] {
        #[derive(Serialize)]
        struct Canonical<'a> {
            session_fp: StateFingerprint,
            activity_owners: &'a BTreeMap<u16, PlayerId>,
            free_will: bool,
            intervening_selections: &'a BTreeSet<PlayerId>,
        }
        let body = postcard::to_allocvec(&Canonical {
            session_fp: self.session.session_fingerprint(),
            activity_owners: &self.activity_owners,
            free_will: self.free_will,
            intervening_selections: &self.intervening_selections,
        })
        .expect("FrozenDialogTick postcard serialization is infallible");
        let mut h = Sha256::new();
        h.update(b"sims-vm/coop-frozen-dialog/v2\0");
        h.update(&body);
        h.finalize().into()
    }
}

/// Co-op multiplayer context attached to a [`Lot`] / [`Interpreter`].
///
/// Always `None` in single-player so single-player execution stays 100%
/// bit-for-bit identical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoopVmContext {
    /// Deterministic shared co-op session state.
    pub session: SessionState,
    /// Maps an object/Sim ID to the player whose command most recently started
    /// an activity involving it (used to answer *"Who is the player's Sim?"*
    /// for Global 3 and per-player camera rotation on non-Sim activity scripts).
    pub activity_owners: BTreeMap<u16, PlayerId>,
    /// Next deterministic blocking modal [`DialogId`] to allocate.
    pub next_dialog_id: u64,
    /// Currently open blocking modal dialog awaiting first-answer-wins, if any.
    pub active_dialog: Option<DialogId>,
    /// Currently open blocking special panel (`Dialog` kinds `5..=15`) awaiting
    /// first-answer-wins, if any.
    pub active_special_panel: Option<(DialogId, ShownSpecialPanel)>,
    /// Frozen pre-tick inputs for an interrupted dialog tick (preserved across
    /// every dialog in the tick and through answered-but-paused states until
    /// replay completes).
    pub(crate) frozen_dialog: Option<FrozenDialogTick>,
    /// Deferred `Command::SetSpeed` received while a blocking dialog froze
    /// `self.world`, applied as soon as the dialog replay completes.
    pub(crate) pending_speed_sync: bool,
    /// Authoritative co-op frame draw schedule installed on every peer so
    /// animator/room/outfit side effects of drawing never diverge after resync.
    pub frames: Frames,
    /// Optional authoritative lot-coordinate tile `[tx, ty]` used as the
    /// portrait search start in co-op mode (never influenced by local camera
    /// panning or per-player rotation). `None` uses tile-independent search.
    pub portrait_anchor: Option<[i8; 2]>,
    /// Shared Architecture pricing table used for co-op Build Mode edits.
    pub build_prices: BuildPrices,
    /// Whether [`Lot::apply_coop_batch`] steps in play mode (`Lot::step_play`)
    /// instead of strict mode (`Lot::step`). Defaults to `false`.
    pub play_mode: bool,
    /// Temporary per-player override active only during throwaway local checks
    /// (such as [`Lot::coop_pie_menu`]) so check trees see that exact player's
    /// selected Sim and camera rotation.
    pub(crate) active_player_override: Option<PlayerId>,
}

impl CoopVmContext {
    /// Creates a new co-op VM context wrapping `session` with the authoritative
    /// default [`Frames`] schedule.
    pub fn new(session: SessionState) -> Self {
        Self {
            session,
            activity_owners: BTreeMap::new(),
            next_dialog_id: 1,
            active_dialog: None,
            active_special_panel: None,
            frozen_dialog: None,
            pending_speed_sync: false,
            frames: Frames::default(),
            portrait_anchor: None,
            build_prices: BuildPrices::default(),
            play_mode: false,
            active_player_override: None,
        }
    }

    /// Configures whether [`Lot::apply_coop_batch`] runs in play mode.
    pub fn with_play_mode(mut self, play_mode: bool) -> Self {
        self.play_mode = play_mode;
        self
    }

    /// Sets the authoritative co-op [`Frames`] draw schedule.
    pub fn with_frames(mut self, frames: Frames) -> Self {
        self.frames = frames;
        self
    }

    /// Sets the authoritative lot-coordinate portrait search anchor `[tx, ty]`.
    pub fn with_portrait_anchor(mut self, anchor: Option<[i8; 2]>) -> Self {
        self.portrait_anchor = anchor;
        self
    }

    /// Sets the authoritative catalog [`BuildPrices`] table for co-op Build
    /// Mode edits (e.g. from [`build_prices_from_install`]).
    pub fn with_build_prices(mut self, prices: BuildPrices) -> Self {
        self.build_prices = prices;
        self
    }

    /// Records that `player` initiated an activity on `object_id`.
    pub fn record_activity_owner(&mut self, object_id: u16, player: PlayerId) {
        if object_id != 0 {
            self.activity_owners.insert(object_id, player);
        }
    }

    /// Removes a deleted object/Sim from `activity_owners` and clears any
    /// player selection pointing to it.
    pub fn remove_object(&mut self, id: u16) {
        self.activity_owners.remove(&id);
        self.session.apply_script_selection_change(id, None);
    }

    /// Returns all unique Sim IDs currently selected by any player, in
    /// deterministic ascending order.
    pub fn selected_sims(&self) -> Vec<u16> {
        let mut sims = BTreeSet::new();
        for player in self.session.players.values() {
            if let Some(sim) = player.selected_sim {
                sims.insert(sim);
            }
        }
        sims.into_iter().collect()
    }

    /// Resolves a script read of Global 3 (`Selected Person ID`) according to
    /// `docs/vision.md` lines 155–168:
    ///
    /// 1. During a throwaway per-player check (`active_player_override`),
    ///    returns that player's selected Sim.
    /// 2. *"Am I the selected Sim?"*: when a Sim's own script (`me` is a
    ///    person) reads Global 3 and **any** player has `me` selected, returns
    ///    `me` so every player's Sim behaves identically to a selected Sim.
    /// 3. *"Who is the player's Sim?"*: otherwise resolves the most relevant
    ///    player's selected Sim — the player whose command started an activity
    ///    on `me`, then `stack_object`, then whichever player currently selects
    ///    `stack_object`, falling back to the host's selected Sim.
    pub fn resolve_global_3(&self, world: &World, me: u16, stack_object: i16) -> i16 {
        if let Some(player_id) = self.active_player_override {
            return self
                .session
                .players
                .get(&player_id)
                .and_then(|p| p.selected_sim)
                .map_or(0, |id| id as i16);
        }

        let me_is_person = world.objects.get(&me).is_some_and(|o| o.person.is_some());
        if me_is_person && self.session.is_sim_selected_by_any(me) {
            return me as i16;
        }

        let stack_u16 = u16::try_from(stack_object).ok().filter(|&id| id != 0);
        let actor_player = self
            .activity_owners
            .get(&me)
            .copied()
            .or_else(|| stack_u16.and_then(|id| self.activity_owners.get(&id).copied()))
            .or_else(|| {
                stack_u16.and_then(|id| {
                    self.session
                        .players
                        .values()
                        .find(|p| p.selected_sim == Some(id))
                        .map(|p| p.player_id)
                })
            });

        self.session
            .selected_sim_for_activity(actor_player)
            .map_or(0, |id| id as i16)
    }

    /// Applies a script-driven write to Global 3 (`docs/vision.md` lines
    /// 169–171, e.g. *Magic – Kid To Adult Backfire*).
    ///
    /// Updates every player who currently had the affected Sim selected
    /// (preferring `me` if it is a person, then `stack_object` if it is a
    /// person), falling back to the activity owner and finally the host.
    /// Returns the list of affected [`PlayerId`]s.
    pub fn write_global_3(
        &mut self,
        world: &World,
        me: u16,
        stack_object: i16,
        value: i16,
    ) -> Vec<PlayerId> {
        let new_sim = u16::try_from(value)
            .ok()
            .filter(|&id| id != 0 && world.objects.get(&id).is_some_and(|o| o.person.is_some()));
        if let Some(player_id) = self.active_player_override {
            if let Some(player) = self.session.players.get_mut(&player_id) {
                player.selected_sim = new_sim;
                return vec![player_id];
            }
            return Vec::new();
        }

        let me_is_person = world.objects.get(&me).is_some_and(|o| o.person.is_some());
        let stack_u16 = u16::try_from(stack_object).ok().filter(|&id| id != 0);
        let stack_is_person = stack_u16
            .and_then(|id| world.objects.get(&id))
            .is_some_and(|o| o.person.is_some());

        if me_is_person {
            let updated = self.session.apply_script_selection_change(me, new_sim);
            if !updated.is_empty() {
                return updated;
            }
        }
        if stack_is_person && let Some(target_sim) = stack_u16 {
            let updated = self
                .session
                .apply_script_selection_change(target_sim, new_sim);
            if !updated.is_empty() {
                return updated;
            }
        }
        if let Some(owner) = self
            .activity_owners
            .get(&me)
            .copied()
            .or_else(|| stack_u16.and_then(|id| self.activity_owners.get(&id).copied()))
            && let Some(player) = self.session.players.get_mut(&owner)
        {
            player.selected_sim = new_sim;
            return vec![owner];
        }
        let host = self.session.host_id;
        if let Some(player) = self.session.players.get_mut(&host) {
            player.selected_sim = new_sim;
            return vec![host];
        }
        Vec::new()
    }

    /// Resolves per-player `camera_rotation` (`0..=3`) for a script or
    /// simulation query on behalf of `me` (`docs/vision.md` lines 130–134).
    ///
    /// - For a **person** (`me` is a Sim): strictly resolves via
    ///   [`SessionState::camera_rotation_for_sim`], using the rotation of the
    ///   player selecting `me` (lowest [`PlayerId`] tie-breaker) and falling
    ///   back directly to the host's rotation if `me` is unselected.
    /// - For a **non-person** script: resolves via the player selecting
    ///   `stack_object` or owning the activity on `me`/`stack_object`, falling
    ///   back to the host's rotation.
    pub fn resolve_camera_rotation(&self, world: &World, me: u16, stack_object: Option<i16>) -> u8 {
        if let Some(player_id) = self.active_player_override {
            if let Some(player) = self.session.players.get(&player_id) {
                return player.camera_rotation.quarter_turns();
            }
            return self.host_camera_rotation();
        }

        if world.objects.get(&me).is_some_and(|o| o.person.is_some()) {
            return self.session.camera_rotation_for_sim(me).quarter_turns();
        }

        let stack_u16 = stack_object
            .and_then(|id| u16::try_from(id).ok())
            .filter(|&id| id != 0);
        if let Some(stack_id) = stack_u16
            && world
                .objects
                .get(&stack_id)
                .is_some_and(|o| o.person.is_some())
            && self.session.is_sim_selected_by_any(stack_id)
        {
            return self
                .session
                .camera_rotation_for_sim(stack_id)
                .quarter_turns();
        }

        if let Some(owner) = self
            .activity_owners
            .get(&me)
            .copied()
            .or_else(|| stack_u16.and_then(|id| self.activity_owners.get(&id).copied()))
            && let Some(player) = self.session.players.get(&owner)
        {
            return player.camera_rotation.quarter_turns();
        }

        self.host_camera_rotation()
    }

    /// Returns the host's normalized camera rotation (`0..=3`).
    pub fn host_camera_rotation(&self) -> u8 {
        self.session
            .players
            .get(&self.session.host_id)
            .map(|p| p.camera_rotation.quarter_turns())
            .unwrap_or(0)
    }
}

/// Execution outcome of a single stamped command inside [`Lot::apply_coop_batch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoopCommandStatus {
    /// Command executed cleanly against session/lot state.
    Applied,
    /// `SimActionPick` queued an interaction (or queue was full / at-once
    /// declined -> `None`).
    QueuedInteraction { uid: Option<i32> },
    /// `SimActionCancel` executed (`counted` indicates whether an action matched).
    CancelledInteraction { counted: bool },
    /// Winning `AnswerBlockingDialog` answered the active modal dialog.
    DialogAnswered {
        dialog_id: DialogId,
        winner: PlayerId,
    },
    /// Buy or Build edit committed cleanly.
    BuyBuildApplied,
    /// Rejected deterministically by [`SessionState`] policy (e.g. host lock).
    RejectedBySession(CommandRejectReason),
    /// Ignored deterministically by [`SessionState`] (e.g. losing dialog race).
    IgnoredBySession(CommandIgnoreReason),
    /// Gameplay command could not execute while a blocking modal dialog awaits
    /// answer or replay.
    IgnoredWhileDialogPending,
    /// Command was refused cleanly by the lot without halting the session.
    Refused(String),
}

/// Result of executing one authoritative lockstep batch on a [`Lot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoopBatchReport {
    /// Lockstep tick number executed.
    pub tick: Tick,
    /// Whether the lot actually stepped a simulation tick (`false` while
    /// paused or waiting on an unanswered blocking dialog).
    pub stepped: bool,
    /// Lot update details when `stepped` is `true` and no blocking dialog
    /// interrupted the tick.
    pub update: Option<Update>,
    /// Newly opened blocking modal dialog if stepping hit an unanswered dialog.
    pub opened_dialog: Option<(DialogId, Shown)>,
    /// Newly opened blocking special panel (`Dialog` kinds `5..=15`) if stepping
    /// hit an unanswered special panel.
    pub opened_special_panel: Option<(DialogId, ShownSpecialPanel)>,
    /// Per-command deterministic outcomes in exact batch sequence order.
    pub command_results: Vec<(StampedCommand, CoopCommandStatus)>,
    /// Any player selections altered by scripts (including immediate at-once /
    /// cancel callbacks and end-of-update reselection) during this batch.
    pub script_selection_changes: Vec<(PlayerId, Option<u16>)>,
    /// Deterministic VM world hash after this batch (pass to
    /// `LockstepNode::report_local_tick`).
    pub world_hash: [u8; 32],
    /// Combined [`StateFingerprint`] matching `session.combined_fingerprint(world_hash)`.
    pub state_fingerprint: StateFingerprint,
}

/// Error returned by co-op batch execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoopError {
    /// [`Lot::attach_coop`] has not been called on this lot.
    NotAttached,
    /// Lockstep [`SessionState::apply_batch`] or continuity validation rejected
    /// the batch.
    Session(SessionError),
    /// Simulation tick failed fatally.
    Step(StepError),
    /// Lot was not ready to attach or execute co-op commands.
    Order(OrderError),
}

impl fmt::Display for CoopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAttached => f.write_str("co-op VM context is not attached to this lot"),
            Self::Session(e) => write!(f, "co-op session error: {e}"),
            Self::Step(e) => write!(f, "co-op step error: {e}"),
            Self::Order(e) => write!(f, "co-op order error: {e}"),
        }
    }
}

impl std::error::Error for CoopError {}

impl From<SessionError> for CoopError {
    fn from(e: SessionError) -> Self {
        Self::Session(e)
    }
}

impl From<StepError> for CoopError {
    fn from(e: StepError) -> Self {
        Self::Step(e)
    }
}

impl From<OrderError> for CoopError {
    fn from(e: OrderError) -> Self {
        Self::Order(e)
    }
}

/// Auxiliary co-op VM metadata bundled with [`Saved`] bytes in
/// [`Lot::coop_snapshot_bytes`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct CoopSnapshotMeta {
    activity_owners: BTreeMap<u16, PlayerId>,
    next_dialog_id: u64,
    active_dialog: Option<DialogId>,
    active_special_panel: Option<(DialogId, ShownSpecialPanel)>,
    frozen_dialog: Option<FrozenDialogTick>,
    pending_speed_sync: bool,
    frames: Frames,
    portrait_anchor: Option<[i8; 2]>,
    build_prices: BuildPrices,
    play_mode: bool,
}

/// Canonical snapshot payload transmitted inside `SnapshotEnvelope::redb_bytes`
/// for mid-game join and drift recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CoopSnapshotEnvelope {
    saved_bytes: Vec<u8>,
    meta: CoopSnapshotMeta,
}

/// Canonical subset of format-8 unencoded `World` fields plus `Lot`/co-op
/// continuation inputs hashed into [`Lot::coop_world_hash`].
///
/// **A change to these fields bumps `sims_net::PROTOCOL_VERSION`**: peers
/// on two builds would otherwise pass the handshake and then fail every
/// fingerprint check, even on an idle lot.
#[derive(Serialize)]
struct CoopWorldFingerprintTail<'a> {
    world_hash: [u8; 32],
    ticks: u32,
    view_rotation: u8,
    answered: bool,
    asking: Option<(u32, u16)>,
    asking_special_panel: Option<(u32, &'a SpecialPanelRequest)>,
    special_panels: &'a crate::SpecialPanels,
    special_panel_responses: &'a std::collections::VecDeque<crate::SpecialPanelResponse>,
    answers: &'a Answers,
    replay_view_rotation: Option<u8>,
    frames: &'a Frames,
    portrait_anchor: Option<[i8; 2]>,
    build_prices: Option<&'a BuildPrices>,
    play_mode: bool,
    play_fresh: &'a BTreeSet<u16>,
    last_dialog_text: &'a str,
    character_names: &'a BTreeMap<u32, String>,
    generated_portrait_2002: &'a BTreeSet<sims_sim::PortraitFile>,
    made_portrait_head: &'a BTreeMap<u32, Option<bool>>,
    ambiguous_portraits: &'a BTreeSet<u32>,
    loaded_indexed_skeletons: &'a BTreeMap<String, Option<String>>,
    loaded_indexed_suits: &'a BTreeMap<String, Option<bool>>,
    loaded_indexed_suit_bones: &'a BTreeMap<String, Vec<Vec<String>>>,
    made_outfits: &'a BTreeMap<u32, BTreeMap<usize, String>>,
    person_outfits: &'a BTreeMap<u16, sims_sim::PersonOutfit>,
    scratch_ptr_set: sims_sim::ScratchPtrBitSerialize<'a>,
    scratch_ptr_set_to_next: sims_sim::ScratchPtrBitSerialize<'a>,
    scratch_ptr_expression: sims_sim::ScratchPtrBitSerialize<'a>,
    scratch_ptr_arg_slot: sims_sim::ScratchPtrBitSerialize<'a>,
    runtime_added_suits: &'a Vec<String>,
    suit_scan_cache: &'a Option<Vec<String>>,
    family_flags: &'a BTreeMap<i32, u32>,
    activity_owners: Option<&'a BTreeMap<u16, PlayerId>>,
    next_dialog_id: Option<u64>,
    active_dialog: Option<DialogId>,
    active_special_panel: Option<(DialogId, &'a ShownSpecialPanel)>,
    frozen_dialog_digest: Option<[u8; 32]>,
    pending_speed_sync: bool,
    /// The people the next draw refreshes the indicators of: the lot
    /// snapshot carries them (`Saved` format 11).
    indicator_queues: &'a sims_sim::IndicatorQueues,
}

/// Writes fixed session globals (`Free Will` 30, `Machine Level` 33,
/// `Current Language` 15) from `session` into `world.globals`.
pub(crate) fn sync_session_globals(world: &mut World, session: &SessionState) {
    if let Some(slot) = world.globals.get_mut(GLOBAL_FREE_WILL) {
        *slot = i16::from(session.config.free_will);
    }
    if let Some(slot) = world.globals.get_mut(GLOBAL_MACHINE_LEVEL) {
        *slot = session.config.machine_level;
    }
    if let Some(slot) = world.globals.get_mut(GLOBAL_LANGUAGE) {
        *slot = session.config.sim_language;
    }
}

fn sync_pause_global(world: &mut World, paused: bool) {
    if let Some(slot) = world.globals.get_mut(GLOBAL_PAUSED) {
        *slot = i16::from(paused);
    }
}

/// Applies one non-dialog/non-selection [`StampedCommand`] to `session`
/// matching [`SessionState::apply_batch`] semantics in exact stamped order.
fn apply_session_command(
    session: &mut SessionState,
    stamped: &StampedCommand,
) -> Result<(), CoopCommandStatus> {
    let issuer = stamped.issuer;
    match &stamped.command {
        Command::PlayerJoined {
            player_id,
            name,
            selected_sim,
            camera_rotation,
            endpoint_hint,
            transport_addr,
        } => {
            if issuer != session.host_id {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::HostOnly,
                ));
            }
            if session.players.contains_key(player_id) {
                return Err(CoopCommandStatus::IgnoredBySession(
                    CommandIgnoreReason::RedundantMembershipChange,
                ));
            }
            if player_id.0 >= session.next_player_id {
                session.next_player_id = player_id.0.saturating_add(1);
            }
            let canonical_transport = transport_addr.clone().or_else(|| {
                endpoint_hint.as_deref().map(|hint| {
                    InviteTicket::decode_string(hint)
                        .map(|ticket| ticket.endpoint_addr().id.to_string())
                        .unwrap_or_else(|_| hint.to_string())
                })
            });
            session.players.insert(
                *player_id,
                sims_net::PerPlayerState {
                    player_id: *player_id,
                    name: name.clone(),
                    role: PlayerRole::Peer,
                    mode: PlayerMode::Live,
                    selected_sim: *selected_sim,
                    camera_rotation: camera_rotation.normalized(),
                    endpoint_hint: endpoint_hint.clone(),
                    transport_addr: canonical_transport,
                },
            );
            Ok(())
        }
        Command::PlayerLeft { player_id, .. } => {
            if issuer != session.host_id {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::HostOnly,
                ));
            }
            if session.players.remove(player_id).is_none() {
                return Err(CoopCommandStatus::IgnoredBySession(
                    CommandIgnoreReason::RedundantMembershipChange,
                ));
            }
            Ok(())
        }
        Command::HostChanged { new_host, epoch } => {
            if issuer != session.host_id && issuer != *new_host {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::HostOnly,
                ));
            }
            if !session.players.contains_key(new_host) {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::UnknownIssuer,
                ));
            }
            for player in session.players.values_mut() {
                player.role = if player.player_id == *new_host {
                    PlayerRole::Host
                } else {
                    PlayerRole::Peer
                };
            }
            session.host_id = *new_host;
            if *epoch > session.epoch {
                session.epoch = *epoch;
            }
            Ok(())
        }
        Command::SetHostSpeedLock { locked } => {
            if issuer != session.host_id {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::HostOnly,
                ));
            }
            session.time.host_speed_lock = *locked;
            session.config.host_speed_lock = *locked;
            Ok(())
        }
        Command::SetSpeed { speed } => {
            if !session.players.contains_key(&issuer) {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::UnknownIssuer,
                ));
            }
            if session.time.host_speed_lock && issuer != session.host_id {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::SpeedLockedByHost,
                ));
            }
            session.time.speed = SimSpeed::new(speed.value());
            session.time.last_speed_change_by = Some(issuer);
            Ok(())
        }
        Command::SetPaused { paused } => {
            if !session.players.contains_key(&issuer) {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::UnknownIssuer,
                ));
            }
            if session.time.host_speed_lock && issuer != session.host_id {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::SpeedLockedByHost,
                ));
            }
            session.time.explicit_pause = *paused;
            session.time.explicit_pause_by = Some(issuer);
            Ok(())
        }
        Command::SetFreeWill { enabled } => {
            if !session.players.contains_key(&issuer) {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::UnknownIssuer,
                ));
            }
            session.config.free_will = *enabled;
            Ok(())
        }
        Command::SetCameraRotation { rotation } => {
            let Some(player) = session.players.get_mut(&issuer) else {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::UnknownIssuer,
                ));
            };
            player.camera_rotation = rotation.normalized();
            Ok(())
        }
        Command::SetMode { mode } => {
            let Some(player) = session.players.get_mut(&issuer) else {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::UnknownIssuer,
                ));
            };
            player.mode = *mode;
            Ok(())
        }
        Command::SelectPerson { .. }
        | Command::AnswerBlockingDialog { .. }
        | Command::SimActionPick { .. }
        | Command::SimActionCancel { .. }
        | Command::BuyBuild(_) => {
            if !session.players.contains_key(&issuer) {
                return Err(CoopCommandStatus::RejectedBySession(
                    CommandRejectReason::UnknownIssuer,
                ));
            }
            Ok(())
        }
    }
}

/// Resolves a 16-bit wire cancel UID to the exact `i32` UID on `sim_id`'s
/// active/queued interactions (at most 9 actions), supporting `action_serial >
/// 32767` cleanly while deterministically refusing any ambiguous 16-bit wrap
/// collision.
fn resolve_cancel_uid(world: &World, sim_id: u16, wire_uid: i16) -> Result<Option<i32>, String> {
    let Some(person) = world.objects.get(&sim_id).and_then(|o| o.person.as_ref()) else {
        return Err(format!("object {sim_id} is not a person on the lot"));
    };
    let mut matches = Vec::new();
    let mut check = |uid: i32| {
        if uid != 0 && (uid as i16) == wire_uid && !matches.contains(&uid) {
            matches.push(uid);
        }
    };
    check(person.active.uid);
    for queued in &person.queue {
        check(queued.uid);
    }
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches[0])),
        _ => Err(format!(
            "ambiguous 16-bit cancel UID {wire_uid} matches multiple interactions {matches:?} on \
             Sim {sim_id}"
        )),
    }
}

impl Lot {
    /// Seeds a freshly created [`SessionState`] from this loaded lot's initial
    /// host selection, camera rotation, speed, and fixed session globals
    /// (`Free Will`, `Current Language`, `Machine Level`).
    ///
    /// Calling this on the host before starting a co-op session ensures the
    /// initial host state matches single-player load state with zero selection
    /// or speed churn on tick 1.
    pub fn sync_initial_coop_session(&self, session: &mut SessionState) {
        if let Some(host) = session.players.get_mut(&session.host_id) {
            host.selected_sim = self.world.selected();
            host.camera_rotation = CameraRotation::new(self.view_rotation);
        }
        session.config.free_will = self
            .world
            .globals
            .get(GLOBAL_FREE_WILL)
            .copied()
            .unwrap_or(1)
            != 0;
        session.config.sim_language = self
            .world
            .globals
            .get(GLOBAL_LANGUAGE)
            .copied()
            .unwrap_or(1);
        session.config.machine_level = self
            .world
            .globals
            .get(GLOBAL_MACHINE_LEVEL)
            .copied()
            .unwrap_or(2);
        session.time.speed = SimSpeed::new(self.world.speed());
    }

    /// Builds default co-op [`BuildPrices`] from this lot's loaded surface
    /// catalogs (`self.surfaces`) and built-in wall style prices.
    ///
    /// For full built-in floor/wallpaper pricing from `GameData/Build.iff`,
    /// construct [`BuildPrices`] via [`build_prices_from_install`] and pass it
    /// to [`CoopVmContext::with_build_prices`].
    pub fn default_coop_build_prices(&self) -> BuildPrices {
        default_build_prices_from_surfaces(&self.surfaces)
    }

    /// Attaches a co-op [`SessionState`] to this lot, installing the
    /// authoritative co-op [`Frames`] schedule, suppressing single-player
    /// Tutorial/Help System controllers, enabling interactive blocking dialog
    /// waits (`Lot::wait_for_answers`), and synchronizing fixed session globals,
    /// host camera rotation, host selection, mode, pause, and speed.
    ///
    /// Note: without an [`Install`] handle, built-in floor/wallpaper prices from
    /// `GameData/Build.iff` are not populated (only plug-in `.flr`/`.wll` prices
    /// and wall style prices are loaded), and co-op Build edits referencing
    /// unpriced built-in patterns are refused deterministically. For full
    /// install-backed pricing, use [`Self::attach_coop_with_install`].
    pub fn attach_coop(&mut self, session: SessionState) -> Result<(), OrderError> {
        self.attach_coop_context(CoopVmContext::new(session))
    }

    /// Attaches a co-op [`SessionState`] to this lot with full catalog
    /// [`BuildPrices`] loaded from `install` (`GameData/Build.iff`) and
    /// `self.surfaces`.
    pub fn attach_coop_with_install(
        &mut self,
        session: SessionState,
        install: &Install,
    ) -> Result<(), OrderError> {
        let prices = build_prices_from_install(install, &self.surfaces);
        self.attach_coop_context(CoopVmContext::new(session).with_build_prices(prices))
    }

    /// Attaches a preconfigured [`CoopVmContext`] to this lot.
    pub fn attach_coop_context(&mut self, mut ctx: CoopVmContext) -> Result<(), OrderError> {
        self.ready()?;
        if self
            .world
            .globals
            .get(GLOBAL_TUTORIAL)
            .copied()
            .unwrap_or(0)
            != 0
        {
            return Err(OrderError::Refused(crate::Fault {
                object: 0,
                at: None,
                trap: crate::Trap::Invalid(
                    "tutorial lots are single-player only and cannot be attached in co-op mode"
                        .into(),
                ),
            }));
        }
        if ctx.build_prices.styles.is_empty()
            && ctx.build_prices.floors.is_empty()
            && ctx.build_prices.walls.is_empty()
        {
            ctx.build_prices = self.default_coop_build_prices();
        }
        self.wait_for_answers();
        self.frames = ctx.frames.clone();
        self.disable_coop_tutorial_and_help();
        self.sanitize_session_selections(&mut ctx.session);
        self.sync_world_from_session(&mut ctx, true)?;
        self.coop = Some(ctx);
        Ok(())
    }

    /// Deterministically parks single-player Tutorial and Help System
    /// controllers (non-person objects whose linked library uses opcode 10
    /// `Tutorial` or opcode 34 `UI Effect`, per `docs/vision.md` line 153) so
    /// single-player help prompts never pause a co-op lot.
    fn disable_coop_tutorial_and_help(&mut self) {
        let mut parked_ids = BTreeSet::new();
        for (&id, obj) in &self.world.objects {
            if obj.person.is_some() {
                continue;
            }
            let Some(def) = self.catalog.object(obj.guid) else {
                continue;
            };
            let Some(library) = self.program.libraries.get(&def.resource) else {
                continue;
            };
            let is_tutorial_or_help = library.behaviors.values().any(|beh| {
                beh.instructions
                    .iter()
                    .any(|ins| ins.opcode == OPCODE_TUTORIAL || ins.opcode == OPCODE_UI_EFFECT)
            });
            if is_tutorial_or_help {
                parked_ids.insert(id);
            }
        }
        if parked_ids.is_empty() {
            return;
        }
        for &id in &parked_ids {
            if let Some(obj) = self.world.objects.get_mut(&id) {
                obj.set_flags(SCHEDULER_READY, false);
            }
            if self.world.event_owner == id {
                self.world.event_owner = 0;
            }
        }
        self.world
            .dialogs
            .records
            .retain(|rec| !parked_ids.contains(&rec.caller));
    }

    /// Detaches and returns the active [`CoopVmContext`], restoring
    /// single-player execution (`coop == None`).
    pub fn detach_coop(&mut self) -> Option<CoopVmContext> {
        self.coop.take()
    }

    /// Read-only access to the attached [`CoopVmContext`], if any.
    pub fn coop(&self) -> Option<&CoopVmContext> {
        self.coop.as_ref()
    }

    /// Mutable access to the attached [`CoopVmContext`], if any.
    pub fn coop_mut(&mut self) -> Option<&mut CoopVmContext> {
        self.coop.as_mut()
    }

    /// Executes an authoritative [`AppliedBatch`] (produced by
    /// `LockstepNode::step_next_tick`) against this lot in exact stamped
    /// command order, validating tick and epoch continuity and keeping
    /// `session` synchronized with any VM-validated selections, dialog
    /// answers, script-driven selection changes, or newly opened blocking
    /// modal dialogs.
    pub fn apply_coop_batch(
        &mut self,
        batch: &AppliedBatch,
        session: &mut SessionState,
    ) -> Result<CoopBatchReport, CoopError> {
        let Some(mut ctx) = self.coop.take() else {
            return Err(CoopError::NotAttached);
        };

        // Validate tick and epoch continuity so duplicate or out-of-order
        // externally supplied batches can never execute twice.
        let expected_tick = ctx.session.current_tick.next();
        if batch.tick != expected_tick {
            self.coop = Some(ctx);
            return Err(CoopError::Session(SessionError::UnexpectedTick {
                expected: expected_tick,
                actual: batch.tick,
            }));
        }
        if session.current_tick != batch.tick {
            self.coop = Some(ctx);
            return Err(CoopError::Session(SessionError::UnexpectedTick {
                expected: batch.tick,
                actual: session.current_tick,
            }));
        }
        let batch_has_host_changed = batch
            .commands
            .iter()
            .any(|c| matches!(c.stamped.command, Command::HostChanged { .. }));
        if batch.epoch < ctx.session.epoch {
            let expected = ctx.session.epoch;
            self.coop = Some(ctx);
            return Err(CoopError::Session(SessionError::EpochMismatch {
                expected,
                actual: batch.epoch,
            }));
        }
        if batch.epoch > session.epoch || (!batch_has_host_changed && batch.epoch != session.epoch)
        {
            let expected = session.epoch;
            self.coop = Some(ctx);
            return Err(CoopError::Session(SessionError::EpochMismatch {
                expected,
                actual: batch.epoch,
            }));
        }
        // Reconcile out-of-band host failover (`LockstepNode` host promotion)
        // that may have advanced `session.epoch`, changed `host_id`, and
        // removed unreachable failover candidates before `batch` ran. Reject
        // arbitrary future epochs that are not backed by an actual failover to
        // a known surviving player (or an in-band `HostChanged`).
        if !batch_has_host_changed && session.epoch > ctx.session.epoch {
            let old_host = ctx.session.host_id;
            if session.host_id == old_host || !ctx.session.players.contains_key(&session.host_id) {
                let expected = ctx.session.epoch;
                self.coop = Some(ctx);
                return Err(CoopError::Session(SessionError::EpochMismatch {
                    expected,
                    actual: session.epoch,
                }));
            }
            let _ = ctx.session.promote_host(session.host_id);
            ctx.activity_owners.retain(|_, owner| *owner != old_host);
            // Also remove any unreachable candidates dropped by lockstep
            // failover (preserving players removed by an in-batch `PlayerLeft`
            // so sequential batch replay processes them in exact order).
            let batch_left_players: BTreeSet<PlayerId> = batch
                .commands
                .iter()
                .filter_map(|c| match c.stamped.command {
                    Command::PlayerLeft { player_id, .. } => Some(player_id),
                    _ => None,
                })
                .collect();
            let failover_removed: Vec<PlayerId> = ctx
                .session
                .players
                .keys()
                .copied()
                .filter(|pid| {
                    !session.players.contains_key(pid) && !batch_left_players.contains(pid)
                })
                .collect();
            for gone in failover_removed {
                ctx.session.players.remove(&gone);
                ctx.activity_owners.retain(|_, owner| *owner != gone);
            }
            ctx.session.epoch = session.epoch;
        } else if batch_has_host_changed && batch.epoch > ctx.session.epoch {
            let old_host = ctx.session.host_id;
            let pre_batch_host = batch
                .commands
                .iter()
                .find_map(|c| match c.stamped.command {
                    Command::HostChanged { .. }
                        if c.stamped.issuer != old_host
                            && ctx.session.players.contains_key(&c.stamped.issuer) =>
                    {
                        Some(c.stamped.issuer)
                    }
                    _ => None,
                })
                .unwrap_or(session.host_id);
            if pre_batch_host == old_host || !ctx.session.players.contains_key(&pre_batch_host) {
                let expected = ctx.session.epoch;
                self.coop = Some(ctx);
                return Err(CoopError::Session(SessionError::EpochMismatch {
                    expected,
                    actual: batch.epoch,
                }));
            }
            let _ = ctx.session.promote_host(pre_batch_host);
            ctx.activity_owners.retain(|_, owner| *owner != old_host);
            let batch_left_players: BTreeSet<PlayerId> = batch
                .commands
                .iter()
                .filter_map(|c| match c.stamped.command {
                    Command::PlayerLeft { player_id, .. } => Some(player_id),
                    _ => None,
                })
                .collect();
            let failover_removed: Vec<PlayerId> = ctx
                .session
                .players
                .keys()
                .copied()
                .filter(|pid| {
                    !session.players.contains_key(pid) && !batch_left_players.contains(pid)
                })
                .collect();
            for gone in failover_removed {
                ctx.session.players.remove(&gone);
                ctx.activity_owners.retain(|_, owner| *owner != gone);
            }
            ctx.session.epoch = batch.epoch;
        }

        let result = self.execute_coop_batch(batch, &mut ctx, session);
        self.coop = Some(ctx);
        result
    }

    /// Convenience wrapper when this [`Lot`] owns its [`SessionState`]
    /// directly: applies `batch` to `self.coop.session` and executes it.
    pub fn apply_coop_tick_batch(
        &mut self,
        batch: &TickBatch,
    ) -> Result<CoopBatchReport, CoopError> {
        let Some(mut ctx) = self.coop.take() else {
            return Err(CoopError::NotAttached);
        };
        let mut session = ctx.session.clone();
        let applied = match session.apply_batch(batch) {
            Ok(applied) => applied,
            Err(err) => {
                self.coop = Some(ctx);
                return Err(CoopError::Session(err));
            }
        };
        let result = self.execute_coop_batch(&applied, &mut ctx, &mut session);
        ctx.session = session;
        self.coop = Some(ctx);
        result
    }

    /// Whether the lot is currently waiting on either an ordinary modal dialog
    /// or a special panel (`Dialog` kinds `5..=15`).
    fn has_pending_dialog(&self) -> bool {
        self.asking.is_some() || self.asking_special_panel.is_some()
    }

    fn execute_coop_batch(
        &mut self,
        batch: &AppliedBatch,
        ctx: &mut CoopVmContext,
        session: &mut SessionState,
    ) -> Result<CoopBatchReport, CoopError> {
        // Ensure local `self.frames` always matches the authoritative co-op
        // frame schedule before running commands or stepping.
        self.frames = ctx.frames.clone();

        // Start from the exact pre-batch session and replay every command
        // sequentially in host-stamped order so early commands never observe
        // future commands in the same batch and VM-rejected selections/dialog
        // answers never clobber valid state.
        let mut running_session = ctx.session.clone();
        ctx.session = running_session.clone();

        let mut command_results = Vec::with_capacity(batch.commands.len());
        let mut script_changes: BTreeMap<PlayerId, Option<u16>> = BTreeMap::new();

        for item in &batch.commands {
            let stamped = &item.stamped;
            let status = self.execute_single_coop_command(
                batch.tick,
                stamped,
                ctx,
                &mut running_session,
                &mut script_changes,
            );
            command_results.push((stamped.clone(), status));
        }

        running_session.current_tick = batch.tick;
        self.sanitize_session_selections(&mut running_session);
        ctx.session = running_session.clone();

        // Sync globals, mode, pause, host camera, and host plumbob when not
        // frozen on a dialog tick (best-effort so callback errors never strand
        // VM tick continuity behind lockstep).
        if ctx.frozen_dialog.is_none() && !self.has_pending_dialog() && !self.answered {
            let apply_speed = ctx.pending_speed_sync;
            let before_sync = snapshot_selections(&ctx.session);
            self.sync_world_from_session_best_effort(ctx, apply_speed);
            record_selection_deltas(&before_sync, &ctx.session, &mut script_changes);
            running_session = ctx.session.clone();
            ctx.pending_speed_sync = false;
        }

        let mut stepped = false;
        let mut update = None;
        let mut opened_dialog = None;
        let mut opened_special_panel = None;

        if !running_session.is_paused() && !self.has_pending_dialog() {
            let play_mode = ctx.play_mode;
            let was_frozen = ctx.frozen_dialog.is_some();
            let frozen = ctx
                .frozen_dialog
                .clone()
                .unwrap_or_else(|| FrozenDialogTick {
                    session: running_session.clone(),
                    activity_owners: ctx.activity_owners.clone(),
                    free_will: running_session.config.free_will,
                    intervening_selections: BTreeSet::new(),
                });

            // Build the exact step context from `frozen` so live replay and
            // snapshot restore execute with bit-for-bit identical pre-tick
            // inputs (session, activity_owners, free_will).
            let mut step_ctx = ctx.clone();
            step_ctx.session = frozen.session.clone();
            step_ctx.activity_owners = frozen.activity_owners.clone();
            if let Some(slot) = self.world.globals.get_mut(GLOBAL_FREE_WILL) {
                *slot = i16::from(frozen.free_will);
            }

            let pre_step_selections = snapshot_selections(&step_ctx.session);

            // Install `step_ctx` on `self.coop` so `step_in` threads it into
            // `Interpreter` and rolls it back atomically on `Stop::Asks`.
            self.coop = Some(step_ctx);
            let step_result = self.step_in(play_mode);
            let updated_step_ctx = self.coop.take().unwrap_or_else(|| ctx.clone());

            match step_result {
                Ok(upd) => {
                    stepped = true;
                    ctx.frozen_dialog = None;

                    if was_frozen {
                        // Explicit merge policy after replaying an interrupted
                        // dialog tick:
                        // 1. Apply any new activity_owners created during the
                        //    replayed tick that weren't removed by departures.
                        for (&obj_id, &owner) in &updated_step_ctx.activity_owners {
                            if running_session.players.contains_key(&owner)
                                && !ctx.activity_owners.contains_key(&obj_id)
                            {
                                ctx.activity_owners.insert(obj_id, owner);
                            }
                        }
                        // 2. For every player whose selection was rewritten by
                        //    a script during the replayed tick, apply the
                        //    script's new selection ONLY if that player did not
                        //    issue an accepted `SelectPerson` while the tick
                        //    was frozen (even if they reselected the original
                        //    Sim or switched away and back).
                        for (&pid, post_player) in &updated_step_ctx.session.players {
                            let pre_sel = pre_step_selections.get(&pid).copied().flatten();
                            let post_sel = post_player.selected_sim;
                            if post_sel != pre_sel
                                && !frozen.intervening_selections.contains(&pid)
                                && let Some(live_player) = running_session.players.get_mut(&pid)
                            {
                                live_player.selected_sim = post_sel;
                                script_changes.insert(pid, post_sel);
                            }
                        }
                    } else {
                        ctx.activity_owners = updated_step_ctx.activity_owners;
                        record_selection_deltas(
                            &pre_step_selections,
                            &updated_step_ctx.session,
                            &mut script_changes,
                        );
                        running_session = updated_step_ctx.session;
                    }

                    // Prune deleted objects from both `ctx.activity_owners` and
                    // `running_session` selections.
                    let before_prune = snapshot_selections(&running_session);
                    ctx.session = running_session.clone();
                    for &removed_id in &upd.removed {
                        ctx.remove_object(removed_id);
                    }
                    running_session = ctx.session.clone();
                    self.sanitize_session_selections(&mut running_session);
                    record_selection_deltas(&before_prune, &running_session, &mut script_changes);
                    ctx.session = running_session.clone();

                    // Reconcile all post-replay / intervening session state
                    // (`FreeWill`, `Mode`, `Pause`, `HostCamera`,
                    // `HostPlumbob`, plus `Speed` ONLY if an explicit
                    // `Command::SetSpeed` arrived while frozen so residential
                    // idle-speed transitions from the replayed tick are never
                    // overwritten).
                    let apply_speed = ctx.pending_speed_sync;
                    let before_post = snapshot_selections(&ctx.session);
                    self.sync_world_from_session_best_effort(ctx, apply_speed);
                    record_selection_deltas(&before_post, &ctx.session, &mut script_changes);
                    running_session = ctx.session.clone();
                    ctx.pending_speed_sync = false;

                    update = Some(upd);
                }
                Err(StepError::Asking { .. }) => {
                    if let Some(asking) = self.asking.as_ref() {
                        let dialog_id = DialogId(ctx.next_dialog_id);
                        ctx.next_dialog_id = ctx.next_dialog_id.saturating_add(1);
                        ctx.active_dialog = Some(dialog_id);
                        ctx.active_special_panel = None;
                        // Preserve the ORIGINAL frozen pre-tick context across
                        // every dialog or special panel in this interrupted tick.
                        if ctx.frozen_dialog.is_none() {
                            ctx.frozen_dialog = Some(frozen);
                        }
                        let shown = asking.shown.clone();
                        running_session.open_blocking_dialog(dialog_id, shown.object, batch.tick);
                        ctx.session = running_session.clone();
                        opened_dialog = Some((dialog_id, shown));
                    } else if let Some(asking_sp) = self.asking_special_panel.as_ref() {
                        let dialog_id = DialogId(ctx.next_dialog_id);
                        ctx.next_dialog_id = ctx.next_dialog_id.saturating_add(1);
                        let shown = asking_sp.shown.clone();
                        ctx.active_dialog = None;
                        ctx.active_special_panel = Some((dialog_id, shown.clone()));
                        if ctx.frozen_dialog.is_none() {
                            ctx.frozen_dialog = Some(frozen);
                        }
                        running_session.open_blocking_dialog(
                            dialog_id,
                            shown.request.caller_object().unwrap_or(0),
                            batch.tick,
                        );
                        ctx.session = running_session.clone();
                        opened_special_panel = Some((dialog_id, shown));
                    }
                }
                Err(err) => {
                    *session = running_session;
                    ctx.session = session.clone();
                    return Err(CoopError::Step(err));
                }
            }
        }

        // Write our exact sequentially-validated session back to the caller.
        *session = running_session;
        ctx.session = session.clone();

        let script_selection_changes: Vec<(PlayerId, Option<u16>)> =
            script_changes.into_iter().collect();

        // Install `ctx` briefly so `coop_world_hash` includes all `ctx` metadata.
        self.coop = Some(ctx.clone());
        let world_hash = self.coop_world_hash();
        let _ = self.coop.take();
        let state_fingerprint = session.combined_fingerprint(world_hash);

        Ok(CoopBatchReport {
            tick: batch.tick,
            stepped,
            update,
            opened_dialog,
            opened_special_panel,
            command_results,
            script_selection_changes,
            world_hash,
            state_fingerprint,
        })
    }

    fn execute_single_coop_command(
        &mut self,
        tick: Tick,
        stamped: &StampedCommand,
        ctx: &mut CoopVmContext,
        session: &mut SessionState,
        script_changes: &mut BTreeMap<PlayerId, Option<u16>>,
    ) -> CoopCommandStatus {
        let issuer = stamped.issuer;
        match &stamped.command {
            Command::SelectPerson { sim_id } => {
                if !session.players.contains_key(&issuer) {
                    return CoopCommandStatus::RejectedBySession(
                        CommandRejectReason::UnknownIssuer,
                    );
                }
                if let Some(id) = *sim_id
                    && !self
                        .world
                        .objects
                        .get(&id)
                        .is_some_and(|o| o.person.is_some())
                {
                    // Validate BEFORE mutating `session` so an invalid
                    // selection never clobbers a prior valid selection.
                    return CoopCommandStatus::Refused(format!(
                        "object {id} is not a person on the lot"
                    ));
                }
                let prev_sim = session.players.get(&issuer).and_then(|p| p.selected_sim);
                if let Some(player) = session.players.get_mut(&issuer) {
                    player.selected_sim = *sim_id;
                }
                ctx.session = session.clone();
                if ctx.frozen_dialog.is_none()
                    && !self.has_pending_dialog()
                    && !self.answered
                    && issuer == session.host_id
                {
                    let before = snapshot_selections(&ctx.session);
                    if let Err(err) = self.sync_host_plumbob_with_coop(ctx) {
                        // Roll back the failed host selection atomically so a
                        // callback fault (e.g. occupied indicator slot) never
                        // strands batch tick continuity.
                        if let Some(player) = session.players.get_mut(&issuer) {
                            player.selected_sim = prev_sim;
                        }
                        ctx.session = session.clone();
                        return CoopCommandStatus::Refused(err.to_string());
                    }
                    record_selection_deltas(&before, &ctx.session, script_changes);
                    *session = ctx.session.clone();
                } else if let Some(frozen) = ctx.frozen_dialog.as_mut() {
                    frozen.intervening_selections.insert(issuer);
                }
                CoopCommandStatus::Applied
            }
            Command::AnswerBlockingDialog { dialog_id, answer } => {
                if !session.players.contains_key(&issuer) {
                    return CoopCommandStatus::RejectedBySession(
                        CommandRejectReason::UnknownIssuer,
                    );
                }
                if !session.open_dialogs.contains_key(dialog_id) {
                    return CoopCommandStatus::IgnoredBySession(
                        CommandIgnoreReason::DialogAlreadyAnswered,
                    );
                }
                if ctx
                    .active_special_panel
                    .as_ref()
                    .is_some_and(|(id, _)| id == dialog_id)
                {
                    if self.asking_special_panel.is_none() {
                        ctx.active_special_panel = None;
                        session.open_dialogs.remove(dialog_id);
                        ctx.session = session.clone();
                        return CoopCommandStatus::Refused(
                            "lot is not waiting on a special panel".to_string(),
                        );
                    }
                    let response = match dialog_answer_to_special_panel_response(answer) {
                        Ok(response) => response,
                        Err(msg) => return CoopCommandStatus::Refused(msg),
                    };
                    return match self.answer_special_panel(response) {
                        Ok(()) => {
                            session.open_dialogs.remove(dialog_id);
                            ctx.active_special_panel = None;
                            ctx.session = session.clone();
                            CoopCommandStatus::DialogAnswered {
                                dialog_id: *dialog_id,
                                winner: issuer,
                            }
                        }
                        Err(SpecialPanelAnswerError::WrongResponse {
                            expected_kind,
                            given,
                        }) => CoopCommandStatus::Refused(format!(
                            "special panel {dialog_id} (kind {expected_kind}) does not accept {given:?}"
                        )),
                        Err(err) => CoopCommandStatus::Refused(err.to_string()),
                    };
                }
                if ctx.active_dialog != Some(*dialog_id) {
                    return CoopCommandStatus::Refused(format!(
                        "no active blocking dialog matching {dialog_id}"
                    ));
                }
                let Some(asking) = self.asking.as_ref() else {
                    ctx.active_dialog = None;
                    session.open_dialogs.remove(dialog_id);
                    ctx.session = session.clone();
                    return CoopCommandStatus::Refused(
                        "lot is not waiting on a dialog".to_string(),
                    );
                };
                // Validate button status BEFORE removing `dialog_id` from
                // `session.open_dialogs` so an invalid answer leaves the dialog
                // open for a later valid answer in the SAME batch!
                let offered = asking.shown.answers;
                if !offered.contains(&answer.status) {
                    return CoopCommandStatus::Refused(format!(
                        "dialog {dialog_id} offers {offered:?}, not {}",
                        answer.status
                    ));
                }
                let vm_answer = Answer {
                    status: answer.status,
                    bitmap_id: answer.bitmap_id,
                    text: answer.text.clone(),
                };
                match self.answer(vm_answer) {
                    Ok(()) => {
                        session.open_dialogs.remove(dialog_id);
                        ctx.active_dialog = None;
                        ctx.session = session.clone();
                        CoopCommandStatus::DialogAnswered {
                            dialog_id: *dialog_id,
                            winner: issuer,
                        }
                    }
                    Err(AnswerError::NotOffered { answer, offered }) => CoopCommandStatus::Refused(
                        format!("dialog {dialog_id} offers {offered:?}, not {answer}"),
                    ),
                    Err(err) => CoopCommandStatus::Refused(err.to_string()),
                }
            }
            Command::SetCameraRotation { rotation } => {
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                ctx.session = session.clone();
                if issuer == session.host_id
                    && ctx.frozen_dialog.is_none()
                    && !self.has_pending_dialog()
                    && !self.answered
                {
                    self.set_view_rotation(rotation.quarter_turns());
                }
                CoopCommandStatus::Applied
            }
            Command::SetMode { .. } => {
                let prev_mode = session.players.get(&issuer).map(|p| p.mode);
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                ctx.session = session.clone();
                if ctx.frozen_dialog.is_none() && !self.has_pending_dialog() && !self.answered {
                    let target_mode = player_mode_to_panel_mode(session.effective_mode());
                    if self.mode() != target_mode {
                        let before = snapshot_selections(&ctx.session);
                        if let Err(err) = self.set_mode_with_coop(target_mode, ctx) {
                            if let (Some(prev), Some(player)) =
                                (prev_mode, session.players.get_mut(&issuer))
                            {
                                player.mode = prev;
                            }
                            ctx.session = session.clone();
                            return CoopCommandStatus::Refused(err.to_string());
                        }
                        record_selection_deltas(&before, &ctx.session, script_changes);
                        *session = ctx.session.clone();
                    }
                    sync_pause_global(&mut self.world, session.is_paused());
                }
                CoopCommandStatus::Applied
            }
            Command::SetPaused { .. } => {
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                ctx.session = session.clone();
                if ctx.frozen_dialog.is_none() && !self.has_pending_dialog() && !self.answered {
                    sync_pause_global(&mut self.world, session.is_paused());
                }
                CoopCommandStatus::Applied
            }
            Command::SetSpeed { .. } => {
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                ctx.session = session.clone();
                if ctx.frozen_dialog.is_none() && !self.has_pending_dialog() && !self.answered {
                    self.world.set_speed(i32::from(session.time.speed.value()));
                    ctx.pending_speed_sync = false;
                } else {
                    ctx.pending_speed_sync = true;
                }
                CoopCommandStatus::Applied
            }
            Command::SetFreeWill { .. } => {
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                ctx.session = session.clone();
                // While a blocking dialog tick is frozen, keep pre-tick
                // `world.globals[30]` untouched so replay and snapshot probes
                // run the exact tick that asked; post-replay sync writes the
                // updated `session.config.free_will`.
                if ctx.frozen_dialog.is_none()
                    && !self.has_pending_dialog()
                    && !self.answered
                    && let Err(err) = self.set_free_will(session.config.free_will)
                {
                    return CoopCommandStatus::Refused(err.to_string());
                }
                CoopCommandStatus::Applied
            }
            Command::SetHostSpeedLock { .. } => {
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                ctx.session = session.clone();
                CoopCommandStatus::Applied
            }
            Command::PlayerJoined { player_id, .. } => {
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                if let Some(player) = session.players.get_mut(player_id)
                    && let Some(sim_id) = player.selected_sim
                    && !self
                        .world
                        .objects
                        .get(&sim_id)
                        .is_some_and(|o| o.person.is_some())
                {
                    player.selected_sim = None;
                }
                ctx.session = session.clone();
                CoopCommandStatus::Applied
            }
            Command::PlayerLeft { player_id, .. } => {
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                ctx.activity_owners.retain(|_, owner| owner != player_id);
                ctx.session = session.clone();
                if ctx.frozen_dialog.is_none() && !self.has_pending_dialog() && !self.answered {
                    let target_mode = player_mode_to_panel_mode(session.effective_mode());
                    if self.mode() != target_mode {
                        let before = snapshot_selections(&ctx.session);
                        if self.set_mode_with_coop(target_mode, ctx).is_ok() {
                            record_selection_deltas(&before, &ctx.session, script_changes);
                            *session = ctx.session.clone();
                        }
                    }
                    sync_pause_global(&mut self.world, session.is_paused());
                }
                CoopCommandStatus::Applied
            }
            Command::HostChanged { .. } => {
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                ctx.session = session.clone();
                if ctx.frozen_dialog.is_none() && !self.has_pending_dialog() && !self.answered {
                    self.set_view_rotation(ctx.host_camera_rotation());
                    let before = snapshot_selections(&ctx.session);
                    if self.sync_host_plumbob_with_coop(ctx).is_ok() {
                        record_selection_deltas(&before, &ctx.session, script_changes);
                        *session = ctx.session.clone();
                    } else if let Some(slot) = self.world.globals.get_mut(GLOBAL_SELECTED) {
                        *slot = ctx
                            .session
                            .players
                            .get(&ctx.session.host_id)
                            .and_then(|p| p.selected_sim)
                            .map_or(0, |id| id as i16);
                    }
                }
                CoopCommandStatus::Applied
            }
            Command::SimActionPick {
                sim_id,
                target_id,
                menu_index,
                joining,
                args,
            } => {
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                if ctx.frozen_dialog.is_some() || self.has_pending_dialog() || self.answered {
                    return CoopCommandStatus::IgnoredWhileDialogPending;
                }
                if self.mode() != PanelMode::Live {
                    return CoopCommandStatus::Refused(
                        "cannot queue Sim actions while lot is in Buy/Build mode".into(),
                    );
                }
                let Ok(menu) = u32::try_from(*menu_index) else {
                    return CoopCommandStatus::Refused(format!("negative menu index {menu_index}"));
                };
                ctx.session = session.clone();
                let before = snapshot_selections(&ctx.session);
                let status =
                    self.execute_coop_pick(issuer, *sim_id, *target_id, menu, *joining, *args, ctx);
                record_selection_deltas(&before, &ctx.session, script_changes);
                *session = ctx.session.clone();
                status
            }
            Command::SimActionCancel {
                sim_id,
                interaction_uid,
            } => {
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                if ctx.frozen_dialog.is_some() || self.has_pending_dialog() || self.answered {
                    return CoopCommandStatus::IgnoredWhileDialogPending;
                }
                if self.mode() != PanelMode::Live {
                    return CoopCommandStatus::Refused(
                        "cannot cancel Sim actions while lot is in Buy/Build mode".into(),
                    );
                }
                let resolved_uid = match resolve_cancel_uid(&self.world, *sim_id, *interaction_uid)
                {
                    Ok(Some(uid)) => uid,
                    Ok(None) => {
                        return CoopCommandStatus::CancelledInteraction { counted: false };
                    }
                    Err(msg) => return CoopCommandStatus::Refused(msg),
                };
                ctx.session = session.clone();
                let before = snapshot_selections(&ctx.session);
                let status = match self.cancel_with_coop(*sim_id, resolved_uid, ctx) {
                    Ok(counted) => CoopCommandStatus::CancelledInteraction { counted },
                    Err(err) => CoopCommandStatus::Refused(err.to_string()),
                };
                record_selection_deltas(&before, &ctx.session, script_changes);
                *session = ctx.session.clone();
                status
            }
            Command::BuyBuild(edit) => {
                if let Err(status) = apply_session_command(session, stamped) {
                    return status;
                }
                if ctx.frozen_dialog.is_some() || self.has_pending_dialog() || self.answered {
                    return CoopCommandStatus::IgnoredWhileDialogPending;
                }
                let issuer_mode = session
                    .players
                    .get(&issuer)
                    .map(|p| p.mode)
                    .unwrap_or(PlayerMode::Live);
                let required_mode = match edit {
                    BuyBuildEdit::BuyObject { .. }
                    | BuyBuildEdit::MoveObject { .. }
                    | BuyBuildEdit::SellObject { .. } => PlayerMode::Buy,
                    BuyBuildEdit::PlaceWall { .. }
                    | BuyBuildEdit::RemoveWall { .. }
                    | BuyBuildEdit::SetFloorTile { .. }
                    | BuyBuildEdit::SetWallpaper { .. }
                    | BuyBuildEdit::SetTerrainHeight { .. } => PlayerMode::Build,
                };
                if issuer_mode != required_mode {
                    return CoopCommandStatus::Refused(format!(
                        "{issuer} must be in {required_mode:?} mode to execute this edit \
                         (currently {issuer_mode:?}, tick {tick})"
                    ));
                }
                if self.mode() == PanelMode::Live {
                    return CoopCommandStatus::Refused(
                        "cannot execute Buy/Build edits while lot is in Live mode".into(),
                    );
                }
                ctx.session = session.clone();
                let before = snapshot_selections(&ctx.session);
                let status = self.execute_coop_buy_build(edit, ctx);
                record_selection_deltas(&before, &ctx.session, script_changes);
                *session = ctx.session.clone();
                status
            }
        }
    }

    /// The controlled family's members for a co-op interpreter when global 9
    /// is 0. A lot loaded with family number 0 holds the family numbered 0
    /// (0x428041 at 0x42827c..0x42830d). When the World keeps that family,
    /// `None` lets each reader look it up live (`family_zero_members`), as
    /// the solo arrival path (0x4f1c87..0x4f1cec) refills it; otherwise the
    /// family is empty, as on an unoccupied lot.
    fn coop_active_family(&self) -> Option<&'static [u32]> {
        (self.load.house != 0
            && self.load.family == 0
            && !self.world.families.iter().any(|f| f.id == 0))
        .then_some(&[][..])
    }

    /// Constructs a co-op [`Interpreter`] with `coop` attached, host camera
    /// rotation from `coop`, and `view_centre: None` (so local camera panning
    /// never leaks into shared simulation callbacks).
    fn coop_interpreter<'a>(
        &'a self,
        sounds: &'a RefCell<Vec<crate::SoundEvent>>,
        character_names: &'a RefCell<BTreeMap<u32, String>>,
        coop: &'a RefCell<CoopVmContext>,
    ) -> Interpreter<'a> {
        let host_rotation = coop.borrow().host_camera_rotation();
        Interpreter {
            sounds: Some(sounds),
            view_rotation: host_rotation,
            view_centre: None,
            character_names: Some(character_names),
            default_character_name: Some(&self.default_character_name),
            architecture_prices: Some(&self.architecture_prices),
            active_family: self.coop_active_family(),
            coop: Some(coop),
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        }
    }

    fn set_mode_with_coop(
        &mut self,
        mode: PanelMode,
        ctx: &mut CoopVmContext,
    ) -> Result<(), OrderError> {
        self.ready()?;
        let previous = self.mode();
        let sounds = RefCell::new(Vec::new());
        let character_names = RefCell::new(self.character_names.clone());
        let coop_cell = RefCell::new(ctx.clone());
        let interpreter = self.coop_interpreter(&sounds, &character_names, &coop_cell);
        let mut world = self.world.clone();
        interpreter
            .set_mode(&mut world, mode)
            .map_err(OrderError::Refused)?;
        if previous != PanelMode::Live && mode == PanelMode::Live {
            interpreter
                .update_comeandsee(&mut world)
                .map_err(OrderError::Refused)?;
        }
        self.world = world;
        self.character_names = character_names.into_inner();
        *ctx = coop_cell.into_inner();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_coop_pick(
        &mut self,
        issuer: PlayerId,
        sim_id: u16,
        target_id: u16,
        menu: u32,
        joining: bool,
        args: [i16; 4],
        ctx: &mut CoopVmContext,
    ) -> CoopCommandStatus {
        if let Err(err) = self.ready() {
            return CoopCommandStatus::Refused(err.to_string());
        }

        // 1. Validate availability against a throwaway issuer-specific
        //    `pie_menu` probe (CoW World clone, zero side effects on shared
        //    state) so stale, unavailable, or forged picks and join offers are
        //    rejected cleanly and join offers retain their exact running flags.
        let matched_entry = {
            let mut probe_ctx = ctx.clone();
            probe_ctx.active_player_override = Some(issuer);
            let player_rotation = probe_ctx
                .session
                .players
                .get(&issuer)
                .map(|p| p.camera_rotation.quarter_turns())
                .unwrap_or_else(|| probe_ctx.host_camera_rotation());
            let probe_coop = RefCell::new(probe_ctx);
            let probe_sounds = RefCell::new(Vec::new());
            let probe_names = RefCell::new(self.character_names.clone());
            let probe_interp = Interpreter {
                sounds: Some(&probe_sounds),
                view_rotation: player_rotation,
                view_centre: None,
                character_names: Some(&probe_names),
                default_character_name: Some(&self.default_character_name),
                architecture_prices: Some(&self.architecture_prices),
                active_family: self.coop_active_family(),
                coop: Some(&probe_coop),
                ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
            };
            let mut throwaway_world = self.world.clone();
            let (probe_caller, probe_clicked) = if joining {
                let Ok(origin_sim) = u16::try_from(args[2]) else {
                    return CoopCommandStatus::Refused(format!(
                        "invalid join origin Sim ID {}",
                        args[2]
                    ));
                };
                if origin_sim == 0 {
                    return CoopCommandStatus::Refused("join action missing origin Sim ID".into());
                }
                (origin_sim, sim_id)
            } else {
                (sim_id, target_id)
            };
            let entries =
                match probe_interp.pie_menu(&mut throwaway_world, probe_caller, probe_clicked) {
                    Ok(entries) => entries,
                    Err(fault) => return CoopCommandStatus::Refused(fault.to_string()),
                };
            let want_join = if joining { 1 } else { 0 };
            entries.into_iter().find(|entry| {
                entry.action.caller == sim_id as i16
                    && entry.action.target == target_id as i16
                    && entry.action.table_index == menu as i32
                    && (entry.action.join != 0) == (want_join != 0)
                    && entry.action.args == args
            })
        };

        let Some(entry) = matched_entry else {
            return CoopCommandStatus::Refused(format!(
                "interaction (sim={sim_id}, target={target_id}, menu={menu}, joining={joining}) \
                 is not currently offered"
            ));
        };

        // 2. Stage `(mut world, coop_cell)` with `issuer` recorded as activity
        //    owner BEFORE running `interpreter.pick` so any `at_once` script
        //    observes the new owner. Commit ONLY on `Ok(Some(uid))` so a failed
        //    at-once check or full queue (`Ok(None)`) never leaks RNG draws,
        //    check-tree mutations, or phantom ownership into shared state.
        let target_lead = u16::try_from(entry.action.target).ok().unwrap_or(target_id);
        let mut staged_ctx = ctx.clone();
        staged_ctx.record_activity_owner(sim_id, issuer);
        staged_ctx.record_activity_owner(target_id, issuer);
        staged_ctx.record_activity_owner(target_lead, issuer);

        let sounds = RefCell::new(Vec::new());
        let character_names = RefCell::new(self.character_names.clone());
        let coop_cell = RefCell::new(staged_ctx);
        let interpreter = self.coop_interpreter(&sounds, &character_names, &coop_cell);
        let mut world = self.world.clone();
        match interpreter.pick(&mut world, sim_id, entry.action) {
            Ok(Some(uid)) => {
                self.world = world;
                self.character_names = character_names.into_inner();
                *ctx = coop_cell.into_inner();
                CoopCommandStatus::QueuedInteraction { uid: Some(uid) }
            }
            Ok(None) => CoopCommandStatus::QueuedInteraction { uid: None },
            Err(fault) => CoopCommandStatus::Refused(fault.to_string()),
        }
    }

    fn cancel_with_coop(
        &mut self,
        person: u16,
        uid: i32,
        ctx: &mut CoopVmContext,
    ) -> Result<bool, OrderError> {
        self.ready()?;
        let sounds = RefCell::new(Vec::new());
        let character_names = RefCell::new(self.character_names.clone());
        let coop_cell = RefCell::new(ctx.clone());
        let interpreter = self.coop_interpreter(&sounds, &character_names, &coop_cell);
        let mut world = self.world.clone();
        let counts = interpreter
            .cancel(&mut world, person, uid)
            .map_err(OrderError::Refused)?;
        self.world = world;
        self.character_names = character_names.into_inner();
        *ctx = coop_cell.into_inner();
        Ok(counts)
    }

    /// Executes one [`BuyBuildEdit`] atomically on a staged `(World,
    /// CoopVmContext)` with co-op callbacks enabled and catalog `build_prices`,
    /// purging all detached objects before committing so live state is always
    /// cursor-free and snapshot-ready. Any refusal or cleanup error leaves
    /// `self` and `ctx` 100% untouched.
    fn execute_coop_buy_build(
        &mut self,
        edit: &BuyBuildEdit,
        ctx: &mut CoopVmContext,
    ) -> CoopCommandStatus {
        if let Err(err) = self.ready() {
            return CoopCommandStatus::Refused(err.to_string());
        }
        let sounds = RefCell::new(Vec::new());
        let character_names = RefCell::new(self.character_names.clone());
        let coop_cell = RefCell::new(ctx.clone());
        let interpreter = self.coop_interpreter(&sounds, &character_names, &coop_cell);
        let mut world = self.world.clone();
        let empty_keep = BTreeSet::new();

        let result: Result<(), String> = match edit {
            BuyBuildEdit::BuyObject {
                guid,
                position,
                facing,
                container,
            } => (|| {
                let target_facing = facing.rem_euclid(8);
                let snapped = snap_to_tile_centre([position[0], position[1]]);
                let mut cur = interpreter
                    .buy_start(&mut world, *guid, position[2], target_facing)
                    .map_err(|f| f.to_string())?
                    .ok_or_else(|| "cannot start Buy cursor (insufficient funds)".to_string())?;
                if let Some(target_container) = *container {
                    cur.position = [snapped[0], snapped[1], position[2]];
                    cur.container = Some(target_container);
                }
                let valid = interpreter
                    .buy_move(
                        &mut world,
                        &mut cur,
                        [position[0], position[1]],
                        position[2],
                    )
                    .map_err(|f| f.to_string())?;
                if !valid || !cur.valid {
                    return Err("Buy placement does not fit or is unaffordable".into());
                }
                let actual_facing = world
                    .objects
                    .get(&cur.held)
                    .and_then(|o| o.data.get(DATA_FACING).copied())
                    .unwrap_or(0)
                    .rem_euclid(8);
                if actual_facing != target_facing {
                    return Err(format!(
                        "Buy placement facing drifted from requested {target_facing} to \
                         {actual_facing}"
                    ));
                }
                if cur.container != *container {
                    return Err(format!(
                        "Buy placement container {:?} did not match requested {:?}",
                        cur.container, container
                    ));
                }
                if cur.position != [snapped[0], snapped[1], position[2]] {
                    return Err(format!(
                        "Buy placement position {:?} did not match requested [{}, {}, {}]",
                        cur.position, snapped[0], snapped[1], position[2]
                    ));
                }
                let purchase = interpreter
                    .buy_commit_record(&mut world, &cur)
                    .map_err(|f| f.to_string())?;
                verify_committed_buy_placement(
                    &mut world,
                    cur.held,
                    &purchase,
                    [snapped[0], snapped[1], position[2]],
                    target_facing,
                    *container,
                    "Buy",
                )?;
                Ok(())
            })(),
            BuyBuildEdit::MoveObject {
                object_id,
                position,
                facing,
                container,
            } => (|| {
                let target_facing = facing.rem_euclid(8);
                let snapped = snap_to_tile_centre([position[0], position[1]]);
                let mut cur = interpreter
                    .buy_pick_up(&mut world, *object_id)
                    .map_err(|f| f.to_string())?
                    .ok_or_else(|| format!("object {object_id} cannot be picked up"))?;
                interpreter
                    .buy_rotate(&mut world, &mut cur, target_facing)
                    .map_err(|f| f.to_string())?;
                let rotated_facing = world
                    .objects
                    .get(&cur.held)
                    .and_then(|o| o.data.get(DATA_FACING).copied())
                    .unwrap_or(0)
                    .rem_euclid(8);
                if rotated_facing != target_facing {
                    return Err(format!(
                        "object {object_id} cannot rotate to requested facing {target_facing}"
                    ));
                }
                if let Some(target_container) = *container {
                    cur.position = [snapped[0], snapped[1], position[2]];
                    cur.container = Some(target_container);
                } else {
                    cur.container = None;
                }
                let valid = interpreter
                    .buy_move(
                        &mut world,
                        &mut cur,
                        [position[0], position[1]],
                        position[2],
                    )
                    .map_err(|f| f.to_string())?;
                if !valid || !cur.valid {
                    return Err("moved object does not fit at target position".into());
                }
                let actual_facing = world
                    .objects
                    .get(&cur.held)
                    .and_then(|o| o.data.get(DATA_FACING).copied())
                    .unwrap_or(0)
                    .rem_euclid(8);
                if actual_facing != target_facing {
                    return Err(format!(
                        "Move placement facing drifted from requested {target_facing} to \
                         {actual_facing}"
                    ));
                }
                if cur.container != *container {
                    return Err(format!(
                        "Move placement container {:?} did not match requested {:?}",
                        cur.container, container
                    ));
                }
                if cur.position != [snapped[0], snapped[1], position[2]] {
                    return Err(format!(
                        "Move placement position {:?} did not match requested [{}, {}, {}]",
                        cur.position, snapped[0], snapped[1], position[2]
                    ));
                }
                let purchase = interpreter
                    .buy_commit_record(&mut world, &cur)
                    .map_err(|f| f.to_string())?;
                verify_committed_buy_placement(
                    &mut world,
                    cur.held,
                    &purchase,
                    [snapped[0], snapped[1], position[2]],
                    target_facing,
                    *container,
                    "Move",
                )?;
                Ok(())
            })(),
            BuyBuildEdit::SellObject { object_id } => (|| {
                let cur = interpreter
                    .buy_pick_up(&mut world, *object_id)
                    .map_err(|f| f.to_string())?
                    .ok_or_else(|| format!("object {object_id} cannot be picked up to sell"))?;
                let sold = interpreter
                    .buy_sell(&mut world, &cur)
                    .map_err(|f| f.to_string())?
                    .ok_or_else(|| format!("object {object_id} cannot be sold"))?;
                if let Some(purchase) = sold {
                    // Purge the detached sold instance immediately inside the
                    // staged transaction so `removal.rs` cleans `activity_owners`
                    // and live state stays 100% snapshot-ready.
                    interpreter
                        .buy_purge_detached(
                            &mut world,
                            std::slice::from_ref(&purchase),
                            &empty_keep,
                        )
                        .map_err(|f| f.to_string())?;
                }
                Ok(())
            })(),
            BuyBuildEdit::PlaceWall {
                x,
                y,
                level,
                style,
                pattern,
            } => (|| {
                // Only the 5 directly buildable styles may be requested;
                // internal derived styles (e.g. 22/23) are refused rather than
                // silently rewritten to Standard style 1.
                let wall_style = match *style {
                    1 => WallStyle::Standard,
                    2 => WallStyle::WhitePicket,
                    12 => WallStyle::PrivacyFence,
                    13 => WallStyle::WroughtIron,
                    14 => WallStyle::Monticello,
                    _ => return Err(format!("unsupported wall style {style}")),
                };
                if WallStyle::is_fence_style(wall_style.style_id())
                    && *pattern != 0
                    && *pattern != wall_style.default_pattern()
                {
                    return Err(format!(
                        "fence style {style} cannot take wallpaper pattern {pattern}"
                    ));
                }
                let wants_wallpaper = *pattern != 0 && *pattern != wall_style.default_pattern();
                if wants_wallpaper {
                    require_priced_wall_pattern(&ctx.build_prices, *pattern)?;
                }
                let plan = plan_wall_drag(
                    &world,
                    [i32::from(*x), i32::from(*y)],
                    [i32::from(*x) + 1, i32::from(*y)],
                    *level,
                    wall_style,
                    WallDragMode::Build,
                    &ctx.build_prices,
                );
                if !plan.valid || !plan.affordable {
                    return Err("wall placement invalid or unaffordable".into());
                }
                let history = interpreter
                    .build_commit_wall(&mut world, &plan)
                    .map_err(|f| f.to_string())?
                    .ok_or_else(|| "wall placement produced no change".to_string())?;
                interpreter
                    .buy_purge_detached(&mut world, history.detached_objects(), &empty_keep)
                    .map_err(|f| f.to_string())?;

                // If a non-default wallpaper pattern was requested on a
                // standard wall, require BOTH faces (`(tx, ty) WALL_NORTH` and
                // `(tx, ty - 1) WALL_SOUTH`) to be buildable and painted to
                // `*pattern` inside the same atomic transaction.
                if wants_wallpaper {
                    let (Ok(tx), Ok(ty)) = (u8::try_from(*x), u8::try_from(*y)) else {
                        return Err("wall tile out of range for wallpaper pattern".into());
                    };
                    let Some(north_y) = ty.checked_sub(1) else {
                        return Err("north wall at y = 0 has no opposite face to wallpaper".into());
                    };
                    if !is_buildable_tile(&world, usize::from(tx), usize::from(ty), *level)
                        || !is_buildable_tile(&world, usize::from(tx), usize::from(north_y), *level)
                    {
                        return Err(format!(
                            "cannot wallpaper both faces of north wall at ({tx}, {ty}) level \
                             {level}: adjacent tile is not buildable"
                        ));
                    }
                    let faces = [
                        WallFace {
                            tile: [tx, ty],
                            bit: WALL_NORTH,
                            side: 0,
                        },
                        WallFace {
                            tile: [tx, north_y],
                            bit: WALL_SOUTH,
                            side: 0,
                        },
                    ];
                    let wp_plan = plan_wallpaper_edit(
                        &world,
                        &faces,
                        *level,
                        *pattern,
                        WallpaperDragMode::Apply,
                        &ctx.build_prices,
                    );
                    if !wp_plan.valid || !wp_plan.affordable {
                        return Err(format!(
                            "wall pattern {pattern} invalid or unaffordable on new wall"
                        ));
                    }
                    let wp_hist = interpreter
                        .build_commit_wallpaper(&mut world, &wp_plan)
                        .map_err(|f| f.to_string())?
                        .ok_or_else(|| "wall pattern application produced no change".to_string())?;
                    interpreter
                        .buy_purge_detached(&mut world, wp_hist.detached_objects(), &empty_keep)
                        .map_err(|f| f.to_string())?;

                    // Verify both faces now hold the exact requested pattern.
                    let level_idx = usize::from(level.saturating_sub(1));
                    let Some(walls_arr) = world.arrays.get(&WALLS[level_idx]) else {
                        return Err("wall array missing after wallpaper commit".into());
                    };
                    let active = usize::try_from(world.size).unwrap_or(64);
                    let south_cell = ExpandedWall::get(
                        &walls_arr.data,
                        active,
                        usize::from(tx),
                        usize::from(ty),
                    );
                    let north_cell = ExpandedWall::get(
                        &walls_arr.data,
                        active,
                        usize::from(tx),
                        usize::from(north_y),
                    );
                    if south_cell.patterns[1] != *pattern || north_cell.patterns[3] != *pattern {
                        return Err(format!(
                            "wall pattern {pattern} did not apply to both faces of north wall at \
                             ({tx}, {ty})"
                        ));
                    }
                }
                Ok(())
            })(),
            BuyBuildEdit::RemoveWall { x, y, level } => (|| {
                let plan = plan_wall_drag(
                    &world,
                    [i32::from(*x), i32::from(*y)],
                    [i32::from(*x) + 1, i32::from(*y)],
                    *level,
                    WallStyle::Standard,
                    WallDragMode::Demolish,
                    &ctx.build_prices,
                );
                if !plan.valid {
                    return Err("wall removal invalid".into());
                }
                if (1..=2).contains(level)
                    && let (Ok(tx), Ok(ty)) = (usize::try_from(*x), usize::try_from(*y))
                    && let Some(walls_arr) = world.arrays.get(&WALLS[usize::from(*level - 1)])
                {
                    let active = usize::try_from(world.size).unwrap_or(64);
                    let cell = ExpandedWall::get(&walls_arr.data, active, tx, ty);
                    require_priced_wall_pattern(&ctx.build_prices, cell.patterns[1])?;
                    if ty > 0 {
                        let north_cell = ExpandedWall::get(&walls_arr.data, active, tx, ty - 1);
                        if (north_cell.segments
                            & (crate::build::WALL_DESCENDING | crate::build::WALL_ASCENDING))
                            == 0
                        {
                            require_priced_wall_pattern(&ctx.build_prices, north_cell.patterns[3])?;
                        }
                    }
                }
                let history = interpreter
                    .build_commit_wall(&mut world, &plan)
                    .map_err(|f| f.to_string())?
                    .ok_or_else(|| "wall removal produced no change".to_string())?;
                interpreter
                    .buy_purge_detached(&mut world, history.detached_objects(), &empty_keep)
                    .map_err(|f| f.to_string())?;
                Ok(())
            })(),
            BuyBuildEdit::SetFloorTile {
                x,
                y,
                level,
                pattern,
            } => (|| {
                if *pattern != 0 {
                    require_priced_floor(&ctx.build_prices, *pattern)?;
                }
                let mode = if *pattern == 0 {
                    FloorDragMode::Remove
                } else {
                    FloorDragMode::Lay
                };
                let tile = [i32::from(*x), i32::from(*y)];
                let plan = plan_floor_edit(
                    &world,
                    tile,
                    tile,
                    *level,
                    *pattern,
                    mode,
                    &ctx.build_prices,
                    None,
                );
                if !plan.valid || !plan.affordable {
                    return Err("floor edit invalid or unaffordable".into());
                }
                if mode.is_remove() {
                    for &(_idx, before_floor, _after_floor) in &plan.floor_changes {
                        require_priced_floor(&ctx.build_prices, before_floor)?;
                    }
                    for &(_idx, before_wall, after_wall) in &plan.wall_changes {
                        let old_h0 = u16::from_le_bytes([before_wall[2], before_wall[3]]);
                        let new_h0 = u16::from_le_bytes([after_wall[2], after_wall[3]]);
                        if old_h0 != new_h0 {
                            require_priced_floor(&ctx.build_prices, old_h0)?;
                        }
                        let old_h1 = u16::from_le_bytes([before_wall[6], before_wall[7]]);
                        let new_h1 = u16::from_le_bytes([after_wall[6], after_wall[7]]);
                        if old_h1 != new_h1 {
                            require_priced_floor(&ctx.build_prices, old_h1)?;
                        }
                    }
                }
                let history = interpreter
                    .build_commit_floor(&mut world, &plan)
                    .map_err(|f| f.to_string())?
                    .ok_or_else(|| "floor edit produced no change".to_string())?;
                interpreter
                    .buy_purge_detached(&mut world, history.detached_objects(), &empty_keep)
                    .map_err(|f| f.to_string())?;
                Ok(())
            })(),
            BuyBuildEdit::SetWallpaper {
                x,
                y,
                level,
                side,
                pattern,
            } => (|| {
                let (Ok(tx), Ok(ty)) = (u8::try_from(*x), u8::try_from(*y)) else {
                    return Err("wallpaper tile out of range".into());
                };
                let (bit, pattern_idx) = match *side {
                    0 => (WALL_NORTH, 1usize),
                    1 => (WALL_EAST, 2usize),
                    2 => (WALL_SOUTH, 3usize),
                    3 => (WALL_WEST, 0usize),
                    _ => return Err(format!("invalid wallpaper side {side} (expected 0..=3)")),
                };
                if *pattern != 0 {
                    require_priced_wall_pattern(&ctx.build_prices, *pattern)?;
                }
                let face = WallFace {
                    tile: [tx, ty],
                    bit,
                    side: 0,
                };
                let mode = if *pattern == 0 {
                    WallpaperDragMode::Strip
                } else {
                    WallpaperDragMode::Apply
                };
                let plan =
                    plan_wallpaper_edit(&world, &[face], *level, *pattern, mode, &ctx.build_prices);
                if !plan.valid || !plan.affordable {
                    return Err("wallpaper edit invalid or unaffordable".into());
                }
                if mode.is_strip()
                    && (1..=2).contains(level)
                    && let Some(walls_arr) = world.arrays.get(&WALLS[usize::from(*level - 1)])
                {
                    let cell = ExpandedWall::get(
                        &walls_arr.data,
                        usize::try_from(world.size).unwrap_or(64),
                        usize::from(tx),
                        usize::from(ty),
                    );
                    require_priced_wall_pattern(&ctx.build_prices, cell.patterns[pattern_idx])?;
                }
                let history = interpreter
                    .build_commit_wallpaper(&mut world, &plan)
                    .map_err(|f| f.to_string())?
                    .ok_or_else(|| "wallpaper edit produced no change".to_string())?;
                interpreter
                    .buy_purge_detached(&mut world, history.detached_objects(), &empty_keep)
                    .map_err(|f| f.to_string())?;
                Ok(())
            })(),
            BuyBuildEdit::SetTerrainHeight { .. } => {
                Err("terrain height editing is not yet ported".to_string())
            }
        };

        match result {
            Ok(()) => {
                self.world = world;
                self.character_names = character_names.into_inner();
                *ctx = coop_cell.into_inner();
                CoopCommandStatus::BuyBuildApplied
            }
            Err(msg) => CoopCommandStatus::Refused(msg),
        }
    }

    fn sanitize_session_selections(&self, session: &mut SessionState) {
        for player in session.players.values_mut() {
            if let Some(sim_id) = player.selected_sim
                && !self
                    .world
                    .objects
                    .get(&sim_id)
                    .is_some_and(|o| o.person.is_some())
            {
                player.selected_sim = None;
            }
        }
    }

    fn sync_world_from_session(
        &mut self,
        ctx: &mut CoopVmContext,
        apply_speed: bool,
    ) -> Result<(), OrderError> {
        sync_session_globals(&mut self.world, &ctx.session);
        if apply_speed {
            self.world
                .set_speed(i32::from(ctx.session.time.speed.value()));
        }
        self.set_view_rotation(ctx.host_camera_rotation());

        let target_mode = player_mode_to_panel_mode(ctx.session.effective_mode());
        if self.mode() != target_mode {
            self.set_mode_with_coop(target_mode, ctx)?;
        }
        sync_pause_global(&mut self.world, ctx.session.is_paused());
        self.sync_host_plumbob_with_coop(ctx)?;
        Ok(())
    }

    /// Best-effort variant used inside `execute_coop_batch` after commands or a
    /// simulation tick have already committed so a callback fault (such as an
    /// occupied plumbob indicator slot) can never abort before `current_tick`
    /// advances to `batch.tick`.
    fn sync_world_from_session_best_effort(&mut self, ctx: &mut CoopVmContext, apply_speed: bool) {
        sync_session_globals(&mut self.world, &ctx.session);
        if apply_speed {
            self.world
                .set_speed(i32::from(ctx.session.time.speed.value()));
        }
        self.set_view_rotation(ctx.host_camera_rotation());

        let target_mode = player_mode_to_panel_mode(ctx.session.effective_mode());
        if self.mode() != target_mode {
            let _ = self.set_mode_with_coop(target_mode, ctx);
        }
        sync_pause_global(&mut self.world, ctx.session.is_paused());
        if self.sync_host_plumbob_with_coop(ctx).is_err()
            && let Some(slot) = self.world.globals.get_mut(GLOBAL_SELECTED)
        {
            *slot = ctx
                .session
                .players
                .get(&ctx.session.host_id)
                .and_then(|p| p.selected_sim)
                .map_or(0, |id| id as i16);
        }
    }

    fn sync_host_plumbob_with_coop(&mut self, ctx: &mut CoopVmContext) -> Result<(), OrderError> {
        let host_selected = ctx
            .session
            .players
            .get(&ctx.session.host_id)
            .and_then(|p| p.selected_sim);
        if self.world.selected() == host_selected {
            if let Some(slot) = self.world.globals.get_mut(GLOBAL_SELECTED) {
                *slot = host_selected.map_or(0, |id| id as i16);
            }
            return Ok(());
        }
        self.ready()?;
        let sounds = RefCell::new(Vec::new());
        let character_names = RefCell::new(self.character_names.clone());
        let coop_cell = RefCell::new(ctx.clone());
        let interpreter = self.coop_interpreter(&sounds, &character_names, &coop_cell);
        let mut world = self.world.clone();
        interpreter
            .select(&mut world, host_selected)
            .map_err(OrderError::Refused)?;
        interpreter.refresh_family_panel(&mut world);
        self.world = world;
        self.character_names = character_names.into_inner();
        *ctx = coop_cell.into_inner();
        Ok(())
    }

    /// Runs a throwaway per-player pie-menu availability check for `player`'s
    /// currently selected Sim on `target` (`docs/vision.md` lines 174–176).
    ///
    /// Takes `&self`, running all check trees against a throwaway CoW clone of
    /// [`World`] with `player`'s selected Sim and `camera_rotation` so random
    /// draws and scratch mutations never touch the shared simulation.
    pub fn coop_pie_menu(&self, player: PlayerId, target: u16) -> Result<Menu, OrderError> {
        let ctx = self.coop.as_ref().ok_or_else(|| {
            OrderError::Refused(crate::Fault {
                object: target,
                at: None,
                trap: crate::Trap::Invalid("co-op context is not attached".into()),
            })
        })?;
        let sim_id = ctx
            .session
            .players
            .get(&player)
            .and_then(|p| p.selected_sim)
            .ok_or_else(|| {
                OrderError::Refused(crate::Fault {
                    object: target,
                    at: None,
                    trap: crate::Trap::Invalid(format!("{player} has no selected Sim")),
                })
            })?;
        self.coop_pie_menu_for_sim(player, sim_id, target)
    }

    /// Runs a throwaway per-player pie-menu availability check for `sim_id` on
    /// `target` under `player`'s co-op context (`&self`: never mutates the
    /// shared simulation).
    pub fn coop_pie_menu_for_sim(
        &self,
        player: PlayerId,
        sim_id: u16,
        target: u16,
    ) -> Result<Menu, OrderError> {
        self.ready()?;
        let ctx = self.coop.as_ref().ok_or_else(|| {
            OrderError::Refused(crate::Fault {
                object: sim_id,
                at: None,
                trap: crate::Trap::Invalid("co-op context is not attached".into()),
            })
        })?;
        let mut probe_ctx = ctx.clone();
        probe_ctx.active_player_override = Some(player);
        let player_rotation = probe_ctx
            .session
            .players
            .get(&player)
            .map(|p| p.camera_rotation.quarter_turns())
            .unwrap_or_else(|| probe_ctx.host_camera_rotation());
        let coop_cell = RefCell::new(probe_ctx);
        let sounds = RefCell::new(Vec::new());
        let character_names = RefCell::new(self.character_names.clone());
        let interpreter = Interpreter {
            sounds: Some(&sounds),
            view_rotation: player_rotation,
            view_centre: None,
            character_names: Some(&character_names),
            default_character_name: Some(&self.default_character_name),
            architecture_prices: Some(&self.architecture_prices),
            active_family: self.coop_active_family(),
            coop: Some(&coop_cell),
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut throwaway_world = self.world.clone();
        let entries = interpreter
            .pie_menu(&mut throwaway_world, sim_id, target)
            .map_err(OrderError::Refused)?;
        Ok(Menu {
            entries,
            sounds: sounds.into_inner(),
        })
    }

    /// Computes a deterministic 32-byte SHA-256 hash covering `World` (format
    /// 32 + format-8 auxiliary tables) and all `Lot`/co-op continuation inputs
    /// (`frames`, `portrait_anchor`, `build_prices`, `answers`, `replay_view`,
    /// `frozen_dialog`, `play_mode`, `play.fresh`, `activity_owners`).
    ///
    /// Pass this digest to `sims_net::LockstepNode::report_local_tick` and
    /// `sims_net::LockstepNode::send_snapshot` so periodic drift detection and
    /// snapshot verification match [`Self::coop_state_fingerprint`] bit-for-bit.
    pub fn coop_world_hash(&self) -> [u8; 32] {
        let w = &self.world;
        let effective_view_rotation = self
            .coop
            .as_ref()
            .map_or(self.view_rotation, CoopVmContext::host_camera_rotation);
        let frozen_dialog_digest = self
            .coop
            .as_ref()
            .and_then(|c| c.frozen_dialog.as_ref())
            .map(FrozenDialogTick::digest);
        let tail = CoopWorldFingerprintTail {
            world_hash: w.hash(),
            ticks: self.ticks,
            view_rotation: effective_view_rotation,
            answered: self.answered,
            asking: self.asking.as_ref().map(|a| (a.shown.tick, a.shown.object)),
            asking_special_panel: self
                .asking_special_panel
                .as_ref()
                .map(|a| (a.shown.tick, &a.shown.request)),
            special_panels: &self.special_panels,
            special_panel_responses: &self.special_panel_responses,
            answers: &self.player,
            replay_view_rotation: self.replay_view.map(|(rot, _)| rot),
            frames: self.coop.as_ref().map_or(&self.frames, |c| &c.frames),
            portrait_anchor: self.coop.as_ref().and_then(|c| c.portrait_anchor),
            build_prices: self.coop.as_ref().map(|c| &c.build_prices),
            play_mode: self.coop.as_ref().is_some_and(|c| c.play_mode),
            play_fresh: &self.play.fresh,
            last_dialog_text: &self.last_dialog_text,
            character_names: &self.character_names,
            generated_portrait_2002: &w.generated_portrait_2002,
            made_portrait_head: &w.made_portrait_head,
            ambiguous_portraits: &w.ambiguous_portraits,
            loaded_indexed_skeletons: &w.loaded_indexed_skeletons,
            loaded_indexed_suits: &w.loaded_indexed_suits,
            loaded_indexed_suit_bones: &w.loaded_indexed_suit_bones,
            made_outfits: &w.made_outfits,
            person_outfits: &w.person_outfits,
            scratch_ptr_set: w.scratch_ptrs.serialize_bit(sims_sim::SCRATCH_PTR_SET),
            scratch_ptr_set_to_next: w
                .scratch_ptrs
                .serialize_bit(sims_sim::SCRATCH_PTR_SET_TO_NEXT),
            scratch_ptr_expression: w
                .scratch_ptrs
                .serialize_bit(sims_sim::SCRATCH_PTR_EXPRESSION),
            scratch_ptr_arg_slot: w.scratch_ptrs.serialize_bit(sims_sim::SCRATCH_PTR_ARG_SLOT),
            runtime_added_suits: &w.runtime_added_suits,
            suit_scan_cache: &w.suit_scan_cache,
            family_flags: &w.family_flags,
            activity_owners: self.coop.as_ref().map(|c| &c.activity_owners),
            next_dialog_id: self.coop.as_ref().map(|c| c.next_dialog_id),
            active_dialog: self.coop.as_ref().and_then(|c| c.active_dialog),
            active_special_panel: self
                .coop
                .as_ref()
                .and_then(|c| c.active_special_panel.as_ref().map(|(id, s)| (*id, s))),
            frozen_dialog_digest,
            pending_speed_sync: self.coop.as_ref().is_some_and(|c| c.pending_speed_sync),
            indicator_queues: &w.indicator_queues,
        };
        let mut hasher = Sha256::new();
        hasher.update(COOP_WORLD_HASH_DOMAIN);
        sims_sim::hash_postcard(&tail, &mut hasher);
        hasher.finalize().into()
    }

    /// Computes the combined [`StateFingerprint`] matching
    /// `session.combined_fingerprint(self.coop_world_hash())`.
    pub fn coop_state_fingerprint(&self, session: &SessionState) -> StateFingerprint {
        session.combined_fingerprint(self.coop_world_hash())
    }

    /// Encodes a deterministic co-op snapshot payload (`Saved` bytes + co-op VM
    /// metadata) suitable for `sims_net::LockstepNode::send_snapshot` (mid-game
    /// join and drift resync).
    pub fn coop_snapshot_bytes(&self) -> Result<Vec<u8>, SaveError> {
        let saved_bytes = self.save()?.encode();
        let meta = match self.coop.as_ref() {
            Some(ctx) => CoopSnapshotMeta {
                activity_owners: ctx.activity_owners.clone(),
                next_dialog_id: ctx.next_dialog_id,
                active_dialog: ctx.active_dialog,
                active_special_panel: ctx.active_special_panel.clone(),
                frozen_dialog: ctx.frozen_dialog.clone(),
                pending_speed_sync: ctx.pending_speed_sync,
                frames: ctx.frames.clone(),
                portrait_anchor: ctx.portrait_anchor,
                build_prices: ctx.build_prices.clone(),
                play_mode: ctx.play_mode,
            },
            None => CoopSnapshotMeta {
                activity_owners: BTreeMap::new(),
                next_dialog_id: 1,
                active_dialog: None,
                active_special_panel: None,
                frozen_dialog: None,
                pending_speed_sync: false,
                frames: self.frames.clone(),
                portrait_anchor: None,
                build_prices: self.default_coop_build_prices(),
                play_mode: false,
            },
        };
        let envelope = CoopSnapshotEnvelope { saved_bytes, meta };
        let body = postcard::to_allocvec(&envelope)
            .map_err(|e| SaveError::Format(format!("encoding co-op snapshot: {e}")))?;
        let mut out = Vec::with_capacity(8 + body.len());
        out.extend_from_slice(COOP_SNAPSHOT_MAGIC);
        out.extend_from_slice(&COOP_SNAPSHOT_VERSION.to_le_bytes());
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Restores a co-op snapshot emitted via `NodeEvent::ApplySnapshot`
    /// (`redb_bytes` + `session`) onto this lot transactionally: if decoding,
    /// metadata validation, or mid-dialog re-probing fails, `self` is restored
    /// to its previous state unchanged.
    ///
    /// Accepts both [`Self::coop_snapshot_bytes`] envelopes and plain
    /// [`Saved::encode`] bytes (when `session.open_dialogs` is empty).
    pub fn restore_coop_snapshot(
        &mut self,
        bytes: &[u8],
        session: SessionState,
    ) -> Result<(), SaveError> {
        self.restore_coop_snapshot_inner(bytes, session, None)
    }

    /// Transactionally restores a co-op snapshot AND verifies that the
    /// resulting [`StateFingerprint`] matches `expected_fingerprint`, rolling
    /// back `self` unchanged if verification fails.
    pub fn restore_coop_snapshot_verified(
        &mut self,
        bytes: &[u8],
        session: SessionState,
        expected_fingerprint: StateFingerprint,
    ) -> Result<(), SaveError> {
        self.restore_coop_snapshot_inner(bytes, session, Some(expected_fingerprint))
    }

    fn restore_coop_snapshot_inner(
        &mut self,
        bytes: &[u8],
        session: SessionState,
        expected_fingerprint: Option<StateFingerprint>,
    ) -> Result<(), SaveError> {
        // 1. Validate envelope, `Saved` payload, and dialog consistency BEFORE
        //    mutating `self`.
        let (saved_bytes, meta) = if bytes.starts_with(COOP_SNAPSHOT_MAGIC) {
            let Some((header, body)) = bytes.split_first_chunk::<8>() else {
                return Err(SaveError::Format("truncated co-op snapshot header".into()));
            };
            let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
            if version != COOP_SNAPSHOT_VERSION {
                return Err(SaveError::Format(format!(
                    "unsupported co-op snapshot version {version}, expected {COOP_SNAPSHOT_VERSION}"
                )));
            }
            let (env, rest): (CoopSnapshotEnvelope, _) =
                postcard::take_from_bytes(body).map_err(|e| SaveError::Format(e.to_string()))?;
            if !rest.is_empty() {
                return Err(SaveError::Format(format!(
                    "unexpected {} trailing bytes in co-op snapshot",
                    rest.len()
                )));
            }
            (env.saved_bytes, env.meta)
        } else {
            if !session.open_dialogs.is_empty() {
                return Err(SaveError::Format(
                    "raw Saved snapshot lacks co-op dialog metadata required by open session \
                     dialogs"
                        .into(),
                ));
            }
            (
                bytes.to_vec(),
                CoopSnapshotMeta {
                    activity_owners: BTreeMap::new(),
                    next_dialog_id: 1,
                    active_dialog: None,
                    active_special_panel: None,
                    frozen_dialog: None,
                    pending_speed_sync: false,
                    frames: Frames::default(),
                    portrait_anchor: None,
                    build_prices: self.default_coop_build_prices(),
                    play_mode: false,
                },
            )
        };

        if meta.active_dialog.is_some() && meta.active_special_panel.is_some() {
            return Err(SaveError::Format(
                "co-op snapshot cannot have both active_dialog and active_special_panel".into(),
            ));
        }
        let meta_active_id = meta
            .active_dialog
            .or_else(|| meta.active_special_panel.as_ref().map(|(id, _)| *id));
        if !session.open_dialogs.is_empty() {
            let Some(active_id) = meta_active_id else {
                return Err(SaveError::Format(
                    "co-op snapshot has open session dialogs without active dialog/panel metadata"
                        .into(),
                ));
            };
            if !session.open_dialogs.contains_key(&active_id) {
                return Err(SaveError::Format(format!(
                    "co-op snapshot active dialog {active_id} missing from session.open_dialogs"
                )));
            }
        } else if meta_active_id.is_some() {
            return Err(SaveError::Format(
                "co-op snapshot has active dialog/panel metadata while session.open_dialogs is \
                 empty"
                    .into(),
            ));
        }

        let saved = Saved::decode(&saved_bytes)?;

        // 2. Capture rollback checkpoint of all mutable `Lot` fields so any
        //    later failure (including mid-dialog re-probe or fingerprint
        //    mismatch) leaves `self` completely intact.
        let backup_world = self.world.clone();
        let backup_ticks = self.ticks;
        let backup_start = self.start;
        let backup_player = self.player.clone();
        let backup_frames = self.frames.clone();
        let backup_play = self.play.clone();
        let backup_asks = self.asks;
        let backup_view_rotation = self.view_rotation;
        let backup_view_centre = self.view_centre;
        let backup_replay_view = self.replay_view;
        let backup_last_dialog_text = self.last_dialog_text.clone();
        let backup_character_names = self.character_names.clone();
        let backup_lot_name = self.lot_name.clone();
        let backup_lot_description = self.lot_description.clone();
        let backup_halted = self.halted;
        let backup_asking = self.asking.clone();
        let backup_asking_special_panel = self.asking_special_panel.clone();
        let backup_special_panels = self.special_panels.clone();
        let backup_special_panel_responses = self.special_panel_responses.clone();
        let backup_answered = self.answered;
        let backup_buy_undo = self.buy_undo.clone();
        let backup_buy_redo = self.buy_redo.clone();
        let backup_buy_orphans = self.buy_orphans.clone();
        let backup_active_buy_cursor = self.active_buy_cursor.clone();
        let backup_build_undo = self.build_undo.clone();
        let backup_build_redo = self.build_redo.clone();
        let backup_undo_order = self.undo_order.clone();
        let backup_redo_order = self.redo_order.clone();
        let backup_active_wall_cursor = self.active_wall_cursor.clone();
        let backup_neighborhood_origin = self.neighborhood_origin.clone();
        let backup_coop = self.coop.clone();

        let apply_result: Result<(), SaveError> = (|| {
            self.restore(saved)?;
            if self
                .world
                .globals
                .get(GLOBAL_TUTORIAL)
                .copied()
                .unwrap_or(0)
                != 0
            {
                return Err(SaveError::Format(
                    "tutorial lots are single-player only and cannot be restored in co-op mode"
                        .into(),
                ));
            }
            self.disable_coop_tutorial_and_help();
            self.wait_for_answers();
            self.frames = meta.frames.clone();

            let probe_frozen = meta.frozen_dialog.clone();
            let probe_session = probe_frozen
                .as_ref()
                .map(|f| f.session.clone())
                .unwrap_or_else(|| session.clone());
            let probe_owners = probe_frozen
                .as_ref()
                .map(|f| f.activity_owners.clone())
                .unwrap_or_else(|| meta.activity_owners.clone());

            let mut ctx = CoopVmContext::new(probe_session);
            ctx.activity_owners = probe_owners;
            ctx.next_dialog_id = meta.next_dialog_id.max(1);
            ctx.active_dialog = meta.active_dialog;
            ctx.active_special_panel = meta.active_special_panel.clone();
            ctx.frozen_dialog = meta.frozen_dialog;
            ctx.pending_speed_sync = meta.pending_speed_sync;
            ctx.frames = meta.frames;
            ctx.portrait_anchor = meta.portrait_anchor;
            ctx.build_prices = meta.build_prices;
            ctx.play_mode = meta.play_mode;
            self.coop = Some(ctx);

            // If the snapshot was captured while a blocking dialog or special
            // panel was awaiting an answer (`Saved` intentionally stores the
            // pre-tick world without `Lot::asking` / `Lot::asking_special_panel`),
            // re-run the tick probe using the exact frozen
            // `(session, activity_owners, free_will)` so `self.asking` /
            // `self.asking_special_panel` and `self.replay_view` match the host
            // identically.
            if (meta.active_dialog.is_some() || meta.active_special_panel.is_some())
                && !self.has_pending_dialog()
                && !self.answered
            {
                if let Some(frozen) = probe_frozen.as_ref()
                    && let Some(slot) = self.world.globals.get_mut(GLOBAL_FREE_WILL)
                {
                    *slot = i16::from(frozen.free_will);
                }
                match self.step_in(meta.play_mode) {
                    Err(StepError::Asking { .. }) => {
                        if meta.active_dialog.is_some() && self.asking.is_none() {
                            return Err(SaveError::Format(
                                "co-op snapshot expected an ordinary blocking dialog on restore, \
                                 but tick asked for a different prompt"
                                    .into(),
                            ));
                        }
                        if let Some((_, expected_shown)) = meta.active_special_panel.as_ref()
                            && self.asking_special_panel.as_ref().map(|a| &a.shown)
                                != Some(expected_shown)
                        {
                            return Err(SaveError::Format(format!(
                                "co-op snapshot expected special panel {expected_shown:?} on \
                                 restore, but tick asked {:?}",
                                self.asking_special_panel.as_ref().map(|a| &a.shown)
                            )));
                        }
                    }
                    Ok(_) => {
                        return Err(SaveError::Format(
                            "co-op snapshot expected an open blocking dialog/panel on restore, \
                             but tick completed without asking"
                                .into(),
                        ));
                    }
                    Err(err) => {
                        return Err(SaveError::Format(format!(
                            "replaying open co-op blocking dialog/panel failed: {err}"
                        )));
                    }
                }
            }

            if let Some(ctx) = self.coop.as_mut() {
                ctx.activity_owners = meta.activity_owners;
                ctx.session = session;
                self.view_rotation = ctx.host_camera_rotation();
            }

            if let Some(expected) = expected_fingerprint {
                let actual = self
                    .coop_state_fingerprint(&self.coop.as_ref().expect("coop installed").session);
                if actual != expected {
                    return Err(SaveError::Format(format!(
                        "restored co-op snapshot fingerprint {actual:?} did not match expected \
                         {expected:?}"
                    )));
                }
            }
            Ok(())
        })();

        if let Err(err) = apply_result {
            self.world = backup_world;
            self.ticks = backup_ticks;
            self.start = backup_start;
            self.player = backup_player;
            self.frames = backup_frames;
            self.play = backup_play;
            self.asks = backup_asks;
            self.view_rotation = backup_view_rotation;
            self.view_centre = backup_view_centre;
            self.replay_view = backup_replay_view;
            self.last_dialog_text = backup_last_dialog_text;
            self.character_names = backup_character_names;
            self.lot_name = backup_lot_name;
            self.lot_description = backup_lot_description;
            self.halted = backup_halted;
            self.asking = backup_asking;
            self.asking_special_panel = backup_asking_special_panel;
            self.special_panels = backup_special_panels;
            self.special_panel_responses = backup_special_panel_responses;
            self.answered = backup_answered;
            self.buy_undo = backup_buy_undo;
            self.buy_redo = backup_buy_redo;
            self.buy_orphans = backup_buy_orphans;
            self.active_buy_cursor = backup_active_buy_cursor;
            self.build_undo = backup_build_undo;
            self.build_redo = backup_build_redo;
            self.undo_order = backup_undo_order;
            self.redo_order = backup_redo_order;
            self.active_wall_cursor = backup_active_wall_cursor;
            self.neighborhood_origin = backup_neighborhood_origin;
            self.coop = backup_coop;
            return Err(err);
        }

        Ok(())
    }
}

fn require_priced_floor(prices: &BuildPrices, floor_id: u16) -> Result<(), String> {
    if floor_id == 0 || floor_id == SPLIT_FLOOR_ID || prices.floors.contains_key(&floor_id) {
        return Ok(());
    }
    Err(format!(
        "floor pattern {floor_id} is not in co-op BuildPrices catalog; attach with \
         Lot::attach_coop_with_install or build_prices_from_install"
    ))
}

fn require_priced_wall_pattern(prices: &BuildPrices, pattern_id: u16) -> Result<(), String> {
    if pattern_id == 0
        || (248..=255).contains(&pattern_id)
        || prices.walls.contains_key(&pattern_id)
    {
        return Ok(());
    }
    Err(format!(
        "wallpaper pattern {pattern_id} is not in co-op BuildPrices catalog; attach with \
         Lot::attach_coop_with_install or build_prices_from_install"
    ))
}

fn verify_committed_buy_placement(
    world: &mut World,
    held_id: u16,
    purchase: &BuyPurchase,
    expected_position: [i32; 3],
    expected_facing: i16,
    expected_container: Option<(u16, i16)>,
    label: &str,
) -> Result<(), String> {
    if !world.objects.contains_key(&purchase.object) {
        return Err(format!(
            "{label} placement object {} vanished during USER_PLACEMENT callback",
            purchase.object
        ));
    }
    let held_obj = world.objects.get(&held_id).ok_or_else(|| {
        format!("{label} placement object {held_id} vanished during USER_PLACEMENT callback")
    })?;
    let post_facing = held_obj
        .data
        .get(DATA_FACING)
        .copied()
        .unwrap_or(0)
        .rem_euclid(8);
    if post_facing != expected_facing || purchase.facing.rem_euclid(8) != expected_facing {
        return Err(format!(
            "{label} placement facing drifted after commit from requested {expected_facing} to \
             {post_facing} (record {})",
            purchase.facing
        ));
    }
    let place = crate::placement::capture(world, held_id)
        .map_err(|e| format!("{label} placement capture failed after commit: {e}"))?;
    let post_pos = [place.position[0], place.position[1], place.level];
    if !place.on_lot || post_pos != expected_position {
        return Err(format!(
            "{label} placement position {post_pos:?} (on_lot={}) did not match requested \
             {expected_position:?} after commit",
            place.on_lot
        ));
    }
    let post_container = place
        .container
        .and_then(|cid| (place.slot >= 0).then_some((cid, i16::try_from(place.slot).unwrap_or(0))));
    if post_container != expected_container {
        return Err(format!(
            "{label} placement container {post_container:?} did not match requested \
             {expected_container:?} after commit"
        ));
    }
    if held_id == purchase.object {
        if purchase.position != expected_position {
            return Err(format!(
                "{label} committed record position {:?} did not match requested \
                 {expected_position:?}",
                purchase.position
            ));
        }
        if purchase.container != expected_container {
            return Err(format!(
                "{label} committed record container {:?} did not match requested \
                 {expected_container:?}",
                purchase.container
            ));
        }
    }
    Ok(())
}

fn snapshot_selections(session: &SessionState) -> BTreeMap<PlayerId, Option<u16>> {
    session
        .players
        .iter()
        .map(|(&pid, p)| (pid, p.selected_sim))
        .collect()
}

fn record_selection_deltas(
    before: &BTreeMap<PlayerId, Option<u16>>,
    after: &SessionState,
    out: &mut BTreeMap<PlayerId, Option<u16>>,
) {
    for (&pid, p) in &after.players {
        let prev = before.get(&pid).copied().flatten();
        if prev != p.selected_sim {
            out.insert(pid, p.selected_sim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Program;
    use sims_content::Catalog;
    use sims_formats::behavior::{Behavior, Instruction};
    use sims_formats::saved_objects::{Frame, Interaction, Person};
    use sims_net::SessionConfig;
    use sims_sim::Object;

    fn test_person(id: u16, guid: u32, pos: [i32; 3]) -> Object {
        let idle = Interaction {
            join: 0,
            uid: 0,
            caller: 0,
            target: 0,
            icon: 0,
            table_index: -1,
            args: [0; 4],
            priority: 0,
            action: 0,
            attenuation: 0.0,
            flags: 0,
        };
        let person = Person {
            animation_events: 0,
            engaged: 0,
            appearance: Default::default(),
            accessories: Vec::new(),
            animations: Default::default(),
            routing_state: 0,
            floats: [0.0; 9],
            previous_motives: [0.0; 16],
            motives: [0.0; 16],
            data: vec![0; sims_sim::neighbors::PERSON_WORDS],
            routing_frames: 0,
            active: idle.clone(),
            last: idle,
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        };
        let mut data = vec![0; 128];
        data[11] = id as i16;
        Object {
            id,
            guid,
            footprint: [0; 4],
            position: pos,
            unknown_short: 0,
            attributes: vec![0; 8],
            temps: vec![0; 8],
            data,
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: Vec::new(),
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: Some(person),
            registry_word: None,
            last_result: false,
            removal_skips_resets: false,
            selected: false,
        }
    }

    fn test_object(id: u16, guid: u32, pos: [i32; 3]) -> Object {
        let mut data = vec![0; 128];
        data[11] = id as i16;
        Object {
            id,
            guid,
            footprint: [0; 4],
            position: pos,
            unknown_short: 0,
            attributes: vec![0; 8],
            temps: vec![0; 8],
            data,
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
        }
    }

    fn two_player_session(host_sim: Option<u16>, peer_sim: Option<u16>) -> SessionState {
        let mut session = SessionState::new("Host", None, SessionConfig::default());
        session.players.get_mut(&PlayerId(1)).unwrap().selected_sim = host_sim;
        session
            .players
            .get_mut(&PlayerId(1))
            .unwrap()
            .camera_rotation = CameraRotation::NORTH;
        session.players.insert(
            PlayerId(2),
            sims_net::PerPlayerState {
                player_id: PlayerId(2),
                name: "Peer".to_string(),
                role: PlayerRole::Peer,
                mode: PlayerMode::Live,
                selected_sim: peer_sim,
                camera_rotation: CameraRotation::SOUTH,
                endpoint_hint: None,
                transport_addr: None,
            },
        );
        session
    }

    fn run_expr_world() -> (Program, Catalog, World) {
        let make_tree = |args: u8, instructions: Vec<Instruction>| Behavior {
            format: 0x8002,
            kind: 0,
            args,
            locals: 0,
            revision: 0,
            instructions,
        };
        let mut behaviors = BTreeMap::new();
        // Tree 4096: Expression `Temp[0] := Global[3]`
        behaviors.insert(
            4096,
            make_tree(
                0,
                vec![Instruction {
                    opcode: 2,
                    on_true: 0xfe,
                    on_false: 0xff,
                    operands: [0, 0, 3, 0, 0, 5, 8, 6],
                }],
            ),
        );
        // Tree 4097: Expression `MyData[11] == Global[3]`
        behaviors.insert(
            4097,
            make_tree(
                0,
                vec![Instruction {
                    opcode: 2,
                    on_true: 0xfe,
                    on_false: 0xff,
                    operands: [11, 0, 3, 0, 0, 2, 3, 6],
                }],
            ),
        );
        // Tree 4098: Expression `Global[3] := Parameter[0]`
        behaviors.insert(
            4098,
            make_tree(
                1,
                vec![Instruction {
                    opcode: 2,
                    on_true: 0xfe,
                    on_false: 0xff,
                    operands: [3, 0, 0, 0, 0, 5, 6, 9],
                }],
            ),
        );
        // Tree 4099: Generic Sims Call 12 (tile_depth -> Temp[0])
        behaviors.insert(
            4099,
            make_tree(
                0,
                vec![Instruction {
                    opcode: 1,
                    on_true: 0xfe,
                    on_false: 0xff,
                    operands: [12, 0, 0, 0, 0, 0, 0, 0],
                }],
            ),
        );

        let mut libraries = BTreeMap::new();
        libraries.insert(
            0,
            crate::Library {
                behaviors,
                ..crate::Library::default()
            },
        );
        let program = Program {
            global: 0,
            libraries,
            semi_globals: BTreeMap::new(),
            careers: None,
            career_texts_end: false,
            labels_hold_no_token: false,
            zoning: BTreeMap::new(),
            animations: Default::default(),
            people: Default::default(),
            icon_frames: None,
            wardrobe: None,
            ordered_wardrobe: None,
            character_files: Default::default(),
            expansions: 0,
            characters: Default::default(),
        };
        let objects_catalog = vec![
            sims_content::Object {
                guid: 0x1001,
                resource: 0,
                chunk: 1,
                kind: 2,
                master: 0,
            },
            sims_content::Object {
                guid: 0x1002,
                resource: 0,
                chunk: 2,
                kind: 2,
                master: 0,
            },
            sims_content::Object {
                guid: 0x1003,
                resource: 0,
                chunk: 3,
                kind: 2,
                master: 0,
            },
            sims_content::Object {
                guid: 0x2001,
                resource: 0,
                chunk: 4,
                kind: 4,
                master: 0,
            },
        ];
        let by_guid = objects_catalog
            .iter()
            .enumerate()
            .map(|(i, o)| (o.guid, i))
            .collect();
        let catalog = Catalog {
            global: 0,
            objects: objects_catalog,
            by_guid,
            semi_globals: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
        let mut world = World::empty();
        world.size = 64;
        world.globals = vec![0; 64];
        world.globals[GLOBAL_SELECTED] = 10;
        world.types.insert(1, 0x1001);
        world.types.insert(2, 0x1002);
        world.types.insert(3, 0x1003);
        world.types.insert(4, 0x2001);
        world
            .objects
            .insert(10, test_person(10, 0x1001, [160, 320, 1]));
        world
            .objects
            .insert(20, test_person(20, 0x1002, [160, 320, 1]));
        world
            .objects
            .insert(30, test_person(30, 0x1003, [160, 320, 1]));
        world
            .objects
            .insert(40, test_object(40, 0x2001, [160, 320, 1]));
        world.order = vec![10, 20, 30, 40];
        world.people = vec![10, 20, 30];
        world.objects.get_mut(&10).unwrap().selected = true;
        let _ = std::mem::size_of::<Frame>();
        (program, catalog, world)
    }

    #[test]
    fn global_3_policy_answers_per_player_selected_sim_and_activity_owner() {
        let (program, catalog, mut world) = run_expr_world();
        let mut ctx = CoopVmContext::new(two_player_session(Some(10), Some(20)));
        ctx.record_activity_owner(40, PlayerId(2));
        let coop_cell = RefCell::new(ctx);
        let interp = Interpreter {
            coop: Some(&coop_cell),
            ..Interpreter::new(&program, &catalog, Stream::Lot)
        };

        assert!(interp.run_tree(&mut world, 10, 4097, 0, &[], 100).unwrap());
        assert!(interp.run_tree(&mut world, 20, 4097, 0, &[], 100).unwrap());
        assert!(!interp.run_tree(&mut world, 30, 4097, 0, &[], 100).unwrap());

        interp.run_tree(&mut world, 40, 4096, 0, &[], 100).unwrap();
        assert_eq!(world.objects[&40].temps[0], 20);

        coop_cell.borrow_mut().activity_owners.clear();
        interp.run_tree(&mut world, 40, 4096, 0, &[], 100).unwrap();
        assert_eq!(world.objects[&40].temps[0], 10);
    }

    #[test]
    fn global_3_script_write_updates_only_affected_players_and_reconciles_host_projection() {
        let (program, catalog, mut world) = run_expr_world();
        let coop_cell = RefCell::new(CoopVmContext::new(two_player_session(Some(10), Some(20))));
        let interp = Interpreter {
            coop: Some(&coop_cell),
            ..Interpreter::new(&program, &catalog, Stream::Lot)
        };

        // Sim 20 (selected by Peer 2) writes Global[3] := 30.
        assert!(
            interp
                .run_tree(&mut world, 20, 4098, 0, &[30], 100)
                .unwrap()
        );
        {
            let ctx = coop_cell.borrow();
            assert_eq!(ctx.session.players[&PlayerId(1)].selected_sim, Some(10));
            assert_eq!(ctx.session.players[&PlayerId(2)].selected_sim, Some(30));
        }
        assert_eq!(world.selected(), Some(10));
        assert_eq!(world.globals[GLOBAL_SELECTED], 10);

        // Sim 10 (selected by Host 1) writes Global[3] := 20 -> host projection
        // (`world.selected()` + `world.globals[3]`) must reconcile immediately!
        assert!(
            interp
                .run_tree(&mut world, 10, 4098, 0, &[20], 100)
                .unwrap()
        );
        assert_eq!(
            coop_cell.borrow().session.players[&PlayerId(1)].selected_sim,
            Some(20)
        );
        assert_eq!(world.selected(), Some(20));
        assert_eq!(world.globals[GLOBAL_SELECTED], 20);
    }

    #[test]
    fn per_player_camera_rotation_drives_tile_depth_and_unselected_sim_uses_host() {
        let (program, catalog, mut world) = run_expr_world();
        let mut ctx = CoopVmContext::new(two_player_session(Some(10), Some(20)));
        // Even if Peer 2 owns an activity on unselected Sim 30, Sim 30 is a
        // person and must strictly fall back to the host's rotation (NORTH = 0),
        // even when `stack_object` is Peer 2's selected Sim (20)!
        ctx.record_activity_owner(30, PlayerId(2));
        let coop_cell = RefCell::new(ctx);
        let interp = Interpreter {
            view_rotation: 0,
            coop: Some(&coop_cell),
            ..Interpreter::new(&program, &catalog, Stream::Lot)
        };

        interp.run_tree(&mut world, 10, 4099, 40, &[], 100).unwrap();
        interp.run_tree(&mut world, 20, 4099, 40, &[], 100).unwrap();
        interp.run_tree(&mut world, 30, 4099, 20, &[], 100).unwrap();
        assert_eq!(world.objects[&10].temps[0], -30);
        assert_eq!(world.objects[&20].temps[0], -96);
        assert_eq!(
            world.objects[&30].temps[0], -30,
            "unselected Sim must never borrow another player's camera"
        );
    }

    #[test]
    fn single_player_leaves_global_3_read_only() {
        let (program, catalog, mut world) = run_expr_world();
        let interp = Interpreter::new(&program, &catalog, Stream::Lot);
        let err = interp.run_tree(&mut world, 10, 4098, 0, &[30], 100);
        assert!(err.is_err(), "single-player must keep Global 3 read-only");
        assert_eq!(world.globals[GLOBAL_SELECTED], 10);
    }

    #[test]
    fn player_mode_maps_to_simulator_global_19_encoding() {
        assert_eq!(player_mode_to_panel_mode(PlayerMode::Live), PanelMode::Live);
        assert_eq!(
            player_mode_to_panel_mode(PlayerMode::Build),
            PanelMode::Build
        );
        assert_eq!(player_mode_to_panel_mode(PlayerMode::Buy), PanelMode::Buy);
    }

    #[test]
    fn resolve_cancel_uid_handles_large_serials_and_rejects_wrap_collision() {
        let (_, _, mut world) = run_expr_world();
        let person = world.objects.get_mut(&10).unwrap().person.as_mut().unwrap();
        person.active.uid = 40_000;
        let mut q1 = person.active.clone();
        q1.uid = 100_000;
        person.queue.push(q1);

        assert_eq!(
            resolve_cancel_uid(&world, 10, cancel_uid_to_wire(40_000)).unwrap(),
            Some(40_000)
        );
        assert_eq!(
            resolve_cancel_uid(&world, 10, cancel_uid_to_wire(100_000)).unwrap(),
            Some(100_000)
        );
        assert_eq!(resolve_cancel_uid(&world, 10, 123).unwrap(), None);

        // Two interactions sharing low 16 bits (40_000 and 40_000 + 65_536) must
        // refuse deterministically rather than cancelling the wrong one.
        let person = world.objects.get_mut(&10).unwrap().person.as_mut().unwrap();
        let mut q2 = person.active.clone();
        q2.uid = 40_000 + 65_536;
        person.queue.push(q2);
        assert!(resolve_cancel_uid(&world, 10, cancel_uid_to_wire(40_000)).is_err());
    }

    fn synthetic_coop_lot(
        program: Program,
        catalog: impl Into<std::sync::Arc<Catalog>>,
        world: World,
        frames: Frames,
    ) -> Lot {
        use crate::boot::{Finished, LoadReport};
        use crate::play::Play;
        use sims_sim::motives::{Curve, Rates, Tuning};
        let rates = || Rates::from_lookup(0, |_| None);
        let curve = |motive| Curve::parse(motive, "(-100;1)(100;1)", 8);
        let tuning = Tuning {
            rates: [rates(), rates(), rates()],
            adult_mood: vec![curve(7)],
            child_mood: vec![curve(7)],
        };
        let catalog: std::sync::Arc<Catalog> = catalog.into();
        let fast = crate::program::ProgramFast::build(&program, &catalog);
        Lot {
            program,
            fast,
            catalog,
            surfaces: sims_content::Surfaces::default(),
            architecture_prices: crate::lifecycle::LotPrices::default(),
            world,
            tuning: Some(tuning),
            player: Answers::default(),
            frames,
            ticks: 0,
            start: 0,
            load: LoadReport {
                finished: Some(Finished {
                    indicators_made: false,
                    sim: Some(10),
                    counted: 0,
                }),
                ..LoadReport::default()
            },
            halted: false,
            play: Play::default(),
            asks: false,
            answered: false,
            asking: None,
            asking_special_panel: None,
            special_panels: crate::SpecialPanels::default(),
            special_panel_responses: std::collections::VecDeque::new(),
            view_rotation: 0,
            floor_rotation: 0,
            view_centre: None,
            replay_view: None,
            last_dialog_text: String::new(),
            character_names: BTreeMap::new(),
            default_character_name: String::new(),
            lot_name: String::new(),
            lot_description: String::new(),
            loaded: [0; 32],
            buy_undo: Vec::new(),
            buy_redo: Vec::new(),
            buy_orphans: Vec::new(),
            active_buy_cursor: None,
            build_undo: Vec::new(),
            build_redo: Vec::new(),
            undo_order: Vec::new(),
            redo_order: Vec::new(),
            active_wall_cursor: None,
            neighborhood_origin: None,
            coop: None,
            move_objects: false,
        }
    }

    #[test]
    fn tutorial_lot_refused_and_duplicate_batch_rejected() {
        let (program, catalog, mut world) = run_expr_world();
        world.globals[GLOBAL_TUTORIAL] = 1;
        let mut lot =
            synthetic_coop_lot(program.clone(), catalog.clone(), world, Frames::default());
        let session = two_player_session(Some(10), Some(20));
        assert!(
            lot.attach_coop(session.clone()).is_err(),
            "tutorial lots (Global 26 != 0) must be rejected in co-op mode"
        );

        let (_, _, mut world2) = run_expr_world();
        for obj in world2.objects.values_mut() {
            obj.set_flags(SCHEDULER_READY, false);
        }
        let mut lot2 = synthetic_coop_lot(program, catalog, world2, Frames::parse("none").unwrap());
        let mut s = session;
        lot2.attach_coop_context(
            CoopVmContext::new(s.clone()).with_frames(Frames::parse("none").unwrap()),
        )
        .expect("attach");
        let batch1 = TickBatch {
            epoch: s.epoch,
            tick: Tick(1),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: Vec::new(),
        };
        let applied1 = s.apply_batch(&batch1).expect("apply batch 1");
        let _ = lot2.apply_coop_batch(&applied1, &mut s).expect("tick 1");
        // Re-applying tick 1 must fail deterministically!
        let err = lot2.apply_coop_batch(&applied1, &mut s).unwrap_err();
        assert!(matches!(
            err,
            CoopError::Session(SessionError::UnexpectedTick {
                expected: Tick(2),
                actual: Tick(1)
            })
        ));
    }

    #[test]
    fn blocking_dialog_same_batch_valid_answer_frozen_replay_and_snapshot() {
        use sims_sim::Dialog;
        // Build a synthetic non-tutorial lot with a pending 2-button modal
        // dialog record on object 40 (offers statuses [2, 3], kind 1).
        let (mut program, catalog, mut world) = run_expr_world();
        // Give object 40 a definition so scheduler/end-of-update handles it cleanly.
        for chunk in 1..=4 {
            program
                .libraries
                .get_mut(&0)
                .unwrap()
                .definitions
                .insert(chunk, [0; crate::DEFINITION_WORDS]);
        }
        for obj in world.objects.values_mut() {
            obj.set_flags(SCHEDULER_READY, false);
        }
        // Push a NEW modal Yes/No dialog (kind 1, modal bit 7 clear, non-inform bit 0 clear).
        world.dialogs.records.push(Dialog {
            caller: 40,
            operands: [0, 0, 0, 0, 0, 1, 0, 0],
            status: 0,
        });

        let mut host_lot = synthetic_coop_lot(
            program.clone(),
            catalog.clone(),
            world.clone(),
            Frames::parse("none").unwrap(),
        );
        let mut peer_lot = synthetic_coop_lot(
            program,
            catalog,
            world,
            Frames::default(), // intentionally different local schedule!
        );

        let mut session = two_player_session(Some(10), Some(20));
        let host_ctx =
            CoopVmContext::new(session.clone()).with_frames(Frames::parse("none").unwrap());
        host_lot.attach_coop_context(host_ctx).expect("attach host");

        // Tick 1: stepping hits the non-tutorial blocking dialog and freezes `FrozenDialogTick`.
        let b1 = TickBatch {
            epoch: session.epoch,
            tick: Tick(1),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: Vec::new(),
        };
        let a1 = session.apply_batch(&b1).unwrap();
        let r1 = host_lot.apply_coop_batch(&a1, &mut session).unwrap();
        assert!(!r1.stepped);
        let (dlg_id, shown) = r1.opened_dialog.expect("blocking dialog opens");
        assert_eq!(shown.answers, &[2, 3]);
        assert!(session.is_paused());

        // Intervening changes while the dialog waits: Free Will + selection + camera rotation.
        let b2 = TickBatch {
            epoch: session.epoch,
            tick: Tick(2),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: vec![
                StampedCommand {
                    tick: Tick(2),
                    sequence: 0,
                    issuer: PlayerId(1),
                    command: Command::SetFreeWill { enabled: false },
                },
                StampedCommand {
                    tick: Tick(2),
                    sequence: 1,
                    issuer: PlayerId(2),
                    command: Command::SelectPerson { sim_id: Some(30) },
                },
                StampedCommand {
                    tick: Tick(2),
                    sequence: 2,
                    issuer: PlayerId(2),
                    command: Command::SetCameraRotation {
                        rotation: CameraRotation::WEST,
                    },
                },
            ],
        };
        let a2 = session.apply_batch(&b2).unwrap();
        let r2 = host_lot.apply_coop_batch(&a2, &mut session).unwrap();
        assert!(!r2.stepped);
        // Pre-tick world must stay untouched while frozen (Free Will still 0/original!).
        assert_eq!(host_lot.world().globals[GLOBAL_FREE_WILL], 1);

        // Mid-dialog snapshot restore onto peer with DIFFERENT local `Frames`:
        let snap = host_lot.coop_snapshot_bytes().expect("mid-dialog snap");
        let fp2 = host_lot.coop_state_fingerprint(&session);
        peer_lot
            .restore_coop_snapshot_verified(&snap, session.clone(), fp2)
            .expect("verified mid-dialog restore");
        assert!(peer_lot.asking().is_some());
        assert_eq!(
            peer_lot.coop().unwrap().frames,
            Frames::parse("none").unwrap()
        );

        // Tick 3: invalid answer (status 99) from Host followed by VALID answer (status 3)
        // from Peer in the SAME batch, PLUS explicit pause so replay waits!
        let mut peer_session = session.clone();
        let b3 = TickBatch {
            epoch: session.epoch,
            tick: Tick(3),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: vec![
                StampedCommand {
                    tick: Tick(3),
                    sequence: 0,
                    issuer: PlayerId(1),
                    command: Command::SetPaused { paused: true },
                },
                StampedCommand {
                    tick: Tick(3),
                    sequence: 1,
                    issuer: PlayerId(1),
                    command: Command::AnswerBlockingDialog {
                        dialog_id: dlg_id,
                        answer: sims_net::DialogAnswer {
                            status: 99,
                            bitmap_id: None,
                            text: None,
                        },
                    },
                },
                StampedCommand {
                    tick: Tick(3),
                    sequence: 2,
                    issuer: PlayerId(2),
                    command: Command::AnswerBlockingDialog {
                        dialog_id: dlg_id,
                        answer: sims_net::DialogAnswer {
                            status: 3,
                            bitmap_id: None,
                            text: None,
                        },
                    },
                },
            ],
        };
        let a3_h = session.apply_batch(&b3).unwrap();
        let a3_p = peer_session.apply_batch(&b3).unwrap();
        let r3_h = host_lot.apply_coop_batch(&a3_h, &mut session).unwrap();
        let r3_p = peer_lot.apply_coop_batch(&a3_p, &mut peer_session).unwrap();
        assert!(!r3_h.stepped, "explicit pause holds replay until unpaused");
        assert!(matches!(
            r3_h.command_results[1].1,
            CoopCommandStatus::Refused(_)
        ));
        assert!(matches!(
            r3_h.command_results[2].1,
            CoopCommandStatus::DialogAnswered {
                winner: PlayerId(2),
                ..
            }
        ));
        assert_eq!(r3_h.state_fingerprint, r3_p.state_fingerprint);
        assert!(session.open_dialogs.is_empty());

        // Answered-but-paused snapshot restore must also match bit-for-bit!
        let snap_ans = host_lot.coop_snapshot_bytes().expect("answered snap");
        let mut joiner = synthetic_coop_lot(
            host_lot.program.clone(),
            host_lot.catalog.clone(),
            World::empty(),
            Frames::default(),
        );
        joiner
            .restore_coop_snapshot_verified(&snap_ans, session.clone(), r3_h.state_fingerprint)
            .expect("answered-but-paused snapshot restore");

        // Tick 4: unpause -> replay runs with frozen pre-tick context, then applies
        // deferred Free Will (false) and preserves Peer 2's intervening selection (30)!
        let mut joiner_session = session.clone();
        let b4 = TickBatch {
            epoch: session.epoch,
            tick: Tick(4),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: vec![StampedCommand {
                tick: Tick(4),
                sequence: 0,
                issuer: PlayerId(1),
                command: Command::SetPaused { paused: false },
            }],
        };
        let a4_h = session.apply_batch(&b4).unwrap();
        let a4_p = peer_session.apply_batch(&b4).unwrap();
        let a4_j = joiner_session.apply_batch(&b4).unwrap();
        let r4_h = host_lot.apply_coop_batch(&a4_h, &mut session).unwrap();
        let r4_p = peer_lot.apply_coop_batch(&a4_p, &mut peer_session).unwrap();
        let r4_j = joiner.apply_coop_batch(&a4_j, &mut joiner_session).unwrap();
        assert!(r4_h.stepped && r4_p.stepped && r4_j.stepped);
        assert_eq!(r4_h.state_fingerprint, r4_p.state_fingerprint);
        assert_eq!(r4_h.state_fingerprint, r4_j.state_fingerprint);
        assert_eq!(
            host_lot.world().globals[GLOBAL_FREE_WILL],
            0,
            "deferred Free Will change must apply once dialog replay completes"
        );
        assert_eq!(
            session.players[&PlayerId(2)].selected_sim,
            Some(30),
            "intervening selection must be preserved after replay"
        );
    }

    #[test]
    fn sequential_batch_selection_and_mode_gating() {
        let (program, catalog, mut world) = run_expr_world();
        for obj in world.objects.values_mut() {
            obj.set_flags(SCHEDULER_READY, false);
        }
        let mut lot = synthetic_coop_lot(program, catalog, world, Frames::parse("none").unwrap());
        let mut session = two_player_session(Some(10), Some(20));
        lot.attach_coop_context(
            CoopVmContext::new(session.clone()).with_frames(Frames::parse("none").unwrap()),
        )
        .expect("attach");

        // Same batch: Peer 2 selects invalid ID 9999 -> refused WITHOUT clobbering 20;
        // then selects valid 30 -> applied; then selects invalid 8888 -> refused WITHOUT clobbering 30!
        // Also BuyBuild in Live mode -> refused!
        let b1 = TickBatch {
            epoch: session.epoch,
            tick: Tick(1),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: vec![
                StampedCommand {
                    tick: Tick(1),
                    sequence: 0,
                    issuer: PlayerId(2),
                    command: Command::SelectPerson { sim_id: Some(9999) },
                },
                StampedCommand {
                    tick: Tick(1),
                    sequence: 1,
                    issuer: PlayerId(2),
                    command: Command::SelectPerson { sim_id: Some(30) },
                },
                StampedCommand {
                    tick: Tick(1),
                    sequence: 2,
                    issuer: PlayerId(2),
                    command: Command::SelectPerson { sim_id: Some(8888) },
                },
                StampedCommand {
                    tick: Tick(1),
                    sequence: 3,
                    issuer: PlayerId(1),
                    command: Command::BuyBuild(BuyBuildEdit::PlaceWall {
                        x: 5,
                        y: 5,
                        level: 1,
                        style: 1,
                        pattern: 0,
                    }),
                },
            ],
        };
        let a1 = session.apply_batch(&b1).unwrap();
        let r1 = lot.apply_coop_batch(&a1, &mut session).unwrap();
        assert!(matches!(
            r1.command_results[0].1,
            CoopCommandStatus::Refused(_)
        ));
        assert_eq!(r1.command_results[1].1, CoopCommandStatus::Applied);
        assert!(matches!(
            r1.command_results[2].1,
            CoopCommandStatus::Refused(_)
        ));
        assert!(matches!(
            r1.command_results[3].1,
            CoopCommandStatus::Refused(_)
        ));
        assert_eq!(
            session.players[&PlayerId(2)].selected_sim,
            Some(30),
            "invalid SelectPerson must never clobber valid selection in the same batch"
        );
    }

    #[test]
    fn restore_coop_snapshot_is_transactional_on_error() {
        let (program, catalog, mut world) = run_expr_world();
        for obj in world.objects.values_mut() {
            obj.set_flags(SCHEDULER_READY, false);
        }
        let mut lot = synthetic_coop_lot(program, catalog, world, Frames::parse("none").unwrap());
        let mut session = two_player_session(Some(10), Some(20));
        lot.attach_coop_context(
            CoopVmContext::new(session.clone()).with_frames(Frames::parse("none").unwrap()),
        )
        .expect("attach");
        let before_fp = lot.coop_state_fingerprint(&session);
        let snap = lot.coop_snapshot_bytes().unwrap();

        // Corrupt expected fingerprint -> restore must fail AND leave `lot` completely intact!
        let bad_fp = StateFingerprint([0xaa; 32]);
        assert!(
            lot.restore_coop_snapshot_verified(&snap, session.clone(), bad_fp)
                .is_err()
        );
        assert_eq!(lot.coop_state_fingerprint(&session), before_fp);

        // Open dialog in session without dialog metadata in raw Saved -> refused & intact!
        session.open_blocking_dialog(DialogId(1), 40, Tick(1));
        let raw_saved = lot.save().unwrap().encode();
        assert!(lot.restore_coop_snapshot(&raw_saved, session).is_err());
        assert!(lot.coop().is_some());
    }

    #[test]
    fn frozen_dialog_replay_preserves_intervening_reselection_and_idle_speed() {
        use sims_sim::Dialog;
        let (mut program, catalog, mut world) = run_expr_world();
        // Residential zoning so `World::idle_speed(true)` transitions
        // `(globals[16], globals[18])` from `(760, -1)` to `(1000, 760)` when
        // all Sims are sleeping (`motives[11] < 0`).
        world.globals[10] = 5;
        world.globals[18] = -1;
        program.zoning.insert(5, 1);
        // Tree 4100 on Sim 20:
        //   0: `Global[3] := 30` (script rewrites everyone who had Sim 20 selected -> Sim 30)
        //   1: `Sleep(Parameter[0])` (yield cleanly so the outermost thread never returns)
        program.libraries.get_mut(&0).unwrap().behaviors.insert(
            4100,
            Behavior {
                format: 0x8002,
                kind: 0,
                args: 1,
                locals: 0,
                revision: 0,
                instructions: vec![
                    Instruction {
                        opcode: 2,
                        on_true: 1,
                        on_false: 0xff,
                        operands: [3, 0, 30, 0, 0, 5, 6, 7],
                    },
                    Instruction {
                        opcode: 0,
                        on_true: 1,
                        on_false: 1,
                        operands: [0; 8],
                    },
                ],
            },
        );
        for chunk in 1..=4 {
            program
                .libraries
                .get_mut(&0)
                .unwrap()
                .definitions
                .insert(chunk, [0; crate::DEFINITION_WORDS]);
        }
        for obj in world.objects.values_mut() {
            obj.registry_word = Some(0);
            obj.set_flags(SCHEDULER_READY, false);
            if let Some(person) = obj.person.as_mut() {
                person.motives[11] = -10.0;
            }
        }
        // Object 40 runs Tree 4100 (with `stack_object = 20`) during the tick
        // that also opens a dialog.
        {
            let obj40 = world.objects.get_mut(&40).unwrap();
            obj40.registry_word = Some(0);
            obj40.set_countdown(-1);
            obj40.set_flags(SCHEDULER_READY, true);
            assert_ne!(obj40.flags() & SCHEDULER_READY, 0);
            obj40.stack.push(Frame {
                tree: 4100,
                node: 0,
                stack_object: 20,
                parameters: vec![5],
                locals: Vec::new(),
                primitive_state: 0,
                code_owner_type: 4,
            });
        }
        world.dialogs.records.push(Dialog {
            caller: 40,
            operands: [0, 0, 0, 0, 0, 1, 0, 0],
            status: 0,
        });

        let mut host_lot = synthetic_coop_lot(
            program.clone(),
            catalog.clone(),
            world.clone(),
            Frames::parse("none").unwrap(),
        );
        let mut peer_lot =
            synthetic_coop_lot(program, catalog, world, Frames::parse("none").unwrap());

        // Both Peer 2 and Peer 3 start on Sim 20.
        let mut session = two_player_session(Some(10), Some(20));
        session.players.insert(
            PlayerId(3),
            sims_net::PerPlayerState {
                player_id: PlayerId(3),
                name: "Carol".to_string(),
                role: PlayerRole::Peer,
                mode: PlayerMode::Live,
                selected_sim: Some(20),
                camera_rotation: CameraRotation::EAST,
                endpoint_hint: None,
                transport_addr: None,
            },
        );
        host_lot
            .attach_coop_context(
                CoopVmContext::new(session.clone()).with_frames(Frames::parse("none").unwrap()),
            )
            .expect("attach");
        assert_eq!(host_lot.world().globals[16], 760);

        // Tick 1: dialog opens -> pre-tick context frozen.
        let b1 = TickBatch {
            epoch: session.epoch,
            tick: Tick(1),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: Vec::new(),
        };
        let a1 = session.apply_batch(&b1).unwrap();
        let r1 = host_lot.apply_coop_batch(&a1, &mut session).unwrap();
        let (dlg_id, _) = r1.opened_dialog.expect("dialog opens");

        // Tick 2: Peer 2 switches 20 -> 30 -> 20 (back to original `pre_sel == Some(20)`!),
        // Peer 3 issues NO SelectPerson, and NO SetSpeed command is issued.
        let b2 = TickBatch {
            epoch: session.epoch,
            tick: Tick(2),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: vec![
                StampedCommand {
                    tick: Tick(2),
                    sequence: 0,
                    issuer: PlayerId(2),
                    command: Command::SelectPerson { sim_id: Some(30) },
                },
                StampedCommand {
                    tick: Tick(2),
                    sequence: 1,
                    issuer: PlayerId(2),
                    command: Command::SelectPerson { sim_id: Some(20) },
                },
            ],
        };
        let a2 = session.apply_batch(&b2).unwrap();
        let r2 = host_lot.apply_coop_batch(&a2, &mut session).unwrap();
        assert!(!r2.stepped);

        // Snapshot mid-dialog -> joiner must preserve `intervening_selections`.
        let snap = host_lot.coop_snapshot_bytes().unwrap();
        peer_lot
            .restore_coop_snapshot_verified(&snap, session.clone(), r2.state_fingerprint)
            .expect("mid-dialog snapshot restore with intervening_selections");

        // Tick 3: answer dialog -> tick replays Tree 4100 (rewriting Global 3 -> 30)
        // and runs `World::idle_speed(true)` (`globals[16] = 1000`, `globals[18] = 760`).
        let mut peer_session = session.clone();
        let b3 = TickBatch {
            epoch: session.epoch,
            tick: Tick(3),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: vec![StampedCommand {
                tick: Tick(3),
                sequence: 0,
                issuer: PlayerId(1),
                command: Command::AnswerBlockingDialog {
                    dialog_id: dlg_id,
                    answer: sims_net::DialogAnswer {
                        status: 2,
                        bitmap_id: None,
                        text: None,
                    },
                },
            }],
        };
        let a3_h = session.apply_batch(&b3).unwrap();
        let a3_p = peer_session.apply_batch(&b3).unwrap();
        let r3_h = host_lot.apply_coop_batch(&a3_h, &mut session).unwrap();
        let r3_p = peer_lot.apply_coop_batch(&a3_p, &mut peer_session).unwrap();
        assert!(r3_h.stepped && r3_p.stepped);
        assert_eq!(r3_h.state_fingerprint, r3_p.state_fingerprint);
        assert_eq!(
            session.players[&PlayerId(2)].selected_sim,
            Some(20),
            "replayed script must NOT overwrite Peer 2's explicit intervening re-selection of Sim 20"
        );
        assert_eq!(
            session.players[&PlayerId(3)].selected_sim,
            Some(30),
            "replayed script MUST still update Peer 3 who issued no intervening SelectPerson"
        );
        assert_eq!(
            host_lot.world().globals[16],
            1000,
            "replay without SetSpeed must preserve residential idle-speed Global 16 (1000)"
        );
        assert_eq!(
            host_lot.world().globals[18],
            760,
            "replay without SetSpeed must preserve residential idle-speed return Global 18 (760)"
        );
    }

    #[test]
    fn command_callback_error_preserves_batch_tick_continuity() {
        let (program, catalog, mut world) = run_expr_world();
        for obj in world.objects.values_mut() {
            obj.set_flags(SCHEDULER_READY, false);
        }
        // Add indicator object 50 and occupy Sim 20's indicator slot (slots[2])
        // with object 40 so `Interpreter::select(Some(20))` faults cleanly.
        let mut ind = test_object(50, 0x2001, [-16, -16, 1]);
        ind.set_flags(SCHEDULER_READY, false);
        world.objects.insert(50, ind);
        world.indicators = [50, 0];
        world.objects.get_mut(&20).unwrap().slots = vec![
            sims_formats::saved_objects::Slot {
                unknown: 0,
                object: 0,
            },
            sims_formats::saved_objects::Slot {
                unknown: 0,
                object: 0,
            },
            sims_formats::saved_objects::Slot {
                unknown: 0,
                object: 40,
            },
        ];

        let mut lot = synthetic_coop_lot(program, catalog, world, Frames::parse("none").unwrap());
        let mut session = two_player_session(Some(10), Some(20));
        lot.attach_coop_context(
            CoopVmContext::new(session.clone()).with_frames(Frames::parse("none").unwrap()),
        )
        .expect("attach");

        // Tick 1: Host attempts to select Sim 20 -> indicator slot fault must be
        // returned as `Refused` (rolling back host selection to 10) WITHOUT
        // stranding `current_tick` behind lockstep!
        let b1 = TickBatch {
            epoch: session.epoch,
            tick: Tick(1),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: vec![StampedCommand {
                tick: Tick(1),
                sequence: 0,
                issuer: PlayerId(1),
                command: Command::SelectPerson { sim_id: Some(20) },
            }],
        };
        let a1 = session.apply_batch(&b1).unwrap();
        let r1 = lot
            .apply_coop_batch(&a1, &mut session)
            .expect("batch 1 must complete instead of stranding tick continuity");
        assert!(matches!(
            r1.command_results[0].1,
            CoopCommandStatus::Refused(_)
        ));
        assert_eq!(
            session.players[&PlayerId(1)].selected_sim,
            Some(10),
            "failed host selection must roll back cleanly"
        );

        // Tick 2 must succeed cleanly with zero tick mismatch!
        let b2 = TickBatch {
            epoch: session.epoch,
            tick: Tick(2),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: Vec::new(),
        };
        let a2 = session.apply_batch(&b2).unwrap();
        let r2 = lot
            .apply_coop_batch(&a2, &mut session)
            .expect("tick 2 must continue smoothly");
        assert_eq!(r2.tick, Tick(2));
    }

    #[test]
    fn failover_reconciliation_drops_unreachable_candidates_and_rejects_unbacked_epoch() {
        let (program, catalog, mut world) = run_expr_world();
        for obj in world.objects.values_mut() {
            obj.set_flags(SCHEDULER_READY, false);
        }
        let mut lot = synthetic_coop_lot(program, catalog, world, Frames::parse("none").unwrap());
        let mut session = two_player_session(Some(10), Some(20));
        // Add Player 3; put unreachable Player 2 in Build mode (holding a pause!).
        session.players.get_mut(&PlayerId(2)).unwrap().mode = PlayerMode::Build;
        session.players.insert(
            PlayerId(3),
            sims_net::PerPlayerState {
                player_id: PlayerId(3),
                name: "Carol".to_string(),
                role: PlayerRole::Peer,
                mode: PlayerMode::Live,
                selected_sim: Some(30),
                camera_rotation: CameraRotation::EAST,
                endpoint_hint: None,
                transport_addr: None,
            },
        );
        let mut ctx =
            CoopVmContext::new(session.clone()).with_frames(Frames::parse("none").unwrap());
        ctx.record_activity_owner(40, PlayerId(2));
        lot.attach_coop_context(ctx).expect("attach");

        // Unbacked future epoch (same host P1, epoch jumped to 99) -> MUST fail!
        let mut tampered_session = session.clone();
        tampered_session.epoch = sims_net::Epoch(99);
        let bad_batch = TickBatch {
            epoch: sims_net::Epoch(99),
            tick: Tick(1),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: Vec::new(),
        };
        let bad_applied = tampered_session.apply_batch(&bad_batch).unwrap();
        assert!(matches!(
            lot.apply_coop_batch(&bad_applied, &mut tampered_session),
            Err(CoopError::Session(SessionError::EpochMismatch { .. }))
        ));

        // Real LockstepNode failover: promote Player 3 AND drop unreachable Player 2!
        session.promote_host(PlayerId(3)).expect("promote P3");
        session.players.remove(&PlayerId(2));
        assert!(!session.is_paused());

        let b1 = TickBatch {
            epoch: session.epoch,
            tick: Tick(1),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: Vec::new(),
        };
        let a1 = session.apply_batch(&b1).unwrap();
        let r1 = lot.apply_coop_batch(&a1, &mut session).unwrap();
        assert!(
            r1.stepped,
            "simulation must unpause once unreachable Build-mode Player 2 is dropped"
        );
        assert!(
            !session.players.contains_key(&PlayerId(2)),
            "unreachable Player 2 must NOT be resurrected by VM failover reconciliation"
        );
        assert!(
            !lot.coop().unwrap().activity_owners.contains_key(&40),
            "activity_owners for unreachable Player 2 must be cleaned"
        );
    }

    #[test]
    fn player_joined_canonicalizes_invite_ticket_transport_addr() {
        let (program, catalog, mut world) = run_expr_world();
        for obj in world.objects.values_mut() {
            obj.set_flags(SCHEDULER_READY, false);
        }
        let mut lot = synthetic_coop_lot(program, catalog, world, Frames::parse("none").unwrap());
        let mut session = SessionState::new("Host", None, SessionConfig::default());
        session.players.get_mut(&PlayerId(1)).unwrap().selected_sim = Some(10);
        lot.attach_coop_context(
            CoopVmContext::new(session.clone()).with_frames(Frames::parse("none").unwrap()),
        )
        .expect("attach");

        // Build a valid InviteTicket string via `sims_net`? Check a synthetic
        // non-ticket hint with leading/trailing whitespace matches
        // `SessionState::apply_batch` bit-for-bit, AND verify a real `InviteTicket`
        // (encoded via a random keypair is unnecessary if we construct a valid
        // `simscoop1...` or test via `SessionState::new` parity).
        let raw_hint = "  mem:peer-2  ".to_string();
        let b1 = TickBatch {
            epoch: session.epoch,
            tick: Tick(1),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: vec![StampedCommand {
                tick: Tick(1),
                sequence: 0,
                issuer: PlayerId(1),
                command: Command::PlayerJoined {
                    player_id: PlayerId(2),
                    name: "Peer".to_string(),
                    selected_sim: Some(20),
                    camera_rotation: CameraRotation::NORTH,
                    endpoint_hint: Some(raw_hint.clone()),
                    transport_addr: None,
                },
            }],
        };
        let mut ref_session = session.clone();
        let _ = ref_session.apply_batch(&b1).unwrap();
        let a1 = session.apply_batch(&b1).unwrap();
        lot.apply_coop_batch(&a1, &mut session).unwrap();
        assert_eq!(
            session.players[&PlayerId(2)].transport_addr,
            ref_session.players[&PlayerId(2)].transport_addr,
            "VM join replay must match SessionState::apply_batch transport_addr bit-for-bit"
        );
    }

    #[test]
    fn raw_saved_restore_rejects_tutorial_parks_help_and_rejects_scop_v1() {
        let (mut program, catalog, mut world) = run_expr_world();
        // Add a Help System behavior using OPCODE_UI_EFFECT (34) on library 0.
        program.libraries.get_mut(&0).unwrap().behaviors.insert(
            4200,
            Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                revision: 0,
                instructions: vec![Instruction {
                    opcode: OPCODE_UI_EFFECT,
                    on_true: 0xfe,
                    on_false: 0xff,
                    operands: [0; 8],
                }],
            },
        );
        // Object 40 is a non-person object backed by library 0 with READY set.
        {
            let obj40 = world.objects.get_mut(&40).unwrap();
            obj40.registry_word = Some(0);
            obj40.set_flags(SCHEDULER_READY, true);
            assert_ne!(obj40.flags() & SCHEDULER_READY, 0);
        }
        let mut raw_lot = synthetic_coop_lot(
            program.clone(),
            catalog.clone(),
            world.clone(),
            Frames::parse("none").unwrap(),
        );
        let raw_help_bytes = raw_lot.save().unwrap().encode();

        // Also build a raw Saved tutorial lot (Global 26 == 1).
        raw_lot.world.globals[GLOBAL_TUTORIAL] = 1;
        let raw_tutorial_bytes = raw_lot.save().unwrap().encode();

        let mut target =
            synthetic_coop_lot(program, catalog, world, Frames::parse("none").unwrap());
        let session = two_player_session(Some(10), Some(20));

        // 1. Raw Saved tutorial lot must be rejected.
        assert!(
            target
                .restore_coop_snapshot(&raw_tutorial_bytes, session.clone())
                .is_err()
        );

        // 2. Raw Saved non-tutorial lot must succeed AND park Help System object 40!
        target
            .restore_coop_snapshot(&raw_help_bytes, session.clone())
            .expect("raw Saved restore");
        assert_eq!(
            target.world().objects[&40].flags() & SCHEDULER_READY,
            0,
            "raw Saved restore must deterministically park Help System controllers"
        );

        // 3. Old "SCOP" v1/v2 snapshot bytes must be rejected by v3 parser.
        for old_version in [1u32, 2u32] {
            let mut old_snap = target.coop_snapshot_bytes().unwrap();
            old_snap[4..8].copy_from_slice(&old_version.to_le_bytes());
            assert!(
                target
                    .restore_coop_snapshot(&old_snap, session.clone())
                    .is_err(),
                "SCOP v{old_version} must be rejected by v{COOP_SNAPSHOT_VERSION} parser"
            );
        }
    }

    #[test]
    fn coop_buy_build_enforces_catalog_prices_wall_styles_and_committed_placement() {
        let (program, catalog, mut world) = run_expr_world();
        for obj in world.objects.values_mut() {
            obj.set_flags(SCHEDULER_READY, false);
        }
        let mut lot = synthetic_coop_lot(program, catalog, world, Frames::parse("none").unwrap());
        let mut session = two_player_session(Some(10), Some(20));
        session.players.get_mut(&PlayerId(1)).unwrap().mode = PlayerMode::Build;
        lot.attach_coop_context(
            CoopVmContext::new(session.clone()).with_frames(Frames::parse("none").unwrap()),
        )
        .expect("attach");

        // 1. Unpriced built-in floor pattern (id 1) and wallpaper pattern (id 1)
        //    must be refused cleanly when BuildPrices lacks them, and PlaceWall
        //    styles 22 and 23 must be refused!
        let b1 = TickBatch {
            epoch: session.epoch,
            tick: Tick(1),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: vec![
                StampedCommand {
                    tick: Tick(1),
                    sequence: 0,
                    issuer: PlayerId(1),
                    command: Command::BuyBuild(BuyBuildEdit::SetFloorTile {
                        x: 5,
                        y: 5,
                        level: 1,
                        pattern: 1,
                    }),
                },
                StampedCommand {
                    tick: Tick(1),
                    sequence: 1,
                    issuer: PlayerId(1),
                    command: Command::BuyBuild(BuyBuildEdit::SetWallpaper {
                        x: 5,
                        y: 5,
                        level: 1,
                        side: 0,
                        pattern: 1,
                    }),
                },
                StampedCommand {
                    tick: Tick(1),
                    sequence: 2,
                    issuer: PlayerId(1),
                    command: Command::BuyBuild(BuyBuildEdit::PlaceWall {
                        x: 5,
                        y: 5,
                        level: 1,
                        style: 22,
                        pattern: 0,
                    }),
                },
                StampedCommand {
                    tick: Tick(1),
                    sequence: 3,
                    issuer: PlayerId(1),
                    command: Command::BuyBuild(BuyBuildEdit::PlaceWall {
                        x: 5,
                        y: 5,
                        level: 1,
                        style: 23,
                        pattern: 0,
                    }),
                },
            ],
        };
        let a1 = session.apply_batch(&b1).unwrap();
        let r1 = lot.apply_coop_batch(&a1, &mut session).unwrap();
        for (idx, (_, status)) in r1.command_results.iter().enumerate() {
            assert!(
                matches!(status, CoopCommandStatus::Refused(_)),
                "command {idx} should be refused, got {status:?}"
            );
        }

        // 2. Verify post-commit Buy placement validator rejects drifted facing/position/missing.
        let drifted_purchase = BuyPurchase {
            kind: crate::BuyActionKind::Buy,
            object: 40,
            guid: 0x2001,
            position: [160, 320, 1],
            facing: 4, // drifted from requested 2!
            container: None,
            price: 100,
            budget_category: 6,
            placed: true,
            before: None,
            after: None,
        };
        let mut world_check = lot.world().clone();
        assert!(
            verify_committed_buy_placement(
                &mut world_check,
                40,
                &drifted_purchase,
                [160, 320, 1],
                2,
                None,
                "Buy",
            )
            .is_err()
        );
    }

    #[test]
    fn coop_special_panel_freezes_resyncs_and_answers_in_lockstep() {
        let (mut program, catalog, mut world) = run_expr_world();
        // Object 40 executes Dialog kind 13 (MagicTownLotPicker) and records the
        // picked lot into MyData[0] when it returns True.
        program.libraries.get_mut(&0).unwrap().behaviors.insert(
            4100,
            Behavior {
                format: 0x8002,
                kind: 0,
                args: 1,
                locals: 0,
                revision: 0,
                instructions: vec![
                    // 0: Dialog kind 13 -> 1 on True, 2 on False
                    Instruction {
                        opcode: 36,
                        on_true: 1,
                        on_false: 2,
                        operands: [0, 0, 0, 0, 0, 13, 0, 0],
                    },
                    // 1: MyData[0] := Temp[0] -> 2
                    Instruction {
                        opcode: 2,
                        on_true: 2,
                        on_false: 2,
                        operands: [0, 0, 0, 0, 0, 5, 3, 8],
                    },
                    // 2: Sleep param[0] (yield)
                    Instruction {
                        opcode: 0,
                        on_true: 2,
                        on_false: 2,
                        operands: [0; 8],
                    },
                ],
            },
        );
        for obj in world.objects.values_mut() {
            obj.set_flags(SCHEDULER_READY, false);
        }
        {
            let obj40 = world.objects.get_mut(&40).unwrap();
            obj40.registry_word = Some(0);
            obj40.set_countdown(-1);
            obj40.set_flags(SCHEDULER_READY, true);
            obj40.stack.push(Frame {
                tree: 4100,
                node: 0,
                stack_object: 0,
                parameters: vec![1],
                locals: Vec::new(),
                primitive_state: 0,
                code_owner_type: 4,
            });
        }

        let make_peer = || {
            let mut lot = synthetic_coop_lot(
                program.clone(),
                catalog.clone(),
                world.clone(),
                Frames::parse("none").unwrap(),
            );
            let session = two_player_session(Some(10), Some(20));
            lot.attach_coop_context(
                CoopVmContext::new(session.clone()).with_frames(Frames::parse("none").unwrap()),
            )
            .expect("attach");
            (lot, session)
        };

        let (mut host, mut host_session) = make_peer();
        let (mut peer, mut peer_session) = make_peer();

        // Batch 1: triggers Dialog kind 13 special panel -> opens & freezes tick!
        let b1 = TickBatch {
            epoch: host_session.epoch,
            tick: Tick(1),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: Vec::new(),
        };
        let a1_h = host_session.apply_batch(&b1).unwrap();
        let r1_h = host.apply_coop_batch(&a1_h, &mut host_session).unwrap();
        let a1_p = peer_session.apply_batch(&b1).unwrap();
        let r1_p = peer.apply_coop_batch(&a1_p, &mut peer_session).unwrap();

        assert!(!r1_h.stepped);
        assert!(r1_h.opened_dialog.is_none());
        let (dialog_id, opened_shown) = r1_h
            .opened_special_panel
            .clone()
            .expect("tick 1 must open MagicTownLotPicker special panel");
        assert_eq!(
            opened_shown.request,
            SpecialPanelRequest::MagicTownLotPicker { via_blimp: false }
        );
        assert_eq!(r1_h.state_fingerprint, r1_p.state_fingerprint);
        assert!(host_session.open_dialogs.contains_key(&dialog_id));
        assert!(host.coop().unwrap().frozen_dialog.is_some());

        // Batch 2 (while special panel is open): gameplay commands are ignored and
        // SetFreeWill does NOT mutate pre-tick world.globals[30] before replay!
        let b2 = TickBatch {
            epoch: host_session.epoch,
            tick: Tick(2),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: vec![
                StampedCommand {
                    tick: Tick(2),
                    sequence: 0,
                    issuer: PlayerId(1),
                    command: Command::SetFreeWill { enabled: false },
                },
                StampedCommand {
                    tick: Tick(2),
                    sequence: 1,
                    issuer: PlayerId(2),
                    command: Command::SimActionPick {
                        sim_id: 20,
                        target_id: 40,
                        menu_index: 0,
                        joining: false,
                        args: [0; 4],
                    },
                },
            ],
        };
        let a2_h = host_session.apply_batch(&b2).unwrap();
        let r2_h = host.apply_coop_batch(&a2_h, &mut host_session).unwrap();
        let a2_p = peer_session.apply_batch(&b2).unwrap();
        let r2_p = peer.apply_coop_batch(&a2_p, &mut peer_session).unwrap();
        assert!(!r2_h.stepped);
        assert_eq!(r2_h.command_results[0].1, CoopCommandStatus::Applied);
        assert_eq!(
            r2_h.command_results[1].1,
            CoopCommandStatus::IgnoredWhileDialogPending
        );
        assert_eq!(
            host.world().globals[GLOBAL_FREE_WILL],
            1,
            "pre-tick Free Will must remain frozen while special panel waits"
        );
        assert_eq!(r2_h.state_fingerprint, r2_p.state_fingerprint);

        // Mid-wait snapshot + verified restore onto a joining peer!
        let snap = host
            .coop_snapshot_bytes()
            .expect("encode mid-panel snapshot");
        let host_fp = host.coop_state_fingerprint(&host_session);
        let mut joiner = synthetic_coop_lot(
            program.clone(),
            catalog.clone(),
            world.clone(),
            Frames::parse("none").unwrap(),
        );
        let mut joiner_session = host_session.clone();
        joiner
            .restore_coop_snapshot_verified(&snap, joiner_session.clone(), host_fp)
            .expect("joiner must verify mid-special-panel snapshot bit-for-bit");
        assert_eq!(
            joiner.asking_special_panel().map(|a| &a.shown),
            Some(&opened_shown)
        );
        assert_eq!(
            joiner.coop().unwrap().active_special_panel,
            Some((dialog_id, opened_shown))
        );

        // Batch 3: wrong-variant answer is Refused (leaving panel open), then valid
        // special_panel_answer_command wins and replays tick 1 across all three peers!
        let b3 = TickBatch {
            epoch: host_session.epoch,
            tick: Tick(3),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: vec![
                StampedCommand {
                    tick: Tick(3),
                    sequence: 0,
                    issuer: PlayerId(1),
                    command: special_panel_answer_command(
                        dialog_id,
                        &SpecialPanelResponse::Dismissed,
                    ),
                },
                StampedCommand {
                    tick: Tick(3),
                    sequence: 1,
                    issuer: PlayerId(2),
                    command: special_panel_answer_command(
                        dialog_id,
                        &SpecialPanelResponse::Lot(Some(93)),
                    ),
                },
            ],
        };
        let a3_h = host_session.apply_batch(&b3).unwrap();
        let r3_h = host.apply_coop_batch(&a3_h, &mut host_session).unwrap();
        let a3_p = peer_session.apply_batch(&b3).unwrap();
        let r3_p = peer.apply_coop_batch(&a3_p, &mut peer_session).unwrap();
        let a3_j = joiner_session.apply_batch(&b3).unwrap();
        let r3_j = joiner.apply_coop_batch(&a3_j, &mut joiner_session).unwrap();

        assert!(matches!(
            r3_h.command_results[0].1,
            CoopCommandStatus::Refused(_)
        ));
        assert_eq!(
            r3_h.command_results[1].1,
            CoopCommandStatus::DialogAnswered {
                dialog_id,
                winner: PlayerId(2),
            }
        );
        assert!(r3_h.stepped);
        assert_eq!(host.special_panels().magic_lot, 93);
        assert_eq!(
            host.world().globals[GLOBAL_FREE_WILL],
            0,
            "intervening SetFreeWill must apply after replay completes"
        );
        assert_eq!(r3_h.state_fingerprint, r3_p.state_fingerprint);
        assert_eq!(r3_h.state_fingerprint, r3_j.state_fingerprint);

        // Batch 4: object 40 reads magic_lot = 93 into MyData[0] and clears slot!
        let b4 = TickBatch {
            epoch: host_session.epoch,
            tick: Tick(4),
            pacing_hint_ms: 0,
            host_fingerprint: None,
            commands: Vec::new(),
        };
        let a4_h = host_session.apply_batch(&b4).unwrap();
        let r4_h = host.apply_coop_batch(&a4_h, &mut host_session).unwrap();
        let a4_j = joiner_session.apply_batch(&b4).unwrap();
        let r4_j = joiner.apply_coop_batch(&a4_j, &mut joiner_session).unwrap();
        assert_eq!(host.world().objects[&40].data[0], 93);
        assert_eq!(joiner.world().objects[&40].data[0], 93);
        assert_eq!(r4_h.state_fingerprint, r4_j.state_fingerprint);
    }

    #[test]
    fn lot_mixed_dialog_and_special_panel_double_wait_guards_and_headless_book() {
        let (mut program, catalog, mut world) = run_expr_world();
        program.labels_hold_no_token = true;
        {
            let lib = program.libraries.get_mut(&0).unwrap();
            lib.definitions.insert(1, [0; 108]);
            lib.definitions.insert(4, [0; 108]);
        }
        // Object 10 (person): inline Spellbook (kind 12) then ordinary Dialog (kind 0).
        program.libraries.get_mut(&0).unwrap().behaviors.insert(
            4101,
            Behavior {
                format: 0x8002,
                kind: 0,
                args: 1,
                locals: 0,
                revision: 0,
                instructions: vec![
                    Instruction {
                        opcode: 36,
                        on_true: 1,
                        on_false: 1,
                        operands: [0, 0, 0, 0, 0, 12, 0, 0],
                    },
                    Instruction {
                        opcode: 36,
                        on_true: 2,
                        on_false: 2,
                        operands: [0, 0, 0, 0, 0, 1, 0, 0],
                    },
                    Instruction {
                        opcode: 0,
                        on_true: 2,
                        on_false: 2,
                        operands: [0; 8],
                    },
                ],
            },
        );
        // Object 40: Dialog kind 13 (MagicTownLotPicker).
        program.libraries.get_mut(&0).unwrap().behaviors.insert(
            4102,
            Behavior {
                format: 0x8002,
                kind: 0,
                args: 1,
                locals: 0,
                revision: 0,
                instructions: vec![
                    Instruction {
                        opcode: 36,
                        on_true: 1,
                        on_false: 1,
                        operands: [0, 0, 0, 0, 0, 13, 0, 0],
                    },
                    Instruction {
                        opcode: 0,
                        on_true: 1,
                        on_false: 1,
                        operands: [0; 8],
                    },
                ],
            },
        );
        world
            .objects
            .insert(41, test_object(41, 0x2001, [160, 320, 1]));
        world.order.push(41);
        for obj in world.objects.values_mut() {
            obj.set_flags(SCHEDULER_READY, false);
        }
        {
            let obj40 = world.objects.get_mut(&40).unwrap();
            obj40.registry_word = Some(0);
            obj40.set_countdown(-1);
            obj40.set_flags(SCHEDULER_READY, true);
            obj40.stack.push(Frame {
                tree: 4101,
                node: 0,
                stack_object: 0,
                parameters: vec![1],
                locals: Vec::new(),
                primitive_state: 0,
                code_owner_type: 4,
            });
        }
        {
            let obj41 = world.objects.get_mut(&41).unwrap();
            obj41.registry_word = Some(0);
            obj41.set_countdown(-1);
            obj41.set_flags(SCHEDULER_READY, true);
            obj41.stack.push(Frame {
                tree: 4102,
                node: 0,
                stack_object: 0,
                parameters: vec![1],
                locals: Vec::new(),
                primitive_state: 0,
                code_owner_type: 4,
            });
        }

        // 1. Headless mode (`asks = false`): queued `Lot(Some(93))` MUST NOT be consumed
        //    by the preceding Spellbook (kind 12); the book auto-dismisses and kind 13
        //    receives `Lot(Some(93))` on the same tick!
        let mut headless = synthetic_coop_lot(
            program.clone(),
            catalog.clone(),
            world.clone(),
            Frames::parse("none").unwrap(),
        );
        headless.player = Answers::choices(&[2]);
        headless
            .queue_special_panel_responses(&[SpecialPanelResponse::Lot(Some(93))])
            .unwrap();
        let u_headless = headless
            .step()
            .expect("headless step must complete cleanly");
        assert_eq!(
            u_headless.special_panels_answered,
            vec![
                (
                    SpecialPanelRequest::SpellBook { caller: None },
                    SpecialPanelResponse::Dismissed
                ),
                (
                    SpecialPanelRequest::MagicTownLotPicker { via_blimp: false },
                    SpecialPanelResponse::Lot(Some(93))
                ),
            ]
        );
        assert_eq!(headless.special_panels().magic_lot, 93);

        // 2. Interactive mode (`wait_for_answers`): tick 1 waits THREE times
        //    (wait #1: inline Spellbook from object 40, wait #2: ordinary dialog
        //    from object 40, wait #3: special panel kind 13 from object 41) across
        //    rollback, v9 save/restore, and build/travel pending-input guards!
        let mut interactive = synthetic_coop_lot(
            program.clone(),
            catalog.clone(),
            world.clone(),
            Frames::parse("none").unwrap(),
        );
        interactive.wait_for_answers();

        // Wait #1: inline Spellbook (kind 12).
        assert_eq!(
            interactive.step().unwrap_err(),
            StepError::Asking { tick: 1 }
        );
        assert!(interactive.asking().is_none());
        assert_eq!(
            interactive
                .asking_special_panel()
                .expect("first wait must be inline Spellbook")
                .shown
                .request,
            SpecialPanelRequest::SpellBook { caller: None }
        );
        interactive
            .answer_special_panel(SpecialPanelResponse::Dismissed)
            .unwrap();

        // Wait #2: ordinary dialog (kind 0).
        assert_eq!(
            interactive.step().unwrap_err(),
            StepError::Asking { tick: 1 }
        );
        assert!(interactive.asking().is_some());
        assert!(interactive.asking_special_panel().is_none());

        // Answer ordinary dialog #2 -> replaying tick 1 now pauses at Wait #3 (special panel kind 13)!
        interactive.answer(2).unwrap();
        assert_eq!(
            interactive.step().unwrap_err(),
            StepError::Asking { tick: 1 }
        );
        assert!(interactive.asking().is_none());
        let sp = interactive
            .asking_special_panel()
            .expect("third wait in same tick must be MagicTownLotPicker");
        assert_eq!(
            sp.shown.request,
            SpecialPanelRequest::MagicTownLotPicker { via_blimp: false }
        );

        // Verify Build commit and Travel depart/return are blocked while special panel waits!
        assert!(matches!(
            interactive.build_commit_roof(&crate::build::RoofPlan {
                before_raw: None,
                before: crate::build::RoofState::default(),
                after: crate::build::RoofState::default(),
                valid: true,
            }),
            Err(OrderError::Asking)
        ));
        let db_path = std::env::temp_dir().join(format!(
            "sims-vm-special-panel-guard-{}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&db_path);
        struct TempDbGuard(std::path::PathBuf);
        impl Drop for TempDbGuard {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _db_guard = TempDbGuard(db_path.clone());
        let dummy_store = sims_sim::neighborhood_store::NeighborhoodStore::open(&db_path).unwrap();
        assert!(matches!(
            crate::travel::depart_to_destination(
                std::path::Path::new("."),
                &dummy_store,
                "UserData",
                &mut interactive,
                &crate::travel::TravelRequest::downtown(21, 0x1001),
            ),
            Err(crate::travel::TravelError::Lot(msg)) if msg.contains("special panel")
        ));
        let dummy_session = crate::travel::TravelSession {
            store_id: dummy_store.store_id(),
            neighborhood_id: "UserData".to_owned(),
            instance_id: 1,
            trip_id: 1,
            home_house: 5,
            home_family: 1,
            destination_house: 21,
            destination: crate::travel::TravelDestination::Downtown,
            transport: crate::travel::TravelTransport::DowntownCab,
            accommodation: crate::travel::VacationAccommodation::None,
            fare_paid: 50,
            departure_ticks: 0,
            departure_world_ticks: 0,
            departure_clock: (8, 0, 0),
            funds_before_fare: 1000,
            funds_at_arrival: 950,
            magic_coins_at_arrival: 0,
            leader_guid: 0x1001,
            travelers: Vec::new(),
        };
        assert!(matches!(
            crate::travel::return_from_destination(
                std::path::Path::new("."),
                &dummy_store,
                &dummy_session,
                &mut interactive,
                &[],
                Frames::parse("none").unwrap(),
            ),
            Err(crate::travel::TravelError::Lot(msg)) if msg.contains("special panel")
        ));

        // Save/restore v9 mid-second-wait and verify replay reproduces wait #2 & converges!
        let saved_bytes = interactive
            .save()
            .expect("v9 save mid-second-wait")
            .encode();
        let decoded = Saved::decode(&saved_bytes).expect("decode v9 save");
        let mut restored =
            synthetic_coop_lot(program, catalog, world, Frames::parse("none").unwrap());
        restored.wait_for_answers();
        restored.restore(decoded).unwrap();
        assert_eq!(restored.step().unwrap_err(), StepError::Asking { tick: 1 });
        assert_eq!(
            restored.asking_special_panel().unwrap().shown.request,
            SpecialPanelRequest::MagicTownLotPicker { via_blimp: false }
        );

        interactive
            .answer_special_panel(SpecialPanelResponse::Lot(Some(93)))
            .unwrap();
        restored
            .answer_special_panel(SpecialPanelResponse::Lot(Some(93)))
            .unwrap();
        let u_live = interactive.step().unwrap();
        let u_restored = restored.step().unwrap();
        assert_eq!(
            u_live.special_panels_answered,
            u_restored.special_panels_answered
        );
        assert_eq!(interactive.world().hash(), restored.world().hash());
        assert_eq!(interactive.world().hash(), headless.world().hash());
    }

    #[test]
    fn a_pending_indicator_refresh_counts_in_the_fingerprint_and_lot_compare() {
        // A fame write that leaves the word as it was still queues a star
        // refresh (0x444962), which the next draw acts on.
        let (program, catalog, world) = run_expr_world();
        let lot_a = synthetic_coop_lot(
            program.clone(),
            catalog.clone(),
            world.clone(),
            Frames::default(),
        );
        let mut lot_b = synthetic_coop_lot(program, catalog, world, Frames::default());
        lot_b.world.indicator_queues.queue_star(1);
        assert_eq!(lot_a.world().hash(), lot_b.world().hash());
        assert_ne!(lot_a.coop_world_hash(), lot_b.coop_world_hash());
        assert!(!crate::both_ways::same_lot(lot_a.world(), lot_b.world()));
    }

    #[test]
    fn coop_fingerprint_covers_family_flags_and_refuses_local_cheats() {
        let (program, catalog, world) = run_expr_world();
        let mut lot_a = synthetic_coop_lot(
            program.clone(),
            catalog.clone(),
            world.clone(),
            Frames::default(),
        );
        let mut lot_b = synthetic_coop_lot(program, catalog, world, Frames::default());
        assert_eq!(lot_a.coop_world_hash(), lot_b.coop_world_hash());

        // Different family_flags must produce distinct coop_world_hash while
        // preserving identical World::hash().
        lot_b.world.family_flags.insert(5, 0x1);
        assert_eq!(lot_a.world().hash(), lot_b.world().hash());
        assert_ne!(lot_a.coop_world_hash(), lot_b.coop_world_hash());

        // Attach a co-op context and verify local cheats are refused.
        let session = SessionState::new("Host", None, SessionConfig::default());
        lot_a.attach_coop(session).unwrap();
        assert!(matches!(lot_a.add_funds(1000), Err(OrderError::Refused(_))));
        assert!(matches!(lot_a.set_hour(12), Err(OrderError::Refused(_))));
    }
}
