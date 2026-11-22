//! Real `iroh` P2P loopback integration tests for `sims-net`.
//!
//! Uses `IrohConfig::Loopback` (`presets::Minimal` on `127.0.0.1:0`), so these
//! tests exercise real QUIC/TLS streams, ALPN negotiation, invite tickets,
//! length-prefixed postcard framing, lockstep command broadcast, and mid-game
//! REDB snapshot streaming without requiring external internet or relay access.

use sims_net::manifest::sha256_bytes;
use sims_net::{
    BuyBuildEdit, CameraRotation, Command, InviteTicket, IrohConfig, IrohTransport, LockstepNode,
    ManifestDigest, NodeEvent, PlayerId, ResyncReason, SessionConfig, SnapshotCompression, Tick,
    Transport,
};
use std::time::Duration;
use tokio::time::timeout;

async fn pump_until<F>(
    host_node: &mut LockstepNode,
    host_tr: &mut IrohTransport,
    peers: &mut [(&mut LockstepNode, &mut IrohTransport)],
    mut condition: F,
) where
    F: FnMut(&mut LockstepNode, &mut [(&mut LockstepNode, &mut IrohTransport)]) -> bool,
{
    timeout(Duration::from_secs(10), async {
        loop {
            host_node.poll(host_tr);
            for (node, tr) in peers.iter_mut() {
                node.poll(*tr);
            }
            if condition(host_node, peers) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("timed out waiting for iroh loopback condition");
}

#[tokio::test]
async fn iroh_invite_ticket_round_trip_and_loopback_lockstep_with_midgame_snapshot() {
    let manifest = ManifestDigest(sha256_bytes(b"complete-collection-loopback"));
    let config = SessionConfig {
        tick_delay: 1,
        fingerprint_interval: 5,
        max_players: 4,
        ..SessionConfig::default()
    };

    // 1. Bind host endpoint on loopback and export shareable InviteTicket.
    let mut host_tr = IrohTransport::bind(IrohConfig::Loopback)
        .await
        .expect("bind host iroh loopback");
    let ticket = host_tr.invite_ticket();
    let ticket_str = ticket.encode_string().expect("encode invite ticket");
    let decoded = InviteTicket::decode_string(&ticket_str).expect("decode invite ticket");
    assert_eq!(decoded, ticket);

    let mut host_node = LockstepNode::new_host("Host", Some(ticket_str.clone()), manifest, config);

    // 2. Peer 2 binds on loopback and connects via the invite ticket.
    let mut p2_tr = IrohTransport::bind(IrohConfig::Loopback)
        .await
        .expect("bind peer 2 iroh loopback");
    let host_peer_addr = p2_tr
        .connect_ticket(&decoded)
        .await
        .expect("peer 2 connect to host");
    let p2_ticket = p2_tr.invite_ticket().encode_string().unwrap();
    let mut p2_node =
        LockstepNode::new_peer("Peer2", Some(p2_ticket), manifest, host_peer_addr.clone());

    // Wait for Peer 2 handshake to complete over real iroh QUIC stream.
    pump_until(
        &mut host_node,
        &mut host_tr,
        &mut [(&mut p2_node, &mut p2_tr)],
        |_, peers| peers[0].0.local_player_id() == Some(PlayerId(2)),
    )
    .await;

    // 3. Exchange commands and run 10 lockstep ticks over real QUIC streams.
    p2_node
        .submit_command(
            &mut p2_tr,
            Command::SetCameraRotation {
                rotation: CameraRotation::EAST,
            },
        )
        .unwrap();
    host_node
        .submit_command(
            &mut host_tr,
            Command::BuyBuild(BuyBuildEdit::BuyObject {
                guid: 0xABCD,
                position: [10, 12, 0],
                facing: 1,
                container: None,
            }),
        )
        .unwrap();

    for target_tick in 1..=10u64 {
        if target_tick == 1 {
            // Allow peer 2's SubmitCommand to reach host before scheduling tick 1.
            tokio::time::sleep(Duration::from_millis(20)).await;
            host_node.poll(&mut host_tr);
        }
        host_node.advance_host_tick(&mut host_tr).unwrap();
        let host_batches = host_node.drain_ready_ticks(true).unwrap();
        for batch in &host_batches {
            let hash = sha256_bytes(&batch.tick.0.to_le_bytes());
            host_node
                .report_local_tick(&mut host_tr, batch.tick, 1_500, Some(hash))
                .unwrap();
        }

        pump_until(
            &mut host_node,
            &mut host_tr,
            &mut [(&mut p2_node, &mut p2_tr)],
            |_, peers| {
                let _ = peers[0].0.drain_ready_ticks(true);
                peers[0].0.session().current_tick == Tick(target_tick)
            },
        )
        .await;
        let hash = sha256_bytes(&target_tick.to_le_bytes());
        p2_node
            .report_local_tick(&mut p2_tr, Tick(target_tick), 2_000, Some(hash))
            .unwrap();
    }

    assert_eq!(host_node.session(), p2_node.session());
    assert_eq!(
        p2_node.session().players[&PlayerId(2)].camera_rotation,
        CameraRotation::EAST
    );

    // 4. Peer 3 joins mid-game at tick 10 and receives a chunked/deflated REDB snapshot over iroh!
    let mut p3_tr = IrohTransport::bind(IrohConfig::Loopback)
        .await
        .expect("bind peer 3 iroh loopback");
    let host_addr_for_p3 = p3_tr
        .connect_ticket(&decoded)
        .await
        .expect("peer 3 connect to host");
    let p3_ticket = p3_tr.invite_ticket().encode_string().unwrap();
    let mut p3_node = LockstepNode::new_peer("Peer3", Some(p3_ticket), manifest, host_addr_for_p3);

    // Wait for host to emit SnapshotNeeded for Peer 3.
    let mut midgame_sid = None;
    timeout(Duration::from_secs(10), async {
        loop {
            p3_node.poll(&mut p3_tr);
            host_node.poll(&mut host_tr);
            for ev in host_node.drain_events() {
                if let NodeEvent::SnapshotNeeded {
                    snapshot_id,
                    reason: ResyncReason::MidGameJoin,
                    ..
                } = ev
                {
                    midgame_sid = Some(snapshot_id);
                }
            }
            if midgame_sid.is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("timed out waiting for mid-game SnapshotNeeded");

    let fake_redb = vec![0x5Au8; 128 * 1024]; // 128 KiB mock REDB snapshot
    let world_hash = sha256_bytes(&10u64.to_le_bytes());
    host_node
        .send_snapshot(
            &mut host_tr,
            midgame_sid.unwrap(),
            world_hash,
            fake_redb.clone(),
            SnapshotCompression::Deflate,
            16 * 1024,
        )
        .unwrap();

    // Advance tick 11 (which carries PlayerJoined for Peer 3).
    host_node.advance_host_tick(&mut host_tr).unwrap();
    host_node.drain_ready_ticks(true).unwrap();

    // Wait for Peer 3 to apply the snapshot AND execute tick 11, and Peer 2 to execute tick 11.
    let mut p3_applied_bytes = None;
    timeout(Duration::from_secs(10), async {
        loop {
            host_node.poll(&mut host_tr);
            p2_node.poll(&mut p2_tr);
            p3_node.poll(&mut p3_tr);
            for ev in p3_node.drain_events() {
                if let NodeEvent::ApplySnapshot { redb_bytes, .. } = ev {
                    p3_applied_bytes = Some(redb_bytes);
                }
            }
            let _ = p2_node.drain_ready_ticks(true);
            let _ = p3_node.drain_ready_ticks(true);
            if p3_applied_bytes.is_some()
                && p2_node.session().current_tick == Tick(11)
                && p3_node.session().current_tick == Tick(11)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("timed out waiting for iroh snapshot apply and tick 11 convergence");

    assert_eq!(p3_applied_bytes.as_deref(), Some(fake_redb.as_slice()));
    assert_eq!(host_node.session().players.len(), 3);
    assert_eq!(p2_node.session(), host_node.session());
    assert_eq!(p3_node.session(), host_node.session());

    // Graceful shutdown.
    assert!(!host_tr.connected_peers().is_empty());
    p2_tr.close().await;
    p3_tr.close().await;
    host_tr.close().await;
}

#[tokio::test]
async fn iroh_loopback_three_player_host_failover_and_manifest_rejection() {
    let manifest = ManifestDigest(sha256_bytes(b"loopback-failover-v1"));
    let bad_manifest = ManifestDigest(sha256_bytes(b"different-custom-content"));
    let config = SessionConfig {
        tick_delay: 1,
        max_players: 4,
        ..SessionConfig::default()
    };

    let mut p1_tr = IrohTransport::bind(IrohConfig::Loopback).await.unwrap();
    let p1_ticket = p1_tr.invite_ticket();
    let mut p1_node = LockstepNode::new_host(
        "P1",
        Some(p1_ticket.encode_string().unwrap()),
        manifest,
        config,
    );

    // 1. Verify mismatched manifest is rejected over real iroh QUIC.
    let mut bad_tr = IrohTransport::bind(IrohConfig::Loopback).await.unwrap();
    let bad_host_addr = bad_tr.connect_ticket(&p1_ticket).await.unwrap();
    let mut bad_node = LockstepNode::new_peer("Bad", None, bad_manifest, bad_host_addr);
    let mut rejected = false;
    timeout(Duration::from_secs(10), async {
        loop {
            p1_node.poll(&mut p1_tr);
            bad_node.poll(&mut bad_tr);
            if bad_node.drain_events().iter().any(|e| {
                matches!(
                    e,
                    NodeEvent::JoinRejected(sims_net::RejectReason::ManifestMismatch { .. })
                )
            }) {
                rejected = true;
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("timed out waiting for manifest rejection");
    assert!(rejected);
    bad_tr.close().await;

    // 2. Join P2 and P3 with valid manifest and their own loopback InviteTickets as endpoint hints.
    let mut p2_tr = IrohTransport::bind(IrohConfig::Loopback).await.unwrap();
    let mut p3_tr = IrohTransport::bind(IrohConfig::Loopback).await.unwrap();
    let p2_host_addr = p2_tr.connect_ticket(&p1_ticket).await.unwrap();
    let p3_host_addr = p3_tr.connect_ticket(&p1_ticket).await.unwrap();

    let mut p2_node = LockstepNode::new_peer(
        "P2",
        Some(p2_tr.invite_ticket().encode_string().unwrap()),
        manifest,
        p2_host_addr,
    );
    let mut p3_node = LockstepNode::new_peer(
        "P3",
        Some(p3_tr.invite_ticket().encode_string().unwrap()),
        manifest,
        p3_host_addr,
    );

    pump_until(
        &mut p1_node,
        &mut p1_tr,
        &mut [(&mut p2_node, &mut p2_tr), (&mut p3_node, &mut p3_tr)],
        |_, peers| peers[0].0.local_player_id().is_some() && peers[1].0.local_player_id().is_some(),
    )
    .await;

    // Advance ticks 1..=3 so all three nodes have full roster & endpoint hints at tick 3.
    for target in 1..=3u64 {
        p1_node.advance_host_tick(&mut p1_tr).unwrap();
        p1_node.drain_ready_ticks(true).unwrap();
        pump_until(
            &mut p1_node,
            &mut p1_tr,
            &mut [(&mut p2_node, &mut p2_tr), (&mut p3_node, &mut p3_tr)],
            |_, peers| {
                let _ = peers[0].0.drain_ready_ticks(true);
                let _ = peers[1].0.drain_ready_ticks(true);
                peers[0].0.session().current_tick == Tick(target)
                    && peers[1].0.session().current_tick == Tick(target)
            },
        )
        .await;
    }

    // Determine which peer got PlayerId(2) vs PlayerId(3) (iroh accepts concurrently).
    let (mut new_host_node, mut new_host_tr, mut survivor_node, mut survivor_tr) =
        if p2_node.local_player_id() == Some(PlayerId(2)) {
            (p2_node, p2_tr, p3_node, p3_tr)
        } else {
            (p3_node, p3_tr, p2_node, p2_tr)
        };

    // 3. Original host P1 leaves and closes its iroh endpoint!
    p1_node.leave_gracefully(&mut p1_tr).unwrap();
    p1_tr.close().await;

    // Wait for PlayerId(2) to promote to Host at Epoch(2) and PlayerId(3) to reconnect via InviteTicket!
    let mut promoted = false;
    let mut adopted = false;
    timeout(Duration::from_secs(10), async {
        loop {
            new_host_node.poll(&mut new_host_tr);
            survivor_node.poll(&mut survivor_tr);
            for ev in new_host_node.drain_events() {
                if matches!(
                    ev,
                    NodeEvent::HostPromoted {
                        new_host: PlayerId(2),
                        new_epoch: sims_net::Epoch(2),
                    }
                ) {
                    promoted = true;
                }
            }
            for ev in survivor_node.drain_events() {
                if matches!(
                    ev,
                    NodeEvent::HostChanged {
                        new_host: PlayerId(2),
                        new_epoch: sims_net::Epoch(2),
                    }
                ) {
                    adopted = true;
                }
            }
            if promoted && adopted {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("timed out waiting for iroh host failover");

    // 4. Advance tick 4 under the newly promoted iroh Host!
    new_host_node.advance_host_tick(&mut new_host_tr).unwrap();
    new_host_node.drain_ready_ticks(true).unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            new_host_node.poll(&mut new_host_tr);
            survivor_node.poll(&mut survivor_tr);
            let _ = survivor_node.drain_ready_ticks(true);
            if survivor_node.session().current_tick == Tick(4) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("timed out waiting for post-failover tick 4 over iroh");

    assert_eq!(new_host_node.session(), survivor_node.session());
    assert_eq!(survivor_node.session().epoch, sims_net::Epoch(2));
    assert_eq!(survivor_node.session().host_id, PlayerId(2));

    survivor_tr.close().await;
    new_host_tr.close().await;
}
