//! A person chooses an interaction by itself: Find Best Interaction
//! (primitive 3, 0x45dd39) scores the lot's interaction table entries
//! against the person's motives and picks one, and Gosub Found Action
//! (primitive 30, 0x45a859) starts it.

use crate::support::*;
use sims_formats::behavior::{InteractionTable, MotiveAdvertisement};
use sims_vm::{Interpreter, Status};

/// The Sim's tree: Find Best Interaction, then Gosub Found Action.
const CHOOSE: u16 = 4096;
/// The lamp's action tree: sets its first argument to 1 and sleeps on it.
const ACTION: u16 = 4200;
/// The lamp's check tree, which returns false.
const CHECK: u16 = 4201;
/// The lamp's interaction table (definition word 7, which counts up from
/// 1000 in the synthetic definitions).
const TABLE: u16 = 1007;

/// An entry with menu number `menu` whose advertisement raises energy
/// (motive 5) by `energy` thousandths, with the entry's own attenuation
/// 0.25 (code 0) and autonomy threshold 50.
fn entry(menu: u32, energy: i16, flags: u32) -> sims_formats::behavior::Interaction {
    let mut motives = vec![
        MotiveAdvertisement {
            minimum: 0,
            delta: 0,
            personality: 0,
        };
        16
    ];
    motives[5].delta = energy;
    sims_formats::behavior::Interaction {
        action: ACTION,
        test: 0,
        string_index: menu,
        flags,
        autonomy: 50,
        attenuation_code: 0,
        attenuation: 0.25,
        joining_index: -1,
        motives,
        flags2: 0,
    }
}

/// A lot where the Sim may choose one of the lamp's entries: menu 3,
/// which adds 10 to energy, and menu 5, which adds 20. Every score curve
/// is the line through (-100;-100) and (100;100), so a score is the mean
/// of the nine scored motives, and those entries improve it by 10/9 and
/// 20/9, above the family threshold of 0.2 (the default; the lot has no
/// tuning table 2). Menu 5 has flag 0x100, so it is taken without a draw
/// when it is the best.
fn lot(first: u32) -> Fixture {
    let mut fx = Fixture::new();
    fx.add(
        SIM_FILE,
        CHOOSE,
        tree(vec![
            ins(3, 1, RETURN_FALSE, [0; 8]),
            ins(30, RETURN_TRUE, RETURN_FALSE, [0; 8]),
        ]),
    );
    fx.add(
        LAMP_FILE,
        ACTION,
        tree(vec![
            assign(scope::PARAMETERS, 0, scope::LITERAL, 1, 1),
            ins(0, RETURN_TRUE, RETURN_FALSE, [0; 8]),
        ]),
    );
    fx.add(
        LAMP_FILE,
        CHECK,
        tree(vec![expression(
            scope::MY_DATA,
            0,
            op::EQUAL,
            scope::LITERAL,
            12345,
        )]),
    );
    fx.program
        .libraries
        .get_mut(&GLOBAL)
        .unwrap()
        .strings
        .insert(501, vec!["(-100;-100)(100;100)".to_owned(); 9]);
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .interactions
        .insert(
            TABLE,
            Some(InteractionTable {
                version: 7,
                interactions: vec![entry(3, 10000, 0), entry(5, 20000, first)],
            }),
        );
    // No custom score curves: an adult's are the global table 501.
    fx.words(SIM_FILE)[51] = 0;
    // Free will on, the family 109 lives here, and lot 0 is residential.
    fx.world.globals[10] = 0;
    // The lamp advertises and is not in use.
    fx.lamp_mut().registry_word = Some(8 << 16);
    fx.lamp_mut().data[25] = 0;
    let person = fx.sim_mut().person.as_mut().unwrap();
    // A human resident adult, standing, with no footprint to redo.
    for word in [0, 32, 65, 73] {
        person.data[word] = 0;
    }
    fx.thread(SIM, CHOOSE, &[]);
    fx
}

fn turn(fx: &mut Fixture) -> Status {
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.turn(&mut fx.world, SIM, 100).unwrap()
}

#[test]
fn a_person_chooses_the_best_interaction_and_starts_it() {
    let mut fx = lot(0x100);
    let rng = fx.world.lot_rng;
    let before = fx.sim().person.clone().unwrap();
    assert_eq!(turn(&mut fx), Status::Yielded);
    // The best entry, menu 5, has flag 0x100: no draw.
    assert_eq!(fx.world.lot_rng, rng);
    // Find Best Interaction left the menu in data word 20 and the lamp
    // as the frame's stack object; Gosub Found Action pushed the entry's
    // action tree with the lamp as its stack object and four zero
    // arguments (the tree set the first to 1, and the Sleep counted it
    // down to 0).
    assert_eq!(fx.sim().data[20], 5);
    let stack = &fx.sim().stack;
    assert_eq!(stack.len(), 2);
    assert_eq!(stack[0].stack_object, LAMP as i16);
    assert_eq!(
        (stack[1].tree, stack[1].stack_object),
        (ACTION, LAMP as i16)
    );
    assert_eq!(stack[1].parameters, [0; 4]);
    // The Sim uses the lamp, which counts one more user.
    let person = fx.sim().person.as_ref().unwrap();
    let uses: Vec<_> = person
        .object_uses
        .iter()
        .map(|u| (u.target, u.stack_depth))
        .collect();
    assert_eq!(uses, [(LAMP as i16, 2)]);
    assert_eq!(fx.lamp().data[62], 124 + 1);
    // The running action: the first unique number, the Sim as caller,
    // the lamp as target and icon, priority 2, the entry's tree and
    // attenuation, and flag bit 0 from the entry's flag 0x100.
    let action = &person.active;
    assert_eq!(fx.world.action_serial, 1);
    assert_eq!(
        (action.uid, action.caller, action.target, action.icon),
        (1, SIM as i16, LAMP as i16, LAMP as i16)
    );
    assert_eq!(
        (
            action.table_index,
            action.args,
            action.priority,
            action.action
        ),
        (5, [0; 4], 2, ACTION)
    );
    assert_eq!((action.attenuation, action.flags), (0.25, 1));
    // The old action had no target, so it is not the last one. Word 33
    // takes the priority; words 38, 39, 71 and 72 are cleared.
    assert_eq!(format!("{:?}", person.last), format!("{:?}", before.last));
    let mut expected = before.data.clone();
    for word in [38, 39, 71, 72] {
        expected[word] = 0;
    }
    expected[33] = 2;
    assert_eq!(person.data, expected);
    // The tree wakes and returns; the use ends and so does the action,
    // which becomes the last one. Gosub Found Action takes the true
    // branch.
    let mut status = turn(&mut fx);
    for _ in 0..4 {
        if status != Status::Yielded {
            break;
        }
        status = turn(&mut fx);
    }
    assert_eq!(status, Status::Returned(true));
    let person = fx.sim().person.as_ref().unwrap();
    assert_eq!((person.last.uid, person.last.target), (1, LAMP as i16));
    assert_eq!(person.active.table_index, -1);
    assert_eq!(fx.lamp().data[62], 124);
    assert_eq!(fx.world.action_serial, 1);
}

#[test]
fn without_a_first_entry_one_draw_picks_among_the_best() {
    let mut fx = lot(0);
    let mut rng = fx.world.lot_rng;
    // The candidates sorted by falling improvement are menus 5 and 3,
    // and the random selection count, 10, is cut to the two there are.
    let menu = [5, 3][(rng.next_u32() % 2) as usize];
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.world.lot_rng, rng);
    assert_eq!(fx.sim().data[20], menu);
    assert_eq!(
        fx.sim().person.as_ref().unwrap().active.table_index,
        i32::from(menu)
    );
}

#[test]
fn a_check_tree_rules_an_entry_out() {
    // Menu 5's check tree returns false, so only menu 3 is a candidate.
    let mut fx = lot(0x100);
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .interactions
        .get_mut(&TABLE)
        .unwrap()
        .as_mut()
        .unwrap()
        .interactions[1]
        .test = CHECK;
    let rng = fx.world.lot_rng;
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.sim().data[20], 3);
    // One candidate: the draw still runs, modulo 1.
    assert_ne!(fx.world.lot_rng, rng);
    // A check tree the file lacks rules its entry out too.
    let mut fx = lot(0x100);
    let table = fx
        .program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .interactions
        .get_mut(&TABLE)
        .unwrap()
        .as_mut()
        .unwrap();
    table.interactions[1].test = 4999;
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.sim().data[20], 3);
}

#[test]
fn no_candidate_is_the_false_branch() {
    // Both entries improve the score less than the threshold.
    let mut fx = lot(0x100);
    let table = fx
        .program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .interactions
        .get_mut(&TABLE)
        .unwrap()
        .as_mut()
        .unwrap();
    for entry in &mut table.interactions {
        entry.motives[5].delta = 1000;
    }
    assert_eq!(turn(&mut fx), Status::Returned(false));
    // An entry whose autonomy threshold is above the person's word 36 is
    // never looked at; nor is an object in use (data word 25 above 0).
    let mut fx = lot(0x100);
    fx.sim_mut().person.as_mut().unwrap().data[36] = 49;
    assert_eq!(turn(&mut fx), Status::Returned(false));
    let mut fx = lot(0x100);
    fx.lamp_mut().data[25] = 1;
    assert_eq!(turn(&mut fx), Status::Returned(false));
    // With free will off (global 30), a resident does not choose.
    let mut fx = lot(0x100);
    fx.world.globals[30] = 0;
    assert_eq!(turn(&mut fx), Status::Returned(false));
}

/// A Sim tree of Gosub Found Action alone, with `object` as the frame's
/// stack object and `menu` in data word 20.
fn gosub(object: i16, menu: i16) -> Fixture {
    let mut fx = lot(0x100);
    fx.add(
        SIM_FILE,
        CHOOSE,
        tree(vec![ins(30, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    fx.sim_mut().stack[0].stack_object = object;
    fx.sim_mut().data[20] = menu;
    fx
}

#[test]
fn gosub_found_action_without_an_object_or_an_entry_fails() {
    let interpreter_error = |fx: &mut Fixture| {
        let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        interpreter.turn(&mut fx.world, SIM, 100).unwrap_err()
    };
    // No stack object: native error 0x15.
    let mut fx = gosub(0, 3);
    assert_eq!(interpreter_error(&mut fx).trap.code(), Some(0x15));
    // No entry with the menu number: native error 0x16.
    let mut fx = gosub(LAMP as i16, 4);
    assert_eq!(interpreter_error(&mut fx).trap.code(), Some(0x16));
    // An ID that names no object, or an object without a table: false.
    let mut fx = gosub(77, 3);
    assert_eq!(turn(&mut fx), Status::Returned(false));
    let mut fx = gosub(LAMP as i16, 3);
    fx.words(LAMP_FILE)[7] = 0;
    assert_eq!(turn(&mut fx), Status::Returned(false));
    assert_eq!(fx.world.action_serial, 0);
}

#[test]
fn an_action_that_has_its_number_keeps_it() {
    // The serial counts on from where it is; the running action with a
    // target becomes the last one.
    let mut fx = gosub(LAMP as i16, 3);
    fx.world.action_serial = 41;
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.active.target = LAMP as i16;
    person.active.uid = 7;
    assert_eq!(turn(&mut fx), Status::Yielded);
    let person = fx.sim().person.as_ref().unwrap();
    assert_eq!(fx.world.action_serial, 42);
    assert_eq!((person.active.uid, person.active.table_index), (42, 3));
    assert_eq!(person.last.uid, 7);
    // Menu 3 lacks flag 0x100.
    assert_eq!(person.active.flags, 0);
}

#[test]
fn a_check_tree_may_change_its_entry_s_advertisements() {
    // Menu 3's check tree names the interaction (dropped in a check
    // tree), asks whether the Sim uses the lamp (it does not), and raises
    // its energy advertisement's delta (scope 12, motive 5) to 30000. Menu
    // 3 is then the best, and with flag 0x100 it is taken without a draw.
    let mut fx = lot(0x100);
    fx.add(
        LAMP_FILE,
        CHECK,
        tree(vec![
            ins(50, 1, 1, [0; 8]),
            ins(37, RETURN_FALSE, 2, [0; 8]),
            assign(12, 5, scope::LITERAL, 30000, RETURN_TRUE),
        ]),
    );
    let table = fx
        .program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .interactions
        .get_mut(&TABLE)
        .unwrap()
        .as_mut()
        .unwrap();
    table.interactions[0].test = CHECK;
    table.interactions[0].flags = 0x100;
    let rng = fx.world.lot_rng;
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.sim().data[20], 3);
    assert_eq!(fx.world.lot_rng, rng);
    // An advertisement index past the sixteen is native error 0x1a.
    let mut fx = lot(0x100);
    fx.add(
        LAMP_FILE,
        CHECK,
        tree(vec![assign(12, 16, scope::LITERAL, 1, RETURN_TRUE)]),
    );
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .interactions
        .get_mut(&TABLE)
        .unwrap()
        .as_mut()
        .unwrap()
        .interactions[0]
        .test = CHECK;
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.turn(&mut fx.world, SIM, 100).unwrap_err();
    assert!(fault.to_string().contains("check tree 0x1069"), "{fault}");
    assert!(fault.to_string().contains("0x1a"), "{fault}");
}

#[test]
fn the_primitives_check_trees_use_fail_loudly_elsewhere() {
    // Add/Change Action String outside a check tree names the running
    // interaction, which is not ported.
    let mut fx = lot(0x100);
    fx.add(
        SIM_FILE,
        CHOOSE,
        tree(vec![ins(50, RETURN_TRUE, RETURN_TRUE, [0; 8])]),
    );
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.turn(&mut fx.world, SIM, 100).unwrap_err();
    assert!(fault.to_string().contains("0x41f1ae"), "{fault}");
    // The advertisement scopes outside a check tree.
    let mut fx = lot(0x100);
    fx.add(
        SIM_FILE,
        CHOOSE,
        tree(vec![assign(12, 0, scope::LITERAL, 1, RETURN_TRUE)]),
    );
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert!(interpreter.turn(&mut fx.world, SIM, 100).is_err());
}

#[test]
fn test_sim_interacting_with_looks_at_the_objects_in_use() {
    // The Sim's tree asks whether it uses its stack object, the lamp.
    let mut fx = lot(0x100);
    fx.add(
        SIM_FILE,
        CHOOSE,
        tree(vec![ins(37, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    fx.sim_mut().stack[0].stack_object = LAMP as i16;
    assert_eq!(turn(&mut fx), Status::Returned(false));
    let mut fx = lot(0x100);
    fx.add(
        SIM_FILE,
        CHOOSE,
        tree(vec![ins(37, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    fx.sim_mut().stack[0].stack_object = LAMP as i16;
    fx.sim_mut().person.as_mut().unwrap().object_uses.push(
        sims_formats::saved_objects::ObjectUse {
            target: LAMP as i16,
            stack_depth: 1,
            unknown: 0,
        },
    );
    assert_eq!(turn(&mut fx), Status::Returned(true));
    // Byte 0 = 1: the person is the one a temp names (scope 8, temp 0).
    // The lamp is not a person: native error 0x1c.
    let mut fx = lot(0x100);
    fx.add(
        SIM_FILE,
        CHOOSE,
        tree(vec![ins(
            37,
            RETURN_TRUE,
            RETURN_FALSE,
            [1, scope::TEMPS, 0, 0, 0, 0, 0, 0],
        )]),
    );
    fx.sim_mut().temps[0] = LAMP as i16;
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.turn(&mut fx.world, SIM, 100).unwrap_err();
    assert_eq!(fault.trap.code(), Some(0x1c));
}
