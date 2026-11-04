//! Pose-channel portion of native 0x597c4c. Clock progression and event dispatch
//! supply the already-updated layer phase; neither is inferred here.
use super::{LocalBone, blend_quaternion_with_control};
use serde::{Deserialize, Serialize};
use sims_formats::{
    animation::{Animation, Motion, Samples},
    avatar::Skeleton,
};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub first: usize,
    pub second: usize,
    pub first_weight: f32,
    pub second_weight: f32,
    pub interpolate: bool,
}
impl Selection {
    /// Original frame selection after clock/loop/fade processing. Only loop
    /// mode 1 interpolates across the last/first frame boundary.
    pub fn new(
        phase: f32,
        frames: u32,
        mode: i32,
        elapsed: f32,
        duration: f32,
        skip: u32,
    ) -> Result<Self, String> {
        Self::new_with_control(phase, frames, mode, elapsed, duration, skip, 0x037f)
    }
    pub fn new_with_control(
        phase: f32,
        frames: u32,
        mode: i32,
        elapsed: f32,
        duration: f32,
        skip: u32,
        fp_control: u16,
    ) -> Result<Self, String> {
        if !matches!(fp_control, 0x003f | 0x037f) {
            return Err("unsupported native sample floating-point control".into());
        }
        if !phase.is_finite() || phase.abs() > 1_000_000. {
            return Err("invalid native sample phase".into());
        }
        Self::at_position(
            if fp_control == 0x003f {
                f64::from(phase * frames as f32)
            } else {
                f64::from(phase) * f64::from(frames)
            },
            frames,
            mode,
            elapsed,
            duration,
            skip,
            fp_control,
        )
    }
    fn at_position(
        position: f64,
        frames: u32,
        mode: i32,
        elapsed: f32,
        duration: f32,
        skip: u32,
        fp_control: u16,
    ) -> Result<Self, String> {
        if !elapsed.is_finite()
            || !duration.is_finite()
            || duration < 0.
            || frames == 0
            || frames > 108_001
            || skip >= 108_001
            || !(0..=3).contains(&mode)
        {
            return Err("invalid native sample selection".into());
        }
        let stride = i64::from(skip) + 1;
        // Native floor receives the extended product, while the fractional
        // weight later reads its single-precision spill.
        let first =
            ((position.floor() as i64 / stride) * stride).clamp(0, i64::from(frames) - 1) as usize;
        let effective = u64::from(frames).div_ceil(stride as u64);
        let interpolate = if fp_control == 0x003f {
            let interval = duration / effective as f32;
            elapsed < interval + interval
        } else {
            f64::from(elapsed) < 2. * f64::from(duration) / effective as f64
        };
        let second = if interpolate {
            let next = first + stride as usize;
            if next < frames as usize {
                next
            } else if mode == 1 {
                0
            } else {
                first
            }
        } else {
            first
        };
        let second_weight = if !interpolate {
            0.
        } else if skip == 0 {
            position as f32 - first as f32
        } else {
            let distance = if first < second {
                second - first
            } else {
                frames as usize - first
            };
            ((f64::from(position as f32) - first as f64) / distance as f64) as f32
        };
        Ok(Self {
            first,
            second,
            first_weight: 1. - second_weight,
            second_weight,
            interpolate,
        })
    }
}

/// Values established by the enclosing native layer pass. The channel sampler
/// reads these values, but does not choose priorities or update accumulated
/// bone markers on behalf of that pass.
#[derive(Debug, Clone, Copy)]
pub struct Layer {
    pub weight: f32,
    pub priority: i32,
    pub translation_policy: i32,
    pub rotation_policy: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoneInput {
    pub local: LocalBone,
    pub priority: i32,
    /// Native bone +0x74: third BCF bone flag (595947). Supplied by the rig.
    pub blend_marker: u32,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneOutput {
    pub local: LocalBone,
    pub translation_changed: bool,
    pub rotation_changed: bool,
}

/// One already-bound bone/motion pair, including original layer masking,
/// endpoint cutoffs and unchanged channels. Missing samples are errors.
pub fn motion(
    bone: BoneInput,
    layer: Layer,
    selection: Selection,
    motion: &Motion,
    samples: &Samples,
) -> Result<BoneOutput, String> {
    motion_with_control(bone, layer, selection, motion, samples, 0x037f)
}
pub fn motion_with_control(
    bone: BoneInput,
    layer: Layer,
    selection: Selection,
    motion: &Motion,
    samples: &Samples,
    fp_control: u16,
) -> Result<BoneOutput, String> {
    if !matches!(fp_control, 0x003f | 0x037f) {
        return Err("unsupported native pose floating-point control".into());
    }
    if !layer.weight.is_finite()
        || !selection.first_weight.is_finite()
        || !selection.second_weight.is_finite()
        || bone
            .local
            .translation
            .iter()
            .chain(bone.local.rotation.iter())
            .any(|v| !v.is_finite())
    {
        return Err("invalid native pose channel input".into());
    }
    let mut out = BoneOutput {
        local: bone.local,
        translation_changed: false,
        rotation_changed: false,
    };
    if layer.weight <= 0. || bone.priority > layer.priority {
        return Ok(out);
    }
    if let Some(start) = motion
        .translation
        .filter(|_| bone.blend_marker != 0 || layer.translation_policy != -1)
    {
        let get = |frame: usize| {
            samples
                .translations
                .get(
                    (start as usize)
                        .checked_add(frame)
                        .ok_or("native translation index overflow")?,
                )
                .copied()
                .ok_or("missing native translation sample")
        };
        let a = get(selection.first)?;
        let b = if selection.interpolate {
            get(selection.second)?
        } else {
            a
        };
        let mut sample = std::array::from_fn(|i| {
            if selection.interpolate {
                if fp_control == 0x003f {
                    f64::from(a[i] * selection.first_weight + b[i] * selection.second_weight)
                } else {
                    f64::from(a[i]) * f64::from(selection.first_weight)
                        + f64::from(b[i]) * f64::from(selection.second_weight)
                }
            } else {
                f64::from(a[i])
            }
        });
        if layer.weight < 1. && bone.blend_marker != 0 {
            // In the interpolated branch X/Y are spilled before the layer
            // blend, but Z remains on the x87 stack.
            if selection.interpolate {
                sample[0] = f64::from(sample[0] as f32);
                sample[1] = f64::from(sample[1] as f32);
            }
            let other = f64::from(1. - layer.weight);
            for (i, v) in sample.iter_mut().enumerate() {
                *v = if fp_control == 0x003f {
                    f64::from(*v as f32 * layer.weight + bone.local.translation[i] * other as f32)
                } else {
                    *v * f64::from(layer.weight) + f64::from(bone.local.translation[i]) * other
                };
            }
        }
        out.local.translation = sample.map(|v| v as f32);
        out.translation_changed = true;
    }
    if let Some(start) = motion
        .rotation
        .filter(|_| bone.blend_marker != 0 || layer.rotation_policy != -1)
    {
        let get = |frame: usize| {
            samples
                .rotations
                .get(
                    (start as usize)
                        .checked_add(frame)
                        .ok_or("native rotation index overflow")?,
                )
                .copied()
                .ok_or("missing native rotation sample")
        };
        let a = get(selection.first)?;
        let mut sample = a;
        if selection.interpolate {
            let b = get(selection.second)?;
            // The original retains A when its weight is <= .01, rather than
            // copying B. This outer branch precedes the quaternion helper.
            if selection.first_weight > 0.01 && a != b {
                sample =
                    blend_quaternion_with_control(selection.first_weight, a, b, a, fp_control)?;
            }
        }
        if layer.weight >= 1. || bone.blend_marker == 0 {
            out.local.rotation = sample;
            out.rotation_changed = true;
        } else if layer.weight > 0.01 && sample != bone.local.rotation {
            out.local.rotation = blend_quaternion_with_control(
                layer.weight,
                sample,
                bone.local.rotation,
                [0., 0., 0., 1.],
                fp_control,
            )?;
            out.rotation_changed = true;
        }
    }
    if out
        .local
        .translation
        .iter()
        .chain(out.local.rotation.iter())
        .any(|v| !v.is_finite())
    {
        return Err("native pose sample overflow".into());
    }
    Ok(out)
}

/// A full-weight clip over the decoded rest pose. Bone binding is exact-case,
/// as in the original string lookup. This is also usable without a GPU.
pub fn locals(
    skeleton: &Skeleton,
    clip: &Animation,
    samples: &Samples,
    selection: Selection,
) -> Result<Vec<LocalBone>, String> {
    skeleton
        .bones
        .iter()
        .map(|b| {
            let local = LocalBone {
                translation: b.translation,
                rotation: b.rotation,
            };
            let Some(m) = clip.motions.iter().find(|m| m.bone == b.name) else {
                return Ok(local);
            };
            Ok(motion(
                BoneInput {
                    local,
                    priority: 0,
                    blend_marker: 0,
                },
                Layer {
                    weight: 1.,
                    priority: 0,
                    translation_policy: 0,
                    rotation_policy: 0,
                },
                selection,
                m,
                samples,
            )?
            .local)
        })
        .collect()
}

/// Presentation adapter for existing frame cursors. It does not change their
/// clock model. Native runtime callers use Selection::new with their own phase,
/// elapsed time, duration and frame-skip state.
pub fn at_frame(
    skeleton: &Skeleton,
    clip: &Animation,
    samples: &Samples,
    frame: f32,
    looping: bool,
) -> Result<Vec<LocalBone>, String> {
    if !frame.is_finite()
        || frame < 0.
        || clip.frames == 0
        || clip
            .motions
            .iter()
            .any(|m| m.frames == 0 && (m.translation.is_some() || m.rotation.is_some()))
    {
        return Err("invalid animation frame".into());
    }
    let frame = if looping {
        frame % clip.frames as f32
    } else {
        frame.min((clip.frames - 1) as f32)
    };
    // Keep the supplied frame position. A frame->phase->frame roundtrip can
    // move an integer endpoint into the previous frame's native cutoff branch.
    let selection = Selection::at_position(
        f64::from(frame),
        clip.frames,
        i32::from(looping),
        0.,
        clip.duration_ms,
        0,
        0x037f,
    )?;
    locals(skeleton, clip, samples, selection)
}
