//! The simulation float model.
//!
//! The original game stores quantities such as motives as 32-bit floats but
//! computes with the x87 FPU, whose registers hold a 64-bit significand.
//! Results are rounded to 32 bits only when stored back to memory. Two types
//! model that split:
//!
//! - [`F32`] is the stored form. It is plain bits with NaNs normalized, so it
//!   compares, hashes and serializes identically on every machine.
//! - [`Ext`] is an FPU register: a 64-bit significand with round-to-nearest-
//!   even after every operation, implemented with integer arithmetic only.
//!
//! Arithmetic happens on [`Ext`], and conversion back to [`F32`] is explicit
//! with [`Ext::to_f32`], so every store (and its rounding) is visible in the
//! code. Whether a particular original routine kept a value in a register or
//! stored it in between is something the reference oracle decides; this module
//! is the single place where that behavior is defined.
//!
//! Exceptions are masked, as in a default Windows process: invalid operations
//! give NaN, division by zero gives infinity, and overflow gives infinity.
//! NaN payloads are not modeled. Every NaN is the x87 "real indefinite"
//! pattern, because scripts can't observe payloads. Values below the x87
//! normal range (around 2^-16382) flush to zero instead of becoming x87
//! denormals; no sequence of operations on game data reaches that range.

use core::cmp::Ordering;
use core::ops::{Add, Div, Mul, Neg, Sub};
use serde::{Deserialize, Serialize};

/// The bits every NaN is normalized to: the x87 "real indefinite" (negative
/// quiet NaN) that the FPU produces for invalid operations.
pub const CANONICAL_NAN_BITS: u32 = 0xffc0_0000;

/// A stored 32-bit float.
///
/// Equality and hashing compare bits, so `+0` and `-0` differ. That is right
/// for state comparison and fingerprints. Numeric comparison goes through
/// [`Ext`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "u32", into = "u32")]
pub struct F32(u32);

impl F32 {
    pub const ZERO: Self = Self(0);

    /// Creates a value from IEEE single-precision bits, normalizing NaNs.
    pub const fn from_bits(bits: u32) -> Self {
        if bits & 0x7f80_0000 == 0x7f80_0000 && bits & 0x007f_ffff != 0 {
            Self(CANONICAL_NAN_BITS)
        } else {
            Self(bits)
        }
    }

    pub const fn to_bits(self) -> u32 {
        self.0
    }

    /// Converts a host `f32`, such as a constant written in source code.
    pub fn from_f32(value: f32) -> Self {
        Self::from_bits(value.to_bits())
    }

    /// The host `f32` with the same bits, for presentation only. Never do
    /// simulation arithmetic on the result.
    pub fn to_f32(self) -> f32 {
        f32::from_bits(self.0)
    }

    /// Loads the value into an FPU register (exact).
    pub fn ext(self) -> Ext {
        Ext::from_f32(self)
    }

    pub const fn is_nan(self) -> bool {
        self.0 == CANONICAL_NAN_BITS
    }
}

impl From<u32> for F32 {
    fn from(bits: u32) -> Self {
        Self::from_bits(bits)
    }
}

impl From<F32> for u32 {
    fn from(value: F32) -> Self {
        value.0
    }
}

impl core::fmt::Debug for F32 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "F32({:?})", self.to_f32())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Zero,
    Finite,
    Infinite,
    Nan,
}

/// An x87 extended-precision register value.
///
/// A finite value is `mantissa * 2^exponent`, with `mantissa` normalized so
/// its top bit is set.
#[derive(Clone, Copy, Debug)]
pub struct Ext {
    class: Class,
    negative: bool,
    mantissa: u64,
    exponent: i32,
}

/// Largest and smallest unbiased exponents of the leading bit in the x87
/// extended format's normal range.
const MAX_LEADING_EXPONENT: i32 = 16383;
const MIN_LEADING_EXPONENT: i32 = -16382;

/// `value / 2^shift`, rounded to nearest with ties to even.
fn round_shift(value: u128, shift: u32) -> u128 {
    match shift {
        0 => value,
        1..=127 => {
            let quotient = value >> shift;
            let remainder = value & ((1u128 << shift) - 1);
            let half = 1u128 << (shift - 1);
            quotient + u128::from(remainder > half || remainder == half && quotient & 1 != 0)
        }
        128 => u128::from(value > 1u128 << 127),
        _ => 0,
    }
}

/// Floor of the square root.
fn isqrt(value: u128) -> u128 {
    let mut remainder = value;
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
    root
}

impl Ext {
    pub const ZERO: Self = Self::zero(false);
    const NAN: Self = Self {
        class: Class::Nan,
        negative: true,
        mantissa: 0,
        exponent: 0,
    };

    const fn zero(negative: bool) -> Self {
        Self {
            class: Class::Zero,
            negative,
            mantissa: 0,
            exponent: 0,
        }
    }

    const fn infinity(negative: bool) -> Self {
        Self {
            class: Class::Infinite,
            negative,
            mantissa: 0,
            exponent: 0,
        }
    }

    /// Rounds `magnitude * 2^exponent` to a 64-bit significand.
    fn finite(negative: bool, magnitude: u128, exponent: i32) -> Self {
        if magnitude == 0 {
            return Self::zero(negative);
        }
        let bits = 128 - magnitude.leading_zeros();
        let (mut mantissa, mut exponent) = if bits > 64 {
            (
                round_shift(magnitude, bits - 64),
                exponent + (bits - 64) as i32,
            )
        } else {
            (magnitude << (64 - bits), exponent - (64 - bits) as i32)
        };
        if mantissa == 1u128 << 64 {
            mantissa >>= 1;
            exponent += 1;
        }
        let leading = exponent.saturating_add(63);
        if leading > MAX_LEADING_EXPONENT {
            return Self::infinity(negative);
        }
        if leading < MIN_LEADING_EXPONENT {
            return Self::zero(negative);
        }
        Self {
            class: Class::Finite,
            negative,
            mantissa: mantissa as u64,
            exponent,
        }
    }

    /// Loads an integer (`FILD`, exact).
    pub fn from_i32(value: i32) -> Self {
        Self::finite(value < 0, u128::from(value.unsigned_abs()), 0)
    }

    /// Loads a stored float (`FLD m32`, exact).
    pub fn from_f32(value: F32) -> Self {
        let bits = value.to_bits();
        let negative = bits >> 31 != 0;
        let biased = (bits >> 23) & 0xff;
        let fraction = bits & 0x007f_ffff;
        match biased {
            0xff if fraction == 0 => Self::infinity(negative),
            0xff => Self::NAN,
            0 => Self::finite(negative, u128::from(fraction), -149),
            _ => Self::finite(
                negative,
                u128::from(fraction | 0x0080_0000),
                biased as i32 - 150,
            ),
        }
    }

    /// Stores to 32 bits (`FSTP m32`), rounding to nearest even. Overflow
    /// gives infinity and tiny values become subnormal or zero.
    pub fn to_f32(self) -> F32 {
        let sign = u32::from(self.negative) << 31;
        match self.class {
            Class::Nan => F32(CANONICAL_NAN_BITS),
            Class::Zero => F32(sign),
            Class::Infinite => F32(sign | 0x7f80_0000),
            Class::Finite => {
                let mut leading = self.exponent + 63;
                if leading < -126 {
                    // Subnormal: the significand is value / 2^-149. Rounding
                    // up to 2^23 yields the smallest normal bit pattern.
                    let shift = (-149i64 - i64::from(self.exponent)).min(129) as u32;
                    let m = round_shift(u128::from(self.mantissa), shift) as u32;
                    return F32(sign | m);
                }
                let mut m = round_shift(u128::from(self.mantissa), 40);
                if m == 1 << 24 {
                    m >>= 1;
                    leading += 1;
                }
                if leading > 127 {
                    return F32(sign | 0x7f80_0000);
                }
                F32(sign | (((leading + 127) as u32) << 23) | (m as u32 & 0x007f_ffff))
            }
        }
    }

    /// Converts like MSVC's `__ftol`, which C casts such as `(int)x` compile
    /// to: truncate toward zero to a 64-bit integer and keep the low 32
    /// bits. Out-of-range values and NaN produce the 64-bit "integer
    /// indefinite", whose low word is 0.
    pub fn to_i32_truncate(self) -> i32 {
        if self.class != Class::Finite {
            return 0;
        }
        let integer_bits = self.exponent + 64;
        if !(1..=64).contains(&integer_bits) {
            return 0;
        }
        let magnitude = u128::from(self.mantissa >> (-self.exponent) as u32);
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

    /// Converts like `FISTP m32` in the default rounding mode: round to
    /// nearest even. Out-of-range values and NaN give `i32::MIN`, the 32-bit
    /// "integer indefinite".
    pub fn to_i32_round(self) -> i32 {
        match self.class {
            Class::Zero => return 0,
            Class::Nan | Class::Infinite => return i32::MIN,
            Class::Finite => {}
        }
        if self.exponent >= 0 {
            return i32::MIN;
        }
        let shift = (-i64::from(self.exponent)).min(129) as u32;
        let magnitude = round_shift(u128::from(self.mantissa), shift);
        if self.negative {
            if magnitude > 1u128 << 31 {
                i32::MIN
            } else {
                (magnitude as i64).wrapping_neg() as i32
            }
        } else if magnitude > i32::MAX as u128 {
            i32::MIN
        } else {
            magnitude as i32
        }
    }

    /// `FSQRT`. Negative inputs give NaN; `-0` stays `-0`.
    pub fn sqrt(self) -> Self {
        match self.class {
            Class::Nan => Self::NAN,
            Class::Zero => self,
            _ if self.negative => Self::NAN,
            Class::Infinite => self,
            Class::Finite => {
                // Scale to a radicand in [2^126, 2^128) with an even exponent,
                // so the integer root has exactly 64 bits.
                let (radicand, exponent) = if self.exponent % 2 == 0 {
                    (u128::from(self.mantissa) << 64, (self.exponent - 64) / 2)
                } else {
                    (u128::from(self.mantissa) << 63, (self.exponent - 63) / 2)
                };
                let root = isqrt(radicand);
                // The exact root lies in (root, root + 1). It is nearer the
                // upper bound exactly when radicand - root^2 > root, and a
                // tie is impossible for an integer radicand.
                let round_up = radicand - root * root > root;
                Self::finite(false, root + u128::from(round_up), exponent)
            }
        }
    }

    pub fn abs(self) -> Self {
        match self.class {
            Class::Nan => Self::NAN,
            _ => Self {
                negative: false,
                ..self
            },
        }
    }

    pub fn is_nan(self) -> bool {
        self.class == Class::Nan
    }

    pub fn is_zero(self) -> bool {
        self.class == Class::Zero
    }

    /// True for values below zero (not `-0`, not NaN).
    pub fn is_negative(self) -> bool {
        self.negative && matches!(self.class, Class::Finite | Class::Infinite)
    }

    fn magnitude_cmp(self, other: Self) -> Ordering {
        let rank = |c: Class| match c {
            Class::Zero => 0,
            Class::Finite => 1,
            Class::Infinite | Class::Nan => 2,
        };
        rank(self.class).cmp(&rank(other.class)).then_with(|| {
            if self.class == Class::Finite {
                (self.exponent, self.mantissa).cmp(&(other.exponent, other.mantissa))
            } else {
                Ordering::Equal
            }
        })
    }
}

impl Neg for Ext {
    type Output = Self;
    fn neg(self) -> Self {
        match self.class {
            Class::Nan => Self::NAN,
            _ => Self {
                negative: !self.negative,
                ..self
            },
        }
    }
}

impl Add for Ext {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        use Class::*;
        match (self.class, other.class) {
            (Nan, _) | (_, Nan) => Self::NAN,
            (Infinite, Infinite) if self.negative != other.negative => Self::NAN,
            (Infinite, _) => self,
            (_, Infinite) => other,
            (Zero, Zero) => Self::zero(self.negative && other.negative),
            (Zero, _) => other,
            (_, Zero) => self,
            (Finite, Finite) => {
                let base = self.exponent.max(other.exponent);
                // Three guard bits plus a jammed sticky bit keep enough of
                // any discarded tail to round correctly, including after
                // cancellation.
                let aligned = |v: Self| -> u128 {
                    let distance = (base - v.exponent) as u32;
                    if distance <= 3 {
                        return u128::from(v.mantissa) << (3 - distance);
                    }
                    let shift = distance - 3;
                    if shift >= 64 {
                        return 1;
                    }
                    let sticky = v.mantissa & ((1u64 << shift) - 1) != 0;
                    u128::from((v.mantissa >> shift) | u64::from(sticky))
                };
                let (a, b) = (aligned(self), aligned(other));
                if self.negative == other.negative {
                    Self::finite(self.negative, a + b, base - 3)
                } else if a >= b {
                    // Exact cancellation gives +0 in round-to-nearest.
                    Self::finite(self.negative && a != b, a - b, base - 3)
                } else {
                    Self::finite(other.negative, b - a, base - 3)
                }
            }
        }
    }
}

impl Sub for Ext {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        self + -other
    }
}

impl Mul for Ext {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        use Class::*;
        let negative = self.negative ^ other.negative;
        match (self.class, other.class) {
            (Nan, _) | (_, Nan) => Self::NAN,
            (Infinite, Zero) | (Zero, Infinite) => Self::NAN,
            (Infinite, _) | (_, Infinite) => Self::infinity(negative),
            (Zero, _) | (_, Zero) => Self::zero(negative),
            (Finite, Finite) => Self::finite(
                negative,
                u128::from(self.mantissa) * u128::from(other.mantissa),
                self.exponent + other.exponent,
            ),
        }
    }
}

impl Div for Ext {
    type Output = Self;
    fn div(self, other: Self) -> Self {
        use Class::*;
        let negative = self.negative ^ other.negative;
        match (self.class, other.class) {
            (Nan, _) | (_, Nan) => Self::NAN,
            (Infinite, Infinite) | (Zero, Zero) => Self::NAN,
            (Infinite, _) => Self::infinity(negative),
            (_, Infinite) => Self::zero(negative),
            (_, Zero) => Self::infinity(negative),
            (Zero, _) => Self::zero(negative),
            (Finite, Finite) => {
                let shift = if self.mantissa >= other.mantissa {
                    63
                } else {
                    64
                };
                let numerator = u128::from(self.mantissa) << shift;
                let denominator = u128::from(other.mantissa);
                let mut quotient = numerator / denominator;
                let remainder = numerator % denominator;
                quotient += u128::from(
                    remainder * 2 > denominator
                        || remainder * 2 == denominator && quotient & 1 != 0,
                );
                Self::finite(negative, quotient, self.exponent - other.exponent - shift)
            }
        }
    }
}

impl PartialEq for Ext {
    fn eq(&self, other: &Self) -> bool {
        self.partial_cmp(other) == Some(Ordering::Equal)
    }
}

impl PartialOrd for Ext {
    /// Numeric order, like `FCOM`: NaN is unordered and `-0 == +0`.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.is_nan() || other.is_nan() {
            return None;
        }
        if self.is_zero() && other.is_zero() {
            return Some(Ordering::Equal);
        }
        let below = |v: &Self| v.negative && !v.is_zero();
        Some(match (below(self), below(other)) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => self.magnitude_cmp(*other),
            (true, true) => other.magnitude_cmp(*self),
        })
    }
}

#[cfg(test)]
mod tests;
