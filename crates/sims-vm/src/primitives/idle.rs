//! Idle For Input (opcode 17, 0x45db94): a person waits for something to
//! do.
//!
//! The person's own dispatcher (0x45cbb3) runs this opcode; the general
//! table does not. Anything that is not a person fails loudly here.
//!
//! Operand word 0 picks the parameter of the running tree that holds a
//! count; one outside the frame's parameters is native error 8. Operand
//! word 1 asks for the queued action to be started when there is one.
//!
//! First the person decides whether something should interrupt the wait
//! (0x45d7c0, [`Machine::wants_interrupt`]). Then:
//!
//! - Nothing interrupts and the count is 0: bit 0x40 of object data word
//!   8 is cleared and the branch is true.
//! - Nothing interrupts and the count is not 0: the count goes down by
//!   one and the thread yields, to run this instruction again next turn.
//! - Something interrupts: bit 0x40 is set. Without operand word 1, or
//!   with an empty action queue, the branch is true. Otherwise the
//!   original takes the front action off the queue and starts it
//!   (0x45dc14 onwards: 0x45f77b, 0x4582ad, 0x458319, 0x45e6cd), which is
//!   **not ported** and fails loudly. (When the frame resumes a dispatch,
//!   only an action pushed to the front is started; resuming a primitive
//!   already fails in the interpreter.)
//!
//! Ported from the original's code (above) and checked against the old
//! prototype's `idle_for_input`, `idle_decision` and `should_interrupt`
//! (legacy/crates/sims-vm/src/action.rs), which agree on these paths.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, word};
use sims_core::{Ext, F32};
use std::cmp::Ordering;

/// Object data word 8, whose bit 0x40 (byte +0x5a) says the person was
/// interrupted while idle.
const FLAGS: usize = 8;
const INTERRUPTED: i16 = 0x40;

/// Person data words the interrupt test reads (person data at +0x58c).
const WORD_71: usize = 71; // +0x61a
const WORD_32: usize = 32; // +0x5cc
const WORD_33: usize = 33; // +0x5ce
const WORD_65: usize = 65; // +0x60e

/// Motives the interrupt test reads (motives at +0x78c).
const MOTIVE_5: usize = 5; // +0x7a0
const MOTIVE_7: usize = 7; // +0x7a8
const MOTIVE_9: usize = 9; // +0x7b0

/// Action flag 2 (+0x40 of a queue entry): pushed to the front.
const PUSHED_TO_FRONT: u32 = 2;

/// The global that holds the lot's house number.
const LOT: usize = 10;

impl Machine<'_, '_> {
    pub(crate) fn idle_for_input(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        if self.me()?.person.is_none() {
            return Err(Trap::Unsupported(
                "Idle For Input on an object that is not a person: the general primitive table \
                 handles it, which is not ported"
                    .into(),
            ));
        }
        let interrupt = self.wants_interrupt()?;
        let frame = self.frame()?;
        let index = usize::try_from(word(operands, 0))
            .ok()
            .filter(|&i| i < frame.parameters.len())
            .ok_or(Trap::native(8, "invalid Idle For Input parameter"))?;
        let count = frame.parameters[index];
        if !interrupt {
            if count != 0 {
                self.frame_mut()?.parameters[index] = count.wrapping_sub(1);
                return Ok(Flow::Yield);
            }
            self.flags_mut()?[FLAGS] &= !INTERRUPTED;
            return Ok(Flow::Branch(true));
        }
        self.flags_mut()?[FLAGS] |= INTERRUPTED;
        let queued = self
            .me()?
            .person
            .as_ref()
            .is_some_and(|p| !p.queue.is_empty());
        if word(operands, 2) == 0 || !queued {
            return Ok(Flow::Branch(true));
        }
        Err(Trap::Unsupported(
            "Idle For Input starting the queued action (0x45dc14) is not ported".into(),
        ))
    }

    fn flags_mut(&mut self) -> Result<&mut Vec<i16>, Trap> {
        let data = &mut self.me_mut()?.data;
        if data.len() <= FLAGS {
            return Err(Trap::Invalid("object data has no word 8".into()));
        }
        Ok(data)
    }

    /// Whether something should interrupt the person's wait (0x45d7c0).
    ///
    /// 1. A person whose data word 71 is not 0 is not interrupted.
    /// 2. The front queued action interrupts when its priority is above
    ///    data word 33, or when it was pushed to the front (flag 2).
    /// 3. On a residential lot, with nothing queued that interrupts, a
    ///    person whose data word 32 is below 2 is interrupted when motive
    ///    7 or 9 is below -99, or when motive 5 is below -99 and data word
    ///    65 is at most 1. "Below" includes not a number, as the
    ///    original's x87 compare sets the carry flag for it.
    ///
    /// The queue is the saved one: nothing in this port adds to it yet.
    pub(crate) fn wants_interrupt(&self) -> Result<bool, Trap> {
        let person = self
            .me()?
            .person
            .as_ref()
            .ok_or_else(|| Trap::Invalid("the interrupt test needs a person".into()))?;
        let data = |i: usize| {
            person
                .data
                .get(i)
                .copied()
                .ok_or_else(|| Trap::Invalid(format!("person data has no word {i}")))
        };
        if data(WORD_71)? != 0 {
            return Ok(false);
        }
        if let Some(front) = person.queue.first()
            && (front.priority > i32::from(data(WORD_33)?) || front.flags & PUSHED_TO_FRONT != 0)
        {
            return Ok(true);
        }
        let lot = self.world.globals.get(LOT).copied().unwrap_or(0);
        if self.program.zoning.get(&lot) != Some(&1) || data(WORD_32)? >= 2 {
            return Ok(false);
        }
        let low = |i: usize| below_minus_99(person.motives[i]);
        Ok(low(MOTIVE_7) || low(MOTIVE_9) || (low(MOTIVE_5) && data(WORD_65)? <= 1))
    }
}

/// Below -99.0 (the constant at 0x667428) or not a number.
fn below_minus_99(value: f32) -> bool {
    let threshold = Ext::from_i32(-99);
    !matches!(
        F32::from_f32(value).ext().partial_cmp(&threshold),
        Some(Ordering::Greater | Ordering::Equal)
    )
}
