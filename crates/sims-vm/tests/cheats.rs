//! Live-lot native cheat APIs: `move_objects`, `add_funds`, and `set_hour`.

use sims_sim::neighborhood_store::NeighborhoodStore;
use sims_vm::boot::{Frames, Lot, OrderError};
use sims_vm::lifecycle::{enter_lot, save_and_leave_lot};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn install() -> Option<PathBuf> {
    std::env::var_os("SIMS_INSTALL").map(PathBuf::from)
}

struct TempDb(PathBuf);

impl TempDb {
    fn new(tag: &str) -> Self {
        let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sims-vm-cheats-{tag}-{}-{n}.redb",
            std::process::id()
        ));
        Self(path)
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn load_goth_lot(tag: &str) -> Option<(PathBuf, TempDb, NeighborhoodStore, Lot)> {
    let root = install()?;
    let db = TempDb::new(tag);
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();
    let lot = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    Some((root, db, store, lot))
}

/// Finds a single-tile ground furniture object on Goth `House05` that is not a
/// person, indicator, sentinel, surface container with children, or controller.
fn find_movable_single_tile_furniture(lot: &Lot) -> (u16, [i32; 2], i32) {
    let world = lot.world();
    for (&id, obj) in &world.objects {
        if obj.person.is_some()
            || obj.multipart.is_some()
            || world.indicators.contains(&id)
            || obj.position[0] < 0
            || obj.position[1] < 0
            || obj.position[2] != 1
            || obj.data.get(26).copied().unwrap_or(0) != 0
            || obj.slots.iter().any(|s| s.object > 0)
        {
            continue;
        }
        let Some(def) = lot.catalog().object(obj.guid) else {
            continue;
        };
        if def.kind != 4 {
            continue;
        }
        let Some(words) = lot.program().definition(def.resource, def.chunk) else {
            continue;
        };
        if words[42] != 0 {
            continue;
        }
        // Check that strict mode allows picking it up (not in-use, deletable).
        if obj.data.get(43).copied().unwrap_or(0) & 2 == 0 {
            continue;
        }
        if obj.data.get(6).copied().unwrap_or(0) != 0 {
            continue;
        }
        return (id, [obj.position[0], obj.position[1]], obj.position[2]);
    }
    panic!("expected at least one single-tile movable furniture object on Goth House05");
}

#[test]
fn add_funds_and_set_hour_mutate_live_world_and_preserve_undo_history() {
    let Some((root, _db, store, mut lot)) = load_goth_lot("funds-hour-undo") else {
        return;
    };

    // Default move_objects is false.
    assert!(!lot.move_objects());

    // Push a real Buy Mode undo entry so we can prove add_funds & set_hour
    // preserve undo history and neighborhood_origin (unlike Lot::restore).
    let (item_id, orig_pos, level) = find_movable_single_tile_furniture(&lot);
    let mut cursor = lot
        .buy_pick_up(item_id)
        .unwrap()
        .expect("strict pickup succeeds on free furniture");
    let shifted = [orig_pos[0] + 16, orig_pos[1]];
    let moved_pos = if lot.buy_move(&mut cursor, shifted, level).unwrap() {
        shifted
    } else if lot
        .buy_move(&mut cursor, [orig_pos[0], orig_pos[1] + 16], level)
        .unwrap()
    {
        [orig_pos[0], orig_pos[1] + 16]
    } else {
        assert!(lot.buy_move(&mut cursor, orig_pos, level).unwrap());
        orig_pos
    };
    let committed_id = lot.buy_commit(&cursor).unwrap();
    assert_eq!(committed_id, item_id);
    assert_eq!(
        [
            lot.world().objects[&item_id].position[0],
            lot.world().objects[&item_id].position[1],
        ],
        moved_pos
    );

    // 1. add_funds positive, negative, saturating, and global 13/14 (base-10000) parity.
    let start_funds = lot.world().budget.funds;
    let after_plus = lot.add_funds(1000).unwrap();
    assert_eq!(after_plus, start_funds + 1000);
    assert_eq!(lot.world().budget.funds, start_funds + 1000);
    assert_eq!(
        i32::from(lot.world().globals[14]) * 10_000 + i32::from(lot.world().globals[13]),
        after_plus
    );

    let after_minus = lot.add_funds(-250).unwrap();
    assert_eq!(after_minus, start_funds + 750);
    assert_eq!(lot.world().budget.funds, start_funds + 750);

    // Saturating addition never overflows.
    let saturated = lot.add_funds(i32::MAX).unwrap();
    assert_eq!(saturated, i32::MAX);
    assert_eq!(lot.world().budget.funds, i32::MAX);

    // Reset to a sane value before saving.
    lot.add_funds(-(i32::MAX - 5000)).unwrap();
    assert_eq!(lot.world().budget.funds, 5000);

    // 2. set_hour rejects >= 24, updates global 0/4/5/6, and marks rooms on period change.
    assert!(matches!(lot.set_hour(24), Err(OrderError::Refused(_))));
    assert!(matches!(lot.set_hour(255), Err(OrderError::Refused(_))));

    for (hour, expected_period) in [(0u8, 2i16), (6, 3), (12, 0), (18, 1), (23, 2)] {
        lot.set_hour(hour).unwrap();
        assert_eq!(lot.world().globals[0], i16::from(hour));
        assert_eq!(lot.world().globals[4], expected_period);
        assert_eq!(lot.world().globals[5], 0);
        assert_eq!(lot.world().globals[6], 0);
    }

    // 3. Undo history survived both add_funds and set_hour!
    assert!(
        lot.buy_undo().unwrap(),
        "add_funds and set_hour must preserve Buy Mode undo history"
    );
    assert_eq!(
        [
            lot.world().objects[&item_id].position[0],
            lot.world().objects[&item_id].position[1],
        ],
        orig_pos
    );

    // 4. Live stepping and Save & Leave still work cleanly (neighborhood_origin preserved).
    for _ in 0..10 {
        lot.step().unwrap();
    }
    let report = save_and_leave_lot(&root, &store, "UserData", &mut lot).unwrap();
    assert_eq!(report.house, 5);
}

#[test]
fn move_objects_overrides_in_use_and_overlap_while_preserving_invariants() {
    let Some((_root, _db, _store, mut lot)) = load_goth_lot("move-objects") else {
        return;
    };

    let (item_id, orig_pos, level) = find_movable_single_tile_furniture(&lot);

    // Find a second distinct occupied tile on level 1 to test overlap placement.
    let occupied_pos = lot
        .world()
        .objects
        .iter()
        .find_map(|(&id, obj)| {
            if id != item_id
                && obj.person.is_none()
                && !lot.world().indicators.contains(&id)
                && obj.position[0] >= 16
                && obj.position[1] >= 16
                && obj.position[2] == 1
                && (obj.position[0] != orig_pos[0] || obj.position[1] != orig_pos[1])
                && lot.catalog().object(obj.guid).is_some_and(|d| d.kind == 4)
            {
                Some([obj.position[0], obj.position[1]])
            } else {
                None
            }
        })
        .expect("second occupied ground object on Goth House05");

    // 1. Find a real strict-blocked object on Goth House05 (e.g. mailbox, trash
    //    can, or non-deletable/locked-tile object) that strict mode refuses to
    //    pick up, and verify `move_objects = true` unlocks pickup and placement!
    let strict_blocked_id = lot
        .world()
        .objects
        .iter()
        .find_map(|(&id, obj)| {
            if obj.person.is_some()
                || obj.multipart.is_some()
                || lot.world().indicators.contains(&id)
                || obj.position[0] < 0
                || obj.position[1] < 0
                || obj.position[2] != 1
                || obj.slots.iter().any(|s| s.object > 0)
            {
                return None;
            }
            let def = lot.catalog().object(obj.guid)?;
            if def.kind == 7 {
                return None;
            }
            let words = lot.program().definition(def.resource, def.chunk)?;
            if words[42] != 0 {
                return None;
            }
            // Non-deletable (bit 1 clear) or in-use (data[6] != 0).
            let non_deletable = obj.data.get(43).copied().unwrap_or(0) & 2 == 0;
            let in_use = obj.data.get(6).copied().unwrap_or(0) != 0;
            (non_deletable || in_use).then_some(id)
        })
        .expect("Goth House05 has at least one non-deletable/in-use object (e.g. mailbox/trash)");

    assert!(!lot.move_objects());
    assert!(
        lot.buy_pick_up(strict_blocked_id).unwrap().is_none(),
        "strict mode must refuse picking up a non-deletable/in-use object ({strict_blocked_id})"
    );
    lot.set_move_objects(true);
    let blocked_cursor = lot
        .buy_pick_up(strict_blocked_id)
        .unwrap()
        .expect("move_objects must allow picking up a non-deletable/in-use object");
    lot.buy_snap_back(&blocked_cursor).unwrap();

    // 2. Test strict overlap rejection vs move_objects overlap placement on free furniture.
    lot.set_move_objects(false);
    let mut strict_cursor = lot
        .buy_pick_up(item_id)
        .unwrap()
        .expect("strict pickup succeeds on free furniture");
    assert!(
        !lot.buy_move(&mut strict_cursor, occupied_pos, level)
            .unwrap(),
        "strict mode must refuse overlapping placement onto an occupied tile"
    );
    lot.buy_snap_back(&strict_cursor).unwrap();

    lot.set_move_objects(true);
    assert!(lot.move_objects());
    let mut cursor = lot
        .buy_pick_up(item_id)
        .unwrap()
        .expect("move_objects allows picking up furniture");

    // 2. Overlap onto an occupied tile succeeds under move_objects, whereas
    //    off-lot coordinates and invalid levels are STILL rejected!
    assert!(
        !lot.buy_move(&mut cursor, [-16, -16], level).unwrap(),
        "move_objects must still reject negative/off-lot coordinates"
    );
    assert!(
        !lot.buy_move(&mut cursor, [64 * 16, 64 * 16], level)
            .unwrap(),
        "move_objects must still reject coordinates past lot edge"
    );
    assert!(
        !lot.buy_move(&mut cursor, occupied_pos, 0).unwrap(),
        "move_objects must still reject invalid level 0"
    );
    assert!(
        !lot.buy_move(&mut cursor, occupied_pos, 3).unwrap(),
        "move_objects must still reject invalid level 3"
    );

    assert!(
        lot.buy_move(&mut cursor, occupied_pos, level).unwrap(),
        "move_objects must allow overlapping placement on an occupied in-bounds tile"
    );
    assert!(
        lot.buy_put_if_fits(&mut cursor, level).unwrap(),
        "buy_put_if_fits must agree with move_objects override"
    );
    let placed_id = lot.buy_commit(&cursor).unwrap();
    assert_eq!(placed_id, item_id);
    assert_eq!(
        [
            lot.world().objects[&item_id].position[0],
            lot.world().objects[&item_id].position[1],
        ],
        occupied_pos
    );

    // 3. Undo & redo work cleanly under move_objects.
    assert!(lot.buy_undo().unwrap());
    assert_eq!(
        [
            lot.world().objects[&item_id].position[0],
            lot.world().objects[&item_id].position[1],
        ],
        orig_pos
    );
    assert!(lot.buy_redo().unwrap());
    assert_eq!(
        [
            lot.world().objects[&item_id].position[0],
            lot.world().objects[&item_id].position[1],
        ],
        occupied_pos
    );

    // 4. Guardrails stay enforced even with move_objects enabled:
    //    - ID 0 and nonexistent IDs refused
    //    - Persons refused
    //    - Indicator sentinels refused
    //    - Global controllers (OBJD word 42 != 0) refused
    assert!(lot.buy_pick_up(0).unwrap().is_none());
    assert!(lot.buy_pick_up(u16::MAX).unwrap().is_none());

    let person_id = *lot
        .world()
        .people
        .first()
        .expect("Goth House05 has live Sims");
    assert!(
        lot.buy_pick_up(person_id).unwrap().is_none(),
        "move_objects must never pick up a live Sim person"
    );

    for &indicator_id in &lot.world().indicators.clone() {
        assert!(
            lot.buy_pick_up(indicator_id).unwrap().is_none(),
            "move_objects must never pick up world.indicators sentinel {indicator_id}"
        );
    }

    let controller_id = lot
        .world()
        .objects
        .iter()
        .find_map(|(&id, obj)| {
            let def = lot.catalog().object(obj.guid)?;
            let words = lot.program().definition(def.resource, def.chunk)?;
            (words[42] != 0).then_some(id)
        })
        .expect("Goth House05 has at least one global controller");
    assert!(
        lot.buy_pick_up(controller_id).unwrap().is_none(),
        "move_objects must never pick up a global controller"
    );

    // 5. Turning move_objects back off restores strict pickup rejection.
    lot.set_move_objects(false);
    assert!(!lot.move_objects());
    assert!(
        lot.buy_pick_up(strict_blocked_id).unwrap().is_none(),
        "disabling move_objects must immediately restore strict pickup guards"
    );
}
