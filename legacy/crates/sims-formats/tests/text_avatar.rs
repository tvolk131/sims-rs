use serde_json::{Value, json};
use sims_formats::avatar::{read_cmx, read_skn};

fn check(row: &Value) {
    let input = &row["input"];
    let original = &row["original"];
    let text = input["text"].as_str().unwrap();
    if input["kind"] == "cmx" {
        let file = format!("// schema fixture\nversion 300\n0\n1\n{text}\n0\n");
        for file in [file.clone(), file.replace('\n', "\r\n")] {
            let parsed = read_cmx(file.as_bytes()).unwrap();
            let a = &parsed.appearances[0];
            assert_eq!(
                json!({"name":a.name.as_bytes(), "kind":a.kind,
                "bindings": a.bindings.iter().map(|b| json!({"bone":b.bone.as_bytes(),
                    "mesh":b.mesh.as_bytes(), "censor":b.censor})).collect::<Vec<_>>()}),
                *original
            );
        }
    } else {
        let (bytes, _, errors) = encoding_rs::WINDOWS_1252.encode(text);
        assert!(!errors);
        let mesh = read_skn(&bytes).unwrap();
        assert_eq!(json!(mesh.name.as_bytes()), original["name"]);
        assert_eq!(json!(mesh.texture.as_bytes()), original["texture"]);
        assert_eq!(
            json!(mesh.bones.iter().map(|b| b.as_bytes()).collect::<Vec<_>>()),
            original["bones"]
        );
        assert_eq!(json!(mesh.triangles), original["triangles"]);
        assert_eq!(
            json!(
                mesh.bindings
                    .iter()
                    .map(|b| [
                        b.bone,
                        b.first_vertex,
                        b.vertex_count,
                        b.first_blend,
                        b.blend_count,
                        0
                    ])
                    .collect::<Vec<_>>()
            ),
            original["bindings"]
        );
        assert_eq!(
            json!(
                mesh.vertices
                    .iter()
                    .map(|v| v.uv.map(f32::to_bits))
                    .collect::<Vec<_>>()
            ),
            original["uv_bits"]
        );
        assert_eq!(
            json!(
                mesh.blends
                    .iter()
                    .map(|b| [(b.weight * 32768.) as u32, b.vertex as u32])
                    .collect::<Vec<_>>()
            ),
            original["blends"]
        );
        let samples: Vec<_> = mesh
            .vertices
            .iter()
            .map(|v| (v.position, v.normal))
            .chain(mesh.blends.iter().map(|v| (v.position, v.normal)))
            .map(|(p, n)| p.into_iter().chain(n).map(f32::to_bits).collect::<Vec<_>>())
            .collect();
        assert_eq!(json!(samples), original["sample_bits"]);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn text_avatar_records_match_original_reader() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-text-avatar.json")).unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        check(row);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn text_avatar_rejects_invalid_or_unrepresented_data() {
    let cmx = "// fixture\nversion 300\n0\n1\nbody\n0\n0\n1\nPELVIS\nskin\n0\n0\n0\n";
    for bad in [
        cmx.replace("version 300", "version 301"),
        cmx.replace("body\n0\n0", "body\n0\n1"),
        cmx.replace("PELVIS", "PE\0LVIS"),
        format!("{cmx}unexpected"),
    ] {
        assert!(read_cmx(bad.as_bytes()).is_err());
    }
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-text-avatar.json")).unwrap();
    let skn = fixture["cases"].as_array().unwrap().last().unwrap()["input"]["text"]
        .as_str()
        .unwrap();
    for bad in [
        skn.replace("0 1 2", "0 1 999"),
        skn.replace("-0.25", "NaN"),
        skn.replace("0 0 2 1 1", "0 0 200 1 1"),
        skn.replace("0 32768", "0 32769"),
        format!("{skn}unexpected"),
        skn[..skn.len() / 2].to_owned(),
    ] {
        assert!(read_skn(bad.as_bytes()).is_err());
    }
    // Counts are checked before allocation, and incomplete line reads terminate.
    assert!(read_skn(b"skin\ntex\n2147483647\n").is_err());
    assert!(read_skn(b"skin\n\n").is_err());
}

#[test]
#[ignore = "requires private original text mesh native report and assets"]
fn original_text_meshes_match_native_records_and_catalogs_decode() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-original-text-avatar.json")).unwrap(),
    )
    .unwrap();
    let rows = fixture["cases"].as_array().unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        eprintln!("{}", row["input"]["path"]);
        check(row);
    }
    fn visit(path: &std::path::Path, count: &mut usize) {
        for entry in std::fs::read_dir(path).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                visit(&p, count);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cmx")) {
                read_cmx(&std::fs::read(&p).unwrap())
                    .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                *count += 1;
            }
        }
    }
    let mut catalogs = 0;
    visit(&root.join("game-data/cabinet-extracted"), &mut catalogs);
    println!(
        "{} native SKN records match, {catalogs} original CMX catalogs decode",
        rows.len()
    );
}
