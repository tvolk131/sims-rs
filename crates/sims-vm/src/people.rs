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
use crate::object_uses::no_action;
use crate::placement::{capture, stands_on_lot};
use sims_content::{People, Profile, Skeleton};
use sims_sim::World;
use sims_sim::characters;
use std::borrow::Cow;

/// The callback table's entries a person's reset asks.
const RESET: usize = 3;
const CANCELLED: usize = 4;

/// Person data words.
pub(crate) mod field {
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

/// The revision a neighborhood record gets when a person is saved into
/// it (0x423e04 returns 9).
pub const SAVED_REVISION: i32 = 9;

/// Saves person `id` into its neighborhood record (0x425a69): the record
/// is the one person data word 31 names (0x4256f4, slot `id - 1`), not
/// the first of the person's type; it gets revision [`SAVED_REVISION`],
/// and each word the table at 0x423827 lists for that revision
/// ([`persistent_fields`]) is copied from the person into it. The copy
/// does not test the words' revisions, as the attach does (0x4258ff):
/// the table is built for revision 9 and every entry is copied. A word
/// 31 that names no record saves nothing (0x425a84). An object that is
/// not a person fails: the callers cast first.
pub(crate) fn save_to_record(world: &mut World, id: u16) -> Result<(), Trap> {
    let person = world
        .objects
        .get(&id)
        .ok_or(Trap::native(10, "missing object"))?
        .person
        .as_ref()
        .ok_or(Trap::native(0x1c, "object is not a person"))?;
    let fields = persistent_fields(SAVED_REVISION);
    let mut values = Vec::with_capacity(fields.len());
    for &word in &fields {
        values.push(
            *person
                .data
                .get(word)
                .ok_or_else(|| Trap::Invalid(format!("the person has no data word {word}")))?,
        );
    }
    let neighbor = person.data[field::NEIGHBOR];
    let Some(record) = world.neighbors.get_mut(neighbor) else {
        return Ok(());
    };
    record.revision = SAVED_REVISION;
    for (&word, value) in fields.iter().zip(values) {
        *record.data.get_mut(word).ok_or_else(|| {
            Trap::Invalid(format!(
                "neighborhood record {} has no person data word {word}",
                record.id
            ))
        })? = value;
    }
    Ok(())
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
        (characters::BODY, &mut profile.body),
        (characters::HEAD, &mut profile.head),
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

/// A person type's skeleton (read-only lookup): the one its body string 0
/// names. A type made while the game ran names the skeleton Make New
/// Character wrote, or its template's; lazy `.ndx` catalogs already loaded at
/// runtime (`World::loaded_indexed_skeletons`) take precedence over unopened
/// catalog entries. A type with no skeleton in `people` fails.
pub(crate) fn skeleton<'p>(
    people: &'p People,
    world: &World,
    guid: u32,
) -> Result<&'p Skeleton, Trap> {
    let made = world.characters.made.get(&guid);
    let template = made.map_or(guid, |made| made.template);
    let requested_name = made
        .and_then(|made| made.strings.get(&characters::SKELETON))
        .or_else(|| people.skeleton_names.get(&template));
    if let Some(name) = requested_name {
        if people.skeletons_by_name.contains_key(name)
            || world.loaded_indexed_skeletons.contains_key(name)
            || people.indexed_skeletons.contains_key(name)
        {
            return lookup_loaded_skeleton_by_name(people, world, name).ok_or_else(|| {
                Trap::Unsupported(format!(
                    "no person type's skeleton is named {name:?}, which the person type \
                     0x{guid:08X} names"
                ))
            });
        }
        if let Some(skeleton) = people.skeletons.get(&template).filter(|s| s.name == *name) {
            return Ok(skeleton);
        }
        return lookup_loaded_skeleton_by_name(people, world, name).ok_or_else(|| {
            Trap::Unsupported(format!(
                "no person type's skeleton is named {name:?}, which the person type \
                 0x{guid:08X} names"
            ))
        });
    }
    people.skeletons.get(&template).ok_or_else(|| {
        Trap::Unsupported(format!(
            "no skeleton was supplied for the person type 0x{guid:08X}"
        ))
    })
}

/// Read-only lookup of a skeleton by exact name (`0x5a280f`), checking:
/// 1. startup-loaded skeletons (`people.skeletons_by_name`);
/// 2. runtime-loaded `.ndx` skeletons and filled siblings
///    (`world.loaded_indexed_skeletons` → `people.indexed_catalogs`);
/// 3. still-unloaded `.ndx` entry's own catalog (for read-only callers that
///    inspect a person whose skeleton was not resolved via the factory);
/// 4. fallback over `people.skeletons.values()` for synthetic fixtures.
fn lookup_loaded_skeleton_by_name<'p>(
    people: &'p People,
    world: &World,
    name: &str,
) -> Option<&'p Skeleton> {
    if let Some(skeleton) = people.skeletons_by_name.get(name) {
        return Some(skeleton);
    }
    if let Some(slot) = world.loaded_indexed_skeletons.get(name) {
        return slot.as_deref().and_then(|loc| {
            people
                .indexed_catalogs
                .get(loc)
                .and_then(|(items, _)| items.iter().find(|s| s.name == name))
        });
    }
    if let Some(Ok(loc)) = people.indexed_skeletons.get(name)
        && let Some((items, _)) = people.indexed_catalogs.get(loc)
        && let Some(skeleton) = items.iter().find(|s| s.name == name)
    {
        return Some(skeleton);
    }
    people.skeletons.values().find(|s| s.name == name)
}

/// Mutable avatar-registry and CRT-RNG state threaded through skeleton and
/// appearance resolution (`0x5a0384`, `0x5a044d`, `0x59e60e` → `0x59f087`).
pub(crate) struct RegistryMut<'a, 'b> {
    pub loaded_skeletons: &'a mut std::collections::BTreeMap<String, Option<String>>,
    pub loaded_suits: &'a mut std::collections::BTreeMap<String, Option<bool>>,
    pub loaded_suit_bones: &'a mut std::collections::BTreeMap<String, Vec<Vec<String>>>,
    pub runtime_added_suits: &'a mut Vec<String>,
    pub draws: &'a mut crate::objects::Draws<'b>,
    pub files: &'a mut BTreeSet<String>,
}

/// Loads one `.ndx`-referenced `.bcf` catalog (`0x59e60e` → `0x59f087`),
/// building its skeletons once into the C runtime RNG stream (`0x59f107` →
/// `0x598af7`, tracked in `files`) and populating BOTH unloaded skeletons
/// (`0x59f18c`–`0x59f19f`) AND unloaded appearances (`0x59f34c`–`0x59f35f`)
/// without overwriting eager or previously attempted entries.
fn load_indexed_bcf_catalog_into(people: &People, loc: &str, reg: &mut RegistryMut<'_, '_>) {
    if let Some((items, _)) = people.indexed_catalogs.get(loc) {
        if let Some(first) = items.first() {
            // Mark the file read so a subsequent `build_skeleton` for the
            // resolved person draws only its own copy (`0x598da3`).
            reg.files.insert(first.file.clone());
        }
        // 0x59e70a -> 0x59f087 -> 0x59f107 -> 0x598af7: EVERY time an
        // unresolved `.ndx` entry opens this `.bcf` catalog (including a
        // later reopen for another unresolved `.ndx` entry, and including
        // duplicate skeleton entries in the file), every skeleton decoded
        // from the catalog draws its wiggle scales before checking whether
        // its name is already registered.
        for item in items {
            if let Some(scales) = item.scales.get(item.place) {
                reg.draws.skeleton(scales);
            }
        }
        for item in items {
            if !people.skeletons_by_name.contains_key(&item.name)
                && !reg.loaded_skeletons.contains_key(&item.name)
            {
                reg.loaded_skeletons
                    .insert(item.name.clone(), Some(loc.to_owned()));
            }
        }
    }
    if let Some((catalog_heads, _)) = people.indexed_suit_catalogs.get(loc) {
        let catalog_bones = people.indexed_suit_head_bones.get(loc);
        for (app_name, has_head) in catalog_heads {
            if !people.eager_suits.contains(app_name) && !reg.loaded_suits.contains_key(app_name) {
                reg.loaded_suits.insert(app_name.clone(), Some(*has_head));
                if let Some(bones) = catalog_bones.and_then(|m| m.get(app_name)) {
                    reg.loaded_suit_bones
                        .insert(app_name.clone(), bones.clone());
                }
                // 0x59f320 -> 0x5a1fb1: appearances not already present in the
                // startup eager or `.ndx` hash table are inserted as new nodes.
                if !people.indexed_suits.contains_key(app_name)
                    && !reg.runtime_added_suits.contains(app_name)
                {
                    reg.runtime_added_suits.push(app_name.clone());
                }
            }
        }
    }
}

fn resolve_skeleton_by_name_into<'p>(
    people: &'p People,
    guid: u32,
    name: &str,
    reg: &mut RegistryMut<'_, '_>,
) -> Result<&'p Skeleton, Trap> {
    if let Some(skeleton) = people.skeletons_by_name.get(name) {
        return Ok(skeleton);
    }
    if let Some(slot) = reg.loaded_skeletons.get(name) {
        return match slot.as_deref() {
            Some(loc) => people
                .indexed_catalogs
                .get(loc)
                .and_then(|(items, _)| items.iter().find(|s| s.name == name))
                .ok_or_else(|| {
                    Trap::Unsupported(format!(
                        "0x5a03c0: loaded catalog {loc} does not define skeleton {name:?}"
                    ))
                }),
            None => Err(Trap::Unsupported(format!(
                "0x5a389b: earlier lazy lookup for skeleton {name:?} failed"
            ))),
        };
    }
    if let Some(entry) = people.indexed_skeletons.get(name) {
        let loc = match entry {
            Ok(loc) => loc,
            Err(reason) => {
                // 0x5a389b: mark failed so later catalog loads cannot overwrite it.
                reg.loaded_skeletons.insert(name.to_owned(), None);
                return Err(Trap::Unsupported(reason.clone()));
            }
        };
        let (_, decode_error) = people.indexed_catalogs.get(loc).ok_or_else(|| {
            Trap::Unsupported(format!(
                "0x5a3810: indexed catalog {loc} for skeleton {name:?} is unavailable"
            ))
        })?;
        // 0x59e60e -> 0x59f087: loading this `.bcf` builds its skeletons
        // (`0x59f107`), registers skeletons (`0x59f18c`), and registers
        // appearances (`0x59f34c`).
        load_indexed_bcf_catalog_into(people, loc, reg);
        if let Some(Some(filled_loc)) = reg.loaded_skeletons.get(name)
            && let Some(skeleton) = people
                .indexed_catalogs
                .get(filled_loc)
                .and_then(|(catalog, _)| catalog.iter().find(|s| s.name == name))
        {
            return Ok(skeleton);
        }
        // 0x5a389b: mark failed so later catalog loads cannot overwrite it.
        reg.loaded_skeletons.insert(name.to_owned(), None);
        let reason = if let Some(error) = decode_error {
            format!("0x5a3810: the skeletons of {loc} do not decode ({error})")
        } else {
            format!("0x5a03c0: {loc} does not define skeleton {name:?}")
        };
        return Err(Trap::Unsupported(reason));
    }
    people
        .skeletons
        .values()
        .find(|s| s.name == name)
        .ok_or_else(|| {
            Trap::Unsupported(format!(
                "no person type's skeleton is named {name:?}, which the person type \
                 0x{guid:08X} names"
            ))
        })
}

/// Resolves a skeleton by exact byte-for-byte name (`0x5a0384`, `0x5a280f`),
/// executing lazy `.ndx` catalog loads in runtime call order.
pub(crate) fn resolve_skeleton_by_name<'p>(
    people: &'p People,
    world: &mut World,
    guid: u32,
    name: &str,
) -> Result<&'p Skeleton, Trap> {
    let mut draws = crate::objects::Draws {
        stream: &mut world.crt_rng,
        count: 0,
    };
    let mut reg = RegistryMut {
        loaded_skeletons: &mut world.loaded_indexed_skeletons,
        loaded_suits: &mut world.loaded_indexed_suits,
        loaded_suit_bones: &mut world.loaded_indexed_suit_bones,
        runtime_added_suits: &mut world.runtime_added_suits,
        draws: &mut draws,
        files: &mut world.skeleton_files,
    };
    resolve_skeleton_by_name_into(people, guid, name, &mut reg)
}

/// Resolves the skeleton for constructing a saved-house person of `guid`
/// (`0x58838b` → `0x5a073b` → `0x5a0384`), honoring runtime `.ndx` catalog
/// load order and sibling registration on the house load's working state.
pub(crate) fn resolve_skeleton_for_saved<'p>(
    people: &'p People,
    guid: u32,
    reg: &mut RegistryMut<'_, '_>,
) -> Result<&'p Skeleton, Trap> {
    if let Some(name) = people.skeleton_names.get(&guid) {
        if people.skeletons_by_name.contains_key(name)
            || reg.loaded_skeletons.contains_key(name)
            || people.indexed_skeletons.contains_key(name)
        {
            return resolve_skeleton_by_name_into(people, guid, name, reg);
        }
        if let Some(skeleton) = people.skeletons.get(&guid).filter(|s| s.name == *name) {
            return Ok(skeleton);
        }
        return resolve_skeleton_by_name_into(people, guid, name, reg);
    }
    people.skeletons.get(&guid).ok_or_else(|| {
        Trap::Unsupported(format!(
            "no skeleton was supplied for the person type 0x{guid:08X}"
        ))
    })
}

/// Resolves the skeleton for constructing a person of `guid` (`0x58833c` →
/// `0x5a0384`), honoring runtime-made overrides, `early_name` (loader-ordered
/// body string 0), and runtime `.ndx` load order.
pub(crate) fn resolve_skeleton<'p>(
    people: &'p People,
    world: &mut World,
    guid: u32,
    early_name: Option<&str>,
) -> Result<&'p Skeleton, Trap> {
    let made = world.characters.made.get(&guid);
    let template = made.map_or(guid, |made| made.template);
    let requested_name = early_name
        .map(str::to_owned)
        .or_else(|| {
            made.and_then(|made| made.strings.get(&characters::SKELETON))
                .cloned()
        })
        .or_else(|| people.skeleton_names.get(&template).cloned());
    if let Some(name) = requested_name {
        // Check startup-loaded and runtime-ordered `.ndx` registries BEFORE
        // `people.skeletons` so catalog types whose skeletons are `.ndx`-indexed
        // honor exact runtime lookup order (`0x5a0384`, `0x59f18c`–`0x59f19f`).
        if people.skeletons_by_name.contains_key(&name)
            || world.loaded_indexed_skeletons.contains_key(&name)
            || people.indexed_skeletons.contains_key(&name)
        {
            return resolve_skeleton_by_name(people, world, guid, &name);
        }
        if let Some(skeleton) = people.skeletons.get(&template).filter(|s| s.name == name) {
            return Ok(skeleton);
        }
        return resolve_skeleton_by_name(people, world, guid, &name);
    }
    people.skeletons.get(&template).ok_or_else(|| {
        Trap::Unsupported(format!(
            "no skeleton was supplied for the person type 0x{guid:08X}"
        ))
    })
}

/// Result of looking up lowercased appearance `suit` via `0x5a044d` →
/// `0x5a38da` → `0x59e60e` → `0x59f087`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolvedSuit {
    /// Appearance exists; `bool` is whether it binds `"HEAD"` with a valid mesh.
    Found(bool),
    /// Appearance is confirmed absent (or previously attempted and failed cleanly at `0x5a3965`).
    Missing,
    /// An earlier decode error prevented knowing whether `suit` exists / binds `"HEAD"`.
    Unknown,
}

/// Whether a previously failed indexed lookup for `suit` (`loaded_indexed_suits[suit] == None`)
/// failed because decoding stopped with an error (leaving existence / HEAD binding unknown)
/// rather than a clean catalog miss or missing file (`0x5a392f -> 0x5a3965`).
fn failed_indexed_suit_is_unknown(people: &People, suit: &str) -> bool {
    match people.indexed_suits.get(suit) {
        Some(Ok(loc)) => people
            .indexed_suit_catalogs
            .get(loc)
            .is_none_or(|(_, err)| err.is_some()),
        // 0x5a392f -> 0x5a3965: an `.ndx` entry pointing to a missing file is
        // a definite miss even if an unrelated catalog populated `suit_errors`;
        // only a genuine lookup error recorded in `suit_errors` is unknown.
        Some(Err(reason)) => people.suit_errors.values().any(|err| err == reason),
        None => !people.suit_errors.is_empty(),
    }
}

/// Read-only inspection of already-loaded appearance state (`0x58a1f8` skips
/// dressing without triggering new `.ndx` catalog loads).
pub(crate) fn peek_suit_head(people: &People, world: &World, suit: &str) -> ResolvedSuit {
    if people.eager_suits.contains(suit) {
        return ResolvedSuit::Found(people.head_suits.contains(suit));
    }
    if let Some(&slot) = world.loaded_indexed_suits.get(suit) {
        return match slot {
            Some(has_head) => ResolvedSuit::Found(has_head),
            None if failed_indexed_suit_is_unknown(people, suit) => ResolvedSuit::Unknown,
            None => ResolvedSuit::Missing,
        };
    }
    if people.indexed_suits.contains_key(suit) {
        return ResolvedSuit::Missing;
    }
    if people.head_suits.contains(suit) {
        ResolvedSuit::Found(true)
    } else if people.suits.contains(suit) {
        ResolvedSuit::Found(false)
    } else if !people.suit_errors.is_empty() {
        ResolvedSuit::Unknown
    } else {
        ResolvedSuit::Missing
    }
}

/// Resolves appearance `suit` byte-for-byte via `0x5a044d` → `0x5a38da` →
/// `0x59e60e` → `0x59f087` against explicit transactional registry state
/// (used both by live runtime dressing and saved-house object reads).
pub(crate) fn resolve_suit_into(
    people: &People,
    suit: &str,
    reg: &mut RegistryMut<'_, '_>,
) -> ResolvedSuit {
    if people.eager_suits.contains(suit) {
        return ResolvedSuit::Found(people.head_suits.contains(suit));
    }
    if let Some(&slot) = reg.loaded_suits.get(suit) {
        return match slot {
            Some(has_head) => ResolvedSuit::Found(has_head),
            None if failed_indexed_suit_is_unknown(people, suit) => ResolvedSuit::Unknown,
            None => ResolvedSuit::Missing,
        };
    }
    if let Some(entry) = people.indexed_suits.get(suit) {
        match entry {
            Ok(loc) => {
                if let Some((_, decode_error)) = people.indexed_suit_catalogs.get(loc) {
                    // 0x59e60e -> 0x59f087: loading this `.bcf` builds its
                    // skeletons (`0x59f107`), registers skeletons (`0x59f18c`),
                    // and registers appearances (`0x59f34c`).
                    load_indexed_bcf_catalog_into(people, loc, reg);
                    if let Some(&Some(has_head)) = reg.loaded_suits.get(suit) {
                        return ResolvedSuit::Found(has_head);
                    }
                    // 0x5a3965: mark the requested entry attempted (`None`)
                    // BEFORE returning even on a decode error, so later
                    // catalog loads cannot fill it (`0x59f34c`), while
                    // `failed_indexed_suit_is_unknown` preserves decode
                    // uncertainty for subsequent queries.
                    reg.loaded_suits.insert(suit.to_owned(), None);
                    return if decode_error.is_some() {
                        ResolvedSuit::Unknown
                    } else {
                        ResolvedSuit::Missing
                    };
                }
                reg.loaded_suits.insert(suit.to_owned(), None);
                return ResolvedSuit::Unknown;
            }
            Err(reason) => {
                // 0x5a392f -> 0x5a3965: missing catalog file is a definite
                // miss regardless of unrelated `suit_errors`.
                reg.loaded_suits.insert(suit.to_owned(), None);
                return if people.suit_errors.values().any(|err| err == reason) {
                    ResolvedSuit::Unknown
                } else {
                    ResolvedSuit::Missing
                };
            }
        }
    }
    if people.head_suits.contains(suit) {
        ResolvedSuit::Found(true)
    } else if people.suits.contains(suit) {
        ResolvedSuit::Found(false)
    } else if !people.suit_errors.is_empty() {
        ResolvedSuit::Unknown
    } else {
        ResolvedSuit::Missing
    }
}

pub(crate) fn resolve_suit(people: &People, world: &mut World, suit: &str) -> ResolvedSuit {
    let mut draws = crate::objects::Draws {
        stream: &mut world.crt_rng,
        count: 0,
    };
    let mut reg = RegistryMut {
        loaded_skeletons: &mut world.loaded_indexed_skeletons,
        loaded_suits: &mut world.loaded_indexed_suits,
        loaded_suit_bones: &mut world.loaded_indexed_suit_bones,
        runtime_added_suits: &mut world.runtime_added_suits,
        draws: &mut draws,
        files: &mut world.skeleton_files,
    };
    resolve_suit_into(people, suit, &mut reg)
}

/// Whether `suit` has a `"HEAD"` mesh binding whose required bones all exist
/// in `skeleton` (`0x599f8a..0x59a01c`), using the winning runtime-loaded
/// `.bcf` catalog's mesh requirements (`World::loaded_indexed_suit_bones`,
/// `0x59f34c..0x59f35f`) for lazy suits and `People::head_mesh_bones` for
/// eager or synthetic suits.
pub(crate) fn suit_head_matches_skeleton(
    people: &People,
    world: &World,
    suit: &str,
    has_head: bool,
    skeleton: Option<&Skeleton>,
) -> bool {
    let mesh_bones = if people.eager_suits.contains(suit) {
        people.head_mesh_bones.get(suit).map(Vec::as_slice)
    } else if world.loaded_indexed_suits.contains_key(suit) {
        world.loaded_indexed_suit_bones.get(suit).map(Vec::as_slice)
    } else {
        people.head_mesh_bones.get(suit).map(Vec::as_slice)
    };
    people.suit_head_matches_skeleton_with_bones(suit, has_head, mesh_bones, skeleton)
}

/// Reads person `id`'s appearance again if its animator says it is to be
/// read (`stale`, animator byte +0xd8), executing `0x5888d3`:
///
/// 1. `vtable[+0x10]` (`0x5887fc`): detach all body/extra/accessory/head slots;
/// 2. `0x588914`, `0x588921`: zero live hand words 22 and 23;
/// 3. `0x588930`: call `cTSPerson::Dress(0)` (`0x58a178(0)`) BEFORE reloading
///    profile words;
/// 4. `0x588935..0x588ce5`: reload gender (65), age (58), and skin tone (60)
///    from `STR# 200` ([`reload_profile`]);
/// 5. `0x588cea` → `vtable[+0x18]` (`0x588d11`): destroy and clear accessories;
/// 6. `0x588ced`, `0x588cf4`: reset cached suit and variant (`last_dressed[0]`
///    and `last_dressed[1]`) to `-1`.
///
/// A person type with no profile in [`crate::Program::people`] fails before
/// mutating state.
pub(crate) fn read_appearance_if_stale(
    catalog: &sims_content::Catalog,
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
    // Looked up before mutating so a missing object, non-person, or missing
    // profile still fails cleanly.
    let profile = profile(&program.people, world, guid);
    if world
        .objects
        .get(&id)
        .and_then(|object| object.person.as_ref())
        .is_none()
    {
        return Err(Trap::native(0x1c, "object is not a person"));
    }
    let profile = profile?.into_owned();
    *stale = false;

    // 1. 0x5888f1 -> 0x5887fc: detach all bound attachment slots (keeping
    //    `last_dressed` untouched for the immediately following `0x58a178(0)`).
    {
        let outfit = world.person_outfits.entry(id).or_default();
        outfit.head = None;
        outfit.body = None;
        outfit.body_alt = None;
        outfit.extra_left = None;
        outfit.extra_right = None;
        outfit.accessories.clear();
    }
    // 2. 0x588914, 0x588921: clear live hand words 22 & 23 before dressing.
    if let Some(person) = world
        .objects
        .get_mut(&id)
        .and_then(|object| object.person.as_mut())
    {
        if let Some(w) = person.data.get_mut(22) {
            *w = 0;
        }
        if let Some(w) = person.data.get_mut(23) {
            *w = 0;
        }
    }
    world.routing.unknown_words.remove(&(id, 22));
    world.routing.unknown_words.remove(&(id, 23));

    // 3. 0x588930 -> 0x58a178(0): dress BEFORE reloading profile words.
    crate::outfits::dress_person(catalog, program, world, id, false)?;

    // 4. 0x588935..0x588ce5: reload profile words (gender, age, skin, hands).
    let person = world
        .objects
        .get_mut(&id)
        .and_then(|object| object.person.as_mut())
        .ok_or(Trap::native(0x1c, "object is not a person"))?;
    reload_profile(&mut person.data, &profile)?;
    // 5. 0x588cea -> 0x588d11: clear all accessories.
    person.accessories.clear();
    if let Some(outfit) = world.person_outfits.get_mut(&id) {
        outfit.accessories.clear();
        // 6. 0x588ced, 0x588cf4: reset cached suit & variant to -1.
        outfit.last_dressed[0] = -1;
        outfit.last_dressed[1] = -1;
    }
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

/// Decrements data word 62 (`USERS`) on every target object currently listed in
/// `person_id`'s `object_uses` and clears `person_id`'s `object_uses`.
pub(crate) fn clear_person_uses_and_counts(world: &mut World, person_id: u16) {
    let targets: Vec<i16> = world
        .objects
        .get_mut(&person_id)
        .and_then(|o| o.person.as_mut())
        .map(|p| p.object_uses.drain(..).map(|u| u.target).collect())
        .unwrap_or_default();
    for target in targets {
        if let Ok(target_id) = u16::try_from(target)
            && let Some(obj) = world.objects.get_mut(&target_id)
            && let Some(users) = obj.data.get_mut(62)
            && *users > 0
        {
            *users -= 1;
        }
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
    ///    cleared. The running action becomes the empty action (0x45eab3,
    ///    0x45e6cd): one with a target becomes the last action, its flag
    ///    0x20 the object's last result (+0x20). The motive changes an
    ///    action left running go, and person data words 33, 71, 72 and 73
    ///    are cleared.
    /// 6. Each queued action is dropped, first to last, and its target is
    ///    asked function 4 with the person as the stack object (0x45ecc2).
    ///    A reset letting go of an object skips this: the filters for the
    ///    object follow instead ([`Interpreter::reset_person_letting_go`]).
    ///
    /// Between steps 2 and 3 the person's route contexts (person +0xaf0)
    /// are erased (0x45e8aa, 0x45fad7), whose destructors (0x5d5653) only
    /// free memory. Its path (+0xae0) stays. Its headlines are cleared
    /// (0x45e8c3, 0x46e042), which changes nothing kept
    /// (`primitives::headline`).
    ///
    /// A person inside another object (0x45e8dd), or off the lot outside
    /// deletion (0x45e977), fails because its take-off can leave an
    /// animator refresh (0x58da12) pending beyond the scheduler pass.
    /// A person whose data word 73 is set fails because the original then
    /// works out its footprint again.
    ///
    /// So does an animator the reset cannot handle (see
    /// [`sims_sim::animator::Animator::reset`]).
    ///
    /// The person is marked as in the middle of a reset (object flag
    /// 0x800) throughout, so a broadcast to let go of an object that its
    /// callbacks set off skips it ([`crate::forget`]). A reset of a person
    /// already marked does nothing (0x45e7b2), as when the person's reset
    /// ends its use of itself and resets the object it used.
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
        // A person whose deletion has begun has its deletion mark
        // (object flag 0x40), whoever resets it.
        let deleting = deleting || self.being_deleted(id);
        // 0x45e7b2: a person already in the middle of a reset or of
        // letting go is skipped, as when its reset ends its use of itself.
        if busy.borrow().contains(&id) {
            return Ok(());
        }
        busy.borrow_mut().insert(id);
        let result = self.reset_marked_fully(world, id, people, deleting, None);
        busy.borrow_mut().remove(&id);
        result
    }

    /// A person's full reset as it lets go of object `named` that it uses
    /// or whose icon its running action shows (0x45e791 with the object,
    /// 0x45e7e3 to 0x45e814, then 0x45e814 to 0x45eac4): the reset of
    /// [`Interpreter::reset_person`], except that the objects it used are
    /// let go of and reset only if `named` lacks object flag 0x1000
    /// (0x45e7ca, the removal queue's mark,
    /// [`sims_sim::Object::removal_skips_resets`]), and its queue is left
    /// for the filters that follow (0x45eac7, [`crate::forget`]) instead
    /// of being emptied (0x45ecc2 is the path with no object). The person
    /// is already marked as letting go (object flag 0x800). A person whose
    /// deletion has begun keeps its deletion mark (object flag 0x40): it
    /// can be told to let go by a reset its own deletion set off.
    pub(crate) fn reset_person_letting_go(
        &self,
        world: &mut World,
        id: u16,
        named: u16,
    ) -> Result<(), Fault> {
        let deleting = self.being_deleted(id);
        self.reset_marked_fully(world, id, &self.program.people, deleting, Some(named))
    }

    /// Recovers a surviving person after an orphaned visitor it was interacting
    /// with was purged on saved-lot re-entry.
    ///
    /// Uses `reset_person_letting_go` so unrelated player orders already queued
    /// in `person.queue` are preserved (`0x45eac7`). When the person is in a
    /// state whose full reset is not ported (for example seated inside a chair,
    /// off-lot, or with word 73 set), performs an in-place action/stack reset
    /// via `install_main` while preserving both the person's seat and its
    /// surviving queue.
    pub(crate) fn reset_person_after_purge(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let in_container = capture(world, id).map_err(fail)?.container.is_some();
        if !in_container {
            let busy = RefCell::new(BTreeSet::new());
            busy.borrow_mut().insert(id);
            let outer = Interpreter {
                letting_go: Some(&busy),
                ..*self
            };
            match outer.reset_person_letting_go(world, id, id) {
                Ok(()) => return Ok(()),
                Err(Fault {
                    trap: Trap::Unsupported(_),
                    ..
                }) => {}
                Err(err) => return Err(err),
            }
        }
        if let Some(route) = world.routing.routes.get_mut(&id) {
            route.contexts.clear();
        }
        world.headlines.remove(&id);
        clear_person_uses_and_counts(world, id);
        {
            let obj = world
                .objects
                .get_mut(&id)
                .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
            obj.data[34] = 0;
            obj.data[9] = 0;
            let person = obj
                .person
                .as_mut()
                .ok_or_else(|| fail(Trap::native(0x1c, "object is not a person")))?;
            person.active = no_action();
            person.motive_deltas.clear();
            for word in [0, 30, 33, 40, 59, 71, 72] {
                if let Some(slot) = person.data.get_mut(word) {
                    *slot = 0;
                }
            }
        }
        self.machine(world, id, 0, false)
            .install_main()
            .map_err(fail)
    }

    /// The reset of a person marked as resetting (object flag 0x800):
    /// its uses released ([`Interpreter::release_uses`]), then the rest
    /// ([`Interpreter::reset_marked`]). `named` is the object being let go
    /// of, if any. Whether the objects used reset is decided once, before
    /// the loop, by object flag 0x1000 (0x45e7c8 to 0x45e7db): the named
    /// object's, or with none the person's own.
    ///
    /// A reset that releases uses and then fails leaves the lot as it was
    /// before the reset, and takes back the sounds, callbacks and
    /// deletions it noted, so the fault reports a consistent lot.
    fn reset_marked_fully(
        &self,
        world: &mut World,
        id: u16,
        people: &People,
        deleting: bool,
        named: Option<u16>,
    ) -> Result<(), Fault> {
        let reset_used = !world
            .objects
            .get(&named.unwrap_or(id))
            .is_some_and(|o| o.removal_skips_resets);
        let using = world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .is_some_and(|p| !p.object_uses.is_empty());
        let before = using.then(|| {
            (
                world.clone(),
                self.sounds.map(|s| s.borrow().len()),
                self.asked.map(|a| a.borrow().len()),
                self.deleted.map(|d| d.borrow().len()),
            )
        });
        let result = self
            .release_uses(world, id, reset_used)
            .and_then(|()| self.reset_marked(world, id, people, deleting, named.is_some()));
        if result.is_err()
            && let Some((lot, sounds, asked, deleted)) = before
        {
            *world = lot;
            if let (Some(s), Some(n)) = (self.sounds, sounds) {
                s.borrow_mut().truncate(n);
            }
            if let (Some(a), Some(n)) = (self.asked, asked) {
                a.borrow_mut().truncate(n);
            }
            if let (Some(d), Some(n)) = (self.deleted, deleted) {
                d.borrow_mut().truncate(n);
            }
        }
        result
    }

    /// The first step of a person's full reset (0x45e814 to 0x45e871):
    /// while the person uses an object (person +0xb00), the last use is
    /// ended.
    ///
    /// 1. The person's thread pops frames (0x4752f6, then the person's
    ///    virtual function 8, [`crate::interpreter::Machine::pop_frame`])
    ///    while it has more than the use's frame count less one. The last
    ///    pop releases the use ([`crate::interpreter::Machine::release_use`]),
    ///    and the person's running action ends when the list empties.
    /// 2. If `reset_used`, every person lets go of the object
    ///    ([`Interpreter::drop_references`], 0x444306), which skips this
    ///    one, being in the middle of its reset, and the object resets (its
    ///    virtual function 0x38 with 0: [`Interpreter::reset`], or
    ///    [`Interpreter::reset_person`] for a person). The original decides
    ///    that once, before the loop, by object flag 0x1000 (0x45e7c8 to
    ///    0x45e7db): the person's own with no object named, else the named
    ///    object's. The flag is the removal queue's mark
    ///    ([`sims_sim::Object::removal_skips_resets`]); the one other place
    ///    that sets it, a new object's reset at 0x42de09, is not ported.
    ///
    /// The object is the one the entry pointed to when the step began.
    /// One that is gone by then, when it is to reset, fails loudly, as
    /// does a step that leaves the list as long as it was, which the
    /// original repeats forever.
    fn release_uses(&self, world: &mut World, id: u16, reset_used: bool) -> Result<(), Fault> {
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
            if reset_used {
                if !world.objects.contains_key(&target) {
                    return Err(fail(Trap::Unsupported(format!(
                        "person {id}'s reset: object {target}, which it used, went as its \
                         frames were popped; the original resets it through a stale pointer \
                         (0x45e86e)"
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
        letting_go: bool,
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
        if person.data.get(73).is_none_or(|&word| word != 0) {
            return Err(unsupported("whose data word 73 is set"));
        }
        let profile = profile(people, world, guid).map_err(fail)?;
        if capture(world, id).map_err(fail)?.container.is_some() {
            return Err(fail(Trap::Unsupported(
                "resetting a person inside an object (0x45e8dd: take-off, place test and \
                 searches, with an animator refresh (0x58da12) that can stay pending \
                 beyond the scheduler pass), which is not ported"
                    .into(),
            )));
        }
        // 0x45e95d to 0x45e971: the deletion mark skips the lot bounds test.
        if !deleting && !stands_on_lot(world, id).map_err(fail)? {
            return Err(fail(Trap::Unsupported(
                "resetting a person off the lot (0x45e977: take-off, place test, searches \
                 and put, with an animator refresh (0x58da12) that can stay pending \
                 beyond the scheduler pass), which is not ported"
                    .into(),
            )));
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
        let last_result = object.last_result;
        let person = object
            .person
            .as_mut()
            .ok_or_else(|| fail(Trap::native(0x1c, "object is not a person")))?;
        for word in [0, 40, 59] {
            person.data[word] = 0;
        }
        // 0x45eab3: the running action becomes the empty action (0x45e6cd,
        // as `Machine::set_action` with no target): one with a target
        // becomes the last action, its flag 0x20 the object's last result
        // (+0x20); priority word 33 follows the empty action's, the motive
        // changes go, and words 71 to 73 are cleared.
        if person.data[73] != 0 {
            return Err(unsupported("whose data word 73 is set"));
        }
        if person.active.target != 0 {
            let mut last = person.active.clone();
            last.flags = (last.flags & !0x20) | (u32::from(last_result) << 5);
            person.last = last;
        }
        person.active = no_action();
        person.motive_deltas.clear();
        for word in [33, 71, 72, 73] {
            person.data[word] = 0;
        }
        // 0x45eac7: letting go of an object, the queue is left to the
        // filters (`crate::forget`); with none, it is emptied (0x45ecc2).
        if letting_go {
            return Ok(());
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
