use serde::Deserialize;
use serde_json::{Value, json};
use sims_vm::{
    ObjectId,
    load_indicators::{GUIDS, Host, Indicators},
};
#[derive(Deserialize)]
struct Input {
    available: [bool; 2],
    created: [i16; 2],
    initial: [i16; 2],
    flags: u16,
}
struct Supplied {
    input: Input,
    flags: Vec<u16>,
    events: Vec<Value>,
}
impl Host for Supplied {
    fn lookup(&mut self, guid: u32) -> Result<bool, String> {
        self.events.push(json!(["lookup", guid]));
        Ok(self.input.available[GUIDS.iter().position(|&g| g == guid).unwrap()])
    }
    fn create(&mut self, guid: u32) -> Result<Option<ObjectId>, String> {
        let id = self.input.created[GUIDS.iter().position(|&g| g == guid).unwrap()];
        self.events.push(json!(["create", guid, id]));
        Ok((id != 0).then_some(id))
    }
    fn hide(&mut self, id: ObjectId) -> Result<(), String> {
        self.flags[id as usize - 1] |= 0x4000;
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn load_indicator_order_cache_and_flags_match_original() {
    let data: Value = serde_json::from_str(include_str!("fixtures/load-indicators.json")).unwrap();
    assert_eq!(data["cases"].as_array().unwrap().len(), 144);
    for c in data["cases"].as_array().unwrap() {
        let input: Input = serde_json::from_value(c["input"].clone()).unwrap();
        let mut state = Indicators {
            primary: (input.initial[0] != 0).then_some(input.initial[0]),
            secondary: (input.initial[1] != 0).then_some(input.initial[1]),
        };
        let mut host = Supplied {
            flags: vec![input.flags; 8],
            input,
            events: vec![],
        };
        state.initialize(&mut host).unwrap();
        assert_eq!(
            json!({"pointers":[state.primary.unwrap_or(0),state.secondary.unwrap_or(0)],"flags":host.flags,"events":host.events}),
            c["original"],
            "{}",
            c["input"]
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn saved_initializer_retains_lookups_factories_and_post_hide_cache_writes() {
    use sims_vm::load_indicators::{Initialization, Reply, Request};
    let data: Value = serde_json::from_str(include_str!("fixtures/load-indicators.json")).unwrap();
    for c in data["cases"].as_array().unwrap() {
        let input: Input = serde_json::from_value(c["input"].clone()).unwrap();
        let mut indicators = Indicators {
            primary: (input.initial[0] != 0).then_some(input.initial[0]),
            secondary: (input.initial[1] != 0).then_some(input.initial[1]),
        };
        let mut host = Supplied {
            flags: vec![input.flags; 8],
            input,
            events: vec![],
        };
        let mut call = Initialization::default();
        while let Some(request) = call.advance(&indicators).unwrap() {
            (call, indicators) =
                serde_json::from_slice(&serde_json::to_vec(&(&call, &indicators)).unwrap())
                    .unwrap();
            call.validate().unwrap();
            let before = (call.clone(), indicators.clone());
            assert_eq!(call.advance(&indicators).unwrap(), Some(request));
            let wrong = if matches!(request, Request::Hide(_)) {
                Reply::Available(true)
            } else {
                Reply::Done
            };
            assert!(call.acknowledge(&mut indicators, request, wrong).is_err());
            assert_eq!((call.clone(), indicators.clone()), before);
            let reply = match request {
                Request::Lookup(i) => Reply::Available(host.lookup(GUIDS[i]).unwrap()),
                Request::Create(i) => Reply::Created(host.create(GUIDS[i]).unwrap()),
                Request::Hide(id) => {
                    host.hide(id).unwrap();
                    Reply::Done
                }
            };
            // The cache store follows the hide callback. A save here must not
            // publish the new primary/secondary before that callback returns.
            (call, indicators) =
                serde_json::from_slice(&serde_json::to_vec(&(&call, &indicators)).unwrap())
                    .unwrap();
            assert_eq!(indicators, before.1);
            call.acknowledge(&mut indicators, request, reply).unwrap();
            assert!(call.acknowledge(&mut indicators, request, reply).is_err());
        }
        assert!(call.complete());
        assert_eq!(
            json!({"pointers":[indicators.primary.unwrap_or(0),indicators.secondary.unwrap_or(0)],"flags":host.flags,"events":host.events}),
            c["original"]
        );
    }
}
