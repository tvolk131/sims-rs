#[cfg(feature = "private-fixtures")]
#[test]
fn original_ground_branch_cover_water_short_circuit_and_inclusive_clamps() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-ground-selection.json")).unwrap();
    assert_eq!(v["cases"].as_array().unwrap().len(), 1024);
    for (n, row) in v["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut covering = vec![0; 4096];
        let mut water = vec![0; 4096];
        for cell in c["cells"].as_array().unwrap() {
            let p: [usize; 2] = serde_json::from_value(cell[0].clone()).unwrap();
            let i = p[0] * 64 + p[1];
            covering[i] = cell[1][0].as_u64().unwrap() as u16;
            water[i] = cell[1][1].as_u64().unwrap() as u8;
        }
        let selected = sims_sim::lot_presenter::ground::selected(
            c["enabled"].as_u64().unwrap() as u8,
            serde_json::from_value(c["point"].clone()).unwrap(),
            c["current"].as_u64().unwrap() as u16,
            64,
            c["active"].as_i64().unwrap() as i32,
            &covering,
            &water,
        )
        .unwrap();
        assert_eq!(selected, row["selected"].as_bool().unwrap(), "case{n}");
    }
}
