//! Grab (opcode 4, 0x451dda, from the general table at 0x45246a): put the
//! stack object into my first container slot.
//!
//! The original ignores the operands. In order:
//!
//! 1. The stack object (frame +4) is looked up in the registry (0x40743b):
//!    an ID that is not above 0, or names no object, takes the false
//!    branch.
//! 2. The stack object's place test (virtual function 0x54), for my slot
//!    0 as scripts count it, at my position (+0xf4) and level (+0x110)
//!    ([`crate::Interpreter::fits_in`]). If it refuses, the false branch
//!    is taken and nothing changes.
//! 3. The stack object is put there with the lot told (virtual function
//!    0x58, its last argument 1; [`crate::Interpreter::put`]), and the true
//!    branch is taken. What the put returns is not read.
//!
//! There are three classes of object, each with its own virtual
//! functions: an ordinary object (0x666d34) and a person (0x667378) share
//! the place test 0x433311 and the put 0x433340; a tile of a multi-tile
//! group (0x666c20) has 0x422abb and 0x422c2f. The group's place test
//! refuses any container at once (0x422ac8, code 0x10), so a tile of a
//! group is never grabbed: the false branch is taken.
//!
//! A put that fails partway (on what the port does not model) leaves the
//! lot as it was before the Grab, so the fault reports a consistent lot.
//!
//! My slot 0 is my first container slot: for a person with no SLOT
//! containers of its own definition, the slot it carries in, at height 6
//! (0x45962b; see the place test's container slots).
//!
//! **Unverified**: no capture covers a Grab. The first Grab reached is
//! the snack order's fridge tree 0x100d (node 9), in play mode after a
//! player's snack order: the person takes the snack the fridge made into
//! the slot it carries in.
//!
//! The old prototype read the same steps
//! (legacy/crates/sims-vm/src/transfer.rs and
//! legacy/crates/sims-sim/src/lot_transfer.rs).

use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::{Place, on_lot};

impl Machine<'_, '_> {
    pub(crate) fn grab(&mut self) -> Result<bool, Trap> {
        let target = self.frame()?.stack_object;
        let Some(target) = u16::try_from(target)
            .ok()
            .filter(|id| *id > 0 && self.world.objects.contains_key(id))
        else {
            return Ok(false);
        };
        let me = self.me;
        let [p0, p1, level] = self
            .world
            .objects
            .get(&me)
            .ok_or(Trap::native(10, "missing object"))?
            .position;
        let position = [p0, p1];
        let interpreter = self.interpreter();
        let unsupported =
            |fault| Trap::Unsupported(format!("Grab of object {target} by object {me}: {fault}"));
        if !interpreter
            .fits_in(self.world, target, me, 0, position, level)
            .map_err(unsupported)?
        {
            return Ok(false);
        }
        let before = self.world.clone();
        let put = interpreter.put(
            self.world,
            target,
            Place {
                on_lot: on_lot(position),
                position,
                container: Some(me),
                slot: 0,
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
