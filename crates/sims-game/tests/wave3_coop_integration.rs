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
    let action = picked.expect("expected at least one offered throwaway pie action on Goth lot");
    peer.coop_submit_command(NetCommand::SimActionPick {
        sim_id: action.caller as u16,
        target_id: action.target as u16,
        menu_index: action.table_index as i16,
        joining: action.join != 0,
        args: action.args,
    })
    .expect("submit SimActionPick");

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

#[test]
fn sol_round1_regressions_build_guards_saves_restage_advance_and_failover() {
    use sims_game::coop::is_coop_manifest_file;
    use sims_ui::build::{BuildSubtoolAction, RoofPitchPreset};
    use sims_ui::command::BuildTool;

    // P2 #11: Manifest allowlist covers Animation/*.far and TemplateNPCs/*.
    assert!(is_coop_manifest_file("gamedata/animation/animation.far"));
    assert!(is_coop_manifest_file("templatenpcs/npcs.iff"));
    assert!(is_coop_manifest_file("downloads/custom.iff"));
    assert!(!is_coop_manifest_file("expansionpack/sound/ambient.far"));

    let Some(install) = install_root() else {
        eprintln!("skipping: SIMS_INSTALL not set");
        return;
    };

    let mut host = open_goth_game(&install);
    let mut peer = open_goth_game(&install);
    let mesh = MemoryMesh::new(777);
    let host_tx = mesh.add_node("mem://r1-host");
    let host_addr = host_tx.local_address().clone();
    let peer_tx = mesh.add_node("mem://r1-peer");
    let manifest = ManifestDigest([0x33; 32]);

    host.coop_start_host_with_transport(
        CoopTransport::memory(host_tx),
        manifest,
        CoopStartOptions {
            player_name: "HostR1".into(),
            session_config: SessionConfig {
                fingerprint_interval: 4,
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
        "PeerR1",
    )
    .expect("peer start");

    for _ in 0..40 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
        if peer.coop_controller().snapshot_restore_count() >= 1
            && coop_player_count(&host) == 2
            && coop_player_count(&peer) == 2
        {
            break;
        }
    }
    assert_eq!(host.lot().coop_world_hash(), peer.lot().coop_world_hash());

    // P1 #1: Selecting Wall, Wallpaper, Hand, and Roof pitch/pattern in co-op must NEVER mutate
    // the shared Lot world out-of-band.
    let hash_before_build = host.lot().coop_world_hash();
    host.set_mode(Mode::Build);
    host.select_build_tool(BuildTool::Wall);
    host.select_build_tool(BuildTool::Wallpaper);
    host.select_build_tool(BuildTool::Hand);
    host.select_build_tool(BuildTool::Roof);
    assert!(!host.apply_build_subtool(BuildSubtoolAction::RoofPitch(RoofPitchPreset::Steep)));
    assert!(!host.apply_build_subtool(BuildSubtoolAction::RoofPattern(0)));
    assert_eq!(
        host.lot().coop_world_hash(),
        hash_before_build,
        "Build tool selections and unsupported Roof subtools must never mutate Lot out-of-band"
    );

    // P2 #6: Off-diagonal Build floor mouse-release commits to exact (x, y), NOT swapped (y, x).
    const FLOORS_L1_ARRAY: u16 = 1;
    let prices = host
        .lot()
        .coop()
        .map(|ctx| ctx.build_prices.clone())
        .unwrap_or_default();
    let test_floor_id = prices
        .floors
        .keys()
        .copied()
        .find(|&pat| pat != 0)
        .unwrap_or(1);
    let off_diag = {
        let world = host.lot().world();
        let active = usize::try_from(world.size).unwrap_or(52) as i32;
        let mut chosen = None;
        'outer: for x in 12..(active - 12) {
            for y in 12..(active - 12) {
                if x == y {
                    continue;
                }
                let plan = sims_vm::plan_floor_edit(
                    world,
                    [x, y],
                    [x, y],
                    1,
                    test_floor_id,
                    sims_vm::FloorDragMode::Lay,
                    &prices,
                    None,
                );
                let cur_floor = world
                    .arrays
                    .get(&FLOORS_L1_ARRAY)
                    .and_then(|a| {
                        let at = (x as usize * 64 + y as usize) * 2;
                        a.data
                            .get(at..at + 2)
                            .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    })
                    .unwrap_or(0);
                if plan.valid && plan.affordable && cur_floor == 0 {
                    chosen = Some([x, y]);
                    break 'outer;
                }
            }
        }
        chosen.expect("expected an open flat off-diagonal grass tile")
    };
    assert_ne!(off_diag[0], off_diag[1]);
    // Pump so Host's Mode::Build commits before floor drag.
    for _ in 0..4 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }
    host.select_build_tool(BuildTool::Floor);
    let lot_shape = sims_render::Lot::from_world(host.lot().world()).expect("lot shape");
    let fine = [off_diag[0] * 16 + 8, off_diag[1] * 16 + 8, 1];
    host.view.camera.center_on([fine[0], fine[1]], [400, 220]);
    let actual = sims_game::sound::object_point_on_lot(&host.view.camera, &lot_shape, fine);
    host.view.camera.origin[0] += 400 - (actual[0] + 1);
    host.view.camera.origin[1] += 220 - actual[1];
    host.render();
    let pt = sims_game::sound::object_point_on_lot(&host.view.camera, &lot_shape, fine);
    let screen_pt = [pt[0] + 1, pt[1]];
    let _ = host.press(screen_pt);
    let _ = host.release(screen_pt);
    for _ in 0..6 {
        pump_memory_pair(&mesh, &mut host, &mut peer, true);
    }
    let floor_at = |g: &Game, tx: i32, ty: i32| -> u16 {
        g.lot()
            .world()
            .arrays
            .get(&FLOORS_L1_ARRAY)
            .and_then(|a| {
                let at = (tx as usize * 64 + ty as usize) * 2;
                a.data
                    .get(at..at + 2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
            })
            .unwrap_or(0)
    };
    let placed_id = floor_at(&host, off_diag[0], off_diag[1]);
    assert_ne!(
        placed_id, 0,
        "mouse-released off-diagonal Build floor drag must paint exact (x, y)={off_diag:?}"
    );
    assert_eq!(
        floor_at(&peer, off_diag[0], off_diag[1]),
        placed_id,
        "peer must receive exact off-diagonal floor tile at (x, y)={off_diag:?}"
    );
    assert_eq!(host.lot().coop_world_hash(), peer.lot().coop_world_hash());

    // Return to Live Mode and drive BOTH Host and Peer via live `advance_while`
    // (normal jitter buffering + interleaved 1-for-1 host scheduling/stepping, P1 #5 + P3 #16).
    host.set_mode(Mode::Live);
    let before_live_ticks = host.lot().ticks();
    for _ in 0..24 {
        let _ = mesh.step_n(4);
        let _ = host.advance_while(45_000, 45_000, |ran| ran < 1);
        let _ = mesh.step_n(4);
        let _ = peer.advance_while(45_000, 45_000, |ran| ran < 4);
    }
    assert!(
        host.lot().ticks() > before_live_ticks + 4,
        "live advance_while must advance host ticks"
    );
    assert!(
        peer.lot().ticks() > before_live_ticks,
        "live advance_while must advance peer ticks under normal jitter buffering"
    );

    // P1 #2 & P1 #3: Active co-op blocks F9 restore (host & peer) AND blocks peer standalone save
    // even AFTER disconnecting from the host!
    let tmp = TempDir::new("save-guards");
    let store = sims_sim::Snapshots::open(&tmp.path().join("standalone.redb")).expect("open store");
    assert!(
        !peer.save_to(&store, "slot1"),
        "active peer must be refused local standalone save"
    );
    assert!(
        !host.restore_from(&store, "slot1"),
        "active host must be refused local standalone restore"
    );
    assert!(
        !peer.restore_from(&store, "slot1"),
        "active peer must be refused local standalone restore"
    );

    // P3 #16: Host departure promotes Peer cleanly to Host and Peer continues advancing ticks!
    host.leave_coop();
    let _ = mesh.step_n(8);
    let ticks_before_failover = peer.lot().ticks();
    for _ in 0..20 {
        let _ = mesh.step_n(4);
        let _ = peer.advance_while(50_000, 50_000, |ran| ran < 2);
        if peer.coop_is_host() && peer.lot().ticks() > ticks_before_failover + 2 {
            break;
        }
    }
    assert!(
        peer.coop_is_host(),
        "surviving peer must promote to Host after host departs"
    );
    assert!(
        peer.lot().ticks() > ticks_before_failover + 2,
        "promoted host must continue advancing ticks via advance_while"
    );

    // After Peer disconnects, remote host world STILL refuses local standalone save!
    peer.leave_coop();
    assert!(
        peer.coop_blocks_local_save(),
        "disconnected guest world must remain save-blocked"
    );
    assert!(
        !peer.save_to(&store, "slot1"),
        "disconnected guest world must never overwrite local standalone save slot"
    );

    // P1 #4 & P2 #10: NeighborhoodShell lot lobby keyboard routing + cross-lot restage!
    // Host boots House02.iff (Roomies) while Guest joins from NeighborhoodShell (which defaults
    // to Goth House05.iff) -> Guest receives host's snapshot -> `pump_coop_restage()` seamlessly
    // stages House02 and converges bit-for-bit!
    let house2 = install.join("UserData").join("Houses").join("House02.iff");
    if house2.is_file() {
        let mut host2 = Game::open(&Options {
            install: install.clone(),
            house: house2,
            answers: Vec::new(),
            play: true,
            frames: Frames::default(),
        })
        .expect("boot House02");
        host2.resize([800, 600]);
        let saves2 = tmp.path().join("shell-restage.redb");
        let mut guest_shell =
            NeighborhoodShell::open(install, saves2, 1, true, vec![3], Frames::default())
                .expect("open guest NeighborhoodShell");
        guest_shell.resize([800, 600]);
        // Enter default Goth lot (House 5) and verify lot lobby receives keyboard input (P2 #10).
        guest_shell.enter_lot(5).expect("enter Goth lot 5");
        assert!(guest_shell.toggle_coop_lobby());
        assert!(guest_shell.coop_lobby_open());
        let _ = guest_shell.key_tab();
        let _ = guest_shell.key_char('a');
        let _ = guest_shell.key_backspace();
        let _ = guest_shell.escape();
        assert!(
            !guest_shell.coop_lobby_open(),
            "Escape in NeighborhoodShell must close the active lot's Co-op Lobby"
        );

        let mesh2 = MemoryMesh::new(888);
        let h2_tx = mesh2.add_node("mem://h2");
        let h2_addr = h2_tx.local_address().clone();
        let g2_tx = mesh2.add_node("mem://g2");
        let m2 = ManifestDigest([0x99; 32]);

        host2
            .coop_start_host_with_transport(
                CoopTransport::memory(h2_tx),
                m2,
                CoopStartOptions {
                    player_name: "HostHouse2".into(),
                    prime_tick_on_start: true,
                    ..CoopStartOptions::default()
                },
            )
            .expect("start House02 host");
        guest_shell
            .game_mut()
            .expect("guest lot")
            .coop_start_join_with_transport(CoopTransport::memory(g2_tx), h2_addr, m2, "Guest")
            .expect("start guest join from House05");

        for _ in 0..40 {
            let _ = mesh2.step_n(4);
            let need_host_tick = guest_shell.game().is_none_or(|g| coop_player_count(g) < 2);
            host2.coop_pump(need_host_tick, true).expect("host2 pump");
            let _ = mesh2.step_n(4);
            if let Some(g) = guest_shell.game_mut() {
                g.coop_pump(false, true).expect("guest pump");
            }
            let _ = guest_shell.pump_coop_restage();
            let _ = mesh2.step_n(4);
            if let Some(g) = guest_shell.game_mut() {
                g.coop_pump(false, true).expect("guest follow-up pump");
            }
            if guest_shell.game().is_some_and(|g| {
                g.coop_controller().snapshot_restore_count() >= 1
                    && coop_player_count(g) == 2
                    && g.lot().ticks() == host2.lot().ticks()
            }) && coop_player_count(&host2) == 2
            {
                break;
            }
        }
        let guest_game = guest_shell.game().expect("guest restaged game");
        assert_eq!(
            guest_game.coop_controller().snapshot_restore_count(),
            1,
            "guest shell must restage House02 and restore verified SCOP v2 snapshot"
        );
        assert_eq!(
            host2.lot().coop_world_hash(),
            guest_game.lot().coop_world_hash(),
            "restaged guest House02 must match host bit-for-bit"
        );
        assert!(
            guest_shell
                .enqueue_background_lot_save(sims_sim::autosave::SaveTarget::Autosave)
                .is_err(),
            "restaged guest lot must refuse background lot autosave"
        );
        guest_shell.leave_coop_if_active();
        host2.leave_coop();
    }
}
