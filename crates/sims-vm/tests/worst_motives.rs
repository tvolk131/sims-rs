use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{Definition, Module, Object, Program, Routine, Snapshot, Start, Status, Vm, World};
use std::collections::BTreeMap;

fn program(operands: [u8; 8], target_kind: i16) -> Program {
    let mut caller_definition = vec![0; 10];
    caller_definition[9] = 2;
    let mut target_definition = caller_definition.clone();
    target_definition[9] = target_kind;
    Program {
        globals: "test".into(),
        modules: BTreeMap::from([(
            "test".into(),
            Module {
                source: "test".into(),
                object_definition_words: BTreeMap::from([
                    (1, caller_definition),
                    (2, target_definition),
                ]),
                routines: BTreeMap::from([(
                    4096,
                    Routine {
                        name: "rank motives".into(),
                        body: Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 0,
                            locals: 0,
                            instructions: vec![Instruction {
                                opcode: 0x21,
                                on_true: 254,
                                on_false: 253,
                                operands,
                            }],
                        },
                    },
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    }
}

fn world(values: [f32; 16]) -> World {
    World {
        objects: [1, 2]
            .into_iter()
            .map(|identity| {
                (
                    identity,
                    Object {
                        definition: Some(Definition {
                            source: "test".into(),
                            object: identity as u16,
                            guid: identity as u32,
                            slots: 0,
                        }),
                        person: Some(Default::default()),
                        motives: Some(values.map(f32::to_bits)),
                        temps: [101, 102, 103, 104, 105, 106, 107, 108],
                        ..Default::default()
                    },
                )
            })
            .collect(),
        ..Default::default()
    }
}

fn execute(program: &Program, world: World, stack_object: i16, check: bool) -> (Vm, u64) {
    let mut vm = Vm::new(world, 0x12345678);
    vm.lot_rng_state = Some(0x98765432);
    vm.crt_rng_state = Some(0xabcdef12);
    let thread = vm
        .spawn(
            program,
            Start {
                owner: "test",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object,
                args: vec![],
                check,
            },
        )
        .unwrap();
    assert_eq!(
        Snapshot::capture(program, &vm).restore(program).unwrap(),
        vm
    );
    vm.run_slice(program, thread, 1).unwrap();
    assert_eq!(vm.tick, 0);
    assert_eq!(vm.rng_state, 0x12345678);
    assert_eq!(vm.lot_rng_state, Some(0x98765432));
    assert_eq!(vm.crt_rng_state, Some(0xabcdef12));
    assert_eq!(
        Snapshot::capture(program, &vm).restore(program).unwrap(),
        vm
    );
    (vm, thread)
}

fn assert_ranking(values: [f32; 16], expected: [i16; 5]) {
    let program = program([0; 8], 2);
    let original = world(values);
    for check in [false, true] {
        let (vm, thread) = execute(&program, original.clone(), 2, check);
        assert_eq!(vm.threads[&thread].status, Status::Returned(true));
        let mut expected_world = original.clone();
        expected_world.objects.get_mut(&1).unwrap().temps[..5].copy_from_slice(&expected);
        assert_eq!(vm.world, expected_world);
    }
}

#[test]
fn ranks_raw_motives_without_advertisements_or_integer_rounding() {
    let mut values = [-1000.; 16];
    for (index, value) in [
        (5, -0.25),
        (6, -0.875),
        (7, -0.5),
        (8, 20.),
        (9, 10.),
        (13, 30.),
        (14, 40.),
        (15, 50.),
    ] {
        values[index] = value;
    }
    assert_ranking(values, [6, 7, 5, 9, 8]);
}

#[test]
fn retains_native_shortsort_tie_order() {
    assert_ranking([0.; 16], [6, 7, 8, 9, 13]);
    let mut values = [0.; 16];
    values[5] = 0.005;
    values[8] = -0.005;
    assert_ranking(values, [6, 7, 8, 9, 13]);
}

#[test]
fn tolerance_boundary_uses_single_precision_subtraction() {
    for (lower, upper, expected) in [
        (0., 0.01f32, [6, 5, 7, 8, 9]),
        (0., f32::from_bits(0.01f32.to_bits() + 1), [5, 6, 7, 8, 9]),
        (0., 0.015, [5, 6, 7, 8, 9]),
        (-2f32.powi(-31), 0.01, [6, 5, 7, 8, 9]),
    ] {
        let mut values = [0.; 16];
        for (position, index) in [5, 6, 7, 8, 9, 13, 14, 15].into_iter().enumerate() {
            values[index] = position as f32;
        }
        values[5] = lower;
        values[6] = upper;
        assert_ranking(values, expected);
    }
}

#[test]
fn nested_target_tree_uses_current_stack_person_and_executor_temporaries() {
    for selector in [0, 1] {
        let mut operands = [0; 8];
        operands[4] = selector;
        let mut program = program(operands, 2);
        let mut child = program.modules["test"].routines[&4096].clone();
        child.body.instructions.insert(
            0,
            Instruction {
                opcode: 2,
                on_true: 1,
                on_false: 255,
                operands: [0, 0, 3, 0, 0, 5, 10, 7],
            },
        );
        let caller = program.modules.get_mut("test").unwrap();
        caller.strings.insert(303, vec![child.name.clone()]);
        caller.routines.get_mut(&4096).unwrap().body.instructions = vec![Instruction {
            opcode: 0x1c,
            on_true: 254,
            on_false: 253,
            operands: [47, 1, 0, 0, 1, 2, 0, 0],
        }];
        program.modules.insert(
            "target".into(),
            Module {
                source: "target".into(),
                object_definition_words: BTreeMap::from([(
                    2,
                    program.modules["test"].object_definition_words[&2].clone(),
                )]),
                routines: BTreeMap::from([(4097, child)]),
                ..Default::default()
            },
        );
        let mut original = world([0.; 16]);
        let mut third = original.objects[&1].clone();
        third.motives = Some(std::array::from_fn(|index| (100. - index as f32).to_bits()));
        original.objects.insert(3, third);
        let target = original.objects.get_mut(&2).unwrap();
        target.definition.as_mut().unwrap().source = "target".into();
        target.motives = Some(std::array::from_fn(|index| (index as f32).to_bits()));
        target.data.insert(62, 0);
        original.actions = Some(sims_vm::action::WorldState {
            actors: BTreeMap::from([(1, Default::default())]),
            ..Default::default()
        });
        original.objects.get_mut(&1).unwrap().person =
            Some(BTreeMap::from([(33, 0), (71, 0), (72, 0), (73, 0)]));
        let mut vm = Vm::new(original.clone(), 123);
        let thread = vm
            .spawn(
                &program,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: 2,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        vm.run_slice(&program, thread, 2).unwrap();
        let frames = &vm.threads[&thread].stack;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].stack_object, 2);
        assert_eq!(frames[1].stack_object, 3);
        assert_eq!(frames[1].caller, 1);
        assert_eq!(frames[1].callee, 2);
        assert_eq!(frames[1].code_owner, "target");
        let mut restored = Snapshot::capture(&program, &vm).restore(&program).unwrap();
        assert_eq!(restored, vm);
        vm.run_slice(&program, thread, 1).unwrap();
        restored.run_slice(&program, thread, 1).unwrap();
        assert_eq!(restored, vm);
        assert_eq!(
            Snapshot::capture(&program, &vm).restore(&program).unwrap(),
            vm
        );
        let expected = if selector == 0 {
            [6, 7, 8, 9, 13]
        } else {
            [15, 14, 13, 9, 8]
        };
        original.objects.get_mut(&1).unwrap().temps[..5].copy_from_slice(&expected);
        assert_eq!(vm.world, original);
        vm.run_slice(&program, thread, 1).unwrap();
        assert_eq!(vm.threads[&thread].status, Status::Returned(true));
        assert_eq!(vm.tick, 0);
        assert_eq!(vm.rng_state, 123);
    }
}

#[test]
fn stack_person_reads_still_write_only_executor_temporaries() {
    let mut original = world([0.; 16]);
    let target = original
        .objects
        .get_mut(&2)
        .unwrap()
        .motives
        .as_mut()
        .unwrap();
    for (index, value) in [
        (5, 80f32),
        (6, 70.),
        (7, 60.),
        (8, 50.),
        (9, 40.),
        (13, 30.),
        (14, 20.),
        (15, 10.),
    ] {
        target[index] = value.to_bits();
    }
    for mode in 0u16..=2 {
        let mut operands = [0x81, 0xfe, 0xab, 0x33, 1, 0, 0, 0];
        operands[6..].copy_from_slice(&mode.to_le_bytes());
        let program = program(operands, 2);
        let (vm, thread) = execute(&program, original.clone(), 2, false);
        assert_eq!(vm.threads[&thread].status, Status::Returned(true));
        let mut expected = original.clone();
        expected.objects.get_mut(&1).unwrap().temps[..5].copy_from_slice(&[15, 14, 13, 9, 8]);
        assert_eq!(vm.world, expected);
    }
}

#[test]
fn other_full_word_selectors_use_executor_and_ignore_first_four_bytes() {
    for selector in [0u16, 3, 0x100, 0x101, u16::MAX] {
        let mut operands = [0xaa, 0xbb, 0xcc, 0xdd, 0, 0, 0, 0];
        operands[4..6].copy_from_slice(&selector.to_le_bytes());
        let program = program(operands, 4);
        let (vm, thread) = execute(&program, world([0.; 16]), -1, false);
        assert_eq!(vm.threads[&thread].status, Status::Returned(true));
        assert_eq!(&vm.world.objects[&1].temps[..5], &[6, 7, 8, 9, 13]);
    }
}

fn assert_rejected(operands: [u8; 8], target_kind: i16, stack_object: i16, message: &str) {
    let program = program(operands, target_kind);
    let original = world([0.; 16]);
    let (vm, thread) = execute(&program, original.clone(), stack_object, false);
    let Status::Faulted(fault) = &vm.threads[&thread].status else {
        panic!("invalid ranking request was not rejected");
    };
    assert!(fault.message.contains(message), "{}", fault.message);
    assert_eq!(vm.world, original);
}

#[test]
fn invalid_person_and_mode_keep_native_validation_order_without_writes() {
    assert_rejected([0, 0, 0, 0, 2, 0, 0, 0], 2, 2, "native error 0x0a");
    assert_rejected([0, 0, 0, 0, 1, 0, 0, 0], 2, -1, "native error 0x0a");
    assert_rejected([0, 0, 0, 0, 1, 0, 3, 0], 4, 2, "native error 0x0a");
    assert_rejected([0, 0, 0, 0, 1, 0, 3, 0], 2, -1, "native error 0x0a");
    assert_rejected([0, 0, 0, 0, 0, 0, 3, 0], 2, 2, "native error 0x1e");
    assert_rejected([0, 0, 0, 0, 0, 0, 0, 1], 2, 2, "native error 0x1e");
}

#[test]
fn missing_person_storage_is_an_explicit_unchanged_world_error() {
    let program = program([0; 8], 2);
    let mut original = world([0.; 16]);
    original.objects.get_mut(&1).unwrap().motives = None;
    let (vm, thread) = execute(&program, original.clone(), 2, false);
    let Status::Faulted(fault) = &vm.threads[&thread].status else {
        panic!("missing motive storage was not rejected");
    };
    assert!(fault.message.contains("motive storage absent"));
    assert_eq!(vm.world, original);
}

#[test]
fn diagnostic_name_and_capability_identify_the_implemented_operation() {
    assert_eq!(sims_vm::primitive_name(0x21), "find five worst motives");
    let support = sims_vm::compatibility::primitive(0x21);
    assert_eq!(support.rust_support, "partial");
    assert!(support.limits.contains("five caller temporaries"));
}
