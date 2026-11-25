//! Window size, the shell's tick, wake-up times and rendering.

use super::*;

impl NeighborhoodShell {
    pub fn resize(&mut self, size: [i32; 2]) -> bool {
        let size = [size[0].max(1), size[1].max(1)];
        let changed = self.size != size;
        self.size = size;
        if changed {
            // The shell's notice (also the one over the Photo Album on a
            // lot) is centred on the window.
            self.relayout_dialog();
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            return session.game.resize(size) || changed;
        }
        if changed && matches!(self.screen, ActiveScreen::Neighborhood) {
            if let Some(point) = self.pointer {
                let _ = self.hover(point);
            }
            self.refresh_hovered_lot();
        }
        changed
    }

    /// Advances shell-level animation timers (e.g. loading captions, neighborhood
    /// water waves, Photo Album caret blink, CAF turntable & caret blink) and
    /// polls background save completions. Returns `true` when the active screen
    /// should be redrawn.
    pub fn tick_shell(&mut self, delta_us: u64) -> bool {
        self.clock_us = self.clock_us.saturating_add(delta_us);
        let mut redraw = self.poll_background_saves();
        if self.album_dialog.is_open() && self.album_dialog.sync_blink(delta_us) {
            redraw = true;
        }
        if self.coop_lobby.is_open() && self.coop_lobby.sync_blink(self.clock_us) {
            redraw = true;
        }
        if let Some(dlg) = &mut self.dialog {
            if dlg.message.entry.is_some() && dlg.state.sync_blink(self.clock_us) {
                redraw = true;
            }
            let now_ms = self.now_ms() as u32;
            let anim = self.clock_us > 0 && self.dialog_fade.is_animating(now_ms);
            if anim || self.dialog_fade_was_animating {
                redraw = true;
            }
            self.dialog_fade_was_animating = anim;
        } else {
            self.dialog_fade_was_animating = false;
        }
        match &mut self.screen {
            ActiveScreen::Loading(_) => {
                let ms = (delta_us / 1_000).max(u64::from(delta_us > 0));
                self.loading_screen.tick(ms);
                redraw = true;
            }
            ActiveScreen::Neighborhood => {
                let now_ms = self.clock_us / 1_000;
                let fade_anim_now = self.hover_fade.is_animating(now_ms);
                if fade_anim_now || self.hover_fade_was_animating {
                    redraw = true;
                }
                self.hover_fade_was_animating = fade_anim_now;
                self.hover_fade.prune(now_ms);

                let next_wave = (self.clock_us / WATER_STEP_US) as usize;
                if next_wave != self.water_frame {
                    self.water_frame = next_wave;
                    let has_waves = self.map.waves.iter().any(Option::is_some);
                    let has_filter_anim = destination_for_kind(self.kind).is_some()
                        && self.travel_state.active_filter.is_some();
                    redraw |= has_waves || has_filter_anim;
                }
                let modal_open = self.dialog.is_some()
                    || self.save_manager.is_open()
                    || self.album_dialog.is_open()
                    || self.travel_dialog_state.is_some()
                    || self.coop_lobby.is_open()
                    || self.import_picker.is_open()
                    || self.credits_dialog.is_open();
                if !modal_open
                    && let (Some(lot_id), Some(since_us)) =
                        (self.hovered_lot_id, self.hovered_lot_since_us)
                    && self.hover_card_shown_for != Some(lot_id)
                    && self.clock_us.saturating_sub(since_us) >= HOVER_CARD_DELAY_US
                {
                    self.hover_card_shown_for = Some(lot_id);
                    self.hover_card_cursor = self.pointer;
                    self.nbhd_state.hovered_lot = self.build_lot_info(lot_id);
                    if self.nbhd_state.hovered_lot.is_some() {
                        redraw = true;
                    }
                }
            }
            ActiveScreen::CreateFamily(caf) => {
                redraw |= caf.tick(delta_us);
                self.drain_caf_voice_previews();
            }
            _ => {}
        }
        redraw
    }

    /// Ensures immutable install-wide assets are loaded once and cached on this
    /// shell for fast cross-lot transitions.
    pub(super) fn ensure_install_assets(&mut self) -> Result<Arc<InstallAssets>, String> {
        if let Some(cached) = &self.install_assets {
            return Ok(Arc::clone(cached));
        }
        let loaded = Arc::new(InstallAssets::load(
            &self.install,
            Arc::clone(&self.pool),
            self.base_index.clone(),
            self.base_catalog.clone(),
        )?);
        self.base_index = Some(Arc::clone(&loaded.base_index));
        self.base_catalog = Some(Arc::clone(&loaded.base_catalog));
        self.install_assets = Some(Arc::clone(&loaded));
        Ok(loaded)
    }

    /// Exact microseconds until this shell next needs an event-loop wakeup, or
    /// `None` when completely idle (allowing `ControlFlow::Wait`).
    pub fn next_wake_us(&self, dragging: bool, anim_interval_us: u64) -> Option<u64> {
        self.next_wake_us_with_window(dragging, anim_interval_us, true, false)
    }

    /// Like [`NeighborhoodShell::next_wake_us`], gating pure presentation
    /// wakeups on `can_draw` and letting the redraw pacer own already-dirty
    /// frame deadlines so a zero-sized window or deferred redraw never spins.
    pub fn next_wake_us_with_window(
        &self,
        dragging: bool,
        anim_interval_us: u64,
        can_draw: bool,
        frame_dirty: bool,
    ) -> Option<u64> {
        let mut best: Option<u64> = None;
        let mut consider = |us: u64| {
            best = Some(best.map_or(us, |prev| prev.min(us)));
        };
        if self.autosave_worker.has_pending_or_unread() {
            consider(25_000);
        }
        if can_draw {
            if let Some(us) = self.album_dialog.next_blink_us() {
                consider(us);
            }
            if let Some(us) = self.coop_lobby.next_blink_us(self.clock_us) {
                consider(us);
            }
            if let Some(dialog) = &self.dialog {
                let now_ms = self.now_ms() as u32;
                if (self.clock_us > 0 && self.dialog_fade.is_animating(now_ms))
                    || self.dialog_fade_was_animating
                {
                    consider(anim_interval_us.max(1_000));
                }
                if dialog.message.entry.is_some()
                    && let Some(us) = dialog.state.next_blink_us(self.clock_us)
                {
                    consider(us);
                }
            }
        }
        let modal_open = self.dialog.is_some()
            || self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
            || self.coop_lobby.is_open()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open();
        match &self.screen {
            ActiveScreen::Loading(_) => {
                if can_draw {
                    consider(anim_interval_us.max(1_000));
                }
            }
            ActiveScreen::Neighborhood => {
                if can_draw {
                    let now_ms = self.clock_us / 1_000;
                    if self.hover_fade.is_animating(now_ms) || self.hover_fade_was_animating {
                        consider(anim_interval_us.max(1_000));
                    }
                    let has_waves = self.map.waves.iter().any(Option::is_some);
                    let has_filter_anim = destination_for_kind(self.kind).is_some()
                        && self.travel_state.active_filter.is_some();
                    if has_waves || has_filter_anim {
                        let rem = WATER_STEP_US - (self.clock_us % WATER_STEP_US);
                        consider(rem.max(1_000));
                    }
                    if !modal_open
                        && let (Some(lot_id), Some(since_us)) =
                            (self.hovered_lot_id, self.hovered_lot_since_us)
                        && self.hover_card_shown_for != Some(lot_id)
                    {
                        let elapsed = self.clock_us.saturating_sub(since_us);
                        let rem = HOVER_CARD_DELAY_US.saturating_sub(elapsed);
                        if rem > 0 {
                            consider(rem);
                        } else if !frame_dirty {
                            consider(anim_interval_us.max(1_000));
                        }
                    }
                }
            }
            ActiveScreen::FamilyBin => {}
            ActiveScreen::CreateFamily(caf) => {
                if can_draw {
                    consider(caf.next_wake_us());
                }
            }
            ActiveScreen::Lot(session) => {
                let shell_modal_open = self.save_manager.is_open()
                    || self.album_dialog.is_open()
                    || self.travel_dialog_state.is_some();
                if shell_modal_open {
                    if session.game.coop_active() || session.game.pending_coop_restage().is_some() {
                        consider(5_000);
                    }
                } else if let Some(us) = session.game.next_wake_us_with_window(
                    dragging,
                    anim_interval_us,
                    can_draw,
                    frame_dirty,
                ) {
                    consider(us);
                }
            }
        }
        best
    }

    pub fn canvas(&self) -> &Canvas {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
        {
            return &self.canvas;
        }
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.canvas(),
            _ => &self.canvas,
        }
    }

    /// Canvas-space dirty rects accumulated since the last successful window
    /// present, or `None` when a full blit/present is required.
    pub fn peek_present_damage(&self) -> Option<&[sims_render::canvas::Rect]> {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
        {
            return None;
        }
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.peek_present_damage(),
            ActiveScreen::Neighborhood => self.present_damage.as_deref(),
            _ => None,
        }
    }

    /// Marks the active screen's current canvas as successfully presented to
    /// the window backbuffer.
    pub fn on_presented(&mut self) {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
        {
            self.present_damage = None;
            if let ActiveScreen::Lot(session) = &mut self.screen {
                session.game.invalidate_present_damage();
            }
            return;
        }
        match &mut self.screen {
            ActiveScreen::Lot(session) => {
                session.game.on_presented();
                self.present_damage = None;
            }
            ActiveScreen::Neighborhood => {
                self.present_damage = Some(Vec::new());
            }
            _ => {
                self.present_damage = None;
            }
        }
    }

    pub fn render(&mut self) {
        if let ActiveScreen::Lot(session) = &mut self.screen {
            self.prev_nb_steady_key = None;
            self.prev_nb_dynamic_rects.clear();
            self.present_damage = None;
            session.game.render();
            let has_overlay = self.save_manager.is_open()
                || self.album_dialog.is_open()
                || self.travel_dialog_state.is_some();
            let album_open = self.album_dialog.is_open();
            if has_overlay {
                session.game.invalidate_present_damage();
                let lot_canvas = session.game.canvas();
                let w = lot_canvas.width;
                let h = lot_canvas.height;
                if let Ok(mut frame) =
                    Frame::from_packed(w as u32, h as u32, lot_canvas.color.clone())
                {
                    if self.album_dialog.is_open()
                        && let Some(snapshot) = &self.album_snapshot
                    {
                        self.album_dialog
                            .draw(&mut frame, snapshot, &self.album_art);
                    }
                    if let Some(td_state) = &self.travel_dialog_state {
                        self.travel_dialog.draw(
                            &mut frame,
                            td_state,
                            &self.travel_art,
                            &self.travel_strings,
                        );
                    }
                    if self.save_manager.is_open()
                        && let Some(snapshot) = &self.save_manager_snapshot
                    {
                        self.save_manager
                            .draw(&mut frame, snapshot, &self.save_manager_art);
                    }
                    if album_open {
                        self.draw_shell_dialog(&mut frame);
                    }
                    let mut composed = Canvas::new(w, h, [0, 0, 0]);
                    composed.color = frame.into_packed();
                    self.canvas = composed;
                }
            }
            return;
        }
        let w = self.size[0].max(1) as usize;
        let h = self.size[1].max(1) as usize;
        if self.canvas.width != w || self.canvas.height != h {
            self.canvas = Canvas::new(w, h, [0, 0, 0]);
        }

        match &mut self.screen {
            ActiveScreen::Loading(state) => {
                self.prev_nb_steady_key = None;
                self.prev_nb_dynamic_rects.clear();
                self.present_damage = None;
                let packed = std::mem::take(&mut self.canvas.color);
                if let Ok(mut frame) = Frame::from_packed(w as u32, h as u32, packed) {
                    self.loading_screen.draw(
                        &mut frame,
                        state,
                        &self.loading_art,
                        &self.loading_strings,
                    );
                    self.canvas.color = frame.into_packed();
                } else {
                    self.canvas.color = vec![0u32; w * h];
                }
            }
            ActiveScreen::Neighborhood => {
                let is_travel_map = destination_for_kind(self.kind).is_some();
                if is_travel_map {
                    self.sync_travel_state();
                }
                let highlighted_lots = if is_travel_map {
                    self.travel_filtered_lot_ids()
                } else {
                    BTreeSet::new()
                };
                let active_filter = if is_travel_map {
                    self.travel_state
                        .active_filter
                        .map(community_filter_for_lot_filter)
                } else {
                    None
                };
                let mut opts = RenderOptions {
                    viewport: [w as u32, h as u32],
                    hovered_lot: self.hovered_lot_id,
                    hover_roofs_off: true,
                    highlight_hovered: self.should_highlight_hovered_lot(),
                    highlighted_lots,
                    water_frame: Some(self.water_frame),
                    active_filter,
                    filter_frame: self.water_frame,
                    show_occupancy_markers: matches!(
                        self.nbhd_state.mode,
                        NeighborhoodMode::MoveIn { .. }
                    ),
                    ..RenderOptions::default()
                };
                if self.clock_us > 0 {
                    let now_ms = self.clock_us / 1_000;
                    self.hover_fade.prune(now_ms);
                    self.hover_fade.apply_to(&mut opts, now_ms);
                    // Preserve a zero-clock hover as already settled at 1.0 if
                    // the clock starts advancing before the pointer moves.
                    if let Some(lot) = self.hovered_lot_id
                        && self.hover_fade.hovered() != Some(lot)
                    {
                        opts.hovered_lot = Some(lot);
                        opts.hover_roof_fade = 1.0;
                        opts.lot_roof_fades.insert(lot, 1.0);
                    }
                }
                let map_damage = self.map.render_into_with_damage(&mut self.canvas, &opts);
                let packed = std::mem::take(&mut self.canvas.color);
                if let Ok(mut frame) = Frame::from_packed(w as u32, h as u32, packed) {
                    if is_travel_map {
                        self.travel_chrome.draw(
                            &mut frame,
                            &self.travel_state,
                            &self.travel_art,
                            &self.travel_strings,
                        );
                    } else {
                        // The card fades in with the lot's roof (`0x54f939`).
                        if let Some(info) = &mut self.nbhd_state.hovered_lot {
                            let fade = if self.clock_us > 0
                                && self.hover_fade.hovered() == Some(info.lot_number)
                            {
                                self.hover_fade
                                    .fade_for(info.lot_number, self.clock_us / 1_000)
                            } else {
                                1.0
                            };
                            info.alpha_shift = Some(sims_ui::neighborhood::popup_alpha_shift(fade));
                        }
                        self.chrome.draw(
                            &mut frame,
                            &self.nbhd_state,
                            &self.chrome_art,
                            &self.chrome_strings,
                        );
                    }
                    self.draw_overlay_note_and_dialog(&mut frame);
                    if self.album_dialog.is_open()
                        && let Some(snapshot) = &self.album_snapshot
                    {
                        self.album_dialog
                            .draw(&mut frame, snapshot, &self.album_art);
                        self.draw_shell_dialog(&mut frame);
                    }
                    if let Some(td_state) = &self.travel_dialog_state {
                        self.travel_dialog.draw(
                            &mut frame,
                            td_state,
                            &self.travel_art,
                            &self.travel_strings,
                        );
                    }
                    if self.save_manager.is_open()
                        && let Some(snapshot) = &self.save_manager_snapshot
                    {
                        self.save_manager
                            .draw(&mut frame, snapshot, &self.save_manager_art);
                    }
                    if self.import_picker.is_open()
                        && let Some(snapshot) = &self.import_picker_snapshot
                    {
                        self.import_picker.draw(
                            &mut frame,
                            snapshot,
                            &self.import_picker_strings,
                            &self.save_manager_art,
                        );
                    }
                    if self.credits_dialog.is_open()
                        && let Some(snapshot) = &self.credits_snapshot
                    {
                        self.credits_dialog.draw(
                            &mut frame,
                            snapshot,
                            &self.book_strings,
                            &self.save_manager_art,
                        );
                    }
                    self.canvas.color = frame.into_packed();
                } else {
                    self.canvas.color = vec![0u32; w * h];
                }

                let steady_eligible = map_damage.is_some()
                    && !self.debug
                    && self.dialog.is_none()
                    && !self.album_dialog.is_open()
                    && self.travel_dialog_state.is_none()
                    && !self.save_manager.is_open()
                    && !self.coop_lobby.is_open()
                    && !self.import_picker.is_open()
                    && !self.credits_dialog.is_open()
                    && opts.active_filter.is_none()
                    && !opts.show_occupancy_markers;
                let cur_key = steady_eligible.then(|| NeighborhoodSteadyKey {
                    size: self.size,
                    slot: self.slot,
                    kind: self.kind,
                    water_frame: self.water_frame,
                    pointer: self.pointer,
                    nbhd_state: self.nbhd_state.clone(),
                    travel_state: self.travel_state.clone(),
                    chrome_hovered: self.chrome.hovered(),
                    chrome_pressed: self.chrome.pressed(),
                    travel_hovered: self.travel_chrome.hovered(),
                    travel_pressed: self.travel_chrome.pressed(),
                    pressed_lot_id: self.pressed_lot_id,
                });
                let cur_rects = map_damage.unwrap_or_default();
                if cur_key.is_some() && self.prev_nb_steady_key == cur_key {
                    if let Some(dmg) = &mut self.present_damage {
                        dmg.extend_from_slice(&self.prev_nb_dynamic_rects);
                        dmg.extend_from_slice(&cur_rects);
                        if dmg.len() > 32 {
                            self.present_damage = None;
                        }
                    }
                } else {
                    self.present_damage = None;
                }
                self.prev_nb_dynamic_rects = cur_rects;
                self.prev_nb_steady_key = cur_key;
            }
            ActiveScreen::FamilyBin => {
                self.prev_nb_steady_key = None;
                self.prev_nb_dynamic_rects.clear();
                self.present_damage = None;
                let packed = std::mem::take(&mut self.canvas.color);
                if let Ok(mut frame) = Frame::from_packed(w as u32, h as u32, packed) {
                    let view = self.family_bin_view();
                    self.family_bin.draw(
                        &mut frame,
                        &view,
                        &self.family_bin_art,
                        &self.dialogs.family_bin,
                    );
                    self.draw_overlay_note_and_dialog(&mut frame);
                    self.canvas.color = frame.into_packed();
                } else {
                    self.canvas.color = vec![0u32; w * h];
                }
            }
            ActiveScreen::CreateFamily(caf) => {
                self.prev_nb_steady_key = None;
                self.prev_nb_dynamic_rects.clear();
                self.present_damage = None;
                let packed = std::mem::take(&mut self.canvas.color);
                if let Ok(mut frame) = Frame::from_packed(w as u32, h as u32, packed) {
                    caf.draw(&mut frame);
                    if self.debug
                        && let Some(note) = caf.note().or(self.note.as_deref())
                        && let Some(font) = self
                            .chrome_art
                            .font(12)
                            .or_else(|| self.chrome_art.font(10))
                    {
                        let bar_h = font.line_height + 6;
                        frame.fill(Rect::new(0, 0, frame.width() as i32, bar_h), [96, 24, 24]);
                        font.draw(&mut frame, note, [8, 3], [255, 255, 255]);
                    }
                    self.canvas.color = frame.into_packed();
                } else {
                    self.canvas.color = vec![0u32; w * h];
                }
            }
            ActiveScreen::Lot(_) => unreachable!(),
        }
    }

    pub(super) fn draw_overlay_note_and_dialog(&mut self, frame: &mut Frame) {
        if self.debug
            && let Some(note) = &self.note
            && let Some(font) = self
                .chrome_art
                .font(12)
                .or_else(|| self.chrome_art.font(10))
        {
            let y = sims_ui::neighborhood::BAR_HEIGHT + 4;
            let bar_h = font.line_height + 6;
            let w = frame.width() as i32;
            frame.fill(Rect::new(0, y, w, bar_h), [16, 24, 68]);
            font.draw(frame, note, [8, y + 3], [240, 245, 255]);
        }
        // A notice over the open album is drawn after the album.
        if !self.album_dialog.is_open() {
            self.draw_shell_dialog(frame);
        }
        if self.coop_lobby.is_open() {
            let snap = self.shell_coop_snapshot();
            self.coop_lobby.draw(frame, &snap, &self.coop_art);
        }
    }

    pub(super) fn family_bin_view(&self) -> FamilyBinView<'_> {
        let mode = if self.kind == NeighborhoodKind::MagicTown {
            FamilyBinMode::MagicTown
        } else {
            FamilyBinMode::Standard
        };
        FamilyBinView {
            mode,
            families: &self.family_bin_entries,
        }
    }

    pub(super) fn frame_size_u32(&self) -> [u32; 2] {
        [self.size[0].max(1) as u32, self.size[1].max(1) as u32]
    }
}
