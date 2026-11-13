use serde_json::Value;
use sims_formats::animation::{Motion, Samples};
use sims_sim::pose::{
    LocalBone,
    sampling::{self, BoneInput, Layer, Selection},
};

fn compare(report: Value) {
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let original = &row["original"];
        let selection = Selection::new(
            original["phase"].as_f64().unwrap() as f32,
            c["frames"].as_u64().unwrap() as u32,
            c["mode"].as_i64().unwrap() as i32,
            c["elapsed"].as_i64().unwrap() as f32,
            original["duration"].as_f64().unwrap() as f32,
            c["skip"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let samples = Samples {
            translations: serde_json::from_value(c["samples"]["translations"].clone()).unwrap(),
            rotations: serde_json::from_value(c["samples"]["rotations"].clone()).unwrap(),
        };
        let layer = Layer {
            weight: original["weight"].as_f64().unwrap() as f32,
            priority: c["priority"].as_i64().unwrap() as i32,
            translation_policy: c["translation_policy"].as_i64().unwrap() as i32,
            rotation_policy: c["rotation_policy"].as_i64().unwrap() as i32,
        };
        for (i, b) in c["bones"].as_array().unwrap().iter().enumerate() {
            let input = BoneInput {
                local: LocalBone {
                    translation: serde_json::from_value(b["translation"].clone()).unwrap(),
                    rotation: serde_json::from_value(b["rotation"].clone()).unwrap(),
                },
                priority: b["priority"].as_i64().unwrap() as i32,
                blend_marker: b["blend_marker"].as_u64().unwrap() as u32,
            };
            let m = &c["motions"][i];
            let motion = Motion {
                bone: String::new(),
                frames: m.get("frames").unwrap_or(&c["frames"]).as_u64().unwrap() as u32,
                duration_ms: c["duration"].as_f64().unwrap() as f32,
                translation: serde_json::from_value(m["translation"].clone()).unwrap(),
                rotation: serde_json::from_value(m["rotation"].clone()).unwrap(),
                properties: vec![],
                event_lists: vec![],
            };
            let got = if c["update"].as_bool() == Some(false) {
                sampling::BoneOutput {
                    local: input.local,
                    translation_changed: false,
                    rotation_changed: false,
                }
            } else {
                sampling::motion(input, layer, selection, &motion, &samples).unwrap()
            };
            let expected = &original["bones"][i];
            let t: [f32; 3] = serde_json::from_value(expected["translation"].clone()).unwrap();
            let q: [f32; 4] = serde_json::from_value(expected["rotation"].clone()).unwrap();
            let message = || {
                format!(
                    "case {case}, bone {i}, label {}, selection {selection:?}, input {input:?}, layer {layer:?}, got {got:?}, expected {expected}",
                    c["label"]
                )
            };
            for (a, b) in got.local.translation.into_iter().zip(t) {
                assert!((a - b).abs() < 2e-6, "{}", message());
            }
            for (a, b) in got.local.rotation.into_iter().zip(q) {
                assert!((a - b).abs() < 1e-6, "{}", message());
            }
            assert_eq!(
                u64::from(got.translation_changed) + u64::from(got.rotation_changed),
                expected["updates"].as_u64().unwrap(),
                "{}",
                message()
            );
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_full_layer_pose_sampling_matches() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-pose-samples.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 1600);
    compare(report);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_empty_motion_records_preserve_channels() {
    compare(
        serde_json::from_str(include_str!("fixtures/native-empty-motion-samples.json")).unwrap(),
    );
}

#[test]
#[ignore = "private original pet animations and native oracle output"]
fn original_pet_empty_motion_samples_match() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-pet-pose-samples.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 16);
    compare(report);
}
#[test]
#[ignore = "requires private original decoded assets and native oracle output"]
fn original_resident_animation_samples_match() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-pose-samples.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 176);
    compare(report);
}
