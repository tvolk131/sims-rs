//! Synthetic counterparts to native_autonomy.py's original Gosub Found Action probes.
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{
    Definition, Module, Object, Program, Routine, Snapshot, Start, Vm, World, action,
    program::MenuEntry,
};
use std::collections::BTreeMap;

fn routine(opcode: u16) -> Routine {
    Routine {
        name: "synthetic".into(),
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
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn found_action_matches_native_frame_binding_and_resets_motive_changes() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/autonomy-selection.json")).unwrap();
    for row in report["found"].as_array().unwrap() {
        let target = row["input"]["target"].as_i64().unwrap() as i16;
        let menu = row["input"]["menu"].as_i64().unwrap() as i16;
        let mut module = Module {
            source: "local".into(),
            routines: BTreeMap::from([(4096, routine(0)), (4097, routine(30))]),
            ..Module::default()
        };
        let parent = module.routines.get_mut(&4097).unwrap();
        parent.body.instructions[0].on_true = 1;
        parent.body.instructions[0].on_false = 1;
        parent
            .body
            .instructions
            .push(routine(0).body.instructions[0].clone());
        module.object_interactions.insert(
            2,
            vec![MenuEntry {
                id: 0,
                action: 4096,
                ..MenuEntry::default()
            }],
        );
        let program = Program {
            globals: "local".into(),
            modules: BTreeMap::from([("local".into(), module)]),
            ..Program::default()
        };
        let mut world = World::default();
        world.objects.insert(
            1,
            Object {
                person: Some((0..256).map(|i| (i, 0)).collect()),
                motives: Some([0; 16]),
                data: BTreeMap::from([(8, 0), (20, menu)]),
                motive_changes: vec![sims_vm::motives::Change::new(6, 100, 90).unwrap()],
                ..Object::default()
            },
        );
        world.objects.insert(
            2,
            Object {
                definition: Some(Definition {
                    source: "local".into(),
                    object: 2,
                    guid: 0,
                    slots: 0,
                }),
                data: BTreeMap::from([(62, 82)]),
                ..Object::default()
            },
        );
        world.actions = Some(action::WorldState {
            serial: 100,
            actors: BTreeMap::from([(1, action::Actor::default())]),
            ..Default::default()
        });
        let mut vm = Vm::new(world, 0);
        let thread = vm
            .spawn(
                &program,
                Start {
                    owner: "local",
                    bhav: 4097,
                    caller: 1,
                    callee: 1,
                    stack_object: target,
                    args: vec![1, 2, 3, 4],
                    check: false,
                },
            )
            .unwrap();
        vm.run_slice(&program, thread, 1).unwrap();
        let original = &row["original"];
        let t = &vm.threads[&thread];
        let error = if let sims_vm::Status::Faulted(f) = &t.status {
            if f.message.contains("0x15") {
                serde_json::json!(21)
            } else if f.message.contains("0x16") {
                serde_json::json!(22)
            } else {
                panic!("{f:?}")
            }
        } else {
            serde_json::Value::Null
        };
        assert_eq!(error, original["error"]);
        assert_eq!(serde_json::json!(t.stack.len()), original["frames"]);
        assert_eq!(
            serde_json::json!(t.stack[0].continuation),
            original["parent_continuation"]
        );
        if t.stack.len() > 1 {
            assert_eq!(serde_json::json!(t.stack[1].args), original["args"]);
        }
        let actor = &vm.world.objects[&1];
        assert_eq!(
            serde_json::json!(actor.person.as_ref().unwrap()[&33]),
            original["active_priority"]
        );
        assert_eq!(
            serde_json::json!(actor.motive_changes.len()),
            original["changes"]
        );
        let actions = vm.world.actions.as_ref().unwrap();
        assert_eq!(serde_json::json!(actions.serial), original["next_serial"]);
        assert_eq!(
            serde_json::json!(actions.actors[&1].active.as_ref().map_or(0, |a| a.id)),
            original["active_id"]
        );
        assert_eq!(
            serde_json::json!(actions.actors[&1].active.as_ref().map_or(0, |a| a.flags)),
            original["active_flags"]
        );
        assert_eq!(
            serde_json::json!(vm.world.objects[&2].data[&62]),
            original["use_count"]
        );
        if t.stack.len() > 1 {
            let mut resumed = Snapshot::from_json(
                &serde_json::to_vec(&Snapshot::capture(&program, &vm)).unwrap(),
                &program,
            )
            .unwrap();
            assert_eq!(
                vm.run_slice(&program, thread, 100).unwrap(),
                resumed.run_slice(&program, thread, 100).unwrap()
            );
            assert_eq!(vm, resumed);
            vm.tick += 1;
            resumed.tick += 1;
            assert_eq!(
                vm.run_slice(&program, thread, 100).unwrap(),
                resumed.run_slice(&program, thread, 100).unwrap()
            );
            assert_eq!(vm, resumed);
            assert_eq!(vm.world.objects[&2].data[&62], 82);
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn found_action_transition_matches_original_history_attenuation_and_frame_reset() {
    use sims_formats::tuning::{Constant, Constants};
    use sims_vm::program::AutonomyAdvertisement;
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/action-transition.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let queued = c["queued"] == true;
        let mut module = Module {
            source: "local".into(),
            routines: BTreeMap::from([(4096, routine(0)), (4097, routine(30))]),
            ..Default::default()
        };
        if queued {
            module.routines.get_mut(&4097).unwrap().body.instructions = vec![
                Instruction {
                    opcode: 13,
                    on_true: 1,
                    on_false: 253,
                    operands: [0, 0, 2, 4, 0, 0, 0, 0],
                },
                Instruction {
                    opcode: 17,
                    on_true: 254,
                    on_false: 255,
                    operands: [0, 0, 1, 0, 0, 0, 0, 0],
                },
            ];
        }
        module.named_constants.insert(
            2,
            Constants {
                version: 1,
                entries: [
                    ("low attenuation", 0.125),
                    ("moderate attenuation", 0.25),
                    ("high attenuation", 0.5),
                    ("visitor low attenuation", 0.0625),
                    ("visitor moderate attenuation", 0.375),
                    ("visitor high attenuation", 0.875),
                ]
                .into_iter()
                .map(|(name, value)| Constant {
                    name: name.into(),
                    value,
                    comment: String::new(),
                })
                .collect(),
            },
        );
        module.object_interactions.insert(
            2,
            vec![MenuEntry {
                action: 4096,
                flags: c["flags"].as_u64().unwrap() as u32,
                autonomy: AutonomyAdvertisement {
                    attenuation_code: c["code"].as_u64().unwrap() as u32,
                    attenuation: f32::from_bits(c["explicit_bits"].as_u64().unwrap() as u32),
                    ..Default::default()
                },
                ..Default::default()
            }],
        );
        let program = Program {
            globals: "local".into(),
            modules: BTreeMap::from([("local".into(), module)]),
            ..Default::default()
        };
        let mut world = World::default();
        let mut owner = sims_vm::autonomy_constants::Owner::default();
        for (field, value) in [
            (sims_vm::autonomy_constants::Field::LowAttenuation, 0.125f32),
            (
                sims_vm::autonomy_constants::Field::ModerateAttenuation,
                0.25,
            ),
            (sims_vm::autonomy_constants::Field::HighAttenuation, 0.5),
            (sims_vm::autonomy_constants::Field::VisitorLow, 0.0625),
            (sims_vm::autonomy_constants::Field::VisitorModerate, 0.375),
            (sims_vm::autonomy_constants::Field::VisitorHigh, 0.875),
        ] {
            owner.process.words[field.index()] = Some(value.to_bits());
        }
        world.autonomy_constants = Some(owner);
        let mut person: BTreeMap<i16, i16> = (0..256).map(|i| (i, 0)).collect();
        person.insert(32, c["visitor"].as_i64().unwrap() as i16);
        world.objects.insert(
            1,
            Object {
                person: Some(person),
                motives: Some([0; 16]),
                data: BTreeMap::from([(8, 0), (20, 0)]),
                ..Default::default()
            },
        );
        world.objects.insert(
            2,
            Object {
                definition: Some(Definition {
                    source: "local".into(),
                    object: 2,
                    guid: 0,
                    slots: 0,
                }),
                data: (0..72).map(|i| (i, 0)).collect(),
                ..Default::default()
            },
        );
        let record = |id| action::Action {
            id,
            target: 2,
            icon_owner: 2,
            priority: 2,
            routine: 4096,
            flags: 14,
            attenuation_bits: 0x3e99999a,
            ..Default::default()
        };
        world.actions = Some(action::WorldState {
            serial: 100,
            actors: BTreeMap::from([(
                1,
                action::Actor {
                    active: c["active"].as_bool().unwrap().then(|| record(9)),
                    previous: c["previous"].as_bool().unwrap().then(|| record(8)),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        });
        let mut vm = Vm::new(world, 12345);
        let thread = vm
            .spawn(
                &program,
                Start {
                    owner: "local",
                    bhav: 4097,
                    caller: 1,
                    callee: 1,
                    stack_object: if queued { 1 } else { 2 },
                    args: if queued { vec![2, 0, 0, 0] } else { vec![0; 4] },
                    check: false,
                },
            )
            .unwrap();
        if queued {
            vm.run_slice(&program, thread, 1).unwrap();
            // Match the independently supplied last-result/idle counter at
            // the native dispatch boundary, after constructing the queue entry.
            vm.threads.get_mut(&thread).unwrap().stack[0].args[0] = 5;
        }
        vm.threads.get_mut(&thread).unwrap().last_result = c["last_result"].as_bool().unwrap();
        vm.threads
            .get_mut(&thread)
            .unwrap()
            .frame_memory
            .resize(48, Some(0xa5a5u16 as i16));
        let mut resumed = Snapshot::from_json(
            &serde_json::to_vec(&Snapshot::capture(&program, &vm)).unwrap(),
            &program,
        )
        .unwrap();
        assert_eq!(
            vm.run_slice(&program, thread, 1).unwrap(),
            resumed.run_slice(&program, thread, 1).unwrap()
        );
        assert_eq!(vm, resumed, "{c}");
        let actions = vm.world.actions.as_ref().unwrap();
        let project = |a: &Option<action::Action>| {
            a.as_ref().map(|a| {
                serde_json::json!({
                    "id":a.id, "flags":a.flags, "attenuation_bits":a.attenuation_bits
                })
            })
        };
        let t = &vm.threads[&thread];
        let actual = serde_json::json!({
            "active":project(&actions.actors[&1].active), "previous":project(&actions.actors[&1].previous),
            "frames":t.stack.len(), "parent_continuation":t.stack[0].continuation,
            "child_continuation":t.stack[1].continuation, "next_serial":actions.serial,
        });
        // Native return 3 denotes entering a child frame; compare the actual
        // stack and records, rather than manufacture a native return value.
        let mut expected = row["original"].clone();
        assert_eq!(
            expected.as_object_mut().unwrap().remove("returned"),
            Some(serde_json::json!(3))
        );
        assert_eq!(actual, expected, "{c}");
        assert_eq!(vm.rng_state, 12345);
        let mut resumed = Snapshot::from_json(
            &serde_json::to_vec(&Snapshot::capture(&program, &vm)).unwrap(),
            &program,
        )
        .unwrap();
        assert_eq!(
            vm.run_slice(&program, thread, 8).unwrap(),
            resumed.run_slice(&program, thread, 8).unwrap()
        );
        assert_eq!(vm, resumed, "{c}");
    }
}
