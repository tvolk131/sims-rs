use serde_json::{Value, json};
use sims_vm::animation::object_action_table;

fn compare(report: &Value) {
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let gate = c["foreground"] == true
            && c["active_matches"] == true
            && c["icon_present"] == true
            && c["icon_kind"] == 2;
        let icon_age = (gate && c["cast"] == true).then(|| c["icon_age"].as_i64().unwrap() as i16);
        let table = object_action_table(
            c["age"].as_i64().unwrap() as i16,
            c["foreground"].as_bool().unwrap(),
            c["active_matches"].as_bool().unwrap(),
            icon_age,
        );
        let mut calls = vec![];
        if gate {
            calls.push(json!(["cast"]));
        }
        calls.push(json!(["resource", table]));
        if c["variant"] != "absent" {
            calls.push(json!(["string", table, c["index"].as_i64().unwrap() + 1]));
        }
        let present = c["variant"] == "value";
        if !present {
            calls.push(json!(["warning"]));
        }
        calls.push(json!(["resolve", present]));
        assert_eq!(json!(table), row["original"]["table"], "{c}");
        assert_eq!(json!(calls), row["original"]["calls"], "{c}");
        assert_eq!(
            json!(if present { 123 } else { 0 }),
            row["original"]["result"],
            "{c}"
        );
        assert!(row["original"]["error"].is_null());
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_object_and_social_animation_tables() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/object-animation-selection.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 288);
    compare(&report);
}

#[test]
#[ignore = "requires private native capture probe output"]
fn complete_original_object_animation_selection_corpus() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/object-animation-selection.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 1440);
    compare(&report);
}
