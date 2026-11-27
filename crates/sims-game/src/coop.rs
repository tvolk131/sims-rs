//! Co-op multiplayer live controller, transport bridge (`MemoryMesh` and real
//! `iroh` P2P QUIC), install manifest digesting, and lockstep session lifecycle.
//!
//! Bridges the synchronous `winit` / headless [`crate::Game`] loop with
//! [`sims_net::LockstepNode`] and [`sims_vm::coop`] without blocking the UI
//! thread:
//! - [`IrohBridge`] runs `iroh::Endpoint` on a dedicated background Tokio
//!   runtime and hands non-blocking events/frames across channels.
//! - [`CoopTransport`] unifies [`sims_net::MemoryNodeTransport`] and
//!   [`IrohBridge`] behind one [`sims_net::Transport`] implementation so tests
//!   and live gameplay execute the exact same [`CoopController`] code path.
//! - [`CoopController`] manages host/join handshakes, `"SCOP"` v2 snapshot
//!   transfer & verified restoration, periodic [`Lot::coop_world_hash`] drift
//!   detection and automatic resync, host failover, UI [`CoopSnapshot`]
//!   projection, and clean zero-leak shutdown.

use sims_content::Install;
use sims_net::{
    Command, DEFAULT_SNAPSHOT_CHUNK_BYTES, FileManifest, HostStepResult, InviteTicket, IrohConfig,
    IrohTransport, LeaveReason, LockstepNode, ManifestDigest, ManifestError, MemoryNodeTransport,
    NodeEvent, PeerAddress, PlayerId, PlayerMode, PlayerRole, RejectReason, ResyncReason,
    SessionConfig, SimSpeed, SnapshotCompression, Tick, Transport, TransportError, TransportEvent,
    WireMessage,
};
use sims_ui::coop::{
    ConnectionState, CoopRole, CoopSnapshot, PauseReason, PlayerId as UiPlayerId, PlayerInfo,
    PlayerMode as UiPlayerMode, ResyncProgress, TimeAction, TimeAttribution,
};
use sims_vm::boot::{Frames, Lot};
use sims_vm::coop::{CoopBatchReport, CoopCommandStatus};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

/// Duration (ms) a speed/pause attribution toast remains visible in
/// [`sims_ui::coop::CoopHudOverlay`].
const TIME_TOAST_DURATION_MS: u32 = 3_000;

/// Brief drain window before `iroh::Endpoint::close()` so queued `Goodbye`
/// frames flush cleanly after `leave_gracefully` calls `transport.disconnect`.
const GOODBYE_DRAIN_WINDOW: Duration = Duration::from_millis(35);

/// Bounded wait for graceful `iroh::Endpoint::close()` on disconnect/drop so
/// `Goodbye` frames flush cleanly without leaking threads or hanging exit.
const IROH_CLOSE_TIMEOUT: Duration = Duration::from_millis(450);

/// Bounded wait for Tokio worker pool shutdown after closing the endpoint.
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(300);

/// Fallback process-local clipboard buffer used when native OS clipboard tools
/// are unavailable (or in headless tests).
static FALLBACK_CLIPBOARD: OnceLock<Mutex<String>> = OnceLock::new();

/// Copies `text` to the system clipboard (macOS `pbcopy`, plus in-process
/// fallback).
pub fn clipboard_copy(text: &str) -> Result<(), String> {
    if let Ok(mut guard) = FALLBACK_CLIPBOARD
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
    {
        *guard = text.to_owned();
    }
    #[cfg(target_os = "macos")]
    {
        use std::io::Write;
        use std::process::{Command as ProcessCommand, Stdio};
        let mut child = ProcessCommand::new("pbcopy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not launch pbcopy: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| format!("could not write to pbcopy: {e}"))?;
        }
        let status = child
            .wait()
            .map_err(|e| format!("pbcopy wait failed: {e}"))?;
        if !status.success() {
            return Err(format!("pbcopy exited with {status}"));
        }
    }
    Ok(())
}

/// Reads text from the system clipboard (macOS `pbpaste`, falling back to the
/// process-local clipboard buffer).
pub fn clipboard_paste() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command as ProcessCommand;
        if let Ok(output) = ProcessCommand::new("pbpaste").output()
            && output.status.success()
            && let Ok(text) = String::from_utf8(output.stdout)
            && !text.is_empty()
        {
            return Some(text);
        }
    }
    FALLBACK_CLIPBOARD
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .filter(|s| !s.is_empty())
}

/// Distinct RGB badge/plumbob accent color for `player_id`.
pub const fn player_badge_color(player_id: PlayerId) -> [u8; 3] {
    const PALETTE: [[u8; 3]; 8] = [
        [80, 232, 112],  // P1 (Host): Sims emerald green
        [64, 212, 255],  // P2: bright cyan
        [255, 178, 56],  // P3: warm amber
        [236, 108, 255], // P4: vivid orchid
        [255, 112, 128], // P5: coral rose
        [168, 240, 88],  // P6: lime
        [136, 160, 255], // P7: periwinkle
        [255, 230, 102], // P8: sunlit yellow
    ];
    let idx = (player_id.0.saturating_sub(1) as usize) % PALETTE.len();
    PALETTE[idx]
}

/// Predicate selecting simulation-relevant game files and custom content for
/// [`FileManifest::from_directory`] so mismatched installs or `Downloads/` CC
/// are rejected cleanly on join without hashing pure audio directories.
pub fn is_coop_manifest_file(rel: &str) -> bool {
    let ext_ok = rel.ends_with(".iff")
        || rel.ends_with(".far")
        || rel.ends_with(".flr")
        || rel.ends_with(".wll");
    if !ext_ok {
        return false;
    }
    if rel.starts_with("downloads/") || rel.starts_with("templatenpcs/") {
        return true;
    }
    if rel.starts_with("gamedata/") {
        return rel.starts_with("gamedata/objects/")
            || rel.starts_with("gamedata/userobjects/")
            || rel.starts_with("gamedata/global/")
            || rel.starts_with("gamedata/globals/")
            || rel.starts_with("gamedata/behavior/")
            || rel.starts_with("gamedata/animation/")
            || rel.starts_with("gamedata/floors/")
            || rel.starts_with("gamedata/walls/")
            || rel == "gamedata/build.iff"
            || rel == "gamedata/work.iff"
            || rel == "gamedata/fame.iff"
            || rel == "gamedata/tuning.iff"
            || rel == "gamedata/social.iff";
    }
    if (rel.starts_with("expansionpack")
        || rel.starts_with("expansionshared/")
        || rel.starts_with("deluxe/"))
        && !rel.contains("/sound/")
        && !rel.contains("/music/")
    {
        return true;
    }
    false
}

/// Deterministically builds the game-file and custom-content [`FileManifest`]
/// for `install_root`.
pub fn build_install_manifest(install_root: &Path) -> Result<FileManifest, ManifestError> {
    FileManifest::from_directory(install_root, is_coop_manifest_file)
}

/// Cooperatively cancellable manifest scan for background startup tasks: once
/// `cancelled` is set, remaining files are skipped immediately without reading
/// or hashing them, and `Err("co-op startup cancelled")` is returned.
fn build_install_manifest_cancellable(
    install_root: &Path,
    cancelled: &AtomicBool,
) -> Result<FileManifest, String> {
    let manifest = FileManifest::from_directory(install_root, |rel| {
        !cancelled.load(Ordering::Relaxed) && is_coop_manifest_file(rel)
    })
    .map_err(|e| format!("manifest scan failed: {e}"))?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("co-op startup cancelled".to_owned());
    }
    Ok(manifest)
}

/// Default local co-op display name derived from environment (`USER` / `USERNAME`)
/// or fallback `fallback`.
pub fn default_player_name(fallback: &str) -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty() && s.len() <= 32)
        .unwrap_or_else(|| fallback.to_owned())
}

/// Builds a dedicated background Tokio runtime for `iroh` P2P networking.
fn build_iroh_runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("sims-coop-iroh")
        .build()
        .map_err(|e| format!("could not start co-op async runtime: {e}"))
}

/// Owns a bound [`IrohTransport`] alongside its dedicated background Tokio
/// runtime so the synchronous `winit` loop never blocks on async I/O and
/// teardown closes the `iroh::Endpoint` cleanly with zero leaked threads.
pub struct IrohBridge {
    transport: Option<IrohTransport>,
    runtime: Option<tokio::runtime::Runtime>,
    local_addr: PeerAddress,
    invite_ticket: String,
}

impl IrohBridge {
    /// Wraps an already-bound [`IrohTransport`] and its dedicated Tokio runtime.
    pub fn from_parts(
        transport: IrohTransport,
        runtime: tokio::runtime::Runtime,
    ) -> Result<Self, String> {
        let local_addr = transport.local_address().clone();
        let invite_ticket = transport
            .invite_ticket()
            .encode_string()
            .map_err(|e| format!("could not encode invite ticket: {e}"))?;
        Ok(Self {
            transport: Some(transport),
            runtime: Some(runtime),
            local_addr,
            invite_ticket,
        })
    }

    /// Synchronously binds a new [`IrohBridge`] on its own dedicated Tokio
    /// runtime (intended for loopback tests and CLI bootstrapping).
    pub fn bind_blocking(config: IrohConfig) -> Result<Self, String> {
        let runtime = build_iroh_runtime()?;
        let transport = runtime
            .block_on(IrohTransport::bind(config))
            .map_err(|e| format!("could not bind iroh endpoint: {e}"))?;
        Self::from_parts(transport, runtime)
    }

    /// Returns this endpoint's shareable `simscoop1...` invite ticket string.
    pub fn invite_ticket_string(&self) -> &str {
        &self.invite_ticket
    }

    /// Gracefully closes all peer connections, closes the `iroh::Endpoint`,
    /// and shuts down the background Tokio runtime cleanly.
    pub fn shutdown(&mut self) {
        if let (Some(transport), Some(runtime)) = (self.transport.take(), self.runtime.take()) {
            runtime.block_on(async {
                tokio::time::sleep(GOODBYE_DRAIN_WINDOW).await;
                let _ = tokio::time::timeout(IROH_CLOSE_TIMEOUT, transport.close()).await;
            });
            runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
        } else if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
        }
    }

    /// Moves endpoint drain + close and runtime shutdown onto a short-lived
    /// background thread so interactive UI Cancel/Leave never blocks the
    /// `winit` event loop while still letting `Goodbye` frames drain cleanly.
    pub fn shutdown_background(&mut self) {
        let transport = self.transport.take();
        let runtime = self.runtime.take();
        if transport.is_none() && runtime.is_none() {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("sims-coop-close".into())
            .spawn(move || match (transport, runtime) {
                (Some(transport), Some(runtime)) => {
                    runtime.block_on(async {
                        tokio::time::sleep(GOODBYE_DRAIN_WINDOW).await;
                        let _ = tokio::time::timeout(IROH_CLOSE_TIMEOUT, transport.close()).await;
                    });
                    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
                }
                (None, Some(runtime)) => {
                    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
                }
                _ => {}
            });
        if spawned.is_err() {
            // Fall back cleanly if OS thread spawn fails.
        }
    }
}

impl Drop for IrohBridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Transport for IrohBridge {
    fn local_address(&self) -> &PeerAddress {
        &self.local_addr
    }

    fn connect(&mut self, addr: &PeerAddress) -> Result<(), TransportError> {
        self.transport
            .as_mut()
            .ok_or(TransportError::Closed)?
            .connect(addr)
    }

    fn disconnect(&mut self, peer: &PeerAddress) -> Result<(), TransportError> {
        self.transport
            .as_mut()
            .ok_or(TransportError::Closed)?
            .disconnect(peer)
    }

    fn send(&mut self, to: &PeerAddress, message: WireMessage) -> Result<(), TransportError> {
        self.transport
            .as_mut()
            .ok_or(TransportError::Closed)?
            .send(to, message)
    }

    fn drain_events(&mut self) -> Vec<TransportEvent> {
        self.transport
            .as_mut()
            .map_or_else(Vec::new, Transport::drain_events)
    }

    fn connected_peers(&self) -> Vec<PeerAddress> {
        self.transport
            .as_ref()
            .map_or_else(Vec::new, Transport::connected_peers)
    }
}

/// Thread-local handle wrapping a deterministic [`MemoryNodeTransport`] so
/// [`CoopTransport`], [`CoopController`], [`crate::Game`], and
/// [`crate::neighborhood::NeighborhoodShell`] remain `Send` (required by the
/// background startup loader in `main.rs`) with zero `unsafe` code.
pub struct MemoryTransportHandle {
    id: u64,
    local_addr: PeerAddress,
}

static NEXT_MEMORY_TRANSPORT_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

thread_local! {
    static MEMORY_TRANSPORTS: std::cell::RefCell<BTreeMap<u64, MemoryNodeTransport>> =
        const { std::cell::RefCell::new(BTreeMap::new()) };
}

impl MemoryTransportHandle {
    /// Registers `transport` on the current thread and returns a `Send` handle.
    pub fn new(transport: MemoryNodeTransport) -> Self {
        let local_addr = transport.local_address().clone();
        let id = NEXT_MEMORY_TRANSPORT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        MEMORY_TRANSPORTS.with(|slot| {
            slot.borrow_mut().insert(id, transport);
        });
        Self { id, local_addr }
    }

    fn with_mut<T>(
        &self,
        f: impl FnOnce(&mut MemoryNodeTransport) -> T,
    ) -> Result<T, TransportError> {
        MEMORY_TRANSPORTS.with(|slot| {
            let mut map = slot.borrow_mut();
            let transport = map.get_mut(&self.id).ok_or(TransportError::Closed)?;
            Ok(f(transport))
        })
    }

    fn with_ref<T>(&self, f: impl FnOnce(&MemoryNodeTransport) -> T) -> Option<T> {
        MEMORY_TRANSPORTS.with(|slot| {
            let map = slot.borrow();
            map.get(&self.id).map(f)
        })
    }
}

impl Drop for MemoryTransportHandle {
    fn drop(&mut self) {
        let _ = MEMORY_TRANSPORTS.try_with(|slot| {
            slot.borrow_mut().remove(&self.id);
        });
    }
}

impl Transport for MemoryTransportHandle {
    fn local_address(&self) -> &PeerAddress {
        &self.local_addr
    }

    fn connect(&mut self, addr: &PeerAddress) -> Result<(), TransportError> {
        self.with_mut(|t| t.connect(addr))?
    }

    fn disconnect(&mut self, peer: &PeerAddress) -> Result<(), TransportError> {
        self.with_mut(|t| t.disconnect(peer))?
    }

    fn send(&mut self, to: &PeerAddress, message: WireMessage) -> Result<(), TransportError> {
        self.with_mut(|t| t.send(to, message))?
    }

    fn drain_events(&mut self) -> Vec<TransportEvent> {
        self.with_mut(Transport::drain_events).unwrap_or_default()
    }

    fn connected_peers(&self) -> Vec<PeerAddress> {
        self.with_ref(Transport::connected_peers)
            .unwrap_or_default()
    }
}

/// Unified non-blocking transport enum driven identically by [`CoopController`]
/// for both deterministic [`MemoryMesh`](sims_net::MemoryMesh) tests and live
/// [`IrohTransport`] P2P / loopback sessions.
pub enum CoopTransport {
    Memory(MemoryTransportHandle),
    Iroh(IrohBridge),
}

impl CoopTransport {
    /// Wraps a deterministic [`MemoryNodeTransport`] for headless tests.
    pub fn memory(transport: MemoryNodeTransport) -> Self {
        Self::Memory(MemoryTransportHandle::new(transport))
    }

    /// Returns the shareable `simscoop1...` invite ticket if backed by `iroh`.
    pub fn invite_ticket_string(&self) -> Option<String> {
        match self {
            Self::Memory(_) => None,
            Self::Iroh(bridge) => Some(bridge.invite_ticket_string().to_owned()),
        }
    }

    /// Cleanly shuts down the underlying transport (closing `iroh` endpoint and
    /// joining/stopping its worker runtime if present).
    pub fn shutdown(&mut self) {
        if let Self::Iroh(bridge) = self {
            bridge.shutdown();
        }
    }

    /// Non-blocking interactive teardown of the underlying transport so the UI
    /// thread never stalls on `Goodbye` drain or Tokio worker shutdown.
    pub fn shutdown_background(&mut self) {
        if let Self::Iroh(bridge) = self {
            bridge.shutdown_background();
        }
    }
}

impl From<MemoryNodeTransport> for CoopTransport {
    fn from(transport: MemoryNodeTransport) -> Self {
        Self::memory(transport)
    }
}

impl From<IrohBridge> for CoopTransport {
    fn from(bridge: IrohBridge) -> Self {
        Self::Iroh(bridge)
    }
}

impl Transport for CoopTransport {
    fn local_address(&self) -> &PeerAddress {
        match self {
            Self::Memory(t) => t.local_address(),
            Self::Iroh(t) => t.local_address(),
        }
    }

    fn connect(&mut self, addr: &PeerAddress) -> Result<(), TransportError> {
        match self {
            Self::Memory(t) => t.connect(addr),
            Self::Iroh(t) => t.connect(addr),
        }
    }

    fn disconnect(&mut self, peer: &PeerAddress) -> Result<(), TransportError> {
        match self {
            Self::Memory(t) => t.disconnect(peer),
            Self::Iroh(t) => t.disconnect(peer),
        }
    }

    fn send(&mut self, to: &PeerAddress, message: WireMessage) -> Result<(), TransportError> {
        match self {
            Self::Memory(t) => t.send(to, message),
            Self::Iroh(t) => t.send(to, message),
        }
    }

    fn drain_events(&mut self) -> Vec<TransportEvent> {
        match self {
            Self::Memory(t) => t.drain_events(),
            Self::Iroh(t) => t.drain_events(),
        }
    }

    fn connected_peers(&self) -> Vec<PeerAddress> {
        match self {
            Self::Memory(t) => t.connected_peers(),
            Self::Iroh(t) => t.connected_peers(),
        }
    }
}

/// Configuration options for starting a co-op Host or Join session.
#[derive(Debug, Clone)]
pub struct CoopStartOptions {
    /// Local player display name shown in the lobby and HUD overlay.
    pub player_name: String,
    /// `iroh` binding mode (`P2pDefault` for internet P2P, `Loopback` for local tests).
    pub iroh_config: IrohConfig,
    /// Lockstep session configuration (used when hosting).
    pub session_config: SessionConfig,
    /// Optional explicit manifest digest (skips scanning `install_root` when set).
    pub manifest_override: Option<ManifestDigest>,
    /// When `true` (default for live UI hosting), schedules and executes an
    /// immediate lockstep prime tick on host start so every joining peer always
    /// receives a full `"SCOP"` v2 snapshot even if the host started at tick 0.
    pub prime_tick_on_start: bool,
}

impl Default for CoopStartOptions {
    fn default() -> Self {
        Self {
            player_name: default_player_name("Host"),
            iroh_config: IrohConfig::P2pDefault,
            session_config: SessionConfig::default(),
            manifest_override: None,
            prime_tick_on_start: true,
        }
    }
}

/// Summary of events produced by one [`CoopController::poll_and_handle`] pass.
#[derive(Debug, Default)]
pub struct CoopPollOutput {
    /// Visual or session state changed and the frame should redraw.
    pub redraw: bool,
    /// A `"SCOP"` v2 snapshot was verified and restored onto the lot.
    pub snapshot_restored: bool,
    /// Human-readable status note for the top status band, if any.
    pub note: Option<String>,
    /// Number of drift events observed during this poll.
    pub drift_events: usize,
    /// Whether the session ended or was rejected and has detached.
    pub ended: bool,
    /// Set when the host's verified `"SCOP"` v2 snapshot belongs to a different
    /// `(house, family)` than the currently booted guest lot so the shell can
    /// stage the matching house before completing verified restore & ACK.
    pub needs_lot_restage: Option<(u32, u32)>,
}

/// Outcome of attempting to restore a parked host-lot snapshot onto a candidate
/// guest `Lot` during [`CoopController::try_complete_pending_restage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestageAttemptError {
    /// This candidate lot's `(house, family)` or load fingerprint did not match
    /// (`SaveError::OtherHouse` / `SaveError::OtherLoad`). The snapshot remains
    /// parked so the caller can try another local candidate (e.g. base
    /// `HouseNN.iff`) before disconnecting.
    Mismatch(String),
    /// Non-recoverable verification or confirmation failure; the controller has
    /// already detached and recorded the terminal error state.
    Fatal(String),
}

/// Snapshot parked while the guest shell re-stages the host's matching base
/// lot `(host_house, host_family)` prior to verified restore and ACK.
struct PendingHostLotRestage {
    snapshot_id: sims_net::SnapshotId,
    at_tick: Tick,
    state_fingerprint: sims_net::StateFingerprint,
    redb_bytes: Vec<u8>,
    host_house: u32,
    host_family: u32,
    local_house: u32,
    local_family: u32,
}

/// Summary of a deterministic [`crate::Game::coop_pump`] call in headless tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CoopPumpStats {
    pub host_scheduled: bool,
    pub batches_applied: u32,
    pub sim_ticks_stepped: u32,
    pub snapshots_restored: u32,
    pub drift_events: u32,
}

type AsyncBindOutcome = Result<(IrohTransport, ManifestDigest), String>;

struct PendingIrohStart {
    is_host: bool,
    ticket: Option<String>,
    options: CoopStartOptions,
    runtime: Option<tokio::runtime::Runtime>,
    abort_handle: tokio::task::AbortHandle,
    cancelled: Arc<AtomicBool>,
    rx: mpsc::Receiver<AsyncBindOutcome>,
}

impl PendingIrohStart {
    fn cancel_blocking(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.abort_handle.abort();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
        }
    }

    fn cancel_background(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.abort_handle.abort();
        if let Some(runtime) = self.runtime.take() {
            let _ = std::thread::Builder::new()
                .name("sims-coop-cancel".into())
                .spawn(move || {
                    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
                });
        }
    }
}

impl Drop for PendingIrohStart {
    fn drop(&mut self) {
        self.cancel_blocking();
    }
}

struct ActiveCoop {
    node: LockstepNode,
    transport: CoopTransport,
    invite_ticket: Option<String>,
    connection: ConnectionState,
    resync_reason: Option<String>,
    resync_progress_ticks: u64,
    time_toast: Option<TimeAttribution>,
    /// Accumulated wall-clock microseconds toward the next host lockstep tick.
    host_accum_us: u64,
    /// Number of drift events detected during this session.
    drift_count: u64,
    /// Number of snapshots restored during this session.
    snapshot_restore_count: u64,
    /// Recorded player names so leave/failover messages still show names.
    known_names: BTreeMap<PlayerId, String>,
    /// Snapshot awaiting guest lot re-staging to `(host_house, host_family)`.
    pending_restage: Option<PendingHostLotRestage>,
    /// The report of the host's prime tick, for the `Game` to take
    /// ([`CoopController::take_prime_report`]): what it showed (a non-modal
    /// dialog's text, read from that tick) and its sounds.
    prime_report: Option<Box<CoopBatchReport>>,
}

impl ActiveCoop {
    fn player_name(&self, id: PlayerId) -> String {
        self.node
            .session()
            .players
            .get(&id)
            .map(|p| p.name.clone())
            .or_else(|| self.known_names.get(&id).cloned())
            .unwrap_or_else(|| format!("{id}"))
    }

    fn refresh_known_names(&mut self) {
        for (&id, player) in &self.node.session().players {
            self.known_names.insert(id, player.name.clone());
        }
    }
}

enum CoopPhase {
    Idle,
    Starting(PendingIrohStart),
    Active(Box<ActiveCoop>),
    Failed(ConnectionState),
}

/// Live co-op multiplayer session controller owned by [`crate::Game`].
pub struct CoopController {
    phase: CoopPhase,
}

impl Default for CoopController {
    fn default() -> Self {
        Self::new()
    }
}

impl CoopController {
    /// Creates an idle (offline) co-op controller.
    pub fn new() -> Self {
        Self {
            phase: CoopPhase::Idle,
        }
    }

    /// The report of the prime tick the host ran as it started, once: the
    /// `Game` applies its side effects as for any batch.
    pub fn take_prime_report(&mut self) -> Option<CoopBatchReport> {
        match &mut self.phase {
            CoopPhase::Active(active) => active.prime_report.take().map(|r| *r),
            _ => None,
        }
    }

    /// Whether an async `iroh` startup task is currently in flight.
    pub fn is_starting(&self) -> bool {
        matches!(self.phase, CoopPhase::Starting(_))
    }

    /// Returns whether a co-op session is actively attached or starting up.
    pub fn is_active_or_starting(&self) -> bool {
        matches!(self.phase, CoopPhase::Starting(_) | CoopPhase::Active(_))
    }

    /// Returns whether the local node is currently in an active joined/hosting
    /// lockstep session.
    pub fn is_active(&self) -> bool {
        matches!(self.phase, CoopPhase::Active(_))
    }

    /// Returns whether the local node is currently the authoritative Host.
    pub fn is_host(&self) -> bool {
        matches!(
            &self.phase,
            CoopPhase::Active(active) if active.node.role() == PlayerRole::Host
        )
    }

    /// Returns whether the local node is currently a joined/connecting Peer.
    pub fn is_peer(&self) -> bool {
        matches!(
            &self.phase,
            CoopPhase::Active(active) if active.node.role() == PlayerRole::Peer
        )
    }

    /// Returns the local assigned [`PlayerId`], once joined or hosting.
    pub fn local_player_id(&self) -> Option<PlayerId> {
        match &self.phase {
            CoopPhase::Active(active) => active.node.local_player_id(),
            _ => None,
        }
    }

    /// Returns the local player's currently selected Sim in the deterministic
    /// session state.
    pub fn local_selected_sim(&self) -> Option<u16> {
        let CoopPhase::Active(active) = &self.phase else {
            return None;
        };
        let pid = active.node.local_player_id()?;
        active
            .node
            .session()
            .players
            .get(&pid)
            .and_then(|p| p.selected_sim)
    }

    /// Returns the shareable `simscoop1...` invite ticket when hosting over `iroh`.
    pub fn invite_ticket(&self) -> Option<&str> {
        match &self.phase {
            CoopPhase::Active(active) => active.invite_ticket.as_deref(),
            _ => None,
        }
    }

    /// Returns the current deterministic [`sims_net::SessionState`] when active.
    pub fn session(&self) -> Option<&sims_net::SessionState> {
        match &self.phase {
            CoopPhase::Active(active) => Some(active.node.session()),
            _ => None,
        }
    }

    /// Returns the active [`LockstepNode`] for read-only inspection (e.g. tests).
    pub fn node(&self) -> Option<&LockstepNode> {
        match &self.phase {
            CoopPhase::Active(active) => Some(&active.node),
            _ => None,
        }
    }

    /// Total number of drift events detected in the active session.
    pub fn drift_count(&self) -> u64 {
        match &self.phase {
            CoopPhase::Active(active) => active.drift_count,
            _ => 0,
        }
    }

    /// Total number of `"SCOP"` v2 snapshots verified & restored in the active session.
    pub fn snapshot_restore_count(&self) -> u64 {
        match &self.phase {
            CoopPhase::Active(active) => active.snapshot_restore_count,
            _ => 0,
        }
    }

    /// Current UI [`ConnectionState`].
    pub fn connection_state(&self) -> ConnectionState {
        match &self.phase {
            CoopPhase::Idle => ConnectionState::Disconnected,
            CoopPhase::Starting(_) => ConnectionState::Connecting,
            CoopPhase::Active(active) => active.connection.clone(),
            CoopPhase::Failed(state) => state.clone(),
        }
    }

    /// Records a non-destructive pre-flight error (such as refusing a tutorial
    /// lot or travel destination) in the lobby UI state.
    pub fn set_error(&mut self, message: impl Into<String>) {
        if !self.is_active_or_starting() {
            self.phase = CoopPhase::Failed(ConnectionState::Error(message.into()));
        }
    }

    /// Starts hosting over real `iroh` asynchronously on a background Tokio
    /// runtime (zero blocking on the `winit` UI thread).
    pub fn start_host_iroh_async(
        &mut self,
        install_root: PathBuf,
        options: CoopStartOptions,
    ) -> Result<(), String> {
        if self.is_active_or_starting() {
            return Err("a co-op session is already active or starting".into());
        }
        options
            .session_config
            .validate()
            .map_err(|e| format!("invalid co-op config: {e}"))?;
        let runtime = build_iroh_runtime()?;
        let (tx, rx) = mpsc::channel();
        let iroh_config = options.iroh_config;
        let manifest_override = options.manifest_override;
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_bg = Arc::clone(&cancelled);
        let cancelled_task = Arc::clone(&cancelled);
        let handle = runtime.spawn(async move {
            let outcome = async {
                let manifest = match manifest_override {
                    Some(digest) => digest,
                    None => tokio::task::spawn_blocking(move || {
                        build_install_manifest_cancellable(&install_root, &cancelled_bg)
                            .map(|m| m.digest())
                    })
                    .await
                    .map_err(|e| format!("manifest task failed: {e}"))??,
                };
                if cancelled_task.load(Ordering::Relaxed) {
                    return Err("co-op startup cancelled".to_owned());
                }
                let transport = IrohTransport::bind(iroh_config)
                    .await
                    .map_err(|e| format!("could not bind iroh endpoint: {e}"))?;
                Ok((transport, manifest))
            }
            .await;
            let _ = tx.send(outcome);
        });
        self.phase = CoopPhase::Starting(PendingIrohStart {
            is_host: true,
            ticket: None,
            options,
            runtime: Some(runtime),
            abort_handle: handle.abort_handle(),
            cancelled: Arc::clone(&cancelled),
            rx,
        });
        Ok(())
    }

    /// Validates `ticket` synchronously and starts joining a host over real
    /// `iroh` asynchronously on a background Tokio runtime (zero blocking on
    /// the UI thread).
    pub fn start_join_iroh_async(
        &mut self,
        install_root: PathBuf,
        ticket: String,
        options: CoopStartOptions,
    ) -> Result<(), String> {
        if self.is_active_or_starting() {
            return Err("a co-op session is already active or starting".into());
        }
        let trimmed = ticket.trim().to_owned();
        if let Err(err) = InviteTicket::decode_string(&trimmed) {
            let msg = format!("invalid invite ticket: {err}");
            self.phase = CoopPhase::Failed(ConnectionState::Error(msg.clone()));
            return Err(msg);
        }
        let runtime = build_iroh_runtime()?;
        let (tx, rx) = mpsc::channel();
        let iroh_config = options.iroh_config;
        let manifest_override = options.manifest_override;
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_bg = Arc::clone(&cancelled);
        let cancelled_task = Arc::clone(&cancelled);
        let handle = runtime.spawn(async move {
            let outcome = async {
                let manifest = match manifest_override {
                    Some(digest) => digest,
                    None => tokio::task::spawn_blocking(move || {
                        build_install_manifest_cancellable(&install_root, &cancelled_bg)
                            .map(|m| m.digest())
                    })
                    .await
                    .map_err(|e| format!("manifest task failed: {e}"))??,
                };
                if cancelled_task.load(Ordering::Relaxed) {
                    return Err("co-op startup cancelled".to_owned());
                }
                let transport = IrohTransport::bind(iroh_config)
                    .await
                    .map_err(|e| format!("could not bind iroh endpoint: {e}"))?;
                Ok((transport, manifest))
            }
            .await;
            let _ = tx.send(outcome);
        });
        self.phase = CoopPhase::Starting(PendingIrohStart {
            is_host: false,
            ticket: Some(trimmed),
            options,
            runtime: Some(runtime),
            abort_handle: handle.abort_handle(),
            cancelled: Arc::clone(&cancelled),
            rx,
        });
        Ok(())
    }

    /// Synchronously starts hosting on `lot` using an already-constructed
    /// [`CoopTransport`] (`MemoryMesh` or pre-bound [`IrohBridge`]).
    #[allow(clippy::too_many_arguments)]
    pub fn start_host_with_transport(
        &mut self,
        lot: &mut Lot,
        install: &Install,
        play_mode: bool,
        frames: Frames,
        transport: CoopTransport,
        manifest: ManifestDigest,
        options: CoopStartOptions,
    ) -> Result<(), String> {
        if self.is_active_or_starting() {
            return Err("a co-op session is already active or starting".into());
        }
        options
            .session_config
            .validate()
            .map_err(|e| format!("invalid co-op config: {e}"))?;
        let invite_ticket = transport.invite_ticket_string();
        let endpoint_hint = invite_ticket
            .clone()
            .or_else(|| Some(transport.local_address().0.clone()));
        let mut node = LockstepNode::new_host(
            options.player_name.clone(),
            endpoint_hint,
            manifest,
            options.session_config.clone(),
        );
        lot.sync_initial_coop_session(node.session_mut());
        lot.attach_coop_with_install(node.session().clone(), install)
            .map_err(|e| format!("could not attach co-op VM: {e}"))?;
        if let Some(ctx) = lot.coop_mut() {
            ctx.play_mode = play_mode;
            ctx.frames = frames;
        }
        let mut active = ActiveCoop {
            node,
            transport,
            invite_ticket: invite_ticket.clone(),
            connection: ConnectionState::Hosting {
                invite_ticket: invite_ticket.unwrap_or_else(|| "local-memory-session".to_owned()),
            },
            resync_reason: None,
            resync_progress_ticks: 0,
            time_toast: None,
            host_accum_us: 0,
            drift_count: 0,
            snapshot_restore_count: 0,
            known_names: BTreeMap::new(),
            pending_restage: None,
            prime_report: None,
        };
        active.refresh_known_names();
        if options.prime_tick_on_start && active.node.session().current_tick == Tick::ZERO {
            // Prime tick 1 so `current_tick >= 1` and every subsequent joiner
            // receives a verified `"SCOP"` v2 snapshot immediately on `Welcome`.
            let _ = active.node.advance_host_tick(&mut active.transport);
            if let Ok(Some(applied)) = active.node.step_next_tick(true) {
                let report = lot
                    .apply_coop_batch(&applied, active.node.session_mut())
                    .map_err(|e| format!("co-op prime tick failed: {e}"))?;
                let _ = active.node.report_local_tick(
                    &mut active.transport,
                    report.tick,
                    0,
                    Some(report.world_hash),
                );
                active.prime_report = Some(Box::new(report));
            }
        }
        self.phase = CoopPhase::Active(Box::new(active));
        Ok(())
    }

    /// Synchronously begins joining a host at `host_addr` using an
    /// already-constructed [`CoopTransport`] (`MemoryMesh` or pre-bound [`IrohBridge`]).
    pub fn start_join_with_transport(
        &mut self,
        mut transport: CoopTransport,
        host_addr: PeerAddress,
        manifest: ManifestDigest,
        player_name: impl Into<String>,
    ) -> Result<(), String> {
        if self.is_active_or_starting() {
            return Err("a co-op session is already active or starting".into());
        }
        let local_hint = transport
            .invite_ticket_string()
            .or_else(|| Some(transport.local_address().0.clone()));
        transport
            .connect(&host_addr)
            .map_err(|e| format!("could not connect to host: {e}"))?;
        let node = LockstepNode::new_peer(player_name, local_hint, manifest, host_addr);
        let mut active = ActiveCoop {
            node,
            transport,
            invite_ticket: None,
            connection: ConnectionState::Connecting,
            resync_reason: Some("Connecting to host...".to_owned()),
            resync_progress_ticks: 0,
            time_toast: None,
            host_accum_us: 0,
            drift_count: 0,
            snapshot_restore_count: 0,
            known_names: BTreeMap::new(),
            pending_restage: None,
            prime_report: None,
        };
        active.refresh_known_names();
        self.phase = CoopPhase::Active(Box::new(active));
        Ok(())
    }

    /// Submits a local [`sims_net::Command`] through [`LockstepNode`].
    pub fn submit_command(&mut self, command: Command) -> Result<(), String> {
        let CoopPhase::Active(active) = &mut self.phase else {
            return Err("not in an active co-op session".into());
        };
        active
            .node
            .submit_command(&mut active.transport, command)
            .map_err(|e| format!("could not submit co-op command: {e}"))
    }

    /// Polls pending async startup, network transport events, and high-level
    /// [`NodeEvent`]s (handling `"SCOP"` v2 snapshot serving & verified
    /// restoration, manifest rejection, drift detection, and host failover).
    pub fn poll_and_handle(
        &mut self,
        lot: &mut Lot,
        install: &Install,
        play_mode: bool,
        frames: Frames,
    ) -> CoopPollOutput {
        let mut out = CoopPollOutput::default();

        // 1. Check non-blocking async Iroh startup completion.
        let completed_start = if let CoopPhase::Starting(pending) = &mut self.phase {
            match pending.rx.try_recv() {
                Ok(res) => Some(res),
                Err(mpsc::TryRecvError::Empty) => return out,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("co-op startup task terminated unexpectedly".to_owned()))
                }
            }
        } else {
            None
        };

        if let Some(res) = completed_start {
            let CoopPhase::Starting(mut pending) =
                std::mem::replace(&mut self.phase, CoopPhase::Idle)
            else {
                unreachable!();
            };
            let runtime = pending.runtime.take();
            match (res, runtime) {
                (Ok((transport, manifest)), Some(runtime)) => {
                    match IrohBridge::from_parts(transport, runtime) {
                        Ok(bridge) => {
                            let start_res = if pending.is_host {
                                self.start_host_with_transport(
                                    lot,
                                    install,
                                    play_mode,
                                    frames.clone(),
                                    CoopTransport::Iroh(bridge),
                                    manifest,
                                    pending.options.clone(),
                                )
                            } else {
                                let ticket = pending.ticket.clone().unwrap_or_default();
                                self.start_join_with_transport(
                                    CoopTransport::Iroh(bridge),
                                    PeerAddress::new(ticket),
                                    manifest,
                                    pending.options.player_name.clone(),
                                )
                            };
                            match start_res {
                                Ok(()) => {
                                    out.redraw = true;
                                    if pending.is_host
                                        && let Some(ticket) = self.invite_ticket()
                                    {
                                        out.note = Some(format!(
                                            "Hosting co-op session. Ticket: {}...",
                                            &ticket[..ticket.len().min(28)]
                                        ));
                                    }
                                }
                                Err(err) => {
                                    self.phase =
                                        CoopPhase::Failed(ConnectionState::Error(err.clone()));
                                    out.redraw = true;
                                    out.note = Some(err);
                                    out.ended = true;
                                    return out;
                                }
                            }
                        }
                        Err(err) => {
                            self.phase = CoopPhase::Failed(ConnectionState::Error(err.clone()));
                            out.redraw = true;
                            out.note = Some(err);
                            out.ended = true;
                            return out;
                        }
                    }
                }
                (Err(err), Some(runtime)) => {
                    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
                    self.phase = CoopPhase::Failed(ConnectionState::Error(err.clone()));
                    out.redraw = true;
                    out.note = Some(err);
                    out.ended = true;
                    return out;
                }
                (_, None) => {
                    let err = "co-op async runtime missing".to_owned();
                    self.phase = CoopPhase::Failed(ConnectionState::Error(err.clone()));
                    out.redraw = true;
                    out.note = Some(err);
                    out.ended = true;
                    return out;
                }
            }
        }

        // 2. Poll active LockstepNode + transport and drain NodeEvents.
        let mut disconnect_with_state: Option<ConnectionState> = None;
        if let CoopPhase::Active(active) = &mut self.phase {
            active.node.poll(&mut active.transport);
            if active.node.is_syncing() {
                active.resync_progress_ticks = active.resync_progress_ticks.saturating_add(1);
            }
            let events = active.node.drain_events();
            for event in events {
                out.redraw = true;
                match event {
                    NodeEvent::Joined {
                        player_id,
                        awaiting_snapshot,
                        ..
                    } => {
                        active.connection = ConnectionState::Connected;
                        active.refresh_known_names();
                        if awaiting_snapshot {
                            active.resync_reason =
                                Some("Receiving host lot snapshot...".to_owned());
                            active.resync_progress_ticks = 1;
                            out.note = Some(format!(
                                "Joined co-op as {player_id}; syncing lot snapshot..."
                            ));
                        } else {
                            active.resync_reason = None;
                            active.resync_progress_ticks = 0;
                            if lot.coop().is_none() {
                                if let Err(err) = lot.attach_coop_with_install(
                                    active.node.session().clone(),
                                    install,
                                ) {
                                    let msg = format!("Could not attach co-op VM on join: {err}");
                                    out.note = Some(msg.clone());
                                    disconnect_with_state = Some(ConnectionState::Error(msg));
                                    break;
                                }
                                if let Some(ctx) = lot.coop_mut() {
                                    ctx.play_mode = play_mode;
                                    ctx.frames = frames.clone();
                                }
                            }
                            out.note = Some(format!("Joined co-op session as {player_id}."));
                        }
                    }
                    NodeEvent::JoinRejected(reason) => {
                        let (conn_state, note) = match reason {
                            RejectReason::ManifestMismatch { expected, actual } => (
                                ConnectionState::ManifestMismatch {
                                    local_summary: format!("{actual}"),
                                    remote_summary: format!("{expected}"),
                                    mismatched_files: vec![
                                        "Game files or Downloads/ custom content differ from host"
                                            .to_owned(),
                                    ],
                                },
                                format!(
                                    "Join rejected: game files/custom content manifest mismatch (local {actual}, host {expected})."
                                ),
                            ),
                            RejectReason::ProtocolVersionMismatch { expected, actual } => {
                                let msg = sims_net::protocol_mismatch_message(expected, actual);
                                (ConnectionState::Error(msg.clone()), msg)
                            }
                            RejectReason::BuildMismatch { expected, actual } => {
                                let msg = sims_net::build_mismatch_message(expected, actual);
                                (ConnectionState::Error(msg.clone()), msg)
                            }
                            RejectReason::SessionFull { max_players } => {
                                let msg = format!("Session is full ({max_players} players max)");
                                (ConnectionState::Error(msg.clone()), msg)
                            }
                            RejectReason::NotHost { current_host } => {
                                let msg = format!(
                                    "Remote peer is not the session host (host is {current_host})"
                                );
                                (ConnectionState::Error(msg.clone()), msg)
                            }
                            RejectReason::InvalidPlayerName => {
                                let msg = "Invalid player display name".to_owned();
                                (ConnectionState::Error(msg.clone()), msg)
                            }
                            RejectReason::InvalidEndpointHint => {
                                let msg = "Peer endpoint identity verification failed".to_owned();
                                (ConnectionState::Error(msg.clone()), msg)
                            }
                            RejectReason::InvalidConfig => {
                                let msg = "Host sent an invalid session configuration".to_owned();
                                (ConnectionState::Error(msg.clone()), msg)
                            }
                        };
                        out.note = Some(note);
                        disconnect_with_state = Some(conn_state);
                        break;
                    }
                    NodeEvent::SnapshotNeeded {
                        player_id,
                        snapshot_id,
                        reason,
                        ..
                    } => match lot.coop_snapshot_bytes() {
                        Ok(redb_bytes) => {
                            let world_hash = lot.coop_world_hash();
                            if let Err(err) = active.node.send_snapshot(
                                &mut active.transport,
                                snapshot_id,
                                world_hash,
                                redb_bytes,
                                SnapshotCompression::Deflate,
                                DEFAULT_SNAPSHOT_CHUNK_BYTES,
                            ) {
                                active
                                    .node
                                    .fail_snapshot(&mut active.transport, snapshot_id);
                                out.note =
                                    Some(format!("Could not send snapshot to {player_id}: {err}"));
                            } else if reason == ResyncReason::DriftDetected {
                                out.note = Some(format!(
                                    "Sending drift-recovery snapshot to {player_id}..."
                                ));
                            }
                        }
                        Err(err) => {
                            active
                                .node
                                .fail_snapshot(&mut active.transport, snapshot_id);
                            out.note = Some(format!(
                                "Could not capture co-op snapshot for {player_id}: {err}"
                            ));
                        }
                    },
                    NodeEvent::ApplySnapshot {
                        snapshot_id,
                        at_tick,
                        state_fingerprint,
                        redb_bytes,
                    } => {
                        // Transactionally verify and restore the VM snapshot BEFORE
                        // confirming completion to the host so a failed or mismatched
                        // restore never sends `SnapshotAck(complete=true)`.
                        let Some(mut pending_session) =
                            active.node.pending_snapshot_session(snapshot_id).cloned()
                        else {
                            let msg = format!("Missing pending session for snapshot {snapshot_id}");
                            out.note = Some(msg.clone());
                            disconnect_with_state = Some(ConnectionState::Error(msg));
                            break;
                        };
                        pending_session.current_tick = at_tick;
                        match lot.restore_coop_snapshot_verified(
                            &redb_bytes,
                            pending_session,
                            state_fingerprint,
                        ) {
                            Ok(()) => {
                                if let Err(err) = active
                                    .node
                                    .confirm_snapshot_applied(&mut active.transport, snapshot_id)
                                {
                                    let msg =
                                        format!("Could not confirm snapshot {snapshot_id}: {err}");
                                    out.note = Some(msg.clone());
                                    disconnect_with_state = Some(ConnectionState::Error(msg));
                                    break;
                                }
                                active.pending_restage = None;
                                active.resync_reason = None;
                                active.resync_progress_ticks = 0;
                                active.snapshot_restore_count =
                                    active.snapshot_restore_count.saturating_add(1);
                                active.refresh_known_names();
                                out.snapshot_restored = true;
                                out.note = Some(format!(
                                    "Co-op lot synced at tick {} (verified).",
                                    at_tick.0
                                ));
                            }
                            Err(sims_vm::boot::save::SaveError::OtherHouse {
                                saved: (host_house, host_family),
                                loaded: (local_house, local_family),
                            }) => {
                                // Park snapshot for guest shell lot restaging without
                                // confirming or disconnecting yet.
                                active.resync_reason = Some(format!(
                                    "Loading host's Lot {host_house} before snapshot restore..."
                                ));
                                active.pending_restage = Some(PendingHostLotRestage {
                                    snapshot_id,
                                    at_tick,
                                    state_fingerprint,
                                    redb_bytes,
                                    host_house,
                                    host_family: u32::from(host_family),
                                    local_house,
                                    local_family: u32::from(local_family),
                                });
                                out.needs_lot_restage = Some((host_house, u32::from(host_family)));
                                out.note = Some(format!(
                                    "Host is playing Lot {host_house} (family {host_family}); staging matching lot..."
                                ));
                            }
                            Err(sims_vm::boot::save::SaveError::OtherLoad) => {
                                let msg = "Host's lot was loaded from a different neighborhood or \
                                           save layout. Open the matching neighborhood/lot and rejoin."
                                    .to_owned();
                                out.note = Some(msg.clone());
                                disconnect_with_state = Some(ConnectionState::Error(msg));
                                break;
                            }
                            Err(err) => {
                                let msg = format!("Snapshot verification/restore failed: {err}");
                                out.note = Some(msg.clone());
                                disconnect_with_state = Some(ConnectionState::Error(msg));
                                break;
                            }
                        }
                    }
                    NodeEvent::SnapshotFailed { snapshot_id, error } => {
                        active.resync_reason =
                            Some(format!("Retrying snapshot {snapshot_id} ({error})..."));
                        out.note = Some(format!("Snapshot {snapshot_id} failed: {error}"));
                    }
                    NodeEvent::DriftDetected {
                        player_id, tick, ..
                    } => {
                        active.drift_count = active.drift_count.saturating_add(1);
                        out.drift_events = out.drift_events.saturating_add(1);
                        if active.node.role() == PlayerRole::Peer {
                            active.resync_reason = Some(format!(
                                "Drift detected at tick {}; resyncing from host...",
                                tick.0
                            ));
                            active.resync_progress_ticks = 1;
                        }
                        out.note = Some(format!(
                            "Sync drift detected for {player_id} at tick {}; resyncing...",
                            tick.0
                        ));
                    }
                    NodeEvent::HostPromoted {
                        new_host,
                        new_epoch,
                    } => {
                        let ticket = active.transport.invite_ticket_string();
                        active.invite_ticket = ticket.clone();
                        active.connection = ConnectionState::Hosting {
                            invite_ticket: ticket
                                .unwrap_or_else(|| "local-memory-session".to_owned()),
                        };
                        active.refresh_known_names();
                        out.note = Some(format!(
                            "Previous host left; you ({new_host}) are now Host ({new_epoch})."
                        ));
                    }
                    NodeEvent::HostChanged {
                        new_host,
                        new_epoch,
                    } => {
                        active.refresh_known_names();
                        if Some(new_host) != active.node.local_player_id() {
                            active.invite_ticket = None;
                            active.connection = ConnectionState::Connected;
                            if active.node.is_syncing() {
                                active.resync_reason =
                                    Some("Resyncing with new co-op host...".to_owned());
                                active.resync_progress_ticks = 1;
                            } else {
                                active.resync_reason = None;
                                active.resync_progress_ticks = 0;
                            }
                        }
                        let name = active.player_name(new_host);
                        out.note = Some(format!(
                            "Host migrated to {name} ({new_host}, {new_epoch})."
                        ));
                    }
                    NodeEvent::PeerAccepted {
                        player_id, name, ..
                    } => {
                        active.known_names.insert(player_id, name.clone());
                        out.note = Some(format!("{name} ({player_id}) joined the lot."));
                    }
                    NodeEvent::PeerDisconnected {
                        player_id, reason, ..
                    } => {
                        let name = active.player_name(player_id);
                        let suffix = match reason {
                            LeaveReason::Voluntary => "left the session",
                            LeaveReason::Disconnected => "disconnected",
                            LeaveReason::HostDisconnected => "host disconnected",
                            LeaveReason::SessionClosed => "session closed",
                        };
                        out.note = Some(format!("{name} ({player_id}) {suffix}."));
                    }
                    NodeEvent::SessionEnded { reason } => {
                        let msg = format!("Co-op session ended: {reason}");
                        out.note = Some(msg.clone());
                        disconnect_with_state = Some(ConnectionState::Error(msg));
                        break;
                    }
                }
            }
        }

        if let Some(failed_state) = disconnect_with_state {
            self.shutdown_active(lot, false);
            self.phase = CoopPhase::Failed(failed_state);
            out.ended = true;
            out.redraw = true;
        }

        out
    }

    /// Host-only: schedules one lockstep [`TickBatch`](sims_net::TickBatch) if
    /// adaptive pacing allows it.
    pub fn advance_host_tick(&mut self) -> Result<Option<HostStepResult>, String> {
        let CoopPhase::Active(active) = &mut self.phase else {
            return Ok(None);
        };
        if active.node.role() != PlayerRole::Host {
            return Ok(None);
        }
        let res = active
            .node
            .advance_host_tick(&mut active.transport)
            .map_err(|e| format!("host tick scheduling failed: {e}"))?;
        Ok(Some(res))
    }

    /// Returns `(host_house, host_family)` if a verified snapshot is parked
    /// waiting for the guest shell to load the host's matching base house.
    pub fn pending_restage_target(&self) -> Option<(u32, u32)> {
        match &self.phase {
            CoopPhase::Active(active) => active
                .pending_restage
                .as_ref()
                .map(|p| (p.host_house, p.host_family)),
            _ => None,
        }
    }

    /// Attempts to complete a parked host-lot snapshot restore onto candidate
    /// `lot` without disconnecting on `OtherHouse` / `OtherLoad` so callers can
    /// try multiple local candidates (e.g. REDB store first, then installed
    /// `HouseNN.iff`) before failing and disconnecting.
    pub fn try_complete_pending_restage(
        &mut self,
        lot: &mut Lot,
    ) -> Result<String, RestageAttemptError> {
        let CoopPhase::Active(active) = &mut self.phase else {
            return Err(RestageAttemptError::Fatal(
                "not in an active co-op session".into(),
            ));
        };
        let Some(pending) = active.pending_restage.take() else {
            return Err(RestageAttemptError::Fatal(
                "no pending co-op snapshot restage".into(),
            ));
        };
        let Some(mut pending_session) = active
            .node
            .pending_snapshot_session(pending.snapshot_id)
            .cloned()
        else {
            let msg = format!(
                "Missing pending session for snapshot {}",
                pending.snapshot_id
            );
            self.shutdown_active(lot, false);
            self.phase = CoopPhase::Failed(ConnectionState::Error(msg.clone()));
            return Err(RestageAttemptError::Fatal(msg));
        };
        pending_session.current_tick = pending.at_tick;
        match lot.restore_coop_snapshot_verified(
            &pending.redb_bytes,
            pending_session,
            pending.state_fingerprint,
        ) {
            Ok(()) => {
                if let Err(err) = active
                    .node
                    .confirm_snapshot_applied(&mut active.transport, pending.snapshot_id)
                {
                    let msg = format!(
                        "Could not confirm restaged snapshot {}: {err}",
                        pending.snapshot_id
                    );
                    self.shutdown_active(lot, false);
                    self.phase = CoopPhase::Failed(ConnectionState::Error(msg.clone()));
                    return Err(RestageAttemptError::Fatal(msg));
                }
                active.resync_reason = None;
                active.resync_progress_ticks = 0;
                active.snapshot_restore_count = active.snapshot_restore_count.saturating_add(1);
                active.refresh_known_names();
                Ok(format!(
                    "Co-op Lot {} synced at tick {} (verified).",
                    pending.host_house, pending.at_tick.0
                ))
            }
            Err(sims_vm::boot::save::SaveError::OtherHouse { .. })
            | Err(sims_vm::boot::save::SaveError::OtherLoad) => {
                let msg = format!(
                    "Host is playing Lot {} (family {}), which does not match your local Lot {} \
                     (family {}) or neighborhood load. Load the matching lot and rejoin.",
                    pending.host_house,
                    pending.host_family,
                    pending.local_house,
                    pending.local_family
                );
                // Keep snapshot parked so caller can try another local candidate!
                active.pending_restage = Some(pending);
                Err(RestageAttemptError::Mismatch(msg))
            }
            Err(err) => {
                let msg = format!("Snapshot verification/restore failed: {err}");
                self.shutdown_active(lot, false);
                self.phase = CoopPhase::Failed(ConnectionState::Error(msg.clone()));
                Err(RestageAttemptError::Fatal(msg))
            }
        }
    }

    /// Completes a parked host-lot snapshot restore onto `lot` (single-shot
    /// wrapper that disconnects cleanly on any mismatch or failure).
    pub fn complete_pending_restage(&mut self, lot: &mut Lot) -> Result<String, String> {
        match self.try_complete_pending_restage(lot) {
            Ok(note) => Ok(note),
            Err(RestageAttemptError::Mismatch(msg)) => {
                self.fail_pending_restage(lot, msg.clone());
                Err(msg)
            }
            Err(RestageAttemptError::Fatal(msg)) => Err(msg),
        }
    }

    /// Aborts a parked host-lot restage cleanly with an actionable UI error.
    pub fn fail_pending_restage(&mut self, lot: &mut Lot, reason: String) {
        self.shutdown_active(lot, false);
        self.phase = CoopPhase::Failed(ConnectionState::Error(reason));
    }

    /// Host-only: accumulates `step_elapsed_us` toward owed lockstep ticks
    /// (capped at `max_ticks` periods). Pair with
    /// [`Self::schedule_one_host_tick_if_due`] interleaved 1-for-1 with
    /// [`Self::step_next_batch`] so the host never yields with scheduled-but-
    /// unapplied batches that could create a tick gap for a mid-game joiner.
    pub fn accumulate_host_elapsed(&mut self, step_elapsed_us: u64, max_ticks: u32) {
        let CoopPhase::Active(active) = &mut self.phase else {
            return;
        };
        if active.node.role() != PlayerRole::Host {
            return;
        }
        let period_us = Self::host_period_us(active);
        active.host_accum_us = active
            .host_accum_us
            .saturating_add(step_elapsed_us)
            .min(period_us.saturating_mul(u64::from(max_ticks.max(1))));
    }

    /// Host-only: schedules at most ONE owed lockstep batch if the accumulated
    /// elapsed time covers one tick period and pacing permits it.
    pub fn schedule_one_host_tick_if_due(&mut self) -> Result<bool, String> {
        let CoopPhase::Active(active) = &mut self.phase else {
            return Ok(false);
        };
        if active.node.role() != PlayerRole::Host {
            return Ok(false);
        }
        let period_us = Self::host_period_us(active);
        if active.host_accum_us < period_us {
            return Ok(false);
        }
        match active
            .node
            .advance_host_tick(&mut active.transport)
            .map_err(|e| format!("host tick scheduling failed: {e}"))?
        {
            HostStepResult::Scheduled(_) => {
                active.host_accum_us = active.host_accum_us.saturating_sub(period_us);
                Ok(true)
            }
            HostStepResult::WaitingForSlowestPeer { .. } => {
                active.host_accum_us = period_us;
                Ok(false)
            }
        }
    }

    fn host_period_us(active: &ActiveCoop) -> u64 {
        let period_ms = if active.node.session().is_paused() {
            SimSpeed::NORMAL.period_ms()
        } else {
            let speed_period = active.node.session().time.speed.period_ms();
            active.node.pacing_hint_ms().max(speed_period).max(1)
        };
        u64::from(period_ms).saturating_mul(1_000).max(1_000)
    }

    /// Advances host pacing by `step_elapsed_us` and schedules owed lockstep
    /// ticks (including command ticks while paused so unpause / Buy-Build /
    /// dialogs remain live).
    pub fn schedule_host_ticks_for_elapsed(
        &mut self,
        step_elapsed_us: u64,
        max_ticks: u32,
    ) -> Result<u32, String> {
        self.accumulate_host_elapsed(step_elapsed_us, max_ticks);
        let mut scheduled = 0u32;
        while scheduled < max_ticks && self.schedule_one_host_tick_if_due()? {
            scheduled += 1;
        }
        Ok(scheduled)
    }

    /// Executes the next ready lockstep batch on `lot`, reports the resulting
    /// [`Lot::coop_world_hash`] to [`LockstepNode::report_local_tick`], and
    /// updates speed/pause attribution toasts.
    pub fn step_next_batch(
        &mut self,
        lot: &mut Lot,
        flush: bool,
    ) -> Result<Option<CoopBatchReport>, String> {
        let CoopPhase::Active(active) = &mut self.phase else {
            return Ok(None);
        };
        if lot.coop().is_none() {
            return Ok(None);
        }
        let Some(applied) = active
            .node
            .step_next_tick(flush)
            .map_err(|e| format!("lockstep batch error: {e}"))?
        else {
            return Ok(None);
        };

        let started = Instant::now();
        let report = lot
            .apply_coop_batch(&applied, active.node.session_mut())
            .map_err(|e| format!("co-op VM batch error: {e}"))?;
        let step_us = started.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;

        active.refresh_known_names();
        // Record transient speed/pause attribution toasts for accepted commands.
        for (stamped, status) in &report.command_results {
            if *status != CoopCommandStatus::Applied {
                continue;
            }
            let player_name = active.player_name(stamped.issuer);
            let action = match stamped.command {
                Command::SetPaused { paused: true } => Some(TimeAction::Paused),
                Command::SetPaused { paused: false } => Some(TimeAction::Resumed),
                Command::SetSpeed { speed } => {
                    let v = speed.value();
                    if v >= SimSpeed::ULTRA.value() {
                        Some(TimeAction::UltraSpeed)
                    } else if v >= SimSpeed::FAST.value() {
                        Some(TimeAction::FastSpeed)
                    } else {
                        Some(TimeAction::NormalSpeed)
                    }
                }
                _ => None,
            };
            if let Some(action) = action {
                active.time_toast = Some(TimeAttribution {
                    player_name,
                    action,
                    remaining_ms: TIME_TOAST_DURATION_MS,
                });
            }
        }

        active
            .node
            .report_local_tick(
                &mut active.transport,
                report.tick,
                step_us,
                Some(report.world_hash),
            )
            .map_err(|e| format!("lockstep report_local_tick error: {e}"))?;

        Ok(Some(report))
    }

    /// Ticks transient HUD attribution toast timers by `elapsed_ms`. Returns
    /// `true` if toast visibility expired and the HUD should redraw.
    pub fn tick_toasts(&mut self, elapsed_ms: u32) -> bool {
        let CoopPhase::Active(active) = &mut self.phase else {
            return false;
        };
        let Some(toast) = active.time_toast.as_mut() else {
            return false;
        };
        let was_visible = toast.remaining_ms > 0;
        toast.remaining_ms = toast.remaining_ms.saturating_sub(elapsed_ms);
        if toast.remaining_ms == 0 {
            active.time_toast = None;
            return was_visible;
        }
        false
    }

    /// Gracefully leaves any active session (sending `Goodbye`), detaches
    /// `lot.detach_coop()`, and cleanly shuts down any `iroh` endpoint and
    /// Tokio worker runtime synchronously (blocking up to bounded drain/close
    /// timeouts so tests and process exit leave zero background threads).
    pub fn leave_and_shutdown(&mut self, lot: &mut Lot) {
        self.shutdown_active(lot, true);
        self.phase = CoopPhase::Idle;
    }

    /// Interactive non-blocking leave/cancel for UI buttons: sends `Goodbye`,
    /// detaches `lot.detach_coop()`, and hands endpoint drain/close + runtime
    /// shutdown to a short-lived background teardown thread so the `winit` UI
    /// thread never stalls.
    pub fn leave_nonblocking(&mut self, lot: &mut Lot) {
        self.shutdown_active(lot, false);
        self.phase = CoopPhase::Idle;
    }

    fn shutdown_active(&mut self, lot: &mut Lot, blocking: bool) {
        let prev = std::mem::replace(&mut self.phase, CoopPhase::Idle);
        match prev {
            CoopPhase::Starting(mut pending) => {
                if blocking {
                    pending.cancel_blocking();
                } else {
                    pending.cancel_background();
                }
            }
            CoopPhase::Active(mut active) => {
                let _ = active.node.leave_gracefully(&mut active.transport);
                if blocking {
                    active.transport.shutdown();
                } else {
                    active.transport.shutdown_background();
                }
            }
            CoopPhase::Idle | CoopPhase::Failed(_) => {}
        }
        let _ = lot.detach_coop();
    }

    /// Builds a read-only [`CoopSnapshot`] for [`sims_ui::coop::CoopLobbyDialog`]
    /// and [`sims_ui::coop::CoopHudOverlay`].
    pub fn ui_snapshot<F>(&self, lot: &Lot, mut sim_name: F) -> CoopSnapshot
    where
        F: FnMut(u16) -> Option<String>,
    {
        match &self.phase {
            CoopPhase::Idle => CoopSnapshot::default(),
            CoopPhase::Failed(state) => CoopSnapshot {
                role: CoopRole::Offline,
                connection: state.clone(),
                ..CoopSnapshot::default()
            },
            CoopPhase::Starting(pending) => CoopSnapshot {
                role: if pending.is_host {
                    CoopRole::Host
                } else {
                    CoopRole::Client
                },
                connection: ConnectionState::Connecting,
                resync: Some(ResyncProgress {
                    bytes_received: 15,
                    bytes_total: 100,
                    reason: if pending.is_host {
                        "Starting iroh host & hashing game manifest...".to_owned()
                    } else {
                        "Connecting & verifying game file manifest...".to_owned()
                    },
                }),
                ..CoopSnapshot::default()
            },
            CoopPhase::Active(active) => {
                let session = active.node.session();
                let local_id = active.node.local_player_id();
                let role = match active.node.role() {
                    PlayerRole::Host => CoopRole::Host,
                    PlayerRole::Peer => CoopRole::Client,
                };
                let players: Vec<PlayerInfo> = session
                    .players
                    .values()
                    .map(|p| PlayerInfo {
                        id: UiPlayerId(u64::from(p.player_id.0)),
                        name: p.name.clone(),
                        is_host: p.role == PlayerRole::Host,
                        is_local: Some(p.player_id) == local_id,
                        selected_sim_name: p.selected_sim.and_then(&mut sim_name),
                        mode: match p.mode {
                            PlayerMode::Live => UiPlayerMode::Live,
                            PlayerMode::Buy => UiPlayerMode::Buy,
                            PlayerMode::Build => UiPlayerMode::Build,
                        },
                        badge_color: player_badge_color(p.player_id),
                        ping_ms: None,
                    })
                    .collect();

                let raw_reasons = session.pause_reasons();
                let mut pause_reasons = Vec::new();
                if raw_reasons.explicit_pause {
                    let who = raw_reasons
                        .explicit_pause_by
                        .map(|id| active.player_name(id))
                        .unwrap_or_else(|| "Host".to_owned());
                    pause_reasons.push(PauseReason::Explicit { player_name: who });
                }
                for (pid, mode) in raw_reasons.players_in_buy_or_build {
                    let who = active.player_name(pid);
                    match mode {
                        PlayerMode::Buy => {
                            pause_reasons.push(PauseReason::BuyMode { player_name: who })
                        }
                        PlayerMode::Build => {
                            pause_reasons.push(PauseReason::BuildMode { player_name: who })
                        }
                        PlayerMode::Live => {}
                    }
                }
                if !raw_reasons.unanswered_dialogs.is_empty() || lot.asking().is_some() {
                    pause_reasons.push(PauseReason::BlockingDialog);
                }

                let resync = if active.node.is_syncing() {
                    let pct = (active.resync_progress_ticks.saturating_mul(8)).clamp(10, 92);
                    Some(ResyncProgress {
                        bytes_received: pct,
                        bytes_total: 100,
                        reason: active
                            .resync_reason
                            .clone()
                            .unwrap_or_else(|| "Syncing lot snapshot...".to_owned()),
                    })
                } else {
                    None
                };

                CoopSnapshot {
                    role,
                    connection: active.connection.clone(),
                    players,
                    host_only_time_control: session.time.host_speed_lock,
                    pause_reasons,
                    time_toast: active.time_toast.clone(),
                    resync,
                }
            }
        }
    }
}

impl Drop for CoopController {
    fn drop(&mut self) {
        let prev = std::mem::replace(&mut self.phase, CoopPhase::Idle);
        match prev {
            CoopPhase::Starting(mut pending) => pending.cancel_blocking(),
            CoopPhase::Active(mut active) => {
                let _ = active.node.leave_gracefully(&mut active.transport);
                active.transport.shutdown();
            }
            CoopPhase::Idle | CoopPhase::Failed(_) => {}
        }
    }
}
