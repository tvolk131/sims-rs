use serde_json::{Value, json};
use sims_vm::{
    Object, World,
    action::{Action, Actor},
    removal::{Pending, already_queued, selection},
};

#[cfg(feature = "private-fixtures")]
#[test]
fn self_removal_in_synchronous_callbacks_follows_original_return_mode() {
    use sims_formats::behavior::{Behavior, Instruction};
    use sims_vm::{Definition, Module, Program, Routine, Start, Status, Vm};
    let report: Value = serde_json::from_str(include_str!("fixtures/remove-object.json")).unwrap();
    for check in [false, true] {
        for flags in [0, 1, 2, 3, 255] {
            let mut p = Program {
                globals: "test".into(),
                ..Default::default()
            };
            let mut module = Module {
                source: "test".into(),
                ..Default::default()
            };
            module.object_functions.insert(1, vec![]);
            module.routines.insert(
                4096,
                Routine {
                    name: "self removal".into(),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 0,
                        locals: 0,
                        instructions: vec![Instruction {
                            opcode: 0x12,
                            on_true: 254,
                            on_false: 255,
                            operands: [0, 0, flags, 0, 0, 0, 0, 0],
                        }],
                    },
                },
            );
            p.modules.insert("test".into(), module);
            let mut world = World::default();
            world.objects.insert(
                1,
                Object {
                    definition: Some(Definition {
                        source: "test".into(),
                        object: 1,
                        guid: 7,
                        slots: 0,
                    }),
                    data: [(8, 0x100)].into(),
                    ..Default::default()
                },
            );
            world.positions.insert(
                1,
                sims_vm::autonomy::Placement {
                    x: -16,
                    y: -16,
                    level: 1,
                },
            );
            let mut vm = Vm::new(world, 1);
            let thread = vm
                .spawn(
                    &p,
                    Start {
                        owner: "test",
                        bhav: 4096,
                        caller: 1,
                        callee: 1,
                        stack_object: 1,
                        args: vec![],
                        check,
                    },
                )
                .unwrap();
            vm.run_slice(&p, thread, 1).unwrap();
            let Status::AwaitingRemoval(r) = vm.threads[&thread].status.clone() else {
                panic!("removal must reach host")
            };
            assert_eq!(
                sims_vm::Snapshot::capture(&p, &vm).restore(&p).unwrap(),
                vm,
                "self removal/check continuation must save before host completion"
            );
            let mut corrupt = sims_vm::Snapshot::capture(&p, &vm);
            let Status::AwaitingRemoval(pending) =
                &mut corrupt.state.threads.get_mut(&thread).unwrap().status
            else {
                unreachable!()
            };
            pending.flags ^= 1;
            assert!(corrupt.restore(&p).is_err());
            let mut queue = vec![];
            vm.defer_offlot_removal(&p, &mut queue, thread, &r).unwrap();
            let original = report["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| {
                    let c = &r["input"];
                    c["mode"] == 0
                        && c["flags"] == flags
                        && c["target"] == 1
                        && c["live"] == !check
                        && c["parent"] == false
                        && c["queue"] == json!([])
                })
                .unwrap();
            let returned = match vm.threads[&thread].status {
                Status::Sleeping { .. } => 2,
                Status::Returned(true) => 1,
                ref s => panic!("unexpected removal result {s:?}"),
            };
            assert_eq!(returned, original["original"]["returned"]);
            assert_eq!(
                queue,
                [Pending {
                    target: 1,
                    suppress_reset: flags & 2 != 0
                }]
            );
            assert!(
                vm.world.objects.contains_key(&1),
                "removal is deferred until registry flush"
            );
            assert_eq!(vm.world.objects[&1].data[&8], 0);
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_remove_selection_duplicate_ancestor_and_flags() {
    let report: Value = serde_json::from_str(include_str!("fixtures/remove-object.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let mut world = World::default();
        world.objects.insert(1, Object::default());
        world.objects.insert(
            2,
            Object {
                container: c["parent"].as_bool().unwrap().then_some((1, 0)),
                ..Default::default()
            },
        );
        let target = selection(
            c["mode"].as_i64().unwrap() as i16,
            1,
            c["target"].as_i64().unwrap() as i16,
        );
        if n["error"] == 9 {
            assert!(target.is_err());
            continue;
        }
        let target = target.unwrap();
        let flags = c["flags"].as_u64().unwrap() as u8;
        let mut queue: Vec<_> = c["queue"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| Pending {
                target: id.as_i64().unwrap() as i16,
                suppress_reset: false,
            })
            .collect();
        let mut detached = vec![];
        let mut internal = [0x1234u32; 2];
        let returned = if world.objects.contains_key(&target) {
            if !already_queued(&world, &queue, target).unwrap() {
                detached.push(target);
                queue.push(Pending {
                    target,
                    suppress_reset: flags & 2 != 0,
                });
                internal[target as usize - 1] = (internal[target as usize - 1] & !0x1000)
                    | if flags & 2 != 0 { 0x1000 } else { 0 };
            }
            if target == 1 && c["live"] == true && flags & 1 == 0 {
                2
            } else {
                1
            }
        } else {
            0
        };
        assert_eq!(json!(returned), n["returned"], "{c}");
        assert_eq!(
            json!(queue.iter().map(|r| r.target).collect::<Vec<_>>()),
            n["queue"],
            "{c}"
        );
        assert_eq!(json!(detached), n["detached"], "{c}");
        assert_eq!(json!(internal), n["flags"], "{c}");
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_person_inactive_target_reference_cleanup() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/delete-references.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let obj = |b: &Value| if b.as_bool().unwrap() { 2 } else { 1 };
        let mut actor = Actor {
            previous: Some(Action {
                id: 99,
                target: obj(&c["previous_target"]),
                icon_owner: obj(&c["previous_icon"]),
                priority: 77,
                ..Default::default()
            }),
            ..Default::default()
        };
        for (i, a) in c["queue"].as_array().unwrap().iter().enumerate() {
            actor.queue.pending.push_back(Action {
                id: i as u32 + 1,
                target: obj(&a[0]),
                icon_owner: obj(&a[1]),
                priority: 100,
                ..Default::default()
            });
        }
        let callbacks = actor.remove_object_references(2).unwrap();
        assert_eq!(
            json!(
                callbacks
                    .iter()
                    .map(|&object| json!({"object":object,"function":4,"stack":1}))
                    .collect::<Vec<_>>()
            ),
            n["callbacks"]
        );
        assert_eq!(
            json!(
                actor
                    .queue
                    .pending
                    .iter()
                    .map(|a| json!({"priority":a.priority,"flags":a.flags,"serial":a.id}))
                    .collect::<Vec<_>>()
            ),
            n["queue"]
        );
        assert_eq!(
            json!(actor.previous.as_ref().map_or(0, |a| a.target)),
            n["previous_target"]
        );
        assert_eq!(
            json!(actor.previous.as_ref().map_or(0, |a| a.icon_owner)),
            n["previous_icon"]
        );
        assert_eq!(
            json!(actor.previous.as_ref().map_or(0, |a| a.id)),
            n["previous_serial"]
        );
    }
}

#[test]
fn self_removal_yields_without_executing_its_success_branch() {
    use sims_vm::*;
    use std::collections::BTreeMap;
    for flags in [0, 1, 2, 3] {
        let p = Program {
            globals: "test".into(),
            modules: BTreeMap::from([(
                "test".into(),
                Module {
                    source: "test".into(),
                    routines: BTreeMap::from([(
                        4096,
                        Routine {
                            name: "self removal".into(),
                            body: sims_formats::behavior::Behavior {
                                format: 0x8002,
                                kind: 0,
                                args: 0,
                                locals: 0,
                                instructions: vec![sims_formats::behavior::Instruction {
                                    opcode: 18,
                                    on_true: 254,
                                    on_false: 255,
                                    operands: [0, 0, flags, 0, 0, 0, 0, 0],
                                }],
                            },
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let mut vm = Vm::new(
            World {
                objects: BTreeMap::from([(1, Object::default())]),
                ..Default::default()
            },
            1,
        );
        let t = vm
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: 1,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        vm.run_slice(&p, t, 1).unwrap();
        let Status::AwaitingRemoval(r) = vm.threads[&t].status.clone() else {
            panic!()
        };
        vm.finish_removal(&p, t, r.id).unwrap();
        if flags & 1 == 0 {
            assert_eq!(vm.threads[&t].status, Status::Sleeping { wake_tick: 1 });
            assert_eq!(vm.threads[&t].stack[0].location.instruction, 0);
            Snapshot::capture(&p, &vm).restore(&p).unwrap();
        } else {
            assert_eq!(vm.threads[&t].status, Status::Returned(true));
        }
    }
}
