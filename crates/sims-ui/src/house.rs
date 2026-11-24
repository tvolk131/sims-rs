//! What the house panel shows: the lot's five ratings and four facts,
//! worked out from the simulation as the original's 0x416622 works them
//! out each time the panel draws (0x52e49e).

use crate::{Error, Result};
use sims_content::ui_graphics::US_ENGLISH;
use sims_content::{ArchivePool, Catalog, Index, Install, Surfaces, atoi};
use sims_core::{Ext, F32};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_formats::tuning::Constants;
use sims_sim::load::ExpandedWall;
use sims_sim::motives::Curve;
use sims_sim::{FULL_SIZE, HouseUsage, World};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// The ratings in the panel's order (Live.iff `STR#` 133, strings 1 to
/// 5).
pub const RATINGS: [&str; 5] = ["Size", "Furnishings", "Yard", "Upkeep", "Layout"];
pub const SIZE: usize = 0;
pub const FURNISHINGS: usize = 1;
pub const YARD: usize = 2;
pub const UPKEEP: usize = 3;
pub const LAYOUT: usize = 4;

/// What the ratings are worked out with, read from the global file when
/// the lot is made (0x415dc0) and when the game starts (0x424216).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tuning {
    /// The Size rating by square feet per member of the family: the
    /// global file's `STR#` 505, string 0.
    pub size: Curve,
    /// The Furnishings rating by the architecture's value over the
    /// objects': string 1.
    pub furnishings: Curve,
    /// What the outdoor rooms' score is multiplied by for the Yard rating:
    /// the global file's tuning table 4, "yard score multiplier".
    pub yard: F32,
    /// What a dirty object and a broken one take off the Upkeep rating's
    /// count: "upkeep penalty per dirty object" and "... per broken
    /// object", cut toward zero when read (0x42434a, 0x42436c).
    pub dirty: i32,
    pub broken: i32,
}

/// The global file's string table with the two curves, and its tuning
/// table with the house's constants.
const CURVES: u16 = 505;
const CONSTANTS: u16 = 4;
/// The lot holds each curve in a table of 8 points (0x46131d).
const CURVE_POINTS: usize = 8;
/// The name the yard's multiplier is read by, and the original's value
/// where the table lacks it (0x666cc0).
const YARD_NAME: &str = "yard score multiplier";
const YARD_DEFAULT: f32 = 0.33;
/// The upkeep penalties' names, and the original's values where the
/// table lacks them (0x666cbc, 0x666cd0).
const DIRTY_NAME: &str = "upkeep penalty per dirty object";
const DIRTY_DEFAULT: f32 = 6.0;
const BROKEN_NAME: &str = "upkeep penalty per broken object";
const BROKEN_DEFAULT: f32 = 15.0;

impl Default for Tuning {
    /// No curves, and the yard multiplier's default.
    fn default() -> Self {
        Self::from_tables(None, None)
    }
}

impl Tuning {
    /// Reads the curves and the yard multiplier from the install's global
    /// file.
    pub fn load(index: &Index, catalog: &Catalog) -> Result<Self> {
        Self::load_with_pool(index, catalog, &ArchivePool::new())
    }

    /// Like [`Tuning::load`], reusing open archive handles from `pool`.
    pub fn load_with_pool(index: &Index, catalog: &Catalog, pool: &ArchivePool) -> Result<Self> {
        let Some(global) = index.resources.get(catalog.global) else {
            return Ok(Self::default());
        };
        let iff =
            Iff::parse(global.read_with(pool).map_err(Error::Content)?).map_err(Error::Format)?;
        // By its ID and a flags' high byte of 0, as the original's string
        // loader looks it up (0x415dd6, through 0x46ff01 and 0x41bab1).
        let curves = iff
            .get(b"STR#", CURVES)
            .map(|chunk| StringTable::parse_chunk(iff.data(chunk), iff.resource_flags(chunk)))
            .transpose()
            .map_err(Error::Format)?;
        let constants = iff
            .get(b"FCNS", CONSTANTS)
            .map(|chunk| Constants::parse(iff.data(chunk)))
            .transpose()
            .map_err(Error::Format)?;
        Ok(Self::from_tables(curves.as_ref(), constants.as_ref()))
    }

    /// The tuning from the curves' string table and the constants' table.
    /// A curve that is empty, or whose table is missing, has no points
    /// and weighs everything 0, as the original's does (0x4167de; a
    /// missing table skips both curves, 0x415ddb). A string missing from
    /// a table that is there does the same here; the original fetches it
    /// without a check (0x415df9) and its scanner crashes on the null it
    /// gets (0x415e02, 0x461435). The install's table has both. The curve's text is its bytes up to the first zero byte
    /// (0x4703ad), each read as it is. **Unverified:** the points are read by
    /// `sims_sim::motives::Curve::parse`, which the motives' curves share;
    /// where text is not well formed (a point such as `(100;90 coins)`)
    /// it drops a point that the original's scanner (0x4614a2) reads,
    /// and it takes the reciprocal of the points' span whole, where the
    /// original rounds the span to single precision first (0x4eedf5,
    /// 0x46140a to 0x461416), which can differ for a wide span.
    /// The install's curves are well formed and narrow. The original looks names
    /// up ignoring the case of A to Z (0x4131f4) and takes the default
    /// for a missing one (0x413271).
    pub fn from_tables(curves: Option<&StringTable>, constants: Option<&Constants>) -> Self {
        // The original copies the string's bytes as a C string
        // (0x4703ad), so a zero byte inside a length-prefixed string ends
        // it, and its scanner (0x461435) reads those bytes as they are: a
        // byte order mark is two bytes that are not a point, not a change
        // of encoding.
        let curve = |n: usize| {
            let text: String = curves
                .and_then(|t| t.entry(n, US_ENGLISH))
                .map(|entry| {
                    let bytes = entry.raw_text.split(|&b| b == 0).next().unwrap_or(&[]);
                    bytes.iter().copied().map(char::from).collect()
                })
                .unwrap_or_default();
            Curve::parse(0, &text, CURVE_POINTS)
        };
        let constant = |name: &str, default: f32| {
            constants
                .and_then(|table| {
                    table
                        .entries
                        .iter()
                        .find(|entry| entry.name.eq_ignore_ascii_case(name))
                })
                .map_or(default, |entry| entry.value)
        };
        let whole = |value: f32| F32::from_f32(value).ext().to_i32_truncate();
        Self {
            size: curve(0),
            furnishings: curve(1),
            yard: F32::from_f32(constant(YARD_NAME, YARD_DEFAULT)),
            dirty: whole(constant(DIRTY_NAME, DIRTY_DEFAULT)),
            broken: whole(constant(BROKEN_NAME, BROKEN_DEFAULT)),
        }
    }
}

/// What the house panel knows of the lot's object types, by GUID, read
/// from the install's definitions as the lot's objects show up
/// ([`Kinds::learn`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kinds {
    /// Which types the Upkeep rating counts (0x4452eb): those whose
    /// definition has room flags, function flags or a build mode type
    /// (definition +0x4e, +0x50 and +0x8a: OBJD words 37, 38 and 67 after
    /// its version) that are not 0. A type not read counts for nothing.
    pub counted: BTreeMap<u32, bool>,
    /// What the lot's values read of each type ([`Type`]). A type not
    /// read is passed over.
    pub types: BTreeMap<u32, Type>,
}

/// The definition words the lot's values read of a type (0x444cb9,
/// 0x43219a, 0x4354d5), counted in 16-bit words from the definition's
/// start, its version included, as the original holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Type {
    /// Word 9 (+0x12): 2 for a person; 7 for a type the scan passes over.
    pub kind: i16,
    /// Word 10 (+0x14): the multi-tile group, 0 for none.
    pub group: i16,
    /// Word 69 (+0x8a, the build mode type) of the type's catalog master
    /// (0x45728d): the type itself outside a group, else the first
    /// definition of its file, in resource order, with the same group and
    /// word 11 -1. Of each ID it reads the definition with flags' high
    /// byte 0, once, and not one with word 16 set, as the original
    /// registers the file's definitions (0x43b9dc to 0x43ba0b; 0x43ba7b,
    /// through 0x41bab1; 0x43baba) and its search walks them (0x43ae36 to
    /// 0x43ae5f). None when the group has no
    /// master, where the original reads through a null pointer.
    pub build: Option<i16>,
}

/// Definition words the lot's values and the Upkeep count read.
mod word {
    pub const KIND: usize = 9;
    pub const GROUP: usize = 10;
    pub const GROUP_INDEX: usize = 11;
    /// Not 0 on a definition the object catalog does not register
    /// (0x43baba, with the catalog's flag set: 0x413f7d, 0x41415f,
    /// 0x43aee7).
    pub const UNREGISTERED: usize = 16;
    /// Room flags, function flags and the build mode type (+0x4e, +0x50,
    /// +0x8a).
    pub const COUNTED: [usize; 3] = [39, 40, 69];
    pub const BUILD: usize = 69;
}

/// The length the original fills a definition out to (0x431759).
const DEFINITION_BYTES: usize = 216;

impl Kinds {
    /// Reads the definition of every object type in `world` not read
    /// yet; a person type made while the game runs reads its template's.
    /// A type the catalog lacks, or whose file or definition cannot be
    /// read, counts for nothing and is passed over by the values. The
    /// definition is read as the original holds it ([`Kinds::words`]).
    pub fn learn(&mut self, world: &World, index: &Index, catalog: &Catalog) {
        let mut files: BTreeMap<usize, Option<Iff>> = BTreeMap::new();
        for object in world.objects.values() {
            if self.counted.contains_key(&object.guid) {
                continue;
            }
            // A type Make New Character made is a copy of its template's
            // definition but for its GUIDs (bytes 0x1c and 0x8c).
            let guid = world.characters.definition(object.guid);
            let read = catalog.object(guid).and_then(|definition| {
                let iff = files.entry(definition.resource).or_insert_with(|| {
                    let bytes = index.resources.get(definition.resource)?.read().ok()?;
                    Iff::parse(bytes).ok()
                });
                let iff = iff.as_ref()?;
                let words = Self::words(iff.data(iff.get(b"OBJD", definition.chunk)?));
                let master = if words[word::GROUP] == 0 {
                    Some(words)
                } else {
                    Self::master(iff, words[word::GROUP])?
                };
                Some((words, master))
            });
            let counted = read.is_some_and(|(words, _)| Self::counted_words(&words));
            self.counted.insert(object.guid, counted);
            if let Some((words, master)) = read {
                self.types.insert(
                    object.guid,
                    Type {
                        kind: words[word::KIND],
                        group: words[word::GROUP],
                        build: master.map(|m| m[word::BUILD]),
                    },
                );
            }
        }
    }

    /// Whether a definition's bytes make its objects count for Upkeep.
    pub fn counts(definition: &[u8]) -> bool {
        Self::counted_words(&Self::words(definition))
    }

    /// The catalog master of group `group` in `iff` (see [`Type::build`]):
    /// its first definition, in resource order, with that group and word
    /// 11 -1, of each ID's definition with flags' high byte 0, each ID
    /// once, less those with word 16 not 0, which the original does not
    /// register (0x43baba to 0x43badb). An ID with no definition of high
    /// byte 0 is passed over; the original reads through the null handle
    /// it gets (0x43ba7b, 0x431759, 0x41fff6). No definition of the
    /// install has another high byte. None inside when there is no
    /// master; None when the file's resource map cannot be used.
    fn master(iff: &Iff, group: i16) -> Option<Option<[i16; DEFINITION_BYTES / 2]>> {
        let mut seen = BTreeSet::new();
        Some(
            iff.resource_order(b"OBJD")
                .ok()?
                .into_iter()
                .filter(|chunk| seen.insert(chunk.id))
                .filter_map(|chunk| iff.get(b"OBJD", chunk.id))
                .map(|chunk| Self::words(iff.data(chunk)))
                .filter(|other| other[word::UNREGISTERED] == 0)
                .find(|other| other[word::GROUP] == group && other[word::GROUP_INDEX] == -1),
        )
    }

    fn counted_words(words: &[i16]) -> bool {
        word::COUNTED.iter().any(|&w| words[w] != 0)
    }

    /// A definition's words as the original holds them (0x431759, as
    /// `sims-vm` reads one): a version 136 or 137 definition shorter than
    /// 216 bytes and odd has its last byte cleared, and every definition
    /// is filled out with zeros to 216 bytes, so a word cut in half keeps
    /// its low byte. (The original also writes 138 over those versions,
    /// which no word read here sees.)
    pub fn words(definition: &[u8]) -> [i16; DEFINITION_BYTES / 2] {
        let mut bytes = definition.to_vec();
        let version = bytes.first_chunk::<4>().map(|&b| u32::from_le_bytes(b));
        if matches!(version, Some(136 | 137))
            && bytes.len() < DEFINITION_BYTES
            && !bytes.len().is_multiple_of(2)
            && let Some(last) = bytes.last_mut()
        {
            *last = 0;
        }
        bytes.resize(bytes.len().max(DEFINITION_BYTES), 0);
        let mut words = [0; DEFINITION_BYTES / 2];
        for (word, pair) in words.iter_mut().zip(bytes.as_chunks::<2>().0) {
            *word = i16::from_le_bytes(*pair);
        }
        words
    }
}

/// An object's data words the Upkeep scan reads (object +0x68 and
/// +0x98): broken when the first is not 0, dirty when the second is above
/// 0.
const BROKEN: usize = 15;
const DIRTY: usize = 39;

/// The lot's size class, by the side of its square (`World::size`, global
/// 23): under 40 tiles small, under 50 medium, otherwise large
/// (0x4166c0), as Live.iff `STR#` 138 names them.
pub const LOT_SIZES: [&str; 3] = ["Small", "Medium", "Large"];

/// The house panel's ratings and facts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct House {
    /// The ratings in [`RATINGS`] order, as the original's accessors
    /// return them (0x4167af, 0x416851, 0x41689a, 0x4168d4, 0x41691c);
    /// the panel clamps them to 0 to 100. `None` for a rating not ported:
    /// the panel draws its empty bar.
    pub ratings: [Option<i32>; 5],
    /// The indoor rooms' area.
    pub square_feet: i32,
    pub bedrooms: i32,
    pub bathrooms: i32,
    /// An index into [`LOT_SIZES`].
    pub lot: usize,
    /// The lot's values, which the Furnishings rating weighs.
    pub values: Values,
}

impl House {
    /// Works the panel's numbers out from `world` (0x416622).
    ///
    /// - It walks the rooms by ID and counts those that are indoors
    ///   (0x46876f) and still marked to be counted (room +0x34,
    ///   [`sims_sim::rooms::Room::dirty`]); a room the last update
    ///   counted and left unmarked is left out. Each such room adds its
    ///   half tiles halved, rounded toward zero (0x46810a), times 9 to
    ///   the square feet; it is a bedroom if it counted an object of
    ///   category 4 (room +0x58) and a bathroom if one of 5 to 7 (+0x5c).
    /// - Size is the square feet over the lot family's members (global
    ///   9's family, lot +0x10), through the size curve, or 0 with no
    ///   members (0x4167af).
    /// - Yard is the outdoor rooms' summed score (the room manager's
    ///   +0x40) times the yard multiplier, kept to 0 to 100 (0x41689a).
    /// - Upkeep (0x4168d4) goes through every object on the lot
    ///   (0x4452eb). Each broken one (data word 15 not 0) adds the broken
    ///   penalty, and each other dirty one (data word 39 above 0) the
    ///   dirty penalty, whether or not it is counted; an object counts if
    ///   its type does ([`Kinds`]). The rating is the count less the
    ///   penalties over the count, kept to 0 to 1, times 100; 0 when
    ///   nothing counts.
    /// - Each rating is cut to a whole number toward zero (`__ftol`), and
    ///   the arithmetic is single precision, as the original's FPU is
    ///   set.
    ///
    /// - Furnishings (0x416851) weighs the lot's values as the simulation
    ///   keeps them ([`Values::stored`]): see [`furnishings`].
    /// - Layout (0x4166ee..0x41674f) weighs the house's usage counts
    ///   ([`World::usage`]): see [`layout`].
    pub fn read(world: &World, tuning: &Tuning, kinds: &Kinds) -> Self {
        let mut house = Self::default();
        for room in world.rooms.rooms.values() {
            if room.outdoors || !room.dirty {
                continue;
            }
            let tiles = room.half_tiles / 2;
            house.square_feet = house.square_feet.wrapping_add(tiles.wrapping_mul(9));
            if room.category_5_to_7 > 0 {
                house.bathrooms += 1;
            }
            if room.category_4 > 0 {
                house.bedrooms += 1;
            }
        }
        house.lot = match world.size {
            ..40 => 0,
            40..50 => 1,
            _ => 2,
        };
        let members = lot_family_members(world);
        house.ratings[SIZE] = Some(if members == 0 {
            0
        } else {
            let per = (Ext::from_i32(house.square_feet) / Ext::from_i32(members)).single();
            tuning.size.weight(per.to_f32()).to_i32_truncate()
        });
        let yard = (world.rooms.outdoor_sum.ext() * tuning.yard.ext()).single();
        let hundred = Ext::from_i32(100);
        let yard = if yard.partial_cmp(&Ext::ZERO).is_some_and(Ordering::is_ge) {
            yard
        } else {
            Ext::ZERO
        };
        let yard = if yard.partial_cmp(&hundred) == Some(Ordering::Greater) {
            hundred
        } else {
            yard
        };
        house.ratings[YARD] = Some(yard.to_i32_truncate());
        house.ratings[UPKEEP] = Some(upkeep(world, tuning, kinds));
        house.values = Values::stored(world);
        house.ratings[FURNISHINGS] = Some(furnishings(house.values, &tuning.furnishings));
        house.ratings[LAYOUT] = Some(layout(&world.usage));
        house
    }
}

/// The Upkeep rating (0x4452eb, 0x4168d4), in 32-bit sums that wrap.
fn upkeep(world: &World, tuning: &Tuning, kinds: &Kinds) -> i32 {
    let (mut count, mut penalty) = (0i32, 0i32);
    for object in world.objects.values() {
        let word = |n: usize| object.data.get(n).copied().unwrap_or(0);
        if word(BROKEN) != 0 {
            penalty = penalty.wrapping_add(tuning.broken);
        } else if word(DIRTY) > 0 {
            penalty = penalty.wrapping_add(tuning.dirty);
        }
        if kinds.counted.get(&object.guid).copied().unwrap_or(false) {
            count = count.wrapping_add(1);
        }
    }
    if count == 0 {
        return 0;
    }
    let share = (Ext::from_i32(count.wrapping_sub(penalty)) / Ext::from_i32(count)).single();
    // `jae` fails for an unordered compare, `jbe` holds for one.
    let share = if share.partial_cmp(&Ext::ZERO).is_some_and(Ordering::is_ge) {
        share
    } else {
        Ext::ZERO
    };
    let one = Ext::from_i32(1);
    let share = if share.partial_cmp(&one) == Some(Ordering::Greater) {
        one
    } else {
        share
    };
    (share * Ext::from_i32(100)).single().to_i32_truncate()
}

/// How many members the lot's family has (0x41024e): the first family
/// whose ID is global 9 (0x425d69, which looks for 0 too), or none.
fn lot_family_members(world: &World) -> i32 {
    let id = i32::from(world.globals.get(9).copied().unwrap_or(0));
    world
        .families
        .iter()
        .find(|f| f.id == id)
        .map_or(0, |f| i32::try_from(f.members.len()).unwrap_or(i32::MAX))
}

/// What the architecture's value (0x47b006) counts walls, floors and
/// the pool at, read from the install when the game starts
/// ([`Prices::load`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prices {
    /// The price of a wall by its style (the table at 0x6e5838). Build
    /// mode's catalog fills six entries from `Build.iff` `STR#` 129
    /// (0x4cd149): its entries 1 to 5 give styles 1, 2, 13, 12 and 14, and
    /// entry 1 style 255 too (0x4cd48d). Every other style costs 0.
    pub styles: BTreeMap<u16, i32>,
    /// The price of a pool tile (0x6c5ff8): `STR#` 133 entry 1 (0x4cd757).
    pub pool: i32,
    /// The price of a floor pattern by its catalog key (0x4a1439).
    pub floors: BTreeMap<u16, i32>,
    /// The price of a wall pattern by its catalog key (0x4c63cf).
    pub walls: BTreeMap<u16, i32>,
}

/// `Build.iff`'s string tables: the wall styles, the built-in floors and
/// wall patterns, the pool, and the prices a pattern file without its own
/// costs (0x4cd453, 0x4a0167, 0x4c4e8e, 0x4cd71b, 0x4a0189, 0x4c4ea9).
const STYLE_TABLE: u16 = 129;
const FLOOR_TABLE: u16 = 130;
const WALL_TABLE: u16 = 131;
const POOL_TABLE: u16 = 133;
const FLOOR_DEFAULT_TABLE: u16 = 134;
const WALL_DEFAULT_TABLE: u16 = 135;
/// The styles `STR#` 129's entries 1 to 5 price (0x4cd488 to 0x4cd5da).
const PRICED_STYLES: [u16; 5] = [1, 2, 13, 12, 14];
/// The style entry 1 also prices (0x4cd48d).
const SECOND_STYLE: u16 = 255;
/// The built-in floors' keys, 1 to 29 (0x4a01c5 to 0x4a02a8), and the
/// built-in wall patterns', 0 to 255 less the plug-ins' range 31 to 247
/// (0x4c4ee5 to 0x4c50fd).
const BUILTIN_FLOORS: std::ops::RangeInclusive<u16> = 1..=29;

impl Prices {
    /// Reads the prices from the install's `GameData/Build.iff` and the
    /// pattern catalogs. A missing file or table prices what it would
    /// have priced at 0. **Not ported:** the lookup (0x47028b) tries a
    /// resource of type `'CST'` (0x00435354, the chunk tag `\0CST` on
    /// disk) with the same ID when the `STR#` is missing, and reads it by
    /// another path (0x470178); the install's files have no such chunk.
    /// Nor is `Build.iff` looked for inside a mounted archive, which the
    /// original's file opener can read it from (0x4cd1bd, 0x5b9520 to
    /// 0x5b9550); the install has it loose.
    pub fn load(install: &Install, surfaces: &Surfaces) -> Self {
        let tables: BTreeMap<u16, StringTable> = install
            .file("GameData/Build.iff")
            .ok()
            .and_then(|path| Iff::parse(std::fs::read(path).ok()?).ok())
            .map(|iff| {
                [
                    STYLE_TABLE,
                    FLOOR_TABLE,
                    WALL_TABLE,
                    POOL_TABLE,
                    FLOOR_DEFAULT_TABLE,
                    WALL_DEFAULT_TABLE,
                ]
                .into_iter()
                .filter_map(|id| {
                    // By its ID and a flags' high byte of 0 (0x4cd45f,
                    // through 0x46ff01 and 0x41bab1).
                    let chunk = iff.get(b"STR#", id)?;
                    Some((
                        id,
                        StringTable::parse_chunk(iff.data(chunk), iff.resource_flags(chunk))
                            .ok()?,
                    ))
                })
                .collect()
            })
            .unwrap_or_default();
        Self::from_tables(&tables, surfaces)
    }

    /// The prices from `Build.iff`'s string tables by ID and the pattern
    /// catalogs. Each entry of a table is three strings, the first its
    /// price, read as C's `atoi` reads its bytes; the strings are looked up with
    /// the 1-based accessor (0x4704fa), so entry `n` is string `3n - 3`.
    ///
    /// The strings are US English's (language 1), as everything here
    /// reads; the original takes the game's language (0x47028b,
    /// 0x470256), and some tables price a style differently in another
    /// language, so a game in another language would price by those.
    pub fn from_tables(tables: &BTreeMap<u16, StringTable>, surfaces: &Surfaces) -> Self {
        let price = |table: u16, entry: u16| -> Option<i32> {
            let at = usize::from(entry).checked_sub(1)? * 3;
            Some(atoi(&tables.get(&table)?.entry(at, US_ENGLISH)?.raw_text))
        };
        let mut styles: BTreeMap<u16, i32> = PRICED_STYLES
            .iter()
            .zip(1..)
            .map(|(&style, entry)| (style, price(STYLE_TABLE, entry).unwrap_or(0)))
            .collect();
        styles.insert(SECOND_STYLE, styles[&PRICED_STYLES[0]]);
        // A style's or the pool's string, or its whole table, missing
        // costs 0 here; the original reads it without a check (0x4cd4d3,
        // 0x470531 returning null; a missing table's failed lookup is
        // passed over at 0x4cd45f and 0x4cd727) and crashes in `atoi`
        // (0x4cdace, 0x635fbe). The install's tables have them all.
        let pool = price(POOL_TABLE, 1).unwrap_or(0);
        // A built-in pattern whose entry is missing costs 0 (0x4a01eb,
        // 0x4c4f1e); a plug-in without a price of its own costs the
        // default (0x4a09ac, 0x4c58cc).
        let mut floors: BTreeMap<u16, i32> = BTreeMap::new();
        for key in BUILTIN_FLOORS {
            floors.insert(key, price(FLOOR_TABLE, key).unwrap_or(0));
        }
        let mut walls: BTreeMap<u16, i32> = BTreeMap::new();
        for key in (0..=30).chain(248..=255) {
            walls.insert(key, price(WALL_TABLE, key).unwrap_or(0));
        }
        let plug_ins = |catalog: &sims_content::SurfaceCatalog,
                        default: i32,
                        prices: &mut BTreeMap<u16, i32>| {
            for file in catalog.files.iter().skip(1) {
                for &key in file.keys.values() {
                    prices.insert(key, file.price.unwrap_or(default));
                }
            }
        };
        let floor_default = price(FLOOR_DEFAULT_TABLE, 1).unwrap_or(0);
        let wall_default = price(WALL_DEFAULT_TABLE, 1).unwrap_or(0);
        plug_ins(&surfaces.floors, floor_default, &mut floors);
        plug_ins(&surfaces.walls, wall_default, &mut walls);
        Self {
            styles,
            pool,
            floors,
            walls,
        }
    }
}

/// The lot's two values, which the Furnishings rating weighs. The
/// original keeps them in its simulator (+0xa0 and +0xa4, saved and
/// loaded in `SIMI`, 0x46d1ad and 0x46d1bd), and the panel reads them
/// there (0x41685f, 0x41687e). Its lot statistics (0x416085) work them
/// out ([`Values::read`]) and store them (0x416118, 0x416126): on going
/// to live mode (0x4cea9e through 0x416534), so after a load, at the
/// day's turn (0x41647e, signal 0xdc), from the budget dialog (0x50206e
/// through 0x416558, which stores them only when the simulator's word
/// +0x36 is not 0, 0x41655b) and before the house is written (0x41621f).
/// So an object that breaks or is mended shows at the next of those.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Values {
    /// What the objects that are valued are worth.
    pub objects: i32,
    /// What the architecture is worth ([`architecture`]), plus every
    /// other object the scan does not pass over whose master's build mode
    /// type is not 0 or 4.
    pub architecture: i32,
}

/// An object's data words the values read: its value (+0x9c), and its
/// flags (+0xa0), whose bit 0x10 keeps it from being valued (0x4354d8).
const VALUE: usize = 41;
const FLAGS: usize = 43;
const NOT_VALUED: i16 = 0x10;
/// Definition word 9 of a person, and of a type the scan passes over
/// (0x4354e7, 0x44508a).
const PERSON: i16 = 2;
const SKIPPED: i16 = 7;

impl Values {
    /// The values as the simulation keeps them
    /// ([`sims_sim::Budget::objects_value`] and `architecture_value`),
    /// which the panel reads as the original's does (0x416851). A
    /// checkpoint keeps them with the rest of the world.
    ///
    /// The simulation stores the objects' value at the day's turn (in
    /// `sims-vm`) and neither value elsewhere: the architecture's stays
    /// what the house was saved with, past any day's turn, and going to
    /// live mode stores neither, so a house saved with values that are
    /// out of date shows the objects' until the day turns and the
    /// architecture's for good. A known gap in the simulation, not the
    /// original's behaviour; [`Values::read`] is the sum it would store.
    #[must_use]
    pub fn stored(world: &World) -> Self {
        Self {
            objects: world.budget.objects_value,
            architecture: world.budget.architecture_value,
        }
    }

    /// The object scan (0x444cb9) and the architecture's value (0x47b006),
    /// as the lot's statistics add them up. The objects are read in the
    /// lot's order, each in 32-bit sums that wrap:
    ///
    /// - A tile of a multi-tile group that is not its lead, and an object
    ///   of type 7, are passed over.
    /// - One that stands on the lot (0x432595, 0x422632: its tile, as a
    ///   signed byte, at least 1 and below the lot's size less 1 each way,
    ///   as `sims-vm` reads it) and is valued adds its value to the
    ///   objects'. Valued (0x4354d5): not with flag 0x10 in data word 43,
    ///   and then a person (whatever its master), or any other type whose
    ///   master's build mode type is 0; a type other than a person without
    ///   a master is not valued.
    /// - Any other adds its value to the architecture's when its master's
    ///   build mode type is neither 0 nor 4 (0x445190 to 0x445223).
    /// - An object's value (0x43219a) is its data word 41, or for a tile
    ///   of a group its lead's (none when it has no group record), halved
    ///   toward zero when data word 15 is not 0 on it or, for a group, on
    ///   any tile from the lead along the chain.
    ///
    /// The original's pass also sets the lot's globals 22, 23 (its size,
    /// 0x416132 to 0x416137) and 37 and two of the family's flags
    /// (0x4160e5, 0x4160f9), and puts right a slot word of 0 on an object
    /// it asks (0x43264c). Those are the simulation's: it keeps the
    /// globals and the slot word, and not the family's flags (see
    /// `sims-vm`'s day's turn). Nothing here changes the world. A type
    /// [`Kinds`] has not read is passed over.
    ///
    /// The panel does not run this: it reads the stored values
    /// ([`Values::stored`]). This is the sum for the simulation to store
    /// when it takes the architecture's value over.
    pub fn read(world: &World, kinds: &Kinds, prices: &Prices) -> Self {
        let mut values = Self {
            objects: 0,
            architecture: architecture(world, prices),
        };
        let size = i32::from(world.size);
        for &id in &world.order {
            let Some(object) = world.objects.get(&id) else {
                continue;
            };
            let Some(ty) = kinds.types.get(&object.guid) else {
                continue;
            };
            let lead = object
                .multipart
                .as_ref()
                .is_none_or(|part| part.parent == 0 || part.parent as u16 == id);
            if (ty.group != 0 && !lead) || ty.kind == SKIPPED {
                continue;
            }
            let [a, b, _] = World::tile(object.position);
            let inside = |n: i8| n >= 1 && i32::from(n) < size - 1;
            let on_lot = inside(a) && inside(b);
            let flags = object.data.get(FLAGS).copied().unwrap_or(0);
            let valued = flags & NOT_VALUED == 0 && (ty.kind == PERSON || ty.build == Some(0));
            let value = i32::from(value(world, ty, id));
            if on_lot && valued {
                values.objects = values.objects.wrapping_add(value);
            } else if !matches!(ty.build, Some(0 | 4) | None) {
                values.architecture = values.architecture.wrapping_add(value);
            }
        }
        values
    }
}

/// The lot arrays' side, whatever the lot's size.
const FULL: i32 = FULL_SIZE as i32;

/// An object's value as the statistics count it (0x43219a).
fn value(world: &World, ty: &Type, id: u16) -> i16 {
    let word = |id: u16, n: usize| {
        world
            .objects
            .get(&id)
            .and_then(|o| o.data.get(n).copied())
            .unwrap_or(0)
    };
    let (value, halved) = if ty.group == 0 {
        (word(id, VALUE), word(id, BROKEN) != 0)
    } else {
        let Some(part) = world.objects.get(&id).and_then(|o| o.multipart.as_ref()) else {
            return 0;
        };
        let lead = if part.parent == 0 {
            id
        } else {
            part.parent as u16
        };
        let mut halved = false;
        let mut tile = lead;
        for _ in 0..=world.objects.len() {
            halved |= word(tile, BROKEN) != 0;
            match world.next_parts.get(&tile) {
                Some(&next) if next != 0 => tile = next,
                _ => break,
            }
        }
        (word(lead, VALUE), halved)
    };
    if halved { value / 2 } else { value }
}

/// The lot arrays the architecture's value reads: each level's tile
/// flags, floors and walls, and the pool.
const TILE_FLAGS: [u16; 2] = [8, 108];
const FLOOR_ARRAY: [u16; 2] = [1, 101];
const WALL_ARRAY: [u16; 2] = [2, 102];
const POOL_ARRAY: u16 = 9;
/// A tile flag that keeps a tile out of the statistics (0x47b067).
const UNCOUNTED: u8 = 0x20;
/// The floor word of a tile split by a diagonal wall, whose two halves'
/// floors the wall cell holds.
const SPLIT_FLOOR: u16 = 0xff;
/// The wall styles whose walls also count their patterns (0x47b0d3).
const PATTERNED: [u16; 4] = [1, 7, 8, 9];
/// A pool byte below this is a pool tile (0x47b1be).
const POOL_TILE: u8 = 0x10;

/// The architecture's value (0x47b006): over the tiles of levels 1 and 2
/// inside the lot's size, less those with tile flag 0x20.
///
/// - **Walls:** the tile's wall cell, as the wall service hands it out
///   (0x4795c5, [`ExpandedWall`]). From its lowest wall segment, then
///   each higher one below 0x20 (so the ascending diagonal, 0x20, counts
///   only when it is the lowest), the segment's style adds its price to a
///   sum that is halved, toward zero, at the end: a wall between two
///   tiles is counted from both. A wall of style 1, 7, 8 or 9 also adds
///   its pattern's price, whole: the side facing the tile for a straight
///   wall, the descending diagonal's second pattern, or the ascending
///   one's first. The style of a segment is the cell's own for its
///   west and north walls and both diagonals, and its east and south
///   neighbours' for those sides (as [`ExpandedWall::styles`]); a side
///   whose neighbour does not own the wall, where the original reads
///   whatever its stack held, costs 0 here (**unverified**: a lot's cells
///   mirror their neighbours' walls).
/// - **Floors:** the tile's floor pattern adds its price. A tile split
///   by a diagonal (floor 0xff) with a diagonal wall adds half the price,
///   toward zero, of each of its halves' floors, which the wall cell keeps
///   in its west style and west pattern words.
/// - **Pool:** on level 1, a pool byte below 0x10 adds the pool's price.
///
/// A pattern or style without a price costs 0, as a key the catalog
/// lacks does (0x4a145d). A level whose tile flags, floors or walls are
/// missing (or not the size the lot's arrays are) adds nothing, and a
/// missing pool array leaves out only the pool's price; the other level
/// and the rest of level 1 still count.
pub fn architecture(world: &World, prices: &Prices) -> i32 {
    let size = usize::try_from(world.size)
        .unwrap_or(0)
        .min(FULL_SIZE as usize);
    let array = |id: u16, stride: usize| {
        world
            .arrays
            .get(&id)
            .filter(|a| a.stride == stride && a.data.len() == (FULL * FULL) as usize * stride)
            .map(|a| a.data.as_slice())
    };
    let (mut walls_sum, mut rest) = (0i32, 0i32);
    let floor_price = |key: u16| prices.floors.get(&key).copied().unwrap_or(0);
    for level in 0..2 {
        let (Some(flags), Some(floors), Some(walls)) = (
            array(TILE_FLAGS[level], 1),
            array(FLOOR_ARRAY[level], 2),
            array(WALL_ARRAY[level], WALL_CELL),
        ) else {
            continue;
        };
        for b in 0..size {
            for a in 0..size {
                let cell = a * FULL as usize + b;
                if flags[cell] & UNCOUNTED != 0 {
                    continue;
                }
                let wall = ExpandedWall::get(walls, size, a, b);
                let segments = u32::from(wall.segments);
                // The style words, then the pattern words, as the
                // service's record lays them out (+8, +0x14).
                let style = |index: usize| match index {
                    0..6 => wall.styles[index].unwrap_or(0),
                    _ => wall.patterns.get(index - 6).copied().unwrap_or(0),
                };
                let mut bit = segments & segments.wrapping_neg();
                while bit != 0 {
                    let index = bit.trailing_zeros() as usize;
                    let kind = style(index);
                    walls_sum =
                        walls_sum.wrapping_add(prices.styles.get(&kind).copied().unwrap_or(0));
                    if PATTERNED.contains(&kind) {
                        let side = index + usize::from(bit == 0x10 || bit == 0x20);
                        let pattern = wall.patterns.get(side).copied().unwrap_or(0);
                        rest = rest.wrapping_add(prices.walls.get(&pattern).copied().unwrap_or(0));
                    }
                    loop {
                        bit <<= 1;
                        if segments & bit == bit || bit >= 0x20 {
                            break;
                        }
                    }
                    if bit >= 0x20 {
                        break;
                    }
                }
                let floor = u16::from_le_bytes([floors[cell * 2], floors[cell * 2 + 1]]);
                if floor == SPLIT_FLOOR {
                    if segments & 0x30 != 0 {
                        for half in [style(0), wall.patterns[0]] {
                            if half != 0 {
                                rest = rest.wrapping_add(floor_price(half) / 2);
                            }
                        }
                    }
                } else if floor != 0 {
                    rest = rest.wrapping_add(floor_price(floor));
                }
                if level == 0
                    && let Some(pool) = array(POOL_ARRAY, 1)
                    && pool[cell] < POOL_TILE
                {
                    rest = rest.wrapping_add(prices.pool);
                }
            }
        }
    }
    (walls_sum / 2).wrapping_add(rest)
}

/// The bytes of a wall cell.
const WALL_CELL: usize = 14;

/// The Furnishings rating (0x416851): the architecture's value over the
/// objects', through the furnishings curve, cut toward zero. The objects'
/// value is rounded to single precision first (fild, fstp); the
/// architecture's is loaded whole (fild) and divided by it, and the
/// quotient rounded to single. With the objects' value 0 after rounding
/// the ratio is 100 (0x66674c).
pub fn furnishings(values: Values, curve: &Curve) -> i32 {
    let objects = Ext::from_i32(values.objects).to_f32();
    let ratio = if objects.ext().partial_cmp(&Ext::ZERO) == Some(Ordering::Equal) {
        F32::from_f32(100.0)
    } else {
        (Ext::from_i32(values.architecture) / objects.ext())
            .single()
            .to_f32()
    };
    curve.weight(ratio).to_i32_truncate()
}

/// The Layout rating (0x4166ee..0x41674f): the five earlier days' and
/// today's samples summed, and those spent following a route; the
/// samples less twice the routing ones, over the samples (at least 1),
/// kept to 0 to 1, times 100, cut toward zero. The sums wrap at 32 bits
/// and the arithmetic is single precision, as the original's FPU is set.
pub fn layout(usage: &HouseUsage) -> i32 {
    let (total, routing) = usage
        .history
        .iter()
        .chain(std::iter::once(&usage.today))
        .fold((0i32, 0i32), |(t, r), day| {
            (t.wrapping_add(day.total), r.wrapping_add(day.routing))
        });
    let total = Ext::from_i32(total.max(1));
    let twice = Ext::from_i32(routing.wrapping_mul(2));
    let ratio = ((total - twice).single() / total).single();
    let ratio = if ratio.partial_cmp(&Ext::ZERO).is_some_and(Ordering::is_ge) {
        ratio
    } else {
        Ext::ZERO
    };
    let one = Ext::from_i32(1);
    let ratio = if ratio.partial_cmp(&one) == Some(Ordering::Greater) {
        one
    } else {
        ratio
    };
    (ratio * Ext::from_i32(100)).single().to_i32_truncate()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::strings::{Format, TextEntry};
    use sims_formats::tuning::Constant;
    use sims_sim::neighbors::Family;
    use sims_sim::rooms::Room;

    fn strings(texts: &[&str]) -> StringTable {
        StringTable {
            format: Format::Localized,
            entries: texts
                .iter()
                .map(|t| TextEntry {
                    language: US_ENGLISH,
                    text: (*t).to_owned(),
                    comment: String::new(),
                    raw_text: t.as_bytes().to_vec(),
                    raw_comment: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn the_tuning_takes_the_curves_by_place_and_the_multiplier_by_name() {
        let tuning = Tuning::default();
        assert!(tuning.size.points.is_empty() && tuning.furnishings.points.is_empty());
        assert_eq!(tuning.yard, F32::from_f32(0.33));
        assert_eq!((tuning.dirty, tuning.broken), (6, 15));
        let constant = |name: &str, value: f32| Constant {
            name: name.into(),
            value,
            comment: String::new(),
        };
        let constants = Constants {
            version: 1,
            entries: vec![
                constant("Yard Score MULTIPLIER", 0.5),
                // Cut toward zero.
                constant("upkeep penalty per dirty object", 3.9),
                constant("upkeep penalty per broken object", -6.5),
            ],
        };
        let tuning = Tuning::from_tables(
            Some(&strings(&["(0;0) (100;50)", "(5;0) (0;100)"])),
            Some(&constants),
        );
        assert_eq!(tuning.yard, F32::from_f32(0.5));
        assert_eq!((tuning.dirty, tuning.broken), (3, -6));
        assert_eq!(tuning.size.points.len(), 2);
        // A zero byte ends a curve's text, before decoding: here a
        // length-prefixed string of a UTF-16 byte order mark and "(0;90)".
        let cut = Tuning::from_tables(Some(&strings(&["(0;0)\0(200;100)"])), None);
        assert_eq!(cut.size.points.len(), 1);
        let mut bytes = vec![0xff, 0xfe];
        for b in b"(0;90)" {
            bytes.extend([*b, 0]);
        }
        let wide = StringTable::parse_chunk(
            &[[1u8, 0].as_slice(), &[bytes.len() as u8], &bytes].concat(),
            0x10,
        )
        .unwrap();
        assert!(
            Tuning::from_tables(Some(&wide), None)
                .size
                .points
                .is_empty()
        );
        // A byte order mark before one-byte text is two bytes that are
        // not a point: the point after it is read.
        let marked = [[0xffu8, 0xfe].as_slice(), b"(0;90)"].concat();
        let marked = StringTable::parse_chunk(
            &[[1u8, 0].as_slice(), &[marked.len() as u8], &marked].concat(),
            0x10,
        )
        .unwrap();
        assert_eq!(
            Tuning::from_tables(Some(&marked), None).size.points.len(),
            1
        );
        // Sorted by x as the original's curves are.
        assert_eq!(
            tuning.furnishings.points[0],
            [F32::from_f32(0.0), F32::from_f32(100.0)]
        );
    }

    fn room(half_tiles: i32, outdoors: bool, dirty: bool, beds: i32, baths: i32) -> Room {
        Room {
            half_tiles,
            outdoors,
            dirty,
            category_4: beds,
            category_5_to_7: baths,
            ..Room::default()
        }
    }

    #[test]
    fn the_facts_count_the_marked_indoor_rooms() {
        let mut world = World::empty();
        world.rooms.rooms.insert(1, room(21, false, true, 1, 0));
        // Halved toward zero: -1 tile.
        world.rooms.rooms.insert(2, room(-3, false, true, 0, 2));
        // Counted by the last update and not marked again: left out.
        world.rooms.rooms.insert(3, room(100, false, false, 1, 1));
        world.rooms.rooms.insert(4, room(100, true, true, 1, 1));
        world.size = 40;
        world.globals[9] = 7;
        world.families.push(Family {
            id: 7,
            house: 1,
            members: vec![1, 2],
            friends: 0,
            stale: false,
        });
        world.rooms.outdoor_sum = F32::from_f32(400.0);
        let tuning = Tuning::from_tables(Some(&strings(&["(0;0) (100;50)"])), None);
        let house = House::read(&world, &tuning, &Kinds::default());
        assert_eq!(
            (
                house.square_feet,
                house.bedrooms,
                house.bathrooms,
                house.lot
            ),
            (81, 1, 1, 1)
        );
        // 81 / 2 = 40.5 square feet a member, 20.25 on the curve; the
        // yard's 132 is kept to 100.
        // Nothing counts for Upkeep: 0.
        // No objects: Furnishings weighs 100 on a missing curve, 0.
        // No usage samples: Layout 100 (`0x4166ee`).
        assert_eq!(
            house.ratings,
            [Some(20), Some(0), Some(100), Some(0), Some(100)]
        );

        // No family: Size 0. A negative or unordered yard score: 0.
        world.globals[9] = 8;
        world.rooms.outdoor_sum = F32::from_f32(-1.0);
        world.size = 39;
        let house = House::read(&world, &tuning, &Kinds::default());
        assert_eq!(
            (house.ratings[SIZE], house.ratings[YARD]),
            (Some(0), Some(0))
        );
        assert_eq!(house.lot, 0);
        world.rooms.outdoor_sum = F32::from_f32(f32::NAN);
        world.size = 50;
        let house = House::read(&world, &tuning, &Kinds::default());
        assert_eq!((house.ratings[YARD], house.lot), (Some(0), 2));
    }

    fn object(id: u16, guid: u32, broken: i16, dirty: i16) -> sims_sim::Object {
        let mut data = vec![0; 128];
        data[BROKEN] = broken;
        data[DIRTY] = dirty;
        sims_sim::Object {
            id,
            guid,
            footprint: [0; 4],
            position: [0; 3],
            unknown_short: 0,
            attributes: Vec::new(),
            temps: Vec::new(),
            data,
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: Vec::new(),
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: None,
            registry_word: None,
            last_result: false,
            removal_skips_resets: false,
            selected: false,
        }
    }

    #[test]
    fn upkeep_takes_the_penalties_off_the_counted_objects() {
        let tuning = Tuning {
            dirty: 3,
            broken: 6,
            ..Tuning::default()
        };
        // GUID 1 counts and 2 does not. A broken object takes its penalty
        // whether or not it is dirty, and an object that does not count
        // still takes its penalty.
        let kinds = Kinds {
            counted: BTreeMap::from([(1, true), (2, false)]),
            ..Kinds::default()
        };
        let mut world = World::empty();
        let upkeep = |world: &World| House::read(world, &tuning, &kinds).ratings[UPKEEP];
        for id in 1..=20 {
            world.objects.insert(id, object(id, 1, 0, 0));
        }
        world.objects.insert(21, object(21, 2, 0, 5));
        world.objects.insert(22, object(22, 1, 1, 9));
        world.objects.insert(23, object(23, 9, 0, -1));
        // 21 counted (GUID 9 was not read: it counts for nothing), 9
        // taken off: 12 / 21 is 57.14 percent.
        assert_eq!(upkeep(&world), Some(57));
        // More taken off than counted: 0; a negative penalty can take it
        // over 1, which is kept to 100.
        world.objects.insert(24, object(24, 2, 2, 0));
        world.objects.insert(25, object(25, 2, 2, 0));
        world.objects.insert(26, object(26, 2, 2, 0));
        world.objects.insert(27, object(27, 2, 2, 0));
        assert_eq!(upkeep(&world), Some(0));
        let generous = Tuning {
            dirty: -3,
            broken: -6,
            ..Tuning::default()
        };
        assert_eq!(
            House::read(&world, &generous, &kinds).ratings[UPKEEP],
            Some(100)
        );
        // Nothing counted: 0, penalties or not.
        assert_eq!(
            House::read(&world, &tuning, &Kinds::default()).ratings[UPKEEP],
            Some(0)
        );
    }

    #[test]
    fn a_type_counts_by_its_room_and_function_flags_or_build_mode_type() {
        let mut definition = vec![0u8; 216];
        assert!(!Kinds::counts(&definition));
        for at in word::COUNTED.map(|w| w * 2) {
            definition.fill(0);
            definition[at + 1] = 0x80;
            assert!(Kinds::counts(&definition), "{at:#x}");
        }
        // The word after the room flags is the function flags; one past
        // the build mode type does not count.
        definition.fill(0);
        definition[0x8c] = 1;
        assert!(!Kinds::counts(&definition));
        // A short definition reads the words it lacks as 0, and half a
        // word as its low byte (Sol's review: a version 138 definition
        // ending at 0x8b).
        definition.fill(0);
        definition[..4].copy_from_slice(&138u32.to_le_bytes());
        definition[0x8a] = 1;
        assert!(Kinds::counts(&definition[..0x8b]));
        assert!(!Kinds::counts(&definition[..0x8a]));
        definition[0x8a] = 0;
        definition[0x4e] = 1;
        assert!(Kinds::counts(&definition[..0x4f]));
        // A version 136 or 137 definition of odd length loses its last
        // byte; an even one keeps it.
        for version in [136u32, 137] {
            definition[..4].copy_from_slice(&version.to_le_bytes());
            assert!(!Kinds::counts(&definition[..0x4f]), "{version}");
            assert!(Kinds::counts(&definition[..0x50]), "{version}");
        }
    }

    #[test]
    fn the_prices_come_from_the_build_tables_and_the_pattern_files() {
        // Three strings an entry, the price first.
        let entries = |prices: &[&str]| {
            let texts: Vec<&str> = prices.iter().flat_map(|p| [*p, "name", "about"]).collect();
            strings(&texts)
        };
        let tables = BTreeMap::from([
            (STYLE_TABLE, entries(&["70", "10", "45", "35", "40"])),
            (FLOOR_TABLE, entries(&["5", " 15 dollars", "x"])),
            (WALL_TABLE, entries(&["6", "-4"])),
            (POOL_TABLE, entries(&["75"])),
            (FLOOR_DEFAULT_TABLE, entries(&["20"])),
        ]);
        let file = |keys: &[(u16, u16)], price| sims_content::SurfaceFile {
            name: String::new(),
            location: String::new(),
            keys: keys.iter().copied().collect(),
            price,
        };
        let surfaces = Surfaces {
            floors: sims_content::SurfaceCatalog {
                files: vec![
                    file(&[], None),
                    file(&[(1, 300), (2, 301)], Some(12)),
                    file(&[(1, 302)], None),
                ],
            },
            walls: sims_content::SurfaceCatalog {
                files: vec![file(&[], None), file(&[(1, 400)], None)],
            },
        };
        let prices = Prices::from_tables(&tables, &surfaces);
        assert_eq!(
            prices.styles,
            BTreeMap::from([(1, 70), (2, 10), (13, 45), (12, 35), (14, 40), (255, 70)])
        );
        assert_eq!(prices.pool, 75);
        // A built-in floor beyond the table, and one whose price is not a
        // number, cost 0; a plug-in without a price costs the default.
        assert_eq!(
            [1, 2, 3, 4, 29, 30, 300, 301, 302].map(|k| prices.floors.get(&k).copied()),
            [
                Some(5),
                Some(15),
                Some(0),
                Some(0),
                Some(0),
                None,
                Some(12),
                Some(12),
                Some(20)
            ]
        );
        // Wall pattern 0 has no entry; the plug-ins' default table is
        // missing here, so it is 0.
        assert_eq!(
            [0, 1, 2, 30, 31, 248, 400].map(|k| prices.walls.get(&k).copied()),
            [Some(0), Some(6), Some(-4), Some(0), None, Some(0), Some(0)]
        );
        // The price is read from the string's bytes: a byte order mark
        // before the digits reads 0.
        let tables = BTreeMap::from([(POOL_TABLE, entries(&["\u{feff}75"]))]);
        assert_eq!(Prices::from_tables(&tables, &surfaces).pool, 0);
    }

    /// A lot array of the arrays' full size, every byte `fill`.
    fn array(stride: usize, fill: u8) -> sims_formats::lot::Array {
        sims_formats::lot::Array {
            width: FULL as usize,
            height: FULL as usize,
            stride,
            encoding: 0,
            data: vec![fill; (FULL * FULL) as usize * stride],
        }
    }

    #[test]
    fn the_architecture_counts_walls_floors_and_the_pool() {
        let mut world = World::empty();
        world.size = 4;
        for (id, stride, fill) in [
            (1, 2, 0),
            (101, 2, 0),
            (2, WALL_CELL, 0),
            (102, WALL_CELL, 0),
            (8, 1, 0),
            (108, 1, 0),
            (9, 1, 0xff),
        ] {
            world.arrays.insert(id, array(stride, fill));
        }
        let cell = |a: usize, b: usize| a * FULL as usize + b;
        let set_floor = |world: &mut World, a, b, key: u16| {
            let at = cell(a, b) * 2;
            world.arrays.get_mut(&1).unwrap().data[at..at + 2].copy_from_slice(&key.to_le_bytes());
        };
        // A wall cell: its segments, then its six words (styles west and
        // north, patterns west, north, south and east).
        let set_wall = |world: &mut World, a, b, segments: u8, words: [u16; 6]| {
            let at = cell(a, b) * WALL_CELL;
            let data = &mut world.arrays.get_mut(&2).unwrap().data[at..at + WALL_CELL];
            data[0] = segments;
            for (n, word) in words.iter().enumerate() {
                data[2 + n * 2..4 + n * 2].copy_from_slice(&word.to_le_bytes());
            }
        };
        // (1, 1): a wall of style 1 to the west with pattern 3, a fence
        // (style 2, no pattern counted) to the north, floor 5.
        set_wall(&mut world, 1, 1, 0x03, [1, 2, 3, 1, 0, 0]);
        set_floor(&mut world, 1, 1, 5);
        // (2, 2): a descending diagonal of style 1 (the north style word)
        // splitting the floor; its second pattern (the east word) counts,
        // and the halves' floors are the west style and pattern words.
        set_wall(&mut world, 2, 2, 0x10, [1, 1, 4, 0, 3, 2]);
        set_floor(&mut world, 2, 2, 0xff);
        // (1, 2): the ascending diagonal is not the lowest segment, so only
        // the west wall counts.
        set_wall(&mut world, 1, 2, 0x21, [1, 2, 1, 0, 0, 0]);
        // (3, 3): kept out by tile flag 0x20.
        set_wall(&mut world, 3, 3, 0x01, [1, 0, 0, 0, 0, 0]);
        set_floor(&mut world, 3, 3, 5);
        world.arrays.get_mut(&8).unwrap().data[cell(3, 3)] = UNCOUNTED;
        // A pool tile at (0, 0), and one below 0x10 outside the lot's size.
        world.arrays.get_mut(&9).unwrap().data[cell(0, 0)] = 0x0f;
        world.arrays.get_mut(&9).unwrap().data[cell(5, 0)] = 0;
        let prices = Prices {
            styles: BTreeMap::from([(1, 70), (2, 15)]),
            pool: 75,
            floors: BTreeMap::from([(1, 5), (4, 15), (5, 20)]),
            walls: BTreeMap::from([(1, 6), (2, 4), (3, 4)]),
        };
        // Styles: 70 + 15, 70 and 70, halved: 112. Patterns 4, 4 and 6;
        // floors 20, 5 / 2 and 15 / 2; the pool 75.
        assert_eq!(architecture(&world, &prices), 112 + 14 + 20 + 2 + 7 + 75);
        // Without its arrays a level counts nothing.
        assert_eq!(architecture(&World::empty(), &prices), 0);
    }

    /// The arrays' index of tile (a, b).
    fn cell(a: usize, b: usize) -> usize {
        a * FULL as usize + b
    }

    /// Sets a wall cell in array `id`: its segments, then its six words
    /// (styles west and north, patterns west, north, south and east).
    fn put_wall(world: &mut World, id: u16, (a, b): (usize, usize), segments: u8, words: [u16; 6]) {
        let at = cell(a, b) * WALL_CELL;
        let data = &mut world.arrays.get_mut(&id).unwrap().data[at..at + WALL_CELL];
        data[0] = segments;
        for (n, word) in words.iter().enumerate() {
            data[2 + n * 2..4 + n * 2].copy_from_slice(&word.to_le_bytes());
        }
    }

    /// A 4-tile lot with the given arrays, all 0.
    fn lot(arrays: &[(u16, usize)]) -> World {
        let mut world = World::empty();
        world.size = 4;
        for &(id, stride) in arrays {
            world.arrays.insert(id, array(stride, 0));
        }
        world
    }

    #[test]
    fn the_ascending_diagonal_alone_counts_its_first_pattern() {
        // Level 1 only, and no pool array: the pool alone is left out.
        let mut world = lot(&[(1, 2), (2, WALL_CELL), (8, 1)]);
        // An ascending diagonal of style 7 (the north style word), its
        // first pattern the south word, splitting the floor into floors 1
        // (the west style word) and 4 (the west pattern word).
        put_wall(&mut world, 2, (1, 1), 0x20, [1, 7, 4, 0, 3, 9]);
        let at = cell(1, 1) * 2;
        world.arrays.get_mut(&1).unwrap().data[at..at + 2].copy_from_slice(&0xffu16.to_le_bytes());
        let prices = Prices {
            styles: BTreeMap::from([(7, 10)]),
            pool: 75,
            floors: BTreeMap::from([(1, 5), (4, 15)]),
            walls: BTreeMap::from([(3, 4), (9, 1000)]),
        };
        assert_eq!(architecture(&world, &prices), 10 / 2 + 4 + 5 / 2 + 15 / 2);
    }

    #[test]
    fn the_walls_sum_wraps_before_it_is_halved() {
        let mut world = lot(&[(1, 2), (2, WALL_CELL), (8, 1)]);
        put_wall(&mut world, 2, (1, 1), 0x03, [2, 2, 0, 0, 0, 0]);
        let prices = Prices {
            styles: BTreeMap::from([(2, i32::MAX)]),
            ..Prices::default()
        };
        assert_eq!(architecture(&world, &prices), i32::MAX.wrapping_mul(2) / 2);
        assert_eq!(architecture(&world, &prices), -1);
    }

    #[test]
    fn a_level_missing_an_array_counts_nothing_but_the_other_still_counts() {
        // Level 1 has no floors, so neither its wall nor the pool counts;
        // level 2 is whole.
        let mut world = lot(&[
            (2, WALL_CELL),
            (8, 1),
            (9, 1),
            (101, 2),
            (102, WALL_CELL),
            (108, 1),
        ]);
        put_wall(&mut world, 2, (1, 1), 0x01, [2, 0, 0, 0, 0, 0]);
        put_wall(&mut world, 102, (2, 2), 0x01, [2, 0, 0, 0, 0, 0]);
        let prices = Prices {
            styles: BTreeMap::from([(2, 30)]),
            pool: 75,
            ..Prices::default()
        };
        assert_eq!(architecture(&world, &prices), 15);
    }

    #[test]
    fn the_scan_values_the_objects_and_gives_the_rest_to_the_architecture() {
        let mut world = World::empty();
        world.size = 10;
        let ty = |kind, group, build| Type { kind, group, build };
        let kinds = Kinds {
            types: BTreeMap::from([
                (10, ty(0, 0, Some(0))),
                (11, ty(0, 0, Some(5))),
                (12, ty(0, 0, Some(4))),
                (13, ty(PERSON, 0, Some(5))),
                (14, ty(SKIPPED, 0, Some(5))),
                (15, ty(0, 3, Some(0))),
                (16, ty(PERSON, 0, None)),
                (17, ty(0, 0, None)),
                (18, ty(0, 0, Some(5))),
            ]),
            ..Kinds::default()
        };
        let on_lot = [5 * 16, 5 * 16, 1];
        let mut add = |id: u16, guid: u32, position: [i32; 3], value: i16| {
            let mut object = object(id, guid, 0, 0);
            object.position = position;
            object.data[VALUE] = value;
            world.objects.insert(id, object);
            world.order.push(id);
        };
        add(1, 10, on_lot, 100);
        // Broken: halved toward zero.
        add(2, 10, on_lot, -51);
        add(3, 10, on_lot, 7);
        add(4, 11, on_lot, 30);
        add(5, 12, on_lot, 40);
        add(6, 13, on_lot, 9);
        // Off the lot (tile 0): to the architecture, by its build type.
        add(7, 13, [0, 0, 1], 3);
        add(8, 14, on_lot, 1000);
        // A group: the lead's value, halved for its broken tile, once.
        add(9, 15, on_lot, 80);
        add(10, 15, on_lot, 999);
        // A type not read.
        add(11, 99, on_lot, 500);
        // Without a master, a person is still valued; any other type is
        // neither valued nor architecture.
        add(12, 16, on_lot, 11);
        add(13, 17, on_lot, 600);
        // The lot is 10 tiles: tile 8 (its size less 2) is on it, tile 9
        // (its size less 1) is not, whatever the arrays' size.
        add(14, 10, [8 * 16, 5 * 16, 1], 1);
        add(15, 10, [5 * 16, 9 * 16, 1], 700);
        add(16, 18, [9 * 16, 8 * 16, 1], 2);
        world.objects.get_mut(&2).unwrap().data[BROKEN] = 1;
        world.objects.get_mut(&3).unwrap().data[FLAGS] = NOT_VALUED;
        world.objects.get_mut(&10).unwrap().data[BROKEN] = 1;
        let part = |parent| sims_formats::saved_objects::Multipart {
            offset: [0; 3],
            group: [0; 3],
            parent,
        };
        world.objects.get_mut(&9).unwrap().multipart = Some(part(0));
        world.objects.get_mut(&10).unwrap().multipart = Some(part(9));
        world.next_parts.insert(9, 10);
        let values = Values::read(&world, &kinds, &Prices::default());
        assert_eq!(
            values,
            Values {
                objects: 100 - 25 + 9 + 40 + 11 + 1,
                architecture: 30 + 3 + 2,
            }
        );
    }

    #[test]
    fn a_groups_master_is_read_from_each_ids_definition_of_high_byte_0() {
        // A definition of group 3: its word 11 and build mode type.
        let definition = |index: i16, build: i16, unregistered: i16| {
            let mut words = [0i16; DEFINITION_BYTES / 2];
            words[word::GROUP] = 3;
            words[word::GROUP_INDEX] = index;
            words[word::BUILD] = build;
            words[word::UNREGISTERED] = unregistered;
            words
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect::<Vec<u8>>()
        };
        let mut bytes = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOLINE 1998".to_vec();
        bytes.resize(64, 0);
        // ID 5 with flags' high byte 1 looks like a master and comes first,
        // but the original registers ID 5 by its high byte 0 definition,
        // which is not one. ID 4 is one with word 16 set, which the
        // original does not register; ID 6 is the master.
        for (id, flags, data) in [
            (5u16, 0x0110u16, definition(-1, 9, 0)),
            (5, 0x0010, definition(0, 1, 0)),
            (4, 0x0010, definition(-1, 7, 1)),
            (6, 0x0010, definition(-1, 2, 0)),
        ] {
            bytes.extend(b"OBJD");
            bytes.extend((76 + data.len() as u32).to_be_bytes());
            bytes.extend(id.to_be_bytes());
            bytes.extend(flags.to_be_bytes());
            bytes.extend([0u8; 64]);
            bytes.extend(data);
        }
        let iff = Iff::parse(bytes).unwrap();
        let master = Kinds::master(&iff, 3).unwrap().unwrap();
        assert_eq!(master[word::BUILD], 2);
        assert_eq!(Kinds::master(&iff, 4).unwrap(), None);
    }

    #[test]
    fn the_panel_weighs_the_values_the_simulation_keeps() {
        let mut world = World::empty();
        world.budget.objects_value = 2;
        world.budget.architecture_value = 300;
        let tuning = Tuning::from_tables(Some(&strings(&["", "(0;0) (200;100)"])), None);
        let house = House::read(&world, &tuning, &Kinds::default());
        assert_eq!(
            house.values,
            Values {
                objects: 2,
                architecture: 300,
            }
        );
        assert_eq!(house.ratings[FURNISHINGS], Some(75));
    }

    #[test]
    fn layout_weighs_the_samples_spent_routing() {
        use sims_sim::Usage;
        let usage = |days: [(i32, i32); 6]| {
            let day = |(total, routing)| Usage { total, routing };
            HouseUsage {
                history: [days[0], days[1], days[2], days[3], days[4]].map(day),
                today: day(days[5]),
            }
        };
        // Six days summed: 40 samples, 7 routing: (40 - 14) / 40.
        let week = usage([(10, 2), (5, 1), (5, 0), (5, 1), (5, 1), (10, 2)]);
        assert_eq!(layout(&week), 65);
        // No samples: 0 over at least 1, so full.
        assert_eq!(layout(&HouseUsage::default()), 100);
        // More than half routing: kept to 0.
        assert_eq!(
            layout(&usage([(0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (4, 3)])),
            0
        );
        // A third routing: 1/3 in single precision, times 100, cut.
        assert_eq!(
            layout(&usage([(0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (3, 1)])),
            33
        );
    }

    #[test]
    fn furnishings_weighs_the_architecture_over_the_objects() {
        let curve = Curve::parse(0, "(0;0) (200;100)", CURVE_POINTS);
        let rating = |objects, architecture| {
            furnishings(
                Values {
                    objects,
                    architecture,
                },
                &curve,
            )
        };
        assert_eq!(rating(2, 300), 75);
        // Nothing valued: the ratio is 100.
        assert_eq!(rating(0, 7), 50);
        // Cut toward zero.
        assert_eq!(rating(3, 2), 0);
        assert_eq!(rating(-1, 1), 0);
    }
}
