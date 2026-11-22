//! Integration tests for crash-safe manual saves, rotating autosave rings,
//! corrupt-save recovery, shared-DB `NeighborhoodStore::autosaves()`, and
//! background worker thread commits.

use redb::{ReadableDatabase, ReadableTable};
use sims_sim::{
    AUTOSAVE_FORMAT, AutosavePolicy, AutosaveStore, DEFAULT_MAX_AUTOSAVES, NeighborhoodStore,
    SaveError, SaveJob, SavePayload, SavePayloadKind, SaveSlot, SaveSlotEntry, StoredNeighborhood,
    World,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempDb(PathBuf);

impl TempDb {
    fn new(label: &str) -> Self {
        let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sims-sim-autosave-{label}-{}-{n}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn assert_send_sync<T: Send + Sync>() {}
fn assert_send<T: Send>() {}

fn sample_neighborhood(name: &str) -> StoredNeighborhood {
    StoredNeighborhood {
        name: name.to_owned(),
        families: BTreeMap::new(),
        neighbors: Vec::new(),
        inventories: Vec::new(),
        data: [0; 16],
        options: None,
        type_attributes: None,
        zoning: BTreeMap::from([(5, 1)]),
        lot_rows: vec![5],
        made_characters: Default::default(),
        deleted_characters: BTreeSet::new(),
        deletion_epoch: 0,
        deleted_at_epoch: BTreeMap::new(),
        instance_id: 1,
        revision: 0,
    }
}

#[test]
fn types_are_thread_safe_for_background_save_workers() {
    assert_send_sync::<AutosaveStore>();
    assert_send::<SaveJob>();
    assert_send::<SavePayload>();
}

#[test]
fn manual_and_rotating_autosave_ring_enforces_retention_and_isolation() {
    assert_eq!(DEFAULT_MAX_AUTOSAVES, 5);
    assert!(matches!(
        AutosavePolicy::new(0),
        Err(SaveError::InvalidInput(_))
    ));

    let db = TempDb::new("ring");
    let store = AutosaveStore::open_with_policy(&db.0, AutosavePolicy::new(3).unwrap()).unwrap();

    // Empty neighborhood_id is rejected cleanly.
    assert!(matches!(
        store.save_autosave("", 5, 10, None, "bad", SavePayload::saved_lot(vec![1])),
        Err(SaveError::InvalidInput(_))
    ));
    assert!(matches!(
        store.load_manual("UserData", 5),
        Err(SaveError::Missing {
            slot: Some(SaveSlot::Manual),
            ..
        })
    ));

    // Write manual save + 5 rotating autosaves for ("UserData", 5).
    let manual_meta = store
        .save_manual(
            "UserData",
            5,
            100,
            Some(1_700_000_000),
            "Goth Manual",
            SavePayload::saved_lot(b"manual-lot-v1".to_vec()),
        )
        .unwrap();
    assert_eq!(manual_meta.slot, SaveSlot::Manual);
    assert_eq!(manual_meta.format_version, AUTOSAVE_FORMAT);
    assert_eq!(manual_meta.payload_kind, SavePayloadKind::SavedLot);

    for i in 1u64..=5 {
        let meta = store
            .save_autosave(
                "UserData",
                5,
                100 + i * 30,
                Some(1_700_000_000 + i * 60),
                &format!("Auto {i}"),
                SavePayload::saved_lot(format!("auto-payload-{i}").into_bytes()),
            )
            .unwrap();
        assert_eq!(meta.slot, SaveSlot::Autosave(i));
    }

    // Separate lot and separate neighborhood stay strictly isolated.
    store
        .save_autosave(
            "UserData",
            7,
            20,
            None,
            "Newbie Auto",
            SavePayload::saved_lot(b"lot-7".to_vec()),
        )
        .unwrap();
    store
        .save_manual(
            "UserData2",
            5,
            30,
            None,
            "NH2 Manual",
            SavePayload::custom("test-fmt", b"nh2-manual".to_vec()),
        )
        .unwrap();

    assert_eq!(store.list_lots_with_saves("UserData").unwrap(), vec![5, 7]);
    assert_eq!(store.list_lots_with_saves("UserData2").unwrap(), vec![5]);

    // Listing returns Manual first, then rotating autosaves newest-first (5, 4, 3).
    let entries = store.list_saves("UserData", 5).unwrap();
    let slots: Vec<SaveSlot> = entries.iter().map(SaveSlotEntry::slot).collect();
    assert_eq!(
        slots,
        vec![
            SaveSlot::Manual,
            SaveSlot::Autosave(5),
            SaveSlot::Autosave(4),
            SaveSlot::Autosave(3),
        ]
    );

    // Pruned slots 1 and 2 report Missing cleanly.
    assert!(matches!(
        store.load_save("UserData", 5, SaveSlot::Autosave(1)),
        Err(SaveError::Missing {
            slot: Some(SaveSlot::Autosave(1)),
            ..
        })
    ));

    // Manual slot was never touched by autosave rotation.
    let loaded_manual = store.load_manual("UserData", 5).unwrap();
    assert_eq!(loaded_manual.payload, b"manual-lot-v1");
    assert_eq!(loaded_manual.metadata.label, "Goth Manual");
    assert_eq!(
        loaded_manual.metadata.saved_at_unix_secs,
        Some(1_700_000_000)
    );

    // Deleting the newest autosave does NOT reset or reuse sequence numbers.
    assert!(
        store
            .delete_slot("UserData", 5, SaveSlot::Autosave(5))
            .unwrap()
    );
    assert!(
        !store
            .delete_slot("UserData", 5, SaveSlot::Autosave(5))
            .unwrap()
    );
    let next = store
        .save_autosave(
            "UserData",
            5,
            500,
            None,
            "Auto 6",
            SavePayload::saved_lot(b"auto-payload-6".to_vec()),
        )
        .unwrap();
    assert_eq!(next.slot, SaveSlot::Autosave(6));
}

#[test]
fn corrupt_saves_return_typed_errors_and_never_delete_or_replace() {
    let db = TempDb::new("corrupt-errors");
    let store = AutosaveStore::open_with_policy(&db.0, AutosavePolicy::new(6).unwrap()).unwrap();

    store
        .save_manual(
            "UserData",
            5,
            50,
            Some(100),
            "Manual",
            SavePayload::saved_lot(b"manual-good".to_vec()),
        )
        .unwrap();
    for seq in 1..=5 {
        store
            .save_autosave(
                "UserData",
                5,
                50 + seq,
                None,
                &format!("slot {seq}"),
                SavePayload::saved_lot(format!("payload-{seq}").into_bytes()),
            )
            .unwrap();
    }
    store
        .save_autosave(
            "UserData",
            7,
            99,
            None,
            "Other lot",
            SavePayload::saved_lot(b"other-lot".to_vec()),
        )
        .unwrap();
    drop(store);

    // Corrupt specific raw REDB entries to exercise every error variant:
    // - Manual: bit-flip in payload -> CorruptChecksum
    // - Autosave(1): truncated header (< 8 bytes) -> DecodeFailed
    // - Autosave(2): bad magic -> DecodeFailed
    // - Autosave(3): future format version -> UnsupportedVersion
    // - Autosave(4): valid bytes copied from lot 7 -> DecodeFailed (key mismatch)
    // - Autosave(5): truncated payload body -> CorruptChecksum
    const MANUAL_TABLE: redb::TableDefinition<(&str, u32), &[u8]> =
        redb::TableDefinition::new("autosave_manual");
    const RING_TABLE: redb::TableDefinition<(&str, u32, u64), &[u8]> =
        redb::TableDefinition::new("autosave_ring");

    let (corrupt_manual_bytes, corrupt_v99_bytes);
    {
        let raw_db = redb::Database::open(&db.0).unwrap();
        let w = raw_db.begin_write().unwrap();
        {
            let mut m = w.open_table(MANUAL_TABLE).unwrap();
            let mut mbytes = m.get(("UserData", 5)).unwrap().unwrap().value().to_vec();
            let last = mbytes.len() - 1;
            mbytes[last] ^= 0xFF;
            m.insert(("UserData", 5), mbytes.as_slice()).unwrap();
            corrupt_manual_bytes = mbytes;

            let mut r = w.open_table(RING_TABLE).unwrap();
            let other_lot_bytes = r.get(("UserData", 7, 1)).unwrap().unwrap().value().to_vec();
            let mut s3 = r.get(("UserData", 5, 3)).unwrap().unwrap().value().to_vec();
            s3[4..8].copy_from_slice(&(AUTOSAVE_FORMAT + 7).to_le_bytes());
            corrupt_v99_bytes = s3.clone();

            let mut s5 = r.get(("UserData", 5, 5)).unwrap().unwrap().value().to_vec();
            s5.truncate(s5.len() - 3);

            r.insert(("UserData", 5, 1), &[1u8, 2, 3][..]).unwrap();
            r.insert(("UserData", 5, 2), b"BAD!12345678".as_slice())
                .unwrap();
            r.insert(("UserData", 5, 3), s3.as_slice()).unwrap();
            r.insert(("UserData", 5, 4), other_lot_bytes.as_slice())
                .unwrap();
            r.insert(("UserData", 5, 5), s5.as_slice()).unwrap();
        }
        w.commit().unwrap();
    }

    let store = AutosaveStore::open_with_policy(&db.0, AutosavePolicy::new(6).unwrap()).unwrap();

    assert!(matches!(
        store.load_manual("UserData", 5),
        Err(SaveError::CorruptChecksum {
            slot: SaveSlot::Manual,
            ..
        })
    ));
    assert!(matches!(
        store.load_save("UserData", 5, SaveSlot::Autosave(1)),
        Err(SaveError::DecodeFailed {
            slot: SaveSlot::Autosave(1),
            ..
        })
    ));
    assert!(matches!(
        store.load_save("UserData", 5, SaveSlot::Autosave(2)),
        Err(SaveError::DecodeFailed {
            slot: SaveSlot::Autosave(2),
            ..
        })
    ));
    assert!(matches!(
        store.load_save("UserData", 5, SaveSlot::Autosave(3)),
        Err(SaveError::UnsupportedVersion {
            slot: SaveSlot::Autosave(3),
            found,
            ..
        }) if found == AUTOSAVE_FORMAT + 7
    ));
    assert!(matches!(
        store.load_save("UserData", 5, SaveSlot::Autosave(4)),
        Err(SaveError::DecodeFailed {
            slot: SaveSlot::Autosave(4),
            ..
        })
    ));
    assert!(matches!(
        store.load_save("UserData", 5, SaveSlot::Autosave(5)),
        Err(SaveError::CorruptChecksum {
            slot: SaveSlot::Autosave(5),
            ..
        })
    ));

    // `save_manual` refuses to silently replace the corrupt manual save!
    assert!(matches!(
        store.save_manual(
            "UserData",
            5,
            200,
            None,
            "Overwrite Attempt",
            SavePayload::saved_lot(b"new".to_vec()),
        ),
        Err(SaveError::CorruptChecksum {
            slot: SaveSlot::Manual,
            ..
        })
    ));

    // All corrupt entries remain intact in `list_saves` and on disk.
    let listed = store.list_saves("UserData", 5).unwrap();
    assert_eq!(listed.len(), 6);
    assert!(
        listed
            .iter()
            .all(|e| matches!(e, SaveSlotEntry::Corrupt { .. }))
    );

    drop(store);
    {
        let raw_db = redb::Database::open(&db.0).unwrap();
        let r = raw_db.begin_read().unwrap();
        let m = r.open_table(MANUAL_TABLE).unwrap();
        assert_eq!(
            m.get(("UserData", 5)).unwrap().unwrap().value(),
            corrupt_manual_bytes.as_slice()
        );
        let ring = r.open_table(RING_TABLE).unwrap();
        assert_eq!(
            ring.get(("UserData", 5, 3)).unwrap().unwrap().value(),
            corrupt_v99_bytes.as_slice()
        );
    }

    // Explicit `save_manual_force_replace` succeeds when the user confirms overwrite.
    let store = AutosaveStore::open_with_policy(&db.0, AutosavePolicy::new(6).unwrap()).unwrap();
    store
        .save_manual_force_replace(
            "UserData",
            5,
            201,
            None,
            "Recovered Manual",
            SavePayload::saved_lot(b"manual-replaced".to_vec()),
        )
        .unwrap();
    assert_eq!(
        store.load_manual("UserData", 5).unwrap().payload,
        b"manual-replaced"
    );
}

#[test]
fn pruning_preserves_corrupt_slots_and_fallback_recovers_newest_intact() {
    let db = TempDb::new("recovery-and-pruning");
    let store = AutosaveStore::open_with_policy(&db.0, AutosavePolicy::new(3).unwrap()).unwrap();

    store
        .save_manual(
            "UserData",
            5,
            10,
            None,
            "Manual Backup",
            SavePayload::saved_lot(b"manual-backup".to_vec()),
        )
        .unwrap();
    for seq in 1..=3 {
        store
            .save_autosave(
                "UserData",
                5,
                10 + seq,
                None,
                &format!("Auto {seq}"),
                SavePayload::saved_lot(format!("auto-{seq}").into_bytes()),
            )
            .unwrap();
    }
    drop(store);

    // Corrupt slot 2 in place.
    const RING_TABLE: redb::TableDefinition<(&str, u32, u64), &[u8]> =
        redb::TableDefinition::new("autosave_ring");
    {
        let raw_db = redb::Database::open(&db.0).unwrap();
        let w = raw_db.begin_write().unwrap();
        {
            let mut r = w.open_table(RING_TABLE).unwrap();
            let mut s2 = r.get(("UserData", 5, 2)).unwrap().unwrap().value().to_vec();
            s2[40] ^= 0x55;
            r.insert(("UserData", 5, 2), s2.as_slice()).unwrap();
        }
        w.commit().unwrap();
    }

    let store = AutosaveStore::open_with_policy(&db.0, AutosavePolicy::new(3).unwrap()).unwrap();

    // Writing slot 4 yields 3 intact slots [1, 3, 4] (<= max_autosaves 3), so
    // NEITHER intact slot 1 NOR corrupt slot 2 is pruned.
    let m4 = store
        .save_autosave(
            "UserData",
            5,
            14,
            None,
            "Auto 4",
            SavePayload::saved_lot(b"auto-4".to_vec()),
        )
        .unwrap();
    assert_eq!(m4.slot, SaveSlot::Autosave(4));
    let slots_after_4: Vec<SaveSlot> = store
        .list_saves("UserData", 5)
        .unwrap()
        .iter()
        .map(SaveSlotEntry::slot)
        .collect();
    assert_eq!(
        slots_after_4,
        vec![
            SaveSlot::Manual,
            SaveSlot::Autosave(4),
            SaveSlot::Autosave(3),
            SaveSlot::Autosave(2),
            SaveSlot::Autosave(1),
        ]
    );

    // Writing slot 5 yields 4 intact slots [1, 3, 4, 5] -> oldest intact slot 1
    // is pruned atomically, leaving intact [3, 4, 5] + untouched corrupt slot 2.
    let m5 = store
        .save_autosave(
            "UserData",
            5,
            15,
            None,
            "Auto 5",
            SavePayload::saved_lot(b"auto-5".to_vec()),
        )
        .unwrap();
    assert_eq!(m5.slot, SaveSlot::Autosave(5));
    let slots_after_5: Vec<SaveSlot> = store
        .list_saves("UserData", 5)
        .unwrap()
        .iter()
        .map(SaveSlotEntry::slot)
        .collect();
    assert_eq!(
        slots_after_5,
        vec![
            SaveSlot::Manual,
            SaveSlot::Autosave(5),
            SaveSlot::Autosave(4),
            SaveSlot::Autosave(3),
            SaveSlot::Autosave(2),
        ]
    );

    // Now corrupt slot 5 (checksum) and slot 4 (future version) and Manual (bad magic)
    // and verify fallback recovery lands on slot 3 without deleting any corrupt slot.
    drop(store);
    {
        let raw_db = redb::Database::open(&db.0).unwrap();
        let w = raw_db.begin_write().unwrap();
        {
            const MANUAL_TABLE: redb::TableDefinition<(&str, u32), &[u8]> =
                redb::TableDefinition::new("autosave_manual");
            let mut m = w.open_table(MANUAL_TABLE).unwrap();
            m.insert(("UserData", 5), b"NOPE1234".as_slice()).unwrap();

            let mut r = w.open_table(RING_TABLE).unwrap();
            let mut s5 = r.get(("UserData", 5, 5)).unwrap().unwrap().value().to_vec();
            let last = s5.len() - 1;
            s5[last] ^= 0x01;
            r.insert(("UserData", 5, 5), s5.as_slice()).unwrap();

            let mut s4 = r.get(("UserData", 5, 4)).unwrap().unwrap().value().to_vec();
            s4[4..8].copy_from_slice(&42u32.to_le_bytes());
            r.insert(("UserData", 5, 4), s4.as_slice()).unwrap();
        }
        w.commit().unwrap();
    }

    let store = AutosaveStore::open_with_policy(&db.0, AutosavePolicy::new(3).unwrap()).unwrap();
    let recovered = store.load_latest_intact_autosave("UserData", 5).unwrap();
    assert_eq!(recovered.loaded.metadata.slot, SaveSlot::Autosave(3));
    assert_eq!(recovered.loaded.payload, b"auto-3");
    assert_eq!(recovered.skipped_corrupt.len(), 2);
    assert_eq!(recovered.skipped_corrupt[0].0, SaveSlot::Autosave(5));
    assert!(matches!(
        recovered.skipped_corrupt[0].1,
        SaveError::CorruptChecksum { .. }
    ));
    assert_eq!(recovered.skipped_corrupt[1].0, SaveSlot::Autosave(4));
    assert!(matches!(
        recovered.skipped_corrupt[1].1,
        SaveError::UnsupportedVersion { found: 42, .. }
    ));

    // `load_with_fallback(..., Manual)` skips corrupt Manual + Autosave(5) +
    // Autosave(4) and recovers Autosave(3), leaving all 4 corrupt entries intact.
    let manual_fallback = store
        .load_with_fallback("UserData", 5, SaveSlot::Manual)
        .unwrap();
    assert_eq!(manual_fallback.loaded.metadata.slot, SaveSlot::Autosave(3));
    assert_eq!(manual_fallback.skipped_corrupt.len(), 3);
    assert_eq!(manual_fallback.skipped_corrupt[0].0, SaveSlot::Manual);
    assert_eq!(store.list_saves("UserData", 5).unwrap().len(), 5);
}

#[test]
fn shared_neighborhood_store_and_background_worker_cow_world_commits() {
    let db = TempDb::new("shared-bg-worker");
    let nh_store = NeighborhoodStore::open(&db.0).unwrap();
    nh_store
        .save_neighborhood("UserData", &sample_neighborhood("UserData"))
        .unwrap();
    nh_store
        .save_neighborhood("UserData2", &sample_neighborhood("UserData2"))
        .unwrap();
    let ud1_instance = nh_store
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap()
        .instance_id;
    let ud2_instance = nh_store
        .load_neighborhood("UserData2")
        .unwrap()
        .unwrap()
        .instance_id;

    // Share the exact same REDB database handle with a background worker thread.
    let worker_autosaves = nh_store
        .autosaves_with_policy(AutosavePolicy::new(3).unwrap())
        .unwrap();

    // Shared handles reject jobs that omit `expected_instance_id` or pass `0`.
    assert!(matches!(
        worker_autosaves.commit_job(SaveJob::autosave(
            "UserData",
            5,
            1,
            SavePayload::saved_lot(b"unfenced".to_vec()),
        )),
        Err(SaveError::InvalidInput(_))
    ));
    assert!(matches!(
        worker_autosaves.commit_job(SaveJob::autosave_for_instance(
            "UserData",
            0,
            5,
            1,
            SavePayload::saved_lot(b"zero-id".to_vec()),
        )),
        Err(SaveError::InvalidInput(_))
    ));

    let (tx, rx) = mpsc::channel::<SaveJob>();
    let worker = std::thread::spawn(move || {
        let mut results = Vec::new();
        while let Ok(job) = rx.recv() {
            results.push(worker_autosaves.commit_job(job));
        }
        results
    });

    // Main thread clones a CoW `World` snapshot, mutates live state immediately,
    // and hands the owned `SaveJob` off to the background thread while also
    // exercising `NeighborhoodStore` reads/writes concurrently.
    let mut world = World::empty();
    let mut expected_hashes = Vec::new();
    for tick in 1u32..=4 {
        world.ticks = tick * 100;
        world.budget.funds = 1_000 * tick as i32;
        expected_hashes.push(world.hash());

        let snapshot = world.clone();
        tx.send(
            SaveJob::autosave_for_instance(
                "UserData",
                ud1_instance,
                5,
                u64::from(world.ticks),
                SavePayload::from_world(snapshot),
            )
            .with_label(format!("Tick {}", world.ticks))
            .with_saved_at_unix_secs(1_700_000_000 + u64::from(tick)),
        )
        .unwrap();

        // Immediately mutate the live `World` on the main thread to prove CoW isolation.
        world.budget.funds = -999;
        assert!(nh_store.load_neighborhood("UserData").unwrap().is_some());
    }

    // Also commit a manual save for UserData2 on the same worker.
    world.ticks = 777;
    world.budget.funds = 25_000;
    let nh2_hash = world.hash();
    tx.send(
        SaveJob::manual_for_instance(
            "UserData2",
            ud2_instance,
            5,
            u64::from(world.ticks),
            SavePayload::from_world(world.clone()),
        )
        .with_label("NH2 World"),
    )
    .unwrap();
    drop(tx);

    let commit_results = worker.join().unwrap();
    assert_eq!(commit_results.len(), 5);
    for res in &commit_results {
        assert!(res.is_ok(), "worker commit failed: {res:?}");
    }

    // Verify all retained snapshots round-trip to the exact pre-mutation World hash.
    let autosaves = nh_store.autosaves();
    let listed = autosaves.list_saves("UserData", 5).unwrap();
    assert_eq!(listed.len(), 3);
    for (idx, seq) in [(3usize, 4u64), (2, 3), (1, 2)] {
        let loaded = autosaves
            .load_save("UserData", 5, SaveSlot::Autosave(seq))
            .unwrap();
        let decoded = loaded.decode_world().unwrap();
        assert_eq!(decoded.hash(), expected_hashes[idx]);
        assert_eq!(decoded.budget.funds, 1_000 * (idx as i32 + 1));
    }

    let nh2_loaded = autosaves.load_manual("UserData2", 5).unwrap();
    assert_eq!(nh2_loaded.decode_world().unwrap().hash(), nh2_hash);

    // Deleting neighborhood "UserData" atomically cleans up its autosaves
    // without touching "UserData2"'s saves, AND a late queued job carrying
    // `ud1_instance` fails cleanly without recreating orphaned saves.
    assert!(nh_store.delete_neighborhood("UserData").unwrap());
    assert!(autosaves.list_saves("UserData", 5).unwrap().is_empty());
    assert_eq!(autosaves.list_saves("UserData2", 5).unwrap().len(), 1);

    let late_after_delete = autosaves.commit_job(SaveJob::autosave_for_instance(
        "UserData",
        ud1_instance,
        5,
        999,
        SavePayload::from_world(world.clone()),
    ));
    assert!(matches!(
        late_after_delete,
        Err(SaveError::StaleNeighborhoodInstance {
            expected_instance_id,
            current_instance_id: None,
            ..
        }) if expected_instance_id == ud1_instance
    ));
    assert!(autosaves.list_saves("UserData", 5).unwrap().is_empty());

    // Recreating "UserData" mints a new instance_id; a late manual or autosave
    // job from the old instance cannot overwrite the replacement's manual save!
    nh_store
        .save_neighborhood("UserData", &sample_neighborhood("UserData"))
        .unwrap();
    let replacement_instance = nh_store
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap()
        .instance_id;
    assert_ne!(replacement_instance, ud1_instance);
    autosaves
        .commit_job(SaveJob::manual_for_instance(
            "UserData",
            replacement_instance,
            5,
            10,
            SavePayload::saved_lot(b"replacement-manual".to_vec()),
        ))
        .unwrap();

    let late_after_replace = autosaves.commit_job(SaveJob::manual_force_replace_for_instance(
        "UserData",
        ud1_instance,
        5,
        999,
        SavePayload::saved_lot(b"stale-overwrite".to_vec()),
    ));
    assert!(matches!(
        late_after_replace,
        Err(SaveError::StaleNeighborhoodInstance {
            expected_instance_id,
            current_instance_id: Some(current),
            ..
        }) if expected_instance_id == ud1_instance && current == replacement_instance
    ));
    assert_eq!(
        autosaves.load_manual("UserData", 5).unwrap().payload,
        b"replacement-manual"
    );
}

#[test]
fn validated_commits_pruning_and_fallback_reject_malformed_payloads() {
    let db = TempDb::new("validated-payloads");
    let store = AutosaveStore::open_with_policy(&db.0, AutosavePolicy::new(1).unwrap()).unwrap();

    let lot_validator = |kind: &SavePayloadKind, bytes: &[u8]| -> Result<(), String> {
        if *kind == SavePayloadKind::SavedLot && !bytes.starts_with(b"LOT!") {
            return Err("missing LOT! payload magic".into());
        }
        Ok(())
    };

    // 1. Write one valid SavedLot slot with max_autosaves = 1.
    let s1 = store
        .commit_job_validated(
            SaveJob::autosave(
                "UserData",
                5,
                10,
                SavePayload::saved_lot(b"LOT!good-1".to_vec()),
            ),
            lot_validator,
        )
        .unwrap();
    assert_eq!(s1.slot, SaveSlot::Autosave(1));

    // 2. Attempting to write a malformed SavedLot via `commit_job_validated`
    //    is rejected BEFORE touching REDB, so slot 1 is never evicted!
    assert!(matches!(
        store.commit_job_validated(
            SaveJob::autosave(
                "UserData",
                5,
                20,
                SavePayload::saved_lot(b"BAD-LOT".to_vec())
            ),
            lot_validator,
        ),
        Err(SaveError::DecodeFailed { .. })
    ));
    assert_eq!(
        store
            .load_latest_intact_autosave_validated("UserData", 5, lot_validator)
            .unwrap()
            .loaded
            .payload,
        b"LOT!good-1"
    );

    // 3. Even if an unusable SavedLot was written into a newer slot (say by a
    //    store opened without a validator), validated fallback skips it without
    //    deleting it and returns the usable predecessor!
    let store_keep_2 = store.with_policy(AutosavePolicy::new(2).unwrap()).unwrap();
    let s2 = store_keep_2
        .save_autosave(
            "UserData",
            5,
            30,
            None,
            "Unusable",
            SavePayload::saved_lot(b"NOT-A-LOT".to_vec()),
        )
        .unwrap();
    assert_eq!(s2.slot, SaveSlot::Autosave(2));

    let recovered = store
        .load_latest_intact_autosave_validated("UserData", 5, lot_validator)
        .unwrap();
    assert_eq!(recovered.loaded.metadata.slot, SaveSlot::Autosave(1));
    assert_eq!(recovered.loaded.payload, b"LOT!good-1");
    assert_eq!(recovered.skipped_corrupt.len(), 1);
    assert_eq!(recovered.skipped_corrupt[0].0, SaveSlot::Autosave(2));

    // 4. `load_with_fallback_validated` also prefers direct lookup of a healthy
    //    manual slot without tripping on unusable ring slots.
    store
        .commit_job_validated(
            SaveJob::manual(
                "UserData",
                5,
                40,
                SavePayload::saved_lot(b"LOT!manual".to_vec()),
            ),
            lot_validator,
        )
        .unwrap();
    let direct = store
        .load_with_fallback_validated("UserData", 5, SaveSlot::Manual, lot_validator)
        .unwrap();
    assert_eq!(direct.loaded.metadata.slot, SaveSlot::Manual);
    assert!(direct.skipped_corrupt.is_empty());
}

#[test]
fn autosave_v2_compression_v1_compat_and_fast_metadata_listing() {
    use redb::ReadableDatabase;
    use sha2::{Digest, Sha256};

    let db = TempDb::new("autosave_v1_v2_meta");
    let store = AutosaveStore::open(&db.0).unwrap();

    // Repetitive 64 KiB payload representing a realistic serialized Saved lot.
    let mut payload = Vec::with_capacity(64 * 1024);
    for i in 0..(64 * 1024 / 16) {
        payload.extend_from_slice(&(i as u32).to_le_bytes());
        payload.extend_from_slice(&[0u8; 12]);
    }
    let expected_sha: [u8; 32] = Sha256::digest(&payload).into();

    let v2_meta = store
        .save_manual(
            "UserData",
            5,
            9001,
            Some(1_700_000_000),
            "Goth V2",
            SavePayload::saved_lot(payload.clone()),
        )
        .unwrap();
    assert_eq!(v2_meta.format_version, AUTOSAVE_FORMAT);
    assert_eq!(v2_meta.payload_len, payload.len());
    assert_eq!(v2_meta.payload_sha256, expected_sha);

    let loaded_v2 = store.load_manual("UserData", 5).unwrap();
    assert_eq!(loaded_v2.payload, payload);
    assert_eq!(loaded_v2.metadata, v2_meta);
    drop(store);

    // Inject a genuine V1 uncompressed autosave record at Autosave(1) and verify
    // V2 size is dramatically smaller while V1 still decodes bit-identically.
    const MANUAL_TABLE: redb::TableDefinition<(&str, u32), &[u8]> =
        redb::TableDefinition::new("autosave_manual");
    const RING_TABLE: redb::TableDefinition<(&str, u32, u64), &[u8]> =
        redb::TableDefinition::new("autosave_ring");

    #[derive(serde::Serialize)]
    struct LegacyAutosaveV1 {
        slot: SaveSlot,
        neighborhood_id: String,
        lot_id: u32,
        sim_tick: u64,
        saved_at_unix_secs: Option<u64>,
        label: String,
        payload_kind: SavePayloadKind,
        payload_sha256: [u8; 32],
        payload: Vec<u8>,
    }
    let v1_body = postcard::to_allocvec(&LegacyAutosaveV1 {
        slot: SaveSlot::Autosave(1),
        neighborhood_id: "UserData".into(),
        lot_id: 5,
        sim_tick: 8000,
        saved_at_unix_secs: Some(1_699_999_000),
        label: "Goth V1".into(),
        payload_kind: SavePayloadKind::SavedLot,
        payload_sha256: expected_sha,
        payload: payload.clone(),
    })
    .unwrap();
    let mut h = Sha256::new();
    h.update(b"sims-sim:autosave-record:v1");
    h.update(1u32.to_le_bytes());
    h.update(&v1_body);
    let mut v1_framed = Vec::new();
    v1_framed.extend_from_slice(b"SAVG");
    v1_framed.extend_from_slice(&1u32.to_le_bytes());
    v1_framed.extend_from_slice(&h.finalize());
    v1_framed.extend_from_slice(&v1_body);

    let raw_db = redb::Database::open(&db.0).unwrap();
    let v2_len = {
        let r = raw_db.begin_read().unwrap();
        let t = r.open_table(MANUAL_TABLE).unwrap();
        t.get(("UserData", 5)).unwrap().unwrap().value().len()
    };
    assert!(
        v2_len * 4 < v1_framed.len(),
        "expected V2 ({v2_len} B) to be >4x smaller than V1 ({} B)",
        v1_framed.len()
    );
    {
        let w = raw_db.begin_write().unwrap();
        {
            let mut rt = w.open_table(RING_TABLE).unwrap();
            rt.insert(("UserData", 5, 1), v1_framed.as_slice()).unwrap();
        }
        w.commit().unwrap();
    }
    drop(raw_db);

    let store = AutosaveStore::open(&db.0).unwrap();
    // Fast metadata-only listing sees both V2 manual and V1 autosave cleanly.
    let meta_list = store.list_saves_metadata("UserData", 5).unwrap();
    assert_eq!(meta_list.len(), 2);
    assert_eq!(meta_list[0].metadata(), Some(&v2_meta));
    let v1_meta = meta_list[1].metadata().expect("V1 metadata intact");
    assert_eq!(v1_meta.format_version, 1);
    assert_eq!(v1_meta.label, "Goth V1");
    assert_eq!(v1_meta.payload_len, payload.len());
    assert_eq!(v1_meta.payload_sha256, expected_sha);

    // Full V1 load restores exact payload bytes.
    let loaded_v1 = store
        .load_save("UserData", 5, SaveSlot::Autosave(1))
        .unwrap();
    assert_eq!(loaded_v1.payload, payload);
    assert_eq!(loaded_v1.metadata.format_version, 1);
}

#[test]
fn autosave_v2_rejects_oversized_writes_trailing_deflate_bytes_and_bad_payload_sha() {
    use redb::ReadableDatabase;
    use sha2::{Digest, Sha256};

    let db = TempDb::new("autosave_v2_hardening");
    let store = AutosaveStore::open_with_policy(&db.0, AutosavePolicy::new(1).unwrap()).unwrap();

    // Seed one intact autosave with retention = 1.
    let kept = store
        .save_autosave(
            "UserData",
            5,
            100,
            None,
            "Intact Slot",
            SavePayload::saved_lot(b"keep-me".to_vec()),
        )
        .unwrap();

    // Oversized payload (> 128 MiB) must be refused BEFORE committing or
    // pruning the existing intact autosave.
    let oversized = vec![0u8; 128 * 1024 * 1024 + 1];
    assert!(matches!(
        store.save_autosave(
            "UserData",
            5,
            101,
            None,
            "Too Big",
            SavePayload::saved_lot(oversized),
        ),
        Err(SaveError::InvalidInput(_))
    ));
    let loaded = store.load_save("UserData", 5, kept.slot).unwrap();
    assert_eq!(loaded.payload, b"keep-me");
    drop(store);

    const MANUAL_TABLE: redb::TableDefinition<(&str, u32), &[u8]> =
        redb::TableDefinition::new("autosave_manual");

    let sign_v2 = |body: &[u8]| -> Vec<u8> {
        let mut h = Sha256::new();
        h.update(b"sims-sim:autosave-record:v1");
        h.update(2u32.to_le_bytes());
        h.update(body);
        let mut framed = Vec::with_capacity(40 + body.len());
        framed.extend_from_slice(b"SAVG");
        framed.extend_from_slice(&2u32.to_le_bytes());
        framed.extend_from_slice(&h.finalize());
        framed.extend_from_slice(body);
        framed
    };

    // Write a valid V2 manual save first, then tamper its raw record with
    // (a) trailing bytes after the deflate stream (re-signed outer checksum) and
    // (b) a mismatched payload_sha256 in the V2 header (re-signed outer checksum).
    let store = AutosaveStore::open(&db.0).unwrap();
    store
        .save_manual(
            "UserData",
            5,
            200,
            None,
            "Valid V2",
            SavePayload::saved_lot(b"hello-v2-payload".to_vec()),
        )
        .unwrap();
    drop(store);

    let raw_db = redb::Database::open(&db.0).unwrap();
    let valid_raw = {
        let r = raw_db.begin_read().unwrap();
        let t = r.open_table(MANUAL_TABLE).unwrap();
        t.get(("UserData", 5)).unwrap().unwrap().value().to_vec()
    };

    // (a) Append trailing junk bytes after valid compressed stream + re-sign.
    let mut trailing_body = valid_raw[40..].to_vec();
    trailing_body.extend_from_slice(b"JUNK");
    let trailing_framed = sign_v2(&trailing_body);
    {
        let w = raw_db.begin_write().unwrap();
        {
            let mut t = w.open_table(MANUAL_TABLE).unwrap();
            t.insert(("UserData", 5), trailing_framed.as_slice())
                .unwrap();
        }
        w.commit().unwrap();
    }
    drop(raw_db);

    let store = AutosaveStore::open(&db.0).unwrap();
    assert!(matches!(
        store.load_manual("UserData", 5),
        Err(SaveError::DecodeFailed { .. })
    ));
    drop(store);

    // (b) Corrupt payload_sha256 in V2 header + re-sign outer checksum.
    let mut bad_sha_body = valid_raw[40..].to_vec();
    let expected_sha: [u8; 32] = Sha256::digest(b"hello-v2-payload").into();
    let pos = bad_sha_body
        .windows(32)
        .position(|w| w == expected_sha)
        .expect("payload_sha256 in V2 header");
    bad_sha_body[pos] ^= 0xff;
    let bad_sha_framed = sign_v2(&bad_sha_body);
    {
        let raw_db = redb::Database::open(&db.0).unwrap();
        let w = raw_db.begin_write().unwrap();
        {
            let mut t = w.open_table(MANUAL_TABLE).unwrap();
            t.insert(("UserData", 5), bad_sha_framed.as_slice())
                .unwrap();
        }
        w.commit().unwrap();
    }

    let store = AutosaveStore::open(&db.0).unwrap();
    assert!(matches!(
        store.load_manual("UserData", 5),
        Err(SaveError::CorruptChecksum { .. })
    ));
}
