//! The state-only steps of loading a lot: the globals the running game
//! supplies, and selecting the household. The load sequence in `docs/roadmap.md`
//! lists every step of the original's lot load in order; the ones that run
//! scripts live in `sims-vm`.

use crate::{FULL_SIZE, World};
use serde::{Deserialize, Serialize};
use sims_formats::lot::{Array, SurfaceMapping};
use sims_formats::tatt::Tatt;
use std::collections::BTreeMap;

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
    /// The house maps saved floor or wall IDs to pattern files. Resolving
    /// them needs the game's loaded floor and wall patterns, which are
    /// not ported.
    SurfaceCatalog,
    /// The renumbering would rewrite this wall cell, or cannot read its
    /// segments. Writing a wall cell is not ported.
    SurfaceRewrite { level: u8, x: usize, y: usize },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
                "house version {version}: only the array layout of versions 53 to 62 is ported"
            ),
            Self::SurfaceCatalog => write!(
                f,
                "the house maps saved floor or wall IDs to pattern files; resolving them is \
                 not ported"
            ),
            Self::SurfaceRewrite { level, x, y } => write!(
                f,
                "renumbering would rewrite the wall cell at {x}, {y} on level {level}; that is \
                 not ported"
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
/// reads. From version 63 the floors and walls have other IDs (11, 111, 12
/// and 112), and before 53 some arrays are made instead of read; neither
/// is ported.
pub const SAVED_ARRAYS: [u16; 14] = [0, 4, 1, 2, 3, 8, 101, 102, 103, 108, 6, 7, 9, 10];

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

/// Whether the renumbering looks a saved ID up (0x4a0e3b, 0x4c5d35).
/// Floors 0 to 29 and walls 0 to 30 are built in and keep their IDs, as do
/// 255 (a floor cut by a diagonal wall), wall IDs 248 to 255 and 65535.
fn renumbered(id: u16, wall: bool) -> bool {
    let low = if wall { 31..=247 } else { 30..=254 };
    low.contains(&id) || (256..=65534).contains(&id)
}

/// The global that holds the budget mode. Funds change only while it is 0.
const BUDGET_MODE: usize = 32;

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
    /// the lot's size keep what the lot's array held. A lot with no such
    /// array has zeros there, which is **unverified**: the original's lot
    /// always has arrays, and what they hold outside the size is not
    /// known.
    ///
    /// For the Goth house the cells inside the lot's size of arrays 0, 1,
    /// 2, 3, 8, 101, 102, 103, 108, 9 and 10 were compared with a capture
    /// of the original taken after this step and the renumbering; see
    /// `docs/decisions.md`. Arrays 4, 6 and 7 and every cell outside the
    /// size are **unverified**.
    ///
    /// Two things differ from the original. It stops at the first array
    /// that fails to read and goes on with the load; here a missing array
    /// is an error. And every array must be 64 by 64, as the original's are
    /// whatever the lot's size. The lot's lighting, which the original
    /// resets here, is not modeled.
    ///
    /// The widening follows the old prototype's reading of the code
    /// (legacy/docs/research/saved-world-array-contract-20261026.md).
    pub fn read_arrays(
        &mut self,
        saved: &BTreeMap<u16, Array>,
        version: u32,
    ) -> Result<(), LoadError> {
        if !(53..=62).contains(&version) {
            return Err(LoadError::ArrayVersion(version));
        }
        let full = FULL_SIZE as usize;
        let active = usize::try_from(self.size).unwrap_or(0).min(full);
        let mut read = Vec::new();
        for id in SAVED_ARRAYS {
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
    /// pattern file each ID means. The original turns each map into a
    /// table from saved ID to the ID of the pattern it has loaded, and
    /// rewrites every cell inside the lot's size, both levels.
    ///
    /// `floors` and `walls` are the house's maps, or None for a map the
    /// file lacks; the original then leaves that kind alone. Returns how
    /// many floor cells changed. Call it after [`World::read_arrays`]. On an
    /// error the lot is unchanged.
    ///
    /// Only an empty map is ported. A map with entries needs the game's
    /// loaded patterns to resolve them, and fails. With an empty map no
    /// saved ID is listed, and the original gives an unlisted ID that
    /// [`renumbered`] covers the ID 65535. A floor cell is rewritten that
    /// way here. A wall cell is not: the original writes one through a
    /// service that also updates its neighbours, which is not ported, so a
    /// wall pattern or a diagonal wall's half floor that would change is an
    /// error.
    ///
    /// For the Goth house both maps are empty and no cell holds an ID the
    /// renumbering covers, so nothing changes. A capture of the original
    /// taken after this step agrees: its floors and walls are the saved
    /// ones (see [`World::read_arrays`]). The rewrite to 65535 is
    /// **unverified** against the running game. It follows the old
    /// prototype's port (legacy/crates/sims-sim/src/surface_remap.rs),
    /// which was compared with the original's code run in isolation on 24
    /// made-up lots (legacy/docs/research/surface-remap-native-20261026.md).
    pub fn renumber_surfaces(
        &mut self,
        floors: Option<&[SurfaceMapping]>,
        walls: Option<&[SurfaceMapping]>,
    ) -> Result<usize, LoadError> {
        let full = FULL_SIZE as usize;
        let active = usize::try_from(self.size).unwrap_or(0).min(full);
        for map in [floors, walls].into_iter().flatten() {
            if map.iter().any(|group| !group.entries.is_empty()) {
                return Err(LoadError::SurfaceCatalog);
            }
        }
        let array = |id: u16, stride: usize| -> Result<&Array, LoadError> {
            let array = self.arrays.get(&id).ok_or(LoadError::MissingArray(id))?;
            if array.stride != stride || array.data.len() != full * full * stride {
                return Err(LoadError::ArrayStride {
                    array: id,
                    stride: array.stride,
                });
            }
            Ok(array)
        };
        let word = |cell: &[u8], at: usize| u16::from_le_bytes([cell[at], cell[at + 1]]);
        let mut rewrites = Vec::new();
        for level in 0..2 {
            let floor = array(FLOORS[level], FLOOR_CELL)?;
            let wall = array(WALLS[level], WALL_CELL)?;
            for y in 0..active {
                for x in 0..active {
                    let cell = x * full + y;
                    let stop = LoadError::SurfaceRewrite {
                        level: level as u8 + 1,
                        x,
                        y,
                    };
                    let wall = &wall.data[cell * WALL_CELL..(cell + 1) * WALL_CELL];
                    let segments = wall[0];
                    // The lowest segment the cell has, or 0 for none.
                    let lowest = segments & segments.wrapping_neg();
                    if floors.is_some() {
                        let id = word(&floor.data, cell * FLOOR_CELL);
                        if renumbered(id, false) {
                            rewrites.push((FLOORS[level], cell));
                        } else if id == 255 && matches!(lowest, 0x10 | 0x20) {
                            // A diagonal wall splits the cell: its two
                            // half floors are the wall's first style and
                            // first pattern words.
                            if renumbered(word(wall, 2), false) || renumbered(word(wall, 6), false)
                            {
                                return Err(stop);
                            }
                        }
                    }
                    if walls.is_some() && lowest != 0 {
                        // The original starts at the lowest segment and
                        // stops once it has moved on to the last one
                        // (0x20). Segments above that have no pattern.
                        if lowest > 0x20 {
                            return Err(stop);
                        }
                        let mut segment = lowest;
                        loop {
                            // Each segment's pattern words, as offsets in
                            // the cell: one for a straight segment, two
                            // for a diagonal one.
                            let patterns: &[usize] = match segment {
                                1 => &[6],
                                2 => &[8],
                                4 => &[12],
                                8 => &[10],
                                _ => &[10, 12],
                            };
                            if patterns.iter().any(|&at| renumbered(word(wall, at), true)) {
                                return Err(stop);
                            }
                            segment <<= 1;
                            while segment < 0x20 && segments & segment == 0 {
                                segment <<= 1;
                            }
                            if segment >= 0x20 {
                                break;
                            }
                        }
                    }
                }
            }
        }
        for &(id, cell) in &rewrites {
            if let Some(array) = self.arrays.get_mut(&id) {
                array.data[cell * FLOOR_CELL..(cell + 1) * FLOOR_CELL]
                    .copy_from_slice(&u16::MAX.to_le_bytes());
            }
        }
        Ok(rewrites.len())
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

    /// Selects the household that lives on the lot (0x428041): global 9
    /// takes its family number, global 10 the house number, and the
    /// family's money becomes the lot's funds. `family` is 0 for an empty
    /// lot, which has no money.
    ///
    /// The funds setter (0x46d8dc) also queues signal 0xda for the
    /// interface. That is presentation, not simulation state, so it is not
    /// modeled.
    pub fn select_family(&mut self, family: i16, house: i16, money: i32) -> Result<(), LoadError> {
        self.context_applied()?;
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

    fn context_applied(&self) -> Result<(), LoadError> {
        if self.globals.len() == 64 {
            Ok(())
        } else {
            Err(LoadError::NoContext)
        }
    }
}
