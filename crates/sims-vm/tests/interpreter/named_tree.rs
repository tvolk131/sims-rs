//! Run Tree By Name (opcode 28, 0x44ba26) in modes 0 and 1: the named
//! tree of the stack object's type runs at once, to its end, on my thread
//! (mode 0) or on the stack object's (mode 1), and its result is the
//! branch. Synthetic; no game data.

use crate::support::*;
use sims_vm::Trap;

/// The lamp's tree named "Named".
const NAMED: u16 = 4300;
const SIM_TREE: u16 = 4096;
const MY_DATA: u8 = scope::MY_DATA;

/// A lot whose lamp has "Named", whose code is `code`, and whose Sim's
/// tree 4096 runs it by name in `mode` (string table 303 of the running
/// code's file, name 1). The Sim's temp 0 is 5 and the lamp's is 9.
fn lot(mode: u8, code: Vec<sims_formats::behavior::Instruction>) -> Fixture {
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, NAMED, tree(code));
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .names
        .insert(NAMED, "Named".to_owned());
    fx.program.libraries.get_mut(&SIM_FILE).unwrap().tree_names = vec!["Named".to_owned()];
    fx.add(
        SIM_FILE,
        SIM_TREE,
        tree(vec![ins(
            28,
            RETURN_TRUE,
            RETURN_FALSE,
            [47, 1, 0, 0, 1, mode, 0, 0],
        )]),
    );
    fx.sim_mut().temps[0] = 5;
    fx.lamp_mut().temps[0] = 9;
    fx
}

/// The named tree records its first argument in my data word 20 and its
/// stack object in my data word 21, then returns `result`.
fn recording(result: u8) -> Vec<sims_formats::behavior::Instruction> {
    vec![
        assign(MY_DATA, 20, scope::PARAMETERS, 0, 1),
        assign(MY_DATA, 21, scope::STACK_OBJECT, 0, result),
    ]
}

/// Mode 0 runs the lamp's tree on my thread: I stay me, the lamp is its
/// stack object, my temps are its arguments, and its result is the
/// branch. My frames are as they were.
#[test]
fn mode_0_runs_the_tree_on_my_thread() {
    for (result, branch) in [(RETURN_TRUE, true), (RETURN_FALSE, false)] {
        let mut fx = lot(0, recording(result));
        let lamp_data = fx.lamp().data.clone();
        assert_eq!(
            fx.run_with(SIM, SIM_TREE, LAMP as i16, &[], 1000),
            Ok(branch)
        );
        assert_eq!(&fx.sim().data[20..22], &[5, LAMP as i16]);
        assert_eq!(fx.lamp().data, lamp_data);
        assert!(fx.sim().stack.is_empty());
    }
}

/// Mode 1 runs it on the lamp's thread, with the lamp's temps.
#[test]
fn mode_1_runs_the_tree_on_the_stack_objects_thread() {
    let mut fx = lot(1, recording(RETURN_FALSE));
    let sim_data = fx.sim().data.clone();
    assert_eq!(
        fx.run_with(SIM, SIM_TREE, LAMP as i16, &[], 1000),
        Ok(false)
    );
    assert_eq!(&fx.lamp().data[20..22], &[9, LAMP as i16]);
    assert_eq!(fx.sim().data, sim_data);
    assert!(fx.lamp().stack.is_empty());
}

/// Mode 1 when the stack object is me is the same run as mode 0: on my
/// thread, above my frames.
#[test]
fn mode_1_on_myself_runs_on_my_thread() {
    // The Sim's tree, with itself as stack object, names its own "Named".
    let mut fx = lot(1, recording(RETURN_TRUE));
    fx.add(
        SIM_FILE,
        NAMED,
        tree(vec![
            assign(MY_DATA, 20, scope::PARAMETERS, 0, 1),
            assign(MY_DATA, 21, scope::STACK_OBJECT, 0, RETURN_TRUE),
        ]),
    );
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .names
        .insert(NAMED, "Named".to_owned());
    assert_eq!(fx.run_with(SIM, SIM_TREE, SIM as i16, &[], 1000), Ok(true));
    assert_eq!(&fx.sim().data[20..22], &[5, SIM as i16]);
}

/// A named tree that would wait fails, as 0x4749b5 does (native error
/// 0x44f), and the failure names the mode and the thread.
#[test]
fn a_named_tree_run_at_once_cannot_wait() {
    let mut fx = lot(0, vec![ins(0, RETURN_TRUE, RETURN_FALSE, [0; 8])]);
    let fault = fx
        .run_with(SIM, SIM_TREE, LAMP as i16, &[], 1000)
        .unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("mode 0") && m.contains("object 2's thread")),
        "{fault}"
    );
}
