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
        self.ui_effect_portrait_for_guid(guid)
    }

    /// Runs `0x4e92a8` for a known `guid` (`0x47689c`, `0x4363d1`).
    pub(crate) fn ui_effect_portrait_for_guid(&mut self, guid: u32) -> Result<(), Trap> {
        if !self.in_portrait_table(guid)? {
            return Ok(());
        }
        // 0x4e92e5: when the slot's dirty flag ([entry + 0x18], set by
        // 0x4e895e) is set, call 0x4e8c5d(index, 1, 0) first (save flag 0).
        if self.world.dirty_portraits.contains(&guid) {
            let _ = self.try_render_ui_effect_portrait(guid, false)?;
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
            let _ = self.can_render_portrait(guid)?;
        }
        // 0x4e938c: BMP_ 2002 absent or invalid -> call 0x4e8c5d(index, 1, 1)
        // (save flag 1: persists BMP_ 2002 via 0x4e9000 on success when writable).
        let _ = self.try_render_ui_effect_portrait(guid, true)?;
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

    /// Extracts the lowercased suit name from a `STR# 200` outfit row
    /// (`"suit,..."`), returning `None` for empty rows or `"none"`.
    fn extract_suit_name(raw: &str) -> Option<String> {
        let suit = raw
            .split(';')
            .next()
            .unwrap_or(raw)
            .split(',')
            .next()
            .unwrap_or(raw)
            .to_ascii_lowercase();
        (!suit.is_empty() && suit != "none").then_some(suit)
    }

    /// Classifies one `STR# 200` outfit row string against
    /// [`sims_content::People::head_suits`]: `Some(true)` if its suit binds
    /// `"HEAD"` with a valid mesh, `Some(false)` if confirmed not to, or
    /// `None` if an earlier suit decode error hid that suit.
    fn classify_portrait_row(&self, raw: Option<&str>) -> Option<bool> {
        let Some(suit) = raw.and_then(Self::extract_suit_name) else {
            return Some(false);
        };
        if self.program.people.head_suits.contains(&suit) {
            Some(true)
        } else if !self.program.people.suit_errors.is_empty()
            && !self.program.people.suits.contains(&suit)
        {
            None
        } else {
            Some(false)
        }
    }

    /// Whether the renderer (`0x4e725f` → `0x4e7801` → `0x58a178` → `0x4e7817`
    /// → `0x4e810d` → `0x4e81a4` → `0x431726` → `0x4e7824`) finds a
    /// case-sensitive `"HEAD"` binding on the specific Sim `person_id` in
    /// `world`, accounting for:
    /// - whether the Sim's skeleton defines a `"HEAD"` bone (`0x595674`);
    /// - live attached accessories (`person.accessories`, opcode 6);
    /// - live changed outfits (`words[102] & 8`, `person.data[65] & 6`,
    ///   `person.data[8]` at `0x58a178`);
    /// - runtime-made overrides (`Character::strings`, `World::made_outfits`)
    ///   and template `Profile::{body, head}` / `People::portrait_head`.
    fn can_render_person_portrait(
        &self,
        world: &sims_sim::World,
        person_id: u16,
        guid: u32,
        defer_unknown_head: bool,
    ) -> Result<Option<bool>, Trap> {
        // 0x595674..0x595699: a "HEAD" binding attaches only when the Sim's
        // skeleton defines a bone named "HEAD". Synthetic fixtures that leave
        // `bones` empty skip this check.
        if let Ok(skel) = crate::people::skeleton(&self.program.people, world, guid)
            && !skel.bones.is_empty()
            && !skel.bones.iter().any(|b| b.name == "HEAD")
        {
            return Ok(Some(false));
        }

        let person = world
            .objects
            .get(&person_id)
            .and_then(|object| object.person.as_ref());

        // Attached accessories (`person.accessories`, opcode 6) are dressed on
        // the skeleton and scanned by `0x4e810d` alongside body/head suits.
        if person.is_some_and(|p| {
            p.accessories
                .iter()
                .any(|a| !a.name.is_empty() && self.program.people.head_suits.contains(&a.name))
        }) {
            return Ok(Some(true));
        }

        let template = world.characters.definition(guid);
        let profile = self.program.people.profiles.get(&template);
        let made = world.characters.made.get(&guid);
        let made_outfit = world.made_outfits.get(&guid);
        let body_row = made
            .and_then(|m| m.strings.get(&1).map(String::as_str))
            .or_else(|| made_outfit.and_then(|o| o.0.as_deref()))
            .or_else(|| profile.and_then(|p| p.body.as_deref()));
        let head_row = made
            .and_then(|m| m.strings.get(&2).map(String::as_str))
            .or_else(|| made_outfit.and_then(|o| o.1.as_deref()))
            .or_else(|| profile.and_then(|p| p.head.as_deref()));

        let def_flags = self.definition_words(guid).map(|w| w[102]).unwrap_or(0);
        let gender_word = person.and_then(|p| p.data.get(65).copied()).unwrap_or(0);
        let age_word = person.and_then(|p| p.data.get(58).copied()).unwrap_or(0);
        let is_child = age_word != 0 && age_word < 18;
        let raw_suit = person.and_then(|p| p.data.get(8).copied()).unwrap_or(0);

        let suit_row = |idx: usize| -> Option<&str> {
            profile
                .and_then(|p| p.suit_rows.get(&idx))
                .map(String::as_str)
                .filter(|s| !s.is_empty())
        };

        let head_status = if head_row.is_some() {
            self.classify_portrait_row(head_row)
        } else if body_row.is_none() {
            self.portrait_head_status(guid)
        } else {
            Some(false)
        };

        let status = if (def_flags & 8) != 0 && raw_suit > 0 {
            // 0x58a1e4..0x58a255: special NPC flag dresses STR# 200 row
            // `raw_suit` as body, leaving head row untouched.
            let npc_row = match raw_suit as usize {
                1 => body_row,
                2 => head_row,
                n => suit_row(n),
            };
            match (self.classify_portrait_row(npc_row), head_status) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            }
        } else {
            // 0x58a2d6..0x58a2e5: pet species bits (data[65] & 6) wear suit 0.
            let suit = if (gender_word & 6) != 0 { 0 } else { raw_suit };
            match suit {
                0 => {
                    if body_row.is_some() || head_row.is_some() {
                        match (
                            self.classify_portrait_row(body_row),
                            self.classify_portrait_row(head_row),
                        ) {
                            (Some(true), _) | (_, Some(true)) => Some(true),
                            (Some(false), Some(false)) => Some(false),
                            _ => None,
                        }
                    } else {
                        self.portrait_head_status(guid)
                    }
                }
                1 | 2 | 4 | 17 | 18 => {
                    // 0x58d79b: suits 1 (naked), 2 (swim), 4 (formal), 17
                    // (winter), 18 (high fashion) replace the body row from
                    // STR# 200 (falling back to row 1 if empty at 0x58d492)
                    // and keep head row 2 unchanged.
                    if head_status == Some(true) {
                        Some(true)
                    } else {
                        let outfit_body = match suit {
                            1 => suit_row(15),
                            2 => suit_row(31).or_else(|| suit_row(16)),
                            4 => {
                                if is_child {
                                    suit_row(27)
                                } else {
                                    suit_row(30).or_else(|| suit_row(27))
                                }
                            }
                            17 => suit_row(33),
                            18 => suit_row(34),
                            _ => None,
                        }
                        .or(body_row);
                        match (self.classify_portrait_row(outfit_body), head_status) {
                            (Some(true), _) | (_, Some(true)) => Some(true),
                            (Some(false), Some(false)) => Some(false),
                            _ => None,
                        }
                    }
                }
                5 => {
                    // Suit 5 (pajamas): row 32 if non-empty; otherwise matched
                    // pajamas (0x45f034), which is unmodeled here unless head
                    // row 2 already binds "HEAD".
                    if head_status == Some(true) {
                        Some(true)
                    } else if let Some(pj_row) = suit_row(32) {
                        match (self.classify_portrait_row(Some(pj_row)), head_status) {
                            (Some(true), _) | (_, Some(true)) => Some(true),
                            (Some(false), Some(false)) => Some(false),
                            _ => None,
                        }
                    } else {
                        return Err(Trap::Unsupported(format!(
                            "portrait of person {person_id} (GUID 0x{guid:08X}) wearing matched \
                             pajamas (suit 5, 0x58a178 -> 0x45f034): whether the matched body \
                             binds HEAD is not ported"
                        )));
                    }
                }
                3 => {
                    // Suit 3 (work/career, 0x45ee5f): head row 2 is kept.
                    if head_status == Some(true) {
                        Some(true)
                    } else {
                        return Err(Trap::Unsupported(format!(
                            "portrait of person {person_id} (GUID 0x{guid:08X}) wearing work \
                             clothes (suit 3, 0x58a178 -> 0x45ee5f): whether the career body \
                             binds HEAD is not ported"
                        )));
                    }
                }
                other => {
                    // Costumes (6..=16, 19..=60) can replace both body and head.
                    return Err(Trap::Unsupported(format!(
                        "portrait of person {person_id} (GUID 0x{guid:08X}) wearing suit \
                         {other} (0x58a178): whether that costume binds HEAD is not ported"
                    )));
                }
            }
        };

        if status.is_none() && !defer_unknown_head {
            return Err(Trap::Unsupported(format!(
                "UI Effect's portrait of type 0x{guid:08X} (0x4e7824): whether its suit binds \
                 HEAD is unknown"
            )));
        }
        Ok(status)
    }

    /// Fallback GUID-level `"HEAD"` binding status for synthetic fixtures that
    /// populate `People::portrait_head` or `World::made_portrait_head` without
    /// `Profile::{body, head}`.
    pub(crate) fn portrait_head_status(&self, guid: u32) -> Option<bool> {
        if let Some(made) = self.world.characters.made.get(&guid) {
            let mut has_suit_override = false;
            for row in [1usize, 2] {
                if let Some(raw) = made.strings.get(&row) {
                    has_suit_override = true;
                    if Self::extract_suit_name(raw)
                        .is_some_and(|suit| self.program.people.head_suits.contains(&suit))
                    {
                        return Some(true);
                    }
                }
            }
            if has_suit_override {
                return Some(false);
            }
            if let Some(&status) = self.world.made_portrait_head.get(&guid) {
                return status;
            }
            if let Some(&has_head) = self.program.people.portrait_head.get(&guid) {
                return Some(has_head);
            }
            return Some(
                self.program
                    .people
                    .portrait_head
                    .get(&made.template)
                    .copied()
                    .unwrap_or(false),
            );
        }
        Some(
            self.program
                .people
                .portrait_head
                .get(&guid)
                .copied()
                .unwrap_or(false),
        )
    }

    /// Resolves [`Machine::portrait_head_status`] for portrait rendering,
    /// failing loudly if a `Make New Character` pick has an unknown `"HEAD"`
    /// binding.
    pub(crate) fn can_render_portrait(&self, guid: u32) -> Result<bool, Trap> {
        self.portrait_head_status(guid).ok_or_else(|| {
            Trap::Unsupported(format!(
                "UI Effect's portrait of runtime-made type 0x{guid:08X} (0x4e7824): whether the \
                 wardrobe pick binds HEAD is unknown"
            ))
        })
    }

    /// Whether the winning definition's file for `guid` is writable
    /// (`0x4e8cc6` → `0x4067b3` → `0x4174dc`). Runtime-made characters
    /// (`0x4260e1`) live in newly created loose `Characters/User%05d.iff`
    /// files and are writable even if their template was inside a `.far`
    /// archive.
    fn is_portrait_file_writable(&self, guid: u32) -> bool {
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
    ) -> Result<bool, Trap> {
        self.render_portrait(guid, save_bitmap, false)
    }

    /// Runs `0x4e8c5d(index, 0x1f, 1)` for `Make New Character` pet creation
    /// (`0x44b126`): preserves all Init/CRT RNG/removal side effects and
    /// caches/writes `BMP_` 2002 when `HEAD` binding is known, without turning
    /// a mixed custom pet wardrobe into a fault inside opcode 19 (deferring
    /// any trap until `UI Effect` or `Dialog` actually queries the portrait).
    pub(crate) fn try_render_pet_portrait(&mut self, guid: u32) -> Result<bool, Trap> {
        self.render_portrait(guid, true, true)
    }

    fn render_portrait(
        &mut self,
        guid: u32,
        save_bitmap: bool,
        defer_unknown_head: bool,
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
            let can_render = match self.can_render_person_portrait(
                self.world,
                person_id,
                guid,
                defer_unknown_head,
            )? {
                Some(ok) => ok,
                None => {
                    self.world.ambiguous_portraits.insert(guid);
                    false
                }
            };
            if !can_render {
                return Ok(false);
            }
            self.world.portraits.insert(guid);
            self.world.dirty_portraits.remove(&guid);
            if save_bitmap && self.is_portrait_file_writable(guid) {
                self.world.generated_portrait_2002.insert(file_key);
            }
            return Ok(true);
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
        let can_render = if !placed {
            false
        } else {
            match self.can_render_person_portrait(&copy, id, guid, defer_unknown_head)? {
                Some(ok) => ok,
                None => {
                    copy.ambiguous_portraits.insert(guid);
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
            copy.portraits.insert(guid);
            copy.dirty_portraits.remove(&guid);
            if save_bitmap {
                copy.generated_portrait_2002.insert(file_key);
            }
        }
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
        Ok(rendered)
    }
}
