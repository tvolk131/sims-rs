use serde_json::{Value, json};
use sims_formats::avatar::Skeleton;
use sims_sim::pose::model::Bound;

pub fn compare(row: &Value) {
    let source: Skeleton = serde_json::from_value(row["input"]["skeleton"].clone()).unwrap();
    let mut crt = row["input"]["crt_state"].as_u64().unwrap() as u32;
    let bound = Bound::new(&source, &mut crt).unwrap();
    let expected = &row["original"];
    assert_eq!(json!(crt), expected["crt_state"]);
    compare_state(&bound, expected);
}

fn compare_state(bound: &Bound, expected: &Value) {
    assert_eq!(json!(bound.model), expected["model"]);
    assert_eq!(json!(bound.matrix), expected["skeleton_matrix"]);
    let bounds = |b: sims_sim::pose::model::Bounds| {
        b.minimum.into_iter().chain(b.maximum).collect::<Vec<_>>()
    };
    assert_eq!(json!(bounds(bound.local_bounds)), expected["bounds"]);
    assert_eq!(json!(bounds(bound.render_bounds)), expected["model_bounds"]);
    let s = &bound.skeleton;
    assert_eq!(
        json!(s.hierarchy.updates),
        expected["propagated"]["updates"]
    );
    assert_eq!(
        json!(s.hierarchy.evaluated_generation),
        expected["propagated"]["evaluated_generation"]
    );
    if s.hierarchy.evaluated_generation != u32::MAX {
        assert_eq!(
            json!(s.hierarchy.absolute),
            expected["propagated"]["absolute"]
        );
    }
    let state = json!(s.skin.animator);
    assert_eq!(state["generation"], expected["skin"]["0xb4"]);
    assert_eq!(state["pose_generation"], expected["skin"]["0xa4"]);
    assert_eq!(
        json!(s.geometry.dirty),
        json!(expected["skin"]["0x98"] != 0)
    );
    assert_eq!(json!(s.geometry.generation), expected["skin"]["0xac"]);
    assert_eq!(
        json!(s.geometry.evaluated_generation),
        expected["skin"]["0xa8"]
    );
    assert_eq!(
        json!(s.geometry.bounds_generation),
        expected["skin"]["0xb0"]
    );
    for (i, b) in expected["bones"].as_array().unwrap().iter().enumerate() {
        assert_eq!(json!(s.wiggle[i]), b["wiggle"]);
        assert_eq!(json!(s.flags[i]), b["flags"]);
        assert_eq!(json!(s.named_bone_flags[i]), b["named_bone"]);
        assert_eq!(json!(s.geometry.bone_generations[i]), b["geometry_stamp"]);
        assert_eq!(
            json!(bound.bone_bounds_generations[i]),
            expected["bone_geometry"][i]["stamp"]
        );
        assert_eq!(
            json!(bounds(bound.bone_bounds[i])),
            expected["bone_geometry"][i]["bounds"]
        );
    }
    // Native absolute bone memory remains uninitialized here. Compare only its
    // validity stamp; neither zero allocation nor Rust's provisional pose is a
    // published pose. A real animation/hierarchy pass must precede observation.
    let restored: Bound = serde_json::from_slice(&serde_json::to_vec(&bound).unwrap()).unwrap();
    assert_eq!(*bound, restored);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_empty_model_refresh_preserves_distinct_geometry_and_pose_stamps() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-empty-model-refresh.json")).unwrap();
    let mut count = 0;
    for row in report["sequences"].as_array().unwrap() {
        let source = serde_json::from_value(row["input"]["skeleton"].clone()).unwrap();
        let mut crt = row["input"]["crt_state"].as_u64().unwrap() as u32;
        let mut bound = Bound::new(&source, &mut crt).unwrap();
        compare_state(&bound, &row["initial"]);
        for step in row["steps"].as_array().unwrap() {
            let action = &step["action"];
            match action["op"].as_str().unwrap() {
                "accessories" => bound.refresh_empty_accessories(),
                "geometry" => bound
                    .refresh_geometry(action["force"].as_bool().unwrap())
                    .unwrap(),
                "hierarchy" => bound
                    .skeleton
                    .hierarchy
                    .update(&mut bound.skeleton.skin, action["force"].as_bool().unwrap())
                    .unwrap(),
                _ => unreachable!(),
            }
            compare_state(&bound, &step["original"]);
            count += 1;
        }
    }
    assert_eq!(count, 44);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_bare_model_binding_retains_unpublished_pose_and_empty_bounds() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-model-binding.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        compare(row);
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 52);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_builtin_root_has_distinct_constructor_stamps_and_rng() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-root-model.json")).unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        let mut crt = row["input"]["crt_state"].as_u64().unwrap() as u32;
        let bound = Bound::root(&mut crt).unwrap();
        compare_state(&bound, &row["original"]);
        assert_eq!(json!(crt), row["original"]["crt_state"]);
        let source = sims_sim::pose::skeleton::root_source();
        assert_eq!(source.bones.len(), 1);
        let native = &row["original"]["bones"][0];
        assert_eq!(json!(source.bones[0].name), native["name"]);
        let bone = &bound.skeleton.skin.bones[0];
        assert_eq!(
            json!(
                bone.local
                    .translation
                    .into_iter()
                    .chain(bone.local.rotation)
                    .collect::<Vec<_>>()
            ),
            native["local"]
        );
        assert!(bound.skeleton.named_bone_flags[0]);
    }
}

#[test]
#[ignore = "requires private original resident model-binding report"]
fn original_resident_model_bindings_match_native() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-model-binding.json")).unwrap(),
    )
    .unwrap();
    for row in report["cases"].as_array().unwrap() {
        compare(row);
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 7);
}
