use serde_json::Value;
use sims_vm::career_text::{Text, localize};
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn career_text_matches_native_expansion_and_string_table_cases() {
    let report: Value = serde_json::from_str(include_str!("fixtures/career-text.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 21);
    for (n, row) in cases.iter().enumerate() {
        let input = &row["input"];
        let mut ids = Vec::new();
        let mut records = Vec::new();
        for record in input["records"].as_array().unwrap() {
            ids.push(record["id"].as_i64().unwrap() as i32);
            let carr = sims_formats::career::Career {
                version: 1,
                packed: false,
                consumed: 0,
                name: record["name"].as_str().unwrap().into(),
                levels: record["titles"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|title| {
                        let mut strings: [String; 5] = Default::default();
                        strings[0] = title.as_str().unwrap().into();
                        sims_formats::career::Level {
                            values: [0; 21],
                            strings,
                        }
                    })
                    .collect(),
            };
            records.push(Text::from_carr(&carr));
        }
        let tables: BTreeMap<u16, BTreeMap<usize, String>> =
            serde_json::from_value(input["tables"].clone()).unwrap();
        localize(
            &ids,
            &mut records,
            &tables,
            input["expansions"].as_i64().unwrap() as i16,
        )
        .unwrap();
        let expected: Vec<Text> =
            serde_json::from_value(row["original"]["records"].clone()).unwrap();
        assert_eq!(records, expected, "native case {n}");
    }
}
