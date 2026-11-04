use serde::Deserialize;
use serde_json::{Value, json};
use sims_sim::pose::clothing::catalog_paths::{self, Host, Kind};
#[derive(Deserialize)]
struct Input {
    prefix: String,
    flags: i16,
    split: u8,
    registered: Vec<String>,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    original: Value,
}
#[derive(Deserialize)]
struct Report {
    cases: Vec<Case>,
}
struct Mounts {
    registered: Vec<String>,
    events: Vec<Value>,
}
impl Host for Mounts {
    fn registered(&mut self, path: &[u8]) -> Result<bool, String> {
        let path = String::from_utf8(path.to_vec()).unwrap();
        let found = self.registered.contains(&path);
        self.events.push(json!(["registered", path, found]));
        Ok(found)
    }
    fn register(&mut self, path: &[u8], recursive: bool, archives: bool) -> Result<(), String> {
        let path = String::from_utf8(path.to_vec()).unwrap();
        self.events.push(json!([
            "register",
            path,
            u8::from(recursive),
            u8::from(archives)
        ]));
        self.registered.push(path);
        Ok(())
    }
    fn exists(&mut self, _: &[u8]) -> Result<bool, String> {
        panic!("unexpected existence query")
    }
    fn providers(&mut self) -> Result<Vec<Vec<u8>>, String> {
        panic!("unexpected provider query")
    }
    fn load(&mut self, _: Kind, _: &[Vec<u8>]) -> Result<(), String> {
        panic!("unexpected catalog load")
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_initial_mounts_preserve_path_flags_and_order() {
    let report: Report =
        serde_json::from_str(include_str!("fixtures/avatar-initial-mounts.json")).unwrap();
    assert_eq!(report.cases.len(), 528);
    for (i, case) in report.cases.into_iter().enumerate() {
        let mut host = Mounts {
            registered: case.input.registered,
            events: vec![],
        };
        catalog_paths::initial_mounts(
            case.input.prefix.as_bytes(),
            case.input.flags,
            case.input.split != 0,
            &mut host,
        )
        .unwrap();
        assert_eq!(json!({"events": host.events}), case.original, "case {i}");
    }
}
