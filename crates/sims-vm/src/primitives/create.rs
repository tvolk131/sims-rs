//! Create New Object Instance (opcode 42): make an object through the
//! factory.
//!
//! Ported from the old prototype's creation request
//! (legacy/crates/sims-vm/src/create.rs and
//! legacy/crates/sims-sim/src/lot_creation.rs), recovered from the
//! original's 0x44e0c8.
//!
//! Operands: the GUID (bytes 0..4), where the object goes (byte 4), flags
//! (byte 5) and a local (byte 6). Two cases are ported:
//!
//! - Mode 6, no flag: the new object stays off the lot.
//! - Mode 8, with no flag or only flag 0x20: the new object goes where the
//!   object the local names stands (the jump table at 0x44e92e sends
//!   mode 8 to 0x44e430). See `Machine::create_at_local`.
//!
//! The object is made by [`crate::Interpreter::create`] (0x440b2d), so its
//! Init and first turn have run before it is placed. It becomes the stack
//! object and the true branch is taken. Every other mode and flag fails
//! loudly.
//!
//! **Unverified**: a GUID with no definition takes the false branch here.
//! Mode 8 is first reached after the five ticks the capture holds, so
//! nothing of it has been compared with the original.

use crate::factory::word::{GROUP, TYPE};
use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::{Place, on_lot};

/// Flag 0x20: turn the new object to face as the stack object faces.
const FACE_AS_STACK_OBJECT: u8 = 0x20;

/// The person type in definition word 9.
const PERSON: i16 = 2;

impl Machine<'_, '_> {
    pub(crate) fn create_object(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let guid = u32::from_le_bytes([operands[0], operands[1], operands[2], operands[3]]);
        let (mode, flags) = (operands[4], operands[5]);
        let supported = match mode {
            6 => flags == 0,
            8 => flags & !FACE_AS_STACK_OBJECT == 0,
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
        if mode == 8 {
            return self.create_at_local(guid, flags, operands[6] as i8);
        }
        let created = self.make(guid)?;
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

    /// Mode 8 of 0x44e0c8, in the order the original runs it:
    ///
    /// 1. The local must be one the frame has (native error 8). The object
    ///    it names gives the position and level; a local naming no object
    ///    takes the false branch before anything is made.
    /// 2. A position off the lot (0x422632: a tile byte below 1 or at
    ///    least the lot's size less 1) takes the false branch.
    /// 3. The object is made (0x440b2d).
    /// 4. With flag 0x20 the new object turns to the stack object's facing
    ///    with its lowest bit cleared: up to four calls of virtual function
    ///    0x4c, one step each. Nothing turns when the stack object names no
    ///    object. Only the case with nothing to turn is ported; a turn
    ///    (0x433f75, which redraws, updates seats and the walls beside)
    ///    fails loudly.
    /// 5. The place test (virtual function 0x54, with no container) at the
    ///    exact position. If it refuses, the new object is removed at once
    ///    (0x44123f) and the false branch is taken.
    /// 6. The object is put there with the lot told (virtual function
    ///    0x58).
    ///
    /// A new person (which copies the facing and has steps of its own),
    /// and an object whose definition names a group (word 10, which sends
    /// the original through a group's own steps), fail loudly before
    /// anything is made. Flags 1, 2, 4, 8, 0x10 and 0x40, which this mode
    /// can also carry, are refused by the caller.
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
        let size = i32::from(self.world.size);
        let inside = |n: i32| {
            let tile = i32::from((n >> 4) as i8);
            tile >= 1 && tile < size - 1
        };
        if !(inside(p0) && inside(p1)) {
            return Ok(false);
        }
        let words = self.definition_words(guid)?;
        if words[TYPE] == PERSON {
            return Err(Trap::Unsupported(
                "Create New Object Instance placing a new person is not ported".into(),
            ));
        }
        if words[GROUP] != 0 {
            return Err(Trap::Unsupported(format!(
                "Create New Object Instance placing an object of type 0x{guid:08X}, \
                 which belongs to a group, is not ported"
            )));
        }
        let created = self.make(guid)?;
        if flags & FACE_AS_STACK_OBJECT != 0 {
            let source = self.frame()?.stack_object;
            if let Some(source) = u16::try_from(source)
                .ok()
                .filter(|id| *id > 0)
                .and_then(|id| self.world.objects.get(&id))
            {
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
        let interpreter = self.interpreter();
        let unsupported = |fault| {
            Trap::Unsupported(format!(
                "Create New Object Instance placing object {created}: {fault}"
            ))
        };
        if !interpreter
            .fits(self.world, created, position, level)
            .map_err(unsupported)?
        {
            interpreter
                .remove_now(self.world, created)
                .map_err(unsupported)?;
            return Ok(false);
        }
        interpreter
            .put(
                self.world,
                created,
                Place {
                    on_lot: on_lot(position),
                    position,
                    container: None,
                    slot: -1,
                    level,
                },
                true,
            )
            .map_err(unsupported)?;
        self.frame_mut()?.stack_object = created as i16;
        Ok(true)
    }
}
