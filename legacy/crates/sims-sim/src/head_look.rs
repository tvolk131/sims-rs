//! Native head-look control and temporary composition (0x590768 / 0x5904d2).
//! Skeleton composition and world bone sampling are separate inputs. The clock
//! step uses the previous sampled angles; target observation runs afterward.
use serde::{Deserialize, Serialize};
use sims_vm::ObjectId;
pub mod rig;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct State {
    pub last_clock: u32,
    /// Heading sampled from the head anchors, in native eighth-turn units.
    /// This includes the temporary head pose, not just the body's facing.
    pub basis_heading: f32,
    pub target_heading: f32,
    pub turn: f32,
    /// Fresh construction leaves these words unwritten. Disabled tracking may
    /// retain that state; an enabled observation must establish a target before
    /// using its coordinates. Saved animators contain an explicit point.
    pub target_point: Option<[f32; 3]>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub target: ObjectId,
    pub flags: i16,
    pub mode: i16,
    pub countdown: i16,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub level: i32,
    /// A person without the queried bone leaves the previous point intact.
    pub point: Option<[f32; 3]>,
}
impl State {
    /// Decode the six floats read by 0x5918ba. The load/update coordinator must
    /// supply the head clock: the original descriptor does not serialize it.
    /// Disabled tracking can retain stale target coordinates; preserve them.
    pub fn from_saved(
        person: &sims_formats::saved_objects::Person,
        last_clock: u32,
    ) -> Result<Self, String> {
        let v = person.floats;
        let state = Self {
            last_clock,
            basis_heading: v[0],
            target_heading: v[1],
            turn: v[2],
            target_point: Some([v[3], v[4], v[5]]),
        };
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.basis_heading.is_finite()
            || !self.target_heading.is_finite()
            || !self.turn.is_finite()
            || self.target_point.iter().flatten().any(|v| !v.is_finite())
            || self.basis_heading.abs() > 1_000_000.
            || self.target_heading.abs() > 1_000_000.
        {
            return Err("invalid native head-look state".into());
        }
        Ok(())
    }
    /// Run the original timer/angle step. True requests temporary head-bone
    /// composition, including when the same clock is observed more than once.
    pub fn advance(&mut self, clock: u32, fields: &mut Fields) -> Result<bool, String> {
        self.validate()?;
        if self.last_clock != clock {
            if fields.countdown > 0 {
                fields.countdown -= 1;
                if fields.countdown == 0 {
                    fields.flags = 8;
                }
            }
            if fields.flags & 1 == 0 {
                self.turn = (f64::from(self.turn) * 0.8) as f32;
            } else {
                let mut delta = f64::from(self.basis_heading) - f64::from(self.target_heading);
                while delta < -4. {
                    delta += 8.;
                }
                while delta >= 4. {
                    delta -= 8.;
                }
                let limit = f64::from(0.2_f32);
                let next = f64::from(self.turn) + delta.clamp(-limit, limit);
                self.turn = next as f32;
                if !(-1.5..=1.5).contains(&next) {
                    self.turn = if next < -1.5 { -1.5 } else { 1.5 };
                    match fields.mode {
                        0 => fields.flags = 5,
                        1 => fields.flags = 4,
                        _ => (),
                    }
                }
            }
            self.last_clock = clock;
        }
        if self.turn.abs() < 0.01_f32 {
            self.turn = 0.;
            Ok(false)
        } else {
            Ok(true)
        }
    }
    /// Observe the target first, then sample the two original rig points. The
    /// missing-target/wrong-floor branches still calculate angles this visit.
    pub fn observe(
        &mut self,
        fields: &mut Fields,
        owner_level: i32,
        has_basis: bool,
        mut target: impl FnMut(ObjectId) -> Result<Option<Target>, String>,
        mut basis: impl FnMut() -> Result<[[f32; 3]; 2], String>,
    ) -> Result<(), String> {
        self.validate()?;
        if !has_basis || fields.flags & 1 == 0 {
            return Ok(());
        }
        if fields.target != 0 {
            if let Some(target) = target(fields.target)? {
                if let Some(point) = target.point {
                    if point.iter().any(|v| !v.is_finite()) {
                        return Err("invalid native head target point".into());
                    }
                    self.target_point = Some(point);
                }
                if target.level != owner_level {
                    fields.target = 0;
                    fields.flags = 16;
                }
            } else {
                fields.target = 0;
                fields.flags = 16;
            }
        }
        let [origin, forward] = basis()?;
        if origin.iter().chain(forward.iter()).any(|v| !v.is_finite()) {
            return Err("invalid native head basis points".into());
        }
        // Native spills these two differences to f32 before atan2; the
        // target-point differences stay extended until their angle is formed.
        let dx = forward[0] - origin[0];
        let dz = forward[2] - origin[2];
        if dx != 0. || dz != 0. {
            self.basis_heading = heading(f64::from(dx), f64::from(dz), false);
        }
        let point = self
            .target_point
            .ok_or("native head target coordinates are uninitialized")?;
        let dx = f64::from(point[0]) - f64::from(origin[0]);
        let dz = f64::from(point[2]) - f64::from(origin[2]);
        if dx != 0. || dz != 0. {
            self.target_heading = heading(dx, dz, true);
        }
        Ok(())
    }
}
fn heading(dx: f64, dz: f64, spill: bool) -> f32 {
    let mut value = dz.atan2(-dx) / (2. * f64::from(std::f32::consts::PI)) * 8.;
    if spill {
        value = f64::from(value as f32);
    }
    while value < 0. {
        value += 8.;
        if spill {
            value = f64::from(value as f32);
        }
    }
    while value >= 8. {
        value -= 8.;
        if spill {
            value = f64::from(value as f32);
        }
    }
    value as f32
}
/// 0x454f5c at zero vertical offset, for an ordinary target's native point.
pub fn ordinary_target_point(position: [i32; 2]) -> [f32; 3] {
    [position[0] as f32 * 0.1875, 0., position[1] as f32 * 0.1875]
}

/// 0x590768's temporary bone-31 rotation, in the original quaternion convention.
/// The base quaternion stays unchanged, corresponding to 0x590b27's restore.
/// Apply the returned pose while sampling head targets or drawing, then discard
/// it; accumulating it into the source animation would turn the head repeatedly.
pub fn compose(turn: f32, base: [f32; 4]) -> Result<Option<[f32; 4]>, String> {
    if !turn.is_finite() || base.iter().any(|v| !v.is_finite()) {
        return Err("invalid native head quaternion input".into());
    }
    if turn.abs() < 0.01_f32 {
        return Ok(None);
    }
    let length = base
        .iter()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        .sqrt();
    let q = if length == 0. {
        [0., 0., 0., 1.]
    } else {
        base.map(|v| (f64::from(v) / length) as f32)
    };
    let [x, y, z, w] = q.map(f64::from);
    let m5 = ((1. - (x * x + x * x)) - (z * z + z * z)) as f32;
    let m6 = ((z * y + z * y) - (w * x + w * x)) as f32;
    let angle = f64::from(-m6).atan2(f64::from(m5));
    let mut addition = (f64::from(turn) * 0.125 * (2. * f64::from(std::f32::consts::PI))) as f32;
    let total = angle + f64::from(addition);
    let limit = f64::from(1.2_f32);
    if total < -limit {
        addition = (-limit - angle) as f32;
    } else if total > limit {
        addition = (limit - angle) as f32;
    }
    let half = f64::from(addition * 0.5);
    let s = f64::from(half.sin() as f32);
    let c = f64::from(half.cos() as f32);
    Ok(Some([
        (c * x + s * w) as f32,
        (c * y - s * z) as f32,
        (s * y + c * z) as f32,
        (c * w - s * x) as f32,
    ]))
}
