use serde_json::{Value, json};
use sims_sim::pose::clothing::catalog_mount::{
    ArchiveHost, DirectoryHost, Found, Manager, Metadata, Opened,
};

fn bytes(v: &Value) -> Vec<u8> {
    serde_json::from_value(v.clone()).unwrap()
}
fn seed(value: &Value) -> Manager {
    let mut manager = Manager::default();
    for directory in value.as_array().unwrap() {
        let files = manager
            .directory_mut(&bytes(&directory["directory"]))
            .unwrap();
        for file in directory["files"].as_array().unwrap() {
            let words: [u32; 4] = serde_json::from_value(file["words"].clone()).unwrap();
            let flags: [u8; 3] = serde_json::from_value(file["flags"].clone()).unwrap();
            assert!(
                files
                    .insert(
                        &bytes(&file["name"]),
                        Metadata {
                            archive: words[0],
                            offset: words[1],
                            reserved: words[2],
                            size: words[3],
                            archived: flags[0],
                            writable: flags[1],
                            directory: flags[2],
                        }
                    )
                    .unwrap()
            );
        }
    }
    manager
}
fn state(manager: &Manager) -> Value {
    json!(manager.directories().map(|(directory, files)| json!({"directory": directory,
        "files": files.iter().map(|(name,m)| json!({"name": name,
            "words": [m.archive,m.offset,m.reserved,m.size], "flags": [m.archived,m.writable,m.directory]})).collect::<Vec<_>>()
    })).collect::<Vec<_>>())
}
fn canonical(mut value: Value) -> Value {
    // Native map storage is a service in these probes. Compare its records by
    // key; actual hash iteration/order has independent native map fixtures.
    match &mut value {
        Value::Array(rows) => {
            for row in rows.iter_mut() {
                *row = canonical(row.take());
            }
            for key in ["directory", "name"] {
                if !rows.is_empty() && rows.iter().all(|r| r.get(key).is_some()) {
                    rows.sort_by_key(|r| bytes(&r[key]));
                }
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                *v = canonical(v.take());
            }
        }
        _ => (),
    }
    value
}
struct Scan {
    input: Value,
    events: Vec<Value>,
}
impl DirectoryHost for Scan {
    type Search = usize;
    fn first(&mut self, pattern: &[u8]) -> Result<Option<usize>, String> {
        self.events.push(json!(["first", pattern]));
        Ok((!self.input["entries"].as_array().unwrap().is_empty()).then_some(0))
    }
    fn next(&mut self, cursor: &mut usize) -> Result<Option<Found>, String> {
        *cursor += 1;
        let row = self.input["entries"].as_array().unwrap().get(*cursor);
        self.events.push(json!(["next", row.map(|_| *cursor)]));
        Ok(row.map(|r| Found {
            name: bytes(&r["name"]),
            attributes: r["attributes"].as_u64().unwrap() as u32,
        }))
    }
    fn close(&mut self, _: usize) -> Result<(), String> {
        self.events.push(json!(["close"]));
        Ok(())
    }
    fn recurse(
        &mut self,
        manager: &mut Manager,
        path: &[u8],
        recursive: bool,
        mount: bool,
    ) -> Result<(), String> {
        self.events.push(json!([
            "recurse",
            path,
            u8::from(recursive),
            u8::from(mount),
            state(manager)
        ]));
        Ok(())
    }
    fn mount(&mut self, manager: &mut Manager, path: &[u8]) -> Result<bool, String> {
        self.events.push(json!(["mount", path, state(manager)]));
        Ok(self.input["mount_result"] == 1)
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_directory_scan_matches_skips_metadata_and_dispatch_order() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/avatar-directory-scan.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 22);
    for (i, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let mut manager = seed(&input["existing"]);
        let mut host = Scan {
            input: input.clone(),
            events: vec![],
        };
        manager
            .scan(
                &bytes(&input["directory"]),
                input["recursive"] == 1,
                input["mount"] == 1,
                &mut host,
            )
            .unwrap();
        assert_eq!(
            canonical(json!({"events": host.events, "directories": state(&manager)})),
            canonical(row["original"].clone()),
            "case {i}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_file_queries_preserve_registration_and_physical_fallback() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/avatar-file-queries.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 64);
    for (i, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let manager = seed(&input["existing"]);
        let path = bytes(&input["path"]);
        let before = state(&manager);
        let mut events = vec![];
        let registered = manager.registered(&path).unwrap();
        let mounted = manager.contains_path(&path).unwrap();
        let exists = manager
            .exists(&path, |path| {
                let value = input["physical"].as_bool().unwrap();
                events.push(json!(["physical", path, value]));
                Ok(value)
            })
            .unwrap();
        assert_eq!(
            json!({"registered":u8::from(registered),"mounted":u8::from(mounted),"exists":u8::from(exists),"events":events}),
            row["original"],
            "case {i}"
        );
        assert_eq!(state(&manager), before);
    }
}
struct Archive {
    operation: Value,
    serial: u32,
    events: Vec<Value>,
}
impl ArchiveHost for Archive {
    fn open(&mut self, path: &[u8]) -> Result<Option<Opened>, String> {
        let open = self.operation["open"].as_bool().unwrap();
        self.events.push(json!(["open", path, open]));
        Ok(open.then(|| Opened {
            identity: self.serial,
            bytes: bytes(&self.operation["data"]),
        }))
    }
    fn open_failed(&mut self, path: &[u8]) -> Result<(), String> {
        self.events.push(json!(["error", path]));
        self.events.push(json!(["dispose", path]));
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_archive_mount_matches_winners_ancestors_and_failed_read_cache() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/avatar-archive-mount.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 24);
    for (case, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let mut manager = seed(&row["input"]["existing"]);
        for (i, operation) in row["input"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let mut host = Archive {
                operation: operation.clone(),
                serial: i as u32 + 1,
                events: vec![],
            };
            let result = manager
                .mount(&bytes(&operation["path"]), &mut host)
                .unwrap();
            if let Some(offset) = result.manifest_offset {
                host.events.push(json!(["seek", offset]));
            }
            let expected = &row["original"][i];
            assert_eq!(
                result.success,
                expected["returned"] == 1,
                "case {case}, operation {i}"
            );
            assert_eq!(
                json!(result.consumed),
                expected["consumed"],
                "case {case} consumed"
            );
            assert_eq!(
                json!(u8::from(manager.has_archives)),
                expected["mounted_flag"]
            );
            assert_eq!(host.events, *expected["events"].as_array().unwrap());
            assert_eq!(
                canonical(state(&manager)),
                canonical(expected["directories"].clone()),
                "case {case}, operation {i} records"
            );
            let mut archives: Vec<Vec<u8>> =
                serde_json::from_value(expected["archives"].clone()).unwrap();
            archives.sort();
            assert_eq!(
                manager
                    .archives()
                    .map(|(p, _)| p.to_vec())
                    .collect::<Vec<_>>(),
                archives
            );
        }
    }
}
