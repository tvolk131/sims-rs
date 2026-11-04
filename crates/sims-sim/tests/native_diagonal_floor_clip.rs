use sims_sim::lot_presenter::floor_prep::diagonal_clip;

#[cfg(feature = "private-fixtures")]
#[test]
fn half_clips_match_original_signed_word_arithmetic() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-diagonal-floor-clip.json")).unwrap();
    let rows = v["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 8192);
    for (n, row) in rows.iter().enumerate() {
        let i = &row["input"];
        let clip = std::array::from_fn(|k| i["clip"][k].as_i64().unwrap() as i16);
        let screen = std::array::from_fn(|k| i["screen"][k].as_i64().unwrap() as i32);
        let result = diagonal_clip(
            i["orientation"].as_u64().unwrap() as u8,
            i["half"].as_u64().unwrap() as usize,
            clip,
            screen,
            i["corner"].as_u64().unwrap() as u8,
            i["zoom"].as_u64().unwrap() as u32,
            Some(i["height"].as_i64().unwrap() as i16),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            row["original"],
            "case{n}"
        );
    }
    assert!(diagonal_clip(16, 0, [0; 4], [0; 2], 0, 1, None).is_err());
    assert!(diagonal_clip(32, 0, [0; 4], [0; 2], 0, 1, None).is_ok());
}
