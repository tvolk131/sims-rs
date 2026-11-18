//! Head tracking during a frame draw (0x5924a4, 0x5a199b): which anchor
//! the head is measured from. Synthetic; no game data.

use crate::support::*;
use sims_content::{Bone, Clip, Profile, Skeleton};
use sims_core::F32;
use sims_sim::animator::{Animator, Base, LOADED, Layer};

/// The rotations every test clip reads, by sample index: the bone's rest
/// rotation (0), and a quarter turn about the vertical axis (1).
const AT_REST: u32 = 0;
const TURNED: u32 = 1;

/// The sample file of the skill "skill": no translations and the two
/// rotations above, each component written whole (CFP code 255).
fn samples() -> Vec<u8> {
    let half = std::f32::consts::FRAC_1_SQRT_2;
    let rotations = [[0., 0., 0., 1.], [0., half, 0., half]];
    let mut bytes = Vec::new();
    for axis in 0..4 {
        for rotation in &rotations {
            bytes.push(255);
            bytes.extend(f32::to_le_bytes(rotation[axis]));
        }
    }
    bytes
}

/// A one-frame clip of the skill "skill" that turns each bone by its
/// sample.
fn clip(name: &str, turns: &[(&str, u32)]) -> Clip {
    Clip {
        name: name.into(),
        skill: "skill".into(),
        duration: 100f32.to_bits(),
        distance: 0,
        moving: false,
        frames: 1,
        motions: 1,
        tracks: vec![],
        channels: turns
            .iter()
            .map(|&(bone, sample)| sims_content::Channel {
                bone: bone.into(),
                translation: None,
                rotation: Some(sample),
            })
            .collect(),
        samples: [0, 2],
    }
}

/// A person with one bone, "HEAD", whose rig names `point` for the top
/// anchor, playing a one-frame clip that holds the head at rest, with a
/// head target off to the side.
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
                blends: true,
            }],
        },
    );
    fx.program
        .animations
        .insert("test", clip("clip", &[("HEAD", AT_REST)]));
    fx.program.animations.insert_sample_file("skill", samples());
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

/// The target heading after one frame draw of [`anchored`] with the
/// person on `level` and the view turned to `rotation`.
fn heading_on(level: i32, rotation: u8) -> Result<F32, sims_vm::Fault> {
    let mut fx = anchored(Some("Top"));
    fx.sim_mut().position[2] = level;
    let mut vm = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    vm.view_rotation = rotation;
    vm.draw_frame(&mut fx.world, 1)?;
    Ok(fx.world.animators[&SIM].head.target_heading)
}

#[test]
fn upstairs_the_head_is_measured_from_the_raised_model() {
    // The rise moves the model a half unit along both floor axes as well
    // as up (0x4ca054), so the heading to the same target changes
    // (2.8269 on the ground, 2.8257 a level up).
    let ground = heading_on(1, 0).unwrap();
    assert_eq!(heading_on(1, 2).unwrap(), ground);
    let upstairs = heading_on(2, 0).unwrap();
    assert_ne!(upstairs, ground);
    assert_ne!(heading_on(3, 0).unwrap(), upstairs);
}

#[test]
fn upstairs_with_the_view_turned_fails_loudly() {
    let fault = heading_on(2, 1).unwrap_err();
    assert!(
        matches!(&fault.trap, sims_vm::Trap::Unsupported(m) if m.contains("0x5c3a20")),
        "{fault}"
    );
}

/// [`anchored`] with a skeleton of a root, the head under it and an arm
/// beside the head, the foreground holding the root and the head at rest,
/// and a carry layer at full weight playing a clip that turns `carried`.
fn carrying(carried: &str) -> Fixture {
    let mut fx = anchored(Some("Top"));
    let bone = |name: &str, parent| Bone {
        name: name.into(),
        parent,
        translation: [0; 3],
        rotation: [0, 0, 0, 1f32.to_bits()],
        blends: true,
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
        clip("clip", &[("ROOT", AT_REST), ("HEAD", AT_REST)]),
    );
    fx.program
        .animations
        .insert("test", clip("carry", &[(carried, TURNED)]));
    let animator = fx.world.animators.get_mut(&SIM).unwrap();
    let mut layer = Layer::start("carry", F32::from_f32(100.), 1, true, 0, false, 50);
    layer.priority = 10;
    animator.carry = Some(layer);
    animator.carry_selection = 0;
    fx
}

fn draw(mut fx: Fixture) -> Result<F32, sims_vm::Fault> {
    let vm = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    vm.draw_frame(&mut fx.world, 1)?;
    Ok(fx.world.animators[&SIM].head.heading)
}

/// `fx` without its carry layer.
fn without_carry(mut fx: Fixture) -> Fixture {
    let animator = fx.world.animators.get_mut(&SIM).unwrap();
    animator.carry = None;
    animator.carry_selection = -1;
    fx
}

#[test]
fn a_carry_layer_that_turns_only_the_arm_leaves_the_head_as_it_was() {
    let heading = draw(carrying("ARM")).unwrap();
    assert_eq!(heading, draw(without_carry(carrying("ARM"))).unwrap());
}

#[test]
fn a_carry_layer_turns_the_head_over_the_foreground() {
    // The skin's update reaches the carry layer (priority 10) after the
    // foreground (priority 0), so its turn of the head or of the root the
    // head hangs from is what the head is measured with (0x5992b3).
    for bone in ["HEAD", "ROOT"] {
        let heading = draw(carrying(bone)).unwrap();
        assert_ne!(
            heading,
            draw(without_carry(carrying(bone))).unwrap(),
            "{bone}"
        );
    }
}

/// [`carrying`] with the carry layer turning the head at `weight`, the
/// head bone blending or not.
fn carrying_head(weight: f32, blends: bool) -> Fixture {
    let mut fx = carrying("HEAD");
    let animator = fx.world.animators.get_mut(&SIM).unwrap();
    animator.carry.as_mut().unwrap().weight = F32::from_f32(weight);
    let skeleton = fx.program.people.skeletons.get_mut(&SIM_GUID).unwrap();
    skeleton.bones[1].blends = blends;
    fx
}

#[test]
fn a_layer_below_full_weight_blends_with_what_the_bones_held() {
    // 0x598226 to 0x5985f9: at weight 0.5 the head's turn is half the
    // carry layer's over the foreground's rest, neither of the two.
    let full = draw(carrying_head(1.0, true)).unwrap();
    let none = draw(without_carry(carrying("HEAD"))).unwrap();
    let half = draw(carrying_head(0.5, true)).unwrap();
    assert_ne!(half, full);
    assert_ne!(half, none);
    // A bone that does not blend (native bone +0x74) takes the layer's
    // sample whatever its weight.
    assert_eq!(draw(carrying_head(0.5, false)).unwrap(), full);
    // Above 1 is whole.
    assert_eq!(draw(carrying_head(2.0, true)).unwrap(), full);
}

#[test]
fn a_bone_no_layer_turns_fails_loudly() {
    // The skin never puts its bones back at rest: a bone no layer playing
    // now turns holds what the last layer that turned it left (0x5992b3).
    let mut fx = anchored(Some("Top"));
    fx.program
        .animations
        .insert("test", clip("clip", &[("ARM", AT_REST)]));
    let fault = draw(fx).unwrap_err();
    assert!(
        matches!(&fault.trap, sims_vm::Trap::Unsupported(m) if m.contains("turned by no layer")),
        "{fault}"
    );
}

#[test]
fn a_bone_no_layer_moves_is_at_rest_until_a_layer_has_moved_it() {
    // A motion with no positions leaves the bone's translation as it was
    // (0x598226, 0x598450). The clip moves no bone: the head is at rest
    // while no layer has ever moved it, and is not known once one has.
    let at_rest = draw(anchored(Some("Top"))).unwrap();
    let mut fx = anchored(Some("Top"));
    fx.world
        .translated_bones
        .insert(SIM, ["ARM".to_owned()].into());
    assert_eq!(draw(fx).unwrap(), at_rest);
    let mut fx = anchored(Some("Top"));
    fx.world
        .translated_bones
        .insert(SIM, ["HEAD".to_owned()].into());
    let fault = draw(fx).unwrap_err();
    assert!(
        matches!(&fault.trap, sims_vm::Trap::Unsupported(m)
            if m.contains("0x598226") && m.contains("\"HEAD\"")),
        "{fault}"
    );
}

/// [`carrying`] with no carry layer, the foreground turning `fore`, and,
/// in the base slot, a clip a script put there (0x58957e) that turns the
/// head.
fn based(fore: &str) -> Fixture {
    let mut fx = without_carry(carrying("ARM"));
    fx.program
        .animations
        .insert("test", clip("clip", &[("ROOT", AT_REST), (fore, AT_REST)]));
    fx.program
        .animations
        .insert("test", clip("base", &[("HEAD", TURNED)]));
    let animator = fx.world.animators.get_mut(&SIM).unwrap();
    animator.base = Base::Script {
        clip: "base".into(),
        reverse: false,
        start: 0,
    };
    let mut layer = Layer::start("base", F32::from_f32(100.), 1, true, 0, false, 50);
    layer.mode = 1;
    layer.priority = -10;
    fx.world.base_layers.insert(SIM, layer);
    fx
}

#[test]
fn the_base_layer_shows_where_the_foreground_does_not_turn() {
    // The base layer (priority -10) comes first: a foreground that turns
    // the head too covers its turn, and one that turns only the arm leaves
    // it showing.
    let covered = draw(based("HEAD")).unwrap();
    let shown = draw(based("ARM")).unwrap();
    let mut rest = based("HEAD");
    rest.world.animators.get_mut(&SIM).unwrap().base = Base::Empty;
    rest.world.base_layers.clear();
    assert_eq!(covered, draw(rest).unwrap());
    assert_ne!(shown, covered);
}

#[test]
fn a_filled_base_slot_with_no_layer_kept_is_a_broken_state() {
    let mut fx = based("HEAD");
    fx.world.base_layers.clear();
    let fault = draw(fx).unwrap_err();
    assert!(
        matches!(&fault.trap, sims_vm::Trap::Invalid(m) if m.contains("base slot")),
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
    // (times 3/16), at height 0 (0x59058f, 0x454f5c). The original passes
    // native +0xf4 first, which is the second lot coordinate, so that comes
    // first and the first lot coordinate last.
    let mut fx = anchored(Some("Top"));
    fx.lamp_mut().position = [160, 320, 1];
    fx.sim_mut().person.as_mut().unwrap().data[41] = LAMP as i16;
    let vm = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    vm.draw_frame(&mut fx.world, 1).unwrap();
    let target = fx.world.animators[&SIM].head.target.unwrap();
    assert_eq!(target.map(F32::to_f32), [60.0, 0.0, 30.0]);
    assert_eq!(target[1].to_bits(), 0, "positive zero");
}

#[test]
fn a_head_looking_at_itself_aims_at_its_own_top_and_keeps_its_target_heading() {
    // 0x59293d with 1 on its own animator: the target is the top anchor
    // the heading is measured from, so the target's heading, from a
    // difference of 0 and 0, stays as it was. The heading is measured as
    // for any target.
    let mut fx = anchored(Some("Top"));
    fx.sim_mut().person.as_mut().unwrap().data[41] = SIM as i16;
    let kept = F32::from_f32(1.25);
    fx.world
        .animators
        .get_mut(&SIM)
        .unwrap()
        .head
        .target_heading = kept;
    let vm = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    vm.draw_frame(&mut fx.world, 1).unwrap();
    let head = &fx.world.animators[&SIM].head;
    assert_eq!(head.target_heading, kept);
    assert_eq!(head.heading, heading(Some("Top")));
    assert_ne!(head.target, Some([F32::from_f32(100.); 3]));
    assert!(head.target.is_some());
    // The person still looks at itself: its target word is kept.
    assert_eq!(fx.sim().person.as_ref().unwrap().data[41], SIM as i16);
}

#[test]
fn a_head_looking_at_itself_queued_for_removal_fails_loudly() {
    let mut fx = anchored(Some("Top"));
    fx.sim_mut().person.as_mut().unwrap().data[41] = SIM as i16;
    fx.world.removals.push(SIM);
    let fault = draw(fx).unwrap_err();
    assert!(
        matches!(&fault.trap, sims_vm::Trap::Unsupported(m)
            if m.contains("looks at itself") && m.contains("0x58da12")),
        "{fault}"
    );
}
