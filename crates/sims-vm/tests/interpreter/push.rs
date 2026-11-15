//! Push Interaction (primitive 13, 0x44cd2b): a script queues an
//! interaction of an object on the person in its stack object.
//! Synthetic; no game data.

use crate::support::*;
use sims_formats::behavior::{Interaction, InteractionTable};
use sims_vm::{Status, Trap};

/// The lamp's interaction table (definition word 7, which counts up from
/// 1000 in the synthetic definitions).
const TABLE: u16 = 1007;
/// The lamp's action tree for its entries.
const ACTION: u16 = 4200;

fn entry(menu: u32, flags: u32) -> Interaction {
    Interaction {
        action: ACTION,
        test: 0,
        string_index: menu,
        flags,
        autonomy: 50,
        attenuation_code: 0,
        attenuation: 0.25,
        joining_index: -1,
        motives: Vec::new(),
        flags2: 0,
    }
}

/// A lot whose lamp has menu 3 (flag 0x100) and menu 4 (flag 4, runs at
/// once), and whose tree 4096 is one Push Interaction with `operands`.
/// The Sim is a resident whose running action has priority 7.
fn lot(operands: [u8; 8]) -> Fixture {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(13, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .interactions
        .insert(
            TABLE,
            Some(InteractionTable {
                version: 7,
                interactions: vec![entry(3, 0x100), entry(4, 4)],
            }),
        );
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[32] = 0;
    person.data[33] = 7;
    person.queue.clear();
    fx
}

/// Runs the lamp's tree with the Sim as stack object and `args` as its
/// parameters.
fn push(fx: &mut Fixture, args: &[i16]) -> Result<bool, sims_vm::Fault> {
    fx.run_with(LAMP, 4096, SIM as i16, args, 1000)
}

#[test]
fn an_interaction_is_queued_on_the_stack_object() {
    // Menu 3 of the object in parameter 0, priority 100, flag bit 3.
    let mut fx = lot([3, 0, 1, 8, 0, 0, 0, 0]);
    assert_eq!(push(&mut fx, &[LAMP as i16]), Ok(true));
    let queue = &fx.sim().person.as_ref().unwrap().queue;
    let [action] = queue.as_slice() else {
        panic!("{queue:?}");
    };
    assert_eq!(fx.world.action_serial, 1);
    assert_eq!(
        (action.uid, action.caller, action.target, action.icon),
        (1, SIM as i16, LAMP as i16, LAMP as i16)
    );
    assert_eq!(
        (
            action.table_index,
            action.priority,
            action.action,
            action.args
        ),
        (3, 100, ACTION, [0; 4])
    );
    // Flag bit 0 from the entry's flag 0x100, and 0x40 from bit 3.
    assert_eq!(action.flags, 0x41);
    assert_eq!(action.attenuation, 0.25);
}

#[test]
fn priority_0_is_the_running_persons_or_1() {
    // The lamp is not a person: 1.
    let mut fx = lot([3, 0, 0, 0, 0, 0, 0, 0]);
    push(&mut fx, &[LAMP as i16]).unwrap();
    assert_eq!(fx.sim().person.as_ref().unwrap().queue[0].priority, 1);
    for (byte, priority) in [(2, 2), (3, 50)] {
        let mut fx = lot([3, 0, byte, 0, 0, 0, 0, 0]);
        push(&mut fx, &[LAMP as i16]).unwrap();
        assert_eq!(
            fx.sim().person.as_ref().unwrap().queue[0].priority,
            priority
        );
    }
}

#[test]
fn the_icon_comes_from_a_local_and_is_none_for_no_object() {
    // The object from local 0, the icon from local 1.
    for (icon, expected) in [(SIM as i16, SIM as i16), (999, 0)] {
        let mut fx = lot([3, 0, 1, 3, 1, 0, 0, 0]);
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, LAMP as i16, 1),
                assign(scope::LOCALS, 1, scope::LITERAL, icon, 2),
                ins(13, RETURN_TRUE, RETURN_FALSE, [3, 0, 1, 3, 1, 0, 0, 0]),
            ]),
        );
        assert_eq!(push(&mut fx, &[]), Ok(true));
        assert_eq!(fx.sim().person.as_ref().unwrap().queue[0].icon, expected);
    }
}

#[test]
fn what_is_missing_is_a_native_error() {
    // No object with the ID: 0x17.
    let mut fx = lot([3, 0, 1, 0, 0, 0, 0, 0]);
    assert_eq!(code(push(&mut fx, &[999])), Some(0x17));
    // No entry with the menu number: 0x16.
    let mut fx = lot([9, 0, 1, 0, 0, 0, 0, 0]);
    assert_eq!(code(push(&mut fx, &[LAMP as i16])), Some(0x16));
    // An unknown priority: 0x2d.
    let mut fx = lot([3, 0, 4, 0, 0, 0, 0, 0]);
    assert_eq!(code(push(&mut fx, &[LAMP as i16])), Some(0x2d));
    // A stack object that is not a person: 0x1c.
    let mut fx = lot([3, 0, 1, 0, 0, 0, 0, 0]);
    assert_eq!(
        code(fx.run_with(LAMP, 4096, LAMP as i16, &[LAMP as i16], 1000)),
        Some(0x1c)
    );
    // No stack object: 0x15.
    assert_eq!(
        code(fx.run_with(LAMP, 4096, 0, &[LAMP as i16], 1000)),
        Some(0x15)
    );
    // A parameter past the count: 8.
    let mut fx = lot([3, 5, 1, 0, 0, 0, 0, 0]);
    assert_eq!(code(push(&mut fx, &[LAMP as i16])), Some(8));
    // One just past the last, which the original's bound lets through
    // to the word after: that fails loudly.
    let mut fx = lot([3, 4, 1, 0, 0, 0, 0, 0]);
    assert!(matches!(
        trap(push(&mut fx, &[LAMP as i16])),
        sims_vm::Trap::Unsupported(m) if m.contains("0x44cd5a")
    ));
}

#[test]
fn what_the_queue_does_not_port_fails_loudly() {
    // An entry that runs at once, queued by an object other than the
    // person.
    let mut fx = lot([4, 0, 1, 0, 0, 0, 0, 0]);
    assert!(matches!(
        trap(push(&mut fx, &[LAMP as i16])),
        sims_vm::Trap::Unsupported(m) if m.contains("0x45d240") && m.contains("another person")
    ));
    // The queue's front.
    let mut fx = lot([3, 0, 1, 4, 0, 0, 0, 0]);
    assert!(matches!(
        trap(push(&mut fx, &[LAMP as i16])),
        sims_vm::Trap::Unsupported(m) if m.contains("0x45d322")
    ));
}

/// A player's order queues the action the pie menu makes: priority 50,
/// flags 4 and 8 from the check it passed, and the entry's flag 0x100 as
/// bit 0 (`sims_vm::orders`).
#[test]
fn a_players_order_queues_the_menus_action() {
    let mut fx = lot([0; 8]);
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.order(&mut fx.world, SIM, LAMP, 3), Ok(Some(1)));
    let queue = &fx.sim().person.as_ref().unwrap().queue;
    let [action] = queue.as_slice() else {
        panic!("{queue:?}");
    };
    assert_eq!(
        (action.uid, action.caller, action.target, action.icon),
        (1, SIM as i16, LAMP as i16, LAMP as i16)
    );
    assert_eq!(
        (
            action.table_index,
            action.priority,
            action.action,
            action.args
        ),
        (3, 50, ACTION, [0; 4])
    );
    assert_eq!(action.flags, 0xd);
    assert_eq!(action.attenuation, 0.25);
}

#[test]
fn an_order_for_no_entry_or_no_person_is_refused() {
    let mut fx = lot([0; 8]);
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    for (person, menu) in [(SIM, 9), (LAMP, 3), (999, 3)] {
        assert!(matches!(
            interpreter.order(&mut fx.world, person, LAMP, menu),
            Err(sims_vm::Fault {
                trap: sims_vm::Trap::Invalid(_),
                ..
            })
        ));
    }
    assert_eq!(fx.world.action_serial, 0);
}

/// A local that still holds half of a pointer the original left in the
/// frame storage fails loudly, for the object and for the icon.
#[test]
fn a_local_left_half_a_pointer_fails_loudly() {
    // The object from local 0 and the icon from local 1. For the icon,
    // local 0 is written first, so it names the lamp.
    let push_op = ins(13, RETURN_TRUE, RETURN_FALSE, [3, 0, 1, 3, 1, 0, 0, 0]);
    for unknown in [0, 1] {
        let mut fx = lot([0; 8]);
        let code = if unknown == 0 {
            vec![push_op.clone()]
        } else {
            vec![
                assign(scope::LOCALS, 0, scope::LITERAL, LAMP as i16, 1),
                push_op.clone(),
            ]
        };
        fx.add(LAMP_FILE, 4096, tree(code));
        let mut storage = sims_sim::FrameStorage::default();
        storage.unknown.insert((0, unknown));
        fx.world.frame_storage.insert(LAMP, storage);
        let text = trap(push(&mut fx, &[])).to_string();
        assert!(text.contains("half of a pointer"), "{text}");
    }
}

/// A full queue takes nothing, and the branch is false, but the action
/// took its number first, as the original's (0x45d30d).
#[test]
fn a_full_queue_takes_nothing_but_the_number_is_used() {
    let mut fx = lot([3, 0, 1, 0, 0, 0, 0, 0]);
    for _ in 0..8 {
        if !push(&mut fx, &[LAMP as i16]).unwrap() {
            break;
        }
    }
    let full = fx.sim().person.as_ref().unwrap().queue.len();
    let serial = fx.world.action_serial;
    assert_eq!(push(&mut fx, &[LAMP as i16]), Ok(false));
    assert_eq!(fx.sim().person.as_ref().unwrap().queue.len(), full);
    assert_eq!(fx.world.action_serial, serial + 1);
    // A player's order on a full queue does the same.
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.order(&mut fx.world, SIM, LAMP, 3), Ok(None));
    assert_eq!(fx.world.action_serial, serial + 2);
}

/// The Sim's tree 4096: Idle For Input asked to start the queued action,
/// with parameter 0 as its count.
const IDLE: u16 = 4096;

/// The lot with the Sim idle (count 3) and the lamp's action tree a Sleep
/// on its parameter 0. The Sim's order for menu 3 is queued with
/// priority 50, above the Sim's 7, so it interrupts the wait.
fn ordered() -> Fixture {
    let mut fx = lot([0; 8]);
    fx.add(
        SIM_FILE,
        IDLE,
        tree(vec![ins(
            17,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 1, 0, 0, 0, 0, 0],
        )]),
    );
    fx.add(
        LAMP_FILE,
        ACTION,
        tree(vec![ins(0, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    fx.world.globals[10] = 4;
    fx.program.zoning.insert(4, 1);
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[71] = 0;
    // Word 73 clear (no footprint to work out again); an adult person,
    // not a pet.
    person.data[73] = 0;
    person.data[58] = 30;
    person.data[65] = 0;
    person.motives = [0.0; 16];
    fx.thread(SIM, IDLE, &[3]);
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.order(&mut fx.world, SIM, LAMP, 3), Ok(Some(1)));
    fx
}

fn idle_turn(fx: &mut Fixture) -> (Status, Vec<sims_vm::Asked>) {
    let asked = std::cell::RefCell::new(Vec::new());
    let mut interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.asked = Some(&asked);
    let status = interpreter
        .turn(&mut fx.world, SIM, 100)
        .unwrap_or_else(|f| panic!("{f}"));
    (status, asked.into_inner())
}

#[test]
fn idle_for_input_starts_the_players_order() {
    let mut fx = ordered();
    fx.sim_mut().person.as_mut().unwrap().queue[0].args = [2, 0, 0, 0];
    fx.sim_mut().temps[0] = 11;
    let (status, asked) = idle_turn(&mut fx);
    // The lamp's action tree runs on the Sim with the action's arguments
    // and the lamp as its stack object, and sleeps.
    assert_eq!(status, Status::Yielded);
    assert!(asked.is_empty(), "{asked:?}");
    let sim = fx.sim();
    let [idle, action] = sim.stack.as_slice() else {
        panic!("{:?}", sim.stack);
    };
    assert_eq!(
        (idle.tree, idle.parameters[0], idle.primitive_state),
        (IDLE, 0, 1)
    );
    assert_eq!((action.tree, action.stack_object), (ACTION, LAMP as i16));
    assert_eq!(action.parameters[0], 1);
    assert_eq!(sim.data[8] & 0x40, 0x40);
    // It is the running action, off the queue, with the check's flags.
    let person = sim.person.as_ref().unwrap();
    assert!(person.queue.is_empty());
    assert_eq!((person.active.uid, person.active.target), (1, LAMP as i16));
    assert_eq!(person.active.flags & 0x1c, 0xc);
    assert_eq!(person.data[33], 50);
    assert_eq!(sim.temps[0], 11);
    // When the action tree returns, Idle For Input looks again: nothing
    // is queued or low, and the count is 0, so it is done.
    assert_eq!(idle_turn(&mut fx).0, Status::Yielded);
    assert_eq!(idle_turn(&mut fx).0, Status::Returned(true));
    assert_eq!(fx.sim().data[8] & 0x40, 0);
}

#[test]
fn after_an_action_only_one_pushed_to_the_front_is_started() {
    for (flags, started) in [(0xd, false), (0xf, true)] {
        // The first action sleeps a turn; another, of priority 60, is
        // queued meanwhile and interrupts once it returns.
        let mut fx = ordered();
        fx.sim_mut().person.as_mut().unwrap().queue[0].args = [1, 0, 0, 0];
        assert_eq!(idle_turn(&mut fx).0, Status::Yielded);
        let person = fx.sim_mut().person.as_mut().unwrap();
        let mut next = person.active.clone();
        (next.uid, next.flags, next.args, next.priority) = (0, flags, [5, 0, 0, 0], 60);
        person.queue.push(next);
        let (status, _) = idle_turn(&mut fx);
        let person = fx.sim().person.as_ref().unwrap();
        if started {
            assert_eq!(status, Status::Yielded, "flags {flags:#x}");
            assert!(person.queue.is_empty());
            assert_eq!(person.active.uid, 2);
        } else {
            assert_eq!(status, Status::Returned(true), "flags {flags:#x}");
            assert_eq!(person.queue.len(), 1);
        }
    }
}

#[test]
fn a_refused_order_is_cancelled_on_its_target() {
    // A target with data word 8 bit 9, a visitor without the entry's
    // flag 1, or a check tree that returns false.
    let refusals: [fn(&mut Fixture); 3] = [
        |fx| fx.lamp_mut().data[8] |= 0x200,
        |fx| fx.sim_mut().person.as_mut().unwrap().data[32] = 1,
        |fx| {
            fx.add(
                LAMP_FILE,
                4300,
                // A Sleep on parameter 0, which the check sets to 0.
                tree(vec![ins(0, RETURN_FALSE, RETURN_FALSE, [0; 8])]),
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
            table.interactions[0].test = 4300;
        },
    ];
    for (case, refuse) in refusals.into_iter().enumerate() {
        let mut fx = ordered();
        refuse(&mut fx);
        let before = format!("{:?}", fx.sim().person.as_ref().unwrap().active);
        let (status, asked) = idle_turn(&mut fx);
        assert_eq!(status, Status::Yielded, "case {case}");
        let functions: Vec<_> = asked.iter().map(|a| (a.object, a.function)).collect();
        assert_eq!(functions, [(LAMP, 4)], "case {case}");
        let person = fx.sim().person.as_ref().unwrap();
        assert!(person.queue.is_empty(), "case {case}");
        assert_eq!(format!("{:?}", person.active), before, "case {case}");
        assert_eq!(fx.sim().stack.len(), 1, "case {case}");
    }
    // An action with no target is dropped without asking anyone.
    let mut fx = ordered();
    fx.sim_mut().person.as_mut().unwrap().queue[0].target = 0;
    let (status, asked) = idle_turn(&mut fx);
    assert_eq!(status, Status::Yielded);
    assert!(asked.is_empty());
    assert!(fx.sim().person.as_ref().unwrap().queue.is_empty());
}

/// Gives the lamp's menu 3 the check tree `test` and the action tree
/// `action` (an action already queued keeps the tree it was made with).
fn entry_trees(fx: &mut Fixture, test: u16, action: u16) {
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
    (table.interactions[0].test, table.interactions[0].action) = (test, action);
}

/// A check tree that writes 99 to temp 0 and passes.
fn temp_writing_check(fx: &mut Fixture) {
    fx.add(
        LAMP_FILE,
        4300,
        tree(vec![assign(
            scope::TEMPS,
            0,
            scope::LITERAL,
            99,
            RETURN_TRUE,
        )]),
    );
}

#[test]
fn the_temps_the_check_tree_changes_are_put_back() {
    // After a start.
    let mut fx = ordered();
    temp_writing_check(&mut fx);
    entry_trees(&mut fx, 4300, ACTION);
    fx.sim_mut().person.as_mut().unwrap().queue[0].args = [2, 0, 0, 0];
    fx.sim_mut().temps[0] = 11;
    assert_eq!(idle_turn(&mut fx).0, Status::Yielded);
    assert_eq!(fx.sim().stack.len(), 2);
    assert_eq!(fx.sim().temps[0], 11);
    // And when the start then fails: an action tree the file lacks.
    let mut fx = ordered();
    temp_writing_check(&mut fx);
    entry_trees(&mut fx, 4300, ACTION);
    fx.sim_mut().person.as_mut().unwrap().queue[0].action = 4999;
    fx.sim_mut().temps[0] = 11;
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.turn(&mut fx.world, SIM, 100).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
    assert_eq!(fx.sim().temps[0], 11);
}

#[test]
fn a_missing_check_tree_refuses_after_writing_the_synchronous_mark() {
    let mut fx = ordered();
    entry_trees(&mut fx, 4998, ACTION);
    let (status, asked) = idle_turn(&mut fx);
    assert_eq!(status, Status::Yielded);
    let functions: Vec<_> = asked.iter().map(|a| (a.object, a.function)).collect();
    assert_eq!(functions, [(LAMP, 4)]);
    let words = &fx.world.frame_storage[&SIM].words;
    let mark = [-1, 0, 0, 0, 1, 0, 0, 0].map(Some);
    assert!(words.windows(8).any(|w| w == mark), "{words:?}");
}

#[test]
fn idle_for_input_keeps_its_state_when_it_yields() {
    // The first action sleeps a turn. Meanwhile a target-0 action pushed
    // to the front and an ordinary one, both of priority 60, are queued.
    let mut fx = ordered();
    fx.sim_mut().person.as_mut().unwrap().queue[0].args = [1, 0, 0, 0];
    assert_eq!(idle_turn(&mut fx).0, Status::Yielded);
    let person = fx.sim_mut().person.as_mut().unwrap();
    let mut dropped = person.active.clone();
    (dropped.uid, dropped.flags, dropped.priority, dropped.target) = (0, 0xf, 60, 0);
    let mut ordinary = dropped.clone();
    (ordinary.flags, ordinary.target) = (0xd, LAMP as i16);
    person.queue.extend([dropped, ordinary]);
    // The action returns; the target-0 one is started, dropped, and the
    // thread yields with Idle For Input's state still 1.
    assert_eq!(idle_turn(&mut fx).0, Status::Yielded);
    assert_eq!(fx.sim().stack.len(), 1);
    assert_eq!(fx.sim().stack[0].primitive_state, 1);
    assert_eq!(fx.sim().person.as_ref().unwrap().queue.len(), 1);
    // Next turn the ordinary one interrupts but was not pushed to the
    // front, so it stays queued and the wait is over.
    assert_eq!(idle_turn(&mut fx).0, Status::Returned(true));
    assert_eq!(fx.sim().person.as_ref().unwrap().queue.len(), 1);
}

/// The lot with an adult resident Sim, not a pet, and the lamp's table
/// set to `entries`.
fn menu_lot(entries: Vec<Interaction>) -> Fixture {
    let mut fx = lot([0; 8]);
    let person = fx.sim_mut().person.as_mut().unwrap();
    (person.data[73], person.data[58], person.data[65]) = (0, 30, 0);
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
    table.interactions = entries;
    fx
}

fn with_test(menu: u32, flags: u32, test: u16) -> Interaction {
    Interaction {
        test,
        ..entry(menu, flags)
    }
}

#[test]
fn the_pie_menu_lists_each_entry_whose_check_passes_in_order() {
    // Menu 5 is refused by its check tree; menu 3 appears twice, and the
    // second makes the first's action (its flags and tree), as find-first
    // does; menu 0x80's flag is only a name prefix.
    let mut second = entry(3, 0);
    second.action = 4201;
    let mut fx = menu_lot(vec![
        with_test(5, 0, 4301),
        entry(3, 0x100),
        entry(9, 0x80),
        second,
        with_test(4, 4, 4300),
    ]);
    temp_writing_check(&mut fx);
    fx.add(
        LAMP_FILE,
        4301,
        tree(vec![ins(0, RETURN_FALSE, RETURN_FALSE, [0; 8])]),
    );
    fx.sim_mut().temps[0] = 11;
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let listed = interpreter
        .pie_menu(&mut fx.world, SIM, LAMP)
        .unwrap_or_else(|f| panic!("{f}"));
    assert!(listed.iter().all(|e| e.name.is_none()));
    let shown: Vec<_> = listed
        .iter()
        .map(|e| &e.action)
        .map(|a| (a.table_index, a.flags, a.action, a.uid, a.priority))
        .collect();
    assert_eq!(
        shown,
        [
            (3, 0xd, ACTION, 0, 50),
            (9, 0xc, ACTION, 0, 50),
            (3, 0xd, ACTION, 0, 50),
            (4, 0xc, ACTION, 0, 50),
        ]
    );
    // The check trees' temps are put back; nothing is queued or numbered.
    assert_eq!(fx.sim().temps[0], 11);
    assert!(fx.sim().person.as_ref().unwrap().queue.is_empty());
    assert_eq!(fx.world.action_serial, 0);
}

#[test]
fn a_picked_action_is_queued_as_listed() {
    let mut fx = menu_lot(vec![entry(3, 0x100), entry(4, 4)]);
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let listed = interpreter.pie_menu(&mut fx.world, SIM, LAMP).unwrap();
    assert_eq!(
        interpreter.pick(&mut fx.world, SIM, listed[0].action.clone()),
        Ok(Some(1))
    );
    let queue = &fx.sim().person.as_ref().unwrap().queue;
    let [queued] = queue.as_slice() else {
        panic!("{queue:?}");
    };
    let mut expected = listed[0].action.clone();
    expected.uid = 1;
    assert_eq!(format!("{queued:?}"), format!("{expected:?}"));
    // The same as an order for the entry.
    let mut ordered = menu_lot(vec![entry(3, 0x100), entry(4, 4)]);
    assert_eq!(
        interpreter.order(&mut ordered.world, SIM, LAMP, 3),
        Ok(Some(1))
    );
    assert_eq!(
        format!("{:?}", ordered.sim().person.as_ref().unwrap().queue),
        format!("{queue:?}")
    );
    // An entry that runs at once is run, not queued: it takes the next
    // number.
    assert_eq!(
        interpreter.pick(&mut fx.world, SIM, listed[1].action.clone()),
        Ok(Some(2))
    );
    assert_eq!(fx.sim().person.as_ref().unwrap().queue.len(), 1);
}

#[test]
fn what_the_pie_menu_does_not_port_fails_loudly() {
    let unsupported = |fx: &mut Fixture, object: u16, address: &str| {
        let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        let fault = interpreter
            .pie_menu(&mut fx.world, SIM, object)
            .unwrap_err();
        assert!(
            matches!(&fault.trap, Trap::Unsupported(m) if m.contains(address)),
            "{fault}"
        );
    };
    // A pet.
    let mut fx = menu_lot(vec![entry(3, 0)]);
    fx.sim_mut().person.as_mut().unwrap().data[65] = 2;
    unsupported(&mut fx, LAMP, "0x4585b1");
    // Another person, while my running interaction may be joined.
    let mut fx = menu_lot(vec![entry(3, 2)]);
    fx.add_sim(7);
    let person = fx.sim_mut().person.as_mut().unwrap();
    (person.active.target, person.active.table_index) = (LAMP as i16, 3);
    unsupported(&mut fx, 7, "0x45866d");
    // Not a person.
    let mut fx = menu_lot(vec![entry(3, 0)]);
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.pie_menu(&mut fx.world, LAMP, LAMP).unwrap_err();
    assert!(matches!(fault.trap, Trap::Invalid(_)), "{fault}");
}

#[test]
fn a_picked_action_whose_entry_is_gone_is_not_queued() {
    let mut fx = menu_lot(vec![entry(3, 0x100)]);
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let listed = interpreter.pie_menu(&mut fx.world, SIM, LAMP).unwrap();
    // Its menu number is no longer in the table, or its target has no
    // table: nothing is queued and no number is used (0x45d22a).
    let mut renumbered = listed[0].action.clone();
    renumbered.table_index = 9;
    let mut on_sim = listed[0].action.clone();
    on_sim.target = SIM as i16;
    for action in [renumbered, on_sim] {
        assert_eq!(interpreter.pick(&mut fx.world, SIM, action), Ok(None));
    }
    assert!(fx.sim().person.as_ref().unwrap().queue.is_empty());
    assert_eq!(fx.world.action_serial, 0);
}

#[test]
fn each_listed_action_reads_whether_i_am_a_visitor_after_the_checks_before_it() {
    // Low attenuation (code 2) differs for visitors. Menu 3's check makes
    // me a visitor (person data word 32); menu 4, allowed to visitors
    // (flag 1), is then made with the visitors' attenuation.
    let low = |name: &str, value| sims_formats::tuning::Constant {
        name: name.into(),
        value,
        comment: String::new(),
    };
    let coded = |menu, flags, test| Interaction {
        attenuation_code: 2,
        ..with_test(menu, flags, test)
    };
    let mut fx = menu_lot(vec![coded(3, 0, 4300), coded(4, 1, 0)]);
    let global = fx.program.global;
    fx.program
        .libraries
        .get_mut(&global)
        .unwrap()
        .tuning
        .insert(
            2,
            sims_formats::tuning::Constants {
                version: 0,
                entries: vec![
                    low("low attenuation", 0.25),
                    low("visitor low attenuation", 0.75),
                ],
            },
        );
    fx.add(
        LAMP_FILE,
        4300,
        tree(vec![assign(18, 32, scope::LITERAL, 1, RETURN_TRUE)]),
    );
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let listed = interpreter.pie_menu(&mut fx.world, SIM, LAMP).unwrap();
    let made: Vec<_> = listed
        .iter()
        .map(|e| (e.action.table_index, e.action.attenuation))
        .collect();
    assert_eq!(made, [(3, 0.25), (4, 0.75)]);
}

/// Add/Change Action String's operands: string `index` (from 1) of the
/// string table 300.
fn action_string(index: u8) -> [u8; 8] {
    let mut operands = [0; 8];
    operands[..2].copy_from_slice(&300u16.to_le_bytes());
    operands[4] = index;
    operands
}

/// A menu lot whose lamp's file has string table 300 with `strings`.
fn named_menu_lot(entries: Vec<Interaction>, strings: &[&str]) -> Fixture {
    let mut fx = menu_lot(entries);
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .strings
        .insert(300, strings.iter().map(|s| s.to_string()).collect());
    fx
}

#[test]
fn a_check_tree_adds_named_entries_in_place_of_its_action() {
    // Menu 3's check names two entries and passes; menu 4's names one and
    // refuses; menu 5 has no check tree. Menu 3's check writes temp 0.
    let mut fx = named_menu_lot(
        vec![
            with_test(3, 0x100, 4302),
            with_test(4, 0, 4303),
            entry(5, 0),
        ],
        &["Have Snack", "Serve Dinner", "Grab Juice"],
    );
    fx.add(
        LAMP_FILE,
        4302,
        tree(vec![
            assign(scope::TEMPS, 0, scope::LITERAL, 99, 1),
            ins(50, 2, RETURN_FALSE, action_string(1)),
            ins(50, RETURN_TRUE, RETURN_FALSE, action_string(2)),
        ]),
    );
    fx.add(
        LAMP_FILE,
        4303,
        tree(vec![
            ins(50, 1, RETURN_FALSE, action_string(3)),
            ins(0, RETURN_FALSE, RETURN_FALSE, [0; 8]),
        ]),
    );
    fx.sim_mut().temps[0] = 11;
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let listed = interpreter
        .pie_menu(&mut fx.world, SIM, LAMP)
        .unwrap_or_else(|f| panic!("{f}"));
    let shown: Vec<_> = listed
        .iter()
        .map(|e| {
            let a = &e.action;
            (e.name.as_deref(), a.table_index, a.flags, a.args, a.target)
        })
        .collect();
    // Each named entry is a copy of the action being checked, with flags
    // 4 and 8, the stack object (the lamp) as its first argument; the
    // action itself is left out whether or not its check passed.
    let lamp = LAMP as i16;
    assert_eq!(
        shown,
        [
            (Some("Have Snack"), 3, 0xd, [lamp, 0, 0, 0], lamp),
            (Some("Serve Dinner"), 3, 0xd, [lamp, 0, 0, 0], lamp),
            (Some("Grab Juice"), 4, 0xc, [lamp, 0, 0, 0], lamp),
            (None, 5, 0xc, [0; 4], lamp),
        ]
    );
    assert!(listed.iter().all(|e| e.action.priority == 50));
    assert_eq!(fx.sim().temps[0], 11);
    // Picking a named entry queues its action, arguments and all.
    assert_eq!(
        interpreter.pick(&mut fx.world, SIM, listed[1].action.clone()),
        Ok(Some(1))
    );
    let queued = &fx.sim().person.as_ref().unwrap().queue[0];
    assert_eq!((queued.table_index, queued.args), (3, [lamp, 0, 0, 0]));
}

#[test]
fn an_entrys_string_with_a_token_or_missing_fails_loudly() {
    // Index 2 has a token the formatter would replace; 0 and 3 are not in
    // the table.
    for (index, address) in [(2, "0x44bf1a"), (0, "0x4704fa"), (3, "0x4704fa")] {
        let mut fx = named_menu_lot(vec![with_test(4, 0, 4302)], &["Eat", "Eat $Object"]);
        fx.add(
            LAMP_FILE,
            4302,
            tree(vec![ins(
                50,
                RETURN_TRUE,
                RETURN_FALSE,
                action_string(index),
            )]),
        );
        let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        let fault = interpreter.pie_menu(&mut fx.world, SIM, LAMP).unwrap_err();
        assert!(
            matches!(&fault.trap, Trap::Unsupported(m) if m.contains(address)),
            "{index}: {fault}"
        );
    }
}

/// A menu lot whose lamp heads tile (2, 2) of level 1, with a second
/// lamp (object 3) linked from its slot 0. Menu 5's check passes for the
/// lamp as stack object 3 only, and only while temp 0 is 99; menu 6's
/// sets temp 0 to 99 and refuses.
fn tile_lot() -> Fixture {
    let mut fx = menu_lot(vec![with_test(5, 0, 4302), with_test(6, 0, 4303)]);
    fx.add(
        LAMP_FILE,
        4302,
        tree(vec![
            ins(
                2,
                1,
                RETURN_FALSE,
                expr(scope::TEMPS, 0, op::EQUAL, scope::LITERAL, 99),
            ),
            ins(
                2,
                RETURN_TRUE,
                RETURN_FALSE,
                expr(scope::STACK_DATA, 11, op::EQUAL, scope::LITERAL, 3),
            ),
        ]),
    );
    fx.add(
        LAMP_FILE,
        4303,
        tree(vec![
            assign(scope::TEMPS, 0, scope::LITERAL, 99, 1),
            ins(0, RETURN_FALSE, RETURN_FALSE, [0; 8]),
        ]),
    );
    let mut grid = vec![0; 64 * 64 * 2];
    grid[(2 * 64 + 2) * 2..][..2].copy_from_slice(&(LAMP as i16).to_le_bytes());
    fx.world.arrays.insert(
        3,
        sims_formats::lot::Array {
            width: 64,
            height: 64,
            stride: 2,
            encoding: 0,
            data: grid,
        },
    );
    let lamp = fx.lamp_mut();
    lamp.position = [40, 40, 1];
    lamp.data[14] = -1;
    lamp.data[26] = 0;
    lamp.slots = vec![sims_formats::saved_objects::Slot {
        unknown: 0,
        object: 3,
    }];
    let mut other = fx.lamp().clone();
    other.id = 3;
    other.data[11] = 3;
    other.data[14] = 0;
    other.data[26] = LAMP as i16;
    other.slots.clear();
    fx.world.objects.insert(3, other);
    fx.world.order.push(3);
    fx.world.size = 64;
    fx.sim_mut().temps[0] = 11;
    fx
}

#[test]
fn a_menu_that_offers_nothing_lists_the_other_objects_on_the_tile() {
    let mut fx = tile_lot();
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let listed = interpreter
        .pie_menu(&mut fx.world, SIM, LAMP)
        .unwrap_or_else(|f| panic!("{f}"));
    // The lamp offers nothing; object 3's menu 5 passes, because the
    // lamp's own temps are not yet put back when object 3 is listed.
    let shown: Vec<_> = listed
        .iter()
        .map(|e| (e.action.target, e.action.table_index))
        .collect();
    assert_eq!(shown, [(3, 5)]);
    // All the temps are put back in the end.
    assert_eq!(fx.sim().temps[0], 11);
    // Listed from object 3 directly, it offers nothing: menu 5 sees temp
    // 0 as 11 and the lamp on its tile is refused too. The menu is empty.
    let mut fx = tile_lot();
    let listed = interpreter.pie_menu(&mut fx.world, SIM, 3).unwrap();
    assert!(listed.is_empty());
    assert_eq!(fx.sim().temps[0], 11);
}

#[test]
fn the_tiles_listing_leaves_out_type_8_and_objects_without_word_91_bit_0() {
    for (word, value) in [(9, 8), (91, 0x0442)] {
        let mut fx = tile_lot();
        fx.words(LAMP_FILE)[word] = value;
        let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        let listed = interpreter.pie_menu(&mut fx.world, SIM, LAMP).unwrap();
        assert!(listed.is_empty(), "word {word}: {listed:?}");
    }
}

#[test]
fn the_tiles_listing_finds_nothing_outside_the_lots_size() {
    // Tile (2, 2) is outside a lot of size 2.
    let mut fx = tile_lot();
    fx.world.size = 2;
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let listed = interpreter.pie_menu(&mut fx.world, SIM, LAMP).unwrap();
    assert!(listed.is_empty(), "{listed:?}");
}

#[test]
fn each_listing_on_the_tile_checks_again_for_a_pet() {
    // Menu 6's check makes me a dog (person data word 65) as it refuses;
    // the next listing, on object 3, is a pet's, which is not ported.
    let mut fx = tile_lot();
    fx.add(
        LAMP_FILE,
        4303,
        tree(vec![
            assign(scope::TEMPS, 0, scope::LITERAL, 99, 1),
            assign(18, 65, scope::LITERAL, 2, 2),
            ins(0, RETURN_FALSE, RETURN_FALSE, [0; 8]),
        ]),
    );
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.pie_menu(&mut fx.world, SIM, LAMP).unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("0x4585b1")),
        "{fault}"
    );
}

/// The lamp's action tree records, in the lamp's object data words 20 to 23, my person
/// data word 33 (my priority) and its parameters 4 to 6 (the priority's
/// two halves and the tree), and writes my temp 0.
fn recording_action(fx: &mut Fixture) {
    const MY_PERSON_DATA: u8 = 18;
    fx.add(
        LAMP_FILE,
        ACTION,
        // Seven parameters, as the original passes.
        shaped(
            0x8002,
            0,
            7,
            4,
            vec![
                assign(scope::STACK_DATA, 20, MY_PERSON_DATA, 33, 1),
                assign(scope::STACK_DATA, 21, scope::PARAMETERS, 4, 2),
                assign(scope::STACK_DATA, 22, scope::PARAMETERS, 5, 3),
                assign(scope::STACK_DATA, 23, scope::PARAMETERS, 6, 4),
                assign(scope::TEMPS, 0, scope::LITERAL, 99, RETURN_TRUE),
            ],
        ),
    );
}

/// An order for an entry that runs at once (0x45d240) runs its action
/// tree there and then, with my priority set to the action's for the run;
/// nothing is queued, and the action takes the next number afterwards.
#[test]
fn an_action_that_runs_at_once_runs_its_tree_with_its_priority() {
    let mut fx = menu_lot(vec![entry(3, 0x100), entry(4, 4)]);
    recording_action(&mut fx);
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.order(&mut fx.world, SIM, LAMP, 4), Ok(Some(1)));
    assert_eq!(fx.world.action_serial, 1);
    let lamp = &fx.world.objects[&LAMP];
    assert_eq!(&lamp.data[20..24], &[50, 50, 0, ACTION as i16]);
    let person = fx.sim().person.as_ref().unwrap();
    assert!(person.queue.is_empty());
    // My priority is back, and with no check tree run my temps are not
    // restored.
    assert_eq!(person.data[33], 7);
    assert_eq!(fx.sim().temps[0], 99);
}

/// A check tree that ran saves my temps, and they are restored after the
/// action ran (0x4582dd); a check that fails runs nothing and uses no
/// number.
#[test]
fn an_action_run_at_once_restores_my_temps_after_its_check() {
    let mut fx = menu_lot(vec![with_test(4, 4, 4300)]);
    recording_action(&mut fx);
    fx.add(
        LAMP_FILE,
        4300,
        tree(vec![ins(0, RETURN_TRUE, RETURN_TRUE, [0; 8])]),
    );
    fx.sim_mut().temps[0] = 11;
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.order(&mut fx.world, SIM, LAMP, 4), Ok(Some(1)));
    assert_eq!(fx.world.objects[&LAMP].data[23], ACTION as i16);
    assert_eq!(fx.sim().temps[0], 11);

    let mut fx = menu_lot(vec![with_test(4, 4, 4300)]);
    recording_action(&mut fx);
    fx.add(
        LAMP_FILE,
        4300,
        tree(vec![ins(0, RETURN_FALSE, RETURN_FALSE, [0; 8])]),
    );
    let before = fx.world.objects[&LAMP].data.clone();
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.order(&mut fx.world, SIM, LAMP, 4), Ok(None));
    assert_eq!(fx.world.objects[&LAMP].data, before);
    assert_eq!(fx.world.action_serial, 0);
}

/// While an action runs at once, another one for the same person runs
/// nothing (the pointer at 0x6e5294 is taken), and queueing one on that
/// person, which the original copies over the running action (0x45d5a5),
/// fails loudly.
#[test]
fn an_action_run_at_once_does_not_nest() {
    // Menu 4's tree pushes menu 4 again on me (the stack object, made
    // me), then records in my temp 1 that it went on.
    let mut fx = menu_lot(vec![entry(3, 0x100), entry(4, 4)]);
    fx.add(
        LAMP_FILE,
        ACTION,
        tree(vec![
            assign(scope::STACK_OBJECT, 0, scope::LITERAL, SIM as i16, 1),
            assign(scope::LOCALS, 0, scope::LITERAL, LAMP as i16, 2),
            ins(13, 3, 3, [4, 0, 1, 2, 0, 0, 0, 0]),
            assign(scope::TEMPS, 1, scope::LITERAL, 1, RETURN_TRUE),
        ]),
    );
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.order(&mut fx.world, SIM, LAMP, 4), Ok(Some(1)));
    assert_eq!(fx.sim().temps[1], 1);
    assert_eq!(fx.world.action_serial, 1);
    assert!(fx.sim().person.as_ref().unwrap().queue.is_empty());

    // Menu 4's tree queues menu 3 on me.
    let mut fx = menu_lot(vec![entry(3, 0x100), entry(4, 4)]);
    fx.add(
        LAMP_FILE,
        ACTION,
        tree(vec![
            assign(scope::STACK_OBJECT, 0, scope::LITERAL, SIM as i16, 1),
            assign(scope::LOCALS, 0, scope::LITERAL, LAMP as i16, 2),
            ins(13, RETURN_TRUE, RETURN_TRUE, [3, 0, 1, 2, 0, 0, 0, 0]),
        ]),
    );
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.order(&mut fx.world, SIM, LAMP, 4).unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("0x45d5a5")),
        "{fault}"
    );
}

/// An action run at once whose tree fails puts my priority back.
#[test]
fn an_action_run_at_once_that_fails_puts_my_priority_back() {
    let mut fx = menu_lot(vec![entry(3, 0x100), entry(4, 4)]);
    // The tree names a primitive.
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
        .action = 4;
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert!(interpreter.order(&mut fx.world, SIM, LAMP, 4).is_err());
    assert_eq!(fx.sim().person.as_ref().unwrap().data[33], 7);

    // The tree queues an action on me while it runs.
    let mut fx = menu_lot(vec![entry(3, 0x100), entry(4, 4)]);
    fx.add(
        LAMP_FILE,
        ACTION,
        tree(vec![
            assign(scope::STACK_OBJECT, 0, scope::LITERAL, SIM as i16, 1),
            assign(scope::LOCALS, 0, scope::LITERAL, LAMP as i16, 2),
            ins(13, RETURN_TRUE, RETURN_TRUE, [3, 0, 1, 2, 0, 0, 0, 0]),
        ]),
    );
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert!(interpreter.order(&mut fx.world, SIM, LAMP, 4).is_err());
    assert_eq!(fx.sim().person.as_ref().unwrap().data[33], 7);
}
