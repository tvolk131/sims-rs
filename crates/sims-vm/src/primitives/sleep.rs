//! Sleep (opcode 0): wait a number of turns.
//!
//! Ported from the old prototype's `sleep`
//! (legacy/crates/sims-vm/src/idle.rs:172), recovered from the original's
//! 0x44aa57.
//!
//! Operand word 0 picks the parameter of the running tree that holds the
//! count. A count of zero or less is done at once and takes the true
//! branch. Otherwise the thread yields and this instruction runs again on
//! the object's next turn.
//!
//! There are two ways the count goes down. An object that is not a person
//! and is not locked out (data word 25 is zero) hands the count to the
//! scheduler: its countdown becomes the count less one, the scheduler
//! lowers it each tick without running the object, and the object's next
//! turn finds the countdown at zero and finishes the Sleep. Anything else
//! keeps running every tick and lowers its own parameter.
//!
//! The original also keeps an object with a headline showing out of the
//! scheduler's hands. Headlines are not modeled, so that test is missing.
//! **Unverified** for any object that shows one.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, word};
use crate::memory::LOCKOUT;

impl Machine<'_, '_> {
    pub(crate) fn sleep(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        let frame = self.frame()?;
        let index = usize::try_from(word(operands, 0))
            .ok()
            .filter(|&i| i < frame.parameters.len())
            .ok_or(Trap::native(8, "invalid Sleep parameter"))?;
        let mut count = frame.parameters[index];
        let me = self.me()?;
        let scheduled = me.countdown();
        // A countdown of exactly zero means the scheduler finished it, or
        // something notified the object: either way the Sleep is over.
        let finished = scheduled == Some(0);
        if finished {
            count = 0;
        }
        count = count.max(0);
        let passive = me.person.is_none() && me.data.get(LOCKOUT) == Some(&0);
        if count == 0 {
            self.frame_mut()?.parameters[index] = 0;
            if finished {
                self.me_mut()?.set_countdown(-1);
            }
            return Ok(Flow::Branch(true));
        }
        if self.synchronous {
            return Err(Trap::native(0x44f, "a synchronous tree yielded in Sleep"));
        }
        if scheduled.is_some() && passive {
            self.frame_mut()?.parameters[index] = count;
            self.me_mut()?.set_countdown(count - 1);
        } else {
            self.frame_mut()?.parameters[index] = count - 1;
        }
        Ok(Flow::Yield)
    }
}
