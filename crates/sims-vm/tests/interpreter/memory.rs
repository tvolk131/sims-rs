//! Operand scopes: reads, writes, read-only values and bounds errors.
//! The lamp runs; its stack object is the Sim.

use crate::support::*;
use sims_vm::{Fault, Trap};

const EQUAL: u8 = op::EQUAL;
const ASSIGN: u8 = op::ASSIGN;
const LITERAL: u8 = scope::LITERAL;

/// True when the operand equals `expected`.
fn reads(fx: &mut Fixture, scope: u8, index: i16, expected: i16) -> bool {
    fx.expression(scope, index, EQUAL, LITERAL, expected)
        .unwrap_or_else(|f| panic!("{f}"))
}

fn write(fx: &mut Fixture, scope: u8, index: i16, value: i16) -> Result<bool, Fault> {
    fx.expression(scope, index, ASSIGN, LITERAL, value)
}

fn read(fx: &mut Fixture, scope: u8, index: i16) -> Result<bool, Fault> {
    fx.expression(scope, index, EQUAL, LITERAL, 0)
}

#[test]
fn my_attributes() {
    let mut fx = Fixture::new();
    assert!(reads(&mut fx, scope::MY_ATTRIBUTES, 2, 12));
    assert_eq!(write(&mut fx, scope::MY_ATTRIBUTES, 1, -7), Ok(true));
    assert_eq!(fx.lamp().attributes, [10, -7, 12]);
    assert_eq!(code(read(&mut fx, scope::MY_ATTRIBUTES, 3)), Some(6));
    assert_eq!(code(read(&mut fx, scope::MY_ATTRIBUTES, -1)), Some(6));
}

#[test]
fn stack_object_attributes() {
    let mut fx = Fixture::new();
    assert!(reads(&mut fx, scope::STACK_ATTRIBUTES, 1, 21));
    assert_eq!(write(&mut fx, scope::STACK_ATTRIBUTES, 0, 5), Ok(true));
    assert_eq!(fx.sim().attributes, [5, 21]);
    assert_eq!(code(read(&mut fx, scope::STACK_ATTRIBUTES, 2)), Some(6));
    // A missing stack object.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![expression(
            scope::STACK_ATTRIBUTES,
            0,
            EQUAL,
            LITERAL,
            0,
        )]),
    );
    assert_eq!(code(fx.run_with(LAMP, 4096, 40, &[], 10)), Some(10));
}

#[test]
fn invalid_scopes() {
    for scope in [2, 5, 39, 200, 255] {
        let mut fx = Fixture::new();
        assert_eq!(code(read(&mut fx, scope, 0)), Some(9), "scope {scope}");
    }
}

#[test]
fn object_data() {
    let mut fx = Fixture::new();
    assert!(reads(&mut fx, scope::MY_DATA, 71, 142));
    assert!(reads(&mut fx, scope::STACK_DATA, 5, 10));
    assert_eq!(write(&mut fx, scope::MY_DATA, 40, 1), Ok(true));
    assert_eq!(write(&mut fx, scope::STACK_DATA, 41, 2), Ok(true));
    assert_eq!((fx.lamp().data[40], fx.sim().data[41]), (1, 2));
    assert_eq!(code(read(&mut fx, scope::MY_DATA, 72)), Some(7));
    assert_eq!(code(read(&mut fx, scope::STACK_DATA, -1)), Some(7));
    // Word 11 (the object's ID) reads, but writes to it, and to the
    // container words 2 and 3, need native side effects.
    assert!(reads(&mut fx, scope::MY_DATA, 11, 22));
    for word in [2, 3, 11] {
        assert!(matches!(
            trap(write(&mut fx, scope::MY_DATA, word, 0)),
            Trap::Unsupported(_)
        ));
    }
    // Word 25 wakes the object, which needs the scheduler.
    assert!(matches!(
        trap(read(&mut fx, scope::STACK_DATA, 25)),
        Trap::Unsupported(_)
    ));
}

#[test]
fn globals_are_read_only_except_34_and_36() {
    let mut fx = Fixture::new();
    assert!(reads(&mut fx, scope::GLOBAL, 5, 105));
    assert_eq!(code(write(&mut fx, scope::GLOBAL, 5, 1)), Some(0x13));
    assert_eq!(fx.world.globals[5], 105);
    assert_eq!(write(&mut fx, scope::GLOBAL, 34, 1), Ok(true));
    assert_eq!(write(&mut fx, scope::GLOBAL, 36, 2), Ok(true));
    assert_eq!((fx.world.globals[34], fx.world.globals[36]), (1, 2));
    assert_eq!(code(read(&mut fx, scope::GLOBAL, 64)), Some(0x3f));
    // A lot saved with 32 globals has no global 40.
    fx.world.globals.truncate(32);
    assert!(matches!(
        trap(read(&mut fx, scope::GLOBAL, 40)),
        Trap::Invalid(_)
    ));
}

#[test]
fn literals_read_but_take_no_writes() {
    let mut fx = Fixture::new();
    assert!(reads(&mut fx, LITERAL, -300, -300));
    assert_eq!(code(write(&mut fx, LITERAL, 3, 1)), Some(0x13));
}

#[test]
fn temps() {
    let mut fx = Fixture::new();
    assert_eq!(write(&mut fx, scope::TEMPS, 7, 3), Ok(true));
    assert!(reads(&mut fx, scope::TEMPS, 7, 3));
    assert_eq!(fx.lamp().temps[7], 3);
    assert_eq!(code(read(&mut fx, scope::TEMPS, 8)), Some(2));
}

#[test]
fn parameters_and_locals() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            ins(2, 1, 1, expr(scope::PARAMETERS, 3, ASSIGN, LITERAL, 9)),
            ins(
                2,
                2,
                2,
                expr(scope::LOCALS, 3, ASSIGN, scope::PARAMETERS, 3),
            ),
            ins(2, 3, 3, expr(scope::MY_DATA, 40, ASSIGN, scope::LOCALS, 3)),
            expression(scope::PARAMETERS, 0, EQUAL, LITERAL, 6),
        ]),
    );
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[6], 10), Ok(true));
    assert_eq!(fx.lamp().data[40], 9);
    assert_eq!(code(read(&mut fx, scope::PARAMETERS, 4)), Some(8));
    assert_eq!(code(read(&mut fx, scope::LOCALS, 4)), Some(8));
    assert_eq!(code(read(&mut fx, scope::LOCALS, -1)), Some(8));
}

#[test]
fn stack_object_id() {
    let mut fx = Fixture::new();
    assert!(reads(&mut fx, scope::STACK_OBJECT, 0, SIM as i16));
    assert_eq!(code(read(&mut fx, scope::STACK_OBJECT, 1)), Some(0xf));
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            ins(
                2,
                1,
                1,
                expr(scope::STACK_OBJECT, 0, ASSIGN, LITERAL, LAMP as i16),
            ),
            // The stack object's attributes are now the lamp's.
            expression(1, 2, EQUAL, LITERAL, 12),
        ]),
    );
    assert_eq!(fx.run(), Ok(true));
}

#[test]
fn temp_indexed_by_temp() {
    let mut fx = Fixture::new();
    fx.lamp_mut().temps[0] = 3;
    fx.lamp_mut().temps[3] = 44;
    assert!(reads(&mut fx, 11, 0, 44));
    assert_eq!(write(&mut fx, 11, 0, 45), Ok(true));
    assert_eq!(fx.lamp().temps[3], 45);
    // The index must be a temp, and the temp's value a temp index.
    assert_eq!(code(read(&mut fx, 11, 8)), Some(2));
    fx.lamp_mut().temps[0] = 8;
    assert_eq!(code(read(&mut fx, 11, 0)), Some(2));
}

#[test]
fn stack_object_temps() {
    let mut fx = Fixture::new();
    assert_eq!(write(&mut fx, 13, 2, 8), Ok(true));
    assert_eq!(fx.sim().temps[2], 8);
    assert!(reads(&mut fx, 13, 2, 8));
    assert_eq!(code(read(&mut fx, 13, 8)), Some(2));
}

#[test]
fn motives() {
    let mut fx = Fixture::new();
    // My motives: the lamp is not a person.
    assert_eq!(code(read(&mut fx, 14, 0)), Some(0x1c));
    // Stack object's motives: motive n is 10n - 50.
    assert!(reads(&mut fx, 15, 7, 20));
    assert_eq!(code(read(&mut fx, 15, 16)), Some(0x1a));
    // Indexed by temp.
    fx.lamp_mut().temps[1] = 2;
    assert!(reads(&mut fx, 17, 1, -30));
    fx.lamp_mut().temps[1] = 16;
    assert_eq!(code(read(&mut fx, 17, 1)), Some(0x1a));
    // Reads round half to even.
    let motives = &mut fx.sim_mut().person.as_mut().unwrap().motives;
    motives[0] = 2.5;
    motives[1] = 3.5;
    motives[2] = -2.5;
    motives[3] = 2.75;
    assert!(reads(&mut fx, 15, 0, 2));
    assert!(reads(&mut fx, 15, 1, 4));
    assert!(reads(&mut fx, 15, 2, -2));
    assert!(reads(&mut fx, 15, 3, 3));
    // A non-finite motive needs an unverified conversion.
    fx.sim_mut().person.as_mut().unwrap().motives[4] = f32::INFINITY;
    assert!(matches!(trap(read(&mut fx, 15, 4)), Trap::Unsupported(_)));
}

#[test]
fn slots_are_read_only() {
    let mut fx = Fixture::new();
    // Script slot 0 is saved slot 1.
    assert!(reads(&mut fx, 20, 0, SIM as i16));
    assert!(reads(&mut fx, 20, 1, 0));
    assert_eq!(code(read(&mut fx, 20, 2)), Some(0x1b));
    assert!(reads(&mut fx, 16, 0, SIM as i16));
    assert_eq!(code(write(&mut fx, 16, 0, 1)), Some(0x13));
}

#[test]
fn person_data() {
    let mut fx = Fixture::new();
    // My person data: the lamp is not a person.
    assert_eq!(code(read(&mut fx, 18, 0)), Some(0x1c));
    assert!(reads(&mut fx, 19, 255, 765));
    assert_eq!(write(&mut fx, 19, 10, -1), Ok(true));
    assert_eq!(fx.sim().person.as_ref().unwrap().data[10], -1);
    assert_eq!(code(read(&mut fx, 19, 256)), Some(0x21));
    assert_eq!(code(read(&mut fx, 19, -1)), Some(0x21));
    // An older save keeps 80 words.
    fx.sim_mut().person.as_mut().unwrap().data.truncate(80);
    assert!(matches!(trap(read(&mut fx, 19, 100)), Trap::Invalid(_)));
}

#[test]
fn person_data_indexed_by_temp() {
    let mut fx = Fixture::new();
    fx.lamp_mut().temps[4] = 6;
    assert!(reads(&mut fx, 31, 4, 18));
    assert_eq!(write(&mut fx, 31, 4, 1), Ok(true));
    assert_eq!(fx.sim().person.as_ref().unwrap().data[6], 1);
    assert_eq!(code(read(&mut fx, 31, 8)), Some(2));
    assert_eq!(code(read(&mut fx, 30, 4)), Some(0x1c));
    // The Sim runs, with itself as stack object.
    let mut fx = Fixture::new();
    fx.sim_mut().temps[0] = 2;
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![expression(30, 0, EQUAL, LITERAL, 6)]),
    );
    assert_eq!(fx.run_with(SIM, 4096, SIM as i16, &[], 10), Ok(true));
}

#[test]
fn stack_object_attribute_indexed_by_parameter() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            ins(2, 1, 1, expr(22, 0, ASSIGN, LITERAL, 33)),
            expression(22, 1, EQUAL, LITERAL, 33),
        ]),
    );
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[1, 1], 10), Ok(true));
    assert_eq!(fx.sim().attributes[1], 33);
    // The parameter must exist, and its value index an attribute.
    assert_eq!(code(fx.run_with(LAMP, 4096, SIM as i16, &[2], 10)), Some(6));
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![expression(22, 4, EQUAL, LITERAL, 0)]),
    );
    assert_eq!(code(fx.run()), Some(8));
}

#[test]
fn tuning_constants() {
    let mut fx = Fixture::new();
    // The owner's own table 4096, its semi-global 8192, the global 256.
    assert!(reads(&mut fx, 26, 1, -5));
    assert!(reads(&mut fx, 26, 64 << 7, 7));
    assert!(reads(&mut fx, 26, (128 << 7) | 1, 301));
    assert!(matches!(trap(read(&mut fx, 26, 2)), Trap::Invalid(_)));
    assert!(matches!(trap(read(&mut fx, 26, -1)), Trap::Invalid(_)));
    assert_eq!(code(write(&mut fx, 26, 0, 1)), Some(0x13));
}

#[test]
fn dynamic_sprite_flags_indexed_by_temp() {
    let mut fx = Fixture::new();
    fx.lamp_mut().temps[2] = 1;
    assert!(reads(&mut fx, 27, 2, 6));
    assert_eq!(write(&mut fx, 27, 2, 0), Ok(true));
    assert_eq!(fx.sim().dynamic_sprites, [5, 0]);
    fx.lamp_mut().temps[2] = 2;
    assert_eq!(code(read(&mut fx, 27, 2)), Some(0x31));
    assert_eq!(code(read(&mut fx, 27, 8)), Some(2));
}

#[test]
fn stack_object_callbacks() {
    let mut fx = Fixture::new();
    // The lamp's callback table is [4096, 0, 4097].
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![expression(35, 0, EQUAL, LITERAL, 1)]),
    );
    assert_eq!(fx.run_with(LAMP, 4096, LAMP as i16, &[], 10), Ok(true));
    for (index, expected) in [(1, 0), (2, 1), (3, 0), (-1, 0)] {
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![expression(35, index, EQUAL, LITERAL, expected)]),
        );
        assert_eq!(fx.run_with(LAMP, 4096, LAMP as i16, &[], 10), Ok(true));
    }
    assert_eq!(code(write(&mut fx, 35, 0, 1)), Some(0x13));
}

#[test]
fn stack_object_definition() {
    let mut fx = Fixture::new();
    // The Sim's definition: type 2 in word 9, its GUID in words 14 and 15,
    // and 1000 + i in the other words.
    assert!(reads(&mut fx, 21, 9, 2));
    assert!(reads(&mut fx, 21, 14, SIM_GUID as u16 as i16));
    assert!(reads(&mut fx, 21, 0, 1000));
    assert!(reads(&mut fx, 21, 107, 1107));
    assert_eq!(code(read(&mut fx, 21, 108)), Some(0x23));
    assert_eq!(code(read(&mut fx, 21, -1)), Some(0x23));
    // Definition words are read only.
    assert_eq!(code(write(&mut fx, 21, 0, 1)), Some(0x13));
    // The index is checked before the stack object.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![expression(21, 0, EQUAL, LITERAL, 0)]),
    );
    assert_eq!(code(fx.run_with(LAMP, 4096, 40, &[], 10)), Some(10));
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![expression(21, 108, EQUAL, LITERAL, 0)]),
    );
    assert_eq!(code(fx.run_with(LAMP, 4096, 40, &[], 10)), Some(0x23));
    // The lamp's own definition, as stack object.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![expression(21, 9, EQUAL, LITERAL, 4)]),
    );
    assert_eq!(fx.run_with(LAMP, 4096, LAMP as i16, &[], 10), Ok(true));
}

#[test]
fn unsupported_scopes_name_themselves() {
    for scope in [12, 23, 24, 28, 29, 32, 33, 34, 36, 37, 38] {
        let mut fx = Fixture::new();
        let fault = read(&mut fx, scope, 0).unwrap_err();
        assert_eq!(fault.trap, Trap::UnsupportedScope { scope });
        let text = fault.to_string();
        assert!(
            text.contains(&format!(
                "unsupported scope {scope} ({})",
                sims_vm::scope_name(scope)
            )),
            "{text}"
        );
    }
    assert_eq!(sims_vm::scope_name(24), "neighbor in stack object");
}
