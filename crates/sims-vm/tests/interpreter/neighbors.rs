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
use sims_vm::{Status, Trap};

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
fn scope_24_field_9_is_0_without_a_family_with_a_house() {
    // A family with no house: 0, and its count is not worked out.
    let mut fx = friends();
    fx.world.families[1].house = 0;
    assert_eq!(field(&mut fx, 9), 0);
    assert!(fx.world.families[1].stale);
    // The record's word 61 names no family.
    let mut fx = friends();
    fx.world.neighbors.get_mut(2).unwrap().data[FAMILY] = 6;
    assert_eq!(field(&mut fx, 9), 0);
    // Word 61 is 0, even with a family 0 that has a house.
    let mut fx = friends();
    fx.world.families.push(family(0, 3, &[]));
    fx.world.neighbors.get_mut(2).unwrap().data[FAMILY] = 0;
    assert_eq!(field(&mut fx, 9), 0);
    assert!(fx.world.families[2].stale);
    // The first family of an ID is the one read.
    let mut fx = friends();
    fx.world.families.insert(0, family(5, 0, &[ANNE, BOB]));
    assert_eq!(field(&mut fx, 9), 0);
    assert!(fx.world.families[2].stale);
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
    // Fields that read what the simulation does not keep fail loudly,
    // naming the original's code, even in a write.
    for (index, address) in [
        (0, "0x425baa"),
        (1, "0x425be2"),
        (5, "0x427148"),
        (7, "0x425c5e"),
        (8, "0x425cb4"),
    ] {
        for operator in [op::EQUAL, op::ASSIGN] {
            let fault = fx
                .expression(24, index, operator, scope::LITERAL, 0)
                .unwrap_err();
            let Trap::Unsupported(text) = &fault.trap else {
                panic!("{fault}");
            };
            assert!(text.contains(address), "{text}");
        }
    }
    // No record in the stack object's slot: every field is 0.
    fx.world.neighbors.slots.truncate(1);
    for index in [0, 1, 2, 5, 7, 8, 9] {
        assert_eq!(field(&mut fx, index), 0);
    }
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
    // Other operations are not ported.
    let mut add = FIND;
    add[0] = 1;
    assert!(matches!(
        trap(sim_runs(&mut fx, 51, add, 0)),
        Trap::Unsupported(_)
    ));
    // A person type with no record.
    fx.world.neighbors.slots[0] = None;
    assert!(matches!(
        trap(sim_runs(&mut fx, 51, FIND, 0)),
        Trap::Unsupported(_)
    ));
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
