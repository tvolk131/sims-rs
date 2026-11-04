use serde_json::{Value, json};
use sims_vm::animation::{NameTable, global_action_table, posture_name};

#[cfg(feature = "private-fixtures")]
#[test]
fn animation_age_profiles_overrides_and_fallbacks_match_native() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/animation-selection.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 446);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let age = c["age"].as_i64().unwrap() as i16;
        let index = c["index"].as_i64().unwrap() as i16;
        let tables = c["tables"].as_object().unwrap();
        let mut calls = vec![];
        let selected = if c["kind"] == "global" {
            let table = global_action_table(age).to_string();
            calls.push(json!([table, i32::from(index) + 1]));
            tables
                .get(&table)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        } else {
            posture_name(
                age,
                c["special"].as_i64().unwrap() as i16,
                c["species"].as_i64().unwrap() as i16,
                index as usize,
                |table, index| {
                    let key = match table {
                        NameTable::Private(150) => "private".to_owned(),
                        NameTable::Global(id) => id.to_string(),
                        _ => unreachable!(),
                    };
                    // An absent private resource returns before StringSet lookup.
                    if key != "private" || tables.contains_key(&key) {
                        calls.push(json!([key, index + 1]));
                    }
                    tables.get(&key).and_then(Value::as_str)
                },
            )
        };
        assert_eq!(json!(selected), row["original"]["selected"], "{c}");
        assert_eq!(json!(calls), row["original"]["calls"], "{c}");
    }
}
