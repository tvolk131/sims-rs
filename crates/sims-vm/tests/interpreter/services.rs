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
    assert_eq!(fx.world.hash(), expected.hash());
    fx.world.magic_history = Some(MagicHistory {
        family: 5,
        values: [0; 6],
    });
    assert!(matches!(trap(generic(&mut fx, 43)), Trap::Unsupported(_)));
}

#[test]
fn other_generic_calls_name_themselves() {
    let mut fx = Fixture::new();
    let text = trap(generic(&mut fx, 27)).to_string();
    assert!(text.contains("Generic Sims Call routine 27"), "{text}");
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
    fx.program.careers = Some(vec![4, -1, 0, 9]);
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
