use serde_json::{Value, json};
use sims_sim::{
    lot_route::search::{Config, Input, goals},
    routing::{Candidate, CandidateSet},
};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_room_policy_budget_preliminary_tail_and_goal_construction() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-route-search-policy.json")).unwrap();
    assert_eq!(report["original_execution"], true);
    assert_eq!(report["policy_cases"].as_array().unwrap().len(), 1880);
    for (index, row) in report["policy_cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let input: Input = serde_json::from_value(row["input"].clone()).unwrap();
        let config = Config::prepare(input);
        let expected = &row["original"];
        assert_eq!(json!(config.bounded), expected["bounded"], "{index}");
        assert_eq!(json!(config.maximum), expected["maximum"], "{index}");
        assert_eq!(json!(config.free_cost), expected["free_cost"], "{index}");
        assert_eq!(json!(config.limit), expected["limit"], "{index}");
        assert_eq!(
            json!(config.preliminary),
            expected["preliminary"],
            "{index}"
        );
        if config.preliminary {
            let points = row["input"]["preliminary_points"].as_u64().unwrap() as usize;
            let result = config.preliminary_result(points);
            assert_eq!(
                json!(result.is_some()),
                expected["tail"]["failed"],
                "{index}"
            );
            assert_eq!(
                json!(result.unwrap_or(0)),
                expected["tail"]["wrapper_state"],
                "{index}"
            );
            assert_eq!(
                expected["tail"]["reset_bounded_bytes"],
                json!([1]),
                "{index}"
            );
        }
        assert_eq!(
            config.steps(),
            if config.limit < 0 {
                0
            } else {
                config.limit as usize + 1
            }
        );
    }
    assert_eq!(report["goal_cases"].as_array().unwrap().len(), 160);
    for (index, row) in report["goal_cases"].as_array().unwrap().iter().enumerate() {
        let candidates: Vec<[i32; 4]> =
            serde_json::from_value(row["input"]["candidates"].clone()).unwrap();
        let set = CandidateSet {
            candidates: candidates
                .iter()
                .map(|c| Candidate {
                    position: [c[0], c[1]],
                    score: c[2],
                    seat: 0,
                    flags: 0,
                })
                .collect(),
            maximum: row["input"]["maximum"].as_i64().unwrap() as i32,
            result: 0,
            actor_facing: 0,
        };
        let (actual, indices) = goals(&set);
        assert_eq!(json!(actual), row["original"]["goals"], "{index}");
        assert_eq!(json!(indices), row["original"]["indices"], "{index}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_lazy_fixed_rectangles_reuse_empty_retry_and_room_reset() {
    use sims_sim::{
        lot_environment::{Scores, Totals, manager::Room},
        regions::Region,
    };
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-route-search-policy.json")).unwrap();
    assert_eq!(report["cache_cases"].as_array().unwrap().len(), 24);
    for (i, row) in report["cache_cases"].as_array().unwrap().iter().enumerate() {
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
            models: vec![],
            positions: vec![],
            route_rectangles: serde_json::from_value(row["input"]["initial"].clone()).unwrap(),
        };
        let built: Vec<Region> = serde_json::from_value(row["input"]["built"].clone()).unwrap();
        let mut calls = vec![];
        let mut queries = vec![];
        for n in 0..3 {
            if n == 2 {
                room.reset();
                assert!(room.route_rectangles.is_empty());
            }
            queries.push(
                room.route_rectangles_with(|| {
                    calls.push(json!({"room":row["input"]["room"],"terrain":1}));
                    Ok(built.clone())
                })
                .unwrap()
                .to_vec(),
            );
            room = serde_json::from_slice(&serde_json::to_vec(&room).unwrap()).unwrap();
        }
        assert_eq!(json!(queries), row["original"]["queries"], "{i}");
        assert_eq!(json!(calls), row["original"]["calls"], "{i}");
        assert_eq!(row["original"]["after_reset"], json!([]), "{i}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_person_path_is_cleared_before_entering_the_wrapper() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-route-search-policy.json")).unwrap();
    let cases = report["caller_clear_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 4);
    for row in cases {
        let o = &row["original"];
        for field in [
            "points_cleared",
            "storage_preserved",
            "context_preserved",
            "output_is_person_path",
            "old_storage_bytes_preserved",
        ] {
            assert_eq!(o[field], true, "{field}");
        }
    }
}

#[test]
fn empty_final_search_preserves_result_three_without_preliminary_pass() {
    use sims_sim::{Point, regions::Region};
    //The full unchanged wrapper's small-room failure records result3 even
    //when there was no preliminary pass (see native-full-route-wrapper probe).
    let config = Config::prepare(Input {
        half_tiles: 100,
        objects: 144,
        special: false,
        ground: true,
        level: 1,
        outdoors: true,
        positive_goals: 1,
    });
    assert!(!config.preliminary);
    let fixed = [
        Region([-100, -100, 100, 0, i32::MAX]),
        Region([-100, 64, 100, 100, i32::MAX]),
        Region([-100, 0, 0, 64, i32::MAX]),
        Region([64, 0, 100, 64, i32::MAX]),
    ];
    let outcome = config
        .run(
            Point::new(8, 8),
            &[Region([-1000, -1000, -999, -999, 0])],
            &[],
            &fixed,
        )
        .unwrap();
    assert!(outcome.preliminary.is_none());
    assert!(outcome.search.as_ref().unwrap().points.is_empty());
    assert_eq!(outcome.failure_result, Some(3));
}

#[test]
#[ignore = "private unchanged-instruction blocked search corpus"]
fn blocked_preliminary_search_preserves_original_result_and_skips_final_pass() {
    use sims_sim::{Point, regions::Region};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/native-route-preliminary-failures-20261022.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["original_execution"], true);
    assert_eq!(report["supplied_stack_pointer"], false);
    assert_eq!(report["cases"].as_array().unwrap().len(), 24);
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let config = Config::prepare(serde_json::from_value(c["room_policy"].clone()).unwrap());
        assert!(config.preliminary);
        assert_eq!(json!(config.steps()), c["limit"]);
        let vector = |name: &str| serde_json::from_value::<Vec<Region>>(c[name].clone()).unwrap();
        let outcome = config
            .run(
                Point::new(
                    c["start"][0].as_i64().unwrap() as i32,
                    c["start"][1].as_i64().unwrap() as i32,
                ),
                &vector("goals"),
                &vector("dynamic"),
                &vector("obstacles"),
            )
            .unwrap();
        assert_eq!(outcome.failure_result, Some(3), "{i}");
        assert!(outcome.search.is_none(), "{i}: final pass must not run");
        let result = outcome.preliminary.unwrap();
        let actual = json!({"goal":result.goal.map_or(-1,|n|n as i64),
            "path":result.points.iter().map(|p|[p.x,p.y]).collect::<Vec<_>>(),
            "rectangles":result.regions,"iterations":result.iterations});
        let o = &row["original"];
        assert_eq!(
            actual,
            json!({"goal":o["goal"],"path":o["path"],"rectangles":o["rectangles"],"iterations":o["iterations"]}),
            "{i}"
        );
    }
}
