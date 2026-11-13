use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{idle::IdleState, *};
use std::collections::BTreeMap;

fn fixture(opcode: u16, scheduled: i16) -> (Program, Vm, u64, u64) {
    let p = Program {
        globals: "test".into(),
        modules: BTreeMap::from([(
            "test".into(),
            Module {
                source: "test".into(),
                routines: BTreeMap::from([(
                    4096,
                    Routine {
                        name: "synthetic cooldown probe".into(),
                        body: Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 4,
                            locals: 0,
                            instructions: vec![Instruction {
                                opcode,
                                on_true: 254,
                                on_false: 255,
                                operands: [0; 8],
                            }],
                        },
                    },
                )]),
                ..Module::default()
            },
        )]),
        ..Program::default()
    };
    let mut v = Vm::new(
        World {
            objects: (1..=2)
                .map(|id| {
                    (
                        id,
                        Object {
                            data: BTreeMap::from([(25, 17)]),
                            idle: Some(IdleState {
                                countdown: scheduled,
                                ..IdleState::default()
                            }),
                            ..Object::default()
                        },
                    )
                })
                .collect(),
            ..World::default()
        },
        1,
    );
    let mut ids = vec![];
    for object in [1, 2] {
        let id = v
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: object,
                    callee: object,
                    stack_object: 2,
                    args: vec![50, 60, 70, 80],
                    check: false,
                },
            )
            .unwrap();
        v.register_object_thread(object, id).unwrap();
        ids.push(id);
    }
    (p, v, ids[0], ids[1])
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_cooldown_access_wake_and_post_update_vectors() {
    let report: Value = serde_json::from_str(include_str!("fixtures/object-update.json")).unwrap();
    for row in report["access"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let (p, mut v, first, second) = fixture(
            c["opcode"].as_u64().unwrap() as u16,
            c["scheduled"].as_i64().unwrap() as i16,
        );
        let mut current = v.threads.remove(&first).unwrap();
        let object = if c["scope"] == 3 { 1 } else { 2 };
        v.wake_object(&p, &mut current, object).unwrap();
        let target = if object == 1 {
            &current
        } else {
            &v.threads[&second]
        };
        assert_eq!(n["error"], Value::Null);
        assert_eq!(n["value"], 17);
        assert_eq!(
            json!(v.world.objects[&object].idle.as_ref().unwrap().countdown),
            n["scheduled"],
            "{c}"
        );
        assert_eq!(json!(target.stack[0].args), n["args"], "{c}");
    }
    for row in report["updates"].as_array().unwrap() {
        let c = &row["input"];
        let (_, mut v, _, _) = fixture(0, -1);
        v.world
            .objects
            .get_mut(&1)
            .unwrap()
            .data
            .insert(25, c["value"].as_i64().unwrap() as i16);
        v.finish_object_update(1, c["completed"] == 1).unwrap();
        assert_eq!(
            json!(v.world.objects[&1].data[&25]),
            row["original"]["value"],
            "{c}"
        );
        let state = v.clone();
        v.finish_object_update(1, c["completed"] == 1).unwrap();
        assert_eq!(v, state, "repeated host visit changed countdown");
    }
}

#[test]
fn reading_and_writing_cooldown_wakes_sleep_without_discarding_remaining_time() {
    for write in [false, true] {
        let (mut p, mut v, first, second) = fixture(0, 60);
        let mut routine = p.modules["test"].routines[&4096].clone();
        routine.body.instructions[0] = Instruction {
            opcode: 2,
            on_true: 254,
            on_false: 255,
            operands: if write {
                [25, 0, 9, 0, 0, 5, 4, 7]
            } else {
                [25, 0, 17, 0, 0, 2, 4, 7]
            },
        };
        p.modules
            .get_mut("test")
            .unwrap()
            .routines
            .insert(4097, routine);
        v.threads.get_mut(&first).unwrap().stack[0].location.bhav = 4097;
        v.world
            .objects
            .get_mut(&1)
            .unwrap()
            .idle
            .as_mut()
            .unwrap()
            .countdown = -1;
        v.threads.get_mut(&second).unwrap().status = Status::Sleeping { wake_tick: 60 };
        assert_eq!(
            v.run_slice(&p, first, 10).unwrap().stop,
            Stop::Returned(true)
        );
        assert_eq!(v.world.objects[&2].idle.as_ref().unwrap().countdown, -2);
        assert_eq!(v.threads[&second].stack[0].args, [60, 60, 70, 80]);
        assert_eq!(v.threads[&second].status, Status::Ready);
        assert_eq!(v.world.objects[&2].data[&25], if write { 9 } else { 17 });
    }
}
