//! Buy mode: the catalog, the held object, undo and redo, and the modifier keys.

use super::*;

impl Game {
    /// Steps the active Buy or Build catalog grid page by `delta` (`+1` for
    /// `X`, `-1` for `Z`, `0x4d11b5` / `0x4d1224`).
    pub fn page_catalog(&mut self, delta: i32) -> bool {
        match self.hud.controls.mode {
            Mode::Buy => self.page_buy(delta),
            Mode::Build => self.page_build(delta),
            _ => false,
        }
    }

    /// The Buy Mode catalog master GUID currently armed for placement, if any.
    pub fn buy_selected(&self) -> Option<u32> {
        self.buy_selected
    }

    /// The active Buy Mode placement cursor (once the pointer has entered
    /// the house view or picked up an existing object), if any.
    pub fn buy_cursor(&self) -> Option<&BuyCursor> {
        self.buy_cursor.as_ref()
    }

    /// The Buy Mode catalog strip state (sort, category, subcategory, page,
    /// selected GUID).
    pub fn buy_state(&self) -> sims_ui::buy::BuyState {
        self.hud.buy_state
    }

    /// The Buy Mode catalog strip layout as last built.
    pub fn buy_layout(&self) -> &sims_ui::buy::BuyLayout {
        self.hud.buy_layout()
    }

    /// Computes sell prompt info (`0x487d7a` → `0x4cf90b` → `0x50a343`) when
    /// the Hand Tool is currently holding a picked-up existing lot object.
    pub(super) fn buy_sell_info(&mut self) -> Option<sims_ui::buy::SellInfo> {
        let cursor = self.buy_cursor.as_ref()?;
        if !cursor.is_existing() {
            return None;
        }
        let held = cursor.held;
        let guid = cursor.guid;
        let (icon_guid, name) = self
            .hud
            .ensure_buy_catalog(&self.assets)
            .display_for_object(guid);
        let refund = self.lot.resale_value(held).unwrap_or(0);
        let can_delete = self.lot.can_delete(held).unwrap_or(false);
        Some(sims_ui::buy::SellInfo {
            guid: icon_guid,
            name,
            refund,
            can_delete,
        })
    }

    /// Rebuilds the cached Buy Mode strip layout (catalog or active Hand Tool
    /// sell prompt) for the current viewport size and world state.
    pub(super) fn layout_buy(&mut self) {
        let sell = self.buy_sell_info();
        self.hud.layout_buy_for_lot(
            self.view.size,
            self.lot.world(),
            self.lot.lot_zoning(),
            &self.assets,
            sell.as_ref(),
        );
    }

    /// Toggles the Buy Mode catalog sort order between `ByFunction` and
    /// `ByRoom` (`0x53a2c3` → `0x4cf3af`), cancelling any active cursor and
    /// returning to the top-level categories.
    pub fn toggle_buy_sort(&mut self) -> bool {
        if self.hud.controls.mode != Mode::Buy {
            return false;
        }
        self.cancel_buy();
        if self.buy_cursor.is_some() {
            return false;
        }
        let changed = self.hud.buy_state.apply(&Command::BuyToggleSort);
        self.layout_buy();
        changed
    }

    /// Steps the Buy Mode catalog grid page by `delta` (`+1` for `x`, `-1` for
    /// `z`, `0x4d0ef0`). Returns `true` if the displayed page changed.
    pub fn page_buy(&mut self, delta: i32) -> bool {
        if self.hud.controls.mode != Mode::Buy
            || !self.hud.controls.expanded
            || self.hud.buy_state.category.is_none()
        {
            return false;
        }
        let before = self.hud.buy_state.page;
        if !self.hud.buy_state.apply(&Command::BuyPage(delta)) {
            return false;
        }
        self.layout_buy();
        self.hud.buy_state.page != before
    }

    /// Sells or deletes the currently held Buy Mode object (`Delete` /
    /// `Backspace` key `0x486b5b` or clicking/dropping onto the catalog strip
    /// `0x50d04d` → `0x4cf97a` → `0x487e18`). Returns `true` if a Buy cursor
    /// was active and handled.
    pub fn sell_held(&mut self) -> bool {
        if !matches!(self.hud.controls.mode, Mode::Buy | Mode::Build) || self.stopped.is_some() {
            return false;
        }
        if self.coop.is_active() {
            let Some(preview) = self.coop_buy_preview.take() else {
                return false;
            };
            self.buy_dragging = None;
            self.buy_commit_on_release = false;
            self.buy_in_house = false;
            self.buy_selected = None;
            self.hud.buy_state.selected = None;
            if let Some(object_id) = preview.existing_object_id {
                if let Err(e) = self
                    .coop
                    .submit_command(NetCommand::BuyBuild(BuyBuildEdit::SellObject { object_id }))
                {
                    self.note = Some(format!("Could not send Sell command: {e}"));
                    self.hud.play(crate::hud::ERROR);
                } else {
                    self.hud.play(crate::hud::MONEYBACK);
                }
            }
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return true;
        }
        let Some(cursor) = self.buy_cursor.take() else {
            return false;
        };
        let was_existing = cursor.is_existing();
        let refund = if was_existing {
            self.lot.resale_value(cursor.held).unwrap_or(0)
        } else {
            0
        };
        match self.lot.buy_sell(&cursor) {
            Ok(true) => {
                if was_existing {
                    self.dirty = true;
                    // Native `0x487e18` -> `0x492ad4` (`UI_moneyback`) /
                    // `0x492b05` (`UI_archtool_delete`) with 400 ms cooldown,
                    // keyed off `resale_value` even when unlimited budget
                    // (`global[32] != 0`) keeps household funds flat.
                    if refund != 0 {
                        if self
                            .buy_moneyback_us
                            .is_none_or(|t| self.clock_us.saturating_sub(t) > 400_000)
                        {
                            self.hud.play(crate::hud::MONEYBACK);
                            self.buy_moneyback_us = Some(self.clock_us);
                        }
                    } else if self
                        .buy_delete_us
                        .is_none_or(|t| self.clock_us.saturating_sub(t) > 400_000)
                    {
                        self.hud.play(crate::hud::ARCHTOOL_DELETE);
                        self.buy_delete_us = Some(self.clock_us);
                    }
                }
                self.buy_dragging = None;
                self.buy_grab_offset = [0, 0];
                self.buy_commit_on_release = false;
                self.buy_in_house = false;
                self.buy_selected = None;
                self.hud.buy_state.selected = None;
                self.refresh_world();
                self.layout_buy();
            }
            Ok(false) => {
                self.buy_cursor = Some(cursor);
                self.hud.play(crate::hud::ERROR);
            }
            Err(e) => {
                self.buy_cursor = Some(cursor);
                self.note = Some(format!("The object was not sold: {e}"));
                self.hud.play(crate::hud::ERROR);
            }
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        true
    }

    /// Whether a committed Buy or Build Mode action can be undone (`0x4cf074`).
    pub fn can_undo(&self) -> bool {
        match self.hud.controls.mode {
            Mode::Buy => self.lot.can_buy_undo(),
            Mode::Build => self.lot.can_undo(),
            _ => false,
        }
    }

    /// Whether an undone Buy or Build Mode action can be redone (`0x4cf08c`).
    pub fn can_redo(&self) -> bool {
        match self.hud.controls.mode {
            Mode::Buy => self.lot.can_buy_redo(),
            Mode::Build => self.lot.can_redo(),
            _ => false,
        }
    }

    /// Runs a Buy/Build Mode Undo (`undo = true`) or Redo (`undo = false`). If the
    /// Hand Tool currently holds a picked-up existing lot object, snaps it
    /// back and clears cursor ownership first (without clearing the redo
    /// stack) so the Hand Tool never conflicts with or double-refunds an
    /// undone/redone object; unpaid new catalog previews stay armed
    /// (`0x4a8545`, `0x4a856a`).
    pub(super) fn apply_buy_history(&mut self, undo: bool) -> bool {
        if self.coop.is_active() {
            self.note = Some("Undo/Redo is disabled during co-op multiplayer sessions.".into());
            return false;
        }
        let allowed = if undo {
            self.can_undo()
        } else {
            self.can_redo()
        };
        if !allowed || self.stopped.is_some() {
            return false;
        }
        self.build_drag = None;
        self.build_preview_shape = None;
        let snapped_existing = if self.buy_cursor.as_ref().is_some_and(BuyCursor::is_existing) {
            let cursor = self.buy_cursor.take().unwrap();
            match self.lot.buy_snap_back(&cursor) {
                Ok(()) => {
                    self.buy_dragging = None;
                    self.buy_grab_offset = [0, 0];
                    self.buy_commit_on_release = false;
                    self.buy_in_house = false;
                    true
                }
                Err(e) => {
                    self.buy_cursor = Some(cursor);
                    self.note = Some(format!("The held object could not be released: {e}"));
                    return false;
                }
            }
        } else {
            false
        };
        let result = if self.hud.controls.mode == Mode::Build {
            if undo {
                self.lot.undo()
            } else {
                self.lot.redo()
            }
        } else if undo {
            self.lot.buy_undo()
        } else {
            self.lot.buy_redo()
        };
        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        if let Some(cursor) = self.buy_cursor.as_mut() {
            let held_on_lot = self
                .lot
                .world()
                .objects
                .get(&cursor.held)
                .is_some_and(|o| o.position[0] >= 0);
            let affordable =
                cursor.price <= 0 || cursor.price <= sims_vm::available_funds(self.lot.world());
            cursor.valid = held_on_lot && affordable;
        }
        match result {
            Ok(true) => {
                self.dirty = true;
                self.refresh_world();
                if self.hud.controls.mode == Mode::Build {
                    self.sync_build_roof_ui();
                    self.layout_build();
                } else {
                    self.layout_buy();
                }
                true
            }
            Ok(false) => {
                self.refresh_world();
                if self.hud.controls.mode == Mode::Build {
                    self.layout_build();
                } else if snapped_existing {
                    self.layout_buy();
                }
                false
            }
            Err(e) => {
                let verb = if undo { "undone" } else { "redone" };
                self.note = Some(format!("The action was not {verb}: {e}"));
                self.refresh_world();
                if self.hud.controls.mode == Mode::Build {
                    self.layout_build();
                } else if snapped_existing {
                    self.layout_buy();
                }
                false
            }
        }
    }

    /// Undoes the most recent committed Buy/Build Mode action (`0x4cf2e0` /
    /// event `0xec` → `0x4932fa`). Returns `true` if an action was undone.
    pub fn undo_buy(&mut self) -> bool {
        self.apply_buy_history(true)
    }

    /// Redoes the most recently undone Buy/Build Mode action (`0x4cf317` /
    /// event `0xed` → `0x493338`). Returns `true` if an action was redone.
    pub fn redo_buy(&mut self) -> bool {
        self.apply_buy_history(false)
    }

    /// Sets whether the Shift modifier is held (`VK_SHIFT`, `0x4864f9`),
    /// which keeps the Buy tool armed for repeat purchases after placing any
    /// affordable item (`0x48680f`) and toggles Build Mode Room-Box / Room-Fill.
    pub fn set_shift(&mut self, held: bool) {
        if self.shift_held == held {
            return;
        }
        self.shift_held = held;
        if self.hud.controls.mode == Mode::Build
            && let Some(pt) = self.pointer
            && !self.hud.covers(pt)
        {
            self.hover_build(pt);
        }
    }

    /// Sets whether the Ctrl modifier is held (`VK_CONTROL`), which switches
    /// Build Mode Wall/Floor/Wallpaper tools to Demolish / Remove / Strip.
    pub fn set_ctrl(&mut self, held: bool) {
        if self.ctrl_held == held {
            return;
        }
        self.ctrl_held = held;
        if self.hud.controls.mode == Mode::Build
            && let Some(pt) = self.pointer
            && !self.hud.covers(pt)
        {
            self.hover_build(pt);
        }
    }

    /// Switches between Live, Buy, Build, and Options modes (`0x4cea5d`,
    /// `0x4ce44f`). Cancels any unplaced Buy Mode item, closes open pie menus,
    /// updates `global[19]` (calling `go_live` `0x416534` when returning to
    /// Live mode), and refreshes the control panel and Buy catalog strip.
    pub fn set_mode(&mut self, mode: Mode) {
        self.hud.close_camera_strip();
        // `0x4f2482..0x4f248b` -> `0x53b233..0x53b254`: edit-mode lots lock
        // out Live mode while `[lot + 0x50] == 1`.
        if mode == Mode::Live && self.edit_mode {
            return;
        }
        // Native `0x4cea5d`: pressing a mode shortcut for the already-active
        // mode (e.g. F2 in Buy Mode, F1 in Live Mode) is a no-op and preserves
        // the open catalog state and any active Hand Tool pickup; in Options
        // mode, re-clicking while collapsed expands the strip (`0x53a51b`).
        if self.hud.controls.mode == mode && (mode != Mode::Options || self.hud.controls.expanded) {
            return;
        }
        // Native `0x4cea5d` runs `0x416534` (`go_live`), `0x40726a`
        // (`update_comeandsee`), and `0x4933dc` (`buy_purge_detached`) BEFORE
        // tearing down the Buy tool's active cursor (`0x4cec3a` -> `0x4cef9b`
        // -> `0x487191` / `0x487105`). Execute all four steps atomically so
        // marker IDs match native and any snapback refusal leaves the mode and
        // held cursor intact.
        if self.coop.is_active() {
            if let Some(net_mode) = match mode {
                Mode::Live => Some(NetPlayerMode::Live),
                Mode::Buy => Some(NetPlayerMode::Buy),
                Mode::Build => Some(NetPlayerMode::Build),
                Mode::Options => None,
            } && let Err(e) = self
                .coop
                .submit_command(NetCommand::SetMode { mode: net_mode })
            {
                self.note = Some(format!("The mode was not changed: {e}"));
                return;
            }
            self.coop_buy_preview = None;
        } else if let Err(e) = self
            .lot
            .set_mode_releasing_buy_cursor(panel_mode_for(mode), self.buy_cursor.as_ref())
        {
            self.note = Some(format!("The mode was not changed: {e}"));
            return;
        }
        self.buy_cursor = None;
        self.buy_dragging = None;
        self.buy_grab_offset = [0, 0];
        self.buy_commit_on_release = false;
        self.buy_in_house = false;
        self.buy_selected = None;
        self.wall_cursor = None;
        self.build_drag = None;
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
        self.hud.buy_state.selected = None;
        self.close_menu();
        self.reached = None;
        self.listed = None;
        self.highlighted = None;
        if self.hud.controls.mode == Mode::Live && mode != Mode::Live {
            self.live_panel_expanded = self.hud.controls.expanded;
        }
        let prev_mode = self.hud.controls.mode;
        let was_expanded = self.hud.controls.expanded;
        let next_expanded = if mode == Mode::Live {
            self.live_panel_expanded
        } else {
            true
        };
        if mode == Mode::Options && (prev_mode != Mode::Options || !was_expanded) {
            self.hud.controls.options.tab = None;
        }
        self.hud.controls.mode = mode;
        self.hud.controls.expanded = next_expanded;
        if was_expanded != next_expanded {
            self.sync_view_centre();
        }
        self.hud.controls.help = None;
        self.hud.controls.options.help = None;
        self.hud.buy_state.reset();
        self.hud.build_state.reset();
        // Native `0x487191` clears the active selection/cursor while retaining
        // `buy_last_guid` (`tool[0x12]`) and `buy_facing` (`tool[0x10]`).
        self.buy_in_house = false;
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
        self.refresh_world();
        self.hud.relayout_panel(
            self.view.size,
            self.lot.world(),
            self.lot.residential(),
            self.lot.lot_zoning(),
            self.lot_visibility(),
            &self.assets,
            None,
        );
        if mode == Mode::Build {
            self.sync_build_roof_ui();
            self.layout_build();
        }
    }

    /// Arms catalog master `guid` on the Buy Mode placement tool (`0x5065c8`).
    /// Does not create the object until the pointer enters the house view
    /// (`0x487d3d`). Plays `UI_error` if the household cannot afford it.
    pub(super) fn select_buy_item(&mut self, guid: u32) {
        self.cancel_buy();
        if self.buy_cursor.is_some() {
            return;
        }
        let price = self
            .hud
            .ensure_buy_catalog(&self.assets)
            .get(guid)
            .map(|item| item.price);
        let funds = sims_vm::available_funds(self.lot.world());
        match price {
            Some(price) if price <= 0 || price <= funds => {
                self.buy_selected = Some(guid);
                self.hud.buy_state.selected = Some(guid);
            }
            Some(_) => {
                self.hud.play(crate::hud::ERROR);
                self.buy_selected = None;
                self.hud.buy_state.selected = None;
            }
            None => {
                self.buy_selected = None;
                self.hud.buy_state.selected = None;
            }
        }
        self.layout_buy();
    }

    /// Cancels any active Buy Mode cursor or armed catalog selection
    /// (`0x487105`), deleting the unplaced object and `"Tile Cursor"` without
    /// charging funds. Returns `true` if anything was cancelled. If deleting
    /// the preview fails, the cursor handle is kept so it can be retried.
    pub fn cancel_buy(&mut self) -> bool {
        self.buy_dragging = None;
        self.buy_grab_offset = [0, 0];
        self.buy_commit_on_release = false;
        self.buy_in_house = false;
        let had_coop_preview = self.coop_buy_preview.take().is_some();
        let had_cursor = if let Some(cursor) = self.buy_cursor.take() {
            match self.lot.buy_cancel(&cursor) {
                Ok(()) => {
                    self.refresh_world();
                    true
                }
                Err(e) => {
                    self.buy_cursor = Some(cursor);
                    self.note = Some(format!("The Buy cursor was not cancelled: {e}"));
                    return false;
                }
            }
        } else {
            false
        };
        let had_build_drag = self.build_drag.take().is_some();
        if had_build_drag {
            self.build_preview_shape = None;
        }
        let had_selection = self.buy_selected.take().is_some();
        let had_hud = self.hud.buy_state.selected.take().is_some();
        if had_selection || had_hud || had_cursor || had_build_drag || had_coop_preview {
            if self.hud.controls.mode == Mode::Buy {
                self.layout_buy();
            } else if self.hud.controls.mode == Mode::Build {
                self.layout_build();
            }
            true
        } else {
            false
        }
    }

    /// Rotates the held Buy Mode object by one signed turn step (`0x486cb8` →
    /// `0x421be4(±1)`), returning `true` if a Buy cursor was active.
    pub fn rotate_held(&mut self, clockwise: bool) -> bool {
        if self.coop.is_active() {
            let Some(preview) = self.coop_buy_preview.as_mut() else {
                return false;
            };
            let delta = if clockwise { 2 } else { -2 };
            preview.facing = (preview.facing + delta).rem_euclid(8);
            self.buy_facing = Some(preview.facing);
            return true;
        }
        let Some(mut cursor) = self.buy_cursor.take() else {
            return false;
        };
        let steps = if clockwise { 1 } else { -1 };
        if let Err(e) = self.lot.buy_turn(&mut cursor, steps) {
            self.note = Some(format!("The object was not rotated: {e}"));
        } else {
            self.refresh_world();
        }
        self.buy_cursor = Some(cursor);
        true
    }

    /// Handles pointer movement over the house view in Buy Mode (`0x487d3d`,
    /// `0x485a2e`): on mouse entry (`!self.buy_in_house`), recreates the
    /// armed catalog preview at the tool's stored facing (`0x487d3d` →
    /// `0x486f5a`); then rechecks the previous cursor position (`0x485bda`),
    /// rotates the held object in place while dragging past the 6 px
    /// threshold (`dx^2 + dy^2 > 36`), and otherwise moves the cursor to the
    /// terrain-unprojected tile (including within the 6 px drag radius).
    pub(super) fn hover_buy(&mut self, point: [i32; 2]) -> bool {
        if self.stopped.is_some() {
            return false;
        }
        if self.coop.is_active() {
            return self.hover_buy_coop(point);
        }
        if !self.buy_in_house {
            self.buy_in_house = true;
            if let Some(guid) = self.buy_selected {
                // Native `0x487d3d` → `0x486f5a`: destroy any hidden previous
                // preview (`0x487105`) and instantiate a fresh one using the
                // tool's stored facing (`tool[0x10]`), resetting it to
                // `default_buy_direction` only when switching catalog items.
                if let Some(old) = self.buy_cursor.take()
                    && let Err(e) = self.lot.buy_cancel(&old)
                {
                    self.buy_cursor = Some(old);
                    self.buy_in_house = false;
                    self.note = Some(format!("Could not reset Buy preview: {e}"));
                    return true;
                }
                if self.buy_last_guid != Some(guid) {
                    self.buy_facing =
                        Some(sims_vm::default_buy_direction(self.view.camera.rotation));
                    self.buy_last_guid = Some(guid);
                }
                let dir = self
                    .buy_facing
                    .unwrap_or_else(|| sims_vm::default_buy_direction(self.view.camera.rotation));
                match self.lot.buy_start(guid, self.view.camera.level, dir) {
                    Ok(Some(cursor)) => self.buy_cursor = Some(cursor),
                    Ok(None) => {
                        self.hud.play(crate::hud::ERROR);
                        self.buy_selected = None;
                        self.hud.buy_state.selected = None;
                        self.layout_buy();
                        return true;
                    }
                    Err(e) => {
                        self.note = Some(format!("Could not start placing 0x{guid:08X}: {e}"));
                        return true;
                    }
                }
            }
        }
        let Some(mut cursor) = self.buy_cursor.take() else {
            return false;
        };
        // Native `0x485b4c..0x485bda`: ALWAYS recheck the `"Tile Cursor"`'s
        // physical on-lot position on the currently displayed floor
        // (`0x485ba2`) BEFORE checking the drag threshold.
        let level = self.view.camera.level;
        if let Err(e) = self.lot.buy_recheck(&mut cursor, level) {
            self.note = Some(format!("Could not recheck Buy placement: {e}"));
        }
        let rotated = if let Some(anchor) = self.buy_dragging {
            let dx = point[0] - anchor[0];
            let dy = point[1] - anchor[1];
            if dx * dx + dy * dy > 36 {
                // Native `0x485d5d..0x485dad`: `iVar6 = dy`, `iVar8 = dx`.
                // Upper-left -> 6, upper-right -> 4, lower-left -> 0,
                // lower-right -> 2. Compare `desired` against post-`0x485bda`
                // current facing FIRST (`0x485da9`), and only if they differ
                // subtract the OBJD `+0x7a` facing offset (`0x434f8d`) before
                // turning.
                let quad = if dy < 0 {
                    if dx < 0 { 6 } else { 4 }
                } else if dx < 0 {
                    0
                } else {
                    2
                };
                let desired = (quad - 2 * i16::from(self.view.camera.rotation & 3)).rem_euclid(8);
                let current = self
                    .lot
                    .world()
                    .objects
                    .get(&cursor.held)
                    .and_then(|o| o.data.get(1).copied())
                    .unwrap_or(0)
                    .rem_euclid(8);
                if current != desired {
                    let target = (desired - cursor.facing_offset).rem_euclid(8);
                    if let Err(e) = self.lot.buy_rotate(&mut cursor, target) {
                        self.note = Some(format!("The object was not rotated: {e}"));
                    }
                }
                // Native `0x485e59` → `0x485e72`: test Tile Cursor fit on the
                // existing preview in place on the displayed floor (without an
                // extra TakeOff) and place if it fits, including when the
                // facing was unchanged.
                if let Err(e) = self.lot.buy_put_if_fits(&mut cursor, level) {
                    self.note = Some(format!("Could not finalize Buy placement: {e}"));
                }
                true
            } else {
                false
            }
        } else {
            false
        };
        if !rotated {
            let grab = if cursor.is_existing() {
                self.buy_grab_offset
            } else {
                [0, 0]
            };
            let at = [point[0] - grab[0] - pick::ORIGIN_COLUMN, point[1] - grab[1]];
            let fine = sims_render::unproject_terrain(&self.lot_shape, &self.view.camera, at)
                .unwrap_or_else(|| self.view.camera.unproject_fine(at));
            if let Err(e) = self.lot.buy_move(&mut cursor, fine, self.view.camera.level) {
                self.note = Some(format!("Could not move Buy cursor: {e}"));
            }
        }
        self.buy_cursor = Some(cursor);
        self.refresh_world();
        true
    }

    /// Hides the active Buy Mode cursor off the lot on mouse exit (`0x487d4e`).
    pub(super) fn hide_buy_cursor(&mut self) -> bool {
        let was_in = std::mem::replace(&mut self.buy_in_house, false);
        if !was_in {
            return false;
        }
        let Some(mut cursor) = self.buy_cursor.take() else {
            return false;
        };
        if let Err(e) = self.lot.buy_hide(&mut cursor) {
            self.note = Some(format!("Could not hide Buy cursor: {e}"));
        }
        self.buy_cursor = Some(cursor);
        self.refresh_world();
        true
    }

    /// Commits a Buy Mode placement on mouse release (`0x4864d0` → `0x486ada`
    /// → `0x421a0e`, `0x4929fb`). For repeat-buy items (`cursor.keep_buying`,
    /// OBJD word 96 == 1 outside tutorial), immediately starts another copy
    /// with the placed item's facing (`0x4864d0` → `0x486f5a`) if affordable.
    pub(super) fn commit_buy(&mut self, point: [i32; 2]) -> Outcome {
        if self.coop.is_active() {
            return self.commit_buy_coop();
        }
        let Some(cursor) = self.buy_cursor.take() else {
            return Outcome::default();
        };
        if !cursor.valid {
            self.buy_cursor = Some(cursor);
            self.hud.play(crate::hud::ERROR);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return Outcome::redraw();
        }
        let last_facing = self
            .lot
            .world()
            .objects
            .get(&cursor.held)
            .and_then(|o| o.data.get(1).copied())
            .unwrap_or_else(|| sims_vm::default_buy_direction(self.view.camera.rotation));
        match self.lot.buy_commit(&cursor) {
            Ok(_) => {
                self.buy_grab_offset = [0, 0];
                self.buy_commit_on_release = false;
                self.dirty = true;
                // Native `0x486ada` line `0x486af6`: remember the committed
                // facing on the tool (`tool[0x10]`).
                self.buy_facing = Some(last_facing);
                self.buy_last_guid = Some(cursor.guid);
                // Native `0x4864d0` line `0x4867e8`: repeat-buy (`keep_buying`
                // or `VK_SHIFT`) runs only for newly bought catalog objects
                // (`+0x75 != 0`), never when dropping a moved existing object.
                if !cursor.is_existing() && (cursor.keep_buying || self.shift_held) {
                    let funds = sims_vm::available_funds(self.lot.world());
                    if cursor.price <= 0 || cursor.price <= funds {
                        self.buy_selected = Some(cursor.guid);
                        self.hud.buy_state.selected = Some(cursor.guid);
                        match self
                            .lot
                            .buy_start(cursor.guid, self.view.camera.level, last_facing)
                        {
                            Ok(Some(next)) => {
                                self.buy_cursor = Some(next);
                                self.buy_in_house = !self.hud.covers(point);
                                self.refresh_world();
                                if !self.hud.covers(point) {
                                    self.hover_buy(point);
                                }
                            }
                            Ok(None) => {
                                self.hud.play(crate::hud::ERROR);
                                self.buy_selected = None;
                                self.hud.buy_state.selected = None;
                                self.refresh_world();
                            }
                            Err(e) => {
                                self.note = Some(format!(
                                    "Could not start repeat purchase 0x{:08X}: {e}",
                                    cursor.guid
                                ));
                                self.buy_selected = None;
                                self.hud.buy_state.selected = None;
                                self.refresh_world();
                            }
                        }
                    } else {
                        self.hud.play(crate::hud::ERROR);
                        self.buy_selected = None;
                        self.hud.buy_state.selected = None;
                        self.refresh_world();
                    }
                } else {
                    self.buy_selected = None;
                    self.hud.buy_state.selected = None;
                    self.refresh_world();
                }
                if self.hud.controls.mode == Mode::Build {
                    self.layout_build();
                } else {
                    self.layout_buy();
                }
            }
            Err(e) => {
                // `Lot::buy_commit` rolled back the world; keep the cursor
                // handle so the preview objects remain manageable.
                self.buy_cursor = Some(cursor);
                self.note = Some(format!("The item was not placed: {e}"));
                self.hud.play(crate::hud::ERROR);
            }
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }
}
