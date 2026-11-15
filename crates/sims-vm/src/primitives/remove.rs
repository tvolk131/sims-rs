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
//! The queueing is the original's (0x442d70, `queue_removal` below): an
//! object on the lot leaves it through the lot's placement
//! (`Interpreter::take_off`, the object's virtual function 0x50), and an
//! object already off the lot through `Machine::leave_lot` in
//! `removal.rs`, which fails loudly for one that is not simple. A tile of
//! a group and a person fail loudly.
//!
//! Flag bit 1 asks that removing the object does not reset
//! the objects that depend on it; it is recorded on the object for
//! whatever carries the removal out.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, word};
use crate::placement::capture;
use std::collections::BTreeSet;

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
        self.queue_removal(id, flags)?;
        if !self.synchronous && id == self.me && flags & 1 == 0 {
            return Ok(Flow::Yield);
        }
        Ok(Flow::Branch(true))
    }

    /// The removal queue (0x442d70): the object leaves the lot and is
    /// queued, unless it or an object holding it is queued already.
    ///
    /// 1. A tile of a group stands for its group: the original queues the
    ///    lead tile unless a test of the object (0x42344e) says otherwise.
    ///    That test is not decoded, so a tile of a group fails loudly.
    /// 2. The objects holding it are walked, each the container of the
    ///    one before as `capture` gives it (0x43265e, 0x4325b5); if one of
    ///    them is queued, nothing happens.
    /// 3. If the object itself is queued, nothing happens.
    /// 4. It leaves the lot (its virtual function 0x50, 0x433125, asked to
    ///    tell the registry: `Interpreter::take_off`). An object already
    ///    off the lot, as the lot-wide controllers are, still goes the
    ///    narrower way of `Machine::leave_lot`, which the load and the
    ///    ticks so far were compared through: the general way works out
    ///    the room of the off-lot place, and during the load the rooms are
    ///    not worked out yet.
    /// 5. Its mark not to reset what depends on it (object flag 0x1000) is
    ///    set from flag bit 1, and it joins the queue.
    fn queue_removal(&mut self, id: u16, flags: u8) -> Result<(), Trap> {
        if self
            .world
            .objects
            .get(&id)
            .ok_or(Trap::native(10, "missing object"))?
            .multipart
            .is_some()
        {
            return Err(Trap::Unsupported(
                "removing a tile of a group, which may stand for its lead tile (0x442db5)".into(),
            ));
        }
        let mut seen = BTreeSet::from([id]);
        let mut holder = id;
        while let Some(container) = capture(self.world, holder)?.container {
            if !seen.insert(container) {
                return Err(Trap::Invalid(format!(
                    "the objects holding object {id} form a loop"
                )));
            }
            if self.world.removals.contains(&container) {
                return Ok(());
            }
            holder = container;
        }
        if self.world.removals.contains(&id) {
            return Ok(());
        }
        let position = self.world.objects[&id].position;
        if position[..2] == [OFF_LOT, OFF_LOT] {
            self.leave_lot(id)?;
        } else {
            self.interpreter()
                .take_off(self.world, id, true)
                .map_err(|fault| fault.trap)?;
        }
        let object = self
            .world
            .objects
            .get_mut(&id)
            .ok_or(Trap::native(10, "missing object"))?;
        object.removal_skips_resets = flags & 2 != 0;
        self.world.removals.push(id);
        Ok(())
    }
}
