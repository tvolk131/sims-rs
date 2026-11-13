//! Fresh skeleton ownership from 598da3/598f20/59894b/599028. Prototype assets
//! stay immutable. This is the model-binding input to 58831d, before its Reset,
//! resource selections and walking measurements, not a fully activated person.
use super::{LocalBone, animator::Animator, live::Live, refresh::Hierarchy, sampling::BoneInput};
use crate::head_look::rig::AbsoluteBone;
use serde::{Deserialize, Serialize};
use sims_formats::avatar::{Skeleton, Wiggle};

/// 5a06d4 reserves the case-insensitive name `root`: no catalog lookup occurs.
/// This describes its built-in metadata for mesh binding, not a disk asset.
pub fn root_source() -> Skeleton {
    Skeleton {
        name: "root".into(),
        bones: vec![sims_formats::avatar::Bone {
            name: "ROOT".into(),
            parent: None,
            translation: [0.; 3],
            rotation: [0., 0., 0., 1.],
            properties: vec![],
            flags: [1; 3],
            wiggle: Wiggle {
                gradient_scale: 1,
                amplitude: 0.,
            },
        }],
    }
}

/// Native +0xe8 noise state. Cloning retains this even when the amplitude is
/// zero; construction consumes the process CRT stream in either case. Applying
/// procedural wiggle during animation remains a separate evaluation step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WiggleState {
    pub gradient_scale: i32,
    pub amplitude: f32,
    /// Four native records, each containing two coordinates and two gradients.
    pub records: [[i32; 4]; 4],
}
impl WiggleState {
    fn new(parameters: Wiggle, crt: &mut u32) -> Self {
        fn word(crt: &mut u32) -> i32 {
            ((sims_vm::rng::next_crt(crt) << 16) ^ sims_vm::rng::next_crt(crt)) as i32
        }
        fn gradient(record: &mut [i32; 4], scale: i32, crt: &mut u32) {
            // 595d9c/595de8: normalize two signed integers, store the scale as
            // f32, then floor(component + 0.5). Intermediate products stay in
            // x87 registers; do not round the squared length to f32.
            let x = word(crt) >> 8;
            let y = word(crt) >> 8;
            let squared = f64::from(x).powi(2) + f64::from(y).powi(2);
            [record[2], record[3]] = if squared > 0. {
                let factor = (f64::from(scale) / squared.sqrt()) as f32;
                [x, y].map(|v| (f64::from(factor) * f64::from(v) + 0.5).floor() as i64 as i32)
            } else {
                [x, y]
            };
        }
        let mut records = [[0; 4]; 4];
        // 598a6e initializes each complete record before moving to the next.
        for record in &mut records {
            record[0] = word(crt);
            record[1] = word(crt);
            gradient(record, 1, crt);
        }
        // 595d65 compares integer words. Copying the constructor's scale 1
        // consumes no extra draws; every other scale consumes sixteen.
        if parameters.gradient_scale != 1 {
            for record in &mut records {
                gradient(record, parameters.gradient_scale, crt);
            }
        }
        Self {
            gradient_scale: parameters.gradient_scale,
            amplitude: parameters.amplitude,
            records,
        }
    }
}

/// Resource-owned noise constructed by 598af7/595947, before any instance
/// clone. The clone independently constructs its bones; it does not copy these
/// random records. Keep this owner alive for the lifetime of the loaded catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    pub source: Skeleton,
    pub wiggle: Vec<WiggleState>,
}
impl Resource {
    pub fn read(source: Skeleton, crt: &mut u32) -> Result<Self, String> {
        if source.bones.is_empty()
            || source.bones.len() > 256
            || source.bones.iter().any(|b| !b.wiggle.amplitude.is_finite())
        {
            return Err("unsupported skeleton resource bones".into());
        }
        let wiggle = source
            .bones
            .iter()
            .map(|b| WiggleState::new(b.wiggle, crt))
            .collect();
        Ok(Self { source, wiggle })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    pub dirty: bool,
    pub generation: u32,
    pub evaluated_generation: u32,
    pub bounds_generation: u32,
    pub bone_generations: Vec<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub skin: Live,
    pub hierarchy: Hierarchy,
    pub geometry: Geometry,
    pub wiggle: Vec<WiggleState>,
    pub flags: Vec<[u32; 3]>,
    /// Native +0x12c classification from 595bf4, copied by 598da3.
    pub named_bone_flags: Vec<bool>,
}
impl Instance {
    /// 5a06d4's 598f20/59894b/599028 built-in branch. The bone keeps constructor
    /// scale 1 (32 CRT draws) and is explicitly marked named, unlike a cloned
    /// catalog ROOT. Its unpublished pose/cache stamps also differ from clones.
    pub fn root(crt: &mut u32) -> Self {
        let bone = root_source().bones.remove(0);
        let local = LocalBone {
            translation: bone.translation,
            rotation: bone.rotation,
        };
        Self {
            skin: Live::new(
                Animator::new(-1, 0, 0, true, -1),
                vec![BoneInput {
                    local,
                    priority: 0,
                    blend_marker: 1,
                }],
            ),
            hierarchy: Hierarchy {
                parents: vec![None],
                absolute: vec![AbsoluteBone {
                    translation: local.translation,
                    rotation: local.rotation,
                }],
                updates: vec![1],
                evaluated_generation: u32::MAX,
            },
            geometry: Geometry {
                dirty: true,
                generation: 0,
                evaluated_generation: u32::MAX,
                bounds_generation: u32::MAX,
                bone_generations: vec![0],
            },
            wiggle: vec![WiggleState::new(bone.wiggle, crt)],
            flags: vec![[1; 3]],
            named_bone_flags: vec![true],
        }
    }

    /// Clone a normal BCF rig whose parents precede their children, including
    /// each bone's independently initialized noise state and exact CRT draws.
    pub fn clone_prototype(source: &Skeleton, crt: &mut u32) -> Result<Self, String> {
        if source.bones.is_empty() || source.bones.len() > 256 {
            return Err("native skeleton clone bone count is invalid".into());
        }
        for (i, b) in source.bones.iter().enumerate() {
            if b.name.is_empty()
                || source.bones[..i].iter().any(|other| other.name == b.name)
                || (i == 0) != b.parent.is_none()
                || b.parent.is_some_and(|p| p >= i)
                || b.flags.iter().any(|&v| v > 1)
            {
                return Err("native skeleton clone requires ordered unique bones".into());
            }
            if !b.wiggle.amplitude.is_finite() {
                return Err(format!("nonfinite bone wiggle amplitude: {}", b.name));
            }
        }
        let bones: Vec<_> = source
            .bones
            .iter()
            .map(|b| BoneInput {
                local: LocalBone {
                    translation: b.translation,
                    rotation: b.rotation,
                },
                priority: 0,
                blend_marker: b.flags[2],
            })
            .collect();
        let parents: Vec<_> = source.bones.iter().map(|b| b.parent).collect();
        // The derived pose is provisional until update() publishes its stamp.
        // Validate all source transforms before consuming process RNG state.
        let absolute = super::hierarchy(
            &parents,
            &bones.iter().map(|b| b.local).collect::<Vec<_>>(),
            AbsoluteBone {
                translation: [0.; 3],
                rotation: [0., 0., 0., 1.],
            },
        )?;
        let wiggle = source
            .bones
            .iter()
            .map(|b| WiggleState::new(b.wiggle, crt))
            .collect();
        Ok(Self {
            skin: Live::new(Animator::new(-1, 0, 0, true, -1), bones),
            hierarchy: Hierarchy {
                updates: parents
                    .iter()
                    .map(|p| if p.is_some() { 11 } else { 10 })
                    .collect(),
                parents,
                absolute,
                evaluated_generation: u32::MAX,
            },
            geometry: Geometry {
                dirty: true,
                generation: 0,
                evaluated_generation: u32::MAX,
                bounds_generation: u32::MAX,
                bone_generations: source
                    .bones
                    .iter()
                    .map(|b| if b.parent.is_some() { 3 } else { 2 })
                    .collect(),
            },
            wiggle,
            flags: source.bones.iter().map(|b| b.flags).collect(),
            named_bone_flags: source
                .bones
                .iter()
                .map(|b| {
                    matches!(
                        b.name.as_str(),
                        "HEAD" | "PELVIS" | "R_FOOT" | "L_FOOT" | "R_HAND" | "L_HAND"
                    )
                })
                .collect(),
        })
    }
}
