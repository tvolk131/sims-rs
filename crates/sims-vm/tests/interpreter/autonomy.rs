//! A person chooses an interaction by itself: Find Best Interaction
//! (primitive 3, 0x45dd39) scores the lot's interaction table entries
//! against the person's motives and picks one, and Gosub Found Action
//! (primitive 30, 0x45a859) starts it.

use crate::support::*;
use sims_formats::behavior::{InteractionTable, MotiveAdvertisement};
use sims_vm::{Interpreter, Status, Trap};

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
    // Each entry the scan may choose was copied over the one
    // advertisement record. The scan takes a table's entries last to
    // first, so menu 3's, checked last, stays there.
    let record = fx.world.advertisement_record.unwrap();
    assert_eq!(record[5].delta, 10000);
    assert!(
        record
            .iter()
            .enumerate()
            .all(|(i, ad)| i == 5 || *ad == Default::default())
    );
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
    // The string (table 300, string 1) is looked up and formatted even
    // there (0x4704fa, 0x44bf1a).
    let mut fx = lot(0x100);
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .strings
        .insert(300, vec!["Eat".to_owned()]);
    fx.add(
        LAMP_FILE,
        CHECK,
        tree(vec![
            ins(50, 1, 1, [0x2c, 0x01, 0, 0, 1, 0, 0, 0]),
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
    // The check's fault keeps its trap: a failure, not something
    // unported.
    let Trap::CheckTree {
        routine: None,
        tree: CHECK,
        fault: inner,
        ..
    } = &fault.trap
    else {
        panic!("{fault:?}");
    };
    assert_eq!(inner.trap.code(), Some(0x1a), "{fault}");
    assert!(!fault.trap.unported(), "{fault}");
}

/// A check tree that runs a named tree at once (Run Tree By Name mode 0)
/// shares its advertisements with it: the named tree's change counts.
#[test]
fn a_named_tree_a_check_tree_runs_may_change_its_advertisements() {
    const ADJUST: u16 = 4300;
    let mut fx = lot(0x100);
    fx.add(
        LAMP_FILE,
        ADJUST,
        tree(vec![assign(12, 5, scope::LITERAL, 30000, RETURN_TRUE)]),
    );
    let lamp = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
    lamp.names.insert(ADJUST, "Adjust".to_owned());
    lamp.tree_names = vec!["Adjust".to_owned()];
    fx.add(
        LAMP_FILE,
        CHECK,
        tree(vec![ins(
            28,
            RETURN_TRUE,
            RETURN_FALSE,
            [47, 1, 0, 0, 1, 0, 0, 0],
        )]),
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
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.sim().data[20], 3);
}

#[test]
fn the_advertisement_scopes_find_the_games_start_before_the_first_copy() {
    // Before Find Best Interaction first copies into the record, it holds
    // the sixteen zeroed advertisements the game's start leaves.
    // (Add/Change Action String outside a check tree is in
    // action_string.rs.)
    let mut fx = lot(0x100);
    fx.add(
        SIM_FILE,
        CHOOSE,
        tree(vec![assign(12, 0, scope::LITERAL, 1, RETURN_TRUE)]),
    );
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter
        .turn(&mut fx.world, SIM, 100)
        .unwrap_or_else(|f| panic!("{f}"));
    let record = fx.world.advertisement_record.unwrap();
    assert_eq!(record[0].delta, 1);
    assert!(record[1..].iter().all(|ad| *ad == Default::default()));
}

/// An entry with fewer than sixteen advertisements copies only its own
/// over the record (0x4763c5); the rest stay as the last copy left them.
#[test]
fn an_entry_with_fewer_advertisements_keeps_the_rest_of_the_last_copy() {
    let shorten = |fx: &mut Fixture, index: usize| {
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
        table.interactions[index].motives.truncate(4);
    };
    // Menu 5 is copied first and menu 3, with four, over it: menu 5's
    // energy delta stays.
    let mut fx = lot(0x100);
    shorten(&mut fx, 0);
    assert_eq!(turn(&mut fx), Status::Yielded);
    let record = fx.world.advertisement_record.unwrap();
    assert_eq!(record[5].delta, 20000);
    // With both entries short, the first copies of the game keep the
    // zeroes the game's start left in the rest. (Neither entry then
    // advertises enough to be chosen.)
    let mut fx = lot(0x100);
    shorten(&mut fx, 0);
    shorten(&mut fx, 1);
    turn(&mut fx);
    let record = fx.world.advertisement_record.unwrap();
    assert!(record[4..].iter().all(|ad| *ad == Default::default()));
}

/// The room adjustment compares the rooms after the check tree (0x45852f,
/// 0x458536), and changes the record whatever the check returned.
#[test]
fn the_room_adjustment_reads_the_rooms_the_check_left() {
    const ROOM: i16 = 29;
    let mut fx = lot(0x100);
    // Menu 3's check, which runs on the Sim, moves the Sim to room 1 and
    // fails.
    fx.add(
        LAMP_FILE,
        CHECK,
        tree(vec![assign(
            scope::MY_DATA,
            ROOM,
            scope::LITERAL,
            1,
            RETURN_FALSE,
        )]),
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
    table.interactions[0].motives[13].minimum = 123;
    table.interactions[0].motives[13].delta = 456;
    // Both start in room 0.
    fx.lamp_mut().data[ROOM as usize] = 0;
    fx.sim_mut().data[ROOM as usize] = 0;
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.sim().data[ROOM as usize], 1);
    // Menu 5 was chosen; menu 3's failed check left its record, adjusted.
    assert_eq!(fx.sim().data[20], 5);
    let record = fx.world.advertisement_record.unwrap();
    assert_eq!((record[13].minimum, record[13].delta), (123, -123));
}

/// Outside a check tree, the advertisement scopes read and write the
/// record the last check left (0x453a09 always reads [0x6c437c]), as a
/// check tree the pie menu runs does.
#[test]
fn a_tree_after_a_check_finds_the_record_it_left() {
    let mut fx = lot(0x100);
    assert_eq!(turn(&mut fx), Status::Yielded);
    let mut record = fx.world.advertisement_record.unwrap();
    fx.add(
        SIM_FILE,
        CHOOSE,
        tree(vec![
            ins(
                2,
                1,
                RETURN_FALSE,
                expr(12, 5, op::EQUAL, scope::LITERAL, 10000),
            ),
            assign(12, 0, scope::LITERAL, -9, RETURN_TRUE),
        ]),
    );
    fx.world.objects.get_mut(&SIM).unwrap().stack.clear();
    fx.thread(SIM, CHOOSE, &[]);
    assert_eq!(turn(&mut fx), Status::Returned(true));
    record[0].delta = -9;
    assert_eq!(fx.world.advertisement_record, Some(record));
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

/// In Find Best Interaction, a visitor joining another person's running
/// interaction on a category 4 or 7 object reaches 0x458319's hotel-key
/// check when the joining entry lacks flag 1: without the key it creates
/// an empty inventory and declines; with the key it chooses the join.
#[test]
fn autonomy_allows_a_visitor_join_on_category_4_only_with_the_hotel_key() {
    use sims_sim::neighbors::{Inventory, Item};

    const KEY: u32 = 0x3ac0_22d6;
    const OTHER: u16 = 7;
    const NEIGHBOR: i16 = 55;

    let setup = |with_key: bool| {
        let mut fx = lot(0x100);
        // Lamp is category 4; menu 3 is joinable into menu 5, and neither
        // has flag 1 (VISITORS).
        fx.lamp_mut().data[59] = 4;
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
        table.interactions[0].flags = 2; // JOINABLE
        table.interactions[0].joining_index = 5;
        table.interactions[1].flags = 0x100; // FIRST, no VISITORS

        // A second person running menu 3 on the lamp; advertises, is not
        // in use, and has object type 2 and a table so autonomy_entries
        // appends the join offer.
        fx.words(SIM_FILE)[9] = 2;
        let mut other = fx.sim().clone();
        other.id = OTHER;
        other.data[25] = 0;
        other.registry_word = Some(8 << 16);
        let op = other.person.as_mut().unwrap();
        op.data[32] = 0;
        op.active.target = LAMP as i16;
        op.active.table_index = 3;
        fx.world.objects.insert(OTHER, other);
        let sim_table = fx.words(SIM_FILE)[7] as u16;
        fx.program
            .libraries
            .get_mut(&SIM_FILE)
            .unwrap()
            .interactions
            .insert(
                sim_table,
                Some(InteractionTable {
                    version: 7,
                    interactions: Vec::new(),
                }),
            );

        // SIM is a visitor with neighbor ID 55.
        let sim = fx.sim_mut().person.as_mut().unwrap();
        sim.data[31] = NEIGHBOR;
        sim.data[32] = 1;
        fx.catalog.objects.push(sims_content::Object {
            guid: KEY,
            resource: 0,
            chunk: 0,
            kind: 0xd,
            master: 0,
        });
        if with_key {
            fx.world.neighbors.inventories.push(Inventory {
                neighbor: NEIGHBOR,
                items: vec![Item {
                    kind: 0,
                    guid: KEY,
                    count: 1,
                }],
            });
        }
        fx
    };

    let mut fx = setup(false);
    assert_eq!(turn(&mut fx), Status::Returned(false));
    assert_eq!(
        fx.world.neighbors.inventories,
        vec![Inventory {
            neighbor: NEIGHBOR,
            items: Vec::new(),
        }]
    );

    let mut fx = setup(true);
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.sim().data[20], 5);

    // 0x458319 reads person word 32 live for each action check: if an
    // earlier check tree in the scan clears the visitor flag, the later
    // category-4 join is allowed without touching or creating inventories.
    let mut fx = setup(false);
    // Give OTHER a visitor-allowed entry checked before the join entry;
    // its check tree clears SIM's person data word 32 and returns false.
    fx.add(
        SIM_FILE,
        4305,
        tree(vec![assign(
            18, // My Person Data
            32,
            scope::LITERAL,
            0,
            RETURN_FALSE,
        )]),
    );
    let sim_table = fx.words(SIM_FILE)[7] as u16;
    let mut clear_visitor = entry(1, 0, 1); // VISITORS
    clear_visitor.test = 4305;
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .interactions
        .get_mut(&sim_table)
        .unwrap()
        .as_mut()
        .unwrap()
        .interactions
        .push(clear_visitor);
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.sim().data[20], 5);
    assert!(fx.world.neighbors.inventories.is_empty());
}

#[test]
fn community_and_vacation_lots_skip_objects_with_master_word_69_in_4_or_5() {
    for (lot_num, zoning) in [(58i16, 2i32), (40, 0)] {
        for word69 in [0i16, 4, 5] {
            let mut fx = lot(0x100);
            fx.world.globals[10] = lot_num;
            if zoning != 0 {
                fx.program.zoning.insert(lot_num, zoning);
            }
            fx.words(LAMP_FILE)[69] = word69;
            // Ensure unzoned Vacation lot scans all objects (quarter 0).
            fx.world.ticks = 4 - u32::from(SIM);
            let status = turn(&mut fx);
            if matches!(word69, 4 | 5) {
                assert_eq!(
                    status,
                    Status::Returned(false),
                    "lot {lot_num} word69={word69} should skip lamp"
                );
            } else {
                assert_eq!(
                    status,
                    Status::Yielded,
                    "lot {lot_num} word69={word69} should choose lamp"
                );
            }
        }
    }
}

#[test]
fn vacation_island_hotel_filter_skips_indoor_categories_5_to_7_for_non_family() {
    use sims_sim::rooms::Room;

    let setup = |sim_family: i16, has_hotel_marker: bool, outdoors: bool, category: i16| {
        let mut fx = lot(0x100);
        fx.world.globals[10] = 40; // Vacation Island (40..50)
        fx.world.globals[9] = 109;
        fx.world.ticks = 4 - u32::from(SIM); // quarter 0
        fx.sim_mut().person.as_mut().unwrap().data[61] = sim_family;
        fx.world.rooms.rooms.insert(
            3,
            Room {
                outdoors,
                ..Room::default()
            },
        );
        fx.lamp_mut().data[29] = 3;
        fx.lamp_mut().data[59] = category;
        if has_hotel_marker {
            fx.add_object(99);
            fx.world.objects.get_mut(&99).unwrap().data[59] = 0x2e;
        }
        fx
    };

    // Non-family Sim on a Vacation lot with a 0x2e object skips indoor 5/6/7.
    for cat in [5i16, 6, 7] {
        let mut fx = setup(200, true, false, cat);
        assert_eq!(turn(&mut fx), Status::Returned(false), "indoor cat {cat}");
    }
    // Category 4 indoors is allowed when the Sim holds a hotel key (0x45891b).
    // Without a hotel key on Vacation Island (lot 40), category 4 and 7 targets
    // are skipped in autonomy_entries (and create an empty inventory for the Sim).
    let mut fx = setup(200, true, false, 4);
    assert_eq!(turn(&mut fx), Status::Returned(false));
    assert!(!fx.world.neighbors.inventories.is_empty());

    // Category 3 indoors is allowed without a hotel key.
    let mut fx = setup(200, true, false, 3);
    assert_eq!(turn(&mut fx), Status::Yielded);

    // With a hotel-key token (kind 0xd definition + inventory count != 0),
    // category 4 on Vacation Island is emitted by autonomy_entries (0x45891b..
    // 0x4589e4) even when NOT_AUTONOMOUS is set.
    let mut fx = setup(109, true, false, 4);
    const KEY_GUID: u32 = 0x3ac0_22d6;
    let key_idx = fx.catalog.objects.len();
    fx.catalog.objects.push(sims_content::Object {
        guid: KEY_GUID,
        resource: LAMP_FILE,
        chunk: 99,
        kind: 0xd,
        master: 0,
    });
    fx.catalog.by_guid.insert(KEY_GUID, key_idx);
    fx.sim_mut().person.as_mut().unwrap().data[31] = 1;
    fx.world
        .neighbors
        .inventories
        .push(sims_sim::neighbors::Inventory {
            neighbor: 1,
            items: vec![sims_sim::neighbors::Item {
                kind: 0,
                guid: KEY_GUID,
                count: 1,
            }],
        });
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
        .flags |= 0x80; // NOT_AUTONOMOUS: bypassed by 0x45891b..0x4589e4
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.sim().data[20], 5);

    // Outdoor category 5 is allowed.
    let mut fx = setup(200, true, true, 5);
    assert_eq!(turn(&mut fx), Status::Yielded);
    // Without a 0x2e object on the lot, indoor category 5 is allowed.
    let mut fx = setup(200, false, false, 5);
    assert_eq!(turn(&mut fx), Status::Yielded);
    // Controlled family member (data[61] == global 9) is allowed on category 5.
    let mut fx = setup(109, true, false, 5);
    assert_eq!(turn(&mut fx), Status::Yielded);
}

#[test]
fn unzoned_lot_cycles_through_four_quarter_filters() {
    // Unzoned lot (lot 99, zoning 0): quarter = (world.ticks + SIM) & 3.
    // Place LAMP (id = 1, odd) far from SIM (distance > 7.0 tiles).
    let setup = |quarter: u32, lamp_id: u16, far: bool| {
        let mut fx = lot(0x100);
        fx.world.globals[10] = 99;
        fx.world.ticks = (quarter + 4 - (u32::from(SIM) & 3)) & 3;
        if lamp_id != LAMP {
            let mut extra = fx.lamp().clone();
            extra.id = lamp_id;
            fx.lamp_mut().registry_word = Some(0); // silence original odd LAMP
            fx.world.objects.insert(lamp_id, extra);
        }
        fx.sim_mut().position = [16, 16, 0];
        fx.world.objects.get_mut(&lamp_id).unwrap().position = if far {
            [16 + 10 * 16, 16, 0]
        } else {
            [16 + 7 * 16, 16, 0]
        };
        fx
    };

    // Quarter 0: scans all objects (even far ones).
    assert_eq!(turn(&mut setup(0, LAMP, true)), Status::Yielded);
    // Quarter 1: distance <= 7.0 allowed, > 7.0 skipped.
    assert_eq!(turn(&mut setup(1, LAMP, false)), Status::Yielded);
    assert_eq!(turn(&mut setup(1, LAMP, true)), Status::Returned(false));
    // Quarter 2: even IDs only (id = 4 allowed, id = 1 skipped).
    assert_eq!(turn(&mut setup(2, 4, true)), Status::Yielded);
    assert_eq!(turn(&mut setup(2, LAMP, true)), Status::Returned(false));
    // Quarter 3: odd IDs only (id = 1 allowed, id = 4 skipped).
    assert_eq!(turn(&mut setup(3, LAMP, true)), Status::Yielded);
    assert_eq!(turn(&mut setup(3, 4, true)), Status::Returned(false));
}

#[test]
fn free_will_off_visitor_and_global_9_zero_follow_active_family() {
    // Free will off for a visitor: refused only if person.data[61] == global 9
    // and kind <= 1 (0x45dd87..0x45ddaa). Flag 1 (`VISITORS`) makes the lamp
    // eligible to visitors so the free-will gate is what decides.
    let mut fx = lot(0x100 | 1);
    fx.world.globals[30] = 0; // free will off
    fx.sim_mut().person.as_mut().unwrap().data[32] = 1; // visitor
    fx.sim_mut().person.as_mut().unwrap().data[61] = 109; // matches global 9
    fx.sim_mut().person.as_mut().unwrap().data[65] = 0; // human (<= 1)
    assert_eq!(turn(&mut fx), Status::Returned(false));

    // Visitor from another family is still autonomous with free will off.
    let mut fx = lot(0x100 | 1);
    fx.world.globals[30] = 0;
    fx.sim_mut().person.as_mut().unwrap().data[32] = 1;
    fx.sim_mut().person.as_mut().unwrap().data[61] = 200;
    assert_eq!(turn(&mut fx), Status::Yielded);

    // Global 9 == 0 with active_family = Some(&[SIM_GUID]) refuses autonomy
    // for the first member on the lot (0x45ddb0..0x45ddd5), while
    // active_family = Some(&[]) allows it and None traps Unsupported.
    let mut fx = lot(0x100);
    fx.world.globals[9] = 0;
    let err = Interpreter::new(&fx.program, &fx.catalog, fx.stream)
        .turn(&mut fx.world, SIM, 100)
        .unwrap_err();
    assert!(matches!(err.trap, Trap::Unsupported(_)));

    let mut fx = lot(0x100);
    fx.world.globals[9] = 0;
    let first_member = [SIM_GUID];
    let mut interp = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interp.active_family = Some(&first_member);
    assert_eq!(
        interp.turn(&mut fx.world, SIM, 100).unwrap(),
        Status::Returned(false)
    );

    let mut fx = lot(0x100);
    fx.world.globals[9] = 0;
    let mut interp = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interp.active_family = Some(&[]);
    assert_eq!(
        interp.turn(&mut fx.world, SIM, 100).unwrap(),
        Status::Yielded
    );
}
