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
//! Three cases are ported. One is what the lot-wide controllers and the
//! indicators need: an object that is off the lot, one tile, not a
//! person, held by nothing, with no callbacks for leaving the lot, and
//! that no person on the lot is using. Another is a person on its way
//! out (Remove Object Instance on itself, then the removal at the end of
//! the lot update), which leaves through the lot's placement
//! (`Interpreter::take_off_to_remove`) and resets in its own way
//! (`Interpreter::reset_person_to_delete`). The third is a multi-tile
//! object, which leaves through the lot's placement too
//! (`Interpreter::take_off`, 0x422d86) and is deleted tile by tile
//! (0x440f37). Anything else fails with an error that says what is
//! missing.

use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Machine};
use crate::sounds::SoundEvent;
use sims_sim::World;
use sims_sim::rooms::RoomError;

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
    /// callback run; both fail. Its footprint is worked out again at the
    /// off-lot place, as the original's move does (0x433e50), since the
    /// factory's may differ.
    pub(crate) fn leave_lot(&mut self, id: u16) -> Result<(), Trap> {
        let object = self
            .world
            .objects
            .get(&id)
            .ok_or(Trap::native(10, "missing object"))?;
        let definition = self.definition(object.guid)?;
        // Taking it off the lot works its footprint out again (0x433e50),
        // which changes one the factory left otherwise.
        let footprint = self.interpreter().off_lot_footprint(self.world, id)?;
        let object = &self.world.objects[&id];
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
        object.footprint = footprint;
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

    /// Removes an object at once (0x44123f): it leaves the lot (its
    /// virtual function 0x50 with 1, 0x4412a4) and is then deleted.
    ///
    /// An object of one tile that is not a person leaves the narrow way
    /// of [`Machine::leave_lot`], which the load was compared through. A
    /// tile of a group takes its whole group off through the lot's
    /// placement (its vtable 0x666c20's function 0x50, 0x422d86,
    /// [`Interpreter::take_off`]), which also needs the rooms worked out.
    /// A person leaves through
    /// [`Interpreter::take_off_to_remove`], which needs the lot's rooms
    /// worked out; before the load has done so it fails.
    ///
    /// The original remembers the room of an object that lights it (data
    /// word 7 nonzero) and marks that room for redrawing afterwards
    /// (signal 0xf3), and first drops the interface's references to the
    /// object (0x447d8b). Both are presentation.
    pub fn remove_now(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let person = world.objects.get(&id).is_some_and(|o| o.person.is_some());
        if person {
            if matches!(
                world.room_at([OFF_LOT, OFF_LOT, 1]),
                Err(RoomError::NoRooms)
            ) {
                return Err(fail(Trap::Unsupported(
                    "removing a person before the load has worked the lot's rooms out is not \
                     ported"
                        .into(),
                )));
            }
            // The take-off's animator refresh (0x58da12) is not modeled:
            // the person is deleted right after, so only its last reset
            // could read what the refresh writes (see
            // `Interpreter::take_off_to_remove`).
            self.take_off_to_remove(world, id, true)?;
        } else if world
            .objects
            .get(&id)
            .is_some_and(|o| o.multipart.is_some())
        {
            if matches!(
                world.room_at([OFF_LOT, OFF_LOT, 1]),
                Err(RoomError::NoRooms)
            ) {
                return Err(fail(Trap::Unsupported(
                    "removing a tile of a group (0x422d86, from 0x4412a4) before the load has \
                     worked the lot's rooms out is not ported"
                        .into(),
                )));
            }
            self.take_off(world, id, true)?;
        } else {
            let mut machine = self.machine(world, id, 0, false);
            machine.leave_lot(id).map_err(fail)?;
        }
        self.delete(world, id)
    }

    /// Deletes an object that is off the lot (0x440eeb):
    ///
    /// 1. It gives up the ownership of interface events (registry +0xa8,
    ///    0x444bc8 with 0).
    /// 2. If it is the selected Sim, nobody is selected (0x445449 with 0,
    ///    [`Interpreter::select`]), which is not ported and fails.
    /// 3. Every person lets go of it ([`crate::forget`], 0x444306).
    /// 4. It resets one last time (its virtual function 0x38 with 0): an
    ///    object through [`Interpreter::reset`], which runs its Reset tree
    ///    if it has one; a person through
    ///    [`Interpreter::reset_person_to_delete`] (0x45e791), with the
    ///    deletion mark (object flag 0x40) that keeps it off the lot.
    /// 5. Each object it holds, slot 1 on as scripts count them, is
    ///    removed at once ([`Interpreter::remove_now`], 0x441196): each
    ///    slot is read again as the walk reaches it, over the slots it had
    ///    when the walk began.
    /// 6. Its sounds are stopped (the base destructor 0x432331 calling
    ///    0x4323c9, [`crate::sounds`]), and it is unlinked from the
    ///    registration list and from its type's instance list (0x44383c),
    ///    and its storage is freed. A person's destructor (0x459511) also
    ///    frees its route (+0xaec), its animator (+0xadc, 0x588080), its
    ///    uses (+0xb00), route contexts (+0xaf0), path (+0xae0) and queue
    ///    (+0x820), so what [`World`] keeps for them by the person's ID
    ///    goes: its animator, route, tick state and unknown data words. It
    ///    also leaves the lot's list of people (lot +0x84).
    ///
    /// A person using the object fails in step 3 (`crate::forget`). A
    /// route aimed at the object fails before anything changes: the
    /// original leaves the route a pointer to the freed object.
    ///
    /// A tile of a multi-tile object is deleted with its whole group
    /// (0x440f37 to 0x440ffa): a tile that is not the lead deletes its
    /// lead instead; the lead marks every tile for deletion (object flag
    /// 0x40, not kept) and then deletes each tile in turn, from itself
    /// along the group's list, each through the steps above
    /// (0x440eeb with 1). The next tile is read before a tile goes. Each
    /// tile's reset, if its type has no Reset tree, takes what is left of
    /// its group off the lot once more ([`Interpreter::reset`]), and its
    /// destructor (0x42283a) takes it out of its group first
    /// (`leave_group`, 0x4228d6), so that the next tile leads the rest.
    ///
    /// The ID stays in [`World::removals`] if it is queued there. The
    /// original's queue holds bare IDs and whoever drains it skips the ones
    /// that name no object.
    ///
    /// Nothing here writes a neighborhood record, a relationship or a
    /// family: a person's record keeps what it held, and other objects'
    /// relationships and frames that name the ID keep it. Neither does
    /// the deletion touch the rooms: the room a person was last seen in
    /// keeps its mark that a person is in it, which the next person to
    /// leave it clears.
    ///
    /// Not modeled, being presentation: the signals to the interface
    /// (0x87, 0x88), the object's redraw (0x434101), the registry's
    /// interface hooks (0x4481eb, which changes nothing for a person or
    /// any object but an indicator or a star marker, and the list at
    /// registry +0x30, 0x464fa2), the interface object of GUID 0x3EEC206C
    /// (registry +0x80), and the release of its indicator. The original
    /// also sets the object's deletion mark (object flag 0x40), which only
    /// a person's reset reads.
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
        let Some(part) = &object.multipart else {
            return self.delete_one(world, id);
        };
        if part.parent != 0 {
            let lead = part.parent as u16;
            if world
                .objects
                .get(&lead)
                .and_then(|o| o.multipart.as_ref())
                .is_none_or(|p| p.parent != 0)
            {
                return Err(fail(Trap::Invalid(format!(
                    "object {id}'s lead tile {lead} is no lead of a group"
                ))));
            }
            return self.delete(world, lead);
        }
        let mut tile = Some(id);
        while let Some(this) = tile {
            tile = world.next_parts.get(&this).copied();
            self.delete_one(world, this)?;
        }
        Ok(())
    }

    /// One object's deletion (0x440fff on, 0x440eeb with 1): the steps of
    /// [`Interpreter::delete`].
    fn delete_one(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let person = object.person.is_some();
        let selected = object.selected;
        // A route context holds a pointer to its slot's object
        // (context +0x48); what the original does with it when the object
        // goes is not known.
        if let Some((&other, _)) = world
            .routing
            .routes
            .iter()
            .find(|(_, r)| r.contexts.iter().any(|c| c.target == id))
        {
            return Err(fail(Trap::Unsupported(format!(
                "person {other} has a route to object {id}; deleting an object a route \
                 leads to is not ported"
            ))));
        }
        if world.event_owner == id {
            world.event_owner = 0;
        }
        if selected {
            self.select(world, None)?;
        }
        self.drop_references(world, id)?;
        if person {
            self.reset_person_to_delete(world, id)?;
        } else {
            self.reset(world, id, true)?;
        }
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::Invalid("the object's Reset deleted it".into())))?;
        // 0x4410e7: the count is taken once; each slot is read as the walk
        // reaches it.
        for slot in 1..object.slots.len() {
            let held = world
                .objects
                .get(&id)
                .and_then(|o| o.slots.get(slot))
                .map_or(0, |s| s.object);
            // 0x44123f does nothing for an ID that names no object.
            if let Ok(held) = u16::try_from(held)
                && held != 0
                && world.objects.contains_key(&held)
            {
                self.remove_now(world, held)?;
            }
        }
        // A tile's destructor leaves its group (0x42283a, 0x4228d6) before
        // the base destructor runs.
        if world
            .objects
            .get(&id)
            .is_some_and(|o| o.multipart.is_some())
        {
            leave_group(world, id).map_err(fail)?;
        }
        // The object's destructor stops its sounds (0x4323c9).
        self.sound(SoundEvent::Stop { object: id as i16 });
        world.objects.remove(&id);
        if let Some(deleted) = self.deleted {
            deleted.borrow_mut().push(id);
        }
        world.frame_storage.remove(&id);
        // So does its headline list (0x4323ce).
        world.headlines.remove(&id);
        // The object's slot table goes with it.
        world
            .routing
            .slot_flags
            .retain(|&(object, _), _| object != id);
        world.routing.slots.retain(|&(object, _), _| object != id);
        world.fires.retain(|&other| other != id);
        world.order.retain(|&other| other != id);
        world.people.retain(|&other| other != id);
        // A type's instance list outlives its instances (0x44383c).
        for instances in world.instances.values_mut() {
            instances.retain(|&other| other != id);
        }
        if person {
            // The person's destructor (0x459511).
            world.animators.remove(&id);
            world.base_layers.remove(&id);
            world.translated_bones.remove(&id);
            world.routing.routes.remove(&id);
            world.person_ticks.remove(&id);
            world
                .routing
                .unknown_words
                .retain(|&(other, _)| other != id);
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
    /// Without one, an object that is being deleted is taken off the lot
    /// (its virtual function 0x50 with 1, 0x432055) and that is all: a
    /// tile of a group takes what is left of its group off
    /// ([`Interpreter::take_off`], 0x422d86), so each of its tiles is
    /// asked function 10 again; any other object leaves the narrow way of
    /// [`Machine::leave_lot`]. Any other is rebuilt ([`Interpreter::rebuild`]):
    /// taken off the lot, its storage reset, its Init and a first turn
    /// run, and put back where it was.
    ///
    /// Either way the scheduler's countdown for the object is dropped, its
    /// sounds are stopped (0x4320b4, [`crate::sounds`]), and the object
    /// gives up the ownership of interface events. Being deleted (object
    /// flag 0x40) only spares the rebuild (0x432063): the deletion's reset
    /// stops the object's sounds too, and its destructor then once more.
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
                "a person resets in its own way (Interpreter::reset_person); this is an \
                 object's reset"
                    .into(),
            )));
        }
        if self.has_function(world, id, RESET) {
            self.callback(world, id, RESET, 0)?;
            let mut machine = self.machine(world, id, 0, false);
            let object = machine.me_mut().map_err(fail)?;
            object.data[8] &= !0x20;
            object.data[62] = 0;
            machine.install_main().map_err(fail)?;
        } else if deleting && object.multipart.is_some() {
            self.take_off(world, id, true)?;
        } else if deleting {
            self.machine(world, id, 0, false)
                .leave_lot(id)
                .map_err(fail)?;
        } else {
            self.rebuild(world, id)?;
        }
        if let Some(object) = world.objects.get_mut(&id) {
            object.set_countdown(-1);
        }
        // Every path meets at 0x43208b and stops the object's sounds
        // (0x4320b4).
        self.sound(SoundEvent::Stop { object: id as i16 });
        if world.event_owner == id {
            world.event_owner = 0;
        }
        Ok(())
    }
}

/// A tile of a group leaves it, as its destructor does (0x4228d6). The
/// lead hands the group to the next tile: every tile after it takes that
/// tile as its lead, and that tile leads (0x4228e6 to 0x42290d). The
/// tile's own links are cleared; it is deleted right after.
///
/// Another tile would be taken out of its lead's list (0x422919 to
/// 0x422935), but [`Interpreter::delete`] deletes a group from its lead,
/// so that tile is always the lead by then; another is refused.
fn leave_group(world: &mut World, id: u16) -> Result<(), Trap> {
    let parent = world
        .objects
        .get(&id)
        .and_then(|o| o.multipart.as_ref())
        .ok_or_else(|| Trap::Invalid(format!("object {id} is no tile of a group")))?
        .parent;
    if parent != 0 {
        return Err(Trap::Invalid(format!(
            "tile {id} is deleted before its lead {parent}"
        )));
    }
    let next = world.next_parts.remove(&id);
    let mut steps = world.objects.len();
    let mut step = || {
        steps = steps
            .checked_sub(1)
            .ok_or_else(|| Trap::Invalid("the tiles of a group form a loop".into()))?;
        Ok::<_, Trap>(())
    };
    let set_lead = |world: &mut World, tile: u16, lead: u16| {
        world
            .objects
            .get_mut(&tile)
            .and_then(|o| o.multipart.as_mut())
            .ok_or_else(|| Trap::Invalid(format!("object {tile} is no tile of a group")))
            .map(|part| part.parent = lead as i16)
    };
    if let Some(next) = next {
        let mut tile = Some(next);
        while let Some(this) = tile {
            step()?;
            set_lead(world, this, next)?;
            tile = world.next_parts.get(&this).copied();
        }
        set_lead(world, next, 0)?;
    }
    Ok(())
}
