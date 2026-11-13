//! Expression (opcode 2): compare or assign two operands.
//!
//! Ported from the old prototype (legacy/crates/sims-vm/src/memory.rs:803),
//! which checked its integer behavior against the original routine at
//! 0x452c45 (Sims 1.91).
//!
//! Operands: left index (word 0), right index (word 2), operator (byte 5),
//! left scope (byte 6), right scope (byte 7). Byte 4 is ignored.
//!
//! Operators: 0 `>`, 1 `<`, 2 `==`, 3 `+=`, 4 `-=`, 5 `=`, 6 `*=`, 7 `/=`,
//! 8 flag test, 9 set flag, 10 clear flag, 11 increment and compare `<`,
//! 12 `%=`, 13 `&=`, 14 `>=`, 15 `<=`, 16 `!=`, 17 decrement and compare
//! `>`, 18 `|=`, 19 `^=`, 20 square root.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use crate::memory::Cell;
use sims_core::{Ext, F32};

fn not_writable() -> Trap {
    Trap::native(0x13, "expression destination is not writable")
}

impl Machine<'_, '_> {
    pub(crate) fn expression(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let (left_index, right_index) = (word(operands, 0), word(operands, 2));
        let (operator, left_scope, right_scope) = (operands[5], operands[6], operands[7]);
        // The original resolves the right operand before the left.
        let rhs = i32::from(self.read(right_scope, right_index)?);
        let write = matches!(operator, 3..=7 | 9..=13 | 17..=20);
        let (destination, writable) = self.resolve(left_scope, left_index, write)?;
        // The left operand is read even when only assigned to, so an
        // unreadable destination faults for every operator.
        let lhs = i32::from(self.value(destination)?);

        if let Cell::Motive(id, index) = destination {
            if matches!(operator, 3..=7) {
                return self.motive_arithmetic(id, index, operator, rhs);
            }
            if matches!(operator, 11 | 17) {
                return Err(Trap::Unsupported(
                    "the original's motive increment/decrement dereferences a null integer cell"
                        .into(),
                ));
            }
            if matches!(operator, 9 | 10) && !(0..=16).contains(&rhs) {
                return Err(Trap::native(0x0d, "flag index out of bounds"));
            }
            if matches!(operator, 9 | 10 | 12 | 13 | 18 | 19 | 20) {
                return Err(Trap::native(
                    0x13,
                    "expression needs an integer destination",
                ));
            }
        }

        if matches!(operator, 11 | 17) {
            let value = (lhs + if operator == 11 { 1 } else { -1 }) as i16;
            if !writable || !self.store(destination, value)? {
                return Err(not_writable());
            }
            // The comparison uses the right operand as read before the store.
            return Ok(if operator == 11 {
                i32::from(value) < rhs
            } else {
                i32::from(value) > rhs
            });
        }

        let bit = || {
            if !(0..=16).contains(&rhs) {
                Err(Trap::native(0x0d, "flag index out of bounds"))
            } else {
                // Flags count from 1. x86 masks the shift count, so flag 0
                // tests the sign bit after sign extension, and setting or
                // clearing it leaves the 16-bit word unchanged.
                Ok(1i32.wrapping_shl((rhs - 1) as u32))
            }
        };
        let predicate = match operator {
            0 => Some(lhs > rhs),
            1 => Some(lhs < rhs),
            2 => Some(lhs == rhs),
            8 => Some(lhs & bit()? != 0),
            14 => Some(lhs >= rhs),
            15 => Some(lhs <= rhs),
            16 => Some(lhs != rhs),
            _ => None,
        };
        if let Some(result) = predicate {
            return Ok(result);
        }

        let value = match operator {
            3 => lhs + rhs,
            4 => lhs - rhs,
            5 => rhs,
            6 => lhs * rhs,
            7 => {
                if rhs == 0 {
                    return Err(Trap::native(0x26, "expression division by zero"));
                }
                lhs / rhs
            }
            9 => lhs | bit()?,
            10 => lhs & !bit()?,
            12 => {
                // The original requires a writable integer cell before it
                // checks for zero.
                if !writable {
                    return Err(not_writable());
                }
                if rhs == 0 {
                    return Err(Trap::native(0x26, "expression remainder by zero"));
                }
                // Both sides take their absolute value in a 16-bit cell
                // first, so -32768 stays negative.
                let left = (lhs as i16).wrapping_abs();
                let right = (rhs as i16).wrapping_abs();
                i32::from(left) % i32::from(right)
            }
            13 => lhs & rhs,
            18 => lhs | rhs,
            19 => lhs ^ rhs,
            20 if rhs >= 0 => (rhs as u32).isqrt() as i32,
            20 => {
                return Err(Trap::Unsupported(
                    "square root of a negative number needs the original's x87 exception policy"
                        .into(),
                ));
            }
            _ => return Err(Trap::native(0x01, "unknown expression operator")),
        } as i16;
        if !writable || !self.store(destination, value)? {
            return Err(not_writable());
        }
        Ok(true)
    }

    /// `+=`, `-=`, `=`, `*=` and `/=` on a motive, which is a float: the
    /// sum is formed in an FPU register, stored back to 32 bits and clamped
    /// to -100..=100. The old prototype computed in f64 (legacy memory.rs:
    /// 823-844), which double-rounds; this rounds once from the x87's
    /// 64-bit significand. Which precision the original's FPU control word
    /// sets here is unverified.
    fn motive_arithmetic(
        &mut self,
        id: u16,
        index: usize,
        operator: u8,
        rhs: i32,
    ) -> Result<bool, Trap> {
        let old = self.motive(id, index)?.ext();
        let right = Ext::from_i32(rhs);
        let value = match operator {
            3 => old + right,
            4 => old - right,
            5 => right,
            6 => old * right,
            7 if rhs != 0 => old / right,
            _ => return Err(Trap::native(0x26, "expression division by zero")),
        };
        let stored = value.to_f32();
        let (low, high) = (F32::from_f32(-100.0), F32::from_f32(100.0));
        let clamped = if stored.ext() < low.ext() {
            low
        } else if stored.ext() > high.ext() {
            high
        } else {
            stored
        };
        self.set_motive(id, index, clamped)?;
        Ok(true)
    }
}
