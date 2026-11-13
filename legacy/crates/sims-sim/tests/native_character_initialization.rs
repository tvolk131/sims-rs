use serde_json::{Value, json};
use sims_formats::{
    animation::{Animation, Motion, Samples, TimedProperty},
    avatar::{Bone, Skeleton},
};
use sims_sim::pose::{
    animator::{Settings, Slot},
    initialize::{Initialization, Request},
    properties, transform,
};
use sims_vm::{Program, program::AnimationAsset};

fn resources(case: &Value) -> (Skeleton, Program) {
    let source = Skeleton {
        name: "catalog-rig".into(),
        bones: ["root", "foot0", "foot1"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| Bone {
                name: name.into(),
                parent: (i != 0).then_some(0),
                translation: [0.; 3],
                rotation: [0., 0., 0., 1.],
                flags: [0; 3],
                properties: vec![],
                wiggle: Default::default(),
            })
            .collect(),
    };
    let motions = (0..2)
        .map(|i| Motion {
            bone: format!("foot{i}"),
            frames: 30,
            duration_ms: 1000.,
            translation: None,
            rotation: None,
            properties: vec![],
            event_lists: vec![vec![TimedProperty {
                bone: format!("foot{i}"),
                time_ms: case["times"][i].as_i64().unwrap() as i32,
                pairs: vec![("anchor".into(), (i + 1).to_string())],
            }]],
        })
        .collect();
    let base = Animation {
        name: "base".into(),
        skill: "base".into(),
        duration_ms: 1000.,
        distance: 0.,
        moving: false,
        translation_count: 0,
        rotation_count: 0,
        frames: 30,
        motions,
        events: vec![],
    };
    let mut program = Program::default();
    for (name, distance) in std::iter::once(("base".to_owned(), 0.)).chain(
        [19, 21, 20, 22, 23]
            .into_iter()
            .enumerate()
            .filter_map(|(i, n)| {
                case["distances"][i]
                    .as_f64()
                    .map(|d| (format!("clip{n}"), d as f32))
            }),
    ) {
        let mut clip = base.clone();
        clip.name = name.clone();
        clip.skill = name.clone();
        clip.distance = distance;
        program.animations.insert(
            name,
            AnimationAsset {
                clip,
                samples: Samples {
                    translations: vec![],
                    rotations: vec![],
                },
                source: "synthetic".into(),
                content_hash: "synthetic".into(),
                samples_source: "synthetic".into(),
                samples_hash: "synthetic".into(),
            },
        );
    }
    (source, program)
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_initializer_controller_order_weight_and_retained_state() {
    let report: Value = serde_json::from_str(include_str!(
        "fixtures/native-character-initialization.json"
    ))
    .unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let original = &row["original"];
        let (source, program) = resources(c);
        let profile = ["controller-rig", "", "", "foot0", "", "foot1", ""].map(str::to_owned);
        let settings = Settings {
            fade_enabled: false,
            fade_duration: 200,
            sampling_config: 1,
        };
        let mut e = Initialization::new(
            source,
            &profile,
            0,
            c["flags"].as_u64().unwrap() as u8,
            settings,
            &mut 1,
        )
        .unwrap();
        // Supplied nonzero retained fields expose accidental resets. This is a
        // controller test, not the constructor or complete household activation.
        e.scalars.clock = c["clock"].as_i64().unwrap() as i32;
        e.scalars.head_clock = 91;
        e.scalars.head_angles = [1., 2., 3.];
        e.scalars.head_target_point = Some([4., 5., 6.]);
        let mut person = (0..256).map(|i| (i, 0)).collect();
        let context = properties::Context {
            actor: 1,
            sound_visible: false,
            selected: false,
            debug_sound: false,
            position_x: -999.,
            appearance_mode: 0,
            snow: false,
            lot_kind: 1,
        };
        let mut requests = vec![];
        let mut samples = vec![];
        let mut placements = vec![];
        let mut geometry = 0;
        let terrain = vec![[0; 4]; 4];
        for _ in 0..40 {
            let Some(request) = e.next(&program, &mut person, context).unwrap() else {
                break;
            };
            let saved = e.clone();
            e = serde_json::from_slice(&serde_json::to_vec(&e).unwrap()).unwrap();
            assert_eq!(e, saved);
            assert_eq!(
                e.next(&program, &mut person, context).unwrap(),
                Some(request.clone())
            );
            assert_eq!(e, saved);
            requests.push(json!(request));
            match request {
                Request::NeedsAppearance { name } => e.needs_appearance(&name).unwrap(),
                Request::ReloadProfile => e.profile_reloaded().unwrap(), // supplied empty appearance service
                Request::Animation { index } => {
                    let name = if index == 3 {
                        "base".to_owned()
                    } else {
                        format!("clip{index}")
                    };
                    e.animation(
                        program
                            .animations
                            .contains_key(&name)
                            .then_some(name.as_str()),
                        &program,
                    )
                    .unwrap();
                    if let Some(id) = e.bound.skeleton.skin.animator.slot(Slot::Foreground) {
                        let mut a = json!(e.bound.skeleton.skin.animator);
                        let layer = a["layers"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .find(|l| l["id"] == json!(id))
                            .unwrap();
                        layer["layer"]["clock"]["speed"] = json!(2.25);
                        layer["layer"]["clock"]["weight"] = json!(0.125);
                        e.bound.skeleton.skin.animator = serde_json::from_value(a).unwrap();
                    }
                }
                Request::ZeroAltitude => e.altitude_written().unwrap(),
                Request::Placement => {
                    placements.push(u32::from(e.initialized));
                    if let Some(id) = e.bound.skeleton.skin.animator.slot(Slot::Foreground) {
                        assert_eq!(e.scalars.selection, 20);
                        let layer = e.bound.skeleton.skin.animator.layer(id).unwrap();
                        samples.push(json!({"speed":layer.clock.speed,"phase":layer.clock.phase,"weight":layer.clock.weight}));
                        // The native probe supplies the same first-pass weight
                        // change. The second phase setter must preserve it.
                        let mut a = json!(e.bound.skeleton.skin.animator);
                        let l = a["layers"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .find(|l| l["id"] == json!(id))
                            .unwrap();
                        l["layer"]["clock"]["weight"] = json!(0.375);
                        e.bound.skeleton.skin.animator = serde_json::from_value(a).unwrap();
                    }
                    e.place(
                        transform::Source {
                            position: [-16, -16],
                            facing: 0,
                            level: 1,
                            flags: [0, 0],
                        },
                        transform::View {
                            rotation: 0,
                            floor_vector: [0.; 3],
                        },
                        transform::Terrain {
                            size: 2,
                            corners: &terrain,
                        },
                    )
                    .unwrap();
                }
                Request::Geometry { force } => {
                    e.bound.refresh_geometry(force).unwrap();
                    // As in the native controller probe, absolute anchor poses
                    // are supplied. Real pose composition is compared separately.
                    for (i, bone) in e.anchors.into_iter().enumerate() {
                        e.bound.skeleton.hierarchy.absolute[bone.unwrap()].translation[2] =
                            c["z"][geometry][i].as_f64().unwrap() as f32;
                    }
                    geometry += 1;
                    e.geometry_done().unwrap();
                }
                Request::Viewport => e.viewport([4, 8, 640, 480]).unwrap(),
                Request::Visibility { .. } => e.visibility(false).unwrap(),
                Request::Property(p) => panic!("unexpected property {p:?}"),
            }
        }
        assert!(e.is_complete());
        assert_eq!(json!(samples), row["samples"]);
        let expected_requests: Vec<Value> = row["events"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|event| match event[0].as_str().unwrap() {
                "needs" => vec![json!(Request::NeedsAppearance {
                    name: event[1].as_str().unwrap().into()
                })],
                "reload_profile" => vec![json!(Request::ReloadProfile)],
                "animation" => vec![json!(Request::Animation {
                    index: event[1].as_u64().unwrap() as usize
                })],
                "foreground" => vec![json!(Request::Animation { index: 20 })],
                "altitude" => vec![json!(Request::ZeroAltitude)],
                "placement" => vec![json!(Request::Placement)],
                "geometry" => vec![json!(Request::Geometry { force: false })],
                "visibility" => vec![
                    json!(Request::Viewport),
                    json!(Request::Visibility {
                        viewport: [4, 8, 640, 480]
                    }),
                ],
                _ => vec![],
            })
            .collect();
        assert_eq!(requests, expected_requests);
        assert_eq!(
            placements,
            row["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r[0] == "placement")
                .map(|r| r[1].as_u64().unwrap() as u32)
                .collect::<Vec<_>>()
        );
        for (actual, key) in [
            (json!(e.scalars.clip_distances), "clip_distances"),
            (json!(e.scalars.phase_window), "phase_window"),
            (json!(e.scalars.stop_distance), "stop_distance"),
        ] {
            for (a, b) in actual
                .as_array()
                .unwrap()
                .iter()
                .zip(original[key].as_array().unwrap())
            {
                assert_eq!(
                    (a.as_f64().unwrap() as f32).to_bits(),
                    (b.as_f64().unwrap() as f32).to_bits(),
                    "{key}"
                );
            }
        }
        assert_eq!(json!(e.scalars.selection), original["selection"]);
        assert_eq!(json!(e.scalars.clock), original["clock"]);
        assert_eq!(json!(e.scalars.head_clock), original["head_clock"]);
        assert_eq!(
            json!([e.scalars.head_angles, e.scalars.head_target_point.unwrap()].concat()),
            original["head"]
        );
        assert!(
            e.bound
                .skeleton
                .skin
                .animator
                .slot(Slot::Foreground)
                .is_none()
        );
        assert_eq!(
            e.bound.skeleton.skin.animator.layers().len(),
            1,
            "calibration must not leave a fading foreground"
        );
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 24);
}
