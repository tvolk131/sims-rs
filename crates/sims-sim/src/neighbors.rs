//! The neighborhood's people, as the game holds them while a lot runs.
//!
//! The original keeps one record per neighbor in the neighborhood object
//! (`[[0x6e48bc]+0x24]`), in a vector of record pointers at +0x148. It
//! builds them when it reads the neighborhood file (NBRS, read by
//! 0x42feb2) and rebuilds part of them each time it reloads the
//! neighborhood (0x424806), before any lot loads. A lot's scripts read
//! and write them: Set To Next walks them (modes 5 and 7), an expression
//! reads a record's person data (scope 32), and Relationship keeps the
//! rows between people in them.
//!
//! A capture of the original's Goth house load (the old prototype's
//! harness, stopped at the resize, both clears, the object read and the
//! post-load) shows the records unchanged from before the lot load to the
//! first tick, so they are state the lot load does not touch.

use crate::World;
use crate::load::LoadError;
use serde::{Deserialize, Serialize};
use sims_content::Neighborhood;
use sims_formats::neighborhood::Neighbor as Saved;
use std::collections::BTreeMap;

/// How many person data words a record holds (+0x74, 256 words).
pub const PERSON_WORDS: usize = 256;

/// Person data word 61: the family the person belongs to, by the
/// family's FAMI chunk ID.
pub const FAMILY: usize = 61;

/// The neighborhood's records and the one registry byte their
/// relationships mark.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Neighbors {
    /// The record vector (neighborhood +0x148), in order. A script names a
    /// record by its place plus one (0x4256f4); that is not the record's
    /// own ID, though in the shipped neighborhoods the two are equal.
    pub slots: Vec<Option<Neighbor>>,
    /// The neighbors' inventories (neighborhood +0x194), in order. A
    /// neighbor has at most one; Manage Inventory makes one for a neighbor
    /// that has none (0x42a8a1).
    pub inventories: Vec<Inventory>,
    /// Registry byte 0x12, which a write to a relationship between
    /// people sets (0x44485b).
    ///
    /// **Unverified** where it starts: the capture has it 0 at the resize
    /// and 1 after the object read, which is not ported, so it is false
    /// here until a script writes a relationship.
    pub relationships_changed: bool,
}

/// One neighbor's record (628 bytes in the original).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Neighbor {
    /// The record's ID (+0). A relationship row names its target by it.
    pub id: i16,
    /// The GUID of the person's type, as saved (+4).
    pub guid: u32,
    /// The revision of the person data (+0x18). Below 1 the file saved
    /// none.
    pub revision: i32,
    /// The person data, [`PERSON_WORDS`] words (+0x74). Words the file
    /// did not save are 0.
    pub data: Vec<i16>,
    /// The relationship rows (+0xc), in the order they were made: each
    /// target's ID and its values.
    pub relationships: Vec<Row>,
    /// The cached count of friends (+0x20); the reader sets 0.
    pub friends: i32,
    /// Whether that count is out of date (+0x24); the reader sets it.
    pub stale: bool,
}

/// A relationship row of a record: 16 bytes in the original, a vector of
/// dwords and the target's ID at +12.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub target: i32,
    pub values: Vec<i32>,
}

/// A neighbor's inventory (16 bytes in the original).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    /// The neighbor's record ID.
    pub neighbor: i16,
    /// The tokens, in order (12 bytes each).
    pub items: Vec<Item>,
}

/// One token kind of an inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub kind: i32,
    pub guid: u32,
    pub count: u16,
}

impl Neighbor {
    /// A record as the reader (0x42feb2) leaves it.
    pub fn read(saved: &Saved) -> Self {
        let mut data = vec![0; PERSON_WORDS];
        let saved_words = saved.person_data.len().min(PERSON_WORDS);
        data[..saved_words].copy_from_slice(&saved.person_data[..saved_words]);
        Self {
            id: saved.id,
            guid: saved.guid,
            revision: saved.person_mode,
            data,
            relationships: saved
                .relationships
                .iter()
                .map(|r| Row {
                    target: r.target,
                    values: r.values.clone(),
                })
                .collect(),
            friends: 0,
            stale: true,
        }
    }

    /// The row whose target is `target`: the first, as the original
    /// searches them in order (0x464f74).
    pub fn row(&self, target: i32) -> Option<&Row> {
        self.relationships.iter().find(|r| r.target == target)
    }

    pub fn row_mut(&mut self, target: i32) -> Option<&mut Row> {
        self.relationships.iter_mut().find(|r| r.target == target)
    }
}

impl Neighbors {
    /// The record a script names by `id`: slot `id - 1` (0x4256f4). None
    /// for an ID outside the slots and for an empty slot.
    pub fn get(&self, id: i16) -> Option<&Neighbor> {
        let place = usize::try_from(id).ok()?.checked_sub(1)?;
        self.slots.get(place)?.as_ref()
    }

    pub fn get_mut(&mut self, id: i16) -> Option<&mut Neighbor> {
        let place = usize::try_from(id).ok()?.checked_sub(1)?;
        self.slots.get_mut(place)?.as_mut()
    }

    /// The ID of the first occupied slot after the one `cursor` names
    /// (0x425b15): 0 when there is none. A cursor that is negative, or not
    /// below the number of slots, gives 0, as the original compares it
    /// unsigned.
    pub fn next(&self, cursor: i16) -> i16 {
        let Ok(start) = usize::try_from(cursor) else {
            return 0;
        };
        if start >= self.slots.len() {
            return 0;
        }
        self.slots
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, slot)| slot.is_some())
            .and_then(|(place, _)| i16::try_from(place + 1).ok())
            .unwrap_or(0)
    }

    /// The first record of a person type, in slot order: the record the
    /// original gives a person of that type.
    pub fn first_of(&self, guid: u32) -> Option<&Neighbor> {
        self.slots.iter().flatten().find(|n| n.guid == guid)
    }
}

impl World {
    /// Reads the neighborhood's records (NBRS, 0x42feb2) and inventories
    /// (NGBH, 0x4254f6), and does what the neighborhood's reload (0x424806)
    /// does to the records before a lot loads. `characters` are the GUIDs
    /// of the person types the game has registered.
    ///
    /// An empty slot in the file's inventory vector fails: the original
    /// would hold a null pointer there, which its inventory lookup
    /// (0x42a8a1) does not expect.
    ///
    /// The reload:
    ///
    /// 1. drops each family's members that no record has the GUID of;
    /// 2. orders the families by their number, then their chunk ID, both
    ///    descending (0x425209), with an empty family 0 of number -1
    ///    added when the file has none;
    /// 3. makes a record for each person type that has none, which is
    ///    **not ported**: a type without a record is an error here;
    /// 4. rebuilds person data word 61, the family (0x425244): every
    ///    record's is cleared, then each family in that order writes its
    ///    chunk ID into the first record of each member's GUID.
    ///
    /// Ported from the old prototype (legacy/crates/sims-vm/src/neighborhood.rs,
    /// `prepare_family_reload` and `rebuild_family_links`). The families
    /// themselves are not kept: nothing a lot has been seen to run reads
    /// them yet.
    ///
    /// Checked against the original: from the install's neighborhood
    /// file, every record's word 61 equals the capture's at the Goth
    /// house's first tick. The capture's game read another neighborhood
    /// file, which differs from the install's in five records (47, 48,
    /// 301, 344 and 351); see `docs/decisions.md`.
    pub fn read_neighbors(
        &mut self,
        neighborhood: &Neighborhood,
        characters: impl IntoIterator<Item = u32>,
    ) -> Result<(), LoadError> {
        let families = &neighborhood.families;
        let mut inventories = Vec::new();
        for inventory in &neighborhood.inventories {
            let inventory = inventory.as_ref().ok_or_else(|| {
                LoadError::Neighborhood("the file's inventory vector has an empty slot".into())
            })?;
            inventories.push(Inventory {
                neighbor: inventory.neighbor,
                items: inventory
                    .items
                    .iter()
                    .map(|i| Item {
                        kind: i.kind,
                        guid: i.guid,
                        count: i.count,
                    })
                    .collect(),
            });
        }
        let mut neighbors = Neighbors {
            slots: neighborhood
                .neighbors
                .iter()
                .map(|s| s.as_ref().map(Neighbor::read))
                .collect(),
            inventories,
            relationships_changed: false,
        };
        let mut first = BTreeMap::new();
        for (place, record) in neighbors.slots.iter().enumerate() {
            if let Some(record) = record {
                first.entry(record.guid).or_insert(place);
            }
        }
        for guid in characters {
            if !first.contains_key(&guid) {
                return Err(LoadError::Neighborhood(format!(
                    "the person type 0x{guid:08X} has no neighborhood record; the reload makes \
                     one (0x424806), which is not ported"
                )));
            }
        }
        let mut ordered: Vec<(i32, i32, &[u32])> = families
            .iter()
            .map(|(&id, f)| (f.number, i32::from(id), f.members.as_slice()))
            .collect();
        if !families.contains_key(&0) {
            ordered.push((-1, 0, &[]));
        }
        ordered.sort_unstable_by_key(|&(number, id, _)| std::cmp::Reverse((number, id)));
        for record in neighbors.slots.iter_mut().flatten() {
            record.data[FAMILY] = 0;
        }
        for (_, id, members) in ordered {
            let word = i16::try_from(id).map_err(|_| {
                LoadError::Neighborhood(format!("family chunk ID {id} is not a word"))
            })?;
            for guid in members {
                if let Some(&place) = first.get(guid)
                    && let Some(record) = &mut neighbors.slots[place]
                {
                    record.data[FAMILY] = word;
                }
            }
        }
        self.neighbors = neighbors;
        Ok(())
    }
}
