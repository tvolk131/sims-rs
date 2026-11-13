//! Original5d53eb: preserve event order, rebuild durations, swap/clear the
//! deferred vectors and retain all three explicit RDTSC results.
use super::{Buffer, Record};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Duration {
    pub start: u64,
    pub microseconds_bits: u32,
    pub name: u32,
}
impl Duration {
    pub fn from_words(words: [u32; 4]) -> Self {
        Self {
            start: u64::from(words[0]) | (u64::from(words[1]) << 32),
            microseconds_bits: words[2],
            name: words[3],
        }
    }
    pub fn words(self) -> [u32; 4] {
        [
            self.start as u32,
            (self.start >> 32) as u32,
            self.microseconds_bits,
            self.name,
        ]
    }
}

/// Separate logical owners for the native vectors at+0,+c,+18,+24 and
/// the last record at+30. Pointer aliases are qualified by the source importer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// The original QEMU capture and instruction-only Unicorn oracle differ
    /// in converting the FDIV memory operand at PC24. Never conflate them.
    pub arithmetic: Arithmetic,
    pub events: Buffer,
    pub pending: Buffer<[u32; 4]>,
    pub published: Buffer<[u32; 4]>,
    pub durations: Buffer<Duration>,
    pub last: Record,
    pub frequency_bits: u64,
    pub enabled: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arithmetic {
    CapturedQemu72Pc24Memory,
    Unicorn2Pc24ExactMemory,
}

/// Exact signed integer ratio rounded once to binary24, ties to even. For
/// integral frequency1..2^32 and a signed64 delta all shifts fit u128;
/// the result is a normal f32. FILD itself does not round the integer input.
fn quotient(delta: i64, frequency: u64) -> f32 {
    if delta == 0 {
        return 0.;
    }
    let n = u128::from(delta.unsigned_abs());
    let d = u128::from(frequency);
    let mut exponent = n.ilog2() as i32 - d.ilog2() as i32;
    if (exponent >= 0 && n < d << exponent) || (exponent < 0 && n << -exponent < d) {
        exponent -= 1;
    }
    let shift = 23 - exponent;
    let (n, d) = if shift >= 0 {
        (n << shift, d)
    } else {
        (n, d << -shift)
    };
    let mut m = n / d;
    let r = n % d;
    m += u128::from(r * 2 > d || (r * 2 == d && m & 1 != 0));
    if m == 1 << 24 {
        m >>= 1;
        exponent += 1;
    }
    f32::from_bits(
        (u32::from(delta < 0) << 31) | (((exponent + 127) as u32) << 23) | ((m as u32) & 0x7fffff),
    )
}

impl State {
    pub fn validate(&self) -> Result<(), String> {
        self.events.validate()?;
        self.pending.validate()?;
        self.published.validate()?;
        self.durations.validate()?;
        Ok(())
    }
    /// Executes the qualified nonallocating tail. Unsupported allocation or
    /// arithmetic stops before mutation. Disabled profiling consumes no sample.
    pub fn finish_frame(&mut self, timing: Option<[u64; 3]>, control: u16) -> Result<(), String> {
        self.validate()?;
        if self.enabled == 0 {
            return if timing.is_none() {
                Ok(())
            } else {
                Err("disabled profiler consumes no RDTSC input".into())
            };
        }
        let [end, next_last, next_start] =
            timing.ok_or("profiler tail needs three original RDTSC inputs")?;
        if control != 0x003f {
            return Err("profiler tail arithmetic requires qualified FCW003f".into());
        }
        let frequency = f64::from_bits(self.frequency_bits);
        if !frequency.is_finite()
            || frequency < 1.
            || frequency > f64::from(u32::MAX)
            || frequency.fract() != 0.
        {
            return Err(
                "profiler frequency needs its qualified integral arithmetic profile".into(),
            );
        }
        let divisor = match self.arithmetic {
            Arithmetic::CapturedQemu72Pc24Memory => (frequency as f32) as u64,
            Arithmetic::Unicorn2Pc24ExactMemory => frequency as u64,
        };
        let required = self
            .events
            .records()
            .len()
            .checked_add(1)
            .ok_or("profiler event length overflow")?;
        if required as u64 > u64::from(self.events.capacity()) {
            return Err("profiler tail needs original event allocation child4b411c".into());
        }
        if required as u64 > u64::from(self.durations.capacity()) {
            return Err("profiler tail needs original duration allocation child5d56d7".into());
        }
        let mark = Record {
            timestamp: end,
            name: 0x6d6e3c,
            argument: 0,
        };
        let mut previous = self.last;
        let mut durations = Vec::with_capacity(required);
        for event in self.events.records().iter().chain(std::iter::once(&mark)) {
            let delta = event.timestamp.wrapping_sub(previous.timestamp) as i64;
            // Native FDIV rounds to24 before the subsequent FMUL1e6.
            let microseconds = quotient(delta, divisor) * 1_000_000_f32;
            durations.push(Duration {
                start: previous.timestamp,
                microseconds_bits: microseconds.to_bits(),
                name: previous.name,
            });
            previous = *event;
        }
        self.events.append(mark)?;
        self.durations.clear();
        for duration in durations {
            self.durations.append(duration)?;
        }
        self.events.clear();
        std::mem::swap(&mut self.pending, &mut self.published);
        self.pending.clear();
        self.last = Record {
            timestamp: next_last,
            name: 0x6d6e30,
            argument: 0,
        };
        self.events.append(Record {
            timestamp: next_start,
            name: 0x6d6e1c,
            argument: 0,
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Deserialize)]
    struct NextTime {
        before: [u32; 6],
        sample: u32,
        after: [u32; 6],
    }
    #[derive(Deserialize)]
    struct Native {
        before: State,
        after: State,
        timing: [u64; 3],
        control: u16,
        next_time: NextTime,
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn actual_tail_and_next_external_clock_with_restore() -> Result<(), Box<dyn std::error::Error>>
    {
        let native: Native = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/native-profiler-tail.json"
        ))?;
        let second: Native = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/native-profiler-tail-fpu.json"
        ))?;
        let mut another = second.before;
        another.finish_frame(Some(second.timing), second.control)?;
        assert_eq!(another, second.after);
        let mut next =
            super::super::super::application::Fields::from_words(second.next_time.before);
        next.update(second.next_time.sample);
        assert_eq!(next.words(), second.next_time.after);
        let mut state: State = serde_json::from_slice(&serde_json::to_vec(&native.before)?)?;
        let unchanged = serde_json::to_vec(&state)?;
        assert!(state.finish_frame(None, native.control).is_err());
        assert!(state.finish_frame(Some(native.timing), 0x037f).is_err());
        assert_eq!(serde_json::to_vec(&state)?, unchanged);
        state.finish_frame(Some(native.timing), native.control)?;
        assert_eq!(state, native.after);
        let mut clock =
            super::super::super::application::Fields::from_words(native.next_time.before);
        clock.update(native.next_time.sample);
        assert_eq!(clock.words(), native.next_time.after);
        let mut full = native.before.clone();
        full.events.capacity = full.events.records().len().try_into()?;
        let before = serde_json::to_vec(&full)?;
        assert!(
            full.finish_frame(Some(native.timing), native.control)
                .unwrap_err()
                .contains("4b411c")
        );
        assert_eq!(serde_json::to_vec(&full)?, before);
        let mut full = native.before;
        full.durations.capacity = 0;
        full.durations.clear();
        let before = serde_json::to_vec(&full)?;
        assert!(
            full.finish_frame(Some(native.timing), native.control)
                .unwrap_err()
                .contains("5d56d7")
        );
        assert_eq!(serde_json::to_vec(&full)?, before);
        Ok(())
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_instruction_tail_cases_under_the_separate_unicorn_profile()
    -> Result<(), Box<dyn std::error::Error>> {
        #[derive(Deserialize)]
        struct Case {
            before: State,
            after: State,
            timing: Option<[u64; 3]>,
            control: u16,
        }
        #[derive(Deserialize)]
        struct Oracle {
            cases: Vec<Case>,
        }
        let cases: Oracle = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/native-profiler-tail-cases.json"
        ))?;
        assert_eq!(cases.cases.len(), 161);
        for (index, case) in cases.cases.into_iter().enumerate() {
            assert_eq!(case.before.arithmetic, Arithmetic::Unicorn2Pc24ExactMemory);
            let mut current = case.before;
            current.finish_frame(case.timing, case.control)?;
            assert_eq!(current, case.after, "native instruction case{index}");
        }
        Ok(())
    }
}
