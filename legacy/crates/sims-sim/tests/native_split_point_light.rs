use sims_sim::lot_environment::lighting::point::{split_sample, RoomLight};

#[cfg(feature = "private-fixtures")]
#[test]
fn diagonal_light_matches_original_internal_cut_and_float_spill() {
    let source: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-split-point-light.json")).unwrap();
    assert_eq!(source["source_admission"], false);
    assert_eq!(source["native_parity_credit"], 0);
    let rows = source["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 4096);
    for (ordinal, row) in rows.iter().enumerate() {
        let c = &row["input"];
        let rooms: [Option<RoomLight>; 2] = serde_json::from_value(c["rooms"].clone()).unwrap();
        let result = split_sample(
            serde_json::from_value(c["physical"].clone()).unwrap(),
            c["rotation"].as_u64().unwrap() as u8,
            c["flags"].as_u64().unwrap() as u32,
            serde_json::from_value(c["directions"].clone()).unwrap(),
            rooms,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(result.room).unwrap(),
            row["original"]["room"],
            "case{ordinal}"
        );
        assert_eq!(
            result.light,
            row["original"]["light"].as_u64().unwrap() as u32,
            "case{ordinal}"
        );
    }
    assert!(split_sample([1024, 0], 0, 0, [0, 0], [None, None]).is_err());
    assert!(split_sample([0, 0], 4, 0, [0, 0], [None, None]).is_err());
}
