//! Original instruction controls, including non-boolean visibility words.
#[cfg(feature = "private-fixtures")]
#[test]
fn restoration_matches_original_truth_normalization() {
    let source: serde_json::Value = serde_json::from_str(include_str!("fixtures/native-visibility-restore.json")).unwrap();
    assert_eq!(source["source_unchanged"],true);
    let cases=source["cases"].as_array().unwrap();assert_eq!(cases.len(),48);
    for c in cases {
        let prior=c["prior"].as_i64().unwrap()as i16;
        let current=c["current"].as_i64().unwrap()as i16;
        let active=c["active"].as_bool().unwrap();
        let result=sims_sim::lot_presenter::object_cutaway::restored(prior,active,current);
        assert_eq!(i64::from(result),c["result"].as_i64().unwrap(),"{c}");
        let stores=if active {vec![i64::from(result)]}else{vec![]};
        assert_eq!(serde_json::json!(stores),c["writes"]);
    }
}
