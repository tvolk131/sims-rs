//! Unit and `SIMS_INSTALL` integration tests for `NeighborhoodStore`.

use sims_content::{Catalog, Index, Install};
use sims_formats::neighborhood::Family;
use sims_sim::characters::{Character, Characters};
use sims_sim::neighbors::{Inventory, Item, Neighbor, PERSON_WORDS, Row};
use sims_sim::{
    DEFAULT_ARCH_VALUE_MULTIPLIER, FAMILY_FLAG_BIT_3, FAMILY_FLAG_HAS_CATEGORY_1,
    FAMILY_FLAG_HAS_CATEGORY_9, FAMILY_FLAG_NEW_MOVE_IN, MagicHistory, NEIGHBORHOOD_FORMAT,
    NeighborhoodStore, StoredNeighborhood, TypeAttributes, World, family_net_worth,
    lot_total_value,
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
            "sims-sim-nh-{label}-{}-{n}.redb",
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

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping install-backed neighborhood store test");
            None
        }
    }
}

fn sample_family(house: i32, budget: i32, arch: i32, members: Vec<u32>) -> Family {
    let mut tail = vec![0u8; 20];
    tail[0..4].copy_from_slice(&0i32.to_le_bytes());
    Family {
        version: 9,
        house,
        number: 1,
        budget,
        architecture_value: arch,
        friends: 0,
        flags: 0,
        members,
        tail,
    }
}

fn sample_neighborhood(name: &str) -> StoredNeighborhood {
    let mut families = BTreeMap::new();
    families.insert(5, sample_family(5, 2542, 33116, vec![0x1111, 0x2222]));
    families.insert(2, sample_family(0, 20000, 0, vec![0x3333]));
    let mut zoning = BTreeMap::new();
    zoning.insert(5, 1);
    zoning.insert(4, 1);
    zoning.insert(21, 2);
    StoredNeighborhood {
        name: name.to_owned(),
        families,
        neighbors: vec![None],
        inventories: Vec::new(),
        data: [0; 16],
        options: Some([
            false, true, false, true, false, true, false, true, false, true, false,
        ]),
        type_attributes: None,
        zoning,
        lot_rows: vec![4, 5, 21],
        made_characters: Characters::default(),
        deleted_characters: BTreeSet::new(),
        instance_id: 1,
        revision: 0,
    }
}

#[test]
fn lot_total_value_and_net_worth_match_sims_exe_191_formula() {
    // Verified against Complete Collection shipped UserData FAMI vs SIMI:
    // House07: lot=6000 obj=6129 arch=7281 -> 6000 + 6129 + trunc(7281 * 0.7f) = 17225
    assert_eq!(
        lot_total_value(6000, 6129, 7281, DEFAULT_ARCH_VALUE_MULTIPLIER),
        17225
    );
    // House56: lot=4500 obj=16252 arch=9639 -> 27499
    assert_eq!(
        lot_total_value(4500, 16252, 9639, DEFAULT_ARCH_VALUE_MULTIPLIER),
        27499
    );
    // House77: lot=7000 obj=78210 arch=66685 -> 131889
    assert_eq!(
        lot_total_value(7000, 78210, 66685, DEFAULT_ARCH_VALUE_MULTIPLIER),
        131889
    );
    // Unoccupied House02: lot=11500 obj=0 arch=56110 -> 11500 + trunc(56110.0f32 * 0.7f32) = 50777
    assert_eq!(
        lot_total_value(11500, 0, 56110, DEFAULT_ARCH_VALUE_MULTIPLIER),
        50777
    );
    // Exact x87 fild + fmul f32 rounding (0x425408): 16_777_217 loses its LSB
    // if prematurely narrowed to f32 before multiplying by 0.7f, and
    // 830_472_197 * 0.7f double-rounds to 581_330_560 via f64 instead of the
    // single-rounded exact product 581_330_496.
    assert_eq!(
        lot_total_value(0, 0, 16_777_217, DEFAULT_ARCH_VALUE_MULTIPLIER),
        11_744_052
    );
    assert_eq!(
        lot_total_value(0, 0, 830_472_197, DEFAULT_ARCH_VALUE_MULTIPLIER),
        581_330_496
    );
    let fam = sample_family(5, 2542, 33116, vec![0x1111]);
    assert_eq!(family_net_worth(&fam), 35658);
}

#[test]
fn stored_neighborhood_decode_rejects_short_or_wrong_version() {
    assert!(StoredNeighborhood::decode("bad", &[1, 2]).is_err());
    let mut wrong = sample_neighborhood("UserData").encode();
    wrong[0..4].copy_from_slice(&(NEIGHBORHOOD_FORMAT + 99).to_le_bytes());
    let err = StoredNeighborhood::decode("UserData", &wrong)
        .unwrap_err()
        .to_string();
    assert!(err.contains("neighborhood format"));
}

#[test]
fn multi_neighborhood_crud_and_active_switching_round_trip() {
    let db = TempDb::new("crud");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    assert!(store.list_neighborhoods().unwrap().is_empty());
    assert_eq!(store.active_neighborhood().unwrap(), None);

    let nh1 = sample_neighborhood("UserData");
    let nh2 = sample_neighborhood("UserData2");
    store.save_neighborhood("UserData", &nh1).unwrap();
    store.save_neighborhood("UserData2", &nh2).unwrap();
    store
        .save_character_file("UserData", "User00001.iff", b"fake-char")
        .unwrap();
    store.save_base_house("UserData", 5, b"fake-house").unwrap();
    store
        .save_lot_and_neighborhood("UserData", 5, b"saved-lot-bytes", &nh1)
        .unwrap();

    assert_eq!(
        store.list_neighborhoods().unwrap(),
        vec!["UserData".to_owned(), "UserData2".to_owned()]
    );
    assert!(store.set_active_neighborhood("Missing").is_err());
    store.set_active_neighborhood("UserData2").unwrap();
    assert_eq!(
        store.active_neighborhood().unwrap().as_deref(),
        Some("UserData2")
    );
    store.set_active_neighborhood("UserData").unwrap();
    assert_eq!(
        store.load_neighborhood("UserData").unwrap().as_ref(),
        Some(&nh1)
    );
    assert_eq!(
        store.load_saved_lot("UserData", 5).unwrap().as_deref(),
        Some(b"saved-lot-bytes".as_slice())
    );
    assert_eq!(store.list_houses("UserData").unwrap(), vec![5]);
    assert_eq!(store.load_character_files("UserData").unwrap().len(), 1);

    assert!(store.delete_neighborhood("UserData").unwrap());
    assert!(!store.delete_neighborhood("UserData").unwrap());
    assert_eq!(store.active_neighborhood().unwrap(), None);
    assert_eq!(
        store.list_neighborhoods().unwrap(),
        vec!["UserData2".to_owned()]
    );
    assert_eq!(store.load_saved_lot("UserData", 5).unwrap(), None);
    assert!(store.load_character_files("UserData").unwrap().is_empty());
}

#[test]
fn write_back_world_flushes_fami_nbrs_ngbh_tatt_and_characters() {
    let mut stored = sample_neighborhood("UserData");
    stored.families.get_mut(&5).unwrap().flags = FAMILY_FLAG_BIT_3 | FAMILY_FLAG_NEW_MOVE_IN;

    let mut world = World::empty();
    world.budget.funds = 4321;
    world.globals[26] = 1;
    world.neighborhood_data[0] = 77;
    world.magic_history = Some(MagicHistory {
        family: 5,
        values: [11, 22, 33, 44, 55, 66],
    });
    world.families = vec![sims_sim::neighbors::Family {
        id: 5,
        house: 5,
        members: vec![0x1111, 0x2222],
        friends: 0,
        stale: true,
    }];
    let mut d1 = vec![0; PERSON_WORDS];
    d1[10] = 500; // Cooking skill
    d1[61] = 5;
    let mut d2 = vec![0; PERSON_WORDS];
    d2[61] = 2;
    world.neighbors.slots = vec![
        Some(Neighbor {
            id: 1,
            guid: 0x1111,
            revision: 9,
            data: d1,
            relationships: vec![Row {
                target: 2,
                values: vec![75, 40],
            }],
            friends: 0,
            stale: true,
        }),
        Some(Neighbor {
            id: 2,
            guid: 0x3333,
            revision: 9,
            data: d2,
            relationships: vec![Row {
                target: 1,
                values: vec![80, 50],
            }],
            friends: 0,
            stale: true,
        }),
    ];
    world.neighbors.inventories.push(Inventory {
        neighbor: 1,
        items: vec![Item {
            kind: 3,
            guid: 0xABCD,
            count: 2,
        }],
    });
    world.type_attributes.push(TypeAttributes {
        guid: 0x9999,
        values: vec![1, 2, 3],
    });
    world.characters.made.insert(
        0xCAFE,
        Character {
            template: 0xDEAD,
            file: 42,
            strings: BTreeMap::from([(12, "female".to_owned())]),
        },
    );
    world.characters.next_file = 43;

    stored.write_back_world(&world, 5, 5, 41000, Some((true, true)), 50);

    let fam5 = &stored.families[&5];
    assert_eq!(fam5.budget, 4321);
    assert_eq!(fam5.architecture_value, 41000);
    assert_eq!(
        fam5.friends, 1,
        "mutual 75/80 >= 50 counts as 1 family friend"
    );
    assert_eq!(
        fam5.flags,
        FAMILY_FLAG_HAS_CATEGORY_1 | FAMILY_FLAG_HAS_CATEGORY_9
    );
    assert_eq!(
        fam5.magic_history().unwrap(),
        Some([11, 22, 33, 44, 55, 66])
    );
    assert_eq!(stored.data[0], 77);
    assert_eq!(
        stored.data[2], 5,
        "global 26 writes current house to NGBH data[2]"
    );
    assert_eq!(stored.inventories.len(), 1);
    assert_eq!(stored.neighbors.len(), 2);
    assert_eq!(stored.neighbors[0].as_ref().unwrap().person_data[10], 500);
    assert_eq!(
        stored.type_attributes.as_ref().unwrap().entries[0].values,
        vec![1, 2, 3]
    );
    assert!(stored.made_characters.made.contains_key(&0xCAFE));
    assert_eq!(stored.made_characters.next_file, 43);
    assert_eq!(stored.revision, 1);
}

#[test]
fn import_userdata_and_userdata2_without_modifying_install_and_match_catalog() {
    let Some(root) = install() else {
        return;
    };
    let ud1 = root.join("UserData");
    let ud2 = root.join("UserData2");
    let before_nh_bytes = std::fs::read(ud1.join("Neighborhood.iff")).unwrap();

    let db = TempDb::new("import-multi");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    let s1 = store.import_directory("UserData", &ud1, 0xff).unwrap();
    let s2 = store.import_directory("UserData2", &ud2, 0xff).unwrap();
    assert!(s1.families > 5 && s1.houses > 10 && s1.characters > 10);
    assert!(s2.families >= 2 && s2.houses > 10);
    assert_eq!(
        store.list_neighborhoods().unwrap(),
        vec!["UserData".to_owned(), "UserData2".to_owned()]
    );
    assert_eq!(
        store.active_neighborhood().unwrap().as_deref(),
        Some("UserData")
    );
    store.set_active_neighborhood("UserData2").unwrap();
    assert_eq!(
        store.active_neighborhood().unwrap().as_deref(),
        Some("UserData2")
    );

    // Install files remain strictly unmodified.
    assert_eq!(
        std::fs::read(ud1.join("Neighborhood.iff")).unwrap(),
        before_nh_bytes
    );

    // Inline character mount reproduces the exact same Catalog GUID order as disk.
    let install = Install::detect(&root).unwrap();
    let mut disk_index = Index::build(&install).unwrap();
    disk_index.add_characters(&ud1.join("Characters")).unwrap();
    let disk_catalog = Catalog::build(&disk_index).unwrap();

    let mut redb_index = Index::build(&install).unwrap();
    let chars = store.load_character_files("UserData").unwrap();
    redb_index.add_character_files(&chars).unwrap();
    let redb_catalog = Catalog::build(&redb_index).unwrap();

    let disk_guids: Vec<u32> = disk_catalog.objects.iter().map(|o| o.guid).collect();
    let redb_guids: Vec<u32> = redb_catalog.objects.iter().map(|o| o.guid).collect();
    assert_eq!(redb_guids, disk_guids);

    // Verify shipped lot prices against Neighborhood.iff FAMI.architecture_value.
    let nh1 = store.load_neighborhood("UserData").unwrap().unwrap();
    for house in [7u32, 56, 63, 69, 76, 77] {
        let (_, fam) = nh1.family_in(house).unwrap();
        let price = store
            .base_house_lot_price("UserData", house, DEFAULT_ARCH_VALUE_MULTIPLIER)
            .unwrap()
            .unwrap();
        assert_eq!(price, fam.architecture_value, "house {house}");
    }
    assert_eq!(
        store
            .base_house_lot_price("UserData", 4, DEFAULT_ARCH_VALUE_MULTIPLIER)
            .unwrap(),
        Some(5500)
    );
    assert_eq!(
        store
            .base_house_lot_price("UserData", 2, DEFAULT_ARCH_VALUE_MULTIPLIER)
            .unwrap(),
        Some(50777)
    );
}

#[test]
fn write_back_world_upgrades_v7_family_and_merges_magic_controller_attributes() {
    let mut stored = sample_neighborhood("UserData2");
    // Construct a pre-Makin'-Magic v7 family (empty tail, no magic_history yet).
    stored.families.insert(
        5,
        Family {
            version: 7,
            house: 5,
            number: 1,
            budget: 5000,
            architecture_value: 10000,
            friends: 0,
            flags: 0,
            members: vec![0x1111],
            tail: Vec::new(),
        },
    );

    let mut world = World::empty();
    world.budget.funds = 4000;
    world.globals[9] = 5;
    world.magic_history = Some(MagicHistory {
        family: 5,
        values: [10, 500, 30, 40, 50, 60],
    });
    // Place a Magic Controller (`0xb6c90029`) with mixed higher & lower attributes.
    let controller = sims_sim::Object {
        id: 1,
        guid: 0xb6c9_0029,
        footprint: [0; 4],
        position: [-16, -16, 1],
        unknown_short: 0,
        attributes: vec![100, 200, 300, 10, 500, 600],
        temps: Vec::new(),
        data: Vec::new(),
        obsolete_word: 0,
        stack_capacity_bytes: 0,
        stack: Vec::new(),
        relationship_flag: 0,
        relationships: Vec::new(),
        slots: Vec::new(),
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: None,
        person: None,
        registry_word: None,
        last_result: false,
        removal_skips_resets: false,
        selected: false,
    };
    world.objects.insert(1, controller);
    world.order.push(1);

    stored.write_back_world(&world, 5, 5, 12000, None, 50);

    let fam5 = &stored.families[&5];
    assert_eq!(fam5.version, 9, "writeback always upgrades families to v9");
    assert_eq!(
        fam5.magic_history().unwrap(),
        Some([100, 500, 300, 40, 500, 600]),
        "signed max of retained history and Magic Controller attributes 0..6 (0x410a87)"
    );
    // Secondary house occupancy check (0x425d95): a family with house == 0 and
    // secondary_house != 0 occupies that house AND is excluded from family_bin().
    assert!(stored.family_bin().iter().any(|&(id, _)| id == 2));
    let fam2 = stored.families.get_mut(&2).unwrap();
    fam2.set_secondary_house(4);
    assert_eq!(stored.family_in(4).map(|(id, _)| id), Some(2));
    assert!(
        stored.family_bin().iter().all(|&(id, _)| id != 2),
        "secondary-only occupants must not appear in family_bin()"
    );
}

#[test]
fn list_houses_propagates_non_missing_table_redb_errors() {
    let db = TempDb::new("bad-houses-schema");
    {
        // Create `neighborhood_houses` with an incompatible key type (&str instead of (&str, u32)).
        let raw = redb::Database::create(&db.0).unwrap();
        let write = raw.begin_write().unwrap();
        {
            const WRONG: redb::TableDefinition<&str, &[u8]> =
                redb::TableDefinition::new("neighborhood_houses");
            let mut table = write.open_table(WRONG).unwrap();
            table.insert("wrong", b"schema".as_slice()).unwrap();
        }
        write.commit().unwrap();
    }
    let store = NeighborhoodStore::open(&db.0).unwrap();
    assert!(
        store.list_houses("UserData").is_err(),
        "list_houses must propagate schema/corruption errors rather than returning Ok([])"
    );
}

#[test]
fn add_character_files_matches_disk_for_subdirs_far_and_win1252_names() {
    let Some(root) = install() else {
        return;
    };
    let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("sims-sim-char-parity-{}-{n}", std::process::id()));
    let sub = dir.join("SubDir");
    std::fs::create_dir_all(&sub).unwrap();

    // Grab three real character IFFs from UserData/Characters and pack one into
    // a FAR v1 archive and one into a nested subdirectory with a Windows-1252
    // accented filename.
    let ud_nh = sims_content::Neighborhood::open(&root.join("UserData")).unwrap();
    let ud_chars = ud_nh.character_tree().unwrap();
    let real_iffs: Vec<PathBuf> = ud_chars
        .into_iter()
        .filter(|(rel, _)| rel.to_ascii_lowercase().ends_with(".iff"))
        .map(|(_, path)| path)
        .collect();
    let bytes_a = std::fs::read(&real_iffs[0]).unwrap();
    let bytes_b = std::fs::read(&real_iffs[1]).unwrap();
    let bytes_c = std::fs::read(&real_iffs[2]).unwrap();

    std::fs::write(dir.join("caf\u{00e9}.iff"), &bytes_a).unwrap();
    std::fs::write(sub.join("nested.iff"), &bytes_b).unwrap();

    // Build a valid FAR v1b archive containing `archived.iff`.
    let member_name = b"archived.iff";
    let header_len = 8 + 4 + 4; // magic + version + manifest_offset
    let data_offset = header_len as u32;
    let manifest_offset = data_offset + bytes_c.len() as u32;
    let mut far = Vec::new();
    far.extend_from_slice(b"FAR!byAZ");
    far.extend_from_slice(&1u32.to_le_bytes());
    far.extend_from_slice(&manifest_offset.to_le_bytes());
    far.extend_from_slice(&bytes_c);
    far.extend_from_slice(&1u32.to_le_bytes()); // file count
    far.extend_from_slice(&(bytes_c.len() as u32).to_le_bytes());
    far.extend_from_slice(&(bytes_c.len() as u32).to_le_bytes());
    far.extend_from_slice(&data_offset.to_le_bytes());
    far.extend_from_slice(&(member_name.len() as u32).to_le_bytes());
    far.extend_from_slice(member_name);
    std::fs::write(dir.join("pack.far"), &far).unwrap();
    // An empty directory with the same name as a FAR member must hide that
    // member in both the disk index and the inline index.
    std::fs::create_dir_all(dir.join("archived.iff")).unwrap();

    let install = Install::detect(&root).unwrap();
    let mut disk_index = Index::build(&install).unwrap();
    disk_index.add_characters(&dir).unwrap();
    let disk_catalog = Catalog::build(&disk_index).unwrap();

    let mut files = BTreeMap::new();
    files.insert("caf\u{00e9}.iff".to_owned(), bytes_a);
    files.insert("SubDir/nested.iff".to_owned(), bytes_b);
    files.insert("pack.far".to_owned(), far);
    files.insert("archived.iff/".to_owned(), Vec::new());
    let mut inline_index = Index::build(&install).unwrap();
    inline_index.add_character_files(&files).unwrap();
    let inline_catalog = Catalog::build(&inline_index).unwrap();

    let disk_guids: Vec<u32> = disk_catalog.objects.iter().map(|o| o.guid).collect();
    let inline_guids: Vec<u32> = inline_catalog.objects.iter().map(|o| o.guid).collect();
    assert_eq!(inline_guids, disk_guids);
    let _ = std::fs::remove_dir_all(&dir);
}
