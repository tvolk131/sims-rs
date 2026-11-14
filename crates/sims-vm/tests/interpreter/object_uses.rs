//! The objects a person uses: Run Tree By Name in mode 2 starts a use
//! (0x45e59b), each return looks at the last one (0x45e5fd), and the end
//! of the last use ends the person's running action (0x45e6cd).

use crate::support::*;
use sims_formats::saved_objects::{Interaction, MotiveDelta, ObjectUse};
use sims_vm::{Interpreter, Status};

/// The lamp's tree "Use": sleep for as many turns as its first argument
/// says, then return true.
const USE: u16 = 4300;
const SIM_TREE: u16 = 4096;

/// The Sim's thread runs the lamp's "Use" with the lamp as its stack
/// object. The Sim's first temp, the tree's first argument, is how many
/// turns the tree sleeps.
fn using() -> Fixture {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        USE,
        tree(vec![ins(0, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .names
        .insert(USE, "Use".to_owned());
    fx.program.libraries.get_mut(&SIM_FILE).unwrap().tree_names = vec!["Use".to_owned()];
    // Run Tree By Name: string table 303 of the running code's file, name
    // 1, mode 2.
    fx.add(
        SIM_FILE,
        SIM_TREE,
        tree(vec![ins(
            28,
            RETURN_TRUE,
            RETURN_FALSE,
            [47, 1, 0, 0, 1, 2, 0, 0],
        )]),
    );
    fx.thread(SIM, SIM_TREE, &[]);
    fx.sim_mut().stack[0].stack_object = LAMP as i16;
    fx.sim_mut().temps[0] = 1;
    fx.lamp_mut().data[62] = 5;
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[73] = 0;
    person.active = Interaction {
        uid: 9,
        caller: SIM as i16,
        target: LAMP as i16,
        icon: 3,
        table_index: 4,
        args: [1, 2, 3, 4],
        priority: 50,
        action: 7,
        attenuation: 0.5,
        flags: 0x21,
    };
    person.motive_deltas.push(MotiveDelta {
        motive: 1,
        delta: 0.25,
        stop_at: 100.0,
    });
    fx
}

fn turn(fx: &mut Fixture) -> Status {
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.turn(&mut fx.world, SIM, 100).unwrap()
}

/// Interactions do not compare; their debug forms do.
fn same(a: &Interaction, b: &Interaction) -> bool {
    format!("{a:?}") == format!("{b:?}")
}

fn uses(fx: &Fixture) -> Vec<(i16, i32, u8)> {
    let person = fx.sim().person.as_ref().unwrap();
    person
        .object_uses
        .iter()
        .map(|u| (u.target, u.stack_depth, u.unknown))
        .collect()
}

#[test]
fn a_person_that_runs_a_tree_of_an_object_uses_it_until_the_tree_returns() {
    let mut fx = using();
    let before = fx.sim().person.clone().unwrap();
    // The tree is pushed and sleeps. The Sim uses the lamp, noted with
    // the number of frames once the tree was pushed, and the lamp counts
    // one more user.
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(fx.sim().stack.len(), 2);
    assert_eq!(uses(&fx), [(LAMP as i16, 2, 0)]);
    assert_eq!(fx.lamp().data[62], 6);
    assert!(same(
        &fx.sim().person.as_ref().unwrap().active,
        &before.active
    ));
    // The tree returns. The use ends, the lamp counts one fewer, and with
    // no use left the running action ends.
    assert_eq!(turn(&mut fx), Status::Returned(true));
    assert_eq!(uses(&fx), []);
    assert_eq!(fx.lamp().data[62], 5);
    let person = fx.sim().person.as_ref().unwrap();
    // The old action had a target, so it is the last action now, with
    // flag 0x20 holding the Sim's last result before the return: the
    // Sleep's, true.
    let last = before.active.clone();
    assert!(same(&person.last, &last));
    // The running action is the empty one: no target, menu entry -1.
    assert_eq!((person.active.target, person.active.table_index), (0, -1));
    assert_eq!(person.active.priority, 0);
    // Word 33 takes its priority; words 71 and 72 are cleared, and so are
    // the motive changes. Nothing else of the person changes.
    assert_eq!(person.data[33], 0);
    assert_eq!((person.data[71], person.data[72]), (0, 0));
    assert!(person.motive_deltas.is_empty());
    let mut expected = before.data.clone();
    (expected[33], expected[71], expected[72]) = (0, 0, 0);
    assert_eq!(person.data, expected);
    // The return's own result is written after: true.
    assert!(fx.sim().last_result);
}

#[test]
fn the_last_result_before_the_return_goes_into_the_last_action() {
    // "Use" sleeps, then tests a word that is false and returns true from
    // the false branch. The last result before the return is false, so
    // flag 0x20 of the last action is cleared.
    let mut fx = using();
    fx.add(
        LAMP_FILE,
        USE,
        tree(vec![
            ins(0, 1, RETURN_FALSE, [0; 8]),
            ins(
                2,
                RETURN_FALSE,
                RETURN_TRUE,
                expr(scope::MY_DATA, 0, op::EQUAL, scope::LITERAL, 1),
            ),
        ]),
    );
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(turn(&mut fx), Status::Returned(true));
    assert_eq!(fx.sim().person.as_ref().unwrap().last.flags, 0x01);
    assert!(fx.sim().last_result);
}

#[test]
fn an_action_without_a_target_does_not_become_the_last_action() {
    let mut fx = using();
    fx.sim_mut().person.as_mut().unwrap().active.target = 0;
    let last = fx.sim().person.as_ref().unwrap().last.clone();
    assert_eq!(turn(&mut fx), Status::Yielded);
    assert_eq!(turn(&mut fx), Status::Returned(true));
    let person = fx.sim().person.as_ref().unwrap();
    assert!(same(&person.last, &last));
    assert_eq!(person.active.table_index, -1);
}

#[test]
fn a_use_ends_only_when_its_frame_is_gone() {
    let mut fx = using();
    // A use noted deeper than the thread is: it is ended by the first
    // return, without the noting at person +0xa54.
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.object_uses = vec![
        ObjectUse {
            target: LAMP as i16,
            stack_depth: 1,
            unknown: 0,
        },
        ObjectUse {
            target: LAMP as i16,
            stack_depth: 9,
            unknown: 0,
        },
    ];
    assert_eq!(turn(&mut fx), Status::Yielded);
    // The push added a third; nothing returned yet.
    assert_eq!(
        uses(&fx),
        [
            (LAMP as i16, 1, 0),
            (LAMP as i16, 9, 0),
            (LAMP as i16, 2, 0)
        ]
    );
    assert_eq!(fx.lamp().data[62], 6);
    // The tree returns: one frame is left, so the last use (made at 2)
    // ends. Then the Sim's own tree returns, no frame is left, and the use
    // made at 9 ends. The one made at 1 never does in this run, as each
    // return looks at the last use only.
    assert_eq!(turn(&mut fx), Status::Returned(true));
    assert_eq!(uses(&fx), [(LAMP as i16, 1, 0)]);
    assert_eq!(fx.lamp().data[62], 4);
    // The list did not empty, so the running action goes on.
    assert_eq!(fx.sim().person.as_ref().unwrap().active.table_index, 4);
}

#[test]
fn what_the_end_of_a_use_does_not_port_fails_loudly() {
    // A use with its flag set: the original would note its object at
    // person +0xa54.
    let mut fx = using();
    assert_eq!(turn(&mut fx), Status::Yielded);
    fx.sim_mut().person.as_mut().unwrap().object_uses[0].unknown = 1;
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.turn(&mut fx.world, SIM, 100).unwrap_err();
    assert!(fault.to_string().contains("+0xa54"), "{fault}");
    // Person word 73 set when the action ends: the original works out the
    // person's footprint again (0x433d16).
    let mut fx = using();
    assert_eq!(turn(&mut fx), Status::Yielded);
    fx.sim_mut().person.as_mut().unwrap().data[73] = 1;
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.turn(&mut fx.world, SIM, 100).unwrap_err();
    assert!(fault.to_string().contains("0x433d16"), "{fault}");
    // A used object that is gone.
    let mut fx = using();
    assert_eq!(turn(&mut fx), Status::Yielded);
    fx.sim_mut().person.as_mut().unwrap().object_uses[0].target = 77;
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.turn(&mut fx.world, SIM, 100).unwrap_err();
    assert!(fault.to_string().contains("is gone"), "{fault}");
}

#[test]
fn an_object_that_runs_a_tree_of_another_uses_nothing() {
    // An object that is not a person pushes the tree the same way but
    // keeps no list, and the used object's count does not move.
    let mut fx = using();
    fx.add(
        LAMP_FILE,
        SIM_TREE,
        tree(vec![ins(
            28,
            RETURN_TRUE,
            RETURN_FALSE,
            [47, 1, 0, 0, 1, 2, 0, 0],
        )]),
    );
    fx.program.libraries.get_mut(&LAMP_FILE).unwrap().tree_names = vec!["Use".to_owned()];
    fx.add_object(3);
    fx.thread(3, SIM_TREE, &[]);
    fx.world.objects.get_mut(&3).unwrap().stack[0].stack_object = LAMP as i16;
    fx.world.objects.get_mut(&3).unwrap().temps[0] = 0;
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(
        interpreter.turn(&mut fx.world, 3, 100),
        Ok(Status::Returned(true))
    );
    assert_eq!(fx.lamp().data[62], 5);
}
