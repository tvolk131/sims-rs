//! Make New Character (opcode 19, 0x44ae08): a new person type cloned
//! from a template, with a neighbour record for it.
//!
//! The general table (0x45246a) sends opcode 19 to 0x44ae08. Baby's
//! "process" tree reaches it when a baby grows into a child (tree 0x1010),
//! and a debug interaction for pets.
//!
//! **Operands.** Bytes 0, 1 and 2 index the frame's locals (0x47518f):
//! the skin tone (string local + 1, counting from 1, of STR# 0x2000), the
//! age in years (below 18 a child; negative copies the running object's
//! own body, 0x44b23c), and the sex (0 male, any other female). They are
//! read in the order b0, b2, b1 (0x44ae23, 0x44ae49, 0x44ae70), and an
//! index past the locals throws native error 8 (b0 at 0x44b3b3, b2 at
//! 0x44b39e, b1 at 0x44b389). A local that starts as half of a pointer
//! fails loudly. Byte 3 picks the template (1 TemplateDog, 2 TemplateCat,
//! any other value TemplatePerson, 0x4260fa).
//!
//! **The clone (0x4260e1).** The template's file is copied to
//! `Characters/User%05d.iff`, each object definition in the copy getting a
//! new GUID (0x4725d5); the copy is registered, and the first person
//! definition no record links to gets a new record (0x43ac3a, 0x425770,
//! 0x4257a5). The port keeps the made type in [`World::characters`]
//! ([`sims_sim::characters`]): see [`Machine::clone_template`].
//!
//! **The body (0x44aef7 to 0x44b13f).** The skin tone's name must not be
//! empty (native error 0x3b, 0x44af58, after the clone, whose type and
//! record stay). Then the Character helper writes body strings 13 (the
//! age, "9" or "27"), 0 (the skeleton, "child" or "adult"), 14 (the skin
//! tone) and 12 (the gender), and counts the bodies and heads the type can
//! wear ([`sims_content::Wardrobe::count`]).
//!
//! **Mode 0, a person (0x44b13f).** With both counts above 0, one game
//! draw picks the head and a second the body (0x46f457); the type is
//! committed, named, and the stack object takes the new record's ID.
//!
//! **Presentation, not kept:** the copied file, the strings written to
//! disk, the derived strings (0x409315, 0x40a588), the name (0x6e4ac0,
//! STR# 0x8e string 5, "No childname"), and which head and body the draws
//! pick (body strings 2 and 1), which only the renderer reads.
//!
//! **Not ported, and failing loudly:** a negative age (0x44b23c), a
//! template byte past 2 (0x44afa6, which reads an undefined gender code),
//! the pets (0x44b051), a count of 0 (0x44b203, whose 0x426fcd deletes the
//! type, its record and its objects), a clone the original fails
//! (0x426335, after which it reads an undefined pointer: no template, or
//! no record of it), and a template whose GUID more than one definition
//! has.

use sims_sim::characters::{self, Character};

use crate::fault::Trap;
use crate::interpreter::Machine;

/// The templates, by template byte (0x4260fa).
const TEMPLATE_PERSON: u32 = 0x7fd9_6b54;
const TEMPLATE_DOG: u32 = 0x4a70_df92;
const TEMPLATE_CAT: u32 = 0x7bea_0977;

/// The file number the counter goes back to 0 at (0x42615b).
const FILES: i32 = 0x7d00;

/// The first age that is not a child's (0x44af66).
const ADULT_AGE: i16 = 18;

impl Machine<'_, '_> {
    pub(crate) fn make_new_character(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let local = |byte: usize, at: u32| -> Result<i16, Trap> {
            let index = usize::from(operands[byte]);
            let value = self
                .frame()?
                .locals
                .get(index)
                .copied()
                .ok_or(Trap::native(8, "Make New Character local out of range"))?;
            self.check_local(index).map_err(|trap| match trap {
                Trap::Unsupported(message) => {
                    Trap::Unsupported(format!("Make New Character (0x{at:x}) {message}"))
                }
                other => other,
            })?;
            Ok(value)
        };
        let skin = local(0, 0x44ae23)?;
        let sex = local(2, 0x44ae49)?;
        let age = local(1, 0x44ae70)?;
        let species = operands[3];
        let template = match species {
            1 => TEMPLATE_DOG,
            2 => TEMPLATE_CAT,
            _ => TEMPLATE_PERSON,
        };
        let (guid, record) = self.clone_template(template)?;
        if age < 0 {
            return Err(Trap::Unsupported(format!(
                "Make New Character with a negative age ({age}) copies the running object's \
                 body (0x44b23c), which is not ported"
            )));
        }
        // The skin tone's name: string local + 1 of STR# 0x2000 in the
        // type's semi-global file, counting from 1 (0x47028b, 0x4704fa),
        // which is the template's.
        let tone = crate::people::profile(&self.program.people, self.world, guid)?
            .tones
            .get(usize::try_from(skin).unwrap_or(usize::MAX))
            .filter(|tone| !tone.is_empty())
            .cloned()
            .ok_or(Trap::native(
                0x3b,
                "Make New Character skin tone has no name",
            ))?;
        let female = sex != 0;
        let gender = match species {
            0 => usize::from(female),
            1 | 2 => {
                return Err(Trap::Unsupported(format!(
                    "Make New Character of a pet (template byte {species}) picks its body \
                     from the pet lists and the dialog's answer (0x44b051), which is not ported"
                )));
            }
            _ => {
                return Err(Trap::Unsupported(format!(
                    "Make New Character with template byte {species} passes an undefined \
                     gender code to the Character helper (0x44afa6)"
                )));
            }
        };
        let child = age < ADULT_AGE;
        let made = self
            .world
            .characters
            .made
            .get_mut(&guid)
            .ok_or(Trap::Invalid("a made person type went missing".into()))?;
        // 0x40cc2a: the age, then the skeleton (the template is no pet,
        // 0x40cee4); 0x40d02f: the skin tone; 0x40ce41: the gender.
        let strings = [
            (characters::AGE, if child { "9" } else { "27" }),
            (characters::SKELETON, if child { "child" } else { "adult" }),
            (characters::SKIN, tone.as_str()),
            (characters::GENDER, ["male", "female"][gender]),
        ];
        for (index, string) in strings {
            made.strings.insert(index, string.to_owned());
        }
        let (bodies, heads) = self.suit_counts(&tone, female, child)?;
        if bodies == 0 || heads == 0 {
            return Err(Trap::Unsupported(format!(
                "Make New Character found {bodies} bodies and {heads} heads for the skin tone \
                 {tone:?}; with none it deletes the type and its record (0x44b203, 0x426fcd), \
                 which is not ported"
            )));
        }
        // The head, then the body (0x44b151, 0x44b167). Which they pick
        // is presentation (body strings 2 and 1, 0x40d46f, 0x40d10b).
        let _head = self.draw() % heads;
        let _body = self.draw() % bodies;
        self.frame_mut()?.stack_object = record;
        Ok(true)
    }

    /// Clones `template` (0x4260e1 with no slot asked for): the made type
    /// and its record, which is returned with the type's GUID.
    ///
    /// - **The template.** The template's definition is the one the first
    ///   neighbour record of its GUID links to (0x42571f, record +8); with
    ///   no such record the clone fails (0x426335). The port's records
    ///   link to the definition the GUID resolves to, so more than one
    ///   definition of the template's GUID fails here.
    /// - **The file number.** The copy tries the neighborhood's counter,
    ///   and again with the counter one higher while the file exists
    ///   (0x4722b8 returning -0x2b); the counter goes up after each try and
    ///   back to 0 at 32000, where the clone fails. The port writes no
    ///   files: a number is taken when the neighborhood's `Characters`
    ///   folder held its file at boot ([`crate::Program::character_files`])
    ///   or a made type holds it. That the counter starts at 0 is
    ///   **unverified**.
    /// - **The GUID.** For each object definition in the copy, 0x4725d5
    ///   writes timeGetTime << 16 XOR one C runtime draw (0x4725ff,
    ///   0x636c74) at OBJD byte 0x1c, and the old GUID at byte 0x8c. The
    ///   port takes the low 16 bits of [`sims_sim::World::ticks`] for the
    ///   clock, keeping the draw: an **unverified** substitute. A template
    ///   whose file has more than one definition fails. The original
    ///   checks no collision and overwrites its map (0x43bda9); a GUID
    ///   another type has fails here.
    /// - **The record.** The copy's definitions are registered at the end
    ///   (0x43c33d), and the first person definition no record links to
    ///   gets one (0x4257a5). Every other person definition has its
    ///   record, as the neighborhood's load made them and the port deletes
    ///   none, so that is the new one.
    fn clone_template(&mut self, template: u32) -> Result<(u32, i16), Trap> {
        let missing = || {
            Trap::Unsupported(format!(
                "Make New Character's template 0x{template:08X} is not registered; the clone \
                 fails (0x426335) and the original reads an undefined pointer (0x44aec1)"
            ))
        };
        let definition = self.catalog.object(template).ok_or_else(missing)?;
        if self.world.neighbors.first_of(template).is_none() {
            return Err(Trap::Unsupported(format!(
                "Make New Character's template 0x{template:08X} has no neighbour record; the \
                 clone fails (0x426335) and the original reads an undefined pointer (0x44aec1)"
            )));
        }
        let registered = self
            .catalog
            .objects
            .iter()
            .filter(|o| o.guid == template)
            .count();
        if registered != 1 {
            return Err(Trap::Unsupported(format!(
                "Make New Character's template 0x{template:08X} has {registered} definitions; \
                 which one its record links to (0x42571f) is not ported"
            )));
        }
        let definitions = self
            .catalog
            .objects
            .iter()
            .filter(|o| o.resource == definition.resource)
            .count();
        if definitions != 1 {
            return Err(Trap::Unsupported(format!(
                "Make New Character's template 0x{template:08X} shares its file with \
                 {definitions} definitions, each of which the copy gives a new GUID (0x47249d)"
            )));
        }
        let files = &self.program.character_files;
        let characters = &mut self.world.characters;
        let file = loop {
            if characters.next_file >= FILES {
                characters.next_file = 0;
                return Err(Trap::Unsupported(
                    "Make New Character found every character file number taken; the clone \
                     fails (0x4262c0) and the original reads an undefined pointer (0x44aec1)"
                        .into(),
                ));
            }
            let file = characters.next_file;
            characters.next_file += 1;
            if !files.contains(&file) && !characters.made.values().any(|made| made.file == file) {
                break file;
            }
        };
        if characters.next_file >= FILES {
            characters.next_file = 0;
        }
        let draw = u32::from(self.world.crt_rng.next_u15());
        let guid = ((self.world.ticks & 0xffff) << 16) ^ draw;
        if self.catalog.object(guid).is_some() || self.world.characters.made.contains_key(&guid) {
            return Err(Trap::Unsupported(format!(
                "Make New Character's new GUID 0x{guid:08X} is another type's; the original \
                 registers it over that type (0x43bda9), which is not ported"
            )));
        }
        self.world.characters.made.insert(
            guid,
            Character {
                template,
                file,
                strings: Default::default(),
            },
        );
        let record = self
            .world
            .neighbors
            .make_record(guid)
            .map_err(|error| Trap::Invalid(error.to_string()))?;
        Ok((guid, record))
    }

    /// The number of bodies and heads a person of skin tone `tone` can
    /// wear (0x40d077, 0x40d3db): the length of the lists 0x408bf5 builds
    /// ([`sims_content::Wardrobe::count`]). The original keeps each list
    /// by skin tone, sex and age for the rest of the game (the cache at
    /// 0x6e4890); the names it is built from do not change while the game
    /// runs, so counting again gives the same length.
    fn suit_counts(&self, tone: &str, female: bool, child: bool) -> Result<(u32, u32), Trap> {
        let wardrobe = self.program.wardrobe.as_ref().ok_or_else(|| {
            Trap::Unsupported(
                "Make New Character needs the install's suit and texture names, which were \
                 not read"
                    .into(),
            )
        })?;
        if let Some((file, error)) = wardrobe.errors.first_key_value() {
            return Err(Trap::Unsupported(format!(
                "Make New Character counts the suits, and the catalog {file} could not be read \
                 ({error})"
            )));
        }
        let count = |head| {
            u32::try_from(wardrobe.count(head, female, child, tone))
                .map_err(|_| Trap::Invalid("more suits than a count holds".into()))
        };
        Ok((count(false)?, count(true)?))
    }
}
