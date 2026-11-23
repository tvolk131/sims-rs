//! A need's recent history, for the Needs panel's trend (`cAverageHistory`).
//!
//! **What the original does.** The live panel's two sets of need bars,
//! at +0x128 and +0x134, each keep one history per family member:
//!
//! - 0x513a9b and 0x513b41 grow the list to the household's size, with
//!   capacity 10 and limit 5 (0x513aeb, 0x513b87).
//! - When the live panel's update says so (0x51d149..0x51d166), 0x513b41
//!   samples each member's need. That is the person's float at +0x78c plus
//!   4 times the bar's need number (0x4ea6a6 → 0x45d125), rounded half away
//!   from zero (0x513bbf..0x513bdb).
//! - The shown member's history (+0x198) goes to the bar (0x50c5da). The
//!   bar's own sample (0x50c63d) adds a value; when the bar is shown
//!   (vtable +0x8c), 0x50c66f stores the trend in its byte +0xde, with
//!   flags +0xdc and +0xdd.
//!
//! **Not ported:** that wiring. The bars do not draw a trend yet, the update
//! that gates the sample (`ebx` at 0x51d149) has not been read, and how
//! +0xde is drawn is unknown. The history is ported so the bars can use it.

/// Samples a history keeps (0x513aeb: `push 0xa`).
pub const CAPACITY: usize = 10;
/// The largest trend either way (0x513aeb: `push 5`).
pub const LIMIT: i32 = 5;

/// One member's history of one need (0x4cb4a5): the samples, each with the
/// mean of the history as it stood when the sample came in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    capacity: usize,
    limit: i32,
    samples: Vec<i32>,
    /// +0x10: the mean after each sample. It is not worked out again when
    /// an old sample leaves.
    means: Vec<i32>,
    /// +0x1c: the samples' sum, in 32 bits.
    sum: i32,
}

impl Default for History {
    fn default() -> Self {
        Self::new(CAPACITY, LIMIT)
    }
}

impl History {
    /// An empty history of `capacity` samples whose trend is at most
    /// `limit` either way. A capacity of 0 is taken as 1.
    pub fn new(capacity: usize, limit: i32) -> Self {
        Self {
            capacity: capacity.max(1),
            limit,
            samples: Vec::new(),
            means: Vec::new(),
            sum: 0,
        }
    }

    pub fn samples(&self) -> &[i32] {
        &self.samples
    }

    pub fn means(&self) -> &[i32] {
        &self.means
    }

    /// The mean of the samples (0x4cb58a). It is 0 with none. Otherwise it
    /// is the 32-bit sum over the count, then 0.5 is added, or taken away
    /// below zero, and the result is cut toward zero. So it rounds half
    /// away from zero, and with these magnitudes the x87 quotient is exact
    /// enough that this integer form agrees.
    pub fn mean(&self) -> i32 {
        if self.samples.is_empty() {
            return 0;
        }
        let sum = i64::from(self.sum);
        let count = self.samples.len() as i64;
        let magnitude = (sum.abs() * 2 + count) / (count * 2);
        (if sum < 0 { -magnitude } else { magnitude }) as i32
    }

    /// Adds a sample (0x4cb5d6). A full history first drops its oldest
    /// sample and that sample's mean.
    pub fn push(&mut self, value: i32) {
        if self.samples.len() == self.capacity {
            self.sum = self.sum.wrapping_sub(self.samples.remove(0));
            self.means.remove(0);
        }
        self.samples.push(value);
        self.sum = self.sum.wrapping_add(value);
        self.means.push(self.mean());
    }

    /// The newest mean less the oldest, at most the limit either way
    /// (0x4cb635). The original reads both ends without a check, and its
    /// bar always samples first; with no sample there is no trend here.
    pub fn trend(&self) -> Option<i32> {
        let delta = self.means.last()?.wrapping_sub(*self.means.first()?);
        let low = self.limit.wrapping_neg();
        Some(if delta < low {
            low
        } else if self.limit < delta {
            self.limit
        } else {
            delta
        })
    }
}

/// A need's value as 0x513bbf..0x513bdb samples it: rounded half away from
/// zero, then cut toward zero.
pub fn sample(need: f32) -> i32 {
    let need = f64::from(need);
    (if need < 0.0 { need - 0.5 } else { need + 0.5 }) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_history_keeps_each_samples_mean_and_rounds_ties_away_from_zero() {
        let mut h = History::new(3, 5);
        assert_eq!(h.mean(), 0);
        assert_eq!(h.trend(), None);
        for v in [1, 2, 8, -10, -11] {
            h.push(v);
        }
        assert_eq!(h.samples(), [8, -10, -11]);
        // When each came in: 11/3 is 4, 0/3 is 0, -13/3 is -4.
        assert_eq!(h.means(), [4, 0, -4]);
        assert_eq!(h.trend(), Some(-5), "-8, bounded");
        for (values, expected) in [
            (&[0, 1][..], 1),
            (&[0, -1], -1),
            (&[0, 3], 2),
            (&[0, -3], -2),
            (&[1, -1], 0),
        ] {
            let mut h = History::default();
            for &v in values {
                h.push(v);
            }
            assert_eq!(h.mean(), expected, "{values:?}");
        }
    }

    #[test]
    fn the_sum_wraps_in_32_bits() {
        let mut h = History::new(2, 5);
        h.push(i32::MAX);
        h.push(i32::MAX);
        assert_eq!(h.mean(), -1, "the sum wraps to -2");
        h.push(i32::MIN);
        assert_eq!(h.mean(), -1, "-1 / 2 rounds away from zero");
        h.push(0);
        assert_eq!(h.mean(), i32::MIN / 2);
    }

    #[test]
    fn the_trend_is_bounded_by_the_limit() {
        let mut h = History::default();
        h.push(0);
        assert_eq!(h.trend(), Some(0));
        h.push(6);
        // Means 0 then 3.
        assert_eq!(h.trend(), Some(3));
        for _ in 0..8 {
            h.push(60);
        }
        assert_eq!(h.trend(), Some(LIMIT));
        let mut h = History::default();
        h.push(0);
        h.push(-100);
        assert_eq!(h.trend(), Some(-LIMIT));
    }

    #[test]
    fn a_need_samples_rounded_half_away_from_zero() {
        assert_eq!(sample(0.5), 1);
        assert_eq!(sample(-0.5), -1);
        assert_eq!(sample(49.49), 49);
        assert_eq!(sample(-49.5), -50);
        assert_eq!(sample(0.0), 0);
    }
}
