use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_ui::art::Art;
use sims_ui::coop::{
    ConnectionState, CoopCommand, CoopHudOverlay, CoopLobbyDialog, CoopRole, CoopSnapshot,
    LobbyTab, PauseReason, PlayerId, PlayerInfo, PlayerMode, ResyncProgress, TimeAction,
    TimeAttribution, load_art,
};
use sims_ui::dialog::CARET_BLINK_US;
use sims_ui::frame::{Frame, Rect};
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    path.is_dir().then_some(path)
}

fn center(r: Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

fn intersects(a: Rect, b: Rect) -> bool {
    let ax1 = i64::from(a.x) + i64::from(a.width);
    let ay1 = i64::from(a.y) + i64::from(a.height);
    let bx1 = i64::from(b.x) + i64::from(b.width);
    let by1 = i64::from(b.y) + i64::from(b.height);
    i64::from(a.x) < bx1 && ax1 > i64::from(b.x) && i64::from(a.y) < by1 && ay1 > i64::from(b.y)
}

fn sample_players() -> Vec<PlayerInfo> {
    vec![
        PlayerInfo {
            id: PlayerId(1),
            name: "Alice".into(),
            is_host: true,
            is_local: true,
            selected_sim_name: Some("Bella".into()),
            mode: PlayerMode::Live,
            badge_color: [64, 224, 208],
            ping_ms: Some(0),
        },
        PlayerInfo {
            id: PlayerId(2),
            name: "Bob".into(),
            is_host: false,
            is_local: false,
            selected_sim_name: Some("Mortimer".into()),
            mode: PlayerMode::Buy,
            badge_color: [255, 168, 64],
            ping_ms: Some(38),
        },
    ]
}

#[test]
fn host_tab_start_copy_time_lock_and_stop_commands() {
    let mut dialog = CoopLobbyDialog::new();
    dialog.open(LobbyTab::Host);
    assert!(dialog.is_open());

    let size = [800, 600];
    let disconnected = CoopSnapshot::default();
    let layout = dialog.layout(size, &disconnected);
    let (px, py) = center(layout.primary_btn);
    assert!(dialog.pointer_down(size, &disconnected, px, py));
    assert_eq!(
        dialog.pointer_up(size, &disconnected, px, py),
        Some(CoopCommand::StartHosting)
    );

    // Transition to Hosting state: ticket box, copy button, time lock toggle, and player list appear.
    let hosting = CoopSnapshot {
        role: CoopRole::Host,
        connection: ConnectionState::Hosting {
            invite_ticket: "iroh-ticket-abc123xyz".into(),
        },
        players: sample_players(),
        host_only_time_control: false,
        ..CoopSnapshot::default()
    };
    let hlayout = dialog.layout(size, &hosting);
    let copy = hlayout.copy_btn.expect("copy button");
    let (cx, cy) = center(copy);
    assert!(dialog.pointer_down(size, &hosting, cx, cy));
    assert_eq!(
        dialog.pointer_up(size, &hosting, cx, cy),
        Some(CoopCommand::CopyInviteTicket(
            "iroh-ticket-abc123xyz".into()
        ))
    );
    assert!(dialog.copied_feedback());

    let toggle = hlayout.time_lock_toggle.expect("time lock toggle");
    let (tx, ty) = center(toggle);
    assert!(dialog.pointer_down(size, &hosting, tx, ty));
    assert_eq!(
        dialog.pointer_up(size, &hosting, tx, ty),
        Some(CoopCommand::SetHostOnlyTimeControl(true))
    );

    let (sx, sy) = center(hlayout.primary_btn);
    assert!(dialog.pointer_down(size, &hosting, sx, sy));
    assert_eq!(
        dialog.pointer_up(size, &hosting, sx, sy),
        Some(CoopCommand::Disconnect)
    );
}

#[test]
fn join_tab_paste_type_connect_cancel_and_error_states() {
    let mut dialog = CoopLobbyDialog::new();
    dialog.open(LobbyTab::Join);
    assert!(dialog.is_input_focused());

    let size = [800, 600];
    let snapshot = CoopSnapshot::default();

    // Empty ticket cannot submit or connect.
    assert_eq!(dialog.submit(&snapshot), None);
    let layout = dialog.layout(size, &snapshot);
    let (px, py) = center(layout.primary_btn);
    dialog.pointer_down(size, &snapshot, px, py);
    assert_eq!(dialog.pointer_up(size, &snapshot, px, py), None);

    // Paste ticket with surrounding whitespace/newlines (stripped cleanly).
    assert!(dialog.paste_text("  node123\n456  "));
    assert_eq!(dialog.ticket_input(), "node123456");
    assert!(dialog.insert_char('Z'));
    assert_eq!(dialog.ticket_input(), "node123456Z");
    assert!(dialog.backspace());
    assert_eq!(dialog.ticket_input(), "node123456");

    // Caret blinks on 500 ms interval.
    assert!(!dialog.sync_blink(0));
    assert!(dialog.sync_blink(CARET_BLINK_US));

    // Clicking Connect emits JoinSession.
    dialog.pointer_down(size, &snapshot, px, py);
    assert_eq!(
        dialog.pointer_up(size, &snapshot, px, py),
        Some(CoopCommand::JoinSession {
            ticket: "node123456".into()
        })
    );

    // While Connecting, Cancel emits CancelConnect instead of closing.
    let connecting = CoopSnapshot {
        connection: ConnectionState::Connecting,
        ..CoopSnapshot::default()
    };
    let sec = dialog.layout(size, &connecting).secondary_btn.unwrap();
    let (cx, cy) = center(sec);
    dialog.pointer_down(size, &connecting, cx, cy);
    assert_eq!(
        dialog.pointer_up(size, &connecting, cx, cy),
        Some(CoopCommand::CancelConnect)
    );
    assert!(dialog.is_open());

    // Rendering manifest mismatch and connection error states never panics.
    let art = Art::default();
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let mismatch = CoopSnapshot {
        connection: ConnectionState::ManifestMismatch {
            local_summary: "1420 files".into(),
            remote_summary: "1421 files".into(),
            mismatched_files: vec!["Downloads/custom.iff".into()],
        },
        ..CoopSnapshot::default()
    };
    dialog.draw(&mut frame, &mismatch, &art);

    let err = CoopSnapshot {
        connection: ConnectionState::Error("peer unreachable".into()),
        ..CoopSnapshot::default()
    };
    dialog.draw(&mut frame, &err, &art);
}

#[test]
fn overlay_layout_is_strictly_non_overlapping_across_viewports() {
    let art = Art::default();
    let snapshot = CoopSnapshot {
        role: CoopRole::Client,
        connection: ConnectionState::Connected,
        players: sample_players(),
        host_only_time_control: true,
        pause_reasons: vec![
            PauseReason::Explicit {
                player_name: "Alice".into(),
            },
            PauseReason::BuyMode {
                player_name: "Bob".into(),
            },
            PauseReason::BlockingDialog,
        ],
        time_toast: Some(TimeAttribution {
            player_name: "Alice".into(),
            action: TimeAction::UltraSpeed,
            remaining_ms: 2500,
        }),
        resync: Some(ResyncProgress {
            bytes_received: 64_000,
            bytes_total: 128_000,
            reason: "Fingerprint mismatch at tick 1420".into(),
        }),
    };

    assert_eq!(snapshot.pause_reasons[0].label(), "Paused by Alice");
    assert_eq!(snapshot.pause_reasons[1].label(), "Bob is in Buy Mode");
    assert_eq!(snapshot.pause_reasons[2].label(), "Waiting on dialog");
    assert_eq!(
        snapshot.time_toast.as_ref().unwrap().label(),
        "Alice set Ultra Speed"
    );
    assert_eq!(snapshot.resync.as_ref().unwrap().percent(), 50);

    for size in [
        [320, 240],
        [640, 480],
        [800, 600],
        [1280, 720],
        [1920, 1080],
    ] {
        let layout = CoopHudOverlay::layout(size, &snapshot, &art);
        let sp = layout.status_panel.expect("status panel");
        let toast = layout.time_toast.expect("time toast");
        let resync = layout.resync_banner.expect("resync banner");

        assert!(
            !intersects(sp, toast),
            "status panel {sp:?} overlapped toast {toast:?} at {size:?}"
        );
        assert!(
            !intersects(sp, resync),
            "status panel {sp:?} overlapped resync {resync:?} at {size:?}"
        );
        assert!(
            !intersects(toast, resync),
            "toast {toast:?} overlapped resync {resync:?} at {size:?}"
        );

        let mut frame = Frame::new(size[0], size[1], [0, 0, 0]);
        CoopHudOverlay::draw(&mut frame, &snapshot, &art);
        assert!(
            frame.rgb().iter().any(|&px| px != [0, 0, 0]),
            "overlay drew nothing at {size:?}"
        );
    }
}

#[test]
fn complete_collection_live_install_renders_lobby_and_overlay_with_native_chrome() {
    let Some(root) = install() else {
        return;
    };
    let install = Install::detect(&root).expect("detect install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = load_art(&ui, 1).expect("load coop UI art");
    assert!(
        art.missing.is_empty(),
        "missing native 1.91 coop chrome resources: {:?}",
        art.missing
    );

    let snapshot = CoopSnapshot {
        role: CoopRole::Host,
        connection: ConnectionState::Hosting {
            invite_ticket: "iroh-complete-collection-test-ticket-0123456789".into(),
        },
        players: sample_players(),
        host_only_time_control: true,
        pause_reasons: vec![PauseReason::BuyMode {
            player_name: "Bob".into(),
        }],
        time_toast: Some(TimeAttribution {
            player_name: "Alice".into(),
            action: TimeAction::FastSpeed,
            remaining_ms: 1800,
        }),
        resync: Some(ResyncProgress {
            bytes_received: 75,
            bytes_total: 100,
            reason: "Catching up to host snapshot".into(),
        }),
    };

    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    CoopHudOverlay::draw(&mut frame, &snapshot, &art);
    let mut dialog = CoopLobbyDialog::new();
    dialog.open(LobbyTab::Host);
    dialog.draw(&mut frame, &snapshot, &art);

    let non_black = frame.rgb().iter().filter(|&&px| px != [0, 0, 0]).count();
    assert!(
        non_black > 20_000,
        "expected rich 1.91 9-slice chrome rendering, got {non_black} non-black pixels"
    );
}
