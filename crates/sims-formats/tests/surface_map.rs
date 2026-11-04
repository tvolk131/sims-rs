use serde_json::{Value, json};
use sims_formats::lot::surface_map;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_surface_vectors_preserve_wide_ids_and_multiple_materials_per_file() {
    let report: Value = serde_json::from_str(include_str!("fixtures/surface-map.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let bytes: Vec<u8> = serde_json::from_value(c["bytes"].clone()).unwrap();
        let id = c["version"].as_u64().unwrap() as u16;
        let floor = c["floor"].as_bool().unwrap();
        let groups = surface_map(&bytes, id, floor).unwrap();
        assert_eq!(json!(groups), row["original"]["groups"], "{c}");
        assert_eq!(json!(bytes.len()), row["original"]["cursor"], "{c}");
        for end in 0..bytes.len() {
            assert!(
                surface_map(&bytes[..end], id, floor).is_err(),
                "accepted truncation {end}: {c}"
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(surface_map(&trailing, id, floor).is_err());
        assert!(surface_map(&bytes, 2, floor).is_err());
    }
}
