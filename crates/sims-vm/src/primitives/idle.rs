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
//!   with an empty action queue, the branch is true. When the frame's
//!   primitive state is 1 (the instruction started an action whose tree
//!   has returned), the branch is also true unless the front action was
//!   pushed to the front (its flag 2). Otherwise the front action is
//!   started (0x45dc14 to 0x45dd1f):
//!
//!   1. It is taken off the queue (0x45f77b). One with no target is
//!      dropped, and the thread yields.
//!   2. It is checked as the pie menu checks it
//!      ([`Machine::check_chosen`], 0x4582ad and 0x458319 without
//!      advertisements).
//!   3. If the check leaves flag 8 and the action has an action tree, the
//!      tree is pushed as the person's virtual function 0x34 does
//!      (0x45e59b: the person is woken, the tree of the target's type is
//!      pushed with the target as its stack object and the action's four
//!      arguments, and the person starts using the target). The action
//!      becomes the running one (0x45e6cd), the count parameter becomes
//!      0, the frame's primitive state 1, and bit 0x40 stays set; the
//!      pushed tree runs next. When it returns, the instruction runs
//!      again (0x474daa).
//!   4. Otherwise the target is asked to run its function 4, the action
//!      cancelled, with me as its stack object (0x422d46), and the thread
//!      yields; the instruction runs again next turn.
//!
//!   Either way the person's temps, if the check tree's run saved them,
//!   are put back last (0x4582dd). A tree that cannot be pushed in step 3
//!   (the original then takes step 4) fails loudly.
//!
//! Ported from the original's code (above) and checked against the old
//! prototype's `idle_for_input`, `idle_decision` and `should_interrupt`
//! (legacy/crates/sims-vm/src/action.rs), which agree on these paths.

use crate::autonomy::checked;
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

/// The frame's primitive state once the instruction started an action
/// (0x45dce4).
pub(crate) const STARTED: i32 = 1;

/// The object function that tells a target its action was cancelled.
const CANCELLED: usize = 4;

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
        if self.frame()?.primitive_state == STARTED {
            let pushed = self
                .me()?
                .person
                .as_ref()
                .and_then(|p| p.queue.first())
                .is_some_and(|front| front.flags & PUSHED_TO_FRONT != 0);
            if !pushed {
                return Ok(Flow::Branch(true));
            }
        }
        self.start_queued(index)
    }

    /// Starts the front queued action (0x45dc14 to 0x45dd1f; see the
    /// module's documentation). `index` is the count parameter's.
    fn start_queued(&mut self, index: usize) -> Result<Flow, Trap> {
        let action = self
            .me_mut()?
            .person
            .as_mut()
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .queue
            .remove(0);
        if action.target == 0 {
            return Ok(Flow::Yield);
        }
        let target = u16::try_from(action.target)
            .ok()
            .filter(|id| self.world.objects.contains_key(id))
            .ok_or_else(|| {
                Trap::Invalid(format!(
                    "a queued action's target, object {}, is not there",
                    action.target
                ))
            })?;
        let mut saved = None;
        let flow = self.check_and_start(action, target, index, &mut saved);
        // The temps go back however the start ended (0x4582dd, also on
        // the original's unwinding, 0x64e050).
        if let Some(temps) = saved {
            self.me_mut()?.temps = temps;
        }
        flow
    }

    /// Checks the action taken off the queue and starts or cancels it
    /// (0x45dc74 to 0x45dd00).
    fn check_and_start(
        &mut self,
        mut action: sims_formats::saved_objects::Interaction,
        target: u16,
        index: usize,
        saved: &mut Option<Vec<i16>>,
    ) -> Result<Flow, Trap> {
        let me = self.me;
        self.check_chosen(&mut action, saved, None)?;
        let flow = if action.flags & checked::PASSED != 0 && action.action != 0 {
            let frame = self.me()?.stack.len() - 1;
            let tree = action.action;
            self.wake(me)?;
            self.enter_on(target, tree, &action.args).map_err(|fault| {
                Trap::Unsupported(format!(
                    "Idle For Input could not push tree {tree:#06x} of object {target}, where \
                     the original cancels the action: {fault}"
                ))
            })?;
            self.count_use(target, false)?;
            let last_result = self.me()?.last_result;
            self.set_action(action, last_result)?;
            let object = self.me_mut()?;
            let idle = &mut object.stack[frame];
            idle.parameters[index] = 0;
            idle.primitive_state = STARTED;
            if let Some(top) = object.stack.last_mut() {
                top.stack_object = target as i16;
            }
            self.flags_mut()?[FLAGS] |= INTERRUPTED;
            Flow::Pushed
        } else {
            self.interpreter()
                .callback(self.world, target, CANCELLED, me as i16)
                .map_err(|fault| {
                    Trap::Unsupported(format!(
                        "cancelling a queued action on object {target} (0x45dcc8): {fault}"
                    ))
                })?;
            Flow::Yield
        };
        Ok(flow)
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
