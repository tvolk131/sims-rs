//! The clothes a person's suit puts them in: what the original's outfit
//! choice (0x58a178) picks when the simulation changes a person's suit
//! (person data 8, set by the Change Suit primitive), such as Mortimer
//! Goth's work clothes after he dresses for his job.
//!
//! The choice reads the person type's body strings ([`Profile`], counted
//! from 1 as the original's entries are, so entry 2 is row 1) and, for a
//! career or pajamas, matches a name template against every appearance
//! and texture the install has ([`Pattern`], [`Wardrobe`]). By suit
//! (the jump table at 0x58d79b):
//!
//! - 0, everyday clothes: entry 2. The record's own clothes are kept, as
//!   they were before the suit was followed; they are what the original
//!   last chose for the person, and for the people tried they are entry 2.
//! - 1: entry 16. 17: entry 34. 18: entry 35.
//! - 2: entry 32, or entry 17 when 32 is empty.
//! - 3, work clothes: the person's career level (0x45ee5f, [`career`]).
//! - 4: for a child (entry 14, the age, read as a number, under 18) entry
//!   28; otherwise entry 31, or when that is empty entry 28 with entries
//!   29 and 30 for the left and right hands.
//! - 5, pajamas: entry 33, or when that is empty the matched pajamas
//!   (0x45f034, [`pajamas`]).
//! - Any other suit, 6 to 60, is a costume with its own branch, not
//!   ported: the record's clothes are kept.
//!
//! When a suit other than 0 leaves the body without a name, entry 2 is
//! worn (0x58d492). A person whose species bits (person data 65, bits 1
//! and 2) are set wears suit 0 whatever the word says (the old
//! prototype's reading, legacy/crates/sims-sim/src/pose/appearance_selection.rs).
//!
//! Not ported: the hands the hand words (person data 22 and 23) pick
//! (the record's hands are kept, except for suit 4's), the profile
//! migrations the original runs when it binds a profile (0x409315), and
//! the cache 0x45ee5f keeps by career and level, which can keep a
//! person's old work clothes when only their age or sex changes.

use crate::appearance::{Outfit, appearance_names, body_string};
use crate::assets::Assets;
use sims_formats::career::Level;
use sims_formats::saved_objects::Person;
use std::collections::BTreeMap;

/// The person data words the choice reads.
pub const SUIT: usize = 8;
pub const JOB: usize = 56;
pub const JOB_LEVEL: usize = 57;
pub const AGE: usize = 58;
pub const GENDER: usize = 65;

/// An appearance's name and its texture string (`KEY=texture`).
pub type Worn = (String, String);

/// The careers of `work.iff`, by CARR chunk ID, each with its levels.
pub type Careers = BTreeMap<i16, Vec<Level>>;

/// A person's clothes as the choice sees them: the body, an extra body
/// part a career can add (bound with no texture string), and the hands.
/// Each is an appearance name and its texture string (`KEY=texture`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Clothes {
    pub body: (String, String),
    pub extra: String,
    pub hands: [(String, String); 2],
}

/// The names a template is matched against: every appearance the
/// install's catalogs give, and every texture (a `.bmp` file's name
/// without its extension), each in lowercase and in name order.
///
/// The original keeps them in the order it loaded them (the appearance
/// registry 0x41570c, read through 0x409255; the texture names 0x40895a
/// gathers, BMP then TGA). Name order is an approximation that only
/// matters when two names match a template equally well; none do for the
/// careers tried. TGA textures are not listed: the person files sims-people
/// finds include none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wardrobe {
    pub appearances: Vec<String>,
    pub textures: Vec<String>,
}

impl Wardrobe {
    /// The names `assets` give.
    pub fn of(assets: &Assets) -> Self {
        Self {
            appearances: appearance_names(assets).map(str::to_owned).collect(),
            textures: assets
                .names()
                .filter_map(|n| n.strip_suffix(".bmp"))
                .map(str::to_owned)
                .collect(),
        }
    }
}

/// A person type's body strings, read as the original reads its profile
/// entries.
#[derive(Debug, Clone, Copy)]
pub struct Profile<'a> {
    pub rows: &'a [String],
}

impl Profile<'_> {
    /// Entry `entry`, counted from 1. An entry the strings do not hold
    /// reads as empty.
    pub fn raw(&self, entry: usize) -> &str {
        entry
            .checked_sub(1)
            .and_then(|row| self.rows.get(row))
            .map_or("", String::as_str)
    }

    /// Entry `entry` as an appearance and its texture string (0x5874ca):
    /// the part before any `;`, split at its first `,`; the name in
    /// lowercase, and the texture's value (after its `=`) in lowercase.
    pub fn read(&self, entry: usize) -> (String, String) {
        let text = self.raw(entry);
        let first = text.split_once(';').map_or(text, |(a, _)| a);
        let (name, texture) = first.split_once(',').unwrap_or((first, ""));
        let texture = match texture.split_once('=') {
            Some((key, value)) => format!("{key}={}", value.to_ascii_lowercase()),
            None => texture.to_ascii_lowercase(),
        };
        (name.to_ascii_lowercase(), texture)
    }

    /// The sex a template's `$g` gives (0x40e477, entry 13): 0 for
    /// `male`, 1 for `female`, more for the pets' (2 to 4) and 5 for
    /// anything else, compared without case.
    fn sex(&self) -> i32 {
        let value = fixed(self.raw(13).as_bytes(), 15);
        [
            b"male".as_slice(),
            b"female",
            b"dogmale",
            b"dogfemale",
            b"catmale",
        ]
        .iter()
        .position(|name| value.eq_ignore_ascii_case(name))
        .map_or(5, |n| n as i32)
    }

    /// The body type a template's `$b` gives: up to three bytes from the
    /// seventh of entry 2's appearance name (`fit` of `b004mafit_01`), in
    /// lowercase. None when the name is shorter than six bytes.
    fn body_type(&self) -> Option<Vec<u8>> {
        let value = fixed(self.raw(2).as_bytes(), 255);
        let end = value
            .iter()
            .position(|&b| b == b',' || b == b';')
            .unwrap_or(value.len())
            .min(63);
        let name = &value[..end];
        (name.len() >= 6).then(|| name[6..name.len().min(9)].to_ascii_lowercase())
    }

    /// The skin tone a template's `$c` gives: entry 15's first three
    /// bytes, or `???` when the entry is absent.
    fn tone(&self) -> Vec<u8> {
        let entry = 15;
        match self.rows.get(entry - 1) {
            Some(value) => fixed(value.as_bytes(), 3).to_vec(),
            None => b"???".to_vec(),
        }
    }
}

/// The C string a fixed buffer of `maximum` bytes would hold.
fn fixed(bytes: &[u8], maximum: usize) -> &[u8] {
    let end = bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(bytes.len())
        .min(maximum);
    &bytes[..end]
}

/// A name template expanded for one person (0x40e477): a run of groups,
/// each a choice of texts, a name matching it when it starts with one
/// text of each group in turn, compared without case (0x4727f1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pattern {
    groups: Vec<Vec<Vec<u8>>>,
}

impl Pattern {
    /// Expands `template`: its text as it stands, except that `$g` is the
    /// sex's letter or `u` (`f` for a female, `m` otherwise), `$b` the
    /// body type and `$c` the skin tone. Any other `$` and the letter
    /// after it add nothing. None when the template has a `$b` and the
    /// body strings give no body type.
    pub fn expand(template: &[u8], profile: &Profile) -> Option<Self> {
        let mut pattern = Self::default();
        let mut index = 0;
        while index < template.len() {
            let end = template[index..]
                .iter()
                .position(|&b| b == b'$')
                .map_or(template.len(), |n| index + n);
            if index < end {
                // The original copies each run through 256 bytes.
                pattern.literal(&template[index..end.min(index + 255)]);
                index = end;
            } else {
                index += 1;
                if let Some(token) = template.get(index) {
                    match token.to_ascii_lowercase() {
                        b'g' => {
                            let sex: &[u8] = if profile.sex() == 1 { b"f" } else { b"m" };
                            pattern.either(sex, b"u");
                        }
                        b'b' => pattern.literal(&profile.body_type()?),
                        b'c' => pattern.literal(&profile.tone()),
                        _ => {}
                    }
                }
                index += 1;
            }
        }
        Some(pattern)
    }

    pub fn literal(&mut self, text: &[u8]) {
        self.groups.push(vec![text.to_vec()]);
    }

    pub fn either(&mut self, first: &[u8], second: &[u8]) {
        self.groups.push(vec![first.to_vec(), second.to_vec()]);
    }

    /// How many bytes of `candidate` the longest way through the pattern
    /// matches, or 0 when no way matches. The original walks the pattern
    /// as linked nodes from the first; a choice that starts the pattern
    /// is entered at its first text only.
    pub fn matched_prefix(&self, candidate: &[u8]) -> usize {
        fn walk(groups: &[Vec<Vec<u8>>], candidate: &[u8], offset: usize, first: bool) -> usize {
            let Some((group, rest)) = groups.split_first() else {
                return offset;
            };
            let choices = if first { &group[..1] } else { &group[..] };
            choices
                .iter()
                .filter(|text| {
                    candidate
                        .get(offset..offset + text.len())
                        .is_some_and(|v| v.eq_ignore_ascii_case(text))
                })
                .map(|text| walk(rest, candidate, offset + text.len(), false))
                .max()
                .unwrap_or(0)
        }
        if self.groups.is_empty() {
            return 0;
        }
        walk(&self.groups, candidate, 0, true)
    }
}

/// The best-ranked candidate so far (0x472a25): the one the pattern
/// matches the largest fraction of, an equal fraction (within 0.0001)
/// replacing it only with more bytes matched.
#[derive(Debug, Clone, Default)]
struct Best {
    name: Vec<u8>,
    matched: usize,
    fraction: f32,
}

impl Best {
    fn consider(&mut self, pattern: &Pattern, candidate: &[u8]) {
        let matched = pattern.matched_prefix(candidate);
        if matched == 0 {
            return;
        }
        // The original stores the count as a float, divides on the x87,
        // and compares the quotient unrounded with the stored float.
        let fraction = f64::from(matched as f32) / candidate.len() as f64;
        let old = f64::from(self.fraction);
        if ((fraction - old).abs() <= f64::from(0.0001_f32) && self.matched < matched)
            || old < fraction
        {
            self.fraction = fraction as f32;
            self.matched = matched;
            self.name = candidate[..candidate.len().min(255)].to_vec();
        }
    }
}

/// The body and texture two templates pick (0x40e5ce, 0x40e674). The
/// body is the first appearance the body template matches; empty when
/// none does. The texture template gets one more choice at its end, the
/// rest of entry 2's texture after its first 17 bytes (`_rat` of
/// `BODY=b004mafitlgt_rat`, the skipped `BODY=` and 12 more,
/// 0x40b75a) or nothing, and the best-ranked texture is taken, written
/// `BODY=<name>`; empty when none matches. None when a template cannot
/// be expanded, or entry 2's texture is shorter than 17 bytes.
pub fn select(
    body: &[u8],
    texture: &[u8],
    profile: &Profile,
    wardrobe: &Wardrobe,
) -> Option<(String, String)> {
    let pattern = Pattern::expand(body, profile)?;
    let body = wardrobe
        .appearances
        .iter()
        .map(|n| fixed(n.as_bytes(), 63))
        .find(|n| pattern.matched_prefix(n) != 0)
        .unwrap_or_default();
    let mut pattern = Pattern::expand(texture, profile)?;
    let normal = fixed(profile.raw(2).as_bytes(), 255);
    let primary = normal.split(|&b| b == b';').next().unwrap_or_default();
    let raw = primary
        .iter()
        .position(|&b| b == b',')
        .map_or(&b""[..], |n| &primary[n + 1..]);
    let mut normal_texture = fixed(raw, 63).to_vec();
    let value = normal_texture
        .iter()
        .position(|&b| b == b'=')
        .map_or(0, |n| n + 1);
    normal_texture[value..].make_ascii_lowercase();
    pattern.either(normal_texture.get(17..)?, b"");
    let mut best = Best::default();
    for name in &wardrobe.textures {
        best.consider(&pattern, fixed(name.as_bytes(), 63));
    }
    let texture = if best.name.is_empty() {
        String::new()
    } else {
        format!("BODY={}", String::from_utf8_lossy(&best.name))
    };
    Some((String::from_utf8_lossy(body).into_owned(), texture))
}

/// The work clothes of a person's career level (0x45ee5f): career `job`
/// (person data 56), level `level` (data 57, level 0 for a child, an age
/// from 1 to 17), the level's female body when `female` and it has one,
/// its male body otherwise, matched with its texture ([`select`]); and
/// the level's extra part as it stands. Ok(None) when the career or level
/// does not exist; Err when the templates cannot be matched.
pub fn career(
    careers: &Careers,
    job: i16,
    level: i16,
    age: i16,
    female: bool,
    profile: &Profile,
    wardrobe: &Wardrobe,
) -> Result<Option<(Worn, String)>, String> {
    let Some(levels) = careers.get(&job) else {
        return Ok(None);
    };
    let level = if age != 0 && age < 18 { 0 } else { level };
    let Some(level) = usize::try_from(level).ok().and_then(|l| levels.get(l)) else {
        return Ok(None);
    };
    let [_, male, woman, texture, extra] = &level.strings;
    let body = if female && !woman.is_empty() {
        woman
    } else {
        male
    };
    let chosen = select(body.as_bytes(), texture.as_bytes(), profile, wardrobe)
        .ok_or("the career's templates cannot be matched for this person")?;
    Ok(Some((chosen, extra.clone())))
}

/// The pajamas a person wears when their body strings name none
/// (0x45f034): `pjs$gchd` for a child (`age` from 1 to 17), `pajama$g$b`
/// otherwise, with the texture `pajama$g$c` ([`select`]).
pub fn pajamas(age: i16, profile: &Profile, wardrobe: &Wardrobe) -> Option<(String, String)> {
    let body: &[u8] = if age != 0 && age < 18 {
        b"pjs$gchd"
    } else {
        b"pajama$g$b"
    };
    select(body, b"pajama$g$c", profile, wardrobe)
}

/// The leading decimal number of `text`, as C's `atoi` reads it.
fn atoi(text: &str) -> i64 {
    let text = text.trim_start();
    let (sign, digits) = match text.as_bytes().first() {
        Some(b'-') => (-1, &text[1..]),
        Some(b'+') => (1, &text[1..]),
        _ => (1, text),
    };
    let value = digits
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0i64, |n, d| {
            n.saturating_mul(10).saturating_add(i64::from(d - b'0'))
        });
    sign * value
}

/// What a person with the data words `data` wears for their suit.
/// `worn` is the record's clothes, kept for suit 0 and the costumes. Err
/// when a career or pajamas cannot be matched (see [`select`]); the
/// caller keeps the record's clothes.
pub fn dress(
    worn: &Clothes,
    data: &[i16],
    rows: &[String],
    careers: Option<&Careers>,
    wardrobe: &Wardrobe,
) -> Result<Clothes, String> {
    let word = |n: usize| data.get(n).copied().unwrap_or(0);
    let suit = if word(GENDER) & 6 != 0 { 0 } else { word(SUIT) };
    let profile = Profile { rows };
    let mut clothes = worn.clone();
    let entry = match suit {
        0 => return Ok(clothes),
        1 => Some(16),
        2 => Some(if profile.raw(32).is_empty() { 17 } else { 32 }),
        3 => {
            let careers =
                careers.ok_or("the careers are not read (the install has a Careers.txt)")?;
            let female = word(GENDER) & 1 != 0;
            let (job, level, age) = (word(JOB), word(JOB_LEVEL), word(AGE));
            match career(careers, job, level, age, female, &profile, wardrobe)? {
                Some((body, extra)) => {
                    clothes.body = body;
                    clothes.extra = extra;
                }
                // 0x45ee5f leaves its strings empty.
                None => clothes.body = Default::default(),
            }
            None
        }
        4 => {
            if atoi(profile.raw(14)) < 18 {
                Some(28)
            } else if profile.raw(31).is_empty() {
                clothes.body = profile.read(28);
                let hands = [profile.read(29), profile.read(30)];
                // The hands are bound only when both are named.
                if hands.iter().all(|(name, _)| !name.is_empty()) {
                    clothes.hands = hands;
                }
                None
            } else {
                Some(31)
            }
        }
        5 => {
            if profile.raw(33).is_empty() {
                clothes.body = pajamas(word(AGE), &profile, wardrobe)
                    .ok_or("the pajamas' templates cannot be matched for this person")?;
                None
            } else {
                Some(33)
            }
        }
        17 => Some(34),
        18 => Some(35),
        _ => return Ok(clothes),
    };
    if let Some(entry) = entry {
        clothes.body = profile.read(entry);
    }
    if clothes.body.0.is_empty() {
        clothes.body = profile.read(2);
    }
    Ok(clothes)
}

/// Whether `person`'s record names no body, as
/// [`crate::house::Cast::residents`] tells (it then draws the type's
/// default outfit).
fn unchosen(person: &Person) -> bool {
    person.appearance[0].trim().is_empty()
}

impl Clothes {
    /// The record's clothes: its body (appearance slots 0 and 1), left
    /// hand (4, 5) and right hand (6, 7).
    ///
    /// A record that names no body (a person a script made, whose clothes
    /// were never chosen) gives instead the type's default outfit from its
    /// body strings `rows`: the body (row 1), left hand (row 17) and right
    /// hand (row 18), as [`Outfit::from_body_strings`] reads them.
    pub fn of(person: &Person, rows: &[String]) -> Self {
        let a = &person.appearance;
        if unchosen(person) {
            let row = |n: usize| rows.get(n).map(|s| body_string(s)).unwrap_or_default();
            return Self {
                body: row(1),
                extra: String::new(),
                hands: [row(17), row(18)],
            };
        }
        Self {
            body: (a[0].clone(), a[1].clone()),
            extra: String::new(),
            hands: [(a[4].clone(), a[5].clone()), (a[6].clone(), a[7].clone())],
        }
    }

    /// The outfit `person` wears with its body and hands replaced by
    /// these: the body, the extra part, the hands, then the head and
    /// accessories of the record ([`Outfit::from_person`]), or for a record
    /// that names no body the head of the type's body strings `rows` (row
    /// 2, as [`Outfit::from_body_strings`] gives it).
    pub fn wear(&self, person: &Person, rows: &[String]) -> Outfit {
        let rest: Vec<Worn> = if unchosen(person) {
            rows.get(2).map(|s| body_string(s)).into_iter().collect()
        } else {
            let a = &person.appearance;
            // Every part after the record's body and hands, whichever of
            // those it names.
            let skip = [&a[0], &a[4], &a[6]]
                .iter()
                .filter(|n| !n.is_empty())
                .count();
            Outfit::from_person(person)
                .parts
                .into_iter()
                .skip(skip)
                .collect()
        };
        let mut parts = vec![self.body.clone()];
        if !self.extra.is_empty() {
            parts.push((self.extra.clone(), String::new()));
        }
        parts.extend(self.hands.iter().cloned());
        parts.extend(rest);
        parts.retain(|(name, _)| !name.is_empty());
        Outfit { parts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<String> {
        let mut rows = vec![String::new(); 30];
        rows[0] = "adult".into();
        rows[1] = "b004mafit_01,BODY=B004MAFitlgt_rat".into();
        rows[12] = "male".into();
        rows[13] = "27".into();
        rows[14] = "lgt".into();
        rows[15] = "nmfit_01,BODY=nmfitlgt_01".into();
        rows[27] = "fmfit_01,BODY=fmfitlgt_01".into();
        rows
    }

    fn wardrobe() -> Wardrobe {
        Wardrobe {
            appearances: ["scienceffit_01", "sciencemfat_01", "sciencemfit_01"]
                .map(String::from)
                .to_vec(),
            textures: [
                "sciencefmed_01",
                "sciencemdrk_01",
                "sciencemlgt_01",
                "sciencemlgt_01x",
            ]
            .map(String::from)
            .to_vec(),
        }
    }

    fn level(male: &str, female: &str, texture: &str) -> Level {
        Level {
            values: [0; 21],
            strings: [
                "Test Subject".into(),
                male.into(),
                female.into(),
                texture.into(),
                String::new(),
            ],
        }
    }

    #[test]
    fn a_template_fills_in_the_sex_body_type_and_tone() {
        let rows = rows();
        let profile = Profile { rows: &rows };
        let p = Pattern::expand(b"Science$g$b_01", &profile).unwrap();
        assert_eq!(p.matched_prefix(b"sciencemfit_01"), 14);
        assert_eq!(p.matched_prefix(b"scienceufit_01_more"), 14);
        assert_eq!(p.matched_prefix(b"scienceffit_01"), 0);
        // A choice that starts a pattern is entered at its first text.
        let p = Pattern::expand(b"$gx", &profile).unwrap();
        assert_eq!(p.matched_prefix(b"mx"), 2);
        assert_eq!(p.matched_prefix(b"ux"), 0);
    }

    #[test]
    fn a_career_level_dresses_a_person_in_its_matched_clothes() {
        let rows = rows();
        let profile = Profile { rows: &rows };
        let careers = Careers::from([(
            9,
            vec![level("Science$g$b_01", "Science$g$b_01", "Science$g$c_01")],
        )]);
        let (body, extra) = career(&careers, 9, 0, 27, false, &profile, &wardrobe())
            .unwrap()
            .unwrap();
        // The longer texture matches a smaller fraction of its name.
        assert_eq!(
            body,
            ("sciencemfit_01".into(), "BODY=sciencemlgt_01".into())
        );
        assert!(extra.is_empty());
        // A level that does not exist gives nothing; a child is at level 0.
        let at = |job, level, age| career(&careers, job, level, age, false, &profile, &wardrobe());
        assert_eq!(at(9, 1, 27), Ok(None));
        assert!(at(9, 1, 9).unwrap().is_some());
        assert_eq!(at(8, 0, 27), Ok(None));
    }

    #[test]
    fn the_normal_textures_ending_is_preferred_when_a_texture_has_it() {
        let rows = rows();
        let profile = Profile { rows: &rows };
        let mut w = wardrobe();
        w.textures.push("sciencemlgt_01_rat".into());
        let (_, texture) = select(b"Science$g$b_01", b"Science$g$c_01", &profile, &w).unwrap();
        assert_eq!(texture, "BODY=sciencemlgt_01_rat");
    }

    #[test]
    fn a_suit_picks_its_body_strings_entry_and_falls_back_to_entry_2() {
        let rows = rows();
        let worn = Clothes {
            body: ("b004mafit_01".into(), "BODY=b004mafitlgt_rat".into()),
            ..Clothes::default()
        };
        let w = wardrobe();
        let wear = |suit: i16, gender: i16| {
            let mut data = vec![0; 80];
            data[SUIT] = suit;
            data[GENDER] = gender;
            dress(&worn, &data, &rows, Some(&Careers::new()), &w).unwrap()
        };
        assert_eq!(wear(0, 0), worn);
        assert_eq!(
            wear(1, 0).body,
            ("nmfit_01".into(), "BODY=nmfitlgt_01".into())
        );
        assert_eq!(wear(4, 0).body.0, "fmfit_01");
        // Entry 17 is empty here, as is the career: entry 2 is worn.
        assert_eq!(wear(2, 0).body.0, "b004mafit_01");
        assert_eq!(wear(3, 0).body, worn.body);
        // A costume keeps the record's clothes, as does a pet.
        assert_eq!(wear(40, 0), worn);
        assert_eq!(wear(1, 2), worn);
    }
}
