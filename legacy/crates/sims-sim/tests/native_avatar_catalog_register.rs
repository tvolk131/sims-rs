use serde_json::{Value, json};
use sims_formats::avatar::index::Kind;
use sims_sim::pose::clothing::{
    catalog_index::Lazy,
    catalog_record::{AssetId, Assets, CatalogReader, Record, Registry},
};

struct Host {
    input: Value,
    assets: Vec<Value>,
    events: Vec<Value>,
    item: usize,
    decoded: u64,
    text: bool,
    failed: Option<u64>,
}
impl Assets for Host {
    fn name(&self, id: AssetId) -> Result<Vec<u8>, String> {
        Ok(serde_json::from_value(
            self.assets.iter().find(|a| a["id"] == id).unwrap()["name"].clone(),
        )
        .unwrap())
    }
    fn rename(&mut self, id: AssetId, name: Vec<u8>) -> Result<(), String> {
        self.assets.iter_mut().find(|a| a["id"] == id).unwrap()["name"] = json!(name);
        Ok(())
    }
    fn dispose(&mut self, id: AssetId) -> Result<(), String> {
        self.events.push(json!(["dispose", id]));
        self.assets.iter_mut().find(|a| a["id"] == id).unwrap()["disposed"] = json!(true);
        Ok(())
    }
}
impl CatalogReader for Host {
    fn count(&mut self, kind: Kind) -> Result<Option<i32>, String> {
        self.item = 0;
        let group = &self.input["groups"][kind.index()];
        let count = group
            .as_array()
            .map(|g| g.len() as i32)
            .or_else(|| group.as_i64().map(|n| n as i32));
        self.events.push(json!(["count", kind.index(), count]));
        Ok(count)
    }
    fn read(&mut self, kind: Kind, context: u32) -> Result<Option<AssetId>, String> {
        assert_eq!(context as u64, self.input["context"].as_u64().unwrap());
        let record = &self.input["groups"][kind.index()][self.item];
        self.item += 1;
        self.decoded += 1;
        self.assets
            .push(json!({"id": self.decoded, "name": record["name"], "disposed": false}));
        self.events
            .push(json!(["read", kind.index(), self.decoded, record["ok"]]));
        let ok = record["ok"].as_bool().unwrap();
        if !ok && self.text {
            self.failed = Some(self.decoded);
        }
        Ok(ok.then_some(self.decoded))
    }
    fn report_error(&mut self, catalog: &[u8]) -> Result<(), String> {
        self.events.push(json!(["error", catalog]));
        if let Some(id) = self.failed.take() {
            self.dispose(id)?;
        }
        Ok(())
    }
}
fn location(value: &Value) -> Lazy {
    Lazy {
        name: serde_json::from_value(value["name"].clone()).unwrap_or_default(),
        directory: serde_json::from_value(value["directory"].clone()).unwrap(),
        catalog: serde_json::from_value(value["catalog"].clone()).unwrap(),
        context: value["context"].as_u64().unwrap() as u32,
    }
}
fn encode(record: &Record) -> Value {
    json!({"name": record.location.name, "directory": record.location.directory, "catalog": record.location.catalog,
        "context": record.location.context, "attempted": record.attempted, "asset": record.asset})
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_binary_registration_keeps_loaded_owners_and_partial_success() {
    compare(include_str!("fixtures/avatar-catalog-register.json"), false);
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_text_registration_keeps_loaded_owners_and_partial_success() {
    compare(include_str!("fixtures/avatar-text-register.json"), true);
}
fn compare(fixture: &str, text: bool) {
    let report: Value = serde_json::from_str(fixture).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 26);
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let mut registry = Registry::default();
        let mut host = Host {
            input: row["input"].clone(),
            assets: vec![],
            events: vec![],
            item: 0,
            decoded: 0,
            text,
            failed: None,
        };
        for kind in [Kind::Skeleton, Kind::Appearance, Kind::Animation] {
            for value in row["input"]["existing"][kind.index()].as_array().unwrap() {
                let record = Record {
                    location: location(value),
                    attempted: value["attempted"].as_bool().unwrap(),
                    asset: value["asset"].as_u64(),
                };
                if let Some(id) = record.asset {
                    host.assets
                        .push(json!({"id": id, "name": record.location.name, "disposed": false}));
                }
                registry.insert(kind, record);
            }
        }
        let returned = registry
            .read_catalog(&location(&row["input"]), &mut host)
            .unwrap();
        assert_eq!(json!(returned), row["original"]["returned"], "case {case}");
        let events: Vec<_> = row["original"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e[0] != "lookup" && e[0] != "insert")
            .cloned()
            .collect();
        assert_eq!(
            host.events, events,
            "case {case} parser/disposal/error order"
        );
        assert_eq!(
            json!(host.assets),
            row["original"]["assets"],
            "case {case} names and disposal"
        );
        for kind in [Kind::Skeleton, Kind::Appearance, Kind::Animation] {
            let expected = row["original"]["maps"][kind.index()].as_array().unwrap();
            assert_eq!(registry.iter(kind).count(), expected.len());
            for record in expected {
                let name: Vec<u8> = serde_json::from_value(record["name"].clone()).unwrap();
                assert_eq!(
                    encode(registry.get(kind, &name).unwrap()),
                    *record,
                    "case {case} final record"
                );
            }
        }
    }
}
