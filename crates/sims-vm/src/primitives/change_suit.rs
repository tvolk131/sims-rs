//! Change Suit/Accessory (opcode 6, a person's primitive at 0x45d962).
//!
//! Operand 0 is an index, operand 1 says what it indexes, and bit 0 of
//! operand 2 takes the suit or accessory off instead of putting it on.
//!
//! - **Operand 1 = 1, the suit.** Taking a suit off does nothing.
//!   Otherwise the index becomes the person's suit (person data word 8,
//!   +0x59c), and word 21 (+0x5b6) its variant: 0 for suits 0 to 7, else
//!   drawn from the active random stream (0x45d994). A child (word 58,
//!   the age, nonzero and below 18 signed) gets variant 0 or 1: on the suit it already
//!   wears, the other one; on a new suit, a draw's low bit. Anyone else
//!   gets a draw modulo 10, and on the suit it already wears, draws again
//!   until the variant differs.
//! - **Operand 1 = 0 or 2, an accessory.** Its name is string `index` of
//!   string table 304 in the global file (0) or the running code's file
//!   (2), lowered in ASCII (0x46f8f7). A missing file is native error
//!   0x2f, and a missing or empty string 0x30. The accessory goes on
//!   through the animator's virtual function 0xd8 (0x58d88f, "dress")
//!   with an empty binding, and comes off through 0xdc (0x58d919,
//!   "undress"). A failed suit lookup (0x5a044d) leaves an entry at the
//!   end with an empty name and binding. Our lookup miss traps if any
//!   avatar files stopped decoding, since they might name the suit.
//!
//! Any other operand 1 is native error 0x2f. The primitive always
//! branches true.

use crate::fault::Trap;
use crate::interpreter::Machine;
use sims_content::People;
use sims_formats::saved_objects::Accessory;

/// Person data words the suit is kept in (data at person +0x58c).
const SUIT: usize = 8;
const VARIANT: usize = 21;
const AGE: usize = 58;

/// The string table accessory names come from.
const ACCESSORIES: u16 = 0x130;

impl Machine<'_, '_> {
    pub(crate) fn change_suit(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        if self.me()?.person.is_none() {
            return Err(Trap::Unsupported(
                "Change Suit/Accessory on an object that is not a person: the general \
                 primitive table handles it, which is not ported"
                    .into(),
            ));
        }
        let [index, scope, flags, ..] = *operands;
        let off = flags & 1 != 0;
        match scope {
            1 if off => {}
            1 => self.change_into(index)?,
            0 | 2 => {
                let name = self.accessory_name(scope, index)?;
                let program = self.program;
                let me = self.me;
                if off {
                    let accessories = &mut self
                        .me_mut()?
                        .person
                        .as_mut()
                        .ok_or(Trap::native(0x1c, "object is not a person"))?
                        .accessories;
                    undress(accessories, &name);
                } else {
                    put_on(&program.people, self.world, me, name)?;
                }
            }
            _ => return Err(Trap::native(0x2f, "invalid accessory string table")),
        }
        Ok(true)
    }

    /// Puts on suit `index` (0x45d98e to 0x45da0d). It reads only the
    /// words its branch needs: none for suits 0 to 7, the suit and age
    /// otherwise, and the variant only when the suit is the one worn.
    fn change_into(&mut self, index: u8) -> Result<(), Trap> {
        let variant = if index < 8 {
            0
        } else {
            // A signed compare (0x45d9b4): a negative age is a child's.
            let age = self.known_word(AGE)?;
            let child = age != 0 && age < 18;
            if i16::from(index) == self.known_word(SUIT)? {
                let variant = self.known_word(VARIANT)?;
                if child {
                    i16::from(variant == 0)
                } else {
                    // Draws until the variant differs (0x45d9c3).
                    loop {
                        let drawn = (self.draw() % 10) as i16;
                        if drawn != variant {
                            break drawn;
                        }
                    }
                }
            } else if child {
                (self.draw() & 1) as i16
            } else {
                (self.draw() % 10) as i16
            }
        };
        let me = self.me;
        let data = &mut self
            .me_mut()?
            .person
            .as_mut()
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .data;
        for (i, value) in [(VARIANT, variant), (SUIT, i16::from(index))] {
            *data
                .get_mut(i)
                .ok_or_else(|| Trap::Invalid(format!("person data has no word {i}")))? = value;
        }
        for i in [VARIANT, SUIT] {
            self.world.routing.unknown_words.remove(&(me, i));
        }
        Ok(())
    }

    /// Person data word `i` of the running object, failing when the
    /// snapshot left it unknown.
    fn known_word(&self, i: usize) -> Result<i16, Trap> {
        let me = self.me;
        if self.world.routing.unknown_words.contains(&(me, i)) {
            return Err(Trap::Unsupported(format!(
                "Change Suit reads person {me}'s data word {i}, which is not known"
            )));
        }
        self.me()?
            .person
            .as_ref()
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .data
            .get(i)
            .copied()
            .ok_or_else(|| Trap::Invalid(format!("person data has no word {i}")))
    }

    /// The accessory named by string `index` of table 304 in the global
    /// file (scope 0) or the running code's file (scope 2), lowered.
    fn accessory_name(&self, scope: u8, index: u8) -> Result<String, Trap> {
        let file = match scope {
            0 => self.program.global,
            _ => self.owner(self.frame()?)?,
        };
        let library = self.program.libraries.get(&file).ok_or(Trap::native(
            0x2f,
            "no file for the accessory's string table",
        ))?;
        let name = library
            .strings
            .get(&ACCESSORIES)
            .and_then(|table| table.get(usize::from(index)))
            .filter(|name| !name.is_empty())
            .ok_or(Trap::native(0x30, "no accessory name"))?;
        Ok(name.to_ascii_lowercase())
    }
}

/// Puts on accessory `name` (`0x58d88f` → `0x58d903` → `0x583559` → `0x5a044d`),
/// looking it up in `people`'s avatar registry in runtime `.ndx` load order.
/// The lookup compares bytes exactly (`0x5a25dc`, `0x5a27f2`). A miss fails if
/// any avatar files stopped decoding, since they might name it. An empty name
/// is not looked up (`0x583569`): it gets an unnamed entry.
pub(crate) fn put_on(
    people: &People,
    world: &mut sims_sim::World,
    id: u16,
    name: String,
) -> Result<(), Trap> {
    let found = if name.is_empty() {
        false
    } else {
        match crate::people::resolve_suit_exists(people, world, &name) {
            Some(found) => found,
            None => {
                let errors = &people.suit_errors;
                return Err(Trap::Unsupported(format!(
                    "accessory {name:?} is put on, but no decoded avatar file names it. {} avatar \
                     files stopped decoding ({})",
                    errors.len(),
                    errors
                        .keys()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
    };
    let accessories = &mut world
        .objects
        .get_mut(&id)
        .and_then(|o| o.person.as_mut())
        .ok_or(Trap::native(0x1c, "object is not a person"))?
        .accessories;
    dress(accessories, name, found);
    Ok(())
}

/// Puts on accessory `name` (0x58d88f). Takes off any of that name, then
/// adds one at the end with an empty binding.
///
/// The original appends an empty entry, then fills it after binding its
/// suit to the skeleton (0x583559). The attach (0x599427, through
/// 0x5954d8) always returns its object, so a lookup miss (0x5a044d) is
/// the only way a nonempty requested name leaves the entry unnamed.
/// We use `People::suits` for the lookup. Its agreement with the
/// original registry and lazy loads (0x5a38da) is unverified.
pub(crate) fn dress(accessories: &mut Vec<Accessory>, name: String, found: bool) {
    undress(accessories, &name);
    accessories.push(Accessory {
        name: if found { name } else { String::new() },
        binding: String::new(),
    });
}

/// Takes off every accessory named exactly `name` (0x58d919; the compare,
/// 0x431726, matches bytes and length).
pub(crate) fn undress(accessories: &mut Vec<Accessory>, name: &str) {
    accessories.retain(|accessory| accessory.name != name);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Accessory {
        Accessory {
            name: name.into(),
            binding: "R_HAND".into(),
        }
    }

    #[test]
    fn dressing_replaces_an_accessory_of_the_same_name_at_the_end() {
        let mut list = vec![named("suit-book101"), named("hat")];
        dress(&mut list, "suit-book101".into(), true);
        let names: Vec<_> = list.iter().map(|a| (&*a.name, &*a.binding)).collect();
        assert_eq!(names, [("hat", "R_HAND"), ("suit-book101", "")]);
    }

    #[test]
    fn a_failed_dress_replaces_the_named_entry_with_an_empty_one_at_the_end() {
        let mut list = vec![named("hat"), named("cane"), named("hat")];
        dress(&mut list, "hat".into(), false);
        let entries: Vec<_> = list.iter().map(|a| (&*a.name, &*a.binding)).collect();
        assert_eq!(entries, [("cane", "R_HAND"), ("", "")]);
    }

    #[test]
    fn undressing_takes_off_every_exact_match_only() {
        let mut list = vec![named("hat"), named("Hat"), named("hat"), named("cane")];
        undress(&mut list, "hat");
        let names: Vec<_> = list.iter().map(|a| &*a.name).collect();
        assert_eq!(names, ["Hat", "cane"]);
    }
}
