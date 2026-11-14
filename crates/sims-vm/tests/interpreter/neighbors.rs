//! What a lot's scripts do with the neighborhood's records, and the
//! person primitives the Goth house's second tick reaches: an
//! expression's neighbor person data (scope 32), Relationship between
//! records, Manage Inventory, Idle For Input and Get Distance To.
//!
//! Each primitive's doc comment cites the original's code it was ported
//! from. These tests check the port's rules on synthetic data only.

use crate::support::*;
use sims_sim::neighbors::{Inventory, Item, Neighbor};
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

/// The Sim is neighbor 1. Slot 3 holds a record whose own ID is 7.
fn neighbors() -> Fixture {
    let mut fx = Fixture::new();
    fx.sim_mut().person.as_mut().unwrap().data[31] = 1;
    fx.sim_mut().temps = vec![0; 8];
    fx.world.neighbors.slots = vec![Some(record(1, SIM_GUID)), None, Some(record(7, LAMP_GUID))];
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
    // The write marked both records and the registry byte.
    assert!(fx.world.neighbors.get(1).unwrap().stale);
    assert!(fx.world.neighbors.get(3).unwrap().stale);
    assert!(fx.world.neighbors.relationships_changed);
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
    // Asked to start the queued action: not ported.
    let mut fx = queued(51, 0);
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![ins(
            17,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 1, 0, 0, 0, 0, 0],
        )]),
    );
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.turn(&mut fx.world, SIM, BUDGET).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
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
