//! Drop Onto (opcode 43, 0x44e96a, from the general table at 0x45246a):
//! put what I hold in one of my container slots into a container slot of
//! the stack object.
//!
//! Operands: two pairs of words, the source slot (words 0 and 1) and the
//! destination slot (words 2 and 3). In each pair the first word says
//! where the second comes from: 0, the second word is the slot itself;
//! anything else, the second word names one of the frame's arguments
//! (0x47518b), which must be at least 0 and below the frame's argument
//! count (frame byte +7), or it is native error 8. The source is read
//! first, then the destination. In order:
//!
//! 1. My source slot, as scripts count it (object +0x130, record `slot +
//!    1` of 0x24 bytes), must be one I have, or it is native error 0x1b.
//! 2. The stack object (frame +4) is looked up in the registry
//!    (0x40743b); none is native error 0x15.
//! 3. The object the slot holds (record +0x10): none (0) takes the false
//!    branch. Its ID is looked up again; one that names no object is
//!    called through a null pointer in the original, and fails here.
//! 4. The held object's place test (virtual function 0x54) for the stack
//!    object's destination slot ([`crate::Interpreter::fits_in`]). The
//!    record it is given (0x432679) takes the container's position, and
//!    level 1 whatever the container's. If it refuses, the false branch is
//!    taken and nothing changes.
//! 5. The held object is put there with the lot told (virtual function
//!    0x58, its last argument 1; [`crate::Interpreter::put`]), and the
//!    true branch is taken. What the put returns is not read.
//!
//! As for Grab and Drop, a put that fails partway (on what the port does
//! not model) leaves the lot as it was before, so the fault reports a
//! consistent lot.
//!
//! **Unverified**: no capture covers a Drop Onto. The first reached is
//! the Goth house's tick 14673 (person 24, tree 0x101c node 19).

use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::{Place, on_lot};

/// The level the record of 0x44e96a's place test and put names (0x44ea93).
const LEVEL: i32 = 1;

impl Machine<'_, '_> {
    pub(crate) fn drop_onto(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let word = |n: usize| i16::from_le_bytes([operands[2 * n], operands[2 * n + 1]]);
        let source = self.drop_onto_slot(word(0), word(1))?;
        let destination = self.drop_onto_slot(word(2), word(3))?;
        let me = self.me;
        let record = usize::try_from(source)
            .ok()
            .map(|slot| slot + 1)
            .and_then(|record| self.me().ok()?.slots.get(record))
            .ok_or(Trap::native(0x1b, "Drop Onto names a slot I do not have"))?;
        let held = record.object;
        let target = self.frame()?.stack_object;
        let target = u16::try_from(target)
            .ok()
            .filter(|id| *id > 0 && self.world.objects.contains_key(id))
            .ok_or(Trap::native(0x15, "Drop Onto has no stack object"))?;
        if held == 0 {
            return Ok(false);
        }
        let held = u16::try_from(held)
            .ok()
            .filter(|id| self.world.objects.contains_key(id))
            .ok_or_else(|| {
                Trap::Unsupported(format!(
                    "Drop Onto by object {me}: slot {source} holds {held}, which names no \
                     object (the original calls through a null pointer, 0x44ea98)"
                ))
            })?;
        let [p0, p1, _] = self
            .world
            .objects
            .get(&target)
            .ok_or(Trap::native(0x15, "missing object"))?
            .position;
        let position = [p0, p1];
        let slot = i32::from(destination);
        let interpreter = self.interpreter();
        let unsupported = |fault| {
            Trap::Unsupported(format!(
                "Drop Onto of object {held} by object {me} into object {target}: {fault}"
            ))
        };
        if !interpreter
            .fits_in(self.world, held, target, slot, position, LEVEL)
            .map_err(unsupported)?
        {
            return Ok(false);
        }
        let before = self.world.clone();
        let put = interpreter.put(
            self.world,
            held,
            Place {
                on_lot: on_lot(position),
                position,
                container: Some(target),
                slot,
                level: LEVEL,
            },
            true,
        );
        if let Err(fault) = put {
            *self.world = before;
            return Err(unsupported(fault));
        }
        Ok(true)
    }

    /// One of Drop Onto's slots from its pair of operand words (0x44e977
    /// to 0x44e9e2): `source` 0, `value` itself; otherwise argument
    /// `value` of the frame, native error 8 when it is not one.
    fn drop_onto_slot(&self, source: i16, value: i16) -> Result<i16, Trap> {
        if source == 0 {
            return Ok(value);
        }
        let parameters = &self.frame()?.parameters;
        usize::try_from(value)
            .ok()
            .and_then(|n| parameters.get(n).copied())
            .ok_or(Trap::native(
                8,
                "Drop Onto names an argument the frame does not have",
            ))
    }
}
