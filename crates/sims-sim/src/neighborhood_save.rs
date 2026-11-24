//! A stored neighborhood saved as the original's `Neighborhood.iff`
//! (`Sims.exe` 1.91 0x4252cc).
//!
//! [`sims_formats::neighborhood_file`] writes the file; this module builds
//! what it writes from a [`StoredNeighborhood`]. The save serializes the
//! neighborhood as it stands, so a lot's writeback
//! ([`StoredNeighborhood::write_back_world`]) comes first, as the save's
//! own updates of data word 2 (+0x12c) and the current family's
//! architecture value (+0x11c) come before its chunks. The store does not keep two things the
//! file has: each family's FAMh, which the save copies as it is, and the
//! bytes the save leaves uninitialized. [`SaveInputs`] carries them, with
//! the NGBH option bytes as read (the store keeps only their truth).
//!
//! Family 0 is written without a member list (0x4105e7). The import reads
//! it that way since this change; a store imported before it holds family
//! 0 read with one, which the save drops, so such a store is re-imported.

use crate::neighborhood_store::StoredNeighborhood;
use crate::{Error, Result};
use sims_formats::neighborhood::{Neighborhood, Neighbors};
use sims_formats::neighborhood_file::{FamilyRecord, NeighborhoodFile, Uninitialized};
use sims_formats::tatt::Tatt;
use std::collections::BTreeMap;

/// What the save writes that the store does not keep.
#[derive(Debug, Clone, Default)]
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
}

impl SaveInputs {
    /// The inputs a file the save wrote holds, so that saving the
    /// neighborhood read from it gives the same bytes.
    pub fn from_saved(bytes: &[u8]) -> Result<Self> {
        let (file, uninitialized) = NeighborhoodFile::parse(bytes)
            .map_err(|error| Error::Store(format!("Neighborhood.iff: {error}")))?;
        let option_bytes = file.neighborhood.option_bytes;
        let histories = file
            .families
            .into_iter()
            .filter_map(|record| Some((record.id, record.history?)))
            .collect();
        Ok(Self {
            histories,
            uninitialized,
            option_bytes,
        })
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
                    name: self.family_surname(id).unwrap_or_default().to_owned(),
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
}
