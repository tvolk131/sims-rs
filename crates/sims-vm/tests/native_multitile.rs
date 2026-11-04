#![cfg(feature = "private-fixtures")]

use serde_json::{Value, json};
use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::ObjectFunction,
};
use sims_vm::{multitile::Part, *};
use std::collections::BTreeMap;

fn report() -> Value {
    serde_json::from_str(include_str!("fixtures/multitile.json")).unwrap()
}
fn routine(instruction: Instruction) -> Routine {
    Routine {
        name: "synthetic".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            instructions: vec![instruction],
        },
    }
}
fn make_world(origins: &Value, direction: i16) -> (Program, Vm) {
    let mut module = Module {
        source: "test".into(),
        ..Default::default()
    };
    // Synthetic bodies expose the original ordering boundary through attributes.
    module.routines.insert(
        4096,
        routine(Instruction {
            opcode: 2,
            on_true: 254,
            on_false: 253,
            operands: [0, 0, 99, 0, 0, 5, 0, 7],
        }),
    );
    module.routines.insert(
        4097,
        routine(Instruction {
            opcode: 2,
            on_true: 254,
            on_false: 253,
            operands: [0, 0, 100, 0, 0, 5, 0, 7],
        }),
    );
    let origins = origins.as_array().unwrap();
    for (i, origin) in origins.iter().enumerate() {
        let id = i as u16 + 1;
        let mut words = vec![0; 108];
        words[10] = 1;
        words[11] = ((origin[0].as_i64().unwrap() as i16) << 8)
            | (origin[1].as_i64().unwrap() as i16 & 255);
        words[56] = origin[2].as_i64().unwrap() as i16;
        module.object_definition_words.insert(id, words);
        module.object_functions.insert(
            id,
            vec![
                ObjectFunction {
                    action: 4096,
                    condition: 0,
                },
                ObjectFunction {
                    action: 4097,
                    condition: 0,
                },
            ],
        );
    }
    let p = Program {
        globals: "test".into(),
        modules: BTreeMap::from([("test".into(), module)]),
        ..Default::default()
    };
    let mut w = World::default();
    for i in 0..origins.len() {
        let id = i as i16 + 1;
        let mut o = Object::construct(
            Definition {
                source: "test".into(),
                object: id as u16,
                guid: id as u32,
                slots: 0,
            },
            &p,
            0,
        )
        .unwrap();
        o.data.insert(1, direction);
        o.attributes.fill(123);
        o.multitile = Some(Part {
            head: (id != 1).then_some(1),
            next: (i + 1 < origins.len()).then_some(id + 1),
            origin: multitile::origin(&p.modules["test"].object_definition_words[&(id as u16)])
                .unwrap(),
            offset: [0; 3],
        });
        w.objects.insert(id, o);
    }
    (p, Vm::new(w, 1))
}
#[test]
fn original_link_offsets_reset_order_and_snapshot_integrity() {
    for row in report()["reset"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let (p, mut v) = make_world(&c["origins"], c["direction"].as_i64().unwrap() as i16);
        let ids = v
            .prepare_multitile_reset(&p, c["part"].as_i64().unwrap() as i16, Default::default())
            .unwrap();
        assert!(
            v.world
                .objects
                .values()
                .all(|o| o.attributes.iter().all(|a| *a == 0))
        );
        let mut order: Vec<_> = ids.iter().map(|id| json!(["reset", id])).collect();
        assert!(v.threads.values().all(|t| t.callback.is_none()));
        v = Snapshot::capture(&p, &v).restore(&p).unwrap();
        for id in ids {
            v.begin_multitile_init(&p, id).unwrap();
            let t = v.object_threads[&id];
            v = Snapshot::capture(&p, &v).restore(&p).unwrap();
            v.run_slice(&p, t, 1).unwrap();
            assert_eq!(v.world.objects[&id].attributes[0], 99);
            order.push(json!(["init", id]));
            if c["update"] == 1 {
                assert_eq!(v.run_slice(&p, t, 1).unwrap().stop, Stop::Returned(true));
                order.push(json!(["update", id]));
            }
            Snapshot::capture(&p, &v).restore(&p).unwrap();
        }
        assert_eq!(json!(order), n["order"], "{c}");
        let parts:Vec<_>=v.world.objects.values().map(|o| { let part=o.multitile.as_ref().unwrap();json!({"head":part.head.unwrap_or(0),"next":part.next.unwrap_or(0),"origin":part.origin,"offset":part.offset,"world_offset":multitile::world_offset(part.offset)}) }).collect();
        assert_eq!(json!(parts), n["parts"], "{c}");
        let mut bad = Snapshot::capture(&p, &v);
        bad.state
            .world
            .objects
            .get_mut(&1)
            .unwrap()
            .multitile
            .as_mut()
            .unwrap()
            .next = Some(1);
        assert!(bad.restore(&p).is_err());
    }
}
#[test]
fn original_set_to_next_multitile() {
    for row in report()["iteration"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let (mut p, mut v) = make_world(
            &json!(vec![[0, 0, 0]; c["count"].as_u64().unwrap() as usize]),
            0,
        );
        if c["group"] == false {
            for o in v.world.objects.values_mut() {
                o.multitile = None;
            }
        }
        p.modules.get_mut("test").unwrap().routines.insert(
            4098,
            routine(Instruction {
                opcode: 31,
                on_true: 254,
                on_false: 255,
                operands: [0, 0, 0, 0, 131, c["scope"].as_u64().unwrap() as u8, 0, 0],
            }),
        );
        let current = c["current"].as_i64().unwrap() as i16;
        v.world.objects.get_mut(&1).unwrap().temps[0] = current;
        let t = v
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4098,
                    caller: 1,
                    callee: 1,
                    stack_object: current,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let run = v.run_slice(&p, t, 1).unwrap();
        assert_eq!(
            run.stop,
            Stop::Returned(n["returned"].as_bool().unwrap()),
            "{c}"
        );
        // The primitive's trace and returned branch both depend on the written value.
        let expected = n["value"].as_i64().unwrap() as i16;
        if c["scope"] == 8 {
            assert_eq!(v.world.objects[&1].temps[0], expected, "{c}");
        }
        assert!(
            run.trace[0].effect.contains(&format!("→ {expected}")),
            "{c}: {:?}",
            run.trace
        );
    }
}
#[test]
fn original_empty_type_search_from_zero_cursor() {
    for row in report()["empty_types"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let (mut p, mut v) = make_world(&json!([[0, 0, 0]]), 0);
        let mut operands = [0; 8];
        operands[..4].copy_from_slice(&0x12345678u32.to_le_bytes());
        operands[4] = 132;
        operands[5] = c["scope"].as_u64().unwrap() as u8;
        p.modules.get_mut("test").unwrap().routines.insert(
            4098,
            routine(Instruction {
                opcode: 31,
                on_true: 1,
                on_false: 0,
                operands,
            }),
        );
        let t = v
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4098,
                    caller: 1,
                    callee: 1,
                    stack_object: 0,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let run = v.run_slice(&p, t, 1).unwrap();
        assert_eq!(n["returned"], false);
        assert_eq!(n["value"], 0);
        assert_eq!(run.stop, Stop::Budget);
        assert_eq!(v.threads[&t].stack[0].location.instruction, 0);
        assert_eq!(v.threads[&t].stack[0].stack_object, 0);
        assert_eq!(v.world.objects[&1].temps[0], 0);
    }
}
#[test]
fn original_primary_resource_selection_and_file_isolation() {
    for row in report()["resources"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let mut m = Module::default();
        for (i, d) in c["definitions"].as_array().unwrap().iter().enumerate() {
            if d["file"] != 0 {
                continue;
            }
            let id = i as u16 + 1;
            let mut w = vec![0; 108];
            for (index, key) in [(10, "group"), (11, "index"), (56, "level"), (63, "primary")] {
                w[index] = d[key].as_i64().unwrap() as i16;
            }
            m.object_definition_words.insert(id, w);
            m.object_definition_order.push(id);
        }
        let plan = multitile::construction_plan(&m, c["catalog"].as_u64().unwrap() as u16).unwrap();
        let primary = n["primary"].as_u64().unwrap();
        let expected: Vec<_> = std::iter::once(primary)
            .chain(
                n["parts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap())
                    .filter(|id| *id != 0 && *id != primary),
            )
            .collect();
        assert_eq!(json!(plan), json!(expected), "{c}");
    }
}
#[test]
fn original_type_test_accepts_part_and_catalog_guids() {
    for row in report()["types"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let (mut p, mut v) = make_world(&json!([[0, 0, 0], [0, 1, 0]]), 0);
        let m = p.modules.get_mut("test").unwrap();
        let mut catalog = vec![0; 108];
        catalog[10] = 1;
        catalog[11] = -1;
        catalog[14] = 99;
        m.object_definition_words.insert(3, catalog);
        m.object_definition_order = vec![3, 1, 2];
        for id in [1, 2] {
            if c["group"] == false {
                m.object_definition_words.get_mut(&id).unwrap()[10] = 0;
            }
            let o = v.world.objects.get_mut(&(id as i16)).unwrap();
            o.definition.as_mut().unwrap().guid = 100 + id as u32;
            if c["resource"] == false {
                o.definition = None;
            }
        }
        let mut operands = [0; 8];
        operands[..4].copy_from_slice(&(c["guid"].as_u64().unwrap() as u32).to_le_bytes());
        operands[6] = 10;
        m.routines.insert(
            4098,
            routine(Instruction {
                opcode: 32,
                on_true: 254,
                on_false: 255,
                operands,
            }),
        );
        let t = v
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4098,
                    caller: 1,
                    callee: 1,
                    stack_object: c["target"].as_i64().unwrap() as i16,
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let run = v.run_slice(&p, t, 1).unwrap();
        if n["error"] == 23 {
            assert_eq!(run.stop, Stop::Faulted);
            assert!(run.trace[0].effect.contains("0x17"));
        } else {
            assert_eq!(
                run.stop,
                Stop::Returned(n["returned"].as_bool().unwrap()),
                "{c}"
            );
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_interactions_follow_signed_part_indices() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/domestic-interaction-target.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let (mut p, v) = make_world(&json!([[0, 0, 0], [0, 1, 0], [1, 0, 0]]), 0);
        for (i, index) in c["indices"].as_array().unwrap().iter().enumerate() {
            let words = p
                .modules
                .get_mut("test")
                .unwrap()
                .object_definition_words
                .get_mut(&(i as u16 + 1))
                .unwrap();
            words[8] = index.as_i64().unwrap() as i16;
            words[10] = i16::from(c["group"].as_bool().unwrap());
        }
        assert_eq!(
            json!(
                multitile::interaction_target(&v.world, &p, c["part"].as_i64().unwrap() as i16)
                    .unwrap()
            ),
            row["original"],
            "{c}"
        );
    }
}
