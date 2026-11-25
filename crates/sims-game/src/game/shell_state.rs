//! Animation, cheat and cheat console, camera strip, travel request, debug, roof, voice and sound state.

use super::*;

impl Game {
    /// Whether the player is currently holding an interactive tool, drag,
    /// slider, or modal dialog that benefits from display-refresh redraws.
    pub fn is_actively_interacting(&self) -> bool {
        self.buy_cursor.is_some()
            || self.coop_buy_preview.is_some()
            || self.build_drag.is_some()
            || self.wall_cursor.is_some()
            || self.shop_dialog.is_open()
            || self.phonebook_dialog.is_open()
            || self.coop_lobby.is_open()
            || self.hud.dragging_slider()
            || self.hud.dragging_dialog_entry()
            || self.has_dialog()
            || self.pie_animating()
            || self.camera_panning()
            || self.ui_animating()
            || self.cheat_console_open()
    }

    /// Whether any temporal UI animation (250 ms help-balloon fade, modal
    /// dialog fade-in, or 500 ms action-queue icon slide) is currently in flight.
    /// Always `false` when `clock_us == 0` (static/headless frames).
    pub fn ui_animating(&self) -> bool {
        self.hud.is_ui_animating(self.clock_us)
    }

    /// Current Live help-balloon fade state (`BalloonFadeState`).
    pub fn balloon_fade(&self) -> &BalloonFadeState {
        self.hud.balloon_fade()
    }

    /// Current modal dialog fade state (`DialogFadeState`).
    pub fn dialog_fade(&self) -> &DialogFadeState {
        self.hud.dialog_fade()
    }

    /// Current action-queue icon slide state (`QueueSlideState`).
    pub fn queue_slide(&self) -> &QueueSlideState {
        self.hud.queue_slide()
    }

    /// Enables or disables smooth camera pan interpolation on portrait
    /// double-click / re-click (also enabled automatically once `clock_us > 0`).
    pub fn set_smooth_camera_pan(&mut self, enabled: bool) {
        self.smooth_camera_pan = enabled;
        if !enabled {
            self.camera_pan = None;
        }
    }

    pub(super) fn smooth_pan_active(&self) -> bool {
        self.smooth_camera_pan || self.clock_us > 0
    }

    /// Whether a smooth camera pan interpolation is currently in flight.
    pub fn camera_panning(&self) -> bool {
        self.camera_pan.is_some()
    }

    /// Returns and clears whether the camera origin moved during the last
    /// smooth camera pan step.
    pub fn take_camera_pan_moved(&mut self) -> bool {
        std::mem::take(&mut self.camera_pan_moved)
    }

    pub(super) fn step_camera_pan(&mut self) -> bool {
        let Some(pan) = self.camera_pan else {
            return false;
        };
        let (origin, finished) = pan.sample(self.clock_us);
        let moved = self.view.set_origin(origin);
        if finished {
            self.camera_pan = None;
            self.sync_view();
        }
        if moved {
            self.camera_pan_moved = true;
        }
        moved || finished
    }

    /// Enables or disables 400 ms pie-menu pop-in / submenu glide / direction
    /// ease-out animations explicitly (also enabled automatically once `clock_us > 0`).
    pub fn set_pie_animations(&mut self, enabled: bool) {
        self.pie_animations = enabled;
    }

    pub(super) fn pie_animations_active(&self) -> bool {
        self.pie_animations || self.clock_us > 0
    }

    /// Whether an open pie menu currently has an active 400 ms animation
    /// (backdrop pop-in, submenu glide, or head direction ease-out).
    pub fn pie_animating(&self) -> bool {
        let now_ms = (self.clock_us / 1_000) as u32;
        self.hud
            .menu
            .as_ref()
            .is_some_and(|m| m.is_animating(now_ms))
    }

    /// Whether the lot's live `World` was replaced via `Lot::restore` (e.g.
    /// Save Manager load or world-mutating cheat), clearing `neighborhood_origin`.
    pub fn world_mutated_via_restore(&self) -> bool {
        self.world_mutated_via_restore
    }

    /// Monotonic counter bumped on each `Game::restore` that resets `errors()`.
    pub fn restore_generation(&self) -> u64 {
        self.restore_generation
    }

    /// Marks the lot clean after an external host/REDB save succeeds.
    pub fn mark_saved_clean(&mut self, note: impl Into<String>) {
        self.dirty = false;
        self.note = Some(note.into());
    }

    /// Whether `move_objects on` is currently enabled on the live lot.
    pub fn cheat_move_objects(&self) -> bool {
        self.lot.move_objects()
    }

    /// Whether `map_edit on` is currently enabled via the cheat console.
    pub fn cheat_map_edit(&self) -> bool {
        self.cheat_map_edit
    }

    /// Last `autonomy <0..=100>` level applied via the cheat console, if any.
    pub fn cheat_autonomy(&self) -> Option<u8> {
        self.cheat_autonomy
    }

    /// Last `sim_speed <-1000..=1000>` value applied via the cheat console, if any.
    pub fn cheat_sim_speed(&self) -> Option<i16> {
        self.cheat_sim_speed
    }

    /// Screen rectangle of the Options-strip "Saves" button when visible.
    pub fn options_saves_button_rect(&self) -> Option<sims_ui::frame::Rect> {
        self.hud.options_saves_button_rect()
    }

    /// Screen rectangle of the Options-strip "Co-op" button when visible.
    pub fn options_coop_button_rect(&self) -> Option<sims_ui::frame::Rect> {
        self.hud.options_coop_button_rect()
    }

    /// Read-only access to the classic top-left cheat console (`Ctrl+Shift+C`).
    pub fn cheat_console(&self) -> &sims_ui::cheat_console::CheatConsole {
        &self.hud.cheat_console
    }

    /// Whether the classic cheat console overlay is currently open.
    pub fn cheat_console_open(&self) -> bool {
        self.hud.cheat_console.is_open()
    }

    /// Toggles the classic cheat console (`Ctrl+Shift+C` / `Cmd+Shift+C`).
    pub fn toggle_cheat_console(&mut self) -> bool {
        self.hud.cheat_console.toggle()
    }

    /// Closes the cheat console if open; returns `true` if it was open.
    pub fn close_cheat_console(&mut self) -> bool {
        if self.hud.cheat_console.is_open() {
            self.hud.cheat_console.close();
            true
        } else {
            false
        }
    }

    pub fn cheat_console_char(&mut self, ch: char) -> bool {
        self.hud.cheat_console.insert_char(ch)
    }

    pub fn cheat_console_backspace(&mut self) -> bool {
        self.hud.cheat_console.backspace()
    }

    pub fn cheat_console_delete(&mut self) -> bool {
        self.hud.cheat_console.delete_forward()
    }

    pub fn cheat_console_left(&mut self) -> bool {
        self.hud.cheat_console.move_left()
    }

    pub fn cheat_console_right(&mut self) -> bool {
        self.hud.cheat_console.move_right()
    }

    pub fn cheat_console_home(&mut self) -> bool {
        self.hud.cheat_console.move_home()
    }

    pub fn cheat_console_end(&mut self) -> bool {
        self.hud.cheat_console.move_end()
    }

    pub fn cheat_console_history_up(&mut self) -> bool {
        self.hud.cheat_console.history_up()
    }

    pub fn cheat_console_history_down(&mut self) -> bool {
        self.hud.cheat_console.history_down()
    }

    /// Submits the current cheat console input line and executes every emitted
    /// [`sims_ui::cheat_console::CheatCommand`] against the active lot.
    pub fn cheat_console_submit(
        &mut self,
    ) -> Result<Vec<sims_ui::cheat_console::CheatCommand>, sims_ui::cheat_console::CheatParseError>
    {
        let commands = self.hud.cheat_console.submit()?;
        self.execute_cheat_commands(&commands);
        Ok(commands)
    }

    /// Executes a batch of parsed [`sims_ui::cheat_console::CheatCommand`]s
    /// cleanly against `self` and updates console feedback.
    pub fn execute_cheat_commands(&mut self, commands: &[sims_ui::cheat_console::CheatCommand]) {
        use sims_ui::cheat_console::{CheatCommand, FeedbackKind};

        if self.coop.is_active_or_starting() {
            self.hud.cheat_console.set_feedback(
                FeedbackKind::Error,
                "Cheats are disabled during co-op multiplayer sessions.",
            );
            return;
        }

        let mut total_funds_delta: i32 = 0;
        let mut target_hour: Option<u8> = None;
        let mut messages: Vec<String> = Vec::new();
        let mut saw_help = false;

        for cmd in commands {
            match *cmd {
                CheatCommand::AddFunds(amount) => {
                    total_funds_delta = total_funds_delta.saturating_add(amount);
                }
                CheatCommand::SetHour(hour) => {
                    target_hour = Some(hour);
                }
                CheatCommand::MoveObjects(on) => {
                    self.lot.set_move_objects(on);
                    messages.push(format!("move_objects {}", if on { "on" } else { "off" }));
                }
                CheatCommand::MapEdit(on) => {
                    self.cheat_map_edit = on;
                    self.lot.set_edit_budget(on);
                    messages.push(format!("map_edit {}", if on { "on" } else { "off" }));
                }
                CheatCommand::WaterTool => {
                    self.set_mode(Mode::Build);
                    self.select_build_tool(BuildTool::Water);
                    messages.push("Water tool selected".into());
                }
                CheatCommand::Autonomy(level) => {
                    self.cheat_autonomy = Some(level);
                    let free_will = level > 0;
                    let _ = self.lot.set_free_will(free_will);
                    self.hud.controls.options.play.free_will = free_will;
                    messages.push(format!("Autonomy set to {level}"));
                }
                CheatCommand::SimSpeed(val) => {
                    self.cheat_sim_speed = Some(val);
                    let speed = if val > 600 {
                        Speed::Ultra
                    } else if val > 200 {
                        Speed::Fast
                    } else {
                        Speed::Normal
                    };
                    self.set_speed(speed);
                    messages.push(format!("sim_speed {val}"));
                }
                CheatCommand::Interests => {
                    self.set_mode(Mode::Live);
                    self.hud.controls.expanded = true;
                    self.hud.controls.panel = Some(sims_ui::command::Panel::Interests);
                    messages.push("Interests panel opened".into());
                }
                CheatCommand::PrepareLot => {
                    self.buy_cursor = None;
                    self.buy_selected = None;
                    self.wall_cursor = None;
                    self.build_drag = None;
                    self.build_preview_shape = None;
                    self.refresh_world();
                    self.clear_lot_cache();
                    messages.push("Lot prepared".into());
                }
                CheatCommand::Help => {
                    saw_help = true;
                }
            }
        }

        if total_funds_delta != 0 {
            let edit_budget = self.lot.world().globals.get(32).copied().unwrap_or(0) != 0;
            match self.lot.add_funds(total_funds_delta) {
                Ok(_) => {
                    if edit_budget {
                        messages.push("Funds unchanged in edit-budget mode".into());
                    } else {
                        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                        self.dirty = true;
                        messages.push(format!("Added ${total_funds_delta}"));
                    }
                }
                Err(err) => {
                    self.hud
                        .cheat_console
                        .set_feedback(FeedbackKind::Error, format!("Cheat failed: {err}"));
                    return;
                }
            }
        }

        if let Some(hour) = target_hour {
            match self.lot.set_hour(hour) {
                Ok(()) => {
                    self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                    self.dirty = true;
                    self.clear_lot_cache();
                    let world = self.lot.world();
                    if let Ok(shape) = sims_render::Lot::from_world(world) {
                        self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation());
                    }
                    self.hud.sync_clock_blink(self.lot.world(), self.clock_us);
                    messages.push(format!("Hour set to {hour:02}:00"));
                }
                Err(err) => {
                    self.hud
                        .cheat_console
                        .set_feedback(FeedbackKind::Error, format!("Cheat failed: {err}"));
                    return;
                }
            }
        }

        if !messages.is_empty() {
            self.hud
                .cheat_console
                .set_feedback(FeedbackKind::Ok, messages.join("; "));
        } else if !saw_help {
            self.hud
                .cheat_console
                .set_feedback(FeedbackKind::Info, "Done.");
        }
    }

    /// Releases any active Buy/Build preview cursors or drags so a subsequent
    /// `Lot::restore` never leaves stale handles in `Game`.
    pub(super) fn cancel_active_buy_build_tools(&mut self) {
        if let Some(cursor) = self.buy_cursor.take() {
            if cursor.is_existing() {
                let _ = self.lot.buy_snap_back(&cursor);
            } else {
                let _ = self.lot.buy_cancel(&cursor);
            }
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        }
        self.buy_selected = None;
        self.buy_in_house = false;
        self.buy_dragging = None;
        self.buy_commit_on_release = false;
        if let Some(cursor) = self.wall_cursor.take() {
            let _ = self.lot.build_cursor_close(&cursor);
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        }
        self.build_drag = None;
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
    }

    /// Applies an in-place mutation to the live `World` via lossless `Saved`
    /// round-tripping while preserving UI mode, camera, and open cheat console.
    pub(super) fn mutate_world_via_saved(
        &mut self,
        mutator: impl FnOnce(&mut sims_sim::World),
    ) -> Result<(), String> {
        self.cancel_active_buy_build_tools();
        let saved = self.save()?;
        let orig_world_body = saved.world().encode();
        let encoded = saved.encode();
        let saved_body = encoded
            .get(8..)
            .ok_or_else(|| "invalid Saved snapshot header".to_owned())?;
        let tail = saved_body
            .strip_prefix(orig_world_body.as_slice())
            .ok_or_else(|| "Saved snapshot world prefix mismatch".to_owned())?;
        let mut world = saved.world().clone();
        mutator(&mut world);
        let new_world_body = world.encode();
        let mut rebuilt = Vec::with_capacity(8 + new_world_body.len() + tail.len());
        rebuilt.extend_from_slice(&encoded[..8]);
        rebuilt.extend_from_slice(&new_world_body);
        rebuilt.extend_from_slice(tail);
        let patched = Saved::decode(&rebuilt).map_err(|e| e.to_string())?;
        self.lot.restore(patched).map_err(|e| e.to_string())?;
        self.world_mutated_via_restore = true;
        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        self.dirty = true;
        self.clear_lot_cache();
        let world = self.lot.world();
        if !self.open_pack_mode {
            self.people.refresh(world);
        }
        if let Ok(shape) = sims_render::Lot::from_world(world) {
            self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation());
        }
        self.hud.sync_clock_blink(self.lot.world(), self.clock_us);
        Ok(())
    }

    /// Current lot geometry and render assets for capturing map thumbnails on save.
    pub fn render_parts_mut(
        &mut self,
    ) -> (
        &sims_sim::World,
        &sims_render::Lot,
        &mut sims_render::Assets,
    ) {
        (self.lot.world(), &self.lot_shape, &mut self.assets)
    }

    /// The lot, for reading.
    pub fn lot(&self) -> &Lot {
        &self.lot
    }

    /// Mutable access to the active [`Lot`] (used by travel departure/return).
    pub fn lot_mut(&mut self) -> &mut Lot {
        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        &mut self.lot
    }

    /// Enables or disables the Camera subpanel's Photo Album action (`CamViewAlbum`, `1105`).
    pub fn set_album_button_enabled(&mut self, enabled: bool) {
        self.hud.set_album_button_enabled(enabled);
    }

    /// Current native Camera Mode settings (`SnapshotSize`, `SnapshotQuality`,
    /// `album_enabled`, `quick_tips`).
    pub fn camera_state(&self) -> CameraState {
        self.hud.camera_state()
    }

    /// Active native `cWinCamera` panel controller.
    pub fn camera_panel(&self) -> &CameraPanel {
        self.hud.camera_panel()
    }

    /// Loaded `STR# 140` Camera Mode strings (button coordinates, tooltips, Quick Tips).
    pub fn camera_strings(&self) -> &CameraStrings {
        self.hud.camera_strings()
    }

    /// Last laid-out native [`CameraLayout`] when Camera Mode is open.
    pub fn camera_layout(&self) -> Option<&CameraLayout> {
        self.hud.camera_layout()
    }

    /// Screen rectangle of a specific native `cWinCamera` button when Camera Mode is open.
    pub fn camera_button_rect(&self, button: CameraButton) -> Option<sims_ui::frame::Rect> {
        self.hud.camera_button_rect(button)
    }

    /// Whether the bottom native Camera subpanel (`cWinCamera`) is currently open.
    pub fn camera_strip_open(&self) -> bool {
        self.hud.camera_strip_open()
    }

    /// Closes the bottom Camera subpanel if open. Returns `true` if it was open.
    pub fn close_camera_strip(&mut self) -> bool {
        self.hud.close_camera_strip()
    }

    /// Bounding rectangle of the bottom Camera subpanel when open.
    pub fn camera_strip_rect(&self) -> Option<sims_ui::frame::Rect> {
        self.hud.camera_strip_rect()
    }

    /// Screen rectangle of the native `CamViewAlbum` (`1105`) button, if open and enabled.
    pub fn album_button_rect(&self) -> Option<sims_ui::frame::Rect> {
        self.hud.album_button_rect()
    }

    /// Supplies real installed neighborhood/destination lot metadata (names,
    /// addresses, zoning) to destination lot pickers (`Dialog` kinds `5, 7, 8, 11, 13`).
    pub fn set_special_lot_catalog(&mut self, lots: Vec<SpecialLotEntry>) {
        self.special_lot_catalog = lots;
    }

    /// Sets the owning neighborhood slot (`1..=8`) so graphical destination
    /// map pickers load the matching `UserData*` map.
    pub fn set_neighborhood_slot(&mut self, slot: u8) {
        self.neighborhood_slot = slot.clamp(1, 8);
    }

    /// Whether the graphical destination map picker (`Dialog` kinds `5, 7, 8, 11, 13`)
    /// is currently open.
    pub fn destination_picker_open(&self) -> bool {
        self.destination_picker.is_some()
    }

    /// Active graphical [`DestinationMapPicker`], when a destination lot-picker
    /// special panel is open.
    pub fn destination_picker(&self) -> Option<&DestinationMapPicker> {
        self.destination_picker.as_ref()
    }

    /// Cancels the open graphical destination map picker (`Esc` or Return button),
    /// delivering `SpecialPanelResponse::Lot(None)` to the VM.
    pub fn destination_picker_escape(&mut self) -> Outcome {
        if self.destination_picker.is_none() {
            return Outcome::default();
        }
        if self.answer_special_panel(SpecialPanelResponse::Lot(None)) {
            self.hud.play(crate::hud::CLICK);
        } else {
            self.hud.play(crate::hud::ERROR);
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    /// Confirms the currently hovered visitable lot on the graphical
    /// destination map picker (`Enter`), if any.
    pub fn destination_picker_submit(&mut self) -> Outcome {
        let Some(lot_id) = self
            .destination_picker
            .as_ref()
            .and_then(DestinationMapPicker::hovered_lot)
        else {
            return Outcome::default();
        };
        if self.answer_special_panel(SpecialPanelResponse::Lot(Some(lot_id as i16))) {
            self.hud.play(crate::hud::CLICK);
        } else {
            self.hud.play(crate::hud::ERROR);
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    /// Takes any destination lot chosen in a VM special lot picker (`5, 7, 8,
    /// 11, 13`) and the picker it was chosen in. The pick does not travel by
    /// itself: the script asks for the trip ([`Game::trip_request`]), as the
    /// original changes house only through 0x4fc713 (Generic Sims Call 17's
    /// 0x45a22d, its one caller).
    pub fn take_pending_travel_lot(&mut self) -> Option<(SpecialPanelRequest, u32)> {
        self.pending_travel_lot.take()
    }

    /// The trip a script asked for with Generic Sims Call 17
    /// ([`sims_sim::World::trip_request`]) that the host has to carry out
    /// before the next tick, as the original's main loop does when it finds
    /// the game's byte +0x1c +0xe1 set between ticks. `None` once the host
    /// declined it ([`Game::decline_trip`]).
    ///
    /// While one is pending no tick runs: [`Game::advance`] and
    /// [`Game::run`] stop after the tick that asked for it.
    pub fn trip_request(&self) -> Option<&sims_sim::TripRequest> {
        self.lot.world().trip_request.as_ref().filter(|&trip| {
            !self
                .declined_trip
                .as_ref()
                .is_some_and(|d| same_trip(d, trip))
        })
    }

    /// Lets the lot go on without carrying out the pending trip
    /// ([`Game::trip_request`]): a lot without a neighborhood, or a trip the
    /// host refused. Returns the request dropped, if any.
    ///
    /// The lot drops the request ([`Lot::cancel_trip_request`]) and takes
    /// back the saved-state tokens routine 17 wrote for it, so the script's
    /// tail on this lot restores nothing from them and a later Generic Sims
    /// Call 17 asks again. The original has no refusal: its main loop always
    /// leaves the house (0x4fc713).
    ///
    /// In co-op the world changes only through the lockstep, so the request
    /// stays in the lot and is only marked declined here. A later Generic
    /// Sims Call 17 does not replace a pending request; it only adds the
    /// neighbors it saved (`saved`). So the trip stays declined, whatever
    /// later calls add, until the lot is left or reloaded, or until it ticks
    /// on its own again after co-op, which drops the request.
    pub fn decline_trip(&mut self) -> Option<sims_sim::TripRequest> {
        if self.coop.is_active_or_starting() {
            self.declined_trip = self.lot.world().trip_request.clone();
            return self.declined_trip.clone();
        }
        self.declined_trip = None;
        self.lot.cancel_trip_request()
    }

    /// Drops a trip [`Game::decline_trip`] only marked declined during
    /// co-op, just before the lot's next tick of its own. Left in the lot,
    /// the stale request would hide every later trip: Generic Sims Call 17
    /// only adds to a pending request, and the mark hides it (`same_trip`).
    /// A different request pending by then is not the one declined, so it
    /// stays and only the mark goes.
    pub(super) fn drop_trip_declined_in_coop(&mut self) {
        if self.coop.is_active_or_starting() {
            return;
        }
        let Some(declined) = self.declined_trip.take() else {
            return;
        };
        if self
            .lot
            .world()
            .trip_request
            .as_ref()
            .is_some_and(|trip| same_trip(&declined, trip))
        {
            self.lot.cancel_trip_request();
        }
    }

    /// Enables or disables `--debug` mode (which shows the maroon top
    /// diagnostic/status banner and logs diagnostic notes to stderr).
    pub fn set_debug(&mut self, debug: bool) {
        self.debug = debug;
        if !debug {
            self.last_debug_logged_note = None;
        }
    }

    /// Whether `--debug` diagnostic banner & stderr logging mode is active.
    pub fn debug(&self) -> bool {
        self.debug
    }

    /// Surfaces a genuine user-facing error or confirmation via a native Sims 1
    /// modal message dialog (`cTSWinMsgBox`, OK button), while also recording
    /// `body` in the programmatic [`Game::note`] getter (and logging to stderr
    /// when `--debug` is enabled).
    pub fn show_notice(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.pending_after_notice = None;
        let body = body.into();
        self.set_note(body.clone());
        self.hud.open_notice_dialog(title, body, self.view.size);
    }

    pub(super) fn log_debug_note_if_new(&mut self) {
        if !self.debug {
            return;
        }
        let current = self.note.as_deref();
        if current != self.last_debug_logged_note.as_deref() {
            if let Some(msg) = current {
                eprintln!("sims-game [debug]: {msg}");
            }
            self.last_debug_logged_note = current.map(str::to_owned);
        }
    }

    /// House baseline roof state read from `HOUS 0`.
    pub fn base_roof(&self) -> &RoofState {
        &self.base_roof
    }

    /// Active roof state (live `ROOF_META_ARRAY` edit or `HOUS 0` baseline).
    pub fn current_roof_state(&self) -> RoofState {
        self.lot.roof_state_with_base(&self.base_roof)
    }

    /// Laid-out HUD items from the last rendered frame.
    pub fn hud_items(&self) -> &[sims_ui::hud::Item] {
        self.hud.items()
    }

    /// Whether the Camera Roof preview toggle (`Camera::Roof`) is active.
    pub fn roof_view(&self) -> bool {
        self.roof_view
    }

    /// Sets the Camera Roof view (`Camera::Roof`, native 4-way radio state
    /// `[ui + 0x48] == 8`). Turning Roof view on also raises walls to
    /// [`Walls::Up`] so the roof never floats over lowered walls.
    pub fn set_roof_view(&mut self, on: bool) {
        if on {
            self.view.camera.walls = Walls::Up;
        }
        self.roof_view = on;
        self.hud.set_roof_view(on);
    }

    /// Queues a Simlish voice clip for live person `sim_id` (using its person
    /// data age/gender/pet flags) to be mixed on the next [`sound::Feed::update`].
    pub fn play_sim_voice(&mut self, sim_id: u16, category: Option<&str>, seed: u64) -> bool {
        let Some(person) = self
            .lot
            .world()
            .objects
            .get(&sim_id)
            .and_then(|o| o.person.as_ref())
        else {
            return false;
        };
        let age = person.data.get(58).copied().unwrap_or(27);
        let gender = person.data.get(65).copied().unwrap_or(0);
        let flags = person.data.get(74).copied().unwrap_or(0);
        let voice = sound::voice_for_person(age, gender, flags);
        self.voice_inputs.push_back(sound::Input::PlayVoice {
            voice,
            category: category.map(str::to_owned),
            pitch: sims_audio::voices::VoicePitch::NATIVE,
            seed,
            object: Some(u32::from(sim_id)),
        });
        true
    }

    /// Drains queued Simlish voice mixer inputs (called by [`sound::Feed::update`]).
    pub fn take_voice_inputs(&mut self) -> Vec<sound::Input> {
        self.voice_inputs.drain(..).collect()
    }

    /// The hour of the game's clock (global 0), 0 to 23.
    pub fn hour(&self) -> u32 {
        let hour = self.lot.world().globals.first().copied().unwrap_or(0);
        u32::try_from(hour.rem_euclid(24)).unwrap_or(0)
    }

    /// The ambience's camera value (`sims_audio::ambience::view`): the
    /// share, in percent, of the tiles on the screen that are outdoors,
    /// on the floor shown. The original works it out each frame from its
    /// camera (0x570c3e).
    pub fn ambience_view(&self) -> i32 {
        let camera = &self.view.camera;
        // The camera's arithmetic overflows for an origin this far off;
        // no view the controls make comes near it.
        if camera.origin.iter().any(|n| n.unsigned_abs() >= 1 << 26) {
            return 0;
        }
        let corner = camera
            .rotate_fine(camera.unproject_fine([0, 0]))
            .map(|n| n >> 4);
        let level = u8::try_from(camera.level).unwrap_or(0);
        let world = self.lot.world();
        sims_audio::ambience::view(self.view.size, camera.zoom.level(), corner, |tile| {
            let tile = camera.unrotate_tile(tile.map(i32::from));
            world.outside(level, tile).unwrap_or(false)
        })
    }

    /// What of the camera sends the sound's kind 33 when it changes
    /// ([`sound::Shot`]).
    pub fn shot(&self) -> sound::Shot {
        sound::Shot::of(&self.view.camera, self.view.size)
    }

    /// What the sound scripts may ask about the lot's objects, with where
    /// each is on the screen as the camera shows it now
    /// ([`sound::sources`], [`sound::screen`]).
    pub fn sound_sources(&self) -> sims_audio::sources::Snapshot {
        let world = self.lot.world();
        let mut snapshot = sound::sources(world);
        snapshot.screen = sound::screen(world, &self.lot_shape, &self.view.camera, self.view.size);
        snapshot
    }

    /// Screen-space sound projections alone, used by [`sound::Feed`] on pure
    /// camera pans to avoid rebuilding `objects` and `positions`.
    pub(crate) fn sound_screen(&self) -> BTreeMap<i32, sims_audio::placement::Projection> {
        sound::screen(
            self.lot.world(),
            &self.lot_shape,
            &self.view.camera,
            self.view.size,
        )
    }

    /// Lightweight O(1) dirty-tracking key for [`sound::Feed::update`].
    pub(crate) fn sound_world_key(&self) -> sound::SoundWorldKey {
        let world = self.lot.world();
        sound::SoundWorldKey {
            ticks: self.lot.ticks(),
            restore_gen: self.restore_generation,
            sound_rev: self.world_sound_rev,
            error_count: self.errors.lines.len(),
            object_count: world.objects.len(),
            hour: world.globals.first().copied().unwrap_or(0),
            lot_id: world.globals.get(10).copied().unwrap_or(0),
            buy_held_pos: self
                .buy_cursor
                .as_ref()
                .and_then(|c| world.objects.get(&c.held).map(|o| (c.held, o.position))),
        }
    }
}
