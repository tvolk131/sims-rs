//! Options, dialogs and their text entry, edge scrolling and the mode buttons' hover.

use super::*;

impl Game {
    /// The Options panel's current state (active tab, graphics, sound, and play settings).
    pub fn options(&self) -> &OptionsState {
        &self.hud.controls.options
    }

    /// Whether the lot has unsaved changes (`[app + 0xf7]`, checked by Quit at
    /// `0x516809` and Neighborhood at `0x5170ff`).
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Whether a UI-owned modal confirmation dialog is currently open.
    pub fn has_ui_dialog(&self) -> bool {
        self.hud.has_ui_dialog()
    }

    /// Whether any modal dialog (UI confirmation, simulation prompt, or special
    /// panel `Dialog` kinds `5..=15`) is open.
    pub fn has_dialog(&self) -> bool {
        self.hud.has_dialog()
            || self.book_dialog.is_open()
            || self.phonebook_dialog.is_open()
            || self.destination_picker.is_some()
            || self.lot.asking().is_some()
            || self.lot.asking_special_panel().is_some()
            || self.active_special_panel.is_some()
    }

    /// Loads persisted Options preferences from an `Options.iff` (`Optn` 128)
    /// file (`0x4e6505..0x4e685e`), applying Free Will to the lot's global 30.
    pub fn load_options(&mut self, path: &Path) -> Result<(), String> {
        let mut loaded = OptionsState::load_iff(path).map_err(|e| e.to_string())?;
        loaded.tab = self.hud.controls.options.tab;
        loaded.help = self.hud.controls.options.help;
        let free_will = loaded.play.free_will;
        self.hud.controls.options = loaded;
        if self.lot.world().globals.get(30).copied().unwrap_or(1) != i16::from(free_will) {
            let _ = self.lot.set_free_will(free_will);
        }
        Ok(())
    }

    /// Saves the current Options preferences to `path` as an `Options.iff`
    /// (`Optn` 128) file (`0x4e6505..0x4e685e`), making its folder if it
    /// has none yet.
    pub fn save_options(&self, path: &Path) -> Result<(), String> {
        if let Some(folder) = path.parent().filter(|f| !f.as_os_str().is_empty()) {
            std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        self.hud
            .controls
            .options
            .save_iff(path)
            .map_err(|e| e.to_string())
    }

    /// The lot's editable name shown in the edit-mode Save Card (`cWinEditLotInfo`,
    /// `0x50e1f8..0x50e42c`, persisted in [`Saved`]).
    pub fn lot_name(&self) -> &str {
        self.lot.lot_name()
    }

    /// The lot's editable description shown in the edit-mode Save Card
    /// (`cWinEditLotInfo`, `0x50e1f8..0x50e42c`, persisted in [`Saved`]).
    pub fn lot_description(&self) -> &str {
        self.lot.lot_description()
    }

    /// The text currently in the active modal dialog's secondary
    /// `cTSWinTextEdit2` field (Lot Description on the edit-mode Save Card).
    pub fn dialog_secondary_entry(&self) -> Option<&str> {
        self.hud.active_dialog_secondary_entry()
    }

    /// Switches keyboard focus between Lot Name and Lot Description on an open
    /// edit-mode Save Card (`Tab`).
    pub fn dialog_switch_field(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.switch_dialog_field()
    }

    /// Whether pressing Return/Enter in the active dialog inserts a newline
    /// into a multiline field (Lot Description on the edit-mode Save Card,
    /// `0x50db98..0x50dba0`, `0x5f28b6..0x5f28f9`) instead of submitting.
    pub fn dialog_enter_is_newline(&self) -> bool {
        !self.ui_modal_in_front() && self.hud.is_multiline_entry()
    }

    /// Handles a native modal confirmation key (`Enter`, `Escape`, or localized
    /// Yes/No from `STR# 161`, `0x5e5b3b..0x5e5bfd`, `0x6d7530`).
    pub fn dialog_confirm_key(&mut self, key: DialogKey) -> Outcome {
        if self.phonebook_dialog.is_open() {
            return match key {
                DialogKey::Enter => self.phonebook_submit(),
                DialogKey::Escape => self.phonebook_escape(),
                _ => Outcome::default(),
            };
        }
        if self.book_dialog.is_open() {
            return match key {
                DialogKey::Enter | DialogKey::Escape => self.book_close(),
                _ => Outcome::default(),
            };
        }
        // Special-panel pickers sit in front of any VM prompt: their keys
        // never reach it.
        if self.shop_dialog.is_open() {
            return match key {
                DialogKey::Enter => self.shop_submit(),
                DialogKey::Escape => self.shop_escape(),
                _ => Outcome::default(),
            };
        }
        if self.active_special_panel.is_some() || self.destination_picker.is_some() {
            return Outcome::default();
        }
        if key == DialogKey::Tab {
            return Outcome {
                redraw: self.hud.switch_dialog_field(),
                ..Outcome::default()
            };
        }
        if key == DialogKey::Enter && self.hud.is_multiline_entry() {
            return Outcome {
                redraw: self.hud.type_dialog_char('\n'),
                ..Outcome::default()
            };
        }
        let Some(command) = self.hud.confirm_key(key) else {
            return Outcome::default();
        };
        let outcome = self.command(command, None);
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        outcome
    }

    /// Called by the window host when a save requested after edit-mode
    /// `SaveCardQuit` failed: if `ConfirmQuit` was queued while `dirty` is
    /// still set, upgrades it to `ConfirmSaveBeforeQuit` (`0x516821 ->
    /// 0x516866 -> 0x4f290d` with `dirty == true`).
    pub fn after_save_failed(&mut self) {
        if self.dirty && self.hud.ui_dialog_kind() == Some(OptionsDialog::ConfirmQuit) {
            let name = self.lot.lot_name().to_owned();
            let desc = self.lot.lot_description().to_owned();
            self.hud.open_options_dialog(
                OptionsDialog::ConfirmSaveBeforeQuit,
                self.view.size,
                &name,
                &desc,
            );
        }
    }

    /// Called by the window host when a save failed and a user-facing notice
    /// should explain the write failure first. When `ConfirmQuit` was queued
    /// after edit-mode `SaveCardQuit` while `dirty` is still set, chains
    /// `OptionsDialog::Notice` -> `OptionsDialog::ConfirmSaveBeforeQuit` on
    /// dismissal so the player sees the exact error before retrying or quitting
    /// without saving.
    pub fn after_save_failed_with_notice(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) {
        if self.dirty && self.hud.ui_dialog_kind() == Some(OptionsDialog::ConfirmQuit) {
            self.show_notice(title, body);
            self.pending_after_notice = Some(OptionsDialog::ConfirmSaveBeforeQuit);
        } else if !self.hud.has_ui_dialog() {
            self.show_notice(title, body);
        }
    }

    /// Whether a Sound volume slider drag is currently captured (`0x5f6245`).
    pub fn dragging_slider(&self) -> bool {
        self.hud.dragging_slider()
    }

    /// Updates an active Sound volume slider drag with a canvas x-coordinate
    /// (including out-of-window coordinates while captured, `0x5f6245..0x5f6336`).
    pub fn drag_slider(&mut self, x: i32) -> bool {
        self.hud.drag_slider(x)
    }

    /// Whether a `cTSWinTextEdit2` mouse-drag text selection is currently captured
    /// (`[esi + 0x157]` at `0x5f27a6..0x5f2887`).
    pub fn dragging_dialog_entry(&self) -> bool {
        self.hud.dragging_dialog_entry()
    }

    /// Updates a captured `cTSWinTextEdit2` mouse-drag text selection with canvas
    /// coordinates (including out-of-window coordinates while captured,
    /// `0x5f27ae..0x5f2887`).
    pub fn drag_dialog_entry(&mut self, point: [i32; 2]) -> bool {
        self.hud.drag_dialog_entry(point)
    }

    /// Displays a status note across the top of the frame, clearing any
    /// pending post-save follow-up note so a failed host store open never
    /// leaks a stale Neighborhood or Reset Tutorial note onto a later save.
    pub fn set_note(&mut self, note: impl Into<String>) {
        self.pending_after_save_note = None;
        self.note = Some(note.into());
        self.log_debug_note_if_new();
    }

    /// The status note currently displayed across the top of the frame, if any.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Whether the active modal dialog has an editable `cTSWinTextEdit2` field.
    pub fn has_dialog_entry(&self) -> bool {
        !self.ui_modal_in_front() && self.hud.active_dialog_entry().is_some()
    }

    /// Whether a Game-drawn modal (phone book, book, shop picker, special
    /// panel or destination picker) sits in front of any HUD or VM dialog.
    /// Keys then go to it, never to a text field behind it.
    pub(super) fn ui_modal_in_front(&self) -> bool {
        self.phonebook_dialog.is_open()
            || self.book_dialog.is_open()
            || self.shop_dialog.is_open()
            || self.active_special_panel.is_some()
            || self.destination_picker.is_some()
    }

    /// The text currently in the active modal dialog's `cTSWinTextEdit2` field,
    /// if any.
    pub fn dialog_entry(&self) -> Option<&str> {
        self.hud.active_dialog_entry()
    }

    /// The text entered in the last answered VM kind 3 text-entry dialog
    /// (`0x451a6b..0x451a85` -> global `[0x6e4ac0]`).
    pub fn last_dialog_text(&self) -> &str {
        self.lot.last_dialog_text()
    }

    /// Whether the lot is open in edit mode (`[lot + 0x50] == 1`, `0x4f24b5`,
    /// `0x4f24d9`, `0x517c21`).
    pub fn edit_mode(&self) -> bool {
        self.edit_mode
    }

    /// Sets whether the lot is open in neighborhood edit mode (`[lot + 0x50] == 1`).
    pub fn set_edit_mode(&mut self, edit: bool) {
        self.edit_mode = edit;
        self.lot.set_edit_budget(edit);
    }

    /// Per-lot Options visibility (`0x4f2490..0x4f24e7`, `0x5177aa..0x517833`).
    pub fn lot_visibility(&self) -> LotVisibility {
        let world = self.lot.world();
        let has_family = world.globals.get(9).copied().unwrap_or(0) != 0;
        LotVisibility::for_lot(
            self.lot.lot_id(),
            self.lot.lot_zoning(),
            has_family,
            self.edit_mode,
        )
    }

    /// Current `[fx, music, vox]` Sound slider steps (`0..=10`), wired to
    /// `sims_audio::player::Player` via [`crate::sound::Feed`].
    pub fn volumes(&self) -> [u8; 3] {
        self.hud.controls.options.sound.steps()
    }

    /// Sets one Sound volume slider step (`0..=10`, clamped).
    pub fn set_volume(&mut self, channel: VolumeChannel, step: u8) -> bool {
        self.hud.controls.options.sound.set(channel, step)
    }

    /// Types `ch` into the active modal dialog's `cTSWinTextEdit2` input field.
    pub fn type_dialog_char(&mut self, ch: char) -> bool {
        !self.ui_modal_in_front() && self.hud.type_dialog_char(ch)
    }

    /// Deletes the character before the caret in the active dialog's input field.
    pub fn dialog_backspace(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_backspace()
    }

    /// Deletes the character at the caret in the active dialog's input field.
    pub fn dialog_delete(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_delete()
    }

    /// Moves the active dialog's text-entry caret one character left.
    pub fn dialog_move_left(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_move_left()
    }

    /// Moves the active dialog's text-entry caret one character right.
    pub fn dialog_move_right(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_move_right()
    }

    /// Moves the active dialog's text-entry caret one visual row up (multiline
    /// fields, `0x5f29f2..0x5f2a90`).
    pub fn dialog_move_up(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_move_up()
    }

    /// Moves the active dialog's text-entry caret one visual row down (multiline
    /// fields, `0x5f2a90..0x5f2b40`).
    pub fn dialog_move_down(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_move_down()
    }

    /// Moves the active dialog's text-entry caret to the start of the current
    /// line (or field, `0x5f2c13`).
    pub fn dialog_home(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_home()
    }

    /// Moves the active dialog's text-entry caret to the end of the field.
    pub fn dialog_end(&mut self) -> bool {
        !self.ui_modal_in_front() && self.hud.dialog_end()
    }

    /// Submits the active modal dialog via Enter (activates button 0 if enabled).
    pub fn dialog_submit(&mut self) -> Outcome {
        // The foreground modal takes Enter first, so it never reaches a VM
        // prompt waiting behind it.
        if self.phonebook_dialog.is_open()
            || self.book_dialog.is_open()
            || self.shop_dialog.is_open()
        {
            return self.dialog_confirm_key(DialogKey::Enter);
        }
        if self.active_special_panel.is_some() || self.destination_picker.is_some() {
            return Outcome::default();
        }
        let Some(command) = self.hud.submit_dialog() else {
            return Outcome::default();
        };
        let outcome = self.command(command, None);
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        outcome
    }

    /// Scrolls the camera when `Edge Scrolling` is enabled (`0x4e6246..0x4e6286`,
    /// `0x4d25b6..0x4d25c7` -> `0x4d45f3` -> `0x4d4709..0x4d4ade`) and the
    /// pointer rests within the 32-pixel windowed edge band of the house
    /// viewport in any mode with no modal dialog or pie menu open. Returns
    /// whether the camera moved.
    pub fn edge_scroll(&mut self, elapsed_us: u64) -> bool {
        if !self.hud.controls.options.play.edge_scrolling
            || self.hud.has_ui_dialog()
            || self.lot.asking().is_some()
            || self.lot.asking_special_panel().is_some()
            || self.shop_dialog.is_open()
            || self.book_dialog.is_open()
            || self.phonebook_dialog.is_open()
            || self.hud.menu.is_some()
            || self.cheat_console_open()
        {
            self.edge_scroll_rem = [0, 0];
            return false;
        }
        let Some([px, py]) = self.pointer else {
            self.edge_scroll_rem = [0, 0];
            return false;
        };
        // `0x4d45c8` -> `0x4d4843`: subtract the expanded HUD strip height so
        // bottom-edge scrolling triggers at the house viewport bottom and does
        // not pan when hovering over the control panel strip.
        let (vw, vh) = self.house_viewport_size();
        let (w, h) = (vw as i32, vh as i32);
        // `0x4d4586..0x4d45b1`, `0x4d4843`: windowed mode uses a 32-pixel
        // (`0x20`) edge scroll band inside the house viewport.
        const EDGE: i32 = 32;
        if w <= EDGE * 2 || h <= EDGE * 2 || px < 0 || py < 0 || px >= w || py >= h {
            self.edge_scroll_rem = [0, 0];
            return false;
        }
        let dir_x: i128 = if px < EDGE {
            1
        } else if px >= w - EDGE {
            -1
        } else {
            0
        };
        let dir_y: i128 = if py < EDGE {
            1
        } else if py >= h - EDGE {
            -1
        } else {
            0
        };
        if dir_x == 0 || dir_x * self.edge_scroll_rem[0] < 0 {
            self.edge_scroll_rem[0] = 0;
        }
        if dir_y == 0 || dir_y * self.edge_scroll_rem[1] < 0 {
            self.edge_scroll_rem[1] = 0;
        }
        // `0x4d4440`: `[esi + 0x60] = -562.5` px/s and `[esi + 0x64] = 562.5` px/s.
        // Straight edges (`0x4d4839`, `0x4d4946`, `0x4d499d`, `0x4d4a06`) scroll
        // at 562.5 px/s = 9 / 16_000 px/us exactly. Corners (`0x4d4893`,
        // `0x4d48f9`) start at raw `(±1125.0, ±562.5)` and are normalized by
        // `0x4d4ab0..0x4d4ad8` (`0x4d4ab5`) to magnitude 562.5 px/s
        // (`(1125 / sqrt(5), 562.5 / sqrt(5))` px/s). `0x4d50ef..0x4d5160` caps
        // each step at 100 ms and skips movement when `dt == 0`.
        if elapsed_us == 0 || (dir_x == 0 && dir_y == 0) {
            return false;
        }
        const SCALE: i128 = 16_000_000_000;
        const STRAIGHT_PER_US: i128 = 9_000_000; // 562.5 px/s exactly
        const CORNER_X_PER_US: i128 = 8_049_845; // 1125 / sqrt(5) px/s
        const CORNER_Y_PER_US: i128 = 4_024_922; // 562.5 / sqrt(5) px/s
        let (rate_x, rate_y) = if dir_x != 0 && dir_y != 0 {
            (CORNER_X_PER_US, CORNER_Y_PER_US)
        } else {
            (STRAIGHT_PER_US, STRAIGHT_PER_US)
        };
        let dt_us = i128::from(elapsed_us.min(100_000));
        let acc_x = self.edge_scroll_rem[0] + dir_x * dt_us * rate_x;
        let acc_y = self.edge_scroll_rem[1] + dir_y * dt_us * rate_y;
        let dx = (acc_x / SCALE) as i32;
        let dy = (acc_y / SCALE) as i32;
        self.edge_scroll_rem = [acc_x % SCALE, acc_y % SCALE];
        if dx == 0 && dy == 0 {
            return false;
        }
        let moved = self.apply(Move::Scroll([dx, dy]));
        if moved && let Some(point) = self.pointer {
            let _ = self.refresh_mode_hover(point);
        }
        moved
    }

    /// Refreshes Buy/Build hover previews (including held furniture in Build
    /// mode) or Live pointer following after pointer movement or edge scrolling.
    pub(super) fn refresh_mode_hover(&mut self, point: [i32; 2]) -> bool {
        if self.hud.controls.mode == Mode::Buy {
            if !self.hud.covers(point) || self.buy_dragging.is_some() {
                self.hover_buy(point)
            } else {
                self.hide_buy_cursor()
            }
        } else if self.hud.controls.mode == Mode::Build {
            if self.buy_selected.is_some()
                || self.buy_cursor.is_some()
                || self.coop_buy_preview.is_some()
            {
                if !self.hud.covers(point) || self.buy_dragging.is_some() {
                    self.hover_buy(point)
                } else {
                    self.hide_buy_cursor()
                }
            } else if !self.hud.covers(point) {
                self.hover_build(point)
            } else if self.build_drag.is_none() {
                self.hide_build_cursor()
            } else {
                false
            }
        } else {
            self.follow()
        }
    }
}
