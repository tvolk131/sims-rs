//! Synthetic resources exercise recovered contracts without shipping game assets.
use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::ObjectFunction,
};
use sims_vm::{
    action::{self, Action, Actor, Queue, WorldState},
    program::MenuEntry,
    *,
};
use std::collections::{BTreeMap, VecDeque};

fn expr(scope: u8, index: i16, value: i16, next: u8) -> Instruction {
    let mut operands = [0; 8];
    operands[..2].copy_from_slice(&index.to_le_bytes());
    operands[2..4].copy_from_slice(&value.to_le_bytes());
    operands[5..].copy_from_slice(&[5, scope, 7]);
    Instruction {
        opcode: 2,
        on_true: next,
        on_false: 253,
        operands,
    }
}
fn routine(instructions: Vec<Instruction>) -> Routine {
    Routine {
        name: "synthetic action contract".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 4,
            locals: 0,
            instructions,
        },
    }
}
fn action(priority: i32, id: u32) -> Action {
    Action {
        id,
        priority,
        target: 2,
        icon_owner: 2,
        menu_id: 19,
        routine: 4097,
        ..Default::default()
    }
}
fn fixture(child_return: u8, check_return: u8) -> (Program, Vm) {
    let mut module = Module {
        source: "fixture".into(),
        ..Default::default()
    };
    module.routines = BTreeMap::from([
        (
            4096,
            routine(vec![Instruction {
                opcode: 17,
                on_true: 254,
                on_false: 255,
                operands: [0, 0, 1, 0, 0, 0, 0, 0],
            }]),
        ),
        (4097, routine(vec![expr(0, 1, 77, child_return)])),
        (
            4098,
            routine(vec![expr(8, 0, 99, 1), expr(0, 0, 321, check_return)]),
        ),
    ]);
    for id in [1, 2] {
        module.object_interactions.insert(
            id,
            vec![MenuEntry {
                autonomy: Default::default(),
                id: if id == 1 { 33 } else { 19 },
                flags: 1,
                label: "synthetic".into(),
                action: 4097,
                check: if id == 1 { 0 } else { 4098 },
            }],
        );
        module.object_functions.insert(
            id,
            vec![
                ObjectFunction {
                    action: 0,
                    condition: 0
                };
                5
            ],
        );
    }
    let program = Program {
        globals: "fixture".into(),
        modules: BTreeMap::from([("fixture".into(), module)]),
        ..Default::default()
    };
    let mut person: BTreeMap<i16, i16> = (0..256).map(|i| (i, 0)).collect();
    person.insert(58, 30);
    let objects = (1..=2)
        .map(|id| {
            (
                id,
                Object {
                    definition: Some(Definition {
                        source: "fixture".into(),
                        object: id as u16,
                        guid: 0,
                        slots: 0,
                    }),
                    person: (id == 1).then(|| person.clone()),
                    motives: (id == 1).then_some([0f32.to_bits(); 16]),
                    data: (0..72).map(|i| (i, 0)).collect(),
                    attributes: vec![0; 2],
                    temps: [12; 8],
                    ..Default::default()
                },
            )
        })
        .collect();
    let world = World {
        objects,
        actions: Some(WorldState {
            live_family: true,
            actors: BTreeMap::from([(1, Actor::default())]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut vm = Vm::new(world, 123);
    vm.enqueue_action(&program, 1, action(100, 0)).unwrap();
    vm.spawn(
        &program,
        Start {
            owner: "fixture",
            bhav: 4096,
            caller: 1,
            callee: 1,
            stack_object: 0,
            args: vec![0; 4],
            check: false,
        },
    )
    .unwrap();
    (program, vm)
}
fn restored(p: &Program, vm: &Vm) -> Vm {
    Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(p, vm)).unwrap(), p).unwrap()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn inherited_action_labels_match_original_queue_and_restore() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/action-label-inheritance.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 216);
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let record = |a: &Action| {
        serde_json::json!({
            "flags":a.flags,"label":a.label.as_deref().unwrap_or(""),"serial":a.id
        })
    };
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let (mut p, mut vm) = fixture(254, 254);
        let entry = &mut p
            .modules
            .get_mut("fixture")
            .unwrap()
            .object_interactions
            .get_mut(&2)
            .unwrap()[0];
        entry.flags = c["menu_flags"].as_u64().unwrap() as u32;
        entry.label = "Sit / Read".into();
        let mut source = action(50, 80);
        source.flags = c["source_flags"].as_u64().unwrap() as u32;
        source.label = Some(c["label"].as_str().unwrap().into());
        let mut immediate = source.clone();
        immediate.id = 81;
        let state = vm.world.actions.as_mut().unwrap();
        state.serial = 100;
        let actor = state.actors.get_mut(&1).unwrap();
        actor.active = (c["active"] == true).then_some(source);
        actor.queue.pending = (1..=c["count"].as_u64().unwrap() as u32)
            .map(|id| action(100, id))
            .collect();
        let owner = c["immediate"].as_i64().unwrap() as i16;
        state.immediate = (owner != 0).then_some((owner, immediate.clone()));
        let mut input = action(100, 99);
        input.flags = c["flags"].as_u64().unwrap() as u32;
        input.label = Some("new action".into());
        let mut resumed = restored(&p, &vm);
        let actual = vm.enqueue_action(&p, 1, input.clone()).unwrap();
        assert_eq!(resumed.enqueue_action(&p, 1, input).unwrap(), actual);
        assert_eq!(resumed, vm, "{c}");
        assert_eq!(restored(&p, &vm), vm, "{c}");
        assert!(n["error"].is_null());
        assert_eq!(serde_json::json!(actual.1), n["accepted"], "{c}");
        let state = vm.world.actions.as_ref().unwrap();
        assert_eq!(state.serial, 101);
        let actor = &state.actors[&1];
        assert_eq!(
            record(actor.active.as_ref().unwrap_or(&Action::default())),
            n["active"],
            "{c}"
        );
        assert_eq!(
            record(state.immediate.as_ref().map_or(&immediate, |(_, a)| a)),
            n["immediate"],
            "{c}"
        );
        assert_eq!(
            serde_json::json!(actor.queue.pending.iter().map(record).collect::<Vec<_>>()),
            n["queue"],
            "{c}"
        );
    }
}

#[test]
fn priority_insertion_only_truncates_the_trailing_suffix_and_front_overflow_drops_two() {
    let mut q = Queue {
        pending: [action(100, 1), action(2, 2), action(100, 3), action(2, 4)].into(),
    };
    let insertion = q.insert(action(50, 5)).unwrap();
    assert_eq!(
        insertion.cancelled.iter().map(|a| a.id).collect::<Vec<_>>(),
        [4]
    );
    assert_eq!(
        q.pending.iter().map(|a| a.id).collect::<Vec<_>>(),
        [1, 2, 3, 5]
    );
    let mut q = Queue {
        pending: (1..=8).map(|id| action(100, id)).collect(),
    };
    assert!(!q.insert(action(100, 9)).unwrap().accepted);
    let mut front = action(-5, 10);
    front.flags = action::PUSH_FRONT | 64;
    let insertion = q.insert(front).unwrap();
    assert!(insertion.cancelled.is_empty());
    assert_eq!(
        insertion.discarded.iter().map(|a| a.id).collect::<Vec<_>>(),
        [7, 8]
    );
    assert_eq!(
        q.pending.iter().map(|a| a.id).collect::<Vec<_>>(),
        [10, 1, 2, 3, 4, 5, 6]
    );
    assert_ne!(q.pending[0].flags & 128, 0);
}
#[test]
fn original_idle_dispatch_restores_temps_preserves_check_effects_and_ignores_child_return_branch() {
    for (child_return, check_args) in [(254, 4), (255, 4), (254, 5), (255, 5)] {
        let (mut p, mut vm) = fixture(child_return, 254);
        p.modules
            .get_mut("fixture")
            .unwrap()
            .routines
            .get_mut(&4098)
            .unwrap()
            .body
            .args = check_args;
        let mut reference = vm.clone();
        // Synchronous callback completion is a resumable VM boundary.
        while reference.run_slice(&p, 1, 100).unwrap().stop == Stop::Budget {}
        let mut saw_check = false;
        let mut saw_action = false;
        loop {
            let run = vm.run_slice(&p, 1, 1).unwrap();
            saw_check |= vm.threads[&1].callback.is_some();
            if check_args == 5 && vm.threads[&1].callback.is_some() {
                assert_eq!(
                    vm.threads[&1].stack.last().unwrap().args,
                    [0, 0, 0, 0, 4098]
                );
            }
            if vm.world.actions.as_ref().unwrap().actors[&1]
                .active
                .is_some()
            {
                saw_action = true;
                assert_eq!(vm.world.objects[&1].temps, [12; 8]);
                assert_eq!(vm.world.objects[&2].data[&62], 1);
                assert_eq!(vm.threads[&1].stack[0].continuation, 1);
            }
            vm = restored(&p, &vm);
            if run.stop != Stop::Budget {
                assert_eq!(run.stop, Stop::Returned(true));
                break;
            }
        }
        assert!(saw_check && saw_action);
        assert_eq!(vm, reference);
        assert_eq!(vm.world.objects[&1].attributes, [321, 77]);
        assert_eq!(vm.world.objects[&2].data[&62], 0);
        let actor = &vm.world.actions.as_ref().unwrap().actors[&1];
        assert!(actor.active.is_none());
        assert_eq!(
            actor.previous.as_ref().unwrap().flags,
            action::CHECKED | action::AVAILABLE | 32
        );
    }
}
#[test]
fn false_eligibility_keeps_persistent_effects_and_services_empty_cancellation_before_resuming() {
    let (p, mut vm) = fixture(254, 255);
    assert_eq!(vm.run_slice(&p, 1, 100).unwrap().stop, Stop::Budget);
    assert_eq!(vm.run_slice(&p, 1, 100).unwrap().stop, Stop::Sleeping);
    assert_eq!(vm.world.objects[&1].attributes, [321, 0]);
    assert_eq!(vm.world.objects[&1].temps, [12; 8]);
    assert_eq!(vm.world.actions.as_ref().unwrap().callbacks.len(), 1);
    vm = restored(&p, &vm);
    vm.tick += 1;
    assert_eq!(vm.run_slice(&p, 1, 100).unwrap().stop, Stop::Returned(true));
    assert!(vm.world.actions.as_ref().unwrap().callbacks.is_empty());
}
#[test]
fn missing_check_rejects_queued_action_preserves_sentinel_and_resumes() {
    let (mut p, mut vm) = fixture(254, 254);
    p.modules.get_mut("fixture").unwrap().routines.remove(&4098);
    assert_eq!(vm.run_slice(&p, 1, 100).unwrap().stop, Stop::Sleeping);
    assert_eq!(vm.world.objects[&1].attributes, [0, 0]);
    assert_eq!(vm.world.objects[&1].temps, [12; 8]);
    let state = vm.world.actions.as_ref().unwrap();
    assert_eq!(state.callbacks.len(), 1);
    assert!(state.actors[&1].active.is_none());
    assert!(state.actors[&1].queue.pending.is_empty());
    assert_eq!(
        &vm.threads[&1].frame_memory[12..20],
        &[
            Some(-1),
            Some(0),
            Some(0),
            Some(0),
            Some(1),
            Some(0),
            Some(0),
            Some(0)
        ]
    );
    vm = restored(&p, &vm);
    vm.tick += 1;
    assert_eq!(vm.run_slice(&p, 1, 100).unwrap().stop, Stop::Returned(true));
    assert!(vm.world.actions.as_ref().unwrap().callbacks.is_empty());
}
#[test]
fn active_cancellation_uses_native_self_menu_fallback_and_global_serial_wrap() {
    let (p, mut vm) = fixture(254, 254);
    vm.run_slice(&p, 1, 100).unwrap();
    vm.run_slice(&p, 1, 1).unwrap();
    let id = vm.world.actions.as_ref().unwrap().actors[&1]
        .active
        .as_ref()
        .unwrap()
        .id;
    assert!(vm.cancel_action(&p, 1, id).unwrap());
    let actor = &vm.world.actions.as_ref().unwrap().actors[&1];
    assert_ne!(actor.active.as_ref().unwrap().flags & action::CANCELLED, 0);
    assert_eq!(vm.world.objects[&1].person.as_ref().unwrap()[&33], 0);
    let fallback = &actor.queue.pending[0];
    assert_eq!(
        (
            fallback.target,
            fallback.menu_id,
            fallback.priority,
            fallback.icon_owner
        ),
        (1, 33, 50, 0)
    );
    vm = restored(&p, &vm);
    vm.world.actions.as_mut().unwrap().serial = u32::MAX;
    assert_eq!(vm.enqueue_action(&p, 1, action(100, 0)).unwrap(), (0, true));
    vm.service_empty_action_callbacks(&p).unwrap();
    restored(&p, &vm);
}
#[test]
fn idle_negative_timer_wrap_and_interruption_locks_are_native_rules() {
    let (_, vm) = fixture(254, 254);
    let mut person = vm.world.objects[&1].clone();
    let queue = Queue {
        pending: VecDeque::from([action(100, 1)]),
    };
    assert!(queue.should_interrupt(&person, true).unwrap());
    person.person.as_mut().unwrap().insert(71, 1);
    assert!(!queue.should_interrupt(&person, true).unwrap());
    let mut counter = i16::MIN;
    let mut flags = 0x40;
    assert_eq!(
        action::idle_decision(&queue, false, &mut counter, &mut flags, 1, 0),
        action::IdleDecision::Yield
    );
    assert_eq!((counter, flags), (i16::MAX, 0x40));
    assert_eq!(
        action::idle_decision(&queue, true, &mut counter, &mut flags, 1, 1),
        action::IdleDecision::Complete
    );
}
#[test]
fn snapshot_rejects_forged_dispatch_frames_and_nonempty_callbacks_fail_explicitly() {
    let (mut p, mut vm) = fixture(254, 254);
    vm.run_slice(&p, 1, 100).unwrap();
    let mut bad = Snapshot::capture(&p, &vm);
    bad.state
        .threads
        .get_mut(&1)
        .unwrap()
        .dispatch
        .as_mut()
        .unwrap()
        .parent_depth = usize::MAX;
    assert!(bad.restore(&p).is_err());
    vm.run_slice(&p, 1, 1).unwrap();
    for mutation in 0..3 {
        let mut bad = Snapshot::capture(&p, &vm);
        let stack = &mut bad.state.threads.get_mut(&1).unwrap().stack;
        match mutation {
            0 => stack[0].continuation = 0,
            1 => stack[1].action_target = Some(1),
            _ => stack[0].action_target = Some(2),
        }
        assert!(bad.restore(&p).is_err());
    }
    let (_, mut pending) = fixture(254, 254);
    p.modules
        .get_mut("fixture")
        .unwrap()
        .object_functions
        .get_mut(&2)
        .unwrap()[4]
        .action = 4097;
    assert!(pending.cancel_action(&p, 1, 1).unwrap());
    assert_eq!(
        pending.service_empty_action_callbacks(&p).unwrap_err(),
        "nonempty action-cancellation callback is unsupported"
    );
    assert_eq!(pending.world.actions.as_ref().unwrap().callbacks.len(), 1);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn displayed_live_and_queued_labels_match_current_original_and_restore() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-displayed-action-label.json")).unwrap();
    assert_eq!(
        data["source_sha256"],
        "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321"
    );
    assert_eq!(data["cases"].as_array().unwrap().len(), 96);
    for row in data["cases"].as_array().unwrap() {
        let c = &row["input"];
        for slot in [action::DisplaySlot::Active, action::DisplaySlot::Queued(0)] {
            let (mut program, mut vm) = fixture(254, 254);
            let menu = program
                .modules
                .get_mut("fixture")
                .unwrap()
                .object_interactions
                .get_mut(&2)
                .unwrap();
            if c["entry"] == false {
                menu.clear();
            } else {
                menu[0].flags = c["menu_flags"].as_u64().unwrap() as u32;
                menu[0].label = c["menu"].as_str().unwrap().into();
            }
            let mut input = action(50, 80);
            input.flags = c["flags"].as_u64().unwrap() as u32;
            input.label = Some(c["label"].as_str().unwrap().into());
            let actor = vm
                .world
                .actions
                .as_mut()
                .unwrap()
                .actors
                .get_mut(&1)
                .unwrap();
            actor.active = Some(input.clone());
            actor.queue.pending = std::collections::VecDeque::from([input.clone()]);
            let before = vm.clone();
            // Native getter also accepts a missing TTAB entry. That diagnostic
            // input is intentionally not a valid whole-VM saved action binding.
            let mut cold = if c["entry"]==true {restored(&program,&vm)} else {vm.clone()};
            let label = vm.displayed_action_label(&program, 1, slot).unwrap();
            assert_eq!(
                cold.displayed_action_label(&program, 1, slot).unwrap(),
                label
            );
            assert_eq!(cold, vm);
            let state = &vm.world.actions.as_ref().unwrap().actors[&1];
            let changed = match slot {
                action::DisplaySlot::Active => {
                    assert_eq!(
                        state.queue,
                        before.world.actions.as_ref().unwrap().actors[&1].queue
                    );
                    state.active.as_ref().unwrap()
                }
                action::DisplaySlot::Queued(_) => {
                    assert_eq!(
                        state.active,
                        before.world.actions.as_ref().unwrap().actors[&1].active
                    );
                    &state.queue.pending[0]
                }
            };
            assert_eq!(
                serde_json::json!({"label":label,"flags":changed.flags}),
                serde_json::json!({"label":row["expected"]["label"],"flags":row["expected"]["flags"]})
            );
            let before = vm.clone();
            assert!(
                vm.displayed_action_label(&program, 1, action::DisplaySlot::Queued(99))
                    .is_err()
            );
            assert_eq!(before, vm);
        }
    }
}
