//! What the control panel shows, read from the simulation, and the
//! portraits and first names of the household's people.

use crate::art::Art;
use crate::house::{self, House};
use crate::{Error, Result};
use sims_content::{ArchivePool, Catalog, Expansions, Index, Install};
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
/// The budget-mode global (`[sim + 0x50]`, `0x4f24fa`): when equal to 1,
/// the control panel hides both the funds box and the family-friends
/// counter (`0x4cfe0a`, `0x4cfe18`, `0x53b2a4..0x53b2e1`).
const BUDGET_MODE: usize = 32;

/// A person of the household, as the panel shows them.
/// Which pet species panel (`[esi + 0x114]` dog vs `[esi + 0x118]` cat) the
/// live-mode Job/Skills tab shows (`0x51d5ca`..`0x51d5f0`,
/// `0x51cbf8`..`0x51cc5a`), from `person.data[65]` (`sims_sim::neighbors::KIND`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PetKind {
    /// Bit 1 (`(kind >> 1) & 1 != 0`): House Breaking, Tricks, Obedience.
    Dog,
    /// Bit 2 (`(kind >> 2) & 1 != 0`, when bit 1 is clear): Hunting, Tricks, Obedience.
    Cat,
}

impl PetKind {
    /// Classifies `person.data[65]` (`word [person + 0x60e]`) in the exact bit
    /// order of `0x51d5d3`..`0x51d5e7` and `0x51cc01`..`0x51cc2e`.
    pub fn from_kind(kind: i16) -> Option<Self> {
        let bits = i32::from(kind);
        if (bits >> 1) & 1 != 0 {
            Some(Self::Dog)
        } else if (bits >> 2) & 1 != 0 {
            Some(Self::Cat)
        } else {
            None
        }
    }
}

/// Which subpanel the live-mode Job tab (`Panel::Job`, panel 3) shows for a
/// selected household Sim (`0x51d598..0x51d624`, `0x51cbf8..0x51ccf4`,
/// `0x51d09a..0x51d149`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobPanel {
    /// Child Report Card (`[esi + 0x124]`, `age != 0 && age < 18`).
    ReportCard,
    /// Dog (`[esi + 0x114]`) or Cat (`[esi + 0x118]`) Pet Skills.
    PetSkills(PetKind),
    /// Superstar / Fame (`[esi + 0x11c]`, `0x4cfe2c`).
    Fame,
    /// Adult Job (`[esi + 0x110]`).
    Adult,
}

/// A household member on the lot, in the family's portrait order.
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
    /// of a point (0 to 1000). For pets (`0x66e360`), the first three
    /// words (`[10, 12, 11]`) are House Breaking/Hunting, Tricks, and
    /// Obedience.
    pub skills: [i16; 6],
    /// The career (a `CARR` resource's ID; 0 for none) and the level in
    /// it, person data 56 and 57.
    pub job: [i16; 2],
    /// Whether `person.data[56]` resolves to a career in `work.iff`
    /// (`0x4ea701`), used by `0x4cfe2c` so Sims with a regular career stay
    /// on the Adult Job panel even if they have a nonzero fame score.
    pub has_career: bool,
    /// What the next level of the career needs of each skill, where the
    /// person has less: 0 elsewhere, and everywhere until
    /// [`Snapshot::with_careers`] fills it in.
    pub needs: [i32; 6],
    /// Whether the person is a child: person data 58, the age, under 18
    /// (0x4ea908). The job panel shows a child at school, level 0
    /// (0x4ea722).
    pub child: bool,
    /// Whether `0x51d5af`..`0x51d5bf` routes the Job tab to the child
    /// Report Card panel before checking pet bits (`age != 0 && age < 18`).
    pub school_age: bool,
    /// Whether the low bit of person data 65 is set, as the job panel
    /// tells a woman (0x4eaa75).
    pub female: bool,
    /// Whether the person is a pet: person data 65 above 1 (`0x51c61f`,
    /// `0x51ce13`, `0x51cd11`). Pets have no zodiac sign, use pet trait
    /// labels (`Live.iff STR# 170`), disable the Interests and Gifts
    /// gadget buttons, and return from those panels to Needs when selected.
    pub pet: bool,
    /// Whether the person's `person.data[65]` selects the Dog (`[esi + 0x114]`)
    /// or Cat (`[esi + 0x118]`) Pet Skills panel (`0x51d5ca`..`0x51d5f0`).
    pub pet_kind: Option<PetKind>,
    /// The job panel's performance: person data 63, or a child's data 57
    /// (0x4ea88c).
    pub performance: i16,
    /// Child Report Card grade text (`0x532dc0..0x532e0e`, `0x4034ce` reading
    /// `work.iff STR# 4097` index `performance + 1`, or `"No cJob Object!"`
    /// when `work.iff` lacks school career `-1`). Filled in by
    /// [`Snapshot::with_careers`].
    pub school_grade: String,
    /// Whether the Superstar Fame panel applies: person data 80 (`+0xa0`,
    /// fame score, `0x4cfe5c`) and person data 81 (`+0xa2`, fame level
    /// `0..=10` half-stars, `0x4051ec`).
    pub fame_score: i16,
    pub fame_level: i16,
    /// Superstar fame title (`fame.iff STR# 1[1 + fame_level]`, `0x52d3e6`),
    /// skill point targets (`0..=10`, `0x405190`), and formatted friend star
    /// power text (`0x52d1af`), filled in by [`Snapshot::with_fame`].
    pub fame_title: String,
    pub fame_needs: [i32; 6],
    pub fame_friends_text: String,
    /// What the job panel's left column shows: the default until
    /// [`Snapshot::with_careers`] fills it in.
    pub work: Work,
    /// The 19 interest scores (`0..=1000`, 100 per bar segment) in native
    /// `cWinInterests` order (`0x50f595..0x50f6a6`, [`INTEREST_WORDS`]).
    pub interests: [i16; 19],
}

impl Resident {
    /// Whether `0x4cfe2c` routes this resident to the Superstar / Fame
    /// subpanel (`[esi + 0x11c]`): Superstar pack installed (`0x40`),
    /// `person.data[80] > 0` (`fame_score > 0`), and no active regular
    /// career (`0x4ea701 == 0`).
    pub fn is_superstar(&self, superstar_pack: bool) -> bool {
        superstar_pack && self.fame_score > 0 && !self.has_career
    }

    /// Which Job-tab subpanel (`Panel::Job`) shows for this resident in the
    /// exact priority order of `0x51d598..0x51d624`.
    pub fn job_panel(&self, superstar_pack: bool) -> JobPanel {
        if self.school_age {
            JobPanel::ReportCard
        } else if let Some(kind) = self.pet_kind {
            JobPanel::PetSkills(kind)
        } else if self.is_superstar(superstar_pack) {
            JobPanel::Fame
        } else {
            JobPanel::Adult
        }
    }
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

/// The person data words of the 19 interests in native `cWinInterests` order
/// (`0x50f595..0x50f6a6`, UIGraphics IDs `1600..=1618`, `Live.iff STR# 140[1..=19]`):
/// - `0..=7`: Travel, Money, Politics, The 60's, Weather, Sports, Music, Outdoors (`46..=53`)
/// - `8..=11`: Child-only Toys, Aliens, Pets, School (share `46..=49`)
/// - `12..=18`: Exercise (`13`), Food (`14`), Parties (`16`), Style (`20`),
///   Hollywood (`26`), Technology (`54`), Romance (`55`).
pub const INTEREST_WORDS: [usize; 19] = [
    46, 47, 48, 49, 50, 51, 52, 53, 46, 47, 48, 49, 13, 14, 16, 20, 26, 54, 55,
];

/// One inventory token displayed in the Live HUD Gifts/Inventory subpanel
/// (`Panel::Gifts`, `4507`, `0x51e343..0x51e440`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    /// Native inventory token kind (`0, 4, 5` Gifts; `7` Magic; `8` Other).
    pub kind: i32,
    /// Catalog object GUID of the token.
    pub guid: u32,
    /// Quantity carried by the selected Sim.
    pub count: u16,
    /// Localized display name from the object's `CTSS` / `OBJD`.
    pub name: String,
}

/// The person data words of the career and the level in it.
const JOB: [usize; 2] = [56, 57];

/// Person data words the job panel reads: the age, the performance, and
/// the low bit of the kind for a woman (0x4ea908, 0x4ea88c, 0x4eaa75).
const AGE: usize = 58;
const PERFORMANCE: usize = 63;
/// Person data word 80 (`+0xa0`): Superstar fame score (`0x4cfe5c`).
const FAME_SCORE: usize = 80;
/// The age a person is a child under.
const GROWN: i16 = 18;

/// Fallback text when `0x4ea722` finds no school career object (`0x532dce`, `0x6ccde8`).
pub const NO_JOB_OBJECT: &str = "No cJob Object!";
/// `work.iff` string table holding school grades (`0x403f43 -> 0x6de7d8`).
const GRADE_STRINGS: u16 = 4097;

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
    /// Whether the cluster shows the funds and family-friends counters:
    /// false when global 32 (budget mode, `[sim + 0x50]`) is 1
    /// (`0x4f24fa` → `0x4cfe0a` / `0x4cfe18` → `0x53b2a4..0x53b2e1`).
    pub show_money: bool,
    pub paused: bool,
    /// Whether the idle loop is currently holding the lot at
    /// [`sims_sim::speed::IDLE`] (`[control_panel + 0x1ec]` set by `0x46d7b5`
    /// → `0x4f688e`, cleared on `0x46d803` or speed click at
    /// `0x53a5e3`..`0x53a6e1`): `world.return_speed() >= 0 && world.speed() ==
    /// sims_sim::speed::IDLE`.
    pub idle_speed: bool,
    /// Whether the lot is residential (`0x40745f`), used to pick which
    /// family supplies [`Snapshot::family_friends`] (`0x4cfcdb`).
    /// Defaults to `true` until [`Snapshot::with_zoning`] sets it.
    pub residential: bool,
    /// The household's people on the lot, in the family's member order.
    pub residents: Vec<Resident>,
    /// The selected Sim's object ID.
    pub selected: Option<u16>,
    /// The selected Sim's type GUID (`0x4e986d` → `0x4e9753`), used by
    /// [`Snapshot::selected_resident`] to match the household roster by
    /// neighbor/GUID (`0x51c5ef` → `0x4e97dc`) even if a second live
    /// instance of a household Sim is selected.
    pub selected_guid: Option<u32>,
    /// How many friends the lot's family (global 9) has, as the job panel
    /// compares it (`0x4ea8ed..0x4ea8f7`, `0x531095..0x5310a4`, `0x410260`):
    /// [`sims_sim::neighbors::Family::friends`], or 0 with no such family.
    /// [`Snapshot::with_zoning`] and [`Snapshot::with_relations`] work it
    /// out if it is out of date.
    pub family_friends: i32,
    /// How many friends the control-panel cluster displays (`[esi + 0x1d0]`,
    /// `0x4cfcdb`): the lot's family (global 9) on residential lots, or the
    /// selected Sim's family (`person.data[61]`, `0x425d69`) on
    /// non-residential lots, or 0 with no such family.
    pub cluster_friends: i32,
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
    /// The selected Sim's inventory tokens (`Neighbor.inventory`, `0x51e343`):
    /// empty until [`Snapshot::with_inventory`] reads them.
    pub inventory: Vec<InventoryEntry>,
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
    /// The interface's table of person types (0x6f2698), by GUID, in its
    /// order: each person definition (`OBJD` type 2) in the definition
    /// registry's order at startup (0x4e8b3d walks the registry's vector
    /// with 0x43ac3a), a GUID's first place kept (0x4e9753 finds the
    /// first). The relationship panel lists only people in it, and breaks
    /// its last tie by their place (0x4e9e80, 0x4ea035). A person type
    /// made while the game runs is appended after these
    /// ([`Acquaintances::portrait_place`]).
    pub portraits: Vec<u32>,
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
        let mut portraits = Vec::new();
        for object in &catalog.objects {
            if object.kind == PERSON_KIND && !portraits.contains(&object.guid) {
                portraits.push(object.guid);
            }
        }
        Self {
            friendship,
            characters,
            superstar: expansions.contains(Self::SUPERSTAR),
            portraits,
        }
    }

    /// A person type's place in the interface's table of person types
    /// (0x4e9753), or None when it is not there (-1).
    ///
    /// Types made while the game runs (`world.characters.made`) come
    /// after the startup ones, in the order made: making one appends it
    /// (0x4265fd → 0x4e8a3a, which walks the registry again and appends
    /// each person GUID the table lacks, 0x4e8add, and then only the new
    /// type is missing). The GUID does not give that order: two types
    /// made in one tick can have descending GUIDs.
    pub fn portrait_place(&self, world: &World, guid: u32) -> Option<usize> {
        if let Some(place) = self.portraits.iter().position(|&other| other == guid) {
            return Some(place);
        }
        world
            .characters
            .made
            .keys()
            .filter(|other| !self.portraits.contains(other))
            .position(|&other| other == guid)
            .map(|place| self.portraits.len() + place)
    }
}

/// The `OBJD` type of a person (definition +0x12, 0x4e8b6a).
const PERSON_KIND: u16 = 2;

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

/// Finds the first live person of type `guid` in person-registry order
/// (`0x4e9608`: scans `[registry + 0x84]` = `world.people` and returns `None`
/// on a miss). Falls back to `world.objects` only when `world.people` is empty
/// (for synthetic worlds that never populate the person registry).
pub fn first_person_for_guid(world: &World, guid: u32) -> Option<&sims_sim::Object> {
    if !world.people.is_empty() {
        return world
            .people
            .iter()
            .filter_map(|id| world.objects.get(id))
            .find(|o| o.guid == guid && o.person.is_some());
    }
    world
        .objects
        .values()
        .find(|o| o.guid == guid && o.person.is_some())
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
            // The first person of the type in person-registry order
            // (`0x4e9608` scanning `[registry + 0x84]`).
            if let Some(object) = first_person_for_guid(world, guid) {
                let person = object.person.as_ref();
                let data =
                    |word: usize| person.and_then(|p| p.data.get(word).copied()).unwrap_or(0);
                let age = data(AGE);
                let child = age < GROWN;
                let school_age = age != 0 && age < GROWN;
                let kind = data(sims_sim::neighbors::KIND);
                residents.push(Resident {
                    object: object.id,
                    guid,
                    motives: person.map(|p| p.motives).unwrap_or_default(),
                    personality: PERSONALITY.map(data),
                    zodiac: zodiac(data),
                    skills: SKILLS.map(data),
                    job: JOB.map(data),
                    has_career: false,
                    needs: [0; 6],
                    child,
                    school_age,
                    female: kind & 1 == 1,
                    pet: kind > 1,
                    pet_kind: PetKind::from_kind(kind),
                    performance: data(if child { JOB[1] } else { PERFORMANCE }),
                    school_grade: String::new(),
                    fame_score: data(FAME_SCORE),
                    fame_level: data(FAME),
                    fame_title: String::new(),
                    fame_needs: [0; 6],
                    fame_friends_text: String::new(),
                    work: Work::default(),
                    interests: INTEREST_WORDS.map(data),
                });
            }
        }
        let selected_obj = world
            .objects
            .values()
            .find(|o| o.selected && o.person.is_some());
        let selected = selected_obj.map(|o| o.id);
        let selected_guid = selected_obj.map(|o| o.guid);
        let lot_friends = lot_family(world).map_or(0, |f| f.friends);
        Self {
            hour: global(HOURS),
            minute: global(MINUTES),
            funds: world.budget.funds,
            show_money: global(BUDGET_MODE) != 1,
            paused: global(PAUSED) != 0,
            idle_speed: world.return_speed() >= 0 && world.speed() == sims_sim::speed::IDLE,
            residential: true,
            residents,
            selected,
            selected_guid,
            family_friends: lot_friends,
            cluster_friends: lot_friends,
            relations: Vec::new(),
            superstar: false,
            house: House::default(),
            inventory: Vec::new(),
        }
    }

    /// Overrides [`Snapshot::idle_speed`] with the host control panel's
    /// tracked idle-blink flag (`[control_panel + 0x1ec]`, set by `0x109(1)`,
    /// cleared by `0x109(0)` or manual speed click `0x53a5e3..0x53a6e1`, and
    /// preserved across Special Effect `0x10c` speed resets at
    /// `0x4f6856..0x4f687a`).
    #[must_use]
    pub fn with_idle_speed(mut self, idle_speed: bool) -> Self {
        self.idle_speed = idle_speed;
        self
    }

    /// Sets whether the lot is residential (`0x40745f`) and reads both
    /// family-friends counts (`0x410260` → `0x427219`):
    /// - [`Snapshot::family_friends`] (Job panel, `0x4ea8ed`, `0x531095`)
    ///   from the lot's family (global 9);
    /// - [`Snapshot::cluster_friends`] (cluster counter, `0x4cfcdb`) from
    ///   the lot's family on residential lots or the selected Sim's family
    ///   (`person.data[61]`, `0x425d69`) on non-residential lots.
    ///
    /// Either count is recomputed with `friendship` if its family is stale.
    #[must_use]
    pub fn with_zoning(mut self, world: &World, residential: bool, friendship: i32) -> Self {
        self.residential = residential;
        let count = |family: Option<&sims_sim::neighbors::Family>| {
            family.map_or(0, |f| {
                if f.stale {
                    world.count_family_friends(f, friendship)
                } else {
                    f.friends
                }
            })
        };
        self.family_friends = count(lot_family(world));
        self.cluster_friends = count(friends_family(world, self.selected, residential));
        self
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
            resident.has_career = career != 0 && careers.has(career);
            let shown_level = careers.level(shown, at);
            resident.school_grade = if shown_level.is_some() {
                careers.grade(resident.performance).to_owned()
            } else {
                NO_JOB_OBJECT.to_owned()
            };
            resident.work = Work {
                icon: if careers.has(career) { career } else { 0 },
                level: shown_level.map(|l| WorkLevel {
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

    /// Populates [`Snapshot::inventory`] for the selected Sim from
    /// `world.neighbors.inventories` (`0x51e375..0x51e3a7`), resolving each
    /// token's display name via `resolve_name`.
    #[must_use]
    pub fn with_inventory(
        mut self,
        world: &World,
        mut resolve_name: impl FnMut(u32) -> String,
    ) -> Self {
        let selected_guid = self.selected_guid.or_else(|| {
            self.selected
                .and_then(|id| world.objects.get(&id))
                .map(|o| o.guid)
        });
        let Some(guid) = selected_guid else {
            self.inventory.clear();
            return self;
        };
        let neighbor_id = world
            .neighbors
            .slots
            .iter()
            .flatten()
            .find(|n| n.guid == guid)
            .map(|n| n.id);
        let Some(neighbor_id) = neighbor_id else {
            self.inventory.clear();
            return self;
        };
        self.inventory = world
            .neighbors
            .inventories
            .iter()
            .find(|inv| inv.neighbor == neighbor_id)
            .map(|inv| {
                inv.items
                    .iter()
                    .filter(|item| item.count > 0)
                    .map(|item| {
                        let mut name = resolve_name(item.guid);
                        if name.is_empty() {
                            name = format!("0x{:08X}", item.guid);
                        }
                        InventoryEntry {
                            kind: item.kind,
                            guid: item.guid,
                            count: item.count,
                            name,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
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
        if let Some(family) =
            friends_family(world, self.selected, self.residential).filter(|f| f.stale)
        {
            self.cluster_friends = world.count_family_friends(family, known.friendship);
        }
        let selected = self.selected.and_then(|id| world.objects.get(&id));
        self.relations = selected
            .map(|o| relations(world, o.guid, members, known))
            .unwrap_or_default();
        self.superstar = known.superstar;
        self
    }

    /// Populates [`Snapshot::superstar`] and each resident's Superstar / Fame
    /// subpanel fields (`fame_title`, `fame_needs`, `fame_friends_text`) from
    /// `world`, `known`, and `fame` (`0x4cfe2c`, `0x405190`, `0x40520b`,
    /// `0x427237`, `0x52d1af`, `0x52d3e6`).
    #[must_use]
    pub fn with_fame(mut self, world: &World, known: &Acquaintances, fame: &Fame) -> Self {
        self.superstar = known.superstar;
        for resident in &mut self.residents {
            resident.fame_title = fame.level_title(resident.fame_level).to_owned();
            resident.fame_needs = fame.skill_needs(resident.fame_level, &resident.skills);
            let raw_half_stars = friend_star_power(world, resident.guid, known);
            resident.fame_friends_text =
                fame.format_friend_star_power(resident.fame_level, raw_half_stars);
        }
        self
    }

    /// The selected Sim, if one of the household's: what the live panel
    /// counts as someone selected (the original's +0xe8, the selected
    /// Sim's place among the members, matched by neighbor/GUID at
    /// `0x51c5ef` → `0x4e97dc`, `0x4e986d`, `0x4e9608`).
    pub fn selected_resident(&self) -> Option<&Resident> {
        if let Some(guid) = self.selected_guid {
            return self.residents.iter().find(|r| r.guid == guid);
        }
        self.selected.and_then(|s| self.resident(s))
    }

    pub fn resident(&self, object: u16) -> Option<&Resident> {
        self.residents.iter().find(|r| r.object == object)
    }

    /// The clock as the panel writes it (`0x5fe51e`, format `0x6d84a4`
    /// `"%s %2u:%.2u"` with `"AM"` at `0x6d84b4` and `"PM"` at `0x6d84b0`):
    /// `AM  6:30` for single-digit hours and `PM 12:00` for two-digit hours.
    /// Midnight and noon are 12.
    pub fn clock(&self) -> String {
        let hour = self.hour.rem_euclid(24);
        let twelve = match hour % 12 {
            0 => 12,
            h => h,
        };
        let half = if hour < 12 { "AM" } else { "PM" };
        format!("{half} {twelve:2}:{:02}", self.minute.clamp(0, 59))
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
/// titles and the Child Report Card grade strings (`STR# 4097`, `0x403f43`).
#[derive(Debug, Clone, Default)]
pub struct Careers {
    careers: BTreeMap<i16, Vec<Level>>,
    /// Each career's levels' titles, a man's and a woman's, as the job
    /// panel and dialogs show them (+0x54, +0xe0).
    titles: BTreeMap<i16, Vec<[String; 2]>>,
    /// The same from the career's first table (+0x16c, +0x1f8), as a list
    /// of people shows them ([`Careers::list_title`]).
    list_titles: BTreeMap<i16, Vec<[String; 2]>>,
    /// Each career's levels' descriptions (`STR#` entry `4 + 2*k`).
    descriptions: BTreeMap<i16, Vec<String>>,
    /// Each career's track/offer name (`STR#` entry `0` / `1`).
    offers: BTreeMap<i16, String>,
    /// School grade strings (`work.iff STR# 4097`, `"A+"` .. `"F (off to military school)"`).
    grades: Vec<String>,
}

/// The career, in `work.iff`'s order, whose titles the original never
/// reads (0x403f8f): school, which has no `STR#`.
const UNTITLED: usize = 10;

impl Careers {
    /// Reads every `CARR` of the install's `work.iff`, the levels' titles
    /// and the school grade strings from `STR# 4097` (0x403f43). An
    /// install without the file has no careers.
    ///
    /// A career's strings are in the `STR#` of its ID (0x403f26), in US
    /// English; a career without that table keeps none (0x403fec). Its
    /// level `k`'s description is string `4 + 2k` (+0x3b4). The titles the
    /// game shows, a man's and a woman's (+0x54, +0xe0), are strings
    /// `3 + 2k` and `23 + k` of a second table, the `STR#` of the ID plus
    /// 1000 (0x404027..0x404057, 0x40427a..0x40430f). Without that table
    /// they are strings 1000 further on in the first (0x4041d4..0x404269),
    /// which no career has. The first table's own titles go to +0x16c and
    /// +0x1f8 ([`Careers::list_title`]).
    ///
    /// The second tables differ: they spell some titles differently
    /// ("Back-up Musician", "Free Lance Photographer", "Douser",
    /// "Papparazzi"), and the Livin' Large careers' woman's titles are
    /// empty, so the man's shows ([`Careers::title`]).
    ///
    /// The original reads the titles of only the first 11, 16 or 21
    /// careers, by the game's expansion level (0x403f53); here every
    /// career's, which is all 21 of the full install's.
    pub fn load(index: &Index) -> Result<Self> {
        Self::load_with_pool(index, &ArchivePool::new())
    }

    /// Like [`Careers::load`], reusing open archive handles from `pool`.
    pub fn load_with_pool(index: &Index, pool: &ArchivePool) -> Result<Self> {
        let mut out = Self::default();
        let Some((_, file)) = index.named("work.iff").next() else {
            return Ok(out);
        };
        let iff =
            Iff::parse(file.read_with(pool).map_err(Error::Content)?).map_err(Error::Format)?;
        if let Some(chunk) = iff.get(b"STR#", GRADE_STRINGS) {
            let table = StringTable::parse(iff.data(chunk)).map_err(Error::Format)?;
            out.grades = (0..table.entries.len())
                .map_while(|i| {
                    table
                        .get(i, sims_content::ui_graphics::US_ENGLISH)
                        .map(str::to_owned)
                })
                .collect();
        }
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
                let shown = match iff.get(b"STR#", chunk.id.wrapping_add(1000)) {
                    Some(chunk) => {
                        Some(StringTable::parse(iff.data(chunk)).map_err(Error::Format)?)
                    }
                    None => None,
                };
                let title = |i: usize| match &shown {
                    Some(table) => table
                        .get(i, sims_content::ui_graphics::US_ENGLISH)
                        .unwrap_or_default()
                        .to_owned(),
                    None => text(i + 1000),
                };
                let titles = (0..career.levels.len())
                    .map(|k| [title(3 + 2 * k), title(23 + k)])
                    .collect();
                let list_titles = (0..career.levels.len())
                    .map(|k| [text(3 + 2 * k), text(23 + k)])
                    .collect();
                let descriptions = (0..career.levels.len()).map(|k| text(4 + 2 * k)).collect();
                let offer = {
                    let s1 = text(1);
                    if !s1.is_empty() { s1 } else { text(0) }
                };
                out.titles.insert(id, titles);
                out.list_titles.insert(id, list_titles);
                out.descriptions.insert(id, descriptions);
                if !offer.is_empty() {
                    out.offers.insert(id, offer);
                }
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

    /// Sets the school grade strings (`work.iff STR# 4097`), as tests do.
    pub fn insert_grades(&mut self, grades: Vec<String>) {
        self.grades = grades;
    }

    /// The school grade label for `grade` (`person.data[57]`, `0x4034ce`
    /// reading 1-based `grade + 1` from `work.iff STR# 4097`, returning `""`
    /// when out of range).
    pub fn grade(&self, grade: i16) -> &str {
        usize::try_from(grade)
            .ok()
            .and_then(|i| self.grades.get(i))
            .map_or("", String::as_str)
    }

    /// Whether career `career` exists (0x404cb3).
    pub fn has(&self, career: i16) -> bool {
        self.careers.contains_key(&career)
    }

    /// The title of level `level` of career `career` as the game shows it
    /// (0x4033d2, from the second table): a woman's where the level has
    /// one, else the level's; empty for a level without a title. The live
    /// job panel (0x53074c, at 0x530e79) and `$Job` (0x44ca89) read it.
    pub fn title(&self, career: i16, level: i32, female: bool) -> Option<&str> {
        pick_title(&self.titles, career, level, female)
    }

    /// The title from the career's first table (0x4033a4), chosen the same
    /// way. Only a list box of people reads it: 0x52ff7e (vtable 0x66e354,
    /// at 0x530674) fills a row's first column with it, next to a number
    /// and a field of the person, which the port does not draw.
    pub fn list_title(&self, career: i16, level: i32, female: bool) -> Option<&str> {
        pick_title(&self.list_titles, career, level, female)
    }

    /// The description of level `level` of career `career` (`STR#` entry `4 + 2*k`),
    /// used by `$JobDesc` (`0x44c966`).
    pub fn description(&self, career: i16, level: i32) -> Option<&str> {
        self.descriptions
            .get(&career)?
            .get(usize::try_from(level).ok()?)
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }

    /// The career offer/track title for `career` (`STR#` entry `1` / `0`),
    /// used by `$JobOffer` (`0x44c966`).
    pub fn offer(&self, career: i16) -> Option<&str> {
        self.offers.get(&career).map(String::as_str)
    }

    /// Level `level` (from 0) of career `career`, if both exist.
    pub fn level(&self, career: i16, level: i32) -> Option<&Level> {
        if career == 0 {
            return None;
        }
        self.careers.get(&career)?.get(usize::try_from(level).ok()?)
    }
}

/// Superstar fame progression data read from `GameData/fame.iff`
/// (`0x404e74`, `0x52bd10`): 11 levels (0 = 0.0 stars .. 10 = 5.0 stars).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fame {
    /// Panel title (`STR# 1[0]`, `"Fame"`).
    pub title: String,
    /// Level titles (`STR# 1[1..=11]`, `"Nobody"` .. `"Superstar"`).
    pub level_titles: Vec<String>,
    /// Format string when more friend star power is needed (`STR# 1[12]`,
    /// `"%s (Need %d)"`).
    pub need_friends_format: String,
    /// Friend star power needed at each level (`STR# 4[0..11]`).
    pub friend_star_power_needed: Vec<i32>,
    /// Fame score needed at each level (`STR# 5[0..11]`).
    pub score_needed: Vec<i32>,
    /// Skill points (`0..=10`) needed in [`SKILLS`] order at each level
    /// (`STR# 6[0..11]`).
    pub skills_needed: Vec<[i32; 6]>,
}

impl Fame {
    /// Reads `GameData/fame.iff` from `install`. Returns an empty [`Fame`]
    /// on installs without the file (pre-Superstar).
    pub fn load(install: &Install) -> Result<Self> {
        let Ok(path) = install.file("GameData/fame.iff") else {
            return Ok(Self::default());
        };
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let table = |id: u16| -> Result<Vec<String>> {
            let Some(chunk) = iff.get(b"STR#", id) else {
                return Ok(Vec::new());
            };
            let st = StringTable::parse(iff.data(chunk)).map_err(Error::Format)?;
            Ok((0..st.entries.len())
                .map_while(|i| {
                    st.get(i, sims_content::ui_graphics::US_ENGLISH)
                        .map(str::to_owned)
                })
                .collect())
        };
        let panel = table(1)?;
        let friend_needed = table(4)?;
        let score_needed = table(5)?;
        let skills_needed = table(6)?;
        Ok(Self::from_tables(
            &panel,
            &friend_needed,
            &score_needed,
            &skills_needed,
        ))
    }

    /// Parses the four `fame.iff` `STR#` tables (`1`, `4`, `5`, `6`), matching
    /// `0x404e74` and `0x52bd10`.
    pub fn from_tables(
        panel: &[String],
        friend_needed: &[String],
        score_needed: &[String],
        skills_needed: &[String],
    ) -> Self {
        let parse_i32 = |s: &String| s.trim().parse::<i32>().unwrap_or(0);
        let parse_skills = |s: &String| -> [i32; 6] {
            let mut out = [0; 6];
            for (slot, part) in out.iter_mut().zip(s.split(',')) {
                *slot = part.trim().parse::<i32>().unwrap_or(0);
            }
            out
        };
        Self {
            title: panel.first().cloned().unwrap_or_default(),
            level_titles: panel.iter().skip(1).take(11).cloned().collect(),
            need_friends_format: panel.get(12).cloned().unwrap_or_default(),
            friend_star_power_needed: friend_needed.iter().take(11).map(parse_i32).collect(),
            score_needed: score_needed.iter().take(11).map(parse_i32).collect(),
            skills_needed: skills_needed.iter().take(11).map(parse_skills).collect(),
        }
    }

    /// The fame rank title for `level` (`0..=10`), matching `0x52d3e6`
    /// (`0x52c014` initializes the button to level 1 if out of range).
    pub fn level_title(&self, level: i16) -> &str {
        let idx = usize::try_from(level).ok().filter(|&i| i < 11);
        idx.and_then(|i| self.level_titles.get(i))
            .or_else(|| self.level_titles.get(1))
            .map_or("", String::as_str)
    }

    /// Required skill points (`0..=10`) still missing for `level` (`0x405190`):
    /// returns `req` in points where `skills[n] < req * 100`, else `0`.
    pub fn skill_needs(&self, level: i16, skills: &[i16; 6]) -> [i32; 6] {
        let Some(reqs) = usize::try_from(level)
            .ok()
            .filter(|&i| i < 11)
            .and_then(|i| self.skills_needed.get(i))
        else {
            return [0; 6];
        };
        std::array::from_fn(|n| {
            let req = reqs[n];
            if i32::from(skills[n]) < req.saturating_mul(100) {
                req
            } else {
                0
            }
        })
    }

    /// Friend star power needed for `level` (`0x40520b`).
    pub fn friend_star_power_needed(&self, level: i16) -> i32 {
        usize::try_from(level)
            .ok()
            .filter(|&i| i < 11)
            .and_then(|i| self.friend_star_power_needed.get(i).copied())
            .unwrap_or(0)
    }

    /// Formats the friend star power button label (`0x40529c`, `0x404dde`,
    /// `0x52d1af`): `"X.0"` / `"X.5"`, or `"%s (Need %d)"` when below the
    /// current level's required friend star power.
    pub fn format_friend_star_power(&self, level: i16, raw_half_stars: i32) -> String {
        let mut f = raw_half_stars as f32;
        if raw_half_stars > 0 {
            f *= 0.5f32;
        }
        let whole = f as i32;
        let suffix = if (f - whole as f32) != 0.0f32 {
            ".5"
        } else {
            ".0"
        };
        let current = format!("{whole}{suffix}");
        let needed = self.friend_star_power_needed(level);
        // 0x404dde: raw_half_stars * 0.5 >= needed (uses single-precision f32).
        if (raw_half_stars as f32) * 0.5f32 >= needed as f32 {
            current
        } else if self.need_friends_format.contains("%s") && self.need_friends_format.contains("%d")
        {
            self.need_friends_format
                .replacen("%s", &current, 1)
                .replacen("%d", &needed.to_string(), 1)
        } else {
            format!("{current} (Need {needed})")
        }
    }
}

/// Sums the fame half-stars (`word [neighbor + 0x116]`, person data word 81) of
/// all neighborhood characters (`0x423f30`: disk characters or runtime-created
/// `world.characters.made`) in a different non-zero family (`word [neighbor + 0xee]`)
/// who are mutual friends with `guid`'s Sim (`0x4ea6c4 -> 0x427237 -> 0x4271c5`).
pub fn friend_star_power(world: &World, guid: u32, known: &Acquaintances) -> i32 {
    let neighbors = &world.neighbors;
    let Some(viewer) = neighbors.first_of(guid) else {
        return 0;
    };
    let viewer_family = viewer
        .data
        .get(sims_sim::neighbors::FAMILY)
        .copied()
        .unwrap_or(0);
    let mut total = 0i32;
    for record in neighbors.slots.iter().flatten() {
        if record.id == viewer.id {
            continue;
        }
        if !known.characters.contains(&record.guid)
            && !world.characters.made.contains_key(&record.guid)
        {
            continue;
        }
        let family = record
            .data
            .get(sims_sim::neighbors::FAMILY)
            .copied()
            .unwrap_or(0);
        if family == 0 || family == viewer_family {
            continue;
        }
        if sims_sim::neighbors::mutual_friends(viewer, record, known.friendship) {
            total = total.saturating_add(i32::from(record.data.get(FAME).copied().unwrap_or(0)));
        }
    }
    total
}

/// The lot's family: the first whose ID is global 9, as the lot's
/// family panel finds it (`refresh_family_panel` in sims-vm).
fn lot_family(world: &World) -> Option<&sims_sim::neighbors::Family> {
    let id = i32::from(world.globals.get(9).copied().unwrap_or(0));
    (id != 0).then(|| world.families.iter().find(|f| f.id == id))?
}

/// The family whose friends count the control panel displays (`0x4cfcdb`):
/// on residential lots (`0x40745f`), the lot's family (global 9); on
/// non-residential lots, the family named by the selected Sim's person
/// data word 61 (`sims_sim::neighbors::FAMILY`, `[person + 0x606]`),
/// matched linearly against `world.families` (`0x425d69`).
fn friends_family(
    world: &World,
    selected: Option<u16>,
    residential: bool,
) -> Option<&sims_sim::neighbors::Family> {
    if residential {
        return lot_family(world);
    }
    let person = selected.and_then(|id| {
        let obj = world.objects.get(&id)?;
        world
            .people
            .iter()
            .filter_map(|pid| world.objects.get(pid))
            .find(|o| o.guid == obj.guid && o.person.is_some())
            .unwrap_or(obj)
            .person
            .as_ref()
    })?;
    let family_id = i32::from(
        person
            .data
            .get(sims_sim::neighbors::FAMILY)
            .copied()
            .unwrap_or(0),
    );
    world.families.iter().find(|f| f.id == family_id)
}

/// The people `guid`'s Sim has a relationship with, in the relationship
/// panel's order: 0x4e9deb lists the neighbor records, in slot order,
/// that the Sim's own record (the first of its type) has a row toward,
/// and 0x4e9fad sorts them: the household's members first, then people on
/// the lot, then by the daily relationship, highest first, then by their
/// place in the interface's table of person types (0x4ea035 compares the
/// places 0x4e9753 gave, [`Acquaintances::portrait_place`]). A record
/// whose type is not in that table is left out (0x4e9e8c).
///
/// **Not ported:** 0x4e9deb's test of each record (0x423f30, and words
/// of the person on the lot), so a record the original leaves out may be
/// listed. Two records of one type tie completely; the port keeps them
/// in slot order, the original's sort (0x4eb3d1) is not stable.
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
fn pick_title(
    titles: &BTreeMap<i16, Vec<[String; 2]>>,
    career: i16,
    level: i32,
    female: bool,
) -> Option<&str> {
    let [man, woman] = titles.get(&career)?.get(usize::try_from(level).ok()?)?;
    Some(if female && !woman.is_empty() {
        woman
    } else {
        man
    })
}

fn person_data(world: &World, guid: u32) -> Option<&[i16]> {
    world
        .people
        .iter()
        .filter_map(|id| world.objects.get(id))
        .find_map(|o| o.person.as_ref().filter(|_| o.guid == guid))
        .map(|p| p.data.as_slice())
        .or_else(|| world.neighbors.first_of(guid).map(|r| r.data.as_slice()))
}

/// The person `guid`'s Sim's relationship list starts with, in the
/// panel's order ([`relations`]): what a `rel` dialog icon shows
/// (0x436762..0x4367ae takes the first of 0x4e9deb's sorted list).
pub fn first_relation(
    world: &World,
    guid: u32,
    members: &[u32],
    known: &Acquaintances,
) -> Option<u32> {
    relations(world, guid, members, known)
        .first()
        .map(|relation| relation.guid)
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
        let Some(place) = known.portrait_place(world, record.guid) else {
            continue;
        };
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
            place,
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
/// A person's 25 by 25 thumbnail, framed in blue on black: `BMP_` 2004,
/// which the neighborhood's lot card draws for each member (`0x4e9556`
/// reads it through `0x4e9260(person, 0x7d4)`).
pub const THUMBNAIL: u16 = 2004;
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
    thumbnails: BTreeMap<u32, Image>,
    names: BTreeMap<u32, String>,
}

impl Faces {
    /// Reads the faces and the first name of each of `guids` from its
    /// character file. A type with no file, no faces or no name is left
    /// out of what it lacks; a face that does not decode is an error.
    pub fn load(index: &Index, catalog: &Catalog, guids: &[u32]) -> Result<Self> {
        Self::load_with_pool(index, catalog, guids, &ArchivePool::new())
    }

    /// Like [`Faces::load`], reusing open archive handles from `pool` and
    /// decoding character portraits in parallel while inserting in exact
    /// `guids` order.
    pub fn load_with_pool(
        index: &Index,
        catalog: &Catalog,
        guids: &[u32],
        pool: &ArchivePool,
    ) -> Result<Self> {
        type FaceRow = Result<
            Option<(
                u32,
                Option<String>,
                Option<Image>,
                Option<Image>,
                Option<Image>,
            )>,
        >;
        let decode_one = |&guid: &u32| -> FaceRow {
            let Some(object) = catalog.object(guid) else {
                return Ok(None);
            };
            let Some(resource) = index.resources.get(object.resource) else {
                return Ok(None);
            };
            let iff = Iff::parse(resource.read_with(pool).map_err(Error::Content)?)
                .map_err(Error::Format)?;
            let name = first_name(&iff, object.chunk);
            let mood = match iff.get(b"BMP_", MOODS) {
                Some(chunk) => Some(bitmap::bmp(iff.data(chunk)).map_err(Error::Format)?),
                None => None,
            };
            let thumbnail = match iff.get(b"BMP_", THUMBNAIL) {
                Some(chunk) => Some(bitmap::bmp(iff.data(chunk)).map_err(Error::Format)?),
                None => None,
            };
            let face = match iff.get(b"BMP_", FACES) {
                Some(chunk) => {
                    let image = bitmap::bmp(iff.data(chunk)).map_err(Error::Format)?;
                    image.crop(
                        0,
                        0,
                        (image.width / 3).min(FACE[2]),
                        image.height.min(FACE[3]),
                    )
                }
                None => None,
            };
            Ok(Some((guid, name, mood, face, thumbnail)))
        };
        let workers = if guids.len() <= 2 {
            1
        } else {
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
                .clamp(1, 8)
                .min(guids.len())
        };
        let rows: Vec<FaceRow> = if workers <= 1 {
            guids.iter().map(decode_one).collect()
        } else {
            let chunk_size = guids.len().div_ceil(workers);
            std::thread::scope(|scope| {
                let handles: Vec<_> = guids
                    .chunks(chunk_size)
                    .map(|chunk| {
                        scope.spawn(move || chunk.iter().map(decode_one).collect::<Vec<_>>())
                    })
                    .collect();
                let mut out = Vec::with_capacity(guids.len());
                for handle in handles {
                    out.extend(handle.join().expect("faces worker panicked"));
                }
                out
            })
        };
        let mut faces = BTreeMap::new();
        let mut moods = BTreeMap::new();
        let mut thumbnails = BTreeMap::new();
        let mut names = BTreeMap::new();
        for row in rows {
            let Some((guid, name, mood, face, thumbnail)) = row? else {
                continue;
            };
            if let Some(thumbnail) = thumbnail {
                thumbnails.insert(guid, thumbnail);
            }
            if let Some(name) = name {
                names.insert(guid, name);
            }
            if let Some(mood) = mood {
                moods.insert(guid, mood);
            }
            if let Some(face) = face {
                faces.insert(guid, face);
            }
        }
        Ok(Self {
            faces,
            moods,
            thumbnails,
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

    pub fn insert_thumbnail(&mut self, guid: u32, thumbnail: Image) {
        self.thumbnails.insert(guid, thumbnail);
    }

    /// The framed 25 by 25 thumbnail of `guid` ([`THUMBNAIL`]).
    pub fn thumbnail(&self, guid: u32) -> Option<&Image> {
        self.thumbnails.get(&guid)
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
            show_money: true,
            paused: false,
            idle_speed: false,
            residential: true,
            residents: Vec::new(),
            selected: None,
            selected_guid: None,
            family_friends: 0,
            cluster_friends: 0,
            relations: Vec::new(),
            superstar: false,
            house: House::default(),
            inventory: Vec::new(),
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
            portraits: (100..1000).collect(),
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

    /// 0x4e9fad's last tie: two equally liked acquaintances come in the
    /// order of their places in the interface's table of person types
    /// (0x4ea035 compares the places 0x4e9753 gave), not by record. A
    /// type missing from the table is not listed (0x4e9e8c); a type made
    /// while the game runs comes after the startup ones (0x4e8a3a).
    #[test]
    fn equally_liked_acquaintances_tie_on_the_portrait_table_place() {
        let mut world = World::empty();
        world.neighbors.slots = vec![
            Some(record(1, 100, &[(2, 30), (3, 30), (4, 30), (5, 30)])),
            Some(record(2, 200, &[])),
            Some(record(3, 300, &[])),
            Some(record(4, 400, &[])),
            Some(record(5, 500, &[])),
        ];
        let mut known = known();
        // The registry put 300 before 200; 400 has no place; 500 was made.
        known.portraits = vec![100, 300, 200];
        world.characters.made.insert(
            500,
            sims_sim::characters::Character {
                template: 100,
                file: 0,
                strings: BTreeMap::new(),
            },
        );
        let ids: Vec<_> = relations(&world, 100, &[], &known)
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, [3, 2, 5]);
        assert_eq!(known.portrait_place(&world, 500), Some(3));
        assert_eq!(known.portrait_place(&world, 400), None);
        assert_eq!(first_relation(&world, 100, &[], &known), Some(300));
    }

    /// Made types take their places in the order made (0x4265fd →
    /// 0x4e8a3a), not by GUID: two made in one tick share the GUID's high
    /// word (the tick) and can descend in the low (0x4260e1's draw).
    #[test]
    fn made_types_tie_in_the_order_they_were_made() {
        // Made in one tick: first 0x0005_7000, then 0x0005_1000.
        let (first, second) = (0x0005_7000_u32, 0x0005_1000_u32);
        let mut world = World::empty();
        world.neighbors.slots = vec![
            Some(record(1, 100, &[(6, 30), (7, 30)])),
            Some(record(6, first, &[])),
            Some(record(7, second, &[])),
        ];
        let made = |template| sims_sim::characters::Character {
            template,
            file: 0,
            strings: BTreeMap::new(),
        };
        world.characters.made.insert(first, made(100));
        world.characters.made.insert(second, made(100));
        let mut known = known();
        known.characters.extend([first, second]);
        known.portraits = vec![100];
        assert_eq!(known.portrait_place(&world, first), Some(1));
        assert_eq!(known.portrait_place(&world, second), Some(2));
        let ids: Vec<_> = relations(&world, 100, &[], &known)
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(
            ids,
            [6, 7],
            "the first made first, though its GUID is higher"
        );
        assert_eq!(first_relation(&world, 100, &[], &known), Some(first));

        // The order survives the world's encoding, and a save (`Saved`
        // keeps the world's encoding), as the map is written in it.
        let back = World::decode(&world.encode()).unwrap();
        assert_eq!(
            back.characters.made.keys().copied().collect::<Vec<_>>(),
            [first, second]
        );
        assert_eq!(known.portrait_place(&back, second), Some(2));
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
            join: 0,
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
        assert_eq!(snapshot(6, 30, 0).clock(), "AM  6:30");
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
            has_career: false,
            needs: [7; 6],
            child: false,
            school_age: false,
            female: false,
            pet: false,
            pet_kind: None,
            performance: 0,
            school_grade: String::new(),
            fame_score: 0,
            fame_level: 0,
            fame_title: String::new(),
            fame_needs: [0; 6],
            fame_friends_text: String::new(),
            work: Work::default(),
            interests: [0; 19],
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
            has_career: false,
            needs: [0; 6],
            child,
            school_age: child,
            female,
            pet: false,
            pet_kind: None,
            performance: 0,
            school_grade: String::new(),
            fame_score: 0,
            fame_level: 0,
            fame_title: String::new(),
            fame_needs: [0; 6],
            fame_friends_text: String::new(),
            work: Work::default(),
            interests: [0; 19],
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

    #[test]
    fn budget_mode_pet_flag_and_non_residential_family_friends() {
        use sims_formats::saved_objects::{Interaction, Person};
        let idle = Interaction {
            join: 0,
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
        let make_sim = |id: u16, guid: u32, family: i16, kind: i16, selected: bool| {
            let mut data = vec![0; sims_sim::neighbors::PERSON_WORDS];
            data[sims_sim::neighbors::FAMILY] = family;
            data[sims_sim::neighbors::KIND] = kind;
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
                last: idle.clone(),
                queue: Vec::new(),
                object_uses: Vec::new(),
                motive_deltas: Vec::new(),
            };
            sims_sim::Object {
                id,
                guid,
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
                selected,
            }
        };

        let mut world = World::empty();
        world.globals.resize(33, 0);
        world.globals[9] = 1; // lot family = 1
        world.families.push(sims_sim::neighbors::Family {
            id: 1,
            house: 5,
            members: vec![100],
            friends: 4,
            stale: false,
        });
        world.families.push(sims_sim::neighbors::Family {
            id: 2,
            house: 7,
            members: vec![200],
            friends: 9,
            stale: false,
        });
        // Sim 10 (human, family 2, selected) and Sim 11 (pet, family 1).
        world.objects.insert(10, make_sim(10, 200, 2, 1, true));
        world.objects.insert(11, make_sim(11, 100, 1, 2, false));
        world.people = vec![10, 11];

        let members = [200, 100];
        // Global 32 == 0 -> show_money true; pet flag reads data[KIND] > 1.
        let s = Snapshot::read(&world, &members);
        assert!(s.show_money);
        assert!(!s.residents[0].pet);
        assert!(s.residents[1].pet);
        assert_eq!(s.family_friends, 4);

        // Global 32 == 1 -> show_money false (0x4f24fa -> 0x4cfe0a -> 0x53b2a4).
        // Global 32 == 2 -> show_money true (exact equality with 1).
        world.globals[32] = 1;
        assert!(!Snapshot::read(&world, &members).show_money);
        world.globals[32] = 2;
        assert!(Snapshot::read(&world, &members).show_money);
        world.globals[32] = 0;

        // Residential lot reads global 9's family (4) for both cluster and job;
        // non-residential reads selected Sim's person.data[FAMILY] (family 2 -> 9)
        // for cluster_friends (0x4cfcdb), while family_friends stays global 9 (4)
        // for the Job panel (0x4ea8ed, 0x531095).
        let res = Snapshot::read(&world, &members).with_zoning(&world, true, 50);
        assert_eq!((res.family_friends, res.cluster_friends), (4, 4));
        let non_res = Snapshot::read(&world, &members).with_zoning(&world, false, 50);
        assert_eq!((non_res.family_friends, non_res.cluster_friends), (4, 9));

        // Stale non-residential family recomputes cluster_friends via
        // count_family_friends (0x427219), keeping family_friends on global 9.
        world.neighbors.slots = vec![
            Some(record(1, 200, &[(2, 60)])),
            Some(record(2, 300, &[(1, 60)])),
        ];
        world.neighbors.slots[0].as_mut().unwrap().data[sims_sim::neighbors::FAMILY] = 2;
        world.neighbors.slots[1].as_mut().unwrap().data[sims_sim::neighbors::FAMILY] = 3;
        world.families[1].friends = 0;
        world.families[1].stale = true;
        let stale_non_res = Snapshot::read(&world, &members).with_zoning(&world, false, 50);
        assert_eq!(
            (stale_non_res.family_friends, stale_non_res.cluster_friends),
            (4, 1)
        );
        // with_relations also respects residential=false for stale recomputation.
        let mut stale_rel = Snapshot::read(&world, &members);
        stale_rel.residential = false;
        let stale_rel = stale_rel.with_relations(&world, &members, &known());
        assert_eq!(
            (stale_rel.family_friends, stale_rel.cluster_friends),
            (4, 1)
        );

        // Non-residential with nobody selected -> cluster_friends 0, family_friends 4.
        world.objects.get_mut(&10).unwrap().selected = false;
        let nobody = Snapshot::read(&world, &members).with_zoning(&world, false, 50);
        assert_eq!((nobody.family_friends, nobody.cluster_friends), (4, 0));

        // Duplicate GUID with a LOWER object ID appended later in `world.people`
        // (`0x4e9608`): registry scan order wins over ascending object ID.
        let mut lower_dup = make_sim(1, 200, 2, 1, true);
        lower_dup.person.as_mut().unwrap().data[10] = 900;
        world
            .objects
            .get_mut(&10)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[10] = 200;
        world.objects.insert(1, lower_dup);
        world.people.push(1);
        let dup_snap = Snapshot::read(&world, &members);
        assert_eq!(dup_snap.residents[0].object, 10);
        assert_eq!(dup_snap.residents[0].skills[0], 200);
        assert_eq!(dup_snap.selected, Some(1));
        assert_eq!(dup_snap.selected_guid, Some(200));
        assert_eq!(dup_snap.selected_resident().map(|r| r.object), Some(10));

        // When `world.people` is populated, an unregistered person in
        // `world.objects` is NOT surfaced (`0x4e9625..0x4e964c` returns null).
        world.objects.insert(99, make_sim(99, 300, 1, 1, false));
        assert!(first_person_for_guid(&world, 300).is_none());
    }

    #[test]
    fn fame_progression_and_friend_star_power_follow_disassembly() {
        let mut panel: Vec<String> = vec!["Fame".into()];
        panel.extend((0..11).map(|i| format!("Rank{i}")));
        panel.push("%s (Need %d)".into());
        let friend_needed: Vec<String> = ["0", "0", "0", "0", "2", "4", "7", "11", "14", "18", "0"]
            .into_iter()
            .map(String::from)
            .collect();
        let score_needed: Vec<String> = [
            "2", "9", "21", "35", "52", "78", "156", "300", "550", "900", "0",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let skills_needed: Vec<String> = [
            "0,0,0,0,0,0",
            "0,0,1,0,0,1",
            "0,0,2,1,0,2",
            "0,0,3,2,0,3",
            "0,0,4,3,0,4",
            "0,0,6,4,0,4",
            "0,0,6,5,0,6",
            "0,0,7,6,0,7",
            "0,0,8,7,0,8",
            "0,0,10,8,0,9",
            "0,0,0,0,0,0",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let fame = Fame::from_tables(&panel, &friend_needed, &score_needed, &skills_needed);
        assert_eq!(fame.level_title(0), "Rank0");
        assert_eq!(fame.level_title(10), "Rank10");
        // Out of range falls back to initial level 1 title (0x52c014 / 0x52d402).
        assert_eq!(fame.level_title(11), "Rank1");
        assert_eq!(fame.level_title(-1), "Rank1");
        // Skill needs (`0x405190`): compares person skill < req * 100.
        assert_eq!(
            fame.skill_needs(2, &[0, 0, 200, 99, 0, 150]),
            [0, 0, 0, 1, 0, 2]
        );
        // Friend star power formatting (`0x40529c`, `0x404dde`, `0x52d1af`).
        assert_eq!(fame.format_friend_star_power(0, 0), "0.0");
        assert_eq!(fame.format_friend_star_power(4, 3), "1.5 (Need 2)");
        assert_eq!(fame.format_friend_star_power(4, 4), "2.0");
        assert_eq!(fame.format_friend_star_power(4, 5), "2.5");
        assert_eq!(fame.format_friend_star_power(0, -1), "-1.0 (Need 0)");

        // `friend_star_power` (`0x427237`): sums word 81 of mutual-friend
        // neighborhood characters in a different non-zero family.
        let mut world = World::empty();
        let make_neighbor =
            |id: i16, guid: u32, family: i16, fame_lvl: i16, rows: &[(i32, i32)]| {
                let mut n = record(id, guid, rows);
                n.data[sims_sim::neighbors::FAMILY] = family;
                n.data[FAME] = fame_lvl;
                n
            };
        world.neighbors.slots = vec![
            Some(make_neighbor(
                1,
                0x100,
                1,
                2,
                &[(2, 80), (3, 80), (4, 80), (5, 80), (6, 80)],
            )),
            Some(make_neighbor(2, 0x200, 2, 5, &[(1, 60)])),
            Some(make_neighbor(3, 0x300, 1, 7, &[(1, 60)])), // same family
            Some(make_neighbor(4, 0x400, 0, 9, &[(1, 60)])), // family 0
            Some(make_neighbor(5, 0x500, 3, 6, &[(1, 20)])), // one-way (< 50)
            Some(make_neighbor(6, 0x600, 4, 8, &[(1, 80)])), // non-character
        ];
        let known = Acquaintances {
            friendship: 50,
            characters: [0x100, 0x200, 0x300, 0x400, 0x500].into_iter().collect(),
            superstar: true,
            portraits: Vec::new(),
        };
        // Only `star_friend` (5 half-stars) qualifies.
        assert_eq!(friend_star_power(&world, 0x100, &known), 5);

        // 0x427272..0x4272aa reads family and fame directly from each
        // neighborhood record, NOT live person data, and handles duplicate GUIDs
        // independently per record; 0x423f30 also counts runtime-created
        // characters in `world.characters.made`.
        let make_person_obj = |id: u16, guid: u32, family: i16, fame_lvl: i16| {
            use sims_formats::saved_objects::{Interaction, Person};
            let idle = Interaction {
                join: 0,
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
            data[sims_sim::neighbors::FAMILY] = family;
            data[FAME] = fame_lvl;
            sims_sim::Object {
                id,
                guid,
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
                person: Some(Person {
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
                }),
                registry_word: None,
                last_result: false,
                removal_skips_resets: false,
                selected: false,
            }
        };
        world.objects.insert(10, make_person_obj(10, 0x100, 99, 99));
        world.people.push(10);
        world.objects.insert(11, make_person_obj(11, 0x200, 2, 99));
        world.people.push(11);
        // Add a second record with the same GUID `0x200` but its own fame (3),
        // and a runtime-created character `0x700` present only in `world.characters.made` (4).
        world.neighbors.slots[0]
            .as_mut()
            .unwrap()
            .relationships
            .push(sims_sim::neighbors::Row {
                target: 7,
                values: vec![80],
            });
        world.neighbors.slots[0]
            .as_mut()
            .unwrap()
            .relationships
            .push(sims_sim::neighbors::Row {
                target: 8,
                values: vec![80],
            });
        world
            .neighbors
            .slots
            .push(Some(make_neighbor(7, 0x200, 3, 3, &[(1, 70)])));
        world
            .neighbors
            .slots
            .push(Some(make_neighbor(8, 0x700, 4, 4, &[(1, 70)])));
        world.characters.made.insert(
            0x700,
            sims_sim::characters::Character {
                template: 0x100,
                file: 99,
                strings: Default::default(),
            },
        );
        // 5 (record 2) + 3 (duplicate-GUID record 7) + 4 (runtime record 8) = 12.
        assert_eq!(friend_star_power(&world, 0x100, &known), 12);
    }

    #[test]
    fn career_grade_strings_map_index_zero_through_f_and_out_of_range() {
        let mut careers = Careers::default();
        careers.insert_grades(vec![
            "A+".into(),
            "A".into(),
            "A-".into(),
            "B+".into(),
            "B".into(),
        ]);
        assert_eq!(careers.grade(0), "A+");
        assert_eq!(careers.grade(4), "B");
        assert_eq!(careers.grade(-1), "");
        assert_eq!(careers.grade(5), "");
    }
}
