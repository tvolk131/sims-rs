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
    /// With the Superstar pack, 10 or more gives a person who is not
    /// selected a star marker (0x446c15).
    pub const FAME: usize = 81;
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

/// What deselecting a Sim does about its animator's refresh (0x58da12
/// with 0, from 0x446921), which is not ported: see
/// [`Interpreter::select_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    /// Fail loudly, as for anything not ported: the default.
    Fail,
    /// Leave the refresh out, so the deselected Sim's animator, head and
    /// person words do not move as the original's would. For play mode
    /// only (`Lot::select_play`); a lot that did this is not one to
    /// compare with the original.
    Skip,
}

/// The selected Sim (0x4456e4): [`World::selected`].
pub fn selected(world: &World) -> Option<u16> {
    world.selected()
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
    /// is already what is selected. Otherwise the Sim selected before, if
    /// any, is deselected (`Interpreter::deselect`); then the target
    /// gets the selected mark, the indicators are moved for it
    /// (0x44664c), and the selection is announced (signal 0xe4), which
    /// writes the Sim's ID, or 0, to global 3 (0x46d2c2).
    ///
    /// After a load nobody is selected. The player selects a Sim from the
    /// interface (a portrait), between ticks.
    ///
    /// Deselecting a Sim while the lot has indicators refreshes its
    /// animator, which is not ported, so that fails ([`Refresh::Fail`]).
    pub fn select(&self, world: &mut World, target: Option<u16>) -> Result<(), Fault> {
        self.select_with(world, target, Refresh::Fail)
    }

    /// [`Interpreter::select`], with `refresh` saying what deselecting
    /// does about the deselected Sim's animator refresh.
    pub fn select_with(
        &self,
        world: &mut World,
        target: Option<u16>,
        refresh: Refresh,
    ) -> Result<(), Fault> {
        let previous = selected(world);
        if previous == target {
            return Ok(());
        }
        if let Some(previous) = previous {
            self.deselect(world, previous, refresh)?;
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

    /// Deselects the selected Sim `sim`, the first half of 0x445449: its
    /// selected mark is taken off (0x4320da with the flags' low five bits
    /// less bit 1), and the indicators are moved for a Sim without the
    /// mark (0x44664c from 0x4468c3). With nobody selected now:
    ///
    /// 1. The first indicator is taken off the lot (its virtual function
    ///    0x50 with 1, 0x4468fd), as `Interpreter::take_off` takes it.
    /// 2. The person's animator is refreshed (its virtual function 0x54
    ///    with 0, 0x446921, 0x58da12). With 0 the refresh always runs: it
    ///    reads the clock again (virtual function 0x34), brings the skin
    ///    up to date at that clock (0x5992b3), places the animator
    ///    (virtual function 0x60), runs the head's step, measurement and
    ///    restoration (0x590768, 0x5904d2, 0x590b27), which can count
    ///    person word 45 down and write word 42, and redraws what the
    ///    person carries in its slot 1. Between ticks the clock has moved
    ///    on from the animator's, so this changes what the simulation
    ///    reads. It is not ported: with [`Refresh::Fail`] it fails; with
    ///    [`Refresh::Skip`] it is left out.
    /// 3. The second indicator is taken off the lot too (0x446be8).
    /// 4. With the Superstar pack (bit 0x40 of the installed-expansions
    ///    mask, application field +0x66, set on a Complete Collection
    ///    install and not modeled), a person whose word 81 is 10 or more
    ///    gets a star marker (0x446c15). That is not ported and fails,
    ///    whatever the mask, as `Interpreter::placed_from_outside`
    ///    does.
    ///
    /// Not modeled, being presentation: the object is redrawn (0x434101
    /// with 0, from the outermost object holding it; that it writes
    /// nothing the simulation reads is **unverified**). A lot without
    /// indicators does nothing more than take the mark off (0x44667f).
    fn deselect(&self, world: &mut World, sim: u16, refresh: Refresh) -> Result<(), Fault> {
        let object = world
            .objects
            .get_mut(&sim)
            .ok_or_else(|| fault(sim, Trap::native(10, "missing object")))?;
        object.selected = false;
        let [first, second] = world.indicators;
        if first == 0 {
            return Ok(());
        }
        if selected(world).is_none() {
            self.take_off(world, first, true)?;
            if refresh == Refresh::Fail {
                return Err(unsupported(
                    sim,
                    "deselecting a Sim refreshes its animator (0x58da12 with 0, from 0x446921), \
                     which reads the clock again, updates the skin and steps the head; not \
                     ported",
                ));
            }
        }
        if selected(world).is_none() && second != 0 {
            self.take_off(world, second, true)?;
        }
        if person_word(world, sim, person::FAME)? >= 10 {
            return Err(unsupported(
                sim,
                "a Sim deselected whose person word 81 is 10 or more takes a star marker \
                 (0x446c15); not ported",
            ));
        }
        Ok(())
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

    /// Load steps 20 to 22: what the game does to the simulation's state
    /// before it draws a frame and runs a tick.
    ///
    /// 20. The interface's family panel shows the selected family's
    ///     friend count ([`Interpreter::refresh_family_panel`]), which
    ///     works the count out again, as it is out of date since the
    ///     neighborhood was read.
    /// 21. The rooms that wait are counted
    ///     ([`Interpreter::update_pending_rooms`], 0x467094).
    /// 22. The interface's wait check (0x4764fe) writes what it found to
    ///     global 12: the control a tutorial is waiting for, or 0. There
    ///     is no interface here, so it is 0.
    ///
    /// Returns the rooms counted. In a capture of the original at the
    /// Goth house's first tick, the Goth family's count is up to date
    /// and every other family's is not.
    pub fn before_frame(&self, world: &mut World) -> Result<Vec<i16>, Fault> {
        self.refresh_family_panel(world);
        let counted = self.update_pending_rooms(world)?;
        set_global(world, WAITED_FOR, 0)?;
        Ok(counted)
    }

    /// The interface's family panel shows the selected family's friend
    /// count again (0x53a01f, slot +0x17c of the `cWinViewControl`
    /// vtable at 0x66f220, which the generic window update 0x5dce19 calls
    /// at 0x5dd0de): 0x4cfcdb → 0x410260 → 0x427219, which works the
    /// count out again if a relationship write put it out of date
    /// ([`sims_sim::World::family_friends`]). The family is the first
    /// whose ID is global 9, with or without a house; on an empty lot (0)
    /// nothing is counted, which is **unverified**.
    ///
    /// The game updates its windows whenever it handles its messages:
    /// between lot updates, and while a dialog that stops the game waits
    /// for its answer (0x436da0 → 0x4f6b95 → 0x5dab4b). [`crate::Lot`]
    /// calls this after each lot update and the registry's dialog pass
    /// calls it as it shows each such dialog. A count is cached until the
    /// family is put out of date, so a refresh can change what a later
    /// read of scope 24 field 9 gives: a change the count reads that is
    /// not a relationship write (such as a row a relationship read makes)
    /// leaves the count the refresh worked out.
    ///
    /// In every capture of the Goth house (608 updates) the Goth family's
    /// count is up to date after the update. Whether the original redraws
    /// the panel on every pass of its messages, or only when the panel is
    /// marked, is **unverified**; the count is worked out only when it is
    /// out of date, so the two differ only if a relationship write and a
    /// read of the count fall between the same two redraws.
    pub fn refresh_family_panel(&self, world: &mut World) {
        let family = i32::from(world.globals.get(9).copied().unwrap_or(0));
        if family != 0
            && let Some(place) = world.families.iter().position(|f| f.id == family)
        {
            let threshold = self.program.autonomy_constants().friendship_threshold;
            world.family_friends(place, threshold);
        }
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
