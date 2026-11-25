//! Travel: destinations, departures, returns and trips a script asks for.

use super::*;

impl NeighborhoodShell {
    // =========================================================================
    // Travel Destinations Shell (`TravelChrome`, `TravelDialog`, `sims_vm::travel`)
    // =========================================================================

    pub(super) fn sync_travel_state(&mut self) {
        if let Some(dest) = destination_for_kind(self.kind) {
            self.travel_state.destination = dest;
        }
        self.travel_state.mode = match self.nbhd_state.mode {
            NeighborhoodMode::Bulldoze => TravelBarMode::Bulldoze,
            NeighborhoodMode::Visit => TravelBarMode::Visit,
            // `cWinMagicland` holds Move In down while a family is placed
            // (`0x55478a`).
            NeighborhoodMode::MoveIn { .. } => TravelBarMode::MoveIn,
            _ => TravelBarMode::Browse,
        };
        self.travel_state.expansions = self.nbhd_state.expansions;
    }

    /// Opens the credits book with the map's credits music. Every map
    /// screen's logo handler switches music before showing credits:
    /// `bkground_credits` on the neighborhood (`0x559397`, `0x56099a`),
    /// Downtown (`0x54b811`), Studio Town (`0x565b90`) and Vacation Island
    /// (`0x56a545`); `music_magictown_credits` on Magic Town (`0x552f5d`).
    pub(super) fn open_credits_with_music(&mut self) {
        let mode = match self.kind {
            NeighborhoodKind::MagicTown => MusicMode::MagicTownCredits,
            _ => MusicMode::Credits,
        };
        self.audio_queue.push(Input::MusicMode(mode));
        self.open_credits_modal();
    }

    pub(super) fn travel_filtered_lot_ids(&self) -> BTreeSet<u16> {
        let filter_mask = self
            .travel_state
            .active_filter
            .map(|f| community_filter_for_lot_filter(f) as u16);
        let Some(mask) = filter_mask else {
            return BTreeSet::new();
        };
        self.map
            .lots
            .iter()
            .filter(|lot| (lot.filter_mask & mask) != 0)
            .map(|lot| lot.id)
            .collect()
    }

    /// Handles a [`TravelCommand`] emitted by [`TravelChrome`] on expansion
    /// sub-neighborhood maps.
    pub fn handle_travel_command(&mut self, cmd: TravelCommand) -> ShellOutcome {
        match cmd {
            TravelCommand::ReturnToNeighborhood => {
                let home = default_map_kind(&self.install, self.slot);
                if let Err(e) = self.switch_kind(home) {
                    self.note = Some(format!("Could not return to neighborhood map: {e}"));
                }
                ShellOutcome::redraw()
            }
            TravelCommand::ToggleBulldoze => {
                if self.travel_state.mode == TravelBarMode::Bulldoze {
                    self.travel_state.mode = TravelBarMode::Browse;
                    self.nbhd_state.mode = NeighborhoodMode::Normal;
                } else {
                    self.travel_state.mode = TravelBarMode::Bulldoze;
                    self.nbhd_state.mode = NeighborhoodMode::Bulldoze;
                }
                self.refresh_hovered_lot();
                ShellOutcome::redraw()
            }
            TravelCommand::ImportLot => {
                self.note = Some("Community lot import is not ported yet.".into());
                ShellOutcome::redraw()
            }
            TravelCommand::SetFilter(filter) => {
                self.travel_state.active_filter = filter;
                ShellOutcome::redraw()
            }
            // `kNghInet` and `kDowntownCredits` (the Sims logo) behave as on
            // the neighborhood bar; Magic Town's `kNghMoveIn` opens the
            // family bin like the neighborhood's Move In button.
            TravelCommand::OpenWeb => {
                self.handle_neighborhood_command(NeighborhoodCommand::OpenWeb)
            }
            TravelCommand::OpenCredits => {
                self.handle_neighborhood_command(NeighborhoodCommand::OpenCredits)
            }
            TravelCommand::OpenFamilyBin => {
                // The native resets the placement mode before reopening the
                // bin (`0x552d96`).
                self.nbhd_state.mode = NeighborhoodMode::Normal;
                self.move_in_family_id = None;
                self.sync_travel_state();
                self.open_family_bin();
                ShellOutcome::redraw()
            }
        }
    }

    /// Opens an unoccupied travel destination community lot in Build/Buy edit
    /// mode via [`vm_travel::open_destination_for_edit`]. Edits persist via
    /// [`Self::save_current_lot`] / [`Self::leave_lot`].
    pub fn open_destination_for_edit(&mut self, destination_house: u32) -> Result<(), String> {
        self.start_load_audio();
        self.present_loading_step(
            loading_kind_for_house(destination_house),
            Some(format!("Destination Lot {destination_house}")),
            0.2,
        );
        let snapshot = match self
            .store
            .load_lot_snapshot(&self.neighborhood_id, destination_house)
            .map_err(|e| e.to_string())
            .and_then(|opt| {
                opt.ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))
            }) {
            Ok(s) => s,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        let lot = match vm_travel::open_destination_for_edit(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            destination_house,
            &self.answers,
            self.frames.clone(),
        ) {
            Ok(lot) => lot,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err.to_string());
            }
        };
        let install_assets = self.ensure_install_assets().ok();
        let mut game = match Game::from_loaded_lot_cached(
            &self.install_root,
            lot,
            Vec::new(),
            &snapshot.character_files,
            snapshot.base_house.as_deref(),
            self.play,
            install_assets.as_ref(),
        ) {
            Ok(game) => game,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        if self.options_file.is_file() {
            let _ = game.load_options(&self.options_file);
        }
        game.coop_set_player_name(self.coop_player_name.clone());
        game.coop_set_iroh_config(self.coop_iroh_config);
        game.coop_set_travel_blocked(true);
        game.set_album_button_enabled(true);
        game.set_neighborhood_slot(self.slot);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());
        self.stored = snapshot.neighborhood;
        self.sync_neighborhood_iff();
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: self.neighborhood_id.clone(),
            neighborhood_instance_id: self.stored.instance_id,
            neighborhood_revision: self.stored.revision,
            staging: None,
            house: destination_house,
            last_days_played: initial_days_played,
            active_trip: None,
            is_destination_edit: true,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.dialog = None;
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = Some(destination_house);
        self.note = None;
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
        self.pressed_lot_id = None;
        self.nbhd_state.hovered_lot = None;
        self.stop_load_audio(next_music);
        Ok(())
    }

    /// Opens [`TravelDialog`] from the currently open home lot using the first
    /// eligible Complete Collection travel destination (defaulting to Downtown,
    /// falling back across Vacation Island, Old Town Community, Studio Town,
    /// and Magic Town). Desktop users can press `Left`/`Right`/`Tab` inside the
    /// open dialog to cycle destinations before confirming.
    pub fn open_default_travel_dialog(&mut self) -> Result<ValidatedTravelPlan, String> {
        let mut last_err = "no travel destinations available".to_owned();
        for dest in [
            TravelDestination::Downtown,
            TravelDestination::VacationIsland,
            TravelDestination::OldTown,
            TravelDestination::StudioTown,
            TravelDestination::MagicTown,
        ] {
            match self.open_travel_dialog_for_destination(dest) {
                Ok(plan) => return Ok(plan),
                Err(err) => last_err = err,
            }
        }
        Err(last_err)
    }

    /// Opens [`TravelDialog`] for `destination` from the currently open home lot,
    /// selecting the first available community lot for `destination` and the
    /// appropriate traveling party (selected Sim or whole household).
    pub fn open_travel_dialog_for_destination(
        &mut self,
        destination: TravelDestination,
    ) -> Result<ValidatedTravelPlan, String> {
        let dest_house = self
            .default_house_for_destination(destination)
            .ok_or_else(|| format!("no community lot found for {destination:?}"))?;
        let leader_guid = {
            let ActiveScreen::Lot(session) = &self.screen else {
                return Err("enter a home lot before traveling".into());
            };
            let world = session.game.lot().world();
            let home_family_id = self
                .stored
                .family_in(session.house)
                .map(|(id, _)| id)
                .or_else(|| {
                    u16::try_from(world.globals.get(9).copied().unwrap_or(0))
                        .ok()
                        .filter(|&id| id != 0)
                })
                .ok_or_else(|| format!("lot {} has no resident household", session.house))?;
            let fam_members: Vec<u32> = self
                .stored
                .families
                .get(&home_family_id)
                .map(|f| f.members.clone())
                .unwrap_or_default();
            let selected_guid = world
                .selected()
                .and_then(|sel| world.objects.get(&sel))
                .map(|o| o.guid)
                .filter(|g| fam_members.contains(g));
            selected_guid
                .or_else(|| fam_members.first().copied())
                .ok_or_else(|| format!("household {home_family_id} has no living members"))?
        };
        let party = match destination {
            TravelDestination::VacationIsland
            | TravelDestination::OldTown
            | TravelDestination::MagicTown => vm_travel::TravelParty::WholeFamily { leader_guid },
            TravelDestination::Downtown | TravelDestination::StudioTown => {
                vm_travel::TravelParty::Single {
                    sim_guid: leader_guid,
                }
            }
        };
        let request = TravelRequest {
            destination_house: dest_house,
            party,
            transport: None,
            accommodation: vm_travel::VacationAccommodation::None,
            fare_override: None,
            answers: self.answers.clone(),
            frames: self.frames.clone(),
            view_centre: None,
        };
        self.open_travel_dialog(request)
    }

    /// Cycles the open [`TravelDialog`] to the next (`delta > 0`) or previous
    /// (`delta < 0`) eligible destination without leaving the dialog.
    pub fn cycle_travel_destination(&mut self, delta: i32) -> bool {
        let Some(current) = self.pending_travel.as_ref().map(|p| p.plan.destination) else {
            return false;
        };
        const ORDER: [TravelDestination; 5] = [
            TravelDestination::Downtown,
            TravelDestination::VacationIsland,
            TravelDestination::OldTown,
            TravelDestination::StudioTown,
            TravelDestination::MagicTown,
        ];
        let start = ORDER.iter().position(|&d| d == current).unwrap_or(0);
        let step: isize = if delta >= 0 { 1 } else { -1 };
        for offset in 1..ORDER.len() {
            let idx =
                (start as isize + step * offset as isize).rem_euclid(ORDER.len() as isize) as usize;
            if self.open_travel_dialog_for_destination(ORDER[idx]).is_ok() {
                return true;
            }
        }
        false
    }

    pub(super) fn default_house_for_destination(&self, dest: TravelDestination) -> Option<u32> {
        let candidates: std::ops::RangeInclusive<u32> = match dest {
            TravelDestination::Downtown => 21..=30,
            TravelDestination::VacationIsland => 40..=48,
            TravelDestination::OldTown => 50..=79,
            TravelDestination::StudioTown => 81..=89,
            TravelDestination::MagicTown => 93..=98,
        };
        candidates.into_iter().find(|&house| {
            TravelDestination::classify(house, &self.stored.zoning) == Some(dest)
                && self
                    .store
                    .load_lot_snapshot(&self.neighborhood_id, house)
                    .ok()
                    .flatten()
                    .is_some_and(|s| s.base_house.is_some() || s.saved_lot.is_some())
        })
    }

    /// Validates `request` against the currently open home lot via
    /// [`vm_travel::validate_travel_plan`] and opens [`TravelDialog`] for
    /// player confirmation.
    pub fn open_travel_dialog(
        &mut self,
        request: TravelRequest,
    ) -> Result<ValidatedTravelPlan, String> {
        let (plan, travelers, simoleons, magic_coins) = {
            let ActiveScreen::Lot(session) = &self.screen else {
                return Err("enter a home lot before traveling".into());
            };
            if session.game.coop_active() {
                return Err("Leave the co-op session before traveling to a destination.".into());
            }
            if session.active_trip.is_some() {
                return Err("household is already visiting a travel destination".into());
            }
            let world = session.game.lot().world();
            let home_family_id = self
                .stored
                .family_in(session.house)
                .map(|(id, _)| id)
                .or_else(|| {
                    u16::try_from(world.globals.get(9).copied().unwrap_or(0))
                        .ok()
                        .filter(|&id| id != 0)
                })
                .ok_or_else(|| format!("lot {} has no resident household", session.house))?;
            let plan = vm_travel::validate_travel_plan(
                world,
                session.house,
                home_family_id,
                u16::from(self.install.expansions.0),
                &self.stored,
                &request,
            )
            .map_err(|e| e.to_string())?;
            let travelers: Vec<String> = plan
                .members
                .iter()
                .map(|t| {
                    self.faces
                        .name(t.guid)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("Sim 0x{:08X}", t.guid))
                })
                .collect();
            let simoleons = sims_vm::available_funds(world).max(0);
            let members: Vec<u32> = self
                .stored
                .families
                .get(&home_family_id)
                .map(|f| f.members.clone())
                .unwrap_or_default();
            let magic_coins = vm_travel::family_magic_coins(world, &members).max(0);
            (plan, travelers, simoleons, magic_coins)
        };
        let ui_dest = ui_destination_for_vm(plan.destination);
        let mut td_state = TravelDialogState::for_destination(ui_dest, travelers);
        td_state.transport = ui_transport_for_vm(plan.transport);
        td_state.fare_simoleons = plan.fare;
        td_state.household_simoleons = simoleons;
        td_state.household_magic_coins = magic_coins;
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.close_camera_strip();
        }
        self.travel_dialog = TravelDialog::new();
        self.travel_dialog_state = Some(td_state);
        self.pending_travel = Some(PendingTravel {
            request,
            plan: plan.clone(),
        });
        Ok(plan)
    }

    /// Closes [`TravelDialog`] without departing.
    pub fn close_travel_dialog(&mut self) {
        self.travel_dialog_state = None;
        self.pending_travel = None;
    }

    pub(super) fn handle_travel_dialog_command(
        &mut self,
        cmd: TravelDialogCommand,
    ) -> ShellOutcome {
        match cmd {
            TravelDialogCommand::Cancel => {
                self.close_travel_dialog();
                ShellOutcome::redraw()
            }
            TravelDialogCommand::Confirm { .. } => {
                let pending = self.pending_travel.take();
                self.travel_dialog_state = None;
                if let Some(pending) = pending {
                    let _ = &pending.plan;
                    if let Err(err) = self.depart_to_destination(pending.request) {
                        if let ActiveScreen::Lot(session) = &mut self.screen {
                            session.game.set_note(format!("Travel failed: {err}"));
                        } else if self.note.is_none() {
                            self.note = Some(format!("Travel failed: {err}"));
                        }
                    }
                }
                ShellOutcome::redraw()
            }
        }
    }

    /// Builds real neighborhood / destination lot entries (names, street
    /// addresses, descriptions, and zoning) from the live neighborhood store
    /// and installed `*Desc.iff` / `StreetNames.iff` tables for VM special lot
    /// pickers (`Dialog` kinds `5, 7, 8, 11, 13`).
    pub(super) fn build_special_lot_catalog(&self) -> Vec<SpecialLotEntry> {
        let mut lot_ids: BTreeSet<u16> = BTreeSet::new();
        // Single key-scan over stored houses (no character-file copying), so
        // only lots that actually exist in the neighborhood store are offered
        // (never unbacked map metadata entries).
        if let Ok(stored_houses) = self.store.list_houses(&self.neighborhood_id) {
            for house in stored_houses {
                if let Ok(id) = u16::try_from(house) {
                    lot_ids.insert(id);
                }
            }
        }
        lot_ids
            .into_iter()
            .map(|lot_id| {
                let (name, description) = if let Some(meta) = self.lot_meta.get(&lot_id) {
                    (meta.lot_name.clone(), meta.lot_description.clone())
                } else {
                    self.descriptions.lot_info(lot_id)
                };
                let address = self.street_names.address_for(lot_id);
                let zoning =
                    self.stored
                        .zoning
                        .get(&(lot_id as i16))
                        .copied()
                        .unwrap_or(match lot_id {
                            21..=30 | 40..=48 | 81..=89 | 93..=98 => 2,
                            _ => 1,
                        }) as i16;
                SpecialLotEntry {
                    lot_id,
                    name,
                    address,
                    description,
                    zoning,
                }
            })
            .collect()
    }

    /// Carries out the trip a script on the open lot asked for with Generic
    /// Sims Call 17 ([`Game::trip_request`]), as the original's main loop
    /// does when it finds the request between ticks. True if there was a
    /// request to carry out or refuse.
    ///
    /// - **From home** (the phone's Go Downtown, the Magic Hole...): the
    ///   departure is [`vm_travel::depart_from_request_with`]. Routine 17
    ///   already wrote the travelers' tokens and the script paid the fare.
    ///   The lot picker the script asked with recorded the same destination
    ///   ([`Game::take_pending_travel_lot`]) and gives the transport
    ///   ([`picker_transport`]), which the arrival places the travelers by.
    ///   The shell's answers and frames boot the destination, as for a trip
    ///   it plans itself.
    /// - **Home from a destination** (a destination script's Go Home, which
    ///   asks for house -1, resolved by 0x45a22d to the family's house): the
    ///   return is [`vm_travel::return_from_request`].
    ///
    /// A trip the shell cannot make (co-op, a trip from a destination to
    /// another, editing a destination, a house that is no destination, or a
    /// departure or return the VM refuses) shows a notice and is declined
    /// ([`Game::decline_trip`]): the lot drops the request and goes on.
    pub fn pump_trip_request(&mut self) -> bool {
        let ActiveScreen::Lot(session) = &mut self.screen else {
            return false;
        };
        if session.game.asking().is_some() || session.game.asking_special_panel().is_some() {
            return false;
        }
        let Some(trip) = session.game.trip_request().cloned() else {
            return false;
        };
        let house = u32::try_from(trip.house).unwrap_or(0);
        // The lot pick the script asked with only says how the trip goes, and
        // only when the trip goes to the lot picked. The pick is dropped
        // either way, so a stale one never colours a later trip.
        let transport = session
            .game
            .take_pending_travel_lot()
            .filter(|&(_, lot)| lot == house)
            .and_then(|(picker, _)| picker_transport(&picker));
        let home = session.active_trip.as_ref().map(|t| t.home_house);
        let refusal = if session.game.coop_active() {
            Some("Leave the co-op session before traveling to a destination.".to_owned())
        } else if home.is_some_and(|home| home != house) {
            Some("Return home before starting another destination trip.".to_owned())
        } else if session.is_destination_edit {
            Some("Travel is unavailable while editing a destination lot.".to_owned())
        } else if home.is_none()
            && TravelDestination::classify(house, &self.stored.zoning).is_none()
        {
            Some(format!("Lot {} is not a travel destination.", trip.house))
        } else {
            None
        };
        if let Some(msg) = refusal {
            session.game.decline_trip();
            report_travel_refusal(&mut self.screen, msg);
            return true;
        }
        let failed = if home.is_some() {
            self.return_home(true)
                .err()
                .map(|err| format!("Could not return home to lot {house}: {err}"))
        } else {
            self.depart(Departure::Trip { house, transport })
                .err()
                .map(|err| format!("Could not travel to lot {house}: {err}"))
        };
        if let Some(msg) = failed
            && let ActiveScreen::Lot(session) = &mut self.screen
        {
            session.game.decline_trip();
            report_travel_refusal(&mut self.screen, msg);
        }
        true
    }

    /// Departs from the currently open home lot to a travel destination via
    /// [`vm_travel::depart_to_destination`], driving [`LoadingScreen`] audio
    /// cues and wrapping the booted destination lot into the active session.
    pub fn depart_to_destination(
        &mut self,
        request: TravelRequest,
    ) -> Result<TravelSession, String> {
        self.depart(Departure::Request(request))
    }

    /// Departs from the open home lot ([`Departure`]), driving
    /// [`LoadingScreen`] audio cues and wrapping the booted destination lot
    /// into the active session.
    pub(super) fn depart(&mut self, source: Departure) -> Result<TravelSession, String> {
        let (destination_house, transport) = match &source {
            Departure::Request(request) => (request.destination_house, request.transport),
            Departure::Trip { house, transport } => (*house, *transport),
        };
        self.note = None;
        self.flush_background_saves();
        let nh_id = match &self.screen {
            ActiveScreen::Lot(session) => {
                if session.game.coop_active() {
                    return Err("Leave the co-op session before traveling to a destination.".into());
                }
                if session.active_trip.is_some() {
                    return Err("already on an active travel trip".into());
                }
                session.neighborhood_id.clone()
            }
            _ => return Err("no home lot is open for travel departure".into()),
        };
        // Pre-validate snapshot availability BEFORE committing `TRVL` to REDB.
        let _ = self
            .store
            .load_lot_snapshot(&nh_id, destination_house)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {nh_id:?} not found before departure"))?;

        self.start_load_audio();
        let load_kind = loading_kind_for_travel(destination_house, transport);
        self.present_loading_step(
            load_kind,
            Some(format!("Destination Lot {destination_house}")),
            0.2,
        );
        let departure = {
            let ActiveScreen::Lot(session) = &mut self.screen else {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err("no home lot is open for travel departure".into());
            };
            let departed = match &source {
                Departure::Request(request) => vm_travel::depart_to_destination(
                    &self.install_root,
                    &self.store,
                    &nh_id,
                    session.game.lot_mut(),
                    request,
                ),
                Departure::Trip { transport, .. } => vm_travel::depart_from_request_with(
                    &self.install_root,
                    &self.store,
                    &nh_id,
                    session.game.lot_mut(),
                    &vm_travel::TripDeparture {
                        transport: *transport,
                        answers: self.answers.clone(),
                        frames: self.frames.clone(),
                        view_centre: None,
                    },
                ),
            };
            match departed {
                Ok(dep) => dep,
                Err(err) => {
                    let fallback = self.desired_music_mode();
                    self.stop_load_audio(fallback);
                    return Err(err.to_string());
                }
            }
        };
        self.present_loading_step(
            loading_kind_for_travel(departure.session.destination_house, transport),
            Some(format!(
                "Destination Lot {}",
                departure.session.destination_house
            )),
            0.75,
        );
        let trip = departure.session;
        let dest_house = trip.destination_house;
        let install_assets = self.ensure_install_assets().ok();
        let post_commit_res: Result<(Game, StoredNeighborhood), String> = (|| {
            let snapshot = self
                .store
                .load_lot_snapshot(&nh_id, dest_house)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("neighborhood {nh_id:?} not found after departure"))?;
            let members = snapshot
                .neighborhood
                .families
                .get(&trip.home_family)
                .map(|f| f.members.clone())
                .unwrap_or_default();
            let game = Game::from_loaded_lot_cached(
                &self.install_root,
                departure.destination_lot,
                members,
                &snapshot.character_files,
                snapshot.base_house.as_deref(),
                self.play,
                install_assets.as_ref(),
            )?;
            Ok((game, snapshot.neighborhood))
        })();
        let (mut game, next_stored) = match post_commit_res {
            Ok(ok) => ok,
            Err(err) => {
                // Departure already committed `TRVL` to REDB: never leave the
                // frozen home lot on screen with `active_trip == None`. Return
                // cleanly to the Neighborhood map so clicking `trip.home_house`
                // offers Resume or Abort.
                self.screen = ActiveScreen::Neighborhood;
                self.lot_generation = self.lot_generation.wrapping_add(1);
                self.close_photo_album();
                self.close_travel_dialog();
                self.save_manager.close();
                self.save_manager_snapshot = None;
                self.save_manager_target_lot = None;
                let _ = self.refresh_from_store();
                self.note = Some(format!(
                    "Trip departed (saved in REDB), but destination lot {dest_house} could not open: {err}. Click lot {} to Resume or Abort.",
                    trip.home_house
                ));
                self.audio_queue
                    .push(Input::Events(vec![SoundEvent::StopAll]));
                self.stop_load_audio(music_mode_for_kind(self.kind));
                return Err(err);
            }
        };
        if self.options_file.is_file() {
            let _ = game.load_options(&self.options_file);
        }
        game.coop_set_player_name(self.coop_player_name.clone());
        game.coop_set_iroh_config(self.coop_iroh_config);
        game.coop_set_travel_blocked(true);
        game.set_album_button_enabled(true);
        game.set_neighborhood_slot(self.slot);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());
        self.stored = next_stored;
        self.sync_neighborhood_iff();
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: nh_id,
            neighborhood_instance_id: self.stored.instance_id,
            neighborhood_revision: self.stored.revision,
            staging: None,
            house: dest_house,
            last_days_played: initial_days_played,
            active_trip: Some(trip.clone()),
            is_destination_edit: false,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = None;
        self.note = None;
        self.audio_queue
            .push(Input::Events(vec![SoundEvent::StopAll]));
        self.stop_load_audio(next_music);
        Ok(trip)
    }

    /// Returns the traveling household from the active destination lot back to
    /// their reconciled home lot via [`vm_travel::return_from_destination`].
    pub fn return_from_destination(&mut self) -> Result<ReturnReport, String> {
        self.return_home(false)
    }

    /// Returns home from the active destination lot: through the trip home
    /// a script asked for ([`vm_travel::return_from_request`]) when
    /// `requested`, else as the shell's own Go Home
    /// ([`vm_travel::return_from_destination`]).
    pub(super) fn return_home(&mut self, requested: bool) -> Result<ReturnReport, String> {
        self.note = None;
        let (nh_id, trip) = match &self.screen {
            ActiveScreen::Lot(session) => {
                let Some(trip) = session.active_trip.clone() else {
                    return Err("current lot is not an active travel trip".into());
                };
                (session.neighborhood_id.clone(), trip)
            }
            _ => return Err("no active travel destination lot is open".into()),
        };
        // Pre-validate home snapshot availability BEFORE committing return.
        let _ = self
            .store
            .load_lot_snapshot(&nh_id, trip.home_house)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {nh_id:?} not found before return"))?;

        self.start_load_audio();
        self.present_loading_step(
            LoadingKind::Lot,
            Some(format!("Home Lot {}", trip.home_house)),
            0.2,
        );
        let ret = {
            let ActiveScreen::Lot(session) = &mut self.screen else {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err("no active travel destination lot is open".into());
            };
            let returned = if requested {
                vm_travel::return_from_request(
                    &self.install_root,
                    &self.store,
                    &trip,
                    session.game.lot_mut(),
                    &self.answers,
                    self.frames.clone(),
                )
            } else {
                vm_travel::return_from_destination(
                    &self.install_root,
                    &self.store,
                    &trip,
                    session.game.lot_mut(),
                    &self.answers,
                    self.frames.clone(),
                )
            };
            match returned {
                Ok(ret) => ret,
                Err(err) => {
                    let fallback = self.desired_music_mode();
                    self.stop_load_audio(fallback);
                    return Err(err.to_string());
                }
            }
        };
        let home_house = ret.report.home_house;
        self.present_loading_step(
            LoadingKind::Lot,
            Some(format!("Home Lot {home_house}")),
            0.75,
        );
        let install_assets = self.ensure_install_assets().ok();
        let post_commit_res: Result<(Game, StoredNeighborhood), String> = (|| {
            let snapshot = self
                .store
                .load_lot_snapshot(&nh_id, home_house)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("neighborhood {nh_id:?} not found after return"))?;
            let members = snapshot
                .neighborhood
                .family_in(home_house)
                .map(|(_, f)| f.members.clone())
                .unwrap_or_default();
            let game = Game::from_loaded_lot_cached(
                &self.install_root,
                ret.home_lot,
                members,
                &snapshot.character_files,
                snapshot.base_house.as_deref(),
                self.play,
                install_assets.as_ref(),
            )?;
            Ok((game, snapshot.neighborhood))
        })();
        let (mut game, next_stored) = match post_commit_res {
            Ok(ok) => ok,
            Err(err) => {
                // Return already reconciled the home lot and consumed `TRVL` in
                // REDB: never leave the consumed destination lot on screen with
                // a stale `active_trip`. Transition cleanly to Neighborhood so
                // the player can immediately enter their saved home lot.
                self.screen = ActiveScreen::Neighborhood;
                self.lot_generation = self.lot_generation.wrapping_add(1);
                self.close_photo_album();
                self.close_travel_dialog();
                self.save_manager.close();
                self.save_manager_snapshot = None;
                self.save_manager_target_lot = Some(home_house);
                let _ = self.refresh_from_store();
                self.note = Some(format!(
                    "Returned home to lot {home_house} (saved in REDB), but failed to reopen home lot: {err}. Click lot {home_house} to enter."
                ));
                self.audio_queue
                    .push(Input::Events(vec![SoundEvent::StopAll]));
                self.stop_load_audio(music_mode_for_kind(self.kind));
                return Err(err);
            }
        };
        if self.options_file.is_file() {
            let _ = game.load_options(&self.options_file);
        }
        game.set_album_button_enabled(true);
        game.set_neighborhood_slot(self.slot);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());
        self.stored = next_stored;
        self.sync_neighborhood_iff();
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: nh_id,
            neighborhood_instance_id: self.stored.instance_id,
            neighborhood_revision: self.stored.revision,
            staging: None,
            house: home_house,
            last_days_played: initial_days_played,
            active_trip: None,
            is_destination_edit: false,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = Some(home_house);
        self.note = None;
        self.audio_queue
            .push(Input::Events(vec![SoundEvent::StopAll]));
        self.stop_load_audio(next_music);
        Ok(ret.report)
    }

    /// Inspects whether `home_house` in the active neighborhood has an active
    /// crash-persisted travel trip envelope (`TRVL`) in REDB.
    pub fn inspect_active_trip(&self, home_house: u32) -> Result<Option<TravelSession>, String> {
        vm_travel::inspect_active_trip(&self.store, &self.neighborhood_id, home_house)
            .map_err(|e| e.to_string())
    }

    /// Scans occupied residential lots in the active neighborhood for any
    /// crash-interrupted travel trips persisted in REDB.
    pub fn inspect_any_active_trips(&self) -> Vec<(u32, TravelSession)> {
        let mut out = Vec::new();
        for fam in self.stored.families.values() {
            if fam.house > 0
                && let Ok(house) = u32::try_from(fam.house)
                && let Ok(Some(session)) =
                    vm_travel::inspect_active_trip(&self.store, &self.neighborhood_id, house)
            {
                out.push((house, session));
            }
        }
        out
    }

    /// Resumes an interrupted active travel trip for `home_house` directly into
    /// its destination lot via [`vm_travel::resume_active_trip`].
    pub fn resume_active_trip(&mut self, home_house: u32) -> Result<TravelSession, String> {
        self.start_load_audio();
        self.present_loading_step(
            LoadingKind::Lot,
            Some(format!("Resuming Trip (Lot {home_house})")),
            0.2,
        );
        let resumed = match vm_travel::resume_active_trip(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            home_house,
            &self.answers,
            self.frames.clone(),
        ) {
            Ok(dep) => dep,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err.to_string());
            }
        };
        let trip = resumed.session;
        let dest_house = trip.destination_house;
        let snapshot = match self
            .store
            .load_lot_snapshot(&self.neighborhood_id, dest_house)
            .map_err(|e| e.to_string())
            .and_then(|opt| {
                opt.ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))
            }) {
            Ok(s) => s,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        let members = snapshot
            .neighborhood
            .families
            .get(&trip.home_family)
            .map(|f| f.members.clone())
            .unwrap_or_default();
        let install_assets = self.ensure_install_assets().ok();
        let mut game = match Game::from_loaded_lot_cached(
            &self.install_root,
            resumed.destination_lot,
            members,
            &snapshot.character_files,
            snapshot.base_house.as_deref(),
            self.play,
            install_assets.as_ref(),
        ) {
            Ok(game) => game,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        if self.options_file.is_file() {
            let _ = game.load_options(&self.options_file);
        }
        game.coop_set_player_name(self.coop_player_name.clone());
        game.coop_set_iroh_config(self.coop_iroh_config);
        game.coop_set_travel_blocked(true);
        game.set_album_button_enabled(true);
        game.set_neighborhood_slot(self.slot);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());
        self.stored = snapshot.neighborhood;
        self.sync_neighborhood_iff();
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: self.neighborhood_id.clone(),
            neighborhood_instance_id: self.stored.instance_id,
            neighborhood_revision: self.stored.revision,
            staging: None,
            house: dest_house,
            last_days_played: initial_days_played,
            active_trip: Some(trip.clone()),
            is_destination_edit: false,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.dialog = None;
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = None;
        self.note = None;
        self.stop_load_audio(next_music);
        Ok(trip)
    }

    /// Aborts a crash-interrupted travel trip for `home_house` via
    /// [`vm_travel::abort_active_trip`], restoring the frozen home lot bytes in
    /// REDB and refreshing the neighborhood view.
    pub fn abort_active_trip(&mut self, home_house: u32) -> Result<u32, String> {
        let restored = vm_travel::abort_active_trip(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            home_house,
            &self.answers,
            self.frames.clone(),
        )
        .map_err(|e| e.to_string())?;
        self.refresh_from_store()?;
        Ok(restored.report.home_house)
    }
}
