//! Native ranking/selection after eligibility has produced candidate scores.
//! Enumeration and behavior callbacks are intentionally separate from this layer.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    pub object: i16,
    pub menu: u32,
    pub score: f32,
    pub force_best: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Selection {
    pub posture: i16,
    pub person18: i16,
    pub visitor: i16,
    pub live_family: bool,
    pub random_count: i32,
    pub sitting_threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Enumeration {
    pub age: i16,
    pub species: i16,
    pub visitor: i16,
    pub live_family: bool,
    pub level: i16,
    pub pet_permission: i16,
    pub target_kind: i16,
    pub target_room: i16,
}
impl Enumeration {
    /// Ordinary-entry part of 0x458826 on a residential lot, including people.
    /// Call this in reverse TTAB order, before the optional active-action join.
    /// It admits candidates for eligibility callbacks; it does not run the tests.
    pub fn permits(&self, flags: u32, threshold: i16) -> Result<bool, String> {
        let dog = self.species & 2 != 0;
        let cat = self.species & 4 != 0;
        if (self.target_kind == 7 && !dog && !cat) || threshold <= 0 {
            return Ok(false);
        }
        if (dog && flags & 1024 == 0) || (cat && flags & 512 == 0) {
            return Ok(false);
        }
        if self.age != 0 && self.age < 18 {
            if flags & 16 != 0 {
                return Ok(false);
            }
        } else if !dog && !cat && flags & 64 != 0 {
            return Ok(false);
        }
        if self.level < threshold || flags & 128 != 0 {
            return Ok(false);
        }
        if self.visitor != 0 && flags & 1 == 0 && (self.live_family || flags & 32 == 0) {
            return Ok(false);
        }
        if self.live_family
            && self.visitor != 0
            && (dog || cat)
            && self.pet_permission == 0
            && self.target_room != 0
        {
            return Ok(false);
        }
        Ok(true)
    }
}

fn compare(a: &Candidate, b: &Candidate) -> i8 {
    let difference = f64::from(b.score) - f64::from(a.score);
    let epsilon = f64::from(0.0000001f32);
    if difference > epsilon {
        1
    } else if difference < -epsilon {
        -1
    } else {
        0
    }
}

/// The executable's CRT sort has observable tie ordering. A generic stable or
/// Rust unstable sort does not preserve the original candidate/RNG pairing.
fn sort(candidates: &mut [Candidate]) {
    if candidates.len() <= 8 {
        for end in (1..candidates.len()).rev() {
            let mut greatest = 0;
            for i in 1..=end {
                if compare(&candidates[i], &candidates[greatest]) > 0 {
                    greatest = i;
                }
            }
            candidates.swap(greatest, end);
        }
        return;
    }
    candidates.swap(0, candidates.len() / 2);
    let mut left = 0;
    let mut right = candidates.len();
    loop {
        left += 1;
        while left < candidates.len() && compare(&candidates[left], &candidates[0]) <= 0 {
            left += 1;
        }
        right -= 1;
        while right > 0 && compare(&candidates[right], &candidates[0]) >= 0 {
            right -= 1;
        }
        if left > right {
            break;
        }
        candidates.swap(left, right);
    }
    candidates.swap(0, right);
    if right > 1 {
        sort(&mut candidates[..right]);
    }
    if left + 1 < candidates.len() {
        sort(&mut candidates[left..]);
    }
}

/// Sims 1.91 0x45e1ad..0x45e2ad. A rejected seated selection does not retry
/// another candidate; random choices consume one value even with a single option.
pub fn select(
    candidates: &mut [Candidate],
    policy: &Selection,
    rng: &mut u32,
) -> Result<Option<Candidate>, String> {
    if candidates.len() > 65536
        || !policy.sitting_threshold.is_finite()
        || candidates.iter().any(|c| !c.score.is_finite())
    {
        return Err("invalid autonomy selection data".into());
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    sort(candidates);
    let count = policy
        .random_count
        .wrapping_sub(i32::from(policy.person18) / 2000)
        .max(1) as usize;
    let index = if (policy.visitor != 0 || candidates[0].force_best) && policy.live_family {
        0
    } else {
        crate::rng::next(rng) as usize % candidates.len().min(count)
    };
    let candidate = &candidates[index];
    Ok(
        (policy.posture == 0 || candidate.score >= policy.sitting_threshold)
            .then(|| candidate.clone()),
    )
}

mod query;
pub use query::Query;

/// Host-supplied spatial and lot-classification boundaries. Registry membership
/// is the original object's bit-19 advertisement flag, not a guessed radius.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Context {
    pub residential: bool,
    pub selected_person: Option<i16>,
    pub suppressed: bool,
    /// None uses 0x45e2be's current-lot zoning query. Authored fixtures can
    /// explicitly select actors for a complete scan instead of phased subsets.
    pub full_scan: Option<std::collections::BTreeSet<i16>>,
    /// None reads the live native registry flags. Authored fixtures may supply
    /// an explicit set instead of claiming native registry construction.
    pub advertised: Option<std::collections::BTreeSet<i16>>,
    pub placements: std::collections::BTreeMap<i16, Placement>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub level: i32,
}
/// 0x434148: horizontal coordinates are sixteenths of a tile; each floor adds
/// twenty distance units. Native integer arithmetic precedes floating arithmetic.
pub fn distance(a: Placement, b: Placement, target_kind: i16) -> f64 {
    if target_kind == 7 {
        return 0.0;
    }
    let x = a.x.wrapping_sub(b.x);
    let y = a.y.wrapping_sub(b.y);
    let horizontal = x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y));
    let vertical = f64::from(b.level.wrapping_sub(a.level).wrapping_mul(20));
    (vertical * vertical + f64::from(horizontal) / 256.0).sqrt()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tuning {
    pub family_threshold: f32,
    pub visitor_threshold: f32,
    pub sitting_threshold: f32,
    pub random_count: i32,
    pub family_attenuation: [f32; 3],
    pub visitor_attenuation: [f32; 3],
}
impl Tuning {
    /// Projection of the retained process words, after the native one-time
    /// initializer has run. FCNS is not reread here: its names/defaults and
    /// integer conversion belong to459120. A missing owner remains unknown.
    pub fn from_world(world: &crate::World) -> Result<Self, String> {
        use crate::autonomy_constants::Field::*;
        let result = Self {
            family_threshold: process_float(world, FamilyThreshold)?,
            visitor_threshold: process_float(world, VisitorThreshold)?,
            sitting_threshold: process_float(world, SittingThreshold)?,
            random_count: process_word(world, RandomCount)? as i32,
            family_attenuation: [
                process_float(world, LowAttenuation)?,
                process_float(world, ModerateAttenuation)?,
                process_float(world, HighAttenuation)?,
            ],
            visitor_attenuation: [
                process_float(world, VisitorLow)?,
                process_float(world, VisitorModerate)?,
                process_float(world, VisitorHigh)?,
            ],
        };
        Ok(result)
    }
    pub fn attenuation(&self, code: u32, explicit: f32, visitor: bool) -> f32 {
        match code {
            0 => explicit,
            1 => 0.,
            2..=4 => {
                if visitor {
                    self.visitor_attenuation[(code - 2) as usize]
                } else {
                    self.family_attenuation[(code - 2) as usize]
                }
            }
            _ => 0.02,
        }
    }
}

/// 0x45e2be and 0x42b9e3: a nonzero zoning result forces a complete scan.
/// Lot zero returns one without consulting the table; a missing lot returns zero.
pub fn full_lot_scan(lot: i16, zoning: &std::collections::BTreeMap<i16, i32>) -> bool {
    lot == 0 || zoning.get(&lot).copied().unwrap_or(0) != 0
}

/// Direct lazy read of a known native process DWORD. The loaded gate is not a
/// prerequisite for consumers that read these globals independently of459844.
pub(crate) fn process_word(
    world: &crate::World,
    field: crate::autonomy_constants::Field,
) -> Result<u32, String> {
    world
        .autonomy_constants
        .as_ref()
        .and_then(|owner| owner.process.words[field.index()])
        .ok_or_else(|| format!("native autonomy process word {field:?} unavailable"))
}

pub(crate) fn process_float(
    world: &crate::World,
    field: crate::autonomy_constants::Field,
) -> Result<f32, String> {
    let value = f32::from_bits(process_word(world, field)?);
    if !value.is_finite() {
        return Err(format!("nonfinite native autonomy process word {field:?}"));
    }
    Ok(value)
}

///475ec6 selects only the requested word. Codes0/1/unknown do not require the
/// process owner, another attenuation word, or any initializer configuration.
pub fn process_attenuation(
    world: &crate::World,
    code: u32,
    explicit: f32,
    visitor: bool,
) -> Result<f32, String> {
    use crate::autonomy_constants::Field::*;
    match code {
        0 => Ok(explicit),
        1 => Ok(0.),
        2..=4 => {
            let fields = if visitor {
                [VisitorLow, VisitorModerate, VisitorHigh]
            } else {
                [LowAttenuation, ModerateAttenuation, HighAttenuation]
            };
            process_float(world, fields[(code - 2) as usize])
        }
        _ => Ok(0.02),
    }
}
