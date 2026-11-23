//! Deterministic co-op multiplayer lockstep protocol and `iroh` P2P transport.
//!
//! Per `docs/vision.md` ("Multiplayer (v1)"), co-op multiplayer runs the same
//! deterministic simulation on every machine on one shared lot, with shared
//! money and clock:
//!
//! - [`protocol`]: Wire-clean `serde`/`postcard` messages, commands, tick
//!   batches, per-player simulation inputs (`Selected Person ID`,
//!   `camera_rotation`, Live/Buy/Build mode), and framing helpers.
//! - [`manifest`]: Deterministic SHA-256 file manifest digest over game files
//!   and custom content so mismatched installs are rejected cleanly on join.
//! - [`snapshot`]: Bounded, compressed, chunked REDB snapshot transfer framing
//!   for mid-game join and drift recovery.
//! - [`session`]: Pure deterministic session state (multi-reason pause, Mode
//!   Global 19 aggregation, host speed lock & attribution, first-answer-wins
//!   blocking dialogs, per-player selection/rotation resolution) and the
//!   host-sequenced [`session::LockstepNode`] scheduler with adaptive pacing,
//!   periodic fingerprint comparison, drift resync, and deterministic host
//!   failover.
//! - [`transport`]: Non-blocking channel-oriented [`transport::Transport`]
//!   trait, a deterministic in-memory simulated mesh ([`transport::MemoryMesh`])
//!   supporting latency/drop/reorder/disconnect injection, and a real
//!   [`transport::IrohTransport`] over `iroh` QUIC streams with invite tickets.
//!
//! Nothing in [`protocol`], [`manifest`], [`snapshot`], [`session`], or
//! [`transport::MemoryMesh`] reads wall clocks or uses `HashMap`/`HashSet`.

pub mod error;
pub mod manifest;
pub mod protocol;
pub mod session;
pub mod snapshot;
pub mod transport;

pub use error::{
    ManifestError, ProtocolError, SessionError, SnapshotError, TicketError, TransportError,
};
pub use manifest::{FileManifest, FileManifestEntry, ManifestDiff};
pub use protocol::{
    ALPN, BuyBuildEdit, CameraRotation, Command, CommandSeq, DEFAULT_SNAPSHOT_CHUNK_BYTES,
    DialogAnswer, DialogId, Epoch, LeaveReason, MAX_WIRE_FRAME_BYTES, ManifestDigest,
    PROTOCOL_VERSION, PeerRosterEntry, PlayerId, PlayerMode, PlayerRole, RejectReason,
    ResyncReason, SessionConfig, SimSpeed, SnapshotId, StampedCommand, StateFingerprint, Tick,
    TickBatch, WireMessage, decode_framed, decode_payload, encode_framed, encode_payload,
    protocol_mismatch_message,
};
pub use session::{
    ActivePauseReasons, AppliedBatch, AppliedCommand, BlockingDialog, CommandIgnoreReason,
    CommandOutcome, CommandRejectReason, HostStepResult, LockstepNode, NodeEvent, PerPlayerState,
    SessionState, TimeControlState,
};
pub use snapshot::{
    CompletedSnapshot, MAX_SNAPSHOT_CHUNK_BYTES, MAX_SNAPSHOT_CHUNKS,
    MAX_SNAPSHOT_UNCOMPRESSED_BYTES, SnapshotChunk, SnapshotCompression, SnapshotEnvelope,
    SnapshotHeader, SnapshotReceiver, SnapshotSender,
};
pub use transport::{
    InviteTicket, IrohConfig, IrohTransport, LinkConfig, MemoryMesh, MemoryNodeTransport,
    PeerAddress, Transport, TransportEvent,
};
