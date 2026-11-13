use sims_vm::{
    interface::{
        speed::{self, Timers},
        state::State,
    },
    scheduler::clock::Timer,
};

#[test]
fn original_speed_setter_preserves_raw_input_and_independent_timer_history() {
    for value in [i32::MIN, -1, 0, 760, 924, 994, 1000, 1015, i32::MAX] {
        let mut state = State::constructed(760);
        state.buy.dirty = 0x12340000;
        let before = Timers {
            first: Timer {
                elapsed: 123,
                interval: 99,
                last: 456,
                limit: 7,
            },
            second: Timer {
                elapsed: 999,
                interval: 88,
                last: 222,
                limit: 3,
            },
        };
        state.speed_timers = Some(before);
        state.set_speed(value).unwrap();
        assert_eq!(state.speed, value);
        assert_eq!(state.buy.dirty, 0x12340040);
        let timers = state.speed_timers.unwrap();
        assert_eq!(
            (timers.first.elapsed, timers.first.last, timers.first.limit),
            (123, 456, 7)
        );
        assert_eq!(
            (
                timers.second.elapsed,
                timers.second.last,
                timers.second.limit
            ),
            (999, 222, 3)
        );
        let restored: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(state, restored);
    }
    assert_eq!(speed::intervals(speed::NORMAL), [1000, 3000]);
    assert_eq!(speed::intervals(speed::FAST), [333, 1000]);
    assert_eq!(speed::intervals(speed::ULTRA), [48, 143]);
    // Rounded decimal0.001 would select87 instead of86 at this boundary.
    assert_eq!(speed::intervals(500), [2048, 6143]);
    assert_ne!(speed::intervals(1015), speed::intervals(1000));
}

#[test]
fn historical_controls_do_not_acquire_unknown_speed_timers() {
    let mut data = serde_json::to_value(State::constructed(760)).unwrap();
    data.as_object_mut().unwrap().remove("speed_timers");
    let mut state: State = serde_json::from_value(data.clone()).unwrap();
    assert!(state.set_speed(924).is_err());
    assert_eq!(serde_json::to_value(state).unwrap(), data);
}

#[test]
#[ignore = "private guarded4cee89 original-instruction corpus"]
fn original_control_speed_setter_matches_original_instructions() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/interface-speed-setter.json");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let mut state = State::constructed(760);
        state.buy.dirty = row["input"]["dirty"].as_u64().unwrap() as u32;
        state.speed_timers = Some(serde_json::from_value(row["input"]["timers"].clone()).unwrap());
        state
            .set_speed(row["input"]["speed"].as_i64().unwrap() as i32)
            .unwrap();
        assert_eq!(
            serde_json::json!({"dirty":state.buy.dirty,"speed":state.speed,"timers":state.speed_timers}),
            row["original"],
            "{}",
            row["input"]
        );
    }
}
