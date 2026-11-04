//! Original mesh-bound update (59a1c1), before renderer handedness conversion.
//! Bone matrices/stamps and per-binding local bounds come from the native
//! binding/geometry host. This does not replace mesh construction or skinning.
use super::model::Bounds;
use serde::{Deserialize, Serialize};

/// Fresh 59a45d local-box construction after the BMF reader. The captured
/// 5c7610 helper ignores the requested 24-byte stride and scans `count` packed
/// triples starting at the first primary vertex, including intervening normals.
/// Preserve that observed behavior, rather than computing an ideal vertex box.
/// This is distinct from the posed box at mesh +94 and its per-binding cache.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalBounds {
    pub local: Vec<Bounds>,
    pub bounds: Bounds,
}
impl LocalBounds {
    pub fn from_mesh(mesh: &sims_formats::avatar::Mesh) -> Result<Self, String> {
        let mut result = Self {
            local: Vec::with_capacity(mesh.bindings.len()),
            bounds: Bounds::EMPTY,
        };
        for binding in &mesh.bindings {
            let mut bounds = Bounds::EMPTY;
            // Native checks count before using first_vertex. Empty rows can
            // carry UINT_MAX as their starting offset.
            if binding.vertex_count != 0 {
                let end = binding
                    .first_vertex
                    .checked_add(binding.vertex_count)
                    .ok_or("mesh local-bound range overflow")?;
                let vertices = mesh
                    .vertices
                    .get(binding.first_vertex..end)
                    .ok_or("mesh local-bound range outside primary vertices")?;
                for i in 0..binding.vertex_count {
                    let vertex = &vertices[i / 2];
                    let point = if i % 2 == 0 {
                        vertex.position
                    } else {
                        vertex.normal
                    };
                    if point.iter().any(|v| !v.is_finite()) {
                        return Err("nonfinite mesh local-bound position".into());
                    }
                    for axis in 0..3 {
                        bounds.minimum[axis] = bounds.minimum[axis].min(point[axis]);
                        bounds.maximum[axis] = bounds.maximum[axis].max(point[axis]);
                    }
                }
                for axis in 0..3 {
                    result.bounds.minimum[axis] =
                        result.bounds.minimum[axis].min(bounds.minimum[axis]);
                    result.bounds.maximum[axis] =
                        result.bounds.maximum[axis].max(bounds.maximum[axis]);
                }
            }
            result.local.push(bounds);
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub position: [f32; 3],
    /// The native bone +c4 matrix, in its stored nine-float order.
    pub rotation: [f32; 9],
    pub stamp: u32,
}
impl Pose {
    /// 596254 / 5a69c0. The mesh matrix has different x87 spills from the
    /// attachment matrix; extracting nine entries from that matrix is inexact.
    pub fn from_absolute(
        bone: crate::head_look::rig::AbsoluteBone,
        stamp: u32,
    ) -> Result<Self, String> {
        Self::from_absolute_with_control(bone, stamp, 0x037f)
    }
    pub fn from_absolute_with_control(
        bone: crate::head_look::rig::AbsoluteBone,
        stamp: u32,
        control: u16,
    ) -> Result<Self, String> {
        if !matches!(control, 0x003f | 0x037f) {
            return Err("unsupported native mesh matrix x87 control".into());
        }
        if bone
            .translation
            .iter()
            .chain(&bone.rotation)
            .any(|v| !v.is_finite())
        {
            return Err("invalid mesh bone pose".into());
        }
        if control == 0x003f {
            // 5a69c0 doubles rounded products (not a reassociated dot product).
            let [x, y, z, w] = bone.rotation;
            let w = -w;
            let twice = |a: f32, b: f32| {
                let v = a * b;
                v + v
            };
            let (xx, yy, zz) = (twice(x, x), twice(y, y), twice(z, z));
            let (xy, xz, wx) = (twice(y, x), twice(z, x), twice(w, x));
            let (yz, wy, wz) = (twice(z, y), twice(w, y), twice(w, z));
            let rotation = [
                1. - yy - zz,
                xy - wz,
                wy + xz,
                wz + xy,
                1. - xx - zz,
                yz - wx,
                xz - wy,
                yz + wx,
                (1. - xx) - yy,
            ];
            if rotation.iter().any(|v| !v.is_finite()) {
                return Err("mesh bone matrix overflow".into());
            }
            return Ok(Self {
                position: bone.translation,
                rotation,
                stamp,
            });
        }
        let [x, y, z, w] = bone.rotation.map(f64::from);
        let w = -w;
        let spill = |v: f64| f64::from(v as f32);
        let xx = 2. * x * x;
        let yy = 2. * y * y;
        let zz = 2. * z * z;
        let xy = 2. * x * y;
        let xz = spill(2. * x * z);
        let wx = spill(2. * w * x);
        let yz = spill(2. * y * z);
        let wy = spill(2. * w * y);
        let wz = 2. * w * z;
        let rotation = [
            1. - yy - zz,
            xy - wz,
            wy + xz,
            wz + xy,
            1. - xx - zz,
            yz - wx,
            xz - wy,
            yz + wx,
            spill(1. - xx) - yy,
        ]
        .map(|v| v as f32);
        if rotation.iter().any(|v| !v.is_finite()) {
            return Err("mesh bone matrix overflow".into());
        }
        Ok(Self {
            position: bone.translation,
            rotation,
            stamp,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshBounds {
    pub local: Vec<Bounds>,
    pub stamps: Vec<u32>,
    pub bounds: Bounds,
}
impl MeshBounds {
    /// Native cache gate applies even for rest-space evaluation. Only changed
    /// bindings contribute corners; if every changed box is empty, retain the
    /// previous combined box while still committing each changed pose stamp.
    pub fn update(&mut self, poses: &[Pose], rest: bool) -> Result<bool, String> {
        if self.local.len() != poses.len() || self.stamps.len() != poses.len() {
            return Err("mesh-bound binding storage length mismatch".into());
        }
        for (local, pose) in self.local.iter().zip(poses) {
            if local
                .minimum
                .iter()
                .chain(&local.maximum)
                .chain(&pose.position)
                .chain(&pose.rotation)
                .any(|v| !v.is_finite())
            {
                return Err("nonfinite mesh-bound input".into());
            }
        }
        let mut combined = Bounds::EMPTY;
        let mut changed = false;
        for (i, &pose) in poses.iter().enumerate() {
            if self.stamps[i] == pose.stamp {
                continue;
            }
            let local = self.local[i];
            if (0..3).all(|axis| local.maximum[axis] >= local.minimum[axis]) {
                for corner in 0..8 {
                    let point = std::array::from_fn(|axis| {
                        if corner & (1 << axis) == 0 {
                            local.minimum[axis]
                        } else {
                            local.maximum[axis]
                        }
                    });
                    let point = if rest { point } else { transform(point, pose) };
                    if point.iter().any(|v| !v.is_finite()) {
                        return Err("mesh-bound transform overflow".into());
                    }
                    for axis in 0..3 {
                        combined.minimum[axis] = combined.minimum[axis].min(point[axis]);
                        combined.maximum[axis] = combined.maximum[axis].max(point[axis]);
                    }
                }
            }
            changed = true;
            self.stamps[i] = pose.stamp;
        }
        if changed && (0..3).all(|axis| combined.maximum[axis] >= combined.minimum[axis]) {
            self.bounds = combined;
        }
        Ok(changed)
    }
}
fn transform(p: [f32; 3], pose: Pose) -> [f32; 3] {
    // The captured native CPU dispatch uses SSE with a float rounding after
    // each multiply/add. Keep those spills; this path is not x87 matrix.point.
    std::array::from_fn(|axis| {
        (p[0] * pose.rotation[axis] + pose.position[axis])
            + (p[1] * pose.rotation[3 + axis] + p[2] * pose.rotation[6 + axis])
    })
}
