//! Remove Object Instance (opcode 18): queue an object for removal.
//!
//! Ported from the old prototype (legacy/crates/sims-vm/src/removal.rs,
//! legacy/crates/sims-vm/src/lib.rs:1056-1074), recovered from the original's
//! 0x451b6a and its removal queue at 0x442d70.
//!
//! Operands: which object (word 0: 0 me, 1 the stack object, 2 object 0)
//! and flags (byte 2). A missing object takes the false branch.
//!
//! The object is not destroyed here. It is detached from the lot, added to
//! [`sims_sim::World::removals`], and stays registered until the removals
//! are carried out. An object that removes itself yields at this
//! instruction unless flag bit 0 is set, so its tree runs no further. In a
//! synchronous tree such as Init it cannot yield and takes the true
//! branch.
//!
//! Only the case the lot-wide controllers need is implemented: an object
//! that is off the lot, holds nothing, sits in nothing, is one tile, is
//! not a person, and has no callbacks for leaving the lot (9, 10 and 26).
//! Anything else needs the lot's placement and callback services and
//! fails loudly. Flag bit 1 asks that removing the object does not reset
//! the objects that depend on it; it is recorded on the object for
//! whatever carries the removal out.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, word};

/// Off the lot: where the factory puts a new object.
const OFF_LOT: i32 = -16;

impl Machine<'_, '_> {
    pub(crate) fn remove(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        let target = match word(operands, 0) {
            0 => i16::try_from(self.me).unwrap_or(0),
            1 => self.frame()?.stack_object,
            2 => 0,
            _ => return Err(Trap::native(9, "invalid Remove Object Instance mode")),
        };
        let flags = operands[2];
        let Some(id) = u16::try_from(target)
            .ok()
            .filter(|id| self.world.objects.contains_key(id))
        else {
            return Ok(Flow::Branch(false));
        };
        if !self.world.removals.contains(&id) {
            self.queue_removal(id, flags)?;
        }
        if !self.synchronous && id == self.me && flags & 1 == 0 {
            return Ok(Flow::Yield);
        }
        Ok(Flow::Branch(true))
    }

    fn queue_removal(&mut self, id: u16, flags: u8) -> Result<(), Trap> {
        let object = &self.world.objects[&id];
        let definition = self.definition(object.guid)?;
        let callbacks = self
            .program
            .libraries
            .get(&definition.resource)
            .and_then(|l| l.callbacks.get(&definition.chunk));
        let leaves_quietly = [9, 10, 26].iter().all(|&i| {
            callbacks
                .and_then(|c| c.get(i))
                .is_none_or(|f| f.action == 0)
        });
        let simple = object.data.len() == 72
            && object.person.is_none()
            && object.multipart.is_none()
            && object.data.get(2) == Some(&0)
            && object.slots.iter().skip(1).all(|slot| slot.object == 0)
            && object.position[..2] == [OFF_LOT, OFF_LOT]
            && leaves_quietly;
        if !simple {
            return Err(Trap::Unsupported(format!(
                "removing object {id} needs the lot's placement and callback services"
            )));
        }
        let object = self
            .world
            .objects
            .get_mut(&id)
            .ok_or(Trap::native(10, "missing object"))?;
        // Detached: no container, no slot, no room.
        for (word, value) in [(2, 0), (3, -1), (14, -1), (26, 0), (29, -5)] {
            object.data[word] = value;
        }
        object.data[8] &= !0x100;
        object.removal_skips_resets = flags & 2 != 0;
        self.world.removals.push(id);
        Ok(())
    }
}
