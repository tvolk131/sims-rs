#[cfg(feature = "private-fixtures")]
#[test]
fn mesh_texture_selection_matches_native_prefix() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/avatar-texture.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for case in report["cases"].as_array().unwrap() {
        let input = &case["input"];
        assert_eq!(
            sims_content::saved_avatar::texture_override(
                input["mesh"].as_str().unwrap(),
                input["binding"].as_str().unwrap()
            ),
            case["original"].as_str().unwrap(),
            "{input}"
        );
    }
}
