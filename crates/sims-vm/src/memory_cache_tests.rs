use super::*;
use crate::{
    Definition, Module, Object, Routine, Snapshot, Start, World,
    image_cache::{Event, State},
};
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use std::collections::BTreeMap;

fn state(c: &Value) -> State {
    State {
        entries: serde_json::from_value(c["entries"].clone()).unwrap(),
        refs: serde_json::from_value(c["refs"].clone()).unwrap(),
        ui: c["ui"]
            .as_bool()
            .unwrap()
            .then(|| c["flags"].as_u64().unwrap() as u32),
    }
}

fn vm(c: &Value) -> (Program, Vm, u64) {
    let mut p = Program {
        globals: "script".into(),
        ..Default::default()
    };
    let operands = [
        c["field"].as_i64().unwrap() as i16,
        c["rhs"].as_i64().unwrap_or(0) as i16,
    ];
    let mut bytes = [0; 8];
    bytes[..2].copy_from_slice(&operands[0].to_le_bytes());
    bytes[2..4].copy_from_slice(&operands[1].to_le_bytes());
    bytes[5] = c["operator"].as_u64().unwrap_or(5) as u8;
    bytes[6] = c["scope"].as_u64().unwrap() as u8;
    bytes[7] = c["rhs_scope"].as_u64().unwrap_or(7) as u8;
    p.modules.insert(
        "script".into(),
        Module {
            source: "script".into(),
            routines: BTreeMap::from([(
                4096,
                Routine {
                    name: "person-field invalidation".into(),
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
            )]),
            ..Default::default()
        },
    );
    let mut world = World {
        image_cache: state(c),
        ..Default::default()
    };
    for i in 0..2 {
        let field = c["field"].as_i64().unwrap() as i16;
        let mut person: BTreeMap<_, _> = (0..256).map(|f| (f, (i as i16 + 1) * 100 + f)).collect();
        if (0..256).contains(&field) {
            person.insert(field, c["before"][i].as_i64().unwrap() as i16);
        }
        world.objects.insert(
            i as i16 + 1,
            Object {
                person: Some(person),
                definition: Some(Definition {
                    source: "script".into(),
                    object: i as u16 + 1,
                    guid: c["guids"][i].as_u64().unwrap() as u32,
                    slots: 0,
                }),
                ..Default::default()
            },
        );
    }
    let mut vm = Vm::new(world, 77);
    let id = vm
        .spawn(
            &p,
            Start {
                owner: "script",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: c["target"].as_i64().unwrap_or(2) as i16,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    (p, vm, id)
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_cache_release_and_dirty_publication_match_at_every_boundary() {
    let report: Value =
        serde_json::from_str(include_str!("../tests/fixtures/person-field-cache.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 534);
    for row in report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["input"]["mode"] == "cache")
    {
        let c = &row["input"];
        let n = &row["original"];
        let mut s = state(c);
        let mut events = vec![];
        let mut returned = false;
        for _ in 0..c["repeat"].as_u64().unwrap_or(1) {
            returned = s
                .notify_with(
                    c["guid"].as_u64().unwrap() as u32,
                    c["first"] == true,
                    &mut |e, s| {
                        events.push(match e {
                            Event::Release(id) => json!(["release", id, s]),
                            Event::Destroy(id) => json!(["destroy", id, 1]),
                            Event::Ui => json!(["ui", crate::image_cache::CHANGED, c["before"]]),
                        });
                    },
                )
                .unwrap();
            s.validate().unwrap();
            let restored: State = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
            assert_eq!(restored, s);
        }
        assert_eq!(json!(s), n["state"], "{c}");
        assert_eq!(json!(events), n["calls"], "{c}");
        assert_eq!(json!(returned), n["returned"], "{c}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_person_memory_and_expression_invalidate_only_at_write_intent() {
    let report: Value =
        serde_json::from_str(include_str!("../tests/fixtures/person-field-cache.json")).unwrap();
    for (i, row) in report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, r)| r["input"]["mode"] != "cache")
    {
        let c = &row["input"];
        let n = &row["original"];
        let (p, mut v, id) = vm(c);
        let mut t = v.threads.remove(&id).unwrap();
        let field = c["field"].as_i64().unwrap() as i16;
        let result = if c["mode"] == "memory" {
            v.resolve_runtime_memory(
                &p,
                &mut t,
                c["scope"].as_u64().unwrap() as u8,
                field,
                c["write"].as_bool().unwrap(),
            )
            .and_then(|(cell, _)| v.cell_value(&t, cell))
            .map(|x| json!(x))
        } else {
            let operands = p.modules["script"].routines[&4096].body.instructions[0].operands;
            v.expression(&p, &mut t, &operands).map(|x| json!(x.0))
        };
        if let Some(error) = n["error"].as_u64() {
            let actual = result.unwrap_err();
            assert!(
                actual.contains(&format!("native error 0x{error:02x}")),
                "case {i}: {actual}"
            );
        } else {
            assert_eq!(
                result.unwrap_or_else(|e| panic!("case {i}: {e}")),
                n["returned"],
                "case {i}"
            );
        }
        assert_eq!(json!(v.world.image_cache), n["state"], "case {i}: {c}");
        let words: Vec<_> = if (0..256).contains(&field) {
            [1, 2]
                .map(|id| v.world.objects[&id].person.as_ref().unwrap()[&field])
                .to_vec()
        } else {
            vec![]
        };
        assert_eq!(json!(words), n["person"], "case {i}");
        v.threads.insert(id, t);
        let restored =
            Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &v)).unwrap(), &p)
                .unwrap();
        assert_eq!(restored, v, "case {i}: save/restore");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn image_cache_rejects_dangling_or_underowned_images() {
    let report: Value =
        serde_json::from_str(include_str!("../tests/fixtures/person-field-cache.json")).unwrap();
    let mut s = state(&report["cases"][0]["input"]);
    s.refs[0] = 0;
    assert!(s.validate().is_err());
    s.entries[0].images[0] = Some(99);
    assert!(s.validate().is_err());
}
