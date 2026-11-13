//! Test Object Type (opcode 32): whether an object is of a GUID's type.
//!
//! Ported from the old prototype (legacy/crates/sims-vm/src/lib.rs:1167-1174,
//! legacy/crates/sims-vm/src/multitile.rs:106-140).
//!
//! Operands: the GUID (bytes 0..4), the object's index (word 4) and scope
//! (byte 6). Byte 7 is ignored.
//!
//! An object is of a type when its own GUID matches, or when it is a tile
//! of a multi-tile group whose catalog master has that GUID. The master is
//! the first definition in the tile's file, in resource order, with the
//! same group ID (word 10) and sub-index -1 (word 11).

use crate::fault::Trap;
use crate::interpreter::{Machine, word};

impl Machine<'_, '_> {
    pub(crate) fn test_object_type(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let guid = u32::from_le_bytes([operands[0], operands[1], operands[2], operands[3]]);
        let id = self.read(operands[6], word(operands, 4))?;
        let object = u16::try_from(id)
            .ok()
            .and_then(|id| self.world.objects.get(&id))
            .ok_or(Trap::native(0x17, "Test Object Type object is missing"))?;
        if object.guid == guid {
            return Ok(true);
        }
        let group = self.definition_words(object.guid)?[10];
        if group == 0 {
            return Ok(false);
        }
        let definition = self.definition(object.guid)?;
        let library = self
            .program
            .libraries
            .get(&definition.resource)
            .ok_or_else(|| Trap::Invalid("the object's file is not linked".into()))?;
        let master = library
            .definition_order
            .iter()
            .filter_map(|id| library.definitions.get(id))
            .find(|words| words[10] == group && words[11] == -1)
            .ok_or_else(|| {
                // The old prototype failed here too; what the original does
                // with a group that has no catalog master is unverified.
                Trap::Unsupported(format!(
                    "multi-tile group {group} of GUID 0x{:08X} has no catalog master",
                    object.guid
                ))
            })?;
        Ok(guid == (master[14] as u16 as u32 | (master[15] as u16 as u32) << 16))
    }
}
