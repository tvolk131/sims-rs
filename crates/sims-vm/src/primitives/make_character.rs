//! Make New Character (opcode 19, 0x44ae08): a new person or pet type cloned
//! from a template, with a neighbour record for it.
//!
//! The general table (0x45246a) sends opcode 19 to 0x44ae08. Baby's
//! "process" tree reaches it when a baby grows into a child (tree 0x1010),
//! and pet adoption / debug interactions reach the pet modes.
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
//! **Negative age (0x44b23c).** Skips the skin tone lookup (`0x44af49`) and
//! template-byte gender check (`0x44afa6`), then copies age (and skeleton
//! unless the cloned template is already a pet, `0x40cc2a` → `0x40cee4`),
//! skin tone (`0x40d007`, defaulting a missing string to `"???"`), and
//! gender (`0x40ccac` → `0x40ce41`) from the running object's definition,
//! commits with no game RNG draws, sets the stack object to the new record
//! ID, and returns true. If the running object is not a person (`0x457f76`)
//! or has a pet/unknown gender (`0x40d42e`, where `0x40d408` dereferences a
//! null head list), it fails loudly after the clone.
//!
//! **Non-negative age (0x44aef7 to 0x44b21a).** The skin tone's name must
//! not be empty (native error 0x3b, 0x44af58, after the clone, whose type
//! and record stay). Then the Character helper writes body strings 13 (age,
//! `"9"` or `"27"`, always `"27"` for pets at `0x44af89`), 0 (skeleton:
//! `"child"`, `"adult"`, `"dog"`, or `"kat"`), 14 (skin tone), and 12
//! (gender: `"male"`, `"female"`, `"dogmale"`, `"dogfemale"`, `"catmale"`,
//! or `"catfemale"`), and counts the bodies and (for humans) heads the type
//! can wear ([`sims_content::Wardrobe::count`],
//! [`sims_content::Wardrobe::count_pet`]).
//!
//! - **Mode 0, a person (0x44b13f).** With both counts above 0, one game
//!   draw picks the head and a second the body (0x46f457); the type is
//!   committed, named, and the stack object takes the new record's ID.
//! - **Modes 1 and 2, a dog or cat (0x44b051).** With body count above 0,
//!   reads registry word 0x100 (`world.dialogs.declined`): if `-1`, one game
//!   draw picks the body (`0x46f457`); otherwise uses the dialog's answer
//!   without drawing, then resets word 0x100 to `-1` (`0x44b09b`). Sets the
//!   stack object to the new record's ID and returns true.
//! - **Zero count cleanup (0x44b203 → 0x426fcd → 0x4259a2).** If a human's
//!   body or head count is 0, or a pet's body count is 0, discards the edits,
//!   deletes the made type and its record (appending/clearing its inventory
//!   kinds 0..=9 and vacating its slot in place), leaves the stack object
//!   and `world.dialogs.declined` unchanged, and returns false.
//!
//! **Presentation, not kept:** the copied file, the strings written to
//! disk, the derived strings (0x409315, 0x40a588), the name (0x6e4ac0,
//! STR# 0x8e string 5, "No childname"), the HUD thumbnail refresh for pets
//! (0x4e9753, 0x4e8c5d), and which head and body the draws pick (body
//! strings 2 and 1), which only the renderer reads.
//!
//! **Not ported, and failing loudly:** a template byte past 2 with
//! non-negative age (0x44afa6, which reads an undefined gender code), a
//! clone the original fails (0x426335, after which it reads an undefined
//! pointer: no template, or no record of it), a template whose GUID more
//! than one definition has, and negative age when the running object would
//! dereference a null table (0x457f76, 0x40d42e).

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

/// Whether a body-string gender marks a pet (`0x40cee4`).
fn is_pet_gender(gender: Option<&str>) -> bool {
    gender.is_some_and(|g| {
        ["dogmale", "dogfemale", "catmale", "catfemale"]
            .iter()
            .any(|name| name.eq_ignore_ascii_case(g))
    })
}

/// Truncates a skin tone string to the native 3-byte fixed buffer (`0x40bf8b`,
/// capacity 4 including NUL, single-byte Windows-1252 code page).
fn truncate_skin_tone(text: &str) -> String {
    text.chars().take(3).collect()
}

/// C-locale `atoi` <= 17 check on the native 15-byte fixed buffer (`0x40cbdd`,
/// capacity 16 including NUL, → `0x636029`).
fn is_child_age(text: &str) -> bool {
    let bounded: String = text.chars().take(15).collect();
    let text = bounded.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut value = 0i32;
    for digit in digits.bytes().take_while(u8::is_ascii_digit) {
        value = value.wrapping_mul(10).wrapping_add(i32::from(digit - b'0'));
    }
    let signed = if negative {
        value.wrapping_neg()
    } else {
        value
    };
    signed <= 17
}

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
            return self.copy_running_character(guid, record);
        }
        // The skin tone's name: string local + 1 of STR# 0x2000 in the
        // type's semi-global file, counting from 1 (0x47028b, 0x4704fa),
        // truncated into the native 3-byte fixed buffer (0x40bf8b) before the
        // emptiness check (0x44af49).
        let tone = crate::people::profile(&self.program.people, self.world, guid)?
            .tones
            .get(usize::try_from(skin).unwrap_or(usize::MAX))
            .map(|tone| truncate_skin_tone(tone))
            .filter(|tone| !tone.is_empty())
            .ok_or(Trap::native(
                0x3b,
                "Make New Character skin tone has no name",
            ))?;
        let female = sex != 0;
        let gender_code = match species {
            0 => usize::from(female),
            1 => 2 + usize::from(female),
            2 => 4 + usize::from(female),
            _ => {
                return Err(Trap::Unsupported(format!(
                    "Make New Character with template byte {species} passes an undefined \
                     gender code to the Character helper (0x44afa6)"
                )));
            }
        };
        // 0x44af66..0x44af89: pets always pass child = 0 to 0x40cc2a.
        let child = species == 0 && age < ADULT_AGE;
        // 0x44afc0 -> 0x409315: initialize derived clothing rows (30..=34) on
        // the newly cloned character before editing its age, skin, or gender.
        let tpl_profile = crate::people::profile(&self.program.people, self.world, guid)
            .ok()
            .map(std::borrow::Cow::into_owned);
        crate::outfits::init_clothing_rows(self.program, self.world, guid, tpl_profile.as_ref());
        let dst_is_pet = is_pet_gender(tpl_profile.as_ref().and_then(|p| p.gender.as_deref()));
        let made = self
            .world
            .characters
            .made
            .get_mut(&guid)
            .ok_or(Trap::Invalid("a made person type went missing".into()))?;
        // 0x40cc2a: writes age, and writes skeleton only when dst is not
        // already a pet (0x40cee4). Then 0x40d02f writes skin tone, and
        // 0x40ce41 writes gender (and overwrites skeleton for codes 2..=5).
        made.strings
            .insert(characters::AGE, if child { "9" } else { "27" }.to_owned());
        if !dst_is_pet {
            made.strings.insert(
                characters::SKELETON,
                if child { "child" } else { "adult" }.to_owned(),
            );
        }
        made.strings.insert(characters::SKIN, tone.clone());
        let gender_name = [
            "male",
            "female",
            "dogmale",
            "dogfemale",
            "catmale",
            "catfemale",
        ][gender_code];
        made.strings
            .insert(characters::GENDER, gender_name.to_owned());
        match gender_code {
            2 | 3 => {
                made.strings.insert(characters::SKELETON, "dog".to_owned());
            }
            4 | 5 => {
                made.strings.insert(characters::SKELETON, "kat".to_owned());
            }
            _ => {}
        }

        if species == 0 {
            let (bodies, heads) = self.suit_counts(&tone, female, child)?;
            if bodies == 0 || heads == 0 {
                self.cleanup_failed_character(guid, record)?;
                return Ok(false);
            }
            // The head, then the body (0x44b151, 0x44b167). Record the exact
            // chosen suits/rows in World::made_outfits (out of snapshot format
            // 31) so portrait rendering (0x4e7824) and later negative-age
            // copies use the specific wardrobe entries native code writes at
            // 0x40d4e7 and 0x40d183.
            let head_idx = (self.draw() % heads) as usize;
            let body_idx = (self.draw() % bodies) as usize;
            let picked_head = self.pick_human_suit(true, female, child, &tone, head_idx);
            let picked_body = self.pick_human_suit(false, female, child, &tone, body_idx);
            let head_status = match (
                picked_head
                    .as_ref()
                    .and_then(|(s, _)| self.classify_written_wardrobe_suit(s)),
                picked_body
                    .as_ref()
                    .and_then(|(s, _)| self.classify_written_wardrobe_suit(s)),
            ) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            };
            let mut outfit_rows = std::collections::BTreeMap::new();
            if let Some((_, row)) = picked_body {
                outfit_rows.insert(1, row);
            }
            if let Some((_, row)) = picked_head {
                outfit_rows.insert(2, row);
            }
            if child {
                // 0x40a667: child commits write "ADDED" into rows 30 and 34.
                outfit_rows.insert(30, "ADDED".to_owned());
                outfit_rows.insert(34, "ADDED".to_owned());
            }
            self.world
                .made_outfits
                .entry(guid)
                .or_default()
                .extend(outfit_rows);
            // 0x44b185 -> 0x409492(1) -> 0x40a588 -> 0x40af5b: revalidate or
            // regenerate rows 30..=34 and rebuild rows 15, 16, 27, and hands
            // for the clone's new gender, age, skin tone, and body.
            let final_profile = crate::people::profile(&self.program.people, self.world, guid)
                .ok()
                .map(std::borrow::Cow::into_owned);
            crate::outfits::commit_character_clothing(
                self.program,
                self.world,
                guid,
                final_profile.as_ref(),
                crate::outfits::BodyCommitState::Matched,
                None,
            );
            self.world.made_portrait_head.insert(guid, head_status);
            self.frame_mut()?.stack_object = record;
            Ok(true)
        } else {
            let bodies = self.pet_body_count(species == 2)?;
            if bodies == 0 {
                self.cleanup_failed_character(guid, record)?;
                return Ok(false);
            }
            // 0x44b064..0x44b09b: if registry word 0x100 is -1, draw one game
            // RNG for the pet body; otherwise use the dialog's answer without
            // drawing, then reset word 0x100 to -1.
            let pet_idx = self.world.dialogs.declined;
            let picked_idx = if pet_idx == -1 {
                Some((self.draw() % bodies) as usize)
            } else if pet_idx >= 0 && (pet_idx as u32) < bodies {
                Some(pet_idx as usize)
            } else {
                None
            };
            self.world.dialogs.declined = -1;
            let mut outfit_rows = std::collections::BTreeMap::new();
            // 0x40a5d5..0x40a614: pet commits write "NONE" to 1-based rows
            // 3 (head, 0-based 2) and 0x1f..=0x23 (0-based 30..=34).
            for row in [2usize, 30, 31, 32, 33, 34] {
                outfit_rows.insert(row, "NONE".to_owned());
            }
            let head_status = if let Some(idx) = picked_idx {
                let picked = self.pick_pet_suit(species == 2, idx);
                let status = picked
                    .as_ref()
                    .and_then(|(s, _)| self.classify_written_wardrobe_suit(s));
                if let Some((_, row)) = picked {
                    outfit_rows.insert(1, row);
                }
                status
            } else {
                // 0x40d14d / 0x40d162 leaves body row 1 at the template's
                // value, then 0x44b0a3 -> 0x40a588 -> 0x40a5a1 rechecks it
                // against the pet wardrobe list (`0x40d0a4`). On a miss (-1),
                // the empty pet head list sends execution to `0x40ac57..0x40acac`,
                // clearing 0-based rows `1, 2, 15, 16, 17, 18` to `""` (leaving
                // rows `30..=34` as `"NONE"`).
                let tpl_body = crate::people::profile(&self.program.people, self.world, guid)
                    .ok()
                    .and_then(|p| p.body.clone());
                let matched = tpl_body
                    .as_deref()
                    .and_then(|raw| self.find_pet_suit(species == 2, raw));
                if let Some((suit, _)) = matched {
                    if let Some(row) = tpl_body {
                        outfit_rows.insert(1, row);
                    }
                    self.classify_written_wardrobe_suit(&suit)
                } else {
                    for row in [1usize, 2, 15, 16, 17, 18] {
                        outfit_rows.insert(row, String::new());
                    }
                    Some(false)
                }
            };
            self.world
                .made_outfits
                .entry(guid)
                .or_default()
                .extend(outfit_rows);
            self.world.made_portrait_head.insert(guid, head_status);
            self.frame_mut()?.stack_object = record;
            // 0x44b111..0x44b12b: pet creation immediately calls
            // 0x4e8c5d(index, 0x1f, 1), rendering and caching the portrait
            // and persisting BMP_ 2002 before opcode 19 returns (ignoring the
            // renderer's bool return at 0x44b12b).
            if self.in_portrait_table(guid)? {
                let _ = self.try_render_pet_portrait(guid)?;
            }
            Ok(true)
        }
    }

    /// Negative-age mode (`0x44b23c`): copies age, skin tone, and gender from
    /// the running object's definition into the cloned character, with no game
    /// RNG draws.
    fn copy_running_character(&mut self, guid: u32, record: i16) -> Result<bool, Trap> {
        let self_guid = self.me()?.guid;
        let self_def = self.world.characters.definition(self_guid);
        if !self
            .catalog
            .object(self_def)
            .is_some_and(|object| object.kind == 2)
        {
            return Err(Trap::Unsupported(format!(
                "Make New Character with negative age: running object {} (GUID 0x{self_guid:08X}) \
                 is not a person definition; 0x457f76 returns null and 0x40934b dereferences it",
                self.me
            )));
        }
        let src = crate::people::profile(&self.program.people, self.world, self_guid)?.into_owned();
        // 0x44b26e -> 0x409315 on source, then 0x44b27b -> 0x409315 on clone,
        // before editing the clone's age, skin, or gender.
        crate::outfits::init_clothing_rows(self.program, self.world, self_guid, Some(&src));
        let dst_profile = crate::people::profile(&self.program.people, self.world, guid)
            .ok()
            .map(std::borrow::Cow::into_owned);
        crate::outfits::init_clothing_rows(self.program, self.world, guid, dst_profile.as_ref());
        // 0x40ccac maps "male" -> 0, "female" -> 1, and every pet or unknown
        // gender string to 2..=5. At 0x44b2ef, 0x40d408 -> 0x40d3db ->
        // 0x408bf5(1) returns null for codes 2..=5 and 0x40d42e dereferences it.
        let src_gender = match src.gender.as_deref() {
            Some(g) if g.eq_ignore_ascii_case("male") => "male",
            Some(g) if g.eq_ignore_ascii_case("female") => "female",
            other => {
                return Err(Trap::Unsupported(format!(
                    "Make New Character with negative age: running object {} has gender \
                     {other:?}; 0x40d408 dereferences a null head list for pet/unknown genders \
                     (0x40d42e)",
                    self.me
                )));
            }
        };
        let src_child = src.age.as_deref().map(is_child_age).unwrap_or(true);
        // 0x40d007 returns "???" when STR# 200 string 14 is absent (null),
        // and copies into the native 3-byte fixed buffer (0x40bf8b).
        let src_skin = truncate_skin_tone(src.skin.as_deref().unwrap_or("???"));
        let dst_is_pet = is_pet_gender(dst_profile.as_ref().and_then(|p| p.gender.as_deref()));
        let made = self
            .world
            .characters
            .made
            .get_mut(&guid)
            .ok_or(Trap::Invalid("a made person type went missing".into()))?;
        made.strings.insert(
            characters::AGE,
            if src_child { "9" } else { "27" }.to_owned(),
        );
        if !dst_is_pet {
            made.strings.insert(
                characters::SKELETON,
                if src_child { "child" } else { "adult" }.to_owned(),
            );
        }
        made.strings.insert(characters::SKIN, src_skin.clone());
        made.strings
            .insert(characters::GENDER, src_gender.to_owned());
        // 0x44b2d7 -> 0x40d0a4 -> 0x40d10b and 0x44b2ef -> 0x40d408 ->
        // 0x40d46f look up the running person's body and head rows in the
        // human wardrobe list via 0x472f92, writing "" on a miss (-1).
        let src_female = src_gender == "female";
        let has_manual_made_suit = self.world.characters.made.get(&self_guid).is_some_and(|m| {
            m.strings.contains_key(&characters::BODY) || m.strings.contains_key(&characters::HEAD)
        });
        let loaded_body_rows =
            crate::interpreter::object_definition(self.catalog, self.world, self_guid).and_then(
                |def| {
                    let library = self.program.libraries.get(&def.resource)?;
                    let table = library.definitions.get(&def.chunk)?[19] as u16;
                    library.loaded_strings.get(&table)
                },
            );
        let src_body_override = self
            .world
            .characters
            .string(self_guid, characters::BODY)
            .map(str::to_owned)
            .or_else(|| {
                self.world
                    .made_outfits
                    .get(&self_guid)
                    .and_then(|o| o.get(&1).cloned())
            });
        let src_head_override = self
            .world
            .characters
            .string(self_guid, characters::HEAD)
            .map(str::to_owned)
            .or_else(|| {
                self.world
                    .made_outfits
                    .get(&self_guid)
                    .and_then(|o| o.get(&2).cloned())
            });
        let src_body = src_body_override
            .as_deref()
            .or_else(|| match loaded_body_rows {
                Some(loaded) => loaded
                    .as_deref()
                    .and_then(|rows| rows.get(characters::BODY))
                    .map(String::as_str),
                None => src.body.as_deref(),
            });
        let src_head = src_head_override
            .as_deref()
            .or_else(|| match loaded_body_rows {
                Some(loaded) => loaded
                    .as_deref()
                    .and_then(|rows| rows.get(characters::HEAD))
                    .map(String::as_str),
                None => src.head.as_deref(),
            });
        // 0x44b2d7 -> 0x40d0a4 -> 0x40d10b, 0x44b2ef -> 0x40d408 -> 0x40d46f,
        // and 0x44b308 -> 0x40a588 (0x40ab7f..0x40acbc):
        // Look up the running person's body and head rows in the human wardrobe
        // list via 0x472f92 (capped at 255 bytes per 0x587511..0x58752c). If
        // either lookup misses (-1), 0x40ab7f checks both counts: when both
        // body_count > 0 and head_count > 0, each missed slot falls back to
        // index 0 of its list; otherwise 0x40ac57 clears BOTH body and head to "".
        let build_copy_rows = |body: Option<String>,
                               head: Option<String>,
                               cleared_empty_list: bool|
         -> std::collections::BTreeMap<usize, String> {
            let mut rows = std::collections::BTreeMap::new();
            rows.insert(1, body.unwrap_or_default());
            rows.insert(2, head.unwrap_or_default());
            if src_child {
                // 0x40a667: child commits write "ADDED" into rows 30 and 34.
                rows.insert(30, "ADDED".to_owned());
                rows.insert(34, "ADDED".to_owned());
            }
            if cleared_empty_list {
                // 0x40ac57..0x40acb0: empty-list fallback clears 1-based rows
                // 2, 3, 0x10..=0x13 (0-based 1, 2, 15..=18) to "".
                for idx in [15usize, 16, 17, 18] {
                    rows.insert(idx, String::new());
                }
            }
            rows
        };
        let live_ordered = self.live_ordered_wardrobe()?;
        let (copied_outfits, head_status, body_commit_state, pre_clear_body_row) =
            match &live_ordered {
                Some(ordered) => {
                    let mut body_hit = src_body.and_then(|raw| {
                        ordered.find_outfit(false, src_female, src_child, &src_skin, raw)
                    });
                    let mut head_hit = src_head.and_then(|raw| {
                        ordered.find_outfit(true, src_female, src_child, &src_skin, raw)
                    });
                    let initial_body_row = body_hit.as_ref().map(|(_, row)| row.clone());
                    let mut cleared_empty_list = false;
                    let mut body_state = crate::outfits::BodyCommitState::Matched;
                    if body_hit.is_none() || head_hit.is_none() {
                        let body_count = ordered.count(false, src_female, src_child, &src_skin);
                        let head_count = ordered.count(true, src_female, src_child, &src_skin);
                        if body_count > 0 && head_count > 0 {
                            if body_hit.is_none() {
                                body_hit = ordered.pick(false, src_female, src_child, &src_skin, 0);
                                body_state = crate::outfits::BodyCommitState::ReplacedWithFallback;
                            }
                            if head_hit.is_none() {
                                head_hit = ordered.pick(true, src_female, src_child, &src_skin, 0);
                            }
                        } else {
                            if body_hit.is_none() {
                                body_state = crate::outfits::BodyCommitState::ClearedEmptyList;
                            }
                            body_hit = None;
                            head_hit = None;
                            cleared_empty_list = true;
                        }
                    }
                    let body_status = match &body_hit {
                        Some((suit, _)) => self.classify_written_wardrobe_suit(suit),
                        None => Some(false),
                    };
                    let head_only_status = match &head_hit {
                        Some((suit, _)) => self.classify_written_wardrobe_suit(suit),
                        None => Some(false),
                    };
                    let combined = match (body_status, head_only_status) {
                        (Some(true), _) | (_, Some(true)) => Some(true),
                        (Some(false), Some(false)) => Some(false),
                        _ => None,
                    };
                    let rows = build_copy_rows(
                        body_hit.map(|(_, row)| row),
                        head_hit.map(|(_, row)| row),
                        cleared_empty_list,
                    );
                    (Some(rows), combined, body_state, initial_body_row)
                }
                None => {
                    let has_made_outfit = self
                        .world
                        .made_outfits
                        .get(&self_guid)
                        .is_some_and(|m| m.contains_key(&1) || m.contains_key(&2));
                    match self.program.wardrobe.as_ref() {
                        None => {
                            let status = if !has_manual_made_suit
                                && !has_made_outfit
                                && let Some(&status) = self.world.made_portrait_head.get(&self_guid)
                            {
                                status
                            } else if src_body.is_none()
                                && src_head.is_none()
                                && loaded_body_rows.is_none()
                                && self.program.people.portrait_head.get(&self_guid) == Some(&true)
                            {
                                None
                            } else {
                                Some(false)
                            };
                            (None, status, crate::outfits::BodyCommitState::Matched, None)
                        }
                        Some(wardrobe) if wardrobe.errors.is_empty() => {
                            // Match `0x40d0a4 -> 0x408bf5 -> 0x409663 -> 0x40922f`:
                            // freeze the shared `0x6e48a0` suit scan cache.
                            let _ = crate::outfits::suit_scan_order(self.program, self.world);
                            let format_matched_row = |raw: &str, matched_suit: &str| -> String {
                                let row_255: String = raw.chars().take(255).collect();
                                let before_semi = row_255.split(';').next().unwrap_or(&row_255);
                                let raw_tex = before_semi.split_once(',').map_or("", |(_, t)| t);
                                let suit_63: String = matched_suit.chars().take(63).collect();
                                let tex_63: String = raw_tex.chars().take(63).collect();
                                format!("{suit_63},{tex_63}")
                            };
                            let mut body_suits: Option<(Vec<String>, String)> =
                                src_body.and_then(|raw| {
                                    let matched = wardrobe.matching_outfits(
                                        false, src_female, src_child, &src_skin, raw,
                                    );
                                    (!matched.is_empty()).then(|| {
                                        let row = format_matched_row(raw, matched[0]);
                                        (matched.into_iter().map(str::to_owned).collect(), row)
                                    })
                                });
                            let mut head_suits: Option<(Vec<String>, String)> =
                                src_head.and_then(|raw| {
                                    let matched = wardrobe.matching_outfits(
                                        true, src_female, src_child, &src_skin, raw,
                                    );
                                    (!matched.is_empty()).then(|| {
                                        let row = format_matched_row(raw, matched[0]);
                                        (matched.into_iter().map(str::to_owned).collect(), row)
                                    })
                                });
                            let initial_body_row = body_suits.as_ref().map(|(_, row)| row.clone());
                            let mut cleared_empty_list = false;
                            let mut body_state = crate::outfits::BodyCommitState::Matched;
                            if body_suits.is_none() || head_suits.is_none() {
                                let body_count =
                                    wardrobe.count(false, src_female, src_child, &src_skin);
                                let head_count =
                                    wardrobe.count(true, src_female, src_child, &src_skin);
                                if body_count > 0 && head_count > 0 {
                                    if body_suits.is_none() {
                                        body_suits = wardrobe
                                            .pick(false, src_female, src_child, &src_skin, 0)
                                            .map(|(s, r)| (vec![s], r));
                                        body_state =
                                            crate::outfits::BodyCommitState::ReplacedWithFallback;
                                    }
                                    if head_suits.is_none() {
                                        head_suits = wardrobe
                                            .pick(true, src_female, src_child, &src_skin, 0)
                                            .map(|(s, r)| (vec![s], r));
                                    }
                                } else {
                                    if body_suits.is_none() {
                                        body_state =
                                            crate::outfits::BodyCommitState::ClearedEmptyList;
                                    }
                                    body_suits = None;
                                    head_suits = None;
                                    cleared_empty_list = true;
                                }
                            }
                            let classify_list =
                                |hit: &Option<(Vec<String>, String)>| -> Option<bool> {
                                    let Some((suits, _)) = hit else {
                                        return Some(false);
                                    };
                                    if suits.iter().all(|s| {
                                        self.classify_written_wardrobe_suit(s) == Some(true)
                                    }) {
                                        Some(true)
                                    } else if suits.iter().all(|s| {
                                        self.classify_written_wardrobe_suit(s) == Some(false)
                                    }) {
                                        Some(false)
                                    } else {
                                        None
                                    }
                                };
                            let combined =
                                match (classify_list(&body_suits), classify_list(&head_suits)) {
                                    (Some(true), _) | (_, Some(true)) => Some(true),
                                    (Some(false), Some(false)) => Some(false),
                                    _ => None,
                                };
                            let rows = build_copy_rows(
                                body_suits.map(|(_, row)| row),
                                head_suits.map(|(_, row)| row),
                                cleared_empty_list,
                            );
                            (Some(rows), combined, body_state, initial_body_row)
                        }
                        Some(wardrobe) => {
                            let _ = crate::outfits::suit_scan_order(self.program, self.world);
                            let body_miss = self.is_definite_outfit_miss(
                                wardrobe, src_body, false, src_female, src_child, &src_skin,
                            );
                            let head_miss = self.is_definite_outfit_miss(
                                wardrobe, src_head, true, src_female, src_child, &src_skin,
                            );
                            let empty_list = !wardrobe
                                .errors
                                .contains_key("the install's person files")
                                && (!wardrobe
                                    .has_any_texture(false, src_female, src_child, &src_skin)
                                    || !wardrobe
                                        .has_any_texture(true, src_female, src_child, &src_skin));
                            if (body_miss || head_miss) && empty_list {
                                let state = if body_miss {
                                    crate::outfits::BodyCommitState::ClearedEmptyList
                                } else {
                                    crate::outfits::BodyCommitState::Matched
                                };
                                (
                                    Some(build_copy_rows(None, None, true)),
                                    Some(false),
                                    state,
                                    src_body.map(str::to_owned),
                                )
                            } else {
                                (None, None, crate::outfits::BodyCommitState::Matched, None)
                            }
                        }
                    }
                }
            };
        if let Some(rows) = copied_outfits {
            self.world
                .made_outfits
                .entry(guid)
                .or_default()
                .extend(rows);
        }
        // 0x44b308 -> 0x409492(1) -> 0x40a588 -> 0x40af5b: revalidate or
        // regenerate rows 30..=34 and rebuild rows 15, 16, 27, and hands for
        // the copied clone's final profile.
        let final_profile = crate::people::profile(&self.program.people, self.world, guid)
            .ok()
            .map(std::borrow::Cow::into_owned);
        crate::outfits::commit_character_clothing(
            self.program,
            self.world,
            guid,
            final_profile.as_ref(),
            body_commit_state,
            pre_clear_body_row.as_deref(),
        );
        self.world.made_portrait_head.insert(guid, head_status);
        self.frame_mut()?.stack_object = record;
        Ok(true)
    }

    /// Cleans up a newly cloned character whose wardrobe count was 0
    /// (`0x44b203` → `0x409492(0)` → `0x426fcd` → `0x4259a2`): discards
    /// uncommitted body-string edits, unregisters the made type, and deletes
    /// its neighborhood record.
    fn cleanup_failed_character(&mut self, guid: u32, record: i16) -> Result<(), Trap> {
        // 0x44b20a: 0x409492(0) discards uncommitted body-string edits before
        // calling 0x426fcd, so even if a family protects the GUID, the clone
        // keeps its template strings.
        if let Some(made) = self.world.characters.made.get_mut(&guid) {
            made.strings.clear();
        }
        if self
            .world
            .objects
            .values()
            .any(|object| object.guid == guid)
        {
            return Err(Trap::Unsupported(format!(
                "Make New Character cleanup (0x426fcd): deleting live lot objects of newly \
                 cloned GUID 0x{guid:08X} is not ported"
            )));
        }
        // 0x42702c..0x42704f: if any family still lists the GUID, 0x426fcd
        // returns -1 without unregistering or deleting the record.
        if self
            .world
            .families
            .iter()
            .any(|family| family.members.contains(&guid))
        {
            return Ok(());
        }
        if let Some(removed) = self.world.characters.made.remove(&guid) {
            self.world
                .generated_portrait_2002
                .remove(&sims_sim::PortraitFile::Made(removed.file));
        }
        self.world.made_portrait_head.remove(&guid);
        self.world.made_outfits.remove(&guid);
        self.world.ambiguous_portraits.remove(&guid);
        self.world.neighbors.remove_record(record);
        Ok(())
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
        let inherited_portrait_2002 = self.has_portrait_2002(template);
        self.world.characters.made.insert(
            guid,
            Character {
                template,
                file,
                strings: Default::default(),
            },
        );
        // 0x42629e copies the template's current IFF resources into
        // `Characters/User%05d.iff`, including any BMP_ 2002 written to the
        // template earlier in this session.
        if inherited_portrait_2002 {
            self.world
                .generated_portrait_2002
                .insert(sims_sim::PortraitFile::Made(file));
        }
        let record = self
            .world
            .neighbors
            .make_record(guid)
            .map_err(|error| Trap::Invalid(error.to_string()))?;
        Ok((guid, record))
    }

    /// Returns the native-ordered wardrobe sharing the lazy `0x409255`
    /// (`0x6e48a0`) suit scan cache (`World::suit_scan_cache`) with generated
    /// outfit dressing (`0x40e622`). Any `.bcf` sibling appearances registered
    /// before the first non-empty scan are included in both paths.
    fn live_ordered_wardrobe(&mut self) -> Result<Option<sims_content::ImportWardrobe>, Trap> {
        let ordered = match &self.program.ordered_wardrobe {
            Some(Ok(ordered)) => ordered.clone(),
            Some(Err(reason)) => {
                return Err(Trap::Unsupported(format!(
                    "Make New Character needs the install's native-ordered wardrobe ({reason})"
                )));
            }
            None => return Ok(None),
        };
        let scan = crate::outfits::suit_scan_order(self.program, self.world);
        let bytes = scan
            .iter()
            .map(|s| crate::outfits::encode_cp1252(s))
            .collect();
        Ok(Some(ordered.with_suits(bytes)))
    }

    /// The number of bodies and heads a person of skin tone `tone` can
    /// wear (0x40d077, 0x40d3db): the length of the lists 0x408bf5 builds
    /// ([`sims_content::Wardrobe::count`]), sharing `0x409255` (`0x6e48a0`)
    /// with generated dressing.
    fn suit_counts(&mut self, tone: &str, female: bool, child: bool) -> Result<(u32, u32), Trap> {
        if let Some(ordered) = self.live_ordered_wardrobe()? {
            let count = |head| {
                u32::try_from(ordered.count(head, female, child, tone))
                    .map_err(|_| Trap::Invalid("more suits than a count holds".into()))
            };
            return Ok((count(false)?, count(true)?));
        }
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
        // 0x40d077 -> 0x408bf5 -> 0x409663 -> 0x40922f freezes `0x6e48a0` on
        // the first non-empty scan even when using the fallback `Wardrobe`.
        let _ = crate::outfits::suit_scan_order(self.program, self.world);
        let count = |head| {
            u32::try_from(wardrobe.count(head, female, child, tone))
                .map_err(|_| Trap::Invalid("more suits than a count holds".into()))
        };
        Ok((count(false)?, count(true)?))
    }

    /// The number of bodies a dog (`cat` false) or cat can wear (0x40d077 →
    /// 0x408bf5 → 0x409663, [`sims_content::Wardrobe::count_pet`]).
    fn pet_body_count(&mut self, cat: bool) -> Result<u32, Trap> {
        if let Some(ordered) = self.live_ordered_wardrobe()? {
            return u32::try_from(ordered.count_pet(cat))
                .map_err(|_| Trap::Invalid("more suits than a count holds".into()));
        }
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
        let _ = crate::outfits::suit_scan_order(self.program, self.world);
        u32::try_from(wardrobe.count_pet(cat))
            .map_err(|_| Trap::Invalid("more suits than a count holds".into()))
    }

    /// Picks the `index`-th human wardrobe entry (`0x44b151`, `0x44b167`),
    /// using exact native `0x409255`/`0x40895a` order when `ordered_wardrobe`
    /// is loaded, or `wardrobe`'s order in synthetic fixtures.
    fn pick_human_suit(
        &mut self,
        head: bool,
        female: bool,
        child: bool,
        tone: &str,
        index: usize,
    ) -> Option<(String, String)> {
        if let Ok(Some(ordered)) = self.live_ordered_wardrobe() {
            ordered.pick(head, female, child, tone, index)
        } else {
            self.program
                .wardrobe
                .as_ref()?
                .pick(head, female, child, tone, index)
        }
    }

    /// Looks up `raw_row` in the pet body wardrobe list (`0x40a5a1` →
    /// `0x40d0a4` → `0x472f92`), returning `(suit_name_63, formatted_row)` on hit.
    fn find_pet_suit(&mut self, cat: bool, raw_row: &str) -> Option<(String, String)> {
        if let Ok(Some(ordered)) = self.live_ordered_wardrobe() {
            ordered.find_pet_outfit(cat, raw_row)
        } else {
            self.program
                .wardrobe
                .as_ref()?
                .find_pet_outfit(cat, raw_row)
        }
    }

    /// Picks the `index`-th pet wardrobe entry (`0x44b071`/`0x44b07e`),
    /// using exact native `0x409255`/`0x40895a` order when `ordered_wardrobe`
    /// is loaded, or `wardrobe`'s order in synthetic fixtures.
    fn pick_pet_suit(&mut self, cat: bool, index: usize) -> Option<(String, String)> {
        if let Ok(Some(ordered)) = self.live_ordered_wardrobe() {
            ordered.pick_pet(cat, index)
        } else {
            self.program.wardrobe.as_ref()?.pick_pet(cat, index)
        }
    }

    /// Classifies one wardrobe suit name: `Some(true)` if it binds `"HEAD"`,
    /// `Some(false)` if confirmed not to, or `None` if an earlier suit decode
    /// error left it unknown.
    fn classify_suit_head(&self, suit: &str) -> Option<bool> {
        if self.program.people.head_suits.contains(suit) {
            Some(true)
        } else if !self.program.people.suit_errors.is_empty()
            && !self.program.people.suits.contains(suit)
        {
            None
        } else {
            Some(false)
        }
    }

    /// Classifies a suit name written from a wardrobe entry (`0x4092c5`–
    /// `0x4092e1` copies at most 63 bytes into the entry buffer, `0x472f41`
    /// serializes that 63-byte string, and `0x5a27f9` matches exact length):
    /// `Some(true)` if the written 63-byte name binds `"HEAD"`, `Some(false)`
    /// if confirmed not to, or `None` if an earlier suit decode error left it
    /// unknown.
    fn classify_written_wardrobe_suit(&self, suit: &str) -> Option<bool> {
        let written: String = suit.chars().take(63).collect();
        self.classify_suit_head(&written)
    }

    /// Whether `raw` is guaranteed to miss in `0x40d0a4`/`0x40d408` even if
    /// `.cmx`/`.bcf` catalog decode errors hid some suit names (missing row,
    /// invalid syntax / comma past 255 bytes, or missing texture when texture
    /// enumeration succeeded).
    fn is_definite_outfit_miss(
        &self,
        wardrobe: &sims_content::Wardrobe,
        raw: Option<&str>,
        is_head: bool,
        female: bool,
        child: bool,
        skin: &str,
    ) -> bool {
        let Some(raw) = raw else {
            return true;
        };
        if !sims_content::Wardrobe::can_match_outfit_row(is_head, female, child, skin, raw) {
            return true;
        }
        !wardrobe.errors.contains_key("the install's person files")
            && !wardrobe.has_outfit_texture(is_head, female, child, skin, raw)
    }
}
