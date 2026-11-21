//! The person factory: how the original makes a new person on a lot (the
//! person path of 0x440b2d, from 0x440d63).
//!
//! [`Interpreter::create_person`] follows it step by step:
//!
//! 1. The pet guard (0x440d63): a person type whose skeleton is named
//!    `dog` or `kat` (body string 1) is not made unless the
//!    installed-expansions mask (application field +0x66,
//!    [`crate::Program::expansions`]) has bit 0x20 (Unleashed) or 0x80
//!    (Makin' Magic). A pet the mask does not allow fails here: the
//!    original returns 0, no person, and what its callers do then is not
//!    ported.
//! 2. The lowest unused object ID, the storage, and registration at the
//!    head of the registration list (0x44099b, 0x459327, 0x4434cd); see
//!    [`Machine::construct`].
//! 3. The post-constructor (0x45962b): the object's base fields (0x431aaf)
//!    and five slots of the person's own, which [`Machine::construct`]
//!    already made; the skeleton (0x588dcb), whose own copy draws from the
//!    C runtime's random stream ([`build_skeleton`]); the neighborhood
//!    record (0x4258a3, [`Interpreter::attach_neighbor`]); and default
//!    interests and personality if interests 46 to 53 are all 10 or less
//!    (0x459a9c), which draw too.
//! 4. The date stamp (0x431df3), also made in [`Machine::construct`].
//! 5. The construction reset (0x4598e8 with 1), in [`construction_reset`].
//! 6. Data word 41 takes the definition's price, and the scheduler's
//!    ready flag is set (0x4457ef).
//! 7. The factory announces the new object (signal 0xe2, 0x440ed7), and
//!    for a person the lot works out again who is a visitor (0x4163f7,
//!    0x42a2b4; [`Interpreter::visitor_pass`]).
//!
//! Ported from the old prototype's `PostConstruction`, `ConstructionReset`,
//! `InitialReset`, `default_traits` and `attach_person_neighbor`
//! (legacy/crates/sims-vm/src/person_factory.rs), which compared each step
//! with the original's code run alone, without its request and reply
//! protocol.
//!
//! Compared with the original: the Goth house's tick 6, in which object
//! 382 makes person 259. The whole state after it, the C runtime's random
//! stream included, equals the capture's (`docs/decisions/`). What
//! that person does not reach (a pet, a lot that is not residential, a
//! failure) is checked only by the synthetic tests.
//!
//! Not modeled, because nothing the simulation keeps depends on it: the
//! person's graphics and its skin, which the skeleton's bind builds
//! (0x58831d) and whose animator work the tick-0 update and the reset
//! overwrite; the definition's own setup (0x457f60 with 1); the autonomy
//! tables the first person made reads once (string tables 0x1f6 and
//! 0x1f8); the motive curves (0x459d23); and object flag 0x80.

use crate::Program;
use crate::factory::word;
use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::objects::{Draws, INTERESTS, build_skeleton};
use crate::people::reload_profile;
use sims_sim::World;

/// The global that holds the lot's number.
const LOT: usize = 10;

/// Person data words.
mod person {
    /// Kept by the construction reset.
    pub const KEPT: [usize; 3] = [32, 34, 35];
    /// 0x7ae means: do not attach to the neighborhood record again.
    pub const NO_ATTACH: usize = 25;
    /// 0 for a member of the household that lives on the lot; 1 for a
    /// visitor; 2 kept as it is by the visitor pass, unless the person is
    /// a member of the family.
    pub const VISITOR: usize = 32;
}

/// The pet guard's test of the installed-expansions mask (0x440d8b to
/// 0x440dab): with bit 0x20 (Unleashed) or bit 0x80 (Makin' Magic) set, a
/// pet is made like any other person.
fn pets_allowed(expansions: u16) -> bool {
    expansions & (0x20 | 0x80) != 0
}

/// Whether interests 46 to 53 are all 10 or less, compared as signed
/// words (0x45988a, 0x459942).
fn interests_low(data: &[i16]) -> bool {
    INTERESTS
        .clone()
        .all(|word| data.get(word).is_some_and(|&v| v <= 10))
}

impl Interpreter<'_> {
    /// Makes a person of the type `guid` names and returns its ID. Its
    /// Init has run twice and its first update once when this returns; it
    /// is off the lot, and every person's word 32 has been worked out
    /// again (the visitor pass, 0x42a2b4).
    ///
    /// These fail: a type that is not a person's; a pet the game does not
    /// allow (step 1); a type with no skeleton or profile in
    /// [`crate::Program::people`]; a type with no neighborhood record, for
    /// which the original makes one (0x4257a5); and anything the person's
    /// scripts or first update reach that is not ported.
    ///
    /// On a fault the person stays registered, as far as it got.
    pub fn create_person(&self, world: &mut World, guid: u32) -> Result<u16, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let skeleton =
            crate::people::skeleton(&self.program.people, world, guid).map_err(|t| fail(0, t))?;
        if !pets_allowed(self.program.expansions)
            && ["dog", "kat"]
                .iter()
                .any(|pet| skeleton.name.eq_ignore_ascii_case(pet))
        {
            return Err(fail(
                0,
                Trap::Unsupported(format!(
                    "0x440dd5: the person type 0x{guid:08X} is a pet ({:?}) and the \
                     installed-expansions mask 0x{:04X} has neither Unleashed (0x20) nor \
                     Makin' Magic (0x80), so the factory makes no person; what its caller does \
                     then is not ported",
                    skeleton.name, self.program.expansions
                )),
            ));
        }
        self.construct_person(world, guid, skeleton)
    }

    /// Reads body row 0 (skeleton name, `0x440d63` → `0x457f60` → `0x4704fa`)
    /// without requiring the `.bcf` skeleton itself to be loaded, checking
    /// runtime-made overrides first, then the definition's `STR#` body table,
    /// then [`sims_content::People::skeleton_names`] / `skeletons`.
    fn early_skeleton_name<'s>(&'s self, world: &'s World, guid: u32) -> Option<&'s str> {
        if let Some(name) = world
            .characters
            .string(guid, sims_sim::characters::SKELETON)
        {
            return Some(name);
        }
        let template = world.characters.definition(guid);
        if let Some(def) = crate::interpreter::object_definition(self.catalog, world, guid)
            && let Some(library) = self.program.libraries.get(&def.resource)
            && let Some(words) = library.definitions.get(&def.chunk)
            && let Some(row0) = library
                .strings
                .get(&(words[19] as u16))
                .and_then(|rows| rows.first())
            && !row0.is_empty()
        {
            return Some(row0.as_str());
        }
        if let Some(name) = self.program.people.skeleton_names.get(&template) {
            return Some(name.as_str());
        }
        self.program
            .people
            .skeletons
            .get(&template)
            .map(|s| s.name.as_str())
    }

    /// Like [`Self::create_person`], but returns `Ok(None)` when the native
    /// factory refuses a pet because neither Unleashed nor Makin' Magic is
    /// installed (`0x440dd5`).
    pub(crate) fn try_create_person(
        &self,
        world: &mut World,
        guid: u32,
    ) -> Result<Option<u16>, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        // 0x440d63..0x440dd8 checks body string 0 ("dog" / "kat") for both
        // runtime-made and catalog pets before constructing cXPerson or
        // loading the skeleton.
        if !pets_allowed(self.program.expansions)
            && self.early_skeleton_name(world, guid).is_some_and(|name| {
                ["dog", "kat"]
                    .iter()
                    .any(|pet| name.eq_ignore_ascii_case(pet))
            })
        {
            return Ok(None);
        }
        let skeleton =
            crate::people::skeleton(&self.program.people, world, guid).map_err(|t| fail(0, t))?;
        if !pets_allowed(self.program.expansions)
            && ["dog", "kat"]
                .iter()
                .any(|pet| skeleton.name.eq_ignore_ascii_case(pet))
        {
            return Ok(None);
        }
        self.construct_person(world, guid, skeleton).map(Some)
    }

    fn construct_person(
        &self,
        world: &mut World,
        guid: u32,
        skeleton: &sims_content::Skeleton,
    ) -> Result<u16, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let id = (1..=i16::MAX as u16)
            .find(|id| !world.objects.contains_key(id))
            .ok_or_else(|| fail(0, Trap::Unsupported("no unused object ID".into())))?;
        let factory = Interpreter {
            making_person: Some(id),
            ..*self
        };
        let words = factory
            .machine(world, id, 0, false)
            .construct(guid, true)
            .map_err(|trap| fail(id, trap))?;
        // The person joins the end of the person vector. A load's people
        // keep its order, and Set To Next in tick 409 of the Goth house
        // shows a person made at run time coming after them; when in the
        // making the original appends it is not read, so while the
        // factory runs scripts a walk of the vector traps
        // ([`Interpreter::making_person`]).
        world.people.push(id);
        // The post-constructor.
        let mut draws = Draws {
            stream: &mut world.crt_rng,
            count: 0,
        };
        build_skeleton(skeleton, &mut draws, &mut world.skeleton_files)
            .map_err(|trap| fail(id, trap))?;
        factory
            .attach_neighbor(world, id)
            .map_err(|trap| fail(id, trap))?;
        factory
            .default_traits_if_low(world, id)
            .map_err(|trap| fail(id, trap))?;
        factory.construction_reset(world, id, words)?;
        let object = world
            .objects
            .get_mut(&id)
            .ok_or_else(|| fail(id, Trap::Invalid("the new person was deleted".into())))?;
        object.data[41] = words[word::PRICE];
        object.set_flags(1, true);
        // The factory's end announces the new object (signal 0xe2,
        // 0x440ed7), and the lot's handler works out again who is a
        // visitor when it is a person (0x4163f7).
        self.visitor_pass(world).map_err(|trap| fail(id, trap))?;
        Ok(id)
    }

    /// Whether the lot is residential (0x40745f): its zoning (0x42b9e3,
    /// by the lot number in global 10) is 1. Lot 0 has zoning 1.
    fn residential(&self, world: &World) -> bool {
        residential(self.program, world)
    }

    /// The visitor pass (0x42a2b4), which the lot runs when the factory
    /// announces a new person (signal 0xe2, 0x440ed7, handled at
    /// 0x4163f7): every person's word 32 is worked out again
    /// ([`sims_sim::load::visitor_again`]), the new one's too.
    ///
    /// The family is the selected one (game +0x10), whose members the
    /// pass tests (0x41019b); it is taken to be the family global 9 names,
    /// as for scope 24 field 1. With global 9 at 0 the World does not say
    /// which family the game holds, so that fails loudly, as does a
    /// global 9 that names no family. With no person in the person vector
    /// the pass returns before it reads the family (0x42a2cc), so nothing
    /// fails. The lot is residential when global 10 is 0 or a house zoned
    /// 1 (0x40745f, 0x42b9e3).
    pub(crate) fn visitor_pass(&self, world: &mut World) -> Result<(), Trap> {
        visitor_pass(self.program, world)
    }

    /// Gives person `id` default interests and personality (0x459a9c) if
    /// interests 46 to 53 are all 10 or less. The values are drawn from
    /// the C runtime's random stream ([`Draws::default_traits`]).
    fn default_traits_if_low(&self, world: &mut World, id: u16) -> Result<(), Trap> {
        let object = world
            .objects
            .get_mut(&id)
            .ok_or(Trap::native(10, "missing object"))?;
        let person = object
            .person
            .as_mut()
            .ok_or(Trap::native(0x1c, "object is not a person"))?;
        if !interests_low(&person.data) {
            return Ok(());
        }
        let mut draws = Draws {
            stream: &mut world.crt_rng,
            count: 0,
        };
        for (word, value) in draws.default_traits() {
            person.data[word] = value;
        }
        Ok(())
    }

    /// A new person's construction reset (0x4598e8 with 1):
    ///
    /// 1. All 256 person data words are cleared but 32, 34 and 35.
    /// 2. The appearance is read again ([`reload_profile`], the animator's
    ///    virtual function 0x14).
    /// 3. Whether interests 46 to 53 are all 10 or less is noted. They
    ///    are, since they were just cleared.
    /// 4. The person is attached to its neighborhood record
    ///    ([`Interpreter::attach_neighbor`]).
    /// 5. The object's reset (0x431be1 with 1): its storage is reset and
    ///    its Main tree installed, as for any new object; then, for an object that
    ///    is not part of a group, Init runs, the person's update runs once
    ///    with tick 0 ([`Interpreter::tick_person`], the object's virtual
    ///    function 0x14), and the scheduler flags are refreshed.
    /// 6. Init runs again (0x422d46).
    /// 7. If person word 25 is 0x7ae it is cleared; otherwise the person is
    ///    attached to its record again.
    /// 8. On a lot that is not residential, person word 32 becomes 1
    ///    unless it is 2 (0x40745f: the lot's zoning is 1).
    /// 9. If the interests were low at step 3: if they still are, default
    ///    interests and personality are made (0x459a9c). Otherwise the
    ///    record's interests and personality are copied (0x425930). (Step 3
    ///    makes the second branch unreachable here.)
    /// 10. The animator is reset ([`sims_sim::animator::Animator::reset`],
    ///     its virtual function 0xc).
    /// 11. The motive curves are loaded (0x459d23; not modeled).
    pub(crate) fn construction_reset(
        &self,
        world: &mut World,
        id: u16,
        words: &[i16; crate::program::DEFINITION_WORDS],
    ) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let tuning = self
            .program
            .motive_tuning()
            .map_err(|e| fail(Trap::Unsupported(e.to_string())))?;
        let guid = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?
            .guid;
        // Looked up before the person is borrowed; a missing profile still
        // fails where it is read.
        let profile = crate::people::profile(&self.program.people, world, guid);
        let person = world
            .objects
            .get_mut(&id)
            .and_then(|object| object.person.as_mut())
            .ok_or_else(|| fail(Trap::native(0x1c, "object is not a person")))?;
        let kept = person::KEPT.map(|word| person.data[word]);
        person.data.fill(0);
        for (word, value) in person::KEPT.into_iter().zip(kept) {
            person.data[word] = value;
        }
        let profile = profile.map_err(fail)?;
        reload_profile(&mut person.data, &profile).map_err(fail)?;
        let low = interests_low(&person.data);
        self.attach_neighbor(world, id).map_err(fail)?;
        let mut machine = self.machine(world, id, 0, false);
        let run_init = machine
            .reset(words)
            .and_then(|run_init| machine.install_main().map(|()| run_init))
            .map_err(fail)?;
        if run_init {
            self.init(world, id)?;
            // What the update returns is ignored (0x431d7f).
            self.tick_person(world, id, 0, &tuning)?;
            self.machine(world, id, 0, false)
                .refresh_flags()
                .map_err(fail)?;
        }
        self.init(world, id)?;
        let residential = self.residential(world);
        let person = world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .ok_or_else(|| fail(Trap::Invalid("the new person's Init deleted it".into())))?;
        if person.data[person::NO_ATTACH] == 0x7ae {
            person.data[person::NO_ATTACH] = 0;
        } else {
            self.attach_neighbor(world, id).map_err(fail)?;
        }
        let person = world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        if !residential && person.data[person::VISITOR] != 2 {
            person.data[person::VISITOR] = 1;
        }
        if low {
            self.default_traits_if_low(world, id).map_err(fail)?;
        } else {
            self.attach_traits(world, id).map_err(fail)?;
        }
        if !world
            .reset_animator(id)
            .map_err(|e| fail(Trap::Unsupported(e)))?
        {
            return Err(fail(Trap::Invalid(
                "a new person without an animator".into(),
            )));
        }
        Ok(())
    }
}

/// Whether the lot is residential (0x40745f): its zoning (0x42b9e3, by the
/// lot number in global 10) is 1. Lot 0 has zoning 1.
fn residential(program: &Program, world: &World) -> bool {
    let lot = world.globals.get(LOT).copied().unwrap_or(0);
    lot == 0 || program.zoning.get(&lot) == Some(&1)
}

/// The visitor pass (0x42a2b4): see [`Interpreter::visitor_pass`].
pub(crate) fn visitor_pass(program: &Program, world: &mut World) -> Result<(), Trap> {
    if !world.people.iter().any(|id| world.objects.contains_key(id)) {
        return Ok(());
    }
    let family = i32::from(world.globals.get(9).copied().unwrap_or(0));
    if family == 0 {
        return Err(Trap::Unsupported(
            "0x42a2b4: the visitor pass with global 9 at 0: which family the game holds \
             (game +0x10) is not kept"
                .into(),
        ));
    }
    let members = world
        .families
        .iter()
        .find(|f| f.id == family)
        .map(|f| f.members.clone())
        .ok_or_else(|| Trap::Invalid(format!("global 9 names family {family}, which is gone")))?;
    let residential = residential(program, world);
    for id in world.people.clone() {
        let Some(object) = world.objects.get_mut(&id) else {
            continue;
        };
        let member = members.contains(&object.guid);
        if let Some(word) = object
            .person
            .as_mut()
            .and_then(|p| p.data.get_mut(person::VISITOR))
        {
            *word = sims_sim::load::visitor_again(*word, member, residential);
        }
    }
    Ok(())
}
