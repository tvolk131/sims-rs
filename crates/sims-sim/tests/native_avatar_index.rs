//! Compare registration at the native map boundary, including successful work
//! before a later short read. Native map hashing has its own fixture corpus.
use serde_json::{Value, json};
use sims_formats::avatar::index::{IndexReader, Kind};
use sims_sim::pose::clothing::catalog_index::{self, Host, Lazy};

#[derive(Default)]
struct Maps {
    values: [Vec<Value>; 3],
    events: Vec<Value>,
}
impl Host for Maps {
    fn contains(&mut self, kind: Kind, name: &[u8]) -> Result<bool, String> {
        let found = self.values[kind.index()]
            .iter()
            .any(|v| v["name"] == json!(name));
        self.events
            .push(json!(["lookup", kind.index(), name, found]));
        Ok(found)
    }
    fn insert(&mut self, kind: Kind, record: Lazy) -> Result<(), String> {
        let value = json!({
            "name": record.name, "directory": record.directory,
            "catalog": record.catalog, "context": record.context,
            "loaded": false, "pointer": 0
        });
        self.events.push(json!(["insert", kind.index(), value]));
        self.values[kind.index()].push(value);
        Ok(())
    }
}

fn compare(report: &Value) -> usize {
    let mut records = 0;
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let mut host = Maps::default();
        for (kind, names) in row["input"]["existing"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            for name in names.as_array().unwrap() {
                host.values[kind].push(json!({"seed": true, "name": name}));
            }
        }
        for (file, (input, expected)) in row["input"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["original"]["files"].as_array().unwrap())
            .enumerate()
        {
            let data: Vec<u8> = serde_json::from_value(input["data"].clone()).unwrap();
            let directory: Vec<u8> = serde_json::from_value(input["directory"].clone()).unwrap();
            let mut reader = IndexReader::new(&data);
            let result = catalog_index::register(
                &mut reader,
                &directory,
                input["context"].as_u64().unwrap() as u32,
                &mut host,
            );
            assert_eq!(
                json!({"returned": u32::from(result.is_err()),
                "consumed": reader.bytes_consumed(), "events": host.events}),
                *expected,
                "case {case}, file {file}: {result:?}"
            );
            assert!(
                reader.next().is_none(),
                "completed/failed reader must stay finished"
            );
            records += host.events.iter().filter(|e| e[0] == "insert").count();
            host.events.clear();
        }
        assert_eq!(
            json!(host.values),
            row["original"]["maps"],
            "case {case} retained maps"
        );
    }
    records
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_lazy_index_registration_matches_original() {
    let report: Value = serde_json::from_str(include_str!(
        "../../sims-formats/tests/fixtures/avatar-index.json"
    ))
    .unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 23);
    compare(&report);
}

#[test]
#[ignore = "requires the private original NDX native-probe report"]
fn original_archived_indexes_match_native_registration() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/native-original-avatar-index.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 94);
    assert_eq!(compare(&report), 10225);
}
