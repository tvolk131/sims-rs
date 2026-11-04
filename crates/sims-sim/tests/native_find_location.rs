use serde_json::{Value, json};
use sims_sim::{Point, placement::spiral};

#[cfg(feature = "private-fixtures")]
#[test]
fn random_search_matches_native_draws_filters_retries_and_final_recheck() {
    use sims_sim::placement::{RandomOrigin, SearchTile, Spiral, search_tile_allowed};
    let mut report: Value =
        serde_json::from_str(include_str!("fixtures/find-location-random.json")).unwrap();
    let unanchored: Value = serde_json::from_str(include_str!(
        "fixtures/find-location-random-unanchored.json"
    ))
    .unwrap();
    report["cases"]
        .as_array_mut()
        .unwrap()
        .extend(unanchored["cases"].as_array().unwrap().iter().cloned());
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let original: [i32; 2] = if c["flags"].as_u64().unwrap() & 1 == 0 {
            [16, 16]
        } else {
            serde_json::from_value(c["position"].clone()).unwrap()
        };
        let bounds = serde_json::from_value(c["bounds"].clone()).unwrap();
        let rotation = c["rotation"].as_i64().unwrap() as i32;
        let level = c["level"].as_i64().unwrap() as i32;
        let mut rng = c["seed"].as_u64().unwrap() as u32;
        let mut origin = RandomOrigin::new(original);
        let mut tiles = c["tiles"].as_array().unwrap().clone();
        let mut checks = vec![];
        let mut draws = vec![];
        let mut passes = vec![];
        let mut found = None;
        for pass in 0..2 {
            origin.reset();
            passes.push(
                json!({"position":original,"same_room":u8::from(pass==0),"draws":draws.len()}),
            );
            let center = loop {
                let mut shadow = rng;
                draws.push(sims_vm::rng::next(&mut shadow));
                draws.push(sims_vm::rng::next(&mut shadow));
                let result = origin.advance(8, bounds, rotation, &mut rng);
                assert_eq!(rng, shadow);
                // Exercise suspended random walks as well as candidate cursors.
                origin = serde_json::from_value(json!(&origin)).unwrap();
                assert!(
                    draws.len() <= row["original"]["draws"].as_array().unwrap().len(),
                    "excess draws: {c}"
                );
                if let Some(center) = result {
                    break center;
                }
            };
            let mut cursor = Spiral::new(8, center).unwrap();
            let [x, y] = cursor.center_tile();
            let room = tiles[x as usize * 8 + y as usize]["room"].as_i64().unwrap() as i16;
            for pos in &mut cursor {
                let t = &tiles[(pos[1] >> 4) as usize * 8 + (pos[0] >> 4) as usize];
                if !search_tile_allowed(
                    SearchTile {
                        segments: t["segments"].as_u64().unwrap() as u8,
                        head: t["head"].as_i64().unwrap() as i16,
                        room: t["room"].as_i64().unwrap() as i16,
                        pool: t["pool"].as_u64().unwrap() as u8,
                        ground_flags: t["ground_flags"].as_u64().unwrap() as u8,
                    },
                    level,
                    c["flags"].as_u64().unwrap() as u8,
                    (pass == 0).then_some(room),
                    false,
                ) {
                    continue;
                }
                checks.push(pos);
                if checks.len() == 1 && c["mutate_rooms"] == true {
                    for t in &mut tiles {
                        t["room"] = json!(7);
                    }
                }
                if checks.len() > c["reject"].as_u64().unwrap() as usize {
                    found = Some(pos);
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        if let Some(pos) = found {
            checks.push(pos);
            if c["fail_recheck"] == true {
                found = None;
            }
        }
        assert_eq!(json!(checks), row["original"]["checks"], "{c}");
        assert_eq!(json!(found), row["original"]["placed"], "{c}");
        assert_eq!(json!(draws), row["original"]["draws"], "{c}");
        assert_eq!(json!(passes), row["original"]["passes"], "{c}");
        assert_eq!(json!(rng), row["original"]["rng"], "{c}");
        assert_eq!(
            json!(u8::from(found.is_some())),
            row["original"]["returned"],
            "{c}"
        );
        assert_eq!(row["original"]["refreshed"], row["original"]["returned"]);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_post_placement_appearance_refresh() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/find-location-refresh.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 144);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let person = c["person"].as_bool().unwrap();
        let profile = if person {
            sims_sim::lot_find_location::refresh_pool_profile(
                8,
                [
                    c["position"][0].as_i64().unwrap() as i32,
                    c["position"][1].as_i64().unwrap() as i32,
                    c["level"].as_i64().unwrap() as i32,
                ],
                &vec![c["pool"].as_u64().unwrap() as u8; 64],
            )
            .unwrap()
        } else {
            7
        };
        assert_eq!(json!(profile), row["original"]["profile"], "{c}");
        assert_eq!(
            if person || c["view"] == true {
                json!([[0, 7]])
            } else {
                json!([])
            },
            row["original"]["calls"],
            "{c}"
        );
        assert!(row["original"]["error"].is_null());
    }
}

fn compare_filters(report: Value) {
    use sims_sim::placement::{SearchTile, Spiral, search_tile_allowed};
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let center = serde_json::from_value::<[i32; 2]>(c["position"].clone()).unwrap();
        let level = c["level"].as_i64().unwrap() as i32;
        let mut tiles = c["tiles"].as_array().unwrap().clone();
        let mut checks = vec![];
        let mut found = None;
        for pass in 0..2 {
            let mut cursor = Spiral::new(8, center).unwrap();
            let [x, y] = cursor.center_tile();
            let room = tiles[x as usize * 8 + y as usize]["room"].as_i64().unwrap() as i16;
            while let Some(pos) = cursor.next() {
                let saved: Spiral =
                    serde_json::from_value(serde_json::to_value(&cursor).unwrap()).unwrap();
                saved.validate(8, center).unwrap();
                cursor = saved;
                let v = &tiles[(pos[1] >> 4) as usize * 8 + (pos[0] >> 4) as usize];
                let tile = SearchTile {
                    segments: v["segments"].as_u64().unwrap() as u8,
                    head: v["head"].as_i64().unwrap() as i16,
                    room: v["room"].as_i64().unwrap() as i16,
                    pool: v["pool"].as_u64().unwrap() as u8,
                    ground_flags: v["ground_flags"].as_u64().unwrap() as u8,
                };
                if search_tile_allowed(
                    tile,
                    level,
                    c["flags"].as_u64().unwrap() as u8,
                    (pass == 0).then_some(room),
                    c["ignore_marker"].as_bool().unwrap(),
                ) {
                    checks.push(pos);
                    if checks.len() == 1 && c["mutate_rooms"] == true {
                        for t in &mut tiles {
                            t["room"] = json!(7);
                        }
                    }
                    if checks.len() > c["reject"].as_u64().unwrap() as usize {
                        found = Some(pos);
                        break;
                    }
                }
            }
            if found.is_some() {
                break;
            }
        }
        if let Some(pos) = found {
            checks.push(pos);
            if c["fail_recheck"] == true {
                found = None;
            }
        }
        assert_eq!(json!(checks), row["original"]["checks"], "{c}");
        assert_eq!(json!(found), row["original"]["placed"], "{c}");
        assert_eq!(
            json!(u8::from(found.is_some())),
            row["original"]["returned"],
            "{c}"
        );
        assert_eq!(
            row["original"]["refreshed"], row["original"]["returned"],
            "{c}"
        );
        assert!(row["original"]["error"].is_null());
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn filtered_native_search_and_failed_final_recheck() {
    compare_filters(
        serde_json::from_str(include_str!("fixtures/find-location-filters.json")).unwrap(),
    );
}
#[test]
#[ignore = "requires private native filtered-search corpus"]
fn full_filtered_native_search_corpus() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/find-location-filters.json");
    compare_filters(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_spiral_checks_fallback_and_recheck_order() {
    let report: Value = serde_json::from_str(include_str!("fixtures/find-location.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let center = Point::new(
            c["position"][0].as_i64().unwrap() as i32,
            c["position"][1].as_i64().unwrap() as i32,
        );
        let mut checks = vec![];
        let mut accept = |p: Point| {
            checks.push([p.x, p.y]);
            checks.len() > c["reject"].as_u64().unwrap() as usize
        };
        let mut found = spiral(8, center, &mut accept).unwrap();
        if found.is_none() {
            found = spiral(8, center, &mut accept).unwrap();
        }
        let found = found.filter(|p| accept(*p));
        assert_eq!(json!(checks), n["checks"], "{c}");
        assert_eq!(json!(found.map(|p| [p.x, p.y])), n["placed"], "{c}");
        assert_eq!(json!(u8::from(found.is_some())), n["returned"], "{c}");
        assert_eq!(n["returned"], n["refreshed"]);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn directional_search_matches_native_axes_signed_arithmetic_and_no_fallback() {
    use sims_sim::placement::Spiral;
    let report: Value = serde_json::from_str(include_str!(
        "../../sims-vm/tests/fixtures/find-direction.json"
    ))
    .unwrap();
    for row in report["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["original"]["error"].is_null())
    {
        let c = &row["input"];
        let n = &row["original"];
        let center: [i32; 2] = serde_json::from_value(c["position"].clone()).unwrap();
        let source = c["direction"].as_i64().unwrap() as i32;
        let direction = if c["mode"] == 4 {
            (source + 2) % 8
        } else {
            source
        };
        let mut cursor = Spiral::new_direction(8, center, direction).unwrap();
        let mut checks = vec![];
        let mut found = None;
        while let Some(pos) = cursor.next() {
            checks.push(pos);
            // Do not do quadratic reconstruction for 4096 repeated odd-direction checks.
            cursor = serde_json::from_value(json!(&cursor)).unwrap();
            if checks.len() > c["reject"].as_u64().unwrap() as usize {
                found = Some(pos);
                break;
            }
        }
        cursor.validate(8, center).unwrap();
        if let Some(pos) = found {
            checks.push(pos);
        }
        assert_eq!(json!(checks), n["checks"], "{c}");
        assert_eq!(json!(found), n["placed"], "{c}");
        assert_eq!(
            n["returned"].as_u64().unwrap() & 255,
            u64::from(found.is_some()),
            "{c}"
        );
        assert_eq!(n["refreshed"], 0);
    }
}
