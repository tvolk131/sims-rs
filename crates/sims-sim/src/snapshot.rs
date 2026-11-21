use crate::{Error, Result, World};
use redb::{Database, ReadOnlyDatabase, ReadableDatabase, TableDefinition};
use std::path::Path;

/// Snapshots by name. Each value is a format version (u32, little-endian), an
/// optional floor-rotation byte when [`FLOOR_ROTATION_FLAG`] is set in the
/// version word, and then the world's canonical encoding.
const SNAPSHOTS: TableDefinition<&str, &[u8]> = TableDefinition::new("snapshots");

/// Saved lots by name: what `sims_vm::boot::save` encodes, the world with
/// the lot's own state, kept as its bytes.
const LOTS: TableDefinition<&str, &[u8]> = TableDefinition::new("lots");

/// Bumped whenever [`World`]'s encoding changes, so an old snapshot is
/// refused instead of misread.
pub const FORMAT: u32 = 31;

/// High bit set on [`FORMAT`] inside a `SNAPSHOTS` record when a 1-byte
/// `HOUS 0` floor rotation (`0..=3`) immediately follows the 4-byte version
/// word before the world's canonical encoding. Keeping the rotation in the
/// snapshot record itself ensures any writer overwriting a named snapshot
/// replaces both the world and its rotation metadata atomically.
const FLOOR_ROTATION_FLAG: u32 = 1 << 31;

/// A REDB file of named world snapshots.
pub struct Snapshots {
    db: Database,
}

fn store(error: impl std::fmt::Display) -> Error {
    Error::Store(error.to_string())
}

impl Snapshots {
    /// Opens the file, creating it if needed.
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            db: Database::create(path).map_err(store)?,
        })
    }

    /// Saves `world` under `name`, replacing any snapshot of that name.
    pub fn save(&self, name: &str, world: &World) -> Result<()> {
        let mut value = FORMAT.to_le_bytes().to_vec();
        value.extend(world.encode());
        self.write_snapshot(name, &value)
    }

    /// Saves `world` under `name` together with the `HOUS 0` camera rotation
    /// (`floor_rotation & 3`) that its level-1 road-dash floor tiles (`10` and
    /// `11`) are oriented for (`0x4a8042`, `0x4a861f`), inside the snapshot
    /// record itself.
    pub fn save_with_floor_rotation(
        &self,
        name: &str,
        world: &World,
        floor_rotation: u8,
    ) -> Result<()> {
        let mut value = (FORMAT | FLOOR_ROTATION_FLAG).to_le_bytes().to_vec();
        value.push(floor_rotation & 3);
        value.extend(world.encode());
        self.write_snapshot(name, &value)
    }

    fn write_snapshot(&self, name: &str, value: &[u8]) -> Result<()> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut table = txn.open_table(SNAPSHOTS).map_err(store)?;
            table.insert(name, value).map_err(store)?;
        }
        txn.commit().map_err(store)
    }

    /// The snapshot saved under `name`, if any.
    pub fn load(&self, name: &str) -> Result<Option<World>> {
        Ok(load_with_floor_rotation(&self.db, name)?.map(|(world, _)| world))
    }

    /// The snapshot saved under `name`, if any, together with the `HOUS 0`
    /// floor rotation (`0..=3`) recorded inside that snapshot record, if any.
    pub fn load_with_floor_rotation(&self, name: &str) -> Result<Option<(World, Option<u8>)>> {
        load_with_floor_rotation(&self.db, name)
    }

    /// Keeps a saved lot's bytes under `name`, replacing any of that name.
    /// The lots are a table of their own, apart from the snapshots.
    pub fn save_lot(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut table = txn.open_table(LOTS).map_err(store)?;
            table.insert(name, bytes).map_err(store)?;
        }
        txn.commit().map_err(store)
    }

    /// The saved lot's bytes kept under `name`, if any.
    pub fn load_lot(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(LOTS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        Ok(table
            .get(name)
            .map_err(store)?
            .map(|value| value.value().to_vec()))
    }
}

/// A REDB file of named world snapshots, opened only to read: opening it
/// neither creates nor changes the file, so a viewer can read a snapshot
/// another program keeps.
///
/// Opening fails if the file does not exist, if a program has it open to
/// write ([`Snapshots`]) on a platform that locks files, or if it needs
/// the repair a crash while writing leaves, which only opening it to
/// write does.
pub struct ReadOnlySnapshots {
    db: ReadOnlyDatabase,
}

impl ReadOnlySnapshots {
    /// Opens an existing file.
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            db: ReadOnlyDatabase::open(path).map_err(store)?,
        })
    }

    /// The snapshot saved under `name`, if any, as [`Snapshots::load`].
    pub fn load(&self, name: &str) -> Result<Option<World>> {
        Ok(load_with_floor_rotation(&self.db, name)?.map(|(world, _)| world))
    }

    /// The snapshot and optional `HOUS 0` floor rotation (`0..=3`) saved under
    /// `name`, if any, as [`Snapshots::load_with_floor_rotation`].
    pub fn load_with_floor_rotation(&self, name: &str) -> Result<Option<(World, Option<u8>)>> {
        load_with_floor_rotation(&self.db, name)
    }
}

fn load_with_floor_rotation(
    db: &impl ReadableDatabase,
    name: &str,
) -> Result<Option<(World, Option<u8>)>> {
    let txn = db.begin_read().map_err(store)?;
    let table = match txn.open_table(SNAPSHOTS) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
        Err(error) => return Err(store(error)),
    };
    let Some(value) = table.get(name).map_err(store)? else {
        return Ok(None);
    };
    let bytes = value.value();
    let (version, rest) = bytes
        .split_first_chunk::<4>()
        .ok_or_else(|| Error::Snapshot(format!("{name}: shorter than its header")))?;
    let version = u32::from_le_bytes(*version);
    let (floor_rotation, encoded) = if version == FORMAT {
        (None, rest)
    } else if version == (FORMAT | FLOOR_ROTATION_FLAG) {
        let (&rotation, encoded) = rest
            .split_first()
            .ok_or_else(|| Error::Snapshot(format!("{name}: shorter than its header")))?;
        if rotation >= 4 {
            return Err(Error::Snapshot(format!(
                "{name}: invalid floor rotation {rotation}"
            )));
        }
        (Some(rotation), encoded)
    } else {
        return Err(Error::Snapshot(format!(
            "{name}: format {version}, expected {FORMAT}"
        )));
    };
    Ok(Some((World::decode(encoded)?, floor_rotation)))
}
