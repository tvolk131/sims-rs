//! A skeleton posed by one clip at one phase, in the skeleton's own space.
//!
//! The simulation needs a pose in one place: a person's constructor
//! (0x58831d) poses the skeleton with the walk clip at each of the walk's
//! two footfalls and measures how far apart the feet are (the strides,
//! animator +0x178 and +0x17c), which the end of a path reads (0x58e8be).
//! It plays the walk clip as the only layer, at weight 1 (0x5885dc), with
//! the skeleton's root at the origin with no rotation.
//!
//! The game runs the x87 FPU at 24-bit precision, so every operation
//! rounds to single precision ([`Ext::single`]). The order of operations
//! follows the old prototype's 24-bit variants, which it compared in
//! isolation with the original's code:
//!
//! - choosing the frames ([`Selection::at`], from `Selection::at_position`
//!   in legacy/crates/sims-sim/src/pose/sampling.rs, the frame choice of
//!   0x597c4c);
//! - sampling a motion at full weight ([`sample`], from
//!   `motion_with_control` in the same file);
//! - blending two rotations ([`blend`], 0x593989, from
//!   `blend_quaternion_with_control` in legacy/crates/sims-sim/src/pose.rs);
//! - composing a bone with its parent ([`compose`], 0x59615b and 0x596285,
//!   from `compose_bone_with_control` in the same file).
//!
//! Whether the whole matches the original is checked against the strides
//! the original stored (see `crates/sims-vm/tests/integration/resize.rs`).

use sims_core::{Ext, F32};

/// Where a bone is and how it is turned, relative to its parent or, once
/// composed, to the skeleton's root.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bone {
    pub translation: [F32; 3],
    /// A quaternion, x, y, z then w.
    pub rotation: [F32; 4],
}

/// The identity: no offset and no turn.
pub const ORIGIN: Bone = Bone {
    translation: [F32::ZERO; 3],
    rotation: [F32::ZERO, F32::ZERO, F32::ZERO, F32::from_bits(0x3f80_0000)],
};

/// A pose that cannot be worked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

/// The two frames a phase falls between and the weight of each.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    pub first: usize,
    pub second: usize,
    pub first_weight: F32,
    pub second_weight: F32,
    pub interpolate: bool,
}

/// 0.01, below which a weight counts as none (0x593989).
const SMALL: F32 = F32::from_bits(0x3c23_d70a);
/// 0.99, above which a weight counts as whole (0x593989).
const LARGE: F32 = F32::from_bits(0x3f7d_70a4);
/// 0.00001, how far from 1 a rotation's squared length may be before it
/// is scaled back to 1 (0x593989).
const NORMAL: F32 = F32::from_bits(0x3727_c5ac);

fn s(value: Ext) -> Ext {
    value.single()
}

fn one() -> Ext {
    Ext::from_i32(1)
}

impl Selection {
    /// The frames at `phase` (0 to 1) of a clip of `frames` frames and
    /// `duration` milliseconds, with no frames skipped, `elapsed`
    /// milliseconds after the previous pose. The frame is the whole part
    /// of phase times frames. The next frame is blended in when the time
    /// since the previous pose is under two frames' time; after the last
    /// frame comes the first for a looping clip (`looping`), else the last
    /// again.
    pub fn at(
        phase: F32,
        frames: u32,
        duration: F32,
        elapsed: F32,
        looping: bool,
    ) -> Result<Self, Error> {
        let count = i32::try_from(frames)
            .ok()
            .filter(|&n| n > 0)
            .ok_or_else(|| Error(format!("a clip of {frames} frames")))?;
        let position = s(phase.ext() * Ext::from_i32(count));
        let floor = position.floor();
        if floor.is_nan() {
            return Err(Error("the phase is not a number".into()));
        }
        let first = floor.to_i32_truncate().clamp(0, count - 1);
        let interval = s(duration.ext() / Ext::from_i32(count));
        let interpolate = elapsed.ext() < s(interval + interval);
        let second = if !interpolate {
            first
        } else if first + 1 < count {
            first + 1
        } else if looping {
            0
        } else {
            first
        };
        let second_weight = if interpolate {
            s(position - Ext::from_i32(first))
        } else {
            Ext::ZERO
        };
        Ok(Self {
            first: first as usize,
            second: second as usize,
            first_weight: s(one() - second_weight).to_f32(),
            second_weight: second_weight.to_f32(),
            interpolate,
        })
    }
}

/// One motion's samples: a bone's positions and rotations, each from the
/// motion's first frame on, or None for a motion without them.
#[derive(Debug, Clone, Copy)]
pub struct Motion<'a> {
    pub translations: Option<&'a [[F32; 3]]>,
    pub rotations: Option<&'a [[F32; 4]]>,
}

fn frame<T: Copy>(samples: &[T], index: usize) -> Result<T, Error> {
    samples
        .get(index)
        .copied()
        .ok_or_else(|| Error(format!("no sample for frame {index}")))
}

/// A bone's pose from one motion at full weight: each channel the motion
/// has replaces the bone's, blended between the selection's two frames;
/// a channel it lacks is left as it was.
pub fn sample(bone: &mut Bone, motion: Motion<'_>, at: Selection) -> Result<(), Error> {
    if let Some(samples) = motion.translations {
        let a = frame(samples, at.first)?;
        bone.translation = if at.interpolate {
            let b = frame(samples, at.second)?;
            std::array::from_fn(|i| {
                s(s(a[i].ext() * at.first_weight.ext()) + s(b[i].ext() * at.second_weight.ext()))
                    .to_f32()
            })
        } else {
            a
        };
    }
    if let Some(samples) = motion.rotations {
        let a = frame(samples, at.first)?;
        let mut rotation = a;
        if at.interpolate {
            let b = frame(samples, at.second)?;
            let differ = a.iter().zip(&b).any(|(x, y)| x.ext() != y.ext());
            if at.first_weight.ext() > SMALL.ext() && differ {
                rotation = blend(at.first_weight, a, b, a);
            }
        }
        bone.rotation = rotation;
    }
    Ok(())
}

/// A bone's pose from one motion of a layer at `weight` (0x598226 to
/// 0x5985f9): the motion's sample ([`sample`]) replaces what the bone
/// holds when the weight is 1 or more or the bone does
/// not blend (`blends`, native bone +0x74). Otherwise each channel the
/// motion has blends with what the bone holds:
///
/// - the translation becomes, coordinate by coordinate, the sample times
///   the weight plus what it held times 1 minus the weight;
/// - the rotation is left as it is for a weight of 0.01 or less, or when
///   the sample is the same as what it holds; otherwise it becomes the
///   blend of the sample, at the weight, with what it holds ([`blend`],
///   whose result in the bounds is the identity, [`ORIGIN`]).
///
/// A layer at weight 0 or below is not sampled at all (0x597c4c).
pub fn sample_weighted(
    bone: &mut Bone,
    motion: Motion<'_>,
    at: Selection,
    weight: F32,
    blends: bool,
) -> Result<(), Error> {
    let mut sampled = *bone;
    sample(&mut sampled, motion, at)?;
    let w = weight.ext();
    // 0x5982b2: `w >= 1.0`, an x87 compare with a double; not a number
    // compares unordered and blends.
    let whole = w >= one();
    if whole || !blends {
        *bone = sampled;
        return Ok(());
    }
    if motion.translations.is_some() {
        let rest = s(one() - w);
        bone.translation = std::array::from_fn(|i| {
            s(s(sampled.translation[i].ext() * w) + s(bone.translation[i].ext() * rest)).to_f32()
        });
    }
    if motion.rotations.is_some() {
        let differ = sampled
            .rotation
            .iter()
            .zip(&bone.rotation)
            .any(|(x, y)| x.ext() != y.ext());
        if w > SMALL.ext() && differ {
            bone.rotation = blend(weight, sampled.rotation, bone.rotation, ORIGIN.rotation);
        }
    }
    Ok(())
}

/// Blends rotation `a`, at `weight`, with `b` (0x593989): a weight within
/// 0.01 of 1 gives `a`, within 0.01 of 0 gives `b`, and any other weight
/// at or beyond those bounds leaves `destination`. Otherwise the sum is
/// taken along the shorter way round and scaled to length 1, unless its
/// squared length is already within 0.00001 of 1 (or of 0, which gives
/// no turn).
pub fn blend(weight: F32, a: [F32; 4], b: [F32; 4], destination: [F32; 4]) -> [F32; 4] {
    let w = weight.ext();
    if w >= LARGE.ext() || w <= SMALL.ext() {
        return if s(w - one()).abs() <= SMALL.ext() {
            a
        } else if w.abs() <= SMALL.ext() {
            b
        } else {
            destination
        };
    }
    let (a, b) = (a.map(F32::ext), b.map(F32::ext));
    let dot = s(s(s(s(a[0] * b[0]) + s(a[3] * b[3])) + s(a[2] * b[2])) + s(a[1] * b[1]));
    let other = if dot < Ext::ZERO {
        s(w - one())
    } else {
        s(one() - w)
    };
    let out: [Ext; 4] = std::array::from_fn(|i| s(s(w * a[i]) + s(other * b[i])));
    let norm =
        s(s(s(s(out[0] * out[0]) + s(out[1] * out[1])) + s(out[2] * out[2])) + s(out[3] * out[3]));
    if s(norm - one()).abs() <= NORMAL.ext() {
        return out.map(Ext::to_f32);
    }
    if norm.abs() <= NORMAL.ext() {
        return ORIGIN.rotation;
    }
    let scale = s(one() / s(norm.sqrt()));
    out.map(|v| s(v * scale).to_f32())
}

/// A bone placed by its parent's placement (0x59615b): the rotations
/// multiplied, and the bone's offset turned by the parent's rotation
/// (0x596285) and added to the parent's position.
pub fn compose(local: Bone, parent: Bone) -> Bone {
    let [x, y, z, w] = local.rotation.map(F32::ext);
    let [px, py, pz, pw] = parent.rotation.map(F32::ext);
    let qw = -pw;
    let rotation = [
        s(s(s(s(qw * x) + s(w * px)) + s(y * pz)) - s(z * py)),
        s(s(s(s(w * py) - s(pz * x)) + s(y * qw)) + s(z * px)),
        s(s(s(s(x * py) + s(w * pz)) - s(y * px)) + s(z * qw)),
        -s(s(s(s(w * qw) - s(px * x)) - s(y * py)) - s(z * pz)),
    ];
    let [x, y, z, w] = parent.rotation.map(F32::ext);
    let [vx, vy, vz] = local.translation.map(F32::ext);
    let two = Ext::from_i32(2);
    let xy = s(s(y * x) * two);
    let wz = s(s(w * z) * two);
    let wx = s(s(w * x) * two);
    let yz = s(s(z * y) * two);
    let wy = s(s(w * y) * two);
    let xz = s(s(z * x) * two);
    let (ww, xx, yy, zz) = (s(w * w), s(x * x), s(y * y), s(z * z));
    let turned = [
        s(s(s(s(s(xx + ww) - yy) - zz) * vx) + s(s(xz + wy) * vz)) + s(s(xy - wz) * vy),
        s(s(s(s(s(ww - xx) + yy) - zz) * vy) + s(s(yz - wx) * vz)) + s(s(wz + xy) * vx),
        s(s(s(s(s(ww - xx) - yy) + zz) * vz) + s(s(xz - wy) * vx)) + s(s(yz + wx) * vy),
    ];
    Bone {
        translation: std::array::from_fn(|i| {
            s(s(turned[i]) + parent.translation[i].ext()).to_f32()
        }),
        rotation: rotation.map(Ext::to_f32),
    }
}

/// Every bone placed relative to the root, which stands at [`ORIGIN`].
/// `parents` gives each bone's parent by place; a parent must come before
/// its children, as in a skeleton file.
pub fn absolute(parents: &[Option<usize>], locals: &[Bone]) -> Result<Vec<Bone>, Error> {
    if parents.len() != locals.len() {
        return Err(Error("a parent for every bone".into()));
    }
    let mut out: Vec<Bone> = Vec::with_capacity(locals.len());
    for (i, (parent, local)) in parents.iter().zip(locals).enumerate() {
        let base = match parent {
            None => ORIGIN,
            Some(p) => *out
                .get(*p)
                .filter(|_| *p < i)
                .ok_or_else(|| Error(format!("bone {i} comes before its parent {p}")))?,
        };
        out.push(compose(*local, base));
    }
    Ok(out)
}
