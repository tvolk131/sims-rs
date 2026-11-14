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
//! With flag 2 the rows are the neighborhood's records': the source and
//! target are neighbor IDs (for directions 0 and 1, mine is my person
//! data word 31), the source's record keeps the row, and the row's key is
//! the target record's own ID. Two people on the lot keep theirs there
//! too, without the flag: the source person's record, under the target
//! person's word 31. Either way a record that is missing is native error
//! 0x15, and a write marks the two records first (`mark_relationships`).

use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use sims_formats::saved_objects::Relationship;
use sims_sim::neighbors::Row;

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
        let me = self.me as i16;
        let writing = operands[2] & 4 != 0;
        let (table, key) = if operands[2] & 2 != 0 {
            // Directions 0 and 1 name me by my neighbor ID (person data
            // word 31); the stack object and the local are neighbor IDs.
            let neighbor = |id: i16| -> Result<i16, Trap> {
                let object = u16::try_from(id)
                    .ok()
                    .and_then(|id| self.world.objects.get(&id))
                    .ok_or(Trap::native(0x15, "missing Relationship object"))?;
                let person = object
                    .person
                    .as_ref()
                    .ok_or(Trap::native(0x1c, "Relationship caller is not a person"))?;
                person
                    .data
                    .get(NEIGHBOR)
                    .copied()
                    .ok_or_else(|| Trap::Invalid("the person has no neighbor ID".into()))
            };
            let (source, target) = match direction {
                0 => (neighbor(me)?, stack_object),
                1 => (stack_object, neighbor(me)?),
                2 => (stack_object, named),
                3 => (named, stack_object),
                _ => return Err(Trap::native(0x15, "invalid Relationship direction")),
            };
            let key = self.records(source, target)?;
            if writing {
                self.mark_relationships(source, target);
            }
            (Table::Neighbor(source), key)
        } else {
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
            match (&from.person, &to.person) {
                (Some(from), Some(to)) => {
                    // Two people keep theirs in the source's record, under
                    // the target's neighbor ID.
                    let word = |p: &sims_formats::saved_objects::Person| {
                        p.data
                            .get(NEIGHBOR)
                            .copied()
                            .ok_or_else(|| Trap::Invalid("the person has no neighbor ID".into()))
                    };
                    let (source, target) = (word(from)?, word(to)?);
                    self.records(source, target)?;
                    if writing {
                        self.mark_relationships(source, target);
                    }
                    (Table::Neighbor(source), i32::from(target))
                }
                _ => (Table::Object(source as u16), i32::from(target)),
            }
        };
        let column = usize::from(operands[0]);
        if values(self.world, table, key).is_none_or(|values| values.len() <= column) {
            if operands[2] & 1 != 0 {
                return Ok(false);
            }
            add_row(self.world, table, key)?;
            values(self.world, table, key)
                .ok_or_else(|| Trap::Invalid("a relationship row vanished".into()))?
                .resize(column + 1, 0);
        }
        // The operand is resolved for writing in both directions.
        let (cell, writable) = self.resolve(operands[4], word(operands, 6), true)?;
        let vanished = || Trap::Invalid("a relationship row vanished".into());
        if writing {
            let value = self.value(cell)?;
            *values(self.world, table, key)
                .and_then(|v| v.get_mut(column))
                .ok_or_else(vanished)? = i32::from(value);
        } else {
            let old = *values(self.world, table, key)
                .and_then(|v| v.get_mut(column))
                .ok_or_else(vanished)? as i16;
            if !writable || !self.store(cell, old)? {
                return Err(Trap::native(
                    0x13,
                    "Relationship destination is not writable",
                ));
            }
        }
        Ok(true)
    }

    /// Checks that both neighbor IDs name records, as the original needs
    /// both (native error 0x15), and gives the key the source's row for
    /// the target has: the target record's own ID.
    fn records(&self, source: i16, target: i16) -> Result<i32, Trap> {
        let neighbors = &self.world.neighbors;
        let missing = || Trap::native(0x15, "missing Relationship neighbor");
        neighbors.get(source).ok_or_else(missing)?;
        Ok(i32::from(neighbors.get(target).ok_or_else(missing)?.id))
    }

    /// What a write to a relationship between people marks (0x44485b),
    /// before the column is looked at, so even when it is absent: both
    /// records' friend counts as out of date (0x4246b9), and the
    /// registry's byte 0x12.
    ///
    /// Not modeled: 0x4246b9 also marks the friend count of the family
    /// whose chunk ID is the record's person data word 61 (family +0x124).
    /// The neighborhood's families are not kept; nothing ported reads
    /// that count.
    fn mark_relationships(&mut self, source: i16, target: i16) {
        for id in [source, target] {
            if let Some(record) = self.world.neighbors.get_mut(id) {
                record.stale = true;
            }
        }
        self.world.neighbors.relationships_changed = true;
    }
}

/// Person data word 31: a person's neighbor ID.
const NEIGHBOR: usize = 31;

/// Where a relationship's rows are kept.
#[derive(Debug, Clone, Copy)]
enum Table {
    /// On a lot object, by its ID.
    Object(u16),
    /// In a neighborhood record, by its neighbor ID.
    Neighbor(i16),
}

/// The values of the row for `key`, if the table has one.
fn values(world: &mut sims_sim::World, table: Table, key: i32) -> Option<&mut Vec<i32>> {
    match table {
        Table::Object(id) => world
            .objects
            .get_mut(&id)?
            .relationships
            .iter_mut()
            .find(|r| r.present != 0 && r.target == key)
            .map(|r| &mut r.values),
        Table::Neighbor(id) => world
            .neighbors
            .get_mut(id)?
            .row_mut(key)
            .map(|r| &mut r.values),
    }
}

/// Adds an empty row for `key` at the end of the table, unless it has one.
fn add_row(world: &mut sims_sim::World, table: Table, key: i32) -> Result<(), Trap> {
    if values(world, table, key).is_some() {
        return Ok(());
    }
    match table {
        Table::Object(id) => world
            .objects
            .get_mut(&id)
            .ok_or(Trap::native(0x15, "missing Relationship object"))?
            .relationships
            .push(Relationship {
                present: 1,
                target: key,
                values: Vec::new(),
            }),
        Table::Neighbor(id) => world
            .neighbors
            .get_mut(id)
            .ok_or(Trap::native(0x15, "missing Relationship neighbor"))?
            .relationships
            .push(Row {
                target: key,
                values: Vec::new(),
            }),
    }
    Ok(())
}
