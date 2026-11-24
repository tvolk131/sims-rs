//! A stored neighborhood saved as the original's `Neighborhood.iff`
//! (`Sims.exe` 1.91 0x4252cc).
//!
//! [`sims_formats::neighborhood_file`] writes the file; this module builds
//! what it writes from a [`StoredNeighborhood`] and writes it into a
//! neighborhood folder ([`StoredNeighborhood::save_neighborhood_iff`]).
//! The save serializes the neighborhood as it stands, so a lot's
//! writeback ([`StoredNeighborhood::write_back_world`]) comes first, as
//! the save's own updates of data word 2 (+0x12c) and the current
//! family's architecture value (+0x11c) come before its chunks.
//!
//! The file holds things the [`StoredNeighborhood`] record does not: each
//! family's FAMh, which the save copies as it is, the bytes the save
//! leaves uninitialized, and the NGBH option bytes and family names as
//! read (the record keeps only the options' truth and trimmed names).
//! [`SaveInputs`] carries them. The store keeps one per neighborhood
//! beside the record ([`crate::NeighborhoodStore::load_save_inputs`]),
//! read from the imported folder's `Neighborhood.iff`.

use crate::neighborhood_store::{StoredNeighborhood, neighborhood_iff_path};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sims_formats::iff::Iff;
use sims_formats::neighborhood::{Neighborhood, Neighbors};
use sims_formats::neighborhood_file::{
    FamilyRecord, NeighborhoodFile, Uninitialized, parse_family_name,
};
use sims_formats::tatt::Tatt;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The version of an encoded [`SaveInputs`] ([`SaveInputs::encode`]).
pub const SAVE_INPUTS_FORMAT: u32 = 1;

/// The file the save writes before it moves it onto `Neighborhood.iff`
/// (0x6c2608).
pub const TEMPORARY_FILE: &str = "NTemp.iff";

/// What the save writes that the [`StoredNeighborhood`] record does not
/// keep.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveInputs {
    /// FAMh payloads by family ID. A family without one gets no FAMh, as
    /// one with neither a `.FAM` file (+0x140) nor a record (+0x144) does
    /// (0x4107cf).
    pub histories: BTreeMap<u16, Vec<u8>>,
    /// The bytes the save leaves uninitialized; zero by default.
    pub uninitialized: Uninitialized,
    /// NGBH option bytes as read. Each is written as read while its truth
    /// is the stored option's; otherwise the option is written as 0 or 1.
    pub option_bytes: Vec<u8>,
    /// Family names as read. The store keeps each trimmed, so a name read
    /// with spaces round it is written as read while it trims to the
    /// stored one.
    pub family_names: BTreeMap<u16, String>,
}

impl SaveInputs {
    /// The inputs a file the save wrote holds, so that saving the
    /// neighborhood read from it gives the same bytes.
    pub fn from_saved(bytes: &[u8]) -> Result<Self> {
        let (file, uninitialized) = NeighborhoodFile::parse(bytes)
            .map_err(|error| Error::Store(format!("Neighborhood.iff: {error}")))?;
        let option_bytes = file.neighborhood.option_bytes;
        let family_names = file
            .families
            .iter()
            .map(|record| (record.id, record.name.clone()))
            .collect();
        let histories = file
            .families
            .into_iter()
            .filter_map(|record| Some((record.id, record.history?)))
            .collect();
        Ok(Self {
            histories,
            uninitialized,
            option_bytes,
            family_names,
        })
    }

    /// The inputs any `Neighborhood.iff` holds: all of them when the save
    /// wrote it ([`SaveInputs::from_saved`]). A file it did not write, such
    /// as a shipped one at an older NGBH version, gives what reads: each
    /// FAMh, each one-entry FAMs name with its last byte, and the NGBH
    /// option bytes.
    pub fn from_file(bytes: &[u8]) -> Result<Self> {
        if let Ok(inputs) = Self::from_saved(bytes) {
            return Ok(inputs);
        }
        let iff = Iff::parse(bytes.to_vec())
            .map_err(|error| Error::Store(format!("Neighborhood.iff: {error}")))?;
        let mut inputs = Self::default();
        // Each ID's resource as the original finds it ([`Iff::get`]), as
        // the strict reader takes it.
        for chunk in iff.all(b"FAMh") {
            if let Some(chunk) = iff.get(b"FAMh", chunk.id) {
                inputs.histories.insert(chunk.id, iff.data(chunk).to_vec());
            }
        }
        for chunk in iff.all(b"FAMs") {
            let Some(chunk) = iff.get(b"FAMs", chunk.id) else {
                continue;
            };
            if let Ok((name, stray)) = parse_family_name(iff.data(chunk)) {
                inputs.family_names.insert(chunk.id, name);
                inputs.uninitialized.family_names.insert(chunk.id, stray);
            }
        }
        if let Some(chunk) = iff.get(b"NGBH", 1)
            && let Ok(neighborhood) = Neighborhood::parse(iff.data(chunk))
        {
            inputs.option_bytes = neighborhood.option_bytes;
        }
        Ok(inputs)
    }

    /// The inputs of a neighborhood folder: those of its `Neighborhood.iff`
    /// ([`SaveInputs::from_file`]), or none if it has none.
    pub fn from_directory(directory: &Path) -> Result<Self> {
        let Some(path) = neighborhood_iff_path(directory) else {
            return Ok(Self::default());
        };
        let bytes = std::fs::read(&path)
            .map_err(|error| Error::Store(format!("{}: {error}", path.display())))?;
        Self::from_file(&bytes)
    }

    /// Keeps the FAMh, name and name byte of the families `keep` accepts.
    pub fn retain_families(&mut self, mut keep: impl FnMut(u16) -> bool) {
        self.histories.retain(|&family, _| keep(family));
        self.family_names.retain(|&family, _| keep(family));
        self.uninitialized
            .family_names
            .retain(|&family, _| keep(family));
    }

    /// [`SAVE_INPUTS_FORMAT`] (little-endian), then the inputs (postcard).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = SAVE_INPUTS_FORMAT.to_le_bytes().to_vec();
        out.extend(postcard::to_allocvec(self).expect("encoding to memory cannot fail"));
        out
    }

    /// Reads [`SaveInputs::encode`]'s bytes; any other version is refused.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let Some((version, body)) = bytes.split_first_chunk::<4>() else {
            return Err(Error::Store("save inputs: truncated header".into()));
        };
        let version = u32::from_le_bytes(*version);
        if version != SAVE_INPUTS_FORMAT {
            return Err(Error::Store(format!(
                "save inputs: unsupported version {version}"
            )));
        }
        let (inputs, rest) = postcard::take_from_bytes(body)
            .map_err(|error| Error::Store(format!("save inputs: {error}")))?;
        if !rest.is_empty() {
            return Err(Error::Store("save inputs: trailing bytes".into()));
        }
        Ok(inputs)
    }
}

/// The option bytes a neighborhood has when no NGBH was read: the
/// reader's defaults for a chunk without them (0x4254f6), as
/// [`Neighborhood::parse`] leaves them.
const DEFAULT_OPTIONS: [bool; 11] = [
    false, true, false, true, false, true, false, true, false, true, true,
];

impl StoredNeighborhood {
    /// The family IDs in the order of the original's family vector, which
    /// it keeps sorted with comparator 0x425209: number (+0x114)
    /// descending, then ID (+0x10c) descending. The ID is compared as a
    /// signed word, so -1 (`0xffff`) sorts last.
    pub fn family_order(&self) -> Vec<u16> {
        let mut order: Vec<_> = self
            .families
            .iter()
            .map(|(&id, family)| (id, family))
            .collect();
        order.sort_by(|(a_id, a), (b_id, b)| {
            (b.number, *b_id as i16).cmp(&(a.number, *a_id as i16))
        });
        order.into_iter().map(|(id, _)| id).collect()
    }

    /// What the neighborhood save writes for this neighborhood.
    pub fn neighborhood_file(&self, inputs: &SaveInputs) -> Result<NeighborhoodFile> {
        let families = self
            .family_order()
            .into_iter()
            .map(|id| {
                Ok(FamilyRecord {
                    id,
                    family: self.families[&id].clone(),
                    name: self.family_name(id, inputs),
                    history: inputs.histories.get(&id).cloned(),
                })
            })
            .collect::<Result<_>>()?;
        Ok(NeighborhoodFile {
            neighborhood: Neighborhood {
                version: 0x49,
                data: self.data,
                inventories: self.inventories.clone(),
                options: self.options.unwrap_or(DEFAULT_OPTIONS),
                option_bytes: inputs.option_bytes.clone(),
                tail: Vec::new(),
            },
            neighbors: Neighbors {
                version: 0x49,
                entries: self.neighbors.clone(),
                tail: Vec::new(),
            },
            families,
            type_attributes: self.type_attributes.clone().unwrap_or(Tatt {
                version: 0,
                packed: true,
                entries: Vec::new(),
                count_end: 0,
                consumed: 0,
            }),
        })
    }

    /// The bytes of `Neighborhood.iff` as the neighborhood save writes
    /// them.
    pub fn neighborhood_iff(&self, inputs: &SaveInputs) -> Result<Vec<u8>> {
        self.neighborhood_file(inputs)?
            .to_bytes(&inputs.uninitialized)
            .map_err(|error| Error::Store(format!("Neighborhood.iff: {error}")))
    }

    /// Family `id`'s name: as read while it trims to the stored surname
    /// (an empty one when none is stored), else the stored surname.
    fn family_name(&self, id: u16, inputs: &SaveInputs) -> String {
        let stored = self.family_surname(id).unwrap_or_default();
        inputs
            .family_names
            .get(&id)
            .filter(|read| read.trim() == stored)
            .map_or_else(|| stored.to_owned(), Clone::clone)
    }

    /// Saves this neighborhood into `directory` as 0x4252cc does: the file
    /// is written as `NTemp.iff`, then moved onto `Neighborhood.iff`.
    ///
    /// When its status is still clear after the close (0x425474 tests the
    /// status the save kept, `[ebp-0x10]`), the original deletes the old
    /// `Neighborhood.iff` (0x425481 → 0x46b42a; "not found", -0x28, is no
    /// error) and moves the new file onto its name (0x4254a2 → 0x5b9988).
    /// Otherwise it deletes `NTemp.iff` (0x4254bd). The port renames over
    /// the old file in one step, which leaves the same files and never
    /// leaves the folder without one; a `Neighborhood.iff` of another case
    /// is replaced under its own name. A `NTemp.iff` left by an earlier
    /// save is deleted first and the file is created anew, so a link
    /// under that name is never written through. If the write or the
    /// rename fails, `NTemp.iff` is deleted.
    ///
    /// Like the original, one save at a time writes a folder: two at once
    /// share `NTemp.iff`.
    pub fn save_neighborhood_iff(&self, directory: &Path, inputs: &SaveInputs) -> Result<PathBuf> {
        let bytes = self.neighborhood_iff(inputs)?;
        let temporary = directory.join(TEMPORARY_FILE);
        let target =
            neighborhood_iff_path(directory).unwrap_or_else(|| directory.join("Neighborhood.iff"));
        match std::fs::remove_file(&temporary) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(Error::Store(format!("{}: {error}", temporary.display())));
            }
        }
        let saved = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .and_then(|mut file| {
                std::io::Write::write_all(&mut file, &bytes)?;
                file.sync_all()
            })
            .map_err(|error| format!("{}: {error}", temporary.display()))
            .and_then(|()| {
                std::fs::rename(&temporary, &target)
                    .map_err(|error| format!("{}: {error}", target.display()))
            });
        if let Err(error) = saved {
            let _ = std::fs::remove_file(&temporary);
            return Err(Error::Store(error));
        }
        Ok(target)
    }
}
