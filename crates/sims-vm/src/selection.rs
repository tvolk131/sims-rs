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
//! interface/person_selection.rs). `docs/decisions/` says what a trace
//! of the running game confirms.

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::placement::{Place, capture, head_slot_redraw, off_lot};
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

/// The selected Sim (0x4456e4): [`World::selected`].
pub fn selected(world: &World) -> Option<u16> {
    world.selected()
}

/// Publish the application's wait check (0x4764fe → 0x476548 →
/// 0x46d9ce, setter 0x46d9c4). A valid button release (0x5e45ed)
/// records the button in [0x6fe9a4]. The poll reads its bitmap at +0xd8
/// and the resource ID at bitmap +0x30, or returns 0 if either pointer
/// is null. Destroying the button clears the pointer (0x5e39cf).
pub(crate) fn publish_waited_for(world: &mut World, bitmap_id: i16) -> Result<(), Fault> {
    set_global(world, WAITED_FOR, bitmap_id)
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
    pub fn select(&self, world: &mut World, target: Option<u16>) -> Result<(), Fault> {
        let previous = selected(world);
        if previous == target {
            return Ok(());
        }
        if let Some(previous) = previous {
            self.deselect(world, previous)?;
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

    /// Selects the next household member on the lot (0x445501, normal path with
    /// `[0x6e4f64] == 0`): finds the selected family (`globals[9]`), locates the
    /// currently selected Sim among its `members` via `0x44093d` (first person
    /// in `world.people` of that GUID), and selects the next member present on
    /// the lot wrapping around. When nobody is selected, starts at member 0.
    /// Returns `Ok(None)` without error when the family has no member on the
    /// lot (such as an empty or newly moved-in lot before arrival spawns them)
    /// or when a currently selected Sim is not in the family's member list.
    pub fn select_next_resident(&self, world: &mut World) -> Result<Option<u16>, Fault> {
        let family_id = i32::from(world.globals.get(FAMILY).copied().unwrap_or(0));
        let Some(members) = world
            .families
            .iter()
            .find(|f| f.id == family_id)
            .map(|f| f.members.clone())
        else {
            return Ok(None);
        };
        if members.is_empty() {
            return Ok(None);
        }
        let find_person = |world: &World, guid: u32| -> Option<u16> {
            world
                .people
                .iter()
                .copied()
                .find(|id| world.objects.get(id).is_some_and(|o| o.guid == guid))
        };
        let current = selected(world);
        let start: isize = match current {
            None => -1,
            Some(cur) => match members
                .iter()
                .position(|&guid| find_person(world, guid) == Some(cur))
            {
                Some(index) => index as isize,
                None => return Ok(None),
            },
        };
        let len = members.len();
        for offset in 1..=len {
            let index = ((start + offset as isize) as usize) % len;
            if let Some(person) = find_person(world, members[index]) {
                self.select(world, Some(person))?;
                return Ok(Some(person));
            }
        }
        Ok(None)
    }

    /// Deselects the selected Sim `sim`, the first half of 0x445449: its
    /// selected mark is taken off (0x4320da with the flags' low five bits
    /// less bit 1), and the indicators are moved for a Sim without the
    /// mark (0x44664c from 0x4468c3). With nobody selected now:
    ///
    /// 1. The first indicator is taken off the lot (its virtual function
    ///    0x50 with 1, 0x4468fd), as `Interpreter::take_off` takes it.
    /// 2. The person's animator is refreshed (its virtual function 0x54
    ///    with 0, 0x446921, 0x58da12). With 0 the refresh always runs,
    ///    whether or not the house view drew the person (the +0x109 gate
    ///    at 0x58da2c is skipped): it reads the clock again (virtual
    ///    function 0x34, the lot's tick count, which between ticks is the
    ///    count after the last update), removes a blend layer with no
    ///    weight (none is made here), updates the draw state and the face
    ///    (0x434101, 0x58faf6, presentation, left out as in the
    ///    animator's tick), forces the skin's update at that clock
    ///    (0x5992b3 with 1), places the animator (virtual function 0x60),
    ///    and runs the head's step, measurement and restoration (0x590768,
    ///    0x5904d2, 0x590b27), which can count person word 45 down and
    ///    write word 42. This is the refresh a take-off makes when the
    ///    view drew the person (`Interpreter::refresh_animator`). Then it
    ///    redraws what the person's head slot holds (0x58dae5 to 0x58dc55,
    ///    the third container entry), which fails for a person or a tile
    ///    of a group there (`crate::placement::head_slot_redraw`).
    /// 3. The second indicator is taken off the lot too (0x446be8).
    /// 4. With the Superstar pack (bit 0x40 of the installed-expansions
    ///    mask, application field +0x66, [`crate::Program::expansions`]),
    ///    a person whose word 81 is 10 or more gets a star marker from the
    ///    interface's pool (0x446c15, [`Interpreter::mark_star`]).
    ///
    /// Not modeled, being presentation: the object is redrawn (0x434101
    /// with 0, from the outermost object holding it; that it writes
    /// nothing the simulation reads is **unverified**). A lot without
    /// indicators does nothing more than take the mark off (0x44667f).
    fn deselect(&self, world: &mut World, sim: u16) -> Result<(), Fault> {
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
            self.refresh_animator(world, sim)
                .and_then(|()| head_slot_redraw(world, sim))
                .map_err(|trap| fault(sim, trap))?;
        }
        if selected(world).is_none() && second != 0 {
            self.take_off(world, second, true)?;
        }
        if person_word(world, sim, person::FAME)? >= 10 {
            self.mark_star(world, sim)?;
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
        // 0x446727 -> 0x422632: skip head-slot release and indicator placement
        // when the Sim stands on or outside the lot's outer border tiles.
        let on_inner_lot = !off_lot(world, [position[0], position[1]]);
        // Slot 1 as scripts count it is the third the object has: the
        // first names the next object on its tile. The original also
        // asks that the slot's flag 2 is set. A person's constructor sets
        // it on this slot, and the lot does not keep slot flags, so every
        // person with the slot is taken to have it. In the trace's memory
        // the two Goths whose slots it holds have the flag there and on
        // no other slot. **Unverified** for any other person.
        if on_inner_lot && let Some(slot) = person.slots.get(2) {
            let held = u16::try_from(slot.object)
                .ok()
                .filter(|&id| id != 0 && world.objects.contains_key(&id));
            if let Some(held) = held {
                // 0x4467d7: release any indicator or star marker held in the
                // head slot (0x4481eb) before placing the selection indicator.
                self.release_from_head(world, held)?;
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
    /// 22. The interface's wait check (0x4764fe) publishes 0 to global 12.
    ///     A modal dialog can publish a button's bitmap ID later in the
    ///     lot update, through [`crate::Answer`].
    ///
    /// Returns the rooms counted. In a capture of the original at the
    /// Goth house's first tick, the Goth family's count is up to date
    /// and every other family's is not.
    pub fn before_frame(&self, world: &mut World) -> Result<Vec<i16>, Fault> {
        self.refresh_family_panel(world);
        let counted = self.update_pending_rooms(world)?;
        publish_waited_for(world, 0)?;
        Ok(counted)
    }

    /// The interface's family panel shows the selected family's friend
    /// count again (0x53a01f, slot +0x17c of the `cWinViewControl`
    /// vtable at 0x66f220, which the generic window update 0x5dce19 calls
    /// at 0x5dd0de): 0x4cfcdb → 0x410260 → 0x427219, which works the
    /// count out again if a relationship write put it out of date
    /// ([`sims_sim::World::family_friends`]). On residential lots
    /// (`0x40745f`: global 10 is 0 or has zoning 1), the family is the
    /// first whose ID is global 9 (`0x4cfd4a`; 0 counts nothing). On
    /// non-residential lots (`0x4cfd1a..0x4cfd48`), it looks up the
    /// selected Sim's person data word 61 (`person::FAMILY`,
    /// `[person + 0x606]`) via `0x425d69`.
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
        let lot = world.globals.get(LOT).copied().unwrap_or(0);
        let residential = lot == 0 || self.program.zoning.get(&lot) == Some(&1);
        let place = if residential {
            let family = i32::from(world.globals.get(FAMILY).copied().unwrap_or(0));
            (family != 0)
                .then(|| world.families.iter().position(|f| f.id == family))
                .flatten()
        } else {
            selected(world)
                .and_then(|id| {
                    let obj = world.objects.get(&id)?;
                    world
                        .people
                        .iter()
                        .filter_map(|pid| world.objects.get(pid))
                        .find(|o| o.guid == obj.guid && o.person.is_some())
                        .unwrap_or(obj)
                        .person
                        .as_ref()
                })
                .and_then(|p| {
                    let family = i32::from(p.data.get(person::FAMILY).copied().unwrap_or(0));
                    world.families.iter().position(|f| f.id == family)
                })
        };
        if let Some(place) = place {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Program;
    use sims_formats::saved_objects::{Interaction, Person};

    fn test_person(id: u16, guid: u32, family: i16, selected: bool) -> sims_sim::Object {
        let idle = Interaction {
            join: 0,
            uid: 0,
            caller: 0,
            target: 0,
            icon: 0,
            table_index: -1,
            args: [0; 4],
            priority: 0,
            action: 0,
            attenuation: 0.0,
            flags: 0,
        };
        let mut data = vec![0; sims_sim::neighbors::PERSON_WORDS];
        data[person::FAMILY] = family;
        let person = Person {
            animation_events: 0,
            engaged: 0,
            appearance: Default::default(),
            accessories: Vec::new(),
            animations: Default::default(),
            routing_state: 0,
            floats: [0.0; 9],
            previous_motives: [0.0; 16],
            motives: [0.0; 16],
            data,
            routing_frames: 0,
            active: idle.clone(),
            last: idle,
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        };
        sims_sim::Object {
            id,
            guid,
            footprint: [0; 4],
            position: [0; 3],
            unknown_short: 0,
            attributes: Vec::new(),
            temps: Vec::new(),
            data: vec![0; 128],
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: Vec::new(),
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: Some(person),
            registry_word: None,
            last_result: false,
            removal_skips_resets: false,
            selected,
        }
    }

    fn neighbor(
        id: i16,
        guid: u32,
        family: i16,
        rows: &[(i32, i32)],
    ) -> sims_sim::neighbors::Neighbor {
        let mut data = vec![0; sims_sim::neighbors::PERSON_WORDS];
        data[person::FAMILY] = family;
        sims_sim::neighbors::Neighbor {
            id,
            guid,
            revision: 1,
            data,
            relationships: rows
                .iter()
                .map(|&(target, daily)| sims_sim::neighbors::Row {
                    target,
                    values: vec![daily, 0, 0, 0],
                })
                .collect(),
            friends: 0,
            stale: false,
        }
    }

    #[test]
    fn refresh_family_panel_updates_selected_sims_family_cache_on_community_lots() {
        use sims_content::Catalog;
        use std::collections::BTreeMap;
        let program = Program {
            global: 0,
            libraries: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            careers: None,
            career_texts_end: false,
            labels_hold_no_token: false,
            zoning: BTreeMap::from([(5, 1), (7, 2)]),
            animations: Default::default(),
            people: Default::default(),
            icon_frames: None,
            wardrobe: None,
            ordered_wardrobe: None,
            character_files: Default::default(),
            expansions: 0,
            characters: Default::default(),
        };
        let catalog = Catalog {
            global: 0,
            objects: Vec::new(),
            by_guid: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
        let mut world = World::empty();
        world.globals.resize(16, 0);
        world.globals[FAMILY] = 1;
        world.globals[LOT] = 7; // community lot
        world.families.push(sims_sim::neighbors::Family {
            id: 1,
            house: 5,
            members: vec![100],
            friends: 0,
            stale: true,
        });
        world.families.push(sims_sim::neighbors::Family {
            id: 2,
            house: 9,
            members: vec![200],
            friends: 0,
            stale: true,
        });
        world.neighbors.slots = vec![
            Some(neighbor(1, 100, 1, &[(3, 80)])),
            Some(neighbor(2, 200, 2, &[(3, 80)])),
            Some(neighbor(3, 300, 3, &[(1, 80), (2, 80)])),
        ];
        world.objects.insert(10, test_person(10, 200, 2, true));
        world.order.push(10);
        world.people.push(10);

        let interpreter = Interpreter::new(&program, &catalog, crate::Stream::Lot);
        interpreter.refresh_family_panel(&mut world);

        // On community lot 7, 0x4cfcdb refreshes the selected Sim's family (2),
        // clearing its stale flag and caching its count, while leaving global 9's
        // family (1) stale and untouched.
        assert!(world.families[0].stale);
        assert_eq!(world.families[0].friends, 0);
        assert!(!world.families[1].stale);
        assert_eq!(world.families[1].friends, 1);

        // Switching to residential lot 5 refreshes global 9's family (1).
        world.globals[LOT] = 5;
        interpreter.refresh_family_panel(&mut world);
        assert!(!world.families[0].stale);
        assert_eq!(world.families[0].friends, 1);
    }
}
