//! Push Interaction (primitive 13, 0x44cd2b): a script queues an
//! interaction of an object on the person in its stack object.
//! Synthetic; no game data.

use crate::support::*;
use sims_formats::behavior::{Interaction, InteractionTable};

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
    // An entry that runs at once.
    let mut fx = lot([4, 0, 1, 0, 0, 0, 0, 0]);
    assert!(matches!(
        trap(push(&mut fx, &[LAMP as i16])),
        sims_vm::Trap::Unsupported(m) if m.contains("0x45d240")
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
    // An entry that runs at once is not ported.
    assert!(matches!(
        interpreter.order(&mut fx.world, SIM, LAMP, 4),
        Err(sims_vm::Fault { trap: sims_vm::Trap::Unsupported(m), .. }) if m.contains("0x45d240")
    ));
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
