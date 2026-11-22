//! Neighborhood REDB lot lifecycle: entering a lot from a [`NeighborhoodStore`],
//! saving & leaving back to the neighborhood with faithful `Sims.exe` 1.91
//! writeback, and Move-In / Evict / Bulldoze backend operations.
//!
//! All state transitions follow `Sims.exe` 1.91 disassembly:
//! - Save & Leave (`0x428687` → `0x41615d` → `0x43fb1b` → `0x4252cc`):
//!   - Flushes every live person into its neighborhood record via
//!     [`crate::people::save_to_record`] (`0x425a69`, called from
//!     `cXPerson::PreSave` `0x459d8b`).
//!   - Refreshes lot statistics (`0x416085` → `0x444cb9` + `0x47b006`):
//!     `world.budget.objects_value`, `world.budget.architecture_value`,
//!     globals 22/23/37, and family flags bits 0 & 1.
//!   - Writes back `FAMI`, `NBRS`, `NGBH`, `TATT`, runtime-made characters, and
//!     the lot's [`Saved`] state atomically into REDB.
//! - Re-entry reconciliation (`0x424806`, `0x428041`):
//!   - Restores lot-local state from REDB's saved lot, reloads the latest
//!     neighborhood state (`FAMI`/`NBRS`/`NGBH`/`TATT`/`made_characters`),
//!     re-attaches every live person's persistent fields (`0x4258a3`), and runs
//!     household selection + move-in reset if [`FAMILY_FLAG_NEW_MOVE_IN`] is set.
//! - Move-In (`0x42731e`, `0x427f78`, `0x4282bc..0x4284f9`):
//!   - Validates Family Bin household (`house == 0`, non-empty), residential
//!     unoccupied lot (`zoning == 1`), and funds (`budget >= lot_price`).
//!   - Deducts `lot_price` into `family.architecture_value`, sets
//!     `family.house`, clears flag bit 3, and sets [`FAMILY_FLAG_NEW_MOVE_IN`].
//!   - On first entry, clears [`FAMILY_FLAG_NEW_MOVE_IN`], sets clock to
//!     `"new family start hour"` (default 7:00), resets budget & usage history,
//!     unpauses, broadcasts `"reset lot"` (`0x444741`), and refreshes visitors.
//! - Evict (`0x42737e(house, false)`) & Bulldoze (`0x42737e(house, true)`):
//!   - Flushes persons to `NBRS`, broadcasts `"evict lot"` (`0x444741`),
//!     removes removable objects (`0x4354d5`; and on Bulldoze also non-locked
//!     build objects, non-`0x20` floors & walls via `ExpandedWall::put`, pool
//!     array `Arry #9` via `0x461a34`, and house-bound tombstone neighbor
//!     records via `0x427697..0x4276ce`).
//!   - Refunds exact depreciated `family.architecture_value` into
//!     `family.budget`, zeroes `family.architecture_value` & `family.house`,
//!     resets lot funds/budget/usage, recomputes lot values (`0x416085`), and
//!     commits both lot and neighborhood atomically to REDB.

use crate::boot::save::Saved;
use crate::boot::{Finished, Frames, LoadReport, Lot, NeighborhoodOrigin, opening_rotation};
use crate::daily::value as object_value;
use crate::factory::word;
use crate::fault::Fault;
use crate::interpreter::{Interpreter, Stream};
use crate::people;
use crate::play::Play;
use crate::program::{Callback, DEFINITION_WORDS};
use crate::selection::selected;
use crate::{Answers, BuyCursor, Program};
use sims_content::{Animations, Catalog, House, Index, Install, People, SurfaceCatalog, Surfaces};
use sims_core::CrtRng;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;
use sims_sim::load::{Context, ExpandedWall, FLOORS, WALLS, visitor_again};
use sims_sim::neighborhood_store::{
    CheckedCommitError, DEFAULT_ARCH_VALUE_MULTIPLIER, DEFAULT_LAYOUT_HISTORY_FILL,
    DEFAULT_NEW_FAMILY_START_HOUR, FAMILY_FLAG_BIT_3, FAMILY_FLAG_NEW_MOVE_IN, NeighborhoodStore,
    StoredNeighborhood, lot_total_value,
};
use sims_sim::neighbors::registry;
use sims_sim::{FULL_SIZE, HouseUsage, Usage, World};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

/// `Global.far` `FCNS` chunk IDs read by neighborhood tuning (`0x424216`) and
/// autonomy (`0x6c4398`).
const NEIGHBORHOOD_TUNING: u16 = 4;
const AUTONOMY_TUNING: u16 = 2;

/// Default friendship threshold when `FCNS #2` lacks `"friendship threshold"`.
const DEFAULT_FRIENDSHIP_THRESHOLD: i32 = 50;

/// Global indices manipulated during Move-In / Evict / Bulldoze / Statistics.
const HOURS: usize = 0;
const TIME_OF_DAY: usize = 4;
const MINUTES: usize = 5;
const PAUSED: usize = 17;
const BUILT: usize = 22;
const GLOBAL_26: usize = 26;
const DAYS_PLAYED: usize = 28;
const BUDGET_MODE: usize = 32;

/// MagiCoin inventory token (`TokensMagic.iff` `"Magic Token - Currency"`,
/// `Sims.exe` `0x410282` / `0x556467`).
pub const MAGIC_COIN_TOKEN_KIND: i32 = 7;
pub const MAGIC_COIN_TOKEN_GUID: u32 = 0x99e8_1bec;
/// Magic Town Creepy Hollow residential lots (`0x5500ee..0x5500f6`).
pub const MAGIC_TOWN_RESIDENTIAL_LOTS: std::ops::RangeInclusive<u32> = 90..=92;
const UITEXT_HOUSE_INFO_TABLE: u16 = 134;
const DEFAULT_MAGIC_LOT_91_COST: i32 = 1500;
const DEFAULT_MAGIC_LOT_OTHER_COST: i32 = 1000;

/// Object data words.
const DATA_FLAGS: usize = 43;
const DATA_CATEGORY: usize = 59;
const FLAG_NOT_VALUED: i16 = 0x10;

/// OBJD words.
const DEF_TYPE_SKIPPED: i16 = 7;
const DEF_GLOBAL_CONTROLLER: usize = 42;
const DEF_BUILT: [usize; 3] = [39, 40, 69];
const DEF_BUILD_KIND: usize = 69;

/// Person data words.
const PERSON_VISITOR: usize = 32;
const PERSON_FAMILY: usize = 61;
const PERSON_HOME: usize = 67;

/// Lot array IDs used for architecture valuation and bulldozing.
const TILE_FLAGS: [u16; 2] = [8, 108];
const POOL_ARRAY: u16 = 9;
const TILE_LOCKED: u8 = 0x20;
const SPLIT_FLOOR: u16 = 0xff;
const POOL_TILE_LIMIT: u8 = 0x10;
const PATTERNED_WALL_STYLES: [u16; 4] = [1, 7, 8, 9];
const WALL_CELL: usize = 14;

/// `Build.iff` string tables used to price walls, floors, and pools (`0x47b006`).
const STYLE_TABLE: u16 = 129;
const FLOOR_TABLE: u16 = 130;
const WALL_TABLE: u16 = 131;
const POOL_TABLE: u16 = 133;
const FLOOR_DEFAULT_TABLE: u16 = 134;
const WALL_DEFAULT_TABLE: u16 = 135;
const PRICED_STYLES: [u16; 5] = [1, 2, 13, 12, 14];
const SECOND_STYLE: u16 = 255;
const US_ENGLISH: u8 = 1;

/// Errors returned by neighborhood lifecycle operations.
#[derive(Debug)]
pub enum LifecycleError {
    Store(sims_sim::Error),
    NeighborhoodNotFound(String),
    HouseNotFound {
        neighborhood: String,
        house: u32,
    },
    FamilyNotFound {
        neighborhood: String,
        family: u16,
    },
    FamilyNotInBin {
        family: u16,
        house: i32,
    },
    FamilyEmpty {
        family: u16,
    },
    LotNotResidential {
        house: u32,
        zoning: Option<i32>,
    },
    LotOccupied {
        house: u32,
        family: u16,
    },
    LotUnoccupied {
        house: u32,
    },
    InsufficientFunds {
        family: u16,
        house: u32,
        available: i32,
        required: i32,
    },
    InsufficientMagicCoins {
        family: u16,
        house: u32,
        available: i32,
        required: i32,
    },
    NeighborhoodMismatch {
        expected: String,
        actual: String,
    },
    StaleNeighborhood {
        neighborhood: String,
        expected_revision: u64,
        current_revision: u64,
    },
    Lot(String),
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::NeighborhoodNotFound(id) => write!(f, "neighborhood {id:?} not found in store"),
            Self::HouseNotFound {
                neighborhood,
                house,
            } => write!(
                f,
                "house {house} not found in neighborhood {neighborhood:?}"
            ),
            Self::FamilyNotFound {
                neighborhood,
                family,
            } => write!(
                f,
                "family {family} not found in neighborhood {neighborhood:?}"
            ),
            Self::FamilyNotInBin { family, house } => write!(
                f,
                "family {family} is already living in house {house}, not in the Family Bin"
            ),
            Self::FamilyEmpty { family } => {
                write!(f, "family {family} has no members and cannot move in")
            }
            Self::LotNotResidential { house, zoning } => write!(
                f,
                "house {house} is not zoned residential (zoning: {zoning:?})"
            ),
            Self::LotOccupied { house, family } => {
                write!(f, "house {house} is already occupied by family {family}")
            }
            Self::LotUnoccupied { house } => write!(f, "house {house} has no occupying family"),
            Self::InsufficientFunds {
                family,
                house,
                available,
                required,
            } => write!(
                f,
                "family {family} has ${available} but house {house} costs ${required}"
            ),
            Self::InsufficientMagicCoins {
                family,
                house,
                available,
                required,
            } => write!(
                f,
                "family {family} has {available} MagiCoins but house {house} requires {required} MagiCoins"
            ),
            Self::NeighborhoodMismatch { expected, actual } => write!(
                f,
                "lot was opened from neighborhood {expected:?}, cannot save into {actual:?}"
            ),
            Self::StaleNeighborhood {
                neighborhood,
                expected_revision,
                current_revision,
            } => write!(
                f,
                "neighborhood {neighborhood:?} changed since lot was opened (expected revision \
                 {expected_revision}, store is at revision {current_revision})"
            ),
            Self::Lot(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for LifecycleError {}

impl From<sims_sim::Error> for LifecycleError {
    fn from(error: sims_sim::Error) -> Self {
        Self::Store(error)
    }
}

/// Summary of [`move_in`] / [`move_in_with_multiplier`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveInReport {
    pub family: u16,
    pub house: u32,
    pub lot_price: i32,
    pub budget_before: i32,
    pub budget_after: i32,
    pub architecture_value: i32,
    pub committed_instance_id: u128,
    pub committed_revision: u64,
}

/// Summary of [`evict`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictReport {
    pub house: u32,
    pub family: u16,
    pub refund: i32,
    pub budget_after: i32,
    pub removed_objects: usize,
    pub remaining_objects_value: i32,
    pub remaining_architecture_value: i32,
}

/// Summary of [`bulldoze`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulldozeReport {
    pub house: u32,
    pub evicted_family: Option<u16>,
    pub refund: i32,
    pub removed_objects: usize,
    pub cleared_tiles: usize,
    pub deleted_tombstones: usize,
    pub remaining_objects_value: i32,
    pub remaining_architecture_value: i32,
}

/// Summary of [`save_and_leave_lot`] / [`save_and_leave_lot_without_buy_cursor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReport {
    pub neighborhood: String,
    pub house: u32,
    pub family: u16,
    pub saved_people: usize,
    pub budget: i32,
    pub lot_total_value: i32,
    pub objects_value: i32,
    pub architecture_value: i32,
    pub revision: u64,
}

/// Neighborhood tuning constants read from `Global.far` `FCNS #4` (`0x424216`)
/// and `FCNS #2`.
#[derive(Debug, Clone, Copy)]
struct NeighborhoodTuning {
    arch_value_multiplier: f32,
    new_family_start_hour: i16,
    layout_history_fill: i32,
    friendship_threshold: i32,
}

impl Default for NeighborhoodTuning {
    fn default() -> Self {
        Self {
            arch_value_multiplier: DEFAULT_ARCH_VALUE_MULTIPLIER,
            new_family_start_hour: DEFAULT_NEW_FAMILY_START_HOUR,
            layout_history_fill: DEFAULT_LAYOUT_HISTORY_FILL,
            friendship_threshold: DEFAULT_FRIENDSHIP_THRESHOLD,
        }
    }
}

impl NeighborhoodTuning {
    fn from_program(program: &Program) -> Self {
        let mut tuning = Self::default();
        if let Some(global) = program.libraries.get(&program.global) {
            if let Some(fcns) = global.tuning.get(&NEIGHBORHOOD_TUNING) {
                if let Some(v) = find_tuning(fcns, "architecture value multiplier") {
                    tuning.arch_value_multiplier = v;
                }
                if let Some(v) = find_tuning(fcns, "new family start hour") {
                    tuning.new_family_start_hour = (v as i32) as i16;
                }
                if let Some(v) = find_tuning(fcns, "fill value for layout history") {
                    tuning.layout_history_fill = v as i32;
                }
            }
            if let Some(fcns) = global.tuning.get(&AUTONOMY_TUNING)
                && let Some(v) = find_tuning(fcns, "friendship threshold")
            {
                tuning.friendship_threshold = v as i32;
            }
        }
        tuning
    }

    fn from_install(install_root: &Path) -> Self {
        let Ok(install) = Install::detect(install_root) else {
            return Self::default();
        };
        let Ok(index) = Index::build(&install) else {
            return Self::default();
        };
        let Some((_, resource)) = index.named("Global.iff").next() else {
            return Self::default();
        };
        let Ok(bytes) = resource.read() else {
            return Self::default();
        };
        let Ok(iff) = Iff::parse(bytes) else {
            return Self::default();
        };
        let mut tuning = Self::default();
        if let Some(chunk) = iff.get(b"FCNS", NEIGHBORHOOD_TUNING)
            && let Ok(fcns) = sims_formats::tuning::Constants::parse(iff.data(chunk))
        {
            if let Some(v) = find_tuning(&fcns, "architecture value multiplier") {
                tuning.arch_value_multiplier = v;
            }
            if let Some(v) = find_tuning(&fcns, "new family start hour") {
                tuning.new_family_start_hour = (v as i32) as i16;
            }
            if let Some(v) = find_tuning(&fcns, "fill value for layout history") {
                tuning.layout_history_fill = v as i32;
            }
        }
        if let Some(chunk) = iff.get(b"FCNS", AUTONOMY_TUNING)
            && let Ok(fcns) = sims_formats::tuning::Constants::parse(iff.data(chunk))
            && let Some(v) = find_tuning(&fcns, "friendship threshold")
        {
            tuning.friendship_threshold = v as i32;
        }
        tuning
    }
}

fn find_tuning(fcns: &sims_formats::tuning::Constants, name: &str) -> Option<f32> {
    fcns.entries
        .iter()
        .find(|entry| entry.name.eq_ignore_ascii_case(name))
        .map(|entry| entry.value)
}

/// Architecture surface prices (`Build.iff` + pattern catalogs) needed by
/// `0x47b006` (`architecture_value`).
#[derive(Debug, Clone, Default)]
struct LotPrices {
    styles: BTreeMap<u16, i32>,
    pool: i32,
    floors: BTreeMap<u16, i32>,
    walls: BTreeMap<u16, i32>,
}

impl LotPrices {
    fn load(install: &Install, surfaces: &Surfaces) -> Self {
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
        let price = |table: u16, entry: u16| -> Option<i32> {
            let at = usize::from(entry).checked_sub(1)? * 3;
            Some(atoi(&tables.get(&table)?.entry(at, US_ENGLISH)?.raw_text))
        };
        let mut styles: BTreeMap<u16, i32> = PRICED_STYLES
            .iter()
            .zip(1u16..)
            .map(|(&style, entry)| (style, price(STYLE_TABLE, entry).unwrap_or(0)))
            .collect();
        styles.insert(SECOND_STYLE, styles[&PRICED_STYLES[0]]);
        let pool = price(POOL_TABLE, 1).unwrap_or(0);
        let mut floors = BTreeMap::new();
        for key in 1u16..=29 {
            floors.insert(key, price(FLOOR_TABLE, key).unwrap_or(0));
        }
        let mut walls = BTreeMap::new();
        for key in (0u16..=30).chain(248..=255) {
            walls.insert(key, price(WALL_TABLE, key).unwrap_or(0));
        }
        let plug_ins = |catalog: &SurfaceCatalog, default: i32, out: &mut BTreeMap<u16, i32>| {
            for file in catalog.files.iter().skip(1) {
                for &key in file.keys.values() {
                    out.insert(key, file.price.unwrap_or(default));
                }
            }
        };
        plug_ins(
            &surfaces.floors,
            price(FLOOR_DEFAULT_TABLE, 1).unwrap_or(0),
            &mut floors,
        );
        plug_ins(
            &surfaces.walls,
            price(WALL_DEFAULT_TABLE, 1).unwrap_or(0),
            &mut walls,
        );
        Self {
            styles,
            pool,
            floors,
            walls,
        }
    }
}

fn atoi(bytes: &[u8]) -> i32 {
    let mut i = 0;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let negative = match bytes.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut value = 0i64;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        value = value
            .wrapping_mul(10)
            .wrapping_add(i64::from(bytes[i] - b'0'));
        i += 1;
    }
    if negative {
        value.wrapping_neg() as i32
    } else {
        value as i32
    }
}

/// Computes the architecture value (`0x47b006`) over levels 1 and 2 and
/// returns `(architecture_value, has_architecture_for_global_22)`.
fn architecture_value_and_built(world: &World, prices: &LotPrices) -> (i32, bool) {
    let full = FULL_SIZE as usize;
    let size = usize::try_from(world.size).unwrap_or(0).min(full);
    let array = |id: u16, stride: usize| {
        world
            .arrays
            .get(&id)
            .filter(|a| a.stride == stride && a.data.len() == full * full * stride)
            .map(|a| a.data.as_slice())
    };
    let mut walls_sum = 0i32;
    let mut rest = 0i32;
    let mut built = false;
    let floor_price = |key: u16| prices.floors.get(&key).copied().unwrap_or(0);
    for level in 0..2 {
        let (Some(flags), Some(floors), Some(walls)) = (
            array(TILE_FLAGS[level], 1),
            array(FLOORS[level], 2),
            array(WALLS[level], WALL_CELL),
        ) else {
            continue;
        };
        for b in 0..size {
            for a in 0..size {
                let cell = a * full + b;
                if flags[cell] & TILE_LOCKED != 0 {
                    continue;
                }
                let wall = ExpandedWall::get(walls, size, a, b);
                let segments = u32::from(wall.segments);
                if segments != 0 {
                    built = true;
                }
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
                    if PATTERNED_WALL_STYLES.contains(&kind) {
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
                if floor != 0 {
                    built = true;
                }
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
                {
                    if pool[cell] < POOL_TILE_LIMIT {
                        built = true;
                        rest = rest.wrapping_add(prices.pool);
                    } else if pool[cell] == POOL_TILE_LIMIT {
                        // 0x47b1cc: pool byte 0x10 marks global 22 built at zero cost.
                        built = true;
                    }
                }
            }
        }
    }
    ((walls_sum / 2).wrapping_add(rest), built)
}

/// Runs `0x416085` (`0x444cb9` + `0x47b006`): recomputes `objects_value`,
/// `architecture_value`, globals 22/23/37, and family flags `(has_category_1,
/// has_category_9)`.
fn update_lot_values(
    world: &mut World,
    interpreter: &Interpreter<'_>,
    prices: &LotPrices,
) -> Result<(bool, bool), LifecycleError> {
    let mut objects_value = 0i32;
    let mut arch_objects_value = 0i32;
    let mut has_cat1 = false;
    let mut has_cat9 = false;
    let mut built_from_objects = false;

    for id in world.order.clone() {
        let Some(object) = world.objects.get(&id) else {
            continue;
        };
        let category = object.data.get(DATA_CATEGORY).copied().unwrap_or(0);
        let lead = object
            .multipart
            .as_ref()
            .is_none_or(|part| part.parent == 0 || part.parent as u16 == id);
        let Ok(words) = interpreter.words(world, id).copied() else {
            continue;
        };
        if category == 1 {
            if crate::placement::stands_on_lot(world, id).unwrap_or(false) {
                has_cat1 = true;
            }
        } else if category == 9 {
            has_cat9 = true;
        }
        if (words[word::GROUP] != 0 && !lead) || words[word::TYPE] == DEF_TYPE_SKIPPED {
            continue;
        }
        let on_lot = crate::placement::stands_on_lot(world, id).unwrap_or(false);
        if on_lot && DEF_BUILT.iter().any(|&w| words[w] != 0) {
            let [a, b, level] = World::tile(world.objects[&id].position);
            if crate::placement::tile_flags(world, a, b, i32::from(level)).unwrap_or(TILE_LOCKED)
                & TILE_LOCKED
                == 0
            {
                built_from_objects = true;
            }
        }
        let valued = interpreter.valued(world, id).unwrap_or(false);
        let val = i32::from(object_value(world, &words, id).unwrap_or(0));
        if on_lot && valued {
            objects_value = objects_value.wrapping_add(val);
        } else if let Some(def) = crate::interpreter::object_definition(
            interpreter.catalog,
            world,
            world.objects[&id].guid,
        ) && let Ok(Some(master)) = interpreter
            .program
            .master_definition(def.resource, def.chunk)
            && !matches!(master[DEF_BUILD_KIND], 0 | 4)
        {
            arch_objects_value = arch_objects_value.wrapping_add(val);
        }
    }

    let (arch_surfaces, built_from_arch) = architecture_value_and_built(world, prices);
    world.budget.objects_value = objects_value;
    world.budget.architecture_value = arch_surfaces.wrapping_add(arch_objects_value);
    world
        .update_statistics()
        .map_err(|e| LifecycleError::Lot(e.to_string()))?;
    if let Some(slot) = world.globals.get_mut(BUILT) {
        *slot = i16::from(built_from_objects || built_from_arch);
    }
    Ok((has_cat1, has_cat9))
}

/// Broadcasts a named BHAV tree (`0x444741`: `"evict lot"` / `"reset lot"`) to
/// every registered object whose own resource defines a BHAV of that name.
fn broadcast_tree(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    name: &str,
    arg0: i16,
) -> Result<usize, Fault> {
    let mut ran = 0usize;
    for id in world.order.clone() {
        let Some(guid) = world.objects.get(&id).map(|o| o.guid) else {
            continue;
        };
        let Some(definition) =
            crate::interpreter::object_definition(interpreter.catalog, world, guid)
        else {
            continue;
        };
        let Some(library) = interpreter.program.libraries.get(&definition.resource) else {
            continue;
        };
        let tree_id = library
            .names
            .iter()
            .find(|(_, label)| label.eq_ignore_ascii_case(name))
            .map(|(&chunk_id, _)| chunk_id);
        if let Some(tree_id) = tree_id {
            let _ = interpreter.run_tree(
                world,
                id,
                tree_id,
                0,
                &[arg0, 0, 0, 0],
                crate::TURN_BUDGET,
            )?;
            ran += 1;
        }
    }
    Ok(ran)
}

/// Computes the time of day for an hour (`0x46d525`).
fn time_of_day(hours: i16) -> i16 {
    match hours {
        ..=5 => 2,
        6 => 3,
        7..=17 => 0,
        18 => 1,
        _ => 2,
    }
}

/// Resets budget history (`0x46d97a`) and days played (`global 28`).
fn reset_budget_history(world: &mut World) {
    world.budget.days = vec![Some([0; 8]); 6];
    if let Some(days) = world.globals.get_mut(DAYS_PLAYED) {
        *days = 0;
    }
}

/// Resets house usage history (`0x4165f3`) with `fill` (`FCNS #4 "fill value
/// for layout history"`, default `200`).
fn reset_house_usage(world: &mut World, fill: i32) {
    let day = Usage {
        total: fill,
        routing: fill,
    };
    world.usage = HouseUsage {
        history: [day; 5],
        today: Usage::default(),
    };
}

/// Applies the `0x42848d..0x4284f9` new move-in reset when a family's
/// [`FAMILY_FLAG_NEW_MOVE_IN`] bit is set.
fn apply_move_in_reset(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    tuning: NeighborhoodTuning,
) -> Result<(), LifecycleError> {
    // 0x46d86f: set clock to (new_family_start_hour, 0), update time_of_day.
    if let Some(slot) = world.globals.get_mut(HOURS) {
        *slot = tuning.new_family_start_hour;
    }
    if let Some(slot) = world.globals.get_mut(MINUTES) {
        *slot = 0;
    }
    let tod = time_of_day(tuning.new_family_start_hour);
    if world.globals.get(TIME_OF_DAY).copied() != Some(tod) {
        if let Some(slot) = world.globals.get_mut(TIME_OF_DAY) {
            *slot = tod;
        }
        world.mark_all_rooms();
    }
    // 0x46d97a: reset budget history + days played.
    reset_budget_history(world);
    // 0x46d83a: unpause.
    if let Some(slot) = world.globals.get_mut(PAUSED) {
        *slot = 0;
    }
    // 0x4165f3: clear house usage history.
    reset_house_usage(world, tuning.layout_history_fill);
    // 0x4284f4: broadcast "reset lot".
    broadcast_tree(interpreter, world, "reset lot", 0)
        .map_err(|e| LifecycleError::Lot(format!("broadcasting \"reset lot\": {e}")))?;
    Ok(())
}

/// Runs `0x4283cf..0x4285f9` (end of household selection) with full support for
/// move-in flag 4 resets and stray-character removal.
#[allow(clippy::too_many_arguments)]
fn finish_selection_for_lifecycle(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    house: u32,
    family_id: u16,
    family_flags: u32,
    members: &[u32],
    residents_before: &BTreeSet<u16>,
    tuning: NeighborhoodTuning,
) -> Result<(), LifecycleError> {
    let house_i32 = i32::try_from(house).map_err(|e| LifecycleError::Lot(e.to_string()))?;
    let is_character = |world: &World, guid: u32| {
        interpreter.program.characters.contains(&guid) || world.characters.made.contains_key(&guid)
    };
    let person_word = |world: &World, id: u16, index: usize| -> i16 {
        world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.data.get(index).copied())
            .unwrap_or(0)
    };

    // 1. Delete stray neighborhood characters of no family whose word 67 != house (0x4283e1).
    let stray_unfamilied: Vec<u16> = world
        .people
        .iter()
        .copied()
        .filter(|&id| {
            let Some(obj) = world.objects.get(&id) else {
                return false;
            };
            is_character(world, obj.guid)
                && person_word(world, id, PERSON_FAMILY) == 0
                && i32::from(person_word(world, id, PERSON_HOME)) != house_i32
        })
        .collect();
    for id in stray_unfamilied {
        if world.objects.contains_key(&id) {
            remove_person_cleanly(interpreter, world, id)?;
        }
    }

    // 2. If the family just moved in (flag 4), run the move-in reset (0x42848d).
    if house != 0 && family_id != 0 && family_flags & FAMILY_FLAG_NEW_MOVE_IN != 0 {
        apply_move_in_reset(interpreter, world, tuning)?;
    }

    // 3. Recompute visitor word 32 (0x42a2b4).
    let lot = world.globals.get(10).copied().unwrap_or(0);
    let residential = lot == 0 || interpreter.program.zoning.get(&lot) == Some(&1);
    let mut became_visitor_characters = Vec::new();
    for id in world.people.clone() {
        let Some(obj) = world.objects.get(&id) else {
            continue;
        };
        let guid = obj.guid;
        let visitor = visitor_again(
            person_word(world, id, PERSON_VISITOR),
            members.contains(&guid),
            residential,
        );
        if let Some(data) = world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .and_then(|p| p.data.get_mut(PERSON_VISITOR))
        {
            *data = visitor;
        }
        if is_character(world, guid) && residents_before.contains(&id) && visitor != 0 {
            became_visitor_characters.push(id);
        }
    }

    // 4. Delete former resident characters who became visitors (0x428550..0x4285f4).
    for id in became_visitor_characters {
        if world.objects.contains_key(&id) {
            remove_person_cleanly(interpreter, world, id)?;
        }
    }
    Ok(())
}

/// Safely saves a person to its neighbor record, clears any lingering route /
/// selection / object-use state that could block deletion, and removes it via
/// `remove_now` (`0x44123f`).
fn remove_person_cleanly(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    id: u16,
) -> Result<(), LifecycleError> {
    if !world.objects.contains_key(&id) {
        return Ok(());
    }
    let _ = people::save_to_record(world, id);
    if selected(world) == Some(id) {
        interpreter
            .select(world, None)
            .map_err(|e| LifecycleError::Lot(format!("deselecting person {id}: {e}")))?;
    }
    // Clear route contexts on this person and any routes targeting it before
    // `remove_now`, matching post-reload deletion preconditions.
    if let Some(route) = world.routing.routes.get_mut(&id) {
        route.contexts.clear();
    }
    for route in world.routing.routes.values_mut() {
        route.contexts.retain(|c| c.target != id);
        for c in &mut route.contexts {
            if c.objects[1] == id {
                c.byte_84 = true;
                c.objects[1] = 0;
                c.action = 0;
            }
        }
    }
    // If another person on the lot is seated/off-lot/wide and currently
    // referencing `id`, recover that person first via `reset_person_after_purge`
    // so `remove_now`'s `drop_references` (`0x444306` -> `0x45e8dd`) does not
    // trap on an unported seated/off-lot person reset.
    let id_i16 = id as i16;
    let mut blocked_others = Vec::new();
    for other in world.people.clone() {
        if other == id {
            continue;
        }
        let (references_id, wide) = {
            let Some(person) = world.objects.get(&other).and_then(|o| o.person.as_ref()) else {
                continue;
            };
            let references = person.object_uses.iter().any(|u| u.target == id_i16)
                || person.active.target == id_i16
                || person.active.icon == id_i16
                || person.active.caller == id_i16;
            let wide = person.data.get(73).copied().unwrap_or(0) != 0;
            (references, wide)
        };
        if !references_id {
            continue;
        }
        let in_container =
            crate::placement::capture(world, other).is_ok_and(|place| place.container.is_some());
        let off_lot = !crate::placement::stands_on_lot(world, other).unwrap_or(true);
        if in_container || off_lot || wide {
            blocked_others.push(other);
        }
    }
    for other in blocked_others {
        if let Some(person) = world
            .objects
            .get_mut(&other)
            .and_then(|o| o.person.as_mut())
        {
            person.queue.retain(|action| {
                action.target != id_i16 && action.icon != id_i16 && action.caller != id_i16
            });
            if person.last.target == id_i16 || person.last.caller == id_i16 {
                person.last = crate::object_uses::no_action();
            } else if person.last.icon == id_i16 {
                person.last.icon = 0;
            }
        }
        interpreter
            .reset_person_after_purge(world, other)
            .map_err(|e| {
                LifecycleError::Lot(format!(
                    "releasing seated/off-lot person {other} from {id}: {e}"
                ))
            })?;
    }
    // Release `id`'s own object uses (decrementing target `data[62]` counts)
    // and clear its queue; if `id` itself is seated inside a container, unlink
    // it off-lot first so `reset_person_to_delete` does not trap at `0x45e8dd`.
    people::clear_person_uses_and_counts(world, id);
    if let Some(person) = world
        .objects
        .get_mut(&id)
        .and_then(|obj| obj.person.as_mut())
    {
        person.queue.clear();
    }
    if crate::placement::capture(world, id).is_ok_and(|place| place.container.is_some()) {
        let _ = crate::placement::unlink_off_lot(world, id);
    }
    match interpreter.remove_now(world, id) {
        Ok(()) => Ok(()),
        Err(crate::fault::Fault {
            trap: crate::fault::Trap::Unsupported(_),
            ..
        }) => {
            let (people_to_reset, objects_to_reset) =
                purge_orphaned_person(interpreter, world, id)?;
            for other in people_to_reset {
                if world
                    .objects
                    .get(&other)
                    .is_some_and(|o| o.person.is_some())
                {
                    interpreter
                        .reset_person_after_purge(world, other)
                        .map_err(|e| {
                            LifecycleError::Lot(format!(
                                "resetting person {other} after fallback removal of {id}: {e}"
                            ))
                        })?;
                }
            }
            for other in objects_to_reset {
                if world.objects.contains_key(&other) {
                    interpreter.reset(world, other, false).map_err(|e| {
                        LifecycleError::Lot(format!(
                            "resetting object {other} after fallback removal of {id}: {e}"
                        ))
                    })?;
                }
            }
            Ok(())
        }
        Err(e) => Err(LifecycleError::Lot(format!("removing person {id}: {e}"))),
    }
}

/// Removes an orphaned live person whose neighborhood record (and possibly its
/// character file in `NEIGHBORHOOD_CHARACTERS`) was deleted while another lot
/// was saved (e.g. a visitor whose home lot was bulldozed via `0x426fcd`).
///
/// Always tears down the person's lot state directly rather than running
/// `remove_now` / BHAV Reset callbacks, because the person's neighborhood
/// record is already gone and its character `.iff` file may have been removed.
fn purge_orphaned_person(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    id: u16,
) -> Result<(Vec<u16>, Vec<u16>), LifecycleError> {
    let Some(deleted_guid) = world.objects.get(&id).map(|o| o.guid) else {
        return Ok((Vec::new(), Vec::new()));
    };
    // Release any surviving objects currently marked in use by `id` (decrement
    // their data word 62 use count) before tearing down `id`.
    people::clear_person_uses_and_counts(world, id);
    // Unlink cleanly from the object's tile (ground or upper floor) or
    // container FIRST so any surviving object behind `id` in `slot 0` is
    // promoted into `id`'s tile/container head (`placement::relink` / 0x4329c1).
    let _ = crate::placement::unlink_off_lot(world, id);
    if world.event_owner == id {
        world.event_owner = 0;
    }
    if selected(world) == Some(id) {
        if let Some(obj) = world.objects.get_mut(&id) {
            obj.selected = false;
        }
        if let Some(slot) = world.globals.get_mut(3) {
            *slot = 0;
        }
    }
    let held_ids: Vec<u16> = world
        .objects
        .get(&id)
        .map(|o| {
            o.slots
                .iter()
                .skip(1)
                .filter_map(|s| u16::try_from(s.object).ok())
                .filter(|&held| held != 0)
                .collect()
        })
        .unwrap_or_default();
    for held in held_ids {
        if !world.objects.contains_key(&held) {
            continue;
        }
        if world.indicators.contains(&held) {
            if let Some(ind) = world.objects.get_mut(&held) {
                ind.position = [-16, -16, 1];
                if let Some(slot0) = ind.slots.first_mut() {
                    slot0.object = 0;
                }
            }
            continue;
        }
        let held_guid = world.objects[&held].guid;
        if world.types.values().any(|&g| g == held_guid)
            && crate::interpreter::object_definition(interpreter.catalog, world, held_guid)
                .is_some()
        {
            let _ = interpreter.remove_now(world, held);
        } else if let Some(obj) = world.objects.get_mut(&held)
            && let Some(slot0) = obj.slots.first_mut()
        {
            slot0.object = 0;
        }
    }
    let id_i16 = id as i16;
    let mut people_to_reset = Vec::new();
    let mut objects_to_reset = Vec::new();
    let deleted_types: BTreeSet<i16> = world
        .types
        .iter()
        .filter_map(|(&ty, &guid)| {
            if guid == deleted_guid
                || crate::interpreter::object_definition(interpreter.catalog, world, guid).is_none()
            {
                i16::try_from(ty).ok()
            } else {
                None
            }
        })
        .collect();
    for (&other_id, obj) in &mut world.objects {
        if other_id == id {
            continue;
        }
        for slot in &mut obj.slots {
            if slot.object == id_i16 {
                slot.object = 0;
            }
        }
        let stack_hits_deleted = obj.stack.iter().any(|frame| {
            frame.stack_object == id_i16 || deleted_types.contains(&frame.code_owner_type)
        });
        if let Some(person) = obj.person.as_mut() {
            let uses_deleted = person.object_uses.iter().any(|used| used.target == id_i16);
            let active_hits_deleted = person.active.target == id_i16
                || person.active.icon == id_i16
                || person.active.caller == id_i16;
            if uses_deleted || active_hits_deleted || stack_hits_deleted {
                people_to_reset.push(other_id);
                person.active = crate::object_uses::no_action();
            }
            person.object_uses.retain(|used| used.target != id_i16);
            person.queue.retain(|action| {
                action.target != id_i16 && action.icon != id_i16 && action.caller != id_i16
            });
            if person.last.target == id_i16 || person.last.caller == id_i16 {
                person.last = crate::object_uses::no_action();
            } else if person.last.icon == id_i16 {
                person.last.icon = 0;
            }
        } else if stack_hits_deleted {
            objects_to_reset.push(other_id);
        }
    }
    world.objects.remove(&id);
    world.frame_storage.remove(&id);
    world.headlines.remove(&id);
    world.fires.retain(|&other| other != id);
    world.order.retain(|&other| other != id);
    world.people.retain(|&other| other != id);
    world.removals.retain(|&other| other != id);
    for instances in world.instances.values_mut() {
        instances.retain(|&other| other != id);
    }
    world.animators.remove(&id);
    world.base_layers.remove(&id);
    world.translated_bones.remove(&id);
    world.held_poses.remove(&id);
    world.stops_asked.remove(&id);
    world.person_ticks.remove(&id);
    world.routing.routes.remove(&id);
    for route in world.routing.routes.values_mut() {
        route.contexts.retain(|c| c.target != id);
        for c in &mut route.contexts {
            if c.objects[1] == id {
                c.byte_84 = true;
                c.objects[1] = 0;
                c.action = 0;
            }
        }
    }
    world.routing.slot_flags.retain(|&(obj, _), _| obj != id);
    world.routing.slots.retain(|&(obj, _), _| obj != id);
    world.routing.unknown_words.retain(|&(obj, _)| obj != id);
    let target = i32::from(id_i16);
    for obj in world.objects.values_mut() {
        obj.relationships.retain(|row| row.target != target);
    }
    // Clear any lingering grid head on either floor (ground array 3 or upper
    // array 103) in case a saved lot had an inconsistent grid cell pointing to
    // the purged person.
    let id_bytes = id_i16.to_le_bytes();
    for grid_id in crate::placement::LEVEL_GRIDS {
        if let Some(grid) = world.arrays.get_mut(&grid_id)
            && grid.stride == 2
        {
            for cell in grid.data.as_chunks_mut::<2>().0 {
                if *cell == id_bytes {
                    *cell = [0, 0];
                }
            }
        }
    }
    world.repair_grid().map_err(|e| {
        LifecycleError::Lot(format!("repairing grid after purging person {id}: {e}"))
    })?;
    Ok((people_to_reset, objects_to_reset))
}

/// Removes any saved `ObjM` instance from a base house file whose character
/// GUID was permanently deleted when another lot was bulldozed (`0x426fcd`) and
/// no longer has a catalog definition, promoting any `slot 0` tile/chain
/// successor, decrementing `data[62]` use counts on surviving objects it used,
/// and scrubbing references in surviving instances.
fn strip_deleted_base_instances(
    world: &mut World,
    objects: &mut sims_formats::saved_objects::Objects,
    deleted_characters: &BTreeSet<u32>,
    deleted_at_epoch: &BTreeMap<u32, u64>,
) -> (BTreeSet<u16>, BTreeSet<u16>) {
    let deleted_ids: Vec<u16> = objects
        .order
        .iter()
        .copied()
        .filter(|id| {
            objects.instances.get(id).is_some_and(|inst| {
                deleted_characters.contains(&inst.guid)
                    || deleted_at_epoch.get(&inst.guid).is_some_and(|&e| e > 0)
                    || (inst.person.is_some() && world.neighbors.first_of(inst.guid).is_none())
            })
        })
        .collect();
    let mut people_to_reset = BTreeSet::new();
    let mut objects_to_reset = BTreeSet::new();
    if deleted_ids.is_empty() {
        return (people_to_reset, objects_to_reset);
    }
    let full = FULL_SIZE as usize;
    for id in deleted_ids {
        let Some(removed) = objects.instances.remove(&id) else {
            continue;
        };
        objects.order.retain(|&other| other != id);
        objects.types_by_id.remove(&id);
        let id_i16 = id as i16;
        let deleted_type_id = removed.type_id as i16;

        // Decrement data[62] on any surviving object marked in use by `removed`.
        if let Some(person) = &removed.person {
            for used in &person.object_uses {
                if let Ok(target_id) = u16::try_from(used.target)
                    && let Some(target_obj) = objects.instances.get_mut(&target_id)
                    && let Some(users) = target_obj.data.get_mut(62)
                    && *users > 0
                {
                    *users -= 1;
                }
            }
        }

        // Promote any surviving `slot 0` successor into `removed`'s tile grid
        // cell or predecessor slot.
        let next_id = removed
            .slots
            .first()
            .and_then(|s| u16::try_from(s.object).ok())
            .filter(|&n| n != 0 && objects.instances.contains_key(&n));
        let slot_word = removed.data.get(14).copied().unwrap_or(-1);
        let before_word = removed.data.get(26).copied().unwrap_or(0);
        if slot_word >= 0
            && let Ok(parent_id) = u16::try_from(before_word)
            && let Some(parent) = objects.instances.get_mut(&parent_id)
            && let Some(slot_entry) = parent.slots.get_mut(slot_word as usize)
            && slot_entry.object == id_i16
        {
            slot_entry.object = next_id.map_or(0, |n| n as i16);
        } else {
            let level = if removed.position[2] == 2 { 1 } else { 0 };
            let grid_id = crate::placement::LEVEL_GRIDS[level];
            let tx = removed.position[0].div_euclid(16);
            let ty = removed.position[1].div_euclid(16);
            if tx >= 0
                && ty >= 0
                && (tx as usize) < full
                && (ty as usize) < full
                && let Some(grid) = world.arrays.get_mut(&grid_id)
                && grid.stride == 2
            {
                let cell = (tx as usize * full + ty as usize) * 2;
                if let Some(bytes) = grid.data.get_mut(cell..cell + 2)
                    && bytes == id_i16.to_le_bytes()
                {
                    let replacement = next_id.unwrap_or(0u16);
                    bytes.copy_from_slice(&replacement.to_le_bytes());
                }
            }
        }
        if let Some(next) = next_id
            && let Some(next_obj) = objects.instances.get_mut(&next)
        {
            if let Some(s) = next_obj.data.get_mut(14) {
                *s = slot_word;
            }
            if let Some(b) = next_obj.data.get_mut(26) {
                *b = before_word;
            }
        }

        // Scrub any remaining references to `id` or its deleted type in
        // surviving saved instances.
        let target_i32 = i32::from(id_i16);
        for (&other_id, inst) in &mut objects.instances {
            for slot in &mut inst.slots {
                if slot.object == id_i16 {
                    slot.object = 0;
                }
            }
            inst.relationships.retain(|r| r.target != target_i32);
            let stack_hits = inst
                .stack
                .iter()
                .any(|f| f.stack_object == id_i16 || f.code_owner_type == deleted_type_id);
            if let Some(person) = inst.person.as_mut() {
                let uses_hit = person.object_uses.iter().any(|u| u.target == id_i16);
                let active_hit = person.active.target == id_i16
                    || person.active.icon == id_i16
                    || person.active.caller == id_i16;
                if uses_hit || active_hit || stack_hits {
                    people_to_reset.insert(other_id);
                    person.active = crate::object_uses::no_action();
                }
                person.object_uses.retain(|u| u.target != id_i16);
                person
                    .queue
                    .retain(|a| a.target != id_i16 && a.icon != id_i16 && a.caller != id_i16);
                if person.last.target == id_i16 || person.last.caller == id_i16 {
                    person.last = crate::object_uses::no_action();
                } else if person.last.icon == id_i16 {
                    person.last.icon = 0;
                }
            } else if stack_hits {
                objects_to_reset.insert(other_id);
            }
        }

        // Clear any lingering grid cell on either floor pointing to `id`.
        let id_bytes = id_i16.to_le_bytes();
        for grid_id in crate::placement::LEVEL_GRIDS {
            if let Some(grid) = world.arrays.get_mut(&grid_id)
                && grid.stride == 2
            {
                for cell in grid.data.as_chunks_mut::<2>().0 {
                    if *cell == id_bytes {
                        *cell = [0, 0];
                    }
                }
            }
        }
    }
    (people_to_reset, objects_to_reset)
}

/// Computes the purchase price of `house` in `neighborhood_id` (`0x427f78`):
/// reads the latest saved lot if present in REDB, otherwise reads the base
/// `HouseXX.iff`'s `SIMI` chunk, from a single atomic REDB read snapshot.
pub fn lot_price(
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    house: u32,
    arch_multiplier: f32,
) -> Result<i32, LifecycleError> {
    let snapshot = store
        .load_lot_snapshot(neighborhood_id, house)?
        .ok_or_else(|| LifecycleError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    if let Some(saved_bytes) = snapshot.saved_lot {
        let saved = Saved::decode(&saved_bytes)
            .map_err(|e| LifecycleError::Lot(format!("decoding saved house {house}: {e}")))?;
        let budget = &saved.world.budget;
        return Ok(lot_total_value(
            budget.lot_value,
            budget.objects_value,
            budget.architecture_value,
            arch_multiplier,
        ));
    }
    let Some(base_bytes) = snapshot.base_house else {
        return Err(LifecycleError::HouseNotFound {
            neighborhood: neighborhood_id.to_owned(),
            house,
        });
    };
    let iff = Iff::parse(base_bytes)
        .map_err(|e| LifecycleError::Lot(format!("{neighborhood_id} House{house:02}: {e}")))?;
    let chunk = iff
        .chunks
        .iter()
        .find(|c| &c.tag == b"SIMI")
        .ok_or_else(|| {
            LifecycleError::Lot(format!(
                "{neighborhood_id} House{house:02}: missing SIMI chunk"
            ))
        })?;
    let sim = sims_formats::lot::Simulator::parse(iff.data(chunk))
        .map_err(|e| LifecycleError::Lot(format!("{neighborhood_id} House{house:02} SIMI: {e}")))?;
    Ok(lot_total_value(
        sim.lot_value,
        sim.objects_value,
        sim.architecture_value,
        arch_multiplier,
    ))
}

/// Counts a household's MagiCoins from its members' `NGBH` inventories,
/// matching `Sims.exe` `0x410282`: for each member GUID in order, looks up the
/// first neighbor record matching that GUID (`0x42571f`), finds its inventory
/// (`0x42a8a1`), and sums the first `(kind = 7, guid = 0x99e81bec)` token count
/// (`0x41f660`), sign-extended through 16 bits (`0x4102fc`).
pub fn family_magic_coins(stored: &StoredNeighborhood, family_id: u16) -> i32 {
    let Some(family) = stored.families.get(&family_id) else {
        return 0;
    };
    let mut sum = 0i32;
    for &guid in &family.members {
        if guid == 0 {
            continue;
        }
        let Some(record) = stored
            .neighbors
            .iter()
            .flatten()
            .find(|record| record.guid == guid)
        else {
            continue;
        };
        let Some(inventory) = stored
            .inventories
            .iter()
            .flatten()
            .find(|inv| inv.neighbor == record.id)
        else {
            continue;
        };
        if let Some(item) = inventory
            .items
            .iter()
            .find(|item| item.kind == MAGIC_COIN_TOKEN_KIND && item.guid == MAGIC_COIN_TOKEN_GUID)
        {
            sum = sum.wrapping_add(i32::from(item.count as i16));
        }
    }
    i32::from(sum as i16)
}

/// Returns the MagiCoin cost for moving into `house`, matching `Sims.exe`
/// `0x5500c5..0x550124`:
/// - `91` (`0x5b`, Alfred's B & B): `UIText.iff STR# 134[32]` (default `1500`)
/// - `90` / `92` (EverAfter Oaks / Old Man Lum's): `STR# 134[33]` (default `1000`)
/// - All other lots: `0`.
pub fn magic_town_lot_cost(install_root: Option<&Path>, house: u32) -> i32 {
    if !MAGIC_TOWN_RESIDENTIAL_LOTS.contains(&house) {
        return 0;
    }
    let (idx, fallback) = if house == 91 {
        (32usize, DEFAULT_MAGIC_LOT_91_COST)
    } else {
        (33usize, DEFAULT_MAGIC_LOT_OTHER_COST)
    };
    let Some(root) = install_root else {
        return fallback;
    };
    let Ok(install) = Install::detect(root) else {
        return fallback;
    };
    let Ok(path) = install.file("GameData/UIText.iff") else {
        return fallback;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return fallback;
    };
    let Ok(iff) = Iff::parse(bytes) else {
        return fallback;
    };
    let Some(chunk) = iff.get(b"STR#", UITEXT_HOUSE_INFO_TABLE) else {
        return fallback;
    };
    let Ok(table) = StringTable::parse(iff.data(chunk)) else {
        return fallback;
    };
    table
        .get(idx, US_ENGLISH)
        .and_then(|s| s.trim().parse::<i32>().ok())
        .unwrap_or(fallback)
}

/// Deducts `amount` MagiCoins across `family_id`'s members' inventories in
/// member order, matching `Sims.exe` `0x556467` (`0x41f660` + `0x41f49b`).
fn deduct_family_magic_coins(stored: &mut StoredNeighborhood, family_id: u16, mut remaining: i32) {
    if remaining <= 0 {
        return;
    }
    let Some(members) = stored.families.get(&family_id).map(|f| f.members.clone()) else {
        return;
    };
    for guid in members {
        if remaining <= 0 {
            break;
        }
        if guid == 0 {
            continue;
        }
        let Some(neighbor_id) = stored
            .neighbors
            .iter()
            .flatten()
            .find(|record| record.guid == guid)
            .map(|record| record.id)
        else {
            continue;
        };
        let Some(inventory) = stored
            .inventories
            .iter_mut()
            .flatten()
            .find(|inv| inv.neighbor == neighbor_id)
        else {
            continue;
        };
        if let Some(pos) = inventory.items.iter().position(|item| {
            item.kind == MAGIC_COIN_TOKEN_KIND && item.guid == MAGIC_COIN_TOKEN_GUID
        }) {
            let have = i32::from(inventory.items[pos].count as i16);
            if have <= 0 {
                continue;
            }
            let take = remaining.min(have);
            let left = have - take;
            if left <= 0 {
                inventory.items.remove(pos);
            } else {
                inventory.items[pos].count = left as u16;
            }
            remaining -= take;
        }
    }
}

/// Moves a Family Bin household (`house == 0`) onto an unoccupied residential
/// lot (`zoning == 1`), validating funds against `lot_price` using the
/// install's `FCNS #4` `"architecture value multiplier"` and (for Magic Town
/// residential lots `90..=92`) validating and deducting MagiCoins.
pub fn move_in(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    family_id: u16,
    house: u32,
) -> Result<MoveInReport, LifecycleError> {
    let tuning = NeighborhoodTuning::from_install(install_root);
    let magic_cost = magic_town_lot_cost(Some(install_root), house);
    move_in_inner(
        store,
        neighborhood_id,
        family_id,
        house,
        tuning.arch_value_multiplier,
        magic_cost,
    )
}

/// Like [`move_in`], taking an explicit `arch_multiplier` (e.g.
/// [`DEFAULT_ARCH_VALUE_MULTIPLIER`]) so callers and unit tests can execute
/// Move-In without needing an install directory on disk.
pub fn move_in_with_multiplier(
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    family_id: u16,
    house: u32,
    arch_multiplier: f32,
) -> Result<MoveInReport, LifecycleError> {
    let magic_cost = magic_town_lot_cost(None, house);
    move_in_inner(
        store,
        neighborhood_id,
        family_id,
        house,
        arch_multiplier,
        magic_cost,
    )
}

fn move_in_inner(
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    family_id: u16,
    house: u32,
    arch_multiplier: f32,
    required_magic: i32,
) -> Result<MoveInReport, LifecycleError> {
    let mut stored = store
        .load_neighborhood(neighborhood_id)?
        .ok_or_else(|| LifecycleError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    let origin_instance_id = stored.instance_id;
    let origin_revision = stored.revision;
    if family_id == 0 {
        return Err(LifecycleError::FamilyEmpty { family: family_id });
    }
    let family = stored
        .families
        .get(&family_id)
        .ok_or_else(|| LifecycleError::FamilyNotFound {
            neighborhood: neighborhood_id.to_owned(),
            family: family_id,
        })?;
    if family.members.is_empty() {
        return Err(LifecycleError::FamilyEmpty { family: family_id });
    }
    if family.house != 0 || family.secondary_house() != 0 {
        let occupied = if family.house != 0 {
            family.house
        } else {
            family.secondary_house()
        };
        return Err(LifecycleError::FamilyNotInBin {
            family: family_id,
            house: occupied,
        });
    }
    let house_i16 = i16::try_from(house).ok();
    let zoning = house_i16.and_then(|lot| stored.zoning.get(&lot).copied());
    if house == 0 || zoning != Some(1) {
        return Err(LifecycleError::LotNotResidential { house, zoning });
    }
    if let Some((occupant, _)) = stored.family_in(house) {
        return Err(LifecycleError::LotOccupied {
            house,
            family: occupant,
        });
    }
    let required = lot_price(store, neighborhood_id, house, arch_multiplier)?;
    let budget_before = family.budget;
    if budget_before < required {
        return Err(LifecycleError::InsufficientFunds {
            family: family_id,
            house,
            available: budget_before,
            required,
        });
    }
    if required_magic > 0 {
        let available_magic = family_magic_coins(&stored, family_id);
        if available_magic < required_magic {
            return Err(LifecycleError::InsufficientMagicCoins {
                family: family_id,
                house,
                available: available_magic,
                required: required_magic,
            });
        }
    }

    let family = stored.families.get_mut(&family_id).expect("checked above");
    // 0x4282ff..0x428308 + 0x42735b..0x427376:
    family.budget = budget_before
        .wrapping_sub(required)
        .wrapping_add(family.architecture_value);
    family.architecture_value = required;
    family.house = i32::try_from(house).map_err(|e| LifecycleError::Lot(e.to_string()))?;
    family.flags = (family.flags & !FAMILY_FLAG_BIT_3) | FAMILY_FLAG_NEW_MOVE_IN;
    let budget_after = family.budget;
    if required_magic > 0 {
        deduct_family_magic_coins(&mut stored, family_id, required_magic);
    }
    stored.revision = stored.revision.saturating_add(1);

    let (committed_instance_id, committed_revision) = match store.save_neighborhood_checked(
        neighborhood_id,
        origin_instance_id,
        origin_revision,
        &stored,
    )? {
        Ok(version) => version,
        Err(CheckedCommitError::InstanceMismatch { expected, current }) => {
            return Err(LifecycleError::NeighborhoodMismatch {
                expected: format!("{neighborhood_id}#{expected:032x}"),
                actual: format!("{neighborhood_id}#{current:032x}"),
            });
        }
        Err(CheckedCommitError::RevisionMismatch { expected, current }) => {
            return Err(LifecycleError::StaleNeighborhood {
                neighborhood: neighborhood_id.to_owned(),
                expected_revision: expected,
                current_revision: current,
            });
        }
    };
    Ok(MoveInReport {
        family: family_id,
        house,
        lot_price: required,
        budget_before,
        budget_after,
        architecture_value: required,
        committed_instance_id,
        committed_revision,
    })
}

/// Shared install + neighborhood resources built when opening a lot from REDB.
struct StoreRuntime {
    install: Install,
    index: Index,
    catalog: Catalog,
    program: Program,
    surfaces: Surfaces,
    tuning: NeighborhoodTuning,
    stored: StoredNeighborhood,
}

impl StoreRuntime {
    fn from_snapshot(
        install_root: &Path,
        stored: StoredNeighborhood,
        character_files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Self, LifecycleError> {
        let install =
            Install::detect(install_root).map_err(|e| LifecycleError::Lot(e.to_string()))?;
        let mut index = Index::build(&install).map_err(|e| LifecycleError::Lot(e.to_string()))?;
        if !character_files.is_empty() {
            index
                .add_character_files(character_files)
                .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        }
        let catalog = Catalog::build(&index).map_err(|e| LifecycleError::Lot(e.to_string()))?;
        let context = Context::COMPLETE_COLLECTION;
        let mut program = Program::link(&index, &catalog, catalog.objects.iter().map(|o| o.guid))
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        program.zoning = stored.zoning.clone();
        program.expansions = context.expansions;
        program.characters = catalog
            .objects
            .iter()
            .map(|o| o.guid)
            .filter(|&guid| {
                catalog.object(guid).is_some_and(|o| {
                    index.mounts[index.resources[o.resource].mount].path == Index::CHARACTERS
                })
            })
            .collect();
        program.animations =
            Animations::load(&install, &index).map_err(|e| LifecycleError::Lot(e.to_string()))?;
        let person_types = catalog
            .objects
            .iter()
            .filter(|o| o.kind == 2)
            .map(|o| o.guid);
        program.people = People::load_each(&install, &index, &catalog, person_types)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?
            .0;
        program.icon_frames = crate::program::icon_frames(&install)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        program.wardrobe = Some(match sims_content::Assets::scan(&install.root) {
            Ok(assets) => sims_content::Wardrobe::load(&assets),
            Err(error) => sims_content::Wardrobe::unread(error),
        });
        if !character_files.is_empty() {
            program.character_files =
                crate::program::character_files(&index, Path::new(Index::CHARACTERS));
        }
        program
            .characters
            .extend(stored.made_characters.made.keys().copied());
        program
            .characters
            .retain(|guid| !stored.deleted_characters.contains(guid));
        program
            .character_files
            .extend(stored.made_characters.made.values().map(|c| c.file));
        let surfaces = Surfaces::build(&install).map_err(|e| LifecycleError::Lot(e.to_string()))?;
        let tuning = NeighborhoodTuning::from_program(&program);
        Ok(Self {
            install,
            index,
            catalog,
            program,
            surfaces,
            tuning,
            stored,
        })
    }
}

/// Builds the definition registry for `World::read_neighbors`, excluding any
/// character definitions permanently deleted via Bulldoze (`0x426fcd`) and
/// including runtime-created character definitions persisted in
/// `made_characters` (`Make New Character`, primitive 19 / `0x43c3b2`).
fn neighborhood_registry(
    index: &Index,
    catalog: &Catalog,
    made: &sims_sim::characters::Characters,
    deleted: &BTreeSet<u32>,
) -> Vec<sims_sim::neighbors::Registered> {
    let mut reg = registry(index, catalog);
    reg.retain(|entry| !deleted.contains(&entry.guid));
    let existing: BTreeSet<u32> = reg.iter().map(|entry| entry.guid).collect();
    for (&guid, character) in &made.made {
        if existing.contains(&guid) || deleted.contains(&guid) {
            continue;
        }
        let kind = catalog
            .object(character.template)
            .map_or(2, |object| object.kind);
        reg.push(sims_sim::neighbors::Registered {
            guid,
            kind,
            resolves: true,
        });
    }
    reg
}

/// Enters `house` in `neighborhood_id` from REDB without reading or modifying
/// any `UserData` directory on disk.
///
/// If a saved lot exists in REDB for `(neighborhood_id, house)`, restores its
/// lot-local state and reconciles it with the latest neighborhood-wide `FAMI`,
/// `NBRS`, `NGBH`, `TATT`, and runtime-made characters. Otherwise boots the
/// house cleanly from its imported base `HouseXX.iff` bytes.
pub fn enter_lot(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    house: u32,
    answers: &[u8],
    frames: Frames,
) -> Result<Lot, LifecycleError> {
    enter_lot_with(
        install_root,
        store,
        neighborhood_id,
        house,
        answers,
        frames,
        CrtRng::from_state(1),
        None,
    )
}

/// Like [`enter_lot`], allowing the caller to specify the starting [`CrtRng`]
/// and initial `view_centre`.
#[allow(clippy::too_many_arguments)]
pub fn enter_lot_with(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    house: u32,
    answers: &[u8],
    frames: Frames,
    crt: CrtRng,
    view_centre: Option<[i8; 2]>,
) -> Result<Lot, LifecycleError> {
    let snapshot = store
        .load_lot_snapshot(neighborhood_id, house)?
        .ok_or_else(|| LifecycleError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    if snapshot.saved_lot.is_none() && snapshot.base_house.is_none() {
        return Err(LifecycleError::HouseNotFound {
            neighborhood: neighborhood_id.to_owned(),
            house,
        });
    }
    let runtime = StoreRuntime::from_snapshot(
        install_root,
        snapshot.neighborhood,
        &snapshot.character_files,
    )?;
    if let Some(saved_bytes) = snapshot.saved_lot {
        boot_from_saved_lot(
            runtime,
            store.store_id(),
            neighborhood_id,
            house,
            &saved_bytes,
            answers,
            frames,
            view_centre,
        )
    } else {
        boot_from_base_house(
            runtime,
            store.store_id(),
            neighborhood_id,
            house,
            snapshot.base_house.expect("checked above"),
            answers,
            frames,
            crt,
            view_centre,
            true,
        )
    }
}

/// Re-enters a previously saved lot from REDB, reconciling all neighborhood-wide
/// state (`NBRS`, `FAMI`, `NGBH`, `TATT`, `made_characters`) with the latest
/// [`StoredNeighborhood`].
#[allow(clippy::too_many_arguments)]
fn boot_from_saved_lot(
    runtime: StoreRuntime,
    store_id: u128,
    neighborhood_id: &str,
    house: u32,
    saved_bytes: &[u8],
    answers: &[u8],
    frames: Frames,
    view_centre: Option<[i8; 2]>,
) -> Result<Lot, LifecycleError> {
    let saved = Saved::decode(saved_bytes)
        .map_err(|e| LifecycleError::Lot(format!("decoding saved lot {house}: {e}")))?;
    if saved.house != house {
        return Err(LifecycleError::Lot(format!(
            "saved lot is for house {}, requested house {house}",
            saved.house
        )));
    }
    let StoreRuntime {
        install,
        index,
        catalog,
        program,
        surfaces,
        tuning,
        stored,
        ..
    } = runtime;
    let origin_instance_id = stored.instance_id;
    let origin_revision = stored.revision;

    let mut world = saved.world;
    let reg = neighborhood_registry(
        &index,
        &catalog,
        &stored.made_characters,
        &stored.deleted_characters,
    );
    let content_nh = stored.to_content_neighborhood();
    world
        .read_neighbors(&content_nh, &reg)
        .map_err(|e| LifecycleError::Lot(e.to_string()))?;
    world.type_attributes = program
        .type_attributes(&catalog)
        .map_err(|e| LifecycleError::Lot(e.to_string()))?;
    if let Some(saved_tatt) = &stored.type_attributes {
        world.read_type_attributes(saved_tatt);
    }
    // `made_characters` is neighborhood-wide state: replace from the latest
    // `StoredNeighborhood` so characters deleted via Bulldoze never reappear
    // from an older saved lot.
    world.characters = stored.made_characters.clone();

    let (family_id, family_budget, family_flags, family_members, family_magic) =
        match stored.family_in(house) {
            Some((id, record)) => (
                id,
                record.budget,
                record.flags,
                record.members.clone(),
                record
                    .magic_history()
                    .map_err(|e| LifecycleError::Lot(e.to_string()))?,
            ),
            None => (0u16, 0, 0, Vec::new(), None),
        };
    let effective_view_centre = view_centre.or(saved.view_centre);
    let new_move_in = family_id != 0 && family_flags & FAMILY_FLAG_NEW_MOVE_IN != 0;
    let sim = {
        let interpreter = Interpreter {
            view_rotation: saved.view_rotation,
            view_centre: effective_view_centre,
            ..Interpreter::new(&program, &catalog, Stream::Global)
        };
        // When a household moves into a previously saved (evicted or bulldozed)
        // lot, run the native `cHouse::Load` controller-reconciliation and
        // load-visit steps (`0x440328` -> `0x42830d` -> `0x43fc94` -> `0x42848d`)
        // so global controllers removed by `evict`/`bulldoze` (including
        // `VisitGenerator`, whose `"reset lot"` tree spawns the arriving
        // household) are recreated before household selection and `"reset lot"`.
        if new_move_in {
            let _ = interpreter
                .reconcile_controllers(&mut world)
                .map_err(|e| LifecycleError::Lot(format!("reconciling controllers: {e}")))?;
            let _ = world.discard_removals();
        }

        let residents_before = world.residents();
        // An occupied residential lot is in Live mode, never vacant edit mode
        // (`global[32] == 1`). Clearing `global[32]` before `select_family`
        // ensures `World::set_funds` writes the arriving household's funds
        // instead of keeping a previously edited vacant lot's edit budget.
        if family_id != 0
            && stored.is_residential(house)
            && let Some(edit_budget) = world.globals.get_mut(BUDGET_MODE)
        {
            *edit_budget = 0;
        }
        world
            .select_family(
                i16::try_from(family_id).map_err(|e| LifecycleError::Lot(e.to_string()))?,
                i16::try_from(house).map_err(|e| LifecycleError::Lot(e.to_string()))?,
                family_budget,
                family_magic,
            )
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;

        // Purge any live person whose neighborhood record was deleted while
        // this lot was saved (for example a visitor whose home lot was
        // bulldozed via `0x426fcd`, even if a replacement character later
        // reused the same GUID at a newer `deletion_epoch`) before re-attaching
        // remaining persons.
        let lot_epoch = saved.deletion_epoch;
        let orphaned_people: Vec<u16> = world
            .people
            .iter()
            .copied()
            .filter(|&person_id| {
                world.objects.get(&person_id).is_some_and(|o| {
                    o.person.is_some()
                        && (stored.deleted_characters.contains(&o.guid)
                            || world.neighbors.first_of(o.guid).is_none()
                            || stored
                                .deleted_at_epoch
                                .get(&o.guid)
                                .is_some_and(|&del_epoch| lot_epoch < del_epoch))
                })
            })
            .collect();
        let mut people_to_reset = BTreeSet::new();
        let mut objects_to_reset = BTreeSet::new();
        for person_id in orphaned_people {
            let (p_reset, o_reset) = purge_orphaned_person(&interpreter, &mut world, person_id)?;
            people_to_reset.extend(p_reset);
            objects_to_reset.extend(o_reset);
        }

        // Re-attach every live person's persistent fields from the latest NBRS
        // records (0x4258a3) so cross-lot edits propagate cleanly.
        for person_id in world.people.clone() {
            if world
                .objects
                .get(&person_id)
                .is_some_and(|o| o.person.is_some())
            {
                interpreter
                    .attach_neighbor(&mut world, person_id)
                    .map_err(|e| {
                        LifecycleError::Lot(format!("attaching person {person_id}: {e}"))
                    })?;
            }
        }

        // Reset any surviving person or object whose active action or BHAV
        // stack referenced a purged visitor or deleted character definition so
        // its next tick resumes cleanly on its own Main tree.
        for person_id in people_to_reset {
            if world
                .objects
                .get(&person_id)
                .is_some_and(|o| o.person.is_some())
            {
                interpreter
                    .reset_person_after_purge(&mut world, person_id)
                    .map_err(|e| {
                        LifecycleError::Lot(format!(
                            "resetting person {person_id} after orphaned visitor purge: {e}"
                        ))
                    })?;
            }
        }
        for object_id in objects_to_reset {
            if world.objects.contains_key(&object_id) {
                interpreter
                    .reset(&mut world, object_id, false)
                    .map_err(|e| {
                        LifecycleError::Lot(format!(
                            "resetting object {object_id} after orphaned visitor purge: {e}"
                        ))
                    })?;
            }
        }
        if new_move_in {
            let visited = interpreter.visit_objects(&mut world, &program.people);
            if let Some(fault) = visited.stopped {
                return Err(LifecycleError::Lot(format!(
                    "house {house} load did not finish: {fault}"
                )));
            }
            let _ = interpreter
                .ensure_indicators(&mut world)
                .map_err(|e| LifecycleError::Lot(e.to_string()))?;
            finish_selection_for_lifecycle(
                &interpreter,
                &mut world,
                house,
                family_id,
                family_flags,
                &family_members,
                &residents_before,
                tuning,
            )?;
            let _ = interpreter
                .announce_house(&mut world)
                .map_err(|e| LifecycleError::Lot(e.to_string()))?;
            let sim = interpreter
                .select_resident(&mut world, family_budget, &family_members)
                .map_err(|e| LifecycleError::Lot(e.to_string()))?;
            interpreter
                .go_live(&mut world)
                .map_err(|e| LifecycleError::Lot(e.to_string()))?;
            let _ = interpreter
                .before_frame(&mut world)
                .map_err(|e| LifecycleError::Lot(e.to_string()))?;
            sim
        } else {
            finish_selection_for_lifecycle(
                &interpreter,
                &mut world,
                house,
                family_id,
                family_flags,
                &family_members,
                &residents_before,
                tuning,
            )?;
            let _ = interpreter
                .ensure_indicators(&mut world)
                .map_err(|e| LifecycleError::Lot(e.to_string()))?;
            let sim = if family_id != 0 && selected(&world).is_none() {
                interpreter
                    .select_resident(&mut world, family_budget, &family_members)
                    .map_err(|e| LifecycleError::Lot(e.to_string()))?
            } else {
                selected(&world)
            };
            if family_id != 0 && world.globals[19] != 0 {
                interpreter
                    .go_live(&mut world)
                    .map_err(|e| LifecycleError::Lot(e.to_string()))?;
                let _ = interpreter
                    .before_frame(&mut world)
                    .map_err(|e| LifecycleError::Lot(e.to_string()))?;
            }
            sim
        }
    };

    let mut load = LoadReport {
        family: family_id,
        house,
        finished: Some(Finished {
            indicators_made: false,
            sim,
            counted: 0,
        }),
        ..LoadReport::default()
    };
    load.libraries = program.libraries.len();

    let player = if answers.is_empty() {
        saved.answers
    } else if saved.answered || saved.replay_view.is_some() {
        // Preserve any already-committed dialog answer(s) for an in-flight
        // dialog tick (including multi-dialog ticks paused on a second dialog
        // where `answered` was cleared while `replay_view` stays set), and
        // stage caller-supplied future answers in choice-only mode behind them.
        let mut player = saved.answers;
        player.extend_choices_after(answers);
        player
    } else {
        Answers::choices(answers)
    };
    let play = Play { fresh: saved.fresh };
    Ok(Lot {
        program,
        catalog: std::sync::Arc::new(catalog),
        surfaces,
        world,
        tuning: None,
        player,
        frames,
        ticks: saved.ticks,
        start: saved.start,
        load,
        halted: false,
        play,
        asks: false,
        asking: None,
        answered: saved.answered,
        view_rotation: saved.view_rotation,
        floor_rotation: saved.floor_rotation,
        view_centre: effective_view_centre,
        replay_view: saved.replay_view,
        last_dialog_text: saved.last_dialog_text,
        character_names: saved.character_names,
        default_character_name: crate::boot::read_default_character_name(&install),
        lot_name: saved.lot_name,
        lot_description: saved.lot_description,
        loaded: saved.loaded,
        buy_undo: Vec::new(),
        buy_redo: Vec::new(),
        buy_orphans: Vec::new(),
        active_buy_cursor: None,
        build_undo: Vec::new(),
        build_redo: Vec::new(),
        undo_order: Vec::new(),
        redo_order: Vec::new(),
        active_wall_cursor: None,
        neighborhood_origin: Some(NeighborhoodOrigin {
            store_id,
            neighborhood_id: neighborhood_id.to_owned(),
            instance_id: origin_instance_id,
            revision: origin_revision,
        }),
        coop: None,
    })
}

/// Boots a lot from its imported base `HouseXX.iff` bytes in REDB, supporting
/// occupied, newly moved-in (`FAMILY_FLAG_NEW_MOVE_IN`), and unoccupied lots.
#[allow(clippy::too_many_arguments)]
fn boot_from_base_house(
    runtime: StoreRuntime,
    store_id: u128,
    neighborhood_id: &str,
    house: u32,
    bytes: Vec<u8>,
    answers: &[u8],
    frames: Frames,
    crt: CrtRng,
    view_centre: Option<[i8; 2]>,
    enter_live: bool,
) -> Result<Lot, LifecycleError> {
    let StoreRuntime {
        install,
        index,
        catalog,
        program,
        surfaces,
        tuning,
        stored,
    } = runtime;
    let origin_instance_id = stored.instance_id;
    let origin_revision = stored.revision;

    let virtual_path = PathBuf::from(format!("{neighborhood_id}/Houses/House{house:02}.iff"));
    // Include any GUID ever deleted via Bulldoze (`deleted_characters` union
    // `deleted_at_epoch` keys) so even if a replacement runtime-made character
    // later reused a deleted loose character's GUID, `ObjM` parsing succeeds
    // and `strip_deleted_base_instances` strips the stale base-house instance.
    let stripped_guids: BTreeSet<u32> = stored
        .deleted_characters
        .iter()
        .copied()
        .chain(stored.deleted_at_epoch.keys().copied())
        .collect();
    let mut file = House::parse_bytes_with_deleted(bytes, &virtual_path, &catalog, &stripped_guids)
        .map_err(|e| LifecycleError::Lot(e.to_string()))?;
    let view_rotation = opening_rotation(&file);
    let context = Context::COMPLETE_COLLECTION;
    let mut world = World::empty();
    world.crt_rng = crt;
    let mut load = LoadReport::default();

    {
        let interpreter = Interpreter {
            view_rotation,
            view_centre,
            ..Interpreter::new(&program, &catalog, Stream::Global)
        };
        world
            .skeleton_files
            .extend(crate::objects::read_before_the_first_load(&program.people));
        let content_nh = stored.to_content_neighborhood();
        let reg = neighborhood_registry(
            &index,
            &catalog,
            &stored.made_characters,
            &stored.deleted_characters,
        );
        world
            .read_neighbors(&content_nh, &reg)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        world
            .drop_empty_families()
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        world.type_attributes = program
            .type_attributes(&catalog)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        if let Some(saved_tatt) = &stored.type_attributes {
            world.read_type_attributes(saved_tatt);
        }
        world.characters = stored.made_characters.clone();
        interpreter
            .create_indicators(&mut world)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        world.read_simulator(&file.simulator);
        world.read_house_model(
            file.model
                .as_ref()
                .ok_or_else(|| LifecycleError::Lot("house has no HOUS 1 chunk".into()))?,
        );
        world
            .apply_context(context)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        load.created = interpreter
            .resize(&mut world)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?
            .created
            .len();
        world
            .clear_family()
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        load.queued = world.removals.len();
        load.removed = interpreter
            .clear(&mut world)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?
            .len();
        let version = file
            .version()
            .ok_or_else(|| LifecycleError::Lot("house has no HOUS 1 version".into()))?;
        world
            .read_arrays(&file.arrays, version)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        load.renumbered = world
            .renumber_surfaces(
                file.floor_map.as_deref(),
                file.wall_map.as_deref(),
                &surfaces,
            )
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;

        let types = &file.types.entries;
        for entry in types {
            let Some(object) = catalog.object(entry.guid) else {
                // If a character file was permanently deleted when another lot
                // was bulldozed (`0x426fcd`), even if a runtime-made character
                // later reused that GUID, `read_types` skips the missing
                // catalog entry and `strip_deleted_base_instances` removes any
                // stale saved instance before object read.
                if stripped_guids.contains(&entry.guid) {
                    continue;
                }
                return Err(LifecycleError::Lot(format!(
                    "type {} has no definition",
                    entry.id
                )));
            };
            if interpreter.type_changed(entry) == Some(true) {
                let current = [Callback::Init, Callback::Main]
                    .map(|c| program.signature(object.resource, program.callback(object, c)));
                load.mismatches.push(format!(
                    "  type {} {:?} (0x{:08X}): saved {:08x}/{:08x}, linked {:08x}/{:08x}",
                    entry.id,
                    entry.name,
                    entry.guid,
                    entry.init_signature,
                    entry.main_signature,
                    current[0],
                    current[1]
                ));
            }
        }
        load.types = types.len();
        load.changed = interpreter
            .read_types(&mut world, &file.types)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?
            .len();
        load.cleared_again = interpreter
            .clear(&mut world)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?
            .len();
        let (stripped_people_to_reset, stripped_objects_to_reset) = strip_deleted_base_instances(
            &mut world,
            &mut file.objects,
            &stored.deleted_characters,
            &stored.deleted_at_epoch,
        );
        let people = People::load(
            &install,
            &index,
            &catalog,
            file.objects.instances.values().map(|o| o.guid),
        )
        .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        let read = interpreter
            .read_objects_with_frame_tree_bits(&mut world, &file.objects, &people, &BTreeSet::new())
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        load.read = read.objects.len();
        load.draws = read.draws;
        let reconciled = interpreter
            .reconcile_controllers(&mut world)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        load.reconciled_created = reconciled.created.len();
        load.reconciled_removed = reconciled.removed.len();
        load.repaired = world
            .repair_grid()
            .map_err(|e| LifecycleError::Lot(e.to_string()))?
            .len();
        world.finish_object_read();
        load.queued_again = world.removals.len();
        world
            .rebuild_rooms()
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        let roomed = interpreter
            .assign_rooms(&mut world)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        load.moved = roomed.moved.len();
        load.asked = roomed.asked.len();
        interpreter
            .update_rooms(&mut world, 0)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        world.mark_all_rooms();
        interpreter
            .update_pending_rooms(&mut world)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        load.dropped = world.discard_removals().len();

        let (family_id, family_budget, family_flags, family_members, family_magic) = if enter_live {
            match stored.family_in(house) {
                Some((id, record)) => (
                    id,
                    record.budget,
                    record.flags,
                    record.members.clone(),
                    record
                        .magic_history()
                        .map_err(|e| LifecycleError::Lot(e.to_string()))?,
                ),
                None => (0u16, 0, 0, Vec::new(), None),
            }
        } else {
            (0u16, 0, 0, Vec::new(), None)
        };
        load.family = family_id;
        load.house = house;
        let residents = world.residents();
        // Base residential houses (notably Magic Town lots 90..=92) can store
        // `global[32] == 1` from when they were authored vacant; clear it
        // before `select_family` when an occupying household enters Live mode.
        if enter_live
            && family_id != 0
            && stored.is_residential(house)
            && let Some(edit_budget) = world.globals.get_mut(BUDGET_MODE)
        {
            *edit_budget = 0;
        }
        world
            .select_family(
                i16::try_from(family_id).map_err(|e| LifecycleError::Lot(e.to_string()))?,
                i16::try_from(house).map_err(|e| LifecycleError::Lot(e.to_string()))?,
                family_budget,
                family_magic,
            )
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;

        load.to_visit = world.order.len();
        if enter_live {
            let visited = interpreter.visit_objects(&mut world, &people);
            load.visited = visited.objects.len();
            load.reset = visited.reset.len();
            load.stopped = visited.stopped.map(|fault| fault.to_string());
            if let Some(reason) = &load.stopped {
                return Err(LifecycleError::Lot(format!(
                    "house {house} load did not finish: {reason}"
                )));
            }
        } else {
            // During `evict`/`bulldoze` (`enter_live == false`), visit every
            // object so people attach their neighbor records and positions are
            // restored; if a stock furniture object's Load callback hits an
            // empty/unported BHAV (e.g. Complete Collection House 56 object
            // 302 tree `0x100e`), still restore its position and continue so
            // the household can be evicted cleanly.
            let mut current = world.order.first().copied();
            while let Some(id) = current {
                let is_person = world.objects.get(&id).is_some_and(|o| o.person.is_some());
                let changed = world
                    .objects
                    .get(&id)
                    .is_some_and(|o| world.changed_types.contains(&o.guid));
                let mut next = world.clone();
                match interpreter.visit(&mut next, id, &people) {
                    Ok(()) => {
                        world = next;
                        load.visited += 1;
                        if changed {
                            load.reset += 1;
                        }
                    }
                    Err(fault) if is_person => {
                        return Err(LifecycleError::Lot(format!(
                            "house {house} load did not finish: {fault}"
                        )));
                    }
                    Err(_) => {
                        if let Some(mut pos) = world.objects.get(&id).map(|o| o.position) {
                            if pos[2] == 0 {
                                pos[2] = 1;
                            }
                            let _ = interpreter.move_tree(&mut world, id, pos);
                        }
                    }
                }
                current = world
                    .order
                    .iter()
                    .position(|&other| other == id)
                    .and_then(|place| world.order.get(place + 1).copied());
            }
        }
        for person_id in stripped_people_to_reset {
            if world
                .objects
                .get(&person_id)
                .is_some_and(|o| o.person.is_some())
            {
                interpreter
                    .reset_person_after_purge(&mut world, person_id)
                    .map_err(|e| {
                        LifecycleError::Lot(format!(
                            "resetting person {person_id} after stripped base character: {e}"
                        ))
                    })?;
            }
        }
        for object_id in stripped_objects_to_reset {
            if world.objects.contains_key(&object_id) {
                interpreter
                    .reset(&mut world, object_id, false)
                    .map_err(|e| {
                        LifecycleError::Lot(format!(
                            "resetting object {object_id} after stripped base character: {e}"
                        ))
                    })?;
            }
        }

        let made = interpreter
            .ensure_indicators(&mut world)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?
            .is_some();
        finish_selection_for_lifecycle(
            &interpreter,
            &mut world,
            house,
            family_id,
            family_flags,
            &family_members,
            &residents,
            tuning,
        )?;
        let made_again = interpreter
            .announce_house(&mut world)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?
            .is_some();
        let sim = if enter_live && family_id != 0 {
            interpreter
                .select_resident(&mut world, family_budget, &family_members)
                .map_err(|e| LifecycleError::Lot(e.to_string()))?
        } else {
            None
        };
        if enter_live {
            interpreter
                .go_live(&mut world)
                .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        }
        let counted = interpreter
            .before_frame(&mut world)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?
            .len();
        load.finished = Some(Finished {
            indicators_made: made || made_again,
            sim,
            counted,
        });
    }

    let saved_instances = &file.objects.instances;
    load.saved_objects = saved_instances.len();
    load.saved_people = saved_instances
        .values()
        .filter(|o| o.person.is_some())
        .count();
    load.saved_ticks = file.simulator.ticks;
    load.libraries = program.libraries.len();
    let start = world.ticks;
    let loaded = world.hash();
    Ok(Lot {
        program,
        catalog: std::sync::Arc::new(catalog),
        surfaces,
        world,
        tuning: None,
        player: Answers::choices(answers),
        frames,
        ticks: 0,
        start,
        load,
        halted: false,
        play: Play::default(),
        asks: false,
        answered: false,
        asking: None,
        view_rotation,
        floor_rotation: view_rotation,
        view_centre,
        replay_view: None,
        last_dialog_text: String::new(),
        character_names: std::collections::BTreeMap::new(),
        default_character_name: crate::boot::read_default_character_name(&install),
        lot_name: String::new(),
        lot_description: String::new(),
        loaded,
        buy_undo: Vec::new(),
        buy_redo: Vec::new(),
        buy_orphans: Vec::new(),
        active_buy_cursor: None,
        build_undo: Vec::new(),
        build_redo: Vec::new(),
        undo_order: Vec::new(),
        redo_order: Vec::new(),
        active_wall_cursor: None,
        neighborhood_origin: Some(NeighborhoodOrigin {
            store_id,
            neighborhood_id: neighborhood_id.to_owned(),
            instance_id: origin_instance_id,
            revision: origin_revision,
        }),
        coop: None,
    })
}

/// Saves `lot` and atomically writes back neighborhood-wide `FAMI`, `NBRS`,
/// `NGBH`, `TATT`, and runtime-made characters into `store`, following
/// `Sims.exe` 1.91 `SaveCurrentHouse` (`0x428687`) → `cHouse::Save`
/// (`0x41615d`) → `cXObjectManager::Save` (`0x43fb1b`) → `cTSNeighborhood::Save`
/// (`0x4252cc`).
///
/// Rejects lots opened from a different store, neighborhood ID, or re-imported
/// neighborhood instance ([`LifecycleError::NeighborhoodMismatch`]), lots
/// replaced via [`Lot::restore`], or lots whose neighborhood revision changed
/// since they were opened ([`LifecycleError::StaleNeighborhood`]), and
/// advances the lot's tracked revision after a successful save so repeated
/// saves from the same live lot continue to succeed.
pub fn save_and_leave_lot(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    lot: &mut Lot,
) -> Result<SaveReport, LifecycleError> {
    save_and_leave_lot_inner(install_root, store, neighborhood_id, lot, None)
}

/// Like [`save_and_leave_lot`], stripping an active unpaid Buy Mode preview
/// cursor first (`Lot::save_without_buy_cursor`).
pub fn save_and_leave_lot_without_buy_cursor(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    lot: &mut Lot,
    cursor: &BuyCursor,
) -> Result<SaveReport, LifecycleError> {
    save_and_leave_lot_inner(install_root, store, neighborhood_id, lot, Some(cursor))
}

fn save_and_leave_lot_inner(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    lot: &mut Lot,
    cursor: Option<&BuyCursor>,
) -> Result<SaveReport, LifecycleError> {
    let Some(origin) = lot.neighborhood_origin.clone() else {
        return Err(LifecycleError::Lot(
            "lot was not opened from a NeighborhoodStore (or was replaced via restore)".into(),
        ));
    };
    if origin.store_id != store.store_id() || origin.neighborhood_id != neighborhood_id {
        return Err(LifecycleError::NeighborhoodMismatch {
            expected: format!("{}@store:{:032x}", origin.neighborhood_id, origin.store_id),
            actual: format!("{neighborhood_id}@store:{:032x}", store.store_id()),
        });
    }

    let mut stored = store
        .load_neighborhood(neighborhood_id)?
        .ok_or_else(|| LifecycleError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    if stored.instance_id != origin.instance_id {
        return Err(LifecycleError::NeighborhoodMismatch {
            expected: format!("{neighborhood_id}#{:032x}", origin.instance_id),
            actual: format!("{neighborhood_id}#{:032x}", stored.instance_id),
        });
    }
    if stored.revision != origin.revision {
        return Err(LifecycleError::StaleNeighborhood {
            neighborhood: neighborhood_id.to_owned(),
            expected_revision: origin.revision,
            current_revision: stored.revision,
        });
    }

    let mut saved = match cursor {
        Some(cursor) => lot.save_without_buy_cursor(cursor),
        None => lot.save(),
    }
    .map_err(|e| LifecycleError::Lot(e.to_string()))?;

    let install = Install::detect(install_root).map_err(|e| LifecycleError::Lot(e.to_string()))?;
    let prices = LotPrices::load(&install, &lot.surfaces);
    let tuning = NeighborhoodTuning::from_program(&lot.program);

    // 1. Flush every live person into its neighborhood record (0x459d8b → 0x425a69).
    let mut saved_people = 0usize;
    for person_id in saved.world.people.clone() {
        if saved
            .world
            .objects
            .get(&person_id)
            .is_some_and(|o| o.person.is_some())
        {
            people::save_to_record(&mut saved.world, person_id)
                .map_err(|e| LifecycleError::Lot(format!("saving person {person_id}: {e}")))?;
            saved_people += 1;
        }
    }

    // 2. Recompute lot values and family flags (0x416085 inside 0x41615d).
    let interpreter = Interpreter {
        view_rotation: lot.view_rotation,
        view_centre: lot.view_centre,
        ..Interpreter::new(&lot.program, &lot.catalog, Stream::Global)
    };
    let family_flags = update_lot_values(&mut saved.world, &interpreter, &prices)?;

    let total_value = lot_total_value(
        saved.world.budget.lot_value,
        saved.world.budget.objects_value,
        saved.world.budget.architecture_value,
        tuning.arch_value_multiplier,
    );
    let house = saved.house;
    let family_id = saved.family;
    let budget = saved.world.budget.funds;
    let objects_value = saved.world.budget.objects_value;
    let architecture_value = saved.world.budget.architecture_value;

    // 3. Write back neighborhood state (0x428687 → 0x4252cc) and commit atomically
    // with an in-transaction revision check.
    stored.write_back_world(
        &saved.world,
        house,
        family_id,
        total_value,
        Some(family_flags),
        tuning.friendship_threshold,
    );
    saved.deletion_epoch = stored.deletion_epoch;
    let lot_bytes = saved.encode();
    let (committed_instance, committed_revision) = match store.save_lot_and_neighborhood_checked(
        neighborhood_id,
        house,
        Some((origin.instance_id, origin.revision)),
        &lot_bytes,
        &stored,
        &[],
    )? {
        Ok(version) => version,
        Err(CheckedCommitError::InstanceMismatch { expected, current }) => {
            return Err(LifecycleError::NeighborhoodMismatch {
                expected: format!("{neighborhood_id}#{expected:032x}"),
                actual: format!("{neighborhood_id}#{current:032x}"),
            });
        }
        Err(CheckedCommitError::RevisionMismatch { expected, current }) => {
            return Err(LifecycleError::StaleNeighborhood {
                neighborhood: neighborhood_id.to_owned(),
                expected_revision: expected,
                current_revision: current,
            });
        }
    };
    lot.neighborhood_origin = Some(NeighborhoodOrigin {
        store_id: store.store_id(),
        neighborhood_id: neighborhood_id.to_owned(),
        instance_id: committed_instance,
        revision: committed_revision,
    });

    Ok(SaveReport {
        neighborhood: neighborhood_id.to_owned(),
        house,
        family: family_id,
        saved_people,
        budget,
        lot_total_value: total_value,
        objects_value,
        architecture_value,
        revision: committed_revision,
    })
}

/// Evicts the household occupying `house` back to the Family Bin (`0x42737e`
/// with `bulldoze = false`), refunding the exact depreciated lot, furniture,
/// and architecture value (`family.architecture_value`) into `family.budget`
/// and removing all people and removable furniture (`0x4354d5`).
pub fn evict(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    house: u32,
) -> Result<EvictReport, LifecycleError> {
    let (report, budget_after) =
        evict_or_bulldoze(install_root, store, neighborhood_id, house, false)?;
    let family = report
        .evicted_family
        .ok_or(LifecycleError::LotUnoccupied { house })?;
    Ok(EvictReport {
        house,
        family,
        refund: report.refund,
        budget_after,
        removed_objects: report.removed_objects,
        remaining_objects_value: report.remaining_objects_value,
        remaining_architecture_value: report.remaining_architecture_value,
    })
}

/// Bulldozes `house` (`0x42737e` with `bulldoze = true`), evicting any
/// occupying household first (with the exact same refund as [`evict`]), then
/// removing all remaining non-locked build objects, clearing all non-`0x20`
/// floors & walls on levels 1–2, clearing the pool array (`Arry #9`), and
/// deleting house-bound tombstone neighbor records.
pub fn bulldoze(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    house: u32,
) -> Result<BulldozeReport, LifecycleError> {
    Ok(evict_or_bulldoze(install_root, store, neighborhood_id, house, true)?.0)
}

fn evict_or_bulldoze(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    house: u32,
    bulldoze: bool,
) -> Result<(BulldozeReport, i32), LifecycleError> {
    let snapshot = store
        .load_lot_snapshot(neighborhood_id, house)?
        .ok_or_else(|| LifecycleError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    if snapshot.saved_lot.is_none() && snapshot.base_house.is_none() {
        return Err(LifecycleError::HouseNotFound {
            neighborhood: neighborhood_id.to_owned(),
            house,
        });
    }
    let mut runtime = StoreRuntime::from_snapshot(
        install_root,
        snapshot.neighborhood,
        &snapshot.character_files,
    )?;
    let origin_instance_id = runtime.stored.instance_id;
    let origin_revision = runtime.stored.revision;
    let evicted = runtime
        .stored
        .family_in(house)
        .map(|(id, f)| (id, f.clone()));
    if evicted.is_none() && !bulldoze {
        return Err(LifecycleError::LotUnoccupied { house });
    }
    let saved_lot = snapshot.saved_lot;
    let base_house = snapshot.base_house;

    // Map loose `Characters` files to their catalog GUIDs before `runtime` is
    // consumed so bulldoze tombstone deletion (`0x426fcd` → `0x46b42a`) can
    // remove the deleted character's file from `NEIGHBORHOOD_CHARACTERS`.
    let mut char_file_by_guid: BTreeMap<u32, String> = BTreeMap::new();
    let mut guids_per_char_file: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    if bulldoze
        && let Some(char_mount) = runtime
            .index
            .mounts
            .iter()
            .position(|m| m.path == Index::CHARACTERS)
    {
        for object in &runtime.catalog.objects {
            let Some(res) = runtime.index.resources.get(object.resource) else {
                continue;
            };
            if res.mount == char_mount && res.member.is_none() {
                let rel = res
                    .path
                    .strip_prefix(Path::new(Index::CHARACTERS))
                    .unwrap_or(&res.path)
                    .to_string_lossy()
                    .replace('\\', "/");
                char_file_by_guid.insert(object.guid, rel.clone());
                guids_per_char_file
                    .entry(rel)
                    .or_default()
                    .insert(object.guid);
            }
        }
    }

    // 0x4273d1: clear `family.house = 0` (or `secondary_house = 0` when the
    // family owns this lot as its secondary residence, `0x425d95`) BEFORE
    // loading the lot so the load selects no household and never runs a
    // pending move-in reset.
    let house_i32 = i32::try_from(house).map_err(|e| LifecycleError::Lot(e.to_string()))?;
    let mut cleared_secondary = false;
    if let Some((evicted_id, _)) = &evicted
        && let Some(fam) = runtime.stored.families.get_mut(evicted_id)
    {
        if fam.house == house_i32 {
            fam.house = 0;
        } else if fam.secondary_house() == house_i32 {
            fam.set_secondary_house(0);
            cleared_secondary = true;
        }
    }

    let mut stored = runtime.stored.clone();
    let tuning = runtime.tuning;
    let prices = LotPrices::load(&runtime.install, &runtime.surfaces);

    // Load the lot (from saved lot if present, else base HouseXX.iff).
    let mut lot = if let Some(saved_bytes) = saved_lot {
        boot_from_saved_lot(
            runtime,
            store.store_id(),
            neighborhood_id,
            house,
            &saved_bytes,
            &[],
            Frames::default(),
            None,
        )?
    } else {
        boot_from_base_house(
            runtime,
            store.store_id(),
            neighborhood_id,
            house,
            base_house.expect("checked above"),
            &[],
            Frames::default(),
            CrtRng::from_state(1),
            None,
            false,
        )?
    };

    let interpreter = Interpreter {
        view_rotation: lot.view_rotation,
        view_centre: lot.view_centre,
        ..Interpreter::new(&lot.program, &lot.catalog, Stream::Global)
    };

    // Deselect any selected Sim and flush every person's persistent data first.
    if selected(&lot.world).is_some() {
        interpreter
            .select(&mut lot.world, None)
            .map_err(|e| LifecycleError::Lot(format!("deselecting Sim for evict: {e}")))?;
    }
    for person_id in lot.world.people.clone() {
        if lot
            .world
            .objects
            .get(&person_id)
            .is_some_and(|o| o.person.is_some())
        {
            let _ = people::save_to_record(&mut lot.world, person_id);
        }
    }

    // 0x427456..0x427462: set simulator funds to evicted family's budget before
    // broadcasting "evict lot".
    if let Some((_, ref family)) = evicted {
        lot.world
            .set_funds(family.budget)
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
    }

    // 0x427495: broadcast "evict lot".
    let _ = broadcast_tree(&interpreter, &mut lot.world, "evict lot", 0)
        .map_err(|e| LifecycleError::Lot(format!("broadcasting \"evict lot\": {e}")))?;

    // 0x42749a..0x427536: remove all removable people first — including seated
    // residents and seated visitors whose chairs will be removed — so no
    // lingering seated person or route/use blocks furniture removal.
    let mut removed_objects = 0usize;
    let people_ids: Vec<u16> = lot.world.people.clone();
    for id in people_ids {
        if !lot.world.objects.contains_key(&id) || lot.world.indicators.contains(&id) {
            continue;
        }
        let mut removable =
            should_remove_on_evict_or_bulldoze(&interpreter, &mut lot.world, id, bulldoze);
        if !removable
            && let Ok(place) = crate::placement::capture(&mut lot.world, id)
            && let Some(mut container) = place.container
        {
            while let Ok(parent) = crate::placement::capture(&mut lot.world, container) {
                if let Some(next) = parent.container {
                    container = next;
                } else {
                    break;
                }
            }
            removable = should_remove_on_evict_or_bulldoze(
                &interpreter,
                &mut lot.world,
                container,
                bulldoze,
            );
        }
        if removable {
            remove_person_cleanly(&interpreter, &mut lot.world, id)?;
            removed_objects += 1;
        }
    }
    // Clear any remaining stale routes before object removal.
    for route in lot.world.routing.routes.values_mut() {
        route.contexts.clear();
    }

    // Walk every registered non-person object (including non-lead tiles of a
    // multi-tile group, matching `0x42749a..0x427528`), restarting the scan
    // after each removal since deleting a multi-tile group or container removes
    // multiple IDs at once.
    loop {
        let mut next_victim = None;
        for id in lot.world.order.clone() {
            if lot.world.indicators.contains(&id) || !lot.world.objects.contains_key(&id) {
                continue;
            }
            let in_container = crate::placement::capture(&mut lot.world, id)
                .map(|c| c.container.is_some())
                .unwrap_or(false);
            if !in_container
                && should_remove_on_evict_or_bulldoze(&interpreter, &mut lot.world, id, bulldoze)
            {
                next_victim = Some(id);
                break;
            }
        }
        let Some(victim) = next_victim else {
            break;
        };
        let count_before = lot.world.objects.len();
        interpreter
            .remove_now(&mut lot.world, victim)
            .map_err(|e| LifecycleError::Lot(format!("removing object {victim}: {e}")))?;
        removed_objects += count_before.saturating_sub(lot.world.objects.len());
    }
    // Discard any pending removal IDs queued during `"evict lot"` or object
    // removal (matching `cHouse::Load`'s `discard_removals`) without deleting
    // surviving lot fixtures, so recycled object IDs on future move-ins never
    // inherit stale `world.removals` entries.
    let _ = lot.world.discard_removals();

    // 0x427536..0x427616: on bulldoze, clear non-0x20 floors & walls on levels
    // 1 and 2 and reset the pool array (Arry #9) to 0xff.
    let mut cleared_tiles = 0usize;
    if bulldoze {
        cleared_tiles = clear_lot_architecture(&mut lot.world)?;
        lot.world
            .rebuild_rooms()
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;
        lot.world.mark_all_rooms();
    }

    // 0x42761d..0x427685: financial settlement & simulator reset.
    let funds_after_evict = lot.world.budget.funds;
    lot.world
        .set_funds(0)
        .map_err(|e| LifecycleError::Lot(e.to_string()))?;
    if let Some(g26) = lot.world.globals.get_mut(GLOBAL_26) {
        *g26 = 0;
    }
    reset_budget_history(&mut lot.world);
    reset_house_usage(&mut lot.world, tuning.layout_history_fill);

    // Clear family selection on the lot (house is now unoccupied).
    lot.world
        .select_family(
            0,
            i16::try_from(house).map_err(|e| LifecycleError::Lot(e.to_string()))?,
            0,
            None,
        )
        .map_err(|e| LifecycleError::Lot(e.to_string()))?;
    lot.load.family = 0;

    // 0x427697..0x4276ce → 0x426fcd: on bulldoze, remove home-bound tombstone
    // character records (word 67 == house, belonging to no family) AND delete
    // their character files (`0x46b42a`) so reloads never recreate them.
    let mut deleted_tombstones = 0usize;
    let mut removed_character_files: Vec<String> = Vec::new();
    if bulldoze {
        let member_guids: BTreeSet<u32> = stored
            .families
            .values()
            .flat_map(|f| f.members.iter().copied())
            .collect();
        let tombstones: Vec<(i16, u32)> = lot
            .world
            .neighbors
            .slots
            .iter()
            .flatten()
            .filter(|n| {
                let is_char = lot.program.characters.contains(&n.guid)
                    || lot.world.characters.made.contains_key(&n.guid);
                is_char
                    && !member_guids.contains(&n.guid)
                    && i32::from(n.data.get(PERSON_HOME).copied().unwrap_or(0)) == house_i32
            })
            .map(|n| (n.id, n.guid))
            .collect();
        let mut deleted_guids: BTreeSet<u32> = BTreeSet::new();
        if !tombstones.is_empty() {
            stored.deletion_epoch = stored.deletion_epoch.saturating_add(1);
        }
        let current_epoch = stored.deletion_epoch;
        for (id, guid) in tombstones {
            lot.world.neighbors.remove_record(id);
            lot.world.characters.made.remove(&guid);
            stored.made_characters.made.remove(&guid);
            stored.deleted_characters.insert(guid);
            stored.deleted_at_epoch.insert(guid, current_epoch);
            deleted_guids.insert(guid);
            deleted_tombstones += 1;
        }
        for guid in &deleted_guids {
            if let Some(rel) = char_file_by_guid.get(guid)
                && guids_per_char_file
                    .get(rel)
                    .is_some_and(|guids| guids.is_subset(&deleted_guids))
                && !removed_character_files.contains(rel)
            {
                removed_character_files.push(rel.clone());
            }
        }
    }

    // Refresh lot statistics (0x416085) so the cleared/bulldozed lot saves
    // accurate remaining `objects_value` and `architecture_value`.
    let _ = update_lot_values(&mut lot.world, &interpreter, &prices)?;
    let remaining_objects_value = lot.world.budget.objects_value;
    let remaining_architecture_value = lot.world.budget.architecture_value;

    // Update evicted family in `world.families` & `stored.families`:
    // 0x427674 (cFamily::GetNetWorth with house == 0) + 0x42767f (arch = 0).
    let mut refund = 0i32;
    let mut evicted_budget_after = 0i32;
    let evicted_family = evicted.as_ref().map(|(id, _)| *id);
    if let Some((family_id, _)) = evicted {
        if let Some(live_fam) = lot
            .world
            .families
            .iter_mut()
            .find(|f| f.id == i32::from(family_id))
            && !cleared_secondary
        {
            live_fam.house = 0;
        }
        if let Some(stored_fam) = stored.families.get_mut(&family_id) {
            if cleared_secondary {
                stored_fam.set_secondary_house(0);
            } else {
                refund = stored_fam.architecture_value;
                stored_fam.budget = funds_after_evict.wrapping_add(refund);
                stored_fam.architecture_value = 0;
                stored_fam.house = 0;
                stored_fam.flags &= !(FAMILY_FLAG_BIT_3 | FAMILY_FLAG_NEW_MOVE_IN);
            }
            evicted_budget_after = stored_fam.budget;
        }
    }

    // Write back updated NBRS / NGBH / TATT / FAMI, delete removed tombstone
    // character files, and save the cleared lot atomically into REDB.
    stored.write_back_world(&lot.world, house, 0, 0, None, tuning.friendship_threshold);
    let mut saved = lot
        .save()
        .map_err(|e| LifecycleError::Lot(format!("saving cleared lot {house}: {e}")))?;
    saved.deletion_epoch = stored.deletion_epoch;
    let lot_bytes = saved.encode();
    match store.save_lot_and_neighborhood_checked(
        neighborhood_id,
        house,
        Some((origin_instance_id, origin_revision)),
        &lot_bytes,
        &stored,
        &removed_character_files,
    )? {
        Ok((_, _)) => {}
        Err(CheckedCommitError::InstanceMismatch { expected, current }) => {
            return Err(LifecycleError::NeighborhoodMismatch {
                expected: format!("{neighborhood_id}#{expected:032x}"),
                actual: format!("{neighborhood_id}#{current:032x}"),
            });
        }
        Err(CheckedCommitError::RevisionMismatch { expected, current }) => {
            return Err(LifecycleError::StaleNeighborhood {
                neighborhood: neighborhood_id.to_owned(),
                expected_revision: expected,
                current_revision: current,
            });
        }
    }

    Ok((
        BulldozeReport {
            house,
            evicted_family,
            refund,
            removed_objects,
            cleared_tiles,
            deleted_tombstones,
            remaining_objects_value,
            remaining_architecture_value,
        },
        evicted_budget_after,
    ))
}

/// Evaluates `0x4274bd..0x427516` (`0x4354d5`, plus the bulldoze branch at
/// `0x4274c8..0x427514`) for one top-level object.
fn should_remove_on_evict_or_bulldoze(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    id: u16,
    bulldoze: bool,
) -> bool {
    if interpreter.valued(world, id).unwrap_or(false) {
        return true;
    }
    if !bulldoze {
        return false;
    }
    let Some(obj) = world.objects.get(&id) else {
        return false;
    };
    let flags = obj.data.get(DATA_FLAGS).copied().unwrap_or(0);
    let position = obj.position;
    let on_lot = crate::placement::stands_on_lot(world, id).unwrap_or(false);
    if !on_lot {
        let words: [i16; DEFINITION_WORDS] = match interpreter.words(world, id) {
            Ok(w) => *w,
            Err(_) => return false,
        };
        // 0x4274e2..0x4274e7: off-lot objects check only `words[42] == 0`
        // (bypassing the `FLAG_NOT_VALUED` 0x10 check); global controllers stay
        // while stray off-lot non-controllers are removed.
        return words[DEF_GLOBAL_CONTROLLER] == 0;
    }
    if flags & FLAG_NOT_VALUED != 0 {
        return false;
    }
    let [a, b, level] = World::tile(position);
    let tile_flag =
        crate::placement::tile_flags(world, a, b, i32::from(level)).unwrap_or(TILE_LOCKED);
    tile_flag & TILE_LOCKED == 0
}

/// Clears all non-`0x20` floors & walls on levels 1 and 2 (`0x427536..0x427608`)
/// and fills the pool array (`Arry #9`, `0x461a34`) with `0xff`.
fn clear_lot_architecture(world: &mut World) -> Result<usize, LifecycleError> {
    let full = FULL_SIZE as usize;
    let active = usize::try_from(world.size).unwrap_or(0).min(full);
    if active <= 2 {
        return Ok(0);
    }
    let empty_wall = ExpandedWall {
        segments: 0,
        flags: 0,
        styles: [Some(0); 6],
        patterns: [0; 8],
    };
    let mut cleared = 0usize;
    for level in 0..2 {
        let Some(flags) = world
            .arrays
            .get(&TILE_FLAGS[level])
            .filter(|a| a.stride == 1 && a.data.len() == full * full)
            .map(|a| a.data.clone())
        else {
            continue;
        };
        let mut floors = world
            .arrays
            .get(&FLOORS[level])
            .filter(|a| a.stride == 2 && a.data.len() == full * full * 2)
            .map(|a| a.data.clone());
        let mut walls = world
            .arrays
            .get(&WALLS[level])
            .filter(|a| a.stride == WALL_CELL && a.data.len() == full * full * WALL_CELL)
            .map(|a| a.data.clone());
        for y in 1..(active - 1) {
            for x in 1..(active - 1) {
                let cell = x * full + y;
                if flags[cell] & TILE_LOCKED != 0 {
                    continue;
                }
                let mut touched = false;
                if let Some(ref mut floor_data) = floors {
                    let at = cell * 2;
                    if floor_data[at] != 0 || floor_data[at + 1] != 0 {
                        floor_data[at] = 0;
                        floor_data[at + 1] = 0;
                        touched = true;
                    }
                }
                if let Some(ref mut wall_data) = walls {
                    let current = ExpandedWall::get(wall_data, active, x, y);
                    // 0x4275cb calls 0x4797e2 unconditionally on every non-0x20 tile.
                    let _ = empty_wall.put(wall_data, active, x, y);
                    if current.segments != 0 {
                        touched = true;
                    }
                }
                if touched {
                    cleared += 1;
                }
            }
        }
        if let Some(floor_data) = floors
            && let Some(array) = world.arrays.get_mut(&FLOORS[level])
        {
            array.data = floor_data;
        }
        if let Some(wall_data) = walls
            && let Some(array) = world.arrays.get_mut(&WALLS[level])
        {
            array.data = wall_data;
        }
    }
    // 0x478f42 → 0x461a34: reset pool array (Arry #9) to 0xff.
    if let Some(pool) = world.arrays.get_mut(&POOL_ARRAY) {
        pool.data.fill(0xff);
    }
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_sim::neighbors::Row;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DB_ID: AtomicU64 = AtomicU64::new(0);

    struct TempDb(PathBuf);

    impl TempDb {
        fn new(label: &str) -> Self {
            let n = TEST_DB_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "sims-vm-lifecycle-unit-{label}-{}-{n}.redb",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            Self(path)
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn cross_lot_mutations_propagate_across_goth_newbie_goth_round_trip() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db = TempDb::new("cross-lot-mutations");
        let store = NeighborhoodStore::open(&db.0).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();

        let mut goth = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
        for _ in 0..10 {
            goth.step().unwrap();
        }
        let goth_person_id = *goth.world.people.first().unwrap();
        let goth_neighbor_id = goth.world.objects[&goth_person_id]
            .person
            .as_ref()
            .unwrap()
            .data[31];
        let newbie_member_guid = store
            .load_neighborhood("UserData")
            .unwrap()
            .unwrap()
            .family_in(7)
            .unwrap()
            .1
            .members[0];
        let newbie_neighbor_id = goth
            .world
            .neighbors
            .first_of(newbie_member_guid)
            .unwrap()
            .id;

        // Mutate Goth funds, person Cooking skill (word 10), relationship to
        // Newbie, NGBH data[15], and TATT entry, then save & leave.
        goth.world.set_funds(9_876).unwrap();
        goth.world
            .objects
            .get_mut(&goth_person_id)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[10] = 777;
        goth.world.neighborhood_data[15] = 4321;
        let tatt_guid = goth.world.type_attributes[0].guid;
        if let Some(tatt) = goth.world.type_attributes.first_mut()
            && !tatt.values.is_empty()
        {
            tatt.values[0] = 321;
        }
        {
            let rec = goth.world.neighbors.get_mut(goth_neighbor_id).unwrap();
            if let Some(row) = rec.row_mut(i32::from(newbie_neighbor_id)) {
                row.values[0] = 88;
            } else {
                rec.relationships.push(Row {
                    target: i32::from(newbie_neighbor_id),
                    values: vec![88, 44],
                });
            }
        }
        goth.world.mark_friends(goth_neighbor_id);
        save_and_leave_lot(&root, &store, "UserData", &mut goth).unwrap();

        // Enter Newbie House 7: Goth's NBRS, relationship, NGBH, and TATT edits
        // must be live immediately.
        let mut newbie = enter_lot(&root, &store, "UserData", 7, &[3], Frames::default()).unwrap();
        assert_eq!(
            newbie.world.neighbors.get(goth_neighbor_id).unwrap().data[10],
            777
        );
        assert_eq!(newbie.world.neighborhood_data[15], 4321);
        assert_eq!(
            newbie
                .world
                .type_attributes
                .iter()
                .find(|t| t.guid == tatt_guid)
                .unwrap()
                .values[0],
            321
        );
        assert_eq!(
            newbie
                .world
                .neighbors
                .get(goth_neighbor_id)
                .unwrap()
                .row(i32::from(newbie_neighbor_id))
                .unwrap()
                .values[0],
            88
        );

        // Mutate Bob Newbie's Mechanical skill (word 9) & relationship back to
        // Goth, then save & leave.
        let newbie_person_id = *newbie
            .world
            .people
            .iter()
            .find(|&&id| newbie.world.objects[&id].guid == newbie_member_guid)
            .unwrap();
        newbie.world.set_funds(4_567).unwrap();
        newbie
            .world
            .objects
            .get_mut(&newbie_person_id)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[9] = 888;
        {
            let rec = newbie.world.neighbors.get_mut(newbie_neighbor_id).unwrap();
            if let Some(row) = rec.row_mut(i32::from(goth_neighbor_id)) {
                row.values[0] = 91;
            } else {
                rec.relationships.push(Row {
                    target: i32::from(goth_neighbor_id),
                    values: vec![91, 55],
                });
            }
        }
        newbie.world.mark_friends(newbie_neighbor_id);
        save_and_leave_lot(&root, &store, "UserData", &mut newbie).unwrap();

        // Re-enter Goth House 5: Goth's lot-local state AND Newbie's cross-lot
        // edits must both be present and consistent.
        let goth_again = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
        assert_eq!(goth_again.ticks, 10);
        assert_eq!(goth_again.world.budget.funds, 9_876);
        assert_eq!(
            goth_again.world.objects[&goth_person_id]
                .person
                .as_ref()
                .unwrap()
                .data[10],
            777
        );
        assert_eq!(
            goth_again
                .world
                .neighbors
                .get(newbie_neighbor_id)
                .unwrap()
                .data[9],
            888
        );
        assert_eq!(
            goth_again
                .world
                .neighbors
                .get(newbie_neighbor_id)
                .unwrap()
                .row(i32::from(goth_neighbor_id))
                .unwrap()
                .values[0],
            91
        );
    }

    #[test]
    fn pool_byte_0x10_marks_lot_built_at_zero_cost() {
        let mut world = World::empty();
        world.size = 8;
        let full = FULL_SIZE as usize;
        world.arrays.insert(
            TILE_FLAGS[0],
            sims_formats::lot::Array {
                width: full,
                height: full,
                stride: 1,
                encoding: 0,
                data: vec![0u8; full * full],
            },
        );
        world.arrays.insert(
            FLOORS[0],
            sims_formats::lot::Array {
                width: full,
                height: full,
                stride: 2,
                encoding: 0,
                data: vec![0u8; full * full * 2],
            },
        );
        world.arrays.insert(
            WALLS[0],
            sims_formats::lot::Array {
                width: full,
                height: full,
                stride: WALL_CELL,
                encoding: 0,
                data: vec![0u8; full * full * WALL_CELL],
            },
        );
        let mut pool = vec![0xffu8; full * full];
        // Cell (2, 2) has exact 0x10 sentinel (0x47b1cc: built=true, cost=0).
        pool[2 * full + 2] = 0x10;
        world.arrays.insert(
            POOL_ARRAY,
            sims_formats::lot::Array {
                width: full,
                height: full,
                stride: 1,
                encoding: 0,
                data: pool,
            },
        );
        let prices = LotPrices {
            pool: 99,
            ..LotPrices::default()
        };
        let (value, built) = architecture_value_and_built(&world, &prices);
        assert_eq!(value, 0, "pool byte 0x10 adds zero cost");
        assert!(built, "pool byte 0x10 still marks global 22 built");

        // Cell (2, 3) with value < 0x10 adds prices.pool.
        world.arrays.get_mut(&POOL_ARRAY).unwrap().data[2 * full + 3] = 0x05;
        let (value2, built2) = architecture_value_and_built(&world, &prices);
        assert_eq!(value2, 99);
        assert!(built2);
    }

    #[test]
    fn bulldoze_off_lot_ignores_flag_0x10_and_removes_group_from_non_lead_tile() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db = TempDb::new("bulldoze-edge-predicates");
        let store = NeighborhoodStore::open(&db.0).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
        let no_frames = Frames::parse("none").unwrap();
        let mut goth = enter_lot(&root, &store, "UserData", 5, &[3], no_frames).unwrap();

        // 1. Off-lot non-controller (`words[42] == 0`) with `FLAG_NOT_VALUED`
        //    (0x10) set must still be marked removable on bulldoze (`0x4274e2`).
        let interp = Interpreter::new(&goth.program, &goth.catalog, Stream::Global);
        let off_lot_non_ctrl = goth
            .world
            .order
            .iter()
            .copied()
            .find(|&id| {
                !goth.world.indicators.contains(&id)
                    && goth.world.objects[&id].person.is_none()
                    && goth.world.objects[&id].multipart.is_none()
                    && interp
                        .words(&goth.world, id)
                        .is_ok_and(|w| w[DEF_GLOBAL_CONTROLLER] == 0)
            })
            .expect("Goth lot has a single-tile non-controller object");
        {
            let obj = goth.world.objects.get_mut(&off_lot_non_ctrl).unwrap();
            obj.position = [-16, -16, 1];
            obj.data[DATA_FLAGS] |= FLAG_NOT_VALUED;
        }
        assert!(
            should_remove_on_evict_or_bulldoze(&interp, &mut goth.world, off_lot_non_ctrl, true),
            "off-lot non-controller with flag 0x10 must be removed on bulldoze (0x4274e2)"
        );

        // 2. Multi-tile group whose lead tile is marked `FLAG_NOT_VALUED` (0x10)
        //    while a non-lead tile is unlocked & unflagged must still be
        //    deleted as a whole group when `bulldoze` scans all registered IDs.
        let (lead_id, non_lead_id) = goth
            .world
            .order
            .iter()
            .copied()
            .find_map(|id| {
                let obj = goth.world.objects.get(&id)?;
                let part = obj.multipart.as_ref()?;
                if part.parent != 0 && !goth.world.indicators.contains(&id) {
                    Some((u16::try_from(part.parent).ok()?, id))
                } else {
                    None
                }
            })
            .expect("Goth lot has a multi-tile object");
        let lead_guid = goth.world.objects[&lead_id].guid;
        // Make the lead tile fail both `valued` (set FLAG_NOT_VALUED) and the
        // on-lot check while leaving the non-lead tile removable on bulldoze.
        goth.world.objects.get_mut(&lead_id).unwrap().data[DATA_FLAGS] |= FLAG_NOT_VALUED;
        goth.world.objects.get_mut(&non_lead_id).unwrap().data[DATA_FLAGS] &= !FLAG_NOT_VALUED;
        // Also test Magic Controller (`0xb6c90029`) writeback on the same save.
        if let Some(ctrl_id) = goth
            .world
            .order
            .iter()
            .copied()
            .find(|&id| goth.world.objects[&id].guid == 0xb6c9_0029)
        {
            let attrs = &mut goth.world.objects.get_mut(&ctrl_id).unwrap().attributes;
            if attrs.len() >= 6 {
                for (i, slot) in attrs.iter_mut().take(6).enumerate() {
                    *slot = 1000 + i as i16;
                }
            }
        }
        save_and_leave_lot(&root, &store, "UserData", &mut goth).unwrap();

        let nh_saved = store.load_neighborhood("UserData").unwrap().unwrap();
        assert_eq!(
            nh_saved.families[&5].magic_history().unwrap(),
            Some([1000, 1001, 1002, 1003, 1004, 1005]),
            "Magic Controller attributes 0..6 must merge into FAMI v9 magic_history on save"
        );

        let _ = bulldoze(&root, &store, "UserData", 5).unwrap();
        let after_doze = enter_lot(
            &root,
            &store,
            "UserData",
            5,
            &[3],
            Frames::parse("none").unwrap(),
        )
        .unwrap();
        assert!(
            after_doze
                .world
                .objects
                .values()
                .all(|o| o.id != lead_id && o.id != non_lead_id && o.guid != lead_guid),
            "multi-tile group whose non-lead tile qualifies must be removed on bulldoze"
        );
    }

    #[test]
    fn saved_lot_with_deleted_bulldozed_visitor_reopens_and_steps_cleanly() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db = TempDb::new("deleted-visitor-purge");
        let store = NeighborhoodStore::open(&db.0).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
        let no_frames = Frames::parse("none").unwrap();
        let chars_before = store.load_character_files("UserData").unwrap().len();

        // Save House 7 containing two live visitors with GUID `0x7757D9EE`
        // (Goth House 5 tombstone whose loose `.iff` is deleted on bulldoze):
        //   1. `visitor_ground` stacked at the head of an occupied ground-floor
        //      tile ahead of a surviving object `survivor_id`.
        //   2. `visitor_upstairs` standing on the upper floor (level 2, grid 103).
        let mut house7 = enter_lot(&root, &store, "UserData", 7, &[3], no_frames.clone()).unwrap();
        let template_id = house7.world.people[0];
        let base_id = house7.world.objects.keys().copied().max().unwrap_or(100) + 1;
        let visitor_ground_id = base_id;
        let visitor_upstairs_id = base_id + 1;

        // Find an occupied ground-floor tile with a surviving non-person head.
        let (survivor_id, ground_tile) = house7
            .world
            .order
            .iter()
            .copied()
            .find_map(|oid| {
                let obj = house7.world.objects.get(&oid)?;
                if obj.person.is_some()
                    || obj.multipart.is_some()
                    || house7.world.indicators.contains(&oid)
                {
                    return None;
                }
                let tile = World::tile(obj.position);
                if tile[2] != 1 || tile[0] <= 0 || tile[1] <= 0 {
                    return None;
                }
                if crate::placement::head(&house7.world, tile) == Some(oid) {
                    Some((oid, tile))
                } else {
                    None
                }
            })
            .expect("House 7 has a ground-floor tile head object");

        let tomb_nbr = house7.world.neighbors.first_of(0x7757_D9EE).unwrap().id;
        let next_type = house7.world.types.keys().copied().max().unwrap_or(0) + 1;
        house7.world.types.insert(next_type, 0x7757_D9EE);

        let full = FULL_SIZE as usize;
        let write_grid_head = |world: &mut World, array_id: u16, tile: [i8; 3], val: u16| {
            let cell = (tile[0] as usize * full + tile[1] as usize) * 2;
            world.arrays.get_mut(&array_id).unwrap().data[cell..cell + 2]
                .copy_from_slice(&val.to_le_bytes());
        };

        // 1. Place `visitor_ground_id` as the head of `ground_tile`, pushing
        //    `survivor_id` into `visitor_ground_id`'s slot 0.
        let mut visitor_ground = house7.world.objects[&template_id].clone();
        visitor_ground.id = visitor_ground_id;
        visitor_ground.guid = 0x7757_D9EE;
        visitor_ground.selected = false;
        visitor_ground.position = house7.world.objects[&survivor_id].position;
        visitor_ground.data[2] = 0; // CONTAINER
        visitor_ground.data[3] = -1; // CONTAINER_SLOT
        visitor_ground.data[14] = -1; // SLOT (tile head)
        visitor_ground.data[26] = 0; // BEFORE
        visitor_ground.slots[0].object = survivor_id as i16;
        if let Some(person) = visitor_ground.person.as_mut() {
            person.data[31] = tomb_nbr;
            person.data[PERSON_HOME] = 5;
            person.data[61] = 0;
        }
        {
            let surv = house7.world.objects.get_mut(&survivor_id).unwrap();
            surv.data[14] = 0; // SLOT (linked after visitor_ground in slot 0)
            surv.data[26] = visitor_ground_id as i16; // BEFORE
        }
        write_grid_head(&mut house7.world, 3, ground_tile, visitor_ground_id);
        house7
            .world
            .objects
            .insert(visitor_ground_id, visitor_ground);
        house7.world.order.push(visitor_ground_id);
        house7.world.people.push(visitor_ground_id);

        // 2. Place `visitor_upstairs_id` on an empty upper-floor tile (level 2, grid 103).
        let upstairs_tile = [12i8, 12i8, 2i8];
        let mut visitor_upstairs = house7.world.objects[&template_id].clone();
        visitor_upstairs.id = visitor_upstairs_id;
        visitor_upstairs.guid = 0x7757_D9EE;
        visitor_upstairs.selected = false;
        visitor_upstairs.position = [
            i32::from(upstairs_tile[0]) * 16,
            i32::from(upstairs_tile[1]) * 16,
            2,
        ];
        visitor_upstairs.data[2] = 0;
        visitor_upstairs.data[3] = -1;
        visitor_upstairs.data[14] = -1;
        visitor_upstairs.data[26] = 0;
        if !visitor_upstairs.slots.is_empty() {
            visitor_upstairs.slots[0].object = 0;
        }
        if let Some(person) = visitor_upstairs.person.as_mut() {
            person.data[31] = tomb_nbr;
            person.data[PERSON_HOME] = 5;
            person.data[61] = 0;
        }
        write_grid_head(&mut house7.world, 103, upstairs_tile, visitor_upstairs_id);
        house7
            .world
            .objects
            .insert(visitor_upstairs_id, visitor_upstairs);
        house7.world.order.push(visitor_upstairs_id);
        house7.world.people.push(visitor_upstairs_id);

        // 3. Stage the standing resident `template_id` actively executing BHAV
        //    4096 owned by `next_type` (`0x7757D9EE`) targeting
        //    `visitor_ground_id`, WITH an unrelated player order queued on
        //    `survivor_id` (must survive visitor cleanup!).
        let unrelated_order = sims_formats::saved_objects::Interaction {
            target: survivor_id as i16,
            icon: survivor_id as i16,
            action: 4096,
            priority: 50,
            ..crate::object_uses::no_action()
        };
        {
            let resident = house7.world.objects.get_mut(&template_id).unwrap();
            resident.stack.push(sims_formats::saved_objects::Frame {
                stack_object: visitor_ground_id as i16,
                tree: 4096,
                node: 0,
                parameters: Vec::new(),
                locals: Vec::new(),
                primitive_state: 0,
                code_owner_type: next_type as i16,
            });
            if let Some(person) = resident.person.as_mut() {
                person.active.target = visitor_ground_id as i16;
                person.active.icon = visitor_ground_id as i16;
                person.active.action = 4096;
                person.queue.push(unrelated_order.clone());
            }
            house7.world.frame_storage.remove(&template_id);
        }

        // 4. Also test a SEATED resident (placed in a container slot of an
        //    object on the lot) running BHAV 4096 owned by `0x7757D9EE` with an
        //    unrelated queued order: `enter_lot` must recover the seated
        //    resident cleanly without tripping `reset_person`'s container trap.
        let seated_resident_id = house7
            .world
            .people
            .iter()
            .copied()
            .find(|&pid| {
                pid != template_id && pid != visitor_ground_id && pid != visitor_upstairs_id
            })
            .expect("House 7 has a second resident (Betty Newbie)");
        let container_id = house7
            .world
            .order
            .iter()
            .copied()
            .find(|&oid| {
                oid != survivor_id
                    && !house7.world.indicators.contains(&oid)
                    && house7.world.objects[&oid].person.is_none()
                    && house7.world.objects[&oid].slots.len() > 1
            })
            .expect("House 7 has a furniture object with a container slot");
        let visitor_used_obj_id = house7
            .world
            .order
            .iter()
            .copied()
            .find(|&oid| {
                oid != survivor_id
                    && oid != container_id
                    && !house7.world.indicators.contains(&oid)
                    && house7.world.objects[&oid].person.is_none()
                    && house7.world.objects[&oid].data.get(62).copied() == Some(0)
            })
            .expect("House 7 has an unused surviving object for the visitor to use");
        let container_users_before = house7.world.objects[&container_id].data[62];
        {
            // Mark `visitor_used_obj_id` in use by `visitor_ground_id` (`data[62] += 1`).
            house7
                .world
                .objects
                .get_mut(&visitor_used_obj_id)
                .unwrap()
                .data[62] += 1;
            if let Some(person) = house7
                .world
                .objects
                .get_mut(&visitor_ground_id)
                .and_then(|o| o.person.as_mut())
            {
                person
                    .object_uses
                    .push(sims_formats::saved_objects::ObjectUse {
                        target: visitor_used_obj_id as i16,
                        stack_depth: 1,
                        unknown: 0,
                    });
            }
        }
        {
            let _ = crate::placement::unlink_off_lot(&mut house7.world, seated_resident_id);
            let container_obj = house7.world.objects.get_mut(&container_id).unwrap();
            container_obj.slots[1].object = seated_resident_id as i16;
            container_obj.data[62] += 1;
            let container_pos = container_obj.position;
            let seated = house7.world.objects.get_mut(&seated_resident_id).unwrap();
            seated.position = container_pos;
            seated.data[2] = container_id as i16; // CONTAINER
            seated.data[3] = 0; // CONTAINER_SLOT (slot 1 - 1)
            seated.data[14] = 1; // SLOT (physical slot 1)
            seated.data[26] = container_id as i16; // BEFORE
            seated.stack.push(sims_formats::saved_objects::Frame {
                stack_object: visitor_ground_id as i16,
                tree: 4096,
                node: 0,
                parameters: Vec::new(),
                locals: Vec::new(),
                primitive_state: 0,
                code_owner_type: next_type as i16,
            });
            if let Some(person) = seated.person.as_mut() {
                person
                    .object_uses
                    .push(sims_formats::saved_objects::ObjectUse {
                        target: container_id as i16,
                        stack_depth: 1,
                        unknown: 0,
                    });
                person.active.target = visitor_ground_id as i16;
                person.active.icon = visitor_ground_id as i16;
                person.active.action = 4096;
                person.queue.push(unrelated_order);
            }
            house7.world.frame_storage.remove(&seated_resident_id);
        }
        save_and_leave_lot(&root, &store, "UserData", &mut house7).unwrap();

        // Bulldoze House 5: deletes `0x7757D9EE` from NBRS and its loose `.iff`.
        let report = bulldoze(&root, &store, "UserData", 5).unwrap();
        assert!(report.deleted_tombstones >= 1);
        let nh_after = store.load_neighborhood("UserData").unwrap().unwrap();
        assert!(nh_after.deleted_characters.contains(&0x7757_D9EE));
        assert!(
            nh_after
                .neighbors
                .iter()
                .flatten()
                .all(|n| n.guid != 0x7757_D9EE)
        );
        assert!(
            store.load_character_files("UserData").unwrap().len() < chars_before,
            "bulldozing House 5 must remove deleted tombstone character files from REDB"
        );

        // Reopening House 7 must purge both deleted visitors cleanly, promote
        // `survivor_id` back to the ground-floor tile head, clear the upstairs
        // grid cell, decrement data[62] use counts on surviving objects used by
        // the deleted visitor or seated resident, preserve both residents'
        // unrelated queued orders, recover the seated resident cleanly, and step!
        let mut house7_after = enter_lot(&root, &store, "UserData", 7, &[3], no_frames).unwrap();
        assert!(!house7_after.world.objects.contains_key(&visitor_ground_id));
        assert!(!house7_after.world.people.contains(&visitor_ground_id));
        assert!(
            !house7_after
                .world
                .objects
                .contains_key(&visitor_upstairs_id)
        );
        assert!(!house7_after.world.people.contains(&visitor_upstairs_id));
        assert_eq!(
            crate::placement::head(&house7_after.world, ground_tile),
            Some(survivor_id),
            "surviving object behind purged visitor must be promoted to the tile head"
        );
        let survivor_after = &house7_after.world.objects[&survivor_id];
        assert_eq!(survivor_after.data[14], -1);
        assert_eq!(survivor_after.data[26], 0);
        assert_eq!(
            crate::placement::head(&house7_after.world, upstairs_tile),
            None,
            "upper-floor grid must not retain purged visitor ID"
        );
        assert_eq!(
            house7_after.world.objects[&container_id].data[62], container_users_before,
            "seated resident's chair must have its data[62] use count decremented"
        );
        assert_eq!(
            house7_after.world.objects[&visitor_used_obj_id].data[62], 0,
            "object used by purged visitor must have its data[62] use count decremented"
        );
        assert!(
            house7_after.world.objects[&template_id]
                .person
                .as_ref()
                .unwrap()
                .queue
                .iter()
                .any(|a| a.target == survivor_id as i16),
            "standing resident's unrelated queued player order must survive visitor purge"
        );
        assert!(
            house7_after.world.objects[&seated_resident_id]
                .person
                .as_ref()
                .unwrap()
                .queue
                .iter()
                .any(|a| a.target == survivor_id as i16),
            "seated resident's unrelated queued player order must survive visitor purge"
        );
        for _ in 0..3 {
            house7_after.step().unwrap();
        }
        save_and_leave_lot(&root, &store, "UserData", &mut house7_after).unwrap();
    }

    #[test]
    fn evict_and_bulldoze_succeed_when_seated_resident_interacts_with_another_resident() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let no_frames = Frames::parse("none").unwrap();
        for do_bulldoze in [false, true] {
            let db = TempDb::new(if do_bulldoze {
                "seated-interact-bulldoze"
            } else {
                "seated-interact-evict"
            });
            let store = NeighborhoodStore::open(&db.0).unwrap();
            store
                .import_directory("UserData", &root.join("UserData"), 0xff)
                .unwrap();

            let mut house7 =
                enter_lot(&root, &store, "UserData", 7, &[3], no_frames.clone()).unwrap();
            let seated_id = house7.world.people[0];
            let target_id = house7.world.people[1];
            let chair_id = house7
                .world
                .order
                .iter()
                .copied()
                .find(|&oid| {
                    !house7.world.indicators.contains(&oid)
                        && house7.world.objects[&oid].person.is_none()
                        && house7.world.objects[&oid].slots.len() > 1
                })
                .expect("House 7 has a container furniture object");

            let _ = crate::placement::unlink_off_lot(&mut house7.world, seated_id);
            house7.world.objects.get_mut(&chair_id).unwrap().slots[1].object = seated_id as i16;
            house7.world.objects.get_mut(&chair_id).unwrap().data[62] += 1;
            house7.world.objects.get_mut(&target_id).unwrap().data[62] += 1;
            let chair_pos = house7.world.objects[&chair_id].position;
            let seated = house7.world.objects.get_mut(&seated_id).unwrap();
            seated.position = chair_pos;
            seated.data[2] = chair_id as i16;
            seated.data[3] = 0;
            seated.data[14] = 1;
            seated.data[26] = chair_id as i16;
            if let Some(person) = seated.person.as_mut() {
                person
                    .object_uses
                    .push(sims_formats::saved_objects::ObjectUse {
                        target: chair_id as i16,
                        stack_depth: 1,
                        unknown: 0,
                    });
                person
                    .object_uses
                    .push(sims_formats::saved_objects::ObjectUse {
                        target: target_id as i16,
                        stack_depth: 1,
                        unknown: 1,
                    });
                person.active.target = target_id as i16;
                person.active.icon = target_id as i16;
            }
            save_and_leave_lot(&root, &store, "UserData", &mut house7).unwrap();

            if do_bulldoze {
                let report = bulldoze(&root, &store, "UserData", 7).unwrap();
                assert_eq!(report.evicted_family, Some(1));
            } else {
                let report = evict(&root, &store, "UserData", 7).unwrap();
                assert_eq!(report.family, 1);
            }
            assert!(
                store
                    .load_neighborhood("UserData")
                    .unwrap()
                    .unwrap()
                    .family_in(7)
                    .is_none()
            );
        }
    }

    #[test]
    fn reopening_answered_dialog_lot_preserves_chosen_answer() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let no_frames = Frames::parse("none").unwrap();

        let save_answered = |label: &str, chosen: u8| -> (TempDb, NeighborhoodStore) {
            let db = TempDb::new(label);
            let store = NeighborhoodStore::open(&db.0).unwrap();
            store
                .import_directory("UserData", &root.join("UserData"), 0xff)
                .unwrap();
            let mut lot = enter_lot(&root, &store, "UserData", 5, &[], no_frames.clone()).unwrap();
            lot.wait_for_answers();
            lot.step().unwrap();
            assert_eq!(
                lot.step().unwrap_err(),
                crate::boot::StepError::Asking { tick: 2 }
            );
            lot.answer(chosen).unwrap();
            save_and_leave_lot(&root, &store, "UserData", &mut lot).unwrap();
            (db, store)
        };

        // 1. Answered 3, saved before stepping, reopened with `&[]` (baseline).
        let (_db_three, store_three) = save_answered("dialog-answer-three", 3);
        let mut baseline_three =
            enter_lot(&root, &store_three, "UserData", 5, &[], no_frames.clone()).unwrap();
        for _ in 0..4 {
            baseline_three.step().unwrap();
        }
        let hash_three = baseline_three.world.hash();

        // 2. Answered 2, saved before stepping, reopened with `&[]` (contrast).
        let (_db_two, store_two) = save_answered("dialog-answer-two", 2);
        let mut baseline_two =
            enter_lot(&root, &store_two, "UserData", 5, &[], no_frames.clone()).unwrap();
        for _ in 0..4 {
            baseline_two.step().unwrap();
        }
        let hash_two = baseline_two.world.hash();
        assert_ne!(
            hash_three, hash_two,
            "tick-2 Help System answers 3 and 2 must diverge in world state"
        );

        // 3. Answered 3, saved before stepping, reopened with `answers = &[2]`:
        //    replay must apply the saved answer 3 (matching `hash_three`), NOT
        //    overwrite it with 2, and keep 2 staged in `choices_after` behind it.
        let mut reopened = enter_lot(&root, &store_three, "UserData", 5, &[2], no_frames).unwrap();
        assert!(reopened.answered);
        assert_eq!(reopened.player.queue.len(), 1);
        assert_eq!(reopened.player.queue[0].status, 3);
        assert_eq!(
            reopened
                .player
                .choices_after
                .as_ref()
                .map(|q| q.iter().map(|a| a.status).collect::<Vec<_>>()),
            Some(vec![2])
        );
        for _ in 0..4 {
            reopened.step().unwrap();
        }
        assert_eq!(
            reopened.world.hash(),
            hash_three,
            "reopening an already-answered lot with different future answers must replay the saved answer (3), not replace it with 2"
        );
        assert_eq!(
            reopened
                .player
                .choices_after
                .as_ref()
                .map(|q| q.iter().map(|a| a.status).collect::<Vec<_>>()),
            Some(vec![2])
        );
    }

    #[test]
    fn evict_and_bulldoze_succeed_with_seated_visitor() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let no_frames = Frames::parse("none").unwrap();
        for do_bulldoze in [false, true] {
            let db = TempDb::new(if do_bulldoze {
                "seated-visitor-bulldoze"
            } else {
                "seated-visitor-evict"
            });
            let store = NeighborhoodStore::open(&db.0).unwrap();
            store
                .import_directory("UserData", &root.join("UserData"), 0xff)
                .unwrap();

            let mut house7 =
                enter_lot(&root, &store, "UserData", 7, &[3], no_frames.clone()).unwrap();
            let goth_guid = store
                .load_neighborhood("UserData")
                .unwrap()
                .unwrap()
                .family_in(5)
                .unwrap()
                .1
                .members[0];
            let goth_nbr = house7.world.neighbors.first_of(goth_guid).unwrap().id;
            let template_id = house7.world.people[0];
            let visitor_id = house7.world.objects.keys().copied().max().unwrap_or(100) + 1;
            let next_type = house7.world.types.keys().copied().max().unwrap_or(0) + 1;
            house7.world.types.insert(next_type, goth_guid);

            let chair_id = house7
                .world
                .order
                .iter()
                .copied()
                .find(|&oid| {
                    !house7.world.indicators.contains(&oid)
                        && house7.world.objects[&oid].person.is_none()
                        && house7.world.objects[&oid].slots.len() > 1
                })
                .expect("House 7 has a furniture chair with a container slot");
            let chair_pos = house7.world.objects[&chair_id].position;

            let mut visitor = house7.world.objects[&template_id].clone();
            visitor.id = visitor_id;
            visitor.guid = goth_guid;
            visitor.selected = false;
            visitor.position = chair_pos;
            visitor.data[2] = chair_id as i16;
            visitor.data[3] = 0;
            visitor.data[14] = 1;
            visitor.data[26] = chair_id as i16;
            if let Some(person) = visitor.person.as_mut() {
                person.data[31] = goth_nbr;
                person.data[PERSON_HOME] = 5;
                person.data[61] = 5;
            }
            house7.world.objects.get_mut(&chair_id).unwrap().slots[1].object = visitor_id as i16;
            house7.world.objects.insert(visitor_id, visitor);
            house7.world.order.push(visitor_id);
            house7.world.people.push(visitor_id);
            save_and_leave_lot(&root, &store, "UserData", &mut house7).unwrap();

            if do_bulldoze {
                let report = bulldoze(&root, &store, "UserData", 7).unwrap();
                assert_eq!(report.evicted_family, Some(1));
            } else {
                let report = evict(&root, &store, "UserData", 7).unwrap();
                assert_eq!(report.family, 1);
            }
            assert!(
                store
                    .load_neighborhood("UserData")
                    .unwrap()
                    .unwrap()
                    .family_in(7)
                    .is_none()
            );
        }
    }

    #[test]
    fn bulldozing_lot_allows_another_imported_lot_with_unused_deleted_objt_to_open() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db = TempDb::new("unused-deleted-objt");
        let store = NeighborhoodStore::open(&db.0).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();

        // Append an unused `objt` entry for Goth House 5 tombstone `0x7757D9EE`
        // to House 7's imported base `House07.iff`.
        let orig_bytes = store.load_base_house("UserData", 7).unwrap().unwrap();
        let iff = sims_formats::iff::Iff::parse(orig_bytes).unwrap();
        let mut rebuilt = Vec::new();
        rebuilt.extend_from_slice(b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1");
        rebuilt.extend_from_slice(&0u32.to_be_bytes()); // no rsmp offset
        for chunk in &iff.chunks {
            if &chunk.tag == b"rsmp" {
                continue;
            }
            let mut payload = iff.data(chunk).to_vec();
            if &chunk.tag == b"objt" {
                let parsed = sims_formats::lot::ObjectTypes::parse(&payload).unwrap();
                let new_id = parsed
                    .entries
                    .iter()
                    .map(|e| e.id)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);
                payload.extend_from_slice(&0x7757_D9EEu32.to_le_bytes());
                payload.extend_from_slice(&0u32.to_le_bytes());
                payload.extend_from_slice(&0u32.to_le_bytes());
                payload.extend_from_slice(&new_id.to_le_bytes());
                payload.extend_from_slice(&2u16.to_le_bytes());
                payload.extend_from_slice(b"tomb\0\0");
                if parsed.version == 3 {
                    payload.extend_from_slice(&0u32.to_le_bytes());
                }
            }
            let total_size = (76 + payload.len()) as u32;
            rebuilt.extend_from_slice(&chunk.tag);
            rebuilt.extend_from_slice(&total_size.to_be_bytes());
            rebuilt.extend_from_slice(&chunk.id.to_be_bytes());
            rebuilt.extend_from_slice(&chunk.flags.to_be_bytes());
            let mut label = [0u8; 64];
            let raw_label = chunk.label.as_bytes();
            let n = raw_label.len().min(63);
            label[..n].copy_from_slice(&raw_label[..n]);
            rebuilt.extend_from_slice(&label);
            rebuilt.extend_from_slice(&payload);
        }
        store.save_base_house("UserData", 7, &rebuilt).unwrap();

        let no_frames = Frames::parse("none").unwrap();
        // Opens cleanly before Bulldozing House 5.
        let _before = enter_lot(&root, &store, "UserData", 7, &[3], no_frames.clone()).unwrap();
        // Bulldoze House 5: deletes `0x7757D9EE` character file from REDB.
        let report = bulldoze(&root, &store, "UserData", 5).unwrap();
        assert!(report.deleted_tombstones >= 1);
        // Must STILL open cleanly after Bulldozing House 5!
        let mut after = enter_lot(&root, &store, "UserData", 7, &[3], no_frames).unwrap();
        after.step().unwrap();
    }

    #[test]
    fn multi_dialog_and_replay_mode_answers_preserve_queue_and_auto_answer_notices() {
        use crate::Player;
        use crate::tick::Shown;

        // 1. Explicit `Answers::replay` queue with appended choices must consume
        //    the saved replay answer first, then switch to choice-only mode so a
        //    later one-button notice auto-answers WITHOUT consuming the choice!
        let mut player = Answers::replay(&[crate::Answer::from(2)]);
        player.extend_choices_after(&[3]);
        let choice_dialog = Shown {
            tick: 1,
            object: 1,
            answers: &[2, 3],
            text_entry: false,
        };
        let one_button_notice = Shown {
            tick: 2,
            object: 1,
            answers: &[2],
            text_entry: false,
        };
        assert_eq!(player.answer(&choice_dialog).map(|a| a.status), Some(2));
        assert_eq!(
            player.answer(&one_button_notice).map(|a| a.status),
            Some(2),
            "one-button notice after drained replay queue must auto-answer without consuming choice 3"
        );
        assert_eq!(
            player.answer(&choice_dialog).map(|a| a.status),
            Some(3),
            "appended choice 3 must still be available for the next multi-button dialog"
        );
        assert_eq!(
            player.answer(&one_button_notice).map(|a| a.status),
            Some(2),
            "subsequent one-button notice must continue to auto-answer in choice mode"
        );

        // 2. Saving a lot while paused on a second dialog (`answered == false`,
        //    `replay_view.is_some()`, first answer `2` already in `saved.answers`)
        //    and reopening with `[3, 3]` must keep `2` at the front and stage
        //    `[3, 3]` behind it (`[2, 3]` instead of `[3, 3]`).
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db = TempDb::new("multi-dialog-replay-view");
        let store = NeighborhoodStore::open(&db.0).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
        let no_frames = Frames::parse("none").unwrap();
        let mut lot = enter_lot(&root, &store, "UserData", 5, &[], no_frames.clone()).unwrap();
        // Simulate a tick paused on a second dialog after answering the first
        // dialog with `2` (`answered = false`, `replay_view = Some(...)`,
        // `player = Answers::replay([2])`).
        lot.player = Answers::replay(&[crate::Answer::from(2)]);
        lot.answered = false;
        lot.replay_view = Some((lot.view_rotation, lot.view_centre));
        save_and_leave_lot(&root, &store, "UserData", &mut lot).unwrap();

        let mut reopened = enter_lot(&root, &store, "UserData", 5, &[3, 3], no_frames).unwrap();
        assert_eq!(
            reopened.player.answer(&choice_dialog).map(|a| a.status),
            Some(2),
            "first already-chosen answer must be preserved when replay_view is active"
        );
        assert_eq!(
            reopened.player.answer(&one_button_notice).map(|a| a.status),
            Some(2),
            "one-button notice after saved replay answer must auto-answer"
        );
        assert_eq!(
            reopened.player.answer(&choice_dialog).map(|a| a.status),
            Some(3),
            "second dialog must receive caller-supplied choice 3"
        );

        // 3. Interactive `push_back_answer` after reopening an explicit-replay
        //    save with `[3]` must append onto `choices_after` (NOT `queue`),
        //    so retrying `[Choice1, Notice, Choice2, Choice3]` applies
        //    `[2, 2, 3, 3]` cleanly instead of feeding `3` into `Notice`.
        let mut replay_plus_interactive = Answers::replay(&[crate::Answer::from(2)]);
        replay_plus_interactive.extend_choices_after(&[3]);
        replay_plus_interactive.push_back_answer(crate::Answer::from(3));
        assert_eq!(
            replay_plus_interactive
                .answer(&choice_dialog)
                .map(|a| a.status),
            Some(2)
        );
        assert_eq!(
            replay_plus_interactive
                .answer(&one_button_notice)
                .map(|a| a.status),
            Some(2),
            "interactive answer appended after replay+choices must NOT intercept the earlier one-button notice"
        );
        assert_eq!(
            replay_plus_interactive
                .answer(&choice_dialog)
                .map(|a| a.status),
            Some(3)
        );
        assert_eq!(
            replay_plus_interactive
                .answer(&choice_dialog)
                .map(|a| a.status),
            Some(3)
        );
    }

    #[test]
    fn recreating_deleted_made_character_guid_leaves_neighborhood_loadable() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db = TempDb::new("reuse-deleted-made-guid");
        let store = NeighborhoodStore::open(&db.0).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
        let no_frames = Frames::parse("none").unwrap();

        // 1. Add a runtime-made character `0x61520029` homed on House 5
        //    (non-member), save House 5, then bulldoze House 5 (deleting it).
        let guid = 0x6152_0029u32;
        let mut house5 = enter_lot(&root, &store, "UserData", 5, &[3], no_frames.clone()).unwrap();
        let template_guid = house5.world.objects[&house5.world.people[0]].guid;
        let next_file = house5
            .world
            .characters
            .made
            .values()
            .map(|c| c.file)
            .max()
            .unwrap_or(900)
            + 1;
        house5.world.characters.made.insert(
            guid,
            sims_sim::characters::Character {
                template: template_guid,
                file: next_file,
                strings: BTreeMap::new(),
            },
        );
        let new_nbr_id = (house5.world.neighbors.slots.len() as i16) + 1;
        let mut data = vec![0i16; sims_sim::neighbors::PERSON_WORDS];
        data[PERSON_HOME] = 5;
        house5
            .world
            .neighbors
            .slots
            .push(Some(sims_sim::neighbors::Neighbor {
                id: new_nbr_id,
                guid,
                revision: 9,
                data,
                relationships: Vec::new(),
                friends: 0,
                stale: false,
            }));
        save_and_leave_lot(&root, &store, "UserData", &mut house5).unwrap();

        let dozed = bulldoze(&root, &store, "UserData", 5).unwrap();
        assert!(dozed.deleted_tombstones >= 1);
        assert!(
            store
                .load_neighborhood("UserData")
                .unwrap()
                .unwrap()
                .deleted_characters
                .contains(&guid)
        );

        // 2. Recreate the exact same GUID `0x61520029` on House 7 and save.
        let mut house7 = enter_lot(&root, &store, "UserData", 7, &[3], no_frames.clone()).unwrap();
        house7.world.characters.made.insert(
            guid,
            sims_sim::characters::Character {
                template: template_guid,
                file: next_file + 1,
                strings: BTreeMap::new(),
            },
        );
        let nbr_id2 = (house7.world.neighbors.slots.len() as i16) + 1;
        let mut data2 = vec![0i16; sims_sim::neighbors::PERSON_WORDS];
        data2[PERSON_HOME] = 7;
        house7
            .world
            .neighbors
            .slots
            .push(Some(sims_sim::neighbors::Neighbor {
                id: nbr_id2,
                guid,
                revision: 9,
                data: data2,
                relationships: Vec::new(),
                friends: 0,
                stale: false,
            }));
        save_and_leave_lot(&root, &store, "UserData", &mut house7).unwrap();

        // 3. `deleted_characters` no longer suppresses `guid`, and unrelated
        //    House 4 opens cleanly!
        let nh = store.load_neighborhood("UserData").unwrap().unwrap();
        assert!(!nh.deleted_characters.contains(&guid));
        assert!(nh.made_characters.made.contains_key(&guid));
        let _house4 = enter_lot(&root, &store, "UserData", 4, &[3], no_frames).unwrap();
    }

    #[test]
    fn bulldozing_lot_strips_deleted_character_instance_from_another_imported_base_house() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db = TempDb::new("bulldoze-strips-base-instance");
        let store = NeighborhoodStore::open(&db.0).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
        let no_frames = Frames::parse("none").unwrap();

        // Remove Bob Newbie (`0x32AA2056`) from family 1 and set his NBRS home
        // to House 5 while leaving imported base `House07.iff` untouched.
        let bob_guid = 0x32AA_2056u32;
        let mut nh = store.load_neighborhood("UserData").unwrap().unwrap();
        nh.families
            .get_mut(&1)
            .unwrap()
            .members
            .retain(|&g| g != bob_guid);
        for nbr in nh.neighbors.iter_mut().flatten() {
            if nbr.guid == bob_guid {
                nbr.person_data[PERSON_HOME] = 5;
            }
        }
        store.save_neighborhood("UserData", &nh).unwrap();

        // House 7 opens before bulldozing House 5.
        let _before = enter_lot(&root, &store, "UserData", 7, &[3], no_frames.clone()).unwrap();

        // Bulldoze House 5: deletes Bob Newbie's loose `.iff` and NBRS record.
        let _ = bulldoze(&root, &store, "UserData", 5).unwrap();
        let nh_after = store.load_neighborhood("UserData").unwrap().unwrap();
        assert!(nh_after.deleted_characters.contains(&bob_guid));

        // Base `House07.iff` (still containing Bob's `ObjM` instance) must
        // strip Bob cleanly and open + step without error!
        let mut after = enter_lot(&root, &store, "UserData", 7, &[3], no_frames.clone()).unwrap();
        assert!(after.world.objects.values().all(|o| o.guid != bob_guid));
        after.step().unwrap();

        // Now save a runtime-made replacement reusing Bob's deleted loose GUID
        // (`0x32AA2056`) on House 4 (clearing `deleted_characters` while
        // `deleted_at_epoch` retains the deletion epoch): untouched imported
        // base `House07.iff` MUST STILL parse, strip the stale base instance,
        // and open + step cleanly!
        move_in(&root, &store, "UserData", 2, 4).unwrap();
        let mut house4 = enter_lot(&root, &store, "UserData", 4, &[3], no_frames.clone()).unwrap();
        let template_guid = house4.world.objects[&house4.world.people[0]].guid;
        house4.world.characters.made.insert(
            bob_guid,
            sims_sim::characters::Character {
                template: template_guid,
                file: 905,
                strings: BTreeMap::new(),
            },
        );
        let new_nbr_id = (house4.world.neighbors.slots.len() as i16) + 1;
        let mut new_data = vec![0i16; sims_sim::neighbors::PERSON_WORDS];
        new_data[PERSON_HOME] = 4;
        new_data[61] = 2;
        house4
            .world
            .neighbors
            .slots
            .push(Some(sims_sim::neighbors::Neighbor {
                id: new_nbr_id,
                guid: bob_guid,
                revision: 9,
                data: new_data,
                relationships: Vec::new(),
                friends: 0,
                stale: false,
            }));
        save_and_leave_lot(&root, &store, "UserData", &mut house4).unwrap();

        let mut after_reuse = enter_lot(&root, &store, "UserData", 7, &[3], no_frames).unwrap();
        assert!(
            after_reuse
                .world
                .objects
                .values()
                .all(|o| o.guid != bob_guid)
        );
        after_reuse.step().unwrap();
    }

    #[test]
    fn stock_house_56_can_be_evicted_and_bulldozed() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        for do_bulldoze in [false, true] {
            let db = TempDb::new(if do_bulldoze {
                "stock-house-56-bulldoze"
            } else {
                "stock-house-56-evict"
            });
            let store = NeighborhoodStore::open(&db.0).unwrap();
            store
                .import_directory("UserData", &root.join("UserData"), 0xff)
                .unwrap();
            let before = store.load_neighborhood("UserData").unwrap().unwrap();
            let (fam_id, fam_before) = before.family_in(56).expect("House 56 is occupied");
            let expected_net_worth = fam_before.budget + fam_before.architecture_value;

            if do_bulldoze {
                let report = bulldoze(&root, &store, "UserData", 56).unwrap();
                assert_eq!(report.evicted_family, Some(fam_id));
            } else {
                let report = evict(&root, &store, "UserData", 56).unwrap();
                assert_eq!(report.family, fam_id);
            }
            let after = store.load_neighborhood("UserData").unwrap().unwrap();
            assert!(after.family_in(56).is_none());
            assert_eq!(after.families[&fam_id].house, 0);
            assert_eq!(after.families[&fam_id].architecture_value, 0);
            assert_eq!(after.families[&fam_id].budget, expected_net_worth);
        }
    }

    #[test]
    fn reusing_deleted_guid_purges_stale_visitor_on_older_saved_lot() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db = TempDb::new("reuse-guid-purges-stale-visitor");
        let store = NeighborhoodStore::open(&db.0).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
        let no_frames = Frames::parse("none").unwrap();

        // 1. Create runtime-made character `0x61520029` homed on House 5
        //    (non-member) and save House 5.
        let guid = 0x6152_0029u32;
        let mut house5 = enter_lot(&root, &store, "UserData", 5, &[3], no_frames.clone()).unwrap();
        let template_guid = house5.world.objects[&house5.world.people[0]].guid;
        house5.world.characters.made.insert(
            guid,
            sims_sim::characters::Character {
                template: template_guid,
                file: 901,
                strings: BTreeMap::new(),
            },
        );
        let old_nbr_id = (house5.world.neighbors.slots.len() as i16) + 1;
        let mut data = vec![0i16; sims_sim::neighbors::PERSON_WORDS];
        data[PERSON_HOME] = 5;
        house5
            .world
            .neighbors
            .slots
            .push(Some(sims_sim::neighbors::Neighbor {
                id: old_nbr_id,
                guid,
                revision: 9,
                data,
                relationships: Vec::new(),
                friends: 0,
                stale: false,
            }));
        save_and_leave_lot(&root, &store, "UserData", &mut house5).unwrap();

        // 2. Save House 7 containing a live visitor instance of `0x61520029`
        //    BEFORE House 5 is bulldozed.
        let mut house7 = enter_lot(&root, &store, "UserData", 7, &[3], no_frames.clone()).unwrap();
        let template_id = house7.world.people[0];
        let stale_visitor_id = house7.world.objects.keys().copied().max().unwrap_or(100) + 1;
        let next_type = house7.world.types.keys().copied().max().unwrap_or(0) + 1;
        house7.world.types.insert(next_type, guid);
        let mut visitor = house7.world.objects[&template_id].clone();
        visitor.id = stale_visitor_id;
        visitor.guid = guid;
        visitor.selected = false;
        if let Some(person) = visitor.person.as_mut() {
            person.data[31] = old_nbr_id;
            person.data[PERSON_HOME] = 5;
            person.data[61] = 0;
        }
        house7.world.objects.insert(stale_visitor_id, visitor);
        house7.world.order.push(stale_visitor_id);
        house7.world.people.push(stale_visitor_id);
        save_and_leave_lot(&root, &store, "UserData", &mut house7).unwrap();

        // 3. Bulldoze House 5: deletes `0x61520029` (advancing `deletion_epoch`).
        let dozed = bulldoze(&root, &store, "UserData", 5).unwrap();
        assert!(dozed.deleted_tombstones >= 1);

        // 4. Create a replacement character reusing `0x61520029` in family 2 on
        //    House 4 and save House 4.
        move_in(&root, &store, "UserData", 2, 4).unwrap();
        let mut house4 = enter_lot(&root, &store, "UserData", 4, &[3], no_frames.clone()).unwrap();
        house4.world.characters.made.insert(
            guid,
            sims_sim::characters::Character {
                template: template_guid,
                file: 902,
                strings: BTreeMap::new(),
            },
        );
        let new_nbr_id = (house4.world.neighbors.slots.len() as i16) + 1;
        let mut new_data = vec![0i16; sims_sim::neighbors::PERSON_WORDS];
        new_data[PERSON_HOME] = 4;
        new_data[61] = 2;
        house4
            .world
            .neighbors
            .slots
            .push(Some(sims_sim::neighbors::Neighbor {
                id: new_nbr_id,
                guid,
                revision: 9,
                data: new_data,
                relationships: Vec::new(),
                friends: 0,
                stale: false,
            }));
        save_and_leave_lot(&root, &store, "UserData", &mut house4).unwrap();

        // 5. Reopening older saved House 7 MUST purge the stale pre-bulldoze
        //    visitor `stale_visitor_id` instead of reviving it attached to the
        //    replacement's NBRS record!
        let mut house7_after =
            enter_lot(&root, &store, "UserData", 7, &[3], no_frames.clone()).unwrap();
        assert!(
            !house7_after.world.objects.contains_key(&stale_visitor_id),
            "stale pre-bulldoze visitor must be purged when reopening House 7 after GUID reuse"
        );
        assert!(
            house7_after.world.objects.values().all(|o| o.guid != guid),
            "reused GUID must not revive the deleted visitor on older saved House 7"
        );
        house7_after.step().unwrap();
        // And House 4 (saved at the newer epoch) still opens cleanly!
        let mut house4_after = enter_lot(&root, &store, "UserData", 4, &[3], no_frames).unwrap();
        assert!(house4_after.world.neighbors.first_of(guid).is_some());
        house4_after.step().unwrap();
    }

    #[test]
    fn bulldozing_lot_strips_far_archived_deleted_character_from_imported_base_house() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db = TempDb::new("bulldoze-far-archived-base-instance");
        let store = NeighborhoodStore::open(&db.0).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
        let no_frames = Frames::parse("none").unwrap();

        // Replace Bob Newbie's loose `User00000.iff` with a FAR archive
        // containing `User00000.iff` (so bulldozing House 5 cannot delete the
        // FAR and Bob's OBJD stays in `catalog`).
        let bob_guid = 0x32AA_2056u32;
        let bob_bytes = std::fs::read(root.join("UserData/Characters/User00000.iff")).unwrap();
        let member_name = b"User00000.iff";
        let data_offset = 16u32;
        let manifest_offset = data_offset + bob_bytes.len() as u32;
        let mut far = Vec::new();
        far.extend_from_slice(b"FAR!byAZ");
        far.extend_from_slice(&1u32.to_le_bytes());
        far.extend_from_slice(&manifest_offset.to_le_bytes());
        far.extend_from_slice(&bob_bytes);
        far.extend_from_slice(&1u32.to_le_bytes());
        far.extend_from_slice(&(bob_bytes.len() as u32).to_le_bytes());
        far.extend_from_slice(&(bob_bytes.len() as u32).to_le_bytes());
        far.extend_from_slice(&data_offset.to_le_bytes());
        far.extend_from_slice(&(member_name.len() as u32).to_le_bytes());
        far.extend_from_slice(member_name);

        // Save the FAR into REDB and remove the loose `User00000.iff` key.
        let mut nh = store.load_neighborhood("UserData").unwrap().unwrap();
        nh.families
            .get_mut(&1)
            .unwrap()
            .members
            .retain(|&g| g != bob_guid);
        for nbr in nh.neighbors.iter_mut().flatten() {
            if nbr.guid == bob_guid {
                nbr.person_data[PERSON_HOME] = 5;
            }
        }
        store
            .save_character_file("UserData", "bob.far", &far)
            .unwrap();
        // Remove loose `User00000.iff` via `save_lot_and_neighborhood_checked`
        // (saving House 5? No, remove only the character file via neighborhood
        // store update — wait: just overwrite `User00000.iff`? No, removing it
        // leaves ONLY `bob.far`!).
        let cur = store.load_neighborhood("UserData").unwrap().unwrap();
        let house9_bytes = store.load_base_house("UserData", 9).unwrap().unwrap();
        nh.instance_id = cur.instance_id;
        nh.revision = cur.revision;
        let _ = store
            .save_lot_and_neighborhood_checked(
                "UserData",
                9,
                Some((cur.instance_id, cur.revision)),
                &house9_bytes,
                &nh,
                &["User00000.iff".to_owned()],
            )
            .unwrap()
            .unwrap();

        // Bulldoze House 5: deletes Bob from NBRS and marks him in
        // `deleted_characters`, while `bob.far` remains in REDB & `catalog`.
        let _ = bulldoze(&root, &store, "UserData", 5).unwrap();
        assert!(
            store
                .load_character_files("UserData")
                .unwrap()
                .contains_key("bob.far")
        );

        // Untouched base `House07.iff` (still containing Bob object 105) must
        // strip Bob cleanly even though his OBJD is still in `catalog` via FAR!
        let mut after = enter_lot(&root, &store, "UserData", 7, &[3], no_frames).unwrap();
        assert!(after.world.objects.values().all(|o| o.guid != bob_guid));
        after.step().unwrap();
    }

    #[test]
    fn saving_empty_household_does_not_block_unrelated_lots() {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            return;
        };
        let db = TempDb::new("empty-household-save");
        let store = NeighborhoodStore::open(&db.0).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
        let no_frames = Frames::parse("none").unwrap();

        // Move single-member Bachelor family (id=2) into House 4, clear its
        // sole member (mirroring Generic Sims Call 6 removing the last member),
        // and Save & Leave.
        move_in(&root, &store, "UserData", 2, 4).unwrap();
        let mut house4 = enter_lot(&root, &store, "UserData", 4, &[3], no_frames.clone()).unwrap();
        if let Some(fam2) = house4.world.families.iter_mut().find(|f| f.id == 2) {
            fam2.members.clear();
        }
        save_and_leave_lot(&root, &store, "UserData", &mut house4).unwrap();

        // The empty household must be pruned from stored families, leaving
        // House 4 unoccupied and unrelated Goth House 5 both loadable and
        // bulldozable!
        let nh = store.load_neighborhood("UserData").unwrap().unwrap();
        assert!(!nh.families.contains_key(&2));
        assert!(nh.family_in(4).is_none());

        let mut goth = enter_lot(&root, &store, "UserData", 5, &[3], no_frames.clone()).unwrap();
        goth.step().unwrap();
        let _ = bulldoze(&root, &store, "UserData", 5).unwrap();
        let _reopened4 = enter_lot(&root, &store, "UserData", 4, &[3], no_frames).unwrap();
    }
}
