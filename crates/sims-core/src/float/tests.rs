use super::*;

/// SplitMix64, used only to generate test inputs.
struct Inputs(u64);

impl Inputs {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A mix of arbitrary bit patterns (including subnormals, infinities and
    /// NaNs), values of similar magnitude (to exercise cancellation and
    /// rounding), and hand-picked edge cases.
    fn f32_bits(&mut self) -> u32 {
        const EDGES: [u32; 12] = [
            0x0000_0000,
            0x8000_0000,
            0x7f80_0000,
            0xff80_0000,
            0x7fc0_0000,
            0x0000_0001,
            0x007f_ffff,
            0x0080_0000,
            0x7f7f_ffff,
            0x3f80_0000,
            0xbf80_0000,
            0x4b80_0000,
        ];
        let r = self.next();
        match r % 8 {
            0 => EDGES[(r >> 8) as usize % EDGES.len()],
            1..=3 => r as u32 >> 3 | (r >> 60) as u32 & 1 << 31,
            _ => {
                let exponent = 120 + (r >> 32) as u32 % 16;
                let sign = (r >> 40) as u32 & 1;
                sign << 31 | exponent << 23 | (r as u32 & 0x007f_ffff)
            }
        }
    }
}

fn f(bits: u32) -> F32 {
    F32::from_bits(bits)
}

/// From f32 inputs, an extended-precision operation followed by a store gives
/// the correctly rounded f32 result, because 64 >= 2 * 24 + 2 makes double
/// rounding harmless for these operations. Hardware f32 arithmetic is
/// therefore an exact reference.
#[test]
fn single_operations_then_store_match_ieee_f32() {
    let mut inputs = Inputs(1);
    for _ in 0..400_000 {
        let (a, b) = (inputs.f32_bits(), inputs.f32_bits());
        let (ha, hb) = (f32::from_bits(a), f32::from_bits(b));
        let (ea, eb) = (f(a).ext(), f(b).ext());
        let cases = [
            ("+", F32::from_f32(ha + hb), (ea + eb).to_f32()),
            ("-", F32::from_f32(ha - hb), (ea - eb).to_f32()),
            ("*", F32::from_f32(ha * hb), (ea * eb).to_f32()),
            ("/", F32::from_f32(ha / hb), (ea / eb).to_f32()),
            ("sqrt", F32::from_f32(ha.sqrt()), ea.sqrt().to_f32()),
        ];
        for (op, expected, actual) in cases {
            assert_eq!(actual, expected, "{a:#010x} {op} {b:#010x}");
        }
    }
}

#[test]
fn comparisons_match_ieee_f32() {
    let mut inputs = Inputs(2);
    for _ in 0..200_000 {
        let (a, b) = (inputs.f32_bits(), inputs.f32_bits());
        let expected = f32::from_bits(a).partial_cmp(&f32::from_bits(b));
        assert_eq!(
            f(a).ext().partial_cmp(&f(b).ext()),
            expected,
            "{a:#010x} vs {b:#010x}"
        );
    }
}

#[test]
fn intermediates_keep_extended_precision_until_stored() {
    let big = Ext::from_i32(1 << 24);
    let one = Ext::from_i32(1);
    // In f32, 2^24 + 1 rounds back to 2^24 and the sum below would be 0.
    assert_eq!((big + one - big).to_f32(), F32::from_f32(1.0));
    // Storing in between loses the bit, as the original would.
    assert_eq!(((big + one).to_f32().ext() - big).to_f32(), F32::ZERO);
    // 64-bit significand: 2^63 + 1 is exact, 2^64 + 1 is not.
    let two_63 = Ext::from_i32(1 << 30) * Ext::from_i32(1 << 30) * Ext::from_i32(8);
    assert!(two_63 + one > two_63);
    assert!(two_63 + two_63 + one == two_63 + two_63);
}

#[test]
fn integer_loads_round_trip() {
    let mut inputs = Inputs(3);
    for n in [0, 1, -1, i32::MAX, i32::MIN, 1 << 24, -(1 << 24) - 1] {
        assert_eq!(Ext::from_i32(n).to_i32_round(), n);
        assert_eq!(Ext::from_i32(n).to_i32_truncate(), n);
    }
    for _ in 0..10_000 {
        let n = inputs.next() as i32;
        assert_eq!(Ext::from_i32(n).to_i32_round(), n);
        assert_eq!(Ext::from_i32(n).to_i32_truncate(), n);
    }
}

#[test]
fn truncating_conversion_matches_msvc_ftol() {
    let value = |x: f32| F32::from_f32(x).ext();
    assert_eq!(value(2.7).to_i32_truncate(), 2);
    assert_eq!(value(-2.7).to_i32_truncate(), -2);
    assert_eq!(value(0.999).to_i32_truncate(), 0);
    assert_eq!(value(-0.0).to_i32_truncate(), 0);
    // Values beyond i32 keep the low word of the 64-bit result.
    assert_eq!(value(2_147_483_648.0).to_i32_truncate(), i32::MIN);
    assert_eq!(value(4_294_967_296.0).to_i32_truncate(), 0);
    assert_eq!(value(4_294_967_808.0).to_i32_truncate(), 512);
    // Beyond i64, infinity and NaN: integer indefinite, low word 0.
    assert_eq!(value(9.3e18).to_i32_truncate(), 0);
    assert_eq!(value(f32::INFINITY).to_i32_truncate(), 0);
    assert_eq!(value(f32::NAN).to_i32_truncate(), 0);
    // Agrees with a 64-bit truncating cast for everything in range.
    let mut inputs = Inputs(4);
    for _ in 0..200_000 {
        let x = f32::from_bits(inputs.f32_bits());
        if x.is_finite() && x.abs() < 9.2e18 {
            assert_eq!(value(x).to_i32_truncate(), x as i64 as i32, "{x}");
        }
    }
}

#[test]
fn rounding_conversion_matches_fistp() {
    let value = |x: f32| F32::from_f32(x).ext();
    assert_eq!(value(2.5).to_i32_round(), 2);
    assert_eq!(value(3.5).to_i32_round(), 4);
    assert_eq!(value(-2.5).to_i32_round(), -2);
    assert_eq!(value(-3.5).to_i32_round(), -4);
    assert_eq!(value(0.49999997).to_i32_round(), 0);
    assert_eq!(value(-0.5).to_i32_round(), 0);
    assert_eq!(value(2_147_483_520.0).to_i32_round(), 2_147_483_520);
    assert_eq!(value(2_147_483_648.0).to_i32_round(), i32::MIN);
    assert_eq!(value(-2_147_483_648.0).to_i32_round(), i32::MIN);
    assert_eq!(value(f32::INFINITY).to_i32_round(), i32::MIN);
    assert_eq!(value(f32::NAN).to_i32_round(), i32::MIN);
    assert_eq!(value(1.0e-30).to_i32_round(), 0);
}

#[test]
fn masked_exceptions_give_ieee_special_values() {
    let one = Ext::from_i32(1);
    let zero = Ext::ZERO;
    assert_eq!((one / zero).to_f32(), F32::from_f32(f32::INFINITY));
    assert_eq!((-one / zero).to_f32(), F32::from_f32(f32::NEG_INFINITY));
    assert!((zero / zero).is_nan());
    assert!((-one).sqrt().is_nan());
    assert_eq!((-zero).sqrt().to_f32().to_bits(), 0x8000_0000);
    // Overflow on store.
    let big = F32::from_f32(f32::MAX).ext();
    assert_eq!((big + big).to_f32(), F32::from_f32(f32::INFINITY));
    // ...but not in the register, which has a wider exponent range.
    assert_eq!((big + big - big).to_f32(), F32::from_f32(f32::MAX));
    // Register overflow.
    let mut x = big;
    for _ in 0..12 {
        x = x * x;
    }
    assert_eq!(x.to_f32(), F32::from_f32(f32::INFINITY));
}

#[test]
fn nans_are_normalized_in_storage() {
    for bits in [0x7fc0_0000, 0x7f80_0001, 0xffc0_0001, 0xffff_ffff] {
        assert_eq!(F32::from_bits(bits).to_bits(), CANONICAL_NAN_BITS);
        assert!(F32::from_bits(bits).is_nan());
    }
    assert_eq!(F32::from_bits(0x7f80_0000).to_bits(), 0x7f80_0000);
    assert_eq!(
        (-Ext::from_f32(F32::from_bits(0x7fc0_0000)))
            .to_f32()
            .to_bits(),
        CANONICAL_NAN_BITS
    );
}

#[test]
fn serializes_as_exact_bits() {
    let value = F32::from_f32(0.1);
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(json, "1036831949");
    assert_eq!(serde_json::from_str::<F32>(&json).unwrap(), value);
    let nan: F32 = serde_json::from_str("2143289345").unwrap();
    assert_eq!(nan.to_bits(), CANONICAL_NAN_BITS);
}

/// Checks a register-precision square root exactly: with `x = R * 2^(2e)`
/// and `root = r * 2^e`, the root is correctly rounded when
/// `r^2 - r < R <= r^2 + r`.
fn assert_sqrt_correctly_rounded(x: Ext) {
    let root = x.sqrt();
    let shift = x.exponent - 2 * root.exponent;
    assert!((0..=64).contains(&shift), "{x:?} -> {root:?}");
    let radicand = u128::from(x.mantissa) << shift;
    let r = u128::from(root.mantissa);
    let square = r * r;
    let ok = if radicand >= square {
        radicand - square <= r
    } else {
        square - radicand < r
    };
    assert!(ok, "sqrt({x:?}) = {root:?} is not correctly rounded");
}

#[test]
fn square_roots_round_correctly_at_register_precision() {
    // 1 + 2^-63 has root 1 + 2^-64 - tiny, which must round down to 1.
    let one = Ext::from_i32(1);
    let two_31 = Ext::from_i32(1 << 30) * Ext::from_i32(2);
    let ulp = one / (two_31 * two_31 * Ext::from_i32(2));
    assert_eq!((one + ulp).sqrt(), one);
    assert_sqrt_correctly_rounded(one + ulp);

    let mut inputs = Inputs(5);
    for _ in 0..200_000 {
        let x = Ext {
            class: Class::Finite,
            negative: false,
            mantissa: inputs.next() | 1 << 63,
            exponent: (inputs.next() % 200) as i32 - 100,
        };
        assert_sqrt_correctly_rounded(x);
    }
}

/// Addition at register precision against an exact reference: with
/// exponents at most 62 apart, both operands align into a u128 without
/// losing bits, and the exact sum is rounded once.
#[test]
fn sums_round_correctly_at_register_precision() {
    let mut inputs = Inputs(6);
    for _ in 0..400_000 {
        let make = |mantissa: u64, exponent: i32, negative: bool| Ext {
            class: Class::Finite,
            negative,
            mantissa: mantissa | 1 << 63,
            exponent,
        };
        let a = make(inputs.next(), 0, inputs.next() & 1 != 0);
        // Mostly tiny distances and near-equal mantissas, to hit cancellation.
        let distance = match inputs.next() % 4 {
            0 => 0,
            1 => 1,
            _ => (inputs.next() % 63) as i32,
        };
        let mantissa = if inputs.next().is_multiple_of(4) {
            a.mantissa ^ (inputs.next() & 0xffff)
        } else {
            inputs.next()
        };
        let b = make(mantissa, -distance, inputs.next() & 1 != 0);
        let wide_a = u128::from(a.mantissa) << distance;
        let wide_b = u128::from(b.mantissa);
        let expected = if a.negative == b.negative {
            Ext::finite(a.negative, wide_a + wide_b, -distance)
        } else if wide_a >= wide_b {
            Ext::finite(a.negative && wide_a != wide_b, wide_a - wide_b, -distance)
        } else {
            Ext::finite(b.negative, wide_b - wide_a, -distance)
        };
        let actual = a + b;
        assert!(
            actual.class == expected.class
                && actual.negative == expected.negative
                && actual.mantissa == expected.mantissa
                && actual.exponent == expected.exponent,
            "{a:?} + {b:?}: {actual:?} != {expected:?}"
        );
    }
}
