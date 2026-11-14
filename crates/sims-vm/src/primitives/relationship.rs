//! Relationship (opcode 26): read or write one number of the relationship
//! one object has to another.
//!
//! Ported from the old prototype's `relationship_impl`
//! (legacy/crates/sims-vm/src/relationship.rs), recovered from the
//! original's 0x44d17c.
//!
//! Operands: the column (byte 0); the direction (byte 1: 0 from me to the
//! stack object, 1 from the stack object to me, 2 from the stack object to
//! the object a local names, 3 the other way round); flags (byte 2: 1
//! take the false branch if the number is not there, 2 use the
//! neighborhood's records, 4 write); the local (byte 3); and the operand
//! that gives or takes the number (scope in byte 4, index in bytes 6 and
//! 7).
//!
//! An object keeps its relationships as rows, one per other object, in
//! the order they were first used. A row or column that is missing is
//! made, as zeros, unless flag 1 is set.
//!
//! Only relationships kept on the lot's objects are ported. Two people
//! keep theirs in the neighborhood's records, as does flag 2; both fail
//! loudly.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use sims_formats::saved_objects::Relationship;

impl Machine<'_, '_> {
    pub(crate) fn relationship(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        self.relationship_with(operands, None)
    }

    /// Old Relationship (opcode 24), the same primitive with its operands
    /// in an older order: whether to write (byte 0), the column (byte 1),
    /// the parameter that gives or takes the number (byte 2, signed), the
    /// direction (byte 3) and the flags 1 and 2 (byte 4). Where the newer
    /// form reads a local for directions 2 and 3, this one reads parameter
    /// 0. Ported from the old prototype's `legacy_relationship`.
    pub(crate) fn old_relationship(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        if (operands[1] as i8) < 0 {
            return Err(Trap::Unsupported(
                "Old Relationship with a negative column".into(),
            ));
        }
        let named = if matches!(operands[3], 2 | 3) {
            Some(
                *self
                    .frame()?
                    .parameters
                    .first()
                    .ok_or(Trap::native(8, "Old Relationship needs parameter 0"))?,
            )
        } else {
            None
        };
        let index = i16::from(operands[2] as i8).to_le_bytes();
        let flags = (operands[4] & 3) | if operands[0] != 0 { 4 } else { 0 };
        self.relationship_with(
            &[operands[1], operands[3], flags, 0, 9, 0, index[0], index[1]],
            named,
        )
    }

    fn relationship_with(&mut self, operands: &[u8; 8], named: Option<i16>) -> Result<bool, Trap> {
        let frame = self.frame()?;
        let stack_object = frame.stack_object;
        let direction = operands[1];
        let named = if let Some(named) = named {
            named
        } else if matches!(direction, 2 | 3) {
            let local = usize::from(operands[3]);
            let value = *frame
                .locals
                .get(local)
                .ok_or(Trap::native(0x33, "Relationship local out of bounds"))?;
            self.check_local(local)?;
            value
        } else {
            0
        };
        if operands[2] & 2 != 0 {
            return Err(Trap::Unsupported(
                "Relationship on the neighborhood's records".into(),
            ));
        }
        let me = self.me as i16;
        let (source, target) = match direction {
            0 => (me, stack_object),
            1 => (stack_object, me),
            2 => (stack_object, named),
            3 => (named, stack_object),
            _ => return Err(Trap::native(0x15, "invalid Relationship direction")),
        };
        let find = |id: i16| {
            u16::try_from(id)
                .ok()
                .and_then(|id| self.world.objects.get(&id))
                .ok_or(Trap::native(0x15, "missing Relationship object"))
        };
        let (from, to) = (find(source)?, find(target)?);
        if from.person.is_some() && to.person.is_some() {
            return Err(Trap::Unsupported(
                "Relationship between two people, which the neighborhood keeps".into(),
            ));
        }
        let source = source as u16;
        let writing = operands[2] & 4 != 0;
        let column = usize::from(operands[0]);
        let row = from
            .relationships
            .iter()
            .position(|r| r.present != 0 && r.target == i32::from(target));
        if row.is_none_or(|row| from.relationships[row].values.len() <= column) {
            if operands[2] & 1 != 0 {
                return Ok(false);
            }
            let rows = &mut self
                .world
                .objects
                .get_mut(&source)
                .ok_or(Trap::native(0x15, "missing Relationship object"))?
                .relationships;
            let row = row.unwrap_or_else(|| {
                rows.push(Relationship {
                    present: 1,
                    target: i32::from(target),
                    values: Vec::new(),
                });
                rows.len() - 1
            });
            rows[row].values.resize(column + 1, 0);
        }
        // The operand is resolved for writing in both directions.
        let (cell, writable) = self.resolve(operands[4], word(operands, 6), true)?;
        if writing {
            let value = self.value(cell)?;
            *cell_of(self.world, source, target, column)? = i32::from(value);
        } else {
            let old = *cell_of(self.world, source, target, column)? as i16;
            if !writable || !self.store(cell, old)? {
                return Err(Trap::native(
                    0x13,
                    "Relationship destination is not writable",
                ));
            }
        }
        Ok(true)
    }
}

fn cell_of(
    world: &mut sims_sim::World,
    source: u16,
    target: i16,
    column: usize,
) -> Result<&mut i32, Trap> {
    world
        .objects
        .get_mut(&source)
        .and_then(|o| {
            o.relationships
                .iter_mut()
                .find(|r| r.present != 0 && r.target == i32::from(target))
        })
        .and_then(|r| r.values.get_mut(column))
        .ok_or_else(|| Trap::Invalid("a relationship row vanished".into()))
}
