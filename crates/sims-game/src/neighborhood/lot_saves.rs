//! Lot outcomes, background saves, the save manager, and saving, loading and restoring the lot.

use super::*;

impl NeighborhoodShell {
    /// Handles a [`crate::game::Outcome`] produced by an input to the active lot
    /// (including Save, Save Manager, Photo Album, Save & Leave to Neighborhood,
    /// Quit, and Photo).
    pub fn handle_lot_outcome(&mut self, outcome: crate::game::Outcome) -> ShellOutcome {
        self.persist_options();
        // A Camera Mode click's photo goes to the household's album only
        // (`cWinCamera`, `0x4a9144`): the host writes no file for it. The
        // screenshot hotkey keeps its own route.
        let mut out = ShellOutcome {
            redraw: outcome.redraw,
            quit: false,
            photo: false,
        };
        if outcome.photo
            && let Err(err) = self.capture_lot_photo()
        {
            self.show_notice("Photo Album", format!("The photo was not saved: {err}"));
        }
        // A lot pick does not travel by itself: the script that asked for
        // the lot asks for the trip (Generic Sims Call 17) a tick later.
        if self.pump_trip_request() {
            out.redraw = true;
            return out;
        }
        if outcome.open_save_manager {
            self.open_save_manager(None);
            out.redraw = true;
            return out;
        }
        if outcome.open_coop_lobby {
            if let ActiveScreen::Lot(session) = &mut self.screen {
                session.game.open_coop_lobby(None);
            }
            out.redraw = true;
            return out;
        }
        if outcome.open_photo_album {
            if let Err(err) = self.open_photo_album()
                && let ActiveScreen::Lot(session) = &mut self.screen
            {
                session
                    .game
                    .show_notice("Photo Album", format!("Photo Album unavailable: {err}"));
            }
            out.redraw = true;
            return out;
        }
        if outcome.neighborhood {
            match self.leave_lot(outcome.save) {
                Ok(()) => {
                    out.redraw = true;
                }
                Err(e) => {
                    if let ActiveScreen::Lot(session) = &mut self.screen {
                        session.game.after_save_failed();
                        session.game.show_notice(
                            "Neighborhood",
                            format!("Could not return to Neighborhood: {e}"),
                        );
                    }
                    out.redraw = true;
                }
            }
            return out;
        }
        let save_err = if outcome.save {
            self.save_current_lot().err()
        } else {
            None
        };
        let saved_ok = save_err.is_none();
        if let Some(err) = save_err
            && let ActiveScreen::Lot(session) = &mut self.screen
        {
            session.game.after_save_failed_with_notice(
                "Save Failed",
                format!("The lot was not saved: {err}"),
            );
            out.redraw = true;
        }
        if outcome.quit && saved_ok {
            self.leave_coop_if_active();
            self.flush_background_saves();
            out.quit = true;
        }
        out
    }

    /// Enqueues a non-blocking CoW [`sims_sim::World`] snapshot onto the
    /// background [`AutosaveWorker`] with `expected_instance_id` fencing.
    pub fn enqueue_background_world_save(&mut self, target: SaveTarget) -> Result<(), String> {
        let ActiveScreen::Lot(session) = &self.screen else {
            return Err("no lot is currently open".into());
        };
        if session.game.coop_blocks_local_save() {
            return Err("Only the co-op host can save the lot.".into());
        }
        if session.active_trip.is_some() {
            return Err(
                "cannot save directly while visiting a travel destination; return home first"
                    .into(),
            );
        }
        let world = session.game.lot().world().clone();
        let label = self.active_lot_save_label(session);
        self.autosave_worker.enqueue_world(
            session.neighborhood_id.clone(),
            session.neighborhood_instance_id,
            session.house,
            target,
            u64::from(session.game.lot().ticks()),
            label,
            world,
        )
    }

    /// Enqueues a non-blocking CoW [`Saved`] lot snapshot onto the background
    /// [`AutosaveWorker`] (encoding runs off-thread).
    pub fn enqueue_background_lot_save(&mut self, target: SaveTarget) -> Result<(), String> {
        let ActiveScreen::Lot(session) = &self.screen else {
            return Err("no lot is currently open".into());
        };
        if session.game.coop_blocks_local_save() {
            return Err("Only the co-op host can save the lot.".into());
        }
        if session.active_trip.is_some() {
            return Err(
                "cannot save directly while visiting a travel destination; return home first"
                    .into(),
            );
        }
        let saved = session.game.save()?;
        let label = self.active_lot_save_label(session);
        self.autosave_worker.enqueue_saved_lot(
            session.neighborhood_id.clone(),
            session.neighborhood_instance_id,
            session.house,
            target,
            u64::from(session.game.lot().ticks()),
            label,
            saved,
        )
    }

    pub(super) fn active_lot_save_label(&self, session: &LotSession) -> String {
        let world = session.game.lot().world();
        let hour = session.game.hour().min(23) as u8;
        let minute = world.globals.get(5).copied().unwrap_or(0).clamp(0, 59) as u8;
        let sim_day = (world.globals.get(28).copied().unwrap_or(0).max(0) as u32) + 1;
        let fam_name = self
            .stored
            .family_in(session.house)
            .and_then(|(id, _)| self.family_names.get(&id))
            .map(String::as_str);
        format_save_label(fam_name, sim_day, hour, minute)
    }

    /// Polls background save completions and triggers a periodic rotating
    /// autosave whenever the active lot's VM `days_played` (`world.globals[28]`)
    /// advances across midnight.
    /// Returns `true` if status feedback changed and the screen should redraw.
    pub fn poll_background_saves(&mut self) -> bool {
        let mut redraw = false;
        let mut refresh_manager = false;
        let completions = self.autosave_worker.poll_completions();
        for completion in completions {
            if self.save_manager.is_open()
                && self.save_manager_target_lot == Some(completion.lot_id)
                && completion.neighborhood_id == self.neighborhood_id
            {
                refresh_manager = true;
            }
            match &completion.result {
                Ok(meta) => {
                    if completion.target == SaveTarget::Autosave
                        && let ActiveScreen::Lot(session) = &mut self.screen
                        && session.house == completion.lot_id
                        && session.game.note().is_none()
                    {
                        session.game.set_note(format!(
                            "Autosaved ({}) at tick {}.",
                            meta.slot, meta.sim_tick
                        ));
                        redraw = true;
                    }
                }
                Err(err) => {
                    let msg = format!("Background save warning (existing saves preserved): {err}");
                    if let ActiveScreen::Lot(session) = &mut self.screen {
                        session.game.set_note(msg);
                    } else {
                        self.note = Some(msg);
                    }
                    redraw = true;
                }
            }
            if self.last_save_completions.len() >= 64 {
                self.last_save_completions.remove(0);
            }
            self.last_save_completions.push(completion);
        }

        if refresh_manager && let Some(lot_id) = self.save_manager_target_lot {
            let active_err = self
                .save_manager_snapshot
                .as_ref()
                .and_then(|s| s.active_error.clone());
            let label = self.format_lot_label_for_saves(lot_id);
            self.save_manager_snapshot = Some(build_save_manager_snapshot(
                &self.store.autosaves(),
                &self.neighborhood_id,
                self.slot,
                lot_id,
                label,
                active_err,
            ));
            redraw = true;
        }

        // Periodic midnight rotating autosave (triggered strictly when VM
        // `daily::turn_day` increments `world.globals[28]`). Skip during active
        // travel visits so destination community templates are never overwritten.
        let trigger_midnight = if let ActiveScreen::Lot(session) = &mut self.screen {
            let days_played = session
                .game
                .lot()
                .world()
                .globals
                .get(28)
                .copied()
                .unwrap_or(0);
            let turned = session.active_trip.is_none()
                && !session.game.coop_blocks_local_save()
                && days_played > session.last_days_played;
            session.last_days_played = days_played;
            turned
        } else {
            false
        };
        if trigger_midnight && let Err(err) = self.enqueue_background_lot_save(SaveTarget::Autosave)
        {
            if let ActiveScreen::Lot(session) = &mut self.screen {
                session.game.set_note(format!("Autosave skipped: {err}"));
            }
            redraw = true;
        }
        redraw
    }

    /// Flushes all pending background save/autosave jobs and drains completions.
    pub fn flush_background_saves(&mut self) {
        self.autosave_worker.flush();
        let _ = self.poll_background_saves();
    }

    /// Returns completed background save results observed since the last drain.
    pub fn take_save_completions(&mut self) -> Vec<SaveCompletion> {
        std::mem::take(&mut self.last_save_completions)
    }

    /// Opens the crash-safe [`SaveManagerDialog`] for the currently active lot
    /// (or hovered/last lot), flushing pending background saves first so every
    /// manual and rotating autosave slot is shown.
    pub fn open_save_manager(&mut self, active_error: Option<String>) -> bool {
        self.flush_background_saves();
        let lot_id = match &self.screen {
            ActiveScreen::Lot(session) => {
                if let Some(trip) = &session.active_trip {
                    Some(trip.home_house)
                } else {
                    Some(session.house)
                }
            }
            _ => self
                .save_manager_target_lot
                .or_else(|| self.hovered_lot_id.map(u32::from)),
        };
        let Some(lot_id) = lot_id else {
            self.note = Some("Enter a lot to manage its saves.".into());
            return false;
        };
        self.save_manager_target_lot = Some(lot_id);
        let lot_label = self.format_lot_label_for_saves(lot_id);
        let snapshot = build_save_manager_snapshot(
            &self.store.autosaves(),
            &self.neighborhood_id,
            self.slot,
            lot_id,
            lot_label,
            active_error,
        );
        self.clear_hover_state();
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.close_camera_strip();
        }
        self.save_manager_snapshot = Some(snapshot);
        self.save_manager.open();
        true
    }

    /// Closes the [`SaveManagerDialog`] without modifying any save slot.
    pub fn close_save_manager(&mut self) {
        self.save_manager.close();
        self.save_manager_snapshot = None;
    }

    pub(super) fn format_lot_label_for_saves(&self, lot_id: u32) -> String {
        let fam = self
            .stored
            .family_in(lot_id)
            .and_then(|(id, _)| self.family_names.get(&id))
            .cloned();
        match fam {
            Some(name) => format!("Neighborhood {} · Lot {lot_id} ({name})", self.slot),
            None => format!("Neighborhood {} · Lot {lot_id}", self.slot),
        }
    }

    /// Executes a [`SaveManagerCommand`] emitted by [`SaveManagerDialog`]
    /// (`LoadSlot`, `FallbackToNewestIntact`, `DeleteSlot`, or `Cancel`).
    pub fn handle_save_manager_command(&mut self, cmd: SaveManagerCommand) -> ShellOutcome {
        // Always target the lot displayed in Save Manager (e.g. the home house
        // during a live travel visit), never overriding it with a destination lot.
        let Some(lot_id) = self
            .save_manager_target_lot
            .or_else(|| self.active_lot_house())
        else {
            self.close_save_manager();
            return ShellOutcome::redraw();
        };
        match cmd {
            SaveManagerCommand::Cancel => {
                self.close_save_manager();
                ShellOutcome::redraw()
            }
            SaveManagerCommand::DeleteSlot { slot_id } => {
                let err = match parse_slot_id(&slot_id) {
                    Some(slot) => self
                        .store
                        .autosaves()
                        .delete_slot(&self.neighborhood_id, lot_id, slot)
                        .err()
                        .map(|e| format!("Could not delete save slot: {e}")),
                    None => Some(format!("Unknown save slot identifier: {slot_id}")),
                };
                let label = self.format_lot_label_for_saves(lot_id);
                self.save_manager_snapshot = Some(build_save_manager_snapshot(
                    &self.store.autosaves(),
                    &self.neighborhood_id,
                    self.slot,
                    lot_id,
                    label,
                    err,
                ));
                ShellOutcome::redraw()
            }
            SaveManagerCommand::LoadSlot { slot_id } => {
                let result = if self.active_trip().is_some() {
                    Err(
                        "Cannot load a save slot while visiting a travel destination; return home first."
                            .to_owned(),
                    )
                } else {
                    parse_slot_id(&slot_id)
                        .ok_or_else(|| format!("unknown save slot {slot_id:?}"))
                        .and_then(|slot| self.load_autosave_slot_into_lot(lot_id, slot))
                };
                match result {
                    Ok(tick) => {
                        self.close_save_manager();
                        if let ActiveScreen::Lot(session) = &mut self.screen {
                            session
                                .game
                                .set_note(format!("Loaded save slot {slot_id} (tick {tick})."));
                        }
                    }
                    Err(err) => {
                        // Non-destructive: keep dialog open with warning banner & fallback.
                        let label = self.format_lot_label_for_saves(lot_id);
                        self.save_manager_snapshot = Some(build_save_manager_snapshot(
                            &self.store.autosaves(),
                            &self.neighborhood_id,
                            self.slot,
                            lot_id,
                            label,
                            Some(err),
                        ));
                    }
                }
                ShellOutcome::redraw()
            }
            SaveManagerCommand::FallbackToNewestIntact { slot_id } => {
                let result = if self.active_trip().is_some() {
                    Err(
                        "Cannot load a save slot while visiting a travel destination; return home first."
                            .to_owned(),
                    )
                } else {
                    // Always consult current REDB metadata first and validate
                    // candidates in descending `(sim_tick, saved_at_unix_secs, slot)`
                    // order so a newer save committed after the UI snapshot is
                    // never bypassed in favor of an older slot.
                    let mut candidates: Vec<sims_sim::autosave::SaveMetadata> = self
                        .store
                        .autosaves()
                        .list_saves_metadata(&self.neighborhood_id, lot_id)
                        .map_err(|e| e.to_string())
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|entry| match entry {
                            sims_sim::autosave::SaveSlotEntry::Intact(meta) => Some(meta),
                            sims_sim::autosave::SaveSlotEntry::Corrupt { .. } => None,
                        })
                        .collect();
                    candidates.sort_by(|a, b| {
                        let key_a = match a.slot {
                            SaveSlot::Manual => (1u8, 0u64),
                            SaveSlot::Autosave(seq) => (0u8, seq),
                        };
                        let key_b = match b.slot {
                            SaveSlot::Manual => (1u8, 0u64),
                            SaveSlot::Autosave(seq) => (0u8, seq),
                        };
                        b.sim_tick
                            .cmp(&a.sim_tick)
                            .then_with(|| b.saved_at_unix_secs.cmp(&a.saved_at_unix_secs))
                            .then_with(|| key_b.cmp(&key_a))
                    });
                    let mut ordered_slots: Vec<SaveSlot> =
                        candidates.into_iter().map(|m| m.slot).collect();
                    if let Some(hint_slot) = parse_slot_id(&slot_id)
                        && !ordered_slots.contains(&hint_slot)
                    {
                        ordered_slots.push(hint_slot);
                    }
                    let mut recovered_tick = None;
                    let mut last_err = format!("no intact save slots available for lot {lot_id}");
                    for slot in ordered_slots {
                        match self.load_autosave_slot_into_lot(lot_id, slot) {
                            Ok(tick) => {
                                recovered_tick = Some(tick);
                                break;
                            }
                            Err(err) => {
                                last_err = err;
                            }
                        }
                    }
                    recovered_tick.ok_or(last_err)
                };
                match result {
                    Ok(tick) => {
                        self.close_save_manager();
                        if let ActiveScreen::Lot(session) = &mut self.screen {
                            session.game.set_note(format!(
                                "Recovered newest intact save (tick {tick}); corrupt saves preserved."
                            ));
                        }
                    }
                    Err(err) => {
                        let label = self.format_lot_label_for_saves(lot_id);
                        self.save_manager_snapshot = Some(build_save_manager_snapshot(
                            &self.store.autosaves(),
                            &self.neighborhood_id,
                            self.slot,
                            lot_id,
                            label,
                            Some(format!("Recovery failed: {err}")),
                        ));
                    }
                }
                ShellOutcome::redraw()
            }
        }
    }

    pub(super) fn load_autosave_slot_into_lot(
        &mut self,
        lot_id: u32,
        slot: sims_sim::autosave::SaveSlot,
    ) -> Result<u64, String> {
        let loaded = self
            .store
            .autosaves()
            .load_save_validated_with_sha256(
                &self.neighborhood_id,
                lot_id,
                slot,
                validate_save_payload_cached_with_sha256,
            )
            .map_err(|e| e.to_string())?;
        self.apply_loaded_save_into_lot(lot_id, &loaded)
    }

    pub(super) fn apply_loaded_save_into_lot(
        &mut self,
        lot_id: u32,
        loaded: &sims_sim::autosave::LoadedSave,
    ) -> Result<u64, String> {
        if matches!(&self.screen, ActiveScreen::Lot(s) if s.game.coop_active()) {
            return Err("Leave the co-op session before loading a local save slot.".into());
        }
        if self.active_trip().is_some() {
            return Err(
                "cannot restore a save slot while visiting a travel destination; return home first"
                    .into(),
            );
        }
        let snapshot = self
            .store
            .load_lot_snapshot(&self.neighborhood_id, lot_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))?;
        let has_trvl_envelope = snapshot
            .saved_lot
            .as_deref()
            .is_some_and(|bytes| bytes.len() >= 4 && &bytes[0..4] == b"TRVL");
        let secondary_dest = snapshot
            .neighborhood
            .family_in(lot_id)
            .map(|(_, f)| f.secondary_house())
            .filter(|&d| d != 0);
        if has_trvl_envelope || secondary_dest.is_some() {
            let dest_detail = match secondary_dest {
                Some(dest) => format!(" to lot {dest}"),
                None => String::new(),
            };
            return Err(format!(
                "lot {lot_id} has an active travel trip{dest_detail}; resume or abort the trip before restoring a save slot"
            ));
        }
        let real_instance_id = snapshot.neighborhood.instance_id;
        let real_revision = snapshot.neighborhood.revision;

        // Boot through an ephemeral staging NeighborhoodStore populated with the
        // CURRENT StoredNeighborhood (all newer CAF families, NBRS, NGBH, TATT,
        // made_characters, and deletion_epoch tombstones) + the selected validated
        // save slot. This avoids retrying a corrupt SAVED_LOTS record and runs
        // `sims_vm::lifecycle::enter_lot`'s full neighborhood reconciliation
        // before replacing the live session.
        let staging_dir = TempStagingDir::new("slot-boot")?;
        let staging_store = NeighborhoodStore::open(&staging_dir.path().join("stage.redb"))
            .map_err(|e| e.to_string())?;
        for (file_name, bytes) in &snapshot.character_files {
            staging_store
                .save_character_file(&self.neighborhood_id, file_name, bytes)
                .map_err(|e| e.to_string())?;
        }
        if let Some(base_house) = &snapshot.base_house {
            staging_store
                .save_base_house(&self.neighborhood_id, lot_id, base_house)
                .map_err(|e| e.to_string())?;
        }

        let install_assets = self.ensure_install_assets()?;
        let (staged_saved_bytes, restored_tick) = match &loaded.metadata.payload_kind {
            SavePayloadKind::SavedLot => {
                let _ = Saved::decode(&loaded.payload).map_err(|e| e.to_string())?;
                (loaded.payload.clone(), None)
            }
            SavePayloadKind::WorldSnapshot => {
                let (mut world, queues) = sims_sim::World::decode_snapshot_parts(&loaded.payload)
                    .map_err(|e| e.to_string())?;
                if loaded.metadata.format_version < sims_sim::AUTOSAVE_MADE_ORDER_FORMAT {
                    world.characters.made.sort_by_guid();
                }
                // A payload saved before the queues were kept takes its star
                // pools from where its stars stand, as a `Saved` of that age
                // does.
                let queues =
                    queues.unwrap_or_else(|| sims_vm::boot::save::queues_saved_before_kept(&world));
                // Obtain a compatible base `Saved` envelope without touching a
                // potentially corrupt SAVED_LOTS entry: reuse the open lot if it
                // matches `lot_id`, otherwise boot the clean base house in staging.
                let base_saved = match &self.screen {
                    ActiveScreen::Lot(session) if session.house == lot_id => session.game.save()?,
                    _ => {
                        staging_store
                            .save_neighborhood(&self.neighborhood_id, &snapshot.neighborhood)
                            .map_err(|e| e.to_string())?;
                        let base_game = Game::open_from_store_cached(
                            &self.install_root,
                            &staging_store,
                            &self.neighborhood_id,
                            lot_id,
                            &self.answers,
                            self.play,
                            self.frames.clone(),
                            Some(&install_assets),
                        )?;
                        base_game.save()?
                    }
                };
                let spliced = splice_world_into_saved_bytes(&base_saved, &world)?;
                // The splice keeps the base lot's pending indicator refreshes,
                // which may name people the restored world lacks: take the
                // snapshot's own (`World::encode_snapshot`).
                let mut spliced = Saved::decode(&spliced).map_err(|e| e.to_string())?;
                spliced.set_indicator_queues(queues);
                let spliced = spliced.encode();
                (spliced, Some(loaded.metadata.sim_tick))
            }
            SavePayloadKind::Custom(tag) => {
                return Err(format!("unsupported custom payload kind {tag:?}"));
            }
        };

        // Keep `snapshot.neighborhood` unmodified so restoring an older slot
        // reconciles against the live neighborhood's current FAMI budgets, NBRS,
        // made_characters, and deletion_epoch without rolling any of them back.
        staging_store
            .save_lot_and_neighborhood(
                &self.neighborhood_id,
                lot_id,
                &staged_saved_bytes,
                &snapshot.neighborhood,
            )
            .map_err(|e| e.to_string())?;

        let mut game = Game::open_from_store_cached(
            &self.install_root,
            &staging_store,
            &self.neighborhood_id,
            lot_id,
            &self.answers,
            self.play,
            self.frames.clone(),
            Some(&install_assets),
        )?;
        self.load_saved_options(&mut game);
        game.set_album_button_enabled(true);
        game.set_neighborhood_slot(self.slot);
        game.set_special_lot_catalog(self.build_special_lot_catalog());
        game.set_debug(self.debug);
        game.resize(self.size);
        let initial_days_played = game.lot().world().globals.get(28).copied().unwrap_or(0);
        let ticks = restored_tick.unwrap_or_else(|| u64::from(game.lot().ticks()));
        let next_music = sound::lot_music_mode(game.mode(), game.lot().lot_id());

        // Replace live session only after the reconciled boot succeeded.
        self.stored = snapshot.neighborhood;
        self.sync_neighborhood_iff();
        self.screen = ActiveScreen::Lot(Box::new(LotSession {
            game,
            neighborhood_id: self.neighborhood_id.clone(),
            neighborhood_instance_id: real_instance_id,
            neighborhood_revision: real_revision,
            staging: Some(StagingStoreHandle {
                _dir: staging_dir,
                store: staging_store,
            }),
            house: lot_id,
            last_days_played: initial_days_played,
            active_trip: None,
            is_destination_edit: false,
        }));
        self.lot_generation = self.lot_generation.wrapping_add(1);
        self.dialog = None;
        self.save_manager_target_lot = Some(lot_id);
        self.note = None;
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
        self.pressed_lot_id = None;
        self.nbhd_state.hovered_lot = None;
        self.audio_queue
            .push(Input::Events(vec![SoundEvent::StopAll]));
        self.audio_queue.push(Input::MusicMode(next_music));
        Ok(ticks)
    }

    /// Saves the currently open lot to REDB (F5 or Options → Save): performs the
    /// strictly fenced `SAVED_LOTS` + neighborhood writeback commit first, then
    /// mirrors the exact committed `Saved` bytes to the background
    /// [`AutosaveWorker`] manual slot and updates the cached neighborhood map
    /// thumbnail immediately.
    pub fn save_current_lot(&mut self) -> Result<SaveReport, String> {
        let cached_vm = self.install_assets.as_ref().map(|a| a.vm_cache().clone());
        let (report, committed_bytes, nh_id, expected_instance_id, house, ticks, label) = {
            let ActiveScreen::Lot(session) = &mut self.screen else {
                return Err("no lot is currently open".into());
            };
            if session.game.coop_blocks_local_save() {
                let msg = "Only the co-op host can save the lot.";
                session.game.set_note(msg);
                return Err(msg.into());
            }
            if session.active_trip.is_some() {
                let msg =
                    "Cannot save directly while visiting a travel destination; return home first.";
                session.game.set_note(msg);
                return Err(msg.into());
            }
            let (report, lot_bytes) = if session.game.world_mutated_via_restore() {
                save_cheat_mutated_lot_to_store(
                    &self.install_root,
                    &self.store,
                    &session.neighborhood_id,
                    session.neighborhood_instance_id,
                    session.neighborhood_revision,
                    session.house,
                    self.arch_multiplier,
                    &self.answers,
                    self.frames.clone(),
                    cached_vm.as_deref(),
                    &mut session.game,
                )?
            } else if let Some(staging) = &session.staging {
                save_staged_lot_to_store(
                    &self.install_root,
                    &self.store,
                    &staging.store,
                    &session.neighborhood_id,
                    session.neighborhood_instance_id,
                    session.neighborhood_revision,
                    session.house,
                    &mut session.game,
                )?
            } else {
                // Propagate fencing / stale-revision / replacement errors directly
                // instead of bypassing them with an unfenced fallback.
                let report = session.game.save_to_neighborhood(
                    &self.install_root,
                    &self.store,
                    &session.neighborhood_id,
                )?;
                let bytes = self
                    .store
                    .load_saved_lot(&session.neighborhood_id, session.house)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "committed saved lot missing after save".to_owned())?;
                (report, bytes)
            };
            session.neighborhood_revision = report.revision;
            let nh_id = session.neighborhood_id.clone();
            let inst = session.neighborhood_instance_id;
            let house = session.house;
            let ticks = u64::from(session.game.lot().ticks());
            let world = session.game.lot().world();
            let hour = session.game.hour().min(23) as u8;
            let minute = world.globals.get(5).copied().unwrap_or(0).clamp(0, 59) as u8;
            let sim_day = (world.globals.get(28).copied().unwrap_or(0).max(0) as u32) + 1;
            let fam_name = self
                .stored
                .family_in(house)
                .and_then(|(id, _)| self.family_names.get(&id))
                .map(String::as_str);
            let label = format_save_label(fam_name, sim_day, hour, minute);
            (report, lot_bytes, nh_id, inst, house, ticks, label)
        };
        if let Ok(Some(reloaded)) = self.store.load_neighborhood(&nh_id) {
            self.stored = reloaded;
        } else {
            self.stored.revision = report.revision;
        }
        self.sync_neighborhood_iff();
        // Mirror the exact committed `Saved` bytes into the crash-safe manual slot
        // on the background worker so F9 and Save Manager share one source of truth.
        if let Err(err) = self.autosave_worker.enqueue_saved_lot_bytes(
            nh_id,
            expected_instance_id,
            house,
            SaveTarget::Manual,
            ticks,
            label,
            committed_bytes,
        ) && let ActiveScreen::Lot(session) = &mut self.screen
        {
            session
                .game
                .set_note(format!("Saved lot; manual slot mirror warning: {err}"));
        }
        self.capture_active_lot_thumbnail();
        Ok(report)
    }

    pub(super) fn capture_active_lot_thumbnail(&mut self) {
        if let ActiveScreen::Lot(session) = &mut self.screen
            && session.active_trip.is_none()
            && let Ok(lot_id) = u16::try_from(session.house)
        {
            let target_size_and_cutaway = self
                .map
                .lot(lot_id)
                .map(|m| ([m.normal.width, m.normal.height], m.roofs_off.is_some()))
                .or_else(|| {
                    let kind = map_kind_for_house(&self.install, self.slot, session.house);
                    load_map_for_slot(&self.install, self.slot, kind)
                        .ok()
                        .and_then(|m| {
                            m.lot(lot_id)
                                .map(|l| ([l.normal.width, l.normal.height], l.roofs_off.is_some()))
                        })
                });
            if let Some((normal_size, has_cutaway)) = target_size_and_cutaway {
                let (world, lot_shape, assets) = session.game.render_parts_mut();
                let thumb =
                    capture_lot_thumbnail(world, lot_shape, assets, normal_size, has_cutaway);
                let stored_thumb = StoredLotThumbnail {
                    normal: to_stored_thumb_surface(&thumb.normal),
                    roofs_off: thumb.roofs_off.as_ref().map(to_stored_thumb_surface),
                };
                match self.store.save_lot_thumbnail_checked(
                    &session.neighborhood_id,
                    session.neighborhood_instance_id,
                    session.house,
                    &stored_thumb,
                ) {
                    Ok(Ok(())) => {
                        self.custom_thumbs
                            .insert((session.neighborhood_id.clone(), lot_id), thumb);
                    }
                    Ok(Err(commit_err)) => {
                        session.game.set_note(format!(
                            "Saved lot; map thumbnail was not updated ({commit_err:?})."
                        ));
                    }
                    Err(err) => {
                        session
                            .game
                            .set_note(format!("Saved lot; map thumbnail update warning: {err}"));
                    }
                }
            }
        }
    }

    /// Reloads the currently open lot from its latest REDB manual save (F9),
    /// preferring the primary committed `SAVED_LOTS` record whenever it is
    /// intact (so a newer F5 commit is never shadowed by a stale manual mirror)
    /// and falling back non-destructively to [`SaveManagerDialog`] if either
    /// the manual slot or `SAVED_LOTS` is corrupt.
    pub fn restore_current_lot(&mut self) -> Result<(), String> {
        self.flush_background_saves();
        let (neighborhood_id, house) = match &mut self.screen {
            ActiveScreen::Lot(session) => {
                if session.game.coop_active() {
                    let msg = "Leave the co-op session before restoring a local save.";
                    session.game.set_note(msg);
                    return Err(msg.into());
                }
                if session.active_trip.is_some() {
                    let msg = "Cannot restore directly while visiting a travel destination; return home first.";
                    session.game.set_note(msg);
                    return Err(msg.into());
                }
                (session.neighborhood_id.clone(), session.house)
            }
            _ => return Err("no lot is currently open".into()),
        };
        let manual_slot = self.store.autosaves().load_save_validated_with_sha256(
            &neighborhood_id,
            house,
            sims_sim::autosave::SaveSlot::Manual,
            validate_save_payload_cached_with_sha256,
        );
        let manual_loaded = match manual_slot {
            Ok(loaded) => Some(loaded),
            Err(sims_sim::autosave::SaveError::Missing { .. }) => None,
            Err(corrupt_err) => {
                let msg = format!(
                    "Manual save could not be loaded ({corrupt_err}). Choose an intact autosave below."
                );
                self.open_save_manager(Some(msg.clone()));
                return Err(msg);
            }
        };
        let saved_lot_bytes = self
            .store
            .load_saved_lot(&neighborhood_id, house)
            .map_err(|e| e.to_string())?;
        if let Some(bytes) = saved_lot_bytes {
            if Saved::decode(&bytes).is_ok() {
                match self.enter_lot(house) {
                    Ok(()) => {
                        if let ActiveScreen::Lot(session) = &mut self.screen {
                            let ticks = session.game.lot().ticks();
                            session
                                .game
                                .set_note(format!("Back to the save after tick {ticks}."));
                        }
                        return Ok(());
                    }
                    Err(err) => {
                        self.open_save_manager(Some(format!(
                            "Saved lot could not be restored ({err}). Choose an intact autosave below."
                        )));
                        return Err(err);
                    }
                }
            } else if let Some(loaded) = manual_loaded {
                // Primary SAVED_LOTS is unreadable, but the validated Manual slot is intact.
                return match self.apply_loaded_save_into_lot(house, &loaded) {
                    Ok(ticks) => {
                        if let ActiveScreen::Lot(session) = &mut self.screen {
                            session
                                .game
                                .set_note(format!("Back to the save after tick {ticks}."));
                        }
                        Ok(())
                    }
                    Err(err) => {
                        self.open_save_manager(Some(format!(
                            "Manual save could not be restored ({err}). Choose an intact autosave below."
                        )));
                        Err(err)
                    }
                };
            } else {
                let msg =
                    "Saved lot is corrupt and no intact manual slot is available. Choose an intact autosave below."
                        .to_owned();
                self.open_save_manager(Some(msg.clone()));
                return Err(msg);
            }
        }
        if let Some(loaded) = manual_loaded {
            return match self.apply_loaded_save_into_lot(house, &loaded) {
                Ok(ticks) => {
                    if let ActiveScreen::Lot(session) = &mut self.screen {
                        session
                            .game
                            .set_note(format!("Back to the save after tick {ticks}."));
                    }
                    Ok(())
                }
                Err(err) => {
                    self.open_save_manager(Some(format!(
                        "Manual save could not be restored ({err}). Choose an intact autosave below."
                    )));
                    Err(err)
                }
            };
        }
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session
                .game
                .set_note("The save was not restored: nothing is saved for this house");
        }
        Err("nothing is saved for this house".into())
    }
}
