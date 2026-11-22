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
//! - 1: the next person in the person vector ([`World::people`],
//!   0x44efb4), which is not registration order: a person made at run
//!   time comes last.
//! - 2: the next object that is not a person (definition word 9 is not 2).
//! - 3: the next tile of the cursor's multi-tile group, along the
//!   group's links (0x44ec79).
//! - 4: the next instance of the type the GUID names, in that type's list
//!   in [`World::instances`], which is made here on first use. A GUID
//!   with no definition is the false branch with the cursor kept.
//! - 6: the next object whose data word 59 equals parameter 0 (0x44f211).
//! - 10: the next career ID in [`crate::Program::careers`] (0x404cb3). A
//!   cursor that is no career ID starts from the first. Unlike the object
//!   modes, a next ID of zero still takes the true branch.
//! - 12: the next member of the cursor's family, round the person
//!   vector (0x44f0d4).
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
//! - 11: the neighborhood's other lots, nearest first. The original
//!   (0x44f08d) walks a vector of 32-bit lot numbers at neighborhood
//!   +0x11c: a cursor of 0 gives the first, any other the one after it
//!   (the low word is stored); a cursor not in the vector, or the last,
//!   is the false branch (0x44eccf) with nothing stored. The lot load
//!   (0x428041, at 0x428350) fills the vector with every lot number from
//!   1 to below the length of the neighborhood's table of lot positions
//!   (+0x110, 8 bytes a lot, read by 0x42886e) but the lot being loaded,
//!   and sorts it (std::sort, 0x42ffce) by the squared distance of each
//!   lot's position from the loaded lot's (0x427fb9). Both are empty in
//!   every capture of the original (`neighborhood_owners.coordinates`
//!   and `.nearby_houses`, their begin pointers 0 as the neighborhood's
//!   constructor 0x42441a leaves them): the Goth house's from its load
//!   to update 8699, and the snack order's, the Pleasants', House 7's
//!   and the Roomies'. A write watch on +0x110 to +0x11f from the
//!   neighborhood view to the Goth lot's first update saw no write to
//!   the table; the one write to the vector was the lot load clearing it
//!   (0x428110, `erase(begin, end)` at 0x407161) before rebuilding it
//!   from the empty table. Nothing that fills +0x110 ran, so the vector
//!   is empty here ([`NEARBY_LOTS`]) and mode 11 always takes the false
//!   branch.
//!
//! [`World::order`]: sims_sim::World::order
//! [`World::people`]: sims_sim::World::people

use crate::factory::word::GROUP;
use crate::fault::Trap;
use crate::interpreter::Machine;
use sims_sim::Object;

/// The neighborhood's other lots, nearest first (neighborhood +0x11c),
/// for mode 11: empty in every capture of the original (module doc).
const NEARBY_LOTS: &[i32] = &[];

/// Mode 11's walk of the lot numbers `order` (0x44f08d): a cursor of 0
/// gives the first, any other the one after it, cut to a word. A cursor
/// that is not there, or the last, gives none.
fn next_lot(order: &[i32], current: i16) -> Option<i16> {
    let at = if current == 0 {
        0
    } else {
        order.iter().position(|&lot| lot == i32::from(current))? + 1
    };
    order.get(at).map(|&lot| lot as i16)
}

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
                    .position(|career| career.id == i32::from(current))
                    .map_or(0, |i| i + 1);
                let Some(next) = careers.get(start) else {
                    return Ok(false);
                };
                self.store(destination, next.id as i16)?;
                return Ok(true);
            }
            11 => next_lot(NEARBY_LOTS, current),
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
                if crate::interpreter::object_definition(self.catalog, self.world, record.guid)
                    .is_none()
                {
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
        let ids: &[u16] = if mode == 1 {
            if let Some(id) = self.making_person {
                return Err(Trap::Unsupported(format!(
                    "a walk of the person vector while the factory makes person {id}; when in \
                     the making the original appends a person is not read"
                )));
            }
            // Validate every entry of the small person vector upfront (matching
            // `person_order`), without allocating a temporary `Vec`.
            for &raw_id in &self.world.people {
                let object = self.world.objects.get(&raw_id).ok_or_else(|| {
                    Trap::Invalid(format!(
                        "person vector names object {raw_id}, which is gone"
                    ))
                })?;
                if object.person.is_none() {
                    return Err(Trap::Invalid(format!(
                        "person vector names object {raw_id}, which is not a person"
                    )));
                }
                i16::try_from(raw_id).map_err(|_| {
                    Trap::Invalid(format!("object ID {raw_id} does not fit a script word"))
                })?;
            }
            &self.world.people
        } else {
            &self.world.order
        };
        let remaining = match current {
            0 => ids,
            _ => match u16::try_from(current)
                .ok()
                .and_then(|cur| ids.iter().position(|&id| id == cur))
            {
                Some(i) => &ids[i + 1..],
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
        for &raw_id in remaining {
            let id = i16::try_from(raw_id).map_err(|_| {
                Trap::Invalid(format!("object ID {raw_id} does not fit a script word"))
            })?;
            if mode == 1 {
                return Ok(Some(id));
            }
            let object = self.world.objects.get(&raw_id).ok_or_else(|| {
                Trap::Invalid(format!("registered object {raw_id} does not exist"))
            })?;
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

    /// Mode 3 (0x44ec79): the tile after `current` in its multi-tile
    /// group, or zero.
    ///
    /// The original looks the cursor up (0x40743b); no object, or one
    /// whose definition word 10 ([`GROUP`], +0x118 +0x14) is 0, gives
    /// zero (0x44ecc9). Otherwise it casts the object to a tile (0x6c0ef8)
    /// and takes the tile's next link (+0x18c, [`World::next_parts`]),
    /// or, when that is null, its lead (+0x190, [`Multipart::parent`],
    /// null for the lead itself). Both null gives zero.
    ///
    /// The list runs from the lead through its tiles in the order they
    /// joined it: the file's order for a loaded group, the order they were
    /// made for one made at run time ([`crate::factory`]), which is not
    /// [`World::order`], where each new object goes first. An object
    /// with word 10 set that is not a tile fails: the cast gives null and
    /// the original reads +0x18c through it (0x44ecaa).
    ///
    /// [`World::next_parts`]: sims_sim::World::next_parts
    /// [`World::order`]: sims_sim::World::order
    /// [`Multipart::parent`]: sims_formats::saved_objects::Multipart::parent
    fn next_tile(&self, current: i16) -> Result<i16, Trap> {
        let Some(id) = u16::try_from(current).ok().filter(|&id| id > 0) else {
            return Ok(0);
        };
        let Some(object) = self.world.objects.get(&id) else {
            return Ok(0);
        };
        if self.interpreter().words(self.world, id)?[GROUP] == 0 {
            return Ok(0);
        }
        let part = object.multipart.as_ref().ok_or_else(|| {
            Trap::Unsupported(format!(
                "Set To Next mode 3: object {id} has definition word 10 set but is no tile; \
                 the original reads its next link through a null cast (0x44ecaa)"
            ))
        })?;
        Ok(match self.world.next_parts.get(&id) {
            Some(&next) => next as i16,
            None => part.parent,
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

    /// The native person vector ([`sims_sim::World::people`]), which
    /// Set To Next and the search for a person in a route's way walk.
    pub(super) fn person_order(&self) -> Result<Vec<(i16, &Object)>, Trap> {
        if let Some(id) = self.making_person {
            return Err(Trap::Unsupported(format!(
                "a walk of the person vector while the factory makes person {id}; when in \
                 the making the original appends a person is not read"
            )));
        }
        self.world
            .people
            .iter()
            .map(|&id| {
                let object = self.world.objects.get(&id).ok_or_else(|| {
                    Trap::Invalid(format!("person vector names object {id}, which is gone"))
                })?;
                if object.person.is_none() {
                    return Err(Trap::Invalid(format!(
                        "person vector names object {id}, which is not a person"
                    )));
                }
                let id = i16::try_from(id).map_err(|_| {
                    Trap::Invalid(format!("object ID {id} does not fit a script word"))
                })?;
                Ok((id, object))
            })
            .collect()
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

#[cfg(test)]
mod tests {
    use super::next_lot;

    #[test]
    fn mode_11_walks_the_lots_from_the_cursor() {
        let order = [4, 2, 0x1_0007];
        assert_eq!(next_lot(&order, 0), Some(4));
        assert_eq!(next_lot(&order, 4), Some(2));
        // The low word of the 32-bit lot number is stored.
        assert_eq!(next_lot(&order, 2), Some(7));
        // The cursor is sign-extended before it is matched, so the low
        // word of a lot number over 0x7fff is not found.
        assert_eq!(next_lot(&order, 7), None);
        assert_eq!(next_lot(&order, 3), None);
        assert_eq!(next_lot(&[], 0), None);
        assert_eq!(next_lot(&[], 5), None);
        assert_eq!(next_lot(&[-1, 9], -1), Some(9));
    }
}
