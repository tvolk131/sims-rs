use serde_json::{Value, json};
use sims_formats::avatar::index::Kind;
use sims_sim::pose::clothing::{
    catalog_index::Lazy,
    catalog_record::{AssetId, Assets, Loader, Record, Registry},
};
use std::collections::BTreeMap;

const ASSET: u64 = 0x10002000;
fn decode(value: &Value) -> Record {
    Record {
        location: Lazy {
            name: serde_json::from_value(value["name"].clone()).unwrap(),
            directory: serde_json::from_value(value["directory"].clone()).unwrap(),
            catalog: serde_json::from_value(value["catalog"].clone()).unwrap(),
            context: value["context"].as_u64().unwrap() as u32,
        },
        attempted: value["attempted"].as_bool().unwrap(),
        asset: value["asset"].as_u64().filter(|&a| a != 0),
    }
}
fn record(value: &Record) -> Value {
    json!({"name": value.location.name, "directory": value.location.directory,
        "catalog": value.location.catalog, "context": value.location.context,
        "attempted": value.attempted, "asset": value.asset.unwrap_or(0)})
}
struct Host {
    input: Value,
    assets: BTreeMap<AssetId, Vec<u8>>,
    kind: Kind,
    key: Vec<u8>,
    events: Vec<Value>,
}
impl Host {
    fn state(&self, registry: &Registry) -> Value {
        let record = registry.get(self.kind, &self.key).unwrap();
        json!([
            registry.loading_lazy,
            record.attempted,
            record.asset.unwrap_or(0)
        ])
    }
}
impl Assets for Host {
    fn name(&self, asset: AssetId) -> Result<Vec<u8>, String> {
        Ok(self.assets[&asset].clone())
    }
    fn rename(&mut self, asset: AssetId, name: Vec<u8>) -> Result<(), String> {
        self.assets.insert(asset, name);
        Ok(())
    }
    fn dispose(&mut self, asset: AssetId) -> Result<(), String> {
        self.events.push(json!([
            "dispose",
            asset,
            [
                self.input["loading"],
                self.input["record"]["attempted"],
                self.input["record"]["asset"]
            ]
        ]));
        self.assets.remove(&asset);
        Ok(())
    }
}
impl Loader for Host {
    type Handle = ();
    fn open(&mut self, path: &[u8], registry: &Registry) -> Result<Option<()>, String> {
        self.events
            .push(json!(["open", path, self.state(registry)]));
        Ok(self.input["open"].as_bool().unwrap().then_some(()))
    }
    fn load(&mut self, location: &Lazy, registry: &mut Registry) -> Result<u32, String> {
        self.events.push(json!([
            "load",
            location.directory,
            location.catalog,
            location.context,
            self.state(registry)
        ]));
        if let Some(asset) = self.input["effect"].as_u64() {
            let entry = registry.get_mut(self.kind, &self.key).unwrap();
            entry.asset = Some(asset);
            entry.attempted = true;
        }
        Ok(self.input["status"].as_u64().unwrap() as u32)
    }
    fn close(&mut self, _: (), registry: &Registry) -> Result<(), String> {
        self.events.push(json!(["close", self.state(registry)]));
        Ok(())
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_record_construction_replacement_and_failed_attempts_match() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/avatar-lazy-records.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 90);
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let kind = [Kind::Skeleton, Kind::Appearance, Kind::Animation]
            [input["kind"].as_u64().unwrap() as usize];
        let mut value = decode(&input["record"]);
        let mut host = Host {
            input: input.clone(),
            kind,
            key: value.location.name.clone(),
            events: vec![],
            assets: [(
                ASSET,
                serde_json::from_value(input["asset_name"].clone()).unwrap(),
            )]
            .into(),
        };
        let mut loading = input["loading"].as_bool().unwrap();
        match input["operation"].as_str().unwrap() {
            "construct" => value = Record::loaded(value.location, ASSET, &host).unwrap(),
            "replace" => value.replace(kind, ASSET, &mut host).unwrap(),
            "load" => {
                let mut registry = Registry::default();
                registry.loading_lazy = loading;
                assert!(registry.insert(kind, value));
                registry
                    .attempt(kind, &host.key.clone(), &mut host)
                    .unwrap();
                value = registry.get(kind, &host.key).unwrap().clone();
                loading = registry.loading_lazy;
            }
            _ => unreachable!(),
        }
        assert_eq!(
            json!({"record": record(&value), "asset_name": host.assets[&ASSET],
            "loading": loading, "events": host.events}),
            row["original"],
            "case {case}"
        );
    }
}
