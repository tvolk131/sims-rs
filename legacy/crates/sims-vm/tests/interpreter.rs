use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::{Slot, Slots},
};
use sims_vm::{Module, Object, Program, Routine, Snapshot, Start, Status, Stop, Vm, World};
use std::collections::{BTreeMap, BTreeSet};

fn instruction(opcode: u16, yes: u8, no: u8, operands: [u8; 8]) -> Instruction {
    Instruction {
        opcode,
        on_true: yes,
        on_false: no,
        operands,
    }
}
fn expression(ls: u8, li: i16, op: u8, rs: u8, ri: i16, yes: u8, no: u8) -> Instruction {
    let mut bytes = [0; 8];
    bytes[..2].copy_from_slice(&li.to_le_bytes());
    bytes[2..4].copy_from_slice(&ri.to_le_bytes());
    bytes[5..].copy_from_slice(&[op, ls, rs]);
    instruction(2, yes, no, bytes)
}
fn routine(instructions: Vec<Instruction>) -> Routine {
    Routine {
        name: "synthetic".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 4,
            locals: 4,
            instructions,
        },
    }
}
fn program(instructions: Vec<Instruction>) -> Program {
    let mut p = Program {
        globals: "global".into(),
        modules: BTreeMap::new(),
        ..Program::default()
    };
    p.modules.insert(
        "local".into(),
        Module {
            source: "local".into(),
            semiglobal: Some("semi".into()),
            routines: BTreeMap::from([(4096, routine(instructions))]),
            ..Module::default()
        },
    );
    for name in ["global", "semi"] {
        p.modules.insert(
            name.into(),
            Module {
                source: name.into(),
                ..Module::default()
            },
        );
    }
    p
}
fn vm(p: &Program, args: Vec<i16>) -> Vm {
    let mut world = World::default();
    world.objects.insert(
        1,
        Object {
            attributes: vec![0; 4],
            slots: vec![None],
            ..Object::default()
        },
    );
    world.objects.insert(
        2,
        Object {
            attributes: vec![0; 4],
            slots: vec![None],
            ..Object::default()
        },
    );
    let mut vm = Vm::new(world, 77);
    vm.spawn(
        p,
        Start {
            owner: "local",
            bhav: 4096,
            caller: 1,
            callee: 2,
            stack_object: 2,
            args,
            check: false,
        },
    )
    .unwrap();
    vm
}
fn pass() -> Instruction {
    expression(7, 0, 2, 7, 0, 254, 255)
}

#[test]
fn script_motive_changes_append_in_order_resume_and_clear_without_reading_operands() {
    let delta = |motive, flags, rate, target| {
        let mut operands = [7, 7, motive, flags, 0, 0, 0, 0];
        operands[4..6].copy_from_slice(&i16::to_le_bytes(rate));
        operands[6..8].copy_from_slice(&i16::to_le_bytes(target));
        operands
    };
    let p = program(vec![
        instruction(29, 1, 253, delta(7, 0, 1800, 30)),
        instruction(29, 2, 253, delta(7, 2, -900, -10)),
        // Clear ignores both scope bytes and an otherwise illegal motive index.
        instruction(29, 254, 253, [255, 255, 255, 1, 255, 255, 255, 255]),
    ]);
    let mut v = vm(&p, vec![]);
    let o = v.world.objects.get_mut(&1).unwrap();
    o.person = Some(BTreeMap::new());
    o.motives = Some([0; 16]);
    assert_eq!(v.run_slice(&p, 1, 2).unwrap().stop, Stop::Budget);
    assert_eq!(v.world.objects[&1].motive_changes.len(), 2);
    let changes = &v.world.objects[&1].motive_changes;
    assert_eq!(
        (
            f32::from_bits(changes[0].delta),
            f32::from_bits(changes[1].delta)
        ),
        (1., -0.5)
    );
    let mut resumed =
        Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &v)).unwrap(), &p).unwrap();
    assert_eq!(
        v.run_slice(&p, 1, 1).unwrap(),
        resumed.run_slice(&p, 1, 1).unwrap()
    );
    assert!(v.world.objects[&1].motive_changes.is_empty());
}

#[test]
fn signed_comparison_and_16_bit_overflow_do_not_depend_on_build_mode() {
    let p = program(vec![
        expression(8, 0, 5, 7, 32767, 1, 253),
        expression(8, 0, 3, 7, 1, 2, 253),
        expression(8, 0, 1, 7, 0, 254, 255),
    ]);
    let mut vm = vm(&p, vec![]);
    assert_eq!(vm.run_slice(&p, 1, 20).unwrap().stop, Stop::Returned(true));
    assert_eq!(vm.world.objects[&1].temps[0], i16::MIN);
}
#[test]
fn arithmetic_flags_and_ts1_or_xor_are_not_tso_list_operations() {
    let cases = [
        (3, 32767, 1, -32768),
        (4, -32768, 1, 32767),
        (6, 200, 200, -25536),
        (7, -32768, -1, -32768),
        (12, -5, 3, 2),
        (12, 5, -3, 2),
        (12, -32768, 3, -2),
        (12, 5, -32768, 5),
        (13, 7, 3, 3),
        (18, 1, 4, 5),
        (19, 7, 3, 4),
        (9, 0, 16, -32768),
        (10, -1, 16, 32767),
        (20, 0, 81, 9),
    ];
    for (op, lhs, rhs, expected) in cases {
        let p = program(vec![expression(8, 0, op, 7, rhs, 254, 255)]);
        let mut vm = vm(&p, vec![]);
        vm.world.objects.get_mut(&1).unwrap().temps[0] = lhs;
        assert_eq!(
            vm.run_slice(&p, 1, 1).unwrap().stop,
            Stop::Returned(true),
            "op {op}"
        );
        assert_eq!(vm.world.objects[&1].temps[0], expected, "op {op}");
    }
}
#[test]
fn native_increment_captures_rhs_before_write_and_compares_narrowed_value() {
    let p = program(vec![expression(8, 0, 11, 8, 0, 254, 255)]);
    let mut state = vm(&p, vec![]);
    assert_eq!(
        state.run_slice(&p, 1, 1).unwrap().stop,
        Stop::Returned(false)
    );
    assert_eq!(state.world.objects[&1].temps[0], 1);
    let p = program(vec![expression(8, 0, 11, 7, 0, 254, 255)]);
    let mut state = vm(&p, vec![]);
    state.world.objects.get_mut(&1).unwrap().temps[0] = 32767;
    assert_eq!(
        state.run_slice(&p, 1, 1).unwrap().stop,
        Stop::Returned(true)
    );
    assert_eq!(state.world.objects[&1].temps[0], -32768);
    // Aliasing distinguishes captured RHS from rereading the changed destination.
    let p = program(vec![expression(8, 0, 11, 8, 0, 254, 255)]);
    let mut state = vm(&p, vec![]);
    state.world.objects.get_mut(&1).unwrap().temps[0] = 32767;
    assert_eq!(
        state.run_slice(&p, 1, 1).unwrap().stop,
        Stop::Returned(true)
    );
    let p = program(vec![expression(8, 0, 17, 8, 0, 254, 255)]);
    let mut state = vm(&p, vec![]);
    state.world.objects.get_mut(&1).unwrap().temps[0] = -32768;
    assert_eq!(
        state.run_slice(&p, 1, 1).unwrap().stop,
        Stop::Returned(true)
    );
    assert_eq!(state.world.objects[&1].temps[0], 32767);
}
#[test]
fn scopes_and_indirection() {
    let p = program(vec![
        expression(8, 0, 5, 7, 1, 1, 253),
        expression(11, 0, 5, 7, 23, 2, 253),
        expression(25, 1, 5, 11, 0, 3, 253),
        expression(1, 0, 5, 25, 1, 4, 253),
        pass(),
    ]);
    let mut vm = vm(&p, vec![]);
    assert_eq!(vm.run_slice(&p, 1, 20).unwrap().stop, Stop::Returned(true));
    assert_eq!(vm.world.objects[&2].attributes[0], 23);
    assert_eq!(vm.world.objects[&1].attributes[0], 0);
}

#[test]
fn native_expression_faults_preserve_integer_destination() {
    // Original 1.91 routine 0x00452c45, private oracle: zero divisors throw 0x26;
    // read-only destinations throw 0x13, rather than taking the false branch.
    for (scope, op, rhs, code) in [
        (8, 7, 0, "26"),
        (8, 12, 0, "26"),
        (7, 5, 4, "13"),
        (8, 9, 17, "0d"),
    ] {
        let p = program(vec![expression(scope, 0, op, 7, rhs, 254, 255)]);
        let mut state = vm(&p, vec![]);
        state.world.objects.get_mut(&1).unwrap().temps[0] = -5;
        assert_eq!(state.run_slice(&p, 1, 1).unwrap().stop, Stop::Faulted);
        let Status::Faulted(fault) = &state.threads[&1].status else {
            panic!()
        };
        assert!(fault.message.contains(&format!("native error 0x{code}")));
        assert_eq!(state.world.objects[&1].temps[0], -5);
    }
}

#[test]
fn native_expression_byte4_is_ignored_and_zero_flag_index_uses_x86_shift() {
    for flags in [0, 1, 3, 6, 128, 254, 255] {
        let mut instruction = expression(8, 0, 3, 7, 3, 254, 255);
        instruction.operands[4] = flags;
        let p = program(vec![instruction]);
        let mut state = vm(&p, vec![]);
        state.world.objects.get_mut(&1).unwrap().temps[0] = -5;
        assert_eq!(
            state.run_slice(&p, 1, 1).unwrap().stop,
            Stop::Returned(true)
        );
        assert_eq!(state.world.objects[&1].temps[0], -2);
    }
    for (op, lhs, returned) in [(8, -1, true), (8, 1, false), (9, -1, true), (10, -1, true)] {
        let p = program(vec![expression(8, 0, op, 7, 0, 254, 255)]);
        let mut state = vm(&p, vec![]);
        state.world.objects.get_mut(&1).unwrap().temps[0] = lhs;
        assert_eq!(
            state.run_slice(&p, 1, 1).unwrap().stop,
            Stop::Returned(returned)
        );
        assert_eq!(state.world.objects[&1].temps[0], lhs);
    }
}

#[test]
fn native_expression_checkpoint_preserves_overflow_branch_and_rejects_old_semantics() {
    use sha2::{Digest, Sha256};
    let p = program(vec![
        expression(8, 0, 11, 8, 0, 1, 255),
        expression(8, 0, 12, 7, 3, 254, 255),
    ]);
    let mut state = vm(&p, vec![]);
    state.world.objects.get_mut(&1).unwrap().temps[0] = 32767;
    assert_eq!(state.run_slice(&p, 1, 1).unwrap().stop, Stop::Budget);
    let snapshot = Snapshot::capture(&p, &state);
    let mut resumed = snapshot.clone().restore(&p).unwrap();
    assert_eq!(
        resumed.run_slice(&p, 1, 1).unwrap(),
        state.run_slice(&p, 1, 1).unwrap()
    );
    assert_eq!(resumed, state);
    assert_eq!(state.world.objects[&1].temps[0], -2);
    let mut previous = snapshot;
    let mut hash = Sha256::new();
    hash.update(b"sims-vm/program/v4\0");
    hash.update(serde_json::to_vec(&p).unwrap());
    previous.program_hash = format!("{:x}", hash.finalize());
    assert!(previous.restore(&p).unwrap_err().contains("fingerprint"));
}
#[test]
fn global_and_semiglobal_calls_retain_private_owner_and_frame_isolation() {
    let mut p = program(vec![
        instruction(256, 1, 255, [255; 8]),
        expression(10, 0, 2, 7, 2, 254, 255),
    ]);
    p.modules
        .get_mut("global")
        .unwrap()
        .routines
        .insert(256, routine(vec![instruction(8192, 254, 255, [255; 8])]));
    p.modules
        .get_mut("semi")
        .unwrap()
        .routines
        .insert(8192, routine(vec![instruction(4097, 254, 255, [255; 8])]));
    p.modules.get_mut("local").unwrap().routines.insert(
        4097,
        routine(vec![
            expression(10, 0, 5, 7, 1, 1, 253),
            expression(8, 0, 5, 9, 0, 254, 253),
        ]),
    );
    let mut state = vm(&p, vec![]);
    state.world.objects.get_mut(&1).unwrap().temps[0] = 17;
    let result = state.run_slice(&p, 1, 20).unwrap();
    assert_eq!(result.stop, Stop::Returned(true));
    assert_eq!(
        result
            .trace
            .iter()
            .map(|t| t.location.source.as_str())
            .collect::<Vec<_>>(),
        ["local", "global", "semi", "local", "local", "local"]
    );
    assert_eq!(state.world.objects[&1].temps[0], 17);
}
#[test]
fn minus_one_argument_convention_and_return_false_unwind() {
    for (bytes, expected) in [([255; 8], 42), ([255, 255, 0, 0, 0, 0, 0, 0], -1)] {
        let mut p = program(vec![instruction(4097, 255, 254, bytes)]);
        p.modules
            .get_mut("local")
            .unwrap()
            .routines
            .insert(4097, routine(vec![expression(8, 1, 5, 9, 0, 255, 253)]));
        let mut state = vm(&p, vec![]);
        state.world.objects.get_mut(&1).unwrap().temps[0] = 42;
        assert_eq!(
            state.run_slice(&p, 1, 20).unwrap().stop,
            Stop::Returned(true)
        );
        assert_eq!(state.world.objects[&1].temps[1], expected);
    }
}
#[test]
fn tuning_addresses_resolve_the_three_resource_scopes() {
    let mut p = program(vec![
        expression(8, 0, 5, 26, 0, 1, 253),
        expression(8, 1, 5, 26, 0x2000, 2, 253),
        expression(8, 2, 5, 26, 0x4000, 254, 253),
    ]);
    for (name, table, value) in [("local", 4096, 11), ("semi", 8192, 22), ("global", 256, 33)] {
        p.modules
            .get_mut(name)
            .unwrap()
            .constants
            .insert(table, vec![value]);
    }
    let mut state = vm(&p, vec![]);
    assert_eq!(
        state.run_slice(&p, 1, 20).unwrap().stop,
        Stop::Returned(true)
    );
    assert_eq!(&state.world.objects[&1].temps[..3], &[11, 22, 33]);
}
#[test]
fn unknown_operations_invalid_branches_and_bad_indices_fault_with_source() {
    for ins in [
        instruction(99, 254, 255, [0; 8]),
        expression(7, 0, 2, 7, 0, 253, 255),
        expression(7, 0, 2, 7, 0, 42, 255),
        expression(8, -1, 5, 7, 0, 254, 255),
        expression(8, 0, 5, 2, 0, 254, 255),
        instruction(4109, 254, 255, [0; 8]),
    ] {
        let p = program(vec![ins]);
        let mut state = vm(&p, vec![]);
        assert_eq!(state.run_slice(&p, 1, 10).unwrap().stop, Stop::Faulted);
        let Status::Faulted(f) = &state.threads[&1].status else {
            panic!()
        };
        assert_eq!(f.stack.last().unwrap().source, "local");
        assert_eq!(f.stack.last().unwrap().instruction, 0);
    }
}
#[test]
fn instruction_budget_and_stack_limit_are_distinct() {
    let p = program(vec![expression(7, 1, 2, 7, 1, 0, 0)]);
    let mut state = vm(&p, vec![]);
    assert_eq!(state.run_slice(&p, 1, 50).unwrap().trace.len(), 50);
    assert_eq!(state.threads[&1].status, Status::Ready);
    assert_eq!(state.tick, 0);
    assert!(state.advance_tick().is_err());
    let p = program(vec![instruction(4096, 254, 255, [0; 8])]);
    let mut state = vm(&p, vec![]);
    assert_eq!(state.run_slice(&p, 1, 1000).unwrap().stop, Stop::Faulted);
    assert_eq!(state.threads[&1].stack.len(), sims_vm::MAX_STACK);
}
#[test]
fn sleep_snapshot_and_rng_resume_identically() {
    let p = program(vec![
        instruction(0, 1, 253, [0; 8]),
        instruction(8, 254, 253, [0, 0, 8, 0, 100, 0, 7, 0]),
    ]);
    let mut uninterrupted = vm(&p, vec![2]);
    assert_eq!(
        uninterrupted.run_slice(&p, 1, 20).unwrap().stop,
        Stop::Sleeping
    );
    let bytes = serde_json::to_vec(&Snapshot::capture(&p, &uninterrupted)).unwrap();
    let mut restored = Snapshot::from_json(&bytes, &p).unwrap();
    for _ in 0..2 {
        uninterrupted.advance_tick().unwrap();
        restored.advance_tick().unwrap();
        assert_eq!(
            uninterrupted.run_slice(&p, 1, 20).unwrap(),
            restored.run_slice(&p, 1, 20).unwrap()
        );
    }
    assert_eq!(uninterrupted, restored);
    assert_eq!(restored.threads[&1].status, Status::Returned(true));
    assert!((0..100).contains(&restored.world.objects[&1].temps[0]));
}

#[test]
fn dialog_modes_restore_before_and_after_explicit_response() {
    for mode in [0, 1, 2, 4] {
        // This fixture tests the modal protocol without a portrait host. Native
        // icon mode 1 supplies no resource image; mode 0 now correctly suspends
        // in the separately tested resource/portrait construction service.
        let mut p = program(vec![instruction(
            0x24,
            254,
            253,
            [0, 0, 2, 0, 0, mode, 1, 2],
        )]);
        p.modules
            .get_mut("local")
            .unwrap()
            .strings
            .insert(301, vec!["Caption".into(), "Body".into()]);
        p.modules.insert(
            "ui".into(),
            Module {
                source: "UIText.iff".into(),
                strings: BTreeMap::from([(
                    142,
                    vec!["OK".into(), "Cancel".into(), "Yes".into(), "No".into()],
                )]),
                ..Default::default()
            },
        );
        let mut s = vm(&p, vec![]);
        assert_eq!(s.run_slice(&p, 1, 100).unwrap().stop, Stop::Sleeping);
        let id = s.pump_dialogs().unwrap().unwrap();
        let pending = Snapshot::capture(&p, &s);
        let mut resumed = pending.clone().restore(&p).unwrap();
        assert_eq!(resumed, s);
        assert!(resumed.advance_tick().is_err());
        assert_eq!(resumed, s);
        let record = &s.world.dialogs.records[0];
        assert_eq!(record.key_result(13), Some(2));
        assert_eq!(record.key_result(65), None);
        assert_eq!(record.key_result(32), matches!(mode, 0 | 4).then_some(2));
        for vm in [&mut s, &mut resumed] {
            vm.answer_dialog(id, 2).unwrap();
            assert_eq!(vm.world.dialogs.records[0].key_result(13), None);
            *vm = Snapshot::capture(&p, vm).restore(&p).unwrap();
            vm.advance_tick().unwrap();
            assert_eq!(vm.run_slice(&p, 1, 100).unwrap().stop, Stop::Returned(true));
            assert_eq!(vm.pump_dialogs().unwrap(), None);
            assert!(vm.world.dialogs.records.is_empty());
        }
        assert_eq!(s, resumed);
        // Unsupported text input is still rejected even if an untrusted save
        // supplies an otherwise well-formed sleeping frame and dialog record.
        let mut invalid_program = p.clone();
        invalid_program
            .modules
            .get_mut("local")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body
            .instructions[0]
            .operands[5] = 3;
        let mut invalid = pending;
        invalid.program_hash = invalid_program.fingerprint();
        invalid.state.world.dialogs.records[0].operands[5] = 3;
        assert!(invalid.restore(&invalid_program).is_err());
    }
}

#[test]
fn sleep_does_not_advance_without_ticks_and_interrupt_is_explicit() {
    let p = program(vec![instruction(0, 254, 253, [0; 8])]);
    let mut state = vm(&p, vec![20]);
    state.run_slice(&p, 1, 20).unwrap();
    assert!(state.run_slice(&p, 1, 20).unwrap().trace.is_empty());
    state.interrupt(1).unwrap();
    assert_eq!(
        state.run_slice(&p, 1, 20).unwrap().stop,
        Stop::Returned(true)
    );
    assert!(!state.threads[&1].interrupted);
}
#[test]
fn stable_thread_order_and_budget_split_do_not_change_results() {
    let p = program(vec![
        expression(6, 34, 3, 7, 1, 1, 253),
        expression(8, 0, 5, 6, 34, 254, 253),
    ]);
    let mut state = vm(&p, vec![]);
    state.world.globals.insert(34, 0);
    state
        .spawn(
            &p,
            Start {
                owner: "local",
                bhav: 4096,
                caller: 2,
                callee: 2,
                stack_object: 1,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    let mut budgeted = state.clone();
    let result = state.run_round(&p, 10);
    assert_eq!(result.iter().map(|r| r.0).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(state.world.objects[&1].temps[0], 1);
    assert_eq!(state.world.objects[&2].temps[0], 2);
    assert_eq!(budgeted.run_round(&p, 1).len(), 1);
    assert!(budgeted.advance_tick().is_err());
    budgeted.run_round(&p, 1);
    budgeted.run_round(&p, 1);
    assert_eq!(state, budgeted);
    // Splitting one thread's execution without advancing the scheduler is equivalent.
    let p = program(vec![expression(8, 0, 3, 7, 1, 1, 253), pass()]);
    let mut a = vm(&p, vec![]);
    let mut b = a.clone();
    a.run_slice(&p, 1, 20).unwrap();
    b.run_slice(&p, 1, 1).unwrap();
    b.run_slice(&p, 1, 1).unwrap();
    assert_eq!(a, b);
}
#[test]
fn route_wait_roundtrips_and_responses_are_consumed_once() {
    let mut p = program(vec![instruction(0x2d, 254, 255, [0, 0, 2, 0, 0, 0, 0, 0])]);
    p.modules.get_mut("global").unwrap().slots.insert(
        100,
        Slots {
            version: 10,
            entries: vec![Slot {
                kind: 3,
                offset: [0.0; 3],
                standing: 1,
                sitting: 0,
                ground: 0,
                flags: 1,
                snap_target: -1,
                proximity: [0; 3],
                max_size: 100,
                unknown: 0,
                gradient: 0.0,
                height: 0,
                facing: -2,
                resolution: 16,
            }],
        },
    );
    let mut state = vm(&p, vec![]);
    assert_eq!(
        state.run_slice(&p, 1, 20).unwrap().stop,
        Stop::AwaitingRoute
    );
    let bytes = serde_json::to_vec(&Snapshot::capture(&p, &state)).unwrap();
    let mut restored = Snapshot::from_json(&bytes, &p).unwrap();
    let mut corrupted = Snapshot::capture(&p, &state);
    if let Status::AwaitingRoute(r) = &mut corrupted.state.threads.get_mut(&1).unwrap().status {
        r.no_failure_trees = true;
    }
    assert!(corrupted.restore(&p).is_err());
    assert!(restored.run_slice(&p, 1, 20).unwrap().trace.is_empty());
    assert!(restored.finish_route(&p, 1, 99, false).is_err());
    state.finish_route(&p, 1, 1, false).unwrap();
    restored.finish_route(&p, 1, 1, false).unwrap();
    assert_eq!(state, restored);
    assert_eq!(restored.threads[&1].status, Status::Returned(false));
    assert!(restored.finish_route(&p, 1, 1, true).is_err());

    // Native45ad63 publishes3 while walking,9/10/11 while stopping, and15
    // while stopping after a house-refresh invalidation.
    // These belong to a retained route request, never an ordinary ready frame.
    for continuation in [3, 9, 10, 11, 15] {
        let mut state = vm(&p, vec![]);
        state.run_slice(&p, 1, 20).unwrap();
        state.threads.get_mut(&1).unwrap().stack[0].continuation = continuation;
        let bytes = serde_json::to_vec(&Snapshot::capture(&p, &state)).unwrap();
        let mut restored = Snapshot::from_json(&bytes, &p).unwrap();
        assert_eq!(restored, state);
        let mut bad = Snapshot::capture(&p, &state);
        bad.state.threads.get_mut(&1).unwrap().status = Status::Ready;
        assert!(bad.restore(&p).is_err());
        let mut bad = Snapshot::capture(&p, &state);
        bad.state.threads.get_mut(&1).unwrap().stack[0].continuation = 12;
        assert!(bad.restore(&p).is_err());
        state.finish_route(&p, 1, 1, false).unwrap();
        restored.finish_route(&p, 1, 1, false).unwrap();
        assert_eq!(restored, state);
        assert_eq!(restored.threads[&1].stack.len(), 0);
    }
}
#[test]
fn snapshots_reject_changed_code_invalid_frames_and_schema() {
    let mut p = program(vec![pass()]);
    let state = vm(&p, vec![]);
    let snapshot = Snapshot::capture(&p, &state);
    let mut bad = snapshot.clone();
    bad.version = 99;
    assert!(bad.restore(&p).is_err());
    let mut bad = snapshot.clone();
    bad.state.threads.get_mut(&1).unwrap().stack[0]
        .location
        .instruction = 100;
    assert!(bad.restore(&p).is_err());
    p.modules
        .get_mut("local")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .instructions[0]
        .on_true = 255;
    assert!(snapshot.restore(&p).unwrap_err().contains("fingerprint"));
}

#[test]
fn animation_events_branch_false_store_payload_and_reject_stale_responses() {
    use sims_vm::AnimationOutcome;
    let p = program(vec![instruction(0x2c, 254, 0, [1, 0, 2, 0, 0, 32, 0, 0])]);
    let mut state = vm(&p, vec![]);
    assert_eq!(
        state.run_slice(&p, 1, 10).unwrap().stop,
        Stop::AwaitingAnimation
    );
    let Status::AwaitingAnimation(request) = state.threads[&1].status.clone() else {
        panic!()
    };
    assert!(
        state
            .finish_animation(&p, 1, request.id + 1, AnimationOutcome::Completed)
            .is_err()
    );
    state
        .finish_animation(&p, 1, request.id, AnimationOutcome::Event(42))
        .unwrap();
    assert_eq!(state.threads[&1].stack[0].locals[2], 42);
    assert_eq!(
        state.run_slice(&p, 1, 10).unwrap().stop,
        Stop::AwaitingAnimation
    );
    assert!(
        state
            .finish_animation(&p, 1, request.id, AnimationOutcome::Completed)
            .is_err()
    );
    let Status::AwaitingAnimation(next) = state.threads[&1].status.clone() else {
        panic!()
    };
    state
        .finish_animation(&p, 1, next.id, AnimationOutcome::Completed)
        .unwrap();
    assert_eq!(state.threads[&1].status, Status::Returned(true));
}

#[test]
fn notify_wakes_registered_object_thread_and_sound_has_a_named_presentation_command() {
    let mut p = program(vec![
        instruction(0x31, 1, 253, [0; 8]),
        instruction(0x17, 254, 253, [4, 0, 0, 0, 2, 0, 0, 0]),
    ]);
    p.modules
        .get_mut("local")
        .unwrap()
        .sounds
        .insert(4, "synthetic_chair".into());
    p.modules
        .get_mut("local")
        .unwrap()
        .routines
        .insert(4097, routine(vec![instruction(0, 254, 253, [0; 8])]));
    let mut state = vm(&p, vec![]);
    let idle = state
        .spawn(
            &p,
            Start {
                owner: "local",
                bhav: 4097,
                caller: 2,
                callee: 2,
                stack_object: 2,
                args: vec![100],
                check: false,
            },
        )
        .unwrap();
    state.register_object_thread(2, idle).unwrap();
    assert!(state.register_object_thread(2, idle).is_err());
    state.run_slice(&p, idle, 10).unwrap();
    assert!(matches!(
        state.threads[&idle].status,
        Status::Sleeping { .. }
    ));
    assert_eq!(
        state.run_slice(&p, 1, 10).unwrap().stop,
        Stop::Returned(true)
    );
    assert_eq!(state.threads[&idle].status, Status::Ready);
    assert!(!state.threads[&idle].interrupted);
    assert_eq!(state.threads[&idle].stack.last().unwrap().args[0], 0);
    assert_eq!(state.sounds.len(), 1);
    assert_eq!(state.sounds[0].name, "synthetic_chair");
    assert_eq!(state.sounds[0].object, 2);
    let mut resumed = Snapshot::capture(&p, &state).restore(&p).unwrap();
    assert_eq!(
        resumed.run_slice(&p, idle, 10).unwrap(),
        state.run_slice(&p, idle, 10).unwrap()
    );
    assert_eq!(state, resumed);
}

#[test]
fn native_scope_bounds_globals_and_shared_object_temporaries() {
    for (scope, index, code) in [
        (8, 8, "02"),
        (8, -1, "02"),
        (10, 1, "0f"),
        (6, 64, "3f"),
        (3, 72, "07"),
        (40, 0, "09"),
        (41, 0, "09"),
    ] {
        let p = program(vec![expression(8, 0, 5, scope, index, 254, 253)]);
        let mut s = vm(&p, vec![]);
        s.run_slice(&p, 1, 1).unwrap();
        let Status::Faulted(f) = &s.threads[&1].status else {
            panic!("expected fault")
        };
        assert!(
            f.message.contains(&format!("native error 0x{code}")),
            "{}",
            f.message
        );
    }
    for i in [0, 34, 36, 63] {
        let p = program(vec![expression(6, i, 5, 7, 99, 254, 253)]);
        let mut s = vm(&p, vec![]);
        s.world.globals.insert(i, 7);
        s.run_slice(&p, 1, 1).unwrap();
        assert_eq!(
            s.world.globals[&i],
            if matches!(i, 34 | 36) { 99 } else { 7 }
        );
        assert_eq!(
            matches!(s.threads[&1].status, Status::Returned(true)),
            matches!(i, 34 | 36)
        );
    }
    let p = program(vec![expression(13, 0, 5, 7, 42, 254, 253)]);
    let mut s = vm(&p, vec![]);
    s.run_slice(&p, 1, 1).unwrap();
    assert_eq!(s.world.objects[&2].temps[0], 42);
    assert_eq!(s.world.objects[&1].temps[0], 0);
}

#[test]
fn native_mixed_arguments_and_reused_locals_resume_in_nested_calls() {
    let mut p = program(vec![
        instruction(4097, 1, 253, [255, 255, 9, 0, 0, 0, 0, 0]),
        instruction(4097, 254, 253, [255, 255, 9, 0, 0, 0, 0, 0]),
    ]);
    p.modules.get_mut("local").unwrap().routines.insert(
        4097,
        routine(vec![
            expression(25, 0, 3, 7, 1, 1, 253),
            expression(0, 0, 5, 25, 0, 2, 253),
            expression(0, 1, 5, 9, 0, 254, 253),
        ]),
    );
    let mut state = vm(&p, vec![]);
    state.world.objects.get_mut(&1).unwrap().temps[0] = 123;
    assert_eq!(state.run_slice(&p, 1, 2).unwrap().stop, Stop::Budget);
    let bytes = serde_json::to_vec(&Snapshot::capture(&p, &state)).unwrap();
    let mut resumed = Snapshot::from_json(&bytes, &p).unwrap();
    state.run_slice(&p, 1, 20).unwrap();
    while resumed.run_slice(&p, 1, 1).unwrap().stop == Stop::Budget {}
    assert_eq!(state, resumed);
    assert_eq!(state.world.objects[&1].attributes[..2], [2, -1]);
}

#[test]
fn native_header_dimensions_and_indexed_entry() {
    for (format, args, locals, expected) in [
        (0x8000, 9, 256, (4, 0)),
        (0x8001, 9, 257, (4, 1)),
        (0x8002, 1, 257, (1, 1)),
        (0x8002, 0, 0, (0, 0)),
    ] {
        let mut p = program(vec![pass(); 3]);
        let b = &mut p
            .modules
            .get_mut("local")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body;
        b.format = format;
        b.args = args;
        b.locals = locals;
        b.kind = 4;
        let s = vm(&p, vec![10]);
        let f = &s.threads[&1].stack[0];
        assert_eq!((f.args.len(), f.locals.len()), expected);
        assert_eq!(f.location.instruction, 2);
    }
}

#[test]
fn fractional_motives_round_clamp_and_snapshot_exactly() {
    for (value, expected) in [(0.5f32, 1), (-0.5, -1), (2.5, 3), (-2.5, -3)] {
        let p = program(vec![expression(8, 0, 5, 14, 0, 254, 253)]);
        let mut s = vm(&p, vec![]);
        let o = s.world.objects.get_mut(&1).unwrap();
        o.person = Some(BTreeMap::new());
        o.motives = Some([value.to_bits(); 16]);
        s.run_slice(&p, 1, 1).unwrap();
        assert_eq!(s.world.objects[&1].temps[0], expected);
    }

    let p = program(vec![
        expression(8, 0, 5, 14, 0, 1, 253),
        expression(14, 0, 3, 7, 1, 2, 253),
        expression(14, 1, 5, 7, 200, 254, 253),
    ]);
    let mut s = vm(&p, vec![]);
    let o = s.world.objects.get_mut(&1).unwrap();
    o.person = Some(BTreeMap::new());
    o.motives = Some([1.5f32.to_bits(); 16]);
    s.run_slice(&p, 1, 1).unwrap();
    assert_eq!(s.world.objects[&1].temps[0], 2);
    let mut resumed = Snapshot::capture(&p, &s).restore(&p).unwrap();
    s.run_slice(&p, 1, 9).unwrap();
    resumed.run_slice(&p, 1, 9).unwrap();
    assert_eq!(s, resumed);
    assert_eq!(f32::from_bits(s.world.objects[&1].motives.unwrap()[0]), 2.5);
    assert_eq!(
        f32::from_bits(s.world.objects[&1].motives.unwrap()[1]),
        100.0
    );
}

#[test]
fn lifecycle_init_main_interrupt_and_resume_preserve_bindings() {
    use sims_formats::execution::ObjectFunction;
    use sims_vm::{Definition, lifecycle::Phase};
    let mut p = program(vec![expression(0, 0, 5, 7, 42, 255, 253)]);
    let m = p.modules.get_mut("local").unwrap();
    m.routines.insert(
        4097,
        routine(vec![instruction(256, 0, 253, [10, 0, 0, 0, 0, 0, 0, 0])]),
    );
    m.object_functions.insert(
        128,
        vec![
            ObjectFunction {
                condition: 0,
                action: 4096,
            },
            ObjectFunction {
                condition: 0,
                action: 4097,
            },
        ],
    );
    p.modules
        .get_mut("global")
        .unwrap()
        .routines
        .insert(256, routine(vec![instruction(0, 254, 253, [0; 8])]));
    let mut world = World::default();
    world.objects.insert(
        1,
        Object {
            attributes: vec![0],
            definition: Some(Definition {
                source: "local".into(),
                object: 128,
                guid: 1,
                slots: 0,
            }),
            ..Object::default()
        },
    );
    let mut s = Vm::new(world, 1);
    let id = s.start_object_lifecycle(&p, 1).unwrap();
    assert!(s.start_object_lifecycle(&p, 1).is_err());
    assert_eq!(s.lifecycles[&1].phase, Phase::Init);
    s = Snapshot::capture(&p, &s).restore(&p).unwrap();
    s.run_slice(&p, id, 10).unwrap();
    assert_eq!(s.lifecycles[&1].phase, Phase::Main);
    assert_eq!(s.world.objects[&1].attributes[0], 42);
    assert_eq!(s.run_slice(&p, id, 10).unwrap().stop, Stop::Sleeping);
    let mut resumed = Snapshot::capture(&p, &s).restore(&p).unwrap();
    s.interrupt(id).unwrap();
    resumed.interrupt(id).unwrap();
    assert_eq!(s.run_slice(&p, id, 20).unwrap().stop, Stop::Sleeping);
    resumed.run_slice(&p, id, 20).unwrap();
    assert_eq!(s, resumed);
    let mut bad = Snapshot::capture(&p, &s);
    bad.state.lifecycles.get_mut(&1).unwrap().main = 4096;
    assert!(bad.restore(&p).is_err());
}

#[test]
fn refresh_invalidates_container_tree_and_room_and_persists_pending_work() {
    use sims_vm::refresh::{DrawState, RefreshState, RoomState};
    let p = program(vec![
        instruction(7, 1, 253, [1, 0, 0, 0, 0, 0, 0, 0]),
        instruction(7, 254, 253, [0, 0, 2, 0, 0, 0, 0, 0]),
    ]);
    let mut s = vm(&p, vec![]);
    s.world.objects.get_mut(&1).unwrap().slots[0] = Some(2);
    s.world.objects.get_mut(&2).unwrap().container = Some((1, 0));
    s.world.objects.get_mut(&1).unwrap().data.insert(29, 0x401);
    s.world.refresh = Some(RefreshState {
        visible_level: 1,
        render_clock: 77,
        draw: (1..=2)
            .map(|id| {
                (
                    id,
                    DrawState {
                        level: 1,
                        ..Default::default()
                    },
                )
            })
            .collect(),
        rooms: BTreeMap::from([(
            0x401,
            RoomState {
                category: 1,
                dirty: false,
            },
        )]),
        ..Default::default()
    });
    s.run_slice(&p, 1, 1).unwrap();
    let mut resumed =
        Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(), &p).unwrap();
    assert_eq!(
        s.run_slice(&p, 1, 1).unwrap(),
        resumed.run_slice(&p, 1, 1).unwrap()
    );
    assert_eq!(s, resumed);
    let r = s.world.refresh.as_ref().unwrap();
    assert_eq!(
        r.invalidated.iter().copied().collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(
        r.draw
            .values()
            .all(|d| d.dynamic && d.expiry == i32::MAX && d.refreshed_at == 77)
    );
    assert!(r.rooms[&0x401].dirty);
    assert_eq!(r.dirty_categories, 2);
    let mut invalid = Snapshot::capture(&p, &s);
    invalid
        .state
        .world
        .refresh
        .as_mut()
        .unwrap()
        .rooms
        .get_mut(&0x401)
        .unwrap()
        .category = 4;
    assert!(invalid.restore(&p).is_err());
    s.refresh_object(1, 2, 0, 1).unwrap();
    assert_eq!(
        s.world.refresh.as_ref().unwrap().redraw_rooms,
        BTreeSet::from([0x401])
    );
    let resumed = Snapshot::capture(&p, &s).restore(&p).unwrap();
    assert_eq!(resumed, s);
}
#[test]
fn notify_does_not_interrupt_nonidle_work_or_select_sleep_operand_parameter() {
    let mut p = program(vec![instruction(0x31, 254, 253, [0; 8])]);
    p.modules.get_mut("local").unwrap().routines.insert(
        4097,
        routine(vec![instruction(0, 254, 253, [1, 0, 0, 0, 0, 0, 0, 0])]),
    );
    let mut s = vm(&p, vec![]);
    let target = s
        .spawn(
            &p,
            Start {
                owner: "local",
                bhav: 4097,
                caller: 2,
                callee: 2,
                stack_object: 2,
                args: vec![50, 60],
                check: false,
            },
        )
        .unwrap();
    s.register_object_thread(2, target).unwrap();
    s.run_slice(&p, 1, 10).unwrap();
    assert_eq!(s.threads[&target].stack[0].args[..2], [0, 60]);
    assert!(!s.threads[&target].interrupted);
    // A target currently executing Expression must keep both args and its state.
    p.modules
        .get_mut("local")
        .unwrap()
        .routines
        .insert(4097, routine(vec![pass()]));
    let notifier = s
        .spawn(
            &p,
            Start {
                owner: "local",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 2,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    s.threads.get_mut(&target).unwrap().stack[0].args[0] = 50;
    s.run_slice(&p, notifier, 10).unwrap();
    assert_eq!(s.threads[&target].stack[0].args[..2], [50, 60]);
    assert!(!s.threads[&target].interrupted);
}
#[test]
fn passive_scheduler_visits_once_per_tick_and_defers_late_notify() {
    use sims_vm::idle::IdleState;
    let mut p = program(vec![instruction(0x31, 254, 253, [0; 8])]);
    p.modules
        .get_mut("local")
        .unwrap()
        .routines
        .insert(4097, routine(vec![instruction(0, 254, 253, [0; 8])]));
    let mut s = vm(&p, vec![]);
    let o = s.world.objects.get_mut(&2).unwrap();
    o.idle = Some(IdleState::default());
    o.data.insert(25, 0);
    let target = s
        .spawn(
            &p,
            Start {
                owner: "local",
                bhav: 4097,
                caller: 2,
                callee: 2,
                stack_object: 2,
                args: vec![60],
                check: false,
            },
        )
        .unwrap();
    s.register_object_thread(2, target).unwrap();
    s.run_slice(&p, target, 10).unwrap();
    p.modules
        .get_mut("local")
        .unwrap()
        .routines
        .insert(4098, routine(vec![pass()]));
    let check = s
        .spawn(
            &p,
            Start {
                owner: "local",
                bhav: 4098,
                caller: 2,
                callee: 2,
                stack_object: 2,
                args: vec![],
                check: true,
            },
        )
        .unwrap();
    assert_eq!(
        s.run_slice(&p, check, 10).unwrap().stop,
        Stop::Returned(true)
    );
    assert_eq!(s.world.objects[&2].idle.as_ref().unwrap().countdown, 59);
    s.run_slice(&p, 1, 10).unwrap(); // target has already had its native scheduler turn
    assert_eq!(s.threads[&target].status, Status::Sleeping { wake_tick: 1 });
    assert!(s.run_slice(&p, target, 10).unwrap().trace.is_empty());
    let mut resumed = Snapshot::capture(&p, &s).restore(&p).unwrap();
    s.advance_tick().unwrap();
    resumed.advance_tick().unwrap();
    assert_eq!(
        s.run_slice(&p, target, 10).unwrap(),
        resumed.run_slice(&p, target, 10).unwrap()
    );
    assert_eq!(s.threads[&target].status, Status::Returned(true));
    assert_eq!(s, resumed);
}

#[test]
fn original_rng_stream_and_zero_range_fault_preserve_exact_state() {
    let mut state = 1;
    // Original 0x46f457 outputs, including its unmasked upper output bits.
    for (output, next_state) in [
        (16838, 1103527590),
        (1507104382, 2524885223),
        (3204589441, 662824084),
        (2509948011, 3295386429),
    ] {
        assert_eq!(sims_vm::rng::next(&mut state), output);
        assert_eq!(state, next_state);
    }
    let p = program(vec![instruction(8, 254, 253, [0, 0, 8, 0, 0, 0, 7, 0])]);
    let mut s = vm(&p, vec![]);
    let before = s.rng_state;
    s.run_slice(&p, 1, 1).unwrap();
    assert!(
        matches!(&s.threads[&1].status,Status::Faulted(f) if f.message.contains("native error 0x04"))
    );
    assert_eq!(s.rng_state, before);
    assert_eq!(Snapshot::capture(&p, &s).restore(&p).unwrap(), s);
}

#[test]
fn reset_reuses_main_storage_and_rejects_invalid_callback_snapshots() {
    use sims_formats::execution::ObjectFunction;
    use sims_vm::{Definition, construction::ObjectDate, lifecycle::Phase};
    let mut p = program(vec![expression(0, 0, 5, 7, 99, 254, 253)]);
    let m = p.modules.get_mut("local").unwrap();
    m.routines
        .insert(4097, routine(vec![instruction(0, 0, 253, [0; 8])]));
    m.object_definition_words.insert(1, vec![0; 108]);
    m.object_functions.insert(
        1,
        vec![
            ObjectFunction {
                action: 4096,
                condition: 0,
            },
            ObjectFunction {
                action: 4097,
                condition: 0,
            },
        ],
    );
    let o = Object::construct(
        Definition {
            source: "local".into(),
            object: 1,
            guid: 0,
            slots: 0,
        },
        &p,
        0,
    )
    .unwrap();
    let mut w = World::default();
    w.objects.insert(1, o);
    let mut s = Vm::new(w, 1);
    let id = s
        .reset_object_lifecycle(&p, 1, ObjectDate::default())
        .unwrap();
    assert_eq!(s.threads[&id].stack[0].location.bhav, 4097);
    assert_eq!(s.threads[&id].stack[1].location.bhav, 4096);
    assert_eq!(s.threads[&id].stack[1].storage_offset, 24); // Main + 8-word sentinel.
    let mut bad = Snapshot::capture(&p, &s);
    bad.state
        .threads
        .get_mut(&id)
        .unwrap()
        .callback
        .as_mut()
        .unwrap()
        .base_depth = 0;
    assert!(bad.restore(&p).is_err());
    let mut bad = Snapshot::capture(&p, &s);
    bad.state.threads.get_mut(&id).unwrap().frame_memory[16] = Some(0);
    assert!(bad.restore(&p).is_err());
    s.run_slice(&p, id, 100).unwrap();
    assert_eq!(s.lifecycles[&1].phase, Phase::Main);
    assert_eq!(s.world.objects[&1].attributes[0], 99);
    s.threads.get_mut(&id).unwrap().stack[0].locals = vec![101, 102, 103, 104];
    s.world.objects.get_mut(&1).unwrap().temps.fill(17);
    let mut replay = Snapshot::capture(&p, &s).restore(&p).unwrap();
    for v in [&mut s, &mut replay] {
        assert_eq!(
            v.reset_object_lifecycle(&p, 1, ObjectDate::default())
                .unwrap(),
            id
        );
        assert_eq!(v.threads[&id].stack[0].locals, [101, 102, 103, 104]);
        assert_eq!(v.world.objects[&1].attributes[0], 0);
        assert_eq!(v.threads[&id].stack[1].args, [0; 4]);
        v.run_slice(&p, id, 100).unwrap();
    }
    assert_eq!(s, replay);
}

#[test]
fn person_constructor_drives_two_inits_and_one_tick_zero_update_across_snapshots() {
    use sims_formats::execution::ObjectFunction;
    use sims_vm::{
        Definition,
        construction::ObjectDate,
        person_factory::{
            ConstructionReset, ConstructionResetRequest as R, InitialResetRequest as S,
        },
    };
    // Count both Init invocations in one attribute. Main yields without returning
    // so the second Init must preserve that suspended frame and its wake time.
    let mut p = program(vec![expression(0, 0, 3, 7, 1, 254, 253)]);
    let m = p.modules.get_mut("local").unwrap();
    m.routines.insert(
        4097,
        routine(vec![
            expression(9, 0, 5, 7, 3, 1, 253),
            instruction(0, 254, 253, [0; 8]),
        ]),
    );
    let mut words = vec![0; 108];
    words[9] = 2;
    m.object_definition_words.insert(1, words);
    m.object_functions.insert(
        1,
        vec![
            ObjectFunction {
                action: 4096,
                condition: 0,
            },
            ObjectFunction {
                action: 4097,
                condition: 0,
            },
        ],
    );
    let mut finals = vec![];
    for budget in [1, 100] {
        let o = Object::construct_person(
            Definition {
                source: "local".into(),
                object: 1,
                guid: 0,
                slots: 0,
            },
            &p,
            5,
        )
        .unwrap();
        let mut w = World::default();
        w.objects.insert(1, o);
        w.positions.insert(
            1,
            sims_vm::autonomy::Placement {
                x: -16,
                y: -16,
                level: 1,
            },
        );
        let mut vm = Vm::new(w, 1);
        vm.tick = 9876;
        let mut reset = ConstructionReset::begin(&mut vm, 1, ObjectDate::default()).unwrap();
        let mut updates = 0;
        while let Some(request) = reset.next(&mut vm, &p).unwrap() {
            // A complete checkpoint includes both the VM and constructor token.
            let bytes = serde_json::to_vec(&(Snapshot::capture(&p, &vm), &reset)).unwrap();
            let (snapshot, restored): (Snapshot, ConstructionReset) =
                serde_json::from_slice(&bytes).unwrap();
            vm = snapshot.restore(&p).unwrap();
            reset = restored;
            assert_eq!(reset.next(&mut vm, &p).unwrap(), Some(request));
            match request {
                R::Script(id) => {
                    vm.run_slice(&p, id, budget).unwrap();
                    continue;
                }
                R::ImmediatePersonUpdate { object, tick } => {
                    assert_eq!((object, tick), (1, 0));
                    assert_eq!(vm.world.objects[&1].attributes[0], 1);
                    assert_eq!(vm.tick, 9876);
                    updates += 1;
                }
                // These synthetic host services are covered independently by
                // native fixtures; this regression tests callback composition.
                R::Service(S::BaseReset { .. } | S::Init) => panic!("uncomposed callback"),
                R::Service(_) => {}
            }
            let before = reset.clone();
            assert!(reset.complete(&mut vm, &p, R::Script(999), None).is_err());
            assert_eq!(reset, before);
            reset
                .complete(
                    &mut vm,
                    &p,
                    request,
                    (request == R::Service(S::IsResidential)).then_some(true),
                )
                .unwrap();
        }
        assert_eq!(updates, 1);
        assert_eq!(vm.world.objects[&1].attributes[0], 2);
        assert!(matches!(
            vm.threads[&vm.object_threads[&1]].status,
            Status::Sleeping { .. }
        ));
        assert!(ConstructionReset::begin(&mut vm, 1, ObjectDate::default()).is_err());
        finals.push(vm);
    }
    assert_eq!(finals[0], finals[1]);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn factory_price_store_matches_native_and_preserves_init_placement_flags() {
    use sims_formats::execution::ObjectFunction;
    use sims_vm::{Definition, construction::ObjectDate};
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/factory-price.json")).unwrap();
    for c in report["cases"].as_array().unwrap() {
        let mut p = program(vec![
            expression(9, 0, 5, 7, 3, 1, 253),
            instruction(0, 254, 253, [0; 8]),
        ]);
        let mut words = vec![0; 108];
        words[18] = c["price"].as_i64().unwrap() as i16;
        let m = p.modules.get_mut("local").unwrap();
        m.object_definition_words.insert(1, words);
        m.object_functions.insert(
            1,
            vec![
                ObjectFunction {
                    action: 0,
                    condition: 0,
                },
                ObjectFunction {
                    action: 4096,
                    condition: 0,
                },
            ],
        );
        let mut w = World::default();
        w.objects.insert(
            1,
            Object::construct(
                Definition {
                    source: "local".into(),
                    object: 1,
                    guid: 0,
                    slots: 0,
                },
                &p,
                0,
            )
            .unwrap(),
        );
        let mut vm = Vm::new(w, 1);
        let thread = vm
            .reset_object_lifecycle(&p, 1, ObjectDate::default())
            .unwrap();
        vm.run_slice(&p, thread, 100).unwrap();
        for field in [41, 42] {
            vm.world
                .objects
                .get_mut(&1)
                .unwrap()
                .data
                .insert(field, c["before"][field as usize].as_i64().unwrap() as i16);
        }
        vm.finish_object_factory(&p, 1).unwrap();
        assert_eq!(vm.world.objects[&1].native_flags, Some(0x280));
        for field in [41, 42] {
            assert_eq!(
                serde_json::json!(vm.world.objects[&1].data[&field]),
                c["after"][field as usize]
            );
        }
    }
}
