use serde::Deserialize;
use serde_json::{Value, json};
use sims_sim::object_catalog::access::{Access, Host};

#[derive(Deserialize)]
struct Input {
    opened: bool,
    writable: u8,
    request: u32,
    write: bool,
    read: bool,
}
struct Services {
    write: bool,
    read: bool,
    events: Vec<Value>,
}
impl Host for Services {
    fn open(&mut self, writable: bool) -> Result<bool, String> {
        self.events.push(json!(["open", u8::from(writable)]));
        Ok(if writable { self.write } else { self.read })
    }
    fn buffer(&mut self, read: u32, write: u32) -> Result<(), String> {
        self.events.push(json!(["buffer", read, write]));
        Ok(())
    }
    fn close(&mut self) -> Result<(), String> {
        self.events.push(json!(["close"]));
        Ok(())
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_stream_open_fallback_close_and_flags_match() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/resource-access.json")).unwrap();
    let cases = report["opens"].as_array().unwrap();
    assert_eq!(cases.len(), 120);
    for (i, case) in cases.iter().enumerate() {
        let input: Input = serde_json::from_value(case["input"].clone()).unwrap();
        let mut state = Access {
            opened: input.opened,
            writable: input.writable,
        };
        let mut host = Services {
            write: input.write,
            read: input.read,
            events: vec![],
        };
        let returned = state.open(input.request, &mut host).unwrap();
        let opened = state;
        // Restore between open and close, including the unusual stale flag.
        state = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        let close = state.close(&mut host).unwrap();
        assert_eq!(
            json!({"returned":returned,"state":opened,"close":close,"after":state,"events":host.events}),
            case["original"],
            "case {i}"
        );
    }
}
