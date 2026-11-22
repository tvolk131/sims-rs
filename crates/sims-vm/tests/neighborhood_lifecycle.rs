//! Unit and `SIMS_INSTALL` integration tests for neighborhood REDB lot
//! lifecycle: multi-lot round-trip (`Goth House 5` ↔ `Newbie House 7`),
//! Move-In, Evict, and Bulldoze.

use sims_formats::neighborhood::Family;
use sims_sim::characters::Characters;
use sims_sim::{
    DEFAULT_ARCH_VALUE_MULTIPLIER, FAMILY_FLAG_BIT_3, FAMILY_FLAG_NEW_MOVE_IN, NeighborhoodStore,
    StoredNeighborhood,
};
use sims_vm::boot::Frames;
use sims_vm::lifecycle::{
    LifecycleError, bulldoze, enter_lot, evict, lot_price, move_in, move_in_with_multiplier,
    save_and_leave_lot,
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
            "sims-vm-lifecycle-{label}-{}-{n}.redb",
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
            eprintln!("SIMS_INSTALL is not set; skipping install-backed lifecycle test");
            None
        }
    }
}

/// Builds a minimal valid IFF file containing a single `SIMI` chunk (version 65)
/// with the requested `(lot_value, objects_value, architecture_value)` for
/// pure unit testing without an install.
fn synthetic_house_iff(lot_value: i32, objects_value: i32, architecture_value: i32) -> Vec<u8> {
    let mut simi = Vec::new();
    simi.extend_from_slice(&0u32.to_le_bytes()); // outer mode
    simi.extend_from_slice(&0x41u32.to_le_bytes()); // version 65 (64 globals)
    simi.extend_from_slice(b"IMIS");
    simi.extend(std::iter::repeat_n(0u8, 64 * 2)); // 64 globals
    simi.extend_from_slice(&0u32.to_le_bytes()); // ticks
    simi.extend_from_slice(&0i16.to_le_bytes()); // scheduler_cursor
    simi.extend_from_slice(&0i32.to_le_bytes()); // funds
    simi.extend_from_slice(&0u32.to_le_bytes()); // lot_rng
    simi.extend_from_slice(&0u32.to_le_bytes()); // global_rng
    simi.extend_from_slice(&0i32.to_le_bytes()); // pending_funds
    simi.extend_from_slice(&lot_value.to_le_bytes());
    simi.extend_from_slice(&objects_value.to_le_bytes());
    simi.extend_from_slice(&architecture_value.to_le_bytes());
    for _ in 0..6 {
        simi.extend_from_slice(&0i32.to_le_bytes()); // 6 empty budget_days
    }

    let mut iff = Vec::new();
    let header = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1\0\0\0\0";
    assert_eq!(header.len(), 64);
    iff.extend_from_slice(header);
    let chunk_size = (76 + simi.len()) as u32;
    iff.extend_from_slice(b"SIMI");
    iff.extend_from_slice(&chunk_size.to_be_bytes());
    iff.extend_from_slice(&1u16.to_be_bytes()); // chunk id
    iff.extend_from_slice(&0u16.to_be_bytes()); // flags
    iff.extend(std::iter::repeat_n(0u8, 64)); // label
    iff.extend_from_slice(&simi);
    iff
}

fn synthetic_family(house: i32, budget: i32, arch: i32, members: Vec<u32>, flags: u32) -> Family {
    Family {
        version: 9,
        house,
        number: 1,
        budget,
        architecture_value: arch,
        friends: 0,
        flags,
        members,
        tail: vec![0; 16],
    }
}

#[test]
fn move_in_unit_validation_and_accounting() {
    let db = TempDb::new("movein-unit");
    let store = NeighborhoodStore::open(&db.0).unwrap();

    let mut families = BTreeMap::new();
    families.insert(
        2,
        synthetic_family(0, 20_000, 0, vec![0xAAA1], FAMILY_FLAG_BIT_3),
    );
    families.insert(3, synthetic_family(5, 5_000, 12_000, vec![0xBBB1], 0));
    families.insert(4, synthetic_family(0, 20_000, 0, Vec::new(), 0));
    let mut zoning = BTreeMap::new();
    zoning.insert(4, 1);
    zoning.insert(5, 1);
    zoning.insert(8, 1);
    zoning.insert(21, 2);

    let nh = StoredNeighborhood {
        name: "TestNH".into(),
        families,
        neighbors: Vec::new(),
        inventories: Vec::new(),
        data: [0; 16],
        options: None,
        type_attributes: None,
        zoning,
        lot_rows: vec![4, 5, 8, 21],
        made_characters: Characters::default(),
        deleted_characters: BTreeSet::new(),
        instance_id: 1,
        revision: 0,
    };
    store.save_neighborhood("TestNH", &nh).unwrap();
    store
        .save_base_house("TestNH", 4, &synthetic_house_iff(5_500, 0, 10_000))
        .unwrap(); // price = 5500 + trunc(10000 * 0.7f) = 12500
    store
        .save_base_house("TestNH", 5, &synthetic_house_iff(7_000, 0, 0))
        .unwrap();
    store
        .save_base_house("TestNH", 8, &synthetic_house_iff(25_000, 0, 0))
        .unwrap();
    store
        .save_base_house("TestNH", 21, &synthetic_house_iff(1_000, 0, 0))
        .unwrap();

    // Error cases:
    assert!(matches!(
        move_in_with_multiplier(&store, "Missing", 2, 4, DEFAULT_ARCH_VALUE_MULTIPLIER),
        Err(LifecycleError::NeighborhoodNotFound(_))
    ));
    assert!(matches!(
        move_in_with_multiplier(&store, "TestNH", 99, 4, DEFAULT_ARCH_VALUE_MULTIPLIER),
        Err(LifecycleError::FamilyNotFound { family: 99, .. })
    ));
    assert!(matches!(
        move_in_with_multiplier(&store, "TestNH", 0, 4, DEFAULT_ARCH_VALUE_MULTIPLIER),
        Err(LifecycleError::FamilyEmpty { family: 0 })
    ));
    assert!(matches!(
        move_in_with_multiplier(&store, "TestNH", 4, 4, DEFAULT_ARCH_VALUE_MULTIPLIER),
        Err(LifecycleError::FamilyEmpty { family: 4 })
    ));
    assert!(matches!(
        move_in_with_multiplier(&store, "TestNH", 3, 4, DEFAULT_ARCH_VALUE_MULTIPLIER),
        Err(LifecycleError::FamilyNotInBin {
            family: 3,
            house: 5
        })
    ));
    assert!(matches!(
        move_in_with_multiplier(&store, "TestNH", 2, 21, DEFAULT_ARCH_VALUE_MULTIPLIER),
        Err(LifecycleError::LotNotResidential {
            house: 21,
            zoning: Some(2)
        })
    ));
    assert!(matches!(
        move_in_with_multiplier(&store, "TestNH", 2, 5, DEFAULT_ARCH_VALUE_MULTIPLIER),
        Err(LifecycleError::LotOccupied {
            house: 5,
            family: 3
        })
    ));
    assert!(matches!(
        move_in_with_multiplier(&store, "TestNH", 2, 8, DEFAULT_ARCH_VALUE_MULTIPLIER),
        Err(LifecycleError::InsufficientFunds {
            family: 2,
            house: 8,
            available: 20_000,
            required: 25_000,
        })
    ));

    // Secondary-house occupancy check (`0x425d95`): a family owning house 4 as
    // its secondary house blocks another family from moving in AND cannot move
    // in from the Family Bin itself.
    {
        let mut nh_sec = store.load_neighborhood("TestNH").unwrap().unwrap();
        nh_sec.families.get_mut(&3).unwrap().set_secondary_house(4);
        store.save_neighborhood("TestNH", &nh_sec).unwrap();
        assert!(matches!(
            move_in_with_multiplier(&store, "TestNH", 2, 4, DEFAULT_ARCH_VALUE_MULTIPLIER),
            Err(LifecycleError::LotOccupied {
                house: 4,
                family: 3
            })
        ));
        nh_sec.families.get_mut(&3).unwrap().set_secondary_house(0);
        nh_sec.families.get_mut(&2).unwrap().set_secondary_house(8);
        store.save_neighborhood("TestNH", &nh_sec).unwrap();
        assert!(matches!(
            move_in_with_multiplier(&store, "TestNH", 2, 4, DEFAULT_ARCH_VALUE_MULTIPLIER),
            Err(LifecycleError::FamilyNotInBin {
                family: 2,
                house: 8
            })
        ));
        nh_sec.families.get_mut(&2).unwrap().set_secondary_house(0);
        store.save_neighborhood("TestNH", &nh_sec).unwrap();
    }

    // Success case:
    let expected_price = 5_500 + ((10_000.0_f32 * DEFAULT_ARCH_VALUE_MULTIPLIER) as i32);
    let report =
        move_in_with_multiplier(&store, "TestNH", 2, 4, DEFAULT_ARCH_VALUE_MULTIPLIER).unwrap();
    assert_eq!(report.lot_price, expected_price);
    assert_eq!(report.budget_before, 20_000);
    assert_eq!(report.budget_after, 20_000 - expected_price);
    assert_eq!(report.architecture_value, expected_price);

    let updated = store.load_neighborhood("TestNH").unwrap().unwrap();
    let fam2 = &updated.families[&2];
    assert_eq!(fam2.house, 4);
    assert_eq!(fam2.budget, 20_000 - expected_price);
    assert_eq!(fam2.architecture_value, expected_price);
    assert_eq!(
        fam2.flags & FAMILY_FLAG_NEW_MOVE_IN,
        FAMILY_FLAG_NEW_MOVE_IN
    );
    assert_eq!(fam2.flags & FAMILY_FLAG_BIT_3, 0);
}

#[test]
fn goth_to_newbie_to_goth_round_trip_preserves_and_propagates_state() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("roundtrip-goth-newbie");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();

    // 1. Enter Goth House 5, run 20 ticks, save & leave.
    let mut goth = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    for _ in 0..20 {
        goth.step().unwrap();
    }
    assert_eq!(goth.ticks(), 20);
    let goth_world_ticks = goth.world().ticks;
    let goth_funds = goth.world().budget.funds;
    let goth_objects = goth.world().objects.len();

    let save_goth = save_and_leave_lot(&root, &store, "UserData", &mut goth).unwrap();
    assert_eq!(save_goth.house, 5);
    assert_eq!(save_goth.family, 5);
    assert_eq!(save_goth.budget, goth_funds);
    assert!(save_goth.saved_people >= 3);

    // 2. Enter Newbie House 7, step 20 ticks, save & leave.
    let mut newbie = enter_lot(&root, &store, "UserData", 7, &[3], Frames::default()).unwrap();
    for _ in 0..20 {
        newbie.step().unwrap();
    }
    assert_eq!(newbie.ticks(), 20);
    let newbie_funds = newbie.world().budget.funds;
    let save_newbie = save_and_leave_lot(&root, &store, "UserData", &mut newbie).unwrap();
    assert_eq!(save_newbie.house, 7);
    assert_eq!(save_newbie.family, 1);
    assert_eq!(save_newbie.budget, newbie_funds);
    assert!(save_newbie.revision > save_goth.revision);

    // 3. Re-enter Goth House 5 and confirm lot-local state is preserved and
    //    stepping continues cleanly from tick 20 -> 30.
    let mut goth_again = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    assert_eq!(goth_again.ticks(), 20);
    assert_eq!(goth_again.world().ticks, goth_world_ticks);
    assert_eq!(goth_again.world().budget.funds, goth_funds);
    assert_eq!(goth_again.world().objects.len(), goth_objects);
    for _ in 0..10 {
        goth_again.step().unwrap();
    }
    assert_eq!(goth_again.ticks(), 30);

    // 4. Re-enter Newbie House 7 and confirm its saved state stayed intact too.
    let newbie_again = enter_lot(&root, &store, "UserData", 7, &[3], Frames::default()).unwrap();
    assert_eq!(newbie_again.ticks(), 20);
    assert_eq!(newbie_again.world().budget.funds, newbie_funds);
}

#[test]
fn move_in_enter_save_and_evict_family_bin_household_on_empty_lot() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("movein-evict-house04");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();

    // UserData FAMI #2 is in the Family Bin with $20,000; House04 is an empty
    // residential lot costing $5,500; House02 costs $50,776 (unaffordable).
    let before_nh = store.load_neighborhood("UserData").unwrap().unwrap();
    let bin_fam_id = 2u16;
    assert_eq!(before_nh.families[&bin_fam_id].house, 0);
    assert_eq!(before_nh.families[&bin_fam_id].budget, 20_000);

    assert!(matches!(
        move_in(&root, &store, "UserData", bin_fam_id, 5),
        Err(LifecycleError::LotOccupied {
            house: 5,
            family: 5
        })
    ));
    assert!(matches!(
        move_in(&root, &store, "UserData", bin_fam_id, 2),
        Err(LifecycleError::InsufficientFunds {
            family: 2,
            house: 2,
            available: 20_000,
            required: 50_777,
        })
    ));

    let moved = move_in(&root, &store, "UserData", bin_fam_id, 4).unwrap();
    assert_eq!(moved.lot_price, 5_500);
    assert_eq!(moved.budget_after, 14_500);
    assert_eq!(moved.architecture_value, 5_500);

    // First entry runs flag-4 move-in reset (0x42848d..0x4284f9) and steps
    // cleanly with `Frames::default()`.
    let mut lot4 = enter_lot(&root, &store, "UserData", 4, &[3], Frames::default()).unwrap();
    assert_eq!(lot4.world().globals[0], 7, "new family start hour = 7:00");
    assert_eq!(lot4.world().globals[5], 0, "minutes = 0");
    assert_eq!(lot4.world().globals[17], 0, "unpaused");
    assert_eq!(lot4.world().globals[28], 0, "days played reset");
    assert_eq!(lot4.world().budget.funds, 14_500);
    assert_eq!(lot4.world().budget.days, vec![Some([0; 8]); 6]);
    assert_eq!(lot4.world().usage.history[0].total, 200);

    // Step a few ticks and save & leave; flag 4 is cleared in REDB.
    for _ in 0..5 {
        lot4.step().unwrap();
    }
    let saved = save_and_leave_lot(&root, &store, "UserData", &mut lot4).unwrap();
    assert_eq!(saved.house, 4);
    assert_eq!(saved.family, bin_fam_id);
    let after_save = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(
        after_save.families[&bin_fam_id].flags & FAMILY_FLAG_NEW_MOVE_IN,
        0
    );

    // Evict back to Family Bin: exact refund returns household to $20,000.
    let evicted = evict(&root, &store, "UserData", 4).unwrap();
    assert_eq!(evicted.family, bin_fam_id);
    assert_eq!(evicted.refund, 5_500);
    assert_eq!(evicted.budget_after, 20_000);

    let after_evict = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(after_evict.families[&bin_fam_id].house, 0);
    assert_eq!(after_evict.families[&bin_fam_id].architecture_value, 0);
    assert_eq!(after_evict.families[&bin_fam_id].budget, 20_000);
    assert!(after_evict.family_in(4).is_none());

    // Evicting an already-empty lot fails cleanly.
    assert!(matches!(
        evict(&root, &store, "UserData", 4),
        Err(LifecycleError::LotUnoccupied { house: 4 })
    ));
}

#[test]
fn evict_then_bulldoze_goth_house_5_and_re_move_in() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("evict-bulldoze-goth");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();

    let before_nh = store.load_neighborhood("UserData").unwrap().unwrap();
    let goth_before = before_nh.families[&5].clone();
    let expected_net_worth = goth_before.budget + goth_before.architecture_value;

    // 1. Evict Goths from House 5: exact refund, furniture & Sims removed,
    //    house shell remains intact.
    let evicted = evict(&root, &store, "UserData", 5).unwrap();
    assert_eq!(evicted.family, 5);
    assert_eq!(evicted.refund, goth_before.architecture_value);
    assert_eq!(evicted.budget_after, expected_net_worth);
    assert!(evicted.removed_objects > 10);
    assert_eq!(evicted.remaining_objects_value, 0);
    assert!(
        evicted.remaining_architecture_value > 0,
        "evict keeps the house's walls, floors, doors, and windows"
    );

    let shell_price = lot_price(&store, "UserData", 5, DEFAULT_ARCH_VALUE_MULTIPLIER).unwrap();
    assert!(
        shell_price > 7_000 && shell_price < goth_before.architecture_value,
        "unfurnished house price {shell_price} lies between bare lot ($7,000) and furnished (${})",
        goth_before.architecture_value
    );

    // 1b. Move Goths back onto the EVICTED saved lot and verify all 3 family
    //     members arrive, one is selected, and strict `Frames::default()` steps cleanly!
    let moved_shell = move_in(&root, &store, "UserData", 5, 5).unwrap();
    assert_eq!(moved_shell.lot_price, shell_price);
    let mut shell_lot = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    assert_eq!(
        shell_lot.world().people.len(),
        3,
        "evicted saved lot must spawn all 3 Goth residents on move-in entry"
    );
    assert!(
        shell_lot.world().objects.values().any(|o| o.selected),
        "a Goth resident must be selected on evicted saved lot entry"
    );
    for _ in 0..5 {
        shell_lot.step().unwrap();
    }
    save_and_leave_lot(&root, &store, "UserData", &mut shell_lot).unwrap();

    // 2. Bulldoze occupied House 5: evicts Goths cleanly, clears walls, floors,
    //    doors, windows, pool, and deletes tombstone character files (`0x426fcd`).
    let chars_before = store.load_character_files("UserData").unwrap().len();
    let dozed = bulldoze(&root, &store, "UserData", 5).unwrap();
    assert_eq!(dozed.evicted_family, Some(5));
    assert!(dozed.cleared_tiles > 0);
    assert!(dozed.deleted_tombstones >= 1);
    assert_eq!(dozed.remaining_objects_value, 0);
    assert_eq!(dozed.remaining_architecture_value, 0);
    assert_eq!(
        lot_price(&store, "UserData", 5, DEFAULT_ARCH_VALUE_MULTIPLIER).unwrap(),
        7_000
    );
    assert!(
        store.load_character_files("UserData").unwrap().len() < chars_before,
        "bulldoze must remove deleted tombstone character files from REDB"
    );
    // Deleted tombstone GUID (0x7757D9EE) must stay gone when entering another lot.
    let newbie_after_doze =
        enter_lot(&root, &store, "UserData", 7, &[3], Frames::default()).unwrap();
    assert!(
        newbie_after_doze
            .world()
            .neighbors
            .first_of(0x7757_D9EE)
            .is_none(),
        "deleted tombstone GUID 0x7757D9EE must not reappear on reload"
    );

    // 3. Move the Goths back onto their freshly bulldozed lot and verify all 3
    //    Goth members arrive, one is selected, and `Frames::default()` steps cleanly!
    let nh_after_doze = store.load_neighborhood("UserData").unwrap().unwrap();
    let goth_budget_after_doze = nh_after_doze.families[&5].budget;
    let moved_back = move_in(&root, &store, "UserData", 5, 5).unwrap();
    assert_eq!(moved_back.lot_price, 7_000);
    assert_eq!(moved_back.budget_after, goth_budget_after_doze - 7_000);
    let mut lot5 = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    assert_eq!(lot5.world().budget.funds, goth_budget_after_doze - 7_000);
    assert_eq!(lot5.world().globals[0], 7);
    assert_eq!(
        lot5.world().people.len(),
        3,
        "bulldozed saved lot must spawn all 3 Goth residents on move-in entry"
    );
    assert!(
        lot5.world().objects.values().any(|o| o.selected),
        "a Goth resident must be selected on bulldozed saved lot entry"
    );
    assert!(
        lot5.world().neighbors.first_of(0x7757_D9EE).is_none(),
        "deleted tombstone GUID 0x7757D9EE must not reappear on House 5"
    );
    for _ in 0..5 {
        lot5.step().unwrap();
    }
}

#[test]
fn stale_revision_and_cross_neighborhood_saves_are_rejected() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("stale-and-cross");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("nh_a", &root.join("UserData"), 0xff)
        .unwrap();
    store
        .import_directory("nh_b", &root.join("UserData"), 0xff)
        .unwrap();
    let no_frames = Frames::parse("none").unwrap();

    // 1. Cross-neighborhood save rejected without mutating target neighborhood.
    let mut goth_a = enter_lot(&root, &store, "nh_a", 5, &[3], no_frames.clone()).unwrap();
    let b_before = store.load_neighborhood("nh_b").unwrap().unwrap();
    assert!(matches!(
        save_and_leave_lot(&root, &store, "nh_b", &mut goth_a),
        Err(LifecycleError::NeighborhoodMismatch { .. })
    ));
    assert_eq!(store.load_neighborhood("nh_b").unwrap().unwrap(), b_before);
    assert!(store.load_saved_lot("nh_b", 5).unwrap().is_none());

    // 2. Repeated saves on the SAME open lot advance tracked revision cleanly.
    let r1 = save_and_leave_lot(&root, &store, "nh_a", &mut goth_a).unwrap();
    let r2 = save_and_leave_lot(&root, &store, "nh_a", &mut goth_a).unwrap();
    assert_eq!(r2.revision, r1.revision + 1);

    // 3. Stale save after another committed mutation (`move_in` on House 4) is
    //    rejected and leaves Family 2 in House 4 untouched.
    move_in(&root, &store, "nh_a", 2, 4).unwrap();
    assert!(matches!(
        save_and_leave_lot(&root, &store, "nh_a", &mut goth_a),
        Err(LifecycleError::StaleNeighborhood { .. })
    ));
    let a_after = store.load_neighborhood("nh_a").unwrap().unwrap();
    assert_eq!(a_after.families[&2].house, 4);
}

#[test]
fn runtime_made_character_persists_across_base_and_saved_lot_reentry() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("made-char");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();
    let no_frames = Frames::parse("none").unwrap();

    // Save Goth House 5 first so House 5 is a saved lot while House 7 stays base.
    let mut goth = enter_lot(&root, &store, "UserData", 5, &[3], no_frames.clone()).unwrap();
    save_and_leave_lot(&root, &store, "UserData", &mut goth).unwrap();

    // Persist a runtime-made character (`Make New Character`, primitive 19)
    // alongside its NBRS record directly in the stored neighborhood.
    let mut nh = store.load_neighborhood("UserData").unwrap().unwrap();
    let template_guid = nh.families[&5].members[0];
    let made_guid = 0xDEAD_BEEFu32;
    let next_id = nh
        .neighbors
        .iter()
        .flatten()
        .map(|n| n.id)
        .max()
        .unwrap_or(0)
        + 1;
    nh.made_characters.made.insert(
        made_guid,
        sims_sim::characters::Character {
            template: template_guid,
            file: 999,
            strings: BTreeMap::new(),
        },
    );
    nh.made_characters.next_file = 1000;
    nh.neighbors
        .push(Some(sims_formats::neighborhood::Neighbor {
            version: 10,
            version_extra: Some(1),
            name: "RuntimeMade".into(),
            unknown_zero: 0,
            person_mode: 9,
            person_data: vec![0; sims_sim::neighbors::PERSON_WORDS],
            id: next_id,
            guid: made_guid,
            unknown_negative: -1,
            relationships: Vec::new(),
        }));
    store.save_neighborhood("UserData", &nh).unwrap();

    // Both saved lot (House 5) and base lot (House 7) must open cleanly with
    // the runtime-made character intact in `world.neighbors` and `made`.
    let goth_re = enter_lot(&root, &store, "UserData", 5, &[3], no_frames.clone()).unwrap();
    assert!(goth_re.world().neighbors.first_of(made_guid).is_some());
    assert!(goth_re.world().characters.made.contains_key(&made_guid));

    let newbie_base = enter_lot(&root, &store, "UserData", 7, &[3], no_frames).unwrap();
    assert!(newbie_base.world().neighbors.first_of(made_guid).is_some());
    assert!(newbie_base.world().characters.made.contains_key(&made_guid));
}

#[test]
fn base_house_enter_lot_errors_when_visit_objects_stops() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("house56-stopped");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();
    let no_frames = Frames::parse("none").unwrap();
    // House 56 stops mid-visit in `visit_objects`; `enter_lot` must return Err
    // rather than an incomplete `Lot`.
    let err = enter_lot(&root, &store, "UserData", 56, &[3], no_frames)
        .err()
        .expect("House 56 incomplete load must return Err");
    assert!(
        err.to_string().contains("did not finish"),
        "unexpected error: {err}"
    );
}

#[test]
fn userdata2_v7_family_upgrades_to_v9_and_persists_magic_history() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("userdata2-v7-upgrade");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData2", &root.join("UserData2"), 0xff)
        .unwrap();
    let before = store.load_neighborhood("UserData2").unwrap().unwrap();
    let fam_before = before.family_in(9).expect("UserData2 House 9 occupied").1;
    assert_eq!(fam_before.version, 7);

    let no_frames = Frames::parse("none").unwrap();
    let mut lot9 = enter_lot(&root, &store, "UserData2", 9, &[3], no_frames).unwrap();
    save_and_leave_lot(&root, &store, "UserData2", &mut lot9).unwrap();

    let after = store.load_neighborhood("UserData2").unwrap().unwrap();
    for fam in after.families.values() {
        assert_eq!(
            fam.version, 9,
            "all families upgrade to v9 on save (0x410696)"
        );
    }
}

#[test]
fn concurrent_move_in_only_allows_one_winner() {
    let db = TempDb::new("concurrent-move-in");
    let store = std::sync::Arc::new(NeighborhoodStore::open(&db.0).unwrap());

    let mut families = BTreeMap::new();
    families.insert(2, synthetic_family(0, 20_000, 0, vec![0xAAA1], 0));
    families.insert(5, synthetic_family(0, 20_000, 0, vec![0xBBB1], 0));
    let mut zoning = BTreeMap::new();
    zoning.insert(4, 1);
    let nh = StoredNeighborhood {
        name: "Race".into(),
        families,
        neighbors: Vec::new(),
        inventories: Vec::new(),
        data: [0; 16],
        options: None,
        type_attributes: None,
        zoning,
        lot_rows: vec![4],
        made_characters: Characters::default(),
        deleted_characters: BTreeSet::new(),
        instance_id: 77,
        revision: 0,
    };
    store.save_neighborhood("Race", &nh).unwrap();
    store
        .save_base_house("Race", 4, &synthetic_house_iff(5_500, 0, 0))
        .unwrap();

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let s1 = std::sync::Arc::clone(&store);
    let b1 = std::sync::Arc::clone(&barrier);
    let t1 = std::thread::spawn(move || {
        b1.wait();
        move_in_with_multiplier(&s1, "Race", 2, 4, DEFAULT_ARCH_VALUE_MULTIPLIER)
    });
    let s2 = std::sync::Arc::clone(&store);
    let b2 = std::sync::Arc::clone(&barrier);
    let t2 = std::thread::spawn(move || {
        b2.wait();
        move_in_with_multiplier(&s2, "Race", 5, 4, DEFAULT_ARCH_VALUE_MULTIPLIER)
    });
    let r1 = t1.join().unwrap();
    let r2 = t2.join().unwrap();
    assert_eq!(
        usize::from(r1.is_ok()) + usize::from(r2.is_ok()),
        1,
        "concurrent move_in into the same house must yield exactly one winner: r1={r1:?} r2={r2:?}"
    );
    let after = store.load_neighborhood("Race").unwrap().unwrap();
    assert_eq!(after.revision, 1);
    let winner = if r1.is_ok() { 2 } else { 5 };
    let loser = if r1.is_ok() { 5 } else { 2 };
    assert_eq!(after.families[&winner].house, 4);
    assert_eq!(after.families[&loser].house, 0);
}

#[test]
fn reimport_cross_store_and_restore_cannot_bypass_save_guards() {
    let Some(root) = install() else {
        return;
    };
    let db_a = TempDb::new("guard-store-a");
    let db_b = TempDb::new("guard-store-b");
    let store_a = NeighborhoodStore::open(&db_a.0).unwrap();
    let store_b = NeighborhoodStore::open(&db_b.0).unwrap();
    store_a
        .import_directory("same_id", &root.join("UserData"), 0xff)
        .unwrap();
    store_b
        .import_directory("same_id", &root.join("UserData"), 0xff)
        .unwrap();
    let no_frames = Frames::parse("none").unwrap();

    // 1. Cross-store save with the SAME neighborhood ID and revision 0 is rejected.
    let mut goth_a = enter_lot(&root, &store_a, "same_id", 5, &[3], no_frames.clone()).unwrap();
    let b_before = store_b.load_neighborhood("same_id").unwrap().unwrap();
    assert!(matches!(
        save_and_leave_lot(&root, &store_b, "same_id", &mut goth_a),
        Err(LifecycleError::NeighborhoodMismatch { .. })
    ));
    assert_eq!(
        store_b.load_neighborhood("same_id").unwrap().unwrap(),
        b_before
    );
    assert!(store_b.load_saved_lot("same_id", 5).unwrap().is_none());

    // 2. Re-importing a different neighborhood (`UserData2`) under the SAME ID
    //    and revision 0 invalidates the previously opened lot.
    store_a
        .import_directory("same_id", &root.join("UserData2"), 0xff)
        .unwrap();
    let ud2_before = store_a.load_neighborhood("same_id").unwrap().unwrap();
    assert!(matches!(
        save_and_leave_lot(&root, &store_a, "same_id", &mut goth_a),
        Err(LifecycleError::NeighborhoodMismatch { .. })
    ));
    assert_eq!(
        store_a.load_neighborhood("same_id").unwrap().unwrap(),
        ud2_before
    );

    // 3. Restoring an older lot snapshot via `Lot::restore` clears store origin
    //    so Save & Leave cannot overwrite a newer committed `move_in`.
    let db_c = TempDb::new("guard-restore");
    let store_c = NeighborhoodStore::open(&db_c.0).unwrap();
    store_c
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();
    let mut goth_v0 = enter_lot(&root, &store_c, "UserData", 5, &[3], no_frames.clone()).unwrap();
    save_and_leave_lot(&root, &store_c, "UserData", &mut goth_v0).unwrap();
    let old_snapshot = goth_v0.save().unwrap();
    move_in(&root, &store_c, "UserData", 2, 4).unwrap();
    let mut goth_v1 = enter_lot(&root, &store_c, "UserData", 5, &[3], no_frames).unwrap();
    goth_v1.restore(old_snapshot).unwrap();
    assert!(
        matches!(
            save_and_leave_lot(&root, &store_c, "UserData", &mut goth_v1),
            Err(LifecycleError::Lot(_))
        ),
        "saving a lot after Lot::restore must be rejected"
    );
    let after_c = store_c.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(after_c.families[&2].house, 4);
}

#[test]
fn bulldozed_made_and_far_tombstones_never_reappear_and_inline_chars_are_writable() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("bulldoze-tombstone-persistence");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();
    let no_frames = Frames::parse("none").unwrap();

    // 1. Add a runtime-made tombstone bound to House 5, save House 7 so the
    //    saved House 7 snapshot contains `0xDEADBEEF` in its world.characters.
    let mut nh = store.load_neighborhood("UserData").unwrap().unwrap();
    let template_guid = nh.families[&5].members[0];
    let made_guid = 0xDEAD_BEEFu32;
    let next_id = nh
        .neighbors
        .iter()
        .flatten()
        .map(|n| n.id)
        .max()
        .unwrap_or(0)
        + 1;
    nh.made_characters.made.insert(
        made_guid,
        sims_sim::characters::Character {
            template: template_guid,
            file: 999,
            strings: BTreeMap::new(),
        },
    );
    nh.made_characters.next_file = 1000;
    let mut pdata = vec![0i16; sims_sim::neighbors::PERSON_WORDS];
    pdata[67] = 5; // PERSON_HOME = House 5
    nh.neighbors
        .push(Some(sims_formats::neighborhood::Neighbor {
            version: 10,
            version_extra: Some(1),
            name: "MadeTombstone".into(),
            unknown_zero: 0,
            person_mode: 9,
            person_data: pdata,
            id: next_id,
            guid: made_guid,
            unknown_negative: -1,
            relationships: Vec::new(),
        }));
    store.save_neighborhood("UserData", &nh).unwrap();

    let mut house7 = enter_lot(&root, &store, "UserData", 7, &[3], no_frames.clone()).unwrap();
    // Verify inline loose character libraries are writable regardless of CWD.
    let loose_char_guid = nh.families[&5].members[0];
    let loose_char_res = house7.catalog().object(loose_char_guid).unwrap().resource;
    assert!(
        house7.program().libraries[&loose_char_res].writable,
        "inline loose character files must always have Library::writable == true"
    );
    save_and_leave_lot(&root, &store, "UserData", &mut house7).unwrap();

    // 2. Also pack Goth tombstone `0x7757D9EE` inside a FAR archive so its bytes
    //    stay in `NEIGHBORHOOD_CHARACTERS` even after bulldoze!
    let chars = store.load_character_files("UserData").unwrap();
    let (tomb_key, tomb_bytes) = chars
        .iter()
        .find(|(_, b)| {
            sims_formats::iff::Iff::parse((*b).clone()).is_ok_and(|iff| {
                iff.chunks.iter().any(|c| {
                    &c.tag == b"OBJD"
                        && iff.data(c).len() >= 32
                        && u32::from_le_bytes(iff.data(c)[28..32].try_into().unwrap())
                            == 0x7757_D9EE
                })
            })
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .expect("found Goth tombstone IFF");
    let member_name = tomb_key.as_bytes();
    let data_offset = 16u32;
    let manifest_offset = data_offset + tomb_bytes.len() as u32;
    let mut far = Vec::new();
    far.extend_from_slice(b"FAR!byAZ");
    far.extend_from_slice(&1u32.to_le_bytes());
    far.extend_from_slice(&manifest_offset.to_le_bytes());
    far.extend_from_slice(&tomb_bytes);
    far.extend_from_slice(&1u32.to_le_bytes());
    far.extend_from_slice(&(tomb_bytes.len() as u32).to_le_bytes());
    far.extend_from_slice(&(tomb_bytes.len() as u32).to_le_bytes());
    far.extend_from_slice(&data_offset.to_le_bytes());
    far.extend_from_slice(&(member_name.len() as u32).to_le_bytes());
    far.extend_from_slice(member_name);
    store
        .save_character_file("UserData", "tombstone_pack.far", &far)
        .unwrap();

    // 3. Bulldoze House 5: removes BOTH `0xDEADBEEF` and `0x7757D9EE`.
    bulldoze(&root, &store, "UserData", 5).unwrap();

    // 4. Re-enter & re-save the pre-existing saved House 7, then re-enter again:
    //    neither `0xDEADBEEF` nor FAR-archived `0x7757D9EE` may reappear!
    let mut house7_re = enter_lot(&root, &store, "UserData", 7, &[3], no_frames.clone()).unwrap();
    assert!(house7_re.world().neighbors.first_of(made_guid).is_none());
    assert!(!house7_re.world().characters.made.contains_key(&made_guid));
    assert!(house7_re.world().neighbors.first_of(0x7757_D9EE).is_none());
    save_and_leave_lot(&root, &store, "UserData", &mut house7_re).unwrap();

    let house7_again = enter_lot(&root, &store, "UserData", 7, &[3], no_frames).unwrap();
    assert!(house7_again.world().neighbors.first_of(made_guid).is_none());
    assert!(
        house7_again
            .world()
            .neighbors
            .first_of(0x7757_D9EE)
            .is_none()
    );
}

#[test]
fn copied_redb_and_save_neighborhood_writes_invalidate_open_lots() {
    let Some(root) = install() else {
        return;
    };
    let no_frames = Frames::parse("none").unwrap();

    // 1. Copy an imported REDB on disk: opening from the original and saving
    //    into the copy must fail with NeighborhoodMismatch and leave copy intact.
    let db_orig = TempDb::new("copied-orig");
    let db_copy = TempDb::new("copied-clone");
    {
        let s = NeighborhoodStore::open(&db_orig.0).unwrap();
        s.import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
    }
    std::fs::copy(&db_orig.0, &db_copy.0).unwrap();
    let store_orig = NeighborhoodStore::open(&db_orig.0).unwrap();
    let store_copy = NeighborhoodStore::open(&db_copy.0).unwrap();
    let mut goth_orig =
        enter_lot(&root, &store_orig, "UserData", 5, &[3], no_frames.clone()).unwrap();
    assert!(matches!(
        save_and_leave_lot(&root, &store_copy, "UserData", &mut goth_orig),
        Err(LifecycleError::NeighborhoodMismatch { .. })
    ));
    assert!(store_copy.load_saved_lot("UserData", 5).unwrap().is_none());

    // 2. Public `save_neighborhood` overwriting an existing neighborhood bumps
    //    its revision so an already-open lot cannot overwrite it.
    let mut nh = store_orig.load_neighborhood("UserData").unwrap().unwrap();
    nh.families.get_mut(&2).unwrap().house = 4;
    store_orig.save_neighborhood("UserData", &nh).unwrap();
    assert!(matches!(
        save_and_leave_lot(&root, &store_orig, "UserData", &mut goth_orig),
        Err(LifecycleError::StaleNeighborhood { .. })
    ));
    assert_eq!(
        store_orig
            .load_neighborhood("UserData")
            .unwrap()
            .unwrap()
            .families[&2]
            .house,
        4
    );
}

#[test]
fn bulldoze_occupied_newbie_house_7_evicts_and_clears_lot_atomically() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("bulldoze-occupied-newbie");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();

    let before_nh = store.load_neighborhood("UserData").unwrap().unwrap();
    let newbie_before = before_nh.families[&1].clone();
    let expected_net_worth = newbie_before.budget + newbie_before.architecture_value;

    let dozed = bulldoze(&root, &store, "UserData", 7).unwrap();
    assert_eq!(dozed.evicted_family, Some(1));
    assert_eq!(dozed.refund, newbie_before.architecture_value);
    assert!(dozed.removed_objects > 5);
    assert!(dozed.cleared_tiles > 0);
    assert_eq!(dozed.remaining_objects_value, 0);
    assert_eq!(dozed.remaining_architecture_value, 0);

    let after_nh = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(after_nh.families[&1].house, 0);
    assert_eq!(after_nh.families[&1].architecture_value, 0);
    assert_eq!(after_nh.families[&1].budget, expected_net_worth);
    assert!(after_nh.family_in(7).is_none());
    assert_eq!(
        lot_price(&store, "UserData", 7, DEFAULT_ARCH_VALUE_MULTIPLIER).unwrap(),
        6_000
    );
}
