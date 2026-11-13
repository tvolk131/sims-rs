//! Opcode 10, which the community's references call Tutorial: take or give
//! up the ownership of interface events.
//!
//! Ported from the old prototype (legacy/crates/sims-vm/src/ui_owner.rs),
//! recovered from the original's 0x44d906 and 0x444bc8. The interface asks
//! the owning object what event it is waiting for.
//!
//! Operand byte 0 is the mode. Mode 0 takes the ownership and succeeds
//! only when no object has it, so taking it twice fails. Mode 1 gives it
//! up and succeeds only for the owner. Any other mode is native error
//! 0x35.
//!
//! A change of owner also closes the old owner's dialogs and signals the
//! interface (0x107). Both are presentation and are not modeled.

use crate::fault::Trap;
use crate::interpreter::Machine;

impl Machine<'_, '_> {
    pub(crate) fn own_events(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let owner = &mut self.world.event_owner;
        let (free, taken) = match operands[0] {
            0 => (0, self.me),
            1 => (self.me, 0),
            _ => return Err(Trap::native(0x35, "invalid event ownership mode")),
        };
        if *owner != free {
            return Ok(false);
        }
        *owner = taken;
        Ok(true)
    }
}
