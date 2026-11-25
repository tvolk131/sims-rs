//! The co-op lobby and multiplayer live shell (Wave 3).

use super::*;

impl Game {
    // -------------------------------------------------------------------------
    // Co-op Multiplayer Live Shell (Wave 3)
    // -------------------------------------------------------------------------

    /// Whether the Host / Join Co-op Lobby modal dialog is currently open.
    pub fn coop_lobby_open(&self) -> bool {
        self.coop_lobby.is_open()
    }

    /// Opens the Host / Join Co-op Lobby dialog on `tab` (defaulting to Host
    /// if offline/hosting or Join if currently a client).
    pub fn open_coop_lobby(&mut self, tab: Option<LobbyTab>) {
        let target_tab = tab.unwrap_or(if self.coop.is_peer() {
            LobbyTab::Join
        } else {
            LobbyTab::Host
        });
        self.coop_lobby.open(target_tab);
    }

    /// Closes the Host / Join Co-op Lobby dialog.
    pub fn close_coop_lobby(&mut self) {
        self.coop_lobby.close();
    }

    /// Toggles the Host / Join Co-op Lobby dialog (`F6` / `Ctrl+M`).
    pub fn toggle_coop_lobby(&mut self) -> bool {
        if self.coop_lobby.is_open() {
            self.coop_lobby.close();
        } else {
            self.open_coop_lobby(None);
        }
        true
    }

    /// Switches between Host and Join tabs in the open Co-op Lobby dialog.
    pub fn coop_lobby_switch_tab(&mut self) -> bool {
        if !self.coop_lobby.is_open() {
            return false;
        }
        let next = match self.coop_lobby.tab() {
            LobbyTab::Host => LobbyTab::Join,
            LobbyTab::Join => LobbyTab::Host,
        };
        self.coop_lobby.set_tab(next);
        true
    }

    /// Types a character into the Co-op Lobby Join ticket box.
    pub fn coop_lobby_char(&mut self, ch: char) -> bool {
        self.coop_lobby.insert_char(ch)
    }

    /// Pastes text (or system clipboard when `text` is `None`) into the Co-op
    /// Lobby Join ticket box.
    pub fn coop_lobby_paste(&mut self, text: Option<&str>) -> bool {
        match text {
            Some(s) => self.coop_lobby.paste_text(s),
            None => clipboard_paste().is_some_and(|s| self.coop_lobby.paste_text(&s)),
        }
    }

    /// Deletes the last character from the Co-op Lobby Join ticket box.
    pub fn coop_lobby_backspace(&mut self) -> bool {
        self.coop_lobby.backspace()
    }

    /// Switches between the Host and Join tabs in the open Co-op Lobby (`Tab`).
    pub fn coop_lobby_toggle_tab(&mut self) -> bool {
        if !self.coop_lobby.is_open() {
            return false;
        }
        let next = match self.coop_lobby.tab() {
            LobbyTab::Host => LobbyTab::Join,
            LobbyTab::Join => LobbyTab::Host,
        };
        self.coop_lobby.set_tab(next);
        true
    }

    /// Submits the active Co-op Lobby tab (`Enter`).
    pub fn coop_lobby_submit(&mut self) -> Outcome {
        let snap = self.coop_snapshot();
        match self.coop_lobby.submit(&snap) {
            Some(cmd) => self.execute_coop_ui_command(cmd),
            None => Outcome::default(),
        }
    }

    /// Handles `Escape` while the Co-op Lobby dialog is open.
    pub fn coop_lobby_escape(&mut self) -> Outcome {
        let snap = self.coop_snapshot();
        match self.coop_lobby.escape(&snap) {
            Some(cmd) => self.execute_coop_ui_command(cmd),
            None => Outcome::default(),
        }
    }

    /// Whether a co-op session is active or asynchronously starting up.
    pub fn coop_active(&self) -> bool {
        self.coop.is_active_or_starting()
    }

    /// Whether the local player is currently the authoritative co-op Host.
    pub fn coop_is_host(&self) -> bool {
        self.coop.is_host()
    }

    /// Whether the local player is currently a joined/connecting co-op Peer.
    pub fn coop_is_peer(&self) -> bool {
        self.coop.is_peer()
    }

    /// Whether local saves/autosaves must be blocked because this `Game` is
    /// currently connected as a peer OR holds a remote host's restored world
    /// (persists even after disconnect so a host's world never overwrites a
    /// guest's local REDB/standalone save slots).
    pub fn coop_blocks_local_save(&self) -> bool {
        self.coop.is_peer() || self.coop_remote_world
    }

    /// Returns `(host_house, host_family)` if a verified `"SCOP"` v2 snapshot
    /// is parked awaiting guest shell restaging onto the host's matching lot.
    pub fn pending_coop_restage(&self) -> Option<(u32, u32)> {
        self.coop.pending_restage_target()
    }

    /// Detaches and returns the live [`CoopController`] (used when the shell
    /// re-stages a fresh `Game` for a remote host's lot before completing a
    /// parked snapshot restore).
    pub fn take_coop_controller(&mut self) -> CoopController {
        std::mem::take(&mut self.coop)
    }

    /// Installs a transferred [`CoopController`] onto a freshly staged `Game`.
    pub fn install_coop_controller(&mut self, coop: CoopController, mark_remote: bool) {
        self.coop = coop;
        if mark_remote {
            self.coop_remote_world = true;
        }
    }

    /// Attempts to complete a parked host-lot snapshot restore onto this
    /// candidate `Game`, preserving the parked snapshot on `OtherHouse` /
    /// `OtherLoad` mismatches so the shell can try fallback local candidates.
    pub fn try_complete_pending_coop_restage(
        &mut self,
    ) -> Result<(), crate::coop::RestageAttemptError> {
        let note = self.coop.try_complete_pending_restage(&mut self.lot)?;
        self.coop_remote_world = true;
        self.sync_after_coop_snapshot_restore();
        self.note = Some(note);
        Ok(())
    }

    /// Completes a parked host-lot snapshot restore onto this newly staged
    /// `Game`, verifying the snapshot transactionally BEFORE sending the
    /// completion ACK to the host.
    pub fn complete_pending_coop_restage(&mut self) -> Result<(), String> {
        let note = self.coop.complete_pending_restage(&mut self.lot)?;
        self.coop_remote_world = true;
        self.sync_after_coop_snapshot_restore();
        self.note = Some(note);
        Ok(())
    }

    /// Fails and cleans up a parked host-lot restage with an actionable message.
    pub fn fail_pending_coop_restage(&mut self, reason: String) {
        self.coop
            .fail_pending_restage(&mut self.lot, reason.clone());
        self.note = Some(reason);
    }

    /// Local assigned [`sims_net::PlayerId`], once hosting or joined.
    pub fn coop_local_player_id(&self) -> Option<NetPlayerId> {
        self.coop.local_player_id()
    }

    /// Currently selected Sim for the local player (their per-player selection
    /// in co-op, or `lot.world().selected()` in single-player).
    pub fn local_selected_sim(&self) -> Option<u16> {
        if self.coop.is_active() {
            self.coop.local_selected_sim()
        } else {
            self.lot.world().selected()
        }
    }

    /// Shareable `simscoop1...` invite ticket when hosting over `iroh`.
    pub fn coop_invite_ticket(&self) -> Option<&str> {
        self.coop.invite_ticket()
    }

    /// Read-only access to the live [`CoopController`].
    pub fn coop_controller(&self) -> &CoopController {
        &self.coop
    }

    /// Sets the local player display name for subsequent co-op sessions.
    pub fn coop_set_player_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        if !name.trim().is_empty() {
            self.coop_player_name = name;
        }
    }

    /// Configures `iroh` binding mode (`P2pDefault` or `Loopback`) for async
    /// co-op startup.
    pub fn coop_set_iroh_config(&mut self, config: IrohConfig) {
        self.coop_iroh_config = config;
    }

    /// Marks whether this lot is an away-from-home travel visit or community
    /// destination edit lot (refused for v1 home-lot co-op).
    pub fn coop_set_travel_blocked(&mut self, blocked: bool) {
        self.coop_travel_blocked = blocked;
    }

    /// Checks v1 home-lot co-op eligibility per `docs/vision.md`:
    /// refuses tutorial lots (`Global 26 != 0`) and away-from-home travel /
    /// community destination lots cleanly with an explicit message.
    pub fn coop_preflight_check(&self) -> Result<(), String> {
        if self.stopped.is_some() {
            return Err("cannot start co-op while the lot is stopped".into());
        }
        if self.lot.world().globals.get(26).copied().unwrap_or(0) != 0 {
            return Err(
                "Tutorial lots are single-player only and cannot be played in co-op mode.".into(),
            );
        }
        if self.coop_travel_blocked || !self.lot.residential() {
            return Err(
                "Co-op multiplayer v1 is only available on home lots (return home from travel first)."
                    .into(),
            );
        }
        Ok(())
    }

    /// Starts hosting a co-op session over real `iroh` asynchronously without
    /// blocking the UI thread.
    pub fn coop_start_host_async(
        &mut self,
        options: Option<CoopStartOptions>,
    ) -> Result<(), String> {
        if let Err(err) = self.coop_preflight_check() {
            self.coop.set_error(err.clone());
            self.note = Some(err.clone());
            return Err(err);
        }
        self.cancel_active_buy_build_tools();
        self.close_menu();
        let opts = options.unwrap_or_else(|| CoopStartOptions {
            player_name: self.coop_player_name.clone(),
            iroh_config: self.coop_iroh_config,
            ..CoopStartOptions::default()
        });
        let install_root = self.hud.install().root.clone();
        if let Err(err) = self.coop.start_host_iroh_async(install_root, opts) {
            self.coop.set_error(err.clone());
            self.note = Some(err.clone());
            return Err(err);
        }
        self.note = Some("Starting co-op host over iroh...".into());
        Ok(())
    }

    /// Validates `ticket` and joins a co-op session over real `iroh`
    /// asynchronously without blocking the UI thread.
    pub fn coop_start_join_async(
        &mut self,
        ticket: String,
        options: Option<CoopStartOptions>,
    ) -> Result<(), String> {
        if let Err(err) = self.coop_preflight_check() {
            self.coop.set_error(err.clone());
            self.note = Some(err.clone());
            return Err(err);
        }
        self.cancel_active_buy_build_tools();
        self.close_menu();
        let opts = options.unwrap_or_else(|| CoopStartOptions {
            player_name: self.coop_player_name.clone(),
            iroh_config: self.coop_iroh_config,
            ..CoopStartOptions::default()
        });
        let install_root = self.hud.install().root.clone();
        if let Err(err) = self.coop.start_join_iroh_async(install_root, ticket, opts) {
            self.note = Some(err.clone());
            return Err(err);
        }
        self.note = Some("Connecting to co-op host...".into());
        Ok(())
    }

    /// Synchronously starts hosting on this lot over `transport`
    /// (`CoopTransport::Memory` or pre-bound `CoopTransport::Iroh`).
    pub fn coop_start_host_with_transport(
        &mut self,
        transport: CoopTransport,
        manifest: ManifestDigest,
        options: CoopStartOptions,
    ) -> Result<(), String> {
        if let Err(err) = self.coop_preflight_check() {
            self.coop.set_error(err.clone());
            self.note = Some(err.clone());
            return Err(err);
        }
        self.cancel_active_buy_build_tools();
        self.close_menu();
        let install = self.hud.install().clone();
        let frames = self.frames.clone();
        self.coop.start_host_with_transport(
            &mut self.lot,
            &install,
            self.play,
            frames,
            transport,
            manifest,
            options,
        )?;
        self.refresh_world();
        Ok(())
    }

    /// Synchronously starts joining a host at `host_addr` over `transport`
    /// (`CoopTransport::Memory` or pre-bound `CoopTransport::Iroh`).
    pub fn coop_start_join_with_transport(
        &mut self,
        transport: CoopTransport,
        host_addr: PeerAddress,
        manifest: ManifestDigest,
        player_name: impl Into<String>,
    ) -> Result<(), String> {
        if let Err(err) = self.coop_preflight_check() {
            self.coop.set_error(err.clone());
            self.note = Some(err.clone());
            return Err(err);
        }
        self.cancel_active_buy_build_tools();
        self.close_menu();
        self.coop
            .start_join_with_transport(transport, host_addr, manifest, player_name)?;
        Ok(())
    }

    /// Submits a raw [`sims_net::Command`] through the active co-op
    /// [`LockstepNode`](sims_net::LockstepNode).
    pub fn coop_submit_command(&mut self, command: NetCommand) -> Result<(), String> {
        self.coop.submit_command(command)
    }

    /// Submits a [`sims_net::BuyBuildEdit`] through the active co-op lockstep
    /// session.
    pub fn coop_submit_buy_build(&mut self, edit: BuyBuildEdit) -> Result<(), String> {
        self.coop.submit_command(NetCommand::BuyBuild(edit))
    }

    /// Polls pending async co-op startup and network/snapshot events.
    pub fn coop_poll(&mut self) -> CoopPollOutput {
        // Ensure no lingering VM Buy/Build cursors (e.g. Wall Cursor) survive
        // into the moment async startup finishes and attaches co-op to `Lot`.
        if self.coop.is_starting() && (self.wall_cursor.is_some() || self.buy_cursor.is_some()) {
            self.cancel_active_buy_build_tools();
        }
        let install = self.hud.install().clone();
        let frames = self.frames.clone();
        let out = self
            .coop
            .poll_and_handle(&mut self.lot, &install, self.play, frames);
        if out.snapshot_restored {
            self.sync_after_coop_snapshot_restore();
        }
        if let Some(note) = &out.note {
            self.note = Some(note.clone());
        }
        out
    }

    /// Deterministic headless co-op pump step (polls network/snapshots,
    /// optionally schedules one host tick, and steps all ready lockstep
    /// batches). Used by integration tests for both `MemoryMesh` and real
    /// `IrohTransport` loopback.
    pub fn coop_pump(
        &mut self,
        schedule_host_tick: bool,
        flush_peer: bool,
    ) -> Result<CoopPumpStats, String> {
        let mut stats = CoopPumpStats::default();
        let poll = self.coop_poll();
        if poll.snapshot_restored {
            stats.snapshots_restored += 1;
        }
        stats.drift_events += poll.drift_events as u32;

        if schedule_host_tick
            && self.coop.is_host()
            && let Some(sims_net::HostStepResult::Scheduled(_)) = self.coop.advance_host_tick()?
        {
            stats.host_scheduled = true;
        }

        let flush = self.coop.is_host() || flush_peer;
        while let Some(report) = self.coop.step_next_batch(&mut self.lot, flush)? {
            stats.batches_applied += 1;
            if report.stepped {
                stats.sim_ticks_stepped += 1;
            }
            self.apply_coop_batch_side_effects(&report);
        }

        let poll2 = self.coop_poll();
        if poll2.snapshot_restored {
            stats.snapshots_restored += 1;
        }
        stats.drift_events += poll2.drift_events as u32;
        self.tick_coop_special_panel_retry();
        Ok(stats)
    }

    /// Bounded retry guard shared by both headless (`coop_pump`) and live
    /// (`advance_coop_while`) co-op loops: if a peer's special-panel answer was
    /// dropped by full host ingress while the same dialog ID remains active,
    /// reopen the modal so the client can retry instead of waiting forever.
    pub(super) fn tick_coop_special_panel_retry(&mut self) -> bool {
        const COOP_SPECIAL_PANEL_RETRY_PUMPS: u32 = 120;
        let Some((submitted_id, ref mut pumps)) = self.submitted_coop_special_panel else {
            return false;
        };
        let still_active = self
            .lot
            .coop()
            .and_then(|c| c.active_special_panel.as_ref().map(|(id, _)| *id))
            == Some(submitted_id);
        if still_active {
            *pumps = pumps.saturating_add(1);
            if *pumps >= COOP_SPECIAL_PANEL_RETRY_PUMPS {
                self.submitted_coop_special_panel = None;
                self.sync_special_panel();
                return true;
            }
        } else {
            self.submitted_coop_special_panel = None;
        }
        false
    }

    /// Gracefully leaves any active co-op session, detaching `Lot::coop` and
    /// closing any `iroh` endpoint and background Tokio runtime cleanly.
    pub fn leave_coop(&mut self) {
        let was_remote = self.coop_remote_world;
        self.coop_buy_preview = None;
        self.coop_overlay_buttons = CoopOverlayButtons::default();
        self.submitted_coop_special_panel = None;
        self.coop.leave_and_shutdown(&mut self.lot);
        self.note = Some(if was_remote {
            "Disconnected from co-op session (read-only remote lot; return to Neighborhood for local saves)."
                .into()
        } else {
            "Disconnected from co-op session.".into()
        });
    }

    /// Non-blocking interactive Leave/Cancel for UI buttons so endpoint drain
    /// and runtime shutdown never stall the `winit` UI thread.
    pub fn leave_coop_nonblocking(&mut self) {
        let was_remote = self.coop_remote_world;
        self.coop_buy_preview = None;
        self.coop_overlay_buttons = CoopOverlayButtons::default();
        self.submitted_coop_special_panel = None;
        self.coop.leave_nonblocking(&mut self.lot);
        self.note = Some(if was_remote {
            "Disconnected from co-op session (read-only remote lot; return to Neighborhood for local saves)."
                .into()
        } else {
            "Disconnected from co-op session.".into()
        });
    }

    /// Builds a read-only [`CoopSnapshot`] for the Co-op Lobby & HUD Overlay.
    pub fn coop_snapshot(&self) -> CoopSnapshot {
        let lot = &self.lot;
        let hud = &self.hud;
        self.coop.ui_snapshot(lot, |sim_id| {
            let obj = lot.world().objects.get(&sim_id)?;
            lot.character_names()
                .get(&obj.guid)
                .cloned()
                .or_else(|| hud.name(obj.guid).map(str::to_owned))
                .or_else(|| Some(format!("Sim #{sim_id}")))
        })
    }

    pub(super) fn execute_coop_ui_command(&mut self, cmd: CoopCommand) -> Outcome {
        self.hud.play(crate::hud::CLICK);
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        match cmd {
            CoopCommand::StartHosting => {
                let _ = self.coop_start_host_async(None);
            }
            CoopCommand::Disconnect | CoopCommand::CancelConnect => {
                self.leave_coop_nonblocking();
            }
            CoopCommand::CopyInviteTicket(ticket) => match clipboard_copy(&ticket) {
                Ok(()) => {
                    self.note = Some("Invite ticket copied to clipboard.".into());
                }
                Err(err) => {
                    self.note = Some(format!("Could not copy invite ticket: {err}"));
                }
            },
            CoopCommand::SetHostOnlyTimeControl(locked) => {
                if let Err(err) = self
                    .coop
                    .submit_command(NetCommand::SetHostSpeedLock { locked })
                {
                    self.note = Some(format!("Could not toggle host time lock: {err}"));
                }
            }
            CoopCommand::JoinSession { ticket } => {
                let _ = self.coop_start_join_async(ticket, None);
            }
            CoopCommand::CloseDialog => {
                self.coop_lobby.close();
            }
        }
        Outcome::redraw()
    }

    pub(super) fn execute_coop_overlay_action(&mut self, action: CoopOverlayAction) -> Outcome {
        match action {
            CoopOverlayAction::OpenLobby => {
                self.open_coop_lobby(None);
            }
            CoopOverlayAction::CopyTicket => {
                if let Some(ticket) = self.coop.invite_ticket().map(str::to_owned) {
                    match clipboard_copy(&ticket) {
                        Ok(()) => {
                            self.coop_overlay_buttons.copied_flash = true;
                            self.note = Some("Invite ticket copied to clipboard.".into());
                        }
                        Err(err) => {
                            self.note = Some(format!("Could not copy invite ticket: {err}"));
                        }
                    }
                }
            }
            CoopOverlayAction::Leave => {
                self.leave_coop_nonblocking();
            }
        }
        Outcome::redraw()
    }

    pub(super) fn advance_coop_while(
        &mut self,
        clock_elapsed_us: u64,
        step_elapsed_us: u64,
        mut keep_going: impl FnMut(u32) -> bool,
    ) -> (u32, bool) {
        let toast_expired = self
            .coop
            .tick_toasts((clock_elapsed_us / 1_000).min(u64::from(u32::MAX)) as u32);
        if toast_expired {
            self.blink_dirty = true;
        }
        let poll = self.coop_poll();
        if poll.redraw {
            self.blink_dirty = true;
        }
        if !self.coop.is_active() {
            return (0, false);
        }

        let is_host = self.coop.is_host();
        if is_host {
            self.coop.accumulate_host_elapsed(step_elapsed_us, 8);
        }

        let mut ran = 0u32;
        let mut capped = false;
        let mut any_batch = false;

        // Interleave host tick scheduling and stepping 1-for-1 inside the budget
        // loop so the host never yields with scheduled-but-unapplied batches
        // that would create a tick gap for a mid-game joiner.
        while self.stopped.is_none() {
            if !keep_going(ran) {
                capped = true;
                break;
            }
            match self.coop.step_next_batch(&mut self.lot, is_host) {
                Ok(Some(report)) => {
                    any_batch = true;
                    if report.stepped {
                        ran += 1;
                    }
                    self.apply_coop_batch_side_effects(&report);
                    continue;
                }
                Ok(None) => {}
                Err(err) => {
                    self.note = Some(err);
                    break;
                }
            }
            if is_host {
                match self.coop.schedule_one_host_tick_if_due() {
                    Ok(true) => match self.coop.step_next_batch(&mut self.lot, true) {
                        Ok(Some(report)) => {
                            any_batch = true;
                            if report.stepped {
                                ran += 1;
                            }
                            self.apply_coop_batch_side_effects(&report);
                            continue;
                        }
                        Ok(None) => break,
                        Err(err) => {
                            self.note = Some(err);
                            break;
                        }
                    },
                    Ok(false) => break,
                    Err(err) => {
                        self.note = Some(err);
                        break;
                    }
                }
            } else {
                break;
            }
        }

        if self.tick_coop_special_panel_retry() {
            self.blink_dirty = true;
        }
        if any_batch || poll.snapshot_restored {
            self.dirty = true;
            self.blink_dirty = true;
            if self.hud.sync_clock_blink(self.lot.world(), self.clock_us) {
                self.blink_dirty = true;
            }
        }
        (ran, capped)
    }

    pub(super) fn apply_coop_batch_side_effects(
        &mut self,
        report: &sims_vm::coop::CoopBatchReport,
    ) {
        if let Some(update) = &report.update {
            let at = self.lot.ticks();
            for line in in_order(at, &update.errors, &update.frame_errors) {
                self.errors.push(at, line);
            }
            self.view.masks_rebuilt(update.masks_rebuilt);
            self.keep_sounds(update.sounds.clone());
            self.keep_ui_effects(update.ui_effects.clone());
        }
        if report.opened_dialog.is_some() {
            self.hud.clear_vm_dialog();
        }
        if report.opened_special_panel.is_some() {
            self.submitted_coop_special_panel = None;
            self.close_special_panel_ui();
            self.sync_special_panel();
        }
        let mut visual_changed = report.stepped
            || report.opened_dialog.is_some()
            || report.opened_special_panel.is_some();
        for (stamped, status) in &report.command_results {
            match status {
                sims_vm::coop::CoopCommandStatus::BuyBuildApplied
                | sims_vm::coop::CoopCommandStatus::QueuedInteraction { .. }
                | sims_vm::coop::CoopCommandStatus::CancelledInteraction { .. }
                | sims_vm::coop::CoopCommandStatus::DialogAnswered { .. }
                | sims_vm::coop::CoopCommandStatus::Applied => {
                    visual_changed = true;
                }
                sims_vm::coop::CoopCommandStatus::Refused(reason)
                    if Some(stamped.issuer) == self.coop.local_player_id() =>
                {
                    self.submitted_coop_special_panel = None;
                    visual_changed = true;
                    self.note = Some(format!("Co-op command refused: {reason}"));
                }
                sims_vm::coop::CoopCommandStatus::RejectedBySession(reason)
                    if Some(stamped.issuer) == self.coop.local_player_id() =>
                {
                    self.submitted_coop_special_panel = None;
                    visual_changed = true;
                    self.note = Some(format!("Co-op command rejected: {reason:?}"));
                }
                _ => {}
            }
        }
        if visual_changed {
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
            if self.lot.asking_special_panel().is_none() {
                self.submitted_coop_special_panel = None;
                self.close_special_panel_ui();
            } else {
                self.sync_special_panel();
            }
            self.hud.sync_character_names(self.lot.character_names());
            let world = self.lot.world();
            self.people.refresh(world);
            if let Ok(shape) = sims_render::Lot::from_world(world) {
                self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation());
            }
            if self.hud.controls.mode == Mode::Buy {
                self.layout_buy();
            } else if self.hud.controls.mode == Mode::Build {
                self.layout_build();
            }
        }
    }

    pub(super) fn sync_after_coop_snapshot_restore(&mut self) {
        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        if self.coop.is_peer() {
            self.coop_remote_world = true;
        }
        self.stopped = None;
        self.errors = Errors::default();
        self.buy_cursor = None;
        self.coop_buy_preview = None;
        self.buy_selected = None;
        self.buy_dragging = None;
        self.buy_commit_on_release = false;
        self.wall_cursor = None;
        self.build_drag = None;
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
        self.close_menu();
        self.reached = None;
        self.listed = None;
        self.highlighted = None;
        self.submitted_coop_special_panel = None;
        self.close_special_panel_ui();
        self.pending_travel_lot = None;
        self.declined_trip = None;
        self.hud.clear_vm_dialog();
        self.sync_special_panel();
        self.hud.sync_character_names(self.lot.character_names());
        self.people.refresh(self.lot.world());
        if let Ok(shape) = sims_render::Lot::from_world(self.lot.world()) {
            self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation());
        }
        self.clear_lot_cache();
        self.restore_generation = self.restore_generation.wrapping_add(1);
        self.blink_dirty = true;
    }

    /// Presentation-only Hand Tool pickup preview for Buy/Build modes in co-op
    /// (never calls `Lot::buy_pick_up`, keeping the shared world immutable until
    /// `BuyBuildEdit::MoveObject` / `SellObject` commits via lockstep).
    pub(super) fn coop_pick_up_preview(&mut self, target: u16, point: [i32; 2]) -> Outcome {
        if let Some(obj) = self.lot.world().objects.get(&target)
            && obj.person.is_none()
        {
            self.coop_buy_preview = Some(CoopBuyPreview {
                guid: obj.guid,
                existing_object_id: Some(target),
                position: obj.position,
                facing: obj.data.get(1).copied().unwrap_or(0).rem_euclid(8),
            });
            self.buy_in_house = true;
            self.buy_dragging = Some(point);
            self.buy_commit_on_release = false;
            self.pointer = Some(point);
            self.hud.play("UI_object_move");
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
            return Outcome::redraw();
        }
        self.hud.play(crate::hud::ERROR);
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    pub(super) fn hover_buy_coop(&mut self, point: [i32; 2]) -> bool {
        let at = [point[0] - pick::ORIGIN_COLUMN, point[1]];
        let fine = sims_render::unproject_terrain(&self.lot_shape, &self.view.camera, at)
            .unwrap_or_else(|| self.view.camera.unproject_fine(at));
        let snapped = [
            (fine[0] & !15) + 8,
            (fine[1] & !15) + 8,
            self.view.camera.level,
        ];
        if let Some(preview) = self.coop_buy_preview.as_mut() {
            if let Some(anchor) = self.buy_dragging {
                let dx = point[0] - anchor[0];
                let dy = point[1] - anchor[1];
                if dx * dx + dy * dy > 36 {
                    let quad = if dy < 0 {
                        if dx < 0 { 6 } else { 4 }
                    } else if dx < 0 {
                        0
                    } else {
                        2
                    };
                    let desired =
                        (quad - 2 * i16::from(self.view.camera.rotation & 3)).rem_euclid(8);
                    preview.facing = desired;
                    self.buy_facing = Some(desired);
                    return true;
                }
            }
            preview.position = snapped;
            return true;
        }
        if let Some(guid) = self.buy_selected {
            let facing = self
                .buy_facing
                .unwrap_or_else(|| sims_vm::default_buy_direction(self.view.camera.rotation));
            self.coop_buy_preview = Some(CoopBuyPreview {
                guid,
                existing_object_id: None,
                position: snapped,
                facing,
            });
            self.buy_in_house = true;
            return true;
        }
        false
    }

    pub(super) fn commit_buy_coop(&mut self) -> Outcome {
        let Some(preview) = self.coop_buy_preview.take() else {
            return Outcome::default();
        };
        let edit = match preview.existing_object_id {
            Some(object_id) => BuyBuildEdit::MoveObject {
                object_id,
                position: preview.position,
                facing: preview.facing,
                container: None,
            },
            None => BuyBuildEdit::BuyObject {
                guid: preview.guid,
                position: preview.position,
                facing: preview.facing,
                container: None,
            },
        };
        match self.coop.submit_command(NetCommand::BuyBuild(edit)) {
            Ok(()) => {
                self.buy_facing = Some(preview.facing);
                self.buy_last_guid = Some(preview.guid);
                if preview.existing_object_id.is_some() || !self.shift_held {
                    self.buy_selected = None;
                    self.hud.buy_state.selected = None;
                }
                self.hud.play("UI_object_move_place");
            }
            Err(e) => {
                self.note = Some(format!("Co-op placement not sent: {e}"));
                self.hud.play(crate::hud::ERROR);
            }
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    pub(super) fn commit_build_coop(&mut self, drag: BuildDrag) -> Outcome {
        // Stay strictly below `LockstepNode`'s 64-command per-player queue cap
        // so a single Build drag never silently drops trailing commands.
        const MAX_COOP_BUILD_DRAG_COMMANDS: usize = 48;
        // `sims_vm::build::cell_index(x, y) = x * 64 + y` (column-major stride 64).
        const FULL_GRID_STRIDE: usize = 64;
        let level = self.view.camera.level.clamp(1, 2) as u8;
        let mut sent = 0usize;
        let mut err_msg: Option<String> = None;
        match drag {
            BuildDrag::Wall {
                start_corner,
                end_corner,
                style,
                mode,
                plan,
            } => {
                if let Some(plan) = plan
                    && plan.valid
                    && plan.affordable
                {
                    let (x0, x1) = (
                        start_corner[0].min(end_corner[0]),
                        start_corner[0].max(end_corner[0]),
                    );
                    let y = start_corner[1] as i16;
                    let count = usize::try_from(x1.saturating_sub(x0)).unwrap_or(0);
                    if count > MAX_COOP_BUILD_DRAG_COMMANDS {
                        err_msg = Some(format!(
                            "Co-op wall drag ({count} segments) exceeds {MAX_COOP_BUILD_DRAG_COMMANDS}-segment limit; drag a smaller section."
                        ));
                    } else if x1 > x0 {
                        for x in x0..x1 {
                            let edit = if mode.is_demolish() {
                                BuyBuildEdit::RemoveWall {
                                    x: x as i16,
                                    y,
                                    level,
                                }
                            } else {
                                BuyBuildEdit::PlaceWall {
                                    x: x as i16,
                                    y,
                                    level,
                                    style: style.style_id(),
                                    pattern: style.default_pattern(),
                                }
                            };
                            if let Err(e) = self.coop.submit_command(NetCommand::BuyBuild(edit)) {
                                err_msg = Some(e);
                                break;
                            }
                            sent += 1;
                        }
                    } else {
                        err_msg = Some(
                            "Co-op wall edits currently support horizontal wall segments.".into(),
                        );
                    }
                }
            }
            BuildDrag::Floor {
                floor_id,
                mode,
                plan,
                ..
            } => {
                if let Some(plan) = plan
                    && plan.valid
                    && plan.affordable
                {
                    if plan.floor_changes.len() > MAX_COOP_BUILD_DRAG_COMMANDS {
                        err_msg = Some(format!(
                            "Co-op floor drag ({} tiles) exceeds {MAX_COOP_BUILD_DRAG_COMMANDS}-tile limit; drag a smaller area.",
                            plan.floor_changes.len()
                        ));
                    } else {
                        let pattern = if mode.is_remove() { 0 } else { floor_id };
                        for &(idx, _before, _after) in &plan.floor_changes {
                            let x = (idx / FULL_GRID_STRIDE) as i16;
                            let y = (idx % FULL_GRID_STRIDE) as i16;
                            let edit = BuyBuildEdit::SetFloorTile {
                                x,
                                y,
                                level,
                                pattern,
                            };
                            if let Err(e) = self.coop.submit_command(NetCommand::BuyBuild(edit)) {
                                err_msg = Some(e);
                                break;
                            }
                            sent += 1;
                        }
                    }
                }
            }
            BuildDrag::Wallpaper {
                faces,
                pattern_id,
                mode,
                plan,
            } => {
                if let Some(plan) = plan
                    && plan.valid
                    && plan.affordable
                {
                    if faces.len() > MAX_COOP_BUILD_DRAG_COMMANDS {
                        err_msg = Some(format!(
                            "Co-op wallpaper drag ({} faces) exceeds {MAX_COOP_BUILD_DRAG_COMMANDS}-face limit; drag a smaller section.",
                            faces.len()
                        ));
                    } else {
                        let pattern = if mode.is_strip() { 0 } else { pattern_id };
                        for face in faces {
                            let side = match face.bit {
                                WALL_NORTH => 0,
                                WALL_EAST => 1,
                                WALL_SOUTH => 2,
                                WALL_WEST => 3,
                                _ => continue,
                            };
                            let edit = BuyBuildEdit::SetWallpaper {
                                x: i16::from(face.tile[0]),
                                y: i16::from(face.tile[1]),
                                level,
                                side,
                                pattern,
                            };
                            if let Err(e) = self.coop.submit_command(NetCommand::BuyBuild(edit)) {
                                err_msg = Some(e);
                                break;
                            }
                            sent += 1;
                        }
                    }
                }
            }
            BuildDrag::Terrain { .. } | BuildDrag::Water { .. } | BuildDrag::Pool { .. } => {
                err_msg = Some(
                    "Terrain, Water, and Pool tools are not yet supported in co-op mode.".into(),
                );
            }
        }
        if let Some(msg) = err_msg {
            self.note = Some(msg);
            self.hud.play(crate::hud::ERROR);
        } else if sent > 0 {
            self.hud.play(crate::hud::BLD_DRAG_UP);
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    pub(super) fn ensure_coop_art(&mut self) {
        if self.coop_art.is_none()
            && let Ok(ui) = UiGraphics::open(self.hud.install())
            && let Ok(art) = sims_ui::coop::load_art(&ui, US_ENGLISH)
        {
            self.coop_art = Some(art);
        }
    }

    /// Draws distinct colored faceted diamond plumbob indicators above Sims
    /// selected by remote teammates (and above the local peer's own selected
    /// Sim when it differs from the host's native plumbob). Never runs in
    /// single-player (`coop == None`).
    pub(super) fn draw_coop_remote_indicators(&mut self, viewport: (u32, u32)) {
        let Some(session) = self.coop.session() else {
            return;
        };
        let host_selected = session
            .players
            .get(&session.host_id)
            .and_then(|p| p.selected_sim);
        let local_id = self.coop.local_player_id();
        let world = self.lot.world();
        let vp_w = i32::try_from(viewport.0)
            .unwrap_or(i32::MAX)
            .min(self.canvas.width as i32);
        let vp_h = i32::try_from(viewport.1)
            .unwrap_or(i32::MAX)
            .min(self.canvas.height as i32);
        if vp_w <= 0 || vp_h <= 0 {
            return;
        }

        // Group players by selected Sim so co-selected Sims fan out badges cleanly.
        type RemotePlumbobEntry = (NetPlayerId, String, [u8; 3], bool);
        let mut by_sim: BTreeMap<u16, Vec<RemotePlumbobEntry>> = BTreeMap::new();
        for player in session.players.values() {
            let Some(sim_id) = player.selected_sim else {
                continue;
            };
            // The host's selected Sim already has the native 3D plumbob when we
            // are the host (or when it is the host's own Sim); draw colored
            // accents for every remote player AND for the local peer when they
            // select a different Sim from the host.
            let is_local = Some(player.player_id) == local_id;
            if is_local && Some(sim_id) == host_selected && player.player_id == session.host_id {
                continue;
            }
            by_sim.entry(sim_id).or_default().push((
                player.player_id,
                player.name.clone(),
                player_badge_color(player.player_id),
                is_local,
            ));
        }
        if by_sim.is_empty() {
            return;
        }

        let (head_lift, half_w, half_h) = match self.view.camera.zoom {
            sims_render::Zoom::Far => (34i32, 4i32, 7i32),
            sims_render::Zoom::Medium => (68i32, 6i32, 11i32),
            sims_render::Zoom::Near => (132i32, 9i32, 16i32),
        };
        // Gentle vertical bob driven by `clock_us` (pure presentation).
        let phase = ((self.clock_us / 120_000) % 4) as i32;
        let bob = match phase {
            1 => -2,
            2 => -1,
            3 => 1,
            _ => 0,
        };

        for (sim_id, markers) in by_sim {
            let Some(obj) = world.objects.get(&sim_id) else {
                continue;
            };
            if obj.position[2] > self.view.camera.level
                || (obj.position[0] == 0 && obj.position[1] == 0)
            {
                continue;
            }
            let base = lot_point(&self.view.camera, &self.lot_shape, obj.position);
            let base_x = base[0] + pick::ORIGIN_COLUMN;
            let base_y = base[1] - head_lift + bob;
            let spacing = half_w * 2 + 6;
            let start_x = base_x - ((markers.len() as i32 - 1) * spacing) / 2;
            for (idx, (pid, _name, color, is_local)) in markers.iter().enumerate() {
                let cx = start_x + (idx as i32) * spacing;
                let cy = base_y;
                draw_coop_plumbob_diamond(
                    &mut self.canvas,
                    cx,
                    cy,
                    half_w,
                    half_h,
                    *color,
                    *is_local,
                    (vp_w, vp_h),
                );
                let _ = pid;
            }
        }
    }

    pub(super) fn draw_coop_buy_preview(&mut self, viewport: (u32, u32)) {
        let Some(preview) = &self.coop_buy_preview else {
            return;
        };
        let vp_w = i32::try_from(viewport.0)
            .unwrap_or(i32::MAX)
            .min(self.canvas.width as i32);
        let vp_h = i32::try_from(viewport.1)
            .unwrap_or(i32::MAX)
            .min(self.canvas.height as i32);
        let pt = lot_point(&self.view.camera, &self.lot_shape, preview.position);
        let cx = pt[0] + pick::ORIGIN_COLUMN;
        let cy = pt[1];
        draw_coop_plumbob_diamond(
            &mut self.canvas,
            cx,
            cy,
            10,
            6,
            [64, 224, 255],
            true,
            (vp_w, vp_h),
        );
    }

    pub(super) fn draw_coop_hud_and_lobby(&mut self) {
        let show_overlay = self.coop.is_active_or_starting();
        let show_lobby = self.coop_lobby.is_open();
        if !show_overlay && !show_lobby {
            self.coop_overlay_buttons.clear_rects();
            return;
        }
        self.ensure_coop_art();
        let snapshot = self.coop_snapshot();
        let art = self.coop_art.as_ref().unwrap_or(&self.hud.art);
        let (w, h) = (
            u32::try_from(self.canvas.width).unwrap_or(0),
            u32::try_from(self.canvas.height).unwrap_or(0),
        );
        let Ok(mut frame) =
            sims_ui::frame::Frame::from_packed(w, h, std::mem::take(&mut self.canvas.color))
        else {
            return;
        };
        if show_overlay {
            CoopHudOverlay::draw(&mut frame, &snapshot, art);
            let overlay_layout = CoopHudOverlay::layout([w, h], &snapshot, art);
            if let Some(panel) = overlay_layout.status_panel {
                let hosting = matches!(snapshot.connection, CoopConnectionState::Hosting { .. });
                self.coop_overlay_buttons.layout_and_draw(
                    &mut frame,
                    art,
                    panel,
                    hosting,
                    self.pointer,
                );
            } else {
                self.coop_overlay_buttons.clear_rects();
            }
        } else {
            self.coop_overlay_buttons.clear_rects();
        }
        if show_lobby {
            self.coop_lobby.draw(&mut frame, &snapshot, art);
        }
        self.canvas.color = frame.into_packed();
    }
}
