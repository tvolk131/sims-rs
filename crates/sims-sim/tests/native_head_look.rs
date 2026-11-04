use serde_json::{Value, json};
use sims_sim::head_look::{Fields, State, Target, ordinary_target_point};

#[cfg(feature = "private-fixtures")]
#[test]
fn fresh_head_targets_remain_undefined_until_original_writes_them() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-fresh-head-target.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let points: [[f32; 3]; 3] = serde_json::from_value(c["points"].clone()).unwrap();
        let position: [i32; 2] = serde_json::from_value(c["position"].clone()).unwrap();
        let mut state = State {
            last_clock: 0,
            basis_heading: 0.,
            target_heading: 0.,
            turn: 0.,
            target_point: None,
        };
        let mut fields = Fields {
            target: c["target"].as_i64().unwrap() as i16,
            flags: c["flags"].as_i64().unwrap() as i16,
            mode: 0,
            countdown: 0,
        };
        state.advance(1, &mut fields).unwrap();
        let result = state.observe(
            &mut fields,
            1,
            c["bones"] == 3,
            |id| {
                Ok((id == 2).then_some(Target {
                    level: 1,
                    point: if c["person"] == true {
                        (c["target_bone"] == true).then_some(points[2])
                    } else {
                        Some(ordinary_target_point(position))
                    },
                }))
            },
            || Ok([points[0], points[1]]),
        );
        if !row["undefined_reads"].as_array().unwrap().is_empty() {
            assert!(result.unwrap_err().contains("uninitialized"), "{c}");
            assert!(state.target_point.is_none());
        } else {
            result.unwrap();
            let expected = &row["original"];
            assert_eq!(json!(fields.target), expected["target"], "{c}");
            assert_eq!(json!(fields.flags), expected["flags"], "{c}");
            assert_eq!(
                json!(state.basis_heading.to_bits()),
                expected["body_bits"],
                "{c}"
            );
            assert_eq!(
                json!(state.target_heading.to_bits()),
                expected["angle_bits"],
                "{c}"
            );
            assert_eq!(json!(state.target_point.is_some()), row["defined"], "{c}");
            if let Some(point) = state.target_point {
                assert_eq!(
                    json!(point.map(f32::to_bits)),
                    expected["latched_bits"],
                    "{c}"
                );
            }
        }
        assert_eq!(
            serde_json::from_value::<State>(json!(&state)).unwrap(),
            state
        );
    }
}
use std::cell::RefCell;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_combined_head_updates_preserve_reentry_expiry_and_resume() {
    use sims_sim::head_look::{
        compose,
        rig::{AbsoluteBone, Kind},
    };
    let report: Value =
        serde_json::from_str(include_str!("fixtures/head-look-sequence.json")).unwrap();
    let base: [f32; 4] = serde_json::from_value(report["quaternion"].clone()).unwrap();
    let translation = serde_json::from_value(report["translation"].clone()).unwrap();
    let actor = serde_json::from_value(report["actor"].clone()).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 18);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut state = State {
            last_clock: 99,
            basis_heading: 0.,
            target_heading: 2.,
            turn: c["turn"].as_f64().unwrap() as f32,
            target_point: Some([13., 17., 23.]),
        };
        let mut fields = Fields {
            target: 2,
            flags: 1,
            mode: c["mode"].as_i64().unwrap() as i16,
            countdown: c["countdown"].as_i64().unwrap() as i16,
        };
        let rows = row["original"].as_array().unwrap();
        assert_eq!(rows.len(), 90);
        for (i, step) in rows.iter().enumerate() {
            if step["restart"] == true {
                fields.target = 2;
                fields.flags = 1;
            }
            let clock = step["clock"].as_u64().unwrap() as u32;
            state.advance(clock, &mut fields).unwrap();
            let bone = AbsoluteBone {
                translation,
                rotation: compose(state.turn, base).unwrap().unwrap_or(base),
            };
            state
                .observe(
                    &mut fields,
                    1,
                    true,
                    |id| {
                        assert_eq!(id, 2);
                        Ok(Some(Target {
                            level: step["level"].as_i64().unwrap() as i32,
                            point: Some(ordinary_target_point(
                                serde_json::from_value(step["position"].clone()).unwrap(),
                            )),
                        }))
                    },
                    || {
                        Ok([
                            bone.anchor_point(Kind::Top, actor)?,
                            bone.anchor_point(Kind::HeadForward, actor)?,
                        ])
                    },
                )
                .unwrap();
            assert_eq!(
                json!([fields.target, fields.flags, fields.mode, fields.countdown]),
                step["fields"],
                "{c} update {i}"
            );
            let want: [f32; 6] = serde_json::from_value(step["state"].clone()).unwrap();
            for (got, want) in [
                state.basis_heading,
                state.target_heading,
                state.turn,
                state.target_point.unwrap()[0],
                state.target_point.unwrap()[1],
                state.target_point.unwrap()[2],
            ]
            .into_iter()
            .zip(want)
            {
                assert!(
                    (got - want).abs() < 0.00001,
                    "{c} update {i}: {got} != {want}"
                );
            }
            assert_eq!(state.last_clock, clock);
            // Every subsequent step continues from the serialized value. This
            // includes repeated clocks, expired timers and target invalidation.
            let restored: (State, Fields) =
                serde_json::from_slice(&serde_json::to_vec(&(&state, fields)).unwrap()).unwrap();
            assert_eq!(restored, (state, fields));
            (state, fields) = restored;
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_profile_binds_head_hand_pelvis_and_forward_in_order() {
    use sims_formats::avatar::{Bone, Skeleton};
    use sims_sim::head_look::rig::Rig;
    let report: Value =
        serde_json::from_str(include_str!("fixtures/head-rig-profile.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 216);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let bones: Vec<String> = serde_json::from_value(c["bones"].clone()).unwrap();
        let skeleton = Skeleton {
            name: "synthetic".into(),
            bones: bones
                .iter()
                .map(|name| Bone {
                    name: name.clone(),
                    parent: None,
                    translation: [0.; 3],
                    rotation: [0., 0., 0., 1.],
                    properties: vec![],
                    flags: [0; 3],
                    wiggle: Default::default(),
                })
                .collect(),
        };
        let mut profile = vec![String::new(); 3];
        profile.extend(serde_json::from_value::<Vec<String>>(c["profile"].clone()).unwrap());
        let rig = Rig::bind(&skeleton, &profile).unwrap();
        assert_eq!(rig.turn_bone, bones.iter().position(|n| n == "HEAD"));
        for (i, anchor) in [rig.head, rig.hand, rig.pelvis, rig.forward]
            .into_iter()
            .enumerate()
        {
            let want = &row["original"]["anchors"][i];
            assert_eq!(anchor.is_none(), want.is_null(), "{c}, anchor {i}");
            if let Some(a) = anchor {
                assert_eq!(json!(a.bone), want["bone"], "{c}, anchor {i}");
                let matrix: [f32; 16] = serde_json::from_value(want["local"].clone()).unwrap();
                for (got, want) in a.kind.local_matrix().into_iter().zip(matrix) {
                    assert!(
                        (got - want).abs() < 1e-6,
                        "{c}, anchor {i}: {got} != {want}"
                    );
                }
            }
        }
        assert_eq!(row["original"]["entries"], json!([4, 5, 6, 7]));
        assert_eq!(serde_json::from_value::<Rig>(json!(&rig)).unwrap(), rig);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_rig_anchor_selection_and_world_points_match() {
    use sims_sim::head_look::rig::{AbsoluteBone, Kind};
    let report: Value = serde_json::from_str(include_str!("fixtures/head-anchors.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 704);
    assert_eq!(
        report["head_bone"],
        json!({"name":sims_sim::head_look::rig::HEAD_BONE,"index":31})
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let kind = Kind::named(c["anchor"].as_str()).filter(|_| c["bone"] == true);
        assert_eq!(kind.is_none(), row["original"].is_null(), "{c}");
        if let Some(kind) = kind {
            let bone = AbsoluteBone {
                translation: serde_json::from_value(c["translation"].clone()).unwrap(),
                rotation: serde_json::from_value(c["quaternion"].clone()).unwrap(),
            };
            let matrix: [f32; 16] =
                serde_json::from_value(row["original"]["local"].clone()).unwrap();
            for (got, want) in kind.local_matrix().into_iter().zip(matrix) {
                assert!((got - want).abs() < 1e-6, "{c}: {got} != {want}");
            }
            let point: [f32; 3] = serde_json::from_value(row["original"]["point"].clone()).unwrap();
            let got = bone
                .anchor_point(kind, serde_json::from_value(c["actor"].clone()).unwrap())
                .unwrap();
            for (got, want) in got.into_iter().zip(point) {
                assert!((got - want).abs() < 0.00002, "{c}: {got} != {want}");
            }
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_head_quaternion_limits_and_temporary_restore_match() {
    let report: Value = serde_json::from_str(include_str!("fixtures/head-look-pose.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 450);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let q: [f32; 4] = serde_json::from_value(c["quaternion"].clone()).unwrap();
        let result = sims_sim::head_look::compose(c["turn"].as_f64().unwrap() as f32, q).unwrap();
        assert_eq!(
            json!(result.is_some()),
            row["original"]["during"]["changed"],
            "{c}"
        );
        let got = result.unwrap_or(q);
        for (i, value) in got.into_iter().enumerate() {
            let expected = f32::from_bits(
                row["original"]["during"]["quaternion_bits"][i]
                    .as_u64()
                    .unwrap() as u32,
            );
            assert!(
                (value - expected).abs() < 0.000001,
                "{c}: component {i}: {value} != {expected}"
            );
        }
        assert_eq!(
            json!(q.map(f32::to_bits)),
            row["original"]["after"]["quaternion_bits"],
            "{c}"
        );
        assert_eq!(row["original"]["after"]["changed"], false);
        assert_eq!(
            row["original"]["events"],
            if result.is_some() {
                json!([
                    "lookup",
                    "invalidate",
                    "propagate",
                    "lookup",
                    "invalidate",
                    "propagate"
                ])
            } else {
                json!([])
            }
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_head_clock_and_turn_match_with_same_clock_reentry() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/head-look-clock.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 1760);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut fields = Fields {
            target: 2,
            flags: c["flags"].as_i64().unwrap() as i16,
            mode: c["mode"].as_i64().unwrap() as i16,
            countdown: c["countdown"].as_i64().unwrap() as i16,
        };
        let mut state = State {
            last_clock: c["last_clock"].as_i64().unwrap() as u32,
            basis_heading: c["body"].as_f64().unwrap() as f32,
            target_heading: c["target"].as_f64().unwrap() as f32,
            turn: c["turn"].as_f64().unwrap() as f32,
            target_point: Some([0.; 3]),
        };
        let compose = state
            .advance(c["clock"].as_u64().unwrap() as u32, &mut fields)
            .unwrap();
        assert_eq!(
            json!({"flags":fields.flags,"countdown":fields.countdown,
            "last_clock":state.last_clock,"turn_bits":state.turn.to_bits(),"compose_head":compose}),
            row["original"],
            "{c}"
        );
        let mut resumed: State = serde_json::from_value(json!(&state)).unwrap();
        assert_eq!(resumed, state);
        let before = fields;
        let again = resumed.advance(state.last_clock, &mut fields).unwrap();
        assert_eq!((resumed, fields, again), (state, before, compose));
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_head_target_order_missing_bones_and_angle_spills_match() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/head-look-target.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 978);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let points: [[f32; 3]; 3] = serde_json::from_value(c["points"].clone()).unwrap();
        let position: [i32; 2] = serde_json::from_value(c["position"].clone()).unwrap();
        let samples = RefCell::new(vec![]);
        let mut state = State {
            last_clock: 99,
            basis_heading: c["body"].as_f64().unwrap() as f32,
            target_heading: c["angle"].as_f64().unwrap() as f32,
            turn: 0.,
            target_point: Some(serde_json::from_value(c["latched"].clone()).unwrap()),
        };
        let mut fields = Fields {
            target: c["target"].as_i64().unwrap() as i16,
            flags: c["flags"].as_i64().unwrap() as i16,
            mode: 1,
            countdown: 60,
        };
        state
            .observe(
                &mut fields,
                1,
                c["bones"] == 3,
                |id| {
                    Ok(match id {
                        1 => {
                            samples.borrow_mut().push(0);
                            Some(Target {
                                level: 1,
                                point: Some(points[0]),
                            })
                        }
                        2 => Some(Target {
                            level: c["level"].as_i64().unwrap() as i32,
                            point: if c["person"] == true {
                                if c["target_bone"] == true {
                                    samples.borrow_mut().push(2);
                                    Some(points[2])
                                } else {
                                    None
                                }
                            } else {
                                Some(ordinary_target_point(position))
                            },
                        }),
                        _ => None,
                    })
                },
                || {
                    samples.borrow_mut().extend([0, 1]);
                    Ok([points[0], points[1]])
                },
            )
            .unwrap();
        assert_eq!(
            json!({"target":fields.target,"flags":fields.flags,
            "body_bits":state.basis_heading.to_bits(),"angle_bits":state.target_heading.to_bits(),
            "latched_bits":state.target_point.unwrap().map(f32::to_bits),"samples":samples.into_inner()}),
            row["original"],
            "{c}"
        );
        assert_eq!(fields.mode, 1);
        assert_eq!(fields.countdown, 60);
        assert_eq!(
            serde_json::from_value::<State>(json!(&state)).unwrap(),
            state
        );
    }
}
