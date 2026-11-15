//! The neighborhood's records, as `World::read_neighbors` builds them
//! from a neighborhood file. Synthetic data only.
//!
//! What the reader (0x42feb2) and the reload (0x424806, 0x425209,
//! 0x425244, 0x4257a5) do is ported from the original's code and the old
//! prototype's `prepare_family_reload` and `rebuild_family_links`
//! (legacy/crates/sims-vm/src/neighborhood.rs). These tests check the
//! port's rules; the gated resize tests compare its result with the
//! capture of the original.

use sims_content::Neighborhood;
use sims_formats::neighborhood::{Family, Inventory, InventoryItem, Neighbor, Relationship};
use sims_sim::World;
use sims_sim::load::LoadError;
use sims_sim::neighbors::{FAMILY, PERSON_WORDS, Registered};
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
        data: [0, 7, -3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2],
    }
}

const DAVE: u32 = 0xd;
const EVE: u32 = 0xe;
const FRED: u32 = 0xf;
const GINA: u32 = 0x10;

/// A registered definition of type `kind` that its GUID resolves to.
fn registered(guid: u32, kind: u16) -> Registered {
    Registered {
        guid,
        kind,
        resolves: true,
    }
}

/// The people of [`neighborhood`], each registered once.
fn everyone() -> Vec<Registered> {
    [ANNE, BOB, CARL].map(|guid| registered(guid, 2)).to_vec()
}

fn read(neighborhood: &Neighborhood, registry: &[Registered]) -> Result<World, LoadError> {
    let mut world = World::empty();
    world.read_neighbors(neighborhood, registry)?;
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
    let world = read(&neighborhood(), &everyone()).unwrap();
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
    assert_eq!(
        world.neighborhood_data,
        [0, 7, -3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2],
        "the data words are the file's"
    );
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
    let world = read(&file, &everyone()).unwrap();
    // Anne's second record and Carl, in no family, are cleared from the
    // saved value; a member no record has is passed over.
    assert_eq!(families(&world), [Some(1), None, Some(2), Some(0), Some(0)]);
}

#[test]
fn the_families_are_kept_in_family_order_with_their_counts_out_of_date() {
    let mut file = neighborhood();
    let mut housed = family(2, &[ANNE, 0x99, BOB]);
    (housed.house, housed.friends) = (12, 4);
    file.families = BTreeMap::from([(1, family(2, &[CARL])), (2, housed), (3, family(5, &[]))]);
    let world = read(&file, &everyone()).unwrap();
    let kept: Vec<(i32, i32, &[u32], i32, bool)> = world
        .families
        .iter()
        .map(|f| (f.id, f.house, f.members.as_slice(), f.friends, f.stale))
        .collect();
    // A member no record has is dropped; the added family 0 comes last.
    assert_eq!(
        kept,
        [
            (3, 0, &[][..], 0, true),
            (2, 12, &[ANNE, BOB][..], 4, true),
            (1, 0, &[CARL][..], 0, true),
            (0, 0, &[][..], 0, true),
        ]
    );
    // A file with a family 0 has no other.
    file.families.insert(0, family(-3, &[BOB]));
    let world = read(&file, &everyone()).unwrap();
    let ids: Vec<i32> = world.families.iter().map(|f| f.id).collect();
    assert_eq!(ids, [3, 2, 1, 0]);
    assert_eq!(world.families[3].members, [BOB]);
}

#[test]
fn a_family_counts_each_friend_of_a_member_once() {
    use sims_sim::neighbors::{KIND, Row};
    let mut file = neighborhood();
    file.families = BTreeMap::from([(2, family(1, &[ANNE, BOB]))]);
    let mut world = read(&file, &everyone()).unwrap();
    let like = |world: &mut World, from: i16, to: i32, value: i32| {
        let record = world.neighbors.get_mut(from).unwrap();
        record.relationships = vec![Row {
            target: to,
            values: vec![value],
        }];
    };
    // Carl (4) and Anne (1) like each other 30: Carl, in family 0 by his
    // word 61, is not counted.
    like(&mut world, 4, 1, 30);
    like(&mut world, 1, 4, 30);
    world.neighbors.get_mut(4).unwrap().data[KIND] = 1;
    let place = 0;
    assert_eq!(world.families[place].id, 2);
    assert_eq!(world.family_friends(place, 30), Some(0));
    // In a family of his own he is, but only up to the threshold.
    world.neighbors.get_mut(4).unwrap().data[FAMILY] = 9;
    world.mark_friends(1);
    assert!(world.families[place].stale);
    assert_eq!(world.family_friends(place, 31), Some(0));
    world.mark_friends(4);
    assert!(!world.families[place].stale, "Carl's family 9 is not kept");
    world.mark_friends(1);
    assert_eq!(world.family_friends(place, 30), Some(1));
    assert_eq!(world.family_friends(4, 30), None);
}

#[test]
fn each_type_without_a_record_gets_one_in_the_last_empty_slot() {
    let mut file = neighborhood();
    // Slots: Anne, empty, Bob, Carl, Anne's second record, empty.
    file.neighbors.push(None);
    // Dave is a member, but his record is made after the members are
    // checked, so he is dropped from the family.
    file.families = BTreeMap::from([(1, family(1, &[BOB, DAVE]))]);
    let mut registry = everyone();
    registry.extend([
        registered(DAVE, 2),
        registered(FRED, 1),
        registered(EVE, 0x400),
        registered(GINA, 0x1000),
    ]);
    let world = read(&file, &registry).unwrap();
    let slots: Vec<Option<(i16, u32)>> = world
        .neighbors
        .slots
        .iter()
        .map(|s| s.as_ref().map(|n| (n.id, n.guid)))
        .collect();
    // Dave takes the last empty slot, Eve the one left, Gina a new one;
    // Fred, an object of type 1, gets none.
    assert_eq!(
        slots,
        [
            Some((1, ANNE)),
            Some((2, EVE)),
            Some((3, BOB)),
            Some((4, CARL)),
            Some((5, ANNE)),
            Some((6, DAVE)),
            Some((7, GINA)),
        ]
    );
    let dave = world.neighbors.get(6).unwrap();
    assert_eq!(dave.revision, 0);
    assert_eq!(dave.data, vec![0; PERSON_WORDS]);
    assert!(dave.relationships.is_empty());
    assert_eq!((dave.friends, dave.stale), (0, true));
    assert_eq!(world.families[0].members, [BOB]);
    assert_eq!(
        families(&world)[2..],
        [Some(1), Some(0), Some(0), Some(0), Some(0)]
    );
    assert_eq!(world.neighbors.inventories.len(), 1, "no inventory is made");
}

#[test]
fn a_definition_another_replaced_gets_a_record_of_its_own() {
    let mut registry = everyone();
    registry.insert(
        0,
        Registered {
            guid: BOB,
            kind: 2,
            resolves: false,
        },
    );
    let world = read(&neighborhood(), &registry).unwrap();
    // Bob's records are linked to the definition that replaced the first,
    // so the first gets one: the only empty slot.
    let made = world.neighbors.get(2).unwrap();
    assert_eq!((made.id, made.guid), (2, BOB));
    assert_eq!(world.neighbors.slots.len(), 5);
    // The family word goes to the first record of Bob's type, in slot
    // order, which is now the one just made.
    let mut file = neighborhood();
    file.families = BTreeMap::from([(1, family(1, &[BOB]))]);
    let world = read(&file, &registry).unwrap();
    assert_eq!(families(&world)[1..3], [Some(1), Some(0)]);
}

#[test]
fn a_record_whose_type_is_not_registered_is_not_ported() {
    let Err(LoadError::Neighborhood(text)) = read(&neighborhood(), &everyone()[..2]) else {
        panic!("expected a neighborhood error");
    };
    assert!(
        text.contains("0x0000000C") && text.contains("0x4259a2"),
        "{text}"
    );
}

#[test]
fn a_family_without_members_is_not_ported() {
    let mut file = neighborhood();
    // Family 2's only member has no record, so the reload drops him.
    file.families = BTreeMap::from([(1, family(1, &[ANNE])), (2, family(2, &[0x99]))]);
    let world = read(&file, &everyone()).unwrap();
    let Err(LoadError::Neighborhood(text)) = world.drop_empty_families() else {
        panic!("expected a neighborhood error");
    };
    assert!(
        text.contains("family 2") && text.contains("0x425efb"),
        "{text}"
    );
    // Family 0, which the reload adds empty, stays.
    file.families.remove(&2);
    let world = read(&file, &everyone()).unwrap();
    assert!(world.drop_empty_families().is_ok());
}

#[test]
fn an_empty_inventory_slot_fails() {
    let mut file = neighborhood();
    file.inventories.push(None);
    assert!(matches!(
        read(&file, &everyone()),
        Err(LoadError::Neighborhood(_))
    ));
}
