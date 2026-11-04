use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::*;
use std::collections::BTreeMap;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_distance_direction_and_write_order() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/home-queries.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["expected"];
        let n = |k: &str| c[k].as_i64().unwrap() as i16;
        let opcode = n("opcode") as u16;
        let mut bytes = [0; 8];
        bytes[..2].copy_from_slice(&n("index").to_le_bytes());
        if opcode == 11 {
            bytes[2] = n("flags") as u8;
            bytes[3] = n("source_scope") as u8;
            bytes[4..6].copy_from_slice(&n("source").to_le_bytes());
        } else {
            bytes[2] = n("dest_scope") as u8;
            bytes[4] = n("flags") as u8;
            bytes[5] = n("source_scope") as u8;
            bytes[6..8].copy_from_slice(&n("source").to_le_bytes());
        }
        let mut m = Module {
            source: "test".into(),
            ..Default::default()
        };
        m.routines.insert(
            4096,
            Routine {
                name: "probe".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 0,
                    locals: 0,
                    instructions: vec![Instruction {
                        opcode,
                        on_true: 254,
                        on_false: 255,
                        operands: bytes,
                    }],
                },
            },
        );
        let mut world = World::default();
        for id in 1..=2 {
            let mut words = vec![0; 108];
            words[9] = if id == 1 { 2 } else { n("target_kind") };
            m.object_definition_words.insert(id as u16, words);
            let mut o = Object {
                definition: Some(Definition {
                    source: "test".into(),
                    object: id as u16,
                    guid: id as u32,
                    slots: 0,
                }),
                data: (0..72).map(|i| (i, 0)).collect(),
                attributes: vec![100, 101, 102, 103],
                temps: [1, 2, 3, 0, 7, -1, 8, 255],
                ..Default::default()
            };
            o.data.insert(11, id);
            world.objects.insert(id, o);
            let key = if id == 1 { "from" } else { "to" };
            world.positions.insert(
                id,
                autonomy::Placement {
                    x: c[key][0].as_i64().unwrap() as i32,
                    y: c[key][1].as_i64().unwrap() as i32,
                    level: n(if id == 1 { "from_level" } else { "to_level" }) as i32,
                },
            );
        }
        let p = Program {
            modules: BTreeMap::from([("test".into(), m)]),
            ..Default::default()
        };
        let mut vm = Vm::new(world, 1);
        let t = vm
            .spawn(
                &p,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: n("target"),
                    args: vec![],
                    check: false,
                },
            )
            .unwrap();
        let r = vm.run_slice(&p, t, 8).unwrap();
        if e["error"].is_null() {
            assert_eq!(r.stop, Stop::Returned(true), "{c}: {:?}", r.trace);
        } else {
            assert_eq!(r.stop, Stop::Faulted, "{c}");
        }
        assert_eq!(
            serde_json::json!(vm.world.objects[&1].temps),
            e["temps"],
            "{c}"
        );
        assert_eq!(
            serde_json::json!(vm.world.objects[&1].attributes),
            e["attributes"],
            "{c}"
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_adjacent_tile_scan_order_and_exhaustion() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/home-adjacent.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let cursor = c["cursor"]
            .as_array()
            .map(|v| [v[0].as_i64().unwrap() as i8, v[1].as_i64().unwrap() as i8]);
        let found = c["found"].as_u64().unwrap() as usize;
        let mut visits = person::adjacent_offsets(cursor);
        if found > 0 {
            visits.truncate(found);
        }
        let expected: Vec<_> = visits
            .iter()
            .map(|[x, y]| [i32::from(*x) + 5, i32::from(*y) + 5, 1])
            .collect();
        assert_eq!(serde_json::json!(expected), n["visits"], "{c}");
        assert_eq!(
            n["result"],
            found > 0 && found <= person::adjacent_offsets(cursor).len(),
            "{c}"
        );
    }
}
