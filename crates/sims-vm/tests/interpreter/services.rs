//! The smaller services the lot-wide controllers reach: Generic Sims Call,
//! event ownership, type attributes, per-type instance lists and the
//! career list.

use crate::support::*;
use sims_sim::{MagicHistory, TypeAttributes};
use sims_vm::Trap;
use std::collections::BTreeMap;

fn generic(fx: &mut Fixture, routine: u8) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            1,
            RETURN_TRUE,
            RETURN_FALSE,
            [routine, 0, 0, 0, 0, 0, 0, 0],
        )]),
    );
    fx.run()
}

#[test]
fn generic_call_28_classifies_a_lot() {
    let mut fx = Fixture::new();
    fx.program.zoning = BTreeMap::from([(5, 1), (58, 2), (7, 3), (91, 2), (95, 1)]);
    // Residential 0, community 1, unknown -1. Lot 0 is residential, and
    // the Magic Town lots are fixed whatever the table says.
    for (lot, class) in [
        (5, 0),
        (58, 1),
        (7, -1),
        (6, -1),
        (-4, -1),
        (0, 0),
        (90, 0),
        (91, 0),
        (92, 0),
        (93, 1),
        (95, 1),
        (98, 1),
        (99, -1),
    ] {
        fx.lamp_mut().temps[0] = lot;
        assert_eq!(generic(&mut fx, 28), Ok(true));
        assert_eq!(fx.lamp().temps[0], class, "lot {lot}");
    }
}

#[test]
fn generic_call_43_does_nothing_without_a_family_history() {
    let mut fx = Fixture::new();
    let mut expected = fx.world.clone();
    expected.objects.get_mut(&LAMP).unwrap().last_result = true;
    assert_eq!(generic(&mut fx, 43), Ok(true));
    assert_eq!(hash_without_frames(&fx.world), expected.hash());
}

/// The magic controller's type, which routine 43 looks for.
const MAGIC_CONTROLLER: u32 = 0xb6c9_0029;

#[test]
fn generic_call_43_raises_the_magic_controller_to_the_familys_history() {
    let mut fx = Fixture::new();
    fx.world.magic_history = Some(MagicHistory {
        family: 5,
        values: [4, 0, -3, 9, 1, 2],
    });
    (fx.world.globals[9], fx.world.globals[10]) = (5, 0);
    // No controller on the lot: the type's instance list is made, empty.
    assert_eq!(generic(&mut fx, 43), Ok(true));
    assert_eq!(fx.world.instances[&MAGIC_CONTROLLER], [0u16; 0]);
    // With one, each of its first six attributes becomes the larger of
    // itself and the family's word.
    fx.world.instances.clear();
    let mut controller = object(9, MAGIC_CONTROLLER);
    controller.attributes = vec![1, 7, 0, 0, 5, 2, 11, 12];
    fx.world.objects.insert(9, controller);
    assert_eq!(generic(&mut fx, 43), Ok(true));
    assert_eq!(fx.world.instances[&MAGIC_CONTROLLER], [9]);
    assert_eq!(fx.world.objects[&9].attributes, [4, 7, 0, 9, 5, 2, 11, 12]);
    // Another family in global 9, or a lot that is not residential, and
    // nothing happens.
    for (family, lot) in [(6, 0), (5, 3)] {
        fx.world.objects.get_mut(&9).unwrap().attributes[0] = 1;
        (fx.world.globals[9], fx.world.globals[10]) = (family, lot);
        assert_eq!(generic(&mut fx, 43), Ok(true));
        assert_eq!(fx.world.objects[&9].attributes[0], 1);
    }
    fx.program.zoning.insert(3, 1);
    assert_eq!(generic(&mut fx, 43), Ok(true));
    assert_eq!(fx.world.objects[&9].attributes[0], 4);
}

#[test]
fn generic_calls_10_and_11_keep_the_list_of_fires() {
    let mut fx = Fixture::new();
    assert_eq!(generic(&mut fx, 11), Ok(true));
    assert_eq!(fx.world.fires, [0u16; 0]);
    assert_eq!(generic(&mut fx, 10), Ok(true));
    assert_eq!(generic(&mut fx, 10), Ok(true));
    assert_eq!(fx.world.fires, [LAMP, LAMP]);
    // Leaving takes one entry off.
    assert_eq!(generic(&mut fx, 11), Ok(true));
    assert_eq!(fx.world.fires, [LAMP]);
}

/// Runs Generic Sims Call routine 15 as the Sim, with temp 0 at `temp`.
fn generic_15(fx: &mut Fixture, temp: i16) -> Result<bool, sims_vm::Fault> {
    fx.sim_mut().temps[0] = temp;
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![ins(
            1,
            RETURN_TRUE,
            RETURN_FALSE,
            [15, 0, 0, 0, 0, 0, 0, 0],
        )]),
    );
    fx.run_with(SIM, 4096, 0, &[], 10)
}

/// A Sim at (56, 56) facing 2, its definition insetting no side, and
/// person word 73 set.
fn standing_sim() -> Fixture {
    let mut fx = Fixture::new();
    fx.words(SIM_FILE)[0x9a / 2] = 0;
    let sim = fx.sim_mut();
    (sim.position, sim.footprint, sim.data[1]) = ([56, 56, 1], [1, 2, 3, 4], 2);
    sim.person.as_mut().unwrap().data[73] = 0x0102;
    fx
}

#[test]
fn generic_call_15_copies_temp_0_into_person_word_72() {
    let mut fx = standing_sim();
    let mut expected = fx.world.clone();
    let sim = expected.objects.get_mut(&SIM).unwrap();
    sim.temps[0] = 9;
    sim.last_result = true;
    let data = &mut sim.person.as_mut().unwrap().data;
    (data[72], data[73]) = (9, 0);
    // Word 73 is cleared, and with temp 0 set the footprint is kept.
    assert_eq!(generic_15(&mut fx, 9), Ok(true));
    assert_eq!(hash_without_frames(&fx.world), expected.hash());
}

#[test]
fn generic_call_15_with_temp_0_clear_works_the_footprint_out_again() {
    let mut fx = standing_sim();
    let mut expected = fx.world.clone();
    let sim = expected.objects.get_mut(&SIM).unwrap();
    sim.last_result = true;
    sim.footprint = [53, 53, 59, 59];
    let data = &mut sim.person.as_mut().unwrap().data;
    (data[72], data[73]) = (0, 0);
    assert_eq!(generic_15(&mut fx, 0), Ok(true));
    assert_eq!(hash_without_frames(&fx.world), expected.hash());

    // The footprint is worked out after word 73 is cleared: on an even
    // facing the word would otherwise take from the definition's insets.
    let mut fx = standing_sim();
    fx.words(SIM_FILE)[0x9a / 2] = 0x1111;
    assert_eq!(generic_15(&mut fx, 0), Ok(true));
    assert_eq!(fx.sim().footprint, [54, 54, 58, 58]);
}

#[test]
fn generic_call_15_needs_a_person_and_a_footprint_it_can_work_out() {
    // The lamp is not a person: native error 0x1c.
    let mut fx = standing_sim();
    assert_eq!(trap(generic(&mut fx, 15)).code(), Some(0x1c));
    assert!(fx.lamp().person.is_none());
    let sim = fx.sim();
    assert_eq!(sim.person.as_ref().unwrap().data[72..74], [216, 0x0102]);
    assert_eq!(sim.footprint, [1, 2, 3, 4]);
    // A facing the footprint does not handle fails before anything
    // changes.
    let mut fx = standing_sim();
    fx.words(SIM_FILE)[0x9a / 2] = 0x1111;
    fx.sim_mut().data[1] = 10;
    let text = trap(generic_15(&mut fx, 0)).to_string();
    assert!(text.contains("facing 10"), "{text}");
    let sim = fx.sim();
    assert_eq!(sim.person.as_ref().unwrap().data[72..74], [216, 0x0102]);
    assert_eq!(sim.footprint, [1, 2, 3, 4]);
}

/// Runs Generic Sims Call routine 2 as `me`, with `stack` the stack
/// object.
fn generic_2(fx: &mut Fixture, me: u16, stack: i16) -> Result<bool, sims_vm::Fault> {
    let file = if me == SIM { SIM_FILE } else { LAMP_FILE };
    fx.add(
        file,
        4096,
        tree(vec![ins(
            1,
            RETURN_TRUE,
            RETURN_FALSE,
            [2, 0, 0, 0, 0, 0, 0, 0],
        )]),
    );
    fx.run_with(me, 4096, stack, &[], 10)
}

#[test]
fn generic_call_2_makes_the_stack_object_the_running_actions_icon() {
    let mut fx = Fixture::new();
    // With no running action (person +0xa84 is 0) nothing changes.
    assert_eq!(generic_2(&mut fx, SIM, LAMP as i16), Ok(true));
    assert_eq!(fx.sim().person.as_ref().unwrap().active.icon, 0);
    fx.sim_mut().person.as_mut().unwrap().active.uid = 7;
    assert_eq!(generic_2(&mut fx, SIM, LAMP as i16), Ok(true));
    assert_eq!(fx.sim().person.as_ref().unwrap().active.icon, LAMP as i16);
}

#[test]
fn generic_call_2_needs_a_stack_object_then_a_person() {
    let mut fx = Fixture::new();
    fx.sim_mut().person.as_mut().unwrap().active.uid = 7;
    // A stack object not on the lot: native error 0xa, checked first.
    for stack in [0, -3, 999] {
        assert_eq!(trap(generic_2(&mut fx, SIM, stack)).code(), Some(0xa));
        assert_eq!(trap(generic_2(&mut fx, LAMP, stack)).code(), Some(0xa));
    }
    // The lamp is not a person: native error 0x1c.
    assert_eq!(
        trap(generic_2(&mut fx, LAMP, SIM as i16)).code(),
        Some(0x1c)
    );
    assert_eq!(fx.sim().person.as_ref().unwrap().active.icon, 0);
}

#[test]
fn other_generic_calls_name_themselves() {
    let mut fx = Fixture::new();
    let text = trap(generic(&mut fx, 27)).to_string();
    assert!(text.contains("Generic Sims Call routine 27"), "{text}");
}

#[test]
fn generic_call_18_and_26_report_controlled_family_state() {
    use sims_sim::neighbors::Family;
    use sims_vm::Interpreter;

    let run_generic = |fx: &mut Fixture, routine: u8, active: Option<&[u32]>| {
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                1,
                RETURN_TRUE,
                RETURN_FALSE,
                [routine, 0, 0, 0, 0, 0, 0, 0],
            )]),
        );
        let mut interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        interpreter.active_family = active;
        interpreter.run_tree(&mut fx.world, LAMP, 4096, 0, &[], 10)
    };

    // Nonzero family in global 9 whose first member (SIM_GUID) is on the lot.
    let mut fx = Fixture::new();
    fx.world.globals[9] = 7;
    fx.world.families.push(Family {
        id: 7,
        house: 1,
        members: vec![SIM_GUID],
        friends: 0,
        stale: false,
    });
    assert_eq!(run_generic(&mut fx, 18, None), Ok(true));
    assert_eq!(fx.lamp().temps[0], SIM as i16);
    fx.lamp_mut().temps[0] = 99;
    fx.lamp_mut().temps[1] = 88;
    assert_eq!(run_generic(&mut fx, 26, None), Ok(true));
    assert_eq!((fx.lamp().temps[0], fx.lamp().temps[1]), (7, 1));

    // Member 0 off the lot: routine 18 writes -1, routine 26 still writes (family, 1).
    fx.world.people.clear();
    assert_eq!(run_generic(&mut fx, 18, None), Ok(true));
    assert_eq!(fx.lamp().temps[0], -1);
    assert_eq!(run_generic(&mut fx, 26, None), Ok(true));
    assert_eq!((fx.lamp().temps[0], fx.lamp().temps[1]), (7, 1));

    // Empty controlled family (global 9 == 0, active_family = Some(&[])):
    // 0x4e97dc(0) returns -1 so both routines leave temps untouched.
    let mut fx = Fixture::new();
    fx.world.globals[9] = 0;
    fx.lamp_mut().temps[0] = 42;
    fx.lamp_mut().temps[1] = 77;
    assert_eq!(run_generic(&mut fx, 18, Some(&[])), Ok(true));
    assert_eq!(fx.lamp().temps[0], 42);
    assert_eq!(run_generic(&mut fx, 26, Some(&[])), Ok(true));
    assert_eq!((fx.lamp().temps[0], fx.lamp().temps[1]), (42, 77));

    // Global 9 == 0 with active_family = Some(&[SIM_GUID]) on lot:
    // routine 18 writes SIM, routine 26 writes (SIM, 0).
    let members = [SIM_GUID];
    assert_eq!(run_generic(&mut fx, 18, Some(&members)), Ok(true));
    assert_eq!(fx.lamp().temps[0], SIM as i16);
    assert_eq!(run_generic(&mut fx, 26, Some(&members)), Ok(true));
    assert_eq!((fx.lamp().temps[0], fx.lamp().temps[1]), (SIM as i16, 0));

    // Global 9 == 0 with active_family = Some(&[SIM_GUID]) off lot:
    // routine 26 traps Unsupported (0x450764 dereferences null).
    fx.world.people.clear();
    assert!(matches!(
        trap(run_generic(&mut fx, 26, Some(&members))),
        Trap::Unsupported(_)
    ));

    // Global 9 == 0 with active_family = None traps Unsupported for both.
    assert!(matches!(
        trap(run_generic(&mut fx, 18, None)),
        Trap::Unsupported(_)
    ));
    assert!(matches!(
        trap(run_generic(&mut fx, 26, None)),
        Trap::Unsupported(_)
    ));
}

fn own(fx: &mut Fixture, object: u16, mode: u8) -> Result<bool, sims_vm::Fault> {
    let file = if object == LAMP { LAMP_FILE } else { SIM_FILE };
    fx.add(
        file,
        4096,
        tree(vec![ins(
            10,
            RETURN_TRUE,
            RETURN_FALSE,
            [mode, 9, 9, 9, 9, 9, 9, 9],
        )]),
    );
    fx.run_with(object, 4096, 0, &[], 10)
}

#[test]
fn event_ownership_is_taken_once_and_given_up_by_the_owner() {
    let mut fx = Fixture::new();
    // Nothing to give up.
    assert_eq!(own(&mut fx, LAMP, 1), Ok(false));
    assert_eq!(own(&mut fx, LAMP, 0), Ok(true));
    assert_eq!(fx.world.event_owner, LAMP);
    // Not even the owner can take it again.
    assert_eq!(own(&mut fx, LAMP, 0), Ok(false));
    assert_eq!(own(&mut fx, SIM, 0), Ok(false));
    assert_eq!(own(&mut fx, SIM, 1), Ok(false));
    assert_eq!(fx.world.event_owner, LAMP);
    assert_eq!(own(&mut fx, LAMP, 1), Ok(true));
    assert_eq!(fx.world.event_owner, 0);
    assert_eq!(own(&mut fx, SIM, 0), Ok(true));
    assert_eq!(fx.world.event_owner, SIM);
    assert_eq!(code(own(&mut fx, SIM, 2)), Some(0x35));
}

const MY_TYPE: u8 = 36;
const STACK_TYPE: u8 = 37;

/// A fixture whose lamp type has three shared words and whose Sim type
/// has none of its own.
fn typed() -> Fixture {
    let mut fx = Fixture::new();
    for file in [LAMP_FILE, SIM_FILE] {
        let words = fx.words(file);
        (words[92], words[93]) = (0, 0);
    }
    fx.world.type_attributes = vec![
        TypeAttributes {
            guid: 0x9999,
            values: vec![1],
        },
        TypeAttributes {
            guid: LAMP_GUID,
            values: vec![5, 6, 7],
        },
        // A second record for the same type is never reached.
        TypeAttributes {
            guid: LAMP_GUID,
            values: vec![8, 8, 8, 8],
        },
    ];
    fx
}

#[test]
fn type_attributes_are_shared_by_every_object_of_a_type() {
    let mut fx = typed();
    fx.add_object(5);
    assert_eq!(
        fx.expression(MY_TYPE, 1, op::EQUAL, scope::LITERAL, 6),
        Ok(true)
    );
    assert_eq!(
        fx.expression(MY_TYPE, 2, op::ASSIGN, scope::LITERAL, -9),
        Ok(true)
    );
    assert_eq!(fx.world.type_attributes[1].values, [5, 6, -9]);
    assert_eq!(fx.world.type_attributes[2].values, [8, 8, 8, 8]);
    // Another lamp sees the write, through either scope.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![expression(
            STACK_TYPE,
            2,
            op::EQUAL,
            scope::LITERAL,
            -9,
        )]),
    );
    assert_eq!(fx.run_with(5, 4096, LAMP as i16, &[], 10), Ok(true));
    // The index is bounded by the first record.
    assert_eq!(
        code(fx.expression(MY_TYPE, 3, op::EQUAL, scope::LITERAL, 0)),
        Some(0x40)
    );
    assert_eq!(
        code(fx.expression(MY_TYPE, -1, op::EQUAL, scope::LITERAL, 0)),
        Some(0x40)
    );
}

#[test]
fn a_type_with_no_record_has_no_words() {
    let mut fx = typed();
    // The Sim's type has no record.
    assert_eq!(
        code(fx.expression(STACK_TYPE, 0, op::EQUAL, scope::LITERAL, 0)),
        Some(0x40)
    );
    // A missing stack object.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![expression(
            STACK_TYPE,
            0,
            op::EQUAL,
            scope::LITERAL,
            0,
        )]),
    );
    assert_eq!(code(fx.run_with(LAMP, 4096, 40, &[], 10)), Some(10));
}

#[test]
fn a_definition_can_name_another_types_words() {
    let mut fx = typed();
    // The Sim's definition says to use the lamp's.
    let words = fx.words(SIM_FILE);
    (words[92], words[93]) = (LAMP_GUID as i16, 0);
    assert_eq!(
        fx.expression(STACK_TYPE, 0, op::EQUAL, scope::LITERAL, 5),
        Ok(true)
    );
}

#[test]
fn the_catalog_makes_one_record_per_definition_that_asks() {
    let mut fx = Fixture::new();
    fx.words(LAMP_FILE)[90] = 3;
    fx.words(LAMP_FILE)[91] = 1;
    fx.words(SIM_FILE)[90] = 0;
    let quiet = fx.add_type(0x4444, 4, 0, 0);
    fx.words(quiet)[90] = -2; // out of range: a record with no words
    let big = fx.add_type(0x5555, 4, 0, 0);
    fx.words(big)[90] = 32000;
    let records = fx.program.type_attributes(&fx.catalog).unwrap();
    let shape: Vec<_> = records.iter().map(|r| (r.guid, r.values.len())).collect();
    assert_eq!(shape, [(LAMP_GUID, 3), (0x4444, 0), (0x5555, 0)]);
    assert!(records.iter().all(|r| r.values.iter().all(|&v| v == 0)));
    // A type that wants its words cleared for each lot is not ported.
    fx.words(LAMP_FILE)[91] = 2;
    let error = fx.program.type_attributes(&fx.catalog).unwrap_err();
    assert!(error.to_string().contains("0x00001111"), "{error}");
}

/// Set To Next into temp 0 with a GUID.
fn next(fx: &mut Fixture, mode: u8, guid: u32, cursor: i16) -> Result<bool, sims_vm::Fault> {
    let [a, b, c, d] = guid.to_le_bytes();
    fx.lamp_mut().temps[0] = cursor;
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            31,
            RETURN_TRUE,
            RETURN_FALSE,
            [a, b, c, d, 0x80 | mode, scope::TEMPS, 0, 0],
        )]),
    );
    fx.run()
}

#[test]
fn mode_4_walks_the_instances_of_a_type() {
    let mut fx = Fixture::new();
    fx.add_object(9);
    fx.add_object(5);
    // The list is made on first use, in ID order.
    assert_eq!(next(&mut fx, 4, LAMP_GUID, 0), Ok(true));
    assert_eq!(fx.world.instances[&LAMP_GUID], [LAMP, 5, 9]);
    assert_eq!(fx.lamp().temps[0], LAMP as i16);
    assert_eq!(next(&mut fx, 4, LAMP_GUID, 5), Ok(true));
    assert_eq!(fx.lamp().temps[0], 9);
    // After the last, and for a cursor not in the list, the cursor becomes
    // zero.
    for cursor in [9, SIM as i16, 77] {
        assert_eq!(next(&mut fx, 4, LAMP_GUID, cursor), Ok(false));
        assert_eq!(fx.lamp().temps[0], 0);
    }
    // The list is not made again: an object that did not come through the
    // factory is not in it.
    fx.add_object(3);
    assert_eq!(next(&mut fx, 4, LAMP_GUID, LAMP as i16), Ok(true));
    assert_eq!(fx.lamp().temps[0], 5);
}

#[test]
fn mode_4_needs_one_definition_for_the_guid() {
    let mut fx = Fixture::new();
    // No definition: the false branch, the cursor kept, no list made.
    assert_eq!(next(&mut fx, 4, 0x7777, 6), Ok(false));
    assert_eq!(fx.lamp().temps[0], 6);
    assert!(fx.world.instances.is_empty());
    // A type with no instances gets an empty list.
    fx.add_type(0x4444, 4, 0, 0);
    assert_eq!(next(&mut fx, 4, 0x4444, 0), Ok(false));
    assert_eq!(fx.world.instances[&0x4444], [0u16; 0]);
    // Two definitions with one GUID.
    fx.add_type(0x4444, 4, 0, 0);
    assert!(matches!(
        trap(next(&mut fx, 4, 0x4444, 0)),
        Trap::Unsupported(_)
    ));
}

#[test]
fn mode_10_walks_the_career_list() {
    let mut fx = Fixture::new();
    fx.program.careers = Some(
        [4, -1, 0, 9]
            .map(|id| sims_vm::Career {
                id,
                levels: Vec::new(),
            })
            .into(),
    );
    let mut seen = Vec::new();
    let mut cursor = 0;
    // Zero is not in front of the list here: it is a career ID like any
    // other, so a zero cursor continues after it.
    while next(&mut fx, 10, 0, cursor).unwrap() {
        cursor = fx.lamp().temps[0];
        seen.push(cursor);
        assert!(seen.len() < 10);
    }
    assert_eq!(seen, [9]);
    assert_eq!(fx.lamp().temps[0], 9, "the cursor is kept at the end");
    // A cursor that is no career starts from the first, and a next ID of
    // zero is still the true branch.
    assert_eq!(next(&mut fx, 10, 0, 77), Ok(true));
    assert_eq!(fx.lamp().temps[0], 4);
    assert_eq!(next(&mut fx, 10, 0, 4), Ok(true));
    assert_eq!(fx.lamp().temps[0], -1);
    assert_eq!(next(&mut fx, 10, 0, -1), Ok(true));
    assert_eq!(fx.lamp().temps[0], 0);
}

/// Runs Generic Sims Call `routine` as the Sim, with `stack_object`, under
/// the view's `rotation`.
fn generic_as_sim(
    fx: &mut Fixture,
    routine: u8,
    stack_object: i16,
    rotation: u8,
) -> Result<bool, sims_vm::Fault> {
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![ins(
            1,
            RETURN_TRUE,
            RETURN_FALSE,
            [routine, 0, 0, 0, 0, 0, 0, 0],
        )]),
    );
    let mut interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.view_rotation = rotation;
    interpreter.run_tree(&mut fx.world, SIM, 4096, stack_object, &[], 10)
}

#[test]
fn generic_call_2_names_the_running_actions_icon() {
    // 0x44fb51: with an action running (person +0xa84), the stack object
    // becomes its icon (+0xa54).
    let mut fx = Fixture::new();
    fx.sim_mut().person.as_mut().unwrap().active.uid = 5;
    assert_eq!(generic_as_sim(&mut fx, 2, LAMP as i16, 0), Ok(true));
    assert_eq!(fx.sim().person.as_ref().unwrap().active.icon, LAMP as i16);
    // With none running, nothing changes.
    let mut fx = Fixture::new();
    let person = fx.sim_mut().person.as_mut().unwrap();
    (person.active.uid, person.active.icon) = (0, 9);
    assert_eq!(generic_as_sim(&mut fx, 2, LAMP as i16, 0), Ok(true));
    assert_eq!(fx.sim().person.as_ref().unwrap().active.icon, 9);
}

#[test]
fn generic_call_2_from_a_tree_checks_the_stack_object_then_the_person() {
    // The stack object is checked first (native error 0xa), then that the
    // caller is a person (0x1c).
    let mut fx = Fixture::new();
    for missing in [0, -1, 99] {
        assert_eq!(code(generic_as_sim(&mut fx, 2, missing, 0)), Some(0xa));
    }
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            1,
            RETURN_TRUE,
            RETURN_FALSE,
            [2, 0, 0, 0, 0, 0, 0, 0],
        )]),
    );
    assert_eq!(code(fx.run_with(LAMP, 4096, 0, &[], 10)), Some(0xa));
    assert_eq!(
        code(fx.run_with(LAMP, 4096, LAMP as i16, &[], 10)),
        Some(0x1c)
    );
}

#[test]
fn generic_call_12_puts_the_stack_objects_tile_depth_in_temp_0() {
    // 0x44fd4b: minus the sum of the tile's coordinates as the view's
    // rotation turns them (0x434f58). The lamp stands on tile (3, 5).
    let depth = |rotation: u8| {
        let mut fx = Fixture::new();
        fx.lamp_mut().position = [3 * 16 + 8, 5 * 16 + 8, 1];
        assert_eq!(generic_as_sim(&mut fx, 12, LAMP as i16, rotation), Ok(true));
        fx.sim().temps[0]
    };
    assert_eq!(depth(0), -8);
    assert_eq!(depth(1), -(63 - 5 + 3));
    assert_eq!(depth(2), -(60 + 58));
    assert_eq!(depth(3), -(5 + 60));
    // A missing stack object is native error 0xa.
    let mut fx = Fixture::new();
    assert_eq!(code(generic_as_sim(&mut fx, 12, 0, 0)), Some(0xa));
    // Off the rotation's table fails loudly; at rotation 0 nothing is read.
    let mut fx = Fixture::new();
    fx.lamp_mut().position = [-16, -16, 1];
    assert_eq!(generic_as_sim(&mut fx, 12, LAMP as i16, 0), Ok(true));
    assert_eq!(fx.sim().temps[0], 2);
    assert!(matches!(
        trap(generic_as_sim(&mut fx, 12, LAMP as i16, 1)),
        Trap::Unsupported(m) if m.contains("0x434f58")
    ));
}
