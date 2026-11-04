use serde_json::{Value, json};
use sims_sim::{
    lot_acceptance::person::Diagnostics,
    lot_route::candidate::{Check, Input, Reply, Request},
    spatial::PlacementResult,
};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_clearance_order_diagnostics_and_resume() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/candidate-clearance.json")).unwrap();
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let n = |key: &str| c[key].as_i64().unwrap();
        let b = |key: &str| c[key].as_bool().unwrap();
        let input = Input {
            size: 16,
            position: serde_json::from_value(c["position"].clone()).unwrap(),
            target: [80, 80],
            level: 1,
            room: 1,
            flags: n("flags") as u32,
            direction: (n("direction") != -1).then_some(n("direction") as u8),
            ignored: b("ignore").then_some(2),
        };
        let mut check = Check::new(
            input,
            Diagnostics {
                error: 91,
                blocker: Some(2),
            },
            77,
        )
        .unwrap();
        let mut events = vec![];
        while let Some(request) = check.next() {
            let checkpoint = serde_json::to_vec(&check).unwrap();
            let mut restored: Check = serde_json::from_slice(&checkpoint).unwrap();
            assert_eq!(restored.next(), Some(request));
            assert_eq!(restored, check, "pending query repeated a side effect");
            let mut invalid = restored.clone();
            let wrong = match request {
                Request::Room(_) => Reply::Walls(0),
                _ => Reply::Room(0),
            };
            assert!(invalid.resume(request, wrong).is_err());
            assert_eq!(invalid, restored, "wrong reply mutated state");
            let reply = match request {
                Request::Room(tile) => {
                    assert_eq!(
                        tile,
                        [
                            ((n_position(c, 1) >> 4) as i8),
                            ((n_position(c, 0) >> 4) as i8),
                            1
                        ]
                    );
                    Reply::Room(n("room") as i16)
                }
                Request::Walls { .. } => {
                    events.push(serde_json::to_value(request).unwrap());
                    Reply::Walls(n("walls") as u8)
                }
                Request::Heights { .. } => Reply::Heights {
                    candidate: (n("height") as u32).to_le_bytes(),
                    target: (n("target_height") as u32).to_le_bytes(),
                },
                Request::Placement { .. } => {
                    assert_eq!(
                        check.diagnostics(),
                        Diagnostics {
                            error: 91,
                            blocker: None
                        }
                    );
                    events.push(serde_json::to_value(request).unwrap());
                    Reply::Placement(PlacementResult {
                        allowed: b("placement"),
                        error: n("placement_error") as i32,
                        blocker: b("blocker").then_some((2, n("blocker_kind") as i16)),
                    })
                }
            };
            check.resume(request, reply).unwrap();
            restored.resume(request, reply).unwrap();
            assert_eq!(restored, check);
        }
        let result = check.outcome().unwrap();
        let actual = json!({
            "result":result.classification,"blocker":result.blocked_object,
            "placement_error":result.diagnostics.error,"placement_blocker":result.diagnostics.blocker,
            "events":events,"error":null,
        });
        assert_eq!(actual, row["original"], "case {i}: {c}");
    }
}
fn n_position(c: &Value, i: usize) -> i32 {
    c["position"][i].as_i64().unwrap() as i32
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_object_wall_mask_words_and_shift_counts() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/candidate-clearance.json")).unwrap();
    for c in report["object_masks"].as_array().unwrap() {
        let actual = sims_sim::spatial::object_wall_mask(
            c["mask"].as_i64().unwrap() as i16,
            c["facing"].as_i64().unwrap() as i16,
        );
        assert_eq!(json!(actual), c["original"], "{c}");
    }
}

#[test]
fn wall_query_obeys_active_border_before_reading_backing_cells() {
    use sims_sim::{
        lot_geometry::{FloorSource, Lot},
        lot_links::Links,
    };
    use sims_vm::{World, world_map::State};
    let backing = 8;
    let count = backing * backing;
    let lot = Lot::rebuild(
        backing,
        vec![[0; 4]; count],
        vec![255; count],
        vec![255; count],
        vec![FloorSource {
            walls: vec![
                sims_formats::lot::WallTile {
                    segments: 15,
                    ..Default::default()
                };
                count
            ],
            coverings: vec![0; count],
            object_heads: vec![0; count],
            flags: vec![0; count],
        }],
    )
    .unwrap();
    let links = Links {
        size: backing,
        heads: vec![vec![0; count]],
        next: Default::default(),
    };
    let world = World {
        map_size: Some(State { active_size: 4 }),
        ..Default::default()
    };
    let query = |w: &World, x, y| {
        Check::world_reply(
            Request::Walls {
                tile: [x, y, 1],
                rotation: 0,
            },
            w,
            &links,
            &lot,
        )
        .unwrap()
    };
    assert_ne!(query(&world, 2, 2), Some(Reply::Walls(0)));
    for (x, y) in [(0, 2), (2, 0), (3, 2), (2, 3), (6, 6), (-1, 1), (127, 1)] {
        assert_eq!(query(&world, x, y), Some(Reply::Walls(0)), "{x},{y}");
    }
    //Unknown historical inferred sessions retain their original allocation extent.
    assert_ne!(query(&World::default(), 6, 6), Some(Reply::Walls(0)));
}
