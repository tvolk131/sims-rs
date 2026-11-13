//! Authored in-memory controls for the real public owner.
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{
    action::{Action, Actor, WorldState},
    action_observation::{Evidence, Fact, Unavailable},
    program::MenuEntry,
    *,
};
use std::collections::BTreeMap;

fn predicate(value: bool, branch: u8) -> Instruction {
    Instruction {
        opcode: 2,
        on_true: branch,
        on_false: branch,
        operands: [0, 0, u8::from(!value), 0, 0, 2, 7, 7],
    }
}

fn routine(instructions: Vec<Instruction>) -> Routine {
    Routine {
        name: "authored action observation fixture".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 4,
            locals: 0,
            instructions,
        },
    }
}

fn record(serial: u32, priority: i32) -> Action {
    Action {
        id: serial,
        target: 2,
        icon_owner: 2,
        menu_id: 19,
        priority,
        routine: 4097,
        ..Default::default()
    }
}

fn fixture() -> (Program, Vm, ThreadId) {
    let module = Module {
        source: "authored".into(),
        routines: [
            (
                4096,
                routine(vec![Instruction {
                    opcode: 17,
                    on_true: 254,
                    on_false: 255,
                    operands: [0, 0, 1, 0, 0, 0, 0, 0],
                }]),
            ),
            (4097, routine(vec![predicate(true, 254)])),
            (4098, routine(vec![predicate(true, 254)])),
        ]
        .into(),
        object_interactions: [
            (1, vec![]),
            (
                2,
                vec![MenuEntry {
                    id: 19,
                    flags: 1,
                    action: 4097,
                    check: 0,
                    ..Default::default()
                }],
            ),
        ]
        .into(),
        object_definition_words: [(1, vec![0; 100]), (2, vec![0; 100])].into(),
        ..Default::default()
    };
    let program = Program {
        globals: "authored".into(),
        modules: [("authored".into(), module)].into(),
        ..Default::default()
    };
    let mut world = World {
        scheduler_storage: Some(sims_vm::scheduler::storage::Storage::default()),
        actions: Some(WorldState {
            actors: [(1, Actor::default())].into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    for id in 1..=2 {
        let mut person: BTreeMap<i16, i16> = (0..256).map(|index| (index, 0)).collect();
        person.insert(58, 30);
        let mut object = Object {
            definition: Some(Definition {
                source: "authored".into(),
                object: id as u16,
                guid: id as u32,
                slots: 0,
            }),
            person: (id == 1).then_some(person),
            data: (0..80).map(|index| (index, 0)).collect(),
            motives: (id == 1).then_some([0; 16]),
            ..Default::default()
        };
        world.register_scheduler_object(id, &mut object).unwrap();
        world.objects.insert(id, object);
    }
    let mut vm = Vm::new(world, 123);
    let id = vm
        .spawn(
            &program,
            Start {
                owner: "authored",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 2,
                args: vec![0; 4],
                check: false,
            },
        )
        .unwrap();
    vm.register_object_thread(1, id).unwrap();
    (program, vm, id)
}

#[test]
fn public_owner_budgets_and_recording_are_state_and_trace_transparent() {
    let (program, mut initial, thread) = fixture();
    initial.enqueue_action(&program, 1, record(0, 100)).unwrap();
    let mut final_states = Vec::new();
    for budget in [0, 1, 2, 1000] {
        let mut ordinary = initial.clone();
        let mut disabled = initial.clone();
        let mut observed = initial.clone();
        let mut facts = Vec::new();
        for _ in 0..32 {
            let expected = ordinary.run_slice(&program, thread, budget);
            assert_eq!(
                disabled.run_slice_with_action_observer(&program, thread, budget, &mut |_| {}),
                expected
            );
            let actual =
                observed.run_slice_with_action_observer(&program, thread, budget, &mut |fact| {
                    facts.push(fact)
                });
            assert_eq!(actual, expected);
            assert_eq!(ordinary, disabled);
            assert_eq!(ordinary, observed);
            assert_eq!(
                serde_json::to_vec(&Snapshot::capture(&program, &ordinary)).unwrap(),
                serde_json::to_vec(&Snapshot::capture(&program, &observed)).unwrap()
            );
            let run = actual.unwrap();
            assert!(run.trace.len() <= budget);
            if budget == 0 || run.stop != Stop::Budget {
                break;
            }
        }
        if budget == 0 {
            assert!(
                !facts
                    .iter()
                    .any(|fact| matches!(fact, Fact::FrameBound { .. } | Fact::Returned { .. }))
            );
        } else {
            assert!(
                facts
                    .iter()
                    .any(|fact| matches!(fact, Fact::FrameBound { frame, action }
                if frame.thread == Some(thread) && action.actor.id == Some(1)))
            );
            assert!(facts.iter().any(|fact| matches!(fact, Fact::Returned { frame, binding, .. }
                if frame.thread == Some(thread) && *binding == Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding))));
            final_states.push(observed);
        }
    }
    assert!(final_states.windows(2).all(|pair| pair[0] == pair[1]));
}

fn named_fixture() -> (Program, Vm, ThreadId, ThreadId) {
    let (mut program, mut vm, requester) = fixture();
    let module = program.modules.get_mut("authored").unwrap();
    module.strings.insert(303, vec!["named".into()]);
    module.routines.insert(
        4096,
        routine(vec![
            Instruction {
                opcode: 28,
                operands: [47, 1, 0, 0, 1, 1, 0, 0],
                on_true: 1,
                on_false: 253,
            },
            predicate(true, 254),
        ]),
    );
    let mut named = routine(vec![predicate(true, 254)]);
    named.name = "named".into();
    module.routines.insert(4100, named);
    module.routines.insert(
        4101,
        routine(vec![Instruction {
            opcode: 0,
            operands: [0; 8],
            on_true: 254,
            on_false: 255,
        }]),
    );
    let recipient = vm
        .spawn(
            &program,
            Start {
                owner: "authored",
                bhav: 4101,
                caller: 2,
                callee: 2,
                stack_object: 2,
                args: vec![1000, 0, 0, 0],
                check: false,
            },
        )
        .unwrap();
    vm.register_object_thread(2, recipient).unwrap();
    vm.run_slice(&program, recipient, 1).unwrap();
    vm.lifecycles.insert(
        2,
        lifecycle::Lifecycle {
            owner: "authored".into(),
            definition: 2,
            init: 0,
            main: 4101,
            phase: lifecycle::Phase::Main,
            thread: recipient,
            run_init: false,
        },
    );
    (program, vm, requester, recipient)
}

#[test]
fn actual_named_recipient_is_not_misattributed_or_adopted_after_reentry() {
    for budget in [0, 1, 2, 1000] {
        let (program, mut vm, requester, recipient) = named_fixture();
        assert_eq!(
            vm.run_slice(&program, requester, 1).unwrap().stop,
            Stop::Budget
        );
        assert_eq!(vm.named_calls.last().unwrap().recipient, recipient);
        vm.threads
            .get_mut(&recipient)
            .unwrap()
            .stack
            .last_mut()
            .unwrap()
            .action_target = Some(1);
        let mut ordinary = vm.clone();
        let mut facts = Vec::new();
        for _ in 0..16 {
            let expected = ordinary.run_slice(&program, requester, budget);
            let actual =
                vm.run_slice_with_action_observer(&program, requester, budget, &mut |fact| {
                    facts.push(fact)
                });
            assert_eq!(actual, expected);
            assert_eq!(vm, ordinary);
            let run = actual.unwrap();
            assert!(run.trace.len() <= budget);
            if budget == 0 || run.stop != Stop::Budget {
                break;
            }
        }
        if budget > 0 {
            let returns: Vec<_> = facts
                .iter()
                .filter_map(|fact| match fact {
                    Fact::Returned { frame, binding, .. } => Some((frame, binding)),
                    _ => None,
                })
                .collect();
            assert_eq!(returns.len(), 1);
            assert_eq!(returns[0].0.thread, Some(recipient));
            assert_eq!(returns[0].0.caller.id, Some(2));
            assert_eq!(
                *returns[0].1,
                Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding)
            );
            assert!(
                !facts
                    .iter()
                    .any(|fact| matches!(fact, Fact::FrameBound { .. }))
            );
            assert!(vm.named_calls.is_empty());
        }
    }
}

fn assert_rejected(
    program: &Program,
    initial: &Vm,
    thread: ThreadId,
    message: &str,
    unchanged: bool,
) -> Vm {
    let mut ordinary = initial.clone();
    let mut observed = initial.clone();
    let expected = ordinary.run_slice(program, thread, 100);
    let mut facts = Vec::new();
    assert_eq!(
        observed.run_slice_with_action_observer(program, thread, 100, &mut |fact| facts.push(fact)),
        expected
    );
    assert!(expected.unwrap_err().contains(message));
    assert_eq!(observed, ordinary);
    assert!(
        !facts
            .iter()
            .any(|fact| matches!(fact, Fact::FrameBound { .. } | Fact::Returned { .. }))
    );
    if unchanged {
        assert_eq!(observed, *initial);
    }
    observed
}

#[test]
fn menu_and_scheduled_guards_precede_accessory_publication() {
    let (mut program, mut vm, thread) = fixture();
    program
        .accessories
        .insert("loaded".into(), Default::default());
    let origin = vm.threads[&thread].stack[0].location.clone();
    vm.threads.get_mut(&thread).unwrap().status = Status::AwaitingAccessory {
        request: 0,
        origin,
        caller: 1,
        name: "loaded".into(),
    };
    vm.begin_player_menu(&program, 1, 2).unwrap();
    assert_rejected(&program, &vm, thread, "player-menu query owns", true);
    vm.player_menu = None;
    vm.threads.get_mut(&thread).unwrap().status = Status::Ready;
    vm.begin_scheduled_object_visit(1).unwrap();
    assert_rejected(&program, &vm, 999, "native object visit owns", true);
    vm.run_slice(&program, thread, 100).unwrap();
    vm.finish_object_update(1, true).unwrap();
    assert_rejected(
        &program,
        &vm,
        thread,
        "native object update has already returned",
        true,
    );
}

#[test]
fn nonadmitted_native_owner_only_exercises_early_guard_not_source_admission() {
    let (mut program, mut vm, thread) = fixture();
    let recipient = vm
        .spawn(
            &program,
            Start {
                owner: "authored",
                bhav: 4098,
                caller: 2,
                callee: 2,
                stack_object: 2,
                args: vec![0; 4],
                check: false,
            },
        )
        .unwrap();
    let origin = vm.threads[&recipient].stack[0].location.clone();
    vm.threads.get_mut(&recipient).unwrap().status = Status::AwaitingAccessory {
        request: 0,
        origin,
        caller: 2,
        name: "loaded".into(),
    };
    program
        .accessories
        .insert("loaded".into(), Default::default());
    let owner = serde_json::from_value::<native_suspension::Owner>(serde_json::json!({
        "evidence": {"image_sha256": "authored-nonadmitted", "entry_pc": 0,
            "caller_return": 0, "entry_code": ([0; 32])},
        "binding": {"object": 1, "definition": vm.world.objects[&1].definition,
            "native_context": 0, "registry_lifetime": null},
        "ordinary": []
    }))
    .unwrap();
    vm.threads.get_mut(&thread).unwrap().status = Status::NativeSuspended(owner);
    assert_rejected(&program, &vm, thread, "native suspension requires", true);
}

#[test]
fn action_callbacks_stop_before_dispatch_under_the_public_owner() {
    let (mut program, mut vm, thread) = fixture();
    let mut functions = vec![
        sims_formats::execution::ObjectFunction {
            action: 0,
            condition: 0
        };
        5
    ];
    functions[4].action = 4098;
    program
        .modules
        .get_mut("authored")
        .unwrap()
        .object_functions
        .insert(2, functions);
    vm.world
        .actions
        .as_mut()
        .unwrap()
        .callbacks
        .push_back(action::Callback {
            object: 2,
            function: 4,
            stack_object: 1,
        });
    let mut ordinary = vm.clone();
    let mut facts = Vec::new();
    let expected = ordinary.run_slice(&program, thread, 100).unwrap();
    assert_eq!(expected.stop, Stop::ActionCallbacks);
    assert!(expected.trace.is_empty());
    assert_eq!(
        vm.run_slice_with_action_observer(&program, thread, 100, &mut |fact| facts.push(fact))
            .unwrap(),
        expected
    );
    assert_eq!(vm, ordinary);
    assert!(
        !facts
            .iter()
            .any(|fact| matches!(fact, Fact::FrameBound { .. } | Fact::Returned { .. }))
    );
}

#[test]
fn named_fault_and_ui_stops_share_budgets_and_original_callbacks() {
    for budget in [1, 2, 1000] {
        for opcode in [0xffff, 0x22] {
            let (mut program, mut vm, requester, recipient) = named_fixture();
            program
                .modules
                .get_mut("authored")
                .unwrap()
                .routines
                .get_mut(&4100)
                .unwrap()
                .body
                .instructions = vec![Instruction {
                opcode,
                operands: if opcode == 0xffff {
                    [0; 8]
                } else {
                    [0, 7, 0xdd, 7, 1, 0, 0, 0]
                },
                on_true: 254,
                on_false: 255,
            }];
            let mut ordinary = vm.clone();
            let mut reached_stop = false;
            for _ in 0..16 {
                let expected = ordinary.run_slice(&program, requester, budget);
                let actual =
                    vm.run_slice_with_action_observer(&program, requester, budget, &mut |_| {});
                assert_eq!(actual, expected);
                assert_eq!(vm, ordinary);
                let run = actual.unwrap();
                assert!(run.trace.len() <= budget);
                if opcode == 0xffff && run.stop == Stop::Faulted {
                    assert!(matches!(vm.threads[&requester].status, Status::Faulted(_)));
                    reached_stop = true;
                    break;
                }
                if opcode == 0x22 && run.stop == Stop::AwaitingUiEffect {
                    let Status::AwaitingUiEffect(call) = &vm.threads[&recipient].status else {
                        panic!("recipient UI owner missing")
                    };
                    let request = call.request();
                    vm.complete_ui_effect(&program, recipient, request).unwrap();
                    ordinary
                        .complete_ui_effect(&program, recipient, request)
                        .unwrap();
                    assert_eq!(vm, ordinary);
                    assert_eq!(
                        vm.run_slice_with_action_observer(&program, requester, 100, &mut |_| {}),
                        ordinary.run_slice(&program, requester, 100)
                    );
                    assert_eq!(vm, ordinary);
                    reached_stop = true;
                    break;
                }
            }
            assert!(reached_stop, "opcode {opcode:#x}, budget {budget}");
        }
    }
}

#[test]
fn named_owner_rejection_keeps_earlier_accessory_side_effect() {
    let (mut program, mut vm, requester, recipient) = named_fixture();
    vm.run_slice(&program, requester, 1).unwrap();
    program
        .accessories
        .insert("loaded".into(), Default::default());
    let origin = vm.threads[&requester]
        .stack
        .last()
        .unwrap()
        .location
        .clone();
    vm.threads.get_mut(&requester).unwrap().status = Status::AwaitingAccessory {
        request: 0,
        origin,
        caller: 1,
        name: "loaded".into(),
    };
    let after = assert_rejected(&program, &vm, recipient, "another VM owns", false);
    assert_eq!(after.threads[&requester].status, Status::Ready);
    assert_ne!(after, vm);
}

#[test]
fn actual_named_accessory_pause_and_resume_preserve_the_recipient_owner() {
    for budget in [1, 2, 1000] {
        let (mut program, mut vm, requester, recipient) = named_fixture();
        vm.world.objects.get_mut(&2).unwrap().person = Some(Default::default());
        let module = program.modules.get_mut("authored").unwrap();
        module.strings.insert(304, vec!["SUIT-AUTHORED".into()]);
        module.routines.get_mut(&4100).unwrap().body.instructions = vec![Instruction {
            opcode: 6,
            operands: [0, 2, 0, 0, 0, 0, 0, 0],
            on_true: 254,
            on_false: 255,
        }];
        let mut ordinary = vm.clone();
        let mut paused = false;
        for _ in 0..16 {
            let expected = ordinary.run_slice(&program, requester, budget);
            let actual =
                vm.run_slice_with_action_observer(&program, requester, budget, &mut |_| {});
            assert_eq!(actual, expected);
            assert_eq!(vm, ordinary);
            if actual.unwrap().stop == Stop::AwaitingAccessory {
                assert!(matches!(
                    vm.threads[&recipient].status,
                    Status::AwaitingAccessory { .. }
                ));
                assert!(!vm.named_calls.is_empty());
                paused = true;
                break;
            }
        }
        assert!(paused);
        program
            .accessories
            .insert("suit-authored".into(), Default::default());
        assert_eq!(
            vm.run_slice_with_action_observer(&program, requester, 1000, &mut |_| {}),
            ordinary.run_slice(&program, requester, 1000)
        );
        assert_eq!(vm, ordinary);
        assert!(vm.named_calls.is_empty());
    }
}

#[test]
fn restored_active_frames_get_fresh_sinks_without_serial_binding_recovery() {
    let (program, mut vm, thread) = fixture();
    vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
    let mut before_restore = Vec::new();
    vm.run_slice_with_action_observer(&program, thread, 1, &mut |fact| before_restore.push(fact))
        .unwrap();
    assert!(
        before_restore
            .iter()
            .any(|fact| matches!(fact, Fact::FrameBound { .. }))
    );
    let saved = serde_json::to_vec(&Snapshot::capture(&program, &vm)).unwrap();
    let mut restored = serde_json::from_slice::<Snapshot>(&saved)
        .unwrap()
        .restore(&program)
        .unwrap();
    let mut fresh = Vec::new();
    let expected = vm.run_slice(&program, thread, 100);
    assert_eq!(
        restored
            .run_slice_with_action_observer(&program, thread, 100, &mut |fact| fresh.push(fact)),
        expected
    );
    assert_eq!(restored, vm);
    assert!(
        !fresh
            .iter()
            .any(|fact| matches!(fact, Fact::FrameBound { .. }))
    );
    assert!(
        fresh
            .iter()
            .any(|fact| matches!(fact, Fact::Returned { binding, .. }
        if *binding == Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding)))
    );
}
