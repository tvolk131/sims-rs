use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;

fn instruction(opcode: u16, operands: [u8; 8], yes: u8, no: u8) -> Instruction {
    Instruction {
        opcode,
        operands,
        on_true: yes,
        on_false: no,
    }
}
fn routine(name: &str, args: u8, instructions: Vec<Instruction>) -> Routine {
    Routine {
        name: name.into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args,
            locals: 2,
            instructions,
        },
    }
}
fn expression(index: i16, rhs: i16, scope: u8, branch: u8, op: u8) -> Instruction {
    let [a, b] = index.to_le_bytes();
    let [c, d] = rhs.to_le_bytes();
    instruction(2, [a, b, c, d, 0, op, 8, scope], branch, 253)
}
fn fixture(
    mode: u8,
    target: i16,
    found: bool,
    result: bool,
    empty: bool,
) -> (Program, Vm, u64, u64) {
    let mut p = Program {
        globals: "caller".into(),
        ..Default::default()
    };
    let done = |ret| instruction(2, [0, 0, 0, 0, 0, 2, 7, 7], ret, 253);
    for (id, owner) in [(1, "caller"), (2, "target")] {
        let main = if id == 1 {
            vec![
                instruction(28, [47, 1, 0, 0, 1, mode, 0, 0], 1, 2),
                done(254),
                done(255),
            ]
        } else if empty {
            vec![done(254)]
        } else {
            vec![instruction(0, [0; 8], 254, 255)]
        };
        let mut m = Module {
            source: owner.into(),
            strings: [(303, vec!["fixture function".into()])].into(),
            routines: [(4096, routine("Main", 4, main))].into(),
            ..Default::default()
        };
        m.object_functions.insert(
            1,
            vec![
                sims_formats::execution::ObjectFunction {
                    action: 0,
                    condition: 0,
                },
                sims_formats::execution::ObjectFunction {
                    action: 4096,
                    condition: 0,
                },
            ],
        );
        if found {
            m.routines.insert(
                4097,
                routine(
                    "fixture function",
                    4,
                    vec![
                        expression(1, 0, 9, 1, 5),
                        expression(2, 0, 10, 2, 5),
                        expression(3, 11, 3, 3, 5),
                        expression(4, 1, 7, if result { 254 } else { 255 }, 3),
                    ],
                ),
            );
        }
        p.modules.insert(owner.into(), m);
    }
    let mut world = World::default();
    for (id, owner, temps) in [
        (1, "caller", [11, 22, 33, 44, 2, 66, 77, 88]),
        (2, "target", [201, 202, 203, 204, 205, 206, 207, 208]),
    ] {
        let mut data: std::collections::BTreeMap<_, _> = (0..72).map(|n| (n, 0)).collect();
        data.insert(11, id);
        world.objects.insert(
            id,
            Object {
                definition: Some(Definition {
                    source: owner.into(),
                    object: 1,
                    guid: 0,
                    slots: 0,
                }),
                data,
                temps,
                ..Default::default()
            },
        );
    }
    let mut vm = Vm::new(world, 1);
    let mut ids = vec![];
    for (actor, owner) in [(1, "caller"), (2, "target")] {
        let id = vm
            .spawn(
                &p,
                Start {
                    owner,
                    bhav: 4096,
                    caller: actor,
                    callee: actor,
                    stack_object: if actor == 1 { target } else { 1 },
                    args: vec![1000, 2, 3, 4],
                    check: false,
                },
            )
            .unwrap();
        vm.object_threads.insert(actor, id);
        ids.push(id);
    }
    vm.run_slice(&p, ids[1], 10).unwrap();
    vm.lifecycles.insert(
        2,
        lifecycle::Lifecycle {
            owner: "target".into(),
            definition: 1,
            init: 0,
            main: 4096,
            phase: if empty {
                lifecycle::Phase::Finished
            } else {
                lifecycle::Phase::Main
            },
            thread: ids[1],
            run_init: false,
        },
    );
    (p, vm, ids[0], ids[1])
}

#[test]
fn ui_effect_host_calls_preserve_synchronous_named_callback_ownership() {
    let (mut p, initial, root, target) = fixture(1, 2, true, true, false);
    p.modules
        .get_mut("target")
        .unwrap()
        .routines
        .get_mut(&4097)
        .unwrap()
        .body
        .instructions = vec![instruction(0x22, [0, 7, 0xdd, 7, 1, 0, 0, 0], 254, 253)];
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut vm = initial.clone();
        let mut calls = 0;
        for _ in 0..20 {
            let run = vm.run_slice(&p, root, budget).unwrap();
            if run.stop == Stop::AwaitingUiEffect {
                assert_eq!(vm.named_calls.len(), 1);
                let Status::AwaitingUiEffect(call) = &vm.threads[&target].status else {
                    panic!()
                };
                let request = call.request();
                assert_eq!(request.value, 2013);
                assert!(vm.advance_tick().is_err());
                vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
                vm.complete_ui_effect(&p, target, request).unwrap();
                calls += 1;
            }
            if run.stop == Stop::Returned(true) {
                break;
            }
        }
        assert_eq!(calls, 2);
        assert!(vm.named_calls.is_empty());
        assert_eq!(vm.threads[&root].status, Status::Returned(true));
        assert_eq!(vm.threads[&target].status, initial.threads[&target].status);
        assert_eq!(vm.threads[&target].stack, initial.threads[&target].stack);
        outcomes.push(vm);
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn callbacks_match_original_vm_identity_temps_and_frame_restoration() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/named-callback-execution.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let (p, initial, root, target) = fixture(
            c["mode"].as_u64().unwrap() as u8,
            c["target"].as_i64().unwrap() as i16,
            true,
            c["result"].as_bool().unwrap(),
            c["empty"].as_bool().unwrap(),
        );
        let target_before = initial.threads[&target].clone();
        let mut outcomes = vec![];
        for budget in [1, 1000] {
            let mut vm = initial.clone();
            let mut saved = false;
            for _ in 0..50 {
                let run = vm.run_slice(&p, root, budget).unwrap();
                assert_ne!(run.stop, Stop::Faulted, "{c}: {:?}", run.trace);
                if !vm.named_calls.is_empty() {
                    assert!(vm.advance_tick().is_err());
                    assert!(vm.run_slice(&p, target, 1).is_err());
                    let mut bad = Snapshot::capture(&p, &vm);
                    bad.state.named_calls.clear();
                    assert!(bad.restore(&p).is_err());
                    saved = true;
                }
                vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
                if let Status::Returned(result) = vm.threads[&root].status {
                    assert_eq!(result, n["returned"] == 1);
                    break;
                }
            }
            assert!(
                matches!(vm.threads[&root].status, Status::Returned(_)),
                "{c}"
            );
            assert_eq!(
                json!([vm.world.objects[&1].temps, vm.world.objects[&2].temps]),
                n["temps"],
                "{c}"
            );
            assert_eq!(vm.threads[&target].stack, target_before.stack);
            assert_eq!(vm.threads[&target].status, target_before.status);
            assert_eq!(vm.world.objects[&2].data[&62], 0);
            assert_eq!(saved, budget == 1 && c["mode"] == 1 && c["target"] == 2);
            outcomes.push(vm);
        }
        assert_eq!(outcomes[0], outcomes[1], "{c}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_named_callback_dispatch_guards_and_returns() {
    let report: Value = serde_json::from_str(include_str!("fixtures/named-callback.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let (p, mut vm, root, _) = fixture(
            c["mode"].as_u64().unwrap() as u8,
            c["target"].as_i64().unwrap() as i16,
            c["found"].as_bool().unwrap(),
            c["result"].as_bool().unwrap(),
            false,
        );
        for _ in 0..10 {
            let run = vm.run_slice(&p, root, 100).unwrap();
            if run.stop != Stop::Budget {
                break;
            }
        }
        if n["error"].is_null() {
            assert_eq!(
                vm.threads[&root].status,
                Status::Returned(n["returned"] == 1),
                "{c}"
            );
        } else {
            assert!(
                matches!(vm.threads[&root].status, Status::Faulted(_)),
                "{c}"
            );
        }
    }
}

#[test]
fn reentrant_cross_vm_callbacks_preserve_all_ordinary_continuations() {
    let (mut p, initial, root, target) = fixture(1, 2, true, true, false);
    for m in p.modules.values_mut() {
        m.strings.insert(
            303,
            vec!["fixture function".into(), "back".into(), "leaf".into()],
        );
    }
    let stack = |object: u8| instruction(2, [0, 0, object, 0, 0, 5, 10, 7], 1, 253);
    let named = |name| instruction(28, [47, 1, 0, 0, name, 1, 0, 0], 2, 253);
    p.modules.get_mut("target").unwrap().routines.insert(
        4097,
        routine(
            "fixture function",
            4,
            vec![stack(1), named(2), expression(6, 1, 7, 254, 3)],
        ),
    );
    p.modules.get_mut("caller").unwrap().routines.insert(
        4098,
        routine(
            "back",
            4,
            vec![stack(2), named(3), expression(5, 1, 7, 254, 3)],
        ),
    );
    p.modules
        .get_mut("target")
        .unwrap()
        .routines
        .insert(4099, routine("leaf", 4, vec![expression(4, 1, 7, 254, 3)]));
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut vm = initial.clone();
        let mut depth = 0;
        for _ in 0..100 {
            let run = vm.run_slice(&p, root, budget).unwrap();
            assert_ne!(run.stop, Stop::Faulted, "{:?}", run.trace);
            depth = depth.max(vm.named_calls.len());
            vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
            if let Status::Returned(true) = vm.threads[&root].status {
                break;
            }
        }
        assert_eq!(vm.threads[&root].status, Status::Returned(true));
        assert_eq!(vm.threads[&target].stack, initial.threads[&target].stack);
        assert_eq!(vm.threads[&target].status, initial.threads[&target].status);
        assert_eq!(vm.world.objects[&1].temps[5], 67);
        assert_eq!(vm.world.objects[&2].temps[4], 206);
        assert_eq!(vm.world.objects[&2].temps[6], 208);
        assert!(vm.named_calls.is_empty());
        assert_eq!(depth, if budget == 1 { 3 } else { 0 });
        outcomes.push(vm);
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn forbidden_yield_faults_the_logical_caller_without_running_recipient_main() {
    let (mut p, mut vm, root, target) = fixture(1, 2, true, true, false);
    p.modules
        .get_mut("target")
        .unwrap()
        .routines
        .get_mut(&4097)
        .unwrap()
        .body
        .instructions = vec![instruction(0, [0; 8], 254, 255)];
    let before = vm.threads[&target].stack.clone();
    let runs = vm.run_round(&p, 100);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, root);
    assert_eq!(runs[0].1.stop, Stop::Faulted);
    assert!(matches!(vm.threads[&root].status, Status::Faulted(_)));
    assert!(matches!(vm.threads[&target].status, Status::Faulted(_)));
    assert_eq!(vm.threads[&target].stack[..before.len()], before);
    assert_eq!(vm.tick, 0);
    assert!(vm.advance_tick().is_err());
    // Preserve the causal chain for diagnostics rather than fabricate a false
    // callback return or let the recipient continue its ordinary Main.
    assert_eq!(vm.named_calls.len(), 1);
}

#[test]
fn named_callback_can_pause_for_an_accessory_without_vm_yield_or_replay() {
    let (mut p, mut initial, root, target) = fixture(1, 2, true, true, false);
    initial.world.objects.get_mut(&2).unwrap().person = Some(Default::default());
    let m = p.modules.get_mut("target").unwrap();
    m.strings.insert(304, vec!["SUIT-FORK".into()]);
    m.routines.get_mut(&4097).unwrap().body.instructions =
        vec![instruction(6, [0, 2, 0, 0, 0, 0, 0, 0], 254, 255)];
    let mut outcomes = vec![];
    for budget in [1, 1000] {
        let mut vm = initial.clone();
        for _ in 0..10 {
            let run = vm.run_slice(&p, root, budget).unwrap();
            vm = serde_json::from_slice::<Snapshot>(
                &serde_json::to_vec(&Snapshot::capture(&p, &vm)).unwrap(),
            )
            .unwrap()
            .restore(&p)
            .unwrap();
            if run.stop == Stop::AwaitingAccessory {
                break;
            }
            assert_eq!(run.stop, Stop::Budget);
        }
        assert!(matches!(
            vm.threads[&target].status,
            Status::AwaitingAccessory { .. }
        ));
        assert_eq!(
            vm.pending_accessory_assets(&p),
            ["suit-fork".to_string()].into()
        );
        assert!(vm.advance_tick().is_err());
        let mut loaded = p.clone();
        loaded
            .accessories
            .insert("suit-fork".into(), Default::default());
        vm = Snapshot::capture(&loaded, &vm).restore(&loaded).unwrap();
        assert_eq!(
            vm.run_slice(&loaded, root, 1000).unwrap().stop,
            Stop::Returned(true)
        );
        assert!(vm.named_calls.is_empty());
        outcomes.push(vm);
    }
    p.accessories.insert("suit-fork".into(), Default::default());
    initial.run_slice(&p, root, 1000).unwrap();
    assert_eq!(outcomes[0], outcomes[1]);
    assert_eq!(outcomes[0], initial);
}
