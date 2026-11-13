use sims_sim::lot_presenter::level_geometry::orientation_mask;
#[cfg(feature = "private-fixtures")]
#[test]
fn orientations_match_original_unsigned_dispatch() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-geometry-orientation.json")).unwrap();
    let cases = v["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 256);
    for case in cases {
        assert_eq!(
            serde_json::json!(orientation_mask(case["input"].as_u64().unwrap() as u32)),
            case["original"]
        );
    }
}
