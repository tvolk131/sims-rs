//! Native CPU pose math, before rendering handedness or actor placement.
//! Animation layers and clocks provide the local samples separately.
use crate::head_look::rig::AbsoluteBone;
use serde::{Deserialize, Serialize};
pub mod accessory;
pub mod animator;
pub mod appearance;
pub mod appearance_read;
pub mod appearance_reload;
pub mod appearance_selection;
pub mod clock;
pub mod clothing;
pub mod command;
pub mod construction;
pub mod destruction;
pub mod events;
pub mod geometry;
pub mod graphic_appearance;
pub mod graphic_destruction;
pub mod graphic_frame;
pub mod graphic_initialize;
pub mod graphic_owner;
pub mod graphic_profile;
pub mod graphic_records;
pub mod graphic_reset;
pub mod graphic_transform;
pub mod graphic_visibility;
pub mod initialize;
pub mod layer;
pub mod live;
pub mod material_texture;
pub mod mesh;
pub mod model;
pub mod model_frame;
pub mod model_material;
pub mod model_visibility;
pub mod movement;
pub mod ordinary;
pub mod portrait;
pub mod posture;
pub mod properties;
pub mod reach;
pub mod refresh;
pub mod reset;
pub mod root_motion;
pub mod sampling;
pub mod skeleton;
pub mod transform;
pub mod update;
pub mod vertices;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LocalBone {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
}

/// Original 0x593989. Weight belongs to `a`; the other contribution is `b`.
/// The near-endpoint branches copy exactly without normalizing. Out-of-range
/// weights can leave the destination unchanged, so it is an explicit input.
pub fn blend_quaternion(
    weight: f32,
    a: [f32; 4],
    b: [f32; 4],
    destination: [f32; 4],
) -> Result<[f32; 4], String> {
    blend_quaternion_with_control(weight, a, b, destination, 0x037f)
}
pub fn blend_quaternion_with_control(
    weight: f32,
    a: [f32; 4],
    b: [f32; 4],
    destination: [f32; 4],
    fp_control: u16,
) -> Result<[f32; 4], String> {
    if !matches!(fp_control, 0x003f | 0x037f) {
        return Err("unsupported quaternion floating-point control".into());
    }
    if !weight.is_finite()
        || a.iter()
            .chain(b.iter())
            .chain(destination.iter())
            .any(|v| !v.is_finite())
    {
        return Err("invalid native quaternion blend input".into());
    }
    if fp_control == 0x003f {
        if weight >= 0.99 || weight <= 0.01 {
            return Ok(if (weight - 1.).abs() <= 0.01 {
                a
            } else if weight.abs() <= 0.01 {
                b
            } else {
                destination
            });
        }
        let dot = ((a[0] * b[0] + a[3] * b[3]) + a[2] * b[2]) + a[1] * b[1];
        let other = if dot < 0. { weight - 1. } else { 1. - weight };
        let mut out: [f32; 4] = std::array::from_fn(|i| weight * a[i] + other * b[i]);
        let norm = ((out[0] * out[0] + out[1] * out[1]) + out[2] * out[2]) + out[3] * out[3];
        if (norm - 1.).abs() > 0.00001 {
            if norm.abs() <= 0.00001 {
                out = [0., 0., 0., 1.];
            } else {
                let scale = 1. / norm.sqrt();
                out = out.map(|v| v * scale);
            }
        }
        return Ok(out);
    }
    let weight = f64::from(weight);
    let epsilon = f64::from(0.01_f32);
    if weight >= f64::from(0.99_f32) || weight <= epsilon {
        return Ok(if (weight - 1.).abs() <= epsilon {
            a
        } else if weight.abs() <= epsilon {
            b
        } else {
            destination
        });
    }
    let aa = a.map(f64::from);
    let bb = b.map(f64::from);
    let dot = aa[0] * bb[0] + aa[3] * bb[3] + aa[2] * bb[2] + aa[1] * bb[1];
    let other = if dot < 0. { weight - 1. } else { 1. - weight };
    let mut out: [f32; 4] = std::array::from_fn(|i| (weight * aa[i] + other * bb[i]) as f32);
    let norm = out
        .iter()
        .map(|v| f64::from(*v) * f64::from(*v))
        .sum::<f64>();
    let tolerance = f64::from(0.00001_f32);
    if (norm - 1.).abs() > tolerance {
        if norm.abs() <= tolerance {
            out = [0., 0., 0., 1.];
        } else {
            let scale = 1. / norm.sqrt();
            out = out.map(|v| (f64::from(v) * scale) as f32);
        }
    }
    Ok(out)
}

/// The original hierarchy stores absolute rotations in the opposite quaternion
/// convention from local BCF samples. Neither product nor vector rotation
/// invents normalization; source pose channels have already chosen their values.
pub fn compose_bone(local: LocalBone, parent: AbsoluteBone) -> Result<AbsoluteBone, String> {
    compose_bone_with_control(local, parent, 0x037f)
}
pub fn compose_bone_with_control(
    local: LocalBone,
    parent: AbsoluteBone,
    fp_control: u16,
) -> Result<AbsoluteBone, String> {
    if !matches!(fp_control, 0x003f | 0x037f) {
        return Err("unsupported hierarchy floating-point control".into());
    }
    if local
        .translation
        .iter()
        .chain(local.rotation.iter())
        .chain(parent.translation.iter())
        .chain(parent.rotation.iter())
        .any(|v| !v.is_finite())
    {
        return Err("invalid native hierarchy input".into());
    }
    if fp_control == 0x003f {
        let [x, y, z, w] = local.rotation;
        let [px, py, pz, pw] = parent.rotation;
        let qw = -pw;
        // 590aab..590b18: preserve the actual x87 addition order. The
        // decompiler's regrouped quaternion formula differs under003f.
        let rotation = [
            ((qw * x + w * px) + y * pz) - z * py,
            ((w * py - pz * x) + y * qw) + z * px,
            ((x * py + w * pz) - y * px) + z * qw,
            -(((w * qw - px * x) - y * py) - z * pz),
        ];
        let [x, y, z, w] = parent.rotation;
        let [vx, vy, vz] = local.translation;
        let xy = y * x * 2.;
        let wz = w * z * 2.;
        let wx = w * x * 2.;
        let yz = z * y * 2.;
        let wy = w * y * 2.;
        let xz = z * x * 2.;
        let ww = w * w;
        let xx = x * x;
        let yy = y * y;
        let zz = z * z;
        let v = [
            (((xx + ww - yy) - zz) * vx + (xz + wy) * vz) + (xy - wz) * vy,
            (((ww - xx + yy) - zz) * vy + (yz - wx) * vz) + (wz + xy) * vx,
            (((ww - xx - yy) + zz) * vz + (xz - wy) * vx) + (yz + wx) * vy,
        ];
        let translation = std::array::from_fn(|i| v[i] + parent.translation[i]);
        if rotation
            .iter()
            .chain(translation.iter())
            .any(|v| !v.is_finite())
        {
            return Err("native hierarchy overflow".into());
        }
        return Ok(AbsoluteBone {
            translation,
            rotation,
        });
    }
    let [x, y, z, w] = local.rotation.map(f64::from);
    let [px, py, pz, pw] = parent.rotation.map(f64::from);
    // 0x59615b: local * [parent.xyz, -parent.w], then negate result.w.
    let rotation = [
        (y * pz + w * px - pw * x - z * py) as f32,
        (z * px - y * pw + (w * py - pz * x)) as f32,
        (-z * pw + ((w * pz + x * py) - y * px)) as f32,
        -(((-w * pw - px * x) - y * py) - z * pz) as f32,
    ];
    let rotated = rotate_vector(local.translation, parent.rotation);
    let translation = std::array::from_fn(|i| rotated[i] + parent.translation[i]);
    if rotation
        .iter()
        .chain(translation.iter())
        .any(|v| !v.is_finite())
    {
        return Err("native hierarchy overflow".into());
    }
    Ok(AbsoluteBone {
        translation,
        rotation,
    })
}
fn rotate_vector(v: [f32; 3], q: [f32; 4]) -> [f32; 3] {
    // Original 0x596285's coefficient spills; unlike a unit-quaternion matrix,
    // this keeps w*w and observes unnormalized inputs.
    let [x, y, z, w] = q.map(f64::from);
    let [vx, vy, vz] = v.map(f64::from);
    let spill = |v: f64| f64::from(v as f32);
    let xy = spill(y * x * 2.);
    let wz = spill(w * z * 2.);
    let wx = spill(w * x * 2.);
    let yz = spill(z * y * 2.);
    let wy = spill(w * y * 2.);
    // These three values stay in x87 registers. Rounding them to f32 moves
    // descendants by one or more ULPs after rotated hierarchy composition.
    let xz = z * x * 2.;
    let ww = w * w;
    let xx = x * x;
    let yy = spill(y * y);
    let zz = spill(z * z);
    [
        ((((xx + ww - yy) - zz) * vx + (xz + wy) * vz) + (xy - wz) * vy) as f32,
        ((((ww - xx + yy) - zz) * vy + (yz - wx) * vz) + (wz + xy) * vx) as f32,
        ((((ww - xx - yy) + zz) * vz + (xz - wy) * vx) + (yz + wx) * vy) as f32,
    ]
}

/// Evaluate a decoded skeleton's complete tree in original coordinates, matching
/// the recursive 0x59615b traversal. Inputs are validated before recursion.
pub fn hierarchy(
    parents: &[Option<usize>],
    locals: &[LocalBone],
    base: AbsoluteBone,
) -> Result<Vec<AbsoluteBone>, String> {
    hierarchy_with_control(parents, locals, base, 0x037f)
}
pub fn hierarchy_with_control(
    parents: &[Option<usize>],
    locals: &[LocalBone],
    base: AbsoluteBone,
    fp_control: u16,
) -> Result<Vec<AbsoluteBone>, String> {
    if !matches!(fp_control, 0x003f | 0x037f) {
        return Err("unsupported hierarchy floating-point control".into());
    }
    if parents.len() != locals.len()
        || parents.is_empty()
        || parents.len() > 256
        || parents.iter().filter(|p| p.is_none()).count() != 1
        || parents.iter().flatten().any(|p| *p >= parents.len())
    {
        return Err("invalid native hierarchy shape".into());
    }
    fn resolve(
        i: usize,
        parents: &[Option<usize>],
        locals: &[LocalBone],
        base: AbsoluteBone,
        visiting: &mut [bool],
        out: &mut [Option<AbsoluteBone>],
        fp_control: u16,
    ) -> Result<AbsoluteBone, String> {
        if let Some(p) = out[i] {
            return Ok(p);
        }
        if visiting[i] {
            return Err("cyclic native hierarchy".into());
        }
        visiting[i] = true;
        let parent = if let Some(p) = parents[i] {
            resolve(p, parents, locals, base, visiting, out, fp_control)?
        } else {
            base
        };
        let pose = compose_bone_with_control(locals[i], parent, fp_control)?;
        visiting[i] = false;
        out[i] = Some(pose);
        Ok(pose)
    }
    let mut visiting = vec![false; parents.len()];
    let mut out = vec![None; parents.len()];
    for i in 0..parents.len() {
        resolve(
            i,
            parents,
            locals,
            base,
            &mut visiting,
            &mut out,
            fp_control,
        )?;
    }
    Ok(out.into_iter().map(Option::unwrap).collect())
}
