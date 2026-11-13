use super::*;

#[test]
fn levels_preserve_original_extended_then_qword_rounding_boundaries() {
    for (input, expected) in [
        (0.0_f32, 0),
        (1.5, 0),
        (4.5, 0),
        (7.5, 1),
        (12.0, 3),
        (13.5, 3),
        (1537.5, 512),
        (3073.5, 1024),
        (24577.5, 8192),
    ] {
        assert_eq!(levels(input.to_bits()).unwrap(), expected);
        assert_eq!(reserve_bytes(input.to_bits()).unwrap(), (expected + 1) * 4);
    }
    assert_eq!(levels(1).unwrap(), 0);
    assert_eq!(levels(0x007fffff).unwrap(), 0);
    for boundary in [4.5_f32, 7.5, 13.5] {
        let bits = boundary.to_bits();
        assert_eq!(levels(bits - 1).unwrap(), levels(bits).unwrap());
        assert_eq!(levels(bits + 1).unwrap(), levels(bits).unwrap() + 1);
    }
}

#[test]
fn levels_reject_nonfinite_negative_signed_overflow_and_capacity_overflow() {
    for bits in [
        (-0.0_f32).to_bits(),
        (-1.0_f32).to_bits(),
        f32::INFINITY.to_bits(),
        f32::NEG_INFINITY.to_bits(),
        f32::NAN.to_bits(),
        f32::MAX.to_bits(),
    ] {
        assert!(levels(bits).is_err());
        assert!(reserve_bytes(bits).is_err());
    }
    let input = 4294967296.0_f32.to_bits();
    assert!(levels(input).is_ok());
    assert!(reserve_bytes(input).is_err());
}

#[test]
fn binary_rounding_preserves_ties_even_and_normalization_carry() {
    assert_eq!(rounded(20, -4, 3), (5, -2));
    assert_eq!(rounded(21, -4, 3), (5, -2));
    assert_eq!(rounded(22, -4, 3), (6, -2));
    assert_eq!(rounded(18, -4, 3), (4, -2));
    assert_eq!(rounded(31, -4, 3), (4, -1));
    assert_eq!(rounded(0, -4, 3), (0, -4));
}
