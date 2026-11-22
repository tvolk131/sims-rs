//! Animation playback: a clip's samples applied to a skeleton's pose.
//!
//! A clip (`sims_formats::animation::Animation`) has one motion per bone
//! it moves. A motion points into the clip's sample file (`<skill>.cfp`)
//! for its translations and rotations, one per frame. Playing a clip at a
//! phase (0 at its start, 1 at its end) picks the frame `phase * frames`,
//! blends it with the next one, and replaces the moved bones' places
//! relative to their parents. This is the old prototype's sampling
//! (legacy/crates/sims-sim/src/pose/sampling.rs, `Selection` and
//! `locals`) without its bit-exact x87 rules, which only matter for a
//! pose the simulation reads; drawing reads none of it.
//!
//! The simulation decides which clip plays and at which phase
//! (`sims_sim::animator::Layer`); this module only draws it.

use crate::assets::Assets;
use crate::rig::{self, Pose};
use sims_content::ArchivePool;
use sims_formats::animation::{Animation, Samples, read_bcf, read_cfp};
use sims_formats::avatar::Skeleton;
use std::collections::BTreeMap;

/// Every clip of an install's animation files, by lowercase name.
#[derive(Debug, Clone, Default)]
pub struct Library {
    clips: BTreeMap<String, Animation>,
}

impl Library {
    /// Reads every animation file (`.bcf` files whose skeleton and
    /// appearance counts are 0). A file that does not decode is skipped.
    /// A name that two files define takes the first file in name order.
    /// The original keeps the first clip of a name it loads (0x59f087
    /// releases a later one), loading `GameData/Animation` before the
    /// expansions' folders. No clip name is defined by two files in the
    /// Complete Collection, so the two orders never disagree there. Each
    /// file's bytes are the first source's ([`Assets::read`]); which copy
    /// the original reads when two sources differ is **unverified**.
    pub fn scan(assets: &Assets) -> Result<Self, String> {
        Self::scan_with_pool(assets, &ArchivePool::new())
    }

    /// Like [`Library::scan`], reusing open archive handles from `pool` and
    /// decoding animation `.bcf` files in parallel while inserting clips in
    /// exact sorted file-name order.
    pub fn scan_with_pool(assets: &Assets, pool: &ArchivePool) -> Result<Self, String> {
        let names: Vec<&str> = assets.names().filter(|n| n.ends_with(".bcf")).collect();
        let decode_one = |name: &str| -> Result<Vec<Animation>, String> {
            let resource = assets
                .find(name)
                .ok_or_else(|| format!("no person file is named {name:?}"))?;
            let head = resource
                .read_head_with(pool, 8)
                .map_err(|e| e.to_string())?;
            if head.len() < 8 || head != [0; 8] {
                return Ok(Vec::new());
            }
            let bytes = resource.read_with(pool).map_err(|e| e.to_string())?;
            let Ok(animations) = read_bcf(&bytes) else {
                return Ok(Vec::new());
            };
            Ok(animations)
        };
        let workers = if names.len() <= 1 {
            1
        } else {
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
                .clamp(1, 8)
                .min(names.len())
        };
        let decoded: Vec<Result<Vec<Animation>, String>> = if workers <= 1 {
            names.iter().map(|&name| decode_one(name)).collect()
        } else {
            let chunk_size = names.len().div_ceil(workers);
            std::thread::scope(|scope| {
                let handles: Vec<_> = names
                    .chunks(chunk_size)
                    .map(|chunk| {
                        scope.spawn(move || {
                            chunk
                                .iter()
                                .map(|&name| decode_one(name))
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                let mut out = Vec::with_capacity(names.len());
                for handle in handles {
                    out.extend(handle.join().expect("people animation worker panicked"));
                }
                out
            })
        };
        let mut library = Self::default();
        for batch in decoded {
            for animation in batch? {
                library
                    .clips
                    .entry(animation.name.to_ascii_lowercase())
                    .or_insert(animation);
            }
        }
        Ok(library)
    }

    /// How many clips are known.
    pub fn len(&self) -> usize {
        self.clips.len()
    }

    /// Whether no clip is known.
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// The names of every clip, in lowercase.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.clips.keys().map(String::as_str)
    }

    /// The clip named `name`, ignoring ASCII case, with its samples.
    pub fn load(&self, assets: &Assets, name: &str) -> Result<Clip, String> {
        let animation = self
            .clips
            .get(&name.to_ascii_lowercase())
            .ok_or_else(|| format!("no animation file defines the clip {name:?}"))?
            .clone();
        let file = format!("{}.cfp", animation.skill);
        let samples = read_cfp(
            &assets.read(&file)?,
            animation.translation_count,
            animation.rotation_count,
        )
        .map_err(|e| format!("{file}: {e}"))?;
        Ok(Clip { animation, samples })
    }
}

/// A clip with its samples.
#[derive(Debug, Clone)]
pub struct Clip {
    pub animation: Animation,
    pub samples: Samples,
}

impl Clip {
    /// The clip's name.
    pub fn name(&self) -> &str {
        &self.animation.name
    }

    /// Poses the bones the clip moves, at `phase` (0 to 1), `weight` of
    /// the way from where they are (1 replaces them). With `repeat` the
    /// last frame blends into the first, as a clip that repeats does;
    /// otherwise it holds. A motion whose bone the skeleton lacks is
    /// skipped, as the original binds only the bones it finds.
    pub fn apply(
        &self,
        skeleton: &Skeleton,
        pose: &mut Pose,
        phase: f32,
        repeat: bool,
        weight: f32,
    ) {
        self.apply_with_interpolation(skeleton, pose, phase, repeat, true, weight);
    }

    /// Like [`Clip::apply`], honoring `cAnimLayer::Update`'s per-layer
    /// `interpolate` flag (`0x597f1b`–`0x597f38`, `0x59844d`–`0x5985f8`):
    /// when `interpolate` is `false`, each motion snaps directly to
    /// `floor(phase * frames)` without blending toward the next keyframe
    /// (still blending by `weight` over the existing pose when `weight < 1`).
    pub fn apply_with_interpolation(
        &self,
        skeleton: &Skeleton,
        pose: &mut Pose,
        phase: f32,
        repeat: bool,
        interpolate: bool,
        weight: f32,
    ) {
        let weight = if weight.is_finite() {
            weight.clamp(0.0, 1.0)
        } else {
            1.0
        };
        if weight == 0.0 {
            return;
        }
        for motion in &self.animation.motions {
            let Some(bone) = rig::bone(skeleton, &motion.bone) else {
                continue;
            };
            if motion.frames == 0 {
                continue;
            }
            let frames = motion.frames as usize;
            let position = if phase.is_finite() {
                phase.clamp(0.0, 1.0) * frames as f32
            } else {
                0.0
            };
            let first = (position.floor() as usize).min(frames - 1);
            let (second, blend) = if interpolate {
                let second = if first + 1 < frames {
                    first + 1
                } else if repeat {
                    0
                } else {
                    first
                };
                let blend = (position - first as f32).clamp(0.0, 1.0);
                (second, blend)
            } else {
                (first, 0.0)
            };
            let local = &mut pose.local[bone];
            if let Some(start) = motion.translation {
                let at = |f: usize| self.samples.translations.get(start as usize + f).copied();
                if let (Some(a), Some(b)) = (at(first), at(second)) {
                    let t = if interpolate {
                        rig::position(a).lerp(rig::position(b), blend)
                    } else {
                        rig::position(a)
                    };
                    local.translation = if weight < 1.0 {
                        local.translation.lerp(t, weight)
                    } else {
                        t
                    };
                }
            }
            if let Some(start) = motion.rotation {
                let at = |f: usize| self.samples.rotations.get(start as usize + f).copied();
                if let (Some(a), Some(b)) = (at(first), at(second)) {
                    let r = if interpolate {
                        rig::rotation(a).nlerp(rig::rotation(b), blend)
                    } else {
                        rig::rotation(a)
                    };
                    local.rotation = if weight < 1.0 {
                        local.rotation.nlerp(r, weight)
                    } else {
                        r
                    };
                }
            }
        }
    }

    /// The pose of `skeleton` with this clip alone at `phase`, over the
    /// rest pose.
    pub fn pose(&self, skeleton: &Skeleton, phase: f32, repeat: bool) -> Pose {
        let mut pose = Pose::rest(skeleton);
        self.apply(skeleton, &mut pose, phase, repeat, 1.0);
        pose
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;
    use sims_formats::animation::Motion;

    fn clip() -> Clip {
        Clip {
            animation: Animation {
                name: "test".into(),
                skill: "test".into(),
                duration_ms: 1000.0,
                distance: 0.0,
                moving: false,
                translation_count: 2,
                rotation_count: 0,
                frames: 2,
                motions: vec![Motion {
                    bone: "CHILD".into(),
                    frames: 2,
                    duration_ms: 1000.0,
                    translation: Some(0),
                    rotation: None,
                    properties: Vec::new(),
                    event_lists: Vec::new(),
                }],
                events: Vec::new(),
            },
            samples: Samples {
                translations: vec![[0.0, 1.0, 0.0], [0.0, 3.0, 0.0]],
                rotations: Vec::new(),
            },
        }
    }

    #[test]
    fn a_clip_blends_between_frames() {
        let skeleton = rig::tests::skeleton();
        let clip = clip();
        let at = |phase, repeat| clip.pose(&skeleton, phase, repeat).local[1].translation;
        assert_eq!(at(0.0, false), Vec3::new(0.0, 1.0, 0.0));
        // Halfway into the first of two frames: a quarter of the clip.
        assert_eq!(at(0.25, false), Vec3::new(0.0, 2.0, 0.0));
        // Past the last frame a repeating clip blends back to the first.
        assert_eq!(at(0.75, true), Vec3::new(0.0, 2.0, 0.0));
        assert_eq!(at(0.75, false), Vec3::new(0.0, 3.0, 0.0));
        // The root is not moved by the clip.
        assert_eq!(
            clip.pose(&skeleton, 0.5, true).local[0],
            Pose::rest(&skeleton).local[0]
        );
    }

    #[test]
    fn a_weight_blends_from_the_pose_below() {
        let skeleton = rig::tests::skeleton();
        let clip = clip();
        // Halfway through a clip that does not repeat, the child bone is
        // at the last frame, (0, 3, 0); at rest it is at (0, 1, 0).
        let below = Pose::rest(&skeleton).local[1].translation;
        assert_eq!(below, Vec3::new(0.0, 1.0, 0.0));
        let at = |weight| {
            let mut pose = Pose::rest(&skeleton);
            clip.apply(&skeleton, &mut pose, 0.5, false, weight);
            pose.local[1].translation
        };
        assert_eq!(at(0.0), below);
        assert_eq!(at(0.5), Vec3::new(0.0, 2.0, 0.0));
        assert_eq!(at(1.0), Vec3::new(0.0, 3.0, 0.0));
        // A weight outside 0..=1 is clamped; one that is not a number
        // counts as 1.
        assert_eq!(at(2.0), Vec3::new(0.0, 3.0, 0.0));
        assert_eq!(at(-1.0), below);
        assert_eq!(at(f32::NAN), Vec3::new(0.0, 3.0, 0.0));
    }

    #[test]
    fn apply_with_interpolation_false_snaps_to_floor_and_keeps_weight_blend() {
        let skeleton = rig::tests::skeleton();
        let mut clip = clip();
        clip.animation.rotation_count = 2;
        clip.animation.motions[0].rotation = Some(0);
        let q0 = [0.0, 0.0, 0.0, 1.0];
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let q1 = [0.0, half, 0.0, half];
        clip.samples.rotations = vec![q0, q1];

        // Phase 0.25 -> position 0.5 (halfway between frame 0 and frame 1).
        let mut snapped = Pose::rest(&skeleton);
        clip.apply_with_interpolation(&skeleton, &mut snapped, 0.25, false, false, 1.0);
        assert_eq!(snapped.local[1].translation, Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(snapped.local[1].rotation, rig::rotation(q0));

        // Phase 0.75 on a repeating clip -> position 1.5; without interpolation
        // it snaps to frame 1 instead of wrapping back toward frame 0.
        let mut snapped_repeat = Pose::rest(&skeleton);
        clip.apply_with_interpolation(&skeleton, &mut snapped_repeat, 0.75, true, false, 1.0);
        assert_eq!(
            snapped_repeat.local[1].translation,
            Vec3::new(0.0, 3.0, 0.0)
        );
        assert_eq!(snapped_repeat.local[1].rotation, rig::rotation(q1));

        // Weight < 1 still blends the snapped keyframe over the pose below.
        let mut weighted = Pose::rest(&skeleton);
        clip.apply_with_interpolation(&skeleton, &mut weighted, 0.75, true, false, 0.5);
        assert_eq!(weighted.local[1].translation, Vec3::new(0.0, 2.0, 0.0));
        assert_ne!(weighted.local[1].rotation, rig::rotation(q0));
        assert_ne!(weighted.local[1].rotation, rig::rotation(q1));
    }
}
