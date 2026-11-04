//! Original executed activation tails, independent of VM construction/reset.
use serde_json::Value;
use sims_vm::Object;

#[cfg(feature = "private-fixtures")]
#[test]
fn activation_matches_original_writes_without_changing_other_object_state() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/object-activation.json")).unwrap();
    assert_eq!(report["native_execution"], true);
    assert_eq!(report["cases"].as_array().unwrap().len(), 76);
    for case in report["cases"].as_array().unwrap() {
        let mut object = Object {
            native_flags: Some(case["before"].as_u64().unwrap() as u32),
            native_tree_error: Some(-7),
            native_keep_dynamic: Some(3),
            temps: [-7, 8, 9, -10, 11, 12, 13, -14],
            slots: vec![Some(3), None, Some(7)],
            ..Default::default()
        };
        let mut expected = object.clone();
        expected.native_flags = Some(case["after"].as_u64().unwrap() as u32);
        object.activate_native().unwrap();
        assert_eq!(object, expected, "{case}");
        object.activate_native().unwrap();
        assert_eq!(
            object, expected,
            "activation must retain an already active word"
        );
    }
}

#[test]
fn activation_cannot_invent_historical_unknown_flags() {
    let mut object = Object::default();
    let before = object.clone();
    assert!(object.activate_native().is_err());
    assert_eq!(object, before);
}
