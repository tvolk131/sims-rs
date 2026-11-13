#[cfg(feature = "private-fixtures")]
#[test]
fn original_per_sprite_words() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/sprite-visibility.json")).unwrap();
    for row in data["cases"].as_array().unwrap() {
        let c = &row["input"];
        let flags: Vec<_> = c["flags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i16)
            .collect();
        assert_eq!(
            sims_render::sprite_visible(
                c["sprite"].as_i64().unwrap() as i16,
                c["base"].as_i64().unwrap() as i16,
                &flags
            ),
            row["original"]["visible"].as_bool().unwrap(),
            "{c}"
        );
    }
}
