use sims_sim::lot_presenter::floor_prep;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_neighbor_helpers_preserve_wrapping_absent_outputs_and_pair_order() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-floor-neighbors.json")).unwrap();
    let rows = v["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 4096);
    for (ordinal, row) in rows.iter().enumerate() {
        let descriptor: [u8; 4] =
            serde_json::from_value(row["input"]["descriptor"].clone()).unwrap();
        let point = serde_json::from_value(row["input"]["point"].clone()).unwrap();
        let neighbors = floor_prep::neighbors(descriptor, point);
        assert_eq!(
            neighbors.len(),
            row["original"]["count"].as_u64().unwrap() as usize,
            "case{ordinal}"
        );
        let mut outputs = [0xa5; 8];
        for (n, p) in neighbors.iter().enumerate() {
            outputs[n * 4..n * 4 + 3].copy_from_slice(&p.map(|v| v as u8));
        }
        let hex: String = outputs.iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(
            hex,
            row["original"]["neighbors"].as_str().unwrap(),
            "case{ordinal}"
        );
        for (n, index) in [-1, 0, 1, 2].into_iter().enumerate() {
            assert_eq!(
                serde_json::to_value(floor_prep::remove(descriptor, index)).unwrap(),
                row["original"]["removed"][n],
                "case{ordinal}, index{index}"
            );
        }
    }
}
