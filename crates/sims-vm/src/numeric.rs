//! Finite, nearest-even 64-significand intermediates for recovered scalar
//! routines. This is deliberately not a general x87 emulator. NaN/infinity and
//! masked invalid conversion are handled by the caller at the native sites.
#[derive(Clone, Copy, Debug)]
pub struct Wide {
    negative: bool,
    mantissa: u64,
    exponent: i32,
}
fn rounded(value: u128, shift: u32) -> u128 {
    if shift == 0 {
        return value;
    }
    if shift > 128 {
        return 0;
    }
    if shift == 128 {
        return u128::from(value > (1u128 << 127));
    }
    let q = value >> shift;
    let remainder = value & ((1u128 << shift) - 1);
    let half = 1u128 << (shift - 1);
    q + u128::from(remainder > half || remainder == half && q & 1 != 0)
}
impl Wide {
    fn pack(negative: bool, magnitude: u128, exponent: i32) -> Self {
        if magnitude == 0 {
            return Self {
                negative,
                mantissa: 0,
                exponent: 0,
            };
        }
        let bits = 128 - magnitude.leading_zeros();
        let (mut m, mut e) = if bits > 64 {
            (rounded(magnitude, bits - 64), exponent + (bits - 64) as i32)
        } else {
            (magnitude << (64 - bits), exponent - (64 - bits) as i32)
        };
        if m == 1u128 << 64 {
            m >>= 1;
            e += 1;
        }
        Self {
            negative,
            mantissa: m as u64,
            exponent: e,
        }
    }
    pub fn zero() -> Self {
        Self::pack(false, 0, 0)
    }
    pub fn integer(n: i32) -> Self {
        Self::pack(n < 0, i64::from(n).unsigned_abs() as u128, 0)
    }
    pub fn half() -> Self {
        Self::pack(false, 1, -1)
    }
    pub fn from_f32(bits: u32) -> Option<Self> {
        let e = (bits >> 23) & 255;
        let fraction = bits & 0x7fffff;
        if e == 255 {
            return None;
        }
        let (m, exponent) = if e == 0 {
            (fraction, -149)
        } else {
            (fraction | 0x800000, e as i32 - 150)
        };
        Some(Self::pack(bits >> 31 != 0, u128::from(m), exponent))
    }
    pub fn negated(mut self) -> Self {
        self.negative = !self.negative;
        self
    }
    pub fn is_zero(self) -> bool {
        self.mantissa == 0
    }
    pub fn below_zero(self) -> bool {
        self.negative && !self.is_zero()
    }
    pub fn add(self, other: Self) -> Self {
        if self.is_zero() && other.is_zero() {
            return Self::pack(self.negative && other.negative, 0, 0);
        }
        if self.is_zero() {
            return other;
        }
        if other.is_zero() {
            return self;
        }
        let base = self.exponent.max(other.exponent);
        // Three guard bits and a jammed sticky bit retain the direction of
        // discarded tails, including subtraction and cancellation.
        let aligned = |v: Self| -> u128 {
            let distance = (base - v.exponent) as u32;
            if distance <= 3 {
                return u128::from(v.mantissa) << (3 - distance);
            }
            let shift = distance - 3;
            if shift >= 64 {
                return 1;
            }
            u128::from((v.mantissa >> shift) | u64::from(v.mantissa & ((1u64 << shift) - 1) != 0))
        };
        let a = aligned(self);
        let b = aligned(other);
        if self.negative == other.negative {
            Self::pack(self.negative, a + b, base - 3)
        } else if a >= b {
            Self::pack(self.negative && a != b, a - b, base - 3)
        } else {
            Self::pack(other.negative, b - a, base - 3)
        }
    }
    pub fn times_unsigned(self, n: u32) -> Self {
        Self::pack(
            self.negative,
            u128::from(self.mantissa) * u128::from(n),
            self.exponent,
        )
    }
    pub fn times(self, other: Self) -> Self {
        Self::pack(
            self.negative ^ other.negative,
            u128::from(self.mantissa) * u128::from(other.mantissa),
            self.exponent + other.exponent,
        )
    }
    pub fn divided(self, denominator: Self) -> Option<Self> {
        if denominator.is_zero() {
            return None;
        }
        if self.is_zero() {
            return Some(Self::pack(self.negative ^ denominator.negative, 0, 0));
        }
        let shift = if self.mantissa >= denominator.mantissa {
            63
        } else {
            64
        };
        let numerator = u128::from(self.mantissa) << shift;
        let d = u128::from(denominator.mantissa);
        let mut q = numerator / d;
        let r = numerator % d;
        q += u128::from(r * 2 > d || r * 2 == d && q & 1 != 0);
        Some(Self::pack(
            self.negative ^ denominator.negative,
            q,
            self.exponent - denominator.exponent - shift as i32,
        ))
    }
    /// Square root after the original squared distance spills to double.
    pub fn distance(x: i32, y: i32) -> Self {
        let sum = (i128::from(x) * i128::from(x) + i128::from(y) * i128::from(y)) as u64;
        let stored = (sum as f64) as u64;
        if stored == 0 {
            return Self::zero();
        }
        let exponent = (63 - stored.leading_zeros() as i32) / 2 - 63;
        let radicand = u128::from(stored) << (-2 * exponent) as u32;
        let mut remainder = radicand;
        let mut root = 0u128;
        let mut bit = 1u128 << 126;
        while bit > remainder {
            bit >>= 2;
        }
        while bit != 0 {
            if remainder >= root + bit {
                remainder -= root + bit;
                root = (root >> 1) + bit;
            } else {
                root >>= 1;
            }
            bit >>= 2;
        }
        // sqrt(N) is above q+1/2 exactly when N-q*q > q for integer N.
        root += u128::from(radicand - root * root > root);
        Self::pack(false, root, exponent)
    }
    pub fn f32_bits(self) -> u32 {
        let sign = u32::from(self.negative) << 31;
        if self.is_zero() {
            return sign;
        }
        let mut e = self.exponent + 63;
        if e < -126 {
            let shift = (-149 - self.exponent) as u32;
            return sign | rounded(u128::from(self.mantissa), shift) as u32;
        }
        let mut m = rounded(u128::from(self.mantissa), 40);
        if m == 1 << 24 {
            m >>= 1;
            e += 1;
        }
        if e > 127 {
            return sign | 0x7f800000;
        }
        sign | (((e + 127) as u32) << 23) | (m as u32 & 0x7fffff)
    }
    /// Low EAX word of original __ftol's signed-qword truncating store.
    /// Masked invalid conversion stores INT64_MIN, whose low word is zero.
    pub fn ftol(self) -> i32 {
        if self.is_zero() {
            return 0;
        }
        let bits = self.exponent + 64;
        if bits > 64 || bits < 1 {
            return 0;
        }
        let magnitude = if self.exponent >= 0 {
            u128::from(self.mantissa) << self.exponent as u32
        } else {
            u128::from(self.mantissa >> (-self.exponent) as u32)
        };
        if magnitude > 1u128 << 63 || magnitude == 1u128 << 63 && !self.negative {
            return 0;
        }
        let low = magnitude as u32;
        if self.negative {
            low.wrapping_neg() as i32
        } else {
            low as i32
        }
    }
    pub fn rounded_integer(self) -> i32 {
        self.add(if self.below_zero() {
            Self::half().negated()
        } else {
            Self::half()
        })
        .ftol()
    }
}
