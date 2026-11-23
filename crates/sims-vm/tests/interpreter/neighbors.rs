//! What a lot's scripts do with the neighborhood's records, and the
//! person primitives the Goth house's second tick reaches: an
//! expression's neighbor person data (scope 32) and neighbor fields
//! (scope 24, with a family's friend count), Relationship between
//! records, Manage Inventory, Idle For Input, Get Distance To and Get
//! Direction To.
//!
//! Each primitive's doc comment cites the original's code it was ported
//! from. These tests check the port's rules on synthetic data only.

use crate::support::*;
use sims_sim::neighbors::{FAMILY, Family, Inventory, Item, KIND, Neighbor, Row};
use sims_vm::{Interpreter, Status, Trap};

const BUDGET: usize = 100;

/// A record whose person data counts up from 1000.
fn record(id: i16, guid: u32) -> Neighbor {
    Neighbor {
        id,
        guid,
        revision: 5,
        data: (0..256).map(|n| 1000 + n).collect(),
        relationships: Vec::new(),
        friends: 0,
        stale: false,
    }
}

/// Runs one instruction as the Sim's tree 4096.
fn sim_runs(
    fx: &mut Fixture,
    opcode: u16,
    operands: [u8; 8],
    stack_object: i16,
) -> Result<bool, sims_vm::Fault> {
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![ins(opcode, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(SIM, 4096, stack_object, &[], BUDGET)
}

#[test]
fn an_expression_reads_the_person_data_of_the_stack_objects_record() {
    let mut fx = Fixture::new();
    // The lamp's tree has the Sim (2) as stack object: it names the
    // record in slot 2, whatever that record's own ID.
    fx.world.neighbors.slots = vec![None, Some(record(40, SIM_GUID))];
    assert_eq!(
        fx.expression(32, 5, op::EQUAL, scope::LITERAL, 1005),
        Ok(true)
    );
    assert_eq!(
        code(fx.expression(32, 256, op::EQUAL, scope::LITERAL, 0)),
        Some(0x21)
    );
    assert_eq!(
        code(fx.expression(32, -1, op::EQUAL, scope::LITERAL, 0)),
        Some(0x21)
    );
    fx.world.neighbors.slots.truncate(1);
    assert_eq!(
        code(fx.expression(32, 5, op::EQUAL, scope::LITERAL, 0)),
        Some(0x37)
    );
}

/// A family of the neighborhood, with its count out of date.
fn family(id: i32, house: i32, members: &[u32]) -> Family {
    Family {
        id,
        house,
        members: members.to_vec(),
        friends: 0,
        stale: true,
    }
}

/// A record of family `family` whose word 65 is 0.
fn member(id: i16, guid: u32, family: i16) -> Neighbor {
    let mut record = record(id, guid);
    record.data[FAMILY] = family;
    record.data[KIND] = 0;
    record
}

/// Gives record `from` a row for record `to` whose first value is
/// `value`.
fn like(fx: &mut Fixture, from: i16, to: i16, value: i32) {
    let target = i32::from(fx.world.neighbors.get(to).unwrap().id);
    let record = fx.world.neighbors.get_mut(from).unwrap();
    record.relationships.retain(|r| r.target != target);
    record.relationships.push(Row {
        target,
        values: vec![value, -100],
    });
}

const ANNE: u32 = 0xa;
const BOB: u32 = 0xb;

/// The family of Anne (record 1) and Bob (record 2, which the lamp's
/// stack object, the Sim, names), with their friends. The friendship
/// threshold is the default, 25.
///
/// - 3 likes Anne and Anne likes 3 (25 and 30): a friend.
/// - 4 is the same but in no family (word 61 is 0): not counted.
/// - 5 is the same but its word 65 is 2: not counted.
/// - 6 and Anne like each other 30 and 24, but 6 and Bob 26 and 40: a
///   friend, through Bob.
/// - 7 is friends with both: counted once.
/// - 8 is a second record of Anne's type, friends with Bob: a member's
///   type, not counted.
/// - 9 has a row for Anne with no values, and Anne one for 9: not
///   counted.
/// - 10 likes Anne 90 and Anne likes 10 only 24; 11 likes Bob 24 and Bob
///   likes 11 90; 12 likes Anne 90 and Anne has no row for 12: none is
///   counted.
fn friends() -> Fixture {
    let mut fx = Fixture::new();
    let mut slots = vec![Some(member(1, ANNE, 5)), Some(member(2, BOB, 5))];
    slots.extend((3..=12).map(|id| Some(member(id, 0x100 + id as u32, 7))));
    fx.world.neighbors.slots = slots;
    let set = |fx: &mut Fixture, id: i16, word: usize, value: i16| {
        fx.world.neighbors.get_mut(id).unwrap().data[word] = value;
    };
    set(&mut fx, 4, FAMILY, 0);
    set(&mut fx, 5, KIND, 2);
    fx.world.neighbors.get_mut(8).unwrap().guid = ANNE;
    for (a, b, ab, ba) in [
        (3, 1, 25, 30),
        (4, 1, 90, 90),
        (5, 1, 90, 90),
        (6, 1, 30, 24),
        (6, 2, 26, 40),
        (7, 1, 50, 50),
        (7, 2, 50, 50),
        (8, 2, 90, 90),
        (10, 1, 90, 24),
        (11, 2, 24, 90),
    ] {
        like(&mut fx, a, b, ab);
        like(&mut fx, b, a, ba);
    }
    like(&mut fx, 12, 1, 90);
    fx.world
        .neighbors
        .get_mut(9)
        .unwrap()
        .relationships
        .push(Row {
            target: 1,
            values: Vec::new(),
        });
    like(&mut fx, 1, 9, 90);
    fx.world.families = vec![family(7, 0, &[0x107]), family(5, 12, &[ANNE, BOB])];
    fx
}

/// Field `index` of the record the lamp's stack object names, read into
/// the lamp's temp 0.
fn field(fx: &mut Fixture, index: i16) -> i16 {
    fx.expression(scope::TEMPS, 0, op::ASSIGN, 24, index)
        .unwrap_or_else(|f| panic!("{f}"));
    fx.lamp().temps[0]
}

#[test]
fn scope_24_field_9_counts_the_records_family_friends_and_keeps_the_count() {
    let mut fx = friends();
    assert_eq!(field(&mut fx, 9), 3);
    let goths = &fx.world.families[1];
    assert_eq!((goths.friends, goths.stale), (3, false));
    // The other family is left as it was.
    assert_eq!(
        (fx.world.families[0].friends, fx.world.families[0].stale),
        (0, true)
    );
    // A change the count reads that is not a relationship write leaves
    // the cached count: 3 leaves its family, and Anne's row for 3 drops.
    fx.world.neighbors.get_mut(3).unwrap().data[FAMILY] = 0;
    like(&mut fx, 1, 3, 0);
    assert_eq!(field(&mut fx, 9), 3);
    // A relationship write of one of the family's people puts it out of
    // date, and the next read counts again.
    fx.sim_mut().person.as_mut().unwrap().data[31] = 7;
    assert_eq!(
        sim_runs(&mut fx, 26, [0, 0, 6, 0, scope::LITERAL, 0, 50, 0], 2),
        Ok(true)
    );
    assert!(fx.world.families[1].stale);
    assert_eq!(field(&mut fx, 9), 2);
}

#[test]
fn scope_24_field_9_reads_the_friendship_threshold_truncated() {
    let mut fx = friends();
    let library = fx.program.libraries.get_mut(&GLOBAL).unwrap();
    library.tuning.insert(
        2,
        sims_formats::tuning::Constants {
            version: 1,
            entries: vec![sims_formats::tuning::Constant {
                name: "Friendship Threshold".into(),
                value: 25.9,
                comment: String::new(),
            }],
        },
    );
    assert_eq!(field(&mut fx, 9), 3, "25.9 is 25");
    let mut fx = friends();
    let library = fx.program.libraries.get_mut(&GLOBAL).unwrap();
    library.tuning.insert(
        2,
        sims_formats::tuning::Constants {
            version: 1,
            entries: vec![sims_formats::tuning::Constant {
                name: "friendship threshold".into(),
                value: 26.0,
                comment: String::new(),
            }],
        },
    );
    // 3 likes Anne only 25: 6 (through Bob) and 7 remain.
    assert_eq!(field(&mut fx, 9), 2);
}

#[test]
fn scope_24_field_9_reads_the_first_family_of_the_records_id() {
    // A family with no house is counted all the same (0x425cfa does not
    // read the house).
    let mut fx = friends();
    fx.world.families[1].house = 0;
    assert_eq!(field(&mut fx, 9), 3);
    assert!(!fx.world.families[1].stale);
    // The record's word 61 names no family.
    let mut fx = friends();
    fx.world.neighbors.get_mut(2).unwrap().data[FAMILY] = 6;
    assert_eq!(field(&mut fx, 9), 0);
    // Word 61 is 0, even with a family 0.
    let mut fx = friends();
    fx.world.families.push(family(0, 3, &[]));
    fx.world.neighbors.get_mut(2).unwrap().data[FAMILY] = 0;
    assert_eq!(field(&mut fx, 9), 0);
    assert!(fx.world.families[2].stale);
    // The first family of an ID is the one read, with or without a house.
    let mut fx = friends();
    fx.world.families.insert(0, family(5, 0, &[ANNE, BOB]));
    assert_eq!(field(&mut fx, 9), 3);
    assert!(!fx.world.families[0].stale);
    assert!(fx.world.families[2].stale);
}

#[test]
fn the_family_panel_caches_the_selected_familys_count_for_later_reads() {
    // The Goths (family 5) are the selected family on a residential lot.
    let mut fx = friends();
    let lot = fx.world.globals[10];
    fx.program.zoning.insert(lot, 1);
    fx.world.globals[9] = 5;
    let refresh = |fx: &mut Fixture| {
        Interpreter::new(&fx.program, &fx.catalog, fx.stream).refresh_family_panel(&mut fx.world)
    };
    refresh(&mut fx);
    assert_eq!(
        (fx.world.families[1].friends, fx.world.families[1].stale),
        (3, false)
    );
    // The other family is not counted.
    assert!(fx.world.families[0].stale);
    // A change the count reads that is not a relationship write (as a row
    // a relationship read makes) leaves the count the panel worked out.
    like(&mut fx, 1, 3, 0);
    assert_eq!(field(&mut fx, 9), 3);
    // A relationship write puts it out of date; the panel counts again,
    // and a later change is not seen.
    fx.world.families[1].stale = true;
    refresh(&mut fx);
    assert_eq!(
        (fx.world.families[1].friends, fx.world.families[1].stale),
        (2, false)
    );
    like(&mut fx, 1, 3, 90);
    assert_eq!(field(&mut fx, 9), 2);
    // An empty lot counts nothing.
    fx.world.globals[9] = 0;
    fx.world.families[1].stale = true;
    refresh(&mut fx);
    assert!(fx.world.families[1].stale);
}

#[test]
fn scope_24_reads_other_fields_of_the_record() {
    let mut fx = friends();
    // Word 58 of the record.
    assert_eq!(field(&mut fx, 2), 1058);
    assert_eq!(field(&mut fx, 3), 0);
    assert_eq!(field(&mut fx, 4), 0);
    // The house of the first family whose members include the record's
    // type, whatever its word 61 says.
    fx.world.neighbors.get_mut(2).unwrap().data[FAMILY] = 7;
    assert_eq!(field(&mut fx, 6), 12);
    fx.world.families[0].members.push(BOB);
    fx.world.families[0].house = 40;
    assert_eq!(field(&mut fx, 6), 40);
    fx.world.families.iter_mut().for_each(|f| f.members.clear());
    assert_eq!(field(&mut fx, 6), 0);
    // A field above 9, compared unsigned.
    for index in [10, 300, -1] {
        assert_eq!(field(&mut fx, index), 0);
    }
    // The value has no word pointer: a write finds none (0x13).
    assert_eq!(
        code(fx.expression(24, 2, op::ASSIGN, scope::LITERAL, 1)),
        Some(0x13)
    );
    // Fields 7 (0x425c5e) and 8 (0x425cb4) read bits 0 and 1 of the neighbor's
    // family's flags (+0x138) when the family lives in a house (house != 0),
    // and return 0 when unhoused or missing. Writes fail with native error 0x13.
    fx.world.neighbors.get_mut(2).unwrap().data[FAMILY] = 7;
    assert_eq!(field(&mut fx, 7), 0);
    assert_eq!(field(&mut fx, 8), 0);
    fx.world.family_flags.insert(7, 0x1);
    assert_eq!(field(&mut fx, 7), 1);
    assert_eq!(field(&mut fx, 8), 0);
    fx.world.family_flags.insert(7, 0x2);
    assert_eq!(field(&mut fx, 7), 0);
    assert_eq!(field(&mut fx, 8), 1);
    fx.world.family_flags.insert(7, 0x3);
    assert_eq!(field(&mut fx, 7), 1);
    assert_eq!(field(&mut fx, 8), 1);
    let saved_house = fx.world.families[0].house;
    fx.world.families[0].house = 0;
    assert_eq!(field(&mut fx, 7), 0);
    assert_eq!(field(&mut fx, 8), 0);
    fx.world.families[0].house = saved_house;
    for index in [7, 8] {
        assert_eq!(
            code(fx.expression(24, index, op::ASSIGN, scope::LITERAL, 0)),
            Some(0x13)
        );
    }
    // No record in the stack object's slot: every field is 0.
    fx.world.neighbors.slots.truncate(1);
    for index in [0, 1, 2, 5, 7, 8, 9] {
        assert_eq!(field(&mut fx, index), 0);
    }
}

#[test]
fn scope_24_field_5_counts_the_records_own_friends_of_a_character_type() {
    // 0x427148, for Bob (record 2): of his mutual friends at the
    // threshold 25 in a family (6, 7 and the second record of Anne's
    // type, 8; not 11), those whose type is registered from the
    // Characters folder.
    let mut fx = friends();
    fx.program.characters = [ANNE, 0x106, 0x107].into();
    fx.world.neighbors.get_mut(2).unwrap().stale = true;
    assert_eq!(field(&mut fx, 5), 3);
    // The count is kept, and the record stays out of date, so the next
    // read works it out again.
    let bob = fx.world.neighbors.get(2).unwrap();
    assert_eq!((bob.friends, bob.stale), (3, true));
    fx.program.characters.remove(&ANNE);
    assert_eq!(field(&mut fx, 5), 2);
    // An up-to-date record's kept count is read as it is.
    fx.world.neighbors.get_mut(2).unwrap().stale = false;
    fx.world.neighbors.get_mut(2).unwrap().friends = 41;
    assert_eq!(field(&mut fx, 5), 41);
    // The value has no word pointer either.
    assert_eq!(
        code(fx.expression(24, 5, op::ASSIGN, scope::LITERAL, 1)),
        Some(0x13)
    );
}

#[test]
fn scope_24_field_0_is_the_first_person_on_the_lot_of_the_records_type() {
    let mut fx = Fixture::new();
    fx.world.neighbors.slots = vec![None, Some(record(40, SIM_GUID))];
    // The first in the lot's list of people, not the lowest ID.
    fx.add_sim(9);
    fx.world.people = vec![9, SIM];
    assert_eq!(field(&mut fx, 0), 9);
    // No person on the lot has the record's type.
    fx.world.neighbors.slots[1].as_mut().unwrap().guid = LAMP_GUID;
    assert_eq!(field(&mut fx, 0), 0);
    // A type with two definitions: which the record is linked to is not
    // kept.
    let again = fx.catalog.objects[0].clone();
    fx.catalog.objects.push(again);
    let fault = fx
        .expression(scope::TEMPS, 0, op::ASSIGN, 24, 0)
        .unwrap_err();
    let Trap::Unsupported(text) = &fault.trap else {
        panic!("{fault}");
    };
    assert!(text.contains("0x425baa"), "{text}");
}

#[test]
fn scope_24_field_1_is_whether_the_selected_family_has_the_records_type() {
    let mut fx = Fixture::new();
    fx.world.neighbors.slots = vec![None, Some(record(40, SIM_GUID))];
    fx.world.families = vec![family(5, 1, &[LAMP_GUID, SIM_GUID]), family(6, 2, &[])];
    // The selected family is the one whose ID global 9 holds.
    fx.world.globals[9] = 5;
    assert_eq!(field(&mut fx, 1), 1);
    fx.world.globals[9] = 6;
    assert_eq!(field(&mut fx, 1), 0);
    // Global 9 at 0 may stand for no family or for family 0.
    fx.world.globals[9] = 0;
    let fault = fx
        .expression(scope::TEMPS, 0, op::ASSIGN, 24, 1)
        .unwrap_err();
    let Trap::Unsupported(text) = &fault.trap else {
        panic!("{fault}");
    };
    assert!(text.contains("global 9 at 0"), "{text}");
    // Global 9 naming no family.
    fx.world.globals[9] = 7;
    assert!(matches!(
        fx.expression(scope::TEMPS, 0, op::ASSIGN, 24, 1)
            .unwrap_err()
            .trap,
        Trap::Invalid(_)
    ));
    // A record of a type with no definition.
    fx.world.globals[9] = 5;
    fx.world.neighbors.slots[1].as_mut().unwrap().guid = 0x9999;
    let fault = fx
        .expression(scope::TEMPS, 0, op::ASSIGN, 24, 1)
        .unwrap_err();
    let Trap::Unsupported(text) = &fault.trap else {
        panic!("{fault}");
    };
    assert!(text.contains("0x425be2"), "{text}");
}

/// The Sim is neighbor 1. Slot 3 holds a record whose own ID is 7.
/// The families are up to date; the records' word 61 name families 1001
/// and 1006 (the first family 1001 in order is the one marked).
fn neighbors() -> Fixture {
    let mut fx = Fixture::new();
    fx.sim_mut().person.as_mut().unwrap().data[31] = 1;
    fx.sim_mut().temps = vec![0; 8];
    fx.world.neighbors.slots = vec![Some(record(1, SIM_GUID)), None, Some(record(7, LAMP_GUID))];
    fx.world.neighbors.get_mut(1).unwrap().data[FAMILY] = 1001;
    fx.world.neighbors.get_mut(3).unwrap().data[FAMILY] = 1006;
    fx.world.families = [1000, 1001, 1002, 1001, 1006]
        .into_iter()
        .map(|id| Family {
            stale: false,
            ..family(id, 1, &[])
        })
        .collect();
    fx
}

fn row(fx: &Fixture, id: i16, target: i32) -> Option<Vec<i32>> {
    let record = fx.world.neighbors.get(id).unwrap();
    record.row(target).map(|r| r.values.clone())
}

#[test]
fn relationship_between_records_keys_the_row_by_the_targets_own_id() {
    let mut fx = neighbors();
    // Write column 1, from me (neighbor 1) to the stack object (neighbor
    // 3): record 1 keeps the row, under record 3's own ID, 7.
    assert_eq!(
        sim_runs(&mut fx, 26, [1, 0, 6, 0, scope::LITERAL, 0, 9, 0], 3),
        Ok(true)
    );
    assert_eq!(row(&fx, 1, 7), Some(vec![0, 9]));
    assert_eq!(row(&fx, 1, 3), None);
    // The write marked both records, each record's family and the
    // registry byte.
    assert!(fx.world.neighbors.get(1).unwrap().stale);
    assert!(fx.world.neighbors.get(3).unwrap().stale);
    assert!(fx.world.neighbors.relationships_changed);
    let marked: Vec<(i32, bool)> = fx.world.families.iter().map(|f| (f.id, f.stale)).collect();
    assert_eq!(
        marked,
        [
            (1000, false),
            (1001, true),
            (1002, false),
            (1001, false),
            (1006, true)
        ]
    );
    // The other way round: record 3 keeps it, under my record's ID.
    assert_eq!(
        sim_runs(&mut fx, 26, [0, 1, 6, 0, scope::LITERAL, 0, 4, 0], 3),
        Ok(true)
    );
    assert_eq!(row(&fx, 3, 1), Some(vec![4]));
    // Read back into temp 2.
    assert_eq!(
        sim_runs(&mut fx, 26, [0, 1, 2, 0, scope::TEMPS, 0, 2, 0], 3),
        Ok(true)
    );
    assert_eq!(fx.sim().temps[2], 4);
}

#[test]
fn relationship_between_records_from_a_local() {
    let mut fx = neighbors();
    // Direction 2: from the stack object (neighbor 3) to the neighbor
    // local 0 names (1).
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, 1, 1),
            ins(
                26,
                RETURN_TRUE,
                RETURN_FALSE,
                [2, 2, 6, 0, scope::LITERAL, 0, 3, 0],
            ),
        ]),
    );
    assert_eq!(fx.run_with(SIM, 4096, 3, &[], BUDGET), Ok(true));
    assert_eq!(row(&fx, 3, 1), Some(vec![0, 0, 3]));
}

#[test]
fn relationship_between_records_with_flag_1_and_missing_records() {
    let mut fx = neighbors();
    // Reading a missing number with flag 1 is false and makes nothing.
    assert_eq!(
        sim_runs(&mut fx, 26, [5, 0, 3, 0, scope::TEMPS, 0, 2, 0], 3),
        Ok(false)
    );
    assert_eq!(row(&fx, 1, 7), None);
    assert!(!fx.world.neighbors.relationships_changed);
    // A write with flag 1 marks the records before it finds the number
    // missing.
    assert_eq!(
        sim_runs(&mut fx, 26, [5, 0, 7, 0, scope::LITERAL, 0, 1, 0], 3),
        Ok(false)
    );
    assert_eq!(row(&fx, 1, 7), None);
    assert!(fx.world.neighbors.get(3).unwrap().stale);
    assert!(fx.world.neighbors.relationships_changed);
    // The stack object names an empty slot.
    assert_eq!(
        code(sim_runs(
            &mut fx,
            26,
            [0, 0, 2, 0, scope::TEMPS, 0, 2, 0],
            2
        )),
        Some(0x15)
    );
    // So does my neighbor ID.
    fx.sim_mut().person.as_mut().unwrap().data[31] = 9;
    assert_eq!(
        code(sim_runs(
            &mut fx,
            26,
            [0, 0, 2, 0, scope::TEMPS, 0, 2, 0],
            3
        )),
        Some(0x15)
    );
}

/// Operation 3 for token kind 2 of the lamp's type, the count into temp
/// 1 and the place into temp 2.
const FIND: [u8; 8] = [3, 2, 0x20, 0x10, 0x11, 0x11, 0, 0];

fn inventories() -> Fixture {
    let mut fx = neighbors();
    let item = |kind, count| Item {
        kind,
        guid: LAMP_GUID,
        count,
    };
    fx.world.neighbors.inventories = vec![
        Inventory {
            neighbor: 7,
            items: Vec::new(),
        },
        Inventory {
            neighbor: 1,
            items: vec![item(1, 3), item(2, 5), item(2, 6)],
        },
    ];
    fx
}

#[test]
fn manage_inventory_finds_the_first_token_of_a_kind_in_the_owners_inventory() {
    let mut fx = inventories();
    assert_eq!(sim_runs(&mut fx, 51, FIND, 0), Ok(true));
    assert_eq!(fx.sim().temps[1..3], [5, 1]);
    // GUID 0 is the stack object's (the lamp's).
    fx.sim_mut().temps = vec![0; 8];
    let mut stack_objects = FIND;
    stack_objects[4..].fill(0);
    assert_eq!(sim_runs(&mut fx, 51, stack_objects, LAMP as i16), Ok(true));
    assert_eq!(fx.sim().temps[1..3], [5, 1]);
    // A kind it does not hold, or a count of 0: false, nothing written.
    fx.sim_mut().temps = vec![0; 8];
    let mut other = FIND;
    other[1] = 3;
    assert_eq!(sim_runs(&mut fx, 51, other, 0), Ok(false));
    fx.world.neighbors.inventories[1].items[1].count = 0;
    assert_eq!(sim_runs(&mut fx, 51, FIND, 0), Ok(false));
    assert_eq!(fx.sim().temps, [0; 8]);
}

/// Operation 4 for token kind 2: after the place in temp 3, the count
/// into temp 1 and the place into temp 2; flag 0x1 off, so any GUID.
const WALK: [u8; 8] = [4, 2, 0x20, 0x13, 0x11, 0x11, 0, 0];

/// Runs operation 4 with temp 3 holding `after`: the branch, and temps 1
/// and 2.
fn walk(fx: &mut Fixture, operands: [u8; 8], after: i16) -> (bool, [i16; 2]) {
    fx.sim_mut().temps = vec![0; 8];
    fx.sim_mut().temps[3] = after;
    let found = sim_runs(fx, 51, operands, 0).unwrap_or_else(|f| panic!("{f}"));
    (found, [fx.sim().temps[1], fx.sim().temps[2]])
}

#[test]
fn manage_inventory_walks_the_tokens_of_a_kind_after_a_place() {
    let mut fx = inventories();
    // Tokens: kind 1 (3), kind 2 (5), kind 2 (6).
    assert_eq!(walk(&mut fx, WALK, -1), (true, [5, 1]));
    assert_eq!(walk(&mut fx, WALK, 1), (true, [6, 2]));
    // None after: count 0, place -1, false.
    assert_eq!(walk(&mut fx, WALK, 2), (false, [0, -1]));
    assert_eq!(walk(&mut fx, WALK, 40), (false, [0, -1]));
    // A place after 0x7fff wraps negative and finds nothing (0x41f746).
    assert_eq!(walk(&mut fx, WALK, 0x7fff), (false, [0, -1]));
    // Unlike operation 3, a count of 0 is found.
    fx.world.neighbors.inventories[1].items[2].count = 0;
    assert_eq!(walk(&mut fx, WALK, 1), (true, [0, 2]));
    // Without flag 0x1 the GUID is not compared; with it, it is.
    fx.world.neighbors.inventories[1].items[1].guid = 0x9999;
    assert_eq!(walk(&mut fx, WALK, -1), (true, [5, 1]));
    let mut guid = WALK;
    guid[2] |= 1;
    assert_eq!(walk(&mut fx, guid, -1), (true, [0, 2]));
    // A place past 0x7fff is cut to a word: at 32768 it is -32768, and at
    // 65535 it is -1, which is false with the count written.
    let far = |at: usize| {
        let mut items = vec![
            Item {
                kind: 1,
                guid: LAMP_GUID,
                count: 1,
            };
            at
        ];
        items.push(Item {
            kind: 2,
            guid: LAMP_GUID,
            count: 7,
        });
        items
    };
    let mut wide = inventories();
    wide.world.neighbors.inventories[1].items = far(32768);
    assert_eq!(walk(&mut wide, WALK, -1), (true, [7, -32768]));
    wide.world.neighbors.inventories[1].items = far(65535);
    assert_eq!(walk(&mut wide, WALK, -1), (false, [7, -1]));
    // Operation 3 cuts it the same way.
    wide.sim_mut().temps = vec![0; 8];
    assert_eq!(sim_runs(&mut wide, 51, FIND, 0), Ok(true));
    assert_eq!(wide.sim().temps[1..3], [7, -1]);
    // The place is written after the count when both name one temp.
    let mut same = WALK;
    same[3] = 0x0b;
    fx.sim_mut().temps = vec![0; 8];
    fx.sim_mut().temps[3] = -1;
    assert_eq!(sim_runs(&mut fx, 51, same, 0), Ok(true));
    assert_eq!(fx.sim().temps[1], 1);
}

/// Operation 0 for token kind 2 of the lamp's type: one token, or with
/// flag 0x2 the count in temp `(flags >> 2) & 3`.
const ADD: [u8; 8] = [0, 2, 0, 0, 0x11, 0x11, 0, 0];

/// The owner's tokens: (kind, count) of each.
fn tokens(fx: &Fixture) -> Vec<(i32, i16)> {
    fx.world.neighbors.inventories[1]
        .items
        .iter()
        .map(|i| (i.kind, i.count as i16))
        .collect()
}

#[test]
fn manage_inventory_adds_tokens_to_the_first_of_a_kind_or_at_the_end() {
    let mut fx = inventories();
    // 0x41f38d: the first token of kind 2 (count 5) takes one more.
    assert_eq!(sim_runs(&mut fx, 51, ADD, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 6), (2, 6)]);
    // Flag 0x2: the count is in temp 3 ((0x0e >> 2) & 3).
    let mut from_temp = ADD;
    from_temp[2] = 0x0e;
    fx.sim_mut().temps[3] = 10;
    assert_eq!(sim_runs(&mut fx, 51, from_temp, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 16), (2, 6)]);
    // A count of 0 adds nothing and is true.
    fx.sim_mut().temps[3] = 0;
    assert_eq!(sim_runs(&mut fx, 51, from_temp, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 16), (2, 6)]);
    // 0x41f3f6: a sum of 0x7fff or more is 0x7fff.
    fx.sim_mut().temps[3] = 0x7ff0;
    assert_eq!(sim_runs(&mut fx, 51, from_temp, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 0x7fff), (2, 6)]);
    // The counts are signed words: a negative count takes some away.
    fx.sim_mut().temps[3] = -7;
    assert_eq!(sim_runs(&mut fx, 51, from_temp, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 0x7ff8), (2, 6)]);
    // A kind it does not hold: a token of that kind, GUID and count is
    // added at the end (0x41f7c8). The GUID must match too.
    let mut other = ADD;
    other[1] = 3;
    assert_eq!(sim_runs(&mut fx, 51, other, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 0x7ff8), (2, 6), (3, 1)]);
    let mut guid = ADD;
    guid[4] = 0x22;
    assert_eq!(sim_runs(&mut fx, 51, guid, 0), Ok(true));
    let last = fx.world.neighbors.inventories[1].items.last().unwrap();
    assert_eq!((last.kind, last.guid, last.count), (2, 0x1122, 1));
}

#[test]
fn manage_inventory_adds_no_token_to_a_full_inventory() {
    // 0x41f3cb: with 0x7fff tokens and none of the kind, the branch is
    // false and nothing is added; one of the kind still takes the count.
    let mut fx = inventories();
    fx.world.neighbors.inventories[1].items = vec![
        Item {
            kind: 1,
            guid: LAMP_GUID,
            count: 1,
        };
        0x7fff
    ];
    let mut other = ADD;
    other[1] = 3;
    assert_eq!(sim_runs(&mut fx, 51, other, 0), Ok(false));
    assert_eq!(fx.world.neighbors.inventories[1].items.len(), 0x7fff);
    let mut ones = ADD;
    ones[1] = 1;
    assert_eq!(sim_runs(&mut fx, 51, ones, 0), Ok(true));
    assert_eq!(fx.world.neighbors.inventories[1].items[0].count, 2);
}

#[test]
fn manage_inventory_makes_an_inventory_for_an_owner_without_one() {
    let mut fx = inventories();
    fx.world.neighbors.inventories.remove(1);
    assert_eq!(sim_runs(&mut fx, 51, FIND, 0), Ok(false));
    let inventories = &fx.world.neighbors.inventories;
    assert_eq!(inventories.len(), 2);
    assert_eq!(
        (inventories[1].neighbor, inventories[1].items.len()),
        (1, 0)
    );
}

#[test]
fn manage_inventory_needs_a_person_and_fails_loudly_elsewhere() {
    let mut fx = inventories();
    // The lamp is no person, unless flag 0x2000 names one in temp 4.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(51, RETURN_TRUE, RETURN_FALSE, FIND)]),
    );
    assert_eq!(code(fx.run_with(LAMP, 4096, 0, &[], BUDGET)), Some(0x1c));
    let mut through_temp = FIND;
    through_temp[3] |= 0x20;
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(51, RETURN_TRUE, RETURN_FALSE, through_temp)]),
    );
    fx.lamp_mut().temps[4] = SIM as i16;
    assert_eq!(fx.run_with(LAMP, 4096, 0, &[], BUDGET), Ok(true));
    assert_eq!(fx.lamp().temps[1..3], [5, 1]);
    // Operations 5 and 6 fail loudly when temp 0 names no neighbor record.
    for (op, addr) in [(5, "0x45544b"), (6, "0x4553fc")] {
        let mut ops = FIND;
        ops[0] = op;
        fx.sim_mut().temps[0] = 2; // empty slot
        assert!(
            matches!(trap(sim_runs(&mut fx, 51, ops, 0)), Trap::Unsupported(m) if m.contains(addr))
        );
    }
    // Operand GUID 0 resolves the stack object's GUID (0x455397) after
    // getting or creating the owner's inventory (0x455386), before
    // dispatch, even for operations 2, 5, 6 and 7+.
    fx.sim_mut().temps[0] = 1; // valid slot so ops 5 and 6 cannot trap there
    for op in [0, 1, 2, 3, 4, 5, 6, 7] {
        fx.world.neighbors.inventories.retain(|i| i.neighbor != 1);
        let no_guid = [op, 2, 0, 0, 0, 0, 0, 0];
        assert!(
            matches!(
                trap(sim_runs(&mut fx, 51, no_guid, 0)),
                Trap::Unsupported(m) if m.contains("no stack object")
            ),
            "op {op}"
        );
        assert!(
            fx.world
                .neighbors
                .inventories
                .iter()
                .any(|i| i.neighbor == 1),
            "op {op} should create the owner's empty inventory before reading stack object"
        );
    }
    // A person type with no record.
    fx.world.neighbors.slots[0] = None;
    assert!(matches!(
        trap(sim_runs(&mut fx, 51, FIND, 0)),
        Trap::Unsupported(_)
    ));
}

#[test]
fn manage_inventory_op_1_decrements_by_one_and_erases_at_zero_or_minus_one() {
    // 0x45569d → 0x41f427: default count 1 subtracts 1 from the first
    // matching (kind, guid).
    let mut fx = inventories();
    let remove = [1, 2, 0, 0, 0x11, 0x11, 0, 0];
    assert_eq!(sim_runs(&mut fx, 51, remove, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 4), (2, 6)]);

    // Flag 0x2 reads count from temp 3 ((0x0e >> 2) & 3). Even a count of
    // 3 only subtracts 1 in the original (0x41f447 pushes 1 into 0x41f471).
    let mut from_temp = remove;
    from_temp[2] = 0x0e;
    fx.sim_mut().temps[3] = 3;
    assert_eq!(sim_runs(&mut fx, 51, from_temp, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 3), (2, 6)]);

    // Any negative count other than -1 also subtracts 1 without erasing
    // while the remaining count stays positive (a token whose signed
    // count is 1 or less is still erased).
    fx.sim_mut().temps[3] = -2;
    assert_eq!(sim_runs(&mut fx, 51, from_temp, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 2), (2, 6)]);

    // Count 0 returns true without searching or modifying anything, even
    // for a kind that is not in the inventory.
    let mut missing_kind = from_temp;
    missing_kind[1] = 9;
    fx.sim_mut().temps[3] = 0;
    assert_eq!(sim_runs(&mut fx, 51, missing_kind, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 2), (2, 6)]);
    // With nonzero count, a missing kind or GUID returns false.
    fx.sim_mut().temps[3] = 1;
    assert_eq!(sim_runs(&mut fx, 51, missing_kind, 0), Ok(false));

    // Count -1 erases the first matching token even when its count > 1.
    fx.sim_mut().temps[3] = -1;
    assert_eq!(sim_runs(&mut fx, 51, from_temp, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 6)]);

    // Reaching 0 erases the token; a token whose signed count is <= 1
    // (including 0 or negative) is clamped to 0 and erased.
    fx.world.neighbors.inventories[1].items[0].count = 1;
    let mut kind1 = remove;
    kind1[1] = 1;
    assert_eq!(sim_runs(&mut fx, 51, kind1, 0), Ok(true));
    assert_eq!(tokens(&fx), [(2, 6)]);

    fx.world.neighbors.inventories[1].items[0].count = (-3i16) as u16;
    assert_eq!(sim_runs(&mut fx, 51, remove, 0), Ok(true));
    assert!(tokens(&fx).is_empty());
}

#[test]
fn manage_inventory_op_2_removes_by_index() {
    // 0x45560c → 0x41f56b: index in temp (flags >> 8) & 3 (temp 2 here),
    // count in temp (flags >> 2) & 3 (temp 3 here) when flag 0x2 is set.
    let mut fx = inventories();
    let by_index = [2, 0, 0x0e, 0x02, 0x11, 0x11, 0, 0];

    // Count 0 returns true without checking the index (even out of range).
    fx.sim_mut().temps[2] = -1;
    fx.sim_mut().temps[3] = 0;
    assert_eq!(sim_runs(&mut fx, 51, by_index, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 5), (2, 6)]);

    // Negative or past-the-end index returns false with nonzero count.
    fx.sim_mut().temps[3] = 1;
    assert_eq!(sim_runs(&mut fx, 51, by_index, 0), Ok(false));
    fx.sim_mut().temps[2] = 3;
    assert_eq!(sim_runs(&mut fx, 51, by_index, 0), Ok(false));

    // Positive count subtracts that count from the indexed token.
    fx.sim_mut().temps[2] = 1;
    fx.sim_mut().temps[3] = 3;
    assert_eq!(sim_runs(&mut fx, 51, by_index, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 2), (2, 6)]);

    // Subtracting to 0 or below erases the token.
    fx.sim_mut().temps[2] = 1;
    fx.sim_mut().temps[3] = 2;
    assert_eq!(sim_runs(&mut fx, 51, by_index, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3), (2, 6)]);

    // Any negative count (not just -1) erases the indexed token.
    fx.sim_mut().temps[2] = 1;
    fx.sim_mut().temps[3] = -5;
    assert_eq!(sim_runs(&mut fx, 51, by_index, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 3)]);

    // Without flag 0x2, default count is 1.
    let default_one = [2, 0, 0, 0x02, 0x11, 0x11, 0, 0];
    fx.sim_mut().temps[2] = 0;
    assert_eq!(sim_runs(&mut fx, 51, default_one, 0), Ok(true));
    assert_eq!(tokens(&fx), [(1, 2)]);
}

#[test]
fn manage_inventory_ops_5_and_6_replace_kind_1_or_3_with_temp_0_neighbor_guid() {
    let mut fx = inventories();
    fx.world.neighbors.inventories[1].items.push(Item {
        kind: 1,
        guid: 0xdead,
        count: 9,
    });
    fx.world.neighbors.inventories[1].items.push(Item {
        kind: 3,
        guid: 0xbeef,
        count: 4,
    });

    // Op 5 (0x455437): clears all kind 1 tokens and adds (1, slot3.guid, 1).
    // Slot 3 holds LAMP_GUID (record ID 7).
    fx.sim_mut().temps[0] = 3;
    let op5 = [5, 0, 0, 0, 0x11, 0x11, 0, 0];
    assert_eq!(sim_runs(&mut fx, 51, op5, 0), Ok(true));
    let items: Vec<_> = fx.world.neighbors.inventories[1]
        .items
        .iter()
        .map(|i| (i.kind, i.guid, i.count))
        .collect();
    assert_eq!(
        items,
        [
            (2, LAMP_GUID, 5),
            (2, LAMP_GUID, 6),
            (3, 0xbeef, 4),
            (1, LAMP_GUID, 1),
        ]
    );

    // Op 6 (0x4553e8): clears all kind 3 tokens and adds (3, slot1.guid, 1).
    // Run from the lamp with OWNER_IN_TEMP_4 (0x2000) to confirm temp 0
    // is read from the caller (the lamp), not the owner (the Sim).
    let op6 = [6, 0, 0, 0x20, 0x11, 0x11, 0, 0];
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(51, RETURN_TRUE, RETURN_FALSE, op6)]),
    );
    fx.lamp_mut().temps[0] = 1; // slot 1 holds SIM_GUID
    fx.lamp_mut().temps[4] = SIM as i16;
    fx.sim_mut().temps[0] = 99; // invalid slot on owner: must not be read
    assert_eq!(fx.run_with(LAMP, 4096, 0, &[], BUDGET), Ok(true));
    let items: Vec<_> = fx.world.neighbors.inventories[1]
        .items
        .iter()
        .map(|i| (i.kind, i.guid, i.count))
        .collect();
    assert_eq!(
        items,
        [
            (2, LAMP_GUID, 5),
            (2, LAMP_GUID, 6),
            (1, LAMP_GUID, 1),
            (3, SIM_GUID, 1),
        ]
    );

    // Full inventory (0x7fff tokens of another kind): op 5/6 still return
    // true (0x455418 ignores 0x41f38d's false) without exceeding 0x7fff.
    fx.world.neighbors.inventories[1].items = vec![
        Item {
            kind: 2,
            guid: LAMP_GUID,
            count: 1,
        };
        0x7fff
    ];
    fx.sim_mut().temps[0] = 1;
    assert_eq!(sim_runs(&mut fx, 51, op5, 0), Ok(true));
    assert_eq!(fx.world.neighbors.inventories[1].items.len(), 0x7fff);
}

#[test]
fn manage_inventory_unknown_operations_return_true_after_setup() {
    // 0x4553e6 → 0x455418: operations 7 and above still create the owner's
    // inventory if missing and return true.
    let mut fx = inventories();
    fx.world.neighbors.inventories.remove(1);
    let op7 = [7, 2, 0, 0, 0x11, 0x11, 0, 0];
    assert_eq!(sim_runs(&mut fx, 51, op7, 0), Ok(true));
    assert_eq!(fx.world.neighbors.inventories.len(), 2);
    assert_eq!(fx.world.neighbors.inventories[1].neighbor, 1);
}

/// The Sim, on a residential lot, with a thread on Idle For Input whose
/// count is parameter 0. Data word 71 is 0, so nothing stops an
/// interrupt; word 32 is 0 and no motive is low.
fn idle(count: i16, ask: u8) -> Fixture {
    let mut fx = Fixture::new();
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![ins(
            17,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, ask, 0, 0, 0, 0, 0],
        )]),
    );
    fx.world.globals[10] = 4;
    fx.program.zoning.insert(4, 1);
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[71] = 0;
    person.data[32] = 0;
    person.motives = [0.0; 16];
    fx.thread(SIM, 4096, &[count]);
    fx
}

fn turn(fx: &mut Fixture) -> Status {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter
        .turn(&mut fx.world, SIM, BUDGET)
        .unwrap_or_else(|f| panic!("{f}"))
}

#[test]
fn idle_for_input_counts_down_and_then_clears_the_interrupted_bit() {
    let mut fx = idle(2, 0);
    fx.sim_mut().data[8] = 0x41;
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.sim().stack[0].parameters[0], 1);
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.sim().data[8], 0x41, "counting leaves the bit");
    assert_eq!(turn(&mut fx), Status::Returned(true));
    assert_eq!(fx.sim().data[8], 0x01);
}

#[test]
fn idle_for_input_is_interrupted_by_a_low_motive_on_a_residential_lot() {
    for (motive, word_65, interrupted) in [
        (7, 9, true),
        (9, 9, true),
        (5, 1, true),
        (5, 2, false),
        (4, 0, false),
    ] {
        let mut fx = idle(3, 0);
        let person = fx.sim_mut().person.as_mut().unwrap();
        person.data[65] = word_65;
        person.motives[motive] = -99.5;
        let status = turn(&mut fx);
        let expected = if interrupted {
            Status::Returned(true)
        } else {
            Status::Yielded
        };
        assert_eq!(status, expected, "motive {motive}");
        assert_eq!(fx.sim().data[8] & 0x40 != 0, interrupted);
    }
    // -99 itself is not below, and not a number is.
    let mut fx = idle(3, 0);
    fx.sim_mut().person.as_mut().unwrap().motives[7] = -99.0;
    assert_eq!(turn(&mut fx), Status::Yielded);
    let mut fx = idle(3, 0);
    fx.sim_mut().person.as_mut().unwrap().motives[7] = f32::NAN;
    assert_eq!(turn(&mut fx), Status::Returned(true));
    // Not on a residential lot, on word 32 of 2, or with word 71 set,
    // nothing interrupts.
    for change in [
        |fx: &mut Fixture| fx.world.globals[10] = 5,
        |fx: &mut Fixture| fx.sim_mut().person.as_mut().unwrap().data[32] = 2,
        |fx: &mut Fixture| fx.sim_mut().person.as_mut().unwrap().data[71] = 1,
    ] {
        let mut fx = idle(3, 0);
        fx.sim_mut().person.as_mut().unwrap().motives[7] = -100.0;
        change(&mut fx);
        assert_eq!(turn(&mut fx), Status::Yielded);
    }
}

#[test]
fn idle_for_input_is_interrupted_by_a_queued_action() {
    let queued = |priority: i32, flags: u32| {
        let mut fx = idle(3, 0);
        let person = fx.sim_mut().person.as_mut().unwrap();
        person.data[33] = 50;
        let mut action = person.active.clone();
        (action.priority, action.flags) = (priority, flags);
        person.queue.push(action);
        fx
    };
    assert_eq!(turn(&mut queued(51, 0)), Status::Returned(true));
    assert_eq!(turn(&mut queued(50, 0)), Status::Yielded);
    assert_eq!(turn(&mut queued(0, 2)), Status::Returned(true));
    // Starting the queued action when asked is in push.rs.
    // With nothing queued, asking does not matter.
    let mut fx = idle(3, 1);
    fx.sim_mut().person.as_mut().unwrap().motives[9] = -100.0;
    assert_eq!(turn(&mut fx), Status::Returned(true));
}

#[test]
fn idle_for_input_needs_a_person_and_a_parameter() {
    let mut fx = idle(3, 0);
    fx.thread(SIM, 4096, &[]);
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.turn(&mut fx.world, SIM, BUDGET).unwrap_err();
    assert_eq!(fault.trap.code(), Some(8));
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(17, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    assert!(matches!(
        trap(fx.run_with(LAMP, 4096, 0, &[1], BUDGET)),
        Trap::Unsupported(_)
    ));
}

/// The lamp measures from the object its data word 11 names, or with
/// flag 1 from the one an operand names, to the Sim, into temp 3.
fn distance(fx: &mut Fixture, operands: [u8; 8]) -> Result<i16, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(11, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(LAMP, 4096, SIM as i16, &[], BUDGET)?;
    Ok(fx.lamp().temps[usize::from(operands[0])])
}

const TO_TEMP_3: [u8; 8] = [3, 0, 0, 0, 0, 0, 0, 0];

#[test]
fn get_distance_to_measures_in_tiles_and_truncates() {
    let mut fx = Fixture::new();
    fx.lamp_mut().data[11] = LAMP as i16;
    // 3 and 4 tiles apart on the same level: 5.
    fx.lamp_mut().position = [48, 0, 1];
    fx.sim_mut().position = [0, 64, 1];
    assert_eq!(distance(&mut fx, TO_TEMP_3), Ok(5));
    // One tile on each axis: 1.41 truncates to 1.
    fx.sim_mut().position = [32, 16, 1];
    assert_eq!(distance(&mut fx, TO_TEMP_3), Ok(1));
    // A level counts 20: sqrt(25 + 400) truncates to 20.
    fx.sim_mut().position = [0, 64, 2];
    assert_eq!(distance(&mut fx, TO_TEMP_3), Ok(20));
    // With flag 1, from the object a literal names: the Sim to itself.
    assert_eq!(
        distance(&mut fx, [3, 0, 1, scope::LITERAL, 2, 0, 0, 0]),
        Ok(0)
    );
    // To an object of type 7, always 0.
    fx.words(SIM_FILE)[9] = 7;
    assert_eq!(distance(&mut fx, TO_TEMP_3), Ok(0));
}

#[test]
fn get_distance_to_checks_its_temp_and_objects() {
    let mut fx = Fixture::new();
    // My data word 11 names no object.
    fx.lamp_mut().data[11] = 50;
    assert_eq!(code(distance(&mut fx, TO_TEMP_3).map(|_| true)), Some(0x17));
    fx.lamp_mut().data[11] = LAMP as i16;
    assert_eq!(
        code(distance(&mut fx, [8, 0, 0, 0, 0, 0, 0, 0]).map(|_| true)),
        Some(2)
    );
    // The stack object is missing.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(11, RETURN_TRUE, RETURN_FALSE, TO_TEMP_3)]),
    );
    assert_eq!(code(fx.run_with(LAMP, 4096, 60, &[], BUDGET)), Some(0x17));
}

/// The lamp looks from the object its data word 11 names, or with flag 1
/// from the one an operand names, towards the Sim, into a temp.
fn direction(fx: &mut Fixture, operands: [u8; 8]) -> Result<i16, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(12, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(LAMP, 4096, SIM as i16, &[], BUDGET)?;
    Ok(fx.lamp().temps[usize::from(operands[0])])
}

const DIRECTION_TO_TEMP_3: [u8; 8] = [3, 0, scope::TEMPS, 0, 0, 0, 0, 0];

#[test]
fn get_direction_to_turns_the_offset_into_eighths() {
    let mut fx = Fixture::new();
    fx.lamp_mut().data[11] = LAMP as i16;
    fx.lamp_mut().position = [64, 64, 1];
    // The position's second coordinate is the original's first (+0xf4),
    // as in route slots. Mostly along it, forwards and backwards.
    fx.sim_mut().position = [80, 160, 1];
    assert_eq!(direction(&mut fx, DIRECTION_TO_TEMP_3), Ok(4));
    fx.sim_mut().position = [64, 0, 1];
    assert_eq!(direction(&mut fx, DIRECTION_TO_TEMP_3), Ok(0));
    // Mostly along the second, forwards and backwards.
    fx.sim_mut().position = [160, 64, 1];
    assert_eq!(direction(&mut fx, DIRECTION_TO_TEMP_3), Ok(2));
    fx.sim_mut().position = [0, 64, 1];
    assert_eq!(direction(&mut fx, DIRECTION_TO_TEMP_3), Ok(6));
    // The diagonals.
    fx.sim_mut().position = [96, 96, 1];
    assert_eq!(direction(&mut fx, DIRECTION_TO_TEMP_3), Ok(3));
    fx.sim_mut().position = [96, 32, 1];
    assert_eq!(direction(&mut fx, DIRECTION_TO_TEMP_3), Ok(1));
    fx.sim_mut().position = [32, 96, 1];
    assert_eq!(direction(&mut fx, DIRECTION_TO_TEMP_3), Ok(5));
    fx.sim_mut().position = [32, 32, 1];
    assert_eq!(direction(&mut fx, DIRECTION_TO_TEMP_3), Ok(7));
    // The level does not count.
    fx.sim_mut().position = [80, 160, 2];
    assert_eq!(direction(&mut fx, DIRECTION_TO_TEMP_3), Ok(4));
    // With flag 1, from the object a literal names: the Sim to itself,
    // two equal points, is 3.
    assert_eq!(
        direction(
            &mut fx,
            [3, 0, scope::TEMPS, 0, 1, scope::LITERAL, SIM as u8, 0]
        ),
        Ok(3)
    );
}

#[test]
fn get_direction_to_checks_its_objects_and_destination() {
    let mut fx = Fixture::new();
    // My data word 11 names no object.
    fx.lamp_mut().data[11] = 50;
    assert_eq!(
        code(direction(&mut fx, DIRECTION_TO_TEMP_3).map(|_| true)),
        Some(0x17)
    );
    fx.lamp_mut().data[11] = LAMP as i16;
    // The stack object is missing.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            12,
            RETURN_TRUE,
            RETURN_FALSE,
            DIRECTION_TO_TEMP_3,
        )]),
    );
    assert_eq!(code(fx.run_with(LAMP, 4096, 60, &[], BUDGET)), Some(0x15));
    // A literal destination has no word behind it.
    assert!(matches!(
        trap(direction(&mut fx, [3, 0, scope::LITERAL, 0, 0, 0, 0, 0]).map(|_| true)),
        Trap::Unsupported(_)
    ));
}

#[test]
fn get_direction_to_from_global_34_writes_global_34() {
    // Global 34 resolves with a word pointer even read-only, and that
    // pointer replaces the destination, as in Random Number: the
    // direction lands in global 34, not the temp, even for a literal
    // destination.
    let mut fx = Fixture::new();
    fx.lamp_mut().position = [64, 64, 1];
    fx.sim_mut().position = [80, 160, 1];
    fx.world.globals[34] = LAMP as i16;
    assert_eq!(
        direction(&mut fx, [3, 0, scope::TEMPS, 0, 1, scope::GLOBAL, 34, 0]),
        Ok(0)
    );
    assert_eq!(fx.world.globals[34], 4);
    fx.world.globals[34] = LAMP as i16;
    assert_eq!(
        direction(&mut fx, [3, 0, scope::LITERAL, 0, 1, scope::GLOBAL, 34, 0]),
        Ok(0)
    );
    assert_eq!(fx.world.globals[34], 4);
}

#[test]
fn get_direction_to_into_a_motive_fails_loudly() {
    // A motive has no word behind it: the original writes through a null
    // pointer. The motive keeps its value.
    let mut fx = Fixture::new();
    fx.lamp_mut().data[11] = LAMP as i16;
    let before = fx.sim().person.as_ref().unwrap().motives[3];
    assert!(matches!(
        trap(direction(&mut fx, [3, 0, 15, 0, 0, 0, 0, 0]).map(|_| true)),
        Trap::Unsupported(_)
    ));
    assert_eq!(fx.sim().person.as_ref().unwrap().motives[3], before);
}

/// Runs Generic Sims Call routine 6 as the Sim, with `stack` the stack
/// object (which names a record by its place plus one).
fn leave_family(fx: &mut Fixture, stack: i16) -> Result<bool, sims_vm::Fault> {
    sim_runs(fx, 1, [6, 0, 0, 0, 0, 0, 0, 0], stack)
}

/// The Sim's record (place 2) and Anne's (place 1), both of family 5,
/// which global 9 selects, on a residential lot; family 7 also lists the
/// Sim's type. Neither family's count is out of date.
fn leaving() -> Fixture {
    let mut fx = Fixture::new();
    fx.world.neighbors.slots = vec![Some(member(30, ANNE, 5)), Some(member(31, SIM_GUID, 5))];
    fx.world.families = vec![
        family(7, 0, &[SIM_GUID]),
        family(5, 12, &[SIM_GUID, ANNE, SIM_GUID]),
    ];
    for family in &mut fx.world.families {
        family.stale = false;
    }
    (fx.world.globals[9], fx.world.globals[10]) = (5, 0);
    let sim = fx.sim_mut().person.as_mut().unwrap();
    (sim.data[FAMILY], sim.data[32]) = (5, 0);
    fx
}

#[test]
fn generic_call_6_takes_the_stack_objects_neighbor_out_of_its_family() {
    let mut fx = leaving();
    assert_eq!(leave_family(&mut fx, 2), Ok(true));
    // The record and the live person of its type leave the family; the
    // first member of the type goes, and the family's count is out of
    // date. Family 7 is not touched.
    assert_eq!(fx.world.neighbors.get(2).unwrap().data[FAMILY], 0);
    assert_eq!(fx.world.neighbors.get(1).unwrap().data[FAMILY], 5);
    assert_eq!(fx.world.families[1].members, [ANNE, SIM_GUID]);
    assert!(fx.world.families[1].stale);
    assert_eq!(fx.world.families[0].members, [SIM_GUID]);
    assert!(!fx.world.families[0].stale);
    let sim = fx.sim().person.as_ref().unwrap();
    assert_eq!(sim.data[FAMILY], 0);
    // The visitor pass: the Sim's type is still a member of family 5.
    assert_eq!(sim.data[32], 0);
    // Once its type is no member, the pass makes it a visitor.
    let mut fx = leaving();
    fx.world.families[1].members = vec![SIM_GUID, ANNE];
    assert_eq!(leave_family(&mut fx, 2), Ok(true));
    assert_eq!(fx.world.families[1].members, [ANNE]);
    assert_eq!(fx.sim().person.as_ref().unwrap().data[32], 1);
}

#[test]
fn generic_call_6_is_false_unless_the_record_is_a_member_of_its_family() {
    // Family word 0, a family ID no family has, and a family that does
    // not list the record's type: false, and nothing changes.
    for change in 0..3 {
        let mut fx = leaving();
        match change {
            0 => fx.world.neighbors.get_mut(2).unwrap().data[FAMILY] = 0,
            1 => fx.world.neighbors.get_mut(2).unwrap().data[FAMILY] = 6,
            _ => fx.world.families[1].members = vec![ANNE],
        }
        let mut expected = fx.world.clone();
        assert_eq!(leave_family(&mut fx, 2), Ok(false), "case {change}");
        expected.objects.get_mut(&SIM).unwrap().last_result = false;
        assert_eq!(fx.world.neighbors, expected.neighbors, "case {change}");
        assert_eq!(fx.world.families, expected.families, "case {change}");
        assert_eq!(
            fx.sim().person.as_ref().unwrap().data,
            expected.objects[&SIM].person.as_ref().unwrap().data,
            "case {change}"
        );
    }
}

#[test]
fn generic_call_6_needs_a_record_at_the_stack_objects_place() {
    let mut fx = leaving();
    for stack in [0, 3, -1] {
        assert_eq!(code(leave_family(&mut fx, stack)), Some(0x37), "{stack}");
    }
}

/// Runs Generic Sims Call routine 21 as the Sim, with temp 0 naming
/// `person`.
fn restore(fx: &mut Fixture, person: i16) -> Result<bool, sims_vm::Fault> {
    fx.sim_mut().temps[0] = person;
    sim_runs(fx, 1, [21, 0, 0, 0, 0, 0, 0, 0], 0)
}

/// A token of `kind` with `guid` and `count`.
fn token(kind: i32, guid: u32, count: i16) -> Item {
    Item {
        kind,
        guid,
        count: count as u16,
    }
}

#[test]
fn generic_call_21_restores_motives_and_words_from_kind_2_tokens_and_erases_them() {
    let mut fx = neighbors();
    fx.world.neighbors.inventories = vec![Inventory {
        neighbor: 1,
        items: vec![
            token(1, 0, 40),
            token(2, 0, 11),
            token(2, 1, 12),
            token(3, 2, 99),
            token(2, 2, 13),
            token(2, 3, -14),
            token(2, 4, 15),
            token(2, 5, 16),
            token(2, 6, 17),
            token(2, 9, 21),
            token(2, 10, 22),
            token(2, 11, 23),
            token(2, 12, 24),
        ],
    }];
    assert_eq!(restore(&mut fx, SIM as i16), Ok(true));
    let sim = fx.sim().person.as_ref().unwrap();
    // GUIDs 0 to 6 name motives 9, 6, 5, 15, 7, 8 and 14 (0x45d133); the
    // count is signed.
    for (motive, value) in [
        (9, 11.0),
        (6, 12.0),
        (5, 13.0),
        (15, -14.0),
        (7, 15.0),
        (8, 16.0),
        (14, 17.0),
    ] {
        assert_eq!(sim.motives[motive], value, "motive {motive}");
    }
    // GUIDs 9 to 12 name words 8, 80, 82 and 81 (+0x59c, +0x62c, +0x630,
    // +0x62e).
    assert_eq!(
        [sim.data[8], sim.data[80], sim.data[82], sim.data[81]],
        [21, 22, 23, 24]
    );
    // Every kind 2 token is gone; the others stay, in order.
    assert_eq!(
        fx.world.neighbors.inventories[0].items,
        [token(1, 0, 40), token(3, 2, 99)]
    );
}

#[test]
fn generic_call_21_leaves_a_token_that_names_nothing_and_stops_there() {
    let mut fx = neighbors();
    fx.world.neighbors.inventories = vec![Inventory {
        neighbor: 1,
        items: vec![token(2, 0, 30), token(2, 7, 5), token(2, 1, 31)],
    }];
    let before = fx.sim().person.as_ref().unwrap().motives[6];
    assert_eq!(restore(&mut fx, SIM as i16), Ok(true));
    // The first token is restored and erased. Each later pass finds the
    // GUID 7 token first and leaves it, so the third is never reached.
    let sim = fx.sim().person.as_ref().unwrap();
    assert_eq!(sim.motives[9], 30.0);
    assert_eq!(sim.motives[6], before);
    assert_eq!(
        fx.world.neighbors.inventories[0].items,
        [token(2, 7, 5), token(2, 1, 31)]
    );
    // GUIDs 8 and above 12 name nothing either.
    for guid in [8, 13, 0x8000_0000] {
        let mut fx = neighbors();
        let items = vec![token(2, guid, 5)];
        fx.world.neighbors.inventories = vec![Inventory {
            neighbor: 1,
            items: items.clone(),
        }];
        let motives = fx.sim().person.as_ref().unwrap().motives;
        assert_eq!(restore(&mut fx, SIM as i16), Ok(true), "{guid}");
        assert_eq!(fx.world.neighbors.inventories[0].items, items, "{guid}");
        assert_eq!(fx.sim().person.as_ref().unwrap().motives, motives, "{guid}");
    }
}

#[test]
fn generic_call_21_adds_an_empty_inventory_for_a_person_without_one() {
    let mut fx = neighbors();
    fx.world.neighbors.inventories = vec![Inventory {
        neighbor: 7,
        items: vec![token(2, 0, 50)],
    }];
    let motives = fx.sim().person.as_ref().unwrap().motives;
    assert_eq!(restore(&mut fx, SIM as i16), Ok(true));
    assert_eq!(fx.sim().person.as_ref().unwrap().motives, motives);
    // Neighbor 7's tokens are not the Sim's (neighbor 1's).
    assert_eq!(
        fx.world.neighbors.inventories,
        [
            Inventory {
                neighbor: 7,
                items: vec![token(2, 0, 50)],
            },
            Inventory {
                neighbor: 1,
                items: Vec::new(),
            },
        ]
    );
}

#[test]
fn generic_call_21_needs_temp_0_to_name_a_person() {
    // No object, an object that is not a person (the lamp), and IDs of 0
    // or below: native error 0x1c.
    for person in [0, -1, LAMP as i16, 99] {
        let mut fx = neighbors();
        assert_eq!(code(restore(&mut fx, person)), Some(0x1c), "{person}");
    }
}

#[test]
fn generic_call_3_takes_the_true_branch_and_changes_nothing() {
    // Taking down the interface's travel picture is presentation.
    let mut fx = neighbors();
    let mut expected = fx.world.clone();
    assert_eq!(sim_runs(&mut fx, 1, [3, 0, 0, 0, 0, 0, 0, 0], 0), Ok(true));
    expected.objects.get_mut(&SIM).unwrap().last_result = true;
    assert_eq!(fx.world.neighbors, expected.neighbors);
    assert_eq!(fx.world.globals, expected.globals);
    let (now, before) = (
        fx.sim().person.as_ref().unwrap(),
        expected.objects[&SIM].person.as_ref().unwrap(),
    );
    assert_eq!(now.data, before.data);
    assert_eq!(now.motives, before.motives);
}

#[test]
fn generic_call_21_reads_a_token_past_0x7fff_through_its_place_cut_to_a_word() {
    // `at` tokens of kind 1 (the one at 5 with GUID 1 and count 44),
    // then one of kind 2 with GUID 2 and count 9.
    let run = |at: usize| {
        let mut fx = neighbors();
        let mut items = vec![token(1, 0, 3); at];
        items[5] = token(1, 1, 44);
        items.push(token(2, 2, 9));
        fx.world.neighbors.inventories = vec![Inventory { neighbor: 1, items }];
        fx.sim_mut().person.as_mut().unwrap().motives = [50.0; 16];
        assert_eq!(restore(&mut fx, SIM as i16), Ok(true), "{at}");
        let items = &fx.world.neighbors.inventories[0].items;
        let motives = fx.sim().person.as_ref().unwrap().motives;
        (items.len(), items[5].clone(), motives)
    };
    // Place -32768: nothing is read, so GUID 0 and count 1 restore motive
    // 9, and nothing is erased.
    let (len, fifth, motives) = run(0x8000);
    assert_eq!((len, fifth), (0x8001, token(1, 1, 44)));
    assert_eq!((motives[9], motives[5]), (1.0, 50.0));
    // Place -1: not found.
    let (len, _, motives) = run(0xffff);
    assert_eq!((len, motives), (0x10000, [50.0; 16]));
    // Place 5 (index 65541): the kind 1 token there is read (GUID 1,
    // motive 6) and erased.
    let (len, fifth, motives) = run(0x10005);
    assert_eq!((len, fifth), (0x10005, token(1, 0, 3)));
    assert_eq!((motives[6], motives[5]), (44.0, 50.0));
}

/// Runs Generic Sims Call 19 on the Sim with `person` in its temp 0, and
/// gives the Sim's temp 0 after.
fn companion(fx: &mut Fixture, person: i16) -> Result<i16, sims_vm::Fault> {
    fx.sim_mut().temps[0] = person;
    assert!(sim_runs(fx, 1, [19, 0, 0, 0, 0, 0, 0, 0], 0)?);
    Ok(fx.sim().temps[0])
}

#[test]
fn generic_call_19_finds_the_companion_on_the_lot_and_takes_one_from_its_token() {
    // The first kind 1 token names the Sim's own type, which is on the
    // lot: temp 0 takes the Sim's ID, and the token's count drops by one.
    let mut fx = neighbors();
    fx.world.neighbors.inventories = vec![Inventory {
        neighbor: 1,
        items: vec![
            token(2, 0, 5),
            token(1, SIM_GUID, 2),
            token(1, LAMP_GUID, 1),
        ],
    }];
    assert_eq!(companion(&mut fx, SIM as i16), Ok(SIM as i16));
    assert_eq!(
        fx.world.neighbors.inventories[0].items,
        [
            token(2, 0, 5),
            token(1, SIM_GUID, 1),
            token(1, LAMP_GUID, 1)
        ]
    );
    // Again: the count runs out and the token is erased.
    assert_eq!(companion(&mut fx, SIM as i16), Ok(SIM as i16));
    assert_eq!(
        fx.world.neighbors.inventories[0].items,
        [token(2, 0, 5), token(1, LAMP_GUID, 1)]
    );
}

#[test]
fn generic_call_19_with_no_kind_1_token_sets_temp_0_to_minus_1() {
    // No inventory: an empty one is added, and temp 0 is -1.
    let mut fx = neighbors();
    fx.world.neighbors.inventories = vec![Inventory {
        neighbor: 7,
        items: vec![token(1, SIM_GUID, 1)],
    }];
    assert_eq!(companion(&mut fx, SIM as i16), Ok(-1));
    assert_eq!(
        fx.world.neighbors.inventories,
        [
            Inventory {
                neighbor: 7,
                items: vec![token(1, SIM_GUID, 1)],
            },
            Inventory {
                neighbor: 1,
                items: Vec::new(),
            },
        ]
    );
    // Tokens of other kinds are left alone.
    let mut fx = neighbors();
    let items = vec![token(2, SIM_GUID, 1), token(3, SIM_GUID, 1)];
    fx.world.neighbors.inventories = vec![Inventory {
        neighbor: 1,
        items: items.clone(),
    }];
    assert_eq!(companion(&mut fx, SIM as i16), Ok(-1));
    assert_eq!(fx.world.neighbors.inventories[0].items, items);
}

#[test]
fn generic_call_19_takes_the_token_even_when_its_type_is_no_person() {
    // A GUID missing from the table of person types (0x4e9753 gives -1),
    // whether no type at all or the lamp's: nothing is made, temp 0 is
    // -1, and the token is still taken.
    for guid in [0xDEAD_BEEF, LAMP_GUID] {
        let mut fx = neighbors();
        fx.world.neighbors.inventories = vec![Inventory {
            neighbor: 1,
            items: vec![token(1, guid, 1), token(1, SIM_GUID, 1)],
        }];
        let objects: Vec<u16> = fx.world.objects.keys().copied().collect();
        assert_eq!(companion(&mut fx, SIM as i16), Ok(-1), "{guid:08x}");
        assert_eq!(
            fx.world.neighbors.inventories[0].items,
            [token(1, SIM_GUID, 1)]
        );
        assert_eq!(
            fx.world.objects.keys().copied().collect::<Vec<_>>(),
            objects
        );
    }
}

#[test]
fn generic_call_19_makes_the_companion_when_none_is_on_the_lot() {
    // The widget is a person type with nobody of it on the lot: the
    // factory makes one (0x4e9719), off the lot, and temp 0 takes its ID.
    let (mut fx, _, _) = crate::create_person::visitors();
    fx.sim_mut().person.as_mut().unwrap().data[31] = 5;
    fx.sim_mut().temps = vec![0; 8];
    fx.world.neighbors.inventories = vec![Inventory {
        neighbor: 5,
        items: vec![token(1, crate::factory::WIDGET, 1)],
    }];
    assert!(
        !fx.world
            .people
            .iter()
            .any(|id| fx.world.objects[id].guid == crate::factory::WIDGET)
    );
    let made = companion(&mut fx, SIM as i16).expect("routine 19 runs");
    let made = u16::try_from(made).expect("a companion was made");
    assert_eq!(fx.world.objects[&made].guid, crate::factory::WIDGET);
    assert!(fx.world.objects[&made].person.is_some());
    assert!(fx.world.people.contains(&made));
    assert_eq!(fx.world.neighbors.inventories[0].items, []);
}

#[test]
fn generic_call_19_needs_temp_0_to_name_a_person() {
    for person in [0, -1, LAMP as i16, 99] {
        let mut fx = neighbors();
        fx.sim_mut().temps[0] = person;
        let result = sim_runs(&mut fx, 1, [19, 0, 0, 0, 0, 0, 0, 0], 0);
        assert_eq!(code(result), Some(0x1c), "{person}");
    }
}

#[test]
fn generic_call_19_reads_a_token_past_0x7fff_through_its_place_cut_to_a_word() {
    // `at` tokens of kind 3 (the one at 5 names the Sim's type, count 2),
    // then one of kind 1 naming the Sim's type, count 1.
    let run = |at: usize| {
        let mut fx = neighbors();
        let mut items = vec![token(3, 0, 1); at];
        items[5] = token(3, SIM_GUID, 2);
        items.push(token(1, SIM_GUID, 1));
        fx.world.neighbors.inventories = vec![Inventory { neighbor: 1, items }];
        let found = companion(&mut fx, SIM as i16).expect("routine 19 runs");
        let items = &fx.world.neighbors.inventories[0].items;
        (found, items.len(), items[5].clone())
    };
    // Place -32768 or -1: as if none were found; nothing is taken.
    assert_eq!(run(0x8000), (-1, 0x8001, token(3, SIM_GUID, 2)));
    assert_eq!(run(0xffff), (-1, 0x10000, token(3, SIM_GUID, 2)));
    // Place 5 (index 65541): the kind 3 token there is read and one is
    // taken from it.
    assert_eq!(run(0x10005), (SIM as i16, 0x10006, token(3, SIM_GUID, 1)));
}
