use super::*;
use crate::{Definition, Module, Object, Routine, Snapshot, Start, World};
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};

fn ids(v: &Value) -> Vec<Option<i16>> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let i = v.as_i64().unwrap() as i16;
            (i != 0).then_some(i)
        })
        .collect()
}
fn queues(q: &crate::indicator_unregister::Queues) -> Value {
    json!({"dates":q.dates.iter().map(|i|i.unwrap_or(0)).collect::<Vec<_>>(),
        "stars":q.stars.iter().map(|i|i.unwrap_or(0)).collect::<Vec<_>>(),"dirty":q.dirty})
}
fn fixture(c: &Value) -> (Program, Vm, u64, [u8; 8]) {
    let field = c["field"].as_i64().unwrap() as i16;
    let scope = c["scope"].as_u64().unwrap() as u8;
    let index = if matches!(scope, 30 | 31) { 0 } else { field };
    let mut bytes = [0; 8];
    bytes[..2].copy_from_slice(&index.to_le_bytes());
    bytes[2..4].copy_from_slice(&(c["rhs"].as_i64().unwrap_or(0) as i16).to_le_bytes());
    bytes[5] = c["operator"].as_u64().unwrap_or(5) as u8;
    bytes[6] = scope;
    bytes[7] = c["rhs_scope"].as_u64().unwrap_or(7) as u8;
    let module = Module {
        source: "script".into(),
        routines: [(
            4096,
            Routine {
                name: "native person notification".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 0,
                    locals: 0,
                    instructions: vec![Instruction {
                        opcode: 2,
                        on_true: 254,
                        on_false: 255,
                        operands: bytes,
                    }],
                },
            },
        )]
        .into(),
        ..Default::default()
    };
    let p = Program {
        globals: "script".into(),
        modules: [("script".into(), module)].into(),
        ..Default::default()
    };
    let mut world = World::default();
    world.indicator_queues = crate::indicator_unregister::Queues {
        dates: ids(&c["dates"]),
        stars: ids(&c["stars"]),
        dirty: c["dirty"] != 0,
    };
    for id in 1..=2 {
        let mut person: std::collections::BTreeMap<_, _> =
            (0..256).map(|f| (f, id * 100 + f)).collect();
        person.insert(77, -2);
        person.insert(field, c["before"][id as usize - 1].as_i64().unwrap() as i16);
        let mut object = Object {
            person: Some(person),
            data: [(11, c["ids"][id as usize - 1].as_i64().unwrap() as i16)].into(),
            definition: Some(Definition {
                source: "script".into(),
                object: id as u16,
                guid: id as u32,
                slots: 0,
            }),
            ..Default::default()
        };
        object.temps[0] = field;
        world.objects.insert(id, object);
    }
    let mut v = Vm::new(world, 77);
    let thread = v
        .spawn(
            &p,
            Start {
                owner: "script",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: c["target"].as_i64().unwrap() as i16,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    (p, v, thread, bytes)
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_person_write_intent_queues_before_mutation_and_survives_errors() {
    let report: Value = serde_json::from_str(include_str!(
        "../tests/fixtures/person-indicator-notifications.json"
    ))
    .unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 838);
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let native = &row["original"];
        let (p, mut v, thread, bytes) = fixture(c);
        let mut t = v.threads.remove(&thread).unwrap();
        let mut result = Ok(Value::Null);
        for _ in 0..c["repeat"].as_u64().unwrap_or(1) {
            result = if c["mode"] == "expression" {
                v.expression(&p, &mut t, &bytes).map(|x| json!(x.0))
            } else {
                v.resolve_runtime_memory(&p, &mut t, bytes[6], word(&bytes, 0), c["write"] == true)
                    .and_then(|(cell, _)| v.cell_value(&t, cell))
                    .map(|x| json!(x))
            };
            if result.is_err() {
                break;
            }
        }
        if let Some(error) = native["error"].as_u64() {
            let actual = result.unwrap_err();
            assert!(
                actual.contains(&format!("native error 0x{error:02x}")),
                "case {n}: {actual}"
            );
        } else {
            assert_eq!(
                result.unwrap_or_else(|e| panic!("case {n}: {e}")),
                native["returned"],
                "case {n}"
            );
        }
        assert_eq!(
            queues(&v.world.indicator_queues),
            native["queues"],
            "case {n}: {c}"
        );
        let field = c["field"].as_i64().unwrap() as i16;
        assert_eq!(
            json!([
                v.world.objects[&1].person.as_ref().unwrap()[&field],
                v.world.objects[&2].person.as_ref().unwrap()[&field]
            ]),
            native["person"],
            "case {n}"
        );
        v.threads.insert(thread, t);
        // Invalid caller inputs are retained only as resolver experiments;
        // do not claim a valid saved VM frame with a missing Stack Object.
        if matches!(c["target"].as_i64(), Some(1 | 2)) {
            let restored = Snapshot::capture(&p, &v).restore(&p).unwrap();
            assert_eq!(restored, v, "case {n}: save/resume");
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn notification_queue_snapshot_rejects_missing_objects_and_old_semantics() {
    let report: Value = serde_json::from_str(include_str!(
        "../tests/fixtures/person-indicator-notifications.json"
    ))
    .unwrap();
    let mut c = report["cases"][0]["input"].clone();
    c["target"] = json!(2);
    let (p, mut v, _, _) = fixture(&c);
    v.world.indicator_queues.stars = vec![None, Some(99)];
    assert!(
        Snapshot::capture(&p, &v)
            .restore(&p)
            .unwrap_err()
            .contains("queue references an absent object")
    );
    v.world.indicator_queues.stars.clear();
    let mut saved = Snapshot::capture(&p, &v);
    saved.program_hash = "prior semantics".into();
    assert!(
        saved
            .restore(&p)
            .unwrap_err()
            .contains("fingerprint mismatch")
    );
}
