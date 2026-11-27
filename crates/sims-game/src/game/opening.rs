//! Opening a game: from a house file, from a store snapshot or the open pack, and saving the lot back into its neighborhood.

use super::*;

impl Game {
    /// Boots the house and reads everything drawing it needs.
    pub fn open(options: &Options) -> Result<Self, String> {
        let house = &options.house;
        let install = Install::detect(&options.install).map_err(|e| e.to_string())?;
        let pool = Arc::new(ArchivePool::new());

        // Stage A: load all independent install/neighborhood inputs in parallel
        // once, sharing a single ArchivePool across every FAR read. Error
        // checks are deferred to match the exact original sequential order:
        // `open_house` -> `Lot::load` -> `Hud::open` -> `People::open`.
        let (
            index_catalog_res,
            people_assets_res,
            ordered_wardrobe,
            registry_res,
            surfaces_res,
            hud_static_res,
            base_assets_res,
            icon_frames_res,
            default_name,
        ) = std::thread::scope(|scope| {
            let idx_handle = scope.spawn(|| -> Result<_, String> {
                let index = Index::build(&install).map_err(|e| e.to_string())?;
                let neighborhood_dir = house
                    .parent()
                    .and_then(Path::parent)
                    .ok_or("the house file is not in a Houses folder")?;
                let neighborhood =
                    Neighborhood::open(neighborhood_dir).map_err(|e| e.to_string())?;
                let (characters, index, catalog) =
                    catalog_with_characters(index, &neighborhood, &pool)?;
                let house_file = House::load(house, &catalog).map_err(|e| e.to_string())?;
                // Match `sims_render::open_house` house validation and initial
                // screen-centre pick before consuming `BaseAssets` or VM boot results.
                let mut world = sims_sim::World::from_house(&house_file, &catalog)
                    .map_err(|e| e.to_string())?;
                let version = house_file
                    .version()
                    .ok_or_else(|| "the house file has no HOUS 1 chunk".to_owned())?;
                if let Some(&requested) = world.globals.get(23) {
                    let requested = if requested == 0 {
                        sims_sim::FULL_SIZE
                    } else {
                        requested
                    };
                    if (4..=sims_sim::FULL_SIZE).contains(&requested) {
                        world.size = requested;
                    }
                }
                for id in sims_sim::load::FLOORS
                    .into_iter()
                    .chain(sims_sim::load::WALLS)
                {
                    world.arrays.remove(&id);
                }
                world
                    .read_arrays(&house_file.arrays, version)
                    .map_err(|e| e.to_string())?;
                world.rebuild_rooms().map_err(|e| e.to_string())?;
                let opening_rotation = house_file
                    .view
                    .as_ref()
                    .map_or(0, |v| (v.camera_direction & 3) as u8);
                let initial_centre = sims_render::Lot::from_world(&world).ok().and_then(|shape| {
                    let shape = shape.with_floor_rotation(opening_rotation);
                    let view = View::new([800, 600], shape.built_center(), opening_rotation);
                    let mut camera = view.camera;
                    camera.origin[0] = camera.origin[0].saturating_add(pick::ORIGIN_COLUMN);
                    sims_render::screen_centre_pick(&shape, &camera, [800, 600])
                        .map(|[x, y]| [(x >> 4) as i8, (y >> 4) as i8])
                });
                Ok((
                    neighborhood,
                    characters,
                    index,
                    catalog,
                    house_file,
                    initial_centre,
                ))
            });
            let people_handle = scope.spawn(|| match ContentAssets::scan(&install.root) {
                Ok(content_assets) => {
                    let (wardrobe, library_res) = std::thread::scope(|inner| {
                        let lib_handle =
                            inner.spawn(|| PeopleLibrary::scan_with_pool(&content_assets, &pool));
                        let wardrobe = Wardrobe::load_with_pool(&content_assets, &pool);
                        (
                            wardrobe,
                            lib_handle.join().expect("people library worker panicked"),
                        )
                    });
                    (
                        wardrobe,
                        library_res.map(|people_library| (content_assets, people_library)),
                    )
                }
                Err(error) => (Wardrobe::unread(error.clone()), Err(error)),
            });
            let ordered_ward_handle = scope.spawn(|| load_ordered_wardrobe(&install, &pool));
            let reg_handle = scope.spawn(|| AvatarRegistry::load_with_pool(&install, &pool));
            let surf_handle =
                scope.spawn(|| Surfaces::build_with_plugins_and_pool(&install, &pool));
            let hud_handle = scope.spawn(|| HudStatic::load(&install, Arc::clone(&pool)));
            let base_assets_res = BaseAssets::load(&install).map_err(|e| e.to_string());
            let icon_frames_res = sims_vm::icon_frames(&install);
            let default_name = default_character_name(&install);
            (
                idx_handle.join().expect("index/catalog worker panicked"),
                people_handle.join().expect("people assets worker panicked"),
                ordered_ward_handle
                    .join()
                    .expect("ordered wardrobe worker panicked"),
                reg_handle.join().expect("avatar registry worker panicked"),
                surf_handle.join().expect("surfaces worker panicked"),
                hud_handle.join().expect("hud static worker panicked"),
                base_assets_res,
                icon_frames_res,
                default_name,
            )
        });

        let (neighborhood, characters, index, catalog, house_file, initial_centre) =
            index_catalog_res?;
        let base_roof = house_file
            .view
            .as_ref()
            .map(RoofState::from_house_view)
            .unwrap_or_default();
        let index = Arc::new(index);
        let catalog = Arc::new(catalog);
        let (surfaces_res, floor_plugins, wall_plugins) = match surfaces_res {
            Ok((surfaces, floor_plugins, wall_plugins)) => {
                (Ok(surfaces), floor_plugins, wall_plugins)
            }
            Err(error) => (
                Err(error),
                std::collections::BTreeMap::new(),
                std::collections::BTreeMap::new(),
            ),
        };
        let assets = sims_render::Assets::from_base(
            base_assets_res?,
            floor_plugins,
            wall_plugins,
            Arc::clone(&index),
            Arc::clone(&catalog),
            Arc::clone(&pool),
        );

        // Stage B: link Program, load Animations, resolve person types, and load
        // Index/Catalog-dependent HUD tables in parallel.
        let (program_res, animations_res, people_res, hud_shared_res) =
            std::thread::scope(|scope| {
                let prog_handle = scope.spawn(|| {
                    Program::link_with_pool(
                        &index,
                        &catalog,
                        catalog.objects.iter().map(|o| o.guid),
                        &pool,
                    )
                });
                let anim_handle =
                    scope.spawn(|| Animations::load_with_pool(&install, &index, &pool));
                let hud_shared_handle = scope.spawn(|| HudShared::load(&index, &catalog, &pool));
                let person_types = catalog
                    .objects
                    .iter()
                    .filter(|o| o.kind == 2)
                    .map(|o| o.guid);
                let people_res = ContentPeople::load_each_with_registry_result(
                    &index,
                    &catalog,
                    person_types,
                    registry_res,
                    &pool,
                );
                (
                    prog_handle.join().expect("program link worker panicked"),
                    anim_handle.join().expect("animations worker panicked"),
                    people_res,
                    hud_shared_handle
                        .join()
                        .expect("hud shared worker panicked"),
                )
            });

        let preloaded_cast = people_res
            .as_ref()
            .ok()
            .map(|(people_types, unresolved_people)| PreloadedCast {
                skeleton_names: people_types
                    .skeletons
                    .iter()
                    .map(|(&guid, skeleton)| (guid, skeleton.name.clone()))
                    .collect(),
                unresolved: unresolved_people.clone(),
            });
        // `Lot::boot` keeps a loaded `Wardrobe` even if `PeopleLibrary::scan`
        // fails, and records a `ContentAssets::scan` error in `Wardrobe::unread`;
        // `People::open` reports either error later after `Hud::open`.
        let (wardrobe, people_assets_for_cast) = people_assets_res;

        // Stage C: run the deterministic lot boot from preloaded content, then
        // assemble HUD portraits and 3D people in parallel against the booted world.
        let mut lot = Lot::load_preloaded(
            house,
            &options.answers,
            options.frames.clone(),
            PreloadedBoot {
                install: &install,
                neighborhood: &neighborhood,
                characters: characters.as_deref(),
                index: &index,
                catalog: Arc::clone(&catalog),
                pool: &pool,
                file: Some(house_file),
                program: program_res,
                animations: animations_res,
                people: people_res,
                icon_frames: icon_frames_res,
                wardrobe,
                ordered_wardrobe,
                surfaces: surfaces_res,
                default_character_name: default_name,
                view_centre: initial_centre,
            },
        )
        .map_err(|e| format!("{}: {e}", house.display()))?;
        // The player answers a dialog when it is shown.
        lot.wait_for_answers();
        let stopped = lot
            .load_report()
            .stopped
            .as_ref()
            .map(|s| format!("The load stopped, so the lot cannot run: {s}"));
        // The booted world, not the file as read: some objects (doors,
        // windows) set how they are drawn in their own init scripts.
        let lot_shape = sims_render::Lot::from_world(lot.world())
            .map_err(|e| e.to_string())?
            .with_floor_rotation(lot.floor_rotation());
        let friendship = lot.program().autonomy_constants().friendship_threshold;
        let world = lot.world();
        let (hud_res, people_open_res) = std::thread::scope(|scope| {
            let people_handle = scope.spawn(|| -> Result<People, String> {
                let (content_assets, people_library) = people_assets_for_cast?;
                let cast = Cast::from_shared(
                    install.clone(),
                    Arc::clone(&index),
                    Arc::clone(&catalog),
                    preloaded_cast,
                    Arc::clone(&pool),
                )?;
                let mut people = People::from_inner(sims_people::People::new(
                    content_assets,
                    people_library,
                    cast,
                ));
                people.refresh(world);
                Ok(people)
            });
            let hud_res = hud_static_res.and_then(|hud_static| {
                Hud::from_preloaded(
                    &install,
                    house,
                    &neighborhood,
                    &assets,
                    world,
                    friendship,
                    hud_static,
                    hud_shared_res,
                    &pool,
                )
            });
            (
                hud_res,
                people_handle
                    .join()
                    .expect("people refresh worker panicked"),
            )
        });
        let mut hud = hud_res?;
        hud.sync_clock_blink(lot.world(), 0);
        // `0x4e6704 -> 0x432220`: apply persisted `UserData/Options.iff` Free
        // Will preference to lot global 30 when it differs from the lot file.
        let free_will = hud.controls.options.play.free_will;
        if lot.world().globals.get(30).copied().unwrap_or(1) != i16::from(free_will) {
            let _ = lot.set_free_will(free_will);
        }
        let mut people = people_open_res?;
        // In case `set_free_will` changed anything (it only sets global 30),
        // refreshing is an instant no-op when people are unchanged.
        people.refresh(lot.world());
        let view = View::new([800, 600], lot_shape.built_center(), lot.view_rotation());
        let drawn_camera = view.camera;
        let has_family = lot.world().globals.get(9).copied().unwrap_or(0) != 0;
        // `0x4f1b5f..0x4f1b76`: any vacant lot with nonzero zoning (residential
        // zoning 1 such as `House02.iff` as well as zoning 2) or in the special
        // neighborhood ranges enters edit mode (`global[32] = 1`).
        let edit_mode = lot.world().globals.get(32).copied().unwrap_or(0) == 1
            || (!has_family
                && (lot.lot_zoning().unwrap_or(0) != 0
                    || matches!(lot.lot_id(), 21..=30 | 40..=48 | 81..=99)));
        let mut game = Self {
            lot,
            lot_shape,
            assets,
            view,
            people,
            hud,
            pace: Pace::default(),
            play: options.play,
            stopped,
            errors: Errors::default(),
            sounds: VecDeque::new(),
            ui_effects: VecDeque::new(),
            highlighted_buttons: BTreeSet::new(),
            highlighted_portraits: BTreeSet::new(),
            highlighted_relationships: BTreeSet::new(),
            tutorial_marker: None,
            marker_cell_drawn: None,
            shell_modal: false,
            local_clipboard: None,
            pointer: None,
            photo_point: None,
            preview_reply: None,
            reached: None,
            listed: None,
            highlighted: None,
            buy_selected: None,
            buy_cursor: None,
            buy_in_house: false,
            buy_last_guid: None,
            buy_facing: None,
            buy_dragging: None,
            buy_grab_offset: [0, 0],
            buy_commit_on_release: false,
            live_panel_expanded: true,
            shift_held: false,
            ctrl_held: false,
            wall_cursor: None,
            build_drag: None,
            build_preview_shape: None,
            base_roof,
            roof_catalog: None,
            roof_view: false,
            terrain_hover_overlay: None,
            water_hover_overlay: None,
            shop_dialog: ShopPickerDialog::new(),
            shop_art: None,
            shop_strings: None,
            shop_snapshot: None,
            shop_preview_assets: None,
            shop_preview_yaw_deg: 0,
            shop_preview_body: None,
            shop_preview_targets: BTreeMap::new(),
            purchased_shop_items: BTreeSet::new(),
            book_dialog: ReferenceBookDialog::new(),
            book_art: None,
            book_strings: None,
            book_snapshot: None,
            phonebook_dialog: PhoneBookDialog::new(),
            phonebook_art: None,
            phonebook_strings: None,
            phonebook_snapshot: None,
            destination_picker: None,
            destination_picker_assets: None,
            neighborhood_slot: 1,
            active_special_panel: None,
            submitted_coop_special_panel: None,
            ordered_wardrobe: None,
            pending_travel_lot: None,
            declined_trip: None,
            special_lot_catalog: Vec::new(),
            ui_animating_prev: false,
            voice_inputs: VecDeque::new(),
            buy_moneyback_us: None,
            buy_delete_us: None,
            note: None,
            clock_us: 0,
            drawn_us: 0,
            blink_dirty: false,
            dirty: false,
            restore_dirty_on_quit_cancel: false,
            pending_after_save_note: None,
            pending_after_notice: None,
            edit_mode,
            edge_scroll_rem: [0, 0],
            canvas: Canvas::new(1, 1, [0, 0, 0]),
            lot_cache: sims_render::LotCache::new(),
            drawn_camera,
            lot_depth: Vec::new(),
            lot_depth_from_cache: false,
            people_ids: sims_render::Ids::default(),
            people_rects: Vec::new(),
            prev_dynamic_rects: Vec::new(),
            prev_steady_key: None,
            present_damage: None,
            pie_animations: false,
            debug: false,
            last_debug_logged_note: None,
            smooth_camera_pan: false,
            camera_pan: None,
            camera_pan_moved: false,
            cheat_map_edit: false,
            cheat_autonomy: None,
            cheat_sim_speed: None,
            world_mutated_via_restore: false,
            restore_generation: 0,
            frames: options.frames.clone(),
            coop: CoopController::new(),
            coop_lobby: CoopLobbyDialog::new(),
            coop_art: None,
            coop_player_name: default_player_name("Player"),
            coop_iroh_config: IrohConfig::P2pDefault,
            coop_travel_blocked: false,
            coop_remote_world: false,
            coop_overlay_buttons: CoopOverlayButtons::default(),
            coop_buy_preview: None,
            world_sound_rev: 0,
            open_pack_mode: false,
            _open_pack_staging: None,
            open_pack_wavs: None,
            open_pack_save_slot: None,
        };
        game.sync_view();
        if edit_mode && game.stopped.is_none() {
            // `0x4f1bd9..0x4f1be5`, `0x4f2510..0x4f2520`, `0x46d8c9`:
            // unoccupied edit-mode lots enable budget mode (global 32 = 1 ->
            // §999,999) and enter Buy mode out of Live mode.
            game.lot.set_edit_budget(true);
            game.set_mode(Mode::Buy);
        }
        Ok(game)
    }

    /// Boots `house` from a REDB [`sims_sim::neighborhood_store::NeighborhoodStore`]
    /// via [`sims_vm::lifecycle::enter_lot`], without reading or mutating any
    /// `UserData` house files on disk.
    #[allow(clippy::too_many_arguments)]
    pub fn open_from_store(
        install_root: &Path,
        store: &sims_sim::neighborhood_store::NeighborhoodStore,
        neighborhood_id: &str,
        house: u32,
        answers: &[u8],
        play: bool,
        frames: Frames,
    ) -> Result<Self, String> {
        Self::open_from_store_cached(
            install_root,
            store,
            neighborhood_id,
            house,
            answers,
            play,
            frames,
            None,
        )
    }

    /// Like [`Game::open_from_store`], reusing preloaded install-wide
    /// [`InstallAssets`] across lot transitions when `cached` is `Some`.
    #[allow(clippy::too_many_arguments)]
    pub fn open_from_store_cached(
        install_root: &Path,
        store: &sims_sim::neighborhood_store::NeighborhoodStore,
        neighborhood_id: &str,
        house: u32,
        answers: &[u8],
        play: bool,
        frames: Frames,
        cached: Option<&Arc<InstallAssets>>,
    ) -> Result<Self, String> {
        let snapshot = store
            .load_lot_snapshot(neighborhood_id, house)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {neighborhood_id:?} not found in store"))?;
        let members = snapshot
            .neighborhood
            .family_in(house)
            .map(|(_, f)| f.members.clone())
            .unwrap_or_default();
        let pool = cached.map_or_else(|| Arc::new(ArchivePool::new()), |c| Arc::clone(&c.pool));
        let character_files = &snapshot.character_files;
        let (lot_res, render_and_index_res, hud_static_res, people_assets_res) =
            std::thread::scope(|scope| {
                let assets_handle =
                    scope.spawn(|| load_lot_assets(install_root, character_files, &pool, cached));
                let hud_handle = scope.spawn(|| load_hud_static(install_root, &pool, cached));
                let people_handle = scope.spawn(|| load_people_assets(install_root, &pool, cached));
                let lot_res = match cached {
                    Some(shared) => sims_vm::lifecycle::enter_lot_with_cache(
                        &shared.vm_cache,
                        store,
                        neighborhood_id,
                        house,
                        answers,
                        frames.clone(),
                    ),
                    None => sims_vm::lifecycle::enter_lot(
                        install_root,
                        store,
                        neighborhood_id,
                        house,
                        answers,
                        frames.clone(),
                    ),
                }
                .map_err(|e| format!("{neighborhood_id} House{house:02}: {e}"));
                (
                    lot_res,
                    assets_handle.join().expect("store assets worker panicked"),
                    hud_handle.join().expect("hud static worker panicked"),
                    people_handle.join().expect("people assets worker panicked"),
                )
            });
        let mut lot = lot_res?;
        lot.wait_for_answers();
        let stopped = lot
            .load_report()
            .stopped
            .as_ref()
            .map(|s| format!("The load stopped, so the lot cannot run: {s}"));
        let (install, index, catalog, hud_shared_res, base_assets, floor_plugins, wall_plugins) =
            render_and_index_res?;
        let assets = sims_render::Assets::from_base(
            base_assets,
            floor_plugins,
            wall_plugins,
            Arc::clone(&index),
            Arc::clone(&catalog),
            Arc::clone(&pool),
        );
        let lot_shape = sims_render::Lot::from_world(lot.world())
            .map_err(|e| e.to_string())?
            .with_floor_rotation(lot.floor_rotation());
        let friendship = lot.program().autonomy_constants().friendship_threshold;
        let world = lot.world();
        let (hud_res, people_open_res) = std::thread::scope(|scope| {
            let people_handle = scope.spawn(|| -> Result<People, String> {
                let (content_assets, people_library) = people_assets_res?;
                let preloaded_cast = cached.and_then(|shared| {
                    shared.preloaded_cast_for(&index, &catalog, !character_files.is_empty())
                });
                let cast = Cast::from_shared(
                    install.clone(),
                    Arc::clone(&index),
                    Arc::clone(&catalog),
                    preloaded_cast,
                    Arc::clone(&pool),
                )?;
                let mut people = People::from_inner(sims_people::People::new(
                    content_assets,
                    people_library,
                    cast,
                ));
                people.refresh(world);
                Ok(people)
            });
            let hud_res = hud_static_res.and_then(|hud_static| {
                Hud::from_preloaded_with_members(
                    &install,
                    members,
                    &assets,
                    world,
                    friendship,
                    hud_static,
                    hud_shared_res,
                    &pool,
                )
            });
            (
                hud_res,
                people_handle
                    .join()
                    .expect("people refresh worker panicked"),
            )
        });
        let mut hud = hud_res?;
        hud.sync_clock_blink(lot.world(), 0);
        let free_will = hud.controls.options.play.free_will;
        if lot.world().globals.get(30).copied().unwrap_or(1) != i16::from(free_will) {
            let _ = lot.set_free_will(free_will);
        }
        let people = people_open_res?;
        let view = View::new([800, 600], lot_shape.built_center(), lot.view_rotation());
        let drawn_camera = view.camera;
        let has_family = lot.world().globals.get(9).copied().unwrap_or(0) != 0;
        let is_residential = lot.lot_zoning().unwrap_or(1) == 1
            && !matches!(lot.lot_id(), 21..=30 | 40..=48 | 81..=89 | 93..=98);
        let edit_mode = if has_family && is_residential {
            false
        } else {
            lot.world().globals.get(32).copied().unwrap_or(0) == 1
                || (!has_family
                    && (lot.lot_zoning().unwrap_or(0) != 0
                        || matches!(lot.lot_id(), 21..=30 | 40..=48 | 81..=99)))
        };
        let mut pace = Pace::default();
        if let Some(speed) = pace::shown(lot.world()) {
            pace.speed = speed;
        }
        let base_roof = snapshot
            .base_house
            .as_deref()
            .and_then(parse_base_roof_from_house_bytes)
            .unwrap_or_default();
        let mut game = Self {
            lot,
            lot_shape,
            assets,
            view,
            people,
            hud,
            pace,
            play,
            stopped,
            errors: Errors::default(),
            sounds: VecDeque::new(),
            ui_effects: VecDeque::new(),
            highlighted_buttons: BTreeSet::new(),
            highlighted_portraits: BTreeSet::new(),
            highlighted_relationships: BTreeSet::new(),
            tutorial_marker: None,
            marker_cell_drawn: None,
            shell_modal: false,
            local_clipboard: None,
            pointer: None,
            photo_point: None,
            preview_reply: None,
            reached: None,
            listed: None,
            highlighted: None,
            buy_selected: None,
            buy_cursor: None,
            buy_in_house: false,
            buy_last_guid: None,
            buy_facing: None,
            buy_dragging: None,
            buy_grab_offset: [0, 0],
            buy_commit_on_release: false,
            live_panel_expanded: true,
            shift_held: false,
            ctrl_held: false,
            wall_cursor: None,
            build_drag: None,
            build_preview_shape: None,
            base_roof,
            roof_catalog: None,
            roof_view: false,
            terrain_hover_overlay: None,
            water_hover_overlay: None,
            shop_dialog: ShopPickerDialog::new(),
            shop_art: None,
            shop_strings: None,
            shop_snapshot: None,
            shop_preview_assets: None,
            shop_preview_yaw_deg: 0,
            shop_preview_body: None,
            shop_preview_targets: BTreeMap::new(),
            purchased_shop_items: BTreeSet::new(),
            book_dialog: ReferenceBookDialog::new(),
            book_art: None,
            book_strings: None,
            book_snapshot: None,
            phonebook_dialog: PhoneBookDialog::new(),
            phonebook_art: None,
            phonebook_strings: None,
            phonebook_snapshot: None,
            destination_picker: None,
            destination_picker_assets: None,
            neighborhood_slot: 1,
            active_special_panel: None,
            submitted_coop_special_panel: None,
            ordered_wardrobe: None,
            pending_travel_lot: None,
            declined_trip: None,
            special_lot_catalog: Vec::new(),
            ui_animating_prev: false,
            voice_inputs: VecDeque::new(),
            buy_moneyback_us: None,
            buy_delete_us: None,
            note: None,
            clock_us: 0,
            drawn_us: 0,
            blink_dirty: false,
            dirty: false,
            restore_dirty_on_quit_cancel: false,
            pending_after_save_note: None,
            pending_after_notice: None,
            edit_mode,
            edge_scroll_rem: [0, 0],
            canvas: Canvas::new(1, 1, [0, 0, 0]),
            lot_cache: sims_render::LotCache::new(),
            drawn_camera,
            lot_depth: Vec::new(),
            lot_depth_from_cache: false,
            people_ids: sims_render::Ids::default(),
            people_rects: Vec::new(),
            prev_dynamic_rects: Vec::new(),
            prev_steady_key: None,
            present_damage: None,
            pie_animations: false,
            debug: false,
            last_debug_logged_note: None,
            smooth_camera_pan: false,
            camera_pan: None,
            camera_pan_moved: false,
            cheat_map_edit: false,
            cheat_autonomy: None,
            cheat_sim_speed: None,
            world_mutated_via_restore: false,
            restore_generation: 0,
            frames,
            coop: CoopController::new(),
            coop_lobby: CoopLobbyDialog::new(),
            coop_art: None,
            coop_player_name: default_player_name("Player"),
            coop_iroh_config: IrohConfig::P2pDefault,
            coop_travel_blocked: false,
            coop_remote_world: false,
            coop_overlay_buttons: CoopOverlayButtons::default(),
            coop_buy_preview: None,
            world_sound_rev: 0,
            open_pack_mode: false,
            _open_pack_staging: None,
            open_pack_wavs: None,
            open_pack_save_slot: None,
        };
        game.sync_view();
        if edit_mode && game.stopped.is_none() {
            game.lot.set_edit_budget(true);
            game.set_mode(Mode::Buy);
        }
        Ok(game)
    }

    /// Wraps an already-booted [`Lot`] (e.g. from `sims_vm::travel::depart_to_destination`,
    /// `resume_active_trip`, `return_from_destination`, or `open_destination_for_edit`)
    /// into a live [`Game`] session without re-booting the VM.
    pub fn from_loaded_lot(
        install_root: &Path,
        lot: Lot,
        members: Vec<u32>,
        character_files: &BTreeMap<String, Vec<u8>>,
        base_house_bytes: Option<&[u8]>,
        play: bool,
    ) -> Result<Self, String> {
        Self::from_loaded_lot_cached(
            install_root,
            lot,
            members,
            character_files,
            base_house_bytes,
            play,
            None,
        )
    }

    /// Like [`Game::from_loaded_lot`], reusing preloaded install-wide
    /// [`InstallAssets`] when `cached` is `Some`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_loaded_lot_cached(
        install_root: &Path,
        mut lot: Lot,
        members: Vec<u32>,
        character_files: &BTreeMap<String, Vec<u8>>,
        base_house_bytes: Option<&[u8]>,
        play: bool,
        cached: Option<&Arc<InstallAssets>>,
    ) -> Result<Self, String> {
        lot.wait_for_answers();
        // A household arriving by travel sets one of the flags +0x140,
        // +0x141 or +0x142 (0x4fc59b), and the lot load then skips edit mode
        // (0x4f1b7c..0x4f1b9d jump past it to 0x4f1c32). A solo arrival
        // leaves global 9 at 0, so the flag, not global 9, decides.
        let household_visit = !members.is_empty();
        let stopped = lot
            .load_report()
            .stopped
            .as_ref()
            .map(|s| format!("The load stopped, so the lot cannot run: {s}"));
        let pool = cached.map_or_else(|| Arc::new(ArchivePool::new()), |c| Arc::clone(&c.pool));
        let (render_and_index_res, hud_static_res, people_assets_res) =
            std::thread::scope(|scope| {
                let assets_handle =
                    scope.spawn(|| load_lot_assets(install_root, character_files, &pool, cached));
                let hud_handle = scope.spawn(|| load_hud_static(install_root, &pool, cached));
                let people_handle = scope.spawn(|| load_people_assets(install_root, &pool, cached));
                (
                    assets_handle
                        .join()
                        .expect("loaded lot assets worker panicked"),
                    hud_handle.join().expect("hud static worker panicked"),
                    people_handle.join().expect("people assets worker panicked"),
                )
            });
        let (install, index, catalog, hud_shared_res, base_assets, floor_plugins, wall_plugins) =
            render_and_index_res?;
        let assets = sims_render::Assets::from_base(
            base_assets,
            floor_plugins,
            wall_plugins,
            Arc::clone(&index),
            Arc::clone(&catalog),
            Arc::clone(&pool),
        );
        let lot_shape = sims_render::Lot::from_world(lot.world())
            .map_err(|e| e.to_string())?
            .with_floor_rotation(lot.floor_rotation());
        let friendship = lot.program().autonomy_constants().friendship_threshold;
        let world = lot.world();
        let (hud_res, people_open_res) = std::thread::scope(|scope| {
            let people_handle = scope.spawn(|| -> Result<People, String> {
                let (content_assets, people_library) = people_assets_res?;
                let preloaded_cast = cached.and_then(|shared| {
                    shared.preloaded_cast_for(&index, &catalog, !character_files.is_empty())
                });
                let cast = Cast::from_shared(
                    install.clone(),
                    Arc::clone(&index),
                    Arc::clone(&catalog),
                    preloaded_cast,
                    Arc::clone(&pool),
                )?;
                let mut people = People::from_inner(sims_people::People::new(
                    content_assets,
                    people_library,
                    cast,
                ));
                people.refresh(world);
                Ok(people)
            });
            let hud_res = hud_static_res.and_then(|hud_static| {
                Hud::from_preloaded_with_members(
                    &install,
                    members,
                    &assets,
                    world,
                    friendship,
                    hud_static,
                    hud_shared_res,
                    &pool,
                )
            });
            (
                hud_res,
                people_handle
                    .join()
                    .expect("people refresh worker panicked"),
            )
        });
        let mut hud = hud_res?;
        hud.sync_clock_blink(lot.world(), 0);
        let free_will = hud.controls.options.play.free_will;
        if lot.world().globals.get(30).copied().unwrap_or(1) != i16::from(free_will) {
            let _ = lot.set_free_will(free_will);
        }
        let people = people_open_res?;
        let view = View::new([800, 600], lot_shape.built_center(), lot.view_rotation());
        let drawn_camera = view.camera;
        let has_family = household_visit || lot.world().globals.get(9).copied().unwrap_or(0) != 0;
        let edit_mode = lot.world().globals.get(32).copied().unwrap_or(0) == 1
            || (!has_family
                && (lot.lot_zoning().unwrap_or(0) != 0
                    || matches!(lot.lot_id(), 21..=30 | 40..=48 | 81..=99)));
        let mut pace = Pace::default();
        if let Some(speed) = pace::shown(lot.world()) {
            pace.speed = speed;
        }
        let base_roof = base_house_bytes
            .and_then(parse_base_roof_from_house_bytes)
            .unwrap_or_default();
        let mut game = Self {
            lot,
            lot_shape,
            assets,
            view,
            people,
            hud,
            pace,
            play,
            stopped,
            errors: Errors::default(),
            sounds: VecDeque::new(),
            ui_effects: VecDeque::new(),
            highlighted_buttons: BTreeSet::new(),
            highlighted_portraits: BTreeSet::new(),
            highlighted_relationships: BTreeSet::new(),
            tutorial_marker: None,
            marker_cell_drawn: None,
            shell_modal: false,
            local_clipboard: None,
            pointer: None,
            photo_point: None,
            preview_reply: None,
            reached: None,
            listed: None,
            highlighted: None,
            buy_selected: None,
            buy_cursor: None,
            buy_in_house: false,
            buy_last_guid: None,
            buy_facing: None,
            buy_dragging: None,
            buy_grab_offset: [0, 0],
            buy_commit_on_release: false,
            live_panel_expanded: true,
            shift_held: false,
            ctrl_held: false,
            wall_cursor: None,
            build_drag: None,
            build_preview_shape: None,
            base_roof,
            roof_catalog: None,
            roof_view: false,
            terrain_hover_overlay: None,
            water_hover_overlay: None,
            shop_dialog: ShopPickerDialog::new(),
            shop_art: None,
            shop_strings: None,
            shop_snapshot: None,
            shop_preview_assets: None,
            shop_preview_yaw_deg: 0,
            shop_preview_body: None,
            shop_preview_targets: BTreeMap::new(),
            purchased_shop_items: BTreeSet::new(),
            book_dialog: ReferenceBookDialog::new(),
            book_art: None,
            book_strings: None,
            book_snapshot: None,
            phonebook_dialog: PhoneBookDialog::new(),
            phonebook_art: None,
            phonebook_strings: None,
            phonebook_snapshot: None,
            destination_picker: None,
            destination_picker_assets: None,
            neighborhood_slot: 1,
            active_special_panel: None,
            submitted_coop_special_panel: None,
            ordered_wardrobe: None,
            pending_travel_lot: None,
            declined_trip: None,
            special_lot_catalog: Vec::new(),
            ui_animating_prev: false,
            voice_inputs: VecDeque::new(),
            buy_moneyback_us: None,
            buy_delete_us: None,
            note: None,
            clock_us: 0,
            drawn_us: 0,
            blink_dirty: false,
            dirty: false,
            restore_dirty_on_quit_cancel: false,
            pending_after_save_note: None,
            pending_after_notice: None,
            edit_mode,
            edge_scroll_rem: [0, 0],
            canvas: Canvas::new(1, 1, [0, 0, 0]),
            lot_cache: sims_render::LotCache::new(),
            drawn_camera,
            lot_depth: Vec::new(),
            lot_depth_from_cache: false,
            people_ids: sims_render::Ids::default(),
            people_rects: Vec::new(),
            prev_dynamic_rects: Vec::new(),
            prev_steady_key: None,
            present_damage: None,
            pie_animations: false,
            debug: false,
            last_debug_logged_note: None,
            smooth_camera_pan: false,
            camera_pan: None,
            camera_pan_moved: false,
            cheat_map_edit: false,
            cheat_autonomy: None,
            cheat_sim_speed: None,
            world_mutated_via_restore: false,
            restore_generation: 0,
            frames: Frames::default(),
            coop: CoopController::new(),
            coop_lobby: CoopLobbyDialog::new(),
            coop_art: None,
            coop_player_name: default_player_name("Player"),
            coop_iroh_config: IrohConfig::P2pDefault,
            coop_travel_blocked: false,
            coop_remote_world: false,
            coop_overlay_buttons: CoopOverlayButtons::default(),
            coop_buy_preview: None,
            world_sound_rev: 0,
            open_pack_mode: false,
            _open_pack_staging: None,
            open_pack_wavs: None,
            open_pack_save_slot: None,
        };
        game.sync_view();
        if edit_mode && game.stopped.is_none() {
            game.lot.set_edit_budget(true);
            game.set_mode(Mode::Buy);
        }
        Ok(game)
    }

    /// Compiles and boots a clean-room standalone `open-pack/` directory with
    /// zero commercial EA/Maxis assets required.
    ///
    /// Uses `sims_antics::compile_standalone_pack_dir` to build all 20 objects,
    /// 4 `.flr` floors, 4 `.wll` walls, `House01.iff`, `Neighborhood.iff`,
    /// starter Sims, and synthesized PCM WAV bank, paired with
    /// [`sims_ui::art::Art::clean_room_procedural`] and 2D clean-room Sim
    /// sprite rendering.
    pub fn open_open_pack(
        pack_dir: &Path,
        answers: &[u8],
        play: bool,
        frames: Frames,
    ) -> Result<Self, String> {
        let staged = crate::open_pack::stage_open_pack(pack_dir)?;
        let install_root = staged.staging_dir.path();
        let house = &staged.house_path;
        let neighborhood_dir = house
            .parent()
            .and_then(Path::parent)
            .ok_or("staged open-pack house is not in a Houses folder")?;
        let install = Install::detect(install_root).map_err(|e| e.to_string())?;
        let neighborhood = Neighborhood::open(neighborhood_dir).map_err(|e| e.to_string())?;
        let pool = Arc::new(ArchivePool::new());
        let index = Index::build(&install).map_err(|e| e.to_string())?;
        let (_, index, catalog) = catalog_with_characters(index, &neighborhood, &pool)?;
        let catalog = Arc::new(catalog);
        let index = Arc::new(index);
        let base_assets = BaseAssets::load(&install).map_err(|e| e.to_string())?;
        let (floor_plugins, wall_plugins) = Surfaces::plugins_with_pool(&install, &pool);

        let mut lot = Lot::load_open_pack(install_root, house, answers, frames.clone())
            .map_err(|e| format!("{}: {e}", house.display()))?;
        lot.wait_for_answers();
        let stopped = lot
            .load_report()
            .stopped
            .as_ref()
            .map(|s| format!("The load stopped, so the lot cannot run: {s}"));

        let assets = sims_render::Assets::from_base(
            base_assets,
            floor_plugins,
            wall_plugins,
            Arc::clone(&index),
            Arc::clone(&catalog),
            Arc::clone(&pool),
        );
        let lot_shape = sims_render::Lot::from_world(lot.world())
            .map_err(|e| e.to_string())?
            .with_floor_rotation(lot.floor_rotation());
        let friendship = lot.program().autonomy_constants().friendship_threshold;
        let members: Vec<u32> = staged.starter_sims.iter().map(|s| s.guid).collect();
        let mut hud = Hud::open_open_pack(
            &install,
            members,
            &staged.starter_sims,
            &assets,
            lot.world(),
            friendship,
            &pool,
        )?;
        hud.sync_clock_blink(lot.world(), 0);
        let content_assets = ContentAssets::scan(install_root)?;
        let people_library = PeopleLibrary::scan_with_pool(&content_assets, &pool)?;
        let cast = Cast::from_shared(
            install,
            Arc::clone(&index),
            Arc::clone(&catalog),
            None,
            Arc::clone(&pool),
        )?;
        let people = People::from_inner(sims_people::People::new(
            content_assets,
            people_library,
            cast,
        ));
        let view = View::new([800, 600], lot_shape.built_center(), lot.view_rotation());
        let drawn_camera = view.camera;

        let mut game = Self {
            lot,
            lot_shape,
            assets,
            view,
            people,
            hud,
            pace: Pace::default(),
            play,
            stopped,
            errors: Errors::default(),
            sounds: VecDeque::new(),
            ui_effects: VecDeque::new(),
            highlighted_buttons: BTreeSet::new(),
            highlighted_portraits: BTreeSet::new(),
            highlighted_relationships: BTreeSet::new(),
            tutorial_marker: None,
            marker_cell_drawn: None,
            shell_modal: false,
            local_clipboard: None,
            pointer: None,
            photo_point: None,
            preview_reply: None,
            reached: None,
            listed: None,
            highlighted: None,
            buy_selected: None,
            buy_cursor: None,
            buy_in_house: false,
            buy_last_guid: None,
            buy_facing: None,
            buy_dragging: None,
            buy_grab_offset: [0, 0],
            buy_commit_on_release: false,
            live_panel_expanded: true,
            shift_held: false,
            ctrl_held: false,
            wall_cursor: None,
            build_drag: None,
            build_preview_shape: None,
            base_roof: RoofState::default(),
            roof_catalog: None,
            roof_view: false,
            terrain_hover_overlay: None,
            water_hover_overlay: None,
            shop_dialog: ShopPickerDialog::new(),
            shop_art: None,
            shop_strings: None,
            shop_snapshot: None,
            shop_preview_assets: None,
            shop_preview_yaw_deg: 0,
            shop_preview_body: None,
            shop_preview_targets: BTreeMap::new(),
            purchased_shop_items: BTreeSet::new(),
            book_dialog: ReferenceBookDialog::new(),
            book_art: None,
            book_strings: None,
            book_snapshot: None,
            phonebook_dialog: PhoneBookDialog::new(),
            phonebook_art: None,
            phonebook_strings: None,
            phonebook_snapshot: None,
            destination_picker: None,
            destination_picker_assets: None,
            neighborhood_slot: 1,
            active_special_panel: None,
            submitted_coop_special_panel: None,
            ordered_wardrobe: None,
            pending_travel_lot: None,
            declined_trip: None,
            special_lot_catalog: Vec::new(),
            ui_animating_prev: false,
            voice_inputs: VecDeque::new(),
            buy_moneyback_us: None,
            buy_delete_us: None,
            note: None,
            clock_us: 0,
            drawn_us: 0,
            blink_dirty: false,
            dirty: false,
            restore_dirty_on_quit_cancel: false,
            pending_after_save_note: None,
            pending_after_notice: None,
            edit_mode: false,
            edge_scroll_rem: [0, 0],
            canvas: Canvas::new(1, 1, [0, 0, 0]),
            lot_cache: sims_render::LotCache::new(),
            drawn_camera,
            lot_depth: Vec::new(),
            lot_depth_from_cache: false,
            people_ids: sims_render::Ids::default(),
            people_rects: Vec::new(),
            prev_dynamic_rects: Vec::new(),
            prev_steady_key: None,
            present_damage: None,
            pie_animations: false,
            debug: false,
            last_debug_logged_note: None,
            smooth_camera_pan: false,
            camera_pan: None,
            camera_pan_moved: false,
            cheat_map_edit: false,
            cheat_autonomy: None,
            cheat_sim_speed: None,
            world_mutated_via_restore: false,
            restore_generation: 0,
            frames,
            coop: CoopController::new(),
            coop_lobby: CoopLobbyDialog::new(),
            coop_art: None,
            coop_player_name: default_player_name("Player"),
            coop_iroh_config: IrohConfig::P2pDefault,
            coop_travel_blocked: false,
            coop_remote_world: false,
            coop_overlay_buttons: CoopOverlayButtons::default(),
            coop_buy_preview: None,
            world_sound_rev: 0,
            open_pack_mode: true,
            _open_pack_staging: Some(staged.staging_dir),
            open_pack_wavs: Some(staged.sounds_wav),
            open_pack_save_slot: Some(staged.save_slot),
        };
        game.sync_view();
        Ok(game)
    }

    /// Whether this game is running in standalone clean-room `open-pack/` mode.
    pub fn is_open_pack(&self) -> bool {
        self.open_pack_mode
    }

    /// Stable save slot key for standalone clean-room `open-pack/` mode,
    /// derived from the source pack directory and lot manifest.
    pub fn open_pack_save_slot(&self) -> Option<&str> {
        self.open_pack_save_slot.as_deref()
    }

    /// Synthesized PCM WAV sound bank loaded from `open-pack/`, when running
    /// in standalone clean-room mode.
    pub fn open_pack_sounds(&self) -> Option<&BTreeMap<String, Vec<u8>>> {
        self.open_pack_wavs.as_ref()
    }

    /// Saves the active lot back to its REDB [`sims_sim::neighborhood_store::NeighborhoodStore`]
    /// via [`sims_vm::lifecycle::save_and_leave_lot`] (stripping any unpaid
    /// Buy Mode placement cursor first).
    pub fn save_to_neighborhood(
        &mut self,
        install_root: &Path,
        store: &sims_sim::neighborhood_store::NeighborhoodStore,
        neighborhood_id: &str,
    ) -> Result<sims_vm::lifecycle::SaveReport, String> {
        if self.coop_blocks_local_save() {
            let msg = "Only the co-op host can save a local lot (remote guest lots are read-only).";
            self.note = Some(msg.into());
            return Err(msg.into());
        }
        let result = match self.buy_cursor.as_ref() {
            Some(cursor) => sims_vm::lifecycle::save_and_leave_lot_without_buy_cursor(
                install_root,
                store,
                neighborhood_id,
                &mut self.lot,
                cursor,
            ),
            None => sims_vm::lifecycle::save_and_leave_lot(
                install_root,
                store,
                neighborhood_id,
                &mut self.lot,
            ),
        }
        .map_err(|e| e.to_string());
        let followup = self.pending_after_save_note.take();
        match &result {
            Ok(_) => {
                self.dirty = false;
                self.note = Some(match followup {
                    Some(extra) => format!("Saved after tick {}. {extra}", self.lot.ticks()),
                    None => format!("Saved after tick {}.", self.lot.ticks()),
                });
            }
            Err(e) => {
                self.note = Some(format!("The lot was not saved: {e}"));
            }
        }
        result
    }
}

/// Adds `neighborhood`'s Characters folder (returned when it has one) to
/// the install's `index`, and builds its catalog.
fn catalog_with_characters(
    mut index: Index,
    neighborhood: &Neighborhood,
    pool: &ArchivePool,
) -> Result<(Option<PathBuf>, Index, Catalog), String> {
    let characters = neighborhood.characters().map_err(|e| e.to_string())?;
    if let Some(characters) = &characters {
        index
            .add_characters(characters)
            .map_err(|e| e.to_string())?;
    }
    let catalog = Catalog::build_with_pool(&index, pool).map_err(|e| e.to_string())?;
    Ok((characters, index, catalog))
}

/// What a lot's render and HUD need from the install: the install, its
/// index and catalog with the lot's character files added, the shared HUD
/// data, the base render assets, and the floor and wall plugins.
type LotAssets = (
    Install,
    Arc<Index>,
    Arc<Catalog>,
    Result<HudShared, String>,
    BaseAssets,
    BTreeMap<u16, SurfacePlugin>,
    BTreeMap<u16, SurfacePlugin>,
);

/// Loads [`LotAssets`], from `cached` when it is `Some` (building a
/// catalog only when the lot adds character files) and from the install
/// at `install_root` otherwise.
fn load_lot_assets(
    install_root: &Path,
    character_files: &BTreeMap<String, Vec<u8>>,
    pool: &Arc<ArchivePool>,
    cached: Option<&Arc<InstallAssets>>,
) -> Result<LotAssets, String> {
    if let Some(shared) = cached {
        let (idx_cat_res, base_res) = std::thread::scope(|inner| {
            let base_handle =
                inner.spawn(|| BaseAssets::load(&shared.install).map_err(|e| e.to_string()));
            let idx_cat_res = if character_files.is_empty() {
                Ok((
                    Arc::clone(&shared.base_index),
                    Arc::clone(&shared.base_catalog),
                ))
            } else {
                (|| -> Result<_, String> {
                    let mut index = (*shared.base_index).clone();
                    index
                        .add_character_files(character_files)
                        .map_err(|e| e.to_string())?;
                    let catalog = Catalog::build_with_pool(&index, &shared.pool)
                        .map_err(|e| e.to_string())?;
                    Ok((Arc::new(index), Arc::new(catalog)))
                })()
            };
            (
                idx_cat_res,
                base_handle.join().expect("base assets worker panicked"),
            )
        });
        let (index, catalog) = idx_cat_res?;
        let base = base_res?;
        return Ok((
            shared.install.clone(),
            index,
            catalog,
            Ok(shared.hud_shared.clone()),
            base,
            shared.floor_plugins.clone(),
            shared.wall_plugins.clone(),
        ));
    }
    let install = Install::detect(install_root).map_err(|e| e.to_string())?;
    let (idx_cat_res, base_res, (floor_plugins, wall_plugins)) = std::thread::scope(|inner| {
        let plugins_handle = inner.spawn(|| Surfaces::plugins_with_pool(&install, pool));
        let base_handle = inner.spawn(|| BaseAssets::load(&install).map_err(|e| e.to_string()));
        let idx_cat_res = (|| -> Result<_, String> {
            let mut index = Index::build(&install).map_err(|e| e.to_string())?;
            if !character_files.is_empty() {
                index
                    .add_character_files(character_files)
                    .map_err(|e| e.to_string())?;
            }
            let catalog = Catalog::build_with_pool(&index, pool).map_err(|e| e.to_string())?;
            let index = Arc::new(index);
            let catalog = Arc::new(catalog);
            let hud_shared_res = HudShared::load(&index, &catalog, pool);
            Ok((index, catalog, hud_shared_res))
        })();
        (
            idx_cat_res,
            base_handle.join().expect("base assets worker panicked"),
            plugins_handle
                .join()
                .expect("surface plugins worker panicked"),
        )
    });
    let (index, catalog, hud_shared_res) = idx_cat_res?;
    let base = base_res?;
    Ok((
        install,
        index,
        catalog,
        hud_shared_res,
        base,
        floor_plugins,
        wall_plugins,
    ))
}

/// Loads the HUD's static art, from `cached` when it is `Some`.
fn load_hud_static(
    install_root: &Path,
    pool: &Arc<ArchivePool>,
    cached: Option<&Arc<InstallAssets>>,
) -> Result<HudStatic, String> {
    if let Some(shared) = cached {
        return Ok(shared.hud_static.clone());
    }
    let install = Install::detect(install_root).map_err(|e| e.to_string())?;
    HudStatic::load(&install, Arc::clone(pool))
}

/// Scans the install's content and people library, from `cached` when it
/// is `Some`.
fn load_people_assets(
    install_root: &Path,
    pool: &Arc<ArchivePool>,
    cached: Option<&Arc<InstallAssets>>,
) -> Result<(ContentAssets, PeopleLibrary), String> {
    if let Some(shared) = cached {
        return Ok((shared.content_assets.clone(), shared.people_library.clone()));
    }
    let content_assets = ContentAssets::scan(install_root)?;
    let people_library = PeopleLibrary::scan_with_pool(&content_assets, pool)?;
    Ok((content_assets, people_library))
}
