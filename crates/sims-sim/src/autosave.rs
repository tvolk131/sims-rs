//! Crash-safe manual & rotating autosave backend backed by REDB.
//!
//! Stores per-`(neighborhood_id, lot_id)` manual saves and a configurable
//! rotating autosave ring inside REDB tables that can either share the same
//! `.redb` database handle as [`crate::NeighborhoodStore`] (via
//! [`crate::NeighborhoodStore::autosaves`]) or live in a standalone REDB file
//! ([`AutosaveStore::open`]).
//!
//! ## Crash-Safety & Non-Destructive Guarantees
//! - Every save record carries a 4-byte magic (`"SAVG"`), a 4-byte little-endian
//!   schema version (`AUTOSAVE_FORMAT`), a 32-byte record SHA-256 covering the
//!   version and serialized body (metadata + payload), and an inner payload
//!   SHA-256 + key-binding check (`neighborhood_id`, `lot_id`, `slot`).
//! - Writing an autosave allocates a strictly monotonic `autosave_seq`
//!   (`max(stored_seq, max_existing_ring_seq) + 1`), inserts the new slot, and
//!   prunes only oldest *intact* slots beyond `policy.max_autosaves` inside the
//!   same atomic REDB write transaction. Manual slots and corrupt/unrecognized
//!   slots are never modified or deleted by autosave writes or pruning.
//! - Writing a manual save (`SaveTarget::Manual`) validates any existing manual
//!   slot first and refuses to overwrite a corrupt or unsupported-version
//!   manual save unless the caller explicitly requests
//!   [`SaveTarget::ManualForceReplace`].
//! - Reading or listing a missing, corrupt, or unsupported-version save returns
//!   a clear typed [`SaveError`] and never deletes or replaces any record.
//! - [`AutosaveStore`] is `Clone + Send + Sync` and [`SaveJob`]/[`SavePayload`]
//!   are fully owned (`Send + 'static`), so a main thread can clone a CoW
//!   [`World`] snapshot (~4.25 µs) and hand off encoding, hashing, and REDB
//!   commits to a background worker thread with zero borrows.

use crate::migration::decode_postcard_exact;
use crate::neighborhood_store::{NEIGHBORHOODS as NEIGHBORHOODS_TABLE, StoredNeighborhood};
use crate::{Error, World};
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use postcard::ser_flavors::ExtendFlavor;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

/// 4-byte magic prefix identifying an autosave/manual-save record envelope.
const AUTOSAVE_MAGIC: [u8; 4] = *b"SAVG";

/// Domain separator for the outer record SHA-256 checksum.
const RECORD_HASH_DOMAIN: &[u8] = b"sims-sim:autosave-record:v1";

/// Length of the fixed envelope prefix: 4-byte magic + 4-byte version +
/// 32-byte record SHA-256.
const RECORD_HEADER_LEN: usize = 4 + 4 + 32;

/// Upper bound on decompressed V2 autosave payload size (128 MiB) to guard
/// against malformed compressed payloads.
const MAX_AUTOSAVE_PAYLOAD_BYTES: usize = 128 * 1024 * 1024;

/// Earliest supported autosave record format version.
pub const AUTOSAVE_MIN_FORMAT: u32 = 1;

/// Current autosave record format version (V2: deflate-compressed payload with
/// split metadata header for fast metadata-only listing).
pub const AUTOSAVE_FORMAT: u32 = 2;

/// Default maximum number of rotating autosave slots kept per
/// `(neighborhood_id, lot_id)`.
pub const DEFAULT_MAX_AUTOSAVES: usize = 5;

/// Manual save slot keyed by `(neighborhood_id, lot_id)`.
const AUTOSAVE_MANUAL: TableDefinition<(&str, u32), &[u8]> =
    TableDefinition::new("autosave_manual");

/// Rotating autosave ring keyed by `(neighborhood_id, lot_id, autosave_seq)`.
const AUTOSAVE_RING: TableDefinition<(&str, u32, u64), &[u8]> =
    TableDefinition::new("autosave_ring");

/// Monotonic autosave sequence counter keyed by `(neighborhood_id, lot_id)`.
const AUTOSAVE_SEQ: TableDefinition<(&str, u32), u64> = TableDefinition::new("autosave_seq");

/// Retention policy for [`AutosaveStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutosavePolicy {
    /// Maximum number of intact rotating autosave slots retained per
    /// `(neighborhood_id, lot_id)`. Must be at least `1`.
    pub max_autosaves: usize,
}

impl Default for AutosavePolicy {
    fn default() -> Self {
        Self {
            max_autosaves: DEFAULT_MAX_AUTOSAVES,
        }
    }
}

impl AutosavePolicy {
    /// Constructs a validated retention policy, rejecting `max_autosaves == 0`.
    pub fn new(max_autosaves: usize) -> std::result::Result<Self, SaveError> {
        if max_autosaves == 0 {
            return Err(SaveError::InvalidInput(
                "max_autosaves must be at least 1".into(),
            ));
        }
        Ok(Self { max_autosaves })
    }
}

/// Identifies a persisted save slot for a `(neighborhood_id, lot_id)` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SaveSlot {
    /// Single player-triggered manual save slot.
    Manual,
    /// One slot in the rotating autosave ring, identified by its strictly
    /// monotonic sequence number (`>= 1`).
    Autosave(u64),
}

impl fmt::Display for SaveSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manual => write!(f, "manual"),
            Self::Autosave(seq) => write!(f, "autosave#{seq}"),
        }
    }
}

/// Tag describing the serialized bytes inside a [`LoadedSave`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SavePayloadKind {
    /// Encoded [`World`] snapshot ([`World::encode_snapshot`]: the world's
    /// canonical encoding, then its pending indicator refreshes).
    WorldSnapshot,
    /// Encoded `sims_vm::boot::save::Saved` lot bytes (`Saved::encode`).
    SavedLot,
    /// Caller-defined payload format tag.
    Custom(String),
}

/// Owned payload handed to a background save worker via [`SaveJob`].
#[derive(Debug, Clone)]
pub enum SavePayload {
    /// Cloned CoW [`World`] snapshot; encoded ([`World::encode_snapshot`]) on the worker
    /// thread rather than blocking the simulation thread.
    World(Box<World>),
    /// Pre-encoded `sims_vm::boot::save::Saved` lot bytes (`Saved::encode`).
    SavedLot(Vec<u8>),
    /// Pre-encoded custom payload with a format tag.
    Custom { kind: String, bytes: Vec<u8> },
}

impl SavePayload {
    /// Wraps an owned [`World`] snapshot (typically a cheap CoW `world.clone()`).
    #[must_use]
    pub fn from_world(world: World) -> Self {
        Self::World(Box::new(world))
    }

    /// Wraps pre-encoded `Saved` lot bytes.
    #[must_use]
    pub fn saved_lot(bytes: Vec<u8>) -> Self {
        Self::SavedLot(bytes)
    }

    /// Wraps custom payload bytes under a caller-chosen format tag.
    #[must_use]
    pub fn custom(kind: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self::Custom {
            kind: kind.into(),
            bytes,
        }
    }

    fn into_encoded(self) -> (SavePayloadKind, Vec<u8>) {
        match self {
            Self::World(world) => (SavePayloadKind::WorldSnapshot, world.encode_snapshot()),
            Self::SavedLot(bytes) => (SavePayloadKind::SavedLot, bytes),
            Self::Custom { kind, bytes } => (SavePayloadKind::Custom(kind), bytes),
        }
    }
}

/// Destination behavior for a [`SaveJob`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveTarget {
    /// Writes to the manual save slot, refusing to silently overwrite a corrupt
    /// or unsupported-version manual save.
    Manual,
    /// Explicitly replaces the manual save slot even if the previous manual
    /// record was corrupt or written by an unsupported version.
    ManualForceReplace,
    /// Appends a new monotonic autosave slot and atomically prunes oldest
    /// intact autosaves beyond `policy.max_autosaves`.
    Autosave,
}

/// Self-contained, `Send + 'static` save request for background worker threads.
#[derive(Debug, Clone)]
pub struct SaveJob {
    pub neighborhood_id: String,
    pub lot_id: u32,
    /// Expected `StoredNeighborhood::instance_id` for lifetime fencing inside
    /// the REDB write transaction. Required on shared [`crate::NeighborhoodStore`]
    /// handles so a queued background job cannot commit after its neighborhood
    /// was deleted or re-imported.
    pub expected_instance_id: Option<u128>,
    pub target: SaveTarget,
    /// Logical simulation tick at which the snapshot was captured.
    pub sim_tick: u64,
    /// Optional caller-supplied Unix timestamp (seconds). `sims-sim` never
    /// reads the wall clock directly so simulation crates stay deterministic.
    pub saved_at_unix_secs: Option<u64>,
    /// Optional human-readable label (e.g. household name or day/time).
    pub label: String,
    pub payload: SavePayload,
}

impl SaveJob {
    /// Creates an autosave job for `(neighborhood_id, lot_id)` (standalone
    /// store usage, or chain [`Self::with_expected_instance_id`] for a shared
    /// [`crate::NeighborhoodStore`]).
    #[must_use]
    pub fn autosave(
        neighborhood_id: impl Into<String>,
        lot_id: u32,
        sim_tick: u64,
        payload: SavePayload,
    ) -> Self {
        Self {
            neighborhood_id: neighborhood_id.into(),
            lot_id,
            expected_instance_id: None,
            target: SaveTarget::Autosave,
            sim_tick,
            saved_at_unix_secs: None,
            label: String::new(),
            payload,
        }
    }

    /// Creates an autosave job fenced to `expected_instance_id` for a shared
    /// [`crate::NeighborhoodStore`].
    #[must_use]
    pub fn autosave_for_instance(
        neighborhood_id: impl Into<String>,
        expected_instance_id: u128,
        lot_id: u32,
        sim_tick: u64,
        payload: SavePayload,
    ) -> Self {
        Self::autosave(neighborhood_id, lot_id, sim_tick, payload)
            .with_expected_instance_id(expected_instance_id)
    }

    /// Creates a manual save job for `(neighborhood_id, lot_id)`.
    #[must_use]
    pub fn manual(
        neighborhood_id: impl Into<String>,
        lot_id: u32,
        sim_tick: u64,
        payload: SavePayload,
    ) -> Self {
        Self {
            neighborhood_id: neighborhood_id.into(),
            lot_id,
            expected_instance_id: None,
            target: SaveTarget::Manual,
            sim_tick,
            saved_at_unix_secs: None,
            label: String::new(),
            payload,
        }
    }

    /// Creates a manual save job fenced to `expected_instance_id` for a shared
    /// [`crate::NeighborhoodStore`].
    #[must_use]
    pub fn manual_for_instance(
        neighborhood_id: impl Into<String>,
        expected_instance_id: u128,
        lot_id: u32,
        sim_tick: u64,
        payload: SavePayload,
    ) -> Self {
        Self::manual(neighborhood_id, lot_id, sim_tick, payload)
            .with_expected_instance_id(expected_instance_id)
    }

    /// Creates a user-confirmed manual save job that may replace a corrupt or
    /// unsupported-version manual record.
    #[must_use]
    pub fn manual_force_replace(
        neighborhood_id: impl Into<String>,
        lot_id: u32,
        sim_tick: u64,
        payload: SavePayload,
    ) -> Self {
        Self {
            neighborhood_id: neighborhood_id.into(),
            lot_id,
            expected_instance_id: None,
            target: SaveTarget::ManualForceReplace,
            sim_tick,
            saved_at_unix_secs: None,
            label: String::new(),
            payload,
        }
    }

    /// Creates a user-confirmed manual force-replace job fenced to
    /// `expected_instance_id` for a shared [`crate::NeighborhoodStore`].
    #[must_use]
    pub fn manual_force_replace_for_instance(
        neighborhood_id: impl Into<String>,
        expected_instance_id: u128,
        lot_id: u32,
        sim_tick: u64,
        payload: SavePayload,
    ) -> Self {
        Self::manual_force_replace(neighborhood_id, lot_id, sim_tick, payload)
            .with_expected_instance_id(expected_instance_id)
    }

    /// Binds this job to a specific [`crate::StoredNeighborhood::instance_id`]
    /// verified atomically inside the REDB write transaction before mutating
    /// any slot.
    #[must_use]
    pub const fn with_expected_instance_id(mut self, instance_id: u128) -> Self {
        self.expected_instance_id = Some(instance_id);
        self
    }

    /// Attaches a human-readable label to this save job.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Attaches a caller-supplied Unix timestamp (seconds) to this save job.
    #[must_use]
    pub const fn with_saved_at_unix_secs(mut self, unix_secs: u64) -> Self {
        self.saved_at_unix_secs = Some(unix_secs);
        self
    }
}

/// Verified metadata for a committed or loaded save slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveMetadata {
    pub slot: SaveSlot,
    pub neighborhood_id: String,
    pub lot_id: u32,
    pub sim_tick: u64,
    pub saved_at_unix_secs: Option<u64>,
    pub label: String,
    pub payload_kind: SavePayloadKind,
    pub payload_len: usize,
    pub payload_sha256: [u8; 32],
    pub format_version: u32,
}

/// Verified save record loaded from [`AutosaveStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSave {
    pub metadata: SaveMetadata,
    pub payload: Vec<u8>,
}

impl LoadedSave {
    /// Decodes `self.payload` as a [`World`] when `metadata.payload_kind` is
    /// [`SavePayloadKind::WorldSnapshot`].
    pub fn decode_world(&self) -> std::result::Result<World, SaveError> {
        if self.metadata.payload_kind != SavePayloadKind::WorldSnapshot {
            return Err(SaveError::DecodeFailed {
                neighborhood_id: self.metadata.neighborhood_id.clone(),
                lot_id: self.metadata.lot_id,
                slot: self.metadata.slot,
                detail: format!(
                    "expected WorldSnapshot payload, found {:?}",
                    self.metadata.payload_kind
                ),
            });
        }
        World::decode_snapshot(&self.payload).map_err(|error| SaveError::DecodeFailed {
            neighborhood_id: self.metadata.neighborhood_id.clone(),
            lot_id: self.metadata.lot_id,
            slot: self.metadata.slot,
            detail: format!("invalid world snapshot payload: {error}"),
        })
    }
}

/// Entry returned by [`AutosaveStore::list_saves`].
///
/// Corrupt or unsupported-version slots are surfaced inline without modifying
/// or deleting them, and without hiding intact sibling slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveSlotEntry {
    Intact(SaveMetadata),
    Corrupt { slot: SaveSlot, error: SaveError },
}

impl SaveSlotEntry {
    /// Slot identifier for this entry.
    #[must_use]
    pub const fn slot(&self) -> SaveSlot {
        match self {
            Self::Intact(meta) => meta.slot,
            Self::Corrupt { slot, .. } => *slot,
        }
    }

    /// Returns metadata if this entry is intact.
    #[must_use]
    pub const fn metadata(&self) -> Option<&SaveMetadata> {
        match self {
            Self::Intact(meta) => Some(meta),
            Self::Corrupt { .. } => None,
        }
    }
}

/// Outcome of loading the newest intact save via fallback recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryLoad {
    /// The intact save that was loaded.
    pub loaded: LoadedSave,
    /// Any newer/preferred candidate slots that failed validation and were
    /// skipped (left untouched in REDB for diagnosis).
    pub skipped_corrupt: Vec<(SaveSlot, SaveError)>,
}

/// Typed errors returned by [`AutosaveStore`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// Requested save slot (or any save for the lot) does not exist.
    Missing {
        neighborhood_id: String,
        lot_id: u32,
        slot: Option<SaveSlot>,
    },
    /// SHA-256 checksum verification failed (bit-flip, truncation, tampering).
    CorruptChecksum {
        neighborhood_id: String,
        lot_id: u32,
        slot: SaveSlot,
        expected: [u8; 32],
        actual: [u8; 32],
    },
    /// Save record was written by an unsupported or newer format version.
    UnsupportedVersion {
        neighborhood_id: String,
        lot_id: u32,
        slot: SaveSlot,
        found: u32,
        min_supported: u32,
        max_supported: u32,
    },
    /// Save record header, key binding, or postcard body could not be decoded.
    DecodeFailed {
        neighborhood_id: String,
        lot_id: u32,
        slot: SaveSlot,
        detail: String,
    },
    /// Neighborhood was deleted or replaced by a different instance before a
    /// queued save job committed.
    StaleNeighborhoodInstance {
        neighborhood_id: String,
        expected_instance_id: u128,
        current_instance_id: Option<u128>,
    },
    /// Caller supplied invalid input (e.g. empty neighborhood ID or zero cap).
    InvalidInput(String),
    /// Underlying REDB storage or transaction error.
    Store(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing {
                neighborhood_id,
                lot_id,
                slot: Some(slot),
            } => write!(
                f,
                "missing save slot {slot} for neighborhood {neighborhood_id:?} lot {lot_id}"
            ),
            Self::Missing {
                neighborhood_id,
                lot_id,
                slot: None,
            } => write!(
                f,
                "no saves found for neighborhood {neighborhood_id:?} lot {lot_id}"
            ),
            Self::CorruptChecksum {
                neighborhood_id,
                lot_id,
                slot,
                ..
            } => write!(
                f,
                "corrupt save checksum in {slot} for neighborhood {neighborhood_id:?} lot {lot_id}"
            ),
            Self::UnsupportedVersion {
                neighborhood_id,
                lot_id,
                slot,
                found,
                min_supported,
                max_supported,
            } => write!(
                f,
                "unsupported save format {found} in {slot} for neighborhood {neighborhood_id:?} lot {lot_id} (supported {min_supported}..={max_supported})"
            ),
            Self::DecodeFailed {
                neighborhood_id,
                lot_id,
                slot,
                detail,
            } => write!(
                f,
                "failed to decode {slot} for neighborhood {neighborhood_id:?} lot {lot_id}: {detail}"
            ),
            Self::StaleNeighborhoodInstance {
                neighborhood_id,
                expected_instance_id,
                current_instance_id: Some(current),
            } => write!(
                f,
                "stale neighborhood instance for {neighborhood_id:?}: expected {expected_instance_id:#034x}, current {current:#034x}"
            ),
            Self::StaleNeighborhoodInstance {
                neighborhood_id,
                expected_instance_id,
                current_instance_id: None,
            } => write!(
                f,
                "stale neighborhood instance for {neighborhood_id:?}: expected {expected_instance_id:#034x}, neighborhood no longer exists"
            ),
            Self::InvalidInput(message) => write!(f, "invalid save input: {message}"),
            Self::Store(message) => write!(f, "autosave store error: {message}"),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<SaveError> for Error {
    fn from(error: SaveError) -> Self {
        match error {
            SaveError::Store(message) | SaveError::InvalidInput(message) => Error::Store(message),
            other => Error::Snapshot(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AutosaveBodyV1 {
    slot: SaveSlot,
    neighborhood_id: String,
    lot_id: u32,
    sim_tick: u64,
    saved_at_unix_secs: Option<u64>,
    label: String,
    payload_kind: SavePayloadKind,
    payload_sha256: [u8; 32],
    payload: Vec<u8>,
}

/// Metadata prefix of a V2 autosave record, followed immediately in the
/// checksummed body by raw deflate (`flate2::Compression::fast()`) bytes of
/// `payload`. Splitting the header from the compressed payload allows
/// [`AutosaveStore::list_saves_metadata`] to verify the record checksum and
/// read metadata without decompressing or decoding the lot payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AutosaveHeaderV2 {
    slot: SaveSlot,
    neighborhood_id: String,
    lot_id: u32,
    sim_tick: u64,
    saved_at_unix_secs: Option<u64>,
    label: String,
    payload_kind: SavePayloadKind,
    payload_len: u64,
    payload_sha256: [u8; 32],
}

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn record_checksum(version: u32, body: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(RECORD_HASH_DOMAIN);
    hasher.update(version.to_le_bytes());
    hasher.update(body);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn encode_record_v2(
    header: &AutosaveHeaderV2,
    payload: &[u8],
) -> std::result::Result<Vec<u8>, SaveError> {
    if payload.len() > MAX_AUTOSAVE_PAYLOAD_BYTES {
        return Err(SaveError::InvalidInput(format!(
            "save payload length {} exceeds maximum {MAX_AUTOSAVE_PAYLOAD_BYTES}",
            payload.len()
        )));
    }
    let mut out = Vec::with_capacity(RECORD_HEADER_LEN + 256 + payload.len() / 4);
    out.extend_from_slice(&AUTOSAVE_MAGIC);
    out.extend_from_slice(&AUTOSAVE_FORMAT.to_le_bytes());
    out.extend_from_slice(&[0u8; 32]);
    let mut out =
        postcard::serialize_with_flavor(header, ExtendFlavor::new(out)).map_err(|error| {
            SaveError::DecodeFailed {
                neighborhood_id: header.neighborhood_id.clone(),
                lot_id: header.lot_id,
                slot: header.slot,
                detail: format!("failed to encode save record header: {error}"),
            }
        })?;
    {
        let mut encoder = DeflateEncoder::new(&mut out, Compression::fast());
        encoder
            .write_all(payload)
            .and_then(|()| encoder.finish().map(|_| ()))
            .map_err(|error| SaveError::DecodeFailed {
                neighborhood_id: header.neighborhood_id.clone(),
                lot_id: header.lot_id,
                slot: header.slot,
                detail: format!("failed to compress save record payload: {error}"),
            })?;
    }
    let checksum = record_checksum(AUTOSAVE_FORMAT, &out[RECORD_HEADER_LEN..]);
    out[8..40].copy_from_slice(&checksum);
    Ok(out)
}

fn verify_envelope<'a>(
    neighborhood_id: &str,
    lot_id: u32,
    slot: SaveSlot,
    bytes: &'a [u8],
) -> std::result::Result<(u32, &'a [u8]), SaveError> {
    if bytes.len() < 8 {
        return Err(SaveError::DecodeFailed {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            detail: format!("record shorter than 8-byte header ({} bytes)", bytes.len()),
        });
    }
    if bytes[0..4] != AUTOSAVE_MAGIC {
        return Err(SaveError::DecodeFailed {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            detail: "invalid autosave record magic".into(),
        });
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
    if !(AUTOSAVE_MIN_FORMAT..=AUTOSAVE_FORMAT).contains(&version) {
        return Err(SaveError::UnsupportedVersion {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            found: version,
            min_supported: AUTOSAVE_MIN_FORMAT,
            max_supported: AUTOSAVE_FORMAT,
        });
    }
    if bytes.len() < RECORD_HEADER_LEN {
        return Err(SaveError::DecodeFailed {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            detail: format!(
                "record shorter than {RECORD_HEADER_LEN}-byte header ({} bytes)",
                bytes.len()
            ),
        });
    }
    let expected_record_sha: [u8; 32] = bytes[8..40].try_into().expect("32 bytes");
    let body_bytes = &bytes[RECORD_HEADER_LEN..];
    let actual_record_sha = record_checksum(version, body_bytes);
    if actual_record_sha != expected_record_sha {
        return Err(SaveError::CorruptChecksum {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            expected: expected_record_sha,
            actual: actual_record_sha,
        });
    }
    Ok((version, body_bytes))
}

fn check_key_binding(
    stored_neighborhood: &str,
    stored_lot: u32,
    stored_slot: SaveSlot,
    neighborhood_id: &str,
    lot_id: u32,
    slot: SaveSlot,
) -> std::result::Result<(), SaveError> {
    if stored_neighborhood != neighborhood_id || stored_lot != lot_id || stored_slot != slot {
        return Err(SaveError::DecodeFailed {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            detail: format!(
                "record key binding mismatch: stored ({stored_neighborhood:?}, lot {stored_lot}, {stored_slot}), expected ({neighborhood_id:?}, lot {lot_id}, {slot})"
            ),
        });
    }
    Ok(())
}

fn decompress_v2_payload(
    neighborhood_id: &str,
    lot_id: u32,
    slot: SaveSlot,
    compressed: &[u8],
    expected_len: usize,
) -> std::result::Result<Vec<u8>, SaveError> {
    if expected_len > MAX_AUTOSAVE_PAYLOAD_BYTES {
        return Err(SaveError::DecodeFailed {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            detail: format!(
                "declared payload length {expected_len} exceeds maximum {MAX_AUTOSAVE_PAYLOAD_BYTES}"
            ),
        });
    }
    let mut decoder = DeflateDecoder::new(compressed).take((expected_len as u64).saturating_add(1));
    let mut payload = Vec::with_capacity(expected_len.min(1024 * 1024));
    decoder
        .read_to_end(&mut payload)
        .map_err(|error| SaveError::DecodeFailed {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            detail: format!("failed to decompress V2 save payload: {error}"),
        })?;
    if payload.len() != expected_len {
        return Err(SaveError::DecodeFailed {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            detail: format!(
                "decompressed V2 payload length {} does not match header length {expected_len}",
                payload.len()
            ),
        });
    }
    let consumed = decoder.into_inner().total_in();
    if consumed != compressed.len() as u64 {
        return Err(SaveError::DecodeFailed {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            detail: format!(
                "V2 compressed payload has trailing bytes (consumed {consumed} of {} bytes)",
                compressed.len()
            ),
        });
    }
    Ok(payload)
}

fn noop_payload_validator(
    _kind: &SavePayloadKind,
    _payload_sha256: &[u8; 32],
    _bytes: &[u8],
) -> std::result::Result<(), String> {
    Ok(())
}

fn validate_payload_bytes_with_sha256<F>(
    neighborhood_id: &str,
    lot_id: u32,
    slot: SaveSlot,
    kind: &SavePayloadKind,
    payload_sha256: &[u8; 32],
    payload: &[u8],
    validator: &F,
) -> std::result::Result<(), SaveError>
where
    F: Fn(&SavePayloadKind, &[u8; 32], &[u8]) -> std::result::Result<(), String>,
{
    if *kind == SavePayloadKind::WorldSnapshot {
        World::decode_snapshot(payload).map_err(|error| SaveError::DecodeFailed {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            detail: format!("invalid world snapshot payload: {error}"),
        })?;
    }
    validator(kind, payload_sha256, payload).map_err(|detail| SaveError::DecodeFailed {
        neighborhood_id: neighborhood_id.to_owned(),
        lot_id,
        slot,
        detail,
    })
}

fn decode_record_metadata(
    neighborhood_id: &str,
    lot_id: u32,
    slot: SaveSlot,
    bytes: &[u8],
) -> std::result::Result<SaveMetadata, SaveError> {
    let (version, body_bytes) = verify_envelope(neighborhood_id, lot_id, slot, bytes)?;
    match version {
        1 => {
            let record_name = format!("{neighborhood_id}:lot{lot_id}:{slot}");
            let body: AutosaveBodyV1 = decode_postcard_exact(&record_name, version, body_bytes)
                .map_err(|error| SaveError::DecodeFailed {
                    neighborhood_id: neighborhood_id.to_owned(),
                    lot_id,
                    slot,
                    detail: error.to_string(),
                })?;
            let actual_payload_sha = sha256_bytes(&body.payload);
            if actual_payload_sha != body.payload_sha256 {
                return Err(SaveError::CorruptChecksum {
                    neighborhood_id: neighborhood_id.to_owned(),
                    lot_id,
                    slot,
                    expected: body.payload_sha256,
                    actual: actual_payload_sha,
                });
            }
            check_key_binding(
                &body.neighborhood_id,
                body.lot_id,
                body.slot,
                neighborhood_id,
                lot_id,
                slot,
            )?;
            Ok(SaveMetadata {
                slot: body.slot,
                neighborhood_id: body.neighborhood_id,
                lot_id: body.lot_id,
                sim_tick: body.sim_tick,
                saved_at_unix_secs: body.saved_at_unix_secs,
                label: body.label,
                payload_kind: body.payload_kind,
                payload_len: body.payload.len(),
                payload_sha256: body.payload_sha256,
                format_version: version,
            })
        }
        2 => {
            let mut de = postcard::Deserializer::from_bytes(body_bytes);
            let header = AutosaveHeaderV2::deserialize(&mut de).map_err(|error| {
                SaveError::DecodeFailed {
                    neighborhood_id: neighborhood_id.to_owned(),
                    lot_id,
                    slot,
                    detail: format!("failed to decode V2 save header: {error}"),
                }
            })?;
            check_key_binding(
                &header.neighborhood_id,
                header.lot_id,
                header.slot,
                neighborhood_id,
                lot_id,
                slot,
            )?;
            let payload_len =
                usize::try_from(header.payload_len).map_err(|_| SaveError::DecodeFailed {
                    neighborhood_id: neighborhood_id.to_owned(),
                    lot_id,
                    slot,
                    detail: format!("V2 payload length {} overflows usize", header.payload_len),
                })?;
            if payload_len > MAX_AUTOSAVE_PAYLOAD_BYTES {
                return Err(SaveError::DecodeFailed {
                    neighborhood_id: neighborhood_id.to_owned(),
                    lot_id,
                    slot,
                    detail: format!(
                        "declared payload length {payload_len} exceeds maximum {MAX_AUTOSAVE_PAYLOAD_BYTES}"
                    ),
                });
            }
            Ok(SaveMetadata {
                slot: header.slot,
                neighborhood_id: header.neighborhood_id,
                lot_id: header.lot_id,
                sim_tick: header.sim_tick,
                saved_at_unix_secs: header.saved_at_unix_secs,
                label: header.label,
                payload_kind: header.payload_kind,
                payload_len,
                payload_sha256: header.payload_sha256,
                format_version: version,
            })
        }
        _ => Err(SaveError::UnsupportedVersion {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            found: version,
            min_supported: AUTOSAVE_MIN_FORMAT,
            max_supported: AUTOSAVE_FORMAT,
        }),
    }
}

fn decode_record_validated_with_sha256<F>(
    neighborhood_id: &str,
    lot_id: u32,
    slot: SaveSlot,
    bytes: &[u8],
    validator: &F,
) -> std::result::Result<LoadedSave, SaveError>
where
    F: Fn(&SavePayloadKind, &[u8; 32], &[u8]) -> std::result::Result<(), String>,
{
    let (version, body_bytes) = verify_envelope(neighborhood_id, lot_id, slot, bytes)?;
    match version {
        1 => {
            let record_name = format!("{neighborhood_id}:lot{lot_id}:{slot}");
            let body: AutosaveBodyV1 = decode_postcard_exact(&record_name, version, body_bytes)
                .map_err(|error| SaveError::DecodeFailed {
                    neighborhood_id: neighborhood_id.to_owned(),
                    lot_id,
                    slot,
                    detail: error.to_string(),
                })?;
            let actual_payload_sha = sha256_bytes(&body.payload);
            if actual_payload_sha != body.payload_sha256 {
                return Err(SaveError::CorruptChecksum {
                    neighborhood_id: neighborhood_id.to_owned(),
                    lot_id,
                    slot,
                    expected: body.payload_sha256,
                    actual: actual_payload_sha,
                });
            }
            check_key_binding(
                &body.neighborhood_id,
                body.lot_id,
                body.slot,
                neighborhood_id,
                lot_id,
                slot,
            )?;
            validate_payload_bytes_with_sha256(
                neighborhood_id,
                lot_id,
                slot,
                &body.payload_kind,
                &actual_payload_sha,
                &body.payload,
                validator,
            )?;
            let metadata = SaveMetadata {
                slot: body.slot,
                neighborhood_id: body.neighborhood_id,
                lot_id: body.lot_id,
                sim_tick: body.sim_tick,
                saved_at_unix_secs: body.saved_at_unix_secs,
                label: body.label,
                payload_kind: body.payload_kind,
                payload_len: body.payload.len(),
                payload_sha256: body.payload_sha256,
                format_version: version,
            };
            Ok(LoadedSave {
                metadata,
                payload: body.payload,
            })
        }
        2 => {
            let mut de = postcard::Deserializer::from_bytes(body_bytes);
            let header = AutosaveHeaderV2::deserialize(&mut de).map_err(|error| {
                SaveError::DecodeFailed {
                    neighborhood_id: neighborhood_id.to_owned(),
                    lot_id,
                    slot,
                    detail: format!("failed to decode V2 save header: {error}"),
                }
            })?;
            let compressed = de.finalize().map_err(|error| SaveError::DecodeFailed {
                neighborhood_id: neighborhood_id.to_owned(),
                lot_id,
                slot,
                detail: format!("failed to read V2 compressed payload: {error}"),
            })?;
            check_key_binding(
                &header.neighborhood_id,
                header.lot_id,
                header.slot,
                neighborhood_id,
                lot_id,
                slot,
            )?;
            let expected_len =
                usize::try_from(header.payload_len).map_err(|_| SaveError::DecodeFailed {
                    neighborhood_id: neighborhood_id.to_owned(),
                    lot_id,
                    slot,
                    detail: format!("V2 payload length {} overflows usize", header.payload_len),
                })?;
            let payload =
                decompress_v2_payload(neighborhood_id, lot_id, slot, compressed, expected_len)?;
            let actual_payload_sha = sha256_bytes(&payload);
            if actual_payload_sha != header.payload_sha256 {
                return Err(SaveError::CorruptChecksum {
                    neighborhood_id: neighborhood_id.to_owned(),
                    lot_id,
                    slot,
                    expected: header.payload_sha256,
                    actual: actual_payload_sha,
                });
            }
            validate_payload_bytes_with_sha256(
                neighborhood_id,
                lot_id,
                slot,
                &header.payload_kind,
                &actual_payload_sha,
                &payload,
                validator,
            )?;
            let metadata = SaveMetadata {
                slot: header.slot,
                neighborhood_id: header.neighborhood_id,
                lot_id: header.lot_id,
                sim_tick: header.sim_tick,
                saved_at_unix_secs: header.saved_at_unix_secs,
                label: header.label,
                payload_kind: header.payload_kind,
                payload_len: payload.len(),
                payload_sha256: header.payload_sha256,
                format_version: version,
            };
            Ok(LoadedSave { metadata, payload })
        }
        _ => Err(SaveError::UnsupportedVersion {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            found: version,
            min_supported: AUTOSAVE_MIN_FORMAT,
            max_supported: AUTOSAVE_FORMAT,
        }),
    }
}

fn verify_neighborhood_instance_in_txn(
    txn: &redb::WriteTransaction,
    neighborhood_id: &str,
    expected_instance_id: u128,
) -> std::result::Result<(), SaveError> {
    let nh_table = txn.open_table(NEIGHBORHOODS_TABLE).map_err(store_err)?;
    let Some(raw) = nh_table.get(neighborhood_id).map_err(store_err)? else {
        return Err(SaveError::StaleNeighborhoodInstance {
            neighborhood_id: neighborhood_id.to_owned(),
            expected_instance_id,
            current_instance_id: None,
        });
    };
    let nh = StoredNeighborhood::decode(neighborhood_id, raw.value()).map_err(|error| {
        SaveError::Store(format!(
            "cannot verify neighborhood {neighborhood_id:?} instance: {error}"
        ))
    })?;
    if nh.instance_id != expected_instance_id {
        return Err(SaveError::StaleNeighborhoodInstance {
            neighborhood_id: neighborhood_id.to_owned(),
            expected_instance_id,
            current_instance_id: Some(nh.instance_id),
        });
    }
    Ok(())
}

fn validate_neighborhood_id(neighborhood_id: &str) -> std::result::Result<(), SaveError> {
    if neighborhood_id.is_empty() {
        return Err(SaveError::InvalidInput(
            "neighborhood_id must not be empty".into(),
        ));
    }
    Ok(())
}

fn store_err(error: impl fmt::Display) -> SaveError {
    SaveError::Store(error.to_string())
}

/// Crash-safe manual & rotating autosave store backed by REDB.
///
/// Cloning an [`AutosaveStore`] clones its underlying `Arc<Database>` handle,
/// allowing background save worker threads to commit jobs concurrently with
/// main-thread readers and [`crate::NeighborhoodStore`] operations on the same
/// `.redb` file.
#[derive(Clone)]
pub struct AutosaveStore {
    db: Arc<Database>,
    policy: AutosavePolicy,
    require_neighborhood_instance: bool,
}

impl fmt::Debug for AutosaveStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AutosaveStore")
            .field("policy", &self.policy)
            .field(
                "require_neighborhood_instance",
                &self.require_neighborhood_instance,
            )
            .finish_non_exhaustive()
    }
}

impl AutosaveStore {
    /// Opens or creates a standalone REDB autosave store at `path` with
    /// [`AutosavePolicy::default`] (5 rotating autosaves per lot).
    pub fn open(path: &Path) -> std::result::Result<Self, SaveError> {
        Self::open_with_policy(path, AutosavePolicy::default())
    }

    /// Opens or creates a standalone REDB autosave store at `path` with an
    /// explicit [`AutosavePolicy`].
    pub fn open_with_policy(
        path: &Path,
        policy: AutosavePolicy,
    ) -> std::result::Result<Self, SaveError> {
        if policy.max_autosaves == 0 {
            return Err(SaveError::InvalidInput(
                "max_autosaves must be at least 1".into(),
            ));
        }
        let db = Database::create(path).map_err(store_err)?;
        Ok(Self {
            db: Arc::new(db),
            policy,
            require_neighborhood_instance: false,
        })
    }

    /// Constructs an [`AutosaveStore`] sharing an existing `Arc<Database>`
    /// (used by [`crate::NeighborhoodStore::autosaves`]).
    ///
    /// Shared handles require every committed [`SaveJob`] to carry
    /// `expected_instance_id` so queued background writes cannot cross a
    /// neighborhood delete or re-import boundary.
    pub(crate) fn from_shared_db(db: Arc<Database>, policy: AutosavePolicy) -> Self {
        let max_autosaves = policy.max_autosaves.max(1);
        Self {
            db,
            policy: AutosavePolicy { max_autosaves },
            require_neighborhood_instance: true,
        }
    }

    /// Returns this handle's active [`AutosavePolicy`].
    #[must_use]
    pub const fn policy(&self) -> AutosavePolicy {
        self.policy
    }

    /// Returns a cloned handle sharing the same REDB database with an updated
    /// [`AutosavePolicy`].
    pub fn with_policy(&self, policy: AutosavePolicy) -> std::result::Result<Self, SaveError> {
        if policy.max_autosaves == 0 {
            return Err(SaveError::InvalidInput(
                "max_autosaves must be at least 1".into(),
            ));
        }
        Ok(Self {
            db: Arc::clone(&self.db),
            policy,
            require_neighborhood_instance: self.require_neighborhood_instance,
        })
    }

    /// Executes an owned [`SaveJob`] atomically with built-in payload
    /// validation (`World::decode` for [`SavePayloadKind::WorldSnapshot`]).
    ///
    /// Payload encoding (`World::encode`), validation, and payload SHA-256
    /// hashing happen *before* acquiring the REDB write transaction so
    /// background workers hold the write lock for minimal duration.
    pub fn commit_job(&self, job: SaveJob) -> std::result::Result<SaveMetadata, SaveError> {
        self.commit_job_validated_with_sha256(job, noop_payload_validator)
    }

    /// Executes an owned [`SaveJob`] atomically, validating both the incoming
    /// payload before writing and existing slots during pruning with
    /// `payload_validator` (in addition to built-in `World::decode` checks for
    /// [`SavePayloadKind::WorldSnapshot`]).
    ///
    /// Use this when storing formats defined outside `sims-sim` (such as
    /// `sims_vm::boot::save::Saved`) so malformed payloads are rejected before
    /// writing and never evict usable slots.
    pub fn commit_job_validated<F>(
        &self,
        job: SaveJob,
        payload_validator: F,
    ) -> std::result::Result<SaveMetadata, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8]) -> std::result::Result<(), String>,
    {
        self.commit_job_validated_with_sha256(job, |kind, _sha256, bytes| {
            payload_validator(kind, bytes)
        })
    }

    /// Like [`Self::commit_job_validated`], passing the already-computed
    /// `payload_sha256` digest alongside each payload slice so callers with
    /// digest-keyed validation caches do not re-hash payloads.
    pub fn commit_job_validated_with_sha256<F>(
        &self,
        job: SaveJob,
        payload_validator: F,
    ) -> std::result::Result<SaveMetadata, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8; 32], &[u8]) -> std::result::Result<(), String>,
    {
        validate_neighborhood_id(&job.neighborhood_id)?;
        if self.policy.max_autosaves == 0 {
            return Err(SaveError::InvalidInput(
                "max_autosaves must be at least 1".into(),
            ));
        }
        if self.require_neighborhood_instance && job.expected_instance_id.is_none() {
            return Err(SaveError::InvalidInput(
                "shared NeighborhoodStore autosave jobs must specify expected_instance_id".into(),
            ));
        }
        if job.expected_instance_id == Some(0) {
            return Err(SaveError::InvalidInput(
                "expected_instance_id must be non-zero".into(),
            ));
        }

        let (payload_kind, payload_bytes) = job.payload.into_encoded();
        if payload_bytes.len() > MAX_AUTOSAVE_PAYLOAD_BYTES {
            return Err(SaveError::InvalidInput(format!(
                "save payload length {} exceeds maximum {MAX_AUTOSAVE_PAYLOAD_BYTES}",
                payload_bytes.len()
            )));
        }
        let payload_sha256 = sha256_bytes(&payload_bytes);
        let validation_slot = match job.target {
            SaveTarget::Manual | SaveTarget::ManualForceReplace => SaveSlot::Manual,
            SaveTarget::Autosave => SaveSlot::Autosave(0),
        };
        validate_payload_bytes_with_sha256(
            &job.neighborhood_id,
            job.lot_id,
            validation_slot,
            &payload_kind,
            &payload_sha256,
            &payload_bytes,
            &payload_validator,
        )?;
        let payload_len = payload_bytes.len();

        match job.target {
            SaveTarget::Manual => self.commit_manual_encoded(
                job.neighborhood_id,
                job.lot_id,
                job.expected_instance_id,
                job.sim_tick,
                job.saved_at_unix_secs,
                job.label,
                payload_kind,
                payload_sha256,
                payload_len,
                payload_bytes,
                false,
                &payload_validator,
            ),
            SaveTarget::ManualForceReplace => self.commit_manual_encoded(
                job.neighborhood_id,
                job.lot_id,
                job.expected_instance_id,
                job.sim_tick,
                job.saved_at_unix_secs,
                job.label,
                payload_kind,
                payload_sha256,
                payload_len,
                payload_bytes,
                true,
                &payload_validator,
            ),
            SaveTarget::Autosave => self.commit_autosave_encoded(
                job.neighborhood_id,
                job.lot_id,
                job.expected_instance_id,
                job.sim_tick,
                job.saved_at_unix_secs,
                job.label,
                payload_kind,
                payload_sha256,
                payload_len,
                payload_bytes,
                &payload_validator,
            ),
        }
    }

    /// Convenience wrapper that commits a [`SaveTarget::Manual`] job.
    pub fn save_manual(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        sim_tick: u64,
        saved_at_unix_secs: Option<u64>,
        label: &str,
        payload: SavePayload,
    ) -> std::result::Result<SaveMetadata, SaveError> {
        let mut job = SaveJob::manual(neighborhood_id, lot_id, sim_tick, payload).with_label(label);
        if let Some(ts) = saved_at_unix_secs {
            job = job.with_saved_at_unix_secs(ts);
        }
        self.commit_job(job)
    }

    /// Convenience wrapper that commits a [`SaveTarget::ManualForceReplace`] job.
    pub fn save_manual_force_replace(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        sim_tick: u64,
        saved_at_unix_secs: Option<u64>,
        label: &str,
        payload: SavePayload,
    ) -> std::result::Result<SaveMetadata, SaveError> {
        let mut job = SaveJob::manual_force_replace(neighborhood_id, lot_id, sim_tick, payload)
            .with_label(label);
        if let Some(ts) = saved_at_unix_secs {
            job = job.with_saved_at_unix_secs(ts);
        }
        self.commit_job(job)
    }

    /// Convenience wrapper that commits a [`SaveTarget::Autosave`] job.
    pub fn save_autosave(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        sim_tick: u64,
        saved_at_unix_secs: Option<u64>,
        label: &str,
        payload: SavePayload,
    ) -> std::result::Result<SaveMetadata, SaveError> {
        let mut job =
            SaveJob::autosave(neighborhood_id, lot_id, sim_tick, payload).with_label(label);
        if let Some(ts) = saved_at_unix_secs {
            job = job.with_saved_at_unix_secs(ts);
        }
        self.commit_job(job)
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_manual_encoded<F>(
        &self,
        neighborhood_id: String,
        lot_id: u32,
        expected_instance_id: Option<u128>,
        sim_tick: u64,
        saved_at_unix_secs: Option<u64>,
        label: String,
        payload_kind: SavePayloadKind,
        payload_sha256: [u8; 32],
        payload_len: usize,
        payload: Vec<u8>,
        force_replace: bool,
        payload_validator: &F,
    ) -> std::result::Result<SaveMetadata, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8; 32], &[u8]) -> std::result::Result<(), String>,
    {
        let header = AutosaveHeaderV2 {
            slot: SaveSlot::Manual,
            neighborhood_id: neighborhood_id.clone(),
            lot_id,
            sim_tick,
            saved_at_unix_secs,
            label: label.clone(),
            payload_kind: payload_kind.clone(),
            payload_len: payload_len as u64,
            payload_sha256,
        };
        let encoded = encode_record_v2(&header, &payload)?;

        let txn = self.db.begin_write().map_err(store_err)?;
        {
            if let Some(expected) = expected_instance_id {
                verify_neighborhood_instance_in_txn(&txn, &neighborhood_id, expected)?;
            }
            let mut table = txn.open_table(AUTOSAVE_MANUAL).map_err(store_err)?;
            if !force_replace
                && let Some(existing) = table
                    .get((neighborhood_id.as_str(), lot_id))
                    .map_err(store_err)?
            {
                // Refuse to silently replace a corrupt or future-version manual save.
                let _ = decode_record_validated_with_sha256(
                    &neighborhood_id,
                    lot_id,
                    SaveSlot::Manual,
                    existing.value(),
                    payload_validator,
                )?;
            }
            table
                .insert((neighborhood_id.as_str(), lot_id), encoded.as_slice())
                .map_err(store_err)?;
        }
        txn.commit().map_err(store_err)?;

        Ok(SaveMetadata {
            slot: SaveSlot::Manual,
            neighborhood_id,
            lot_id,
            sim_tick,
            saved_at_unix_secs,
            label,
            payload_kind,
            payload_len,
            payload_sha256,
            format_version: AUTOSAVE_FORMAT,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_autosave_encoded<F>(
        &self,
        neighborhood_id: String,
        lot_id: u32,
        expected_instance_id: Option<u128>,
        sim_tick: u64,
        saved_at_unix_secs: Option<u64>,
        label: String,
        payload_kind: SavePayloadKind,
        payload_sha256: [u8; 32],
        payload_len: usize,
        payload: Vec<u8>,
        payload_validator: &F,
    ) -> std::result::Result<SaveMetadata, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8; 32], &[u8]) -> std::result::Result<(), String>,
    {
        let txn = self.db.begin_write().map_err(store_err)?;
        let assigned_seq;
        {
            if let Some(expected) = expected_instance_id {
                verify_neighborhood_instance_in_txn(&txn, &neighborhood_id, expected)?;
            }
            let mut ring_table = txn.open_table(AUTOSAVE_RING).map_err(store_err)?;
            let mut seq_table = txn.open_table(AUTOSAVE_SEQ).map_err(store_err)?;

            let stored_seq = seq_table
                .get((neighborhood_id.as_str(), lot_id))
                .map_err(store_err)?
                .map_or(0u64, |v| v.value());

            // Inspect existing ring slots for this lot before writing so we
            // guarantee strictly monotonic sequence allocation AND identify
            // intact slots eligible for post-write pruning without ever
            // deleting corrupt or unsupported-version slots.
            let mut max_existing_seq = 0u64;
            let mut intact_existing_seqs = Vec::new();
            for entry in ring_table
                .range(
                    (neighborhood_id.as_str(), lot_id, 0u64)
                        ..=(neighborhood_id.as_str(), lot_id, u64::MAX),
                )
                .map_err(store_err)?
            {
                let (key, value) = entry.map_err(store_err)?;
                let seq = key.value().2;
                if seq > max_existing_seq {
                    max_existing_seq = seq;
                }
                if decode_record_validated_with_sha256(
                    &neighborhood_id,
                    lot_id,
                    SaveSlot::Autosave(seq),
                    value.value(),
                    payload_validator,
                )
                .is_ok()
                {
                    intact_existing_seqs.push(seq);
                }
            }

            let next_seq = stored_seq
                .max(max_existing_seq)
                .checked_add(1)
                .ok_or_else(|| {
                    SaveError::InvalidInput(format!(
                        "autosave sequence exhausted for ({neighborhood_id:?}, lot {lot_id})"
                    ))
                })?;
            assigned_seq = next_seq;

            let header = AutosaveHeaderV2 {
                slot: SaveSlot::Autosave(next_seq),
                neighborhood_id: neighborhood_id.clone(),
                lot_id,
                sim_tick,
                saved_at_unix_secs,
                label: label.clone(),
                payload_kind: payload_kind.clone(),
                payload_len: payload_len as u64,
                payload_sha256,
            };
            let encoded = encode_record_v2(&header, &payload)?;

            // 1. Insert the new slot and advance the monotonic counter first.
            ring_table
                .insert(
                    (neighborhood_id.as_str(), lot_id, next_seq),
                    encoded.as_slice(),
                )
                .map_err(store_err)?;
            seq_table
                .insert((neighborhood_id.as_str(), lot_id), next_seq)
                .map_err(store_err)?;

            // 2. Prune oldest *intact* slots beyond `policy.max_autosaves`
            //    in the same atomic REDB transaction. Corrupt or unrecognized
            //    slots are never deleted automatically.
            let total_intact = intact_existing_seqs.len() + 1;
            if total_intact > self.policy.max_autosaves {
                let drop_count = total_intact - self.policy.max_autosaves;
                for &old_seq in intact_existing_seqs.iter().take(drop_count) {
                    ring_table
                        .remove((neighborhood_id.as_str(), lot_id, old_seq))
                        .map_err(store_err)?;
                }
            }
        }
        txn.commit().map_err(store_err)?;

        Ok(SaveMetadata {
            slot: SaveSlot::Autosave(assigned_seq),
            neighborhood_id,
            lot_id,
            sim_tick,
            saved_at_unix_secs,
            label,
            payload_kind,
            payload_len,
            payload_sha256,
            format_version: AUTOSAVE_FORMAT,
        })
    }

    /// Lists all save slots for `(neighborhood_id, lot_id)` by verifying each
    /// record's outer SHA-256 envelope, version, and key-bound metadata header
    /// without decompressing V2 payloads or running full `World`/`Saved`
    /// deserialization.
    ///
    /// Corrupt or unsupported-version slots are reported inline as
    /// [`SaveSlotEntry::Corrupt`] without modifying or deleting any record.
    pub fn list_saves_metadata(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
    ) -> std::result::Result<Vec<SaveSlotEntry>, SaveError> {
        validate_neighborhood_id(neighborhood_id)?;
        let txn = self.db.begin_read().map_err(store_err)?;
        let mut entries = Vec::new();

        match txn.open_table(AUTOSAVE_MANUAL) {
            Ok(table) => {
                if let Some(value) = table.get((neighborhood_id, lot_id)).map_err(store_err)? {
                    match decode_record_metadata(
                        neighborhood_id,
                        lot_id,
                        SaveSlot::Manual,
                        value.value(),
                    ) {
                        Ok(metadata) => entries.push(SaveSlotEntry::Intact(metadata)),
                        Err(error) => entries.push(SaveSlotEntry::Corrupt {
                            slot: SaveSlot::Manual,
                            error,
                        }),
                    }
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store_err(error)),
        }

        match txn.open_table(AUTOSAVE_RING) {
            Ok(table) => {
                for item in table
                    .range((neighborhood_id, lot_id, 0u64)..=(neighborhood_id, lot_id, u64::MAX))
                    .map_err(store_err)?
                    .rev()
                {
                    let (key, value) = item.map_err(store_err)?;
                    let seq = key.value().2;
                    let slot = SaveSlot::Autosave(seq);
                    match decode_record_metadata(neighborhood_id, lot_id, slot, value.value()) {
                        Ok(metadata) => entries.push(SaveSlotEntry::Intact(metadata)),
                        Err(error) => entries.push(SaveSlotEntry::Corrupt { slot, error }),
                    }
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store_err(error)),
        }

        Ok(entries)
    }

    /// Lists all save slots for `(neighborhood_id, lot_id)`: the manual save
    /// first (if present), followed by rotating autosaves newest-first
    /// (`autosave_seq` descending).
    ///
    /// Corrupt or unsupported-version slots are reported inline as
    /// [`SaveSlotEntry::Corrupt`] without modifying or deleting any record.
    pub fn list_saves(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
    ) -> std::result::Result<Vec<SaveSlotEntry>, SaveError> {
        self.list_saves_validated_with_sha256(neighborhood_id, lot_id, noop_payload_validator)
    }

    /// Lists all save slots using `payload_validator` in addition to built-in
    /// integrity and `WorldSnapshot` checks.
    pub fn list_saves_validated<F>(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        payload_validator: F,
    ) -> std::result::Result<Vec<SaveSlotEntry>, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8]) -> std::result::Result<(), String>,
    {
        self.list_saves_validated_with_sha256(neighborhood_id, lot_id, |kind, _sha256, bytes| {
            payload_validator(kind, bytes)
        })
    }

    /// Like [`Self::list_saves_validated`], passing the already-verified
    /// `payload_sha256` digest alongside each payload slice.
    pub fn list_saves_validated_with_sha256<F>(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        payload_validator: F,
    ) -> std::result::Result<Vec<SaveSlotEntry>, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8; 32], &[u8]) -> std::result::Result<(), String>,
    {
        validate_neighborhood_id(neighborhood_id)?;
        let txn = self.db.begin_read().map_err(store_err)?;
        let mut entries = Vec::new();

        match txn.open_table(AUTOSAVE_MANUAL) {
            Ok(table) => {
                if let Some(value) = table.get((neighborhood_id, lot_id)).map_err(store_err)? {
                    match decode_record_validated_with_sha256(
                        neighborhood_id,
                        lot_id,
                        SaveSlot::Manual,
                        value.value(),
                        &payload_validator,
                    ) {
                        Ok(loaded) => entries.push(SaveSlotEntry::Intact(loaded.metadata)),
                        Err(error) => entries.push(SaveSlotEntry::Corrupt {
                            slot: SaveSlot::Manual,
                            error,
                        }),
                    }
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store_err(error)),
        }

        match txn.open_table(AUTOSAVE_RING) {
            Ok(table) => {
                for item in table
                    .range((neighborhood_id, lot_id, 0u64)..=(neighborhood_id, lot_id, u64::MAX))
                    .map_err(store_err)?
                    .rev()
                {
                    let (key, value) = item.map_err(store_err)?;
                    let seq = key.value().2;
                    let slot = SaveSlot::Autosave(seq);
                    match decode_record_validated_with_sha256(
                        neighborhood_id,
                        lot_id,
                        slot,
                        value.value(),
                        &payload_validator,
                    ) {
                        Ok(loaded) => entries.push(SaveSlotEntry::Intact(loaded.metadata)),
                        Err(error) => entries.push(SaveSlotEntry::Corrupt { slot, error }),
                    }
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store_err(error)),
        }

        Ok(entries)
    }

    /// Lists all `lot_id`s in `neighborhood_id` that have at least one manual
    /// save or autosave slot, in ascending order.
    pub fn list_lots_with_saves(
        &self,
        neighborhood_id: &str,
    ) -> std::result::Result<Vec<u32>, SaveError> {
        validate_neighborhood_id(neighborhood_id)?;
        let txn = self.db.begin_read().map_err(store_err)?;
        let mut lots = BTreeSet::new();

        match txn.open_table(AUTOSAVE_MANUAL) {
            Ok(table) => {
                for item in table
                    .range((neighborhood_id, 0u32)..=(neighborhood_id, u32::MAX))
                    .map_err(store_err)?
                {
                    let (key, _) = item.map_err(store_err)?;
                    lots.insert(key.value().1);
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store_err(error)),
        }

        match txn.open_table(AUTOSAVE_RING) {
            Ok(table) => {
                for item in table
                    .range((neighborhood_id, 0u32, 0u64)..=(neighborhood_id, u32::MAX, u64::MAX))
                    .map_err(store_err)?
                {
                    let (key, _) = item.map_err(store_err)?;
                    lots.insert(key.value().1);
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store_err(error)),
        }

        Ok(lots.into_iter().collect())
    }

    /// Loads one specific save slot with full checksum, schema, and built-in
    /// payload verification. Never modifies or deletes any record on failure.
    pub fn load_save(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        slot: SaveSlot,
    ) -> std::result::Result<LoadedSave, SaveError> {
        self.load_save_validated_with_sha256(neighborhood_id, lot_id, slot, noop_payload_validator)
    }

    /// Loads one specific save slot using `payload_validator` in addition to
    /// built-in checks. Never modifies or deletes any record on failure.
    pub fn load_save_validated<F>(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        slot: SaveSlot,
        payload_validator: F,
    ) -> std::result::Result<LoadedSave, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8]) -> std::result::Result<(), String>,
    {
        self.load_save_validated_with_sha256(
            neighborhood_id,
            lot_id,
            slot,
            |kind, _sha256, bytes| payload_validator(kind, bytes),
        )
    }

    /// Like [`Self::load_save_validated`], passing the already-verified
    /// `payload_sha256` digest alongside the payload slice.
    pub fn load_save_validated_with_sha256<F>(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        slot: SaveSlot,
        payload_validator: F,
    ) -> std::result::Result<LoadedSave, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8; 32], &[u8]) -> std::result::Result<(), String>,
    {
        validate_neighborhood_id(neighborhood_id)?;
        let txn = self.db.begin_read().map_err(store_err)?;
        match slot {
            SaveSlot::Manual => {
                let table = match txn.open_table(AUTOSAVE_MANUAL) {
                    Ok(table) => table,
                    Err(redb::TableError::TableDoesNotExist(_)) => {
                        return Err(SaveError::Missing {
                            neighborhood_id: neighborhood_id.to_owned(),
                            lot_id,
                            slot: Some(slot),
                        });
                    }
                    Err(error) => return Err(store_err(error)),
                };
                let Some(value) = table.get((neighborhood_id, lot_id)).map_err(store_err)? else {
                    return Err(SaveError::Missing {
                        neighborhood_id: neighborhood_id.to_owned(),
                        lot_id,
                        slot: Some(slot),
                    });
                };
                decode_record_validated_with_sha256(
                    neighborhood_id,
                    lot_id,
                    slot,
                    value.value(),
                    &payload_validator,
                )
            }
            SaveSlot::Autosave(seq) => {
                let table = match txn.open_table(AUTOSAVE_RING) {
                    Ok(table) => table,
                    Err(redb::TableError::TableDoesNotExist(_)) => {
                        return Err(SaveError::Missing {
                            neighborhood_id: neighborhood_id.to_owned(),
                            lot_id,
                            slot: Some(slot),
                        });
                    }
                    Err(error) => return Err(store_err(error)),
                };
                let Some(value) = table
                    .get((neighborhood_id, lot_id, seq))
                    .map_err(store_err)?
                else {
                    return Err(SaveError::Missing {
                        neighborhood_id: neighborhood_id.to_owned(),
                        lot_id,
                        slot: Some(slot),
                    });
                };
                decode_record_validated_with_sha256(
                    neighborhood_id,
                    lot_id,
                    slot,
                    value.value(),
                    &payload_validator,
                )
            }
        }
    }

    /// Convenience wrapper for `load_save(neighborhood_id, lot_id, SaveSlot::Manual)`.
    pub fn load_manual(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
    ) -> std::result::Result<LoadedSave, SaveError> {
        self.load_save(neighborhood_id, lot_id, SaveSlot::Manual)
    }

    /// Loads the newest intact autosave for `(neighborhood_id, lot_id)` in a
    /// single consistent REDB read transaction, reporting any newer corrupt or
    /// unsupported-version autosaves that were skipped (without deleting them).
    pub fn load_latest_intact_autosave(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
    ) -> std::result::Result<RecoveryLoad, SaveError> {
        self.load_latest_intact_autosave_validated_with_sha256(
            neighborhood_id,
            lot_id,
            noop_payload_validator,
        )
    }

    /// Streams rotating autosaves newest-first in a single REDB read
    /// transaction and returns the first candidate passing both built-in
    /// integrity checks and `payload_validator`.
    pub fn load_latest_intact_autosave_validated<F>(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        payload_validator: F,
    ) -> std::result::Result<RecoveryLoad, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8]) -> std::result::Result<(), String>,
    {
        self.load_latest_intact_autosave_validated_with_sha256(
            neighborhood_id,
            lot_id,
            |kind, _sha256, bytes| payload_validator(kind, bytes),
        )
    }

    /// Like [`Self::load_latest_intact_autosave_validated`], passing the
    /// already-verified `payload_sha256` digest alongside each payload slice.
    pub fn load_latest_intact_autosave_validated_with_sha256<F>(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        payload_validator: F,
    ) -> std::result::Result<RecoveryLoad, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8; 32], &[u8]) -> std::result::Result<(), String>,
    {
        validate_neighborhood_id(neighborhood_id)?;
        let txn = self.db.begin_read().map_err(store_err)?;
        let table = match txn.open_table(AUTOSAVE_RING) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => {
                return Err(SaveError::Missing {
                    neighborhood_id: neighborhood_id.to_owned(),
                    lot_id,
                    slot: None,
                });
            }
            Err(error) => return Err(store_err(error)),
        };

        let mut skipped_corrupt = Vec::new();
        let mut newest_error: Option<SaveError> = None;

        for item in table
            .range((neighborhood_id, lot_id, 0u64)..=(neighborhood_id, lot_id, u64::MAX))
            .map_err(store_err)?
            .rev()
        {
            let (key, value) = item.map_err(store_err)?;
            let slot = SaveSlot::Autosave(key.value().2);
            match decode_record_validated_with_sha256(
                neighborhood_id,
                lot_id,
                slot,
                value.value(),
                &payload_validator,
            ) {
                Ok(loaded) => {
                    return Ok(RecoveryLoad {
                        loaded,
                        skipped_corrupt,
                    });
                }
                Err(error) => {
                    if newest_error.is_none() {
                        newest_error = Some(error.clone());
                    }
                    skipped_corrupt.push((slot, error));
                }
            }
        }

        match newest_error {
            Some(error) => Err(error),
            None => Err(SaveError::Missing {
                neighborhood_id: neighborhood_id.to_owned(),
                lot_id,
                slot: None,
            }),
        }
    }

    /// Attempts to load `preferred`, falling back in a single consistent REDB
    /// read transaction to the newest intact autosave (and finally to the manual
    /// slot if `preferred` was an autosave) if `preferred` is missing or
    /// corrupt. Never copies the ring up front and never deletes or modifies
    /// any record.
    pub fn load_with_fallback(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        preferred: SaveSlot,
    ) -> std::result::Result<RecoveryLoad, SaveError> {
        self.load_with_fallback_validated_with_sha256(
            neighborhood_id,
            lot_id,
            preferred,
            noop_payload_validator,
        )
    }

    /// Streams `preferred` first by direct key lookup and falls back lazily in
    /// the same REDB read transaction, validating every candidate with both
    /// built-in checks and `payload_validator`.
    pub fn load_with_fallback_validated<F>(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        preferred: SaveSlot,
        payload_validator: F,
    ) -> std::result::Result<RecoveryLoad, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8]) -> std::result::Result<(), String>,
    {
        self.load_with_fallback_validated_with_sha256(
            neighborhood_id,
            lot_id,
            preferred,
            |kind, _sha256, bytes| payload_validator(kind, bytes),
        )
    }

    /// Like [`Self::load_with_fallback_validated`], passing the
    /// already-verified `payload_sha256` digest alongside each payload slice.
    pub fn load_with_fallback_validated_with_sha256<F>(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        preferred: SaveSlot,
        payload_validator: F,
    ) -> std::result::Result<RecoveryLoad, SaveError>
    where
        F: Fn(&SavePayloadKind, &[u8; 32], &[u8]) -> std::result::Result<(), String>,
    {
        validate_neighborhood_id(neighborhood_id)?;
        let txn = self.db.begin_read().map_err(store_err)?;

        let mut skipped_corrupt = Vec::new();
        let mut first_error: Option<SaveError> = None;

        let mut try_candidate = |slot: SaveSlot, bytes: &[u8]| -> Option<LoadedSave> {
            match decode_record_validated_with_sha256(
                neighborhood_id,
                lot_id,
                slot,
                bytes,
                &payload_validator,
            ) {
                Ok(loaded) => Some(loaded),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error.clone());
                    }
                    skipped_corrupt.push((slot, error));
                    None
                }
            }
        };

        match preferred {
            SaveSlot::Manual => {
                // 1. Direct lookup of the preferred manual slot first (zero ring reads).
                match txn.open_table(AUTOSAVE_MANUAL) {
                    Ok(table) => {
                        if let Some(value) =
                            table.get((neighborhood_id, lot_id)).map_err(store_err)?
                            && let Some(loaded) = try_candidate(SaveSlot::Manual, value.value())
                        {
                            return Ok(RecoveryLoad {
                                loaded,
                                skipped_corrupt,
                            });
                        }
                    }
                    Err(redb::TableError::TableDoesNotExist(_)) => {}
                    Err(error) => return Err(store_err(error)),
                }

                // 2. Stream rotating autosaves newest-first directly from REDB.
                match txn.open_table(AUTOSAVE_RING) {
                    Ok(table) => {
                        for item in table
                            .range(
                                (neighborhood_id, lot_id, 0u64)
                                    ..=(neighborhood_id, lot_id, u64::MAX),
                            )
                            .map_err(store_err)?
                            .rev()
                        {
                            let (key, value) = item.map_err(store_err)?;
                            let slot = SaveSlot::Autosave(key.value().2);
                            if let Some(loaded) = try_candidate(slot, value.value()) {
                                return Ok(RecoveryLoad {
                                    loaded,
                                    skipped_corrupt,
                                });
                            }
                        }
                    }
                    Err(redb::TableError::TableDoesNotExist(_)) => {}
                    Err(error) => return Err(store_err(error)),
                }
            }
            SaveSlot::Autosave(target_seq) => {
                // 1. Direct lookup of the preferred autosave slot first, then
                //    stream remaining autosaves newest-first.
                match txn.open_table(AUTOSAVE_RING) {
                    Ok(table) => {
                        if let Some(value) = table
                            .get((neighborhood_id, lot_id, target_seq))
                            .map_err(store_err)?
                            && let Some(loaded) =
                                try_candidate(SaveSlot::Autosave(target_seq), value.value())
                        {
                            return Ok(RecoveryLoad {
                                loaded,
                                skipped_corrupt,
                            });
                        }
                        for item in table
                            .range(
                                (neighborhood_id, lot_id, 0u64)
                                    ..=(neighborhood_id, lot_id, u64::MAX),
                            )
                            .map_err(store_err)?
                            .rev()
                        {
                            let (key, value) = item.map_err(store_err)?;
                            let seq = key.value().2;
                            if seq == target_seq {
                                continue;
                            }
                            if let Some(loaded) =
                                try_candidate(SaveSlot::Autosave(seq), value.value())
                            {
                                return Ok(RecoveryLoad {
                                    loaded,
                                    skipped_corrupt,
                                });
                            }
                        }
                    }
                    Err(redb::TableError::TableDoesNotExist(_)) => {}
                    Err(error) => return Err(store_err(error)),
                }

                // 2. Fall back to the manual slot only if no autosave was intact.
                match txn.open_table(AUTOSAVE_MANUAL) {
                    Ok(table) => {
                        if let Some(value) =
                            table.get((neighborhood_id, lot_id)).map_err(store_err)?
                            && let Some(loaded) = try_candidate(SaveSlot::Manual, value.value())
                        {
                            return Ok(RecoveryLoad {
                                loaded,
                                skipped_corrupt,
                            });
                        }
                    }
                    Err(redb::TableError::TableDoesNotExist(_)) => {}
                    Err(error) => return Err(store_err(error)),
                }
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Err(SaveError::Missing {
                neighborhood_id: neighborhood_id.to_owned(),
                lot_id,
                slot: Some(preferred),
            }),
        }
    }

    /// Explicitly deletes one save slot (e.g. after user confirmation).
    /// Preserves the monotonic autosave sequence counter so sequence numbers
    /// are never reused. Returns `true` if a record existed.
    pub fn delete_slot(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        slot: SaveSlot,
    ) -> std::result::Result<bool, SaveError> {
        validate_neighborhood_id(neighborhood_id)?;
        let txn = self.db.begin_write().map_err(store_err)?;
        let existed = match slot {
            SaveSlot::Manual => {
                let mut table = txn.open_table(AUTOSAVE_MANUAL).map_err(store_err)?;
                table
                    .remove((neighborhood_id, lot_id))
                    .map_err(store_err)?
                    .is_some()
            }
            SaveSlot::Autosave(seq) => {
                let mut table = txn.open_table(AUTOSAVE_RING).map_err(store_err)?;
                table
                    .remove((neighborhood_id, lot_id, seq))
                    .map_err(store_err)?
                    .is_some()
            }
        };
        txn.commit().map_err(store_err)?;
        Ok(existed)
    }
}

/// Removes all manual saves, rotating autosaves, and sequence counters for
/// `neighborhood_id` inside an open REDB write transaction.
///
/// Called by [`crate::NeighborhoodStore::delete_neighborhood`] and
/// [`crate::NeighborhoodStore::import_directory`] so re-importing or deleting
/// a neighborhood never leaves orphaned saves behind.
pub(crate) fn purge_neighborhood_in_txn(
    txn: &redb::WriteTransaction,
    neighborhood_id: &str,
) -> std::result::Result<(), Error> {
    let mut manual_table = txn
        .open_table(AUTOSAVE_MANUAL)
        .map_err(|e| Error::Store(e.to_string()))?;
    let mut manual_lots = Vec::new();
    for item in manual_table
        .range((neighborhood_id, 0u32)..=(neighborhood_id, u32::MAX))
        .map_err(|e| Error::Store(e.to_string()))?
    {
        let (k, _) = item.map_err(|e| Error::Store(e.to_string()))?;
        manual_lots.push(k.value().1);
    }
    for lot_id in manual_lots {
        manual_table
            .remove((neighborhood_id, lot_id))
            .map_err(|e| Error::Store(e.to_string()))?;
    }

    let mut ring_table = txn
        .open_table(AUTOSAVE_RING)
        .map_err(|e| Error::Store(e.to_string()))?;
    let mut ring_keys = Vec::new();
    for item in ring_table
        .range((neighborhood_id, 0u32, 0u64)..=(neighborhood_id, u32::MAX, u64::MAX))
        .map_err(|e| Error::Store(e.to_string()))?
    {
        let (k, _) = item.map_err(|e| Error::Store(e.to_string()))?;
        let (_, lot_id, seq) = k.value();
        ring_keys.push((lot_id, seq));
    }
    for (lot_id, seq) in ring_keys {
        ring_table
            .remove((neighborhood_id, lot_id, seq))
            .map_err(|e| Error::Store(e.to_string()))?;
    }

    let mut seq_table = txn
        .open_table(AUTOSAVE_SEQ)
        .map_err(|e| Error::Store(e.to_string()))?;
    let mut seq_lots = Vec::new();
    for item in seq_table
        .range((neighborhood_id, 0u32)..=(neighborhood_id, u32::MAX))
        .map_err(|e| Error::Store(e.to_string()))?
    {
        let (k, _) = item.map_err(|e| Error::Store(e.to_string()))?;
        seq_lots.push(k.value().1);
    }
    for lot_id in seq_lots {
        seq_table
            .remove((neighborhood_id, lot_id))
            .map_err(|e| Error::Store(e.to_string()))?;
    }

    Ok(())
}
