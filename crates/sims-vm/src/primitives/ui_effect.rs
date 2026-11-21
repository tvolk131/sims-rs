//! UI Effect (opcode 34, 0x44acff): highlight a control in the interface
//! (used by the Tutorial and Help System).
//!
//! Operands: the mode (byte 0, unsigned), the variable scope (byte 1,
//! unsigned) and index (word 2, signed) holding the control or neighbor
//! ID, and flags (byte 4, bit 0: enable highlight). Bytes 5 to 7 are
//! unread.
//!
//! - **Mode 0** (`0x44ad3f`): reads the variable (`0x453141`), highlights
//!   the matching `cTSWinBtn` (`0x476468` → `0x5de13f`, presentation
//!   only), and **falls through into mode 1** (`0x44ad61` — no `break`
//!   in the original `switch`), reading the variable a second time and
//!   running `0x47689c`.
//! - **Mode 1** (`0x44ad61`): reads the variable (`0x453141`) and runs
//!   `0x47689c`: looks up the 1-based neighbor record (`0x4256f4`), finds
//!   its raw GUID (`0x5ed65e`, record +4) in the interface portrait table
//!   (`0x4e9753`), calls `0x4e92a8` unconditionally (even when flag bit 0
//!   is clear), and highlights the matching portrait button (`0x47692b` →
//!   `0x5de13f`, presentation only).
//! - **Mode 2** (`0x44ad1f`): reads the variable (`0x453141`) and runs
//!   `0x476984`: looks up the neighbor (`0x4256f4`) and its index in the
//!   portrait table (`0x4e9753`, read-only — does **not** call
//!   `0x4e92a8`), and highlights `cWinRelationship` (`0x4769ca` →
//!   `0x5de13f`, presentation only).
//! - **Modes 3 and above** (`0x44ad1d`): skip the variable read and do
//!   nothing.
//!
//! **Global 34 (`0x45412b`).** Unlike primitives that bind a destination
//! pointer at `object + 0x128` before calling `0x453141`, `0x44acff`
//! leaves `object + 0x128` untouched (zeroed by the constructor at
//! `0x431a77`). For scope 6, index 34, `0x45412b` jumps directly to
//! `0x4541d5` even with read intent (`write == 0`), dereferencing
//! `[object + 0x128]` without a null check. Modes 0 to 2 therefore fail
//! loudly on scope 6, index 34.
//!
//! **Portrait table & `0x4e92a8`.** At startup `0x4e8b3d` walks every
//! registered object definition in the catalog and appends each person
//! definition (`object_type == 2`) to `0x6f2698` with its rebuild/dirty
//! flag clear (`0x4e88fa`). Runtime-made characters (`Make New Character`,
//! `0x4262d8`) broadcast `0xe0` (`0x43c5fc` → `0x4f6727` → `0x4e8a44` →
//! `0x4e8add`), appending their person GUID to `0x6f2698` as well.
//! `0x4e92a8` then follows:
//!
//! 1. If the slot's dirty flag (`[entry + 0x18]`, set by `0x4e895e` /
//!    `0x4eb0eb`) is set (`0x4e92e5`), call `0x4e8c5d(index, 1, 0)` first
//!    (`0x4e92f2`).
//! 2. If the slot now holds a bitmap (`World::portraits`, `0x4e931c`),
//!    done.
//! 3. Otherwise try loading `BMP_` 2002 (`0x7d2`) from the winning
//!    definition's file (`0x4e9338` → `0x4e9260` → `0x4dece6`,
//!    [`crate::program::Library::has_portrait_2002`]). When present, cache
//!    it (`0x4e899e`, clearing the dirty flag) without creating a Sim.
//! 4. Otherwise call `0x4e8c5d(index, 1, 1)` (`0x4e938c`).
//!
//! Inside `0x4e8c5d`, an on-lot person of the GUID (`0x4e9608`) is drawn
//! directly without creating a Sim. Otherwise `0x4e9652` resolves the
//! catalog's current definition (`0x43ac10`), calls the object factory
//! (`0x440b2d`), checks `dynamic_cast<cXPerson*>` (`0x4e96b7`), searches
//! for a tile (`0x4347f0`), and removes the temporary person on failure
//! (`0x4e9706`) or after drawing (`0x4e8fdb`). Refused factory creation
//! (`0x4e969d`), non-person dynamic_cast failure (`0x4e96c3`), and
//! no-tile removal (`0x4e9706`) leave the portrait uncached and keep all
//! simulation side effects while `0x44ad83` still takes the true branch.
//!
//! Every mode that completes takes the true branch (`0x44ad83`).

use crate::fault::Trap;
use crate::interpreter::{Machine, word};

/// Scope 6: global words.
const GLOBAL_SCOPE: u8 = 6;

/// Global 34 (`0x22`), whose read path at `0x45412b` unconditionally writes
/// through `[object + 0x128]` at `0x4541d5`.
const GLOBAL_34: i16 = 34;

/// Object definition word 9: the object type (2 for a person).
const TYPE: usize = 9;
const PERSON_TYPE: i16 = 2;

impl Machine<'_, '_> {
    pub(crate) fn ui_effect(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let mode = operands[0];
        let scope = operands[1];
        let index = word(operands, 2);
        match mode {
            0 => {
                // 0x44ad3f: highlight cTSWinBtn (0x476468, presentation), then
                // fall through into mode 1 (0x44ad61), reading the variable a
                // second time.
                let _ = self.read_ui_effect_var(scope, index)?;
                let value = self.read_ui_effect_var(scope, index)?;
                self.ui_effect_portrait(value)?;
            }
            1 => {
                // 0x44ad61: highlight the neighbor's portrait button (0x47689c),
                // running 0x4e92a8 unconditionally.
                let value = self.read_ui_effect_var(scope, index)?;
                self.ui_effect_portrait(value)?;
            }
            2 => {
                // 0x44ad1f: read the variable and highlight cWinRelationship
                // (0x476984, presentation only; does not call 0x4e92a8).
                let _ = self.read_ui_effect_var(scope, index)?;
            }
            _ => {}
        }
        Ok(true)
    }

    /// Reads the operand variable (`0x453141` with `write == 0`), failing
    /// loudly on global 34 where `0x45412b` → `0x4541d5` writes through the
    /// uninitialized pointer at `object + 0x128`.
    fn read_ui_effect_var(&mut self, scope: u8, index: i16) -> Result<i16, Trap> {
        if scope == GLOBAL_SCOPE && index == GLOBAL_34 {
            return Err(Trap::Unsupported(
                "UI Effect reading global 34 (0x45412b -> 0x4541d5 writes through uninitialized \
                 object +0x128)"
                    .into(),
            ));
        }
        self.read(scope, index)
    }

    /// The simulation-observable part of `0x47689c`: look up the 1-based
    /// neighbor record (`0x4256f4`), resolve its raw GUID (`0x5ed65e`,
    /// record +4) in the portrait table (`0x4e9753`), and run `0x4e92a8`.
    fn ui_effect_portrait(&mut self, neighbor: i16) -> Result<(), Trap> {
        let Some(guid) = self.world.neighbors.get(neighbor).map(|r| r.guid) else {
            return Ok(());
        };
        if !self.in_portrait_table(guid)? {
            return Ok(());
        }
        // 0x4e92e5: when the slot's dirty flag ([entry + 0x18], set by
        // 0x4e895e) is set, call 0x4e8c5d(index, 1, 0) first.
        if self.world.dirty_portraits.contains(&guid) {
            let _ = self.try_render_ui_effect_portrait(guid)?;
        }
        // 0x4e931c: if the slot now holds a cached bitmap, done.
        if self.world.portraits.contains(&guid) {
            return Ok(());
        }
        // 0x4e9338 -> 0x4e9260 -> 0x4dece6: try BMP_ 2002 (0x7d2) from the
        // winning definition's file (0x4e893e -> 0x43ac10). When present,
        // cache it (0x4e899e) without calling 0x4e8c5d or creating a Sim.
        if self.has_portrait_2002(guid) {
            self.world.portraits.insert(guid);
            self.world.dirty_portraits.remove(&guid);
            return Ok(());
        }
        // 0x4e938c: BMP_ 2002 absent or invalid -> call 0x4e8c5d(index, 1, 1).
        let _ = self.try_render_ui_effect_portrait(guid)?;
        Ok(())
    }

    /// Whether `guid` has an entry in `0x6f2698` (`0x4e9753`):
    /// - startup `0x4e8b3d` walks every registered catalog definition and
    ///   appends every person definition (`object_type == 2`), even if a
    ///   later non-person definition overwrites `by_guid`;
    /// - runtime-made characters (`0x4262d8` → `0x43c5fc` `0xe0` → `0x4f6727`
    ///   → `0x4e8a44` → `0x4e8add`) append their person GUID while the
    ///   portrait manager is active (`[0x6f26a4] != 0`).
    fn in_portrait_table(&self, guid: u32) -> Result<bool, Trap> {
        if self.catalog.objects.iter().any(|object| {
            object.guid == guid
                && self
                    .program
                    .definition(object.resource, object.chunk)
                    .map_or(object.kind == PERSON_TYPE as u16, |words| {
                        words[TYPE] == PERSON_TYPE
                    })
        }) {
            return Ok(true);
        }
        if self.world.characters.made.contains_key(&guid) {
            return Ok(self.definition_words(guid)?[TYPE] == PERSON_TYPE);
        }
        Ok(false)
    }

    /// Whether the catalog's winning definition for `guid` (`0x4e893e` →
    /// `0x43ac10`) lives in a file holding a decodable `BMP_` 2002 portrait.
    fn has_portrait_2002(&self, guid: u32) -> bool {
        let Some(def) = crate::interpreter::object_definition(self.catalog, self.world, guid)
        else {
            return false;
        };
        self.program
            .libraries
            .get(&def.resource)
            .is_some_and(|library| library.has_portrait_2002)
    }

    /// Runs `0x4e8c5d(index, 1, _)` for a `guid` already verified to be in
    /// `0x6f2698`. Returns `true` when the portrait was rendered and cached,
    /// `false` when `0x4e9652` returned null (leaving the portrait uncached
    /// and dirty state unchanged, while keeping all simulation side effects).
    fn try_render_ui_effect_portrait(&mut self, guid: u32) -> Result<bool, Trap> {
        // 0x4e8c92 -> 0x4e893e -> 0x43ac10: resolve the winning definition.
        if crate::interpreter::object_definition(self.catalog, self.world, guid).is_none() {
            return Ok(false);
        }
        // 0x4e9608: reuse the first on-lot person matching `guid`.
        let on_lot = self.world.people.iter().any(|id| {
            self.world
                .objects
                .get(id)
                .is_some_and(|object| object.guid == guid)
        });
        if on_lot {
            self.world.portraits.insert(guid);
            self.world.dirty_portraits.remove(&guid);
            return Ok(true);
        }
        // 0x4e9652: no on-lot person -> create a temporary instance via
        // 0x440b2d (no household-membership guard in native code).
        let interpreter = self.interpreter();
        let unsupported = |fault| {
            Trap::Unsupported(format!(
                "making a person of type 0x{guid:08X} for UI Effect's portrait (0x4e9652): \
                 {fault}"
            ))
        };
        let mut copy = self.world.clone();
        let cells = crate::both_ways::Cells::of(&interpreter);
        let built = cells.on(&interpreter);
        if self.definition_words(guid)?[TYPE] != PERSON_TYPE {
            // 0x4e8b3d registered an earlier person definition under `guid`,
            // while 0x43ac10 now resolves `guid` to a non-person definition.
            // 0x4e9692 creates the non-person (running Init), dynamic_cast to
            // cXPerson fails at 0x4e96b7, and 0x4e96c3 returns null without
            // removing the created object.
            let _ = built.try_create(&mut copy, guid).map_err(unsupported)?;
            if !cells.same_refresh(&crate::both_ways::Cells::of(&interpreter)) {
                return Err(Trap::Unsupported(format!(
                    "making an object of type 0x{guid:08X} for UI Effect's portrait (0x4e9652) \
                     changed the refresh pending, which is not ported"
                )));
            }
            cells.keep(&interpreter, self.world, copy);
            return Ok(false);
        }
        let Some(id) = built
            .try_create_person(&mut copy, guid)
            .map_err(unsupported)?
        else {
            // 0x4e969d: factory returned 0 (e.g. refused pet). Keep any
            // side effects, leave portrait uncached, return false.
            if !cells.same_refresh(&crate::both_ways::Cells::of(&interpreter)) {
                return Err(Trap::Unsupported(format!(
                    "making a person of type 0x{guid:08X} for UI Effect's portrait (0x4e9652) \
                     changed the refresh pending, which is not ported"
                )));
            }
            cells.keep(&interpreter, self.world, copy);
            return Ok(false);
        };
        let placed = built
            .portrait_put_outcome(&mut copy, id)
            .map_err(|fault| fault.trap)?;
        // Both normal completion (0x4e8fdb) and no-tile failure (0x4e9706)
        // remove the temporary person immediately and keep its Init/RNG
        // side effects.
        built.remove_now(&mut copy, id).map_err(unsupported)?;
        if !cells.same_refresh(&crate::both_ways::Cells::of(&interpreter)) {
            return Err(Trap::Unsupported(format!(
                "making a person of type 0x{guid:08X} for UI Effect's portrait (0x4e9652) \
                 changed the refresh pending, which is not ported"
            )));
        }
        cells.keep(&interpreter, self.world, copy);
        if placed {
            self.world.portraits.insert(guid);
            self.world.dirty_portraits.remove(&guid);
        }
        Ok(placed)
    }
}
