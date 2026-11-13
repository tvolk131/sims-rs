use serde_json::{Value, json};
use sims_sim::pose::animator::{Animator, LayerId, Settings};
#[path = "native_layer_ownership.rs"]
mod ownership;

fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).unwrap()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_skin_destruction_releases_explicit_and_fading_layers_in_order() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-skin-retirement.json")).unwrap();
    let (rig, program) = ownership::resources();
    for row in report["cases"].as_array().unwrap() {
        let settings: Settings = decode(&row["initial"]["settings"]);
        let mut animator = Animator::new(1500, 9, 0, true, 17);
        for step in row["steps"].as_array().unwrap() {
            let a = &step["action"];
            let now = a["now"].as_i64().unwrap() as i32;
            match a["op"].as_str().unwrap() {
                "start" => {
                    animator
                        .start(
                            decode(&a["slot"]),
                            &rig,
                            Some(&program.animations["synthetic"].clip),
                            decode(&a["reverse"]),
                            now,
                            settings,
                        )
                        .unwrap();
                }
                "clear" => animator.clear(decode(&a["slot"]), now, settings).unwrap(),
                other => panic!("unexpected setup {other}"),
            }
        }
        if animator.destruction_layers().iter().any(Option::is_some) {
            assert!(animator.release_model_layers().is_err());
        }
        for step in row["detach"].as_array().unwrap() {
            let id: LayerId = decode(&step["id"]);
            animator.detach_for_destruction(id).unwrap();
            let mut expected = step["original"].clone();
            expected.as_object_mut().unwrap().remove("released");
            expected.as_object_mut().unwrap().remove("bone_priority");
            ownership::compare(&json!(animator), &expected, "explicit detachment");
            animator = serde_json::from_slice(&serde_json::to_vec(&animator).unwrap()).unwrap();
            assert!(animator.detach_for_destruction(id).is_err());
        }
        let released = animator.release_model_layers().unwrap();
        assert_eq!(json!(released), row["original"]["released"]);
        let mut expected = row["original"].clone();
        expected.as_object_mut().unwrap().remove("released");
        expected.as_object_mut().unwrap().remove("bone_priority");
        ownership::compare(&json!(animator), &expected, "model destruction");
        assert!(animator.layers().is_empty());
        assert_eq!(row["references"], 0);
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 48);
}
