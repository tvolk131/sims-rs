//! Transport abstraction, invite tickets, deterministic in-memory mesh, and
//! `iroh` P2P QUIC implementation.

pub mod iroh;
pub mod memory;

use crate::error::{ProtocolError, TicketError, TransportError};
use crate::protocol::WireMessage;
use ::iroh::EndpointAddr;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

pub use self::iroh::{IrohConfig, IrohTransport};
pub use self::memory::{LinkConfig, MemoryMesh, MemoryNodeTransport};

/// Canonical string prefix for [`InviteTicket`] tokens.
pub const INVITE_TICKET_PREFIX: &str = "simscoop1";

/// Wire version stored inside [`InviteTicket`].
const INVITE_TICKET_VERSION: u8 = 1;

/// Opaque transport-level peer address.
///
/// - For [`MemoryMesh`]: logical address such as `"mem:host"`.
/// - For [`IrohTransport`]: canonical `iroh::EndpointId` string (or an
///   [`InviteTicket`] string when initiating a new connection).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PeerAddress(pub String);

impl PeerAddress {
    /// Creates a new peer address from any string.
    pub fn new(addr: impl Into<String>) -> Self {
        Self(addr.into())
    }

    /// Returns the address string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PeerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Event delivered by a [`Transport`] to [`crate::LockstepNode::poll`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportEvent {
    /// A peer connection is ready for messages.
    PeerConnected { peer: PeerAddress },
    /// A peer connection closed or failed.
    PeerDisconnected { peer: PeerAddress, reason: String },
    /// A decoded [`WireMessage`] arrived from `from`.
    Message {
        from: PeerAddress,
        message: WireMessage,
    },
}

/// Channel-oriented non-blocking transport trait shared by [`MemoryMesh`] and
/// [`IrohTransport`].
pub trait Transport {
    /// Returns this node's own transport address.
    fn local_address(&self) -> &PeerAddress;

    /// Initiates a connection to `addr` (idempotent if already connected).
    fn connect(&mut self, addr: &PeerAddress) -> Result<(), TransportError>;

    /// Closes the connection to `peer`.
    fn disconnect(&mut self, peer: &PeerAddress) -> Result<(), TransportError>;

    /// Sends one [`WireMessage`] to `to`.
    fn send(&mut self, to: &PeerAddress, message: WireMessage) -> Result<(), TransportError>;

    /// Broadcasts `message` to all currently connected peers.
    fn broadcast(&mut self, message: WireMessage) -> Result<(), TransportError> {
        let peers = self.connected_peers();
        for peer in peers {
            let _ = self.send(&peer, message.clone());
        }
        Ok(())
    }

    /// Drains all pending [`TransportEvent`]s without blocking.
    fn drain_events(&mut self) -> Vec<TransportEvent>;

    /// Returns all currently connected peer addresses in deterministic order.
    fn connected_peers(&self) -> Vec<PeerAddress>;
}

/// Shareable co-op invite ticket wrapping an `iroh::EndpointAddr`.
///
/// Encodes as `simscoop1<lowercase-base32(version || postcard(EndpointAddr))>`
/// so players can copy-paste an invite string with no external coordination
/// server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteTicket {
    addr: EndpointAddr,
}

#[derive(Serialize, Deserialize)]
struct TicketPayloadV1 {
    addr: EndpointAddr,
}

impl InviteTicket {
    /// Creates an invite ticket from an `iroh` [`EndpointAddr`].
    pub const fn new(addr: EndpointAddr) -> Self {
        Self { addr }
    }

    /// Returns the underlying `iroh` [`EndpointAddr`].
    pub const fn endpoint_addr(&self) -> &EndpointAddr {
        &self.addr
    }

    /// Encodes the ticket into its canonical `simscoop1...` string form.
    pub fn encode_string(&self) -> Result<String, TicketError> {
        let body = postcard::to_allocvec(&TicketPayloadV1 {
            addr: self.addr.clone(),
        })
        .map_err(|e| TicketError::Protocol(ProtocolError::Encode(e.to_string())))?;
        let mut raw = Vec::with_capacity(1 + body.len());
        raw.push(INVITE_TICKET_VERSION);
        raw.extend_from_slice(&body);
        let mut out = String::with_capacity(INVITE_TICKET_PREFIX.len() + raw.len() * 2);
        out.push_str(INVITE_TICKET_PREFIX);
        base32_encode_lower(&raw, &mut out);
        Ok(out)
    }

    /// Decodes a canonical `simscoop1...` invite ticket string.
    pub fn decode_string(s: &str) -> Result<Self, TicketError> {
        let trimmed = s.trim();
        let rest = trimmed
            .strip_prefix(INVITE_TICKET_PREFIX)
            .ok_or(TicketError::InvalidPrefix)?;
        let raw = base32_decode_ci(rest)?;
        let (&version, payload) = raw
            .split_first()
            .ok_or_else(|| TicketError::InvalidEncoding("empty ticket payload".to_string()))?;
        if version != INVITE_TICKET_VERSION {
            return Err(TicketError::UnsupportedVersion(version));
        }
        let (decoded, rest): (TicketPayloadV1, &[u8]) = postcard::take_from_bytes(payload)
            .map_err(|e| TicketError::Protocol(ProtocolError::Decode(e.to_string())))?;
        if !rest.is_empty() {
            return Err(TicketError::Protocol(ProtocolError::Decode(format!(
                "unexpected {} trailing bytes in invite ticket",
                rest.len()
            ))));
        }
        Ok(Self { addr: decoded.addr })
    }
}

impl fmt::Display for InviteTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.encode_string() {
            Ok(encoded) => f.write_str(&encoded),
            Err(_) => Err(fmt::Error),
        }
    }
}

impl FromStr for InviteTicket {
    type Err = TicketError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::decode_string(s)
    }
}

const BASE32_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

fn base32_encode_lower(bytes: &[u8], out: &mut String) {
    let mut buffer = 0u16;
    let mut bits = 0u8;
    for &byte in bytes {
        buffer = (buffer << 8) | u16::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buffer >> bits) & 0x1f) as usize;
            out.push(BASE32_ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buffer << (5 - bits)) & 0x1f) as usize;
        out.push(BASE32_ALPHABET[idx] as char);
    }
}

fn base32_decode_ci(input: &str) -> Result<Vec<u8>, TicketError> {
    let mut out = Vec::with_capacity((input.len() * 5) / 8);
    let mut buffer = 0u16;
    let mut bits = 0u8;
    for ch in input.bytes() {
        let val = match ch {
            b'a'..=b'z' => ch - b'a',
            b'A'..=b'Z' => ch - b'A',
            b'2'..=b'7' => 26 + (ch - b'2'),
            _ => {
                return Err(TicketError::InvalidEncoding(format!(
                    "unexpected character 0x{ch:02x}"
                )));
            }
        };
        buffer = (buffer << 5) | u16::from(val);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    if bits >= 5 {
        return Err(TicketError::InvalidEncoding(
            "invalid trailing base32 length".to_string(),
        ));
    }
    let mask = (1u16 << bits) - 1;
    if (buffer & mask) != 0 {
        return Err(TicketError::InvalidEncoding(
            "non-zero base32 padding bits".to_string(),
        ));
    }
    Ok(out)
}
