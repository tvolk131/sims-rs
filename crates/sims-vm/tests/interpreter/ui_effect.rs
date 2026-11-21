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
use sims_content::{Object as CatalogObject, Wardrobe};
use sims_sim::PortraitFile;
use sims_sim::neighbors::{Family, Neighbor};
use sims_vm::Trap;
use std::collections::BTreeMap;

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
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Made(8))
    );

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
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Resource(file))
    );

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
fn writeback_shares_iff_reloads_in_dialog_honors_reset_and_head_binding() {
    // 1. Generated BMP_ 2002 is keyed by IFF file (0x4e9000 writes into the
    // definition's IFF): a second person GUID in the same IFF loads it via
    // 0x4e9338 -> 0x4e9260 without calling 0x4e8c5d or advancing CRT RNG.
    let (mut fx, file, record1) = visitors();
    let second_guid = 0x0000_3334u32;
    let mut second_words = *fx.words(file);
    second_words[14] = second_guid as u16 as i16;
    second_words[15] = (second_guid >> 16) as u16 as i16;
    {
        let lib = fx.program.libraries.get_mut(&file).unwrap();
        lib.definitions.insert(129, second_words);
        lib.definition_order.push(129);
        let cb128 = lib.callbacks[&128].clone();
        lib.callbacks.insert(129, cb128);
    }
    let idx = fx.catalog.objects.len();
    fx.catalog.objects.push(CatalogObject {
        guid: second_guid,
        resource: file,
        chunk: 129,
        kind: 2,
        master: 0,
    });
    fx.catalog.by_guid.insert(second_guid, idx);
    equip(&mut fx, second_guid, "second", 12);
    let record2 = fx.world.neighbors.slots.len() as i16;

    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record1, 1),
        Ok(true)
    );
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Resource(file))
    );
    let crt_after_first = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record2, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET, second_guid].into());
    assert_eq!(
        fx.world.crt_rng, crt_after_first,
        "second person in same IFF should load persisted BMP_ 2002 without rendering"
    );
    assert!(!fx.world.skeleton_files.contains("second.bcf"));

    // 2. Dialog (0x4363d1 -> 0x4e92a8 -> 0x4e9338) reloads a BMP_ 2002
    // persisted by UI Effect after a clean cache eviction (0x4e897e), without
    // creating a temporary Sim or advancing CRT RNG.
    fx.world.portraits.remove(&WIDGET);
    assert!(!fx.world.dirty_portraits.contains(&WIDGET));
    let crt_before_dialog = fx.world.crt_rng;
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            36,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 0, 0, 0, 0, 0, 5], // non-blocking (bit 0), icon mode 2 (neighbor in stack object)
        )]),
    );
    assert_eq!(fx.run_with(LAMP, 4096, record1, &[], 1000), Ok(true));
    assert!(fx.world.portraits.contains(&WIDGET));
    assert_eq!(
        fx.world.crt_rng, crt_before_dialog,
        "Dialog should reload persisted BMP_ 2002 on clean slot"
    );

    // 3. Cache (0x4e8d14) and BMP_ 2002 staging (0x4e8d2e) happen BEFORE
    // temporary-person removal (0x4e8fdb): if the temporary person's Reset
    // callback (callback 3) creates another Sim of the same type with word 84
    // bit 0 set (0x44e712 -> 0x45810c -> 0x4e895e), the invalidation stays!
    let (mut fx, file, record) = visitors();
    // Mark neighbor record's person word 74 = 1 (revision >= 10 so word 74 is
    // copied) so Create mode 6 triggers 0x4e895e.
    {
        let slot = fx
            .world
            .neighbors
            .slots
            .last_mut()
            .unwrap()
            .as_mut()
            .unwrap();
        slot.revision = 10;
        slot.data[68] = 0;
        slot.data[74] = 1;
        slot.data[77] = 0;
        slot.data[81] = 0;
        slot.data[84] = 0;
    }
    // Reset callback (tree 4098): if global 36 == 0, set global 36 = 1 and
    // run Create New Object Instance mode 6 (from neighbor record in stack
    // object `record`, placed off-lot).
    fx.add(
        file,
        4098,
        tree(vec![
            ins(
                2,
                1,
                RETURN_TRUE,
                expr(scope::GLOBAL, 36, op::EQUAL, scope::LITERAL, 0),
            ),
            assign(scope::GLOBAL, 36, scope::LITERAL, 1, 2),
            assign(scope::STACK_OBJECT, 0, scope::LITERAL, record, 3),
            ins(
                42, // Create New Object Instance
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 0, 0, 0, 6, 0x04, 0, 0], // mode 6 (from neighbor), flag 0x04
            ),
        ]),
    );
    for cb in fx
        .program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .values_mut()
    {
        cb.resize_with(4, || sims_formats::execution::ObjectFunction {
            condition: 0,
            action: 0,
        });
        cb[3].action = 4098;
    }
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_ne!(
        fx.world.globals[36], 0,
        "Reset callback ran during 0x4e8fdb and created the replacement Sim"
    );
    assert!(
        !fx.world.portraits.contains(&WIDGET),
        "Reset's 0x4e895e invalidation must not be overwritten after 0x4e8fdb"
    );
    assert!(
        fx.world.dirty_portraits.contains(&WIDGET),
        "Reset's 0x4e895e dirty flag must survive 0x4e8fdb"
    );
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Resource(file)),
        "0x4e8d2e -> 0x4e8feb still flushes the staged BMP_ 2002"
    );

    // 4. Renderer failure (0x4e8d02 -> 0x4e725f -> 0x4e7824 missing HEAD
    // binding): neither on-lot nor off-lot caches the portrait, clears dirty,
    // or records generated_portrait_2002, while off-lot still creates and
    // removes the temporary Sim and UI Effect returns true.
    let (mut fx, file, record) = visitors();
    fx.program.people.portrait_head.insert(WIDGET, false);
    fx.world.dirty_portraits.insert(WIDGET);
    let crt_before = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_ne!(fx.world.crt_rng, crt_before);
    assert!(fx.world.portraits.is_empty());
    assert!(fx.world.dirty_portraits.contains(&WIDGET));
    assert!(
        !fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Resource(file))
    );

    // Same missing HEAD binding on an on-lot person (SIM): skips caching and
    // writeback, keeps dirty set, returns true.
    let (mut fx, _, _) = visitors();
    fx.program.people.portrait_head.insert(SIM_GUID, false);
    fx.world.dirty_portraits.insert(SIM_GUID);
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
    assert!(!fx.world.portraits.contains(&SIM_GUID));
    assert!(fx.world.dirty_portraits.contains(&SIM_GUID));
    assert!(fx.world.generated_portrait_2002.is_empty());

    // 5. Missing portrait_head metadata does NOT default to true, and
    // runtime-made characters check Character::strings rows 1/2 against
    // People::head_suits (or fall back to template's portrait_head).
    let (mut fx, _, record) = visitors();
    fx.program.people.portrait_head.remove(&WIDGET);
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert!(
        fx.world.portraits.is_empty(),
        "missing portrait_head metadata must not default to render success"
    );
    assert!(fx.world.generated_portrait_2002.is_empty());

    // Runtime-made character with explicit head suit string in Character::strings:
    // - without matching head_suits -> render fails (no cache/writeback);
    // - with matching head_suits -> render succeeds (caches & writes BMP_ 2002),
    //   and after clean eviction Dialog reloads it without trapping or RNG!
    let (mut fx, _, _) = visitors();
    fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
    let made_guid = 0x5555_0003u32;
    fx.world.characters.made.insert(
        made_guid,
        sims_sim::characters::Character {
            template: WIDGET,
            file: 9,
            strings: std::collections::BTreeMap::from([(
                2,
                "c001_custom,HEAD-HEAD=c001".to_owned(),
            )]),
        },
    );
    fx.world.neighbors.slots.push(Some(Neighbor {
        guid: made_guid,
        id: 53,
        data: vec![12; 256],
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
    assert!(
        fx.world.portraits.is_empty(),
        "override head suit missing from head_suits must not cache or stage writeback"
    );
    assert!(fx.world.generated_portrait_2002.is_empty());

    fx.program.people.head_suits.insert("c001_custom".into());
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, made_record, 1),
        Ok(true)
    );
    assert!(fx.world.portraits.contains(&made_guid));
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Made(9))
    );

    // Cleanly evict made_guid and run Dialog (mode 2): Dialog reloads
    // generated_portrait_2002 at 0x4e9338 without trapping on characters.made!
    fx.world.portraits.remove(&made_guid);
    let crt_before_made_dialog = fx.world.crt_rng;
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            36,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 0, 0, 0, 0, 0, 5],
        )]),
    );
    assert_eq!(fx.run_with(LAMP, 4096, made_record, &[], 1000), Ok(true));
    assert!(fx.world.portraits.contains(&made_guid));
    assert_eq!(fx.world.crt_rng, crt_before_made_dialog);

    // 6. Dialog dirty rebuild with no tile (0x4e9706) falls back to the
    // persisted BMP_ 2002 at 0x4e9338 instead of propagating a placement fault.
    let (mut fx, file, record) = visitors();
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Resource(file))
    );
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
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            36,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 0, 0, 0, 0, 0, 5],
        )]),
    );
    assert_eq!(fx.run_with(LAMP, 4096, record, &[], 1000), Ok(true));
    assert_eq!(
        fx.world.globals[36], 2,
        "Dialog dirty rebuild attempted once before falling back to 0x4e9338"
    );
    assert!(fx.world.portraits.contains(&WIDGET));
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

#[test]
fn make_new_character_wardrobe_head_binding_drives_ui_effect_render_and_writeback() {
    const TEMPLATE_PERSON: u32 = 0x7fd9_6b54;
    const TEMPLATE_DOG: u32 = 0x4a70_df92;

    let setup = |species: u8,
                 template: u32,
                 suits: &[&str],
                 textures: &[&str],
                 head_suits: &[&str]|
     -> (Fixture, i16, u32) {
        let (mut fx, _, _) = visitors();
        fx.program.expansions = 0xffff;
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        let tpl_file = fx.add_type(template, 2, 4096, 4097);
        fx.add(
            tpl_file,
            4096,
            tree(vec![
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, RETURN_TRUE),
            ]),
        );
        fx.add(
            tpl_file,
            4097,
            tree(vec![
                assign(scope::PARAMETERS, 0, scope::LITERAL, 10, 1),
                ins(0, 0, 0, [0; 8]),
            ]),
        );
        let skel_name = if species == 1 { "dog" } else { "adult" };
        equip(&mut fx, template, skel_name, 12);
        // TemplatePerson.iff / TemplateDog.iff have empty suit rows (no HEAD
        // binding on the template itself).
        fx.program.people.portrait_head.remove(&template);
        fx.program.people.profiles.get_mut(&template).unwrap().tones = vec!["lgt".into()];
        fx.program.people.head_suits = head_suits.iter().map(|s| s.to_string()).collect();
        fx.program.wardrobe = Some(Wardrobe {
            suits: suits.iter().map(|s| s.to_string()).collect(),
            textures: textures.iter().map(|s| s.to_string()).collect(),
            errors: BTreeMap::new(),
        });

        // Execute opcode 19 (Make New Character): skin=0 ("lgt"), age=27, sex=0 (male),
        // storing the resulting neighbor record (stack object) in global 36.
        fx.add(
            LAMP_FILE,
            4098,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
                assign(scope::LOCALS, 1, scope::LITERAL, 27, 2),
                assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
                ins(19, 4, RETURN_FALSE, [0, 1, 2, species, 0, 0, 0, 0]),
                assign(scope::GLOBAL, 36, scope::STACK_OBJECT, 0, RETURN_TRUE),
            ]),
        );
        assert_eq!(fx.run_with(LAMP, 4098, 0, &[], 1000), Ok(true));
        let record = fx.world.globals[36];
        let guid = fx.world.neighbors.get(record).unwrap().guid;
        (fx, record, guid)
    };

    // 1. Human with all matching head suits binding "HEAD": UI Effect renders,
    // caches the portrait, and persists BMP_ 2002 to the made character's file.
    let (mut fx, record, guid) = setup(
        0,
        TEMPLATE_PERSON,
        &["b001mafit_01", "c001ma_bob"],
        &["b001mafitlgt_01.bmp", "c001malgt_bob.bmp"],
        &["c001ma_bob"],
    );
    let made_file = fx.world.characters.made[&guid].file;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [guid].into());
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Made(made_file))
    );

    // 2. Human with NO matching head or body suit binding "HEAD": UI Effect
    // creates and removes the temporary Sim (advancing CRT RNG) and returns
    // true without caching or writing BMP_ 2002.
    let (mut fx, record, guid) = setup(
        0,
        TEMPLATE_PERSON,
        &["b001mafit_01", "c001ma_bob"],
        &["b001mafitlgt_01.bmp", "c001malgt_bob.bmp"],
        &[],
    );
    let crt_before = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_ne!(fx.world.crt_rng, crt_before);
    assert!(!fx.world.portraits.contains(&guid));
    assert!(fx.world.generated_portrait_2002.is_empty());

    // 3. Human with mixed head suits (one binds "HEAD", one does not) and
    // non-HEAD body: UI Effect fails loudly because the pick depends on
    // unmodeled wardrobe order.
    let (mut fx, record, _) = setup(
        0,
        TEMPLATE_PERSON,
        &["b001mafit_01", "c001ma_bob", "c002ma_nohead"],
        &[
            "b001mafitlgt_01.bmp",
            "c001malgt_bob.bmp",
            "c002malgt_nohead.bmp",
        ],
        &["c001ma_bob"],
    );
    let res = run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1);
    assert!(
        matches!(trap(res), Trap::Unsupported(ref m) if m.contains("0x4e7824")),
        "mixed wardrobe should fail loudly in UI Effect"
    );

    // 4. Pet (species 1, TemplateDog) with all matching dog bodies binding
    // "HEAD": opcode 19 calls 0x4e8c5d(index, 0x1f, 1) immediately at 0x44b126,
    // caching the portrait and writing BMP_ 2002 BEFORE returning. A following
    // UI Effect call (and a reload after cache eviction) consumes zero CRT RNG.
    let (mut fx, record, guid) = setup(
        1,
        TEMPLATE_DOG,
        &["b001dog_01"],
        &["b001dog_01.bmp"],
        &["b001dog_01"],
    );
    let made_file = fx.world.characters.made[&guid].file;
    assert_eq!(
        fx.world.portraits,
        [guid].into(),
        "0x44b126 should cache the pet portrait during Make New Character"
    );
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Made(made_file)),
        "0x44b126 should persist BMP_ 2002 during Make New Character"
    );
    let crt_after_make = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(
        fx.world.crt_rng, crt_after_make,
        "UI Effect should hit the portrait cached by 0x44b126 without re-rendering"
    );
    fx.world.portraits.remove(&guid);
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [guid].into());
    assert_eq!(
        fx.world.crt_rng, crt_after_make,
        "evicted pet portrait should reload BMP_ 2002 written at 0x44b126"
    );

    // 4b. Pet with a MIXED custom pet wardrobe (one HEAD dog body, one
    // non-HEAD dog body): Make New Character (0x44b126) still succeeds and
    // returns true (preserving Init/CRT RNG side effects without turning
    // opcode 19 into a VM fault); only a subsequent UI Effect query that would
    // observe whether 0x44b126 cached the portrait fails loudly.
    let (mut fx, record, _) = setup(
        1,
        TEMPLATE_DOG,
        &["b001dog_01", "b002dog_nohead"],
        &["b001dog_01.bmp", "b002dog_nohead.bmp"],
        &["b001dog_01"],
    );
    let res = run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1);
    assert!(
        matches!(trap(res), Trap::Unsupported(ref m) if m.contains("0x4e7824")),
        "mixed pet wardrobe should defer the trap until UI Effect queries the portrait"
    );

    // 4c. Mixed-wardrobe pet whose placement ALWAYS fails (no valid tile):
    // 0x44b126 removes the pet at 0x4e9706 before reaching 0x4e7824, so
    // `ambiguous_portraits` stays clear and a later UI Effect runs its own
    // placement attempt (Init + CRT RNG) and returns true without trapping!
    let (mut fx, _, _) = visitors();
    fx.program.expansions = 0xffff;
    fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
    let tpl_file = fx.add_type(TEMPLATE_DOG, 2, 4096, 4097);
    fx.add(
        tpl_file,
        4096,
        tree(vec![
            ins(2, 1, 1, expr(scope::GLOBAL, 36, op::ADD, scope::LITERAL, 1)),
            assign(scope::MY_DATA, 4, scope::LITERAL, 1, 2),
            assign(scope::MY_DATA, 42, scope::LITERAL, 0, RETURN_TRUE), // unplaceable
        ]),
    );
    fx.add(
        tpl_file,
        4097,
        tree(vec![
            assign(scope::PARAMETERS, 0, scope::LITERAL, 10, 1),
            ins(0, 0, 0, [0; 8]),
        ]),
    );
    equip(&mut fx, TEMPLATE_DOG, "dog", 12);
    fx.program.people.portrait_head.remove(&TEMPLATE_DOG);
    fx.program
        .people
        .profiles
        .get_mut(&TEMPLATE_DOG)
        .unwrap()
        .tones = vec!["lgt".into()];
    fx.program.people.head_suits = ["b001dog_01".to_string()].into();
    fx.program.wardrobe = Some(Wardrobe {
        suits: ["b001dog_01".to_string(), "b002dog_nohead".to_string()].into(),
        textures: [
            "b001dog_01.bmp".to_string(),
            "b002dog_nohead.bmp".to_string(),
        ]
        .into(),
        errors: BTreeMap::new(),
    });
    fx.add(
        LAMP_FILE,
        4098,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
            assign(scope::LOCALS, 1, scope::LITERAL, 27, 2),
            assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
            ins(19, 4, RETURN_FALSE, [0, 1, 2, 1, 0, 0, 0, 0]),
            assign(scope::GLOBAL, 34, scope::STACK_OBJECT, 0, RETURN_TRUE),
        ]),
    );
    assert_eq!(fx.run_with(LAMP, 4098, 0, &[], 1000), Ok(true));
    let unplaceable_pet_rec = fx.world.globals[34];
    let unplaceable_pet_guid = fx.world.neighbors.get(unplaceable_pet_rec).unwrap().guid;
    assert!(!fx.world.ambiguous_portraits.contains(&unplaceable_pet_guid));
    let init_after_make = fx.world.globals[36];
    let crt_after_make = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(
            &mut fx,
            LAMP,
            LAMP_FILE,
            1,
            scope::LITERAL,
            unplaceable_pet_rec,
            1
        ),
        Ok(true)
    );
    assert_eq!(fx.world.globals[36], init_after_make + 2);
    assert_ne!(fx.world.crt_rng, crt_after_make);
    assert!(!fx.world.portraits.contains(&unplaceable_pet_guid));

    // Dialog (0x4363d1 -> 0x4e92a8 -> 0x4e938c -> 0x4e9706) also retries
    // without trapping when placement still fails.
    let init_before_dialog = fx.world.globals[36];
    let crt_before_dialog = fx.world.crt_rng;
    fx.add(
        LAMP_FILE,
        4099,
        tree(vec![ins(
            36,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 0, 0, 0, 0, 0, 5],
        )]),
    );
    assert_eq!(
        fx.run_with(LAMP, 4099, unplaceable_pet_rec, &[], 1000),
        Ok(true)
    );
    assert_eq!(fx.world.globals[36], init_before_dialog + 2);
    assert_ne!(fx.world.crt_rng, crt_before_dialog);
    assert!(!fx.world.portraits.contains(&unplaceable_pet_guid));

    // 5. Invalid pet-picker index (>= bodies) on a template whose ONLY HEAD
    // binding is in row 2 (head) — or whose row 1 has trailing whitespace
    // ("b001dog_01 ,BODY=...", preserved by 0x5874ca and rejected by 0x5a27f2):
    // 0x40d162 leaves body row 1 at the template's value while 0x44b0a3 ->
    // 0x40a5e2 overwrites head row 2 with "NONE", so neither 0x44b126 nor UI
    // Effect caches or persists BMP_ 2002.
    for body_row in [
        "b001dog_nohead,BODY=b001dog_nohead",
        "b001dog_01 ,BODY=b001dog_01",
    ] {
        let (mut fx, _, _) = visitors();
        fx.program.expansions = 0xffff;
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        let tpl_file = fx.add_type(TEMPLATE_DOG, 2, 4096, 4097);
        fx.add(
            tpl_file,
            4096,
            tree(vec![
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, RETURN_TRUE),
            ]),
        );
        fx.add(
            tpl_file,
            4097,
            tree(vec![
                assign(scope::PARAMETERS, 0, scope::LITERAL, 10, 1),
                ins(0, 0, 0, [0; 8]),
            ]),
        );
        equip(&mut fx, TEMPLATE_DOG, "dog", 12);
        {
            let prof = fx.program.people.profiles.get_mut(&TEMPLATE_DOG).unwrap();
            prof.body = Some(body_row.into());
            prof.head = Some("c001ma_bob,HEAD-HEAD=c001malgt_bob".into());
            prof.tones = vec!["lgt".into()];
        }
        fx.program.people.portrait_head.insert(TEMPLATE_DOG, true);
        fx.program.people.head_suits = ["c001ma_bob".to_string(), "b001dog_01".to_string()].into();
        fx.program.wardrobe = Some(Wardrobe {
            suits: ["b001dog_01".to_string()].into(),
            textures: ["b001dog_01.bmp".to_string()].into(),
            errors: BTreeMap::new(),
        });
        fx.world.dialogs.declined = 99; // invalid index >= bodies (1)
        fx.add(
            LAMP_FILE,
            4098,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
                assign(scope::LOCALS, 1, scope::LITERAL, 27, 2),
                assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
                ins(19, 4, RETURN_FALSE, [0, 1, 2, 1, 0, 0, 0, 0]),
                assign(scope::GLOBAL, 36, scope::STACK_OBJECT, 0, RETURN_TRUE),
            ]),
        );
        assert_eq!(fx.run_with(LAMP, 4098, 0, &[], 1000), Ok(true));
        let pet_record = fx.world.globals[36];
        let pet_guid = fx.world.neighbors.get(pet_record).unwrap().guid;
        assert!(
            !fx.world.portraits.contains(&pet_guid),
            "body_row={body_row}"
        );
        assert!(
            fx.world.generated_portrait_2002.is_empty(),
            "body_row={body_row}"
        );
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, pet_record, 1),
            Ok(true)
        );
        assert!(
            !fx.world.portraits.contains(&pet_guid),
            "body_row={body_row}"
        );
        assert!(
            fx.world.generated_portrait_2002.is_empty(),
            "body_row={body_row}"
        );
    }

    // 6. Negative-age copy (0x44b23c): 0x44b2d7 / 0x44b2ef compare the source's
    // body and head rows against the wardrobe via 0x472f92 (using 64-byte fixed
    // buffers capped at 63 text bytes), classifying the 63-byte written suit
    // serialized by 0x4092c5..0x4092e1 -> 0x40d4d7 -> 0x472f41 rather than the
    // raw source suit or an overlong 64-byte registry key.
    let suit_63 = format!("c001fa_{}", "a".repeat(56));
    assert_eq!(suit_63.len(), 63);
    let suit_64 = format!("c001fa_{}", "b".repeat(57));
    assert_eq!(suit_64.len(), 64);
    let long_suit_row = format!("{suit_63}_extra,HEAD-HEAD=c001falgt_alice01");
    let comma_past_255_row = format!("{suit_63}{},HEAD-HEAD=c001falgt_alice01", "x".repeat(194));
    assert_eq!(comma_past_255_row.find(','), Some(257));
    let overlong_64_row = format!("{suit_64},HEAD-HEAD=c001falgt_alice01");
    let long_head_tex = format!("c001falgt_{}_aaa.bmp", "z".repeat(50));
    let long_head_row = format!("c001fa_alice,HEAD-HEAD=c001falgt_{}_bbb", "z".repeat(50));
    for (body_row, head_row, with_wardrobe_err, expect_render) in [
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            "c999fa_masseuse,HEAD-HEAD=c999falgt_masseuse",
            false,
            false,
        ),
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            "c001fa_alice,HEAD-HEAD=C001FAlgt_Alice01",
            false,
            true,
        ),
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            long_head_row.as_str(),
            false,
            true,
        ),
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            long_suit_row.as_str(),
            false,
            true,
        ),
        // Comma past byte 255 (0x587511..0x58752c caps row at 255 bytes):
        // definite miss with or without a wardrobe catalog error.
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            comma_past_255_row.as_str(),
            false,
            false,
        ),
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            comma_past_255_row.as_str(),
            true,
            false,
        ),
        // Valid head descriptor whose texture is missing from wardrobe.textures
        // with an unrelated .cmx error: definite miss at 0x40a502 without trap.
        (
            "NONE",
            "c001fa_alice,HEAD-HEAD=c001falgt_missing",
            true,
            false,
        ),
        // 64-byte registered suit whose 63-byte prefix is NOT registered:
        // 0x4092c5..0x4092e1 writes the 63-byte prefix and 0x5a27f9 misses.
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            overlong_64_row.as_str(),
            false,
            false,
        ),
        // Both rows "NONE" with an unread wardrobe catalog (wardrobe.errors
        // non-empty): definite misses at 0x40d0f9 / 0x40d45d without trapping.
        ("NONE", "NONE", true, false),
    ] {
        let (mut fx, _, _) = visitors();
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        let tpl_file = fx.add_type(TEMPLATE_PERSON, 2, 4096, 4097);
        fx.add(
            tpl_file,
            4096,
            tree(vec![
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, RETURN_TRUE),
            ]),
        );
        fx.add(
            tpl_file,
            4097,
            tree(vec![
                assign(scope::PARAMETERS, 0, scope::LITERAL, 10, 1),
                ins(0, 0, 0, [0; 8]),
            ]),
        );
        equip(&mut fx, TEMPLATE_PERSON, "adult", 12);
        fx.program.people.portrait_head.remove(&TEMPLATE_PERSON);
        fx.program.people.profiles.insert(
            SIM_GUID,
            sims_content::Profile {
                body: Some(body_row.into()),
                head: Some(head_row.into()),
                gender: Some("female".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                ..Default::default()
            },
        );
        fx.program.people.portrait_head.insert(SIM_GUID, true);
        fx.program.people.head_suits = [
            "c999fa_masseuse".to_string(),
            "c001fa_alice".to_string(),
            suit_63.clone(),
            suit_64.clone(),
        ]
        .into();
        let mut errors = BTreeMap::new();
        if with_wardrobe_err {
            errors.insert("bad.cmx".to_string(), "corrupt".to_string());
        }
        fx.program.wardrobe = Some(Wardrobe {
            suits: [
                "b001fafit_01".to_string(),
                "c001fa_alice".to_string(),
                suit_63.clone(),
                suit_64.clone(),
            ]
            .into(),
            textures: [
                "b001fafitlgt_01.bmp".to_string(),
                "c001falgt_alice01.bmp".to_string(),
                long_head_tex.clone(),
            ]
            .into(),
            errors,
        });
        fx.add(
            SIM_FILE,
            4098,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
                assign(scope::LOCALS, 1, scope::LITERAL, -1, 2),
                assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
                ins(19, 4, RETURN_FALSE, [0, 1, 2, 0, 0, 0, 0, 0]),
                assign(scope::GLOBAL, 36, scope::STACK_OBJECT, 0, RETURN_TRUE),
            ]),
        );
        assert_eq!(fx.run_with(SIM, 4098, 0, &[], 1000), Ok(true));
        let copy_record = fx.world.globals[36];
        let copy_guid = fx.world.neighbors.get(copy_record).unwrap().guid;
        let copy_file = fx.world.characters.made[&copy_guid].file;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, copy_record, 1),
            Ok(true),
            "head_row={head_row}"
        );
        assert_eq!(
            fx.world.portraits.contains(&copy_guid),
            expect_render,
            "head_row={head_row}"
        );
        assert_eq!(
            fx.world
                .generated_portrait_2002
                .contains(&PortraitFile::Made(copy_file)),
            expect_render,
            "head_row={head_row}"
        );
    }

    // 7. Early pet expansion guard (0x440d63..0x440dd8) for a writable off-lot
    // catalog pet whose live STR# body row 0 is "kat" and whose .bcf skeleton
    // is unavailable: refused cleanly (factory 0 -> portrait uncached, returns
    // true) without faulting on the missing skeleton.
    let (mut fx, file, record) = visitors();
    fx.program.expansions = 0;
    fx.program.people.skeletons.remove(&WIDGET);
    fx.program.people.skeleton_names.remove(&WIDGET);
    fx.words(file)[19] = 200;
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .strings
        .insert(200, vec!["kat".into()]);
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert!(fx.world.portraits.is_empty());

    // 7b. Deleted STR# 200 with row 0 == "kat" must NOT override the live
    // STR# 200 (`loaded_strings` skips deleted chunks per 0x41c542): a non-pet
    // catalog person is NOT refused at 0x440dd5.
    let (mut fx, file, record) = visitors();
    fx.program.expansions = 0;
    fx.words(file)[19] = 200;
    {
        let lib = fx.program.libraries.get_mut(&file).unwrap();
        lib.strings.insert(200, vec!["kat".into()]);
        lib.loaded_strings.insert(200, Some(vec!["widget".into()]));
    }
    let crt_before = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert_ne!(fx.world.crt_rng, crt_before);

    // 7c. Split US-English runs in STR# 200 (`0x470002`–`0x470049`):
    // `loaded_strings` puts the later run ("kat") first even if physical row 0
    // in `people.skeleton_names` was "widget", so 0x440dd5 still refuses the
    // pet without touching CRT RNG when both pet expansions are absent.
    let (mut fx, file, record) = visitors();
    fx.program.expansions = 0;
    fx.program
        .people
        .skeleton_names
        .insert(WIDGET, "widget".into());
    fx.words(file)[19] = 200;
    {
        let lib = fx.program.libraries.get_mut(&file).unwrap();
        lib.strings.insert(200, vec!["widget".into(), "kat".into()]);
        lib.loaded_strings
            .insert(200, Some(vec!["kat".into(), "widget".into()]));
    }
    let crt_before = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert!(fx.world.portraits.is_empty());
    assert_eq!(fx.world.crt_rng, crt_before);

    // 7d. Reverse split US-English runs (`0x470002`–`0x470049`): physical row 0
    // is "kat" (leaving a stale cached "kat" skeleton on WIDGET), while a later
    // US-English run names the human skeleton "adult" (present in
    // `people.skeletons`). With pet expansions absent, the factory passes
    // 0x440d7d, binds "adult" (0x58833c), advances CRT RNG, and caches WIDGET.
    let (mut fx, file, record) = visitors();
    fx.program.expansions = 0;
    let adult_skel = fx.program.people.skeletons[&WIDGET].clone();
    let mut kat_skel = adult_skel.clone();
    kat_skel.name = "kat".into();
    let mut named_adult = adult_skel;
    named_adult.name = "adult".into();
    fx.program.people.skeletons.insert(WIDGET, kat_skel);
    fx.program.people.skeletons.insert(SIM_GUID, named_adult);
    fx.program
        .people
        .skeleton_names
        .insert(WIDGET, "kat".into());
    fx.words(file)[19] = 200;
    {
        let lib = fx.program.libraries.get_mut(&file).unwrap();
        lib.strings.insert(200, vec!["kat".into(), "adult".into()]);
        lib.loaded_strings
            .insert(200, Some(vec!["adult".into(), "kat".into()]));
    }
    let crt_before = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert_ne!(fx.world.crt_rng, crt_before);

    // 7e. Byte-exact skeleton binding (`0x5a280f`): when distinct "adult" and
    // "Adult" skeletons exist and loaded_strings names "Adult", the factory
    // binds "Adult"'s file (not "adult"'s).
    let (mut fx, file, record) = visitors();
    let base_skel = fx.program.people.skeletons[&WIDGET].clone();
    let mut lower_adult = base_skel.clone();
    lower_adult.name = "adult".into();
    lower_adult.file = "lower-adult.bcf".into();
    let mut upper_adult = base_skel;
    upper_adult.name = "Adult".into();
    upper_adult.file = "upper-adult.bcf".into();
    fx.program.people.skeletons.insert(WIDGET, lower_adult);
    fx.program.people.skeletons.insert(SIM_GUID, upper_adult);
    fx.words(file)[19] = 200;
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .loaded_strings
        .insert(200, Some(vec!["Adult".into()]));
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert!(fx.world.skeleton_files.contains("upper-adult.bcf"));
    assert!(!fx.world.skeleton_files.contains("lower-adult.bcf"));

    // 7e2. Runtime-made character naming "adult" resolves via
    // `people.skeletons_by_name` (`0x5a0384`/`0x5a280f`) even when only
    // "Adult" is referenced by a catalog person in `people.skeletons`.
    let (mut fx, _, _) = visitors();
    fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
    let base_skel = fx.program.people.skeletons[&WIDGET].clone();
    let mut upper_adult = base_skel.clone();
    upper_adult.name = "Adult".into();
    upper_adult.file = "upper-adult.bcf".into();
    let mut lower_adult = base_skel;
    lower_adult.name = "adult".into();
    lower_adult.file = "lower-adult.bcf".into();
    fx.program.people.skeletons.insert(WIDGET, upper_adult);
    fx.program
        .people
        .skeletons_by_name
        .insert("adult".into(), lower_adult);
    let made_guid = 0x5555_9999u32;
    fx.world.characters.made.insert(
        made_guid,
        sims_sim::characters::Character {
            template: WIDGET,
            file: 99,
            strings: [(sims_sim::characters::SKELETON, "adult".to_owned())].into(),
        },
    );
    fx.world.neighbors.slots.push(Some(Neighbor {
        guid: made_guid,
        id: 99,
        data: vec![12; 88],
        relationships: Vec::new(),
        friends: 0,
        revision: 1,
        stale: false,
    }));
    let made_rec = fx.world.neighbors.slots.len() as i16;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, made_rec, 1),
        Ok(true)
    );
    assert!(fx.world.skeleton_files.contains("lower-adult.bcf"));
    assert!(!fx.world.skeleton_files.contains("upper-adult.bcf"));

    // 7f. Negative-age copy prefers loader-ordered `loaded_strings` head row
    // (`0x470049`) over physically ordered `Profile::head`: when the first run
    // had a matching HEAD suit and the later run placed a comma-past-255 row
    // at index 2, the native copy misses and writes "" (no portrait).
    let (mut fx, _, _) = visitors();
    fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
    let tpl_file = fx.add_type(TEMPLATE_PERSON, 2, 4096, 4097);
    fx.add(
        tpl_file,
        4096,
        tree(vec![
            assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
            assign(scope::MY_DATA, 42, scope::LITERAL, 2, RETURN_TRUE),
        ]),
    );
    fx.add(
        tpl_file,
        4097,
        tree(vec![
            assign(scope::PARAMETERS, 0, scope::LITERAL, 10, 1),
            ins(0, 0, 0, [0; 8]),
        ]),
    );
    equip(&mut fx, TEMPLATE_PERSON, "adult", 12);
    fx.program.people.portrait_head.remove(&TEMPLATE_PERSON);
    fx.program.people.profiles.insert(
        SIM_GUID,
        sims_content::Profile {
            body: Some("NONE".into()),
            head: Some("c001fa_alice,HEAD-HEAD=c001falgt_alice01".into()),
            gender: Some("female".into()),
            age: Some("27".into()),
            skin: Some("lgt".into()),
            ..Default::default()
        },
    );
    fx.program.people.portrait_head.insert(SIM_GUID, true);
    fx.program.people.head_suits = ["c001fa_alice".to_string(), suit_63.clone()].into();
    fx.program.wardrobe = Some(Wardrobe {
        suits: ["c001fa_alice".to_string(), suit_63.clone()].into(),
        textures: ["c001falgt_alice01.bmp".to_string()].into(),
        errors: BTreeMap::new(),
    });
    fx.words(SIM_FILE)[19] = 200;
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .loaded_strings
        .insert(
            200,
            Some(vec![
                "adult".into(),
                "NONE".into(),
                comma_past_255_row.clone(),
            ]),
        );
    fx.add(
        SIM_FILE,
        4098,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
            assign(scope::LOCALS, 1, scope::LITERAL, -1, 2),
            assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
            ins(19, 4, RETURN_FALSE, [0, 1, 2, 0, 0, 0, 0, 0]),
            assign(scope::GLOBAL, 36, scope::STACK_OBJECT, 0, RETURN_TRUE),
        ]),
    );
    assert_eq!(fx.run_with(SIM, 4098, 0, &[], 1000), Ok(true));
    let copy_record = fx.world.globals[36];
    let copy_guid = fx.world.neighbors.get(copy_record).unwrap().guid;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, copy_record, 1),
        Ok(true)
    );
    assert!(!fx.world.portraits.contains(&copy_guid));
    assert!(fx.world.generated_portrait_2002.is_empty());
}

#[test]
fn dialog_does_not_reload_shared_writeback_when_winning_definition_is_not_a_person() {
    // 0x43639e -> 0x4575d2: Dialog checks that the winning definition is a
    // person BEFORE calling 0x4e9753 / 0x4e92a8. Even if an earlier person
    // definition put `shared_guid` in 0x6f2698 and another person in the same
    // IFF persisted BMP_ 2002, Dialog skips the reload when the winning
    // definition for `shared_guid` is a non-person.
    let (mut fx, file, record1) = visitors();
    let shared_guid = 0x0000_5556u32;

    // Earlier person definition (puts shared_guid in 0x6f2698 via 0x4e8b3d).
    let mut person_words = *fx.words(file);
    person_words[14] = shared_guid as u16 as i16;
    person_words[15] = (shared_guid >> 16) as u16 as i16;
    // Later non-person definition in the same IFF (overwrites by_guid).
    let mut obj_words = person_words;
    obj_words[9] = 4; // normal object, not person (2)
    {
        let lib = fx.program.libraries.get_mut(&file).unwrap();
        lib.definitions.insert(129, person_words);
        lib.definitions.insert(130, obj_words);
        lib.definition_order.extend([129, 130]);
    }
    fx.catalog.objects.push(CatalogObject {
        guid: shared_guid,
        resource: file,
        chunk: 129,
        kind: 2,
        master: 0,
    });
    let winner_idx = fx.catalog.objects.len();
    fx.catalog.objects.push(CatalogObject {
        guid: shared_guid,
        resource: file,
        chunk: 130,
        kind: 4,
        master: 0,
    });
    fx.catalog.by_guid.insert(shared_guid, winner_idx);
    fx.world.neighbors.slots.push(Some(Neighbor {
        guid: shared_guid,
        id: 60,
        data: vec![0; 88],
        relationships: Vec::new(),
        friends: 0,
        revision: 0,
        stale: false,
    }));
    let shared_record = fx.world.neighbors.slots.len() as i16;

    // UI Effect on WIDGET persists BMP_ 2002 into `file`.
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record1, 1),
        Ok(true)
    );
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Resource(file))
    );

    // Dialog (opcode 36, non-blocking inform, icon mode 2 = neighbor in stack
    // object) on `shared_record` must NOT reload `file`'s BMP_ 2002 because
    // the winning definition for `shared_guid` has type 4 != 2.
    fx.add(
        LAMP_FILE,
        4099,
        tree(vec![ins(
            36,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 0, 0, 0, 0, 0, 5],
        )]),
    );
    assert_eq!(fx.run_with(LAMP, 4099, shared_record, &[], 1000), Ok(true));
    assert!(
        !fx.world.portraits.contains(&shared_guid),
        "Dialog must skip 0x4e92a8 when the winning definition is a non-person (0x43639e)"
    );
}
