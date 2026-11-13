use sims_sim::lot_presenter::level_geometry::tile_point;
#[cfg(feature = "private-fixtures")]
#[test]
fn tile_points_match_original_wrapping_and_in_place_cases() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-geometry-point.json")).unwrap();
    let cases = v["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 2048);
    for (n, case) in cases.iter().enumerate() {
        let p = std::array::from_fn(|k| case["input"]["point"][k].as_u64().unwrap() as u8);
        let orientation = case["input"]["orientation"].as_u64().unwrap() as u32;
        assert_eq!(
            serde_json::json!(tile_point(p, orientation)),
            case["original"],
            "case{n}"
        );
    }
}
