//! Add/Change Action String (primitive 50, 0x44cc07) outside a pie menu's
//! listing and autonomy's check trees.

use crate::support::*;
use sims_vm::{Interpreter, Status};

/// The tree each test's object runs as its thread.
const TREE: u16 = 4200;

/// Operands naming string `index` (from 1) of string table 300.
fn action_string(index: u8) -> [u8; 8] {
    let mut operands = [0; 8];
    operands[..2].copy_from_slice(&300u16.to_le_bytes());
    operands[4] = index;
    operands
}

/// A lot where `object`'s thread runs Add/Change Action String on string
/// `index`. Both the Sim's and the lamp's files have table 300 with one
/// string, and the Sim's running action has flags 0x41.
fn named(object: u16, file: usize, index: u8) -> Fixture {
    let mut fx = Fixture::new();
    for resource in [SIM_FILE, LAMP_FILE] {
        fx.program
            .libraries
            .get_mut(&resource)
            .unwrap()
            .strings
            .insert(300, vec!["Eat".to_owned()]);
    }
    fx.add(
        file,
        TREE,
        tree(vec![ins(
            50,
            RETURN_TRUE,
            RETURN_FALSE,
            action_string(index),
        )]),
    );
    fx.sim_mut().person.as_mut().unwrap().active.flags = 0x41;
    fx.thread(object, TREE, &[]);
    fx
}

fn turn(fx: &mut Fixture, object: u16) -> Result<Status, sims_vm::Fault> {
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.turn(&mut fx.world, object, 100)
}

fn active_flags(fx: &Fixture) -> u32 {
    fx.sim().person.as_ref().unwrap().active.flags
}

#[test]
fn a_persons_thread_names_its_running_action() {
    // 0x6c4d6c is set in a thread's run, so the string names the running
    // action (0x41f1ae), which sets its flag 0x80 and nothing else.
    let mut fx = named(SIM, SIM_FILE, 1);
    assert_eq!(turn(&mut fx, SIM), Ok(Status::Returned(true)));
    assert_eq!(active_flags(&fx), 0xc1);
}

#[test]
fn another_objects_thread_changes_nothing() {
    let mut fx = named(LAMP, LAMP_FILE, 1);
    let before = format!("{:?}", fx.sim());
    assert_eq!(turn(&mut fx, LAMP), Ok(Status::Returned(true)));
    assert_eq!(format!("{:?}", fx.sim()), before);
}

#[test]
fn a_missing_string_or_a_run_at_once_fails_loudly() {
    // A string the file lacks: what the original makes of it is not read.
    let mut fx = named(SIM, SIM_FILE, 2);
    let fault = turn(&mut fx, SIM).unwrap_err();
    assert!(fault.to_string().contains("0x4704fa"), "{fault}");
    assert_eq!(active_flags(&fx), 0x41);
    // A tree run at once that is not a check tree.
    let mut fx = named(SIM, LAMP_FILE, 1);
    let fault = fx.run_with(LAMP, TREE, SIM as i16, &[], 100).unwrap_err();
    assert!(fault.to_string().contains("0x474a37"), "{fault}");
    assert_eq!(active_flags(&fx), 0x41);
}

#[test]
fn a_string_with_tokens_fails_loudly_before_anything_changes() {
    // The string is formatted (0x44bf1a) before 0x44cc9a looks at where
    // it goes, and a pass can raise an error (native 0x1c for a
    // non-person, 0x44c57b), so a string with "$" fails loudly in a
    // thread's run as well: the person's and another object's.
    for (object, file) in [(SIM, SIM_FILE), (LAMP, LAMP_FILE)] {
        let mut fx = named(object, file, 1);
        for resource in [SIM_FILE, LAMP_FILE] {
            fx.program
                .libraries
                .get_mut(&resource)
                .unwrap()
                .strings
                .insert(300, vec!["$TokenNameLocal:0".to_owned()]);
        }
        let fault = turn(&mut fx, object).unwrap_err();
        assert!(fault.to_string().contains("0x44bf1a"), "{fault}");
        assert_eq!(active_flags(&fx), 0x41);
    }
}
