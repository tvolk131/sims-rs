use serde_json::Value;
use sims_vm::zoning;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_zoning_text_and_expansion_tail() {
    let report: Value = serde_json::from_str(include_str!("fixtures/lot-zoning.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 44);
    for row in report["cases"].as_array().unwrap() {
        let lines: Vec<String> = serde_json::from_value(row["input"]["lines"].clone()).unwrap();
        let original: Vec<[i32; 2]> = serde_json::from_value(row["original"].clone()).unwrap();
        assert_eq!(
            zoning::parse(&lines, row["input"]["expansions"].as_u64().unwrap() as u16).unwrap(),
            original
        );
    }
}
#[test]
fn zoning_query_keeps_first_id_and_never_aliases_wide_ids() {
    let table = zoning::query_table(&[[7, 2], [7, 1], [65543, 3], [-32769, 4], [-1, 2]]);
    assert_eq!(table, [(7, 2), (-1, 2)].into());
    for text in [
        "2147483648, residential",
        "-2147483649, residential",
        "é, community",
    ] {
        assert!(zoning::parse(&[text.to_owned()], 0).is_err());
    }
    assert!(zoning::parse(&["x".repeat(99)], 0).is_err());
}
