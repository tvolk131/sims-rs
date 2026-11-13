//! 599bf2 / 59a020 CPU vertex publication, before presentation conversion.
//! Drawable storage is already allocated. The caller supplies resolved binding
//! poses and services the returned renderer notification. This pass does not
//! sample animation or change mesh/bone pose-cache stamps.
use super::{mesh::Pose, model::Bounds};
use serde::{Deserialize, Serialize};
use sims_formats::avatar::Mesh;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Buffer {
    /// Primary vertices, followed by the auxiliary blend vertices.
    pub vertices: Vec<Vertex>,
    pub flags: u32,
    pub buffer_flags: u32,
    /// Native drawable's first inclusive primary-vertex range.
    pub range: [i32; 2],
    pub bounds: Bounds,
}
impl Buffer {
    /// Execute against an existing drawable. `cached_drawable` selects the
    /// original mesh owner's renderer branch. False sets buffer bit 0 and
    /// requests virtual +28(0) after completing the update. GPU notification
    /// effects are deliberately left to the caller.
    pub fn update(
        &mut self,
        mesh: &Mesh,
        poses: &[Option<Pose>],
        rest: bool,
        cached_drawable: bool,
        binding_bounds: Bounds,
    ) -> Result<bool, String> {
        self.update_with_control(mesh, poses, rest, cached_drawable, binding_bounds, 0x037f)
    }

    /// The batch transforms use the native SSE operation order. Interior
    /// blends and normal normalization use x87 and its caller-owned precision.
    pub fn update_with_control(
        &mut self,
        mesh: &Mesh,
        poses: &[Option<Pose>],
        rest: bool,
        cached_drawable: bool,
        binding_bounds: Bounds,
        control: u16,
    ) -> Result<bool, String> {
        if !matches!(control, 0x003f | 0x037f) {
            return Err("native vertex floating-point control unsupported".into());
        }
        let primary = mesh.vertices.len();
        if self.vertices.len() != primary + mesh.blends.len()
            || mesh.bindings.len() != poses.len()
            || primary > i32::MAX as usize
        {
            return Err("native vertex buffer shape mismatch".into());
        }
        for row in &mesh.bindings {
            for (first, count, total) in [
                (row.first_vertex, row.vertex_count, primary),
                (row.first_blend, row.blend_count, mesh.blends.len()),
            ] {
                if count != 0 && first.checked_add(count).is_none_or(|end| end > total) {
                    return Err("native vertex binding range outside samples".into());
                }
            }
        }
        if mesh.blends.iter().any(|b| {
            b.vertex >= primary
                || !b.weight.is_finite()
                || !(0. ..=1.).contains(&b.weight)
                || (b.weight * 32768.).fract() != 0.
        }) {
            return Err("native vertex blend descriptor invalid".into());
        }
        if !cached_drawable {
            self.buffer_flags |= 1;
        }
        let identity = Pose {
            position: [0.; 3],
            rotation: [1., 0., 0., 0., 1., 0., 0., 0., 1.],
            stamp: 0,
        };
        for (row, pose) in mesh.bindings.iter().zip(poses) {
            let pose = if rest {
                identity
            } else {
                pose.unwrap_or(identity)
            };
            for (first, count, secondary) in [
                (row.first_vertex, row.vertex_count, false),
                (row.first_blend, row.blend_count, true),
            ] {
                // Empty native rows may retain UINT_MAX offsets.
                if count == 0 {
                    continue;
                }
                self.flags |= 1;
                self.buffer_flags |= 4;
                for i in first..first + count {
                    let (position, normal, target) = if secondary {
                        let v = &mesh.blends[i];
                        (v.position, v.normal, primary + i)
                    } else {
                        let v = &mesh.vertices[i];
                        (v.position, v.normal, i)
                    };
                    self.vertices[target].position = transform(position, pose, true);
                    self.vertices[target].normal = transform(normal, pose, false);
                }
            }
        }
        self.range = [0, primary as i32 - 1];
        if !mesh.blends.is_empty() {
            self.flags |= 1;
            self.buffer_flags |= 4;
        }
        for (i, blend) in mesh.blends.iter().enumerate() {
            let source = self.vertices[primary + i];
            let target = &mut self.vertices[blend.vertex];
            if blend.weight == 0. {
                continue;
            }
            if blend.weight == 1. {
                target.position = source.position;
                target.normal = source.normal;
                continue;
            }
            // 59a55a rounds every arithmetic operation to PC=24 under 003f.
            // The historical 037f path retains its wider intermediates.
            if control == 0x003f {
                let q = super::transform::retained_single;
                let weight = f64::from(blend.weight) * 32768.;
                let mix = |a: f32, b: f32| {
                    q(q(q((32768. - weight) * f64::from(a)) + q(weight * f64::from(b)))
                        * (1. / 32768.)) as f32
                };
                target.position = std::array::from_fn(|i| mix(target.position[i], source.position[i]));
                let normal: [f32; 3] = std::array::from_fn(|i| mix(target.normal[i], source.normal[i]));
                let [x, y, z] = normal.map(f64::from);
                // 49b6be followed by 49b698: squared products, two sums,
                // square root, reciprocal, then each component multiply.
                // Retain x87's exponent range between stores, even when a
                // squared normal would underflow an intermediate f32.
                let length = q(q(q(x * x) + q(y * y)) + q(z * z));
                let scale = q(1. / q(length.sqrt()));
                target.normal = normal.map(|v| q(f64::from(v) * scale) as f32);
                continue;
            }
            let weight = f64::from(blend.weight) * 32768.;
            let mix = |a: f32, b: f32| {
                (((32768. - weight) * f64::from(a) + weight * f64::from(b)) / 32768.) as f32
            };
            target.position = std::array::from_fn(|i| mix(target.position[i], source.position[i]));
            let normal: [f32; 3] = std::array::from_fn(|i| mix(target.normal[i], source.normal[i]));
            let [x, y, z] = normal.map(f64::from);
            let scale = 1. / ((x * x + y * y) + z * z).sqrt();
            target.normal = normal.map(|v| (f64::from(v) * scale) as f32);
        }
        if self
            .vertices
            .iter()
            .any(|v| v.position.iter().chain(&v.normal).any(|v| !v.is_finite()))
        {
            return Err("native vertex update produced nonfinite coordinates".into());
        }
        self.bounds = binding_bounds;
        self.flags &= !1;
        Ok(!cached_drawable)
    }
}
fn transform(point: [f32; 3], pose: Pose, translate: bool) -> [f32; 3] {
    std::array::from_fn(|axis| {
        let first = point[0] * pose.rotation[axis];
        let first = if translate {
            first + pose.position[axis]
        } else {
            first
        };
        first + (point[1] * pose.rotation[3 + axis] + point[2] * pose.rotation[6 + axis])
    })
}
