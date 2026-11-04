use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_content::{Index, saved_layers, saves::House};
use sims_sim::pose::{LocalBone, layer::Layer, sampling::BoneInput};

/// Explicit input fixture for the read service. Native rig initialization is
/// tested separately; this fixture must not be used to activate a household.
fn owner(
    skeleton: &sims_formats::avatar::Skeleton,
    profile: &[String],
    id: i16,
) -> Result<sims_sim::character::Resident> {
    use pose::{
        animator::Settings,
        model::{Matrix, Model},
        refresh::{Censor, Character},
    };
    use sims_sim::{
        character::{Controls, Resident},
        head_look, pose,
    };
    let mut crt = 0x51a5cafe;
    let mut cloned = pose::skeleton::Instance::clone_prototype(skeleton, &mut crt)
        .map_err(anyhow::Error::msg)?;
    cloned
        .hierarchy
        .update(&mut cloned.skin, false)
        .map_err(anyhow::Error::msg)?;
    let character = Character::new(
        cloned.skin,
        cloned.hierarchy,
        head_look::State {
            last_clock: 19,
            basis_heading: 1.,
            target_heading: 2.,
            turn: 0.3,
            target_point: Some([4., 5., 6.]),
        },
        head_look::rig::Rig::bind(skeleton, profile).map_err(anyhow::Error::msg)?,
        Matrix::IDENTITY.values,
        pose::construction::State::new(2).placement,
        Model {
            placement: Matrix::IDENTITY,
            scale: Matrix::IDENTITY,
            bounds_transform: Matrix::IDENTITY,
            bounds_flags: 0,
        },
        Censor {
            binding: None,
            mask: 0,
        },
        100,
    )
    .map_err(anyhow::Error::msg)?;
    Ok(Resident::new(
        character,
        2,
        Controls {
            ordinary: pose::ordinary::State {
                clock_mode: 0,
                base_posture: -1,
                pickup: 0,
                carry: sims_vm::animation::CarrySelector {
                    mode: 0,
                    selection: -1,
                },
                mode: 3,
                rate: 1.,
                target_rate: 1.,
            },
            movement: pose::movement::Movement::new(pose::construction::State::new(2).movement),
            settings: Settings {
                fade_enabled: true,
                fade_duration: 200,
                sampling_config: 1,
            },
            movement_anchors: [None; 2],
            geometry_generation: 0,
            animation_events: 0,
            engaged: 0,
        },
        pose::properties::Context {
            actor: id,
            sound_visible: false,
            selected: false,
            debug_sound: false,
            position_x: 0.,
            appearance_mode: 0,
            snow: false,
            lot_kind: 1,
        },
    ))
}

#[test]
#[ignore = "requires private original assets and native saved-layer reports"]
fn original_saved_layers_bind_parse_and_compose_like_native() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let descriptors: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-layer-descriptors.json"),
    )?)?;
    let poses: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-layer-poses.json"),
    )?)?;
    let mut program = sims_vm::Program::default();
    let mut residents = 0;
    let mut count = 0;
    let mut rig_inputs = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = House::load(&index, source)?;
        for resident in &house.household.as_ref().context("household")?.residents {
            residents += 1;
            let saved = house.objects.instances[&resident.object]
                .person
                .as_ref()
                .context("person")?;
            let avatar =
                sims_content::saved_avatar::load(&index, &resident.body_strings[0], saved)?;
            rig_inputs.push(serde_json::json!({"source":source,"object":resident.object,
                "skeleton":avatar.skeleton,"crt_state":0x51a5cafe_u32}));
            let mut layers =
                saved_layers::load(&index, &mut program, &avatar.skeleton, saved, 123456, 1)?;
            let mut owner = owner(
                &avatar.skeleton,
                &resident.body_strings,
                resident.object as i16,
            )?;
            saved_layers::restore_character(
                &index,
                &mut program,
                &avatar.skeleton,
                saved,
                &mut owner,
                123456,
            )?;
            ensure!(
                owner.character.clock == 100 && owner.character.head.last_clock == 19,
                "read service replaced retained clocks"
            );
            ensure!(
                owner.controls.animation_events == saved.animation_events
                    && owner.controls.engaged == saved.engaged
            );
            ensure!(owner.character.placement.offset == saved.floats[6..9]);
            ensure!(
                owner.character.head.basis_heading == saved.floats[0]
                    && owner.character.head.target_heading == saved.floats[1]
                    && owner.character.head.turn == saved.floats[2]
                    && owner.character.head.target_point == Some(saved.floats[3..6].try_into()?)
            );
            for (slot, expected) in [
                sims_sim::pose::animator::Slot::Foreground,
                sims_sim::pose::animator::Slot::Carry,
                sims_sim::pose::animator::Slot::Base,
            ]
            .into_iter()
            .zip(&layers)
            {
                assert_eq!(
                    owner.character.skin.animator.slot(slot).map(|id| owner
                        .character
                        .skin
                        .animator
                        .layer(id)
                        .unwrap()),
                    expected.as_ref()
                );
            }
            let restored: sims_sim::character::Resident =
                serde_json::from_slice(&serde_json::to_vec(&owner)?)?;
            ensure!(restored == owner);
            ensure!(
                owner.reset(123456).is_err(),
                "Reset must wait for the read tail"
            );
            ensure!(
                restored == owner,
                "rejected reset changed pending read state"
            );
            for (slot, l) in layers.iter().enumerate() {
                if let Some(l) = l {
                    count += 1;
                    let row = descriptors
                        .iter()
                        .find(|r| {
                            r["input"]["source"] == source
                                && r["input"]["object"] == resident.object
                                && r["input"]["slot"] == slot
                        })
                        .context("native descriptor")?;
                    let original = &row["original"];
                    ensure!(
                        original["events"]
                            == serde_json::json!(["clear", "lookup", "clock", "create"])
                    );
                    assert_eq!(l.name, original["parameters"]["name"]);
                    assert_eq!(
                        l.priority as i64,
                        original["parameters"]["priority"].as_i64().unwrap()
                    );
                    for (a, b) in [
                        (l.clock.speed, &original["parameters"]["speed"]),
                        (l.clock.weight, &original["parameters"]["weight"]),
                        (l.clock.phase, &original["layer"]["phase"]),
                    ] {
                        assert_eq!(a.to_bits(), (b.as_f64().unwrap() as f32).to_bits());
                    }
                    assert_eq!(
                        l.clock.mode as i64,
                        original["layer"]["mode"].as_i64().unwrap()
                    );
                    assert_eq!(
                        l.translation_policy as i64,
                        original["layer"]["translation_policy"].as_i64().unwrap()
                    );
                    assert_eq!(
                        l.rotation_policy as i64,
                        original["layer"]["rotation_policy"].as_i64().unwrap()
                    );
                    ensure!(
                        l.clock.current == 123456
                            && l.clock.previous == 123456
                            && l.clock.event_time == 0
                            && l.clock.previous_event_time == 0
                    );
                    ensure!(
                        l.bindings
                            .iter()
                            .all(|b| b.event_cursor.is_none_or(|i| i == 0))
                    );
                }
            }
            let original = poses
                .iter()
                .find(|r| r["source"] == source && r["object"] == resident.object)
                .context("native pose")?;
            let mut sorted = layers
                .iter_mut()
                .filter_map(Option::take)
                .collect::<Vec<_>>();
            let mut priorities = vec![0; avatar.skeleton.bones.len()];
            sims_sim::pose::layer::prepare(&mut sorted, &mut priorities)
                .map_err(anyhow::Error::msg)?;
            let mut bones = avatar
                .skeleton
                .bones
                .iter()
                .zip(priorities)
                .map(|(b, priority)| BoneInput {
                    local: LocalBone {
                        translation: b.translation,
                        rotation: b.rotation,
                    },
                    priority,
                    blend_marker: b.flags[2],
                })
                .collect::<Vec<_>>();
            ensure!(sorted.len() == original["steps"].as_array().unwrap().len());
            for (l, step) in sorted.iter().zip(original["steps"].as_array().unwrap()) {
                let restored: Layer = serde_json::from_slice(&serde_json::to_vec(l)?)?;
                let a = &program.animations[&restored.name];
                restored
                    .sample(&a.clip, &a.samples, &mut bones, 0.)
                    .map_err(anyhow::Error::msg)?;
                for (i, (b, expected)) in bones
                    .iter()
                    .zip(step["original"]["bones"].as_array().unwrap())
                    .enumerate()
                {
                    let t: [f32; 3] = serde_json::from_value(expected["translation"].clone())?;
                    let q: [f32; 4] = serde_json::from_value(expected["rotation"].clone())?;
                    for (a, b) in b.local.translation.into_iter().zip(t) {
                        ensure!((a - b).abs() < 2e-6, "translation {}, bone {i}", l.name);
                    }
                    for (a, b) in b.local.rotation.into_iter().zip(q) {
                        ensure!((a - b).abs() < 1e-6, "rotation {}, bone {i}", l.name);
                    }
                }
            }
        }
    }
    ensure!(residents == 7 && count == 11);
    std::fs::write(
        root.join("cache/original-skeleton-clones-input.json"),
        serde_json::to_vec(&rig_inputs)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires private original skeleton clone probe report"]
fn original_resident_skeleton_clones_match_native() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-skeleton-clones.json"),
    )?)?;
    for row in report["cases"].as_array().context("clone cases")? {
        let skeleton: sims_formats::avatar::Skeleton =
            serde_json::from_value(row["input"]["skeleton"].clone())?;
        let mut crt = row["input"]["crt_state"].as_u64().unwrap() as u32;
        let mut rig = sims_sim::pose::skeleton::Instance::clone_prototype(&skeleton, &mut crt)
            .map_err(anyhow::Error::msg)?;
        let expected = &row["original"];
        assert_eq!(serde_json::json!(crt), expected["crt_state"]);
        for (i, bone) in expected["bones"].as_array().unwrap().iter().enumerate() {
            assert_eq!(serde_json::json!(rig.hierarchy.updates[i]), bone["update"]);
            assert_eq!(
                serde_json::json!(rig.geometry.bone_generations[i]),
                bone["geometry_stamp"]
            );
            assert_eq!(serde_json::json!(rig.wiggle[i]), bone["wiggle"]);
            assert_eq!(serde_json::json!(rig.flags[i]), bone["flags"]);
            assert_eq!(
                serde_json::json!(rig.named_bone_flags[i]),
                bone["named_bone"]
            );
        }
        rig.hierarchy
            .update(&mut rig.skin, false)
            .map_err(anyhow::Error::msg)?;
        assert_eq!(
            serde_json::json!(rig.hierarchy.updates),
            expected["propagated"]["updates"]
        );
        for (i, (bone, expected)) in rig
            .hierarchy
            .absolute
            .iter()
            .zip(expected["propagated"]["absolute"].as_array().unwrap())
            .enumerate()
        {
            let translation: [f32; 3] = serde_json::from_value(expected["translation"].clone())?;
            let rotation: [f32; 4] = serde_json::from_value(expected["rotation"].clone())?;
            for (actual, expected) in bone
                .translation
                .into_iter()
                .zip(translation)
                .chain(bone.rotation.into_iter().zip(rotation))
            {
                ensure!(
                    (actual - expected).abs() < 2e-6,
                    "original clone bone {i}: {actual} != {expected}"
                );
            }
        }
    }
    ensure!(report["cases"].as_array().unwrap().len() == 7);
    Ok(())
}
