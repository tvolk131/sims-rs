//! The pose of a skeleton by one clip (`sims_sim::pose`), on synthetic
//! values whose results are exact.

use sims_core::F32;
use sims_sim::pose::{self, Bone, Motion, ORIGIN, Selection};

fn f(v: f32) -> F32 {
    F32::from_f32(v)
}

fn bits<const N: usize>(v: [F32; N]) -> [u32; N] {
    v.map(F32::to_bits)
}

#[test]
fn a_phase_falls_between_two_frames() {
    // Four frames over 100 ms: phase 0.5 is frame 2 exactly.
    let at = Selection::at(f(0.5), 4, f(100.0), F32::ZERO, true).unwrap();
    assert_eq!((at.first, at.second, at.interpolate), (2, 3, true));
    assert_eq!((at.first_weight, at.second_weight), (f(1.0), F32::ZERO));
    // Phase 0.9 is 3.6 frames: after the last frame comes the first for a
    // looping clip, the last again for one that does not loop. 3.6 is
    // rounded to single precision before 3 is taken from it.
    let at = Selection::at(f(0.9), 4, f(100.0), F32::ZERO, true).unwrap();
    assert_eq!((at.first, at.second), (3, 0));
    assert_eq!(at.second_weight.to_bits(), 0x3f19_9998);
    let at = Selection::at(f(0.9), 4, f(100.0), F32::ZERO, false).unwrap();
    assert_eq!((at.first, at.second), (3, 3));
    // Two frames' time (50 ms) or more since the previous pose: no blend.
    let at = Selection::at(f(0.9), 4, f(100.0), f(50.0), true).unwrap();
    assert_eq!((at.first, at.second, at.interpolate), (3, 3, false));
    assert_eq!(at.second_weight, F32::ZERO);
    // A phase past the end keeps to the last frame.
    let at = Selection::at(f(1.5), 4, f(100.0), F32::ZERO, false).unwrap();
    assert_eq!(at.first, 3);
    assert!(Selection::at(f(0.5), 0, f(100.0), F32::ZERO, true).is_err());
}

#[test]
fn a_motion_replaces_only_the_channels_it_has() {
    let translations = [[f(0.0), f(2.0), f(4.0)], [f(1.0), f(4.0), f(8.0)]];
    let at = Selection::at(f(0.125), 2, f(100.0), F32::ZERO, true).unwrap();
    assert_eq!(at.second_weight, f(0.25));
    let mut bone = ORIGIN;
    let motion = Motion {
        translations: Some(&translations),
        rotations: None,
    };
    pose::sample(&mut bone, motion, at).unwrap();
    assert_eq!(bits(bone.translation), bits([f(0.25), f(2.5), f(5.0)]));
    assert_eq!(bone.rotation, ORIGIN.rotation);
    // Two equal rotations are not blended; a missing frame fails.
    let rotations = [[f(0.0), f(1.0), f(0.0), f(0.0)]; 2];
    let motion = Motion {
        translations: None,
        rotations: Some(&rotations),
    };
    pose::sample(&mut bone, motion, at).unwrap();
    assert_eq!(bone.rotation, rotations[0]);
    let motion = Motion {
        translations: Some(&translations[..1]),
        rotations: None,
    };
    assert!(pose::sample(&mut bone, motion, at).is_err());
}

#[test]
fn rotations_blend_the_short_way_and_are_scaled_to_length_one() {
    let a = [f(0.0), f(0.0), f(0.0), f(1.0)];
    let b = [f(1.0), f(0.0), f(0.0), f(0.0)];
    let d = [f(0.0), f(1.0), f(0.0), f(0.0)];
    // Halfway: (0.5, 0, 0, 0.5), whose squared length is 0.5, scaled by
    // 1 / sqrt(0.5) to 0.70710677 each.
    let half = pose::blend(f(0.5), a, b, d);
    assert_eq!(bits(half), [0x3f35_04f3, 0, 0, 0x3f35_04f3]);
    // When the two point apart (a negative product), the other is
    // turned round: b and -b give the same rotation.
    let c = [f(0.6), f(0.0), f(0.0), f(0.8)];
    let negative = c.map(|v| F32::from_f32(-v.to_f32()));
    assert_eq!(
        bits(pose::blend(f(0.5), a, negative, d)),
        bits(pose::blend(f(0.5), a, c, d))
    );
    // Near a whole weight it is a; near none, b; and a weight beyond
    // those bounds leaves the destination.
    assert_eq!(pose::blend(f(0.995), a, b, d), a);
    assert_eq!(pose::blend(f(0.005), a, b, d), b);
    assert_eq!(pose::blend(f(1.5), a, b, d), d);
}

#[test]
fn a_bone_is_placed_by_its_parent() {
    let local = Bone {
        translation: [f(1.0), f(2.0), f(3.0)],
        rotation: [f(0.0), f(0.0), f(0.0), f(1.0)],
    };
    // At the origin the offset and the rotation stay.
    let placed = pose::compose(local, ORIGIN);
    assert_eq!(placed.translation, local.translation);
    assert_eq!(bits(placed.rotation), bits(local.rotation));
    // A turn is kept in the other convention, its vector part negated.
    let turned = Bone {
        rotation: [f(0.6), f(0.0), f(0.0), f(0.8)],
        ..local
    };
    let placed = pose::compose(turned, ORIGIN);
    assert_eq!(placed.rotation[0], f(-0.6));
    assert_eq!(placed.rotation[3], f(0.8));
    // A parent turned half round about the third axis, at (10, 0, 0),
    // turns the offset to (-1, -2, 3).
    let parent = Bone {
        translation: [f(10.0), F32::ZERO, F32::ZERO],
        rotation: [f(0.0), f(0.0), f(1.0), f(0.0)],
    };
    let placed = pose::compose(local, parent);
    assert_eq!(bits(placed.translation), bits([f(9.0), f(-2.0), f(3.0)]));
    // A chain: each bone after its parent.
    let all = pose::absolute(&[None, Some(0)], &[parent, local]).unwrap();
    assert_eq!(all[1].translation, placed.translation);
    assert!(pose::absolute(&[Some(1), None], &[local, parent]).is_err());
}
