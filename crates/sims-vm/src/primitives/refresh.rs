//! Refresh (opcode 7): redraw an object or mark its room for rebuilding.
//!
//! Ported from the old prototype (legacy/crates/sims-vm/src/lib.rs:1241-1253,
//! legacy/crates/sims-vm/src/refresh.rs:88-165).
//!
//! Operands: the object (word 0: 0 me, 1 the stack object) and the mode
//! (word 2). Mode 0 invalidates the drawing of the object's whole
//! containment tree, which only the renderer sees, so it does nothing here.
//! Modes 1 and 2 mark the room in the object's data word 29 as not
//! counted, unless that word is -5 (no room) or names no room. Such a room
//! is counted again, and has its score and lighting rebuilt, at the next
//! room update (0x467094). Mode 1 also asks for the room to be redrawn,
//! which is not modeled.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};

/// Data word 29: the room the object stands in.
const ROOM: usize = 29;
/// The room word of an object in no room.
const NO_ROOM: i16 = -5;

impl Machine<'_, '_> {
    pub(crate) fn refresh(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let (selector, mode) = (word(operands, 0), word(operands, 2));
        let id = match selector {
            0 => self.me,
            1 => u16::try_from(self.frame()?.stack_object)
                .ok()
                .filter(|id| self.world.objects.contains_key(id))
                .ok_or(Trap::native(0x15, "Refresh stack object is missing"))?,
            _ => return Err(Trap::native(3, "invalid Refresh object selector")),
        };
        match mode {
            // Presentation only: the drawing invalidation is dropped.
            0 => Ok(true),
            1 | 2 => {
                // Mode 1 goes through the object module's room broadcast
                // (event 0xf2), which ignores IDs below 1; mode 2 marks the
                // room directly.
                if mode == 1 && id as i16 <= 0 {
                    return Ok(true);
                }
                let object = self
                    .world
                    .objects
                    .get(&id)
                    .ok_or(Trap::native(10, "missing object"))?;
                let room = *object.data.get(ROOM).ok_or_else(|| {
                    Trap::Invalid(format!("object {id} has {} data words", object.data.len()))
                })?;
                if room == NO_ROOM {
                    return Ok(true);
                }
                self.world.mark_room(room);
                Ok(true)
            }
            _ => Err(Trap::native(3, "invalid Refresh mode")),
        }
    }
}
