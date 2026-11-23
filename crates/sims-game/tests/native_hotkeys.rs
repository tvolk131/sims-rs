//! Headless integration tests for authentic Sims 1.91 hotkeys and zero
//! gray-patch HUD compositing on the Goth house (`SIMS_INSTALL`).

use sims_game::game::goth_house;
use sims_game::{Frames, Game, Options};
use sims_render::{Walls, png};
use sims_ui::command::{Mode, Speed};
use std::path::PathBuf;

fn install_root() -> Option<PathBuf> {
    std::env::var_os("SIMS_INSTALL").map(PathBuf::from)
}

fn open_goth(root: &std::path::Path) -> Game {
    Game::open(&Options {
        house: goth_house(root),
        install: root.to_path_buf(),
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .expect("open Goth house")
}

#[test]
fn native_hotkeys_control_pause_speed_modes_sim_cycling_walls_and_floors() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to test native hotkeys on the Goth house");
        return;
    };
    let mut game = open_goth(&root);

    // 1. Pause toggle (` / P) and Speed 1/2/3 unpausing.
    assert!(!game.paused());
    game.toggle_pause();
    assert!(game.paused(), "toggle_pause must pause live lot");
    game.set_speed(Speed::Fast);
    assert!(!game.paused(), "Speed 2 must unpause");
    assert_eq!(game.speed(), Speed::Fast);
    game.set_speed(Speed::Ultra);
    assert_eq!(game.speed(), Speed::Ultra);
    game.set_speed(Speed::Normal);
    assert_eq!(game.speed(), Speed::Normal);

    // 2. Space: cycle through all 3 Goths (Mortimer 21 -> Bella 18 -> Cassandra 16 -> 21).
    let _ = game.take_sounds();
    let first = game.local_selected_sim().expect("initial selected Sim");
    assert!(game.select_next_sim());
    assert!(
        game.take_sounds().iter().any(
            |s| matches!(s, sims_vm::SoundEvent::Play { name, .. } if name == sims_game::hud::CLICK)
        ),
        "Space next-Sim switch must flush UI_click immediately"
    );
    let second = game.local_selected_sim().expect("second Sim");
    assert_ne!(second, first);
    assert!(game.select_next_sim());
    let third = game.local_selected_sim().expect("third Sim");
    assert_ne!(third, second);
    assert_ne!(third, first);
    assert!(game.select_next_sim());
    assert_eq!(
        game.local_selected_sim(),
        Some(first),
        "Space must wrap around the household roster"
    );

    // 3. Enter: centre camera on selected Sim.
    assert!(game.centre_on_selected());

    // 4. Home / End: step walls Up (Down -> Cutaway -> Up) and Down (Up -> Cutaway -> Down).
    let _ = game.apply(sims_game::view::Move::Walls(Walls::Down));
    assert_eq!(game.walls(), Walls::Down);
    let _ = game.take_sounds();
    assert!(
        !game.step_walls(false),
        "End at Walls::Down is a boundary no-op"
    );
    assert!(
        game.take_sounds().iter().any(
            |s| matches!(s, sims_vm::SoundEvent::Play { name, .. } if name == sims_game::hud::ERROR)
        ),
        "wall boundary no-op must flush UI_error immediately"
    );
    assert!(game.step_walls(true), "Home steps Down -> Cutaway");
    assert_eq!(game.walls(), Walls::Cutaway);
    assert!(game.step_walls(true), "Home steps Cutaway -> Up");
    assert_eq!(game.walls(), Walls::Up);
    assert!(
        !game.step_walls(true),
        "Home at Walls::Up is a boundary no-op"
    );
    assert!(game.step_walls(false), "End steps Up -> Cutaway");
    assert_eq!(game.walls(), Walls::Cutaway);
    assert!(game.step_walls(false), "End steps Cutaway -> Down");
    assert_eq!(game.walls(), Walls::Down);

    // 5. Tab: toggle floor 1 <-> 2.
    let initial_floor = game.level();
    assert!(game.toggle_floor());
    assert_ne!(game.level(), initial_floor);
    assert!(game.toggle_floor());
    assert_eq!(game.level(), initial_floor);

    // 6. F1 / F2 / F3 / F4 / F5 / F10 mode and panel transitions.
    assert_eq!(game.mode(), Mode::Live);
    let was_expanded = game.panel_expanded();
    game.hotkey_mode(Mode::Live); // F1 while already in Live toggles panel
    assert_eq!(game.panel_expanded(), !was_expanded);
    game.toggle_panel(); // F10 toggles back
    assert_eq!(game.panel_expanded(), was_expanded);

    game.hotkey_mode(Mode::Buy); // F2 -> Buy
    assert_eq!(game.mode(), Mode::Buy);
    assert!(game.panel_expanded());
    // Space (select_next_sim) must NOT switch Sims outside Live Mode.
    let sim_before_buy_space = game.local_selected_sim();
    assert!(!game.select_next_sim());
    assert_eq!(game.local_selected_sim(), sim_before_buy_space);

    let sort_before = game.buy_state().sort;
    game.hotkey_mode(Mode::Buy); // F2 again -> toggles ByFunction <-> ByRoom
    assert_ne!(game.buy_state().sort, sort_before);

    game.hotkey_mode(Mode::Build); // F3 -> Build
    assert_eq!(game.mode(), Mode::Build);
    assert!(!game.select_next_sim());

    game.toggle_camera_strip(); // F4 -> Camera strip
    assert!(game.camera_strip_open());
    game.hotkey_mode(Mode::Options); // F5 -> Options (closes Camera strip)
    assert!(!game.camera_strip_open());
    assert_eq!(game.mode(), Mode::Options);
    assert!(!game.select_next_sim());

    // Rejected Camera open on a sub-minimum canvas must not expand a collapsed HUD.
    game.hotkey_mode(Mode::Live); // F1 -> back to Live
    assert_eq!(game.mode(), Mode::Live);
    if game.panel_expanded() {
        assert!(game.toggle_panel());
    }
    assert!(!game.panel_expanded());
    game.resize([250, 200]);
    let _ = game.toggle_camera_strip();
    assert!(!game.camera_strip_open());
    assert!(
        !game.panel_expanded(),
        "rejected Camera open on narrow canvas must not leave HUD expanded"
    );
    game.resize([800, 600]);

    // P1 regression: an active Buy drag must defer F10/F4 panel toggles so
    // releasing the pointer at the bottom edge cannot trigger an unintended sale.
    game.hotkey_mode(Mode::Buy);
    assert_eq!(game.mode(), Mode::Buy);
    assert!(game.panel_expanded());
    let _ = game.render();
    // Pick up an existing object in the Goth house via Buy Mode Hand Tool drag.
    let mut picked_up = false;
    'scan: for y in (160..460).step_by(12) {
        for x in (200..640).step_by(12) {
            let _ = game.press([x, y]);
            if game.buy_cursor().is_some() {
                picked_up = true;
                break 'scan;
            }
            let _ = game.release([x, y]);
        }
    }
    assert!(
        picked_up,
        "expected to pick up an existing Goth house object in Buy Mode"
    );
    let before_toggle = game.panel_expanded();
    assert!(
        !game.toggle_panel(),
        "F10 toggle_panel must defer while a Buy drag is in progress"
    );
    assert_eq!(game.panel_expanded(), before_toggle);
    assert!(
        !game.toggle_camera_strip(),
        "F4 toggle_camera_strip must defer while a Buy drag is in progress"
    );
    assert!(game.cancel_buy());
}

#[test]
fn expanded_hud_renders_zero_gray_background_leak_next_to_camera_button() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to render the Goth house HUD");
        return;
    };
    let mut game = open_goth(&root);
    let shot_dir = std::env::var_os("SIMS_HUD_SHOTS_DIR").map(PathBuf::from);
    if let Some(dir) = &shot_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    let save_shot = |name: &str, canvas: &sims_render::Canvas| {
        if let Some(dir) = &shot_dir {
            let path = dir.join(format!("{name}.png"));
            let _ = std::fs::write(path, png::encode(canvas));
        }
    };

    let bg = sims_render::pack_rgb(sims_render::scene::BACKGROUND);
    for [w, h] in [[800, 600], [1024, 768], [1280, 720], [1600, 900]] {
        game.resize([w, h]);
        for (mode, label) in [
            (Mode::Live, "live_needs"),
            (Mode::Buy, "buy"),
            (Mode::Build, "build"),
            (Mode::Options, "options"),
        ] {
            game.set_mode(mode);
            if !game.panel_expanded() {
                game.toggle_panel();
            }
            let canvas = game.render();
            save_shot(&format!("{w}x{h}_{label}"), canvas);
            // The curved cluster silhouette next to Camera/Options (`x = 168..220`,
            // `y = h - 100..h`) must be 100% covered by `BackPatch.bmp` + `UniversalBack.TGA`
            // with zero lot void `BACKGROUND` (`115, 113, 115`) pixels leaking through.
            let mut leaked = 0usize;
            for y in (h as usize - 100)..(h as usize) {
                for x in 168..220usize {
                    if canvas.color[y * (w as usize) + x] == bg {
                        leaked += 1;
                    }
                }
            }
            assert_eq!(
                leaked, 0,
                "lot BACKGROUND gray leaked at {w}x{h} in {mode:?} ({leaked} px in 168..220)"
            );
        }

        // Also click through Live subpanel tabs (Relationships, Personality, Job, House)
        // at fixed gadget x = 413 to confirm zero teleporting and zero gray leak.
        game.set_mode(Mode::Live);
        if !game.panel_expanded() {
            game.toggle_panel();
        }
        let _ = game.render();
        // Native gadget button centers relative to gadget origin `(413, h - 100)`:
        // Relationships `(76, 15)`, Personality `(50, 48)`, Job `(76, 48)`, House `(76, 80)`.
        for (tab_name, [gx, gy]) in [
            ("live_relationships", [76, 15]),
            ("live_personality", [50, 48]),
            ("live_job", [76, 48]),
            ("live_house", [76, 80]),
        ] {
            let pt = [413 + gx, h - 100 + gy];
            let _ = game.press(pt);
            let _ = game.release(pt);
            let canvas = game.render();
            save_shot(&format!("{w}x{h}_{tab_name}"), canvas);
            let mut leaked = 0usize;
            for y in (h as usize - 100)..(h as usize) {
                for x in 168..220usize {
                    if canvas.color[y * (w as usize) + x] == bg {
                        leaked += 1;
                    }
                }
            }
            assert_eq!(
                leaked, 0,
                "lot BACKGROUND gray leaked at {w}x{h} in {tab_name} ({leaked} px)"
            );
        }
        // Reset back to Needs (`(18, 50)` inside gadget).
        let needs_pt = [413 + 18, h - 100 + 50];
        let _ = game.press(needs_pt);
        let _ = game.release(needs_pt);

        // Camera Mode opened even while strip is collapsed also covers the wedge via CameraPatch.
        game.set_mode(Mode::Live);
        if game.panel_expanded() {
            game.toggle_panel();
        }
        game.toggle_camera_strip();
        let canvas = game.render();
        save_shot(&format!("{w}x{h}_camera"), canvas);
        let mut leaked = 0usize;
        for y in (h as usize - 100)..(h as usize) {
            for x in 168..220usize {
                if canvas.color[y * (w as usize) + x] == bg {
                    leaked += 1;
                }
            }
        }
        assert_eq!(
            leaked, 0,
            "lot BACKGROUND gray leaked in Camera Mode at {w}x{h} ({leaked} px)"
        );
        game.close_camera_strip();
    }
}
