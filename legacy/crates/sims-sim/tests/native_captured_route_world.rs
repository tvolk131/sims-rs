//! Differential producer check on a paused native world, not a tick replay.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_formats::lot::WallTile;
use sims_sim::{
    Simulation,
    architecture::{Architecture, Footprint},
    lot_geometry::{Floor, FloorSource, Lot},
    lot_links::Links,
    lot_rooms::{Partition, metrics},
    lot_route::geometry,
    portals::{PairPart, partner},
};
use sims_vm::{Definition, Object, World, multitile::Part};
use std::{collections::BTreeMap, path::Path};

fn hex(v: &Value) -> Vec<u8> {
    let s = v.as_str().unwrap();
    assert!(s.len().is_multiple_of(2));
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn short(b: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(b[offset..offset + 2].try_into().unwrap())
}
fn word(b: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(b[offset..offset + 4].try_into().unwrap())
}
fn shorts(v: &Value) -> Vec<i16> {
    hex(v).chunks_exact(2).map(|b| short(b, 0)).collect()
}
fn read(p: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

#[test]
#[ignore = "private original paused-world and unchanged producer execution; fresh SIMS_ROUTE_WORLD_OUTPUT"]
fn actual_physical_grid_portals_and_all_room_rectangles_match_original_producers() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let configured =
        |name, default: &str| repo.join(std::env::var(name).unwrap_or_else(|_| default.into()));
    let source = configured(
        "SIMS_ROUTE_WORLD_SOURCE",
        "cache/native-route-world-20261022-fifth",
    );
    let native = configured(
        "SIMS_ROUTE_WORLD_ORACLE",
        "cache/native-route-world-oracle-20261022-fifth",
    );
    let output = repo.join(std::env::var("SIMS_ROUTE_WORLD_OUTPUT").unwrap());
    assert!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).unwrap();
    let captured = read(source.join("world.json"));
    let state = read(source.join("state.json"));
    let oracle = read(native.join("report.json"));
    let provenance = read(source.join("report.json"));
    assert_eq!(oracle["source_hashes"], provenance["files"]);
    for (name, expected) in provenance["files"].as_object().unwrap() {
        let bytes = std::fs::read(source.join(name)).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            expected.as_str().unwrap()
        );
    }
    assert_eq!(oracle["original_execution"], true);
    assert_eq!(oracle["supplied_stack_pointer"], false);
    assert_eq!(oracle["tick"], captured["tick"]);
    let objects = captured["objects"].as_array().unwrap();
    let ids: BTreeMap<u32, i16> = objects
        .iter()
        .map(|o| {
            (
                o["pointer"].as_u64().unwrap() as u32,
                o["id"].as_i64().unwrap() as i16,
            )
        })
        .collect();
    let metadata: BTreeMap<i16, Value> = read(native.join("object-inputs.json"))
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (o["id"].as_i64().unwrap() as i16, o.clone()))
        .collect();
    let resources: BTreeMap<u32, Value> = captured["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (o["pointer"].as_u64().unwrap() as u32, o.clone()))
        .collect();
    let size = captured["size"].as_u64().unwrap() as usize;
    let stride = captured["floors"][0]["primary"]["width"].as_u64().unwrap() as usize;
    assert_eq!(captured["floors"][0]["primary"]["height"], json!(stride));
    assert!(size <= stride);
    let index = |i: usize| i / size * stride + i % size;
    let mut world = World {
        map_size: Some(sims_vm::world_map::State {
            active_size: size as i32,
        }),
        ..Default::default()
    };
    // Unused architecture and VM fields are deliberately absent. This is a
    // borrowed producer check, not an import or simulated gameplay checkpoint.
    let mut architecture = Architecture {
        width: (stride * 16) as i32,
        height: (stride * 16) as i32,
        floors: vec![],
        outdoors: BTreeMap::new(),
        footprints: BTreeMap::new(),
        original: None,
    };
    let mut links = Links {
        size: stride,
        heads: captured["floors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| shorts(&f["heads"]["bytes"]))
            .collect(),
        next: BTreeMap::new(),
    };
    for o in state["objects"].as_array().unwrap() {
        let id = o["id"].as_i64().unwrap() as i16;
        let raw = objects.iter().find(|r| r["id"] == o["id"]).unwrap();
        let storage = hex(&raw["storage"]);
        let definition = hex(&o["definition"]);
        let guid = word(&definition, 28);
        let resource = &resources[&word(&storage, 0x11c)];
        assert!(!resource["functions"].is_null());
        let functions = hex(&resource["functions"]["entries"]);
        let function15 = functions.get(60..62).map_or(0, |b| short(b, 0) as u16);
        assert_eq!(json!(function15), metadata[&id]["function15"]);
        let footprint = Footprint {
            kind: short(&definition, 0x12),
            portal_style: short(&definition, 0x8a),
            base_graphic: short(&definition, 0x10),
            portal: function15 != 0,
            size: 0,
            insets: 0,
        };
        if footprint.kind == 8 {
            assert_eq!(
                metadata[&id]["portal_class"], true,
                "captured kind8 must satisfy the runtime producer's class invariant"
            );
        }
        if let Some(previous) = architecture.footprints.insert(guid, footprint.clone()) {
            assert_eq!(previous, footprint);
        }
        let slots: Vec<i16> = serde_json::from_value(raw["slot_ids"].clone()).unwrap();
        assert!(!slots.is_empty());
        links.next.insert(id, slots[0]);
        let part = (metadata[&id]["multipart"] == true).then(|| Part {
            head: (word(&storage, 0x190) != 0).then(|| ids[&word(&storage, 0x190)]),
            next: (word(&storage, 0x18c) != 0).then(|| ids[&word(&storage, 0x18c)]),
            origin: [0; 3],
            offset: [0; 3],
        });
        world.objects.insert(
            id,
            Object {
                data: (0..72)
                    .map(|n| (n, short(&storage, 0x4a + 2 * n as usize)))
                    .collect(),
                slots: slots[1..]
                    .iter()
                    .map(|&id| (id != 0).then_some(id))
                    .collect(),
                definition: Some(Definition {
                    source: "captured".into(),
                    object: 0,
                    guid,
                    slots: 0,
                }),
                multitile: part,
                ..Default::default()
            },
        );
    }
    let chains: Vec<Vec<Vec<i16>>> =
        serde_json::from_value(read(native.join("physical.json"))).unwrap();
    for (level, heads) in links.heads.iter().enumerate() {
        for tile in 0..size * size {
            let head = heads[index(tile)];
            let actual = if head > 0 && world.objects.contains_key(&head) {
                links.walk(&world, head).unwrap()
            } else {
                vec![]
            };
            assert_eq!(
                actual,
                chains[level][tile],
                "physical floor{} tile{tile}",
                level + 1
            );
        }
    }
    let parts: Vec<_> = world
        .objects
        .iter()
        .filter_map(|(&id, o)| {
            let p = o.multitile.as_ref()?;
            let f = &architecture.footprints[&o.definition.as_ref()?.guid];
            Some(PairPart {
                id,
                head: p.head,
                next: p.next,
                style: f.portal_style,
                base_graphic: f.base_graphic,
                portal: f.portal,
            })
        })
        .collect();
    let portals = read(native.join("partners.json"));
    for p in portals.as_array().unwrap() {
        let id = p["id"].as_i64().unwrap() as i16;
        assert_eq!(json!(partner(&parts, id)), p["partner"], "portal{id}");
    }
    let frozen = serde_json::to_vec(&world).unwrap();
    let occupants: Vec<_> = (1..=2)
        .map(|level| {
            Simulation::original_route_occupants(&world, &architecture, &links, level).unwrap()
        })
        .collect();
    assert_eq!(frozen, serde_json::to_vec(&world).unwrap());
    let water = hex(&captured["water"]["bytes"]);
    let ignore_marker = captured["process_settings"]["ignore_marker"]
        .as_u64()
        .unwrap()
        != 0;
    //This profile's captured native lot zoning is residential, the condition
    //which allows terrain-request1 to enable outdoor terrain cost generation.
    let zoning = hex(&captured["zoning"]);
    assert!(
        zoning
            .chunks_exact(8)
            .any(|b| word(b, 0) == 7 && word(b, 4) == 1)
    );
    //Only routing-read fields are projected from the actual captured storage.
    //Heights, pool masks and unused flag summaries are not inputs to this call.
    let mut lot = Lot {
        size: stride,
        active_size: Some(size),
        heights: vec![],
        pool: vec![],
        water,
        floors: captured["floors"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let walls: Vec<_> = hex(&f["walls"]["bytes"])
                    .chunks_exact(14)
                    .map(|b| WallTile {
                        segments: b[0],
                        flags: b[1],
                        styles: [short(b, 2) as u16, short(b, 4) as u16],
                        patterns: std::array::from_fn(|i| short(b, 6 + i * 2) as u16),
                    })
                    .collect();
                let flags = hex(&f["flags"]["bytes"]);
                let rooms = Partition {
                    primary: shorts(&f["primary"]["bytes"]),
                    halves: vec![],
                    room_tiles: captured["rooms"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|r| (r["id"].as_u64().unwrap() as usize >> 10) == i)
                        .map(|r| {
                            (
                                r["id"].as_i64().unwrap() as i16,
                                hex(&r["tiles"])
                                    .chunks_exact(3)
                                    .map(|p| [p[0] as usize, p[1] as usize, p[2] as usize])
                                    .collect(),
                            )
                        })
                        .collect(),
                };
                let metrics = metrics(&rooms, &walls, &flags, stride).unwrap();
                Floor {
                    source: FloorSource {
                        coverings: shorts(&f["coverings"]["bytes"])
                            .into_iter()
                            .map(|v| v as u16)
                            .collect(),
                        walls,
                        flags: flags.clone(),
                        object_heads: vec![],
                    },
                    rooms,
                    metrics,
                    flags: sims_sim::lot_flags::Rebuilt {
                        flags,
                        upper_contents: false,
                        upper_support: false,
                    },
                }
            })
            .collect(),
    };
    //The additional capture retains heights. Its routing-read objects, rooms
    //and arrays must agree exactly with the first accepted projection.
    let services_source = configured(
        "SIMS_ROUTE_CLEARANCE_SOURCE",
        "cache/native-route-world-20261022-sixth",
    );
    let services_native = configured(
        "SIMS_ROUTE_CLEARANCE_ORACLE",
        "cache/native-route-clearance-20261022-first",
    );
    let service_capture = read(services_source.join("world.json"));
    let service_report = read(services_native.join("report.json"));
    let service_provenance = read(services_source.join("report.json"));
    assert_eq!(service_report["source_hashes"], service_provenance["files"]);
    assert_eq!(service_report["original_execution"], true);
    assert_eq!(service_report["supplied_world_replies"], false);
    assert_eq!(service_report["supplied_floor_body"], true);
    for (name, hash) in service_provenance["files"].as_object().unwrap() {
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(std::fs::read(services_source.join(name)).unwrap())
            ),
            hash.as_str().unwrap()
        );
    }
    for field in [
        "tick",
        "size",
        "objects",
        "floors",
        "rooms",
        "resources",
        "process_settings",
    ] {
        assert_eq!(
            captured[field], service_capture[field],
            "changed captured {field}"
        );
    }
    lot.heights = hex(&service_capture["heights"]["bytes"])
        .chunks_exact(4)
        .map(|p| p.try_into().unwrap())
        .collect();
    use sims_sim::{
        lot_acceptance::person::Diagnostics,
        lot_route::candidate::{Check, Input, Reply, Request},
        spatial::PlacementResult,
    };
    let wall_cases = read(services_native.join("walls.json"));
    for (index, c) in wall_cases.as_array().unwrap().iter().enumerate() {
        let request = Request::Walls {
            tile: serde_json::from_value(c["tile"].clone()).unwrap(),
            rotation: c["rotation"].as_u64().unwrap() as u8,
        };
        let actual = Check::world_reply(request, &world, &links, &lot).unwrap();
        assert_eq!(
            actual,
            Some(Reply::Walls(c["mask"].as_u64().unwrap() as u8)),
            "wall case{index}: {c}"
        );
    }
    let clearance_cases = read(services_native.join("clearance.json"));
    let mut suspended = 0;
    for (index, c) in clearance_cases.as_array().unwrap().iter().enumerate() {
        let input = &c["input"];
        let actor = input["actor"].as_i64().unwrap() as i16;
        let target = input["target"].as_i64().unwrap() as i16;
        let raw = objects.iter().find(|o| o["id"] == json!(target)).unwrap();
        let target_raw = hex(&raw["storage"]);
        let raw_actor = hex(&objects.iter().find(|o| o["id"] == json!(actor)).unwrap()["storage"]);
        let direction = input["direction"].as_i64().unwrap();
        let mut check = Check::new(
            Input {
                size: world.map_size.unwrap().checked_size(stride).unwrap(),
                position: serde_json::from_value(input["position"].clone()).unwrap(),
                target: [
                    word(&target_raw, 0xf4) as i32,
                    word(&target_raw, 0xf8) as i32,
                ],
                level: word(&raw_actor, 0x110) as i32,
                room: world.objects[&actor].data[&29],
                flags: input["flags"].as_u64().unwrap() as u32,
                direction: (direction >= 0).then_some(direction as u8),
                ignored: None,
            },
            Diagnostics {
                error: 91,
                blocker: Some(actor),
            },
            77,
        )
        .unwrap();
        let mut events = vec![];
        while let Some(request) = check.next() {
            let mut restored: Check =
                serde_json::from_slice(&serde_json::to_vec(&check).unwrap()).unwrap();
            assert_eq!(restored.next(), Some(request));
            assert_eq!(restored, check);
            suspended += 1;
            let reply =
                if let Some(reply) = Check::world_reply(request, &world, &links, &lot).unwrap() {
                    if matches!(request, Request::Walls { .. }) {
                        events.push(json!(request));
                    }
                    reply
                } else {
                    assert!(matches!(request, Request::Placement { .. }));
                    events.push(json!(request));
                    let supplied = &input["floor_reply"];
                    let blocker = supplied["blocker"].as_i64().map(|v| v as i16);
                    Reply::Placement(PlacementResult {
                        allowed: supplied["accepted"].as_bool().unwrap(),
                        error: supplied["error"].as_i64().unwrap() as i32,
                        blocker: blocker.map(|id| {
                            (
                                id,
                                architecture.footprints
                                    [&world.objects[&id].definition.as_ref().unwrap().guid]
                                    .kind,
                            )
                        }),
                    })
                };
            check.resume(request, reply).unwrap();
            restored.resume(request, reply).unwrap();
            assert_eq!(restored, check);
        }
        let result = check.outcome().unwrap();
        let expected = &c["original"];
        assert_eq!(
            json!(result.classification),
            expected["classification"],
            "clearance{index}: {input}"
        );
        assert_eq!(
            json!(result.blocked_object),
            expected["blocked_object"],
            "clearance{index}"
        );
        assert_eq!(
            json!(result.diagnostics),
            expected["diagnostics"],
            "clearance{index}"
        );
        assert_eq!(json!(events), expected["events"], "clearance{index}");
    }
    assert_eq!(wall_cases.as_array().unwrap().len(), 32773);
    assert_eq!(clearance_cases.as_array().unwrap().len(), 992);
    assert_eq!(frozen, serde_json::to_vec(&world).unwrap());
    std::fs::write(output.join("clearance-report.json"),serde_json::to_vec_pretty(&json!({
        "wall_cases":32773,"clearance_cases":992,"suspended_query_roundtrips":suspended,
        "actual_world_replies":true,"virtual_floor_reply_supplied":true,"borrowed_world_unchanged":true,
        "strict_original_ticks_added":0,"source":services_source,"original":services_native,
        "scope":"Complete original boundary/wall/room/height gates and supplied virtual floor reply; not the floor callback body or full path wrapper."})).unwrap()).unwrap();
    let restored: World = serde_json::from_slice(&frozen).unwrap();
    assert_eq!(restored.map_size, world.map_size);
    let mut changed_global = restored;
    changed_global.globals.insert(23, 64);
    assert_eq!(
        changed_global
            .map_size
            .unwrap()
            .checked_size(stride)
            .unwrap(),
        size
    );
    let mut reports = vec![];
    for c in oracle["rectangles"].as_array().unwrap() {
        let room = c["room"].as_u64().unwrap() as i16;
        let level = ((room as u16 >> 10) & 3) as usize + 1;
        assert_eq!(
            json!(lot.floors[level - 1].metrics[&room].outdoors),
            c["outdoors"],
            "room{room} exterior"
        );
        let result = geometry::finalize(
            lot.route_obstacles_in(
                room,
                &occupants[level - 1],
                c["terrain"] == 1,
                ignore_marker,
                world.map_size.unwrap().checked_size(stride).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let matches = json!(result) == c["rectangles"];
        let row = json!({"room":room,"terrain":c["terrain"],"matches":matches,"actual":result,"original":c["rectangles"]});
        std::fs::write(
            output.join(format!("room-{room}-terrain-{}.json", c["terrain"])),
            serde_json::to_vec(&row).unwrap(),
        )
        .unwrap();
        assert!(
            matches,
            "room{room} terrain{} rectangles differ",
            c["terrain"]
        );
        reports.push(row);
    }
    assert_eq!(reports.len(), 10);
    assert_eq!(
        oracle["excluded_inactive_rooms"].as_array().unwrap().len(),
        3
    );
    let mut live_searches = vec![];
    let mut live_trace_hash = None;
    if let Ok(path) = std::env::var("SIMS_ROUTE_WORLD_WRAPPER_TRACE") {
        let bytes = std::fs::read(repo.join(&path)).unwrap();
        live_trace_hash = Some(format!("{:x}", Sha256::digest(&bytes)));
        let mut entry = None;
        let mut complete = false;
        for line in bytes.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
            let row: Value = serde_json::from_slice(line).unwrap();
            match row["phase"].as_str() {
                Some("wrapper_entry") => {
                    assert!(entry.replace(row.clone()).is_none(), "expected one wrapper");
                    let entry = entry.as_ref().unwrap();
                    assert_eq!(entry["world_inputs"]["files"], provenance["files"]);
                    assert_eq!(entry["tick"], captured["tick"]);
                }
                Some("search_entry") => {
                    let entry = entry.as_ref().expect("search before wrapper entry");
                    let actor = state["objects"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|o| o["id"] == entry["context"]["actor"])
                        .unwrap();
                    let room_bytes = hex(&row["room_bytes"]);
                    let room = short(&room_bytes, 0);
                    assert_eq!(actor["data"][29], json!(room));
                    //482802 bypasses the terrain-enabled retained cache only
                    //for a non-ground actor in the level-one outdoor room.
                    let terrain = !(actor["data"][17] != 0
                        && ((room as u16 >> 10) & 3) == 0
                        && room_bytes[0x72] != 0);
                    let produced = reports
                        .iter()
                        .find(|r| r["room"] == json!(room) && r["terrain"] == u8::from(terrain))
                        .expect("room producer absent");
                    let matches = produced["actual"] == row["fixed"]["values"];
                    live_searches.push(json!({"room":room,"terrain":terrain,
                        "matches":matches,"actual":produced["actual"],"original":row["fixed"]["values"]}));
                }
                _ => (),
            }
            if row["kind"] == "complete" {
                assert_eq!(row["reason"], "requested_wrapper_returns");
                assert_eq!(row["routes"], 1);
                complete = true;
            }
        }
        assert!(entry.is_some() && complete && !live_searches.is_empty());
        std::fs::write(
            output.join("live-fixed-vectors.json"),
            serde_json::to_vec_pretty(&json!({"trace":path,"sha256":live_trace_hash,
                "searches":live_searches,"strict_original_parity_ticks_added":0}))
            .unwrap(),
        )
        .unwrap();
        assert!(
            live_searches.iter().all(|r| r["matches"] == true),
            "production fixed vectors differ from live wrapper; report preserved"
        );
    }
    std::fs::write(output.join("report.json"),serde_json::to_vec_pretty(&json!({
        "source":source,"native":native,"physical_tiles":2*size*size,
        "portals":portals.as_array().unwrap().len(),"room_cases":reports.len(),
            "active_size":size,"backing_stride":stride,"production_room_active_bound_connected":true,
            "production_startup_resize_history_connected":false,"retained_map_serde_roundtrip":true,
            "retained_map_independent_of_global23":true,
            "borrowed_world_unchanged":true,"all_rectangles_match":true,"strict_original_parity_ticks_added":0,
            "live_fixed_searches":live_searches.len(),"live_trace_sha256":live_trace_hash,
            "scope":"Actual captured physical, definition and room inputs through retained-map room adapter; captured field supplied explicitly, earlier startup history and unused gameplay fields absent. Not an import or completed-tick comparison."})).unwrap()).unwrap();
}
