//! Opening the neighborhood shell.

use super::*;

impl NeighborhoodShell {
    /// Opens the Neighborhood shell for `install_root`, storing all play state
    /// inside the REDB file `saves_path` (creating/importing neighborhood `slot`
    /// on first run without mutating `install_root`).
    pub fn open(
        install_root: PathBuf,
        saves_path: PathBuf,
        slot: u8,
        play: bool,
        answers: Vec<u8>,
        frames: Frames,
    ) -> Result<Self, String> {
        let install = Install::detect(&install_root).map_err(|e| e.to_string())?;
        let store = NeighborhoodStore::open(&saves_path).map_err(|e| e.to_string())?;
        // Non-destructive startup migration: surface any schema/corruption error
        // clearly without crashing or overwriting existing saves.
        let (migration_report, migration_error, startup_note) = match store.migrate_all() {
            Ok(report) => (Some(report), None, None),
            Err(err) => {
                let msg =
                    format!("Save migration warning (existing saves preserved untouched): {err}");
                (None, Some(err.to_string()), Some(msg))
            }
        };
        let autosave_worker = AutosaveWorker::spawn(store.autosaves());
        let options_file = saves_path.with_extension("options.iff");
        let slot = slot.clamp(MIN_NEIGHBORHOOD, MAX_NEIGHBORHOOD);
        let kind = default_map_kind(&install, slot);
        let neighborhood_id = neighborhood_id_for_slot(slot);
        let mut stored = ensure_imported(&install, &store, &neighborhood_id)?;
        migrate_legacy_caf_sidecar_if_needed(&store, &saves_path, &neighborhood_id, &mut stored);
        let _ = store.set_active_neighborhood(&neighborhood_id);
        let user_dir = resolve_user_dir(&install, slot)?;
        let pool = Arc::new(ArchivePool::new());

        // Parallelize independent UI art packs, string/tuning tables,
        // neighborhood map, descriptions/street names, and base Index/Faces
        // while sharing a single ArchivePool.
        let (art_res, strings_tuple, map_res, text_meta, faces_tuple, custom_thumbs, family_names) =
            std::thread::scope(|scope| {
                let art_handle = scope.spawn(|| -> Result<_, String> {
                    let ui = UiGraphics::open_with_pool(&install, Arc::clone(&pool))
                        .map_err(|e| e.to_string())?;
                    let chrome_art = sims_ui::neighborhood::load_art(&ui, US_ENGLISH)
                        .map_err(|e| e.to_string())?;
                    let family_bin_art = sims_ui::family_bin::load_art(&ui, US_ENGLISH)
                        .map_err(|e| e.to_string())?;
                    let dialog_art = Art::load(&ui, US_ENGLISH, &dialog::RESOURCES)
                        .map_err(|e| e.to_string())?;
                    let save_manager_art = Art::load_with_optional(
                        &ui,
                        US_ENGLISH,
                        &dialog::RESOURCES,
                        &[sims_ui::neighborhood::art_id::POPUP_TILER],
                    )
                    .map_err(|e| e.to_string())?;
                    let loading_art = sims_ui::loading::load_art(&ui, US_ENGLISH)
                        .or_else(|_| {
                            Art::load_with_optional(
                                &ui,
                                US_ENGLISH,
                                &[],
                                &sims_ui::loading::RESOURCES,
                            )
                        })
                        .map_err(|e| e.to_string())?;
                    let album_art = sims_ui::album::load_art(&ui, US_ENGLISH)
                        .or_else(|_| {
                            Art::load_with_optional(
                                &ui,
                                US_ENGLISH,
                                &dialog::RESOURCES,
                                &sims_ui::album::RESOURCES,
                            )
                        })
                        .map_err(|e| e.to_string())?;
                    let travel_art = sims_ui::travel::load_art(&ui, US_ENGLISH)
                        .or_else(|_| {
                            Art::load_with_optional(
                                &ui,
                                US_ENGLISH,
                                &[],
                                &sims_ui::travel::RESOURCES,
                            )
                        })
                        .map_err(|e| e.to_string())?;
                    let coop_art = sims_ui::coop::load_art(&ui, US_ENGLISH)
                        .or_else(|_| {
                            Art::load_with_optional(&ui, US_ENGLISH, &[], &sims_ui::coop::RESOURCES)
                        })
                        .unwrap_or_default();
                    Ok((
                        chrome_art,
                        family_bin_art,
                        dialog_art,
                        save_manager_art,
                        loading_art,
                        album_art,
                        travel_art,
                        coop_art,
                    ))
                });
                let strings_handle = scope.spawn(|| {
                    let options_state = if options_file.is_file() {
                        OptionsState::load_iff(&options_file)
                            .or_else(|_| OptionsState::load_install(&install))
                            .unwrap_or_default()
                    } else {
                        OptionsState::load_install(&install).unwrap_or_default()
                    };
                    let chrome_strings =
                        NeighborhoodStrings::load(&install, US_ENGLISH).unwrap_or_default();
                    let loading_strings =
                        LoadingStrings::load(&install, US_ENGLISH).unwrap_or_default();
                    let album_strings =
                        AlbumStrings::load_from_install(&install, US_ENGLISH).unwrap_or_default();
                    let travel_strings =
                        TravelStrings::load(&install, US_ENGLISH).unwrap_or_default();
                    let dialogs = DialogStrings::load(&install);
                    let arch_multiplier = load_arch_multiplier(&install);
                    let magic_lot_91_cost = lifecycle::magic_town_lot_cost(Some(&install_root), 91);
                    let magic_lot_other_cost =
                        lifecycle::magic_town_lot_cost(Some(&install_root), 90);
                    (
                        options_state,
                        chrome_strings,
                        loading_strings,
                        album_strings,
                        travel_strings,
                        dialogs,
                        arch_multiplier,
                        magic_lot_91_cost,
                        magic_lot_other_cost,
                    )
                });
                let map_handle = scope.spawn(|| load_map_for_slot(&install, slot, kind));
                let meta_handle = scope.spawn(|| {
                    let street_names = StreetNames::load(&install, &user_dir);
                    let descriptions = NeighborhoodDescriptions::load(&install, &user_dir);
                    (street_names, descriptions)
                });
                let faces_handle = scope.spawn(|| {
                    let base_index = Index::build(&install).ok().map(Arc::new);
                    let (faces, base_catalog) = load_neighborhood_faces_cached(
                        &install,
                        &store,
                        &neighborhood_id,
                        &stored,
                        &pool,
                        base_index.as_ref(),
                        None,
                    );
                    (base_index, base_catalog, faces)
                });
                let family_names = load_family_names_with_caf(&user_dir, &stored);
                let custom_thumbs = load_persisted_custom_thumbs(&store, &neighborhood_id);
                (
                    art_handle.join().expect("neighborhood art worker panicked"),
                    strings_handle
                        .join()
                        .expect("neighborhood strings worker panicked"),
                    map_handle.join().expect("neighborhood map worker panicked"),
                    meta_handle
                        .join()
                        .expect("neighborhood descriptions worker panicked"),
                    faces_handle
                        .join()
                        .expect("neighborhood faces worker panicked"),
                    custom_thumbs,
                    family_names,
                )
            });

        let (
            chrome_art,
            family_bin_art,
            dialog_art,
            save_manager_art,
            loading_art,
            album_art,
            travel_art,
            coop_art,
        ) = art_res?;
        let (
            options_state,
            chrome_strings,
            loading_strings,
            album_strings,
            travel_strings,
            dialogs,
            arch_multiplier,
            magic_lot_91_cost,
            magic_lot_other_cost,
        ) = strings_tuple;
        let saved_optn = options_state.to_optn();
        let mut map = map_res?;
        let (street_names, descriptions) = text_meta;
        let (base_index, base_catalog, faces) = faces_tuple;
        let lot_meta = build_lot_meta(
            &store,
            &neighborhood_id,
            &map,
            &descriptions,
            arch_multiplier,
        );
        sync_map_with_store(
            &mut map,
            &stored,
            &lot_meta,
            &neighborhood_id,
            &custom_thumbs,
        );
        let family_bin_entries = build_family_bin_entries(&stored, &family_names, &faces);

        let nbhd_state = NeighborhoodState {
            neighborhood_number: slot,
            mode: NeighborhoodMode::Normal,
            expansions: sims_ui::neighborhood::ExpansionFlags::ALL,
            has_importable_family: false,
            show_credits_button: destination_for_kind(kind).is_none(),
            hovered_lot: None,
        };
        let import_picker_strings = ShopStrings::load(&install, US_ENGLISH).unwrap_or_default();
        let book_strings = BookStrings::load(&install, US_ENGLISH).unwrap_or_default();
        let travel_state = TravelState::default();

        let vols = [
            options_state.sound.fx,
            options_state.sound.music,
            options_state.sound.vox,
        ];
        let mut loading_screen = LoadingScreen::new();
        let mut audio_queue = vec![
            Input::FxVolume(vols[0]),
            Input::MusicVolume(vols[1]),
            Input::VoxVolume(vols[2]),
        ];
        if let Some(LoadingAudioCue::StartLoadLoop { .. }) = loading_screen.begin(0) {
            audio_queue.push(Input::MusicMode(MusicMode::Load));
        }
        if let Some(LoadingAudioCue::StopLoadLoop) = loading_screen.finish() {
            audio_queue.push(Input::MusicMode(music_mode_for_kind(kind)));
        } else {
            audio_queue.push(Input::MusicMode(music_mode_for_kind(kind)));
        }

        let mut shell = Self {
            install_root,
            install,
            pool,
            base_index,
            base_catalog,
            install_assets: None,
            saves_path,
            store,
            autosave_worker,
            migration_report,
            migration_error,
            options_file,
            neighborhood_iff_synced: Some((
                neighborhood_id.clone(),
                stored.instance_id,
                stored.revision,
            )),
            options_state,
            saved_optn,
            play,
            debug: false,
            answers,
            frames,
            arch_multiplier,
            magic_lot_91_cost,
            magic_lot_other_cost,
            screen: ActiveScreen::Neighborhood,
            lot_generation: 0,
            size: [800, 600],
            canvas: Canvas::new(800, 600, [0, 0, 0]),
            pointer: None,
            clock_us: 0,
            water_frame: 0,
            chrome_art,
            chrome_strings,
            family_bin_art,
            dialog_art,
            save_manager_art,
            loading_art,
            loading_strings,
            album_art,
            travel_art,
            travel_strings,
            dialogs,
            loading_screen,
            loading_caption_seq: 1,
            slot,
            neighborhood_id,
            kind,
            map,
            stored,
            family_names,
            street_names,
            descriptions,
            faces,
            lot_meta,
            custom_thumbs,
            chrome: NeighborhoodChrome::new(),
            nbhd_state,
            hovered_lot_id: None,
            hovered_lot_since_us: None,
            hover_card_shown_for: None,
            hover_card_cursor: None,
            hover_fade: HoverFadeState::new(),
            hover_fade_was_animating: false,
            pressed_lot_id: None,
            move_in_family_id: None,
            travel_chrome: TravelChrome::new(),
            travel_state,
            travel_dialog: TravelDialog::new(),
            travel_dialog_state: None,
            pending_travel: None,
            loading_presenter: None,
            family_bin: FamilyBin::new(),
            family_bin_entries,
            dialog: None,
            dialog_fade: DialogFadeState::hidden(),
            dialog_fade_was_animating: false,
            album_dialog: PhotoAlbumDialog::new(album_strings),
            album_snapshot: None,
            album_token: None,
            album_neighborhood_id: None,
            album_has_image: BTreeSet::new(),
            album_index: BTreeMap::new(),
            save_manager: SaveManagerDialog::new(),
            save_manager_snapshot: None,
            save_manager_target_lot: None,
            last_save_completions: Vec::new(),
            coop_art,
            coop_lobby: CoopLobbyDialog::new(),
            coop_error: None,
            coop_player_name: default_player_name("Player"),
            coop_iroh_config: sims_net::IrohConfig::P2pDefault,
            import_search_dirs: Vec::new(),
            import_picker: ShopPickerDialog::new(),
            import_picker_snapshot: None,
            import_picker_entries: BTreeMap::new(),
            import_picker_strings,
            import_caf_ctx: None,
            import_rng_seed: 1,
            credits_dialog: ReferenceBookDialog::new(),
            credits_snapshot: None,
            book_strings,
            note: startup_note,
            audio_queue,
            prev_nb_steady_key: None,
            prev_nb_dynamic_rects: Vec::new(),
            present_damage: None,
        };
        shell.refresh_importable_families();
        let active_trips = shell.inspect_any_active_trips();
        if !active_trips.is_empty() && shell.note.is_none() {
            let (home_house, trip) = &active_trips[0];
            shell.note = Some(format!(
                "Lot {home_house} has an interrupted trip to lot {}. Click it to Resume or Abort.",
                trip.destination_house
            ));
        }
        Ok(shell)
    }
}
