//! Run Functional Tree (opcode 20, 0x44f681): the running object runs the
//! stack object's action for a function as a call. Synthetic; no game
//! data.

use crate::support::*;
use sims_formats::execution::ObjectFunction;
use sims_vm::{Interpreter, Status, Trap};

/// The lamp's action for function 0 (callback 0x12): sleeps once, then
/// returns true.
const SLEEPS: u16 = 4300;
/// Its action for function 1 (callback 0x13): returns false.
const FALSE: u16 = 4301;
/// Its action for function 8 (callback 0x1a): returns true.
const TRUE: u16 = 4302;
const SIM_TREE: u16 = 4096;

/// The operands for `function` with flag byte `flag`.
fn functional(function: i16, flag: u8) -> [u8; 8] {
    let [a, b] = function.to_le_bytes();
    [a, b, flag, 0, 0, 0, 0, 0]
}

/// The Sim's thread runs one Run Functional Tree with the lamp as its
/// stack object; the lamp's function table has actions for callbacks
/// 0x12, 0x13 and 0x1a only.
fn sim_runs(operands: [u8; 8]) -> Fixture {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        SLEEPS,
        tree(vec![ins(0, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    fx.add(
        LAMP_FILE,
        FALSE,
        tree(vec![expression(
            scope::LITERAL,
            1,
            op::EQUAL,
            scope::LITERAL,
            2,
        )]),
    );
    fx.add(
        LAMP_FILE,
        TRUE,
        tree(vec![expression(
            scope::LITERAL,
            1,
            op::EQUAL,
            scope::LITERAL,
            1,
        )]),
    );
    let table = fx
        .program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap();
    table.resize(
        0x1b,
        ObjectFunction {
            condition: 0,
            action: 0,
        },
    );
    table[0x12].action = SLEEPS;
    table[0x13].action = FALSE;
    table[0x1a].action = TRUE;
    fx.add(
        SIM_FILE,
        SIM_TREE,
        tree(vec![ins(20, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.thread(SIM, SIM_TREE, &[]);
    fx.sim_mut().stack[0].stack_object = LAMP as i16;
    fx.sim_mut().temps[..4].copy_from_slice(&[1, 2, 3, 4]);
    let person = fx.sim_mut().person.as_mut().unwrap();
    person.data[65] = 1;
    person.data[73] = 0;
    fx.lamp_mut().data[62] = 5;
    fx
}

fn turn(fx: &mut Fixture) -> Result<Status, sims_vm::Fault> {
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.turn(&mut fx.world, SIM, 100)
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
fn the_action_runs_as_a_call_and_its_result_is_the_branch() {
    let mut fx = sim_runs(functional(0, 0));
    // The lamp's action is pushed with the lamp as its stack object and
    // the Sim's temps as its arguments (its Sleep counts the first down
    // from 1); the Sim uses the lamp.
    assert_eq!(turn(&mut fx).unwrap(), Status::Yielded);
    let frame = fx.sim().stack.last().unwrap();
    assert_eq!(
        (frame.tree, frame.stack_object, frame.parameters.as_slice()),
        (SLEEPS, LAMP as i16, &[0, 2, 3, 4][..])
    );
    // The calling frame's state is left alone.
    assert_eq!(fx.sim().stack[0].primitive_state, 0);
    assert_eq!(uses(&fx), [(LAMP as i16, 2, 0)]);
    assert_eq!(fx.lamp().data[62], 6);
    // The action returns true: the use ends and the instruction takes its
    // true branch.
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    assert_eq!(uses(&fx), []);
    assert_eq!(fx.lamp().data[62], 5);
    // An action that returns false takes the false branch.
    let mut fx = sim_runs(functional(1, 0));
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(false));
}

#[test]
fn a_function_with_no_action_is_false() {
    // Function 2 names callback 0x14, which has no action; function 14
    // names 0x1d, past the table's end; 15 and -1 name none.
    for function in [2, 14, 15, -1] {
        let mut fx = sim_runs(functional(function, 0));
        assert_eq!(
            turn(&mut fx).unwrap(),
            Status::Returned(false),
            "{function}"
        );
        assert_eq!(fx.sim().stack.len(), 0);
        assert_eq!(uses(&fx), []);
    }
}

#[test]
fn function_8_needs_a_person_whose_word_65_is_at_most_1() {
    let mut fx = sim_runs(functional(8, 0));
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(true));
    let mut fx = sim_runs(functional(8, 0));
    fx.sim_mut().person.as_mut().unwrap().data[65] = 2;
    assert_eq!(turn(&mut fx).unwrap(), Status::Returned(false));
    // An object that is not a person may not ask: the lamp runs it on
    // itself and gets false. Function 1 runs (its action, here, writes
    // the lamp's temp 6 and returns false), notes no flag and counts no
    // use.
    let mut fx = sim_runs(functional(8, 0));
    fx.add(
        LAMP_FILE,
        4303,
        tree(vec![assign(
            scope::TEMPS,
            6,
            scope::LITERAL,
            42,
            RETURN_FALSE,
        )]),
    );
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap()[0x13]
        .action = 4303;
    fx.add(
        LAMP_FILE,
        4200,
        tree(vec![ins(20, RETURN_TRUE, RETURN_FALSE, functional(8, 0))]),
    );
    fx.add(
        LAMP_FILE,
        4201,
        tree(vec![ins(20, RETURN_FALSE, RETURN_TRUE, functional(1, 1))]),
    );
    assert_eq!(fx.run_with(LAMP, 4200, LAMP as i16, &[], 100), Ok(false));
    assert_eq!(fx.lamp().temps[6], 0);
    assert_eq!(fx.run_with(LAMP, 4201, LAMP as i16, &[], 100), Ok(true));
    assert_eq!(fx.lamp().temps[6], 42);
    assert_eq!(fx.lamp().data[62], 5);
}

#[test]
fn a_missing_stack_object_is_error_0x15_and_flag_1_fails_loudly() {
    let mut fx = sim_runs(functional(15, 0));
    fx.sim_mut().stack[0].stack_object = 77;
    let fault = turn(&mut fx).unwrap_err();
    assert!(
        matches!(fault.trap, Trap::Native { code: 0x15, .. }),
        "{fault:?}"
    );
    // A person that notes the flag would note the object at person
    // +0xa54, which is not modeled.
    let mut fx = sim_runs(functional(0, 1));
    let fault = turn(&mut fx).unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("+0xa54")),
        "{fault:?}"
    );
}
