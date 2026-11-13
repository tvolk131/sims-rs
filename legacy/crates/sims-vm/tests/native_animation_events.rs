use serde_json::{Value, json};
use sims_vm::animation::{Poll, poll_events};

#[cfg(feature = "private-fixtures")]
#[test]
fn original_foreground_event_sequence_and_completion_vectors() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/animation-events.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        assert_eq!(n["foreground_retained"], true);
        let mut seen = c["seen"].as_i64().unwrap() as i32;
        let result = poll_events(
            c["count"].as_u64().unwrap() as u8 as i8,
            c["reverse"] == 1,
            &mut seen,
            c["event"].as_i64().map(|v| v as i16),
            c["ended"] == 1 || (c["interruptible"] == true && c["interrupt"] == true),
        );
        let mut frame = vec![50, 60, 70, 80, 90, 91, 92, 93];
        let mut error = Value::Null;
        let returned = match result {
            Poll::Waiting => json!(2),
            Poll::Completed => json!(1),
            Poll::Event(value) => {
                let argc = c["args"].as_u64().unwrap_or(4) as usize;
                let locals = c["locals"].as_u64().unwrap_or(4) as usize;
                let local = c["local"] == 1;
                let index = c["index"].as_u64().unwrap_or(1) as usize;
                if (local && locals <= index) || (!local && argc == 0) {
                    error = json!(8);
                    Value::Null
                } else {
                    frame[if local { argc + index } else { 0 }] = i32::from(value);
                    json!(0)
                }
            }
        };
        assert_eq!(json!(seen), n["seen"], "{c}");
        assert_eq!(returned, n["returned"], "{c}");
        assert_eq!(error, n["error"], "{c}");
        assert_eq!(json!(frame), n["frame"], "{c}");
        assert_eq!(
            json!(u8::from(result != Poll::Completed)),
            n["active"],
            "{c}"
        );
        assert_eq!(
            json!(if result == Poll::Completed { 0 } else { 99 }),
            n["person19"],
            "{c}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_animation_mode_bits_select_pickup_foreground() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/animation-start.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let flags = row["input"]["flags"].as_u64().unwrap() as u8;
        let n = &row["original"];
        let mode = sims_vm::animation::mode(flags);
        assert_eq!(json!(mode), n["mode"]);
        assert_eq!(json!(u8::from(mode == 3)), n["pickup"]);
        assert_eq!(json!(mode != 3), n["carry_present"]);
        assert_eq!(n["interruptible"], u8::from(flags & 8 != 0));
        assert_eq!(n["active"], 1);
        assert_eq!(n["returned"], 2);
    }
}
