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
//! tests the place, makes the object and puts it there. All 15 jump-table
//! modes (0..=14) and the out-of-range fallthrough (`> 14`) are ported:
//!
//! - Mode 0 (0x44e1e6): the tile ahead of the running object, one tile in
//!   its facing, on its level.
//! - Mode 1 (0x44e215) and mode 2 (0x44e22f): into the running object's
//!   container slot 0 (both leave `[ebp-0x2c]` at 0), with room filter -5.
//! - Mode 3 (0x44e34c): the tile ahead of the stack object, one tile in
//!   its facing, on its level, filtered to the stack object's room.
//! - Mode 4 (0x44e3a2): into the stack object's container slot 0, filtered
//!   to the stack object's room.
//! - Mode 5 (0x44e3e9): where the running object stands, on its level.
//! - Mode 6 (0x44e4b5): off the lot (`[-16, -16]`, level 1), skipping the
//!   off-lot, room, place test and put steps (`[ebp-0x15] = 0`).
//! - Mode 7 (0x44e3fd): where the object named by parameter 0 stands, on
//!   its level (native error 8 if the frame has no parameters; false if
//!   parameter 0 names no object).
//! - Mode 8 (0x44e430): where the object named by local `operands[6]`
//!   stands, on its level (native error 8 if the local is out of range;
//!   false if it names no object).
//! - Mode 9 (0x44e157): the tile in front of me, turned by local
//!   `operands[6]`, unless that tile is closed off towards me.
//! - Modes 10 (0x44e24d) and 11 (0x44e26e): into the running object's
//!   container slot 2 or 3, with room filter -5.
//! - Modes 12 (0x44e28f), 13 (0x44e2ce) and 14 (0x44e30d): into the stack
//!   object's container slot 2, 3 or 4, filtered to the stack object's room.
//! - Modes outside 0..=14 (0x44e147, unsigned compare > 14): fall through
//!   to the common tail at `[0, 0]`, which fails the off-lot test and
//!   returns false.
//!
//! Flags (byte 5):
//!
//! - `0x01` (0x44e4e1): fail before creating if an object of the same type
//!   already stands on (or is held on) the destination tile.
//! - `0x02` (0x44e87c): pass `self.me` as the first data word of the new
//!   object's bottom frame and copy the caller's `stack_object` into its
//!   `stack_object`. Combined with `0x10`, raises native error `0x36`
//!   (0x44e88c) after creation and placement.
//! - `0x04` (0x44e0e6, 0x44e7dd): take the type from the neighbor record
//!   named by the stack object instead of the GUID operand; after placement,
//!   a new visitor person runs its `"init visitor"` tree synchronously.
//! - `0x08` (0x44e51e): fail before creating if any object stands on the
//!   destination tile.
//! - `0x10` (0x44e8e1): without `0x02`, write the running object's temp 0
//!   into the first data word of the new object's bottom frame.
//! - `0x20` (0x44e640): copy the stack object's facing onto a new person,
//!   or turn any other new object up to four steps to match `facing & 6`.
//! - `0x40` (0x44e564): clear the room filter (`-5`) and wall-check facing
//!   (`-1`), skipping both checks.
//! - `0x80`: unused by 0x44e0c8.
//!
//! The person test (0x44e6b6 to 0x44e727): when the new object is a
//! person (definition type 2) with bit 0 of person word 84 (+0x634) or 68
//! (+0x614) set, or bit 0 or 1 of word 74 (+0x620), the original calls
//! 0x45810c with 1, then sets bit 0x20000000 of the screen's flags
//! (`[[0x6f2a58]+0xb0]+0xc`, 0x4ceceb). The port removes the type from
//! [`sims_sim::World::portraits`] (`Machine::make_person`); the screen
//! flags are presentation.
//!
//! The object is made by [`crate::Interpreter::create`] (0x440b2d), or a
//! person by [`crate::Interpreter::create_person`], so its Init and first
//! turn have run before it is placed. It becomes the stack object and the
//! true branch is taken.

use crate::factory::TURN_BUDGET;
use crate::factory::word::{GROUP, TYPE};
use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::{Cursor, Place, closed_directions, head, on_lot};
use crate::visits::function;
use sims_sim::World;
use sims_sim::rooms::NO_ROOM;
use std::collections::BTreeSet;

/// Flag 0x1: nothing is made where an object of the same type already
/// stands (0x44e4e1).
const NOT_BESIDE_ITS_TYPE: u8 = 0x1;

/// Flag 0x2: write `self.me` to the new object's bottom frame data word 0
/// and copy the caller's `stack_object` into its `stack_object` (0x44e87c).
/// Combined with `0x10`, raises native error `0x36` (0x44e88c).
const PASS_ME_AND_STACK: u8 = 0x2;

/// Flag 0x4: the type is the neighbor record's that the stack object
/// names, not the GUID operand's (0x44e0e6); and a new person that is a
/// visitor runs its "init visitor" tree (0x44e7dd to 0x44e877).
const FROM_RECORD: u8 = 0x4;

/// Flag 0x8: nothing is made on a tile where any object stands
/// (0x44e51e).
const ON_A_FREE_TILE: u8 = 0x8;

/// Flag 0x10: without flag 0x2, the running object's temp 0 goes into
/// the new object's bottom frame (0x44e8e1); with 0x2, native error 0x36
/// (0x44e88c).
const TEMP_0_TO_NEW: u8 = 0x10;

/// Flag 0x20: turn the new object to face as the stack object faces
/// (0x44e640).
const FACE_AS_STACK_OBJECT: u8 = 0x20;

/// Flag 0x40: skip the room filter and the closed-direction wall check
/// (0x44e564 to 0x44e577).
const IGNORE_ROOM_AND_WALL: u8 = 0x40;

/// Scheduler flag 4 (`0x439a4e`, `0x46d5da`): set when a thread error is
/// caught (`0x474aae`).
const CHECK_ERROR: u16 = 4;

/// Person data word 32 (+0x5cc): nonzero for a visitor.
const VISITOR: usize = 32;

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

/// The destination chosen by the mode jump table (0x44e11d to 0x44e4c5)
/// before the common tail runs.
struct Destination {
    position: [i32; 2],
    level: i32,
    container: Option<u16>,
    slot: i32,
    place_on_lot: bool,
    room: i16,
    facing: Option<i32>,
}

/// Snapshot of the stack object fields read by stack-object create modes.
struct StackObject {
    id: u16,
    position: [i32; 3],
    facing: i16,
    room: i16,
}

impl Machine<'_, '_> {
    pub(crate) fn create_object(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let guid = u32::from_le_bytes([operands[0], operands[1], operands[2], operands[3]]);
        let (mode, flags) = (operands[4], operands[5]);
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
        if crate::interpreter::object_definition(self.catalog, self.world, guid).is_none() {
            return Ok(false);
        }
        let Some(destination) = self.create_destination(mode, operands[6] as i8)? else {
            return Ok(false);
        };
        self.create_common(guid, mode, flags, destination)
    }

    /// Resolves the destination parameters for `mode` (0x44e11d to 0x44e4c5).
    /// Returns `Ok(None)` when a mode's source object does not exist, taking
    /// the false branch before anything is made.
    fn create_destination(&mut self, mode: u8, local: i8) -> Result<Option<Destination>, Trap> {
        let me_id = self.me;
        let me_level = self.me()?.position[2];
        let mut dst = Destination {
            position: [0, 0],
            level: me_level,
            container: None,
            slot: 0,
            place_on_lot: true,
            room: NO_ROOM,
            facing: None,
        };
        match mode {
            // Mode 0 (0x44e1e6): one tile ahead of me in my facing.
            0 => {
                let me = self.me()?;
                let [p0, p1, _] = me.position;
                let [step0, step1] = crate::placement::step(me.data[1]);
                dst.position = [p0.wrapping_add(step0 * 16), p1.wrapping_add(step1 * 16)];
            }
            // Mode 1 (0x44e215), mode 2 (0x44e22f), mode 10 (0x44e24d),
            // mode 11 (0x44e26e): in one of my container slots (0, 0, 2, 3),
            // keeping the room filter at -5.
            1 | 2 | 10 | 11 => {
                let [p0, p1, _] = self.me()?.position;
                dst.position = [p0, p1];
                dst.container = Some(me_id);
                dst.slot = match mode {
                    10 => 2,
                    11 => 3,
                    _ => 0,
                };
            }
            // Mode 3 (0x44e34c): one tile ahead of the stack object, in its room.
            3 => {
                let Some(src) = self.stack_object_info()? else {
                    return Ok(None);
                };
                let [step0, step1] = crate::placement::step(src.facing);
                dst.position = [
                    src.position[0].wrapping_add(step0 * 16),
                    src.position[1].wrapping_add(step1 * 16),
                ];
                dst.level = src.position[2];
                dst.room = src.room;
            }
            // Mode 4 (0x44e3a2), mode 12 (0x44e28f), mode 13 (0x44e2ce),
            // mode 14 (0x44e30d): in container slot 0, 2, 3 or 4 of the
            // stack object, filtered to its room.
            4 | 12 | 13 | 14 => {
                let Some(src) = self.stack_object_info()? else {
                    return Ok(None);
                };
                dst.position = [src.position[0], src.position[1]];
                dst.level = src.position[2];
                dst.container = Some(src.id);
                dst.slot = match mode {
                    12 => 2,
                    13 => 3,
                    14 => 4,
                    _ => 0,
                };
                dst.room = src.room;
            }
            // Mode 5 (0x44e3e9): where I stand, on my level.
            5 => {
                let [p0, p1, _] = self.me()?.position;
                dst.position = [p0, p1];
            }
            // Mode 6 (0x44e4b5): off the lot, on level 1, skipping placement.
            6 => {
                dst.position = OFF_LOT.0;
                dst.level = OFF_LOT.1;
                dst.place_on_lot = false;
            }
            // Mode 7 (0x44e3fd): where the object named by parameter 0 stands.
            7 => {
                let param = *self.frame()?.parameters.first().ok_or(Trap::native(
                    8,
                    "Create New Object Instance names a missing parameter",
                ))?;
                let Some(reference) = self.object_by_i16(param) else {
                    return Ok(None);
                };
                let [p0, p1, level] = reference.position;
                dst.position = [p0, p1];
                dst.level = level;
            }
            // Mode 8 (0x44e430): where the object named by local `local` stands.
            8 => {
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
                let Some(reference) = self.object_by_i16(reference) else {
                    return Ok(None);
                };
                let [p0, p1, level] = reference.position;
                dst.position = [p0, p1];
                dst.level = level;
            }
            // Mode 9 (0x44e157): the tile in front of me, turned by `local`,
            // testing whether that tile is closed off towards me.
            9 => {
                let turn = i32::from(self.read(LOCAL_SCOPE, i16::from(local))?);
                let me = self.me()?;
                let [p0, p1, _] = me.position;
                let direction = i32::from(me.data[1]) + turn;
                let (step0, step1) = match (direction % 8) & 7 {
                    0 => (0, -1),
                    2 => (1, 0),
                    4 => (0, 1),
                    6 => (-1, 0),
                    _ => (0, 0),
                };
                dst.position = [p0.wrapping_add(step0 * 16), p1.wrapping_add(step1 * 16)];
                // 0x44e1dc -> 0x44e5ae: `-1` is the wall-check skip sentinel
                // (e.g. facing 0 + turn -5 => (-1) % 8 == -1).
                let facing = (direction + 4) % 8;
                dst.facing = (facing != -1).then_some(facing);
            }
            // Modes outside 0..=14 (0x44e147 cmp eax, 0xe; ja 0x44e4c5): fall
            // through with the initial `[0, 0]` destination.
            _ => {}
        }
        Ok(Some(dst))
    }

    /// Looks up a positive object ID in `self.world.objects` (0x40743b).
    fn object_by_i16(&self, id: i16) -> Option<&sims_sim::Object> {
        u16::try_from(id)
            .ok()
            .filter(|id| *id > 0)
            .and_then(|id| self.world.objects.get(&id))
    }

    /// Reads the current frame's stack object fields, or `Ok(None)` when
    /// the stack object names no object.
    fn stack_object_info(&self) -> Result<Option<StackObject>, Trap> {
        let source = self.frame()?.stack_object;
        let Some(id) = u16::try_from(source).ok().filter(|id| *id > 0) else {
            return Ok(None);
        };
        let Some(object) = self.world.objects.get(&id) else {
            return Ok(None);
        };
        let room = object
            .data
            .get(ROOM)
            .copied()
            .ok_or_else(|| Trap::Invalid(format!("object {id} has no data word {ROOM}")))?;
        Ok(Some(StackObject {
            id,
            position: object.position,
            facing: object.data[1],
            room,
        }))
    }

    /// The common tail of 0x44e0c8 (0x44e4c5 to 0x44e91c), shared by every
    /// mode.
    fn create_common(
        &mut self,
        guid: u32,
        mode: u8,
        flags: u8,
        mut dst: Destination,
    ) -> Result<bool, Trap> {
        if flags & NOT_BESIDE_ITS_TYPE != 0
            && self.same_type_on_tile(guid, dst.position, dst.level)?
        {
            return Ok(false);
        }
        if flags & ON_A_FREE_TILE != 0 && self.anything_on_tile(dst.position, dst.level)? {
            return Ok(false);
        }
        if dst.place_on_lot && self.off_lot(dst.position) {
            return Ok(false);
        }
        if flags & IGNORE_ROOM_AND_WALL != 0 {
            dst.facing = None;
            dst.room = NO_ROOM;
        }
        if dst.place_on_lot
            && dst.room != NO_ROOM
            && self.room_at(dst.position, dst.level)? != dst.room
        {
            return Ok(false);
        }
        if let Some(facing) = dst.facing {
            let tile = [
                (dst.position[0] >> 4) as i8,
                (dst.position[1] >> 4) as i8,
                dst.level as i8,
            ];
            if closed_directions(self.world, tile, facing)? & 1 != 0 {
                return Ok(false);
            }
        }
        let words = self.definition_words(guid)?;
        let person = words[TYPE] == PERSON && words[GROUP] == 0;
        let created = if person {
            let Some(id) = self.make_person(guid)? else {
                // 0x44e5d8: native person factory returned 0 (0x440dd5).
                return Ok(false);
            };
            id
        } else {
            let Some(id) = self.make(guid)? else {
                // 0x44e5d8: native factory returned 0 (0x440b90 / 0x440c9b / 0x4409f2).
                return Ok(false);
            };
            id
        };
        if flags & FACE_AS_STACK_OBJECT != 0 {
            self.face_as_stack_object(created, person)?;
        }
        if !dst.place_on_lot {
            return self.finish_new(created, flags);
        }
        let dynamic = self.dynamic_group(created)?;
        let position = if let Some(container) = dst.container {
            let Some(host) = self.world.objects.get(&container) else {
                return Err(Trap::Unsupported(format!(
                    "Create New Object Instance mode {mode}: object {created}'s Init removed its \
                     container {container}, which the original reads after it is freed"
                )));
            };
            // 0x43268c: when a container is given, the place takes the
            // container's current X/Y coordinates after Init and turning,
            // keeping the level captured before creation (0x4326b8).
            [host.position[0], host.position[1]]
        } else if dynamic {
            // 0x44e770: the place is the middle of its tile.
            dst.position.map(|p| (p & !7) | 8)
        } else {
            dst.position
        };
        let place = Place {
            on_lot: on_lot(position),
            position,
            container: dst.container,
            slot: if dst.container.is_some() {
                dst.slot
            } else {
                -1
            },
            level: dst.level,
        };
        self.place_new(created, place, flags, dynamic)
    }

    /// The tail's end (0x44e7dd to 0x44e91c) for the object `created`,
    /// made and placed:
    ///
    /// 1. With flag 0x4, a new person that is a visitor (person word 32,
    ///    +0x5cc, nonzero) runs the tree its file names "init visitor"
    ///    (0x4021f2), synchronously on its own thread with itself as stack
    ///    object and its own temps as arguments.
    /// 2. With flag 0x2 (0x44e87c): if flag 0x10 is also set, native error
    ///    0x36 is raised (0x44e88c). Otherwise, if the new object's stack
    ///    is non-empty, `self.me` is written into the first data word of
    ///    its bottom frame (0x44e8d1) and the caller's `stack_object` is
    ///    copied into that frame's `stack_object` (0x44e8db).
    /// 3. Otherwise, with flag 0x10 (0x44e8e1): the running object's temp 0
    ///    is written into the first data word of the new object's bottom
    ///    frame (0x44e90c).
    /// 4. The new object becomes the caller's `stack_object` and the true
    ///    branch is taken (0x44e90f).
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
                    && let Err(fault) = self.interpreter().call_as(
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
                {
                    // 0x474aae catches any native SimAntics error (including
                    // Sleep yield 0x44f and explicit error branch 0xfd / 0x44c)
                    // on the visitor's thread, records the error word and calls
                    // 0x439a4e (setting scheduler flag CHECK_ERROR = 4), and
                    // returns to 0x44e87c. Without a serialized error word on
                    // `Object` we trap loudly, except when flags 0x02|0x10 are
                    // both set: native immediately raises 0x36 on the caller
                    // at 0x44e89c. Mark the visitor with CHECK_ERROR and drop
                    // its live frames (keeping frame-storage metadata in sync
                    // so play-mode recovery can reset the visitor cleanly) so
                    // the failed "init visitor" frame can never resume.
                    let flag_conflict = flags & (PASS_ME_AND_STACK | TEMP_0_TO_NEW)
                        == (PASS_ME_AND_STACK | TEMP_0_TO_NEW);
                    let native_error =
                        !fault.trap.unported() && !matches!(fault.trap, Trap::Invalid(_));
                    if !(flag_conflict && native_error) {
                        return Err(fault.unsupported_in(format!(
                            "Create New Object Instance: the \"init visitor\" tree \
                             {tree:#06x} of person {created}"
                        )));
                    }
                    if let Some(visitor) = self.world.objects.get_mut(&created) {
                        visitor.set_flags(CHECK_ERROR, true);
                        visitor.stack.clear();
                    }
                    if let Some(storage) = self.world.frame_storage.get_mut(&created) {
                        storage.offsets.clear();
                        storage.unknown.clear();
                        storage.restarts = storage.restarts.wrapping_add(1);
                    }
                }
            }
        }
        if flags & PASS_ME_AND_STACK != 0 {
            if flags & TEMP_0_TO_NEW != 0 {
                return Err(Trap::native(
                    0x36,
                    "Create New Object Instance cannot combine flags 0x02 and 0x10",
                ));
            }
            let me_id = self.me as i16;
            let caller_stack = self.frame()?.stack_object;
            let object = self
                .world
                .objects
                .get_mut(&created)
                .ok_or(Trap::native(10, "missing object"))?;
            if let Some(bottom) = object.stack.first_mut() {
                let word = match bottom.parameters.first_mut() {
                    Some(word) => word,
                    None => bottom.locals.first_mut().ok_or_else(|| {
                        Trap::Unsupported(format!(
                            "Create New Object Instance flag 0x02 writing past the bottom \
                             frame of object {created}, which has no argument or local \
                             (0x44e8d1)"
                        ))
                    })?,
                };
                *word = me_id;
                bottom.stack_object = caller_stack;
            }
        } else if flags & TEMP_0_TO_NEW != 0 {
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

    fn make(&mut self, guid: u32) -> Result<Option<u16>, Trap> {
        self.interpreter()
            .try_create(self.world, guid)
            .map_err(|fault: crate::fault::Fault| {
                fault.unsupported_in(format!("creating an object of type 0x{guid:08X}"))
            })
    }

    /// Makes a person (0x440b2d, [`crate::Interpreter::create_person`])
    /// and gives it its neighborhood record's interests and personality
    /// (0x425930), as 0x44e0c8 does for any new person (0x44e608).
    ///
    /// Then, as the original does for a new person in every mode (0x44e6b6
    /// to 0x44e70b, after flag 0x20's turn, which touches none of the
    /// words read): when person word 84 bit 0 (+0x634), word 68 bit 0
    /// (+0x614), or word 74 bit 1 or bit 0 (+0x620) is set, the
    /// interface's portrait entry of its type is reset (0x45810c,
    /// 0x4eb0eb, 0x4e895e: each image let go and marked to build), so the
    /// next dialog's portrait of the type builds it again
    /// ([`sims_sim::World::portraits`], 0x4e92a8). The redraw it asks for
    /// (0x4ceceb) is presentation.
    fn make_person(&mut self, guid: u32) -> Result<Option<u16>, Trap> {
        let interpreter = self.interpreter();
        let unsupported = |fault: crate::fault::Fault| {
            fault.unsupported_in(format!("creating a person of type 0x{guid:08X}"))
        };
        let Some(id) = interpreter
            .try_create_person(self.world, guid)
            .map_err(unsupported)?
        else {
            return Ok(None);
        };
        interpreter.attach_traits(self.world, id).map_err(|trap| {
            unsupported(crate::fault::Fault {
                object: id,
                at: None,
                trap,
            })
        })?;
        let word = |index: usize| {
            self.world
                .objects
                .get(&id)
                .and_then(|object| object.person.as_ref())
                .and_then(|person| person.data.get(index).copied())
                .unwrap_or(0)
        };
        if word(84) & 1 != 0 || word(68) & 1 != 0 || word(74) & 3 != 0 {
            self.world.portraits.remove(&guid);
            self.world.dirty_portraits.insert(guid);
        }
        Ok(Some(id))
    }

    /// Flag 0x1's test (0x44e4e1 to 0x44e51c): whether an object of type
    /// `guid` stands on the tile of `position` at `level`, or is held,
    /// however deep, by one that does. The original walks the tile's
    /// objects and what they hold (0x43f331, 0x43f379; [`Cursor`]),
    /// comparing each one's definition (object +0x11c) with the new
    /// object's and returning on the first match (0x44e508 to 0x44e50e);
    /// a tile off the lot's arrays or off levels 1 and 2 has none (0x432500).
    fn same_type_on_tile(&self, guid: u32, position: [i32; 2], level: i32) -> Result<bool, Trap> {
        let mut cursor =
            Cursor::on_tile(self.world, World::tile([position[0], position[1], level]));
        let mut seen = BTreeSet::new();
        while let Some(id) = cursor.current() {
            if !seen.insert(id) {
                return Err(Trap::Invalid("the objects on a tile link in a loop".into()));
            }
            let object = self
                .world
                .objects
                .get(&id)
                .ok_or(Trap::native(10, "missing object"))?;
            if object.guid == guid {
                return Ok(true);
            }
            cursor.advance(self.world)?;
        }
        Ok(false)
    }

    /// Flag 0x8's test (0x44e51e to 0x44e541): whether any object stands
    /// on the tile of `position` at `level`. The original starts the walk
    /// flag 0x1's test makes ([`Cursor`], 0x43f331) and reads only
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
        let Some(source) = self.object_by_i16(source) else {
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
                interpreter.turn_group(self.world, created, 1).map_err(
                    |fault: crate::fault::Fault| {
                        fault.unsupported_in(format!(
                            "Create New Object Instance turning group {created}"
                        ))
                    },
                )?;
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
            interpreter.turn_one(self.world, created, 1).map_err(
                |fault: crate::fault::Fault| {
                    fault.unsupported_in(format!(
                        "Create New Object Instance turning object {created}"
                    ))
                },
            )?;
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
        let unsupported = |fault: crate::fault::Fault| {
            fault.unsupported_in(format!(
                "Create New Object Instance placing object {created}"
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
