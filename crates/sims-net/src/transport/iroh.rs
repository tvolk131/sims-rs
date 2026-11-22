//! Real `iroh` P2P QUIC transport implementation.
//!
//! Uses `iroh::Endpoint` with ALPN [`crate::ALPN`] (`b"sims-rs/coop/1"`),
//! one bidirectional QUIC stream per peer carrying a 4-byte `"SIM1"` stream
//! preamble followed by 4-byte length-prefixed `postcard` frames, and shareable
//! [`InviteTicket`] tokens.
//!
//! Safety & liveness invariants:
//! - Deterministic simultaneous-dial tie-breaking: both endpoints select the
//!   same winning connection using total order on canonical `EndpointId`s.
//! - Generation-gated event publication: messages and disconnects from a
//!   superseded connection generation are discarded.
//! - Bounded global and per-connection inbound byte/event budgets applying
//!   natural QUIC flow-control backpressure to senders.
//! - Graceful writer drain on disconnect so `Goodbye` frames flush cleanly.

use crate::error::TransportError;
use crate::protocol::{ALPN, MAX_WIRE_FRAME_BYTES, WireMessage, decode_payload, encode_framed};
use crate::transport::{
    INVITE_TICKET_PREFIX, InviteTicket, PeerAddress, Transport, TransportEvent,
};
use ::iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, RecvStream, SendStream, presets},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::task::JoinHandle;

/// 4-byte preamble written by the dialer on every new bidirectional stream so
/// the acceptor's `accept_bi()` wakes immediately upon connection.
const STREAM_MAGIC: &[u8; 4] = b"SIM1";

/// Bounded outbound frame queue depth per connected peer.
const OUTBOUND_CHANNEL_CAP: usize = 256;

/// Maximum buffered decoded inbound events across all connections.
const MAX_INBOUND_EVENTS: usize = 1024;

/// Maximum aggregate inbound decoded payload bytes buffered before applying
/// QUIC stream backpressure (16 MiB).
const MAX_GLOBAL_INBOUND_BYTES: usize = 16 * 1024 * 1024;

/// Maximum inbound decoded payload bytes buffered for a single peer before
/// applying QUIC stream backpressure (4 MiB).
const MAX_PER_PEER_INBOUND_BYTES: usize = 4 * 1024 * 1024;

/// Maximum time a writer task waits after `send.finish()` on graceful shutdown
/// so a queued `Goodbye` frame can flush to the remote peer.
const GRACEFUL_DRAIN_TIMEOUT: Duration = Duration::from_millis(150);

/// Configuration for binding an [`IrohTransport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrohConfig {
    /// Local loopback binding (`127.0.0.1:0`, `presets::Minimal`, no relay/DNS).
    Loopback,
    /// Standard internet P2P binding (`presets::N0`, relay + hole punching).
    P2pDefault,
}

struct PeerHandle {
    conn_id: u64,
    initiator_is_local: bool,
    tx: mpsc::Sender<Vec<u8>>,
    conn: Connection,
}

struct QueuedEvent {
    event: TransportEvent,
    _global_event_permit: Option<OwnedSemaphorePermit>,
    _global_byte_permit: Option<OwnedSemaphorePermit>,
    _peer_byte_permit: Option<OwnedSemaphorePermit>,
}

struct SharedState {
    local_addr: PeerAddress,
    next_conn_id: AtomicU64,
    peers: Mutex<BTreeMap<PeerAddress, PeerHandle>>,
    dialing: Mutex<BTreeSet<PeerAddress>>,
    events: Mutex<VecDeque<QueuedEvent>>,
    event_notify: Notify,
    global_event_budget: Arc<Semaphore>,
    global_byte_budget: Arc<Semaphore>,
}

impl SharedState {
    fn new(local_addr: PeerAddress) -> Self {
        Self {
            local_addr,
            next_conn_id: AtomicU64::new(1),
            peers: Mutex::new(BTreeMap::new()),
            dialing: Mutex::new(BTreeSet::new()),
            events: Mutex::new(VecDeque::new()),
            event_notify: Notify::new(),
            global_event_budget: Arc::new(Semaphore::new(MAX_INBOUND_EVENTS)),
            global_byte_budget: Arc::new(Semaphore::new(MAX_GLOBAL_INBOUND_BYTES)),
        }
    }

    fn alloc_conn_id(&self) -> u64 {
        self.next_conn_id.fetch_add(1, Ordering::Relaxed)
    }

    fn push_control_event(&self, event: TransportEvent) {
        if let Ok(mut q) = self.events.lock() {
            q.push_back(QueuedEvent {
                event,
                _global_event_permit: None,
                _global_byte_permit: None,
                _peer_byte_permit: None,
            });
        }
        self.event_notify.notify_waiters();
    }

    fn push_message_for_conn(
        &self,
        peer: &PeerAddress,
        conn_id: u64,
        message: WireMessage,
        global_event_permit: OwnedSemaphorePermit,
        global_byte_permit: OwnedSemaphorePermit,
        peer_byte_permit: OwnedSemaphorePermit,
    ) -> bool {
        let is_active = self
            .peers
            .lock()
            .ok()
            .is_some_and(|map| map.get(peer).is_some_and(|h| h.conn_id == conn_id));
        if !is_active {
            return false;
        }
        if let Ok(mut q) = self.events.lock() {
            q.push_back(QueuedEvent {
                event: TransportEvent::Message {
                    from: peer.clone(),
                    message,
                },
                _global_event_permit: Some(global_event_permit),
                _global_byte_permit: Some(global_byte_permit),
                _peer_byte_permit: Some(peer_byte_permit),
            });
        }
        self.event_notify.notify_waiters();
        true
    }

    /// Registers a peer connection using deterministic simultaneous-dial
    /// tie-breaking so both endpoints always keep the same QUIC connection.
    ///
    /// Returns `true` if `handle` was installed as the active connection, or
    /// `false` if an existing higher-priority connection won the tie-break (in
    /// which case `handle.conn` is closed).
    fn register_peer(&self, peer: PeerAddress, handle: PeerHandle) -> bool {
        let mut emit_connected = false;
        let accepted = if let Ok(mut map) = self.peers.lock() {
            if let Some(existing) = map.get(&peer) {
                let local_wins_tie = self.local_addr < peer;
                let incoming_wins = if existing.initiator_is_local == handle.initiator_is_local {
                    // Same initiator reconnecting: newer replaces older.
                    true
                } else {
                    // Simultaneous cross-dial: both sides agree on the initiator
                    // with the lexicographically smaller canonical EndpointId.
                    handle.initiator_is_local == local_wins_tie
                };
                if incoming_wins {
                    if let Some(old) = map.insert(peer.clone(), handle) {
                        old.conn.close(0u32.into(), b"replaced");
                    }
                    true
                } else {
                    handle.conn.close(0u32.into(), b"tie-lost");
                    false
                }
            } else {
                map.insert(peer.clone(), handle);
                emit_connected = true;
                true
            }
        } else {
            false
        };
        if emit_connected {
            self.push_control_event(TransportEvent::PeerConnected { peer });
        }
        accepted
    }

    fn remove_peer(&self, peer: &PeerAddress, conn_id: u64, reason: String) {
        let removed = if let Ok(mut map) = self.peers.lock() {
            if map.get(peer).is_some_and(|h| h.conn_id == conn_id) {
                map.remove(peer).is_some()
            } else {
                false
            }
        } else {
            false
        };
        if removed {
            self.push_control_event(TransportEvent::PeerDisconnected {
                peer: peer.clone(),
                reason,
            });
        }
    }
}

/// Real `iroh` P2P transport implementing [`Transport`].
pub struct IrohTransport {
    endpoint: Endpoint,
    local_addr: PeerAddress,
    runtime: Handle,
    shared: Arc<SharedState>,
    accept_task: Option<JoinHandle<()>>,
    closed: bool,
}

impl IrohTransport {
    /// Binds a new `iroh` endpoint according to `config` on the current Tokio
    /// runtime and starts accepting incoming co-op connections.
    pub async fn bind(config: IrohConfig) -> Result<Self, TransportError> {
        let endpoint = match config {
            IrohConfig::Loopback => Endpoint::builder(presets::Minimal)
                .alpns(vec![ALPN.to_vec()])
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .map_err(|e| TransportError::Backend(e.to_string()))?
                .bind()
                .await
                .map_err(|e| TransportError::Backend(e.to_string()))?,
            IrohConfig::P2pDefault => {
                let ep = Endpoint::builder(presets::N0)
                    .alpns(vec![ALPN.to_vec()])
                    .bind()
                    .await
                    .map_err(|e| TransportError::Backend(e.to_string()))?;
                ep.online().await;
                ep
            }
        };

        let local_addr = PeerAddress(endpoint.id().to_string());
        let shared = Arc::new(SharedState::new(local_addr.clone()));
        let runtime = Handle::current();

        let accept_task = runtime.spawn(accept_loop(endpoint.clone(), Arc::clone(&shared)));

        Ok(Self {
            endpoint,
            local_addr,
            runtime,
            shared,
            accept_task: Some(accept_task),
            closed: false,
        })
    }

    /// Returns this endpoint's shareable [`InviteTicket`].
    pub fn invite_ticket(&self) -> InviteTicket {
        InviteTicket::new(self.endpoint.addr())
    }

    /// Returns the underlying `iroh::EndpointAddr`.
    pub fn endpoint_addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    /// Asynchronously connects to a host/peer using an [`InviteTicket`] and
    /// returns the remote peer's canonical [`PeerAddress`].
    pub async fn connect_ticket(
        &mut self,
        ticket: &InviteTicket,
    ) -> Result<PeerAddress, TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        let remote_addr = ticket.endpoint_addr().clone();
        let peer_addr = PeerAddress(remote_addr.id.to_string());
        if self
            .shared
            .peers
            .lock()
            .ok()
            .is_some_and(|m| m.contains_key(&peer_addr))
        {
            return Ok(peer_addr);
        }

        let conn = self
            .endpoint
            .connect(remote_addr, ALPN)
            .await
            .map_err(|e| TransportError::Backend(e.to_string()))?;
        let (mut send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| TransportError::Backend(e.to_string()))?;

        // Write stream magic immediately so the acceptor's `accept_bi` wakes up.
        send.write_all(STREAM_MAGIC)
            .await
            .map_err(|e| TransportError::Backend(e.to_string()))?;

        spawn_peer_io(
            &self.runtime,
            Arc::clone(&self.shared),
            peer_addr.clone(),
            true,
            conn,
            send,
            recv,
        );
        Ok(peer_addr)
    }

    /// Waits asynchronously until at least one [`TransportEvent`] is queued
    /// without missed-wakeup races.
    pub async fn wait_for_event(&self) {
        loop {
            let notified = self.shared.event_notify.notified();
            tokio::pin!(notified);
            // Register waiter before checking the queue.
            notified.as_mut().enable();
            if self
                .shared
                .events
                .lock()
                .ok()
                .is_some_and(|q| !q.is_empty())
            {
                return;
            }
            notified.await;
        }
    }

    /// Gracefully closes all peer connections (allowing queued outbound frames
    /// such as `Goodbye` a brief window to flush) and closes the underlying
    /// `iroh` endpoint.
    pub async fn close(mut self) {
        self.closed = true;
        let handles: Vec<PeerHandle> = self
            .shared
            .peers
            .lock()
            .ok()
            .map(|mut map| std::mem::take(&mut *map).into_values().collect())
            .unwrap_or_default();
        // Drop `tx` senders first so writer tasks flush queued frames + `send.finish()`.
        let conns: Vec<Connection> = handles.into_iter().map(|h| h.conn).collect();
        if !conns.is_empty() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        for conn in conns {
            conn.close(0u32.into(), b"closing");
        }
        self.endpoint.close().await;
        if let Some(task) = self.accept_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    fn resolve_peer_key(addr: &PeerAddress) -> PeerAddress {
        if addr.0.trim().starts_with(INVITE_TICKET_PREFIX)
            && let Ok(ticket) = InviteTicket::decode_string(&addr.0)
        {
            return PeerAddress(ticket.endpoint_addr().id.to_string());
        }
        addr.clone()
    }
}

impl Drop for IrohTransport {
    fn drop(&mut self) {
        if let Some(task) = self.accept_task.take() {
            task.abort();
        }
    }
}

impl Transport for IrohTransport {
    fn local_address(&self) -> &PeerAddress {
        &self.local_addr
    }

    fn connect(&mut self, addr: &PeerAddress) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        let ticket = InviteTicket::decode_string(&addr.0)
            .map_err(|e| TransportError::InvalidAddress(e.to_string()))?;
        let remote_addr = ticket.endpoint_addr().clone();
        let peer_addr = PeerAddress(remote_addr.id.to_string());
        if self
            .shared
            .peers
            .lock()
            .ok()
            .is_some_and(|m| m.contains_key(&peer_addr))
        {
            return Ok(());
        }
        // Deduplicate concurrent in-flight dials to the same peer.
        if let Ok(mut dialing) = self.shared.dialing.lock()
            && !dialing.insert(peer_addr.clone())
        {
            return Ok(());
        }

        let endpoint = self.endpoint.clone();
        let shared = Arc::clone(&self.shared);
        let runtime = self.runtime.clone();
        self.runtime.spawn(async move {
            let outcome: Result<(Connection, SendStream, RecvStream), String> = async {
                let conn = endpoint
                    .connect(remote_addr, ALPN)
                    .await
                    .map_err(|e| format!("connect failed: {e}"))?;
                let (mut send, recv) = conn
                    .open_bi()
                    .await
                    .map_err(|e| format!("open_bi failed: {e}"))?;
                send.write_all(STREAM_MAGIC)
                    .await
                    .map_err(|e| format!("preamble write failed: {e}"))?;
                Ok((conn, send, recv))
            }
            .await;

            if let Ok(mut dialing) = shared.dialing.lock() {
                dialing.remove(&peer_addr);
            }

            match outcome {
                Ok((conn, send, recv)) => {
                    spawn_peer_io(&runtime, shared, peer_addr, true, conn, send, recv);
                }
                Err(reason) => {
                    let already_connected = shared
                        .peers
                        .lock()
                        .ok()
                        .is_some_and(|m| m.contains_key(&peer_addr));
                    if !already_connected {
                        shared.push_control_event(TransportEvent::PeerDisconnected {
                            peer: peer_addr,
                            reason,
                        });
                    }
                }
            }
        });
        Ok(())
    }

    fn disconnect(&mut self, peer: &PeerAddress) -> Result<(), TransportError> {
        let key = Self::resolve_peer_key(peer);
        let removed = self
            .shared
            .peers
            .lock()
            .ok()
            .and_then(|mut map| map.remove(&key));
        if let Some(handle) = removed {
            // Drop `handle.tx` so the writer task finishes flushing any queued
            // frames (such as `Goodbye`) and calls `send.finish()`, then close
            // the QUIC connection after a bounded drain window.
            let conn = handle.conn;
            drop(handle.tx);
            self.runtime.spawn(async move {
                tokio::time::sleep(GRACEFUL_DRAIN_TIMEOUT).await;
                conn.close(0u32.into(), b"disconnect");
            });
            self.shared
                .push_control_event(TransportEvent::PeerDisconnected {
                    peer: key,
                    reason: "disconnected".to_string(),
                });
        }
        Ok(())
    }

    fn send(&mut self, to: &PeerAddress, message: WireMessage) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        let framed = encode_framed(&message)?;
        let key = Self::resolve_peer_key(to);
        let tx = self
            .shared
            .peers
            .lock()
            .ok()
            .and_then(|map| map.get(&key).map(|h| h.tx.clone()))
            .ok_or_else(|| TransportError::NotConnected(key.0.clone()))?;

        tx.try_send(framed).map_err(|err| match err {
            mpsc::error::TrySendError::Full(_) => TransportError::Backpressure(key.0),
            mpsc::error::TrySendError::Closed(_) => TransportError::NotConnected(key.0),
        })
    }

    fn drain_events(&mut self) -> Vec<TransportEvent> {
        self.shared
            .events
            .lock()
            .ok()
            .map(|mut q| q.drain(..).map(|item| item.event).collect())
            .unwrap_or_default()
    }

    fn connected_peers(&self) -> Vec<PeerAddress> {
        self.shared
            .peers
            .lock()
            .ok()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default()
    }
}

async fn accept_loop(endpoint: Endpoint, shared: Arc<SharedState>) {
    let runtime = Handle::current();
    while let Some(incoming) = endpoint.accept().await {
        let shared = Arc::clone(&shared);
        let runtime = runtime.clone();
        tokio::spawn(async move {
            let Ok(conn) = incoming.await else {
                return;
            };
            let peer_addr = PeerAddress(conn.remote_id().to_string());
            let Ok((send, mut recv)) = conn.accept_bi().await else {
                return;
            };
            let mut magic = [0u8; 4];
            if recv.read_exact(&mut magic).await.is_err() || &magic != STREAM_MAGIC {
                conn.close(1u32.into(), b"bad magic");
                return;
            }
            spawn_peer_io(&runtime, shared, peer_addr, false, conn, send, recv);
        });
    }
}

fn spawn_peer_io(
    runtime: &Handle,
    shared: Arc<SharedState>,
    peer_addr: PeerAddress,
    initiator_is_local: bool,
    conn: Connection,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    let conn_id = shared.alloc_conn_id();
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(OUTBOUND_CHANNEL_CAP);
    let accepted = shared.register_peer(
        peer_addr.clone(),
        PeerHandle {
            conn_id,
            initiator_is_local,
            tx,
            conn: conn.clone(),
        },
    );
    if !accepted {
        return;
    }

    let peer_byte_budget = Arc::new(Semaphore::new(MAX_PER_PEER_INBOUND_BYTES));

    // Writer task: drains all queued frames even after `tx` is dropped so
    // `Goodbye` frames flush cleanly before `send.finish()`.
    let writer_shared = Arc::clone(&shared);
    let writer_peer = peer_addr.clone();
    runtime.spawn(async move {
        while let Some(frame) = rx.recv().await {
            if send.write_all(&frame).await.is_err() {
                writer_shared.remove_peer(&writer_peer, conn_id, "write error".to_string());
                return;
            }
        }
        let _ = send.finish();
    });

    // Reader task: acquires per-peer and global event/byte permits before
    // buffering decoded messages, applying natural QUIC flow-control
    // backpressure when the caller falls behind draining events.
    runtime.spawn(async move {
        loop {
            let mut len_buf = [0u8; 4];
            if let Err(err) = recv.read_exact(&mut len_buf).await {
                shared.remove_peer(&peer_addr, conn_id, format!("stream closed: {err}"));
                return;
            }
            let payload_len = u32::from_be_bytes(len_buf) as usize;
            if payload_len > MAX_WIRE_FRAME_BYTES {
                conn.close(2u32.into(), b"frame too large");
                shared.remove_peer(&peer_addr, conn_id, "frame exceeded max length".to_string());
                return;
            }
            let permit_bytes = payload_len.clamp(1, MAX_PER_PEER_INBOUND_BYTES) as u32;
            let Ok(global_event_permit) = Arc::clone(&shared.global_event_budget)
                .acquire_owned()
                .await
            else {
                return;
            };
            let Ok(global_byte_permit) = Arc::clone(&shared.global_byte_budget)
                .acquire_many_owned(permit_bytes)
                .await
            else {
                return;
            };
            let Ok(peer_byte_permit) = Arc::clone(&peer_byte_budget)
                .acquire_many_owned(permit_bytes)
                .await
            else {
                return;
            };

            let mut payload = vec![0u8; payload_len];
            if let Err(err) = recv.read_exact(&mut payload).await {
                shared.remove_peer(&peer_addr, conn_id, format!("truncated frame: {err}"));
                return;
            }
            match decode_payload(&payload) {
                Ok(message) => {
                    if !shared.push_message_for_conn(
                        &peer_addr,
                        conn_id,
                        message,
                        global_event_permit,
                        global_byte_permit,
                        peer_byte_permit,
                    ) {
                        return;
                    }
                }
                Err(err) => {
                    conn.close(3u32.into(), b"invalid payload");
                    shared.remove_peer(&peer_addr, conn_id, format!("decode error: {err}"));
                    return;
                }
            }
        }
    });
}
