//! Headless end-to-end 2-client integration tests for `sims-game` Wave 3:
//! Co-op Multiplayer Live Shell (`game/coop-multiplayer-shell`).
//!
//! Exercises the exact same `CoopController` and live `Game` instances over BOTH:
//! - Deterministic `sims_net::MemoryMesh` (lockstep commands, throwaway pie menu,
//!   per-player Sim selection, speed/pause/mode, Buy/Build placement, camera
//!   rotation, multi-tick `Lot::coop_world_hash` convergence, HUD overlay &
//!   distinct remote-player plumbob rendering, simulated drift detection &
//!   automatic `"SCOP"` v2 snapshot resync recovery).
//! - Real `sims_net::IrohTransport` QUIC loopback (`IrohConfig::Loopback`):
//!   non-blocking async host bind -> `simscoop1...` `InviteTicket` encode/decode
//!   -> join -> chunked `"SCOP"` v2 snapshot restore -> concurrent commands ->
//!   multi-tick hash convergence -> manifest mismatch non-destructive rejection
//!   -> clean endpoint & runtime shutdown with zero panics or leaked threads.
//! - Preflight & UI guards: tutorial lot (`Global 26 != 0`) refusal, away-from-home
//!   travel visit refusal, Options → Co-op button opening `CoopLobbyDialog`,
//!   `NeighborhoodShell` lobby & travel guards, and single-player Live Mode
//!   bit-exactness (`coop == None`).

// Real iroh QUIC loopback integration tests poll live sockets with wall-clock deadlines.
#![allow(clippy::disallowed_methods)]

use sims_game::coop::{CoopStartOptions, CoopTransport, IrohBridge};
use sims_game::game::{Game, Options, goth_house};
use sims_game::neighborhood::NeighborhoodShell;
use sims_game::view::Move;
use sims_net::{
    BuyBuildEdit, Command as NetCommand, InviteTicket, IrohConfig, ManifestDigest, MemoryMesh,
    PeerAddress, SessionConfig, SimSpeed, Transport,
};
use sims_ui::command::{Mode, Speed};
use sims_ui::coop::{ConnectionState, CoopRole, LobbyTab};
use sims_vm::boot::Frames;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

fn install_root() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    dir.is_dir().then_some(dir)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sims-game-wave3-coop-{tag}-{}-{id}",
            std::process::id()
        ));
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

fn open_goth_game(install: &Path) -> Game {
    let mut game = Game::open(&Options {
        install: install.to_path_buf(),
        house: goth_house(install),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("boot Goth house");
    game.resize([800, 600]);
    game
}

fn household_sim_ids(game: &Game) -> Vec<u16> {
    let world = game.lot().world();
    let mut sims: Vec<u16> = world
        .objects
        .iter()
        .filter(|(_, o)| o.person.is_some())
        .map(|(&id, _)| id)
        .collect();
    sims.sort_unstable();
    sims
}

fn coop_player_count(game: &Game) -> usize {
    game.coop_controller()
        .session()
        .map_or(0, |s| s.players.len())
}

fn find_open_ground_tile(game: &Game) -> [i16; 2] {
    let world = game.lot().world();
    let active = usize::try_from(world.size).unwrap_or(52) as i32;
    for x in 12..(active - 12) {
        for y in 12..(active - 12) {
            let occupied = world.objects.values().any(|o| {
                o.position[2] == 1
                    && o.position[0].div_euclid(16) == x
                    && o.position[1].div_euclid(16) == y
            });
            if !occupied {
                return [x as i16, y as i16];
            }
        }
    }
    [16, 16]
}

fn pump_memory_pair(mesh: &MemoryMesh, host: &mut Game, peer: &mut Game, schedule_host_tick: bool) {
    let _ = mesh.step_n(4);
    host.coop_pump(schedule_host_tick, true)
        .expect("host memory pump");
    let _ = mesh.step_n(4);
    peer.coop_pump(false, true).expect("peer memory pump");
    let _ = mesh.step_n(4);
    host.coop_pump(false, true)
        .expect("host memory follow-up pump");
    let _ = mesh.step_n(4);
    peer.coop_pump(false, true)
        .expect("peer memory follow-up pump");
}

#[test]
fn memory_mesh_two_games_full_lockstep_convergence_and_hud() {
    let Some(install) = install_root() else {
        eprintln!("skipping: SIMS_INSTALL not set");
        return;
    };

    let mut host = open_goth_game(&install);
    let mut peer = open_goth_game(&install);

    // Single-player baseline frame (coop == None) must remain untouched when offline.
    host.render();
    let sp_baseline = host.canvas().color.clone();
    host.render();
    assert_eq!(
        host.canvas().color,
        sp_baseline,
        "single-player Live Mode render must be deterministic with coop == None"
    );

    let sims = household_sim_ids(&host);
    assert!(
        sims.len() >= 2,
        "Goth household should have at least 2 selectable Sims, got {sims:?}"
    );
    let sim_host = sims[0];
    let sim_peer = sims[1];

    // Wire both live Games onto a deterministic MemoryMesh via CoopTransport::memory.
    let mesh = MemoryMesh::new(42);
    let host_tx = mesh.add_node("mem://host");
    let host_addr = host_tx.local_address().clone();
    let peer_tx = mesh.add_node("mem://peer");
    let manifest = ManifestDigest([0x5A; 32]);

    host.coop_start_host_with_transport(
        CoopTransport::memory(host_tx),
        manifest,
        CoopStartOptions {
            player_name: "Alice".into(),
            session_config: SessionConfig {
                fingerprint_interval: 4,
                ..SessionConfig::default()
            },
            prime_tick_on_start: true,
            ..CoopStartOptions::default()
        },
    )
    .expect("host start");
    assert!(host.coop_is_host());
    assert!(
        host.lot().ticks() >= 1,
        "prime tick should advance host to tick 1"
    );

    peer.coop_start_join_with_transport(CoopTransport::memory(peer_tx), host_addr, manifest, "Bob")
        .expect("peer join start");
    assert!(peer.coop_is_peer());

    // Pump until Peer receives and verifies the "SCOP" v2 snapshot and PlayerJoined commits.
    for _ in 0..40 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
        if peer.coop_controller().snapshot_restore_count() >= 1
            && coop_player_count(&host) == 2
            && coop_player_count(&peer) == 2
        {
            break;
        }
    }
    assert_eq!(
        peer.coop_controller().snapshot_restore_count(),
        1,
        "peer must restore verified SCOP v2 snapshot on mid-game join"
    );
    assert_eq!(coop_player_count(&host), 2);
    assert_eq!(coop_player_count(&peer), 2);
    assert_eq!(
        host.lot().coop_world_hash(),
        peer.lot().coop_world_hash(),
        "host and peer world hashes must match immediately after SCOP v2 snapshot sync"
    );

    // 1. Concurrent per-player Sim selection: Host picks sim_host, Peer picks sim_peer.
    host.select(sim_host);
    peer.select(sim_peer);
    for _ in 0..4 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }
    assert_eq!(host.local_selected_sim(), Some(sim_host));
    assert_eq!(peer.local_selected_sim(), Some(sim_peer));
    assert_ne!(host.local_selected_sim(), peer.local_selected_sim());
    assert_eq!(
        host.lot().coop_world_hash(),
        peer.lot().coop_world_hash(),
        "per-player Sim selection must keep shared world hash identical"
    );

    // 2. Throwaway pie menu (zero mutation) + SimActionPick submitted by Peer.
    let hash_before_menu = peer.lot().coop_world_hash();
    let peer_player_id = peer.coop_local_player_id().expect("peer player id");
    let mut picked = None;
    for &obj_id in peer.lot().world().objects.keys() {
        if obj_id == sim_peer {
            continue;
        }
        if let Ok(menu) = peer.lot().coop_pie_menu(peer_player_id, obj_id)
            && let Some(first) = menu.entries.first()
        {
            picked = Some(first.action.clone());
            break;
        }
    }
    assert_eq!(
        peer.lot().coop_world_hash(),
        hash_before_menu,
        "throwaway coop_pie_menu must never mutate the shared Lot world"
    );
    if let Some(action) = picked {
        peer.coop_submit_command(NetCommand::SimActionPick {
            sim_id: action.caller as u16,
            target_id: action.target as u16,
            menu_index: action.table_index as i16,
            joining: action.join != 0,
            args: action.args,
        })
        .expect("submit SimActionPick");
    }

    // 3. Speed, Pause, Mode, CameraRotation, and Buy/Build floor tile placement through lockstep.
    host.set_speed(Speed::Fast);
    peer.apply(Move::Rotate(true));

    // Switch Peer into Build Mode, lay a real priced floor tile on level 1, and verify
    // BOTH machines actually deduct Simoleons and converge identically.
    let tile = find_open_ground_tile(&host);
    let priced_floor = host
        .lot()
        .coop()
        .and_then(|ctx| {
            ctx.build_prices
                .floors
                .iter()
                .find(|&(&pat, &price)| pat != 0 && price > 0)
                .map(|(&pat, &price)| (pat, price))
        })
        .expect("host co-op context should carry priced floor catalog entries");
    let funds_before = sims_vm::available_funds(host.lot().world());
    peer.set_mode(Mode::Build);
    for _ in 0..3 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }
    peer.coop_submit_buy_build(BuyBuildEdit::SetFloorTile {
        x: tile[0],
        y: tile[1],
        level: 1,
        pattern: priced_floor.0,
    })
    .expect("submit BuyBuild floor edit");
    for _ in 0..4 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }
    assert_eq!(
        sims_vm::available_funds(host.lot().world()),
        funds_before - priced_floor.1,
        "BuyBuild SetFloorTile must deduct exact floor price on host"
    );
    assert_eq!(
        sims_vm::available_funds(peer.lot().world()),
        funds_before - priced_floor.1,
        "BuyBuild SetFloorTile must deduct exact floor price on peer"
    );
    peer.set_mode(Mode::Live);
    for _ in 0..3 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }

    // Pause, verify mode switch works while paused, then unpause.
    peer.toggle_pause();
    for _ in 0..4 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }
    assert!(
        host.paused() && peer.paused(),
        "both clients must observe lockstep pause"
    );
    host.set_mode(Mode::Buy);
    for _ in 0..3 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }
    assert_eq!(host.mode(), Mode::Buy);
    host.set_mode(Mode::Live);
    peer.toggle_pause();
    for _ in 0..3 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }
    assert!(!host.paused() && !peer.paused());

    // Advance 20 more lockstep ticks across multiple fingerprint intervals.
    for _ in 0..20 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }
    assert_eq!(
        host.lot().ticks(),
        peer.lot().ticks(),
        "host and peer tick counters must match"
    );
    assert_eq!(
        host.lot().coop_world_hash(),
        peer.lot().coop_world_hash(),
        "host and peer world hashes must converge bit-for-bit after 20+ lockstep ticks"
    );
    assert_eq!(
        host.coop_controller().drift_count(),
        0,
        "normal lockstep run must have zero drift events"
    );

    // 4. Render both frames and verify Co-op HUD overlay & remote-player indicators draw cleanly.
    host.render();
    peer.render();
    assert_ne!(
        host.canvas().color,
        sp_baseline,
        "co-op HUD overlay & remote-player plumbob must render visibly onto frame"
    );
    let host_snap = host.coop_snapshot();
    assert_eq!(host_snap.role, CoopRole::Host);
    assert_eq!(host_snap.players.len(), 2);
    assert!(host_snap.is_overlay_active());
    let peer_snap = peer.coop_snapshot();
    assert_eq!(peer_snap.role, CoopRole::Client);
    assert_eq!(peer_snap.players.len(), 2);

    // 5. Clean disconnect on both clients.
    peer.leave_coop();
    host.leave_coop();
    assert!(!host.coop_active());
    assert!(!peer.coop_active());
    assert!(host.lot().coop().is_none());
    assert!(peer.lot().coop().is_none());
}

#[test]
fn memory_mesh_drift_detection_and_automatic_snapshot_resync_recovery() {
    let Some(install) = install_root() else {
        eprintln!("skipping: SIMS_INSTALL not set");
        return;
    };

    let mut host = open_goth_game(&install);
    let mut peer = open_goth_game(&install);

    let mesh = MemoryMesh::new(99);
    let host_tx = mesh.add_node("mem://drift-host");
    let host_addr = host_tx.local_address().clone();
    let peer_tx = mesh.add_node("mem://drift-peer");
    let manifest = ManifestDigest([0x77; 32]);

    host.coop_start_host_with_transport(
        CoopTransport::memory(host_tx),
        manifest,
        CoopStartOptions {
            player_name: "Host".into(),
            session_config: SessionConfig {
                fingerprint_interval: 3,
                ..SessionConfig::default()
            },
            prime_tick_on_start: true,
            ..CoopStartOptions::default()
        },
    )
    .expect("host start");
    peer.coop_start_join_with_transport(
        CoopTransport::memory(peer_tx),
        host_addr,
        manifest,
        "Peer",
    )
    .expect("peer join start");

    for _ in 0..40 {
        pump_memory_pair(&mesh, &mut host, &mut peer, false);
        if peer.coop_controller().snapshot_restore_count() >= 1 {
            break;
        }
    }
    assert_eq!(peer.coop_controller().snapshot_restore_count(), 1);
    assert_eq!(host.lot().coop_world_hash(), peer.lot().coop_world_hash());

    // Advance a few clean ticks first.
    for _ in 0..4 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }
    assert_eq!(host.lot().coop_world_hash(), peer.lot().coop_world_hash());

    // Inject simulated desync on Peer by stepping Peer's Lot out-of-band.
    let _ = peer.lot_mut().step();
    assert_ne!(
        host.lot().coop_world_hash(),
        peer.lot().coop_world_hash(),
        "out-of-band step must diverge Peer's world hash"
    );

    // Pump lockstep across fingerprint check intervals until Host detects drift
    // and automatically streams a fresh SCOP v2 snapshot to resync Peer.
    for _ in 0..40 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
        if peer.coop_controller().snapshot_restore_count() >= 2
            && host.lot().coop_world_hash() == peer.lot().coop_world_hash()
        {
            break;
        }
    }
    assert!(
        host.coop_controller().drift_count() >= 1,
        "host must record at least one drift event, got {}",
        host.coop_controller().drift_count()
    );
    assert!(
        peer.coop_controller().snapshot_restore_count() >= 2,
        "peer must complete automatic SCOP v2 resync snapshot restore, got {}",
        peer.coop_controller().snapshot_restore_count()
    );
    assert_eq!(
        host.lot().coop_world_hash(),
        peer.lot().coop_world_hash(),
        "host and peer world hashes must re-converge bit-for-bit after automatic drift resync"
    );

    // Verify stability for 6 more ticks after recovery.
    for _ in 0..6 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }
    assert_eq!(host.lot().coop_world_hash(), peer.lot().coop_world_hash());

    peer.leave_coop();
    host.leave_coop();
}

#[test]
fn iroh_quic_loopback_two_games_ticket_snapshot_lockstep_and_shutdown() {
    let Some(install) = install_root() else {
        eprintln!("skipping: SIMS_INSTALL not set");
        return;
    };

    let mut host = open_goth_game(&install);
    let mut peer = open_goth_game(&install);

    let sims = household_sim_ids(&host);
    assert!(sims.len() >= 2);
    let sim_host = sims[0];
    let sim_peer = sims[1];

    // Use real non-blocking async startup over iroh QUIC loopback (exact same path as GUI!).
    host.coop_set_player_name("QuicHost");
    host.coop_set_iroh_config(IrohConfig::Loopback);
    peer.coop_set_player_name("QuicPeer");
    peer.coop_set_iroh_config(IrohConfig::Loopback);

    host.coop_start_host_async(Some(CoopStartOptions {
        player_name: "QuicHost".into(),
        iroh_config: IrohConfig::Loopback,
        session_config: SessionConfig {
            fingerprint_interval: 5,
            ..SessionConfig::default()
        },
        prime_tick_on_start: true,
        ..CoopStartOptions::default()
    }))
    .expect("start async iroh host");

    // Poll until background Tokio runtime binds iroh endpoint + builds manifest + emits simscoop1 ticket.
    let deadline = Instant::now() + Duration::from_secs(30);
    let ticket = loop {
        host.coop_poll();
        if let Some(t) = host.coop_invite_ticket() {
            break t.to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for async iroh host bind & invite ticket"
        );
        std::thread::sleep(Duration::from_millis(15));
    };
    assert!(
        ticket.starts_with("simscoop1"),
        "expected canonical simscoop1 invite ticket, got {ticket}"
    );
    InviteTicket::decode_string(&ticket).expect("invite ticket must decode cleanly");

    // Peer joins asynchronously using the canonical ticket string.
    peer.coop_start_join_async(
        ticket,
        Some(CoopStartOptions {
            player_name: "QuicPeer".into(),
            iroh_config: IrohConfig::Loopback,
            ..CoopStartOptions::default()
        }),
    )
    .expect("start async iroh join");

    // Pump both Games over real QUIC loopback until Peer restores the verified SCOP v2 snapshot
    // and the host's PlayerJoined lockstep batch commits on both machines.
    let sync_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let peer_synced = peer.coop_controller().snapshot_restore_count() >= 1;
        host.coop_pump(peer_synced, true).expect("host iroh pump");
        peer.coop_pump(false, true).expect("peer iroh pump");
        if peer.coop_controller().snapshot_restore_count() >= 1
            && coop_player_count(&host) == 2
            && coop_player_count(&peer) == 2
        {
            break;
        }
        if let ConnectionState::Error(err) = peer.coop_snapshot().connection {
            panic!("peer failed to join over iroh loopback: {err}");
        }
        assert!(
            Instant::now() < sync_deadline,
            "timed out waiting for iroh QUIC snapshot sync (host players={}, peer restores={})",
            coop_player_count(&host),
            peer.coop_controller().snapshot_restore_count()
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        host.lot().coop_world_hash(),
        peer.lot().coop_world_hash(),
        "real iroh QUIC snapshot sync must yield identical world hash"
    );

    // Issue concurrent commands over real QUIC loopback, including an actual priced Buy/Build floor tile edit.
    host.select(sim_host);
    peer.select(sim_peer);
    peer.coop_submit_command(NetCommand::SetSpeed {
        speed: SimSpeed::NORMAL,
    })
    .expect("submit speed over QUIC");
    let tile = find_open_ground_tile(&host);
    let priced_floor = host
        .lot()
        .coop()
        .and_then(|ctx| {
            ctx.build_prices
                .floors
                .iter()
                .find(|&(&pat, &price)| pat != 0 && price > 0)
                .map(|(&pat, &price)| (pat, price))
        })
        .expect("priced floor catalog entry");
    let funds_before_quic = sims_vm::available_funds(host.lot().world());
    peer.set_mode(Mode::Build);
    peer.coop_submit_buy_build(BuyBuildEdit::SetFloorTile {
        x: tile[0],
        y: tile[1],
        level: 1,
        pattern: priced_floor.0,
    })
    .expect("submit BuyBuild over QUIC");
    peer.set_mode(Mode::Live);

    // Advance 12 lockstep ticks over real QUIC and wait for Peer to catch up.
    let target_tick = host.lot().ticks() + 12;
    let lockstep_deadline = Instant::now() + Duration::from_secs(20);
    while host.lot().ticks() < target_tick || peer.lot().ticks() < host.lot().ticks() {
        let should_schedule = host.lot().ticks() < target_tick;
        host.coop_pump(should_schedule, true)
            .expect("host lockstep pump");
        peer.coop_pump(false, true).expect("peer lockstep pump");
        assert!(
            Instant::now() < lockstep_deadline,
            "timed out advancing QUIC lockstep (host={}, peer={}, target={})",
            host.lot().ticks(),
            peer.lot().ticks(),
            target_tick
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(host.lot().ticks(), peer.lot().ticks());
    assert_eq!(host.local_selected_sim(), Some(sim_host));
    assert_eq!(peer.local_selected_sim(), Some(sim_peer));
    assert_eq!(
        sims_vm::available_funds(host.lot().world()),
        funds_before_quic - priced_floor.1,
        "QUIC BuyBuild floor edit must apply on host"
    );
    assert_eq!(
        sims_vm::available_funds(peer.lot().world()),
        funds_before_quic - priced_floor.1,
        "QUIC BuyBuild floor edit must apply identically on peer"
    );
    assert_eq!(
        host.lot().coop_world_hash(),
        peer.lot().coop_world_hash(),
        "real iroh QUIC lockstep must converge bit-for-bit"
    );
    assert_eq!(host.coop_controller().drift_count(), 0);

    // Clean disconnect & iroh endpoint + Tokio runtime shutdown.
    peer.leave_coop();
    host.leave_coop();
    assert!(!peer.coop_active());
    assert!(!host.coop_active());
    assert!(peer.lot().coop().is_none());
    assert!(host.lot().coop().is_none());
}

#[test]
fn iroh_quic_loopback_manifest_mismatch_rejected_non_destructively() {
    let Some(install) = install_root() else {
        eprintln!("skipping: SIMS_INSTALL not set");
        return;
    };

    let mut host = open_goth_game(&install);
    let mut peer = open_goth_game(&install);
    let peer_ticks_before = peer.lot().ticks();
    let peer_hash_before = peer.lot().coop_world_hash();

    let host_bridge = IrohBridge::bind_blocking(IrohConfig::Loopback).expect("bind host iroh");
    let peer_bridge = IrohBridge::bind_blocking(IrohConfig::Loopback).expect("bind peer iroh");
    let host_addr = PeerAddress::new(host_bridge.invite_ticket_string().to_owned());

    host.coop_start_host_with_transport(
        CoopTransport::Iroh(host_bridge),
        ManifestDigest([0xAA; 32]),
        CoopStartOptions {
            player_name: "HostA".into(),
            ..CoopStartOptions::default()
        },
    )
    .expect("start host");

    peer.coop_start_join_with_transport(
        CoopTransport::Iroh(peer_bridge),
        host_addr,
        ManifestDigest([0xBB; 32]),
        "PeerB",
    )
    .expect("start mismatched peer");

    let deadline = Instant::now() + Duration::from_secs(15);
    let (local_summary, remote_summary, mismatched_files) = loop {
        host.coop_poll();
        peer.coop_poll();
        if let ConnectionState::ManifestMismatch {
            local_summary,
            remote_summary,
            mismatched_files,
        } = peer.coop_snapshot().connection
        {
            break (local_summary, remote_summary, mismatched_files);
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for manifest mismatch rejection (state={:?})",
            peer.coop_snapshot().connection
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    assert_ne!(
        local_summary, remote_summary,
        "manifest mismatch card must surface distinct local vs host digests"
    );
    assert!(
        !mismatched_files.is_empty(),
        "manifest mismatch card must explain mismatched game/Downloads files"
    );
    assert!(
        !peer.coop_active() && peer.lot().coop().is_none(),
        "rejected peer must be cleanly detached"
    );
    assert_eq!(peer.lot().ticks(), peer_ticks_before);
    assert_eq!(
        peer.lot().coop_world_hash(),
        peer_hash_before,
        "manifest mismatch rejection must be 100% non-destructive to peer's lot"
    );
    assert!(
        host.coop_is_host(),
        "host must remain healthy after rejecting mismatched peer"
    );

    peer.leave_coop();
    host.leave_coop();
}

#[test]
fn coop_preflight_refuses_tutorial_and_travel_and_options_button_opens_lobby() {
    let Some(install) = install_root() else {
        eprintln!("skipping: SIMS_INSTALL not set");
        return;
    };

    // 1. Options → Co-op button visibility (only in Mode::Options, never in Live Mode) & click.
    let mut game = open_goth_game(&install);
    game.render();
    let live_baseline = game.canvas().color.clone();
    assert!(
        game.options_coop_button_rect().is_none(),
        "Co-op button must not exist in Live Mode"
    );

    game.set_mode(Mode::Options);
    game.render();
    let coop_rect = game
        .options_coop_button_rect()
        .expect("Options → Co-op button rect must exist in Mode::Options");
    let click_pt = [
        coop_rect.x + coop_rect.width / 2,
        coop_rect.y + coop_rect.height / 2,
    ];
    let _ = game.press(click_pt);
    let out = game.release(click_pt);
    assert!(
        out.open_coop_lobby,
        "clicking Options → Co-op must set open_coop_lobby"
    );
    assert!(
        game.coop_lobby_open(),
        "Options → Co-op click must open CoopLobbyDialog"
    );

    // Close lobby & return to Live Mode -> frame must match single-player baseline bit-for-bit!
    let _ = game.coop_lobby_escape();
    assert!(!game.coop_lobby_open());
    game.set_mode(Mode::Live);
    game.render();
    assert_eq!(
        game.canvas().color,
        live_baseline,
        "returning to single-player Live Mode after closing Co-op lobby must be bit-for-bit identical"
    );

    // 2. Travel visit guard refuses co-op cleanly with explicit message.
    game.coop_set_travel_blocked(true);
    let travel_err = game
        .coop_start_host_async(None)
        .expect_err("away-from-home travel visit must refuse co-op");
    assert!(
        travel_err.to_ascii_lowercase().contains("home lot")
            || travel_err.to_ascii_lowercase().contains("travel"),
        "expected explicit travel/home-lot error, got: {travel_err}"
    );
    game.coop_set_travel_blocked(false);

    // 3. Tutorial lot (House01.iff, Global 26 != 0) refuses co-op cleanly.
    let tutorial_house = install.join("UserData").join("Houses").join("House01.iff");
    if tutorial_house.is_file() {
        let mut tutorial_game = Game::open(&Options {
            install: install.clone(),
            house: tutorial_house,
            answers: Vec::new(),
            play: false,
            frames: Frames::default(),
        })
        .expect("boot tutorial house");
        let g26 = tutorial_game
            .lot()
            .world()
            .globals
            .get(26)
            .copied()
            .unwrap_or(0);
        if g26 != 0 {
            let tut_err = tutorial_game
                .coop_start_host_async(None)
                .expect_err("tutorial lot (Global 26 != 0) must refuse co-op");
            assert!(
                tut_err.to_ascii_lowercase().contains("tutorial"),
                "expected explicit tutorial error, got: {tut_err}"
            );
            assert!(!tutorial_game.coop_active());
        }
    }

    // 4. NeighborhoodShell lobby toggle + invalid ticket refusal.
    let tmp = TempDir::new("shell");
    let saves = tmp.path().join("saves.redb");
    let mut shell = NeighborhoodShell::open(install, saves, 1, false, vec![3], Frames::default())
        .expect("open NeighborhoodShell");
    shell.resize([800, 600]);
    assert!(!shell.coop_lobby_open());
    assert!(shell.toggle_coop_lobby());
    assert!(shell.coop_lobby_open());
    shell.render();
    let bad_ticket_err = shell
        .coop_join_from_shell("not-a-valid-ticket".into())
        .expect_err("invalid invite ticket must fail cleanly");
    assert!(
        bad_ticket_err.to_ascii_lowercase().contains("invalid"),
        "expected invalid ticket message, got: {bad_ticket_err}"
    );
    shell.open_coop_lobby(Some(LobbyTab::Host));
    assert!(shell.coop_lobby_open());
    assert!(shell.toggle_coop_lobby());
    assert!(!shell.coop_lobby_open());
}
