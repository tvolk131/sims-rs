//! Drop (opcode 5, 0x454430, from the general table at 0x45246a): put
//! what I hold in my first container slot on the floor in front of me.
//! The inverse of [`Grab`](super::grab).
//!
//! The original ignores the operands. In order:
//!
//! 1. The object in my first container slot (object +0x130, its second
//!    record, the slot scripts count as 0; a list of one record or none
//!    holds nothing) is looked up in the registry (0x40743b). Nothing
//!    there takes the false branch.
//! 2. The place is my position (+0xf4, +0xf8) with one tile's step in
//!    my facing (data word 1, +0x4c; 0x42228a) added, on my level
//!    (+0x110).
//! 3. The held object's place test there, on the ground (virtual
//!    function 0x54 with no container; [`crate::Interpreter::fits`]). If
//!    it refuses, the false branch is taken and nothing changes.
//! 4. The object is put there with the lot told (virtual function 0x58,
//!    its last argument 1; [`crate::Interpreter::put`]), and the true
//!    branch is taken. What the put returns is not read. The original
//!    passes slot 0 with no container, which a put on the ground does
//!    not read.
//!
//! A put that fails partway (on what the port does not model) leaves the
//! lot as it was before the Drop, so the fault reports a consistent lot.
//!
//! **Unverified**: no capture covers a Drop. The first one reached is a
//! person's tree 0x0139 (node 14) after the player cancels the snack the
//! person is eating: the person puts the snack down.

use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::{Place, on_lot};
use crate::routing::slot::step;

/// Object data word 1: the facing, 0 to 7.
const DIRECTION: usize = 1;

impl Machine<'_, '_> {
    pub(crate) fn drop_held(&mut self) -> Result<bool, Trap> {
        let me = self.me;
        let object = self
            .world
            .objects
            .get(&me)
            .ok_or(Trap::native(10, "missing object"))?;
        let Some(held) = object
            .slots
            .get(1)
            .and_then(|slot| u16::try_from(slot.object).ok())
            .filter(|id| *id > 0 && self.world.objects.contains_key(id))
        else {
            return Ok(false);
        };
        let [p0, p1, level] = object.position;
        let facing = object
            .data
            .get(DIRECTION)
            .copied()
            .ok_or_else(|| Trap::Invalid(format!("object data has no word {DIRECTION}")))?;
        let [d0, d1] = step(i32::from(facing));
        let position = [p0.wrapping_add(d0), p1.wrapping_add(d1)];
        let interpreter = self.interpreter();
        let unsupported =
            |fault| Trap::Unsupported(format!("Drop of object {held} by object {me}: {fault}"));
        if !interpreter
            .fits(self.world, held, position, level)
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
                container: None,
                slot: -1,
                level,
            },
            true,
        );
        if let Err(fault) = put {
            *self.world = before;
            return Err(unsupported(fault));
        }
        Ok(true)
    }
}
