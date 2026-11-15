//! Find Best Object for Function (opcode 14): the object that serves a
//! function best, by its score less three times its distance.

use crate::support::*;
use sims_formats::execution::ObjectFunction;
use sims_sim::rooms::{NO_ROOM, Room};

const WIDGET: u32 = 0x7777;
const WIDGET_TYPE: u16 = 30;
/// The check tree that passes, the one that fails, and the one that
/// zeroes the running object's score.
const PASS: u16 = 0x1001;
const FAIL: u16 = 0x1002;
const ZERO: u16 = 0x1003;
/// A check tree the widget's file lacks.
const MISSING: u16 = 0x1004;
/// The widget's Main, which sleeps.
const MAIN: u16 = 0x1006;
/// The running object's data word that takes each score.
const SCORE: usize = 66;

/// The lamp (ID 1, at the origin) and the Sim, both in use so the walk
/// skips them, and a widget type whose callbacks 0x0e to 0x1d have an
/// action and no check tree.
fn lot() -> (Fixture, usize) {
    let mut fx = Fixture::new();
    let file = fx.add_type(WIDGET, 4, 0, MAIN);
    fx.add(
        file,
        MAIN,
        tree(vec![
            assign(scope::PARAMETERS, 0, scope::LITERAL, 100, 1),
            ins(0, 0, 0, [0; 8]),
        ]),
    );
    fx.world.types.insert(WIDGET_TYPE, WIDGET);
    let table = fx
        .program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap();
    table.resize(
        0x20,
        ObjectFunction {
            condition: 0,
            action: 0,
        },
    );
    for function in &mut table[0x0e..0x1e] {
        function.action = 0x1000;
    }
    fx.add(
        file,
        PASS,
        tree(vec![expression(
            scope::LITERAL,
            1,
            op::EQUAL,
            scope::LITERAL,
            1,
        )]),
    );
    fx.add(
        file,
        FAIL,
        tree(vec![expression(
            scope::LITERAL,
            1,
            op::EQUAL,
            scope::LITERAL,
            2,
        )]),
    );
    fx.add(
        file,
        ZERO,
        tree(vec![assign(
            scope::MY_DATA,
            SCORE as i16,
            scope::LITERAL,
            0,
            RETURN_TRUE,
        )]),
    );
    for id in [LAMP, SIM] {
        let object = fx.world.objects.get_mut(&id).unwrap();
        object.position = [0, 0, 1];
        object.data[25] = 1;
    }
    (fx, file)
}

/// Adds a widget `tiles` east of the lamp, out of use and in no room, whose
/// data word `attribute` holds `score`.
fn widget(fx: &mut Fixture, id: u16, tiles: i32, attribute: usize, score: i16) {
    let object = fx.add_object(id);
    object.guid = WIDGET;
    object.position = [16 * tiles, 0, 1];
    object.data[8] = 0;
    object.data[25] = 0;
    object.data[29] = NO_ROOM;
    object.data[59] = 0;
    object.data[attribute] = score;
}

fn set_check(fx: &mut Fixture, file: usize, callback: usize, tree: u16) {
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap()[callback]
        .condition = tree;
}

/// The instruction for `function` with flag byte `flags`, and the origin
/// (with flag 8) in temp `temp`.
fn find(function: i16, flags: u8, temp: i16) -> [u8; 8] {
    let [f0, f1] = function.to_le_bytes();
    let [t0, t1] = temp.to_le_bytes();
    [f0, f1, t0, t1, scope::TEMPS, 0, flags, 0]
}

/// Runs the instruction as the lamp's tree; the branch, and the stack
/// object the tree leaves.
fn run(fx: &mut Fixture, operands: [u8; 8]) -> Result<(bool, i16), sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            ins(14, 1, 2, operands),
            assign(scope::TEMPS, 7, scope::STACK_OBJECT, 0, RETURN_TRUE),
            assign(scope::TEMPS, 7, scope::STACK_OBJECT, 0, RETURN_FALSE),
        ]),
    );
    let branch = fx.run_with(LAMP, 4096, 0, &[], 100)?;
    Ok((branch, fx.lamp().temps[7]))
}

#[test]
fn the_best_score_less_three_times_the_distance_wins() {
    let (mut fx, _) = lot();
    // Function 0 scores data word 31: 10 - 3 at one tile beats 12 - 6 at
    // two.
    widget(&mut fx, 3, 1, 31, 10);
    widget(&mut fx, 4, 2, 31, 12);
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((true, 3)));
    // Each score went into my data word 66; the last stays.
    assert_eq!(fx.lamp().data[SCORE], 12);
    // A tie keeps the first in registration order, not the lowest ID.
    fx.world.objects.get_mut(&4).unwrap().data[31] = 13;
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((true, 3)));
    let at = |fx: &Fixture, id| fx.world.order.iter().position(|&o| o == id).unwrap();
    let (three, four) = (at(&fx, 3), at(&fx, 4));
    fx.world.order.swap(three, four);
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((true, 4)));
    // Far away, a score counts as 1, which still beats none.
    let (mut fx, _) = lot();
    widget(&mut fx, 3, 40, 31, 10);
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((true, 3)));
}

#[test]
fn function_11_halves_distances_past_one_tile() {
    // 20 at four tiles: 20 - 3 * 2 = 14 with halving beats 10 here, and
    // 20 - 12 = 8 without does not.
    let (mut fx, _) = lot();
    widget(&mut fx, 3, 4, 39, 20);
    widget(&mut fx, 4, 0, 39, 10);
    assert_eq!(run(&mut fx, find(11, 0, 0)), Ok((true, 3)));
    let (mut fx, _) = lot();
    widget(&mut fx, 3, 4, 31, 20);
    widget(&mut fx, 4, 0, 31, 10);
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((true, 4)));
}

#[test]
fn objects_in_use_disabled_without_an_action_or_scoring_zero_are_skipped() {
    let (mut fx, file) = lot();
    widget(&mut fx, 3, 0, 31, 10);
    fx.world.objects.get_mut(&3).unwrap().data[25] = 1;
    widget(&mut fx, 4, 0, 31, 10);
    fx.world.objects.get_mut(&4).unwrap().data[8] = 0x200;
    widget(&mut fx, 5, 0, 31, 0);
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((false, 0)));
    // The zero score was still written.
    assert_eq!(fx.lamp().data[SCORE], 0);
    // No action for the callback.
    widget(&mut fx, 6, 0, 31, 10);
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap()[0x12]
        .action = 0;
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((false, 0)));
}

#[test]
fn functions_4_5_9_and_past_15_find_nothing() {
    let (mut fx, _) = lot();
    // Every other data word of the widget is nonzero, so every attribute
    // scores.
    widget(&mut fx, 3, 0, 31, 10);
    for function in [4, 5, 9, 16, -1] {
        assert_eq!(run(&mut fx, find(function, 0, 0)), Ok((false, 0)));
    }
    assert_eq!(run(&mut fx, find(14, 0, 0)), Ok((true, 3)));
}

#[test]
fn a_check_tree_that_fails_or_zeroes_the_score_skips_the_object() {
    let (mut fx, file) = lot();
    widget(&mut fx, 3, 0, 31, 10);
    widget(&mut fx, 4, 1, 31, 10);
    set_check(&mut fx, file, 0x12, FAIL);
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((false, 0)));
    set_check(&mut fx, file, 0x12, ZERO);
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((false, 0)));
    set_check(&mut fx, file, 0x12, MISSING);
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((false, 0)));
    set_check(&mut fx, file, 0x12, PASS);
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((true, 3)));
}

#[test]
fn a_check_tree_that_changes_the_object_list_traps_even_when_it_fails() {
    const CREATE: u16 = 0x1005;
    let (mut fx, file) = lot();
    widget(&mut fx, 3, 0, 31, 10);
    // Make a widget off the lot (Create New Object Instance mode 6), which
    // heads the object list, and give false.
    let [g0, g1, g2, g3] = WIDGET.to_le_bytes();
    fx.add(
        file,
        CREATE,
        tree(vec![
            ins(42, 1, 1, [g0, g1, g2, g3, 6, 0, 0, 0]),
            expression(scope::LITERAL, 1, op::EQUAL, scope::LITERAL, 2),
        ]),
    );
    set_check(&mut fx, file, 0x12, CREATE);
    let fault = run(&mut fx, find(0, 0, 0)).unwrap_err();
    assert!(
        matches!(fault.trap, sims_vm::Trap::Unsupported(ref m) if m.contains("object list")),
        "{fault}"
    );
}

#[test]
fn an_unordered_score_counts_as_1() {
    // The original's `fcom` and `jae` clamp a NaN to 1, which beats none.
    let (mut fx, _) = lot();
    widget(&mut fx, 3, 1, 31, 10);
    let global = fx.program.libraries.get_mut(&GLOBAL).unwrap();
    // The global file's autonomy tuning (constants 2).
    global.tuning.insert(
        2,
        sims_formats::tuning::Constants {
            version: 0,
            entries: vec![sims_formats::tuning::Constant {
                name: "functional distance attenuation".into(),
                value: f32::NAN,
                comment: String::new(),
            }],
        },
    );
    assert_eq!(run(&mut fx, find(0, 0, 0)), Ok((true, 3)));
}

#[test]
fn function_8_scores_50_and_asks_a_person_whose_word_65_is_at_most_1() {
    let (mut fx, _) = lot();
    widget(&mut fx, 3, 1, 31, 0);
    // The lamp is not a person.
    assert_eq!(run(&mut fx, find(8, 0, 0)), Ok((false, 0)));
    let program = tree(vec![ins(14, RETURN_TRUE, RETURN_FALSE, find(8, 0, 0))]);
    fx.add(SIM_FILE, 4096, program);
    fx.sim_mut().person.as_mut().unwrap().data[65] = 2;
    assert_eq!(fx.run_with(SIM, 4096, 0, &[], 100), Ok(false));
    fx.sim_mut().person.as_mut().unwrap().data[65] = 1;
    assert_eq!(fx.run_with(SIM, 4096, 0, &[], 100), Ok(true));
    assert_eq!(fx.sim().data[SCORE], 50);
}

#[test]
fn function_15_takes_objects_of_category_4() {
    let (mut fx, _) = lot();
    widget(&mut fx, 3, 0, 31, 0);
    widget(&mut fx, 4, 1, 31, 0);
    fx.world.objects.get_mut(&4).unwrap().data[59] = 4;
    assert_eq!(run(&mut fx, find(15, 0, 0)), Ok((true, 4)));
    assert_eq!(fx.lamp().data[SCORE], 50);
}

#[test]
fn flags_1_2_and_4_choose_by_room() {
    let (mut fx, _) = lot();
    let room = |outdoors| Room {
        outdoors,
        ..Room::default()
    };
    fx.world.rooms.rooms.insert(5, room(true));
    fx.world.rooms.rooms.insert(6, room(false));
    // Object 3 has no room, 4 is outdoors, 5 is indoors, 6 is in a room
    // the lot does not have. Nearer is better.
    for (id, at) in [(3, NO_ROOM), (4, 5), (5, 6), (6, 9)] {
        widget(&mut fx, id, i32::from(id), 31, 30);
        fx.world.objects.get_mut(&id).unwrap().data[29] = at;
    }
    let walk = |fx: &mut Fixture, flags| {
        let mut found = Vec::new();
        for _ in 0..4 {
            match run(fx, find(0, flags, 0)).unwrap() {
                (true, id) => {
                    found.push(id);
                    fx.world.objects.get_mut(&(id as u16)).unwrap().data[25] = 1;
                }
                (false, _) => break,
            }
        }
        for id in 3..=6 {
            fx.world.objects.get_mut(&id).unwrap().data[25] = 0;
        }
        found
    };
    // Outdoors: no room, an outdoor room, or one the lot lacks.
    assert_eq!(walk(&mut fx, 1), [3, 4, 6]);
    // Indoors: a room that is not outdoors, or one the lot lacks.
    assert_eq!(walk(&mut fx, 2), [5, 6]);
    // Flag 1 wins over flag 2, and both over flag 4.
    assert_eq!(walk(&mut fx, 3), [3, 4, 6]);
    // My room.
    fx.lamp_mut().data[29] = 6;
    assert_eq!(walk(&mut fx, 4), [5]);
    assert_eq!(walk(&mut fx, 6), [5, 6]);
}

#[test]
fn flag_8_measures_from_the_object_a_variable_names() {
    let (mut fx, _) = lot();
    widget(&mut fx, 3, 1, 31, 10);
    widget(&mut fx, 4, 9, 31, 10);
    // From object 4's place, object 4 is nearest.
    fx.lamp_mut().temps[2] = 4;
    assert_eq!(run(&mut fx, find(0, 8, 2)), Ok((true, 4)));
    // With flag 4 too, the room is object 4's: object 3, in my room and
    // scoring enough to win without the flag, is not in it.
    fx.world.objects.get_mut(&3).unwrap().data[31] = 100;
    fx.world.objects.get_mut(&3).unwrap().data[29] = 8;
    fx.world.objects.get_mut(&4).unwrap().data[29] = 7;
    fx.lamp_mut().data[29] = 8;
    assert_eq!(run(&mut fx, find(0, 8, 2)), Ok((true, 3)));
    assert_eq!(run(&mut fx, find(0, 12, 2)), Ok((true, 4)));
    // No object with the ID: native error 0x17.
    fx.lamp_mut().temps[2] = 99;
    let fault = run(&mut fx, find(0, 8, 2)).unwrap_err();
    assert_eq!(fault.trap.code(), Some(0x17));
}
