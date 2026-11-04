use serde_json::{Value, json};
use sims_vm::image_cache::{
    Entry, State,
    persistence::{self, Host},
};
struct Services {
    input: Value,
    events: Vec<Value>,
}
impl Host for Services {
    fn resource(&mut self, guid: u32) -> Result<Option<u32>, String> {
        self.events.push(json!(["resource", guid]));
        Ok(self.input["resource"].as_bool().unwrap().then_some(7))
    }
    fn file(&mut self, _: u32) -> Result<Option<u32>, String> {
        Ok(self.input["file"].as_bool().unwrap().then_some(8))
    }
    fn writable(&mut self, _: u32) -> Result<bool, String> {
        self.events.push(json!(["writable"]));
        Ok(self.input["writable"].as_bool().unwrap())
    }
    fn lookup(&mut self, _: u32, tag: u32, chunk: u32) -> Result<u32, String> {
        self.events.push(json!(["lookup", tag, chunk, 0]));
        Ok(if self.input["existing"] == true {
            99
        } else {
            0
        })
    }
    fn delete(&mut self, _: u32, handle: u32) -> Result<(), String> {
        self.events.push(json!(["delete", handle]));
        Ok(())
    }
    fn encode(&mut self, _: u32) -> Result<Option<u32>, String> {
        self.events.push(json!(["encode"]));
        Ok(self.input["encode"].as_bool().unwrap().then_some(9))
    }
    fn add(
        &mut self,
        _: u32,
        buffer: Option<u32>,
        tag: u32,
        chunk: u32,
        name: &[u8],
    ) -> Result<(), String> {
        self.events
            .push(json!(["add", buffer.is_some(), tag, chunk, name, 1]));
        Ok(())
    }
    fn release(&mut self, _: u32, buffer: Option<u32>) -> Result<(), String> {
        self.events.push(json!(["release", buffer.is_some()]));
        Ok(())
    }
    fn error(&mut self, _: u32) -> Result<i32, String> {
        Ok(self.input["error"].as_i64().unwrap() as i32)
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_persistence_guards_cache_delete_export_add_release_and_error_match() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/portrait-persist.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 268);
    for (i, case) in cases.iter().enumerate() {
        let c = &case["input"];
        let mut cache = State {
            entries: vec![Entry::new(42)],
            ..Default::default()
        };
        cache.entries[0].resource = c["cached"].as_bool().unwrap().then_some(7);
        let mut host = Services {
            input: c.clone(),
            events: vec![],
        };
        let name: Vec<u8> = serde_json::from_value(c["name"].clone()).unwrap();
        let returned = persistence::persist(
            &mut cache,
            if c["record"] == true { 0 } else { u32::MAX },
            c["image"].as_bool().unwrap().then_some(1),
            2002,
            &name,
            &mut host,
        )
        .unwrap();
        assert_eq!(
            json!({"returned":returned,"cached":cache.entries[0].resource.is_some(),"events":host.events}),
            case["original"],
            "case {i}"
        );
    }
}
