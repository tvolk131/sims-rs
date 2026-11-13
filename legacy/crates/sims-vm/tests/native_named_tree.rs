use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
fn routine(name: &str, args: u8, locals: u16, instructions: Vec<Instruction>) -> Routine {
    Routine {
        name: name.into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args,
            locals,
            instructions,
        },
    }
}
fn ins(opcode: u16, operands: [u8; 8], on_true: u8, on_false: u8) -> Instruction {
    Instruction {
        opcode,
        operands,
        on_true,
        on_false,
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_named_dispatch_frames_and_resume_gate() {
    let report: Value = serde_json::from_str(include_str!("fixtures/named-tree.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let flag = c["flags"].as_u64().unwrap_or(0) as u8;
        let present = c["name"].as_bool().unwrap_or(true);
        let found = c["found"].as_bool().unwrap_or(true);
        let person = c["person"].as_bool().unwrap_or(true);
        let target = c["target"].as_i64().unwrap_or(2) as i16;
        let strings = BTreeMap::from([(
            303,
            vec![if present {
                "fixture function".into()
            } else {
                "".into()
            }],
        )]);
        let done = ins(2, [0, 0, 0, 0, 0, 2, 7, 7], 254, 255);
        let caller = Module {
            source: "caller".into(),
            strings: strings.clone(),
            routines: BTreeMap::from([(
                4096,
                routine(
                    "main",
                    4,
                    4,
                    vec![
                        ins(28, [47, 1, flag, 0, 1, 2, 0, 0], 1, 2),
                        done.clone(),
                        done.clone(),
                    ],
                ),
            )]),
            ..Default::default()
        };
        let child = Module {
            source: "target".into(),
            routines: if found {
                BTreeMap::from([(
                    4097,
                    routine(
                        "fixture function",
                        c["args"].as_u64().unwrap_or(4) as u8,
                        2,
                        vec![done],
                    ),
                )])
            } else {
                BTreeMap::new()
            },
            ..Default::default()
        };
        let p = Program {
            globals: "global".into(),
            modules: BTreeMap::from([
                ("caller".into(), caller),
                ("target".into(), child),
                (
                    "global".into(),
                    Module {
                        source: "global".into(),
                        strings,
                        ..Default::default()
                    },
                ),
            ]),
            ..Default::default()
        };
        let mut w = World::default();
        for id in [1, 2] {
            w.objects.insert(
                id,
                Object {
                    definition: Some(Definition {
                        source: if id == 1 { "caller" } else { "target" }.into(),
                        object: 1,
                        guid: 0,
                        slots: 0,
                    }),
                    data: (0..72).map(|i| (i, 0)).collect(),
                    person: (id == 1 && person).then(BTreeMap::new),
                    temps: [11, 22, 33, 44, 2, 66, 77, 88],
                    ..Default::default()
                },
            );
        }
        if person {
            w.actions = Some(action::WorldState {
                actors: BTreeMap::from([(1, Default::default())]),
                ..Default::default()
            });
        }
        let mut vm = Vm::new(w, 1);
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: "caller",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: target,
                    args: vec![1, 2, 3, 4],
                    check: false,
                },
            )
            .unwrap();
        let t = vm.threads.get_mut(&id).unwrap();
        t.stack[0].locals = vec![501, 502, 503, 504];
        t.stack[0].continuation = c["continuation"].as_u64().unwrap_or(0) as u32;
        t.last_result = c["last_result"].as_u64().unwrap_or(0) != 0;
        let run = vm.run_slice(&p, id, 1).unwrap();
        if !n["error"].is_null() {
            assert_eq!(run.stop, Stop::Faulted);
            continue;
        }
        let t = &vm.threads[&id];
        assert_eq!(json!(vm.world.objects[&2].data[&62]), n["use_count"], "{c}");
        if n["returned"] == 3 {
            let frames:Vec<_>=t.stack.iter().map(|f|json!({"bhav":f.location.bhav,"pc":f.location.instruction,"stack_object":f.stack_object,"continuation":f.continuation,"args":f.args,"locals":f.locals,"owner":f.code_owner})).collect();
            assert_eq!(json!(frames), n["frames"], "{c}");
            let restored = Snapshot::capture(&p, &vm).restore(&p).unwrap();
            assert_eq!(restored, vm);
            let done = vm.run_slice(&p, id, 10).unwrap();
            assert_eq!(done.stop, Stop::Returned(true), "{c}: {:?}", done.trace);
            assert_eq!(vm.world.objects[&2].data[&62], 0);
        } else {
            assert_eq!(t.stack[0].continuation, 0);
            assert_eq!(
                t.stack[0].location.instruction,
                if n["returned"] == 1 { 1 } else { 2 }
            );
        }
    }
}
