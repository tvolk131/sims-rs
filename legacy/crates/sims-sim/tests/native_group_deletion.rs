//! Compare queue identity and storage unlink independently of the pending live
//! group-destruction adapter. Native Reset/notification services are boundaries
//! in this fixture, not asserted to be implemented by these helper comparisons.
use serde_json::Value;
use sims_formats::execution::ObjectFunction;
use sims_sim::lot_removal;
use sims_vm::{Definition, Module, Object, Program, World, multitile, removal::Pending};
use std::collections::BTreeMap;

fn fixture(c: &Value) -> (Program, World) {
    let mut module = Module {
        source: "group".into(),
        ..Default::default()
    };
    let mut world = World::default();
    for id in 1i16..=4 {
        let group = id != 4 && c["group"] == true;
        let mut words = vec![0; 108];
        words[9] = 4;
        words[10] = c["master"].as_i64().unwrap() as i16;
        words[11] = (id * 2) << 8 | id;
        words[56] = id % 2;
        module.object_definition_words.insert(id as u16, words);
        let mut functions = vec![
            ObjectFunction {
                action: 0,
                condition: 0
            };
            11
        ];
        functions[8].action = if c["dynamic"] == true { 4096 } else { 0 };
        module.object_functions.insert(id as u16, functions);
        world.objects.insert(
            id,
            Object {
                definition: Some(Definition {
                    source: "group".into(),
                    object: id as u16,
                    guid: id as u32,
                    slots: 0,
                }),
                multitile: group.then_some(multitile::Part {
                    head: (id != 1).then_some(1),
                    next: match id {
                        1 => Some(3),
                        3 => Some(2),
                        _ => None,
                    },
                    origin: [i32::from(id) * 2, i32::from(id), i32::from(id % 2)],
                    offset: [
                        10 + i32::from(id) * 2,
                        12 + i32::from(id),
                        i32::from(id % 2),
                    ],
                }),
                ..Default::default()
            },
        );
    }
    (
        Program {
            globals: "group".into(),
            modules: BTreeMap::from([("group".into(), module)]),
            ..Default::default()
        },
        world,
    )
}

#[cfg(feature = "private-fixtures")]
#[test]
fn queue_identity_and_storage_unlink_match_original() {
    let report: Value = serde_json::from_str(include_str!("fixtures/group-deletion.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 207);
    let mut unlink_count = 0;
    for (case, row) in cases.iter().enumerate() {
        let c = &row["input"];
        let (p, mut world) = fixture(c);
        let valid_group = c["master"] == 7 && c["group"] == true;
        if valid_group {
            multitile::validate(&world, &p).unwrap();
            let mut corrupted = world.clone();
            corrupted
                .objects
                .get_mut(&3)
                .unwrap()
                .multitile
                .as_mut()
                .unwrap()
                .offset[0] += 1;
            // Native dynamic joins retain runtime XY origins/caches. Only
            // static groups require the resource-relative offset invariant.
            assert_eq!(
                multitile::validate(&corrupted, &p).is_err(),
                c["dynamic"] != true
            );
        }
        let events = row["original"]["events"].as_array().unwrap();
        if c["operation"] == "mark" {
            let target = c["target"].as_i64().unwrap() as i16;
            let mut queue: Vec<i16> = serde_json::from_value(c["queued"].clone()).unwrap();
            let pending: Vec<_> = queue
                .iter()
                .map(|&target| Pending {
                    target,
                    suppress_reset: false,
                })
                .collect();
            let mut detached = vec![];
            if let Some(selected) = lot_removal::queue_target(&world, &p, target).unwrap()
                && !sims_vm::removal::already_queued(&world, &pending, selected).unwrap()
            {
                queue.push(selected);
                detached.push(selected);
            }
            assert_eq!(
                serde_json::json!(queue),
                row["original"]["queue"],
                "case {case}"
            );
            let expected: Vec<_> = events
                .iter()
                .filter(|e| e["kind"] == "detach")
                .map(|e| e["args"][0].as_i64().unwrap() as i16)
                .collect();
            assert_eq!(detached, expected, "case {case}");
        } else {
            for event in events.iter().filter(|e| e["kind"] == "base_destroy") {
                let id = event["args"][0].as_i64().unwrap() as i16;
                if world.objects[&id].multitile.is_some() {
                    multitile::unlink(&mut world, id).unwrap();
                    unlink_count += 1;
                }
                for state in event["state"].as_array().unwrap() {
                    let id = state["id"].as_i64().unwrap() as i16;
                    if let Some(part) = &world.objects[&id].multitile {
                        assert_eq!(
                            serde_json::json!(part.head),
                            state["head"],
                            "case {case}, part {id}"
                        );
                        assert_eq!(
                            serde_json::json!(part.next),
                            state["next"],
                            "case {case}, part {id}"
                        );
                        assert_eq!(
                            serde_json::json!(part.origin),
                            state["origin"],
                            "case {case}, part {id}"
                        );
                        assert_eq!(
                            serde_json::json!(part.offset),
                            state["offset"],
                            "case {case}, part {id}"
                        );
                    }
                }
                if valid_group {
                    multitile::validate(&world, &p).unwrap();
                    let resumed: World =
                        serde_json::from_slice(&serde_json::to_vec(&world).unwrap()).unwrap();
                    multitile::validate(&resumed, &p).unwrap();
                    assert_eq!(resumed, world);
                }
            }
        }
    }
    assert_eq!(unlink_count, 96);
}
