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
        let yard = constants
            .and_then(|table| {
                table
                    .entries
                    .iter()
                    .find(|entry| entry.name.eq_ignore_ascii_case(YARD_NAME))
            })
            .map_or(YARD_DEFAULT, |entry| entry.value);
        Self {
            size: curve(0),
            furnishings: curve(1),
            yard: F32::from_f32(yard),
        }
    }
}

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
    /// - Each rating is cut to a whole number toward zero (`__ftol`), and
    ///   the arithmetic is single precision, as the original's FPU is
    ///   set.
    ///
    /// Not ported: Furnishings (the lot's object values, 0x444cb9),
    /// Upkeep (its dirty and broken objects, 0x4452eb) and Layout (its
    /// routing history, lot +0x3c to +0x64).
    pub fn read(world: &World, tuning: &Tuning) -> Self {
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
        house
    }
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
        let constants = Constants {
            version: 1,
            entries: vec![Constant {
                name: "Yard Score MULTIPLIER".into(),
                value: 0.5,
                comment: String::new(),
            }],
        };
        let tuning = Tuning::from_tables(
            Some(&strings(&["(0;0) (100;50)", "(5;0) (0;100)"])),
            Some(&constants),
        );
        assert_eq!(tuning.yard, F32::from_f32(0.5));
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
        let house = House::read(&world, &tuning);
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
        assert_eq!(house.ratings, [Some(20), None, Some(100), None, None]);

        // No family: Size 0. A negative or unordered yard score: 0.
        world.globals[9] = 8;
        world.rooms.outdoor_sum = F32::from_f32(-1.0);
        world.size = 39;
        let house = House::read(&world, &tuning);
        assert_eq!(
            (house.ratings[SIZE], house.ratings[YARD]),
            (Some(0), Some(0))
        );
        assert_eq!(house.lot, 0);
        world.rooms.outdoor_sum = F32::from_f32(f32::NAN);
        world.size = 50;
        let house = House::read(&world, &tuning);
        assert_eq!((house.ratings[YARD], house.lot), (Some(0), 2));
    }
}
