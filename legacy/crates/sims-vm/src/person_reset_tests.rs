use super::*;
use crate::*;
use serde_json::{Value, json};
use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::ObjectFunction,
};
use std::collections::BTreeMap;

fn fixture(row: &Value) -> (Program, Vm, ThreadId) {
    let mut module = Module {
        source: "test".into(),
        ..Default::default()
    };
    module.object_functions.insert(
        1,
        [0, 4096, 0, 4098]
            .map(|action| ObjectFunction {
                action,
                condition: 0,
            })
            .into(),
    );
    for (id, args, locals) in [(4096, 4, 0), (4097, 2, 2), (4098, 0, 0)] {
        module.routines.insert(
            id,
            Routine {
                name: "supplied Reset test tree".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args,
                    locals,
                    instructions: vec![
                        Instruction {
                            opcode: 2,
                            on_true: 254,
                            on_false: 255,
                            operands: [0, 0, 0, 0, 0, 2, 7, 7],
                        };
                        4
                    ],
                },
            },
        );
    }
    let p = Program {
        globals: "test".into(),
        modules: [("test".into(), module)].into(),
        ..Default::default()
    };
    let b = &row["original"]["before"];
    let c = &row["input"];
    let mut person: BTreeMap<_, _> = (0..256).map(|i| (i, 0)).collect();
    person.extend(
        b["person"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.parse::<i16>().unwrap(), v.as_i64().unwrap() as i16)),
    );
    let mut w = World::default();
    w.objects.insert(
        1,
        Object {
            definition: Some(Definition {
                source: "test".into(),
                object: 1,
                guid: 1,
                slots: 0,
            }),
            person: Some(person),
            data: (0..80).map(|i| (i, 0)).collect(),
            ..Default::default()
        },
    );
    for (id, count) in [(2, 10), (3, 20)] {
        w.objects.insert(
            id,
            Object {
                data: [(62, count)].into(),
                ..Default::default()
            },
        );
    }
    let action = |id, priority, flags| action::Action {
        id,
        target: 2,
        icon_owner: 2,
        routine: 4097,
        priority,
        flags,
        ..Default::default()
    };
    w.actions = Some(action::WorldState {
        actors: [(
            1,
            action::Actor {
                active: c["active"].as_bool().unwrap().then(|| action(71, 100, 12)),
                previous: Some(action(99, 77, 0)),
                queue: action::Queue {
                    pending: (0..c["queued"].as_u64().unwrap())
                        .map(|i| action(i as u32 + 1, 90 + i as i32, 0))
                        .collect(),
                    ..Default::default()
                },
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    });
    let mut vm = Vm::new(w, 0x51a5cafe);
    let id = vm.start_supplied_person_main(&p, 1).unwrap();
    let t = vm.threads.get_mut(&id).unwrap();
    t.last_result = c["previous_result"].as_bool().unwrap();
    for i in 1..b["depth"].as_u64().unwrap() as usize {
        let mut f = t.stack[0].clone();
        f.location.bhav = 4097;
        f.location.instruction = 3;
        f.stack_object = 2;
        f.continuation = 1;
        f.storage_offset = i * 32;
        f.args = vec![11 + i as i16, 21 + i as i16];
        f.locals = vec![31 + i as i16, 41 + i as i16];
        if let Some(u) = b["uses"].as_array().unwrap().iter().find(|u| u[1] == i + 1) {
            f.action_target = Some(u[0].as_i64().unwrap() as i16);
            f.action_icon = u[2] == 1;
        }
        t.stack.push(f);
    }
    (p, vm, id)
}

fn compact(vm: &Vm, t: &Thread) -> Value {
    let a = &vm.world.actions.as_ref().unwrap().actors[&1];
    let p = vm.world.objects[&1].person.as_ref().unwrap();
    json!({"depth":t.stack.len(),
        "uses":t.stack.iter().enumerate().filter_map(|(i,f)| f.action_target.map(|target| json!([target,i+1,u8::from(f.action_icon)]))).collect::<Vec<_>>(),
        "counts":[vm.world.objects[&2].data[&62],vm.world.objects[&3].data[&62]],
        "active":a.active.as_ref().map_or(0,|a|a.id),
        "active_target":a.active.as_ref().map_or(0,|a|a.target),
        "icon":a.icon_owner(),
        "previous":a.previous.as_ref().map_or(0,|a|a.id),
        "previous_flags":a.previous.as_ref().map_or(0,|a|a.flags),
        "last_result":t.last_result,
        "person":([0,30,33,38,39,40,59,71,72,73].into_iter().map(|i|(i.to_string(),p[&i])).collect::<BTreeMap<_,_>>())})
}

fn bound_fixture(row: &Value) -> (Program, Vm, ThreadId) {
    let (mut p, mut vm, thread) = fixture(row);
    // The earlier interpreter-only fixture leaves used-target resources at its
    // host boundary. Supply their identities for full Snapshot validation.
    let module = p.modules.get_mut("test").unwrap();
    for (id, kind) in [(1, 2), (2, 4), (3, 4)] {
        let mut words = vec![0; 108];
        words[9] = kind;
        words[14] = id;
        module.object_definition_words.insert(id as u16, words);
        module.object_interactions.insert(
            id as u16,
            vec![crate::program::MenuEntry {
                action: 4097,
                ..Default::default()
            }],
        );
        vm.world.objects.get_mut(&id).unwrap().definition = Some(Definition {
            source: "test".into(),
            object: id as u16,
            guid: id as u32,
            slots: 0,
        });
    }
    // Give the native probe's synthetic use stack real dispatch metadata and
    // contiguous frame storage, so every suspended Reset can use the ordinary
    // save validator. Reset never executes these interrupted parent calls.
    let t = vm.threads.get_mut(&thread).unwrap();
    let mut offset = 0;
    for (i, frame) in t.stack.iter_mut().enumerate() {
        if i != 0 {
            let routine = module.routines[&4097].clone();
            frame.location.bhav = 4100 + i as u16;
            module.routines.insert(frame.location.bhav, routine);
        }
        frame.location.instruction = 0;
        frame.continuation = 0;
        frame.storage_offset = offset;
        offset += crate::frame_words(frame.args.len(), frame.locals.len());
    }
    for i in 1..t.stack.len() {
        let (parents, children) = t.stack.split_at_mut(i);
        let parent = &mut parents[i - 1];
        let child = &mut children[0];
        let ins = &mut module
            .routines
            .get_mut(&parent.location.bhav)
            .unwrap()
            .body
            .instructions[0];
        ins.operands = [0; 8];
        if let Some(target) = child.action_target {
            parent.stack_object = target;
            child.callee = target;
            ins.opcode = 20;
            ins.operands[2] = u8::from(child.action_icon);
            let functions = module.object_functions.entry(target as u16).or_default();
            functions.resize(
                19,
                ObjectFunction {
                    action: 0,
                    condition: 0,
                },
            );
            functions[18].action = child.location.bhav;
        } else {
            child.callee = parent.callee;
            ins.opcode = child.location.bhav;
        }
    }
    crate::save_frame_storage(t).unwrap();
    (p, vm, thread)
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_active_reset_pops_and_empty_action_order() {
    let report: Value =
        serde_json::from_str(include_str!("../tests/fixtures/active-person-reset.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 192);
    compare_reset_cases(&report, Wait::None);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn bound_reset_flags_match_native_callback_rereads_and_resume() {
    let report: Value =
        serde_json::from_str(include_str!("../tests/fixtures/object-flags.json")).unwrap();
    assert_eq!(report["resets"].as_array().unwrap().len(), 24);
    for row in report["resets"].as_array().unwrap() {
        let (p, mut vm, thread) = bound_fixture(row);
        let c = &row["input"];
        let n = &row["original"];
        vm.world.objects.get_mut(&1).unwrap().native_flags =
            Some(c["flags"].as_u64().unwrap() as u32);
        let mut reset = Reset::bound(&vm, 1, None).unwrap();
        let mut events = vec![];
        let mut applied = false;
        while let Some(request) = reset.next(&mut vm, &p).unwrap() {
            reset.validate(&vm, &p).unwrap();
            let saved = serde_json::to_vec(&(Snapshot::capture(&p, &vm), &reset)).unwrap();
            let (snapshot, resumed): (Snapshot, Reset) = serde_json::from_slice(&saved).unwrap();
            let restored = snapshot.restore(&p).unwrap();
            assert_eq!(restored, vm);
            vm = restored;
            reset = resumed;
            let before = vm.clone();
            let mut guarded = Reset::bound(&vm, 1, None).unwrap();
            assert_eq!(guarded.next(&mut vm, &p).unwrap(), None);
            assert!(guarded.complete());
            assert_eq!(vm, before, "nested Reset did not respect the stored guard");
            let name = match request {
                Request::NotifyUsed(_) => Some("notify_used"),
                Request::ResetUsed(_) => Some("reset_used"),
                Request::ClearAnimator(_) => Some("clear_animator"),
                Request::ReloadAppearance(_) => Some("reload_appearance"),
                Request::ClearRouting(_) => Some("clear_routing"),
                Request::Script(_) => Some("script"),
                _ => None,
            };
            if let Some(name) = name {
                events.push(json!([name, vm.world.objects[&1].native_flags.unwrap()]));
                if !applied && c["mutate_at"] == name {
                    vm.world.objects.get_mut(&1).unwrap().native_flags =
                        Some(c["mutation"].as_u64().unwrap() as u32);
                    applied = true;
                }
            }
            match request {
                Request::Script(thread) => {
                    vm.run_slice(&p, thread, 1).unwrap();
                }
                Request::Placement {
                    marked_for_deletion,
                    ..
                } => {
                    assert_eq!(
                        marked_for_deletion,
                        vm.world.objects[&1].native_flags.unwrap() & 0x40 != 0
                    );
                }
                _ => {}
            }
        }
        let flags = vm.world.objects[&1].native_flags.unwrap();
        events.push(json!(["return", flags]));
        assert_eq!(json!(events), n["flag_events"], "{c}");
        assert_eq!(json!(flags), n["flags_after"], "{c}");
        assert_eq!(json!(applied), n["mutation_applied"], "{c}");
        assert_eq!(
            compact(&vm, &vm.threads[&thread]),
            n["after"],
            "VM processing changed: {c}"
        );
        reset.validate(&vm, &p).unwrap();
        assert!(reset.complete());
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn bound_reset_rejects_lost_flag_storage_and_preserves_legacy_unknown_words() {
    let report: Value =
        serde_json::from_str(include_str!("../tests/fixtures/object-flags.json")).unwrap();
    let (p, mut vm, _) = bound_fixture(&report["resets"][0]);
    assert!(Reset::bound(&vm, 1, None).is_err());
    // Older Rust saves omit the field. Restore that absence instead of guessing
    // constructor values or using the presentation's selected-person input.
    let mut legacy = serde_json::to_value(Snapshot::capture(&p, &vm)).unwrap();
    legacy["state"]["world"]["objects"]["1"]
        .as_object_mut()
        .unwrap()
        .remove("native_flags");
    let restored = serde_json::from_value::<Snapshot>(legacy)
        .unwrap()
        .restore(&p)
        .unwrap();
    assert_eq!(restored, vm);
    vm.world.objects.get_mut(&1).unwrap().native_flags = Some(0x240);
    let mut reset = Reset::bound(&vm, 1, None).unwrap();
    assert!(reset.next(&mut vm, &p).unwrap().is_some());
    let retained = vm.world.objects[&1].native_flags;
    let before = vm.clone();
    let mut guarded = Reset::bound(&vm, 1, Some(1234)).unwrap();
    assert_eq!(guarded.next(&mut vm, &p).unwrap(), None);
    assert_eq!(vm, before, "guarded Reset touched its absent target");
    vm.world.objects.get_mut(&1).unwrap().native_flags = None;
    assert!(reset.validate(&vm, &p).is_err());
    vm.world.objects.get_mut(&1).unwrap().native_flags = Some(retained.unwrap() & !0x800);
    assert!(reset.validate(&vm, &p).is_err());
    vm.world.objects.get_mut(&1).unwrap().native_flags = retained;
    reset.validate(&vm, &p).unwrap();
    assert!(reset.bind_storage(&vm).is_err());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_reset_services_and_guarded_reentry() {
    let report: Value =
        serde_json::from_str(include_str!("../tests/fixtures/person-reset-services.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 528);
    compare_reset_cases(&report, Wait::None);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn interrupted_animation_retains_native_reset_services_and_suspended_frames() {
    let report: Value =
        serde_json::from_str(include_str!("../tests/fixtures/person-reset-services.json")).unwrap();
    compare_reset_cases(&report, Wait::Animation);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn interrupted_route_retains_native_unwind_and_route_destruction_order() {
    let mut report: Value =
        serde_json::from_str(include_str!("../tests/fixtures/person-route-reset.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 12);
    for row in report["cases"].as_array_mut().unwrap() {
        let native = row["original"].clone();
        row["original"] = native["active_reset"].clone();
        row["original"]["events"] = native["events"].clone();
        assert_eq!(native["path_retained"], true);
        for (key, value) in [
            ("extra_frames", json!(2)),
            ("icons", json!(3)),
            ("active", json!(true)),
            ("previous_result", json!(true)),
            ("queued", json!(2)),
        ] {
            row["input"][key] = value;
        }
    }
    compare_reset_cases(&report, Wait::Route);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
    None,
    Animation,
    Route,
}

fn compare_reset_cases(report: &Value, waiting: Wait) {
    for row in report["cases"].as_array().unwrap() {
        let (mut p, mut vm, id) = fixture(row);
        let c = &row["input"];
        let n = &row["original"];
        if waiting != Wait::None {
            if waiting == Wait::Route {
                // The native reset fixture's bare Main has no stack object.
                // Supply an existing route target before entering the wait.
                vm.threads
                    .get_mut(&id)
                    .unwrap()
                    .stack
                    .last_mut()
                    .unwrap()
                    .stack_object = 2;
            }
            let f = vm.threads[&id].stack.last().unwrap();
            p.modules
                .get_mut("test")
                .unwrap()
                .routines
                .get_mut(&f.location.bhav)
                .unwrap()
                .body
                .instructions[f.location.instruction as usize] = Instruction {
                opcode: if waiting == Wait::Animation {
                    0x2c
                } else {
                    0x1b
                },
                on_true: 1,
                on_false: 255,
                operands: [0; 8],
            };
            vm.run_slice(&p, id, 1).unwrap();
            assert!(
                match waiting {
                    Wait::Animation =>
                        matches!(vm.threads[&id].status, Status::AwaitingAnimation(_)),
                    Wait::Route => matches!(vm.threads[&id].status, Status::AwaitingRoute(_)),
                    Wait::None => unreachable!(),
                },
                "route/animation fixture did not suspend: {:?}",
                vm.threads[&id].status
            );
        }
        assert_eq!(compact(&vm, &vm.threads[&id]), n["before"], "{c}");
        let mut popped = vm.clone();
        let mut t = popped.threads.remove(&id).unwrap();
        // Mutated host cases compare at service boundaries below; their later
        // native pops also incorporate the supplied target/count transfer.
        for expected in n["pops"]
            .as_array()
            .unwrap()
            .iter()
            .take(if c["retarget"] == true { 0 } else { usize::MAX })
        {
            let f = t.stack.last().unwrap().clone();
            popped.pop_person_reset_frame(&mut t).unwrap();
            assert_eq!(compact(&popped, &t), *expected, "pop: {c}");
            assert_eq!(
                t.frame_memory[f.storage_offset + 1],
                Some(3),
                "forced pop must preserve suspended PC"
            );
        }
        let mut reset = Reset::new(1, c["flags"].as_u64().unwrap() as u32);
        let mut cancelled = 0;
        let mut services = vec![];
        let mut reentries = vec![];
        let mut order = vec![];
        let mut retargeted = false;
        let mut route_count = c["capacities"].as_array().map_or(0, Vec::len);
        let mut route_events = vec![];
        let mut main_recorded = false;
        while let Some(request) = reset.next(&mut vm, &p).unwrap() {
            reset.validate(&vm, &p).unwrap();
            assert!(reset.accepts_request(&request));
            // Resume a serialized continuation at every exposed host boundary.
            reset = serde_json::from_slice(&serde_json::to_vec(&reset).unwrap()).unwrap();
            let resumed: Vm = serde_json::from_slice(&serde_json::to_vec(&vm).unwrap()).unwrap();
            assert_eq!(resumed, vm);
            vm = resumed;
            let address = match request {
                Request::NotifyUsed(_) => Some("0x444306"),
                Request::ResetUsed(_) => Some("0x431fbe"),
                Request::ClearAnimator(_) => Some("0x589220"),
                Request::ReloadAppearance(_) => Some("0x5888d3"),
                Request::ClearRouting(_) => Some("0x45fad7"),
                Request::Script(_) if !main_recorded => {
                    main_recorded = true;
                    Some("0x47480c")
                }
                _ => None,
            };
            if let Some(address) = address {
                route_events.push(json!([address, route_count]));
            }
            match request {
                Request::InterruptRoute { thread, request } => {
                    let before = vm.clone();
                    assert_eq!(
                        reset.next(&mut vm, &p).unwrap(),
                        Some(Request::InterruptRoute {
                            thread,
                            request: request.clone()
                        })
                    );
                    assert_eq!(before, vm);
                    let mut stale = request.clone();
                    stale.id += 1;
                    assert!(reset.interrupt_route(&mut vm, thread, &stale).is_err());
                    assert_eq!(before, vm);
                    let mut guarded = Reset::new(1, reset.native_flags());
                    assert_eq!(guarded.next(&mut vm, &p).unwrap(), None);
                    assert_eq!(before, vm);
                    reset.interrupt_route(&mut vm, thread, &request).unwrap();
                    let mut expected = before;
                    expected.threads.get_mut(&thread).unwrap().status = Status::Ready;
                    assert_eq!(
                        vm, expected,
                        "route interruption completed or rewrote an instruction"
                    );
                    assert_eq!(reset.retained_route(thread, request.id), Some(&request));
                    assert!(reset.interrupt_route(&mut vm, thread, &request).is_err());
                    assert_eq!(vm, expected);
                    reset.validate(&vm, &p).unwrap();
                }
                Request::InterruptAnimation { thread, request } => {
                    let before = vm.clone();
                    assert_eq!(
                        reset.next(&mut vm, &p).unwrap(),
                        Some(Request::InterruptAnimation {
                            thread,
                            request: request.clone()
                        })
                    );
                    assert_eq!(before, vm, "pending interruption changed VM state");
                    let mut stale = request.clone();
                    stale.id += 1;
                    assert!(reset.interrupt_animation(&mut vm, thread, &stale).is_err());
                    assert_eq!(before, vm);
                    let mut guarded = Reset::new(1, reset.native_flags());
                    assert_eq!(guarded.next(&mut vm, &p).unwrap(), None);
                    assert_eq!(before, vm, "nested guard changed animation wait");
                    reset
                        .interrupt_animation(&mut vm, thread, &request)
                        .unwrap();
                    let mut expected = before;
                    expected.threads.get_mut(&thread).unwrap().status = Status::Ready;
                    assert_eq!(
                        vm, expected,
                        "interruption completed an instruction or changed VM storage"
                    );
                    assert!(
                        reset
                            .interrupt_animation(&mut vm, thread, &request)
                            .is_err()
                    );
                    assert_eq!(vm, expected);
                    reset.validate(&vm, &p).unwrap();
                }
                Request::NotifyUsed(target) | Request::ResetUsed(target) => {
                    let notify = matches!(request, Request::NotifyUsed(_));
                    services.push(json!({"kind":if notify {"notify"} else {"reset"},
                        "target":target,"flags":reset.native_flags(),
                        "state":compact(&vm,&vm.threads[&id])}));
                    order.push(json!([
                        if notify { "notify_used" } else { "reset_used" },
                        target
                    ]));
                    assert!(!reset.accepts_request(&Request::NotifyUsed(99)));
                    assert!(!reset.accepts_request(&Request::ResetUsed(99)));
                    if notify {
                        // Native 444306 reenters the resetting person. Its live
                        // guard prevents stack/action/queue changes in that call.
                        reentries.push(json!({"target":target,"flags":reset.native_flags(),
                            "state":compact(&vm,&vm.threads[&id])}));
                        let before = vm.clone();
                        let mut nested = Reset::new(1, reset.native_flags());
                        nested.validate(&vm, &p).unwrap();
                        assert_eq!(nested.next(&mut vm, &p).unwrap(), None);
                        assert!(nested.complete());
                        assert_eq!(vm, before);
                        assert_eq!(nested.native_flags(), reset.native_flags());
                    } else if c["retarget"] == true && !retargeted {
                        let f = vm
                            .threads
                            .get_mut(&id)
                            .unwrap()
                            .stack
                            .iter_mut()
                            .rev()
                            .find(|f| f.action_target.is_some())
                            .unwrap();
                        assert_eq!(f.action_target, Some(2));
                        f.action_target = Some(3);
                        *vm.world
                            .objects
                            .get_mut(&2)
                            .unwrap()
                            .data
                            .get_mut(&62)
                            .unwrap() -= 1;
                        *vm.world
                            .objects
                            .get_mut(&3)
                            .unwrap()
                            .data
                            .get_mut(&62)
                            .unwrap() += 1;
                        retargeted = true;
                    }
                }
                Request::ClearAnimator(_) => {
                    assert_eq!(compact(&vm, &vm.threads[&id]), n["before_animator"], "{c}");
                }
                Request::Script(t) => {
                    vm.run_slice(&p, t, 1).unwrap();
                }
                Request::Cancel(_) => cancelled += 1,
                Request::ClearRouting(_) => route_count = 0,
                Request::Footprint(_) => panic!("unexpected footprint"),
                _ => (),
            }
        }
        let guarded = c["flags"].as_u64().unwrap() & 0x800 != 0;
        assert_eq!(
            cancelled,
            if guarded {
                0
            } else {
                c["queued"].as_u64().unwrap()
            }
        );
        let expected_order: Vec<_> = n["order"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e[0] == "notify_used" || e[0] == "reset_used")
            .cloned()
            .collect();
        assert_eq!(order, expected_order, "{c}");
        if !n["services"].is_null() {
            assert_eq!(json!(services), n["services"], "{c}");
            assert_eq!(json!(reentries), n["reentries"], "{c}");
        }
        assert_eq!(compact(&vm, &vm.threads[&id]), n["after"], "after: {c}");
        assert_eq!(reset.native_flags(), c["flags"].as_u64().unwrap() as u32);
        reset.validate(&vm, &p).unwrap();
        assert!(reset.complete());
        assert_eq!(retargeted, c["retarget"] == true);
        if !n["events"].is_null() {
            assert_eq!(json!(route_events), n["events"], "{c}");
        }
    }
}

fn notification_state(vm: &Vm, thread: ThreadId, reset: &Reset) -> Value {
    let actor = &vm.world.actions.as_ref().unwrap().actors[&1];
    json!({"state":compact(vm, &vm.threads[&thread]),
        "queue":actor.queue.pending.iter().map(|a| [i64::from(a.id),i64::from(a.target),i64::from(a.icon_owner)]).collect::<Vec<_>>(),
        "previous_target":actor.previous.as_ref().map_or(0, |a|a.target),
        "previous_icon":actor.previous.as_ref().map_or(0, |a|a.icon_owner),
        "flags":reset.native_flags()})
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_notifications_preserve_selection_target_flags_and_callback_queue_states() {
    let report: Value = serde_json::from_str(include_str!(
        "../tests/fixtures/person-reset-notification.json"
    ))
    .unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 1312);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let (p, mut vm, thread) = fixture(row);
        let actor = vm
            .world
            .actions
            .as_mut()
            .unwrap()
            .actors
            .get_mut(&1)
            .unwrap();
        actor.set_icon_owner(c["icon"].as_i64().unwrap() as i16);
        actor.previous.as_mut().unwrap().target = c["previous_target"].as_i64().unwrap() as i16;
        actor.previous.as_mut().unwrap().icon_owner = c["previous_icon"].as_i64().unwrap() as i16;
        for (a, entry) in actor
            .queue
            .pending
            .iter_mut()
            .zip(c["queue"].as_array().unwrap())
        {
            a.target = entry[0].as_i64().unwrap() as i16;
            a.icon_owner = entry[1].as_i64().unwrap() as i16;
        }
        let mut reset = Reset::notified(
            1,
            c["flags"].as_u64().unwrap() as u32,
            c["notified"].as_i64().unwrap() as i16,
            c["target_flags"].as_u64().unwrap() as u32,
        );
        assert_eq!(
            notification_state(&vm, thread, &reset),
            n["before_notification"],
            "before {c}"
        );
        let mut cancellations = vec![];
        let mut services = vec![];
        let mut animator = false;
        let mut injected = false;
        let before_threads = vm.threads.clone();
        while let Some(request) = reset.next(&mut vm, &p).unwrap() {
            reset.validate(&vm, &p).unwrap();
            assert!(reset.accepts_request(&request));
            reset = serde_json::from_slice(&serde_json::to_vec(&reset).unwrap()).unwrap();
            let resumed: Vm = serde_json::from_slice(&serde_json::to_vec(&vm).unwrap()).unwrap();
            assert_eq!(resumed, vm);
            vm = resumed;
            match request {
                Request::NotifyUsed(target) | Request::ResetUsed(target) => {
                    let notify = matches!(request, Request::NotifyUsed(_));
                    services.push(
                        json!({"kind":if notify {"notify"} else {"reset"},"target":target,
                        "flags":reset.native_flags(),"state":compact(&vm,&vm.threads[&thread])}),
                    );
                    if notify {
                        let before = vm.clone();
                        let mut nested = Reset::notified(1, reset.native_flags(), target, 0);
                        assert_eq!(nested.next(&mut vm, &p).unwrap(), None);
                        nested.validate(&vm, &p).unwrap();
                        assert_eq!(vm, before);
                    }
                }
                Request::ClearAnimator(_) => {
                    animator = true;
                    assert_eq!(
                        compact(&vm, &vm.threads[&thread]),
                        n["before_animator"],
                        "{c}"
                    );
                }
                Request::Script(id) => {
                    vm.run_slice(&p, id, 1).unwrap();
                }
                Request::Cancel(callback) => {
                    assert_eq!(callback.function, 4);
                    assert_eq!(callback.stack_object, 1);
                    cancellations.push(json!({"target":callback.object,"snapshot":notification_state(&vm,thread,&reset)}));
                    if c["append_during_cancel"] == true && !injected {
                        vm.world
                            .actions
                            .as_mut()
                            .unwrap()
                            .actors
                            .get_mut(&1)
                            .unwrap()
                            .queue
                            .pending
                            .push_back(action::Action {
                                id: 777,
                                target: 3,
                                icon_owner: 2,
                                routine: 4097,
                                priority: 123,
                                ..Default::default()
                            });
                        injected = true;
                    }
                }
                Request::Footprint(_) | Request::ClearRouteReferences { .. } => {
                    panic!("unexpected notification service {request:?}")
                }
                _ => (),
            }
        }
        assert_eq!(animator, !n["before_animator"].is_null(), "selection {c}");
        if !animator {
            assert_eq!(vm.threads, before_threads, "unrelated/guarded frames {c}");
        }
        assert_eq!(json!(services), n["services"], "services {c}");
        assert_eq!(
            json!(cancellations),
            n["cancellations"],
            "cancellations {c}"
        );
        assert_eq!(
            notification_state(&vm, thread, &reset),
            n["after_notification"],
            "after {c}"
        );
        assert!(reset.complete());
        reset.validate(&vm, &p).unwrap();
        assert_eq!(injected, c["append_during_cancel"] == true);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn unrelated_notification_leaves_animation_instruction_suspended() {
    let report: Value = serde_json::from_str(include_str!(
        "../tests/fixtures/person-reset-notification.json"
    ))
    .unwrap();
    let row = report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| {
            let c = &row["input"];
            c["flags"] == 0
                && c["uses"] == 0
                && c["active"] == false
                && c["icon"] == 0
                && c["notified"] == 3
                && c["queued"] == 0
        })
        .unwrap();
    let (mut p, mut vm, thread) = fixture(row);
    p.modules
        .get_mut("test")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .instructions[0] = Instruction {
        opcode: 0x2c,
        on_true: 1,
        on_false: 255,
        operands: [0; 8],
    };
    vm.run_slice(&p, thread, 1).unwrap();
    assert!(matches!(
        vm.threads[&thread].status,
        Status::AwaitingAnimation(_)
    ));
    let before = vm.threads[&thread].clone();
    let mut reset = Reset::notified(1, 0, 3, 0);
    assert_eq!(reset.next(&mut vm, &p).unwrap(), None);
    reset.validate(&vm, &p).unwrap();
    assert!(reset.complete());
    assert_eq!(vm.threads[&thread], before);
    assert_eq!(vm.world.objects[&1].person.as_ref().unwrap()[&30], 130);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_broadcast_uses_live_person_registry_order() {
    let report: Value =
        serde_json::from_str(include_str!("../tests/fixtures/reset-broadcast.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 160);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut world = World::default();
        world.person = Some(person::WorldState {
            person_order: serde_json::from_value(c["people"].clone()).unwrap(),
            ..Default::default()
        });
        for id in 1..=4 {
            world.objects.insert(
                id,
                Object {
                    person: Some(Default::default()),
                    ..Default::default()
                },
            );
        }
        let mut broadcast = Broadcast::new(c["target"].as_i64().unwrap() as i16);
        let mut calls = vec![];
        while let Some(person) = broadcast.next(&world).unwrap() {
            let order = &mut world.person.as_mut().unwrap().person_order;
            calls.push(json!({"person":person,"people":order}));
            if calls.len() == 1 {
                let index = order.iter().position(|id| *id == person).unwrap();
                match c["mutation"].as_str().unwrap() {
                    "append" => order.push(4),
                    "remove-next" if index + 1 < order.len() => {
                        order.remove(index + 1);
                    }
                    "remove-current" if index + 1 < order.len() => {
                        order.remove(index);
                    }
                    "reverse-rest" => order[index + 1..].reverse(),
                    _ => (),
                }
            }
            broadcast = serde_json::from_slice(&serde_json::to_vec(&broadcast).unwrap()).unwrap();
        }
        assert_eq!(
            json!({"calls":calls,"people":world.person.as_ref().unwrap().person_order}),
            row["original"],
            "{c}"
        );
        assert!(broadcast.complete());
        world.person.as_mut().unwrap().person_order.push(4);
        assert_eq!(broadcast.next(&world).unwrap(), None);
    }
}
