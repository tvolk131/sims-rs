//! Pointer input: hover, wheel, presses and releases.

use super::*;

impl NeighborhoodShell {
    pub fn hover(&mut self, point: [i32; 2]) -> bool {
        self.pointer = Some(point);
        let frame_size = self.frame_size_u32();
        if self.save_manager.is_open() {
            if let Some(snapshot) = &self.save_manager_snapshot {
                self.save_manager
                    .pointer_move(frame_size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if self.import_picker.is_open() {
            if let Some(snapshot) = &self.import_picker_snapshot {
                self.import_picker
                    .pointer_move(frame_size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if self.credits_dialog.is_open() {
            if let Some(snapshot) = &self.credits_snapshot {
                self.credits_dialog
                    .pointer_move(frame_size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            self.coop_lobby
                .pointer_move(frame_size, &snap, point[0], point[1]);
            return true;
        }
        if self.travel_dialog_state.is_some() {
            return true;
        }
        if self.album_dialog.is_open() {
            if let Some(snapshot) = &self.album_snapshot {
                self.album_dialog
                    .handle_pointer_move(point, self.size, snapshot);
            }
            return true;
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            return session.game.hover(point);
        }
        if self.dialog.is_some() {
            return true;
        }
        match &mut self.screen {
            ActiveScreen::Loading(_) => false,
            ActiveScreen::Neighborhood => {
                let prev_lot = self.hovered_lot_id;
                let (chrome_changed, over_bar) = if destination_for_kind(self.kind).is_some() {
                    let prev_btn = self.travel_chrome.hovered();
                    // The bar mode follows every shell mode change (Move In latch).
                    self.sync_travel_state();
                    let layout =
                        TravelLayout::compute(frame_size[0], frame_size[1], &self.travel_state);
                    self.travel_chrome.pointer_move(point[0], point[1], &layout);
                    let cur_btn = self.travel_chrome.hovered();
                    // A captured press keeps the pointer's moves (`0x5da9e3`):
                    // no lot hovers until it is released.
                    let over_chrome = cur_btn.is_some()
                        || self.travel_chrome.pressed().is_some()
                        || layout.blocks_map(point[0], point[1]);
                    (cur_btn != prev_btn, over_chrome)
                } else {
                    let prev_btn = self.chrome.hovered();
                    let btn = self.chrome.hover(
                        point[0],
                        point[1],
                        frame_size,
                        &self.nbhd_state,
                        &self.chrome_art,
                    );
                    (
                        btn != prev_btn,
                        btn.is_some() || point[1] < sims_ui::neighborhood::BAR_HEIGHT,
                    )
                };
                let next_lot = if over_bar {
                    None
                } else {
                    self.map
                        .hit_test_with_state(frame_size, point, self.hovered_lot_id, true)
                };
                self.hovered_lot_id = next_lot;
                if next_lot != prev_lot {
                    self.hovered_lot_since_us = next_lot.map(|_| self.clock_us);
                    self.hover_card_shown_for = None;
                    self.nbhd_state.hovered_lot = None;
                    self.update_hover_fade(prev_lot, next_lot);
                }
                chrome_changed || next_lot != prev_lot
            }
            ActiveScreen::FamilyBin => {
                let view = FamilyBinView {
                    mode: if self.kind == NeighborhoodKind::MagicTown {
                        FamilyBinMode::MagicTown
                    } else {
                        FamilyBinMode::Standard
                    },
                    families: &self.family_bin_entries,
                };
                self.family_bin.hover(point[0], point[1], frame_size, &view);
                true
            }
            ActiveScreen::CreateFamily(caf) => {
                let _ = caf.pointer_move(point, frame_size);
                self.drain_caf_voice_previews();
                true
            }
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn leave(&mut self) -> bool {
        self.pointer = None;
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
            || self.coop_lobby.is_open()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open()
        {
            return true;
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            return session.game.leave();
        }
        match self.screen {
            ActiveScreen::Loading(_) => false,
            ActiveScreen::Neighborhood => {
                let had_state = self.chrome.hovered().is_some()
                    || self.chrome.pressed().is_some()
                    || self.travel_chrome.hovered().is_some()
                    || self.travel_chrome.pressed().is_some()
                    || self.hovered_lot_id.is_some()
                    || self.pressed_lot_id.is_some();
                self.chrome.clear_pointer();
                self.travel_chrome.clear_pointer();
                let prev_lot = self.hovered_lot_id.take();
                self.hovered_lot_since_us = None;
                self.hover_card_shown_for = None;
                self.pressed_lot_id = None;
                self.nbhd_state.hovered_lot = None;
                self.update_hover_fade(prev_lot, None);
                had_state
            }
            ActiveScreen::FamilyBin => {
                self.family_bin.clear_pointer();
                true
            }
            ActiveScreen::CreateFamily(_) => false,
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn wheel(&mut self, delta_y: f64) -> bool {
        if self.save_manager.is_open() {
            let delta = if delta_y > 0.0 {
                -1
            } else if delta_y < 0.0 {
                1
            } else {
                return false;
            };
            let frame_size = self.frame_size_u32();
            if let Some(snapshot) = &self.save_manager_snapshot {
                return self.save_manager.scroll(delta, frame_size, snapshot);
            }
            return false;
        }
        if self.import_picker.is_open() {
            let delta = if delta_y > 0.0 {
                -1
            } else if delta_y < 0.0 {
                1
            } else {
                return false;
            };
            let frame_size = self.frame_size_u32();
            if let Some(snapshot) = &self.import_picker_snapshot {
                return self.import_picker.scroll(delta, frame_size, snapshot);
            }
            return false;
        }
        if self.credits_dialog.is_open() {
            let delta = if delta_y > 0.0 {
                -1
            } else if delta_y < 0.0 {
                1
            } else {
                return false;
            };
            let frame_size = self.frame_size_u32();
            if let Some(snapshot) = &self.credits_snapshot {
                if self.credits_dialog.scroll_vertical(
                    delta * 3,
                    frame_size,
                    snapshot,
                    &self.save_manager_art,
                ) {
                    return true;
                }
                return self.credits_dialog.scroll(delta, snapshot).is_some();
            }
            return false;
        }
        if self.album_notice_open() {
            // A notice over the album takes the wheel too: nothing behind
            // it scrolls.
            return false;
        }
        if self.album_dialog.is_open() {
            let delta = if delta_y > 0.0 {
                -1
            } else if delta_y < 0.0 {
                1
            } else {
                return false;
            };
            if let Some(snapshot) = &self.album_snapshot {
                // Only the caption scrolls; the modal takes the wheel
                // either way.
                self.album_dialog
                    .handle_scroll(delta, self.size, snapshot, &self.album_art);
                return true;
            }
            return false;
        }
        if self.travel_dialog_state.is_some() || self.dialog.is_some() {
            return false;
        }
        match &mut self.screen {
            ActiveScreen::Lot(session) if session.game.destination_picker_open() => false,
            ActiveScreen::Lot(session) if session.game.shop_open() => {
                session.game.shop_wheel(delta_y)
            }
            ActiveScreen::FamilyBin => {
                let delta = if delta_y > 0.0 {
                    -1
                } else if delta_y < 0.0 {
                    1
                } else {
                    return false;
                };
                let before = self.family_bin.scroll_offset();
                self.family_bin.scroll(
                    delta,
                    self.family_bin_entries.len(),
                    sims_ui::family_bin::VISIBLE_ROWS,
                );
                self.family_bin.scroll_offset() != before
            }
            _ => false,
        }
    }

    pub fn press(&mut self, point: [i32; 2]) -> ShellOutcome {
        self.pointer = Some(point);
        let frame_size = self.frame_size_u32();
        if self.save_manager.is_open() {
            if let Some(snapshot) = &self.save_manager_snapshot {
                let changed = self
                    .save_manager
                    .pointer_down(frame_size, snapshot, point[0], point[1]);
                return ShellOutcome {
                    redraw: changed,
                    ..ShellOutcome::default()
                };
            }
            return ShellOutcome::default();
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            let changed = self
                .coop_lobby
                .pointer_down(frame_size, &snap, point[0], point[1]);
            return ShellOutcome {
                redraw: changed,
                ..ShellOutcome::default()
            };
        }
        if self.import_picker.is_open() {
            if let Some(snapshot) = &self.import_picker_snapshot {
                let changed = self
                    .import_picker
                    .pointer_down(frame_size, snapshot, point[0], point[1]);
                return ShellOutcome {
                    redraw: changed,
                    ..ShellOutcome::default()
                };
            }
            return ShellOutcome::default();
        }
        if self.credits_dialog.is_open() {
            if let Some(snapshot) = &self.credits_snapshot {
                let changed = self
                    .credits_dialog
                    .pointer_down(frame_size, snapshot, point[0], point[1]);
                return ShellOutcome {
                    redraw: changed,
                    ..ShellOutcome::default()
                };
            }
            return ShellOutcome::default();
        }
        if let Some(td_state) = &self.travel_dialog_state {
            let layout = TravelDialogLayout::compute(frame_size[0], frame_size[1]);
            let changed = self
                .travel_dialog
                .pointer_down(point[0], point[1], &layout, td_state);
            return ShellOutcome {
                redraw: changed,
                ..ShellOutcome::default()
            };
        }
        if self.album_notice_open() {
            return self.press_shell_dialog(point);
        }
        if self.album_dialog.is_open() {
            if let Some(snapshot) = &self.album_snapshot {
                let changed = self
                    .album_dialog
                    .handle_pointer_down(point, self.size, snapshot);
                return ShellOutcome {
                    redraw: changed,
                    ..ShellOutcome::default()
                };
            }
            return ShellOutcome::default();
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            let out = session.game.press(point);
            return self.handle_lot_outcome(out);
        }
        if let Some(dlg) = &mut self.dialog {
            if let Some(layout) = &dlg.layout
                && let Some(Command::AnswerDialog { button, .. }) =
                    dialog::press_with_state(layout, point, &dlg.state)
            {
                dlg.state.pressed_button = Some(usize::from(button));
                return ShellOutcome::redraw();
            }
            return ShellOutcome::default();
        }
        self.note = None;
        match &mut self.screen {
            ActiveScreen::Loading(_) => ShellOutcome::default(),
            ActiveScreen::Neighborhood => {
                if destination_for_kind(self.kind).is_some() {
                    // The bar mode follows every shell mode change (Move In latch).
                    self.sync_travel_state();
                    let layout =
                        TravelLayout::compute(frame_size[0], frame_size[1], &self.travel_state);
                    match self.travel_chrome.pointer_down(
                        point[0],
                        point[1],
                        &layout,
                        &self.travel_state,
                    ) {
                        TravelPress::Missed => {}
                        TravelPress::Held => {
                            self.pressed_lot_id = None;
                            return ShellOutcome::redraw();
                        }
                        // Magic Town's Bulldoze and Move In act on mouse-down
                        // (`0x551e49`, `0x5e4506`).
                        TravelPress::Activated(cmd) => {
                            self.pressed_lot_id = None;
                            let outcome = self.handle_travel_command(cmd);
                            // A press that left the map (the family bin) drops
                            // the capture: its release lands elsewhere.
                            if !matches!(self.screen, ActiveScreen::Neighborhood)
                                || self.dialog.is_some()
                            {
                                self.travel_chrome.clear_pointer();
                            }
                            return outcome;
                        }
                    }
                    let over_chrome = layout.blocks_map(point[0], point[1]);
                    let hit = if over_chrome {
                        None
                    } else {
                        self.map
                            .hit_test_with_state(frame_size, point, self.hovered_lot_id, true)
                    };
                    self.pressed_lot_id = hit;
                    if hit != self.hovered_lot_id {
                        let prev_lot = self.hovered_lot_id;
                        self.hovered_lot_id = hit;
                        self.hovered_lot_since_us = hit.map(|_| self.clock_us);
                        self.hover_card_shown_for = None;
                        self.nbhd_state.hovered_lot = None;
                        self.update_hover_fade(prev_lot, hit);
                    }
                    return ShellOutcome {
                        redraw: hit.is_some(),
                        ..ShellOutcome::default()
                    };
                }
                if self.chrome.press(
                    point[0],
                    point[1],
                    frame_size,
                    &self.nbhd_state,
                    &self.chrome_art,
                ) {
                    self.pressed_lot_id = None;
                    return ShellOutcome::redraw();
                }
                let hit =
                    self.map
                        .hit_test_with_state(frame_size, point, self.hovered_lot_id, true);
                self.pressed_lot_id = hit;
                if hit != self.hovered_lot_id {
                    let prev_lot = self.hovered_lot_id;
                    self.hovered_lot_id = hit;
                    self.hovered_lot_since_us = hit.map(|_| self.clock_us);
                    self.hover_card_shown_for = None;
                    self.nbhd_state.hovered_lot = None;
                    self.update_hover_fade(prev_lot, hit);
                }
                ShellOutcome {
                    redraw: hit.is_some(),
                    ..ShellOutcome::default()
                }
            }
            ActiveScreen::FamilyBin => {
                let view = FamilyBinView {
                    mode: if self.kind == NeighborhoodKind::MagicTown {
                        FamilyBinMode::MagicTown
                    } else {
                        FamilyBinMode::Standard
                    },
                    families: &self.family_bin_entries,
                };
                let cmd = self.family_bin.press(point[0], point[1], frame_size, &view);
                if let Some(cmd) = cmd {
                    self.handle_family_bin_command(cmd)
                } else {
                    ShellOutcome::redraw()
                }
            }
            ActiveScreen::CreateFamily(caf) => {
                let outcome = caf.pointer_down(point, frame_size);
                self.drain_caf_voice_previews();
                self.handle_caf_outcome(outcome)
            }
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn double_click(&mut self, point: [i32; 2]) -> ShellOutcome {
        self.pointer = Some(point);
        if self.save_manager.is_open()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open()
            || self.dialog.is_some()
            || !matches!(self.screen, ActiveScreen::FamilyBin)
        {
            return self.press(point);
        }
        let frame_size = self.frame_size_u32();
        let view = FamilyBinView {
            mode: if self.kind == NeighborhoodKind::MagicTown {
                FamilyBinMode::MagicTown
            } else {
                FamilyBinMode::Standard
            },
            families: &self.family_bin_entries,
        };
        if let Some(cmd) = self
            .family_bin
            .double_click(point[0], point[1], frame_size, &view)
        {
            return self.handle_family_bin_command(cmd);
        }
        // Rapid repeat clicks on non-row controls (scrollbar arrows, action
        // buttons) fall through to `press` so their release activates normally.
        self.press(point)
    }

    pub fn release(&mut self, point: [i32; 2]) -> ShellOutcome {
        self.pointer = Some(point);
        let frame_size = self.frame_size_u32();
        if self.save_manager.is_open() {
            let cmd = self.save_manager_snapshot.as_ref().and_then(|snap| {
                self.save_manager
                    .pointer_up(frame_size, snap, point[0], point[1])
            });
            return match cmd {
                Some(cmd) => self.handle_save_manager_command(cmd),
                None => ShellOutcome::redraw(),
            };
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            let cmd = self
                .coop_lobby
                .pointer_up(frame_size, &snap, point[0], point[1]);
            return match cmd {
                Some(cmd) => self.handle_shell_coop_command(cmd),
                None => ShellOutcome::redraw(),
            };
        }
        if self.import_picker.is_open() {
            let cmd = self.import_picker_snapshot.as_ref().and_then(|snap| {
                self.import_picker
                    .pointer_up(frame_size, snap, point[0], point[1])
            });
            return match cmd {
                Some(cmd) => self.handle_import_picker_command(cmd),
                None => ShellOutcome::redraw(),
            };
        }
        if self.credits_dialog.is_open() {
            let cmd = self.credits_snapshot.as_ref().and_then(|snap| {
                self.credits_dialog
                    .pointer_up(frame_size, snap, point[0], point[1])
            });
            return match cmd {
                Some(BookCommand::Close) => {
                    self.close_credits_modal();
                    ShellOutcome::redraw()
                }
                Some(BookCommand::ScrollLines(delta)) => {
                    if let Some(snap) = &self.credits_snapshot {
                        let _ = self.credits_dialog.scroll_vertical(
                            delta,
                            frame_size,
                            snap,
                            &self.save_manager_art,
                        );
                    }
                    ShellOutcome::redraw()
                }
                Some(BookCommand::PageChanged(_)) | None => ShellOutcome::redraw(),
            };
        }
        if let Some(td_state) = self.travel_dialog_state.clone() {
            let layout = TravelDialogLayout::compute(frame_size[0], frame_size[1]);
            let cmd = self
                .travel_dialog
                .pointer_up(point[0], point[1], &layout, &td_state);
            return match cmd {
                Some(cmd) => self.handle_travel_dialog_command(cmd),
                None => ShellOutcome::redraw(),
            };
        }
        if self.album_notice_open() {
            return self.release_shell_dialog(point);
        }
        if self.album_dialog.is_open() {
            let cmds = self
                .album_snapshot
                .clone()
                .map(|snap| self.album_dialog.handle_pointer_up(point, self.size, &snap))
                .unwrap_or_default();
            let out = self.execute_album_commands(cmds);
            self.hydrate_album_images();
            return out;
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            let out = session.game.release(point);
            return self.handle_lot_outcome(out);
        }
        if let Some(dlg) = &mut self.dialog {
            let pressed = dlg.state.pressed_button.take();
            if let (Some(pressed_idx), Some(layout)) = (pressed, &dlg.layout)
                && let Some(Command::AnswerDialog { button, .. }) =
                    dialog::press_with_state(layout, point, &dlg.state)
                && usize::from(button) == pressed_idx
            {
                return self.answer_dialog(button);
            }
            return ShellOutcome {
                redraw: pressed.is_some(),
                ..ShellOutcome::default()
            };
        }
        match &mut self.screen {
            ActiveScreen::Loading(_) => ShellOutcome::default(),
            ActiveScreen::Neighborhood => {
                if destination_for_kind(self.kind).is_some() {
                    // The bar mode follows every shell mode change (Move In latch).
                    self.sync_travel_state();
                    let layout =
                        TravelLayout::compute(frame_size[0], frame_size[1], &self.travel_state);
                    if let Some(cmd) = self.travel_chrome.pointer_up(
                        point[0],
                        point[1],
                        &layout,
                        &self.travel_state,
                    ) {
                        self.pressed_lot_id = None;
                        return self.handle_travel_command(cmd);
                    }
                    let pressed_lot = self.pressed_lot_id.take();
                    let over_chrome = layout.blocks_map(point[0], point[1]);
                    let hit = if over_chrome {
                        None
                    } else {
                        self.map
                            .hit_test_with_state(frame_size, point, self.hovered_lot_id, true)
                    };
                    if let Some(lot_id) = pressed_lot
                        && Some(lot_id) == hit
                    {
                        return self.activate_lot(lot_id);
                    }
                    return ShellOutcome::redraw();
                }
                if let Some(cmd) = self.chrome.release(
                    point[0],
                    point[1],
                    frame_size,
                    &self.nbhd_state,
                    &self.chrome_art,
                ) {
                    self.pressed_lot_id = None;
                    return self.handle_neighborhood_command(cmd);
                }
                let pressed_lot = self.pressed_lot_id.take();
                let hit = if point[1] < sims_ui::neighborhood::BAR_HEIGHT {
                    None
                } else {
                    self.map
                        .hit_test_with_state(frame_size, point, self.hovered_lot_id, true)
                };
                if let Some(lot_id) = pressed_lot
                    && Some(lot_id) == hit
                {
                    return self.activate_lot(lot_id);
                }
                ShellOutcome::redraw()
            }
            ActiveScreen::FamilyBin => {
                let view = FamilyBinView {
                    mode: if self.kind == NeighborhoodKind::MagicTown {
                        FamilyBinMode::MagicTown
                    } else {
                        FamilyBinMode::Standard
                    },
                    families: &self.family_bin_entries,
                };
                if let Some(cmd) = self
                    .family_bin
                    .release(point[0], point[1], frame_size, &view)
                {
                    self.handle_family_bin_command(cmd)
                } else {
                    ShellOutcome::redraw()
                }
            }
            ActiveScreen::CreateFamily(caf) => {
                let outcome = caf.pointer_up();
                self.drain_caf_voice_previews();
                self.handle_caf_outcome(outcome)
            }
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub fn release_outside(&mut self) -> ShellOutcome {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open()
        {
            return ShellOutcome::redraw();
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            let out = session.game.release_outside();
            return self.handle_lot_outcome(out);
        }
        if let Some(dlg) = &mut self.dialog {
            let had = dlg.state.pressed_button.take().is_some();
            return ShellOutcome {
                redraw: had,
                ..ShellOutcome::default()
            };
        }
        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
            let outcome = caf.pointer_up();
            self.drain_caf_voice_previews();
            return self.handle_caf_outcome(outcome);
        }
        self.pressed_lot_id = None;
        self.chrome.clear_pointer();
        self.travel_chrome.clear_pointer();
        self.family_bin.clear_pointer();
        ShellOutcome::redraw()
    }
}
