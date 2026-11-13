use serde_json::Value;
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{Module, Object, Program, Routine, Snapshot, Start, Status, Vm, World, fires};

fn activities(signals: &Value) -> Vec<bool> {
    signals
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            assert_eq!(s[0], 0x108);
            assert_eq!(s[1], 0);
            !s[2].as_array().unwrap().is_empty()
        })
        .collect()
}

fn program() -> Program {
    let routines = [10u8, 11]
        .into_iter()
        .map(|sub| {
            (
                4096 + u16::from(sub - 10),
                Routine {
                    name: format!("Fire registry {sub}"),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 4,
                        locals: 0,
                        instructions: vec![Instruction {
                            opcode: 1,
                            on_true: 254,
                            on_false: 255,
                            operands: [sub, 0, 255, 255, 128, 128, 123, 0],
                        }],
                    },
                },
            )
        })
        .collect();
    Program {
        modules: [(
            "test".into(),
            Module {
                source: "test".into(),
                routines,
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn generic_fire_calls_match_original_caller_identity_duplicates_order_and_activity_events() {
    let data: Value =
        serde_json::from_str(include_str!("fixtures/fire-registration.json")).unwrap();
    let rows = data["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 364);
    let p = program();
    let mut checkpoints = 0;
    for row in rows {
        let input = &row["input"];
        let mut vm = Vm::new(
            World {
                fires: fires::State {
                    registered: serde_json::from_value(input["objects"].clone()).unwrap(),
                    ..Default::default()
                },
                objects: (1..=3).map(|id| (id, Object::default())).collect(),
                ..Default::default()
            },
            1,
        );
        for (op, expected) in input["operations"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["original"].as_array().unwrap())
        {
            let caller = op[1].as_i64().unwrap() as i16;
            let thread = vm
                .spawn(
                    &p,
                    Start {
                        owner: "test",
                        bhav: 4096 + op[0].as_u64().unwrap() as u16 - 10,
                        caller,
                        callee: caller,
                        stack_object: input["target"].as_i64().unwrap() as i16,
                        args: vec![0; 4],
                        check: false,
                    },
                )
                .unwrap();
            let objects_before = vm.world.objects.clone();
            vm.run_slice(&p, thread, 1).unwrap();
            assert_eq!(vm.world.objects, objects_before);
            assert_eq!(expected["error"], Value::Null);
            assert_eq!(expected["returned"], true);
            assert_eq!(
                vm.threads[&thread].status,
                Status::Returned(true),
                "{input}"
            );
            let registered: Vec<i16> = serde_json::from_value(expected["objects"].clone()).unwrap();
            assert_eq!(vm.world.fires.registered, registered, "{input}");
            assert_eq!(
                vm.world.fires.activity_changes,
                activities(&expected["signals"]),
                "{input}"
            );
            assert_eq!(vm.tick, 0);
            let saved = Snapshot::capture(&p, &vm);
            let restored = serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&saved).unwrap())
                .unwrap()
                .restore(&p)
                .unwrap();
            assert_eq!(vm, restored);
            vm = restored;
            let mut broken = saved;
            broken.state.world.fires.registered.push(4);
            assert!(broken.restore(&p).unwrap_err().contains("fire registry"));
            checkpoints += 1;
        }
    }
    assert_eq!(checkpoints, 2348);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn registry_unlink_removes_every_fire_membership_only_for_its_registered_object() {
    let data: Value =
        serde_json::from_str(include_str!("fixtures/fire-registration.json")).unwrap();
    let rows = data["deletions"].as_array().unwrap();
    assert_eq!(rows.len(), 42);
    for row in rows {
        let input = &row["input"];
        let mut state = fires::State {
            registered: serde_json::from_value(input["objects"].clone()).unwrap(),
            ..Default::default()
        };
        if input["registered"].as_bool().unwrap() {
            state.remove_object(input["caller"].as_i64().unwrap() as i16);
        }
        let expected = &row["original"];
        assert_eq!(expected["error"], Value::Null);
        let objects: Vec<i16> = serde_json::from_value(expected["objects"].clone()).unwrap();
        assert_eq!(state.registered, objects, "{input}");
        assert_eq!(
            state.activity_changes,
            activities(&expected["signals"]),
            "{input}"
        );
    }
}
