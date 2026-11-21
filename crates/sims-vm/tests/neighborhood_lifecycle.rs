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
use std::collections::BTreeMap;
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

    let save_goth = save_and_leave_lot(&root, &store, "UserData", &goth).unwrap();
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
    let save_newbie = save_and_leave_lot(&root, &store, "UserData", &newbie).unwrap();
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

    // First entry runs flag-4 move-in reset (0x42848d..0x4284f9).
    let no_frames = Frames::parse("none").unwrap();
    let mut lot4 = enter_lot(&root, &store, "UserData", 4, &[3], no_frames).unwrap();
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
    let saved = save_and_leave_lot(&root, &store, "UserData", &lot4).unwrap();
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

    // 2. Bulldoze unoccupied House 5: clears walls, floors, doors, windows,
    //    and pool so the lot price returns to bare lot_value ($7,000).
    let dozed = bulldoze(&root, &store, "UserData", 5).unwrap();
    assert_eq!(dozed.evicted_family, None);
    assert_eq!(dozed.refund, 0);
    assert!(dozed.cleared_tiles > 0);
    assert_eq!(dozed.remaining_objects_value, 0);
    assert_eq!(dozed.remaining_architecture_value, 0);
    assert_eq!(
        lot_price(&store, "UserData", 5, DEFAULT_ARCH_VALUE_MULTIPLIER).unwrap(),
        7_000
    );

    // 3. Move the Goths back onto their freshly bulldozed lot and enter it!
    let moved_back = move_in(&root, &store, "UserData", 5, 5).unwrap();
    assert_eq!(moved_back.lot_price, 7_000);
    assert_eq!(moved_back.budget_after, expected_net_worth - 7_000);
    let no_frames = Frames::parse("none").unwrap();
    let mut lot5 = enter_lot(&root, &store, "UserData", 5, &[3], no_frames).unwrap();
    assert_eq!(lot5.world().budget.funds, expected_net_worth - 7_000);
    assert_eq!(lot5.world().globals[0], 7);
    for _ in 0..5 {
        lot5.step().unwrap();
    }
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
