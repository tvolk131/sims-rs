use serde_json::{Value, json};
use sims_formats::{
    animation::{Animation, Motion, Samples, TimedProperty},
    avatar::{Bone, Skeleton},
};
use sims_sim::pose::{
    LocalBone,
    animator::{Animator, Settings, Slot},
    properties::{Context, Handler},
    sampling::BoneInput,
};
use sims_vm::{Program, program::AnimationAsset};
use std::collections::BTreeMap;

fn resources() -> (Skeleton, Program) {
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
    let mut program = Program::default();
    for (name, properties) in [
        ("passive", vec![("xevt", "10")]),
        (
            "discontinuity",
            vec![("discontinuity", "1"), ("xevt", "21")],
        ),
    ] {
        let event = TimedProperty {
            time_ms: 100,
            bone: "bone".into(),
            pairs: properties
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        };
        let clip = Animation {
            name: name.into(),
            skill: name.into(),
            duration_ms: 1000.,
            distance: 0.,
            moving: false,
            translation_count: 0,
            rotation_count: 0,
            frames: 30,
            events: vec![event.clone()],
            motions: vec![Motion {
                bone: "bone".into(),
                frames: 30,
                duration_ms: 1000.,
                translation: None,
                rotation: None,
                properties: vec![],
                event_lists: vec![vec![event]],
            }],
        };
        program.animations.insert(
            name.into(),
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
    }
    (skeleton, program)
}

fn compare(actual: &Value, native: &Value, path: &str) {
    match (actual, native) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, expected) in b {
                compare(&a[key], expected, &format!("{path}.{key}"));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}: {a:?} versus {b:?}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &format!("{path}[{i}]"));
            }
        }
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => assert_eq!(
            (a.as_f64().unwrap() as f32).to_bits(),
            (b.as_f64().unwrap() as f32).to_bits(),
            "{path}"
        ),
        _ => assert_eq!(actual, native, "{path}"),
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_callback_removal_preserves_defined_iteration_and_rejects_released_reads() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-callback-removal.json")).unwrap();
    let (rig, program) = resources();
    let settings = Settings {
        fade_enabled: true,
        fade_duration: 200,
        sampling_config: 1,
    };
    let context = Context {
        actor: 137,
        sound_visible: false,
        selected: false,
        debug_sound: false,
        position_x: 32.,
        appearance_mode: 0,
        snow: false,
        lot_kind: 1,
    };
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 16);
    let mut completed = 0;
    for (case, row) in cases.iter().enumerate() {
        let mut animator = Animator::new(0, 0, 0, true, -1);
        let trigger = row["input"]["trigger"].as_str().unwrap();
        let blend_priority = row["input"]["blend_priority"].as_i64().unwrap() as i32;
        for (slot, name) in [
            (Slot::Foreground, "Blend"),
            (Slot::Carry, "Carry"),
            (Slot::Base, "Base"),
        ] {
            let clip = &program.animations[if name == trigger {
                "discontinuity"
            } else {
                "passive"
            }]
            .clip;
            animator
                .start(slot, &rig, Some(clip), false, 0, settings)
                .unwrap();
        }
        animator
            .set_priority(animator.slot(Slot::Foreground).unwrap(), blend_priority - 1)
            .unwrap();
        animator
            .start(
                Slot::Foreground,
                &rig,
                Some(
                    &program.animations[if trigger == "Foreground" {
                        "discontinuity"
                    } else {
                        "passive"
                    }]
                    .clip,
                ),
                false,
                0,
                settings,
            )
            .unwrap();
        for (slot, priority) in [(Slot::Carry, -2), (Slot::Base, 3)] {
            animator
                .set_priority(animator.slot(slot).unwrap(), priority)
                .unwrap();
        }
        // The native probe supplies the same no-priority-override fixture input.
        let mut state = serde_json::to_value(&animator).unwrap();
        for owned in state["layers"].as_array_mut().unwrap() {
            owned["layer"]["priority_override"] = json!(false);
        }
        animator = serde_json::from_value(state).unwrap();
        let mut bones = vec![BoneInput {
            local: LocalBone {
                translation: [0.; 3],
                rotation: [0., 0., 0., 1.],
            },
            priority: -10000,
            blend_marker: 0,
        }];
        animator.begin(100, false, true, false, &mut bones).unwrap();
        let mut person = BTreeMap::new();
        let mut callbacks = 0;
        let error = loop {
            let event = match animator.next(&program, &mut bones) {
                Ok(Some(event)) => event,
                Ok(None) => break None,
                Err(error) => break Some(error),
            };
            callbacks += 1;
            let mut handler = Handler::new(event);
            (animator, handler) =
                serde_json::from_slice(&serde_json::to_vec(&(&animator, &handler)).unwrap())
                    .unwrap();
            assert!(
                handler
                    .next(&mut animator, &mut person, context)
                    .unwrap()
                    .is_none()
            );
            assert!(handler.complete());
            animator = serde_json::from_slice(&serde_json::to_vec(&animator).unwrap()).unwrap();
            animator.acknowledge().unwrap();
        };
        let expected_dangling = row["dangling"]["layer"].as_u64();
        if let Some(id) = expected_dangling {
            assert_eq!(
                error.as_deref(),
                Some(format!("native skin pass references released layer {id}").as_str()),
                "case {case}"
            );
            animator = serde_json::from_slice(&serde_json::to_vec(&animator).unwrap()).unwrap();
            assert_eq!(
                animator.next(&program, &mut bones).unwrap_err(),
                error.unwrap()
            );
        } else {
            assert!(error.is_none(), "case {case}: {error:?}");
            completed += 1;
        }
        let mut state = serde_json::to_value(&animator).unwrap();
        state["callback_count"] = json!(callbacks);
        state["released"] = json!([1]);
        state["result"] = Value::Null;
        state["bones"] = json!(bones.iter().map(|b| json!({
            "translation":b.local.translation,"rotation":b.local.rotation,"priority":b.priority
        })).collect::<Vec<_>>());
        compare(&state, &row["original"], &format!("case {case}"));
    }
    assert_eq!(completed, 9);
}
