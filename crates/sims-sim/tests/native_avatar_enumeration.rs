use serde_json::{Value, json};
use sims_sim::pose::clothing::{
    catalog_files::{self, Host},
    name_map::Files,
};
use std::collections::BTreeMap;

struct Directories {
    maps: BTreeMap<Vec<u8>, Files<usize>>,
    events: Vec<Value>,
}
impl Host for Directories {
    type Source = usize;
    fn files(&mut self, directory: &[u8]) -> Result<Option<&Files<usize>>, String> {
        let found = self.maps.get(&Files::<usize>::key(directory)?);
        self.events
            .push(json!(["directory", directory, found.is_some()]));
        Ok(found)
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_enumeration_preserves_type_path_hash_order_and_duplicate_identity() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/avatar-enumeration.json")).unwrap();
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let mut host = Directories {
            maps: BTreeMap::new(),
            events: vec![],
        };
        for directory in input["directories"].as_array().unwrap() {
            let path: Vec<u8> = serde_json::from_value(directory["path"].clone()).unwrap();
            let mut map = Files::default();
            let files: Vec<Vec<u8>> = serde_json::from_value(directory["files"].clone()).unwrap();
            for (i, name) in files.iter().enumerate() {
                map.insert(name, i).unwrap();
            }
            host.maps.insert(Files::<usize>::key(&path).unwrap(), map);
        }
        let paths: Vec<Vec<u8>> = serde_json::from_value(input["paths"].clone()).unwrap();
        let aliases: Option<Vec<u32>> = serde_json::from_value(input["aliases"].clone()).unwrap();
        let resources = catalog_files::enumerate(
            &paths,
            input["kind"].as_u64().unwrap() as u32,
            aliases.as_deref(),
            &mut host,
        )
        .unwrap();
        let records: Vec<Value> = resources
            .iter()
            .map(|r| {
                json!({
                    "directory": r.directory, "name": r.name.bytes,
                    "kind": r.kind, "identity": r.name.identity
                })
            })
            .collect();
        assert_eq!(json!(records), row["original"]["records"], "case {case}");
        let directory_events: Vec<Value> = row["original"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e[0] == "directory")
            .cloned()
            .collect();
        assert_eq!(host.events, directory_events, "case {case} lookups");
        assert_eq!(row["original"]["returned"], 0);
        for resource in resources {
            // The retained payload must belong to the selected provider, not
            // merely to a same-named file from a later search path.
            let map = &host.maps[&Files::<usize>::key(&resource.directory).unwrap()];
            let filename = map
                .iter()
                .find(|(name, _)| {
                    sims_sim::pose::clothing::resource_name::Name::parse(name).unwrap()
                        == resource.name
                })
                .unwrap();
            assert_eq!(resource.source, *filename.1);
        }
    }
}
