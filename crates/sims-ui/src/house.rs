//! What the house panel shows: the lot's five ratings and four facts,
//! worked out from the simulation as the original's 0x416622 works them
//! out each time the panel draws (0x52e49e).

use crate::{Error, Result};
use sims_content::ui_graphics::US_ENGLISH;
use sims_content::{Catalog, Index};
use sims_core::{Ext, F32};
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_formats::tuning::Constants;
use sims_sim::World;
use sims_sim::motives::Curve;
use std::cmp::Ordering;
use std::collections::BTreeMap;

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
    /// The Furnishings rating: string 1. Read, but the rating is not
    /// ported.
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
        let Some(global) = index.resources.get(catalog.global) else {
            return Ok(Self::default());
        };
        let iff = Iff::parse(global.read().map_err(Error::Content)?).map_err(Error::Format)?;
        let curves = iff
            .get(b"STR#", CURVES)
            .map(|chunk| StringTable::parse(iff.data(chunk)))
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
    /// A missing curve has no points and weighs everything 0, as the
    /// original's empty curve does (0x4167de). The original looks names
    /// up ignoring the case of A to Z (0x4131f4) and takes the default
    /// for a missing one (0x413271).
    pub fn from_tables(curves: Option<&StringTable>, constants: Option<&Constants>) -> Self {
        let curve = |n: usize| {
            let text = curves.and_then(|t| t.get(n, US_ENGLISH)).unwrap_or("");
            Curve::parse(0, text, CURVE_POINTS)
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

/// Which object types the Upkeep rating counts, by GUID (0x4452eb): those
/// whose definition has room flags, function flags or a build mode type
/// (definition +0x4e, +0x50 and +0x8a: OBJD words 37, 38 and 67 after
/// its version) that are not 0. Read from the install's definitions as
/// the lot's objects show up ([`Kinds::learn`]); a type not read counts
/// for nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kinds {
    pub counted: BTreeMap<u32, bool>,
}

/// The definition's bytes the Upkeep count tests, each a 16-bit word.
const COUNTED_WORDS: [usize; 3] = [0x4e, 0x50, 0x8a];

impl Kinds {
    /// Reads the definition of every object type in `world` not read
    /// yet. A type the catalog lacks, or whose file or definition cannot
    /// be read, counts for nothing. A definition shorter than a word
    /// reads that word as 0, as the original's record is filled out with
    /// zeros (0x431759).
    pub fn learn(&mut self, world: &World, index: &Index, catalog: &Catalog) {
        let mut files: BTreeMap<usize, Option<Iff>> = BTreeMap::new();
        for object in world.objects.values() {
            if self.counted.contains_key(&object.guid) {
                continue;
            }
            let counted = catalog.object(object.guid).is_some_and(|definition| {
                let iff = files.entry(definition.resource).or_insert_with(|| {
                    let bytes = index.resources.get(definition.resource)?.read().ok()?;
                    Iff::parse(bytes).ok()
                });
                iff.as_ref()
                    .and_then(|iff| {
                        let chunk = iff.get(b"OBJD", definition.chunk)?;
                        Some(Self::counts(iff.data(chunk)))
                    })
                    .unwrap_or(false)
            });
            self.counted.insert(object.guid, counted);
        }
    }

    /// Whether a definition's bytes make its objects count.
    pub fn counts(definition: &[u8]) -> bool {
        COUNTED_WORDS.iter().any(|&at| {
            let word = definition.get(at..at + 2).map_or([0, 0], |b| [b[0], b[1]]);
            i16::from_le_bytes(word) != 0
        })
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
    /// Not ported: Furnishings (the lot's object values, 0x444cb9) and
    /// Layout (its routing history, lot +0x3c to +0x64).
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
        assert_eq!(house.ratings, [Some(20), None, Some(100), Some(0), None]);

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
        for at in COUNTED_WORDS {
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
        // word as none.
        definition[0x4e] = 1;
        assert!(!Kinds::counts(&definition[..0x4f]));
        assert!(Kinds::counts(&definition[..0x50]));
    }
}
