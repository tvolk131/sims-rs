#![cfg(feature = "private-fixtures")]

use serde_json::Value;
use sims_formats::animation::{Animation, Motion, Samples, TimedProperty};
use sims_sim::pose::{
    LocalBone,
    layer::{Layer, MotionInput, Start},
    sampling::BoneInput,
    update::Update,
};
use sims_sim::pose::{
    clock::Clock,
    events::{self, Entry, Scan},
};
fn report() -> Value {
    serde_json::from_str(include_str!("fixtures/native-layer-events.json")).unwrap()
}
fn step(c: &Value, clock: &mut Clock, cursors: &mut [Option<i32>], elapsed: i32) -> Value {
    let original_duration = c["original_duration"].as_f64().unwrap() as f32;
    let has_frames = c["frames"].as_u64().unwrap() != 0 && !cursors.is_empty();
    clock
        .advance(
            clock.current.wrapping_add(elapsed),
            original_duration,
            has_frames,
        )
        .unwrap();
    let mut dispatched = vec![];
    if has_frames && clock.weight > 0. {
        for (i, cursor) in cursors.iter_mut().enumerate() {
            if c["bone_priorities"][i].as_i64().unwrap() > c["priority"].as_i64().unwrap() {
                continue;
            }
            let Some(cursor) = cursor else {
                continue;
            };
            let entries = c["streams"][i]["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| Entry {
                    time: e["time"].as_i64().unwrap() as i32,
                    properties: if e["properties"].as_bool().unwrap() {
                        vec![("test".into(), "value".into())]
                    } else {
                        vec![]
                    },
                })
                .collect::<Vec<_>>();
            if entries.is_empty() {
                continue;
            }
            let mut scan = Scan::new(
                clock.speed,
                clock.event_time,
                clock.previous_event_time,
                original_duration,
            )
            .unwrap();
            while let Some(event) = scan.next(
                cursor,
                &entries,
                clock.event_time,
                c["callback"].as_bool().unwrap(),
            ) {
                dispatched.push(serde_json::json!({"binding":i,"event":event,"phase":clock.phase}));
                // A callback observes its own entry; cursor movement is deferred
                // until return. Persist the suspended scan at every callback.
                assert_eq!(*cursor as usize, event);
                scan = serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
            }
        }
    }
    if has_frames {
        clock.finish();
    }
    serde_json::json!({"clock":clock,"cursors":cursors,"events":dispatched})
}
fn initial(c: &Value) -> (Clock, Vec<Option<i32>>) {
    (
        serde_json::from_value(c["clock"].clone()).unwrap(),
        c["streams"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["cursor"].as_i64().map(|v| v as i32))
            .collect(),
    )
}
fn production_step(
    c: &Value,
    clock: &mut Clock,
    cursors: &mut [Option<i32>],
    elapsed: i32,
) -> Value {
    let frames = c["frames"].as_u64().unwrap() as u32;
    let original_duration = c["original_duration"].as_f64().unwrap() as f32;
    let motions = c["streams"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, s)| Motion {
            bone: i.to_string(),
            frames,
            duration_ms: original_duration,
            translation: None,
            rotation: None,
            properties: vec![],
            event_lists: if s.is_null() {
                vec![]
            } else {
                vec![
                    s["events"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|e| TimedProperty {
                            time_ms: e["time"].as_i64().unwrap() as i32,
                            bone: i.to_string(),
                            pairs: if e["properties"].as_bool().unwrap() {
                                vec![("test".into(), "value".into())]
                            } else {
                                vec![]
                            },
                        })
                        .collect(),
                ]
            },
        })
        .collect::<Vec<_>>();
    let clip = Animation {
        name: "synthetic".into(),
        skill: "synthetic".into(),
        duration_ms: original_duration,
        distance: 0.,
        moving: false,
        translation_count: 0,
        rotation_count: 0,
        frames,
        motions,
        events: vec![],
    };
    let inputs = clip
        .motions
        .iter()
        .enumerate()
        .map(|(i, m)| MotionInput {
            bone_id: i as u32,
            frames,
            events: m.event_lists.iter().map(Vec::len).sum(),
        })
        .collect::<Vec<_>>();
    let slots = (0..inputs.len()).map(Some).collect::<Vec<_>>();
    let mut layer = Layer::new(
        clip.name.clone(),
        original_duration,
        &inputs,
        &slots,
        Start {
            priority: c["priority"].as_i64().unwrap() as i32,
            speed: 1.,
            weight: 1.,
            now: clock.current,
        },
    )
    .unwrap();
    layer.clock = clock.clone();
    for (b, c) in layer.bindings.iter_mut().zip(cursors.iter()) {
        b.event_cursor = *c;
    }
    let mut bones = c["bone_priorities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| BoneInput {
            local: LocalBone {
                translation: [0.; 3],
                rotation: [0., 0., 0., 1.],
            },
            priority: p.as_i64().unwrap() as i32,
            blend_marker: 0,
        })
        .collect::<Vec<_>>();
    let samples = Samples {
        translations: vec![],
        rotations: vec![],
    };
    let mut update = Update::begin(
        &mut layer,
        clock.current.wrapping_add(elapsed),
        c["update"].as_bool().unwrap(),
        c["callback"].as_bool().unwrap(),
    )
    .unwrap();
    let mut events = vec![];
    while let Some(event) = update
        .next(&mut layer, &clip, &samples, &mut bones)
        .unwrap()
    {
        assert_eq!(
            layer.bindings[event.binding].event_cursor,
            Some(event.event as i32)
        );
        let serialized = serde_json::to_vec(&(&layer, &update, &bones)).unwrap();
        (layer, update, bones) = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(
            update
                .next(&mut layer, &clip, &samples, &mut bones)
                .unwrap(),
            Some(event.clone())
        );
        events.push(
            serde_json::json!({"binding":event.binding,"event":event.event,"phase":event.phase}),
        );
        update.acknowledge().unwrap();
        (layer, update, bones) =
            serde_json::from_slice(&serde_json::to_vec(&(&layer, &update, &bones)).unwrap())
                .unwrap();
    }
    assert!(update.complete());
    *clock = layer.clock;
    for (c, b) in cursors.iter_mut().zip(layer.bindings) {
        *c = b.event_cursor;
    }
    serde_json::json!({"clock":clock,"cursors":cursors,"events":events})
}
fn compare(a: &Value, b: &Value, context: &str) {
    // Value's decimal representation can differ after an f32 round trip. Float
    // fields compare their exact f32 bits, including signed zero.
    if let (Some(a), Some(b)) = (a.as_object(), b.as_object()) {
        for (k, v) in b {
            compare(&a[k], v, &format!("{context}.{k}"));
        }
    } else if let (Some(a), Some(b)) = (a.as_array(), b.as_array()) {
        assert_eq!(a.len(), b.len(), "{context}");
        for (i, (a, b)) in a.iter().zip(b).enumerate() {
            compare(a, b, &format!("{context}[{i}]"));
        }
    } else if a.is_f64() || b.is_f64() {
        assert_eq!(
            (a.as_f64().unwrap() as f32).to_bits(),
            (b.as_f64().unwrap() as f32).to_bits(),
            "{context}"
        );
    } else {
        assert_eq!(a, b, "{context}");
    }
}
#[test]
fn original_event_scans_match_across_clock_modes_priorities_and_callback_gates() {
    let report = report();
    assert_eq!(report["cases"].as_array().unwrap().len(), 1400);
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let (mut clock, mut cursors) = initial(c);
        let actual = step(
            c,
            &mut clock,
            &mut cursors,
            c["elapsed"].as_i64().unwrap() as i32,
        );
        compare(&actual, &row["original"], &format!("case {i}: {c}"));
        let (mut clock, mut cursors) = initial(c);
        let actual = production_step(
            c,
            &mut clock,
            &mut cursors,
            c["elapsed"].as_i64().unwrap() as i32,
        );
        compare(
            &actual,
            &row["original"],
            &format!("production case {i}: {c}"),
        );
    }
}
#[test]
fn native_event_sequences_match_with_save_restore_inside_callbacks_and_between_ticks() {
    let report = report();
    for (i, sequence) in report["sequences"].as_array().unwrap().iter().enumerate() {
        let c = &sequence["initial"];
        let (mut clock, mut cursors) = initial(c);
        let (mut production_clock, mut production_cursors) = initial(c);
        for (j, s) in sequence["steps"].as_array().unwrap().iter().enumerate() {
            let actual = step(
                c,
                &mut clock,
                &mut cursors,
                s["elapsed"].as_i64().unwrap() as i32,
            );
            compare(&actual, &s["original"], &format!("sequence {i} step {j}"));
            let actual = production_step(
                c,
                &mut production_clock,
                &mut production_cursors,
                s["elapsed"].as_i64().unwrap() as i32,
            );
            compare(
                &actual,
                &s["original"],
                &format!("production sequence {i} step {j}"),
            );
            (clock, cursors) =
                serde_json::from_slice(&serde_json::to_vec(&(clock, cursors)).unwrap()).unwrap();
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_event_insertion_replaces_same_timestamp_and_preserves_property_lookup_order() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-animation-event-lists.json")).unwrap();
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let actual = if c["kind"] == "times" {
            let input = c["input"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, v)| (v.as_i64().unwrap() as i32, i));
            serde_json::to_value(
                events::ordered(input)
                    .into_iter()
                    .map(|(time, source)| serde_json::json!({"time":time,"source":source}))
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        } else {
            let input: Vec<(String, String)> = serde_json::from_value(c["input"].clone()).unwrap();
            serde_json::to_value(events::properties(&input)).unwrap()
        };
        assert_eq!(actual, row["original"], "case {i}");
    }
}
