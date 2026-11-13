//! Compare shared action semantics with private original-code probe reports.
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{action::*, program::MenuEntry, *};
use std::{collections::BTreeMap, fs};

fn number(v: &Value, name: &str, default: i64) -> i64 {
    v[name].as_i64().unwrap_or(default)
}
fn action(v: &Value, id: u32) -> action::Action {
    action::Action {
        id,
        target: if v["target"] == false { 0 } else { 2 },
        icon_owner: 2,
        routine: 4097,
        priority: number(v, "priority", 0) as i32,
        flags: number(v, "flags", 0) as u32,
        ..Default::default()
    }
}
fn queue(c: &Value) -> Queue {
    Queue {
        pending: c["queue"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, a)| action(a, (i + 1) as u32))
            .collect(),
    }
}
fn rows(q: &Queue) -> Value {
    json!(
        q.pending
            .iter()
            .map(|a| json!({"priority":a.priority,"flags":a.flags,"serial":a.id}))
            .collect::<Vec<_>>()
    )
}
fn callbacks(removed: &[action::Action]) -> Value {
    json!(
        removed
            .iter()
            .filter(|a| a.target != 0)
            .map(|a| json!({"object":a.target,"function":4,"stack":1}))
            .collect::<Vec<_>>()
    )
}
fn fixture(c: &Value, opcode: u16) -> (Program, Vm, ThreadId) {
    let mut operands = c["operands"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        })
        .unwrap_or(vec![0; 8]);
    if opcode == 17 {
        operands[..2].copy_from_slice(&(number(c, "index", 0) as i16).to_le_bytes());
        operands[2..4].copy_from_slice(&(number(c, "dispatch", 0) as i16).to_le_bytes());
    }
    let mut module = Module {
        source: "fixture".into(),
        ..Default::default()
    };
    module.routines.insert(
        4096,
        Routine {
            name: "action probe".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 4,
                locals: 4,
                instructions: vec![Instruction {
                    opcode,
                    on_true: 0,
                    on_false: 0,
                    operands: operands.try_into().unwrap(),
                }],
            },
        },
    );
    module.routines.insert(
        4097,
        Routine {
            name: "action child".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 4,
                locals: 0,
                instructions: vec![Instruction {
                    opcode: 0,
                    on_true: 254,
                    on_false: 255,
                    operands: [0; 8],
                }],
            },
        },
    );
    if c["check_return"].is_number() {
        let expr = |scope: u8, index: i16, value: i16, yes: u8| {
            let mut operands = [0; 8];
            operands[..2].copy_from_slice(&index.to_le_bytes());
            operands[2..4].copy_from_slice(&value.to_le_bytes());
            operands[5..].copy_from_slice(&[5, scope, 7]);
            Instruction {
                opcode: 2,
                on_true: yes,
                on_false: 253,
                operands,
            }
        };
        module.routines.insert(
            4098,
            Routine {
                name: "eligibility probe".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 4,
                    locals: 0,
                    instructions: vec![
                        expr(8, 0, 99, 1),
                        expr(0, 0, 321, c["check_return"].as_u64().unwrap() as u8),
                    ],
                },
            },
        );
    }
    module.object_interactions.insert(
        2,
        vec![MenuEntry {
            autonomy: Default::default(),
            id: 0,
            flags: number(c, "ttab_flags", 0) as u32,
            label: "action".into(),
            action: 4097,
            check: if c["check_return"].is_number() {
                4098
            } else {
                0
            },
        }],
    );
    module.object_interactions.insert(
        1,
        if c["self_menu"] == true {
            vec![MenuEntry {
                autonomy: Default::default(),
                id: 33,
                flags: 0,
                label: "cancel fallback".into(),
                action: 4097,
                check: 0,
            }]
        } else {
            vec![]
        },
    );
    let program = Program {
        globals: "fixture".into(),
        modules: BTreeMap::from([("fixture".into(), module)]),
        ..Default::default()
    };
    let mut person: BTreeMap<i16, i16> = (0..256).map(|i| (i, 0)).collect();
    for (index, name) in [
        (33, "current_priority"),
        (71, "locked"),
        (32, "autonomy"),
        (65, "species"),
        (58, "age"),
    ] {
        person.insert(index, number(c, name, 0) as i16);
    }
    let mut motives = [0u32; 16];
    if let Some(values) = c["motives"].as_object() {
        for (index, value) in values {
            motives[index.parse::<usize>().unwrap()] = (value.as_f64().unwrap() as f32).to_bits();
        }
    }
    let objects = BTreeMap::from([
        (
            1,
            Object {
                person: Some(person),
                motives: Some(motives),
                definition: Some(Definition {
                    source: "fixture".into(),
                    object: 1,
                    guid: 0,
                    slots: 0,
                }),
                temps: [1, 2, 3, 0, 7, -1, 8, 255],
                attributes: vec![100, 101, 102, 103],
                data: BTreeMap::from([(8, number(c, "flags", 0) as i16)]),
                ..Default::default()
            },
        ),
        (
            2,
            Object {
                definition: Some(Definition {
                    source: "fixture".into(),
                    object: 2,
                    guid: 0,
                    slots: 0,
                }),
                data: BTreeMap::from([
                    (8, number(c, "target_flags", 28) as i16),
                    (59, 79),
                    (62, 82),
                ]),
                ..Default::default()
            },
        ),
    ]);
    let world = World {
        objects,
        actions: Some(WorldState {
            serial: 100,
            live_family: c["live_family"] == true,
            actors: BTreeMap::from([(
                1,
                Actor {
                    queue: queue(c),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut vm = Vm::new(world, 1);
    if c["active"] == true {
        vm.world
            .actions
            .as_mut()
            .unwrap()
            .actors
            .get_mut(&1)
            .unwrap()
            .active = Some(action(
            &json!({"priority":number(c,"current_priority",100)}),
            99,
        ));
    }
    let args = c["args"]
        .as_array()
        .map(|a| a.iter().map(|v| v.as_i64().unwrap() as i16).collect())
        .unwrap_or(vec![
            if opcode == 17 {
                number(c, "counter", 0) as i16
            } else {
                2
            },
            0,
            0,
            0,
        ]);
    let id = vm
        .spawn(
            &program,
            Start {
                owner: "fixture",
                bhav: 4096,
                caller: 1,
                callee: 2,
                stack_object: number(c, "stack", 1) as i16,
                args,
                check: false,
            },
        )
        .unwrap();
    let locals = c["locals"]
        .as_array()
        .map(|a| a.iter().map(|v| v.as_i64().unwrap() as i16).collect())
        .unwrap_or(vec![2, 0, 0, 0]);
    vm.threads.get_mut(&id).unwrap().stack[0].locals = locals;
    vm.threads.get_mut(&id).unwrap().stack[0].continuation = number(c, "continuation", 0) as u32;
    // The native fixture supplies zero backing words after this frame. Explicitly
    // mirror those known words for the original operand's one-past-end read.
    vm.threads
        .get_mut(&id)
        .unwrap()
        .frame_memory
        .resize(32, Some(0));
    (program, vm, id)
}
fn error(vm: &Vm, id: ThreadId) -> Value {
    match &vm.threads[&id].status {
        Status::Faulted(f) => {
            let code = f
                .message
                .strip_prefix("native error 0x")
                .and_then(|s| s.split(':').next())
                .and_then(|s| u32::from_str_radix(s, 16).ok());
            assert!(code.is_some(), "unexpected fault: {}", f.message);
            json!(code.unwrap())
        }
        _ => Value::Null,
    }
}
fn main() {
    let path = std::env::args().nth(1).expect("private actions.json path");
    let report: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let mut count = 0;
    for row in report["insert"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        let mut q = queue(c);
        let out = q.insert(action(&c["incoming"], 101)).unwrap();
        assert_eq!(rows(&q), e["queue"], "insert {c}");
        assert_eq!(
            callbacks(&out.cancelled),
            e["callbacks"],
            "insert callbacks {c}"
        );
        assert_eq!(json!(out.accepted), e["returned"], "insert outcome {c}");
        count += 1;
    }
    for row in report["cancel"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        let (p, mut vm, _) = fixture(c, 0);
        let result = vm
            .cancel_action(&p, 1, c["serial"].as_u64().unwrap() as u32)
            .unwrap();
        let world = vm.world.actions.as_ref().unwrap();
        let state = &world.actors[&1];
        assert_eq!(rows(&state.queue), e["queue"], "cancel {c}");
        assert_eq!(json!(world.callbacks.iter().map(|cb|json!({"object":cb.object,"function":cb.function,"stack":cb.stack_object})).collect::<Vec<_>>()),e["callbacks"],"cancel callbacks {c}");
        assert_eq!(json!(result), e["returned"], "cancel outcome {c}");
        assert_eq!(
            json!(vm.world.objects[&1].person.as_ref().unwrap()[&33]),
            e["priority"],
            "cancel priority {c}"
        );
        assert_eq!(
            json!(state.active.as_ref().map_or(0, |a| a.flags)),
            e["active_flags"],
            "cancel flags {c}"
        );
        assert_eq!(json!(world.serial), e["next_serial"], "cancel serial {c}");
        assert_eq!(vm, Snapshot::capture(&p, &vm).restore(&p).unwrap());
        count += 1;
    }
    let mut dispatched = 0;
    for row in report["idle"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        let (p, mut vm, id) = fixture(c, 17);
        vm.run_slice(&p, id, 1).unwrap();
        for _ in 0..4 {
            if vm.threads[&id].dispatch.is_none() {
                break;
            }
            vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
            vm.run_slice(&p, id, 1).unwrap();
        }
        assert!(
            vm.threads[&id].dispatch.is_none(),
            "unfinished dispatch {c}"
        );
        assert_eq!(error(&vm, id), e["error"], "idle error {c}");
        let t = &vm.threads[&id];
        let state = vm.world.actions.as_ref().unwrap();
        assert_eq!(json!(t.stack[0].args[0]), e["counter"], "idle counter {c}");
        assert_eq!(
            json!(vm.world.objects[&1].data[&8] as u16),
            e["flags"],
            "idle flags {c}"
        );
        if e["error"].is_null() {
            let result = if matches!(t.status, Status::Sleeping { .. }) {
                2
            } else if t.stack.len() == 2 {
                3
            } else {
                1
            };
            if result == 3 {
                dispatched += 1;
            }
            assert_eq!(json!(result), e["returned"], "idle outcome {c}");
            assert_eq!(json!(t.stack.len()), e["frame_count"], "idle frames {c}");
            assert_eq!(
                json!(state.actors[&1].active.as_ref().map_or(0, |a| a.id)),
                e["active_serial"],
                "idle active {c}"
            );
            assert_eq!(
                json!(vm.world.objects[&1].person.as_ref().unwrap()[&33]),
                e["current_priority"],
                "idle priority {c}"
            );
            assert_eq!(
                json!(vm.world.objects[&2].data[&62]),
                e["target_use_count"],
                "idle use count {c}"
            );
        }
        assert_eq!(rows(&state.actors[&1].queue), e["queue"], "idle queue {c}");
        assert_eq!(
            json!(vm.world.objects[&1].temps),
            e["temps"],
            "idle temps {c}"
        );
        assert_eq!(
            json!(vm.world.objects[&1].attributes[0]),
            e["attribute"],
            "idle attribute {c}"
        );
        assert_eq!(json!(state.callbacks.iter().map(|cb|json!({"object":cb.object,"function":cb.function,"stack":cb.stack_object})).collect::<Vec<_>>()),e["callbacks"],"idle callbacks {c}");
        assert_eq!(vm, Snapshot::capture(&p, &vm).restore(&p).unwrap());
        count += 1;
    }
    for row in report["completion"].as_array().unwrap() {
        let c = json!({"queue":[{"priority":100}],"counter":5,"index":0,"dispatch":1});
        let (mut p, mut vm, id) = fixture(&c, 17);
        p.modules
            .get_mut("fixture")
            .unwrap()
            .routines
            .get_mut(&4097)
            .unwrap()
            .body
            .instructions[0]
            .on_true = row["input"]["return"].as_u64().unwrap() as u8;
        vm.run_slice(&p, id, 1).unwrap();
        vm.run_slice(&p, id, 1).unwrap();
        let e = &row["original"][1];
        let state = &vm.world.actions.as_ref().unwrap().actors[&1];
        assert_eq!(json!(vm.threads[&id].stack.len()), e["frames"]);
        assert_eq!(
            json!(vm.threads[&id].stack[0].continuation),
            e["continuation"]
        );
        assert_eq!(
            json!(state.previous.as_ref().unwrap().flags),
            e["previous_flags"]
        );
        assert_eq!(
            json!(state.previous.as_ref().unwrap().id),
            e["previous_serial"]
        );
        assert!(state.active.is_none());
        assert_eq!(json!(vm.world.objects[&2].data[&62]), e["use_count"]);
        assert_eq!(vm, Snapshot::capture(&p, &vm).restore(&p).unwrap());
        count += 1;
    }
    for row in report["push"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        let (p, mut vm, id) = fixture(c, 13);
        vm.run_slice(&p, id, 1).unwrap();
        assert_eq!(error(&vm, id), e["error"], "push error {c}");
        let state = vm.world.actions.as_ref().unwrap();
        assert_eq!(rows(&state.actors[&1].queue), e["queue"], "push queue {c}");
        assert_eq!(json!(state.serial), e["next_serial"], "push serial {c}");
        assert_eq!(vm, Snapshot::capture(&p, &vm).restore(&p).unwrap());
        count += 1;
    }
    println!(
        "{count} native queue/cancel/push/idle cases match, including {dispatched} dispatches"
    );
}
