//! Compare adult-Main shared services against the private original-code report.
use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{person::*, *};
use std::collections::BTreeMap;
fn n(v: &Value, k: &str, d: i64) -> i64 {
    v[k].as_i64().unwrap_or(d)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let report: Value = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).expect("private report"),
    )?)?;
    compare(report)
}
pub fn compare(report: Value) -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let mut count = 0;
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let kind = c["kind"].as_str().unwrap();
        let mut b = [0; 8];
        let opcode = match kind {
            "next" => {
                b[..4].copy_from_slice(&(n(c, "guid", 0) as u32).to_le_bytes());
                b[4] = 128 + n(c, "mode", 5) as u8;
                b[5] = n(c, "scope", 10) as u8;
                b[7] = n(c, "index", 0) as u8;
                31
            }
            "inventory" => {
                b[0] = 3;
                b[1] = n(c, "token_kind", 0) as u8;
                b[2..4].copy_from_slice(&(n(c, "flags", 0) as u16).to_le_bytes());
                b[4..].copy_from_slice(&(n(c, "guid", 0) as u32).to_le_bytes());
                51
            }
            "zoning" => {
                b[0] = 28;
                1
            }
            "autonomy" => 3,
            "look" => 22,
            "neighbor" | "neighbor_metadata" | "functions" => 2,
            "named" => {
                b = [47, 1, 1, 0, 33, n(c, "mode", 0) as u8, 0, 0];
                28
            }
            _ => panic!("unknown kind"),
        };
        let mut p = Program {
            globals: "probe".into(),
            modules: BTreeMap::from([(
                "probe".into(),
                Module {
                    source: "probe".into(),
                    routines: BTreeMap::from([(
                        4096,
                        Routine {
                            name: "probe".into(),
                            body: Behavior {
                                format: 0x8002,
                                kind: 0,
                                args: 4,
                                locals: 4,
                                instructions: vec![Instruction {
                                    opcode,
                                    on_true: 254,
                                    on_false: 255,
                                    operands: b,
                                }],
                            },
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let module = p.modules.get_mut("probe").unwrap();
        let mut entries = vec![
            sims_formats::execution::ObjectFunction {
                action: 0,
                condition: 0
            };
            16
        ];
        entries[15].action = n(c, "function", 0) as u16;
        module.object_functions.insert(1, entries);
        if c["present"] == true {
            let mut strings = vec![String::new(); 33];
            strings[32] = "idle override".into();
            module.strings.insert(303, strings);
        }
        let mut w = World::default();
        let mut ps = WorldState {
            nearby_houses: c["nearby_houses"]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_i64().unwrap() as i32).collect()),
            free_will: c["free_will"] == true,
            pet_autonomy_disabled: c["pet_disabled"] == true,
            controlled_family: n(c, "controlled_family", 1) as i16,
            object_order: c["order"]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_i64().unwrap() as i16).collect())
                .unwrap_or(vec![1, 2]),
            person_order: c["people_order"]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_i64().unwrap() as i16).collect())
                .unwrap_or(vec![2, 1]),
            tile_chains: vec![vec![1, 2]],
            lot_zoning: BTreeMap::from([(1, 1), (2, 2), (3, 7)]),
            ..Default::default()
        };
        if let Some(families) = c["families"].as_object() {
            ps.families = families
                .iter()
                .map(|(family, members)| {
                    (
                        family.parse().unwrap(),
                        members
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|id| id.as_i64().unwrap() as i16)
                            .collect(),
                    )
                })
                .collect();
        }
        if let Some(count) = c["slot_count"].as_u64() {
            ps.neighbor_slots = Some(vec![None; count as usize]);
        }
        for (i, id) in c["neighbors"]
            .as_array()
            .cloned()
            .unwrap_or(vec![json!(1)])
            .iter()
            .enumerate()
        {
            let id = id.as_i64().unwrap() as i16;
            if let Some(slots) = &mut ps.neighbor_slots {
                slots[id as usize - 1] = Some(id);
            }
            ps.neighbors.insert(
                id,
                Neighbor {
                    guid: if let Some(guid) = c["neighbor_guids"][i].as_u64() {
                        guid as u32
                    } else if id == 1 {
                        0x7fd96b54
                    } else {
                        0x7fd96b54 + id as u32
                    },
                    person: (0..256)
                        .map(|i| (i, id.wrapping_mul(100).wrapping_add(i)))
                        .collect(),
                    inventory: if id == 1 {
                        c["entries"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .map(|e| InventoryEntry {
                                        kind: n(e, "kind", 0) as u8,
                                        guid: n(e, "guid", 0) as u32,
                                        count: n(e, "count", 0) as i16,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
            );
        }
        w.person = Some(ps);
        let mut actor = Object {
            definition: Some(Definition {
                source: "probe".into(),
                object: 1,
                guid: 0x7fd96b54,
                slots: 0,
            }),
            person: Some(BTreeMap::from([
                (32, n(c, "autonomy", 0) as i16),
                (61, n(c, "family", 1) as i16),
                (65, n(c, "species", 0) as i16),
            ])),
            temps: [11, 22, 33, 44, 2, 66, 77, 88],
            ..Default::default()
        };
        if kind == "next" {
            actor.temps[0] = n(c, "current", 0) as i16;
        }
        if kind == "zoning" {
            actor.temps[0] = n(c, "lot", 0) as i16;
        }
        w.objects.insert(2, actor.clone());
        w.objects.insert(1, actor);
        if let Some(guids) = c["live_guids"].as_array() {
            for (i, guid) in guids.iter().enumerate() {
                w.objects
                    .get_mut(&((i + 1) as i16))
                    .unwrap()
                    .definition
                    .as_mut()
                    .unwrap()
                    .guid = guid.as_u64().unwrap() as u32;
            }
        }
        if let Some(kinds) = c["kinds"].as_array() {
            for (i, kind) in kinds.iter().enumerate() {
                let id = (i + 1) as i16;
                let mut words = vec![0; 108];
                words[9] = kind.as_i64().unwrap() as i16;
                p.modules
                    .get_mut("probe")
                    .unwrap()
                    .object_definition_words
                    .insert(id as u16, words);
                w.objects
                    .get_mut(&id)
                    .unwrap()
                    .definition
                    .as_mut()
                    .unwrap()
                    .object = id as u16;
            }
        }

        if let Some(categories) = c["categories"].as_array() {
            for (i, category) in categories.iter().enumerate() {
                w.objects
                    .get_mut(&((i + 1) as i16))
                    .unwrap()
                    .data
                    .insert(59, category.as_i64().unwrap() as i16);
            }
        }
        let mut v = Vm::new(w, 1);
        let id = v
            .spawn(
                &p,
                Start {
                    owner: "probe",
                    bhav: 4096,
                    caller: 1,
                    callee: 1,
                    stack_object: if kind == "next" {
                        n(c, "current", 0)
                    } else if kind == "look" {
                        n(c, "target", 0)
                    } else if matches!(
                        kind,
                        "neighbor" | "neighbor_metadata" | "functions" | "named"
                    ) {
                        n(c, "id", 0)
                    } else {
                        2
                    } as i16,
                    args: vec![n(c, "category", 0) as i16, 0, 0, 0],
                    check: false,
                },
            )
            .unwrap();
        if matches!(kind, "neighbor" | "neighbor_metadata" | "functions") {
            let result = v.inspect_memory(
                &p,
                id,
                if kind == "neighbor" {
                    32
                } else if kind == "neighbor_metadata" {
                    24
                } else {
                    35
                },
                n(c, "index", 0) as i16,
                c["write"] == true,
            );
            if let Some(code) = row["original"]["error"].as_u64() {
                assert!(
                    result.unwrap_err().contains(&format!("0x{code:02x}")),
                    "{c}"
                );
            } else {
                let r = result.unwrap();
                assert_eq!(json!(r.value), row["original"]["value"], "{c}");
                assert!(!r.writable);
            }
        } else {
            // Keep the stack for inspecting Set To Next's destination after one instruction.
            let mut p = p;
            let ins = &mut p
                .modules
                .get_mut("probe")
                .unwrap()
                .routines
                .get_mut(&4096)
                .unwrap()
                .body
                .instructions[0];
            ins.on_true = 0;
            ins.on_false = 0;
            // Resume the same enumeration from a serialized VM before dispatch.
            let saved = serde_json::to_vec(&Snapshot::capture(&p, &v)).unwrap();
            let mut resumed = serde_json::from_slice::<Snapshot>(&saved)
                .unwrap()
                .restore(&p)
                .unwrap();
            let run = v.run_slice(&p, id, 1).unwrap();
            assert_eq!(
                resumed.run_slice(&p, id, 1).unwrap(),
                run,
                "resumed run {c}"
            );
            assert_eq!(resumed, v, "resumed state {c}");
            if let Some(code) = row["original"]["error"].as_u64() {
                assert_eq!(run.stop, Stop::Faulted, "{c}");
                assert!(
                    run.trace[0].effect.contains(&format!("0x{code:02x}")),
                    "{c}: {:?}",
                    run.trace
                );
            } else if kind == "autonomy" && row["original"]["value"] == true {
                assert_eq!(run.stop, Stop::Faulted, "{c}");
                assert!(
                    run.trace[0]
                        .effect
                        .contains("autonomy spatial context unavailable"),
                    "{:?}",
                    run.trace
                );
            } else {
                assert_eq!(run.stop, Stop::Budget, "{c}: {:?}", run.trace);
                assert_eq!(
                    json!(v.threads[&id].last_result),
                    row["original"]["returned"],
                    "return {c}"
                );
                let value = match kind {
                    "next" => json!(if b[5] == 10 {
                        v.threads[&id].stack.last().unwrap().stack_object
                    } else {
                        v.world.objects[&1].temps[0]
                    }),
                    "inventory" => json!(v.world.objects[&1].temps),
                    "zoning" => json!(v.world.objects[&1].temps[0]),
                    "look" => json!(
                        (41..=45)
                            .map(|i| v.world.objects[&1].person.as_ref().unwrap()[&i])
                            .collect::<Vec<_>>()
                    ),
                    _ => json!(false),
                };
                assert_eq!(value, row["original"]["value"], "value {c}");
                assert_eq!(
                    json!(v.world.person.as_ref().unwrap().inventory_dirty),
                    row["original"]["dirty"],
                    "dirty {c}"
                );
                assert_eq!(Snapshot::capture(&p, &v).restore(&p).unwrap(), v);
            }
        }
        count += 1;
    }
    println!("{count} original adult Main service comparisons match");
    Ok(())
}
