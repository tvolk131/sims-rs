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
//! - 4: the next instance of the type the GUID names, in that type's list
//!   in [`World::instances`], which is made here on first use. A GUID
//!   with no definition is the false branch with the cursor kept.
//! - 6: the next object whose data word 59 equals parameter 0 (0x44f211).
//! - 10: the next career ID in [`crate::Program::careers`] (0x404cb3). A
//!   cursor that is no career ID starts from the first. Unlike the object
//!   modes, a next ID of zero still takes the true branch.
//! - 12: the next member of the cursor's family (0x44f0d9).
//!
//! - 8: the next object on the cursor's tile, going round: the objects
//!   that stand on the tile and everything they hold, in the order the
//!   lot links them ([`crate::placement::on_tile`]). A cursor that stands
//!   on no tile is the false branch.
//! - 9: the first object on the next tile beside the object a local
//!   names (byte 6), if it is in that object's room. The tiles go east,
//!   south, west, north in the first two coordinates (+1 in the first,
//!   +1 in the second, -1, -1), starting after the tile the cursor stands
//!   on. No such object is the false branch.
//!
//! - 5: the next of the neighborhood's records
//!   ([`sims_sim::neighbors::Neighbors::next`], 0x425b15): the ID of the
//!   first occupied slot after the cursor's. A cursor that is negative or
//!   not below the number of slots gives 0. The destination takes the
//!   next ID, zero included, and the branch is whether it is nonzero
//!   (0x44ece2).
//! - 7: the same walk, until a record of the person type the GUID names
//!   (0x44ed07). Each ID the walk reaches is stored as it goes; the
//!   original compares the GUID of the definition the record is linked to
//!   (0x4572af on record +8), which is the definition of the record's
//!   own GUID. A record whose GUID has no definition fails: what the
//!   original's comparison does without a link was not seen.
//!
//! Mode 11 fails loudly: it needs the neighborhood's lot order.
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
            4 => {
                let guid = u32::from_le_bytes([operands[0], operands[1], operands[2], operands[3]]);
                match self.next_instance(guid, current)? {
                    Some(next) => Some(next),
                    None => return Ok(false),
                }
            }
            5 => Some(self.world.neighbors.next(current)),
            7 => {
                let guid = u32::from_le_bytes([operands[0], operands[1], operands[2], operands[3]]);
                return self.next_neighbor_of(guid, destination, current);
            }
            8 => match self.next_on_tile(current)? {
                Some(next) => Some(next),
                None => return Ok(false),
            },
            9 => match self.next_beside(current, usize::from(operands[6]))? {
                Some(next) => Some(next),
                None => return Ok(false),
            },
            10 => {
                let careers = self
                    .program
                    .careers
                    .as_ref()
                    .ok_or_else(|| missing(mode, "the career list"))?;
                // The cursor is sign-extended and matched against 32-bit
                // IDs; the next ID is cut to a word.
                let start = careers
                    .iter()
                    .position(|&id| id == i32::from(current))
                    .map_or(0, |i| i + 1);
                let Some(&next) = careers.get(start) else {
                    return Ok(false);
                };
                self.store(destination, next as i16)?;
                return Ok(true);
            }
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

    /// Mode 7: walks the neighborhood's records from `current`, storing
    /// each ID it reaches, until one of the person type `guid` (true) or
    /// the end (false, with zero stored).
    fn next_neighbor_of(
        &mut self,
        guid: u32,
        destination: crate::memory::Cell,
        mut current: i16,
    ) -> Result<bool, Trap> {
        loop {
            let next = self.world.neighbors.next(current);
            self.store(destination, next)?;
            if next == 0 {
                return Ok(false);
            }
            if let Some(record) = self.world.neighbors.get(next) {
                if self.catalog.object(record.guid).is_none() {
                    return Err(Trap::Unsupported(format!(
                        "Set To Next mode 7: neighbor {next} has the GUID 0x{:08X}, which no \
                         definition has; the original compares the GUID of the definition the \
                         record is linked to",
                        record.guid
                    )));
                }
                if record.guid == guid {
                    return Ok(true);
                }
            }
            current = next;
        }
    }

    /// Mode 4: the instance after `current` in the GUID's list, or the
    /// first for a zero cursor; zero when there is none or the cursor is
    /// not in the list. None when no definition has the GUID. Ported from
    /// the old prototype (legacy/crates/sims-vm/src/person.rs:919).
    fn next_instance(&mut self, guid: u32, current: i16) -> Result<Option<i16>, Trap> {
        match self
            .catalog
            .objects
            .iter()
            .filter(|o| o.guid == guid)
            .count()
        {
            0 => return Ok(None),
            1 => {}
            _ => {
                // The original lists instances by definition, not GUID.
                return Err(Trap::Unsupported(format!(
                    "Set To Next mode 4: GUID 0x{guid:08X} names more than one definition"
                )));
            }
        }
        let objects = &self.world.objects;
        let list = self.world.instances.entry(guid).or_insert_with(|| {
            objects
                .iter()
                .filter(|(_, o)| o.guid == guid)
                .map(|(&id, _)| id)
                .collect()
        });
        let start = match current {
            0 => Some(0),
            _ => list
                .iter()
                .position(|&id| i16::try_from(id) == Ok(current))
                .map(|i| i + 1),
        };
        let next = start.and_then(|i| list.get(i)).copied().unwrap_or(0);
        i16::try_from(next)
            .map(Some)
            .map_err(|_| Trap::Invalid(format!("object ID {next} does not fit a script word")))
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

impl Machine<'_, '_> {
    /// Mode 8.
    fn next_on_tile(&self, current: i16) -> Result<Option<i16>, Trap> {
        let Some(id) = u16::try_from(current)
            .ok()
            .filter(|id| self.world.objects.contains_key(id))
        else {
            return Ok(None);
        };
        let Some(chain) = crate::placement::on_tile(self.world, id)? else {
            return Ok(None);
        };
        let place = chain.iter().position(|&other| other == id);
        Ok(place.map(|i| chain[(i + 1) % chain.len()] as i16))
    }

    /// Mode 9.
    fn next_beside(&self, current: i16, local: usize) -> Result<Option<i16>, Trap> {
        let reference = *self
            .frame()?
            .locals
            .get(local)
            .ok_or(Trap::native(8, "Set To Next local out of bounds"))?;
        self.check_local(local)?;
        let object = u16::try_from(reference)
            .ok()
            .and_then(|id| self.world.objects.get(&id))
            .ok_or(Trap::native(10, "Set To Next reference object is missing"))?;
        let origin = sims_sim::World::tile(object.position);
        let cursor = u16::try_from(current)
            .ok()
            .and_then(|id| self.world.objects.get(&id))
            .map(|o| sims_sim::World::tile(o.position))
            .map(|at| [at[0].wrapping_sub(origin[0]), at[1].wrapping_sub(origin[1])]);
        let mut delta = cursor.unwrap_or([1, 0]);
        let mut first = cursor.is_none();
        loop {
            if !first {
                delta = if delta[0] >= 1 {
                    [0, 1]
                } else if delta[0] < 0 {
                    [0, -1]
                } else if delta[1] >= 1 {
                    [-1, 0]
                } else {
                    return Ok(None);
                };
            }
            first = false;
            let at = [
                i32::from(origin[0]) + i32::from(delta[0]),
                i32::from(origin[1]) + i32::from(delta[1]),
            ];
            if let Some(found) = crate::placement::first_on(self.world, at, origin[2])
                && self.world.objects[&found].data.get(29) == object.data.get(29)
            {
                return Ok(Some(found as i16));
            }
        }
    }
}

fn missing(mode: u8, state: &str) -> Trap {
    Trap::Unsupported(format!(
        "Set To Next mode {mode} needs {state}, which the simulation does not hold"
    ))
}
