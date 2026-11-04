#[path = "support/movement.rs"]
mod support;

#[cfg(feature = "private-fixtures")]
#[test]
fn original_movement_base_handoff_keeps_layers_cache_and_settling_order() {
    let report = serde_json::from_str(include_str!("fixtures/native-movement-base.json")).unwrap();
    let (cases, services, _) = support::run(report);
    assert_eq!(cases, 480);
    assert!(services >= 200);
}
