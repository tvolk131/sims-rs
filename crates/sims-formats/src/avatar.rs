//! Bounded original avatar catalogs (BCF/CMX) and meshes (BMF/SKN).
//! Coordinates and quaternions remain in the source convention.
pub mod catalog;
pub mod index;
mod text;
pub use text::{Cmx, read_cmx, read_skn};

use crate::animation::{count, finite, pairs, string};
use crate::{Result, reader::Reader};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Wiggle {
    /// Native bone +0x78, an integer gradient magnitude, not a float.
    pub gradient_scale: i32,
    /// Native bone +0x7c.
    pub amplitude: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub properties: Vec<Vec<(String, String)>>,
    /// Native bone +0x6c/+0x70/+0x74. The third flag controls layer blending.
    pub flags: [u32; 3],
    pub wiggle: Wiggle,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Skeleton {
    pub name: String,
    pub bones: Vec<Bone>,
}
#[derive(Debug, Clone)]
pub struct Binding {
    pub bone: String,
    pub mesh: String,
    pub censor: u32,
}
#[derive(Debug, Clone)]
pub struct Appearance {
    pub name: String,
    pub kind: u32,
    pub bindings: Vec<Binding>,
}
#[derive(Debug, Clone)]
pub struct Catalog {
    pub skeletons: Vec<Skeleton>,
    pub appearances: Vec<Appearance>,
}

pub fn read_bcf(data: &[u8]) -> Result<Catalog> {
    let mut r = Reader::new(data);
    let mut skeletons = Vec::new();
    for _ in 0..count(&mut r, 64, 3)? {
        skeletons.push(read_skeleton(&mut r)?);
    }
    let mut appearances = Vec::new();
    for _ in 0..count(&mut r, 256, 13)? {
        appearances.push(read_appearance(&mut r)?);
    }
    if r.u32()? != 0 {
        return r.fail("avatar catalog containing animations is not supported");
    }
    if r.remaining() != 0 {
        return r.fail("trailing avatar catalog bytes");
    }
    Ok(Catalog {
        skeletons,
        appearances,
    })
}
pub(crate) fn read_skeleton(r: &mut Reader<'_>) -> Result<Skeleton> {
    let name = string(r)?;
    let n = r.u32()? as usize;
    if n == 0 || n > 256 {
        return r.fail("invalid skeleton bone count");
    }
    let mut bones = Vec::new();
    let mut parents = Vec::new();
    let mut names = BTreeSet::new();
    for _ in 0..n {
        let name = string(r)?;
        if name.is_empty() || !names.insert(name.to_ascii_lowercase()) {
            return r.fail("empty or duplicate bone name");
        }
        parents.push(string(r)?);
        let mut properties = Vec::new();
        for _ in 0..count(r, 256, 4)? {
            properties.push(pairs(r)?);
        }
        let translation = vector(r)?;
        let rotation = [finite(r)?, finite(r)?, finite(r)?, finite(r)?];
        let len: f32 = rotation.iter().map(|v| v * v).sum();
        if !(0.5..=1.5).contains(&len) {
            return r.fail("invalid bone quaternion");
        }
        let flags = [r.u32()?, r.u32()?, r.u32()?];
        if flags.iter().any(|v| *v > 1) {
            return r.fail("invalid bone flag");
        }
        let wiggle = Wiggle {
            gradient_scale: r.i32()?,
            amplitude: finite(r)?,
        };
        bones.push(Bone {
            name,
            parent: None,
            translation,
            rotation,
            properties,
            flags,
            wiggle,
        });
    }
    let mut roots = 0;
    for i in 0..n {
        if parents[i] == "NULL" {
            roots += 1;
        } else {
            bones[i].parent = Some(
                bones
                    .iter()
                    .position(|b| b.name == parents[i])
                    .ok_or_else(|| crate::Error::new(r.pos, "unknown parent bone"))?,
            );
        }
    }
    if roots != 1 {
        return r.fail("skeleton must have one root");
    }
    for i in 0..n {
        let mut visited = BTreeSet::new();
        let mut next = Some(i);
        while let Some(j) = next {
            if !visited.insert(j) {
                return r.fail("cycle in skeleton");
            }
            next = bones[j].parent;
        }
    }
    Ok(Skeleton { name, bones })
}
pub(crate) fn read_appearance(r: &mut Reader<'_>) -> Result<Appearance> {
    let name = string(r)?;
    let kind = r.u32()?;
    if r.u32()? != 0 {
        return r.fail("unsupported appearance reserved field");
    }
    let mut bindings = Vec::new();
    for _ in 0..count(r, 256, 10)? {
        let bone = string(r)?;
        let mesh = string(r)?;
        let censor = r.u32()?;
        if r.u32()? != 0 {
            return r.fail("unsupported binding reserved field");
        }
        if bone.is_empty() || mesh.is_empty() {
            return r.fail("empty appearance binding");
        }
        bindings.push(Binding { bone, mesh, censor });
    }
    Ok(Appearance {
        name,
        kind,
        bindings,
    })
}
fn vector(r: &mut Reader<'_>) -> Result<[f32; 3]> {
    Ok([finite(r)?, finite(r)?, finite(r)?])
}
#[derive(Debug, Clone)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub bone: usize,
}
#[derive(Debug, Clone)]
pub struct Blend {
    pub vertex: usize,
    pub weight: f32,
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub bone: usize,
}
/// Original BMF binding row. Preserve its order and empty-range offsets: the
/// engine keeps bounds/cache state per row, independently of flattened vertices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MeshBinding {
    pub bone: usize,
    pub first_vertex: usize,
    pub vertex_count: usize,
    pub first_blend: usize,
    pub blend_count: usize,
}
#[derive(Debug, Clone)]
pub struct Mesh {
    pub name: String,
    pub texture: String,
    pub bones: Vec<String>,
    pub bindings: Vec<MeshBinding>,
    pub triangles: Vec<[u32; 3]>,
    pub vertices: Vec<Vertex>,
    pub blends: Vec<Blend>,
}
pub fn read_bmf(data: &[u8]) -> Result<Mesh> {
    let mut r = Reader::new(data);
    let name = string(&mut r)?;
    let texture = string(&mut r)?;
    let mut bones = Vec::new();
    for _ in 0..count(&mut r, 256, 1)? {
        bones.push(string(&mut r)?);
    }
    if bones.is_empty() {
        return r.fail("mesh has no bones");
    }
    let mut triangles = Vec::new();
    for _ in 0..count(&mut r, 100_000, 12)? {
        triangles.push([r.u32()?, r.u32()?, r.u32()?]);
    }
    let mut bindings = Vec::new();
    for _ in 0..count(&mut r, 1024, 20)? {
        let b = [r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?];
        if b[0] as usize >= bones.len() {
            return r.fail("mesh binding bone outside table");
        }
        bindings.push(b);
    }
    let n = count(&mut r, 100_000, 8)?;
    if triangles.iter().flatten().any(|v| *v as usize >= n) {
        return r.fail("triangle vertex outside mesh");
    }
    let mut vertices = Vec::new();
    for _ in 0..n {
        vertices.push(Vertex {
            position: [0.; 3],
            normal: [0.; 3],
            uv: [finite(&mut r)?, finite(&mut r)?],
            bone: usize::MAX,
        });
    }
    let mut blends = Vec::new();
    let mut blended = BTreeSet::new();
    for _ in 0..count(&mut r, 100_000, 8)? {
        let weight = r.u32()?;
        let vertex = r.u32()? as usize;
        if weight > 32768 || vertex >= n || !blended.insert(vertex) {
            return r.fail("invalid or duplicate blend vertex");
        }
        blends.push(Blend {
            vertex,
            weight: weight as f32 / 32768.,
            position: [0.; 3],
            normal: [0.; 3],
            bone: usize::MAX,
        });
    }
    if r.u32()? as usize != n + blends.len() {
        return r.fail("mesh sample count disagrees with vertex counts");
    }
    for v in &mut vertices {
        v.position = vector(&mut r)?;
        v.normal = vector(&mut r)?;
    }
    for v in &mut blends {
        v.position = vector(&mut r)?;
        v.normal = vector(&mut r)?;
    }
    if r.remaining() != 0 {
        return r.fail("trailing BMF bytes");
    }
    let mut mesh = Mesh {
        name,
        texture,
        bones,
        bindings: bindings
            .into_iter()
            .map(|b| MeshBinding {
                bone: b[0] as usize,
                first_vertex: b[1] as usize,
                vertex_count: b[2] as usize,
                first_blend: b[3] as usize,
                blend_count: b[4] as usize,
            })
            .collect(),
        triangles,
        vertices,
        blends,
    };
    validate_mesh(&mut mesh, r.pos)?;
    Ok(mesh)
}

/// Shared structural checks for binary and text mesh inputs. Empty ranges keep
/// their original sentinel offsets; only nonempty ranges address vertex storage.
fn validate_mesh(mesh: &mut Mesh, offset: usize) -> Result<()> {
    let fail = |message| crate::Error::new(offset, message);
    if mesh.bones.is_empty() {
        return Err(fail("mesh has no bones"));
    }
    if mesh
        .triangles
        .iter()
        .flatten()
        .any(|v| *v as usize >= mesh.vertices.len())
    {
        return Err(fail("triangle vertex outside mesh"));
    }
    let mut blended = BTreeSet::new();
    for b in &mesh.blends {
        if b.vertex >= mesh.vertices.len()
            || !(0.0..=1.0).contains(&b.weight)
            || !blended.insert(b.vertex)
        {
            return Err(fail("invalid or duplicate blend vertex"));
        }
    }
    for b in &mesh.bindings {
        if b.bone >= mesh.bones.len() {
            return Err(fail("mesh binding bone outside table"));
        }
        for (first, length, total, blend) in [
            (b.first_vertex, b.vertex_count, mesh.vertices.len(), false),
            (b.first_blend, b.blend_count, mesh.blends.len(), true),
        ] {
            if length == 0 {
                continue;
            }
            let end = first
                .checked_add(length)
                .ok_or_else(|| fail("binding overflow"))?;
            if end > total {
                return Err(fail("mesh binding outside vertices"));
            }
            for i in first..end {
                let slot = if blend {
                    &mut mesh.blends[i].bone
                } else {
                    &mut mesh.vertices[i].bone
                };
                if *slot != usize::MAX {
                    return Err(fail("overlapping mesh bindings"));
                }
                *slot = b.bone;
            }
        }
    }
    if mesh.vertices.iter().any(|v| v.bone == usize::MAX)
        || mesh.blends.iter().any(|v| v.bone == usize::MAX)
    {
        return Err(fail("unbound mesh vertex"));
    }
    Ok(())
}
