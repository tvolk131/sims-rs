//! Play Sound Event (opcode 23) and Stop All Sounds (opcode 48).
//!
//! The old prototype (legacy/crates/sims-vm/src/lib.rs:1175-1217) looks up
//! the event's sound in the code owner's file, then Global.iff, queues it
//! for playback on me or the stack object (flag byte 4, bit 1) and always
//! takes the true branch. Playback is presentation only, so it is dropped;
//! the primitive changes no simulation state. That the original never
//! takes the false branch or raises an error here is unverified.
//!
//! Stop All Sounds (0x45253a, from the general table at 0x45246a) takes
//! me (operand word 0 is 0, object field 0xf0) or the stack object, hands
//! its ID to the sound manager's stop (0x57a6fb, which queues a mixer
//! command, 0x573617), notes it in the debug log when that is on (byte
//! 0x6e509c), and returns 1: the true branch. It reads no other state and
//! raises no error, even for an ID that names no object. The old
//! prototype (legacy/crates/sims-vm/src/lib.rs:1218-1240) queued the
//! same stop for its presenter. Sound is presentation, so here it does
//! nothing but take the true branch.

use crate::fault::Trap;
use crate::interpreter::Machine;

impl Machine<'_, '_> {
    pub(crate) fn play_sound(&mut self, _operands: &[u8; 8]) -> Result<bool, Trap> {
        Ok(true)
    }

    pub(crate) fn stop_all_sounds(&mut self, _operands: &[u8; 8]) -> Result<bool, Trap> {
        Ok(true)
    }
}
