//! Deterministic in-memory simulated network mesh for fast CI testing.
//!
//! Supports configurable per-link latency (in logical steps), deterministic
//! packet drop, packet reordering, and disconnect/partition injection — with
//! zero wall-clock reads, zero threads, and zero `HashMap`/`HashSet`.

use crate::error::TransportError;
use crate::protocol::{WireMessage, decode_framed, encode_framed};
use crate::transport::{PeerAddress, Transport, TransportEvent};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

/// Fault-injection and timing parameters for a simulated link between two
/// nodes in [`MemoryMesh`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkConfig {
    /// Logical steps before a sent packet is delivered (`0` = next `step()`).
    pub latency_steps: u64,
    /// Deterministic drop probability in parts-per-million (`0..=1_000_000`).
    pub drop_ppm: u32,
    /// Deterministic extra jitter (`0..=reorder_jitter_steps`) added to packet
    /// delivery step, enabling controlled packet reordering.
    pub reorder_jitter_steps: u64,
    /// If `true`, all packets between these two endpoints are dropped.
    pub partitioned: bool,
}

#[derive(Debug, Clone)]
struct InFlightFrame {
    deliver_at_step: u64,
    seq: u64,
    from: PeerAddress,
    to: PeerAddress,
    framed_bytes: Vec<u8>,
}

#[derive(Debug)]
struct MeshInner {
    step: u64,
    next_seq: u64,
    rng_state: u64,
    default_link: LinkConfig,
    links: BTreeMap<(PeerAddress, PeerAddress), LinkConfig>,
    nodes: BTreeSet<PeerAddress>,
    connections: BTreeMap<PeerAddress, BTreeSet<PeerAddress>>,
    inbox: BTreeMap<PeerAddress, VecDeque<TransportEvent>>,
    in_flight: Vec<InFlightFrame>,
}

impl MeshInner {
    fn next_u64(&mut self) -> u64 {
        // SplitMix64: simple, fast, 100% deterministic across all platforms.
        self.rng_state = self.rng_state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn link_config(&self, a: &PeerAddress, b: &PeerAddress) -> LinkConfig {
        let key = ordered_pair(a, b);
        self.links.get(&key).copied().unwrap_or(self.default_link)
    }
}

fn ordered_pair(a: &PeerAddress, b: &PeerAddress) -> (PeerAddress, PeerAddress) {
    if a <= b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

/// Shared deterministic simulated network.
///
/// Create nodes with [`MemoryMesh::add_node`], configure optional link
/// conditions with [`MemoryMesh::set_link_config`], and call
/// [`MemoryMesh::step`] to advance the logical network clock and deliver due
/// packets (all messages are round-tripped through full `encode_framed` /
/// `decode_framed` wire serialization!).
#[derive(Debug, Clone)]
pub struct MemoryMesh {
    inner: Rc<RefCell<MeshInner>>,
}

impl MemoryMesh {
    /// Creates a new deterministic mesh with the given PRNG seed.
    pub fn new(seed: u64) -> Self {
        Self {
            inner: Rc::new(RefCell::new(MeshInner {
                step: 0,
                next_seq: 1,
                rng_state: seed,
                default_link: LinkConfig::default(),
                links: BTreeMap::new(),
                nodes: BTreeSet::new(),
                connections: BTreeMap::new(),
                inbox: BTreeMap::new(),
                in_flight: Vec::new(),
            })),
        }
    }

    /// Sets the default [`LinkConfig`] applied to links without a specific
    /// override.
    pub fn set_default_link(&self, cfg: LinkConfig) {
        self.inner.borrow_mut().default_link = cfg;
    }

    /// Sets fault/latency parameters for the undirected link between `a` and `b`.
    pub fn set_link_config(&self, a: &PeerAddress, b: &PeerAddress, cfg: LinkConfig) {
        let key = ordered_pair(a, b);
        self.inner.borrow_mut().links.insert(key, cfg);
    }

    /// Registers a new node address and returns its [`MemoryNodeTransport`].
    pub fn add_node(&self, addr: impl Into<String>) -> MemoryNodeTransport {
        let peer = PeerAddress::new(addr);
        let mut inner = self.inner.borrow_mut();
        inner.nodes.insert(peer.clone());
        inner.connections.entry(peer.clone()).or_default();
        inner.inbox.entry(peer.clone()).or_default();
        MemoryNodeTransport {
            local: peer,
            mesh: Rc::clone(&self.inner),
        }
    }

    /// Current logical step counter.
    pub fn current_step(&self) -> u64 {
        self.inner.borrow().step
    }

    /// Number of frames currently in flight across the mesh.
    pub fn in_flight_count(&self) -> usize {
        self.inner.borrow().in_flight.len()
    }

    /// Injects an abrupt disconnect (crash/link cut) of `node` from all its
    /// currently connected peers, emitting [`TransportEvent::PeerDisconnected`]
    /// on both sides and dropping its in-flight packets.
    pub fn crash_node(&self, node: &PeerAddress) {
        let mut inner = self.inner.borrow_mut();
        let peers: Vec<PeerAddress> = inner
            .connections
            .get(node)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default();
        for peer in peers {
            if let Some(set) = inner.connections.get_mut(&peer) {
                set.remove(node);
            }
            if let Some(q) = inner.inbox.get_mut(&peer) {
                q.push_back(TransportEvent::PeerDisconnected {
                    peer: node.clone(),
                    reason: "peer crashed".to_string(),
                });
            }
            if let Some(q) = inner.inbox.get_mut(node) {
                q.push_back(TransportEvent::PeerDisconnected {
                    peer,
                    reason: "local crash".to_string(),
                });
            }
        }
        if let Some(set) = inner.connections.get_mut(node) {
            set.clear();
        }
        inner.in_flight.retain(|f| &f.from != node && &f.to != node);
    }

    /// Advances the logical clock by `1` step and delivers all frames scheduled
    /// for delivery at or before the new step.
    pub fn step(&self) -> usize {
        let mut inner = self.inner.borrow_mut();
        inner.step = inner.step.saturating_add(1);
        let now = inner.step;

        let mut ready = Vec::new();
        let mut still_in_flight = Vec::new();
        for frame in inner.in_flight.drain(..) {
            if frame.deliver_at_step <= now {
                ready.push(frame);
            } else {
                still_in_flight.push(frame);
            }
        }
        inner.in_flight = still_in_flight;

        // Deterministic delivery order by (deliver_at_step, seq).
        ready.sort_by_key(|f| (f.deliver_at_step, f.seq));

        let mut delivered = 0usize;
        for frame in ready {
            let still_connected = inner
                .connections
                .get(&frame.to)
                .is_some_and(|peers| peers.contains(&frame.from));
            if !still_connected {
                continue;
            }
            if let Ok(message) = decode_framed(&frame.framed_bytes)
                && let Some(q) = inner.inbox.get_mut(&frame.to)
            {
                q.push_back(TransportEvent::Message {
                    from: frame.from,
                    message,
                });
                delivered += 1;
            }
        }
        delivered
    }

    /// Advances the logical clock `n` times.
    pub fn step_n(&self, n: u64) -> usize {
        let mut total = 0usize;
        for _ in 0..n {
            total += self.step();
        }
        total
    }
}

/// Per-node handle implementing [`Transport`] on a [`MemoryMesh`].
#[derive(Debug, Clone)]
pub struct MemoryNodeTransport {
    local: PeerAddress,
    mesh: Rc<RefCell<MeshInner>>,
}

impl Transport for MemoryNodeTransport {
    fn local_address(&self) -> &PeerAddress {
        &self.local
    }

    fn connect(&mut self, addr: &PeerAddress) -> Result<(), TransportError> {
        let mut inner = self.mesh.borrow_mut();
        if !inner.nodes.contains(addr) {
            return Err(TransportError::InvalidAddress(addr.0.clone()));
        }
        if addr == &self.local {
            return Err(TransportError::InvalidAddress(
                "cannot connect to self".to_string(),
            ));
        }
        let cfg = inner.link_config(&self.local, addr);
        if cfg.partitioned {
            return Err(TransportError::Backend("link partitioned".to_string()));
        }
        let already = inner
            .connections
            .get(&self.local)
            .is_some_and(|s| s.contains(addr));
        if already {
            return Ok(());
        }
        inner
            .connections
            .entry(self.local.clone())
            .or_default()
            .insert(addr.clone());
        inner
            .connections
            .entry(addr.clone())
            .or_default()
            .insert(self.local.clone());

        if let Some(q) = inner.inbox.get_mut(&self.local) {
            q.push_back(TransportEvent::PeerConnected { peer: addr.clone() });
        }
        if let Some(q) = inner.inbox.get_mut(addr) {
            q.push_back(TransportEvent::PeerConnected {
                peer: self.local.clone(),
            });
        }
        Ok(())
    }

    fn disconnect(&mut self, peer: &PeerAddress) -> Result<(), TransportError> {
        let mut inner = self.mesh.borrow_mut();
        let removed_local = inner
            .connections
            .get_mut(&self.local)
            .is_some_and(|s| s.remove(peer));
        let removed_remote = inner
            .connections
            .get_mut(peer)
            .is_some_and(|s| s.remove(&self.local));
        if removed_local || removed_remote {
            if let Some(q) = inner.inbox.get_mut(&self.local) {
                q.push_back(TransportEvent::PeerDisconnected {
                    peer: peer.clone(),
                    reason: "disconnected".to_string(),
                });
            }
            if let Some(q) = inner.inbox.get_mut(peer) {
                q.push_back(TransportEvent::PeerDisconnected {
                    peer: self.local.clone(),
                    reason: "peer disconnected".to_string(),
                });
            }
        }
        Ok(())
    }

    fn send(&mut self, to: &PeerAddress, message: WireMessage) -> Result<(), TransportError> {
        // Always round-trip through canonical wire framing even in memory!
        let framed_bytes = encode_framed(&message)?;
        let mut inner = self.mesh.borrow_mut();
        let connected = inner
            .connections
            .get(&self.local)
            .is_some_and(|s| s.contains(to));
        if !connected {
            return Err(TransportError::NotConnected(to.0.clone()));
        }
        let cfg = inner.link_config(&self.local, to);
        if cfg.partitioned {
            return Ok(());
        }
        if cfg.drop_ppm > 0 {
            let roll = (inner.next_u64() % 1_000_000) as u32;
            if roll < cfg.drop_ppm {
                return Ok(());
            }
        }
        let jitter = if cfg.reorder_jitter_steps > 0 {
            inner.next_u64() % (cfg.reorder_jitter_steps + 1)
        } else {
            0
        };
        let deliver_at_step = inner
            .step
            .saturating_add(1)
            .saturating_add(cfg.latency_steps)
            .saturating_add(jitter);
        let seq = inner.next_seq;
        inner.next_seq = seq.saturating_add(1);
        inner.in_flight.push(InFlightFrame {
            deliver_at_step,
            seq,
            from: self.local.clone(),
            to: to.clone(),
            framed_bytes,
        });
        Ok(())
    }

    fn drain_events(&mut self) -> Vec<TransportEvent> {
        let mut inner = self.mesh.borrow_mut();
        inner
            .inbox
            .get_mut(&self.local)
            .map(|q| q.drain(..).collect())
            .unwrap_or_default()
    }

    fn connected_peers(&self) -> Vec<PeerAddress> {
        let inner = self.mesh.borrow();
        inner
            .connections
            .get(&self.local)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }
}
