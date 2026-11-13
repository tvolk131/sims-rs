use serde_json::{Value, json};
use sims_vm::animation::CarrySelector;
#[cfg(feature = "private-fixtures")]
#[test]
fn native_automatic_carry_state_and_clip_boundaries() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/carry-animation.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let mut state = CarrySelector {
            mode: c["mode"].as_i64().unwrap() as i32,
            selection: c["selection"].as_i64().unwrap() as i32,
        };
        let mut calls = vec![];
        let mut clip = c["clip"].as_bool().unwrap();
        if let Some(index) = state.update(c["held"] == true, c["pickup"] == 1) {
            calls.push(json!(["clear"]));
            clip = false;
            if let Some(index) = index {
                calls.push(json!(["lookup", index]));
                if c["resource"] == true {
                    calls.push(json!(["start"]));
                    clip = c["start"] == true;
                    if clip {
                        calls.push(json!(["priority", 10]));
                    }
                }
            }
        }
        assert_eq!(json!(calls), n["calls"], "{c}");
        assert_eq!(json!(state.mode), n["mode"], "{c}");
        assert_eq!(json!(state.selection), n["selection"], "{c}");
        assert_eq!(json!(clip), n["clip"], "{c}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_explicit_carry_start_clear_preserves_foreground() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/carry-animation.json")).unwrap();
    for row in report["scripts"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let mut s = CarrySelector {
            mode: c["mode"].as_i64().unwrap() as i32,
            selection: c["selection"].as_i64().unwrap() as i32,
        };
        s.script_clip(c["number"] != 0);
        assert_eq!(json!(s.mode), n["mode"]);
        assert_eq!(json!(s.selection), n["selection"]);
        assert_eq!(c["number"] != 0, n["clip"] == true);
        assert_eq!(n["foreground_preserved"], true);
        assert_eq!(n["returned"], 1);
    }
}
