use serde_json::{Value, json};
use sims_sim::pose::{
    animator::{Animator, Settings},
    graphic_reset,
    graphic_transform::Placement,
    model::Bound,
};
#[path = "native_layer_ownership.rs"]
mod ownership;
fn decode<T: serde::de::DeserializeOwned>(v: &Value) -> T {
    serde_json::from_value(v.clone()).unwrap()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_base_reset_preserves_person_fields_and_retains_fading_layers() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-graphic-reset.json")).unwrap();
    let (source, program) = ownership::resources();
    for (case, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let settings: Settings = decode(&row["initial"]["settings"]);
        let mut crt = 1;
        let mut bound = Bound::new(&source, &mut crt).unwrap();
        let mut animator = Animator::new(1500, 9, 0, true, 17);
        for step in row["steps"].as_array().unwrap() {
            let a = &step["action"];
            animator
                .start(
                    decode(&a["slot"]),
                    &source,
                    Some(&program.animations["synthetic"].clip),
                    a["reverse"].as_bool().unwrap(),
                    a["now"].as_i64().unwrap() as i32,
                    settings,
                )
                .unwrap();
        }
        let b = &row["before"];
        let mut value = json!(animator);
        value["anchor_valid"] = b["anchor_valid"].clone();
        value["anchor_bone"] = b["anchor_bone"].clone();
        value["anchor_point"] = b["anchor_point"].clone();
        value["numbered_events"] = b["events"].clone();
        value["promote_next_foreground"] = row["promote"].clone();
        bound.skeleton.skin.animator = decode(&value);
        bound.skeleton.geometry.generation = decode(&b["geometry_generation"]);
        let mut placement: Placement = decode(&b["placement"]);
        let mut flags: [u8; 4] = decode(&b["flags"]);
        let mut engaged: i32 = decode(&b["engaged"]);
        let before = bound.clone();
        graphic_reset::apply(
            &mut bound,
            &mut placement,
            &mut flags,
            &mut engaged,
            1600,
            settings,
        )
        .unwrap();
        let actual = json!(bound.skeleton.skin.animator);
        let mut expected = row["original"]["animator"].clone();
        let released: Vec<Value> = expected["released"].as_array().unwrap().clone();
        for key in ["released", "bone_priority"] {
            expected.as_object_mut().unwrap().remove(key);
        }
        ownership::compare(&actual, &expected, &format!("case {case}"));
        let removed = before
            .skeleton
            .skin
            .animator
            .layers()
            .iter()
            .filter(|old| {
                !bound
                    .skeleton
                    .skin
                    .animator
                    .layers()
                    .iter()
                    .any(|new| new.id == old.id)
            })
            .map(|old| json!(old.id))
            .collect::<Vec<_>>();
        assert_eq!(removed.len(), released.len());
        for id in released {
            assert!(removed.contains(&id));
        }
        let s = &row["original"]["scalar"];
        assert_eq!(json!(placement), s["placement"]);
        assert_eq!(json!(flags), s["flags"]);
        assert_eq!(json!(engaged), s["engaged"]);
        assert_eq!(
            json!(bound.skeleton.geometry.generation),
            s["geometry_generation"]
        );
        assert_eq!(actual["anchor_valid"], s["anchor_valid"]);
        assert_eq!(actual["anchor_bone"], s["anchor_bone"]);
        assert_eq!(actual["anchor_point"], s["anchor_point"]);
        assert_eq!(actual["numbered_events"], s["events"]);
        assert_eq!(bound.model, before.model);
        assert_eq!(bound.matrix, before.matrix);
        assert_eq!(bound.skeleton.skin.bones, before.skeleton.skin.bones);
        assert_eq!(bound.skeleton.hierarchy, before.skeleton.hierarchy);
        let restored: Bound = decode(&json!(bound));
        assert_eq!(bound, restored);
    }
}

#[test]
fn base_reset_rejects_a_pending_skin_pass_before_any_writes() {
    let mut crt = 1;
    let mut bound = Bound::root(&mut crt).unwrap();
    bound.skeleton.skin.begin(0, true, true, false).unwrap();
    let mut placement = Placement::constructed();
    let mut flags = [7; 4];
    let mut engaged = 3;
    let before = (bound.clone(), placement.clone(), flags, engaged);
    assert!(
        graphic_reset::apply(
            &mut bound,
            &mut placement,
            &mut flags,
            &mut engaged,
            0,
            Settings {
                fade_enabled: true,
                fade_duration: 200,
                sampling_config: 1
            }
        )
        .is_err()
    );
    assert_eq!((bound, placement, flags, engaged), before);
}
