use serde_json::{Value, json};
use sims_formats::avatar::{Bone, Skeleton, Wiggle};
use sims_sim::pose::skeleton::Instance;

#[cfg(feature = "private-fixtures")]
#[test]
fn original_skeleton_clone_preserves_pose_stamps_and_process_rng() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-skeleton-clone.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let prototype: Skeleton = serde_json::from_value(row["input"]["skeleton"].clone()).unwrap();
        let mut crt = row["input"]["crt_state"].as_u64().unwrap() as u32;
        let before = prototype.clone();
        let mut rig = Instance::clone_prototype(&prototype, &mut crt).unwrap();
        assert_eq!(before, prototype);
        let original = &row["original"];
        // 598af7's resource reader and 598da3's clone both use the same bone
        // constructor/scale setter. Check their retained noise against those
        // native numeric vectors; resource hierarchy stamps are not asserted.
        let mut resource_crt = row["input"]["crt_state"].as_u64().unwrap() as u32;
        let resource =
            sims_sim::pose::skeleton::Resource::read(prototype.clone(), &mut resource_crt).unwrap();
        assert_eq!(json!(resource_crt), original["crt_state"]);
        assert_eq!(
            json!(resource.wiggle),
            json!(
                original["bones"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| &b["wiggle"])
                    .collect::<Vec<_>>()
            )
        );
        assert_eq!(json!(crt), original["crt_state"]);
        for (i, b) in original["bones"].as_array().unwrap().iter().enumerate() {
            assert_eq!(json!(rig.hierarchy.updates[i]), b["update"]);
            assert_eq!(json!(rig.hierarchy.parents[i]), b["parent"]);
            assert_eq!(json!(rig.geometry.bone_generations[i]), b["geometry_stamp"]);
            let mut pose = rig.skin.bones[i].local.translation.to_vec();
            pose.extend(rig.skin.bones[i].local.rotation);
            assert_eq!(json!(pose), b["local"]);
            assert_eq!(json!(rig.skin.bones[i].blend_marker), b["flags"][2]);
            assert_eq!(json!(rig.flags[i]), b["flags"]);
            assert_eq!(json!(rig.named_bone_flags[i]), b["named_bone"]);
            assert_eq!(
                json!(rig.wiggle[i]),
                b["wiggle"],
                "{} bone {i}",
                row["input"]
            );
        }
        let state = json!(rig.skin.animator);
        for (key, offset) in [
            ("dirty", "0x94"),
            ("generation", "0xb4"),
            ("pose_generation", "0xa4"),
            ("last_clock", "0xb8"),
        ] {
            match key {
                "dirty" => assert_eq!(state[key], json!(original["skin"][offset] == 1)),
                "last_clock" => assert_eq!(
                    state[key],
                    json!(original["skin"][offset].as_u64().unwrap() as u32 as i32)
                ),
                _ => assert_eq!(state[key], original["skin"][offset]),
            }
        }
        rig = serde_json::from_slice(&serde_json::to_vec(&rig).unwrap()).unwrap();
        rig.hierarchy.update(&mut rig.skin, false).unwrap();
        assert_eq!(
            json!(rig.hierarchy.absolute),
            original["propagated"]["absolute"]
        );
        assert_eq!(
            json!(rig.hierarchy.updates),
            original["propagated"]["updates"]
        );
        assert_eq!(
            json!(rig.hierarchy.evaluated_generation),
            original["propagated"]["evaluated_generation"]
        );
        let state = json!(rig.skin.animator);
        for key in ["generation", "pose_generation"] {
            assert_eq!(state[key], original["propagated"][key]);
        }
        let before = rig.clone();
        rig.hierarchy.update(&mut rig.skin, false).unwrap();
        assert_eq!(rig, before, "unchanged hierarchy should use its cache");
        let mut unsupported = prototype.clone();
        unsupported.bones[0].wiggle.amplitude = f32::NAN;
        let seed = crt;
        assert!(Instance::clone_prototype(&unsupported, &mut crt).is_err());
        assert_eq!(
            crt, seed,
            "rejected rig must not partially consume process state"
        );
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 52);
    let prototype = Skeleton {
        name: "synthetic-crt-sequence".into(),
        bones: (0usize..29)
            .map(|index| Bone {
                name: format!("bone-{index}"),
                parent: index.checked_sub(1),
                translation: [0.; 3],
                rotation: [0., 0., 0., 1.],
                properties: vec![],
                flags: [1; 3],
                wiggle: Wiggle {
                    gradient_scale: 2,
                    amplitude: 0.,
                },
            })
            .collect(),
    };
    assert_eq!(prototype.bones.len(), 29);
    assert_eq!(
        prototype
            .bones
            .iter()
            .map(|bone| 32
                + if bone.wiggle.gradient_scale == 1 {
                    0
                } else {
                    16
                })
            .sum::<usize>(),
        1392
    );
    for (markers, loaded, before_traits, after_traits) in [
        (0, 3504597470, 3600189262, [4281969421, 3939946756]),
        (2, 3031781918, 317955982, [3110177490, 1750507675]),
    ] {
        let mut crt = 0x51a5cafe;
        for _ in 0..2 {
            Instance::clone_prototype(&prototype, &mut crt).unwrap();
        }
        assert_eq!(crt, 3504597470);
        for _ in 0..markers {
            Instance::root(&mut crt);
        }
        assert_eq!(crt, loaded, "{markers} load markers");
        Instance::clone_prototype(&prototype, &mut crt).unwrap();
        assert_eq!(crt, before_traits, "{markers} load markers, third rig");
        for (iteration, expected_state) in after_traits.into_iter().enumerate() {
            let mut person: std::collections::BTreeMap<_, _> = (0..256)
                .map(|field| {
                    (
                        field as i16,
                        (((field * 257 + iteration * 31 + 12345) % 65536) as i32 - 32768) as i16,
                    )
                })
                .collect();
            sims_vm::person_factory::default_traits(&mut person, &mut crt).unwrap();
            assert_eq!(
                crt, expected_state,
                "{markers} load markers, trait {iteration}"
            );
        }
    }
}
