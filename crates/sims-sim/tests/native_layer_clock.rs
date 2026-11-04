use serde_json::Value;
use sims_sim::pose::clock::Clock;

#[cfg(feature = "private-fixtures")]
#[test]
fn retained_goth_clocks_use_actual_process_control_and_roundtrip_exactly() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/retained-goth-layer-clock-003f.json")).unwrap();
    assert_eq!(report["fp_control"], 0x003f);
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 6);
    for row in rows {
        let mut clock: Clock = serde_json::from_value(row["initial"].clone()).unwrap();
        let original: Clock = serde_json::from_value(row["original"].clone()).unwrap();
        clock
            .advance_with_control(
                row["now"].as_i64().unwrap() as i32,
                row["original_duration"].as_f64().unwrap() as f32,
                true,
                0x003f,
            )
            .unwrap();
        clock.finish();
        compare(&clock, &original, "actual retained003f clock");
        let bytes = serde_json::to_vec(&clock).unwrap();
        let restored: Clock = serde_json::from_slice(&bytes).unwrap();
        compare(&restored,&clock,"retained clock roundtrip");
    }
}
fn compare(a: &Clock, b: &Clock, context: &str) {
    assert_eq!(
        [
            a.current,
            a.previous,
            a.event_time,
            a.previous_event_time,
            a.mode,
            a.fade_start,
            a.fade_duration
        ],
        [
            b.current,
            b.previous,
            b.event_time,
            b.previous_event_time,
            b.mode,
            b.fade_start,
            b.fade_duration
        ],
        "{context}"
    );
    assert_eq!([a.ended, a.fading], [b.ended, b.fading], "{context}");
    assert_eq!(
        [
            a.duration,
            a.speed,
            a.phase,
            a.weight,
            a.fade_from,
            a.fade_to
        ]
        .map(f32::to_bits),
        [
            b.duration,
            b.speed,
            b.phase,
            b.weight,
            b.fade_from,
            b.fade_to
        ]
        .map(f32::to_bits),
        "{context}: {a:?} != {b:?}"
    );
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_loop_fade_clock_and_event_time_match() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-layer-clock.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 1200);
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut clock: Clock = serde_json::from_value(c["clock"].clone()).unwrap();
        let original: Clock = serde_json::from_value(row["original"].clone()).unwrap();
        let has_frames = c["has_frames"].as_bool().unwrap();
        clock
            .advance(
                clock
                    .current
                    .wrapping_add(c["elapsed"].as_i64().unwrap() as i32),
                c["original_duration"].as_f64().unwrap() as f32,
                has_frames,
            )
            .unwrap();
        if has_frames {
            clock.finish();
        }
        compare(&clock, &original, &format!("case {i}: {c}"));
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_layer_clock_sequences_match_across_save_restore_and_signed_wrap() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-layer-clock.json")).unwrap();
    assert_eq!(report["sequences"].as_array().unwrap().len(), 8);
    for (i, row) in report["sequences"].as_array().unwrap().iter().enumerate() {
        let mut clock: Clock = serde_json::from_value(row["initial"].clone()).unwrap();
        for (j, step) in row["steps"].as_array().unwrap().iter().enumerate() {
            clock
                .advance(
                    clock
                        .current
                        .wrapping_add(step["elapsed"].as_i64().unwrap() as i32),
                    row["original_duration"].as_f64().unwrap() as f32,
                    true,
                )
                .unwrap();
            clock.finish();
            let original: Clock = serde_json::from_value(step["original"].clone()).unwrap();
            compare(&clock, &original, &format!("sequence {i} step {j}"));
            clock = serde_json::from_slice(&serde_json::to_vec(&clock).unwrap()).unwrap();
        }
    }
}
