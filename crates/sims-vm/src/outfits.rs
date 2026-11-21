//! Person outfit dressing (`cTSPerson::Dress`, `0x58a178`), covering all 61
//! native suit IDs (`0..=60`), NPC flag `0x08` (`0x58a1e4`), pet suit reset
//! (`0x58a2c7..0x58a2e5`), cached-guard early exit (`0x58a2ed..0x58a333`),
//! per-slot attachment retention on lookup miss (`0x583559` / `0x583576`),
//! head slot invalidation before rebinding (`0x58d4c1..0x58d51d` → `0x4eb0cc` /
//! `0x4eb0eb`), extra hand pieces (`0x58a453..0x58a675` → `0x58d712..0x58d75e`),
//! career workwear (`0x45ee5f`), matched pajamas (`0x45f034`), skeleton
//! costumes (`0x45f15a`), party costumes (`0x45f452`), and Superstar / Makin'
//! Magic costumes (`0x58a948..0x58d475`).

use crate::Program;
use crate::fault::Trap;
use crate::people::{ResolvedSuit, resolve_suit};
use sims_content::{Catalog, People};
use sims_sim::{BoundSlot, World};

/// Person data word indices read/written by `0x58a178`.
const BODY_SUIT: usize = 8;
const VARIANT: usize = 21;
const HAND_LEFT: usize = 22;
const HAND_RIGHT: usize = 23;
const JOB_TYPE: usize = 56;
const JOB_LEVEL: usize = 57;
const AGE: usize = 58;
const GENDER: usize = 65;

/// Parses a `STR# 200` outfit descriptor (`0x5874ca`) into `(suit, texture)`.
///
/// Splits on `;` (`second_half == false` takes the first half, `true` takes
/// the second half when `;` is present), then splits on `,`:
/// - `suit` (before `,`) is lowercased in ASCII (`0x46f8f7`);
/// - `texture` (after `,`): when it contains `=`, the prefix through `=` keeps
///   its original case while the value after `=` is lowercased (`0x587612`–
///   `0x58765c`); otherwise the whole texture is lowercased (`0x587670`).
pub(crate) fn parse_row_descriptor(row: &str, second_half: bool) -> (String, String) {
    let half = if second_half {
        match row.find(';') {
            Some(pos) => &row[pos + 1..],
            None => "",
        }
    } else {
        match row.find(';') {
            Some(pos) => &row[..pos],
            None => row,
        }
    };
    let (suit_raw, tex_raw) = match half.find(',') {
        Some(pos) => (&half[..pos], Some(&half[pos + 1..])),
        None => (half, None),
    };
    let suit = suit_raw.to_ascii_lowercase();
    let texture = match tex_raw {
        None => String::new(),
        Some(rest) => match rest.find('=') {
            Some(eq) => {
                let mut out = String::with_capacity(rest.len());
                out.push_str(&rest[..=eq]);
                out.push_str(&rest[eq + 1..].to_ascii_lowercase());
                out
            }
            None => rest.to_ascii_lowercase(),
        },
    };
    (suit, texture)
}

/// Parses an ASCII integer prefix the way CRT `atoi` does (`0x58a712`).
fn atoi_age(s: &str) -> i32 {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let neg = if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
        true
    } else {
        if i < bytes.len() && bytes[i] == b'+' {
            i += 1;
        }
        false
    };
    let mut acc: i32 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        acc = acc
            .wrapping_mul(10)
            .wrapping_add(i32::from(bytes[i] - b'0'));
        i += 1;
    }
    if neg { acc.wrapping_neg() } else { acc }
}

/// Expands a `$g`/`$b`/`$c` template (`0x40e477`) into candidate ASCII
/// case-insensitive prefix patterns (branching on `$g` → `('f'|'u')` for
/// female, `('m'|'u')` for male).
fn expand_suit_template(
    template: &str,
    is_female: bool,
    body_type: &str,
    skin_tone: &str,
) -> Vec<String> {
    let mut prefixes = vec![String::new()];
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'g' | b'G' => {
                    let g = if is_female { 'f' } else { 'm' };
                    let mut next = Vec::with_capacity(prefixes.len() * 2);
                    for p in &prefixes {
                        let mut a = p.clone();
                        a.push(g);
                        next.push(a);
                        let mut b = p.clone();
                        b.push('u');
                        next.push(b);
                    }
                    prefixes = next;
                    i += 2;
                    continue;
                }
                b'b' | b'B' => {
                    for p in &mut prefixes {
                        p.push_str(body_type);
                    }
                    i += 2;
                    continue;
                }
                b'c' | b'C' => {
                    for p in &mut prefixes {
                        p.push_str(skin_tone);
                    }
                    i += 2;
                    continue;
                }
                _ => {
                    i += 2;
                    continue;
                }
            }
        }
        for p in &mut prefixes {
            p.push(bytes[i] as char);
        }
        i += 1;
    }
    prefixes
}

/// Matches a template against the avatar registry's appearance table
/// (`0x40e5ce` / `0x40e9b2` → `0x409255`, skipping `"suit-*"` at `0x409296`),
/// returning the first appearance whose name starts with any expanded prefix
/// (ASCII case-insensitive, `0x4727f1`).
fn match_registered_suit(
    people: &People,
    template: &str,
    is_female: bool,
    body_type: &str,
    skin_tone: &str,
) -> String {
    if template.is_empty() {
        return String::new();
    }
    let patterns = expand_suit_template(template, is_female, body_type, skin_tone);
    if patterns.iter().all(String::is_empty) {
        return String::new();
    }
    let matches_any = |cand: &str| -> bool {
        if cand.starts_with("suit-") {
            return false;
        }
        patterns.iter().any(|pat| {
            !pat.is_empty()
                && cand.len() >= pat.len()
                && cand[..pat.len()].eq_ignore_ascii_case(pat)
        })
    };
    for name in people
        .eager_suits
        .iter()
        .chain(people.indexed_suits.keys())
        .chain(people.suits.iter())
        .chain(people.head_suits.iter())
    {
        if matches_any(name) {
            return name.clone();
        }
    }
    String::new()
}

/// Expands a hand template (`0x40e795`), replacing `$h` with `'O'`/`'P'`/`'C'`
/// for gesture 0/1/2 and `$g` with `'f'`/`'m'`, prefixing `"HAND="` when no
/// `$g` was present (`0x40e7ec`).
fn expand_hand_template(template: &str, gesture: i16, is_female: bool) -> String {
    let ch = match gesture {
        1 => 'P',
        2 => 'C',
        _ => 'O',
    };
    let mut out = if let Some(pos) = template.find("$h") {
        let mut s = String::new();
        if !template.contains("$g") {
            s.push_str("HAND=");
        }
        s.push_str(&template[..pos]);
        s.push(ch);
        s.push_str(&template[pos + 2..]);
        s
    } else {
        template.to_owned()
    };
    if let Some(pos) = out.find("$g") {
        let g = if is_female { 'f' } else { 'm' };
        let mut s = String::with_capacity(out.len());
        s.push_str(&out[..pos]);
        s.push(g);
        s.push_str(&out[pos + 2..]);
        out = s;
    }
    out
}

/// Costume table entry for Superstar & Makin' Magic suits (`19..=60`).
struct CostumeSpec {
    body: Option<&'static str>,
    body_tex: Option<&'static str>,
    head: Option<&'static str>,
    head_tex: Option<&'static str>,
    hand_left: Option<&'static str>,
    hand_right: Option<&'static str>,
}

const fn body_only(body: &'static str, body_tex: &'static str) -> CostumeSpec {
    CostumeSpec {
        body: Some(body),
        body_tex: Some(body_tex),
        head: None,
        head_tex: None,
        hand_left: None,
        hand_right: None,
    }
}

const fn full_costume(
    body: &'static str,
    body_tex: &'static str,
    head: &'static str,
    head_tex: &'static str,
    hand: &'static str,
) -> CostumeSpec {
    CostumeSpec {
        body: Some(body),
        body_tex: Some(body_tex),
        head: Some(head),
        head_tex: Some(head_tex),
        hand_left: Some(hand),
        hand_right: Some(hand),
    }
}

fn costume_spec(suit: i16, variant: i16) -> Option<CostumeSpec> {
    match suit {
        19 => Some(body_only(
            "superstar$gaFit_movieactor1",
            "superstar$gaFit$c_movieactor1",
        )),
        20 => Some(body_only(
            "superstar$gaFit_movieactor2",
            "superstar$gaFit$c_movieactor2",
        )),
        21 => Some(body_only(
            "superstar$gaFit_movieactor3",
            "superstar$gaFit$c_movieactor3",
        )),
        22 => Some(body_only(
            "Superstar$gAfit_Doctor",
            "Superstar$gAfit$c_Doctor",
        )),
        23 => Some(body_only(
            "Superstar$gAfit_Relative",
            "Superstar$gAfit$c_Relative",
        )),
        24 => Some(body_only("Runway$gAfit_Coffee", "Runway$gAfit$c_Coffee")),
        25 => Some(body_only("Runway$gAfit_Floral", "Runway$gAfit$c_Floral")),
        26 => Some(body_only(
            "Runway$gAfit_Architectural",
            "Runway$gAfit$c_Architectural",
        )),
        27 => Some(body_only("Runway$gAfit_Formal1", "Runway$gAfit$c_Formal1")),
        28 => Some(body_only("Runway$gAfit_Formal2", "Runway$gAfit$c_Formal2")),
        29 => Some(body_only("Runway$gAfit_Formal3", "Runway$gAfit$c_Formal3")),
        30 => Some(body_only(
            "Runway$gAfit_Lingerie1",
            "Runway$gAfit$c_Lingerie1",
        )),
        31 => Some(body_only(
            "Runway$gAfit_Lingerie2",
            "Runway$gAfit$c_Lingerie2",
        )),
        32 => Some(body_only(
            "Runway$gAfit_Lingerie3",
            "Runway$gAfit$c_Lingerie3",
        )),
        33 => Some(body_only(
            "Superstar$gAfit_JumpSuit",
            "Superstar$gAfit$c_JumpSuit",
        )),
        34 => Some(body_only(
            "Superstar$gAfit_WetSuit",
            "Superstar$gAfit$c_WetSuit",
        )),
        35 => Some(body_only(
            "Superstar$gAFit_BackupDancerDance",
            "Superstar$gAFit$c_BackupDancerDance",
        )),
        36 => Some(body_only(
            "Superstar$gAfit_BackupDancerPop",
            "Superstar$gAfit$c_BackupDancerPop",
        )),
        37 => Some(body_only(
            "Superstar$gAfit_BackupDancerRock",
            "Superstar$gAfit$c_BackupDancerRock",
        )),
        38 => Some(CostumeSpec {
            body: Some("MagicMAFat_BeeKeeper"),
            body_tex: Some("MagicMAFat_BeeKeeper"),
            head: None,
            head_tex: None,
            hand_left: Some("HUA$h_BeeKeeper"),
            hand_right: Some("HUA$h_BeeKeeper"),
        }),
        39 => Some(full_costume(
            "Magic$gAFit_BlueGenie",
            "BMagic$gAFit_BlueGenie",
            "CMagic$gA_BlueGenie",
            "CMagic$gAFit_BlueGenie",
            "H$gA$h_BlueGenie",
        )),
        40 => Some(full_costume(
            "MagicMAFit_Minotaur",
            "BMagicMAFit_Minotaur",
            "CMagicMAFit_Minotaur",
            "CMagicMAFit_Minotaur",
            "HMA$h_Minotaur",
        )),
        41 => Some(body_only(
            "Magic$gAFit_Spookshow",
            "Magic$gAFit$c_Spookshow",
        )),
        42 => Some(body_only("Magic$gAFit_SideShow", "Magic$gAFit$c_SideShow")),
        43 => Some(full_costume(
            "MagicFAFit_Vampire",
            "MagicFAFit_Vampire",
            "CMagicFAFit_Vampire",
            "CMagicFAFit_Vampire",
            "HFA$h_Vampire",
        )),
        44 => Some(full_costume(
            "Magic$gAFit_Werewolf",
            "Magic$gAFit_Werewolf",
            "CMagic$gAFit_Werewolf",
            "CMagic$gAFit_Werewolf",
            "HMA$h_Werewolf",
        )),
        45 => Some(full_costume(
            "MagicMAFit_Headless",
            "MagicMAFit_Headless",
            "CMagicMA_Headless",
            "CMagicMA_Headless",
            "HUA$h_Headless",
        )),
        46 => Some(full_costume(
            "MagicMAFat_Troll",
            "MagicMAFat_Troll",
            "CMagicMAFat_Troll",
            "CMagicMAFat_Troll",
            "HUA$h_Troll",
        )),
        47 => Some(full_costume(
            "MagicMAFat_Hunchback",
            "MagicMAFat_Hunchback",
            "CMagicMAFat_Hunchback",
            "CMagicMAFat_Hunchback",
            "HUA$h_Hunchback",
        )),
        48 => Some(full_costume(
            "Magic$gAFit_Sorcerer",
            "Magic$gAFit_Sorcerer",
            "CMagic$gAFit_Sorcerer",
            "CMagic$gAFit_Sorcerer",
            "HMA$h_Sorcerer",
        )),
        49 => Some(full_costume(
            "MagicFAFit_Witch",
            "MagicFAFit_Witch",
            "CMagicFAFit_Witch",
            "CMagicFAFit_Witch",
            "HUA$h_Hunchback",
        )),
        50 => Some(full_costume(
            "MagicFAFat_Ogress",
            "BMagicFAFat_Ogress",
            "CMagicFAFat_Ogress",
            "CMagicFAFat_Ogress",
            "HFA$h_Ogress",
        )),
        51 => Some(full_costume(
            "MagicFASkn_Crone",
            "BMagicFASkn_Crone",
            "CMagicFASkn_Crone",
            "CMagicFASkn_Crone",
            "HFA$h_Crone",
        )),
        52 => Some(full_costume(
            "MagicFASkn_Nymph",
            "BMagicFASkn_Nymph",
            "CMagicFASkn_Nymph",
            "CMagicFASkn_Nymph",
            "HUA$h_Nymph",
        )),
        53 => Some(full_costume(
            "MagicMCChd_Goblin",
            "MagicMCChd_Goblin",
            "CMagicMCChd_Goblin",
            "CMagicMCChd_Goblin",
            "HUC$h_Goblin",
        )),
        54 => Some(full_costume(
            "MagicFCChd_WizardGirl",
            "MagicFCChd_WizardGirl",
            "CMagicFC_WizardGirl",
            "CMagicFC_WizardGirl",
            "HFC$h_WizardGirl",
        )),
        55 | 56 => match variant {
            0 => Some(full_costume(
                "MagicMAFit_MummyFull",
                "MagicMAFit_Mummy",
                "CMagicMA_Mummy",
                "CMagicMA_Mummy",
                "HUA$h_Mummy",
            )),
            1 => Some(full_costume(
                "MagicMAFit_MummyFull",
                "MagicMAFit_Mummy",
                "CMagicMA_Headless",
                "CMagicMA_Headless",
                "HUA$h_Mummy",
            )),
            2 => Some(full_costume(
                "MagicMAFit_MummyTorso",
                "MagicMAFit_Mummy",
                "CMagicMA_Headless",
                "CMagicMA_Headless",
                "HUA$h_Mummy",
            )),
            3 => Some(CostumeSpec {
                body: Some("MagicMAFit_MummyLegs"),
                body_tex: Some("MagicMAFit_Mummy"),
                head: Some("CMagicMA_Headless"),
                head_tex: Some("CMagicMA_Headless"),
                hand_left: Some("nohandl"),
                hand_right: Some("nohandr"),
            }),
            _ => Some(full_costume(
                "MagicMAFit_MummyFull",
                "MagicMAFit_Mummy",
                "CMagicMA_Mummy",
                "CMagicMA_Mummy",
                "HUA$h_Mummy",
            )),
        },
        57 => Some(CostumeSpec {
            body: None,
            body_tex: None,
            head: Some("CMagicMAFat_Donkey"),
            head_tex: Some("CMagicMAFat_Donkey"),
            hand_left: None,
            hand_right: None,
        }),
        58 => Some(full_costume(
            "MagicMASkn_Vampire",
            "MagicMASkn_Vampire",
            "CMagicMASkn_Vampire",
            "CMagicMASkn_Vampire",
            "HMA$h_Vampire",
        )),
        59 => Some(body_only(
            "MagicMAFit_PrinceCharming",
            "MagicMAFitlgt_PrinceCharming",
        )),
        60 => Some(full_costume(
            "MagicMCChd_Skeleton",
            "MagicMCChd_Skeleton",
            "CMagicMCChd_Skeleton",
            "CMagicMCChd_Skeleton",
            "GUA$h_SkeletonChild",
        )),
        _ => None,
    }
}

/// Binds `(suit, texture)` onto `slot` following `0x583559`:
/// - empty `suit` returns immediately without modifying `slot` (`0x583569`);
/// - non-empty `suit` (including literal `"none"`) is lowercased and resolved
///   via `0x5a044d` (`resolve_suit`);
/// - `Found(has_head)` replaces `slot`;
/// - `Missing` returns before `0x58359c` detaches the previous attachment;
/// - `Unknown` (decode error prevented knowing whether `suit` exists/binds
///   `"HEAD"`) preserves the slot's uncertainty (`has_head: None`).
fn bind_slot(
    people: &People,
    world: &mut World,
    slot: &mut Option<BoundSlot>,
    suit: &str,
    texture: &str,
) {
    if suit.is_empty() {
        return;
    }
    let lower = suit.to_ascii_lowercase();
    match resolve_suit(people, world, &lower) {
        ResolvedSuit::Found(has_head) => {
            *slot = Some(BoundSlot {
                suit: lower,
                texture: texture.to_owned(),
                has_head: Some(has_head),
            });
        }
        ResolvedSuit::Missing => {
            // 0x583576: lookup miss returns without detaching the previous slot.
        }
        ResolvedSuit::Unknown => {
            *slot = Some(BoundSlot {
                suit: lower,
                texture: texture.to_owned(),
                has_head: None,
            });
        }
    }
}

/// Whether `existing` differs from `(suit, texture)` per `0x58a12f`:
/// returns `true` when `existing` is empty or either `suit` or `texture`
/// differs (byte-exact per `0x59384d`).
fn slot_differs(existing: &Option<BoundSlot>, suit: &str, texture: &str) -> bool {
    match existing {
        None => true,
        Some(bound) => bound.suit != suit || bound.texture != texture,
    }
}

/// Executes `cTSPerson::Dress(force)` (`0x58a178`) on `person_id` in `world`.
///
/// Called from:
/// - `person_factory::construction_reset` (`0x588930`, `force = false`)
/// - `frame::draw_person` (`0x590cb8`, `force = false`)
/// - portrait rendering in `ui_effect` / `dialog` (`0x4e7801`, `force = true`)
pub(crate) fn dress_person(
    catalog: &Catalog,
    program: &Program,
    world: &mut World,
    person_id: u16,
    force: bool,
) -> Result<(), Trap> {
    let Some(object) = world.objects.get(&person_id) else {
        return Ok(());
    };
    let guid = object.guid;
    let Some(person) = object.person.as_ref() else {
        return Ok(());
    };
    let word = |idx: usize| person.data.get(idx).copied().unwrap_or(0);
    let raw_suit = word(BODY_SUIT);
    let variant = word(VARIANT);
    let hand_left_word = word(HAND_LEFT);
    let hand_right_word = word(HAND_RIGHT);
    let job_type = word(JOB_TYPE);
    let job_level = word(JOB_LEVEL);
    let age_word = word(AGE);
    let gender_word = word(GENDER);

    let def_flags = crate::interpreter::object_definition(catalog, world, guid)
        .and_then(|def| program.definition(def.resource, def.chunk))
        .map(|w| w[102])
        .unwrap_or(0);

    let template = world.characters.definition(guid);
    let profile = program.people.profiles.get(&template);
    let made_strings = world.characters.made.get(&guid).map(|m| m.strings.clone());
    let made_outfit = world.made_outfits.get(&guid).cloned();

    let raw_row = |idx: usize| -> Option<String> {
        if let Some(s) = made_strings.as_ref().and_then(|m| m.get(&idx)) {
            return Some(s.clone());
        }
        if let Some(s) = made_outfit.as_ref().and_then(|rows| rows.get(&idx)) {
            return Some(s.clone());
        }
        match idx {
            1 => profile.and_then(|p| p.body.clone()),
            2 => profile.and_then(|p| p.head.clone()),
            n => profile.and_then(|p| p.suit_rows.get(&n).cloned()),
        }
    };
    let row_desc = |idx: usize| -> (String, String) {
        raw_row(idx)
            .map(|r| parse_row_descriptor(&r, false))
            .unwrap_or_default()
    };

    // 0x58a1e4: definitions with flag `0x08` bypass the normal dressing path.
    if (def_flags & 8) != 0 {
        // 0x58a1f8: `raw_suit <= 0` jumps directly to `0x58d78a`, leaving all
        // attachment slots, `data[8]`, `.ndx` registries, and portraits alone.
        if raw_suit <= 0 {
            return Ok(());
        }
        // 0x58a1fe..0x58a2c2: `raw_suit > 0` binds `STR# 200` row `raw_suit`
        // onto the primary body slot (`0x583559`) and jumps to `0x58d78a`
        // without touching the head slot or calling `0x4eb0cc`.
        let (suit, tex) = row_desc(raw_suit as usize);
        let mut outfit = world.person_outfits.remove(&person_id).unwrap_or_default();
        bind_slot(&program.people, world, &mut outfit.body, &suit, &tex);
        world.person_outfits.insert(person_id, outfit);
        return Ok(());
    }

    // 0x58a2c7..0x58a2e5: pets reset live `person.data[8] = 0`.
    let suit = if (gender_word & 6) != 0 && raw_suit != 0 {
        if let Some(slot) = world
            .objects
            .get_mut(&person_id)
            .and_then(|o| o.person.as_mut())
            .and_then(|p| p.data.get_mut(BODY_SUIT))
        {
            *slot = 0;
        }
        world.routing.unknown_words.remove(&(person_id, BODY_SUIT));
        0
    } else {
        raw_suit
    };

    let dress_key = [
        i32::from(suit),
        i32::from(variant),
        i32::from(hand_left_word),
        i32::from(hand_right_word),
    ];

    let mut outfit = world.person_outfits.remove(&person_id).unwrap_or_default();

    // 0x58a2ed..0x58a333: when `force == false` and `[ebx + 0x1b0..0x1bc]`
    // match `(data[8], data[21], data[22], data[23])`, exit immediately.
    if !force && outfit.last_dressed == dress_key {
        world.person_outfits.insert(person_id, outfit);
        return Ok(());
    }
    outfit.last_dressed = dress_key;

    // 0x40ccac: female when `STR# 200` row 12 is `"female"`, falling back to
    // `person.data[65] & 1`.
    let is_female = made_strings
        .as_ref()
        .and_then(|m| m.get(&sims_sim::characters::GENDER).map(String::as_str))
        .or_else(|| profile.and_then(|p| p.gender.as_deref()))
        .map_or((gender_word & 1) != 0, |g| g.eq_ignore_ascii_case("female"));
    let body_row_str = raw_row(1).unwrap_or_default();
    let body_type: String = body_row_str.get(6..9).unwrap_or("").to_ascii_lowercase();
    let skin_tone: String = made_strings
        .as_ref()
        .and_then(|m| m.get(&sims_sim::characters::SKIN).map(String::as_str))
        .or_else(|| profile.and_then(|p| p.skin.as_deref()))
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_formal_child = {
        let age_str = made_strings
            .as_ref()
            .and_then(|m| m.get(&sims_sim::characters::AGE).map(String::as_str))
            .or_else(|| profile.and_then(|p| p.age.as_deref()))
            .unwrap_or("");
        atoi_age(age_str) <= 17
    };
    let is_word_child = age_word != 0 && age_word < 18;

    // Default head (row 2) and hand extras (words 22 & 23, `0x58a453..0x58a675`).
    let (mut head_suit, mut head_tex) = row_desc(2);
    let (mut extra_left_suit, mut extra_left_tex) = match hand_left_word {
        1 => row_desc(19),
        2 => row_desc(21),
        3 => ("nohandl".to_owned(), "nohand".to_owned()),
        4 => row_desc(25),
        _ => row_desc(17),
    };
    let (mut extra_right_suit, mut extra_right_tex) = match hand_right_word {
        1 => row_desc(20),
        2 => row_desc(22),
        3 => ("nohandr".to_owned(), "nohand".to_owned()),
        4 => row_desc(26),
        _ => row_desc(18),
    };
    let mut body_suit = String::new();
    let mut body_tex = String::new();
    let mut body_alt_suit = String::new();

    match suit {
        0 | ..=-1 | 61.. => {
            (body_suit, body_tex) = row_desc(1);
        }
        1 => {
            (body_suit, body_tex) = row_desc(15);
        }
        2 => {
            let (s31, t31) = row_desc(31);
            if !s31.is_empty() {
                body_suit = s31;
                body_tex = t31;
            } else {
                (body_suit, body_tex) = row_desc(16);
            }
        }
        3 => {
            // 0x45ee5f: career workwear from `Work.iff` (`Program::careers`).
            let level_idx = if is_word_child { 0 } else { job_level };
            if let Some(careers) = program.careers.as_ref()
                && let Some(career) = careers.iter().find(|c| c.id == i32::from(job_type))
                && let Ok(lvl) = usize::try_from(level_idx)
                && let Some(level) = career.levels.get(lvl)
            {
                let tmpl = if is_female && !level.strings[2].is_empty() {
                    &level.strings[2]
                } else {
                    &level.strings[1]
                };
                body_suit =
                    match_registered_suit(&program.people, tmpl, is_female, &body_type, &skin_tone);
                body_tex = level.strings[3].to_ascii_lowercase();
                body_alt_suit = level.strings[4].to_ascii_lowercase();
            }
        }
        4 => {
            // 0x58a6ff..0x58a7b0: formalwear checks `atoi(STR# 200 row 13) <= 17`.
            if is_formal_child {
                (body_suit, body_tex) = row_desc(27);
            } else {
                let (s30, t30) = row_desc(30);
                if !s30.is_empty() {
                    body_suit = s30;
                    body_tex = t30;
                } else {
                    (body_suit, body_tex) = row_desc(27);
                    (extra_left_suit, extra_left_tex) = row_desc(28);
                    (extra_right_suit, extra_right_tex) = row_desc(29);
                }
            }
        }
        5 => {
            // 0x58a7db: row 32 if non-empty; otherwise matched pajamas (`0x45f034`).
            let (s32, t32) = row_desc(32);
            if !s32.is_empty() {
                body_suit = s32;
                body_tex = t32;
            } else {
                let tmpl = if is_word_child {
                    "pjs$gchd"
                } else {
                    "pajama$g$b"
                };
                body_suit =
                    match_registered_suit(&program.people, tmpl, is_female, &body_type, &skin_tone);
                body_tex = "pajama$g$c".to_owned();
            }
        }
        6 | 7 => {
            // 0x45f15a: skeleton (suit 6) and electric-shock skeleton (suit 7).
            let shock = suit == 7;
            if is_word_child {
                body_suit = "skeletonchd_01".to_owned();
                head_suit = "c_kidskull".to_owned();
            } else {
                body_suit = "skeleton_01".to_owned();
                head_suit = "c_skeleton".to_owned();
            }
            if shock {
                body_tex = "BODY=skeleneg_01".to_owned();
                head_tex = "HEAD-HEAD=c_skeleneg".to_owned();
                extra_left_tex = "HAND=_GUAO_sneg".to_owned();
                extra_right_tex = "HAND=_GUAO_sneg".to_owned();
            } else {
                body_tex = "BODY=skeleton_01".to_owned();
                head_tex = "HEAD-HEAD=c_skeleton".to_owned();
                extra_left_tex = "HAND=_GUAO_skel".to_owned();
                extra_right_tex = "HAND=_GUAO_skel".to_owned();
            }
        }
        8..=16 => {
            // 0x45f452: themed party costumes (`ct-{prefix}-$gC-{v:02}` /
            // `ct-{prefix}-$gAFit-{v:02}` with `v = data[21] + 1`).
            const PREFIXES: [&str; 9] = [
                "toga", "cow", "luau", "rave", "cost", "nice", "swim", "pj", "disco",
            ];
            let prefix = PREFIXES[(suit - 8) as usize];
            let v = i32::from(variant) + 1;
            let tmpl = if is_word_child {
                format!("ct-{prefix}-$gC-{v:02}")
            } else {
                format!("ct-{prefix}-$gAFit-{v:02}")
            };
            body_suit =
                match_registered_suit(&program.people, &tmpl, is_female, &body_type, &skin_tone);
            body_tex = tmpl;
        }
        17 => {
            (body_suit, body_tex) = row_desc(33);
        }
        18 => {
            (body_suit, body_tex) = row_desc(34);
        }
        19..=60 => {
            if let Some(spec) = costume_spec(suit, variant) {
                if let Some(b) = spec.body {
                    body_suit = match_registered_suit(
                        &program.people,
                        b,
                        is_female,
                        &body_type,
                        &skin_tone,
                    );
                }
                if let Some(bt) = spec.body_tex {
                    body_tex = bt.to_owned();
                }
                if let Some(h) = spec.head {
                    head_suit = match_registered_suit(
                        &program.people,
                        h,
                        is_female,
                        &body_type,
                        &skin_tone,
                    );
                }
                if let Some(ht) = spec.head_tex {
                    head_tex = ht.to_owned();
                }
                if let Some(hl) = spec.hand_left {
                    if hl == "nohandl" {
                        extra_left_suit = "nohandl".to_owned();
                        extra_left_tex = "nohand".to_owned();
                    } else {
                        extra_left_tex = expand_hand_template(hl, hand_left_word, is_female);
                    }
                }
                if let Some(hr) = spec.hand_right {
                    if hr == "nohandr" {
                        extra_right_suit = "nohandr".to_owned();
                        extra_right_tex = "nohand".to_owned();
                    } else {
                        extra_right_tex = expand_hand_template(hr, hand_right_word, is_female);
                    }
                }
            }
        }
    }

    // 0x58d494..0x58d4bc: when `body_suit` is empty and `suit != 0`, fall back
    // to everyday body row 1.
    if body_suit.is_empty() && suit != 0 {
        (body_suit, body_tex) = row_desc(1);
    }

    // 0x58d4c1..0x58d51d: inspect the EXISTING head slot (`[ebx + 0x258]`)
    // BEFORE rebinding:
    // - empty head slot -> `0x58d4e1 -> 0x4eb0cc -> 0x4e897e` clears cached
    //   portraits AND clears dirty flags, and schedules head rebind;
    // - bound head slot whose `(suit, texture)` differs (`0x58a12f`) ->
    //   `0x58d518 -> 0x4eb0eb -> 0x4e895e` clears cached portraits AND marks
    //   dirty, and schedules head rebind;
    // - unchanged bound head slot -> skips `0x58d606` and leaves portrait state alone.
    let rebind_head = if outfit.head.is_none() {
        world.portraits.remove(&guid);
        world.dirty_portraits.remove(&guid);
        true
    } else if slot_differs(&outfit.head, &head_suit, &head_tex) {
        world.portraits.remove(&guid);
        world.dirty_portraits.insert(guid);
        true
    } else {
        false
    };

    // 0x58d51e..0x58d5a3: check whether body (`[esi][0]`) and body_alt
    // (`[esi][1]`) need rebinding.
    let rebind_body = slot_differs(&outfit.body, &body_suit, &body_tex);
    let rebind_body_alt = if outfit.body_alt.is_some() {
        slot_differs(&outfit.body_alt, &body_alt_suit, "")
    } else {
        !body_alt_suit.is_empty()
    };

    // 0x58d5b4..0x58d70f: bind head, body, and body_alt in exact native order.
    if rebind_head {
        bind_slot(
            &program.people,
            world,
            &mut outfit.head,
            &head_suit,
            &head_tex,
        );
    }
    if rebind_body {
        bind_slot(
            &program.people,
            world,
            &mut outfit.body,
            &body_suit,
            &body_tex,
        );
    }
    if rebind_body_alt {
        bind_slot(
            &program.people,
            world,
            &mut outfit.body_alt,
            &body_alt_suit,
            "",
        );
    }

    // 0x58d712..0x58d75e: bind extra_left (`0x58d742`) and extra_right
    // (`0x58d759`) via `0x58a070` (which checks `0x58a12f`) whenever BOTH have
    // non-empty suit names.
    if !extra_left_suit.is_empty() && !extra_right_suit.is_empty() {
        if slot_differs(&outfit.extra_left, &extra_left_suit, &extra_left_tex) {
            bind_slot(
                &program.people,
                world,
                &mut outfit.extra_left,
                &extra_left_suit,
                &extra_left_tex,
            );
        }
        if slot_differs(&outfit.extra_right, &extra_right_suit, &extra_right_tex) {
            bind_slot(
                &program.people,
                world,
                &mut outfit.extra_right,
                &extra_right_suit,
                &extra_right_tex,
            );
        }
    }

    world.person_outfits.insert(person_id, outfit);
    Ok(())
}
