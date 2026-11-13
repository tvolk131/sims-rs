//! The state-only steps of loading a lot: the globals the running game
//! supplies, and selecting the household. The load sequence in `docs/roadmap.md`
//! lists every step of the original's lot load in order; the ones that run
//! scripts live in `sims-vm`.

use crate::{FULL_SIZE, World};
use serde::{Deserialize, Serialize};
use sims_formats::lot::Array;
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
        }
    }
}

impl std::error::Error for LoadError {}

/// The lot arrays a load reads from the house file, by Arry chunk ID, in
/// the order the original asks for them (0x478599) when the house's
/// version is 62 or later. The order is from the old prototype's reading of
/// the original's code (legacy/docs/research/
/// retained-saved-source-prefix-audit-20261026.md), not from a trace of the
/// reads.
pub const SAVED_ARRAYS: [u16; 14] = [0, 4, 1, 2, 3, 8, 101, 102, 103, 108, 6, 7, 9, 10];

/// The last two of [`SAVED_ARRAYS`]: the original ignores whether reading
/// them worked.
const OPTIONAL_ARRAYS: [u16; 2] = [9, 10];

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
    /// **Unverified**: no capture of the original covers this step. Three
    /// things are known to be missing. The original stops at the first
    /// array that fails to read and goes on with the load; here a missing
    /// array is an error. The original then renumbers the saved floor and
    /// wall IDs (0x4a0e3b, 0x4c5d35), which is not ported. And the lot's
    /// size is not applied to the arrays: every array must be 64 by 64, as
    /// the original's are whatever the lot's size.
    pub fn read_arrays(&mut self, saved: &BTreeMap<u16, Array>) -> Result<(), LoadError> {
        let mut read = Vec::new();
        for id in SAVED_ARRAYS {
            let Some(array) = saved.get(&id) else {
                if OPTIONAL_ARRAYS.contains(&id) {
                    continue;
                }
                return Err(LoadError::MissingArray(id));
            };
            let full = FULL_SIZE as usize;
            if array.width != full || array.height != full {
                return Err(LoadError::ArraySize {
                    array: id,
                    width: array.width,
                    height: array.height,
                });
            }
            read.push((id, array.clone()));
        }
        self.arrays.extend(read);
        Ok(())
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
