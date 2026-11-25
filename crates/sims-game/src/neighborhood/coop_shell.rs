//! The co-op lobby and session helpers.

use super::*;

impl NeighborhoodShell {
    // -------------------------------------------------------------------------
    // Co-op Multiplayer Lobby & Session Helpers
    // -------------------------------------------------------------------------

    /// Whether the co-op lobby modal is currently open (either on the active
    /// lot or over the Neighborhood shell).
    pub fn coop_lobby_open(&self) -> bool {
        if self.coop_lobby.is_open() {
            return true;
        }
        matches!(&self.screen, ActiveScreen::Lot(session) if session.game.coop_lobby_open())
    }

    /// Sets the local player display name used when hosting or joining co-op.
    pub fn coop_set_player_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            self.coop_player_name = trimmed.to_owned();
            if let ActiveScreen::Lot(session) = &mut self.screen {
                session
                    .game
                    .coop_set_player_name(self.coop_player_name.clone());
            }
        }
    }

    /// Configures the `iroh` binding mode (`P2pDefault` or `Loopback`) for
    /// future co-op sessions.
    pub fn coop_set_iroh_config(&mut self, config: sims_net::IrohConfig) {
        self.coop_iroh_config = config;
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.coop_set_iroh_config(config);
        }
    }

    /// Opens the co-op lobby dialog on the active lot or over the Neighborhood
    /// screen.
    pub fn open_coop_lobby(&mut self, tab: Option<LobbyTab>) {
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.close_camera_strip();
            session.game.open_coop_lobby(tab);
            return;
        }
        self.clear_hover_state();
        self.coop_error = None;
        self.coop_lobby.open(tab.unwrap_or(LobbyTab::Join));
    }

    /// Toggles the co-op lobby dialog open or closed (`F6` / `Ctrl+M`).
    pub fn toggle_coop_lobby(&mut self) -> bool {
        if let ActiveScreen::Lot(session) = &mut self.screen {
            return session.game.toggle_coop_lobby();
        }
        if self.coop_lobby.is_open() {
            self.coop_lobby.close();
        } else {
            self.coop_error = None;
            self.coop_lobby.open(LobbyTab::Join);
        }
        true
    }

    /// Pastes a ticket into the open co-op lobby dialog (`Ctrl/Cmd+V`), using
    /// the OS clipboard when `text` is `None`.
    pub fn coop_lobby_paste(&mut self, text: Option<&str>) -> bool {
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_lobby_open()
        {
            return session.game.coop_lobby_paste(text);
        }
        if !self.coop_lobby.is_open() {
            return false;
        }
        let pasted = match text {
            Some(s) => s.to_owned(),
            None => crate::coop::clipboard_paste().unwrap_or_default(),
        };
        if pasted.is_empty() {
            return false;
        }
        self.coop_lobby.paste_text(&pasted)
    }

    /// Cleanly leaves any active co-op session and shuts down its background
    /// transport runtime. Safe to call when offline.
    pub fn leave_coop_if_active(&mut self) {
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.game.coop_active()
        {
            session.game.leave_coop();
        }
    }

    /// Picks a default residential home lot in the current neighborhood for
    /// CLI/shell co-op host or join bootstrapping (prefers lot 5 / Goth house,
    /// then first occupied home lot `1..=10`, then lot 5).
    pub(super) fn default_coop_home_lot(&self) -> u32 {
        if self.stored.family_in(5).is_some() {
            return 5;
        }
        for house in 1..=10 {
            if self.stored.family_in(house).is_some() {
                return house;
            }
        }
        5
    }

    /// Hosts a co-op session on the currently open home lot, or enters the
    /// default residential lot first when called from Neighborhood view.
    pub fn coop_host_current_or_default(&mut self) -> Result<(), String> {
        if !matches!(self.screen, ActiveScreen::Lot(_)) {
            let house = self.default_coop_home_lot();
            self.enter_lot(house)?;
        }
        let ActiveScreen::Lot(session) = &mut self.screen else {
            return Err("could not open a home lot to host co-op".into());
        };
        session.game.open_coop_lobby(Some(LobbyTab::Host));
        session.game.coop_start_host_async(None)
    }

    /// Validates `ticket`, enters a local host-compatible lot shell if on the
    /// Neighborhood screen, and starts joining the remote co-op host via
    /// non-blocking `IrohTransport` + `"SCOP"` v2 snapshot restore.
    pub fn coop_join_from_shell(&mut self, ticket: String) -> Result<(), String> {
        let trimmed = ticket.trim().to_owned();
        if trimmed.is_empty() {
            return Err("Paste a simscoop1... invite ticket first.".into());
        }
        sims_net::InviteTicket::decode_string(&trimmed)
            .map_err(|e| format!("Invalid invite ticket: {e}"))?;
        if !matches!(self.screen, ActiveScreen::Lot(_)) {
            let house = self.default_coop_home_lot();
            self.enter_lot(house)?;
        }
        let ActiveScreen::Lot(session) = &mut self.screen else {
            return Err("could not open lot shell to join co-op".into());
        };
        session.game.open_coop_lobby(Some(LobbyTab::Join));
        session.game.coop_start_join_async(trimmed, None)
    }

    /// Checks if the active lot's co-op session parked a verified `"SCOP"` v2
    /// snapshot for a different `(host_house, host_family)` than the currently
    /// loaded guest lot, re-stages the matching local lot seamlessly, and
    /// completes verified restore + host ACK. Returns `true` if handled.
    pub fn pump_coop_restage(&mut self) -> bool {
        let Some((host_house, host_family)) = (match &self.screen {
            ActiveScreen::Lot(session) => session.game.pending_coop_restage(),
            _ => None,
        }) else {
            return false;
        };
        let (mut coop, lobby_open) = {
            let ActiveScreen::Lot(session) = &mut self.screen else {
                return false;
            };
            let was_open = session.game.coop_lobby_open();
            (session.game.take_coop_controller(), was_open)
        };

        // Try ordered candidate sources (REDB store first, then installed
        // `HouseNN.iff`) so an `OtherLoad` mismatch on the store copy falls
        // back seamlessly to the matching installed base house before failing.
        let mut candidates: Vec<Game> = Vec::new();
        let store_matches = self
            .stored
            .family_in(host_house)
            .is_some_and(|(fid, _)| u32::from(fid) == host_family);
        let install_assets = self.ensure_install_assets().ok();
        if store_matches
            && let Ok(game) = Game::open_from_store_cached(
                &self.install_root,
                &self.store,
                &self.neighborhood_id,
                host_house,
                &self.answers,
                self.play,
                self.frames.clone(),
                install_assets.as_ref(),
            )
        {
            candidates.push(game);
        }
        let base_house = self
            .install_root
            .join("UserData")
            .join("Houses")
            .join(format!("House{host_house:02}.iff"));
        if base_house.is_file()
            && let Ok(game) = Game::open(&crate::game::Options {
                install: self.install_root.clone(),
                house: base_house,
                answers: self.answers.clone(),
                play: self.play,
                frames: self.frames.clone(),
            })
        {
            candidates.push(game);
        }

        let mut last_err = format!(
            "Host is playing Lot {host_house} (family {host_family}), which is not available \
             in this neighborhood or install."
        );
        for mut new_game in candidates {
            self.load_saved_options(&mut new_game);
            new_game.coop_set_player_name(self.coop_player_name.clone());
            new_game.coop_set_iroh_config(self.coop_iroh_config);
            new_game.set_album_button_enabled(true);
            new_game.set_neighborhood_slot(self.slot);
            new_game.set_special_lot_catalog(self.build_special_lot_catalog());
            new_game.set_debug(self.debug);
            new_game.resize(self.size);
            new_game.install_coop_controller(coop, true);
            if lobby_open {
                new_game.open_coop_lobby(Some(LobbyTab::Join));
            }
            match new_game.try_complete_pending_coop_restage() {
                Ok(()) => {
                    let initial_days_played =
                        new_game.lot().world().globals.get(28).copied().unwrap_or(0);
                    if let ActiveScreen::Lot(session) = &mut self.screen {
                        session.game = new_game;
                        session.house = host_house;
                        session.last_days_played = initial_days_played;
                    }
                    self.lot_generation = self.lot_generation.wrapping_add(1);
                    return true;
                }
                Err(crate::coop::RestageAttemptError::Mismatch(err)) => {
                    // Snapshot remains parked; take controller back and try next candidate.
                    coop = new_game.take_coop_controller();
                    last_err = err;
                }
                Err(crate::coop::RestageAttemptError::Fatal(err)) => {
                    let failed_coop = new_game.take_coop_controller();
                    if let ActiveScreen::Lot(session) = &mut self.screen {
                        session.game.install_coop_controller(failed_coop, false);
                        session.game.set_note(err);
                        if lobby_open {
                            session.game.open_coop_lobby(Some(LobbyTab::Join));
                        }
                    }
                    return true;
                }
            }
        }

        if let ActiveScreen::Lot(session) = &mut self.screen {
            coop.fail_pending_restage(session.game.lot_mut(), last_err.clone());
            session.game.install_coop_controller(coop, false);
            session.game.set_note(last_err);
            if lobby_open {
                session.game.open_coop_lobby(Some(LobbyTab::Join));
            }
        }
        true
    }

    pub(super) fn shell_coop_snapshot(&self) -> CoopSnapshot {
        CoopSnapshot {
            role: sims_ui::coop::CoopRole::Offline,
            connection: match &self.coop_error {
                Some(err) => sims_ui::coop::ConnectionState::Error(err.clone()),
                None => sims_ui::coop::ConnectionState::Disconnected,
            },
            players: Vec::new(),
            host_only_time_control: false,
            pause_reasons: Vec::new(),
            time_toast: None,
            resync: None,
        }
    }

    pub(super) fn handle_shell_coop_command(&mut self, cmd: CoopCommand) -> ShellOutcome {
        match cmd {
            CoopCommand::CloseDialog => {
                self.coop_lobby.close();
                self.coop_error = None;
                ShellOutcome::redraw()
            }
            CoopCommand::StartHosting => {
                if let Err(err) = self.coop_host_current_or_default() {
                    self.coop_error = Some(err);
                } else {
                    self.coop_lobby.close();
                    self.coop_error = None;
                }
                ShellOutcome::redraw()
            }
            CoopCommand::JoinSession { ticket } => {
                if let Err(err) = self.coop_join_from_shell(ticket) {
                    self.coop_error = Some(err);
                } else {
                    self.coop_lobby.close();
                    self.coop_error = None;
                }
                ShellOutcome::redraw()
            }
            CoopCommand::CopyInviteTicket(ticket) => {
                let _ = crate::coop::clipboard_copy(&ticket);
                self.note = Some("Copied co-op invite ticket to clipboard.".into());
                ShellOutcome::redraw()
            }
            CoopCommand::Disconnect | CoopCommand::CancelConnect => {
                self.coop_error = None;
                ShellOutcome::redraw()
            }
            CoopCommand::SetHostOnlyTimeControl(_) => ShellOutcome::redraw(),
        }
    }
}
