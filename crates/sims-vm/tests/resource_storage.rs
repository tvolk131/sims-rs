#![cfg(feature = "private-fixtures")]

//! Differential checks against captured native routines; no original assets required.
use serde_json::Value;
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::resource_storage::{Definition as StorageDefinition, Record, State};
use sims_vm::{Definition, Module, Object, Program, Routine, Snapshot, Start, Stop, Vm, World};
use std::collections::BTreeMap;

fn report() -> Value {
    serde_json::from_str(include_str!("fixtures/resource-storage.json")).unwrap()
}
fn fixture(aliases: [u32; 2], records: Vec<Record>, target: i16) -> (Program, Vm) {
    let catalog = records
        .iter()
        .enumerate()
        .map(|(i, r)| StorageDefinition {
            guid: r.guid,
            count: r.values.len() as i16,
            reset_per_lot: false,
            source: "fixture".into(),
            object: i as u16,
            content_hash: String::new(),
        })
        .collect();
    let mut p = Program {
        globals: "script".into(),
        resource_storage: Some(catalog),
        ..Program::default()
    };
    // The executing code owner is deliberately different from both objects.
    p.modules.insert(
        "script".into(),
        Module {
            source: "script".into(),
            routines: BTreeMap::from([(
                4096,
                Routine {
                    name: "shared storage".into(),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 0,
                        locals: 0,
                        instructions: vec![Instruction {
                            opcode: 2,
                            on_true: 254,
                            on_false: 255,
                            operands: [0, 0, 0, 0, 0, 2, 7, 7],
                        }],
                    },
                },
            )]),
            ..Module::default()
        },
    );
    let mut world = World {
        resource_storage: State { records },
        ..World::default()
    };
    for (i, alias) in aliases.into_iter().enumerate() {
        let source = format!("resource-{i}");
        let mut words = vec![0; 108];
        words[92] = alias as i16;
        words[93] = (alias >> 16) as i16;
        p.modules.insert(
            source.clone(),
            Module {
                source: source.clone(),
                object_definition_words: BTreeMap::from([(128, words)]),
                ..Module::default()
            },
        );
        world.objects.insert(
            i as i16 + 1,
            Object {
                definition: Some(Definition {
                    source,
                    object: 128,
                    guid: 101 + i as u32,
                    slots: 0,
                }),
                ..Object::default()
            },
        );
    }
    let mut v = Vm::new(world, 77);
    v.spawn(
        &p,
        Start {
            owner: "script",
            bhav: 4096,
            caller: 1,
            callee: 2,
            stack_object: target,
            args: vec![],
            check: false,
        },
    )
    .unwrap();
    (p, v)
}

#[test]
fn resource_scopes_match_native_aliases_bounds_and_pointer_publication() {
    for row in report()["memory"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let (p, v) = fixture(
            serde_json::from_value(c["aliases"].clone()).unwrap(),
            serde_json::from_value(c["records"].clone()).unwrap(),
            c["target"].as_i64().unwrap() as i16,
        );
        let result = v.inspect_memory(
            &p,
            1,
            c["scope"].as_u64().unwrap() as u8,
            c["index"].as_i64().unwrap() as i16,
            c["write"].as_bool().unwrap(),
        );
        if let Some(code) = n["error"].as_u64() {
            assert!(
                result
                    .unwrap_err()
                    .contains(&format!("native error 0x{code:02x}")),
                "{row}"
            );
        } else {
            let result = result.unwrap();
            assert_eq!(
                i64::from(result.value),
                n["value"].as_i64().unwrap(),
                "{row}"
            );
            assert_eq!(result.writable, n["writable"].as_bool().unwrap(), "{row}");
            assert!(!result.float_pointer);
        }
    }
}

#[test]
fn constructors_and_tatt_loads_match_native_records_and_cursor() {
    let data = report();
    for row in data["constructors"].as_array().unwrap() {
        let record = Record::new(0x12345678, row["count"].as_i64().unwrap() as i32);
        assert_eq!(
            record.values.len() as u64,
            row["original"]["count"].as_u64().unwrap()
        );
        assert!(record.values.iter().all(|v| *v == 0));
    }
    for row in data["loads"].as_array().unwrap() {
        let bytes: Vec<_> = row["bytes"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        let saved = sims_formats::tatt::Tatt::parse(&bytes).unwrap();
        let mut state = State {
            records: serde_json::from_value(row["records"].clone()).unwrap(),
        };
        let consumed = state.load(&saved);
        assert_eq!(
            consumed as u64,
            row["original"]["consumed"].as_u64().unwrap(),
            "{row}"
        );
        assert_eq!(
            serde_json::to_value(state.records).unwrap(),
            row["original"]["records"],
            "{row}"
        );
    }
}

#[test]
fn shared_writes_survive_snapshot_and_aliases_do_not_follow_code_owner() {
    let (mut p, mut v) = fixture(
        [102, 0],
        vec![Record {
            guid: 102,
            values: vec![10, 20],
        }],
        2,
    );
    p.modules
        .get_mut("script")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .instructions = vec![
        // Caller resource aliases 102; the other object's resource owns 102.
        Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 255,
            operands: [0, 0, 77, 0, 0, 5, 36, 7],
        },
        Instruction {
            opcode: 2,
            on_true: 254,
            on_false: 255,
            operands: [1, 0, 0, 0, 0, 5, 37, 36],
        },
    ];
    assert_eq!(v.run_slice(&p, 1, 1).unwrap().stop, Stop::Budget);
    assert_eq!(v.world.resource_storage.records[0].values, vec![77, 20]);
    assert_eq!(v.inspect_memory(&p, 1, 37, 0, false).unwrap().value, 77);
    let snapshot = Snapshot::capture(&p, &v);
    let mut resumed = Snapshot::from_json(&serde_json::to_vec(&snapshot).unwrap(), &p).unwrap();
    assert_eq!(
        v.run_slice(&p, 1, 1).unwrap(),
        resumed.run_slice(&p, 1, 1).unwrap()
    );
    assert_eq!(v.world.resource_storage.records[0].values, vec![77, 77]);
    assert_eq!(Snapshot::capture(&p, &v), Snapshot::capture(&p, &resumed));
    let mut malformed = snapshot.clone();
    malformed.state.world.resource_storage.records[0]
        .values
        .pop();
    assert!(
        malformed
            .restore(&p)
            .unwrap_err()
            .contains("layout differs")
    );
    let mut different = p.clone();
    different.resource_storage.as_mut().unwrap()[0].content_hash = "changed".into();
    assert!(
        snapshot
            .restore(&different)
            .unwrap_err()
            .contains("fingerprint")
    );
}
