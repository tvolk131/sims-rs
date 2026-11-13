use serde_json::{Value, json};
use sims_formats::{
    animation::{Animation, Motion, Samples},
    avatar::{Bone, Skeleton},
};
use sims_sim::pose::{
    LocalBone,
    animator::{Animator, Settings, Slot},
    sampling::BoneInput,
};
use sims_vm::{Program, program::AnimationAsset};

pub(crate) fn compare(a: &Value, b: &Value, path: &str) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                compare(&a[k], v, &format!("{path}.{k}"));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &format!("{path}[{i}]"));
            }
        }
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => assert_eq!(
            (a.as_f64().unwrap() as f32).to_bits(),
            (b.as_f64().unwrap() as f32).to_bits(),
            "{path}"
        ),
        _ => assert_eq!(a, b, "{path}"),
    }
}
fn slot(v: &Value) -> Slot {
    serde_json::from_value(v.clone()).unwrap()
}
fn int(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn flag(v: &Value) -> bool {
    v.as_bool().unwrap()
}
pub(crate) fn resources() -> (Skeleton, Program) {
    let skeleton = Skeleton {
        name: "synthetic".into(),
        bones: vec![Bone {
            name: "bone".into(),
            parent: None,
            translation: [0.; 3],
            rotation: [0., 0., 0., 1.],
            properties: vec![],
            flags: [0; 3],
            wiggle: Default::default(),
        }],
    };
    let clip = Animation {
        name: "synthetic".into(),
        skill: "synthetic".into(),
        duration_ms: 1000.,
        distance: 0.,
        moving: false,
        translation_count: 0,
        rotation_count: 0,
        frames: 30,
        events: vec![],
        motions: vec![Motion {
            bone: "bone".into(),
            frames: 30,
            duration_ms: 1000.,
            translation: None,
            rotation: None,
            properties: vec![],
            event_lists: vec![],
        }],
    };
    let mut program = Program::default();
    program.animations.insert(
        clip.name.clone(),
        AnimationAsset {
            source: "synthetic".into(),
            content_hash: "synthetic".into(),
            samples_source: "synthetic".into(),
            samples_hash: "synthetic".into(),
            clip,
            samples: Samples {
                translations: vec![],
                rotations: vec![],
            },
        },
    );
    (skeleton, program)
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_ownership_fades_retirement_and_update_gates_survive_sequence_restores() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-layer-ownership.json")).unwrap();
    let (rig, program) = resources();
    let mut operations = 0;
    for (seq, row) in report["sequences"].as_array().unwrap().iter().enumerate() {
        let initial = &row["initial"];
        let settings: Settings = serde_json::from_value(initial["settings"].clone()).unwrap();
        let mut animator = Animator::new(
            int(&initial["last_clock"]),
            initial["generation"].as_u64().unwrap() as u32,
            0,
            flag(&initial["dirty"]),
            int(&initial["selection_cache"]),
        );
        let mut bones = vec![BoneInput {
            local: LocalBone {
                translation: [0.; 3],
                rotation: [0., 0., 0., 1.],
            },
            priority: int(&initial["bone_priority"]),
            blend_marker: 0,
        }];
        for (step, row) in row["steps"].as_array().unwrap().iter().enumerate() {
            let a = &row["action"];
            let now = int(&a["now"]);
            let mut result = Value::Null;
            let before = animator.layers().iter().map(|e| e.id.0).collect::<Vec<_>>();
            match a["op"].as_str().unwrap() {
                "start" => {
                    let clip = (flag(&a["present"]) && flag(&a["ready"]))
                        .then_some(&program.animations["synthetic"].clip);
                    result = json!(
                        animator
                            .start(
                                slot(&a["slot"]),
                                &rig,
                                clip,
                                flag(&a["reverse"]),
                                now,
                                settings
                            )
                            .unwrap()
                    );
                }
                "clear" => animator.clear(slot(&a["slot"]), now, settings).unwrap(),
                "clear_foreground" => animator.clear_foreground(now, settings).unwrap(),
                "promote" => animator
                    .set_promote_next_foreground(flag(&a["value"]))
                    .unwrap(),
                "priority" => {
                    if let Some(id) = animator.slot(slot(&a["slot"])) {
                        animator.set_priority(id, int(&a["value"])).unwrap();
                    }
                }
                "finished" => result = json!(animator.finished(slot(&a["slot"])).unwrap()),
                "update" => {
                    animator
                        .begin(
                            now,
                            flag(&a["sample_pose"]),
                            false,
                            flag(&a["force"]),
                            &mut bones,
                        )
                        .unwrap();
                    // Snapshot at the enclosing pass boundary as well as between operations.
                    (animator, bones) =
                        serde_json::from_slice(&serde_json::to_vec(&(&animator, &bones)).unwrap())
                            .unwrap();
                    assert!(animator.next(&program, &mut bones).unwrap().is_none());
                    assert!(!animator.busy());
                }
                other => panic!("unknown op {other}"),
            }
            let mut actual = serde_json::to_value(&animator).unwrap();
            actual["result"] = result;
            actual["bone_priority"] = json!(bones[0].priority);
            let mut expected = row["original"].clone();
            let mut removed = expected
                .as_object_mut()
                .unwrap()
                .remove("released")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect::<Vec<_>>();
            removed.sort_unstable();
            let mut missing = before
                .into_iter()
                .filter(|id| animator.layers().iter().all(|e| e.id.0 != *id))
                .collect::<Vec<_>>();
            missing.sort_unstable();
            assert_eq!(missing, removed, "released {seq}/{step}");
            compare(
                &actual,
                &expected,
                &format!("sequence {seq} step {step} {a}"),
            );
            animator = serde_json::from_slice(&serde_json::to_vec(&animator).unwrap()).unwrap();
            operations += 1;
        }
    }
    assert_eq!(operations, 6480);
}
