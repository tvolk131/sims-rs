use serde_json::{Value, json};
use sims_formats::animation::{Animation, Motion, TimedProperty};
use sims_sim::pose::movement::calibration;

fn numbers(actual: &Value, expected: &Value) {
    match (actual, expected) {
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len());
            for (key, value) in b {
                numbers(&a[key], value);
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                numbers(a, b);
            }
        }
        _ => assert_eq!(
            actual.as_f64().unwrap() as f32,
            expected.as_f64().unwrap() as f32
        ),
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_movement_measurements_and_anchor_selection() {
    // Outputs recorded from unchanged 58831d blocks, 588ed7 and 588df8.
    // Resource lookup / absolute anchor poses are explicit native service inputs.
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-movement-calibration.json")).unwrap();
    for row in report["distances"].as_array().unwrap() {
        let clips: [Option<f32>; 5] =
            serde_json::from_value(row["input"]["distances"].clone()).unwrap();
        let actual = calibration::Distances::new(clips.map(|v| v.unwrap_or(0.))).unwrap();
        numbers(&json!(actual), &row["original"]);
        assert_eq!(row["calls"], json!([19, 21, 20, 22, 23]));
    }
    for row in report["anchors"].as_array().unwrap() {
        let input = &row["input"];
        let phase_window = calibration::phases(
            serde_json::from_value(input["times"].clone()).unwrap(),
            input["duration"].as_f64().unwrap() as f32,
        )
        .unwrap();
        let stop_distance =
            calibration::stop_distances(serde_json::from_value(input["z"].clone()).unwrap())
                .unwrap();
        numbers(
            &json!({"phase_window":phase_window,"stop_distance":stop_distance}),
            &row["original"],
        );
        numbers(&json!(phase_window), &row["samples"]);
    }
    for row in report["properties"].as_array().unwrap() {
        let input = &row["input"];
        let motions = input["streams"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, events)| {
                let bone = format!("bone{i}");
                Motion {
                    bone: bone.clone(),
                    frames: 30,
                    duration_ms: 1000.,
                    translation: None,
                    rotation: None,
                    properties: vec![],
                    event_lists: vec![
                        events
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|e| TimedProperty {
                                bone: bone.clone(),
                                time_ms: e["time"].as_i64().unwrap() as i32,
                                pairs: serde_json::from_value(e["properties"].clone()).unwrap(),
                            })
                            .collect(),
                    ],
                }
            })
            .collect();
        let clip = Animation {
            name: "synthetic".into(),
            skill: "synthetic".into(),
            duration_ms: 1000.,
            distance: 0.,
            moving: false,
            translation_count: 0,
            rotation_count: 0,
            frames: 30,
            motions,
            events: vec![],
        };
        let actual = calibration::property(
            &clip,
            input["key"].as_str().unwrap(),
            input["value"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(json!(actual), row["original"]);
    }
    for key in ["distances", "anchors", "properties"] {
        assert_eq!(report[key].as_array().unwrap().len(), 120);
    }
}
