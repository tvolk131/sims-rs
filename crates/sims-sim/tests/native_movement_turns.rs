#[path = "support/movement.rs"]
mod support;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_turns_adjustment_steps_and_start_stop_transitions_restore_at_services() {
    let report = serde_json::from_str(include_str!("fixtures/native-movement-turns.json")).unwrap();
    let (cases, services, paths) = support::run(report);
    assert!(cases >= 889 && services >= 681 && paths >= 26);
}
