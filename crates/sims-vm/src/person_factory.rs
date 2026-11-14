//! The person factory: how the original makes a new person on a lot (the
//! person path of 0x440b2d, from 0x440d63).
//!
//! [`Interpreter::create_person`] follows it step by step:
//!
//! 1. The pet guard (0x440d63): a person type whose skeleton is named
//!    `dog` or `kat` (body string 1) is not made unless the
//!    installed-expansions mask (application field +0x66) has bit 0x20 or
//!    0x80. The mask is not modeled, so a pet fails here.
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
//!
//! Ported from the old prototype's `PostConstruction`, `ConstructionReset`,
//! `InitialReset`, `default_traits` and `attach_person_neighbor`
//! (legacy/crates/sims-vm/src/person_factory.rs), which compared each step
//! with the original's code run alone, without its request and reply
//! protocol.
//!
//! Compared with the original: the Goth house's tick 6, in which object
//! 382 makes person 259. The whole state after it, the C runtime's random
//! stream included, equals the capture's (`docs/decisions.md`). What
//! that person does not reach (a pet, a lot that is not residential, a
//! failure) is checked only by the synthetic tests.
//!
//! Not modeled, because nothing the simulation keeps depends on it: the
//! person's graphics and its skin, which the skeleton's bind builds
//! (0x58831d) and whose animator work the tick-0 update and the reset
//! overwrite; the definition's own setup (0x457f60 with 1); the autonomy
//! tables the first person made reads once (string tables 0x1f6 and
//! 0x1f8); the motive curves (0x459d23); and object flag 0x80.

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
    /// visitor; 2 kept as it is on a lot that is not residential.
    pub const VISITOR: usize = 32;
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
    /// is off the lot.
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
        let skeleton = self.program.people.skeletons.get(&guid).ok_or_else(|| {
            fail(
                0,
                Trap::Unsupported(format!(
                    "no skeleton was supplied for the person type 0x{guid:08X}"
                )),
            )
        })?;
        if ["dog", "kat"]
            .iter()
            .any(|pet| skeleton.name.eq_ignore_ascii_case(pet))
        {
            return Err(fail(
                0,
                Trap::Unsupported(format!(
                    "the person type 0x{guid:08X} is a pet ({:?}); whether the factory makes it \
                     depends on the installed-expansions mask, which is not modeled",
                    skeleton.name
                )),
            ));
        }
        let id = (1..=i16::MAX as u16)
            .find(|id| !world.objects.contains_key(id))
            .ok_or_else(|| fail(0, Trap::Unsupported("no unused object ID".into())))?;
        let words = self
            .machine(world, id, 0, false)
            .construct(guid, true)
            .map_err(|trap| fail(id, trap))?;
        // The post-constructor.
        let mut draws = Draws {
            stream: &mut world.crt_rng,
            count: 0,
        };
        build_skeleton(
            guid,
            &self.program.people,
            &mut draws,
            &mut world.skeleton_files,
        )
        .map_err(|trap| fail(id, trap))?;
        self.attach_neighbor(world, id)
            .map_err(|trap| fail(id, trap))?;
        self.default_traits_if_low(world, id)
            .map_err(|trap| fail(id, trap))?;
        self.construction_reset(world, id, words)?;
        let object = world
            .objects
            .get_mut(&id)
            .ok_or_else(|| fail(id, Trap::Invalid("the new person was deleted".into())))?;
        object.data[41] = words[word::PRICE];
        object.set_flags(1, true);
        Ok(id)
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
        let object = world
            .objects
            .get_mut(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let guid = object.guid;
        let person = object
            .person
            .as_mut()
            .ok_or_else(|| fail(Trap::native(0x1c, "object is not a person")))?;
        let kept = person::KEPT.map(|word| person.data[word]);
        person.data.fill(0);
        for (word, value) in person::KEPT.into_iter().zip(kept) {
            person.data[word] = value;
        }
        let profile = self.program.people.profiles.get(&guid).ok_or_else(|| {
            fail(Trap::Unsupported(format!(
                "no profile was supplied for the person type 0x{guid:08X}"
            )))
        })?;
        reload_profile(&mut person.data, profile).map_err(fail)?;
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
        let lot = world.globals.get(LOT).copied().unwrap_or(0);
        let residential = self.program.zoning.get(&lot) == Some(&1);
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
        world
            .animators
            .get_mut(&id)
            .ok_or_else(|| fail(Trap::Invalid("a new person without an animator".into())))?
            .reset()
            .map_err(|e| fail(Trap::Unsupported(e)))?;
        Ok(())
    }
}
