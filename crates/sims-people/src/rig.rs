//! Poses a skeleton: each bone's place relative to its parent, and from
//! those its place on the body.
//!
//! The drawing frame is the files' own frame, Y up: a position `[x, y, z]`
//! is kept as it is. The original keeps a turn with x, y and z negated
//! relative to the Hamilton quaternions used here (its bone matrix,
//! 0x595f32, negates them again), so [`from_file`] turns a rotation
//! `[x, y, z, w]` into `[-x, -y, -z, w]`. With that, each bone lands
//! exactly where the original's composition (0x59615b, ported as
//! `sims_sim::pose::absolute`) puts it, as a test below checks.
//!
//! The camera ([`crate::raster::Camera`]) reads the frame as the
//! original's renderer does: x right, y up, z away from the viewer. So
//! the image is the original's, not its mirror image. That was checked
//! against the original: Cassandra Goth's shirt has a pocket protector on
//! her own left side in two screenshots of the original, and is drawn on
//! her left here. The old prototype (and this crate before) mirrored the
//! X axis (legacy/crates/sims-render/src/pose.rs), which drew it on her
//! right.

use crate::math::{Quat, Transform, Vec3};
use sims_core::F32;
use sims_formats::avatar::Skeleton;

/// A position from a file, in the drawing frame.
pub fn position(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}

/// A rotation from a file, in the drawing frame.
pub fn rotation(q: [f32; 4]) -> Quat {
    Quat::new(-q[0], -q[1], -q[2], q[3]).normalize()
}

/// A rotation in the drawing frame, back in the files' convention: the
/// inverse of [`rotation`].
fn to_file(q: Quat) -> [f32; 4] {
    [-q.x, -q.y, -q.z, q.w]
}

/// A bone's place relative to its parent, from a file's position and
/// rotation.
pub fn from_file(translation: [f32; 3], rotation_: [f32; 4]) -> Transform {
    Transform {
        rotation: rotation(rotation_),
        translation: position(translation),
    }
}

/// Each bone's place relative to its parent, in the skeleton's order.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    pub local: Vec<Transform>,
}

impl Pose {
    /// The skeleton's rest pose, as its file gives it.
    pub fn rest(skeleton: &Skeleton) -> Self {
        Self {
            local: skeleton
                .bones
                .iter()
                .map(|b| from_file(b.translation, b.rotation))
                .collect(),
        }
    }

    /// Turns the bone `HEAD` by `turn` eighths of a turn, as the original
    /// draws a person whose head follows something: the simulation keeps
    /// the turn ([`sims_sim::head::Head::turn`], at most 1.5 either way)
    /// and [`sims_sim::head::turned`] works out the bone's rotation, in the
    /// files' convention, with the original's arithmetic. A turn of 0, or
    /// a skeleton without a `HEAD` bone, leaves the pose as it is. Only
    /// that bone turns: the original places it again and leaves the bones
    /// below it where they were (0x590a3c), so each bone hanging from it
    /// is given the turn back. The original's skeletons have none.
    pub fn turn_head(&mut self, skeleton: &Skeleton, turn: F32) {
        if turn == F32::ZERO {
            return;
        }
        let Some(head) = bone(skeleton, "HEAD") else {
            return;
        };
        let Some(local) = self.local.get_mut(head) else {
            return;
        };
        let before = local.rotation;
        let q = to_file(before).map(F32::from_f32);
        let after = rotation(sims_sim::head::turned(q, turn).map(F32::to_f32));
        local.rotation = after;
        // A child's place on the body is head * child; keep it by taking
        // the turn off: child' = after⁻¹ * before * child.
        let undo = Transform {
            rotation: (Quat::new(-after.x, -after.y, -after.z, after.w) * before).normalize(),
            translation: Vec3::ZERO,
        };
        for (i, b) in skeleton.bones.iter().enumerate() {
            if b.parent == Some(head)
                && let Some(child) = self.local.get_mut(i)
            {
                *child = undo * *child;
            }
        }
    }

    /// Each bone's place on the body, with the root placed by `root`.
    /// The skeleton reader guarantees one root and no cycles.
    pub fn world(&self, skeleton: &Skeleton, root: Transform) -> Vec<Transform> {
        let mut out: Vec<Option<Transform>> = vec![None; skeleton.bones.len()];
        for i in 0..skeleton.bones.len() {
            resolve(i, skeleton, &self.local, root, &mut out);
        }
        out.into_iter().map(|t| t.unwrap_or(root)).collect()
    }
}

fn resolve(
    i: usize,
    skeleton: &Skeleton,
    local: &[Transform],
    root: Transform,
    out: &mut [Option<Transform>],
) -> Transform {
    if let Some(t) = out[i] {
        return t;
    }
    let parent = match skeleton.bones[i].parent {
        Some(p) => resolve(p, skeleton, local, root, out),
        None => root,
    };
    let t = parent * local[i];
    out[i] = Some(t);
    t
}

/// The bone named `name`, ignoring ASCII case.
pub fn bone(skeleton: &Skeleton, name: &str) -> Option<usize> {
    skeleton
        .bones
        .iter()
        .position(|b| b.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sims_formats::avatar::{Bone, Wiggle};

    /// A two-bone skeleton: ROOT at the origin, CHILD one unit up its Y
    /// axis, the root turned a quarter about Z in the file.
    pub(crate) fn skeleton() -> Skeleton {
        let bone = |name: &str, parent, translation, rotation| Bone {
            name: name.into(),
            parent,
            translation,
            rotation,
            properties: Vec::new(),
            flags: [0; 3],
            wiggle: Wiggle::default(),
        };
        let h = std::f32::consts::FRAC_1_SQRT_2;
        Skeleton {
            name: "test".into(),
            bones: vec![
                bone("ROOT", None, [0.0, 0.0, 0.0], [0.0, 0.0, h, h]),
                bone("CHILD", Some(0), [0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]),
            ],
        }
    }

    #[test]
    fn the_rest_pose_chains_parents() {
        let s = skeleton();
        let world = Pose::rest(&s).world(&s, Transform::IDENTITY);
        // The root's quarter turn about the file's Z takes the child's +Y
        // offset to +X, as the original composes it (see below).
        let p = world[1].point(Vec3::ZERO);
        assert!((p - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-5, "{p:?}");
        assert_eq!(bone(&s, "child"), Some(1));
    }

    #[test]
    fn a_rotation_goes_back_to_the_files_convention() {
        let q = [0.1, -0.2, 0.3, 0.927_362];
        let back = to_file(rotation(q));
        let len = q.iter().map(|v| v * v).sum::<f32>().sqrt();
        for (a, b) in back.iter().zip(q) {
            assert!((a - b / len).abs() < 1e-6, "{back:?}");
        }
    }

    #[test]
    fn the_head_turns_by_the_simulations_turn() {
        let mut s = skeleton();
        s.bones[1].name = "HEAD".into();
        let rest = Pose::rest(&s);
        let mut pose = rest.clone();
        pose.turn_head(&s, F32::ZERO);
        assert_eq!(pose, rest, "no turn, no change");
        // One eighth of a turn is a rotation of pi/4 about the file's
        // first axis, multiplied on the left in the original's order:
        // [sin(pi/8), 0, 0, cos(pi/8)] for a bone at rest.
        pose.turn_head(&s, F32::from_f32(1.0));
        assert_eq!(pose.local[0], rest.local[0], "only HEAD turns");
        let turned = to_file(pose.local[1].rotation);
        // sin and cos of pi/8.
        let want = [0.382_683_43, 0.0, 0.0, 0.923_879_5];
        for (a, b) in turned.iter().zip(want) {
            assert!((a - b).abs() < 1e-6, "{turned:?}");
        }
        // Past 1.2 radians either way the turn stops (1.5 eighths is
        // about 1.18 radians, so it is reached from a head already turned).
        let mut far = rest.clone();
        far.turn_head(&s, F32::from_f32(1.5));
        let mut farther = far.clone();
        farther.turn_head(&s, F32::from_f32(1.5));
        // A rotation of 1.2 radians about the first axis: sin(0.6) there.
        let x = to_file(farther.local[1].rotation)[0];
        assert!((x - 0.564_642_5).abs() < 1e-5, "{x}");
        // A skeleton without a HEAD bone is left alone.
        let plain = skeleton();
        let mut pose = Pose::rest(&plain);
        pose.turn_head(&plain, F32::from_f32(1.0));
        assert_eq!(pose, Pose::rest(&plain));
    }

    #[test]
    fn the_turn_is_multiplied_in_the_originals_order() {
        // HEAD at rest a sixth of a turn about the file's Z axis, which
        // does not commute with the turn about X. (A quarter turn would
        // put the angle `turned` reads at its arc tangent's edge, where
        // rounding decides between 0 and pi.)
        let mut s = skeleton();
        s.bones[1].name = "HEAD".into();
        let sixth = [0.0, 0.0, 0.5, 0.866_025_4];
        s.bones[1].rotation = sixth;
        let mut pose = Pose::rest(&s);
        pose.turn_head(&s, F32::from_f32(1.0));
        let q = sixth.map(F32::from_f32);
        let r = [0.382_683_43, 0.0, 0.0, 0.923_879_5].map(F32::from_f32);
        let want = sims_sim::head::multiply(r, q).map(F32::to_f32);
        let other = sims_sim::head::multiply(q, r).map(F32::to_f32);
        let got = to_file(pose.local[1].rotation);
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-6);
        assert!(near(got, want), "{got:?} {want:?}");
        assert!(!near(got, other), "the order matters here");
    }

    #[test]
    fn bones_below_the_head_stay_where_they_were() {
        let mut s = skeleton();
        s.bones[1].name = "HEAD".into();
        let mut hat = s.bones[1].clone();
        hat.name = "HAT".into();
        hat.parent = Some(1);
        hat.translation = [0.3, 0.5, -0.2];
        hat.rotation = [0.1, 0.2, 0.3, 0.927_362];
        s.bones.push(hat);
        let rest = Pose::rest(&s);
        let mut pose = rest.clone();
        pose.turn_head(&s, F32::from_f32(1.0));
        assert_ne!(pose.local[1], rest.local[1]);
        let (a, b) = (
            rest.world(&s, Transform::IDENTITY)[2],
            pose.world(&s, Transform::IDENTITY)[2],
        );
        assert!(
            (a.translation - b.translation).length() < 1e-5,
            "{a:?} {b:?}"
        );
        assert!(a.rotation.dot(b.rotation).abs() > 1.0 - 1e-6, "{a:?} {b:?}");
    }

    #[test]
    fn the_root_transform_places_the_whole_body() {
        let s = skeleton();
        let root = Transform {
            rotation: Quat::IDENTITY,
            translation: Vec3::new(5.0, 0.0, 0.0),
        };
        let world = Pose::rest(&s).world(&s, root);
        assert!((world[1].point(Vec3::ZERO) - Vec3::new(6.0, 0.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn bones_are_placed_as_the_original_places_them() {
        // A chain of three bones whose turns do not commute, placed here
        // and by the simulation's port of the original's composition
        // (0x59615b, `sims_sim::pose::absolute`), in the files' frame.
        let mut s = skeleton();
        let mut tip = s.bones[1].clone();
        tip.name = "TIP".into();
        tip.parent = Some(1);
        tip.translation = [0.2, 0.7, -0.4];
        tip.rotation = [0.5, 0.5, 0.5, 0.5];
        s.bones[1].translation = [0.3, 1.0, 0.1];
        s.bones[1].rotation = [0.1, 0.2, 0.3, 0.927_362];
        s.bones.push(tip);
        let ours = Pose::rest(&s).world(&s, Transform::IDENTITY);
        let f = |v: &[f32]| v.iter().map(|&x| F32::from_f32(x)).collect::<Vec<_>>();
        let locals: Vec<sims_sim::pose::Bone> = s
            .bones
            .iter()
            .map(|b| sims_sim::pose::Bone {
                translation: f(&b.translation).try_into().unwrap(),
                rotation: f(&b.rotation).try_into().unwrap(),
            })
            .collect();
        let parents: Vec<_> = s.bones.iter().map(|b| b.parent).collect();
        let original = sims_sim::pose::absolute(&parents, &locals).unwrap();
        for (o, w) in original.iter().zip(&ours) {
            let t = o.translation.map(F32::to_f32);
            let got = [w.translation.x, w.translation.y, w.translation.z];
            for (a, b) in got.iter().zip(t) {
                assert!((a - b).abs() < 1e-5, "{got:?} {t:?}");
            }
            // The same turn (q and -q are one rotation). The original keeps
            // a placed bone's turn with x, y and z negated, as the drawing
            // frame does; its bone matrix negates them again (0x595f32).
            let q = o.rotation.map(F32::to_f32);
            let r = [w.rotation.x, w.rotation.y, w.rotation.z, w.rotation.w];
            let dot: f32 = q.iter().zip(r).map(|(a, b)| a * b).sum();
            assert!(dot.abs() > 1.0 - 1e-5, "{q:?} {r:?}");
        }
    }
}
