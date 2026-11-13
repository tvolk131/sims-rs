use sims_sim::lot_presenter::floor_prep::diagonal_coverings;
#[cfg(feature = "private-fixtures")]
#[test]
fn diagonal_coverings_match_original_priority_fallback_and_half_order() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-diagonal-floor-plan.json")).unwrap();
    let rows = v["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 4096);
    for (n, row) in rows.iter().enumerate() {
        let i = &row["input"];
        let result = diagonal_coverings(
            i["flags"].as_u64().unwrap() as u8,
            i["style"].as_u64().unwrap() as u16,
            i["pattern"].as_u64().unwrap() as u16,
            i["covering"].as_u64().unwrap() as u16,
        );
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            row["original"],
            "case{n}"
        );
    }
}
