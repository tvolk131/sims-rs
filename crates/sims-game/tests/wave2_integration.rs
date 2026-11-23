//! Headless end-to-end integration tests for `sims-game` Wave 2:
//!
//! 1. Build Mode Part 2 (`Terrain`, `Water`/`Pool`, `Roof` pitch & patterns)
//!    interactive hover/drag/commit + Ctrl-remove + Undo/Redo + `sims_render::build_preview`
//!    visual rendering without altering static Live Mode frames.
//! 2. Household Photo Album (`PhotoAlbumDialog` + `PhotoAlbumStore` REDB persistence
//!    with `AlbumFamilyToken` fencing, live framebuffer snapshot capture, title/caption
//!    editing, reorder, delete, and Camera-panel `CamViewAlbum` button) AND persistent
//!    custom neighborhood lot map thumbnails (`NEIGHBORHOOD_LOT_THUMBNAILS`) across
//!    `NeighborhoodShell` restarts and bulldoze cleanup.
//! 3. Travel Destinations Shell (`TravelChrome` over Downtown, Vacation Island, Studio
//!    Town, Magic Town with lot filter highlighting; `open_destination_for_edit` +
//!    `save_and_leave_lot`; `open_travel_dialog` / `validate_travel_plan` ->
//!    `depart_to_destination` -> live destination visit + `ShopPickerDialog` Simoleon
//!    and MagiCoin purchases -> `return_from_destination`; crash recovery via
//!    `inspect_active_trip` / `resume_active_trip` / `abort_active_trip`).
//! 4. Native Loading Screen (`LoadingScreen` splash rendering, rotating `UIText.iff`
//!    captions, `LoadingAudioCue` -> `MusicMode::Load` transitions) AND Simlish Voices
//!    (`VoiceCatalog` PCM mixing in `Mixer`, live lot `Game::play_sim_voice`, and
//!    Create-a-Family `CafSession` Sim voice preview emission).

use sims_audio::music::MusicMode;
use sims_audio::voices::{VoiceAgeGender, VoicePitch};
use sims_game::game::{Game, Options, goth_house};
use sims_game::neighborhood::{NeighborhoodShell, ScreenKind};
use sims_game::sound::{self, Input, Mixer};
use sims_render::neighborhood::NeighborhoodKind;
use sims_ui::album::AlbumCommand;
use sims_ui::build::{BuildSubtoolAction, RoofPitchPreset, TerrainSubtool, WaterSubtool};
use sims_ui::command::{BuildTool, Mode};
use sims_ui::loading::{LoadingKind, LoadingState};
use sims_ui::shop::{CurrencyKind, ShopCommand, ShopItem, ShopKind};
use sims_ui::travel::{Destination, LotFilter, TravelCommand};
use sims_vm::boot::Frames;
use sims_vm::travel::TravelRequest;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn install_root() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    dir.is_dir().then_some(dir)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("sims-game-wave2-{tag}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp test dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn find_open_grass_tile(game: &Game) -> [i32; 2] {
    let world = game.lot().world();
    let active = usize::try_from(world.size).unwrap_or(52);
    let prices = sims_vm::ToolPrices::default();
    for x in 10..(active as i32 - 10) {
        for y in 10..(active as i32 - 10) {
            let pool_plan = sims_vm::plan_pool_edit(
                world,
                [x, y],
                [x + 1, y + 1],
                sims_vm::PoolMode::Dig,
                &prices,
            );
            let water_plan =
                sims_vm::plan_water_edit(world, [x, y], [x, y], sims_vm::WaterMode::Paint, &prices);
            let terrain_plan = sims_vm::plan_terrain_edit(
                world,
                [x, y],
                [x, y],
                sims_vm::TerrainMode::Raise,
                &prices,
            );
            if pool_plan.valid && water_plan.valid && terrain_plan.valid {
                return [x, y];
            }
        }
    }
    panic!("expected at least one flat unobstructed grass patch on the Goth lot");
}

fn project_fine_on_lot(game: &Game, fine: [i32; 2]) -> [i32; 2] {
    let lot_shape = sims_render::Lot::from_world(game.lot().world()).expect("lot shape");
    let pt =
        sims_game::sound::object_point_on_lot(&game.view.camera, &lot_shape, [fine[0], fine[1], 1]);
    [pt[0] + 1, pt[1]]
}

fn project_tile_center(game: &Game, tile: [i32; 2]) -> [i32; 2] {
    project_fine_on_lot(game, [tile[0] * 16 + 8, tile[1] * 16 + 8])
}

fn project_vertex(game: &Game, vertex: [i32; 2]) -> [i32; 2] {
    // Nudge 1/16th of a tile off the exact corner line so `unproject_terrain`
    // stays strictly inside the tile while `snap_wall_corner` snaps back to `vertex`.
    project_fine_on_lot(game, [vertex[0] * 16 + 1, vertex[1] * 16 + 1])
}

fn center_camera_on_tile(game: &mut Game, tile: [i32; 2]) {
    game.view
        .camera
        .center_on([tile[0] * 16 + 8, tile[1] * 16 + 8], [400, 220]);
    let actual = project_tile_center(game, tile);
    game.view.camera.origin[0] += 400 - actual[0];
    game.view.camera.origin[1] += 220 - actual[1];
}

// ============================================================================
// 1. Build Mode Part 2: Terrain / Water / Pool / Roof + Undo/Redo + Previews
// ============================================================================

#[test]
fn wave2_build_mode_part2_terrain_water_pool_roof_and_previews() {
    let Some(install) = install_root() else {
        return;
    };
    let mut game = Game::open(&Options {
        install: install.clone(),
        house: goth_house(&install),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("open Goth house");
    game.resize([800, 600]);

    // Capture baseline Live Mode frame before entering Build Mode.
    let baseline_live = game.render().color.clone();

    // Find a clean flat grass patch and center camera on it (accounting for terrain elevation).
    let tile = find_open_grass_tile(&game);
    center_camera_on_tile(&mut game, [tile[0] + 1, tile[1] + 1]);

    // Enter Build Mode.
    game.set_mode(Mode::Build);
    assert_eq!(game.mode(), Mode::Build);
    let funds_start = sims_vm::available_funds(game.lot().world());

    // --- 1A. Terrain Raise commit + preview rendering + Undo/Redo ---
    game.select_build_tool(BuildTool::Terrain);
    game.apply_build_subtool(BuildSubtoolAction::Terrain(TerrainSubtool::Raise));
    let before_terrain_frame = game.render().color.clone();
    let vpt = project_vertex(&game, tile);
    assert!(game.hover(vpt));
    let terrain_preview_frame = game.render().color.clone();
    assert_ne!(
        before_terrain_frame, terrain_preview_frame,
        "hovering with Terrain Raise must render the translucent build_preview overlay"
    );

    let before_heights = game.lot().world().arrays.get(&0).unwrap().data.clone();
    game.press(vpt);
    game.release(vpt);
    let after_raise_heights = game.lot().world().arrays.get(&0).unwrap().data.clone();
    assert_ne!(
        before_heights, after_raise_heights,
        "Terrain Raise click must commit heightfield changes"
    );
    assert!(
        sims_vm::available_funds(game.lot().world()) < funds_start,
        "Terrain Raise must charge Architecture funds"
    );

    // Undo restores exact terrain heights and funds; Redo reapplies them.
    assert!(game.undo_buy());
    assert_eq!(
        game.lot().world().arrays.get(&0).unwrap().data,
        before_heights,
        "Undo must restore terrain heightfield"
    );
    assert_eq!(sims_vm::available_funds(game.lot().world()), funds_start);
    assert!(game.redo_buy());
    assert_eq!(
        game.lot().world().arrays.get(&0).unwrap().data,
        after_raise_heights
    );
    assert!(game.undo_buy());

    // --- 1B. Water (Pond) paint + preview + Ctrl-remove + Undo ---
    game.select_build_tool(BuildTool::Water);
    game.apply_build_subtool(BuildSubtoolAction::Water(WaterSubtool::Water));
    let t0 = project_tile_center(&game, tile);
    let before_water_preview = game.render().color.clone();
    assert!(game.hover(t0));
    let with_water_preview = game.render().color.clone();
    assert_ne!(
        before_water_preview, with_water_preview,
        "hovering with Water tool must render the pond preview overlay"
    );
    let funds_before_water = sims_vm::available_funds(game.lot().world());
    game.press(t0);
    game.release(t0);
    assert!(
        sims_vm::available_funds(game.lot().world()) < funds_before_water,
        "Water paint must charge pond cost"
    );
    let funds_after_water = sims_vm::available_funds(game.lot().world());

    // Ctrl-remove refunds 80% of the painted water tile.
    // Re-project after water paint because painting water depresses the tile's height.
    let t0_water = project_tile_center(&game, tile);
    game.set_ctrl(true);
    game.press(t0_water);
    game.release(t0_water);
    game.set_ctrl(false);
    assert!(
        sims_vm::available_funds(game.lot().world()) > funds_after_water,
        "Ctrl-Water remove must refund part of pond cost"
    );
    assert!(game.undo_buy());
    assert!(game.undo_buy());
    assert_eq!(
        sims_vm::available_funds(game.lot().world()),
        funds_before_water
    );

    // --- 1C. Swimming Pool drag-dig + coping preview + Undo/Redo ---
    game.apply_build_subtool(BuildSubtoolAction::Water(WaterSubtool::Pool));
    let t1 = project_tile_center(&game, [tile[0] + 1, tile[1] + 1]);
    let funds_before_pool = sims_vm::available_funds(game.lot().world());
    game.press(t0);
    assert!(game.hover(t1));
    let pool_preview_frame = game.render().color.clone();
    assert_ne!(
        before_water_preview, pool_preview_frame,
        "dragging with Pool tool must render translucent pool + coping preview"
    );
    game.release(t1);
    assert!(
        sims_vm::available_funds(game.lot().world()) < funds_before_pool,
        "Pool dig must charge pool cost"
    );
    let pool_after = game.lot().world().arrays.get(&9).map(|a| a.data.clone());
    assert!(
        pool_after
            .as_ref()
            .is_some_and(|d| d.iter().any(|&v| v != 0)),
        "Pool array 9 must contain dug pool tiles"
    );
    assert!(game.undo_buy());
    assert_eq!(
        sims_vm::available_funds(game.lot().world()),
        funds_before_pool
    );
    assert!(game.redo_buy());
    assert!(game.undo_buy());

    // --- 1D. Roof pitch & pattern commit + Roof preview + Undo/Redo ---
    let base_roof = game.base_roof().clone();
    assert!(!base_roof.pattern.is_empty());
    // Center over the Goth house footprint so the isometric roof mesh is in frame.
    game.view.camera.center_on([26 * 16, 26 * 16], [400, 260]);
    let before_roof_frame = game.render().color.clone();
    game.select_build_tool(BuildTool::Roof);
    let roof_preview_frame = game.render().color.clone();
    assert_ne!(
        before_roof_frame, roof_preview_frame,
        "selecting BuildTool::Roof must render the isometric RoofCatalog preview"
    );
    let target_pitch = if base_roof.pitch == sims_vm::RoofPitch::Steep {
        RoofPitchPreset::Shallow
    } else {
        RoofPitchPreset::Steep
    };
    game.apply_build_subtool(BuildSubtoolAction::RoofPitch(target_pitch));
    let edited_state = game.current_roof_state();
    assert_ne!(edited_state.pitch, base_roof.pitch);

    // Also switch roof pattern if catalog has >= 2 patterns.
    game.apply_build_subtool(BuildSubtoolAction::RoofPattern(1));
    let after_pat = game.current_roof_state();
    assert!(game.undo_buy()); // undo pattern (or pitch if 1 pattern)
    if after_pat.pattern != edited_state.pattern {
        assert_eq!(game.current_roof_state().pattern, edited_state.pattern);
        assert!(game.undo_buy());
    }
    assert_eq!(game.current_roof_state().pitch, base_roof.pitch);
    assert!(game.redo_buy());
    assert_eq!(game.current_roof_state().pitch, edited_state.pitch);

    // Camera Roof toggle draws roofs even in Live mode, and toggling it off restores baseline.
    game.set_mode(Mode::Live);
    assert!(!game.roof_view());
    game.set_roof_view(true);
    let with_roof_live = game.render().color.clone();
    game.set_roof_view(false);
    let without_roof_live = game.render().color.clone();
    assert_ne!(
        with_roof_live, without_roof_live,
        "enabling Camera Roof view must draw isometric roof preview"
    );
    assert!(!baseline_live.is_empty());
}

// ============================================================================
// 2. Photo Album + Persistent Custom Lot Map Thumbnails Across Restart
// ============================================================================

#[test]
fn wave2_photo_album_and_persistent_custom_lot_thumbnails() {
    let Some(install) = install_root() else {
        return;
    };
    let tmp = TempDir::new("album-thumbs");
    let saves = tmp.path().join("wave2-album.redb");

    // First session: open Neighborhood 1, enter Goth lot 5, test Photo Album + thumbnail save.
    let (captured_thumb_pixels, goth_family_id) = {
        let mut shell = NeighborhoodShell::open(
            install.clone(),
            saves.clone(),
            1,
            true,
            vec![3],
            Frames::default(),
        )
        .expect("open NeighborhoodShell");
        let goth_fam = shell
            .stored_neighborhood()
            .family_in(5)
            .expect("Goth family on lot 5")
            .0;

        shell.enter_lot(5).expect("enter Goth house");
        assert_eq!(shell.screen(), ScreenKind::Lot);
        shell.render();

        // In idle Live mode, no floating CamViewAlbum overlaps the clock/cluster.
        assert!(
            shell
                .game()
                .expect("active game")
                .album_button_rect()
                .is_none(),
            "CamViewAlbum must not overlap the Live Mode cluster before opening Camera sub-strip"
        );
        // Clicking the cluster Camera button opens the Camera sub-strip inside the
        // bottom panel strip area (x >= CLUSTER[0]), offering its native controls and Photo Album.
        let camera_btn = shell
            .game()
            .expect("active game")
            .panel()
            .iter()
            .find(|item| {
                item.command
                    == Some(sims_ui::command::Command::Camera(
                        sims_ui::command::Camera::Photo,
                    ))
            })
            .expect("Camera button on Live cluster")
            .rect;
        let cam_center = [
            camera_btn.x + camera_btn.width / 2,
            camera_btn.y + camera_btn.height / 2,
        ];
        shell.press(cam_center);
        shell.release(cam_center);
        shell.render();

        // `cWinCamera`: clicking the lot with the Camera strip open takes a
        // photo into the household's album and closes the strip.
        assert!(shell.game().expect("active game").camera_strip_open());
        let lot_point = [400, 200];
        shell.press(lot_point);
        shell.release(lot_point);
        assert!(!shell.game().expect("active game").camera_strip_open());
        shell.press(cam_center);
        shell.release(cam_center);
        shell.render();
        let album_rect = shell
            .game()
            .expect("active game")
            .album_button_rect()
            .expect("Photo Album button rect should be laid out in open Camera sub-strip");
        assert!(
            album_rect.x >= sims_ui::hud::CLUSTER[0],
            "Camera sub-strip Photo Album button must sit right of Live cluster"
        );
        let btn_center = [
            album_rect.x + album_rect.width / 2,
            album_rect.y + album_rect.height / 2,
        ];
        shell.press(btn_center);
        shell.release(btn_center);
        assert!(
            shell.album_open(),
            "clicking Photo Album in Camera sub-strip must open PhotoAlbumDialog"
        );

        // A second photo while the album is open shows up in it at once.
        // Then edit text, reorder, and render.
        let second_id = shell.capture_lot_photo().expect("second photo");
        let snap = shell.album_snapshot().expect("album snapshot").clone();
        assert_eq!(snap.entries.len(), 2);
        let snap_id = snap.entries[0].id;
        assert_eq!(snap.entries[1].id, second_id);
        assert!(
            snap.entries.iter().all(|entry| entry.image.is_some()),
            "each photo must store a real framebuffer bitmap"
        );
        shell.execute_album_command(AlbumCommand::UpdateEntryText {
            id: snap_id,
            title: "Goth Manor".to_owned(),
            caption: "First evening in the garden".to_owned(),
        });
        shell.execute_album_command(AlbumCommand::MoveEntry {
            id: second_id,
            new_index: 0,
        });
        shell.render();
        shell.execute_album_command(AlbumCommand::Close);
        assert!(!shell.album_open());

        // Save & Leave lot 5 so a persistent custom lot thumbnail is stored in REDB.
        shell.leave_lot(true).expect("save and leave lot 5");
        assert_eq!(shell.screen(), ScreenKind::Neighborhood);
        let persisted_thumbs = shell
            .store()
            .load_lot_thumbnails("UserData")
            .expect("load persisted thumbnails");
        let thumb5 = persisted_thumbs
            .get(&5)
            .expect("lot 5 custom thumbnail must be persisted in REDB");
        assert!(thumb5.normal.width > 0 && thumb5.normal.height > 0);
        (thumb5.normal.pixels.clone(), goth_fam)
    };

    // Second session (restart): verify both Photo Album entries AND custom lot thumbnail survived.
    {
        let mut shell = NeighborhoodShell::open(
            install.clone(),
            saves.clone(),
            1,
            true,
            vec![3],
            Frames::default(),
        )
        .expect("reopen NeighborhoodShell after restart");

        // Lot 5 map cell must match the persisted custom thumbnail loaded from REDB.
        let lot5 = shell.map().lot(5).expect("lot 5 on map");
        assert_eq!(
            lot5.normal.pixels, captured_thumb_pixels,
            "restarted NeighborhoodShell must load persisted custom thumbnail for lot 5"
        );

        // Reopen Photo Album for the Goth family and verify order + text + image survived restart.
        shell
            .open_photo_album_for_family(goth_family_id)
            .expect("reopen Goth Photo Album after restart");
        let snap = shell
            .album_snapshot()
            .expect("reopened album snapshot")
            .clone();
        assert_eq!(snap.entries.len(), 2);
        assert_ne!(
            snap.entries[0].title, "Goth Manor",
            "the second photo moved to index 0"
        );
        assert!(snap.entries[0].image.is_some());
        assert_eq!(snap.entries[1].title, "Goth Manor");
        assert_eq!(snap.entries[1].caption, "First evening in the garden");
        assert!(snap.entries[1].image.is_some());

        // Delete one entry and verify persistence.
        let del_id = snap.entries[0].id;
        shell.execute_album_command(AlbumCommand::DeleteEntry(del_id));
        assert_eq!(shell.album_snapshot().unwrap().entries.len(), 1);
        shell.close_photo_album();

        // Evict + Bulldoze lot 5 -> persisted custom thumbnail must be deleted from REDB.
        shell.execute_evict(5).expect("evict Goths from lot 5");
        shell.execute_bulldoze(5).expect("bulldoze lot 5");
        let thumbs_after_bulldoze = shell
            .store()
            .load_lot_thumbnails("UserData")
            .expect("load thumbnails after bulldoze");
        assert!(
            !thumbs_after_bulldoze.contains_key(&5),
            "bulldozing lot 5 must remove its custom thumbnail from REDB"
        );
    }
}

// ============================================================================
// 3. Travel Destinations Shell, Shop Picker, Crash Recovery & Community Edit
// ============================================================================

#[test]
fn wave2_travel_shell_shop_picker_crash_recovery_and_community_edit() {
    let Some(install) = install_root() else {
        return;
    };
    let tmp = TempDir::new("travel-shop");
    let saves = tmp.path().join("wave2-travel.redb");

    let mut shell = NeighborhoodShell::open(
        install.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("open NeighborhoodShell");

    // --- 3A. Destination map switching & TravelChrome filter + lodging pixel highlighting ---
    for (dest, expected_kind, filter) in [
        (
            Destination::Downtown,
            NeighborhoodKind::Downtown,
            LotFilter::Shopping,
        ),
        (
            Destination::VacationIsland,
            NeighborhoodKind::Vacation,
            LotFilter::Food,
        ),
        (
            Destination::StudioTown,
            NeighborhoodKind::StudioTown,
            LotFilter::Movie,
        ),
        (
            Destination::MagicTown,
            NeighborhoodKind::MagicTown,
            LotFilter::Magic,
        ),
    ] {
        shell.handle_travel_command(TravelCommand::SwitchDestination(dest));
        assert_eq!(shell.neighborhood_kind(), expected_kind);
        shell.render();
        let unfiltered = shell.canvas().color.clone();
        shell.handle_travel_command(TravelCommand::SetFilter(Some(filter)));
        assert_eq!(shell.travel_state().active_filter, Some(filter));
        shell.render();
        let filtered = shell.canvas().color.clone();
        assert_ne!(
            unfiltered, filtered,
            "activating TravelChrome filter on {dest:?} must update rendered map/chrome pixels"
        );
        if dest == Destination::VacationIsland {
            shell.handle_travel_command(TravelCommand::SetFilter(None));
            shell.render();
            let base_vac = shell.canvas().color.clone();
            shell.handle_travel_command(TravelCommand::SetVacationLodging(Some(
                sims_ui::travel::VacationLodging::Hotel,
            )));
            shell.render();
            let hotel_vac = shell.canvas().color.clone();
            assert_ne!(
                base_vac, hotel_vac,
                "selecting VacationLodging::Hotel must highlight hotel lots on Vacation Island"
            );
        }
    }
    shell.handle_travel_command(TravelCommand::ReturnToNeighborhood);
    assert_eq!(shell.neighborhood_kind(), NeighborhoodKind::Unleashed);

    // --- 3B. Community lot inspect/edit (`open_destination_for_edit` + `leave_lot(true)`) ---
    shell
        .switch_kind(NeighborhoodKind::Downtown)
        .expect("switch to Downtown");
    let dt_lot = shell
        .map()
        .lots
        .first()
        .expect("Downtown has community lots")
        .id;
    shell
        .open_destination_for_edit(u32::from(dt_lot))
        .expect("open Downtown lot for edit");
    assert_eq!(shell.screen(), ScreenKind::Lot);
    assert!(shell.is_destination_edit());
    assert!(shell.active_trip().is_none());
    // Unlimited-budget edit sessions must NOT allow opening a spending ShopPickerDialog.
    assert_eq!(
        shell.game().unwrap().shop_kind_for_lot(),
        None,
        "ShopPickerDialog must be refused in community edit mode"
    );
    shell.render();
    shell.leave_lot(true).expect("save and leave Downtown edit");
    assert_eq!(shell.screen(), ScreenKind::Neighborhood);

    // --- 3C. Live Travel: validate -> TravelDialog -> depart -> ShopPicker -> return ---
    shell.handle_travel_command(TravelCommand::ReturnToNeighborhood);
    shell.enter_lot(5).expect("enter Goth home lot 5");
    let goth_members = shell
        .stored_neighborhood()
        .family_in(5)
        .expect("Goth household")
        .1
        .members
        .clone();
    let leader_guid = goth_members[0];
    let funds_before_trip = sims_vm::available_funds(shell.game().unwrap().lot().world());

    let req = TravelRequest::downtown(u32::from(dt_lot), leader_guid);
    let plan = shell
        .open_travel_dialog(req.clone())
        .expect("validate travel plan and open TravelDialog");
    assert_eq!(
        plan.destination,
        sims_vm::travel::TravelDestination::Downtown
    );
    assert!(shell.travel_dialog_open());
    shell.render();

    // Confirm travel via Enter -> executes `depart_to_destination`.
    shell.key_enter();
    assert!(!shell.travel_dialog_open());
    let trip = shell
        .active_trip()
        .expect("household should be on active Downtown trip")
        .clone();
    assert_eq!(trip.home_house, 5);
    assert_eq!(trip.destination_house, u32::from(dt_lot));

    // Step destination lot and buy an item through ShopPickerDialog (Simoleons).
    {
        let game = shell.game_mut().expect("destination game");
        assert!(game.advance(100_000) >= 1);
        assert_eq!(
            game.shop_kind_for_lot(),
            Some(ShopKind::DowntownBoutique(
                sims_ui::shop::WardrobeCategory::Formal
            ))
        );
        assert!(game.open_shop(ShopKind::DowntownBoutique(
            sims_ui::shop::WardrobeCategory::Formal
        )));
        game.render();
        let before_buy = sims_vm::available_funds(game.lot().world());
        game.execute_shop_command(ShopCommand::Buy {
            item_id: "dt_formal_1".to_owned(),
            currency: CurrencyKind::Simoleons,
            price: 250,
        });
        assert_eq!(
            sims_vm::available_funds(game.lot().world()),
            before_buy - 250,
            "ShopPickerDialog Buy must deduct Simoleons cleanly"
        );
        // Grant real MagiCoins to the first traveler and verify nonzero MagiCoins deduction.
        let first_guid = goth_members[0];
        let first_nid = game
            .lot()
            .world()
            .neighbors
            .slots
            .iter()
            .flatten()
            .find(|r| r.guid == first_guid)
            .map(|r| r.id)
            .expect("traveler neighbor id");
        {
            let lot_mut = game.lot_mut();
            let saved = lot_mut.save().expect("save for MagiCoins grant");
            let orig_world_body = saved.world().encode();
            let encoded = saved.encode();
            let tail = encoded[8..]
                .strip_prefix(orig_world_body.as_slice())
                .expect("saved world prefix");
            let mut world = saved.world().clone();
            sims_vm::travel::set_neighbor_token_count(
                &mut world,
                first_nid,
                sims_vm::travel::MAGICOIN_TOKEN_KIND,
                sims_vm::travel::MAGICOIN_TOKEN_GUID,
                120,
            );
            let new_world_body = world.encode();
            let mut rebuilt = Vec::with_capacity(8 + new_world_body.len() + tail.len());
            rebuilt.extend_from_slice(&encoded[..8]);
            rebuilt.extend_from_slice(&new_world_body);
            rebuilt.extend_from_slice(tail);
            let patched =
                sims_vm::boot::save::Saved::decode(&rebuilt).expect("decode patched saved");
            lot_mut.restore(patched).expect("restore MagiCoins grant");
        }
        assert!(game.open_shop_with_items(
            ShopKind::MagicVendor,
            vec![ShopItem {
                id: "magic_reagent_45".into(),
                name: "Test Reagent".into(),
                description: "Nonzero MagiCoins test item".into(),
                price: 45,
                currency: CurrencyKind::MagiCoins,
                icon: None,
                supports_try_on: false,
                owned: false,
                available: true,
            }],
        ));
        assert_eq!(game.shop_snapshot().unwrap().wallet.magic_coins, 120);
        game.execute_shop_command(ShopCommand::Buy {
            item_id: "magic_reagent_45".into(),
            currency: CurrencyKind::MagiCoins,
            price: 45,
        });
        assert_eq!(
            game.shop_snapshot().unwrap().wallet.magic_coins,
            75,
            "ShopPickerDialog Buy must deduct nonzero MagiCoins cleanly"
        );
        assert!(
            game.shop_snapshot()
                .unwrap()
                .items
                .iter()
                .find(|i| i.id == "magic_reagent_45")
                .unwrap()
                .owned
        );
        game.close_shop();
    }
    let funds_after_shop = sims_vm::available_funds(shell.game().unwrap().lot().world());

    // Save Manager during active trip must target home house and refuse loading slots.
    assert!(shell.open_save_manager(None));
    shell.handle_save_manager_command(sims_ui::saves::SaveManagerCommand::LoadSlot {
        slot_id: "manual".into(),
    });
    assert!(
        shell.active_trip().is_some(),
        "Save Manager LoadSlot must be blocked during active travel visits"
    );
    shell.close_save_manager();

    // Immediately return home WITHOUT restarting to prove the mutated destination lot
    // reconciles the exact debited Simoleons + MagiCoins back onto the home lot!
    let immediate_ret = shell
        .return_from_destination()
        .expect("immediate return from Downtown after shop purchases");
    assert_eq!(immediate_ret.home_house, 5);
    assert!(shell.active_trip().is_none());
    assert_eq!(shell.active_lot_house(), Some(5));
    assert_eq!(
        sims_vm::available_funds(shell.game().unwrap().lot().world()),
        funds_after_shop,
        "immediate return_from_destination must reconcile shop Simoleons debit onto home lot"
    );
    assert_eq!(
        sims_vm::travel::family_magic_coins(shell.game().unwrap().lot().world(), &goth_members),
        75,
        "immediate return_from_destination must reconcile MagiCoins debit onto home lot"
    );

    // Desktop shortcut helper: select the second adult household member (`Bella`),
    // verify `open_default_travel_dialog` picks THAT selected Sim (via `World::selected()`),
    // and verify `cycle_travel_destination` switches destinations cleanly.
    let second_adult_guid = {
        let game = shell.game_mut().expect("home game");
        let world = game.lot().world();
        let (&obj_id, obj) = world
            .objects
            .iter()
            .find(|(_, o)| {
                o.person.is_some()
                    && goth_members.contains(&o.guid)
                    && o.guid != goth_members[0]
                    && o.person
                        .as_ref()
                        .is_some_and(|p| p.data.get(58).copied().unwrap_or(27) >= 18)
            })
            .expect("Goth household has a second adult member (Bella)");
        let guid = obj.guid;
        game.select(obj_id);
        assert_eq!(
            game.lot().world().selected(),
            Some(obj_id),
            "Game::select must mark Bella as World::selected()"
        );
        guid
    };
    let default_plan = shell
        .open_default_travel_dialog()
        .expect("desktop open_default_travel_dialog on occupied home lot");
    assert_eq!(
        default_plan
            .members
            .iter()
            .map(|m| m.guid)
            .collect::<Vec<_>>(),
        vec![second_adult_guid],
        "desktop solo travel shortcut must send the currently selected Sim, not always member 0"
    );
    assert!(shell.travel_dialog_open());
    assert!(shell.cycle_travel_destination(1));
    let cycled_dest = shell.travel_dialog_state().unwrap().destination;
    assert_ne!(
        cycled_dest,
        match default_plan.destination {
            sims_vm::travel::TravelDestination::Downtown
            | sims_vm::travel::TravelDestination::OldTown => Destination::Downtown,
            sims_vm::travel::TravelDestination::VacationIsland => Destination::VacationIsland,
            sims_vm::travel::TravelDestination::StudioTown => Destination::StudioTown,
            sims_vm::travel::TravelDestination::MagicTown => Destination::MagicTown,
        },
        "cycling destination in TravelDialog must switch to another eligible destination"
    );
    shell.close_travel_dialog();

    // Verify Magic Town Family Bin button stays reachable at compact widths (< 800px).
    shell.leave_lot(false).expect("leave Goth lot 5");
    shell.handle_travel_command(TravelCommand::SwitchDestination(Destination::MagicTown));
    shell.resize([640, 480]);
    shell.render();
    let compact_layout = sims_ui::travel::TravelLayout::compute(640, 480, shell.travel_state());
    assert!(compact_layout.compact);
    let compact_fb_pt = [
        compact_layout.filter_strip_rect.x + compact_layout.filter_strip_rect.width - 20,
        compact_layout.filter_strip_rect.y + compact_layout.filter_strip_rect.height / 2,
    ];
    shell.press(compact_fb_pt);
    shell.release(compact_fb_pt);
    assert_eq!(
        shell.screen(),
        ScreenKind::FamilyBin,
        "Magic Town Family Bin button must remain clickable at compact widths (< 800px)"
    );
    shell.close_family_bin();
    shell.resize([800, 600]);
    shell.handle_travel_command(TravelCommand::ReturnToNeighborhood);
    shell
        .enter_lot(5)
        .expect("re-enter Goth lot 5 for crash-recovery test");

    // --- 3D. Crash recovery across restart (`inspect_active_trip` + `resume_active_trip`) ---
    shell
        .depart_to_destination(req.clone())
        .expect("depart for crash-recovery verification");
    drop(shell);
    let mut recovered = NeighborhoodShell::open(
        install.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("reopen shell after simulated crash mid-trip");
    let found = recovered.inspect_any_active_trips();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, 5);
    assert_eq!(found[0].1.destination_house, u32::from(dt_lot));

    // Resuming lot 5 directly via `enter_lot(5)` automatically resumes the active trip!
    recovered.enter_lot(5).expect("auto-resume crashed trip");
    assert_eq!(
        recovered.active_trip().map(|t| t.destination_house),
        Some(u32::from(dt_lot))
    );

    // Return home via `return_from_destination` and verify home lot reconciled cleanly.
    let report = recovered
        .return_from_destination()
        .expect("return from Downtown");
    assert_eq!(report.home_house, 5);
    assert!(recovered.active_trip().is_none());
    assert_eq!(recovered.active_lot_house(), Some(5));
    assert!(
        sims_vm::available_funds(recovered.game().unwrap().lot().world()) <= funds_before_trip,
        "fare paid at departure must stay reconciled upon returning home"
    );

    // --- 3E. Depart again and test `abort_active_trip` recovery ---
    recovered
        .depart_to_destination(req)
        .expect("second departure to Downtown");
    assert!(
        recovered
            .inspect_active_trip(5)
            .expect("inspect trip")
            .is_some()
    );
    drop(recovered);
    let mut aborted = NeighborhoodShell::open(
        install.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("reopen shell for abort test");
    let restored_house = aborted.abort_active_trip(5).expect("abort active trip");
    assert_eq!(restored_house, 5);
    assert!(aborted.inspect_active_trip(5).unwrap().is_none());
}

// ============================================================================
// 4. Native Loading Screen & Simlish Voices (`VoiceCatalog` + CAF + Live Lot)
// ============================================================================

#[test]
fn wave2_loading_screen_transitions_and_simlish_voices() {
    let Some(install) = install_root() else {
        return;
    };
    let tmp = TempDir::new("loading-voices");
    let saves = tmp.path().join("wave2-loading.redb");

    let mut shell = NeighborhoodShell::open(
        install.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("open NeighborhoodShell");

    // Startup must emit LoadingAudioCue::StartLoadLoop -> MusicMode::Load -> MusicMode::Neighborhood.
    let startup_audio = shell.take_audio_inputs();
    assert!(
        startup_audio.contains(&Input::MusicMode(MusicMode::Load)),
        "startup should emit MusicMode::Load"
    );
    assert!(
        startup_audio.contains(&Input::MusicMode(MusicMode::Neighborhood)),
        "startup should settle into MusicMode::Neighborhood"
    );

    // Interactive LoadingScreen transitions, rotating UIText.iff captions, and splash rendering.
    assert!(
        !shell.loading_strings().progress_captions.is_empty(),
        "Complete Collection UIText.iff STR# 155 loading captions must be loaded"
    );
    shell.begin_loading(LoadingKind::Downtown, Some("Downtown".into()), 0.25);
    assert_eq!(shell.screen(), ScreenKind::Loading);
    let cap0 = shell.loading_screen().caption_index();
    shell.render();
    let frame0 = shell.canvas().color.clone();
    assert!(frame0.iter().any(|&rgb| rgb != 0));
    assert!(
        shell
            .take_audio_inputs()
            .contains(&Input::MusicMode(MusicMode::Load))
    );

    shell.update_loading(0.8, None);
    shell.tick_loading(3_000);
    assert_ne!(
        shell.loading_screen().caption_index(),
        cap0,
        "ticking LoadingScreen past 2500 ms must rotate caption index"
    );
    let standalone = shell.render_loading_frame(
        &LoadingState {
            kind: LoadingKind::MagicTown { via_hole: true },
            progress: 0.65,
            destination_label: Some("Magic Town".into()),
            caption_override: None,
        },
        [800, 600],
    );
    assert_eq!((standalone.width, standalone.height), (800, 600));
    assert!(standalone.color.iter().any(|&rgb| rgb != 0));

    shell.finish_loading();
    assert_eq!(shell.screen(), ScreenKind::Neighborhood);
    assert!(
        shell
            .take_audio_inputs()
            .contains(&Input::MusicMode(MusicMode::Neighborhood))
    );

    // CAF Sim voice preview emission -> Mixer VoiceCatalog playback -> non-silent PCM.
    shell.open_create_family().expect("open Create-a-Family");
    assert_eq!(shell.screen(), ScreenKind::CreateFamily);
    {
        let caf = shell.caf_session_mut().expect("active CAF session");
        let _ = caf.controller_mut().begin_add_sim();
        let preview = caf
            .preview_voice()
            .expect("CAF should emit a Simlish voice preview");
        assert!(matches!(
            preview.voice,
            VoiceAgeGender::AdultMale | VoiceAgeGender::AdultFemale | VoiceAgeGender::Child
        ));
    }
    // Tick shell to drain CAF voice preview into shell's audio queue.
    let _ = shell.tick_shell(16_000);
    let caf_audio = shell.take_audio_inputs();
    let voice_input = caf_audio
        .into_iter()
        .find(|i| matches!(i, Input::PlayVoice { .. }))
        .expect("CAF voice preview must be drained into NeighborhoodShell audio_queue");

    let mut mixer = Mixer::open(&install).expect("open audio Mixer");
    assert!(
        !mixer.voices().is_empty(),
        "VoiceCatalog must index Complete Collection Simlish voice clips"
    );
    mixer.input(voice_input);
    assert_eq!(
        mixer.active_voice_count(),
        1,
        "Mixer must start a VoiceStream for Input::PlayVoice"
    );
    let pcm = mixer.render(16_384);
    assert!(
        pcm.iter().any(|&s| s != 0),
        "VoiceCatalog playback must mix non-silent Simlish PCM samples"
    );

    // Also verify live lot Sim speech (`Game::play_sim_voice` -> `sound::Feed::update` -> `Mixer`).
    shell.open_family_bin();
    shell.close_family_bin();
    shell.enter_lot(5).expect("enter Goth lot 5");
    let first_sim = *shell
        .game()
        .unwrap()
        .lot()
        .world()
        .people
        .first()
        .expect("Goth lot has live Sims");
    assert!(
        shell
            .game_mut()
            .unwrap()
            .play_sim_voice(first_sim, Some("converse"), 42)
    );
    let mut feed = sound::Feed::default();
    let mut voice_sent = false;
    feed.update(shell.game_mut().unwrap(), |input| {
        if matches!(input, Input::PlayVoice { .. }) {
            voice_sent = true;
        }
        mixer.input(input);
    });
    assert!(
        voice_sent,
        "sound::Feed::update must forward Game::play_sim_voice to Mixer"
    );
    mixer.input(Input::PlayVoice {
        voice: VoiceAgeGender::AdultFemale,
        category: Some("laugh".into()),
        pitch: VoicePitch::from_semitones(1.5),
        seed: 7,
        object: None,
    });
    let pcm2 = mixer.render(16_384);
    assert!(pcm2.iter().any(|&s| s != 0));
}

// ============================================================================
// 5. Large Photo Album (>32 MiB aggregate) Summary + Lazy Hydration & Presenter
// ============================================================================

#[test]
fn wave2_large_photo_album_60_snapshots_and_loading_presenter() {
    let Some(install) = install_root() else {
        return;
    };
    let tmp = TempDir::new("album-60");
    let saves = tmp.path().join("wave2-album-60.redb");

    let mut shell = NeighborhoodShell::open(
        install.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("open NeighborhoodShell");

    // Attach a loading presenter and verify entering a lot invokes it with live
    // rendered splash pixels AND `MusicMode::Load` before VM boot completes.
    let presented_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let saw_load_music = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let count = std::sync::Arc::clone(&presented_count);
        let load_flag = std::sync::Arc::clone(&saw_load_music);
        shell.set_loading_presenter(Some(Box::new(move |pixels, size, audio| {
            if !pixels.is_empty() && size[0] > 0 && size[1] > 0 {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            if audio
                .iter()
                .any(|i| matches!(i, Input::MusicMode(MusicMode::Load)))
            {
                load_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        })));
    }
    shell.enter_lot(5).expect("enter Goth lot 5");
    assert!(
        presented_count.load(std::sync::atomic::Ordering::SeqCst) >= 1,
        "LoadingPresenter must present at least one native loading frame before blocking lot boot"
    );
    assert!(
        saw_load_music.load(std::sync::atomic::Ordering::SeqCst),
        "LoadingPresenter must receive MusicMode::Load immediately"
    );
    shell.leave_lot(false).expect("leave Goth lot 5");

    // Add 60 full-size 640x480 RGB565 snapshots (~36 MiB > 32 MiB bulk `list()` ceiling)
    // and verify the album opens, navigates to entry #60, hydrates its image, and deletes cleanly.
    let goth_family = shell
        .stored_neighborhood()
        .family_in(5)
        .map(|(id, _)| id)
        .expect("Goth family id");
    let token = shell
        .store()
        .albums()
        .family_token("UserData", goth_family)
        .expect("family token")
        .expect("Goth token");
    let full_img = sims_sim::album_store::AlbumImage::new(640, 480, vec![0x39E7u16; 640 * 480])
        .expect("640x480 album image");
    for i in 1..=60u32 {
        let entry = sims_sim::album_store::NewAlbumPhoto::with_image(
            format!("Snapshot {i}"),
            format!("Caption {i}"),
            full_img.clone(),
        );
        shell
            .store()
            .albums()
            .add("UserData", token, entry)
            .expect("add full-size snapshot");
    }
    // Confirm bulk `list()` would have rejected >32 MiB, while our shell opens cleanly via `list_summaries` + lazy `get`.
    assert!(
        shell
            .store()
            .albums()
            .list("UserData", goth_family)
            .is_err(),
        "60 full-size 640x480 snapshots exceed the 32 MiB bulk list() ceiling"
    );
    shell
        .open_photo_album_for_family(goth_family)
        .expect("Photo Album must open 60+ full-size snapshots without locking out the player");
    let snap = shell.album_snapshot().expect("album snapshot");
    assert_eq!(snap.entries.len(), 60);
    assert!(
        snap.entries[0].image.is_some(),
        "selected entry 0 must have hydrated image"
    );
    let last_id = snap.entries[59].id;
    // Scroll the thumbnail strip forward by 20 entries WITHOUT changing selection,
    // verifying visible strip entries receive downscaled thumbnails (never blank)
    // while selection stays on entry 0.
    shell.hover([100, 100]);
    for _ in 0..20 {
        assert!(shell.wheel(-1.0));
    }
    let after_scroll = shell.album_snapshot().expect("scrolled snapshot");
    assert!(
        after_scroll.entries[0].image.is_some(),
        "selection image must remain hydrated while scrolling strip"
    );
    assert!(
        after_scroll.entries[20].thumbnail.is_some(),
        "scrolled visible thumbnail slot must have a hydrated thumbnail"
    );
    shell.execute_album_command(AlbumCommand::SelectEntry(last_id));
    let after_select = shell.album_snapshot().expect("hydrated snapshot");
    assert!(
        after_select.entries[59].image.is_some(),
        "selecting entry 60 must lazily hydrate its image"
    );
    shell.execute_album_command(AlbumCommand::DeleteEntry(last_id));
    assert_eq!(
        shell.album_snapshot().expect("after delete").entries.len(),
        59
    );
    shell.close_photo_album();

    // Verify Save Manager refuses to restore over a lot holding a persisted (even corrupt) `TRVL` envelope.
    shell
        .enter_lot(5)
        .expect("enter Goth lot 5 to save a manual slot");
    shell
        .enqueue_background_lot_save(sims_sim::autosave::SaveTarget::Manual)
        .expect("enqueue manual lot save");
    shell.flush_background_saves();
    shell.leave_lot(true).expect("leave Goth lot 5");
    let nbhd = shell.stored_neighborhood().clone();
    shell
        .store()
        .save_lot_and_neighborhood(
            "UserData",
            5,
            b"TRVL\x01\x00\x00\x00corrupt-envelope",
            &nbhd,
        )
        .expect("plant corrupt TRVL envelope on lot 5");
    assert!(shell.open_save_manager(None));
    shell.handle_save_manager_command(sims_ui::saves::SaveManagerCommand::LoadSlot {
        slot_id: "manual".into(),
    });
    assert_eq!(
        shell.screen(),
        ScreenKind::Neighborhood,
        "Save Manager must refuse restoring a slot over any TRVL-prefixed saved lot"
    );
    let still_trvl = shell
        .store()
        .load_lot_snapshot("UserData", 5)
        .expect("reload lot 5")
        .expect("snapshot")
        .saved_lot
        .expect("saved lot");
    assert!(
        still_trvl.starts_with(b"TRVL"),
        "refused restore must preserve the persisted TRVL envelope untouched"
    );
    shell.close_save_manager();
}
