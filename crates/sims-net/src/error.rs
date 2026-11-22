//! Error types for `sims-net`.

use crate::protocol::{Epoch, PlayerId, SnapshotId, Tick};
use std::fmt;

/// Wire serialization and framing errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// Postcard encoding failed.
    Encode(String),
    /// Postcard decoding failed.
    Decode(String),
    /// A wire frame exceeded the maximum allowed byte length.
    FrameTooLarge { len: usize, max: usize },
    /// A framed buffer was truncated or had trailing bytes.
    InvalidFrameLength { expected: usize, actual: usize },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(msg) => write!(f, "protocol encode error: {msg}"),
            Self::Decode(msg) => write!(f, "protocol decode error: {msg}"),
            Self::FrameTooLarge { len, max } => {
                write!(f, "wire frame size {len} exceeds maximum {max} bytes")
            }
            Self::InvalidFrameLength { expected, actual } => {
                write!(
                    f,
                    "invalid wire frame length: expected {expected} bytes, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for ProtocolError {}

/// File manifest construction and validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// Filesystem I/O failure while scanning or hashing game files.
    Io { path: String, message: String },
    /// Two entries collided after canonical path normalization.
    DuplicatePath(String),
    /// An entry path was empty or contained invalid path components (`..`, NUL).
    InvalidPath(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "manifest I/O error at {path}: {message}"),
            Self::DuplicatePath(path) => write!(f, "duplicate canonical manifest path: {path}"),
            Self::InvalidPath(path) => write!(f, "invalid manifest path: {path}"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// Chunked/compressed REDB snapshot transfer errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// Snapshot uncompressed length exceeded the safety cap.
    UncompressedTooLarge { len: u64, max: u64 },
    /// Snapshot compressed length exceeded the safety cap.
    CompressedTooLarge { len: u64, max: u64 },
    /// Invalid chunk size (zero or above cap).
    InvalidChunkSize { chunk_size: u32, max: u32 },
    /// Header chunk count does not match `compressed_len` and `chunk_size`.
    InconsistentChunkCount { expected: u32, actual: u32 },
    /// Chunk belongs to a different snapshot transfer.
    SnapshotIdMismatch {
        expected: SnapshotId,
        actual: SnapshotId,
    },
    /// Chunk index is out of bounds.
    ChunkOutOfRange { index: u32, total_chunks: u32 },
    /// Chunk payload byte count did not match expected slice length.
    ChunkSizeMismatch {
        index: u32,
        expected: usize,
        actual: usize,
    },
    /// A duplicate chunk arrived with different bytes than the earlier one.
    ConflictingDuplicateChunk { index: u32 },
    /// Snapshot was finalized before all chunks arrived.
    Incomplete { received: u32, total: u32 },
    /// Compression or decompression failed.
    Compression(String),
    /// Decompressed byte count did not match header `uncompressed_len`.
    UncompressedLengthMismatch { expected: u64, actual: u64 },
    /// Compressed SHA-256 digest mismatch.
    CompressedDigestMismatch,
    /// Uncompressed SHA-256 digest mismatch.
    UncompressedDigestMismatch,
    /// Inner snapshot envelope failed to decode.
    Envelope(ProtocolError),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UncompressedTooLarge { len, max } => {
                write!(f, "snapshot uncompressed size {len} exceeds max {max}")
            }
            Self::CompressedTooLarge { len, max } => {
                write!(f, "snapshot compressed size {len} exceeds max {max}")
            }
            Self::InvalidChunkSize { chunk_size, max } => {
                write!(f, "invalid snapshot chunk size {chunk_size} (max {max})")
            }
            Self::InconsistentChunkCount { expected, actual } => {
                write!(
                    f,
                    "inconsistent snapshot chunk count: expected {expected}, header says {actual}"
                )
            }
            Self::SnapshotIdMismatch { expected, actual } => {
                write!(
                    f,
                    "snapshot chunk id mismatch: expected {expected}, got {actual}"
                )
            }
            Self::ChunkOutOfRange {
                index,
                total_chunks,
            } => write!(
                f,
                "snapshot chunk index {index} out of range (total {total_chunks})"
            ),
            Self::ChunkSizeMismatch {
                index,
                expected,
                actual,
            } => write!(
                f,
                "snapshot chunk {index} length mismatch: expected {expected}, got {actual}"
            ),
            Self::ConflictingDuplicateChunk { index } => {
                write!(f, "conflicting duplicate snapshot chunk {index}")
            }
            Self::Incomplete { received, total } => {
                write!(f, "snapshot incomplete: received {received}/{total} chunks")
            }
            Self::Compression(msg) => write!(f, "snapshot compression error: {msg}"),
            Self::UncompressedLengthMismatch { expected, actual } => {
                write!(
                    f,
                    "snapshot uncompressed length mismatch: expected {expected}, got {actual}"
                )
            }
            Self::CompressedDigestMismatch => {
                f.write_str("snapshot compressed SHA-256 digest mismatch")
            }
            Self::UncompressedDigestMismatch => {
                f.write_str("snapshot uncompressed SHA-256 digest mismatch")
            }
            Self::Envelope(err) => write!(f, "snapshot envelope error: {err}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Deterministic session & lockstep state machine errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// Batch epoch did not match current session epoch.
    EpochMismatch { expected: Epoch, actual: Epoch },
    /// Batch tick did not equal `current_tick + 1`.
    UnexpectedTick { expected: Tick, actual: Tick },
    /// Command inside a batch carried a tick different from the batch tick.
    CommandTickMismatch {
        batch_tick: Tick,
        command_tick: Tick,
    },
    /// Command sequence numbers inside a batch were not contiguous `0..n`.
    InvalidCommandSequence { expected: u32, actual: u32 },
    /// Operation is only valid on a host node.
    NotHost,
    /// Operation is only valid on a peer node.
    NotPeer,
    /// Operation requires an assigned player ID.
    UnassignedPlayer,
    /// Referenced player is not present in the session.
    UnknownPlayer(PlayerId),
    /// Referenced snapshot transfer is unknown or no longer pending.
    UnknownSnapshot(SnapshotId),
    /// Snapshot transfer error.
    Snapshot(SnapshotError),
    /// Underlying transport error.
    Transport(TransportError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EpochMismatch { expected, actual } => {
                write!(
                    f,
                    "session epoch mismatch: expected {expected}, got {actual}"
                )
            }
            Self::UnexpectedTick { expected, actual } => {
                write!(f, "unexpected tick: expected {expected}, got {actual}")
            }
            Self::CommandTickMismatch {
                batch_tick,
                command_tick,
            } => write!(
                f,
                "command tick {command_tick} does not match batch tick {batch_tick}"
            ),
            Self::InvalidCommandSequence { expected, actual } => {
                write!(
                    f,
                    "invalid command sequence in batch: expected {expected}, got {actual}"
                )
            }
            Self::NotHost => f.write_str("operation requires host role"),
            Self::NotPeer => f.write_str("operation requires peer role"),
            Self::UnassignedPlayer => f.write_str("peer has not been assigned a player id yet"),
            Self::UnknownPlayer(id) => write!(f, "unknown player {id}"),
            Self::UnknownSnapshot(id) => write!(f, "unknown snapshot transfer {id}"),
            Self::Snapshot(err) => err.fmt(f),
            Self::Transport(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<SnapshotError> for SessionError {
    fn from(value: SnapshotError) -> Self {
        Self::Snapshot(value)
    }
}

impl From<TransportError> for SessionError {
    fn from(value: TransportError) -> Self {
        Self::Transport(value)
    }
}

/// Invite ticket parsing/encoding errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TicketError {
    /// Ticket string did not start with the expected prefix.
    InvalidPrefix,
    /// Base32 payload contained invalid characters or length.
    InvalidEncoding(String),
    /// Ticket wire payload version is unsupported.
    UnsupportedVersion(u8),
    /// Postcard serialization/deserialization failed.
    Protocol(ProtocolError),
}

impl fmt::Display for TicketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPrefix => f.write_str("invalid invite ticket prefix"),
            Self::InvalidEncoding(msg) => write!(f, "invalid invite ticket encoding: {msg}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported invite ticket version {v}"),
            Self::Protocol(err) => write!(f, "invite ticket payload error: {err}"),
        }
    }
}

impl std::error::Error for TicketError {}

/// Transport-layer errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// Destination peer is not currently connected.
    NotConnected(String),
    /// Peer address could not be parsed or resolved.
    InvalidAddress(String),
    /// Transport endpoint has been closed.
    Closed,
    /// Outbound queue for a peer is full.
    Backpressure(String),
    /// Wire framing error.
    Protocol(ProtocolError),
    /// Underlying network/endpoint error.
    Backend(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected(addr) => write!(f, "peer not connected: {addr}"),
            Self::InvalidAddress(msg) => write!(f, "invalid peer address: {msg}"),
            Self::Closed => f.write_str("transport is closed"),
            Self::Backpressure(addr) => write!(f, "outbound queue full for peer {addr}"),
            Self::Protocol(err) => err.fmt(f),
            Self::Backend(msg) => write!(f, "transport backend error: {msg}"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<ProtocolError> for TransportError {
    fn from(value: ProtocolError) -> Self {
        Self::Protocol(value)
    }
}
