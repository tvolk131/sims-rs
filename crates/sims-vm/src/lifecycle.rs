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
    if let Some(person) = world
        .objects
        .get_mut(&id)
        .and_then(|obj| obj.person.as_mut())
    {
        person.object_uses.clear();
        person.queue.clear();
    }
    interpreter
        .remove_now(world, id)
        .map_err(|e| LifecycleError::Lot(format!("removing person {id}: {e}")))
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

/// Moves a Family Bin household (`house == 0`) onto an unoccupied residential
/// lot (`zoning == 1`), validating funds against `lot_price` using the
/// install's `FCNS #4` `"architecture value multiplier"`.
pub fn move_in(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    family_id: u16,
    house: u32,
) -> Result<MoveInReport, LifecycleError> {
    let tuning = NeighborhoodTuning::from_install(install_root);
    move_in_with_multiplier(
        store,
        neighborhood_id,
        family_id,
        house,
        tuning.arch_value_multiplier,
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

    let family = stored.families.get_mut(&family_id).expect("checked above");
    // 0x4282ff..0x428308 + 0x42735b..0x427376:
    family.budget = budget_before
        .wrapping_sub(required)
        .wrapping_add(family.architecture_value);
    family.architecture_value = required;
    family.house = i32::try_from(house).map_err(|e| LifecycleError::Lot(e.to_string()))?;
    family.flags = (family.flags & !FAMILY_FLAG_BIT_3) | FAMILY_FLAG_NEW_MOVE_IN;
    let budget_after = family.budget;
    stored.revision = stored.revision.saturating_add(1);

    match store.save_neighborhood_checked(
        neighborhood_id,
        origin_instance_id,
        origin_revision,
        &stored,
    )? {
        Ok(()) => {}
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
    Ok(MoveInReport {
        family: family_id,
        house,
        lot_price: required,
        budget_before,
        budget_after,
        architecture_value: required,
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
        // bulldozed via `0x426fcd`) before re-attaching remaining persons.
        let orphaned_people: Vec<u16> = world
            .people
            .iter()
            .copied()
            .filter(|&person_id| {
                world.objects.get(&person_id).is_some_and(|o| {
                    o.person.is_some()
                        && (stored.deleted_characters.contains(&o.guid)
                            || world.neighbors.first_of(o.guid).is_none())
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
                    .reset_person(&mut world, person_id, &program.people)
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
    } else {
        Answers::choices(answers)
    };
    let play = Play { fresh: saved.fresh };
    Ok(Lot {
        program,
        catalog,
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
        loaded: saved.loaded,
        buy_undo: Vec::new(),
        buy_redo: Vec::new(),
        buy_orphans: Vec::new(),
        neighborhood_origin: Some(NeighborhoodOrigin {
            store_id,
            neighborhood_id: neighborhood_id.to_owned(),
            instance_id: origin_instance_id,
            revision: origin_revision,
        }),
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
    let file = House::parse_bytes(bytes, &virtual_path, &catalog)
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
            let object = catalog.object(entry.guid).ok_or_else(|| {
                LifecycleError::Lot(format!("type {} has no definition", entry.id))
            })?;
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
        world
            .select_family(
                i16::try_from(family_id).map_err(|e| LifecycleError::Lot(e.to_string()))?,
                i16::try_from(house).map_err(|e| LifecycleError::Lot(e.to_string()))?,
                family_budget,
                family_magic,
            )
            .map_err(|e| LifecycleError::Lot(e.to_string()))?;

        load.to_visit = world.order.len();
        let visited = interpreter.visit_objects(&mut world, &people);
        load.visited = visited.objects.len();
        load.reset = visited.reset.len();
        load.stopped = visited.stopped.map(|fault| fault.to_string());
        if let Some(reason) = &load.stopped {
            return Err(LifecycleError::Lot(format!(
                "house {house} load did not finish: {reason}"
            )));
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
        catalog,
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
        loaded,
        buy_undo: Vec::new(),
        buy_redo: Vec::new(),
        buy_orphans: Vec::new(),
        neighborhood_origin: Some(NeighborhoodOrigin {
            store_id,
            neighborhood_id: neighborhood_id.to_owned(),
            instance_id: origin_instance_id,
            revision: origin_revision,
        }),
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
    let lot_bytes = saved.encode();
    match store.save_lot_and_neighborhood_checked(
        neighborhood_id,
        house,
        Some((origin.instance_id, origin.revision)),
        &lot_bytes,
        &stored,
        &[],
    )? {
        Ok(()) => {}
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
    let (committed_instance, committed_revision) = match origin.revision.checked_add(1) {
        Some(next_rev) => (origin.instance_id, next_rev),
        None => store
            .load_neighborhood(neighborhood_id)?
            .map_or((stored.instance_id, 0), |nh| (nh.instance_id, nh.revision)),
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
    let report = evict_or_bulldoze(install_root, store, neighborhood_id, house, false)?;
    let family = report
        .evicted_family
        .ok_or(LifecycleError::LotUnoccupied { house })?;
    let stored = store
        .load_neighborhood(neighborhood_id)?
        .ok_or_else(|| LifecycleError::NeighborhoodNotFound(neighborhood_id.to_owned()))?;
    let budget_after = stored.families.get(&family).map_or(0, |f| f.budget);
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
    evict_or_bulldoze(install_root, store, neighborhood_id, house, true)
}

fn evict_or_bulldoze(
    install_root: &Path,
    store: &NeighborhoodStore,
    neighborhood_id: &str,
    house: u32,
    bulldoze: bool,
) -> Result<BulldozeReport, LifecycleError> {
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

    // 0x42749a..0x427536: remove people first (so no lingering routes/uses
    // block furniture removal), then removable objects.
    let mut removed_objects = 0usize;
    let people_ids: Vec<u16> = lot.world.people.clone();
    for id in people_ids {
        if !lot.world.objects.contains_key(&id) || lot.world.indicators.contains(&id) {
            continue;
        }
        let in_container = crate::placement::capture(&mut lot.world, id)
            .map(|c| c.container.is_some())
            .unwrap_or(false);
        if !in_container
            && should_remove_on_evict_or_bulldoze(&interpreter, &mut lot.world, id, bulldoze)
        {
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
        for (id, guid) in tombstones {
            lot.world.neighbors.remove_record(id);
            lot.world.characters.made.remove(&guid);
            stored.made_characters.made.remove(&guid);
            stored.deleted_characters.insert(guid);
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
        }
    }

    // Write back updated NBRS / NGBH / TATT / FAMI, delete removed tombstone
    // character files, and save the cleared lot atomically into REDB.
    stored.write_back_world(&lot.world, house, 0, 0, None, tuning.friendship_threshold);
    let saved = lot
        .save()
        .map_err(|e| LifecycleError::Lot(format!("saving cleared lot {house}: {e}")))?;
    let lot_bytes = saved.encode();
    match store.save_lot_and_neighborhood_checked(
        neighborhood_id,
        house,
        Some((origin_instance_id, origin_revision)),
        &lot_bytes,
        &stored,
        &removed_character_files,
    )? {
        Ok(()) => {}
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

    Ok(BulldozeReport {
        house,
        evicted_family,
        refund,
        removed_objects,
        cleared_tiles,
        deleted_tombstones,
        remaining_objects_value,
        remaining_architecture_value,
    })
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

        // 3. Stage the resident `template_id` actively executing BHAV 4096
        //    owned by `next_type` (`0x7757D9EE`) targeting `visitor_ground_id`
        //    so re-entry must reset the resident's action & frame cleanly.
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
            }
            house7.world.frame_storage.remove(&template_id);
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
        // grid cell, and step cleanly!
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
        for _ in 0..3 {
            house7_after.step().unwrap();
        }
        save_and_leave_lot(&root, &store, "UserData", &mut house7_after).unwrap();
    }
}
