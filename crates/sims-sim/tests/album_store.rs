//! Integration tests for `sims_sim::album_store::PhotoAlbumStore`.

use redb::ReadableDatabase;
use sha2::{Digest, Sha256};
use sims_formats::neighborhood::Family;
use sims_sim::{
    ALBUM_FORMAT, AlbumError, AlbumFamilyToken, AlbumImage, MAX_ALBUM_CAPTION_CHARS,
    MAX_ALBUM_ENTRIES, MAX_ALBUM_PHOTO_DIM, MAX_ALBUM_PHOTO_ENCODED_BYTES, MAX_ALBUM_TITLE_CHARS,
    NeighborhoodStore, NewAlbumPhoto, PhotoAlbumStore, StoredNeighborhood,
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

fn dummy_family(number: i32) -> Family {
    Family {
        version: 9,
        house: 0,
        number,
        budget: 20_000,
        architecture_value: 0,
        friends: 0,
        flags: 0,
        members: vec![0x1000 + number as u32],
        tail: vec![0; 16],
    }
}

fn sample_neighborhood(name: &str, family_ids: &[u16]) -> StoredNeighborhood {
    let mut families = BTreeMap::new();
    for (i, &fid) in family_ids.iter().enumerate() {
        families.insert(fid, dummy_family((i as i32) + 1));
    }
    StoredNeighborhood {
        name: name.to_owned(),
        families,
        family_surnames: BTreeMap::new(),
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
fn album_store_crud_ordering_paging_summaries_and_rgb565_round_trip() {
    assert_clone_send_sync::<PhotoAlbumStore>();

    let db = TempDb::new("crud-ordering");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("UserData", &sample_neighborhood("UserData", &[1]))
        .unwrap();
    let nh = store.load_neighborhood("UserData").unwrap().unwrap();
    let albums = store.albums();
    let tok = albums.family_token("UserData", 1).unwrap().unwrap();
    assert_eq!(tok.instance_id, nh.instance_id);
    assert_eq!(tok.family_id, 1);
    assert!(albums.family_token("UserData", 99).unwrap().is_none());

    // Initially empty.
    assert!(albums.list("UserData", 1).unwrap().is_empty());
    assert!(albums.list_summaries("UserData", 1).unwrap().is_empty());
    assert!(albums.list_families("UserData").unwrap().is_empty());
    assert!(albums.get("UserData", 1, 1).unwrap().is_none());

    // Add 3 entries (two with RGB565 snapshots, one blank storybook page).
    let img1 = sample_image(8, 6, 10);
    let p1 = albums
        .add(
            "UserData",
            tok,
            NewAlbumPhoto::with_image("Move-in Day", "Welcome to 5 Sim Lane!", img1.clone())
                .with_sim_day(1),
        )
        .unwrap();
    let p2 = albums
        .add(
            "UserData",
            tok,
            NewAlbumPhoto::blank("Chapter Two", "A quiet afternoon.").with_sim_day(3),
        )
        .unwrap();
    let img3 = sample_image(4, 4, 77);
    let p3 = albums
        .add(
            "UserData",
            tok,
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
            tok,
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
    let order = albums.move_entry("UserData", tok, p3.id, 0).unwrap();
    assert_eq!(order, vec![3, 1, 2]);
    let listed = albums.list("UserData", 1).unwrap();
    assert_eq!(
        listed.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![3, 1, 2]
    );
    assert_eq!(listed[0].image.as_ref(), Some(&img3));
    assert_eq!(listed[1].image.as_ref(), Some(&img1));
    assert_eq!(listed[2].title, "Chapter Two (Edited)");

    // Metadata-only summaries & paging & pixel budget enforcement.
    let summaries = albums.list_summaries("UserData", 1).unwrap();
    assert_eq!(summaries.len(), 3);
    assert_eq!(summaries[0].id, 3);
    assert_eq!(summaries[0].image_dimensions, Some((4, 4)));
    assert_eq!(summaries[1].id, 1);
    assert_eq!(summaries[1].image_dimensions, Some((8, 6)));
    assert_eq!(summaries[2].id, 2);
    assert_eq!(summaries[2].image_dimensions, None);

    let page = albums.list_page("UserData", 1, 1, 2).unwrap();
    assert_eq!(page.iter().map(|p| p.id).collect::<Vec<_>>(), vec![1, 2]);
    assert!(albums.list_page("UserData", 1, 5, 2).unwrap().is_empty());
    // img3 is 32 bytes (4*4*2), img1 is 96 bytes (8*6*2) -> 128 total; cap at 64 fails!
    assert!(matches!(
        albums.list_page_with_budget("UserData", 1, 0, 3, 64),
        Err(AlbumError::BudgetExceeded {
            limit_bytes: 64,
            required_bytes: 128,
            ..
        })
    ));

    // Same-index move is a valid no-op; out-of-bounds index is rejected.
    assert_eq!(
        albums.move_entry("UserData", tok, p3.id, 0).unwrap(),
        vec![3, 1, 2]
    );
    assert!(matches!(
        albums.move_entry("UserData", tok, p3.id, 3),
        Err(AlbumError::InvalidIndex { index: 3, len: 3 })
    ));
    assert!(matches!(
        albums.move_entry("UserData", tok, 999, 0),
        Err(AlbumError::PhotoNotFound { photo_id: 999, .. })
    ));

    // Delete entry 3: returns true once, false on repeat, and next add gets ID 4 (never reuses 3).
    assert!(albums.delete("UserData", tok, p3.id).unwrap());
    assert!(!albums.delete("UserData", tok, p3.id).unwrap());
    assert!(albums.get("UserData", 1, p3.id).unwrap().is_none());

    let p4 = albums
        .add("UserData", tok, NewAlbumPhoto::blank("Fourth", ""))
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

    // Clearing the album preserves next_photo_id so newly added photo gets ID 5.
    assert!(albums.delete_family_album("UserData", tok).unwrap());
    assert!(albums.list("UserData", 1).unwrap().is_empty());
    let p5 = albums
        .add("UserData", tok, NewAlbumPhoto::blank("Fifth", ""))
        .unwrap();
    assert_eq!(p5.id, 5);

    // Album operations never bump StoredNeighborhood::revision.
    let nh_after = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(nh_after.revision, nh.revision);
}

#[test]
fn album_store_enforces_bounds_and_max_entries() {
    let db = TempDb::new("bounds");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("UserData", &sample_neighborhood("UserData", &[1, 2]))
        .unwrap();
    let albums = store.albums();
    let tok1 = albums.family_token("UserData", 1).unwrap().unwrap();
    let tok2 = albums.family_token("UserData", 2).unwrap().unwrap();

    // Invalid IDs / zero token fields.
    assert!(matches!(
        albums.add("", tok1, NewAlbumPhoto::blank("t", "c")),
        Err(AlbumError::InvalidInput(_))
    ));
    assert!(matches!(
        albums.add(
            "UserData",
            AlbumFamilyToken {
                instance_id: 0,
                family_id: 1,
                family_epoch: 1
            },
            NewAlbumPhoto::blank("t", "c")
        ),
        Err(AlbumError::InvalidInput(_))
    ));
    assert!(matches!(
        albums.add(
            "UserData",
            AlbumFamilyToken {
                instance_id: tok1.instance_id,
                family_id: 0,
                family_epoch: 1
            },
            NewAlbumPhoto::blank("t", "c")
        ),
        Err(AlbumError::InvalidInput(_))
    ));
    assert!(matches!(
        albums.add(
            "UserData",
            AlbumFamilyToken {
                instance_id: tok1.instance_id,
                family_id: 1,
                family_epoch: 0
            },
            NewAlbumPhoto::blank("t", "c")
        ),
        Err(AlbumError::InvalidInput(_))
    ));

    // Oversized title / caption.
    let long_title = "a".repeat(MAX_ALBUM_TITLE_CHARS + 1);
    let long_caption = "b".repeat(MAX_ALBUM_CAPTION_CHARS + 1);
    assert!(matches!(
        albums.add("UserData", tok1, NewAlbumPhoto::blank(long_title, "")),
        Err(AlbumError::InvalidInput(_))
    ));
    assert!(matches!(
        albums.add("UserData", tok1, NewAlbumPhoto::blank("", long_caption)),
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
                tok2,
                NewAlbumPhoto::blank(format!("Page {i}"), ""),
            )
            .unwrap();
    }
    assert!(matches!(
        albums.add("UserData", tok2, NewAlbumPhoto::blank("Overflow", "")),
        Err(AlbumError::AlbumFull {
            max_entries: MAX_ALBUM_ENTRIES,
            ..
        })
    ));
    assert_eq!(albums.list("UserData", 2).unwrap().len(), MAX_ALBUM_ENTRIES);
}

#[test]
fn album_store_fences_family_lifetime_and_purges_atomically_on_family_or_neighborhood_delete() {
    let db = TempDb::new("fencing-purge");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("NH1", &sample_neighborhood("NH1", &[1, 2]))
        .unwrap();
    store
        .save_neighborhood("NH2", &sample_neighborhood("NH2", &[1]))
        .unwrap();
    let albums = store.albums();
    let tok1 = albums.family_token("NH1", 1).unwrap().unwrap();
    let tok1_fam2 = albums.family_token("NH1", 2).unwrap().unwrap();
    let tok2 = albums.family_token("NH2", 1).unwrap().unwrap();

    let p1 = albums
        .add("NH1", tok1, NewAlbumPhoto::blank("NH1 Fam1 Photo", "cap"))
        .unwrap();
    let _p1_f2 = albums
        .add("NH1", tok1_fam2, NewAlbumPhoto::blank("NH1 Fam2", "keep"))
        .unwrap();
    let _p2 = albums
        .add("NH2", tok2, NewAlbumPhoto::blank("NH2 Photo", "cap"))
        .unwrap();

    // 1. Deleting family 1 via `save_neighborhood_checked` (the Family Bin UI
    //    path!) atomically purges family 1's album and bumps its family epoch,
    //    while keeping family 2's album untouched.
    {
        let mut nh1 = store.load_neighborhood("NH1").unwrap().unwrap();
        nh1.families.remove(&1);
        store
            .save_neighborhood_checked("NH1", nh1.instance_id, nh1.revision, &nh1)
            .unwrap()
            .unwrap();
    }
    assert!(albums.list("NH1", 1).unwrap().is_empty());
    assert_eq!(albums.list("NH1", 2).unwrap().len(), 1);
    assert!(albums.family_token("NH1", 1).unwrap().is_none());

    // Delayed add while family 1 is absent is rejected cleanly.
    assert!(matches!(
        albums.add("NH1", tok1, NewAlbumPhoto::blank("Ghost", "")),
        Err(AlbumError::StaleFamilyToken {
            family_id: 1,
            current_epoch: None,
            ..
        })
    ));

    // Recreate a brand-new household reusing family ID 1 in the SAME neighborhood
    // instance; it starts with an empty album and a strictly newer family_epoch.
    {
        let mut nh1 = store.load_neighborhood("NH1").unwrap().unwrap();
        nh1.families.insert(1, dummy_family(99));
        store
            .save_neighborhood_checked("NH1", nh1.instance_id, nh1.revision, &nh1)
            .unwrap()
            .unwrap();
    }
    assert!(albums.list("NH1", 1).unwrap().is_empty());
    let new_tok1 = albums.family_token("NH1", 1).unwrap().unwrap();
    assert_eq!(new_tok1.instance_id, tok1.instance_id);
    assert_eq!(new_tok1.family_epoch, tok1.family_epoch + 1);
    let repl_p1 = albums
        .add(
            "NH1",
            new_tok1,
            NewAlbumPhoto::blank("Replacement Photo", "safe"),
        )
        .unwrap();
    assert_eq!(repl_p1.id, p1.id); // both are photo ID 1!

    // Stale token from the original household MUST fail on all mutators against
    // the replacement household!
    assert!(matches!(
        albums.update_text("NH1", tok1, p1.id, "Hijacked", "Bad"),
        Err(AlbumError::StaleFamilyToken {
            current_epoch: Some(cur),
            ..
        }) if cur == new_tok1.family_epoch
    ));
    assert!(matches!(
        albums.move_entry("NH1", tok1, p1.id, 0),
        Err(AlbumError::StaleFamilyToken { .. })
    ));
    assert!(matches!(
        albums.delete("NH1", tok1, p1.id),
        Err(AlbumError::StaleFamilyToken { .. })
    ));
    assert!(matches!(
        albums.delete_family_album("NH1", tok1),
        Err(AlbumError::StaleFamilyToken { .. })
    ));
    assert_eq!(albums.list("NH1", 1).unwrap()[0].title, "Replacement Photo");

    // 2. Wrong neighborhood instance_id fails with StaleNeighborhoodInstance.
    let stale_inst_tok = AlbumFamilyToken {
        instance_id: new_tok1.instance_id ^ 0xdead_beef,
        ..new_tok1
    };
    assert!(matches!(
        albums.add("NH1", stale_inst_tok, NewAlbumPhoto::blank("Stale", "")),
        Err(AlbumError::StaleNeighborhoodInstance {
            current_instance_id: Some(cur),
            ..
        }) if cur == new_tok1.instance_id
    ));

    // 3. Deleting NH1 atomically purges NH1's albums while preserving NH2's album.
    assert!(store.delete_neighborhood("NH1").unwrap());
    assert!(albums.list("NH1", 1).unwrap().is_empty());
    assert!(albums.list("NH1", 2).unwrap().is_empty());
    assert_eq!(albums.list("NH2", 1).unwrap().len(), 1);
    assert!(matches!(
        albums.add("NH1", new_tok1, NewAlbumPhoto::blank("AfterDelete", "")),
        Err(AlbumError::StaleNeighborhoodInstance {
            current_instance_id: None,
            ..
        })
    ));

    // Recreating NH1 mints a new instance_id; old token is still fenced out.
    store
        .save_neighborhood("NH1", &sample_neighborhood("NH1", &[1, 5]))
        .unwrap();
    let recreated_tok = albums.family_token("NH1", 5).unwrap().unwrap();
    assert_ne!(recreated_tok.instance_id, new_tok1.instance_id);
    assert!(matches!(
        albums.add("NH1", new_tok1, NewAlbumPhoto::blank("OldInst", "")),
        Err(AlbumError::StaleNeighborhoodInstance {
            current_instance_id: Some(cur),
            ..
        }) if cur == recreated_tok.instance_id
    ));

    // 4. If SIMS_INSTALL is set, verify `import_directory` also purges album rows atomically.
    if let Some(root) = std::env::var_os("SIMS_INSTALL") {
        let user_data = PathBuf::from(root).join("UserData");
        albums
            .add("NH1", recreated_tok, NewAlbumPhoto::blank("PreImport", ""))
            .unwrap();
        assert_eq!(albums.list("NH1", 5).unwrap().len(), 1);
        store.import_directory("NH1", &user_data, 0xff).unwrap();
        assert!(albums.list("NH1", 5).unwrap().is_empty());
        assert_eq!(albums.list("NH2", 1).unwrap().len(), 1);
    }
}

#[test]
fn album_store_detects_checksum_version_key_binding_and_pre_allocation_bounds() {
    let db = TempDb::new("corruption");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("UserData", &sample_neighborhood("UserData", &[1, 2]))
        .unwrap();
    let albums = store.albums();
    let tok1 = albums.family_token("UserData", 1).unwrap().unwrap();
    let tok2 = albums.family_token("UserData", 2).unwrap().unwrap();
    let p1 = albums
        .add(
            "UserData",
            tok1,
            NewAlbumPhoto::with_image("Intact", "Caption", sample_image(2, 2, 1)),
        )
        .unwrap();
    let p2 = albums
        .add("UserData", tok2, NewAlbumPhoto::blank("Other", ""))
        .unwrap();
    drop(albums);
    drop(store);

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
    assert!(matches!(
        store.albums().list("UserData", 2),
        Err(AlbumError::DecodeFailed { .. })
    ));
    drop(store);

    // 2. Correctly checksummed current-version photo claiming `1x1` dimensions
    //    with a mismatched large pixel vector (100,000 elements) must be rejected
    //    during deserialization before allocation!
    let raw_db = redb::Database::open(&db.0).unwrap();
    {
        #[derive(serde::Serialize)]
        struct BadImg {
            width: u32,
            height: u32,
            rgb565: Vec<u16>,
        }
        #[derive(serde::Serialize)]
        struct BadPhoto {
            neighborhood_id: String,
            family_id: u16,
            id: u64,
            title: String,
            caption: String,
            sim_day: Option<u32>,
            image: Option<BadImg>,
        }
        let body = postcard::to_allocvec(&BadPhoto {
            neighborhood_id: "UserData".into(),
            family_id: 2,
            id: p2.id,
            title: "Bomb".into(),
            caption: String::new(),
            sim_day: None,
            image: Some(BadImg {
                width: 1,
                height: 1,
                rgb565: vec![0u16; 100_000],
            }),
        })
        .unwrap();
        let mut h = Sha256::new();
        h.update(b"sims-sim:album-photo:v1");
        h.update(ALBUM_FORMAT.to_le_bytes());
        h.update(&body);
        let mut framed = Vec::new();
        framed.extend_from_slice(b"ALBP");
        framed.extend_from_slice(&ALBUM_FORMAT.to_le_bytes());
        framed.extend_from_slice(&h.finalize());
        framed.extend_from_slice(&body);
        let w = raw_db.begin_write().unwrap();
        {
            let mut t = w.open_table(PHOTO_TABLE).unwrap();
            t.insert(("UserData", 2, p2.id), framed.as_slice()).unwrap();
        }
        w.commit().unwrap();
    }
    drop(raw_db);

    let store = NeighborhoodStore::open(&db.0).unwrap();
    assert!(matches!(
        store.albums().get("UserData", 2, p2.id),
        Err(AlbumError::DecodeFailed { .. })
    ));
    drop(store);

    // 3. Oversized encoded photo blob (> MAX_ALBUM_PHOTO_ENCODED_BYTES + neighborhood_id.len())
    //    rejected before hashing.
    let raw_db = redb::Database::open(&db.0).unwrap();
    {
        let mut oversized = vec![0u8; MAX_ALBUM_PHOTO_ENCODED_BYTES + "UserData".len() + 1];
        oversized[0..4].copy_from_slice(b"ALBP");
        oversized[4..8].copy_from_slice(&ALBUM_FORMAT.to_le_bytes());
        let w = raw_db.begin_write().unwrap();
        {
            let mut t = w.open_table(PHOTO_TABLE).unwrap();
            t.insert(("UserData", 2, p2.id), oversized.as_slice())
                .unwrap();
        }
        w.commit().unwrap();
    }
    drop(raw_db);

    let store = NeighborhoodStore::open(&db.0).unwrap();
    assert!(matches!(
        store.albums().get("UserData", 2, p2.id),
        Err(AlbumError::DecodeFailed { .. })
    ));
    drop(store);

    // 4. Flip a byte in family 1's photo payload -> CorruptChecksum.
    let raw_db = redb::Database::open(&db.0).unwrap();
    {
        let mut flipped = raw_fam1_photo;
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
    assert!(matches!(
        store.albums().list("UserData", 1),
        Err(AlbumError::CorruptChecksum { .. })
    ));
    drop(store);

    // 5. Future version header on index -> UnsupportedVersion, and writes refuse
    //    to silently overwrite it.
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
        Err(AlbumError::UnsupportedVersion { found, .. }) if found == ALBUM_FORMAT + 1
    ));
    assert!(matches!(
        albums.add("UserData", tok1, NewAlbumPhoto::blank("NoClobber", "")),
        Err(AlbumError::UnsupportedVersion { .. })
    ));
}

#[test]
fn album_store_concurrent_shared_handles() {
    let db = TempDb::new("concurrent");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("UserData", &sample_neighborhood("UserData", &[1, 2, 3, 4]))
        .unwrap();
    let albums = store.albums();

    std::thread::scope(|scope| {
        for fam in 1u16..=4 {
            let handle = albums.clone();
            let tok = handle.family_token("UserData", fam).unwrap().unwrap();
            scope.spawn(move || {
                for n in 0..8 {
                    handle
                        .add(
                            "UserData",
                            tok,
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

#[test]
fn preexisting_orphan_album_is_purged_on_new_family_allocation_and_long_ids_round_trip() {
    use redb::ReadableDatabase;

    const INDEX_TABLE: redb::TableDefinition<(&str, u16), &[u8]> =
        redb::TableDefinition::new("photo_album_index");
    const PHOTO_TABLE: redb::TableDefinition<(&str, u16, u64), &[u8]> =
        redb::TableDefinition::new("photo_album_photos");

    // 1. Seed a valid album for family 2, then simulate an older upgraded DB
    //    where family 2 was removed without purging its album rows (orphan).
    let db = TempDb::new("orphan-and-long-id");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("UserData", &sample_neighborhood("UserData", &[1, 2]))
        .unwrap();
    let albums = store.albums();
    let tok2 = albums.family_token("UserData", 2).unwrap().unwrap();
    let orphan_photo = albums
        .add(
            "UserData",
            tok2,
            NewAlbumPhoto::with_image("Orphan", "From legacy DB", sample_image(2, 2, 9)),
        )
        .unwrap();
    assert_eq!(orphan_photo.id, 1);

    // Capture raw album rows for family 2, delete family 2 cleanly, then
    // re-inject the orphaned rows directly in REDB to simulate a pre-fix DB.
    drop(albums);
    drop(store);
    let (raw_index, raw_photo) = {
        let raw_db = redb::Database::open(&db.0).unwrap();
        let r = raw_db.begin_read().unwrap();
        let idx = r
            .open_table(INDEX_TABLE)
            .unwrap()
            .get(("UserData", 2))
            .unwrap()
            .unwrap()
            .value()
            .to_vec();
        let ph = r
            .open_table(PHOTO_TABLE)
            .unwrap()
            .get(("UserData", 2, 1))
            .unwrap()
            .unwrap()
            .value()
            .to_vec();
        (idx, ph)
    };

    let store = NeighborhoodStore::open(&db.0).unwrap();
    let mut without_fam2 = store.load_neighborhood("UserData").unwrap().unwrap();
    let fam2_record = without_fam2.families.remove(&2).unwrap();
    store
        .save_neighborhood_checked(
            "UserData",
            without_fam2.instance_id,
            without_fam2.revision,
            &without_fam2,
        )
        .unwrap()
        .unwrap();
    drop(store);

    // Re-inject orphan album rows while family 2 is absent.
    {
        let raw_db = redb::Database::open(&db.0).unwrap();
        let w = raw_db.begin_write().unwrap();
        {
            w.open_table(INDEX_TABLE)
                .unwrap()
                .insert(("UserData", 2), raw_index.as_slice())
                .unwrap();
            w.open_table(PHOTO_TABLE)
                .unwrap()
                .insert(("UserData", 2, 1), raw_photo.as_slice())
                .unwrap();
        }
        w.commit().unwrap();
    }

    // Creating a replacement household with family_id = 2 via
    // `save_characters_and_neighborhood_checked` must atomically purge the
    // preexisting orphan rows before committing.
    let store = NeighborhoodStore::open(&db.0).unwrap();
    let mut with_replacement = store.load_neighborhood("UserData").unwrap().unwrap();
    with_replacement.families.insert(2, fam2_record);
    store
        .save_characters_and_neighborhood_checked(
            "UserData",
            with_replacement.instance_id,
            with_replacement.revision,
            [("User00042.iff", b"replacement-char".as_slice())],
            &with_replacement,
        )
        .unwrap()
        .unwrap();

    let albums = store.albums();
    assert!(albums.list("UserData", 2).unwrap().is_empty());
    let new_tok2 = albums.family_token("UserData", 2).unwrap().unwrap();
    let fresh = albums
        .add("UserData", new_tok2, NewAlbumPhoto::blank("Fresh", "Clean"))
        .unwrap();
    assert_eq!(fresh.id, 1);

    // 2. Neighborhood IDs longer than 1,024 bytes remain readable and writable.
    let long_id = "UserData_".to_owned() + &"x".repeat(1200);
    store
        .save_neighborhood(&long_id, &sample_neighborhood(&long_id, &[1]))
        .unwrap();
    let long_tok = albums.family_token(&long_id, 1).unwrap().unwrap();
    let p1 = albums
        .add(
            &long_id,
            long_tok,
            NewAlbumPhoto::with_image("Long1", "Cap1", sample_image(2, 2, 5)),
        )
        .unwrap();
    let p2 = albums
        .add(&long_id, long_tok, NewAlbumPhoto::blank("Long2", "Cap2"))
        .unwrap();
    albums
        .update_text(&long_id, long_tok, p1.id, "Long1-Edited", "Cap1-Edited")
        .unwrap();
    assert_eq!(
        albums.move_entry(&long_id, long_tok, p2.id, 0).unwrap(),
        vec![p2.id, p1.id]
    );
    let listed = albums.list(&long_id, 1).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].title, "Long2");
    assert_eq!(listed[1].title, "Long1-Edited");
    assert!(albums.delete(&long_id, long_tok, p2.id).unwrap());
    assert!(albums.delete_family_album(&long_id, long_tok).unwrap());
    assert!(albums.list(&long_id, 1).unwrap().is_empty());
}

#[test]
fn album_v2_roundtrip_compression_and_v1_backwards_compatibility() {
    use redb::ReadableDatabase;
    use sha2::{Digest, Sha256};

    let db = TempDb::new("album_v1_v2_compat");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("UserData", &sample_neighborhood("UserData", &[1]))
        .unwrap();
    let albums = store.albums();
    let tok = albums.family_token("UserData", 1).unwrap().unwrap();

    // Realistic 128x96 RGB565 image with high-bit values (including magenta 0xf81f)
    // that postcard V1 varint-inflated to 3 bytes/pixel.
    let mut pixels = Vec::with_capacity(128 * 96);
    for y in 0..96u16 {
        for x in 0..128u16 {
            pixels.push(if (x + y) % 8 == 0 {
                0xf81f
            } else {
                0x8410 | ((x & 31) << 11) | ((y & 63) << 5)
            });
        }
    }
    let img = AlbumImage::new(128, 96, pixels).unwrap();
    let added = albums
        .add(
            "UserData",
            tok,
            NewAlbumPhoto::with_image("V2 Shot", "Compressed raw LE RGB565", img.clone())
                .with_sim_day(4),
        )
        .unwrap();
    assert_eq!(
        albums.get("UserData", 1, added.id).unwrap().unwrap().image,
        Some(img.clone())
    );
    let summaries = albums.list_summaries("UserData", 1).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].image_dimensions, Some((128, 96)));
    drop(albums);
    drop(store);

    // Measure stored V2 size vs raw V1 varint-encoded size, then inject a genuine
    // V1 record into photo #2 and verify bit-exact backwards-compatible reading.
    const PHOTO_TABLE: redb::TableDefinition<(&str, u16, u64), &[u8]> =
        redb::TableDefinition::new("photo_album_photos");
    const INDEX_TABLE: redb::TableDefinition<(&str, u16), &[u8]> =
        redb::TableDefinition::new("photo_album_index");

    #[derive(serde::Serialize)]
    struct LegacyPhotoV1 {
        neighborhood_id: String,
        family_id: u16,
        id: u64,
        title: String,
        caption: String,
        sim_day: Option<u32>,
        image: Option<AlbumImage>,
    }
    #[derive(serde::Serialize)]
    struct LegacyIndexV1 {
        neighborhood_id: String,
        family_id: u16,
        next_photo_id: u64,
        order: Vec<u64>,
    }

    let v1_body = postcard::to_allocvec(&LegacyPhotoV1 {
        neighborhood_id: "UserData".into(),
        family_id: 1,
        id: 2,
        title: "V1 Legacy".into(),
        caption: "Uncompressed varint V1".into(),
        sim_day: Some(2),
        image: Some(img.clone()),
    })
    .unwrap();
    let mut h = Sha256::new();
    h.update(b"sims-sim:album-photo:v1");
    h.update(1u32.to_le_bytes());
    h.update(&v1_body);
    let mut v1_framed = Vec::new();
    v1_framed.extend_from_slice(b"ALBP");
    v1_framed.extend_from_slice(&1u32.to_le_bytes());
    v1_framed.extend_from_slice(&h.finalize());
    v1_framed.extend_from_slice(&v1_body);

    let idx_body = postcard::to_allocvec(&LegacyIndexV1 {
        neighborhood_id: "UserData".into(),
        family_id: 1,
        next_photo_id: 2,
        order: vec![added.id, 2],
    })
    .unwrap();
    let mut ih = Sha256::new();
    ih.update(b"sims-sim:album-index:v1");
    ih.update(1u32.to_le_bytes());
    ih.update(&idx_body);
    let mut idx_framed = Vec::new();
    idx_framed.extend_from_slice(b"ALBI");
    idx_framed.extend_from_slice(&1u32.to_le_bytes());
    idx_framed.extend_from_slice(&ih.finalize());
    idx_framed.extend_from_slice(&idx_body);

    let raw_db = redb::Database::open(&db.0).unwrap();
    let v2_len = {
        let r = raw_db.begin_read().unwrap();
        let t = r.open_table(PHOTO_TABLE).unwrap();
        t.get(("UserData", 1, added.id))
            .unwrap()
            .unwrap()
            .value()
            .len()
    };
    assert!(
        v2_len * 3 < v1_framed.len(),
        "expected V2 ({v2_len} B) to be >3x smaller than V1 ({} B)",
        v1_framed.len()
    );
    {
        let w = raw_db.begin_write().unwrap();
        {
            let mut pt = w.open_table(PHOTO_TABLE).unwrap();
            pt.insert(("UserData", 1, 2), v1_framed.as_slice()).unwrap();
            let mut it = w.open_table(INDEX_TABLE).unwrap();
            it.insert(("UserData", 1), idx_framed.as_slice()).unwrap();
        }
        w.commit().unwrap();
    }
    drop(raw_db);

    let store = NeighborhoodStore::open(&db.0).unwrap();
    let listed = store.albums().list("UserData", 1).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].image, Some(img.clone()));
    assert_eq!(listed[1].title, "V1 Legacy");
    assert_eq!(listed[1].image, Some(img));
}

#[test]
fn album_v2_rejects_trailing_and_corrupt_deflate_in_both_get_and_list_summaries() {
    use sha2::{Digest, Sha256};

    let db = TempDb::new("album_v2_trailing");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("UserData", &sample_neighborhood("UserData", &[1]))
        .unwrap();
    let albums = store.albums();
    let tok = albums.family_token("UserData", 1).unwrap().unwrap();
    let img = AlbumImage::new(8, 8, vec![0xf81f; 64]).unwrap();
    let added = albums
        .add("UserData", tok, NewAlbumPhoto::with_image("Shot", "", img))
        .unwrap();
    drop(albums);
    drop(store);

    const PHOTO_TABLE: redb::TableDefinition<(&str, u16, u64), &[u8]> =
        redb::TableDefinition::new("photo_album_photos");

    #[derive(serde::Serialize)]
    struct RawImageV2 {
        width: u32,
        height: u32,
        deflated_rgb565_le: Vec<u8>,
    }
    #[derive(serde::Serialize)]
    struct RawPhotoV2 {
        neighborhood_id: String,
        family_id: u16,
        id: u64,
        title: String,
        caption: String,
        sim_day: Option<u32>,
        image: Option<RawImageV2>,
    }

    let sign_v2 = |photo: &RawPhotoV2| -> Vec<u8> {
        let body = postcard::to_allocvec(photo).unwrap();
        let mut h = Sha256::new();
        h.update(b"sims-sim:album-photo:v1");
        h.update(2u32.to_le_bytes());
        h.update(&body);
        let mut framed = Vec::new();
        framed.extend_from_slice(b"ALBP");
        framed.extend_from_slice(&2u32.to_le_bytes());
        framed.extend_from_slice(&h.finalize());
        framed.extend_from_slice(&body);
        framed
    };

    // Valid compressed bytes for 8x8 RGB565 + trailing junk bytes.
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut enc, &[0x1f; 128]).unwrap();
    let mut with_trailing = enc.finish().unwrap();
    with_trailing.extend_from_slice(b"TRAILING");

    for bad_stream in [with_trailing, vec![0xff, 0x00, 0xde, 0xad]] {
        let framed = sign_v2(&RawPhotoV2 {
            neighborhood_id: "UserData".into(),
            family_id: 1,
            id: added.id,
            title: "Bad".into(),
            caption: String::new(),
            sim_day: None,
            image: Some(RawImageV2 {
                width: 8,
                height: 8,
                deflated_rgb565_le: bad_stream,
            }),
        });
        {
            let raw_db = redb::Database::open(&db.0).unwrap();
            let w = raw_db.begin_write().unwrap();
            {
                let mut pt = w.open_table(PHOTO_TABLE).unwrap();
                pt.insert(("UserData", 1, added.id), framed.as_slice())
                    .unwrap();
            }
            w.commit().unwrap();
        }
        let store = NeighborhoodStore::open(&db.0).unwrap();
        let albums = store.albums();
        assert!(matches!(
            albums.get("UserData", 1, added.id),
            Err(AlbumError::DecodeFailed { .. })
        ));
        assert!(matches!(
            albums.list_summaries("UserData", 1),
            Err(AlbumError::DecodeFailed { .. })
        ));
        drop(albums);
        drop(store);
    }
}
