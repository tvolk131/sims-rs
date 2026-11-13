use glam::{Mat4, Vec3};
use sims_formats::avatar::{Blend, Bone, Mesh, MeshBinding, Skeleton, Vertex as MeshVertex};
use sims_render::{Draw, Scene, Texture, Vertex, pose, software};
use std::sync::Arc;
#[test]
fn sprite_depth_interleaves_with_geometry_and_transparent_texels_do_not_occlude() {
    let textures = Arc::new(vec![
        Texture {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 255, 255].into(),
            depth: vec![255].into(),
        },
        Texture {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 255, 0, 0, 255].into(),
            depth: vec![128, 255].into(),
        },
    ]);
    let quad = |z, scale| {
        let v = [
            ([-1., 1., z], [0., 0.]),
            ([1., 1., z], [1., 0.]),
            ([1., -1., z], [1., 1.]),
            ([-1., -1., z], [0., 1.]),
        ];
        [0, 1, 2, 0, 2, 3].map(|i| Vertex {
            position: v[i].0,
            uv: v[i].1,
            tint: [1.; 4],
            depth_scale: scale,
        })
    };
    let mut vertices = quad(0.5, 0.).to_vec();
    vertices.extend(quad(0.6, -0.4));
    let mut scene = Scene {
        censors: vec![],
        width: 8,
        height: 4,
        vertices,
        draws: vec![
            Draw {
                object: Some(2),
                vertices: 0..6,
                texture: 0,
            },
            Draw {
                object: None,
                vertices: 6..12,
                texture: 1,
            },
        ],
        textures,
    };
    let a = software::render(&scene);
    assert_eq!(a.get_pixel(1, 1).0, [255, 0, 0, 255]);
    assert_eq!(a.get_pixel(6, 1).0, [0, 0, 255, 255]);
    assert_eq!(
        scene.pick([1.5, 1.5]),
        None,
        "nearer geometry must occlude picking"
    );
    assert_eq!(scene.pick([6.5, 1.5]), Some(2));
    scene.draws.reverse();
    assert_eq!(software::render(&scene), a);
    let retained = scene.clone();
    let mut textures = (*scene.textures).clone();
    assert!(Arc::ptr_eq(&textures[1].rgba, &retained.textures[1].rgba));
    assert!(Arc::ptr_eq(&textures[1].depth, &retained.textures[1].depth));
    Arc::make_mut(&mut textures[1].rgba)[3] = 0;
    assert!(!Arc::ptr_eq(&textures[1].rgba, &retained.textures[1].rgba));
    assert!(Arc::ptr_eq(&textures[1].depth, &retained.textures[1].depth));
    scene.textures = Arc::new(textures);
    assert_eq!(software::render(&scene).get_pixel(1, 1).0, [0, 0, 255, 255]);
    assert_eq!(
        scene.pick([1.5, 1.5]),
        Some(2),
        "transparent pixels must not occlude picking"
    );
    assert_eq!(
        software::render(&retained),
        a,
        "alpha mutation changed a retained scene"
    );
    assert_eq!(retained.pick([1.5, 1.5]), None);
    let mut depth_changed = retained.clone();
    Arc::make_mut(&mut Arc::make_mut(&mut depth_changed.textures)[1].depth)[0] = 255;
    assert_eq!(
        software::render(&depth_changed).get_pixel(1, 1).0,
        [0, 0, 255, 255]
    );
    assert_eq!(depth_changed.pick([1.5, 1.5]), Some(2));
    assert_eq!(
        software::render(&retained),
        a,
        "depth mutation changed a retained scene"
    );
    assert_eq!(retained.pick([1.5, 1.5]), None);
    assert_eq!(scene.pick([-1., 1.]), None);
    assert_eq!(scene.pick([f32::NAN, 1.]), None);
}
fn bone(name: &str, parent: Option<usize>) -> Bone {
    Bone {
        name: name.into(),
        parent,
        translation: [0.; 3],
        rotation: [0., 0., 0., -1.],
        properties: vec![],
        flags: [1, 1, 0],
        wiggle: Default::default(),
    }
}
#[test]
fn original_pose_sampling_uses_native_blending_exact_names_and_hierarchy() {
    use sims_formats::animation::{Animation, Motion, Samples};
    use sims_vm::program::AnimationAsset;
    let mut skeleton = Skeleton {
        name: "test".into(),
        bones: vec![bone("ROOT", None), bone("CHILD", Some(0))],
    };
    skeleton.bones[1].translation = [4., 0., 0.];
    let mut asset = AnimationAsset {
        source: String::new(),
        content_hash: String::new(),
        samples_source: String::new(),
        samples_hash: String::new(),
        clip: Animation {
            name: "test".into(),
            skill: "test".into(),
            duration_ms: 1000.,
            distance: 0.,
            moving: false,
            translation_count: 0,
            rotation_count: 2,
            frames: 2,
            motions: vec![Motion {
                bone: "ROOT".into(),
                frames: 2,
                duration_ms: 1000.,
                translation: None,
                rotation: Some(0),
                properties: vec![],
                event_lists: vec![],
            }],
            events: vec![],
        },
        samples: Samples {
            translations: vec![],
            rotations: vec![[0., 0., 0., 1.], [0., 0., 1., 0.]],
        },
    };
    let output = pose::evaluate(&skeleton, &asset, 0.25, false).unwrap();
    assert!((output[0].transform_vector3(Vec3::X) - Vec3::new(0.8, 0.6, 0.)).length() < 1e-6);
    assert!((output[1].transform_point3(Vec3::ZERO) - Vec3::new(-3.2, -2.4, 0.)).length() < 2e-6);
    // Retained, non-unit source samples are not normalized by the hierarchy.
    asset.samples.rotations[0] = [0., 0., 0., 0.5];
    let output = pose::evaluate(&skeleton, &asset, 0., false).unwrap();
    assert!((output[1].transform_point3(Vec3::ZERO) - Vec3::new(-1., 0., 0.)).length() < 1e-6);
    // An unmatched spelling preserves the skeleton's rest pose.
    asset.clip.motions[0].bone = "root".into();
    let output = pose::evaluate(&skeleton, &asset, 0., false).unwrap();
    assert!((output[1].transform_point3(Vec3::ZERO) - Vec3::new(-4., 0., 0.)).length() < 1e-6);
    // Integer endpoints supplied by the legacy frame cursor must not acquire a
    // phase-roundtrip error and enter the previous frame's .01 cutoff branch.
    asset.clip.frames = 8;
    asset.clip.motions[0].frames = 8;
    asset.clip.motions[0].bone = "ROOT".into();
    asset.samples.rotations = vec![[0., 0., 0., 1.]; 8];
    asset.samples.rotations[7] = [0., 0., 1., 0.];
    let output = pose::evaluate(&skeleton, &asset, 7., false).unwrap();
    assert!((output[0].transform_vector3(Vec3::X) + Vec3::X).length() < 1e-6);
}
#[test]
fn temporary_head_pose_moves_only_its_subtree_without_accumulation() {
    use glam::Quat;
    let skeleton = Skeleton {
        name: "head test".into(),
        bones: vec![
            bone("ROOT", None),
            bone("SPINE", Some(0)),
            bone("HEAD", Some(1)),
            bone("HAIR", Some(2)),
            bone("R_HAND", Some(1)),
        ],
    };
    let root = Mat4::from_rotation_translation(Quat::from_rotation_y(0.4), Vec3::new(2., 0., 3.));
    let spine = root * Mat4::from_translation(Vec3::Y);
    let head = spine * Mat4::from_rotation_translation(Quat::from_rotation_z(1.2), Vec3::Y);
    let hair_local = Mat4::from_translation(Vec3::new(0., 0.5, 0.2));
    let base = vec![
        root,
        spine,
        head,
        head * hair_local,
        spine * Mat4::from_translation(Vec3::X),
    ];
    let before = base.clone();
    assert_eq!(pose::head_overlay(&skeleton, &base, 0.).unwrap(), base);
    for turn in [-1.5, -0.5, 0.5, 1.5] {
        let got = pose::head_overlay(&skeleton, &base, turn).unwrap();
        assert_eq!(base, before);
        for i in [0, 1, 4] {
            assert_eq!(got[i], base[i]);
        }
        assert_ne!(got[2], base[2]);
        assert_ne!(got[3], base[3]);
        assert!(
            (got[2].transform_point3(Vec3::ZERO) - head.transform_point3(Vec3::ZERO)).length()
                < 1e-6
        );
        assert!((got[2].inverse() * got[3]).abs_diff_eq(hair_local, 1e-6));
        assert_eq!(pose::head_overlay(&skeleton, &base, turn).unwrap(), got);
    }
    assert!(pose::head_overlay(&skeleton, &base, f32::NAN).is_err());
}
#[test]
fn skinning_uses_each_bones_local_vertex_and_blends_after_transform() {
    let skeleton = Skeleton {
        name: "test".into(),
        bones: vec![bone("ROOT", None), bone("CHILD", Some(0))],
    };
    let mesh = Mesh {
        name: "test".into(),
        texture: "test".into(),
        bones: vec!["ROOT".into(), "CHILD".into()],
        bindings: vec![
            MeshBinding {
                bone: 0,
                first_vertex: 0,
                vertex_count: 1,
                first_blend: 0,
                blend_count: 0,
            },
            MeshBinding {
                bone: 1,
                first_vertex: 0,
                vertex_count: 0,
                first_blend: 0,
                blend_count: 1,
            },
        ],
        triangles: vec![],
        vertices: vec![MeshVertex {
            position: [1., 0., 0.],
            normal: [0., 1., 0.],
            uv: [0., 0.],
            bone: 0,
        }],
        blends: vec![Blend {
            vertex: 0,
            weight: 0.25,
            position: [0., 2., 0.],
            normal: [0., 1., 0.],
            bone: 1,
        }],
    };
    let out = pose::skin(
        &mesh,
        &skeleton,
        &[
            Mat4::from_translation(Vec3::new(2., 0., 0.)),
            Mat4::from_translation(Vec3::new(0., 2., 0.)),
        ],
    )
    .unwrap();
    assert!((out[0].0 - Vec3::new(0.75, 1., 0.)).length() < 1e-6);
}
