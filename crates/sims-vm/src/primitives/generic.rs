//! Generic Sims Call (opcode 1): one of several dozen built-in routines,
//! picked by operand word 0.
//!
//! Ported from the old prototype's `generic_person_call`
//! (legacy/crates/sims-vm/src/person.rs:627). Only the routines scripts have
//! reached so far are here; the rest fail loudly with their number.
//!
//! - 28: classify a lot. Temp 0 holds a lot number and receives 0 for a
//!   residential lot, 1 for a community lot and -1 for a lot the zoning
//!   table does not know. Lot 0 and the Magic Town lots 90 to 92 are
//!   residential and 93 to 98 community whatever the table says.
//! - 43: raise the magic controller's attributes to the selected family's
//!   magic history (0x410af6). While the game holds no family's history it
//!   does nothing, which is the only case ported. With a history it needs
//!   the original's per-GUID instance list to find the controller, and
//!   fails loudly.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};

impl Machine<'_, '_> {
    pub(crate) fn generic_call(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        match word(operands, 0) {
            28 => {
                let zoning = &self.program.zoning;
                let temps = &mut self.me_mut()?.temps;
                let lot = temps
                    .first_mut()
                    .ok_or_else(|| Trap::Invalid("object has no temps".into()))?;
                *lot = match *lot {
                    0 | 90..=92 => 0,
                    93..=98 => 1,
                    number => match zoning.get(&number) {
                        Some(1) => 0,
                        Some(2) => 1,
                        // A lot missing from the table reads as zone 0.
                        _ => -1,
                    },
                };
                Ok(true)
            }
            43 => match self.world.magic_history {
                None => Ok(true),
                Some(_) => Err(Trap::Unsupported(
                    "Generic Sims Call routine 43 with a family's magic history".into(),
                )),
            },
            routine => Err(Trap::Unsupported(format!(
                "Generic Sims Call routine {routine}"
            ))),
        }
    }
}
