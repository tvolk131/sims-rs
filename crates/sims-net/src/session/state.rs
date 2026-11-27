//! Pure deterministic co-op session state.
//!
//! Covers all session-level rules from `docs/vision.md` ("Multiplayer (v1)"):
//! - Multi-reason pause (explicit pause, any player in Buy/Build, unanswered
//!   blocking dialog).
//! - Mode Global 19 aggregation (`Build` if any in Build, else `Buy` if any in
//!   Buy, else `Live`).
//! - Shared speed/pause control with optional host lock and player attribution.
//! - Blocking dialogs with deterministic first-answer-wins ordering.
//! - Per-player simulation inputs (`Selected Person ID` and `camera_rotation`).
//! - Deterministic failover host selection and canonical SHA-256 fingerprinting.

use crate::error::SessionError;
use crate::protocol::{
    CameraRotation, Command, DialogAnswer, DialogId, Epoch, LeaveReason, PeerRosterEntry, PlayerId,
    PlayerMode, PlayerRole, SessionConfig, SimSpeed, StampedCommand, StateFingerprint, Tick,
    TickBatch,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const SESSION_FP_DOMAIN: &[u8] = b"sims-net/session-state/v1\0";
const COMBINED_FP_DOMAIN: &[u8] = b"sims-net/combined-state/v1\0";

/// Simulation-visible per-player state kept deterministically in [`SessionState`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerPlayerState {
    pub player_id: PlayerId,
    pub name: String,
    pub role: PlayerRole,
    pub mode: PlayerMode,
    /// Per-player Global 3 (`Selected Person ID`).
    pub selected_sim: Option<u16>,
    /// Per-player `camera_rotation` (`0..=3`), read by SimAntics scripts.
    pub camera_rotation: CameraRotation,
    /// Optional transport address / invite ticket used for host failover.
    pub endpoint_hint: Option<String>,
    /// Canonical authenticated transport identity bound on join.
    pub transport_addr: Option<String>,
}

/// Shared speed, pause, and attribution state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeControlState {
    /// Current shared simulation speed (Global 16, `0..=1000`).
    pub speed: SimSpeed,
    /// Whether an explicit pause is currently active.
    pub explicit_pause: bool,
    /// Who last changed explicit pause (for UI attribution).
    pub explicit_pause_by: Option<PlayerId>,
    /// Who last changed speed (for UI attribution).
    pub last_speed_change_by: Option<PlayerId>,
    /// Optional host lock preventing peers from changing speed or pause.
    pub host_speed_lock: bool,
}

impl TimeControlState {
    /// Creates default time control state from [`SessionConfig`].
    pub const fn from_config(config: &SessionConfig) -> Self {
        Self {
            speed: SimSpeed::NORMAL,
            explicit_pause: false,
            explicit_pause_by: None,
            last_speed_change_by: None,
            host_speed_lock: config.host_speed_lock,
        }
    }
}

/// Breakdown of every pause reason currently holding the simulation paused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivePauseReasons {
    /// Explicit pause toggled by a player.
    pub explicit_pause: bool,
    /// Player who set explicit pause, if any.
    pub explicit_pause_by: Option<PlayerId>,
    /// Players currently in Buy or Build mode (sorted by `PlayerId`).
    pub players_in_buy_or_build: Vec<(PlayerId, PlayerMode)>,
    /// Open unanswered blocking dialogs (sorted by `DialogId`).
    pub unanswered_dialogs: Vec<DialogId>,
}

impl ActivePauseReasons {
    /// Returns whether any pause reason currently holds.
    pub fn is_paused(&self) -> bool {
        self.explicit_pause
            || !self.players_in_buy_or_build.is_empty()
            || !self.unanswered_dialogs.is_empty()
    }
}

/// An open blocking modal dialog waiting for the first answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockingDialog {
    pub dialog_id: DialogId,
    pub object_id: u16,
    pub opened_at: Tick,
}

/// Why a stamped command was rejected by deterministic session policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRejectReason {
    /// Speed/pause controls are locked to the host.
    SpeedLockedByHost,
    /// Command may only be executed by the current host.
    HostOnly,
    /// Issuing player is not currently a member of the session.
    UnknownIssuer,
}

/// Why a valid command had no effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandIgnoreReason {
    /// Another player already answered this blocking dialog earlier in tick order.
    DialogAlreadyAnswered,
    /// Player ID was already present or already absent.
    RedundantMembershipChange,
}

/// Outcome of applying one [`StampedCommand`] to [`SessionState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    /// Command accepted and applied (or forwarded to the simulation VM).
    Applied,
    /// Blocking dialog answered by the winning player (first-answer-wins).
    DialogWon {
        dialog_id: DialogId,
        winner: PlayerId,
        answer: DialogAnswer,
    },
    /// Command rejected by deterministic policy.
    Rejected(CommandRejectReason),
    /// Command was valid but superseded or redundant.
    Ignored(CommandIgnoreReason),
}

/// Result record for one command in an [`AppliedBatch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedCommand {
    pub stamped: StampedCommand,
    pub outcome: CommandOutcome,
}

/// Result of applying a full [`TickBatch`] to [`SessionState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedBatch {
    pub epoch: Epoch,
    pub tick: Tick,
    pub pacing_hint_ms: u32,
    pub commands: Vec<AppliedCommand>,
    /// Effective Mode Global 19 after applying this batch.
    pub effective_mode: PlayerMode,
    /// Whether the simulation is paused after applying this batch.
    pub is_paused: bool,
    /// Active speed after applying this batch.
    pub speed: SimSpeed,
}

impl AppliedBatch {
    /// Iterates over accepted simulation commands in exact deterministic tick
    /// sequence order for execution by the VM.
    pub fn accepted_commands(&self) -> impl Iterator<Item = &StampedCommand> {
        self.commands.iter().filter_map(|item| match item.outcome {
            CommandOutcome::Applied | CommandOutcome::DialogWon { .. } => Some(&item.stamped),
            CommandOutcome::Rejected(_) | CommandOutcome::Ignored(_) => None,
        })
    }
}

/// Deterministic co-op session state shared identically by host and all peers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionState {
    pub epoch: Epoch,
    pub host_id: PlayerId,
    pub next_player_id: u32,
    pub current_tick: Tick,
    pub config: SessionConfig,
    pub players: BTreeMap<PlayerId, PerPlayerState>,
    pub time: TimeControlState,
    pub open_dialogs: BTreeMap<DialogId, BlockingDialog>,
}

impl SessionState {
    /// Initializes a new co-op session hosted by [`PlayerId::INITIAL_HOST`].
    pub fn new(
        host_name: impl Into<String>,
        host_endpoint_hint: Option<String>,
        config: SessionConfig,
    ) -> Self {
        let config = config.sanitized();
        let host_id = PlayerId::INITIAL_HOST;
        let host_transport_addr = host_endpoint_hint
            .as_deref()
            .map(crate::session::lockstep::canonical_addr_string_from_hint);
        let mut players = BTreeMap::new();
        players.insert(
            host_id,
            PerPlayerState {
                player_id: host_id,
                name: host_name.into(),
                role: PlayerRole::Host,
                mode: PlayerMode::Live,
                selected_sim: None,
                camera_rotation: CameraRotation::NORTH,
                endpoint_hint: host_endpoint_hint,
                transport_addr: host_transport_addr,
            },
        );
        let time = TimeControlState::from_config(&config);
        Self {
            epoch: Epoch::INITIAL,
            host_id,
            next_player_id: host_id.0.saturating_add(1),
            current_tick: Tick::ZERO,
            config,
            players,
            time,
            open_dialogs: BTreeMap::new(),
        }
    }

    /// Allocates the next monotonic [`PlayerId`] (called on the host).
    pub fn allocate_player_id(&mut self) -> PlayerId {
        let id = PlayerId(self.next_player_id);
        self.next_player_id = self.next_player_id.saturating_add(1);
        id
    }

    /// Registers a blocking modal dialog opened by the simulation.
    ///
    /// While any blocking dialog remains unanswered, [`Self::is_paused`] is
    /// `true`. The first [`Command::AnswerBlockingDialog`] in host tick order
    /// closes the dialog and wins.
    pub fn open_blocking_dialog(
        &mut self,
        dialog_id: DialogId,
        object_id: u16,
        opened_at: Tick,
    ) -> bool {
        if self.open_dialogs.contains_key(&dialog_id) {
            return false;
        }
        self.open_dialogs.insert(
            dialog_id,
            BlockingDialog {
                dialog_id,
                object_id,
                opened_at,
            },
        );
        true
    }

    /// Returns a detailed breakdown of all active pause reasons.
    pub fn pause_reasons(&self) -> ActivePauseReasons {
        let players_in_buy_or_build = self
            .players
            .values()
            .filter(|p| p.mode.pauses_simulation())
            .map(|p| (p.player_id, p.mode))
            .collect();
        let unanswered_dialogs = self.open_dialogs.keys().copied().collect();
        ActivePauseReasons {
            explicit_pause: self.time.explicit_pause,
            explicit_pause_by: self.time.explicit_pause_by,
            players_in_buy_or_build,
            unanswered_dialogs,
        }
    }

    /// Returns whether the lot simulation is paused (any pause reason holds).
    pub fn is_paused(&self) -> bool {
        self.time.explicit_pause
            || self.players.values().any(|p| p.mode.pauses_simulation())
            || !self.open_dialogs.is_empty()
    }

    /// Returns the aggregated Mode Global 19 policy:
    /// `Build` if any player is in `Build`, else `Buy` if any is in `Buy`, else
    /// `Live`.
    pub fn effective_mode(&self) -> PlayerMode {
        PlayerMode::aggregate(self.players.values().map(|p| p.mode))
    }

    /// Global 19 integer value (`0` Build, `1` Buy, `2` Live).
    pub fn mode_global_19(&self) -> i16 {
        self.effective_mode().to_global_19()
    }

    /// Global 3 ("Am I the selected Sim?"): returns `true` if **any** player in
    /// the session currently has `sim_id` selected.
    pub fn is_sim_selected_by_any(&self, sim_id: u16) -> bool {
        self.players
            .values()
            .any(|p| p.selected_sim == Some(sim_id))
    }

    /// Global 3 ("Who is the player's Sim?"): returns the selected Sim of
    /// `actor_player` if provided and currently selecting a Sim, falling back
    /// to the host's selected Sim.
    pub fn selected_sim_for_activity(&self, actor_player: Option<PlayerId>) -> Option<u16> {
        if let Some(actor) = actor_player
            && let Some(sim) = self.players.get(&actor).and_then(|p| p.selected_sim)
        {
            return Some(sim);
        }
        self.players.get(&self.host_id).and_then(|p| p.selected_sim)
    }

    /// Per-player `camera_rotation` policy (`docs/vision.md`): returns the
    /// rotation of the player who has `sim_id` selected (deterministic lowest
    /// [`PlayerId`] tie-breaker if multiple players selected the same Sim),
    /// falling back to the host's rotation.
    pub fn camera_rotation_for_sim(&self, sim_id: u16) -> CameraRotation {
        if let Some(player) = self
            .players
            .values()
            .find(|p| p.selected_sim == Some(sim_id))
        {
            return player.camera_rotation.normalized();
        }
        self.players
            .get(&self.host_id)
            .map(|p| p.camera_rotation.normalized())
            .unwrap_or(CameraRotation::NORTH)
    }

    /// Script-driven selection change (`docs/vision.md`, e.g. *Magic – Kid To
    /// Adult Backfire*): updates every player who currently had `affected_sim`
    /// selected. Returns the list of affected [`PlayerId`]s.
    pub fn apply_script_selection_change(
        &mut self,
        affected_sim: u16,
        new_sim: Option<u16>,
    ) -> Vec<PlayerId> {
        let mut updated = Vec::new();
        for player in self.players.values_mut() {
            if player.selected_sim == Some(affected_sim) {
                player.selected_sim = new_sim;
                updated.push(player.player_id);
            }
        }
        updated
    }

    /// Deterministically selects the next host if the current host departs:
    /// lowest [`PlayerId`] among remaining players.
    pub fn select_next_host(&self) -> Option<PlayerId> {
        self.players.keys().copied().find(|&id| id != self.host_id)
    }

    /// Deterministically selects the next host excluding both the current host
    /// and any unreachable/failed failover candidates.
    pub fn select_next_host_excluding(&self, excluded: &BTreeSet<PlayerId>) -> Option<PlayerId> {
        self.players
            .keys()
            .copied()
            .find(|id| *id != self.host_id && !excluded.contains(id))
    }

    /// Builds a deterministic roster snapshot (sorted by [`PlayerId`]).
    pub fn roster(&self) -> Vec<PeerRosterEntry> {
        self.players
            .values()
            .map(|p| PeerRosterEntry {
                player_id: p.player_id,
                name: p.name.clone(),
                role: p.role,
                endpoint_hint: p.endpoint_hint.clone(),
                mode: p.mode,
                selected_sim: p.selected_sim,
                camera_rotation: p.camera_rotation.normalized(),
                last_acked_tick: self.current_tick,
                reported_step_us: 0,
            })
            .collect()
    }

    /// Computes a canonical SHA-256 fingerprint of this [`SessionState`].
    pub fn session_fingerprint(&self) -> StateFingerprint {
        let encoded =
            postcard::to_allocvec(self).expect("SessionState postcard serialization is infallible");
        let mut hasher = Sha256::new();
        hasher.update(SESSION_FP_DOMAIN);
        hasher.update(&encoded);
        StateFingerprint(hasher.finalize().into())
    }

    /// Combines a simulation `World::hash()` with this session's canonical
    /// fingerprint into a single 32-byte [`StateFingerprint`].
    pub fn combined_fingerprint(&self, world_hash: [u8; 32]) -> StateFingerprint {
        let session_fp = self.session_fingerprint();
        let mut hasher = Sha256::new();
        hasher.update(COMBINED_FP_DOMAIN);
        hasher.update(world_hash);
        hasher.update(session_fp.0);
        StateFingerprint(hasher.finalize().into())
    }

    /// Applies one authoritative [`TickBatch`] to [`SessionState`], validating
    /// epoch, tick continuity, and intra-tick sequence numbering.
    pub fn apply_batch(&mut self, batch: &TickBatch) -> Result<AppliedBatch, SessionError> {
        if batch.epoch != self.epoch {
            return Err(SessionError::EpochMismatch {
                expected: self.epoch,
                actual: batch.epoch,
            });
        }
        let expected_tick = self.current_tick.next();
        if batch.tick != expected_tick {
            return Err(SessionError::UnexpectedTick {
                expected: expected_tick,
                actual: batch.tick,
            });
        }

        for (idx, cmd) in batch.commands.iter().enumerate() {
            if cmd.tick != batch.tick {
                return Err(SessionError::CommandTickMismatch {
                    batch_tick: batch.tick,
                    command_tick: cmd.tick,
                });
            }
            let expected_seq = idx as u32;
            if cmd.sequence != expected_seq {
                return Err(SessionError::InvalidCommandSequence {
                    expected: expected_seq,
                    actual: cmd.sequence,
                });
            }
        }

        let mut applied = Vec::with_capacity(batch.commands.len());
        for stamped in &batch.commands {
            let outcome = self.apply_single_command(stamped);
            applied.push(AppliedCommand {
                stamped: stamped.clone(),
                outcome,
            });
        }

        self.current_tick = batch.tick;

        Ok(AppliedBatch {
            epoch: batch.epoch,
            tick: batch.tick,
            pacing_hint_ms: batch.pacing_hint_ms,
            commands: applied,
            effective_mode: self.effective_mode(),
            is_paused: self.is_paused(),
            speed: self.time.speed,
        })
    }

    fn apply_single_command(&mut self, stamped: &StampedCommand) -> CommandOutcome {
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
                if issuer != self.host_id {
                    return CommandOutcome::Rejected(CommandRejectReason::HostOnly);
                }
                if self.players.contains_key(player_id) {
                    return CommandOutcome::Ignored(CommandIgnoreReason::RedundantMembershipChange);
                }
                if player_id.0 >= self.next_player_id {
                    self.next_player_id = player_id.0.saturating_add(1);
                }
                let canonical_transport = transport_addr.clone().or_else(|| {
                    endpoint_hint
                        .as_deref()
                        .map(crate::session::lockstep::canonical_addr_string_from_hint)
                });
                self.players.insert(
                    *player_id,
                    PerPlayerState {
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
                CommandOutcome::Applied
            }
            Command::PlayerLeft {
                player_id,
                reason: _,
            } => {
                if issuer != self.host_id {
                    return CommandOutcome::Rejected(CommandRejectReason::HostOnly);
                }
                if self.players.remove(player_id).is_none() {
                    return CommandOutcome::Ignored(CommandIgnoreReason::RedundantMembershipChange);
                }
                CommandOutcome::Applied
            }
            Command::HostChanged { new_host, epoch } => {
                if issuer != self.host_id && issuer != *new_host {
                    return CommandOutcome::Rejected(CommandRejectReason::HostOnly);
                }
                if !self.players.contains_key(new_host) {
                    return CommandOutcome::Rejected(CommandRejectReason::UnknownIssuer);
                }
                for player in self.players.values_mut() {
                    player.role = if player.player_id == *new_host {
                        PlayerRole::Host
                    } else {
                        PlayerRole::Peer
                    };
                }
                self.host_id = *new_host;
                if *epoch > self.epoch {
                    self.epoch = *epoch;
                }
                CommandOutcome::Applied
            }
            Command::SetHostSpeedLock { locked } => {
                if issuer != self.host_id {
                    return CommandOutcome::Rejected(CommandRejectReason::HostOnly);
                }
                self.time.host_speed_lock = *locked;
                self.config.host_speed_lock = *locked;
                CommandOutcome::Applied
            }
            Command::SetSpeed { speed } => {
                if !self.players.contains_key(&issuer) {
                    return CommandOutcome::Rejected(CommandRejectReason::UnknownIssuer);
                }
                if self.time.host_speed_lock && issuer != self.host_id {
                    return CommandOutcome::Rejected(CommandRejectReason::SpeedLockedByHost);
                }
                self.time.speed = SimSpeed::new(speed.value());
                self.time.last_speed_change_by = Some(issuer);
                CommandOutcome::Applied
            }
            Command::SetPaused { paused } => {
                if !self.players.contains_key(&issuer) {
                    return CommandOutcome::Rejected(CommandRejectReason::UnknownIssuer);
                }
                if self.time.host_speed_lock && issuer != self.host_id {
                    return CommandOutcome::Rejected(CommandRejectReason::SpeedLockedByHost);
                }
                self.time.explicit_pause = *paused;
                self.time.explicit_pause_by = Some(issuer);
                CommandOutcome::Applied
            }
            Command::SetFreeWill { enabled } => {
                if !self.players.contains_key(&issuer) {
                    return CommandOutcome::Rejected(CommandRejectReason::UnknownIssuer);
                }
                self.config.free_will = *enabled;
                CommandOutcome::Applied
            }
            Command::AnswerBlockingDialog { dialog_id, answer } => {
                if !self.players.contains_key(&issuer) {
                    return CommandOutcome::Rejected(CommandRejectReason::UnknownIssuer);
                }
                if self.open_dialogs.remove(dialog_id).is_some() {
                    CommandOutcome::DialogWon {
                        dialog_id: *dialog_id,
                        winner: issuer,
                        answer: answer.clone(),
                    }
                } else {
                    CommandOutcome::Ignored(CommandIgnoreReason::DialogAlreadyAnswered)
                }
            }
            Command::SelectPerson { sim_id } => {
                let Some(player) = self.players.get_mut(&issuer) else {
                    return CommandOutcome::Rejected(CommandRejectReason::UnknownIssuer);
                };
                player.selected_sim = *sim_id;
                CommandOutcome::Applied
            }
            Command::SetCameraRotation { rotation } => {
                let Some(player) = self.players.get_mut(&issuer) else {
                    return CommandOutcome::Rejected(CommandRejectReason::UnknownIssuer);
                };
                player.camera_rotation = rotation.normalized();
                CommandOutcome::Applied
            }
            Command::SetMode { mode } => {
                let Some(player) = self.players.get_mut(&issuer) else {
                    return CommandOutcome::Rejected(CommandRejectReason::UnknownIssuer);
                };
                player.mode = *mode;
                CommandOutcome::Applied
            }
            Command::SimActionPick { .. }
            | Command::SimActionCancel { .. }
            | Command::BuyBuild(_)
            | Command::AnswerNonModalDialog { .. } => {
                if !self.players.contains_key(&issuer) {
                    return CommandOutcome::Rejected(CommandRejectReason::UnknownIssuer);
                }
                CommandOutcome::Applied
            }
        }
    }

    /// Removes the departed host and promotes `new_host` in-place during
    /// deterministic host failover (called by the promoted host before
    /// broadcasting [`crate::WireMessage::HostTakeover`]).
    pub fn promote_host(&mut self, new_host: PlayerId) -> Result<Epoch, SessionError> {
        if !self.players.contains_key(&new_host) {
            return Err(SessionError::UnknownPlayer(new_host));
        }
        let old_host = self.host_id;
        if old_host != new_host {
            self.players.remove(&old_host);
        }
        self.epoch = self.epoch.next();
        self.host_id = new_host;
        for player in self.players.values_mut() {
            player.role = if player.player_id == new_host {
                PlayerRole::Host
            } else {
                PlayerRole::Peer
            };
        }
        Ok(self.epoch)
    }

    /// Adopts a `HostTakeover` state snapshot when the peer is already at
    /// `resume_tick` (keeping deterministic membership and epoch in sync).
    pub fn adopt_failover(
        &mut self,
        new_epoch: Epoch,
        new_host: PlayerId,
        old_host_left: Option<PlayerId>,
    ) -> Result<(), SessionError> {
        if new_epoch <= self.epoch {
            return Err(SessionError::EpochMismatch {
                expected: self.epoch.next(),
                actual: new_epoch,
            });
        }
        if let Some(departed) = old_host_left
            && departed != new_host
        {
            self.players.remove(&departed);
        }
        if !self.players.contains_key(&new_host) {
            return Err(SessionError::UnknownPlayer(new_host));
        }
        self.epoch = new_epoch;
        self.host_id = new_host;
        for player in self.players.values_mut() {
            player.role = if player.player_id == new_host {
                PlayerRole::Host
            } else {
                PlayerRole::Peer
            };
        }
        Ok(())
    }

    /// Ensures a peer's `LeaveReason` stays referenced in public signatures.
    pub fn leave_command(player_id: PlayerId, reason: LeaveReason) -> Command {
        Command::PlayerLeft { player_id, reason }
    }
}
