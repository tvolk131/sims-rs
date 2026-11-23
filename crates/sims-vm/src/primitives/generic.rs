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
//! - 14 (0x44fe22): write `temp[0]` (`object +0x3a`) into global 31 (`0x1f`,
//!   via `0x46d9c4`, used by stereos/boom boxes to track the active station)
//!   and take the true branch.
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
//! - 18 (0x4506fb): write the object ID of the controlled family's first
//!   member (`0x4e97dc(0)` -> `0x4e9608`), or -1 if not on the lot, into
//!   temp 0. Leaves temp 0 untouched when `0x4e97dc(0)` returns -1. Always
//!   takes the true branch.
//! - 26 (0x450732): report the controlled family (game +0x10) in temps 0
//!   and 1. If `0x4e97dc(0)` returns -1 (no first member, or its GUID is not
//!   in the portrait table), temps are left untouched. Otherwise, when the
//!   controlled family's ID (`+0x10c`) is nonzero, temp 0 becomes that ID and
//!   temp 1 becomes 1; when its ID is 0, temp 0 becomes the first on-lot
//!   person ID matching member 0 (`0x4e9608`) and temp 1 becomes 0 (or fails
//!   with `Trap::Unsupported` if member 0 is off-lot, where `0x450764`
//!   dereferences null). Takes the true branch when it succeeds.
//! - 21 (0x45098a): restore the saved state of the person whose object ID
//!   is in the running object's temp 0 from that person's neighbor
//!   inventory. See [`Machine::restore_saved_state`]. Magic Town's middle
//!   ped marker (tree 0x100e, node 35) runs it after it places each member
//!   of an arriving family.
//! - 19 (0x450787): bring back the person who traveled with the person
//!   whose object ID is in the running object's temp 0: temp 0 becomes the
//!   companion's object ID, or -1. See [`Machine::find_companion`].
//!   Downtown's middle ped marker (EP3 `PedMarkers.iff` tree 0x100e,
//!   node 11) runs it for the arriving family's first member.
//! - 3 (0x44fb41): take down the interface's travel picture, if it shows
//!   one (0x4fc3ca on the interface, `[0x6f2a58]`). That picture is the
//!   full-screen window 0x4fc416 opens: a bitmap picked by kind and screen
//!   width (interface +0x78) in a window (+0x7c), with the game's
//!   `[0x6e48bc]+0x2c` word +0x60 set to 15. Routine 3 removes the window
//!   from the window manager (0x5da5ce), clears +0x7c and that word, and
//!   frees the bitmap (0x5e369b), then takes the true branch. It is
//!   presentation; the simulation keeps no such window, so here routine 3
//!   only takes the true branch. The middle ped marker runs it once an
//!   arriving family is placed (tree 0x100e, node 36). What else reads
//!   +0x7c (the pause's refusal, `sims_sim::World::pause`; Dialog's
//!   modal check, `dialog.rs`) is not modeled either.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use sims_core::F32;
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
            14 => {
                // `0x44fe22..0x44fe37`: write `temp[0]` (`object +0x3a`) into
                // global 31 (`0x1f`) via `0x46d9c4`, then take the true branch.
                let value = *self
                    .me()?
                    .temps
                    .first()
                    .ok_or_else(|| Trap::Invalid("object has no temps".into()))?;
                *self
                    .world
                    .globals
                    .get_mut(31)
                    .ok_or_else(|| Trap::Invalid("world has no global 31".into()))? = value;
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
            18 => {
                self.controlled_family_head_object()?;
                Ok(true)
            }
            26 => {
                self.controlled_family_info()?;
                Ok(true)
            }
            21 => {
                self.restore_saved_state()?;
                Ok(true)
            }
            19 => {
                self.find_companion()?;
                Ok(true)
            }
            // Presentation only (see the module's notes).
            3 => Ok(true),
            35 => {
                // `0x451142..0x4511a1`: casts caller to cXPerson (native error
                // `0x1c` if not a person) and records it in registry `+0x10c`
                // for Studio Town lot picker (Dialog kind 11, `0x441f94`) star-limo
                // detection.
                if self.me()?.person.is_none() {
                    return Err(Trap::native(0x1c, "object is not a person"));
                }
                if let Some(panels) = self.special_panels {
                    panels.borrow_mut().studio_caller = Some(self.me);
                }
                Ok(true)
            }
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

    /// Resolves `(family_id, first_member_guid)` for the active/controlled
    /// family (`game +0x10`), returning `Ok((family_id, None))` when
    /// `0x4e97dc(0)` returns -1 (empty member list or member 0 not in the
    /// portrait table `0x4e9753`).
    fn controlled_family_first_member(&self, routine: u16) -> Result<(i16, Option<u32>), Trap> {
        let family_id = self.world.globals.get(9).copied().unwrap_or(0);
        let first_guid = if family_id == 0 {
            let Some(members) = self.active_family else {
                return Err(Trap::Unsupported(format!(
                    "Generic Sims Call {routine} with global 9 at 0: which family the game holds \
                     (game +0x10) is not kept"
                )));
            };
            members.first().copied()
        } else {
            let family = self
                .world
                .families
                .iter()
                .find(|f| f.id == i32::from(family_id))
                .ok_or_else(|| {
                    Trap::Invalid(format!("global 9 names family {family_id}, which is gone"))
                })?;
            family.members.first().copied()
        };
        let Some(guid) = first_guid else {
            return Ok((family_id, None));
        };
        if !self.in_portrait_table(guid)? {
            return Ok((family_id, None));
        }
        Ok((family_id, Some(guid)))
    }

    /// First on-lot person in `world.people` whose GUID matches `guid` (`0x4e9608`).
    fn first_on_lot_person(&self, guid: u32) -> Option<u16> {
        self.world
            .people
            .iter()
            .copied()
            .find(|id| self.world.objects.get(id).is_some_and(|o| o.guid == guid))
    }

    /// Routine 18 (0x4506fb): writes the controlled family's first member's
    /// on-lot object ID (`0x4e97dc(0)` -> `0x4e9608`), or -1 if absent, into
    /// temp 0. Leaves temp 0 untouched when `0x4e97dc(0)` returns -1.
    fn controlled_family_head_object(&mut self) -> Result<(), Trap> {
        let (_, Some(guid)) = self.controlled_family_first_member(18)? else {
            return Ok(());
        };
        let result = self.first_on_lot_person(guid).map_or(-1, |id| id as i16);
        *self
            .me_mut()?
            .temps
            .first_mut()
            .ok_or_else(|| Trap::Invalid("object has no temps".into()))? = result;
        Ok(())
    }

    /// Routine 26 (0x450732): writes the active/controlled family (`game +0x10`)
    /// into temps 0 and 1 when `0x4e97dc(0) != -1` (the family has a first
    /// member whose GUID is in the portrait table `0x4e9753`).
    fn controlled_family_info(&mut self) -> Result<(), Trap> {
        let (family_id, Some(guid)) = self.controlled_family_first_member(26)? else {
            return Ok(());
        };
        let (temp0, temp1) = if family_id == 0 {
            let person_id = self.first_on_lot_person(guid).ok_or_else(|| {
                Trap::Unsupported(format!(
                    "Generic Sims Call 26 (0x450764): active family 0's first member \
                     0x{guid:08X} is not on the lot, where the original dereferences null"
                ))
            })?;
            (person_id as i16, 0)
        } else {
            (family_id, 1)
        };
        let temps = &mut self.me_mut()?.temps;
        if temps.len() < 2 {
            return Err(Trap::Invalid(format!(
                "object has {} temps, need at least 2",
                temps.len()
            )));
        }
        temps[0] = temp0;
        temps[1] = temp1;
        Ok(())
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
        let [x, y] = turn_tile([x, y], self.effective_view_rotation())?;
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

/// The inventory token kind routine 21 restores from.
const SAVED_STATE: i32 = 2;

/// The inventory token kind routine 19 reads: the GUID of a person who
/// travels with the token's owner.
const COMPANION: i32 = 1;

/// Definition word 9, the object's type, and the type a person has.
const TYPE: usize = 9;
const PERSON_TYPE: i16 = 2;

/// Where routine 21 puts a token's count, by the token's GUID (the
/// switch at 0x450a53, through the table at 0x4514ea).
enum Restored {
    /// A motive, set to the count as a float (0x45d133: person +0x78c).
    Motive(usize),
    /// A person data word, set to the count.
    Word(usize),
}

/// GUIDs 0 to 6 name motives, 9 to 12 person data words (+0x59c, +0x62c,
/// +0x630, +0x62e); 7, 8 and anything above 12 name nothing.
fn restored(guid: u32) -> Option<Restored> {
    Some(match guid {
        0 => Restored::Motive(9),
        1 => Restored::Motive(6),
        2 => Restored::Motive(5),
        3 => Restored::Motive(15),
        4 => Restored::Motive(7),
        5 => Restored::Motive(8),
        6 => Restored::Motive(14),
        9 => Restored::Word(8),
        10 => Restored::Word(80),
        11 => Restored::Word(82),
        12 => Restored::Word(81),
        _ => return None,
    })
}

impl Machine<'_, '_> {
    /// The person whose object ID is in the running object's temp 0
    /// (+0x3a), and that person's neighbor ID (person data word 31,
    /// +0x5ca), as routines 19 and 21 read them. An ID that names no
    /// object, or an object that is not a person, is native error 0x1c
    /// (the casts at 0x4507bb and 0x4509be).
    fn temp_0_person(&self) -> Result<(u16, i16), Trap> {
        let temp = *self
            .me()?
            .temps
            .first()
            .ok_or_else(|| Trap::Invalid("object has no temps".into()))?;
        let not_person = || Trap::native(0x1c, "object is not a person");
        let person = u16::try_from(temp)
            .ok()
            .filter(|&id| id > 0)
            .filter(|id| {
                self.world
                    .objects
                    .get(id)
                    .is_some_and(|object| object.person.is_some())
            })
            .ok_or_else(not_person)?;
        let neighbor = self.world.objects[&person]
            .person
            .as_ref()
            .ok_or_else(not_person)?
            .data
            .get(31)
            .copied()
            .ok_or_else(|| Trap::Invalid("a person has no data word 31".into()))?;
        Ok((person, neighbor))
    }

    /// Writes `value` into the running object's temp 0.
    fn set_temp_0(&mut self, value: i16) -> Result<(), Trap> {
        *self
            .me_mut()?
            .temps
            .first_mut()
            .ok_or_else(|| Trap::Invalid("object has no temps".into()))? = value;
        Ok(())
    }

    /// Routine 19 (0x450787 to 0x450985).
    ///
    /// 1. The running object's temp 0 names a person ([`Self::temp_0_person`]),
    ///    and that person's neighbor inventory is found, or added empty
    ///    (0x42a8a1).
    /// 2. The first token of kind 1, any GUID, is found from the start
    ///    (0x41f69c). Its place is kept as a word; a negative one (none
    ///    found, or an index past 0x7fff that wraps negative) sets temp 0
    ///    to -1 and ends the routine there, with nothing erased and no
    ///    save (0x450cde). A place that wraps back to 0 or more names the
    ///    token there, whatever its kind.
    /// 3. The token's GUID names a person type ([`Self::person_of_type`]):
    ///    temp 0 becomes the object ID of the lot's first person of that
    ///    type, made if there is none, or -1 (0x45081c to 0x45086f).
    /// 4. One is taken from the token's count, and the token erased when
    ///    none is left (0x41f56b with 1).
    /// 5. The neighborhood is saved (0x4252cc). Here the neighborhood's
    ///    inventories are kept in the world and written when the lot is
    ///    saved, so the save is not modeled.
    ///
    /// The routine always ends with the true branch (0x44f7f0). Kind-1
    /// tokens are what Manage Inventory's operation 5 writes: the GUID of
    /// the neighbor a traveler takes along.
    fn find_companion(&mut self) -> Result<(), Trap> {
        let (_, neighbor) = self.temp_0_person()?;
        let inventory = self.inventory_slot(neighbor);
        let items = &self.world.neighbors.inventories[inventory].items;
        let place = items
            .iter()
            .position(|item| item.kind == COMPANION)
            .map_or(-1, |index| (index as u16) as i16);
        // A place of 0 or more is at or before the token's index, so it
        // is in the inventory (0x41f6fb reads it).
        let Ok(at) = usize::try_from(place) else {
            return self.set_temp_0(-1);
        };
        let guid = items[at].guid;
        let companion = self.person_of_type(guid)?;
        self.set_temp_0(companion.map_or(-1, |id| id as i16))?;
        // 0x41f56b(place, 1), on the inventory as it is now: making the
        // companion ran its scripts.
        let inventory = self.inventory_slot(neighbor);
        let items = &mut self.world.neighbors.inventories[inventory].items;
        if let Some(item) = items.get_mut(at) {
            let remaining = i32::from(item.count as i16) - 1;
            if remaining > 0 {
                item.count = remaining as u16;
            } else {
                items.remove(at);
            }
        }
        Ok(())
    }

    /// The lot's first person of the type `guid` names, made if there is
    /// none (routine 19, 0x450825 to 0x450866).
    ///
    /// - A type missing from the interface's table of person types
    ///   (0x4e9753 gives -1, [`Self::in_portrait_table`]) gives none, and
    ///   nothing is made.
    /// - Otherwise the lot's people are walked for the first of that type
    ///   (0x4e9608, [`Self::first_on_lot_person`]).
    /// - With none, the type's winning definition (0x43ac10) goes to the
    ///   object factory (0x440b2d, from 0x4e9719): a person, or for a
    ///   grouped or non-person definition the object it makes. The walk
    ///   then runs again, so a factory that made no person gives none.
    fn person_of_type(&mut self, guid: u32) -> Result<Option<u16>, Trap> {
        if !self.in_portrait_table(guid)? {
            return Ok(None);
        }
        if let Some(id) = self.first_on_lot_person(guid) {
            return Ok(Some(id));
        }
        if crate::interpreter::object_definition(self.catalog, self.world, guid).is_some() {
            let words = self.definition_words(guid)?;
            let interpreter = self.interpreter();
            let unsupported = |fault: crate::fault::Fault| {
                fault.unsupported_in(format!(
                    "making a person of type 0x{guid:08X} for Generic Sims Call 19 (0x4e9719)"
                ))
            };
            if words[TYPE] == PERSON_TYPE && words[crate::factory::word::GROUP] == 0 {
                interpreter
                    .try_create_person(self.world, guid)
                    .map_err(unsupported)?;
            } else {
                interpreter
                    .try_create(self.world, guid)
                    .map_err(unsupported)?;
            }
        }
        Ok(self.first_on_lot_person(guid))
    }

    /// Routine 21 (0x45098a to 0x450b26).
    ///
    /// 1. The running object's temp 0 (+0x3a) names a person. An ID that
    ///    names no object, or an object that is not a person, is native
    ///    error 0x1c (the cast at 0x4509be).
    /// 2. The person's neighbor inventory is the one with its neighbor ID
    ///    (person data word 31, +0x5ca); one is added, empty, when there
    ///    is none (0x42a8a1).
    /// 3. The tokens of kind 2 are counted (0x41f7a7), and the count cut
    ///    to a signed word. That many times: the first token of kind 2,
    ///    any GUID, is found from the start (0x41f69c, from place -1 each
    ///    time). Its GUID says where its count goes ([`restored`]): a
    ///    motive takes the signed count as a float, a data word the count.
    ///    The token is then erased (0x41f56b with -1). A token whose GUID
    ///    names nothing is left where it is, so later passes find it
    ///    again and the tokens after it are not reached.
    ///
    /// The routine always ends with the true branch (0x44f7f0).
    ///
    /// The place found is kept as a word (0x41f6e1), and adding stops at
    /// 0x7fff tokens, but a loaded inventory can hold more. A token found
    /// past 0x7fff is read and erased through its place cut to a signed
    /// word:
    /// - -1 (index 65535 and every 65536 after) reads as nothing found,
    ///   and the pass ends;
    /// - any other negative place is past the end for the read (0x41f746
    ///   compares it unsigned), so the token keeps the values the routine
    ///   gave it before the search (kind 0, GUID 0, count 1, 0x450a1d to
    ///   0x450a23): motive 9 becomes 1, and nothing is erased (0x41f56b
    ///   also compares unsigned);
    /// - a place that wraps back to 0 or more reads, and erases, the
    ///   token there, whatever its kind.
    fn restore_saved_state(&mut self) -> Result<(), Trap> {
        let (person, neighbor) = self.temp_0_person()?;
        let inventory = self.inventory_slot(neighbor);
        let items = &self.world.neighbors.inventories[inventory].items;
        let passes = items.iter().filter(|item| item.kind == SAVED_STATE).count() as i16;
        for _ in 0..passes.max(0) {
            let items = &self.world.neighbors.inventories[inventory].items;
            let Some(index) = items.iter().position(|item| item.kind == SAVED_STATE) else {
                continue;
            };
            let place = (index as u16) as i16;
            if place == -1 {
                continue;
            }
            let token = usize::try_from(place).ok().and_then(|at| items.get(at));
            // A negative place reads nothing and leaves the routine's own
            // values: GUID 0, count 1 (0x450a1d to 0x450a23).
            let (guid, count) = token.map_or((0, 1), |token| (token.guid, token.count as i16));
            let Some(target) = restored(guid) else {
                continue;
            };
            match target {
                Restored::Motive(motive) => {
                    self.set_motive(person, motive, F32::from_f32(f32::from(count)))?;
                }
                Restored::Word(word) => {
                    *self
                        .world
                        .objects
                        .get_mut(&person)
                        .and_then(|object| object.person.as_mut())
                        .and_then(|data| data.data.get_mut(word))
                        .ok_or_else(|| {
                            Trap::Invalid(format!("a person has no data word {word}"))
                        })? = count;
                }
            }
            if let Ok(at) = usize::try_from(place) {
                self.world.neighbors.inventories[inventory].items.remove(at);
            }
        }
        Ok(())
    }
}
