use serde_json::{Value, json};
use sims_content::{Index, careers};
use sims_formats::iff::{Iff, ResourceMap};

#[test]
#[ignore = "requires original work.iff and the native career localization report"]
fn installed_career_text_matches_native_localization() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted")).unwrap();
    let catalog = careers::load(&index).unwrap();
    let report: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/career-text-installed.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let ids: Vec<i32> = report["input"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_i64().unwrap() as i32)
        .collect();
    assert_eq!(catalog.ids, ids);
    let text = catalog.text.as_ref().unwrap();
    assert_eq!(text.len(), 21);
    assert_eq!(text.iter().map(|r| r.levels.len()).sum::<usize>(), 210);
    assert_eq!(json!(text), report["original"]["records"]);
}
#[test]
#[ignore = "requires original work.iff and the private original resource-map codec report"]
fn original_career_catalog_and_map_records_match_native() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted")).unwrap();
    let catalog = careers::load(&index).unwrap();
    let iff = Iff::parse(
        index.resources[index.find(&catalog.source).unwrap()]
            .read()
            .unwrap(),
    )
    .unwrap();
    let map = ResourceMap::parse(iff.data(iff.get(b"rsmp", 0).unwrap())).unwrap();
    let report: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/work-resource-map.json")).unwrap(),
    )
    .unwrap();
    for row in report["records"].as_array().unwrap() {
        let tag = (row["tag"].as_u64().unwrap() as u32).to_be_bytes();
        let i = row["ordinal"].as_u64().unwrap() as usize - 1;
        let mut expected = row["original"].clone();
        expected.as_object_mut().unwrap().remove("cursor");
        assert_eq!(json!(map.types[&tag][i]), expected);
    }
    let ids: Vec<_> = (1..=10).chain([-1]).chain(12..=21).collect();
    assert_eq!(catalog.ids, ids);
    assert_eq!(
        catalog.content_hash,
        "7038417822b72f1190788b3070d7ec9ccabe7f4becab233b7d59e2003dff6a0f"
    );
}

#[test]
#[ignore = "requires original work.iff and native CARR codec report"]
fn installed_career_levels_match_original_decoder() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted")).unwrap();
    let catalog = careers::load(&index).unwrap();
    let report: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/career-codec.json")).unwrap(),
    )
    .unwrap();
    let records = catalog.records.as_ref().unwrap();
    assert_eq!(records.len(), 21);
    assert_eq!(records.iter().map(|r| r.levels.len()).sum::<usize>(), 210);
    for (&id, record) in catalog.ids.iter().zip(records) {
        let row = report["careers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"].as_u64().unwrap() as u16 as i16 as i32 == id)
            .unwrap();
        assert_eq!(
            serde_json::to_value(record).unwrap(),
            row["original"],
            "career {id}"
        );
    }
}
