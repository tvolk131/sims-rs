use sims_sim::lot_presenter::room_masks::{Camera, SplitRooms, View};
#[cfg(feature = "private-fixtures")]
#[test]
fn original_tile_rectangle_height_padding_and_clamping() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-tile-invalidation.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 512);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let p: [i8; 3] = serde_json::from_value(c["point"].clone()).unwrap();
        let camera: Camera = serde_json::from_value(c["camera"].clone()).unwrap();
        let mut heights = vec![[0; 4]; 4096];
        let cell: [i32; 2] = serde_json::from_value(c["height_cell"].clone()).unwrap();
        if (0..64).contains(&cell[0]) && (0..64).contains(&cell[1]) {
            heights[cell[0] as usize * 64 + cell[1] as usize] =
                serde_json::from_value(c["height_raw"].clone()).unwrap();
        }
        let splits = SplitRooms::new();
        let walls = vec![sims_formats::lot::WallTile::default(); 4096];
        let view = View {
            size: 64,
            rotation: c["rotation"].as_u64().unwrap() as u8,
            bounds: serde_json::from_value(c["bounds"].clone()).unwrap(),
            camera: &camera,
            heights: &heights,
            walls: &walls,
            splits: &splits,
        };
        let actual = view
            .invalidation_rectangle(
                p,
                c["visible_level"].as_i64().unwrap() as i32,
                serde_json::from_value(c["viewport"].clone()).unwrap(),
            )
            .unwrap();
        let expected = row["expected"]["events"]
            .as_array()
            .unwrap()
            .first()
            .map(|e| serde_json::from_value::<[i32; 4]>(e["rectangle"].clone()).unwrap());
        assert_eq!(actual, expected, "native tile rectangle {n}");
    }
}
