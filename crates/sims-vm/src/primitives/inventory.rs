//! Manage Inventory (opcode 51, 0x4552f8): add, remove, look up, walk
//! or replace tokens in a person's inventory.
//!
//! Operands: the operation (byte 0); the token kind (byte 1, unsigned);
//! flags (bytes 2 and 3); and the token's GUID (bytes 4 to 7), or zero for
//! the stack object's.
//!
//! Every operation first runs the common setup:
//!
//! 1. The owner is the caller, or with flag 0x2000 the object the caller's
//!    temp 4 names. An owner that is not a person, or not there, is
//!    native error 0x1c.
//! 2. The owner's record is the first whose GUID (+4, 0x5ed65e) matches
//!    the owner's GUID (0x42571f); `linked_record` also guards against a
//!    record with no registered definition.
//! 3. The inventory is the first in the neighborhood's list with that
//!    record's ID; if there is none, an empty one is added at the end
//!    (0x42a8a1, at 0x455386).
//! 4. When the operand GUID is 0, it is replaced by the stack object's
//!    GUID (0x455397, after 0x42a8a1), even for operations that ignore
//!    the GUID.
//!
//! Operations:
//!
//! - Operation 0 (0x4556f4) adds tokens: the count is 1, or with flag 0x2
//!   the caller's temp `(flags >> 2) & 3`. A count of 0 adds nothing. The
//!   first token of that kind and GUID (0x41f721) takes the count added to
//!   its own, both read as signed words, and 0x7fff when the sum is that
//!   or more (0x41f3f6); without one, a token of that kind, GUID and count
//!   is added at the end (0x41f7c8), unless the inventory already holds
//!   0x7fff tokens, which is the false branch (0x41f3cb). Otherwise the
//!   branch is true (0x41f38d).
//! - Operation 1 (0x45569d) removes from the first token of that kind and
//!   GUID (0x41f427): the count is 1, or with flag 0x2 the caller's temp
//!   `(flags >> 2) & 3`. A count of 0 returns true without searching. With
//!   no matching token, the branch is false. Otherwise the original always
//!   subtracts 1 from the token's signed count (0x41f447 pushes 1 into
//!   0x41f471, clamping results of 0 or less to 0) and erases the token
//!   (0x59b11c) when its new count is 0 or the requested count is -1. The
//!   branch is true.
//! - Operation 2 (0x45560c) removes from the token at the index in the
//!   caller's temp `(flags >> 8) & 3` (0x41f56b): the count is 1, or with
//!   flag 0x2 the caller's temp `(flags >> 2) & 3`. A count of 0 returns
//!   true without checking the index. A negative index or one past the end
//!   returns false. Otherwise a negative count erases that token; a
//!   positive count subtracts from its signed count and erases it if the
//!   difference is 0 or less. The branch is true.
//! - Operation 3 (0x455579) finds the first token of that kind and GUID
//!   (0x41f660, 0x41f721). None, or a count of 0, is the false branch with
//!   nothing written. Otherwise the caller's temp `(flags >> 5) & 3` takes
//!   the count and then temp `(flags >> 11) & 3` the token's place in the
//!   inventory, and the branch is true.
//! - Operation 4 (0x455462) walks the inventory: from the place after the
//!   one in the caller's temp `(flags >> 8) & 3`, it finds the next token
//!   of that kind, and of that GUID too with flag 0x1, any GUID without
//!   (0x41f69c). Temp `(flags >> 5) & 3` takes the token's count, and then
//!   temp `(flags >> 11) & 3` its place. When there is none, they take 0
//!   and -1 and the branch is false; a count of 0 is still found. A place
//!   after 0x7fff wraps to a negative one, which finds nothing (0x41f746).
//!   The branch tests the place written, so a token found at index 65535,
//!   whose place is cut to -1 like every place (both operations store the
//!   index's low word), is false with its count written.
//! - Operations 5 (0x455437) and 6 (0x4553e8) look up the neighbor record
//!   whose 1-based slot is in the caller's temp 0 (0x4256f4) and read its
//!   GUID (0x5ed65e, record +4). They erase every token of kind 1 (op 5)
//!   or kind 3 (op 6) (0x41f4e5), add 1 token of that kind and GUID
//!   through 0x41f38d (leaving the list unchanged if at least 0x7fff
//!   tokens remain after clearing that kind), ignore the add's return
//!   value, and take the true branch (0x455418). A missing record in temp
//!   0 fails loudly, as the original dereferences null at 0x45544b /
//!   0x4553fc.
//! - Operations 7 and above (0x4553e6) take the true branch after the
//!   common setup (0x455418).
//!
//! Ported from the original's code (above) and checked against the old
//! prototype's `inventory` (legacy/crates/sims-vm/src/person.rs), whose
//! operation 3 agrees.
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
/// Flag 0x2: operations 0, 1 and 2 read the count from the caller's temp
/// `(flags >> 2) & 3`.
const COUNT_IN_TEMP: u16 = 0x2;
/// The most tokens, and the greatest count, an inventory holds.
const MOST: i32 = 0x7fff;

impl Machine<'_, '_> {
    pub(crate) fn manage_inventory(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let operation = operands[0];
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
        // 0x455386: get or create the owner's inventory before resolving a
        // zero operand GUID from the stack object (0x455397).
        let inventories = &mut self.world.neighbors.inventories;
        let place = match inventories.iter().position(|i| i.neighbor == neighbor) {
            Some(place) => place,
            None => {
                inventories.push(Inventory {
                    neighbor,
                    items: Vec::new(),
                });
                inventories.len() - 1
            }
        };
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
        let read_count = |temps: &[i16]| {
            if flags & COUNT_IN_TEMP != 0 {
                temps[usize::from((flags >> 2) & 3)]
            } else {
                1
            }
        };
        match operation {
            0 => {
                let count = read_count(&self.me()?.temps);
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
                Ok(true)
            }
            1 => {
                // 0x45569d → 0x41f427: a count of 0 returns true without
                // searching; otherwise 0x41f447 always subtracts 1 (via
                // 0x41f471) and erases the token if the new count is 0 or
                // the requested count was -1.
                let count = read_count(&self.me()?.temps);
                if count == 0 {
                    return Ok(true);
                }
                let items = &mut self.world.neighbors.inventories[place].items;
                let Some(index) = items.iter().position(|i| i.kind == kind && i.guid == guid)
                else {
                    return Ok(false);
                };
                let remaining = i32::from(items[index].count as i16) - 1;
                if remaining <= 0 || count == -1 {
                    items.remove(index);
                } else {
                    items[index].count = remaining as u16;
                }
                Ok(true)
            }
            2 => {
                // 0x45560c → 0x41f56b: remove from the token at the index
                // in temp (flags >> 8) & 3. A count of 0 succeeds before
                // checking the index; a negative count removes the token.
                let temps = &self.me()?.temps;
                let count = read_count(temps);
                let index = temps[usize::from((flags >> 8) & 3)];
                if count == 0 {
                    return Ok(true);
                }
                let items = &mut self.world.neighbors.inventories[place].items;
                let Ok(index) = usize::try_from(index) else {
                    return Ok(false);
                };
                if index >= items.len() {
                    return Ok(false);
                }
                if count < 0 {
                    items.remove(index);
                } else {
                    let remaining = i32::from(items[index].count as i16) - i32::from(count);
                    if remaining > 0 {
                        items[index].count = remaining as u16;
                    } else {
                        items.remove(index);
                    }
                }
                Ok(true)
            }
            3 => {
                let items = &self.world.neighbors.inventories[place].items;
                let temps = &mut self
                    .world
                    .objects
                    .get_mut(&self.me)
                    .ok_or(Trap::native(10, "missing object"))?
                    .temps;
                // The place is the token's index cut to a word, with no
                // check (0x41f677 to 0x41f684).
                let found = items
                    .iter()
                    .enumerate()
                    .find(|(_, item)| item.kind == kind && item.guid == guid)
                    .map(|(index, item)| (index, item.count));
                let Some((index, count)) = found.filter(|&(_, count)| count != 0) else {
                    return Ok(false);
                };
                temps[usize::from((flags >> 5) & 3)] = count as i16;
                temps[usize::from((flags >> 11) & 3)] = index as i16;
                Ok(true)
            }
            4 => {
                // The next token after the place in a temp. The place is
                // cut to a word, with no check (0x41f6d5 to 0x41f6e1).
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
                    Some((index, count)) => (index as i16, count as i16),
                    None => (-1, 0),
                };
                temps[usize::from((flags >> 5) & 3)] = count;
                temps[usize::from((flags >> 11) & 3)] = index;
                Ok(index != -1)
            }
            5 | 6 => {
                // 0x455437 (op 5, kind 1) and 0x4553e8 (op 6, kind 3):
                // replace all tokens of that kind with 1 token carrying
                // the GUID of the neighbor record in the caller's temp 0
                // (0x4256f4, 0x5ed65e), then return true regardless of
                // 0x41f38d's capacity check (0x455418).
                let slot = self.me()?.temps[0];
                let at = if operation == 5 {
                    "0x45544b"
                } else {
                    "0x4553fc"
                };
                let target_guid = self
                    .world
                    .neighbors
                    .get(slot)
                    .ok_or_else(|| {
                        Trap::Unsupported(format!(
                            "Manage Inventory operation {operation}: temp 0 names neighbor \
                             slot {slot}, which has no record (original dereferences null at {at})"
                        ))
                    })?
                    .guid;
                let token_kind = if operation == 5 { 1 } else { 3 };
                let items = &mut self.world.neighbors.inventories[place].items;
                items.retain(|i| i.kind != token_kind);
                if items.len() < MOST as usize {
                    items.push(Item {
                        kind: token_kind,
                        guid: target_guid,
                        count: 1,
                    });
                }
                Ok(true)
            }
            _ => {
                // 0x4553e6 → 0x455418: unknown operations take the true
                // branch after the common setup.
                Ok(true)
            }
        }
    }

    /// The ID of the first neighborhood record whose GUID (+4, 0x5ed65e)
    /// matches `guid` (0x42571f), or None. As a port guard, a record whose
    /// GUID has no registered definition fails loudly; normal neighborhood
    /// loading already rejects such records.
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
