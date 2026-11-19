//! The state-only steps of loading a lot: the globals the running game
//! supplies, the lot arrays, the object grid's repair, discarding the
//! removal queue, and selecting the household. The lot's rooms are in
//! [`crate::rooms`]. The load sequence in `docs/roadmap.md`
//! lists every step of the original's lot load in order; the ones that run
//! scripts live in `sims-vm`.

use crate::{FULL_SIZE, MagicHistory, Object, World};
use serde::{Deserialize, Serialize};
use sims_content::{SurfaceCatalog, Surfaces};
use sims_formats::lot::{Array, SurfaceMapping};
use sims_formats::tatt::Tatt;
use std::collections::{BTreeMap, BTreeSet};

/// What the running game, rather than the house file, supplies to a lot
/// load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// Global 15: the game-context mode byte, sign-extended.
    pub mode: i8,
    /// Global 25, kept from before the load.
    pub global25: i16,
    /// Global 30: free will.
    pub free_will: u8,
    /// Global 20: the installed-expansions mask.
    pub expansions: u16,
    /// Global 33: the hardware performance class, 0 to 2, which the original
    /// picks from memory size and CPU speed (0x413acc).
    pub performance_class: i16,
}

impl Context {
    /// The values the original has when it loads a lot from the
    /// neighborhood on a Complete Collection install: mode 1, global 25
    /// zero, free will on, every expansion, top performance class.
    ///
    /// A capture of the original, taken by the old prototype's harness as
    /// it began to load a house whose saved global 15 is 0, has global 15
    /// at 1, global 20 at 0xff and global 33 at 2 before any script has
    /// run. The mode is 1 on that evidence; the old prototype's headless
    /// profile had 0. Global 25 and free will are **unverified**: the
    /// capture agrees with them but the house saved the same values. The
    /// performance class comes from the machine's memory and CPU speed, so
    /// a comparison harness must read it from the running original.
    pub const COMPLETE_COLLECTION: Self = Self {
        mode: 1,
        global25: 0,
        free_will: 1,
        expansions: 0xff,
        performance_class: 2,
    };
}

/// Errors from the load steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The neighborhood's records cannot be read as the original reads
    /// them, or reading them needs something not ported.
    Neighborhood(String),
    /// The saved globals are neither 32 nor 64 words.
    GlobalCount(usize),
    /// A step that needs all 64 globals ran on fewer, so before
    /// [`World::apply_context`].
    NoContext,
    /// The house file lacks a lot array the load reads.
    MissingArray(u16),
    /// A saved lot array is not 64 by 64 cells.
    ArraySize {
        array: u16,
        width: usize,
        height: usize,
    },
    /// A lot array whose cells do not have the size its step works on.
    ArrayStride { array: u16, stride: usize },
    /// A house version whose array layout is not ported.
    ArrayVersion(u32),
    /// A saved floor or wall ID of the house's maps that no pattern file
    /// gives a key: the original then takes the pattern of the nearest
    /// colour (0x4a0bbd), which is not ported. Also an ID of 65535, which
    /// is past the original's table.
    SurfaceColour { name: String, id: u16 },
    /// The renumbering would write this wall cell back, and the write needs
    /// a style the cell does not have (the original writes whatever its
    /// stack held), or the cell's lowest segment is above 0x20. Neither is
    /// ported.
    SurfaceRewrite { level: u8, x: usize, y: usize },
    /// The end of the household selection would do something that is not
    /// ported. See [`World::finish_selection`].
    Selection(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Neighborhood(message) => write!(f, "neighborhood: {message}"),
            Self::GlobalCount(n) => write!(f, "{n} saved globals; expected 32 or 64"),
            Self::NoContext => write!(f, "the load context has not been applied"),
            Self::MissingArray(array) => write!(f, "the house has no lot array {array}"),
            Self::ArraySize {
                array,
                width,
                height,
            } => write!(
                f,
                "lot array {array} is {width} by {height} cells; only {FULL_SIZE} by \
                 {FULL_SIZE} is supported"
            ),
            Self::ArrayStride { array, stride } => {
                write!(f, "lot array {array} has cells of {stride} bytes")
            }
            Self::ArrayVersion(version) => write!(
                f,
                "house version {version}: the array layout before version 53 is not ported"
            ),
            Self::SurfaceColour { name, id } => write!(
                f,
                "no pattern file gives saved ID {id} of {name:?} a key; matching it by colour \
                 (0x4a0bbd) is not ported"
            ),
            Self::Selection(what) => write!(f, "{what}, which is not ported"),
            Self::SurfaceRewrite { level, x, y } => write!(
                f,
                "renumbering would write back the wall cell at {x}, {y} on level {level} in a \
                 way that is not ported"
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// The lot arrays a load reads from the house file, by Arry chunk ID, in
/// the order the original asks for them (0x478599) when the house's
/// version is 53 to 62. The order is from the old prototype's reading of
/// the original's code (legacy/docs/research/
/// retained-saved-source-prefix-audit-20261026.md), not from a trace of the
/// reads. From version 63 the floors and walls are read from other IDs
/// ([`WIDE_SURFACES`]). Before 53 some arrays are made instead of read,
/// which is not ported.
pub const SAVED_ARRAYS: [u16; 14] = [0, 4, 1, 2, 3, 8, 101, 102, 103, 108, 6, 7, 9, 10];

/// From version 63 (0x47861d to 0x478652), the Arry chunk each floor and
/// wall array of the lot is read from, as the lot holds it (16-bit floor
/// IDs, 14-byte wall cells): the level's base (0 or 100) plus 11 for the
/// floors and 12 for the walls, in place of 1 and 2. Pairs of the
/// lot's array ID and the chunk ID.
pub const WIDE_SURFACES: [(u16, u16); 4] = [(1, 11), (2, 12), (101, 111), (102, 112)];

/// The last two of [`SAVED_ARRAYS`]: the original ignores whether reading
/// them worked.
const OPTIONAL_ARRAYS: [u16; 2] = [9, 10];

/// The floor arrays, ground level then upper level. A lot holds a floor
/// as a 16-bit pattern ID; these house versions save it as one byte.
pub const FLOORS: [u16; 2] = [1, 101];

/// The wall arrays, ground level then upper level. A lot holds a wall cell
/// as 14 bytes: the segments it has, a flags byte, and six 16-bit words
/// (two styles, four patterns). These house versions save the words as
/// one byte each, 8 bytes in all.
pub const WALLS: [u16; 2] = [2, 102];

/// The cell sizes of a lot's floor and wall arrays, and of saved ones.
const FLOOR_CELL: usize = 2;
const WALL_CELL: usize = 14;
const SAVED_WALL_CELL: usize = 8;

/// A ground-level wall cell of a lot nothing was built on: no segments,
/// flags 0x0f, and both styles 1. The upper level's bare cell is all
/// zeros, and so is a bare floor cell.
///
/// From a trace of the original loading the Goth house: every cell of the
/// held lot's ground wall array has these bytes when the load starts, and
/// the cells outside the lot's 52 tiles still have them when it ends.
pub const BARE_GROUND_WALL: [u8; WALL_CELL] = [0, 0x0f, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// Whether the renumbering looks a saved ID up (0x4a0e3b, 0x4c5d35).
/// Floors 0 to 29 and walls 0 to 30 are built in and keep their IDs, as do
/// 255 (a floor cut by a diagonal wall), wall IDs 248 to 255 and 65535.
fn renumbered(id: u16, wall: bool) -> bool {
    let low = if wall { 31..=247 } else { 30..=254 };
    low.contains(&id) || (256..=65534).contains(&id)
}

/// The table from saved ID to catalog key that one of a house's maps gives
/// (0x4a0f00 to 0x4a0fca; the wall step 0x4c5d35 builds its own the same
/// way): 65535 entries, each 65535 until a group lists it.
fn surface_table(map: &[SurfaceMapping], catalog: &SurfaceCatalog) -> Result<Vec<u16>, LoadError> {
    let mut table = vec![u16::MAX; usize::from(u16::MAX)];
    for group in map {
        // 0 when no file has the name: file 0 is never looked in.
        let file = catalog.file(&group.name).unwrap_or(0);
        for entry in &group.entries {
            let slot =
                table
                    .get_mut(usize::from(entry.id))
                    .ok_or_else(|| LoadError::SurfaceColour {
                        name: group.name.clone(),
                        id: entry.id,
                    })?;
            if file != 0 {
                // Written even when the file lacks the pattern, over what
                // an earlier group gave the same ID.
                *slot = catalog.key(file, entry.local).unwrap_or(u16::MAX);
            }
            if *slot == u16::MAX {
                return Err(LoadError::SurfaceColour {
                    name: group.name.clone(),
                    id: entry.id,
                });
            }
        }
    }
    Ok(table)
}

/// The patterns the wall step renumbers in a wall cell (0x4c5d35), as
/// indices into [`ExpandedWall::patterns`], or None for a cell whose lowest
/// segment is above the last one, 0x20, which is not ported. The step
/// starts at the lowest segment and stops once it has moved on to 0x20, so
/// 0x20's patterns count only when it is the lowest.
fn wall_patterns(segments: u8) -> Option<Vec<usize>> {
    let lowest = segments & segments.wrapping_neg();
    if lowest > 0x20 {
        return None;
    }
    let mut patterns = Vec::new();
    let mut segment = lowest;
    while segment != 0 && segment < 0x40 {
        patterns.extend_from_slice(match segment {
            1 => &[0][..],
            2 => &[1],
            4 => &[2],
            8 => &[3],
            0x10 => &[4, 5],
            _ => &[6, 7],
        });
        segment <<= 1;
        while segment < 0x20 && segments & segment == 0 {
            segment <<= 1;
        }
        if segment >= 0x20 {
            break;
        }
    }
    Some(patterns)
}

/// A wall cell as the original's wall service hands it out (0x4795c5) and
/// takes it back (0x4797e2), for the cell at x, y of a level's wall array.
///
/// A lot keeps a wall cell in 14 bytes: the segments, a flags byte, then
/// six words: the west and north styles (2, 4) and the west, north, south
/// and east patterns (6, 8, 10, 12); a diagonal keeps its two patterns in
/// 10 and 12. A tile owns its west and north walls; its east and south
/// ones are the neighbours' west and north.
///
/// Ported from the old prototype's native_wall_io.rs (rotation 0), which
/// was compared with the original's code run in isolation. The service
/// also writes four bytes of a vertex map beside the walls (0x479c58),
/// which only the wall drawing reads and which is not kept here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedWall {
    pub segments: u8,
    /// The cell's flags, with bits 4 and 5 the east neighbour's 0 and 1,
    /// and bits 6 and 7 the south neighbour's 2 and 3, when they own them.
    pub flags: u8,
    /// West, north, east, south, then north twice. East and south are None
    /// when the neighbour does not own the wall: the original leaves them
    /// as whatever its stack held.
    pub styles: [Option<u16>; 6],
    /// West, north, east, south, then the descending diagonal's two and
    /// the ascending diagonal's two (0 when the cell lacks them).
    pub patterns: [u16; 8],
}

impl ExpandedWall {
    fn at(x: usize, y: usize) -> usize {
        (x * FULL_SIZE as usize + y) * WALL_CELL
    }

    fn word(cells: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([cells[at], cells[at + 1]])
    }

    fn set(cells: &mut [u8], at: usize, value: u16) {
        cells[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// 0x4795c5. A neighbour to the east or south counts only inside the
    /// lot's size `active`.
    pub fn get(cells: &[u8], active: usize, x: usize, y: usize) -> Self {
        let c = Self::at(x, y);
        let word = |at| Self::word(cells, at);
        let segments = cells[c];
        let mut flags = cells[c + 1];
        let mut styles = [
            Some(word(c + 2)),
            Some(word(c + 4)),
            None,
            None,
            Some(word(c + 4)),
            Some(word(c + 4)),
        ];
        let diagonal = |bit: u8| {
            if segments & bit != 0 {
                [word(c + 10), word(c + 12)]
            } else {
                [0, 0]
            }
        };
        let [a, b] = diagonal(0x10);
        let [d, e] = diagonal(0x20);
        let patterns = [
            word(c + 6),
            word(c + 8),
            word(c + 12),
            word(c + 10),
            a,
            b,
            d,
            e,
        ];
        if y + 1 < active {
            let south = Self::at(x, y + 1);
            if cells[south] & 2 != 0 {
                styles[3] = Some(word(south + 4));
                flags = (flags & !0xc0) | ((cells[south + 1] & 12) << 4);
            }
        }
        if x + 1 < active {
            let east = Self::at(x + 1, y);
            if cells[east] & 1 != 0 {
                styles[2] = Some(word(east + 2));
                flags = (flags & !0x30) | ((cells[east + 1] & 3) << 4);
            }
        }
        Self {
            segments,
            flags,
            styles,
            patterns,
        }
    }

    /// 0x4797e2: writes the cell and its neighbours back, in the
    /// original's order: the diagonals, then west, north, east and south.
    /// A segment the cell lacks is cleared, with its neighbour's side, only
    /// when the cell still has it. None, with the cells partly written,
    /// when a write needs an east or south style the cell does not have.
    pub fn put(&self, cells: &mut [u8], active: usize, x: usize, y: usize) -> Option<()> {
        let c = Self::at(x, y);
        let mask = self.segments & 0x3f;
        let p = self.patterns;
        for (bit, style, [first, second]) in [(0x10u8, 4, [4, 5]), (0x20, 5, [6, 7])] {
            if mask & bit != 0 {
                cells[c] = bit;
                Self::set(cells, c + 4, self.styles[style]?);
                Self::set(cells, c + 10, p[first]);
                Self::set(cells, c + 12, p[second]);
                Self::set(cells, c + 2, self.styles[0]?);
                Self::set(cells, c + 6, p[0]);
            } else {
                cells[c] &= !bit;
            }
        }
        if mask & 1 != 0 {
            cells[c] |= 1;
            Self::set(cells, c + 2, self.styles[0]?);
            Self::set(cells, c + 6, p[0]);
            cells[c + 1] |= self.flags & 3;
            if x > 0 {
                cells[Self::at(x - 1, y)] |= 4;
            }
        } else if cells[c] & 1 != 0 {
            cells[c] &= !1;
            if x > 0 {
                let west = Self::at(x - 1, y);
                cells[west] &= !4;
                Self::set(cells, west + 12, 0);
            }
        }
        if mask & 2 != 0 {
            cells[c] |= 2;
            Self::set(cells, c + 4, self.styles[1]?);
            Self::set(cells, c + 8, p[1]);
            cells[c + 1] |= self.flags & 12;
            if y > 0 {
                cells[Self::at(x, y - 1)] |= 8;
            }
        } else if cells[c] & 2 != 0 {
            cells[c] &= !2;
            if y > 0 {
                let north = Self::at(x, y - 1);
                cells[north] &= !8;
                Self::set(cells, north + 10, 0);
            }
        }
        if mask & 4 != 0 {
            cells[c] |= 4;
            Self::set(cells, c + 12, p[2]);
            if x + 1 < active {
                let east = Self::at(x + 1, y);
                cells[east] |= 1;
                Self::set(cells, east + 2, self.styles[2]?);
                cells[east + 1] |= (self.flags & 0x30) >> 4;
            }
        } else if cells[c] & 4 != 0 {
            cells[c] &= !4;
            if x + 1 < active {
                let east = Self::at(x + 1, y);
                cells[east] &= !1;
                Self::set(cells, east + 6, 0);
            }
        }
        if mask & 8 != 0 {
            cells[c] |= 8;
            Self::set(cells, c + 10, p[3]);
            if y + 1 < active {
                let south = Self::at(x, y + 1);
                cells[south] |= 2;
                Self::set(cells, south + 4, self.styles[3]?);
                cells[south + 1] |= (self.flags & 0xc0) >> 4;
            }
        } else if cells[c] & 8 != 0 {
            cells[c] &= !8;
            if y + 1 < active {
                let south = Self::at(x, y + 1);
                cells[south] &= !2;
                Self::set(cells, south + 8, 0);
            }
        }
        Some(())
    }
}

/// The global that holds the budget mode. Funds change only while it is 0.
const BUDGET_MODE: usize = 32;
/// Person data word 32 (+0x5cc): whether the person is a visitor.
const VISITOR: usize = 32;
/// Person data word 61 (+0x606): the person's family.
const FAMILY: usize = 61;
/// Person data word 67 (+0x612), which the household selection compares
/// with the house number.
const HOME: usize = 67;

/// One person's word 32 after the visitor pass (0x42a2b4), from its word
/// 32 before it: a 0 becomes 1, a member of the selected family (its type
/// is one of the family's members, 0x41019b) gets 0, and on a lot that is
/// not residential (0x40745f) a 0 becomes 1 again. Any other value stays.
pub fn visitor_again(word: i16, member: bool, residential: bool) -> i16 {
    let mut visitor = if word == 0 { 1 } else { word };
    if member {
        visitor = 0;
    }
    if !residential && visitor == 0 {
        visitor = 1;
    }
    visitor
}

/// What the end of the household selection reads besides the lot. See
/// [`World::finish_selection`].
#[derive(Debug, Clone, Copy)]
pub struct Household<'a> {
    /// The house number the selection was given (0x428265, stored at
    /// +0x164 of the neighborhood).
    pub house: i32,
    /// The selected family record's flags (+0x138).
    pub flags: u32,
    /// The GUIDs of the family's members (+0x12c).
    pub members: &'a [u32],
    /// The GUIDs of the types registered from the neighborhood's
    /// `Characters` folder (kind 1: bits 2 and 3 of the type record's
    /// +0xb4 hold 1, 0x43bd19).
    pub characters: &'a BTreeSet<u32>,
    /// The people who were residents before the family was selected: see
    /// [`World::residents`].
    pub residents: &'a BTreeSet<u16>,
    /// The neighborhood's zoning by house number. The lot is residential
    /// when global 10, as it stands when the step runs, is 0 or a house
    /// zoned 1 (0x40745f, 0x42b9e3). An object's Load or Init tree may
    /// have changed global 10 since the house was chosen.
    pub zoning: &'a BTreeMap<i16, i32>,
}

/// The ground level's object grid: for each tile, the ID of the first
/// object standing on it, or 0. Each object names the next one on its tile
/// in its slot 0.
pub const GROUND_GRID: u16 = 3;

/// A grid cell [`World::repair_grid`] cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridRepair {
    /// The cell: the grid's column, then the place in the column.
    pub tile: [usize; 2],
    /// The ID the cell held.
    pub object: i16,
    /// The tile and level of the object with that ID, or None if the lot
    /// has no such object.
    pub found: Option<[i8; 3]>,
}

impl World {
    /// Turns the saved globals into the ones the lot runs with. The
    /// simulator's setup zeroes all 64 globals (0x46cdc6, 0x46ceb9), the SIMI
    /// reader overlays the saved words and refreshes the running fields
    /// (0x46cfb3), and the house loader then restores the installation's
    /// globals and clears the budget mode (0x415eff). The saved family in
    /// global 9 stays until [`World::clear_family`].
    ///
    /// Call it once, on the globals [`World::read_simulator`] read.
    pub fn apply_context(&mut self, context: Context) -> Result<(), LoadError> {
        let saved = &self.globals;
        if !matches!(saved.len(), 32 | 64) {
            return Err(LoadError::GlobalCount(saved.len()));
        }
        let mut globals = [0i16; 64];
        globals[..saved.len()].copy_from_slice(saved);
        // 0x46cfb3.
        globals[15] = i16::from(context.mode);
        globals[25] = context.global25;
        globals[29] = 30;
        globals[30] = i16::from(context.free_will);
        globals[16] = globals[16].clamp(0, 1000);
        // The speed setter updates global 18 only while it is not negative.
        if globals[18] >= 0 {
            globals[18] = globals[16];
        }
        // 0x415eff.
        globals[20] = context.expansions as i16;
        globals[33] = context.performance_class;
        globals[BUDGET_MODE] = 0;
        self.globals = globals.to_vec();
        Ok(())
    }

    /// Reads the neighborhood's saved type attributes (TATT) into the
    /// records the catalog made (0x43e0f0). Each saved row goes to the
    /// first record with its GUID, and the rest of that record is zeroed. A
    /// row for a GUID with no record is skipped, as long as it has at most
    /// 128 words. A row with more words than its record holds, or more
    /// than 128 with no record, ends the read: it and every later row
    /// change nothing.
    ///
    /// Ported from the old prototype's `State::load`
    /// (legacy/crates/sims-vm/src/resource_storage.rs).
    pub fn read_type_attributes(&mut self, saved: &Tatt) {
        for row in &saved.entries {
            let record = self.type_attributes.iter_mut().find(|r| r.guid == row.guid);
            let capacity = record.as_ref().map_or(128, |r| r.values.len());
            if row.values.len() > capacity {
                return;
            }
            if let Some(record) = record {
                record.values[..row.values.len()].copy_from_slice(&row.values);
                record.values[row.values.len()..].fill(0);
            }
        }
    }

    /// Reads a house's lot arrays onto the lot (0x478599): each array in
    /// [`SAVED_ARRAYS`] replaces the lot's array of that ID. Arrays 9 and 10
    /// may be absent, and then the lot keeps its own. Any other array in
    /// the file is not read. On an error the lot is unchanged.
    ///
    /// `version` is the house's version (HOUS 1). In versions 53 to 62 the
    /// floors and walls are saved narrow and the original widens them into
    /// the lot's own arrays, for the cells inside the lot's size only
    /// (0x478599): a floor byte becomes a 16-bit ID, and a wall's six
    /// bytes after the first two become six 16-bit words. Cells outside
    /// the lot's size keep what the lot's array held. The original's lot
    /// always has these arrays. A lot here may have none, and is then
    /// taken to hold what the original's bare lot holds: zeros, and
    /// [`BARE_GROUND_WALL`] in the ground level's walls.
    ///
    /// For the Goth house the cells inside the lot's size of arrays 0, 1,
    /// 2, 3, 8, 101, 102, 103, 108, 9 and 10 were compared with a capture
    /// of the original taken after this step and the renumbering; see
    /// `docs/decisions/`. Outside the size, the same arrays were
    /// compared with the memory a later trace holds, at the start of load
    /// step 10 and at the end of the load: every cell of both levels'
    /// walls and flags, the ground level's object grid, the pool and the
    /// water, and of the others the cells the trace holds, was equal.
    /// Arrays 4, 6 and 7 are **unverified**.
    ///
    /// From version 63 the floors and walls are saved as the lot holds
    /// them, and the original reads each into the lot's array whole, as it
    /// reads every other array (0x431005 takes the chunk's size and cell
    /// size, then copies all of it): the lot's floors from Arry 11 and
    /// 111, its walls from 12 and 112 ([`WIDE_SURFACES`]). Arrays 1, 2, 101
    /// and 102 of such a file are not read. A floor whose cells are not 2
    /// bytes, or a wall whose cells are not 14, is an error here; the
    /// original would take it, and what it then does with one is not
    /// known. No later version changes the layout: the loader's other
    /// version tests all fall below 53.
    ///
    /// Two things differ from the original. It stops at the first array
    /// that fails to read and goes on with the load; here a missing array
    /// is an error. And every array must be 64 by 64: the original takes
    /// any size (0x431005, whose resize at 0x4312ff to 0x431388 has no such
    /// requirement), but every shipped house's arrays are 64 by 64, whatever
    /// the lot's size. The lot's lighting, which the original
    /// resets here, is not modeled.
    ///
    /// The widening follows the old prototype's reading of the code
    /// (legacy/docs/research/saved-world-array-contract-20261026.md).
    pub fn read_arrays(
        &mut self,
        saved: &BTreeMap<u16, Array>,
        version: u32,
    ) -> Result<(), LoadError> {
        if version < 53 {
            return Err(LoadError::ArrayVersion(version));
        }
        let full = FULL_SIZE as usize;
        let active = usize::try_from(self.size).unwrap_or(0).min(full);
        let wide = version >= 63;
        let mut read = Vec::new();
        for id in SAVED_ARRAYS {
            let surface = WIDE_SURFACES.iter().find(|&&(lot, _)| lot == id);
            if let (true, Some(&(lot, chunk))) = (wide, surface) {
                let array = saved.get(&chunk).ok_or(LoadError::MissingArray(chunk))?;
                if array.width != full || array.height != full {
                    return Err(LoadError::ArraySize {
                        array: chunk,
                        width: array.width,
                        height: array.height,
                    });
                }
                let cell = if FLOORS.contains(&lot) {
                    FLOOR_CELL
                } else {
                    WALL_CELL
                };
                if array.stride != cell || array.data.len() != full * full * cell {
                    return Err(LoadError::ArrayStride {
                        array: chunk,
                        stride: array.stride,
                    });
                }
                read.push((lot, array.clone()));
                continue;
            }
            let Some(array) = saved.get(&id) else {
                if OPTIONAL_ARRAYS.contains(&id) {
                    continue;
                }
                return Err(LoadError::MissingArray(id));
            };
            if array.width != full || array.height != full {
                return Err(LoadError::ArraySize {
                    array: id,
                    width: array.width,
                    height: array.height,
                });
            }
            let (from, to) = if FLOORS.contains(&id) {
                (1, FLOOR_CELL)
            } else if WALLS.contains(&id) {
                (SAVED_WALL_CELL, WALL_CELL)
            } else {
                read.push((id, array.clone()));
                continue;
            };
            if array.stride != from || array.data.len() != full * full * from {
                return Err(LoadError::ArrayStride {
                    array: id,
                    stride: array.stride,
                });
            }
            // The lot's own array, which keeps its cells outside the size.
            let mut data = match self.arrays.get(&id) {
                Some(held) if held.width == full && held.height == full && held.stride == to => {
                    held.data.clone()
                }
                Some(held) => {
                    return Err(LoadError::ArrayStride {
                        array: id,
                        stride: held.stride,
                    });
                }
                None if id == WALLS[0] => BARE_GROUND_WALL.repeat(full * full),
                None => vec![0; full * full * to],
            };
            for x in 0..active {
                for y in 0..active {
                    let cell = x * full + y;
                    let narrow = &array.data[cell * from..(cell + 1) * from];
                    let wide = &mut data[cell * to..(cell + 1) * to];
                    if from == 1 {
                        wide.copy_from_slice(&u16::from(narrow[0]).to_le_bytes());
                    } else {
                        wide[..2].copy_from_slice(&narrow[..2]);
                        let words = wide[2..].as_chunks_mut::<2>().0;
                        for (word, &byte) in words.iter_mut().zip(&narrow[2..]) {
                            *word = u16::from(byte).to_le_bytes();
                        }
                    }
                }
            }
            read.push((
                id,
                Array {
                    width: full,
                    height: full,
                    stride: to,
                    encoding: 0,
                    data,
                },
            ));
        }
        self.arrays.extend(read);
        Ok(())
    }

    /// Renumbers the saved floor and wall pattern IDs (0x4a0e3b, then
    /// 0x4c5d35). A house names the patterns it uses beyond the built-in
    /// ones by IDs of its own, and lists in two maps (FLRm, WALm) which
    /// pattern file and which pattern in it each ID means. The original
    /// turns each map into a table from saved ID to the key the pattern has
    /// in its catalog ([`Surfaces`]), then rewrites every cell inside the
    /// lot's size, level 1 then 2, row by row: floors first, then walls.
    ///
    /// `floors` and `walls` are the house's maps, or None for a map the
    /// file lacks; the original then leaves that kind alone. Returns how
    /// many cells changed: floor cells, plus wall cells written back. Call
    /// it after [`World::read_arrays`]. On an error the lot is unchanged.
    ///
    /// **The table** (0x4a0f00 to 0x4a0fca). Every saved ID starts out as
    /// 65535. For each group of a map, the first catalog file whose name is
    /// the group's, ignoring case, gives each entry's ID the key of the
    /// pattern with the entry's number, or 65535 when the file has none.
    /// File 0, the built-in one, is never looked in. An ID still at 65535
    /// then takes the pattern nearest the entry's colour (0x4a0bbd), or 1;
    /// that match is not ported and fails ([`LoadError::SurfaceColour`]).
    ///
    /// **The cells.** An ID [`renumbered`] covers becomes its table entry,
    /// so an ID no group lists becomes 65535. A floor cell holds its ID; a
    /// floor of 255 is cut by a diagonal wall, and its two halves are the
    /// first style and first pattern words of the wall cell when the wall
    /// cell's lowest segment is a diagonal. A wall cell holds a pattern per
    /// segment ([`wall_patterns`]). A wall cell that changes is written back
    /// through the original's wall service (0x4795c5, 0x4797e2; see
    /// [`ExpandedWall`]), which also writes its neighbours.
    ///
    /// The cell walk is the old prototype's port
    /// (legacy/crates/sims-sim/src/surface_remap.rs), which was compared with
    /// the original's code run in isolation on 24 made-up lots
    /// (legacy/docs/research/surface-remap-native-20261026.md); so is the
    /// wall service (native_wall_io.rs, 33 cases). The Goth house's maps
    /// are empty and none of its cells holds an ID the renumbering covers,
    /// and a capture of the original after this step agrees.
    pub fn renumber_surfaces(
        &mut self,
        floors: Option<&[SurfaceMapping]>,
        walls: Option<&[SurfaceMapping]>,
        surfaces: &Surfaces,
    ) -> Result<usize, LoadError> {
        let full = FULL_SIZE as usize;
        let active = usize::try_from(self.size).unwrap_or(0).min(full);
        let array = |id: u16, stride: usize| -> Result<Vec<u8>, LoadError> {
            let array = self.arrays.get(&id).ok_or(LoadError::MissingArray(id))?;
            if array.stride != stride || array.data.len() != full * full * stride {
                return Err(LoadError::ArrayStride {
                    array: id,
                    stride: array.stride,
                });
            }
            Ok(array.data.clone())
        };
        let mut floor_cells = [array(FLOORS[0], FLOOR_CELL)?, array(FLOORS[1], FLOOR_CELL)?];
        let mut wall_cells = [array(WALLS[0], WALL_CELL)?, array(WALLS[1], WALL_CELL)?];
        let mut changed = 0;
        if let Some(map) = floors {
            let table = surface_table(map, &surfaces.floors)?;
            for (level, (floor, wall)) in floor_cells.iter_mut().zip(&mut wall_cells).enumerate() {
                for y in 0..active {
                    for x in 0..active {
                        let at = (x * full + y) * FLOOR_CELL;
                        let id = u16::from_le_bytes([floor[at], floor[at + 1]]);
                        if renumbered(id, false) {
                            let new = table[usize::from(id)];
                            if new != id {
                                floor[at..at + 2].copy_from_slice(&new.to_le_bytes());
                                changed += 1;
                            }
                        } else if id == 255 {
                            let stop = LoadError::SurfaceRewrite {
                                level: level as u8 + 1,
                                x,
                                y,
                            };
                            let mut cell = ExpandedWall::get(wall, active, x, y);
                            let lowest = cell.segments & cell.segments.wrapping_neg();
                            if matches!(lowest, 0x10 | 0x20) {
                                // The two halves: the first style and the
                                // first pattern word.
                                let halves =
                                    [cell.styles[0].ok_or(stop.clone())?, cell.patterns[0]];
                                let [style, pattern] = halves.map(|id| {
                                    if renumbered(id, false) {
                                        table[usize::from(id)]
                                    } else {
                                        id
                                    }
                                });
                                if [style, pattern] != halves {
                                    cell.styles[0] = Some(style);
                                    cell.patterns[0] = pattern;
                                    cell.put(wall, active, x, y).ok_or(stop)?;
                                    changed += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(map) = walls {
            let table = surface_table(map, &surfaces.walls)?;
            for (level, wall) in wall_cells.iter_mut().enumerate() {
                for y in 0..active {
                    for x in 0..active {
                        let stop = LoadError::SurfaceRewrite {
                            level: level as u8 + 1,
                            x,
                            y,
                        };
                        let mut cell = ExpandedWall::get(wall, active, x, y);
                        let mut any = false;
                        for at in wall_patterns(cell.segments).ok_or(stop.clone())? {
                            let id = cell.patterns[at];
                            if renumbered(id, true) && table[usize::from(id)] != id {
                                cell.patterns[at] = table[usize::from(id)];
                                any = true;
                            }
                        }
                        if any {
                            cell.put(wall, active, x, y).ok_or(stop)?;
                            changed += 1;
                        }
                    }
                }
            }
        }
        for (ids, cells) in [(FLOORS, floor_cells), (WALLS, wall_cells)] {
            for (id, data) in ids.into_iter().zip(cells) {
                if let Some(array) = self.arrays.get_mut(&id) {
                    array.data = data;
                }
            }
        }
        Ok(changed)
    }

    /// The tile an object stands on and its level, as the original keeps
    /// them beside the position (object bytes 0xfc to 0xfe, 0x4341c7): the
    /// two coordinates divided by 16, rounding down, and the level, each
    /// cut to a signed byte. An object off the lot is at -1, -1.
    ///
    /// For the Goth house these were compared with the bytes in a capture
    /// of the original, for all 382 objects after load step 9.
    pub fn tile(position: [i32; 3]) -> [i8; 3] {
        [
            (position[0] >> 4) as i8,
            (position[1] >> 4) as i8,
            position[2] as i8,
        ]
    }

    /// The end of the object read (0x43fc02), after load steps 8 and 9:
    /// registry byte 0x12 ([`crate::neighbors::Neighbors::relationships_changed`])
    /// is set, whatever the steps found (0x43fc89). A hardware watchpoint
    /// on the byte in a capture of the original loading the Goth house
    /// stopped there, with the byte going from 0 to 1. The function also
    /// clears registry byte 0x11 when it starts, which is not kept here.
    pub fn finish_object_read(&mut self) {
        self.neighbors.relationships_changed = true;
    }

    /// Load step 9: clears every cell of the ground level's object grid
    /// that names an object which is not there (0x443cb3). Returns the
    /// cells cleared, in the order the original visits them.
    ///
    /// Only the cells inside the lot's size are looked at, column by
    /// column. A cell is cleared if its ID, read as a signed word, is
    /// negative, or names no object, or names an object whose tile and
    /// level ([`World::tile`]) are not this cell on level 1. The upper
    /// level's grid is not looked at. The step comes after the lot-wide
    /// controllers are reconciled, so an ID a new controller took counts
    /// as an object that is elsewhere.
    ///
    /// The original then walks every object and writes to its log what it
    /// finds wrong with the object's container or tile. That pass changes
    /// nothing and is not ported.
    ///
    /// This follows the original's code, read for this port, and the old
    /// prototype's `repair_ground_object_heads_in_active`
    /// (legacy/crates/sims-vm/src/loading.rs), which was compared with the
    /// original's 0x443cb3 run alone on made-up grids. For the Goth house
    /// a capture of the original taken after this step has the same grid
    /// as before it, and here too nothing is cleared. So no capture shows
    /// a cell being cleared: that is **unverified** against the running
    /// game.
    pub fn repair_grid(&mut self) -> Result<Vec<GridRepair>, LoadError> {
        let full = FULL_SIZE as usize;
        let active = usize::try_from(self.size).unwrap_or(0).min(full);
        let grid = self
            .arrays
            .get(&GROUND_GRID)
            .ok_or(LoadError::MissingArray(GROUND_GRID))?;
        if grid.width != full || grid.height != full {
            return Err(LoadError::ArraySize {
                array: GROUND_GRID,
                width: grid.width,
                height: grid.height,
            });
        }
        if grid.stride != 2 || grid.data.len() != full * full * 2 {
            return Err(LoadError::ArrayStride {
                array: GROUND_GRID,
                stride: grid.stride,
            });
        }
        let mut repairs = Vec::new();
        for column in 0..active {
            for row in 0..active {
                let cell = (column * full + row) * 2;
                let object = i16::from_le_bytes([grid.data[cell], grid.data[cell + 1]]);
                if object == 0 {
                    continue;
                }
                let found = u16::try_from(object)
                    .ok()
                    .and_then(|id| self.objects.get(&id))
                    .map(|o| Self::tile(o.position));
                if found != Some([column as i8, row as i8, 1]) {
                    repairs.push(GridRepair {
                        tile: [column, row],
                        object,
                        found,
                    });
                }
            }
        }
        if let Some(grid) = self.arrays.get_mut(&GROUND_GRID) {
            for repair in &repairs {
                let cell = (repair.tile[0] * full + repair.tile[1]) * 2;
                grid.data[cell..cell + 2].fill(0);
            }
        }
        Ok(repairs)
    }

    /// Load step 11: forgets the objects waiting to be removed (0x428679).
    /// The objects themselves stay, and so does the mark on an object that
    /// asked to be removed without resets
    /// ([`crate::Object::removal_skips_resets`]). Returns the IDs dropped.
    ///
    /// In a trace of the original loading the Goth house the queue holds
    /// four IDs before this step and none after, and nothing else the
    /// trace holds changes. Our state before and after the step was
    /// compared with the trace's and was equal; see `docs/decisions/`.
    pub fn discard_removals(&mut self) -> Vec<u16> {
        std::mem::take(&mut self.removals)
    }

    /// Clears the family in global 9 (0x415eff). The house loader does
    /// this after the lot is resized and before it reads the house's
    /// objects, so the lot-wide controllers the resize creates still see
    /// the saved family. [`World::select_family`] sets it again later.
    pub fn clear_family(&mut self) -> Result<(), LoadError> {
        self.context_applied()?;
        self.globals[9] = 0;
        Ok(())
    }

    /// Load step 12: selects the household that lives on the lot
    /// (0x428041). Global 9 takes the family's ID, which is the ID of its
    /// record in the neighborhood file (the FAMI chunk's ID, not the
    /// number inside the record). Global 10 takes the house number. The
    /// family's money becomes the lot's funds. `family` is 0 for an empty
    /// lot, which has no money.
    ///
    /// The family also becomes the one the game has selected (0x4282a2),
    /// so its magic history becomes [`World::magic_history`]. `magic` is
    /// the six words a family record of version 9 holds
    /// (`sims_formats::neighborhood::Family::magic_history`), or None for
    /// an older record or no family, which have no history to read.
    ///
    /// The funds setter (0x46d8dc) also queues signal 0xda for the
    /// interface. That is presentation, not simulation state, so it is not
    /// modeled.
    ///
    /// For the Goth house the globals, the funds, the family's ID and its
    /// six magic words were compared with a trace of the original taken
    /// when the funds setter returns, and were equal. The house saved its
    /// own number in global 10, so the trace does not show that global
    /// being written. Family 0 and a family record older than version 9
    /// are **unverified**.
    pub fn select_family(
        &mut self,
        family: i16,
        house: i16,
        money: i32,
        magic: Option<[i16; 6]>,
    ) -> Result<(), LoadError> {
        self.context_applied()?;
        self.magic_history = magic.filter(|_| family != 0).map(|values| MagicHistory {
            family: i32::from(family),
            values,
        });
        self.globals[9] = family;
        self.globals[10] = house;
        self.set_funds(if family == 0 { 0 } else { money })
    }

    /// The funds setter (0x46d8dc). It does nothing unless the budget mode
    /// is 0. Globals 14 and 13 hold the funds split at 10,000, each part
    /// truncated toward zero by the original's signed division, then cut
    /// to 16 bits.
    pub fn set_funds(&mut self, funds: i32) -> Result<(), LoadError> {
        self.context_applied()?;
        if self.globals[BUDGET_MODE] != 0 {
            return Ok(());
        }
        self.budget.funds = funds;
        self.globals[14] = (funds / 10000) as i16;
        self.globals[13] = (funds % 10000) as i16;
        Ok(())
    }

    /// The people on the lot whose visitor word (person data word 32) is
    /// 0: the household selection notes them before the family is
    /// selected and the objects are visited (0x42821a), and
    /// [`World::finish_selection`] reads the note.
    pub fn residents(&self) -> BTreeSet<u16> {
        self.people
            .iter()
            .copied()
            .filter(|id| {
                self.objects
                    .get(id)
                    .and_then(|o| o.person.as_ref())
                    .is_some_and(|p| p.data.get(VISITOR).copied().unwrap_or(0) == 0)
            })
            .collect()
    }

    /// Load step 15: the end of the household selection (0x428041, from
    /// 0x4283cf), once every object has been visited. In this order:
    ///
    /// 1. Each person whose type is a neighborhood character (registered
    ///    from the neighborhood's `Characters` folder, the folder of kind
    ///    1, 0x43c228), who belongs to no family (person data word 61)
    ///    and whose word 67 is not this house, is deleted (0x4283e1).
    /// 2. In some game modes a controller object (GUID 0xC3249A1D) is
    ///    created if global 26 is not 0 (0x42842e).
    /// 3. If the family's record asks for it (flag 4), the budget, the
    ///    statistics, the globals and the lot are reset (0x42848d).
    /// 4. Who is a visitor is worked out again (0x42a2b4): each person's
    ///    word 32 becomes 0 if its type is one of the family's members
    ///    (0x41019b), and otherwise stays as it was, or 1 if it was 0; on
    ///    a lot not zoned residential (0x40745f, read from global 10 now)
    ///    a 0 becomes 1 too.
    /// 5. Each neighborhood character who was a resident before (in
    ///    `household.residents`, see [`World::residents`]) and is now a
    ///    visitor is deleted (0x428550). The original then checks the
    ///    person's family number for a range that no number is in (below
    ///    3000 and from 3010 on, 0x428598), so that check deletes no one.
    /// 6. The lot's lighting is updated (0x4c4648), which is not modeled.
    ///
    /// Step 4 is ported. A deletion in steps 1 and 5, step 2 when global
    /// 26 is set and step 3 when flag 4 is set are not: they fail before
    /// anything is written.
    pub fn finish_selection(&mut self, household: &Household<'_>) -> Result<(), LoadError> {
        self.context_applied()?;
        let character = |object: &Object| household.characters.contains(&object.guid);
        let word = |object: &Object, index: usize| {
            object
                .person
                .as_ref()
                .and_then(|p| p.data.get(index).copied())
                .unwrap_or(0)
        };
        let people: Vec<u16> = self
            .people
            .iter()
            .copied()
            .filter(|id| self.objects.get(id).is_some_and(|o| o.person.is_some()))
            .collect();
        for id in &people {
            let object = &self.objects[id];
            if character(object)
                && word(object, FAMILY) == 0
                && i32::from(word(object, HOME)) != household.house
            {
                return Err(LoadError::Selection(format!(
                    "person {id}, a neighborhood character of no family whose word 67 is {}, \
                     not house {}, is deleted (0x42841a)",
                    word(object, HOME),
                    household.house
                )));
            }
        }
        if self.globals[26] != 0 {
            return Err(LoadError::Selection(
                "global 26 is set; the load then may create a controller object".into(),
            ));
        }
        if household.flags & 4 != 0 {
            return Err(LoadError::Selection(
                "the family's record asks for the lot to be reset (flag 4)".into(),
            ));
        }
        let lot = self.globals[10];
        let residential = lot == 0 || household.zoning.get(&lot) == Some(&1);
        let mut visitors = BTreeMap::new();
        for id in &people {
            let object = &self.objects[id];
            let visitor = visitor_again(
                word(object, VISITOR),
                household.members.contains(&object.guid),
                residential,
            );
            if character(object) && household.residents.contains(id) && visitor != 0 {
                return Err(LoadError::Selection(format!(
                    "person {id}, a neighborhood character who lived here, is now a visitor and \
                     is deleted (0x4285e2)"
                )));
            }
            visitors.insert(*id, visitor);
        }
        for (id, visitor) in visitors {
            if let Some(data) = self
                .objects
                .get_mut(&id)
                .and_then(|o| o.person.as_mut())
                .and_then(|p| p.data.get_mut(VISITOR))
            {
                *data = visitor;
            }
        }
        Ok(())
    }

    /// Works out the lot's statistics from its objects (0x416085,
    /// 0x444cb9), as far as that is ported: global 37, which says what
    /// kinds of object the lot has, and global 23, the lot's size. The
    /// original does this when the game goes to live mode, among other
    /// times.
    ///
    /// Global 37 is a set of bits. Each object on the registration list
    /// adds the bit of its category (data word 59, [`feature`]). Three
    /// more bits need two objects: bit 8 if the lot has an object of
    /// category 10 and one of categories 17, 21 to 24, 30, 39, 40, 58, 59
    /// or 74; bit 0x10 if it has category 10 and category 75; bit 0x20 if
    /// it has category 10 and category 76, or has category 97.
    ///
    /// Not ported, though the same pass of the original writes them:
    /// global 22, the value of the lot's objects and of its architecture
    /// ([`crate::Budget`]), and two flags in the family's record. They
    /// stay as they are. **Unverified**: in a trace of a Goth load this
    /// pass changes global 37 and none of the others, but the others
    /// already hold what the house saved, so the trace cannot show them
    /// being written.
    pub fn update_statistics(&mut self) -> Result<(), LoadError> {
        self.context_applied()?;
        let mut mask = 0i16;
        let (mut pair, mut with_10, mut with_75, mut with_76, mut alone) =
            (false, false, false, false, false);
        for id in &self.order {
            let Some(&category) = self.objects.get(id).and_then(|o| o.data.get(CATEGORY)) else {
                continue;
            };
            mask |= feature(category);
            match category {
                10 => with_10 = true,
                17 | 21..=24 | 30 | 39 | 40 | 58 | 59 | 74 => pair = true,
                75 => with_75 = true,
                76 => with_76 = true,
                97 => alone = true,
                _ => {}
            }
        }
        if pair && with_10 {
            mask |= 8;
        }
        if with_75 && with_10 {
            mask |= 0x10;
        }
        if (with_76 && with_10) || alone {
            mask |= 0x20;
        }
        self.globals[LOT_SIZE] = self.size;
        self.globals[LOT_FEATURES] = mask;
        Ok(())
    }

    fn context_applied(&self) -> Result<(), LoadError> {
        if self.globals.len() == 64 {
            Ok(())
        } else {
            Err(LoadError::NoContext)
        }
    }
}

/// Data word 59: an object's category.
const CATEGORY: usize = 59;
/// Globals 23 and 37: the lot's size, and what kinds of object it has.
const LOT_SIZE: usize = 23;
const LOT_FEATURES: usize = 37;

/// The bit of global 37 that an object of a category sets by itself
/// (0x444cb9), or 0. See [`World::update_statistics`].
pub fn feature(category: i16) -> i16 {
    match category {
        4 | 46..=48 | 71 => 1,
        25 | 26 | 50 | 72 => 2,
        73 => 4,
        43 | 49 | 51 | 77 => 0x40,
        80 => 0x80,
        84 => 0x100,
        85 => 0x200,
        83 => 0x400,
        95 => 0x800,
        93 | 94 => 0x1000,
        _ => 0,
    }
}
