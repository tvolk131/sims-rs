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
#[derive(Debug, Clone, PartialEq)]
pub struct Skinned {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
}

/// Places `part`'s vertices on the bones `world` (one per skeleton bone,
/// from [`rig::Pose::world`]).
pub fn skin(part: &Part, world: &[Transform]) -> Skinned {
    let mesh = &part.mesh;
    let bone = |b: usize| world[part.bones[b]];
    let mut positions = Vec::with_capacity(mesh.vertices.len());
    let mut normals = Vec::with_capacity(mesh.vertices.len());
    for v in &mesh.vertices {
        let m = bone(v.bone);
        positions.push(m.point(rig::position(v.position)));
        normals.push(m.vector(rig::position(v.normal)));
    }
    for b in &mesh.blends {
        let m = bone(b.bone);
        let p = m.point(rig::position(b.position));
        let n = m.vector(rig::position(b.normal));
        positions[b.vertex] = positions[b.vertex].lerp(p, b.weight);
        normals[b.vertex] = normals[b.vertex].lerp(n, b.weight);
    }
    for n in &mut normals {
        *n = n.normalize();
    }
    Skinned { positions, normals }
}
