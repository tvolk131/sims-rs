//! Switching neighborhoods, the family bin and CAF, entering and leaving lots, move-in, eviction, bulldozing, and keeping the store and Neighborhood.iff in step.

use super::*;

impl NeighborhoodShell {
    /// Switches to neighborhood slot `1..=8`, importing it into REDB on first visit.
    pub fn switch_neighborhood(&mut self, slot: u8) -> Result<(), String> {
        let slot = slot.clamp(MIN_NEIGHBORHOOD, MAX_NEIGHBORHOOD);
        let kind = default_map_kind(&self.install, slot);
        let id = neighborhood_id_for_slot(slot);
        let mut stored = ensure_imported(&self.install, &self.store, &id)?;
        migrate_legacy_caf_sidecar_if_needed(&self.store, &self.saves_path, &id, &mut stored);
        let _ = self.store.set_active_neighborhood(&id);
        let mut map = load_map_for_slot(&self.install, slot, kind)?;
        let user_dir = resolve_user_dir(&self.install, slot)?;
        let family_names = load_family_names_with_caf(&user_dir, &stored);
        let street_names = StreetNames::load(&self.install, &user_dir);
        let descriptions = NeighborhoodDescriptions::load(&self.install, &user_dir);
        let (faces, base_cat) = load_neighborhood_faces_cached(
            &self.install,
            &self.store,
            &id,
            &stored,
            &self.pool,
            self.base_index.as_ref(),
            self.base_catalog.as_ref(),
        );
        if self.base_catalog.is_none() {
            self.base_catalog = base_cat;
        }
        let lot_meta = build_lot_meta(&self.store, &id, &map, &descriptions, self.arch_multiplier);
        let custom_thumbs = load_persisted_custom_thumbs(&self.store, &id);
        sync_map_with_store(&mut map, &stored, &lot_meta, &id, &custom_thumbs);
        let family_bin_entries = build_family_bin_entries(&stored, &family_names, &faces);

        self.slot = slot;
        self.neighborhood_id = id;
        self.kind = kind;
        self.map = map;
        self.neighborhood_iff_synced = Some((
            self.neighborhood_id.clone(),
            stored.instance_id,
            stored.revision,
        ));
        self.stored = stored;
        self.family_names = family_names;
        self.street_names = street_names;
        self.descriptions = descriptions;
        self.faces = faces;
        self.lot_meta = lot_meta;
        self.custom_thumbs = custom_thumbs;
        self.family_bin_entries = family_bin_entries;
        self.family_bin
            .sync(&self.family_bin_entries, sims_ui::family_bin::VISIBLE_ROWS);
        self.nbhd_state.neighborhood_number = slot;
        self.nbhd_state.mode = NeighborhoodMode::Normal;
        self.nbhd_state.hovered_lot = None;
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
        self.pressed_lot_id = None;
        self.move_in_family_id = None;
        self.dialog = None;
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = None;
        self.close_import_family_picker();
        self.drop_credits_modal();
        self.nbhd_state.show_credits_button = destination_for_kind(kind).is_none();
        self.refresh_importable_families();
        self.note = None;
        self.screen = ActiveScreen::Neighborhood;
        self.sync_travel_state();
        self.audio_queue
            .push(Input::MusicMode(music_mode_for_kind(kind)));
        Ok(())
    }

    /// Switches between the main neighborhood map and expansion sub-neighborhoods
    /// (Downtown, Vacation Island, Studio Town, Magic Town).
    pub fn switch_kind(&mut self, kind: NeighborhoodKind) -> Result<(), String> {
        let mut map = load_map_for_slot(&self.install, self.slot, kind)?;
        let lot_meta = build_lot_meta(
            &self.store,
            &self.neighborhood_id,
            &map,
            &self.descriptions,
            self.arch_multiplier,
        );
        self.custom_thumbs = load_persisted_custom_thumbs(&self.store, &self.neighborhood_id);
        sync_map_with_store(
            &mut map,
            &self.stored,
            &lot_meta,
            &self.neighborhood_id,
            &self.custom_thumbs,
        );
        self.kind = kind;
        self.map = map;
        self.lot_meta = lot_meta;
        self.nbhd_state.mode = NeighborhoodMode::Normal;
        self.nbhd_state.show_credits_button = destination_for_kind(kind).is_none();
        self.nbhd_state.hovered_lot = None;
        self.close_import_family_picker();
        self.drop_credits_modal();
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
        self.pressed_lot_id = None;
        self.move_in_family_id = None;
        self.travel_state.active_filter = None;
        self.travel_state.mode = TravelBarMode::Browse;
        self.travel_chrome.clear_pointer();
        self.sync_travel_state();
        self.audio_queue
            .push(Input::MusicMode(music_mode_for_kind(kind)));
        Ok(())
    }

    /// Opens the Family Bin ("Select a Family") overlay screen.
    pub fn open_family_bin(&mut self) {
        self.family_bin_entries =
            build_family_bin_entries(&self.stored, &self.family_names, &self.faces);
        self.family_bin.clear_pointer();
        self.family_bin
            .sync(&self.family_bin_entries, sims_ui::family_bin::VISIBLE_ROWS);
        self.dialog = None;
        self.note = None;
        self.screen = ActiveScreen::FamilyBin;
        self.audio_queue
            .push(Input::MusicMode(MusicMode::FamilyBin));
    }

    /// Closes the Family Bin and returns to the Neighborhood map.
    pub fn close_family_bin(&mut self) {
        self.dialog = None;
        self.screen = ActiveScreen::Neighborhood;
        self.audio_queue
            .push(Input::MusicMode(music_mode_for_kind(self.kind)));
    }

    /// Enters Move-In placement mode on the Neighborhood map for bin household `family_id`.
    pub fn start_move_in(&mut self, family_id: u16) -> Result<(), String> {
        let fam = self
            .stored
            .families
            .get(&family_id)
            .ok_or_else(|| format!("family {family_id} not found"))?;
        if fam.house != 0 || fam.secondary_house() != 0 || fam.members.is_empty() {
            return Err(format!("family {family_id} is not in the Family Bin"));
        }
        let family_name = self
            .family_names
            .get(&family_id)
            .cloned()
            .unwrap_or_else(|| format!("Family {family_id}"));
        let family_magic_coins = lifecycle::family_magic_coins(&self.stored, family_id);
        self.move_in_family_id = Some(family_id);
        self.nbhd_state.mode = NeighborhoodMode::MoveIn {
            family_name,
            family_funds: fam.budget,
            family_magic_coins,
        };
        self.screen = ActiveScreen::Neighborhood;
        self.dialog = None;
        self.sync_travel_state();
        self.refresh_hovered_lot();
        self.audio_queue
            .push(Input::MusicMode(music_mode_for_kind(self.kind)));
        Ok(())
    }

    /// Opens the interactive Create-a-Family (`CAF`) screen.
    pub fn open_create_family(&mut self) -> Result<(), String> {
        let ui = UiGraphics::open(&self.install).map_err(|e| e.to_string())?;
        let caf = CafSession::open(&self.install_root, &self.install, &ui)?;
        self.dialog = None;
        self.note = None;
        self.screen = ActiveScreen::CreateFamily(Box::new(caf));
        self.audio_queue
            .push(Input::MusicMode(MusicMode::FamilyBin));
        Ok(())
    }

    pub(super) fn handle_caf_outcome(&mut self, outcome: CafOutcome) -> ShellOutcome {
        match outcome {
            CafOutcome::Stay { redraw } => ShellOutcome {
                redraw,
                ..ShellOutcome::default()
            },
            CafOutcome::Cancelled => {
                self.open_family_bin();
                ShellOutcome::redraw()
            }
            CafOutcome::Commit(draft) => {
                let commit_res = match &mut self.screen {
                    ActiveScreen::CreateFamily(caf) => {
                        caf.commit_family_draft(&self.store, &self.neighborhood_id, &draft)
                    }
                    _ => Err("Create-a-Family screen is not active".into()),
                };
                match commit_res {
                    Ok(created) => {
                        self.family_names
                            .insert(created.family_id, created.family_name.clone());
                        if let Err(err) = self.refresh_from_store() {
                            self.note = Some(format!(
                                "Created family {} but refresh failed: {err}",
                                created.family_name
                            ));
                        }
                        self.open_family_bin();
                        // Pre-select the newly created household so the player can
                        // immediately click Move In.
                        if let Some(idx) = self
                            .family_bin_entries
                            .iter()
                            .position(|e| e.id == u32::from(created.family_id))
                        {
                            self.family_bin.select(Some(idx), &self.family_bin_entries);
                        }
                        self.note = Some(format!(
                            "Created family {} ({} Sim{}). Ready to Move In!",
                            created.family_name,
                            created.members.len(),
                            if created.members.len() == 1 { "" } else { "s" }
                        ));
                        ShellOutcome::redraw()
                    }
                    Err(err) => {
                        if let ActiveScreen::CreateFamily(caf) = &mut self.screen {
                            caf.set_note(format!("Could not create family: {err}"));
                        } else {
                            self.note = Some(format!("Could not create family: {err}"));
                        }
                        ShellOutcome::redraw()
                    }
                }
            }
        }
    }

    /// Enters `house` in the active neighborhood via [`lifecycle::enter_lot`],
    /// automatically resuming an active travel trip if crash-recovery detects a
    /// persisted `TRVL` envelope on `house`, and driving [`LoadingScreen`] audio
    /// cues across the transition.
    pub fn enter_lot(&mut self, house: u32) -> Result<(), String> {
        // Crash recovery: if `house` has an active persisted travel trip,
        // resume it into the destination lot instead of treating the `TRVL`
        // envelope as a raw `Saved` lot.
        if let Ok(Some(_)) =
            vm_travel::inspect_active_trip(&self.store, &self.neighborhood_id, house)
        {
            self.resume_active_trip(house)?;
            return Ok(());
        }
        self.start_load_audio();
        self.present_loading_step(LoadingKind::Lot, Some(format!("Lot {house}")), 0.2);
        // Capture the neighborhood version BEFORE booting the VM and confirm it
        // did not advance concurrently so `LotSession` never records a newer
        // `(instance_id, revision)` than the snapshot that actually booted.
        let before_nh = match self
            .store
            .load_neighborhood(&self.neighborhood_id)
            .map_err(|e| e.to_string())
            .and_then(|opt| {
                opt.ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))
            }) {
            Ok(nh) => nh,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        let install_assets = match self.ensure_install_assets() {
            Ok(assets) => assets,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        let mut game = match Game::open_from_store_cached(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            house,
            &self.answers,
            self.play,
            self.frames.clone(),
            Some(&install_assets),
        ) {
            Ok(game) => game,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        let after_nh = match self
            .store
            .load_neighborhood(&self.neighborhood_id)
            .map_err(|e| e.to_string())
            .and_then(|opt| {
                opt.ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))
            }) {
            Ok(nh) => nh,
            Err(err) => {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
        };
        if after_nh.instance_id != before_nh.instance_id || after_nh.revision != before_nh.revision
        {
            let fallback = self.desired_music_mode();
            self.stop_load_audio(fallback);
            return Err(format!(
                "neighborhood {:?} changed while opening lot {house}; please try again",
                self.neighborhood_id
            ));
        }
        self.stored = before_nh;
        self.load_saved_options(&mut game);
        game.coop_set_player_name(self.coop_player_name.clone());
        game.coop_set_iroh_config(self.coop_iroh_config);
        game.set_album_button_enabled(true);
        game.set_neighborhood_slot(self.slot);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: self.neighborhood_id.clone(),
            neighborhood_instance_id: self.stored.instance_id,
            neighborhood_revision: self.stored.revision,
            staging: None,
            house,
            last_days_played: initial_days_played,
            active_trip: None,
            is_destination_edit: false,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.dialog = None;
        self.coop_lobby.close();
        self.coop_error = None;
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.save_manager_target_lot = Some(house);
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

    /// Leaves the currently open lot back to the Neighborhood map, optionally
    /// saving through [`lifecycle::save_and_leave_lot`] + rotating background
    /// autosave first, cleanly draining the background worker, and refreshing
    /// map thumbnails, funds, occupancy, and the Family Bin.
    ///
    /// If the household is currently on an active travel destination visit,
    /// returns them cleanly via [`Self::return_from_destination`] first so
    /// community lot templates are never overwritten and the home lot is
    /// reconciled before returning to the neighborhood map.
    pub fn leave_lot(&mut self, save_first: bool) -> Result<(), String> {
        let mut effective_save_first = save_first;
        if let ActiveScreen::Lot(session) = &mut self.screen {
            if session.game.coop_blocks_local_save() {
                // Peers (and disconnected guest worlds) never overwrite local REDB slots.
                effective_save_first = false;
            }
            if session.game.coop_active() {
                session.game.leave_coop();
            }
        }
        if self.active_trip().is_some() {
            self.return_from_destination()?;
        }
        self.start_load_audio();
        self.present_loading_step(LoadingKind::Neighborhood, None, 0.2);
        let mut autosave_warning: Option<String> = None;
        if effective_save_first {
            if let Err(err) = self.save_current_lot() {
                let fallback = self.desired_music_mode();
                self.stop_load_audio(fallback);
                return Err(err);
            }
            if let Err(err) = self.enqueue_background_lot_save(SaveTarget::Autosave) {
                autosave_warning = Some(format!(
                    "Rotating autosave could not be queued (existing saves preserved): {err}"
                ));
            }
        }
        // Drain any prior completion noise before flushing this leave's jobs so
        // we accurately report failures from leaving the lot.
        let _ = self.take_save_completions();
        self.flush_background_saves();
        for completion in self.take_save_completions() {
            if let Err(err) = completion.result {
                autosave_warning = Some(format!(
                    "Background save warning (existing saves preserved): {err}"
                ));
            }
        }
        self.persist_options();
        self.screen = ActiveScreen::Neighborhood;
        self.nbhd_state.mode = NeighborhoodMode::Normal;
        self.nbhd_state.hovered_lot = None;
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
        self.pressed_lot_id = None;
        self.move_in_family_id = None;
        self.dialog = None;
        self.close_photo_album();
        self.close_travel_dialog();
        self.save_manager.close();
        self.save_manager_snapshot = None;
        self.note = autosave_warning;
        let refresh_res = self.refresh_from_store();
        self.audio_queue
            .push(Input::Events(vec![SoundEvent::StopAll]));
        self.stop_load_audio(music_mode_for_kind(self.kind));
        refresh_res?;
        Ok(())
    }

    /// Executes Move-In for `family_id` onto `house`, refreshes REDB state, and
    /// immediately enters the newly occupied lot. If opening the lot fails,
    /// rolls back the neighborhood commit so the household is not left charged
    /// and moved out of the bin.
    pub fn execute_move_in(&mut self, family_id: u16, house: u32) -> Result<MoveInReport, String> {
        let before = self
            .store
            .load_neighborhood(&self.neighborhood_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))?;
        let report = lifecycle::move_in(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            family_id,
            house,
        )
        .map_err(|e| e.to_string())?;
        match self.enter_lot(house) {
            Ok(()) => {
                self.nbhd_state.mode = NeighborhoodMode::Normal;
                self.move_in_family_id = None;
                let _ = self.refresh_from_store();
                Ok(report)
            }
            Err(enter_err) => {
                let rollback = self
                    .store
                    .save_neighborhood_checked(
                        &self.neighborhood_id,
                        report.committed_instance_id,
                        report.committed_revision,
                        &before,
                    )
                    .map_err(|e| e.to_string())
                    .and_then(|res| res.map(|_| ()).map_err(|e| format!("{e:?}")));
                match rollback {
                    Ok(()) => {
                        let _ = self.refresh_from_store();
                        Err(format!(
                            "could not open lot {house} (move-in rolled back): {enter_err}"
                        ))
                    }
                    Err(rb_err) => {
                        self.nbhd_state.mode = NeighborhoodMode::Normal;
                        self.move_in_family_id = None;
                        let _ = self.refresh_from_store();
                        Err(format!(
                            "family moved into lot {house}, but opening the lot failed \
                             (rollback failed: {rb_err}): {enter_err}"
                        ))
                    }
                }
            }
        }
    }

    /// Executes Evict for the household occupying `house`, returning them to
    /// the Family Bin and refreshing map occupancy and Family Bin immediately.
    pub fn execute_evict(&mut self, house: u32) -> Result<EvictReport, String> {
        let report = lifecycle::evict(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            house,
        )
        .map_err(|e| e.to_string())?;
        self.refresh_from_store()?;
        Ok(report)
    }

    /// Executes Bulldoze on `house`, deleting any persisted custom REDB map
    /// thumbnail and refreshing map occupancy, vacant lot thumbnail, and Family
    /// Bin immediately.
    pub fn execute_bulldoze(&mut self, house: u32) -> Result<BulldozeReport, String> {
        let report = lifecycle::bulldoze(
            &self.install_root,
            &self.store,
            &self.neighborhood_id,
            house,
        )
        .map_err(|e| e.to_string())?;
        let thumb_warning = match self.store.delete_lot_thumbnail_checked(
            &self.neighborhood_id,
            self.stored.instance_id,
            house,
        ) {
            Ok(_) => None,
            Err(err) => Some(format!("Lot bulldozed; thumbnail cleanup warning: {err}")),
        };
        if let Ok(lot_id) = u16::try_from(house) {
            self.custom_thumbs
                .remove(&(self.neighborhood_id.clone(), lot_id));
        }
        self.refresh_from_store()?;
        if let Some(warn) = thumb_warning {
            self.note = Some(warn);
        }
        Ok(report)
    }

    /// Permanently removes a Family Bin household from the active neighborhood
    /// in REDB via [`sims_vm::caf::delete_bin_family`] (vacating `NBRS` slots,
    /// scrubbing reciprocal relationships, and recording deletion tombstones)
    /// and refreshes the Family Bin list.
    pub fn execute_delete_family(&mut self, family_id: u16) -> Result<(), String> {
        sims_vm::caf::delete_bin_family(&self.store, &self.neighborhood_id, family_id)
            .map_err(|e| e.to_string())?;
        self.family_names.remove(&family_id);
        self.refresh_from_store()?;
        Ok(())
    }

    /// Reloads [`StoredNeighborhood`], lot metadata, map occupancy, and Family
    /// Bin entries from REDB.
    pub fn refresh_from_store(&mut self) -> Result<(), String> {
        self.stored = self
            .store
            .load_neighborhood(&self.neighborhood_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))?;
        migrate_legacy_caf_sidecar_if_needed(
            &self.store,
            &self.saves_path,
            &self.neighborhood_id,
            &mut self.stored,
        );
        if let Ok(user_dir) = resolve_user_dir(&self.install, self.slot) {
            self.family_names = load_family_names_with_caf(&user_dir, &self.stored);
        }
        let (faces, base_cat) = load_neighborhood_faces_cached(
            &self.install,
            &self.store,
            &self.neighborhood_id,
            &self.stored,
            &self.pool,
            self.base_index.as_ref(),
            self.base_catalog.as_ref(),
        );
        self.faces = faces;
        if self.base_catalog.is_none() {
            self.base_catalog = base_cat;
        }
        self.lot_meta = build_lot_meta(
            &self.store,
            &self.neighborhood_id,
            &self.map,
            &self.descriptions,
            self.arch_multiplier,
        );
        self.custom_thumbs = load_persisted_custom_thumbs(&self.store, &self.neighborhood_id);
        sync_map_with_store(
            &mut self.map,
            &self.stored,
            &self.lot_meta,
            &self.neighborhood_id,
            &self.custom_thumbs,
        );
        self.family_bin_entries =
            build_family_bin_entries(&self.stored, &self.family_names, &self.faces);
        self.family_bin
            .sync(&self.family_bin_entries, sims_ui::family_bin::VISIBLE_ROWS);
        self.refresh_importable_families();
        self.sync_travel_state();
        self.refresh_hovered_lot();
        self.sync_neighborhood_iff();
        Ok(())
    }

    /// Where the shell keeps the open neighborhood's `Neighborhood.iff`:
    /// `<saves file name>.userdata/<neighborhood id>/Neighborhood.iff`
    /// (`slot.redb.userdata/...`), beside the saves file, never in the
    /// install. The original saves it to its
    /// `UserData` folder whenever the neighborhood changes
    /// (cTSNeighborhood::Save, 0x4252cc).
    pub fn neighborhood_iff_path(&self) -> PathBuf {
        self.neighborhood_iff_dir().join("Neighborhood.iff")
    }

    pub(super) fn neighborhood_iff_dir(&self) -> PathBuf {
        // The whole file name, extension and all, so `slot.redb` and
        // `slot.backup` each have a folder of their own.
        user_files::userdata_dir(&self.saves_path).join(&self.neighborhood_id)
    }

    /// Writes the open neighborhood's committed record as `Neighborhood.iff`
    /// ([`NeighborhoodStore::save_neighborhood_iff`]) once a commit has
    /// changed it: a lot save, a trip, a family moved or evicted. The
    /// original saves it at those points (0x4252cc's callers: the lot save
    /// 0x428687 at 0x428819, the eviction 0x42737e at 0x4273e2, the trip at
    /// 0x45a818, the family screens). REDB stays the store: a write that
    /// fails is reported and does not undo the commit, and the next commit
    /// tries again.
    pub(super) fn sync_neighborhood_iff(&mut self) {
        let key = (
            self.neighborhood_id.clone(),
            self.stored.instance_id,
            self.stored.revision,
        );
        if self.neighborhood_iff_synced.as_ref() == Some(&key) {
            return;
        }
        let dir = self.neighborhood_iff_dir();
        let written = std::fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                self.store
                    .save_neighborhood_iff(&self.neighborhood_id, &dir)
                    .map_err(|e| e.to_string())
            });
        if let Err(err) = written {
            eprintln!(
                "sims-game: could not write {}: {err}",
                dir.join("Neighborhood.iff").display()
            );
        }
        self.neighborhood_iff_synced = Some(key);
    }

    pub(super) fn refresh_hovered_lot(&mut self) {
        self.nbhd_state.hovered_lot = self
            .hovered_lot_id
            .filter(|&id| self.hover_card_shown_for == Some(id))
            .and_then(|id| self.build_lot_info(id));
    }

    pub(super) fn magic_cost_for_lot(&self, lot_id: u16) -> i32 {
        match lot_id {
            91 => self.magic_lot_91_cost,
            90 | 92 => self.magic_lot_other_cost,
            _ => 0,
        }
    }

    pub(super) fn build_lot_info(&self, lot_id: u16) -> Option<LotInfo> {
        let map_lot = self.map.lot(lot_id)?;
        let origin = self.map.map_origin_for(self.frame_size_u32());
        let anchor = [
            origin[0].saturating_add(map_lot.anchor[0]),
            origin[1].saturating_add(map_lot.anchor[1]),
        ];
        let [left, top, right, bottom] = self.map.lot_screen_rect(map_lot, self.frame_size_u32());
        let meta = self.lot_meta.get(&lot_id);
        let is_magic_res = (90..=92).contains(&lot_id);
        let is_magic_comm = (93..=98).contains(&lot_id);
        let kind = if is_magic_res {
            LotKind::MagicResidential
        } else if is_magic_comm {
            LotKind::MagicCommunity
        } else if map_lot.zoning == LotZoning::Community {
            LotKind::Community
        } else {
            LotKind::Residential
        };

        let occupant = self
            .stored
            .family_in(u32::from(lot_id))
            .map(|(fam_id, fam)| {
                let family_name = self
                    .family_names
                    .get(&fam_id)
                    .cloned()
                    .unwrap_or_else(|| format!("Family {fam_id}"));
                // `0x550b02..0x550c53` draws each member's framed
                // thumbnail (`BMP_` 2004) one to one.
                let portraits = fam
                    .members
                    .iter()
                    .filter_map(|guid| {
                        self.faces
                            .thumbnail(*guid)
                            .or_else(|| self.faces.face(*guid))
                            .cloned()
                    })
                    .collect();
                let magic_coins = lifecycle::family_magic_coins(&self.stored, fam_id);
                LotOccupant {
                    family_name,
                    funds: fam.budget,
                    house_value: fam.architecture_value,
                    net_worth: family_net_worth(fam),
                    friends: fam.friends,
                    magic_coins,
                    portraits,
                }
            });

        Some(LotInfo {
            lot_number: lot_id,
            address: self.street_names.address_for(lot_id),
            house_name: meta.map_or_else(String::new, |m| m.lot_name.clone()),
            house_description: meta.map_or_else(String::new, |m| m.lot_description.clone()),
            kind,
            has_house: meta.is_some_and(|m| m.has_house),
            house_cost: meta.map_or(0, |m| m.house_cost),
            magic_cost: self.magic_cost_for_lot(lot_id),
            occupant,
            anchor,
            tooltip: Some(sims_ui::neighborhood::TooltipOwner {
                rect: Rect::new(left, top, right - left, bottom - top),
                cursor_x: self
                    .hover_card_cursor
                    .or(self.pointer)
                    .map_or(anchor[0], |p| p[0]),
            }),
            alpha_shift: None,
        })
    }
}
