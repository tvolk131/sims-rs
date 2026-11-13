//! The operand model: scopes, bounds checks, reads and writes.
//!
//! An operand is a scope (which object or table) and an index. Resolving
//! it gives a [`Cell`] and whether the original hands back a writable word
//! pointer for it. Literal-like scopes (literals, slots, tuning constants,
//! callback tests) resolve to values with no pointer, so they read but never
//! take a write. Ported from the old prototype's `resolve_memory`,
//! `cell_value` and `write_cell` (legacy/crates/sims-vm/src/memory.rs:137-746),
//! keeping its native error codes. Scopes over state the simulation does
//! not hold yet fail with [`Trap::UnsupportedScope`].

use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::program::{DEFINITION_WORDS, Tree};
use sims_core::{Ext, F32};
use sims_formats::saved_objects::Frame;
use sims_sim::{Object, World};

/// Data word 25: how many more turns the object stays locked out. While it
/// is nonzero a Sleep counts down by itself instead of leaving the count
/// to the scheduler.
pub(crate) const LOCKOUT: usize = 25;

/// Scope names, by scope number.
const SCOPE_NAMES: [&str; 39] = [
    "my attributes",
    "stack object's attributes",
    "target object's attributes (invalid)",
    "my object data",
    "stack object's object data",
    "target object's object data (invalid)",
    "global",
    "literal",
    "temps",
    "parameters",
    "stack object ID",
    "temp indexed by temp",
    "check tree advertisement range",
    "stack object's temps",
    "my motives",
    "stack object's motives",
    "stack object's slot",
    "stack object's motive indexed by temp",
    "my person data",
    "stack object's person data",
    "my slot",
    "stack object's definition",
    "stack object's attribute indexed by parameter",
    "room indexed by temp 0",
    "neighbor in stack object",
    "local",
    "tuning constant",
    "stack object's dynamic sprite flag indexed by temp",
    "check tree advertisement personality",
    "check tree advertisement minimum",
    "my person data indexed by temp",
    "stack object's person data indexed by temp",
    "neighbor's person data",
    "job data indexed by temps 0 and 1",
    "neighborhood data",
    "stack object's callback",
    "my type attribute",
    "stack object's type attribute",
    "scope 38",
];

/// A scope's name, for messages.
pub fn scope_name(scope: u8) -> &'static str {
    SCOPE_NAMES
        .get(usize::from(scope))
        .copied()
        .unwrap_or("invalid scope")
}

/// A resolved operand. Object cells hold the object's ID, frame cells
/// address the top frame of the running object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cell {
    /// A value with no word pointer behind it.
    Literal(i16),
    Attribute(u16, usize),
    Data(u16, usize),
    Person(u16, usize),
    Temp(u16, usize),
    DynamicSprite(u16, usize),
    Global(usize),
    /// A type attribute: the record's position in
    /// [`World::type_attributes`], then the word.
    TypeAttribute(usize, usize),
    Parameter(usize),
    Local(usize),
    StackObject,
    /// A motive: a 32-bit float, not an integer word.
    Motive(u16, usize),
}

fn bounded(index: i16, count: usize, code: u16) -> Result<usize, Trap> {
    usize::try_from(index)
        .ok()
        .filter(|&i| i < count)
        .ok_or(Trap::native(code, "memory index out of bounds"))
}

fn unsupported(scope: u8) -> Trap {
    Trap::UnsupportedScope { scope }
}

impl Machine<'_, '_> {
    /// An object by a script's (signed) ID, or `code` if it doesn't exist.
    fn object(&self, id: i16, code: u16) -> Result<(u16, &Object), Trap> {
        u16::try_from(id)
            .ok()
            .and_then(|id| self.world.objects.get_key_value(&id))
            .map(|(&id, object)| (id, object))
            .ok_or(Trap::native(code, "missing object"))
    }

    /// Resolves an operand. `write` is the caller's write intent, which
    /// changes what some globals resolve to.
    ///
    /// Resolving an object's data word 25, to read it or to write it, wakes
    /// that object (legacy memory.rs:631).
    pub(crate) fn resolve(
        &mut self,
        scope: u8,
        index: i16,
        write: bool,
    ) -> Result<(Cell, bool), Trap> {
        let resolved = self.locate(scope, index, write)?;
        if let Cell::Data(id, LOCKOUT) = resolved.0 {
            self.wake(id)?;
        }
        Ok(resolved)
    }

    /// Wakes an object from a Sleep the scheduler is counting down for it
    /// (0x44574b): the object runs every tick again, and the time it had
    /// left goes back into the sleeping frame's first parameter, so the
    /// Sleep goes on counting by itself. Ported from the old prototype's
    /// `wake_object` (legacy/crates/sims-vm/src/idle.rs:55).
    ///
    /// While the object runs a callback, the frame looked at is the one
    /// below the callback, which is where its own thread stopped.
    pub(crate) fn wake(&mut self, id: u16) -> Result<(), Trap> {
        let object = self
            .world
            .objects
            .get(&id)
            .ok_or(Trap::native(10, "missing object"))?;
        let Some(countdown) = object.countdown() else {
            return Ok(());
        };
        let depth = if id == self.me && self.synchronous {
            self.base
        } else {
            object.stack.len()
        };
        let sleeping = match depth.checked_sub(1).and_then(|i| object.stack.get(i)) {
            Some(frame) => {
                let owner = self.owner(frame)?;
                match self.program.tree(owner, frame.tree) {
                    Tree::Behavior(_, behavior) => {
                        let instruction = behavior
                            .instructions
                            .get(usize::from(frame.node))
                            .ok_or_else(|| Trap::Invalid("woken frame is past its tree".into()))?;
                        instruction.opcode & 0x7fff == 0
                    }
                    _ => return Err(Trap::MissingTree { id: frame.tree }),
                }
            }
            None => false,
        };
        let object = object_mut(self.world, id)?;
        object.set_countdown(-2);
        if sleeping && countdown >= 0 {
            *object.stack[depth - 1]
                .parameters
                .first_mut()
                .ok_or_else(|| Trap::Invalid("woken Sleep frame has no parameter".into()))? =
                countdown;
        }
        Ok(())
    }

    /// Where an operand lives, without the side effects of resolving it.
    fn locate(&self, scope: u8, index: i16, write: bool) -> Result<(Cell, bool), Trap> {
        let frame = self.frame()?;
        let me = self.me()?;
        let stack_object = frame.stack_object;
        let temp = |i: i16| -> Result<i16, Trap> {
            let i = bounded(i, 8, 2)?;
            me.temps
                .get(i)
                .copied()
                .ok_or_else(|| Trap::Invalid(format!("object has {} temps", me.temps.len())))
        };
        let attribute = |id: i16, index: i16| -> Result<Cell, Trap> {
            let (id, object) = self.object(id, 10)?;
            Ok(Cell::Attribute(
                id,
                bounded(index, object.attributes.len(), 6)?,
            ))
        };
        let person = |id: i16, index: i16| -> Result<Cell, Trap> {
            let (id, object) = self.object(id, 0x15)?;
            let person = object
                .person
                .as_ref()
                .ok_or(Trap::native(0x1c, "object is not a person"))?;
            let index = bounded(index, 256, 0x21)?;
            if index >= person.data.len() {
                // Older saves keep 80 person data words, not 256.
                return Err(Trap::Invalid(format!(
                    "person data word {index} is not in the save's {} words",
                    person.data.len()
                )));
            }
            Ok(Cell::Person(id, index))
        };
        // Saved slot 0 is the tile chain's next object, not a script slot,
        // so script slot i is saved slot i + 1. The old prototype sized the
        // slot table from the definition's SLOT resource as well
        // (legacy/crates/sims-content/src/save_storage.rs:1102-1113); here
        // only saved slots count. Unverified.
        let slot = |id: i16, index: i16| -> Result<Cell, Trap> {
            let (_, object) = self.object(id, 0x15)?;
            let i = bounded(index, object.slots.len().saturating_sub(1), 0x1b)?;
            Ok(Cell::Literal(object.slots[i + 1].object))
        };
        let me_id = i16::try_from(self.me).map_err(|_| {
            Trap::Invalid(format!("object ID {} does not fit a script word", self.me))
        })?;
        let cell = match scope {
            0 => attribute(me_id, index)?,
            1 => attribute(stack_object, index)?,
            3 | 4 => {
                let index = bounded(index, 72, 7)?;
                let (id, object) =
                    self.object(if scope == 3 { me_id } else { stack_object }, 10)?;
                if index >= object.data.len() {
                    return Err(Trap::Invalid(format!(
                        "object {id} has {} data words",
                        object.data.len()
                    )));
                }
                Cell::Data(id, index)
            }
            6 => {
                let index = bounded(index, 64, 0x3f)?;
                if index >= self.world.globals.len() {
                    // Saves before SIMI version 0x41 keep 32 globals.
                    return Err(Trap::Invalid(format!(
                        "global {index} is not in the lot's {} globals",
                        self.world.globals.len()
                    )));
                }
                Cell::Global(index)
            }
            7 => Cell::Literal(index),
            8 => Cell::Temp(self.me, bounded(index, 8, 2)?),
            9 => Cell::Parameter(bounded(index, frame.parameters.len(), 8)?),
            10 => {
                if index != 0 {
                    return Err(Trap::native(0xf, "stack object selector must be zero"));
                }
                Cell::StackObject
            }
            11 => Cell::Temp(self.me, bounded(temp(index)?, 8, 2)?),
            13 => {
                let (id, _) = self.object(stack_object, 10)?;
                Cell::Temp(id, bounded(index, 8, 2)?)
            }
            14 | 15 | 17 => {
                let index = if scope == 17 { temp(index)? } else { index };
                let index = bounded(index, 16, 0x1a)?;
                let (id, object) =
                    self.object(if scope == 14 { me_id } else { stack_object }, 0x15)?;
                if object.person.is_none() {
                    return Err(Trap::native(0x1c, "object is not a person"));
                }
                Cell::Motive(id, index)
            }
            16 => slot(stack_object, index)?,
            18 => person(me_id, index)?,
            19 => {
                bounded(index, 256, 0x21)?;
                person(stack_object, index)?
            }
            20 => slot(me_id, index)?,
            21 => {
                // The stack object's definition as words
                // (legacy/crates/sims-vm/src/memory.rs:235). Definitions are
                // normalized to 108 words, so every index in bounds reads.
                let index = bounded(index, DEFINITION_WORDS, 0x23)?;
                let (_, object) = self.object(stack_object, 10)?;
                Cell::Literal(self.definition_words(object.guid)?[index])
            }
            22 => {
                let i = bounded(index, frame.parameters.len(), 8)?;
                attribute(stack_object, frame.parameters[i])?
            }
            25 => Cell::Local(bounded(index, frame.locals.len(), 8)?),
            26 => {
                // Tables 0..64 are the owner's own BCONs, 64..128 its
                // semi-global file's and 128..192 the global ones.
                let owner = self.owner(frame)?;
                Cell::Literal(self.program.constant(owner, index as u16).ok_or_else(|| {
                    Trap::Invalid(format!(
                        "no tuning constant at address {:#06x}",
                        index as u16
                    ))
                })?)
            }
            27 => {
                bounded(index, 8, 2)?;
                let (id, object) = self.object(stack_object, 10)?;
                Cell::DynamicSprite(
                    id,
                    bounded(temp(index)?, object.dynamic_sprites.len(), 0x31)?,
                )
            }
            30 => {
                if me.person.is_none() {
                    return Err(Trap::native(0x1c, "object is not a person"));
                }
                person(me_id, temp(index)?)?
            }
            31 => {
                if self.object(stack_object, 0x15)?.1.person.is_none() {
                    return Err(Trap::native(0x1c, "object is not a person"));
                }
                person(stack_object, temp(index)?)?
            }
            35 => {
                // Whether the stack object's definition names a tree for
                // callback `index`.
                let (_, object) = self.object(stack_object, 10)?;
                let definition = self.definition(object.guid)?;
                let callbacks = self
                    .program
                    .libraries
                    .get(&definition.resource)
                    .and_then(|l| l.callbacks.get(&definition.chunk))
                    .ok_or_else(|| {
                        Trap::Invalid(format!("no callback table for GUID 0x{:08X}", object.guid))
                    })?;
                Cell::Literal(i16::from(
                    usize::try_from(index)
                        .ok()
                        .and_then(|i| callbacks.get(i))
                        .is_some_and(|c| c.action != 0),
                ))
            }
            36 | 37 => {
                // The words belong to the object's own type, whatever code
                // is running, or to the type its definition's words 92 and
                // 93 name instead (0x457e95, 0x457eba; legacy
                // legacy/crates/sims-vm/src/resource_storage.rs). A type with no
                // record has no words, so every index is out of bounds.
                let (_, object) =
                    self.object(if scope == 36 { me_id } else { stack_object }, 10)?;
                let words = self.definition_words(object.guid)?;
                let alias = words[92] as u16 as u32 | (words[93] as u16 as u32) << 16;
                let guid = if alias == 0 { object.guid } else { alias };
                let records = &self.world.type_attributes;
                let record = records.iter().position(|r| r.guid == guid);
                let count = record.map_or(0, |r| records[r].values.len());
                let index = bounded(index, count, 0x40)?;
                Cell::TypeAttribute(record.unwrap_or(0), index)
            }
            // Tree advertisements (12, 28, 29), rooms (23), neighbors (24,
            // 32), jobs (33), neighborhood data (34) and scope 38 need
            // state the simulation doesn't hold yet.
            12 | 23 | 24 | 28 | 29 | 32 | 33 | 34 | 38 => {
                return Err(unsupported(scope));
            }
            2 | 5 | 39..=255 => return Err(Trap::native(9, "invalid memory scope")),
        };
        // Only global 34 always comes back with a word pointer, and global
        // 36 with write intent; the other globals resolve to values
        // (legacy memory.rs:519).
        let writable = match cell {
            Cell::Literal(_) => false,
            Cell::Global(i) => i == 34 || (write && i == 36),
            _ => write,
        };
        Ok((cell, writable))
    }

    /// The value in a cell. Motives read as their float rounded to an
    /// integer.
    pub(crate) fn value(&self, cell: Cell) -> Result<i16, Trap> {
        let object = |id: u16| {
            self.world
                .objects
                .get(&id)
                .ok_or(Trap::native(10, "missing object"))
        };
        let get = |words: &[i16], i: usize| {
            words
                .get(i)
                .copied()
                .ok_or_else(|| Trap::Invalid(format!("word {i} out of range")))
        };
        match cell {
            Cell::Literal(value) => Ok(value),
            Cell::Attribute(id, i) => get(&object(id)?.attributes, i),
            Cell::Data(id, i) => get(&object(id)?.data, i),
            Cell::Person(id, i) => get(&person_data(object(id)?)?.data, i),
            Cell::Temp(id, i) => get(&object(id)?.temps, i),
            Cell::DynamicSprite(id, i) => get(&object(id)?.dynamic_sprites, i),
            Cell::Global(i) => get(&self.world.globals, i),
            Cell::TypeAttribute(record, i) => self
                .world
                .type_attributes
                .get(record)
                .ok_or_else(|| Trap::Invalid(format!("no type attribute record {record}")))
                .and_then(|r| get(&r.values, i)),
            Cell::Parameter(i) => get(&self.frame()?.parameters, i),
            Cell::Local(i) => get(&self.frame()?.locals, i),
            Cell::StackObject => Ok(self.frame()?.stack_object),
            Cell::Motive(id, i) => {
                let value = self.motive(id, i)?;
                let register = value.ext();
                let low = Ext::from_i32(i32::MIN);
                if register.is_nan() || register < low || register >= -low {
                    return Err(Trap::Unsupported(format!(
                        "motive {i} holds {:?}, which needs an unvalidated x87 conversion",
                        value
                    )));
                }
                // The old prototype rounded half away from zero
                // (legacy memory.rs:574); the x87 default rounds half to
                // even, which is what this does. Unverified.
                Ok(register.to_i32_round() as i16)
            }
        }
    }

    /// A motive's stored float.
    pub(crate) fn motive(&self, id: u16, index: usize) -> Result<F32, Trap> {
        let object = self
            .world
            .objects
            .get(&id)
            .ok_or(Trap::native(0x15, "missing object"))?;
        person_data(object)?
            .motives
            .get(index)
            .map(|&m| F32::from_f32(m))
            .ok_or(Trap::native(0x1a, "memory index out of bounds"))
    }

    pub(crate) fn set_motive(&mut self, id: u16, index: usize, value: F32) -> Result<(), Trap> {
        let motive = self
            .world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .and_then(|p| p.motives.get_mut(index))
            .ok_or(Trap::native(0x15, "missing person"))?;
        *motive = value.to_f32();
        Ok(())
    }

    /// Stores a word. Returns false for cells with no integer word behind
    /// them: literals and motives.
    pub(crate) fn store(&mut self, cell: Cell, value: i16) -> Result<bool, Trap> {
        let me = self.me;
        let world = &mut *self.world;
        let target = match cell {
            Cell::Literal(_) | Cell::Motive(..) => return Ok(false),
            Cell::Data(_, i @ (2 | 3 | 11)) => {
                // Words 2 and 3 are the container and its slot, and 11 the
                // object's ID; the original updates more than the word
                // (legacy memory.rs:711).
                return Err(Trap::Unsupported(format!(
                    "writing object data word {i} has a native side effect"
                )));
            }
            Cell::Attribute(id, i) => object_mut(world, id)?.attributes.get_mut(i),
            Cell::Data(id, i) => object_mut(world, id)?.data.get_mut(i),
            // Writes to person data words 68, 74 and 84 invalidate cached
            // portraits, and 77 and 81 notify indicators (legacy
            // memory.rs:634-654). Both are presentation only.
            Cell::Person(id, i) => object_mut(world, id)?
                .person
                .as_mut()
                .ok_or(Trap::native(0x1c, "object is not a person"))?
                .data
                .get_mut(i),
            Cell::Temp(id, i) => object_mut(world, id)?.temps.get_mut(i),
            Cell::DynamicSprite(id, i) => object_mut(world, id)?.dynamic_sprites.get_mut(i),
            Cell::Global(i) => world.globals.get_mut(i),
            Cell::TypeAttribute(record, i) => world
                .type_attributes
                .get_mut(record)
                .and_then(|r| r.values.get_mut(i)),
            Cell::Parameter(i) => top_mut(world, me)?.parameters.get_mut(i),
            Cell::Local(i) => top_mut(world, me)?.locals.get_mut(i),
            Cell::StackObject => Some(&mut top_mut(world, me)?.stack_object),
        };
        *target.ok_or_else(|| Trap::Invalid("word out of range".into()))? = value;
        Ok(true)
    }

    /// Reads an operand.
    pub(crate) fn read(&mut self, scope: u8, index: i16) -> Result<i16, Trap> {
        let (cell, _) = self.resolve(scope, index, false)?;
        self.value(cell)
    }
}

fn object_mut(world: &mut World, id: u16) -> Result<&mut Object, Trap> {
    world
        .objects
        .get_mut(&id)
        .ok_or(Trap::native(10, "missing object"))
}

fn top_mut(world: &mut World, me: u16) -> Result<&mut Frame, Trap> {
    object_mut(world, me)?
        .stack
        .last_mut()
        .ok_or_else(|| Trap::Invalid("empty stack".into()))
}

fn person_data(object: &Object) -> Result<&sims_formats::saved_objects::Person, Trap> {
    object
        .person
        .as_ref()
        .ok_or(Trap::native(0x1c, "object is not a person"))
}
