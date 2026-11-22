//! Host-sequenced lockstep scheduler (`LockstepNode`).
//!
//! Drives both host and peer roles deterministically over any [`Transport`]:
//! - Host-sequenced `TickBatch` scheduling and broadcast.
//! - Configurable peer tick buffer delay (`config.tick_delay`).
//! - Adaptive pacing to the slowest machine (bounded host lead + integer-only
//!   smoothed tick period hints).
//! - Periodic state fingerprint comparison & automatic drift detection.
//! - Chunked/compressed REDB snapshot transfer for mid-game join and drift
//!   recovery.
//! - Install & custom-content [`ManifestDigest`] verification on join.
//! - Deterministic host failover with epoch fencing and catch-up.

use crate::error::{SessionError, SnapshotError};
use crate::protocol::{
    CameraRotation, Command, CommandSeq, DEFAULT_SNAPSHOT_CHUNK_BYTES, Epoch, LeaveReason,
    ManifestDigest, PROTOCOL_VERSION, PeerRosterEntry, PlayerId, PlayerRole, RejectReason,
    ResyncReason, SessionConfig, SimSpeed, SnapshotId, StampedCommand, StateFingerprint, Tick,
    TickBatch, WireMessage,
};
use crate::session::state::{AppliedBatch, SessionState};
use crate::snapshot::{SnapshotCompression, SnapshotEnvelope, SnapshotReceiver, SnapshotSender};
use crate::transport::{
    INVITE_TICKET_PREFIX, InviteTicket, PeerAddress, Transport, TransportEvent,
};
use std::collections::{BTreeMap, VecDeque};

/// Outcome of calling [`LockstepNode::advance_host_tick`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostStepResult {
    /// Host scheduled and broadcast a new [`TickBatch`].
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
    /// Peer finished receiving and verifying a REDB snapshot and restored its
    /// [`SessionState`] at `at_tick`. The caller must load `redb_bytes` into
    /// its simulation world before running subsequent ticks.
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
    /// The session ended (e.g. no remaining host candidate after disconnect).
    SessionEnded { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PeerStatus {
    Ready,
    Syncing {
        snapshot_id: SnapshotId,
        reason: ResyncReason,
        snapshot_tick: Option<Tick>,
        pending_join_name: Option<String>,
        pending_endpoint_hint: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct HostPeerTrack {
    player_id: PlayerId,
    status: PeerStatus,
    last_acked_tick: Tick,
    last_command_seq: Option<CommandSeq>,
    reported_step_us: u32,
}

#[derive(Debug, Clone)]
struct PendingSnapshotRequest {
    peer: PeerAddress,
    player_id: PlayerId,
    reason: ResyncReason,
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
    next_schedule_tick: Tick,
    next_snapshot_id: u64,
    host_peers_by_addr: BTreeMap<PeerAddress, HostPeerTrack>,
    host_pending_commands: BTreeMap<PlayerId, VecDeque<Command>>,
    host_system_commands: VecDeque<Command>,
    pending_snapshots: BTreeMap<SnapshotId, PendingSnapshotRequest>,
    smoothed_pacing_ms: u32,

    // Peer-side state.
    host_addr: Option<PeerAddress>,
    peer_phase: PeerConnectionPhase,
    next_command_seq: CommandSeq,
    active_snapshot_rx: Option<SnapshotReceiver>,
    awaiting_snapshot_id: Option<SnapshotId>,
    resync_requested: bool,
    hello_sent: bool,

    // Emitted events drained by the caller.
    events: VecDeque<NodeEvent>,
}

impl LockstepNode {
    /// Creates a new host node at tick `0` (epoch `1`, `PlayerId(1)`).
    pub fn new_host(
        host_name: impl Into<String>,
        host_endpoint_hint: Option<String>,
        manifest: ManifestDigest,
        config: SessionConfig,
    ) -> Self {
        let host_name = host_name.into();
        let initial_period = SimSpeed::NORMAL.period_ms();
        let target_delay = config
            .tick_delay
            .clamp(config.min_tick_delay, config.max_tick_delay);
        let session = SessionState::new(host_name.clone(), host_endpoint_hint.clone(), config);
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
            next_schedule_tick: Tick(1),
            next_snapshot_id: 1,
            host_peers_by_addr: BTreeMap::new(),
            host_pending_commands: BTreeMap::new(),
            host_system_commands: VecDeque::new(),
            pending_snapshots: BTreeMap::new(),
            smoothed_pacing_ms: initial_period,
            host_addr: None,
            peer_phase: PeerConnectionPhase::Joined,
            next_command_seq: CommandSeq(1),
            active_snapshot_rx: None,
            awaiting_snapshot_id: None,
            resync_requested: false,
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
            next_schedule_tick: Tick(1),
            next_snapshot_id: 1,
            host_peers_by_addr: BTreeMap::new(),
            host_pending_commands: BTreeMap::new(),
            host_system_commands: VecDeque::new(),
            pending_snapshots: BTreeMap::new(),
            smoothed_pacing_ms: SimSpeed::NORMAL.period_ms(),
            host_addr: Some(host_addr),
            peer_phase: PeerConnectionPhase::Connecting,
            next_command_seq: CommandSeq(1),
            active_snapshot_rx: None,
            awaiting_snapshot_id: None,
            resync_requested: false,
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

    /// Whether this peer is currently receiving or waiting for a REDB snapshot.
    pub fn is_syncing(&self) -> bool {
        self.awaiting_snapshot_id.is_some()
            || self.active_snapshot_rx.is_some()
            || self.resync_requested
    }

    /// Current adaptive pacing period in milliseconds.
    pub const fn pacing_hint_ms(&self) -> u32 {
        self.smoothed_pacing_ms
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
    /// - On the host, queues directly into the next scheduled tick batch.
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
                self.host_pending_commands
                    .entry(id)
                    .or_default()
                    .push_back(command);
                Ok(())
            }
            PlayerRole::Peer => {
                if self.local_player_id.is_none() {
                    return Err(SessionError::UnassignedPlayer);
                }
                let Some(host_addr) = self.host_addr.clone() else {
                    return Err(SessionError::NotPeer);
                };
                let seq = self.next_command_seq;
                self.next_command_seq = seq.next();
                transport.send(
                    &host_addr,
                    WireMessage::SubmitCommand {
                        epoch: self.session.epoch,
                        seq,
                        command,
                    },
                )?;
                Ok(())
            }
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
                let _ = transport.broadcast(msg);
                let addrs: Vec<PeerAddress> = self.host_peers_by_addr.keys().cloned().collect();
                for addr in addrs {
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

    /// Polls `transport` for incoming events and messages, updating protocol
    /// state and queuing [`NodeEvent`]s.
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
    }

    /// Host-only: sequences queued commands into the next [`TickBatch`],
    /// broadcasts it to all connected peers, and buffers it locally.
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
        let mut commands = Vec::new();
        let mut seq = 0u32;

        // 1. Host system membership/failover commands first, in FIFO order.
        let host_id = self.session.host_id;
        while let Some(cmd) = self.host_system_commands.pop_front() {
            commands.push(StampedCommand {
                tick,
                sequence: seq,
                issuer: host_id,
                command: cmd,
            });
            seq = seq.saturating_add(1);
        }

        // 2. Player commands in deterministic `PlayerId` order, bounded per peer.
        let per_peer_cap = self.session.config.max_commands_per_peer_per_tick.max(1);
        let player_ids: Vec<PlayerId> = self.host_pending_commands.keys().copied().collect();
        for player_id in player_ids {
            if let Some(queue) = self.host_pending_commands.get_mut(&player_id) {
                let mut taken = 0usize;
                while taken < per_peer_cap {
                    let Some(cmd) = queue.pop_front() else {
                        break;
                    };
                    commands.push(StampedCommand {
                        tick,
                        sequence: seq,
                        issuer: player_id,
                        command: cmd,
                    });
                    seq = seq.saturating_add(1);
                    taken += 1;
                }
            }
        }
        self.host_pending_commands.retain(|_, q| !q.is_empty());

        let pacing_hint_ms = self.compute_host_pacing_ms();
        self.smoothed_pacing_ms = pacing_hint_ms;

        // Attach the latest host fingerprint once per fingerprint tick.
        let host_fingerprint = self.last_reported_host_fp.take();

        let batch = TickBatch {
            epoch: self.session.epoch,
            tick,
            commands,
            pacing_hint_ms,
            host_fingerprint,
        };

        self.next_schedule_tick = tick.next();
        self.record_retained_batch(batch.clone());
        self.pending_batches.insert(tick, batch.clone());

        // Broadcast to all tracked peers (ready + syncing peers buffer future ticks).
        let recipients: Vec<PeerAddress> = self.host_peers_by_addr.keys().cloned().collect();
        for addr in recipients {
            let _ = transport.send(&addr, WireMessage::TickBatch(batch.clone()));
        }

        Ok(HostStepResult::Scheduled(batch))
    }

    /// Returns whether the next tick (`session.current_tick + 1`) is ready to
    /// execute under the current buffer delay policy.
    ///
    /// When `flush` is `true`, executes any contiguous buffered tick without
    /// waiting for the buffer delay threshold (useful when draining final ticks
    /// or catching up).
    pub fn can_step_tick(&self, flush: bool) -> bool {
        if self.is_syncing() {
            return false;
        }
        let next = self.session.current_tick.next();
        if !self.pending_batches.contains_key(&next) {
            return false;
        }
        if self.role == PlayerRole::Host || flush {
            return true;
        }
        if !self.primed {
            let min_buf = (self.target_buffer_delay as usize).max(1);
            return self.pending_batches.len() >= min_buf;
        }
        true
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
        self.record_retained_batch(batch);
        Ok(Some(applied))
    }

    /// Executes all currently ready contiguous ticks (respecting `flush`).
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
    /// On a fingerprint interval tick (`fingerprint_interval > 0 && tick %
    /// interval == 0`), the fingerprint is stored for drift comparison and
    /// exchanged between peer and host.
    pub fn report_local_tick<T: Transport>(
        &mut self,
        transport: &mut T,
        tick: Tick,
        sim_step_us: u32,
        world_hash: Option<[u8; 32]>,
    ) -> Result<(), SessionError> {
        self.local_step_us = sim_step_us;
        let is_fp_tick = self.session.config.fingerprint_interval > 0
            && tick.0 > 0
            && tick
                .0
                .is_multiple_of(self.session.config.fingerprint_interval);

        let fingerprint = world_hash.map(|wh| self.session.combined_fingerprint(wh));
        if let Some(fp) = fingerprint
            && is_fp_tick
        {
            self.record_local_fingerprint(tick, fp);
            if self.role == PlayerRole::Host {
                self.last_reported_host_fp = Some((tick, fp));
            }
        }

        if self.role == PlayerRole::Peer
            && self.peer_phase == PeerConnectionPhase::Joined
            && let Some(host_addr) = self.host_addr.clone()
        {
            let ack_fp = if is_fp_tick { fingerprint } else { None };
            transport.send(
                &host_addr,
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

    /// Host-only: compresses, chunks, and transmits a REDB snapshot for a
    /// pending [`NodeEvent::SnapshotNeeded`] request.
    ///
    /// For [`ResyncReason::MidGameJoin`], this also enqueues
    /// [`Command::PlayerJoined`] for the next scheduled tick (strictly after
    /// `self.session.current_tick`) so existing peers and the joining peer
    /// apply the join on the exact same tick!
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
            .remove(&snapshot_id)
            .ok_or(SessionError::UnknownSnapshot(snapshot_id))?;

        let at_tick = self.session.current_tick;
        let state_fingerprint = self.session.combined_fingerprint(world_hash);
        let envelope = SnapshotEnvelope {
            session: self.session.clone(),
            redb_bytes,
        };
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

        transport.send(
            &req.peer,
            WireMessage::SnapshotHeader(sender.header().clone()),
        )?;
        for chunk in sender.chunks() {
            transport.send(&req.peer, WireMessage::SnapshotChunk(chunk))?;
        }

        // Record snapshot tick and, for mid-game join, emit `PlayerJoined`
        // strictly after `at_tick` so joiners load `at_tick` and replay identically.
        if let Some(track) = self.host_peers_by_addr.get_mut(&req.peer)
            && let PeerStatus::Syncing {
                snapshot_tick,
                pending_join_name,
                pending_endpoint_hint,
                ..
            } = &mut track.status
        {
            *snapshot_tick = Some(at_tick);
            if req.reason == ResyncReason::MidGameJoin
                && let Some(name) = pending_join_name.take()
            {
                let hint = pending_endpoint_hint.take();
                self.host_system_commands.push_back(Command::PlayerJoined {
                    player_id: req.player_id,
                    name,
                    selected_sim: None,
                    camera_rotation: CameraRotation::NORTH,
                    endpoint_hint: hint,
                });
            }
        }

        Ok(())
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
                } else if let PeerConnectionPhase::AwaitingFailover { expected_epoch, .. } =
                    self.peer_phase
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
                // If a surviving peer reconnected after host failover, send it
                // `HostTakeover` right away if we know its hint.
                if let Some(player) = self
                    .session
                    .players
                    .values()
                    .find(|p| {
                        p.player_id != self.session.host_id
                            && p.endpoint_hint
                                .as_deref()
                                .is_some_and(|hint| peer_matches_hint(&peer, hint))
                    })
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
                if let Some(track) = self.host_peers_by_addr.remove(&peer) {
                    self.host_pending_commands.remove(&track.player_id);
                    self.pending_snapshots.retain(|_, req| req.peer != peer);
                    if self.session.players.contains_key(&track.player_id) {
                        self.host_system_commands.push_back(Command::PlayerLeft {
                            player_id: track.player_id,
                            reason: LeaveReason::Disconnected,
                        });
                    }
                    self.events.push_back(NodeEvent::PeerDisconnected {
                        peer,
                        player_id: track.player_id,
                        reason: LeaveReason::Disconnected,
                    });
                }
            }
            PlayerRole::Peer => {
                if self.host_addr.as_ref() == Some(&peer) {
                    self.initiate_host_failover(transport);
                }
            }
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
                if self.role == PlayerRole::Peer {
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
            WireMessage::SnapshotHeader(header) => self.on_snapshot_header(transport, header),
            WireMessage::SnapshotChunk(chunk) => self.on_snapshot_chunk(transport, chunk),
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
                session_state,
                roster: _,
            } => self.on_host_takeover(
                transport,
                from,
                new_epoch,
                new_host,
                resume_tick,
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
        if player_name.trim().is_empty() {
            let _ = transport.send(
                &from,
                WireMessage::Reject {
                    reason: RejectReason::InvalidPlayerName,
                },
            );
            return;
        }
        if self.host_peers_by_addr.contains_key(&from) {
            return;
        }
        let active_count = self.session.players.len() + self.pending_join_count();
        if active_count >= usize::from(self.session.config.max_players) {
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

        let assigned_player = self.session.allocate_player_id();
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
                        pending_join_name: Some(player_name.clone()),
                        pending_endpoint_hint: endpoint_hint,
                    },
                    last_acked_tick: self.session.current_tick,
                    last_command_seq: None,
                    reported_step_us: 0,
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
                    last_acked_tick: Tick::ZERO,
                    last_command_seq: None,
                    reported_step_us: 0,
                },
            );
            self.host_system_commands.push_back(Command::PlayerJoined {
                player_id: assigned_player,
                name: player_name.clone(),
                selected_sim: None,
                camera_rotation: CameraRotation::NORTH,
                endpoint_hint,
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
        let _ = transport.send(&from, welcome);

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
        self.host_addr = Some(from);
        self.local_player_id = Some(assigned_player);
        self.target_buffer_delay = config
            .tick_delay
            .clamp(config.min_tick_delay, config.max_tick_delay);
        self.session = session_state;
        self.session.epoch = epoch;
        self.session.host_id = host_id;
        self.session.config = config;
        self.peer_phase = PeerConnectionPhase::Joined;
        self.awaiting_snapshot_id = snapshot_id;
        self.primed = snapshot_id.is_none() && self.target_buffer_delay == 0;
        self.resync_requested = snapshot_id.is_some();

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
            // Deduplicate replayed or out-of-order submissions.
            return;
        }
        track.last_command_seq = Some(seq);
        let player_id = track.player_id;
        self.host_pending_commands
            .entry(player_id)
            .or_default()
            .push_back(command);
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
                let _ = transport.send(
                    &from,
                    WireMessage::RequestSnapshot {
                        epoch: self.session.epoch,
                        reason: ResyncReason::DriftDetected,
                        last_known_tick: self.session.current_tick,
                    },
                );
            }
        }

        if batch.tick > self.session.current_tick {
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
        if tick > track.last_acked_tick {
            track.last_acked_tick = tick;
        }
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
        if matches!(track.status, PeerStatus::Syncing { .. }) {
            return;
        }
        let player_id = track.player_id;
        self.queue_host_resync(from, player_id, reason);
    }

    fn on_snapshot_header<T: Transport>(
        &mut self,
        transport: &mut T,
        header: crate::snapshot::SnapshotHeader,
    ) {
        if self.role != PlayerRole::Peer || header.epoch != self.session.epoch {
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
                    if let Some(host_addr) = self.host_addr.clone() {
                        let _ = transport.send(
                            &host_addr,
                            WireMessage::SnapshotAck {
                                epoch: self.session.epoch,
                                snapshot_id: sid,
                                received_chunks: 0,
                                complete: true,
                            },
                        );
                    }
                    self.finalize_active_snapshot(sid);
                }
            }
            Err(error) => {
                self.events.push_back(NodeEvent::SnapshotFailed {
                    snapshot_id: sid,
                    error,
                });
            }
        }
    }

    fn on_snapshot_chunk<T: Transport>(
        &mut self,
        transport: &mut T,
        chunk: crate::snapshot::SnapshotChunk,
    ) {
        if self.role != PlayerRole::Peer {
            return;
        }
        let Some(rx) = self.active_snapshot_rx.as_mut() else {
            return;
        };
        let sid = rx.header().snapshot_id;
        match rx.push_chunk(chunk) {
            Ok(complete) => {
                let received = rx.received_chunks();
                if let Some(host_addr) = self.host_addr.clone() {
                    let _ = transport.send(
                        &host_addr,
                        WireMessage::SnapshotAck {
                            epoch: self.session.epoch,
                            snapshot_id: sid,
                            received_chunks: received,
                            complete,
                        },
                    );
                }
                if complete {
                    self.finalize_active_snapshot(sid);
                }
            }
            Err(error) => {
                self.active_snapshot_rx = None;
                self.awaiting_snapshot_id = None;
                self.resync_requested = false;
                self.events.push_back(NodeEvent::SnapshotFailed {
                    snapshot_id: sid,
                    error,
                });
            }
        }
    }

    fn finalize_active_snapshot(&mut self, sid: SnapshotId) {
        let Some(rx) = self.active_snapshot_rx.take() else {
            return;
        };
        match rx.finish_envelope() {
            Ok(completed) => {
                let at_tick = completed.at_tick;
                let state_fingerprint = completed.state_fingerprint;
                self.session = completed.envelope.session;
                self.session.current_tick = at_tick;
                // Drop buffered batches at or before `at_tick`; keep strictly newer batches.
                self.pending_batches.retain(|&t, _| t > at_tick);
                self.local_fingerprints.clear();
                self.awaiting_snapshot_id = None;
                self.resync_requested = false;
                self.primed = false;
                self.events.push_back(NodeEvent::ApplySnapshot {
                    snapshot_id: sid,
                    at_tick,
                    state_fingerprint,
                    redb_bytes: completed.envelope.redb_bytes,
                });
            }
            Err(error) => {
                self.awaiting_snapshot_id = None;
                self.resync_requested = false;
                self.events.push_back(NodeEvent::SnapshotFailed {
                    snapshot_id: sid,
                    error,
                });
            }
        }
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
            track.last_acked_tick = snapshot_tick.unwrap_or(self.session.current_tick);
        }
    }

    fn on_host_takeover<T: Transport>(
        &mut self,
        transport: &mut T,
        from: PeerAddress,
        new_epoch: Epoch,
        new_host: PlayerId,
        resume_tick: Tick,
        _new_host_session: SessionState,
    ) {
        if self.role != PlayerRole::Peer {
            return;
        }
        // Only accept failover from the deterministically expected host and epoch.
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
                // Allow direct takeover if peer received HostTakeover before
                // noticing the old host's disconnect event.
                if new_epoch == self.session.epoch.next()
                    && self.session.select_next_host() == Some(new_host)
                {
                    Some(new_host)
                } else {
                    None
                }
            }
            PeerConnectionPhase::Connecting => None,
        };
        if expected_next_host.is_none() {
            return;
        }

        let old_host = self.session.host_id;
        self.host_addr = Some(from.clone());
        self.peer_phase = PeerConnectionPhase::Joined;
        // Discard any stale old-epoch batches.
        self.pending_batches.retain(|_, b| b.epoch >= new_epoch);

        if self.session.current_tick == resume_tick {
            let _ = self
                .session
                .adopt_failover(new_epoch, new_host, Some(old_host));
            let _ = transport.send(
                &from,
                WireMessage::TickAck {
                    epoch: new_epoch,
                    tick: self.session.current_tick,
                    sim_step_us: self.local_step_us,
                    fingerprint: None,
                },
            );
        } else {
            // Tick mismatch across failover -> fence epoch, pause execution,
            // and request a full REDB snapshot so SessionState + World restore atomically.
            let _ = self
                .session
                .adopt_failover(new_epoch, new_host, Some(old_host));
            self.awaiting_snapshot_id = None;
            self.active_snapshot_rx = None;
            self.resync_requested = true;
            let _ = transport.send(
                &from,
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
            // If a surviving peer dialed us during failover, ensure it is
            // registered and receives `HostTakeover`.
            if !self.host_peers_by_addr.contains_key(&from) {
                let matched = sender
                    .and_then(|pid| {
                        if pid == self.session.host_id {
                            None
                        } else {
                            self.session.players.get(&pid).cloned()
                        }
                    })
                    .or_else(|| {
                        self.session
                            .players
                            .values()
                            .find(|p| {
                                p.player_id != self.session.host_id
                                    && p.endpoint_hint
                                        .as_deref()
                                        .is_some_and(|hint| peer_matches_hint(&from, hint))
                            })
                            .cloned()
                    });
                if let Some(player) = matched {
                    self.register_failover_peer(transport, from, player.player_id);
                }
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
                if let Some(track) = self.host_peers_by_addr.remove(&from) {
                    self.host_pending_commands.remove(&track.player_id);
                    self.pending_snapshots.retain(|_, req| req.peer != from);
                    if self.session.players.contains_key(&track.player_id) {
                        self.host_system_commands.push_back(Command::PlayerLeft {
                            player_id: track.player_id,
                            reason,
                        });
                    }
                    self.events.push_back(NodeEvent::PeerDisconnected {
                        peer: from,
                        player_id: track.player_id,
                        reason,
                    });
                }
            }
            PlayerRole::Peer => {
                if self.host_addr.as_ref() == Some(&from) {
                    self.initiate_host_failover(transport);
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Failover & helper routines
    // -------------------------------------------------------------------------

    fn initiate_host_failover<T: Transport>(&mut self, transport: &mut T) {
        if self.role != PlayerRole::Peer {
            return;
        }
        let Some(local_id) = self.local_player_id else {
            self.events.push_back(NodeEvent::SessionEnded {
                reason: "host disconnected before join completed".to_string(),
            });
            return;
        };
        let Some(next_host) = self.session.select_next_host() else {
            self.events.push_back(NodeEvent::SessionEnded {
                reason: "no surviving host candidate".to_string(),
            });
            return;
        };

        let new_epoch = self.session.epoch.next();
        if next_host == local_id {
            // Promote at the already-executed simulation tick so SessionState
            // and World stay atomically aligned; discard unexecuted old-epoch batches.
            self.pending_batches.clear();

            if self.session.promote_host(local_id).is_err() {
                self.events.push_back(NodeEvent::SessionEnded {
                    reason: "failed to promote local host".to_string(),
                });
                return;
            }
            self.role = PlayerRole::Host;
            self.host_addr = None;
            self.peer_phase = PeerConnectionPhase::Joined;
            self.primed = true;
            self.awaiting_snapshot_id = None;
            self.active_snapshot_rx = None;
            self.resync_requested = false;
            self.next_schedule_tick = self.session.current_tick.next();

            // Notify any already-connected surviving peers immediately; unconnected
            // surviving peers proactively dial the newly promoted host to avoid
            // simultaneous double-dial races on real P2P transports.
            let surviving: Vec<(PlayerId, Option<String>)> = self
                .session
                .players
                .values()
                .filter(|p| p.player_id != local_id)
                .map(|p| (p.player_id, p.endpoint_hint.clone()))
                .collect();

            for (pid, hint) in surviving {
                if let Some(addr_str) = hint {
                    let canonical = canonical_addr_from_hint(&addr_str);
                    if transport.connected_peers().contains(&canonical) {
                        self.register_failover_peer(transport, canonical, pid);
                    }
                }
            }

            self.events.push_back(NodeEvent::HostPromoted {
                new_host: local_id,
                new_epoch: self.session.epoch,
            });
        } else {
            self.peer_phase = PeerConnectionPhase::AwaitingFailover {
                expected_host: next_host,
                expected_epoch: new_epoch,
            };
            // Proactively connect to the deterministically elected new host.
            if let Some(hint) = self
                .session
                .players
                .get(&next_host)
                .and_then(|p| p.endpoint_hint.clone())
            {
                let dial_addr = PeerAddress(hint.clone());
                let canonical = canonical_addr_from_hint(&hint);
                self.host_addr = Some(canonical.clone());
                if !transport.connected_peers().contains(&canonical) {
                    let _ = transport.connect(&dial_addr);
                } else {
                    let _ = transport.send(
                        &canonical,
                        WireMessage::Heartbeat {
                            epoch: new_epoch,
                            tick: self.session.current_tick,
                            sender: self.local_player_id,
                        },
                    );
                }
            }
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
                last_acked_tick: self.session.current_tick,
                last_command_seq: None,
                reported_step_us: 0,
            });
        let takeover = WireMessage::HostTakeover {
            new_epoch: self.session.epoch,
            new_host: self.session.host_id,
            resume_tick: self.session.current_tick,
            session_state: Box::new(self.session.clone()),
            roster: self.build_host_roster(),
        };
        let _ = transport.send(&addr, takeover);
    }

    fn queue_host_resync(&mut self, peer: PeerAddress, player_id: PlayerId, reason: ResyncReason) {
        let sid = self.allocate_snapshot_id();
        if let Some(track) = self.host_peers_by_addr.get_mut(&peer) {
            track.status = PeerStatus::Syncing {
                snapshot_id: sid,
                reason,
                snapshot_tick: None,
                pending_join_name: None,
                pending_endpoint_hint: None,
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

    fn allocate_snapshot_id(&mut self) -> SnapshotId {
        let id = SnapshotId(self.next_snapshot_id);
        self.next_snapshot_id = self.next_snapshot_id.saturating_add(1);
        id
    }

    fn pending_join_count(&self) -> usize {
        self.host_peers_by_addr
            .values()
            .filter(|t| {
                matches!(
                    t.status,
                    PeerStatus::Syncing {
                        reason: ResyncReason::MidGameJoin,
                        ..
                    }
                )
            })
            .count()
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

fn canonical_addr_from_hint(hint: &str) -> PeerAddress {
    if hint.trim().starts_with(INVITE_TICKET_PREFIX)
        && let Ok(ticket) = InviteTicket::decode_string(hint)
    {
        return PeerAddress(ticket.endpoint_addr().id.to_string());
    }
    PeerAddress(hint.to_string())
}

fn peer_matches_hint(peer: &PeerAddress, hint: &str) -> bool {
    peer.as_str() == hint || canonical_addr_from_hint(hint) == *peer
}
