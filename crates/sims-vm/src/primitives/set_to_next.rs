//! Set To Next (opcode 31): advance an object cursor.
//!
//! Ported from the old prototype's `set_to_next`
//! (legacy/crates/sims-vm/src/person.rs:842-1169).
//!
//! Operands: a GUID (bytes 0..4, modes 4 and 7), the mode (byte 4, low
//! seven bits), the destination scope (byte 5) and index (byte 7) when bit
//! 7 of byte 4 is set, and a local index (byte 6, mode 9). Without bit 7
//! the cursor is the stack object.
//!
//! The cursor is read, then resolved for writing; a destination that takes
//! no write is native error 0x13. Zero starts from the beginning. A cursor
//! that is not in the order, or that has no successor, exhausts the scan:
//! the false branch, with the cursor kept (except in modes 1 and 3, which
//! store zero).
//!
//! Modes over state `World` holds:
//! - 0: the next object in registration order ([`World::order`]).
//! - 1: the next person, in the same order.
//! - 2: the next object that is not a person (definition word 9 is not 2).
//! - 3: the next tile of the cursor's multi-tile group.
//! - 6: the next object whose data word 59 equals parameter 0 (0x44f211).
//! - 12: the next member of the cursor's family (0x44f0d9).
//!
//! The others fail loudly: 4 needs the original's per-GUID instance cache,
//! 5 and 7 the neighborhood's person directory, 8 and 9 the tile chains, 10
//! the career list and 11 the neighborhood's lot order.
//!
//! [`World::order`]: sims_sim::World::order

use crate::fault::Trap;
use crate::interpreter::Machine;
use sims_sim::Object;

/// Data word 59: an object's category, for mode 6.
const CATEGORY: usize = 59;
/// Person data word 61: the family number, for mode 12.
const FAMILY: usize = 61;

impl Machine<'_, '_> {
    pub(crate) fn set_to_next(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let (scope, index) = if operands[4] & 0x80 == 0 {
            (10, 0)
        } else {
            (operands[5], i16::from(operands[7]))
        };
        // The destination is resolved for writing before the mode is
        // looked at, so the write check covers every mode, the loud ones
        // included.
        let (destination, writable) = self.resolve(scope, index, true)?;
        let current = self.value(destination)?;
        if !writable || !self.store(destination, current)? {
            return Err(Trap::native(
                0x13,
                "Set To Next destination is not writable",
            ));
        }
        let mode = operands[4] & 0x7f;
        let next = match mode {
            0..=2 | 6 => self.next_in_order(mode, current)?,
            3 => Some(self.next_tile(current)?),
            12 => self.next_family_member(current)?,
            4 => return Err(missing(mode, "the original's per-GUID instance cache")),
            5 | 7 => return Err(missing(mode, "the neighborhood's person directory")),
            8 | 9 => return Err(missing(mode, "the lot's tile chains")),
            10 => return Err(missing(mode, "the career list")),
            11 => return Err(missing(mode, "the neighborhood's lot order")),
            _ => {
                return Err(Trap::Unsupported(format!(
                    "Set To Next mode {mode} is not known"
                )));
            }
        };
        let Some(next) = next else {
            return Ok(false);
        };
        self.store(destination, next)?;
        Ok(next != 0)
    }

    /// Modes 0, 1, 2 and 6: the next match after `current` in registration
    /// order, or in the person vector for mode 1. None leaves the cursor as
    /// it was; mode 1 gives zero instead.
    fn next_in_order(&self, mode: u8, current: i16) -> Result<Option<i16>, Trap> {
        let order = if mode == 1 {
            self.person_order()?
        } else {
            self.registration_order()?
        };
        let remaining = match current {
            0 => &order[..],
            _ => match order.iter().position(|&(id, _)| id == current) {
                Some(i) => &order[i + 1..],
                None => &[],
            },
        };
        // Mode 6 compares against the frame's first parameter, which it
        // reads only when there is an object left to compare.
        let category = if mode == 6 && !remaining.is_empty() {
            Some(*self.frame()?.parameters.first().ok_or_else(|| {
                // The old prototype failed here; what the original reads
                // from a frame with no parameters is unverified.
                Trap::Unsupported("Set To Next mode 6 needs parameter 0".into())
            })?)
        } else {
            None
        };
        for &(id, object) in remaining {
            let matches = match mode {
                6 => {
                    Some(*object.data.get(CATEGORY).ok_or_else(|| {
                        Trap::Invalid(format!("object {id} has no data word {CATEGORY}"))
                    })?) == category
                }
                2 => self.definition_words(object.guid)?[9] != 2,
                _ => true,
            };
            if matches {
                return Ok(Some(id));
            }
        }
        Ok((mode == 1).then_some(0))
    }

    /// Mode 3: the tile after `current` in its multi-tile group, the head
    /// after the last tile, or zero for an object in no group.
    ///
    /// `World` keeps each tile's saved head (its parent), not the native
    /// next links. Loading appends each tile to its head's chain in
    /// registration order (0x423223, 0x4228a9; legacy
    /// sims-content/src/save_storage.rs:1243-1267), so the chain is the
    /// head followed by its tiles in [`sims_sim::World::order`]. The
    /// original's next link (legacy multitile.rs:98) is followed, falling
    /// back to the head. Unverified once groups are split or joined at run
    /// time, which nothing does yet.
    fn next_tile(&self, current: i16) -> Result<i16, Trap> {
        let Some(part) = u16::try_from(current)
            .ok()
            .and_then(|id| self.world.objects.get(&id))
            .and_then(|o| o.multipart.as_ref())
        else {
            return Ok(0);
        };
        let head = if part.parent == 0 {
            current
        } else {
            part.parent
        };
        let order = self.registration_order()?;
        let tiles = order
            .iter()
            .filter(|(_, o)| o.multipart.as_ref().is_some_and(|p| p.parent == head));
        let next = if head == current {
            tiles.map(|&(id, _)| id).next()
        } else {
            let mut after = tiles.skip_while(|&&(id, _)| id != current);
            after.next();
            after.next().map(|&(id, _)| id)
        };
        Ok(match next {
            Some(id) => id,
            None if head == current => 0,
            None => head,
        })
    }

    /// Mode 12: the next person after `current` in the person vector, wrapping
    /// around, with the same family number. Zero has no family.
    fn next_family_member(&self, current: i16) -> Result<Option<i16>, Trap> {
        if current == 0 {
            return Ok(None);
        }
        let family = |object: &Object| {
            object
                .person
                .as_ref()
                .ok_or(Trap::native(0x1c, "family cursor is not a person"))?
                .data
                .get(FAMILY)
                .copied()
                .ok_or_else(|| Trap::Invalid(format!("person data has no word {FAMILY}")))
        };
        let cursor = u16::try_from(current)
            .ok()
            .and_then(|id| self.world.objects.get(&id))
            .ok_or(Trap::native(0x1c, "family cursor is not a person"))?;
        let wanted = family(cursor)?;
        let order = self.person_order()?;
        let offset = order
            .iter()
            .position(|&(id, _)| id == current)
            .ok_or_else(|| {
                Trap::Invalid(format!(
                    "family cursor {current} is not in the person vector"
                ))
            })?;
        for &(id, object) in order[offset + 1..].iter().chain(&order[..offset]) {
            if family(object)? == wanted {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// The objects in registration order, by script ID.
    fn registration_order(&self) -> Result<Vec<(i16, &Object)>, Trap> {
        self.world
            .order
            .iter()
            .map(|&id| {
                let object = self.world.objects.get(&id).ok_or_else(|| {
                    Trap::Invalid(format!("registered object {id} does not exist"))
                })?;
                let id = i16::try_from(id).map_err(|_| {
                    Trap::Invalid(format!("object ID {id} does not fit a script word"))
                })?;
                Ok((id, object))
            })
            .collect()
    }

    /// The native person vector. Loading appends each person as it is
    /// registered (legacy sims-content/src/save_storage.rs:1198), so it is
    /// the people in registration order. Unverified once Sims join or leave
    /// the lot, which nothing does yet.
    fn person_order(&self) -> Result<Vec<(i16, &Object)>, Trap> {
        let mut order = self.registration_order()?;
        order.retain(|(_, o)| o.person.is_some());
        Ok(order)
    }
}

fn missing(mode: u8, state: &str) -> Trap {
    Trap::Unsupported(format!(
        "Set To Next mode {mode} needs {state}, which the simulation does not hold"
    ))
}
