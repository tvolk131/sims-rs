use serde_json::{Value, json};
use sims_vm::animation::{Property, property};
#[cfg(feature = "private-fixtures")]
#[test]
fn native_timed_property_dispatch() {
    let report: Value =
        serde_json::from_slice(include_bytes!("fixtures/animation-properties.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut hands = [11i16, 22];
        let mut censor = 33i16;
        let mut interruptible = 7i32;
        let mut events = vec![];
        match property(c["key"].as_str().unwrap(), c["value"].as_str().unwrap()).unwrap() {
            Property::Event(v) => events.push(v),
            Property::Hand(s, v) => hands[s] = v,
            Property::Censor(v) => censor = v,
            Property::Interruptible(v) => interruptible = v,
            Property::Ignored => (),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            json!({"error":null,"hands":hands,"censor":censor,"interruptible":interruptible as u32,"events":events}),
            row["original"],
            "{c}"
        );
    }
}
