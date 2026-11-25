//! Headless integration tests for interactive UX polish:
//! - `--debug` top diagnostic banner gating (off by default, preserved programmatic getters,
//!   native `cTSWinMsgBox` user-facing notice dialogs)
//! - Zero Live-mode `CamViewAlbum` overlap + bottom-strip Camera sub-strip (native
//!   `cWinCamera` controls only, as `Sims.exe` 1.91 `camera-strip.png` shows; a snapshot is
//!   a click on the lot) + Photo Album + direct `F10` path
//! - Neighborhood idle `highlight_hovered == false` (active only in Move-In, Bulldoze, or
//!   Community Filter modes), smooth `water_frame` ripple ticking, and 275 ms hover-card
//!   debounce with immediate leave clear
//! - Smooth camera pan interpolation on portrait re-click / double-click in `Game` without
//!   altering static `clock_us == 0` headless/ratchet frames.

use crate::common::install_root as install;
use sims_game::game::{CAMERA_PAN_DURATION_US, goth_house};
use sims_game::hud::DialogKey;
use sims_game::neighborhood::{HOVER_CARD_DELAY_US, NeighborhoodShell, ScreenKind, WATER_STEP_US};
use sims_game::view::Move;
use sims_game::{Frames, Game, Options};
use sims_render::neighborhood::NeighborhoodKind;
use sims_ui::command::{Camera as UiCamera, Command};
use sims_ui::travel::{LotFilter, TravelCommand};
use std::path::PathBuf;

fn temp_redb(tag: &str) -> (PathBuf, PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("sims-game-ui-polish-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let redb = dir.join("saves.redb");
    (dir, redb)
}

fn find_lot_hit_point(shell: &NeighborhoodShell, lot_id: u16) -> Option<[i32; 2]> {
    let viewport = [800u32, 600u32];
    let map = shell.map();
    let lot = map.lot(lot_id)?;
    let [left, top, right, bottom] = map.lot_screen_rect(lot, viewport);
    let y_start = top.max(sims_ui::neighborhood::BAR_HEIGHT + 2);
    for y in (y_start..bottom.min(600)).step_by(2) {
        for x in (left.max(0)..right.min(800)).step_by(2) {
            if map.hit_test(viewport, [x, y]) == Some(lot_id) {
                return Some([x, y]);
            }
        }
    }
    None
}

fn has_maroon_top_bar(game: &Game) -> bool {
    let canvas = game.canvas();
    if canvas.width < 16 || canvas.height < 12 {
        return false;
    }
    let mut maroon = 0usize;
    let maroon_px = sims_render::pack_rgb([96, 24, 24]);
    for y in 0..12 {
        for x in 0..canvas.width {
            if canvas.color[y * canvas.width + x] == maroon_px {
                maroon += 1;
            }
        }
    }
    maroon > canvas.width * 4
}

#[test]
fn debug_banner_defaults_off_hides_maroon_bar_and_preserves_note_and_notice_dialog() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        install: install.clone(),
        house: goth_house(&install),
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .expect("Goth house opens");
    assert!(!game.debug(), "Game::debug() must default to false");
    game.resize([800, 600]);
    game.set_note("Queued \"Open\" for person 21.");
    game.render();

    // Routine note is readable programmatically, but no maroon top bar is drawn.
    assert_eq!(game.note(), Some("Queued \"Open\" for person 21."));
    assert!(
        !has_maroon_top_bar(&game),
        "maroon top bar must not render when debug == false"
    );
    assert!(
        !game.over_panel([400, 4]),
        "hidden status band must not capture clicks when debug == false"
    );

    // Enabling --debug renders the maroon top bar cleanly.
    game.set_debug(true);
    assert!(game.debug());
    game.render();
    assert!(
        has_maroon_top_bar(&game),
        "maroon top bar must render when debug == true"
    );
    assert!(
        game.over_panel([400, 4]),
        "visible debug status band covers the top strip"
    );

    // Turning --debug back off hides the banner again, while genuine user-facing
    // errors surface via native Sims 1 cTSWinMsgBox (`show_notice`).
    game.set_debug(false);
    game.run(1); // mark lot dirty
    game.show_notice("Save Failed", "The lot was not saved: disk full");
    assert_eq!(game.note(), Some("The lot was not saved: disk full"));
    assert!(
        game.has_ui_dialog(),
        "show_notice must open a native cTSWinMsgBox notice dialog"
    );
    // `after_save_failed()` must only upgrade `ConfirmQuit`, never clobber a Notice dialog.
    game.after_save_failed();
    game.render();
    assert!(
        !has_maroon_top_bar(&game),
        "show_notice with debug == false must use native dialog without maroon top bar"
    );
    let dismissed = game.dialog_confirm_key(DialogKey::Enter);
    assert!(dismissed.redraw);
    assert!(!dismissed.save && !dismissed.quit);
    assert!(
        !game.has_ui_dialog(),
        "Enter must dismiss the native notice dialog"
    );
}

#[test]
fn live_mode_has_no_cam_view_album_overlap_and_camera_button_toggles_substrip() {
    let Some(install) = install() else {
        return;
    };
    let (_tmp, saves) = temp_redb("camera");
    let mut shell = NeighborhoodShell::open(install, saves, 1, true, vec![3], Frames::default())
        .expect("open NeighborhoodShell");
    shell.resize([800, 600]);
    shell.enter_lot(5).expect("enter Goth house");
    assert_eq!(shell.screen(), ScreenKind::Lot);
    shell.render();

    // Idle Live mode: zero floating CamViewAlbum button over the clock or Camera icon.
    {
        let game = shell.game().expect("active game");
        assert!(!game.camera_strip_open());
        assert!(game.camera_strip_rect().is_none());
        assert!(game.album_button_rect().is_none());
    }

    // Click the cluster Camera button (`Command::Camera(Photo)`).
    let cam_rect = shell
        .game()
        .expect("active game")
        .panel()
        .iter()
        .find(|i| i.command == Some(Command::Camera(UiCamera::Photo)))
        .expect("cluster Camera button")
        .rect;
    let cam_pt = [
        cam_rect.x + cam_rect.width / 2,
        cam_rect.y + cam_rect.height / 2,
    ];
    let open_out = shell.press(cam_pt);
    assert!(open_out.redraw);
    assert!(
        !open_out.photo,
        "clicking Camera button toggles sub-strip instead of capturing immediately"
    );
    shell.release(cam_pt);
    shell.render();

    let (strip_rect, album_rect) = {
        let game = shell.game().expect("active game");
        assert!(game.camera_strip_open());
        let strip = game.camera_strip_rect().expect("Camera sub-strip rect");
        let album = game.album_button_rect().expect("Photo Album rect");
        // Strictly to the right of the 220px Live Mode cluster (zero overlap with clock/speed/Camera).
        assert!(
            strip.x >= sims_ui::hud::CLUSTER[0],
            "strip_rect {strip:?} overlaps Live cluster"
        );
        assert!(album.x >= sims_ui::hud::CLUSTER[0]);
        (strip, album)
    };
    assert!(strip_rect.width > 0 && strip_rect.height >= sims_ui::hud::STRIP);

    // A click on the lot takes the snapshot into the album and closes the
    // Camera sub-strip; the host writes no screenshot file for it.
    let snap_out = shell.press([400, 200]);
    shell.release([400, 200]);
    assert!(
        !snap_out.photo,
        "a lot click in Camera Mode saves to the album, not a file"
    );
    assert!(
        !shell.has_dialog(),
        "the album save succeeds, so no notice is raised"
    );
    assert!(
        !shell.game().expect("active game").camera_strip_open(),
        "the snapshot closes the Camera sub-strip"
    );

    // Re-open Camera sub-strip and click Photo Album -> opens PhotoAlbumDialog.
    shell.press(cam_pt);
    shell.release(cam_pt);
    shell.render();
    let album_pt = [
        album_rect.x + album_rect.width / 2,
        album_rect.y + album_rect.height / 2,
    ];
    shell.press(album_pt);
    shell.release(album_pt);
    assert!(
        shell.album_open(),
        "Photo Album button in Camera sub-strip must open PhotoAlbumDialog"
    );
    assert!(!shell.game().expect("active game").camera_strip_open());

    // Close Photo Album, then verify clicking Camera twice toggles open -> closed cleanly.
    shell.close_photo_album();
    assert!(!shell.album_open());
    shell.press(cam_pt);
    shell.release(cam_pt);
    assert!(shell.game().expect("active game").camera_strip_open());
    shell.press(cam_pt);
    shell.release(cam_pt);
    assert!(!shell.game().expect("active game").camera_strip_open());

    // Direct F10 entry path (`open_photo_album`) continues to work directly and
    // closes any open Camera sub-strip underneath.
    shell.press(cam_pt);
    shell.release(cam_pt);
    assert!(shell.game().expect("active game").camera_strip_open());
    shell
        .open_photo_album()
        .expect("direct F10 Photo Album open succeeds");
    assert!(shell.album_open());
    assert!(
        !shell.game().expect("active game").camera_strip_open(),
        "opening Photo Album via F10 must close the Camera sub-strip underneath"
    );
    shell.close_photo_album();

    // Compact 340px viewport keeps Camera sub-strip buttons strictly inside canvas bounds,
    // and sub-minimum widths auto-close rather than leaving an invisible strip open.
    shell.resize([340, 600]);
    shell.render();
    let cam_rect_340 = shell
        .game()
        .expect("active game")
        .panel()
        .iter()
        .find(|i| i.command == Some(Command::Camera(UiCamera::Photo)))
        .expect("cluster Camera button at 340px")
        .rect;
    let cam_pt_340 = [
        cam_rect_340.x + cam_rect_340.width / 2,
        cam_rect_340.y + cam_rect_340.height / 2,
    ];
    shell.press(cam_pt_340);
    shell.release(cam_pt_340);
    shell.render();
    {
        let game = shell.game().expect("active game");
        assert!(game.camera_strip_open());
        let album = game
            .album_button_rect()
            .expect("Photo Album button at 340px");
        assert!(
            album.x + album.width <= 340,
            "Photo Album button ({album:?}) must not clip past 340px right edge"
        );
    }
    shell.resize([260, 600]);
    shell.render();
    assert!(
        !shell.game().expect("active game").camera_strip_open(),
        "narrow viewport below minimum strip width must close Camera sub-strip cleanly"
    );
}

#[test]
fn neighborhood_idle_highlight_off_water_ripples_and_hover_debounce() {
    let Some(install) = install() else {
        return;
    };
    let (_tmp, saves) = temp_redb("nbhd");
    let mut shell = NeighborhoodShell::open(install, saves, 1, true, vec![3], Frames::default())
        .expect("open NeighborhoodShell");
    shell.resize([800, 600]);
    assert!(!shell.debug(), "NeighborhoodShell::debug defaults to false");

    // 1. `highlight_hovered` is false during plain idle residential browsing,
    //    and true only during Move-In, Bulldoze, or active Community Filter modes.
    assert!(
        !shell.should_highlight_hovered_lot(),
        "idle residential browsing must not draw yellow hover highlight"
    );
    // Bulldoze mode enables hover highlight.
    shell.press([0, 0]); // ensure clean state
    shell.handle_travel_command(TravelCommand::ToggleBulldoze);
    assert!(
        shell.should_highlight_hovered_lot(),
        "Bulldoze mode must enable hover highlight"
    );
    shell.handle_travel_command(TravelCommand::ToggleBulldoze);
    assert!(!shell.should_highlight_hovered_lot());

    // Move-In mode enables hover highlight.
    let family_id = shell
        .family_bin_entries()
        .first()
        .expect("at least one family in bin")
        .id as u16;
    shell.start_move_in(family_id).expect("start Move-In");
    assert!(
        shell.should_highlight_hovered_lot(),
        "Move-In mode must enable hover highlight"
    );
    shell.escape();
    assert!(!shell.should_highlight_hovered_lot());

    // Active Community Filter mode on a destination map enables hover highlight.
    shell
        .switch_kind(NeighborhoodKind::Downtown)
        .expect("switch to Downtown");
    assert!(
        !shell.should_highlight_hovered_lot(),
        "idle Downtown browsing without active filter must not draw yellow hover highlight"
    );
    shell.handle_travel_command(TravelCommand::SetFilter(Some(LotFilter::Food)));
    assert!(
        shell.should_highlight_hovered_lot(),
        "active Community Filter mode must enable hover highlight"
    );
    shell.handle_travel_command(TravelCommand::SetFilter(None));
    assert!(!shell.should_highlight_hovered_lot());
    shell.handle_travel_command(TravelCommand::ReturnToNeighborhood);

    // 2. Water ripple animation ticks smoothly across `tick_shell`.
    let start_wave = shell.water_frame();
    let has_waves = shell.map().waves.iter().any(Option::is_some);
    let mut saw_redraw = false;
    for step in 1..=4 {
        saw_redraw |= shell.tick_shell(WATER_STEP_US);
        assert_eq!(shell.water_frame(), start_wave + step);
    }
    if has_waves {
        assert!(
            saw_redraw,
            "water ripple animation must request redraw when wave frames exist"
        );
    }

    // 3. Hover-card debounce: steady hover pops after HOVER_CARD_DELAY_US and
    //    clears immediately upon leaving the lot.
    let goth_pt = find_lot_hit_point(&shell, 5).expect("Goth lot 5 hit point");
    assert!(shell.hover(goth_pt));
    assert_eq!(shell.hovered_lot_id(), Some(5));
    assert!(
        shell.neighborhood_state().hovered_lot.is_none(),
        "sweeping onto a lot must not flash the House Info card immediately"
    );
    shell.tick_shell(HOVER_CARD_DELAY_US - 50_000);
    assert!(
        shell.neighborhood_state().hovered_lot.is_none(),
        "hover card must remain hidden before HOVER_CARD_DELAY_US elapses"
    );
    assert!(shell.tick_shell(50_000));
    assert_eq!(
        shell
            .neighborhood_state()
            .hovered_lot
            .as_ref()
            .map(|info| info.lot_number),
        Some(5),
        "steady hover past HOVER_CARD_DELAY_US must pop the House Info card"
    );
    assert!(shell.leave());
    assert_eq!(shell.hovered_lot_id(), None);
    assert!(
        shell.neighborhood_state().hovered_lot.is_none(),
        "leaving the lot must clear the House Info card immediately"
    );

    // 4. Neighborhood top status banner is gated behind `--debug`, while
    //    `status_note()` remains readable programmatically.
    shell.set_note("Diagnostic test banner");
    assert_eq!(shell.status_note(), Some("Diagnostic test banner"));
    shell.render();
    let hidden_pixels = shell.canvas().color.clone();
    shell.set_debug(true);
    shell.render();
    let debug_pixels = shell.canvas().color.clone();
    assert_ne!(
        hidden_pixels, debug_pixels,
        "enabling --debug must render the top diagnostic banner on Neighborhood screen"
    );
}

#[test]
fn portrait_reclick_smooth_camera_pan_preserves_static_headless_frames() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        install: install.clone(),
        house: goth_house(&install),
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .expect("Goth house opens");
    assert_eq!(game.run(50), 50);
    game.resize([800, 600]);
    game.render();

    let selected = game.lot().world().selected().expect("selected Sim");
    let portrait = game
        .panel()
        .iter()
        .find(|i| i.command == Some(Command::Select { sim: selected }))
        .expect("selected Sim portrait")
        .rect;
    let portrait_pt = [
        portrait.x + portrait.width / 2,
        portrait.y + portrait.height / 2,
    ];

    // 1. At `clock_us == 0` (static headless/ratchet replay), portrait re-click snaps
    //    immediately to the exact centred target origin.
    game.apply(Move::Scroll([320, 180]));
    let scrolled_origin = game.view.camera.origin;
    game.press(portrait_pt);
    assert!(
        !game.camera_panning(),
        "clock_us == 0 headless mode must snap without activating smooth pan"
    );
    let exact_target = game.view.camera.origin;
    assert_ne!(exact_target, scrolled_origin);

    // 2. Once presentation clock is running (`clock_us > 0`, interactive desktop play),
    //    re-clicking / double-clicking the portrait smoothly interpolates over
    //    `CAMERA_PAN_DURATION_US` (even while paused) and lands on the exact same origin.
    game.apply(Move::Scroll([320, 180]));
    let start_origin = game.view.camera.origin;
    game.advance_while(16_000, 0, |_| true);
    game.press(portrait_pt);
    assert!(
        game.camera_panning(),
        "interactive portrait re-click must start smooth camera pan"
    );
    assert_eq!(
        game.view.camera.origin, start_origin,
        "pan starts smoothly from current origin instead of snapping"
    );

    // Mid-pan: strictly between `start_origin` and `exact_target`.
    game.advance_while(CAMERA_PAN_DURATION_US / 2, 0, |_| true);
    assert!(game.camera_panning());
    let mid = game.view.camera.origin;
    assert_ne!(mid, start_origin);
    assert_ne!(mid, exact_target);

    // Completion: lands on exact target origin and clears panning state.
    game.advance_while(CAMERA_PAN_DURATION_US, 0, |_| true);
    assert!(!game.camera_panning());
    assert_eq!(game.view.camera.origin, exact_target);

    // 3. Manual camera input mid-pan immediately cancels smooth interpolation.
    game.apply(Move::Scroll([200, 100]));
    game.press(portrait_pt);
    assert!(game.camera_panning());
    game.apply(Move::Scroll([16, 0]));
    assert!(
        !game.camera_panning(),
        "manual scroll must cancel an in-flight smooth camera pan"
    );
}
