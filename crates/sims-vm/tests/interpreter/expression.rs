//! Expression: every operator, on integer words and on motives.

use crate::support::*;
use sims_vm::Trap;

const TEMPS: u8 = scope::TEMPS;
const LITERAL: u8 = scope::LITERAL;

/// Runs `temp 0 <operator> literal rhs` with temp 0 starting at `lhs`, and
/// returns the branch taken and temp 0 afterwards.
fn apply(lhs: i16, operator: u8, rhs: i16) -> (bool, i16) {
    let mut fx = Fixture::new();
    fx.lamp_mut().temps[0] = lhs;
    let result = fx
        .expression(TEMPS, 0, operator, LITERAL, rhs)
        .unwrap_or_else(|f| panic!("{f}"));
    (result, fx.lamp().temps[0])
}

fn fails(lhs: i16, operator: u8, rhs: i16) -> Trap {
    let mut fx = Fixture::new();
    fx.lamp_mut().temps[0] = lhs;
    trap(fx.expression(TEMPS, 0, operator, LITERAL, rhs))
}

#[test]
fn comparisons() {
    for (operator, cases) in [
        (0, [(2, 1, true), (1, 1, false), (-5, 3, false)]),
        (1, [(1, 2, true), (1, 1, false), (-5, 3, true)]),
        (2, [(4, 4, true), (4, 5, false), (-1, -1, true)]),
        (14, [(2, 1, true), (1, 1, true), (0, 1, false)]),
        (15, [(1, 2, true), (1, 1, true), (2, 1, false)]),
        (16, [(1, 2, true), (1, 1, false), (-1, 1, true)]),
    ] {
        for (lhs, rhs, expected) in cases {
            // Comparisons leave the left operand alone.
            assert_eq!(
                apply(lhs, operator, rhs),
                (expected, lhs),
                "{lhs} op {operator} {rhs}"
            );
        }
    }
}

#[test]
fn comparisons_read_but_never_write() {
    // A literal left operand compares fine.
    let mut fx = Fixture::new();
    assert_eq!(fx.expression(LITERAL, 3, 0, LITERAL, 2), Ok(true));
    // Global 5 has no word pointer, but a comparison needs none.
    assert_eq!(fx.expression(scope::GLOBAL, 5, 2, LITERAL, 105), Ok(true));
}

#[test]
fn arithmetic() {
    assert_eq!(apply(5, 3, 3), (true, 8));
    assert_eq!(apply(5, 4, 7), (true, -2));
    assert_eq!(apply(5, 5, -9), (true, -9));
    assert_eq!(apply(-6, 6, 7), (true, -42));
    assert_eq!(apply(7, 7, 2), (true, 3));
    assert_eq!(apply(-7, 7, 2), (true, -3));
    assert_eq!(apply(0b1100, 13, 0b1010), (true, 0b1000));
    assert_eq!(apply(0b1100, 18, 0b1010), (true, 0b1110));
    assert_eq!(apply(0b1100, 19, 0b1010), (true, 0b0110));
    // Results wrap to 16 bits.
    assert_eq!(apply(32767, 3, 1), (true, -32768));
    assert_eq!(apply(-32768, 4, 1), (true, 32767));
    assert_eq!(apply(300, 6, 300), (true, 90000i32 as i16));
    assert_eq!(apply(-32768, 7, -1), (true, -32768));
}

#[test]
fn division_by_zero() {
    assert_eq!(fails(5, 7, 0).code(), Some(0x26));
    assert_eq!(fails(5, 12, 0).code(), Some(0x26));
}

#[test]
fn remainder_uses_absolute_values() {
    assert_eq!(apply(7, 12, 3), (true, 1));
    assert_eq!(apply(-7, 12, 3), (true, 1));
    assert_eq!(apply(7, 12, -3), (true, 1));
    // -32768 has no 16-bit absolute value and stays negative.
    assert_eq!(apply(-32768, 12, 3), (true, -2));
    // The destination must be writable before zero is checked.
    let mut fx = Fixture::new();
    assert_eq!(code(fx.expression(LITERAL, 7, 12, LITERAL, 0)), Some(0x13));
}

#[test]
fn flags() {
    // Flags count from 1.
    assert_eq!(apply(0b0100, 8, 3), (true, 0b0100));
    assert_eq!(apply(0b0100, 8, 2), (false, 0b0100));
    assert_eq!(apply(0, 9, 4), (true, 0b1000));
    assert_eq!(apply(0b1111, 10, 2), (true, 0b1101));
    assert_eq!(apply(0, 9, 16), (true, i16::MIN));
    // Flag 0 is the sign bit after sign extension: setting or clearing it
    // leaves the word as it was.
    assert_eq!(apply(-1, 8, 0), (true, -1));
    assert_eq!(apply(1, 8, 0), (false, 1));
    assert_eq!(apply(5, 9, 0), (true, 5));
    assert_eq!(apply(-5, 10, 0), (true, -5));
    for operator in [8, 9, 10] {
        assert_eq!(fails(0, operator, 17).code(), Some(0x0d));
        assert_eq!(fails(0, operator, -1).code(), Some(0x0d));
    }
}

#[test]
fn increment_and_decrement_compare_after_storing() {
    assert_eq!(apply(1, 11, 3), (true, 2));
    assert_eq!(apply(2, 11, 3), (false, 3));
    assert_eq!(apply(32767, 11, 0), (true, -32768));
    assert_eq!(apply(3, 17, 1), (true, 2));
    assert_eq!(apply(2, 17, 1), (false, 1));
    let mut fx = Fixture::new();
    assert_eq!(code(fx.expression(LITERAL, 1, 11, LITERAL, 3)), Some(0x13));
}

#[test]
fn square_root() {
    assert_eq!(apply(0, 20, 0), (true, 0));
    assert_eq!(apply(0, 20, 15), (true, 3));
    assert_eq!(apply(0, 20, 16), (true, 4));
    assert_eq!(apply(0, 20, 32767), (true, 181));
    assert!(matches!(fails(0, 20, -1), Trap::Unsupported(_)));
}

#[test]
fn unknown_operators() {
    assert_eq!(fails(0, 21, 0).code(), Some(0x01));
    assert_eq!(fails(0, 255, 0).code(), Some(0x01));
}

#[test]
fn writes_need_a_word_pointer() {
    let mut fx = Fixture::new();
    for operator in [3, 4, 5, 6, 7, 9, 10, 13, 18, 19, 20] {
        assert_eq!(
            code(fx.expression(LITERAL, 1, operator, LITERAL, 1)),
            Some(0x13),
            "operator {operator}"
        );
    }
}

#[test]
fn the_right_operand_resolves_first() {
    // Both operands are out of bounds: the parameter's error (8) wins over
    // the temp's (2).
    let mut fx = Fixture::new();
    assert_eq!(
        code(fx.expression(TEMPS, 99, 5, scope::PARAMETERS, 99)),
        Some(8)
    );
    // The left operand is read even for a plain assignment: the lamp has
    // no motive to read.
    assert_eq!(code(fx.expression(14, 0, 5, LITERAL, 0)), Some(0x1c));
}

/// Runs `stack object's motive 0 <operator> rhs` with the motive at `old`.
fn motive(old: f32, operator: u8, rhs: i16) -> Result<f32, Trap> {
    let mut fx = Fixture::new();
    fx.sim_mut().person.as_mut().unwrap().motives[0] = old;
    fx.expression(15, 0, operator, LITERAL, rhs)
        .map_err(|f| f.trap)?;
    Ok(fx.sim().person.as_ref().unwrap().motives[0])
}

#[test]
fn motive_arithmetic_is_float_and_clamped() {
    assert_eq!(motive(10.5, 3, 2), Ok(12.5));
    assert_eq!(motive(10.5, 4, 2), Ok(8.5));
    assert_eq!(motive(10.5, 5, -20), Ok(-20.0));
    assert_eq!(motive(10.5, 6, 3), Ok(31.5));
    assert_eq!(motive(10.0, 7, 4), Ok(2.5));
    assert_eq!(motive(1.0, 7, 3), Ok(1.0 / 3.0));
    assert_eq!(motive(90.0, 3, 20), Ok(100.0));
    assert_eq!(motive(-90.0, 4, 20), Ok(-100.0));
    assert_eq!(motive(50.0, 6, -3), Ok(-100.0));
    assert_eq!(motive(1.0, 7, 0).unwrap_err().code(), Some(0x26));
}

#[test]
fn motives_compare_as_rounded_integers() {
    assert_eq!(motive(2.5, 2, 2), Ok(2.5));
    let mut fx = Fixture::new();
    fx.sim_mut().person.as_mut().unwrap().motives[0] = 2.5;
    assert_eq!(fx.expression(15, 0, 2, LITERAL, 2), Ok(true));
    assert_eq!(fx.expression(15, 0, 0, LITERAL, 2), Ok(false));
    assert_eq!(fx.expression(15, 0, 8, LITERAL, 2), Ok(true));
}

#[test]
fn integer_only_operators_reject_motives() {
    for operator in [9, 10, 12, 13, 18, 19, 20] {
        assert_eq!(motive(1.0, operator, 1).unwrap_err().code(), Some(0x13));
    }
    assert_eq!(motive(1.0, 9, 17).unwrap_err().code(), Some(0x0d));
    for operator in [11, 17] {
        assert!(matches!(
            motive(1.0, operator, 1),
            Err(Trap::Unsupported(_))
        ));
    }
}
