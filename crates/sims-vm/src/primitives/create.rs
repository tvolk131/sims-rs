//! Create New Object Instance (opcode 42): make an object through the
//! factory.
//!
//! Ported from the old prototype's creation request
//! (legacy/crates/sims-vm/src/create.rs and
//! legacy/crates/sims-sim/src/lot_creation.rs), recovered from the
//! original's 0x44e0c8.
//!
//! Operands: the GUID (bytes 0..4), where the object goes (byte 4), flags
//! (byte 5) and a local (byte 6). The jump table at 0x44e92e picks where
//! the object goes; every mode then runs one common tail (0x44e4c5) that
//! tests the place, makes the object and puts it there. Three modes are
//! ported:
//!
//! - Mode 4 (0x44e3a2), with no flag or flag 0x1: the new object goes into
//!   the stack object's first container slot. See
//!   `Machine::create_in_stack_object`.
//! - Mode 6 (0x44e4b5), with no flag or flag 0x1: the new object stays off
//!   the lot.
//! - Mode 8 (0x44e430), with no flag, flag 0x1, flag 0x20 or both: the new
//!   object goes where the object the local names stands. See
//!   `Machine::create_at_local`.
//!
//! Flag 0x1 (0x44e4e1), the tail's first test: if an object of the same
//! type already stands on the tile the new object is to go to, or is held
//! (however deep) by one that does, nothing is made and the false branch
//! is taken. See `Machine::same_type_on_tile`.
//!
//! The object is made by [`crate::Interpreter::create`] (0x440b2d), or a
//! person by [`crate::Interpreter::create_person`], so its Init and first
//! turn have run before it is placed. It becomes the stack object and the
//! true branch is taken. Every other mode and flag fails loudly.
//!
//! Mode 8 making a person is what the Goth house's tick 6 does (object
//! 382 makes person 259). The state after it, and after ticks 7 to 13 in
//! which that person runs, equals the capture's (`docs/decisions.md`).
//! **Unverified**: a GUID with no definition takes the false branch here;
//! mode 8 making an object that is not a person, the paths of a person
//! that tick 6 does not take, mode 4 and flag 0x1 are checked only by the
//! synthetic tests. Mode 4 with flag 0x1 is what the fridge's snack tree
//! (tree 0x1019 node 1) runs in play mode after a player's snack order.

use crate::factory::word::{GROUP, TYPE};
use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::{Place, on_lot, standing_on};
use sims_sim::World;
use sims_sim::rooms::NO_ROOM;

/// Flag 0x1: nothing is made where an object of the same type already
/// stands.
const NOT_BESIDE_ITS_TYPE: u8 = 0x1;

/// Flag 0x20: turn the new object to face as the stack object faces.
const FACE_AS_STACK_OBJECT: u8 = 0x20;

/// The person type in definition word 9.
const PERSON: i16 = 2;

/// Data word 29: the room an object is in (object +0x84).
const ROOM: usize = 29;

/// Where mode 6 leaves the new object (0x44e4b5): off the lot, on level 1.
const OFF_LOT: ([i32; 2], i32) = ([-16, -16], 1);

impl Machine<'_, '_> {
    pub(crate) fn create_object(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let guid = u32::from_le_bytes([operands[0], operands[1], operands[2], operands[3]]);
        let (mode, flags) = (operands[4], operands[5]);
        let supported = match mode {
            4 | 6 => flags & !NOT_BESIDE_ITS_TYPE == 0,
            8 => flags & !(NOT_BESIDE_ITS_TYPE | FACE_AS_STACK_OBJECT) == 0,
            _ => false,
        };
        if !supported {
            return Err(Trap::Unsupported(format!(
                "Create New Object Instance mode {mode} with flags {flags:#x}"
            )));
        }
        if self.catalog.object(guid).is_none() {
            return Ok(false);
        }
        match mode {
            4 => self.create_in_stack_object(guid, flags),
            8 => self.create_at_local(guid, flags, operands[6] as i8),
            _ => {
                // Mode 6's tile (-1, -1) is off the lot's arrays, so flag 0x1
                // finds nothing there (0x432500 refuses it).
                let (position, level) = OFF_LOT;
                if flags & NOT_BESIDE_ITS_TYPE != 0
                    && self.same_type_on_tile(guid, position, level)?
                {
                    return Ok(false);
                }
                let created = self.make(guid)?;
                self.frame_mut()?.stack_object = created as i16;
                Ok(true)
            }
        }
    }

    fn make(&mut self, guid: u32) -> Result<u16, Trap> {
        self.interpreter()
            .create(self.world, guid)
            .map_err(|fault| {
                Trap::Unsupported(format!("creating an object of type 0x{guid:08X}: {fault}"))
            })
    }

    /// Makes a person (0x440b2d, [`crate::Interpreter::create_person`])
    /// and gives it its neighborhood record's interests and personality
    /// (0x425930), as 0x44e0c8 does for any new person (0x44e608).
    fn make_person(&mut self, guid: u32) -> Result<u16, Trap> {
        let interpreter = self.interpreter();
        let unsupported =
            |fault| Trap::Unsupported(format!("creating a person of type 0x{guid:08X}: {fault}"));
        let id = interpreter
            .create_person(self.world, guid)
            .map_err(unsupported)?;
        interpreter.attach_traits(self.world, id).map_err(|trap| {
            unsupported(crate::fault::Fault {
                object: id,
                at: None,
                trap,
            })
        })?;
        Ok(id)
    }

    /// Flag 0x1's test (0x44e4e1 to 0x44e51c): whether an object of type
    /// `guid` stands on the tile of `position` at `level`, or is held,
    /// however deep, by one that does. The original walks the tile's
    /// objects and what they hold (0x43f331, 0x43f379; [`standing_on`])
    /// and compares each one's definition (object +0x11c) with the new
    /// object's; a tile off the lot's arrays or off levels 1 and 2 has
    /// none (0x432500).
    fn same_type_on_tile(&self, guid: u32, position: [i32; 2], level: i32) -> Result<bool, Trap> {
        let [a, b, level] = World::tile([position[0], position[1], level]);
        for id in standing_on(self.world, [a as u8, b as u8, level as u8])? {
            let object = self
                .world
                .objects
                .get(&id)
                .ok_or(Trap::native(10, "missing object"))?;
            if object.guid == guid {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether a position is off the lot (0x422632): a tile byte below 1
    /// or at least the lot's size less 1.
    fn off_lot(&self, position: [i32; 2]) -> bool {
        let size = i32::from(self.world.size);
        let inside = |n: i32| {
            let tile = i32::from((n >> 4) as i8);
            tile >= 1 && tile < size - 1
        };
        !(inside(position[0]) && inside(position[1]))
    }

    /// Mode 4 of 0x44e0c8 (0x44e3a2, then the common tail from 0x44e4c5),
    /// in the order the original runs it:
    ///
    /// 1. The stack object (frame +4) is the container; one naming no
    ///    object takes the false branch. Its position, level (+0x110) and
    ///    room (data word 29, +0x84) are kept, and the slot is its first
    ///    container slot (slot 0 as scripts count it).
    /// 2. With flag 0x1, an object of the same type on the container's
    ///    tile takes the false branch ([`Machine::same_type_on_tile`]).
    /// 3. A position off the lot (0x422632) takes the false branch.
    /// 4. Unless the container's room is -5, the room the lot's room map
    ///    gives the container's tile must be that room (0x44e582), or the
    ///    false branch is taken.
    /// 5. The object is made (0x440b2d).
    /// 6. The place test (virtual function 0x54) of the container's slot,
    ///    at the container's position and level
    ///    ([`crate::Interpreter::fits_in`]). If it refuses, the new object
    ///    is removed at once (0x44123f) and the false branch is taken.
    /// 7. The object is put in the slot with the lot told (virtual
    ///    function 0x58).
    ///
    /// A person type, which the original gives its record's traits and
    /// sends through its own place test, and an object whose definition
    /// names a group (word 10), fail loudly before anything is made. So
    /// does a container that the new object's Init or first turn removed,
    /// which the original would go on to read through a stale pointer. A
    /// person as the container is tested against its own container slots
    /// too (0x45962b), so its slot 0 is the one it carries in.
    fn create_in_stack_object(&mut self, guid: u32, flags: u8) -> Result<bool, Trap> {
        let source = self.frame()?.stack_object;
        let Some((container, [p0, p1, level], room)) = u16::try_from(source)
            .ok()
            .filter(|id| *id > 0)
            .and_then(|id| Some((id, self.world.objects.get(&id)?)))
            .map(|(id, object)| (id, object.position, object.data.get(ROOM).copied()))
        else {
            return Ok(false);
        };
        let room = room
            .ok_or_else(|| Trap::Invalid(format!("object {container} has no data word {ROOM}")))?;
        let position = [p0, p1];
        if flags & NOT_BESIDE_ITS_TYPE != 0 && self.same_type_on_tile(guid, position, level)? {
            return Ok(false);
        }
        if self.off_lot(position) {
            return Ok(false);
        }
        if room != NO_ROOM && self.room_at(position, level)? != room {
            return Ok(false);
        }
        let words = self.definition_words(guid)?;
        if words[TYPE] == PERSON {
            return Err(Trap::Unsupported(format!(
                "Create New Object Instance mode 4 making a person (type 0x{guid:08X}) \
                 in object {container}'s slot, which takes the person path at 0x44e60e, is \
                 not ported"
            )));
        }
        if words[GROUP] != 0 {
            return Err(Trap::Unsupported(format!(
                "Create New Object Instance placing an object of type 0x{guid:08X}, \
                 which belongs to a group, is not ported"
            )));
        }
        let created = self.make(guid)?;
        if !self.world.objects.contains_key(&container) {
            return Err(Trap::Unsupported(format!(
                "Create New Object Instance mode 4: object {created}'s Init removed its \
                 container {container}, which the original reads after it is freed"
            )));
        }
        self.place_new(
            created,
            Place {
                on_lot: on_lot(position),
                position,
                container: Some(container),
                slot: 0,
                level,
            },
        )
    }

    /// The room the lot's room map gives the tile of `position` on
    /// `level` (the grid at lot +0x28, read at 0x44e588): -5 for a tile a
    /// diagonal wall splits.
    fn room_at(&self, position: [i32; 2], level: i32) -> Result<i16, Trap> {
        let [a, b, _] = World::tile([position[0], position[1], level]);
        usize::try_from(level - 1)
            .ok()
            .and_then(|l| self.world.rooms.map.get(l))
            .filter(|map| map.len() == 64 * 64)
            .and_then(|map| map.get(a as usize * 64 + b as usize))
            .copied()
            .ok_or_else(|| {
                Trap::Unsupported(format!(
                    "Create New Object Instance reading the room of tile ({a}, {b}) on level \
                     {level}, which has no room map"
                ))
            })
    }

    /// Mode 8 of 0x44e0c8, in the order the original runs it:
    ///
    /// 1. The local must be one the frame has (native error 8). The object
    ///    it names gives the position and level; a local naming no object
    ///    takes the false branch before anything is made.
    /// 2. With flag 0x1, an object of the same type on that tile takes the
    ///    false branch ([`Machine::same_type_on_tile`]).
    /// 3. A position off the lot (0x422632: a tile byte below 1 or at
    ///    least the lot's size less 1) takes the false branch.
    /// 4. The object is made (0x440b2d). A new person is made by its own
    ///    path and then takes its record's interests and personality
    ///    ([`Machine::make_person`]).
    /// 5. With flag 0x20 a new person takes the stack object's data word 1
    ///    whole. Any other new object turns to the stack object's facing
    ///    with its lowest bit cleared: up to four calls of virtual function
    ///    0x4c, one step each. Nothing turns when the stack object names no
    ///    object. Only the case with nothing to turn is ported; a turn
    ///    (0x433f75, which redraws, updates seats and the walls beside)
    ///    fails loudly.
    /// 6. A new person with bit 0 of person word 84 or 68 set, or bit 0 or
    ///    1 of word 74, goes through 0x45810c, which is not ported and
    ///    fails.
    /// 7. The place test (virtual function 0x54, with no container) at the
    ///    exact position. If it refuses, the new object is removed at once
    ///    (0x44123f) and the false branch is taken. Removing a person is
    ///    not ported and fails.
    /// 8. The object is put there with the lot told (virtual function
    ///    0x58).
    ///
    /// Mode 8 keeps the room filter at -5 (0x44e13c), so the room test of
    /// mode 4's step 4 never runs. An object whose definition names a
    /// group (word 10, which sends the original through a group's own
    /// steps) fails loudly before anything is made. Flags 2, 4, 8, 0x10
    /// and 0x40, which this mode can also carry, are refused by the
    /// caller.
    fn create_at_local(&mut self, guid: u32, flags: u8, local: i8) -> Result<bool, Trap> {
        let frame = self.frame()?;
        let local = usize::try_from(local)
            .ok()
            .filter(|&l| l < frame.locals.len())
            .ok_or(Trap::native(
                8,
                "Create New Object Instance names a missing local",
            ))?;
        self.check_local(local)?;
        let reference = self.frame()?.locals[local];
        let Some(reference) = u16::try_from(reference)
            .ok()
            .filter(|id| *id > 0)
            .and_then(|id| self.world.objects.get(&id))
        else {
            return Ok(false);
        };
        let [p0, p1, level] = reference.position;
        let position = [p0, p1];
        if flags & NOT_BESIDE_ITS_TYPE != 0 && self.same_type_on_tile(guid, position, level)? {
            return Ok(false);
        }
        if self.off_lot(position) {
            return Ok(false);
        }
        let words = self.definition_words(guid)?;
        let person = words[TYPE] == PERSON;
        if words[GROUP] != 0 {
            return Err(Trap::Unsupported(format!(
                "Create New Object Instance placing an object of type 0x{guid:08X}, \
                 which belongs to a group, is not ported"
            )));
        }
        let created = if person {
            self.make_person(guid)?
        } else {
            self.make(guid)?
        };
        let place = Place {
            on_lot: on_lot(position),
            position,
            container: None,
            slot: -1,
            level,
        };
        if flags & FACE_AS_STACK_OBJECT != 0 {
            let source = self.frame()?.stack_object;
            if let Some(source) = u16::try_from(source)
                .ok()
                .filter(|id| *id > 0)
                .and_then(|id| self.world.objects.get(&id))
            {
                if person {
                    // A person takes the stack object's data word 1 whole
                    // and does not turn (0x44e684).
                    let facing = source.data[1];
                    self.world
                        .objects
                        .get_mut(&created)
                        .ok_or(Trap::native(10, "missing object"))?
                        .data[1] = facing;
                    return self.place_new(created, place);
                }
                let wanted = source.data[1] & 6;
                let facing = self.world.objects[&created].data[1];
                if facing != wanted {
                    return Err(Trap::Unsupported(format!(
                        "Create New Object Instance turning object {created} from facing \
                         {facing} to {wanted}: turning (0x433f75) is not ported"
                    )));
                }
            }
        }
        self.place_new(created, place)
    }

    /// The tail's last steps (from 0x44e6b6) for the object `created` just
    /// made: a person with bit 0 of person word 84 or 68 set, or bit 0 or
    /// 1 of word 74, goes through 0x45810c, which is not ported and fails.
    /// Then the place test of `place` (virtual function 0x54: on the
    /// ground, or in the container's slot), and the put (virtual function
    /// 0x58). A refusal removes the new object at once (0x44123f) and
    /// takes the false branch; otherwise the new object becomes the stack
    /// object.
    fn place_new(&mut self, created: u16, place: Place) -> Result<bool, Trap> {
        if let Some(person) = self
            .world
            .objects
            .get(&created)
            .and_then(|o| o.person.as_ref())
        {
            let word = |index: usize| person.data.get(index).copied().unwrap_or(0);
            if word(84) & 1 != 0 || word(68) & 1 != 0 || word(74) & 3 != 0 {
                return Err(Trap::Unsupported(format!(
                    "Create New Object Instance placing person {created}, whose person word 84, \
                     68 or 74 is set: 0x45810c is not ported"
                )));
            }
        }
        let interpreter = self.interpreter();
        let unsupported = |fault| {
            Trap::Unsupported(format!(
                "Create New Object Instance placing object {created}: {fault}"
            ))
        };
        let fits = match place.container {
            Some(container) => interpreter.fits_in(
                self.world,
                created,
                container,
                place.slot,
                place.position,
                place.level,
            ),
            None => interpreter.fits(self.world, created, place.position, place.level),
        };
        if !fits.map_err(unsupported)? {
            interpreter
                .remove_now(self.world, created)
                .map_err(unsupported)?;
            return Ok(false);
        }
        interpreter
            .put(self.world, created, place, true)
            .map_err(unsupported)?;
        self.frame_mut()?.stack_object = created as i16;
        Ok(true)
    }
}
