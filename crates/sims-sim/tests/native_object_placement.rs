use serde::Deserialize;
use serde_json::{Value, json};
use sims_sim::{
    architecture::Architecture,
    lot_geometry::{FloorSource, Lot},
    lot_links::{Descriptor, Links},
    lot_placement::{Operation, Request, State, VirtualOperation},
};
use sims_vm::{Definition, Module, Object, Program, World, autonomy::Placement};
use std::collections::BTreeMap;
#[derive(Deserialize)]
struct Input {
    size: usize,
    heads: Vec<Vec<i16>>,
    flags: Vec<Vec<u8>>,
    objects: Vec<Item>,
    root: i16,
    kind: String,
    destination: Descriptor,
    notify: bool,
    callback_mutation: bool,
    #[serde(default)]
    group: bool,
    #[serde(default)]
    group_mutation: bool,
    #[serde(default)]
    queue: Option<Vec<i16>>,
    #[serde(default)]
    suppress_reset: bool,
    #[serde(default)]
    desired: Option<i16>,
}
#[derive(Deserialize)]
struct Item {
    id: i16,
    position: [i32; 3],
    data: BTreeMap<i16, i16>,
    slots: Vec<i16>,
    functions: Vec<usize>,
    master: i16,
    marker: i16,
    #[serde(default)]
    person: Option<BTreeMap<i16, i16>>,
    #[serde(default)]
    multitile: Option<sims_vm::multitile::Part>,
}
fn compare(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c: Input = serde_json::from_value(row["input"].clone()).unwrap();
        let mut w = World::default();
        let mut m = Module {
            source: "objects".into(),
            ..Default::default()
        };
        let links = Links {
            size: c.size,
            heads: c.heads.clone(),
            next: c.objects.iter().map(|o| (o.id, o.slots[0])).collect(),
        };
        for o in &c.objects {
            let mut words = vec![0; 128];
            words[9] = if o.person.is_some() { 2 } else { 4 };
            words[10] = o.master;
            words[14] = o.id;
            words[0x86 / 2] = 8;
            words[0x8a / 2] = o.marker;
            m.object_definition_words.insert(o.id as u16, words);
            m.object_functions.insert(
                o.id as u16,
                (0..27)
                    .map(|i| sims_formats::execution::ObjectFunction {
                        action: if o.functions.contains(&i) { 4096 } else { 0 },
                        condition: 0,
                    })
                    .collect(),
            );
            w.objects.insert(
                o.id,
                Object {
                    definition: Some(Definition {
                        source: "objects".into(),
                        object: o.id as u16,
                        guid: o.id as u32,
                        slots: 0,
                    }),
                    data: o.data.clone(),
                    person: o.person.clone(),
                    slots: o
                        .slots
                        .iter()
                        .skip(1)
                        .map(|&id| (id != 0).then_some(id))
                        .collect(),
                    container: (o.data[&2] != 0).then_some((o.data[&2], o.data[&3] as u16)),
                    multitile: o.multitile.clone(),
                    ..Default::default()
                },
            );
            w.positions.insert(
                o.id,
                Placement {
                    x: o.position[0],
                    y: o.position[1],
                    level: o.position[2],
                },
            );
        }
        let p = Program {
            globals: "objects".into(),
            modules: BTreeMap::from([("objects".into(), m)]),
            ..Default::default()
        };
        let n = c.size * c.size;
        let source = FloorSource {
            coverings: vec![0; n],
            walls: vec![Default::default(); n],
            object_heads: vec![0; n],
            flags: vec![0; n],
        };
        let lot = Lot::rebuild(
            c.size,
            vec![[0; 4]; n],
            vec![255; n],
            vec![255; n],
            vec![source; 2],
        )
        .unwrap();
        let a = Architecture::from_original(lot, &p).unwrap();
        let mut state = State::new(links, &a).unwrap();
        state.flags = c.flags.clone();
        let mut queue = c.queue.clone().unwrap_or_default();
        let mut internal_flags = vec![0x1234u32; c.objects.len()];
        let already_queued = c.queue.is_some()
            && sims_sim::lot_removal::already_queued(
                &mut w,
                &state.links,
                &queue
                    .iter()
                    .map(|&target| sims_vm::removal::Pending {
                        target,
                        suppress_reset: false,
                    })
                    .collect::<Vec<_>>(),
                c.root,
            )
            .unwrap();
        let mut op = Some(if let Some(desired) = c.desired {
            sims_vm::multitile::update_offsets(&mut w, c.root).unwrap();
            VirtualOperation::copy_direction(&w, c.root, desired).unwrap()
        } else if c.group {
            if c.kind == "remove" {
                VirtualOperation::remove(&w, &state.links, c.root, c.notify).unwrap()
            } else {
                VirtualOperation::set(&w, &state.links, c.root, c.destination, c.notify).unwrap()
            }
        } else if c.kind == "remove" {
            VirtualOperation::Base(Operation::remove(c.root, c.notify))
        } else if row["original"].get("membership_after_placement").is_some() {
            VirtualOperation::Base(sims_sim::lot_route::movement::commit_operation(
                c.root,
                c.destination.position,
                c.destination.level,
            ))
        } else if w.objects[&c.root].person.is_some() {
            // Exercise the same virtual SetObject entry used by Find Location,
            // not just the already-supported base continuation.
            VirtualOperation::set(&w, &state.links, c.root, c.destination, c.notify).unwrap()
        } else {
            VirtualOperation::Base(Operation::set(c.root, c.destination, c.notify))
        });
        let mut expected = row["original"].clone();
        if let Some(wrapper) = expected.as_object_mut().unwrap().remove("wrapper") {
            let result =
                sims_sim::lot_find_location::detachment_operation(&w, &p, &state.links, c.root);
            if wrapper["split"] == true {
                assert!(result.unwrap_err().contains("dynamic-group split"));
                op = None;
            } else {
                assert_eq!(wrapper["returned"], 1);
                op = Some(result.unwrap());
            }
        }
        let mut events = vec![];
        let mut error = 91;
        let mut checks = 0;
        let mut observations = vec![];
        let mut queue_observations = vec![];
        while !already_queued {
            let Some(operation) = op.as_mut() else { break };
            let Some(request) = operation.next(&mut w, &p, &a, &mut state).unwrap() else {
                break;
            };
            // The native headless visible-world byte is false. This is a
            // conditional host boundary, so neither side invalidates a screen.
            if matches!(request, Request::Screen(_)) {
                continue;
            }
            if let Request::PlacementError(value) = request {
                error = value;
                checks += 1;
                continue;
            }
            queue_observations.push(queue.clone());
            observations.push(
                c.objects
                    .iter()
                    .map(|o| {
                        let q = w.positions[&o.id];
                        let obj = &w.objects[&o.id];
                        [i32::from(o.id), q.x, q.y, q.level]
                            .into_iter()
                            .chain(
                                [1, 2, 3, 8, 14, 26, 29]
                                    .into_iter()
                                    .map(|k| i32::from(obj.data[&k])),
                            )
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>(),
            );
            if let Request::Function {
                object,
                function: 10,
            } = request
            {
                if c.callback_mutation && object == c.root {
                    w.objects.get_mut(&object).unwrap().data.insert(1, 2);
                    w.positions.insert(
                        object,
                        Placement {
                            x: 56,
                            y: 56,
                            level: 1,
                        },
                    );
                }
            }
            if c.group_mutation
                && matches!(
                    request,
                    Request::Function {
                        object: 4,
                        function: 9 | 10
                    }
                )
            {
                w.objects
                    .get_mut(&4)
                    .unwrap()
                    .multitile
                    .as_mut()
                    .unwrap()
                    .next = Some(3);
                w.objects
                    .get_mut(&3)
                    .unwrap()
                    .multitile
                    .as_mut()
                    .unwrap()
                    .offset[0] = 2;
            }
            events.push(request);
            // Split at every native boundary to exercise the saved operation
            // continuation, including pending per-object spatial writes.
            op = serde_json::from_value(json!(op)).unwrap();
            state = serde_json::from_value(json!(state)).unwrap();
            assert!(events.len() < 300, "placement failed to complete");
        }
        if c.desired.is_some() {
            let parts: Vec<_> = c.objects.iter().map(|o| {
                let obj = &w.objects[&o.id];
                let q = w.positions[&o.id];
                json!({"id":o.id,"direction":obj.data[&1],"offset":obj.multitile.as_ref().unwrap().offset,
                    "position":[q.x,q.y,q.level]})
            }).collect();
            let attempts = events
                .iter()
                .filter(|r| matches!(r, Request::Function{object,function:10} if *object==c.root))
                .count();
            assert_eq!(
                json!({"parts":parts,"events":events,"error":error,"checks":checks,"attempts":attempts}),
                row["original"],
                "group rotation case {i}"
            );
            continue;
        }
        if c.queue.is_some() && !already_queued {
            let index = c.objects.iter().position(|o| o.id == c.root).unwrap();
            internal_flags[index] =
                (internal_flags[index] & !0x1000) | if c.suppress_reset { 0x1000 } else { 0 };
            queue.push(c.root);
        }
        let objects: Vec<_> = c
            .objects
            .iter()
            .map(|o| {
                let obj = &w.objects[&o.id];
                let p = w.positions[&o.id];
                let data: BTreeMap<_, _> = [1, 2, 3, 8, 14, 16, 26, 29, 40]
                    .into_iter()
                    .map(|k| (k, obj.data[&k]))
                    .collect();
                let slots: Vec<_> = std::iter::once(state.links.next[&o.id])
                    .chain(obj.slots.iter().map(|s| s.unwrap_or(0)))
                    .collect();
                json!({"id":o.id,"position":[p.x,p.y,p.level],"data":data,"slots":slots})
            })
            .collect();
        let mut actual = json!({"events":events,"observations":observations,"objects":objects,"heads":state.links.heads,"flags":state.flags,"spatial_dirty":state.spatial_dirty});
        if c.queue.is_some() {
            actual["queue"] = json!(queue);
            actual["internal_flags"] = json!(internal_flags);
            actual["queue_observations"] = json!(queue_observations);
        }
        if row["original"].get("bounds").is_some() {
            actual["bounds"] = json!(state.bounds);
        }
        if row["original"].get("membership_after_placement").is_some() {
            actual["membership_after_placement"] = json!(c.root);
        }
        assert_eq!(actual, expected, "base placement case {i}");
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_deferred_removal_and_physical_placement() {
    compare(serde_json::from_str(include_str!("fixtures/remove-placement.json")).unwrap());
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_immediate_removal_captures_room_before_callbacks() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/immediate-removal.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut world = World::default();
        world.objects.insert(
            2,
            Object {
                data: BTreeMap::from([
                    (7, c["word7"].as_i64().unwrap() as i16),
                    (29, c["room"].as_i64().unwrap() as i16),
                ]),
                ..Default::default()
            },
        );
        let room = sims_sim::lot_removal::immediate_room(&world, 2).unwrap();
        if c["mutate"] == true {
            world
                .objects
                .get_mut(&2)
                .unwrap()
                .data
                .extend([(7, 0), (29, 77)]);
        }
        let events = row["original"]["events"].as_array().unwrap();
        let destroyed = events
            .iter()
            .position(|e| *e == json!(["notify", 0x88, 2]))
            .unwrap();
        // Position propagation emits its own earlier F3 events. This is the
        // immediate wrapper's additional notification after destruction.
        let native: Vec<_> = events[destroyed + 1..]
            .iter()
            .filter(|e| e[0] == "notify" && e[1] == 0xf3)
            .map(|e| e[2].as_u64().unwrap() as u32 as i32)
            .collect();
        assert_eq!(
            native,
            room.into_iter().map(i32::from).collect::<Vec<_>>(),
            "{c}"
        );
        if room.is_some() {
            assert_eq!(events.last().unwrap()[1], 0xf3);
            assert_eq!(events[events.len() - 2], json!(["notify", 0x88, 2]));
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_base_placement_observations() {
    compare(serde_json::from_str(include_str!("fixtures/object-placement.json")).unwrap());
}
#[test]
#[ignore = "requires private native base placement oracle"]
fn native_base_placement_full_corpus() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/object-placement.json");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_person_placement_and_cached_bounds() {
    compare(serde_json::from_str(include_str!("fixtures/person-placement.json")).unwrap());
}
#[test]
#[ignore = "requires private native person SetObject report"]
fn native_person_placement_full_corpus() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/person-placement.json");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_group_placement_observations() {
    compare(serde_json::from_str(include_str!("fixtures/group-placement.json")).unwrap());
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_find_location_group_detachment_and_split_boundary() {
    compare(serde_json::from_str(include_str!("fixtures/find-location-group.json")).unwrap());
}
#[test]
#[ignore = "requires private native multipart placement oracle"]
fn native_group_placement_full_corpus() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/group-placement.json");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}

fn compare_walls(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let size = c["size"].as_u64().unwrap() as usize;
        let level = c["level"].as_i64().unwrap() as i32;
        let tile: [i32; 2] = serde_json::from_value(c["tile"].clone()).unwrap();
        let old = c["flags"].as_u64().unwrap() as u16;
        let mut m = Module {
            source: "walls".into(),
            ..Default::default()
        };
        m.object_functions.insert(
            1,
            (0..7)
                .map(|i| sims_formats::execution::ObjectFunction {
                    action: if i == 6 && c["callback"] == true {
                        4096
                    } else {
                        0
                    },
                    condition: 0,
                })
                .collect(),
        );
        let p = Program {
            globals: "walls".into(),
            modules: BTreeMap::from([("walls".into(), m)]),
            ..Default::default()
        };
        let mut w = World::default();
        w.objects.insert(
            1,
            Object {
                definition: Some(Definition {
                    source: "walls".into(),
                    object: 1,
                    guid: 1,
                    slots: 0,
                }),
                data: BTreeMap::from([(1, c["facing"].as_i64().unwrap() as i16), (5, old as i16)]),
                ..Default::default()
            },
        );
        w.positions.insert(
            1,
            Placement {
                x: tile[1] * 16 + 8,
                y: tile[0] * 16 + 8,
                level,
            },
        );
        let walls: Vec<_> = c["walls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| sims_formats::lot::WallTile {
                segments: v["segments"].as_u64().unwrap() as u8,
                styles: serde_json::from_value(v["styles"].clone()).unwrap(),
                ..Default::default()
            })
            .collect();
        let source = FloorSource {
            coverings: vec![0; size * size],
            walls,
            object_heads: vec![0; size * size],
            flags: vec![0; size * size],
        };
        let lot = Lot::rebuild(
            size,
            vec![[0; 4]; size * size],
            vec![255; size * size],
            vec![255; size * size],
            vec![source; 2],
        )
        .unwrap();
        let a = Architecture::from_original(lot, &p).unwrap();
        let flags = sims_sim::lot_placement::wall_flags(&w, &p, &a, 1).unwrap() as u16;
        assert_eq!(
            json!({"flags":flags,"callbacks":if flags!=old {vec![6]}else{vec![]}}),
            row["original"],
            "wall case {case}"
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_wall_placement_observations() {
    compare_walls(serde_json::from_str(include_str!("fixtures/placement-walls.json")).unwrap());
}
#[test]
#[ignore = "requires the private native wall placement oracle"]
fn native_wall_placement_full_corpus() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/placement-walls.json");
    compare_walls(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_walking_placement_and_room_update_order() {
    compare(serde_json::from_str(include_str!("fixtures/walking-placement.json")).unwrap());
}
#[test]
#[ignore = "requires private native walking-placement report"]
fn native_walking_placement_full_corpus() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/walking-placement.json");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn offlot_multipart_rotation_matches_original_at_every_observer() {
    let mut report: Value =
        serde_json::from_str(include_str!("fixtures/group-rotation.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 180);
    for row in report["cases"].as_array_mut().unwrap() {
        let c = &mut row["input"];
        c["kind"] = json!("rotate");
        c["group"] = json!(true);
        c["notify"] = json!(true);
        c["destination"] =
            json!({"on_lot":false,"position":[-16,-16],"container":null,"slot":0,"level":1});
    }
    compare(report);
}
