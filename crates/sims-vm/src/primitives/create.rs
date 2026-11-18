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
//! tests the place, makes the object and puts it there. Seven modes are
//! ported:
//!
//! - Mode 0 (0x44e1e6), with no flag or any of 0x1, 0x4, 0x8, 0x10 and
//!   0x20: the new object goes on the tile ahead of the running object,
//!   one tile in its facing, on its level. Nothing about the tile is
//!   tested before the tail. See `Machine::create_ahead`.
//! - Mode 2 (0x44e22f), with no flag or any of 0x1, 0x4 and 0x10: the new
//!   object goes into the running object's first container slot. See
//!   `Machine::create_in_me`.
//! - Mode 4 (0x44e3a2), with no flag or any of 0x1, 0x4 and 0x10: the new
//!   object goes into the stack object's first container slot. See
//!   `Machine::create_in_stack_object`.
//! - Mode 5 (0x44e3e9), with no flag or any of 0x1, 0x4, 0x8, 0x10 and 0x20: the
//!   new object goes where the running object stands, on its level. As
//!   mode 8 from there; see `Machine::create_at`.
//! - Mode 6 (0x44e4b5), with no flag or any of 0x1, 0x4, 0x10 and 0x20: the
//!   new object stays off the lot. It is not placed (the tail skips the
//!   place test and the put, 0x44e72c). See `Machine::create_off_lot`.
//! - Mode 8 (0x44e430), with no flag or any of 0x1, 0x4, 0x8, 0x10 and 0x20: the new
//!   object goes where the object the local names stands. See
//!   `Machine::create_at_local`.
//! - Mode 9 (0x44e157), with the same flags: the new object goes on the
//!   tile in front of me, turned by a local, unless that tile is closed
//!   off towards me. See `Machine::create_in_front`.
//!
//! Flag 0x4 does two things. At the start (0x44e0e6) the type is not the
//! GUID operand's but that of the neighbor record the stack object names
//! (0x425afe: record slot `id - 1`, 0x4256f4, and the definition linked to
//! it, record +8); a stack object that names no record takes the false
//! branch before anything is made. At the end of the tail (0x44e7dd to
//! 0x44e877) a new person that is a visitor (person word 32, +0x5cc)
//! runs the tree its file names "init visitor" at once
//! (`Machine::finish_new`; 0x4021f2, 0x474afb). For any other object
//! that does nothing.
//!
//! Flag 0x10 without flag 0x2 (0x44e8e1 to 0x44e90c), after "init
//! visitor": if the new object's thread has a frame, the running object's
//! temp 0 (+0x3a) is written to the first word of its bottom frame's data
//! (0x474fa8 with 0, frame +0x10 by 0x47518b): its first argument, or its
//! first local if it takes no argument. A bottom frame with neither fails
//! loudly. With flag 0x2 it is native error 0x36 (0x44e88c); flag 0x2 is
//! not ported. See `Machine::finish_new`.
//!
//! Flag 0x1 (0x44e4e1), the tail's first test: if an object of the same
//! type already stands on the tile the new object is to go to, or is held
//! (however deep) by one that does, nothing is made and the false branch
//! is taken. See `Machine::same_type_on_tile`.
//!
//! Flag 0x8 (0x44e51e), tested next in modes 0, 5, 8 and 9: if any object
//! stands on that tile, or is held by one that does, nothing is made and
//! the false branch is taken. See `Machine::anything_on_tile`. Mode 0 with
//! flags 0x9 is what the Goth house's tick 36311 runs (object 16, tree
//! 0x0183), which is **unverified**: the run has left track 2's recording
//! by then.
//!
//! The person test (0x44e6b6 to 0x44e727) is left out. When the new
//! object is a person (definition type 2) with bit 0 of person word 84
//! (+0x634) or 68 (+0x614) set, or bit 0 or 1 of word 74 (+0x620), the
//! original calls 0x45810c with 1, then sets bit 0x20000000 of the
//! screen's flags (`[[0x6f2a58]+0xb0]+0xc`, 0x4ceceb). 0x45810c looks
//! its type's handle up (0x4eb0eb) in the table at 0x6f2698 (0x44-byte
//! entries, which only the interface code reads) and, when it is there,
//! resets seven of the entry's members (0x4e895e, 0x5a5f79) and sets the
//! same screen flag. Nothing the sim reads is written, so the port does
//! nothing.
//! **Unverified** that the table is only what is drawn: no capture covers
//! a person made with these words set (the Goth house's tick 30320,
//! object 243 making person 36).
//!
//! The object is made by [`crate::Interpreter::create`] (0x440b2d), or a
//! person by [`crate::Interpreter::create_person`], so its Init and first
//! turn have run before it is placed. It becomes the stack object and the
//! true branch is taken. Every other mode and flag fails loudly.
//!
//! Mode 8 making a person is what the Goth house's tick 6 does (object
//! 382 makes person 259). The state after it, and after ticks 7 to 13 in
//! which that person runs, equals the capture's (`docs/decisions/`).
//! Mode 6 with flag 0x4 is what its tick 3603 does (object 354, tree
//! 0x1022, makes two neighbors' people off the lot), which is
//! **unverified**: there is no capture past tick 1000.
//! **Unverified**: a GUID with no definition takes the false branch here;
//! mode 8 making an object that is not a person, the paths of a person
//! that tick 6 does not take, mode 4 and flag 0x1 are checked only by the
//! synthetic tests. Mode 4 with flag 0x1 is what the fridge's snack tree
//! (tree 0x1019 node 1) runs in play mode after a player's snack order.

use crate::factory::TURN_BUDGET;
use crate::factory::word::{GROUP, TYPE};
use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::{Place, closed_directions, head, on_lot, standing_on};
use crate::visits::function;
use sims_sim::World;
use sims_sim::rooms::NO_ROOM;

/// Flag 0x1: nothing is made where an object of the same type already
/// stands.
const NOT_BESIDE_ITS_TYPE: u8 = 0x1;

/// Flag 0x8: nothing is made on a tile where any object stands
/// (0x44e51e).
const ON_A_FREE_TILE: u8 = 0x8;

/// Flag 0x20: turn the new object to face as the stack object faces.
const FACE_AS_STACK_OBJECT: u8 = 0x20;

/// Flag 0x4: the type is the neighbor record's that the stack object
/// names, not the GUID operand's (0x44e0e6); and a new person that is a
/// visitor runs its "init visitor" tree (0x44e7dd to 0x44e877).
const FROM_RECORD: u8 = 0x4;

/// Person data word 32 (+0x5cc): nonzero for a visitor.
const VISITOR: usize = 32;

/// Flag 0x10: without flag 0x2, the running object's temp 0 goes into
/// the new object's bottom frame (0x44e8e1); with it, native error 0x36
/// (0x44e88c), which is not reached here as 0x2 is not ported.
const TEMP_0_TO_NEW: u8 = 0x10;

/// The tree a new visitor runs with flag 0x4 (the string at 0x6c4120).
const INIT_VISITOR_TREE: &str = "init visitor";

/// The person type in definition word 9.
const PERSON: i16 = 2;

/// Data word 29: the room an object is in (object +0x84).
const ROOM: usize = 29;

/// The scope mode 9 reads its turn from (0x44e164): a local.
const LOCAL_SCOPE: u8 = 0x19;

/// Where mode 6 leaves the new object (0x44e4b5): off the lot, on level 1.
const OFF_LOT: ([i32; 2], i32) = ([-16, -16], 1);

impl Machine<'_, '_> {
    pub(crate) fn create_object(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let guid = u32::from_le_bytes([operands[0], operands[1], operands[2], operands[3]]);
        let (mode, flags) = (operands[4], operands[5]);
        let supported = match mode {
            2 | 4 => flags & !(NOT_BESIDE_ITS_TYPE | FROM_RECORD | TEMP_0_TO_NEW) == 0,
            6 => {
                flags & !(NOT_BESIDE_ITS_TYPE | FACE_AS_STACK_OBJECT | FROM_RECORD | TEMP_0_TO_NEW)
                    == 0
            }
            0 | 5 | 8 | 9 => {
                flags
                    & !(NOT_BESIDE_ITS_TYPE
                        | ON_A_FREE_TILE
                        | FACE_AS_STACK_OBJECT
                        | FROM_RECORD
                        | TEMP_0_TO_NEW)
                    == 0
            }
            _ => false,
        };
        if !supported {
            return Err(Trap::Unsupported(format!(
                "Create New Object Instance mode {mode} with flags {flags:#x}"
            )));
        }
        let guid = if flags & FROM_RECORD != 0 {
            // 0x44e0e6: the type of the neighbor record the stack object
            // names (0x425afe, record +8); the GUID operand is not read.
            let record = self.frame()?.stack_object;
            match self.world.neighbors.get(record) {
                Some(record) => record.guid,
                None => return Ok(false),
            }
        } else {
            guid
        };
        if self.catalog.object(guid).is_none() {
            return Ok(false);
        }
        match mode {
            0 => self.create_ahead(guid, flags),
            2 => self.create_in_me(guid, flags),
            4 => self.create_in_stack_object(guid, flags),
            5 => {
                // 0x44e3e9: where I stand, on my level (+0x110, 0x44e132).
                let [p0, p1, level] = self.me()?.position;
                self.create_at(guid, flags, [p0, p1], level, None)
            }
            8 => self.create_at_local(guid, flags, operands[6] as i8),
            9 => self.create_in_front(guid, flags, operands[6] as i8),
            _ => self.create_off_lot(guid, flags),
        }
    }

    /// Mode 6 of 0x44e0c8 (0x44e4b5, then the common tail), in the order
    /// the original runs it:
    ///
    /// 1. With flag 0x1, an object of the same type on the tile takes the
    ///    false branch; mode 6's tile (-1, -1) is off the lot's arrays, so
    ///    there is none (0x432500 refuses it).
    /// 2. Mode 6 clears the byte that asks for the off-lot and room tests
    ///    (`[ebp-0x15]`), so neither runs.
    /// 3. The object is made (0x440b2d). A new person is made by its own
    ///    path and then takes its record's interests and personality
    ///    ([`Machine::make_person`], 0x44e608).
    /// 4. Flag 0x20 turns it ([`Machine::face_as_stack_object`]).
    /// 5. A new person's person test (see the module's note) touches only the
    ///    screen, and is left out.
    /// 6. The place test and the put are skipped (0x44e72c): the object
    ///    stays off the lot where it was made.
    /// 7. Flag 0x4 runs a visitor's "init visitor" tree
    ///    ([`Machine::finish_new`]), and the new object becomes the stack
    ///    object.
    fn create_off_lot(&mut self, guid: u32, flags: u8) -> Result<bool, Trap> {
        let (position, level) = OFF_LOT;
        if flags & NOT_BESIDE_ITS_TYPE != 0 && self.same_type_on_tile(guid, position, level)? {
            return Ok(false);
        }
        let person = self.definition_words(guid)?[TYPE] == PERSON;
        let created = if person {
            self.make_person(guid)?
        } else {
            self.make(guid)?
        };
        if flags & FACE_AS_STACK_OBJECT != 0 {
            self.face_as_stack_object(created, person)?;
        }
        self.finish_new(created, flags)
    }

    /// The tail's end (0x44e7dd to 0x44e91c) for the object `created`,
    /// made and placed: with flag 0x4, a new person that is a visitor
    /// (person word 32, +0x5cc, nonzero) runs the tree its file names
    /// "init visitor" (0x4021f2: that file, then its semi-global and the
    /// global file), at once and to its end on its own thread (0x474afb,
    /// 0x4749b5), with itself as the stack object and, as the arguments
    /// pointer is null, its own temps as the arguments (0x47484a). What it
    /// returns is not read, and no tree of that name runs nothing. Any
    /// other object does nothing here. Then flag 0x10 writes the running
    /// object's temp 0 to the first data word of the new object's bottom
    /// frame (0x44e8e1 to 0x44e90c; module doc). Then the new object
    /// becomes the stack object and the true branch is taken (0x44e90f).
    fn finish_new(&mut self, created: u16, flags: u8) -> Result<bool, Trap> {
        if flags & FROM_RECORD != 0 {
            let object = self
                .world
                .objects
                .get(&created)
                .ok_or(Trap::native(10, "missing object"))?;
            let visitor = object
                .person
                .as_ref()
                .is_some_and(|p| p.data.get(VISITOR).copied().unwrap_or(0) != 0);
            if visitor {
                let owner = self.definition(object.guid)?.resource;
                let temps = object.temps.clone();
                if let Some(tree) = self
                    .program
                    .named_tree(owner, INIT_VISITOR_TREE)
                    .map_err(Trap::Unsupported)?
                {
                    self.interpreter()
                        .call_as(
                            self.world,
                            created,
                            created,
                            tree,
                            created as i16,
                            &temps,
                            temps.len(),
                            TURN_BUDGET,
                            self.checking,
                        )
                        .map_err(|fault| {
                            Trap::Unsupported(format!(
                                "Create New Object Instance: the \"init visitor\" tree \
                                 {tree:#06x} of person {created}: {fault}"
                            ))
                        })?;
                }
            }
        }
        if flags & TEMP_0_TO_NEW != 0 {
            let value = *self
                .me()?
                .temps
                .first()
                .ok_or(Trap::Invalid("an object with no temps".into()))?;
            let object = self
                .world
                .objects
                .get_mut(&created)
                .ok_or(Trap::native(10, "missing object"))?;
            if let Some(bottom) = object.stack.first_mut() {
                // The frame's data is its arguments, then its locals
                // (0x47518b, 0x47518f).
                let word = match bottom.parameters.first_mut() {
                    Some(word) => word,
                    None => bottom.locals.first_mut().ok_or_else(|| {
                        Trap::Unsupported(format!(
                            "Create New Object Instance flag 0x10 writing past the bottom \
                             frame of object {created}, which has no argument or local \
                             (0x44e90c)"
                        ))
                    })?,
                };
                *word = value;
            }
        }
        self.frame_mut()?.stack_object = created as i16;
        Ok(true)
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

    /// Flag 0x8's test (0x44e51e to 0x44e541): whether any object stands
    /// on the tile of `position` at `level`. The original starts the walk
    /// flag 0x1's test makes ([`standing_on`], 0x43f331) and reads only
    /// its first object, the tile's first ([`head`]), so nothing past it is
    /// looked at.
    fn anything_on_tile(&self, position: [i32; 2], level: i32) -> Result<bool, Trap> {
        Ok(head(self.world, World::tile([position[0], position[1], level])).is_some())
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
        self.create_in(guid, flags, 4, container, [p0, p1], level, room)
    }

    /// Mode 2 of 0x44e0c8 (0x44e22f, then the common tail): as mode 4
    /// ([`Machine::create_in_stack_object`]) with the running object as
    /// the container, at its position and on its level (+0x110,
    /// 0x44e132), in its first container slot. Mode 2 keeps the room
    /// filter at -5 (0x44e13c), so the room test of mode 4's step 4 never
    /// runs.
    fn create_in_me(&mut self, guid: u32, flags: u8) -> Result<bool, Trap> {
        let me = self.me;
        let [p0, p1, level] = self.me()?.position;
        self.create_in(guid, flags, 2, me, [p0, p1], level, NO_ROOM)
    }

    /// Steps 2 to 7 of [`Machine::create_in_stack_object`], for mode
    /// `mode` with `container`, its position, level and room filter.
    #[allow(clippy::too_many_arguments)]
    fn create_in(
        &mut self,
        guid: u32,
        flags: u8,
        mode: u8,
        container: u16,
        position: [i32; 2],
        level: i32,
        room: i16,
    ) -> Result<bool, Trap> {
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
                "Create New Object Instance mode {mode} making a person (type 0x{guid:08X}) \
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
                "Create New Object Instance mode {mode}: object {created}'s Init removed its \
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
            flags,
            false,
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
    ///    object. A group turns by 0x422937, a one-tile object by 0x433f75
    ///    ([`crate::Interpreter::turn_one`]).
    /// 6. A new person's person test (see the module's note) touches only the
    ///    screen, and is left out.
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
    /// steps) fails loudly before anything is made. Flags 0x2 and 0x40,
    /// which this mode can also carry, are refused by the caller; flag 0x4
    /// only chose the type, and flag 0x8 is tested after flag 0x1
    /// ([`Machine::create_at`]).
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
        self.create_at(guid, flags, [p0, p1], level, None)
    }

    /// Mode 9 of 0x44e0c8 (0x44e157, then the common tail): the tile in
    /// front of me, turned by a local.
    ///
    /// 1. The turn is local `local` (0x453141 with scope 0x19, the byte
    ///    sign-extended); the direction is my data word 1 (+0x4c) plus the
    ///    turn, taken mod 8 (a signed remainder masked with 7).
    /// 2. Directions 0, 2, 4 and 6 move the place one tile (0x422022, 16
    ///    units) from where I stand, on my level (+0x110): to my position
    ///    word 1 less 16, word 0 plus 16, word 1 plus 16 and word 0 less
    ///    16. An odd direction does not move it.
    /// 3. The facing the tail tests (`[ebp-0x1c]`) is my facing plus the
    ///    turn plus 4, as a signed remainder of 8, so it may be negative:
    ///    the way back towards me. The new object does not take it.
    /// 4. The tail is mode 5's ([`Machine::create_at`]), with one more
    ///    test after the place is found on the lot (0x44e5ae): if the
    ///    tile is closed off in that facing (bit 0 of
    ///    [`crate::placement::closed_directions`], 0x4359ea), nothing is
    ///    made and the false branch is taken.
    fn create_in_front(&mut self, guid: u32, flags: u8, local: i8) -> Result<bool, Trap> {
        let turn = i32::from(self.read(LOCAL_SCOPE, i16::from(local))?);
        let me = self.me()?;
        let [p0, p1, level] = me.position;
        let direction = i32::from(me.data[1]) + turn;
        let (step0, step1) = match (direction % 8) & 7 {
            0 => (0, -1),
            2 => (1, 0),
            4 => (0, 1),
            6 => (-1, 0),
            _ => (0, 0),
        };
        let position = [p0.wrapping_add(step0 * 16), p1.wrapping_add(step1 * 16)];
        let facing = (direction + 4) % 8;
        self.create_at(guid, flags, position, level, Some(facing))
    }

    /// Mode 0 of 0x44e0c8 (0x44e1e6, then the common tail): the tile in
    /// front of me. My position moves one tile (16 units) in my facing
    /// (data word 1, +0x4c, masked with 7; 0x42228a and 0x4222bc, whose
    /// steps are [`crate::placement::forward`]'s), on my level (+0x110).
    /// No facing or room is tested, so the tail is mode 5's
    /// ([`Machine::create_at`]).
    fn create_ahead(&mut self, guid: u32, flags: u8) -> Result<bool, Trap> {
        let me = self.me()?;
        let [p0, p1, level] = me.position;
        let [step0, step1] = crate::placement::step(me.data[1]);
        let position = [p0.wrapping_add(step0 * 16), p1.wrapping_add(step1 * 16)];
        self.create_at(guid, flags, position, level, None)
    }

    /// Modes 0, 5, 8 and 9 of 0x44e0c8 from the position and level the
    /// mode gives on: steps 2 to 8 of [`Machine::create_at_local`], with
    /// flag 0x8's test after flag 0x1's ([`Machine::anything_on_tile`]).
    fn create_at(
        &mut self,
        guid: u32,
        flags: u8,
        position: [i32; 2],
        level: i32,
        facing: Option<i32>,
    ) -> Result<bool, Trap> {
        if flags & NOT_BESIDE_ITS_TYPE != 0 && self.same_type_on_tile(guid, position, level)? {
            return Ok(false);
        }
        if flags & ON_A_FREE_TILE != 0 && self.anything_on_tile(position, level)? {
            return Ok(false);
        }
        if self.off_lot(position) {
            return Ok(false);
        }
        if let Some(facing) = facing {
            let tile = [
                (position[0] >> 4) as i8,
                (position[1] >> 4) as i8,
                level as i8,
            ];
            if closed_directions(self.world, tile, facing)? & 1 != 0 {
                return Ok(false);
            }
        }
        let words = self.definition_words(guid)?;
        let person = words[TYPE] == PERSON;
        let created = if person {
            self.make_person(guid)?
        } else {
            self.make(guid)?
        };
        if flags & FACE_AS_STACK_OBJECT != 0 {
            self.face_as_stack_object(created, person)?;
        }
        let dynamic = self.dynamic_group(created)?;
        let position = if dynamic {
            // 0x44e770: the place is the middle of its tile.
            position.map(|p| (p & !7) | 8)
        } else {
            position
        };
        let place = Place {
            on_lot: on_lot(position),
            position,
            container: None,
            slot: -1,
            level,
        };
        self.place_new(created, place, flags, dynamic)
    }

    /// The tail's group test (0x44e736 to 0x44e76e, again at 0x44e7c3):
    /// the new object's own definition word 10 (+0x118 +0x14) is not 0,
    /// it is a tile of a group (the cast to 0x6c0ef8), and its type has
    /// callback function 8 (0x42344e).
    fn dynamic_group(&self, created: u16) -> Result<bool, Trap> {
        let interpreter = self.interpreter();
        Ok(interpreter.words(self.world, created)?[GROUP] != 0
            && self
                .world
                .objects
                .get(&created)
                .ok_or(Trap::native(10, "missing object"))?
                .multipart
                .is_some()
            && interpreter.has_function(self.world, created, function::DYNAMIC_GROUP))
    }

    /// Flag 0x20 (0x44e640 to 0x44e6b4), for the object `created` just
    /// made: when the frame's stack object names an object, a new person
    /// takes its data word 1 whole and does not turn (0x44e684); any other
    /// new object turns to its facing with the lowest bit cleared, up to
    /// four calls of virtual function 0x4c, one step each, while its
    /// facing differs. Nothing turns when the stack object names no
    /// object. A group turns ([`crate::Interpreter::turn_group`],
    /// 0x422937), a one-tile object by its own turn
    /// ([`crate::Interpreter::turn_one`], 0x433f75).
    fn face_as_stack_object(&mut self, created: u16, person: bool) -> Result<(), Trap> {
        let source = self.frame()?.stack_object;
        let Some(source) = u16::try_from(source)
            .ok()
            .filter(|id| *id > 0)
            .and_then(|id| self.world.objects.get(&id))
        else {
            return Ok(());
        };
        let facing = source.data[1];
        let object = self
            .world
            .objects
            .get_mut(&created)
            .ok_or(Trap::native(10, "missing object"))?;
        if person {
            object.data[1] = facing;
            return Ok(());
        }
        let wanted = facing & 6;
        if object.multipart.is_some() {
            // A group turns by its own virtual function 0x4c (0x422937).
            let interpreter = self.interpreter();
            for _ in 0..4 {
                let now = self
                    .world
                    .objects
                    .get(&created)
                    .ok_or(Trap::native(10, "missing object"))?
                    .data[1];
                if now == wanted {
                    break;
                }
                interpreter
                    .turn_group(self.world, created, 1)
                    .map_err(|fault| {
                        Trap::Unsupported(format!(
                            "Create New Object Instance turning group {created}: {fault}"
                        ))
                    })?;
            }
            return Ok(());
        }
        // A one-tile object turns by its own virtual function 0x4c
        // (0x433f75); see docs/decisions/20261115-snap-offset-order.md.
        let interpreter = self.interpreter();
        for _ in 0..4 {
            let now = self
                .world
                .objects
                .get(&created)
                .ok_or(Trap::native(10, "missing object"))?
                .data[1];
            if now == wanted {
                break;
            }
            interpreter
                .turn_one(self.world, created, 1)
                .map_err(|fault| {
                    Trap::Unsupported(format!(
                        "Create New Object Instance turning object {created}: {fault}"
                    ))
                })?;
        }
        Ok(())
    }

    /// The tail's last steps (from 0x44e6b6) for the object `created` just
    /// made: the place test of `place` (virtual function 0x54: on the ground, or in
    /// the container's slot), and the put (virtual function 0x58). A
    /// refusal removes the new object at once (0x44123f) and takes the
    /// false branch; otherwise the tail ends ([`Machine::finish_new`]).
    fn place_new(
        &mut self,
        created: u16,
        place: Place,
        flags: u8,
        dynamic: bool,
    ) -> Result<bool, Trap> {
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
        if dynamic {
            interpreter
                .join_neighbours(self.world, created)
                .map_err(unsupported)?;
        }
        self.finish_new(created, flags)
    }
}
