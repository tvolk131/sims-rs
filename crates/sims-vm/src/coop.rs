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
//!     the activity owner or host.
//! - **Per-player `camera_rotation`:**
//!   - `Look Towards`, `Generic Sims Call 12` (tile depth), `Find Location`
//!     mode 5, head model placement, and portrait tile placement resolve the
//!     camera rotation of the player who has the asking Sim selected (or owns
//!     its activity), falling back to the host's rotation.
//! - **Shared Mode Global 19, multi-reason pause, and fixed session globals:**
//!   - Maps [`PlayerMode`] (`Build` > `Buy` > `Live`) onto the simulator's
//!     [`PanelMode`] / Global 19 (`0x4ce44f`: `0` Live, `1` Build, `2` Buy).
//!   - Syncs Global 30 (`Free Will`), fixed Global 33 (`Machine Level`), host
//!     Global 15 (`Current Language`), Global 16/18 (`Speed`), and Global 17
//!     (`Paused`) from [`SessionState`].
//!   - Respects multi-reason pause ([`SessionState::is_paused`]) and
//!     first-answer-wins blocking modal dialogs.
//! - **Throwaway per-player pie-menu checks:**
//!   - [`Lot::coop_pie_menu`] takes `&self` and runs check scripts against a
//!     throwaway CoW clone of [`World`] with that player's selected Sim and
//!     camera rotation, never mutating shared RNG or simulation state.
//! - **Deterministic fingerprints & REDB snapshot resync:**
//!   - [`Lot::coop_world_hash`] and [`Lot::coop_state_fingerprint`] produce
//!     canonical SHA-256 digests matching [`sims_net::StateFingerprint`].
//!   - [`Lot::coop_snapshot_bytes`] and [`Lot::restore_coop_snapshot`] bundle
//!     [`crate::boot::save::Saved`] bytes with co-op VM metadata so mid-game
//!     join and drift recovery resume bit-for-bit identically.

use crate::boot::save::{SaveError, Saved};
use crate::boot::{AnswerError, Lot, Menu, OrderError, StepError};
use crate::build::{
    BuildPrices, FloorDragMode, WALL_EAST, WALL_NORTH, WALL_SOUTH, WALL_WEST, WallDragMode,
    WallFace, WallStyle, WallpaperDragMode, plan_floor_edit, plan_wall_drag, plan_wallpaper_edit,
};
use crate::interpreter::{Interpreter, Stream};
use crate::orders::MenuEntry;
use crate::{Answer, PanelMode, Shown, Update};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_net::{
    AppliedBatch, BuyBuildEdit, CameraRotation, Command, CommandIgnoreReason, CommandOutcome,
    CommandRejectReason, DialogAnswer, DialogId, PlayerId, PlayerMode, SessionError, SessionState,
    SimSpeed, StampedCommand, StateFingerprint, Tick, TickBatch,
};
use sims_sim::World;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Global 3: selected person ID (`0x445449`).
const GLOBAL_SELECTED: usize = 3;
/// Global 15: host simulation language.
const GLOBAL_LANGUAGE: usize = 15;
/// Global 17: simulation paused flag (`0x46d813`).
const GLOBAL_PAUSED: usize = 17;
/// Global 30: Free Will (`0x432220`).
const GLOBAL_FREE_WILL: usize = 30;
/// Global 33: Machine Level (fixed per session for determinism).
const GLOBAL_MACHINE_LEVEL: usize = 33;

/// Domain separator for [`Lot::coop_world_hash`].
const COOP_WORLD_HASH_DOMAIN: &[u8] = b"sims-vm/coop-world/v1\0";
/// Magic prefix distinguishing [`CoopSnapshotEnvelope`] from raw [`Saved`] bytes.
const COOP_SNAPSHOT_MAGIC: &[u8; 4] = b"SCOP";
/// Wire version of [`CoopSnapshotEnvelope`].
const COOP_SNAPSHOT_VERSION: u32 = 1;

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
    /// Snapshot of [`SessionState`] at the moment `active_dialog` opened, used
    /// by [`Lot::restore_coop_snapshot`] to re-probe `Lot::asking` deterministically
    /// even if players changed selections or camera rotations while the dialog
    /// remained open.
    pub(crate) dialog_open_session: Option<SessionState>,
    /// Deferred `Command::SetSpeed` received while a blocking dialog froze
    /// `self.world`, applied as soon as the dialog replay completes.
    pub(crate) pending_speed_sync: bool,
    /// Whether [`Lot::apply_coop_batch`] steps in play mode (`Lot::step_play`)
    /// instead of strict mode (`Lot::step`). Defaults to `false`.
    pub play_mode: bool,
    /// Temporary per-player override active only during throwaway local checks
    /// (such as [`Lot::coop_pie_menu`]) so check trees see that exact player's
    /// selected Sim and camera rotation.
    pub(crate) active_player_override: Option<PlayerId>,
}

impl CoopVmContext {
    /// Creates a new co-op VM context wrapping `session`.
    pub fn new(session: SessionState) -> Self {
        Self {
            session,
            activity_owners: BTreeMap::new(),
            next_dialog_id: 1,
            active_dialog: None,
            dialog_open_session: None,
            pending_speed_sync: false,
            play_mode: false,
            active_player_override: None,
        }
    }

    /// Configures whether [`Lot::apply_coop_batch`] runs in play mode.
    pub fn with_play_mode(mut self, play_mode: bool) -> Self {
        self.play_mode = play_mode;
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
        let new_sim = u16::try_from(value).ok().filter(|&id| id != 0);
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
    /// Uses the rotation of the player who has `me` selected (lowest
    /// [`PlayerId`] tie-breaker), or for non-Sim scripts the player who
    /// selects `stack_object` or owns the activity on `me`/`stack_object`,
    /// falling back to the host's rotation.
    pub fn resolve_camera_rotation(&self, world: &World, me: u16, stack_object: Option<i16>) -> u8 {
        if let Some(player_id) = self.active_player_override {
            if let Some(player) = self.session.players.get(&player_id) {
                return player.camera_rotation.quarter_turns();
            }
            return self.host_camera_rotation();
        }

        if world.objects.get(&me).is_some_and(|o| o.person.is_some())
            && self.session.is_sim_selected_by_any(me)
        {
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
    /// `SimActionPick` queued an interaction (or queue was full -> `None`).
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
    /// Per-command deterministic outcomes in exact batch sequence order.
    pub command_results: Vec<(StampedCommand, CoopCommandStatus)>,
    /// Any player selections altered by scripts or end-of-update reselection
    /// during this tick.
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
    /// Lockstep [`SessionState::apply_batch`] rejected the batch.
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
    dialog_open_session: Option<SessionState>,
    pending_speed_sync: bool,
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
/// metadata hashed into [`Lot::coop_world_hash`].
#[derive(Serialize)]
struct CoopWorldFingerprintTail<'a> {
    world_hash: [u8; 32],
    ticks: u32,
    view_rotation: u8,
    answered: bool,
    asking: Option<(u32, u16)>,
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
    scratch_ptr_set: &'a BTreeSet<u16>,
    scratch_ptr_set_to_next: &'a BTreeSet<u16>,
    scratch_ptr_expression: &'a BTreeSet<u16>,
    scratch_ptr_arg_slot: &'a BTreeSet<u16>,
    runtime_added_suits: &'a Vec<String>,
    suit_scan_cache: &'a Option<Vec<String>>,
    activity_owners: Option<&'a BTreeMap<u16, PlayerId>>,
    next_dialog_id: Option<u64>,
    active_dialog: Option<DialogId>,
    pending_speed_sync: bool,
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

fn build_prices_default() -> BuildPrices {
    BuildPrices::default()
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

    /// Attaches a co-op [`SessionState`] to this lot, enabling interactive
    /// blocking dialog waits (`Lot::wait_for_answers`) and synchronizing fixed
    /// session globals, host camera rotation, host selection, mode, pause, and
    /// speed.
    pub fn attach_coop(&mut self, session: SessionState) -> Result<(), OrderError> {
        self.attach_coop_context(CoopVmContext::new(session))
    }

    /// Attaches a preconfigured [`CoopVmContext`] to this lot.
    pub fn attach_coop_context(&mut self, mut ctx: CoopVmContext) -> Result<(), OrderError> {
        self.ready()?;
        self.wait_for_answers();
        self.sanitize_session_selections(&mut ctx.session);
        self.sync_world_from_session(&ctx.session, true)?;
        self.coop = Some(ctx);
        Ok(())
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
    /// `LockstepNode::step_next_tick`) against this lot and keeps `session`
    /// synchronized with any script-driven selection changes or newly opened
    /// blocking modal dialogs.
    pub fn apply_coop_batch(
        &mut self,
        batch: &AppliedBatch,
        session: &mut SessionState,
    ) -> Result<CoopBatchReport, CoopError> {
        let Some(mut ctx) = self.coop.take() else {
            return Err(CoopError::NotAttached);
        };
        ctx.session = session.clone();
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
        ctx.session = session.clone();
        let result = self.execute_coop_batch(&applied, &mut ctx, &mut session);
        ctx.session = session;
        self.coop = Some(ctx);
        result
    }

    fn execute_coop_batch(
        &mut self,
        batch: &AppliedBatch,
        ctx: &mut CoopVmContext,
        session: &mut SessionState,
    ) -> Result<CoopBatchReport, CoopError> {
        let mut command_results = Vec::with_capacity(batch.commands.len());
        let mut speed_changed = false;

        for item in &batch.commands {
            let stamped = &item.stamped;
            let status = match &item.outcome {
                CommandOutcome::Rejected(reason) => CoopCommandStatus::RejectedBySession(*reason),
                CommandOutcome::Ignored(reason) => CoopCommandStatus::IgnoredBySession(*reason),
                CommandOutcome::DialogWon {
                    dialog_id,
                    winner,
                    answer,
                } => self.apply_coop_dialog_answer(
                    batch.tick, *dialog_id, *winner, answer, ctx, session,
                ),
                CommandOutcome::Applied => {
                    ctx.session = session.clone();
                    let status = self.apply_coop_command(stamped, ctx, session, &mut speed_changed);
                    // Preserve any script-driven selection mutations (e.g. an
                    // `at_once` action in `execute_coop_pick`).
                    ctx.session = session.clone();
                    status
                }
            };
            command_results.push((stamped.clone(), status));
        }

        if speed_changed {
            ctx.pending_speed_sync = true;
        }

        // Validate player selections against live persons and keep session
        // synced before stepping.
        self.sanitize_session_selections(session);
        ctx.session = session.clone();

        // Sync globals, mode, pause, host camera, and host plumbob when not
        // frozen on a dialog snapshot.
        if self.asking.is_none() && !self.answered {
            let apply_speed = ctx.pending_speed_sync;
            if let Err(err) = self.sync_world_from_session(session, apply_speed) {
                return Err(CoopError::Order(err));
            }
            ctx.pending_speed_sync = false;
            ctx.session = session.clone();
        } else {
            // Free Will (Global 30) and fixed session globals apply safely even
            // while waiting on a dialog; leave pre-replay world globals intact
            // for mode/pause/selection/speed until replay completes.
            let _ = self.set_free_will(session.config.free_will);
        }

        // Snapshot pre-step selections so we can report script-driven changes.
        let before_selections: BTreeMap<PlayerId, Option<u16>> = session
            .players
            .iter()
            .map(|(&pid, p)| (pid, p.selected_sim))
            .collect();

        let mut stepped = false;
        let mut update = None;
        let mut opened_dialog = None;

        if !session.is_paused() && self.asking.is_none() {
            let play_mode = ctx.play_mode;
            let pre_step_session = session.clone();
            // Temporarily install `ctx` on `self.coop` so `step_in` threads it
            // into `Interpreter` and rolls it back atomically on `Stop::Asks`.
            self.coop = Some(ctx.clone());
            let step_result = self.step_in(play_mode);
            if let Some(updated_ctx) = self.coop.take() {
                *ctx = updated_ctx;
            }
            match step_result {
                Ok(upd) => {
                    stepped = true;
                    ctx.dialog_open_session = None;
                    // Prune activity owners for any removed objects.
                    for &removed_id in &upd.removed {
                        ctx.remove_object(removed_id);
                    }
                    // Sync script-driven changes (Global 3 writes, deletions,
                    // end-of-update reselection) back into authoritative session.
                    *session = ctx.session.clone();
                    self.sanitize_session_selections(session);
                    ctx.session = session.clone();
                    // Sync host plumbob if a script rewrote host's selection.
                    if self.asking.is_none() && !self.answered {
                        let _ = self.sync_host_plumbob(session);
                    }
                    update = Some(upd);
                }
                Err(StepError::Asking { .. }) => {
                    if let Some(asking) = self.asking.as_ref() {
                        let dialog_id = DialogId(ctx.next_dialog_id);
                        ctx.next_dialog_id = ctx.next_dialog_id.saturating_add(1);
                        ctx.active_dialog = Some(dialog_id);
                        ctx.dialog_open_session = Some(pre_step_session);
                        let shown = asking.shown.clone();
                        session.open_blocking_dialog(dialog_id, shown.object, batch.tick);
                        ctx.session = session.clone();
                        opened_dialog = Some((dialog_id, shown));
                    }
                }
                Err(err) => {
                    *session = ctx.session.clone();
                    return Err(CoopError::Step(err));
                }
            }
        }

        // Sync any post-step mode/pause/speed state after a dialog replay finishes.
        if self.asking.is_none() && !self.answered {
            let apply_speed = ctx.pending_speed_sync;
            let _ = self.sync_world_from_session(session, apply_speed);
            ctx.pending_speed_sync = false;
        }
        ctx.session = session.clone();

        let script_selection_changes: Vec<(PlayerId, Option<u16>)> = session
            .players
            .iter()
            .filter_map(|(&pid, p)| {
                let prev = before_selections.get(&pid).copied().flatten();
                (prev != p.selected_sim).then_some((pid, p.selected_sim))
            })
            .collect();

        // Install `ctx` briefly so `coop_world_hash` includes `ctx` metadata.
        self.coop = Some(ctx.clone());
        let world_hash = self.coop_world_hash();
        let _ = self.coop.take();
        let state_fingerprint = session.combined_fingerprint(world_hash);

        Ok(CoopBatchReport {
            tick: batch.tick,
            stepped,
            update,
            opened_dialog,
            command_results,
            script_selection_changes,
            world_hash,
            state_fingerprint,
        })
    }

    fn apply_coop_dialog_answer(
        &mut self,
        tick: Tick,
        dialog_id: DialogId,
        winner: PlayerId,
        answer: &DialogAnswer,
        ctx: &mut CoopVmContext,
        session: &mut SessionState,
    ) -> CoopCommandStatus {
        if ctx.active_dialog != Some(dialog_id) {
            return CoopCommandStatus::Refused(format!(
                "no active blocking dialog matching {dialog_id}"
            ));
        }
        let Some(asking_obj) = self.asking.as_ref().map(|a| a.shown.object) else {
            ctx.active_dialog = None;
            return CoopCommandStatus::Refused("lot is not waiting on a dialog".to_string());
        };
        let vm_answer = Answer {
            status: answer.status,
            bitmap_id: answer.bitmap_id,
            text: answer.text.clone(),
        };
        match self.answer(vm_answer) {
            Ok(()) => {
                ctx.active_dialog = None;
                CoopCommandStatus::DialogAnswered { dialog_id, winner }
            }
            Err(AnswerError::NotOffered { answer, offered }) => {
                // `SessionState::apply_batch` removed `dialog_id` before button
                // validation. Deterministically reopen it on every peer so an
                // invalid button status can never bypass or desync the dialog.
                session.open_blocking_dialog(dialog_id, asking_obj, tick);
                ctx.session = session.clone();
                CoopCommandStatus::Refused(format!(
                    "dialog {dialog_id} offers {offered:?}, not {answer}"
                ))
            }
            Err(err) => CoopCommandStatus::Refused(err.to_string()),
        }
    }

    fn apply_coop_command(
        &mut self,
        stamped: &StampedCommand,
        ctx: &mut CoopVmContext,
        session: &mut SessionState,
        speed_changed: &mut bool,
    ) -> CoopCommandStatus {
        let issuer = stamped.issuer;
        match &stamped.command {
            Command::SelectPerson { sim_id } => {
                if let Some(id) = *sim_id
                    && !self
                        .world
                        .objects
                        .get(&id)
                        .is_some_and(|o| o.person.is_some())
                {
                    if let Some(player) = session.players.get_mut(&issuer) {
                        player.selected_sim = None;
                    }
                    return CoopCommandStatus::Refused(format!(
                        "object {id} is not a person on the lot"
                    ));
                }
                CoopCommandStatus::Applied
            }
            Command::SetCameraRotation { rotation } => {
                if issuer == session.host_id && self.asking.is_none() && !self.answered {
                    self.set_view_rotation(rotation.quarter_turns());
                }
                CoopCommandStatus::Applied
            }
            Command::SetMode { .. }
            | Command::SetPaused { .. }
            | Command::SetHostSpeedLock { .. }
            | Command::PlayerJoined { .. } => CoopCommandStatus::Applied,
            Command::PlayerLeft { player_id, .. } => {
                ctx.activity_owners.retain(|_, owner| owner != player_id);
                CoopCommandStatus::Applied
            }
            Command::HostChanged { .. } => {
                if self.asking.is_none() && !self.answered {
                    let rot = session
                        .players
                        .get(&session.host_id)
                        .map(|p| p.camera_rotation.quarter_turns())
                        .unwrap_or(0);
                    self.set_view_rotation(rot);
                }
                CoopCommandStatus::Applied
            }
            Command::SetSpeed { .. } => {
                *speed_changed = true;
                CoopCommandStatus::Applied
            }
            Command::SetFreeWill { enabled } => match self.set_free_will(*enabled) {
                Ok(()) => CoopCommandStatus::Applied,
                Err(err) => CoopCommandStatus::Refused(err.to_string()),
            },
            Command::AnswerBlockingDialog { .. } => CoopCommandStatus::Applied,
            Command::SimActionPick {
                sim_id,
                target_id,
                menu_index,
                joining,
                args,
            } => {
                if self.asking.is_some() || self.answered {
                    return CoopCommandStatus::IgnoredWhileDialogPending;
                }
                let Ok(menu) = u32::try_from(*menu_index) else {
                    return CoopCommandStatus::Refused(format!("negative menu index {menu_index}"));
                };
                self.execute_coop_pick(
                    issuer, *sim_id, *target_id, menu, *joining, *args, ctx, session,
                )
            }
            Command::SimActionCancel {
                sim_id,
                interaction_uid,
            } => {
                if self.asking.is_some() || self.answered {
                    return CoopCommandStatus::IgnoredWhileDialogPending;
                }
                match self.cancel_with_coop(*sim_id, i32::from(*interaction_uid), ctx) {
                    Ok(counted) => {
                        *session = ctx.session.clone();
                        CoopCommandStatus::CancelledInteraction { counted }
                    }
                    Err(err) => CoopCommandStatus::Refused(err.to_string()),
                }
            }
            Command::BuyBuild(edit) => {
                if self.asking.is_some() || self.answered {
                    return CoopCommandStatus::IgnoredWhileDialogPending;
                }
                let status = self.execute_coop_buy_build(edit);
                if matches!(status, CoopCommandStatus::BuyBuildApplied) {
                    self.purge_coop_buy_build_history();
                }
                status
            }
        }
    }

    /// Purges detached Buy/Build undo/redo objects after a committed co-op
    /// `BuyBuildEdit` so live host state matches `Lot::save()` bytes
    /// bit-for-bit during mid-Buy/Build snapshot resync.
    fn purge_coop_buy_build_history(&mut self) {
        let has_history = !self.buy_undo.is_empty()
            || !self.buy_redo.is_empty()
            || !self.buy_orphans.is_empty()
            || !self.build_undo.is_empty()
            || !self.build_redo.is_empty();
        if !has_history {
            return;
        }
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let empty = BTreeSet::new();
        let mut build_detached = Vec::new();
        for entry in self.build_undo.iter().chain(self.build_redo.iter()) {
            build_detached.extend_from_slice(entry.detached_objects());
        }
        let mut world = self.world.clone();
        let ok = interpreter
            .buy_purge_detached(&mut world, &self.buy_redo, &empty)
            .and_then(|()| interpreter.buy_purge_detached(&mut world, &self.buy_undo, &empty))
            .and_then(|()| interpreter.buy_purge_detached(&mut world, &self.buy_orphans, &empty))
            .and_then(|()| interpreter.buy_purge_detached(&mut world, &build_detached, &empty))
            .is_ok();
        if ok {
            self.world = world;
            self.buy_undo.clear();
            self.buy_redo.clear();
            self.buy_orphans.clear();
            self.build_undo.clear();
            self.build_redo.clear();
            self.undo_order.clear();
            self.redo_order.clear();
        }
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
        session: &mut SessionState,
    ) -> CoopCommandStatus {
        if let Err(err) = self.ready() {
            return CoopCommandStatus::Refused(err.to_string());
        }
        let character_names = RefCell::new(self.character_names.clone());
        let coop_cell = RefCell::new(ctx.clone());
        let interpreter = Interpreter {
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            character_names: Some(&character_names),
            coop: Some(&coop_cell),
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let (mut action, _) = match interpreter.menu_action(&mut world, sim_id, target_id, menu) {
            Ok(built) => built,
            Err(fault) => return CoopCommandStatus::Refused(fault.to_string()),
        };
        action.args = args;
        if joining {
            action.join = 1;
        }
        let target_lead = u16::try_from(action.target).ok().unwrap_or(target_id);
        match interpreter.pick(&mut world, sim_id, action) {
            Ok(uid) => {
                self.world = world;
                *ctx = coop_cell.into_inner();
                *session = ctx.session.clone();
                ctx.record_activity_owner(sim_id, issuer);
                ctx.record_activity_owner(target_id, issuer);
                ctx.record_activity_owner(target_lead, issuer);
                CoopCommandStatus::QueuedInteraction { uid }
            }
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
        let interpreter = Interpreter {
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            character_names: Some(&character_names),
            coop: Some(&coop_cell),
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        let counts = interpreter
            .cancel(&mut world, person, uid)
            .map_err(OrderError::Refused)?;
        self.world = world;
        *ctx = coop_cell.into_inner();
        Ok(counts)
    }

    fn execute_coop_buy_build(&mut self, edit: &BuyBuildEdit) -> CoopCommandStatus {
        match edit {
            BuyBuildEdit::BuyObject {
                guid,
                position,
                facing,
                container: _,
            } => {
                let mut cur = match self.buy_start(*guid, position[2], *facing) {
                    Ok(Some(cur)) => cur,
                    Ok(None) => {
                        return CoopCommandStatus::Refused(
                            "cannot start Buy cursor (insufficient funds or busy)".to_string(),
                        );
                    }
                    Err(err) => return CoopCommandStatus::Refused(err.to_string()),
                };
                let valid = match self.buy_move(&mut cur, [position[0], position[1]], position[2]) {
                    Ok(v) => v,
                    Err(err) => {
                        let _ = self.buy_cancel(&cur);
                        return CoopCommandStatus::Refused(err.to_string());
                    }
                };
                if !valid {
                    let _ = self.buy_cancel(&cur);
                    return CoopCommandStatus::Refused(
                        "Buy placement does not fit at target position".to_string(),
                    );
                }
                match self.buy_commit(&cur) {
                    Ok(_) => CoopCommandStatus::BuyBuildApplied,
                    Err(err) => {
                        let _ = self.buy_cancel(&cur);
                        CoopCommandStatus::Refused(err.to_string())
                    }
                }
            }
            BuyBuildEdit::MoveObject {
                object_id,
                position,
                facing,
                container: _,
            } => {
                let mut cur = match self.buy_pick_up(*object_id) {
                    Ok(Some(cur)) => cur,
                    Ok(None) => {
                        return CoopCommandStatus::Refused(format!(
                            "object {object_id} cannot be picked up"
                        ));
                    }
                    Err(err) => return CoopCommandStatus::Refused(err.to_string()),
                };
                if let Err(err) = self.buy_rotate(&mut cur, *facing) {
                    let _ = self.buy_snap_back(&cur);
                    return CoopCommandStatus::Refused(err.to_string());
                }
                let valid = match self.buy_move(&mut cur, [position[0], position[1]], position[2]) {
                    Ok(v) => v,
                    Err(err) => {
                        let _ = self.buy_snap_back(&cur);
                        return CoopCommandStatus::Refused(err.to_string());
                    }
                };
                if !valid {
                    let _ = self.buy_snap_back(&cur);
                    return CoopCommandStatus::Refused(
                        "moved object does not fit at target position".to_string(),
                    );
                }
                match self.buy_commit(&cur) {
                    Ok(_) => CoopCommandStatus::BuyBuildApplied,
                    Err(err) => {
                        let _ = self.buy_snap_back(&cur);
                        CoopCommandStatus::Refused(err.to_string())
                    }
                }
            }
            BuyBuildEdit::SellObject { object_id } => {
                let cur = match self.buy_pick_up(*object_id) {
                    Ok(Some(cur)) => cur,
                    Ok(None) => {
                        return CoopCommandStatus::Refused(format!(
                            "object {object_id} cannot be picked up to sell"
                        ));
                    }
                    Err(err) => return CoopCommandStatus::Refused(err.to_string()),
                };
                match self.buy_sell(&cur) {
                    Ok(true) => CoopCommandStatus::BuyBuildApplied,
                    Ok(false) => {
                        let _ = self.buy_snap_back(&cur);
                        CoopCommandStatus::Refused(format!("object {object_id} cannot be sold"))
                    }
                    Err(err) => {
                        let _ = self.buy_snap_back(&cur);
                        CoopCommandStatus::Refused(err.to_string())
                    }
                }
            }
            BuyBuildEdit::PlaceWall {
                x,
                y,
                level,
                style,
                pattern: _,
            } => {
                let prices = build_prices_default();
                let wall_style = WallStyle::from_style_id(*style).unwrap_or(WallStyle::Standard);
                let plan = plan_wall_drag(
                    &self.world,
                    [i32::from(*x), i32::from(*y)],
                    [i32::from(*x) + 1, i32::from(*y)],
                    *level,
                    wall_style,
                    WallDragMode::Build,
                    &prices,
                );
                if !plan.valid || !plan.affordable {
                    return CoopCommandStatus::Refused(
                        "wall placement invalid or unaffordable".into(),
                    );
                }
                match self.build_commit_wall(&plan) {
                    Ok(true) => CoopCommandStatus::BuyBuildApplied,
                    Ok(false) => {
                        CoopCommandStatus::Refused("wall placement produced no change".into())
                    }
                    Err(err) => CoopCommandStatus::Refused(err.to_string()),
                }
            }
            BuyBuildEdit::RemoveWall { x, y, level } => {
                let prices = build_prices_default();
                let plan = plan_wall_drag(
                    &self.world,
                    [i32::from(*x), i32::from(*y)],
                    [i32::from(*x) + 1, i32::from(*y)],
                    *level,
                    WallStyle::Standard,
                    WallDragMode::Demolish,
                    &prices,
                );
                if !plan.valid {
                    return CoopCommandStatus::Refused("wall removal invalid".into());
                }
                match self.build_commit_wall(&plan) {
                    Ok(true) => CoopCommandStatus::BuyBuildApplied,
                    Ok(false) => {
                        CoopCommandStatus::Refused("wall removal produced no change".into())
                    }
                    Err(err) => CoopCommandStatus::Refused(err.to_string()),
                }
            }
            BuyBuildEdit::SetFloorTile {
                x,
                y,
                level,
                pattern,
            } => {
                let prices = build_prices_default();
                let mode = if *pattern == 0 {
                    FloorDragMode::Remove
                } else {
                    FloorDragMode::Lay
                };
                let tile = [i32::from(*x), i32::from(*y)];
                let plan = plan_floor_edit(
                    &self.world,
                    tile,
                    tile,
                    *level,
                    *pattern,
                    mode,
                    &prices,
                    None,
                );
                if !plan.valid || !plan.affordable {
                    return CoopCommandStatus::Refused("floor edit invalid or unaffordable".into());
                }
                match self.build_commit_floor(&plan) {
                    Ok(true) => CoopCommandStatus::BuyBuildApplied,
                    Ok(false) => CoopCommandStatus::Refused("floor edit produced no change".into()),
                    Err(err) => CoopCommandStatus::Refused(err.to_string()),
                }
            }
            BuyBuildEdit::SetWallpaper {
                x,
                y,
                level,
                side,
                pattern,
            } => {
                let (Ok(tx), Ok(ty)) = (u8::try_from(*x), u8::try_from(*y)) else {
                    return CoopCommandStatus::Refused("wallpaper tile out of range".into());
                };
                let bit = match *side & 3 {
                    0 => WALL_NORTH,
                    1 => WALL_EAST,
                    2 => WALL_SOUTH,
                    _ => WALL_WEST,
                };
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
                let prices = build_prices_default();
                let plan =
                    plan_wallpaper_edit(&self.world, &[face], *level, *pattern, mode, &prices);
                if !plan.valid || !plan.affordable {
                    return CoopCommandStatus::Refused(
                        "wallpaper edit invalid or unaffordable".into(),
                    );
                }
                match self.build_commit_wallpaper(&plan) {
                    Ok(true) => CoopCommandStatus::BuyBuildApplied,
                    Ok(false) => {
                        CoopCommandStatus::Refused("wallpaper edit produced no change".into())
                    }
                    Err(err) => CoopCommandStatus::Refused(err.to_string()),
                }
            }
            BuyBuildEdit::SetTerrainHeight { .. } => {
                CoopCommandStatus::Refused("terrain height editing is not yet ported".to_string())
            }
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
        session: &SessionState,
        apply_speed: bool,
    ) -> Result<(), OrderError> {
        sync_session_globals(&mut self.world, session);
        if apply_speed {
            self.world.set_speed(i32::from(session.time.speed.value()));
        }
        let host_rot = session
            .players
            .get(&session.host_id)
            .map(|p| p.camera_rotation.quarter_turns())
            .unwrap_or(self.view_rotation);
        self.set_view_rotation(host_rot);

        let target_mode = player_mode_to_panel_mode(session.effective_mode());
        if self.mode() != target_mode {
            self.set_mode(target_mode)?;
        }
        sync_pause_global(&mut self.world, session.is_paused());
        self.sync_host_plumbob(session)?;
        Ok(())
    }

    fn sync_host_plumbob(&mut self, session: &SessionState) -> Result<(), OrderError> {
        let host_selected = session
            .players
            .get(&session.host_id)
            .and_then(|p| p.selected_sim);
        if self.world.selected() == host_selected {
            if let Some(slot) = self.world.globals.get_mut(GLOBAL_SELECTED) {
                *slot = host_selected.map_or(0, |id| id as i16);
            }
            return Ok(());
        }
        self.ready()?;
        let sounds = RefCell::new(Vec::new());
        let interpreter = Interpreter {
            sounds: Some(&sounds),
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            ..Interpreter::new(&self.program, &self.catalog, Stream::Global)
        };
        let mut world = self.world.clone();
        interpreter
            .select(&mut world, host_selected)
            .map_err(OrderError::Refused)?;
        interpreter.refresh_family_panel(&mut world);
        self.world = world;
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
            .unwrap_or(self.view_rotation);
        let coop_cell = RefCell::new(probe_ctx);
        let sounds = RefCell::new(Vec::new());
        let character_names = RefCell::new(self.character_names.clone());
        let interpreter = Interpreter {
            sounds: Some(&sounds),
            view_rotation: player_rotation,
            view_centre: self.view_centre,
            character_names: Some(&character_names),
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

    /// Computes a deterministic 32-byte SHA-256 hash covering both `World`
    /// (format 32 + format-8 auxiliary tables) and `Lot`/co-op VM metadata.
    ///
    /// Pass this digest to `sims_net::LockstepNode::report_local_tick` and
    /// `sims_net::LockstepNode::send_snapshot` so periodic drift detection and
    /// snapshot verification match [`Self::coop_state_fingerprint`] bit-for-bit.
    pub fn coop_world_hash(&self) -> [u8; 32] {
        let w = &self.world;
        let tail = CoopWorldFingerprintTail {
            world_hash: w.hash(),
            ticks: self.ticks,
            view_rotation: self.view_rotation,
            answered: self.answered,
            asking: self.asking.as_ref().map(|a| (a.shown.tick, a.shown.object)),
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
            scratch_ptr_set: &w.scratch_ptr_set,
            scratch_ptr_set_to_next: &w.scratch_ptr_set_to_next,
            scratch_ptr_expression: &w.scratch_ptr_expression,
            scratch_ptr_arg_slot: &w.scratch_ptr_arg_slot,
            runtime_added_suits: &w.runtime_added_suits,
            suit_scan_cache: &w.suit_scan_cache,
            activity_owners: self.coop.as_ref().map(|c| &c.activity_owners),
            next_dialog_id: self.coop.as_ref().map(|c| c.next_dialog_id),
            active_dialog: self.coop.as_ref().and_then(|c| c.active_dialog),
            pending_speed_sync: self.coop.as_ref().is_some_and(|c| c.pending_speed_sync),
        };
        let encoded = postcard::to_allocvec(&tail)
            .expect("CoopWorldFingerprintTail postcard serialization is infallible");
        let mut hasher = Sha256::new();
        hasher.update(COOP_WORLD_HASH_DOMAIN);
        hasher.update(&encoded);
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
                dialog_open_session: ctx.dialog_open_session.clone(),
                pending_speed_sync: ctx.pending_speed_sync,
                play_mode: ctx.play_mode,
            },
            None => CoopSnapshotMeta {
                activity_owners: BTreeMap::new(),
                next_dialog_id: 1,
                active_dialog: None,
                dialog_open_session: None,
                pending_speed_sync: false,
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
    /// (`redb_bytes` + `session`) onto this lot.
    ///
    /// Accepts both [`Self::coop_snapshot_bytes`] envelopes and plain
    /// [`Saved::encode`] bytes.
    pub fn restore_coop_snapshot(
        &mut self,
        bytes: &[u8],
        session: SessionState,
    ) -> Result<(), SaveError> {
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
            (
                bytes.to_vec(),
                CoopSnapshotMeta {
                    activity_owners: BTreeMap::new(),
                    next_dialog_id: 1,
                    active_dialog: None,
                    dialog_open_session: None,
                    pending_speed_sync: false,
                    play_mode: false,
                },
            )
        };

        let saved = Saved::decode(&saved_bytes)?;
        self.restore(saved)?;
        self.wait_for_answers();

        let probe_session = meta
            .dialog_open_session
            .clone()
            .unwrap_or_else(|| session.clone());
        let mut ctx = CoopVmContext::new(probe_session);
        ctx.activity_owners = meta.activity_owners;
        ctx.next_dialog_id = meta.next_dialog_id.max(1);
        ctx.active_dialog = meta.active_dialog;
        ctx.dialog_open_session = meta.dialog_open_session;
        ctx.pending_speed_sync = meta.pending_speed_sync;
        ctx.play_mode = meta.play_mode;
        self.coop = Some(ctx);

        // If the snapshot was captured while a blocking dialog was awaiting an
        // answer (`Saved` intentionally stores the pre-tick world without
        // `Lot::asking`), re-run the tick probe using `dialog_open_session` so
        // `self.asking` and `self.replay_view` match the host identically even
        // if players changed camera/selection while the dialog remained open.
        if meta.active_dialog.is_some() && self.asking.is_none() && !self.answered {
            match self.step_in(meta.play_mode) {
                Err(StepError::Asking { .. }) => {}
                Ok(_) => {
                    return Err(SaveError::Format(
                        "co-op snapshot expected an open blocking dialog on restore, but tick \
                         completed without asking"
                            .into(),
                    ));
                }
                Err(err) => {
                    return Err(SaveError::Format(format!(
                        "replaying open co-op blocking dialog failed: {err}"
                    )));
                }
            }
        }
        if let Some(ctx) = self.coop.as_mut() {
            ctx.session = session;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Program;
    use sims_content::Catalog;
    use sims_formats::behavior::{Behavior, Instruction};
    use sims_formats::saved_objects::{Frame, Interaction, Person};
    use sims_net::{PlayerRole, SessionConfig};
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
        // Operands: left_index=0, right_index=3, op=5 (=), left_scope=8 (temps), right_scope=6 (global)
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
        // Silence unused Frame import when constructing synthetic stacks elsewhere.
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

        // Both Host's Sim (10) and Peer's Sim (20) see `MyData[11] == Global[3]` as true!
        assert!(interp.run_tree(&mut world, 10, 4097, 0, &[], 100).unwrap());
        assert!(interp.run_tree(&mut world, 20, 4097, 0, &[], 100).unwrap());
        // Unselected Sim (30) sees `MyData[11] == Global[3]` as false!
        assert!(!interp.run_tree(&mut world, 30, 4097, 0, &[], 100).unwrap());

        // Non-Sim object (40) owned by Peer 2 reads Peer 2's selected Sim (20).
        interp.run_tree(&mut world, 40, 4096, 0, &[], 100).unwrap();
        assert_eq!(world.objects[&40].temps[0], 20);

        // Removing activity owner falls back to Host's selected Sim (10).
        coop_cell.borrow_mut().activity_owners.clear();
        interp.run_tree(&mut world, 40, 4096, 0, &[], 100).unwrap();
        assert_eq!(world.objects[&40].temps[0], 10);
    }

    #[test]
    fn global_3_script_write_updates_only_affected_players() {
        let (program, catalog, mut world) = run_expr_world();
        let coop_cell = RefCell::new(CoopVmContext::new(two_player_session(Some(10), Some(20))));
        let interp = Interpreter {
            coop: Some(&coop_cell),
            ..Interpreter::new(&program, &catalog, Stream::Lot)
        };

        // Sim 20 (selected by Peer 2) writes Global[3] := 30 (e.g. Kid To Adult Backfire).
        assert!(
            interp
                .run_tree(&mut world, 20, 4098, 0, &[30], 100)
                .unwrap()
        );
        let ctx = coop_cell.borrow();
        assert_eq!(ctx.session.players[&PlayerId(1)].selected_sim, Some(10));
        assert_eq!(ctx.session.players[&PlayerId(2)].selected_sim, Some(30));
        assert_eq!(world.globals[GLOBAL_SELECTED], 10);
    }

    #[test]
    fn per_player_camera_rotation_drives_tile_depth_per_sim() {
        let (program, catalog, mut world) = run_expr_world();
        // Host P1 has Sim 10 with NORTH (0); Peer P2 has Sim 20 with SOUTH (2).
        let coop_cell = RefCell::new(CoopVmContext::new(two_player_session(Some(10), Some(20))));
        let interp = Interpreter {
            view_rotation: 0,
            coop: Some(&coop_cell),
            ..Interpreter::new(&program, &catalog, Stream::Lot)
        };

        // Object 40 is at fine (160, 320) -> tile (10, 20).
        // NORTH (0): -(10 + 20) = -30.
        // SOUTH (2): turn_tile((10, 20), 2) = (53, 43) -> -(53 + 43) = -96.
        interp.run_tree(&mut world, 10, 4099, 40, &[], 100).unwrap();
        interp.run_tree(&mut world, 20, 4099, 40, &[], 100).unwrap();
        interp.run_tree(&mut world, 30, 4099, 40, &[], 100).unwrap();
        assert_eq!(world.objects[&10].temps[0], -30);
        assert_eq!(world.objects[&20].temps[0], -96);
        // Unselected Sim 30 falls back to host rotation (NORTH = 0 -> -30).
        assert_eq!(world.objects[&30].temps[0], -30);
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
}
