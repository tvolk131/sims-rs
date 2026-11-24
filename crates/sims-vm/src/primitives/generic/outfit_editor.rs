//! Generic Sims Call routine 24 (0x44ff6e): change one of the running
//! person's outfits through the outfit editor.
//!
//! The dressers' "Change Normal Outfit" (category 0, the index the
//! clothing dialog picked), the clothing racks' "Set New" and "Revert"
//! and the pet pens' suit changes run it. Temp 0 is the category and
//! temp 1 the index in that category's wardrobe list.
//!
//! The native routine, in order:
//!
//! 1. Casts the running object to a person (0x635abe); anything else
//!    throws native error 0x1c (0x44ff8b).
//! 2. Builds an editor (0x408b3b), keeps person word 8 (+0x59c), and
//!    loads the person's strings into it (0x409315,
//!    [`outfits::init_clothing_rows`]).
//! 3. If temp 1 is not negative (0x44ffd0), picks the category (0x44ffda)
//!    and, when temp 1 is below the category's list length, keeps the
//!    index the person wears now and writes entry temp 1 into the
//!    category's string row without committing (its set accessor with
//!    0). The categories, as count / current / set accessors and the
//!    1-based slot each writes (one more than the row number here):
//!
//!    | temp 0 | accessors | slot | row | list |
//!    | --- | --- | --- | --- | --- |
//!    | 0 | 0x40d077 / 0x40d0a4 / 0x40d10b | 2 | 1, body | 0x408bf5(0) |
//!    | 1 | 0x40d4ff / 0x40d52f / 0x40d599 | 0x1f | 30, formal | 0x408d8e, `f` |
//!    | 2 | 0x40d817 / 0x40d847 / 0x40d8b1 | 0x20 | 31, swimwear | 0x408e7b, `s` |
//!    | 3 | 0x40db2f / 0x40db5f / 0x40dbc9 | 0x21 | 32, pajamas | 0x408f68, `l` |
//!    | 4 | 0x40de47 / 0x40de77 / 0x40dee1 | 0x22 | 33, winter | 0x409055, `w` |
//!    | 5 | 0x40e15f / 0x40e18f / 0x40e1f9 | 0x23 | 34, high fashion | 0x409142, `h` |
//!
//!    Any other category writes nothing.
//! 4. Commits (0x450154: 0x409492(1)), which runs 0x40a588 (below) and
//!    writes the strings back to the definition (0x587aec, 0x587bbd).
//! 5. Puts person word 8 back, refreshes the person's portrait
//!    (0x4572af → 0x4e90c9, 0x4eb0cc) and marks the control panel for
//!    redrawing (`[0x6f2a58]+0xb0`, 0x4ceceb(0x20000000)). Those are
//!    presentation.
//! 6. Temp 1 becomes the kept index (0x4501db), and the routine takes the
//!    true branch (0x450656 → 0x44f7f0).
//!
//! Routines 29 (0x4501bd), 30 (0x450528) and 31 (0x4503f2) share the
//! editor with the stack object as the person: 29 is routine 24 again,
//! 30 reads the current index and 31 the list length, both committing
//! with 0 (no 0x40a588). They are not ported here.

use crate::Program;
use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::outfits::{self, BodyCommitState};
use sims_sim::World;

/// One entry of an editor list (0x472e7e): the suit and the descriptor,
/// each kept to 63 Windows-1252 bytes, and the row they make joined by a
/// comma (0x472f41).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    suit: String,
    descriptor: String,
    row: String,
}

/// Which list an editor accessor reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum List {
    /// 0x408bf5(0): bodies (row 1).
    Body,
    /// 0x408bf5(1): heads (row 2), read by the commit's check (0x40d408).
    Head,
    /// 0x408d8e and its kin: the clothing of one category letter.
    Clothing(u8),
}

/// What the editor reads of the person's strings: the gender code
/// (0x40ccac, row 12), whether the person is a child (0x40cbdd, row 13
/// at most 17) and the skin tone (0x40d007, row 14's first three
/// characters, lowercased, or `???`).
struct Wearer {
    gender: i32,
    child: bool,
    skin: Vec<u8>,
}

impl Wearer {
    fn read(world: &World, guid: u32, profile: Option<&sims_content::Profile>) -> Self {
        let row = |idx| outfits::read_character_row(world, guid, profile, idx);
        let gender = outfits::profile_gender_code(row(12).as_deref());
        let age = row(13).unwrap_or_default();
        let child = outfits::atoi_age(outfits::truncate_cp1252(&age, 15)) <= 17;
        let skin: String = row(14)
            .unwrap_or_else(|| "???".to_owned())
            .chars()
            .take(3)
            .collect();
        Self {
            gender,
            child,
            skin: outfits::encode_cp1252(&skin.to_ascii_lowercase()),
        }
    }

    fn human(&self) -> bool {
        (0..=1).contains(&self.gender)
    }

    fn pet(&self) -> bool {
        (2..=5).contains(&self.gender)
    }
}

/// The entries of `list` the wearer can wear, in the suit scan's order
/// (0x409255) and then the texture list's (0x40895a), or `None` where the
/// native list pointer is null (0x408bf5 for an unknown gender, or a
/// pet's heads).
///
/// - A human body or clothing entry is a suit whose first nine
///   characters are the letter (`b` for bodies), any three, the sex or
///   `u`, the age letter and any three, with each texture that starts
///   with those nine characters and the skin tone (0x409663, 0x4098f1).
///   The filter is the one `outfits::commit_character_clothing` builds
///   its lists by.
/// - A head is `c`, any three, the sex or `u` and the age letter, but
///   not a suit whose second character is `t` (0x40a364, 0x40a453).
/// - A pet body is `b`, any three and `dog` (genders 2 and 3, list
///   0x6e488c) or `cat` (4 and 5, 0x6e4888), with each texture that
///   starts with those seven characters (0x409663, no skin tone).
fn entries(
    program: &Program,
    world: &mut World,
    wearer: &Wearer,
    list: List,
) -> Option<Vec<Entry>> {
    let (first, length, tag, species): (u8, usize, &str, Option<&[u8]>) = match list {
        List::Body if wearer.human() => (b'b', 9, "BODY", None),
        List::Body if (2..=3).contains(&wearer.gender) => (b'b', 7, "BODY", Some(b"dog")),
        List::Body if (4..=5).contains(&wearer.gender) => (b'b', 7, "BODY", Some(b"cat")),
        List::Head if wearer.human() => (b'c', 6, "HEAD-HEAD", None),
        List::Clothing(letter) if wearer.human() => (letter, 9, "BODY", None),
        _ => return None,
    };
    let sex = if wearer.gender == 1 { b'f' } else { b'm' };
    let age = if wearer.child { b'c' } else { b'a' };
    let scan = outfits::suit_scan_order(program, world);
    let textures = outfits::texture_list(program);
    let mut out = Vec::new();
    for suit in &scan {
        let bytes = outfits::encode_cp1252(suit);
        if bytes.len() < length || !bytes[0].eq_ignore_ascii_case(&first) {
            continue;
        }
        let fits = match species {
            Some(kind) => bytes[4..7].eq_ignore_ascii_case(kind),
            None => {
                (bytes[4].eq_ignore_ascii_case(&sex) || bytes[4].eq_ignore_ascii_case(&b'u'))
                    && bytes[5].eq_ignore_ascii_case(&age)
                    && !(list == List::Head && bytes[1].eq_ignore_ascii_case(&b't'))
            }
        };
        if !fits {
            continue;
        }
        let suit_63 = outfits::truncate_cp1252(suit, 63).to_owned();
        let mut prefix = bytes[..length].to_ascii_lowercase();
        if species.is_none() {
            prefix.extend_from_slice(&wearer.skin);
        }
        for texture in &textures {
            let name = outfits::encode_cp1252(texture);
            if name.len() >= prefix.len() && name[..prefix.len()].eq_ignore_ascii_case(&prefix) {
                let descriptor =
                    outfits::truncate_cp1252(&format!("{tag}={texture}"), 63).to_owned();
                let row = format!("{suit_63},{descriptor}");
                out.push(Entry {
                    suit: suit_63.clone(),
                    descriptor,
                    row,
                });
            }
        }
    }
    Some(out)
}

/// The place of `raw` in `list` (0x472ec4 → 0x5874ca → 0x472f92): the row
/// kept to 255 bytes, cut at any `;`, split at its first comma, and each
/// half kept to 63 bytes and compared ignoring ASCII case. An empty or
/// missing row, or one without a comma, is in no place.
fn place(list: &[Entry], raw: Option<&str>) -> Option<usize> {
    let raw = raw.filter(|r| !r.is_empty())?;
    let row = outfits::truncate_cp1252(raw, 255);
    let half = row.split(';').next().unwrap_or(row);
    let (suit, descriptor) = half.split_once(',')?;
    let suit = outfits::truncate_cp1252(suit, 63);
    let descriptor = outfits::truncate_cp1252(descriptor, 63);
    list.iter().position(|entry| {
        entry.suit.eq_ignore_ascii_case(suit) && entry.descriptor.eq_ignore_ascii_case(descriptor)
    })
}

/// The list and row of category `temp 0`, or `None` for another number.
fn category(number: i16) -> Option<(List, usize)> {
    Some(match number {
        0 => (List::Body, 1),
        1 => (List::Clothing(b'f'), 30),
        2 => (List::Clothing(b's'), 31),
        3 => (List::Clothing(b'l'), 32),
        4 => (List::Clothing(b'w'), 33),
        5 => (List::Clothing(b'h'), 34),
        _ => return None,
    })
}

fn write_row(world: &mut World, guid: u32, row: usize, value: String) {
    world
        .made_outfits
        .entry(guid)
        .or_default()
        .insert(row, value);
}

/// 0x40a588, the commit's clothing check, for the person of `guid`.
///
/// - **A pet** (genders 2 to 5, 0x40a5d5..0x40a619) gets `NONE` in rows
///   2 and 30 to 34. If its body row is in no place of its list, the
///   head count is 0 (0x408bf5(1) is null for a pet), so rows 1, 2 and
///   15 to 18 are emptied (0x40ac57). The rest is presentation
///   (0x4eacca at 0x40acb5 and 0x40af46).
/// - **A human** has its body (0x40d0a4) and head (0x40d408) looked up.
///   When either is in no place (0x40ab7f):
///   - with no bodies or no heads (0x40ab91, 0x40aba0), rows 1, 2 and 15
///     to 18 are emptied (0x40ac57);
///   - otherwise a missing head becomes the first head (0x40abce) and a
///     missing body the first body (0x40ac34), which drops the clothing
///     indices (0x40abf9).
///
///   A matched body is compared with the clothing rows before any of
///   that (0x40a6b4), so it is the row as it stood. Then
///   [`outfits::commit_character_clothing`] checks rows 30 to 34 and
///   rebuilds rows 15, 16, 27 and the hands (0x40acbc..0x40af51).
fn commit(
    program: &Program,
    world: &mut World,
    guid: u32,
    profile: Option<&sims_content::Profile>,
    wearer: &Wearer,
) {
    const EMPTIED: [usize; 6] = [1, 2, 17, 18, 15, 16];
    let row = |world: &World, idx| outfits::read_character_row(world, guid, profile, idx);
    let bodies = entries(program, world, wearer, List::Body).unwrap_or_default();
    if wearer.pet() {
        for idx in [2, 30, 31, 32, 33, 34] {
            write_row(world, guid, idx, "NONE".to_owned());
        }
        if place(&bodies, row(world, 1).as_deref()).is_none() {
            for idx in EMPTIED {
                write_row(world, guid, idx, String::new());
            }
        }
        return;
    }
    let body_row = row(world, 1);
    let body = place(&bodies, body_row.as_deref());
    let heads = entries(program, world, wearer, List::Head).unwrap_or_default();
    let head = place(&heads, row(world, 2).as_deref());
    let mut state = BodyCommitState::Matched;
    let mut matched_body_row = None;
    if body.is_none() || head.is_none() {
        if heads.is_empty() || bodies.is_empty() {
            for idx in EMPTIED {
                write_row(world, guid, idx, String::new());
            }
            if body.is_none() {
                state = BodyCommitState::ClearedEmptyList;
            } else {
                matched_body_row = body_row;
            }
        } else {
            if head.is_none() {
                write_row(world, guid, 2, heads[0].row.clone());
            }
            if body.is_none() {
                write_row(world, guid, 1, bodies[0].row.clone());
                state = BodyCommitState::ReplacedWithFallback;
            }
        }
    }
    outfits::commit_character_clothing(
        program,
        world,
        guid,
        profile,
        state,
        matched_body_row.as_deref(),
    );
}

impl Machine<'_, '_> {
    /// Routine 24 (0x44ff6e). See the module notes.
    pub(super) fn change_outfit(&mut self) -> Result<(), Trap> {
        let me = self.me()?;
        if me.person.is_none() {
            return Err(Trap::native(0x1c, "object is not a person"));
        }
        let guid = me.guid;
        let (number, index) = match me.temps[..] {
            [number, index, ..] => (number, index),
            _ => {
                return Err(Trap::Invalid(format!(
                    "object has {} temps, need at least 2",
                    me.temps.len()
                )));
            }
        };
        let profile = crate::people::profile(&self.program.people, self.world, guid)
            .ok()
            .map(std::borrow::Cow::into_owned);
        let profile = profile.as_ref();
        let wearer = Wearer::read(self.world, guid, profile);
        if !wearer.human() && !wearer.pet() {
            // 0x40a5a1 → 0x40d0a4 reads the body list 0x408bf5 left null.
            return Err(Trap::Unsupported(format!(
                "Generic Sims Call 24 for a person of gender code {}: the commit reads a null \
                 body list (0x40d0a4)",
                wearer.gender
            )));
        }
        let picked = if index >= 0 { category(number) } else { None };
        if wearer.pet()
            && let Some((List::Clothing(_), _)) = picked
        {
            return Err(Trap::Unsupported(format!(
                "Generic Sims Call 24 category {number} for a pet: its list (0x408d8e and kin) \
                 is not ported"
            )));
        }
        // 0x44ffcb: the editor's load.
        outfits::init_clothing_rows(self.program, self.world, guid, profile);
        let mut kept = None;
        if let Some((list, row)) = picked {
            let list = entries(self.program, self.world, &wearer, list).unwrap_or_default();
            if let Some(entry) = usize::try_from(index).ok().and_then(|i| list.get(i)) {
                let now = outfits::read_character_row(self.world, guid, profile, row);
                kept = Some(place(&list, now.as_deref()).map_or(-1, |p| p as i16));
                write_row(self.world, guid, row, entry.row.clone());
            }
        }
        // 0x450154: 0x409492(1) → 0x40a588.
        commit(self.program, self.world, guid, profile, &wearer);
        // 0x4501db: temp 1 becomes the kept index. When nothing was
        // written the native routine stores the low word of its first
        // argument (the interpreter frame's address) instead, which is
        // not reproducible; temp 1 is left as it was.
        if let Some(kept) = kept {
            self.me_mut()?.temps[1] = kept;
        }
        Ok(())
    }
}
