//! The neighborhood's people, as the game holds them while a lot runs.
//!
//! The original keeps one record per neighbor in the neighborhood object
//! (`[[0x6e48bc]+0x24]`), in a vector of record pointers at +0x148. It
//! builds them when it reads the neighborhood file (NBRS, read by
//! 0x430ad6, each record by 0x423e08) and rebuilds part of them each time it reloads the
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
use std::collections::{BTreeMap, BTreeSet};

/// How many person data words a record holds (+0x74, 256 words).
pub const PERSON_WORDS: usize = 256;

/// Person data word 61: the family the person belongs to, by the
/// family's FAMI chunk ID.
pub const FAMILY: usize = 61;

/// Person data word 65: the person's kind. A record whose kind is above
/// 1 is never counted as a family's friend (0x427c69).
pub const KIND: usize = 65;

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
    /// people sets (0x44485b). The end of the object read sets it too
    /// ([`crate::World::finish_object_read`], 0x43fc89), and the end of a
    /// full lot update clears it on a tick count that is a multiple of 40
    /// (0x4424cb). Nothing else is known to read it but that clear.
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

/// One family of the neighborhood, as far as the simulation reads it.
///
/// The original keeps the families in the neighborhood object
/// (`[[0x6e48bc]+0x24]`), in a vector of record pointers at +0x154, in
/// the order the reload sorts them ([`World::read_neighbors`]). A record
/// holds much more than this; these are the parts a lot's scripts reach,
/// through scope 24 ("neighbor in stack object", 0x425b5d).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Family {
    /// The family's ID: its FAMI chunk ID (+0x10c). A record's person
    /// data word 61 names a family by it.
    pub id: i32,
    /// The house the family lives in, 0 for none (+0x110).
    pub house: i32,
    /// The GUIDs of its members' person types, in order (+0x12c).
    pub members: Vec<u32>,
    /// The cached count of the family's friends (+0x120): what the file
    /// saved, until the count is worked out again
    /// ([`World::family_friends`]).
    pub friends: i32,
    /// Whether that count is out of date (+0x124). The family's
    /// constructor (0x41003c) sets it and the file's reader (0x4104ce)
    /// leaves it, so every family starts out of date.
    pub stale: bool,
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
    /// A record as the reader (0x423e08) leaves it.
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

/// The OBJD types the reload makes a record for when no record links to
/// a definition of that type (0x424d58 to 0x424e3c): 2, a person, and
/// 0x200, 0x400, 0x800 and 0x1000.
pub const RECORDED_KINDS: [u16; 5] = [2, 0x200, 0x400, 0x800, 0x1000];

/// One registered object definition, as the reload walks them: the game's
/// definition registry (`[[0x6e48bc]+0x28]`) keeps every definition it
/// registers in a vector (+0x20, read by 0x43ac3a), duplicates included,
/// and a map from GUID to the last one registered (+0x125c, read by
/// 0x43ac10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registered {
    pub guid: u32,
    /// The OBJD type (OBJD +0x12).
    pub kind: u16,
    /// Whether the map gives this definition for its GUID: it is the last
    /// registered with that GUID.
    pub resolves: bool,
}

/// The registry as the reload sees it, from the catalog built from
/// `index`.
///
/// The catalog registers a neighborhood's characters where the index
/// mounts them, before `Downloads` ([`sims_content::Index::add_characters`]).
/// The reload reregisters them first (0x43c228, at 0x424a8b): it takes
/// the old characters' definitions out and registers the new ones, which
/// appends them to the vector and makes them the map's for their GUIDs. So
/// here the definitions from the `Characters` mount come after every
/// other, in their own order, and a GUID resolves to the last definition
/// of it in that order.
///
/// Checked against a capture of the original's registry in the base
/// neighborhood (the Newbie house's startup): the 463 definitions of
/// [`RECORDED_KINDS`] come in the same order. Its whole vector differs
/// from the catalog's in other places, which this does not use.
pub fn registry(index: &sims_content::Index, catalog: &sims_content::Catalog) -> Vec<Registered> {
    let character = |object: &&sims_content::Object| {
        index.mounts[index.resources[object.resource].mount].path == "Characters"
    };
    let ordered: Vec<&sims_content::Object> = catalog
        .objects
        .iter()
        .filter(|o| !character(o))
        .chain(catalog.objects.iter().filter(|o| character(o)))
        .collect();
    let mut last = BTreeMap::new();
    for (place, object) in ordered.iter().enumerate() {
        last.insert(object.guid, place);
    }
    ordered
        .iter()
        .enumerate()
        .map(|(place, object)| Registered {
            guid: object.guid,
            kind: object.kind,
            resolves: last[&object.guid] == place,
        })
        .collect()
}

impl World {
    /// Reads the neighborhood's records (NBRS, 0x430ad6), inventories
    /// (NGBH, 0x4254f6, which reads them with 0x42feb2) and data words
    /// ([`World::neighborhood_data`], NGBH), and does what the
    /// neighborhood's reload (0x424806) does to the records before a lot
    /// loads. `registry` is every
    /// definition the game has registered, in order ([`registry`]).
    ///
    /// An empty slot in the file's inventory vector fails: the original
    /// would hold a null pointer there, which its inventory lookup
    /// (0x42a8a1) does not expect.
    ///
    /// Reading a record links it to the definition its GUID resolves to
    /// (record +8, 0x423ef2), or to none. The reload:
    ///
    /// 1. deletes each record linked to no definition (0x424ad8, 0x4259a2),
    ///    which is **not ported**: such a record is an error here;
    /// 2. drops each family's members that no record has the GUID of;
    /// 3. orders the families by their number, then their chunk ID, both
    ///    descending (0x425209), with an empty family 0 of number -1
    ///    added when the file has none;
    /// 4. makes a record for each registered definition, in registration
    ///    order (the neighborhood's characters last, see [`registry`]), of a type in [`RECORDED_KINDS`] that no record is linked
    ///    to (0x424d40 to 0x424e82): a record is linked to the definition
    ///    its GUID resolves to, so a definition another of its GUID
    ///    replaced gets one too. The record (0x4257a5 with no slot asked
    ///    for, then 0x4237a1) goes in the last empty slot, or in a new
    ///    slot at the end when none is empty, and its ID is its slot's
    ///    place plus one. It has the definition's GUID, revision 0, every
    ///    person data word 0 and no relationship rows, and no inventory;
    /// 5. rebuilds person data word 61, the family (0x425244): every
    ///    record's is cleared, then each family in that order writes its
    ///    chunk ID into the first record of each member's GUID.
    ///
    /// The record's name (+0x28, from the definition by 0x457e0e) is
    /// presentation and is not kept. Its friend count and whether that is
    /// out of date (+0x20, +0x24) are left as the allocator returned them.
    /// The original can read them before a relationship write marks the
    /// count out of date: scope 24 field 5 (0x425c19, 0x427148) returns
    /// the count as it is when the flag byte is 0. Here they are 0 and
    /// out of date, as for a record read from the file, and that field is
    /// not ported (it fails loudly), so nothing reads them.
    ///
    /// The original reads the records only when the neighborhood file has
    /// NGBH #1 (0x424a90; without it, it makes every record).
    /// [`Neighborhood::open`] keeps no records then, so every record is
    /// made here too. Every shipped neighborhood has one.
    ///
    /// Steps 2, 3 and 5 are ported from the old prototype
    /// (legacy/crates/sims-vm/src/neighborhood.rs, `prepare_family_reload`
    /// and `rebuild_family_links`), steps 1 and 4 from the original's
    /// code. The families are kept, in that order, as far as [`Family`]
    /// holds them: each with its friend count as the file saved it and
    /// out of date.
    ///
    /// Checked against the original: from the install's neighborhood
    /// file, every record's word 61 equals the capture's at the Goth
    /// house's first tick. The capture's game read another neighborhood
    /// file, which differs from the install's in five records (47, 48,
    /// 301, 344 and 351); see `docs/decisions/`. So do the families'
    /// IDs, houses, members and order, and every friend count but the
    /// Goth family's, which the load counts again (load step 20). That
    /// neighborhood has a record for every registered definition, so step
    /// 4 makes none there; the records it makes for a neighborhood that
    /// lacks them are **unverified** against the original, the order of
    /// the registry most of all.
    pub fn read_neighbors(
        &mut self,
        neighborhood: &Neighborhood,
        registry: &[Registered],
    ) -> Result<(), LoadError> {
        let families = &neighborhood.families;
        self.neighborhood_data = neighborhood.data;
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
        let registered: BTreeSet<u32> = registry.iter().map(|d| d.guid).collect();
        for record in neighbors.slots.iter().flatten() {
            if !registered.contains(&record.guid) {
                return Err(LoadError::Neighborhood(format!(
                    "record {} has the person type 0x{:08X}, which no definition has; the \
                     reload deletes it (0x4259a2), which is not ported",
                    record.id, record.guid
                )));
            }
        }
        let first = first_records(&neighbors);
        // The added family 0 has number -1, no house, no members, and the
        // friend count its constructor gives (0).
        let mut ordered: Vec<(i32, Family)> = families
            .iter()
            .map(|(&id, f)| {
                let family = Family {
                    id: i32::from(id),
                    house: f.house,
                    members: f
                        .members
                        .iter()
                        .copied()
                        .filter(|guid| first.contains_key(guid))
                        .collect(),
                    friends: f.friends,
                    stale: true,
                };
                (f.number, family)
            })
            .collect();
        if !families.contains_key(&0) {
            let family = Family {
                id: 0,
                house: 0,
                members: Vec::new(),
                friends: 0,
                stale: true,
            };
            ordered.push((-1, family));
        }
        ordered.sort_unstable_by_key(|(number, family)| std::cmp::Reverse((*number, family.id)));
        // A record read from the file is linked to the definition its GUID
        // resolves to; a record made here, to the definition it was made
        // for, which is never another registered definition.
        for definition in registry {
            let linked = definition.resolves && first.contains_key(&definition.guid);
            if RECORDED_KINDS.contains(&definition.kind) && !linked {
                neighbors.make_record(definition.guid)?;
            }
        }
        let first = first_records(&neighbors);
        for record in neighbors.slots.iter_mut().flatten() {
            record.data[FAMILY] = 0;
        }
        for (_, family) in &ordered {
            let id = family.id;
            let word = i16::try_from(id).map_err(|_| {
                LoadError::Neighborhood(format!("family chunk ID {id} is not a word"))
            })?;
            for guid in &family.members {
                if let Some(&place) = first.get(guid)
                    && let Some(record) = &mut neighbors.slots[place]
                {
                    record.data[FAMILY] = word;
                }
            }
        }
        self.neighbors = neighbors.into();
        self.families = ordered.into_iter().map(|(_, family)| family).collect();
        Ok(())
    }

    /// What the house load (0x428041) does to the families right after
    /// the reload (0x428071 to 0x4280b1): each family other than family 0
    /// that has no members is taken out of its house and deleted
    /// (0x425efb). That is **not ported**: such a family is an error here.
    pub fn drop_empty_families(&self) -> Result<(), LoadError> {
        match self
            .families
            .iter()
            .find(|f| f.id != 0 && f.members.is_empty())
        {
            Some(family) => Err(LoadError::Neighborhood(format!(
                "family {} has no members; the house load deletes it (0x425efb), which is not \
                 ported",
                family.id
            ))),
            None => Ok(()),
        }
    }
}

/// The place of the first record of each GUID, in slot order.
fn first_records(neighbors: &Neighbors) -> BTreeMap<u32, usize> {
    let mut first = BTreeMap::new();
    for (place, record) in neighbors.slots.iter().enumerate() {
        if let Some(record) = record {
            first.entry(record.guid).or_insert(place);
        }
    }
    first
}

impl Neighbors {
    /// Makes a record of the person type `guid` (0x4257a5 asked for no
    /// slot, then 0x4237a1): in the last empty slot, or a new slot at the
    /// end when none is empty. Its ID is the slot's place plus one, and it
    /// is returned.
    pub fn make_record(&mut self, guid: u32) -> Result<i16, LoadError> {
        let place = match self.slots.iter().rposition(Option::is_none) {
            Some(place) => place,
            None => {
                self.slots.push(None);
                self.slots.len() - 1
            }
        };
        let id = i16::try_from(place + 1).map_err(|_| {
            LoadError::Neighborhood("the neighborhood holds more records than a word counts".into())
        })?;
        self.slots[place] = Some(Neighbor {
            id,
            guid,
            revision: 0,
            data: vec![0; PERSON_WORDS],
            relationships: Vec::new(),
            friends: 0,
            stale: true,
        });
        Ok(id)
    }

    /// Removes the neighbor record at slot `id - 1` (0x4259a2):
    ///
    /// 1. Looks up or appends the record's inventory (`0x42a8a1`) and clears
    ///    tokens of kinds `0..=9` (`0x41f4e5`), leaving the inventory entry
    ///    in [`Neighbors::inventories`].
    /// 2. Vacates slot `id - 1` in place (`slots.len()` stays unchanged,
    ///    `0x425a33`).
    /// 3. Erases the first relationship row targeting `id` from every
    ///    remaining neighbor record (`0x425a38` → `0x464fa2`).
    pub fn remove_record(&mut self, id: i16) {
        let inv = match self.inventories.iter_mut().find(|inv| inv.neighbor == id) {
            Some(inv) => inv,
            None => {
                self.inventories.push(Inventory {
                    neighbor: id,
                    items: Vec::new(),
                });
                self.inventories.last_mut().expect("just pushed")
            }
        };
        inv.items.retain(|item| !(0..=9).contains(&item.kind));
        if let Some(place) = usize::try_from(id).ok().and_then(|n| n.checked_sub(1))
            && let Some(slot) = self.slots.get_mut(place)
        {
            *slot = None;
        }
        let target = i32::from(id);
        for neighbor in self.slots.iter_mut().flatten() {
            if let Some(pos) = neighbor
                .relationships
                .iter()
                .position(|row| row.target == target)
            {
                neighbor.relationships.remove(pos);
            }
        }
    }
}

/// Whether two records are friends both ways (0x4271c5): each has a row
/// for the other with at least one value, and the first value of each
/// row is at least `threshold` (the autonomy tuning's friendship
/// threshold, 0x6c4398).
pub fn mutual_friends(a: &Neighbor, b: &Neighbor, threshold: i32) -> bool {
    let first = |from: &Neighbor, to: &Neighbor| {
        from.row(i32::from(to.id))
            .and_then(|row| row.values.first().copied())
    };
    match (first(a, b), first(b, a)) {
        (Some(ab), Some(ba)) => ab >= threshold && ba >= threshold,
        _ => false,
    }
}

impl World {
    /// The friend count of the family at `place` in [`World::families`]
    /// (0x427219): the cached count, worked out again first when it is
    /// out of date ([`World::count_family_friends`]). `threshold` is the
    /// autonomy tuning's friendship threshold (0x6c4398).
    ///
    /// The count is only worked out when it is read, and it goes out of
    /// date only when a relationship of one of the family's people is
    /// written ([`World::mark_friends`]); a change to anything else it
    /// reads leaves the cached count as it was, as in the original.
    pub fn family_friends(&mut self, place: usize, threshold: i32) -> Option<i32> {
        let family = self.families.get(place)?;
        if family.stale {
            let friends = self.count_family_friends(family, threshold);
            let family = &mut self.families[place];
            family.friends = friends;
            family.stale = false;
        }
        Some(self.families[place].friends)
    }

    /// Counts a family's friends (0x427bd7): the records, in slot order,
    /// that are not of a member's type, belong to some family (word 61),
    /// are of kind (word 65) at most 1 and are friends both ways
    /// ([`mutual_friends`]) with the first record of some member's type.
    /// Each record counts once.
    pub fn count_family_friends(&self, family: &Family, threshold: i32) -> i32 {
        let neighbors = &self.neighbors;
        let mut count = 0i32;
        for candidate in neighbors.slots.iter().flatten() {
            if family.members.contains(&candidate.guid) || candidate.data[FAMILY] == 0 {
                continue;
            }
            for &guid in &family.members {
                // The member's record (0x42571f): none for GUID 0.
                let Some(member) = neighbors.first_of(guid).filter(|_| guid != 0) else {
                    continue;
                };
                if mutual_friends(candidate, member, threshold) && candidate.data[KIND] <= 1 {
                    count = count.wrapping_add(1);
                    break;
                }
            }
        }
        count
    }

    /// The friend count of the record `id` names (0x427148, from scope 24
    /// field 5): while the record's count is out of date (+0x24) it is
    /// worked out again and kept (+0x20); the reader does not mark it up
    /// to date, so it is worked out again at every read until then. It
    /// counts the other records, in slot order, whose type is one of
    /// `characters` or a type made while the game ran
    /// ([`crate::characters`], whose file is in the Characters folder
    /// too, 0x43c3b2) (the record's type, +8, registered with kind 1:
    /// 0x423f30 reads bits 2 and 3 of its +0xb4; a record whose type has
    /// no definition has none, and does not count), whose word 61 (the
    /// family) is not 0, and that are friends both ways with it
    /// ([`mutual_friends`], `threshold` being the autonomy tuning's
    /// friendship threshold, 0x6c4398). None for no such record.
    pub fn record_friends(
        &mut self,
        id: i16,
        threshold: i32,
        characters: &BTreeSet<u32>,
    ) -> Option<i32> {
        let record = self.neighbors.get(id)?;
        if !record.stale {
            return Some(record.friends);
        }
        let place = usize::try_from(id).ok()?.checked_sub(1)?;
        let mut count = 0i32;
        let character =
            |guid: u32| characters.contains(&guid) || self.characters.made.contains_key(&guid);
        for (other_place, other) in self.neighbors.slots.iter().enumerate() {
            let Some(other) = other else { continue };
            if other_place == place || !character(other.guid) || other.data[FAMILY] == 0 {
                continue;
            }
            if mutual_friends(other, record, threshold) {
                count = count.wrapping_add(1);
            }
        }
        self.neighbors.get_mut(id)?.friends = count;
        Some(count)
    }

    /// What a write to a relationship marks out of date about one of its
    /// two records (0x4246b9): the record's own friend count, and the
    /// friend count of the first family whose ID is the record's word 61.
    /// Nothing for a record that does not exist.
    pub fn mark_friends(&mut self, id: i16) {
        let Some(record) = self.neighbors.get_mut(id) else {
            return;
        };
        record.stale = true;
        let family = i32::from(record.data[FAMILY]);
        if let Some(family) = self.families.iter_mut().find(|f| f.id == family) {
            family.stale = true;
        }
    }
}
