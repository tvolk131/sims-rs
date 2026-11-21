//! Manage Inventory (opcode 51, 0x4552f8): look up a token in a person's
//! inventory, or walk it.
//!
//! Operands: the operation (byte 0); the token kind (byte 1, unsigned);
//! flags (bytes 2 and 3); and the token's GUID (bytes 4 to 7), or zero for
//! the stack object's.
//!
//! Every operation first finds the inventory:
//!
//! 1. The owner is the caller, or with flag 0x2000 the object the caller's
//!    temp 4 names. An owner that is not a person, or not there, is
//!    native error 0x1c.
//! 2. The owner's record is the first whose linked definition has the
//!    owner's GUID (0x42571f).
//! 3. The inventory is the first in the neighborhood's list with that
//!    record's ID; if there is none, an empty one is added at the end
//!    (0x42a8a1).
//!
//! Operation 0 (0x4556f4) adds tokens: the count is 1, or with flag 0x2
//! the caller's temp `(flags >> 2) & 3`. A count of 0 adds nothing. The
//! first token of that kind and GUID (0x41f721) takes the count added to
//! its own, both read as signed words, and 0x7fff when the sum is that or
//! more (0x41f3f6); without one, a token of that kind, GUID and count is
//! added at the end (0x41f7c8), unless the inventory already holds 0x7fff
//! tokens, which is the false branch (0x41f3cb). Otherwise the branch is
//! true (0x41f38d).
//!
//! Operation 3 (0x455579) finds the first token of that kind and GUID
//! (0x41f660, 0x41f721). None, or a count of 0, is the false branch with
//! nothing written. Otherwise the caller's temp `(flags >> 5) & 3` takes
//! the count and then temp `(flags >> 11) & 3` the token's place in the
//! inventory, and the branch is true.
//!
//! Operation 4 (0x455462) walks the inventory: from the place after the
//! one in the caller's temp `(flags >> 8) & 3`, it finds the next token of
//! that kind, and of that GUID too with flag 0x1, any GUID without
//! (0x41f69c). Temp `(flags >> 5) & 3` takes the token's count, and then
//! temp `(flags >> 11) & 3` its place. When there is none, they take 0
//! and -1 and the branch is false; a count of 0 is still found. A place
//! after 0x7fff wraps to a negative one, which finds nothing (0x41f746).
//! The branch tests the place written, so a token found at index 65535,
//! whose place is cut to -1 like every place (both operations store the
//! index's low word), is false with its count written.
//!
//! Operations 1, 2, 5, 6 and above fail loudly. Ported from the original's code
//! (above) and checked against the old prototype's `inventory`
//! (legacy/crates/sims-vm/src/person.rs), whose operation 3 agrees.
//!
//! Not modeled: a successful operation also sets the interface's
//! inventory flag (0x4ceceb with 0x4000000), which is presentation.

use crate::fault::Trap;
use crate::interpreter::Machine;
use sims_sim::neighbors::{Inventory, Item};

/// Flag 0x2000: the owner is the object the caller's temp 4 names.
const OWNER_IN_TEMP_4: u16 = 0x2000;
/// Flag 0x1: operation 4 finds only tokens of the GUID too.
const MATCH_GUID: u16 = 0x1;
/// Flag 0x2: operation 0's count is in the caller's temp
/// `(flags >> 2) & 3`.
const COUNT_IN_TEMP: u16 = 0x2;
/// The most tokens, and the greatest count, an inventory holds.
const MOST: i32 = 0x7fff;

impl Machine<'_, '_> {
    pub(crate) fn manage_inventory(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let operation = operands[0];
        if !matches!(operation, 0 | 3 | 4) {
            return Err(Trap::Unsupported(format!(
                "Manage Inventory operation {operation} is not ported"
            )));
        }
        let kind = i32::from(operands[1]);
        let flags = u16::from_le_bytes([operands[2], operands[3]]);
        let me = self.me()?;
        let owner = if flags & OWNER_IN_TEMP_4 != 0 {
            u16::try_from(me.temps[4]).ok()
        } else {
            Some(self.me)
        };
        let owner = owner
            .and_then(|id| self.world.objects.get(&id))
            .filter(|o| o.person.is_some())
            .ok_or(Trap::native(0x1c, "Manage Inventory owner is not a person"))?
            .guid;
        let neighbor = self.linked_record(owner)?.ok_or_else(|| {
            Trap::Unsupported(format!(
                "Manage Inventory: the person type 0x{owner:08X} has no neighborhood record, \
                 which the original does not expect"
            ))
        })?;
        let mut guid = u32::from_le_bytes([operands[4], operands[5], operands[6], operands[7]]);
        if guid == 0 {
            let stack_object = self.frame()?.stack_object;
            guid = u16::try_from(stack_object)
                .ok()
                .and_then(|id| self.world.objects.get(&id))
                .ok_or_else(|| {
                    Trap::Unsupported(
                        "Manage Inventory names the stack object's GUID, and there is no stack \
                         object"
                            .into(),
                    )
                })?
                .guid;
        }
        if operation == 0 {
            let place = self.inventory_slot(neighbor);
            let temps = &self
                .world
                .objects
                .get(&self.me)
                .ok_or(Trap::native(10, "missing object"))?
                .temps;
            let count = if flags & COUNT_IN_TEMP != 0 {
                temps[usize::from((flags >> 2) & 3)]
            } else {
                1
            };
            if count == 0 {
                return Ok(true);
            }
            let items = &mut self.world.neighbors.inventories[place].items;
            match items.iter_mut().find(|i| i.kind == kind && i.guid == guid) {
                Some(item) => {
                    let sum = i32::from(item.count as i16) + i32::from(count);
                    item.count = if sum >= MOST { MOST as u16 } else { sum as u16 };
                }
                None => {
                    if items.len() >= MOST as usize {
                        return Ok(false);
                    }
                    items.push(Item {
                        kind,
                        guid,
                        count: count as u16,
                    });
                }
            }
            return Ok(true);
        }
        // The place is the token's index cut to a word, with no check
        // (0x41f677 to 0x41f684, 0x41f6d5 to 0x41f6e1).
        let place_word = |index: usize| index as i16;
        if operation == 3 {
            let Some((index, count)) = self
                .find_inventory_token(neighbor, kind, guid)
                .filter(|&(_, count)| count != 0)
            else {
                return Ok(false);
            };
            let temps = &mut self
                .world
                .objects
                .get_mut(&self.me)
                .ok_or(Trap::native(10, "missing object"))?
                .temps;
            temps[usize::from((flags >> 5) & 3)] = count as i16;
            temps[usize::from((flags >> 11) & 3)] = place_word(index);
            return Ok(true);
        }
        // Operation 4: the next token after the place in a temp.
        let place = self.inventory_slot(neighbor);
        let items = &self.world.neighbors.inventories[place].items;
        let temps = &mut self
            .world
            .objects
            .get_mut(&self.me)
            .ok_or(Trap::native(10, "missing object"))?
            .temps;
        let guid = if flags & MATCH_GUID != 0 { guid } else { 0 };
        let after = temps[usize::from((flags >> 8) & 3)];
        let start = after.wrapping_add(1);
        let found = usize::try_from(start).ok().and_then(|start| {
            items
                .iter()
                .enumerate()
                .skip(start)
                .find(|(_, item)| item.kind == kind && (guid == 0 || item.guid == guid))
                .map(|(index, item)| (index, item.count))
        });
        let (index, count) = match found {
            Some((index, count)) => (place_word(index), count as i16),
            None => (-1, 0),
        };
        temps[usize::from((flags >> 5) & 3)] = count;
        temps[usize::from((flags >> 11) & 3)] = index;
        Ok(index != -1)
    }

    /// The index in [`sims_sim::neighbors::Neighbors::inventories`] of
    /// `neighbor`'s inventory, adding an empty one at the end when there is
    /// none (0x42a8a1).
    pub(crate) fn inventory_slot(&mut self, neighbor: i16) -> usize {
        let inventories = &mut self.world.neighbors.inventories;
        match inventories.iter().position(|i| i.neighbor == neighbor) {
            Some(place) => place,
            None => {
                inventories.push(Inventory {
                    neighbor,
                    items: Vec::new(),
                });
                inventories.len() - 1
            }
        }
    }

    /// The index and count of the first token of `kind` and `guid` in
    /// `neighbor`'s inventory (0x42a8a1, 0x41f660, 0x41f721), adding an
    /// empty inventory at the end when the neighbor has none.
    pub(crate) fn find_inventory_token(
        &mut self,
        neighbor: i16,
        kind: i32,
        guid: u32,
    ) -> Option<(usize, u16)> {
        let place = self.inventory_slot(neighbor);
        self.world.neighbors.inventories[place]
            .items
            .iter()
            .enumerate()
            .find(|(_, item)| item.kind == kind && item.guid == guid)
            .map(|(index, item)| (index, item.count))
    }

    /// The ID of the first neighborhood record whose linked definition has
    /// the GUID `guid` (0x42571f), or None. The original compares the GUID
    /// of the definition each record is linked to, which is the definition
    /// of the record's own GUID; a record whose GUID has no definition
    /// fails, as what the original compares then was not seen.
    pub(crate) fn linked_record(&self, guid: u32) -> Result<Option<i16>, Trap> {
        if guid == 0 {
            return Ok(None);
        }
        for record in self.world.neighbors.slots.iter().flatten() {
            if crate::interpreter::object_definition(self.catalog, self.world, record.guid)
                .is_none()
            {
                return Err(Trap::Unsupported(format!(
                    "neighbor {} has the GUID 0x{:08X}, which no definition has; the original \
                     compares the GUID of the definition the record is linked to",
                    record.id, record.guid
                )));
            }
            if record.guid == guid {
                return Ok(Some(record.id));
            }
        }
        Ok(None)
    }
}
