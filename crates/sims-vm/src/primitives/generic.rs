//! Generic Sims Call (opcode 1): one of several dozen built-in routines,
//! picked by operand word 0.
//!
//! Ported from the old prototype's `generic_person_call`
//! (legacy/crates/sims-vm/src/person.rs:627). Only the routines scripts have
//! reached so far are here; the rest fail loudly with their number.
//!
//! - 28: classify a lot. Temp 0 holds a lot number and receives 0 for a
//!   residential lot, 1 for a community lot and -1 for a lot the zoning
//!   table does not know. Lot 0 and the Magic Town lots 90 to 92 are
//!   residential and 93 to 98 community whatever the table says.
//! - 10 and 11: add the running object to the lot's list of burning
//!   fires, or take it off (0x44fd29, 0x44fd3a). Ported from the old
//!   prototype's `fires::State` (legacy/crates/sims-vm/src/fires.rs).
//! - 43: raise the magic controller's attributes to the selected family's
//!   magic history (0x410af6). Ported from the old prototype's `restore`
//!   (legacy/crates/sims-vm/src/magic_history.rs). It does nothing unless
//!   the game holds a family's history, that family is the one in global
//!   9, and the lot in global 10 is lot 0 or zoned residential (the raw
//!   zoning table, without routine 28's fixed lots). Then it asks for the
//!   instances of the controller's type, which makes that type's
//!   instance list if it has none (0x445857), and each of the first
//!   instance's first six attributes becomes the larger of itself and the
//!   family's word.
//! - 15: the running object, which must be a person (else the original's
//!   cast throws, error 0x1c), copies temp 0 (object +0x3a) into person
//!   data word 72 and clears word 73 (0x44fe72 to 0x44fe7d). When temp 0
//!   is 0 its footprint is then worked out again where it stands
//!   (0x433d16, through 0x43519f), with word 73 already clear.
//! - 2 (0x44fb51): the stack object becomes the icon of the running
//!   person's action (person +0xa54), if the person has a running action
//!   (+0xa84 not 0). The stack object must be on the lot (native error
//!   0xa otherwise, checked first: the lot's object table at +0x18 of the
//!   person's lot, +0xdc), and the running object a person (error 0x1c).
//! - 12: how far the stack object's tile is from the back of the view
//!   (0x44fd4b). Temp 0 becomes minus the sum of the tile's two
//!   coordinates (object bytes +0xfc and +0xfd), each first turned by the
//!   house view's rotation (0x434f58) through the table 0x407704 builds:
//!   the coordinates (x, y) become (63 - y, x) at rotation 1, (63 - x,
//!   63 - y) at 2 and (y, 63 - x) at 3, and stay at 0. The sum is a
//!   16-bit word. A stack object that is not there throws native error
//!   0xa. Routine 12 takes the true branch.
//! - 6 (0x44fc53): take the neighbor the stack object names out of its
//!   family (0x42601e). See [`Machine::leave_family`].

use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use sims_sim::World;

impl Machine<'_, '_> {
    pub(crate) fn generic_call(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        match word(operands, 0) {
            28 => {
                let zoning = &self.program.zoning;
                let temps = &mut self.me_mut()?.temps;
                let lot = temps
                    .first_mut()
                    .ok_or_else(|| Trap::Invalid("object has no temps".into()))?;
                *lot = match *lot {
                    0 | 90..=92 => 0,
                    93..=98 => 1,
                    number => match zoning.get(&number) {
                        Some(1) => 0,
                        Some(2) => 1,
                        // A lot missing from the table reads as zone 0.
                        _ => -1,
                    },
                };
                Ok(true)
            }
            10 => {
                self.world.fires.push(self.me);
                Ok(true)
            }
            11 => {
                let me = self.me;
                if let Some(place) = self.world.fires.iter().position(|&id| id == me) {
                    self.world.fires.remove(place);
                }
                Ok(true)
            }
            2 => {
                self.set_action_icon()?;
                Ok(true)
            }
            12 => {
                let depth = self.tile_depth()?;
                *self
                    .me_mut()?
                    .temps
                    .first_mut()
                    .ok_or_else(|| Trap::Invalid("object has no temps".into()))? = depth;
                Ok(true)
            }
            15 => {
                self.copy_temp_to_person()?;
                Ok(true)
            }
            43 => {
                self.restore_magic_history();
                Ok(true)
            }
            6 => self.leave_family(),
            routine => Err(Trap::Unsupported(format!(
                "Generic Sims Call routine {routine}"
            ))),
        }
    }
}

impl Machine<'_, '_> {
    /// Routine 6 (0x44fc53): the neighbor the stack object names, by its
    /// record's place plus one (0x4256f4), leaves its family (0x42601e).
    /// No such record throws native error 0x37.
    ///
    /// Before that the routine sends the interface's top window (+0x2c of
    /// the object `[[0x6e48bc]+0x48]` returns from its method 0x18,
    /// 0x4b4491) the Escape key ({3, 0x10, 0x1b, 0} to its method 0) if the
    /// window manager still holds it (0x5da49f). That is taken to be
    /// presentation. Here no modal dialog is open while scripts run (see
    /// `dialog.rs`), so it could close nothing the simulation keeps
    /// (**unverified**).
    ///
    /// Then 0x42601e, which fails (-1, the false branch) when the record's
    /// family word (person data word 61, +0xee) is 0, when no family has
    /// that ID (the first in the neighborhood's order, +0x154, whose +0x10c
    /// matches), or when the record's GUID (+4, 0x5ed65e) is not one of the
    /// family's members (0x4101d0). Otherwise, in order:
    ///
    /// 1. The first person in the person vector (lot +0x84) whose type has
    ///    that GUID (0x44093d) gets family word 61 (+0x606) set to 0.
    /// 2. The record's word 61 becomes 0.
    /// 3. The first member with that GUID is taken out (0x410884).
    /// 4. Who is a visitor is worked out again (0x42a2b4,
    ///    [`crate::person_factory::visitor_pass`]).
    /// 5. The family's flag 8 (+0x138) is cleared. The World does not hold
    ///    the family's flags; a script that reads them fails loudly
    ///    (`memory.rs`).
    /// 6. The family's friend count is out of date (+0x124 = 1).
    /// 7. Signal 0xe8 goes to the game's eight subscribers (0x40760d). Of
    ///    their handlers (0x41637f, 0x43c0e4, 0x44302e, 0x46d2c2, 0x47b22a,
    ///    0x4a82db, 0x4c3ea5, 0x4f65bf) only the interface's has a case for
    ///    it: it marks every part of the control panel for redrawing
    ///    (0x4f6820, 0x4ceceb), which is presentation.
    ///
    /// Then the true branch (0x451000).
    fn leave_family(&mut self) -> Result<bool, Trap> {
        use sims_sim::neighbors::FAMILY;
        let id = self.frame()?.stack_object;
        let record = self
            .world
            .neighbors
            .get(id)
            .ok_or(Trap::native(0x37, "the stack object names no neighbor"))?;
        let guid = record.guid;
        let family = i32::from(record.data[FAMILY]);
        if family == 0 {
            return Ok(false);
        }
        let Some(place) = self.world.families.iter().position(|f| f.id == family) else {
            return Ok(false);
        };
        let Some(member) = self.world.families[place]
            .members
            .iter()
            .position(|&m| m == guid)
        else {
            return Ok(false);
        };
        let live = self.world.people.iter().copied().find(|person| {
            self.world
                .objects
                .get(person)
                .is_some_and(|o| o.person.is_some() && o.guid == guid)
        });
        if let Some(person) = live
            && let Some(data) = self
                .world
                .objects
                .get_mut(&person)
                .and_then(|o| o.person.as_mut())
            && let Some(word) = data.data.get_mut(FAMILY)
        {
            *word = 0;
        }
        if let Some(record) = self.world.neighbors.get_mut(id) {
            record.data[FAMILY] = 0;
        }
        let family = &mut self.world.families[place];
        family.members.remove(member);
        crate::person_factory::visitor_pass(self.program, self.world, self.active_family)?;
        self.world.families[place].stale = true;
        Ok(true)
    }

    /// Routine 2 (0x44fb51 to 0x44fbd8).
    fn set_action_icon(&mut self) -> Result<(), Trap> {
        let stack_object = self.frame()?.stack_object;
        if stack_object <= 0 || !self.world.objects.contains_key(&(stack_object as u16)) {
            return Err(Trap::native(0xa, "the stack object is not on the lot"));
        }
        let action = &mut self
            .me_mut()?
            .person
            .as_mut()
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .active;
        if action.uid != 0 {
            action.icon = stack_object;
        }
        Ok(())
    }

    /// Routine 12's value (0x44fd4b): minus the sum of the stack object's
    /// tile coordinates as the view's rotation turns them.
    fn tile_depth(&self) -> Result<i16, Trap> {
        let target = self.frame()?.stack_object;
        let object = u16::try_from(target)
            .ok()
            .filter(|&id| id > 0)
            .and_then(|id| self.world.objects.get(&id))
            .ok_or_else(|| Trap::native(0xa, "Generic Sims Call 12 has no stack object"))?;
        let [x, y, _] = World::tile(object.position);
        let [x, y] = turn_tile([x, y], self.view_rotation)?;
        Ok((i16::from(x) + i16::from(y)).wrapping_neg())
    }
}

/// A tile's coordinates as the house view at `rotation` sees them
/// (0x434f58): the table at 0x6de86c that 0x407704 builds, 64 by 64 for
/// each of rotations 1 to 3; rotation 0 leaves them. The original reads a
/// coordinate outside 0 to 63 from outside the rotation's table, so that
/// fails.
pub(crate) fn turn_tile([x, y]: [i8; 2], rotation: u8) -> Result<[i8; 2], Trap> {
    if rotation == 0 {
        return Ok([x, y]);
    }
    if !(0..64).contains(&x) || !(0..64).contains(&y) {
        return Err(Trap::Unsupported(format!(
            "turning tile ({x}, {y}) by the view's rotation {rotation} (0x434f58) reads outside \
             its table, which is not ported"
        )));
    }
    match rotation {
        1 => Ok([63 - y, x]),
        2 => Ok([63 - x, 63 - y]),
        3 => Ok([y, 63 - x]),
        _ => Err(Trap::Invalid(format!("the view's rotation {rotation}"))),
    }
}

/// Person data words routine 15 sets (person +0x61c, +0x61e).
const COPIED: usize = 72;
const CLEARED: usize = 73;

impl Machine<'_, '_> {
    /// Routine 15 (0x44fe3c). The footprint is worked out before anything
    /// changes, from the person's data words as they will be, so that a
    /// footprint that is not ported leaves the person as it was.
    fn copy_temp_to_person(&mut self) -> Result<(), Trap> {
        let me = self.me;
        let object = self.me()?;
        let temp = *object
            .temps
            .first()
            .ok_or_else(|| Trap::Invalid("object has no temps".into()))?;
        let mut data = object
            .person
            .as_ref()
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .data
            .clone();
        for (word, value) in [(COPIED, temp), (CLEARED, 0)] {
            *data
                .get_mut(word)
                .ok_or_else(|| Trap::Invalid(format!("a person has no data word {word}")))? = value;
        }
        let footprint = if temp == 0 {
            Some(
                self.interpreter()
                    .standing_footprint(self.world, me, Some(&data))?,
            )
        } else {
            None
        };
        let object = self.me_mut()?;
        if let Some(footprint) = footprint {
            object.footprint = footprint;
        }
        if let Some(person) = object.person.as_mut() {
            person.data = data;
        }
        Ok(())
    }
}

/// The type of the object that keeps a lot's magic state.
const MAGIC_CONTROLLER: u32 = 0xb6c9_0029;

impl Machine<'_, '_> {
    fn restore_magic_history(&mut self) {
        let world = &mut *self.world;
        let Some(history) = &world.magic_history else {
            return;
        };
        let (family, lot) = (world.globals[9], world.globals[10]);
        if family == 0 || history.family == 0 || i32::from(family) != history.family {
            return;
        }
        if lot != 0 && self.program.zoning.get(&lot) != Some(&1) {
            return;
        }
        let values = history.values;
        let objects = &world.objects;
        let instances = world.instances.entry(MAGIC_CONTROLLER).or_insert_with(|| {
            objects
                .iter()
                .filter(|(_, object)| object.guid == MAGIC_CONTROLLER)
                .map(|(&id, _)| id)
                .collect()
        });
        let Some(controller) = instances.first().and_then(|id| world.objects.get_mut(id)) else {
            return;
        };
        for (attribute, value) in controller.attributes.iter_mut().zip(values) {
            *attribute = (*attribute).max(value);
        }
    }
}
