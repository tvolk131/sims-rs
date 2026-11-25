//! Keyboard input.

use super::*;

impl NeighborhoodShell {
    pub fn key_dialog(&mut self, key: DialogKey) -> ShellOutcome {
        if self.album_notice_open() {
            return self.key_shell_dialog(key);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            let out = session.game.dialog_confirm_key(key);
            return self.handle_lot_outcome(out);
        }
        self.key_shell_dialog(key)
    }

    /// Answers the shell dialog from a key.
    pub(super) fn key_shell_dialog(&mut self, key: DialogKey) -> ShellOutcome {
        let Some(dlg) = &self.dialog else {
            return ShellOutcome::default();
        };
        let count = dlg.message.buttons.len();
        let choice = match key {
            DialogKey::Enter => (count > 0).then_some(0u8),
            DialogKey::Escape => match count {
                0 => None,
                1 => Some(0),
                n => Some((n - 1) as u8),
            },
            DialogKey::Char(ch) if count >= 2 => {
                let lower = ch.to_ascii_lowercase();
                if lower == self.dialogs.labels.yes_key().to_ascii_lowercase() {
                    Some(0)
                } else if lower == self.dialogs.labels.no_key().to_ascii_lowercase() {
                    Some(1)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(button) = choice {
            self.answer_dialog(button)
        } else {
            ShellOutcome::default()
        }
    }

    pub fn escape(&mut self) -> ShellOutcome {
        if self.save_manager.is_open() {
            if let Some(cmd) = self.save_manager.escape() {
                return self.handle_save_manager_command(cmd);
            }
            return ShellOutcome::redraw();
        }
        if self.import_picker.is_open() {
            if let Some(cmd) = self.import_picker.escape() {
                return self.handle_import_picker_command(cmd);
            }
            return ShellOutcome::redraw();
        }
        if self.credits_dialog.is_open() {
            self.close_credits_modal();
            return ShellOutcome::redraw();
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            if let Some(cmd) = self.coop_lobby.escape(&snap) {
                return self.handle_shell_coop_command(cmd);
            }
            return ShellOutcome::redraw();
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            let out = session.game.coop_lobby_escape();
            return self.handle_lot_outcome(out);
        }
        if let Some(td_state) = self.travel_dialog_state.clone() {
            if let Some(cmd) = self.travel_dialog.key_press("Escape", &td_state) {
                return self.handle_travel_dialog_command(cmd);
            }
            self.close_travel_dialog();
            return ShellOutcome::redraw();
        }
        if self.album_notice_open() {
            return self.key_shell_dialog(DialogKey::Escape);
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Escape);
        }
        if self.dialog.is_some() {
            return self.key_dialog(DialogKey::Escape);
        }
        match &mut self.screen {
            ActiveScreen::Loading(_) => ShellOutcome::default(),
            ActiveScreen::Neighborhood => {
                if !matches!(self.nbhd_state.mode, NeighborhoodMode::Normal)
                    || self.travel_state.mode != TravelBarMode::Browse
                {
                    self.nbhd_state.mode = NeighborhoodMode::Normal;
                    self.travel_state.mode = TravelBarMode::Browse;
                    self.move_in_family_id = None;
                    self.refresh_hovered_lot();
                    return ShellOutcome::redraw();
                }
                if destination_for_kind(self.kind).is_some() {
                    let home_kind = default_map_kind(&self.install, self.slot);
                    let _ = self.switch_kind(home_kind);
                    return ShellOutcome::redraw();
                }
                self.open_quit_dialog();
                ShellOutcome::redraw()
            }
            ActiveScreen::FamilyBin => {
                let cmd = self.family_bin.cancel();
                self.handle_family_bin_command(cmd)
            }
            ActiveScreen::CreateFamily(caf) => {
                let outcome = caf.key_escape();
                self.drain_caf_voice_previews();
                self.handle_caf_outcome(outcome)
            }
            ActiveScreen::Lot(session) if session.game.destination_picker_open() => {
                let out = session.game.destination_picker_escape();
                self.handle_lot_outcome(out)
            }
            ActiveScreen::Lot(session) if session.game.shop_open() => {
                let out = session.game.shop_escape();
                self.handle_lot_outcome(out)
            }
            ActiveScreen::Lot(session) if session.game.book_open() => {
                let out = session.game.book_close();
                self.handle_lot_outcome(out)
            }
            ActiveScreen::Lot(session) if session.game.camera_strip_open() => {
                session.game.close_camera_strip();
                ShellOutcome::redraw()
            }
            ActiveScreen::Lot(_) => ShellOutcome::default(),
        }
    }

    pub fn key_char(&mut self, ch: char) -> ShellOutcome {
        if self.album_notice_open() {
            return self.key_shell_dialog(DialogKey::Char(ch));
        }
        if self.coop_lobby.is_open() {
            return ShellOutcome {
                redraw: self.coop_lobby.insert_char(ch),
                ..ShellOutcome::default()
            };
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            return ShellOutcome {
                redraw: session.game.coop_lobby_char(ch),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Char(ch));
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_char(ch);
            self.drain_caf_voice_previews();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_backspace(&mut self) -> ShellOutcome {
        if self.coop_lobby.is_open() {
            return ShellOutcome {
                redraw: self.coop_lobby.backspace(),
                ..ShellOutcome::default()
            };
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            return ShellOutcome {
                redraw: session.game.coop_lobby_backspace(),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Backspace);
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_backspace();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_delete(&mut self) -> ShellOutcome {
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Delete);
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_delete();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_left(&mut self) -> ShellOutcome {
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            return ShellOutcome {
                redraw: self.credits_dialog.prev_page(snapshot).is_some(),
                ..ShellOutcome::default()
            };
        }
        if self.travel_dialog_state.is_some() {
            return ShellOutcome {
                redraw: self.cycle_travel_destination(-1),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Left);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.book_open() {
                return ShellOutcome {
                    redraw: session.game.book_prev_page(),
                    ..ShellOutcome::default()
                };
            }
            if session.game.shop_open() {
                let out = session.game.shop_rotate_preview(-45);
                return self.handle_lot_outcome(out);
            }
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_left();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_right(&mut self) -> ShellOutcome {
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            return ShellOutcome {
                redraw: self.credits_dialog.next_page(snapshot).is_some(),
                ..ShellOutcome::default()
            };
        }
        if self.travel_dialog_state.is_some() {
            return ShellOutcome {
                redraw: self.cycle_travel_destination(1),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Right);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.book_open() {
                return ShellOutcome {
                    redraw: session.game.book_next_page(),
                    ..ShellOutcome::default()
                };
            }
            if session.game.shop_open() {
                let out = session.game.shop_rotate_preview(45);
                return self.handle_lot_outcome(out);
            }
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_right();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_up(&mut self) -> ShellOutcome {
        if self.save_manager.is_open()
            && let Some(snapshot) = &self.save_manager_snapshot
        {
            return ShellOutcome {
                redraw: self.save_manager.select_prev(snapshot),
                ..ShellOutcome::default()
            };
        }
        if self.import_picker.is_open()
            && let Some(snapshot) = &self.import_picker_snapshot
        {
            return ShellOutcome {
                redraw: self.import_picker.select_prev(snapshot),
                ..ShellOutcome::default()
            };
        }
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            let frame_size = self.frame_size_u32();
            return ShellOutcome {
                redraw: self.credits_dialog.scroll_vertical(
                    -3,
                    frame_size,
                    snapshot,
                    &self.save_manager_art,
                ),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Up);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.book_open() {
                return ShellOutcome {
                    redraw: session.game.book_scroll_lines(-3),
                    ..ShellOutcome::default()
                };
            }
            if session.game.shop_open() {
                return ShellOutcome {
                    redraw: session.game.shop_select_prev(),
                    ..ShellOutcome::default()
                };
            }
        }
        ShellOutcome::default()
    }

    pub fn key_down(&mut self) -> ShellOutcome {
        if self.save_manager.is_open()
            && let Some(snapshot) = &self.save_manager_snapshot
        {
            return ShellOutcome {
                redraw: self.save_manager.select_next(snapshot),
                ..ShellOutcome::default()
            };
        }
        if self.import_picker.is_open()
            && let Some(snapshot) = &self.import_picker_snapshot
        {
            return ShellOutcome {
                redraw: self.import_picker.select_next(snapshot),
                ..ShellOutcome::default()
            };
        }
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            let frame_size = self.frame_size_u32();
            return ShellOutcome {
                redraw: self.credits_dialog.scroll_vertical(
                    3,
                    frame_size,
                    snapshot,
                    &self.save_manager_art,
                ),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Down);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.book_open() {
                return ShellOutcome {
                    redraw: session.game.book_scroll_lines(3),
                    ..ShellOutcome::default()
                };
            }
            if session.game.shop_open() {
                return ShellOutcome {
                    redraw: session.game.shop_select_next(),
                    ..ShellOutcome::default()
                };
            }
        }
        ShellOutcome::default()
    }

    pub fn key_home(&mut self) -> ShellOutcome {
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Home);
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_home();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_end(&mut self) -> ShellOutcome {
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::End);
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_end();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_tab(&mut self) -> ShellOutcome {
        if self.coop_lobby.is_open() {
            let next = match self.coop_lobby.tab() {
                LobbyTab::Host => LobbyTab::Join,
                LobbyTab::Join => LobbyTab::Host,
            };
            self.coop_lobby.set_tab(next);
            return ShellOutcome::redraw();
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            return ShellOutcome {
                redraw: session.game.coop_lobby_toggle_tab(),
                ..ShellOutcome::default()
            };
        }
        if self.travel_dialog_state.is_some() {
            return ShellOutcome {
                redraw: self.cycle_travel_destination(1),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Tab);
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_tab();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }

    pub fn key_page_up(&mut self) -> ShellOutcome {
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            return ShellOutcome {
                redraw: self.credits_dialog.prev_page(snapshot).is_some(),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::PageUp);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.book_open()
        {
            return ShellOutcome {
                redraw: session.game.book_prev_page(),
                ..ShellOutcome::default()
            };
        }
        ShellOutcome::default()
    }

    pub fn key_page_down(&mut self) -> ShellOutcome {
        if self.credits_dialog.is_open()
            && let Some(snapshot) = &self.credits_snapshot
        {
            return ShellOutcome {
                redraw: self.credits_dialog.next_page(snapshot).is_some(),
                ..ShellOutcome::default()
            };
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::PageDown);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.book_open()
        {
            return ShellOutcome {
                redraw: session.game.book_next_page(),
                ..ShellOutcome::default()
            };
        }
        ShellOutcome::default()
    }

    pub fn key_save_shortcut(&mut self) -> ShellOutcome {
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::SaveShortcut);
        }
        ShellOutcome::default()
    }

    pub fn key_enter(&mut self) -> ShellOutcome {
        if self.save_manager.is_open() {
            let cmd = self
                .save_manager_snapshot
                .as_ref()
                .and_then(|snap| self.save_manager.submit(snap));
            return match cmd {
                Some(cmd) => self.handle_save_manager_command(cmd),
                None => ShellOutcome::default(),
            };
        }
        if self.import_picker.is_open() {
            let cmd = self
                .import_picker_snapshot
                .as_ref()
                .and_then(|snap| self.import_picker.submit(snap));
            return match cmd {
                Some(cmd) => self.handle_import_picker_command(cmd),
                None => ShellOutcome::default(),
            };
        }
        if self.credits_dialog.is_open() {
            self.close_credits_modal();
            return ShellOutcome::redraw();
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            return match self.coop_lobby.submit(&snap) {
                Some(cmd) => self.handle_shell_coop_command(cmd),
                None => ShellOutcome::default(),
            };
        }
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            let out = session.game.coop_lobby_submit();
            return self.handle_lot_outcome(out);
        }
        if let Some(td_state) = self.travel_dialog_state.clone() {
            if let Some(cmd) = self.travel_dialog.key_press("Enter", &td_state) {
                return self.handle_travel_dialog_command(cmd);
            }
            return ShellOutcome::default();
        }
        if self.album_notice_open() {
            return self.key_shell_dialog(DialogKey::Enter);
        }
        if self.album_dialog.is_open() {
            return self.send_album_key(AlbumKey::Enter);
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.destination_picker_open() {
                let out = session.game.destination_picker_submit();
                return self.handle_lot_outcome(out);
            }
            if session.game.shop_open() {
                let out = session.game.shop_submit();
                return self.handle_lot_outcome(out);
            }
            if session.game.book_open() {
                let out = session.game.book_close();
                return self.handle_lot_outcome(out);
            }
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.key_enter();
            self.drain_caf_voice_previews();
            return self.handle_caf_outcome(outcome);
        }
        ShellOutcome::default()
    }
}
