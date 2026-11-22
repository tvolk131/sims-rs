//! Integration tests for `sims_sim::album_store::PhotoAlbumStore`.

use redb::ReadableDatabase;
use sims_sim::{
    ALBUM_FORMAT, AlbumError, AlbumImage, MAX_ALBUM_CAPTION_CHARS, MAX_ALBUM_ENTRIES,
    MAX_ALBUM_PHOTO_DIM, MAX_ALBUM_TITLE_CHARS, NeighborhoodStore, NewAlbumPhoto, PhotoAlbumStore,
    StoredNeighborhood,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempDb(PathBuf);

impl TempDb {
    fn new(label: &str) -> Self {
        let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sims-sim-album-{label}-{}-{n}.redb",
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

fn sample_neighborhood(name: &str) -> StoredNeighborhood {
    StoredNeighborhood {
        name: name.to_owned(),
        families: BTreeMap::new(),
        neighbors: Vec::new(),
        inventories: Vec::new(),
        data: [0; 16],
        options: None,
        type_attributes: None,
        zoning: BTreeMap::new(),
        lot_rows: Vec::new(),
        made_characters: sims_sim::characters::Characters::default(),
        deleted_characters: BTreeSet::new(),
        deletion_epoch: 0,
        deleted_at_epoch: BTreeMap::new(),
        instance_id: 1,
        revision: 0,
    }
}

fn sample_image(width: u32, height: u32, seed: u8) -> AlbumImage {
    let mut rgba = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for y in 0..height {
        for x in 0..width {
            rgba.push(((x * 7 + u32::from(seed)) & 0xff) as u8);
            rgba.push(((y * 11 + u32::from(seed)) & 0xff) as u8);
            rgba.push((((x + y) * 5 + u32::from(seed)) & 0xff) as u8);
            rgba.push(255);
        }
    }
    AlbumImage::from_rgba(width, height, &rgba).unwrap()
}

fn assert_clone_send_sync<T: Clone + Send + Sync>() {}

#[test]
fn album_store_crud_ordering_monotonic_ids_and_rgb565_round_trip() {
    assert_clone_send_sync::<PhotoAlbumStore>();

    let db = TempDb::new("crud-ordering");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("UserData", &sample_neighborhood("UserData"))
        .unwrap();
    let nh = store.load_neighborhood("UserData").unwrap().unwrap();
    let inst = nh.instance_id;
    let albums = store.albums();

    // Initially empty.
    assert!(albums.list("UserData", 1).unwrap().is_empty());
    assert!(albums.list_families("UserData").unwrap().is_empty());
    assert!(albums.get("UserData", 1, 1).unwrap().is_none());

    // Add 3 entries (two with RGB565 snapshots, one blank storybook page).
    let img1 = sample_image(8, 6, 10);
    let p1 = albums
        .add(
            "UserData",
            inst,
            1,
            NewAlbumPhoto::with_image("Move-in Day", "Welcome to 5 Sim Lane!", img1.clone())
                .with_sim_day(1),
        )
        .unwrap();
    let p2 = albums
        .add(
            "UserData",
            inst,
            1,
            NewAlbumPhoto::blank("Chapter Two", "A quiet afternoon.").with_sim_day(3),
        )
        .unwrap();
    let img3 = sample_image(4, 4, 77);
    let p3 = albums
        .add(
            "UserData",
            inst,
            1,
            NewAlbumPhoto::with_image("Party!", "Everyone danced.", img3.clone()).with_sim_day(5),
        )
        .unwrap();

    assert_eq!((p1.id, p2.id, p3.id), (1, 2, 3));
    assert_eq!(albums.list_families("UserData").unwrap(), vec![1]);

    // Check RGB565 -> RGBA expansion preserves 5-6-5 quantized channels.
    let rgba3 = p3.image.as_ref().unwrap().to_rgba();
    assert_eq!(rgba3.len(), 4 * 4 * 4);
    let repacked = AlbumImage::from_rgba(4, 4, &rgba3).unwrap();
    assert_eq!(repacked, img3);

    // Update text on entry 2.
    let updated = albums
        .update_text(
            "UserData",
            inst,
            1,
            p2.id,
            "Chapter Two (Edited)",
            "Updated caption text.",
        )
        .unwrap();
    assert_eq!(updated.id, p2.id);
    assert_eq!(updated.title, "Chapter Two (Edited)");
    assert_eq!(updated.caption, "Updated caption text.");
    assert_eq!(updated.sim_day, Some(3));
    assert!(updated.image.is_none());

    // Reorder: move entry 3 to index 0 -> [3, 1, 2].
    let order = albums.move_entry("UserData", inst, 1, p3.id, 0).unwrap();
    assert_eq!(order, vec![3, 1, 2]);
    let listed = albums.list("UserData", 1).unwrap();
    assert_eq!(
        listed.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![3, 1, 2]
    );
    assert_eq!(listed[0].image.as_ref(), Some(&img3));
    assert_eq!(listed[1].image.as_ref(), Some(&img1));
    assert_eq!(listed[2].title, "Chapter Two (Edited)");

    // Same-index move is a valid no-op; out-of-bounds index is rejected.
    assert_eq!(
        albums.move_entry("UserData", inst, 1, p3.id, 0).unwrap(),
        vec![3, 1, 2]
    );
    assert!(matches!(
        albums.move_entry("UserData", inst, 1, p3.id, 3),
        Err(AlbumError::InvalidIndex { index: 3, len: 3 })
    ));
    assert!(matches!(
        albums.move_entry("UserData", inst, 1, 999, 0),
        Err(AlbumError::PhotoNotFound { photo_id: 999, .. })
    ));

    // Delete entry 3: returns true once, false on repeat, and next add gets ID 4 (never reuses 3).
    assert!(albums.delete("UserData", inst, 1, p3.id).unwrap());
    assert!(!albums.delete("UserData", inst, 1, p3.id).unwrap());
    assert!(albums.get("UserData", 1, p3.id).unwrap().is_none());

    let p4 = albums
        .add("UserData", inst, 1, NewAlbumPhoto::blank("Fourth", ""))
        .unwrap();
    assert_eq!(p4.id, 4);
    assert_eq!(
        albums
            .list("UserData", 1)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect::<Vec<_>>(),
        vec![1, 2, 4]
    );

    // Album operations never bump StoredNeighborhood::revision.
    let nh_after = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(nh_after.revision, nh.revision);
}

#[test]
fn album_store_enforces_bounds_and_max_entries() {
    let db = TempDb::new("bounds");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("UserData", &sample_neighborhood("UserData"))
        .unwrap();
    let inst = store
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap()
        .instance_id;
    let albums = store.albums();

    // Invalid IDs.
    assert!(matches!(
        albums.add("", inst, 1, NewAlbumPhoto::blank("t", "c")),
        Err(AlbumError::InvalidInput(_))
    ));
    assert!(matches!(
        albums.add("UserData", 0, 1, NewAlbumPhoto::blank("t", "c")),
        Err(AlbumError::InvalidInput(_))
    ));
    assert!(matches!(
        albums.add("UserData", inst, 0, NewAlbumPhoto::blank("t", "c")),
        Err(AlbumError::InvalidInput(_))
    ));

    // Oversized title / caption.
    let long_title = "a".repeat(MAX_ALBUM_TITLE_CHARS + 1);
    let long_caption = "b".repeat(MAX_ALBUM_CAPTION_CHARS + 1);
    assert!(matches!(
        albums.add("UserData", inst, 1, NewAlbumPhoto::blank(long_title, "")),
        Err(AlbumError::InvalidInput(_))
    ));
    assert!(matches!(
        albums.add("UserData", inst, 1, NewAlbumPhoto::blank("", long_caption)),
        Err(AlbumError::InvalidInput(_))
    ));

    // Invalid image dimensions / buffer lengths.
    assert!(matches!(
        AlbumImage::new(0, 10, vec![]),
        Err(AlbumError::InvalidInput(_))
    ));
    assert!(matches!(
        AlbumImage::new(
            MAX_ALBUM_PHOTO_DIM + 1,
            1,
            vec![0; (MAX_ALBUM_PHOTO_DIM + 1) as usize]
        ),
        Err(AlbumError::InvalidInput(_))
    ));
    assert!(matches!(
        AlbumImage::new(1024, 1024, vec![]),
        Err(AlbumError::InvalidInput(_))
    ));
    assert!(matches!(
        AlbumImage::new(4, 4, vec![0; 15]),
        Err(AlbumError::InvalidInput(_))
    ));
    assert!(matches!(
        AlbumImage::from_rgba(4, 4, &[0u8; 63]),
        Err(AlbumError::InvalidInput(_))
    ));

    // Fill to MAX_ALBUM_ENTRIES, then verify the next add returns AlbumFull.
    for i in 0..MAX_ALBUM_ENTRIES {
        albums
            .add(
                "UserData",
                inst,
                2,
                NewAlbumPhoto::blank(format!("Page {i}"), ""),
            )
            .unwrap();
    }
    assert!(matches!(
        albums.add("UserData", inst, 2, NewAlbumPhoto::blank("Overflow", "")),
        Err(AlbumError::AlbumFull {
            max_entries: MAX_ALBUM_ENTRIES,
            ..
        })
    ));
    assert_eq!(albums.list("UserData", 2).unwrap().len(), MAX_ALBUM_ENTRIES);
}

#[test]
fn album_store_fences_instance_id_and_purges_on_delete_and_reimport() {
    let db = TempDb::new("fencing-purge");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("NH1", &sample_neighborhood("NH1"))
        .unwrap();
    store
        .save_neighborhood("NH2", &sample_neighborhood("NH2"))
        .unwrap();
    let inst1 = store.load_neighborhood("NH1").unwrap().unwrap().instance_id;
    let inst2 = store.load_neighborhood("NH2").unwrap().unwrap().instance_id;
    let albums = store.albums();

    let p1 = albums
        .add("NH1", inst1, 1, NewAlbumPhoto::blank("NH1 Photo", "cap"))
        .unwrap();
    let _p2 = albums
        .add("NH2", inst2, 1, NewAlbumPhoto::blank("NH2 Photo", "cap"))
        .unwrap();

    // Wrong instance_id fails on all mutators with zero changes.
    let stale_inst = inst1 ^ 0xdead_beef;
    assert!(matches!(
        albums.add("NH1", stale_inst, 1, NewAlbumPhoto::blank("Stale", "")),
        Err(AlbumError::StaleNeighborhoodInstance {
            current_instance_id: Some(cur),
            ..
        }) if cur == inst1
    ));
    assert!(matches!(
        albums.update_text("NH1", stale_inst, 1, p1.id, "Bad", "Bad"),
        Err(AlbumError::StaleNeighborhoodInstance { .. })
    ));
    assert!(matches!(
        albums.move_entry("NH1", stale_inst, 1, p1.id, 0),
        Err(AlbumError::StaleNeighborhoodInstance { .. })
    ));
    assert!(matches!(
        albums.delete("NH1", stale_inst, 1, p1.id),
        Err(AlbumError::StaleNeighborhoodInstance { .. })
    ));
    assert!(matches!(
        albums.delete_family_album("NH1", stale_inst, 1),
        Err(AlbumError::StaleNeighborhoodInstance { .. })
    ));
    assert_eq!(albums.list("NH1", 1).unwrap().len(), 1);

    // Deleting NH1 atomically purges NH1's album while preserving NH2's album.
    assert!(store.delete_neighborhood("NH1").unwrap());
    assert!(albums.list("NH1", 1).unwrap().is_empty());
    assert_eq!(albums.list("NH2", 1).unwrap().len(), 1);

    // Writing with old inst1 after delete returns StaleNeighborhoodInstance { current: None }.
    assert!(matches!(
        albums.add("NH1", inst1, 1, NewAlbumPhoto::blank("AfterDelete", "")),
        Err(AlbumError::StaleNeighborhoodInstance {
            current_instance_id: None,
            ..
        })
    ));

    // Recreating NH1 mints a new instance_id; old inst1 is still fenced out.
    store
        .save_neighborhood("NH1", &sample_neighborhood("NH1"))
        .unwrap();
    let new_inst1 = store.load_neighborhood("NH1").unwrap().unwrap().instance_id;
    assert_ne!(new_inst1, inst1);
    assert!(matches!(
        albums.add("NH1", inst1, 1, NewAlbumPhoto::blank("OldInst", "")),
        Err(AlbumError::StaleNeighborhoodInstance {
            current_instance_id: Some(cur),
            ..
        }) if cur == new_inst1
    ));

    // If SIMS_INSTALL is set, verify `import_directory` also purges album rows atomically.
    if let Some(root) = std::env::var_os("SIMS_INSTALL") {
        let user_data = PathBuf::from(root).join("UserData");
        albums
            .add("NH1", new_inst1, 5, NewAlbumPhoto::blank("PreImport", ""))
            .unwrap();
        assert_eq!(albums.list("NH1", 5).unwrap().len(), 1);
        store.import_directory("NH1", &user_data, 0xff).unwrap();
        assert!(albums.list("NH1", 5).unwrap().is_empty());
        assert_eq!(albums.list("NH2", 1).unwrap().len(), 1);
    }
}

#[test]
fn album_store_detects_checksum_version_and_key_binding_corruption() {
    let db = TempDb::new("corruption");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("UserData", &sample_neighborhood("UserData"))
        .unwrap();
    let inst = store
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap()
        .instance_id;
    let albums = store.albums();
    let p1 = albums
        .add(
            "UserData",
            inst,
            1,
            NewAlbumPhoto::with_image("Intact", "Caption", sample_image(2, 2, 1)),
        )
        .unwrap();
    let p2 = albums
        .add("UserData", inst, 2, NewAlbumPhoto::blank("Other", ""))
        .unwrap();
    drop(albums);
    drop(store);

    // Tamper directly with REDB tables to test integrity verification.
    const INDEX_TABLE: redb::TableDefinition<(&str, u16), &[u8]> =
        redb::TableDefinition::new("photo_album_index");
    const PHOTO_TABLE: redb::TableDefinition<(&str, u16, u64), &[u8]> =
        redb::TableDefinition::new("photo_album_photos");

    let raw_db = redb::Database::open(&db.0).unwrap();
    let raw_fam1_photo = {
        let r = raw_db.begin_read().unwrap();
        let t = r.open_table(PHOTO_TABLE).unwrap();
        t.get(("UserData", 1, p1.id))
            .unwrap()
            .unwrap()
            .value()
            .to_vec()
    };

    // 1. Copy family 1's valid photo bytes into family 2's slot -> key binding mismatch!
    {
        let w = raw_db.begin_write().unwrap();
        {
            let mut t = w.open_table(PHOTO_TABLE).unwrap();
            t.insert(("UserData", 2, p2.id), raw_fam1_photo.as_slice())
                .unwrap();
        }
        w.commit().unwrap();
    }
    drop(raw_db);

    let store = NeighborhoodStore::open(&db.0).unwrap();
    let albums = store.albums();
    assert!(matches!(
        albums.list("UserData", 2),
        Err(AlbumError::DecodeFailed { .. })
    ));
    drop(albums);
    drop(store);

    // 2. Flip a byte in family 1's photo payload -> CorruptChecksum.
    let raw_db = redb::Database::open(&db.0).unwrap();
    {
        let mut flipped = raw_fam1_photo.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0x01;
        let w = raw_db.begin_write().unwrap();
        {
            let mut t = w.open_table(PHOTO_TABLE).unwrap();
            t.insert(("UserData", 1, p1.id), flipped.as_slice())
                .unwrap();
        }
        w.commit().unwrap();
    }
    drop(raw_db);

    let store = NeighborhoodStore::open(&db.0).unwrap();
    let albums = store.albums();
    assert!(matches!(
        albums.list("UserData", 1),
        Err(AlbumError::CorruptChecksum { .. })
    ));
    drop(albums);
    drop(store);

    // 3. Future version header on index -> UnsupportedVersion.
    let raw_db = redb::Database::open(&db.0).unwrap();
    {
        let w = raw_db.begin_write().unwrap();
        {
            let mut t = w.open_table(INDEX_TABLE).unwrap();
            let mut future = Vec::new();
            future.extend_from_slice(b"ALBI");
            future.extend_from_slice(&(ALBUM_FORMAT + 1).to_le_bytes());
            future.extend_from_slice(&[0u8; 32]);
            t.insert(("UserData", 1), future.as_slice()).unwrap();
        }
        w.commit().unwrap();
    }
    drop(raw_db);

    let store = NeighborhoodStore::open(&db.0).unwrap();
    let albums = store.albums();
    assert!(matches!(
        albums.list("UserData", 1),
        Err(AlbumError::UnsupportedVersion {
            found,
            ..
        }) if found == ALBUM_FORMAT + 1
    ));
    // Writes also refuse to silently overwrite unsupported-version indices.
    assert!(matches!(
        albums.add("UserData", inst, 1, NewAlbumPhoto::blank("NoClobber", "")),
        Err(AlbumError::UnsupportedVersion { .. })
    ));
}

#[test]
fn album_store_concurrent_shared_handles() {
    let db = TempDb::new("concurrent");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("UserData", &sample_neighborhood("UserData"))
        .unwrap();
    let inst = store
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap()
        .instance_id;
    let albums = store.albums();

    std::thread::scope(|scope| {
        for fam in 1u16..=4 {
            let handle = albums.clone();
            scope.spawn(move || {
                for n in 0..8 {
                    handle
                        .add(
                            "UserData",
                            inst,
                            fam,
                            NewAlbumPhoto::with_image(
                                format!("F{fam} P{n}"),
                                format!("Caption {n}"),
                                sample_image(4, 4, (fam as u8) * 10 + n as u8),
                            ),
                        )
                        .unwrap();
                }
            });
        }
    });

    assert_eq!(albums.list_families("UserData").unwrap(), vec![1, 2, 3, 4]);
    for fam in 1u16..=4 {
        let entries = albums.list("UserData", fam).unwrap();
        assert_eq!(entries.len(), 8);
        assert_eq!(
            entries.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
    }
}
