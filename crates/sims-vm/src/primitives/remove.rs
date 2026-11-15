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
//! The queueing is the original's (0x442d70, `queue_removal` below): the
//! object leaves the lot through the lot's placement
//! (`Interpreter::take_off`, the object's virtual function 0x50), except
//! while the load has not worked the rooms out, when `Machine::leave_lot`
//! in `removal.rs` takes an object already off the lot and fails loudly
//! for one that is not simple. A tile of a group stands for its lead tile
//! (0x442d9d). A person
//! leaves through `Interpreter::take_off_to_remove`, the one way a person
//! leaves the lot that is ported.
//!
//! Flag bit 1 asks that removing the object does not reset
//! the objects that depend on it; it is recorded on the object for
//! whatever carries the removal out.

use crate::factory::word::GROUP;
use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, word};
use crate::placement::capture;
use crate::visits::function;
use sims_sim::rooms::RoomError;
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

    /// The object the removal queue takes for `id` (0x442d9d to 0x442e38):
    /// for an object whose definition names a group (word 10, +0x14), and
    /// that is a tile of one, the group's lead tile, unless the type names
    /// function 8 (0x42344e), when the tile goes alone. Fails loudly for
    /// that tile alone, which is not ported, and for an object that is a
    /// tile of a group without its definition naming one, or names one
    /// without being a tile (the original then does nothing, 0x442ddd);
    /// neither is a state the factory makes.
    fn removed_for(&self, id: u16) -> Result<u16, Trap> {
        let object = self
            .world
            .objects
            .get(&id)
            .ok_or(Trap::native(10, "missing object"))?;
        let interpreter = self.interpreter();
        let group = interpreter.words(self.world, id)?[GROUP] != 0;
        match (&object.multipart, group) {
            (None, false) => Ok(id),
            (Some(part), true) => {
                if interpreter.has_function(self.world, id, function::DYNAMIC_GROUP) {
                    return Err(Trap::Unsupported(
                        "removing a tile of a group whose type has the dynamic group function \
                         (8): the tile goes alone (0x442def), which is not ported"
                            .into(),
                    ));
                }
                Ok(if part.parent == 0 {
                    id
                } else {
                    part.parent as u16
                })
            }
            (Some(_), false) => Err(Trap::Unsupported(format!(
                "removing object {id}, a tile of a group whose definition names no group \
                 (0x442db5), which is not ported"
            ))),
            (None, true) => Err(Trap::Unsupported(format!(
                "removing object {id}, whose definition names a group but which is no tile of \
                 one: the original queues nothing (0x442ddd), which is not ported"
            ))),
        }
    }

    /// The removal queue (0x442d70): the object leaves the lot and is
    /// queued, unless it or an object holding it is queued already.
    ///
    /// 1. A tile of a group stands for its group: its lead tile is queued
    ///    instead ([`Machine::removed_for`]).
    /// 2. The objects holding it are walked, each the container of the
    ///    one before as `capture` gives it (0x43265e, 0x4325b5); if one of
    ///    them is queued, nothing happens.
    /// 3. If the object itself is queued, nothing happens.
    /// 4. It leaves the lot (its virtual function 0x50, 0x433125, asked to
    ///    tell the registry: `Interpreter::take_off_to_remove`, which for a
    ///    person also covers its animator's refresh). For a person that is
    ///    not removing itself and yielding, the refresh, which runs only if
    ///    the house view drew it, must write nothing but its animator, which
    ///    nothing may then read before the person is deleted
    ///    (`Interpreter::refresh_before_removal`). While the load has
    ///    not worked the lot's rooms out yet, that way cannot find the
    ///    room of the off-lot place, so an object then goes the narrower
    ///    way of `Machine::leave_lot`, which the load was compared
    ///    through. It takes only an object already off the lot that runs
    ///    no callback for leaving, works its footprint out again as the
    ///    general way does, and fails loudly for any other object.
    ///    A fault in a callback the general way runs keeps the object,
    ///    tree and node it happened at.
    /// 5. Its mark not to reset what depends on it (object flag 0x1000) is
    ///    set from flag bit 1, and it joins the queue.
    fn queue_removal(&mut self, id: u16, flags: u8) -> Result<(), Trap> {
        let id = self.removed_for(id)?;
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
        // A person's take-off refreshes its animator (0x58da12), which
        // for a person the view drew advances its head and writes its
        // data words (0x590768). Whether the view drew it is not kept.
        // The person removing itself and yielding is taken as it is: its
        // own tick follows at once and its deletion ends the update. Any
        // other take-off of a person is taken only when the refresh would
        // write nothing but its animator, which nothing may then read
        // before the deletion destroys it.
        let person = self
            .world
            .objects
            .get(&id)
            .is_some_and(|o| o.person.is_some());
        let unseen_refresh = person && (id != self.me || flags & 1 != 0 || self.synchronous);
        if matches!(
            self.world.room_at([OFF_LOT, OFF_LOT, 1]),
            Err(RoomError::NoRooms)
        ) {
            self.leave_lot(id)?;
        } else {
            let me = self.me;
            self.interpreter()
                .take_off_to_remove(self.world, id, true)
                .map_err(|fault| fault.nested(me))?;
        }
        if unseen_refresh {
            let pending = self.interpreter().refresh_before_removal(self.world, id)?;
            if let Some(taken_off) = self.taken_off {
                taken_off.borrow_mut().insert(id, pending);
            }
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
