//! Original named rig anchors (0x5924a4 / 0x5a1631) and point sampling
//! (0x595f32 / 0x59638f / 0x59640d). Inputs remain in native coordinates.
//! Animation evaluation and the actor placement matrix are supplied separately.
use serde::{Deserialize, Serialize};
use sims_formats::avatar::Skeleton;

/// The initialized native name table at 0x6d1aa0 maps index 31 to HEAD.
/// This is not index 31 in a BCF skeleton (the stock adult has only 29 bones).
pub const HEAD_BONE: &str = "HEAD";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Origin,
    Palm,
    Top,
    HeadForward,
    Shoulder,
}
impl Kind {
    /// Exact case-sensitive native anchor labels. Null/empty means the origin;
    /// an unknown nonempty label produces no anchor.
    pub fn named(name: Option<&str>) -> Option<Self> {
        match name.unwrap_or("") {
            "" => Some(Self::Origin),
            "Palm" => Some(Self::Palm),
            "Top" => Some(Self::Top),
            "HeadFake" => Some(Self::HeadForward),
            "Shoulder" => Some(Self::Shoulder),
            _ => None,
        }
    }
    pub fn offset(self) -> [f32; 3] {
        match self {
            Self::Origin => [0., 0., 0.],
            Self::Palm => [0.5, 0., 0.],
            Self::Top => [0.8, 0., 0.],
            Self::HeadForward => [0.8, 0., 100.],
            Self::Shoulder => [0., 0., 1.],
        }
    }
    /// Row-vector transform created by 0x5a1631. These angles deliberately use
    /// the original literals, which differ slightly from f32::consts::PI.
    pub fn local_matrix(self) -> [f32; 16] {
        let angle = match self {
            Self::Origin => 0.,
            Self::Palm | Self::Shoulder => f32::from_bits(0x40490fd0),
            Self::Top | Self::HeadForward => f32::from_bits(0xbfc90fd0),
        };
        let (s, c) = f64::from(angle).sin_cos();
        let [x, y, z] = self.offset();
        [
            c as f32, s as f32, 0., 0., -s as f32, c as f32, 0., 0., 0., 0., 1., 0., x, y, z, 1.,
        ]
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub bone: usize,
    pub kind: Kind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rig {
    pub head: Option<Anchor>,
    pub hand: Option<Anchor>,
    pub pelvis: Option<Anchor>,
    pub forward: Option<Anchor>,
    pub turn_bone: Option<usize>,
}
impl Rig {
    /// Original character STR#200 entries 4–7 select the head and hand anchors.
    pub fn bind(skeleton: &Skeleton, profile: &[String]) -> Result<Self, String> {
        if profile.len() < 7 {
            return Err("incomplete original rig profile".into());
        }
        let bone = |name: &str| skeleton.bones.iter().position(|b| b.name == name);
        let anchor = |name: &str, kind: Option<Kind>| -> Option<Anchor> {
            Some(Anchor {
                bone: bone(name)?,
                kind: kind?,
            })
        };
        Ok(Self {
            head: anchor(&profile[3], Kind::named(Some(&profile[4]))),
            hand: anchor(&profile[5], Kind::named(Some(&profile[6]))),
            pelvis: anchor("PELVIS", Some(Kind::Origin)),
            forward: anchor(
                &profile[3],
                Some(if profile[3].is_empty() {
                    Kind::Origin
                } else {
                    Kind::HeadForward
                }),
            ),
            turn_bone: bone(HEAD_BONE),
        })
    }
}

/// Already-evaluated absolute bone pose, before the actor's placement matrix.
/// The native sampler does not normalize this quaternion.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AbsoluteBone {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
}
impl AbsoluteBone {
    /// Native attachment-space affine matrix, column-major, without invented
    /// quaternion normalization (0x595f32 / 0x59601b).
    pub fn matrix(self) -> Result<[f32; 16], String> {
        if self
            .translation
            .iter()
            .chain(self.rotation.iter())
            .any(|v| !v.is_finite())
        {
            return Err("invalid native rig point input".into());
        }
        let [x, y, z, w] = self.rotation.map(f64::from);
        let spill = |v: f64| f64::from(v as f32);
        // 0x595f32 conjugates the input before 0x59601b's matrix convention.
        let xx = spill(2. * x * x);
        let yy = spill(2. * y * y);
        let zz = spill(2. * z * z);
        let xy = spill(2. * x * y);
        let xz = spill(2. * x * z);
        let yz = 2. * y * z;
        let wx = 2. * w * x;
        let wy = spill(2. * w * y);
        let wz = spill(2. * w * z);
        Ok([
            (1. - yy - zz) as f32,
            (xy + wz) as f32,
            (xz - wy) as f32,
            0.,
            (xy - wz) as f32,
            (1. - xx - zz) as f32,
            (yz + wx) as f32,
            0.,
            (xz + wy) as f32,
            (yz - wx) as f32,
            (1. - xx - yy) as f32,
            0.,
            self.translation[0],
            self.translation[1],
            self.translation[2],
            1.,
        ])
    }
    pub fn anchor_point(self, kind: Kind, actor: [f32; 16]) -> Result<[f32; 3], String> {
        if actor.iter().any(|v| !v.is_finite()) {
            return Err("invalid native rig point input".into());
        }
        let m = self.matrix()?;
        let a = kind.offset().map(f64::from);
        let point: [f32; 3] = std::array::from_fn(|i| {
            (a[0] * f64::from(m[i])
                + a[1] * f64::from(m[4 + i])
                + a[2] * f64::from(m[8 + i])
                + f64::from(self.translation[i])) as f32
        });
        let out = std::array::from_fn(|i| {
            (f64::from(point[0]) * f64::from(actor[i])
                + f64::from(point[1]) * f64::from(actor[4 + i])
                + f64::from(point[2]) * f64::from(actor[8 + i])
                + f64::from(actor[12 + i])) as f32
        });
        if out.iter().any(|v| !v.is_finite()) {
            return Err("native rig point overflow".into());
        }
        Ok(out)
    }
}
