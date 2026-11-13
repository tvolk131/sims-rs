//! Stepping, branching, calls, returns and argument passing.

use crate::support::*;
use sims_formats::saved_objects::Frame;
use sims_vm::{Location, Status, Trap};

/// `my data[index] = value`, then go to `next`.
fn set_data(index: i16, value: i16, next: u8) -> sims_formats::behavior::Instruction {
    ins(
        2,
        next,
        next,
        expr(scope::MY_DATA, index, op::ASSIGN, scope::LITERAL, value),
    )
}

/// `my data[index] = source[source_index]`, then go to `next`.
fn copy_data(
    index: i16,
    source: u8,
    source_index: i16,
    next: u8,
) -> sims_formats::behavior::Instruction {
    ins(
        2,
        next,
        next,
        expr(scope::MY_DATA, index, op::ASSIGN, source, source_index),
    )
}

/// An Expression that is true when `left > right`.
fn compare(
    left: i16,
    right: i16,
    on_true: u8,
    on_false: u8,
) -> sims_formats::behavior::Instruction {
    ins(
        2,
        on_true,
        on_false,
        expr(scope::LITERAL, left, op::GREATER, scope::LITERAL, right),
    )
}

#[test]
fn trees_return_true_and_false() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![compare(1, 0, RETURN_TRUE, RETURN_FALSE)]),
    );
    assert_eq!(fx.run(), Ok(true));
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![compare(0, 1, RETURN_TRUE, RETURN_FALSE)]),
    );
    assert_eq!(fx.run(), Ok(false));
    // The run's frame is gone afterwards.
    assert!(fx.lamp().stack.is_empty());
}

#[test]
fn branches_go_to_nodes() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            compare(1, 0, 2, 1),
            set_data(40, 1, RETURN_TRUE),
            set_data(40, 2, RETURN_FALSE),
        ]),
    );
    assert_eq!(fx.run(), Ok(false));
    assert_eq!(fx.lamp().data[40], 2);
}

#[test]
fn the_error_branch_faults_where_it_was_taken() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![compare(1, 0, 1, 1), compare(1, 0, 253, RETURN_TRUE)]),
    );
    let fault = fx.run().unwrap_err();
    assert_eq!(fault.trap, Trap::ErrorBranch);
    assert_eq!(fault.object, LAMP);
    assert_eq!(
        fault.at,
        Some(Location {
            tree: 4096,
            node: 1
        })
    );
    // The faulting frame stays on the stack.
    assert_eq!(fx.lamp().stack.len(), 1);
    let text = fault.to_string();
    assert!(text.contains("object 1, tree 0x1000 node 1"), "{text}");
}

#[test]
fn branches_past_the_tree_or_into_reserved_targets_fault() {
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, tree(vec![compare(1, 0, 5, 5)]));
    assert_eq!(
        trap(fx.run()),
        Trap::InvalidBranch {
            target: 5,
            count: 1
        }
    );
    for target in [251, 252] {
        let mut fx = Fixture::new();
        fx.add(LAMP_FILE, 4096, tree(vec![compare(1, 0, target, target)]));
        assert_eq!(code(fx.run()), Some(0x44c));
    }
}

#[test]
fn a_return_branches_the_caller_on_the_returned_value() {
    for (value, expected) in [(true, 1), (false, 2)] {
        let mut fx = Fixture::new();
        let (left, right) = if value { (1, 0) } else { (0, 1) };
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![
                ins(4097, 1, 2, [0; 8]),
                set_data(40, 1, RETURN_TRUE),
                set_data(40, 2, RETURN_TRUE),
            ]),
        )
        .add(
            LAMP_FILE,
            4097,
            tree(vec![compare(left, right, RETURN_TRUE, RETURN_FALSE)]),
        );
        assert_eq!(fx.run(), Ok(true));
        assert_eq!(fx.lamp().data[40], expected);
    }
}

#[test]
fn returns_chain_through_several_frames() {
    let mut fx = Fixture::new();
    // 4096 calls 4097, which calls the global 256, which calls the
    // semi-global 8192; each passes the result straight back.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(4097, RETURN_FALSE, RETURN_TRUE, [0; 8])]),
    )
    .add(
        LAMP_FILE,
        4097,
        tree(vec![ins(256, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    )
    .add(
        GLOBAL,
        256,
        tree(vec![ins(8192, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    )
    .add(
        SEMI_FILE,
        8192,
        tree(vec![
            set_data(41, 7, 1),
            compare(1, 0, RETURN_TRUE, RETURN_FALSE),
        ]),
    );
    // 4096 inverts the result.
    assert_eq!(fx.run(), Ok(false));
    assert_eq!(fx.lamp().data[41], 7);
}

#[test]
fn a_semi_global_call_from_a_global_tree_resolves_from_the_owner() {
    let mut fx = Fixture::new();
    // The Sim's file has no semi-global, so the same global tree that
    // reaches 8192 from the lamp misses it from the Sim.
    fx.add(
        GLOBAL,
        256,
        tree(vec![ins(8192, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    )
    .add(
        SEMI_FILE,
        8192,
        tree(vec![compare(1, 0, RETURN_TRUE, RETURN_TRUE)]),
    );
    assert_eq!(fx.run_with(LAMP, 256, 0, &[], 100), Ok(true));
    let fault = fx.run_with(SIM, 256, 0, &[], 100).unwrap_err();
    assert_eq!(fault.trap, Trap::MissingTree { id: 8192 });
    assert_eq!(fault.at, Some(Location { tree: 256, node: 0 }));
}

#[test]
fn missing_trees_fault() {
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, tree(vec![ins(4500, 1, 1, [0; 8])]));
    let fault = fx.run().unwrap_err();
    assert_eq!(fault.trap, Trap::MissingTree { id: 4500 });
    assert_eq!(
        fault.at,
        Some(Location {
            tree: 4096,
            node: 0
        })
    );
    assert!(fault.to_string().contains("tree 0x1194 is missing"));
    // Entering a missing tree names it.
    let fault = fx.run_with(LAMP, 4600, 0, &[], 10).unwrap_err();
    assert_eq!(fault.trap, Trap::MissingTree { id: 4600 });
    assert_eq!(
        fault.at,
        Some(Location {
            tree: 4600,
            node: 0
        })
    );
    assert!(fx.lamp().stack.len() <= 1);
}

#[test]
fn arguments_are_read_contiguously_from_the_operands_on() {
    let mut fx = Fixture::new();
    // The callee takes six arguments; the call's eight operand bytes give
    // four, and the next instruction's first four bytes the other two.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            ins(
                4097,
                RETURN_TRUE,
                RETURN_FALSE,
                [1, 0, 2, 0, 0xfd, 0xff, 4, 0],
            ),
            ins(0x0102, 0x03, 0x04, [0; 8]),
        ]),
    )
    .add(
        LAMP_FILE,
        4097,
        shaped(
            0x8002,
            0,
            6,
            0,
            (0..6)
                .map(|i| {
                    copy_data(
                        40 + i,
                        scope::PARAMETERS,
                        i,
                        if i == 5 { RETURN_TRUE } else { i as u8 + 1 },
                    )
                })
                .collect(),
        ),
    );
    assert_eq!(fx.run(), Ok(true));
    assert_eq!(&fx.lamp().data[40..46], &[1, 2, -3, 4, 0x0102, 0x0403]);
}

#[test]
fn an_argument_read_past_the_end_of_the_tree_is_unsupported() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(4097, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    )
    .add(
        LAMP_FILE,
        4097,
        shaped(
            0x8002,
            0,
            5,
            0,
            vec![compare(1, 0, RETURN_TRUE, RETURN_TRUE)],
        ),
    );
    assert!(matches!(trap(fx.run()), Trap::Unsupported(_)));
}

#[test]
fn all_0xff_operands_pass_the_temps() {
    let mut fx = Fixture::new();
    fx.lamp_mut().temps = vec![11, 12, 13, 14, 15, 16, 17, 18];
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(4097, RETURN_TRUE, RETURN_FALSE, [255; 8])]),
    )
    .add(
        LAMP_FILE,
        4097,
        shaped(
            0x8002,
            0,
            3,
            0,
            (0..3)
                .map(|i| {
                    copy_data(
                        40 + i,
                        scope::PARAMETERS,
                        i,
                        if i == 2 { RETURN_TRUE } else { i as u8 + 1 },
                    )
                })
                .collect(),
        ),
    );
    assert_eq!(fx.run(), Ok(true));
    assert_eq!(&fx.lamp().data[40..43], &[11, 12, 13]);

    // More than eight arguments can't come from the temps.
    fx.add(
        LAMP_FILE,
        4097,
        shaped(
            0x8002,
            0,
            9,
            0,
            vec![compare(1, 0, RETURN_TRUE, RETURN_TRUE)],
        ),
    );
    assert!(matches!(trap(fx.run()), Trap::Unsupported(_)));
}

#[test]
fn formats_limit_arguments_and_locals() {
    // 0x8000: four arguments and no locals, whatever the header says.
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        shaped(
            0x8000,
            0,
            0,
            9,
            vec![
                copy_data(40, scope::PARAMETERS, 3, 1),
                copy_data(41, scope::LOCALS, 0, RETURN_TRUE),
            ],
        ),
    );
    assert_eq!(code(fx.run()), Some(8));
    assert_eq!(fx.lamp().data[40], 0);
    // 0x8001: at most four of each.
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        shaped(
            0x8001,
            0,
            6,
            6,
            vec![
                copy_data(40, scope::LOCALS, 3, 1),
                copy_data(41, scope::PARAMETERS, 4, RETURN_TRUE),
            ],
        ),
    );
    assert_eq!(code(fx.run()), Some(8));
    // 0x8002: the local count is a byte (0x0102 gives 2 locals).
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        shaped(
            0x8002,
            0,
            0,
            0x0102,
            vec![
                copy_data(40, scope::LOCALS, 1, 1),
                copy_data(41, scope::LOCALS, 2, RETURN_TRUE),
            ],
        ),
    );
    assert_eq!(code(fx.run()), Some(8));
}

#[test]
fn indexed_trees_enter_at_their_first_argument() {
    let entries = |fx: &mut Fixture, entry: [u8; 2]| {
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                4097,
                RETURN_TRUE,
                RETURN_FALSE,
                [entry[0], entry[1], 0, 0, 0, 0, 0, 0],
            )]),
        )
        .add(
            LAMP_FILE,
            4097,
            shaped(
                0x8002,
                4,
                4,
                0,
                vec![
                    set_data(40, 10, RETURN_TRUE),
                    set_data(40, 11, RETURN_TRUE),
                    set_data(40, 12, RETURN_TRUE),
                ],
            ),
        );
        fx.run()
    };
    let mut fx = Fixture::new();
    assert_eq!(entries(&mut fx, [1, 0]), Ok(true));
    assert_eq!(fx.lamp().data[40], 11);
    // Past the end clamps to the last node.
    assert_eq!(entries(&mut fx, [9, 0]), Ok(true));
    assert_eq!(fx.lamp().data[40], 12);
    // A negative entry is unsupported.
    assert!(matches!(
        trap(entries(&mut fx, [0xff, 0xff])),
        Trap::Unsupported(_)
    ));
    // run_tree enters at its first argument.
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4097,
        shaped(
            0x8002,
            4,
            1,
            0,
            vec![set_data(40, 10, RETURN_TRUE), set_data(40, 11, RETURN_TRUE)],
        ),
    );
    assert_eq!(fx.run_with(LAMP, 4097, 0, &[1, 5], 10), Ok(true));
    assert_eq!(fx.lamp().data[40], 11);
}

#[test]
fn entry_arguments_are_padded_and_cut_to_the_tree() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        shaped(
            0x8002,
            0,
            2,
            0,
            vec![
                copy_data(40, scope::PARAMETERS, 0, 1),
                copy_data(41, scope::PARAMETERS, 1, RETURN_TRUE),
            ],
        ),
    );
    assert_eq!(fx.run_with(LAMP, 4096, 0, &[7], 10), Ok(true));
    assert_eq!(&fx.lamp().data[40..42], &[7, 0]);
    fx.add(
        LAMP_FILE,
        4096,
        shaped(
            0x8002,
            0,
            1,
            0,
            vec![copy_data(41, scope::PARAMETERS, 1, RETURN_TRUE)],
        ),
    );
    assert_eq!(code(fx.run_with(LAMP, 4096, 0, &[7, 8], 10)), Some(8));
}

#[test]
fn locals_belong_to_their_frame_and_start_at_zero() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            ins(
                2,
                1,
                1,
                expr(scope::LOCALS, 0, op::ASSIGN, scope::LITERAL, 9),
            ),
            ins(4097, 2, 2, [0; 8]),
            copy_data(40, scope::LOCALS, 0, RETURN_TRUE),
        ]),
    )
    .add(
        LAMP_FILE,
        4097,
        tree(vec![
            copy_data(41, scope::LOCALS, 0, 1),
            ins(
                2,
                RETURN_TRUE,
                RETURN_TRUE,
                expr(scope::LOCALS, 0, op::ASSIGN, scope::LITERAL, 5),
            ),
        ]),
    );
    assert_eq!(fx.run(), Ok(true));
    assert_eq!(fx.lamp().data[40], 9);
    assert_eq!(fx.lamp().data[41], 0);
}

#[test]
fn calls_keep_the_stack_object_and_parameters_are_writable() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            ins(4097, 1, 1, [3, 0, 0, 0, 0, 0, 0, 0]),
            copy_data(40, scope::STACK_OBJECT, 0, RETURN_TRUE),
        ]),
    )
    .add(
        LAMP_FILE,
        4097,
        tree(vec![
            copy_data(41, scope::STACK_OBJECT, 0, 1),
            ins(
                2,
                2,
                2,
                expr(scope::PARAMETERS, 0, op::ADD, scope::LITERAL, 1),
            ),
            copy_data(42, scope::PARAMETERS, 0, 3),
            // Changing the stack object here does not reach the caller.
            ins(
                2,
                RETURN_TRUE,
                RETURN_TRUE,
                expr(scope::STACK_OBJECT, 0, op::ASSIGN, scope::LITERAL, 1),
            ),
        ]),
    );
    assert_eq!(fx.run(), Ok(true));
    assert_eq!(&fx.lamp().data[40..43], &[SIM as i16, SIM as i16, 4]);
}

#[test]
fn deep_recursion_overflows_the_stack() {
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, tree(vec![ins(4096, 1, 1, [0; 8])]));
    assert_eq!(trap(fx.run()), Trap::StackOverflow);
    assert_eq!(fx.lamp().stack.len(), sims_vm::MAX_STACK);
}

#[test]
fn the_budget_bounds_a_run() {
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, tree(vec![compare(1, 0, 0, 0)]));
    let fault = fx.run_with(LAMP, 4096, 0, &[], 50).unwrap_err();
    assert_eq!(fault.trap, Trap::BudgetExhausted { budget: 50 });
    assert_eq!(
        fault.at,
        Some(Location {
            tree: 4096,
            node: 0
        })
    );
}

#[test]
fn flagged_calls_are_unsupported() {
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, tree(vec![ins(0x9000, 1, 1, [0; 8])]));
    assert!(matches!(trap(fx.run()), Trap::Unsupported(_)));
}

#[test]
fn unimplemented_primitives_name_themselves() {
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, tree(vec![ins(11, 1, 1, [0; 8])]));
    let fault = fx.run().unwrap_err();
    assert_eq!(fault.trap, Trap::Unimplemented { opcode: 11 });
    let text = fault.to_string();
    assert!(
        text.contains("unimplemented primitive 11 (Get Distance To)"),
        "{text}"
    );
    for (opcode, name) in [(0, "Sleep"), (36, "Dialog"), (44, "Animate Sim")] {
        let mut fx = Fixture::new();
        fx.add(LAMP_FILE, 4096, tree(vec![ins(opcode, 1, 1, [0; 8])]));
        assert!(trap(fx.run()).to_string().contains(name));
    }
    assert_eq!(sims_vm::primitive_name(200), "unnamed primitive");
}

/// A saved frame running the lamp's tree 4096 at `node`.
fn saved(node: u16) -> Frame {
    Frame {
        stack_object: SIM as i16,
        tree: 4096,
        node,
        parameters: vec![0; 4],
        locals: vec![0; 4],
        primitive_state: 0,
        code_owner_type: LAMP_TYPE,
    }
}

#[test]
fn step_runs_a_saved_stack() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![set_data(40, 1, 1), set_data(41, 2, RETURN_FALSE)]),
    );
    fx.lamp_mut().stack.push(saved(1));
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(
        interpreter.step(&mut world, LAMP),
        Ok(Status::Returned(false))
    );
    let lamp = &world.objects[&LAMP];
    assert_eq!((lamp.data[40], lamp.data[41]), (80, 2));
    assert!(lamp.stack.is_empty());
    // An empty stack has nothing to step.
    let fault = interpreter.step(&mut world, LAMP).unwrap_err();
    assert_eq!(fault.at, None);
}

#[test]
fn run_tree_leaves_saved_frames_alone() {
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, tree(vec![set_data(40, 1, RETURN_TRUE)]));
    fx.add(LAMP_FILE, 4097, tree(vec![set_data(41, 2, RETURN_TRUE)]));
    fx.lamp_mut().stack.push(saved(0));
    assert_eq!(fx.run_with(LAMP, 4097, 0, &[], 10), Ok(true));
    assert_eq!(fx.lamp().stack.len(), 1);
    assert_eq!(fx.lamp().stack[0].node, 0);
    assert_eq!(fx.lamp().data[41], 2);
    assert_eq!(fx.lamp().data[40], 80);
}

#[test]
fn saved_primitive_state_is_unsupported() {
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, tree(vec![set_data(40, 1, RETURN_TRUE)]));
    let mut frame = saved(0);
    frame.primitive_state = 3;
    fx.lamp_mut().stack.push(frame);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    let fault = interpreter.step(&mut world, LAMP).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)));
    // Nor can a run return into such a frame.
    fx.add(LAMP_FILE, 4096, tree(vec![ins(4097, 1, 1, [0; 8])]));
    fx.add(LAMP_FILE, 4097, tree(vec![set_data(40, 1, RETURN_TRUE)]));
    fx.lamp_mut().stack.push(Frame {
        tree: 4097,
        ..saved(0)
    });
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    let fault = interpreter.step(&mut world, LAMP).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)));
}

#[test]
fn frames_need_a_known_code_owner() {
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, tree(vec![set_data(40, 1, RETURN_TRUE)]));
    fx.lamp_mut().stack.push(Frame {
        code_owner_type: 3,
        ..saved(0)
    });
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    let fault = interpreter.step(&mut world, LAMP).unwrap_err();
    assert!(matches!(fault.trap, Trap::Invalid(_)));
    // Frames run their code owner's trees, not the running object's.
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, tree(vec![set_data(40, 1, RETURN_TRUE)]));
    fx.sim_mut().stack.push(saved(0));
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(
        interpreter.step(&mut world, SIM),
        Ok(Status::Returned(true))
    );
    assert_eq!(world.objects[&SIM].data[40], 1);
    // An object whose type is not in the lot's table can't enter a tree.
    let mut fx = Fixture::new();
    fx.world.types.remove(&8);
    assert!(matches!(
        trap(fx.run_with(SIM, 4096, 0, &[], 10)),
        Trap::Invalid(_)
    ));
}
