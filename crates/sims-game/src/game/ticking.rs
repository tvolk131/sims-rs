//! Errors, speed and pause, wake-up times, and running the lot's ticks.

use super::*;

impl Game {
    /// Why the lot stopped, if it has.
    pub fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// The errors play mode caught and recovered from, one line each, in
    /// order: the first `LOGGED` (256) different ones (an error again at a
    /// later tick is not listed again), then a line saying more were
    /// caught.
    pub fn errors(&self) -> &[String] {
        &self.errors.lines
    }

    /// The sound requests of the steps run since the last call, in
    /// order, for the sound system ([`crate::sound::event`]).
    pub fn take_sounds(&mut self) -> Vec<SoundEvent> {
        self.sounds.drain(..).collect()
    }

    /// The HUD highlight requests emitted by UI Effect (opcode 34, `0x44acff`)
    /// since the last call, in order.
    pub fn take_ui_effects(&mut self) -> Vec<UiEffectEvent> {
        self.ui_effects.drain(..).collect()
    }

    /// Control IDs currently highlighted by UI Effect mode 0 (`0x476485`).
    pub fn highlighted_buttons(&self) -> &BTreeSet<i16> {
        &self.highlighted_buttons
    }

    /// Person GUIDs whose portrait button is currently highlighted by UI Effect
    /// mode 1 (`0x47690d`).
    pub fn highlighted_portraits(&self) -> &BTreeSet<u32> {
        &self.highlighted_portraits
    }

    /// Neighbor GUIDs whose relationship control is currently highlighted by
    /// UI Effect mode 2 (`0x4769c3`).
    pub fn highlighted_relationships(&self) -> &BTreeSet<u32> {
        &self.highlighted_relationships
    }

    /// How many errors play mode caught.
    pub fn caught(&self) -> u64 {
        self.errors.count
    }

    /// Problems drawing the people, one line each.
    pub fn problems(&self) -> &[String] {
        &self.people.problems
    }

    /// Sets the game's speed, as the panel's speed buttons do in live
    /// mode (`0x53a62a`, `0x53a6ad`, `0x53a728`): a paused game runs
    /// again (the original applies it after the speed; both restart the
    /// same clocks, so the order does not matter), and the simulation's speed ([`Lot::set_speed`]) and the
    /// time between ticks with it are set, unless the speed is already
    /// the one asked for (`0x4ce588`), when the time owed carries on. A
    /// lot that cannot take it keeps its speed, and the note across the
    /// top says why.
    pub fn set_speed(&mut self, speed: Speed) {
        let level = pace::level(speed);
        if self.coop.is_active() {
            if self.coop.session().is_some_and(|s| s.time.explicit_pause)
                && let Err(e) = self
                    .coop
                    .submit_command(NetCommand::SetPaused { paused: false })
            {
                self.note = Some(format!("The pause was not changed: {e}"));
                return;
            }
            if let Err(e) = self.coop.submit_command(NetCommand::SetSpeed {
                speed: NetSimSpeed::new(level),
            }) {
                self.note = Some(format!("The speed was not changed: {e}"));
            } else if self.hud.controls.mode == sims_ui::command::Mode::Live {
                self.hud.set_idle_speed(false);
            }
            return;
        }
        let result = if self.paused() {
            self.lot.unpause().map(|_| self.pace.restart())
        } else {
            Ok(())
        };
        let result = result.and_then(|()| {
            if self.lot.world().speed() == level {
                self.pace.speed = speed;
                return Ok(());
            }
            self.lot
                .set_speed(i32::from(level))
                .map(|_| self.pace.set_speed(speed))
        });
        match result {
            Ok(()) => {
                // 0x53a5e3, 0x53a661..0x53a666, 0x53a6dc..0x53a6e1: manual
                // speed presses in Live mode clear `[control_panel + 0x1ec]`.
                if self.hud.controls.mode == sims_ui::command::Mode::Live {
                    self.hud.set_idle_speed(false);
                }
            }
            Err(e) => {
                self.note = Some(format!("The speed was not changed: {e}"));
            }
        }
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
    }

    /// Whether the game is paused (global 17).
    pub fn paused(&self) -> bool {
        self.lot.world().paused()
    }

    /// Pauses the game, or runs it again, as the panel's pause button
    /// does. A paused lot goes on ticking, with its clock standing
    /// ([`Lot::pause`]).
    pub fn toggle_pause(&mut self) {
        if self.coop.is_active() {
            let next_paused = !self.coop.session().is_some_and(|s| s.time.explicit_pause);
            if let Err(e) = self.coop.submit_command(NetCommand::SetPaused {
                paused: next_paused,
            }) {
                self.note = Some(format!("The pause was not changed: {e}"));
            }
            return;
        }
        let result = if self.paused() {
            self.lot.unpause().map(|_| self.pace.restart())
        } else {
            self.lot.pause().map(|_| ())
        };
        if let Err(e) = result {
            self.note = Some(format!("The pause was not changed: {e}"));
        }
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
    }

    /// Whether the pointer is currently inside an active edge-scrolling band
    /// (without mutating accumulator state), used to schedule display-refresh
    /// wakeups.
    pub fn is_edge_scrolling(&self, dragging: bool) -> bool {
        if dragging
            || !self.hud.controls.options.play.edge_scrolling
            || self.hud.has_ui_dialog()
            || self.lot.asking().is_some()
            || self.lot.asking_special_panel().is_some()
            || self.shop_dialog.is_open()
            || self.book_dialog.is_open()
            || self.phonebook_dialog.is_open()
            || self.hud.menu.is_some()
            || self.cheat_console_open()
        {
            return false;
        }
        let Some([px, py]) = self.pointer else {
            return false;
        };
        let (vw, vh) = self.house_viewport_size();
        let (w, h) = (vw as i32, vh as i32);
        const EDGE: i32 = 32;
        if w <= EDGE * 2 || h <= EDGE * 2 || px < 0 || py < 0 || px >= w || py >= h {
            return false;
        }
        px < EDGE || px >= w - EDGE || py < EDGE || py >= h - EDGE
    }

    /// Exact microseconds until the lot next needs an event-loop wakeup, or
    /// `None` when completely idle (allowing `ControlFlow::Wait`).
    pub fn next_wake_us(&self, dragging: bool, anim_interval_us: u64) -> Option<u64> {
        self.next_wake_us_with_window(dragging, anim_interval_us, true, false)
    }

    /// Like [`Game::next_wake_us`], gating pure presentation wakeups on
    /// `can_draw` and yielding to the redraw pacer when `frame_dirty` is
    /// already set so a zero-sized window or deferred redraw never spins.
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
        if self.coop.is_active_or_starting() || self.pending_coop_restage().is_some() {
            consider(5_000);
        }
        let ticks_active = self.hud.controls.mode == Mode::Live
            && !self.hud.has_ui_dialog()
            && !self.book_dialog.is_open()
            && !self.phonebook_dialog.is_open()
            && self.destination_picker.is_none()
            && self.lot.asking().is_none()
            && self.lot.asking_special_panel().is_none()
            && self.stopped.is_none();
        if ticks_active {
            consider(self.pace.remaining_us(self.lot.world().speed()));
        }
        // The simulation dialog's 40 s timer presses its button whether or
        // not the window can draw.
        if (!self.coop.is_active_or_starting() || self.coop_runs_auto_answer())
            && !self.auto_answer_held()
            && let Some(us) = self.hud.auto_answer_remaining_us(self.clock_us)
        {
            consider(us.max(1));
        }
        if can_draw {
            if let Some(picker) = &self.destination_picker {
                if let Some(us) = picker.next_wake_us(self.clock_us, anim_interval_us) {
                    consider(us);
                }
                if let Some(us) = self.coop_lobby.next_blink_us(self.clock_us) {
                    consider(us);
                }
                return best;
            }
            if self.is_edge_scrolling(dragging)
                || self.pie_animating()
                || self.ui_animating()
                || self.ui_animating_prev
                || self.camera_panning()
            {
                consider(anim_interval_us.max(1_000));
            }
            if self.people.has_bob(self.lot.world()) {
                let elapsed = self.clock_us.saturating_sub(self.drawn_us);
                let rem = SPIN_FRAME_US.saturating_sub(elapsed);
                if rem > 0 {
                    consider(rem);
                } else if !frame_dirty {
                    // Once `frame_dirty` is true, `redraw_pacer` owns the next
                    // presentation deadline; never return 0 us and busy-spin.
                    consider(anim_interval_us.max(1_000));
                }
            }
            if let Some(us) = self.hud.next_blink_us(self.clock_us) {
                consider(us);
            }
            if let Some(us) = self.coop_lobby.next_blink_us(self.clock_us) {
                consider(us);
            }
        }
        best
    }

    /// Whether the pause button's 500 ms blink phase changed since the
    /// frame was last drawn.
    pub fn blink_due(&self) -> bool {
        self.blink_dirty
    }

    /// Whether the plumb bob has spun enough since the frame was drawn to
    /// draw it again, though no tick ran (as while a dialog waits): a
    /// plumb bob is on the lot and [`SPIN_FRAME_US`] have passed.
    pub fn spin_due(&self) -> bool {
        self.clock_us.saturating_sub(self.drawn_us) >= SPIN_FRAME_US
            && self.people.has_bob(self.lot.world())
    }

    /// Runs the ticks owed after `elapsed_us` more microseconds at the
    /// simulation's speed. Returns how many ran.
    ///
    /// While a dialog waits for the player, no time is owed: the game
    /// waits, as the original does, and the idle loop's speed is not set
    /// (the lot takes nothing between ticks then; **unverified** whether
    /// the original's idle loop sets it while its dialog is up).
    ///
    /// Each call is a pass of the original's idle loop, so it first sets
    /// the idle loop's own speed ([`Lot::idle_speed`]), whether a tick is
    /// owed or not, paused or not: a speed the player picks after that
    /// stands. The same check runs again before each tick owed. Whenever
    /// it calls the setter, even for the speed the game already has, the
    /// time owed starts again and no more ticks run in this call, as the
    /// setter restarts the original's clocks. The original may then run
    /// an update at once, when its second clock (`0x4732de`) is ready
    /// (`0x46da1b`); here the next tick waits a period of the new speed
    /// (**unverified**, and the window's timing, not the simulation's).
    pub fn advance(&mut self, elapsed_us: u64) -> u32 {
        self.advance_while(elapsed_us, elapsed_us, |_| true).0
    }

    /// Advances the presentation clock by `clock_elapsed_us` and runs the
    /// ticks owed for `step_elapsed_us` while `keep_going(ran)` returns
    /// `true`. Returns `(ran, capped)`, where `capped` is `true` if
    /// `keep_going` stopped the batch before all owed ticks ran (in which case
    /// [`Pace::restart`] drops the leftover fraction).
    ///
    /// Keeping `clock_elapsed_us` separate from `step_elapsed_us` lets the
    /// interactive window loop drop capped simulation time from scheduler
    /// pacing without slowing the plumb bob spin or pause-button blink;
    /// headless callers use [`Game::advance`].
    pub fn advance_while(
        &mut self,
        clock_elapsed_us: u64,
        step_elapsed_us: u64,
        mut keep_going: impl FnMut(u32) -> bool,
    ) -> (u32, bool) {
        self.clock_us = self.clock_us.saturating_add(clock_elapsed_us);
        self.hud.set_clock_us(self.clock_us);
        if self.step_camera_pan() {
            self.blink_dirty = true;
        }
        let ui_anim_now = self.hud.is_ui_animating(self.clock_us);
        if ui_anim_now || self.ui_animating_prev || self.hud.needs_sample_due(self.clock_us) {
            self.blink_dirty = true;
        }
        self.ui_animating_prev = ui_anim_now;
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us)
            || self.hud.sync_dialog_blink(self.clock_us)
            || self.coop_lobby.sync_blink(self.clock_us)
        {
            self.blink_dirty = true;
        }
        if let Some(picker) = &mut self.destination_picker
            && picker.tick(self.clock_us)
        {
            self.blink_dirty = true;
        }
        if self.coop.is_active_or_starting() {
            // In co-op only a non-modal dialog's timer runs: when it runs
            // out it sends the answer as a command, as a press would, and
            // the first answer in tick order wins on every peer.
            if self.coop_runs_auto_answer()
                && !self.auto_answer_held()
                && let Some((caller, status, modal)) = self.hud.take_auto_answer(self.clock_us)
            {
                self.auto_answer(caller, status, modal);
                self.blink_dirty = true;
            }
            return self.advance_coop_while(clock_elapsed_us, step_elapsed_us, keep_going);
        }
        // The shown simulation dialog's 40 s timer (wall clock, paused or
        // not) presses its last button when it runs out (`0x5248a8`).
        // A special panel or shell modal in front holds it until it closes.
        if !self.auto_answer_held()
            && let Some((caller, status, modal)) = self.hud.take_auto_answer(self.clock_us)
        {
            self.auto_answer(caller, status, modal);
            self.blink_dirty = true;
            if modal {
                // The answered tick ran again at once; the time spent
                // waiting is owed nothing, as after a press.
                self.pace.restart();
                return (0, false);
            }
        }
        if self.hud.controls.mode != Mode::Live
            || self.hud.has_ui_dialog()
            || self.lot.asking().is_some()
            || self.lot.asking_special_panel().is_some()
            || self.trip_request().is_some()
        {
            self.pace.restart();
            return (0, false);
        }
        if self.idle_speed() {
            return (0, false);
        }
        let due = self.pace.due(step_elapsed_us, self.lot.world().speed());
        let (ran, capped) = self.run_while(due, true, &mut keep_going);
        if capped {
            self.pace.restart();
        }
        if ran > 0 {
            self.dirty = true;
        }
        if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
            self.blink_dirty = true;
        }
        (ran, capped)
    }

    /// Sets the idle loop's own speed between ticks ([`Lot::idle_speed`]).
    /// True if it called the setter, which starts the time owed again.
    pub(super) fn idle_speed(&mut self) -> bool {
        // A lot that cannot take it (halted, or not loaded) has stopped,
        // and the banner says why.
        let set = self.lot.idle_speed().unwrap_or(false);
        if set {
            self.pace.restart();
            // 0x46d7b5 -> 0x109(1) when entering idle speed, 0x46d803 ->
            // 0x109(0) when leaving (`return_speed` reset to -1).
            self.hud
                .set_idle_speed(self.lot.world().return_speed() >= 0);
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
        }
        set
    }

    /// The dialog the lot waits on, if it waits for the player.
    pub fn asking(&self) -> Option<&sims_vm::boot::Asking> {
        self.lot.asking()
    }

    /// The special panel (`Dialog` kinds `5..=15`) the lot waits on, if it waits
    /// for the player.
    pub fn asking_special_panel(&self) -> Option<&AskingSpecialPanel> {
        self.lot.asking_special_panel()
    }

    /// The active [`SpecialPanelRequest`] currently presented in the UI, if any.
    pub fn active_special_panel_request(&self) -> Option<&SpecialPanelRequest> {
        self.active_special_panel.as_ref().map(|a| &a.request)
    }

    /// Where the dialog shown in the last frame drawn is, and its buttons.
    pub fn dialog(&self) -> Option<&sims_ui::dialog::Layout> {
        self.hud.dialog()
    }

    /// Runs up to `ticks` ticks, stopping at the first that fails or that
    /// shows a dialog the player has to answer, and after a tick that asks
    /// for a trip ([`Game::trip_request`]).
    pub fn run(&mut self, ticks: u32) -> u32 {
        let ran = self.run_while(ticks, false, |_| true).0;
        if ran > 0 {
            self.dirty = true;
        }
        ran
    }

    /// Runs up to `ticks` ticks, stopping at the first that fails, before
    /// `keep_going(ran)` returns `false`, and, if `idle`, before a tick the
    /// idle loop's speed sets the speed for ([`Game::idle_speed`]) or after a
    /// tick that changes the simulation's speed or return speed (such as
    /// Special Effect resetting to normal, `0x4f684c` → `0x46d697`). Any
    /// mid-tick speed or return-speed change updates the panel's speed and
    /// restarts the time owed, whether `idle` is set or not.
    pub(super) fn run_while(
        &mut self,
        ticks: u32,
        idle: bool,
        mut keep_going: impl FnMut(u32) -> bool,
    ) -> (u32, bool) {
        let mut ran = 0;
        let mut capped = false;
        while ran < ticks && self.stopped.is_none() {
            // A trip a script asked for is carried out between ticks: the
            // host leaves the lot before the next one.
            if self.trip_request().is_some() {
                break;
            }
            if idle && self.idle_speed() {
                break;
            }
            if !keep_going(ran) {
                capped = true;
                break;
            }
            self.drop_trip_declined_in_coop();
            let speed_before = (self.lot.world().speed(), self.lot.world().return_speed());
            // A dialog that does not stop the game is left for the player
            // in play mode, as the original does. Set before every step: no
            // save holds it, so a restore or a trip turns it off.
            self.lot.present_non_modal_dialogs(self.play);
            let step = if self.play {
                self.lot.step_play()
            } else {
                self.lot.step()
            };
            match step {
                Ok(update) => {
                    ran += 1;
                    if !update.non_modal_shown.is_empty() {
                        // Read now, as the original makes the widget at the
                        // end of the update (0x436da0): an informing
                        // caller goes on, and its text is that tick's.
                        self.hud.read_non_modal_shown(
                            self.lot.world(),
                            &self.assets,
                            &update.non_modal_shown,
                        );
                    }
                    let at = self.lot.ticks();
                    for line in in_order(at, &update.errors, &update.frame_errors) {
                        self.errors.push(at, line);
                    }
                    self.view.masks_rebuilt(update.masks_rebuilt);
                    self.keep_sounds(update.sounds);
                    self.keep_ui_effects(update.ui_effects);
                    for (req, resp) in update.special_panels_answered {
                        if matches!(
                            req,
                            SpecialPanelRequest::DowntownLotPicker
                                | SpecialPanelRequest::VacationLotPicker { .. }
                                | SpecialPanelRequest::OldTownLotPicker { .. }
                                | SpecialPanelRequest::StudioTownLotPicker { .. }
                                | SpecialPanelRequest::MagicTownLotPicker { .. }
                        ) && let SpecialPanelResponse::Lot(Some(lot_id)) = resp
                            && lot_id > 0
                        {
                            self.pending_travel_lot = Some((req, lot_id as u32));
                        }
                    }
                }
                // The tick shows a dialog or special panel: it runs once it is
                // answered, and no time is owed from before the wait.
                Err(StepError::Asking { .. }) => {
                    self.pace.restart();
                    self.sync_special_panel();
                    // The widget, and so its 40 s timer (0x436c89), is
                    // made now, not at the next frame drawn.
                    if let Some(asking) = self.lot.asking() {
                        self.hud.read_dialog(&asking.world, &self.assets);
                    }
                    break;
                }
                Err(error) => {
                    match &error {
                        // A strict step whose frame failed after its tick
                        // ran still made that tick's sounds.
                        StepError::Frame {
                            update: Some(update),
                            ..
                        } => {
                            self.view.masks_rebuilt(update.masks_rebuilt);
                            self.keep_sounds(update.sounds.clone());
                            self.keep_ui_effects(update.ui_effects.clone());
                        }
                        // A play step whose tick failed hands back what
                        // it caught and made before the failure.
                        StepError::Tick {
                            tick,
                            update: Some(update),
                            ..
                        } => {
                            for line in in_order(*tick, &update.errors, &update.frame_errors) {
                                self.errors.push(*tick, line);
                            }
                            self.view.masks_rebuilt(update.masks_rebuilt);
                            self.keep_sounds(update.sounds.clone());
                            self.keep_ui_effects(update.ui_effects.clone());
                        }
                        _ => {}
                    }
                    self.stop(&error);
                }
            }
            // `Lot::step` runs `World::idle_speed` before the tick (0x46d9d6).
            // Detect `0x109(1)` / `0x109(0)` transitions directly from
            // `return_speed`'s sign change so even a same-tick Special Effect
            // `0x10c` reset (`(NORMAL, -1) -> (IDLE, NORMAL) -> (NORMAL, NORMAL)`)
            // arms `[control_panel + 0x1ec]` under both `Game::run` and
            // `Game::advance`.
            let return_after = self.lot.world().return_speed();
            if speed_before.1 < 0 && return_after >= 0 {
                self.hud.set_idle_speed(true);
            } else if speed_before.1 >= 0 && return_after < 0 {
                self.hud.set_idle_speed(false);
            }
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
            if sync_speed_after_step(&mut self.pace, speed_before, self.lot.world()) && idle {
                break;
            }
        }
        if ticks > 0 && self.stopped.is_none() {
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        }
        if ran > 0 || self.stopped.is_some() {
            self.hud.sync_character_names(self.lot.character_names());
            let world = self.lot.world();
            self.people.refresh(world);
            // The room lights change as lamps and the time of day do.
            match sims_render::Lot::from_world(world) {
                Ok(shape) => self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation()),
                Err(e) => {
                    self.people
                        .note(vec![format!("the lot is not redrawn: {e}")]);
                }
            }
        }
        (ran, capped)
    }

    /// Keeps a step's sound requests for the host, at most
    /// [`SOUNDS_KEPT`], the oldest dropped first.
    pub(super) fn keep_sounds(&mut self, sounds: Vec<SoundEvent>) {
        keep(&mut self.sounds, sounds, SOUNDS_KEPT);
    }

    /// Applies and queues a step's UI Effect highlight requests.
    pub(super) fn keep_ui_effects(&mut self, effects: Vec<UiEffectEvent>) {
        for &effect in &effects {
            match effect {
                UiEffectEvent::Button { id, highlight } => {
                    if highlight {
                        self.highlighted_buttons.insert(id);
                    } else {
                        self.highlighted_buttons.remove(&id);
                    }
                }
                UiEffectEvent::Portrait { guid, highlight } => {
                    if highlight {
                        self.highlighted_portraits.insert(guid);
                    } else {
                        self.highlighted_portraits.remove(&guid);
                    }
                }
                UiEffectEvent::Relationship { guid, highlight } => {
                    // `0x4769ca` resolves the first `cWinRelationship` control
                    // without comparing neighbor identity, so a clear for any
                    // valid neighbor clears the single active highlight.
                    self.highlighted_relationships.clear();
                    if highlight {
                        self.highlighted_relationships.insert(guid);
                    }
                }
            }
        }
        keep(&mut self.ui_effects, effects, SOUNDS_KEPT);
    }

    pub(super) fn stop(&mut self, error: &StepError) {
        let at = self.lot.ticks();
        self.stopped = Some(match error {
            StepError::Incomplete => "The load did not complete, so the lot cannot run.".into(),
            _ => format!("The lot stopped after tick {at}: {error}"),
        });
    }

    /// The status line across the top: why the lot stopped, else the last
    /// error caught for a while after it, then the last press's note.
    pub(super) fn status(&self) -> Option<String> {
        let stopped = self
            .stopped
            .clone()
            .or_else(|| self.errors.recent(self.lot.ticks()));
        match (&stopped, &self.note) {
            (Some(stop), Some(note)) => Some(format!("{stop}  —  {note}")),
            (Some(line), None) | (None, Some(line)) => Some(line.clone()),
            (None, None) => None,
        }
    }
}
