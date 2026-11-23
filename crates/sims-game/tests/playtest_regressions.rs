//! End-to-end headless regressions for:
//! 1. Roof / Walls Up / Cutaway / Walls Down 4-way radio group & roof rendering.
//! 2. Buy Mode 100 px strip (zero 50 px empty blue overhang) & visual screenshots.
//! 3. Real graphical destination maps for Magic Hole (`MagicTownLotPicker`) and
//!    Cab (`DowntownLotPicker`), both in standalone `Game` and `NeighborhoodShell`.

use sims_game::destination_picker::DestinationMapPicker;
use sims_game::game::{SpecialLotEntry, goth_house};
use sims_game::neighborhood::NeighborhoodShell;
use sims_game::{Frames, Game, Options};
use sims_render::neighborhood::NeighborhoodKind;
use sims_render::{Walls, png};
use sims_ui::command::{BuildTool, Camera as UiCamera, Command, Mode};
use sims_ui::hud::{self, Kind, STRIP, id};
use sims_vm::SpecialPanelRequest;
use std::path::PathBuf;

const MAGIC_PORTAL_GUID: u32 = 0x2baa_0029;

fn install_root() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    dir.is_dir().then_some(dir)
}

fn temp_redb(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "sims-game-playtest-reg-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let redb = dir.join("saves.redb");
    (dir, redb)
}

fn save_shot_if_requested(name: &str, canvas: &sims_render::Canvas) {
    if let Some(dir) = std::env::var_os("SIMS_PLAYTEST_SHOTS_DIR").map(PathBuf::from) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(format!("{name}.png")), png::encode(canvas));
    }
}

fn selected_view_buttons(items: &[hud::Item]) -> Vec<i32> {
    items
        .iter()
        .filter_map(|i| match i.kind {
            Kind::Button {
                id, selected: true, ..
            } if matches!(id, id::ROOF | id::WALLS_UP | id::CUTAWAY | id::WALLS_DOWN) => Some(id),
            _ => None,
        })
        .collect()
}

fn button_center(items: &[hud::Item], target_id: i32) -> [i32; 2] {
    let item = items
        .iter()
        .find(|i| matches!(i.kind, Kind::Button { id, .. } if id == target_id))
        .expect("button must exist in HUD layout");
    [
        item.rect.x + item.rect.width / 2,
        item.rect.y + item.rect.height / 2,
    ]
}

fn place_and_queue_magic_hole(game: &mut Game) {
    let jumper = game.lot().world().selected().expect("selected Goth Sim");
    let sim_pos = game.lot().world().objects[&jumper].position;
    let lot = game.lot_mut();
    lot.set_mode(sims_vm::PanelMode::Buy).expect("enter Buy");
    let mut cursor = lot
        .buy_start(MAGIC_PORTAL_GUID, 0, 0)
        .expect("buy_start Magic Hole")
        .expect("Magic Hole cursor");
    let hole_id = cursor.held;
    let base_x = sim_pos[0] / 16;
    let base_y = sim_pos[1] / 16;
    let mut placed = false;
    'search: for r in 1..=14i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                let tx = base_x + dx;
                let ty = base_y + dy;
                if !(4..=60).contains(&tx) || !(4..=60).contains(&ty) {
                    continue;
                }
                if lot
                    .buy_move(&mut cursor, [tx * 16 + 8, ty * 16 + 8], 1)
                    .unwrap_or(false)
                    && cursor.valid
                {
                    lot.buy_commit(&cursor).expect("commit Magic Hole");
                    placed = true;
                    break 'search;
                }
            }
        }
    }
    assert!(placed, "must find valid tile for Magic Hole");
    lot.set_mode(sims_vm::PanelMode::Live).expect("return Live");
    lot.select(jumper).expect("select jumper");
    let menu = lot.pie_menu(jumper, hole_id).expect("Magic Hole pie_menu");
    let jump_in = menu
        .entries
        .iter()
        .find(|e| e.action.action == 0x1001)
        .or_else(|| menu.entries.first())
        .expect("Jump In entry")
        .action
        .clone();
    lot.pick(jumper, jump_in).expect("queue Jump In");
}

#[test]
fn roof_and_walls_buttons_form_mutually_exclusive_4_way_radio_group() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run Roof/Walls radio integration test");
        return;
    };
    let mut game = Game::open(&Options {
        install: root.clone(),
        house: goth_house(&root),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("open Goth house");
    game.resize([1024, 768]);
    let _ = game.render();
    assert_eq!(selected_view_buttons(game.hud_items()), vec![id::CUTAWAY]);

    // Click WALLS_DOWN -> only WALLS_DOWN lit.
    let pt_down = button_center(game.hud_items(), id::WALLS_DOWN);
    let _ = game.press(pt_down);
    let _ = game.release(pt_down);
    let _ = game.render();
    assert!(!game.roof_view());
    assert_eq!(game.walls(), Walls::Down);
    assert_eq!(
        selected_view_buttons(game.hud_items()),
        vec![id::WALLS_DOWN]
    );

    // Click ROOF -> roof ON, walls raised to Up, ONLY ROOF lit (never WALLS_DOWN + ROOF!).
    let pt_roof = button_center(game.hud_items(), id::ROOF);
    let _ = game.press(pt_roof);
    let _ = game.release(pt_roof);
    let canvas_roof = game.render();
    save_shot_if_requested("roof_view_1024x768", canvas_roof);
    assert!(game.roof_view());
    assert_eq!(game.walls(), Walls::Up);
    assert_eq!(selected_view_buttons(game.hud_items()), vec![id::ROOF]);

    // Home / End in Roof view match Sims.exe 1.91 (no-op; stays in Roof view).
    assert!(!game.step_walls(true));
    assert!(!game.step_walls(false));
    assert!(game.roof_view());
    let _ = game.render();
    assert_eq!(selected_view_buttons(game.hud_items()), vec![id::ROOF]);

    // Clicking WALLS_UP turns roof OFF and lights ONLY WALLS_UP (and redraws!).
    let pt_up = button_center(game.hud_items(), id::WALLS_UP);
    let _ = game.press(pt_up);
    let _ = game.release(pt_up);
    let _ = game.render();
    assert!(!game.roof_view());
    assert_eq!(game.walls(), Walls::Up);
    assert_eq!(selected_view_buttons(game.hud_items()), vec![id::WALLS_UP]);

    // Clicking ROOF again then CUTAWAY turns roof OFF and lights ONLY CUTAWAY.
    let _ = game.press(pt_roof);
    let _ = game.release(pt_roof);
    let _ = game.render();
    assert_eq!(selected_view_buttons(game.hud_items()), vec![id::ROOF]);

    let pt_cut = button_center(game.hud_items(), id::CUTAWAY);
    let _ = game.press(pt_cut);
    let _ = game.release(pt_cut);
    let _ = game.render();
    assert!(!game.roof_view());
    assert_eq!(game.walls(), Walls::Cutaway);
    assert_eq!(selected_view_buttons(game.hud_items()), vec![id::CUTAWAY]);

    // Verify PageUp/PageDown (floor toggle) keeps exactly 1 view button lit.
    assert!(game.toggle_floor());
    let _ = game.render();
    assert_eq!(selected_view_buttons(game.hud_items()).len(), 1);
    let _ = UiCamera::Roof;
    let _ = Command::Camera(UiCamera::Roof);
}

#[test]
fn buy_mode_has_zero_50px_blue_overhang_and_renders_clean_shots() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to verify Buy Mode strip pixels");
        return;
    };
    let mut game = Game::open(&Options {
        install: root.clone(),
        house: goth_house(&root),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("open Goth house");
    game.resize([1024, 768]);

    // Baseline Build Mode expanded frame pixels in the 50 px band above the 100 px
    // strip (`y = h - 150 .. h - 100, x = 220 .. 1024`). Both Build and Buy draw
    // the lot tile grid, so any extra 50 px Buy Mode tall-strip overhang would show
    // up immediately as non-zero differing pixels in this band.
    game.set_mode(Mode::Build);
    if !game.panel_expanded() {
        game.toggle_panel();
    }
    let build_baseline = game.render().color.clone();

    // Switch to Buy Mode (By Function top-level).
    game.set_mode(Mode::Buy);
    let buy_canvas = game.render();
    save_shot_if_requested("buy_top_by_function_1024x768", buy_canvas);

    // Every pixel in `x = 220..1024, y = (768 - 150)..(768 - 100)` must match
    // Build Mode's lot+grid view (ZERO 50 px blue tall-strip overhang!).
    let mut mismatched = 0usize;
    for y in (768 - 150)..(768 - STRIP as usize) {
        for x in 220..1024usize {
            let idx = y * 1024 + x;
            if buy_canvas.color[idx] != build_baseline[idx] {
                mismatched += 1;
            }
        }
    }
    assert_eq!(
        mismatched, 0,
        "Buy Mode leaked {mismatched} pixels into the 50 px band above the 100 px strip"
    );

    // Toggle to By Room and capture shot.
    game.hotkey_mode(Mode::Buy);
    let buy_room_canvas = game.render();
    save_shot_if_requested("buy_top_by_room_1024x768", buy_room_canvas);

    // Toggle back to By Function, open category 0 (Seating), select first item -> popup card shot.
    game.hotkey_mode(Mode::Buy);
    let _ = game.render();
    if let Some(cat_btn) = game.buy_layout().widgets.first().map(|w| w.rect) {
        let pt = [
            cat_btn.x + cat_btn.width / 2,
            cat_btn.y + cat_btn.height / 2,
        ];
        let _ = game.press(pt);
        let _ = game.release(pt);
        let cat_canvas = game.render();
        save_shot_if_requested("buy_subcategory_seating_1024x768", cat_canvas);
        if let Some(slot_rect) = game
            .buy_layout()
            .widgets
            .iter()
            .find(|w| matches!(w.kind, sims_ui::buy::BuyWidgetKind::Slot { .. }))
            .map(|w| w.rect)
        {
            let spt = [
                slot_rect.x + slot_rect.width / 2,
                slot_rect.y + slot_rect.height / 2,
            ];
            let _ = game.press(spt);
            let _ = game.release(spt);
            let item_canvas = game.render();
            save_shot_if_requested("buy_item_popup_1024x768", item_canvas);
        }
    }

    // Pick up an existing Goth house object in Buy Mode to capture the Hand Tool sell prompt.
    game.cancel_buy();
    let _ = game.render();
    'pickup: for y in (160..520).step_by(12) {
        for x in (220..760).step_by(12) {
            let _ = game.press([x, y]);
            if game.buy_cursor().is_some() {
                let sell_canvas = game.render();
                save_shot_if_requested("buy_sell_prompt_1024x768", sell_canvas);
                let _ = game.cancel_buy();
                break 'pickup;
            }
            let _ = game.release([x, y]);
        }
    }

    // Also capture Build Mode Doors/Windows/Stairs/Fireplace/Water shots so we visually confirm the OBJD word 69 fix.
    game.set_mode(Mode::Build);
    for (tool, label) in [
        (BuildTool::Doors, "build_doors_1024x768"),
        (BuildTool::Windows, "build_windows_1024x768"),
        (BuildTool::Stairs, "build_stairs_1024x768"),
        (BuildTool::Fireplace, "build_fireplace_1024x768"),
        (BuildTool::Water, "build_water_1024x768"),
    ] {
        game.select_build_tool(tool);
        let canvas = game.render();
        save_shot_if_requested(label, canvas);
    }
}

#[test]
fn magic_hole_and_downtown_pickers_use_real_graphical_destination_maps_end_to_end() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run destination map E2E test");
        return;
    };

    // 1. Verify DowntownLotPicker constructs the real graphical Downtown map (lots 21..=30).
    let install = sims_content::Install::detect(&root).expect("detect install");
    let downtown_picker = DestinationMapPicker::open(
        &install,
        1,
        SpecialPanelRequest::DowntownLotPicker,
        (21..=30)
            .map(|id| SpecialLotEntry {
                lot_id: id,
                name: format!("Downtown {id}"),
                address: Some(format!("{id} Main St")),
                description: "Downtown venue".into(),
                zoning: 2,
            })
            .collect(),
        2500,
        0,
        0,
    )
    .expect("open Downtown graphical destination map picker");
    assert_eq!(downtown_picker.kind(), NeighborhoodKind::Downtown);
    assert!(
        downtown_picker.visitable_lots().contains(&21),
        "Downtown map must include visitable lot 21: {:?}",
        downtown_picker.visitable_lots()
    );
    assert!(
        downtown_picker.hit_point_for_lot([1024, 768], 21).is_some(),
        "Downtown lot 21 must have a clickable silhouette on the graphical map"
    );

    // 2. Full NeighborhoodShell Magic Hole travel E2E:
    //    Enter Goth house -> Jump In Magic Hole -> graphical Magic Town map ->
    //    hover lot 93 (screenshot!) -> click residential lot 90 (refused) ->
    //    click community lot 93 -> real travel transition to Magic Town lot 93!
    let (_tmp, saves) = temp_redb("magic-hole-map-e2e");
    let mut shell =
        NeighborhoodShell::open(root.clone(), saves, 1, true, vec![3], Frames::default())
            .expect("open NeighborhoodShell");
    shell.resize([1024, 768]);
    shell.enter_lot(5).expect("enter Goth house");
    assert_eq!(shell.game_mut().expect("active lot").run(30), 30);

    place_and_queue_magic_hole(shell.game_mut().expect("active lot"));
    let mut reached_picker = false;
    for _ in 0..800 {
        let game = shell.game_mut().expect("active lot");
        game.run(1);
        assert!(game.stopped().is_none(), "lot halted: {:?}", game.stopped());
        if let Some(asking) = game.asking() {
            let caller = asking.shown.object;
            game.answer(caller, 0);
            continue;
        }
        if game.asking_special_panel().is_some() {
            reached_picker = true;
            break;
        }
    }
    assert!(reached_picker, "Magic Hole must trigger MagicTownLotPicker");

    shell.render();
    assert!(shell.destination_picker_open());
    {
        let game = shell.game().expect("active lot");
        assert!(
            !game.shop_open(),
            "MagicTownLotPicker must NEVER open ShopPickerDialog"
        );
        let picker = game.destination_picker().expect("Magic Town map picker");
        assert_eq!(picker.kind(), NeighborhoodKind::MagicTown);
        assert!(picker.visitable_lots().contains(&93));
        assert!(!picker.visitable_lots().contains(&90));
    }

    // Hover visitable lot 93 and render + capture screenshot of the Magic Town graphical map.
    let lot_93_pt = shell
        .game()
        .expect("active lot")
        .destination_picker()
        .expect("picker")
        .hit_point_for_lot([1024, 768], 93)
        .expect("clickable point on Magic Town lot 93");
    let _ = shell.hover(lot_93_pt);
    shell.render();
    save_shot_if_requested("magic_hole_destination_map_1024x768", shell.canvas());
    assert_eq!(
        shell
            .game()
            .expect("active lot")
            .destination_picker()
            .expect("picker")
            .hovered_lot(),
        Some(93)
    );

    // Clicking non-visitable residential lot 90 must keep the map open!
    if let Some(lot_90_pt) = shell
        .game()
        .expect("active lot")
        .destination_picker()
        .expect("picker")
        .hit_point_for_lot([1024, 768], 90)
    {
        let _ = shell.press(lot_90_pt);
        let _ = shell.release(lot_90_pt);
        assert!(
            shell.destination_picker_open(),
            "clicking non-visitable Magic Town residential lot 90 must keep destination map open"
        );
    }

    // Clicking visitable Magic Town community lot 93 travels to lot 93 cleanly!
    let _ = shell.press(lot_93_pt);
    let _ = shell.release(lot_93_pt);
    assert!(!shell.destination_picker_open());
    assert_eq!(
        shell.active_lot_house(),
        Some(93),
        "clicking Magic Town lot 93 on the graphical destination map must travel to lot 93"
    );

    // 3. Cancel a fresh Magic Hole prompt with Esc (the 1.91 dialog frame has
    //    no visible cancel button) and return cleanly to the Goth house.
    let (_tmp2, saves2) = temp_redb("magic-hole-cancel-e2e");
    let mut shell_cancel =
        NeighborhoodShell::open(root, saves2, 1, true, vec![3], Frames::default())
            .expect("open second NeighborhoodShell");
    shell_cancel.resize([1024, 768]);
    shell_cancel.enter_lot(5).expect("enter Goth house");
    assert_eq!(shell_cancel.game_mut().expect("lot").run(30), 30);
    place_and_queue_magic_hole(shell_cancel.game_mut().expect("lot"));
    for _ in 0..800 {
        let game = shell_cancel.game_mut().expect("lot");
        game.run(1);
        if let Some(asking) = game.asking() {
            let caller = asking.shown.object;
            game.answer(caller, 0);
            continue;
        }
        if game.asking_special_panel().is_some() {
            break;
        }
    }
    shell_cancel.render();
    assert!(shell_cancel.destination_picker_open());
    assert!(
        shell_cancel
            .game()
            .expect("lot")
            .destination_picker()
            .expect("picker")
            .cancel_button_rect([1024, 768])
            .is_none(),
        "the Magic Town dialog frame has no visible cancel control"
    );
    let _ = shell_cancel.escape();
    assert!(!shell_cancel.destination_picker_open());
    assert_eq!(
        shell_cancel.active_lot_house(),
        Some(5),
        "cancelling Magic Town destination map must stay cleanly on Goth house lot 5"
    );
}
