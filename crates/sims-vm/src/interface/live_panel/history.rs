//! cAverageHistory constructor4cb4a5, sample4cb5d6, mean4cb58a and trend4cb635. Both
//! vectors are retained: the second stores the average when each sample was
//! added, not averages recomputed after an old sample leaves the window.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AverageHistory {
    capacity: u32,
    /// Constructor+24, consumed by4cb635's separate trend calculation.
    trend_limit: i32,
    samples: Vec<i32>,
    averages: Vec<i32>,
    sum: i32,
}
impl AverageHistory {
    pub fn new(capacity: u32, trend_limit: i32) -> Result<Self, String> {
        if capacity == 0 || capacity > i32::MAX as u32 {
            return Err("unsupported native average history capacity".into());
        }
        Ok(Self {
            capacity,
            trend_limit,
            samples: vec![],
            averages: vec![],
            sum: 0,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.capacity == 0
            || self.capacity > i32::MAX as u32
            || self.samples.len() > self.capacity as usize
            || self.samples.len() != self.averages.len()
            || self
                .samples
                .iter()
                .fold(0i32, |sum, v| sum.wrapping_add(*v))
                != self.sum
            || self.averages.last().is_some_and(|v| *v != self.average())
        {
            Err("invalid native average history".into())
        } else {
            Ok(())
        }
    }
    pub fn samples(&self) -> &[i32] {
        &self.samples
    }
    pub fn averages(&self) -> &[i32] {
        &self.averages
    }
    pub fn trend_limit(&self) -> i32 {
        self.trend_limit
    }
    ///4cb635 compares the newest stored mean with the oldest one, then bounds
    /// their signed32-bit difference. The native routine dereferences both
    /// ends; an empty history is an explicit error, not a synthesized flat trend.
    pub fn trend(&self) -> Result<i32, String> {
        self.validate()?;
        let first = *self
            .averages
            .first()
            .ok_or("native trend requires a sample")?;
        let last = *self.averages.last().unwrap();
        let delta = last.wrapping_sub(first);
        // Its x87 +/-0.5 followed by truncation leaves this integer unchanged.
        // Keep the original comparison order even for negative limits.
        let low = self.trend_limit.wrapping_neg();
        Ok(if delta < low {
            low
        } else if self.trend_limit < delta {
            self.trend_limit
        } else {
            delta
        })
    }
    ///4cb58a divides signed sum by count and adds/subtracts0.5 before ftol.
    /// This integer expression computes the same nearest integer, with ties
    /// away from zero, without introducing host float rounding at a half.
    pub fn average(&self) -> i32 {
        if self.samples.is_empty() {
            return 0;
        }
        let sum = i64::from(self.sum);
        let n = self.samples.len() as i64;
        let magnitude = (sum.abs() * 2 + n) / (n * 2);
        (if sum < 0 { -magnitude } else { magnitude }) as i32
    }
    pub fn push(&mut self, value: i32) -> Result<(), String> {
        self.validate()?;
        if self.samples.len() == self.capacity as usize {
            self.sum = self.sum.wrapping_sub(self.samples[0]);
            self.samples.remove(0);
            self.averages.remove(0);
        }
        self.samples.push(value);
        self.sum = self.sum.wrapping_add(value);
        self.averages.push(self.average());
        Ok(())
    }
}

///473311's two unsigned fields. The caller controls accumulation/initialization
/// separately. A zero interval really returns true without consuming time;
/// bounded refresh execution must preserve that case rather than "fix" it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Elapsed {
    pub accumulated: u32,
    pub interval: u32,
}
impl Elapsed {
    pub fn consume(&mut self) -> bool {
        if self.interval <= self.accumulated {
            self.accumulated -= self.interval;
            true
        } else {
            false
        }
    }
}
