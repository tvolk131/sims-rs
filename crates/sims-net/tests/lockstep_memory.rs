//! Deterministic in-memory mesh integration tests for `sims-net`.
//!
//! Covers:
//! - 2–4 player lockstep convergence and canonical state fingerprints
//! - Multi-reason pause (explicit, Buy/Build, blocking dialog) & Mode Global 19
//! - Host speed lock & player attribution
//! - Blocking dialog first-answer-wins ordering
//! - Per-player `Selected Person ID` (Global 3) & `camera_rotation` rules
//! - Configurable tick delay buffer & adaptive pacing to slowest machine
//! - File manifest digest & mismatch rejection
//! - Mid-game join with chunked/compressed REDB snapshot transfer
//! - State drift detection & automatic snapshot recovery
//! - Deterministic host failover with epoch fencing
//! - Fault injection: latency, packet reordering, duplicate/corrupted chunks, zip bomb guard

use serde::{Deserialize, Serialize};
use sims_net::manifest::sha256_bytes;
use sims_net::{
    AppliedBatch, BuyBuildEdit, CameraRotation, Command, CommandIgnoreReason, CommandOutcome,
    CommandRejectReason, DialogAnswer, DialogId, Epoch, FileManifest, FileManifestEntry,
    HostStepResult, LinkConfig, LockstepNode, ManifestDigest, MemoryMesh, MemoryNodeTransport,
    NodeEvent, PeerAddress, PlayerId, PlayerMode, PlayerRole, RejectReason, ResyncReason,
    SessionConfig, SimSpeed, SnapshotChunk, SnapshotCompression, SnapshotEnvelope, SnapshotError,
    SnapshotId, SnapshotReceiver, SnapshotSender, Tick, Transport, WireMessage, decode_framed,
    encode_framed,
};

/// Minimal deterministic toy lot state to stand in for `sims_sim::World`
/// during Part 1 protocol verification (serializes as mock REDB bytes).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct MockLotWorld {
    sim_ticks: u64,
    funds: i32,
    actions: Vec<(u64, u32, String)>,
    corrupt_salt: u64,
}

impl MockLotWorld {
    fn new() -> Self {
        Self {
            sim_ticks: 0,
            funds: 20_000,
            actions: Vec::new(),
            corrupt_salt: 0,
        }
    }

    fn apply_batch(&mut self, batch: &AppliedBatch) {
        if !batch.is_paused {
            self.sim_ticks += 1;
        }
        for cmd in batch.accepted_commands() {
            match &cmd.command {
                Command::SimActionPick {
                    sim_id,
                    target_id,
                    menu_index,
                    ..
                } => {
                    self.actions.push((
                        batch.tick.0,
                        cmd.issuer.0,
                        format!("pick:{sim_id}->{target_id}:{menu_index}"),
                    ));
                }
                Command::SimActionCancel {
                    sim_id,
                    interaction_uid,
                } => {
                    self.actions.push((
                        batch.tick.0,
                        cmd.issuer.0,
                        format!("cancel:{sim_id}:{interaction_uid}"),
                    ));
                }
                Command::BuyBuild(BuyBuildEdit::BuyObject { guid, .. }) => {
                    self.funds -= 250;
                    self.actions
                        .push((batch.tick.0, cmd.issuer.0, format!("buy:{guid}")));
                }
                Command::BuyBuild(BuyBuildEdit::SellObject { object_id }) => {
                    self.funds += 100;
                    self.actions
                        .push((batch.tick.0, cmd.issuer.0, format!("sell:{object_id}")));
                }
                Command::AnswerBlockingDialog { dialog_id, answer } => {
                    self.actions.push((
                        batch.tick.0,
                        cmd.issuer.0,
                        format!("dlg:{}:{}", dialog_id.0, answer.status),
                    ));
                }
                _ => {}
            }
        }
    }

    fn hash(&self) -> [u8; 32] {
        let bytes = postcard::to_allocvec(self).expect("encode mock world");
        sha256_bytes(&bytes)
    }

    fn to_redb_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("encode mock redb bytes")
    }

    fn from_redb_bytes(bytes: &[u8]) -> Self {
        postcard::from_bytes(bytes).expect("decode mock redb bytes")
    }
}

struct TestPeer {
    node: LockstepNode,
    transport: MemoryNodeTransport,
    world: MockLotWorld,
    step_us: u32,
}

impl TestPeer {
    fn submit(&mut self, command: Command) {
        self.node
            .submit_command(&mut self.transport, command)
            .expect("submit command");
    }

    fn poll_and_apply_snapshots(&mut self) -> Vec<NodeEvent> {
        self.node.poll(&mut self.transport);
        let events = self.node.drain_events();
        for ev in &events {
            if let NodeEvent::ApplySnapshot {
                snapshot_id,
                redb_bytes,
                ..
            } = ev
            {
                self.world = MockLotWorld::from_redb_bytes(redb_bytes);
                self.node
                    .confirm_snapshot_applied(&mut self.transport, *snapshot_id)
                    .expect("confirm snapshot applied");
            }
        }
        events
    }

    fn step_ready(&mut self, flush: bool) -> Vec<AppliedBatch> {
        let mut batches = Vec::new();
        while let Some(batch) = self.node.step_next_tick(flush).expect("step next tick") {
            self.world.apply_batch(&batch);
            self.node
                .report_local_tick(
                    &mut self.transport,
                    batch.tick,
                    self.step_us,
                    Some(self.world.hash()),
                )
                .expect("report local tick");
            batches.push(batch);
        }
        batches
    }
}

fn sample_manifest(tag: &[u8]) -> ManifestDigest {
    let manifest = FileManifest::from_entries([
        FileManifestEntry {
            relative_path: "GameData/Objects/Objects.far".to_string(),
            size_bytes: 1234,
            sha256: sha256_bytes(tag),
        },
        FileManifestEntry {
            relative_path: "ExpansionPack/ExpansionPack.far".to_string(),
            size_bytes: 5678,
            sha256: sha256_bytes(b"ep1-shared"),
        },
    ])
    .expect("valid manifest");
    manifest.digest()
}

#[test]
fn four_player_lockstep_convergence_over_memory_mesh() {
    let mesh = MemoryMesh::new(42);
    let manifest = sample_manifest(b"complete-collection-v1");
    let config = SessionConfig {
        tick_delay: 2,
        fingerprint_interval: 10,
        max_players: 4,
        ..SessionConfig::default()
    };

    let host_addr = PeerAddress::new("mem:host");
    let mut host = TestPeer {
        node: LockstepNode::new_host("Alice", Some("mem:host".to_string()), manifest, config),
        transport: mesh.add_node("mem:host"),
        world: MockLotWorld::new(),
        step_us: 2_000,
    };

    let mut peers = Vec::new();
    for (i, name) in ["Bob", "Carol", "Dave"].iter().enumerate() {
        let addr_str = format!("mem:p{}", i + 2);
        let mut transport = mesh.add_node(&addr_str);
        transport.connect(&host_addr).expect("connect to host");
        let node = LockstepNode::new_peer(*name, Some(addr_str), manifest, host_addr.clone());
        peers.push(TestPeer {
            node,
            transport,
            world: MockLotWorld::new(),
            step_us: 2_500 + (i as u32) * 500,
        });
    }

    // Complete handshake across all 3 peers.
    for _ in 0..4 {
        for peer in &mut peers {
            peer.poll_and_apply_snapshots();
        }
        host.poll_and_apply_snapshots();
        mesh.step();
    }
    for (idx, peer) in peers.iter().enumerate() {
        assert_eq!(
            peer.node.local_player_id(),
            Some(PlayerId((idx + 2) as u32))
        );
    }

    // Run 40 ticks with interleaved commands from all 4 players.
    for t in 1..=40u64 {
        if t == 3 {
            host.submit(Command::SelectPerson { sim_id: Some(10) });
            peers[0].submit(Command::SelectPerson { sim_id: Some(11) });
            peers[1].submit(Command::SetCameraRotation {
                rotation: CameraRotation::WEST,
            });
        }
        if t == 8 {
            peers[2].submit(Command::BuyBuild(BuyBuildEdit::BuyObject {
                guid: 0x1234,
                position: [16, 24, 0],
                facing: 2,
                container: None,
            }));
        }
        if t == 15 {
            peers[0].submit(Command::SimActionPick {
                sim_id: 11,
                target_id: 42,
                menu_index: 3,
                joining: false,
                args: [0; 4],
            });
        }
        if t == 22 {
            peers[1].submit(Command::SetSpeed {
                speed: SimSpeed::FAST,
            });
        }

        mesh.step();
        host.poll_and_apply_snapshots();
        assert!(matches!(
            host.node.advance_host_tick(&mut host.transport).unwrap(),
            HostStepResult::Scheduled(_)
        ));
        host.step_ready(false);

        mesh.step();
        for peer in &mut peers {
            let events = peer.poll_and_apply_snapshots();
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, NodeEvent::DriftDetected { .. })),
                "unexpected drift on peer: {events:?}"
            );
            peer.step_ready(false);
        }
    }

    // Flush remaining buffered ticks on peers so all 4 reach tick 40.
    for _ in 0..5 {
        mesh.step();
        host.poll_and_apply_snapshots();
        host.step_ready(true);
        for peer in &mut peers {
            peer.poll_and_apply_snapshots();
            peer.step_ready(true);
        }
    }

    assert_eq!(host.node.session().current_tick, Tick(40));
    assert_eq!(host.node.session().players.len(), 4);
    let host_fp = host.node.session().combined_fingerprint(host.world.hash());
    for peer in &peers {
        assert_eq!(peer.node.session().current_tick, Tick(40));
        assert_eq!(peer.world, host.world);
        assert_eq!(peer.node.session(), host.node.session());
        assert_eq!(
            peer.node.session().combined_fingerprint(peer.world.hash()),
            host_fp
        );
    }
}

#[test]
fn multi_reason_pause_mode_global_19_speed_lock_and_first_answer_wins() {
    let mesh = MemoryMesh::new(7);
    let manifest = sample_manifest(b"v1");
    let config = SessionConfig {
        tick_delay: 1,
        ..SessionConfig::default()
    };

    let host_addr = PeerAddress::new("mem:host");
    let mut host = TestPeer {
        node: LockstepNode::new_host("Host", Some("mem:host".to_string()), manifest, config),
        transport: mesh.add_node("mem:host"),
        world: MockLotWorld::new(),
        step_us: 1_000,
    };
    let mut p2_t = mesh.add_node("mem:p2");
    let mut p3_t = mesh.add_node("mem:p3");
    p2_t.connect(&host_addr).unwrap();
    p3_t.connect(&host_addr).unwrap();
    let mut p2 = TestPeer {
        node: LockstepNode::new_peer(
            "P2",
            Some("mem:p2".to_string()),
            manifest,
            host_addr.clone(),
        ),
        transport: p2_t,
        world: MockLotWorld::new(),
        step_us: 1_000,
    };
    let mut p3 = TestPeer {
        node: LockstepNode::new_peer("P3", Some("mem:p3".to_string()), manifest, host_addr),
        transport: p3_t,
        world: MockLotWorld::new(),
        step_us: 1_000,
    };

    // Handshake + tick 1 joins P2 and P3.
    for _ in 0..4 {
        p2.poll_and_apply_snapshots();
        p3.poll_and_apply_snapshots();
        host.poll_and_apply_snapshots();
        mesh.step();
    }
    host.node.advance_host_tick(&mut host.transport).unwrap();
    host.step_ready(true);
    mesh.step();
    p2.poll_and_apply_snapshots();
    p3.poll_and_apply_snapshots();
    p2.step_ready(true);
    p3.step_ready(true);

    // 1. Multi-reason pause & Mode Global 19 aggregation.
    assert!(!host.node.session().is_paused());
    assert_eq!(host.node.session().effective_mode(), PlayerMode::Live);
    assert_eq!(host.node.session().mode_global_19(), 2);

    // P2 enters Buy, P3 enters Build -> Mode Global 19 must be Build (0), paused!
    p2.node
        .submit_command(
            &mut p2.transport,
            Command::SetMode {
                mode: PlayerMode::Buy,
            },
        )
        .unwrap();
    p3.node
        .submit_command(
            &mut p3.transport,
            Command::SetMode {
                mode: PlayerMode::Build,
            },
        )
        .unwrap();
    mesh.step();
    host.poll_and_apply_snapshots();
    host.node.advance_host_tick(&mut host.transport).unwrap();
    host.step_ready(true);

    assert!(host.node.session().is_paused());
    assert_eq!(host.node.session().effective_mode(), PlayerMode::Build);
    assert_eq!(host.node.session().mode_global_19(), 0);
    assert_eq!(
        host.node.session().pause_reasons().players_in_buy_or_build,
        vec![
            (PlayerId(2), PlayerMode::Buy),
            (PlayerId(3), PlayerMode::Build)
        ]
    );

    // P3 disconnects while in Build -> Mode Global 19 drops to Buy (1), still paused by P2!
    mesh.crash_node(&PeerAddress::new("mem:p3"));
    host.poll_and_apply_snapshots();
    host.node.advance_host_tick(&mut host.transport).unwrap();
    host.step_ready(true);

    assert!(host.node.session().is_paused());
    assert_eq!(host.node.session().effective_mode(), PlayerMode::Buy);
    assert_eq!(host.node.session().mode_global_19(), 1);

    // P2 returns to Live, AND host enables explicit pause -> still paused by explicit pause!
    p2.node
        .submit_command(
            &mut p2.transport,
            Command::SetMode {
                mode: PlayerMode::Live,
            },
        )
        .unwrap();
    host.node
        .submit_command(&mut host.transport, Command::SetPaused { paused: true })
        .unwrap();
    mesh.step();
    host.poll_and_apply_snapshots();
    host.node.advance_host_tick(&mut host.transport).unwrap();
    host.step_ready(true);

    assert_eq!(host.node.session().effective_mode(), PlayerMode::Live);
    assert!(host.node.session().is_paused());
    assert_eq!(
        host.node.session().time.explicit_pause_by,
        Some(PlayerId(1))
    );

    // 2. Host speed lock & attribution.
    host.node
        .submit_command(
            &mut host.transport,
            Command::SetHostSpeedLock { locked: true },
        )
        .unwrap();
    p2.node
        .submit_command(
            &mut p2.transport,
            Command::SetSpeed {
                speed: SimSpeed::ULTRA,
            },
        )
        .unwrap();
    p2.node
        .submit_command(&mut p2.transport, Command::SetPaused { paused: false })
        .unwrap();
    mesh.step();
    host.poll_and_apply_snapshots();
    host.node.advance_host_tick(&mut host.transport).unwrap();
    let applied = host.step_ready(true);
    let outcomes: Vec<_> = applied[0]
        .commands
        .iter()
        .map(|c| (c.stamped.issuer, &c.outcome))
        .collect();
    assert!(outcomes.iter().any(|(issuer, outcome)| {
        *issuer == PlayerId(2)
            && matches!(
                outcome,
                CommandOutcome::Rejected(CommandRejectReason::SpeedLockedByHost)
            )
    }));
    // Speed stayed NORMAL and explicit pause stayed true!
    assert_eq!(host.node.session().time.speed, SimSpeed::NORMAL);
    assert!(host.node.session().time.explicit_pause);

    // Host unpauses and opens a blocking dialog -> still paused due to dialog!
    host.node
        .submit_command(&mut host.transport, Command::SetPaused { paused: false })
        .unwrap();
    host.node
        .session_mut()
        .open_blocking_dialog(DialogId(99), 50, Tick(5));
    host.node.advance_host_tick(&mut host.transport).unwrap();
    host.step_ready(true);
    assert!(!host.node.session().time.explicit_pause);
    assert!(host.node.session().is_paused());
    assert_eq!(
        host.node.session().pause_reasons().unanswered_dialogs,
        vec![DialogId(99)]
    );

    // 3. First-answer-wins: both Host (P1) and P2 answer DialogId(99) in the same tick.
    // Host sequences commands in deterministic PlayerId order (P1 before P2), so P1 wins!
    host.node
        .submit_command(
            &mut host.transport,
            Command::AnswerBlockingDialog {
                dialog_id: DialogId(99),
                answer: DialogAnswer {
                    status: 2,
                    bitmap_id: None,
                    text: None,
                },
            },
        )
        .unwrap();
    p2.node
        .submit_command(
            &mut p2.transport,
            Command::AnswerBlockingDialog {
                dialog_id: DialogId(99),
                answer: DialogAnswer {
                    status: 3,
                    bitmap_id: None,
                    text: None,
                },
            },
        )
        .unwrap();
    mesh.step();
    host.poll_and_apply_snapshots();
    host.node.advance_host_tick(&mut host.transport).unwrap();
    let applied = host.step_ready(true);
    assert_eq!(applied[0].commands.len(), 2);
    assert!(matches!(
        applied[0].commands[0].outcome,
        CommandOutcome::DialogWon {
            dialog_id: DialogId(99),
            winner: PlayerId(1),
            ..
        }
    ));
    assert_eq!(
        applied[0].commands[1].outcome,
        CommandOutcome::Ignored(CommandIgnoreReason::DialogAlreadyAnswered)
    );
    // Dialog closed -> simulation unpaused!
    assert!(!host.node.session().is_paused());
}

#[test]
fn per_player_selection_and_camera_rotation_policies() {
    let mut state = sims_net::SessionState::new("Host", None, SessionConfig::default());
    let batch = sims_net::TickBatch {
        epoch: Epoch::INITIAL,
        tick: Tick(1),
        pacing_hint_ms: 42,
        host_fingerprint: None,
        commands: vec![
            sims_net::StampedCommand {
                tick: Tick(1),
                sequence: 0,
                issuer: PlayerId(1),
                command: Command::PlayerJoined {
                    player_id: PlayerId(2),
                    name: "Guest".to_string(),
                    selected_sim: Some(20),
                    camera_rotation: CameraRotation::EAST,
                    endpoint_hint: None,
                    transport_addr: None,
                },
            },
            sims_net::StampedCommand {
                tick: Tick(1),
                sequence: 1,
                issuer: PlayerId(1),
                command: Command::SelectPerson { sim_id: Some(10) },
            },
            sims_net::StampedCommand {
                tick: Tick(1),
                sequence: 2,
                issuer: PlayerId(1),
                command: Command::SetCameraRotation {
                    rotation: CameraRotation::SOUTH,
                },
            },
        ],
    };
    state.apply_batch(&batch).unwrap();

    // Global 3 "Am I selected?"
    assert!(state.is_sim_selected_by_any(10));
    assert!(state.is_sim_selected_by_any(20));
    assert!(!state.is_sim_selected_by_any(30));

    // Global 3 "Who is the player's Sim?"
    assert_eq!(state.selected_sim_for_activity(Some(PlayerId(2))), Some(20));
    assert_eq!(state.selected_sim_for_activity(None), Some(10));

    // Per-player camera_rotation (falls back to host's SOUTH for unselected Sim 30).
    assert_eq!(state.camera_rotation_for_sim(20), CameraRotation::EAST);
    assert_eq!(state.camera_rotation_for_sim(10), CameraRotation::SOUTH);
    assert_eq!(state.camera_rotation_for_sim(30), CameraRotation::SOUTH);

    // Script-driven selection change applies only to players who had Sim 20 selected.
    let updated = state.apply_script_selection_change(20, Some(21));
    assert_eq!(updated, vec![PlayerId(2)]);
    assert!(!state.is_sim_selected_by_any(20));
    assert!(state.is_sim_selected_by_any(21));
    assert!(state.is_sim_selected_by_any(10));
}

#[test]
fn adaptive_pacing_stalls_for_slow_peer_and_raises_pacing_hint() {
    let mesh = MemoryMesh::new(99);
    let manifest = sample_manifest(b"v1");
    let config = SessionConfig {
        tick_delay: 2,
        max_ticks_ahead_of_slowest: 4,
        ..SessionConfig::default()
    };

    let host_addr = PeerAddress::new("mem:host");
    let mut host = TestPeer {
        node: LockstepNode::new_host("Host", Some("mem:host".to_string()), manifest, config),
        transport: mesh.add_node("mem:host"),
        world: MockLotWorld::new(),
        step_us: 2_000,
    };
    let mut slow_t = mesh.add_node("mem:slow");
    slow_t.connect(&host_addr).unwrap();
    let mut slow_peer = TestPeer {
        node: LockstepNode::new_peer("Slow", Some("mem:slow".to_string()), manifest, host_addr),
        transport: slow_t,
        world: MockLotWorld::new(),
        step_us: 80_000, // 80 ms/tick machine -> needs 100 ms pacing hint!
    };

    // Handshake.
    for _ in 0..4 {
        slow_peer.poll_and_apply_snapshots();
        host.poll_and_apply_snapshots();
        mesh.step();
    }

    // Host schedules 4 ticks while slow peer hasn't acked tick 1 yet -> 5th stalls!
    for expected in 1..=4u64 {
        let res = host.node.advance_host_tick(&mut host.transport).unwrap();
        assert!(matches!(res, HostStepResult::Scheduled(b) if b.tick == Tick(expected)));
    }
    let stalled = host.node.advance_host_tick(&mut host.transport).unwrap();
    assert_eq!(
        stalled,
        HostStepResult::WaitingForSlowestPeer {
            slowest_peer: PlayerId(2),
            slowest_tick: Tick(0),
        }
    );

    // Deliver batches to slow peer, execute them, and deliver TickAck back to host.
    mesh.step();
    slow_peer.poll_and_apply_snapshots();
    slow_peer.step_ready(false);
    mesh.step();
    host.poll_and_apply_snapshots();

    // Host can now schedule tick 5 and adapts pacing_hint_ms to >= 100 ms (80ms * 1.25).
    let res = host.node.advance_host_tick(&mut host.transport).unwrap();
    match res {
        HostStepResult::Scheduled(batch) => {
            assert_eq!(batch.tick, Tick(5));
            assert!(
                batch.pacing_hint_ms >= 100,
                "expected adaptive pacing >= 100ms, got {}",
                batch.pacing_hint_ms
            );
        }
        other => panic!("expected Scheduled(5), got {other:?}"),
    }
}

#[test]
fn manifest_directory_hashing_diff_and_mismatch_rejection() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(dir_a.path().join("GameData/Objects")).unwrap();
    std::fs::write(
        dir_a.path().join("GameData/Objects/Objects.far"),
        b"same-bytes",
    )
    .unwrap();
    std::fs::write(dir_a.path().join("Downloads/CustomChair.iff"), b"chair-v1").unwrap_or_else(
        |_| {
            std::fs::create_dir_all(dir_a.path().join("Downloads")).unwrap();
            std::fs::write(dir_a.path().join("Downloads/CustomChair.iff"), b"chair-v1").unwrap();
        },
    );

    // Different casing on directory B should produce the exact same digest if files match.
    std::fs::create_dir_all(dir_b.path().join("gamedata/objects")).unwrap();
    std::fs::create_dir_all(dir_b.path().join("downloads")).unwrap();
    std::fs::write(
        dir_b.path().join("gamedata/objects/objects.far"),
        b"same-bytes",
    )
    .unwrap();
    std::fs::write(dir_b.path().join("downloads/customchair.iff"), b"chair-v1").unwrap();

    let man_a = FileManifest::from_directory(dir_a.path(), |_| true).unwrap();
    let man_b = FileManifest::from_directory(dir_b.path(), |_| true).unwrap();
    assert_eq!(man_a.digest(), man_b.digest());
    assert!(man_a.diff(&man_b).is_empty());

    // Mutate custom content on B -> digest mismatches and diff names the file.
    std::fs::write(
        dir_b.path().join("downloads/customchair.iff"),
        b"chair-v2-mod",
    )
    .unwrap();
    let man_b_mod = FileManifest::from_directory(dir_b.path(), |_| true).unwrap();
    assert_ne!(man_a.digest(), man_b_mod.digest());
    let diff = man_b_mod.diff(&man_a);
    assert_eq!(diff.mismatched, vec!["downloads/customchair.iff"]);

    // Verify LockstepNode rejects mismatched manifest cleanly on join.
    let mesh = MemoryMesh::new(11);
    let host_addr = PeerAddress::new("mem:host");
    let mut host_t = mesh.add_node("mem:host");
    let mut host = LockstepNode::new_host("Host", None, man_a.digest(), SessionConfig::default());

    let mut bad_t = mesh.add_node("mem:bad");
    bad_t.connect(&host_addr).unwrap();
    let mut bad_peer = LockstepNode::new_peer("Bad", None, man_b_mod.digest(), host_addr);

    for _ in 0..4 {
        bad_peer.poll(&mut bad_t);
        host.poll(&mut host_t);
        mesh.step();
    }
    let events = bad_peer.drain_events();
    assert!(
        events.contains(&NodeEvent::JoinRejected(RejectReason::ManifestMismatch {
            expected: man_a.digest(),
            actual: man_b_mod.digest(),
        }))
    );
}

#[test]
fn mid_game_join_with_chunked_redb_snapshot_converges_bit_for_bit() {
    let mesh = MemoryMesh::new(123);
    let manifest = sample_manifest(b"v1");
    let config = SessionConfig {
        tick_delay: 2,
        fingerprint_interval: 5,
        ..SessionConfig::default()
    };

    let host_addr = PeerAddress::new("mem:host");
    let mut host = TestPeer {
        node: LockstepNode::new_host("Host", Some("mem:host".to_string()), manifest, config),
        transport: mesh.add_node("mem:host"),
        world: MockLotWorld::new(),
        step_us: 1_000,
    };

    // Advance host to tick 15 before any peer joins.
    for t in 1..=15u64 {
        if t % 3 == 0 {
            host.node
                .submit_command(
                    &mut host.transport,
                    Command::BuyBuild(BuyBuildEdit::BuyObject {
                        guid: t as u32,
                        position: [t as i32, 0, 0],
                        facing: 0,
                        container: None,
                    }),
                )
                .unwrap();
        }
        host.node.advance_host_tick(&mut host.transport).unwrap();
        host.step_ready(true);
    }
    assert_eq!(host.node.session().current_tick, Tick(15));

    // Late joiner connects mid-game.
    let mut late_t = mesh.add_node("mem:late");
    late_t.connect(&host_addr).unwrap();
    let mut late = TestPeer {
        node: LockstepNode::new_peer("Late", Some("mem:late".to_string()), manifest, host_addr),
        transport: late_t,
        world: MockLotWorld::new(),
        step_us: 1_000,
    };

    // Exchange Hello -> Welcome + SnapshotNeeded.
    let mut snapshot_id = None;
    for _ in 0..4 {
        late.poll_and_apply_snapshots();
        for ev in host.poll_and_apply_snapshots() {
            if let NodeEvent::SnapshotNeeded {
                snapshot_id: sid,
                reason: ResyncReason::MidGameJoin,
                ..
            } = ev
            {
                snapshot_id = Some(sid);
            }
        }
        mesh.step();
    }
    let sid = snapshot_id.expect("host must request mid-game snapshot");

    // Host sends chunked deflated snapshot (small 64-byte chunks to exercise multi-chunk framing!)
    // AND schedules ticks 16..=20 concurrently while joiner is receiving chunks.
    host.node
        .send_snapshot(
            &mut host.transport,
            sid,
            host.world.hash(),
            host.world.to_redb_bytes(),
            SnapshotCompression::Deflate,
            64,
        )
        .unwrap();

    for t in 16..=20u64 {
        if t == 18 {
            host.node
                .submit_command(
                    &mut host.transport,
                    Command::SimActionPick {
                        sim_id: 1,
                        target_id: 2,
                        menu_index: 1,
                        joining: false,
                        args: [0; 4],
                    },
                )
                .unwrap();
        }
        host.node.advance_host_tick(&mut host.transport).unwrap();
        host.step_ready(true);
        mesh.step();
        late.poll_and_apply_snapshots();
        late.step_ready(false);
    }

    // Flush late joiner to tick 20.
    for _ in 0..5 {
        mesh.step();
        host.poll_and_apply_snapshots();
        late.poll_and_apply_snapshots();
        late.step_ready(true);
    }

    assert_eq!(late.node.session().current_tick, Tick(20));
    assert_eq!(late.world, host.world);
    assert_eq!(late.node.session(), host.node.session());
    assert_eq!(
        late.node.session().combined_fingerprint(late.world.hash()),
        host.node.session().combined_fingerprint(host.world.hash())
    );
}

#[test]
fn drift_detection_triggers_automatic_snapshot_resync() {
    let mesh = MemoryMesh::new(555);
    let manifest = sample_manifest(b"v1");
    let config = SessionConfig {
        tick_delay: 1,
        fingerprint_interval: 5,
        ..SessionConfig::default()
    };

    let host_addr = PeerAddress::new("mem:host");
    let mut host = TestPeer {
        node: LockstepNode::new_host("Host", Some("mem:host".to_string()), manifest, config),
        transport: mesh.add_node("mem:host"),
        world: MockLotWorld::new(),
        step_us: 1_000,
    };
    let mut peer_t = mesh.add_node("mem:p2");
    peer_t.connect(&host_addr).unwrap();
    let mut peer = TestPeer {
        node: LockstepNode::new_peer("P2", Some("mem:p2".to_string()), manifest, host_addr),
        transport: peer_t,
        world: MockLotWorld::new(),
        step_us: 1_000,
    };

    for _ in 0..4 {
        peer.poll_and_apply_snapshots();
        host.poll_and_apply_snapshots();
        mesh.step();
    }

    // Run ticks 1..=3 cleanly, then inject synthetic state drift on peer before tick 5!
    let mut drift_seen = false;
    let mut resync_sid = None;
    for t in 1..=12u64 {
        if t == 4 {
            peer.world.corrupt_salt = 0xDEAD_BEEF;
        }
        host.node.advance_host_tick(&mut host.transport).unwrap();
        host.step_ready(true);
        mesh.step();
        peer.poll_and_apply_snapshots();
        peer.step_ready(true);
        mesh.step();

        for ev in host.poll_and_apply_snapshots() {
            match ev {
                NodeEvent::DriftDetected {
                    player_id: PlayerId(2),
                    tick: Tick(5),
                    ..
                } => drift_seen = true,
                NodeEvent::SnapshotNeeded {
                    snapshot_id,
                    reason: ResyncReason::DriftDetected,
                    ..
                } => resync_sid = Some(snapshot_id),
                _ => {}
            }
        }
        if let Some(sid) = resync_sid.take() {
            host.node
                .send_snapshot(
                    &mut host.transport,
                    sid,
                    host.world.hash(),
                    host.world.to_redb_bytes(),
                    SnapshotCompression::Deflate,
                    128,
                )
                .unwrap();
        }
    }

    for _ in 0..5 {
        mesh.step();
        host.poll_and_apply_snapshots();
        peer.poll_and_apply_snapshots();
        peer.step_ready(true);
    }

    assert!(drift_seen, "expected host to detect peer drift at tick 5");
    assert_eq!(peer.world.corrupt_salt, 0);
    assert_eq!(peer.world, host.world);
    assert_eq!(peer.node.session(), host.node.session());
}

#[test]
fn deterministic_host_failover_promotes_lowest_player_id_and_fences_old_epoch() {
    let mesh = MemoryMesh::new(777);
    let manifest = sample_manifest(b"v1");
    let config = SessionConfig {
        tick_delay: 1,
        ..SessionConfig::default()
    };

    let host_addr = PeerAddress::new("mem:host");
    let mut p1 = TestPeer {
        node: LockstepNode::new_host("P1", Some("mem:host".to_string()), manifest, config),
        transport: mesh.add_node("mem:host"),
        world: MockLotWorld::new(),
        step_us: 1_000,
    };

    let mut p2_t = mesh.add_node("mem:p2");
    let mut p3_t = mesh.add_node("mem:p3");
    p2_t.connect(&host_addr).unwrap();
    p3_t.connect(&host_addr).unwrap();
    let mut p2 = TestPeer {
        node: LockstepNode::new_peer(
            "P2",
            Some("mem:p2".to_string()),
            manifest,
            host_addr.clone(),
        ),
        transport: p2_t,
        world: MockLotWorld::new(),
        step_us: 1_000,
    };
    let mut p3 = TestPeer {
        node: LockstepNode::new_peer("P3", Some("mem:p3".to_string()), manifest, host_addr),
        transport: p3_t,
        world: MockLotWorld::new(),
        step_us: 1_000,
    };

    // Handshake + run 6 ticks so all 3 players are synchronized at tick 6.
    for _ in 0..4 {
        p2.poll_and_apply_snapshots();
        p3.poll_and_apply_snapshots();
        p1.poll_and_apply_snapshots();
        mesh.step();
    }
    for _ in 1..=6 {
        p1.node.advance_host_tick(&mut p1.transport).unwrap();
        p1.step_ready(true);
        mesh.step();
        p2.poll_and_apply_snapshots();
        p3.poll_and_apply_snapshots();
        p2.step_ready(true);
        p3.step_ready(true);
        mesh.step();
        p1.poll_and_apply_snapshots();
    }
    assert_eq!(p2.node.session().current_tick, Tick(6));
    assert_eq!(p3.node.session().current_tick, Tick(6));

    // Crash original host P1!
    mesh.crash_node(&PeerAddress::new("mem:host"));

    let mut p2_promoted = false;
    let mut p3_adopted = false;
    for _ in 0..6 {
        for ev in p2.poll_and_apply_snapshots() {
            if ev
                == (NodeEvent::HostPromoted {
                    new_host: PlayerId(2),
                    new_epoch: Epoch(2),
                })
            {
                p2_promoted = true;
            }
        }
        for ev in p3.poll_and_apply_snapshots() {
            if ev
                == (NodeEvent::HostChanged {
                    new_host: PlayerId(2),
                    new_epoch: Epoch(2),
                })
            {
                p3_adopted = true;
            }
        }
        mesh.step();
    }

    assert!(p2_promoted, "P2 must deterministically become Host");
    assert!(p3_adopted, "P3 must deterministically adopt P2 as Host");
    assert_eq!(p2.node.role(), PlayerRole::Host);
    assert_eq!(p2.node.session().epoch, Epoch(2));
    assert_eq!(p3.node.session().epoch, Epoch(2));
    assert_eq!(p3.node.session().host_id, PlayerId(2));

    // P3 submits a command to the new host P2 and both advance ticks 7..=10 cleanly.
    p3.node
        .submit_command(
            &mut p3.transport,
            Command::BuyBuild(BuyBuildEdit::SellObject { object_id: 77 }),
        )
        .unwrap();
    for _ in 7..=10 {
        mesh.step();
        p2.poll_and_apply_snapshots();
        p2.node.advance_host_tick(&mut p2.transport).unwrap();
        p2.step_ready(true);
        mesh.step();
        p3.poll_and_apply_snapshots();
        p3.step_ready(true);
    }

    assert_eq!(p2.node.session().current_tick, Tick(10));
    assert_eq!(p3.node.session().current_tick, Tick(10));
    assert_eq!(p2.world, p3.world);
    assert_eq!(p2.world.funds, 20_100);
    assert_eq!(p2.node.session(), p3.node.session());
}

#[test]
fn memory_mesh_latency_reordering_and_snapshot_integrity_guards() {
    // 1. Verify MemoryMesh latency + reordering still delivers valid framed TickBatches
    // and LockstepNode orders them strictly by Tick via BTreeMap.
    let mesh = MemoryMesh::new(2026);
    let host_addr = PeerAddress::new("mem:host");
    let peer_addr = PeerAddress::new("mem:peer");
    let mut host_t = mesh.add_node("mem:host");
    let mut peer_t = mesh.add_node("mem:peer");
    mesh.set_link_config(
        &host_addr,
        &peer_addr,
        LinkConfig {
            latency_steps: 2,
            drop_ppm: 0,
            reorder_jitter_steps: 3,
            partitioned: false,
            max_in_flight_per_direction: 0,
        },
    );
    peer_t.connect(&host_addr).unwrap();

    let manifest = sample_manifest(b"v1");
    let mut host = LockstepNode::new_host("Host", None, manifest, SessionConfig::default());
    let mut peer = LockstepNode::new_peer("Peer", None, manifest, host_addr);

    for _ in 0..15 {
        peer.poll(&mut peer_t);
        host.poll(&mut host_t);
        mesh.step();
    }
    assert_eq!(peer.local_player_id(), Some(PlayerId(2)));

    for _ in 1..=8 {
        host.advance_host_tick(&mut host_t).unwrap();
        host.drain_ready_ticks(true).unwrap();
    }
    for _ in 0..15 {
        mesh.step();
        peer.poll(&mut peer_t);
    }
    let applied = peer.drain_ready_ticks(true).unwrap();
    assert_eq!(applied.len(), 8);
    assert_eq!(peer.session().current_tick, Tick(8));

    // 2. Verify SnapshotReceiver handles out-of-order + duplicate chunks and rejects
    // corrupted chunks and mismatched wire frames.
    let env = SnapshotEnvelope {
        session: host.session().clone(),
        redb_bytes: vec![0xAB; 5000],
    };
    let sender = SnapshotSender::from_envelope(
        SnapshotId(42),
        Epoch::INITIAL,
        Tick(8),
        host.session().session_fingerprint(),
        &env,
        SnapshotCompression::Deflate,
        32,
    )
    .unwrap();
    let chunks: Vec<SnapshotChunk> = sender.chunks().collect();
    assert!(chunks.len() > 2);

    // Out-of-order + duplicate identical chunk succeeds.
    let mut rx = SnapshotReceiver::new(sender.header().clone()).unwrap();
    for chunk in chunks.iter().rev() {
        rx.push_chunk(chunk.clone()).unwrap();
    }
    assert!(rx.push_chunk(chunks[0].clone()).unwrap());
    let completed = rx.finish_envelope().unwrap();
    assert_eq!(completed.envelope, env);

    // Conflicting duplicate chunk rejected.
    let mut rx_bad = SnapshotReceiver::new(sender.header().clone()).unwrap();
    rx_bad.push_chunk(chunks[0].clone()).unwrap();
    let mut tampered = chunks[0].clone();
    tampered.data[0] ^= 0xFF;
    assert_eq!(
        rx_bad.push_chunk(tampered),
        Err(SnapshotError::ConflictingDuplicateChunk { index: 0 })
    );

    // Corrupted final digest rejected.
    let mut rx_corrupt = SnapshotReceiver::new(sender.header().clone()).unwrap();
    for (idx, mut chunk) in chunks.into_iter().enumerate() {
        if idx == 0 {
            chunk.data[0] ^= 0xFF;
        }
        rx_corrupt.push_chunk(chunk).unwrap();
    }
    assert_eq!(
        rx_corrupt.finish_raw(),
        Err(SnapshotError::CompressedDigestMismatch)
    );

    // Wire framing round-trip check.
    let msg = WireMessage::Heartbeat {
        epoch: Epoch(3),
        tick: Tick(99),
        sender: Some(PlayerId(2)),
    };
    let framed = encode_framed(&msg).unwrap();
    assert_eq!(decode_framed(&framed).unwrap(), msg);
}

#[test]
fn sim_speed_periods_match_original_and_snapshot_guards_reject_zip_bomb() {
    // Exact parity with crates/sims-sim/src/speed.rs:period_ms.
    assert_eq!(SimSpeed::NORMAL.period_ms(), 42);
    assert_eq!(SimSpeed::FAST.period_ms(), 14);
    assert_eq!(SimSpeed::ULTRA.period_ms(), 2);
    assert_eq!(SimSpeed::new(0).period_ms(), 172);
    assert_eq!(SimSpeed::new(1000).period_ms(), 1);
    assert_eq!(SimSpeed::new(-5).period_ms(), 172);
    assert_eq!(SimSpeed::new(2000).period_ms(), 1);

    // Craft a deflate payload that decompresses to 8192 bytes while claiming uncompressed_len = 128.
    let big_payload = vec![0x41u8; 8192];
    let sender = SnapshotSender::from_raw_bytes(
        SnapshotId(7),
        Epoch::INITIAL,
        Tick(1),
        sims_net::StateFingerprint([0u8; 32]),
        &big_payload,
        SnapshotCompression::Deflate,
        1024,
    )
    .unwrap();
    let mut forged_header = sender.header().clone();
    forged_header.uncompressed_len = 128;

    let mut rx = SnapshotReceiver::new(forged_header).unwrap();
    for chunk in sender.chunks() {
        rx.push_chunk(chunk).unwrap();
    }
    assert_eq!(
        rx.finish_raw(),
        Err(SnapshotError::UncompressedLengthMismatch {
            expected: 128,
            actual: 129,
        })
    );
}

#[test]
fn transport_identity_binding_config_validation_and_trailing_byte_rejection() {
    // 1. SessionConfig::validate rejects invalid delay/lead combinations that would deadlock.
    let bad_cfg = SessionConfig {
        tick_delay: 12,
        max_tick_delay: 12,
        max_ticks_ahead_of_slowest: 10,
        ..SessionConfig::default()
    };
    assert!(bad_cfg.validate().is_err());
    let bad_bounds = SessionConfig {
        min_tick_delay: 8,
        max_tick_delay: 4,
        ..SessionConfig::default()
    };
    assert!(bad_bounds.validate().is_err());

    // 2. Spoofed Welcome from non-host is ignored, and spoofed endpoint_hint on Hello is rejected.
    let mesh = MemoryMesh::new(301);
    let manifest = sample_manifest(b"auth-v1");
    let host_addr = PeerAddress::new("mem:host");
    let peer_addr = PeerAddress::new("mem:peer");

    let mut host_t = mesh.add_node("mem:host");
    let mut attacker_t = mesh.add_node("mem:attacker");
    let mut peer_t = mesh.add_node("mem:peer");
    attacker_t.connect(&host_addr).unwrap();
    attacker_t.connect(&peer_addr).unwrap();
    peer_t.connect(&host_addr).unwrap();

    let mut host = LockstepNode::new_host(
        "Host",
        Some("mem:host".to_string()),
        manifest,
        SessionConfig::default(),
    );
    let mut peer = LockstepNode::new_peer(
        "Peer",
        Some("mem:peer".to_string()),
        manifest,
        host_addr.clone(),
    );

    // Attacker tries to spoof Welcome to peer before real host responds.
    attacker_t
        .send(
            &peer_addr,
            WireMessage::Welcome {
                epoch: Epoch::INITIAL,
                assigned_player: PlayerId(99),
                host_id: PlayerId(1),
                config: SessionConfig::default(),
                current_tick: Tick::ZERO,
                session_state: Box::new(host.session().clone()),
                snapshot_id: None,
                roster: Vec::new(),
            },
        )
        .unwrap();
    // Attacker sends Hello claiming victim's endpoint_hint.
    attacker_t
        .send(
            &host_addr,
            WireMessage::Hello {
                protocol_version: sims_net::PROTOCOL_VERSION,
                player_name: "Attacker".to_string(),
                manifest,
                endpoint_hint: Some("mem:peer".to_string()),
            },
        )
        .unwrap();
    mesh.step();

    peer.poll(&mut peer_t);
    host.poll(&mut host_t);
    // Peer must ignore spoofed Welcome from attacker.
    assert_eq!(peer.local_player_id(), None);
    mesh.step();

    // Attacker receives InvalidEndpointHint rejection from host.
    let attacker_events = attacker_t.drain_events();
    assert!(attacker_events.iter().any(|ev| matches!(
        ev,
        sims_net::TransportEvent::Message {
            message: WireMessage::Reject {
                reason: RejectReason::InvalidEndpointHint
            },
            ..
        }
    )));

    // 3. Trailing bytes rejected in decode_payload and SnapshotEnvelope::decode,
    //    and rooted/drive paths rejected by FileManifest::from_entries.
    let mut payload = sims_net::encode_payload(&WireMessage::Heartbeat {
        epoch: Epoch::INITIAL,
        tick: Tick(1),
        sender: None,
    })
    .unwrap();
    payload.push(0x99);
    assert!(sims_net::decode_payload(&payload).is_err());

    let env = SnapshotEnvelope {
        session: host.session().clone(),
        redb_bytes: vec![1, 2, 3],
    };
    let mut env_bytes = env.encode().unwrap();
    env_bytes.push(0x00);
    assert!(SnapshotEnvelope::decode(&env_bytes).is_err());

    for bad_path in [
        "/gamedata/a.iff",
        "\\gamedata\\a.iff",
        "C:a.iff",
        "C:\\a.iff",
        "../a.iff",
    ] {
        let res = FileManifest::from_entries([FileManifestEntry {
            relative_path: bad_path.to_string(),
            size_bytes: 1,
            sha256: [0; 32],
        }]);
        assert!(res.is_err(), "expected {bad_path} to be rejected");
    }
}

#[test]
fn resumable_snapshot_and_batch_delivery_under_transport_backpressure() {
    let mesh = MemoryMesh::new(404);
    let manifest = sample_manifest(b"bp-v1");
    let host_addr = PeerAddress::new("mem:host");
    let late_addr = PeerAddress::new("mem:late");

    let mut host = TestPeer {
        node: LockstepNode::new_host(
            "Host",
            Some("mem:host".to_string()),
            manifest,
            SessionConfig {
                tick_delay: 1,
                ..SessionConfig::default()
            },
        ),
        transport: mesh.add_node("mem:host"),
        world: MockLotWorld::new(),
        step_us: 1_000,
    };

    // Advance host to tick 3.
    for _ in 1..=3 {
        host.node.advance_host_tick(&mut host.transport).unwrap();
        host.step_ready(true);
    }

    // Impose tiny in-flight cap (2 frames) so multi-chunk snapshot hits Backpressure!
    let mut late_t = mesh.add_node("mem:late");
    late_t.connect(&host_addr).unwrap();
    mesh.set_link_config(
        &host_addr,
        &late_addr,
        LinkConfig {
            latency_steps: 0,
            drop_ppm: 0,
            reorder_jitter_steps: 0,
            partitioned: false,
            max_in_flight_per_direction: 2,
        },
    );
    let mut late = TestPeer {
        node: LockstepNode::new_peer("Late", Some("mem:late".to_string()), manifest, host_addr),
        transport: late_t,
        world: MockLotWorld::new(),
        step_us: 1_000,
    };

    let mut sid = None;
    for _ in 0..8 {
        late.poll_and_apply_snapshots();
        for ev in host.poll_and_apply_snapshots() {
            if let NodeEvent::SnapshotNeeded { snapshot_id, .. } = ev {
                sid = Some(snapshot_id);
            }
        }
        mesh.step();
    }
    let sid = sid.expect("SnapshotNeeded");

    // 4 KiB payload in 64-byte chunks -> > 10 chunks over a 2-frame link cap!
    let big_world_bytes = vec![0x77u8; 4096];
    host.node
        .send_snapshot(
            &mut host.transport,
            sid,
            host.world.hash(),
            big_world_bytes.clone(),
            SnapshotCompression::None,
            64,
        )
        .unwrap();
    // Advance tick 4 (PlayerJoined) while snapshot is still draining under backpressure.
    host.node.advance_host_tick(&mut host.transport).unwrap();
    host.step_ready(true);

    // Pump mesh + poll until all chunks resume and arrive cleanly.
    let mut applied_bytes = None;
    for _ in 0..120 {
        host.node.poll(&mut host.transport);
        mesh.step();
        late.node.poll(&mut late.transport);
        for ev in late.node.drain_events() {
            if let NodeEvent::ApplySnapshot {
                snapshot_id,
                redb_bytes,
                ..
            } = ev
            {
                // Verify peer stays syncing BEFORE confirm_snapshot_applied!
                assert!(late.node.is_syncing());
                assert!(!late.node.can_step_tick(true));
                applied_bytes = Some(redb_bytes);
                late.node
                    .confirm_snapshot_applied(&mut late.transport, snapshot_id)
                    .unwrap();
                assert!(!late.node.is_syncing());
            }
        }
        let _ = late.node.drain_ready_ticks(true);
        if applied_bytes.is_some() && late.node.session().current_tick == Tick(4) {
            break;
        }
    }

    assert_eq!(applied_bytes.as_deref(), Some(big_world_bytes.as_slice()));
    assert_eq!(host.node.active_snapshot_tx_count(), 0);
    assert_eq!(late.node.session(), host.node.session());
}

#[test]
fn join_capacity_ghost_prevention_dialog_receipt_order_and_failover_fallback() {
    // 1. Tick-0 capacity enforcement & disconnect before tick 1 leaves zero ghost player.
    let mesh = MemoryMesh::new(808);
    let manifest = sample_manifest(b"cap-v1");
    let host_addr = PeerAddress::new("mem:host");
    let mut host_t = mesh.add_node("mem:host");
    let mut host = LockstepNode::new_host(
        "Host",
        Some("mem:host".to_string()),
        manifest,
        SessionConfig {
            max_players: 2,
            tick_delay: 2,
            ..SessionConfig::default()
        },
    );

    let mut p2_t = mesh.add_node("mem:p2");
    let mut p3_t = mesh.add_node("mem:p3");
    p2_t.connect(&host_addr).unwrap();
    p3_t.connect(&host_addr).unwrap();
    let mut p2 = LockstepNode::new_peer(
        "P2",
        Some("mem:p2".to_string()),
        manifest,
        host_addr.clone(),
    );
    let mut p3 = LockstepNode::new_peer(
        "P3",
        Some("mem:p3".to_string()),
        manifest,
        host_addr.clone(),
    );

    // Both P2 and P3 send Hello at tick 0 -> P2 accepted, P3 rejected SessionFull!
    p2.poll(&mut p2_t);
    p3.poll(&mut p3_t);
    mesh.step();
    host.poll(&mut host_t);
    mesh.step();
    p2.poll(&mut p2_t);
    p3.poll(&mut p3_t);
    assert_eq!(p2.local_player_id(), Some(PlayerId(2)));
    assert!(
        p3.drain_events()
            .contains(&NodeEvent::JoinRejected(RejectReason::SessionFull {
                max_players: 2
            }))
    );

    // Verify P2's pending join did NOT mutate host's SessionState fingerprint before tick 1!
    let fresh_session = sims_net::SessionState::new(
        "Host",
        Some("mem:host".to_string()),
        SessionConfig {
            max_players: 2,
            tick_delay: 2,
            ..SessionConfig::default()
        },
    );
    assert_eq!(
        host.session().session_fingerprint(),
        fresh_session.session_fingerprint()
    );

    // P2 disconnects BEFORE tick 1 is scheduled -> queued PlayerJoined cancelled, zero ghost player!
    p2_t.disconnect(&host_addr).unwrap();
    host.poll(&mut host_t);
    host.advance_host_tick(&mut host_t).unwrap();
    host.drain_ready_ticks(true).unwrap();
    assert_eq!(host.session().players.len(), 1);
    assert!(!host.session().players.contains_key(&PlayerId(2)));

    // 2. Blocking dialog first-answer-wins follows host RECEIPT order (P3 before P2 -> P3 wins!).
    let mesh2 = MemoryMesh::new(909);
    let mut h2_t = mesh2.add_node("mem:host");
    let mut h2 = LockstepNode::new_host(
        "Host",
        Some("mem:host".to_string()),
        manifest,
        SessionConfig {
            max_players: 4,
            tick_delay: 2,
            ..SessionConfig::default()
        },
    );
    let mut a_t = mesh2.add_node("mem:a");
    let mut b_t = mesh2.add_node("mem:b");
    a_t.connect(&host_addr).unwrap();
    b_t.connect(&host_addr).unwrap();
    let mut a = LockstepNode::new_peer("A", Some("mem:a".to_string()), manifest, host_addr.clone());
    let mut b = LockstepNode::new_peer("B", Some("mem:b".to_string()), manifest, host_addr.clone());
    for _ in 0..4 {
        a.poll(&mut a_t);
        b.poll(&mut b_t);
        h2.poll(&mut h2_t);
        mesh2.step();
    }
    let a_id = a.local_player_id().unwrap();
    let b_id = b.local_player_id().unwrap();
    let higher_is_b = b_id > a_id;
    assert!(higher_is_b);

    // Tick 1 joins A and B; tick 2 opens dialog.
    h2.advance_host_tick(&mut h2_t).unwrap();
    h2.drain_ready_ticks(true).unwrap();
    h2.session_mut()
        .open_blocking_dialog(DialogId(5), 10, Tick(1));

    // Higher PlayerId (B = P3) sends answer BEFORE lower PlayerId (A = P2) in the same tick!
    b.submit_command(
        &mut b_t,
        Command::AnswerBlockingDialog {
            dialog_id: DialogId(5),
            answer: DialogAnswer {
                status: 3,
                bitmap_id: None,
                text: None,
            },
        },
    )
    .unwrap();
    mesh2.step();
    h2.poll(&mut h2_t);
    a.submit_command(
        &mut a_t,
        Command::AnswerBlockingDialog {
            dialog_id: DialogId(5),
            answer: DialogAnswer {
                status: 2,
                bitmap_id: None,
                text: None,
            },
        },
    )
    .unwrap();
    mesh2.step();
    h2.poll(&mut h2_t);

    h2.advance_host_tick(&mut h2_t).unwrap();
    let applied = h2.step_next_tick(true).unwrap().unwrap();
    assert!(matches!(
        applied.commands[0].outcome,
        CommandOutcome::DialogWon {
            dialog_id: DialogId(5),
            winner,
            ..
        } if winner == b_id
    ));
    assert_eq!(
        applied.commands[1].outcome,
        CommandOutcome::Ignored(CommandIgnoreReason::DialogAlreadyAnswered)
    );

    // 3. Failover candidate fallback: both Host (P1) AND primary successor (A = P2) crash
    //    before B (P3) receives PlayerLeft(P2) -> B skips unreachable P2 and promotes itself!
    mesh2.step();
    b.poll(&mut b_t);
    b.drain_ready_ticks(true).unwrap();
    assert_eq!(b.session().players.len(), 3);

    mesh2.crash_node(&PeerAddress::new("mem:a"));
    mesh2.crash_node(&host_addr);
    b.poll(&mut b_t);
    let b_events = b.drain_events();
    assert!(
        b_events.contains(&NodeEvent::HostPromoted {
            new_host: b_id,
            new_epoch: Epoch(2),
        }),
        "expected B to skip crashed P2 and promote itself: {b_events:?}"
    );
    assert_eq!(b.role(), PlayerRole::Host);
}

#[test]
fn round2_host_without_hint_delay_cap_split_brain_and_peer_ack_backpressure() {
    // 1. Host without endpoint_hint produces identical SessionState fingerprint on joiner.
    let mesh = MemoryMesh::new(1111);
    let manifest = sample_manifest(b"nohint-v1");
    let host_addr = PeerAddress::new("mem:host");
    let mut host_t = mesh.add_node("mem:host");
    let mut peer_t = mesh.add_node("mem:peer");
    peer_t.connect(&host_addr).unwrap();
    let mut host = LockstepNode::new_host("Host", None, manifest, SessionConfig::default());
    let mut peer = LockstepNode::new_peer("Peer", None, manifest, host_addr.clone());
    for _ in 0..4 {
        peer.poll(&mut peer_t);
        host.poll(&mut host_t);
        mesh.step();
    }
    assert_eq!(
        host.session().session_fingerprint(),
        peer.session().session_fingerprint()
    );

    // 2. SessionConfig rejects tick_delay exceeding peer receive buffer capacity.
    let too_deep = SessionConfig {
        tick_delay: 129,
        max_tick_delay: 129,
        max_ticks_ahead_of_slowest: 129,
        retained_batch_history: 64,
        ..SessionConfig::default()
    };
    assert!(too_deep.validate().is_err());
    assert!(too_deep.sanitized().validate().is_ok());

    // 3. Split-brain resolution: if P2 and P3 both promote to Host in Epoch(2),
    //    P3 deterministically demotes to Peer when P2's HostTakeover arrives!
    let mesh_sb = MemoryMesh::new(2222);
    let mut p2_t = mesh_sb.add_node("mem:p2");
    let mut p3_t = mesh_sb.add_node("mem:p3");
    let p2_addr = PeerAddress::new("mem:p2");
    let p3_addr = PeerAddress::new("mem:p3");
    p2_t.connect(&p3_addr).unwrap();

    // Construct two hosts in Epoch 2 (simulating asymmetric failover split-brain).
    let mut h_p2 = LockstepNode::new_host(
        "P2",
        Some("mem:p2".to_string()),
        manifest,
        SessionConfig::default(),
    );
    let mut h_p3 = LockstepNode::new_host(
        "P3",
        Some("mem:p3".to_string()),
        manifest,
        SessionConfig::default(),
    );
    h_p2.session_mut().epoch = Epoch(2);
    h_p2.session_mut().host_id = PlayerId(2);
    h_p2.session_mut().players.clear();
    h_p2.session_mut().players.insert(
        PlayerId(2),
        sims_net::PerPlayerState {
            player_id: PlayerId(2),
            name: "P2".to_string(),
            role: PlayerRole::Host,
            mode: PlayerMode::Live,
            selected_sim: None,
            camera_rotation: CameraRotation::NORTH,
            endpoint_hint: Some("mem:p2".to_string()),
            transport_addr: Some("mem:p2".to_string()),
        },
    );
    h_p3.session_mut().epoch = Epoch(2);
    h_p3.session_mut().host_id = PlayerId(3);

    p2_t.send(
        &p3_addr,
        WireMessage::HostTakeover {
            new_epoch: Epoch(2),
            new_host: PlayerId(2),
            resume_tick: Tick::ZERO,
            resume_fingerprint: None,
            session_state: Box::new(h_p2.session().clone()),
            roster: Vec::new(),
        },
    )
    .unwrap();
    mesh_sb.step();
    h_p3.poll(&mut p3_t);
    assert_eq!(h_p3.role(), PlayerRole::Peer);
    assert_eq!(h_p3.session().host_id, PlayerId(2));
    assert!(h_p3.is_syncing());
    let _ = p2_addr;
}

/// A joiner from a build before protocol 2 (`Saved` format 11 and the
/// indicator refresh queues in the co-op fingerprint) says protocol 1. The
/// host turns it away before the snapshot, with a message that names both
/// versions.
#[test]
fn a_hello_from_protocol_1_is_rejected_with_a_clear_message() {
    assert_eq!(sims_net::PROTOCOL_VERSION, 2);
    let mesh = MemoryMesh::new(407);
    let manifest = sample_manifest(b"protocol-v2");
    let host_addr = PeerAddress::new("mem:host");
    let mut host_t = mesh.add_node("mem:host");
    let mut old_t = mesh.add_node("mem:old");
    old_t.connect(&host_addr).unwrap();
    let mut host = LockstepNode::new_host(
        "Host",
        Some("mem:host".to_string()),
        manifest,
        SessionConfig::default(),
    );
    old_t
        .send(
            &host_addr,
            WireMessage::Hello {
                protocol_version: 1,
                player_name: "Old build".to_string(),
                manifest,
                endpoint_hint: Some("mem:old".to_string()),
            },
        )
        .unwrap();
    mesh.step();
    host.poll(&mut host_t);
    mesh.step();

    let replies: Vec<_> = old_t
        .drain_events()
        .into_iter()
        .filter_map(|event| match event {
            sims_net::TransportEvent::Message { message, .. } => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(
        replies,
        [WireMessage::Reject {
            reason: RejectReason::ProtocolVersionMismatch {
                expected: 2,
                actual: 1
            }
        }],
        "the host sends the reject and nothing else"
    );
    assert_eq!(host.session().players.len(), 1, "nobody joined");
    assert_eq!(
        sims_net::protocol_mismatch_message(2, 1),
        "Join rejected: the host runs a different sims-rs build (co-op protocol v2, \
         this build v1). Both players need the same version."
    );
}
