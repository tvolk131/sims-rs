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
//! each with the original's code run alone. `docs/decisions.md` says what
//! a trace of the original loading the Goth house confirms.

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::placement::capture;
use sims_content::{People, Profile};
use sims_sim::World;

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
    /// (0x4258a3): the record's ID goes into person data word 31, and
    /// then each word [`persistent_fields`] lists for the record's
    /// revision is copied from the record.
    ///
    /// The record is the first of the person's type, in the file's slot
    /// order. A person with no record, and a record whose revision is
    /// below 1, fail: the original makes a record for the one and copies
    /// only the family for the other, and neither was seen.
    pub(crate) fn attach_neighbor(&self, world: &mut World, id: u16) -> Result<(), Trap> {
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
                 during a load is not ported"
            ))
        })?;
        if record.revision < 1 {
            return Err(Trap::Unsupported(format!(
                "neighborhood record {} holds no person data (revision {}); attaching it is \
                 unverified",
                record.id, record.revision
            )));
        }
        let fields = persistent_fields(record.revision);
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
    /// 1. Person data word 30 is cleared.
    /// 2. The person's appearance is reloaded ([`reload_profile`]).
    /// 3. Its thread restarts at Main, and its Reset function (3) runs on
    ///    top of that.
    /// 4. Person data words 0, 40 and 59 and data words 34 and 9 are
    ///    cleared. So are the motive changes an action left running and
    ///    person data words 33, 71, 72 and 73.
    /// 5. Each queued action is dropped, first to last, and its target is
    ///    asked function 4 with the person as the stack object.
    ///
    /// These fail, because the original does more for them and none was
    /// seen: a person that is using an object or whose running action has
    /// a target; a person in the middle of a route; a person inside
    /// another object or off the lot, which the original moves; and a
    /// person whose data word 73 is set, whose footprint the original
    /// then works out again.
    ///
    /// A person whose animator has ticked fails too: resetting the
    /// animator (0x589220) is not ported.
    ///
    /// Not modeled: the person's graphics and action icons, and the guard
    /// against a reset inside a reset (object flag 0x800).
    pub(crate) fn reset_person(
        &self,
        world: &mut World,
        id: u16,
        people: &People,
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
            return Err(unsupported("that is using an object"));
        }
        if person.active.target != 0 {
            return Err(unsupported("whose running action has a target"));
        }
        if person.routing_frames != 0 {
            return Err(unsupported("in the middle of a route"));
        }
        if person.data.get(73).is_none_or(|&word| word != 0) {
            return Err(unsupported("whose data word 73 is set"));
        }
        // The reset also resets the person's animator (0x589220), which is
        // not ported. Every reset seen so far is at the load, before the
        // animator has ticked, where it is taken to give
        // [`sims_sim::animator::LOADED`].
        if world.animators.contains_key(&id) {
            return Err(unsupported("whose animator has ticked"));
        }
        let profile = people.profiles.get(&guid).ok_or_else(|| {
            fail(Trap::Unsupported(format!(
                "no profile was supplied for the person type 0x{guid:08X}"
            )))
        })?;
        let place = capture(world, id).map_err(fail)?;
        if place.container.is_some() || !place.on_lot {
            return Err(unsupported("that is inside an object or off the lot"));
        }
        let person = world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        person.data[30] = 0;
        reload_profile(&mut person.data, profile).map_err(fail)?;
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
