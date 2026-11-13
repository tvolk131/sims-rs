#[cfg(feature = "private-fixtures")]
#[test]
fn invalid_cached_graphic_has_no_sprite_in_original() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/graphic-lookup.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let input = &row["input"];
        let actual = sims_render::graphic_group(
            128,
            input["count"].as_u64().unwrap() as u16,
            input["index"].as_i64().unwrap() as i16,
        );
        assert_eq!(
            actual.unwrap_or(0) as u64,
            row["original"].as_u64().unwrap(),
            "{input}"
        );
    }
}
