//! What the control panel shows, read from the simulation, and the
//! portraits and first names of the household's people.

use crate::art::Art;
use crate::{Error, Result};
use sims_content::{Catalog, Index};
use sims_formats::bitmap::{self, Image};
use sims_formats::career::{Career, Level};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_sim::World;
use std::collections::BTreeMap;

/// Globals the clock reads (`crates/sims-vm/src/tick.rs`).
const HOURS: usize = 0;
const MINUTES: usize = 5;
/// The global that pauses the lot (`crates/sims-vm/src/tick.rs`).
const PAUSED: usize = 17;

/// A person of the household, as the panel shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct Resident {
    /// The person's object ID.
    pub object: u16,
    pub guid: u32,
    pub motives: [f32; 16],
    /// The personality in the panel's order ([`PERSONALITY`]), in
    /// hundredths of a point (0 to 1000).
    pub personality: [i16; 5],
    /// The person's sign of the zodiac, 1 (Aries) to 12 (Pisces), as
    /// the personality panel shows it ([`zodiac`]); none for a person
    /// the panel shows no sign for.
    pub zodiac: Option<u8>,
    /// The skills in the job panel's order ([`SKILLS`]), in hundredths
    /// of a point (0 to 1000).
    pub skills: [i16; 6],
    /// The career (a `CARR` resource's ID; 0 for none) and the level in
    /// it, person data 56 and 57.
    pub job: [i16; 2],
    /// What the next level of the career needs of each skill, where the
    /// person has less: 0 elsewhere, and everywhere until
    /// [`Snapshot::with_careers`] fills it in.
    pub needs: [i32; 6],
}

/// The person data words of the personality, in the panel's order: Neat,
/// Outgoing, Active, Playful and Nice (0x66c600).
pub const PERSONALITY: [usize; 5] = [7, 6, 3, 5, 2];

/// Person data word 70: the person's sign of the zodiac, 1 to 12, or
/// 0 until the personality panel works it out.
pub const ZODIAC: usize = 70;

/// The person data words the sign is worked out from (0x6c5be4):
/// Outgoing, Nice, Playful, Neat and Active.
const ZODIAC_TRAITS: [usize; 5] = [6, 2, 5, 7, 3];

/// Each sign's traits, in [`ZODIAC_TRAITS`]' order, in points
/// (0x6c5bf8), Aries first.
const ZODIAC_PROFILES: [[f32; 5]; 12] = [
    [8.0, 3.0, 3.0, 5.0, 6.0],
    [5.0, 4.0, 8.0, 5.0, 3.0],
    [7.0, 3.0, 3.0, 4.0, 8.0],
    [3.0, 6.0, 4.0, 6.0, 6.0],
    [10.0, 3.0, 4.0, 4.0, 4.0],
    [2.0, 5.0, 3.0, 9.0, 6.0],
    [8.0, 7.0, 6.0, 2.0, 2.0],
    [5.0, 3.0, 3.0, 6.0, 8.0],
    [3.0, 4.0, 7.0, 2.0, 9.0],
    [4.0, 5.0, 8.0, 7.0, 1.0],
    [4.0, 6.0, 7.0, 4.0, 4.0],
    [3.0, 7.0, 3.0, 5.0, 7.0],
];

/// The sign the personality panel shows for a person whose data word
/// `n` is `data(n)` (0x51cd11). A person of a kind above 1 (person data
/// 65) gets none. Otherwise the saved sign, held to 1 to 12 as the
/// name lookup holds it (0x4842de); where none is saved (0), the sign
/// whose traits are nearest (0x4841fc).
///
/// The original saves the sign it works out into the person's data
/// (0x51cd44); the panel here only reads, so a host that wants the
/// original's data writes it.
pub fn zodiac(data: impl Fn(usize) -> i16) -> Option<u8> {
    if data(sims_sim::neighbors::KIND) > 1 {
        return None;
    }
    Some(match data(ZODIAC) {
        0 => nearest_sign(ZODIAC_TRAITS.map(&data)),
        // Held to 1..=12 just above, so it fits.
        sign => sign.clamp(1, 12) as u8,
    })
}

/// The sign whose traits are nearest `traits` (in hundredths of a point,
/// [`ZODIAC_TRAITS`]' order), as 0x4841fc measures: each trait times the
/// float 0.01, the squared distances summed, the least sum kept, the
/// first sign kept on a tie. The game sets the x87 to 24-bit precision
/// (0x4fad5c), so every step rounds as `f32` arithmetic does.
fn nearest_sign(traits: [i16; 5]) -> u8 {
    let scale = f32::from_bits(0x3c23_d70a);
    let points = traits.map(|t| f32::from(t) * scale);
    let mut best = f32::from_bits(0x7e96_7699);
    let mut sign = 1;
    for (n, profile) in ZODIAC_PROFILES.iter().enumerate() {
        let distance = points.iter().zip(profile).fold(0.0f32, |sum, (p, q)| {
            let d = p - q;
            d * d + sum
        });
        if distance < best {
            best = distance;
            sign = n as u8 + 1;
        }
    }
    sign
}

/// The person data words of the skills, in the job panel's order:
/// Cooking, Mechanical, Charisma, Body, Logic and Creativity (0x66dfec).
pub const SKILLS: [usize; 6] = [10, 12, 11, 17, 18, 15];

/// The person data words of the career and the level in it.
const JOB: [usize; 2] = [56, 57];

/// Which of a career level's values is the need of each of [`SKILLS`]
/// (0x4ea823).
const SKILL_NEEDS: [usize; 6] = [1, 2, 3, 4, 5, 6];

/// The simulation state the control panel shows, copied out of a
/// [`World`] so drawing never holds the world.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub hour: i16,
    pub minute: i16,
    pub funds: i32,
    pub paused: bool,
    /// The household's people on the lot, in the family's member order.
    pub residents: Vec<Resident>,
    /// The selected Sim's object ID.
    pub selected: Option<u16>,
    /// The people the selected Sim has a relationship with, in the
    /// relationship panel's order ([`Relation`]): none until
    /// [`Snapshot::with_relations`] reads them.
    pub relations: Vec<Relation>,
}

/// A person in the relationship panel: what 0x4ea042 reads of the
/// selected Sim's relationship row toward them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    /// The GUID of the person's type, for their faces.
    pub guid: u32,
    /// Their neighbor record's ID.
    pub id: i16,
    /// The row's value 0 (the daily relationship, the top bar) and value
    /// 2 (the lifetime one, the bottom bar), each -100 to 100.
    pub daily: i32,
    pub lifetime: i32,
    /// Whether both have a daily relationship of at least the
    /// friendship threshold toward each other: the smiley.
    pub friends: bool,
    /// Whether the row's value 1, and its value 3, are set: the pink
    /// heart is drawn for the first and the red one for the second.
    /// Between the married Goths value 1 is 1.
    pub pink_heart: bool,
    pub red_heart: bool,
    /// Whether they belong to the selected Sim's family (person data
    /// word 61, [`sims_sim::neighbors::FAMILY`]): the family button's
    /// list.
    pub family: bool,
}

impl Snapshot {
    /// Reads the snapshot. `members` is the family's member GUIDs in the
    /// order the neighborhood keeps them (`Family::members`): the world
    /// does not keep that list, and the portraits follow it. A member with
    /// no person object on the lot is left out.
    pub fn read(world: &World, members: &[u32]) -> Self {
        let global = |n: usize| world.globals.get(n).copied().unwrap_or(0);
        let mut residents = Vec::new();
        for &guid in members {
            // The first person of the type, as the original's lookup by
            // type does (crates/sims-vm/src/selection.rs).
            if let Some(object) = world
                .objects
                .values()
                .find(|o| o.guid == guid && o.person.is_some())
            {
                let person = object.person.as_ref();
                let data =
                    |word: usize| person.and_then(|p| p.data.get(word).copied()).unwrap_or(0);
                residents.push(Resident {
                    object: object.id,
                    guid,
                    motives: person.map(|p| p.motives).unwrap_or_default(),
                    personality: PERSONALITY.map(data),
                    zodiac: zodiac(data),
                    skills: SKILLS.map(data),
                    job: JOB.map(data),
                    needs: [0; 6],
                });
            }
        }
        Self {
            hour: global(HOURS),
            minute: global(MINUTES),
            funds: world.budget.funds,
            paused: global(PAUSED) != 0,
            residents,
            selected: world
                .objects
                .values()
                .find(|o| o.selected && o.person.is_some())
                .map(|o| o.id),
            relations: Vec::new(),
        }
    }

    /// Fills in each resident's [`Resident::needs`] from `careers`, as
    /// the original's 0x4ea823 works them out: the value of the level
    /// after the person's own, where it is more than the person's skill.
    /// A person with no career, or at its top level, needs nothing.
    #[must_use]
    pub fn with_careers(mut self, careers: &Careers) -> Self {
        for resident in &mut self.residents {
            let [career, level] = resident.job;
            let next = careers.level(career, i32::from(level) + 1);
            resident.needs = std::array::from_fn(|n| {
                let need = next
                    .and_then(|l| l.values.get(SKILL_NEEDS[n]).copied())
                    .unwrap_or(0);
                if need > i32::from(resident.skills[n]) {
                    need
                } else {
                    0
                }
            });
        }
        self
    }

    /// Reads the selected Sim's [`Snapshot::relations`] from `world`.
    /// `members` is as for [`Snapshot::read`]; `friendship` is the
    /// autonomy tuning's friendship threshold (0x6c4398, read from the
    /// tuning: 50 in the original's capture), which the smiley needs.
    #[must_use]
    pub fn with_relations(mut self, world: &World, members: &[u32], friendship: i32) -> Self {
        let selected = self.selected.and_then(|id| world.objects.get(&id));
        self.relations = selected
            .map(|o| relations(world, o.guid, members, friendship))
            .unwrap_or_default();
        self
    }

    pub fn resident(&self, object: u16) -> Option<&Resident> {
        self.residents.iter().find(|r| r.object == object)
    }

    /// The clock as the panel writes it: `AM 6:30`. Midnight and noon
    /// are 12. Checked against the original at 6:30 and 10:10 in the
    /// morning and 7:31 and 7:55 in the evening (`PM 7:31`); midnight
    /// and noon are **unverified**.
    pub fn clock(&self) -> String {
        let hour = self.hour.rem_euclid(24);
        let twelve = match hour % 12 {
            0 => 12,
            h => h,
        };
        let half = if hour < 12 { "AM" } else { "PM" };
        format!("{half} {twelve}:{:02}", self.minute.clamp(0, 59))
    }

    /// The funds as the panel writes them: `$2,542`. Checked against
    /// the original for that value; how it writes a debt is
    /// **unverified** (here `-$5`).
    pub fn money(&self) -> String {
        let digits = self.funds.unsigned_abs().to_string();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        format!("{}${out}", if self.funds < 0 { "-" } else { "" })
    }
}

/// The careers of `work.iff`, by the ID of their `CARR` resource, as
/// the original looks a person's career up (0x404cb3).
#[derive(Debug, Clone, Default)]
pub struct Careers {
    careers: BTreeMap<i16, Vec<Level>>,
}

impl Careers {
    /// Reads every `CARR` of the install's `work.iff`. An install without
    /// the file has no careers.
    pub fn load(index: &Index) -> Result<Self> {
        let mut careers = BTreeMap::new();
        let Some((_, file)) = index.named("work.iff").next() else {
            return Ok(Self { careers });
        };
        let iff = Iff::parse(file.read().map_err(Error::Content)?).map_err(Error::Format)?;
        for chunk in iff.resource_order(b"CARR").map_err(Error::Format)? {
            let career = Career::parse(iff.data(chunk)).map_err(Error::Format)?;
            careers.insert(chunk.id as i16, career.levels);
        }
        Ok(Self { careers })
    }

    /// Adds or replaces a career, as tests do.
    pub fn insert(&mut self, id: i16, levels: Vec<Level>) {
        self.careers.insert(id, levels);
    }

    /// Level `level` (from 0) of career `career`, if both exist.
    pub fn level(&self, career: i16, level: i32) -> Option<&Level> {
        if career == 0 {
            return None;
        }
        self.careers.get(&career)?.get(usize::try_from(level).ok()?)
    }
}

/// The people `guid`'s Sim has a relationship with, in the relationship
/// panel's order: 0x4e9deb lists the neighbor records, in slot order,
/// that the Sim's own record (the first of its type) has a row toward,
/// and 0x4e9fad sorts them: the household's members first, then people on
/// the lot, then by the daily relationship, highest first, then by
/// record.
///
/// **Not ported:** 0x4e9deb's test of each record (0x423f30, and words
/// of the person on the lot), so a record the original leaves out may be
/// listed; and the original breaks the last tie by its own list of
/// people (0x6f2698), not by record (**unverified** that they agree).
fn relations(world: &World, guid: u32, members: &[u32], friendship: i32) -> Vec<Relation> {
    let neighbors = &world.neighbors;
    let Some(viewer) = neighbors.first_of(guid) else {
        return Vec::new();
    };
    let family = viewer.data.get(sims_sim::neighbors::FAMILY).copied();
    let on_lot = |guid: u32| {
        world
            .objects
            .values()
            .any(|o| o.guid == guid && o.person.is_some())
    };
    let mut listed = Vec::new();
    for record in neighbors.slots.iter().flatten() {
        if record.id == viewer.id {
            continue;
        }
        // As the original (0x4e9e77..0x4e9f69), a record stands for its
        // type: its GUID gives a person (0x4e9753), and the row asked for
        // is toward that person's record, the first of the type
        // (0x4ea192). So a second record of a type is listed with the
        // first's row, or not at all where only it has a row; 0x4ea042
        // reads friendship between the first records too.
        let Some(target) = neighbors.first_of(record.guid) else {
            continue;
        };
        let Some(row) = viewer.row(i32::from(target.id)) else {
            continue;
        };
        let value = |n: usize| row.values.get(n).copied().unwrap_or(0);
        if row.values.is_empty() {
            continue;
        }
        let relation = Relation {
            guid: record.guid,
            id: record.id,
            daily: value(0).clamp(-100, 100),
            lifetime: value(2).clamp(-100, 100),
            friends: sims_sim::neighbors::mutual_friends(viewer, target, friendship),
            pink_heart: value(1) != 0,
            red_heart: value(3) != 0,
            family: family.is_some()
                && record.data.get(sims_sim::neighbors::FAMILY).copied() == family,
        };
        let order = (
            !members.contains(&record.guid),
            !on_lot(record.guid),
            std::cmp::Reverse(value(0)),
            record.id,
        );
        listed.push((order, relation));
    }
    listed.sort_by_key(|a| a.0);
    listed.into_iter().map(|(_, r)| r).collect()
}

/// The face of a person in their character file: `BMP_` 2002 holds three
/// faces side by side, and the panel shows the first.
pub const FACES: u16 = 2002;
/// The relationship panel's faces of a person: `BMP_` 2003 holds five
/// side by side, on red through green, for the lifetime relationship
/// (0x4e94a4 asks for 2003).
pub const MOODS: u16 = 2003;
/// The portrait frame the face is drawn on (kPeopleTemplate): four 45 by
/// 45 cells.
pub const TEMPLATE: i32 = 4600;
/// Where the face goes in a portrait cell, and its size: the old
/// prototype's layout (legacy/crates/sims-app/src/original_hud.rs
/// `Thumbnails`, `[45, 45, 5, 2, 35, 41]`).
const FACE: [u32; 4] = [5, 2, 35, 41];

/// The definition word that names a type's catalog strings (`CTSS`):
/// word 41 of its `OBJD`. A person's first name is the table's first
/// string; Mortimer's file has 2000 there, and `CTSS` 2000 starts with
/// "Mortimer".
const CATALOG_STRINGS: usize = 41;

/// The faces and first names of people, by the GUID of their type.
#[derive(Debug, Clone, Default)]
pub struct Faces {
    faces: BTreeMap<u32, Image>,
    moods: BTreeMap<u32, Image>,
    names: BTreeMap<u32, String>,
}

impl Faces {
    /// Reads the faces and the first name of each of `guids` from its
    /// character file. A type with no file, no faces or no name is left
    /// out of what it lacks; a face that does not decode is an error.
    pub fn load(index: &Index, catalog: &Catalog, guids: &[u32]) -> Result<Self> {
        let mut faces = BTreeMap::new();
        let mut moods = BTreeMap::new();
        let mut names = BTreeMap::new();
        for &guid in guids {
            let Some(object) = catalog.object(guid) else {
                continue;
            };
            let Some(resource) = index.resources.get(object.resource) else {
                continue;
            };
            let iff =
                Iff::parse(resource.read().map_err(Error::Content)?).map_err(Error::Format)?;
            if let Some(name) = first_name(&iff, object.chunk) {
                names.insert(guid, name);
            }
            if let Some(chunk) = iff.get(b"BMP_", MOODS) {
                moods.insert(guid, bitmap::bmp(iff.data(chunk)).map_err(Error::Format)?);
            }
            let Some(chunk) = iff.get(b"BMP_", FACES) else {
                continue;
            };
            let image = bitmap::bmp(iff.data(chunk)).map_err(Error::Format)?;
            if let Some(face) = image.crop(
                0,
                0,
                (image.width / 3).min(FACE[2]),
                image.height.min(FACE[3]),
            ) {
                faces.insert(guid, face);
            }
        }
        Ok(Self {
            faces,
            moods,
            names,
        })
    }

    /// The first name of `guid`'s people, if their file gives one.
    pub fn name(&self, guid: u32) -> Option<&str> {
        self.names.get(&guid).map(String::as_str)
    }

    pub fn insert_name(&mut self, guid: u32, name: &str) {
        self.names.insert(guid, name.to_owned());
    }

    pub fn insert(&mut self, guid: u32, face: Image) {
        self.faces.insert(guid, face);
    }

    pub fn face(&self, guid: u32) -> Option<&Image> {
        self.faces.get(&guid)
    }

    pub fn insert_moods(&mut self, guid: u32, moods: Image) {
        self.moods.insert(guid, moods);
    }

    /// The relationship panel's five faces of `guid` ([`MOODS`]).
    pub fn moods(&self, guid: u32) -> Option<&Image> {
        self.moods.get(&guid)
    }

    /// The 45 by 45 portrait of `guid` on cell `cell` of the template.
    pub fn portrait(&self, art: &Art, guid: u32, cell: u32) -> Option<Image> {
        let mut portrait = art.cell(TEMPLATE, 4, cell)?;
        let face = self.face(guid)?;
        for y in 0..face.height.min(FACE[3]) {
            for x in 0..face.width.min(FACE[2]) {
                let (Some(p), Some(i)) = (
                    face.pixel(x, y),
                    portrait.width.checked_mul(y + FACE[1]).and_then(|r| {
                        r.checked_add(x + FACE[0])?
                            .checked_mul(4)
                            .map(|i| i as usize)
                    }),
                ) else {
                    continue;
                };
                if let Some(target) = portrait.rgba.get_mut(i..i + 4) {
                    target.copy_from_slice(&p);
                }
            }
        }
        Some(portrait)
    }
}

/// The first string, in US English, of the catalog strings that the
/// definition `OBJD` `chunk` of `iff` names. None if any of it is missing
/// or the string is empty.
fn first_name(iff: &Iff, chunk: u16) -> Option<String> {
    let definition = iff.data(iff.get(b"OBJD", chunk)?);
    let at = CATALOG_STRINGS * 2;
    let id = u16::from_le_bytes(definition.get(at..at + 2)?.try_into().ok()?);
    let table = StringTable::parse(iff.data(iff.get(b"CTSS", id)?)).ok()?;
    let name = table.get(0, sims_content::ui_graphics::US_ENGLISH)?;
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(hour: i16, minute: i16, funds: i32) -> Snapshot {
        Snapshot {
            hour,
            minute,
            funds,
            paused: false,
            residents: Vec::new(),
            selected: None,
            relations: Vec::new(),
        }
    }

    #[test]
    fn the_sign_is_the_nearest_profile_or_the_saved_one() {
        // Each sign's own traits give it.
        for (n, profile) in ZODIAC_PROFILES.iter().enumerate() {
            let traits = profile.map(|p| (p * 100.0) as i16);
            assert_eq!(nearest_sign(traits), n as u8 + 1);
        }
        // Equally near two signs, the first is kept: no traits at all are
        // 133 from both Cancer and Aquarius.
        assert_eq!(nearest_sign([0; 5]), 4);
        // In the game's 24-bit precision these traits are as near
        // Sagittarius as Aquarius, so Sagittarius, the first, is kept; in
        // 53-bit precision Aquarius would be nearer.
        assert_eq!(nearest_sign([274, 503, 864, 245, 614]), 9);
        // Through the person data: the traits by their words, a saved
        // sign held to 1..=12, and none for a kind above 1.
        let person = |words: &[(usize, i16)]| {
            let words = words.to_vec();
            move |n: usize| words.iter().find(|w| w.0 == n).map_or(0, |w| w.1)
        };
        let leo = [(6, 1000), (2, 300), (5, 400), (7, 400), (3, 400)];
        assert_eq!(zodiac(person(&leo)), Some(5));
        let mut saved = leo.to_vec();
        saved.push((ZODIAC, 2));
        assert_eq!(zodiac(person(&saved)), Some(2));
        assert_eq!(zodiac(person(&[(ZODIAC, 40)])), Some(12));
        assert_eq!(zodiac(person(&[(ZODIAC, -3)])), Some(1));
        assert_eq!(zodiac(person(&[(65, 2)])), None);
        assert_eq!(zodiac(person(&[(65, 1), (ZODIAC, 4)])), Some(4));
    }

    #[test]
    fn a_record_stands_for_its_type_in_the_relationship_list() {
        let record = |id: i16, guid: u32, rows: &[(i32, i32)]| sims_sim::neighbors::Neighbor {
            id,
            guid,
            revision: 1,
            data: vec![0; sims_sim::neighbors::PERSON_WORDS],
            relationships: rows
                .iter()
                .map(|&(target, daily)| sims_sim::neighbors::Row {
                    target,
                    values: vec![daily, 0, 0, 0],
                })
                .collect(),
            friends: 0,
            stale: false,
        };
        let listed = |viewer_rows: &[(i32, i32)]| {
            let mut world = World::empty();
            world.neighbors.slots = vec![
                Some(record(1, 100, viewer_rows)),
                Some(record(2, 200, &[])),
                Some(record(3, 200, &[])),
                Some(record(4, 300, &[])),
            ];
            relations(&world, 100, &[], 50)
                .iter()
                .map(|r| (r.id, r.daily))
                .collect::<Vec<_>>()
        };
        // Records 2 and 3 are one type: both are listed with the row
        // toward 2, the type's first record, and a row toward 3 alone
        // lists neither, as the original's list does.
        assert_eq!(listed(&[(2, 40), (4, 10)]), [(2, 40), (3, 40), (4, 10)]);
        assert_eq!(listed(&[(3, 40), (4, 10)]), [(4, 10)]);
        // Nobody known, or a viewer without a record: an empty list.
        assert_eq!(listed(&[]), []);
        assert!(relations(&World::empty(), 100, &[], 50).is_empty());
    }

    #[test]
    fn clock_and_money_text() {
        assert_eq!(snapshot(6, 30, 0).clock(), "AM 6:30");
        assert_eq!(snapshot(0, 5, 0).clock(), "AM 12:05");
        assert_eq!(snapshot(12, 0, 0).clock(), "PM 12:00");
        assert_eq!(snapshot(23, 59, 0).clock(), "PM 11:59");
        assert_eq!(snapshot(-1, 99, 0).clock(), "PM 11:59");
        assert_eq!(snapshot(0, 0, 2542).money(), "$2,542");
        assert_eq!(snapshot(0, 0, 0).money(), "$0");
        assert_eq!(snapshot(0, 0, 1_234_567).money(), "$1,234,567");
        assert_eq!(snapshot(0, 0, i32::MIN).money(), "-$2,147,483,648");
    }

    #[test]
    fn the_next_level_needs_what_the_person_lacks() {
        let level = |values: [i32; 6]| {
            let mut level = Level {
                values: [0; 21],
                strings: Default::default(),
            };
            level.values[1..7].copy_from_slice(&values);
            level
        };
        let mut careers = Careers::default();
        careers.insert(
            9,
            vec![level([0; 6]), level([100, 0, 300, 40_000, 200, -5])],
        );
        let resident = |job: [i16; 2]| Resident {
            object: 1,
            guid: 1,
            motives: [0.0; 16],
            personality: [0; 5],
            zodiac: None,
            skills: [0, 0, 300, 1000, 199, 0],
            job,
            needs: [7; 6],
        };
        let mut s = snapshot(0, 0, 0);
        s.residents = vec![resident([9, 0]), resident([9, 1]), resident([0, 0])];
        let needs: Vec<[i32; 6]> = s
            .with_careers(&careers)
            .residents
            .iter()
            .map(|r| r.needs)
            .collect();
        // Only more than the skill counts, and as the career has it, past
        // the 16-bit range too; at the top level, or with no career,
        // nothing.
        assert_eq!(needs, [[100, 0, 0, 40_000, 200, 0], [0; 6], [0; 6]]);
    }
}
