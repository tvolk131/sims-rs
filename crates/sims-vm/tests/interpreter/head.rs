//! Head tracking during a frame draw (0x5924a4, 0x5a199b): which anchor
//! the head is measured from. Synthetic; no game data.

use crate::support::*;
use sims_content::{Bone, Clip, Profile, Skeleton};
use sims_core::F32;
use sims_sim::animator::{Animator, LOADED, Layer};

/// A person with one bone, "HEAD", whose rig names `point` for the top
/// anchor, playing a one-frame clip, with a head target off to the side.
fn anchored(point: Option<&str>) -> Fixture {
    let mut fx = Fixture::new();
    fx.world.size = 64;
    fx.words(SIM_FILE)[101] = 0;
    fx.words(SIM_FILE)[102] = 0;
    fx.sim_mut().position = [100, 200, 1];
    fx.sim_mut().person.as_mut().unwrap().data[41..46].copy_from_slice(&[0, 1, 0, 0, 0]);
    fx.program.people.profiles.insert(
        SIM_GUID,
        Profile {
            rig: [Some("HEAD".into()), point.map(String::from), None, None],
            ..Default::default()
        },
    );
    fx.program.people.skeletons.insert(
        SIM_GUID,
        Skeleton {
            name: "test".into(),
            file: "test".into(),
            place: 0,
            scales: vec![vec![0]],
            bones: vec![Bone {
                name: "HEAD".into(),
                parent: None,
                translation: [0; 3],
                rotation: [0, 0, 0, 1f32.to_bits()],
            }],
        },
    );
    fx.program.animations.insert(
        "test",
        Clip {
            name: "clip".into(),
            skill: "skill".into(),
            duration: 100f32.to_bits(),
            distance: 0,
            moving: false,
            frames: 1,
            motions: 1,
            tracks: vec![],
            channels: vec![],
            samples: [0, 0],
        },
    );
    fx.program.animations.insert_sample_file("skill", vec![]);
    fx.world.arrays.insert(
        0,
        sims_formats::lot::Array {
            width: 64,
            height: 64,
            stride: 4,
            encoding: 0,
            data: vec![0; 64 * 64 * 4],
        },
    );
    let mut animator = Animator {
        clock: 50,
        head_clock: 50,
        skin_clock: 50,
        undrawn: false,
        profile_stale: false,
        position: [F32::from_f32(100.), F32::from_f32(200.), F32::from_f32(2.)],
        foreground: Some(Layer::start(
            "clip",
            F32::from_f32(100.),
            1,
            true,
            0,
            false,
            50,
        )),
        ..LOADED
    };
    animator.head.target = Some([F32::from_f32(100.); 3]);
    fx.world.animators.insert(SIM, animator);
    fx
}

/// The heading the head turns to after one frame draw.
fn heading(point: Option<&str>) -> F32 {
    let mut fx = anchored(point);
    let vm = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    vm.draw_frame(&mut fx.world, 1).unwrap();
    fx.world.animators[&SIM].head.heading
}

#[test]
fn a_named_point_measures_the_head_from_that_anchor() {
    assert_ne!(heading(Some("Top")), F32::ZERO);
}

#[test]
fn an_empty_point_measures_the_head_from_the_bones_origin() {
    assert_ne!(heading(Some("")), F32::ZERO);
    assert_eq!(heading(Some("")), heading(None));
}

#[test]
fn an_absent_point_measures_the_head_from_the_bones_origin() {
    assert_ne!(heading(None), F32::ZERO);
}
