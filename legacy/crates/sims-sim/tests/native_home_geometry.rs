use sims_sim::spatial;
#[cfg(feature = "private-fixtures")]
#[test]
fn recovered_footprints_walls_room_cache_and_route_classification() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/home-geometry.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = |k: &str| c[k].as_i64().unwrap();
        let b = |k: &str| c[k].as_bool().unwrap();
        let expected = &row["original"];
        assert!(expected["error"].is_null());
        match row["kind"].as_str().unwrap() {
            "footprint" => {
                let p = [
                    c["position"][0].as_i64().unwrap() as i32,
                    c["position"][1].as_i64().unwrap() as i32,
                ];
                let actual = spatial::footprint(
                    p,
                    b("person"),
                    b("zero_size"),
                    n("size") as i16,
                    n("mask") as u16,
                    n("extension") as u16,
                    n("facing") as u8,
                );
                assert_eq!(serde_json::json!(actual), expected["bounds"], "{c}");
            }
            "classify" => {
                let p = [
                    c["position"][0].as_i64().unwrap() as i32,
                    c["position"][1].as_i64().unwrap() as i32,
                ];
                let outside = p.iter().any(|p| {
                    let tile = (p >> 4) as i8;
                    !(1..15).contains(&tile)
                });
                let (result, blocker) = spatial::classify(
                    outside,
                    n("flags") as u32,
                    n("room") == 1,
                    n("direction") as i32,
                    n("walls") as u8,
                    n("height") as u32,
                    n("target_height") as u32,
                    spatial::PlacementResult {
                        allowed: b("placement"),
                        error: n("placement_error") as i32,
                        blocker: b("blocker").then_some((2, n("blocker_kind") as i16)),
                    },
                    b("ignore").then_some(2),
                );
                assert_eq!(serde_json::json!(result), expected["result"], "{c}");
                assert_eq!(
                    serde_json::json!(blocker.unwrap_or(77)),
                    expected["blocker"],
                    "{c}"
                );
            }
            "walls" => assert_eq!(
                serde_json::json!(spatial::wall_mask(
                    n("tile_bits") as u8,
                    0,
                    n("rotation") as u8
                )),
                expected["mask"]
            ),
            "exterior" => {
                let tiles: Vec<_> = c["tiles"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| (v[0].as_i64().unwrap() as i16, v[1].as_u64().unwrap() as u8))
                    .collect();
                assert_eq!(
                    spatial::room_outdoors(n("room") as i16, &tiles),
                    expected["outdoors"] == 1,
                    "{c}"
                );
            }
            _ => panic!("unknown fixture kind"),
        }
    }
}
