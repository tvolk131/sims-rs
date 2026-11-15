//! Head tracking during a frame draw (0x5924a4, 0x5a199b): which anchor
//! the head is measured from. Synthetic; no game data.

use crate::support::*;
use sims_content::{Bone, Clip, Profile, Skeleton};
use sims_core::F32;
use sims_sim::animator::{Animator, Base, LOADED, Layer};

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

/// [`anchored`] with a skeleton of a root, the head under it and an arm
/// beside the head, and a carry layer at half weight playing a clip that
/// moves `carried`.
fn carrying(carried: &str) -> Fixture {
    let mut fx = anchored(Some("Top"));
    let bone = |name: &str, parent| Bone {
        name: name.into(),
        parent,
        translation: [0; 3],
        rotation: [0, 0, 0, 1f32.to_bits()],
    };
    fx.program
        .people
        .skeletons
        .get_mut(&SIM_GUID)
        .unwrap()
        .bones = vec![
        bone("ROOT", None),
        bone("HEAD", Some(0)),
        bone("ARM", Some(0)),
    ];
    fx.program.animations.insert(
        "test",
        Clip {
            name: "carry".into(),
            skill: "skill".into(),
            duration: 100f32.to_bits(),
            distance: 0,
            moving: false,
            frames: 1,
            motions: 1,
            tracks: vec![],
            channels: vec![sims_content::Channel {
                bone: carried.into(),
                translation: None,
                rotation: Some(0),
            }],
            samples: [0, 0],
        },
    );
    let animator = fx.world.animators.get_mut(&SIM).unwrap();
    let mut layer = Layer::start("carry", F32::from_f32(100.), 1, true, 0, false, 50);
    layer.priority = 10;
    layer.weight = F32::from_f32(0.5);
    animator.carry = Some(layer);
    animator.carry_selection = 0;
    fx
}

fn draw(mut fx: Fixture) -> Result<F32, sims_vm::Fault> {
    let vm = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    vm.draw_frame(&mut fx.world, 1)?;
    Ok(fx.world.animators[&SIM].head.heading)
}

#[test]
fn a_carry_layer_that_moves_only_the_arm_is_passed_over() {
    let heading = draw(carrying("ARM")).unwrap();
    let mut alone = carrying("ARM");
    let animator = alone.world.animators.get_mut(&SIM).unwrap();
    animator.carry = None;
    animator.carry_selection = -1;
    assert_eq!(heading, draw(alone).unwrap());
}

#[test]
fn a_carry_layer_that_moves_the_head_or_its_root_fails_loudly() {
    for bone in ["HEAD", "ROOT"] {
        let fault = draw(carrying(bone)).unwrap_err();
        assert!(
            matches!(&fault.trap, sims_vm::Trap::Unsupported(m) if m.contains("carry clip")),
            "{bone}: {fault}"
        );
    }
}

/// [`carrying`] with no carry layer and, in the base slot, a clip a script
/// put there (0x58957e) that moves `moved`.
fn based(moved: &str) -> Fixture {
    let mut fx = carrying("ARM");
    fx.program.animations.insert(
        "test",
        Clip {
            name: "base".into(),
            skill: "skill".into(),
            duration: 100f32.to_bits(),
            distance: 0,
            moving: false,
            frames: 1,
            motions: 1,
            tracks: vec![],
            channels: vec![sims_content::Channel {
                bone: moved.into(),
                translation: None,
                rotation: Some(0),
            }],
            samples: [0, 0],
        },
    );
    let animator = fx.world.animators.get_mut(&SIM).unwrap();
    animator.carry = None;
    animator.carry_selection = -1;
    animator.base = Base::Script {
        clip: "base".into(),
        reverse: false,
        start: 0,
    };
    fx
}

#[test]
fn a_scripts_base_clip_is_checked_against_the_foreground() {
    // A base clip that moves only the arm leaves the head as the
    // foreground put it.
    let heading = draw(based("ARM")).unwrap();
    let mut alone = based("ARM");
    alone.world.animators.get_mut(&SIM).unwrap().base = Base::Empty;
    assert_eq!(heading, draw(alone).unwrap());
    // One that moves the head, which the foreground leaves alone, would
    // show through, and fails loudly naming the script's clip.
    let fault = draw(based("HEAD")).unwrap_err();
    assert!(
        matches!(&fault.trap, sims_vm::Trap::Unsupported(m) if m.contains("base clip \"base\"")),
        "{fault}"
    );
}

#[test]
fn a_head_looking_at_a_person_queued_for_removal_fails_loudly() {
    // The person left the lot without its animator's refresh, which the
    // original makes only if the view drew it (0x58da12).
    const OTHER: u16 = 77;
    let mut fx = anchored(Some("Top"));
    let mut other = fx.world.objects[&SIM].clone();
    other.id = OTHER;
    fx.world.objects.insert(OTHER, other);
    let animator = fx.world.animators[&SIM].clone();
    fx.world.animators.insert(OTHER, animator);
    fx.sim_mut().person.as_mut().unwrap().data[41] = OTHER as i16;
    fx.world.removals.push(OTHER);
    let fault = draw(fx).unwrap_err();
    assert!(
        matches!(&fault.trap, sims_vm::Trap::Unsupported(m) if m.contains("queued for removal")),
        "{fault}"
    );
}

#[test]
fn a_head_looking_at_an_object_aims_at_its_position_on_the_floor() {
    // The lamp is not a person: the target is its position in world units
    // (times 3/16), the second lot coordinate last, at height 0 (0x59058f,
    // 0x454f5c).
    let mut fx = anchored(Some("Top"));
    fx.lamp_mut().position = [160, 320, 1];
    fx.sim_mut().person.as_mut().unwrap().data[41] = LAMP as i16;
    let vm = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    vm.draw_frame(&mut fx.world, 1).unwrap();
    let target = fx.world.animators[&SIM].head.target.unwrap();
    assert_eq!(target.map(F32::to_f32), [30.0, 0.0, 60.0]);
    assert_eq!(target[1].to_bits(), 0, "positive zero");
}
