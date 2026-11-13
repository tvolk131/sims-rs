//! UI uses of4733b7/473390: finite endpoints0/1 with extended intermediates,
//! f32 storage and original __ftol duration conversion. A stored UI target must
//! be0 or1; this is not a general x87 implementation. Slopes/start samples can
//! legitimately become infinity/NaN through a zero rounded duration.
//! Exceptional arithmetic follows Intel SDM Vol.1,4.8.3 and8.5.1.2; comparison
//! with original transition instructions is separately tracked in the corpus.
use crate::numeric::Wide;

#[derive(Clone, Copy, Debug)]
enum Value {
    Finite(Wide),
    Infinity(bool),
    Nan(u32),
}
impl Value {
    fn load(bits: u32) -> Self {
        if let Some(v) = Wide::from_f32(bits) {
            Self::Finite(v)
        } else if bits & 0x7fffff == 0 {
            Self::Infinity(bits >> 31 != 0)
        } else {
            Self::Nan(bits | 0x400000)
        }
    }
    fn invalid() -> Self {
        Self::Nan(0xffc00000)
    }
    fn minus(self, rhs: Self) -> Self {
        match (self, rhs) {
            (Self::Nan(a), _) => Self::Nan(a),
            (_, Self::Nan(b)) => Self::Nan(b),
            (Self::Infinity(a), Self::Infinity(b)) if a == b => Self::invalid(),
            (Self::Infinity(a), _) => Self::Infinity(a),
            (_, Self::Infinity(b)) => Self::Infinity(!b),
            (Self::Finite(a), Self::Finite(b)) => Self::Finite(a.add(b.negated())),
        }
    }
    fn times_integer(self, n: i32) -> Self {
        match self {
            Self::Finite(v) => Self::Finite(v.times(Wide::integer(n))),
            Self::Infinity(_) if n == 0 => Self::invalid(),
            Self::Infinity(negative) => Self::Infinity(negative ^ (n < 0)),
            v => v,
        }
    }
    fn times_unsigned(self, n: u32) -> Self {
        match self {
            Self::Finite(v) => Self::Finite(v.times_unsigned(n)),
            Self::Infinity(_) if n == 0 => Self::invalid(),
            v => v,
        }
    }
    fn divided(self, n: i32) -> Self {
        match self {
            Self::Finite(v) if n == 0 => {
                if v.is_zero() {
                    Self::invalid()
                } else {
                    Self::Infinity(v.below_zero())
                }
            }
            Self::Finite(v) => Self::Finite(v.divided(Wide::integer(n)).unwrap()),
            Self::Infinity(negative) => Self::Infinity(negative ^ (n < 0)),
            v => v,
        }
    }
    fn bits(self) -> u32 {
        match self {
            Self::Finite(v) => v.f32_bits(),
            Self::Infinity(n) => 0x7f800000 | (u32::from(n) << 31),
            Self::Nan(bits) => bits,
        }
    }
    fn negative_or_unordered(self) -> bool {
        match self {
            Self::Finite(v) => v.below_zero(),
            Self::Infinity(n) => n,
            Self::Nan(_) => true,
        }
    }
    fn scaled_duration(self, n: i32) -> i32 {
        match self.times_integer(n) {
            Self::Finite(v) => v.add(Wide::half()).ftol(),
            _ => 0,
        }
    }
}
fn target(show: bool) -> Value {
    Value::Finite(Wide::integer(i32::from(show)))
}
fn sample_value(c: [u32; 4], now: u32) -> Result<Value, String> {
    if !matches!(c[0], 0 | 0x3f800000) {
        return Err("UI transition curve target is not0/1".into());
    }
    let target = Value::load(c[0]);
    Ok(if now >= c[3] {
        target
    } else {
        target.minus(Value::load(c[2]).times_unsigned(c[3] - now))
    })
}
pub fn configure(start_bits: u32, show: bool, duration: i32, now: u32) -> [u32; 4] {
    let slope = target(show)
        .minus(Value::load(start_bits))
        .divided(duration);
    let bits = slope.bits();
    [
        if show { 0x3f800000 } else { 0 },
        if slope.negative_or_unordered() {
            bits ^ 0x80000000
        } else {
            bits
        },
        bits,
        now.wrapping_add(duration as u32),
    ]
}
pub fn sample_bits(c: [u32; 4], now: u32) -> Result<u32, String> {
    Ok(sample_value(c, now)?.bits())
}
///5de47d/5de4d6 compute duration from the unspilled sample, then separately
/// sample and spill the new start to f32. The unsigned deadline test is retained.
pub fn reverse(
    c: [u32; 4],
    old_direction: i32,
    duration: i32,
    now: u32,
) -> Result<[u32; 4], String> {
    let sample = sample_value(c, now)?;
    let fraction = if old_direction < 0 {
        target(true).minus(sample)
    } else {
        sample
    };
    let new_duration = fraction.scaled_duration(duration);
    let second = sample_value(c, now)?;
    let start = if old_direction < 0 {
        target(true).minus(second)
    } else {
        second
    };
    Ok(configure(
        start.bits(),
        old_direction < 0,
        new_duration,
        now,
    ))
}
