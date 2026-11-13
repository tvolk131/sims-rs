use serde_json::{Value, json};
use sims_sim::pose::model::{Bound, Bounds, Matrix};

pub fn matrix(a: Matrix, b: Matrix, tolerance: f32) {
    assert_eq!(a.kind, b.kind);
    for (a, b) in a.values.into_iter().zip(b.values) {
        assert!((a - b).abs() <= tolerance, "matrix component {a}/{b}");
    }
}

fn bounds(value: Bounds) -> Value {
    json!(
        value
            .minimum
            .into_iter()
            .chain(value.maximum)
            .collect::<Vec<_>>()
    )
}
pub fn compare(bound: &Bound, expected: &Value, tolerance: f32) {
    let model = &bound.model;
    let original: sims_sim::pose::model::Model =
        serde_json::from_value(expected["model"].clone()).unwrap();
    matrix(model.placement, original.placement, tolerance);
    matrix(model.scale, original.scale, tolerance);
    matrix(model.bounds_transform, original.bounds_transform, tolerance);
    assert_eq!(model.bounds_flags, original.bounds_flags);
    matrix(
        bound.matrix,
        serde_json::from_value(expected["skeleton_matrix"].clone()).unwrap(),
        tolerance,
    );
    assert_eq!(bounds(bound.local_bounds), expected["bounds"]);
    assert_eq!(bounds(bound.render_bounds), expected["model_bounds"]);
    let skeleton = &bound.skeleton;
    let animator = json!(skeleton.skin.animator);
    for (key, offset) in [("generation", "0xb4"), ("pose_generation", "0xa4")] {
        assert_eq!(animator[key], expected["skin"][offset]);
    }
    assert_eq!(
        animator["last_clock"].as_i64().unwrap() as u32 as u64,
        expected["skin"]["0xb8"].as_u64().unwrap()
    );
    assert_eq!(animator["dirty"], json!(expected["skin"]["0x94"] != 0));
    assert_eq!(
        json!(skeleton.hierarchy.updates),
        expected["propagated"]["updates"]
    );
    assert_eq!(
        json!(skeleton.hierarchy.evaluated_generation),
        expected["skin"]["0xa0"]
    );
    if skeleton.hierarchy.evaluated_generation != u32::MAX {
        assert_eq!(
            json!(skeleton.hierarchy.absolute),
            expected["propagated"]["absolute"]
        );
    }
    assert_eq!(
        json!(skeleton.geometry.dirty),
        json!(expected["skin"]["0x98"] != 0)
    );
    for (value, offset) in [
        (skeleton.geometry.generation, "0xac"),
        (skeleton.geometry.evaluated_generation, "0xa8"),
        (skeleton.geometry.bounds_generation, "0xb0"),
    ] {
        assert_eq!(json!(value), expected["skin"][offset]);
    }
    for (i, bone) in skeleton.skin.bones.iter().enumerate() {
        assert_eq!(
            json!(
                bone.local
                    .translation
                    .into_iter()
                    .chain(bone.local.rotation)
                    .collect::<Vec<_>>()
            ),
            expected["bones"][i]["local"]
        );
        assert_eq!(json!(skeleton.wiggle[i]), expected["bones"][i]["wiggle"]);
        assert_eq!(
            json!(skeleton.geometry.bone_generations[i]),
            expected["bones"][i]["geometry_stamp"]
        );
        assert_eq!(
            json!(bound.bone_bounds_generations[i]),
            expected["bone_geometry"][i]["stamp"]
        );
        assert_eq!(
            bounds(bound.bone_bounds[i]),
            expected["bone_geometry"][i]["bounds"]
        );
    }
}
