//! Original executable outputs on synthetic records; no game content required.
#[allow(dead_code)]
#[path = "../examples/compare_native_routing.rs"]
mod comparison;

#[cfg(feature = "private-fixtures")]
#[test]
fn original_endpoint_look_uses_path_and_descriptor() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/route-look.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let path: Vec<[i32; 2]> = serde_json::from_value(c["path"].clone()).unwrap();
        let look = sims_sim::routing::endpoint_look(
            &path,
            serde_json::from_value(c["target"].clone()).unwrap(),
            c["rotation"].as_u64().unwrap() as u8,
            c["flags"].as_u64().unwrap() as u32,
            c["facing"].as_i64().unwrap() as i32,
            c["actor_facing"].as_u64().unwrap() as u8,
        )
        .unwrap();
        assert_eq!(serde_json::json!(look), row["original"]["look"], "{c}");
        assert_eq!(
            if look.is_some() { 2 } else { 0 },
            row["original"]["returned"].as_u64().unwrap()
        );
        assert_eq!(row["original"]["entry_flags"], c["entry_flags"]);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_room_gate_runs_before_destination_preparation() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/route-room-gate.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        assert_eq!(
            sims_sim::routing::needs_portal(
                c["actor_room"].as_i64().unwrap() as i16,
                c["target_room"].as_i64().unwrap() as i16,
                c["person68"].as_i64().unwrap() as i16
            ),
            row["portal"].as_bool().unwrap(),
            "{c}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn standing_scan_suspends_without_reordering_original_callback_effects() {
    use serde_json::{Value, json};
    use sims_sim::routing::{StandingRequest, StandingScan};
    for (wrapper, source) in [
        (false, include_str!("fixtures/standing-callbacks.json")),
        (true, include_str!("fixtures/candidate-wrapper.json")),
    ] {
        let report: Value = serde_json::from_str(source).unwrap();
        for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
            let input = &row["input"];
            let request: StandingRequest = serde_json::from_value(input.clone()).unwrap();
            let mut scan = StandingScan::new(request).unwrap();
            let mut rng = input["seed"].as_u64().unwrap() as u32;
            let mut checks = vec![];
            while let Some(check) = scan.next(&mut rng).unwrap() {
                scan.validate().unwrap();
                let mut resumed: StandingScan =
                    serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
                let before = rng;
                assert_eq!(resumed.next(&mut rng).unwrap(), Some(check));
                assert_eq!(before, rng, "replaying a pending request drew randomness");
                let index = checks.len();
                let classification =
                    [6, 6, 5, 3, 6, 4, 1][(index + input["phase"].as_u64().unwrap() as usize) % 7];
                let after_rng = rng ^ (index as u32 + 1).wrapping_mul(0x9e3779b9);
                let after_facing = (check.actor_facing + 3) & 7;
                checks.push(json!({"point":check.position,"direction":check.direction,
                "classification":classification,"rng":rng,"actor_facing":check.actor_facing,
                "after_rng":after_rng,"after_facing":after_facing}));
                for s in [&mut scan, &mut resumed] {
                    s.resume(check, classification).unwrap();
                    s.observe_actor_facing(after_facing).unwrap();
                }
                assert_eq!(resumed, scan);
                scan = serde_json::from_slice(&serde_json::to_vec(&resumed).unwrap()).unwrap();
                rng = after_rng;
            }
            scan.validate().unwrap();
            let mut set = scan.outcome().unwrap().clone();
            let limit =
                if input["room_exists"] == true && input["room_tiles"].as_i64().unwrap() > 250 {
                    8
                } else {
                    48
                };
            if wrapper {
                set = sims_sim::routing::finish_candidate_scan(
                    set,
                    limit,
                    &mut rng,
                    input["actor_facing"].as_u64().unwrap() as u8,
                    input["retained_result"].as_u64().unwrap_or(0) as u8,
                );
            }
            let mut actual = serde_json::to_value(set).unwrap();
            if wrapper {
                actual["ignored_after"] = json!(0);
                actual["limit_after"] = json!(limit);
            }
            actual["rng"] = json!(rng);
            actual["checks"] = json!(checks);
            actual["error"] = Value::Null;
            assert_eq!(actual, row["original"], "native callback case {i}");
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_candidates_preserve_previous_retry_result_on_success() {
    use sims_sim::routing::{CandidateSet, PointCandidate, StandingRequest};
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/route-retained-result.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let r: StandingRequest = serde_json::from_value(c.clone()).unwrap();
        let expected: CandidateSet = serde_json::from_value(row["original"].clone()).unwrap();
        let classification = c["classification"][0].as_u64().unwrap() as u8;
        let mut rng = c["seed"].as_u64().unwrap() as u32;
        let core = if r.flags & 255 == 0 {
            PointCandidate::prepare(r.target, r.offset, r.rotation, r.flags)
                .unwrap()
                .finish(classification, 0)
                .unwrap()
        } else {
            sims_sim::routing::standing_candidates(&r, &mut rng, |_, _| i32::from(classification))
                .unwrap()
        };
        let actual = sims_sim::routing::finish_candidate_scan(
            core,
            48,
            &mut rng,
            expected.actor_facing,
            c["retained_result"].as_u64().unwrap() as u8,
        );
        assert_eq!(actual, expected, "{row}");
        assert_eq!(rng, row["original"]["rng"].as_u64().unwrap() as u32);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_exact_point_candidates() {
    use serde_json::{Value, json};
    use sims_sim::routing::PointCandidate;
    let report: Value =
        serde_json::from_str(include_str!("fixtures/point-candidates.json")).unwrap();
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let point = PointCandidate::prepare(
            serde_json::from_value(c["target"].clone()).unwrap(),
            serde_json::from_value(c["offset"].clone()).unwrap(),
            c["rotation"].as_u64().unwrap() as u8,
            c["flags"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let classification = c["classification"][usize::from(point.direction.unwrap_or(7))]
            .as_u64()
            .unwrap() as u8;
        let retained = c["retained_result"].as_u64().unwrap_or(0) as u8;
        let mut actual =
            serde_json::to_value(point.finish(classification, retained).unwrap()).unwrap();
        actual["checks"] = json!([{
            "point":point.position,
            "direction":point.direction.map(u32::from).unwrap_or(u32::MAX),
            "classification":classification,
        }]);
        actual["rng"] = c["seed"].clone();
        actual["error"] = Value::Null;
        assert_eq!(actual, row["original"], "case {i}: {c}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_domestic_seated_candidates() {
    use serde_json::{Value, json};
    use sims_sim::routing::{self, Seat};
    let report: Value = serde_json::from_str(include_str!("fixtures/domestic-seats.json")).unwrap();
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let request: routing::StandingRequest = serde_json::from_value(c.clone()).unwrap();
        let mut rng = c["seed"].as_u64().unwrap() as u32;
        let mut checks = vec![];
        let (set, blocked) = routing::posture_candidates(
            &request,
            c["sitting"].as_i64().unwrap() as i32,
            &mut rng,
            |p| {
                c["seats"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|s| {
                        s["position"][0].as_i64().unwrap() as i32 >> 4 == p.x >> 4
                            && s["position"][1].as_i64().unwrap() as i32 >> 4 == p.y >> 4
                    })
                    .map(|s| Seat {
                        id: s["id"].as_i64().unwrap() as i16,
                        facing: s["facing"].as_i64().unwrap() as i16,
                        allowed: s["mask"].as_u64().unwrap_or(255) as u16,
                        sit: s["sit"].as_bool().unwrap_or(true),
                        other_occupant: if s["occupant"] == 2 { 2 } else { 0 },
                    })
            },
            |p, d| {
                let classification = c["classification"][d as usize].as_i64().unwrap() as i32;
                checks
                    .push(json!({"point":[p.x,p.y],"direction":d,"classification":classification}));
                classification
            },
        )
        .unwrap();
        // Each native corpus case also runs through a JSON checkpoint at every
        // clearance boundary. Calling next twice must not repeat lookup/RNG.
        let mut split =
            routing::PostureScan::new(request.clone(), c["sitting"].as_i64().unwrap() as i32, 0)
                .unwrap();
        let mut split_rng = c["seed"].as_u64().unwrap() as u32;
        let lookup = |point: sims_sim::Point, _| -> Result<Option<Seat>, String> {
            Ok(c["seats"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| {
                    s["position"][0].as_i64().unwrap() as i32 >> 4 == point.x >> 4
                        && s["position"][1].as_i64().unwrap() as i32 >> 4 == point.y >> 4
                })
                .map(|s| Seat {
                    id: s["id"].as_i64().unwrap() as i16,
                    facing: s["facing"].as_i64().unwrap() as i16,
                    allowed: s["mask"].as_u64().unwrap_or(255) as u16,
                    sit: s["sit"].as_bool().unwrap_or(true),
                    other_occupant: if s["occupant"] == 2 { 2 } else { 0 },
                }))
        };
        let mut split_checks = vec![];
        while let Some(check) = split.next(&mut split_rng, lookup).unwrap() {
            split.validate().unwrap();
            split = serde_json::from_slice(&serde_json::to_vec(&split).unwrap()).unwrap();
            let seed = split_rng;
            assert_eq!(
                split
                    .next(&mut split_rng, |_, _| panic!("repeated seat lookup"))
                    .unwrap(),
                Some(check)
            );
            assert_eq!(seed, split_rng);
            let classification = c["classification"][check.direction as usize]
                .as_u64()
                .unwrap() as u8;
            split_checks.push(json!({"point":check.position,"direction":check.direction,"classification":classification}));
            split.resume(check, classification).unwrap();
            split.validate().unwrap();
            split = serde_json::from_slice(&serde_json::to_vec(&split).unwrap()).unwrap();
        }
        split.validate().unwrap();
        assert_eq!(split.outcome(), Some(&set));
        assert_eq!(split.blocked_object(), blocked);
        assert_eq!(split_rng, rng);
        assert_eq!(split_checks, checks);
        let mut actual = serde_json::to_value(set).unwrap();
        actual["blocked_seat"] = json!(blocked);
        actual["rng"] = json!(rng);
        actual["checks"] = json!(checks);
        actual["error"] = Value::Null;
        assert_eq!(actual, row["original"], "case {i}: {c}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_routing_decision_vectors() {
    comparison::compare(serde_json::from_str(include_str!("fixtures/routing.json")).unwrap());
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_diagonal_and_capped_search_vectors() {
    comparison::compare(
        serde_json::from_str(include_str!("fixtures/route-diagonal.json")).unwrap(),
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_relative_route_flags_do_not_change_standing_candidates() {
    comparison::compare(
        serde_json::from_str(include_str!("fixtures/relative-candidates.json")).unwrap(),
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_direct_snap_destination_vectors() {
    use serde_json::{Value, json};
    let report: Value = serde_json::from_str(include_str!("fixtures/snap.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let front = c["mode"] == 2;
        let (position, facing) = sims_sim::routing::direct_snap(
            serde_json::from_value(c["target"].clone()).unwrap(),
            c["rotation"].as_u64().unwrap() as u8,
            c["facing"].as_u64().unwrap() as u8,
            front,
        )
        .unwrap();
        assert_eq!(
            json!([
                position[0],
                position[1],
                1,
                if front { 0 } else { 2 },
                0,
                0,
                i32::from(facing),
                ((c["flags"].as_u64().unwrap() >> 2) & 1) as i32
            ]),
            n["placement"],
            "{c}"
        );
        assert_eq!(n["error"], Value::Null);
        assert_eq!(n["returned"], c["success"]);
        assert_eq!(
            n["offset"],
            if c["success"] == 1 {
                json!([0., 0., 0.])
            } else {
                Value::Null
            }
        );
        assert_eq!(n["result"], 0);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_mixed_slots_without_nearby_seats() {
    let mut report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/home-mixed-slots.json")).unwrap();
    report["departures"] = serde_json::json!([]);
    report["candidates"] = report["cases"].take();
    comparison::compare(report);
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_surface_seat_adjacency_and_notifications() {
    use sims_sim::routing::{self, DIRECTION_OFFSETS};
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/domestic-adjacency.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        let mut lookups = vec![vec![5, 5]];
        let seat = routing::adjacent_seat(|d| {
            let (dx, dy) = DIRECTION_OFFSETS[d as usize];
            let point = vec![5 + dy / 16, 5 + dx / 16];
            lookups.push(point.clone());
            (serde_json::json!(point) == c["seat"] && c["seat_flags"] == 0).then(|| {
                (
                    c["sit"].as_bool().unwrap(),
                    c["facing"].as_u64().unwrap() as u8,
                )
            })
        });
        let (flags, notify) = routing::adjacency_flags(
            c["flags"].as_u64().unwrap() as u16,
            seat,
            c["surface"].as_bool().unwrap(),
        );
        assert_eq!(serde_json::json!(lookups), e["lookups"], "{c}");
        assert_eq!(serde_json::json!(flags), e["flags"], "{c}");
        assert_eq!(
            usize::from(notify),
            e["callbacks"].as_array().unwrap().len(),
            "{c}"
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_current_seat_reuse_requires_a_matching_usable_candidate() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/domestic-seat-reuse.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let keep = sims_sim::routing::keeps_current_seat(
            c["seats"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_i64().unwrap() as i16),
            c["current"].as_i64().unwrap() as i16,
            |id| [1, 2].contains(&id) && c["sit"].as_bool().unwrap(),
        );
        assert_eq!(keep, row["original"]["keep"].as_bool().unwrap(), "{c}");
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_retry_distance_threshold() {
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/domestic-route-retry.json")).unwrap();
    for c in doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["stage"] == 0)
    {
        let point = |k: &str| {
            sims_sim::Point::new(
                c[k][0].as_i64().unwrap() as i32,
                c[k][1].as_i64().unwrap() as i32,
            )
        };
        assert_eq!(
            sims_sim::routing::made_route_progress(point("previous"), point("current")),
            c["decision"] == "replan",
            "{c}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn seated_scan_preserves_intervening_callback_rng_facing_and_blocker_writes() {
    use serde_json::{Value, json};
    use sims_sim::routing::{PostureScan, Seat, StandingRequest};
    let report: Value =
        serde_json::from_str(include_str!("fixtures/seated-callbacks.json")).unwrap();
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let request: StandingRequest = serde_json::from_value(c.clone()).unwrap();
        let mut scan = PostureScan::new(request, c["sitting"].as_i64().unwrap() as i32, 0).unwrap();
        let mut rng = c["seed"].as_u64().unwrap() as u32;
        let mut checks = vec![];
        while let Some(check) = scan
            .next(&mut rng, |p, _| {
                Ok(c["seats"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|s| {
                        s["position"][0].as_i64().unwrap() as i32 >> 4 == p.x >> 4
                            && s["position"][1].as_i64().unwrap() as i32 >> 4 == p.y >> 4
                    })
                    .map(|s| Seat {
                        id: s["id"].as_i64().unwrap() as i16,
                        facing: s["facing"].as_i64().unwrap() as i16,
                        allowed: s["mask"].as_u64().unwrap() as u16,
                        sit: true,
                        other_occupant: s["occupant"].as_i64().unwrap_or(0) as i16,
                    }))
            })
            .unwrap()
        {
            scan.validate().unwrap();
            scan = serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
            let seed = rng;
            assert_eq!(
                scan.next(&mut rng, |_, _| panic!("repeated seat lookup"))
                    .unwrap(),
                Some(check)
            );
            assert_eq!(seed, rng);
            let i = checks.len();
            let classification =
                [6, 6, 5, 3, 6, 4, 1][(i + c["phase"].as_u64().unwrap() as usize) % 7];
            let after_rng = rng ^ (i as u32 + 1).wrapping_mul(0x9e3779b9);
            let after_facing = (check.actor_facing + 3) & 7;
            let blocker = if i % 2 == 0 { 0 } else { 7 };
            checks.push(json!({"point":check.position,"direction":check.direction,
                "classification":classification,"rng":rng,"actor_facing":check.actor_facing,
                "after_rng":after_rng,"after_facing":after_facing,"blocker":blocker}));
            scan.resume(check, classification).unwrap();
            scan.observe_actor_facing(after_facing).unwrap();
            scan.observe_blocker(blocker);
            rng = after_rng;
        }
        scan.validate().unwrap();
        let mut actual = serde_json::to_value(scan.outcome().unwrap()).unwrap();
        actual["checks"] = json!(checks);
        actual["blocked_seat"] = json!(scan.blocked_object());
        actual["rng"] = json!(rng);
        actual["error"] = Value::Null;
        assert_eq!(actual, row["original"], "mixed callback case {case}");
    }
}
