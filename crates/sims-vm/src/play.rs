//! Play mode: a lot update that carries on past an object's error.
//!
//! In strict mode, the default and the mode every comparison with the
//! original uses, any fault stops the tick and the lot halts
//! ([`crate::boot::Lot::step`]). In play mode
//! ([`crate::boot::Lot::step_play`]) the scheduler catches a fault in an
//! object's tick and deals with it the way the original deals with a
//! SimAntics error, and the lot goes on.
//!
//! **What the original does** (read from its code):
//!
//! 1. The thread's run (0x474e88) catches the error (0x474ec1): object
//!    word +0x28 takes the error's code, and the object's virtual
//!    function 4 (0x439a4e for both an object and a person) sets the
//!    object's pending bit (+0x115 bit 0) and scheduler flag 4 (0x4457ef).
//!    The run then returns true, so the rest of the object's tick runs
//!    (the tail of 0x4521f7, and for a person the rest of 0x45cc8c).
//! 2. After the tick the scheduler's pass (0x46d553) reads the error word
//!    (0x4745b2). As it is not 0, the pass calls 0x439a6f on the object
//!    and returns false: the pass is partial, the clock does not advance,
//!    and the cursor stays on the object.
//! 3. 0x439a6f, with the pending bit set and the error word not 0, clears
//!    the bit. With the debug byte 0x6c2983 clear (it is, in a memory
//!    image of the running game) it then tells every other person that
//!    this object resets (0x444306: their virtual function 0x38 with the
//!    object, 0x45e791, by which a person lets go of it;
//!    [`crate::forget`]; objects that are not people are not told), and
//!    resets the object (its own
//!    virtual function 0x38 with 0: [`Interpreter::reset`], or
//!    [`Interpreter::reset_person`] for a person). The reset restarts the
//!    thread at Main (0x47480c), which clears the error word. With the
//!    debug byte set it would build an error report instead.
//! 4. At the next update the pass starts at the object. Its flag 4 is
//!    set, but its error word is 0, so the flag is cleared and the object
//!    is ticked as usual.
//!
//! Here the error word is never kept: a reset that succeeds clears it in
//! the same pass, so step 4 always finds it 0, as strict mode assumes.
//!
//! **Approximations**, play mode only (`docs/decisions/`):
//!
//! - An unported primitive or service ([`crate::Trap::Unsupported`]) is
//!   handled like a SimAntics error, as is a thread that returns from its
//!   outermost frame (0x3e9) or runs past the instruction budget (0x44d).
//! - A fault in the rest of the tick (a person's animator, motives or
//!   room) is not a SimAntics error in the original but a part this port
//!   lacks. It is handled the same way, except that the rest of that tick
//!   does not run. When it follows a fault caught in the thread, both are
//!   noted ([`ScriptError::then`]) and the object is dealt with once.
//! - Where the reset itself fails (much of it is not ported, such as a
//!   person that is using an object or is partway along a route), the
//!   object's thread restarts at Main and a person's animator resets
//!   where it can, and nothing more is reset ([`Recovery::Restarted`]). If even that fails the object is parked
//!   ([`Recovery::Parked`]): its scheduler flag 1 is cleared, so it is
//!   never ticked again.
//! - An object that faults again on its first tick after a reset would
//!   stop every pass at itself, and the clock with it. It is parked
//!   instead ([`Recovery::Parked`]). An object deleted after its reset
//!   is no longer fresh, so a later object given its ID is not parked
//!   for its first fault ([`crate::Interpreter::deleted`]). Deletions
//!   made outside a play-mode update are not tracked.
//! - A frame of the house view that fails is skipped, and noted in
//!   [`crate::Update::frame_errors`] with the tick it was drawn before or
//!   after.
//!
//! Only an object's tick and the frames are caught. A fault anywhere else
//! in the update (the registry's dialogs and removals, the clock) still
//! halts the lot, as in strict mode. The failure then carries what the
//! step did before it: the errors caught, a skipped frame before the
//! first tick, and the sounds ([`crate::boot::StepError::Tick`]).

use crate::fault::Fault;
use std::collections::BTreeSet;

/// Play mode's own state, kept outside the [`sims_sim::World`] so that it
/// does not change the world's hash.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Play {
    /// The objects play mode reset that have not finished a tick since.
    pub(crate) fresh: BTreeSet<u16>,
}

/// An object's error that play mode caught, and what it did about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    /// The lot's tick count when it happened.
    pub tick: u32,
    pub object: u16,
    pub fault: Fault,
    /// Whether it came from the object's thread, which the original
    /// catches. Otherwise it came from the rest of the object's tick.
    pub in_thread: bool,
    /// A fault in the rest of the tick after the thread's caught one,
    /// which stopped that rest. The object is dealt with once, for both.
    pub then: Option<Fault>,
    pub recovery: Recovery,
}

/// What play mode did with an object after its error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    /// What the original does: the other objects were told and the object
    /// was reset.
    Reset,
    /// The reset failed with this fault; the object's thread restarted at
    /// Main instead.
    Restarted(Fault),
    /// The object was parked and is never ticked again: it faulted on its
    /// first tick after a reset, or (with this fault) even restarting its
    /// thread failed.
    Parked(Option<Fault>),
}
