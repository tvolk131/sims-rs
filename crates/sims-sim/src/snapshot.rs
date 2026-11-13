use crate::{Error, Result, World};
use redb::{Database, ReadableDatabase, TableDefinition};
use std::path::Path;

/// Snapshots by name. Each value is a format version (u32, little-endian)
/// followed by the world's canonical encoding.
const SNAPSHOTS: TableDefinition<&str, &[u8]> = TableDefinition::new("snapshots");

/// Bumped whenever [`World`]'s encoding changes, so an old snapshot is
/// refused instead of misread.
pub const FORMAT: u32 = 3;

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
        let txn = self.db.begin_write().map_err(store)?;
        {
            let mut table = txn.open_table(SNAPSHOTS).map_err(store)?;
            table.insert(name, value.as_slice()).map_err(store)?;
        }
        txn.commit().map_err(store)
    }

    /// The snapshot saved under `name`, if any.
    pub fn load(&self, name: &str) -> Result<Option<World>> {
        let txn = self.db.begin_read().map_err(store)?;
        let table = match txn.open_table(SNAPSHOTS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(store(error)),
        };
        let Some(value) = table.get(name).map_err(store)? else {
            return Ok(None);
        };
        let bytes = value.value();
        let (version, encoded) = bytes
            .split_first_chunk::<4>()
            .ok_or_else(|| Error::Snapshot(format!("{name}: shorter than its header")))?;
        let version = u32::from_le_bytes(*version);
        if version != FORMAT {
            return Err(Error::Snapshot(format!(
                "{name}: format {version}, expected {FORMAT}"
            )));
        }
        World::decode(encoded).map(Some)
    }
}
