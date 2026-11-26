//! Window size, pointer input, pie menus, selection and centring the view.

use super::*;

impl Game {
    /// Resizes the canvas to `size` of the original's pixels.
    pub fn resize(&mut self, size: [i32; 2]) -> bool {
        if size == self.view.size {
            return false;
        }
        self.camera_pan = None;
        self.view.resize(size);
        self.sync_view_centre();
        self.hud.resize_menu(size);
        self.hud.relayout_dialog(size);
        self.hud.layout_camera_strip(size);
        if matches!(
            self.hud.controls.mode,
            Mode::Buy | Mode::Build | Mode::Options
        ) {
            let sell = self.buy_sell_info();
            self.hud.relayout_panel(
                size,
                self.lot.world(),
                self.lot.residential(),
                self.lot.lot_zoning(),
                self.lot_visibility(),
                &self.assets,
                sell.as_ref(),
            );
            if self.hud.controls.mode == Mode::Build {
                self.layout_build();
            }
        }
        true
    }

    /// Moves the camera. The view's rotation and screen-centre pick are the
    /// lot's inputs too ([`Lot::set_view_rotation`], [`Lot::set_view_centre`]),
    /// synced only when the picker's camera geometry actually changes so
    /// no-op or wall-mode commands preserve a restored pick.
    pub fn apply(&mut self, command: Move) -> bool {
        let cancelled_pan = self.camera_pan.take().is_some();
        let cleared_roof = matches!(command, Move::Walls(_)) && self.roof_view;
        if cleared_roof {
            self.roof_view = false;
            self.hud.set_roof_view(false);
        }
        let before = self.view.camera;
        let (w, h) = self.house_viewport_size();
        let viewport = [w as i32, h as i32];
        let moved = match command {
            Move::ZoomIn | Move::ZoomOut | Move::Rotate(_) if self.lot_shape.size > 0 => {
                let changed = native_zoom_or_rotate(
                    &self.lot_shape,
                    &mut self.view.camera,
                    viewport,
                    command,
                );
                self.view.bound();
                changed || self.view.camera != before
            }
            _ => self.view.apply_in(command, viewport, pick::ORIGIN_COLUMN),
        };
        let after = self.view.camera;
        if after.rotation != before.rotation && self.coop.is_active() {
            let _ = self.coop.submit_command(NetCommand::SetCameraRotation {
                rotation: CameraRotation::new(after.rotation),
            });
        }
        if cancelled_pan
            || after.origin != before.origin
            || after.zoom != before.zoom
            || after.rotation != before.rotation
            || after.level != before.level
        {
            self.sync_view();
        }
        moved || cleared_roof
    }

    /// Whether the panel takes presses at `point`.
    pub fn over_panel(&self, point: [i32; 2]) -> bool {
        self.coop_lobby.is_open()
            || self.coop_overlay_buttons.contains(point)
            || self.has_dialog()
            || self.hud.covers(point)
    }

    /// Whether `point` lies inside the canvas (`[0..width, 0..height)`).
    pub(super) fn in_view(&self, point: [i32; 2]) -> bool {
        point[0] >= 0
            && point[1] >= 0
            && point[0] < self.view.size[0]
            && point[1] < self.view.size[1]
    }

    /// Moves the pointer to `point`. When it first reaches an object on
    /// the lot in Live mode, the selected Sim's pie menu on it is listed, as
    /// the original lists it then (0x48c834); listing changes the lot. In Buy
    /// mode, hovering over the house view activates and moves (or drag-rotates)
    /// the placement cursor (`0x487d3d`, `0x485a2e`), while moving onto the
    /// panel hides it (`0x487d4e`). A point outside the canvas is treated as
    /// [`Game::leave`].
    pub fn hover(&mut self, point: [i32; 2]) -> bool {
        if !self.in_view(point) {
            return self.leave();
        }
        self.pointer = Some(point);
        if self.coop_lobby.is_open() {
            let snap = self.coop_snapshot();
            let size = [
                self.view.size[0].max(1) as u32,
                self.view.size[1].max(1) as u32,
            ];
            self.coop_lobby
                .pointer_move(size, &snap, point[0], point[1]);
            return true;
        }
        if self.shop_dialog.is_open() {
            if let Some(snapshot) = &self.shop_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.shop_dialog
                    .pointer_move(size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if self.phonebook_dialog.is_open() {
            if let Some(snapshot) = &self.phonebook_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.phonebook_dialog
                    .pointer_move(size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if self.book_dialog.is_open() {
            if let Some(snapshot) = &self.book_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.book_dialog
                    .pointer_move(size, snapshot, point[0], point[1]);
            }
            return true;
        }
        if let Some(picker) = &mut self.destination_picker {
            return picker.hover(self.view.size, point, self.clock_us);
        }
        let mut redraw = self.hud.hover(point);
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        redraw |= self.highlight(point);
        redraw |= self.refresh_mode_hover(point);
        // The cutaway follows the rooms the pointer passes over, while
        // the cutaway is on and the pointer is over the lot.
        if !self.hud.covers(point) && self.view.camera.walls == Walls::Cutaway {
            let camera = self.view.camera;
            self.view
                .camera
                .focus
                .point(&self.lot_shape, &camera, point);
            redraw |= self.view.camera.focus != camera.focus;
        }
        redraw
    }

    /// A left press at `point`, which is over the panel or the lot.
    ///
    /// While a pie menu is open (with no dialog over it), a press only
    /// updates which entry is under `point` and marks the button held
    /// (0x5eb3d1 with `+0x170` clear); the entry is chosen, or the menu
    /// closed, on the matching [`Game::release`] (0x5eb433).
    pub fn press(&mut self, point: [i32; 2]) -> Outcome {
        // A new press starts clean: a non-modal dialog's capture from a press
        // whose release never came (focus lost, an overlay took it) is
        // dropped before anything else sees this one.
        self.hud.cancel_non_modal_press();
        if self.coop_lobby.is_open() {
            self.pointer = Some(point);
            let snap = self.coop_snapshot();
            let size = [
                self.view.size[0].max(1) as u32,
                self.view.size[1].max(1) as u32,
            ];
            let changed = self
                .coop_lobby
                .pointer_down(size, &snap, point[0], point[1]);
            return Outcome {
                redraw: changed,
                ..Outcome::default()
            };
        }
        if self.coop_overlay_buttons.press(point) {
            self.pointer = Some(point);
            return Outcome::redraw();
        }
        if self.shop_dialog.is_open() {
            self.pointer = Some(point);
            if let Some(snapshot) = &self.shop_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                let changed = self
                    .shop_dialog
                    .pointer_down(size, snapshot, point[0], point[1]);
                return Outcome {
                    redraw: changed,
                    ..Outcome::default()
                };
            }
            return Outcome::default();
        }
        if self.phonebook_dialog.is_open() {
            self.pointer = Some(point);
            if let Some(snapshot) = &self.phonebook_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                let changed = self
                    .phonebook_dialog
                    .pointer_down(size, snapshot, point[0], point[1]);
                return Outcome {
                    redraw: changed,
                    ..Outcome::default()
                };
            }
            return Outcome::default();
        }
        if self.book_dialog.is_open() {
            self.pointer = Some(point);
            if let Some(snapshot) = &self.book_snapshot {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                let changed = self
                    .book_dialog
                    .pointer_down(size, snapshot, point[0], point[1]);
                return Outcome {
                    redraw: changed,
                    ..Outcome::default()
                };
            }
            return Outcome::default();
        }
        if let Some(picker) = &mut self.destination_picker {
            self.pointer = Some(point);
            let changed = picker.press(self.view.size, point, self.clock_us);
            return Outcome {
                redraw: changed,
                ..Outcome::default()
            };
        }
        if !self.dialog_takes(point) && self.hud.menu.is_some() {
            self.pointer = Some(point);
            let mut redraw = self.hud.hover(point);
            redraw |= self.highlight(point);
            if let Some(menu) = &mut self.hud.menu {
                redraw |= menu.press();
            }
            return Outcome {
                redraw,
                ..Outcome::default()
            };
        }
        let cleared = self.note.take().is_some();
        if self.has_dialog() && !self.hud.has_dialog() {
            // A newly arrived simulation dialog not yet drawn still blocks
            // clicks from reaching controls behind it.
            return Outcome::redraw();
        }
        if self.hud.covers(point) {
            // Native `0x50d04d` → `0x4cf97a` → `0x487e18`: clicking the Buy
            // catalog strip while holding a picked-up existing object sells
            // it; clicking empty strip background while holding an unpaid
            // preview cancels it.
            let over_camera_strip = self
                .hud
                .camera_strip_rect()
                .is_some_and(|r| r.contains(point[0], point[1]));
            if !self.dialog_takes(point)
                && !over_camera_strip
                && self.hud.controls.mode == Mode::Buy
                && self.hud.over_buy_catalog(self.view.size, point)
                && self.buy_cursor.is_some()
                && (self.buy_cursor.as_ref().is_some_and(BuyCursor::is_existing)
                    || self.hud.buy_layout().press(point).is_none())
            {
                self.sell_held();
                return Outcome::redraw();
            }
            if !self.dialog_takes(point)
                && !over_camera_strip
                && self.hud.controls.mode == Mode::Build
                && let Some(action) = self.hud.press_build_subtool(point)
            {
                self.apply_build_subtool(action);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                return Outcome::redraw();
            }
            // The menu, whose entries a choice is an index into.
            let menu = self.hud.menu.clone();
            let outcome = match self
                .hud
                .press(point, self.lot.world(), self.lot.residential())
            {
                Some(command) => self.command(command, menu.as_ref()),
                None => Outcome::redraw(),
            };
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return outcome;
        }
        // 0x51f3c5..0x51f3e5, 0x51d312, 0x51d354: pressing outside the live
        // control panel dismisses any open help balloon with `UI_whoosh`.
        let dismissed = self.hud.dismiss_help();
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        // Native `cWinCamera`: clicking inside the house viewport while Camera
        // Mode is open closes the Camera subpanel and captures a clean snapshot.
        if self.hud.camera_strip_open() {
            self.photo_point = Some(point);
            self.hud.close_camera_strip();
            self.hud.play(crate::hud::CLICK);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return Outcome {
                redraw: true,
                photo: true,
                ..Outcome::default()
            };
        }
        if self.hud.controls.mode == Mode::Buy {
            if self.stopped.is_some() {
                self.note = Some("The lot has stopped, so it takes no orders.".into());
                return Outcome::redraw();
            }
            if self.buy_selected.is_some()
                || self.buy_cursor.is_some()
                || self.coop_buy_preview.is_some()
            {
                self.pointer = Some(point);
                self.hover_buy(point);
                self.buy_dragging = Some(point);
                self.buy_commit_on_release = true;
                let valid = if self.coop.is_active() {
                    self.coop_buy_preview.is_some()
                } else {
                    self.buy_cursor.as_ref().is_some_and(|c| c.valid)
                };
                self.hud.play(if valid {
                    "UI_object_move_place"
                } else {
                    crate::hud::ERROR
                });
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                return Outcome::redraw();
            }
            // Hand Tool pickup of an existing lot object (`0x485f98` →
            // `0x486960` → `0x4215e6`).
            if let Some(target) = self.under(point) {
                if self.coop.is_active() {
                    return self.coop_pick_up_preview(target, point);
                }
                match self.lot.buy_pick_up(target) {
                    Ok(Some(cursor)) => {
                        let screen = lot_point(&self.view.camera, &self.lot_shape, cursor.position);
                        self.buy_grab_offset = [
                            point[0] - (screen[0] + pick::ORIGIN_COLUMN),
                            point[1] - screen[1],
                        ];
                        self.buy_cursor = Some(cursor);
                        self.buy_in_house = true;
                        self.buy_dragging = Some(point);
                        self.buy_commit_on_release = false;
                        self.pointer = Some(point);
                        self.hud.play("UI_object_move");
                        let sounds = self.hud.take_sounds();
                        self.keep_sounds(sounds);
                        self.refresh_world();
                        self.layout_buy();
                        return Outcome::redraw();
                    }
                    Ok(None) => {
                        self.hud.play(crate::hud::ERROR);
                        let sounds = self.hud.take_sounds();
                        self.keep_sounds(sounds);
                        return Outcome::redraw();
                    }
                    Err(e) => {
                        self.note = Some(format!("Object {target} was not picked up: {e}"));
                        self.hud.play(crate::hud::ERROR);
                        let sounds = self.hud.take_sounds();
                        self.keep_sounds(sounds);
                        return Outcome::redraw();
                    }
                }
            }
            return Outcome {
                redraw: cleared || dismissed,
                ..Outcome::default()
            };
        }
        if self.hud.controls.mode == Mode::Build {
            if self.stopped.is_some() {
                self.note = Some("The lot has stopped, so it takes no orders.".into());
                return Outcome::redraw();
            }
            if self.buy_selected.is_some()
                || self.buy_cursor.is_some()
                || self.coop_buy_preview.is_some()
            {
                self.pointer = Some(point);
                self.hover_buy(point);
                self.buy_dragging = Some(point);
                self.buy_commit_on_release = true;
                let valid = if self.coop.is_active() {
                    self.coop_buy_preview.is_some()
                } else {
                    self.buy_cursor.as_ref().is_some_and(|c| c.valid)
                };
                self.hud.play(if valid {
                    "UI_object_move_place"
                } else {
                    crate::hud::ERROR
                });
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                return Outcome::redraw();
            }
            if self.hud.build_state.tool == Some(BuildTool::Hand) {
                if let Some(target) = self.under(point) {
                    if self.coop.is_active() {
                        return self.coop_pick_up_preview(target, point);
                    }
                    match self.lot.buy_pick_up(target) {
                        Ok(Some(cursor)) => {
                            let screen =
                                lot_point(&self.view.camera, &self.lot_shape, cursor.position);
                            self.buy_grab_offset = [
                                point[0] - (screen[0] + pick::ORIGIN_COLUMN),
                                point[1] - screen[1],
                            ];
                            self.buy_cursor = Some(cursor);
                            self.buy_in_house = true;
                            self.buy_dragging = Some(point);
                            self.buy_commit_on_release = false;
                            self.pointer = Some(point);
                            self.hud.play("UI_object_move");
                            let sounds = self.hud.take_sounds();
                            self.keep_sounds(sounds);
                            self.refresh_world();
                            self.layout_build();
                            return Outcome::redraw();
                        }
                        Ok(None) => {
                            self.hud.play(crate::hud::ERROR);
                            let sounds = self.hud.take_sounds();
                            self.keep_sounds(sounds);
                            return Outcome::redraw();
                        }
                        Err(e) => {
                            self.note = Some(format!("Object {target} was not picked up: {e}"));
                            self.hud.play(crate::hud::ERROR);
                            let sounds = self.hud.take_sounds();
                            self.keep_sounds(sounds);
                            return Outcome::redraw();
                        }
                    }
                }
                return Outcome {
                    redraw: cleared || dismissed,
                    ..Outcome::default()
                };
            }
            return self.press_build(point);
        }
        if self.hud.controls.mode != Mode::Live {
            return Outcome {
                redraw: cleared || dismissed,
                ..Outcome::default()
            };
        }
        let Some(target) = self.under(point) else {
            return Outcome {
                redraw: cleared || dismissed,
                ..Outcome::default()
            };
        };
        self.open_menu(target, point);
        Outcome::redraw()
    }

    /// A left release at `point` (`0x5eb433` in Live mode, `0x4864d0` in Buy
    /// mode, `0x5e4599` for modal dialog buttons, `0x5f62ba` for Sound sliders).
    ///
    /// - Evaluates and ends any active Sound slider drag (`0x5f62ba..0x5f62de`).
    /// - Releases any held modal dialog button (`0x5e4599..0x5e45fc`), activating
    ///   it only when `point` is still inside that button.
    /// - Commits a Buy Mode drag or updates/chooses an open pie menu entry.
    pub fn release(&mut self, point: [i32; 2]) -> Outcome {
        if self.coop_lobby.is_open() {
            self.pointer = Some(point);
            let snap = self.coop_snapshot();
            let size = [
                self.view.size[0].max(1) as u32,
                self.view.size[1].max(1) as u32,
            ];
            let cmd = self.coop_lobby.pointer_up(size, &snap, point[0], point[1]);
            return match cmd {
                Some(cmd) => self.execute_coop_ui_command(cmd),
                None => Outcome::redraw(),
            };
        }
        if let Some(action) = self.coop_overlay_buttons.release(Some(point)) {
            self.hud.play(crate::hud::CLICK);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return self.execute_coop_overlay_action(action);
        }
        if self.shop_dialog.is_open() {
            self.pointer = Some(point);
            let cmd = self.shop_snapshot.as_ref().and_then(|snap| {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.shop_dialog.pointer_up(size, snap, point[0], point[1])
            });
            return match cmd {
                Some(cmd) => self.execute_shop_command(cmd),
                None => Outcome::redraw(),
            };
        }
        if self.phonebook_dialog.is_open() {
            self.pointer = Some(point);
            let cmd = self.phonebook_snapshot.as_ref().and_then(|snap| {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.phonebook_dialog
                    .pointer_up(size, snap, point[0], point[1])
            });
            return match cmd {
                Some(cmd) => self.execute_phonebook_command(cmd),
                None => Outcome::redraw(),
            };
        }
        if self.book_dialog.is_open() {
            self.pointer = Some(point);
            let cmd = self.book_snapshot.as_ref().and_then(|snap| {
                let size = [
                    self.view.size[0].max(1) as u32,
                    self.view.size[1].max(1) as u32,
                ];
                self.book_dialog.pointer_up(size, snap, point[0], point[1])
            });
            return match cmd {
                Some(BookCommand::PageChanged(_)) => {
                    self.hud.play(crate::hud::CLICK);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    Outcome::redraw()
                }
                Some(BookCommand::ScrollLines(delta)) => {
                    if self.book_scroll_lines(delta) {
                        self.hud.play(crate::hud::CLICK);
                        let sounds = self.hud.take_sounds();
                        self.keep_sounds(sounds);
                    }
                    Outcome::redraw()
                }
                Some(BookCommand::Close) => self.book_close(),
                None => Outcome::redraw(),
            };
        }
        if let Some(picker) = &mut self.destination_picker {
            self.pointer = Some(point);
            let action = picker.release(self.view.size, point, self.clock_us);
            return match action {
                Some(DestinationPickerAction::SelectLot(lot_id)) => {
                    if self.answer_special_panel(SpecialPanelResponse::Lot(Some(lot_id as i16))) {
                        self.hud.play(crate::hud::CLICK);
                    } else {
                        self.hud.play(crate::hud::ERROR);
                    }
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    Outcome::redraw()
                }
                Some(DestinationPickerAction::Cancel) => self.destination_picker_escape(),
                Some(DestinationPickerAction::InvalidLot) => {
                    self.hud.play(crate::hud::ERROR);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    Outcome::redraw()
                }
                Some(DestinationPickerAction::Clicked) => {
                    self.hud.play(crate::hud::CLICK);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    Outcome::redraw()
                }
                Some(DestinationPickerAction::Redraw) | None => Outcome::redraw(),
            };
        }
        let released_slider = self.hud.release_slider(Some(point));
        let (released_saves, open_saves) = self.hud.release_saves_button(Some(point));
        if open_saves {
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return Outcome {
                redraw: true,
                open_save_manager: true,
                ..Outcome::default()
            };
        }
        let (released_coop, open_coop) = self.hud.release_coop_button(Some(point));
        if open_coop {
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            self.open_coop_lobby(None);
            return Outcome {
                redraw: true,
                open_coop_lobby: true,
                ..Outcome::default()
            };
        }
        let (released_camera, camera_cmd) = self
            .hud
            .release_camera_strip((!self.has_dialog()).then_some(point));
        if let Some(cmd) = camera_cmd {
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return match cmd {
                CameraCommand::OpenPhotoAlbum => Outcome {
                    redraw: true,
                    open_photo_album: true,
                    ..Outcome::default()
                },
                CameraCommand::SetSize(_)
                | CameraCommand::SetQuality(_)
                | CameraCommand::Help(_) => Outcome::redraw(),
            };
        }
        let (released_action, action_cmd) = self.hud.release_options_action(Some(point));
        if let Some(command) = action_cmd {
            let outcome = self.command(command, None);
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return outcome;
        }
        let (released_dialog, dialog_cmd) = self.hud.release_dialog(Some(point));
        // A press on a non-modal dialog is released there too: it never
        // reaches a pie menu, a held object or a tool under the dialog.
        let dialog_owned = self.hud.take_non_modal_press();
        if let Some(command) = dialog_cmd {
            let outcome = self.command(command, None);
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return outcome;
        }
        if dialog_owned {
            return Outcome::redraw();
        }
        if !self.in_view(point) {
            let mut out = self.release_outside();
            out.redraw |= released_slider
                || released_saves
                || released_coop
                || released_camera
                || released_action
                || released_dialog;
            return out;
        }
        if matches!(self.hud.controls.mode, Mode::Buy | Mode::Build)
            && let Some(anchor) = self.buy_dragging
        {
            // Dropping a held object onto an open Camera sub-strip cancels the
            // drag safely without committing a placement behind the HUD.
            if self
                .hud
                .camera_strip_rect()
                .is_some_and(|r| r.contains(point[0], point[1]))
            {
                self.buy_dragging = None;
                self.buy_commit_on_release = false;
                return Outcome::redraw();
            }
            // Dropping a held object onto the Buy catalog strip sells it
            // (`0x50d04d` → `0x4cf97a` → `0x487e18`).
            if self.hud.controls.mode == Mode::Buy
                && !self.hud.camera_strip_open()
                && self.hud.over_buy_catalog(self.view.size, point)
                && (self.buy_cursor.is_some() || self.coop_buy_preview.is_some())
            {
                self.buy_dragging = None;
                self.buy_commit_on_release = false;
                self.sell_held();
                return Outcome::redraw();
            }
            // Native `0x4864f0` invokes `OnMouseMove` with the release
            // coordinates before evaluating the drag threshold (`dx^2 + dy^2
            // >= 4`) and `[tool + 0x81]` commit flag.
            self.pointer = Some(point);
            self.hover_buy(point);
            self.buy_dragging = None;
            let dx = point[0] - anchor[0];
            let dy = point[1] - anchor[1];
            let dragged = dx * dx + dy * dy >= 4;
            let should_commit = self.buy_commit_on_release || dragged;
            self.buy_commit_on_release = false;
            if should_commit {
                return self.commit_buy(point);
            }
            return Outcome::redraw();
        }
        if self.hud.controls.mode == Mode::Build && self.build_drag.is_some() {
            return self.release_build(point);
        }
        if self.has_dialog() || self.hud.menu.is_none() {
            return Outcome {
                redraw: released_slider
                    || released_saves
                    || released_coop
                    || released_camera
                    || released_action
                    || released_dialog,
                ..Outcome::default()
            };
        }
        self.pointer = Some(point);
        let mut redraw = self.hud.hover(point);
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        redraw |= self.highlight(point);
        let now_ms = (self.clock_us / 1_000) as u32;
        let will_choose = self.hud.menu.as_ref().is_some_and(|m| m.will_choose(point));
        if !will_choose {
            let was_held = self.hud.menu.as_ref().is_some_and(|m| m.held());
            let (release, command) = self.hud.release_menu_at(point, now_ms);
            debug_assert_eq!((release, command), (menu::Release::Settled, None));
            return Outcome {
                redraw: redraw || was_held,
                ..Outcome::default()
            };
        }
        let _ = self.note.take();
        // Choosing an entry checks it again first (0x5eb433 -> 0x5eb62a
        // -> event 0xe -> 0x48df28).
        if let Some(entry) = self.hud.menu.as_ref().and_then(|m| m.highlighted(point)) {
            self.check_entry(entry);
        }
        // A failed check's note stays in front of what the choice says.
        let failed = self.note.take();
        let menu = self.hud.menu.clone();
        let (release, command) = self.hud.release_menu_at(point, now_ms);
        if release == menu::Release::Stays
            && let Some(warped) = self.hud.menu.as_ref().and_then(Menu::pending_pointer)
        {
            self.pointer = Some(warped);
        }
        self.highlighted = None;
        let outcome = match command {
            Some(command) => {
                if let Command::ChooseInteraction { sim, .. } = &command {
                    let origin = menu.as_ref().and_then(Menu::main_center).unwrap_or(point);
                    self.hud.set_queue_origin(Some(*sim), origin);
                }
                self.command(command, menu.as_ref())
            }
            None => Outcome::redraw(),
        };
        if let Some(failed) = failed {
            self.note = Some(match self.note.take() {
                Some(note) => format!("{failed} {note}"),
                None => failed,
            });
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        outcome
    }

    /// A left release while the pointer is outside the window (after
    /// [`Game::leave`]): ends slider/dialog capture without activating, cancels
    /// a Buy Mode drag, settles an opening press on a moved menu, or closes the
    /// pie without checking or choosing any entry.
    pub fn release_outside(&mut self) -> Outcome {
        let _ = self.coop_overlay_buttons.release(None);
        if let Some(picker) = &mut self.destination_picker {
            let redraw = picker.leave(self.clock_us);
            return Outcome {
                redraw,
                ..Outcome::default()
            };
        }
        let released_slider = self.hud.release_slider(None);
        let (released_saves, _) = self.hud.release_saves_button(None);
        let (released_coop, _) = self.hud.release_coop_button(None);
        let (released_camera, _) = self.hud.release_camera_strip(None);
        let (released_action, _) = self.hud.release_options_action(None);
        let (released_dialog, _) = self.hud.release_dialog(None);
        if self.hud.take_non_modal_press() {
            // The press was the non-modal dialog's: what is under it (a pie
            // menu, a drag) never sees its release.
            return Outcome::redraw();
        }
        if self.buy_dragging.take().is_some() {
            self.buy_commit_on_release = false;
            let redraw = self.hide_buy_cursor()
                || released_slider
                || released_saves
                || released_coop
                || released_camera
                || released_action
                || released_dialog;
            return Outcome {
                redraw,
                ..Outcome::default()
            };
        }
        if self.build_drag.take().is_some() {
            self.build_preview_shape = None;
            let _ = self.hide_build_cursor();
            return Outcome::redraw();
        }
        if self.has_dialog() || self.hud.menu.is_none() {
            return Outcome {
                redraw: released_slider
                    || released_saves
                    || released_coop
                    || released_camera
                    || released_action
                    || released_dialog,
                ..Outcome::default()
            };
        }
        self.pointer = None;
        self.highlighted = None;
        let redraw = self.hud.leave();
        let was_held = self.hud.menu.as_ref().is_some_and(|m| m.held());
        match self.hud.release_menu_outside() {
            menu::Release::Settled => Outcome {
                redraw: redraw || was_held,
                ..Outcome::default()
            },
            menu::Release::Closed => {
                let _ = self.note.take();
                Outcome::redraw()
            }
            menu::Release::Chose(_) | menu::Release::Stays => unreachable!(),
        }
    }

    /// The pointer left the window: it is on no object, button, portrait
    /// or pie entry, so coming back onto an entry checks it again. True
    /// if the frame changes.
    pub fn leave(&mut self) -> bool {
        self.pointer = None;
        self.highlighted = None;
        if let Some(picker) = &mut self.destination_picker {
            return picker.leave(self.clock_us);
        }
        let mut redraw = self.hud.leave();
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        if self.hud.controls.mode == Mode::Buy {
            if self.buy_dragging.is_none() {
                redraw |= self.hide_buy_cursor();
            }
            redraw
        } else if self.hud.controls.mode == Mode::Build {
            if self.buy_dragging.is_none() {
                redraw |= self.hide_buy_cursor();
            }
            if self.build_drag.is_none() {
                redraw |= self.hide_build_cursor();
            }
            redraw
        } else {
            self.follow() || redraw
        }
    }

    /// Follows the object under the pointer in the frame last drawn: when
    /// it changes to an object, lists the selected Sim's pie menu on it.
    /// Nothing is listed while the pointer is over the panel, a dialog or
    /// a menu, while a modal is up (the lot behind a modal takes no
    /// pointer, and listing runs check trees that change the world), or
    /// once the lot has stopped. True if a listing may have changed the lot.
    pub(super) fn follow(&mut self) -> bool {
        if self.hud.controls.mode != Mode::Live || self.has_dialog() || self.shop_dialog.is_open() {
            self.reached = None;
            self.listed = None;
            return false;
        }
        let under = match self.pointer {
            Some(point) if self.stopped.is_none() && !self.hud.covers(point) => self.under(point),
            _ => None,
        };

        if under == self.reached {
            return false;
        }
        self.reached = under;
        self.listed = None;
        under.is_some_and(|object| self.list(object))
    }

    /// For whom and on what the pie menu was listed when the pointer
    /// reached an object, until a click opens it or the pointer leaves.
    pub fn listed(&self) -> Option<(u16, u16)> {
        self.listed
            .as_ref()
            .map(|(person, object, _)| (*person, *object))
    }

    /// Checks again the action of the open pie menu's entry the pointer
    /// at `point` has reached, as the original does each time the pointer
    /// reaches an entry (0x48df28, `Lot::highlight`), which changes the
    /// lot. The action the check leaves is the one choosing the entry
    /// queues. True if the check ran.
    pub(super) fn highlight(&mut self, point: [i32; 2]) -> bool {
        let Some(menu) = &self.hud.menu else {
            self.highlighted = None;
            return false;
        };
        let entry = menu.highlighted(point);
        if entry == self.highlighted {
            return false;
        }
        self.highlighted = entry;
        entry.is_some_and(|i| self.check_entry(i))
    }

    /// Checks again the action of the open pie menu's entry `i`
    /// (0x48df28, `Lot::highlight`), keeping the action the check leaves
    /// for choosing it. True if the check ran.
    pub(super) fn check_entry(&mut self, i: usize) -> bool {
        if self.coop.is_active() {
            // Co-op never mutates the shared simulation on local pie hover;
            // `Lot::apply_coop_batch` re-validates on a throwaway probe when
            // `SimActionPick` executes deterministically on every peer.
            return false;
        }
        let Some(menu) = &self.hud.menu else {
            return false;
        };
        let Some(action) = menu.entries.get(i).and_then(|e| e.action.clone()) else {
            return false;
        };
        if self.stopped.is_some() {
            return false;
        }
        let person = menu.person;
        match self.lot.highlight(person, action) {
            Ok(checked) => {
                self.keep_sounds(checked.sounds);
                if let Some(entry) = self.hud.menu.as_mut().and_then(|m| m.entries.get_mut(i)) {
                    entry.action = Some(checked.action);
                }
            }
            Err(e) => self.note = Some(format!("The entry's check did not run: {e}")),
        }
        true
    }

    /// The person or object the pointer at `point` is on in the frame
    /// last drawn, by the original's pointer pick ([`pick::at`]).
    pub fn under(&self, point: [i32; 2]) -> Option<u16> {
        pick::at(
            &self.lot,
            &self.drawn_camera,
            &self.canvas,
            self.lot_depth_slice(),
            &self.people_ids,
            point,
        )
    }

    /// Lists the selected Sim's pie menu on `object` (`Lot::pie_menu`,
    /// which changes the lot), keeping its sounds for the host. True if
    /// the simulation listed it, and so may have changed the lot.
    pub(super) fn list(&mut self, object: u16) -> bool {
        if self.coop.is_active() {
            let Some(player_id) = self.coop.local_player_id() else {
                return false;
            };
            let Some(person) = self.local_selected_sim() else {
                return false;
            };
            let listing = menu::coop_entries(&self.lot, &self.assets, player_id, object);
            if let Ok(listing) = &listing {
                self.keep_sounds(listing.sounds.clone());
            }
            self.listed = Some((person, object, listing));
            // Throwaway co-op pie probe never mutates the shared lot.
            return false;
        }
        let Some(person) = self.lot.world().selected() else {
            return false;
        };
        let listing = menu::entries(&mut self.lot, &self.assets, person, object);
        let changed = listing.as_ref().is_ok_and(|l| l.unchecked.is_none());
        if !listing.as_ref().is_ok_and(|l| l.unchecked.is_some()) {
            // `Lot::pie_menu` commits check-tree side effects before `menu::named`
            // runs, so invalidate steady-state damage even if naming returns `Err`.
            self.prev_steady_key = None;
            self.present_damage = None;
        }
        if let Ok(listing) = &listing {
            self.keep_sounds(listing.sounds.clone());
        }
        self.listed = Some((person, object, listing));
        changed
    }

    /// Opens the pie menu of `object` at `point` for the selected Sim:
    /// the one listed when the pointer reached it, or, when it was not
    /// listed then (the pointer did not move onto it first), one listed
    /// now.
    pub(super) fn open_menu(&mut self, object: u16, point: [i32; 2]) {
        if self.stopped.is_some() {
            self.note = Some("The lot has stopped, so it takes no orders.".into());
            return;
        }
        let Some(person) = self.local_selected_sim() else {
            self.note = Some("Nobody is selected to do anything.".into());
            return;
        };
        // The press lists the menu again: the live mode's press handler
        // (0x48cbec) first sets the object under the pointer to none
        // (0x48c834 with 0), so the press's own pick lists it anew.
        self.list(object);
        self.reached = Some(object);
        let Some((_, _, listing)) = self.listed.take() else {
            return;
        };
        match listing {
            Ok(listing) => {
                // Where the simulation cannot list the menu yet, the note
                // says so and why.
                let why = listing
                    .unchecked
                    .map(|reason| format!("the simulation cannot list this menu yet: {reason}"));
                if listing.entries.is_empty() {
                    let line = format!("Object {object} offers nothing to do");
                    self.note = Some(match why {
                        Some(why) => format!("{line} ({why})."),
                        None => format!("{line}."),
                    });
                } else {
                    self.note = why.map(|why| format!("Every entry is shown, as {why}"));
                    self.highlighted = None;
                    let menu = if self.pie_animations_active() {
                        Menu::new_animated(
                            person,
                            object,
                            listing.entries,
                            point,
                            &self.hud.art,
                            self.view.size,
                            (self.clock_us / 1_000) as u32,
                        )
                    } else {
                        Menu::new(
                            person,
                            object,
                            listing.entries,
                            point,
                            &self.hud.art,
                            self.view.size,
                        )
                    };
                    // Native menu setup reads the current cursor (0x5ebd05)
                    // after any on-screen clamp warp (0x5ebab2) and updates
                    // hit state (0x5ebd29), so the opening frame aims the head
                    // at the press or warped center rather than a stale hover.
                    let effective = menu.pending_pointer().unwrap_or(point);
                    self.pointer = Some(effective);
                    self.hud.set_pointer(effective);
                    self.hud.menu = Some(menu);
                }
            }
            Err(e) => self.note = Some(format!("Object {object}'s menu is not listed: {e}")),
        }
    }

    /// The control panel as it was last laid out, from the world as it
    /// was when the frame was drawn ([`Game::render`]).
    pub fn panel(&self) -> &[sims_ui::hud::Item] {
        self.hud.items()
    }

    /// What the control panel shows of `world`: [`Game::render`] lays
    /// the panel out from this for the lot's world.
    pub fn snapshot(&self, world: &sims_sim::World) -> sims_ui::household::Snapshot {
        self.hud.snapshot(world, self.lot.residential())
    }

    /// The first name the panel shows for people of type `guid`.
    pub fn name(&self, guid: u32) -> Option<&str> {
        self.hud.name(guid)
    }

    /// The pie menu, while one is open. Its labels are placed when the
    /// game is rendered.
    pub fn menu(&self) -> Option<&Menu> {
        self.hud.menu.as_ref()
    }

    /// Where the window is to move the pointer (canvas pixels), once:
    /// the original moves it to the middle of a pie menu that opened
    /// elsewhere than asked, and of a submenu that opens.
    pub fn take_pointer(&mut self) -> Option<[i32; 2]> {
        self.hud.take_pointer()
    }

    /// Closes the pie menu; false if none was open.
    pub fn close_menu(&mut self) -> bool {
        let closed = self.hud.menu.take().is_some();
        if closed {
            self.highlighted = None;
        }
        closed
    }

    /// Handles Escape in Options mode (`cWinOptions::OnKeyDown`,
    /// `0x517904..0x517954`): closes any open Options subpanel tab (and help
    /// balloon) and consumes Escape so pressing Escape in Options never exits
    /// the game directly. Returns `false` outside Options mode.
    pub fn escape_options(&mut self) -> bool {
        if self.hud.controls.mode != Mode::Options {
            return false;
        }
        if self.hud.controls.options.tab.take().is_some() {
            // `0x5175ab..0x5175bb -> 0x50fb7f`: closing a tab also dismisses
            // any open Options help balloon with `UI_whoosh` before `UI_click`.
            if self.hud.controls.options.help.take().is_some() {
                self.hud.play(crate::hud::WHOOSH);
            }
            self.hud.play(crate::hud::CLICK);
            self.hud.relayout_panel(
                self.view.size,
                self.lot.world(),
                self.lot.residential(),
                self.lot.lot_zoning(),
                self.lot_visibility(),
                &self.assets,
                None,
            );
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
        } else if self.hud.controls.options.help.take().is_some() {
            self.hud.play(crate::hud::WHOOSH);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
        }
        true
    }

    /// Queues `entry` of the pie menu on `object` for `person`, as
    /// choosing it does: the action the simulation offered
    /// (`Lot::pick`, on the action's caller), or for a fallback entry the
    /// entry's menu number (`Lot::queue_interaction`).
    pub(super) fn order(&mut self, person: u16, object: u16, entry: &menu::Entry) {
        let label = &entry.label;
        // A picked action is queued on its caller: `person`, but for the
        // offer to join `person`'s running interaction.
        let on = entry
            .action
            .as_ref()
            .and_then(|a| u16::try_from(a.caller).ok())
            .unwrap_or(person);
        if self.coop.is_active() {
            let cmd = match &entry.action {
                Some(action) => NetCommand::SimActionPick {
                    sim_id: action.caller as u16,
                    target_id: action.target as u16,
                    menu_index: action.table_index as i16,
                    joining: action.join != 0,
                    args: action.args,
                },
                None => NetCommand::SimActionPick {
                    sim_id: person,
                    target_id: object,
                    menu_index: entry.menu as i16,
                    joining: false,
                    args: [0; 4],
                },
            };
            self.note = Some(match self.coop.submit_command(cmd) {
                Ok(()) => format!("Sent \"{label}\" for person {on}."),
                Err(e) => format!("\"{label}\" was not sent: {e}"),
            });
            return;
        }
        let queued = match &entry.action {
            Some(action) => self.lot.pick(person, action.clone()),
            None => self.lot.queue_interaction(person, object, entry.menu),
        };
        self.note = Some(match queued {
            Ok(Some(_)) => {
                self.dirty = true;
                format!("Queued \"{label}\" for person {on}.")
            }
            // A full queue, an entry gone from the table, or a check
            // that refused an action run at once.
            Ok(None) => format!("\"{label}\" was not queued for person {on}."),
            Err(e) => format!("\"{label}\" was not queued: {e}"),
        });
    }

    /// Cancels `sim`'s action numbered `action`, as a press on its icon
    /// does (`Lot::cancel`, 0x4ea3e3), keeping the sounds the trees made,
    /// then plays `UI_queue_delete` if the press counts and `UI_error` if
    /// not (0x4cb16e). A cancel the lot refuses, or a stopped lot, leaves
    /// the icon to take another press.
    pub(super) fn cancel(&mut self, sim: u16, action: i32) {
        if self.stopped.is_some() {
            self.note = Some("The lot has stopped, so it takes no orders.".into());
            self.hud.release(action);
            return;
        }
        if self.coop.is_active() {
            let cmd = NetCommand::SimActionCancel {
                sim_id: sim,
                interaction_uid: sims_vm::coop::cancel_uid_to_wire(action),
            };
            match self.coop.submit_command(cmd) {
                Ok(()) => self.hud.play(crate::hud::QUEUE_DELETE),
                Err(e) => {
                    self.hud.release(action);
                    self.note = Some(format!("The action cancel was not sent: {e}"));
                }
            }
            return;
        }
        match self.lot.cancel(sim, action) {
            Ok(cancelled) => {
                if cancelled.counts {
                    self.dirty = true;
                }
                self.keep_sounds(cancelled.sounds);
                self.hud.play(if cancelled.counts {
                    crate::hud::QUEUE_DELETE
                } else {
                    crate::hud::ERROR
                });
            }
            Err(e) => {
                // The lot is as it was, so the icon takes another press.
                self.hud.release(action);
                self.note = Some(format!("The action was not cancelled: {e}"));
            }
        }
    }

    /// The selected Sim's actions as the frame last drawn showed them.
    pub fn queue(&self) -> &sims_ui::queue::Queue {
        self.hud.queue()
    }

    /// Answers the dialog `caller` shows with its button `button` (0 the
    /// first): the answer the button gives (`Shown::answers`) goes to the
    /// lot, and the tick that showed it runs again at once.
    pub fn answer(&mut self, caller: u16, button: u8) {
        let Some(asking) = self.lot.asking() else {
            self.note = Some("No dialog waits for an answer.".into());
            return;
        };
        if asking.shown.object != caller {
            self.note = Some(format!(
                "The dialog shown is object {}'s, not object {caller}'s.",
                asking.shown.object
            ));
            return;
        }
        let Some(&answer) = asking.shown.answers.get(usize::from(button)) else {
            self.note = Some(format!("The dialog has no button {button}."));
            return;
        };
        let text = self.hud.active_dialog_entry().map(str::to_owned);
        if self.coop.is_active() {
            let dialog_id = self.lot.coop().and_then(|c| c.active_dialog).or_else(|| {
                self.coop
                    .session()
                    .and_then(|s| s.open_dialogs.keys().next().copied())
            });
            let Some(dialog_id) = dialog_id else {
                self.note = Some("No co-op blocking dialog is active.".into());
                return;
            };
            let cmd = NetCommand::AnswerBlockingDialog {
                dialog_id,
                answer: sims_net::DialogAnswer {
                    status: answer,
                    bitmap_id: Some(sims_ui::dialog::BUTTON as i16),
                    text,
                },
            };
            if let Err(e) = self.coop.submit_command(cmd) {
                self.note = Some(format!("The answer was not sent: {e}"));
            } else {
                self.hud.clear_vm_dialog();
            }
            return;
        }
        if let Err(e) = self.lot.answer(sims_vm::Answer {
            status: answer,
            bitmap_id: Some(sims_ui::dialog::BUTTON as i16),
            text,
        }) {
            self.note = Some(format!("The answer was not taken: {e}"));
            return;
        }
        // `0x523825..0x523887`: clear the cached VM dialog before replaying the
        // tick so a consecutive prompt with the same caller & operands starts
        // with a freshly initialized entry field.
        self.hud.clear_vm_dialog();
        // The tick that showed the dialog runs again now, with the answer,
        // so nothing is listed or ordered on the lot as it was before it.
        self.run(1);
    }

    /// A press on the portrait of the Sim `sim` (0x4cf9c4). On another
    /// Sim's portrait it selects `sim` (`Lot::select`, through 0x4e989b
    /// and 0x445449), keeping the sounds that made, and drops a menu
    /// listed for the Sim selected before. On the selected Sim's own
    /// portrait it centres the view on the Sim instead
    /// ([`Game::centre_on`]). Either way it then plays `UI_click`
    /// (0x4cfa51).
    ///
    /// On a residential lot 0x4e989b selects the person only when person
    /// word 32 is 0 (it lives here), and nobody otherwise; the portraits
    /// show only the household's people, whose word is 0, so that is not
    /// checked here. Not ported: a person the household's table names
    /// but who is not on the lot (0x4e9652), and the people panel's call
    /// on a press on the selected Sim's portrait (0x51ea7a, behind a
    /// check of global 0x6f6bac) whose effect has not been read.
    pub fn select(&mut self, sim: u16) {
        let world = self.lot.world();
        let current_selected = self.local_selected_sim();
        let same_neighbor = current_selected == Some(sim)
            || current_selected
                .and_then(|id| world.objects.get(&id))
                .zip(world.objects.get(&sim))
                .is_some_and(|(selected, target)| selected.guid == target.guid);
        if same_neighbor {
            // 0x4cfa0d..0x4cfa47: centres on the first live person of that
            // neighbor in person-registry order (`0x4e9608`).
            let target = world
                .objects
                .get(&sim)
                .and_then(|o| sims_ui::household::first_person_for_guid(world, o.guid))
                .map_or(sim, |o| o.id);
            let smooth = self.smooth_pan_active();
            self.centre_on_with_pan(target, smooth);
            self.hud.play(crate::hud::CLICK);
            return;
        }
        if self.coop.is_active() {
            match self
                .coop
                .submit_command(NetCommand::SelectPerson { sim_id: Some(sim) })
            {
                Ok(()) => {
                    self.close_menu();
                    self.reached = None;
                    self.listed = None;
                    self.highlighted = None;
                    self.hud.play(crate::hud::CLICK);
                }
                Err(e) => {
                    self.note = Some(format!("Person {sim} was not selected: {e}"));
                }
            }
            return;
        }
        match self.lot.select(sim) {
            Ok(sounds) => {
                self.dirty = true;
                self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                self.keep_sounds(sounds);
                self.hud.play(crate::hud::CLICK);
            }
            Err(e) => self.note = Some(format!("Person {sim} was not selected: {e}")),
        }
        self.listed = None;
        self.reached = None;
    }

    /// Centres the view on the object `id` as 0x434ced does with 1, but
    /// at once: the original slides the view there over a while
    /// (0x4c9a27, the time at 0x6c7788), which is left out.
    ///
    /// Nothing happens unless object data word 34 is 0 and word 29 is not
    /// negative. For a multi-tile object, `0x4071a3` walks the group from
    /// its lead tile through `next_parts` and averages the parts' `x` and
    /// `y` positions before rotating and projecting. The original's house
    /// view sits above the 100-pixel live-mode strip
    /// (`0x49ae5e` → `0x5cf1b6`, `0x4b136b`), so its middle is
    /// `[width / 2, height / 2]` over [`Game::house_viewport_size`] (full
    /// canvas while the strip is collapsed, `height - 100` while expanded).
    /// A person on the floor shown is
    /// centred a little above its feet: its screen point on the lot's
    /// terrain ([`sound::object_point_on_lot`]) raised by half of 2 √6
    /// times 2.0 times 2 to the power of the zoom plus one, cut to a whole
    /// number (19, 39 and 78 pixels from far to near), comes to the house
    /// view's middle (0x4a7777). Anything else, or a person on another
    /// floor, shows its floor (0x4a80c7) and, unless it stands off the
    /// lot's rotated tile bounds (0x4a7af0), its screen point on that
    /// floor comes to the middle (0x4a7996). Both paths then clamp the
    /// origin to the scroll limits (0x4a9fa9) and align its x coordinate
    /// with `(x + 1) & !3` (0x4a81cc).
    pub fn centre_on(&mut self, id: u16) {
        self.centre_on_with_pan(id, false);
    }

    pub(super) fn centre_on_with_pan(&mut self, id: u16, smooth: bool) {
        let world = self.lot.world();
        let Some(object) = world.objects.get(&id) else {
            return;
        };
        let word = |i: usize| object.data.get(i).copied().unwrap_or(0);
        if word(34) != 0 || word(29) < 0 {
            return;
        }
        let is_person = object.person.is_some();
        let Some(position) = centre_position(world, id) else {
            return;
        };
        let level = position[2];
        let (w, h) = self.house_viewport_size();
        let view_size = [w as i32, h as i32];
        if is_person && level == self.view.camera.level {
            let mut at = lot_point(&self.view.camera, &self.lot_shape, position);
            at[1] -= person_rise(self.view.camera.zoom.level());
            self.centre_screen_point_with_pan(at, view_size, smooth);
            return;
        }
        if self.view.apply(Move::Floor(level)) {
            self.sync_view();
        }
        if !self.on_lot(position) {
            return;
        }
        let at = lot_point(&self.view.camera, &self.lot_shape, position);
        self.centre_screen_point_with_pan(at, view_size, smooth);
    }

    /// The active 3D house viewport size (`0x49ae5e`, `0x4f4b43`, `0x4d363b`,
    /// `0x53b2eb` → `0x4b136b` → `0x49b0bf` → `0x5cf1b6`): full canvas
    /// `(width, height)` while the bottom strip is closed, and
    /// `(width, height - STRIP)` (`height - 150 + 50` from the shared
    /// 150-pixel panel container at `0x4f4b43`) while the strip is open in
    /// Live, Build, Buy, or Options mode.
    pub fn house_viewport_size(&self) -> (u32, u32) {
        viewport_for_controls(self.view.size, &self.hud.controls)
    }

    /// Whether `position` lies on the active lot's rotated tile bounds
    /// (`0x4a7af0`: rotated fine position shifted right by 4, narrowed to
    /// signed bytes, checked against `lot + 0x70..+0x7c`).
    pub(super) fn on_lot(&self, position: [i32; 3]) -> bool {
        if self.lot_shape.size == 0 {
            return false;
        }
        let rotated = sound::rotate_object_fine(&self.view.camera, [position[0], position[1]]);
        let tx = i32::from((rotated[0] >> 4) as i8);
        let ty = i32::from((rotated[1] >> 4) as i8);
        let b = sound::lot_bounds(&self.lot_shape, &self.view.camera);
        tx >= b[0] && tx <= b[1] && ty >= b[2] && ty <= b[3]
    }

    /// Moves the camera origin so screen point `at` comes to the middle
    /// of `view_size`, clamped to the original's scroll limits
    /// (`0x4a9fa9`) and x-aligned via `(x + 1) & !3` (`0x4a81cc`).
    ///
    /// `sims-render` draws walls and floors at `camera.origin[0]` and
    /// objects/people at `camera.origin[0] + 1`, whereas the original
    /// draws walls and floors one column left of `DAT_006f00d0`
    /// (`0x4ac14b`, `0x4ac25d`) and objects/people at it. So `0x4a9fa9`
    /// and `0x4a81cc` compute the original's `origin_x`, and we store
    /// `origin_x - pick::ORIGIN_COLUMN` so both walls and objects land on
    /// the original's exact screen columns.
    pub(super) fn centre_screen_point_with_pan(
        &mut self,
        at: [i32; 2],
        view_size: [i32; 2],
        smooth: bool,
    ) {
        let middle = [view_size[0] / 2, view_size[1] / 2];
        let origin = self.view.camera.origin;
        let target = [
            origin[0].saturating_add(middle[0].saturating_sub(at[0])),
            origin[1].saturating_add(middle[1].saturating_sub(at[1])),
        ];
        let ([min_x, max_x], [min_y, max_y]) =
            scroll_limits(&self.lot_shape, &self.view.camera, view_size);
        let native_x = target[0].max(min_x).min(max_x).wrapping_add(1) & !3;
        let clamped_x = native_x.saturating_sub(pick::ORIGIN_COLUMN);
        let clamped_y = target[1].max(min_y).min(max_y);
        let goal = [clamped_x, clamped_y];
        if smooth && origin != goal {
            if self.camera_pan.is_some_and(|pan| pan.target_origin == goal) {
                return;
            }
            self.camera_pan = Some(CameraPan {
                start_origin: origin,
                target_origin: goal,
                start_us: self.clock_us,
                duration_us: CAMERA_PAN_DURATION_US,
            });
            self.camera_pan_moved = true;
            return;
        }
        self.camera_pan = None;
        if self.view.set_origin(goal) {
            self.sync_view();
        }
    }
}
