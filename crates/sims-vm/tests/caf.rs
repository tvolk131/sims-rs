//! Unit, oracle cross-check, and `SIMS_INSTALL` end-to-end integration tests
//! for `sims_vm::caf` (Create-a-Family character synthesis, Family Bin commit &
//! deletion, `.FAM` export/import, and Move-In -> 500-tick lot simulation).

use sims_core::CrtRng;
use sims_formats::bitmap::bmp;
use sims_formats::family_file::FamilyFile;
use sims_formats::iff::{ChunkData, Iff, write as write_iff};
use sims_formats::neighborhood::{Family, Inventory, InventoryItem, Neighbors, Relationship};
use sims_formats::strings::StringTable;
use sims_sim::characters::Characters;
use sims_sim::{FAMILY_FLAG_NEW_MOVE_IN, NeighborhoodStore, StoredNeighborhood};
use sims_vm::boot::Frames;
use sims_vm::caf::{
    Age, CAF_INITIAL_FAMILY_FLAGS, CafContext, CafError, DEFAULT_STARTING_FUNDS,
    FAMILY_FLAG_CAF_CREATED, FamilySpec, Gender, Personality, PortraitBitmaps, PortraitKind,
    SimSpec, SkinTone, TEMPLATE_PERSON_GUID, allocate_family_id, commit_family,
    commit_family_with_context, compute_zodiac, delete_bin_family, encode_bmp24, encode_fami,
    encode_fams, export_family_fam, import_family_fam, person_word, synthesize_character_iff,
    synthesize_neighbor, zodiac_name,
};
use sims_vm::lifecycle::{enter_lot, move_in, save_and_leave_lot};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempDb(PathBuf);

impl TempDb {
    fn new(label: &str) -> Self {
        let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sims-vm-caf-{label}-{}-{n}.redb",
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
            eprintln!("SIMS_INSTALL is not set; skipping install-backed CAF test");
            None
        }
    }
}

fn caf_oracle_dir() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("SIMS_CAF_ORACLE").map(PathBuf::from),
        Some(PathBuf::from(
            "/Users/tommyvolk/Desktop/sims-rs/cache/caf-create-family-20261121",
        )),
    ];
    for candidate in candidates.into_iter().flatten() {
        if candidate.join("manifest.json").is_file() {
            return Some(candidate);
        }
    }
    eprintln!("CAF oracle directory not present; skipping oracle byte-for-field test");
    None
}

fn alice_sim_spec(portraits: Option<PortraitBitmaps>) -> SimSpec {
    SimSpec {
        first_name: "Alice".into(),
        bio: "Track 4 Create-a-Family oracle Sim.".into(),
        age: Age::Adult,
        gender: Gender::Female,
        skin_tone: SkinTone::Medium,
        body_appearance: "b002fafit_01".into(),
        body_texture: "b002fafitmed_mom02".into(),
        head_appearance: "c809fa_beth".into(),
        head_texture: "c809famed_beth3".into(),
        personality: Personality::new(7, 5, 3, 4, 6).unwrap(),
        portraits,
    }
}

fn synthetic_template_iff() -> Iff {
    let mut objd = vec![0u8; 216];
    objd[0..4].copy_from_slice(&138u32.to_le_bytes());
    objd[0x12..0x14].copy_from_slice(&2u16.to_le_bytes());
    objd[0x1c..0x20].copy_from_slice(&TEMPLATE_PERSON_GUID.to_le_bytes());
    objd[0x26..0x28].copy_from_slice(&200u16.to_le_bytes());
    objd[0x52..0x54].copy_from_slice(&2000u16.to_le_bytes());
    let bytes = write_iff(&[
        ChunkData {
            tag: *b"OBJD",
            id: 128,
            flags: 0x10,
            label: "TemplatePerson".to_owned(),
            data: objd,
        },
        ChunkData {
            tag: *b"STR#",
            id: 200,
            flags: 0x10,
            label: "bodystring".to_owned(),
            data: vec![0xfd, 0xff, 1, 0, 1, b'a', b'd', b'u', b'l', b't', 0, 0, 0],
        },
    ])
    .unwrap();
    Iff::parse(bytes).unwrap()
}

fn empty_stored_neighborhood(name: &str) -> StoredNeighborhood {
    let mut families = BTreeMap::new();
    families.insert(
        0,
        Family {
            version: 9,
            house: 0,
            number: -1,
            budget: 0,
            architecture_value: 0,
            friends: 0,
            flags: 0,
            members: Vec::new(),
            tail: vec![0; 16],
        },
    );
    StoredNeighborhood {
        name: name.into(),
        families,
        neighbors: Vec::new(),
        inventories: Vec::new(),
        data: [0; 16],
        options: None,
        type_attributes: None,
        zoning: BTreeMap::from([(4, 1)]),
        lot_rows: vec![4],
        made_characters: Characters::default(),
        deleted_characters: BTreeSet::new(),
        deletion_epoch: 0,
        deleted_at_epoch: BTreeMap::new(),
        instance_id: 1,
        revision: 0,
    }
}

#[test]
fn validation_rejects_empty_names_child_only_over_eight_and_over_budget_traits() {
    let adult = alice_sim_spec(None);
    let mut child = alice_sim_spec(None);
    child.first_name = "Timmy".into();
    child.age = Age::Child;
    child.gender = Gender::Male;
    child.body_appearance = "b001mcchd_01".into();
    child.body_texture = "b001mcchdmed_boy01".into();
    child.head_appearance = "c001mc_ave".into();
    child.head_texture = "c001mcmed_ave".into();

    // Empty family name
    assert!(matches!(
        FamilySpec::new("   ", "", vec![adult.clone()]).validate(),
        Err(CafError::EmptyFamilyName)
    ));
    // Zero members
    assert!(matches!(
        FamilySpec::new("Smith", "", vec![]).validate(),
        Err(CafError::InvalidMemberCount { count: 0 })
    ));
    // More than 8 members
    assert!(matches!(
        FamilySpec::new("Big", "", vec![adult.clone(); 9]).validate(),
        Err(CafError::InvalidMemberCount { count: 9 })
    ));
    // Child-only household (must have >= 1 adult)
    assert!(matches!(
        FamilySpec::new("Kids", "", vec![child.clone()]).validate(),
        Err(CafError::NoAdultMember)
    ));
    // Empty Sim first name
    let mut unnamed = adult.clone();
    unnamed.first_name = " ".into();
    assert!(matches!(
        FamilySpec::new("Smith", "", vec![unnamed]).validate(),
        Err(CafError::EmptyFirstName { member: 0 })
    ));
    // Personality trait > 10 or total > 25
    assert!(matches!(
        Personality::new(11, 0, 0, 0, 0),
        Err(CafError::InvalidPersonality(_))
    ));
    assert!(matches!(
        Personality::new(10, 10, 6, 0, 0),
        Err(CafError::InvalidPersonality(_))
    ));
    // Valid 1 adult + 7 children (max 8) passes
    let mut full = vec![adult];
    full.extend(std::iter::repeat_n(child, 7));
    FamilySpec::new("FullHouse", "Eight Sims.", full)
        .validate()
        .unwrap();
}

#[test]
fn synthetic_commit_delete_tombstone_and_fam_round_trip() {
    let db = TempDb::new("synthetic-commit-delete");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    let nh = empty_stored_neighborhood("SyntheticNH");
    store.save_neighborhood("SyntheticNH", &nh).unwrap();

    let ctx = CafContext::synthetic(
        synthetic_template_iff(),
        BTreeSet::from([TEMPLATE_PERSON_GUID, 0x1111_2222]),
        DEFAULT_STARTING_FUNDS,
    );
    let mut child = alice_sim_spec(None);
    child.first_name = "Ben".into();
    child.bio = "Younger sibling.".into();
    child.age = Age::Child;
    child.gender = Gender::Male;
    child.skin_tone = SkinTone::Light;
    child.personality = Personality::new(3, 8, 6, 5, 3).unwrap();

    let spec = FamilySpec::new(
        "Alice",
        "Synthetic household",
        vec![alice_sim_spec(None), child],
    );
    let mut rng = CrtRng::from_state(2026_1121);
    let created = commit_family_with_context(&ctx, &store, "SyntheticNH", &spec, &mut rng).unwrap();

    assert_eq!(created.family_id, 1);
    assert_eq!(created.family.budget, DEFAULT_STARTING_FUNDS);
    assert_eq!(created.family.house, 0);
    assert_eq!(created.family.flags, CAF_INITIAL_FAMILY_FLAGS);
    assert_eq!(created.family.number, 1);
    assert_eq!(created.members.len(), 2);
    assert_ne!(created.members[0].guid, created.members[1].guid);
    assert_eq!(created.members[0].file_name, "User00000.iff");
    assert_eq!(created.members[1].file_name, "User00001.iff");
    assert_eq!(created.members[0].zodiac, 4); // Cancer
    // Child gets school career (-1) and starting grade (4 = C).
    assert_eq!(created.members[1].neighbor.person_data[person_word::AGE], 9);
    assert_eq!(
        created.members[1].neighbor.person_data[person_word::JOB],
        -1
    );
    assert_eq!(
        created.members[1].neighbor.person_data[person_word::JOB_LEVEL_OR_GRADE],
        4
    );

    // Appears in Family Bin immediately.
    let stored = store.load_neighborhood("SyntheticNH").unwrap().unwrap();
    let bin = stored.family_bin();
    assert_eq!(bin.len(), 1);
    assert_eq!(bin[0].0, created.family_id);

    // Seed intra-family relationships, an external relationship, architecture_value,
    // and non-empty inventory items on family 1 before exporting to .FAM so export,
    // import remapping, and delete_bin_family scrubbing are all genuinely verified.
    {
        let mut seeded = store.load_neighborhood("SyntheticNH").unwrap().unwrap();
        seeded.families.get_mut(&1).unwrap().architecture_value = 3_450;
        let id0 = created.members[0].neighbor_id;
        let id1 = created.members[1].neighbor_id;
        // Member 0 -> Member 1 (with duplicate row to verify first-row deduplication) + external 99.
        seeded.neighbors[(id0 - 1) as usize]
            .as_mut()
            .unwrap()
            .relationships = vec![
            Relationship {
                target: i32::from(id1),
                values: vec![75, 50],
            },
            Relationship {
                target: i32::from(id1),
                values: vec![12, 10],
            },
            Relationship {
                target: 99,
                values: vec![40, 20],
            },
        ];
        seeded.neighbors[(id1 - 1) as usize]
            .as_mut()
            .unwrap()
            .relationships = vec![Relationship {
            target: i32::from(id0),
            values: vec![80, 60],
        }];
        seeded.inventories.push(Some(Inventory {
            neighbor: id0,
            items: vec![InventoryItem {
                kind: 2,
                guid: 0x1234_abcd,
                count: 3,
            }],
        }));
        store.save_neighborhood("SyntheticNH", &seeded).unwrap();
    }

    // Export to .FAM and re-import as a second household with fresh unique GUIDs.
    let stored = store.load_neighborhood("SyntheticNH").unwrap().unwrap();
    let chars = store.load_character_files("SyntheticNH").unwrap();
    let fam_bytes = export_family_fam(
        &stored,
        &chars,
        created.family_id,
        &created.family_name,
        1,
        true,
    )
    .unwrap();
    let parsed_fam = FamilyFile::parse(&fam_bytes).unwrap();
    assert_eq!(parsed_fam.export_info.family_id, created.family_id);
    assert_eq!(parsed_fam.family.members.len(), 2);
    assert_eq!(parsed_fam.members.len(), 2);

    let reimported =
        import_family_fam(&ctx, &store, "SyntheticNH", &fam_bytes, 1, &mut rng).unwrap();
    // Exported family ID 1 is occupied, so import falls back to lowest free ID (2).
    assert_eq!(reimported.family_id, 2);
    assert_eq!(reimported.family_name, "Alice");
    assert_eq!(reimported.family.architecture_value, 3_450);
    assert_eq!(reimported.family.flags, 0);
    assert_eq!(reimported.members.len(), 2);
    for imported_sim in &reimported.members {
        assert!(!created.members.iter().any(|m| m.guid == imported_sim.guid));
    }
    // Verify intra-family relationships were remapped & deduplicated, external
    // target 99 was dropped, and FINV #2 inventory items were transferred to the
    // remapped neighbor ID!
    let new_id0 = reimported.members[0].neighbor_id;
    let new_id1 = reimported.members[1].neighbor_id;
    assert_eq!(
        reimported.members[0].neighbor.relationships,
        vec![Relationship {
            target: i32::from(new_id1),
            values: vec![75, 50],
        }]
    );
    assert_eq!(
        reimported.members[1].neighbor.relationships,
        vec![Relationship {
            target: i32::from(new_id0),
            values: vec![80, 60],
        }]
    );
    let after_reimport = store.load_neighborhood("SyntheticNH").unwrap().unwrap();
    let transferred_inv = after_reimport
        .inventories
        .iter()
        .flatten()
        .find(|inv| inv.neighbor == new_id0)
        .expect("FINV #2 inventory must transfer to remapped neighbor ID");
    assert_eq!(
        transferred_inv.items,
        vec![InventoryItem {
            kind: 2,
            guid: 0x1234_abcd,
            count: 3,
        }]
    );

    // Seed a cross-family relationship from family 2 member -> family 1 member 0 so
    // we verify `delete_bin_family` scrubs reciprocal relationships AND inventories!
    {
        let mut with_cross = store.load_neighborhood("SyntheticNH").unwrap().unwrap();
        with_cross.neighbors[(new_id0 - 1) as usize]
            .as_mut()
            .unwrap()
            .relationships
            .push(Relationship {
                target: i32::from(created.members[0].neighbor_id),
                values: vec![55, 30],
            });
        store.save_neighborhood("SyntheticNH", &with_cross).unwrap();
    }

    // Deleting a reserved pool family or housed family is rejected cleanly.
    assert!(matches!(
        delete_bin_family(&store, "SyntheticNH", 0),
        Err(CafError::ReservedFamily { family: 0 })
    ));
    assert!(matches!(
        delete_bin_family(&store, "SyntheticNH", 2000),
        Err(CafError::ReservedFamily { family: 2000 })
    ));
    {
        let mut housed = store.load_neighborhood("SyntheticNH").unwrap().unwrap();
        housed.families.get_mut(&2).unwrap().house = 4;
        store.save_neighborhood("SyntheticNH", &housed).unwrap();
        assert!(matches!(
            delete_bin_family(&store, "SyntheticNH", 2),
            Err(CafError::FamilyNotInBin {
                family: 2,
                house: 4
            })
        ));
    }

    // Delete un-housed Family Bin family 1: tombstones GUIDs, vacates NBRS slots,
    // scrubs reciprocal relationships on family 2 and removes family 1's inventory.
    let deleted = delete_bin_family(&store, "SyntheticNH", 1).unwrap();
    assert_eq!(deleted.family_id, 1);
    assert_eq!(deleted.deletion_epoch, 1);
    assert_eq!(deleted.deleted_guids.len(), 2);
    assert_eq!(deleted.vacated_neighbor_ids, vec![1, 2]);

    let after_delete = store.load_neighborhood("SyntheticNH").unwrap().unwrap();
    assert!(!after_delete.families.contains_key(&1));
    assert!(after_delete.neighbors[0].is_none());
    assert!(after_delete.neighbors[1].is_none());
    assert_eq!(after_delete.deletion_epoch, 1);
    for &guid in &deleted.deleted_guids {
        assert!(after_delete.deleted_characters.contains(&guid));
        assert_eq!(after_delete.deleted_at_epoch.get(&guid), Some(&1));
    }
    // Surviving family 2 member must have its reciprocal relationship to deleted slot 1 scrubbed.
    let surviving_rels = &after_delete.neighbors[(new_id0 - 1) as usize]
        .as_ref()
        .unwrap()
        .relationships;
    assert!(
        surviving_rels
            .iter()
            .all(|r| r.target != i32::from(created.members[0].neighbor_id)),
        "delete_bin_family must scrub reciprocal relationships pointing to deleted members"
    );
    // Deleted family 1's inventory must be removed while surviving family 2's remains intact.
    assert!(
        after_delete
            .inventories
            .iter()
            .flatten()
            .all(|inv| inv.neighbor != created.members[0].neighbor_id)
    );
    assert!(
        after_delete
            .inventories
            .iter()
            .flatten()
            .any(|inv| inv.neighbor == new_id0)
    );

    // Re-importing the same `.FAM` now that family ID 1 & slots 1..=2 are free
    // must retain exported family ID 1 and exported neighbor slots 1 & 2!
    let restored = import_family_fam(&ctx, &store, "SyntheticNH", &fam_bytes, 1, &mut rng).unwrap();
    assert_eq!(restored.family_id, 1);
    assert_eq!(restored.members[0].neighbor_id, 1);
    assert_eq!(restored.members[1].neighbor_id, 2);

    // Concurrent multi-threaded CAF commits sharing `&NeighborhoodStore` must
    // serialize cleanly without overwriting filenames or colliding GUIDs.
    let concurrent_results: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let ctx_ref = &ctx;
                let store_ref = &store;
                scope.spawn(move || {
                    let mut sim = alice_sim_spec(None);
                    sim.first_name = format!("ThreadSim{i}");
                    let fam_spec = FamilySpec::new(format!("Concurrent{i}"), "", vec![sim]);
                    let mut local_rng = CrtRng::from_state(9000 + i as u32);
                    commit_family_with_context(
                        ctx_ref,
                        store_ref,
                        "SyntheticNH",
                        &fam_spec,
                        &mut local_rng,
                    )
                    .unwrap()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let final_chars = store.load_character_files("SyntheticNH").unwrap();
    let mut seen_fams = BTreeSet::new();
    let mut seen_files = BTreeSet::new();
    let mut seen_guids = BTreeSet::new();
    for res in &concurrent_results {
        assert!(seen_fams.insert(res.family_id));
        let sim = &res.members[0];
        assert!(seen_files.insert(sim.file_name.clone()));
        assert!(seen_guids.insert(sim.guid));
        assert_eq!(
            final_chars.get(&sim.file_name).map(Vec::as_slice),
            Some(sim.iff_bytes.as_slice()),
            "concurrent CAF commit must not overwrite another household's character IFF"
        );
    }
}

/// Cross-checks synthesized character IFF (`User00172.iff`), `FAMI 7`, `FAMs 7`,
/// `NBRS 464`, `BMP_ 2002..=2007`, and `Export/Alice_7.FAM` field-for-field
/// against Track 2's Complete Collection 1.91 oracle capture.
#[test]
fn oracle_cross_check_against_alice_capture() {
    let Some(root) = install() else { return };
    let Some(oracle) = caf_oracle_dir() else {
        return;
    };

    let oracle_initial_nh =
        Iff::parse(std::fs::read(oracle.join("00-initial/Neighborhood.iff")).unwrap()).unwrap();
    let oracle_after_nh =
        Iff::parse(std::fs::read(oracle.join("01-after-create/Neighborhood.iff")).unwrap())
            .unwrap();
    let oracle_user =
        Iff::parse(std::fs::read(oracle.join("01-after-create/Characters/User00172.iff")).unwrap())
            .unwrap();
    let oracle_fam_bytes =
        std::fs::read(oracle.join("01-after-create/Export/Alice_7.FAM")).unwrap();
    let oracle_fam = FamilyFile::parse(&oracle_fam_bytes).unwrap();

    // 1. Decode oracle BMP_ 2002..=2007 and verify our `encode_bmp24` reproduces
    //    every single BMP byte-for-byte!
    let mut decoded_imgs = Vec::new();
    for kind in PortraitKind::ALL {
        let chunk = oracle_user.get(b"BMP_", kind.chunk_id()).unwrap();
        assert_eq!(chunk.label, kind.label());
        let raw = oracle_user.data(chunk);
        let img = bmp(raw).unwrap();
        assert_eq!([img.width, img.height], kind.expected_size());
        let reencoded = encode_bmp24(&img).unwrap();
        // Native 54-byte header and total length match verbatim; per-row BGR pixel
        // bytes match verbatim (native Sims.exe leaves uninitialized Win32 heap
        // bytes in 4-byte row-alignment slack for odd-width bitmaps like 105/25/45,
        // whereas `encode_bmp24` zero-fills them cleanly).
        assert_eq!(reencoded.len(), raw.len());
        assert_eq!(
            &reencoded[..54],
            &raw[..54],
            "BMP_ {} ({}) 54-byte header must match User00172.iff",
            kind.chunk_id(),
            kind.label()
        );
        let row_bytes = (img.width as usize) * 3;
        let stride = row_bytes.div_ceil(4) * 4;
        for row in 0..(img.height as usize) {
            let start = 54 + row * stride;
            assert_eq!(
                &reencoded[start..start + row_bytes],
                &raw[start..start + row_bytes],
                "BMP_ {} ({}) row {row} pixels must match User00172.iff",
                kind.chunk_id(),
                kind.label()
            );
        }
        assert_eq!(bmp(&reencoded).unwrap(), img);
        decoded_imgs.push(img);
    }
    let portraits = PortraitBitmaps::from_images(
        &decoded_imgs[0],
        &decoded_imgs[1],
        &decoded_imgs[2],
        &decoded_imgs[3],
        &decoded_imgs[4],
        &decoded_imgs[5],
    )
    .unwrap();

    // 2. Synthesize User00172.iff with Alice's exact GUID (0xe3050120) and file
    //    number (172) using the real install TemplatePerson + ImportWardrobe.
    let ctx = CafContext::from_install(&root).unwrap();
    assert_eq!(ctx.starting_funds, 20_000);
    let alice_spec = alice_sim_spec(Some(portraits));
    let oracle_guid = 0xe305_0120u32;
    let synth_bytes = synthesize_character_iff(
        &ctx.template_iff,
        &alice_spec,
        oracle_guid,
        172,
        1,
        ctx.wardrobe.as_ref(),
    )
    .unwrap();
    let synth_iff = Iff::parse(synth_bytes).unwrap();

    // OBJD 128: exact byte-for-byte match AND label match (`user00172 - Alice`).
    let synth_objd = synth_iff.get(b"OBJD", 128).unwrap();
    let oracle_objd = oracle_user.get(b"OBJD", 128).unwrap();
    assert_eq!(synth_objd.label, oracle_objd.label);
    assert_eq!(synth_iff.data(synth_objd), oracle_user.data(oracle_objd));

    // CTSS 2000: exact length (52) and exact strings [name, bio].
    let synth_ctss_chunk = synth_iff.get(b"CTSS", 2000).unwrap();
    let oracle_ctss_chunk = oracle_user.get(b"CTSS", 2000).unwrap();
    assert_eq!(
        synth_iff.data(synth_ctss_chunk).len(),
        oracle_user.data(oracle_ctss_chunk).len()
    );
    let synth_ctss = StringTable::parse(synth_iff.data(synth_ctss_chunk)).unwrap();
    let oracle_ctss = StringTable::parse(oracle_user.data(oracle_ctss_chunk)).unwrap();
    assert_eq!(synth_ctss.entries.len(), oracle_ctss.entries.len());
    for (s, o) in synth_ctss.entries.iter().zip(&oracle_ctss.entries) {
        assert_eq!(s.language, o.language);
        assert_eq!(s.text, o.text);
        assert_eq!(s.comment, o.comment);
    }

    // STR# 200 (`bodystring`): ALL 35 rows must match User00172.iff verbatim!
    let synth_str200_chunk = synth_iff.get(b"STR#", 200).unwrap();
    let oracle_str200_chunk = oracle_user.get(b"STR#", 200).unwrap();
    assert_eq!(synth_str200_chunk.label, oracle_str200_chunk.label);
    let synth_str200 = StringTable::parse(synth_iff.data(synth_str200_chunk)).unwrap();
    let oracle_str200 = StringTable::parse(oracle_user.data(oracle_str200_chunk)).unwrap();
    assert_eq!(synth_str200.entries.len(), 35);
    assert_eq!(synth_str200.entries.len(), oracle_str200.entries.len());
    for (row, (s, o)) in synth_str200
        .entries
        .iter()
        .zip(&oracle_str200.entries)
        .enumerate()
    {
        assert_eq!(
            s.text, o.text,
            "STR# 200 row {row} mismatch vs User00172.iff"
        );
    }

    // All inherited TemplatePerson chunks (BHAV, TPRP, TREE, SLOT, GLOB, STR# 256..304) match.
    for (tag, id) in [
        (*b"GLOB", 128),
        (*b"SLOT", 128),
        (*b"BHAV", 4096),
        (*b"BHAV", 4097),
        (*b"BHAV", 4098),
        (*b"BHAV", 4100),
        (*b"TPRP", 4096),
        (*b"TPRP", 4097),
        (*b"TPRP", 4098),
        (*b"TPRP", 4100),
        (*b"TREE", 4096),
        (*b"TREE", 4097),
        (*b"TREE", 4098),
        (*b"TREE", 4100),
        (*b"STR#", 256),
        (*b"STR#", 257),
        (*b"STR#", 258),
        (*b"STR#", 300),
        (*b"STR#", 304),
    ] {
        assert_eq!(
            synth_iff.data(synth_iff.get(&tag, id).unwrap()),
            oracle_user.data(oracle_user.get(&tag, id).unwrap()),
            "chunk {} #{id} must match User00172.iff",
            String::from_utf8_lossy(&tag)
        );
    }

    // 3. Family allocation & FAMI 7 / FAMs 7 byte-for-byte check vs Neighborhood.iff.
    let mut initial_families = BTreeMap::new();
    for chunk in oracle_initial_nh.all(b"FAMI") {
        initial_families.insert(
            chunk.id,
            Family::parse(oracle_initial_nh.data(chunk)).unwrap(),
        );
    }
    let (alloc_id, alloc_num) = allocate_family_id(&initial_families).unwrap();
    assert_eq!(alloc_id, 7);
    assert_eq!(alloc_num, 16);

    let oracle_fami7_raw = oracle_after_nh.data(oracle_after_nh.get(b"FAMI", 7).unwrap());
    let oracle_fami7 = Family::parse(oracle_fami7_raw).unwrap();
    let synth_fami7 = Family {
        version: 9,
        house: 0,
        number: alloc_num,
        budget: ctx.starting_funds,
        architecture_value: 0,
        friends: 0,
        flags: CAF_INITIAL_FAMILY_FLAGS,
        members: vec![oracle_guid],
        tail: vec![0; 16],
    };
    assert_eq!(synth_fami7, oracle_fami7);
    assert_eq!(encode_fami(&synth_fami7), oracle_fami7_raw);

    let synth_fams7 = encode_fams("Alice", 1).unwrap();
    let oracle_fams7_raw = oracle_after_nh.data(oracle_after_nh.get(b"FAMs", 7).unwrap());
    assert_eq!(synth_fams7.len(), oracle_fams7_raw.len());
    let synth_fams7_tbl = StringTable::parse(&synth_fams7).unwrap();
    let oracle_fams7_tbl = StringTable::parse(oracle_fams7_raw).unwrap();
    assert_eq!(synth_fams7_tbl.get(0, 1), oracle_fams7_tbl.get(0, 1));

    // 4. NBRS 464 field-for-field check vs 01-after-create/Neighborhood.iff!
    let oracle_nbrs =
        Neighbors::parse(oracle_after_nh.data(oracle_after_nh.get(b"NBRS", 1).unwrap())).unwrap();
    let oracle_alice = oracle_nbrs.entries[463].as_ref().unwrap();
    let mut rng = CrtRng::from_state(1);
    let synth_alice = synthesize_neighbor(&alice_spec, 464, oracle_guid, 172, 7, &mut rng);

    assert_eq!(synth_alice.version, oracle_alice.version);
    assert_eq!(synth_alice.version_extra, oracle_alice.version_extra);
    assert_eq!(synth_alice.name, oracle_alice.name);
    assert_eq!(synth_alice.unknown_zero, oracle_alice.unknown_zero);
    assert_eq!(synth_alice.person_mode, oracle_alice.person_mode);
    assert_eq!(synth_alice.id, oracle_alice.id);
    assert_eq!(synth_alice.guid, oracle_alice.guid);
    assert_eq!(synth_alice.unknown_negative, oracle_alice.unknown_negative);
    assert_eq!(synth_alice.relationships, oracle_alice.relationships);
    assert_eq!(
        synth_alice.person_data.len(),
        oracle_alice.person_data.len()
    );

    // Deterministic non-RNG person_data words must match oracle verbatim.
    for word in [
        person_word::NICE,
        person_word::ACTIVE,
        person_word::PLAYFUL,
        person_word::OUTGOING,
        person_word::NEAT,
        person_word::JOB,
        person_word::JOB_LEVEL_OR_GRADE,
        person_word::AGE,
        person_word::SKIN,
        person_word::FAMILY,
        person_word::GENDER,
        person_word::ZODIAC,
    ] {
        assert_eq!(
            synth_alice.person_data[word], oracle_alice.person_data[word],
            "NBRS person_data[{word}] mismatch vs 01-after-create/Neighborhood.iff"
        );
    }
    assert_eq!(
        zodiac_name(compute_zodiac(alice_spec.personality)),
        "Cancer"
    );

    // Verify the 15 interest words obey the exact `0x459a9c` bucket distribution
    // ([4, 3, 3] for words 46..=55 and [1, 2, 2] for [13, 14, 16, 20, 26]) in both
    // oracle and synthesized records, and every other word is 0.
    let check_buckets = |data: &[i16], words: &[usize], expected: [usize; 3]| {
        let mut counts = [0usize; 3];
        for &w in words {
            let v = data[w];
            assert!((0..=1000).contains(&v) && v % 100 == 0);
            match v / 100 {
                0..=3 => counts[0] += 1,
                4..=6 => counts[1] += 1,
                7..=10 => counts[2] += 1,
                _ => unreachable!(),
            }
        }
        assert_eq!(counts, expected);
    };
    let hot_date_interests = [46, 47, 48, 49, 50, 51, 52, 53, 54, 55];
    let base_interests = [13, 14, 16, 20, 26];
    check_buckets(&oracle_alice.person_data, &hot_date_interests, [4, 3, 3]);
    check_buckets(&synth_alice.person_data, &hot_date_interests, [4, 3, 3]);
    check_buckets(&oracle_alice.person_data, &base_interests, [1, 2, 2]);
    check_buckets(&synth_alice.person_data, &base_interests, [1, 2, 2]);

    let interest_set: BTreeSet<usize> = hot_date_interests
        .into_iter()
        .chain(base_interests)
        .collect();
    for w in 0..256 {
        if !interest_set.contains(&w) {
            assert_eq!(
                synth_alice.person_data[w], oracle_alice.person_data[w],
                "non-interest NBRS person_data[{w}] mismatch"
            );
        }
    }

    // 5. Verify Export/Alice_7.FAM structure and exact chunk parity with our
    //    `export_family_fam` (export clears neighborhood-local CAF flags to `flags & 1`).
    let mut expected_exported_fami = oracle_fami7.clone();
    expected_exported_fami.flags &= 1;
    assert_eq!(oracle_fam.family, expected_exported_fami);
    assert_eq!(oracle_fam.export_info.family_id, 7);
    assert_eq!(oracle_fam.export_info.member_ids[0], 464);
    assert_eq!(oracle_fam.export_info.language, Some(1));
    assert!(oracle_fam.iff.get(b"FINV", 2).is_some());

    let mut oracle_nh_for_export = empty_stored_neighborhood("OracleExport");
    oracle_nh_for_export.families.insert(7, oracle_fami7);
    oracle_nh_for_export.neighbors = oracle_nbrs.entries;
    let oracle_chars = BTreeMap::from([("User00172.iff".to_owned(), synth_iff.bytes.clone())]);
    let synth_fam_bytes =
        export_family_fam(&oracle_nh_for_export, &oracle_chars, 7, "Alice", 1, true).unwrap();
    let synth_fam = FamilyFile::parse(&synth_fam_bytes).unwrap();
    assert_eq!(synth_fam.family, oracle_fam.family);
    assert_eq!(
        synth_fam.iff.data(synth_fam.iff.get(b"FAMI", 7).unwrap()),
        oracle_fam.iff.data(oracle_fam.iff.get(b"FAMI", 7).unwrap())
    );
    assert_eq!(
        synth_fam.iff.data(synth_fam.iff.get(b"EXPi", 0).unwrap()),
        oracle_fam.iff.data(oracle_fam.iff.get(b"EXPi", 0).unwrap())
    );
    assert_eq!(
        synth_fam.iff.data(synth_fam.iff.get(b"FINV", 2).unwrap()),
        oracle_fam.iff.data(oracle_fam.iff.get(b"FINV", 2).unwrap())
    );
    assert_eq!(
        synth_fam.export_info.member_ids,
        oracle_fam.export_info.member_ids
    );
    assert_eq!(
        synth_fam.export_info.language,
        oracle_fam.export_info.language
    );

    // Also verify member uChr 464, CTSS 464, CTSS 2464, BMP_ 464, and NBRS #1
    // against Export/Alice_7.FAM!
    let synth_m = synth_fam.members.get(&464).unwrap();
    let oracle_m = oracle_fam.members.get(&464).unwrap();
    assert_eq!(
        synth_m
            .body_strings
            .entries
            .iter()
            .map(|e| &e.text)
            .collect::<Vec<_>>(),
        oracle_m
            .body_strings
            .entries
            .iter()
            .map(|e| &e.text)
            .collect::<Vec<_>>(),
        "exported uChr 464 body strings must match Alice_7.FAM"
    );
    let synth_ctss_m = StringTable::parse(synth_m.ctss.as_deref().unwrap()).unwrap();
    let oracle_ctss_m = StringTable::parse(oracle_m.ctss.as_deref().unwrap()).unwrap();
    assert_eq!(synth_ctss_m.get(0, 1), oracle_ctss_m.get(0, 1));
    assert_eq!(synth_ctss_m.get(1, 1), oracle_ctss_m.get(1, 1));
    let synth_loc_ctss = StringTable::parse(synth_m.localized_ctss.as_deref().unwrap()).unwrap();
    let oracle_loc_ctss = StringTable::parse(oracle_m.localized_ctss.as_deref().unwrap()).unwrap();
    assert_eq!(synth_loc_ctss.entries.len(), oracle_loc_ctss.entries.len());
    for lang in 1u8..=20 {
        assert_eq!(synth_loc_ctss.get(0, lang), oracle_loc_ctss.get(0, lang));
        assert_eq!(synth_loc_ctss.get(1, lang), oracle_loc_ctss.get(1, lang));
    }
    let synth_bmp = bmp(synth_m.portrait.as_deref().unwrap()).unwrap();
    let oracle_bmp = bmp(oracle_m.portrait.as_deref().unwrap()).unwrap();
    assert_eq!(synth_bmp, oracle_bmp);
    assert_eq!(
        synth_fam.neighbors.entries.len(),
        oracle_fam.neighbors.entries.len()
    );
    assert_eq!(
        synth_fam.neighbors.entries[463],
        oracle_fam.neighbors.entries[463]
    );
}

/// Full end-to-end gameplay lifecycle test on Complete Collection `UserData`:
/// 1. Create a 2-Sim CAF household (Adult + Child) in REDB `NeighborhoodStore`
/// 2. Verify Family Bin membership, unique GUIDs, `User%05d.iff` portraits, and `.FAM` export
/// 3. Move the household into vacant residential lot `House04` ($5,500)
/// 4. Boot the lot (`enter_lot`), verify both Sims spawn, step 500 ticks cleanly with 0 errors
/// 5. Save & Leave (`save_and_leave_lot`) and re-enter cleanly
/// 6. Create a second CAF family in the Family Bin, delete it (`delete_bin_family`),
///    and verify another lot (`House05` Goth) boots cleanly without resurrecting deleted Sims
#[test]
fn end_to_end_create_family_move_in_500_ticks_save_and_delete_bin_family() {
    let Some(root) = install() else { return };
    let db = TempDb::new("e2e-caf-movein-500ticks");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();

    // 1. Create a 3-Sim household (Adult Alice + Light Male Child + Dark Female
    //    Child to exercise Maxis's shipped `L100FCChddrk_orignal.bmp` sleepwear
    //    typo!) in UserData's Family Bin.
    let mut child = SimSpec {
        first_name: "Charlie".into(),
        bio: "CAF child Sim.".into(),
        age: Age::Child,
        gender: Gender::Male,
        skin_tone: SkinTone::Light,
        body_appearance: "b001mcchd_01".into(),
        body_texture: "b001mcchdlgt_boy01".into(),
        head_appearance: "c001mc_ave".into(),
        head_texture: "c001mclgt_ave".into(),
        personality: Personality::new(4, 6, 7, 5, 3).unwrap(),
        portraits: None,
    };
    let dark_girl = SimSpec {
        first_name: "Maya".into(),
        bio: "Dark-skinned female child Sim.".into(),
        age: Age::Child,
        gender: Gender::Female,
        skin_tone: SkinTone::Dark,
        body_appearance: "b009fcchd_01".into(),
        body_texture: "b009fcchddrk_alice".into(),
        head_appearance: "c504fc_winter2".into(),
        head_texture: "c504fcdrk_winter2".into(),
        personality: Personality::new(5, 5, 5, 5, 5).unwrap(),
        portraits: None,
    };
    let spec = FamilySpec::new(
        "Alice",
        "End-to-end CAF family.",
        vec![alice_sim_spec(None), child.clone(), dark_girl],
    );
    let mut rng = CrtRng::from_state(0x51a5_cafe);
    let created = commit_family(&root, &store, "UserData", &spec, &mut rng).unwrap();
    assert_eq!(created.family.budget, 20_000);
    assert_eq!(created.family.house, 0);
    assert_eq!(created.family.flags, CAF_INITIAL_FAMILY_FLAGS);
    assert_eq!(created.members.len(), 3);

    // Check Family Bin visibility and exported .FAM round-trip.
    let nh_after_create = store.load_neighborhood("UserData").unwrap().unwrap();
    assert!(
        nh_after_create
            .family_bin()
            .iter()
            .any(|&(id, _)| id == created.family_id),
        "newly created household must immediately appear in the Family Bin"
    );
    let char_files = store.load_character_files("UserData").unwrap();
    for m in &created.members {
        assert!(char_files.contains_key(&m.file_name));
    }
    let exported_fam = export_family_fam(
        &nh_after_create,
        &char_files,
        created.family_id,
        &created.family_name,
        1,
        true,
    )
    .unwrap();
    let parsed_fam = FamilyFile::parse(&exported_fam).unwrap();
    assert_eq!(parsed_fam.family.members, created.family.members);
    assert_eq!(parsed_fam.members.len(), 3);

    // 2. Move the CAF household into vacant residential lot House04 ($5,500).
    let moved = move_in(&root, &store, "UserData", created.family_id, 4).unwrap();
    assert_eq!(moved.lot_price, 5_500);
    assert_eq!(moved.budget_before, 20_000);
    assert_eq!(moved.budget_after, 14_500);

    // 3. Boot House04, verify both Sims spawn, portraits load, and step 500 ticks cleanly!
    let mut lot4 = enter_lot(&root, &store, "UserData", 4, &[3], Frames::default()).unwrap();
    assert_eq!(lot4.world().budget.funds, 14_500);
    assert_eq!(lot4.world().globals[0], 7); // new family start hour
    for m in &created.members {
        let def = lot4.catalog().object(m.guid).expect("CAF Sim in catalog");
        let lib = lot4
            .program()
            .libraries
            .get(&def.resource)
            .expect("CAF Sim linked");
        assert!(
            lib.has_portrait_bitmap,
            "synthesized character {} must expose decodable BMP_ 2002",
            m.file_name
        );
        assert!(
            lot4.world()
                .objects
                .values()
                .any(|o| o.guid == m.guid && o.person.is_some()),
            "CAF member {} ({:#010x}) must spawn on vacant lot",
            m.first_name,
            m.guid
        );
    }

    for tick in 0..500 {
        lot4.step()
            .unwrap_or_else(|e| panic!("CAF lot stepped into error at tick {tick}: {e}"));
    }
    assert_eq!(lot4.ticks(), 500);

    // 4. Save & Leave, verify flags cleared (bit 4 preserved, flag 4 cleared), and re-enter.
    let saved = save_and_leave_lot(&root, &store, "UserData", &mut lot4).unwrap();
    assert_eq!(saved.house, 4);
    assert_eq!(saved.family, created.family_id);
    assert!(saved.saved_people >= 2);

    let nh_after_save = store.load_neighborhood("UserData").unwrap().unwrap();
    let fam_after_save = &nh_after_save.families[&created.family_id];
    assert_eq!(fam_after_save.flags & FAMILY_FLAG_NEW_MOVE_IN, 0);
    assert_eq!(
        fam_after_save.flags & FAMILY_FLAG_CAF_CREATED,
        FAMILY_FLAG_CAF_CREATED
    );
    let lot4_reentered = enter_lot(&root, &store, "UserData", 4, &[3], Frames::default()).unwrap();
    assert_eq!(lot4_reentered.ticks(), 500);
    assert_eq!(lot4_reentered.world().budget.funds, saved.budget);

    // 5. Create a second household in the Family Bin, delete it via `delete_bin_family`,
    //    and confirm Goth House 5 boots cleanly with no trace of the deleted Sim.
    child.first_name = "TempKid".into();
    let temp_spec = FamilySpec::new("TempBin", "", vec![alice_sim_spec(None)]);
    let temp = commit_family(&root, &store, "UserData", &temp_spec, &mut rng).unwrap();
    let temp_guid = temp.members[0].guid;
    let deleted = delete_bin_family(&store, "UserData", temp.family_id).unwrap();
    assert_eq!(deleted.deleted_guids, vec![temp_guid]);

    let mut goth = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    assert!(
        goth.world().neighbors.first_of(temp_guid).is_none(),
        "deleted Family Bin Sim GUID must not reappear in NBRS on lot load"
    );
    for _ in 0..10 {
        goth.step().unwrap();
    }

    // 6. Verify crash/mid-flight safety of Phase 1 tombstone shielding:
    //    if a character IFF is staged in REDB while its GUID is reserved in
    //    `deleted_characters` (before Phase 3 publishes FAMI/NBRS), `enter_lot`
    //    must never register that staged/orphaned character as a stray neighbor.
    let staged_guid = 0xfeed_beef_u32;
    let ctx = CafContext::from_install(&root).unwrap();
    let staged_iff = synthesize_character_iff(
        &ctx.template_iff,
        &alice_sim_spec(None),
        staged_guid,
        1999,
        1,
        ctx.wardrobe.as_ref(),
    )
    .unwrap();
    {
        let mut reserved = store.load_neighborhood("UserData").unwrap().unwrap();
        reserved.deleted_characters.insert(staged_guid);
        store.save_neighborhood("UserData", &reserved).unwrap();
        store
            .save_character_file("UserData", "User01999.iff", &staged_iff)
            .unwrap();
    }
    let goth_with_staged =
        enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    assert!(
        goth_with_staged
            .world()
            .neighbors
            .first_of(staged_guid)
            .is_none(),
        "tombstone-shielded staged character IFF must never be registered in NBRS on lot load"
    );
}

/// Ensures the oracle `.FAM` file (`Export/Alice_7.FAM`) imports cleanly into
/// REDB `NeighborhoodStore` via `import_family_fam` and boots on `House04`.
#[test]
fn oracle_alice_fam_imports_and_boots_cleanly() {
    let Some(root) = install() else { return };
    let Some(oracle) = caf_oracle_dir() else {
        return;
    };
    let fam_bytes = std::fs::read(oracle.join("01-after-create/Export/Alice_7.FAM")).unwrap();

    let db = TempDb::new("oracle-fam-import");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();

    let ctx = CafContext::from_install(&root).unwrap();
    let mut rng = CrtRng::from_state(0xface_b00c);
    let imported = import_family_fam(&ctx, &store, "UserData", &fam_bytes, 1, &mut rng).unwrap();
    assert_eq!(imported.family_name, "Alice");
    assert_eq!(imported.family.budget, 20_000);
    assert_eq!(imported.members.len(), 1);
    assert_eq!(imported.members[0].first_name, "Alice");
    assert_eq!(
        imported.members[0].bio,
        "Track 4 Create-a-Family oracle Sim."
    );
    assert_eq!(imported.members[0].zodiac, 4);

    let moved = move_in(&root, &store, "UserData", imported.family_id, 4).unwrap();
    assert_eq!(moved.budget_after, 14_500);
    let mut lot4 = enter_lot(&root, &store, "UserData", 4, &[3], Frames::default()).unwrap();
    for _ in 0..50 {
        lot4.step().unwrap();
    }
    assert_eq!(lot4.ticks(), 50);
    let _ = Path::new(&db.0);
}

/// Regression test for single-transaction atomic CAF commits:
/// 1. Committing a household advances `StoredNeighborhood::revision` by exactly
///    +1 (single write transaction instead of multi-phase staging).
/// 2. Racing concurrent `commit_family_with_context` / `import_family_fam`
///    against concurrent neighborhood replacement (`delete_neighborhood` +
///    `save_neighborhood`, or `import_directory` when `SIMS_INSTALL` is present)
///    never leaves orphaned `User%05d.iff` character files or half-committed
///    tombstones in the store: every character file present in the final
///    neighborhood corresponds to a committed `NBRS` member of that exact
///    neighborhood instance.
/// 3. `delete_bin_family` purges the deleted family's persisted photo album.
#[test]
fn atomic_caf_commit_survives_concurrent_reimport_and_purges_album() {
    use sims_sim::NewAlbumPhoto;
    use std::sync::Barrier;

    let db = TempDb::new("atomic-caf-concurrency");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .save_neighborhood("RaceNH", &empty_stored_neighborhood("RaceNH"))
        .unwrap();
    let ctx = CafContext::synthetic(
        synthetic_template_iff(),
        BTreeSet::from([TEMPLATE_PERSON_GUID]),
        DEFAULT_STARTING_FUNDS,
    );

    // 1. Single-transaction commit bumps revision by exactly +1 for a 3-Sim household.
    let before = store.load_neighborhood("RaceNH").unwrap().unwrap();
    let three_sims = FamilySpec::new(
        "Atomic",
        "Single txn",
        vec![
            alice_sim_spec(None),
            alice_sim_spec(None),
            alice_sim_spec(None),
        ],
    );
    let mut rng = CrtRng::from_state(0x1234_5678);
    let created =
        commit_family_with_context(&ctx, &store, "RaceNH", &three_sims, &mut rng).unwrap();
    assert_eq!(created.revision, before.revision + 1);
    let after = store.load_neighborhood("RaceNH").unwrap().unwrap();
    assert_eq!(after.revision, before.revision + 1);
    assert!(after.deleted_characters.is_empty());

    // Prepare a valid `.FAM` archive for concurrent import_family_fam racing too.
    let chars_for_export = store.load_character_files("RaceNH").unwrap();
    let fam_bytes = export_family_fam(
        &after,
        &chars_for_export,
        created.family_id,
        "Atomic",
        1,
        true,
    )
    .unwrap();

    // Seed a photo album for `created.family_id`, then delete the bin family and
    // confirm its album is purged cleanly.
    store
        .albums()
        .add(
            "RaceNH",
            after.instance_id,
            created.family_id,
            NewAlbumPhoto::blank("Pre-Delete", "Should be purged"),
        )
        .unwrap();
    assert_eq!(
        store
            .albums()
            .list("RaceNH", created.family_id)
            .unwrap()
            .len(),
        1
    );
    delete_bin_family(&store, "RaceNH", created.family_id).unwrap();
    assert!(
        store
            .albums()
            .list("RaceNH", created.family_id)
            .unwrap()
            .is_empty()
    );

    // 2. Race CAF commits and FAM imports against concurrent neighborhood
    //    replacements (using real `import_directory` if `SIMS_INSTALL` is set,
    //    plus synthetic `delete_neighborhood` + `save_neighborhood`).
    let barrier = Barrier::new(3);
    std::thread::scope(|scope| {
        let store_a = &store;
        let ctx_a = &ctx;
        let barrier_a = &barrier;
        let t_commit = scope.spawn(move || {
            barrier_a.wait();
            let mut local_rng = CrtRng::from_state(0xaaaa_1111);
            for i in 0..12u32 {
                let mut sim = alice_sim_spec(None);
                sim.first_name = format!("RaceA{i}");
                let spec = FamilySpec::new(format!("FamA{i}"), "", vec![sim.clone(), sim]);
                match commit_family_with_context(ctx_a, store_a, "RaceNH", &spec, &mut local_rng) {
                    Ok(_)
                    | Err(CafError::NeighborhoodMismatch { .. })
                    | Err(CafError::StaleNeighborhood { .. })
                    | Err(CafError::NeighborhoodNotFound(_))
                    | Err(CafError::Store(_)) => {}
                    Err(other) => panic!("unexpected CAF commit error under race: {other}"),
                }
            }
        });

        let store_b = &store;
        let ctx_b = &ctx;
        let fam_b = &fam_bytes;
        let barrier_b = &barrier;
        let t_import = scope.spawn(move || {
            barrier_b.wait();
            let mut local_rng = CrtRng::from_state(0xbbbb_2222);
            for _ in 0..8u32 {
                match import_family_fam(ctx_b, store_b, "RaceNH", fam_b, 1, &mut local_rng) {
                    Ok(_)
                    | Err(CafError::NeighborhoodMismatch { .. })
                    | Err(CafError::StaleNeighborhood { .. })
                    | Err(CafError::NeighborhoodNotFound(_))
                    | Err(CafError::Store(_)) => {}
                    Err(other) => panic!("unexpected FAM import error under race: {other}"),
                }
            }
        });

        let store_c = &store;
        let barrier_c = &barrier;
        let t_replace = scope.spawn(move || {
            barrier_c.wait();
            for _ in 0..10u32 {
                let _ = store_c.delete_neighborhood("RaceNH");
                store_c
                    .save_neighborhood("RaceNH", &empty_stored_neighborhood("RaceNH"))
                    .unwrap();
            }
        });

        t_commit.join().unwrap();
        t_import.join().unwrap();
        t_replace.join().unwrap();
    });

    // After the race settles, EVERY character file in `RaceNH` must correspond
    // to a live `NBRS` entry in `RaceNH` — zero orphaned files and zero
    // lingering staging tombstones!
    let final_snap = store.load_lot_snapshot("RaceNH", 0).unwrap().unwrap();
    assert!(
        final_snap.neighborhood.deleted_characters.is_empty(),
        "atomic CAF commits must never leave staging tombstones"
    );
    let expected_files: BTreeSet<String> = final_snap
        .neighborhood
        .neighbors
        .iter()
        .flatten()
        .map(|n| format!("U{}.iff", &n.name[1..]))
        .collect();
    let actual_files: BTreeSet<String> = final_snap.character_files.keys().cloned().collect();
    assert_eq!(
        actual_files, expected_files,
        "character files must exactly match committed NBRS entries with zero orphans"
    );

    // 3. Also race against real `NeighborhoodStore::import_directory` when SIMS_INSTALL is available.
    if let Some(root) = install() {
        let ud_dir = root.join("UserData");
        let db_imp = TempDb::new("atomic-caf-vs-import-dir");
        let store_imp = NeighborhoodStore::open(&db_imp.0).unwrap();
        store_imp
            .import_directory("UserData", &ud_dir, 0xff)
            .unwrap();
        let baseline_chars: BTreeSet<String> = store_imp
            .load_character_files("UserData")
            .unwrap()
            .into_keys()
            .collect();
        let install_ctx = CafContext::from_install(&root).unwrap();
        let barrier2 = Barrier::new(2);
        std::thread::scope(|scope| {
            let s1 = &store_imp;
            let c1 = &install_ctx;
            let b1 = &barrier2;
            let h1 = scope.spawn(move || {
                b1.wait();
                let mut local_rng = CrtRng::from_state(0xcccc_3333);
                for i in 0..6u32 {
                    let mut sim = alice_sim_spec(None);
                    sim.first_name = format!("ImpRace{i}");
                    let spec = FamilySpec::new(format!("ImpFam{i}"), "", vec![sim]);
                    match commit_family_with_context(c1, s1, "UserData", &spec, &mut local_rng) {
                        Ok(_)
                        | Err(CafError::NeighborhoodMismatch { .. })
                        | Err(CafError::StaleNeighborhood { .. }) => {}
                        Err(other) => panic!("unexpected error racing import_directory: {other}"),
                    }
                }
            });
            let s2 = &store_imp;
            let ud_ref = &ud_dir;
            let b2 = &barrier2;
            let h2 = scope.spawn(move || {
                b2.wait();
                for _ in 0..3u32 {
                    s2.import_directory("UserData", ud_ref, 0xff).unwrap();
                }
            });
            h1.join().unwrap();
            h2.join().unwrap();
        });

        let post = store_imp.load_lot_snapshot("UserData", 0).unwrap().unwrap();
        assert!(post.neighborhood.deleted_characters.is_empty());
        let nbr_files: BTreeSet<String> = post
            .neighborhood
            .neighbors
            .iter()
            .flatten()
            .filter_map(|n| {
                n.name
                    .strip_prefix("user")
                    .map(|digits| format!("User{digits}.iff"))
            })
            .collect();
        for file in post.character_files.keys() {
            assert!(
                baseline_chars.contains(file) || nbr_files.contains(file),
                "leaked orphan character file {file:?} after concurrent import_directory"
            );
        }
    }
}
