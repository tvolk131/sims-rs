//! Alter Budget (opcode 25, 0x44eade): test, and maybe spend, an amount
//! of the household's money.
//!
//! Operands: byte 0 the amount's scope (0 a literal, 1 an argument, 2 a
//! local: scopes 7, 9 and 25; anything else the scope in byte 1), word 2
//! its index,
//! byte 4 the flags (bit 0: only test; bit 1: the amount is income, so it
//! is negated), byte 6 the budget category, 0 to 8 (native error 0x38
//! otherwise, raised before the amount is read).
//!
//! The money available is 999,999 when the budget mode (global 32) is 1,
//! and the funds otherwise (0x46d8c9). An amount that is not negative and
//! more than that takes the false branch. Otherwise the branch is true,
//! and unless bit 0 is set the amount is spent (0x46d91c): with the
//! budget mode not 1 it is taken from the funds, added to the category's
//! entry for today, globals 14 and 13 take the funds split at 10,000, and
//! signal 0xda goes to the interface.
//!
//! Only the test is ported. Spending fails loudly: the day's entries are
//! modeled ([`sims_sim::world::Budget::days`]) but which one is today's
//! has not been checked against the original. Ported from the original's
//! code, with the old prototype's `change_budget`
//! (legacy/crates/sims-vm/src/budget.rs) as a guide.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};

/// Global 32: the budget mode. At 1 money is not counted.
const BUDGET_MODE: usize = 32;

/// The money available when the budget mode is 1.
const UNLIMITED: i32 = 999_999;

impl Machine<'_, '_> {
    pub(crate) fn alter_budget(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let scope = match operands[0] {
            0 => 7,
            1 => 9,
            2 => 25,
            _ => operands[1],
        };
        let category = operands[6];
        if category > 8 {
            return Err(Trap::native(0x38, "invalid budget category"));
        }
        let mut amount = i32::from(self.read(scope, word(operands, 2))?);
        let flags = operands[4];
        if flags & 2 != 0 {
            amount = -amount;
        }
        let mode = self.world.globals.get(BUDGET_MODE).copied().unwrap_or(0);
        let available = if mode == 1 {
            UNLIMITED
        } else {
            self.world.budget.funds
        };
        if amount >= 0 && available < amount {
            return Ok(false);
        }
        if flags & 1 == 0 && mode != 1 {
            return Err(Trap::Unsupported(format!(
                "Alter Budget spending {amount} in category {category}, which writes the \
                 day's budget entry (0x46d91c)"
            )));
        }
        Ok(true)
    }
}
