//! Set Motive Change (opcode 29, a person's primitive at 0x45a16f): add a
//! change to one of my motives that the person's tick applies, or clear
//! them all.
//!
//! Operands: the scopes of the amount and of the value to stop at (bytes
//! 0 and 1), the motive (byte 2), flags (byte 3), and the indexes of the
//! amount and of the stop (words 4 and 6).
//!
//! 1. With flag bit 0, every change I have is dropped (0x46085a over
//!    person +0xad0 to +0xad4), and nothing else is read.
//! 2. Otherwise the motive must be 5 to 9 or 14 or 15 (0x45a19d), else
//!    native error 0x1a; it is checked before either operand is read.
//! 3. The amount and the stop are read (0x453141, for reading), in that
//!    order.
//! 4. The change goes on the end of my list (0x45f837): the motive, the
//!    amount per hour as a change per tick (the amount times the float at
//!    0x667408, 1/1800, rounded to a float once), and the stop as a float.
//!    Two changes may name the same motive; the person's tick applies them
//!    in order ([`sims_sim::motives::apply_changes`]).
//!
//! The branch is always true.
//!
//! The handler is a method of the person; an object that is not a person
//! runs the general table's, which is not ported.
//!
//! Ported from the original's code (above), with the old prototype's port
//! (legacy/crates/sims-vm/src/motives.rs, `set_motive_changes`) agreeing.
//! The comparisons that cover it are whole ticks of the Goth house.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use sims_formats::saved_objects::MotiveDelta;

/// The float at 0x667408 that turns an amount per hour into one per tick.
const PER_TICK: f32 = f32::from_bits(0x3a11_a2b4);

/// Whether a motive may have a change (0x45a19d).
fn changeable(motive: u8) -> bool {
    matches!(motive, 5..=9 | 14..=15)
}

impl Machine<'_, '_> {
    pub(crate) fn set_motive_change(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        if self.me()?.person.is_none() {
            return Err(Trap::Unsupported(
                "Set Motive Change on an object that is not a person: the general primitive \
                 table's handler runs, not the person's (0x45a16f), and it is not ported"
                    .into(),
            ));
        }
        let [amount_scope, stop_scope, motive, flags, ..] = *operands;
        if flags & 1 != 0 {
            self.person_mut()?.motive_deltas.clear();
            return Ok(true);
        }
        if !changeable(motive) {
            return Err(Trap::native(0x1a, "invalid motive for a motive change"));
        }
        let amount = self.read(amount_scope, word(operands, 4))?;
        let stop = self.read(stop_scope, word(operands, 6))?;
        // The product of a 16-bit integer and a float is exact in a double,
        // so this rounds once, as the original's x87 product stored to a
        // float does.
        let delta = (f64::from(amount) * f64::from(PER_TICK)) as f32;
        self.person_mut()?.motive_deltas.push(MotiveDelta {
            motive: i32::from(motive),
            delta,
            stop_at: f32::from(stop),
        });
        Ok(true)
    }

    fn person_mut(&mut self) -> Result<&mut sims_formats::saved_objects::Person, Trap> {
        self.me_mut()?
            .person
            .as_mut()
            .ok_or_else(|| Trap::Invalid("a person with no person record".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_motives_that_take_a_change() {
        let allowed: Vec<u8> = (0..=255).filter(|&m| changeable(m)).collect();
        assert_eq!(allowed, [5, 6, 7, 8, 9, 14, 15]);
    }

    #[test]
    fn an_amount_per_hour_becomes_one_per_tick() {
        assert_eq!(PER_TICK.to_bits(), 0x3a11_a2b4);
        let delta = (f64::from(-200i16) * f64::from(PER_TICK)) as f32;
        assert_eq!(delta, -200.0 * PER_TICK);
    }
}
