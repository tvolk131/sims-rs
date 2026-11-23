//! Taking objects off the lot and destroying them.
//!
//! The original has three layers, and this module follows them:
//!
//! - [`Machine::leave_lot`] is the object's own "remove from lot"
//!   (0x433125): it detaches the object from its tile and its container
//!   and leaves it registered, off the lot.
//! - [`Interpreter::delete`] is the registry's deletion (0x440eeb): the
//!   object is marked, people let go of it, and it resets one last time
//!   ([`Interpreter::reset`]). Then it is unlinked and freed, and incoming
//!   object relationships are erased.
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

use std::cell::RefCell;
use std::collections::BTreeSet;

use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Machine};
use crate::sounds::SoundEvent;
use sims_sim::World;
use sims_sim::rooms::RoomError;

/// Off the lot: where the factory puts a new object.
const OFF_LOT: i32 = -16;

/// Sentinel stored in [`sims_sim::route::Context::target`] when its target
/// object is deleted while the route context survives (`0x440fff`). Because
/// live object IDs are positive `i16` (`1..=32767`) and `0` is reserved for
/// loaded contexts, `u16::MAX` can never collide with a reused object ID.
pub(crate) const DELETED_ROUTE_TARGET: u16 = u16::MAX;

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
    /// at the moment the clear returns; see `docs/decisions/`.
    ///
    /// Not modeled: the interface is switched off for the duration and each
    /// indicator's graphics are released. Pooled star markers (`STAR_MARKER`,
    /// `0x44602d..0x446133`) are cleared right after `world.indicators`.
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
        // 0x44602d..0x446133: after the two indicators, clear pooled star
        // markers (in-use first, then available).
        let mut stars_in_use = Vec::new();
        let mut stars_avail = Vec::new();
        for &id in world.order.iter().rev() {
            if let Some(obj) = world.objects.get(&id)
                && obj.guid == crate::placement::STAR_MARKER
            {
                if obj.data.get(2).copied().unwrap_or(0) != 0 {
                    stars_in_use.push(id);
                } else {
                    stars_avail.push(id);
                }
            }
        }
        for id in stars_in_use.into_iter().chain(stars_avail) {
            if world.objects.contains_key(&id) {
                self.remove_now(world, id)?;
                removed.push(id);
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
    /// An object of one tile that is not a person leaves through the
    /// lot's placement (0x433125, [`Interpreter::take_off`]) once the load
    /// has worked the lot's rooms out, and before that the narrow way of
    /// [`Machine::leave_lot`], which the load was compared through
    /// ([`Interpreter::leave_lot_to_delete`]). A
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
            self.leave_lot_to_delete(world, id)?;
        }
        if !world.objects.contains_key(&id) {
            return Ok(());
        }
        self.delete(world, id)
    }

    /// An object of one tile that is not a person leaves the lot on its
    /// way to deletion: its virtual function 0x50 with 1, which for an
    /// ordinary object is 0x433125, the lot's placement
    /// ([`Interpreter::take_off`]). That asks it function 10, moves it and
    /// what it holds off the lot, and asks whatever held it function 9.
    ///
    /// Before the load has worked the lot's rooms out (its clear, load
    /// step 5), the placement cannot find the off-lot place's room, so
    /// the object goes the narrow way of [`Machine::leave_lot`], which
    /// the load was compared through and which fails loudly for anything
    /// but an object already off the lot that holds nothing and runs no
    /// callback for leaving.
    pub(crate) fn leave_lot_to_delete(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        if matches!(
            world.room_at([OFF_LOT, OFF_LOT, 1]),
            Err(RoomError::NoRooms)
        ) {
            return self
                .machine(world, id, 0, false)
                .leave_lot(id)
                .map_err(|trap| Fault {
                    object: id,
                    at: None,
                    trap,
                });
        }
        self.take_off(world, id, true)
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
    ///    and its storage, including its own relationship table (0x43242e),
    ///    is freed. A person's destructor (0x459511) also
    ///    frees its route (+0xaec), its animator (+0xadc, 0x588080), its
    ///    uses (+0xb00), route contexts (+0xaf0), path (+0xae0) and queue
    ///    (+0x820), so what [`World`] keeps for them by the person's ID
    ///    goes: its animator, route, tick state and unknown data words. It
    ///    also leaves the lot's list of people (lot +0x84).
    /// 7. The registry walks its remaining registered objects (+0x30,
    ///    links at object +0xe0) and erases the first row keyed by the
    ///    deleted object ID from each object's relationship table (+0xe4,
    ///    call at 0x441223 to 0x464fa2). This includes the object tables
    ///    of people. The other rows keep their order (0x59b0bd).
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
    /// The sweep uses lot object IDs. Neighborhood relationships use
    /// neighbor IDs and stay in their records. Remove Object Instance
    /// saves a person's data to its record when it queues the person
    /// (`crate::people::save_to_record`). Other objects' frames keep
    /// references to the deleted ID. The room a person was last seen in
    /// keeps its mark that a person is in it until the next person to
    /// leave it clears the mark.
    ///
    /// Not modeled, being presentation: the signals to the interface
    /// (0x87, 0x88), the object's redraw (0x434101), the registry's
    /// interface hooks (0x4481eb, which changes nothing for a person or
    /// any object but an indicator or a star marker), the interface object
    /// of GUID 0x3EEC206C (registry +0x80), and the release of its indicator.
    ///
    /// Before people let go of it, the object gets its deletion mark
    /// (object flag 0x40, 0x441039; [`Interpreter::deleting`]), which it
    /// keeps until it is freed: a reset of it meanwhile, such as one a
    /// person's reset makes as it ends its use of the object, is the
    /// reset of an object being deleted. A group's lead marks every tile
    /// before any is deleted (0x440fbc).
    pub fn delete(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let Some(marks) = self.deleting else {
            let marks = RefCell::new(BTreeSet::new());
            let outer = Interpreter {
                deleting: Some(&marks),
                ..*self
            };
            return outer.delete(world, id);
        };
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
        marks
            .borrow_mut()
            .extend(self.parts(world, id).map_err(fail)?);
        let mut tile = Some(id);
        while let Some(this) = tile {
            tile = world.next_parts.get(&this).copied();
            if world.objects.contains_key(&this) {
                self.delete_one(world, this)?;
            }
        }
        Ok(())
    }

    /// Whether object `id`'s deletion has begun (object flag 0x40,
    /// [`Interpreter::deleting`]).
    pub(crate) fn being_deleted(&self, id: u16) -> bool {
        self.deleting
            .is_some_and(|marks| marks.borrow().contains(&id))
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
        // Native `0x440fff` never refuses deletion when another person holds a
        // route context targeting `id`: `0x441096` (`drop_references` ->
        // `0x45e791`) resets any person using `id` or displaying its icon
        // (clearing its route contexts at `0x45e8aa`), and any surviving route
        // context targeting `id` transitions cleanly through `LOST` /
        // `LOST_UNDER_WAY` (`0x45ae8d`) when `Go To Routing Slot` next ticks
        // and observes `frame.stack_object` absent from `world.objects`.
        if world.event_owner == id {
            world.event_owner = 0;
        }
        if selected {
            self.select(world, None)?;
        }
        if let Some(coop) = self.coop {
            coop.borrow_mut().remove_object(id);
        }
        if let Some(marks) = self.deleting {
            marks.borrow_mut().insert(id);
        }
        self.drop_references(world, id)?;
        // Record target loss on any surviving route context pointing at `id` so
        // immediate object-ID reuse before `Go To Routing Slot` next ticks
        // cannot redirect the route onto an unrelated newly-created object.
        for route in world.routing.routes.values_mut() {
            for context in &mut route.contexts {
                if context.target == id {
                    context.target = DELETED_ROUTE_TARGET;
                }
            }
        }
        if person {
            self.reset_person_to_delete(world, id)?;
        } else {
            self.reset(world, id, true)?;
        }
        // 0x4481eb: release any indicator or pooled star marker sitting in a
        // person's head slot (slot 2) before `0x4410e7` deletes held children,
        // so deleting a Sim (such as a famous visitor or date partner) never
        // destroys the lot's plumb bob (`world.indicators[0]`), date marker
        // (`world.indicators[1]`), or Superstar star marker pool.
        if person
            && world.room_at([OFF_LOT, OFF_LOT, 1]).is_ok()
            && let Some(held) = world
                .objects
                .get(&id)
                .and_then(|o| o.slots.get(2))
                .and_then(|s| u16::try_from(s.object).ok())
                .filter(|&held| held != 0)
            && world.objects.get(&held).is_some_and(|o| {
                world.indicators.contains(&held)
                    || crate::load::INDICATORS.contains(&o.guid)
                    || o.guid == crate::placement::STAR_MARKER
            })
        {
            self.take_off(world, held, false)?;
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
        for indicator in &mut world.indicators {
            if *indicator == id {
                *indicator = 0;
            }
        }
        if let Some(marks) = self.deleting {
            marks.borrow_mut().remove(&id);
        }
        if let Some(deleted) = self.deleted {
            deleted.borrow_mut().push(id);
        }
        world.frame_storage.remove(&id);
        if world.dirty_frames.contains_key(&id) {
            world.dirty_frames.as_mut().remove(&id);
        }
        world.scratch_ptrs.clear_id(id);
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
            world.person_outfits.remove(&id);
            world.base_layers.remove(&id);
            world.translated_bones.remove(&id);
            world.held_poses.remove(&id);
            world.stops_asked.remove(&id);
            world.routing.routes.remove(&id);
            world.person_ticks.remove(&id);
            world
                .routing
                .unknown_words
                .retain(|&(other, _)| other != id);
        }
        // 0x441223 passes the sign-extended lot ID to 0x464fa2.
        let target = i32::from(id as i16);
        for &other in &world.order {
            let object = world.objects.get_mut(&other).ok_or_else(|| {
                fail(Trap::Invalid(format!(
                    "relationship cleanup (0x44120f) found missing registered object {other}"
                )))
            })?;
            // 0x464fab to 0x464fbd reads each row through its pointer, so
            // an empty row before the match would fault there.
            let mut found = None;
            for (place, row) in object.relationships.iter().enumerate() {
                if row.present == 0 {
                    return Err(fail(Trap::Invalid(format!(
                        "object {other}'s relationship row {place} is empty, which the \
                         cleanup (0x464fb1) would read through"
                    ))));
                }
                if row.target == target {
                    found = Some(place);
                    break;
                }
            }
            if let Some(place) = found {
                object.relationships.remove(place);
            }
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
    /// asked function 10 again; any other object leaves as on its way to
    /// deletion ([`Interpreter::leave_lot_to_delete`]). Any other is
    /// rebuilt ([`Interpreter::rebuild`]): taken off the lot, its storage
    /// reset, its Init and a first turn run, and put back where it was.
    ///
    /// Either way the scheduler's countdown for the object is dropped, its
    /// sounds are stopped (0x4320b4, [`crate::sounds`]), and the object
    /// gives up the ownership of interface events. Being deleted (object
    /// flag 0x40: `deleting`, or the object's deletion has begun,
    /// [`Interpreter::deleting`]) only spares the rebuild (0x432063): the
    /// deletion's reset stops the object's sounds too, and its destructor
    /// then once more.
    ///
    /// A Reset entry whose tree is missing runs nothing but still counts
    /// as a Reset tree.
    pub fn reset(&self, world: &mut World, id: u16, deleting: bool) -> Result<(), Fault> {
        let deleting = deleting || self.being_deleted(id);
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
            self.leave_lot_to_delete(world, id)?;
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

    /// Play-mode fallback teardown when `remove_now` faults during
    /// `end_of_update`: unlinks `id` (plus any remaining parts of its
    /// multi-tile group and non-indicator held children) cleanly without
    /// running object callbacks so the lot never halts and leaves zero
    /// dangling references.
    pub(crate) fn force_remove_object(&self, world: &mut World, id: u16) -> Vec<u16> {
        let mut to_remove = Vec::new();
        let mut seen = BTreeSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(id);
        while let Some(cur) = queue.pop_front() {
            if !seen.insert(cur) {
                continue;
            }
            to_remove.push(cur);
            if !world.objects.contains_key(&cur) {
                continue;
            }
            // Expand multipart group (walking parent -> lead, next_parts, and
            // any surviving objects whose parent points to `lead` or `cur`).
            if let Some(parent_raw) = world
                .objects
                .get(&cur)
                .and_then(|o| o.multipart.as_ref())
                .map(|m| m.parent)
            {
                let lead = u16::try_from(parent_raw)
                    .ok()
                    .filter(|&p| p != 0 && world.objects.contains_key(&p))
                    .unwrap_or(cur);
                if !seen.contains(&lead) {
                    queue.push_back(lead);
                }
                let mut walk = Some(lead);
                let mut group_seen = BTreeSet::new();
                while let Some(part) = walk {
                    if !group_seen.insert(part) {
                        break;
                    }
                    if world.objects.contains_key(&part) && !seen.contains(&part) {
                        queue.push_back(part);
                    }
                    walk = world.next_parts.get(&part).copied();
                }
                let lead_i16 = lead as i16;
                let cur_i16 = cur as i16;
                let stragglers: Vec<u16> = world
                    .objects
                    .iter()
                    .filter_map(|(&oid, o)| {
                        o.multipart
                            .as_ref()
                            .filter(|m| m.parent == lead_i16 || m.parent == cur_i16)
                            .map(|_| oid)
                    })
                    .collect();
                for oid in stragglers {
                    if !seen.contains(&oid) {
                        queue.push_back(oid);
                    }
                }
            }
            // Expand held children (slots[1..]), detaching indicators and
            // pooled star markers off-lot instead of deleting them.
            let held_slots: Vec<(usize, u16)> = world
                .objects
                .get(&cur)
                .map(|o| {
                    o.slots
                        .iter()
                        .enumerate()
                        .skip(1)
                        .filter_map(|(idx, s)| {
                            u16::try_from(s.object)
                                .ok()
                                .filter(|&h| h != 0 && world.objects.contains_key(&h))
                                .map(|h| (idx, h))
                        })
                        .collect()
                })
                .unwrap_or_default();
            for (slot_idx, held) in held_slots {
                let is_protected_marker = held != id
                    && world.objects.get(&held).is_some_and(|o| {
                        world.indicators.contains(&held)
                            || crate::load::INDICATORS.contains(&o.guid)
                            || o.guid == crate::placement::STAR_MARKER
                    });
                if is_protected_marker {
                    let _ = crate::placement::unlink_off_lot(world, held);
                    if let Some(parent_obj) = world.objects.get_mut(&cur)
                        && let Some(slot) = parent_obj.slots.get_mut(slot_idx)
                        && slot.object == held as i16
                    {
                        slot.object = 0;
                    }
                    if let Some(marker) = world.objects.get_mut(&held) {
                        marker.position = [OFF_LOT, OFF_LOT, 1];
                        if marker.data.len() > 26 {
                            marker.data[2] = 0;
                            marker.data[3] = -1;
                            marker.data[14] = -1;
                            marker.data[26] = 0;
                        }
                        if let Some(slot0) = marker.slots.first_mut() {
                            slot0.object = 0;
                        }
                    }
                } else if !seen.contains(&held) {
                    queue.push_back(held);
                }
            }
        }

        let mut people_to_recover = BTreeSet::new();
        for &victim in &to_remove {
            let victim_i16 = victim as i16;
            let (old_tile, is_person, was_selected, is_portal) = match world.objects.get(&victim) {
                Some(obj) => (
                    World::tile(obj.position),
                    obj.person.is_some(),
                    obj.selected,
                    self.is_portal_object(world, victim).unwrap_or(false),
                ),
                None => ([0, 0, 1], false, false, false),
            };
            if world.event_owner == victim {
                world.event_owner = 0;
            }
            if was_selected {
                if let Some(obj) = world.objects.get_mut(&victim) {
                    obj.selected = false;
                }
                if world.globals.get(3).copied() == Some(victim_i16) {
                    world.globals[3] = 0;
                }
            }
            if let Some(coop) = self.coop {
                coop.borrow_mut().remove_object(victim);
            }
            if world.objects.contains_key(&victim) {
                crate::people::clear_person_uses_and_counts(world, victim);
                let unlinked = crate::placement::unlink_off_lot(world, victim);
                if is_portal {
                    let _ = self.clear_portal_if_unoccupied(world, victim, old_tile);
                }
                if unlinked.is_err() {
                    let needle = victim.to_le_bytes();
                    for array_id in crate::placement::LEVEL_GRIDS {
                        if let Some(array) = world.arrays.get_mut(&array_id) {
                            for cell in array.data.as_chunks_mut::<2>().0 {
                                if *cell == needle {
                                    *cell = [0, 0];
                                }
                            }
                        }
                    }
                }
            }
            crate::placement::unlink_tile_force(world, victim);
            self.sound(SoundEvent::Stop { object: victim_i16 });
            world.dialogs.records.retain(|r| r.caller != victim);
            for indicator in &mut world.indicators {
                if *indicator == victim {
                    *indicator = 0;
                }
            }
            // Scrub route contexts referencing `victim`.
            for route in world.routing.routes.values_mut() {
                for context in &mut route.contexts {
                    if context.target == victim {
                        context.target = DELETED_ROUTE_TARGET;
                    }
                    if is_person && context.objects[1] == victim {
                        context.byte_84 = true;
                        context.objects[1] = 0;
                        context.action = 0;
                    }
                }
            }
            // Scrub references in all surviving objects.
            let target_i32 = i32::from(victim_i16);
            for (&other_id, obj) in &mut world.objects {
                if other_id == victim {
                    continue;
                }
                for slot in &mut obj.slots {
                    if slot.object == victim_i16 {
                        slot.object = 0;
                    }
                }
                if obj.data.get(2).copied() == Some(victim_i16) {
                    obj.data[2] = 0;
                    if let Some(slot_word) = obj.data.get_mut(3) {
                        *slot_word = -1;
                    }
                }
                if obj.data.get(26).copied() == Some(victim_i16) {
                    obj.data[26] = 0;
                    if let Some(slot_word) = obj.data.get_mut(14) {
                        *slot_word = -1;
                    }
                }
                if let Some(pos) = obj
                    .relationships
                    .iter()
                    .position(|r| r.present != 0 && r.target == target_i32)
                {
                    obj.relationships.remove(pos);
                }
                if let Some(person) = obj.person.as_mut() {
                    let uses_victim = person.object_uses.iter().any(|u| u.target == victim_i16);
                    let active_hits = person.active.target == victim_i16
                        || person.active.icon == victim_i16
                        || person.active.caller == victim_i16;
                    let stack_hits = obj.stack.iter().any(|f| f.stack_object == victim_i16);
                    if (uses_victim || active_hits || stack_hits) && !seen.contains(&other_id) {
                        people_to_recover.insert(other_id);
                    }
                    if person.active.target == victim_i16 || person.active.caller == victim_i16 {
                        person.active = crate::object_uses::no_action();
                    } else if person.active.icon == victim_i16 {
                        person.active.icon = 0;
                    }
                    person.object_uses.retain(|u| u.target != victim_i16);
                    person.queue.retain(|a| {
                        a.target != victim_i16 && a.icon != victim_i16 && a.caller != victim_i16
                    });
                    if person.last.target == victim_i16 || person.last.caller == victim_i16 {
                        person.last = crate::object_uses::no_action();
                    } else if person.last.icon == victim_i16 {
                        person.last.icon = 0;
                    }
                }
            }
            // Remove `victim` from `world.objects` and all side tables.
            world.objects.remove(&victim);
            if let Some(marks) = self.deleting {
                marks.borrow_mut().remove(&victim);
            }
            if let Some(deleted) = self.deleted {
                deleted.borrow_mut().push(victim);
            }
            world.frame_storage.remove(&victim);
            if world.dirty_frames.contains_key(&victim) {
                world.dirty_frames.as_mut().remove(&victim);
            }
            world.scratch_ptrs.clear_id(victim);
            world.headlines.remove(&victim);
            world
                .routing
                .slot_flags
                .retain(|&(object, _), _| object != victim);
            world
                .routing
                .slots
                .retain(|&(object, _), _| object != victim);
            world.fires.retain(|&other| other != victim);
            world.order.retain(|&other| other != victim);
            world.people.retain(|&other| other != victim);
            world.removals.retain(|&other| other != victim);
            for instances in world.instances.values_mut() {
                instances.retain(|&other| other != victim);
            }
            world.animators.remove(&victim);
            world.person_outfits.remove(&victim);
            world.base_layers.remove(&victim);
            world.translated_bones.remove(&victim);
            world.held_poses.remove(&victim);
            world.stops_asked.remove(&victim);
            world.routing.routes.remove(&victim);
            world.person_ticks.remove(&victim);
            world
                .routing
                .unknown_words
                .retain(|&(other, _)| other != victim);
        }

        for other in people_to_recover {
            if world
                .objects
                .get(&other)
                .is_some_and(|o| o.person.is_some())
                && self.reset_person_after_purge(world, other).is_err()
            {
                if let Some(route) = world.routing.routes.get_mut(&other) {
                    route.contexts.clear();
                }
                world.headlines.remove(&other);
                crate::people::clear_person_uses_and_counts(world, other);
                if let Some(person) = world
                    .objects
                    .get_mut(&other)
                    .and_then(|o| o.person.as_mut())
                {
                    person.active = crate::object_uses::no_action();
                    person.motive_deltas.clear();
                }
                let _ = self.machine(world, other, 0, false).install_main();
            }
        }
        to_remove
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
