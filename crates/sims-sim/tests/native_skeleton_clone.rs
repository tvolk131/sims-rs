use serde_json::{Value, json};
use sims_formats::avatar::Skeleton;
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
}
