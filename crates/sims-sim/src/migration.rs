//! Explicit, deterministic forward schema migrations for REDB-persisted records.
//!
//! Each versioned record is stored on disk as a 4-byte little-endian `u32`
//! schema version followed by its `postcard` payload. [`MigrationRegistry`]
//! defines a contiguous forward chain (`v -> v + 1`) from `min_version` to
//! `current_version`, strictly decoding every intermediate payload (rejecting
//! truncated or trailing bytes) and validating the final current-version body.
//!
//! For [`StoredNeighborhood`], the registered migration chain is:
//! - `v1 -> v2`: adds `deleted_characters` (empty) and derives a deterministic
//!   non-zero 128-bit `instance_id` from `(record_name, v1_body)`.
//! - `v2 -> v3`: adds `deletion_epoch` and `deleted_at_epoch`, backfilling
//!   `deletion_epoch = 1` and `deleted_at_epoch[guid] = 1` whenever a `v2`
//!   record already carries non-empty `deleted_characters` so pre-migration
//!   saved lots (`deletion_epoch == 0`) still strip stale instances if a deleted
//!   GUID is later reused, and normalizing any zero `instance_id` to a
//!   deterministic non-zero value.
//! - `v3 -> v4`: adds `family_surnames` (`BTreeMap<u16, String>`, empty on
//!   upgrade) so Create-A-Family and imported `FAMs` surnames persist natively
//!   inside REDB without an external sidecar file.

use crate::characters::Characters;
use crate::neighborhood_store::{NEIGHBORHOOD_FORMAT, StoredNeighborhood};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_formats::neighborhood::{Family, Inventory, Neighbor};
use sims_formats::tatt::Tatt;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Earliest [`StoredNeighborhood`] schema version supported by the forward
/// migration chain.
pub const NEIGHBORHOOD_MIN_FORMAT: u32 = 1;

/// Structured error returned when decoding or migrating a versioned record fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationError {
    /// Record is shorter than the 4-byte little-endian version header.
    TruncatedHeader { record: String, len: usize },
    /// Record version is outside the supported `[min_supported, max_supported]`
    /// range (e.g. `0` or written by a newer build).
    UnsupportedVersion {
        record: String,
        found: u32,
        min_supported: u32,
        max_supported: u32,
    },
    /// Record payload at `version` could not be decoded or had trailing bytes.
    CorruptPayload {
        record: String,
        version: u32,
        detail: String,
    },
    /// A registered migration step failed or the chain is missing a step.
    StepFailed {
        record: String,
        from: u32,
        to: u32,
        detail: String,
    },
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedHeader { record, len } => {
                write!(
                    f,
                    "{record}: shorter than 4-byte version header ({len} bytes)"
                )
            }
            Self::UnsupportedVersion {
                record,
                found,
                min_supported,
                max_supported,
            } => {
                if min_supported == max_supported {
                    write!(
                        f,
                        "{record}: neighborhood format {found}, expected {max_supported}"
                    )
                } else {
                    write!(
                        f,
                        "{record}: unsupported schema format {found} (supported {min_supported}..={max_supported})"
                    )
                }
            }
            Self::CorruptPayload {
                record,
                version,
                detail,
            } => write!(f, "{record}: corrupt format {version} payload: {detail}"),
            Self::StepFailed {
                record,
                from,
                to,
                detail,
            } => write!(f, "{record}: migration v{from} -> v{to} failed: {detail}"),
        }
    }
}

impl std::error::Error for MigrationError {}

/// Error returned by atomic store-wide or single-record REDB migrations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreMigrationError {
    /// A record in the database failed validation or migration; the transaction
    /// was aborted without mutating any records.
    Migration(MigrationError),
    /// Underlying REDB storage or transaction error.
    Store(String),
}

impl fmt::Display for StoreMigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Migration(error) => write!(f, "schema migration aborted: {error}"),
            Self::Store(message) => write!(f, "schema migration store error: {message}"),
        }
    }
}

impl std::error::Error for StoreMigrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Migration(error) => Some(error),
            Self::Store(_) => None,
        }
    }
}

impl From<MigrationError> for StoreMigrationError {
    fn from(error: MigrationError) -> Self {
        Self::Migration(error)
    }
}

/// One deterministic forward schema migration step (`from -> to`, with
/// `to == from + 1`).
#[derive(Debug, Clone, Copy)]
pub struct MigrationStep {
    pub from: u32,
    pub to: u32,
    pub description: &'static str,
    pub apply: fn(&str, &[u8]) -> std::result::Result<Vec<u8>, MigrationError>,
}

/// Output of running [`MigrationRegistry::migrate_bytes`] on a versioned record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigratedBytes {
    pub original_version: u32,
    pub final_version: u32,
    pub steps_applied: u32,
    /// Full version-prefixed bytes at `final_version`.
    pub encoded: Vec<u8>,
}

/// Summary of decoding a single record through the migration chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    pub original_version: u32,
    pub final_version: u32,
    pub steps_applied: u32,
}

impl MigrationReport {
    /// Whether at least one forward migration step ran.
    #[must_use]
    pub const fn was_migrated(&self) -> bool {
        self.steps_applied > 0
    }
}

/// One neighborhood row upgraded during an atomic store migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigratedRecord {
    pub id: String,
    pub from_version: u32,
    pub to_version: u32,
    pub steps_applied: u32,
}

/// Summary returned by [`crate::NeighborhoodStore::migrate_all`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreMigrationReport {
    pub target_version: u32,
    pub inspected: usize,
    pub already_current: usize,
    pub migrated: Vec<MigratedRecord>,
}

/// Read-only inspection status of one stored neighborhood row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeighborhoodSchemaStatus {
    Current {
        id: String,
        version: u32,
    },
    NeedsMigration {
        id: String,
        from_version: u32,
        target_version: u32,
    },
    Invalid {
        id: String,
        error: MigrationError,
    },
}

/// Registry of sequential forward schema migrations from `min_version` to
/// `current_version`.
#[derive(Debug, Clone, Copy)]
pub struct MigrationRegistry {
    pub min_version: u32,
    pub current_version: u32,
    pub steps: &'static [MigrationStep],
    pub validate_current: fn(&str, &[u8]) -> std::result::Result<(), MigrationError>,
}

impl MigrationRegistry {
    /// Parses the 4-byte version header and migrates `bytes` step-by-step to
    /// `self.current_version`, validating the resulting current-version body.
    pub fn migrate_bytes(
        &self,
        record: &str,
        bytes: &[u8],
    ) -> std::result::Result<MigratedBytes, MigrationError> {
        let Some((header, body_slice)) = bytes.split_first_chunk::<4>() else {
            return Err(MigrationError::TruncatedHeader {
                record: record.to_owned(),
                len: bytes.len(),
            });
        };
        let original_version = u32::from_le_bytes(*header);
        if original_version < self.min_version || original_version > self.current_version {
            return Err(MigrationError::UnsupportedVersion {
                record: record.to_owned(),
                found: original_version,
                min_supported: self.min_version,
                max_supported: self.current_version,
            });
        }

        let mut version = original_version;
        let mut body = body_slice.to_vec();
        let mut steps_applied = 0u32;

        while version < self.current_version {
            let expected_next = version + 1;
            let Some(step) = self
                .steps
                .iter()
                .find(|s| s.from == version && s.to == expected_next)
            else {
                return Err(MigrationError::StepFailed {
                    record: record.to_owned(),
                    from: version,
                    to: expected_next,
                    detail: "no registered forward migration step".into(),
                });
            };
            body = (step.apply)(record, &body)?;
            version = step.to;
            steps_applied = steps_applied.saturating_add(1);
        }

        (self.validate_current)(record, &body)?;

        let mut encoded = Vec::with_capacity(4 + body.len());
        encoded.extend_from_slice(&self.current_version.to_le_bytes());
        encoded.extend_from_slice(&body);

        Ok(MigratedBytes {
            original_version,
            final_version: self.current_version,
            steps_applied,
            encoded,
        })
    }

    /// Returns the registered migration steps in order.
    #[must_use]
    pub const fn steps(&self) -> &'static [MigrationStep] {
        self.steps
    }
}

/// Decodes a `postcard` value while strictly rejecting any trailing bytes.
pub fn decode_postcard_exact<'a, T: Deserialize<'a>>(
    record: &str,
    version: u32,
    body: &'a [u8],
) -> std::result::Result<T, MigrationError> {
    let (value, trailing) =
        postcard::take_from_bytes::<T>(body).map_err(|error| MigrationError::CorruptPayload {
            record: record.to_owned(),
            version,
            detail: error.to_string(),
        })?;
    if !trailing.is_empty() {
        return Err(MigrationError::CorruptPayload {
            record: record.to_owned(),
            version,
            detail: format!("{} unexpected trailing byte(s)", trailing.len()),
        });
    }
    Ok(value)
}

/// Historical schema v1 for [`StoredNeighborhood`] (before `deleted_characters`,
/// `instance_id`, `deletion_epoch`, and `deleted_at_epoch`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNeighborhoodV1 {
    pub name: String,
    pub families: BTreeMap<u16, Family>,
    pub neighbors: Vec<Option<Neighbor>>,
    pub inventories: Vec<Option<Inventory>>,
    pub data: [i16; 16],
    pub options: Option<[bool; 11]>,
    pub type_attributes: Option<Tatt>,
    pub zoning: BTreeMap<i16, i32>,
    pub lot_rows: Vec<i32>,
    pub made_characters: Characters,
    pub revision: u64,
}

impl StoredNeighborhoodV1 {
    /// Encodes a version-1 neighborhood record (`1u32.to_le_bytes()` + postcard).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = 1u32.to_le_bytes().to_vec();
        out.extend(postcard::to_allocvec(self).expect("encoding to memory cannot fail"));
        out
    }
}

/// Historical schema v2 for [`StoredNeighborhood`] (added `deleted_characters`
/// and `instance_id`, before `deletion_epoch` and `deleted_at_epoch`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNeighborhoodV2 {
    pub name: String,
    pub families: BTreeMap<u16, Family>,
    pub neighbors: Vec<Option<Neighbor>>,
    pub inventories: Vec<Option<Inventory>>,
    pub data: [i16; 16],
    pub options: Option<[bool; 11]>,
    pub type_attributes: Option<Tatt>,
    pub zoning: BTreeMap<i16, i32>,
    pub lot_rows: Vec<i32>,
    pub made_characters: Characters,
    pub deleted_characters: BTreeSet<u32>,
    pub instance_id: u128,
    pub revision: u64,
}

impl StoredNeighborhoodV2 {
    /// Encodes a version-2 neighborhood record (`2u32.to_le_bytes()` + postcard).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = 2u32.to_le_bytes().to_vec();
        out.extend(postcard::to_allocvec(self).expect("encoding to memory cannot fail"));
        out
    }
}

/// Historical schema v3 for [`StoredNeighborhood`] (added `deletion_epoch` and
/// `deleted_at_epoch`, before `family_surnames`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNeighborhoodV3 {
    pub name: String,
    pub families: BTreeMap<u16, Family>,
    pub neighbors: Vec<Option<Neighbor>>,
    pub inventories: Vec<Option<Inventory>>,
    pub data: [i16; 16],
    pub options: Option<[bool; 11]>,
    pub type_attributes: Option<Tatt>,
    pub zoning: BTreeMap<i16, i32>,
    pub lot_rows: Vec<i32>,
    pub made_characters: Characters,
    pub deleted_characters: BTreeSet<u32>,
    pub deletion_epoch: u64,
    pub deleted_at_epoch: BTreeMap<u32, u64>,
    pub instance_id: u128,
    pub revision: u64,
}

impl StoredNeighborhoodV3 {
    /// Encodes a version-3 neighborhood record (`3u32.to_le_bytes()` + postcard).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = 3u32.to_le_bytes().to_vec();
        out.extend(postcard::to_allocvec(self).expect("encoding to memory cannot fail"));
        out
    }
}

/// Deterministically derives a non-zero 128-bit `instance_id` for a legacy
/// record that predated `instance_id` (or recorded `0`).
///
/// Using a pure SHA-256 hash of `(record_name, body)` ensures that read-time
/// migration (`StoredNeighborhood::decode`) and atomic in-DB migration
/// (`NeighborhoodStore::migrate_all`) always produce the exact same
/// `instance_id` across calls, threads, processes, and platforms.
#[must_use]
pub fn derive_legacy_instance_id(record: &str, body: &[u8]) -> u128 {
    let mut hasher = Sha256::new();
    hasher.update(b"sims-sim:neighborhood-legacy-instance-id:v1");
    hasher.update((record.len() as u64).to_le_bytes());
    hasher.update(record.as_bytes());
    hasher.update((body.len() as u64).to_le_bytes());
    hasher.update(body);
    let digest = hasher.finalize();
    let bytes: [u8; 16] = digest[..16].try_into().expect("sha256 is 32 bytes");
    let id = u128::from_le_bytes(bytes);
    if id == 0 { 1 } else { id }
}

fn migrate_v1_to_v2(record: &str, body: &[u8]) -> std::result::Result<Vec<u8>, MigrationError> {
    let v2 = match decode_postcard_exact::<StoredNeighborhoodV1>(record, 1, body) {
        Ok(v1) => StoredNeighborhoodV2 {
            name: v1.name,
            families: v1.families,
            neighbors: v1.neighbors,
            inventories: v1.inventories,
            data: v1.data,
            options: v1.options,
            type_attributes: v1.type_attributes,
            zoning: v1.zoning,
            lot_rows: v1.lot_rows,
            made_characters: v1.made_characters,
            deleted_characters: BTreeSet::new(),
            instance_id: derive_legacy_instance_id(record, body),
            revision: v1.revision,
        },
        Err(v1_err) => {
            // Historical commit `0c25ad21` wrote header `1` with the extended
            // v2 field layout (`deleted_characters` + `instance_id`). Under
            // exact postcard decoding the two layouts are strictly disjoint, so
            // accept it losslessly and preserve any fields already present.
            let mut legacy = decode_postcard_exact::<StoredNeighborhoodV2>(record, 1, body)
                .map_err(|_| v1_err)?;
            if legacy.instance_id == 0 {
                legacy.instance_id = derive_legacy_instance_id(record, body);
            }
            legacy
        }
    };
    postcard::to_allocvec(&v2).map_err(|error| MigrationError::StepFailed {
        record: record.to_owned(),
        from: 1,
        to: 2,
        detail: error.to_string(),
    })
}

fn migrate_v2_to_v3(record: &str, body: &[u8]) -> std::result::Result<Vec<u8>, MigrationError> {
    let v3 = match decode_postcard_exact::<StoredNeighborhoodV2>(record, 2, body) {
        Ok(v2) => {
            let (deletion_epoch, deleted_at_epoch) = if v2.deleted_characters.is_empty() {
                (0u64, BTreeMap::new())
            } else {
                let map = v2
                    .deleted_characters
                    .iter()
                    .map(|&guid| (guid, 1u64))
                    .collect();
                (1u64, map)
            };
            let instance_id = if v2.instance_id == 0 {
                derive_legacy_instance_id(record, body)
            } else {
                v2.instance_id
            };
            StoredNeighborhoodV3 {
                name: v2.name,
                families: v2.families,
                neighbors: v2.neighbors,
                inventories: v2.inventories,
                data: v2.data,
                options: v2.options,
                type_attributes: v2.type_attributes,
                zoning: v2.zoning,
                lot_rows: v2.lot_rows,
                made_characters: v2.made_characters,
                deleted_characters: v2.deleted_characters,
                deletion_epoch,
                deleted_at_epoch,
                instance_id,
                revision: v2.revision,
            }
        }
        Err(v2_err) => {
            // Historical commit `1719402e` wrote header `2` with the full v3
            // field layout (`deletion_epoch` + `deleted_at_epoch`). Under exact
            // postcard decoding the two layouts are strictly disjoint, so
            // preserve existing epoch fields verbatim (backfilling epoch 1 only
            // if deleted_characters was non-empty with zeroed epoch metadata).
            let mut legacy = decode_postcard_exact::<StoredNeighborhoodV3>(record, 2, body)
                .map_err(|_| v2_err)?;
            if !legacy.deleted_characters.is_empty()
                && legacy.deletion_epoch == 0
                && legacy.deleted_at_epoch.is_empty()
            {
                legacy.deletion_epoch = 1;
                legacy.deleted_at_epoch = legacy
                    .deleted_characters
                    .iter()
                    .map(|&guid| (guid, 1u64))
                    .collect();
            }
            if legacy.instance_id == 0 {
                legacy.instance_id = derive_legacy_instance_id(record, body);
            }
            legacy
        }
    };
    postcard::to_allocvec(&v3).map_err(|error| MigrationError::StepFailed {
        record: record.to_owned(),
        from: 2,
        to: 3,
        detail: error.to_string(),
    })
}

fn migrate_v3_to_v4(record: &str, body: &[u8]) -> std::result::Result<Vec<u8>, MigrationError> {
    let v3: StoredNeighborhoodV3 = decode_postcard_exact(record, 3, body)?;
    let v4 = StoredNeighborhood {
        name: v3.name,
        families: v3.families,
        family_surnames: BTreeMap::new(),
        neighbors: v3.neighbors,
        inventories: v3.inventories,
        data: v3.data,
        options: v3.options,
        type_attributes: v3.type_attributes,
        zoning: v3.zoning,
        lot_rows: v3.lot_rows,
        made_characters: v3.made_characters,
        deleted_characters: v3.deleted_characters,
        deletion_epoch: v3.deletion_epoch,
        deleted_at_epoch: v3.deleted_at_epoch,
        instance_id: v3.instance_id,
        revision: v3.revision,
    };
    postcard::to_allocvec(&v4).map_err(|error| MigrationError::StepFailed {
        record: record.to_owned(),
        from: 3,
        to: 4,
        detail: error.to_string(),
    })
}

fn validate_neighborhood_v4(record: &str, body: &[u8]) -> std::result::Result<(), MigrationError> {
    let _: StoredNeighborhood = decode_postcard_exact(record, NEIGHBORHOOD_FORMAT, body)?;
    Ok(())
}

static NEIGHBORHOOD_STEPS: &[MigrationStep] = &[
    MigrationStep {
        from: 1,
        to: 2,
        description: "add deleted_characters set and deterministic non-zero instance_id",
        apply: migrate_v1_to_v2,
    },
    MigrationStep {
        from: 2,
        to: 3,
        description: "add deletion_epoch and deleted_at_epoch backfill for deleted_characters",
        apply: migrate_v2_to_v3,
    },
    MigrationStep {
        from: 3,
        to: 4,
        description: "add native family_surnames map for CAF and FAMs persistence",
        apply: migrate_v3_to_v4,
    },
];

/// Registered forward migration chain for [`StoredNeighborhood`] (`v1 -> v2 -> v3 -> v4`).
pub const NEIGHBORHOOD_MIGRATIONS: MigrationRegistry = MigrationRegistry {
    min_version: NEIGHBORHOOD_MIN_FORMAT,
    current_version: NEIGHBORHOOD_FORMAT,
    steps: NEIGHBORHOOD_STEPS,
    validate_current: validate_neighborhood_v4,
};

/// Decodes a [`StoredNeighborhood`] record from any supported version
/// (`v1..=v4`), applying forward migrations in memory and returning both the
/// decoded neighborhood and its [`MigrationReport`].
pub fn decode_neighborhood_with_report(
    name: &str,
    bytes: &[u8],
) -> std::result::Result<(StoredNeighborhood, MigrationReport), MigrationError> {
    let migrated = NEIGHBORHOOD_MIGRATIONS.migrate_bytes(name, bytes)?;
    let (_, body) = migrated
        .encoded
        .split_first_chunk::<4>()
        .expect("migrated bytes always carry 4-byte header");
    let neighborhood: StoredNeighborhood =
        decode_postcard_exact(name, migrated.final_version, body)?;
    Ok((
        neighborhood,
        MigrationReport {
            original_version: migrated.original_version,
            final_version: migrated.final_version,
            steps_applied: migrated.steps_applied,
        },
    ))
}

/// Decodes a [`StoredNeighborhood`] record strictly at [`NEIGHBORHOOD_FORMAT`]
/// without applying older-version migrations.
pub fn decode_neighborhood_strict(
    name: &str,
    bytes: &[u8],
) -> std::result::Result<StoredNeighborhood, MigrationError> {
    let Some((header, body)) = bytes.split_first_chunk::<4>() else {
        return Err(MigrationError::TruncatedHeader {
            record: name.to_owned(),
            len: bytes.len(),
        });
    };
    let version = u32::from_le_bytes(*header);
    if version != NEIGHBORHOOD_FORMAT {
        return Err(MigrationError::UnsupportedVersion {
            record: name.to_owned(),
            found: version,
            min_supported: NEIGHBORHOOD_FORMAT,
            max_supported: NEIGHBORHOOD_FORMAT,
        });
    }
    decode_postcard_exact(name, version, body)
}
