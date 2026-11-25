//! Headless integration tests for wiring PR #373 (special `Dialog` kinds `5..=15`,
//! single-player + co-op lockstep + inline books) and PR #371 (native `CameraPanel`,
//! 300 ms Neighborhood hover roof cross-fade, and temporal UI animations) into `sims-game`.

use crate::common::install_root as install;
use sims_game::coop::{CoopStartOptions, CoopTransport};
use sims_game::game::goth_house;
use sims_game::hud::{DIALOG_FADE_DURATION_MS, DialogKey};
use sims_game::neighborhood::NeighborhoodShell;
use sims_game::{Frames, Game, Options};
use sims_net::{ManifestDigest, MemoryMesh, SessionConfig, Transport};
use sims_ui::balloon::FADE_DURATION_MS as BALLOON_FADE_MS;
use sims_ui::camera::{CameraButton, SnapshotQuality, SnapshotSize};
use sims_ui::command::{Camera as UiCamera, Command, Mode, Panel as UiPanel};
use sims_ui::queue::SLIDE_DURATION_MS;
use sims_vm::{SpecialPanelRequest, SpecialPanelResponse};
use std::path::PathBuf;

const MAGIC_PORTAL_GUID: u32 = 0x2baa_0029;

fn temp_redb(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "sims-game-sp-cam-anim-{tag}-{}",
        std::process::id()
    ));
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

/// Places the Goth Magic Hole (`0x2baa_0029`) near the selected Sim and queues
/// "Jump In" (`0x1001`), returning `(jumper_id, magic_hole_id)`.
fn place_and_queue_magic_hole(game: &mut Game) -> (u16, u16) {
    let jumper = game.lot().world().selected().expect("selected Goth Sim");
    let sim_pos = game.lot().world().objects[&jumper].position;

    let lot = game.lot_mut();
    lot.set_mode(sims_vm::PanelMode::Buy)
        .expect("enter Buy mode");
    let mut cursor = lot
        .buy_start(MAGIC_PORTAL_GUID, 0, 0)
        .expect("buy_start Magic Hole")
        .expect("Magic Hole cursor created");
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
    assert!(placed, "must find a valid nearby tile for Magic Hole");
    lot.set_mode(sims_vm::PanelMode::Live)
        .expect("return to Live mode");
    lot.select(jumper).expect("select jumper");
    let menu = lot
        .pie_menu(jumper, hole_id)
        .expect("Magic Hole pie_menu succeeds");
    let jump_in = menu
        .entries
        .iter()
        .find(|e| e.action.action == 0x1001)
        .or_else(|| menu.entries.first())
        .expect("Magic Hole offers Jump In (0x1001)")
        .action
        .clone();
    lot.pick(jumper, jump_in).expect("queue Jump In");
    (jumper, hole_id)
}

#[test]
fn goth_magic_hole_opens_kind13_picker_in_game_and_answers_cleanly() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        install: install.clone(),
        house: goth_house(&install),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("Goth house opens");
    game.resize([800, 600]);
    assert_eq!(game.run(30), 30);

    place_and_queue_magic_hole(&mut game);

    let mut reached_picker = false;
    for _ in 0..800 {
        game.run(1);
        assert!(
            game.stopped().is_none(),
            "Goth Magic Hole must never halt the lot: {:?}",
            game.stopped()
        );
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
    assert!(
        reached_picker,
        "Goth Magic Hole 'Jump In' must pause on Dialog kind 13 (MagicTownLotPicker)"
    );

    // Render to sync and draw the real graphical Magic Town destination map.
    game.render();
    assert!(game.has_dialog(), "special panel counts as an active modal");
    assert_eq!(
        game.active_special_panel_request(),
        Some(&SpecialPanelRequest::MagicTownLotPicker { via_blimp: false })
    );
    assert!(
        !game.shop_open(),
        "MagicTownLotPicker must NOT open fake ShopPickerDialog"
    );
    assert!(
        game.destination_picker_open(),
        "MagicTownLotPicker must open real graphical DestinationMapPicker"
    );
    let picker = game.destination_picker().expect("destination map picker");
    assert_eq!(
        picker.kind(),
        sims_render::neighborhood::NeighborhoodKind::MagicTown
    );
    assert!(
        picker.visitable_lots().contains(&93),
        "Magic Town destination map must include visitable community lot 93: {:?}",
        picker.visitable_lots()
    );
    assert!(
        !picker.visitable_lots().contains(&90),
        "Magic Town residential lot 90 must NOT be a visitable travel target"
    );

    // Clicking real visitable lot 93 on the graphical map delivers
    // SpecialPanelResponse::Lot(Some(93)) cleanly without halting and records
    // the pending travel destination for NeighborhoodShell.
    let lot_93_pt = picker
        .hit_point_for_lot([800, 600], 93)
        .expect("clickable screen point on Magic Town lot 93");
    let _ = game.hover(lot_93_pt);
    let _ = game.press(lot_93_pt);
    let out = game.release(lot_93_pt);
    assert!(out.redraw);
    assert!(game.stopped().is_none());
    assert!(game.asking_special_panel().is_none());
    assert!(!game.destination_picker_open());
    let pending = game
        .take_pending_travel_lot()
        .expect("picking a Magic Town lot must queue travel transition");
    assert_eq!(
        pending.0,
        SpecialPanelRequest::MagicTownLotPicker { via_blimp: false }
    );
    assert_eq!(pending.1, 93);
    assert_eq!(game.lot().special_panels().magic_lot, 93);

    // Also verify cancelling a Magic Town picker via Escape (`destination_picker_escape`)
    // cleanly answers `SpecialPanelResponse::Lot(None)` without halting or stranding.
    let mut game_cancel = Game::open(&Options {
        install: install.clone(),
        house: goth_house(&install),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("second Goth house opens");
    game_cancel.resize([800, 600]);
    assert_eq!(game_cancel.run(30), 30);
    place_and_queue_magic_hole(&mut game_cancel);
    for _ in 0..800 {
        game_cancel.run(1);
        if let Some(asking) = game_cancel.asking() {
            let caller = asking.shown.object;
            game_cancel.answer(caller, 0);
            continue;
        }
        if game_cancel.asking_special_panel().is_some() {
            break;
        }
    }
    assert!(game_cancel.asking_special_panel().is_some());
    game_cancel.render();
    assert!(game_cancel.destination_picker_open());
    assert!(!game_cancel.shop_open());
    let cancel_out = game_cancel.destination_picker_escape();
    assert!(cancel_out.redraw);
    assert!(game_cancel.stopped().is_none());
    assert!(game_cancel.asking_special_panel().is_none());
    assert!(!game_cancel.destination_picker_open());
    assert!(
        game_cancel.take_pending_travel_lot().is_none(),
        "cancelling MagicTownLotPicker must not trigger travel"
    );
    assert_eq!(game_cancel.lot().special_panels().magic_lot, 0);
}

#[test]
fn coop_lockstep_special_panel_opens_and_answers_via_special_panel_answer_command() {
    let Some(install) = install() else {
        return;
    };
    let mut host = Game::open(&Options {
        install: install.clone(),
        house: goth_house(&install),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("host Goth opens");
    let mut peer = Game::open(&Options {
        install: install.clone(),
        house: goth_house(&install),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("peer Goth opens");
    host.resize([800, 600]);
    peer.resize([800, 600]);

    assert_eq!(host.run(30), 30);
    place_and_queue_magic_hole(&mut host);

    let mesh = MemoryMesh::new(42);
    let host_tx = mesh.add_node("mem://host");
    let host_addr = host_tx.local_address().clone();
    let peer_tx = mesh.add_node("mem://peer");
    let manifest = ManifestDigest([0x5A; 32]);

    host.coop_start_host_with_transport(
        CoopTransport::memory(host_tx),
        manifest,
        CoopStartOptions {
            player_name: "Host".into(),
            session_config: SessionConfig {
                fingerprint_interval: 4,
                ..SessionConfig::default()
            },
            prime_tick_on_start: true,
            ..CoopStartOptions::default()
        },
    )
    .expect("start host co-op");
    peer.coop_start_join_with_transport(
        CoopTransport::memory(peer_tx),
        host_addr,
        manifest,
        "Peer",
    )
    .expect("start peer co-op");

    let pump_pair = |mesh: &MemoryMesh, host: &mut Game, peer: &mut Game, schedule_host: bool| {
        let _ = mesh.step_n(4);
        let h = host.coop_pump(schedule_host, true).expect("host pump");
        let _ = mesh.step_n(4);
        let p = peer.coop_pump(false, true).expect("peer pump");
        let _ = mesh.step_n(4);
        let h2 = host.coop_pump(false, true).expect("host follow-up");
        let _ = mesh.step_n(4);
        let p2 = peer.coop_pump(false, true).expect("peer follow-up");
        (
            h.drift_events + h2.drift_events,
            p.drift_events + p2.drift_events,
        )
    };

    // Complete handshake + initial snapshot transfer.
    for _ in 0..40 {
        pump_pair(&mesh, &mut host, &mut peer, true);
        if host.coop_active()
            && peer.coop_active()
            && peer.coop_controller().snapshot_restore_count() >= 1
        {
            break;
        }
    }
    assert!(host.coop_active() && peer.coop_active());

    // Step lockstep until the Magic Hole opens Dialog kind 13 across both peers.
    let mut opened_on_both = false;
    for _ in 0..800 {
        pump_pair(&mesh, &mut host, &mut peer, true);
        if let Some(asking) = host.asking() {
            let caller = asking.shown.object;
            host.answer(caller, 0);
            continue;
        }
        if host.asking_special_panel().is_some() && peer.asking_special_panel().is_some() {
            opened_on_both = true;
            break;
        }
    }
    assert!(
        opened_on_both,
        "Magic Hole special panel must open in lockstep on both host and peer"
    );

    host.render();
    peer.render();
    assert!(
        host.destination_picker_open() && !host.shop_open(),
        "host must open graphical destination map picker (never ShopPickerDialog)"
    );
    assert!(
        peer.destination_picker_open() && !peer.shop_open(),
        "peer must open graphical destination map picker (never ShopPickerDialog)"
    );
    assert_eq!(
        peer.active_special_panel_request(),
        Some(&SpecialPanelRequest::MagicTownLotPicker { via_blimp: false })
    );

    // Peer submits its choice through `answer_special_panel`, which dispatches
    // `sims_vm::coop::special_panel_answer_command(...)` over lockstep!
    assert!(peer.answer_special_panel(SpecialPanelResponse::Lot(Some(93))));
    assert!(
        !peer.destination_picker_open(),
        "peer local destination map closes immediately after submitting answer"
    );

    // Pump lockstep batches so both peers apply the answer deterministically.
    for _ in 0..20 {
        let (hd, pd) = pump_pair(&mesh, &mut host, &mut peer, true);
        assert_eq!(hd, 0, "host must have zero co-op drift");
        assert_eq!(pd, 0, "peer must have zero co-op drift");
        if host.asking_special_panel().is_none() && peer.asking_special_panel().is_none() {
            break;
        }
    }
    assert!(host.asking_special_panel().is_none());
    assert!(peer.asking_special_panel().is_none());
    assert!(
        !host.destination_picker_open(),
        "host destination map must auto-close once peer's lockstep answer resolves the special panel"
    );
    assert_eq!(
        host.lot().world().hash(),
        peer.lot().world().hash(),
        "host and peer worlds must remain bit-identical after special panel resolution"
    );
    host.leave_coop();
    peer.leave_coop();
}

#[test]
fn native_camera_panel_wires_all_8_buttons_strings_album_and_clean_capture() {
    let Some(install) = install() else {
        return;
    };
    let (_tmp, saves) = temp_redb("camera-panel");
    let mut shell = NeighborhoodShell::open(install, saves, 1, true, vec![3], Frames::default())
        .expect("open NeighborhoodShell");
    shell.resize([800, 600]);
    shell.enter_lot(5).expect("enter Goth house");
    shell.render();

    // Open native cWinCamera subpanel via cluster Camera button.
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
    shell.press(cam_pt);
    shell.release(cam_pt);
    shell.render();

    let (large_rect, low_rect, album_rect) = {
        let game = shell.game().expect("active game");
        assert!(game.camera_strip_open());
        let layout = game.camera_layout().expect("native CameraLayout");
        assert!(!layout.compact, "800x600 uses full native 1.91 layout");
        assert_eq!(
            layout.buttons.len(),
            8,
            "all 8 native cWinCamera buttons must be laid out"
        );
        assert!(
            layout.strip.x >= sims_ui::hud::CLUSTER[0],
            "CameraPanel strip must never overlap Live cluster/clock"
        );
        // Verify STR# 140 strings loaded from SIMS_INSTALL (no empty tooltips).
        assert!(
            !game
                .camera_strings()
                .tooltip(CameraButton::ViewAlbum)
                .is_empty(),
            "STR# 140 ViewAlbum tooltip must be loaded from SIMS_INSTALL"
        );
        assert!(
            !game.camera_strings().size_help_title.is_empty(),
            "STR# 140 Snapshot Sizes Quick Tips title must be loaded from SIMS_INSTALL"
        );
        let large = game
            .camera_button_rect(CameraButton::Large)
            .expect("Large button rect");
        let low = game
            .camera_button_rect(CameraButton::QualityLow)
            .expect("QualityLow button rect");
        let album = game
            .camera_button_rect(CameraButton::ViewAlbum)
            .expect("ViewAlbum (1105) button rect");
        assert_eq!(Some(album), game.album_button_rect());
        (large, low, album)
    };

    // Hover Large button: with Play Options Quick Tips enabled, the native
    // Quick Tips help balloon (`STR# 140` Snapshot Sizes) is active.
    let large_pt = [
        large_rect.x + large_rect.width / 2,
        large_rect.y + large_rect.height / 2,
    ];
    shell.hover(large_pt);
    shell.render();
    {
        let game = shell.game().expect("active game");
        assert_eq!(game.camera_panel().hovered(), Some(CameraButton::Large));
        let state = game.camera_state();
        if state.quick_tips {
            assert!(
                game.camera_panel().quick_tip_command(&state).is_some(),
                "hovering Large with Quick Tips enabled must produce a Quick Tips help topic"
            );
        }
    }

    // Toggle snapshot size -> Large (keeps CameraPanel open).
    shell.press(large_pt);
    let out_size = shell.release(large_pt);
    assert!(out_size.redraw && !out_size.photo);
    assert!(shell.game().unwrap().camera_strip_open());
    assert_eq!(
        shell.game().unwrap().camera_state().size,
        SnapshotSize::Large
    );
    shell.render();
    // The Large frame is 600x400 (`0x6f0ef0`), centred on the clicked
    // point and ending at its row (`0x4b14a1`); a frame above the top of
    // the viewport is shifted down into it. The photo is the frame less
    // its one-pixel outline (`0x4d1f6e..0x4d1f98`): 598x400.
    let click = [400, 300];
    shell.game_mut().unwrap().hover(click);
    let large_cap = shell.game_mut().unwrap().take_photo();
    {
        let game = shell.game().unwrap();
        let canvas = game.canvas();
        let (view_w, view_h) = game.house_viewport_size();
        assert!(
            (view_h as usize) < canvas.height,
            "the panel is open below the viewport"
        );
        assert_eq!(
            sims_ui::camera::snapshot_frame(
                click,
                [600, 400],
                [0, 0, view_w as i32, view_h as i32]
            ),
            [99, 0, 699, 402]
        );
        assert_eq!(
            (large_cap.width, large_cap.height),
            (598, 400),
            "Large SnapshotSize must crop to the native 600x400 frame less its outline"
        );
        // The photo is the house view inside the frame, never the panel
        // below it: its rows match the frame's own rows of the screen. Only
        // the upper rows are compared: the Large button's Quick Tips
        // balloon, which the photo leaves out, covers part of the lower
        // viewport.
        for y in 0..150 {
            let row = (1 + y) * canvas.width + 100;
            assert_eq!(
                &large_cap.color[y * 598..(y + 1) * 598],
                &canvas.color[row..row + 598],
                "photo row {y} must be screen row {}",
                1 + y
            );
        }
        // A click near the bottom-right corner shifts the frame left into
        // the viewport; it still ends at the clicked row.
        let (view_w, view_h) = (view_w as i32, view_h as i32);
        let near_edge = sims_ui::camera::snapshot_capture_rect(
            [view_w - 5, view_h - 10],
            [600, 400],
            [0, 0, view_w, view_h],
        );
        assert_eq!(
            near_edge,
            [view_w - 600, view_h - 411, view_w - 2, view_h - 11]
        );
    }

    // Toggle snapshot quality -> Low (keeps CameraPanel open).
    let low_pt = [
        low_rect.x + low_rect.width / 2,
        low_rect.y + low_rect.height / 2,
    ];
    shell.press(low_pt);
    let out_qual = shell.release(low_pt);
    assert!(out_qual.redraw && !out_qual.photo);
    assert!(shell.game().unwrap().camera_strip_open());
    assert_eq!(
        shell.game().unwrap().camera_state().quality,
        SnapshotQuality::Low
    );

    // Click native CamViewAlbum (1105) -> opens Photo Album and closes CameraPanel.
    let album_pt = [
        album_rect.x + album_rect.width / 2,
        album_rect.y + album_rect.height / 2,
    ];
    shell.press(album_pt);
    shell.release(album_pt);
    assert!(shell.album_open());
    assert!(!shell.game().unwrap().camera_strip_open());
    let photos_before = shell.album_snapshot().map_or(0, |a| a.entries.len());
    shell.close_photo_album();

    // Re-open CameraPanel, then click the house viewport: the click closes
    // CameraPanel and stores the photo in the album, and only there
    // (`cWinCamera`): the host is not asked to write a file.
    shell.press(cam_pt);
    shell.release(cam_pt);
    shell.render();
    assert!(shell.game().unwrap().camera_strip_open());
    let viewport_snap = shell.press([400, 200]);
    assert!(
        !viewport_snap.photo,
        "a Camera Mode click saves to the album, not a screenshot file"
    );
    shell
        .open_photo_album()
        .expect("open the album after the photo");
    assert_eq!(
        shell.album_snapshot().map_or(0, |a| a.entries.len()),
        photos_before + 1,
        "clicking the house viewport in Camera Mode stores one album photo"
    );
    shell.close_photo_album();
    assert!(
        !shell.game().unwrap().camera_strip_open(),
        "viewport snapshot capture must close CameraPanel before PNG render"
    );

    // Mode switch & Escape both close CameraPanel cleanly.
    shell.press(cam_pt);
    shell.release(cam_pt);
    assert!(shell.game().unwrap().camera_strip_open());
    shell.game_mut().unwrap().set_mode(Mode::Buy);
    assert!(!shell.game().unwrap().camera_strip_open());
    shell.game_mut().unwrap().set_mode(Mode::Live);
    shell.press(cam_pt);
    shell.release(cam_pt);
    assert!(shell.game().unwrap().camera_strip_open());
    shell.escape();
    assert!(!shell.game().unwrap().camera_strip_open());
}

#[test]
fn neighborhood_hover_roof_fade_transitions_over_300ms_and_settles_idle() {
    let Some(install) = install() else {
        return;
    };
    let (_tmp, saves) = temp_redb("nbhd-roof-fade");
    let mut shell = NeighborhoodShell::open(install, saves, 1, true, vec![3], Frames::default())
        .expect("open NeighborhoodShell");
    shell.resize([800, 600]);

    let goth_pt = find_lot_hit_point(&shell, 5).expect("Goth lot 5 hit point");
    let other_lot = (1u16..=10)
        .find(|&id| id != 5 && find_lot_hit_point(&shell, id).is_some())
        .expect("another neighborhood lot");
    let other_pt = find_lot_hit_point(&shell, other_lot).expect("other lot hit point");

    // 1. Static clock_us == 0 baseline render (deterministic) & zero-clock hover guard.
    shell.render();
    let idle_pixels = shell.canvas().color.clone();
    shell.hover(goth_pt);
    assert!(
        !shell.has_active_fades(),
        "clock_us == 0 hover must never start a hidden fade ramp"
    );
    shell.leave();
    assert!(!shell.has_active_fades());

    // 2. Advance presentation clock so live 300 ms cross-fade activates.
    shell.tick_shell(100_000); // now = 100 ms
    assert!(!shell.has_active_fades());

    // Hover Goth lot 5 -> starts 300 ms roof fade-out (100 ms -> 400 ms).
    assert!(shell.hover(goth_pt));
    assert!(shell.has_active_fades());
    assert!(shell.is_actively_interacting());
    assert!((shell.hover_fade_state().fade_for(5, 100) - 0.0).abs() < 1e-6);

    // Mid-fade at +150 ms (now = 250 ms): fade == 0.5, still animating, requests redraw.
    assert!(shell.tick_shell(150_000));
    assert!(shell.has_active_fades());
    assert!((shell.hover_fade_state().fade_for(5, 250) - 0.5).abs() < 1e-6);
    shell.render();
    let mid_pixels = shell.canvas().color.clone();
    assert_ne!(
        idle_pixels, mid_pixels,
        "150 ms mid-fade must visually blend Goth roof cutaway"
    );

    // Complete fade at +150 ms (now = 400 ms): fade == 1.0, lands final frame, then settles.
    assert!(shell.tick_shell(150_000));
    assert!(!shell.has_active_fades());
    assert!((shell.hover_fade_state().fade_for(5, 400) - 1.0).abs() < 1e-6);
    shell.render();
    let full_cutaway_pixels = shell.canvas().color.clone();
    assert_ne!(
        mid_pixels, full_cutaway_pixels,
        "full 300 ms cutaway must differ from 150 ms mid-fade"
    );

    // Simultaneous multi-lot fade: move hover from lot 5 -> other_lot at 400 ms.
    assert!(shell.hover(other_pt));
    assert!(shell.has_active_fades());
    shell.tick_shell(150_000); // now = 550 ms
    assert!((shell.hover_fade_state().fade_for(5, 550) - 0.5).abs() < 1e-6);
    assert!((shell.hover_fade_state().fade_for(other_lot, 550) - 0.5).abs() < 1e-6);

    // Leaving the map fades other_lot back to 0.0 over 300 ms and then settles idle.
    shell.tick_shell(150_000); // now = 700 ms (lot 5 = 0.0, other_lot = 1.0)
    assert!(!shell.has_active_fades());
    assert!(shell.leave());
    assert!(shell.has_active_fades());
    shell.tick_shell(150_000); // now = 850 ms
    assert!((shell.hover_fade_state().fade_for(other_lot, 850) - 0.5).abs() < 1e-6);
    shell.tick_shell(150_000); // now = 1000 ms -> settled!
    assert!(!shell.has_active_fades());
    assert!((shell.hover_fade_state().fade_for(other_lot, 1000) - 0.0).abs() < 1e-6);
}

#[test]
fn temporal_ui_animations_advance_when_clock_positive_and_snap_at_clock_zero() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        install: install.clone(),
        house: goth_house(&install),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("Goth house opens");
    game.resize([800, 600]);
    assert_eq!(game.run(50), 50);
    game.render();

    // --- 1. Static clock_us == 0: every temporal UI animation snaps instantaneously ---
    game.show_notice("Static Notice", "Instant at clock_us == 0");
    game.render();
    assert!(
        !game.ui_animating(),
        "clock_us == 0 must not animate dialogs"
    );
    assert_eq!(game.dialog_fade().alpha(0), 255);
    game.dialog_confirm_key(DialogKey::Enter);
    game.render();

    // Open Needs panel and trigger a Live help balloon at clock_us == 0.
    let needs_btn = game
        .panel()
        .iter()
        .find(|i| i.command == Some(Command::OpenPanel(UiPanel::Needs)))
        .map(|i| i.rect);
    if let Some(r) = needs_btn {
        let pt = [r.x + r.width / 2, r.y + r.height / 2];
        game.press(pt);
        game.release(pt);
        game.render();
    }
    let help_btn = game
        .panel()
        .iter()
        .find(|i| matches!(i.command, Some(Command::Help(_))))
        .map(|i| i.rect)
        .expect("Live help button");
    let help_pt = [
        help_btn.x + help_btn.width / 2,
        help_btn.y + help_btn.height / 2,
    ];
    game.press(help_pt);
    game.release(help_pt);
    game.render();
    assert!(
        !game.ui_animating(),
        "clock_us == 0 must not animate balloons"
    );
    assert_eq!(game.balloon_fade().alpha(0), 255);
    // Dismiss help balloon before starting interactive clock_us > 0 checks.
    game.press(help_pt);
    game.release(help_pt);
    game.render();

    // --- 2. Live clock_us > 0: DialogFadeState, BalloonFadeState, and QueueSlideState animate ---
    game.advance_while(100_000, 0, |_| true); // clock_us = 100 ms

    // DialogFadeState: fades 0 -> 255 over DIALOG_FADE_DURATION_MS.
    game.show_notice("Animated Notice", "Fades in smoothly");
    assert!(game.ui_animating());
    assert_eq!(game.dialog_fade().alpha(100), 0);
    game.advance_while(u64::from(DIALOG_FADE_DURATION_MS / 2) * 1_000, 0, |_| true);
    game.render();
    let mid_dlg_alpha = game.dialog_fade().alpha(100 + DIALOG_FADE_DURATION_MS / 2);
    assert!(
        (1..255).contains(&mid_dlg_alpha),
        "dialog fade at half-duration must be translucent, got {mid_dlg_alpha}"
    );
    game.advance_while(u64::from(DIALOG_FADE_DURATION_MS) * 1_000, 0, |_| true);
    game.render();
    assert!(
        !game
            .dialog_fade()
            .is_animating(100 + DIALOG_FADE_DURATION_MS * 2)
    );
    game.dialog_confirm_key(DialogKey::Enter);
    game.render();

    // BalloonFadeState: 250 ms fade-in & 250 ms fade-out.
    game.press(help_pt);
    game.release(help_pt);
    game.render();
    assert!(
        game.ui_animating(),
        "opening help balloon at clock_us > 0 must start 250 ms fade-in"
    );
    game.advance_while(u64::from(BALLOON_FADE_MS / 2) * 1_000, 0, |_| true);
    game.render();
    assert!(game.ui_animating());
    game.advance_while(u64::from(BALLOON_FADE_MS) * 1_000, 0, |_| true);
    game.render();
    assert!(!game.ui_animating(), "help balloon settles after 250 ms");
    // Dismiss balloon -> fades out over 250 ms.
    game.press(help_pt);
    game.release(help_pt);
    game.render();
    assert!(
        game.ui_animating(),
        "closing help balloon at clock_us > 0 must start 250 ms fade-out"
    );
    game.advance_while(u64::from(BALLOON_FADE_MS + 50) * 1_000, 0, |_| true);
    game.render();
    assert!(
        !game.ui_animating(),
        "help balloon fade-out settles cleanly"
    );

    // QueueSlideState: 500 ms action-queue icon slide-in from pie-menu click origin,
    // and mid-slide icons ignore clicks (`Queue::at_clickable`).
    let (_, hole_id) = place_and_queue_magic_hole(&mut game);
    // Clear existing queue and trigger via real pie-menu click so `set_queue_origin` fires.
    let sim = game.lot().world().selected().expect("selected Sim");
    let uids: Vec<i32> = game.queue().icons.iter().map(|i| i.uid).collect();
    for uid in uids {
        let _ = game.lot_mut().cancel(sim, uid);
    }
    game.run(2);
    game.render();

    // Open pie menu on the Magic Hole via `Menu` or find any clickable object on screen.
    let mut clicked_pie = false;
    'find_obj: for y in (80..440).step_by(12) {
        for x in (100..700).step_by(12) {
            if game.under([x, y]).is_some() {
                game.press([x, y]);
                game.release([x, y]);
                if game.menu().is_some() {
                    game.render();
                    if let Some(menu) = game.menu()
                        && let Some(label_rect) = menu.labels().first().map(|l| l.1)
                    {
                        let choice_pt = [
                            label_rect.x + label_rect.width / 2,
                            label_rect.y + label_rect.height / 2,
                        ];
                        game.press(choice_pt);
                        game.release(choice_pt);
                        game.render();
                        if !game.queue().icons.is_empty() {
                            clicked_pie = true;
                            break 'find_obj;
                        }
                    } else {
                        game.close_menu();
                    }
                }
            }
        }
    }
    let _ = hole_id;
    assert!(
        clicked_pie,
        "must queue a live interaction via pie menu to exercise QueueSlideState"
    );
    assert!(
        game.ui_animating(),
        "newly queued interaction icon must slide in over 500 ms"
    );
    let sliding_icon = game.queue().icons[0].clone();
    // Mid-slide click on the sliding icon is covered by the HUD (so it does
    // not activate the lot behind it) while ignoring cancellation (`0x4cb406`).
    let icon_pt = [
        sliding_icon.rect.x + sliding_icon.rect.width / 2,
        sliding_icon.rect.y + sliding_icon.rect.height / 2,
    ];
    assert!(
        game.over_panel(icon_pt),
        "visible mid-slide queue icon must still intercept pointer hits"
    );
    game.press(icon_pt);
    game.release(icon_pt);
    game.render();
    assert!(
        !game.queue().icons[0].cancelled,
        "mid-slide queue icon must ignore cancellation clicks (0x4cb406)"
    );
    // Advance past SLIDE_DURATION_MS (500 ms) -> icon lands at [32, 32] and settles!
    game.advance_while(u64::from(SLIDE_DURATION_MS + 50) * 1_000, 0, |_| true);
    game.render();
    assert!(!game.ui_animating(), "queue slide settles after 500 ms");
    assert_eq!(
        [game.queue().icons[0].rect.x, game.queue().icons[0].rect.y],
        sims_ui::queue::FIRST,
        "settled first action-queue icon must land at FIRST [32, 32]"
    );
}
