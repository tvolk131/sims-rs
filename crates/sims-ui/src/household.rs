//! What the control panel shows, read from the simulation, and the
//! portraits and first names of the household's people.

use crate::art::Art;
use crate::house::{self, House};
use crate::{Error, Result};
use sims_content::{Catalog, Expansions, Index};
use sims_formats::bitmap::{self, Image};
use sims_formats::career::{Career, Level};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_sim::World;
use std::collections::{BTreeMap, BTreeSet};

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
    /// Whether the person is a child: person data 58, the age, under 18
    /// (0x4ea908). The job panel shows a child at school, level 0
    /// (0x4ea722).
    pub child: bool,
    /// Whether the low bit of person data 65 is set, as the job panel
    /// tells a woman (0x4eaa75).
    pub female: bool,
    /// The job panel's performance: person data 63, or a child's data 57
    /// (0x4ea88c).
    pub performance: i16,
    /// What the job panel's left column shows: the default until
    /// [`Snapshot::with_careers`] fills it in.
    pub work: Work,
}

/// What the job panel's left column shows of a person (0x530501).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Work {
    /// The row of the career icons ([`crate::hud`]'s art 4902) the panel
    /// shows: the ID of the career the person has (person data 56), 0
    /// with none or one `work.iff` lacks (0x4ea701). A row the icons do
    /// not have shows none: a child's -1, school.
    pub icon: i16,
    /// The level the person is at ([`Snapshot::with_careers`]): none for a
    /// person without a career, or past its levels.
    pub level: Option<WorkLevel>,
    /// The family friends the next level of the person's career needs:
    /// its value 0, or 0 with no next level (0x4ea7cc). The panel shows
    /// it where the family has fewer ([`Snapshot::family_friends`],
    /// 0x4ea8d3).
    pub friends: i32,
}

/// A career level as the job panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkLevel {
    /// The level's title: a woman's where the career has one (0x4033a4).
    pub title: String,
    /// The daily pay: the level's value 17 (0x4b4513).
    pub salary: i32,
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

/// Person data words the job panel reads: the age, the performance, and
/// the low bit of the kind for a woman (0x4ea908, 0x4ea88c, 0x4eaa75).
const AGE: usize = 58;
const PERFORMANCE: usize = 63;
/// The age a person is a child under.
const GROWN: i16 = 18;

/// The career a child is in, and the level (0x4ea722): school, `CARR`
/// 0xffff.
const SCHOOL: i16 = -1;

/// The level value of the family friends a career level needs, and of its
/// daily pay (0x403429, 0x4b4513).
const FRIENDS_NEEDED: usize = 0;
const SALARY: usize = 17;

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
    /// How many friends the family has, as the job panel compares it
    /// (0x410260): the count the lot's family (global 9) keeps
    /// ([`sims_sim::neighbors::Family::friends`]), which the lot works
    /// out again after each update (`refresh_family_panel` in sims-vm),
    /// or 0 with no such family. [`Snapshot::with_relations`] works it
    /// out if it is out of date.
    pub family_friends: i32,
    /// The people the selected Sim has a relationship with, in the
    /// relationship panel's order ([`Relation`]): none until
    /// [`Snapshot::with_relations`] reads them.
    pub relations: Vec<Relation>,
    /// Whether the relationship panel's Superstar button can be pressed:
    /// the Superstar pack is installed ([`Acquaintances::superstar`]).
    /// False until [`Snapshot::with_relations`] reads it.
    pub superstar: bool,
    /// What the house panel shows: nothing until [`Snapshot::with_house`]
    /// works it out.
    pub house: House,
    /// Whether the lot is residential (0x40745f), which the House button
    /// needs. The world does not keep the lots' zoning, so the host sets
    /// it (`sims_vm::Lot::residential`); true until then.
    pub residential: bool,
}

/// What the relationship panel needs from outside the world: the
/// install's packs, the catalog's types and the autonomy tuning.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Acquaintances {
    /// The autonomy tuning's friendship threshold (0x6c4398, read from
    /// the tuning: 50 in the original's capture), which the smiley needs.
    pub friendship: i32,
    /// The GUIDs of the neighborhood's characters: the types whose
    /// definition comes from its `Characters` folder. The original
    /// registers that folder with kind 1, which 0x43ba52 keeps in bits 2
    /// and 3 of the type's record (+0xb4); 0x423f30 tests for 4 there
    /// (`docs/decisions/20261115-household-selection-works-out-again-who-visitor.md`).
    pub characters: BTreeSet<u32>,
    /// Whether the Superstar pack is installed: the installed packs'
    /// mask (`[0x6e48bc]+0x66`) has its bit, 0x40 (0x51a7ea).
    pub superstar: bool,
}

impl Acquaintances {
    /// The bit of the Superstar pack in the installed packs' mask.
    pub const SUPERSTAR: u8 = 0x40;

    /// Reads the characters from `catalog` as `index` mounts them (each
    /// GUID's definition, the one it resolves to), and the Superstar
    /// pack from `expansions`.
    pub fn read(index: &Index, catalog: &Catalog, expansions: Expansions, friendship: i32) -> Self {
        let characters = catalog
            .objects
            .iter()
            .map(|o| o.guid)
            .filter(|&guid| {
                catalog.object(guid).is_some_and(|o| {
                    index.mounts[index.resources[o.resource].mount].path == Index::CHARACTERS
                })
            })
            .collect();
        Self {
            friendship,
            characters,
            superstar: expansions.contains(Self::SUPERSTAR),
        }
    }
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
    /// Whether their person data word 81 is set: the Superstar button's
    /// list (0x4e9f22). Superstar keeps a person's fame there.
    pub star: bool,
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
                let child = data(AGE) < GROWN;
                residents.push(Resident {
                    object: object.id,
                    guid,
                    motives: person.map(|p| p.motives).unwrap_or_default(),
                    personality: PERSONALITY.map(data),
                    zodiac: zodiac(data),
                    skills: SKILLS.map(data),
                    job: JOB.map(data),
                    needs: [0; 6],
                    child,
                    female: data(sims_sim::neighbors::KIND) & 1 == 1,
                    performance: data(if child { JOB[1] } else { PERFORMANCE }),
                    work: Work::default(),
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
            family_friends: lot_family(world).map_or(0, |f| f.friends),
            relations: Vec::new(),
            superstar: false,
            house: House::default(),
            residential: true,
        }
    }

    /// Fills in each resident's [`Resident::needs`] from `careers`, as
    /// the original's 0x4ea823 works them out: the value of the level
    /// after the person's own, where it is more than the person's skill.
    /// A person with no career, or at its top level, needs nothing.
    ///
    /// Fills in each resident's [`Resident::work`] too, from the career
    /// and level the job panel reads (0x4ea722: a child's is school, level
    /// 0) and the next level of the person's own career (0x4ea7cc).
    #[must_use]
    pub fn with_careers(mut self, careers: &Careers) -> Self {
        for resident in &mut self.residents {
            let [career, level] = resident.job;
            let next = careers.level(career, i32::from(level) + 1);
            let (shown, at) = if resident.child {
                (SCHOOL, 0)
            } else {
                (career, i32::from(level))
            };
            resident.work = Work {
                icon: if careers.has(career) { career } else { 0 },
                level: careers.level(shown, at).map(|l| WorkLevel {
                    title: careers
                        .title(shown, at, resident.female)
                        .unwrap_or_default()
                        .to_owned(),
                    salary: l.values[SALARY],
                }),
                friends: next.map_or(0, |l| l.values[FRIENDS_NEEDED]),
            };
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

    /// Works out [`Snapshot::house`] from `world` ([`House::read`]).
    #[must_use]
    pub fn with_house(
        mut self,
        world: &World,
        tuning: &house::Tuning,
        kinds: &house::Kinds,
    ) -> Self {
        self.house = House::read(world, tuning, kinds);
        self
    }

    /// Reads the selected Sim's [`Snapshot::relations`] from `world`,
    /// and [`Snapshot::superstar`]. `members` is as for
    /// [`Snapshot::read`].
    ///
    /// Works [`Snapshot::family_friends`] out again too where the count
    /// is out of date, as reading it does in the original (0x427219).
    #[must_use]
    pub fn with_relations(mut self, world: &World, members: &[u32], known: &Acquaintances) -> Self {
        if let Some(family) = lot_family(world).filter(|f| f.stale) {
            self.family_friends = world.count_family_friends(family, known.friendship);
        }
        let selected = self.selected.and_then(|id| world.objects.get(&id));
        self.relations = selected
            .map(|o| relations(world, o.guid, members, known))
            .unwrap_or_default();
        self.superstar = known.superstar;
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
        money(self.funds)
    }
}

/// An amount as the panel writes money: `$2,542` ([`Snapshot::money`]).
pub fn money(amount: i32) -> String {
    let digits = amount.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("{}${out}", if amount < 0 { "-" } else { "" })
}

/// The careers of `work.iff`, by the ID of their `CARR` resource, as
/// the original looks a person's career up (0x404cb3), with the levels'
/// titles.
#[derive(Debug, Clone, Default)]
pub struct Careers {
    careers: BTreeMap<i16, Vec<Level>>,
    /// Each career's levels' titles, a man's and a woman's.
    titles: BTreeMap<i16, Vec<[String; 2]>>,
}

/// The career, in `work.iff`'s order, whose titles the original never
/// reads (0x403f8f): school, which has no `STR#`.
const UNTITLED: usize = 10;

impl Careers {
    /// Reads every `CARR` of the install's `work.iff`, and the levels'
    /// titles from the `STR#` of the career's ID (0x403f26): for level
    /// `k`, string `3 + 2k`, and a woman's, `23 + k`, in US English. An
    /// install without the file has no careers.
    ///
    /// The original reads the titles of only the first 11, 16 or 21
    /// careers, by the game's expansion level (0x403f53); here every
    /// career's, which is all 21 of the full install's.
    pub fn load(index: &Index) -> Result<Self> {
        let mut out = Self::default();
        let Some((_, file)) = index.named("work.iff").next() else {
            return Ok(out);
        };
        let iff = Iff::parse(file.read().map_err(Error::Content)?).map_err(Error::Format)?;
        for (n, chunk) in iff
            .resource_order(b"CARR")
            .map_err(Error::Format)?
            .into_iter()
            .enumerate()
        {
            let career = Career::parse(iff.data(chunk)).map_err(Error::Format)?;
            let id = chunk.id as i16;
            if n != UNTITLED
                && let Some(strings) = iff.get(b"STR#", chunk.id)
            {
                let table = StringTable::parse(iff.data(strings)).map_err(Error::Format)?;
                let text = |i: usize| {
                    table
                        .get(i, sims_content::ui_graphics::US_ENGLISH)
                        .unwrap_or_default()
                        .to_owned()
                };
                let titles = (0..career.levels.len())
                    .map(|k| [text(3 + 2 * k), text(23 + k)])
                    .collect();
                out.titles.insert(id, titles);
            }
            out.careers.insert(id, career.levels);
        }
        Ok(out)
    }

    /// Adds or replaces a career, as tests do.
    pub fn insert(&mut self, id: i16, levels: Vec<Level>) {
        self.careers.insert(id, levels);
    }

    /// Adds or replaces a career's titles, a man's and a woman's for
    /// each level, as tests do.
    pub fn insert_titles(&mut self, id: i16, titles: Vec<[String; 2]>) {
        self.titles.insert(id, titles);
    }

    /// Whether career `career` exists (0x404cb3).
    pub fn has(&self, career: i16) -> bool {
        self.careers.contains_key(&career)
    }

    /// The title of level `level` of career `career` (0x4033a4): a
    /// woman's where the level has one, else the level's; empty for a
    /// level without a title.
    pub fn title(&self, career: i16, level: i32, female: bool) -> Option<&str> {
        let [man, woman] = self
            .titles
            .get(&career)?
            .get(usize::try_from(level).ok()?)?;
        Some(if female && !woman.is_empty() {
            woman
        } else {
            man
        })
    }

    /// Level `level` (from 0) of career `career`, if both exist.
    pub fn level(&self, career: i16, level: i32) -> Option<&Level> {
        if career == 0 {
            return None;
        }
        self.careers.get(&career)?.get(usize::try_from(level).ok()?)
    }
}

/// The lot's family: the first whose ID is global 9, as the lot's
/// family panel finds it (`refresh_family_panel` in sims-vm).
fn lot_family(world: &World) -> Option<&sims_sim::neighbors::Family> {
    let id = i32::from(world.globals.get(9).copied().unwrap_or(0));
    (id != 0).then(|| world.families.iter().find(|f| f.id == id))?
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
/// Person data word 32: whether the person lives on the lot. A 6 keeps
/// anyone in the relationship panel (0x4e9eae); what a 6 stands for is
/// not known.
const VISITOR: usize = 32;
const LISTED_ANYWAY: i16 = 6;
/// Person data word 81: the Superstar button lists those with it set
/// (0x4e9f2a).
const FAME: usize = 81;

/// A person's data as the relationship panel reads it (0x4ea662): the
/// first person on the lot of the record's type, in the lot's people
/// order (0x4e9608 walks the lot's people for the first of the GUID),
/// or else the first record of the type.
fn person_data(world: &World, guid: u32) -> Option<&[i16]> {
    world
        .people
        .iter()
        .filter_map(|id| world.objects.get(id))
        .find_map(|o| o.person.as_ref().filter(|_| o.guid == guid))
        .map(|p| p.data.as_slice())
        .or_else(|| world.neighbors.first_of(guid).map(|r| r.data.as_slice()))
}

fn relations(world: &World, guid: u32, members: &[u32], known: &Acquaintances) -> Vec<Relation> {
    let friendship = known.friendship;
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
        // 0x4e9e9b..0x4e9ed8: a person the panel lists is a neighborhood
        // character of a family, or anyone whose word 32 is 6. Their data
        // is the person's on the lot, or else the first record's.
        let data = person_data(world, record.guid).unwrap_or_default();
        let word = |n: usize| data.get(n).copied().unwrap_or(0);
        let anyway = word(VISITOR) == LISTED_ANYWAY;
        if !anyway
            && (!known.characters.contains(&record.guid) || word(sims_sim::neighbors::FAMILY) == 0)
        {
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
            star: word(FAME) != 0,
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
            family_friends: 0,
            relations: Vec::new(),
            superstar: false,
            house: House::default(),
            residential: true,
        }
    }

    /// A neighbor record of a family (word 61 set) with rows toward
    /// `rows` (target, daily value).
    fn record(id: i16, guid: u32, rows: &[(i32, i32)]) -> sims_sim::neighbors::Neighbor {
        let mut data = vec![0; sims_sim::neighbors::PERSON_WORDS];
        data[sims_sim::neighbors::FAMILY] = 7;
        sims_sim::neighbors::Neighbor {
            id,
            guid,
            revision: 1,
            data,
            relationships: rows
                .iter()
                .map(|&(target, daily)| sims_sim::neighbors::Row {
                    target,
                    values: vec![daily, 0, 0, 0],
                })
                .collect(),
            friends: 0,
            stale: false,
        }
    }

    /// The tuning's threshold, with every GUID from 100 to 999 a
    /// neighborhood character.
    fn known() -> Acquaintances {
        Acquaintances {
            friendship: 50,
            characters: (100..1000).collect(),
            superstar: true,
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
        let listed = |viewer_rows: &[(i32, i32)]| {
            let mut world = World::empty();
            world.neighbors.slots = vec![
                Some(record(1, 100, viewer_rows)),
                Some(record(2, 200, &[])),
                Some(record(3, 200, &[])),
                Some(record(4, 300, &[])),
            ];
            relations(&world, 100, &[], &known())
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
        assert!(relations(&World::empty(), 100, &[], &known()).is_empty());
    }

    #[test]
    fn the_relationship_list_leaves_out_who_the_original_leaves_out() {
        let rows: Vec<(i32, i32)> = (2..=6).map(|id| (id, 10 * id)).collect();
        let mut world = World::empty();
        world.neighbors.slots = vec![Some(record(1, 100, &rows))];
        // 2 is a character of a family; 3 has no family; 4's type is not
        // a character; 5 is neither, but its word 32 is 6; 6 has fame.
        let mut people: Vec<_> = (2..=6).map(|id| record(id, 100 * id as u32, &[])).collect();
        people[1].data[sims_sim::neighbors::FAMILY] = 0;
        people[3].data[sims_sim::neighbors::FAMILY] = 0;
        people[3].data[VISITOR] = LISTED_ANYWAY;
        people[4].data[FAME] = 3;
        world.neighbors.slots.extend(people.into_iter().map(Some));
        let mut known = known();
        known.characters.remove(&400);
        known.characters.remove(&500);
        let listed = relations(&world, 100, &[], &known);
        let ids: Vec<_> = listed.iter().map(|r| (r.id, r.star)).collect();
        assert_eq!(ids, [(6, true), (5, false), (2, false)]);

        // A person on the lot is judged by their own data, not the
        // record's: give person 300 a family on the lot.
        use sims_formats::saved_objects::{Interaction, Person};
        let idle = Interaction {
            uid: 0,
            caller: 0,
            target: 0,
            icon: 0,
            table_index: -1,
            args: [0; 4],
            priority: 0,
            action: 0,
            attenuation: 0.0,
            flags: 0,
        };
        let mut data = vec![0; sims_sim::neighbors::PERSON_WORDS];
        data[sims_sim::neighbors::FAMILY] = 7;
        data[FAME] = 1;
        let person = Person {
            animation_events: 0,
            engaged: 0,
            appearance: Default::default(),
            accessories: Vec::new(),
            animations: Default::default(),
            routing_state: 0,
            floats: [0.0; 9],
            previous_motives: [0.0; 16],
            motives: [0.0; 16],
            data,
            routing_frames: 0,
            active: idle.clone(),
            last: idle,
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        };
        let object = sims_sim::Object {
            id: 30,
            guid: 300,
            footprint: [0; 4],
            position: [0; 3],
            unknown_short: 0,
            attributes: Vec::new(),
            temps: Vec::new(),
            data: vec![0; 128],
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: Vec::new(),
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: Some(person),
            registry_word: None,
            last_result: false,
            removal_skips_resets: false,
            selected: false,
        };
        world.objects.insert(30, object);
        world.people.push(30);
        let listed = relations(&world, 100, &[], &known);
        let ids: Vec<_> = listed.iter().map(|r| (r.id, r.star)).collect();
        assert_eq!(ids, [(3, true), (6, true), (5, false), (2, false)]);
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
            child: false,
            female: false,
            performance: 0,
            work: Work::default(),
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

    #[test]
    fn the_job_panel_shows_the_level_the_person_is_at() {
        let level = |friends: i32, salary: i32| {
            let mut level = Level {
                values: [0; 21],
                strings: Default::default(),
            };
            level.values[FRIENDS_NEEDED] = friends;
            level.values[SALARY] = salary;
            level
        };
        let mut careers = Careers::default();
        careers.insert(9, vec![level(0, 155), level(2, 230)]);
        careers.insert(SCHOOL, vec![level(0, 0), level(0, 0)]);
        careers.insert(3, vec![level(0, 240)]);
        careers.insert_titles(
            9,
            vec![
                ["Test Subject".into(), String::new()],
                ["Lab Assistant".into(), "Lab Lady".into()],
            ],
        );
        // A woman's title only where the level has one; none for a level
        // past the titles, or a career without them.
        assert_eq!(careers.title(9, 0, true), Some("Test Subject"));
        assert_eq!(careers.title(9, 1, true), Some("Lab Lady"));
        assert_eq!(careers.title(9, 1, false), Some("Lab Assistant"));
        assert_eq!(careers.title(9, 2, false), None);
        assert_eq!(careers.title(9, -1, false), None);
        assert_eq!(careers.title(3, 0, false), None);
        assert!(careers.has(SCHOOL) && !careers.has(0) && !careers.has(4));

        let resident = |job: [i16; 2], child: bool, female: bool| Resident {
            object: 1,
            guid: 1,
            motives: [0.0; 16],
            personality: [0; 5],
            zodiac: None,
            skills: [0; 6],
            job,
            needs: [0; 6],
            child,
            female,
            performance: 0,
            work: Work::default(),
        };
        let mut s = snapshot(0, 0, 0);
        s.residents = vec![
            resident([9, 0], false, false),
            resident([9, 1], false, true),
            resident([9, 2], false, false),
            resident([3, 0], false, false),
            resident([4, 0], false, false),
            resident([0, 0], false, false),
            resident([SCHOOL, 4], true, false),
            resident([9, 0], true, false),
        ];
        let work: Vec<_> = s
            .with_careers(&careers)
            .residents
            .into_iter()
            .map(|r| r.work)
            .collect();
        let at = |title: &str, salary: i32| {
            Some(WorkLevel {
                title: title.into(),
                salary,
            })
        };
        let shown = |icon: i16, level: Option<WorkLevel>, friends: i32| Work {
            icon,
            level,
            friends,
        };
        assert_eq!(
            work,
            [
                // The next level needs two friends.
                shown(9, at("Test Subject", 155), 2),
                shown(9, at("Lab Lady", 230), 0),
                // Past the career's levels: no level.
                shown(9, None, 0),
                // A career without titles shows an empty one.
                shown(3, at("", 240), 0),
                // A career work.iff lacks: the first icon, no level.
                shown(0, None, 0),
                shown(0, None, 0),
                // A child is at school, level 0, whatever the words say;
                // the icon and the friends follow the words.
                shown(SCHOOL, at("", 0), 0),
                shown(9, at("", 0), 2),
            ]
        );
    }
}
