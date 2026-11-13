use serde_json::{Value, json};
use sims_formats::{
    animation::{Animation, Motion, Samples, TimedProperty},
    avatar::{Bone, Skeleton},
};
use sims_sim::{
    head_look::{State, Target, ordinary_target_point, rig::Rig},
    pose::{
        animator::{Animator, Settings},
        live::Live,
        ordinary, properties,
        refresh::{Censor, Character, Hierarchy, Request, Response},
        transform::{Source, Terrain, View},
    },
};
use sims_vm::{Program, program::AnimationAsset};
use std::collections::BTreeMap;

fn compare(actual: &Value, expected: &Value, path: &str) {
    match (actual, expected) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, value) in b {
                compare(
                    a.get(key).unwrap_or_else(|| panic!("missing {path}.{key}")),
                    value,
                    &format!("{path}.{key}"),
                );
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &format!("{path}[{i}]"));
            }
        }
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => {
            // Native x87 and the Rust pose math can differ by a final f32 ULP.
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            assert!((a - b).abs() <= 3e-5, "{path}: {a} / {b}");
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}

fn absolute(character: &Character) -> Value {
    json!(
        character
            .hierarchy
            .absolute
            .iter()
            .zip(&character.hierarchy.updates)
            .map(|(b, stamp)| {
                json!({"translation":b.translation,"rotation":b.rotation,"updates":stamp})
            })
            .collect::<Vec<_>>()
    )
}

fn restore(character: &mut Character, person: &mut BTreeMap<i16, i16>) {
    let bytes = serde_json::to_vec(&(&*character, &*person)).unwrap();
    let (saved, saved_person) = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(*character, saved);
    assert_eq!(*person, saved_person);
    (*character, *person) = (saved, saved_person);
}

fn asset(v: &Value) -> AnimationAsset {
    let c = &v["clip"];
    let event = |e: &Value| TimedProperty {
        time_ms: e["time_ms"].as_i64().unwrap() as i32,
        bone: e["bone"].as_str().unwrap().into(),
        pairs: serde_json::from_value(e["pairs"].clone()).unwrap(),
    };
    AnimationAsset {
        source: v["source"].as_str().unwrap().into(),
        content_hash: v["content_hash"].as_str().unwrap().into(),
        samples_source: v["samples_source"].as_str().unwrap().into(),
        samples_hash: v["samples_hash"].as_str().unwrap().into(),
        clip: Animation {
            name: c["name"].as_str().unwrap().into(),
            skill: c["skill"].as_str().unwrap().into(),
            duration_ms: c["duration_ms"].as_f64().unwrap() as f32,
            distance: c["distance"].as_f64().unwrap() as f32,
            moving: c["moving"].as_bool().unwrap(),
            translation_count: c["translation_count"].as_u64().unwrap() as u32,
            rotation_count: c["rotation_count"].as_u64().unwrap() as u32,
            frames: c["frames"].as_u64().unwrap() as u32,
            events: c["events"].as_array().unwrap().iter().map(event).collect(),
            motions: c["motions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| Motion {
                    bone: m["bone"].as_str().unwrap().into(),
                    frames: m["frames"].as_u64().unwrap() as u32,
                    duration_ms: m["duration_ms"].as_f64().unwrap() as f32,
                    translation: serde_json::from_value(m["translation"].clone()).unwrap(),
                    rotation: serde_json::from_value(m["rotation"].clone()).unwrap(),
                    properties: serde_json::from_value(m["properties"].clone()).unwrap(),
                    event_lists: m["event_lists"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|list| list.as_array().unwrap().iter().map(event).collect())
                        .collect(),
                })
                .collect(),
        },
        samples: Samples {
            translations: serde_json::from_value(v["samples"]["translations"].clone()).unwrap(),
            rotations: serde_json::from_value(v["samples"]["rotations"].clone()).unwrap(),
        },
    }
}

fn run(report: Value, inputs: Vec<Value>) {
    if let Some(clocks) = report["clocks"].as_array() {
        for c in clocks {
            assert_eq!(
                json!(ordinary::clock(
                    c["tick"].as_i64().unwrap() as i32,
                    c["mode"].as_u64().unwrap() as u8
                )),
                c["result"]
            );
        }
    }
    let input = &inputs[0];
    let bones = input["bones"].as_array().unwrap();
    let skeleton = Skeleton {
        name: "native-refresh-input".into(),
        bones: input["hierarchy"]
            .as_array()
            .unwrap()
            .iter()
            .zip(bones)
            .map(|(h, b)| Bone {
                name: h["name"].as_str().unwrap().into(),
                parent: serde_json::from_value(h["parent"].clone()).unwrap(),
                translation: serde_json::from_value(b["local"]["translation"].clone()).unwrap(),
                rotation: serde_json::from_value(b["local"]["rotation"].clone()).unwrap(),
                properties: vec![],
                flags: [0, 0, b["blend_marker"].as_u64().unwrap() as u32],
                wiggle: Default::default(),
            })
            .collect(),
    };
    let profile: Vec<String> = serde_json::from_value(input["profile"].clone()).unwrap();
    let mut program = Program::default();
    for input in &inputs {
        program.animations.insert(
            input["asset"]["clip"]["name"].as_str().unwrap().into(),
            asset(&input["asset"]),
        );
    }
    let context = properties::Context {
        actor: 137,
        sound_visible: true,
        selected: true,
        debug_sound: false,
        position_x: 32.,
        appearance_mode: 0,
        snow: false,
        lot_kind: 1,
    };
    let mut count = 0;
    let mut services = 0;
    for (seq, row) in report["sequences"].as_array().unwrap().iter().enumerate() {
        let settings: Settings =
            serde_json::from_value(row["initial"]["settings"].clone()).unwrap();
        let before = &row["before"];
        let values: [f32; 6] = serde_json::from_value(before["head"].clone()).unwrap();
        let mut character = Character::new(
            Live::new(
                Animator::new(0, 0, 0, true, -1),
                serde_json::from_value(input["bones"].clone()).unwrap(),
            ),
            Hierarchy {
                parents: skeleton.bones.iter().map(|b| b.parent).collect(),
                absolute: before["absolute"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| serde_json::from_value(b.clone()).unwrap())
                    .collect(),
                updates: vec![0; bones.len()],
                evaluated_generation: 0,
            },
            State {
                last_clock: 0,
                basis_heading: values[0],
                target_heading: values[1],
                turn: values[2],
                target_point: Some([values[3], values[4], values[5]]),
            },
            Rig::bind(&skeleton, &profile).unwrap(),
            serde_json::from_value(before["matrix"].clone()).unwrap(),
            serde_json::from_value(before["placement"].clone()).unwrap(),
            serde_json::from_value(before["model"].clone()).unwrap(),
            Censor {
                binding: None,
                mask: 0,
            },
            0,
        )
        .unwrap();
        let mut person: BTreeMap<i16, i16> =
            serde_json::from_value(before["person"].clone()).unwrap();
        let mut prefix = before
            .get("prefix")
            .map(|state| ordinary::Prefix::new(serde_json::from_value(state.clone()).unwrap()));
        for (step, row) in row["steps"].as_array().unwrap().iter().enumerate() {
            let action = &row["action"];
            let expected = &row["original"];
            let label = format!("{seq}/{step}: {action}");
            let now = action["now"].as_i64().unwrap() as i32;
            let mut result = Value::Null;
            match action["op"].as_str().unwrap() {
                "start" => {
                    result = json!(
                        character
                            .skin
                            .animator
                            .start(
                                serde_json::from_value(action["slot"].clone()).unwrap(),
                                &skeleton,
                                Some(&program.animations[action["name"].as_str().unwrap()].clip),
                                action["reverse"].as_bool().unwrap(),
                                now,
                                settings,
                            )
                            .unwrap()
                    );
                }
                "clear" => character
                    .skin
                    .animator
                    .clear(
                        serde_json::from_value(action["slot"].clone()).unwrap(),
                        now,
                        settings,
                    )
                    .unwrap(),
                "refresh" => {
                    character
                        .model
                        .set_scale(serde_json::from_value(action["body_scale"].clone()).unwrap())
                        .unwrap();
                    let placement = &action["placement"];
                    let mut context = context;
                    let mut level = 1;
                    if !placement.is_null() {
                        character.placement = serde_json::from_value(placement.clone()).unwrap();
                        context.position_x = character.placement.base[0];
                        person.insert(84, placement["flags"][0].as_i64().unwrap() as i16);
                        person.insert(68, placement["flags"][1].as_i64().unwrap() as i16);
                        level = placement["level"].as_i64().unwrap() as i32;
                    }
                    person.insert(30, action["censor"].as_i64().unwrap() as i16);
                    person.insert(44, action["head_mode"].as_i64().unwrap() as i16);
                    if action["restart_head"].as_bool().unwrap() {
                        person.extend([(41, 2), (42, 1), (45, 60)]);
                    }
                    let mut calls = vec![];
                    let ordinary = action["ordinary_tail"] == true;
                    if expected.get("visible").is_some() {
                        character.visible = action["visible"] == 1;
                    }
                    if let Some(prefix) = &mut prefix {
                        services += run_prefix(
                            prefix,
                            &mut character,
                            &mut person,
                            &program,
                            &skeleton,
                            settings,
                            serde_json::from_value(report["rate_settings"].clone()).unwrap(),
                            &inputs,
                            action,
                            expected,
                            &label,
                        );
                    }
                    if action["loaded_tail"] == true {
                        character.begin_loaded_tail(true).unwrap();
                    } else if ordinary && action["prefix_only"] != true {
                        if prefix.is_none() {
                            character.skin.animator.discard_exhausted_blend().unwrap();
                        }
                        character
                            .begin_ordinary_tail(
                                now,
                                action["only_visible"] == 1,
                                true,
                                serde_json::from_value(action["initial_matrix"].clone()).unwrap(),
                            )
                            .unwrap();
                    } else if !ordinary
                        && character
                            .begin(
                                now,
                                action["only_visible"] == 1,
                                action["visible"] == 1,
                                true,
                            )
                            .unwrap()
                    {
                        calls.push(json!("clock"));
                    }
                    restore(&mut character, &mut person);
                    let mut geometry = 0;
                    let mut visibility = 0;
                    let mut placement_calls = 0;
                    while let Some(request) = character
                        .next(&program, &mut person, context, level)
                        .unwrap()
                    {
                        restore(&mut character, &mut person);
                        assert_eq!(
                            character
                                .next(&program, &mut person, context, level)
                                .unwrap(),
                            Some(request.clone()),
                            "{label}"
                        );
                        let unchanged = character.clone();
                        let wrong = if matches!(request, Request::Viewport) {
                            Response::Done
                        } else {
                            Response::Viewport([0; 4])
                        };
                        assert!(character.acknowledge(wrong).is_err());
                        assert_eq!(character, unchanged, "rejected response {label}");
                        let response = match request {
                            Request::LoadedAppearance => {
                                calls.push(json!("loaded_appearance"));
                                Response::Done // Explicit empty binding vector, as in the native probe.
                            }
                            Request::RefreshClock => {
                                calls.push(json!("clock"));
                                Response::Clock(now)
                            }
                            Request::Viewport => {
                                calls.push(json!("viewport"));
                                Response::Viewport([-10, -20, 640, 480])
                            }
                            Request::Visibility { viewport } => {
                                calls.push(json!({"visibility":viewport}));
                                let first = visibility == 0 && action["only_visible"] == 1;
                                visibility += 1;
                                Response::Visible(if first {
                                    action["visible"] == 1
                                } else {
                                    action["final_visible"] == true
                                })
                            }
                            Request::ModelGeometry => {
                                calls.push(json!({"hierarchy":0}));
                                calls.push(json!("model_geometry"));
                                compare(
                                    &absolute(&character),
                                    &expected["geometry"][geometry],
                                    &format!("{label} first model geometry"),
                                );
                                for bone in &character.hierarchy.absolute {
                                    bone.matrix().unwrap();
                                }
                                geometry += 1;
                                Response::Done
                            }
                            Request::ModelBounds => {
                                calls.push(json!("model_bounds"));
                                Response::Done
                            }
                            Request::RootMotionCorrection => {
                                calls.push(json!("root_correction"));
                                let source = action
                                    .get("root_source")
                                    .map(|v| serde_json::from_value(v.clone()).unwrap())
                                    .unwrap_or(sims_sim::pose::root_motion::Source {
                                        enabled: 0,
                                        movement_status: 0,
                                        excluded_person: false,
                                        world_word_30: 0,
                                        movement_mode: 0,
                                    });
                                character.correct_root(source).unwrap();
                                restore(&mut character, &mut person);
                                services += 1;
                                continue;
                            }
                            Request::Invalidate => {
                                calls.push(json!("invalidate"));
                                Response::Done
                            }
                            Request::BindCensor { mask } => {
                                calls.push(json!({"bind_censor":mask as u32}));
                                Response::CensorBinding(
                                    action["bind_present"].as_bool().unwrap().then_some(1),
                                )
                            }
                            Request::UnbindCensor { binding } => {
                                assert_eq!(binding, 1);
                                calls.push(json!("unbind_censor"));
                                Response::Done
                            }
                            Request::Property(request) => {
                                calls.push(json!({"property":request}));
                                match request {
                                    properties::Request::FootstepSurface { .. } => {
                                        Response::PropertySurface(1)
                                    }
                                    properties::Request::Sound { .. } => Response::Done,
                                    other => panic!("unexpected property service {other:?}"),
                                }
                            }
                            Request::Placement => {
                                calls.push(json!("placement"));
                                let expected_placement = expected
                                    .get("placement_results")
                                    .and_then(|v| v.get(placement_calls))
                                    .unwrap_or(expected);
                                placement_calls += 1;
                                if !placement.is_null() {
                                    let size = placement["size"].as_u64().unwrap() as usize;
                                    let corners: [u8; 4] =
                                        serde_json::from_value(placement["corners"].clone())
                                            .unwrap();
                                    let heights = (0..size)
                                        .flat_map(|x| {
                                            (0..size).map(move |y| {
                                                corners
                                                    .map(|v| v.wrapping_add((x * 3 + y * 5) as u8))
                                            })
                                        })
                                        .collect::<Vec<_>>();
                                    let transform = character
                                        .place(
                                            Source {
                                                position: serde_json::from_value(
                                                    placement["position"].clone(),
                                                )
                                                .unwrap(),
                                                facing: placement["facing"].as_i64().unwrap()
                                                    as i16,
                                                level,
                                                flags: serde_json::from_value(
                                                    placement["flags"].clone(),
                                                )
                                                .unwrap(),
                                            },
                                            View {
                                                rotation: placement["rotation"].as_u64().unwrap()
                                                    as u8,
                                                floor_vector: serde_json::from_value(
                                                    placement["floor_vectors"][placement["zoom"]
                                                        .as_u64()
                                                        .unwrap()
                                                        as usize]
                                                        .clone(),
                                                )
                                                .unwrap(),
                                            },
                                            Terrain {
                                                size,
                                                corners: &heights,
                                            },
                                        )
                                        .unwrap();
                                    compare(
                                        &json!(transform.matrix),
                                        &expected_placement["matrix"],
                                        &format!("{label} placement"),
                                    );
                                    restore(&mut character, &mut person);
                                    services += 1;
                                    continue;
                                }
                                Response::Placement(sims_sim::pose::model::Matrix {
                                    values: serde_json::from_value(
                                        expected_placement["matrix"].clone(),
                                    )
                                    .unwrap(),
                                    kind: 0,
                                })
                            }
                            Request::Geometry { branch, force } => {
                                if branch.is_none() {
                                    calls.push(json!({"hierarchy":u8::from(force)}));
                                }
                                calls.push(json!({"geometry":{"branch":branch,"force":force}}));
                                compare(
                                    &absolute(&character),
                                    &expected["geometry"][geometry],
                                    &format!("{label} geometry {geometry}"),
                                );
                                // Explicit empty-appearance host: derive attachment matrices.
                                // This does not simulate native mesh/bounding-box cache stamps.
                                for bone in &character.hierarchy.absolute {
                                    bone.matrix().unwrap();
                                }
                                geometry += 1;
                                Response::Done
                            }
                            Request::HeadTarget { object } => {
                                assert_eq!(object, 2);
                                calls.push(json!("target"));
                                Response::HeadTarget(
                                    action["target_present"].as_bool().unwrap().then(|| Target {
                                        level: action["target_level"].as_i64().unwrap() as i32,
                                        point: Some(ordinary_target_point(
                                            serde_json::from_value(
                                                action["target_position"].clone(),
                                            )
                                            .unwrap(),
                                        )),
                                    }),
                                )
                            }
                            Request::CarriedObject | Request::OrdinaryCarriedObject => {
                                Response::Done
                            } // Explicit empty carried slot in both hosts.
                        };
                        character.acknowledge(response).unwrap();
                        restore(&mut character, &mut person);
                        services += 1;
                    }
                    assert!(!character.busy(), "{label}");
                    assert_eq!(json!(calls), expected["calls"], "calls {label}");
                    assert_eq!(geometry, expected["geometry"].as_array().unwrap().len());
                    compare(
                        &absolute(&character),
                        &expected["absolute"],
                        &format!("{label} absolute"),
                    );
                    assert_eq!(json!(person), expected["person"], "person {label}");
                    if expected.get("visible").is_some() {
                        assert_eq!(
                            json!(character.visible),
                            expected["visible"],
                            "visible {label}"
                        );
                    }
                    assert_eq!(json!(character.clock), expected["clock"], "clock {label}");
                    compare(
                        &json!(character.model),
                        &expected["model"],
                        &format!("{label} model publication"),
                    );
                    compare(
                        &json!(character.matrix),
                        &expected["skin_matrix"],
                        &format!("{label} effective matrix"),
                    );
                    compare(
                        &json!(character.placement),
                        &expected["placement"],
                        &format!("{label} placement caches"),
                    );
                    assert_eq!(
                        json!(character.head.last_clock),
                        expected["head_clock"],
                        "head clock {label}"
                    );
                    assert_eq!(
                        json!(character.hierarchy.evaluated_generation),
                        expected["evaluated_generation"],
                        "stamp {label}"
                    );
                    assert_eq!(
                        json!(character.censor.binding.is_some()),
                        expected["censor_bound"],
                        "binding {label}"
                    );
                    assert_eq!(
                        json!(character.censor.mask),
                        expected["censor_mask"],
                        "mask {label}"
                    );
                    let head = &character.head;
                    compare(
                        &json!([
                            head.basis_heading,
                            head.target_heading,
                            head.turn,
                            head.target_point.unwrap()[0],
                            head.target_point.unwrap()[1],
                            head.target_point.unwrap()[2]
                        ]),
                        &expected["head"],
                        &format!("{label} head"),
                    );
                    if !expected["during"].is_null() {
                        let during = expected["during"].as_array().unwrap().iter().map(|b| json!({"translation":b["translation"],"rotation":b["rotation"]})).collect::<Vec<_>>();
                        compare(
                            &json!(character.displayed_pose().unwrap()),
                            &json!(during),
                            &format!("{label} display"),
                        );
                    }
                }
                other => panic!("unexpected action {other}"),
            }
            let mut projection = expected.clone();
            for key in [
                "bones",
                "released",
                "actions",
                "callback_count",
                "calls",
                "geometry",
                "absolute",
                "during",
                "matrix",
                "model",
                "skin_matrix",
                "head",
                "head_clock",
                "clock",
                "person",
                "censor_bound",
                "censor_mask",
                "evaluated_generation",
                "placement",
                "placement_calls",
                "placement_results",
                "visible",
                "prefix",
                "prefix_calls",
                "continuation",
            ] {
                projection.as_object_mut().unwrap().remove(key);
            }
            let mut actual = serde_json::to_value(&character.skin.animator).unwrap();
            actual["result"] = result;
            compare(&actual, &projection, &label);
            for (i, (bone, expected)) in character
                .skin
                .bones
                .iter()
                .zip(expected["bones"].as_array().unwrap())
                .enumerate()
            {
                compare(
                    &json!({"translation":bone.local.translation,"rotation":bone.local.rotation,"priority":bone.priority}),
                    expected,
                    &format!("{label} bone {i}"),
                );
            }
            restore(&mut character, &mut person);
            count += 1;
        }
    }
    assert_eq!(count, 154);
    println!(
        "Compared {count} native refresh/ownership steps and {services} resumable host services"
    );
}

fn run_prefix(
    prefix: &mut ordinary::Prefix,
    character: &mut Character,
    person: &mut BTreeMap<i16, i16>,
    program: &Program,
    skeleton: &Skeleton,
    settings: Settings,
    rate: ordinary::RateSettings,
    inputs: &[Value],
    action: &Value,
    expected: &Value,
    label: &str,
) -> usize {
    use ordinary::{Entry, Request, Response, Source};
    let c = &action["prefix"];
    prefix.state.clock_mode = c["clock_mode"].as_u64().unwrap() as u8;
    prefix.state.pickup = c["pickup"].as_i64().unwrap() as i32;
    prefix.state.carry.mode = c["carry_mode"].as_i64().unwrap() as i32;
    prefix.state.mode = c["mode"].as_i64().unwrap() as i32;
    person.insert(0, c["posture"].as_i64().unwrap() as i16);
    person.insert(64, c["movement_input"].as_i64().unwrap() as i16);
    person.insert(65, c["species"].as_i64().unwrap() as i16);
    let mut walk_style = c["walk_style"].as_i64().unwrap() as i16;
    prefix
        .begin(
            character,
            c["tick"].as_i64().unwrap() as i32,
            Entry {
                root_motion: 0,
                movement_input: person[&64],
                walk_style,
            },
        )
        .unwrap();
    let mut calls = vec![];
    loop {
        let source = Source {
            posture: person[&0],
            species: person[&65],
            definition_flags: c["flags"].as_u64().unwrap() as u8,
            walk_style,
        };
        let Some(request) = prefix.next(character, source, settings, rate).unwrap() else {
            break;
        };
        let saved = serde_json::to_vec(&(&*prefix, &*character, &*person, walk_style)).unwrap();
        (*prefix, *character, *person, walk_style) = serde_json::from_slice(&saved).unwrap();
        assert_eq!(
            prefix.next(character, source, settings, rate).unwrap(),
            Some(request.clone())
        );
        let current = serde_json::to_vec(&(&*prefix, &*character)).unwrap();
        let wrong = match request {
            Request::OutsideLot => Response::CarriedSlot(false),
            _ => Response::OutsideLot(false),
        };
        assert!(
            prefix
                .acknowledge(character, program, skeleton, settings, wrong)
                .is_err()
        );
        assert_eq!(
            serde_json::to_vec(&(&*prefix, &*character)).unwrap(),
            current
        );
        let slots = [
            sims_sim::pose::animator::Slot::Foreground,
            sims_sim::pose::animator::Slot::Carry,
            sims_sim::pose::animator::Slot::Base,
        ]
        .map(|s| character.skin.animator.slot(s));
        calls.push(
            json!({"request":request,"state":prefix.state,"clock":character.clock,"slots":slots}),
        );
        let response = match request {
            Request::CarriedSlot => Response::CarriedSlot(c["held"] == true),
            Request::Animation { index, .. } => {
                if index == 38 && c["mutate_lookup"] == true {
                    person.insert(0, 2);
                    person.insert(65, 3);
                    walk_style = 0;
                    prefix.state.clock_mode = 1;
                }
                let present = c[if index == 38 {
                    "carry_present"
                } else {
                    "base_present"
                }] == true;
                Response::Animation {
                    name: present.then(|| {
                        inputs[index as usize % inputs.len()]["asset"]["clip"]["name"]
                            .as_str()
                            .unwrap()
                            .into()
                    }),
                    ready: index != 38 || c["carry_ready"] == true,
                }
            }
            Request::OutsideLot => Response::OutsideLot(c["outside"] == true),
        };
        prefix
            .acknowledge(character, program, skeleton, settings, response)
            .unwrap();
        let saved = serde_json::to_vec(&(&*prefix, &*character, &*person, walk_style)).unwrap();
        (*prefix, *character, *person, walk_style) = serde_json::from_slice(&saved).unwrap();
    }
    let result = prefix.finish().unwrap();
    assert_eq!(
        json!(result),
        expected["continuation"],
        "continuation {label}"
    );
    assert_eq!(
        prefix.state,
        serde_json::from_value(expected["prefix"].clone()).unwrap(),
        "prefix state {label}"
    );
    compare(
        &json!(calls),
        &expected["prefix_calls"],
        &format!("prefix services {label}"),
    );
    calls.len()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn placement_refresh_matches_native_pose_head_callback_and_service_order() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-skin-refresh.json")).unwrap();
    run(
        report.clone(),
        serde_json::from_value(report["inputs"].clone()).unwrap(),
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn saved_animator_load_tail_retains_first_clock_and_forces_second_refresh() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-saved-animator-tail.json")).unwrap();
    run(
        report.clone(),
        serde_json::from_value(report["inputs"].clone()).unwrap(),
    );
}

#[test]
#[ignore = "requires private original clips and saved animator tail report"]
fn original_saved_animator_load_tail_and_callback_order() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let inputs: Vec<Value> = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-layer-updates-input.json")).unwrap(),
    )
    .unwrap();
    let inputs = inputs
        .into_iter()
        .filter(|c| c["layer"]["clock"]["speed"].as_f64().unwrap() > 0.)
        .collect();
    let report = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-saved-animator-tail.json")).unwrap(),
    )
    .unwrap();
    run(report, inputs);
}

#[test]
#[ignore = "requires private walking clip, rig and native calibration report"]
fn original_walking_anchor_measurements_match_native_initializer() {
    use sims_sim::pose::{
        layer::{Layer, Start},
        movement::calibration,
        sampling::BoneInput,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let inputs: Vec<Value> = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-layer-updates-input.json")).unwrap(),
    )
    .unwrap();
    let input = inputs
        .iter()
        .find(|v| v["asset"]["clip"]["name"] == "a2o-walking-loop")
        .unwrap();
    let asset = asset(&input["asset"]);
    let mut bones: Vec<BoneInput> = serde_json::from_value(input["bones"].clone()).unwrap();
    let skeleton = Skeleton {
        name: "native-measurement-input".into(),
        bones: input["hierarchy"]
            .as_array()
            .unwrap()
            .iter()
            .zip(&bones)
            .map(|(h, b)| Bone {
                name: h["name"].as_str().unwrap().into(),
                parent: serde_json::from_value(h["parent"].clone()).unwrap(),
                translation: b.local.translation,
                rotation: b.local.rotation,
                properties: vec![],
                flags: [0, 0, b.blend_marker],
                wiggle: Default::default(),
            })
            .collect(),
    };
    let anchors = ["1", "2"].map(|v| {
        calibration::property(&asset.clip, "anchor", v)
            .unwrap()
            .unwrap()
    });
    let phase_window =
        calibration::phases(anchors.map(|(_, t)| t), asset.clip.duration_ms).unwrap();
    let anchor_bones = anchors.map(|(m, _)| {
        skeleton
            .bones
            .iter()
            .position(|b| b.name == asset.clip.motions[m].bone)
            .unwrap()
    });
    let mut layer = Layer::bind(
        &skeleton,
        &asset.clip,
        Start {
            priority: 1,
            speed: 1.,
            weight: 1.,
            now: 0,
        },
    )
    .unwrap();
    let mut z = [[0.; 2]; 2];
    for (i, phase) in phase_window.into_iter().enumerate() {
        layer.clock.phase = phase;
        layer
            .sample(&asset.clip, &asset.samples, &mut bones, 0.)
            .unwrap();
        let absolute = sims_sim::pose::hierarchy(
            &skeleton.bones.iter().map(|b| b.parent).collect::<Vec<_>>(),
            &bones.iter().map(|b| b.local).collect::<Vec<_>>(),
            sims_sim::head_look::rig::AbsoluteBone {
                translation: [0.; 3],
                rotation: [0., 0., 0., 1.],
            },
        )
        .unwrap();
        z[i] = anchor_bones.map(|bone| absolute[bone].translation[2]);
    }
    let report: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-movement-calibration.json")).unwrap(),
    )
    .unwrap();
    let actual = json!({"anchors":anchors,"anchor_bones":anchor_bones,"phase_window":phase_window,"z":z,"stop_distance":calibration::stop_distances(z).unwrap()});
    for key in [
        "anchors",
        "anchor_bones",
        "phase_window",
        "z",
        "stop_distance",
    ] {
        compare(&actual[key], &report["original"][key], key);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn ordinary_two_pass_update_matches_native_through_save_resume() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-ordinary-skin.json")).unwrap();
    run(
        report.clone(),
        serde_json::from_value(report["inputs"].clone()).unwrap(),
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn ordinary_and_forced_refresh_share_native_character_state() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-mixed-skin.json")).unwrap();
    run(
        report.clone(),
        serde_json::from_value(report["inputs"].clone()).unwrap(),
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_ordinary_prefix_and_shared_tail() {
    for text in [
        include_str!("fixtures/native-ordinary-prefix.json"),
        include_str!("fixtures/native-prefix-skin.json"),
    ] {
        let report: Value = serde_json::from_str(text).unwrap();
        run(
            report.clone(),
            serde_json::from_value(report["inputs"].clone()).unwrap(),
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_root_correction_inside_ordinary_skin_placement_and_head() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-ordinary-root.json")).unwrap();
    run(
        report.clone(),
        serde_json::from_value(report["inputs"].clone()).unwrap(),
    );
}

#[test]
#[ignore = "requires private original clips and root-correction report"]
fn original_ordinary_root_correction_and_repeated_placement() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let inputs: Vec<Value> = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-layer-updates-input.json")).unwrap(),
    )
    .unwrap();
    let inputs = inputs
        .into_iter()
        .filter(|c| c["layer"]["clock"]["speed"].as_f64().unwrap() > 0.)
        .collect();
    let report = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-ordinary-root.json")).unwrap(),
    )
    .unwrap();
    run(report, inputs);
}

#[test]
#[ignore = "requires private original clips and ordinary animator reports"]
fn original_ordinary_two_pass_update_and_terrain_placement() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let inputs: Vec<Value> = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-layer-updates-input.json")).unwrap(),
    )
    .unwrap();
    let inputs: Vec<_> = inputs
        .into_iter()
        .filter(|c| c["layer"]["clock"]["speed"].as_f64().unwrap() > 0.)
        .collect();
    for file in [
        "cache/original-ordinary-skin.json",
        "cache/original-ordinary-placement.json",
        "cache/original-mixed-placement.json",
        "cache/original-prefix-placement.json",
    ] {
        let report = serde_json::from_slice(&std::fs::read(root.join(file)).unwrap()).unwrap();
        run(report, inputs.clone());
    }
}

#[test]
#[ignore = "requires private decoded original clips and native placement-refresh report"]
fn original_placement_refresh_matches_native_through_save_resume() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let inputs: Vec<Value> = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-layer-updates-input.json")).unwrap(),
    )
    .unwrap();
    let report = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-skin-refresh.json")).unwrap(),
    )
    .unwrap();
    run(
        report,
        inputs
            .into_iter()
            .filter(|c| c["layer"]["clock"]["speed"].as_f64().unwrap() > 0.)
            .collect(),
    );
}

#[test]
#[ignore = "requires private original clips and native combined placement/refresh report"]
fn original_refresh_uses_native_terrain_placement_and_restores_head_state() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let inputs: Vec<Value> = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-layer-updates-input.json")).unwrap(),
    )
    .unwrap();
    let report = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-skin-placement.json")).unwrap(),
    )
    .unwrap();
    run(
        report,
        inputs
            .into_iter()
            .filter(|c| c["layer"]["clock"]["speed"].as_f64().unwrap() > 0.)
            .collect(),
    );
}
