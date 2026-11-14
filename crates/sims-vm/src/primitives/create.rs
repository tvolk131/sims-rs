//! Create New Object Instance (opcode 42): make an object through the
//! factory.
//!
//! Ported from the old prototype's creation request
//! (legacy/crates/sims-vm/src/create.rs and
//! legacy/crates/sims-sim/src/lot_creation.rs), recovered from the
//! original's 0x44e0c8.
//!
//! Operands: the GUID (bytes 0..4), where the object goes (byte 4) and
//! flags (byte 5). Only one case is ported: the new object stays off the
//! lot (mode 6) and no flag is set. The object is made by
//! [`crate::Interpreter::create`], so its Init and first turn have run
//! when the primitive returns. It becomes the stack object and the true
//! branch is taken. Every other mode places the object, which needs the
//! original's search for a free place; those fail loudly.
//!
//! **Unverified**: a GUID with no definition takes the false branch here.

use crate::fault::Trap;
use crate::interpreter::Machine;

impl Machine<'_, '_> {
    pub(crate) fn create_object(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let guid = u32::from_le_bytes([operands[0], operands[1], operands[2], operands[3]]);
        if operands[4] != 6 || operands[5] != 0 {
            return Err(Trap::Unsupported(format!(
                "Create New Object Instance mode {} with flags {:#x}",
                operands[4], operands[5]
            )));
        }
        if self.catalog.object(guid).is_none() {
            return Ok(false);
        }
        let created = self
            .interpreter()
            .create(self.world, guid)
            .map_err(|fault| {
                Trap::Unsupported(format!("creating an object of type 0x{guid:08X}: {fault}"))
            })?;
        self.frame_mut()?.stack_object = created as i16;
        Ok(true)
    }
}
