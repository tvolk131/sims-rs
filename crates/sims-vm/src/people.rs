//! What a person's visit in load step 13 does beyond an object's
//! (0x459bf6): taking the person's lasting fields from its neighborhood
//! record, and the person's own reset.
//!
//! Ported from the old prototype: `persistent_person_fields` and
//! `attach_neighbor` (legacy/crates/sims-vm/src/loading.rs), `Reset::next`,
//! `reload_profile_fields` and `profile_age`
//! (legacy/crates/sims-vm/src/person_reset.rs), `clear_person_action`
//! (legacy/crates/sims-vm/src/action.rs), and `score_table`
//! (legacy/crates/sims-vm/src/person_loading.rs). The prototype compared
//! each with the original's code run alone. `docs/decisions/` says what
//! a trace of the original loading the Goth house confirms.

use std::cell::RefCell;
use std::collections::BTreeSet;

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::placement::capture;
use sims_content::{People, Profile, Skeleton};
use sims_sim::World;
use sims_sim::characters;
use std::borrow::Cow;

/// The callback table's entries a person's reset asks.
const RESET: usize = 3;
const CANCELLED: usize = 4;

/// Person data words.
mod field {
    /// The ID of the person's neighborhood record.
    pub const NEIGHBOR: usize = 31;
    pub const AGE: usize = 58;
    pub const SKIN: usize = 60;
    pub const GENDER: usize = 65;
}

/// The person data words a neighborhood record of a given revision keeps
/// for the person (the table at 0x423827). A record whose revision is
/// below 1 holds no person data and keeps none.
pub fn persistent_fields(revision: i32) -> Vec<usize> {
    if revision < 1 {
        return Vec::new();
    }
    let mut fields = vec![
        2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 16, 20, 15, 26, 17, 18, 46, 47, 48, 49, 50, 51, 52, 53,
        13, 14, 58, 60, 61,
    ];
    let later: [(i32, &[usize]); 9] = [
        (2, &[56, 57, 63]),
        (3, &[65]),
        (4, &[67, 68]),
        (5, &[69]),
        (6, &[74]),
        (7, &[54, 55, 75, 76]),
        (8, &[80, 81]),
        (9, &[82]),
        (10, &[83, 84, 85, 86]),
    ];
    for (required, extra) in later {
        if revision >= required {
            fields.extend_from_slice(extra);
        }
    }
    fields
}

/// The number a body string's age stands for, as the C runtime's `atoi`
/// reads it and a 16-bit field keeps it: leading white space, a sign, and
/// digits up to the first character that is not one.
fn age(text: &str) -> Result<i16, Trap> {
    let text = text.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut value = 0i64;
    for digit in digits.bytes().take_while(u8::is_ascii_digit) {
        value = value
            .checked_mul(10)
            .and_then(|v| v.checked_add(i64::from(digit - b'0')))
            .filter(|v| *v <= i64::from(i32::MAX))
            .ok_or_else(|| {
                Trap::Unsupported("a body string's age overflows 32 bits; unverified".into())
            })?;
    }
    Ok(if negative { -value } else { value } as i32 as i16)
}

/// The person data words a reset reloads from the type's body strings
/// (0x5888d3): words 22 and 23 are cleared, and the gender (65), the age
/// (58) and the skin tone (60) are read again. A string the table lacks
/// leaves its word alone. A gender that is none of the six known ones
/// leaves the word alone too; a skin tone that is none of the listed ones
/// gives 0. The first skin tone name is not a tone and is never matched.
/// Names are compared without regard to case.
pub fn reload_profile(data: &mut [i16], profile: &Profile) -> Result<(), Trap> {
    if data.len() <= field::GENDER {
        return Err(Trap::Invalid(format!(
            "a person with {} data words",
            data.len()
        )));
    }
    let years = profile.age.as_deref().map(age).transpose()?;
    if let Some(gender) = &profile.gender {
        const GENDERS: [&str; 6] = [
            "male",
            "female",
            "dogmale",
            "dogfemale",
            "catmale",
            "catfemale",
        ];
        if let Some(index) = GENDERS
            .iter()
            .position(|name| name.eq_ignore_ascii_case(gender))
        {
            data[field::GENDER] = index as i16;
        }
    }
    if let Some(years) = years {
        data[field::AGE] = years;
    }
    if let Some(skin) = &profile.skin {
        data[field::SKIN] = profile
            .tones
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, name)| name.eq_ignore_ascii_case(skin))
            .map_or(0, |(index, _)| index as i16);
    }
    data[22] = 0;
    data[23] = 0;
    Ok(())
}

/// The profile a person type's reset reads (0x5888d3). A type made while
/// the game ran ([`sims_sim::characters`]) reads its template's, with the
/// gender, age and skin tone strings Make New Character wrote. A type
/// with no profile in `people` fails.
pub(crate) fn profile<'p>(
    people: &'p People,
    world: &World,
    guid: u32,
) -> Result<Cow<'p, Profile>, Trap> {
    let made = world.characters.made.get(&guid);
    let template = made.map_or(guid, |made| made.template);
    let profile = people.profiles.get(&template).ok_or_else(|| {
        Trap::Unsupported(format!(
            "no profile was supplied for the person type 0x{guid:08X}"
        ))
    })?;
    let Some(made) = made else {
        return Ok(Cow::Borrowed(profile));
    };
    let mut profile = profile.clone();
    for (index, string) in [
        (characters::GENDER, &mut profile.gender),
        (characters::AGE, &mut profile.age),
        (characters::SKIN, &mut profile.skin),
    ] {
        if let Some(written) = made.strings.get(&index) {
            *string = Some(written.clone());
        }
    }
    Ok(Cow::Owned(profile))
}

/// A person type's skeleton: the one its body string 0 names. A type made
/// while the game ran names the skeleton Make New Character wrote, or its
/// template's; a skeleton named so is the one any person type of that
/// name resolved to, as the name alone chooses the file. A type with no
/// skeleton in `people` fails.
pub(crate) fn skeleton<'p>(
    people: &'p People,
    world: &World,
    guid: u32,
) -> Result<&'p Skeleton, Trap> {
    let made = world.characters.made.get(&guid);
    let template = made.map_or(guid, |made| made.template);
    let none = || {
        Trap::Unsupported(format!(
            "no skeleton was supplied for the person type 0x{guid:08X}"
        ))
    };
    let skeleton = people.skeletons.get(&template).ok_or_else(none)?;
    match made.and_then(|made| made.strings.get(&characters::SKELETON)) {
        Some(name) if !name.eq_ignore_ascii_case(&skeleton.name) => people
            .skeletons
            .values()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                Trap::Unsupported(format!(
                    "no person type's skeleton is named {name:?}, which the made person type \
                         0x{guid:08X} names"
                ))
            }),
        _ => Ok(skeleton),
    }
}

/// Reads person `id`'s appearance again if its animator says it is to be
/// read (`stale`, animator byte +0xd8), as looking up a clip does
/// (0x5893b3, 0x592625): the flag is cleared and the appearance read from
/// the person type's profile in [`crate::Program::people`]
/// ([`reload_profile`], the animator's virtual function 0x14).
///
/// A person type with no profile there fails.
pub(crate) fn read_appearance_if_stale(
    world: &mut World,
    program: &crate::Program,
    id: u16,
    stale: &mut bool,
) -> Result<(), Trap> {
    if !*stale {
        return Ok(());
    }
    let guid = world
        .objects
        .get(&id)
        .ok_or(Trap::native(10, "missing object"))?
        .guid;
    // Looked up before the person is borrowed; a missing profile still
    // fails after the person check.
    let profile = profile(&program.people, world, guid);
    let person = world
        .objects
        .get_mut(&id)
        .and_then(|object| object.person.as_mut())
        .ok_or(Trap::native(0x1c, "object is not a person"))?;
    let profile = profile?;
    *stale = false;
    reload_profile(&mut person.data, &profile)
}

/// Which string table holds a person's motive curves (0x459d23), as
/// whether it is one of the global file's and its ID: the table the
/// person's definition names in word 51 if that is not 0, and otherwise
/// the global table 501 for an adult and 503 for a child. A child is a
/// person whose age (person data word 58) is not 0 and below 18.
///
/// The original loads the curves at the end of a person's visit in load
/// step 13. Nothing here loads them yet. **Unverified** against the
/// running game; the old prototype compared the choice with the
/// original's code run alone.
pub fn curve_table(age: i16, custom: u16) -> (bool, u16) {
    if custom != 0 {
        (false, custom)
    } else {
        (true, if age == 0 || age >= 18 { 501 } else { 503 })
    }
}

impl Interpreter<'_> {
    /// Gives a person the fields its neighborhood record keeps for it
    /// (0x4258a3): the record's ID goes into person data word 31; then, if
    /// the record's revision is not 0, each word [`persistent_fields`]
    /// lists for the revision is copied from the record; last, the
    /// record's word 61 is copied whatever the revision (record +0xee).
    ///
    /// The record is the first of the person's type, in the file's slot
    /// order (0x425770). These fail: a person with no record, for which
    /// the original makes one (0x4257a5), and a record whose revision is
    /// below 0, which the original looks up in its table of fields
    /// (0x423827) past its start.
    pub(crate) fn attach_neighbor(&self, world: &mut World, id: u16) -> Result<(), Trap> {
        self.attach_record(world, id, |revision| {
            let mut fields = persistent_fields(revision);
            if !fields.contains(&61) {
                fields.push(61);
            }
            fields
        })
    }

    /// Gives a person the interests and personality its neighborhood
    /// record keeps (0x425930): the record's ID goes into person data word
    /// 31, and a record of revision 7 or later gives words 46 to 53, 13,
    /// 14, 16, 20, 26, 54 and 55. The record is found and refused as
    /// [`Interpreter::attach_neighbor`] finds and refuses it.
    pub(crate) fn attach_traits(&self, world: &mut World, id: u16) -> Result<(), Trap> {
        self.attach_record(world, id, |revision| {
            if revision >= 7 {
                vec![46, 47, 48, 49, 50, 51, 52, 53, 13, 14, 16, 20, 26, 54, 55]
            } else {
                Vec::new()
            }
        })
    }

    /// Sets person data word 31 to the ID of the first neighborhood record
    /// of the person's type and copies the words `fields` lists for the
    /// record's revision.
    fn attach_record(
        &self,
        world: &mut World,
        id: u16,
        fields: impl Fn(i32) -> Vec<usize>,
    ) -> Result<(), Trap> {
        let object = world
            .objects
            .get_mut(&id)
            .ok_or(Trap::native(10, "missing object"))?;
        let guid = object.guid;
        let person = object
            .person
            .as_mut()
            .ok_or(Trap::native(0x1c, "object is not a person"))?;
        let record = world.neighbors.first_of(guid).ok_or_else(|| {
            Trap::Unsupported(format!(
                "the person type 0x{guid:08X} has no neighborhood record; making one \
                 (0x4257a5) is not ported"
            ))
        })?;
        if record.revision < 0 {
            return Err(Trap::Unsupported(format!(
                "neighborhood record {} has revision {}; which fields it keeps is unknown",
                record.id, record.revision
            )));
        }
        let fields = fields(record.revision);
        let mut values = Vec::with_capacity(fields.len());
        for &word in &fields {
            if word >= person.data.len() {
                return Err(Trap::Invalid(format!("the person has no data word {word}")));
            }
            values.push(*record.data.get(word).ok_or_else(|| {
                Trap::Invalid(format!(
                    "neighborhood record {} has no person data word {word}",
                    record.id
                ))
            })?);
        }
        person.data[field::NEIGHBOR] = record.id;
        for (&word, value) in fields.iter().zip(values) {
            person.data[word] = value;
        }
        Ok(())
    }

    /// A person's reset (0x45e791 with no object named), for a person
    /// that is doing nothing:
    ///
    /// 1. Person data word 30 is cleared (0x59297d).
    /// 2. The person's animator is reset
    ///    ([`sims_sim::animator::Animator::reset`], 0x589220) and the
    ///    person's appearance reloaded ([`reload_profile`]). The reset
    ///    leaves the animator noting that the appearance is to be read
    ///    again, which the reload directly after does not clear.
    /// 3. Its headlines are cleared (0x45e8af,
    ///    [`sims_sim::World::headlines`]).
    /// 4. Its thread restarts at Main, and its Reset function (3) runs on
    ///    top of that.
    /// 5. Person data words 0, 40 and 59 and data words 34 and 9 are
    ///    cleared. So are the motive changes an action left running and
    ///    person data words 33, 71, 72 and 73.
    /// 6. Each queued action is dropped, first to last, and its target is
    ///    asked function 4 with the person as the stack object.
    ///
    /// Between steps 2 and 3 the person's route contexts (person +0xaf0)
    /// are erased (0x45e8aa, 0x45fad7), whose destructors (0x5d5653) only
    /// free memory. Its path (+0xae0) stays. Its headlines are cleared
    /// (0x45e8c3, 0x46e042), which changes nothing kept
    /// (`primitives::headline`).
    ///
    /// These fail, because the original does more for them and none was
    /// seen: a person that is using an object or whose running action has
    /// a target; a person inside another object (0x45e8dd) or, unless it is being
    /// deleted ([`Interpreter::reset_person_to_delete`]), off the lot
    /// (0x45e977), which the original moves; and a person whose data word
    /// 73 is set, whose footprint the original then works out again.
    ///
    /// So does an animator the reset cannot handle (see
    /// [`sims_sim::animator::Animator::reset`]).
    ///
    /// The person is marked as in the middle of a reset (object flag
    /// 0x800) throughout, so a broadcast to let go of an object that its
    /// callbacks set off skips it ([`crate::forget`]). A reset of a person
    /// already marked, which the original skips, is refused, as none was
    /// seen.
    ///
    /// Not modeled: the person's graphics and action icons.
    pub(crate) fn reset_person(
        &self,
        world: &mut World,
        id: u16,
        people: &People,
    ) -> Result<(), Fault> {
        self.reset_person_as(world, id, people, false)
    }

    /// A person's last reset, as its deletion asks it (virtual function
    /// 0x38 with 0 from 0x4410a5): [`Interpreter::reset_person`] with the
    /// person's deletion mark (object flag 0x40) set, which skips the
    /// move back onto the lot (0x45e95d) and leaves it where it is.
    pub(crate) fn reset_person_to_delete(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        self.reset_person_as(world, id, &self.program.people, true)
    }

    fn reset_person_as(
        &self,
        world: &mut World,
        id: u16,
        people: &People,
        deleting: bool,
    ) -> Result<(), Fault> {
        let Some(busy) = self.letting_go else {
            let busy = RefCell::new(BTreeSet::new());
            let outer = Interpreter {
                letting_go: Some(&busy),
                ..*self
            };
            return outer.reset_person_as(world, id, people, deleting);
        };
        if busy.borrow().contains(&id) {
            return Err(Fault {
                object: id,
                at: None,
                trap: Trap::Unsupported(format!(
                    "a reset of person {id} while it is letting go of an object or resetting \
                     (object flag 0x800), which the original skips; not ported"
                )),
            });
        }
        busy.borrow_mut().insert(id);
        // A reset that releases uses and then fails leaves the lot as it
        // was before the reset, so the fault reports a consistent lot.
        let using = world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .is_some_and(|p| !p.object_uses.is_empty());
        let before = using.then(|| world.clone());
        let result = self
            .release_uses(world, id)
            .and_then(|()| self.reset_marked(world, id, people, deleting));
        if result.is_err()
            && let Some(before) = before
        {
            *world = before;
        }
        busy.borrow_mut().remove(&id);
        result
    }

    /// The first step of a person's reset with no object named (0x45e814
    /// to 0x45e871): while the person uses an object (person +0xb00), the
    /// last use is ended.
    ///
    /// 1. The person's thread pops frames (0x4752f6, then the person's
    ///    virtual function 8, [`crate::interpreter::Machine::pop_frame`])
    ///    while it has more than the use's frame count less one. The last
    ///    pop releases the use ([`crate::interpreter::Machine::release_use`]),
    ///    and the person's running action ends when the list empties.
    /// 2. Unless the person has object flag 0x1000, every person lets go
    ///    of the object ([`Interpreter::drop_references`], 0x444306), which
    ///    skips this one, being in the middle of its reset, and the object
    ///    resets (its virtual function 0x38 with 0: [`Interpreter::reset`],
    ///    or [`Interpreter::reset_person`] for a person). Flag 0x1000 is
    ///    set only at 0x42de09, just before that object's own reset, which
    ///    is not ported, so it is clear here.
    ///
    /// The object is the one the entry pointed to when the step began.
    /// One that is gone by then fails loudly, as does a step that leaves
    /// the list as long as it was, which the original repeats forever.
    fn release_uses(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let uses = |world: &World| {
            world
                .objects
                .get(&id)
                .and_then(|o| o.person.as_ref())
                .map(|p| (p.object_uses.len(), p.object_uses.last().cloned()))
                .ok_or_else(|| fail(Trap::native(10, "missing object")))
        };
        loop {
            let (count, Some(last)) = uses(world)? else {
                return Ok(());
            };
            let target = u16::try_from(last.target)
                .ok()
                .filter(|t| *t > 0 && world.objects.contains_key(t))
                .ok_or_else(|| {
                    fail(Trap::Invalid(format!(
                        "the object {} person {id} uses is gone",
                        last.target
                    )))
                })?;
            let mut machine = self.machine(world, id, 0, false);
            while (machine.me().map_err(fail)?.stack.len() as i32) > last.stack_depth - 1 {
                machine.pop_frame().map_err(|trap| machine.fault(trap))?;
            }
            if !world.objects.contains_key(&target) {
                return Err(fail(Trap::Unsupported(format!(
                    "person {id}'s reset: object {target}, which it used, went as its frames \
                     were popped; the original resets it through a stale pointer (0x45e86e)"
                ))));
            }
            self.drop_references(world, target)?;
            if world
                .objects
                .get(&target)
                .is_some_and(|o| o.person.is_some())
            {
                self.reset_person(world, target, &self.program.people)?;
            } else {
                self.reset(world, target, false)?;
            }
            if uses(world)?.0 >= count {
                return Err(fail(Trap::Unsupported(format!(
                    "person {id}'s reset: ending its use of object {target} left as many uses, \
                     which the original repeats forever (0x45e814)"
                ))));
            }
        }
    }

    fn reset_marked(
        &self,
        world: &mut World,
        id: u16,
        people: &People,
        deleting: bool,
    ) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let unsupported = |what: &str| {
            fail(Trap::Unsupported(format!(
                "resetting a person {what} is not ported"
            )))
        };
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let guid = object.guid;
        let person = object
            .person
            .as_ref()
            .ok_or_else(|| fail(Trap::native(0x1c, "object is not a person")))?;
        if !person.object_uses.is_empty() {
            return Err(fail(Trap::Invalid(
                "a person still uses an object after its uses were released".into(),
            )));
        }
        if person.active.target != 0 {
            return Err(unsupported("whose running action has a target"));
        }
        if person.data.get(73).is_none_or(|&word| word != 0) {
            return Err(unsupported("whose data word 73 is set"));
        }
        let profile = profile(people, world, guid).map_err(fail)?;
        let place = capture(world, id).map_err(fail)?;
        if place.container.is_some() {
            return Err(unsupported("that is inside an object (0x45e8dd)"));
        }
        if !place.on_lot && !deleting {
            return Err(unsupported(
                "that is off the lot, which puts it back on the lot (0x45e977)",
            ));
        }
        let person = world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        person.data[30] = 0;
        // An animator that has not ticked is in the state a reset leaves
        // ([`sims_sim::animator::LOADED`]); one that has is reset.
        world
            .reset_animator(id)
            .map_err(|e| fail(Trap::Unsupported(e)))?;
        let person = world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        reload_profile(&mut person.data, &profile).map_err(fail)?;
        // 0x45e8aa: the route contexts are erased (0x45fad7).
        if let Some(route) = world.routing.routes.get_mut(&id) {
            route.contexts.clear();
        }
        // Every headline is cleared (0x45e8af).
        world.headlines.remove(&id);
        self.machine(world, id, 0, false)
            .install_main()
            .map_err(fail)?;
        self.callback(world, id, RESET, 0)?;
        let object = world
            .objects
            .get_mut(&id)
            .ok_or_else(|| fail(Trap::Invalid("the person's Reset deleted it".into())))?;
        object.data[34] = 0;
        object.data[9] = 0;
        let person = object
            .person
            .as_mut()
            .ok_or_else(|| fail(Trap::native(0x1c, "object is not a person")))?;
        for word in [0, 40, 59] {
            person.data[word] = 0;
        }
        if person.active.target != 0 {
            return Err(unsupported("whose Reset tree started an action"));
        }
        person.motive_deltas.clear();
        if person.data[73] != 0 {
            return Err(unsupported("whose data word 73 is set"));
        }
        for word in [33, 71, 72, 73] {
            person.data[word] = 0;
        }
        // One action at a time: the callback may change the rest of the
        // queue.
        loop {
            let person = world
                .objects
                .get_mut(&id)
                .and_then(|o| o.person.as_mut())
                .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
            if person.queue.is_empty() {
                break;
            }
            let action = person.queue.remove(0);
            if action.target != 0 {
                self.callback(world, action.target as u16, CANCELLED, id as i16)?;
            }
        }
        Ok(())
    }
}
