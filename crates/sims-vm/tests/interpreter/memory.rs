//! Operand scopes: reads, writes, read-only values and bounds errors.
//! The lamp runs; its stack object is the Sim.

use crate::support::*;
use sims_formats::career::Level;
use sims_vm::{Career, Fault, Trap};

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
    // Word 25 reads like any other; what resolving it does to a sleeping
    // object is in sleep.rs.
    assert!(reads(&mut fx, scope::STACK_DATA, 25, 50));
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
fn the_room_in_temp_0() {
    use sims_sim::rooms::Room;
    let mut fx = Fixture::new();
    let rooms = &mut fx.world.rooms.rooms;
    rooms.insert(
        3,
        Room {
            half_tiles: 19,
            ..Room::default()
        },
    );
    rooms.insert(
        1025,
        Room {
            outdoors: true,
            half_tiles: 8,
            ..Room::default()
        },
    );
    // Field 1 is whether the room is outdoors, 2 its level, 3 its area in
    // tiles: the half tiles divided by 2, rounding down.
    for (room, fields) in [(3, [0, 1, 9]), (1025, [1, 2, 4]), (-5, [1, 0, 0])] {
        fx.lamp_mut().temps[0] = room;
        for (field, expected) in (1..).zip(fields) {
            assert!(reads(&mut fx, 23, field, expected), "room {room}");
        }
    }
    // Room fields are read only.
    assert_eq!(code(write(&mut fx, 23, 1, 1)), Some(0x13));
    // An ID that names no room.
    fx.lamp_mut().temps[0] = 4;
    assert_eq!(code(read(&mut fx, 23, 1)), Some(0x28));
    // The other fields are not ported, and say so.
    fx.lamp_mut().temps[0] = 3;
    for field in [0, 4, -1] {
        let text = trap(read(&mut fx, 23, field)).to_string();
        assert!(text.contains(&format!("room field {field}")), "{text}");
    }
}

/// A level whose numbers count up from `base`, with `title` as its
/// second string.
fn level(base: i32, title: &str) -> Level {
    Level {
        values: std::array::from_fn(|i| base + i as i32),
        strings: std::array::from_fn(|i| if i == 1 { title.into() } else { String::new() }),
    }
}

/// Careers 4 (two levels) and -1 (one), my temps 0 and 1 naming the
/// career and the level.
fn jobs(career: i16, level_number: i16) -> Fixture {
    let mut fx = Fixture::new();
    fx.program.careers = Some(vec![
        Career {
            id: 4,
            levels: vec![level(100, "Cadet"), level(0x1_0200, "")],
        },
        Career {
            id: -1,
            levels: vec![level(-50, "")],
        },
    ]);
    fx.lamp_mut().temps[0] = career;
    fx.lamp_mut().temps[1] = level_number;
    fx
}

#[test]
fn job_data_reads_the_levels_numbers_in_the_originals_order() {
    let mut fx = jobs(4, 0);
    // Field 0 is the number of levels, whatever the level.
    assert!(reads(&mut fx, 33, 0, 2));
    let fields = [
        (1, 117),
        (2, 100),
        (11, 109),
        (12, 118),
        (13, 119),
        (14, 110),
        (20, 116),
        (21, 120),
        (22, 1),
    ];
    for (field, value) in fields {
        assert!(reads(&mut fx, 33, field, value), "field {field}");
    }
    // The second level: numbers are cut to a word, and its second string
    // is empty.
    let mut fx = jobs(4, 1);
    assert!(reads(&mut fx, 33, 2, 0x200));
    assert!(reads(&mut fx, 33, 22, 0));
    // It reads with no word pointer.
    assert_eq!(code(write(&mut fx, 33, 2, 1)), Some(0x13));
}

#[test]
fn career_minus_one_always_reads_its_first_level() {
    let mut fx = jobs(-1, 5);
    assert!(reads(&mut fx, 33, 2, -50));
    assert!(reads(&mut fx, 33, 0, 1));
}

#[test]
fn job_data_errors() {
    // No such career, even for field 0: the career is looked up first.
    assert_eq!(code(read(&mut jobs(3, 0), 33, 0)), Some(0x39));
    // No such level, unless the field is 0.
    for level in [2, -1] {
        let mut fx = jobs(4, level);
        assert_eq!(code(read(&mut fx, 33, 1)), Some(0x3a));
        assert!(reads(&mut fx, 33, 0, 2));
    }
    // No such field.
    for field in [23, -1] {
        assert_eq!(code(read(&mut jobs(4, 0), 33, field)), Some(0x3a));
    }
    // The lookup runs inside the operand resolver (0x4531fb returns
    // through its tail, 0x453483), before the caller finds there is no
    // word pointer to write: a write names the lookup's error, and only a
    // field that exists gives 0x13.
    assert_eq!(code(write(&mut jobs(3, 0), 33, 2, 1)), Some(0x39));
    assert_eq!(code(write(&mut jobs(4, 2), 33, 2, 1)), Some(0x3a));
    assert_eq!(code(write(&mut jobs(4, 0), 33, 23, 1)), Some(0x3a));
    assert_eq!(code(write(&mut jobs(4, 0), 33, 0, 1)), Some(0x13));
}

#[test]
fn job_data_without_a_career_table_names_the_loader() {
    // The fixture has no career table.
    let fault = read(&mut Fixture::new(), 33, 0).unwrap_err();
    let Trap::Unsupported(text) = &fault.trap else {
        panic!("{fault}");
    };
    assert!(
        text.contains("0x4531fb") && text.contains("0x4036e8"),
        "{text}"
    );
}

#[test]
fn the_advertisement_record_is_the_one_find_best_interaction_last_copied() {
    // Before the first copy the record holds what the game's start left
    // there, which is not read.
    for scope in [12, 28, 29] {
        let fault = read(&mut Fixture::new(), scope, 0).unwrap_err();
        let Trap::Unsupported(text) = &fault.trap else {
            panic!("{fault}");
        };
        assert!(text.contains("0x6e5100"), "{text}");
    }
    // After it, any tree reads and writes the record: the personality
    // number (28), the minimum (29) and the delta (12), sixteen of them.
    let mut fx = Fixture::new();
    fx.world.advertisement_record = Some(std::array::from_fn(|i| sims_sim::Advertisement {
        personality: i as i16,
        minimum: 10 + i as i16,
        delta: 20 + i as i16,
    }));
    assert!(reads(&mut fx, 28, 3, 3));
    assert!(reads(&mut fx, 29, 3, 13));
    assert!(reads(&mut fx, 12, 15, 35));
    assert_eq!(write(&mut fx, 12, 2, -4), Ok(true));
    assert_eq!(fx.world.advertisement_record.unwrap()[2].delta, -4);
    for scope in [12, 28, 29] {
        assert_eq!(code(read(&mut fx, scope, 16)), Some(0x1a));
        assert_eq!(code(read(&mut fx, scope, -1)), Some(0x1a));
    }
}

#[test]
fn unsupported_scopes_name_themselves() {
    for scope in [24, 34, 38] {
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
