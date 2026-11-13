use serde_json::{Value, json};
use sims_vm::routing::{self, Preparation};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_relative_route_descriptor_and_resume_vectors() {
    let report: Value = serde_json::from_str(include_str!("fixtures/relative-route.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let operands = serde_json::from_value(c["operands"].clone()).unwrap();
        let result =
            routing::relative_position(&operands, c["continuation"].as_u64().unwrap() as u32);
        let (called, diagnostics, descriptor) = match result {
            Preparation::Continue => (true, 0, Value::Null),
            Preparation::InvalidFacing => (false, 1, Value::Null),
            Preparation::Start(r) => (true, 0, json!(r.words())),
        };
        assert_eq!(n["error"], Value::Null, "{c}");
        assert_eq!(n["called"], called, "{c}");
        assert_eq!(n["diagnostic_calls"], diagnostics, "{c}");
        assert_eq!(n["descriptor"], descriptor, "{c}");
        assert_eq!(
            n["returned"],
            if called {
                c["status"].clone()
            } else {
                json!(0)
            },
            "{c}"
        );
    }
}
