use serde_json::{Value, json};
use sims_sim::pose::construction::State;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_animator_constructors_define_only_initialized_state() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-animator-construction.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let tick = row["input"]["tick"].as_i64().unwrap() as i32;
        let state = State::new(tick);
        let expected = &row["original"];
        assert_eq!(expected["animation_events"], 0);
        assert_eq!(expected["engaged"], 0);
        let movement = sims_sim::pose::movement::Movement::new(state.movement.clone());
        assert_eq!(json!(movement.scalar_7c), expected["scalar_7c"]);
        let path = sims_sim::pose::movement::path::Path::initialized(
            sims_sim::pose::movement::path::Settings::COMPLETE_COLLECTION,
            state.phase_window,
            state.stop_distance,
            state.clip_distances,
        );
        assert_eq!(
            expected["path_state"],
            json!({"index":path.index,"pointer":0,
            "progress":path.progress,"deadline":path.deadline,"stop_mode":path.stop_mode,
            "interrupt":path.interrupt,"guard":path.guard,"delta":path.delta})
        );
        let actual = json!(state);
        for key in [
            "clock",
            "clock_mode",
            "selection",
            "head_clock",
            "head_angles",
            "mode",
            "previous_mode",
            "base_posture",
            "pickup",
            "carry_mode",
            "rate",
            "target_rate",
            "phase_window",
            "stop_distance",
            "clip_distances",
        ] {
            assert_eq!(actual[key], expected[key], "tick {tick}: {key}");
        }
        for key in ["base", "offset", "previous"] {
            assert_eq!(actual["placement"][key], expected[key]);
        }
        assert_eq!(actual["placement"]["dirty"], expected["placement_dirty"]);
        for key in [
            "status",
            "flags",
            "start",
            "end",
            "turn_delta",
            "nominal_turn",
            "reason",
            "apply_rate",
        ] {
            assert_eq!(actual["movement"][key], expected[key]);
        }
        assert_eq!(
            expected["uninitialized"],
            json!([0x1ac, 0x21c, 0x220, 0x224])
        );
        let fill = row["input"]["fill"].as_u64().unwrap() as u8;
        assert_eq!(
            expected["uninitialized_words"],
            json!(([i32::from_le_bytes([fill; 4]); 4]))
        );
        assert!(state.prefix().is_err());
        assert!(state.head().unwrap().target_point.is_none());
        let restored: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(state, restored);
        assert!(restored.prefix().is_err());
        assert!(restored.head().unwrap().target_point.is_none());
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 18);
}
