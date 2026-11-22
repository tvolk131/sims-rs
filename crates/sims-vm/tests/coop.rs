//! Multi-instance deterministic co-op VM integration tests (`crates/sims-vm/src/coop.rs`).
//!
//! Exercises `CoopVmContext`, `Lot::apply_coop_batch`, `Lot::coop_pie_menu`,
//! `Lot::coop_state_fingerprint`, and `Lot::coop_snapshot_bytes` /
//! `Lot::restore_coop_snapshot` with `sims_net::MemoryMesh` and
//! `sims_net::LockstepNode` — both on synthetic worlds (always run) and on the
//! real Complete Collection Goth House (`UserData/Houses/House05.iff`, when
//! `SIMS_INSTALL` is set).

use sims_content::Catalog;
use sims_formats::behavior::{Behavior, Instruction};
use sims_formats::saved_objects::{Interaction, Person};
use sims_net::manifest::sha256_bytes;
use sims_net::{
    CameraRotation, Command, CommandIgnoreReason, CommandRejectReason, DialogAnswer, DialogId,
    FileManifest, FileManifestEntry, HostStepResult, LockstepNode, ManifestDigest, MemoryMesh,
    MemoryNodeTransport, NodeEvent, PeerAddress, PlayerId, PlayerMode, PlayerRole, SessionConfig,
    SessionState, SimSpeed, SnapshotCompression, StateFingerprint, Tick, Transport,
};
use sims_sim::{Object, World};
use sims_vm::boot::{Frames, Lot};
use sims_vm::{
    CoopBatchReport, CoopCommandStatus, CoopVmContext, Interpreter, PanelMode, Program, Stream,
    menu_entry_to_pick_command,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn goth_house() -> Option<(PathBuf, PathBuf)> {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping real-install co-op test");
        return None;
    };
    let root = PathBuf::from(root);
    let house = root.join("UserData/Houses/House05.iff");
    Some((root, house))
}

fn sample_manifest() -> ManifestDigest {
    let manifest = FileManifest::from_entries([FileManifestEntry {
        relative_path: "GameData/Objects/Objects.far".to_string(),
        size_bytes: 1234,
        sha256: sha256_bytes(b"complete-collection-coop-v1"),
    }])
    .expect("valid manifest");
    manifest.digest()
}

struct CoopPeer {
    node: LockstepNode,
    transport: MemoryNodeTransport,
    lot: Lot,
    step_us: u32,
    drift_events: Vec<(PlayerId, Tick, StateFingerprint, StateFingerprint)>,
    last_opened_dialog: Option<DialogId>,
}

impl CoopPeer {
    fn connect_and_hello(&mut self, host_addr: &PeerAddress) {
        self.transport.connect(host_addr).expect("connect to host");
        self.node
            .send_hello(&mut self.transport)
            .expect("send Hello");
    }

    fn submit(&mut self, command: Command) {
        self.node
            .submit_command(&mut self.transport, command)
            .expect("submit command");
    }

    fn poll_events(&mut self) -> Vec<NodeEvent> {
        self.node.poll(&mut self.transport);
        let events = self.node.drain_events();
        for ev in &events {
            if let NodeEvent::DriftDetected {
                player_id,
                tick,
                expected,
                actual,
            } = ev
            {
                self.drift_events
                    .push((*player_id, *tick, *expected, *actual));
            }
        }
        events
    }

    fn step_ready(&mut self, flush: bool) -> Vec<CoopBatchReport> {
        let mut reports = Vec::new();
        while let Some(applied) = self.node.step_next_tick(flush).expect("step next tick") {
            let report = self
                .lot
                .apply_coop_batch(&applied, self.node.session_mut())
                .expect("apply coop batch");
            if let Some((dialog_id, _)) = report.opened_dialog {
                self.last_opened_dialog = Some(dialog_id);
            }
            self.node
                .report_local_tick(
                    &mut self.transport,
                    applied.tick,
                    self.step_us,
                    Some(report.world_hash),
                )
                .expect("report local tick");
            assert_eq!(
                report.state_fingerprint,
                self.lot.coop_state_fingerprint(self.node.session()),
                "reported fingerprint must match SessionState::combined_fingerprint"
            );
            reports.push(report);
        }
        reports
    }
}

fn pump_mesh(mesh: &MemoryMesh, peers: &mut [&mut CoopPeer], rounds: usize) {
    for _ in 0..rounds {
        mesh.step();
        for peer in peers.iter_mut() {
            peer.poll_events();
        }
    }
}

/// Loads a fresh Goth House lot with default frames and no pre-supplied dialog
/// answers (co-op dialogs are answered interactively via `Command`).
fn load_goth_lot(root: &Path, house: &Path) -> Lot {
    Lot::load(root, house, &[], Frames::default()).expect("load Goth house")
}

/// Runs one lockstep tick on host + all ready peers and returns each peer's
/// `CoopBatchReport` for that tick (in order `[host, peers...]`).
fn step_lockstep_tick(
    mesh: &MemoryMesh,
    host: &mut CoopPeer,
    peers: &mut [&mut CoopPeer],
    flush: bool,
) -> Vec<CoopBatchReport> {
    match host
        .node
        .advance_host_tick(&mut host.transport)
        .expect("advance host tick")
    {
        HostStepResult::Scheduled(_) => {}
        HostStepResult::WaitingForSlowestPeer {
            slowest_peer,
            slowest_tick,
        } => {
            panic!("unexpected pacing stall waiting for {slowest_peer} at {slowest_tick}");
        }
    }
    let mut host_reports = host.step_ready(true);
    assert_eq!(host_reports.len(), 1, "host executes 1 tick per schedule");
    let host_report = host_reports.remove(0);

    for _ in 0..4 {
        mesh.step();
        host.poll_events();
        for peer in peers.iter_mut() {
            peer.poll_events();
        }
    }

    let mut out = vec![host_report];
    for peer in peers.iter_mut() {
        let mut peer_reports = peer.step_ready(flush);
        assert_eq!(
            peer_reports.len(),
            1,
            "peer {:?} expected 1 batch",
            peer.node.local_player_id()
        );
        out.push(peer_reports.remove(0));
    }

    for _ in 0..2 {
        mesh.step();
        host.poll_events();
        for peer in peers.iter_mut() {
            peer.poll_events();
        }
    }
    out
}

fn find_goth_residents(lot: &Lot) -> (u16, u16, u16) {
    let people = &lot.world().people;
    assert!(
        people.len() >= 3,
        "Goth house should have at least 3 household Sims, found {}",
        people.len()
    );
    (people[0], people[1], people[2])
}

#[test]
fn coop_two_players_goth_house_lockstep_convergence_and_throwaway_pie_menu() {
    let Some((root, house)) = goth_house() else {
        return;
    };

    let mesh = MemoryMesh::new(101);
    let manifest = sample_manifest();
    let config = SessionConfig {
        tick_delay: 1,
        min_tick_delay: 1,
        max_tick_delay: 8,
        fingerprint_interval: 10,
        max_players: 4,
        ..SessionConfig::default()
    };

    let mut host_lot = load_goth_lot(&root, &house);
    let bob_lot = load_goth_lot(&root, &house);
    let (mortimer, bella, cassandra) = find_goth_residents(&host_lot);

    let mut host_node = LockstepNode::new_host(
        "Alice",
        Some("mem:host".to_string()),
        manifest,
        config.clone(),
    );
    host_lot.sync_initial_coop_session(host_node.session_mut());
    host_lot
        .attach_coop(host_node.session().clone())
        .expect("attach host coop");

    let host_addr = PeerAddress::new("mem:host");
    let bob_node = LockstepNode::new_peer(
        "Bob",
        Some("mem:bob".to_string()),
        manifest,
        host_addr.clone(),
    );
    let mut host = CoopPeer {
        node: host_node,
        transport: mesh.add_node("mem:host"),
        lot: host_lot,
        step_us: 1_500,
        drift_events: Vec::new(),
        last_opened_dialog: None,
    };
    let mut bob = CoopPeer {
        node: bob_node,
        transport: mesh.add_node("mem:bob"),
        lot: bob_lot,
        step_us: 1_800,
        drift_events: Vec::new(),
        last_opened_dialog: None,
    };

    bob.connect_and_hello(&host_addr);
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 6);
    assert_eq!(bob.node.local_player_id(), Some(PlayerId(2)));
    bob.lot
        .attach_coop(bob.node.session().clone())
        .expect("attach bob coop");

    // Tick 1: PlayerJoined(Bob) executes cleanly; both peers run tick 1.
    let r1 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert!(r1[0].stepped && r1[1].stepped);
    assert_eq!(r1[0].state_fingerprint, r1[1].state_fingerprint);
    assert_eq!(host.lot.world().hash(), bob.lot.world().hash());

    // Tick 2: Goth house Help System modal dialog opens on BOTH peers!
    let r2 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert!(!r2[0].stepped && !r2[1].stepped);
    let &(host_dlg, _) = r2[0]
        .opened_dialog
        .as_ref()
        .expect("host opens blocking dialog");
    let &(bob_dlg, _) = r2[1]
        .opened_dialog
        .as_ref()
        .expect("bob opens blocking dialog");
    assert_eq!(host_dlg, bob_dlg);
    assert!(host.node.session().is_paused());
    assert!(bob.node.session().is_paused());
    assert_eq!(r2[0].state_fingerprint, r2[1].state_fingerprint);

    // First-answer-wins race: Alice submits status 3 first, Bob submits status 2 second.
    host.submit(Command::AnswerBlockingDialog {
        dialog_id: host_dlg,
        answer: DialogAnswer {
            status: 3,
            bitmap_id: None,
            text: None,
        },
    });
    bob.submit(Command::AnswerBlockingDialog {
        dialog_id: bob_dlg,
        answer: DialogAnswer {
            status: 2,
            bitmap_id: None,
            text: None,
        },
    });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);

    let r3 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert!(r3[0].stepped && r3[1].stepped);
    assert_eq!(r3[0].state_fingerprint, r3[1].state_fingerprint);
    assert_eq!(host.lot.world().hash(), bob.lot.world().hash());
    assert!(r3[0].command_results.iter().any(|(_, st)| matches!(
        st,
        CoopCommandStatus::DialogAnswered {
            winner: PlayerId(1),
            ..
        }
    )));
    assert!(r3[0].command_results.iter().any(|(_, st)| matches!(
        st,
        CoopCommandStatus::IgnoredBySession(CommandIgnoreReason::DialogAlreadyAnswered)
    )));

    // Alice selects Mortimer + EAST (1); Bob selects Bella + WEST (3).
    host.submit(Command::SelectPerson {
        sim_id: Some(mortimer),
    });
    host.submit(Command::SetCameraRotation {
        rotation: CameraRotation::EAST,
    });
    bob.submit(Command::SelectPerson {
        sim_id: Some(bella),
    });
    bob.submit(Command::SetCameraRotation {
        rotation: CameraRotation::WEST,
    });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);

    let r4 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert_eq!(r4[0].state_fingerprint, r4[1].state_fingerprint);

    // Verify per-player Global 3 and per-player camera_rotation resolution on both peers.
    for peer in [&host, &bob] {
        let ctx = peer.lot.coop().expect("coop attached");
        assert_eq!(
            ctx.resolve_global_3(peer.lot.world(), mortimer, 0),
            mortimer as i16
        );
        assert_eq!(
            ctx.resolve_global_3(peer.lot.world(), bella, 0),
            bella as i16
        );
        assert_eq!(
            ctx.resolve_global_3(peer.lot.world(), cassandra, 0),
            mortimer as i16,
            "unselected Sim falls back to host selection"
        );
        assert_eq!(
            ctx.resolve_camera_rotation(peer.lot.world(), mortimer, None),
            1
        );
        assert_eq!(
            ctx.resolve_camera_rotation(peer.lot.world(), bella, None),
            3
        );
        assert_eq!(
            ctx.resolve_camera_rotation(peer.lot.world(), cassandra, None),
            1,
            "unselected Sim falls back to host camera rotation"
        );
    }

    // Bob performs multiple throwaway pie-menu checks ONLY on his local machine
    // across several lot objects, verifying zero mutation and zero drift.
    let bob_hash_before = bob.lot.coop_world_hash();
    let bob_raw_before = bob.lot.world().hash();
    let mut picked_cmd = None;
    let candidate_objects: Vec<u16> = bob
        .lot
        .world()
        .order
        .iter()
        .copied()
        .filter(|&id| {
            bob.lot
                .world()
                .objects
                .get(&id)
                .is_some_and(|o| o.person.is_none())
        })
        .take(25)
        .collect();
    for obj_id in candidate_objects {
        if let Ok(menu) = bob.lot.coop_pie_menu(PlayerId(2), obj_id)
            && let Some(entry) = menu.entries.first()
            && picked_cmd.is_none()
        {
            picked_cmd = Some(menu_entry_to_pick_command(entry));
        }
    }
    assert_eq!(
        bob.lot.coop_world_hash(),
        bob_hash_before,
        "throwaway pie-menu checks must never mutate shared co-op world hash"
    );
    assert_eq!(
        bob.lot.world().hash(),
        bob_raw_before,
        "throwaway pie-menu checks must never mutate shared World RNG or objects"
    );

    // Submit the picked interaction from Bob over MemoryMesh!
    let pick_cmd = picked_cmd.expect("Goth house should offer at least one pie-menu interaction");
    let picked_target = match &pick_cmd {
        Command::SimActionPick { target_id, .. } => *target_id,
        _ => unreachable!(),
    };
    bob.submit(pick_cmd);
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);

    let r5 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert_eq!(r5[0].state_fingerprint, r5[1].state_fingerprint);
    assert!(
        r5[0]
            .command_results
            .iter()
            .any(|(_, st)| matches!(st, CoopCommandStatus::QueuedInteraction { .. })),
        "picked interaction should queue cleanly"
    );
    assert_eq!(
        host.lot
            .coop()
            .unwrap()
            .activity_owners
            .get(&picked_target)
            .copied(),
        Some(PlayerId(2)),
        "activity owner should track Bob as initiator"
    );
    assert_eq!(
        host.lot
            .coop()
            .unwrap()
            .resolve_global_3(host.lot.world(), picked_target, 0),
        bella as i16,
        "non-Sim activity object started by Bob should resolve Global 3 to Bella"
    );

    // Advance 115 more ticks (120 lockstep ticks total) verifying bit-for-bit
    // fingerprint parity on every tick and zero drift notifications.
    for tick in 6..=120 {
        let reports = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
        assert_eq!(
            reports[0].state_fingerprint, reports[1].state_fingerprint,
            "state fingerprint mismatch at lockstep tick {tick}"
        );
        assert_eq!(
            host.lot.world().hash(),
            bob.lot.world().hash(),
            "World::hash mismatch at lockstep tick {tick}"
        );
    }

    assert!(host.drift_events.is_empty(), "host saw unexpected drift");
    assert!(bob.drift_events.is_empty(), "bob saw unexpected drift");
}

#[test]
fn coop_three_players_mid_game_join_redb_snapshot_convergence() {
    let Some((root, house)) = goth_house() else {
        return;
    };

    let mesh = MemoryMesh::new(202);
    let manifest = sample_manifest();
    let config = SessionConfig {
        tick_delay: 1,
        min_tick_delay: 1,
        max_tick_delay: 8,
        fingerprint_interval: 10,
        max_players: 4,
        ..SessionConfig::default()
    };

    let mut host_lot = load_goth_lot(&root, &house);
    let bob_lot = load_goth_lot(&root, &house);
    let (mortimer, bella, cassandra) = find_goth_residents(&host_lot);

    let mut host_node =
        LockstepNode::new_host("Alice", Some("mem:host".to_string()), manifest, config);
    host_lot.sync_initial_coop_session(host_node.session_mut());
    host_lot
        .attach_coop(host_node.session().clone())
        .expect("attach host coop");

    let host_addr = PeerAddress::new("mem:host");
    let mut host = CoopPeer {
        node: host_node,
        transport: mesh.add_node("mem:host"),
        lot: host_lot,
        step_us: 1_500,
        drift_events: Vec::new(),
        last_opened_dialog: None,
    };
    let mut bob = CoopPeer {
        node: LockstepNode::new_peer(
            "Bob",
            Some("mem:bob".to_string()),
            manifest,
            host_addr.clone(),
        ),
        transport: mesh.add_node("mem:bob"),
        lot: bob_lot,
        step_us: 1_500,
        drift_events: Vec::new(),
        last_opened_dialog: None,
    };

    bob.connect_and_hello(&host_addr);
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 6);
    bob.lot
        .attach_coop(bob.node.session().clone())
        .expect("attach bob coop");

    // Tick 1 + Tick 2 (blocking dialog) + answer + advance to tick 40.
    step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    let r2 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    let dlg = r2[0].opened_dialog.as_ref().expect("dialog at tick 2").0;
    host.submit(Command::AnswerBlockingDialog {
        dialog_id: dlg,
        answer: DialogAnswer {
            status: 3,
            bitmap_id: None,
            text: None,
        },
    });
    host.submit(Command::SelectPerson {
        sim_id: Some(mortimer),
    });
    bob.submit(Command::SelectPerson {
        sim_id: Some(bella),
    });
    bob.submit(Command::SetCameraRotation {
        rotation: CameraRotation::WEST,
    });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);

    for tick in 3..=40 {
        let reps = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
        assert_eq!(
            reps[0].state_fingerprint, reps[1].state_fingerprint,
            "pre-join tick {tick}"
        );
    }

    // Mid-game join: Carol connects at tick 40 and receives a compressed,
    // chunked co-op snapshot from Alice!
    let mut carol = CoopPeer {
        node: LockstepNode::new_peer(
            "Carol",
            Some("mem:carol".to_string()),
            manifest,
            host_addr.clone(),
        ),
        transport: mesh.add_node("mem:carol"),
        lot: load_goth_lot(&root, &house),
        step_us: 1_500,
        drift_events: Vec::new(),
        last_opened_dialog: None,
    };
    carol.connect_and_hello(&host_addr);

    let mut snap_sent = false;
    let mut snap_applied = false;
    for _ in 0..30 {
        mesh.step();
        for ev in host.poll_events() {
            if let NodeEvent::SnapshotNeeded { snapshot_id, .. } = ev {
                let snap_bytes = host.lot.coop_snapshot_bytes().expect("encode coop snap");
                let wh = host.lot.coop_world_hash();
                host.node
                    .send_snapshot(
                        &mut host.transport,
                        snapshot_id,
                        wh,
                        snap_bytes,
                        SnapshotCompression::Deflate,
                        16 * 1024,
                    )
                    .expect("host send snapshot");
                snap_sent = true;
            }
        }
        bob.poll_events();
        for ev in carol.poll_events() {
            if let NodeEvent::ApplySnapshot {
                snapshot_id,
                at_tick,
                state_fingerprint,
                redb_bytes,
            } = ev
            {
                assert_eq!(at_tick, Tick(40));
                carol
                    .node
                    .confirm_snapshot_applied(&mut carol.transport, snapshot_id)
                    .expect("confirm snapshot applied");
                carol
                    .lot
                    .restore_coop_snapshot(&redb_bytes, carol.node.session().clone())
                    .expect("restore coop snapshot");
                assert_eq!(
                    carol.lot.coop_state_fingerprint(carol.node.session()),
                    state_fingerprint,
                    "restored snapshot fingerprint must match host"
                );
                snap_applied = true;
            }
        }
        if snap_sent && snap_applied {
            break;
        }
    }
    assert!(snap_sent, "host should have sent snapshot");
    assert!(snap_applied, "carol should have installed snapshot");
    pump_mesh(&mesh, &mut [&mut host, &mut bob, &mut carol], 4);

    // Tick 41 stamps PlayerJoined(Carol) across all 3 peers identically!
    let r41 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob, &mut carol], true);
    assert_eq!(r41[0].state_fingerprint, r41[1].state_fingerprint);
    assert_eq!(r41[0].state_fingerprint, r41[2].state_fingerprint);

    // Carol selects Cassandra and rotates her camera to SOUTH (2).
    carol.submit(Command::SelectPerson {
        sim_id: Some(cassandra),
    });
    carol.submit(Command::SetCameraRotation {
        rotation: CameraRotation::SOUTH,
    });
    pump_mesh(&mesh, &mut [&mut host, &mut bob, &mut carol], 4);

    for tick in 42..=120 {
        let reps = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob, &mut carol], true);
        assert_eq!(
            reps[0].state_fingerprint, reps[1].state_fingerprint,
            "alice vs bob mismatch at tick {tick}"
        );
        assert_eq!(
            reps[0].state_fingerprint, reps[2].state_fingerprint,
            "alice vs carol mismatch at tick {tick}"
        );
        assert_eq!(
            host.lot.world().hash(),
            carol.lot.world().hash(),
            "World::hash mismatch after mid-game join at tick {tick}"
        );
    }
    assert!(host.drift_events.is_empty());
    assert!(bob.drift_events.is_empty());
    assert!(carol.drift_events.is_empty());
}

#[test]
fn coop_mode_global_19_multi_reason_pause_and_fixed_session_globals() {
    let Some((root, house)) = goth_house() else {
        return;
    };

    let mesh = MemoryMesh::new(303);
    let manifest = sample_manifest();
    let config = SessionConfig {
        machine_level: 77,
        sim_language: 3,
        free_will: false,
        host_speed_lock: false,
        tick_delay: 1,
        min_tick_delay: 1,
        max_tick_delay: 8,
        fingerprint_interval: 5,
        max_players: 4,
        ..SessionConfig::default()
    };

    let mut host_lot = load_goth_lot(&root, &house);
    let bob_lot = load_goth_lot(&root, &house);

    let mut host_node =
        LockstepNode::new_host("Alice", Some("mem:host".to_string()), manifest, config);
    host_node
        .session_mut()
        .players
        .get_mut(&PlayerId(1))
        .unwrap()
        .selected_sim = host_lot.world().selected();
    host_lot
        .attach_coop(host_node.session().clone())
        .expect("attach host coop");

    // Verify fixed session globals synced immediately on attach!
    assert_eq!(host_lot.world().globals[33], 77);
    assert_eq!(host_lot.world().globals[15], 3);
    assert_eq!(host_lot.world().globals[30], 0);

    let host_addr = PeerAddress::new("mem:host");
    let mut host = CoopPeer {
        node: host_node,
        transport: mesh.add_node("mem:host"),
        lot: host_lot,
        step_us: 1_000,
        drift_events: Vec::new(),
        last_opened_dialog: None,
    };
    let mut bob = CoopPeer {
        node: LockstepNode::new_peer(
            "Bob",
            Some("mem:bob".to_string()),
            manifest,
            host_addr.clone(),
        ),
        transport: mesh.add_node("mem:bob"),
        lot: bob_lot,
        step_us: 1_000,
        drift_events: Vec::new(),
        last_opened_dialog: None,
    };
    bob.connect_and_hello(&host_addr);
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 6);
    bob.lot
        .attach_coop(bob.node.session().clone())
        .expect("attach bob coop");

    // Tick 1 (Live), Tick 2 (blocking dialog), Tick 3 (answer dialog -> resumes).
    step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    let r2 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    let dlg = r2[0].opened_dialog.as_ref().expect("dialog").0;
    host.submit(Command::AnswerBlockingDialog {
        dialog_id: dlg,
        answer: DialogAnswer {
            status: 3,
            bitmap_id: None,
            text: None,
        },
    });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);
    step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);

    let ticks_before_pause = host.lot.world().ticks;

    // 1. Bob enters Buy mode -> Global 19 = 2 (Buy), paused!
    bob.submit(Command::SetMode {
        mode: PlayerMode::Buy,
    });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);
    let r4 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert!(!r4[0].stepped && !r4[1].stepped);
    assert_eq!(host.lot.mode(), PanelMode::Buy);
    assert_eq!(host.lot.world().globals[19], 2);
    assert_eq!(host.lot.world().globals[17], 1);
    assert_eq!(host.lot.world().ticks, ticks_before_pause);
    assert_eq!(r4[0].state_fingerprint, r4[1].state_fingerprint);

    // 2. Alice enters Build mode while Bob is in Buy -> Build takes precedence (Global 19 = 1)!
    host.submit(Command::SetMode {
        mode: PlayerMode::Build,
    });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);
    let r5 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert!(!r5[0].stepped);
    assert_eq!(host.lot.mode(), PanelMode::Build);
    assert_eq!(host.lot.world().globals[19], 1);
    assert_eq!(bob.lot.world().globals[19], 1);
    assert_eq!(r5[0].state_fingerprint, r5[1].state_fingerprint);

    // 3. Alice returns to Live, Bob stays in Buy -> Global 19 = 2 (Buy), STILL paused!
    host.submit(Command::SetMode {
        mode: PlayerMode::Live,
    });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);
    let r6 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert!(!r6[0].stepped);
    assert_eq!(host.lot.mode(), PanelMode::Buy);
    assert_eq!(host.lot.world().globals[19], 2);
    assert_eq!(r6[0].state_fingerprint, r6[1].state_fingerprint);

    // 4. Alice sets explicit pause, Bob leaves Buy -> Global 19 = 0 (Live), STILL paused!
    host.submit(Command::SetPaused { paused: true });
    bob.submit(Command::SetMode {
        mode: PlayerMode::Live,
    });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);
    let r7 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert!(!r7[0].stepped);
    assert_eq!(host.lot.mode(), PanelMode::Live);
    assert_eq!(host.lot.world().globals[19], 0);
    assert_eq!(host.lot.world().globals[17], 1);
    assert_eq!(host.lot.world().ticks, ticks_before_pause);

    // 5. Toggle Free Will on, enable host speed lock, test speed rejection for Bob & success for Alice.
    bob.submit(Command::SetFreeWill { enabled: true });
    host.submit(Command::SetHostSpeedLock { locked: true });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);
    let r8 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert_eq!(host.lot.world().globals[30], 1);
    assert_eq!(bob.lot.world().globals[30], 1);
    assert_eq!(r8[0].state_fingerprint, r8[1].state_fingerprint);

    bob.submit(Command::SetSpeed {
        speed: SimSpeed::ULTRA,
    });
    host.submit(Command::SetSpeed {
        speed: SimSpeed::FAST,
    });
    host.submit(Command::SetPaused { paused: false });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);
    let r9 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert!(r9[0].stepped && r9[1].stepped);
    assert!(r9[0].command_results.iter().any(|(_, st)| matches!(
        st,
        CoopCommandStatus::RejectedBySession(CommandRejectReason::SpeedLockedByHost)
    )));
    assert_eq!(host.lot.world().speed(), SimSpeed::FAST.value());
    assert_eq!(bob.lot.world().speed(), SimSpeed::FAST.value());
    assert_eq!(host.lot.world().globals[17], 0);
    assert_eq!(host.lot.world().ticks, ticks_before_pause + 1);
    assert_eq!(r9[0].state_fingerprint, r9[1].state_fingerprint);
}

#[test]
fn coop_blocking_dialog_invalid_answer_reopens_and_mid_dialog_snapshot_resumes() {
    let Some((root, house)) = goth_house() else {
        return;
    };

    let mesh = MemoryMesh::new(404);
    let manifest = sample_manifest();
    let config = SessionConfig {
        tick_delay: 1,
        min_tick_delay: 1,
        max_tick_delay: 8,
        fingerprint_interval: 5,
        max_players: 4,
        ..SessionConfig::default()
    };

    let mut host_lot = load_goth_lot(&root, &house);
    let mut host_node =
        LockstepNode::new_host("Alice", Some("mem:host".to_string()), manifest, config);
    host_lot.sync_initial_coop_session(host_node.session_mut());
    host_lot
        .attach_coop(host_node.session().clone())
        .expect("attach host");

    let host_addr = PeerAddress::new("mem:host");
    let mut host = CoopPeer {
        node: host_node,
        transport: mesh.add_node("mem:host"),
        lot: host_lot,
        step_us: 1_000,
        drift_events: Vec::new(),
        last_opened_dialog: None,
    };

    // Advance host alone through tick 1 and tick 2 (opens Help System modal dialog).
    step_lockstep_tick(&mesh, &mut host, &mut [], true);
    let r2 = step_lockstep_tick(&mesh, &mut host, &mut [], true);
    let dlg_id = r2[0]
        .opened_dialog
        .as_ref()
        .expect("dialog opens on tick 2")
        .0;
    assert!(host.lot.asking().is_some());

    // Bob joins MID-DIALOG via snapshot!
    let mut bob = CoopPeer {
        node: LockstepNode::new_peer(
            "Bob",
            Some("mem:bob".to_string()),
            manifest,
            host_addr.clone(),
        ),
        transport: mesh.add_node("mem:bob"),
        lot: load_goth_lot(&root, &house),
        step_us: 1_000,
        drift_events: Vec::new(),
        last_opened_dialog: None,
    };
    bob.connect_and_hello(&host_addr);

    let mut applied = false;
    for _ in 0..24 {
        mesh.step();
        for ev in host.poll_events() {
            if let NodeEvent::SnapshotNeeded { snapshot_id, .. } = ev {
                let bytes = host.lot.coop_snapshot_bytes().expect("coop snap bytes");
                let wh = host.lot.coop_world_hash();
                host.node
                    .send_snapshot(
                        &mut host.transport,
                        snapshot_id,
                        wh,
                        bytes,
                        SnapshotCompression::Deflate,
                        32 * 1024,
                    )
                    .expect("send snap");
            }
        }
        for ev in bob.poll_events() {
            if let NodeEvent::ApplySnapshot {
                snapshot_id,
                state_fingerprint,
                redb_bytes,
                ..
            } = ev
            {
                bob.node
                    .confirm_snapshot_applied(&mut bob.transport, snapshot_id)
                    .expect("confirm snap");
                bob.lot
                    .restore_coop_snapshot(&redb_bytes, bob.node.session().clone())
                    .expect("restore mid-dialog snapshot");
                assert!(
                    bob.lot.asking().is_some(),
                    "mid-dialog snapshot must restore open asking state"
                );
                assert_eq!(
                    bob.lot.coop_state_fingerprint(bob.node.session()),
                    state_fingerprint
                );
                applied = true;
            }
        }
        if applied {
            break;
        }
    }
    assert!(applied);
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);

    // Tick 3: PlayerJoined(Bob) executes while dialog stays open; also test an
    // invalid button status (99) is deterministically refused and reopens dialog.
    host.submit(Command::AnswerBlockingDialog {
        dialog_id: dlg_id,
        answer: DialogAnswer {
            status: 99,
            bitmap_id: None,
            text: None,
        },
    });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);
    let r3 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert!(!r3[0].stepped && !r3[1].stepped);
    assert!(host.node.session().open_dialogs.contains_key(&dlg_id));
    assert!(bob.node.session().open_dialogs.contains_key(&dlg_id));
    assert_eq!(r3[0].state_fingerprint, r3[1].state_fingerprint);

    // Tick 4: Bob answers with valid status 3; both peers replay tick 2 and converge!
    bob.submit(Command::AnswerBlockingDialog {
        dialog_id: dlg_id,
        answer: DialogAnswer {
            status: 3,
            bitmap_id: None,
            text: None,
        },
    });
    pump_mesh(&mesh, &mut [&mut host, &mut bob], 4);
    let r4 = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
    assert!(r4[0].stepped && r4[1].stepped);
    assert_eq!(r4[0].state_fingerprint, r4[1].state_fingerprint);
    assert_eq!(host.lot.world().hash(), bob.lot.world().hash());

    for tick in 5..=25 {
        let reps = step_lockstep_tick(&mesh, &mut host, &mut [&mut bob], true);
        assert_eq!(
            reps[0].state_fingerprint, reps[1].state_fingerprint,
            "tick {tick}"
        );
    }
}

#[test]
fn single_player_goth_house_tick_33_bit_for_bit_unchanged() {
    let Some((root, house)) = goth_house() else {
        return;
    };
    const TICK_33: &str = "7fc16846136f3dd5b18a941d4173446d42a799451883d990ed10534b5e994e6d";
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).expect("load Goth house");
    assert!(lot.coop().is_none());
    for _ in 0..33 {
        lot.step().expect("step");
    }
    let hex: String = lot
        .world()
        .hash()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(hex, TICK_33);
}

fn synthetic_person(id: u16, guid: u32, pos: [i32; 3]) -> Object {
    let idle = Interaction {
        join: 0,
        uid: 0,
        caller: 0,
        target: 0,
        icon: 0,
        table_index: -1,
        args: [0; 4],
        priority: 0,
        action: 0,
        attenuation: 0.0,
        flags: 0,
    };
    let mut data = vec![0; 128];
    data[11] = id as i16;
    Object {
        id,
        guid,
        footprint: [0; 4],
        position: pos,
        unknown_short: 0,
        attributes: vec![0; 8],
        temps: vec![0; 8],
        data,
        obsolete_word: 0,
        stack_capacity_bytes: 0,
        stack: Vec::new(),
        relationship_flag: 0,
        relationships: Vec::new(),
        slots: Vec::new(),
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: None,
        person: Some(Person {
            animation_events: 0,
            engaged: 0,
            appearance: Default::default(),
            accessories: Vec::new(),
            animations: Default::default(),
            routing_state: 0,
            floats: [0.0; 9],
            previous_motives: [0.0; 16],
            motives: [0.0; 16],
            data: vec![0; sims_sim::neighbors::PERSON_WORDS],
            routing_frames: 0,
            active: idle.clone(),
            last: idle,
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        }),
        registry_word: None,
        last_result: false,
        removal_skips_resets: false,
        selected: false,
    }
}

#[test]
fn coop_camera_rotation_affects_look_towards_and_gsc_12_per_player() {
    // Always runs without SIMS_INSTALL.
    let mut behaviors = BTreeMap::new();
    // Tree 4096: Generic Sims Call 12 (tile_depth -> Temp[0])
    behaviors.insert(
        4096,
        Behavior {
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            revision: 0,
            instructions: vec![Instruction {
                opcode: 1,
                on_true: 0xfe,
                on_false: 0xff,
                operands: [12, 0, 0, 0, 0, 0, 0, 0],
            }],
        },
    );
    let mut libraries = BTreeMap::new();
    libraries.insert(
        0,
        sims_vm::Library {
            behaviors,
            ..sims_vm::Library::default()
        },
    );
    let program = Program {
        global: 0,
        libraries,
        semi_globals: BTreeMap::new(),
        careers: None,
        career_texts_end: false,
        labels_hold_no_token: false,
        zoning: BTreeMap::new(),
        animations: Default::default(),
        people: Default::default(),
        icon_frames: None,
        wardrobe: None,
        ordered_wardrobe: None,
        character_files: Default::default(),
        expansions: 0,
        characters: Default::default(),
    };
    let objects_catalog = vec![
        sims_content::Object {
            guid: 0xA001,
            resource: 0,
            chunk: 1,
            kind: 2,
            master: 0,
        },
        sims_content::Object {
            guid: 0xA002,
            resource: 0,
            chunk: 2,
            kind: 2,
            master: 0,
        },
    ];
    let by_guid = objects_catalog
        .iter()
        .enumerate()
        .map(|(i, o)| (o.guid, i))
        .collect();
    let catalog = Catalog {
        global: 0,
        objects: objects_catalog,
        by_guid,
        semi_globals: BTreeMap::new(),
        diagnostics: Vec::new(),
    };

    let mut world = World::empty();
    world.size = 64;
    world.globals = vec![0; 64];
    world.types.insert(1, 0xA001);
    world.types.insert(2, 0xA002);
    // Tile (10, 20)
    world
        .objects
        .insert(10, synthetic_person(10, 0xA001, [160, 320, 1]));
    world
        .objects
        .insert(20, synthetic_person(20, 0xA002, [160, 320, 1]));
    world.order = vec![10, 20];
    world.people = vec![10, 20];

    let mut session = SessionState::new("Host", None, SessionConfig::default());
    session.players.get_mut(&PlayerId(1)).unwrap().selected_sim = Some(10);
    session
        .players
        .get_mut(&PlayerId(1))
        .unwrap()
        .camera_rotation = CameraRotation::EAST; // 1: (63-20, 10) = (43, 10) -> -53
    session.players.insert(
        PlayerId(2),
        sims_net::PerPlayerState {
            player_id: PlayerId(2),
            name: "Peer".into(),
            role: PlayerRole::Peer,
            mode: PlayerMode::Live,
            selected_sim: Some(20),
            camera_rotation: CameraRotation::WEST, // 3: (20, 63-10) = (20, 53) -> -73
            endpoint_hint: None,
            transport_addr: None,
        },
    );

    let coop = RefCell::new(CoopVmContext::new(session));
    let mut interp = Interpreter::new(&program, &catalog, Stream::Lot);
    interp.view_rotation = 0;
    interp.coop = Some(&coop);

    interp.run_tree(&mut world, 10, 4096, 10, &[], 100).unwrap();
    interp.run_tree(&mut world, 20, 4096, 20, &[], 100).unwrap();
    assert_eq!(world.objects[&10].temps[0], -53);
    assert_eq!(world.objects[&20].temps[0], -73);
}
