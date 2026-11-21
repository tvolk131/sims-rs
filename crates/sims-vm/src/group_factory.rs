//! The factory for multi-tile objects: the group branch of the original's
//! object factory (0x440b2d, from 0x440b63).
//!
//! A multi-tile object is a group of tiles, each an object of its own with
//! its own definition, sharing the group ID in definition word 10. The
//! group's catalog master (word 11 is -1) is what a script asks for and
//! what the catalog sells; it is never made itself. [`Interpreter::create_group`]
//! makes its tiles, in this order:
//!
//! 1. The definition asked for must be its group's catalog master
//!    (0x43adf7). For any other definition the original makes nothing
//!    and returns 0, which fails here.
//! 2. The tiles, in the order of [`Interpreter::group_plan`]. Each takes
//!    the lowest unused object ID, is constructed with the first as its
//!    lead (0x44099b, 0x4227b3), registered at the head of the
//!    registration list (0x4434cd), given its place in the group
//!    (vtable +0x3c, 0x422872) and dated (0x431df3); see
//!    [`Machine::construct_tile`].
//! 3. The lead takes the catalog master's price in data word 41
//!    (0x440cd9), which the reset keeps.
//! 4. The group's reset with 1 (vtable +0x40, 0x423175), as
//!    [`Interpreter::rebuild`] runs it after taking a group off: every
//!    tile's storage is reset and its thread restarts at Main (0x431be1,
//!    which skips Init for a tile); each tile's offset from the lead is
//!    worked out (vtable +0x88, 0x422a3b); then, tile by tile, Init runs
//!    (0x422d46), the tile gets its first turn (vtable +0x14, 0x4521f7)
//!    and its scheduler flags 2 and 8 are refreshed (0x4457ef).
//! 5. Each tile, from the lead along the list, gets the scheduler's ready
//!    flag (0x440d07 to 0x440d57, 0x4457ef with 1).
//!
//! The factory returns the lead's ID. New tiles stand off the lot; the
//! caller places the group.
//!
//! Not ported, as the single-tile factory does not port them: the
//! failure paths the original takes when it cannot allocate or register
//! a tile (0x440c20 to 0x440c48, then the deletion of the lead at
//! 0x440cb7), and object flag 0x80 (object +0x114), which the ready step
//! clears and sets and which nothing in the simulation reads.
//!
//! Ported from the old prototype's group factory
//! (legacy/crates/sims-vm/src/group_factory.rs and
//! legacy/crates/sims-vm/src/multitile.rs, `construction_plan`), which
//! compared it with the original's code run alone, and read again from
//! the original's code.

use crate::factory::{TURN_BUDGET, flag, word};
use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use sims_sim::World;

/// Result of resolving a multi-tile group's catalog master (`0x440b63`).
pub(crate) enum GroupPlan {
    /// Tile GUIDs in construction order, starting with the lead tile.
    Tiles(Vec<u32>),
    /// `0x440b90`: the requested definition is not a group's catalog master;
    /// the original makes nothing and returns 0.
    NotMaster,
    /// `0x440c9b`: the catalog master's group names no lead tile; the
    /// original makes nothing and returns 0.
    NoLead,
}

impl Interpreter<'_> {
    /// Makes a multi-tile object from its group's catalog master `guid`
    /// and returns its lead tile's ID. Every tile's Init and first turn
    /// have run when this returns.
    ///
    /// On a fault the tiles made so far stay registered, as far as they
    /// got, so the fault can be inspected.
    pub(crate) fn create_group(&self, world: &mut World, guid: u32) -> Result<u16, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let plan = self.group_plan(guid).map_err(|trap| fail(0, trap))?;
        self.create_group_tiles(world, guid, plan)
    }

    /// Like [`Self::create_group`], but returns `Ok(None)` when the native
    /// factory refuses to construct the group and returns 0 (`0x440b90` or
    /// `0x440c9b`).
    pub(crate) fn try_create_group(
        &self,
        world: &mut World,
        guid: u32,
    ) -> Result<Option<u16>, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        match self.try_group_plan(guid).map_err(|trap| fail(0, trap))? {
            GroupPlan::Tiles(plan) => self.create_group_tiles(world, guid, plan).map(Some),
            GroupPlan::NotMaster | GroupPlan::NoLead => Ok(None),
        }
    }

    fn create_group_tiles(
        &self,
        world: &mut World,
        guid: u32,
        plan: Vec<u32>,
    ) -> Result<u16, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let master = self
            .machine(world, 0, 0, false)
            .definition_words(guid)
            .map_err(|trap| fail(0, trap))?;
        let mut parts: Vec<u16> = Vec::with_capacity(plan.len());
        for tile in plan {
            let id = (1..=i16::MAX as u16)
                .find(|id| !world.objects.contains_key(id))
                .ok_or_else(|| fail(0, Trap::Unsupported("no unused object ID".into())))?;
            self.machine(world, id, 0, false)
                .construct_tile(tile, parts.first().copied())
                .map_err(|trap| fail(id, trap))?;
            parts.push(id);
        }
        let lead = parts[0];
        world
            .objects
            .get_mut(&lead)
            .ok_or_else(|| fail(lead, Trap::native(10, "missing object")))?
            .data[41] = master[word::PRICE];
        // The group's reset (0x423175 with 1).
        for &part in &parts {
            let mut machine = self.machine(world, part, 0, false);
            let guid = machine.me().map_err(|trap| fail(part, trap))?.guid;
            let words = machine
                .definition_words(guid)
                .map_err(|trap| fail(part, trap))?;
            machine.reset(words).map_err(|trap| fail(part, trap))?;
            machine.install_main().map_err(|trap| fail(part, trap))?;
        }
        self.update_offsets(world, lead)
            .map_err(|trap| fail(lead, trap))?;
        for &part in &parts {
            self.init(world, part)?;
            self.turn(world, part, TURN_BUDGET)?;
            let mut machine = self.machine(world, part, 0, false);
            machine.end_turn().map_err(|trap| fail(part, trap))?;
            machine.refresh_flags().map_err(|trap| fail(part, trap))?;
        }
        // The ready flag, from the lead along the list as it now stands: a
        // tile that left the group in its Init or first turn is not
        // reached (0x440cf5).
        for part in self.parts(world, lead).map_err(|trap| fail(lead, trap))? {
            if let Some(object) = world.objects.get_mut(&part) {
                object.set_flags(flag::READY, true);
            }
        }
        Ok(lead)
    }

    /// The GUIDs of the tiles a group's factory makes from its catalog
    /// master `guid`, in the order it makes them (0x440b96 to 0x440c8e).
    /// Definitions are taken in the catalog's order, which lists each
    /// file's in its native resource order, and only those of the
    /// master's file and group count (0x5c18e3, word 10):
    ///
    /// 1. The lead: the first definition whose word 63 is nonzero
    ///    (0x43acf5), or else the first whose word 11 is 0 and word 56 is
    ///    0, the tile at the group's origin (0x43ac5c).
    /// 2. Then every definition that is not the catalog master (0x43ad79,
    ///    0x43adf7), skipping the lead's own (0x440bc3).
    ///
    /// A master that names no lead makes nothing and returns 0
    /// (0x440c9b), which fails here, and so does a definition that is
    /// not a catalog master (0x440b90) or a lead that is the master.
    pub(crate) fn group_plan(&self, guid: u32) -> Result<Vec<u32>, Trap> {
        match self.try_group_plan(guid)? {
            GroupPlan::Tiles(tiles) => Ok(tiles),
            GroupPlan::NotMaster => Err(Trap::Unsupported(format!(
                "making 0x{guid:08X}, which is not a multi-tile group's catalog master: the \
                 original makes nothing and returns 0 (0x440b90), and what its caller does \
                 with that is not ported"
            ))),
            GroupPlan::NoLead => Err(Trap::Unsupported(format!(
                "the multi-tile group of 0x{guid:08X} names no lead tile: the original \
                 makes nothing and returns 0 (0x440c9b), and what its caller does with \
                 that is not ported"
            ))),
        }
    }

    /// Like [`Self::group_plan`], distinguishing the two native factory-zero
    /// returns (`0x440b90` and `0x440c9b`) from unported or invalid cases.
    pub(crate) fn try_group_plan(&self, guid: u32) -> Result<GroupPlan, Trap> {
        let definition = self
            .catalog
            .object(guid)
            .ok_or_else(|| Trap::Invalid(format!("no definition for GUID 0x{guid:08X}")))?;
        let library = self
            .program
            .libraries
            .get(&definition.resource)
            .ok_or_else(|| Trap::Invalid("the object's file is not linked".into()))?;
        let master = library.definitions.get(&definition.chunk).ok_or_else(|| {
            Trap::Invalid(format!(
                "the program holds no definition for GUID 0x{guid:08X}"
            ))
        })?;
        let group = master[word::GROUP];
        if group == 0 || master[word::SUB_INDEX] != -1 {
            return Ok(GroupPlan::NotMaster);
        }
        let members: Vec<(u16, &[i16; crate::DEFINITION_WORDS])> = library
            .definition_order
            .iter()
            .filter_map(|chunk| Some((*chunk, library.definitions.get(chunk)?)))
            .filter(|(_, words)| words[word::GROUP] == group)
            .collect();
        let Some(lead) = members
            .iter()
            .find(|(_, words)| words[word::LEAD] != 0)
            .or_else(|| {
                members
                    .iter()
                    .find(|(_, words)| words[word::SUB_INDEX] == 0 && words[word::LEVEL] == 0)
            })
        else {
            return Ok(GroupPlan::NoLead);
        };
        if lead.1[word::SUB_INDEX] == -1 {
            return Err(Trap::Unsupported(format!(
                "the multi-tile group of 0x{guid:08X} names its catalog master as its lead \
                 tile (0x43acf5), which is not ported"
            )));
        }
        let tiles = std::iter::once(lead)
            .chain(
                members
                    .iter()
                    .filter(|(chunk, words)| *chunk != lead.0 && words[word::SUB_INDEX] != -1),
            )
            .map(|(chunk, words)| {
                let tile = (words[word::GUID] as u16 as u32)
                    | ((words[word::GUID + 1] as u16 as u32) << 16);
                // The original constructs a tile from its definition; here
                // the GUID names it, so the catalog must lead back to it.
                match self.catalog.object(tile) {
                    Some(found)
                        if found.resource == definition.resource && found.chunk == *chunk =>
                    {
                        Ok(tile)
                    }
                    _ => Err(Trap::Unsupported(format!(
                        "the tile OBJD #{chunk} of 0x{guid:08X} has GUID 0x{tile:08X}, which the \
                         catalog does not resolve to it"
                    ))),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(GroupPlan::Tiles(tiles))
    }
}
