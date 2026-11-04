use serde_json::{Value, json};
use sims_sim::pose::clothing::catalog_open::{self, Host};
struct Services {
    input: Value,
    events: Vec<Value>,
    attempt: usize,
}
fn bytes(v: &Value) -> Vec<u8> {
    serde_json::from_value(v.clone()).unwrap()
}
impl Host for Services {
    type Search = Vec<u8>;
    fn open(&mut self, path: &[u8]) -> Result<u32, String> {
        let status = self.input["statuses"][self.attempt].as_u64().unwrap() as u32;
        self.attempt += 1;
        self.events.push(json!(["open", path, status]));
        Ok(status)
    }
    fn find_first(&mut self, pattern: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.events
            .push(json!(["find", pattern, self.input["found"]]));
        Ok((!self.input["found"].is_null()).then(|| bytes(&self.input["found"])))
    }
    fn close_search(&mut self, _: &Vec<u8>) -> Result<(), String> {
        self.events.push(json!(["find-close"]));
        Ok(())
    }
    fn found_identity(&mut self, name: &Vec<u8>) -> Result<u32, String> {
        let id = self.input["parsed"].as_u64().unwrap() as u32;
        self.events.push(json!(["parse", name, id]));
        Ok(id)
    }
    fn set_kind(&mut self, kind: u32) -> Result<(), String> {
        self.events.push(json!(["type", kind]));
        Ok(())
    }
    fn set_name(&mut self, name: &[u8]) -> Result<(), String> {
        self.events.push(json!(["name", name]));
        Ok(())
    }
    fn set_identity(&mut self, id: u32) -> Result<(), String> {
        self.events.push(json!(["identity", id]));
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn named_source_lookup_matches_original_unnumbered_numbered_and_os_fallbacks() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/avatar-source-open.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 121);
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let mut kind = [0; 4];
        kind[..3].copy_from_slice(input["type"].as_str().unwrap().as_bytes());
        let mut host = Services {
            input: input.clone(),
            events: vec![],
            attempt: 0,
        };
        let returned = catalog_open::named(
            &bytes(&input["directory"]),
            &bytes(&input["name"]),
            u32::from_be_bytes(kind),
            input["identity"].as_u64().unwrap() as u32,
            &mut host,
        )
        .unwrap();
        assert_eq!(
            json!({"returned":returned,"events":host.events}),
            row["original"],
            "case {i}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn ordered_plain_paths_match_original_first_success_and_status_normalization() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/resource-plain-paths.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 180);
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let mut kind = [0; 4];
        kind[..3].copy_from_slice(input["type"].as_str().unwrap().as_bytes());
        let paths: Vec<Vec<u8>> = serde_json::from_value(input["paths"].clone()).unwrap();
        let name: Option<Vec<u8>> = serde_json::from_value(input["name"].clone()).unwrap();
        let mut host = Services {
            input: input.clone(),
            events: vec![],
            attempt: 0,
        };
        let returned =
            catalog_open::plain_paths(&paths, name.as_deref(), u32::from_be_bytes(kind), &mut host)
                .unwrap();
        assert_eq!(
            json!({"returned":returned,"events":host.events}),
            row["original"],
            "case {i}"
        );
    }
}
