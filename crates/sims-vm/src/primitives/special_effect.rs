//! Special Effect (opcode 35, 0x44aaa6, from the general table at
//! 0x45246a; a person's dispatcher, 0x45cbb3, sends it there too): show an
//! effect over an object, such as the floating text of money earned.
//!
//! Operands: word 0 the index of the effect's duration, byte 4 flags,
//! byte 5 an entry of the tree owner's STR# 0x131 to show (0 for none),
//! bytes 2 and 3 the effect's kind and size.
//!
//! The primitive is a small machine on the frame's primitive state:
//!
//! - State 0: global 11 becomes 1 (0x46d9c4 on the lot's globals), the
//!   state becomes 1 and the primitive yields.
//! - State 1: the state becomes 2 first. The stack object must exist
//!   (0x40743b, native error 0x17). The duration is read (0x453141, from
//!   scope 25 with flag 0x20 and from scope 7 without). Then the effect
//!   is shown only:
//!   - if the game's options show effects (options +4, 0x4cfe18) or flag 8
//!     is set; and then
//!   - if the lot is residential (0x40745f: its zoning is 1, and lot 0
//!     always is) or flag 8 is set, or else if I am a person whose
//!     family (person word 61) is the lot's (global 9), or, when the lot
//!     has no family, whose word 77 is not 0.
//!
//!   A shown effect yields; one not shown takes the true branch at once.
//! - Any other state: the true branch.
//!
//! The options' flag is a check box of the Options panel (0x5164ae,
//! setter 0x4e62fa). The options' constructor (0x4e5f63) writes 0, but in
//! the frozen image of the running game the options (0x6f2628) hold 1 at
//! +4, so the saved options turn it on. The Goth day recording agrees:
//! at tick 2747 objects 12 and 334 run Special Effect (flags 0x43) on the
//! residential Goth lot, and state 1 yields once more before their next
//! sleep, which only a shown effect does. So it is taken as on here.
//! Turning it off in the Options panel changes how long a script waits
//! here; the panel is not ported. See
//! docs/decisions/20261115-snap-offset-order.md.
//!
//! What showing does is presentation and is not modeled: the message
//! 0x10c to the game's listeners unless flag 0x40 is set (0x40760d), the
//! text from STR# 0x131 formatted by 0x44bf1a, and the effect itself
//! (0x4a6950). That the listeners write nothing the simulation reads is
//! **unverified**.
//!
//! Ported from the original's code (above). The old prototype's port
//! (legacy/crates/sims-vm/src/effect.rs) agrees on the states and gates.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, word};

/// Global 9: the family of the lot.
const FAMILY: usize = 9;
/// Global 10: the lot's number.
const LOT: usize = 10;
/// Global 11, which state 0 sets (0x46d9c4 with 0xb and 1). The idle
/// loop clears it before the next update and ends its frame there
/// (0x46da98, `crate::boot`).
const EFFECT: usize = 11;
/// Person data words.
mod person {
    pub const FAMILY: usize = 61;
    pub const DATE: usize = 77;
}
/// Whether the game's options show effects (options +4, 0x4cfe18): on, as
/// the running game holds it (0x6f262c is 1 in the frozen image).
const OPTION_SHOWS_EFFECTS: bool = true;

impl Machine<'_, '_> {
    pub(crate) fn special_effect(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        match self.frame()?.primitive_state {
            0 => {
                *self
                    .world
                    .globals
                    .get_mut(EFFECT)
                    .ok_or_else(|| Trap::Invalid("the lot has no globals".into()))? = 1;
                self.frame_mut()?.primitive_state = 1;
                Ok(Flow::Yield)
            }
            1 => {
                self.frame_mut()?.primitive_state = 2;
                let target = self.frame()?.stack_object;
                if !u16::try_from(target).is_ok_and(|id| self.world.objects.contains_key(&id)) {
                    return Err(Trap::native(0x17, "Special Effect has no stack object"));
                }
                let flags = operands[4];
                let scope = if flags & 0x20 != 0 { 25 } else { 7 };
                self.read(scope, word(operands, 0))?;
                Ok(if self.shows_effect(flags)? {
                    Flow::Yield
                } else {
                    Flow::Branch(true)
                })
            }
            _ => Ok(Flow::Branch(true)),
        }
    }

    /// Whether Special Effect shows its effect (0x44ab42 to 0x44abcc).
    fn shows_effect(&self, flags: u8) -> Result<bool, Trap> {
        let forced = flags & 8 != 0;
        if !OPTION_SHOWS_EFFECTS && !forced {
            return Ok(false);
        }
        let global = |index: usize| {
            self.world
                .globals
                .get(index)
                .copied()
                .ok_or_else(|| Trap::Invalid("the lot has no globals".into()))
        };
        let lot = global(LOT)?;
        if lot == 0 || self.program.zoning.get(&lot) == Some(&1) || forced {
            return Ok(true);
        }
        let Some(me) = self.me()?.person.as_ref() else {
            return Ok(false);
        };
        let word = |index: usize| {
            me.data
                .get(index)
                .copied()
                .ok_or_else(|| Trap::Invalid(format!("no person data word {index}")))
        };
        let family = global(FAMILY)?;
        Ok(word(person::FAMILY)? == family || (family == 0 && word(person::DATE)? != 0))
    }
}
