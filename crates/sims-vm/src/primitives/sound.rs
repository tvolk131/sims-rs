//! Play Sound Event (opcode 23).
//!
//! The old prototype (legacy/crates/sims-vm/src/lib.rs:1175-1217) looks up
//! the event's sound in the code owner's file, then Global.iff, queues it
//! for playback on me or the stack object (flag byte 4, bit 1) and always
//! takes the true branch. Playback is presentation only, so it is dropped;
//! the primitive changes no simulation state. That the original never
//! takes the false branch or raises an error here is unverified.

use crate::fault::Trap;
use crate::interpreter::Machine;

impl Machine<'_, '_> {
    pub(crate) fn play_sound(&mut self, _operands: &[u8; 8]) -> Result<bool, Trap> {
        Ok(true)
    }
}
