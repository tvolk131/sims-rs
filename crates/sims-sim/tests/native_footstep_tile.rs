#[cfg(feature = "private-fixtures")]
#[test]
fn native_footstep_gate_rounding_and_point_narrowing() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-footstep-tile.json")).unwrap();
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 225);
    for row in cases {
        let position = serde_json::from_value(row["position"].clone()).unwrap();
        let expected: Option<[i8; 3]> = serde_json::from_value(row["tile"].clone()).unwrap();
        assert_eq!(
            sims_sim::pose::properties::footstep_tile(
                position,
                row["level"].as_i64().unwrap() as i32
            )
            .unwrap(),
            expected,
            "{row}"
        );
    }
}
