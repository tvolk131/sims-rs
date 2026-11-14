//! Poses a skeleton: each bone's place relative to its parent, and from
//! those its place on the body.
//!
//! The files keep positions and rotations in the original's convention.
//! [`from_file`] converts them once, at this boundary, to a right-handed
//! frame with Y up and Hamilton quaternions: a position `[x, y, z]`
//! becomes `[-x, y, z]`, and a rotation `[x, y, z, w]` becomes
//! `[-x, y, z, w]`. That is a mirror of the X axis, applied to rotations
//! that the original composes the other way round. The old prototype's
//! renderer made the same conversion (legacy/crates/sims-render/src/pose.rs,
//! `translation`, `rotation` and `presentation_matrix`). Whether the
//! result is the original's image or its mirror image is **unverified**
//! until it is compared with a screenshot of the original.

use crate::math::{Quat, Transform, Vec3};
use sims_formats::avatar::Skeleton;

/// A position from a file, in the drawing frame.
pub fn position(v: [f32; 3]) -> Vec3 {
    Vec3::new(-v[0], v[1], v[2])
}

/// A rotation from a file, in the drawing frame.
pub fn rotation(q: [f32; 4]) -> Quat {
    Quat::new(-q[0], q[1], q[2], q[3]).normalize()
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
        // A quarter turn about Z takes the child's +Y offset to -X.
        let p = world[1].point(Vec3::ZERO);
        assert!((p - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-5, "{p:?}");
        assert_eq!(bone(&s, "child"), Some(1));
    }

    #[test]
    fn the_root_transform_places_the_whole_body() {
        let s = skeleton();
        let root = Transform {
            rotation: Quat::IDENTITY,
            translation: Vec3::new(5.0, 0.0, 0.0),
        };
        let world = Pose::rest(&s).world(&s, root);
        assert!((world[1].point(Vec3::ZERO) - Vec3::new(4.0, 0.0, 0.0)).length() < 1e-5);
    }
}
