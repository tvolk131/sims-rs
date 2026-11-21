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
//!    (`0x4e92f2`, save flag `0`: does not write `BMP_` 2002).
//! 2. If the slot now holds a bitmap (`World::portraits`, `0x4e931c`),
//!    done.
//! 3. Otherwise try loading neutral-language `BMP_` 2002 (`0x7d2`, high
//!    byte of flags `0`, `0x41bab1` / `0x41bad8`) from the winning
//!    definition's file (`0x4e9338` → `0x4e9260` → `0x4dece6`,
//!    [`crate::program::Library::has_portrait_2002`] or
//!    [`sims_sim::World::generated_portrait_2002`]). When present, cache
//!    it (`0x4e899e`, clearing the dirty flag) without creating a Sim.
//! 4. Otherwise call `0x4e8c5d(index, 1, 1)` (`0x4e938c`, save flag `1`:
//!    on success for a writable file, `0x4e8d2e` → `0x4e9000` → `0x4e8feb`
//!    writes and flushes `BMP_` 2002).
//!
//! Inside `0x4e8c5d`, an on-lot person of the GUID (`0x4e9608`) is drawn
//! directly without creating a Sim. Otherwise `0x4e8cc6` checks whether
//! the winning definition's resource file is writable (`0x4067b3` →
//! `0x4174dc`) and skips `0x4e9652` when read-only (e.g. a `.far` archive
//! member). For a writable file, `0x4e9652` resolves the catalog's
//! current definition (`0x43ac10`), calls the object factory (`0x440b2d`),
//! checks `dynamic_cast<cXPerson*>` (`0x4e96b7`), searches for a tile
//! (`0x4347f0`), and removes the temporary person on failure (`0x4e9706`)
//! or after drawing (`0x4e8fdb`). Refused factory creation (`0x4e969d`),
//! non-person dynamic_cast failure (`0x4e96c3`), and no-tile removal
//! (`0x4e9706`) leave the portrait uncached and keep all simulation side
//! effects while `0x44ad83` still takes the true branch.
//!
//! Every mode that completes takes the true branch (`0x44ad83`).

use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use sims_sim::PortraitFile;

/// Scope 6: global words.
const GLOBAL_SCOPE: u8 = 6;

/// Global 34 (`0x22`), whose read path at `0x45412b` unconditionally writes
/// through `[object + 0x128]` at `0x4541d5`.
const GLOBAL_34: i16 = 34;

/// Object definition word 9: the object type (2 for a person).
const TYPE: usize = 9;
const PERSON_TYPE: i16 = 2;

/// Extracts the lowercased suit name from a `STR# 200` outfit row
/// (`"suit,..."`), returning `None` only for empty suit names (`0x583569`).
fn extract_suit_name(raw: &str) -> Option<String> {
    let (suit, _) = crate::outfits::parse_row_descriptor(raw, false);
    (!suit.is_empty()).then_some(suit)
}

/// Fallback GUID-level `"HEAD"` binding status for synthetic fixtures that
/// populate `People::portrait_head` or `World::made_portrait_head` without
/// `Profile::{body, head}`.
fn portrait_head_status_in(
    program: &crate::Program,
    world: &sims_sim::World,
    guid: u32,
) -> Option<bool> {
    if let Some(made) = world.characters.made.get(&guid) {
        let mut has_suit_override = false;
        for row in [1usize, 2] {
            if let Some(raw) = made.strings.get(&row) {
                has_suit_override = true;
                if extract_suit_name(raw)
                    .is_some_and(|suit| program.people.head_suits.contains(&suit))
                {
                    return Some(true);
                }
            }
        }
        if has_suit_override {
            return Some(false);
        }
        if let Some(&status) = world.made_portrait_head.get(&guid) {
            return status;
        }
        if let Some(&has_head) = program.people.portrait_head.get(&guid) {
            return Some(has_head);
        }
        return Some(
            program
                .people
                .portrait_head
                .get(&made.template)
                .copied()
                .unwrap_or(false),
        );
    }
    Some(
        program
            .people
            .portrait_head
            .get(&guid)
            .copied()
            .unwrap_or(false),
    )
}

/// Combines two optional `"HEAD"` binding statuses without short-circuiting.
fn combine_head_status(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

/// Whether the renderer (`0x4e725f` → `0x4e7801` → `0x58a178(1)` → `0x4e7817`
/// → `0x4e810d` → `0x4e81a4` → `0x431726` → `0x4e7824`) finds a
/// case-sensitive `"HEAD"` binding on the specific Sim `person_id` in `world`.
///
/// Executes [`crate::outfits::dress_person`] with `force = true` (`0x4e7801`),
/// checks the skeleton's `"HEAD"` bone (`0x595674`), and inspects the Sim's
/// actually attached slots (`World::person_outfits`) and accessories.
pub(crate) fn can_render_person_portrait(
    catalog: &sims_content::Catalog,
    program: &crate::Program,
    world: &mut sims_sim::World,
    person_id: u16,
    guid: u32,
    defer_unknown_head: bool,
    caller: &str,
) -> Result<Option<bool>, Trap> {
    let had_outfit_before = world.person_outfits.contains_key(&person_id);
    // 0x4e7801 -> 0x58a178(1): run full forced dressing BEFORE 0x4e7824 checks
    // for a usable "HEAD" bone.
    crate::outfits::dress_person(catalog, program, world, person_id, true)?;

    // 0x595674..0x595699 (checked at 0x4e7824 AFTER 0x4e7801 -> 0x58a178): a
    // "HEAD" binding attaches only when the Sim's skeleton defines a bone named
    // "HEAD". A decoded zero-bone skeleton (`scales[place].is_empty()`) has no
    // "HEAD" bone and returns false; synthetic fixtures with non-empty scales
    // and omitted `bones` skip this check.
    if let Ok(skel) = crate::people::skeleton(&program.people, world, guid)
        && (!skel.bones.is_empty() || skel.scales.get(skel.place).is_some_and(Vec::is_empty))
        && !skel.bones.iter().any(|b| b.name == "HEAD")
    {
        return Ok(Some(false));
    }

    // Inspect actually attached slots (`[ebx + 0x258]`, `[ebx + 0x25c]`,
    // `[ebx + 0x268]`, and `[ebx + 0x274]` accessories).
    let mut any_slot_bound = false;
    let mut status = Some(false);
    let mut has_bound_accessories = false;
    if let Some(outfit) = world.person_outfits.get(&person_id) {
        for bound in [
            &outfit.head,
            &outfit.body,
            &outfit.body_alt,
            &outfit.extra_left,
            &outfit.extra_right,
        ]
        .into_iter()
        .flatten()
        .chain(&outfit.accessories)
        {
            any_slot_bound = true;
            status = combine_head_status(status, bound.has_head);
        }
        has_bound_accessories = !outfit.accessories.is_empty();
    }
    // Fallback only for synthetic fixtures that directly populate
    // `person.accessories` on a Sim with no pre-existing `PersonOutfit` entry.
    if !had_outfit_before
        && !has_bound_accessories
        && let Some(accessories) = world
            .objects
            .get(&person_id)
            .and_then(|o| o.person.as_ref())
            .map(|p| &p.accessories)
    {
        for acc in accessories {
            if acc.name.is_empty() {
                continue;
            }
            let lower = acc.name.to_ascii_lowercase();
            let acc_status = match crate::people::peek_suit_head(&program.people, world, &lower) {
                crate::people::ResolvedSuit::Found(h) => Some(h),
                crate::people::ResolvedSuit::Missing => Some(false),
                crate::people::ResolvedSuit::Unknown => None,
            };
            status = combine_head_status(status, acc_status);
        }
    }

    let has_profile_rows = {
        let template = world.characters.definition(guid);
        let prof = program.people.profiles.get(&template);
        let made = world.characters.made.get(&guid);
        let made_outfit = world.made_outfits.get(&guid);
        prof.is_some_and(|p| p.body.is_some() || p.head.is_some() || !p.suit_rows.is_empty())
            || made.is_some_and(|m| m.strings.contains_key(&1) || m.strings.contains_key(&2))
            || made_outfit.is_some_and(|o| !o.is_empty())
    };
    if !has_profile_rows && !any_slot_bound && status == Some(false) {
        status = portrait_head_status_in(program, world, guid);
    }

    if status.is_none() && !defer_unknown_head {
        return Err(Trap::Unsupported(format!(
            "{caller} of type 0x{guid:08X} (0x4e7824): whether its suit binds HEAD is unknown"
        )));
    }
    Ok(status)
}

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

    /// Reads the operand variable (`0x453141` with `write == 0`). On global 34,
    /// `0x45412b` → `0x4541d5` writes `&globals[34]` through `[object + 0x128]`
    /// without a null check: `Expression` (`0x452c85`, targets saved `ebp`
    /// restored at `0x452273`), `Random Number` (`0x451e8d`, `ebp + 0xc`),
    /// `Get Direction To` (`0x45202b`, `ebp + 0xc`), and `Relationship`
    /// (opcode 26, `0x44d513`, `ebp - 0xc`) leave `[object + 0x128]` safe
    /// (`World::scratch_ptr_set`), whereas an initial null pointer
    /// (`0x431a77`, unchanged by `Old Relationship` opcode 24 at `0x44d593`) or
    /// `Set To Next` (`0x44eb9d`, `ebp - 0x18` → overwrites `0x44ad70`'s return
    /// address and faults at `&globals[34]`) traps loudly.
    fn read_ui_effect_var(&mut self, scope: u8, index: i16) -> Result<i16, Trap> {
        if scope == GLOBAL_SCOPE
            && index == GLOBAL_34
            && !self.world.scratch_ptr_set.contains(&self.me)
        {
            return Err(Trap::Unsupported(
                "UI Effect reading global 34 (0x45412b -> 0x4541d5 writes through null or \
                 stack-corrupting object +0x128)"
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
        self.ui_effect_portrait_for_guid(guid, "UI Effect's portrait")
    }

    /// Runs `0x4e92a8` for a known `guid` (`0x47689c`, `0x4363d1`).
    pub(crate) fn ui_effect_portrait_for_guid(
        &mut self,
        guid: u32,
        caller: &str,
    ) -> Result<(), Trap> {
        if !self.in_portrait_table(guid)? {
            return Ok(());
        }
        // 0x4e92e5: when the slot's dirty flag ([entry + 0x18], set by
        // 0x4e895e) is set, call 0x4e8c5d(index, 1, 0) first (save flag 0).
        if self.world.dirty_portraits.contains(&guid) {
            let _ = self.try_render_ui_effect_portrait(guid, false, caller)?;
        }
        // 0x4e931c: if the slot now holds a cached bitmap, done.
        if self.world.portraits.contains(&guid) {
            return Ok(());
        }
        // 0x4e9338 -> 0x4e9260 -> 0x4dece6 -> 0x41cb2f -> 0x41bab1: try
        // neutral-language BMP_ 2002 (0x7d2) from the winning definition's
        // file (0x4e893e -> 0x43ac10), or one persisted earlier by 0x4e9000.
        if self.has_portrait_2002(guid) {
            self.world.portraits.insert(guid);
            self.world.dirty_portraits.remove(&guid);
            return Ok(());
        }
        // If an earlier Make New Character pet render (0x44b126) placed the
        // pet and reached 0x4e7824 with an unknown HEAD binding, whether it
        // already cached the portrait or wrote BMP_ 2002 depends on unmodeled
        // wardrobe order; fail loudly before running a second 0x4e8c5d.
        if self.world.ambiguous_portraits.contains(&guid) {
            let _ = self.can_render_portrait(guid, caller)?;
        }
        // 0x4e938c: BMP_ 2002 absent or invalid -> call 0x4e8c5d(index, 1, 1)
        // (save flag 1: persists BMP_ 2002 via 0x4e9000 on success when writable).
        let _ = self.try_render_ui_effect_portrait(guid, true, caller)?;
        Ok(())
    }

    /// Whether `guid` has an entry in `0x6f2698` (`0x4e9753`):
    /// - startup `0x4e8b3d` walks every registered catalog definition and
    ///   appends every person definition (`object_type == 2`), even if a
    ///   later non-person definition overwrites `by_guid`;
    /// - runtime-made characters (`0x4262d8` → `0x43c5fc` `0xe0` → `0x4f6727`
    ///   → `0x4e8a44` → `0x4e8add`) append their person GUID while the
    ///   portrait manager is active (`[0x6f26a4] != 0`).
    pub(crate) fn in_portrait_table(&self, guid: u32) -> Result<bool, Trap> {
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

    /// Which IFF file holds the winning definition for `guid` (`0x4e893e` →
    /// `0x43ac10`), as `0x4e9000` writes `BMP_` 2002 into that file.
    pub(crate) fn portrait_file(&self, guid: u32) -> Option<PortraitFile> {
        if let Some(made) = self.world.characters.made.get(&guid) {
            return Some(PortraitFile::Made(made.file));
        }
        crate::interpreter::object_definition(self.catalog, self.world, guid)
            .map(|def| PortraitFile::Resource(def.resource))
    }

    /// Whether the winning definition's file for `guid` (`0x4e893e` →
    /// `0x43ac10`) holds a neutral-language `BMP_` 2002 portrait (either
    /// loaded from disk or generated earlier by `0x4e938c` → `0x4e9000`).
    pub(crate) fn has_portrait_2002(&self, guid: u32) -> bool {
        if self
            .portrait_file(guid)
            .is_some_and(|file| self.world.generated_portrait_2002.contains(&file))
        {
            return true;
        }
        let Some(def) = crate::interpreter::object_definition(self.catalog, self.world, guid)
        else {
            return false;
        };
        self.program
            .libraries
            .get(&def.resource)
            .is_some_and(|library| library.has_portrait_2002)
    }

    /// Whether the program/world supplies appearance, suit, or bone metadata
    /// for `guid` (distinguishing real installs and portrait-appearance tests
    /// from bare-bones synthetic fixtures that omit all suit/bone metadata).
    pub(crate) fn has_portrait_render_metadata(&self, world: &sims_sim::World, guid: u32) -> bool {
        let template = world.characters.definition(guid);
        let profile = self.program.people.profiles.get(&template);
        if profile.is_some_and(|p| p.body.is_some() || p.head.is_some()) {
            return true;
        }
        if world
            .characters
            .made
            .get(&guid)
            .is_some_and(|m| m.strings.contains_key(&1) || m.strings.contains_key(&2))
        {
            return true;
        }
        if world.made_outfits.contains_key(&guid)
            || world.made_portrait_head.contains_key(&guid)
            || self.program.people.portrait_head.contains_key(&guid)
            || self.program.people.portrait_head.contains_key(&template)
            || !self.program.people.head_suits.is_empty()
            || !self.program.people.suits.is_empty()
            || !self.program.people.indexed_suits.is_empty()
            || !self.program.people.suit_errors.is_empty()
        {
            return true;
        }
        crate::people::skeleton(&self.program.people, world, guid)
            .is_ok_and(|skel| !skel.bones.is_empty())
    }

    /// Fallback GUID-level `"HEAD"` binding status on `self.world`.
    pub(crate) fn portrait_head_status(&self, guid: u32) -> Option<bool> {
        portrait_head_status_in(self.program, self.world, guid)
    }

    /// Resolves [`Machine::portrait_head_status`] for portrait rendering,
    /// failing loudly if a `Make New Character` pick has an unknown `"HEAD"`
    /// binding.
    pub(crate) fn can_render_portrait(&self, guid: u32, caller: &str) -> Result<bool, Trap> {
        self.portrait_head_status(guid).ok_or_else(|| {
            Trap::Unsupported(format!(
                "{caller} of runtime-made type 0x{guid:08X} (0x4e7824): whether the wardrobe \
                 pick binds HEAD is unknown"
            ))
        })
    }

    /// Whether the winning definition's file for `guid` is writable
    /// (`0x4e8cc6` → `0x4067b3` → `0x4174dc`). Runtime-made characters
    /// (`0x4260e1`) live in newly created loose `Characters/User%05d.iff`
    /// files and are writable even if their template was inside a `.far`
    /// archive.
    pub(crate) fn is_portrait_file_writable(&self, guid: u32) -> bool {
        if self.world.characters.made.contains_key(&guid) {
            return true;
        }
        let Some(def) = crate::interpreter::object_definition(self.catalog, self.world, guid)
        else {
            return false;
        };
        self.program
            .libraries
            .get(&def.resource)
            .is_some_and(|library| !library.read_only)
    }

    /// Runs `0x4e8c5d(index, 1, save_bitmap)` for a `guid` already verified
    /// to be in `0x6f2698`. Returns `true` when the portrait was rendered and
    /// cached, `false` when `0x4e8cc6` skipped a read-only off-lot definition,
    /// `0x4e9652` returned null, or `0x4e725f` failed (`0x4e8d08`, e.g.
    /// missing `"HEAD"` binding at `0x4e7824`).
    pub(crate) fn try_render_ui_effect_portrait(
        &mut self,
        guid: u32,
        save_bitmap: bool,
        caller: &str,
    ) -> Result<bool, Trap> {
        self.render_portrait(guid, save_bitmap, false, 0, caller)
    }

    /// Runs `0x4e8c5d(index, 0x1f, 1)` for `Make New Character` pet creation
    /// (`0x44b126`): bit `0x01` renders/caches/writes `BMP_` 2002, then bits
    /// `0x08` (`0x4e7448`), `0x02` (`0x4e7671`), `0x04` (`0x4e81f6` × 2), and
    /// `0x10` (`0x4e8400`) run 5 additional forced dresses (`0x4e77c6` →
    /// `0x4e7801` → `0x58a178(1)`), which can clear the portrait cache via
    /// `0x58d4e1` → `0x4eb0cc` → `0x4e897e` when the pet's head slot is empty.
    pub(crate) fn try_render_pet_portrait(&mut self, guid: u32) -> Result<bool, Trap> {
        self.render_portrait(guid, true, true, 5, "Make New Character's portrait")
    }

    fn render_portrait(
        &mut self,
        guid: u32,
        save_bitmap: bool,
        defer_unknown_head: bool,
        extra_passes: usize,
        caller: &str,
    ) -> Result<bool, Trap> {
        // 0x4e8c92 -> 0x4e893e -> 0x43ac10: resolve the winning definition.
        let Some(file_key) = self.portrait_file(guid) else {
            return Ok(false);
        };
        // 0x4e9608: reuse the first on-lot person matching `guid` (skips the
        // 0x4e8cc6 writability check; 0x4e8d02 still checks 0x4e725f ->
        // 0x4e7801 -> 0x58a178 -> 0x4e810d on THAT person's live appearance
        // before caching at 0x4e8d14 and persisting BMP_ 2002 at 0x4e8d2e).
        let on_lot_id = self.world.people.iter().copied().find(|id| {
            self.world
                .objects
                .get(id)
                .is_some_and(|object| object.guid == guid)
        });
        if let Some(person_id) = on_lot_id {
            let can_render = match can_render_person_portrait(
                self.catalog,
                self.program,
                self.world,
                person_id,
                guid,
                defer_unknown_head,
                caller,
            )? {
                Some(ok) => ok,
                None => {
                    self.world.ambiguous_portraits.insert(guid);
                    false
                }
            };
            if can_render {
                self.world.portraits.insert(guid);
                self.world.dirty_portraits.remove(&guid);
                if save_bitmap && self.is_portrait_file_writable(guid) {
                    self.world.generated_portrait_2002.insert(file_key);
                }
            }
            // 0x4e8dab..0x4e8fbb: additional flags in `0x4e8c5d` run forced
            // dresses (`0x4e77c6` -> `0x4e7801` -> `0x58a178(1)`).
            for _ in 0..extra_passes {
                crate::outfits::dress_person(
                    self.catalog,
                    self.program,
                    self.world,
                    person_id,
                    true,
                )?;
            }
            return Ok(can_render);
        }
        // 0x4e8cc6 -> 0x4067b3 -> 0x4174dc: off-lot rendering only calls
        // 0x4e9652 when the winning definition's file is writable.
        if !self.is_portrait_file_writable(guid) {
            return Ok(false);
        }
        // 0x4e9652: no on-lot person -> create a temporary instance via
        // 0x440b2d (no household-membership guard in native code).
        let interpreter = self.interpreter();
        let unsupported = |fault| {
            Trap::Unsupported(format!(
                "making a person of type 0x{guid:08X} for {caller} (0x4e9652): {fault}"
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
                    "making an object of type 0x{guid:08X} for {caller} (0x4e9652) changed the \
                     refresh pending, which is not ported"
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
                    "making a person of type 0x{guid:08X} for {caller} (0x4e9652) changed the \
                     refresh pending, which is not ported"
                )));
            }
            cells.keep(&interpreter, self.world, copy);
            return Ok(false);
        };
        let plan = built
            .portrait_put_plan(&mut copy, id, caller)
            .map_err(|fault| fault.trap)?;
        let finish_at = |tile: Option<[i32; 2]>,
                         placed: bool,
                         mut lot: sims_sim::World,
                         run_cells: crate::both_ways::Cells|
         -> Result<(bool, sims_sim::World, crate::both_ways::Cells), Trap> {
            let run = run_cells.on(&interpreter);
            if let Some(pos) = tile {
                run.put_person(&mut lot, id, pos, 1)?;
            }
            let can_render = if !placed {
                false
            } else {
                match can_render_person_portrait(
                    self.catalog,
                    self.program,
                    &mut lot,
                    id,
                    guid,
                    defer_unknown_head,
                    caller,
                )? {
                    Some(ok) => ok,
                    None => {
                        lot.ambiguous_portraits.insert(guid);
                        false
                    }
                }
            };
            let rendered = placed && can_render;
            if rendered {
                // 0x4e8d14 -> 0x4e899e caches the portrait and clears dirty, and
                // 0x4e8d2e -> 0x4e9000 stages BMP_ 2002 BEFORE removing the
                // temporary person at 0x4e8fdb (so a Reset callback during removal
                // can still invalidate the portrait via 0x44e712 -> 0x4e895e,
                // while 0x4e8feb flushes the staged BMP_ 2002 afterwards).
                lot.portraits.insert(guid);
                lot.dirty_portraits.remove(&guid);
                if save_bitmap {
                    lot.generated_portrait_2002.insert(file_key);
                }
            }
            // 0x4e8dab..0x4e8fbb: additional flags in `0x4e8c5d(index, 0x1f, 1)`
            // run forced dresses (`0x4e77c6` -> `0x4e7801` -> `0x58a178(1)`) on the
            // temporary person before removal.
            if placed {
                for _ in 0..extra_passes {
                    crate::outfits::dress_person(self.catalog, self.program, &mut lot, id, true)?;
                }
            }
            // Both normal completion (0x4e8fdb) and no-tile failure (0x4e9706)
            // remove the temporary person immediately and keep its Init/RNG
            // side effects.
            run.remove_now(&mut lot, id).map_err(unsupported)?;
            if !run_cells.same_refresh(&crate::both_ways::Cells::of(&interpreter)) {
                return Err(Trap::Unsupported(format!(
                    "making a person of type 0x{guid:08X} for {caller} (0x4e9652) changed the \
                     refresh pending, which is not ported"
                )));
            }
            Ok((rendered, lot, run_cells))
        };
        let (rendered, final_world, final_cells) = match plan {
            crate::placement::PortraitPutPlan::NoTile => finish_at(None, false, copy, cells)?,
            crate::placement::PortraitPutPlan::FastPlaced => finish_at(None, true, copy, cells)?,
            crate::placement::PortraitPutPlan::Exact(pos) => {
                finish_at(Some(pos), true, copy, cells)?
            }
            crate::placement::PortraitPutPlan::Candidates(positions) => {
                let first = finish_at(Some(positions[0]), true, copy.clone(), cells.copy())?;
                let first_lot = crate::both_ways::LotSnapshot::of(&first.1);
                for &pos in &positions[1..] {
                    let next = finish_at(Some(pos), true, copy.clone(), cells.copy())?;
                    if next.0 != first.0
                        || crate::both_ways::LotSnapshot::of(&next.1) != first_lot
                        || !first.2.same(&next.2)
                    {
                        return Err(Trap::Unsupported(format!(
                            "{caller} puts a person on a tile searched for from the house view's \
                             centre (0x4e96dd, 0x4347f0, 0x4a677c; `Lot::set_view_centre`), and \
                             with no view centre given the placement outcome depends on which tile \
                             the search picks"
                        )));
                    }
                }
                first
            }
        };
        final_cells.keep(&interpreter, self.world, final_world);
        Ok(rendered)
    }
}
