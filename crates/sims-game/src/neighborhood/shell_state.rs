//! The shell's state: screens, loading, notices, fades, dialogs and options.

use super::*;

impl NeighborhoodShell {
    pub fn screen(&self) -> ScreenKind {
        match self.screen {
            ActiveScreen::Loading(_) => ScreenKind::Loading,
            ActiveScreen::Neighborhood => ScreenKind::Neighborhood,
            ActiveScreen::FamilyBin => ScreenKind::FamilyBin,
            ActiveScreen::CreateFamily(_) => ScreenKind::CreateFamily,
            ActiveScreen::Lot(_) => ScreenKind::Lot,
        }
    }

    /// Monotonic counter incremented on every lot entry or Save Manager / F9
    /// restore so the window host resets error indices and lot-scoped audio.
    pub fn lot_generation(&self) -> u64 {
        self.lot_generation
    }

    pub fn migration_report(&self) -> Option<&StoreMigrationReport> {
        self.migration_report.as_ref()
    }

    pub fn migration_error(&self) -> Option<&str> {
        self.migration_error.as_deref()
    }

    pub fn autosave_worker(&self) -> &AutosaveWorker {
        &self.autosave_worker
    }

    pub fn pending_background_saves(&self) -> usize {
        self.autosave_worker.pending_count()
    }

    pub fn save_manager_open(&self) -> bool {
        self.save_manager.is_open()
    }

    pub fn save_manager(&self) -> &SaveManagerDialog {
        &self.save_manager
    }

    pub fn save_manager_snapshot(&self) -> Option<&SaveManagerSnapshot> {
        self.save_manager_snapshot.as_ref()
    }

    pub fn loading_screen(&self) -> &LoadingScreen {
        &self.loading_screen
    }

    pub fn loading_strings(&self) -> &LoadingStrings {
        &self.loading_strings
    }

    pub fn loading_state(&self) -> Option<&LoadingState> {
        match &self.screen {
            ActiveScreen::Loading(state) => Some(state),
            _ => None,
        }
    }

    /// Enters the interactive native [`LoadingScreen`] (`ScreenKind::Loading`),
    /// emitting [`Input::MusicMode`]`(`[`MusicMode::Load`]`)` when
    /// [`LoadingAudioCue::StartLoadLoop`] fires.
    pub fn begin_loading(
        &mut self,
        kind: LoadingKind,
        destination_label: Option<String>,
        progress: f32,
    ) {
        self.start_load_audio();
        self.screen = ActiveScreen::Loading(LoadingState {
            kind,
            progress,
            destination_label,
            caption_override: None,
        });
    }

    /// Updates the progress ratio and optional caption override while on
    /// [`ScreenKind::Loading`].
    pub fn update_loading(&mut self, progress: f32, caption_override: Option<String>) -> bool {
        if let ActiveScreen::Loading(state) = &mut self.screen {
            state.progress = progress;
            state.caption_override = caption_override;
            true
        } else {
            false
        }
    }

    /// Advances the active loading screen's rotating caption timer by `delta_ms`.
    pub fn tick_loading(&mut self, delta_ms: u64) {
        self.loading_screen.tick(delta_ms);
    }

    /// Finishes the active [`LoadingScreen`], emitting
    /// [`LoadingAudioCue::StopLoadLoop`] -> destination music mode and returning
    /// to the Neighborhood screen if still on [`ScreenKind::Loading`].
    pub fn finish_loading(&mut self) {
        if matches!(self.screen, ActiveScreen::Loading(_)) {
            self.screen = ActiveScreen::Neighborhood;
        }
        let target_music = self.desired_music_mode();
        self.stop_load_audio(target_music);
    }

    /// Renders a standalone [`LoadingScreen`] frame at `size` using the shell's
    /// loaded splash artwork and `UIText.iff` caption strings.
    pub fn render_loading_frame(&self, state: &LoadingState, size: [u32; 2]) -> Canvas {
        let w = size[0].max(1);
        let h = size[1].max(1);
        let mut frame = Frame::new(w, h, [0, 0, 0]);
        self.loading_screen
            .draw(&mut frame, state, &self.loading_art, &self.loading_strings);
        let mut canvas = Canvas::new(w as usize, h as usize, [0, 0, 0]);
        canvas.color = frame.into_packed();
        canvas
    }

    /// Installs a desktop host presenter invoked immediately before blocking
    /// lot/travel transitions so the native [`LoadingScreen`] splash, progress
    /// bar, rotating caption, and [`MusicMode::Load`] are presented live while
    /// work executes.
    pub fn set_loading_presenter(&mut self, presenter: Option<LoadingPresenter>) {
        self.loading_presenter = presenter;
    }

    /// Enables or disables `--debug` mode (top diagnostic/status banners and
    /// diagnostic stderr logging), propagating to any active lot [`Game`].
    pub fn set_debug(&mut self, debug: bool) {
        self.debug = debug;
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.set_debug(debug);
        }
    }

    /// Whether `--debug` diagnostic banner & stderr logging mode is active.
    pub fn debug(&self) -> bool {
        self.debug
    }

    /// Sets a programmatic/diagnostic status note on the active Lot or
    /// Neighborhood screen. Shown visually in the top strip only when `--debug`
    /// is enabled; always readable via [`Self::note`] / [`Self::status_note`].
    pub fn set_note(&mut self, note: impl Into<String>) {
        let text = note.into();
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.set_note(text);
        } else {
            if self.debug {
                eprintln!("sims-game [debug]: {text}");
            }
            self.note = Some(text);
        }
    }

    /// Surfaces a genuine user-facing error or notice via a native Sims 1 modal
    /// message dialog (`cTSWinMsgBox` / `ShellDialogKind::Notice`), while also
    /// recording `body` in the programmatic status note getter.
    pub fn show_notice(&mut self, title: impl Into<String>, body: impl Into<String>) {
        let title = title.into();
        let body = body.into();
        // Over the open Photo Album the notice is the shell's own dialog,
        // drawn and answered above the album ([`Self::album_notice_open`]).
        if let ActiveScreen::Lot(session) = &mut self.screen
            && !self.album_dialog.is_open()
        {
            session.game.show_notice(title, body);
        } else {
            if self.debug {
                eprintln!("sims-game [debug]: {body}");
            }
            self.note = Some(body.clone());
            self.open_dialog(
                ShellDialogKind::Notice,
                Message::notice(title, body, self.dialogs.ok()).with_frame(FrameStyle::Generic),
            );
        }
    }

    /// Whether a shell notice is open over the Photo Album: it then takes
    /// the pointer and keys before the album.
    pub(super) fn album_notice_open(&self) -> bool {
        self.album_dialog.is_open() && self.dialog.is_some()
    }

    /// Presses the shell dialog's buttons.
    pub(super) fn press_shell_dialog(&mut self, point: [i32; 2]) -> ShellOutcome {
        if let Some(dlg) = &mut self.dialog
            && let Some(layout) = &dlg.layout
            && let Some(Command::AnswerDialog { button, .. }) =
                dialog::press_with_state(layout, point, &dlg.state)
        {
            dlg.state.pressed_button = Some(usize::from(button));
            return ShellOutcome::redraw();
        }
        ShellOutcome::default()
    }

    /// Releases over the shell dialog, answering it on a full click.
    pub(super) fn release_shell_dialog(&mut self, point: [i32; 2]) -> ShellOutcome {
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
        ShellOutcome::default()
    }

    /// Draws the shell dialog (fading in while the clock runs).
    pub(super) fn draw_shell_dialog(&mut self, frame: &mut Frame) {
        if let Some(dlg) = &mut self.dialog {
            if dlg.layout.is_none() {
                dlg.layout = dialog::layout(self.size, &dlg.message, &self.dialog_art);
            }
            if let Some(layout) = &dlg.layout {
                let alpha = if self.clock_us == 0 {
                    255
                } else {
                    self.dialog_fade.alpha((self.clock_us / 1_000) as u32)
                };
                dialog::draw_with_state_alpha(
                    frame,
                    &self.dialog_art,
                    &dlg.message,
                    layout,
                    self.pointer,
                    &dlg.state,
                    alpha,
                );
            }
        }
    }

    /// Returns the active status note on the current Lot or Neighborhood screen, if any.
    pub fn note(&self) -> Option<&str> {
        if let ActiveScreen::Lot(session) = &self.screen {
            session.game.note()
        } else {
            self.note.as_deref()
        }
    }

    pub(super) fn present_loading_step(
        &mut self,
        kind: LoadingKind,
        destination_label: Option<String>,
        progress: f32,
    ) {
        if self.loading_presenter.is_none() {
            return;
        }
        let state = LoadingState {
            kind,
            progress: progress.clamp(0.0, 1.0),
            destination_label,
            caption_override: None,
        };
        let frame_size = self.frame_size_u32();
        let canvas = self.render_loading_frame(&state, frame_size);
        let audio = std::mem::take(&mut self.audio_queue);
        if let Some(mut presenter) = self.loading_presenter.take() {
            presenter(&canvas.color, self.size, audio);
            self.loading_presenter = Some(presenter);
        }
    }

    pub(super) fn start_load_audio(&mut self) {
        let seq = self.loading_caption_seq;
        self.loading_caption_seq = self.loading_caption_seq.wrapping_add(1);
        if let Some(LoadingAudioCue::StartLoadLoop { .. }) = self.loading_screen.begin(seq) {
            self.audio_queue.push(Input::MusicMode(MusicMode::Load));
        }
    }

    pub(super) fn stop_load_audio(&mut self, next_music: MusicMode) {
        if let Some(LoadingAudioCue::StopLoadLoop) = self.loading_screen.finish() {
            self.audio_queue.push(Input::MusicMode(next_music));
        }
    }

    pub fn album_open(&self) -> bool {
        self.album_dialog.is_open()
    }

    pub fn album_dialog(&self) -> &PhotoAlbumDialog {
        &self.album_dialog
    }

    pub fn album_snapshot(&self) -> Option<&AlbumSnapshot> {
        self.album_snapshot.as_ref()
    }

    pub fn travel_state(&self) -> &TravelState {
        &self.travel_state
    }

    pub fn travel_dialog_open(&self) -> bool {
        self.travel_dialog_state.is_some()
    }

    pub fn travel_dialog_state(&self) -> Option<&TravelDialogState> {
        self.travel_dialog_state.as_ref()
    }

    /// Active travel session when the household is currently visiting a
    /// destination lot (`depart_to_destination` / `resume_active_trip`).
    pub fn active_trip(&self) -> Option<&TravelSession> {
        match &self.screen {
            ActiveScreen::Lot(session) => session.active_trip.as_ref(),
            _ => None,
        }
    }

    /// Whether the active lot is an unoccupied destination lot opened for
    /// Build/Buy editing (`open_destination_for_edit`).
    pub fn is_destination_edit(&self) -> bool {
        match &self.screen {
            ActiveScreen::Lot(session) => session.is_destination_edit,
            _ => false,
        }
    }

    pub fn caf_session(&self) -> Option<&CafSession> {
        match &self.screen {
            ActiveScreen::CreateFamily(caf) => Some(caf),
            _ => None,
        }
    }

    pub fn caf_session_mut(&mut self) -> Option<&mut CafSession> {
        match &mut self.screen {
            ActiveScreen::CreateFamily(caf) => Some(caf),
            _ => None,
        }
    }

    pub fn neighborhood_slot(&self) -> u8 {
        self.slot
    }

    pub fn neighborhood_id(&self) -> &str {
        &self.neighborhood_id
    }

    pub fn neighborhood_kind(&self) -> NeighborhoodKind {
        self.kind
    }

    pub fn neighborhood_state(&self) -> &NeighborhoodState {
        &self.nbhd_state
    }

    pub fn map(&self) -> &NeighborhoodMap {
        &self.map
    }

    pub fn store(&self) -> &NeighborhoodStore {
        &self.store
    }

    pub fn stored_neighborhood(&self) -> &StoredNeighborhood {
        &self.stored
    }

    pub fn family_bin(&self) -> &FamilyBin {
        &self.family_bin
    }

    pub fn family_bin_entries(&self) -> &[FamilyEntry] {
        &self.family_bin_entries
    }

    pub fn game(&self) -> Option<&Game> {
        match &self.screen {
            ActiveScreen::Lot(session) => Some(&session.game),
            _ => None,
        }
    }

    pub fn game_mut(&mut self) -> Option<&mut Game> {
        match &mut self.screen {
            ActiveScreen::Lot(session) => Some(&mut session.game),
            _ => None,
        }
    }

    pub fn active_lot_house(&self) -> Option<u32> {
        match &self.screen {
            ActiveScreen::Lot(session) => Some(session.house),
            _ => None,
        }
    }

    pub fn hovered_lot_id(&self) -> Option<u16> {
        self.hovered_lot_id
    }

    pub(super) fn now_ms(&self) -> u64 {
        self.clock_us / 1_000
    }

    pub(super) fn update_hover_fade(&mut self, prev_lot: Option<u16>, hovered: Option<u16>) {
        if self.clock_us > 0 {
            let now_ms = self.clock_us / 1_000;
            // If `prev_lot` was hovered at `clock_us == 0` (rendered settled at
            // 1.0 without starting a hidden ramp), seed it as settled before
            // transitioning so moving away fades its roof back in smoothly.
            if self.hover_fade.hovered().is_none()
                && let Some(prev) = prev_lot
                && prev_lot != hovered
            {
                let seed_ms =
                    now_ms.saturating_sub(sims_render::neighborhood::HOVER_FADE_DURATION_MS);
                self.hover_fade.set_hovered(Some(prev), seed_ms);
            }
            self.hover_fade.set_hovered(hovered, now_ms);
            self.hover_fade_was_animating = self.hover_fade.is_animating(now_ms);
        } else {
            self.hover_fade.reset();
            self.hover_fade_was_animating = false;
        }
    }

    /// Whether any Neighborhood lot roof cross-fade (300 ms `cWinLot` ramp) is
    /// actively transitioning at the current presentation clock.
    pub fn has_active_fades(&self) -> bool {
        self.clock_us > 0
            && matches!(self.screen, ActiveScreen::Neighborhood)
            && self.hover_fade.is_animating(self.now_ms())
    }

    /// Current Neighborhood hover roof cross-fade tracker (`HoverFadeState`).
    pub fn hover_fade_state(&self) -> &HoverFadeState {
        &self.hover_fade
    }

    /// Current shell modal dialog fade state (`DialogFadeState`).
    pub fn dialog_fade_state(&self) -> &DialogFadeState {
        &self.dialog_fade
    }

    /// Current water ripple animation frame index on the Neighborhood map.
    pub fn water_frame(&self) -> usize {
        self.water_frame
    }

    /// Whether hovering a lot on the current map should draw the synthetic
    /// yellow highlight outline (`RenderOptions::highlight_hovered`).
    ///
    /// Disabled during plain idle residential/destination browsing (matching
    /// Complete Collection 1.91, which lifts the roof on hover without a yellow
    /// outline), and enabled only during Move-In, Bulldoze, or active Community
    /// Filter modes.
    pub fn should_highlight_hovered_lot(&self) -> bool {
        let in_action_mode = matches!(
            self.nbhd_state.mode,
            NeighborhoodMode::MoveIn { .. } | NeighborhoodMode::Bulldoze | NeighborhoodMode::Rezone
        ) || self.travel_state.mode == TravelBarMode::Bulldoze;
        let has_active_filter =
            destination_for_kind(self.kind).is_some() && self.travel_state.active_filter.is_some();
        in_action_mode || has_active_filter
    }

    pub fn has_dialog(&self) -> bool {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
            || self.coop_lobby.is_open()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open()
        {
            return true;
        }
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.has_dialog() || session.game.shop_open(),
            ActiveScreen::CreateFamily(caf) => caf.controller().confirm().is_some(),
            _ => self.dialog.is_some(),
        }
    }

    /// Whether the Neighborhood Family Import picker modal is currently open.
    pub fn import_picker_open(&self) -> bool {
        self.import_picker.is_open()
    }

    /// Active snapshot for the Family Import picker modal, if open.
    pub fn import_picker_snapshot(&self) -> Option<&ShopSnapshot> {
        self.import_picker_snapshot.as_ref()
    }

    /// Whether any importable `.FAM` household was found for the top-bar Import button.
    pub fn has_importable_family(&self) -> bool {
        self.nbhd_state.has_importable_family
    }

    /// Whether the Neighborhood Credits/About reference modal is currently open.
    pub fn credits_open(&self) -> bool {
        self.credits_dialog.is_open()
    }

    /// Active [`ReferenceBookDialog`] for the Credits/About modal.
    pub fn credits_dialog(&self) -> &ReferenceBookDialog {
        &self.credits_dialog
    }

    /// Active [`ReferenceBookSnapshot`] for the Credits/About modal, if open.
    pub fn credits_snapshot(&self) -> Option<&ReferenceBookSnapshot> {
        self.credits_snapshot.as_ref()
    }

    /// Whether any paginated reference book modal (Neighborhood Credits or
    /// in-lot Spellbook / Cookbook) is currently open.
    pub fn book_open(&self) -> bool {
        self.credits_dialog.is_open()
            || matches!(&self.screen, ActiveScreen::Lot(s) if s.game.book_open())
    }

    /// Whether the active lot is presenting a graphical destination map picker
    /// (`Dialog` kinds `5, 7, 8, 11, 13`).
    pub fn destination_picker_open(&self) -> bool {
        matches!(&self.screen, ActiveScreen::Lot(s) if s.game.destination_picker_open())
    }

    /// The dialog in front: on a lot, the lot's, unless the shell's notice
    /// is over the Photo Album ([`Self::album_notice_open`]).
    pub fn dialog_layout(&self) -> Option<&dialog::Layout> {
        match &self.screen {
            ActiveScreen::Lot(session) if !self.album_notice_open() => session.game.dialog(),
            _ => self.dialog.as_ref().and_then(|d| d.layout.as_ref()),
        }
    }

    pub fn status_note(&self) -> Option<&str> {
        match &self.screen {
            ActiveScreen::Lot(session) => session.game.note().or(self.note.as_deref()),
            ActiveScreen::CreateFamily(caf) => caf.note().or(self.note.as_deref()),
            _ => self.note.as_deref(),
        }
    }

    pub fn volumes(&self) -> [u8; 3] {
        if let ActiveScreen::Lot(session) = &self.screen {
            session.game.volumes()
        } else {
            [
                self.options_state.sound.fx,
                self.options_state.sound.music,
                self.options_state.sound.vox,
            ]
        }
    }

    pub fn desired_music_mode(&self) -> MusicMode {
        match &self.screen {
            ActiveScreen::Loading(_) => MusicMode::Load,
            ActiveScreen::Neighborhood => music_mode_for_kind(self.kind),
            ActiveScreen::FamilyBin | ActiveScreen::CreateFamily(_) => MusicMode::FamilyBin,
            ActiveScreen::Lot(session) => {
                sound::lot_music_mode(session.game.mode(), session.game.lot().lot_id())
            }
        }
    }

    /// Drains queued mixer inputs generated by screen transitions.
    pub fn take_audio_inputs(&mut self) -> Vec<Input> {
        std::mem::take(&mut self.audio_queue)
    }

    /// Whether the active screen has an interactive tool, drag, or dialog open
    /// that benefits from full monitor refresh rate.
    pub fn is_actively_interacting(&self) -> bool {
        if self.save_manager.is_open()
            || self.album_dialog.is_open()
            || self.travel_dialog_state.is_some()
            || self.coop_lobby.is_open()
            || self.import_picker.is_open()
            || self.credits_dialog.is_open()
        {
            return true;
        }
        match &self.screen {
            ActiveScreen::Loading(_) => true,
            ActiveScreen::Lot(session) => session.game.is_actively_interacting(),
            ActiveScreen::Neighborhood => {
                self.dialog.is_some()
                    || self.has_active_fades()
                    || (self.clock_us > 0 && self.dialog_fade.is_animating(self.now_ms() as u32))
                    || !matches!(self.nbhd_state.mode, NeighborhoodMode::Normal)
                    || self.pressed_lot_id.is_some()
            }
            ActiveScreen::FamilyBin => self.dialog.is_some(),
            ActiveScreen::CreateFamily(_) => true,
        }
    }

    /// Window title string appropriate for the current screen.
    pub fn window_title(&self) -> String {
        match &self.screen {
            ActiveScreen::Loading(state) => match &state.destination_label {
                Some(label) => format!("sims-game: Loading {label}..."),
                None => "sims-game: Loading...".to_owned(),
            },
            ActiveScreen::Neighborhood => {
                if let Some(dest) = destination_for_kind(self.kind) {
                    format!(
                        "sims-game: Neighborhood {} — {}",
                        self.slot,
                        dest.short_label()
                    )
                } else {
                    format!("sims-game: Neighborhood {}", self.slot)
                }
            }
            ActiveScreen::FamilyBin => {
                format!("sims-game: Neighborhood {} — Select a Family", self.slot)
            }
            ActiveScreen::CreateFamily(_) => {
                format!("sims-game: Neighborhood {} — Create a Family", self.slot)
            }
            ActiveScreen::Lot(session) => {
                let suffix = if session.game.stopped().is_some() {
                    ", stopped"
                } else if session.game.paused() {
                    ", paused"
                } else {
                    ""
                };
                format!("sims-game: tick {}{suffix}", session.game.lot().ticks())
            }
        }
    }

    /// Persists user Options (`Optn` 128) if modified in the live lot.
    pub fn persist_options(&mut self) {
        if let ActiveScreen::Lot(session) = &self.screen {
            let current = session.game.options().to_optn();
            if current != self.saved_optn {
                self.options_state = *session.game.options();
                match session.game.save_options(&self.options_file) {
                    Ok(()) => self.saved_optn = current,
                    Err(e) => eprintln!("sims-game: {}: {e}", self.options_file.display()),
                }
            }
        }
    }
}
