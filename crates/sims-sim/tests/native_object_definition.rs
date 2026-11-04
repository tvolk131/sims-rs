use serde_json::{Value, json};
use sims_sim::object_catalog::definition::{Catalog, Host, Settings};

fn bytes(v: &Value) -> Vec<u8> {
    serde_json::from_value(v.clone()).unwrap()
}
struct Services<'a> {
    row: &'a Value,
    data: Vec<u8>,
    first_data: bool,
    events: Vec<Value>,
}
impl Host for Services<'_> {
    type Chunk = ();
    fn definition(&mut self, resource: u32, id: u32) -> Result<(), String> {
        self.events.push(json!(["definition", resource, id]));
        Ok(())
    }
    fn data(&mut self, _: ()) -> Result<&mut Vec<u8>, String> {
        if self.first_data {
            self.first_data = false;
            self.events.push(json!(["normalize"]));
        }
        Ok(&mut self.data)
    }
    fn name(&mut self, _: u32, _: ()) -> Result<Vec<u8>, String> {
        self.events.push(json!(["name"]));
        Ok(bytes(&self.row["name"]))
    }
    fn release_chunk(&mut self, resource: u32, _: ()) -> Result<(), String> {
        self.events.push(json!(["release_chunk", resource]));
        Ok(())
    }
    fn release_resource(&mut self, resource: u32) -> Result<(), String> {
        self.events.push(json!(["release_resource", resource]));
        Ok(())
    }
    fn sprite_resource(&mut self, resource: u32) -> Result<u32, String> {
        self.events.push(json!(["sprite_resource", resource]));
        Ok(resource)
    }
    fn glob(&mut self, resource: u32) -> Result<Option<Vec<u8>>, String> {
        self.events.push(json!(["glob", resource]));
        Ok((!self.row["glob"].is_null()).then(|| bytes(&self.row["glob"])))
    }
    fn acquire(&mut self, path: &[u8], people: u8) -> Result<Option<u32>, String> {
        let id = self.row["semiglobal"].as_u64().unwrap() as u32;
        self.events.push(json!(["acquire", path, people, id]));
        Ok((id != 0).then_some(id))
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_definition_normalization_registration_and_guid_overrides_match() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/object-definition.json")).unwrap();
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 129);
    for (i, case) in cases.iter().enumerate() {
        let input = &case["input"];
        let mut catalog = Catalog::default();
        let mut host = Services {
            row: &Value::Null,
            data: vec![],
            first_data: true,
            events: vec![],
        };
        for (step, row) in input["definitions"].as_array().unwrap().iter().enumerate() {
            host.row = row;
            host.data = bytes(&row["data"]);
            host.first_data = true;
            catalog
                .register(
                    row["resource"].as_u64().unwrap() as u32,
                    row["id"].as_u64().unwrap() as u32,
                    row["category"].as_u64().unwrap() as u32,
                    Settings {
                        filter_flagged: input["filter_flagged"] != 0,
                        global_directory: &bytes(&input["global_directory"]),
                        global_people: input["global_people"].as_u64().unwrap() as u8,
                        global: Some(3),
                    },
                    &mut host,
                )
                .unwrap();
            assert_eq!(
                json!(host.data),
                case["original"]["normalized"][step],
                "case {i} step {step} resident chunk"
            );
        }
        // These synchronous constructors have no external services with the
        // debugger registry empty. Their pointer bindings and defaults were
        // checked directly in the native probe; compare their semantic state.
        let events: Vec<_> = case["original"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| {
                !matches!(
                    e[0].as_str(),
                    Some("language" | "behavior" | "attributes" | "append" | "guid")
                )
            })
            .cloned()
            .collect();
        assert_eq!(host.events, events, "case {i} services");
        let entries: Vec<_> = catalog.entries.iter().map(|e| {
            assert_eq!(e.global, Some(3));
            let mut defaults = vec![0_u32; 18]; defaults[15] = u32::MAX; defaults[16] = u32::MAX;
            json!({"resource":e.key.resource,"semiglobal":e.semiglobal.unwrap_or(0),"sprite_resource":e.sprite_resource,
                    "name":e.name,"data":e.data,"index":e.index,"flags":e.flags,"language_callback":"object",
                "default_words":defaults,"default_word_a4":0})
        }).collect();
        let attributes: Vec<_> = catalog
            .attributes
            .iter()
            .map(|a| {
                assert!(a.values.iter().all(|&v| v == 0));
                json!([a.guid, a.values.len()])
            })
            .collect();
        let guids: Vec<_> = catalog
            .guids
            .iter()
            .map(|(&guid, &index)| json!([guid, index]))
            .collect();
        assert_eq!(
            json!(entries),
            case["original"]["entries"],
            "case {i} entries"
        );
        assert_eq!(
            json!(attributes),
            case["original"]["attributes"],
            "case {i} attributes"
        );
        assert_eq!(json!(guids), case["original"]["guids"], "case {i} guids");
    }
}
