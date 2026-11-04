use serde_json::{Value, json};
use sims_sim::lot_environment::{Totals, Tuning, daylight, score};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_room_light_models_and_projection() {
    use sims_sim::lot_environment::{
        Scores,
        manager::{Room, Settings},
    };
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-room-light-models.json")).unwrap();
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let count = c["count"].as_u64().unwrap() as usize;
        let mut room = Room {
            active: true,
            outdoors: false,
            occupied: false,
            pool: false,
            category8: false,
            tiles: vec![],
            totals: Totals::default(),
            scores: Scores {
                light: 0,
                environment: 0,
                published_light: 0,
            },
            models: vec![None; count],
            positions: vec![],
            route_rectangles: vec![],
        };
        let settings = Settings {
            daylight_multiplier: 1.,
            light_range: 12.,
            rooms_enabled: true,
            models: std::array::from_fn(|i| c["pool"][i].as_bool().unwrap().then_some(i as u8)),
        };
        room.prepare_light(
            settings,
            serde_json::from_value(c["position"].clone()).unwrap(),
            c["height"].as_f64().unwrap() as f32,
        );
        assert_eq!(
            json!(room.models.len()),
            row["original"]["count"],
            "model count {i}"
        );
        assert_eq!(
            json!(&room.models[count..]),
            row["original"]["appended"],
            "model references {i}"
        );
        assert_eq!(
            json!(room.models[count..].iter().flatten().collect::<Vec<_>>()),
            row["original"]["retained"],
            "retained {i}"
        );
        assert_eq!(
            json!(room.positions.iter().flatten().collect::<Vec<_>>()),
            row["original"]["position"],
            "light projection {i}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_dirty_room_manager_and_occupancy() {
    use sims_sim::lot_environment::{
        Aggregate,
        manager::{Manager, Room, Settings},
    };
    use sims_vm::{
        Vm, World,
        refresh::{RefreshState, RoomState},
    };
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-room-manager.json")).unwrap();
    let state = |m: &Manager, vm: &Vm| {
        let refresh = vm.world.refresh.as_ref().unwrap();
        let rooms: Vec<_> = m
            .rooms
            .iter()
            .map(|(&id, r)| {
                json!({
                    "id":id,"active":r.active,"occupied":r.occupied,"pool":r.pool,
                    "dirty":refresh.rooms[&id].dirty,"totals":r.totals,"scores":r.scores,
                })
            })
            .collect();
        json!({"rooms":rooms,"dirty":refresh.dirty_categories,
            "outdoor_sum":m.aggregate.outdoor_sum,"outdoor_environment":m.aggregate.outdoor_environment,
            "global27":m.aggregate.indoor_area as u32})
    };
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut vm = Vm::new(World::default(), 1);
        let mut refresh = RefreshState {
            dirty_categories: c["dirty"].as_u64().unwrap() as u32,
            ..Default::default()
        };
        let mut m = Manager {
            size: 8,
            tuning: Tuning::from_lookup(|_| None).unwrap(),
            settings: Settings {
                daylight_multiplier: 1.,
                light_range: 12.,
                rooms_enabled: true,
                models: [None; 6],
            },
            rooms: Default::default(),
            aggregate: Aggregate {
                indoor_area: 0,
                outdoor_sum: 0,
                outdoor_environment: 0,
                dirty_floors: 0,
            },
            light_bytes: vec![vec![0; 64]; 2],
            light_maps: vec![Default::default(); 2],
            ambient: 0,
        };
        for r in c["rooms"].as_array().unwrap() {
            let id = r["id"].as_u64().unwrap() as u16;
            refresh.rooms.insert(
                id,
                RoomState {
                    category: (id >> 10) as u8,
                    dirty: r["dirty"].as_bool().unwrap(),
                },
            );
            m.rooms.insert(
                id,
                Room {
                    active: r["active"].as_bool().unwrap(),
                    outdoors: r["outdoors"].as_bool().unwrap(),
                    occupied: r["occupied"].as_bool().unwrap(),
                    pool: r["pool"].as_bool().unwrap(),
                    category8: false,
                    tiles: serde_json::from_value(r["tiles"].clone()).unwrap(),
                    totals: serde_json::from_value(r["totals"].clone()).unwrap(),
                    scores: serde_json::from_value(r["scores"].clone()).unwrap(),
                    models: vec![],
                    positions: vec![],
                    route_rectangles: vec![],
                },
            );
        }
        vm.world.refresh = Some(refresh);
        let mut visits = vec![];
        m.rebuild_with(
            &mut vm,
            c["period"].as_i64().unwrap() as i16,
            c["floor"].as_i64().unwrap() as i32,
            |id, room, _, _| {
                visits.push(id);
                let input = c["rooms"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["id"] == id)
                    .unwrap();
                room.pool = input["next_pool"].as_bool().unwrap();
                let mut counts = input["contribution"].clone();
                for v in counts.as_object_mut().unwrap().values_mut() {
                    *v = json!((v.as_i64().unwrap() as i32).wrapping_mul(room.tiles.len() as i32));
                }
                room.totals = serde_json::from_value(counts).unwrap();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            json!(visits),
            row["original"]["visits"],
            "rebuild visits {n}"
        );
        assert_eq!(state(&m, &vm), row["original"]["rebuilt"], "rebuilt {n}");
        assert_eq!(vm.world.globals[&27], m.aggregate.indoor_area as i16);
        for (i, operation) in c["occupancy"].as_array().unwrap().iter().enumerate() {
            let index = operation[0].as_u64().unwrap() as usize;
            let id = c["rooms"][index]["id"].as_i64().unwrap() as i16;
            m.occupied(&mut vm, id, operation[1].as_bool().unwrap())
                .unwrap();
            assert_eq!(
                state(&m, &vm),
                row["original"]["operations"][i],
                "occupancy {n}/{i}"
            );
            let restored: Manager =
                serde_json::from_slice(&serde_json::to_vec(&m).unwrap()).unwrap();
            assert_eq!(m, restored);
            m = restored;
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_live_opening_invalidates_active_rooms_without_rebuilding() {
    use sims_sim::lot_environment::{
        Aggregate,
        manager::{Manager, Room, Settings},
    };
    use sims_vm::{
        Vm, World,
        refresh::{RefreshState, RoomState},
    };
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-room-invalidation.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut vm = Vm::new(World::default(), 37);
        let mut refresh = RefreshState {
            dirty_categories: c["dirty_categories"].as_u64().unwrap() as u32,
            ..Default::default()
        };
        let mut manager = Manager {
            size: 8,
            tuning: Tuning::from_lookup(|_| None).unwrap(),
            settings: Settings {
                daylight_multiplier: 1.,
                light_range: 12.,
                rooms_enabled: true,
                models: [None; 6],
            },
            rooms: Default::default(),
            aggregate: Aggregate {
                indoor_area: 19,
                outdoor_sum: 23,
                outdoor_environment: 29,
                dirty_floors: 0,
            },
            light_bytes: vec![vec![17; 64]; 2],
            light_maps: vec![Default::default(); 2],
            ambient: 31,
        };
        for r in c["rooms"].as_array().unwrap() {
            let id = r["id"].as_u64().unwrap() as u16;
            refresh.rooms.insert(
                id,
                RoomState {
                    category: (id >> 10) as u8,
                    dirty: r["dirty"].as_bool().unwrap(),
                },
            );
            manager.rooms.insert(
                id,
                Room {
                    active: r["active"].as_bool().unwrap(),
                    outdoors: true,
                    occupied: false,
                    pool: false,
                    category8: false,
                    tiles: vec![],
                    totals: Totals::default(),
                    scores: sims_sim::lot_environment::Scores {
                        light: 1f32.to_bits(),
                        environment: (-59f32).to_bits(),
                        published_light: 1f32.to_bits(),
                    },
                    models: vec![],
                    positions: vec![],
                    route_rectangles: vec![],
                },
            );
        }
        vm.world.refresh = Some(refresh);
        let before = serde_json::to_value(&vm).unwrap();
        let original_manager = manager.clone();
        manager.invalidate_active_rooms(&mut vm).unwrap();
        assert_eq!(
            manager, original_manager,
            "invalidation must not rebuild cached scores"
        );
        let refresh = vm.world.refresh.as_ref().unwrap();
        assert_eq!(
            json!(refresh.dirty_categories),
            row["original"]["dirty_categories"]
        );
        for room in row["original"]["rooms"].as_array().unwrap() {
            assert_eq!(
                json!(refresh.rooms[&(room["id"].as_u64().unwrap() as u16)].dirty),
                room["result_dirty"]
            );
        }
        let mut after = serde_json::to_value(&vm).unwrap();
        after["world"]["refresh"] = before["world"]["refresh"].clone();
        assert_eq!(
            after, before,
            "room invalidation changed VM processing state"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_room_light_maps_and_byte_publication() {
    use sims_sim::lot_environment::lighting::{Map, publish};
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-room-lighting.json")).unwrap();
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let size = c["size"].as_u64().unwrap() as usize;
        let rooms = serde_json::from_value(c["rooms"].clone()).unwrap();
        let mut map = Map::default();
        for entry in c["initial"].as_array().unwrap() {
            map.cells[entry[0].as_u64().unwrap() as usize] =
                serde_json::from_value(entry[1].clone()).unwrap();
        }
        for (j, op) in c["ops"].as_array().unwrap().iter().enumerate() {
            map.update(
                size,
                &rooms,
                serde_json::from_value(op["tile"].clone()).unwrap(),
                op["range"].as_f64().unwrap() as f32,
                op["draw"].as_bool().unwrap(),
                c["enabled"].as_bool().unwrap(),
            )
            .unwrap();
            let actual: Vec<_> = map
                .cells
                .iter()
                .enumerate()
                .filter(|(_, v)| **v != [127; 4])
                .collect();
            assert_eq!(json!(actual), row["original"][j], "map {i} step {j}");
            let restored = serde_json::from_slice(&serde_json::to_vec(&map).unwrap()).unwrap();
            assert_eq!(map, restored);
            map = restored;
        }
    }
    for (i, row) in report["publication"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let size = c["size"].as_u64().unwrap() as usize;
        let tiles: Vec<[usize; 3]> = serde_json::from_value(c["tiles"].clone()).unwrap();
        let mut bytes = vec![vec![19; size * size]];
        publish(
            &mut bytes,
            size,
            &tiles,
            c["light"].as_f64().unwrap() as f32,
        )
        .unwrap();
        assert_eq!(json!(bytes[0]), row["original"], "publication {i}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_room_tuning_and_environment_scores() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-room-environment.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let tuning =
            Tuning::from_lookup(|name| c["lookup"][name].as_f64().map(|v| v as f32)).unwrap();
        let bits: std::collections::BTreeMap<_, _> = serde_json::to_value(&tuning)
            .unwrap()
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), (v.as_f64().unwrap() as f32).to_bits()))
            .collect();
        assert_eq!(json!(bits), row["original"]["tuning"], "case {i} tuning");
        let totals: Totals = serde_json::from_value(c["totals"].clone()).unwrap();
        let restored: (Tuning, Totals) =
            serde_json::from_slice(&serde_json::to_vec(&(&tuning, &totals)).unwrap()).unwrap();
        assert_eq!(restored, (tuning.clone(), totals.clone()));
        let result = score(
            &restored.1,
            &restored.0,
            c["outdoors"].as_bool().unwrap(),
            c["pool"].as_bool().unwrap(),
            c["occupied_dark"].as_bool().unwrap(),
            daylight(
                c["period"].as_i64().unwrap() as i16,
                c["daylight_multiplier"].as_f64().unwrap() as f32,
            ),
        );
        assert_eq!(json!(result), row["original"]["scores"], "case {i} score");
    }
    eprintln!(
        "Checked {} original room tuning/scoring cases",
        report["cases"].as_array().unwrap().len()
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_room_floor_and_wall_contribution_sequences() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-room-contributions.json")).unwrap();
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut totals: Totals = serde_json::from_value(c["initial"].clone()).unwrap();
        for (j, tile) in c["tiles"].as_array().unwrap().iter().enumerate() {
            let wall = serde_json::from_value(tile["wall"].clone()).unwrap();
            totals
                .add_tile(
                    c["outdoors"].as_bool().unwrap(),
                    tile["ground"].as_u64().unwrap() as u8,
                    tile["floor"].as_u64().unwrap() as u16,
                    &wall,
                )
                .unwrap();
            assert_eq!(json!(totals), row["original"][j], "sequence {i} tile {j}");
            let restored = serde_json::from_slice(&serde_json::to_vec(&totals).unwrap()).unwrap();
            assert_eq!(totals, restored);
            totals = restored;
        }
    }
    eprintln!(
        "Checked {} original tile contribution sequences",
        report["cases"].as_array().unwrap().len()
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_room_manager_aggregation_and_lookup() {
    use sims_sim::lot_environment::{RoomScore, aggregate, environment};
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-room-aggregate.json")).unwrap();
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let rooms: Vec<RoomScore> = serde_json::from_value(c["rooms"].clone()).unwrap();
        let tuning =
            Tuning::from_lookup(|name| c["lookup"][name].as_f64().map(|v| v as f32)).unwrap();
        let result = aggregate(
            &rooms,
            &tuning,
            c["period"].as_i64().unwrap() as i16,
            c["dirty"].as_u64().unwrap() as u32,
            c["floor"].as_i64().unwrap() as i32,
        );
        assert_eq!(
            json!(result),
            row["original"]["aggregate"],
            "case {i} aggregate"
        );
        for query in row["original"]["queries"].as_array().unwrap() {
            assert_eq!(
                json!(environment(
                    &rooms,
                    &result,
                    query[0].as_i64().unwrap() as i16
                )),
                query[1],
                "case {i} query {query}"
            );
        }
    }
    eprintln!(
        "Checked {} native room aggregate/lookup cases",
        report["cases"].as_array().unwrap().len()
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_room_object_contributions_resume_across_live_callbacks() {
    use sims_sim::lot_environment::objects::{Execution, Input, Request, Response};
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-room-objects.json")).unwrap();
    let mut services = 0;
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut totals: Totals = serde_json::from_value(c["totals"].clone()).unwrap();
        let mut object: Input = serde_json::from_value(c["object"].clone()).unwrap();
        let mut category8 = c["category8"].as_bool().unwrap();
        let mut e = Execution::new(
            c["room"].as_i64().unwrap() as i16,
            c["outdoors"].as_bool().unwrap(),
        );
        for (j, event) in row["original"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let request = e.next(&mut totals, &mut category8, &object).unwrap();
            assert_eq!(json!(request), event["request"], "case {i} service {j}");
            assert_eq!(
                json!({"totals":totals,"category8":category8}),
                event["state"],
                "case {i} state {j}"
            );
            let saved = (e.clone(), totals.clone(), category8, object.clone());
            let bytes = serde_json::to_vec(&saved).unwrap();
            let restored = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(saved, restored);
            (e, totals, category8, object) = restored;
            assert_eq!(
                e.next(&mut totals, &mut category8, &object),
                Some(request.clone())
            );
            assert!(
                e.acknowledge(if request == Request::Location {
                    Response::Done
                } else {
                    Response::Located(false)
                })
                .is_err()
            );
            assert_eq!(
                saved,
                (e.clone(), totals.clone(), category8, object.clone())
            );
            let key = if let Some(name) = event["request"].as_str() {
                name
            } else {
                event["request"].as_object().unwrap().keys().next().unwrap()
            };
            if let Some(patch) = c["patches"][key].as_object() {
                let mut value = json!(object);
                for (key, v) in patch {
                    value[key] = v.clone();
                }
                object = serde_json::from_value(value).unwrap();
            }
            e.acknowledge(if request == Request::Location {
                Response::Located(c["on_lot"].as_bool().unwrap())
            } else {
                Response::Done
            })
            .unwrap();
            services += 1;
        }
        assert!(e.next(&mut totals, &mut category8, &object).is_none());
        assert_eq!(
            json!({"totals":totals,"category8":category8}),
            row["original"]["state"],
            "case {i} final"
        );
    }
    eprintln!(
        "Checked {} original room object visits with restoration at {services} services",
        report["cases"].as_array().unwrap().len()
    );
}
