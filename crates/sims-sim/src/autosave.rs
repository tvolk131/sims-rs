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
use crate::{Error, World};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

/// 4-byte magic prefix identifying an autosave/manual-save record envelope.
const AUTOSAVE_MAGIC: [u8; 4] = *b"SAVG";

/// Domain separator for the outer record SHA-256 checksum.
const RECORD_HASH_DOMAIN: &[u8] = b"sims-sim:autosave-record:v1";

/// Length of the fixed envelope prefix: 4-byte magic + 4-byte version +
/// 32-byte record SHA-256.
const RECORD_HEADER_LEN: usize = 4 + 4 + 32;

/// Earliest supported autosave record format version.
pub const AUTOSAVE_MIN_FORMAT: u32 = 1;

/// Current autosave record format version.
pub const AUTOSAVE_FORMAT: u32 = 1;

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
    /// Encoded [`World`] snapshot (`World::encode`).
    WorldSnapshot,
    /// Encoded `sims_vm::boot::save::Saved` lot bytes (`Saved::encode`).
    SavedLot,
    /// Caller-defined payload format tag.
    Custom(String),
}

/// Owned payload handed to a background save worker via [`SaveJob`].
#[derive(Debug, Clone)]
pub enum SavePayload {
    /// Cloned CoW [`World`] snapshot; encoded (`World::encode`) on the worker
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
            Self::World(world) => (SavePayloadKind::WorldSnapshot, world.encode()),
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
    /// Creates an autosave job for `(neighborhood_id, lot_id)`.
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
            target: SaveTarget::Autosave,
            sim_tick,
            saved_at_unix_secs: None,
            label: String::new(),
            payload,
        }
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
            target: SaveTarget::Manual,
            sim_tick,
            saved_at_unix_secs: None,
            label: String::new(),
            payload,
        }
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
            target: SaveTarget::ManualForceReplace,
            sim_tick,
            saved_at_unix_secs: None,
            label: String::new(),
            payload,
        }
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
        World::decode(&self.payload).map_err(|error| SaveError::DecodeFailed {
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

fn encode_record(body: &AutosaveBodyV1) -> std::result::Result<Vec<u8>, SaveError> {
    let body_bytes = postcard::to_allocvec(body).map_err(|error| SaveError::DecodeFailed {
        neighborhood_id: body.neighborhood_id.clone(),
        lot_id: body.lot_id,
        slot: body.slot,
        detail: format!("failed to encode save record: {error}"),
    })?;
    let checksum = record_checksum(AUTOSAVE_FORMAT, &body_bytes);
    let mut out = Vec::with_capacity(RECORD_HEADER_LEN + body_bytes.len());
    out.extend_from_slice(&AUTOSAVE_MAGIC);
    out.extend_from_slice(&AUTOSAVE_FORMAT.to_le_bytes());
    out.extend_from_slice(&checksum);
    out.extend_from_slice(&body_bytes);
    Ok(out)
}

fn decode_record(
    neighborhood_id: &str,
    lot_id: u32,
    slot: SaveSlot,
    bytes: &[u8],
) -> std::result::Result<LoadedSave, SaveError> {
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

    let record_name = format!("{neighborhood_id}:lot{lot_id}:{slot}");
    let body: AutosaveBodyV1 =
        decode_postcard_exact(&record_name, version, body_bytes).map_err(|error| {
            SaveError::DecodeFailed {
                neighborhood_id: neighborhood_id.to_owned(),
                lot_id,
                slot,
                detail: error.to_string(),
            }
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

    if body.neighborhood_id != neighborhood_id || body.lot_id != lot_id || body.slot != slot {
        return Err(SaveError::DecodeFailed {
            neighborhood_id: neighborhood_id.to_owned(),
            lot_id,
            slot,
            detail: format!(
                "record key binding mismatch: stored ({:?}, lot {}, {}), expected ({:?}, lot {}, {})",
                body.neighborhood_id, body.lot_id, body.slot, neighborhood_id, lot_id, slot
            ),
        });
    }

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
}

impl fmt::Debug for AutosaveStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AutosaveStore")
            .field("policy", &self.policy)
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
        })
    }

    /// Constructs an [`AutosaveStore`] sharing an existing `Arc<Database>`
    /// (used by [`crate::NeighborhoodStore::autosaves`]).
    pub(crate) fn from_shared_db(db: Arc<Database>, policy: AutosavePolicy) -> Self {
        let max_autosaves = policy.max_autosaves.max(1);
        Self {
            db,
            policy: AutosavePolicy { max_autosaves },
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
        })
    }

    /// Executes an owned [`SaveJob`] atomically.
    ///
    /// Payload encoding (`World::encode`) and payload SHA-256 hashing happen
    /// *before* acquiring the REDB write transaction so background workers hold
    /// the write lock for minimal duration.
    pub fn commit_job(&self, job: SaveJob) -> std::result::Result<SaveMetadata, SaveError> {
        validate_neighborhood_id(&job.neighborhood_id)?;
        if self.policy.max_autosaves == 0 {
            return Err(SaveError::InvalidInput(
                "max_autosaves must be at least 1".into(),
            ));
        }

        let (payload_kind, payload_bytes) = job.payload.into_encoded();
        let payload_sha256 = sha256_bytes(&payload_bytes);
        let payload_len = payload_bytes.len();

        match job.target {
            SaveTarget::Manual => self.commit_manual_encoded(
                job.neighborhood_id,
                job.lot_id,
                job.sim_tick,
                job.saved_at_unix_secs,
                job.label,
                payload_kind,
                payload_sha256,
                payload_len,
                payload_bytes,
                false,
            ),
            SaveTarget::ManualForceReplace => self.commit_manual_encoded(
                job.neighborhood_id,
                job.lot_id,
                job.sim_tick,
                job.saved_at_unix_secs,
                job.label,
                payload_kind,
                payload_sha256,
                payload_len,
                payload_bytes,
                true,
            ),
            SaveTarget::Autosave => self.commit_autosave_encoded(
                job.neighborhood_id,
                job.lot_id,
                job.sim_tick,
                job.saved_at_unix_secs,
                job.label,
                payload_kind,
                payload_sha256,
                payload_len,
                payload_bytes,
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
    fn commit_manual_encoded(
        &self,
        neighborhood_id: String,
        lot_id: u32,
        sim_tick: u64,
        saved_at_unix_secs: Option<u64>,
        label: String,
        payload_kind: SavePayloadKind,
        payload_sha256: [u8; 32],
        payload_len: usize,
        payload: Vec<u8>,
        force_replace: bool,
    ) -> std::result::Result<SaveMetadata, SaveError> {
        let body = AutosaveBodyV1 {
            slot: SaveSlot::Manual,
            neighborhood_id: neighborhood_id.clone(),
            lot_id,
            sim_tick,
            saved_at_unix_secs,
            label: label.clone(),
            payload_kind: payload_kind.clone(),
            payload_sha256,
            payload,
        };
        let encoded = encode_record(&body)?;

        let txn = self.db.begin_write().map_err(store_err)?;
        {
            let mut table = txn.open_table(AUTOSAVE_MANUAL).map_err(store_err)?;
            if !force_replace
                && let Some(existing) = table
                    .get((neighborhood_id.as_str(), lot_id))
                    .map_err(store_err)?
            {
                // Refuse to silently replace a corrupt or future-version manual save.
                let _ =
                    decode_record(&neighborhood_id, lot_id, SaveSlot::Manual, existing.value())?;
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
    fn commit_autosave_encoded(
        &self,
        neighborhood_id: String,
        lot_id: u32,
        sim_tick: u64,
        saved_at_unix_secs: Option<u64>,
        label: String,
        payload_kind: SavePayloadKind,
        payload_sha256: [u8; 32],
        payload_len: usize,
        payload: Vec<u8>,
    ) -> std::result::Result<SaveMetadata, SaveError> {
        let txn = self.db.begin_write().map_err(store_err)?;
        let assigned_seq;
        {
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
                if decode_record(
                    &neighborhood_id,
                    lot_id,
                    SaveSlot::Autosave(seq),
                    value.value(),
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

            let body = AutosaveBodyV1 {
                slot: SaveSlot::Autosave(next_seq),
                neighborhood_id: neighborhood_id.clone(),
                lot_id,
                sim_tick,
                saved_at_unix_secs,
                label: label.clone(),
                payload_kind: payload_kind.clone(),
                payload_sha256,
                payload,
            };
            let encoded = encode_record(&body)?;

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
        validate_neighborhood_id(neighborhood_id)?;
        let txn = self.db.begin_read().map_err(store_err)?;
        let mut entries = Vec::new();

        match txn.open_table(AUTOSAVE_MANUAL) {
            Ok(table) => {
                if let Some(value) = table.get((neighborhood_id, lot_id)).map_err(store_err)? {
                    match decode_record(neighborhood_id, lot_id, SaveSlot::Manual, value.value()) {
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
                let mut ring_entries = Vec::new();
                for item in table
                    .range((neighborhood_id, lot_id, 0u64)..=(neighborhood_id, lot_id, u64::MAX))
                    .map_err(store_err)?
                {
                    let (key, value) = item.map_err(store_err)?;
                    let seq = key.value().2;
                    let slot = SaveSlot::Autosave(seq);
                    match decode_record(neighborhood_id, lot_id, slot, value.value()) {
                        Ok(loaded) => ring_entries.push(SaveSlotEntry::Intact(loaded.metadata)),
                        Err(error) => ring_entries.push(SaveSlotEntry::Corrupt { slot, error }),
                    }
                }
                ring_entries.reverse();
                entries.extend(ring_entries);
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

    /// Loads one specific save slot with full checksum and schema verification.
    /// Never modifies or deletes any record on failure.
    pub fn load_save(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        slot: SaveSlot,
    ) -> std::result::Result<LoadedSave, SaveError> {
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
                decode_record(neighborhood_id, lot_id, slot, value.value())
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
                decode_record(neighborhood_id, lot_id, slot, value.value())
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
            match decode_record(neighborhood_id, lot_id, slot, value.value()) {
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
    /// corrupt. Never deletes or modifies any record.
    pub fn load_with_fallback(
        &self,
        neighborhood_id: &str,
        lot_id: u32,
        preferred: SaveSlot,
    ) -> std::result::Result<RecoveryLoad, SaveError> {
        validate_neighborhood_id(neighborhood_id)?;
        let txn = self.db.begin_read().map_err(store_err)?;

        let manual_bytes = match txn.open_table(AUTOSAVE_MANUAL) {
            Ok(table) => table
                .get((neighborhood_id, lot_id))
                .map_err(store_err)?
                .map(|v| v.value().to_vec()),
            Err(redb::TableError::TableDoesNotExist(_)) => None,
            Err(error) => return Err(store_err(error)),
        };

        let mut ring_newest_first: Vec<(u64, Vec<u8>)> = Vec::new();
        match txn.open_table(AUTOSAVE_RING) {
            Ok(table) => {
                for item in table
                    .range((neighborhood_id, lot_id, 0u64)..=(neighborhood_id, lot_id, u64::MAX))
                    .map_err(store_err)?
                    .rev()
                {
                    let (key, value) = item.map_err(store_err)?;
                    ring_newest_first.push((key.value().2, value.value().to_vec()));
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store_err(error)),
        }

        let mut skipped_corrupt = Vec::new();
        let mut first_error: Option<SaveError> = None;

        let mut try_candidate = |slot: SaveSlot, bytes: &[u8]| -> Option<LoadedSave> {
            match decode_record(neighborhood_id, lot_id, slot, bytes) {
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
                if let Some(ref bytes) = manual_bytes
                    && let Some(loaded) = try_candidate(SaveSlot::Manual, bytes)
                {
                    return Ok(RecoveryLoad {
                        loaded,
                        skipped_corrupt,
                    });
                }
                for (seq, bytes) in &ring_newest_first {
                    if let Some(loaded) = try_candidate(SaveSlot::Autosave(*seq), bytes) {
                        return Ok(RecoveryLoad {
                            loaded,
                            skipped_corrupt,
                        });
                    }
                }
            }
            SaveSlot::Autosave(target_seq) => {
                if let Some((_, bytes)) = ring_newest_first.iter().find(|(s, _)| *s == target_seq)
                    && let Some(loaded) = try_candidate(SaveSlot::Autosave(target_seq), bytes)
                {
                    return Ok(RecoveryLoad {
                        loaded,
                        skipped_corrupt,
                    });
                }
                for (seq, bytes) in &ring_newest_first {
                    if *seq == target_seq {
                        continue;
                    }
                    if let Some(loaded) = try_candidate(SaveSlot::Autosave(*seq), bytes) {
                        return Ok(RecoveryLoad {
                            loaded,
                            skipped_corrupt,
                        });
                    }
                }
                if let Some(ref bytes) = manual_bytes
                    && let Some(loaded) = try_candidate(SaveSlot::Manual, bytes)
                {
                    return Ok(RecoveryLoad {
                        loaded,
                        skipped_corrupt,
                    });
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
