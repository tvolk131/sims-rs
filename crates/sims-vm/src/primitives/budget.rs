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
//! and unless bit 0 is set the amount is spent ([`spend`], 0x46d91c).
//!
//! Ported from the original's code, with the old prototype's
//! `change_budget` (legacy/crates/sims-vm/src/budget.rs) as a guide.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use sims_sim::World;

/// Global 32: the budget mode. At 1 money is not counted.
const BUDGET_MODE: usize = 32;

/// Globals 14 and 13: the funds above and below 10,000.
const FUNDS_HIGH: usize = 14;
const FUNDS_LOW: usize = 13;

/// The money available when the budget mode is 1.
const UNLIMITED: i32 = 999_999;

/// The categories a day's entry has.
const CATEGORIES: usize = 8;

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
        if flags & 1 == 0 {
            spend(self.world, category, amount)?;
        }
        Ok(true)
    }
}

/// Spends `amount` of the household's money in budget category
/// `category` (0x46d91c). In budget mode 1 it does nothing. Otherwise the
/// amount is taken from the funds and added to the category's entry for
/// today, each in 32 bits that wrap, and globals 14 and 13 take the new
/// funds split at 10,000, as the funds setter (0x46d8dc) splits them:
/// each part truncated toward zero, then cut to 16 bits. Unlike the
/// setter, which writes only in budget mode 0, this writes in every mode
/// but 1.
///
/// Today's entry is the simulator's eight words at +0x184; the load reads
/// the save's first day there (0x46d1e4), so here it is
/// [`sims_sim::Budget::days`]' first. The original then sends signal
/// 0xda with the funds (0x40760d). Of the game's eight subscribers only
/// the interface takes it (0x4f65bf): it marks the control panel's funds
/// for redrawing and redraws them (0x4ce81c, 0x4cecf5). That is
/// presentation, so it is not modeled.
///
/// Fails loudly, changing nothing, on what is not ported: category 8,
/// which the primitive allows but which is past today's eight words (the
/// original adds the amount to the simulator's word at +0x1a4, the head
/// of a list, 0x46d933); a lot whose days were not read from a save; and
/// a today the save marked with flag 0, for which the original reads only
/// seven words (0x46d270) and the parser none.
pub(crate) fn spend(world: &mut World, category: u8, amount: i32) -> Result<(), Trap> {
    if world.globals.get(BUDGET_MODE).copied().unwrap_or(0) == 1 {
        return Ok(());
    }
    let category = usize::from(category);
    if category >= CATEGORIES {
        return Err(Trap::Unsupported(format!(
            "spending {amount} in budget category {category}, past today's eight entries: the \
             original adds it to the simulator's word at +0x1a4, the head of a list (0x46d933)"
        )));
    }
    if world.globals.len() <= FUNDS_HIGH {
        return Err(Trap::Unsupported(format!(
            "spending {amount} with only {} globals, before the lot's context is applied \
             (0x46d91c)",
            world.globals.len()
        )));
    }
    let today = match world.budget.days.first_mut() {
        Some(Some(today)) => today,
        Some(None) => {
            return Err(Trap::Unsupported(
                "spending into today's budget entry, which the save marked with flag 0: the \
                 original reads seven words for it (0x46d270)"
                    .into(),
            ));
        }
        None => {
            return Err(Trap::Unsupported(
                "spending into today's budget entry, which no save gave this lot (simulator \
                 +0x184, read at 0x46d1e4)"
                    .into(),
            ));
        }
    };
    today[category] = today[category].wrapping_add(amount);
    let funds = world.budget.funds.wrapping_sub(amount);
    world.budget.funds = funds;
    world.globals[FUNDS_HIGH] = (funds / 10000) as i16;
    world.globals[FUNDS_LOW] = (funds % 10000) as i16;
    Ok(())
}
