//! Synthetic native person-constructor vectors; no original resource bytes.
use serde_json::{Value, json};
use sims_vm::*;
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_constructor_presentation_byte_is_owned_and_absence_is_preserved() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/indicator-constructor-fields.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 8);
    for row in report["cases"].as_array().unwrap() {
        let mut words = vec![0; 108];
        words[0x74 / 2] = row["attribute_count"].as_i64().unwrap() as i16;
        let p = Program {
            modules: [(
                "test".into(),
                Module {
                    object_definition_words: [(1, words)].into(),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        let object = Object::construct(
            Definition {
                source: "test".into(),
                object: 1,
                guid: 0,
                slots: 0,
            },
            &p,
            0,
        )
        .unwrap();
        assert_eq!(
            json!(object.native_keep_dynamic),
            row["original"]["keep_dynamic"]
        );
        let restored: Object =
            serde_json::from_slice(&serde_json::to_vec(&object).unwrap()).unwrap();
        assert_eq!(restored, object);
    }
    let old = serde_json::to_value(Object::default()).unwrap();
    assert!(old.get("native_keep_dynamic").is_none());
    let restored: Object = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(restored.native_keep_dynamic, None);
    assert_eq!(serde_json::to_value(restored).unwrap(), old);
    for value in [1, 128, 255] {
        let object = Object {
            native_keep_dynamic: Some(value),
            ..Default::default()
        };
        let restored: Object =
            serde_json::from_slice(&serde_json::to_vec(&object).unwrap()).unwrap();
        assert_eq!(restored.native_keep_dynamic, Some(value));
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_base_constructor_retains_its_own_flag_word() {
    let report: Value = serde_json::from_str(include_str!("fixtures/object-flags.json")).unwrap();
    for row in report["construction"].as_array().unwrap() {
        let mut words = vec![0; 108];
        words[9] = 4;
        words[0x74 / 2] = row["count"].as_i64().unwrap() as i16;
        let p = Program {
            modules: [(
                "test".into(),
                Module {
                    source: "test".into(),
                    object_definition_words: [(1, words)].into(),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        let object = Object::construct(
            Definition {
                source: "test".into(),
                object: 1,
                guid: 0,
                slots: 0,
            },
            &p,
            0,
        )
        .unwrap();
        assert_eq!(json!(object.native_flags.unwrap()), row["flags"]);
        assert_eq!(object.native_tree_error, Some(0));
        assert_eq!(object.data[&8], 0, "script flags are separate storage");
    }
}

#[test]
fn missing_tree_error_does_not_become_native_success_on_restore() {
    let object = Object::default();
    let encoded = serde_json::to_value(&object).unwrap();
    assert!(encoded.get("native_tree_error").is_none());
    let restored: Object = serde_json::from_value(encoded).unwrap();
    assert_eq!(restored.native_tree_error, None);
    for error in [0, 8, 0x44f, -1, i16::MIN] {
        let object = Object {
            native_tree_error: Some(error),
            ..Default::default()
        };
        let restored: Object =
            serde_json::from_slice(&serde_json::to_vec(&object).unwrap()).unwrap();
        assert_eq!(restored.native_tree_error, Some(error));
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_person_constructor_storage_and_previous_motives() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/person-construction.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let mut words = vec![0; 108];
        words[9] = 2;
        words[0x74 / 2] = row["count"].as_i64().unwrap() as i16;
        let p = Program {
            globals: "test".into(),
            modules: BTreeMap::from([(
                "test".into(),
                Module {
                    source: "test".into(),
                    object_definition_words: BTreeMap::from([(1, words)]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let object = Object::construct_person(
            Definition {
                source: "test".into(),
                object: 1,
                guid: 0,
                slots: 0,
            },
            &p,
            0,
        )
        .unwrap();
        let n = &row["original"];
        assert_eq!(json!(object.native_flags.unwrap()), n["native_flags"]);
        assert_eq!(
            json!((0..72).map(|i| object.data[&i]).collect::<Vec<_>>()),
            n["storage"]["data"]
        );
        assert_eq!(json!(object.temps), n["storage"]["temps"]);
        assert_eq!(json!(object.attributes), n["storage"]["attributes"]);
        assert_eq!(
            json!(
                (0..256)
                    .map(|i| object.person.as_ref().unwrap()[&i])
                    .collect::<Vec<_>>()
            ),
            n["person"]
        );
        assert_eq!(json!(object.motives.unwrap()), n["motives"]);
        let u = object.motive_update.as_ref().unwrap();
        assert_eq!(json!(u.previous), n["previous"]);
        assert_eq!(json!(u.last_decay), n["last_decay"]);
        assert!(object.motive_changes.is_empty());
    }
}
