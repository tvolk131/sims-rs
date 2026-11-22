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
        // The original resolves the right operand (`0x452c76`) BEFORE binding
        // `[edi + 0x128]` at `0x452c85`. On a fresh object whose scratch
        // pointer is still null (neither safe nor left by `Set To Next`, whose
        // `[ebp - 0x18]` slot lands inside `Expression`'s 0x58-byte local
        // frame), reading global 34 dereferences null at `0x4541db`.
        let rhs_is_global_34 = right_scope == 6 && right_index == 34;
        let prev_scratch = self.world.scratch_ptrs.get(self.me);
        let prev_was_expression = prev_scratch & sims_sim::SCRATCH_PTR_EXPRESSION != 0;
        let prev_was_arg_slot = prev_scratch & sims_sim::SCRATCH_PTR_ARG_SLOT != 0;
        if rhs_is_global_34
            && prev_scratch & (sims_sim::SCRATCH_PTR_SET | sims_sim::SCRATCH_PTR_SET_TO_NEXT) == 0
        {
            return Err(Trap::Unsupported(
                "Expression reading global 34 before [object + 0x128] is initialized \
                 (0x452c76 -> 0x4541db) dereferences null"
                    .into(),
            ));
        }
        let rhs = i32::from(self.read(right_scope, right_index)?);
        // 0x452c85: binds `[edi + 0x128]` to `[ebp - 0x04]`. Because the
        // dispatcher pushes 1 argument for Expression (`0x45237d`) and 2 for
        // UI Effect (`0x45226e`), this targets UI Effect's saved `ebp` slot,
        // which the dispatcher restores at `0x452273` without faulting.
        self.world.scratch_ptrs.set(
            self.me,
            sims_sim::SCRATCH_PTR_SET | sims_sim::SCRATCH_PTR_EXPRESSION,
        );
        let write = matches!(operator, 3..=7 | 9..=13 | 17..=20);
        // After Random Number (`0x451e8d`) or Get Direction To (`0x45202b`),
        // `[edi + 0x128]` aliases this Expression's `[ebp + 0x08]` write-flag
        // slot. Reading Global 34 on the RHS (`0x452c76` → `0x4541db`) writes
        // `&globals[34]` (`[sim + 0x1c] + 0x54`, whose low byte is always
        // `...4`/`...c`, nonzero) into `[ebp + 0x08]` before `0x452c7b` passes
        // it as `write` to `0x453141` (`resolve`) and before `0x452d01` checks
        // it for the motive clamp.
        let effective_write = write || (rhs_is_global_34 && prev_was_arg_slot);
        let (mut destination, mut writable) =
            self.resolve(left_scope, left_index, effective_write)?;
        let original_destination = destination;
        // The left operand is read even when only assigned to, so an
        // unreadable destination faults for every operator.
        if matches!(operator, 5 | 20)
            && let Cell::Local(local) = destination
        {
            // An assignment and a square root do not use what they read,
            // so a local that still holds what an earlier frame left is
            // fine here.
            self.wrote_local(local);
        }
        let mut lhs = i32::from(self.value(destination)?);
        // After an earlier Expression at the same dispatch depth (`0x452c85`),
        // `[edi + 0x128]` points to `[ebp - 0x04]` (this Expression's integer
        // LHS pointer slot). Reading Global 34 on the RHS (`0x452c76` →
        // `0x4541db`) writes `&globals[34]` into `[ebp - 0x04]`. If resolving
        // the LHS does not write an integer pointer (motives write `[ebp - 0x08]`
        // at `0x453e16`; read-only destinations write neither — EXCEPT scope 24
        // `Neighbor`, which passes `[esi + 0x128]` at `0x453399` and clears it
        // to null at `0x425b8e`), `[ebp - 0x04]` remains `&globals[34]` and
        // takes priority for every mutating operator (`0x452dc0..0x4530c5`).
        if write
            && rhs_is_global_34
            && prev_was_expression
            && left_scope != 24
            && (!writable || matches!(destination, Cell::Motive(..)))
        {
            destination = Cell::Global(34);
            writable = true;
            lhs = i32::from(self.value(destination)?);
        }

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
            if effective_write && let Cell::Motive(id, index) = original_destination {
                self.clamp_motive(id, index)?;
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
            if effective_write && let Cell::Motive(id, index) = original_destination {
                self.clamp_motive(id, index)?;
            }
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
        // 0x452d01..0x452d4f: even when `[ebp - 0x04]` redirected the write to
        // Global 34, the tail still clamps `[ebp - 0x08]` (the resolved motive
        // float) to `[-100.0, 100.0]`.
        if effective_write && let Cell::Motive(id, index) = original_destination {
            self.clamp_motive(id, index)?;
        }
        Ok(true)
    }

    /// Clamps a person's motive float to `[-100.0, 100.0]` (`0x452d22`–`0x452d4f`).
    fn clamp_motive(&mut self, id: u16, index: usize) -> Result<(), Trap> {
        let stored = self.motive(id, index)?;
        let (low, high) = (F32::from_f32(-100.0), F32::from_f32(100.0));
        let clamped = if stored.ext() < low.ext() {
            low
        } else if stored.ext() > high.ext() || stored.ext().is_nan() {
            high
        } else {
            stored
        };
        if clamped != stored {
            self.set_motive(id, index, clamped)?;
        }
        Ok(())
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
