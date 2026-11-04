#![cfg(feature = "private-fixtures")]

use serde_json::Value;
use sims_sim::pose::layer::{Descriptor, Layer, MotionInput, Start};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/native-layer-init.json")).unwrap()
}
fn f(v: &Value) -> f32 {
    v.as_f64().unwrap() as f32
}
fn i(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn compare(a: &Value, b: &Value, path: &str) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                compare(&a[k], v, &format!("{path}.{k}"));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}");
            for (n, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &format!("{path}[{n}]"));
            }
        }
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => {
            assert_eq!(
                (a.as_f64().unwrap() as f32).to_bits(),
                (b.as_f64().unwrap() as f32).to_bits(),
                "{path}"
            );
        }
        _ => assert_eq!(a, b, "{path}"),
    }
}
#[test]
fn native_construction_binds_ordered_motions_and_initializes_all_clock_state() {
    let report = fixture();
    assert_eq!(report["initializers"].as_array().unwrap().len(), 512);
    for (n, row) in report["initializers"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let c = &row["input"];
        let motions = c["motions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| MotionInput {
                bone_id: m["bone_id"].as_u64().unwrap() as u32,
                frames: m["frames"].as_u64().unwrap() as u32,
                events: m["events"].as_u64().unwrap() as usize,
            })
            .collect::<Vec<_>>();
        let slots: Vec<Option<usize>> = serde_json::from_value(c["bone_slots"].clone()).unwrap();
        let layer = Layer::new(
            "synthetic".into(),
            f(&c["duration"]),
            &motions,
            &slots,
            Start {
                priority: i(&c["priority"]),
                speed: f(&c["speed"]),
                weight: f(&c["weight"]),
                now: i(&c["now"]),
            },
        )
        .unwrap();
        compare(
            &serde_json::to_value(&layer).unwrap(),
            &row["original"],
            &format!("initializer {n}"),
        );
        let saved = serde_json::to_vec(&layer).unwrap();
        let restored: Layer = serde_json::from_slice(&saved).unwrap();
        compare(
            &serde_json::to_value(restored).unwrap(),
            &row["original"],
            &format!("restored {n}"),
        );
    }
}

fn descriptor(row: &Value, n: usize) {
    let c = &row["input"];
    let d = Descriptor::parse(c["text"].as_str().unwrap());
    let expected = &row["original"];
    let mut events = vec!["clear"];
    if let Some(d) = d {
        events.push("lookup");
        if c["found"].as_bool().unwrap() {
            events.extend(["clock", "create"]);
            let parsed = serde_json::to_value(&d).unwrap();
            let p = &expected["parameters"];
            for field in ["name", "priority", "speed", "weight"] {
                compare(
                    &parsed[field],
                    &p[field],
                    &format!("descriptor {n} {field}"),
                );
            }
            if c["created"].as_bool().unwrap() {
                for field in ["phase", "mode", "translation_policy", "rotation_policy"] {
                    compare(
                        &parsed[field],
                        &expected["layer"][field],
                        &format!("descriptor {n} {field}"),
                    );
                }
            } else {
                assert!(expected["layer"].is_null());
            }
        }
    }
    assert_eq!(
        serde_json::to_value(events).unwrap(),
        expected["events"],
        "descriptor {n}"
    );
}
#[test]
fn native_saved_descriptors_preserve_atoi_scaling_and_incomplete_input_behavior() {
    let report = fixture();
    assert_eq!(report["descriptors"].as_array().unwrap().len(), 636);
    for (n, row) in report["descriptors"].as_array().unwrap().iter().enumerate() {
        descriptor(row, n);
    }
}
