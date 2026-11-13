use serde_json::{Value, json};
use sims_sim::object_catalog::directory::{Directory, Host};

struct Search {
    found: Vec<bool>,
    calls: usize,
    events: Vec<Value>,
}
impl Host for Search {
    type Search = ();
    fn find_first(&mut self, pattern: &[u8]) -> Result<Option<()>, String> {
        let found = self.found[self.calls % self.found.len()];
        self.calls += 1;
        self.events.push(json!(["find_first", pattern, found]));
        Ok(found.then_some(()))
    }
    fn find_close(&mut self, _: ()) -> Result<(), String> {
        self.events.push(json!(["find_close"]));
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_directory_resource_open_valid_name_and_close() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/object-directory.json")).unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        let input = &row["input"];
        let path: Vec<u8> = serde_json::from_value(input["path"].clone()).unwrap();
        let mut host = Search {
            found: serde_json::from_value(input["found"].clone()).unwrap(),
            calls: 0,
            events: vec![],
        };
        let mut directory = Directory::default();
        let opened = directory
            .open(&path, input["writable"].as_u64().unwrap() as u32, &mut host)
            .unwrap();
        let paths = directory.paths.clone();
        let valid = directory.valid(&mut host).unwrap();
        let filename = paths
            .as_ref()
            .map(|_| directory.filename().unwrap().to_vec());
        let closed = directory.close();
        assert!(directory.paths.is_none() && !directory.valid(&mut host).unwrap());
        assert_eq!(
            json!({"open":opened,"paths":paths,"valid":valid,"filename":filename,"close":closed,
            "error":directory.error,"events":host.events}),
            row["original"],
            "{input}"
        );
    }
}
