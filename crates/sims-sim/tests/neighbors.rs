//! The neighborhood's records, as `World::read_neighbors` builds them
//! from a neighborhood file. Synthetic data only.
//!
//! What the reader (0x42feb2) and the reload (0x424806, 0x425209,
//! 0x425244) do is ported from the original's code and the old
//! prototype's `prepare_family_reload` and `rebuild_family_links`
//! (legacy/crates/sims-vm/src/neighborhood.rs). These tests check the
//! port's rules; the gated resize tests compare its result with the
//! capture of the original.

use sims_content::Neighborhood;
use sims_formats::neighborhood::{Family, Inventory, InventoryItem, Neighbor, Relationship};
use sims_sim::World;
use sims_sim::load::LoadError;
use sims_sim::neighbors::{FAMILY, PERSON_WORDS};
use std::collections::BTreeMap;
use std::path::PathBuf;

const ANNE: u32 = 0xa;
const BOB: u32 = 0xb;
const CARL: u32 = 0xc;

fn saved(id: i16, guid: u32) -> Neighbor {
    Neighbor {
        version: 0,
        version_extra: None,
        name: String::new(),
        unknown_zero: 0,
        person_mode: 9,
        person_data: (0..88).map(|n| n + 100).collect(),
        id,
        guid,
        unknown_negative: -1,
        relationships: vec![Relationship {
            target: 2,
            values: vec![7, 8],
        }],
    }
}

fn family(number: i32, members: &[u32]) -> Family {
    Family {
        version: 0,
        house: 0,
        number,
        budget: 0,
        architecture_value: 0,
        friends: 0,
        flags: 0,
        members: members.to_vec(),
        tail: Vec::new(),
    }
}

/// Anne, an empty slot, Bob, Carl, and a second record of Anne's type.
fn neighborhood() -> Neighborhood {
    Neighborhood {
        directory: PathBuf::new(),
        families: BTreeMap::new(),
        type_attributes: None,
        neighbors: vec![
            Some(saved(1, ANNE)),
            None,
            Some(saved(3, BOB)),
            Some(saved(4, CARL)),
            Some(saved(5, ANNE)),
        ],
        inventories: vec![Some(Inventory {
            neighbor: 3,
            items: vec![InventoryItem {
                kind: 2,
                guid: 0x77,
                count: 4,
            }],
        })],
    }
}

fn read(neighborhood: &Neighborhood, characters: &[u32]) -> Result<World, LoadError> {
    let mut world = World::empty();
    world.read_neighbors(neighborhood, characters.iter().copied())?;
    Ok(world)
}

fn families(world: &World) -> Vec<Option<i16>> {
    world
        .neighbors
        .slots
        .iter()
        .map(|s| s.as_ref().map(|n| n.data[FAMILY]))
        .collect()
}

#[test]
fn records_keep_their_slots_and_are_left_as_the_reader_leaves_them() {
    let world = read(&neighborhood(), &[ANNE, BOB]).unwrap();
    let neighbors = &world.neighbors;
    assert_eq!(neighbors.slots.len(), 5);
    assert!(neighbors.slots[1].is_none());
    let bob = neighbors.get(3).unwrap();
    assert_eq!((bob.id, bob.guid, bob.revision), (3, BOB, 9));
    assert_eq!(bob.data.len(), PERSON_WORDS);
    assert_eq!(bob.data[87], 187);
    assert_eq!(bob.data[88], 0, "words the file did not save are 0");
    assert_eq!((bob.friends, bob.stale), (0, true));
    assert_eq!(bob.row(2).unwrap().values, [7, 8]);
    assert!(!neighbors.relationships_changed);
    assert_eq!(neighbors.inventories.len(), 1);
    assert_eq!(neighbors.inventories[0].neighbor, 3);
    assert_eq!(neighbors.inventories[0].items[0].count, 4);
    // A script names a record by its place plus one.
    assert_eq!(neighbors.get(2), None);
    assert_eq!(neighbors.next(0), 1);
    assert_eq!(neighbors.next(1), 3);
    assert_eq!(neighbors.next(4), 5);
    assert_eq!(neighbors.next(5), 0);
    assert_eq!(neighbors.first_of(ANNE).unwrap().id, 1);
}

#[test]
fn the_family_word_is_rebuilt_in_family_order() {
    let mut file = neighborhood();
    // In order: family 3 (number 5), family 2, family 1 (both number 2),
    // then the added family 0. Each writes the first record of each
    // member's type, so the last family to name a person wins.
    file.families = BTreeMap::from([
        (1, family(2, &[ANNE, 0x99])),
        (2, family(2, &[ANNE, BOB])),
        (3, family(5, &[BOB])),
    ]);
    let world = read(&file, &[]).unwrap();
    // Anne's second record and Carl, in no family, are cleared from the
    // saved value; a member no record has is passed over.
    assert_eq!(families(&world), [Some(1), None, Some(2), Some(0), Some(0)]);
}

#[test]
fn a_person_type_without_a_record_is_not_ported() {
    let Err(LoadError::Neighborhood(text)) = read(&neighborhood(), &[ANNE, 0x99]) else {
        panic!("expected a neighborhood error");
    };
    assert!(text.contains("0x00000099"), "{text}");
}

#[test]
fn an_empty_inventory_slot_fails() {
    let mut file = neighborhood();
    file.inventories.push(None);
    assert!(matches!(read(&file, &[]), Err(LoadError::Neighborhood(_))));
}
