//! Selecting a Sim, and what the game does to a loaded house before its
//! first tick: load steps 16 to 22 of `docs/roadmap.md`.
//!
//! These steps belong to the original's interface, not to its loader.
//! What is ported is what they do to the simulation's state.
//!
//! Selecting a Sim (0x445449) sets the Sim's selected mark, moves the
//! first indicator object into the Sim's slot 1 (0x44664c) and writes the
//! Sim's ID to global 3. Which Sim the game selects after a load is the
//! rule at the end of 0x4f1ae9 ([`Interpreter::select_resident`]).
//!
//! Read from the original's code, with the old prototype as a guide
//! (legacy/crates/sims-vm/src/selection.rs, selection_indicators.rs and
//! interface/person_selection.rs). `docs/decisions.md` says what a trace
//! of the running game confirms.

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::placement::{Place, capture, on_lot};
use sims_sim::World;

/// Global 3: the selected Sim's ID, or 0.
const SELECTED: usize = 3;
/// Globals 9 and 10: the family and the lot's number.
const FAMILY: usize = 9;
const LOT: usize = 10;
/// Global 12: what the interface's wait check found (0x4764fe).
const WAITED_FOR: usize = 12;
/// Global 19: the game's mode. 0 is live mode.
const MODE: usize = 19;
/// Person data words.
mod person {
    /// 0 for a member of the household that lives on the lot.
    pub const VISITOR: usize = 32;
    pub const FAMILY: usize = 61;
    /// The Sim's date: negative while the second indicator marks someone.
    pub const DATE: usize = 77;
}

fn fault(object: u16, trap: Trap) -> Fault {
    Fault {
        object,
        at: None,
        trap,
    }
}

fn unsupported(object: u16, what: &str) -> Fault {
    fault(object, Trap::Unsupported(what.into()))
}

/// The selected Sim (0x4456e4): the first person with the selected mark.
///
/// The original walks its list of people. Here that is the registration
/// order. Which order the original's list has is **unverified**; it
/// matters only if two people are marked.
pub fn selected(world: &World) -> Option<u16> {
    world.order.iter().copied().find(|id| {
        world
            .objects
            .get(id)
            .is_some_and(|object| object.person.is_some() && object.selected)
    })
}

impl Interpreter<'_> {
    /// Load step 16: the loaded house is announced (signal 0xf6), and the
    /// object registry answers (0x44302e). If no Sim is selected it asks
    /// for the indicator objects ([`Interpreter::ensure_indicators`]) and
    /// then announces that nobody is selected (signal 0xe4 with 0), which
    /// the simulator answers by clearing global 3 (0x46d2c2). Returns the
    /// indicators' IDs if it created them.
    ///
    /// A house saves the Sim that was selected in global 3 but not the
    /// Sim's mark, so after a load nobody is selected and global 3 is
    /// cleared here.
    pub fn announce_house(&self, world: &mut World) -> Result<Option<[u16; 2]>, Fault> {
        if selected(world).is_some() {
            return Ok(None);
        }
        let made = self.ensure_indicators(world)?;
        set_global(world, SELECTED, 0)?;
        Ok(made)
    }

    /// Load steps 17 and 18: what the game does once a residential house
    /// is loaded (the end of 0x4f1ae9). It sets the lot's funds from the
    /// family's money again (0x46d8dc). Then, unless a Sim that belongs to
    /// a family is already selected, it selects the family's first member
    /// (0x4e97dc, 0x4e989b). Returns the Sim selected, or None if it
    /// selected nobody.
    ///
    /// `money` is the family's money and `members` its members' types, in
    /// the order of the family's record.
    ///
    /// The first member's Sim is the first object of that type. Looking
    /// it up makes the type's instance list if it has none (0x4408d4,
    /// 0x445857). Whether the Sim may be selected depends on the lot
    /// (0x4e989b):
    ///
    /// - On a residential lot, only a Sim that lives there (person data
    ///   word 32 is 0).
    /// - On lots 21 to 30, any Sim whose word 32 is not 2.
    /// - On a community lot, or lots 40 to 49, a Sim of the selected
    ///   family or one whose word 32 is not 2. On any other lot nothing
    ///   is selected and nobody is deselected.
    ///
    /// A Sim that may not be selected, or a family with no Sim on the
    /// lot, deselects whoever is selected.
    ///
    /// Three things here belong to the interface and are taken as the
    /// trace of a Goth load shows them. All three are **unverified** for
    /// any other case:
    ///
    /// - The original finds the member through its list of portraits. A
    ///   member whose type the list lacks would select nobody. Every
    ///   member is taken to be on the list.
    /// - A process-wide switch lets any Sim be selected (0x6e4f64). It is
    ///   taken to be off, as in the trace.
    /// - The family's record has a field the original clears first
    ///   (0x13c, with a call to 0x4252cc if it was set). It is not
    ///   modeled.
    pub fn select_resident(
        &self,
        world: &mut World,
        money: i32,
        members: &[u32],
    ) -> Result<Option<u16>, Fault> {
        world
            .set_funds(money)
            .map_err(|error| fault(0, Trap::Invalid(error.to_string())))?;
        if let Some(current) = selected(world)
            && person_word(world, current, person::FAMILY)? != 0
        {
            return Ok(Some(current));
        }
        let Some(&guid) = members.first() else {
            // No member 0: the portrait lookup gives -1, which names no
            // type, and nobody is selected.
            self.select(world, None)?;
            return Ok(None);
        };
        if self
            .catalog
            .objects
            .iter()
            .filter(|o| o.guid == guid)
            .count()
            > 1
        {
            // The original lists instances by definition, not GUID.
            return Err(unsupported(
                0,
                "the family's first member has a type that more than one definition names",
            ));
        }
        let objects = &world.objects;
        let instances = world.instances.entry(guid).or_insert_with(|| {
            objects
                .iter()
                .filter(|(_, object)| object.guid == guid)
                .map(|(&id, _)| id)
                .collect()
        });
        let Some(&sim) = instances.first() else {
            self.select(world, None)?;
            return Ok(None);
        };
        let lot = *world
            .globals
            .get(LOT)
            .ok_or_else(|| fault(sim, Trap::Invalid("the lot has no globals".into())))?;
        let zoning = self.program.zoning.get(&lot).copied().unwrap_or(0);
        let visitor = person_word(world, sim, person::VISITOR)?;
        let allowed = if zoning == 1 {
            visitor == 0
        } else if (21..=30).contains(&lot) {
            visitor != 2
        } else if zoning == 2 || (40..50).contains(&lot) {
            person_word(world, sim, person::FAMILY)? == world.globals[FAMILY] || visitor != 2
        } else {
            return Ok(None);
        };
        let target = allowed.then_some(sim);
        self.select(world, target)?;
        Ok(target)
    }

    /// Selects a Sim, or nobody (0x445449). Nothing happens if the target
    /// is already what is selected. Otherwise the target gets the
    /// selected mark, the indicators are moved for it (0x44664c), and the
    /// selection is announced (signal 0xe4), which writes the Sim's ID,
    /// or 0, to global 3 (0x46d2c2).
    ///
    /// Deselecting a Sim that is selected is not ported and fails: the
    /// original takes the mark off and moves the indicators for that Sim
    /// first. After a load nobody is selected.
    pub fn select(&self, world: &mut World, target: Option<u16>) -> Result<(), Fault> {
        let previous = selected(world);
        if previous == target {
            return Ok(());
        }
        if let Some(previous) = previous {
            return Err(unsupported(
                previous,
                "deselecting the selected Sim is not ported",
            ));
        }
        let Some(sim) = target else {
            return set_global(world, SELECTED, 0);
        };
        let object = world
            .objects
            .get_mut(&sim)
            .filter(|object| object.person.is_some())
            .ok_or_else(|| fault(sim, Trap::Invalid("the Sim to select is no person".into())))?;
        if object.multipart.is_some() {
            return Err(unsupported(sim, "selecting a Sim of several tiles"));
        }
        object.selected = true;
        self.move_indicators(world, sim)?;
        // The original reads the Sim's ID again here, after the callbacks.
        set_global(world, SELECTED, sim as i16)
    }

    /// Moves the indicators for a Sim that has just been selected
    /// (0x44664c, for a person with the selected mark). Nothing happens
    /// on a lot without indicators.
    ///
    /// 1. Unless the first indicator already sits in the Sim, it is taken
    ///    off the lot. An indicator that leaves is taken off twice
    ///    ([`Interpreter::take_off`]), so it is asked function 10 twice.
    /// 2. If the Sim stands on the lot and has a slot 1, the indicator is
    ///    put into that slot (0x433340): it is asked function 9, and so
    ///    is the Sim.
    /// 3. The second indicator marks the Sim's date. A Sim with a date
    ///    (person data word 77 not 0) fails: that is not ported.
    ///
    /// Not modeled, being presentation: the indicator's graphic is bound
    /// to the Sim's and both are refreshed. With the Superstar pack the
    /// original also handles star markers here, but only for a Sim that
    /// is not selected.
    fn move_indicators(&self, world: &mut World, sim: u16) -> Result<(), Fault> {
        let [first, second] = world.indicators;
        if first == 0 {
            return Ok(());
        }
        let trap = |trap| fault(sim, trap);
        let missing = || trap(Trap::native(10, "missing object"));
        if !world.objects.contains_key(&first) {
            return Err(missing());
        }
        if capture(world, first).map_err(trap)?.container != Some(sim) {
            self.take_off(world, first, true)?;
        }
        let person = world.objects.get(&sim).ok_or_else(missing)?;
        let position = person.position;
        let on_lot = on_lot([position[0], position[1]]);
        // Slot 1 as scripts count it is the third the object has: the
        // first names the next object on its tile. The original also
        // asks that the slot's flag 2 is set. A person's constructor sets
        // it on this slot, and the lot does not keep slot flags, so every
        // person with the slot is taken to have it. In the trace's memory
        // the two Goths whose slots it holds have the flag there and on
        // no other slot. **Unverified** for any other person.
        if on_lot && let Some(slot) = person.slots.get(2) {
            if slot.object != 0 && world.objects.contains_key(&(slot.object as u16)) {
                return Err(unsupported(
                    sim,
                    "the Sim's slot 1 holds something, which the indicator would replace; \
                     that is not ported",
                ));
            }
            self.put(
                world,
                first,
                Place {
                    on_lot: true,
                    position: [position[0], position[1]],
                    container: Some(sim),
                    slot: 1,
                    level: position[2],
                },
                true,
            )?;
        }
        if second != 0 && person_word(world, sim, person::DATE)? != 0 {
            return Err(unsupported(
                sim,
                "the selected Sim has a date, which the second indicator marks; that is not \
                 ported",
            ));
        }
        Ok(())
    }

    /// Load step 19: the game goes to live mode (0x4cea5d, 0x416534,
    /// 0x4ce44f). Every room is marked as not counted
    /// ([`World::mark_all_rooms`]). The lot's statistics are worked out
    /// ([`World::update_statistics`]). Then global 19, the mode, becomes
    /// 0.
    ///
    /// Not modeled: each room's cached route rectangles are dropped
    /// (0x467436), and each person is told of the mode (0x444805). In a
    /// trace of a Goth load neither changes anything compared.
    pub fn go_live(&self, world: &mut World) -> Result<(), Fault> {
        world.mark_all_rooms();
        world
            .update_statistics()
            .map_err(|error| fault(0, Trap::Invalid(error.to_string())))?;
        set_global(world, MODE, 0)
    }

    /// Load steps 21 and 22: what the game does to the simulation's state
    /// before it draws a frame and runs a tick. The rooms that wait are
    /// counted ([`Interpreter::update_pending_rooms`], 0x467094). Then
    /// the interface's wait check (0x4764fe) writes what it found to
    /// global 12: the control a tutorial is waiting for, or 0. There is
    /// no interface here, so it is 0. Returns the rooms counted.
    ///
    /// Between step 19 and these the original counts the family's
    /// friends (0x427219, step 20), which changes nothing the simulation
    /// holds.
    pub fn before_frame(&self, world: &mut World) -> Result<Vec<i16>, Fault> {
        let counted = self.update_pending_rooms(world)?;
        set_global(world, WAITED_FOR, 0)?;
        Ok(counted)
    }
}

fn set_global(world: &mut World, index: usize, value: i16) -> Result<(), Fault> {
    *world
        .globals
        .get_mut(index)
        .ok_or_else(|| fault(0, Trap::Invalid("the lot has no globals".into())))? = value;
    Ok(())
}

fn person_word(world: &World, id: u16, index: usize) -> Result<i16, Fault> {
    world
        .objects
        .get(&id)
        .and_then(|object| object.person.as_ref())
        .and_then(|person| person.data.get(index))
        .copied()
        .ok_or_else(|| fault(id, Trap::Invalid(format!("no person data word {index}"))))
}
