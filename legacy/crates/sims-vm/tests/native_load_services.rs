//! Synthetic inputs executed by unmodified original routines; no game bytes.
use serde_json::{Value, json};
use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::ObjectFunction,
};
use sims_vm::*;
use std::collections::BTreeMap;

fn routine(args: u8, opcode: u16, operands: [u8; 8]) -> Routine {
    Routine {
        name: "synthetic".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args,
            locals: 0,
            instructions: vec![Instruction {
                opcode,
                on_true: 254,
                on_false: 255,
                operands,
            }],
        },
    }
}
fn program(callback: u16) -> Program {
    Program {
        globals: "test".into(),
        modules: BTreeMap::from([(
            "test".into(),
            Module {
                source: "test".into(),
                routines: BTreeMap::from([
                    (4096, routine(4, 2, [0, 0, 0, 0, 0, 5, 0, 7])),
                    (7901, routine(4, 2, [0, 0, 0, 0, 0, 5, 0, 9])),
                ]),
                object_functions: BTreeMap::from([(
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
                        ObjectFunction {
                            action: callback,
                            condition: 0,
                        },
                    ],
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    }
}
fn vm(p: &Program, previous: bool) -> (Vm, ThreadId) {
    let mut w = World::default();
    w.objects.insert(
        1,
        Object {
            definition: Some(Definition {
                source: "test".into(),
                object: 1,
                guid: 7,
                slots: 0,
            }),
            attributes: vec![100],
            temps: [-123, 2, 3, 4, 5, 6, 7, 8],
            ..Default::default()
        },
    );
    let mut vm = Vm::new(w, 1);
    let id = vm
        .restore_object_stack(
            p,
            1,
            &[loading::SavedFrame {
                owner: "test".into(),
                tree: 4096,
                node: 0,
                stack_object: 0,
                args: vec![0; 4],
                locals: vec![],
                primitive_state: 0,
            }],
            128,
            false,
        )
        .unwrap();
    let t = vm.threads.get_mut(&id).unwrap();
    t.last_result = previous;
    t.frame_memory.resize(64, Some(0));
    t.frame_memory[12..32].fill(Some(0));
    t.frame_memory[32..].fill(Some(0x5a5a));
    (vm, id)
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_loaded_callback_missing_tree_and_success() {
    let r: Value = serde_json::from_str(include_str!("fixtures/load-services.json")).unwrap();
    for row in r["callbacks"].as_array().unwrap() {
        let c = &row["input"];
        let expected = &row["original"];
        let bhav = c["bhav"].as_u64().unwrap() as u16;
        let p = program(bhav);
        let (mut v, id) = vm(&p, c["previous"].as_bool().unwrap());
        let before = v.threads[&id].stack.clone();
        let started = v.begin_loaded_function(&p, 1, 2).unwrap();
        if let Some(thread) = started {
            v.run_slice(&p, thread, 10).unwrap();
        }
        let t = &v.threads[&id];
        assert_eq!(t.stack, before, "ordinary Main must not advance: {c}");
        assert_eq!(
            t.last_result,
            expected["last_result"].as_bool().unwrap(),
            "{c}"
        );
        assert_eq!(
            started.is_some() && t.callback_result == Some(true),
            expected["returned"].as_bool().unwrap(),
            "{c}"
        );
        assert_eq!(
            v.world.objects[&1].attributes[0],
            expected["attribute0"].as_i64().unwrap() as i16,
            "{c}"
        );
        assert_eq!(t.status, Status::Ready);
        assert!(t.callback.is_none());
        for (i, word) in expected["arena_words"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            // Original resource pointers have no portable numeric value.
            if [6, 7, 26, 27].contains(&i) {
                continue;
            }
            assert_eq!(
                t.frame_memory[i],
                Some(word.as_i64().unwrap() as i16),
                "{c}, word {i}"
            );
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_collision_callback_stack_object_null_args_and_resume() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/collision-callbacks.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let expected = &row["original"];
        let mut p = program(0);
        let m = p.modules.get_mut("test").unwrap();
        m.object_functions.get_mut(&1).unwrap().resize(
            6,
            ObjectFunction {
                action: 0,
                condition: 0,
            },
        );
        m.object_functions.get_mut(&1).unwrap()[5].action = c["bhav"].as_u64().unwrap() as u16;
        m.routines.get_mut(&7901).unwrap().body.instructions = vec![
            Instruction {
                opcode: 2,
                on_true: 1,
                on_false: 253,
                operands: [0, 0, 0, 0, 0, 5, 0, 10],
            },
            Instruction {
                opcode: 2,
                on_true: if c["returned"] == true { 254 } else { 255 },
                on_false: 253,
                operands: [0, 0, 0, 0, 0, 5, 8, 9],
            },
        ];
        let (mut v, id) = vm(&p, c["previous"].as_bool().unwrap());
        let before = v.threads[&id].stack.clone();
        let started = v
            .begin_object_function(&p, 1, 5, c["other"].as_i64().unwrap() as i16)
            .unwrap();
        if let Some(thread) = started {
            let step = v.run_slice(&p, thread, 1).unwrap();
            assert_eq!(step.trace.len(), 1);
            let mut resumed = Snapshot::capture(&p, &v).restore(&p).unwrap();
            assert_eq!(v.run_slice(&p, thread, 1), resumed.run_slice(&p, thread, 1));
            assert_eq!(v, resumed);
        }
        let t = &v.threads[&id];
        assert_eq!(t.stack, before, "ordinary Main changed: {c}");
        assert_eq!(t.status, Status::Ready);
        assert!(t.callback.is_none());
        assert_eq!(json!(t.last_result), expected["last_result"], "{c}");
        assert_eq!(
            json!(started.is_some() && t.callback_result == Some(true)),
            expected["returned"],
            "{c}"
        );
        assert_eq!(
            json!(v.world.objects[&1].attributes[0]),
            expected["attribute0"],
            "{c}"
        );
        assert_eq!(
            json!(v.world.objects[&1].temps[0]),
            expected["temp0"],
            "{c}"
        );
        for (i, word) in expected["arena_words"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            if [6, 7, 26, 27].contains(&i) {
                continue;
            } // Resource pointers.
            assert_eq!(
                t.frame_memory[i],
                Some(word.as_i64().unwrap() as i16),
                "{c}, arena word {i}"
            );
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_ui_event_ownership() {
    let report: Value = serde_json::from_str(include_str!("fixtures/load-services.json")).unwrap();
    for row in report["ownership"].as_array().unwrap() {
        let c = &row["input"];
        let r = &row["original"];
        let owner = c["owner"].as_i64().unwrap() as i16;
        let mut state = ui_owner::State {
            owner: (owner != 0).then_some(owner),
            dialog_owners: serde_json::from_value(c["dialogs"].clone()).unwrap(),
            effects: vec![],
        };
        let mut p = program(0);
        let mut operands = [0; 8];
        operands[0] = c["mode"].as_u64().unwrap() as u8;
        for (i, b) in c["tail"].as_array().unwrap().iter().enumerate() {
            operands[i + 1] = b.as_u64().unwrap() as u8;
        }
        p.modules
            .get_mut("test")
            .unwrap()
            .routines
            .insert(4096, routine(4, 10, operands));
        let (mut v, id) = vm(&p, true);
        v.world.ui_owner = state.clone();
        v.world.objects.insert(2, Object::default());
        v.run_slice(&p, id, 1).unwrap();
        if r["error"].is_null() {
            assert_eq!(
                v.threads[&id].status,
                Status::Returned(r["returned"].as_bool().unwrap()),
                "{c}"
            );
        } else {
            assert!(
                matches!(&v.threads[&id].status,Status::Faulted(f) if f.message.contains("0x35")),
                "{c}"
            );
        }
        state = v.world.ui_owner.clone();
        let effects: Vec<_> = state
            .effects
            .iter()
            .map(|e| match e {
                ui_owner::Effect::Changed => json!([0x107, 0]),
                ui_owner::Effect::CloseDialog(i) => json!(["close", i]),
            })
            .collect();
        assert_eq!(json!(state.owner.unwrap_or(0)), r["owner"], "{c}");
        assert_eq!(json!(effects), r["events"], "{c}");
        if r["error"].is_null() {
            let saved = Snapshot::capture(&p, &v);
            let restored = saved.restore(&p).unwrap();
            assert_eq!(restored.world.ui_owner, state);
        }
    }
}
#[test]
fn missing_script_call_still_faults_and_incomplete_link_is_not_absence() {
    let mut p = program(7900);
    p.modules
        .get_mut("test")
        .unwrap()
        .routines
        .insert(4096, routine(4, 7900, [0; 8]));
    let (mut v, id) = vm(&p, false);
    v.run_slice(&p, id, 1).unwrap();
    assert!(
        matches!(&v.threads[&id].status,Status::Faulted(f) if f.message.contains("missing BHAV"))
    );
    let (mut v, _) = vm(&p, false);
    p.modules.get_mut("test").unwrap().semiglobal = Some("missing-link".into());
    p.modules
        .get_mut("test")
        .unwrap()
        .object_functions
        .get_mut(&1)
        .unwrap()[2]
        .action = 8192;
    assert!(
        v.begin_loaded_function(&p, 1, 2)
            .unwrap_err()
            .contains("missing module")
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_base_virtual_reset_callback_branch() {
    let report: Value = serde_json::from_str(include_str!("fixtures/load-services.json")).unwrap();
    for row in report["resets"].as_array().unwrap() {
        let c = &row["input"];
        let r = &row["original"];
        let bhav = c["bhav"].as_u64().unwrap() as u16;
        for incoming in [
            Status::Ready,
            Status::Sleeping { wake_tick: 500 },
            Status::Returned(true),
        ] {
            let mut p = program(0);
            let m = p.modules.get_mut("test").unwrap();
            m.object_functions
                .get_mut(&1)
                .unwrap()
                .push(ObjectFunction {
                    action: bhav,
                    condition: 0,
                });
            let mut definition = vec![0; 108];
            definition[9] = 4;
            m.object_definition_words.insert(1, definition);
            let mut reset = routine(4, 2, [0, 0, 0, 0, 0, 5, 0, 9]);
            reset.body.instructions[0].on_true = 1;
            reset.body.instructions.extend([
                Instruction {
                    opcode: 2,
                    on_true: 2,
                    on_false: 253,
                    operands: [62, 0, 65, 1, 0, 5, 3, 7],
                },
                Instruction {
                    opcode: 2,
                    on_true: if c["returned"] == true { 254 } else { 255 },
                    on_false: 253,
                    operands: [8, 0, 255, 255, 0, 5, 3, 7],
                },
            ]);
            m.routines.insert(7901, reset);
            let (mut v, id) = vm(&p, c["previous"].as_bool().unwrap());
            v.world
                .objects
                .get_mut(&1)
                .unwrap()
                .data
                .extend([(8, c["flags"].as_u64().unwrap() as i16), (62, 987)]);
            v.world.objects.get_mut(&1).unwrap().idle = Some(idle::IdleState {
                countdown: 51,
                ..Default::default()
            });
            v.world.objects.insert(2, Object::default());
            let owner = c["owner"].as_i64().unwrap() as i16;
            v.world.ui_owner.owner = (owner != 0).then_some(owner);
            let main_before = v.threads[&id].stack[0].clone();
            v.threads.get_mut(&id).unwrap().status = incoming;
            let reset = v.begin_loaded_base_callback_reset(&p, 1).unwrap();
            assert_eq!(reset.thread(), id);
            let mut trace = vec![];
            // Small budgets ensure a callback remains resumable without running Main.
            while v.threads[&id].callback.is_some() {
                let run = v.run_slice(&p, id, 1).unwrap();
                trace.extend(run.trace);
                assert!(!matches!(v.threads[&id].status, Status::Faulted(_)), "{c}");
                assert_eq!(v.threads[&id].stack[0], main_before);
            }
            reset.finish(&mut v, &p).unwrap();
            let t = &v.threads[&id];
            let o = &v.world.objects[&1];
            let state = json!({"last_result":t.last_result,"data8":o.data[&8],"data62":o.data[&62],"attribute0":o.attributes[0],
            "schedule":o.idle.as_ref().unwrap().countdown,"owner":v.world.ui_owner.owner.unwrap_or(0)});
            for key in [
                "last_result",
                "data8",
                "data62",
                "attribute0",
                "schedule",
                "owner",
            ] {
                assert_eq!(state[key], r[key], "{c}: {key}");
            }
            let steps: Vec<_> = trace
                .iter()
                .map(|t| json!([t.location.bhav, t.location.instruction]))
                .collect();
            // Rust folds native terminal-PC return steps into the preceding instruction.
            let native_steps: Vec<_> = r["steps"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| s["bhav"] != 65535 && s["pc"].as_u64().unwrap() <= 250)
                .map(|s| json!([s["bhav"], s["pc"]]))
                .collect();
            assert_eq!(steps, native_steps, "{c}");
            assert_eq!(t.stack.len(), 1);
            assert_eq!(t.stack[0], main_before);
            let events: Vec<_> = v
                .world
                .ui_owner
                .effects
                .iter()
                .map(|e| {
                    assert_eq!(*e, ui_owner::Effect::Changed);
                    json!([0x107, 0])
                })
                .collect();
            assert_eq!(json!(events), r["events"], "{c}");
        }
    }
}
