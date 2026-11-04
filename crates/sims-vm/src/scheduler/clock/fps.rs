//! Application inline FPS history: sample473103, count473162, rate473088.
//! This kernel retains all1000 physical timestamp slots, including stale ones.
//! It does not import an application owner, construct one or run a frame.
use serde::{Deserialize, Serialize};

pub const SLOTS: usize = 1000;

/// Both FDIV and the caller's FSTP obey the actual x87 control word. Do not
/// assume the host's floating precision: observed game FCW003f uses24 bits.
fn control(word: u16) -> Result<(u32, u16), String> {
    if word & 0x3f != 0x3f {
        return Err("unmasked native FPS floating exceptions need their caller".into());
    }
    let precision = match (word >> 8) & 3 {
        0 => 24,
        2 => 53,
        3 => 64,
        _ => return Err("reserved native FPS precision control".into()),
    };
    Ok((precision, (word >> 10) & 3))
}

fn rounded_quotient(numerator: u128, denominator: u128, rounding: u16) -> u128 {
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let increment = match rounding {
        0 => remainder * 2 > denominator || (remainder * 2 == denominator && quotient & 1 != 0),
        2 => remainder != 0, // +infinity; FPS values are nonnegative.
        _ => false,          // -infinity and truncate coincide for this domain.
    };
    quotient + u128::from(increment)
}

/// Positive integer ratio, first rounded to the native arithmetic precision,
/// then stored as binary64 under the same rounding mode. The bounded FPS
/// domain is normal at all three precisions and fits every temporary in u128.
fn rate(samples: u32, elapsed: u32, precision: u32, rounding: u16) -> f64 {
    if elapsed == 0 {
        return f64::INFINITY;
    }
    let numerator = u128::from(samples) * 1000;
    let denominator = u128::from(elapsed);
    let mut exponent = numerator.ilog2() as i32 - denominator.ilog2() as i32;
    if (exponent >= 0 && numerator < denominator << exponent)
        || (exponent < 0 && numerator << -exponent < denominator)
    {
        exponent -= 1;
    }
    let mut significand = rounded_quotient(
        numerator << (precision as i32 - 1 - exponent),
        denominator,
        rounding,
    );
    if significand == 1u128 << precision {
        significand >>= 1;
        exponent += 1;
    }
    if precision > 53 {
        significand = rounded_quotient(significand, 1u128 << (precision - 53), rounding);
        if significand == 1u128 << 53 {
            significand >>= 1;
            exponent += 1;
        }
    } else {
        significand <<= 53 - precision;
    }
    f64::from_bits(((exponent + 1023) as u64) << 52 | (significand as u64 & ((1u64 << 52) - 1)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    timestamps: Vec<u32>,
    capacity: u32,
    next: u32,
    oldest: u32,
    /// Native code tests nonzero and preserves the word until rate/reset.
    full: u32,
}

impl History {
    pub fn from_captured_parts(
        timestamps: Vec<u32>,
        capacity: u32,
        next: u32,
        oldest: u32,
        full: u32,
    ) -> Result<Self, String> {
        let value = Self {
            timestamps,
            capacity,
            next,
            oldest,
            full,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.timestamps.len() != SLOTS
            || !(1..=SLOTS as u32).contains(&self.capacity)
            || self.next >= self.capacity
            || self.oldest >= self.capacity
        {
            return Err("unsupported native FPS history extent or indices".into());
        }
        Ok(())
    }

    pub fn timestamps(&self) -> &[u32] {
        &self.timestamps
    }

    /// Native words at+fa4,+fa8,+fac,+fb0, excluding the timestamp array.
    pub fn fields(&self) -> [u32; 4] {
        [self.capacity, self.next, self.oldest, self.full]
    }

    fn count_validated(&self) -> u32 {
        if self.next > self.oldest {
            self.next - self.oldest
        } else if self.next < self.oldest {
            self.capacity - self.oldest + self.next
        } else if self.full != 0 {
            self.capacity
        } else {
            0
        }
    }

    pub fn count(&self) -> Result<u32, String> {
        self.validate()?;
        Ok(self.count_validated())
    }

    pub fn sample(&mut self, now: u32) -> Result<(), String> {
        self.validate()?;
        self.timestamps[self.next as usize] = now;
        self.next += 1;
        if self.next == self.capacity {
            self.next = 0;
        }
        if self.full != 0 {
            self.oldest = self.next;
        } else if self.next == self.oldest {
            self.full = 1;
        }
        Ok(())
    }

    /// Consume the entire sample window. The timestamp array is not erased.
    /// The difference is unsigned32-bit; equal endpoints produce +infinity
    /// when there are at least two samples, as the masked x87 division does.
    /// The control word belongs to the caller and must be supplied explicitly.
    pub fn take_rate(&mut self, control_word: u16) -> Result<f64, String> {
        self.validate()?;
        let (precision, rounding) = control(control_word)?;
        let count = self.count_validated();
        let rate = if count < 2 {
            0.0
        } else {
            let last = (self.oldest + count - 1) % self.capacity;
            let elapsed =
                self.timestamps[last as usize].wrapping_sub(self.timestamps[self.oldest as usize]);
            rate(count - 1, elapsed, precision, rounding)
        };
        self.next = 0;
        self.oldest = 0;
        self.full = 0;
        Ok(rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    fn empty(capacity: u32) -> History {
        History::from_captured_parts(vec![0; SLOTS], capacity, 0, 0, 0).unwrap()
    }

    #[test]
    fn unsigned_time_wrap_reset_and_stale_slots() {
        let mut h = empty(7);
        h.sample(0xfffffff0).unwrap();
        h.sample(0x10).unwrap();
        let slots = h.timestamps.clone();
        assert_eq!(h.take_rate(0x37f).unwrap(), 31.25);
        assert_eq!(h.fields(), [7, 0, 0, 0]);
        assert_eq!(h.timestamps, slots);
        assert_eq!(h.take_rate(0x37f).unwrap().to_bits(), 0);
    }

    #[test]
    fn full_ring_overwrites_oldest_and_retains_nonzero_flag() {
        let mut h = History::from_captured_parts(vec![0; SLOTS], 3, 0, 0, 255).unwrap();
        h.sample(99).unwrap();
        assert_eq!(h.fields(), [3, 1, 1, 255]);
        assert_eq!(h.count().unwrap(), 3);
        assert_eq!(h.timestamps[0], 99);
        h.take_rate(0x37f).unwrap();
        assert_eq!(h.fields(), [3, 0, 0, 0]);
        for now in [4, 5, 6, 7] {
            h.sample(now).unwrap();
        }
        assert_eq!(h.fields(), [3, 1, 1, 1]);
        assert_eq!(&h.timestamps[..3], &[7, 5, 6]);
    }

    #[test]
    fn codec_resume_and_invalid_state_rejection_before_mutation() {
        let mut h = empty(2);
        h.sample(77).unwrap();
        let mut restored: History =
            serde_json::from_value(serde_json::to_value(&h).unwrap()).unwrap();
        restored.sample(77).unwrap();
        h.sample(77).unwrap();
        assert_eq!(restored, h);
        assert_eq!(restored.take_rate(0x37f).unwrap(), f64::INFINITY);
        assert!(History::from_captured_parts(vec![0; SLOTS], 0, 0, 0, 0).is_err());
        assert!(History::from_captured_parts(vec![0; SLOTS - 1], 2, 0, 0, 0).is_err());
        assert!(History::from_captured_parts(vec![0; SLOTS], 2, 2, 0, 0).is_err());
        h.oldest = 2;
        let before = h.clone();
        assert!(h.sample(1).is_err());
        assert!(h.take_rate(0x37f).is_err());
        assert!(h.count().is_err());
        assert_eq!(h, before);
    }

    #[test]
    fn caller_precision_rounding_and_invalid_control_are_explicit() {
        let mut h = empty(2);
        h.sample(0).unwrap();
        h.sample(3).unwrap();
        let before = h.clone();
        for control in [0u16, 0x13f] {
            assert!(h.take_rate(control).is_err());
            assert_eq!(h, before);
        }
        let mut single = h.clone();
        let mut double = h.clone();
        assert_eq!(single.take_rate(0x3f).unwrap(), (1000.0f32 / 3.0) as f64);
        assert_eq!(double.take_rate(0x23f).unwrap(), 1000.0 / 3.0);
        let down = h.clone().take_rate(0x43f).unwrap();
        let up = h.take_rate(0x83f).unwrap();
        assert!(down < up);
    }

    #[test]
    #[ignore = "private unchanged original-code FPS-history oracle results"]
    fn matches_original_instructions_and_all_physical_words() {
        compare_original(
            "application-fps-kernel-20261030-first",
            "c1301071825a0ddb908d8d7dd46c9437e672ce6dd4ada78b015e4fefbe411ef4",
            1985,
        );
        compare_original(
            "application-fps-kernel-20261030-second",
            "6925720a77c1d7c7ea34cd9dbe84137c9f8a8b0a6d1deb417e564f21c55fff9f",
            23820,
        );
    }

    fn compare_original(directory: &str, sha256: &str, expected_actions: usize) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/private")
            .join(directory)
            .join("original-results.json");
        let raw = std::fs::read(path).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&raw)), sha256);
        let evidence: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(
            evidence["kernel_sha256"],
            "6bb64259c8494373b609d173bd0173940c62a6954a353f5b196f619824c98dd6"
        );
        assert_eq!(evidence["native_parity_ticks"], 0);
        assert_eq!(evidence["source_ownership_qualified"], false);
        let mut actions = 0;
        for (ci, case) in evidence["cases"].as_array().unwrap().iter().enumerate() {
            let input = &case["input"];
            let control_word = input["control_word"].as_u64().unwrap_or(0x37f) as u16;
            let mut timestamps: Vec<_> = (0..SLOTS as u32)
                .map(|i| (i * 0x10001) ^ 0xabcddcba)
                .collect();
            for slot in input["slots"].as_array().unwrap() {
                timestamps[slot[0].as_u64().unwrap() as usize] = slot[1].as_u64().unwrap() as u32;
            }
            let field = |key| input[key].as_u64().unwrap() as u32;
            let mut h = History::from_captured_parts(
                timestamps,
                field("capacity"),
                field("head"),
                field("tail"),
                field("full"),
            )
            .unwrap();
            let mut restored: History =
                serde_json::from_value(serde_json::to_value(&h).unwrap()).unwrap();
            for (ai, (action, expected)) in input["actions"]
                .as_array()
                .unwrap()
                .iter()
                .zip(case["original"].as_array().unwrap())
                .enumerate()
            {
                let execute = |h: &mut History| match action["kind"].as_str().unwrap() {
                    "count" => json!(h.count().unwrap()),
                    "sample" => {
                        h.sample(action["value"].as_u64().unwrap() as u32).unwrap();
                        Value::Null
                    }
                    "rate" => json!(
                        h.take_rate(control_word)
                            .unwrap()
                            .to_le_bytes()
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()
                    ),
                    _ => panic!("unknown original oracle action"),
                };
                let result = execute(&mut h);
                assert_eq!(
                    result,
                    execute(&mut restored),
                    "codec result case{ci} action{ai}"
                );
                assert_eq!(h, restored, "codec state case{ci} action{ai}");
                assert_eq!(
                    result, expected["result"],
                    "native result case{ci} action{ai}"
                );
                assert_eq!(
                    json!(h.fields()),
                    expected["fields"],
                    "native fields case{ci} action{ai}"
                );
                let bytes: Vec<_> = std::iter::once(0u32)
                    .chain(h.timestamps.iter().copied())
                    .chain(h.fields())
                    .flat_map(u32::to_le_bytes)
                    .collect();
                assert_eq!(
                    json!(format!("{:x}", Sha256::digest(bytes))),
                    expected["raw_class_sha256"],
                    "native physical state case{ci} action{ai}"
                );
                restored = serde_json::from_value(serde_json::to_value(&h).unwrap()).unwrap();
                actions += 1;
            }
        }
        assert_eq!(actions, expected_actions);
    }
}
