use serde_json::Value;
use sims_formats::{avatar::read_skn, far};

#[cfg(feature = "private-fixtures")]
#[test]
fn text_mesh_name_selection_preserves_original_empty_line_skipping() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-skn-texture-names.json")).unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        let bytes = |value: &Value| {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        };
        let mut input = bytes(&row["input"]["text"]);
        // Complete the synthetic single-bone record with zero geometry.
        input.extend_from_slice(b"0\n0\n0\n0\n0\n");
        let next_number = bytes(&row["original"]["next_numeric_token"]);
        if next_number == b"1" {
            let mesh = read_skn(&input).unwrap();
            assert_eq!(mesh.name.as_bytes(), bytes(&row["original"]["name"]));
            assert_eq!(mesh.texture.as_bytes(), bytes(&row["original"]["texture"]));
            assert_eq!(mesh.bones, ["ROOT"]);
            assert!(mesh.vertices.is_empty());
        } else {
            // Original name selection has already consumed the count as the
            // texture name. Do not invent acceptance of the empty texture line.
            assert_eq!(next_number, b"ROOT");
            assert_eq!(bytes(&row["original"]["texture"]), b"1");
            assert_eq!(
                read_skn(&input).unwrap_err().message,
                "invalid avatar integer"
            );
        }
    }
}

#[test]
#[ignore = "requires the private Complete Collection expansion archive"]
fn original_three_blank_texture_meshes_retain_the_explicit_failure() {
    use std::io::{Read, Seek, SeekFrom};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../game-data/cabinet-extracted/ExpansionPack7/ExpansionPack7.far");
    let mut file = std::fs::File::open(path).unwrap();
    let length = file.metadata().unwrap().len();
    let mut header = [0; 16];
    file.read_exact(&mut header).unwrap();
    let directory = far::manifest_offset(&header, length).unwrap();
    file.seek(SeekFrom::Start(directory.into())).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    let entries = far::directory(&bytes, length).unwrap();
    for (name, offset) in [
        ("Accessories\\xskin-funhousecart-ROOT-CART.skn", 0x27),
        (
            "Accessories\\xskin-magic-wizeyelashes-R_HAND-WAX_JAR01.skn",
            0x36,
        ),
        (
            "Accessories\\xskin-magic-wizeyelashes-R_HAND-WAX_JAR08.skn",
            0x36,
        ),
    ] {
        let entry = entries.iter().find(|e| e.name == name).unwrap();
        assert_eq!(entry.stored_size, entry.unpacked_size);
        file.seek(SeekFrom::Start(entry.offset.into())).unwrap();
        let mut bytes = vec![0; entry.stored_size as usize];
        file.read_exact(&mut bytes).unwrap();
        assert!(bytes.split(|&b| b == b'\n').nth(1).unwrap() == b"\r");
        let error = read_skn(&bytes).unwrap_err();
        assert_eq!(error.message, "invalid avatar integer", "{name}");
        assert_eq!(error.offset, offset, "{name}");
    }
}
