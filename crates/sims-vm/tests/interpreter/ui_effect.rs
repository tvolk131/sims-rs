//! UI Effect (opcode 34, 0x44acff): interface highlight control for the
//! Tutorial and Help System, including mode 0's fallthrough into mode 1
//! (`0x47689c` → `0x4e92a8`), `BMP_` 2002 vs dirty rebuild (`0x4e92e5`,
//! `0x4e9338`, `0x4e938c`), `0x4e9652` off-lot failures, duplicate catalog
//! GUIDs (`0x4e8b3d`), runtime-made characters (`0x4e8add`), mode 2's
//! read-only relationship highlight (`0x476984`), and global 34's
//! uninitialized `[object + 0x128]` write (`0x45412b`). Synthetic; no game
//! data.

use crate::create_person::visitors;
use crate::factory::WIDGET;
use crate::person_factory::equip;
use crate::support::*;
use sims_content::Object as CatalogObject;
use sims_sim::neighbors::{Family, Neighbor};
use sims_vm::Trap;

/// UI Effect (opcode 34).
const UI_EFFECT: u16 = 34;

/// Runs UI Effect on `object` (tree 4096 in `file`) with `(mode, scope, index, flags)`.
fn run_ui_effect(
    fx: &mut Fixture,
    object: u16,
    file: usize,
    mode: u8,
    var_scope: u8,
    index: i16,
    flags: u8,
) -> Result<bool, sims_vm::Fault> {
    let [lo, hi] = index.to_le_bytes();
    fx.add(
        file,
        4096,
        tree(vec![ins(
            UI_EFFECT,
            RETURN_TRUE,
            RETURN_FALSE,
            [mode, var_scope, lo, hi, flags, 0, 0, 0],
        )]),
    );
    fx.run_with(object, 4096, 0, &[], 1000)
}

#[test]
fn mode_0_falls_through_to_mode_1_and_builds_neighbors_portrait_once() {
    // Both mode 0 (0x44ad3f -> fallthrough to 0x44ad61) and mode 1 (0x44ad61)
    // run 0x47689c -> 0x4e92a8, regardless of whether flag bit 0 is 0 or 1,
    // and on both non-person (LAMP) and person (SIM) callers. Without BMP_
    // 2002 on the widget's file, 0x4e92a8 falls through to 0x4e938c (0x4e8c5d).
    for (mode, flags, caller, file) in [
        (0u8, 1u8, LAMP, LAMP_FILE),
        (0, 0, SIM, SIM_FILE),
        (1, 1, LAMP, LAMP_FILE),
        (1, 0, SIM, SIM_FILE),
    ] {
        let (mut fx, _, record) = visitors();
        let before_objects = fx.world.objects.clone();
        assert_eq!(
            run_ui_effect(&mut fx, caller, file, mode, scope::LITERAL, record, flags),
            Ok(true),
            "mode {mode} flags {flags} caller {caller}"
        );
        assert_eq!(
            fx.world.portraits,
            [WIDGET].into(),
            "mode {mode} flags {flags}"
        );
        assert!(
            fx.world.skeleton_files.contains("widget.bcf"),
            "mode {mode} flags {flags}: temporary person should have read its skeleton"
        );
        assert_eq!(
            fx.world.objects.keys().collect::<Vec<_>>(),
            before_objects.keys().collect::<Vec<_>>(),
            "mode {mode} flags {flags}: temporary person should be removed"
        );

        // A second call finds the portrait already cached and changes nothing
        // (no CRT RNG draw or rebuild).
        let after_first = hash_without_frames(&fx.world);
        assert_eq!(
            run_ui_effect(&mut fx, caller, file, mode, scope::LITERAL, record, flags),
            Ok(true),
            "second call mode {mode}"
        );
        assert_eq!(hash_without_frames(&fx.world), after_first);
    }
}

#[test]
fn clean_slot_with_bmp_2002_caches_without_creating_a_sim_while_dirty_slot_renders_first() {
    // Clean, empty slot (0x4e88fa) with BMP_ 2002 present (0x4e9338 -> 0x4e9260):
    // caches the portrait with zero Sim creation, zero skeleton read, zero CRT RNG
    // draw, and zero Init execution.
    let (mut fx, file, record) = visitors();
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .has_portrait_2002 = true;
    let crt_before = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert!(fx.world.skeleton_files.is_empty());
    assert_eq!(fx.world.crt_rng, crt_before);

    // Dirty slot (0x4e895e, e.g. after Create New Object Instance with person
    // word 84/68/74 set) with BMP_ 2002 present: 0x4e92e5 calls 0x4e8c5d FIRST
    // (0x4e92f2, save flag 0), creating and removing the temporary person and
    // clearing dirty without recording a generated writeback.
    let (mut fx, file, record) = visitors();
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .has_portrait_2002 = true;
    fx.world.dirty_portraits.insert(WIDGET);
    let crt_before = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert!(!fx.world.dirty_portraits.contains(&WIDGET));
    assert!(fx.world.generated_portrait_2002.is_empty());
    assert!(fx.world.skeleton_files.contains("widget.bcf"));
    assert_ne!(fx.world.crt_rng, crt_before);
}

#[test]
fn read_only_off_lot_skips_factory_and_fallback_render_persists_bmp_2002() {
    // 1. Read-only off-lot definition (0x4e8cc6 -> 0x4067b3 -> 0x4174dc, e.g.
    // People\TemplatePerson.iff in Objects.far without BMP_ 2002): skips
    // 0x4e9652 completely (no Init, no CRT RNG draw, portrait stays uncached,
    // returns true).
    let (mut fx, file, record) = visitors();
    fx.program.libraries.get_mut(&file).unwrap().read_only = true;
    let crt_before = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert!(fx.world.portraits.is_empty());
    assert!(fx.world.skeleton_files.is_empty());
    assert_eq!(fx.world.crt_rng, crt_before);

    // 2. Read-only on-lot person: 0x4e9608 finds the on-lot Sim and skips the
    // 0x4e8cc6 writability gate, caching the portrait without persisting a
    // generated BMP_ 2002 to the read-only file.
    let (mut fx, _, _) = visitors();
    fx.program.libraries.get_mut(&SIM_FILE).unwrap().read_only = true;
    fx.world.neighbors.slots.push(Some(Neighbor {
        guid: SIM_GUID,
        id: 2,
        data: vec![0; 88],
        relationships: Vec::new(),
        friends: 0,
        revision: 0,
        stale: false,
    }));
    let sim_record = fx.world.neighbors.slots.len() as i16;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [SIM_GUID].into());
    assert!(fx.world.generated_portrait_2002.is_empty());

    // 3. Runtime-made character cloned from a read-only template lives in a
    // loose writable Characters/User%05d.iff file (0x4260e1), so off-lot
    // rendering succeeds and persists BMP_ 2002.
    let (mut fx, file, _) = visitors();
    fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
    fx.program.libraries.get_mut(&file).unwrap().read_only = true;
    let made_guid = 0x5555_0002u32;
    fx.world.characters.made.insert(
        made_guid,
        sims_sim::characters::Character {
            template: WIDGET,
            file: 8,
            strings: Default::default(),
        },
    );
    fx.world.neighbors.slots.push(Some(Neighbor {
        guid: made_guid,
        id: 52,
        data: vec![12; 88],
        relationships: Vec::new(),
        friends: 0,
        revision: 1,
        stale: false,
    }));
    let made_record = fx.world.neighbors.slots.len() as i16;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, made_record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [made_guid].into());
    assert!(fx.world.generated_portrait_2002.contains(&made_guid));

    // 4. Fallback render at 0x4e938c (save flag 1) persists BMP_ 2002 via
    // 0x4e8d2e -> 0x4e9000 -> 0x4e8feb: after a subsequent 0x4e895e
    // invalidation (dirty = 1, bitmap cleared), a failed dirty render at
    // 0x4e92f2 loads the persisted BMP_ 2002 at 0x4e9338 instead of running
    // the factory a second time at 0x4e938c!
    let (mut fx, file, record) = visitors();
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert!(fx.world.generated_portrait_2002.contains(&WIDGET));

    // Simulate 0x4e895e invalidation, and make placement fail on the next
    // attempt while counting Init runs in global 36.
    fx.world.portraits.remove(&WIDGET);
    fx.world.dirty_portraits.insert(WIDGET);
    fx.add(
        file,
        4096,
        tree(vec![
            ins(2, 1, 1, expr(scope::GLOBAL, 36, op::ADD, scope::LITERAL, 1)),
            assign(scope::MY_DATA, 4, scope::LITERAL, 1, 2),
            assign(scope::MY_DATA, 42, scope::LITERAL, 0, RETURN_TRUE),
        ]),
    );
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(
        fx.world.globals[36], 2,
        "persisted BMP_ 2002 satisfies 0x4e9338 after 0x4e92f2 fails (no second render)"
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert!(!fx.world.dirty_portraits.contains(&WIDGET));
}

#[test]
fn mode_1_reads_any_variable_scope_and_skips_missing_or_non_person_neighbors() {
    // Reading neighbor ID from temp 0 (scope 0x08).
    let (mut fx, _, record) = visitors();
    fx.lamp_mut().temps[0] = record;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::TEMPS, 0, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());

    // Zero, negative, or out-of-range neighbor IDs do nothing and return true.
    for missing in [0i16, -1, 99] {
        let (mut fx, _, _) = visitors();
        let mut expected = fx.world.clone();
        expected.objects.get_mut(&LAMP).unwrap().last_result = true;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, missing, 1),
            Ok(true),
            "missing {missing}"
        );
        assert!(fx.world.portraits.is_empty());
        assert_eq!(hash_without_frames(&fx.world), expected.hash());
    }

    // Both a missing GUID (0xDEAD_BEEF) and a registered non-person GUID
    // (LAMP_GUID) are absent from the portrait table (0x4e9753 returns -1)
    // -> no-op, true.
    for non_person_guid in [0xDEAD_BEEF, LAMP_GUID] {
        let (mut fx, _, _) = visitors();
        fx.world.neighbors.slots.push(Some(Neighbor {
            guid: non_person_guid,
            id: 50,
            data: vec![0; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 0,
            stale: false,
        }));
        let non_person_record = fx.world.neighbors.slots.len() as i16;
        let mut expected = fx.world.clone();
        expected.objects.get_mut(&LAMP).unwrap().last_result = true;
        assert_eq!(
            run_ui_effect(
                &mut fx,
                LAMP,
                LAMP_FILE,
                1,
                scope::LITERAL,
                non_person_record,
                1
            ),
            Ok(true),
            "non_person_guid 0x{non_person_guid:08X}"
        );
        assert!(fx.world.portraits.is_empty());
        assert_eq!(hash_without_frames(&fx.world), expected.hash());
    }
}

#[test]
fn mode_1_handles_on_lot_household_and_runtime_made_characters_without_trapping() {
    // When a person of that type is already on the lot (0x4e9608), no
    // temporary person is constructed and the portrait is cached.
    let (mut fx, _, _) = visitors();
    fx.world.neighbors.slots.push(Some(Neighbor {
        guid: SIM_GUID,
        id: 2,
        data: vec![0; 88],
        relationships: Vec::new(),
        friends: 0,
        revision: 0,
        stale: false,
    }));
    let sim_record = fx.world.neighbors.slots.len() as i16;
    assert!(fx.world.people.contains(&SIM));
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [SIM_GUID].into());
    assert!(fx.world.skeleton_files.is_empty());

    // 0x4e9652 has no household-membership guard: an off-lot member of the
    // active household follows the ordinary 0x4e92a8 path and returns true.
    let (mut fx, _, record) = visitors();
    fx.world.globals[9] = 4;
    fx.world.families.push(Family {
        id: 4,
        house: 5,
        members: vec![WIDGET],
        friends: 0,
        stale: false,
    });
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert!(fx.world.skeleton_files.contains("widget.bcf"));

    // Runtime-made character (world.characters.made): Make New Character
    // broadcasts 0xe0 -> 0x4f6727 -> 0x4e8a44 -> 0x4e8add, adding the person
    // GUID to 0x6f2698. UI Effect follows the ordinary 0x4e92a8 path (no trap).
    let (mut fx, file, _) = visitors();
    let made_guid = 0x5555_0001u32;
    fx.world.characters.made.insert(
        made_guid,
        sims_sim::characters::Character {
            template: WIDGET,
            file: 7,
            strings: Default::default(),
        },
    );
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .has_portrait_2002 = true;
    fx.world.neighbors.slots.push(Some(Neighbor {
        guid: made_guid,
        id: 51,
        data: vec![0; 88],
        relationships: Vec::new(),
        friends: 0,
        revision: 0,
        stale: false,
    }));
    let made_record = fx.world.neighbors.slots.len() as i16;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, made_record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [made_guid].into());
    assert!(fx.world.skeleton_files.is_empty());
}

#[test]
fn mode_1_handles_off_lot_failures_and_duplicate_catalog_guids() {
    // 1. Factory returns 0 (0x4e969d, e.g. refused pet when Unleashed is not
    // installed): UI Effect returns true and leaves the portrait uncached.
    let (mut fx, _, _) = visitors();
    fx.program.expansions = 0; // Unleashed not installed
    fx.world.neighbors.slots.clear();
    equip(&mut fx, WIDGET, "kat", 12);
    let pet_record = fx.world.neighbors.slots.len() as i16;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, pet_record, 1),
        Ok(true)
    );
    assert!(fx.world.portraits.is_empty());

    // Helper: make WIDGET's Init (tree 4096) increment global 36 and leave
    // data word 42 (allowed surfaces) at 0 so placement finds no tile from any
    // start (0x4e9706). Note that `create_person` runs Init twice per creation
    // (0x422d46), so each render attempt adds 2 to global 36.
    let count_init_unplaceable = |fx: &mut Fixture, file: usize| {
        fx.add(
            file,
            4096,
            tree(vec![
                ins(2, 1, 1, expr(scope::GLOBAL, 36, op::ADD, scope::LITERAL, 1)),
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 2),
                assign(scope::MY_DATA, 42, scope::LITERAL, 0, RETURN_TRUE),
            ]),
        );
    };

    // 2. Placement finds no tile (0x4e9706): on a clean slot without BMP_
    // 2002, 0x4e938c runs once, Init & CRT RNG side effects are kept, the
    // temporary person is removed, portrait stays uncached, and UI Effect
    // returns true.
    let (mut fx, file, record) = visitors();
    count_init_unplaceable(&mut fx, file);
    let crt_before = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(
        fx.world.globals[36], 2,
        "clean no-tile runs 0x4e938c once (2 Inits)"
    );
    assert_ne!(fx.world.crt_rng, crt_before);
    assert!(fx.world.portraits.is_empty());

    // 3. Dirty slot with no tile and BMP_ 2002 present: 0x4e92f2 tries once
    // (0x4e9706 keeps Init/RNG), then falls through to 0x4e9338 (BMP_ 2002),
    // caching the portrait without a second render!
    let (mut fx, file, record) = visitors();
    fx.world.dirty_portraits.insert(WIDGET);
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .has_portrait_2002 = true;
    count_init_unplaceable(&mut fx, file);
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(
        fx.world.globals[36], 2,
        "dirty no-tile + BMP_ 2002 builds once (2 Inits)"
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert!(!fx.world.dirty_portraits.contains(&WIDGET));

    // 4. Dirty slot with no tile and NO BMP_ 2002: 0x4e92f2 tries once,
    // 0x4e9338 misses, and 0x4e938c retries a second time!
    let (mut fx, file, record) = visitors();
    fx.world.dirty_portraits.insert(WIDGET);
    count_init_unplaceable(&mut fx, file);
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(
        fx.world.globals[36], 4,
        "dirty no-tile without BMP_ 2002 builds twice (0x4e92f2 + 0x4e938c = 4 Inits)"
    );
    assert!(fx.world.portraits.is_empty());
    assert!(fx.world.dirty_portraits.contains(&WIDGET));

    // 5. Duplicate catalog GUID (0x4e8b3d): an earlier person definition puts
    // WIDGET in 0x6f2698, while a later non-person definition overwrites
    // by_guid (0x43ac10). UI Effect finds WIDGET in 0x6f2698 and resolves the
    // winning non-person definition (0x4e9260 uses its BMP_ 2002 when present;
    // without BMP_ 2002, 0x4e9692 creates the non-person and 0x4e96b7 fails
    // dynamic_cast<cXPerson*>, leaving it uncached and returning true).
    let (mut fx, _, record) = visitors();
    let winner_index = fx.catalog.objects.len();
    fx.catalog.objects.push(CatalogObject {
        guid: WIDGET,
        resource: LAMP_FILE,
        chunk: 128,
        kind: 4, // normal non-person object
        master: 0,
    });
    fx.catalog.by_guid.insert(WIDGET, winner_index);
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .has_portrait_2002 = true;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());

    // Duplicate catalog GUID without BMP_ 2002: 0x4e9652 creates the winning
    // non-person object, fails dynamic_cast<cXPerson*>, leaves portrait
    // uncached, and returns true.
    let (mut fx, _, record) = visitors();
    // Give LAMP_FILE chunk 128 a valid Init (4098) and Main (4097) so
    // `try_create` can instantiate it cleanly.
    fx.add(
        LAMP_FILE,
        4098,
        tree(vec![assign(
            scope::MY_DATA,
            0,
            scope::LITERAL,
            0,
            RETURN_TRUE,
        )]),
    );
    fx.add(
        LAMP_FILE,
        4097,
        tree(vec![assign(
            scope::MY_DATA,
            0,
            scope::LITERAL,
            0,
            RETURN_TRUE,
        )]),
    );
    {
        let cb = fx
            .program
            .libraries
            .get_mut(&LAMP_FILE)
            .unwrap()
            .callbacks
            .get_mut(&128)
            .unwrap();
        cb[0].action = 4098;
        cb[1].action = 4097;
    }
    let winner_index = fx.catalog.objects.len();
    fx.catalog.objects.push(CatalogObject {
        guid: WIDGET,
        resource: LAMP_FILE,
        chunk: 128,
        kind: 4,
        master: 0,
    });
    fx.catalog.by_guid.insert(WIDGET, winner_index);
    let before_count = fx.world.objects.len();
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert!(fx.world.portraits.is_empty());
    assert_eq!(fx.world.objects.len(), before_count + 1);
}

#[test]
fn mode_2_reads_the_variable_without_building_any_portrait() {
    // 0x44ad1f -> 0x476984: reads the variable and looks up 0x4e9753 read-only
    // without calling 0x4e92a8, so world.portraits and skeleton_files stay
    // untouched.
    let (mut fx, _, record) = visitors();
    let mut expected = fx.world.clone();
    expected.objects.get_mut(&LAMP).unwrap().last_result = true;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 2, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert!(fx.world.portraits.is_empty());
    assert!(fx.world.skeleton_files.is_empty());
    assert_eq!(hash_without_frames(&fx.world), expected.hash());
}

#[test]
fn modes_0_to_2_validate_variable_reads_and_guard_global_34_while_modes_3_and_up_skip_them() {
    // Modes 0, 1, and 2 call 0x453141:
    // - invalid scope (> 38) raises native error 9;
    // - missing local raises native error 8;
    // - global 34 (scope 6, index 34) fails loudly because 0x45412b ->
    //   0x4541d5 writes through uninitialized [object + 0x128].
    for mode in [0u8, 1, 2] {
        let mut fx = Fixture::new();
        assert_eq!(
            code(run_ui_effect(&mut fx, LAMP, LAMP_FILE, mode, 99, 0, 1)),
            Some(9),
            "invalid scope mode {mode}"
        );
        let mut fx = Fixture::new();
        assert_eq!(
            code(run_ui_effect(
                &mut fx,
                LAMP,
                LAMP_FILE,
                mode,
                scope::LOCALS,
                4,
                1
            )),
            Some(8),
            "missing local mode {mode}"
        );
        let mut fx = Fixture::new();
        let res = run_ui_effect(&mut fx, LAMP, LAMP_FILE, mode, scope::GLOBAL, 34, 1);
        assert!(
            matches!(trap(res), Trap::Unsupported(ref m) if m.contains("0x45412b")),
            "global 34 mode {mode}"
        );
    }

    // Modes 3 and up (0x44ad1d jne 0x44ad83) skip 0x453141 completely and
    // return true even with an invalid scope or global 34.
    for mode in [3u8, 4, 128, 255] {
        let mut fx = Fixture::new();
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, mode, 99, 0, 1),
            Ok(true),
            "mode {mode} invalid scope"
        );
        let mut fx = Fixture::new();
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, mode, scope::GLOBAL, 34, 1),
            Ok(true),
            "mode {mode} global 34"
        );
    }
}
