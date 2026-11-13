use serde_json::Value;
use sims_sim::{
    head_look::rig::AbsoluteBone,
    pose::{self, LocalBone},
};
#[cfg(feature = "private-fixtures")]
#[test]
fn original_quaternion_blends_match_including_endpoints_and_unchanged_destination() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-pose-math.json")).unwrap();
    assert_eq!(report["blend"].as_array().unwrap().len(), 724);
    for row in report["blend"].as_array().unwrap() {
        let c = &row["input"];
        let got = pose::blend_quaternion(
            c["weight"].as_f64().unwrap() as f32,
            serde_json::from_value(c["a"].clone()).unwrap(),
            serde_json::from_value(c["b"].clone()).unwrap(),
            serde_json::from_value(c["destination"].clone()).unwrap(),
        )
        .unwrap();
        let want: [f32; 4] = serde_json::from_value(row["original"].clone()).unwrap();
        for (a, b) in got.into_iter().zip(want) {
            assert!((a - b).abs() < 1e-6, "{c}: {got:?} != {want:?}");
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_recursive_bone_hierarchy_matches_without_normalizing_source_samples() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-pose-math.json")).unwrap();
    assert_eq!(report["hierarchy"].as_array().unwrap().len(), 292);
    for row in report["hierarchy"].as_array().unwrap() {
        let c = &row["input"];
        let bones = c["bones"].as_array().unwrap();
        let parents = bones
            .iter()
            .map(|b| serde_json::from_value(b["parent"].clone()).unwrap())
            .collect::<Vec<_>>();
        let locals = bones
            .iter()
            .map(|b| LocalBone {
                translation: serde_json::from_value(b["translation"].clone()).unwrap(),
                rotation: serde_json::from_value(b["rotation"].clone()).unwrap(),
            })
            .collect::<Vec<_>>();
        let base = AbsoluteBone {
            translation: serde_json::from_value(c["parent_translation"].clone()).unwrap(),
            rotation: serde_json::from_value(c["parent_rotation"].clone()).unwrap(),
        };
        let got = pose::hierarchy(&parents, &locals, base).unwrap();
        for (got, want) in got.iter().zip(row["original"].as_array().unwrap()) {
            assert_eq!(want["updates"], 1);
            let t: [f32; 3] = serde_json::from_value(want["translation"].clone()).unwrap();
            let q: [f32; 4] = serde_json::from_value(want["rotation"].clone()).unwrap();
            for (a, b) in got.translation.into_iter().zip(t) {
                assert!(
                    (a - b).abs() < 0.00002,
                    "{c}: translation {:?} != {t:?}",
                    got.translation
                );
            }
            for (a, b) in got.rotation.into_iter().zip(q) {
                assert!(
                    (a - b).abs() < 0.000002,
                    "{c}: rotation {:?} != {q:?}",
                    got.rotation
                );
            }
        }
    }
}
