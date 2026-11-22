//! Skinning: places a mesh's vertices on the posed bones.
//!
//! Each vertex belongs to one bone (its binding's) and is moved with it.
//! A blended vertex also has a second position on another bone, with a
//! weight from 0 to 1: the vertex ends that fraction of the way from its
//! own bone's position to the blend's. This is the old prototype's
//! skinning (legacy/crates/sims-render/src/pose.rs, `skin`); how close it
//! is to the original's own blend (0x5994c3 and after) is **unverified**.

use crate::appearance::Part;
use crate::math::{Transform, Vec3};
use crate::rig;

/// A mesh's vertices on the posed body: positions and unit normals, in
/// the mesh's vertex order.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Skinned {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
}

/// Places `part`'s vertices on the bones `world` (one per skeleton bone,
/// from [`rig::Pose::world`]), reusing `out`'s allocations.
pub(crate) fn skin_into(part: &Part, world: &[Transform], out: &mut Skinned) {
    let mesh = &part.mesh;
    let bone = |b: usize| world[part.bones[b]];
    out.positions.clear();
    out.normals.clear();
    out.positions.reserve(mesh.vertices.len());
    out.normals.reserve(mesh.vertices.len());
    for v in &mesh.vertices {
        let m = bone(v.bone);
        out.positions.push(m.point(rig::position(v.position)));
        out.normals.push(m.vector(rig::position(v.normal)));
    }
    for b in &mesh.blends {
        let m = bone(b.bone);
        let p = m.point(rig::position(b.position));
        let n = m.vector(rig::position(b.normal));
        out.positions[b.vertex] = out.positions[b.vertex].lerp(p, b.weight);
        out.normals[b.vertex] = out.normals[b.vertex].lerp(n, b.weight);
    }
    for n in &mut out.normals {
        *n = n.normalize();
    }
}

/// Places `part`'s vertices on the bones `world` (one per skeleton bone,
/// from [`rig::Pose::world`]).
pub fn skin(part: &Part, world: &[Transform]) -> Skinned {
    let mut out = Skinned {
        positions: Vec::new(),
        normals: Vec::new(),
    };
    skin_into(part, world, &mut out);
    out
}
