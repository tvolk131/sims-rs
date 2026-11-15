//! Drop Onto (opcode 43, 0x44e96a, from the general table at 0x45246a):
//! put what I hold in one of my container slots into a slot of the stack
//! object.
//!
//! The operands are two pairs of words. In each pair the first word says
//! how to read the second: 0 reads it as a number, anything else as the
//! index of one of my frame's parameters (0 to its count, frame byte +7,
//! less one; native error 8 otherwise; 0x47518b is the parameters'
//! start). Words 0 and 1 give my slot, words 2 and 3 the stack object's,
//! both as scripts count them. In order:
//!
//! 1. Both slot numbers are read, mine first.
//! 2. My slot must be one of my container slots (object +0x130, records
//!    of 0x24 bytes, the first of which is not a container slot): native
//!    error 0x1b otherwise.
//! 3. The stack object (frame +4) is looked up in the registry
//!    (0x40743b): native error 0x15 if it names no object.
//! 4. My slot names the held object by its ID (record +0x10). An ID of 0
//!    takes the false branch.
//! 5. The held object's place test (virtual function 0x54) for the stack
//!    object's slot ([`crate::Interpreter::fits_in`]). The point it is
//!    given, (-16, -16), is not read: with a container the test takes the
//!    container's position (0x432679). The level it is given, 1, is
//!    read, whatever the container's level. If it refuses, the false
//!    branch is taken and nothing changes.
//! 6. The held object is put there with the lot told (virtual function
//!    0x58, its last argument 1; [`crate::Interpreter::put`]), and the
//!    true branch is taken. What the put returns is not read.
//!
//! An ID in my slot that names no object fails loudly, as the original
//! then calls through a null pointer. As for Grab, a put that fails
//! partway (on what the port does not model) leaves the lot as it was
//! before, so the fault reports a consistent lot.
//!
//! **Unverified**: no capture covers a Drop Onto. The first reached is
//! the Pleasants' tick 934 (person 241, tree 0x2018 node 15).

use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::Place;

/// The point the original gives the place test and the put (0x44ea7d),
/// which neither reads for a container.
const POINT: [i32; 2] = [-16, -16];
/// The level it gives them, which the place test reads.
const LEVEL: i32 = 1;

impl Machine<'_, '_> {
    pub(crate) fn drop_onto(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let word = |i: usize| i16::from_le_bytes([operands[2 * i], operands[2 * i + 1]]);
        let mine = self.slot_number(word(0), word(1))?;
        let theirs = self.slot_number(word(2), word(3))?;
        let me = self.me;
        let object = self.me()?;
        // 0x44e9ed: one of my container slots, the record after the first.
        let slot = usize::try_from(mine)
            .ok()
            .and_then(|slot| object.slots.get(slot + 1))
            .ok_or(Trap::native(0x1b, "Drop Onto names no slot of mine"))?;
        let held = slot.object;
        let target = self.frame()?.stack_object;
        let target = u16::try_from(target)
            .ok()
            .filter(|id| self.world.objects.contains_key(id))
            .ok_or(Trap::native(0x15, "Drop Onto has no stack object"))?;
        if held == 0 {
            return Ok(false);
        }
        let held = u16::try_from(held)
            .ok()
            .filter(|id| self.world.objects.contains_key(id))
            .ok_or_else(|| {
                Trap::Unsupported(format!(
                    "Drop Onto by object {me}: its slot {mine} names object {held}, which does \
                     not exist, and the original calls through a null pointer (0x44ea78)"
                ))
            })?;
        let [p0, p1, _] = self
            .world
            .objects
            .get(&target)
            .ok_or(Trap::native(10, "missing object"))?
            .position;
        let interpreter = self.interpreter();
        let unsupported = |fault| {
            Trap::Unsupported(format!(
                "Drop Onto of object {held} by object {me} into object {target}: {fault}"
            ))
        };
        if !interpreter
            .fits_in(self.world, held, target, theirs, [p0, p1], LEVEL)
            .map_err(unsupported)?
        {
            return Ok(false);
        }
        let before = self.world.clone();
        let put = interpreter.put(
            self.world,
            held,
            Place {
                on_lot: false,
                position: POINT,
                container: Some(target),
                slot: theirs,
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

    /// A slot number from a pair of operand words (0x44e977, 0x44e9ae):
    /// `value` itself when `kind` is 0, otherwise my frame's parameter
    /// `value`.
    fn slot_number(&self, kind: i16, value: i16) -> Result<i32, Trap> {
        if kind == 0 {
            return Ok(i32::from(value));
        }
        let frame = self.frame()?;
        usize::try_from(value)
            .ok()
            .and_then(|parameter| frame.parameters.get(parameter))
            .map(|&v| i32::from(v))
            .ok_or(Trap::native(8, "Drop Onto names no parameter"))
    }
}
