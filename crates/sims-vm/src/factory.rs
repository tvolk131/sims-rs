//! The object factory: how the original makes a new object on a lot.
//!
//! [`Interpreter::create`] follows the original's factory (0x440b2d) for an
//! ordinary one-tile object:
//!
//! 1. Pick the lowest unused object ID.
//! 2. Construct the object's storage (0x43189d, 0x431aaf) and register it
//!    at the head of the registration list (0x4434cd). It starts off the
//!    lot.
//! 3. Stamp it with the lot's date and time.
//! 4. Reset its storage (0x431be1) and install its Main tree as the bottom
//!    frame of its thread (0x47480c).
//! 5. Run its Init tree synchronously, on top of the Main frame (0x4749b5).
//! 6. Give the object its first turn: Main runs until it yields or returns.
//! 7. Finish: count down the lockout word, refresh the scheduler flags, and
//!    set the object's value from its definition's price.
//!
//! Ported from the old prototype's `begin_object_factory`,
//! `reset_base_storage`, `install_lifecycle` and `finish_object_factory`
//! (legacy/crates/sims-vm/src/construction.rs, lifecycle.rs and
//! registry_flags.rs), without its threads, hosts and frame arena. People (type 2), the cursor
//! types (8 and 9) and multi-tile objects have their own factories, which
//! are not ported: asking for one fails.

use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Machine};
use crate::memory::LOCKOUT;
use crate::program::{Callback, DEFINITION_WORDS, Tree};
use sims_formats::saved_objects::Slot;
use sims_sim::{Object, World};

/// How many instructions a new object's Init and first turn may each run.
/// The old prototype used the same guard; the original's own limit is not
/// modeled.
pub const TURN_BUDGET: usize = 100_000;

/// Where a new object stands until something places it: off the lot, on
/// level 1.
const OFF_LOT: [i32; 3] = [-16, -16, 1];

/// Scheduler flags, the high half of an object's registry word.
mod flag {
    /// Set once the factory has finished the object.
    pub const READY: u16 = 1;
    /// Data word 60 is nonzero.
    pub const QUEUED: u16 = 2;
    /// The object's interaction table offers something Sims choose by
    /// themselves.
    pub const ADVERTISES: u16 = 8;
}

/// Definition words the factory reads.
mod word {
    pub const TYPE: usize = 9;
    pub const GROUP: usize = 10;
    pub const PRICE: usize = 18;
    /// Copied into data word 58 on every reset (OBJD byte 0x62).
    pub const DATA_58: usize = 49;
    pub const ATTRIBUTES: usize = 58;
    pub const DYNAMIC_SPRITES: usize = 65;
}

impl Interpreter<'_> {
    /// Creates an object of the type `guid` names and returns its ID. The
    /// object's Init and its first turn have run when this returns.
    ///
    /// On a fault the object stays registered, as far as it got, so the
    /// fault's object and frame can be inspected.
    pub fn create(&self, world: &mut World, guid: u32) -> Result<u16, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let id = (1..=i16::MAX as u16)
            .find(|id| !world.objects.contains_key(id))
            .ok_or_else(|| fail(0, Trap::Unsupported("no unused object ID".into())))?;
        let mut machine = self.machine(world, id, 0, false);
        let words = machine.construct(guid).map_err(|trap| fail(id, trap))?;
        let run_init = machine
            .reset(words)
            .and_then(|run_init| machine.install_main().map(|()| run_init))
            .map_err(|trap| fail(id, trap))?;
        if run_init {
            self.init(world, id)?;
        }
        self.turn(world, id, TURN_BUDGET)?;
        self.machine(world, id, 0, false)
            .finish(words)
            .map_err(|trap| fail(id, trap))?;
        Ok(id)
    }

    /// Runs an object's Init tree synchronously on top of whatever its
    /// thread holds. Init gets the object's temps as its arguments and no
    /// stack object. A definition that names no Init, or one whose tree is
    /// missing, runs nothing. Init's result is ignored.
    fn init(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let definition = self
            .catalog
            .object(object.guid)
            .ok_or_else(|| fail(Trap::Invalid("object has no definition".into())))?;
        let init = self.program.callback(definition, Callback::Init);
        if init == 0 || matches!(self.program.tree(definition.resource, init), Tree::Missing) {
            return Ok(());
        }
        let temps = object.temps.clone();
        // The original copies Init's arguments out of the eight temps.
        self.call(world, id, init, 0, &temps, 8, TURN_BUDGET)?;
        Ok(())
    }
}

impl<'a> Machine<'a, '_> {
    /// Steps 1 to 3: storage, registration and the date stamp. Returns the
    /// definition's words.
    fn construct(&mut self, guid: u32) -> Result<&'a [i16; DEFINITION_WORDS], Trap> {
        let id = self.me;
        let definition = self.definition(guid)?;
        let words = self.definition_words(guid)?;
        if matches!(words[word::TYPE], 2 | 8 | 9) {
            return Err(Trap::Unsupported(format!(
                "the factory for object type {} is not ported",
                words[word::TYPE]
            )));
        }
        if words[word::GROUP] != 0 {
            return Err(Trap::Unsupported(
                "the factory for multi-tile objects is not ported".into(),
            ));
        }
        // The base constructor allocates the definition's attribute count,
        // read as a signed word, and never fewer than 8 (0x43189d).
        let attributes = usize::try_from(words[word::ATTRIBUTES]).unwrap_or(0).max(8);
        let sprites = usize::try_from(words[word::DYNAMIC_SPRITES]).map_err(|_| {
            Trap::Invalid(format!(
                "negative dynamic sprite count {}",
                words[word::DYNAMIC_SPRITES]
            ))
        })?;
        let containers = self
            .program
            .containers(definition.resource, definition.chunk);
        let mut data = vec![0i16; 72];
        data[11] = id as i16;
        // No slot, no room, and the number of objects it can hold
        // (0x431aaf).
        data[14] = -1;
        data[29] = -5;
        data[67] = containers as i16;
        // The date stamp: year, month, day, then hour and minute.
        let globals = &self.world.globals;
        if globals.len() < 9 {
            return Err(Trap::Invalid(format!(
                "the lot has {} globals; the factory reads its clock from them",
                globals.len()
            )));
        }
        (data[45], data[46], data[47]) = (globals[8], globals[7], globals[1]);
        (data[24], data[21]) = (globals[0], globals[5]);
        let object = Object {
            id,
            guid,
            // The footprint, the short after it, the frame arena size and
            // the relationship flag are what the original's saves hold for
            // an object the factory made and nothing placed. **Unverified**
            // against the factory itself; nothing in the simulation reads
            // them yet.
            footprint: [-24, -24, -8, -8],
            position: OFF_LOT,
            unknown_short: 1,
            attributes: vec![0; attributes],
            temps: vec![0; 8],
            data,
            obsolete_word: 0,
            stack_capacity_bytes: 512,
            stack: Vec::new(),
            relationship_flag: -1,
            relationships: Vec::new(),
            // Saved slot 0 is the tile chain; the containers follow.
            slots: vec![
                Slot {
                    unknown: 0,
                    object: 0,
                };
                containers + 1
            ],
            dynamic_sprites: vec![0; sprites],
            multipart: None,
            portal: None,
            person: None,
            // Countdown -1 and no flags.
            registry_word: Some(0xffff),
            last_result: false,
            removal_skips_resets: false,
        };
        self.world.objects.insert(id, object);
        // New objects go to the head of the registration list, and to the
        // end of their type's instance list if it has been made.
        self.world.order.insert(0, id);
        if let Some(instances) = self.world.instances.get_mut(&guid) {
            instances.push(id);
        }
        if !self.world.types.values().any(|&g| g == guid) {
            // Frames name the code they run by the lot's type table, which
            // the original only builds when it saves. A type the table
            // lacks takes the lowest unused ID.
            let type_id = (1..=i16::MAX as u16)
                .find(|id| !self.world.types.contains_key(id))
                .ok_or_else(|| Trap::Unsupported("no unused type ID".into()))?;
            self.world.types.insert(type_id, guid);
        }
        Ok(words)
    }

    /// Step 4, the storage part (0x431be1): clears the object's data,
    /// attributes and temps, keeping the words that describe where and
    /// when it was made. Returns whether Init runs: it does not for a tile
    /// of a multi-tile object, whose group resets as a whole.
    fn reset(&mut self, words: &[i16; DEFINITION_WORDS]) -> Result<bool, Trap> {
        let id = self.me;
        let globals = &self.world.globals;
        let (year, month, day) = (globals[8], globals[7], globals[1]);
        let object = self.me_mut()?;
        let old = std::mem::replace(&mut object.data, vec![0; 72]);
        let data = &mut object.data;
        for keep in [21, 24, 29, 41, 45, 46, 47, 67] {
            data[keep] = old[keep];
        }
        data[1] = old[1].rem_euclid(8);
        data[11] = id as i16;
        data[58] = words[word::DATA_58];
        // No container: container ID 0, slot -1. An object in a container
        // would get that container here; the factory never makes one.
        (data[2], data[3], data[14], data[26]) = (0, -1, -1, 0);
        // Age in years, and in months, by the lot's calendar, in the
        // original's wrapping 16-bit arithmetic.
        let mut years = year.wrapping_sub(old[45]);
        let mut months = month.wrapping_sub(old[46]);
        if day.wrapping_sub(old[47]) < 0 {
            months = months.wrapping_sub(1);
        }
        if months < 0 {
            months = months.wrapping_add(12);
            years = years.wrapping_sub(1);
        }
        data[18] = years;
        data[48] = months.wrapping_add(years.wrapping_mul(12));
        object.attributes.fill(0);
        object.temps.fill(0);
        object.set_countdown(-1);
        Ok(words[word::GROUP] == 0)
    }

    /// Step 4, the thread part (0x47480c): the Main tree becomes the only
    /// frame, with four zero arguments and no stack object.
    pub(crate) fn install_main(&mut self) -> Result<(), Trap> {
        let definition = self.definition(self.me()?.guid)?;
        let main = self.program.callback(definition, Callback::Main);
        if main == 0 {
            return Err(Trap::Unsupported(
                "an object whose definition names no Main tree".into(),
            ));
        }
        self.me_mut()?.stack.clear();
        self.enter(main, 0, &[0; 4], 4)
    }

    /// Step 7 (0x4521f7, 0x4457ef, 0x440e44).
    fn finish(&mut self, words: &[i16; DEFINITION_WORDS]) -> Result<(), Trap> {
        let advertises = self.advertises()?;
        let object = self.me_mut()?;
        // The tail of an object's turn counts the lockout word down.
        if object.data[LOCKOUT] != 0 {
            object.data[LOCKOUT] = object.data[LOCKOUT].wrapping_sub(1);
        }
        object.set_flags(flag::QUEUED, object.data[60] != 0);
        object.set_flags(flag::ADVERTISES, advertises);
        object.data[41] = words[word::PRICE];
        object.set_flags(flag::READY, true);
        Ok(())
    }

    /// Whether the object's interaction table has an entry Sims may pick
    /// by themselves (0x4358fc): one whose autonomy threshold is below 100,
    /// compared as a signed number, and whose flag bit 7 is clear.
    ///
    /// A tile of a multi-tile object advertises only if it is the tile
    /// interactions go to; the factory makes no such object, so that test
    /// is not ported.
    fn advertises(&self) -> Result<bool, Trap> {
        let definition = self.definition(self.me()?.guid)?;
        let table = self
            .program
            .interactions(definition.resource, definition.chunk)
            .map_err(Trap::Unsupported)?;
        Ok(table.is_some_and(|table| {
            table
                .interactions
                .iter()
                .any(|entry| (entry.autonomy as i32) < 100 && entry.flags & 128 == 0)
        }))
    }
}
