use serde_json::{Value, json};
use sims_sim::clock::{ClockEvent, LotClock};

fn main() {
    let path = std::env::args().nth(1).expect("private clock report path");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}

pub fn compare(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = report["cases"].as_array().unwrap();
    for (i, row) in rows.iter().enumerate() {
        let mut clock: LotClock = serde_json::from_value(row["input"].clone()).unwrap();
        clock.validate().unwrap();
        let complete = row["input"]["complete"].as_bool().unwrap();
        let mut callbacks = Vec::new();
        let mut events = Vec::new();
        for event in clock.finish_tick(complete) {
            match event {
                ClockEvent::DailyObjects => callbacks.push(json!(["daily_objects"])),
                ClockEvent::DailyNeighborhood => callbacks.push(json!(["daily_neighborhood"])),
                ClockEvent::WorldUpdate { partial } => {
                    callbacks.push(json!(["world_update", u8::from(partial)]))
                }
                ClockEvent::Lighting => callbacks.push(json!(["lighting"])),
                ClockEvent::Signal { kind, argument } => events.push(json!([kind, argument])),
            }
        }
        let original = &row["original"];
        let actual = serde_json::to_value(clock).unwrap();
        for (field, value) in actual.as_object().unwrap() {
            assert_eq!(value, &original[field], "clock {i}, {field}");
        }
        assert_eq!(json!(callbacks), original["callbacks"], "clock {i}");
        assert_eq!(json!(events), original["events"], "clock {i}");
        assert_eq!(complete, original["returned"].as_bool().unwrap());
        assert_eq!(original["error"], Value::Null);
        // The simulator restores the external random stream around its own pass.
        assert_eq!(original["rng"], 456);
        assert_eq!(original["lot_rng"], 123);
    }
    println!("{} original lot-clock comparisons passed", rows.len());
}
