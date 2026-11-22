//! Deterministic co-op multiplayer protocol types and wire framing.
//!
//! All types serialize canonically with `postcard` and avoid floating-point
//! values or `HashMap`/`HashSet` iteration order.

use crate::error::ProtocolError;
use crate::session::SessionState;
use crate::snapshot::{SnapshotChunk, SnapshotHeader};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Wire protocol version negotiated during `Hello` / `Welcome`.
pub const PROTOCOL_VERSION: u32 = 1;

/// ALPN identifier for `iroh` QUIC co-op sessions.
pub const ALPN: &[u8] = b"sims-rs/coop/1";

/// Maximum length of a single length-prefixed wire message (4 MiB).
pub const MAX_WIRE_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Default payload chunk size for REDB snapshot transfers (64 KiB).
pub const DEFAULT_SNAPSHOT_CHUNK_BYTES: u32 = 64 * 1024;

/// Unique player identifier within a co-op session, allocated monotonically
/// starting at [`PlayerId::INITIAL_HOST`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub u32);

impl PlayerId {
    /// Player ID assigned to the initial session host.
    pub const INITIAL_HOST: Self = Self(1);
}

impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "P{}", self.0)
    }
}

/// Simulation tick number. Tick `0` is the initial lot state before the first
/// lockstep batch; the first executed batch is tick `1`.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Tick(pub u64);

impl Tick {
    /// The initial pre-simulation tick (`0`).
    pub const ZERO: Self = Self(0);

    /// Returns the next simulation tick.
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// Returns `self + delta`, saturating at `u64::MAX`.
    pub const fn saturating_add(self, delta: u64) -> Self {
        Self(self.0.saturating_add(delta))
    }

    /// Returns `self - other` if `self >= other`.
    pub const fn checked_sub(self, other: Self) -> Option<u64> {
        self.0.checked_sub(other.0)
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t{}", self.0)
    }
}

/// Host leadership epoch. Starts at `1` and increments by `1` on every
/// deterministic host failover so stale packets from a departed host are
/// rejected cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Epoch(pub u64);

impl Epoch {
    /// Initial leadership epoch.
    pub const INITIAL: Self = Self(1);

    /// Returns the next leadership epoch.
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for Epoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}", self.0)
    }
}

/// Per-player strictly increasing client command submission sequence number.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct CommandSeq(pub u64);

impl CommandSeq {
    /// Returns the next sequence number.
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Unique identifier for a chunked REDB snapshot transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SnapshotId(pub u64);

impl fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "snap#{}", self.0)
    }
}

/// Unique identifier for a blocking modal dialog instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DialogId(pub u64);

impl fmt::Display for DialogId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "dlg#{}", self.0)
    }
}

/// SHA-256 fingerprint of deterministic simulation / session state.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct StateFingerprint(pub [u8; 32]);

impl StateFingerprint {
    /// Returns the underlying 32-byte digest.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for StateFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0[..8] {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// SHA-256 digest of a sorted game-file & custom-content [`crate::FileManifest`].
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct ManifestDigest(pub [u8; 32]);

impl ManifestDigest {
    /// Returns whether two manifest digests are identical.
    pub const fn matches(self, other: Self) -> bool {
        let mut i = 0;
        while i < 32 {
            if self.0[i] != other.0[i] {
                return false;
            }
            i += 1;
        }
        true
    }
}

impl fmt::Display for ManifestDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0[..8] {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Role of a participant in the co-op session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PlayerRole {
    /// Authoritative command sequencer and snapshot source.
    Host,
    /// Lockstep participant receiving host-sequenced tick batches.
    Peer,
}

/// Active UI mode of a player, aggregating into Mode Global 19.
///
/// Per `docs/vision.md`:
/// - `Build` if any player is in `Build`,
/// - else `Buy` if any player is in `Buy`,
/// - else `Live`.
///
/// The numeric values for Global 19 match `sims_vm::PanelMode` (`Build = 0`,
/// `Buy = 1`, `Live = 2`).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum PlayerMode {
    /// Live simulation mode (Global 19 = 2).
    #[default]
    Live,
    /// Buy catalog/furniture editing mode (Global 19 = 1). Pauses simulation.
    Buy,
    /// Architecture/terrain editing mode (Global 19 = 0). Pauses simulation.
    Build,
}

impl PlayerMode {
    /// Converts to the simulator's Global 19 value (`0` Build, `1` Buy, `2` Live).
    pub const fn to_global_19(self) -> i16 {
        match self {
            Self::Build => 0,
            Self::Buy => 1,
            Self::Live => 2,
        }
    }

    /// Returns whether this mode holds a Buy/Build pause reason.
    pub const fn pauses_simulation(self) -> bool {
        matches!(self, Self::Buy | Self::Build)
    }

    /// Aggregates all players' modes into the shared Mode Global 19 policy.
    pub fn aggregate<I>(modes: I) -> Self
    where
        I: IntoIterator<Item = Self>,
    {
        let mut any_buy = false;
        for mode in modes {
            match mode {
                Self::Build => return Self::Build,
                Self::Buy => any_buy = true,
                Self::Live => {}
            }
        }
        if any_buy { Self::Buy } else { Self::Live }
    }
}

/// Camera rotation in quarter turns (`0..=3`), kept per player in simulation
/// state because scripts read it (`Look Towards`, tile-depth, GSC 12, Find Location).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct CameraRotation(u8);

impl CameraRotation {
    pub const NORTH: Self = Self(0);
    pub const EAST: Self = Self(1);
    pub const SOUTH: Self = Self(2);
    pub const WEST: Self = Self(3);

    /// Creates a rotation masked to `0..=3`.
    pub const fn new(quarter_turns: u8) -> Self {
        Self(quarter_turns & 3)
    }

    /// Returns the normalized rotation in `0..=3`.
    pub const fn quarter_turns(self) -> u8 {
        self.0 & 3
    }

    /// Returns a normalized copy (defensive against untrusted wire bytes).
    pub const fn normalized(self) -> Self {
        Self(self.0 & 3)
    }
}

/// Shared simulation speed (Global 16, clamped `0..=1000`).
///
/// Constants match `crates/sims-sim/src/speed.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SimSpeed(i16);

impl SimSpeed {
    /// Normal speed button (`760`, ~42 ms/tick).
    pub const NORMAL: Self = Self(760);
    /// Fast speed button (`924`, ~14 ms/tick).
    pub const FAST: Self = Self(924);
    /// Ultra speed button (`994`, ~2 ms/tick).
    pub const ULTRA: Self = Self(994);

    /// Creates a speed clamped to `0..=1000`.
    pub const fn new(raw: i16) -> Self {
        if raw < 0 {
            Self(0)
        } else if raw > 1000 {
            Self(1000)
        } else {
            Self(raw)
        }
    }

    /// Returns the clamped speed value in `0..=1000`.
    pub const fn value(self) -> i16 {
        Self::new(self.0).0
    }

    /// Integer-only deterministic tick period in milliseconds matching
    /// `172 - round(speed * 171 / 1000)` without platform floats.
    pub const fn period_ms(self) -> u32 {
        let s = self.value() as u32;
        // Exact integer equivalent of floor(172.5 - s * 0.171):
        // (172_500 - s * 171) / 1000, matching 42 / 14 / 2 / 172 / 1 ms.
        (172_500 - s * 171) / 1000
    }
}

impl Default for SimSpeed {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// Fixed session-wide configuration negotiated on join.
///
/// Captures hidden simulation inputs that must be fixed across all machines
/// per `docs/vision.md` (Global 33 Machine Level, Global 15 Current Language,
/// Global 30 Free Will) alongside lockstep buffering and pacing parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Global 33 (`Machine Level`): fixed for the session so differing hardware
    /// never diverges visitor/pedestrian counts.
    pub machine_level: i16,
    /// Global 15 (`Current Language`): host's language for simulation scripts;
    /// each player still displays UI text in their local language.
    pub sim_language: i16,
    /// Global 30 (`Free Will`): shared session setting.
    pub free_will: bool,
    /// Whether only the host may change speed or pause/unpause.
    pub host_speed_lock: bool,
    /// Target peer buffer delay in ticks behind the host.
    pub tick_delay: u32,
    /// Minimum adaptive buffer delay in ticks.
    pub min_tick_delay: u32,
    /// Maximum adaptive buffer delay in ticks.
    pub max_tick_delay: u32,
    /// Interval in ticks between periodic state fingerprint comparisons.
    /// `0` disables periodic fingerprint checks.
    pub fingerprint_interval: u64,
    /// Maximum number of simultaneous players in the session (including host).
    pub max_players: u16,
    /// Maximum commands a single peer may have stamped into one tick batch.
    pub max_commands_per_peer_per_tick: usize,
    /// Maximum ticks the host may schedule ahead of the slowest active peer
    /// before pausing scheduling to wait for that machine.
    pub max_ticks_ahead_of_slowest: u64,
    /// Number of recent `TickBatch`es retained by nodes to serve fast failover
    /// catch-up without needing a full snapshot.
    pub retained_batch_history: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            machine_level: 100,
            sim_language: 1,
            free_will: true,
            host_speed_lock: false,
            tick_delay: 2,
            min_tick_delay: 1,
            max_tick_delay: 12,
            fingerprint_interval: 30,
            max_players: 4,
            max_commands_per_peer_per_tick: 16,
            max_ticks_ahead_of_slowest: 16,
            retained_batch_history: 128,
        }
    }
}

impl SessionConfig {
    /// Validates lockstep delay, pacing, and capacity bounds so startup cannot
    /// panic or deadlock.
    pub fn validate(&self) -> Result<(), crate::error::SessionError> {
        if self.min_tick_delay > self.max_tick_delay {
            return Err(crate::error::SessionError::InvalidConfig(format!(
                "min_tick_delay ({}) > max_tick_delay ({})",
                self.min_tick_delay, self.max_tick_delay
            )));
        }
        if self.tick_delay < self.min_tick_delay || self.tick_delay > self.max_tick_delay {
            return Err(crate::error::SessionError::InvalidConfig(format!(
                "tick_delay ({}) outside [{}..={}]",
                self.tick_delay, self.min_tick_delay, self.max_tick_delay
            )));
        }
        if self.max_players == 0 {
            return Err(crate::error::SessionError::InvalidConfig(
                "max_players must be >= 1".to_string(),
            ));
        }
        if self.max_commands_per_peer_per_tick == 0 {
            return Err(crate::error::SessionError::InvalidConfig(
                "max_commands_per_peer_per_tick must be >= 1".to_string(),
            ));
        }
        let min_lead = u64::from(self.tick_delay).max(1);
        if self.max_ticks_ahead_of_slowest < min_lead {
            return Err(crate::error::SessionError::InvalidConfig(format!(
                "max_ticks_ahead_of_slowest ({}) must be >= tick_delay.max(1) ({min_lead})",
                self.max_ticks_ahead_of_slowest
            )));
        }
        Ok(())
    }

    /// Returns a defensively sanitized copy guaranteed to satisfy [`Self::validate`].
    pub fn sanitized(mut self) -> Self {
        if self.min_tick_delay > self.max_tick_delay {
            self.max_tick_delay = self.min_tick_delay;
        }
        self.tick_delay = self
            .tick_delay
            .clamp(self.min_tick_delay, self.max_tick_delay);
        self.max_players = self.max_players.max(1);
        self.max_commands_per_peer_per_tick = self.max_commands_per_peer_per_tick.max(1);
        self.max_ticks_ahead_of_slowest = self
            .max_ticks_ahead_of_slowest
            .max(u64::from(self.tick_delay).max(1));
        self
    }
}

/// Player answer to a blocking modal dialog (mirrors `sims_vm::Answer`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogAnswer {
    /// Button status (`2`, `3`, or `4` per `0x4370be`).
    pub status: u8,
    /// Optional bitmap ID published to Global 12 while the dialog was open.
    pub bitmap_id: Option<i16>,
    /// Optional entered text for kind-3 text-entry dialogs.
    pub text: Option<String>,
}

/// Buy and Build lot edit operations stamped into lockstep ticks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuyBuildEdit {
    /// Purchase and place a catalog object.
    BuyObject {
        guid: u32,
        position: [i32; 3],
        facing: i16,
        container: Option<(u16, i16)>,
    },
    /// Move and/or rotate an existing object on the lot.
    MoveObject {
        object_id: u16,
        position: [i32; 3],
        facing: i16,
        container: Option<(u16, i16)>,
    },
    /// Sell/delete an existing object on the lot.
    SellObject { object_id: u16 },
    /// Place a wall segment.
    PlaceWall {
        x: i16,
        y: i16,
        level: u8,
        style: u16,
        pattern: u16,
    },
    /// Remove a wall segment.
    RemoveWall { x: i16, y: i16, level: u8 },
    /// Paint or remove a floor tile.
    SetFloorTile {
        x: i16,
        y: i16,
        level: u8,
        pattern: u16,
    },
    /// Apply wallpaper to one side of a wall tile.
    SetWallpaper {
        x: i16,
        y: i16,
        level: u8,
        side: u8,
        pattern: u16,
    },
    /// Raise or lower terrain vertex elevation.
    SetTerrainHeight { x: i16, y: i16, height: i16 },
}

/// Reason a player left the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LeaveReason {
    /// Player gracefully exited.
    Voluntary,
    /// Transport connection dropped or timed out.
    Disconnected,
    /// Host departed and triggered deterministic failover.
    HostDisconnected,
    /// Host kicked or closed the session.
    SessionClosed,
}

/// Deterministic external lot command stamped with a tick by the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Pick an interaction from a pie menu for `sim_id` on `target_id`.
    SimActionPick {
        sim_id: u16,
        target_id: u16,
        menu_index: i16,
        joining: bool,
        args: [i16; 4],
    },
    /// Cancel a running or queued interaction on `sim_id`.
    SimActionCancel { sim_id: u16, interaction_uid: i16 },
    /// Change the shared simulation speed (respects `host_speed_lock`).
    SetSpeed { speed: SimSpeed },
    /// Toggle explicit pause (respects `host_speed_lock`).
    SetPaused { paused: bool },
    /// Enable or disable the optional host speed/pause lock (host-only).
    SetHostSpeedLock { locked: bool },
    /// Toggle shared Global 30 Free Will.
    SetFreeWill { enabled: bool },
    /// Execute a Buy or Build lot edit.
    BuyBuild(BuyBuildEdit),
    /// Answer a blocking modal dialog (first-answer-wins in host tick order).
    AnswerBlockingDialog {
        dialog_id: DialogId,
        answer: DialogAnswer,
    },
    /// Change the issuing player's `Selected Person ID` (per-player Global 3).
    SelectPerson { sim_id: Option<u16> },
    /// Change the issuing player's `camera_rotation` (per-player sim state).
    SetCameraRotation { rotation: CameraRotation },
    /// Change the issuing player's Live / Buy / Build mode (Mode Global 19 &
    /// Buy/Build pause reason).
    SetMode { mode: PlayerMode },
    /// Host-issued membership event: a new player joined the session.
    PlayerJoined {
        player_id: PlayerId,
        name: String,
        selected_sim: Option<u16>,
        camera_rotation: CameraRotation,
        endpoint_hint: Option<String>,
        transport_addr: Option<String>,
    },
    /// Host-issued membership event: a player left the session.
    PlayerLeft {
        player_id: PlayerId,
        reason: LeaveReason,
    },
    /// Host-issued leadership event: deterministic host failover occurred.
    HostChanged { new_host: PlayerId, epoch: Epoch },
}

impl Command {
    /// Returns whether this command is a host-generated system membership
    /// event that peers may never submit via [`WireMessage::SubmitCommand`].
    pub const fn is_system_membership(&self) -> bool {
        matches!(
            self,
            Self::PlayerJoined { .. } | Self::PlayerLeft { .. } | Self::HostChanged { .. }
        )
    }
}

/// A command stamped with its authoritative tick and intra-tick sequence number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StampedCommand {
    /// Simulation tick on which this command executes.
    pub tick: Tick,
    /// Deterministic `0..n` order within `tick`.
    pub sequence: u32,
    /// Player who issued the command.
    pub issuer: PlayerId,
    /// Command payload.
    pub command: Command,
}

/// Host-sequenced tick batch broadcast to every peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickBatch {
    /// Host leadership epoch that issued this batch.
    pub epoch: Epoch,
    /// Simulation tick covered by this batch.
    pub tick: Tick,
    /// Authoritative ordered commands for `tick`.
    pub commands: Vec<StampedCommand>,
    /// Adaptive tick pacing period (ms) computed by the host for the slowest
    /// active machine.
    pub pacing_hint_ms: u32,
    /// Optional host state fingerprint from a previously executed tick
    /// (`fp_tick < tick`) for peer-side drift detection.
    pub host_fingerprint: Option<(Tick, StateFingerprint)>,
}

/// Summary of a connected participant shared during join and host failover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRosterEntry {
    pub player_id: PlayerId,
    pub name: String,
    pub role: PlayerRole,
    /// Transport address or invite ticket used for direct reconnection during
    /// host failover.
    pub endpoint_hint: Option<String>,
    pub mode: PlayerMode,
    pub selected_sim: Option<u16>,
    pub camera_rotation: CameraRotation,
    pub last_acked_tick: Tick,
    pub reported_step_us: u32,
}

/// Reason a host rejected an incoming `Hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    /// Peer protocol version does not match host.
    ProtocolVersionMismatch { expected: u32, actual: u32 },
    /// Game files or custom content manifest digest does not match host.
    ManifestMismatch {
        expected: ManifestDigest,
        actual: ManifestDigest,
    },
    /// Session has reached `max_players`.
    SessionFull { max_players: u16 },
    /// Contacted endpoint is not currently the session host.
    NotHost { current_host: PlayerId },
    /// Player name was empty or invalid.
    InvalidPlayerName,
    /// Provided endpoint hint did not match the authenticated transport peer.
    InvalidEndpointHint,
    /// Peer received an invalid session configuration from host.
    InvalidConfig,
}

/// Reason a REDB snapshot transfer was initiated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResyncReason {
    /// Peer joined an already-running session mid-game.
    MidGameJoin,
    /// State fingerprint comparison detected simulation drift.
    DriftDetected,
    /// Peer was further behind than the new host's retained batch history
    /// after host failover.
    HostFailover,
}

/// Top-level wire message sent over [`crate::transport::Transport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireMessage {
    /// Peer -> Host join handshake carrying protocol version and file manifest.
    Hello {
        protocol_version: u32,
        player_name: String,
        manifest: ManifestDigest,
        endpoint_hint: Option<String>,
    },
    /// Host -> Peer join acceptance.
    Welcome {
        epoch: Epoch,
        assigned_player: PlayerId,
        host_id: PlayerId,
        config: SessionConfig,
        current_tick: Tick,
        session_state: Box<SessionState>,
        snapshot_id: Option<SnapshotId>,
        roster: Vec<PeerRosterEntry>,
    },
    /// Host -> Peer join refusal.
    Reject { reason: RejectReason },
    /// Peer -> Host input command submission.
    SubmitCommand {
        epoch: Epoch,
        seq: CommandSeq,
        command: Command,
    },
    /// Host -> Peer authoritative tick batch.
    TickBatch(TickBatch),
    /// Peer -> Host tick execution acknowledgement, step time, and optional
    /// periodic state fingerprint.
    TickAck {
        epoch: Epoch,
        tick: Tick,
        sim_step_us: u32,
        fingerprint: Option<StateFingerprint>,
    },
    /// Peer -> Host request for a REDB snapshot resync.
    RequestSnapshot {
        epoch: Epoch,
        reason: ResyncReason,
        last_known_tick: Tick,
    },
    /// Host -> Peer start of a chunked/compressed REDB snapshot transfer.
    SnapshotHeader(SnapshotHeader),
    /// Host -> Peer individual snapshot payload chunk.
    SnapshotChunk(SnapshotChunk),
    /// Peer -> Host acknowledgement of snapshot progress or completion.
    SnapshotAck {
        epoch: Epoch,
        snapshot_id: SnapshotId,
        received_chunks: u32,
        complete: bool,
    },
    /// New Host -> Surviving Peers announcement during deterministic failover.
    HostTakeover {
        new_epoch: Epoch,
        new_host: PlayerId,
        resume_tick: Tick,
        resume_fingerprint: Option<StateFingerprint>,
        session_state: Box<SessionState>,
        roster: Vec<PeerRosterEntry>,
    },
    /// Lightweight liveness / failover prompt carrying sender identity.
    Heartbeat {
        epoch: Epoch,
        tick: Tick,
        sender: Option<PlayerId>,
    },
    /// Graceful departure notice.
    Goodbye { epoch: Epoch, reason: LeaveReason },
}

/// Encodes a [`WireMessage`] as raw `postcard` bytes, enforcing
/// [`MAX_WIRE_FRAME_BYTES`].
pub fn encode_payload(msg: &WireMessage) -> Result<Vec<u8>, ProtocolError> {
    let bytes = postcard::to_allocvec(msg).map_err(|e| ProtocolError::Encode(e.to_string()))?;
    if bytes.len() > MAX_WIRE_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            len: bytes.len(),
            max: MAX_WIRE_FRAME_BYTES,
        });
    }
    Ok(bytes)
}

/// Decodes a [`WireMessage`] from raw `postcard` bytes, enforcing
/// [`MAX_WIRE_FRAME_BYTES`] and rejecting trailing bytes.
pub fn decode_payload(bytes: &[u8]) -> Result<WireMessage, ProtocolError> {
    if bytes.len() > MAX_WIRE_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            len: bytes.len(),
            max: MAX_WIRE_FRAME_BYTES,
        });
    }
    let (msg, rest) =
        postcard::take_from_bytes(bytes).map_err(|e| ProtocolError::Decode(e.to_string()))?;
    if !rest.is_empty() {
        return Err(ProtocolError::Decode(format!(
            "unexpected {} trailing bytes in wire payload",
            rest.len()
        )));
    }
    Ok(msg)
}

/// Encodes a [`WireMessage`] with a 4-byte big-endian length prefix.
pub fn encode_framed(msg: &WireMessage) -> Result<Vec<u8>, ProtocolError> {
    let payload = encode_payload(msg)?;
    let len = u32::try_from(payload.len()).map_err(|_| ProtocolError::FrameTooLarge {
        len: payload.len(),
        max: MAX_WIRE_FRAME_BYTES,
    })?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decodes a single 4-byte big-endian length-prefixed [`WireMessage`].
pub fn decode_framed(frame: &[u8]) -> Result<WireMessage, ProtocolError> {
    if frame.len() < 4 {
        return Err(ProtocolError::InvalidFrameLength {
            expected: 4,
            actual: frame.len(),
        });
    }
    let payload_len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if payload_len > MAX_WIRE_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            len: payload_len,
            max: MAX_WIRE_FRAME_BYTES,
        });
    }
    let expected = 4 + payload_len;
    if frame.len() != expected {
        return Err(ProtocolError::InvalidFrameLength {
            expected,
            actual: frame.len(),
        });
    }
    decode_payload(&frame[4..])
}
