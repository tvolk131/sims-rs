use pose::{
    animator::{Animator, Settings},
    live::Live,
    model::Matrix,
    refresh::{Censor, Character, Hierarchy},
};
use serde_json::{Value, json};
use sims_sim::{head_look, pose};
#[path = "native_layer_ownership.rs"]
mod ownership;

fn decode<T: serde::de::DeserializeOwned>(v: &Value) -> T {
    serde_json::from_value(v.clone()).unwrap()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_reset_preserves_fading_layers_and_head_state() {
    let rows: Value =
        serde_json::from_str(include_str!("fixtures/native-animator-reset.json")).unwrap();
    let (skeleton, program) = ownership::resources();
    for row in rows["cases"].as_array().unwrap() {
        let settings: Settings = decode(&row["initial"]["settings"]);
        let mut animator = Animator::new(1500, 9, 0, true, 17);
        for step in row["steps"].as_array().unwrap() {
            let a = &step["action"];
            animator
                .start(
                    decode(&a["slot"]),
                    &skeleton,
                    Some(&program.animations["synthetic"].clip),
                    a["reverse"].as_bool().unwrap(),
                    a["now"].as_i64().unwrap() as i32,
                    settings,
                )
                .unwrap();
        }
        // Establish a nonempty anchor and event queue to ensure the reset clears
        // them without replacing other owner storage with fresh constructor state.
        let mut encoded = json!(animator);
        encoded["anchor_valid"] = json!(true);
        encoded["anchor_bone"] = json!(0);
        encoded["numbered_events"] = json!([3, 7, 11]);
        animator = decode(&encoded);
        let b = &row["before"];
        let bones = vec![pose::sampling::BoneInput {
            local: pose::LocalBone {
                translation: [0.; 3],
                rotation: [0., 0., 0., 1.],
            },
            priority: 0,
            blend_marker: 0,
        }];
        let h: Vec<f32> = decode(&b["head"]);
        let mut character = Character::new(
            Live::new(animator, bones),
            Hierarchy {
                parents: vec![None],
                absolute: vec![head_look::rig::AbsoluteBone {
                    translation: [0.; 3],
                    rotation: [0., 0., 0., 1.],
                }],
                updates: vec![0],
                evaluated_generation: 0,
            },
            head_look::State {
                last_clock: 1000,
                basis_heading: h[0],
                target_heading: h[1],
                turn: h[2],
                target_point: Some(h[3..].try_into().unwrap()),
            },
            head_look::rig::Rig {
                head: None,
                hand: None,
                pelvis: None,
                forward: None,
                turn_bone: None,
            },
            Matrix::IDENTITY.values,
            pose::transform::Placement {
                base: decode(&b["base"]),
                offset: decode(&b["offset"]),
                previous: decode(&b["previous"]),
                dirty: false,
            },
            pose::model::Model {
                placement: Matrix::IDENTITY,
                scale: Matrix::IDENTITY,
                bounds_transform: Matrix::IDENTITY,
                bounds_flags: 0,
            },
            Censor {
                binding: None,
                mask: 0,
            },
            1500,
        )
        .unwrap();
        let mut control = pose::ordinary::State {
            clock_mode: 0,
            base_posture: 6,
            pickup: 2,
            carry: sims_vm::animation::CarrySelector {
                mode: 1,
                selection: 14,
            },
            mode: 5,
            rate: 1.7,
            target_rate: 2.1,
        };
        let mut movement = pose::movement::Movement::new(pose::movement::State {
            status: decode(&row["status"]),
            flags: [3, 4],
            start: [1., 3., 5.],
            end: [2., 4., 6.],
            turn_delta: 0.25,
            nominal_turn: 0.75,
            reason: 5,
            apply_rate: 1,
        });
        movement.path = Some(pose::movement::path::Path {
            points: vec![[10, 20], [30, 40], [31, 40]],
            settings: pose::movement::path::Settings {
                sample_step: 1.,
                turn_threshold: 2.,
                turn_step: 0.2,
                minimum_brake_time: 100,
                endpoint_distance: [2., 3.],
            },
            index: 4,
            progress: 0.25,
            deadline: 999,
            phase_window: [0.25, 0.75],
            stop_distance: [1., 2.],
            start_distance: [3., 4.],
            lookahead: [5., 6.],
            interrupt: 8,
            guard: 2,
            delta: [0.1, 0.2],
            stop_mode: 17,
        });
        let old_path = movement.path.clone().unwrap();
        let bones = character.skin.bones.clone();
        let head = character.head.clone();
        let effects =
            pose::reset::apply(&mut character, &mut control, &mut movement, 1600, settings)
                .unwrap();
        assert_eq!(character.head, head);
        assert_eq!(character.skin.bones, bones);
        assert_eq!(
            character.clock, 1500,
            "Reset obtains a fade clock without replacing retained animator clock"
        );
        assert!(character.skin.animator.pop_numbered_event().is_none());
        let mut expected = row["original"].clone();
        let scalar = expected.as_object_mut().unwrap().remove("scalar").unwrap();
        let diagnostic = expected
            .as_object_mut()
            .unwrap()
            .remove("global_diagnostic")
            .unwrap();
        let released = expected
            .as_object_mut()
            .unwrap()
            .remove("released")
            .unwrap();
        expected.as_object_mut().unwrap().remove("bone_priority");
        let actual = json!(character.skin.animator);
        ownership::compare(&actual, &expected, "animator reset");
        assert_eq!(diagnostic, json!(effects.route_diagnostic));
        assert_eq!(
            scalar["geometry_generation"],
            json!(u32::MAX.wrapping_add(effects.geometry_generation_delta))
        );
        assert!(effects.clear_movement_anchors);
        let path = movement.path.as_ref().unwrap();
        for (key, value) in [
            ("path_index", path.index),
            ("path_deadline", path.deadline),
            ("path_guard", path.guard),
            ("path_interrupt", path.interrupt),
            ("path_stop_mode", path.stop_mode),
        ] {
            assert_eq!(scalar[key], json!(value), "{key}");
        }
        let mut retained = path.clone();
        retained.index = old_path.index;
        retained.deadline = old_path.deadline;
        retained.guard = old_path.guard;
        retained.interrupt = old_path.interrupt;
        retained.stop_mode = old_path.stop_mode;
        assert_eq!(
            retained, old_path,
            "Reset changed retained path samples or calibration"
        );
        let scalar_actual = json!({"base":character.placement.base,"offset":character.placement.offset,
            "previous":character.placement.previous,"mode":control.mode,"base_posture":control.base_posture,
            "pickup":control.pickup,"carry_mode":control.carry.mode,"carry_selection":control.carry.selection,
            "rate":control.rate,"target_rate":control.target_rate,"status":movement.state.status,"flags":movement.state.flags,
            "start":movement.state.start,"end":movement.state.end,"turn_delta":movement.state.turn_delta,
            "nominal_turn":movement.state.nominal_turn,"reason":movement.state.reason,"apply_rate":movement.state.apply_rate});
        for (key, value) in scalar_actual.as_object().unwrap() {
            ownership::compare(value, &scalar[key], key);
        }
        assert!(character.placement.dirty);
        for id in released.as_array().unwrap() {
            assert!(character.skin.animator.layer(decode(id)).is_none());
        }
        let restored: Character = decode(&json!(character));
        assert_eq!(restored, character);
        // The surviving fades must retire through the actual skin update.
        character.skin.begin(1800, true, false, true).unwrap();
        let mut person = Default::default();
        let context = pose::properties::Context {
            actor: 1,
            sound_visible: false,
            selected: false,
            debug_sound: false,
            position_x: 0.,
            appearance_mode: 0,
            snow: false,
            lot_kind: 1,
        };
        assert!(
            character
                .skin
                .next(&program, &mut person, context)
                .unwrap()
                .is_none()
        );
        assert!(character.skin.animator.layers().is_empty());
    }
    assert_eq!(rows["cases"].as_array().unwrap().len(), 42);
}
