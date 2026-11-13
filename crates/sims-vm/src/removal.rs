//! Taking objects off the lot and destroying them.
//!
//! The original has three layers, and this module follows them:
//!
//! - [`Machine::leave_lot`] is the object's own "remove from lot"
//!   (0x433125): it detaches the object from its tile and its container
//!   and leaves it registered, off the lot.
//! - [`Interpreter::delete`] is the registry's deletion (0x440eeb): the
//!   object is marked, everything that refers to it lets go, it resets one
//!   last time ([`Interpreter::reset`]), and its storage is unlinked from
//!   the registry.
//! - [`Interpreter::remove_now`] is the immediate removal (0x44123f):
//!   leave the lot, then delete.
//!
//! [`Interpreter::clear`] is load step 5 (0x44279a), which removes every
//! object this way.
//!
//! Ported from the old prototype (legacy/crates/sims-sim/src/lot_clear.rs,
//! legacy/crates/sims-sim/src/lot_removal/immediate.rs,
//! legacy/crates/sims-sim/src/removal.rs and its `removal/base.rs`,
//! legacy/crates/sims-vm/src/deletion_entry.rs, and `BaseCallbackReset` in
//! legacy/crates/sims-vm/src/loading.rs), without its suspension points:
//! here each removal runs to its end.
//!
//! Only the case the lot-wide controllers and the indicators need is
//! ported: an object that is off the lot, one tile, not a person, holding
//! nothing and held by nothing, with no callbacks for leaving the lot, on a
//! lot with no people. Anything else fails with an error that says what is
//! missing.

use crate::factory::TURN_BUDGET;
use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Machine};
use crate::program::Tree;
use sims_sim::World;

/// Off the lot: where the factory puts a new object.
const OFF_LOT: i32 = -16;

/// The callback table's entry for the Reset tree.
const RESET: usize = 3;

/// The callbacks an object runs as it leaves the lot.
const LEAVING: [usize; 3] = [9, 10, 26];

impl Machine<'_, '_> {
    /// Takes an object off the lot (0x433125). It stays registered.
    ///
    /// Only an object that is already off the lot is handled: then nothing
    /// moves, and the words that say where it is are reset. An object on a
    /// tile, in a container or holding something needs the lot's placement
    /// services, and one with a callback for leaving the lot needs that
    /// callback run; both fail.
    pub(crate) fn leave_lot(&mut self, id: u16) -> Result<(), Trap> {
        let object = self
            .world
            .objects
            .get(&id)
            .ok_or(Trap::native(10, "missing object"))?;
        let definition = self.definition(object.guid)?;
        let callbacks = self
            .program
            .libraries
            .get(&definition.resource)
            .and_then(|l| l.callbacks.get(&definition.chunk));
        let leaves_quietly = LEAVING.iter().all(|&i| {
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
        Ok(())
    }
}

impl Interpreter<'_> {
    /// Load step 5: removes every object on the lot (0x44279a). First the
    /// two indicators in [`World::indicators`] (0x445f84), then whatever
    /// heads the registration list until the list is empty. The list is
    /// read again after each removal, because a Reset tree may create or
    /// queue objects. Returns the IDs in the order they were removed.
    ///
    /// The IDs in [`World::removals`] stay queued: the original erases
    /// that queue later in the load (0x428679), not here. The clock and the
    /// lot's size do not change.
    ///
    /// For the Goth house this was compared with a capture of the original
    /// at the moment the clear returns; see `docs/decisions.md`.
    ///
    /// Not modeled: the interface is switched off for the duration, each
    /// indicator's graphics are released, and the lot's pools of spare
    /// indicators are emptied. None of that is simulation state we hold.
    pub fn clear(&self, world: &mut World) -> Result<Vec<u16>, Fault> {
        let mut removed = Vec::new();
        for slot in 0..world.indicators.len() {
            let id = world.indicators[slot];
            if id != 0 {
                // 0x445f84 takes the indicator off the lot and deletes it
                // directly; the reference is cleared after the deletion.
                self.remove_now(world, id)?;
                removed.push(id);
                world.indicators[slot] = 0;
            }
        }
        while let Some(&id) = world.order.first() {
            self.remove_now(world, id)?;
            removed.push(id);
        }
        if let Some((&id, _)) = world.objects.first_key_value() {
            return Err(Fault {
                object: id,
                at: None,
                trap: Trap::Invalid("an object outside the registration list".into()),
            });
        }
        Ok(removed)
    }

    /// Removes an object at once (0x44123f): it leaves the lot and is then
    /// deleted.
    ///
    /// The original remembers the room of an object that lights it (data
    /// word 7 nonzero) and marks that room for redrawing afterwards, and
    /// first drops the interface's references to the object (0x447d8b).
    /// Rooms and the interface are not modeled.
    pub fn remove_now(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let mut machine = self.machine(world, id, 0, false);
        machine.leave_lot(id).map_err(|trap| Fault {
            object: id,
            at: None,
            trap,
        })?;
        self.delete(world, id)
    }

    /// Deletes an object that is off the lot (0x440eeb):
    ///
    /// 1. It gives up the ownership of interface events.
    /// 2. Every person drops their references to it (0x444306) and the
    ///    actions queued on it are cancelled.
    /// 3. It resets one last time ([`Interpreter::reset`]), which runs its
    ///    Reset tree if it has one.
    /// 4. The objects it holds are removed.
    /// 5. It is unlinked from the registration list and from its type's
    ///    instance list, and its storage is freed.
    ///
    /// Steps 2 and 4 are not ported: a lot with a person on it, and an
    /// object holding another, fail. A person and a tile of a multi-tile
    /// object have their own deletion, which fails too.
    ///
    /// The ID stays in [`World::removals`] if it is queued there. The
    /// original's queue holds bare IDs and whoever drains it skips the ones
    /// that name no object.
    ///
    /// Not modeled, being presentation: the signal to the interface
    /// (0x87), the object's redraw, and the release of its indicator. The
    /// original also sets the object's deletion mark (object flag 0x40),
    /// which only a person's reset reads.
    pub fn delete(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        if object.person.is_some() || object.multipart.is_some() {
            return Err(fail(Trap::Unsupported(
                "deleting a person or a multi-tile object is not ported".into(),
            )));
        }
        if world.objects.values().any(|o| o.person.is_some()) {
            return Err(fail(Trap::Unsupported(
                "deleting an object while people are on the lot is not ported: \
                 they must drop their references to it"
                    .into(),
            )));
        }
        if world.event_owner == id {
            world.event_owner = 0;
        }
        self.reset(world, id, true)?;
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::Invalid("the object's Reset deleted it".into())))?;
        if object.slots.iter().skip(1).any(|slot| slot.object != 0) {
            return Err(fail(Trap::Unsupported(
                "deleting an object that holds another is not ported".into(),
            )));
        }
        world.objects.remove(&id);
        world.order.retain(|&other| other != id);
        // A type's instance list outlives its instances (0x44383c).
        for instances in world.instances.values_mut() {
            instances.retain(|&other| other != id);
        }
        Ok(())
    }

    /// An object's reset (0x431fbe), for an object that is not a person.
    ///
    /// With a Reset tree (callback 3): the tree runs synchronously on top
    /// of whatever the object's thread holds, with the object's temps as
    /// its arguments. Then flag 0x20 of data word 8 and data word 62 are
    /// cleared and the thread restarts at Main.
    ///
    /// Without one, the object is taken off the lot. If it is being
    /// deleted that is all. Otherwise its storage is rebuilt, its Init
    /// runs, and it is put back where it was; that needs placement and is
    /// not ported, so it fails.
    ///
    /// Either way the scheduler's countdown for the object is dropped and
    /// the object gives up the ownership of interface events.
    ///
    /// A Reset entry whose tree is missing runs nothing but still counts
    /// as a Reset tree.
    pub fn reset(&self, world: &mut World, id: u16, deleting: bool) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        if object.person.is_some() {
            return Err(fail(Trap::Unsupported(
                "a person's reset is not ported".into(),
            )));
        }
        let definition = self
            .catalog
            .object(object.guid)
            .ok_or_else(|| fail(Trap::Invalid("object has no definition".into())))?;
        let tree = self
            .program
            .libraries
            .get(&definition.resource)
            .and_then(|l| l.callbacks.get(&definition.chunk))
            .and_then(|table| table.get(RESET))
            .map_or(0, |f| f.action);
        if tree != 0 {
            if !matches!(self.program.tree(definition.resource, tree), Tree::Missing) {
                let temps = object.temps.clone();
                self.call(world, id, tree, 0, &temps, 8, TURN_BUDGET)?;
            }
            let mut machine = self.machine(world, id, 0, false);
            let object = machine.me_mut().map_err(fail)?;
            object.data[8] &= !0x20;
            object.data[62] = 0;
            machine.install_main().map_err(fail)?;
        } else if deleting {
            self.machine(world, id, 0, false)
                .leave_lot(id)
                .map_err(fail)?;
        } else {
            return Err(fail(Trap::Unsupported(
                "resetting an object with no Reset tree rebuilds and replaces it; \
                 that is not ported"
                    .into(),
            )));
        }
        if let Some(object) = world.objects.get_mut(&id) {
            object.set_countdown(-1);
        }
        if world.event_owner == id {
            world.event_owner = 0;
        }
        Ok(())
    }
}
