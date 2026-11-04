//! Original resource/dispatch fixtures with supplied string storage, plus native
//! parameterized-formatting cases. Covers check/live paths and restored actions.
use serde_json::Value;
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn native_literal_check_action_string_vectors() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/action-string-check.json")).unwrap();
    for context in report["autonomy_checks"].as_array().unwrap() {
        assert_eq!(context["live"], 0);
        assert_eq!(context["menu"], serde_json::json!([false, false, false]));
    }
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let index = c["index"].as_u64().unwrap() as u8;
        let target = c["target"].as_i64().unwrap() as i16;
        let p = program(index);
        let mut world = World::default();
        world.objects.insert(1, Object::default());
        for check in [true, false] {
            let mut vm = Vm::new(world.clone(), 1);
            let id = vm
                .spawn(
                    &p,
                    Start {
                        owner: "test",
                        bhav: 4096,
                        caller: 1,
                        callee: 1,
                        stack_object: target,
                        args: vec![],
                        check,
                    },
                )
                .unwrap();
            vm.run_slice(&p, id, 1).unwrap();
            let n = &row["original"];
            assert!(n["error"].is_null());
            assert_eq!(n["returned"], 1);
            assert_eq!(n["renamed"], false);
            assert_eq!(n["published"], serde_json::json!([]));
            assert_eq!(n["formatted"], 1);
            assert_eq!(n["table"], 302);
            assert_eq!(n["index"], index);
            assert_eq!(vm.threads[&id].status, Status::Returned(true));
            assert_eq!(vm.world, world);
            assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
        }
    }
}

fn program(index: u8) -> Program {
    Program {
        globals: "test".into(),
        modules: BTreeMap::from([(
            "test".into(),
            Module {
                source: "test".into(),
                strings: BTreeMap::from([(
                    302,
                    vec!["First literal".into(), "Second literal".into()],
                )]),
                routines: BTreeMap::from([(
                    4096,
                    Routine {
                        name: "label".into(),
                        body: Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 0,
                            locals: 0,
                            instructions: vec![Instruction {
                                opcode: 50,
                                on_true: 254,
                                on_false: 255,
                                operands: [46, 1, 0xcd, 0xab, index, 11, 22, 33],
                            }],
                        },
                    },
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_live_action_label_vectors() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/action-string-live.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let index = c["index"].as_u64().unwrap() as u8;
        let mut p = program(index);
        p.modules
            .get_mut("test")
            .unwrap()
            .object_interactions
            .insert(
                1,
                vec![sims_vm::program::MenuEntry {
                    id: 0,
                    action: 4096,
                    ..Default::default()
                }],
            );
        let mut world = World::default();
        world.objects.insert(
            1,
            Object {
                person: Some(BTreeMap::new()),
                definition: Some(Definition {
                    source: "test".into(),
                    object: 1,
                    guid: 1,
                    slots: 0,
                }),
                ..Default::default()
            },
        );
        let action = action::Action {
            flags: c["flags"].as_u64().unwrap() as u32,
            routine: 4096,
            target: 1,
            ..Default::default()
        };
        world.actions = Some(action::WorldState {
            actors: BTreeMap::from([(
                1,
                action::Actor {
                    active: Some(action.clone()),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        });
        let mut vm = Vm::new(world.clone(), 1);
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: c["target"].as_i64().unwrap() as i16,
                    args: vec![],
                    check: c["live"] == 0,
                },
            )
            .unwrap();
        vm.run_slice(&p, id, 1).unwrap();
        assert_eq!(vm.threads[&id].status, Status::Returned(true), "{c}");
        let actual = vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .as_ref()
            .unwrap();
        assert_eq!(actual.flags, n["flags"].as_u64().unwrap() as u32, "{c}");
        let label = match index {
            1 => "First literal",
            2 => "Second literal",
            _ => "",
        };
        assert_eq!(
            actual.label.as_deref(),
            if c["live"] == 1 { Some(label) } else { None }
        );
        assert_eq!(
            n["copies"],
            if c["live"] == 1 {
                serde_json::json!([40])
            } else {
                serde_json::json!([])
            }
        );
        let mut expected = action;
        expected.flags = actual.flags;
        expected.label = actual.label.clone();
        assert_eq!(*actual, expected);
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_action_string_frame_resource_selection() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/action-string-resource.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let owner = format!("context{}", c["context"].as_u64().unwrap());
        let table = c["table"].as_u64().unwrap() as u16;
        let mut p = program(10);
        for slot in 0..3 {
            let source = format!("context{slot}");
            let mut module = p.modules["test"].clone();
            module.source.clone_from(&source);
            module.strings = BTreeMap::from([(table, vec![source.clone(); 10])]);
            module.routines.get_mut(&4096).unwrap().body.instructions[0].operands[..2]
                .copy_from_slice(&table.to_le_bytes());
            module.object_interactions.insert(
                1,
                vec![sims_vm::program::MenuEntry {
                    id: 0,
                    action: 4096,
                    ..Default::default()
                }],
            );
            p.modules.insert(source, module);
        }
        let mut world = World::default();
        world.objects.insert(
            1,
            Object {
                person: Some(BTreeMap::new()),
                definition: Some(Definition {
                    source: owner.clone(),
                    object: 1,
                    guid: 1,
                    slots: 0,
                }),
                ..Default::default()
            },
        );
        world.actions = Some(action::WorldState {
            actors: BTreeMap::from([(
                1,
                action::Actor {
                    active: Some(action::Action {
                        routine: 4096,
                        target: 1,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        });
        let mut vm = Vm::new(world, 1);
        let id = vm
            .spawn(
                &p,
                Start {
                    owner: &owner,
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: c["target"].as_i64().unwrap() as i16,
                    args: vec![],
                    check: c["live"] == 0,
                },
            )
            .unwrap();
        vm.run_slice(&p, id, 1).unwrap();
        assert_eq!(vm.threads[&id].status, Status::Returned(true));
        assert_eq!(n["resource"], c["context"]);
        assert_eq!(n["context_reads"], serde_json::json!([c["context"]]));
        assert_eq!(n["table"], c["table"]);
        let action = vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .as_ref()
            .unwrap();
        assert_eq!(action.flags, n["flags"].as_u64().unwrap() as u32);
        assert_eq!(
            action.label.as_deref(),
            if c["live"] == 1 {
                Some(owner.as_str())
            } else {
                None
            }
        );
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_parameterized_action_labels_share_the_dialog_formatter() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/action-string-local-format.json")).unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        let locals: Vec<i16> = row["input"]["locals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i16)
            .collect();
        let mut p = program(1);
        let module = p.modules.get_mut("test").unwrap();
        module
            .strings
            .insert(302, vec![row["input"]["text"].as_str().unwrap().into()]);
        module.routines.get_mut(&4096).unwrap().body.locals = locals.len() as u16;
        module.object_interactions.insert(
            1,
            vec![sims_vm::program::MenuEntry {
                id: 0,
                action: 4096,
                ..Default::default()
            }],
        );
        for check in [false, true] {
            let mut world = World::default();
            world.objects.insert(
                1,
                Object {
                    person: Some(BTreeMap::new()),
                    definition: Some(Definition {
                        source: "test".into(),
                        object: 1,
                        guid: 1,
                        slots: 0,
                    }),
                    ..Default::default()
                },
            );
            world.actions = Some(action::WorldState {
                actors: BTreeMap::from([(
                    1,
                    action::Actor {
                        active: Some(action::Action {
                            routine: 4096,
                            target: 1,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            });
            let mut expected = world.clone();
            if !check {
                let a = expected
                    .actions
                    .as_mut()
                    .unwrap()
                    .actors
                    .get_mut(&1)
                    .unwrap()
                    .active
                    .as_mut()
                    .unwrap();
                a.label = Some(row["original"].as_str().unwrap().into());
                a.flags |= 0x80;
            }
            let mut vm = Vm::new(world, 32);
            let rng = (vm.rng_state, vm.lot_rng_state, vm.crt_rng_state);
            let id = vm
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
            vm.threads
                .get_mut(&id)
                .unwrap()
                .stack
                .last_mut()
                .unwrap()
                .locals = locals.clone();
            vm.run_slice(&p, id, 1).unwrap();
            assert_eq!(
                vm.threads[&id].status,
                Status::Returned(true),
                "{:?}",
                row["input"]
            );
            assert_eq!(vm.world, expected, "{:?}", row["input"]);
            assert_eq!((vm.rng_state, vm.lot_rng_state, vm.crt_rng_state), rng);
            assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
        }
    }
}

#[test]
fn action_formatter_does_not_silently_accept_unimplemented_token_families() {
    let mut p = program(1);
    p.modules
        .get_mut("test")
        .unwrap()
        .strings
        .insert(302, vec!["$Unsupported:0".into()]);
    let mut world = World::default();
    world.objects.insert(1, Object::default());
    let mut vm = Vm::new(world, 1);
    let id = vm
        .spawn(
            &p,
            Start {
                owner: "test",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![],
                check: true,
            },
        )
        .unwrap();
    vm.run_slice(&p, id, 1).unwrap();
    let Status::Faulted(fault) = &vm.threads[&id].status else {
        panic!("unsupported formatter silently succeeded");
    };
    assert!(
        fault
            .message
            .contains("unsupported native token $Unsupported:0")
    );
}
