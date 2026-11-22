//! Multi-neighborhood and multi-lot REDB persistence store.
//!
//! Imports shipped neighborhood directories (`UserData`, `UserData2`..`UserData8`)
//! into a single REDB database without modifying any install files, and persists
//! neighborhood-wide records (`NGBH`, `FAMI`, `NBRS`, `TATT`, `LotZoning`,
//! `Characters/*.iff`, runtime-made characters) together with per-lot base
//! `HouseXX.iff` blobs and saved lot snapshots atomically.
//!
//! All valuation, net-worth, and writeback formulas match `Sims.exe` 1.91:
//! - Lot total value (`0x4253ff`, `0x4272c5`, `0x427f78`, `0x4282bc`):
//!   `lot_value + objects_value + trunc(architecture_value as f32 * mult)`,
//!   where `mult` is `Global.far` `FCNS #4` `"architecture value multiplier"`
//!   (`0.7f`, `0x3f333333`).
//! - Family net worth (`0x4272c5` / `0x410271`):
//!   `family.budget + family.architecture_value` (when not currently inside
//!   that family's lot).
//! - Neighborhood save writeback (`0x428687` → `0x4252cc`):
//!   flushes `FAMI` (`budget`, `architecture_value`, `house`, `friends`,
//!   `flags`, `members`, `magic_history`), `NBRS` (slots, person data,
//!   relationships), `NGBH` (16 data words, inventories, options), and `TATT`.

use crate::characters::Characters;
use crate::{Error, Result, World};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_content::Neighborhood;
use sims_core::{Ext, F32};
use sims_formats::iff::Iff;
use sims_formats::lot::Simulator;
use sims_formats::neighborhood::{
    Family, Inventory, InventoryItem, Neighbor, Relationship as SavedRelationship,
};
use sims_formats::tatt::{Entry as TattEntry, Tatt};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Key inside [`NEIGHBORHOOD_META`] holding this REDB store's persistent
/// 128-bit identity (little-endian bytes).
const STORE_ID_KEY: &str = "store_id";

static NEXT_UNIQUE_ID: AtomicU64 = AtomicU64::new(1);

/// Generates a fresh 128-bit identity unique across re-imports and separate
/// store files without touching the wall clock.
fn fresh_unique_id(tag: &str, previous: u128, seed: &[u8]) -> u128 {
    let seq = NEXT_UNIQUE_ID.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(tag.as_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(previous.to_le_bytes());
    hasher.update(seq.to_le_bytes());
    hasher.update(seed);
    let digest = hasher.finalize();
    let bytes: [u8; 16] = digest[..16].try_into().expect("sha256 is 32 bytes");
    let mut id = u128::from_le_bytes(bytes);
    if id == 0 || id == previous {
        id = previous.wrapping_add(1);
        if id == 0 {
            id = 1;
        }
    }
    id
}

/// Reason a checked neighborhood write was rejected inside its REDB write
/// transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckedCommitError {
    /// The neighborhood ID was re-imported or replaced by a different instance.
    InstanceMismatch { expected: u128, current: u128 },
    /// Another operation committed a newer revision to the same instance.
    RevisionMismatch { expected: u64, current: u64 },
}

/// Versioned `StoredNeighborhood` metadata keyed by neighborhood ID.
const NEIGHBORHOODS: TableDefinition<&str, &[u8]> = TableDefinition::new("neighborhoods");

/// Raw immutable `Characters/*.iff` bytes keyed by `(neighborhood_id, filename)`.
const NEIGHBORHOOD_CHARACTERS: TableDefinition<(&str, &str), &[u8]> =
    TableDefinition::new("neighborhood_characters");

/// Raw base `Houses/HouseXX.iff` bytes keyed by `(neighborhood_id, house_number)`.
const NEIGHBORHOOD_HOUSES: TableDefinition<(&str, u32), &[u8]> =
    TableDefinition::new("neighborhood_houses");

/// Latest saved lot bytes (`sims_vm::boot::save::Saved::encode`) keyed by
/// `(neighborhood_id, house_number)`.
const NEIGHBORHOOD_LOTS: TableDefinition<(&str, u32), &[u8]> =
    TableDefinition::new("neighborhood_lots");

/// Store-level metadata (e.g. active neighborhood ID).
const NEIGHBORHOOD_META: TableDefinition<&str, &[u8]> = TableDefinition::new("neighborhood_meta");

/// Key inside [`NEIGHBORHOOD_META`] holding the active neighborhood ID as UTF-8.
const ACTIVE_KEY: &str = "active";

/// Bumped whenever [`StoredNeighborhood`]'s postcard layout changes so an older
/// record is refused cleanly instead of misread.
pub const NEIGHBORHOOD_FORMAT: u32 = 2;

/// Default `"architecture value multiplier"` from `Global.far` `FCNS #4`
/// (`0x6e4a88`, `0x3f333333`).
pub const DEFAULT_ARCH_VALUE_MULTIPLIER: f32 = 0.7_f32;

/// Default `"new family start hour"` from `Global.far` `FCNS #4` (`0x6e4a84`).
pub const DEFAULT_NEW_FAMILY_START_HOUR: i16 = 7;

/// Default `"fill value for layout history"` from `Global.far` `FCNS #4`
/// (`0x6e4a78`, used by `0x4165f3` on move-in / evict / bulldoze).
pub const DEFAULT_LAYOUT_HISTORY_FILL: i32 = 200;

/// Family record flag bit 0 (`0x1`): the lot has an object of category 1
/// (`0x4160e5`).
pub const FAMILY_FLAG_HAS_CATEGORY_1: u32 = 1 << 0;

/// Family record flag bit 1 (`0x2`): the lot has an object of category 9
/// (`0x4160f9`).
pub const FAMILY_FLAG_HAS_CATEGORY_9: u32 = 1 << 1;

/// Family record flag bit 2 (`0x4`): newly moved-in household; first entry
/// resets the clock, budget history, usage history, unpauses, and broadcasts
/// `"reset lot"` (`0x427373`, `0x428487..0x4284f9`).
pub const FAMILY_FLAG_NEW_MOVE_IN: u32 = 1 << 2;

/// Family record flag bit 3 (`0x8`): cleared on save, move-in, and eviction
/// (`0x427370`, `0x42743e`, `0x428762`).
pub const FAMILY_FLAG_BIT_3: u32 = 1 << 3;

/// Computes a lot's total value as `Sims.exe` 1.91 computes it (`0x4253ff`,
/// `0x4272c5`, `0x427f78`, `0x4282bc`):
/// `fild architecture_value; fmul dword ptr [0x6e4a88]; call __ftol`.
///
/// Uses exact 64-bit-significand x87 extended multiplication (`31 + 24 = 55 <=
/// 64` bits, zero intermediate rounding) followed by a single round-to-nearest-
/// even to 24-bit `f32` and truncation toward zero.
#[must_use]
pub fn lot_total_value(
    lot_value: i32,
    objects_value: i32,
    architecture_value: i32,
    arch_multiplier: f32,
) -> i32 {
    let scaled_arch = (Ext::from_i32(architecture_value)
        * F32::from_bits(arch_multiplier.to_bits()).ext())
    .single()
    .to_i32_truncate();
    lot_value
        .wrapping_add(objects_value)
        .wrapping_add(scaled_arch)
}

/// Computes a family's net worth outside of live lot play (`0x4272c5` /
/// `0x410271`): `family.budget + family.architecture_value`.
#[must_use]
pub fn family_net_worth(family: &Family) -> i32 {
    family.budget.wrapping_add(family.architecture_value)
}

/// Consistent point-in-time snapshot of a neighborhood and one house read in a
/// single REDB read transaction, preventing lot entry from mixing two
/// committed revisions.
#[derive(Debug, Clone)]
pub struct LotSnapshot {
    pub neighborhood: StoredNeighborhood,
    pub character_files: BTreeMap<String, Vec<u8>>,
    pub saved_lot: Option<Vec<u8>>,
    pub base_house: Option<Vec<u8>>,
}

/// Summary returned by [`NeighborhoodStore::import_directory`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    pub id: String,
    pub families: usize,
    pub neighbors: usize,
    pub characters: usize,
    pub houses: usize,
}

/// Neighborhood-wide persistent state stored in REDB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNeighborhood {
    pub name: String,
    /// Families by FAMI chunk ID.
    pub families: BTreeMap<u16, Family>,
    /// Neighbor records in native slot order (including empty `None` slots).
    pub neighbors: Vec<Option<Neighbor>>,
    /// Neighbor inventories from `NGBH`.
    pub inventories: Vec<Option<Inventory>>,
    /// Neighborhood's sixteen `NGBH` data words.
    pub data: [i16; 16],
    /// Neighborhood's eleven `NGBH` option booleans.
    pub options: Option<[bool; 11]>,
    /// Saved type attributes (`TATT #1`), if any.
    pub type_attributes: Option<Tatt>,
    /// Lot zoning by house number: `1` residential, `2` community.
    pub zoning: BTreeMap<i16, i32>,
    /// Lot numbers in `LotZoning.iff` row order.
    pub lot_rows: Vec<i32>,
    /// Runtime-created characters (`Make New Character`, primitive 19) persisted
    /// across lot switches.
    pub made_characters: Characters,
    /// Character GUIDs permanently deleted via Bulldoze (`0x426fcd`) so even
    /// characters residing inside read-only `.far` archives or multi-OBJD files
    /// are never re-registered into `NBRS` on subsequent lot loads.
    #[serde(default)]
    pub deleted_characters: BTreeSet<u32>,
    /// Unique 128-bit identity assigned on each import/creation so re-importing
    /// a different neighborhood under the same ID invalidates previously opened
    /// lots even when both are at revision 0.
    #[serde(default)]
    pub instance_id: u128,
    /// Monotonic revision bumped on each save writeback.
    pub revision: u64,
}

impl StoredNeighborhood {
    /// Builds a `sims_content::Neighborhood` view of this stored neighborhood
    /// so existing loaders (`World::read_neighbors`) can read it directly.
    #[must_use]
    pub fn to_content_neighborhood(&self) -> Neighborhood {
        Neighborhood {
            directory: PathBuf::new(),
            families: self.families.clone(),
            type_attributes: self.type_attributes.clone(),
            neighbors: self.neighbors.clone(),
            inventories: self.inventories.clone(),
            data: self.data,
            options: self.options,
        }
    }

    /// The family currently occupying `house`, matching `Sims.exe` `0x425d95`:
    /// checks both the primary `house` field (`+0x110`) and the v8+ secondary
    /// house field (`+0x13c`, `tail[0..4]`). `house == 0` returns `None`.
    #[must_use]
    pub fn family_in(&self, house: u32) -> Option<(u16, &Family)> {
        if house == 0 {
            return None;
        }
        self.families
            .iter()
            .find(|(_, family)| {
                u32::try_from(family.house).ok() == Some(house)
                    || u32::try_from(family.secondary_house()).ok() == Some(house)
            })
            .map(|(&id, family)| (id, family))
    }

    /// Mutable access to the family currently occupying `house` (checking both
    /// primary and secondary house fields per `0x425d95`), if any.
    pub fn family_in_mut(&mut self, house: u32) -> Option<(u16, &mut Family)> {
        if house == 0 {
            return None;
        }
        self.families
            .iter_mut()
            .find(|(_, family)| {
                u32::try_from(family.house).ok() == Some(house)
                    || u32::try_from(family.secondary_house()).ok() == Some(house)
            })
            .map(|(&id, family)| (id, family))
    }

    /// Playable Family Bin households (`id != 0`, `house == 0`,
    /// `secondary_house() == 0`, non-empty member list), in FAMI chunk ID order.
    #[must_use]
    pub fn family_bin(&self) -> Vec<(u16, &Family)> {
        self.families
            .iter()
            .filter(|&(&id, family)| {
                id != 0
                    && family.house == 0
                    && family.secondary_house() == 0
                    && !family.members.is_empty()
            })
            .map(|(&id, family)| (id, family))
            .collect()
    }

    /// Whether `house` is zoned residential (`1`, matching `0x427327..0x42734a`).
    #[must_use]
    pub fn is_residential(&self, house: u32) -> bool {
        i16::try_from(house)
            .ok()
            .and_then(|lot| self.zoning.get(&lot).copied())
            == Some(1)
    }

    /// Encodes this neighborhood state with the [`NEIGHBORHOOD_FORMAT`] header.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = NEIGHBORHOOD_FORMAT.to_le_bytes().to_vec();
        out.extend(postcard::to_allocvec(self).expect("encoding to memory cannot fail"));
        out
    }

    /// Decodes bytes written by [`StoredNeighborhood::encode`].
    pub fn decode(name: &str, bytes: &[u8]) -> Result<Self> {
        let (version, body) = bytes
            .split_first_chunk::<4>()
            .ok_or_else(|| Error::Snapshot(format!("{name}: shorter than its header")))?;
        let version = u32::from_le_bytes(*version);
        if version != NEIGHBORHOOD_FORMAT {
            return Err(Error::Snapshot(format!(
                "{name}: neighborhood format {version}, expected {NEIGHBORHOOD_FORMAT}"
            )));
        }
        postcard::from_bytes(body)
            .map_err(|error| Error::Snapshot(format!("{name}: invalid neighborhood data: {error}")))
    }

    /// Writes back live simulation neighborhood state from `world` into this
    /// persistent neighborhood record, following `Sims.exe` 1.91
    /// `SaveCurrentHouse` (`0x428687`) → `cTSNeighborhood::Save` (`0x4252cc`).
    ///
    /// Callers must have already flushed live persons via
    /// `sims_vm::people::save_to_record` (`0x425a69`) and refreshed lot values
    /// (`0x416085`) before calling this method.
    pub fn write_back_world(
        &mut self,
        world: &World,
        current_house: u32,
        family_id: u16,
        lot_value_total: i32,
        family_flags_update: Option<(bool, bool)>,
        friend_threshold: i32,
    ) {
        // 1. NGBH data words & inventories (0x42535d, 0x425371).
        self.data = world.neighborhood_data;
        if world.globals.get(26).copied().unwrap_or(0) != 0
            && current_house != 0
            && let Ok(lot_word) = i16::try_from(current_house)
        {
            self.data[2] = lot_word;
        }
        self.inventories = world
            .neighbors
            .inventories
            .iter()
            .map(|inv| {
                Some(Inventory {
                    neighbor: inv.neighbor,
                    items: inv
                        .items
                        .iter()
                        .map(|item| InventoryItem {
                            kind: item.kind,
                            guid: item.guid,
                            count: item.count,
                        })
                        .collect(),
                })
            })
            .collect();

        // 2. NBRS slots (0x4253b0): preserve stored header metadata where slot
        // identity matches, updating revision, person_data, and relationships.
        let mut updated_neighbors = Vec::with_capacity(world.neighbors.slots.len());
        for (index, slot) in world.neighbors.slots.iter().enumerate() {
            let Some(live) = slot else {
                updated_neighbors.push(None);
                continue;
            };
            let existing = self
                .neighbors
                .get(index)
                .and_then(|entry| entry.as_ref())
                .filter(|prev| prev.id == live.id && prev.guid == live.guid);
            let person_data = if live.revision > 0 {
                live.data.clone()
            } else {
                Vec::new()
            };
            let relationships = live
                .relationships
                .iter()
                .map(|row| SavedRelationship {
                    target: row.target,
                    values: row.values.clone(),
                })
                .collect();
            let saved = match existing {
                Some(prev) => Neighbor {
                    version: if person_data.len() > 80 {
                        10
                    } else {
                        prev.version
                    },
                    version_extra: if person_data.len() > 80 {
                        Some(prev.version_extra.unwrap_or(1))
                    } else {
                        prev.version_extra
                    },
                    name: prev.name.clone(),
                    unknown_zero: prev.unknown_zero,
                    person_mode: live.revision,
                    person_data,
                    id: live.id,
                    guid: live.guid,
                    unknown_negative: prev.unknown_negative,
                    relationships,
                },
                None => Neighbor {
                    version: 10,
                    version_extra: Some(1),
                    name: String::new(),
                    unknown_zero: 0,
                    person_mode: live.revision,
                    person_data,
                    id: live.id,
                    guid: live.guid,
                    unknown_negative: -1,
                    relationships,
                },
            };
            updated_neighbors.push(Some(saved));
        }
        self.neighbors = updated_neighbors;

        // 3. FAMI records (0x4286a0..0x428771, 0x4253dd..0x425453, 0x410696).
        // Native `0x410696` always serializes FAMI version 9.
        for stored in self.families.values_mut() {
            stored.upgrade_to_v9();
        }
        for live_family in &world.families {
            let Ok(id) = u16::try_from(live_family.id) else {
                continue;
            };
            let Some(stored) = self.families.get_mut(&id) else {
                continue;
            };
            stored.house = live_family.house;
            stored.members = live_family.members.clone();
            stored.friends = if live_family.stale {
                world.count_family_friends(live_family, friend_threshold)
            } else {
                live_family.friends
            };
        }
        let residential = current_house != 0 && self.is_residential(current_house);
        if family_id != 0
            && let Some(active) = self.families.get_mut(&family_id)
        {
            active.budget = world.budget.funds;
            active.flags &= !(FAMILY_FLAG_BIT_3 | FAMILY_FLAG_NEW_MOVE_IN);
            if let Some((has_cat1, has_cat9)) = family_flags_update {
                if has_cat1 {
                    active.flags |= FAMILY_FLAG_HAS_CATEGORY_1;
                } else {
                    active.flags &= !FAMILY_FLAG_HAS_CATEGORY_1;
                }
                if has_cat9 {
                    active.flags |= FAMILY_FLAG_HAS_CATEGORY_9;
                } else {
                    active.flags &= !FAMILY_FLAG_HAS_CATEGORY_9;
                }
            }
            if current_house != 0 && u32::try_from(active.house).ok() == Some(current_house) {
                active.architecture_value = lot_value_total;
            }
            // 0x410a87: merge the signed maximum of each retained magic-history
            // word and the live Magic Controller (GUID 0xb6c90029) attribute
            // on a residential lot before saving FAMI v9.
            let mut history = match &world.magic_history {
                Some(magic) if magic.family == i32::from(family_id) => magic.values,
                _ => active.magic_history().ok().flatten().unwrap_or([0; 6]),
            };
            if residential
                && world.globals.get(9).copied().map(i32::from) == Some(i32::from(family_id))
                && let Some(controller) = world
                    .order
                    .iter()
                    .filter_map(|id| world.objects.get(id))
                    .find(|obj| obj.guid == 0xb6c9_0029)
            {
                for (slot, word) in history.iter_mut().enumerate() {
                    if let Some(&attr) = controller.attributes.get(slot)
                        && attr > *word
                    {
                        *word = attr;
                    }
                }
            }
            let _ = active.set_magic_history(history);
        }

        // 4. TATT (0x425455 → 0x43e0be): merge updated world type attributes.
        if !world.type_attributes.is_empty() {
            let (version, packed, mut entries) = match self.type_attributes.take() {
                Some(prev) => (prev.version, prev.packed, prev.entries),
                None => (0, true, Vec::new()),
            };
            for record in &world.type_attributes {
                if let Some(existing) = entries.iter_mut().find(|e| e.guid == record.guid) {
                    existing.values = record.values.clone();
                } else if record.values.iter().any(|&v| v != 0) {
                    entries.push(TattEntry {
                        guid: record.guid,
                        values: record.values.clone(),
                        count_end: 0,
                        end: 0,
                    });
                }
            }
            self.type_attributes = Some(Tatt {
                version,
                packed,
                entries,
                count_end: 0,
                consumed: 0,
            });
        }

        // 5. Runtime-made characters: replace from the authoritative live world
        // so bulldozed deletions are never resurrected by older lot saves.
        self.made_characters = world.characters.clone();

        self.revision = self.revision.saturating_add(1);
    }
}

/// A REDB-backed multi-neighborhood and multi-lot persistence store.
pub struct NeighborhoodStore {
    db: Database,
    store_id: u128,
}

fn store(error: impl std::fmt::Display) -> Error {
    Error::Store(error.to_string())
}

/// Mixes a store's persisted database ID with its OS file identity (device +
/// inode on Unix, falling back to canonical path when unavailable) so copying a
/// `.redb` file on disk yields a distinct store identity while reopening or
/// renaming the same file on the same volume preserves it.
fn bind_store_file_identity(persisted_id: u128, canonical_path: &Path) -> u128 {
    let mut hasher = Sha256::new();
    hasher.update(b"store-file-identity");
    hasher.update(persisted_id.to_le_bytes());
    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(canonical_path) {
        use std::os::unix::fs::MetadataExt;
        hasher.update(b"unix-dev-ino");
        hasher.update(meta.dev().to_le_bytes());
        hasher.update(meta.ino().to_le_bytes());
    } else {
        hasher.update(b"path");
        hasher.update(canonical_path.to_string_lossy().as_bytes());
    }
    #[cfg(not(unix))]
    {
        hasher.update(b"path");
        hasher.update(canonical_path.to_string_lossy().as_bytes());
    }
    let digest = hasher.finalize();
    let bytes: [u8; 16] = digest[..16].try_into().expect("sha256 is 32 bytes");
    let id = u128::from_le_bytes(bytes);
    if id == 0 { 1 } else { id }
}

/// Strictly advances `NEIGHBORHOODS[id].revision` inside an open write
/// transaction whenever neighborhood `id` already exists, invalidating any
/// already-open lot session against stale overwrites.
fn bump_neighborhood_revision(txn: &redb::WriteTransaction, id: &str) -> Result<()> {
    let mut table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
    let existing = table
        .get(id)
        .map_err(store)?
        .map(|v| StoredNeighborhood::decode(id, v.value()))
        .transpose()?;
    if let Some(mut nh) = existing {
        nh.revision = nh.revision.saturating_add(1);
        let encoded = nh.encode();
        table.insert(id, encoded.as_slice()).map_err(store)?;
    }
    Ok(())
}

impl NeighborhoodStore {
    /// Opens an existing REDB neighborhood store or creates a new one,
    /// ensuring a persistent 128-bit store identity bound to both the database
    /// metadata and the underlying file identity (canonical path and Unix
    /// device/inode) so copied database files never share a store identity.
    pub fn open(path: &Path) -> Result<Self> {
        let db = Database::create(path).map_err(store)?;
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let existing = {
            let txn = db.begin_read().map_err(store)?;
            match txn.open_table(NEIGHBORHOOD_META) {
                Ok(table) => table
                    .get(STORE_ID_KEY)
                    .map_err(store)?
                    .and_then(|v| v.value().try_into().ok())
                    .map(u128::from_le_bytes),
                Err(redb::TableError::TableDoesNotExist(_)) => None,
                Err(error) => return Err(store(error)),
            }
        };
        let persisted_id = match existing {
            Some(id) => id,
            None => {
                let candidate = fresh_unique_id("store", 0, canon.to_string_lossy().as_bytes());
                let txn = db.begin_write().map_err(store)?;
                let committed_id = {
                    let mut table = txn.open_table(NEIGHBORHOOD_META).map_err(store)?;
                    if let Some(found) = table
                        .get(STORE_ID_KEY)
                        .map_err(store)?
                        .and_then(|v| v.value().try_into().ok())
                        .map(u128::from_le_bytes)
                    {
                        found
                    } else {
                        table
                            .insert(STORE_ID_KEY, candidate.to_le_bytes().as_slice())
                            .map_err(store)?;
                        candidate
                    }
                };
                txn.commit().map_err(store)?;
                committed_id
            }
        };
        let store_id = bind_store_file_identity(persisted_id, &canon);
        Ok(Self { db, store_id })
    }

    /// Persistent 128-bit identity of this REDB store file.
    #[must_use]
    pub fn store_id(&self) -> u128 {
        self.store_id
    }

    /// Imports a neighborhood directory (`UserData`, `UserData2`..`UserData8`)
    /// into the store under `id` without modifying any files in `directory`.
    ///
    /// Re-importing an existing `id` atomically replaces its neighborhood
    /// metadata, character files, base house files, and any previously saved
    /// lots.
    pub fn import_directory(
        &self,
        id: &str,
        directory: &Path,
        expansions: u16,
    ) -> Result<ImportSummary> {
        if id.is_empty() {
            return Err(Error::Store("neighborhood id must not be empty".into()));
        }
        let neighborhood = Neighborhood::open(directory).map_err(store)?;
        let zoning = neighborhood.zoning(expansions).map_err(store)?;
        let lot_rows = neighborhood.lot_rows().map_err(store)?;

        let mut characters: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut character_file_count = 0usize;
        for (rel_path, path) in neighborhood.character_tree().map_err(store)? {
            if rel_path.ends_with('/') {
                characters.insert(rel_path, Vec::new());
            } else {
                let bytes = std::fs::read(&path).map_err(store)?;
                characters.insert(rel_path, bytes);
                character_file_count += 1;
            }
        }

        let mut houses: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
        for (number, path) in neighborhood.houses().map_err(store)? {
            let bytes = std::fs::read(&path).map_err(store)?;
            houses.insert(number, bytes);
        }

        let mut stored = StoredNeighborhood {
            name: id.to_owned(),
            families: neighborhood.families,
            neighbors: neighborhood.neighbors,
            inventories: neighborhood.inventories,
            data: neighborhood.data,
            options: neighborhood.options,
            type_attributes: neighborhood.type_attributes,
            zoning,
            lot_rows,
            made_characters: Characters::default(),
            deleted_characters: BTreeSet::new(),
            instance_id: 0,
            revision: 0,
        };
        let summary = ImportSummary {
            id: id.to_owned(),
            families: stored.families.len(),
            neighbors: stored.neighbors.iter().flatten().count(),
            characters: character_file_count,
            houses: houses.len(),
        };

        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut nh_table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            let prev_instance = nh_table
                .get(id)
                .map_err(store)?
                .and_then(|v| StoredNeighborhood::decode(id, v.value()).ok())
                .map_or(0, |prev| prev.instance_id);
            let mut seed = self.store_id.to_le_bytes().to_vec();
            seed.extend_from_slice(id.as_bytes());
            seed.extend_from_slice(directory.to_string_lossy().as_bytes());
            stored.instance_id = fresh_unique_id("neighborhood", prev_instance, &seed);
            let encoded = stored.encode();
            let mut char_table = txn.open_table(NEIGHBORHOOD_CHARACTERS).map_err(store)?;
            let mut house_table = txn.open_table(NEIGHBORHOOD_HOUSES).map_err(store)?;
            let mut lot_table = txn.open_table(NEIGHBORHOOD_LOTS).map_err(store)?;
            let mut meta_table = txn.open_table(NEIGHBORHOOD_META).map_err(store)?;

            // Remove any prior records for this neighborhood ID cleanly.
            let mut old_chars = Vec::new();
            for item in char_table.range((id, "")..).map_err(store)? {
                let (k, _) = item.map_err(store)?;
                let (nh_key, name) = k.value();
                if nh_key != id {
                    break;
                }
                old_chars.push(name.to_owned());
            }
            for name in old_chars {
                char_table.remove((id, name.as_str())).map_err(store)?;
            }
            let old_houses: Vec<u32> = house_table
                .range((id, 0u32)..=(id, u32::MAX))
                .map_err(store)?
                .filter_map(|item| item.ok().map(|(k, _)| k.value().1))
                .collect();
            for number in old_houses {
                house_table.remove((id, number)).map_err(store)?;
            }
            let old_lots: Vec<u32> = lot_table
                .range((id, 0u32)..=(id, u32::MAX))
                .map_err(store)?
                .filter_map(|item| item.ok().map(|(k, _)| k.value().1))
                .collect();
            for number in old_lots {
                lot_table.remove((id, number)).map_err(store)?;
            }

            nh_table.insert(id, encoded.as_slice()).map_err(store)?;
            for (name, bytes) in &characters {
                char_table
                    .insert((id, name.as_str()), bytes.as_slice())
                    .map_err(store)?;
            }
            for (&number, bytes) in &houses {
                house_table
                    .insert((id, number), bytes.as_slice())
                    .map_err(store)?;
            }
            if meta_table.get(ACTIVE_KEY).map_err(store)?.is_none() {
                meta_table
                    .insert(ACTIVE_KEY, id.as_bytes())
                    .map_err(store)?;
            }
        }
        txn.commit().map_err(store)?;
        Ok(summary)
    }

    /// Lists all imported neighborhood IDs in sorted order.
    pub fn list_neighborhoods(&self) -> Result<Vec<String>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOODS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(error) => return Err(store(error)),
        };
        let mut names = Vec::new();
        for entry in table.iter().map_err(store)? {
            let (key, _) = entry.map_err(store)?;
            names.push(key.value().to_owned());
        }
        Ok(names)
    }

    /// Returns the currently selected active neighborhood ID, if any.
    pub fn active_neighborhood(&self) -> Result<Option<String>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_META) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        let Some(value) = table.get(ACTIVE_KEY).map_err(store)? else {
            return Ok(None);
        };
        let text = std::str::from_utf8(value.value())
            .map_err(|error| Error::Snapshot(format!("active neighborhood key: {error}")))?;
        Ok(Some(text.to_owned()))
    }

    /// Switches the active neighborhood to `id`, failing if `id` does not exist.
    pub fn set_active_neighborhood(&self, id: &str) -> Result<()> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let nh_table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            if nh_table.get(id).map_err(store)?.is_none() {
                return Err(Error::Store(format!("neighborhood {id:?} does not exist")));
            }
            let mut meta_table = txn.open_table(NEIGHBORHOOD_META).map_err(store)?;
            meta_table
                .insert(ACTIVE_KEY, id.as_bytes())
                .map_err(store)?;
        }
        txn.commit().map_err(store)
    }

    /// Deletes a neighborhood and all of its characters, base houses, and saved
    /// lots atomically. Returns `true` if the neighborhood existed.
    pub fn delete_neighborhood(&self, id: &str) -> Result<bool> {
        let txn = self.db.begin_write().map_err(store)?;
        let existed;
        {
            let mut nh_table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            existed = nh_table.remove(id).map_err(store)?.is_some();
            if existed {
                let mut char_table = txn.open_table(NEIGHBORHOOD_CHARACTERS).map_err(store)?;
                let mut chars = Vec::new();
                for item in char_table.range((id, "")..).map_err(store)? {
                    let (k, _) = item.map_err(store)?;
                    let (nh_key, name) = k.value();
                    if nh_key != id {
                        break;
                    }
                    chars.push(name.to_owned());
                }
                for name in chars {
                    char_table.remove((id, name.as_str())).map_err(store)?;
                }
                let mut house_table = txn.open_table(NEIGHBORHOOD_HOUSES).map_err(store)?;
                let houses: Vec<u32> = house_table
                    .range((id, 0u32)..=(id, u32::MAX))
                    .map_err(store)?
                    .filter_map(|item| item.ok().map(|(k, _)| k.value().1))
                    .collect();
                for number in houses {
                    house_table.remove((id, number)).map_err(store)?;
                }
                let mut lot_table = txn.open_table(NEIGHBORHOOD_LOTS).map_err(store)?;
                let lots: Vec<u32> = lot_table
                    .range((id, 0u32)..=(id, u32::MAX))
                    .map_err(store)?
                    .filter_map(|item| item.ok().map(|(k, _)| k.value().1))
                    .collect();
                for number in lots {
                    lot_table.remove((id, number)).map_err(store)?;
                }
                let mut meta_table = txn.open_table(NEIGHBORHOOD_META).map_err(store)?;
                let is_active = meta_table
                    .get(ACTIVE_KEY)
                    .map_err(store)?
                    .is_some_and(|v| v.value() == id.as_bytes());
                if is_active {
                    meta_table.remove(ACTIVE_KEY).map_err(store)?;
                }
            }
        }
        txn.commit().map_err(store)?;
        Ok(existed)
    }

    /// Loads the stored neighborhood metadata for `id`, if present.
    pub fn load_neighborhood(&self, id: &str) -> Result<Option<StoredNeighborhood>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOODS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        let Some(value) = table.get(id).map_err(store)? else {
            return Ok(None);
        };
        StoredNeighborhood::decode(id, value.value()).map(Some)
    }

    /// Loads a consistent point-in-time snapshot of neighborhood `id`, its
    /// character files, and `(saved_lot, base_house)` for `house` inside a
    /// single REDB read transaction.
    pub fn load_lot_snapshot(&self, id: &str, house: u32) -> Result<Option<LotSnapshot>> {
        let txn = self.db.begin_read().map_err(store)?;
        let nh_table = match txn.open_table(NEIGHBORHOODS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        let Some(nh_value) = nh_table.get(id).map_err(store)? else {
            return Ok(None);
        };
        let neighborhood = StoredNeighborhood::decode(id, nh_value.value())?;

        let mut character_files = BTreeMap::new();
        match txn.open_table(NEIGHBORHOOD_CHARACTERS) {
            Ok(table) => {
                for entry in table.range((id, "")..).map_err(store)? {
                    let (key, value) = entry.map_err(store)?;
                    let (nh_key, name) = key.value();
                    if nh_key != id {
                        break;
                    }
                    character_files.insert(name.to_owned(), value.value().to_vec());
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store(error)),
        }

        let saved_lot = match txn.open_table(NEIGHBORHOOD_LOTS) {
            Ok(table) => table
                .get((id, house))
                .map_err(store)?
                .map(|value| value.value().to_vec()),
            Err(redb::TableError::TableDoesNotExist(_)) => None,
            Err(error) => return Err(store(error)),
        };

        let base_house = match txn.open_table(NEIGHBORHOOD_HOUSES) {
            Ok(table) => table
                .get((id, house))
                .map_err(store)?
                .map(|value| value.value().to_vec()),
            Err(redb::TableError::TableDoesNotExist(_)) => None,
            Err(error) => return Err(store(error)),
        };

        Ok(Some(LotSnapshot {
            neighborhood,
            character_files,
            saved_lot,
            base_house,
        }))
    }

    /// Saves `state` for neighborhood `id`, replacing any previous state.
    ///
    /// When overwriting an existing neighborhood record, strictly advances the
    /// committed `revision` past the previously stored revision so any already-
    /// open lot session cannot silently overwrite the new state on Save & Leave.
    pub fn save_neighborhood(&self, id: &str, state: &StoredNeighborhood) -> Result<()> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            let previous = table
                .get(id)
                .map_err(store)?
                .and_then(|v| StoredNeighborhood::decode(id, v.value()).ok());
            let mut committed = state.clone();
            if let Some(prev) = previous {
                if committed.revision <= prev.revision {
                    committed.revision = prev.revision.saturating_add(1);
                }
                if committed.instance_id == 0 {
                    committed.instance_id = prev.instance_id;
                }
            }
            if committed.instance_id == 0 {
                committed.instance_id = fresh_unique_id("neighborhood", 0, id.as_bytes());
            }
            let encoded = committed.encode();
            table.insert(id, encoded.as_slice()).map_err(store)?;
        }
        txn.commit().map_err(store)
    }

    /// Atomically saves `state` for neighborhood `id` only if the committed
    /// neighborhood's `(instance_id, revision)` still matches
    /// `(expected_instance_id, expected_revision)`, always preserving
    /// `instance_id` and strictly advancing `revision` past the committed
    /// revision so no concurrent or open lot session can silently overwrite it.
    pub fn save_neighborhood_checked(
        &self,
        id: &str,
        expected_instance_id: u128,
        expected_revision: u64,
        state: &StoredNeighborhood,
    ) -> Result<std::result::Result<(), CheckedCommitError>> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            let current = {
                let Some(existing) = table.get(id).map_err(store)? else {
                    return Err(Error::Store(format!("neighborhood {id:?} does not exist")));
                };
                StoredNeighborhood::decode(id, existing.value())?
            };
            if current.instance_id != expected_instance_id {
                return Ok(Err(CheckedCommitError::InstanceMismatch {
                    expected: expected_instance_id,
                    current: current.instance_id,
                }));
            }
            if current.revision != expected_revision {
                return Ok(Err(CheckedCommitError::RevisionMismatch {
                    expected: expected_revision,
                    current: current.revision,
                }));
            }
            let mut committed = state.clone();
            committed.instance_id = current.instance_id;
            if committed.revision <= current.revision {
                committed.revision = current.revision.saturating_add(1);
            }
            let encoded = committed.encode();
            table.insert(id, encoded.as_slice()).map_err(store)?;
        }
        txn.commit().map_err(store)?;
        Ok(Ok(()))
    }

    /// Loads all character `.iff` files stored for neighborhood `id`.
    pub fn load_character_files(&self, id: &str) -> Result<BTreeMap<String, Vec<u8>>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_CHARACTERS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(BTreeMap::new()),
            Err(error) => return Err(store(error)),
        };
        let mut files = BTreeMap::new();
        for entry in table.range((id, "")..).map_err(store)? {
            let (key, value) = entry.map_err(store)?;
            let (nh_key, name) = key.value();
            if nh_key != id {
                break;
            }
            files.insert(name.to_owned(), value.value().to_vec());
        }
        Ok(files)
    }

    /// Stores a character file under neighborhood `id`, bumping `id`'s
    /// neighborhood revision if it already exists so open lot sessions cannot
    /// overwrite the updated neighborhood state.
    pub fn save_character_file(&self, id: &str, filename: &str, bytes: &[u8]) -> Result<()> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut table = txn.open_table(NEIGHBORHOOD_CHARACTERS).map_err(store)?;
            table.insert((id, filename), bytes).map_err(store)?;
        }
        bump_neighborhood_revision(&txn, id)?;
        txn.commit().map_err(store)
    }

    /// Lists all house numbers available in neighborhood `id` (from base house
    /// files and/or saved lots), in ascending order.
    pub fn list_houses(&self, id: &str) -> Result<Vec<u32>> {
        let txn = self.db.begin_read().map_err(store)?;
        let mut houses = std::collections::BTreeSet::new();
        match txn.open_table(NEIGHBORHOOD_HOUSES) {
            Ok(table) => {
                for entry in table.range((id, 0u32)..=(id, u32::MAX)).map_err(store)? {
                    let (key, _) = entry.map_err(store)?;
                    houses.insert(key.value().1);
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store(error)),
        }
        match txn.open_table(NEIGHBORHOOD_LOTS) {
            Ok(table) => {
                for entry in table.range((id, 0u32)..=(id, u32::MAX)).map_err(store)? {
                    let (key, _) = entry.map_err(store)?;
                    houses.insert(key.value().1);
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {}
            Err(error) => return Err(store(error)),
        }
        Ok(houses.into_iter().collect())
    }

    /// Loads the imported base `HouseXX.iff` bytes for `(id, house)`, if any.
    pub fn load_base_house(&self, id: &str, house: u32) -> Result<Option<Vec<u8>>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_HOUSES) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        Ok(table
            .get((id, house))
            .map_err(store)?
            .map(|value| value.value().to_vec()))
    }

    /// Stores base `HouseXX.iff` bytes for `(id, house)`, bumping `id`'s
    /// neighborhood revision if it already exists so open lot sessions cannot
    /// overwrite the replaced base house on Save & Leave.
    pub fn save_base_house(&self, id: &str, house: u32, bytes: &[u8]) -> Result<()> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut table = txn.open_table(NEIGHBORHOOD_HOUSES).map_err(store)?;
            table.insert((id, house), bytes).map_err(store)?;
        }
        bump_neighborhood_revision(&txn, id)?;
        txn.commit().map_err(store)
    }

    /// Loads the latest saved lot bytes for `(id, house)`, if any.
    pub fn load_saved_lot(&self, id: &str, house: u32) -> Result<Option<Vec<u8>>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(NEIGHBORHOOD_LOTS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        Ok(table
            .get((id, house))
            .map_err(store)?
            .map(|value| value.value().to_vec()))
    }

    /// Atomically persists both a saved lot blob and the updated neighborhood
    /// state in a single REDB write transaction.
    pub fn save_lot_and_neighborhood(
        &self,
        id: &str,
        house: u32,
        lot_bytes: &[u8],
        state: &StoredNeighborhood,
    ) -> Result<()> {
        let _ = self.save_lot_and_neighborhood_checked(id, house, None, lot_bytes, state, &[])?;
        Ok(())
    }

    /// Atomically persists a saved lot blob, the updated neighborhood state,
    /// and any character-file deletions (e.g. bulldozed tombstones per
    /// `0x426fcd`) in a single REDB write transaction.
    ///
    /// When `expected` is `Some((expected_instance_id, expected_revision))`,
    /// verifies inside the write transaction that both the committed
    /// neighborhood's `instance_id` and `revision` still match; if the
    /// neighborhood was re-imported or another operation committed a newer
    /// revision meanwhile, aborts the transaction cleanly and returns
    /// `Ok(Err(CheckedCommitError))`. Always preserves `instance_id` and
    /// strictly advances `revision` past the previously committed revision.
    pub fn save_lot_and_neighborhood_checked(
        &self,
        id: &str,
        house: u32,
        expected: Option<(u128, u64)>,
        lot_bytes: &[u8],
        state: &StoredNeighborhood,
        remove_character_files: &[String],
    ) -> Result<std::result::Result<(), CheckedCommitError>> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut nh_table = txn.open_table(NEIGHBORHOODS).map_err(store)?;
            let committed = if let Some((expected_instance_id, expected_revision)) = expected {
                let Some(existing) = nh_table.get(id).map_err(store)? else {
                    return Err(Error::Store(format!("neighborhood {id:?} does not exist")));
                };
                let current = StoredNeighborhood::decode(id, existing.value())?;
                if current.instance_id != expected_instance_id {
                    return Ok(Err(CheckedCommitError::InstanceMismatch {
                        expected: expected_instance_id,
                        current: current.instance_id,
                    }));
                }
                if current.revision != expected_revision {
                    return Ok(Err(CheckedCommitError::RevisionMismatch {
                        expected: expected_revision,
                        current: current.revision,
                    }));
                }
                let mut next = state.clone();
                next.instance_id = current.instance_id;
                if next.revision <= current.revision {
                    next.revision = current.revision.saturating_add(1);
                }
                next
            } else {
                let previous = nh_table
                    .get(id)
                    .map_err(store)?
                    .and_then(|v| StoredNeighborhood::decode(id, v.value()).ok());
                let mut next = state.clone();
                if let Some(prev) = previous {
                    if next.revision <= prev.revision {
                        next.revision = prev.revision.saturating_add(1);
                    }
                    if next.instance_id == 0 {
                        next.instance_id = prev.instance_id;
                    }
                }
                if next.instance_id == 0 {
                    next.instance_id = fresh_unique_id("neighborhood", 0, id.as_bytes());
                }
                next
            };
            let encoded = committed.encode();
            let mut lot_table = txn.open_table(NEIGHBORHOOD_LOTS).map_err(store)?;
            lot_table.insert((id, house), lot_bytes).map_err(store)?;
            nh_table.insert(id, encoded.as_slice()).map_err(store)?;
            if !remove_character_files.is_empty() {
                let mut char_table = txn.open_table(NEIGHBORHOOD_CHARACTERS).map_err(store)?;
                for rel_path in remove_character_files {
                    char_table.remove((id, rel_path.as_str())).map_err(store)?;
                }
            }
        }
        txn.commit().map_err(store)?;
        Ok(Ok(()))
    }

    /// Computes a base house's total lot price directly from its `SIMI` chunk
    /// (`0x427f00` → `0x427f78`), without requiring a full catalog link.
    pub fn base_house_lot_price(
        &self,
        id: &str,
        house: u32,
        arch_multiplier: f32,
    ) -> Result<Option<i32>> {
        let Some(bytes) = self.load_base_house(id, house)? else {
            return Ok(None);
        };
        let iff =
            Iff::parse(bytes).map_err(|e| Error::Snapshot(format!("{id} House{house:02}: {e}")))?;
        let chunk = iff
            .chunks
            .iter()
            .find(|c| &c.tag == b"SIMI")
            .ok_or_else(|| Error::Snapshot(format!("{id} House{house:02}: missing SIMI chunk")))?;
        let sim = Simulator::parse(iff.data(chunk))
            .map_err(|e| Error::Snapshot(format!("{id} House{house:02} SIMI: {e}")))?;
        Ok(Some(lot_total_value(
            sim.lot_value,
            sim.objects_value,
            sim.architecture_value,
            arch_multiplier,
        )))
    }
}
