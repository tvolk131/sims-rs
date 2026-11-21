//! Drop (opcode 5, 0x454430, from the general table at 0x45246a): put
//! what I hold in my first container slot on the floor in front of me.
//! The inverse of [`Grab`](super::grab).
//!
//! The original ignores the operands. In order:
//!
//! 1. My first container slot (slot 0 as scripts count it; object +0x130,
//!    its second record of 0x24 bytes, when there is one) names the held
//!    object by its ID (record +0x10), which is looked up in the registry
//!    (0x40743b). No such slot, or an ID that is not above 0 or names no
//!    object, takes the false branch.
//! 2. The place is one tile from mine (+0xf4, +0xf8) in my direction
//!    (data word 1, +0x4c; 0x42228a), on my level (+0x110).
//! 3. The held object's place test (virtual function 0x54) there, with no
//!    container ([`crate::Interpreter::fits`]). If it refuses, the false
//!    branch is taken and nothing changes.
//! 4. The held object is put there with the lot told (virtual function
//!    0x58, its last argument 1; [`crate::Interpreter::put`]), and the
//!    true branch is taken. What the put returns is not read.
//!
//! As for Grab, a put that fails partway (on what the port does not
//! model) leaves the lot as it was before the Drop, so the fault reports
//! a consistent lot.
//!
//! **Unverified**: no capture covers a Drop. The first reached is the
//! Goth house's tick 5310 (object 12, tree 0x1006 node 12), past the
//! last capture. In play mode, a person's tree 0x0139 (node 14) drops
//! the snack it is eating when the player cancels it.

use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::{Place, on_lot};
use crate::routing::slot::step;

/// Data word 1: the direction an object faces (+0x4c).
const DIRECTION: usize = 1;

impl Machine<'_, '_> {
    pub(crate) fn drop_held(&mut self) -> Result<bool, Trap> {
        let me = self.me;
        let object = self.me()?;
        let Some(held) = object
            .slots
            .get(1)
            .and_then(|slot| u16::try_from(slot.object).ok())
            .filter(|id| *id > 0 && self.world.objects.contains_key(id))
        else {
            return Ok(false);
        };
        let [p0, p1, level] = object.position;
        let facing = *object
            .data
            .get(DIRECTION)
            .ok_or_else(|| Trap::Invalid(format!("object {me} has no data word {DIRECTION}")))?;
        // The step's first word goes to +0xf4, which is position[1].
        let [first, second] = step(i32::from(facing));
        let position = [p0.wrapping_add(second), p1.wrapping_add(first)];
        let interpreter = self.interpreter();
        let unsupported = |fault: crate::fault::Fault| {
            fault.unsupported_in(format!("Drop of object {held} by object {me}"))
        };
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
