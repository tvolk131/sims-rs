//! CPState4cee89 and the three original speed constants. This setter does not
//! clamp its input, reset timer credit, unpause, or publish to the lot.
use super::state::State;
use crate::scheduler::clock::Timer;
use serde::{Deserialize, Serialize};

pub const NORMAL: i32 = 760; //4425a2
pub const FAST: i32 = 924; //46d264
pub const ULTRA: i32 = 994; //46d26a

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timers {
    /// CPState+200 and+210, constructed separately by4732c0.
    pub first: Timer,
    pub second: Timer,
}

/// Exact fixed-point form of the original x87 sequence over every i32 input.
/// The first polynomial has at most63 significant bits, so extended arithmetic
/// is exact. __ftol truncates, then its EAX is *zero* extended for the next FILD.
pub fn intervals(speed: i32) -> [u32; 2] {
    let period = ((345i128 << 32) - i128::from(speed) * 8_589_935 * 171) / (1 << 33);
    // Original f32 constants: 3a83126f=8589935/2^33 above,
    // 3cc30c31=12782641/2^29 here (not the exact rational1/42).
    let numerator = u128::from(period as u32) * 12_782_641;
    // FMUL1000/FADD0.5 can round in extended precision, but the exact numerator
    // is divisible by8 and their total rounding error is <8 numerator units.
    // Consequently no integer boundary can be crossed before __ftol truncation.
    let first = ((numerator * 1000 + (1 << 28)) >> 29) as u32;
    // FST single retains ST(0) for the first timer. Only the second timer uses
    // the rounded single, reloaded by FLD, before multiplying by3000.
    let shift = (128 - numerator.leading_zeros()).saturating_sub(24);
    let single = if shift == 0 {
        numerator
    } else {
        let high = numerator >> shift;
        let tail = numerator & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        (high + u128::from(tail > half || tail == half && high & 1 != 0)) << shift
    };
    let second = ((single * 3000 + (1 << 28)) >> 29) as u32;
    [first, second]
}

impl State {
    pub fn set_speed(&mut self, speed: i32) -> Result<(), String> {
        self.validate()?;
        if self.speed_timers.is_none() {
            return Err("original control speed timers are not bound".into());
        }
        self.mark_dirty(0x40);
        self.speed = speed;
        let [first, second] = intervals(speed);
        let timers = self.speed_timers.as_mut().unwrap();
        timers.first.set_interval(first);
        timers.second.set_interval(second);
        Ok(())
    }
}
