//! Original camera motion4c9a27 and4733b7/473390 linear curves. Raw float
//! storage preserves exceptional values and signed zero across checkpoints.
use serde::{Deserialize, Serialize};
pub mod cancel;
pub mod frame;
pub(super) mod wide;
use wide::Wide;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Target, magnitude and slope float bits, then unsigned end tick.
    pub x: [u32; 4],
    pub y: [u32; 4],
    pub active: u32,
    pub policy: u32,
    pub duration: i32,
    pub speed_bits: u32,
    pub rate_bits: u32,
}
fn quotient_bits(numerator: Wide, denominator: i32) -> u32 {
    numerator.divided(Wide::integer(denominator)).map_or_else(
        || {
            if numerator.is_zero() {
                0xffc00000
            } else {
                0x7f800000 | (u32::from(numerator.below_zero()) << 31)
            }
        },
        Wide::f32_bits,
    )
}
fn curve(start: i32, target: i32, duration: i32, now: u32) -> [u32; 4] {
    let a = Wide::from_f32((start as f32).to_bits()).unwrap();
    let target_bits = (target as f32).to_bits();
    let delta = Wide::from_f32(target_bits).unwrap().add(a.negated());
    let slope = quotient_bits(delta, duration);
    // An unordered x87 comparison takes the sign-flip branch as well.
    let magnitude = if f32::from_bits(slope).is_nan()
        || delta.below_zero() != (duration < 0) && !delta.is_zero()
    {
        slope ^ 0x80000000
    } else {
        slope
    };
    [
        target_bits,
        magnitude,
        slope,
        now.wrapping_add(duration as u32),
    ]
}
fn rounded_target(curve: [u32; 4]) -> i32 {
    Wide::from_f32(curve[0]).map_or(0, Wide::rounded_integer)
}
fn sample(curve: [u32; 4], now: u32) -> i32 {
    if now >= curve[3] {
        return rounded_target(curve);
    }
    Wide::from_f32(curve[0])
        .zip(Wide::from_f32(curve[2]))
        .map_or(0, |(target, slope)| {
            target
                .add(slope.times_unsigned(curve[3] - now).negated())
                .rounded_integer()
        })
}
impl State {
    /// Supply the centering owner's motion services against these actual
    /// curves. The owner descriptor contains the same three native control
    /// fields; synchronize them at the call boundary and publish mutations.
    pub fn center_service(
        &mut self,
        owner: &mut super::centering::State,
        request: &super::centering::Request,
        now: u32,
    ) -> Result<super::centering::Reply, String> {
        use super::centering::{Reply, Request};
        if !matches!(
            request,
            Request::ConfigureMotion { .. } | Request::SampleMotion { axis: 0 | 1 }
        ) {
            return Err("native centering motion service unavailable".into());
        }
        self.active = owner.motion_active;
        self.policy = owner.motion_policy;
        self.duration = owner.motion_duration;
        let reply = match *request {
            Request::ConfigureMotion {
                start,
                target,
                style,
            } => {
                self.configure(start, target, u32::from(style), now);
                Reply::Done
            }
            Request::SampleMotion { axis } => Reply::Coordinate(self.sample_axis(axis, now)?),
            _ => unreachable!(),
        };
        owner.motion_active = self.active;
        Ok(reply)
    }
    pub fn configure(&mut self, start: [i32; 2], target: [i32; 2], style: u32, now: u32) {
        if style == 0 {
            self.x = curve(target[0], target[0], 1, now);
            self.y = curve(target[1], target[1], 1, now);
        } else {
            let duration = if self.policy != 0 {
                let distance = Wide::distance(
                    target[0].wrapping_sub(start[0]),
                    target[1].wrapping_sub(start[1]),
                );
                self.rate_bits = quotient_bits(distance, self.duration);
                self.duration
            } else {
                let distance = Wide::distance(
                    start[0].wrapping_sub(target[0]),
                    start[1].wrapping_sub(target[1]),
                );
                let speed = Wide::from_f32(self.speed_bits);
                let duration = if self.speed_bits & 0x7fffffff == 0x7f800000 {
                    0 // finite distance divided by either infinity, then+0.5
                } else {
                    speed
                        .and_then(|s| distance.divided(s))
                        .map_or(0, |v| v.add(Wide::half()).ftol())
                };
                self.rate_bits = self.speed_bits;
                duration
            };
            self.x = curve(start[0], target[0], duration, now);
            self.y = curve(start[1], target[1], duration, now);
        }
        self.active = 1;
    }
    /// Destination getters4c9bc4/4c9be7 do not sample time or clear active.
    pub fn destination(&self) -> [i32; 2] {
        [rounded_target(self.x), rounded_target(self.y)]
    }
    /// Current samples4c9b4e/4c9b87 clear active independently on expiry.
    pub fn sample_axis(&mut self, axis: u8, now: u32) -> Result<i32, String> {
        let c = match axis {
            0 => self.x,
            1 => self.y,
            _ => return Err("native camera motion axis unavailable".into()),
        };
        if now >= c[3] {
            self.active = 0;
        }
        Ok(sample(c, now))
    }
    ///4c9b49 clears only the active field; stored endpoints remain available.
    pub fn cancel(&mut self) {
        self.active = 0;
    }
}
