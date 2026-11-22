//! Host-sequenced lockstep scheduler (`LockstepNode`).
//!
//! Drives both host and peer roles deterministically over any [`Transport`]:
//! - Authenticated transport-identity binding on every protocol message.
//! - Host-sequenced `TickBatch` scheduling preserving host receipt order
//!   (including blocking dialog first-answer-wins) with encoded-frame budgeting
//!   and per-peer ordered outbound retry queues.
//! - Configurable peer tick buffer delay (`config.tick_delay`) preserved during
//!   normal stepping and validated against `max_ticks_ahead_of_slowest`.
//! - Adaptive pacing to the slowest machine (bounded host lead + integer-only
//!   smoothed tick period hints).
//! - Periodic state fingerprint comparison & automatic drift detection.
//! - Resumable chunked/compressed REDB snapshot transfer for mid-game join and
//!   drift recovery, acknowledging completion only after digest verification
//!   and caller world installation confirmation.
//! - Deterministic host failover with epoch fencing, candidate retry/fallback,
//!   and fingerprint-verified same-tick continuation.

use crate::error::{SessionError, SnapshotError, TransportError};
use crate::protocol::{
    CameraRotation, Command, CommandSeq, DEFAULT_SNAPSHOT_CHUNK_BYTES, Epoch, LeaveReason,
    MAX_WIRE_FRAME_BYTES, ManifestDigest, PROTOCOL_VERSION, PeerRosterEntry, PlayerId, PlayerRole,
    RejectReason, ResyncReason, SessionConfig, SimSpeed, SnapshotId, StampedCommand,
    StateFingerprint, Tick, TickBatch, WireMessage, encode_payload,
};
use crate::session::state::{AppliedBatch, PerPlayerState, SessionState};
use crate::snapshot::{SnapshotCompression, SnapshotEnvelope, SnapshotReceiver, SnapshotSender};
use crate::transport::{
    INVITE_TICKET_PREFIX, InviteTicket, PeerAddress, Transport, TransportEvent,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Maximum queued outbound wire messages per peer before the host treats the
/// peer as unrecoverably stalled and disconnects it to protect session liveness.
const MAX_PEER_OUTBOUND_QUEUE: usize = 256;

/// High-water mark on a peer's outbound queue while streaming snapshot chunks
/// so regular control/batch traffic retains headroom.
const SNAPSHOT_OUTBOUND_HIGH_WATER: usize = 128;

/// Safety margin below [`MAX_WIRE_FRAME_BYTES`] when budgeting commands into a
/// single [`TickBatch`].
const BATCH_FRAME_HEADROOM: usize = 1024;

/// Maximum consecutive snapshot verification/overflow retries before giving up.
const MAX_SNAPSHOT_RETRIES: u8 = 3;

/// Maximum UTF-8 byte length of a player display name accepted on `Hello`.
const MAX_PLAYER_NAME_BYTES: usize = 64;

/// Maximum byte length of an `endpoint_hint` accepted on `Hello`.
const MAX_ENDPOINT_HINT_BYTES: usize = 2048;

/// Outcome of calling [`LockstepNode::advance_host_tick`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostStepResult {
    /// Host scheduled and queued a new [`TickBatch`] for all connected peers.
    Scheduled(TickBatch),
    /// Adaptive pacing held back scheduling because a ready peer has fallen
    /// behind `max_ticks_ahead_of_slowest`.
    WaitingForSlowestPeer {
        slowest_peer: PlayerId,
        slowest_tick: Tick,
    },
}

/// High-level events emitted by [`LockstepNode::poll`] and execution steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeEvent {
    /// Local peer received `Welcome` and was assigned `player_id`.
    Joined {
        player_id: PlayerId,
        host_id: PlayerId,
        epoch: Epoch,
        awaiting_snapshot: bool,
    },
    /// Host rejected the join attempt (e.g. manifest mismatch or full session).
    JoinRejected(RejectReason),
    /// Host accepted a peer and requests REDB snapshot bytes to stream to it
    /// (mid-game join, drift resync, or host failover catch-up).
    SnapshotNeeded {
        peer: PeerAddress,
        player_id: PlayerId,
        snapshot_id: SnapshotId,
        reason: ResyncReason,
    },
    /// Peer finished receiving and verifying a REDB snapshot. The caller MUST
    /// load `redb_bytes` into its simulation world and then call
    /// [`LockstepNode::confirm_snapshot_applied`] before stepping subsequent
    /// ticks.
    ApplySnapshot {
        snapshot_id: SnapshotId,
        at_tick: Tick,
        state_fingerprint: StateFingerprint,
        redb_bytes: Vec<u8>,
    },
    /// A snapshot transfer failed validation or decoding.
    SnapshotFailed {
        snapshot_id: SnapshotId,
        error: SnapshotError,
    },
    /// State fingerprint comparison detected drift at `tick`.
    DriftDetected {
        player_id: PlayerId,
        tick: Tick,
        expected: StateFingerprint,
        actual: StateFingerprint,
    },
    /// Local node was deterministically promoted to Host after the previous
    /// host departed.
    HostPromoted {
        new_host: PlayerId,
        new_epoch: Epoch,
    },
    /// Surviving peer adopted a new deterministically elected Host.
    HostChanged {
        new_host: PlayerId,
        new_epoch: Epoch,
    },
    /// A peer joined the session (host-side notification).
    PeerAccepted {
        peer: PeerAddress,
        player_id: PlayerId,
        name: String,
    },
    /// A peer disconnected from the host.
    PeerDisconnected {
        peer: PeerAddress,
        player_id: PlayerId,
        reason: LeaveReason,
    },
    /// The session ended (e.g. no remaining host candidate or unrecoverable sync).
    SessionEnded { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PeerStatus {
    Ready,
    Syncing {
        snapshot_id: SnapshotId,
        reason: ResyncReason,
        snapshot_tick: Option<Tick>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JoinLifecycle {
    /// Waiting for host to call `send_snapshot` before emitting `PlayerJoined`.
    PendingSnapshot {
        name: String,
        endpoint_hint: Option<String>,
        transport_addr: String,
    },
    /// `Command::PlayerJoined` is in `host_system_commands`, not yet stamped.
    JoinQueued,
    /// `Command::PlayerJoined` was stamped into a `TickBatch` (may or may not
    /// have executed yet), so disconnecting requires emitting `PlayerLeft`.
    JoinStamped,
    /// Player is already active in `session.players`.
    Active,
}

#[derive(Debug, Clone)]
struct HostPeerTrack {
    player_id: PlayerId,
    status: PeerStatus,
    join_lifecycle: JoinLifecycle,
    last_sent_tick: Tick,
    last_acked_tick: Tick,
    last_command_seq: Option<CommandSeq>,
    reported_step_us: u32,
    outbound: VecDeque<WireMessage>,
}

#[derive(Debug, Clone)]
struct PendingSnapshotRequest {
    peer: PeerAddress,
    player_id: PlayerId,
    reason: ResyncReason,
}

#[derive(Debug, Clone)]
struct ActiveSnapshotTx {
    peer: PeerAddress,
    sender: SnapshotSender,
    header_sent: bool,
    next_chunk: u32,
}

#[derive(Debug, Clone)]
struct PendingSnapshotInstall {
    snapshot_id: SnapshotId,
    at_tick: Tick,
    session: SessionState,
    total_chunks: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PeerConnectionPhase {
    Connecting,
    Joined,
    AwaitingFailover {
        expected_host: PlayerId,
        expected_epoch: Epoch,
    },
}

/// Deterministic host/peer lockstep state machine.
pub struct LockstepNode {
    role: PlayerRole,
    local_name: String,
    local_endpoint_hint: Option<String>,
    local_player_id: Option<PlayerId>,
    manifest: ManifestDigest,
    session: SessionState,

    // Shared lockstep buffering and fingerprint history.
    pending_batches: BTreeMap<Tick, TickBatch>,
    retained_batches: BTreeMap<Tick, TickBatch>,
    local_fingerprints: BTreeMap<Tick, StateFingerprint>,
    last_reported_host_fp: Option<(Tick, StateFingerprint)>,
    local_step_us: u32,
    primed: bool,
    target_buffer_delay: u32,

    // Host-side state.
    host_next_player_id: u32,
    next_schedule_tick: Tick,
    next_snapshot_id: u64,
    host_peers_by_addr: BTreeMap<PeerAddress, HostPeerTrack>,
    /// FIFO host-receipt queue of `(issuer, command)` preserving exact arrival
    /// order across players (e.g. for blocking dialog first-answer-wins).
    host_pending_commands: VecDeque<(PlayerId, Command)>,
    host_peer_queued_counts: BTreeMap<PlayerId, usize>,
    host_system_commands: VecDeque<Command>,
    pending_snapshots: BTreeMap<SnapshotId, PendingSnapshotRequest>,
    active_snapshot_txs: BTreeMap<SnapshotId, ActiveSnapshotTx>,
    smoothed_pacing_ms: u32,

    // Peer-side state.
    host_addr: Option<PeerAddress>,
    peer_phase: PeerConnectionPhase,
    failed_host_candidates: BTreeSet<PlayerId>,
    peer_outbound: VecDeque<WireMessage>,
    next_command_seq: CommandSeq,
    active_snapshot_rx: Option<SnapshotReceiver>,
    pending_snapshot_install: Option<PendingSnapshotInstall>,
    awaiting_snapshot_id: Option<SnapshotId>,
    resync_requested: bool,
    snapshot_retry_count: u8,
    hello_sent: bool,

    // Emitted events drained by the caller.
    events: VecDeque<NodeEvent>,
}

impl LockstepNode {
    /// Creates a new host node at tick `0` (epoch `1`, `PlayerId(1)`).
    ///
    /// Invalid config bounds are sanitized via [`SessionConfig::sanitized`] so
    /// construction never panics; callers wishing strict validation can call
    /// [`SessionConfig::validate`] beforehand.
    pub fn new_host(
        host_name: impl Into<String>,
        host_endpoint_hint: Option<String>,
        manifest: ManifestDigest,
        config: SessionConfig,
    ) -> Self {
        let host_name = host_name.into();
        let config = config.sanitized();
        let initial_period = SimSpeed::NORMAL.period_ms();
        let target_delay = config.tick_delay;
        let session = SessionState::new(host_name.clone(), host_endpoint_hint.clone(), config);
        let host_next_player_id = session.next_player_id;
        Self {
            role: PlayerRole::Host,
            local_name: host_name,
            local_endpoint_hint: host_endpoint_hint,
            local_player_id: Some(PlayerId::INITIAL_HOST),
            manifest,
            session,
            pending_batches: BTreeMap::new(),
            retained_batches: BTreeMap::new(),
            local_fingerprints: BTreeMap::new(),
            last_reported_host_fp: None,
            local_step_us: 0,
            primed: true,
            target_buffer_delay: target_delay,
            host_next_player_id,
            next_schedule_tick: Tick(1),
            next_snapshot_id: 1,
            host_peers_by_addr: BTreeMap::new(),
            host_pending_commands: VecDeque::new(),
            host_peer_queued_counts: BTreeMap::new(),
            host_system_commands: VecDeque::new(),
            pending_snapshots: BTreeMap::new(),
            active_snapshot_txs: BTreeMap::new(),
            smoothed_pacing_ms: initial_period,
            host_addr: None,
            peer_phase: PeerConnectionPhase::Joined,
            failed_host_candidates: BTreeSet::new(),
            peer_outbound: VecDeque::new(),
            next_command_seq: CommandSeq(1),
            active_snapshot_rx: None,
            pending_snapshot_install: None,
            awaiting_snapshot_id: None,
            resync_requested: false,
            snapshot_retry_count: 0,
            hello_sent: true,
            events: VecDeque::new(),
        }
    }

    /// Creates a new peer node ready to connect to `host_addr`.
    pub fn new_peer(
        player_name: impl Into<String>,
        local_endpoint_hint: Option<String>,
        manifest: ManifestDigest,
        host_addr: PeerAddress,
    ) -> Self {
        let config = SessionConfig::default();
        let target_delay = config.tick_delay;
        let session = SessionState::new("host", None, config);
        let host_next_player_id = session.next_player_id;
        Self {
            role: PlayerRole::Peer,
            local_name: player_name.into(),
            local_endpoint_hint,
            local_player_id: None,
            manifest,
            session,
            pending_batches: BTreeMap::new(),
            retained_batches: BTreeMap::new(),
            local_fingerprints: BTreeMap::new(),
            last_reported_host_fp: None,
            local_step_us: 0,
            primed: false,
            target_buffer_delay: target_delay,
            host_next_player_id,
            next_schedule_tick: Tick(1),
            next_snapshot_id: 1,
            host_peers_by_addr: BTreeMap::new(),
            host_pending_commands: VecDeque::new(),
            host_peer_queued_counts: BTreeMap::new(),
            host_system_commands: VecDeque::new(),
            pending_snapshots: BTreeMap::new(),
            active_snapshot_txs: BTreeMap::new(),
            smoothed_pacing_ms: SimSpeed::NORMAL.period_ms(),
            host_addr: Some(host_addr),
            peer_phase: PeerConnectionPhase::Connecting,
            failed_host_candidates: BTreeSet::new(),
            peer_outbound: VecDeque::new(),
            next_command_seq: CommandSeq(1),
            active_snapshot_rx: None,
            pending_snapshot_install: None,
            awaiting_snapshot_id: None,
            resync_requested: false,
            snapshot_retry_count: 0,
            hello_sent: false,
            events: VecDeque::new(),
        }
    }

    /// Current role (`Host` or `Peer`).
    pub const fn role(&self) -> PlayerRole {
        self.role
    }

    /// Local assigned [`PlayerId`], once joined.
    pub const fn local_player_id(&self) -> Option<PlayerId> {
        self.local_player_id
    }

    /// Read-only reference to the deterministic [`SessionState`].
    pub const fn session(&self) -> &SessionState {
        &self.session
    }

    /// Mutable access to register simulation-side events such as
    /// [`SessionState::open_blocking_dialog`] or
    /// [`SessionState::apply_script_selection_change`].
    pub fn session_mut(&mut self) -> &mut SessionState {
        &mut self.session
    }

    /// Current target buffer delay (in ticks) on a peer.
    pub const fn target_buffer_delay(&self) -> u32 {
        self.target_buffer_delay
    }

    /// Number of future [`TickBatch`]es currently buffered locally.
    pub fn buffered_batch_count(&self) -> usize {
        self.pending_batches.len()
    }

    /// Whether this peer is currently receiving, waiting for, or awaiting
    /// caller installation of a REDB snapshot.
    pub fn is_syncing(&self) -> bool {
        self.awaiting_snapshot_id.is_some()
            || self.active_snapshot_rx.is_some()
            || self.pending_snapshot_install.is_some()
            || self.resync_requested
    }

    /// Current adaptive pacing period in milliseconds.
    pub const fn pacing_hint_ms(&self) -> u32 {
        self.smoothed_pacing_ms
    }

    /// Number of active host-side snapshot streams still draining chunks.
    pub fn active_snapshot_tx_count(&self) -> usize {
        self.active_snapshot_txs.len()
    }

    /// Drains accumulated high-level [`NodeEvent`]s in FIFO order.
    pub fn drain_events(&mut self) -> Vec<NodeEvent> {
        self.events.drain(..).collect()
    }

    /// Initiates a peer's `Hello` handshake over `transport`.
    pub fn send_hello<T: Transport>(&mut self, transport: &mut T) -> Result<(), SessionError> {
        if self.role != PlayerRole::Peer {
            return Err(SessionError::NotPeer);
        }
        let Some(host_addr) = self.host_addr.clone() else {
            return Err(SessionError::NotPeer);
        };
        transport.send(
            &host_addr,
            WireMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                player_name: self.local_name.clone(),
                manifest: self.manifest,
                endpoint_hint: self.local_endpoint_hint.clone(),
            },
        )?;
        self.hello_sent = true;
        Ok(())
    }

    /// Submits a local player [`Command`].
    ///
    /// - On the host, queues directly into the FIFO command queue (bounded by
    ///   per-peer ingress limit).
    /// - On a peer, sends [`WireMessage::SubmitCommand`] to the host with a
    ///   monotonic [`CommandSeq`].
    pub fn submit_command<T: Transport>(
        &mut self,
        transport: &mut T,
        command: Command,
    ) -> Result<(), SessionError> {
        if command.is_system_membership() {
            return Err(SessionError::NotHost);
        }
        match self.role {
            PlayerRole::Host => {
                let id = self.local_player_id.ok_or(SessionError::UnassignedPlayer)?;
                if !self.enqueue_host_command(id, command) {
                    return Err(
                        TransportError::Backpressure(format!("host queue full for {id}")).into(),
                    );
                }
                Ok(())
            }
            PlayerRole::Peer => {
                if self.local_player_id.is_none() {
                    return Err(SessionError::UnassignedPlayer);
                }
                let seq = self.next_command_seq;
                self.next_command_seq = seq.next();
                self.enqueue_peer_to_host(
                    transport,
                    WireMessage::SubmitCommand {
                        epoch: self.session.epoch,
                        seq,
                        command,
                    },
                )
            }
        }
    }

    /// Peer-only: inspects the pending snapshot's authoritative [`SessionState`]
    /// before [`Self::confirm_snapshot_applied`] installs it and sends the
    /// completion ACK, allowing callers to transactionally verify/stage the VM
    /// snapshot first.
    pub fn pending_snapshot_session(&self, snapshot_id: SnapshotId) -> Option<&SessionState> {
        self.pending_snapshot_install
            .as_ref()
            .filter(|p| p.snapshot_id == snapshot_id)
            .map(|p| &p.session)
    }

    /// Confirms that the caller has finished loading the REDB snapshot emitted
    /// via [`NodeEvent::ApplySnapshot`], atomically installing the snapshot's
    /// [`SessionState`], unblocking tick stepping, and acknowledging completion
    /// to the host.
    pub fn confirm_snapshot_applied<T: Transport>(
        &mut self,
        transport: &mut T,
        snapshot_id: SnapshotId,
    ) -> Result<(), SessionError> {
        if self.role != PlayerRole::Peer {
            return Err(SessionError::NotPeer);
        }
        let Some(pending) = self.pending_snapshot_install.as_ref() else {
            return Err(SessionError::UnknownSnapshot(snapshot_id));
        };
        if pending.snapshot_id != snapshot_id {
            return Err(SessionError::UnknownSnapshot(snapshot_id));
        }
        let pending = self
            .pending_snapshot_install
            .take()
            .ok_or(SessionError::UnknownSnapshot(snapshot_id))?;

        let at_tick = pending.at_tick;
        self.session = pending.session;
        self.session.current_tick = at_tick;
        // Drop buffered batches at or before `at_tick`; keep strictly newer batches.
        self.pending_batches.retain(|&t, _| t > at_tick);
        self.local_fingerprints.clear();
        self.awaiting_snapshot_id = None;
        self.resync_requested = false;
        self.snapshot_retry_count = 0;
        self.primed = false;

        // Queue completion ACK onto the peer's ordered retry pipeline so
        // transient backpressure can never drop it and strand the host in Syncing.
        let _ = self.enqueue_peer_to_host(
            transport,
            WireMessage::SnapshotAck {
                epoch: self.session.epoch,
                snapshot_id,
                received_chunks: pending.total_chunks,
                complete: true,
            },
        );
        Ok(())
    }

    /// Peer-only: signals that the currently awaited failover host candidate
    /// could not be reached (e.g. caller timeout), deterministically advancing
    /// to the next surviving host candidate or ending the session.
    pub fn failover_candidate_unreachable<T: Transport>(&mut self, transport: &mut T) {
        if self.role != PlayerRole::Peer {
            return;
        }
        if let PeerConnectionPhase::AwaitingFailover { expected_host, .. } = self.peer_phase {
            self.failed_host_candidates.insert(expected_host);
            self.advance_failover_candidate(transport);
        }
    }

    /// Gracefully leaves the session, notifying peers/host via `Goodbye`.
    pub fn leave_gracefully<T: Transport>(
        &mut self,
        transport: &mut T,
    ) -> Result<(), SessionError> {
        let msg = WireMessage::Goodbye {
            epoch: self.session.epoch,
            reason: if self.role == PlayerRole::Host {
                LeaveReason::HostDisconnected
            } else {
                LeaveReason::Voluntary
            },
        };
        match self.role {
            PlayerRole::Host => {
                let addrs: Vec<PeerAddress> = self.host_peers_by_addr.keys().cloned().collect();
                for addr in addrs {
                    let _ = transport.send(&addr, msg.clone());
                    let _ = transport.disconnect(&addr);
                }
            }
            PlayerRole::Peer => {
                if let Some(host_addr) = self.host_addr.clone() {
                    let _ = transport.send(&host_addr, msg);
                    let _ = transport.disconnect(&host_addr);
                }
            }
        }
        Ok(())
    }

    /// Polls `transport` for incoming events and messages, flushes any pending
    /// outbound retries and active snapshot chunk streams, and queues
    /// [`NodeEvent`]s.
    pub fn poll<T: Transport>(&mut self, transport: &mut T) {
        let incoming = transport.drain_events();
        for event in incoming {
            match event {
                TransportEvent::PeerConnected { peer } => {
                    self.handle_peer_connected(transport, peer);
                }
                TransportEvent::PeerDisconnected { peer, reason } => {
                    self.handle_peer_disconnected(transport, peer, reason);
                }
                TransportEvent::Message { from, message } => {
                    self.handle_message(transport, from, message);
                }
            }
        }
        if self.role == PlayerRole::Host {
            self.pump_snapshot_txs(transport);
            self.flush_all_peer_outbound(transport);
        } else {
            self.flush_peer_to_host(transport);
        }
    }

    /// Host-only: sequences queued commands into the next [`TickBatch`]
    /// (preserving host receipt order and encoded-frame size limits), queues it
    /// on every connected peer's ordered outbound pipeline, and buffers it
    /// locally.
    ///
    /// Respects adaptive pacing: if any ready peer's `last_acked_tick` lags
    /// behind `next_schedule_tick` by more than `max_ticks_ahead_of_slowest`,
    /// returns [`HostStepResult::WaitingForSlowestPeer`] without advancing.
    pub fn advance_host_tick<T: Transport>(
        &mut self,
        transport: &mut T,
    ) -> Result<HostStepResult, SessionError> {
        if self.role != PlayerRole::Host {
            return Err(SessionError::NotHost);
        }

        // First drain any pending outbound frames or snapshot chunks.
        self.pump_snapshot_txs(transport);
        self.flush_all_peer_outbound(transport);

        // Adaptive pacing: do not outrun the slowest ready peer past the window.
        let mut slowest: Option<(PlayerId, Tick)> = None;
        for track in self.host_peers_by_addr.values() {
            if !matches!(track.status, PeerStatus::Ready) {
                continue;
            }
            match slowest {
                None => slowest = Some((track.player_id, track.last_acked_tick)),
                Some((_, cur)) if track.last_acked_tick < cur => {
                    slowest = Some((track.player_id, track.last_acked_tick));
                }
                _ => {}
            }
        }
        if let Some((slowest_peer, slowest_tick)) = slowest {
            let lead = self
                .next_schedule_tick
                .checked_sub(slowest_tick)
                .unwrap_or(0);
            if lead > self.session.config.max_ticks_ahead_of_slowest {
                return Ok(HostStepResult::WaitingForSlowestPeer {
                    slowest_peer,
                    slowest_tick,
                });
            }
        }

        let tick = self.next_schedule_tick;
        let pacing_hint_ms = self.compute_host_pacing_ms();
        self.smoothed_pacing_ms = pacing_hint_ms;
        let host_fingerprint = self.last_reported_host_fp.take();

        let mut batch = TickBatch {
            epoch: self.session.epoch,
            tick,
            commands: Vec::new(),
            pacing_hint_ms,
            host_fingerprint,
        };
        let mut seq = 0u32;
        let max_batch_bytes = MAX_WIRE_FRAME_BYTES.saturating_sub(BATCH_FRAME_HEADROOM);

        // 1. Host system membership/failover commands first, in FIFO order,
        //    also budgeted against `max_batch_bytes` so a batch can never exceed
        //    the wire frame limit even under simultaneous tick-0 joins.
        let host_id = self.session.host_id;
        let mut stamped_joins: Vec<PlayerId> = Vec::new();
        while let Some(cmd) = self.host_system_commands.pop_front() {
            let join_pid = match &cmd {
                Command::PlayerJoined { player_id, .. } => Some(*player_id),
                _ => None,
            };
            batch.commands.push(StampedCommand {
                tick,
                sequence: seq,
                issuer: host_id,
                command: cmd,
            });
            let encoded_ok = encode_payload(&WireMessage::TickBatch(batch.clone()))
                .is_ok_and(|bytes| bytes.len() <= max_batch_bytes);
            if !encoded_ok && batch.commands.len() > 1 {
                let popped = batch.commands.pop().expect("just pushed");
                self.host_system_commands.push_front(popped.command);
                break;
            }
            if let Some(pid) = join_pid {
                stamped_joins.push(pid);
            }
            seq = seq.saturating_add(1);
        }

        // 2. Player commands in exact FIFO host receipt order, respecting both
        //    per-peer per-tick quota AND encoded wire-frame size budget.
        let per_peer_cap = self.session.config.max_commands_per_peer_per_tick.max(1);
        let mut taken_per_peer: BTreeMap<PlayerId, usize> = BTreeMap::new();
        let mut deferred: VecDeque<(PlayerId, Command)> = VecDeque::new();

        while let Some((player_id, cmd)) = self.host_pending_commands.pop_front() {
            let taken = taken_per_peer.get(&player_id).copied().unwrap_or(0);
            if taken >= per_peer_cap {
                deferred.push_back((player_id, cmd));
                continue;
            }
            let candidate = StampedCommand {
                tick,
                sequence: seq,
                issuer: player_id,
                command: cmd,
            };
            batch.commands.push(candidate);
            let encoded_ok = encode_payload(&WireMessage::TickBatch(batch.clone()))
                .is_ok_and(|bytes| bytes.len() <= max_batch_bytes);
            if !encoded_ok {
                let popped = batch.commands.pop().expect("just pushed");
                deferred.push_back((popped.issuer, popped.command));
                // Preserve exact FIFO order for remaining commands in queue.
                while let Some(rest) = self.host_pending_commands.pop_front() {
                    deferred.push_back(rest);
                }
                break;
            }
            seq = seq.saturating_add(1);
            taken_per_peer.insert(player_id, taken + 1);
            if let Some(count) = self.host_peer_queued_counts.get_mut(&player_id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.host_peer_queued_counts.remove(&player_id);
                }
            }
        }
        self.host_pending_commands = deferred;

        // Mark stamped joins so a subsequent disconnect always emits PlayerLeft.
        for pid in stamped_joins {
            for track in self.host_peers_by_addr.values_mut() {
                if track.player_id == pid && track.join_lifecycle == JoinLifecycle::JoinQueued {
                    track.join_lifecycle = JoinLifecycle::JoinStamped;
                }
            }
        }

        self.next_schedule_tick = tick.next();
        self.record_retained_batch(batch.clone());
        self.pending_batches.insert(tick, batch.clone());

        // Queue batch onto every tracked peer's ordered outbound pipeline.
        let recipients: Vec<PeerAddress> = self.host_peers_by_addr.keys().cloned().collect();
        for addr in recipients {
            if let Some(track) = self.host_peers_by_addr.get_mut(&addr) {
                track.last_sent_tick = tick;
            }
            self.enqueue_peer_message(transport, &addr, WireMessage::TickBatch(batch.clone()));
        }

        Ok(HostStepResult::Scheduled(batch))
    }

    /// Returns whether the next tick (`session.current_tick + 1`) is ready to
    /// execute under the current buffer delay policy.
    ///
    /// - When `flush` is `false` on a peer, requires at least
    ///   `target_buffer_delay.max(1)` contiguous buffered batches starting at
    ///   `current_tick + 1` so the configured tick delay behind the host is
    ///   both established on startup and maintained during steady-state play.
    /// - When `flush` is `true`, executes any contiguous buffered tick without
    ///   waiting for the buffer threshold (useful for catch-up or teardown).
    /// - While syncing or awaiting host failover, always returns `false`.
    pub fn can_step_tick(&self, flush: bool) -> bool {
        if self.is_syncing() {
            return false;
        }
        if matches!(
            self.peer_phase,
            PeerConnectionPhase::AwaitingFailover { .. }
        ) {
            return false;
        }
        let next = self.session.current_tick.next();
        if !self.pending_batches.contains_key(&next) {
            return false;
        }
        if self.role == PlayerRole::Host || flush {
            return true;
        }
        let min_buf = (self.target_buffer_delay as usize).max(1);
        self.contiguous_buffered_len(next) >= min_buf
    }

    /// Executes the next ready [`TickBatch`] against [`SessionState`], returning
    /// the [`AppliedBatch`] for the simulation VM to execute.
    pub fn step_next_tick(&mut self, flush: bool) -> Result<Option<AppliedBatch>, SessionError> {
        if !self.can_step_tick(flush) {
            return Ok(None);
        }
        let next = self.session.current_tick.next();
        let Some(batch) = self.pending_batches.remove(&next) else {
            return Ok(None);
        };
        if self.role == PlayerRole::Peer {
            self.primed = true;
            self.smoothed_pacing_ms = batch.pacing_hint_ms;
        }
        let applied = self.session.apply_batch(&batch)?;
        // Transition any JoinStamped peers now present in `session.players` to Active.
        if self.role == PlayerRole::Host {
            for track in self.host_peers_by_addr.values_mut() {
                if track.join_lifecycle == JoinLifecycle::JoinStamped
                    && self.session.players.contains_key(&track.player_id)
                {
                    track.join_lifecycle = JoinLifecycle::Active;
                }
            }
        }
        self.record_retained_batch(batch);
        Ok(Some(applied))
    }

    /// Executes all currently ready contiguous ticks (respecting `flush`).
    ///
    /// Note: when reporting periodic state fingerprints, prefer stepping one
    /// tick at a time via [`Self::step_next_tick`] followed immediately by
    /// [`Self::report_local_tick`] so each world hash pairs with its exact
    /// [`SessionState`] tick.
    pub fn drain_ready_ticks(&mut self, flush: bool) -> Result<Vec<AppliedBatch>, SessionError> {
        let mut out = Vec::new();
        while let Some(applied) = self.step_next_tick(flush)? {
            out.push(applied);
        }
        Ok(out)
    }

    /// Reports the local simulation's execution of `tick`, its step duration in
    /// microseconds (for adaptive pacing), and optional state fingerprint.
    ///
    /// Enforces `tick == self.session.current_tick` so a world hash from an
    /// earlier tick can never be paired with a later [`SessionState`].
    pub fn report_local_tick<T: Transport>(
        &mut self,
        transport: &mut T,
        tick: Tick,
        sim_step_us: u32,
        world_hash: Option<[u8; 32]>,
    ) -> Result<(), SessionError> {
        if tick != self.session.current_tick {
            return Err(SessionError::UnexpectedTick {
                expected: self.session.current_tick,
                actual: tick,
            });
        }
        self.local_step_us = sim_step_us;
        let is_fp_tick = self.session.config.fingerprint_interval > 0
            && tick.0 > 0
            && tick
                .0
                .is_multiple_of(self.session.config.fingerprint_interval);

        let fingerprint = world_hash.map(|wh| self.session.combined_fingerprint(wh));
        if let Some(fp) = fingerprint {
            // Always record latest executed tick fingerprint (for failover verification)
            // and periodic interval fingerprints (for drift checks).
            self.record_local_fingerprint(tick, fp);
            if is_fp_tick && self.role == PlayerRole::Host {
                self.last_reported_host_fp = Some((tick, fp));
            }
        }

        if self.role == PlayerRole::Peer && self.peer_phase == PeerConnectionPhase::Joined {
            let ack_fp = if is_fp_tick { fingerprint } else { None };
            self.enqueue_peer_to_host(
                transport,
                WireMessage::TickAck {
                    epoch: self.session.epoch,
                    tick,
                    sim_step_us,
                    fingerprint: ack_fp,
                },
            )?;
        }
        Ok(())
    }

    /// Host-only: validates, compresses, and begins streaming a chunked REDB
    /// snapshot for a pending [`NodeEvent::SnapshotNeeded`] request.
    ///
    /// If the peer's transport hits backpressure, remaining chunks stay queued
    /// in an active transfer cursor and resume automatically on subsequent
    /// [`Self::poll`] / [`Self::advance_host_tick`] calls without losing the
    /// transfer.
    pub fn send_snapshot<T: Transport>(
        &mut self,
        transport: &mut T,
        snapshot_id: SnapshotId,
        world_hash: [u8; 32],
        redb_bytes: Vec<u8>,
        compression: SnapshotCompression,
        chunk_size: u32,
    ) -> Result<(), SessionError> {
        if self.role != PlayerRole::Host {
            return Err(SessionError::NotHost);
        }
        let req = self
            .pending_snapshots
            .get(&snapshot_id)
            .cloned()
            .ok_or(SessionError::UnknownSnapshot(snapshot_id))?;

        let at_tick = self.session.current_tick;
        let state_fingerprint = self.session.combined_fingerprint(world_hash);
        let envelope = SnapshotEnvelope {
            session: self.session.clone(),
            redb_bytes,
        };
        // Validate and compress BEFORE consuming the pending request.
        let sender = SnapshotSender::from_envelope(
            snapshot_id,
            self.session.epoch,
            at_tick,
            state_fingerprint,
            &envelope,
            compression,
            if chunk_size == 0 {
                DEFAULT_SNAPSHOT_CHUNK_BYTES
            } else {
                chunk_size
            },
        )?;

        self.pending_snapshots.remove(&snapshot_id);

        // Record snapshot tick and, for mid-game join, emit `PlayerJoined`
        // strictly after `at_tick` so joiners load `at_tick` and replay identically.
        if let Some(track) = self.host_peers_by_addr.get_mut(&req.peer) {
            if let PeerStatus::Syncing { snapshot_tick, .. } = &mut track.status {
                *snapshot_tick = Some(at_tick);
            }
            if req.reason == ResyncReason::MidGameJoin
                && let JoinLifecycle::PendingSnapshot {
                    name,
                    endpoint_hint,
                    transport_addr,
                } = &track.join_lifecycle
            {
                let cmd = Command::PlayerJoined {
                    player_id: req.player_id,
                    name: name.clone(),
                    selected_sim: None,
                    camera_rotation: CameraRotation::NORTH,
                    endpoint_hint: endpoint_hint.clone(),
                    transport_addr: Some(transport_addr.clone()),
                };
                track.join_lifecycle = JoinLifecycle::JoinQueued;
                self.host_system_commands.push_back(cmd);
            }
        }

        self.active_snapshot_txs.insert(
            snapshot_id,
            ActiveSnapshotTx {
                peer: req.peer,
                sender,
                header_sent: false,
                next_chunk: 0,
            },
        );
        self.pump_snapshot_txs(transport);
        self.flush_all_peer_outbound(transport);

        Ok(())
    }

    /// Host-only: aborts an in-progress or pending snapshot transfer.
    pub fn abort_snapshot(&mut self, snapshot_id: SnapshotId) -> bool {
        let a = self.pending_snapshots.remove(&snapshot_id).is_some();
        let b = self.active_snapshot_txs.remove(&snapshot_id).is_some();
        a || b
    }

    /// Host-only: aborts a failed snapshot transfer AND disconnects the
    /// affected peer cleanly with `Goodbye(SessionClosed)` so the peer exits
    /// its sync screen with an explicit error instead of stalling forever.
    pub fn fail_snapshot<T: Transport>(
        &mut self,
        transport: &mut T,
        snapshot_id: SnapshotId,
    ) -> bool {
        let peer = self
            .pending_snapshots
            .remove(&snapshot_id)
            .map(|req| req.peer)
            .or_else(|| {
                self.active_snapshot_txs
                    .remove(&snapshot_id)
                    .map(|tx| tx.peer)
            });
        let Some(peer) = peer else {
            return false;
        };
        let _ = transport.send(
            &peer,
            WireMessage::Goodbye {
                epoch: self.session.epoch,
                reason: LeaveReason::SessionClosed,
            },
        );
        let _ = transport.disconnect(&peer);
        self.handle_peer_disconnected(transport, peer, "snapshot transfer failed".to_owned());
        true
    }

    // -------------------------------------------------------------------------
    // Internal transport & message handlers
    // -------------------------------------------------------------------------

    fn handle_peer_connected<T: Transport>(&mut self, transport: &mut T, peer: PeerAddress) {
        match self.role {
            PlayerRole::Peer => {
                if self.peer_phase == PeerConnectionPhase::Connecting
                    && self
                        .host_addr
                        .as_ref()
                        .is_some_and(|h| peer_matches_hint(&peer, h.as_str()))
                    && !self.hello_sent
                {
                    self.host_addr = Some(peer);
                    let _ = self.send_hello(transport);
                } else if let PeerConnectionPhase::AwaitingFailover {
                    expected_host,
                    expected_epoch,
                } = self.peer_phase
                    && self
                        .session
                        .players
                        .get(&expected_host)
                        .is_some_and(|p| peer_matches_player(&peer, p))
                {
                    // Prompt the newly promoted host to send `HostTakeover`.
                    let _ = transport.send(
                        &peer,
                        WireMessage::Heartbeat {
                            epoch: expected_epoch,
                            tick: self.session.current_tick,
                            sender: self.local_player_id,
                        },
                    );
                }
            }
            PlayerRole::Host => {
                // If an authenticated surviving peer reconnected after host
                // failover, register it and send `HostTakeover`.
                if let Some(player) = self
                    .session
                    .players
                    .values()
                    .find(|p| p.player_id != self.session.host_id && peer_matches_player(&peer, p))
                    .cloned()
                {
                    self.register_failover_peer(transport, peer, player.player_id);
                }
            }
        }
    }

    fn handle_peer_disconnected<T: Transport>(
        &mut self,
        transport: &mut T,
        peer: PeerAddress,
        _reason: String,
    ) {
        match self.role {
            PlayerRole::Host => {
                self.remove_host_peer(peer, LeaveReason::Disconnected);
            }
            PlayerRole::Peer => match self.peer_phase {
                PeerConnectionPhase::AwaitingFailover { expected_host, .. } => {
                    if self.host_addr.as_ref() == Some(&peer)
                        || self
                            .session
                            .players
                            .get(&expected_host)
                            .is_some_and(|p| peer_matches_player(&peer, p))
                    {
                        self.failed_host_candidates.insert(expected_host);
                        self.advance_failover_candidate(transport);
                    }
                }
                PeerConnectionPhase::Connecting | PeerConnectionPhase::Joined => {
                    if self
                        .host_addr
                        .as_ref()
                        .is_some_and(|h| peer_matches_hint(&peer, h.as_str()))
                    {
                        self.initiate_host_failover(transport);
                    }
                }
            },
        }
    }

    fn handle_message<T: Transport>(
        &mut self,
        transport: &mut T,
        from: PeerAddress,
        message: WireMessage,
    ) {
        match message {
            WireMessage::Hello {
                protocol_version,
                player_name,
                manifest,
                endpoint_hint,
            } => self.on_hello(
                transport,
                from,
                protocol_version,
                player_name,
                manifest,
                endpoint_hint,
            ),
            WireMessage::Welcome {
                epoch,
                assigned_player,
                host_id,
                config,
                current_tick,
                session_state,
                snapshot_id,
                roster: _,
            } => self.on_welcome(
                from,
                epoch,
                assigned_player,
                host_id,
                config,
                current_tick,
                *session_state,
                snapshot_id,
            ),
            WireMessage::Reject { reason } => {
                if self.role == PlayerRole::Peer
                    && self.peer_phase == PeerConnectionPhase::Connecting
                    && self
                        .host_addr
                        .as_ref()
                        .is_some_and(|h| peer_matches_hint(&from, h.as_str()))
                {
                    self.events.push_back(NodeEvent::JoinRejected(reason));
                }
            }
            WireMessage::SubmitCommand {
                epoch,
                seq,
                command,
            } => self.on_submit_command(from, epoch, seq, command),
            WireMessage::TickBatch(batch) => self.on_tick_batch(transport, from, batch),
            WireMessage::TickAck {
                epoch,
                tick,
                sim_step_us,
                fingerprint,
            } => self.on_tick_ack(from, epoch, tick, sim_step_us, fingerprint),
            WireMessage::RequestSnapshot {
                epoch,
                reason,
                last_known_tick,
            } => self.on_request_snapshot(from, epoch, reason, last_known_tick),
            WireMessage::SnapshotHeader(header) => self.on_snapshot_header(transport, from, header),
            WireMessage::SnapshotChunk(chunk) => self.on_snapshot_chunk(transport, from, chunk),
            WireMessage::SnapshotAck {
                epoch,
                snapshot_id,
                received_chunks: _,
                complete,
            } => self.on_snapshot_ack(from, epoch, snapshot_id, complete),
            WireMessage::HostTakeover {
                new_epoch,
                new_host,
                resume_tick,
                resume_fingerprint,
                session_state,
                roster: _,
            } => self.on_host_takeover(
                transport,
                from,
                new_epoch,
                new_host,
                resume_tick,
                resume_fingerprint,
                *session_state,
            ),
            WireMessage::Heartbeat {
                epoch,
                tick,
                sender,
            } => self.on_heartbeat(transport, from, epoch, tick, sender),
            WireMessage::Goodbye { epoch, reason } => {
                self.on_goodbye(transport, from, epoch, reason);
            }
        }
    }

    fn on_hello<T: Transport>(
        &mut self,
        transport: &mut T,
        from: PeerAddress,
        protocol_version: u32,
        player_name: String,
        manifest: ManifestDigest,
        endpoint_hint: Option<String>,
    ) {
        if self.role != PlayerRole::Host {
            let _ = transport.send(
                &from,
                WireMessage::Reject {
                    reason: RejectReason::NotHost {
                        current_host: self.session.host_id,
                    },
                },
            );
            return;
        }
        if protocol_version != PROTOCOL_VERSION {
            let _ = transport.send(
                &from,
                WireMessage::Reject {
                    reason: RejectReason::ProtocolVersionMismatch {
                        expected: PROTOCOL_VERSION,
                        actual: protocol_version,
                    },
                },
            );
            return;
        }
        if !self.manifest.matches(manifest) {
            let _ = transport.send(
                &from,
                WireMessage::Reject {
                    reason: RejectReason::ManifestMismatch {
                        expected: self.manifest,
                        actual: manifest,
                    },
                },
            );
            return;
        }
        if player_name.trim().is_empty() || player_name.len() > MAX_PLAYER_NAME_BYTES {
            let _ = transport.send(
                &from,
                WireMessage::Reject {
                    reason: RejectReason::InvalidPlayerName,
                },
            );
            return;
        }
        // Bind `endpoint_hint` to the cryptographically authenticated sender and cap its size.
        if let Some(hint) = endpoint_hint.as_deref()
            && (hint.len() > MAX_ENDPOINT_HINT_BYTES || !peer_matches_hint(&from, hint))
        {
            let _ = transport.send(
                &from,
                WireMessage::Reject {
                    reason: RejectReason::InvalidEndpointHint,
                },
            );
            return;
        }
        if self.host_peers_by_addr.contains_key(&from) {
            return;
        }
        let active_and_reserved = self.session.players.len() + self.reserved_unapplied_joins();
        if active_and_reserved >= usize::from(self.session.config.max_players) {
            let _ = transport.send(
                &from,
                WireMessage::Reject {
                    reason: RejectReason::SessionFull {
                        max_players: self.session.config.max_players,
                    },
                },
            );
            return;
        }

        // Allocate provisionally in host bookkeeping WITHOUT mutating hashed `SessionState`.
        let assigned_player = self.allocate_provisional_player_id();
        let needs_snapshot =
            self.session.current_tick > Tick::ZERO || self.next_schedule_tick > Tick(1);

        let snapshot_id = if needs_snapshot {
            let sid = self.allocate_snapshot_id();
            self.pending_snapshots.insert(
                sid,
                PendingSnapshotRequest {
                    peer: from.clone(),
                    player_id: assigned_player,
                    reason: ResyncReason::MidGameJoin,
                },
            );
            self.host_peers_by_addr.insert(
                from.clone(),
                HostPeerTrack {
                    player_id: assigned_player,
                    status: PeerStatus::Syncing {
                        snapshot_id: sid,
                        reason: ResyncReason::MidGameJoin,
                        snapshot_tick: None,
                    },
                    join_lifecycle: JoinLifecycle::PendingSnapshot {
                        name: player_name.clone(),
                        endpoint_hint,
                        transport_addr: from.0.clone(),
                    },
                    last_sent_tick: self.session.current_tick,
                    last_acked_tick: self.session.current_tick,
                    last_command_seq: None,
                    reported_step_us: 0,
                    outbound: VecDeque::new(),
                },
            );
            self.events.push_back(NodeEvent::SnapshotNeeded {
                peer: from.clone(),
                player_id: assigned_player,
                snapshot_id: sid,
                reason: ResyncReason::MidGameJoin,
            });
            Some(sid)
        } else {
            self.host_peers_by_addr.insert(
                from.clone(),
                HostPeerTrack {
                    player_id: assigned_player,
                    status: PeerStatus::Ready,
                    join_lifecycle: JoinLifecycle::JoinQueued,
                    last_sent_tick: Tick::ZERO,
                    last_acked_tick: Tick::ZERO,
                    last_command_seq: None,
                    reported_step_us: 0,
                    outbound: VecDeque::new(),
                },
            );
            self.host_system_commands.push_back(Command::PlayerJoined {
                player_id: assigned_player,
                name: player_name.clone(),
                selected_sim: None,
                camera_rotation: CameraRotation::NORTH,
                endpoint_hint,
                transport_addr: Some(from.0.clone()),
            });
            None
        };

        let welcome = WireMessage::Welcome {
            epoch: self.session.epoch,
            assigned_player,
            host_id: self.session.host_id,
            config: self.session.config.clone(),
            current_tick: self.session.current_tick,
            session_state: Box::new(self.session.clone()),
            snapshot_id,
            roster: self.build_host_roster(),
        };
        self.enqueue_peer_message(transport, &from, welcome);

        self.events.push_back(NodeEvent::PeerAccepted {
            peer: from,
            player_id: assigned_player,
            name: player_name,
        });
    }

    #[expect(clippy::too_many_arguments)]
    fn on_welcome(
        &mut self,
        from: PeerAddress,
        epoch: Epoch,
        assigned_player: PlayerId,
        host_id: PlayerId,
        config: SessionConfig,
        _current_tick: Tick,
        session_state: SessionState,
        snapshot_id: Option<SnapshotId>,
    ) {
        if self.role != PlayerRole::Peer || self.peer_phase != PeerConnectionPhase::Connecting {
            return;
        }
        // Only accept Welcome from the invited host transport identity.
        if !self
            .host_addr
            .as_ref()
            .is_some_and(|h| peer_matches_hint(&from, h.as_str()))
        {
            return;
        }
        // Validate untrusted SessionConfig before using it (prevents clamp panic / deadlock).
        if config.validate().is_err() {
            self.events
                .push_back(NodeEvent::JoinRejected(RejectReason::InvalidConfig));
            return;
        }

        // Keep authenticated host address in `self.host_addr` WITHOUT mutating
        // `session_state.players` so peer and host `SessionState` fingerprints stay identical.
        self.host_addr = Some(from);
        self.local_player_id = Some(assigned_player);
        self.target_buffer_delay = config.tick_delay;
        self.session = session_state;
        self.session.epoch = epoch;
        self.session.host_id = host_id;
        self.session.config = config;
        self.peer_phase = PeerConnectionPhase::Joined;
        self.awaiting_snapshot_id = snapshot_id;
        self.primed = snapshot_id.is_none() && self.target_buffer_delay == 0;
        self.resync_requested = snapshot_id.is_some();
        self.snapshot_retry_count = 0;

        self.events.push_back(NodeEvent::Joined {
            player_id: assigned_player,
            host_id,
            epoch,
            awaiting_snapshot: snapshot_id.is_some(),
        });
    }

    fn on_submit_command(
        &mut self,
        from: PeerAddress,
        epoch: Epoch,
        seq: CommandSeq,
        command: Command,
    ) {
        if self.role != PlayerRole::Host || epoch != self.session.epoch {
            return;
        }
        if command.is_system_membership() {
            return;
        }
        let Some(track) = self.host_peers_by_addr.get_mut(&from) else {
            return;
        };
        if !matches!(track.status, PeerStatus::Ready) {
            return;
        }
        if let Some(prev) = track.last_command_seq
            && seq <= prev
        {
            return;
        }
        let player_id = track.player_id;
        if !self.enqueue_host_command(player_id, command) {
            return;
        }
        if let Some(track) = self.host_peers_by_addr.get_mut(&from) {
            track.last_command_seq = Some(seq);
        }
    }

    fn on_tick_batch<T: Transport>(
        &mut self,
        transport: &mut T,
        from: PeerAddress,
        batch: TickBatch,
    ) {
        if self.role != PlayerRole::Peer {
            return;
        }
        if self.peer_phase != PeerConnectionPhase::Joined {
            return;
        }
        if self.host_addr.as_ref() != Some(&from) {
            return;
        }
        if batch.epoch != self.session.epoch {
            return;
        }

        // Check host-supplied fingerprint against our local history.
        if let Some((fp_tick, host_fp)) = batch.host_fingerprint
            && let Some(&local_fp) = self.local_fingerprints.get(&fp_tick)
            && local_fp != host_fp
        {
            if let Some(local_id) = self.local_player_id {
                self.events.push_back(NodeEvent::DriftDetected {
                    player_id: local_id,
                    tick: fp_tick,
                    expected: host_fp,
                    actual: local_fp,
                });
            }
            if !self.resync_requested {
                self.resync_requested = true;
                let _ = self.enqueue_peer_to_host(
                    transport,
                    WireMessage::RequestSnapshot {
                        epoch: self.session.epoch,
                        reason: ResyncReason::DriftDetected,
                        last_known_tick: self.session.current_tick,
                    },
                );
            }
        }

        if batch.tick > self.session.current_tick {
            let max_buffered = self.session.config.max_buffered_batches();
            if self.pending_batches.len() >= max_buffered
                && !self.pending_batches.contains_key(&batch.tick)
            {
                if self.is_syncing() {
                    // Syncing peer fell too far behind to catch up via buffered batches.
                    self.pending_batches.clear();
                    self.active_snapshot_rx = None;
                    self.pending_snapshot_install = None;
                    self.awaiting_snapshot_id = None;
                    self.retry_or_abort_snapshot(transport, "syncing tick buffer overflow");
                }
                return;
            }
            self.pending_batches.entry(batch.tick).or_insert(batch);
        }
    }

    fn on_tick_ack(
        &mut self,
        from: PeerAddress,
        epoch: Epoch,
        tick: Tick,
        sim_step_us: u32,
        fingerprint: Option<StateFingerprint>,
    ) {
        if self.role != PlayerRole::Host || epoch != self.session.epoch {
            return;
        }
        let Some(track) = self.host_peers_by_addr.get_mut(&from) else {
            return;
        };
        // Reject forged future ACKs beyond what was actually sent to this peer,
        // and ignore stale out-of-order ACKs before updating pacing state.
        if tick > track.last_sent_tick || tick < track.last_acked_tick {
            return;
        }
        track.last_acked_tick = tick;
        track.reported_step_us = sim_step_us;
        let player_id = track.player_id;
        let already_syncing = matches!(track.status, PeerStatus::Syncing { .. });

        if let Some(peer_fp) = fingerprint
            && let Some(&host_fp) = self.local_fingerprints.get(&tick)
            && host_fp != peer_fp
        {
            self.events.push_back(NodeEvent::DriftDetected {
                player_id,
                tick,
                expected: host_fp,
                actual: peer_fp,
            });
            if !already_syncing {
                self.queue_host_resync(from, player_id, ResyncReason::DriftDetected);
            }
        }
    }

    fn on_request_snapshot(
        &mut self,
        from: PeerAddress,
        epoch: Epoch,
        reason: ResyncReason,
        _last_known_tick: Tick,
    ) {
        if self.role != PlayerRole::Host || epoch != self.session.epoch {
            return;
        }
        let Some(track) = self.host_peers_by_addr.get(&from) else {
            return;
        };
        // Deduplicate if an unanswered SnapshotNeeded request is already pending
        // for this peer; if an earlier transfer is actively streaming chunks
        // (e.g. peer overflowed or dropped its receiver), cancel the stale stream
        // and queue a fresh snapshot request!
        if let PeerStatus::Syncing { snapshot_id, .. } = track.status {
            if self.pending_snapshots.contains_key(&snapshot_id) {
                return;
            }
            self.active_snapshot_txs.remove(&snapshot_id);
        }
        let player_id = track.player_id;
        self.queue_host_resync(from, player_id, reason);
    }

    fn on_snapshot_header<T: Transport>(
        &mut self,
        transport: &mut T,
        from: PeerAddress,
        header: crate::snapshot::SnapshotHeader,
    ) {
        if self.role != PlayerRole::Peer || self.peer_phase != PeerConnectionPhase::Joined {
            return;
        }
        if self.host_addr.as_ref() != Some(&from) || header.epoch != self.session.epoch {
            return;
        }
        if let Some(expected_id) = self.awaiting_snapshot_id
            && expected_id != header.snapshot_id
        {
            return;
        }
        let sid = header.snapshot_id;
        match SnapshotReceiver::new(header) {
            Ok(rx) => {
                let complete = rx.is_complete();
                self.awaiting_snapshot_id = Some(sid);
                self.active_snapshot_rx = Some(rx);
                if complete {
                    self.finalize_active_snapshot(transport, sid);
                }
            }
            Err(error) => {
                self.events.push_back(NodeEvent::SnapshotFailed {
                    snapshot_id: sid,
                    error,
                });
                self.retry_or_abort_snapshot(transport, "invalid snapshot header");
            }
        }
    }

    fn on_snapshot_chunk<T: Transport>(
        &mut self,
        transport: &mut T,
        from: PeerAddress,
        chunk: crate::snapshot::SnapshotChunk,
    ) {
        if self.role != PlayerRole::Peer || self.peer_phase != PeerConnectionPhase::Joined {
            return;
        }
        if self.host_addr.as_ref() != Some(&from) {
            return;
        }
        let Some(rx) = self.active_snapshot_rx.as_mut() else {
            return;
        };
        let sid = rx.header().snapshot_id;
        match rx.push_chunk(chunk) {
            Ok(complete) => {
                let received = rx.received_chunks();
                // Progress ACK only (`complete: false`): completion ACK is sent
                // ONLY after digest verification AND caller installation via
                // `confirm_snapshot_applied`.
                if let Some(host_addr) = self.host_addr.clone() {
                    let _ = transport.send(
                        &host_addr,
                        WireMessage::SnapshotAck {
                            epoch: self.session.epoch,
                            snapshot_id: sid,
                            received_chunks: received,
                            complete: false,
                        },
                    );
                }
                if complete {
                    self.finalize_active_snapshot(transport, sid);
                }
            }
            Err(error) => {
                self.active_snapshot_rx = None;
                self.awaiting_snapshot_id = None;
                self.events.push_back(NodeEvent::SnapshotFailed {
                    snapshot_id: sid,
                    error,
                });
                self.retry_or_abort_snapshot(transport, "invalid snapshot chunk");
            }
        }
    }

    fn finalize_active_snapshot<T: Transport>(&mut self, transport: &mut T, sid: SnapshotId) {
        let Some(rx) = self.active_snapshot_rx.take() else {
            return;
        };
        let total_chunks = rx.header().total_chunks;
        match rx.finish_envelope() {
            Ok(completed) => {
                if let Err(cfg_err) = completed.envelope.session.config.validate() {
                    self.awaiting_snapshot_id = None;
                    self.events.push_back(NodeEvent::SnapshotFailed {
                        snapshot_id: sid,
                        error: SnapshotError::Envelope(crate::error::ProtocolError::Decode(
                            cfg_err.to_string(),
                        )),
                    });
                    self.retry_or_abort_snapshot(transport, "invalid session config in snapshot");
                    return;
                }
                let at_tick = completed.at_tick;
                let state_fingerprint = completed.state_fingerprint;
                // Keep peer syncing until caller installs `redb_bytes` and calls
                // `confirm_snapshot_applied`.
                self.pending_snapshot_install = Some(PendingSnapshotInstall {
                    snapshot_id: sid,
                    at_tick,
                    session: completed.envelope.session,
                    total_chunks,
                });
                self.events.push_back(NodeEvent::ApplySnapshot {
                    snapshot_id: sid,
                    at_tick,
                    state_fingerprint,
                    redb_bytes: completed.envelope.redb_bytes,
                });
            }
            Err(error) => {
                self.awaiting_snapshot_id = None;
                self.events.push_back(NodeEvent::SnapshotFailed {
                    snapshot_id: sid,
                    error,
                });
                // Keep peer syncing and re-request a valid snapshot instead of
                // unblocking stale state!
                self.retry_or_abort_snapshot(transport, "snapshot verification failed");
            }
        }
    }

    fn retry_or_abort_snapshot<T: Transport>(&mut self, transport: &mut T, reason: &str) {
        self.resync_requested = true;
        self.awaiting_snapshot_id = None;
        self.snapshot_retry_count = self.snapshot_retry_count.saturating_add(1);
        if self.snapshot_retry_count > MAX_SNAPSHOT_RETRIES {
            if let Some(host_addr) = self.host_addr.clone() {
                let _ = transport.disconnect(&host_addr);
            }
            self.events.push_back(NodeEvent::SessionEnded {
                reason: format!("snapshot recovery failed after retries: {reason}"),
            });
            return;
        }
        let _ = self.enqueue_peer_to_host(
            transport,
            WireMessage::RequestSnapshot {
                epoch: self.session.epoch,
                reason: ResyncReason::DriftDetected,
                last_known_tick: self.session.current_tick,
            },
        );
    }

    fn on_snapshot_ack(
        &mut self,
        from: PeerAddress,
        epoch: Epoch,
        snapshot_id: SnapshotId,
        complete: bool,
    ) {
        if self.role != PlayerRole::Host || epoch != self.session.epoch || !complete {
            return;
        }
        if let Some(track) = self.host_peers_by_addr.get_mut(&from)
            && let PeerStatus::Syncing {
                snapshot_id: sid,
                snapshot_tick,
                ..
            } = track.status
            && sid == snapshot_id
        {
            track.status = PeerStatus::Ready;
            let at = snapshot_tick.unwrap_or(self.session.current_tick);
            track.last_acked_tick = at;
            if track.last_sent_tick < at {
                track.last_sent_tick = at;
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    fn on_host_takeover<T: Transport>(
        &mut self,
        transport: &mut T,
        from: PeerAddress,
        new_epoch: Epoch,
        new_host: PlayerId,
        resume_tick: Tick,
        resume_fingerprint: Option<StateFingerprint>,
        new_host_session: SessionState,
    ) {
        if new_host_session.config.validate().is_err() {
            return;
        }
        // Split-brain resolution: if we promoted to Host during an asymmetric
        // partition, a strictly higher epoch OR an equal epoch with a lower
        // authenticated `PlayerId` deterministically wins; demote cleanly back
        // to Peer and resync via snapshot!
        if self.role == PlayerRole::Host {
            let higher_authority = new_epoch > self.session.epoch
                || (new_epoch == self.session.epoch && new_host < self.session.host_id);
            if !higher_authority {
                return;
            }
            let sender_authenticated = new_host_session
                .players
                .get(&new_host)
                .is_some_and(|p| peer_matches_player(&from, p))
                && self
                    .session
                    .players
                    .get(&new_host)
                    .is_none_or(|p| peer_matches_player(&from, p));
            if !sender_authenticated {
                return;
            }
            self.role = PlayerRole::Peer;
            self.host_peers_by_addr.clear();
            self.host_pending_commands.clear();
            self.host_peer_queued_counts.clear();
            self.host_system_commands.clear();
            self.pending_snapshots.clear();
            self.active_snapshot_txs.clear();
            self.host_addr = Some(from);
            self.peer_phase = PeerConnectionPhase::Joined;
            self.failed_host_candidates.clear();
            self.peer_outbound.clear();
            self.pending_batches.clear();
            self.session.epoch = new_epoch;
            self.session.host_id = new_host;
            self.awaiting_snapshot_id = None;
            self.active_snapshot_rx = None;
            self.pending_snapshot_install = None;
            self.resync_requested = true;
            self.snapshot_retry_count = 0;
            let _ = self.enqueue_peer_to_host(
                transport,
                WireMessage::RequestSnapshot {
                    epoch: new_epoch,
                    reason: ResyncReason::HostFailover,
                    last_known_tick: self.session.current_tick,
                },
            );
            self.events.push_back(NodeEvent::HostChanged {
                new_host,
                new_epoch,
            });
            return;
        }

        // Only accept failover from the deterministically expected host and epoch,
        // AND verify the authenticated transport identity of `new_host`.
        let expected_next_host = match self.peer_phase {
            PeerConnectionPhase::AwaitingFailover {
                expected_host,
                expected_epoch,
            } => {
                if new_host != expected_host || new_epoch != expected_epoch {
                    return;
                }
                Some(expected_host)
            }
            PeerConnectionPhase::Joined => {
                if new_epoch == self.session.epoch.next()
                    && self
                        .session
                        .select_next_host_excluding(&self.failed_host_candidates)
                        == Some(new_host)
                {
                    Some(new_host)
                } else {
                    None
                }
            }
            PeerConnectionPhase::Connecting => None,
        };
        let Some(expected_host) = expected_next_host else {
            return;
        };
        let sender_authenticated = self
            .session
            .players
            .get(&expected_host)
            .is_some_and(|p| peer_matches_player(&from, p));
        if !sender_authenticated {
            return;
        }

        let old_host = self.session.host_id;
        self.host_addr = Some(from);
        self.peer_phase = PeerConnectionPhase::Joined;
        self.failed_host_candidates.clear();
        self.peer_outbound.clear();
        // Discard any stale old-epoch batches.
        self.pending_batches.retain(|_, b| b.epoch >= new_epoch);

        // Verify whether the no-snapshot same-tick continuation shortcut is safe:
        // must match tick, adopted SessionState fingerprint, AND (when available)
        // the world+session state fingerprint at `resume_tick`.
        let mut candidate_session = self.session.clone();
        let adopt_ok = candidate_session
            .adopt_failover(new_epoch, new_host, Some(old_host))
            .is_ok();
        let session_matches = adopt_ok
            && candidate_session.current_tick == resume_tick
            && candidate_session.session_fingerprint() == new_host_session.session_fingerprint();
        let world_fp_matches = match resume_fingerprint {
            Some(host_fp) => self.local_fingerprints.get(&resume_tick) == Some(&host_fp),
            None => true,
        };

        if session_matches && world_fp_matches {
            self.session = candidate_session;
            let _ = self.enqueue_peer_to_host(
                transport,
                WireMessage::TickAck {
                    epoch: new_epoch,
                    tick: self.session.current_tick,
                    sim_step_us: self.local_step_us,
                    fingerprint: self.local_fingerprints.get(&resume_tick).copied(),
                },
            );
        } else {
            // Tick or state mismatch across failover -> fence epoch, pause
            // execution, and request an atomic REDB snapshot from the new host.
            let _ = self
                .session
                .adopt_failover(new_epoch, new_host, Some(old_host));
            self.awaiting_snapshot_id = None;
            self.active_snapshot_rx = None;
            self.pending_snapshot_install = None;
            self.resync_requested = true;
            self.snapshot_retry_count = 0;
            let _ = self.enqueue_peer_to_host(
                transport,
                WireMessage::RequestSnapshot {
                    epoch: new_epoch,
                    reason: ResyncReason::HostFailover,
                    last_known_tick: self.session.current_tick,
                },
            );
        }

        self.events.push_back(NodeEvent::HostChanged {
            new_host,
            new_epoch,
        });
    }

    fn on_heartbeat<T: Transport>(
        &mut self,
        transport: &mut T,
        from: PeerAddress,
        epoch: Epoch,
        _tick: Tick,
        sender: Option<PlayerId>,
    ) {
        if self.role == PlayerRole::Host && epoch == self.session.epoch {
            if self.host_peers_by_addr.contains_key(&from) {
                return;
            }
            // Authenticate strictly against `PerPlayerState` transport identity;
            // never trust an unauthenticated `sender` ID alone.
            let matched = self
                .session
                .players
                .values()
                .find(|p| {
                    p.player_id != self.session.host_id
                        && peer_matches_player(&from, p)
                        && sender.is_none_or(|claimed| claimed == p.player_id)
                })
                .cloned();
            if let Some(player) = matched {
                self.register_failover_peer(transport, from, player.player_id);
            }
        }
    }

    fn on_goodbye<T: Transport>(
        &mut self,
        transport: &mut T,
        from: PeerAddress,
        epoch: Epoch,
        reason: LeaveReason,
    ) {
        if epoch != self.session.epoch {
            return;
        }
        match self.role {
            PlayerRole::Host => {
                self.remove_host_peer(from, reason);
            }
            PlayerRole::Peer => {
                if self
                    .host_addr
                    .as_ref()
                    .is_some_and(|h| peer_matches_hint(&from, h.as_str()))
                {
                    self.initiate_host_failover(transport);
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Failover, outbound pipeline & helper routines
    // -------------------------------------------------------------------------

    fn initiate_host_failover<T: Transport>(&mut self, transport: &mut T) {
        if self.role != PlayerRole::Peer {
            return;
        }
        if self.local_player_id.is_none() {
            self.events.push_back(NodeEvent::SessionEnded {
                reason: "host disconnected before join completed".to_string(),
            });
            return;
        }
        // Immediately fence all unexecuted old-epoch batches when failover starts.
        let old_epoch = self.session.epoch;
        self.pending_batches.retain(|_, b| b.epoch > old_epoch);
        self.failed_host_candidates.clear();
        self.advance_failover_candidate(transport);
    }

    fn advance_failover_candidate<T: Transport>(&mut self, transport: &mut T) {
        let Some(local_id) = self.local_player_id else {
            self.events.push_back(NodeEvent::SessionEnded {
                reason: "host disconnected before join completed".to_string(),
            });
            return;
        };

        loop {
            let Some(next_host) = self
                .session
                .select_next_host_excluding(&self.failed_host_candidates)
            else {
                self.events.push_back(NodeEvent::SessionEnded {
                    reason: "no surviving host candidate".to_string(),
                });
                return;
            };

            let new_epoch = self.session.epoch.next();
            if next_host == local_id {
                // Promote at the already-executed simulation tick so SessionState
                // and World stay atomically aligned; discard old-epoch batches.
                self.pending_batches.clear();
                // Remove any failed candidates that were unreachable ahead of us.
                let unreachable: Vec<PlayerId> =
                    self.failed_host_candidates.iter().copied().collect();
                if self.session.promote_host(local_id).is_err() {
                    self.events.push_back(NodeEvent::SessionEnded {
                        reason: "failed to promote local host".to_string(),
                    });
                    return;
                }
                for gone in unreachable {
                    self.session.players.remove(&gone);
                }
                self.role = PlayerRole::Host;
                self.host_addr = None;
                self.peer_phase = PeerConnectionPhase::Joined;
                self.failed_host_candidates.clear();
                self.primed = true;
                self.awaiting_snapshot_id = None;
                self.active_snapshot_rx = None;
                self.pending_snapshot_install = None;
                self.resync_requested = false;
                self.snapshot_retry_count = 0;
                self.host_next_player_id = self.session.next_player_id;
                self.next_schedule_tick = self.session.current_tick.next();

                // Notify any already-connected surviving peers immediately; unconnected
                // surviving peers proactively dial the newly promoted host.
                let surviving: Vec<(PlayerId, Option<String>)> = self
                    .session
                    .players
                    .values()
                    .filter(|p| p.player_id != local_id)
                    .map(|p| {
                        (
                            p.player_id,
                            p.transport_addr.clone().or_else(|| {
                                p.endpoint_hint
                                    .as_deref()
                                    .map(canonical_addr_string_from_hint)
                            }),
                        )
                    })
                    .collect();

                for (pid, addr_opt) in surviving {
                    if let Some(addr_str) = addr_opt {
                        let canonical = PeerAddress(addr_str);
                        if transport.connected_peers().contains(&canonical) {
                            self.register_failover_peer(transport, canonical, pid);
                        }
                    }
                }

                self.events.push_back(NodeEvent::HostPromoted {
                    new_host: local_id,
                    new_epoch: self.session.epoch,
                });
                return;
            }

            // Candidate is another player: check if we have a dialable hint/address.
            let dial_hint = self
                .session
                .players
                .get(&next_host)
                .and_then(|p| p.endpoint_hint.clone().or_else(|| p.transport_addr.clone()));
            let Some(hint) = dial_hint else {
                // No reachable address for this candidate; advance deterministically.
                self.failed_host_candidates.insert(next_host);
                continue;
            };

            let dial_addr = PeerAddress(hint.clone());
            let canonical = canonical_addr_from_hint(&hint);
            self.peer_phase = PeerConnectionPhase::AwaitingFailover {
                expected_host: next_host,
                expected_epoch: new_epoch,
            };
            self.host_addr = Some(canonical.clone());

            if transport.connected_peers().contains(&canonical) {
                if transport
                    .send(
                        &canonical,
                        WireMessage::Heartbeat {
                            epoch: new_epoch,
                            tick: self.session.current_tick,
                            sender: self.local_player_id,
                        },
                    )
                    .is_err()
                {
                    self.failed_host_candidates.insert(next_host);
                    continue;
                }
                return;
            }

            // Synchronous connect errors (e.g. crashed MemoryMesh node) immediately
            // advance to the next deterministic candidate!
            if transport.connect(&dial_addr).is_err() {
                self.failed_host_candidates.insert(next_host);
                continue;
            }
            return;
        }
    }

    fn register_failover_peer<T: Transport>(
        &mut self,
        transport: &mut T,
        addr: PeerAddress,
        player_id: PlayerId,
    ) {
        self.host_peers_by_addr
            .entry(addr.clone())
            .or_insert_with(|| HostPeerTrack {
                player_id,
                status: PeerStatus::Ready,
                join_lifecycle: JoinLifecycle::Active,
                last_sent_tick: self.session.current_tick,
                last_acked_tick: self.session.current_tick,
                last_command_seq: None,
                reported_step_us: 0,
                outbound: VecDeque::new(),
            });
        let resume_tick = self.session.current_tick;
        let takeover = WireMessage::HostTakeover {
            new_epoch: self.session.epoch,
            new_host: self.session.host_id,
            resume_tick,
            resume_fingerprint: self.local_fingerprints.get(&resume_tick).copied(),
            session_state: Box::new(self.session.clone()),
            roster: self.build_host_roster(),
        };
        self.enqueue_peer_message(transport, &addr, takeover);
    }

    fn remove_host_peer(&mut self, peer: PeerAddress, reason: LeaveReason) {
        let Some(track) = self.host_peers_by_addr.remove(&peer) else {
            return;
        };
        let pid = track.player_id;
        self.host_pending_commands.retain(|(p, _)| *p != pid);
        self.host_peer_queued_counts.remove(&pid);
        self.pending_snapshots.retain(|_, req| req.peer != peer);
        self.active_snapshot_txs.retain(|_, tx| tx.peer != peer);

        match track.join_lifecycle {
            JoinLifecycle::PendingSnapshot { .. } => {
                // `PlayerJoined` was never queued; clean drop with zero ghost player.
            }
            JoinLifecycle::JoinQueued => {
                // Cancel the unscheduled `PlayerJoined` in `host_system_commands`.
                self.host_system_commands.retain(|cmd| {
                    !matches!(cmd, Command::PlayerJoined { player_id, .. } if *player_id == pid)
                });
            }
            JoinLifecycle::JoinStamped | JoinLifecycle::Active => {
                // Already stamped or active: sequence `PlayerLeft` after it.
                self.host_system_commands.push_back(Command::PlayerLeft {
                    player_id: pid,
                    reason,
                });
            }
        }
        self.events.push_back(NodeEvent::PeerDisconnected {
            peer,
            player_id: pid,
            reason,
        });
    }

    fn enqueue_peer_message<T: Transport>(
        &mut self,
        transport: &mut T,
        peer: &PeerAddress,
        msg: WireMessage,
    ) {
        let overflowed = if let Some(track) = self.host_peers_by_addr.get_mut(peer) {
            if track.outbound.len() >= MAX_PEER_OUTBOUND_QUEUE {
                true
            } else {
                track.outbound.push_back(msg);
                false
            }
        } else {
            return;
        };
        if overflowed {
            let _ = transport.disconnect(peer);
            self.remove_host_peer(peer.clone(), LeaveReason::Disconnected);
            return;
        }
        self.flush_peer_outbound(transport, peer);
    }

    fn flush_peer_outbound<T: Transport>(&mut self, transport: &mut T, peer: &PeerAddress) {
        let mut disconnect_peer = false;
        if let Some(track) = self.host_peers_by_addr.get_mut(peer) {
            while let Some(msg) = track.outbound.pop_front() {
                match transport.send(peer, msg.clone()) {
                    Ok(()) => {}
                    Err(TransportError::Backpressure(_)) => {
                        track.outbound.push_front(msg);
                        break;
                    }
                    Err(_) => {
                        disconnect_peer = true;
                        break;
                    }
                }
            }
        }
        if disconnect_peer {
            let _ = transport.disconnect(peer);
            self.remove_host_peer(peer.clone(), LeaveReason::Disconnected);
        }
    }

    fn flush_all_peer_outbound<T: Transport>(&mut self, transport: &mut T) {
        let peers: Vec<PeerAddress> = self.host_peers_by_addr.keys().cloned().collect();
        for peer in peers {
            self.flush_peer_outbound(transport, &peer);
        }
    }

    fn pump_snapshot_txs<T: Transport>(&mut self, transport: &mut T) {
        let sids: Vec<SnapshotId> = self.active_snapshot_txs.keys().copied().collect();
        let mut completed = Vec::new();
        for sid in sids {
            let Some(mut tx) = self.active_snapshot_txs.remove(&sid) else {
                continue;
            };
            if !self.host_peers_by_addr.contains_key(&tx.peer) {
                continue;
            }
            // First flush existing outbound frames so we have maximum room.
            self.flush_peer_outbound(transport, &tx.peer);
            if !self.host_peers_by_addr.contains_key(&tx.peer) {
                continue;
            }

            if !tx.header_sent {
                let can_queue = self
                    .host_peers_by_addr
                    .get(&tx.peer)
                    .is_some_and(|t| t.outbound.len() < SNAPSHOT_OUTBOUND_HIGH_WATER);
                if can_queue {
                    let hdr = tx.sender.header().clone();
                    tx.header_sent = true;
                    self.enqueue_peer_message(
                        transport,
                        &tx.peer,
                        WireMessage::SnapshotHeader(hdr),
                    );
                }
            }

            if tx.header_sent {
                let total_chunks = tx.sender.header().total_chunks;
                while tx.next_chunk < total_chunks {
                    let can_queue = self
                        .host_peers_by_addr
                        .get(&tx.peer)
                        .is_some_and(|t| t.outbound.len() < SNAPSHOT_OUTBOUND_HIGH_WATER);
                    if !can_queue {
                        break;
                    }
                    let Some(chunk) = tx.sender.chunk(tx.next_chunk) else {
                        break;
                    };
                    tx.next_chunk = tx.next_chunk.saturating_add(1);
                    self.enqueue_peer_message(
                        transport,
                        &tx.peer,
                        WireMessage::SnapshotChunk(chunk),
                    );
                    if !self.host_peers_by_addr.contains_key(&tx.peer) {
                        break;
                    }
                }
                if tx.next_chunk >= total_chunks {
                    completed.push(sid);
                    continue;
                }
            }

            if self.host_peers_by_addr.contains_key(&tx.peer) {
                self.active_snapshot_txs.insert(sid, tx);
            }
        }
        let _ = completed;
    }

    fn queue_host_resync(&mut self, peer: PeerAddress, player_id: PlayerId, reason: ResyncReason) {
        let sid = self.allocate_snapshot_id();
        if let Some(track) = self.host_peers_by_addr.get_mut(&peer) {
            track.status = PeerStatus::Syncing {
                snapshot_id: sid,
                reason,
                snapshot_tick: None,
            };
        }
        self.pending_snapshots.insert(
            sid,
            PendingSnapshotRequest {
                peer: peer.clone(),
                player_id,
                reason,
            },
        );
        self.events.push_back(NodeEvent::SnapshotNeeded {
            peer,
            player_id,
            snapshot_id: sid,
            reason,
        });
    }

    fn enqueue_peer_to_host<T: Transport>(
        &mut self,
        transport: &mut T,
        msg: WireMessage,
    ) -> Result<(), SessionError> {
        let Some(host_addr) = self.host_addr.clone() else {
            return Err(SessionError::NotPeer);
        };
        if self.peer_outbound.len() >= MAX_PEER_OUTBOUND_QUEUE {
            return Err(TransportError::Backpressure(host_addr.0).into());
        }
        self.peer_outbound.push_back(msg);
        self.flush_peer_to_host(transport);
        Ok(())
    }

    fn flush_peer_to_host<T: Transport>(&mut self, transport: &mut T) {
        let Some(host_addr) = self.host_addr.clone() else {
            return;
        };
        while let Some(msg) = self.peer_outbound.pop_front() {
            match transport.send(&host_addr, msg.clone()) {
                Ok(()) => {}
                Err(TransportError::Backpressure(_)) => {
                    self.peer_outbound.push_front(msg);
                    break;
                }
                Err(_) => {
                    // Preserve un-sent message only if still connected; otherwise clear.
                    if transport.connected_peers().contains(&host_addr) {
                        self.peer_outbound.push_front(msg);
                    }
                    break;
                }
            }
        }
    }

    fn enqueue_host_command(&mut self, player_id: PlayerId, command: Command) -> bool {
        let max_queued = self
            .session
            .config
            .max_commands_per_peer_per_tick
            .saturating_mul(4)
            .max(32);
        let current = self
            .host_peer_queued_counts
            .get(&player_id)
            .copied()
            .unwrap_or(0);
        if current >= max_queued {
            return false;
        }
        self.host_peer_queued_counts.insert(player_id, current + 1);
        self.host_pending_commands.push_back((player_id, command));
        true
    }

    fn allocate_provisional_player_id(&mut self) -> PlayerId {
        if self.host_next_player_id < self.session.next_player_id {
            self.host_next_player_id = self.session.next_player_id;
        }
        let id = PlayerId(self.host_next_player_id);
        self.host_next_player_id = self.host_next_player_id.saturating_add(1);
        id
    }

    fn allocate_snapshot_id(&mut self) -> SnapshotId {
        let id = SnapshotId(self.next_snapshot_id);
        self.next_snapshot_id = self.next_snapshot_id.saturating_add(1);
        id
    }

    fn reserved_unapplied_joins(&self) -> usize {
        self.host_peers_by_addr
            .values()
            .filter(|t| !self.session.players.contains_key(&t.player_id))
            .count()
    }

    fn contiguous_buffered_len(&self, start: Tick) -> usize {
        let mut count = 0usize;
        let mut cursor = start;
        while self.pending_batches.contains_key(&cursor) {
            count += 1;
            cursor = cursor.next();
        }
        count
    }

    fn compute_host_pacing_ms(&self) -> u32 {
        let base_period = self.session.time.speed.period_ms().max(1);
        let mut slowest_us = self.local_step_us;
        for track in self.host_peers_by_addr.values() {
            if matches!(track.status, PeerStatus::Ready) {
                slowest_us = slowest_us.max(track.reported_step_us);
            }
        }
        // Convert microseconds to milliseconds with 25% headroom (integer math).
        let slowest_ms = slowest_us.saturating_mul(5).div_ceil(4000);
        let target = base_period.max(slowest_ms);
        // Deterministic integer EWMA: (3 * prev + target) / 4, bounded below by target
        // when slowing down so we never outrun the slowest machine.
        if target >= self.smoothed_pacing_ms {
            target
        } else {
            (self
                .smoothed_pacing_ms
                .saturating_mul(3)
                .saturating_add(target))
                / 4
        }
    }

    fn record_local_fingerprint(&mut self, tick: Tick, fp: StateFingerprint) {
        self.local_fingerprints.insert(tick, fp);
        let cap = self.session.config.retained_batch_history.max(16);
        while self.local_fingerprints.len() > cap {
            self.local_fingerprints.pop_first();
        }
    }

    fn record_retained_batch(&mut self, batch: TickBatch) {
        let cap = self.session.config.retained_batch_history;
        if cap == 0 {
            return;
        }
        self.retained_batches.insert(batch.tick, batch);
        while self.retained_batches.len() > cap {
            self.retained_batches.pop_first();
        }
    }

    fn build_host_roster(&self) -> Vec<PeerRosterEntry> {
        let mut acks: BTreeMap<PlayerId, (Tick, u32)> = BTreeMap::new();
        for track in self.host_peers_by_addr.values() {
            acks.insert(
                track.player_id,
                (track.last_acked_tick, track.reported_step_us),
            );
        }
        self.session
            .players
            .values()
            .map(|p| {
                let (last_acked_tick, reported_step_us) = acks
                    .get(&p.player_id)
                    .copied()
                    .unwrap_or((self.session.current_tick, self.local_step_us));
                PeerRosterEntry {
                    player_id: p.player_id,
                    name: p.name.clone(),
                    role: p.role,
                    endpoint_hint: p.endpoint_hint.clone(),
                    mode: p.mode,
                    selected_sim: p.selected_sim,
                    camera_rotation: p.camera_rotation.normalized(),
                    last_acked_tick,
                    reported_step_us,
                }
            })
            .collect()
    }
}

pub(crate) fn canonical_addr_string_from_hint(hint: &str) -> String {
    if hint.trim().starts_with(INVITE_TICKET_PREFIX)
        && let Ok(ticket) = InviteTicket::decode_string(hint)
    {
        return ticket.endpoint_addr().id.to_string();
    }
    hint.to_string()
}

fn canonical_addr_from_hint(hint: &str) -> PeerAddress {
    PeerAddress(canonical_addr_string_from_hint(hint))
}

fn peer_matches_hint(peer: &PeerAddress, hint: &str) -> bool {
    peer.as_str() == hint || canonical_addr_from_hint(hint) == *peer
}

fn peer_matches_player(peer: &PeerAddress, player: &PerPlayerState) -> bool {
    if let Some(ref auth_addr) = player.transport_addr {
        return auth_addr == peer.as_str();
    }
    player
        .endpoint_hint
        .as_deref()
        .is_some_and(|hint| peer_matches_hint(peer, hint))
}
