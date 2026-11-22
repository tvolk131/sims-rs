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
        .has_portrait_bitmap = true;
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
        .has_portrait_bitmap = true;
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
    fx.program.libraries.get_mut(&file).unwrap().writable = false;
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
    fx.program.libraries.get_mut(&SIM_FILE).unwrap().writable = false;
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
    fx.program.libraries.get_mut(&file).unwrap().writable = false;
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
    // binding): neither on-lot nor off-lot caches the portrait or records
    // generated_portrait_2002, while off-lot creates and removes the temporary
    // Sim, runs 0x588930/0x58d4e1 -> 0x4eb0cc (clearing dirty_portraits even
    // on render failure so a follow-up query runs only 1 factory pass instead
    // of 2), and UI Effect returns true.
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
    assert!(!fx.world.dirty_portraits.contains(&WIDGET));
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
    fx.words(SIM_FILE)[102] = 0; // clear synthetic Fixture dummy flags
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
        .has_portrait_bitmap = true;
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
        .has_portrait_bitmap = true;
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

    // 4. Dirty slot with no tile and NO BMP_ 2002: 0x4e92f2 tries once (where
    // construction reset `0x588930 -> 0x58d4e1 -> 0x4eb0cc` clears dirty even
    // though `0x4e9706` finds no tile), 0x4e9338 misses, and 0x4e938c retries
    // a second time! A follow-up query runs only 1 attempt (+2 Inits) because
    // dirty was cleared.
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
    assert!(!fx.world.dirty_portraits.contains(&WIDGET));
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(
        fx.world.globals[36], 6,
        "follow-up query runs only 0x4e938c (1 attempt = +2 Inits) after 0x4eb0cc cleared dirty"
    );

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
        .has_portrait_bitmap = true;
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
        let lib = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
        lib.writable = true;
        let cb = lib.callbacks.get_mut(&128).unwrap();
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
    // non-HEAD body: Make New Character records the exact picked head/body
    // entry from the RNG draws (0x44b151, 0x44b167), so UI Effect reflects
    // whether THAT chosen suit binds "HEAD" instead of trapping.
    let (mut fx, record, guid) = setup(
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
    let picked_head_is_bob = fx.world.made_outfits[&guid]
        .get(&2)
        .is_some_and(|r| r.starts_with("c001ma_bob,"));
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits.contains(&guid), picked_head_is_bob);
    let made_file = fx.world.characters.made[&guid].file;
    assert_eq!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Made(made_file)),
        picked_head_is_bob
    );

    // 4. Pet (species 1, TemplateDog) with all matching dog bodies binding
    // "HEAD": opcode 19 calls 0x4e8c5d(index, 0x1f, 1) immediately at 0x44b126,
    // writing BMP_ 2002 on bit 0x01 and then clearing the in-memory portrait
    // cache on the later forced dress passes (`0x58d4e1 -> 0x4e897e`) because
    // the pet's head slot is empty ("NONE"). A following UI Effect call reloads
    // BMP_ 2002 with zero CRT RNG and caches the portrait.
    let (mut fx, record, guid) = setup(
        1,
        TEMPLATE_DOG,
        &["b001dog_01"],
        &["b001dog_01.bmp"],
        &["b001dog_01"],
    );
    let made_file = fx.world.characters.made[&guid].file;
    assert!(
        fx.world.portraits.is_empty(),
        "0x44b126's later forced dresses clear in-memory cache when pet head slot is empty"
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
    assert_eq!(fx.world.portraits, [guid].into());
    assert_eq!(
        fx.world.crt_rng, crt_after_make,
        "UI Effect should reload BMP_ 2002 persisted by 0x44b126 without re-rendering"
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

    // 4b. Pet with a MIXED custom pet wardrobe and explicit picker index
    // (`0x44b07e`): index 0 (`b001dog_01`, binds HEAD) caches and persists
    // BMP_ 2002 immediately at `0x44b126`, while index 1 (`b002dog_nohead`,
    // no HEAD) leaves portrait uncached and UI Effect succeeds without trapping.
    for (picker_idx, expect_portrait) in [(0i16, true), (1i16, false)] {
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
        fx.world.dialogs.declined = picker_idx;
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
        let pet_rec = fx.world.globals[34];
        let pet_guid = fx.world.neighbors.get(pet_rec).unwrap().guid;
        let pet_file = fx.world.characters.made[&pet_guid].file;
        // 0x4e8c5d(index, 0x1f, 1) persists BMP_ 2002 on bit 0x01 when `HEAD`
        // binds, then clears the in-memory `portraits` cache on the later
        // forced dress passes because the pet's head slot is empty ("NONE").
        assert!(!fx.world.portraits.contains(&pet_guid));
        assert_eq!(
            fx.world
                .generated_portrait_2002
                .contains(&PortraitFile::Made(pet_file)),
            expect_portrait
        );
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, pet_rec, 1),
            Ok(true)
        );
        assert_eq!(fx.world.portraits.contains(&pet_guid), expect_portrait);
    }

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
    // With `c000fa_nohead` at head index 0 (not in `head_suits`), a head miss
    // falls back to index 0 (`0x40ab7f..0x40acbc`, non-HEAD -> false), while
    // valid HEAD matches keep their matched HEAD suit (true). When `has_body`
    // is false (`body_count == 0`), `0x40ac57` clears BOTH body and head to ""
    // (false) even if `head_row` matched or an unrelated `.cmx` failed.
    for (body_row, head_row, has_body, with_wardrobe_err, expect_render) in [
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            "c999fa_masseuse,HEAD-HEAD=c999falgt_masseuse",
            true,
            false,
            false,
        ),
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            "c001fa_alice,HEAD-HEAD=C001FAlgt_Alice01",
            true,
            false,
            true,
        ),
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            long_head_row.as_str(),
            true,
            false,
            true,
        ),
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            long_suit_row.as_str(),
            true,
            false,
            true,
        ),
        // Comma past byte 255 (0x587511..0x58752c caps row at 255 bytes):
        // misses -> falls back to non-HEAD index 0 (`c000fa_nohead`).
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            comma_past_255_row.as_str(),
            true,
            false,
            false,
        ),
        // Comma past byte 255 with empty body textures + `.cmx` error:
        // definite miss + `body_count == 0` -> `0x40ac57` clears both to "".
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            comma_past_255_row.as_str(),
            false,
            true,
            false,
        ),
        // Valid head descriptor whose texture is missing from wardrobe.textures
        // with empty body textures + `.cmx` error: definite miss -> `0x40ac57`.
        (
            "NONE",
            "c001fa_alice,HEAD-HEAD=c001falgt_missing",
            false,
            true,
            false,
        ),
        // 64-byte registered suit whose 63-byte prefix is NOT registered:
        // 0x4092c5..0x4092e1 writes the 63-byte prefix and 0x5a27f9 misses.
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            overlong_64_row.as_str(),
            true,
            false,
            false,
        ),
        // Both rows "NONE" with empty body textures + `.cmx` error -> `0x40ac57`.
        ("NONE", "NONE", false, true, false),
        // Matched head row (`c001fa_alice`) + missed body row when
        // `body_count == 0`: `0x40ac57` clears BOTH body and head to "".
        (
            "b999fafit_npc,BODY=b999fafitlgt_npc",
            "c001fa_alice,HEAD-HEAD=C001FAlgt_Alice01",
            false,
            false,
            false,
        ),
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
        let mut textures: std::collections::BTreeSet<String> = [
            "c000falgt_nohead.bmp".to_string(),
            "c001falgt_alice01.bmp".to_string(),
            long_head_tex.clone(),
        ]
        .into();
        if has_body {
            textures.insert("b001fafitlgt_01.bmp".to_string());
        }
        fx.program.wardrobe = Some(Wardrobe {
            suits: [
                "b001fafit_01".to_string(),
                "c000fa_nohead".to_string(),
                "c001fa_alice".to_string(),
                suit_63.clone(),
                suit_64.clone(),
            ]
            .into(),
            textures,
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

    // 7e3. Runtime-made character with a valid skeleton override succeeds in
    // BOTH `Interpreter::create_person` and UI Effect even when its template's
    // own rig is missing from `people.skeletons` (template profile retained).
    let (mut fx, _, _) = visitors();
    fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
    let adult_skel = fx.program.people.skeletons.remove(&WIDGET).unwrap();
    let mut named_adult = adult_skel;
    named_adult.name = "adult".into();
    named_adult.file = "override-adult.bcf".into();
    fx.program
        .people
        .skeletons_by_name
        .insert("adult".into(), named_adult);
    let made_guid = 0x5555_999au32;
    fx.world.characters.made.insert(
        made_guid,
        sims_sim::characters::Character {
            template: WIDGET,
            file: 100,
            strings: [(sims_sim::characters::SKELETON, "adult".to_owned())].into(),
        },
    );
    fx.world.neighbors.slots.push(Some(Neighbor {
        guid: made_guid,
        id: 100,
        data: vec![12; 88],
        relationships: Vec::new(),
        friends: 0,
        revision: 1,
        stale: false,
    }));
    let made_rec = fx.world.neighbors.slots.len() as i16;
    let id = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream)
        .create_person(&mut fx.world, made_guid)
        .expect("public create_person should resolve runtime-made override even when template rig is absent");
    assert!(fx.world.objects.contains_key(&id));
    assert!(fx.world.skeleton_files.contains("override-adult.bcf"));
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, made_rec, 1),
        Ok(true)
    );
    assert_eq!(fx.world.portraits, [made_guid].into());

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

#[test]
fn dialog_reloads_embedded_bmp_2002_and_clones_inherit_session_writeback() {
    // 1. Clean catalog slot with embedded BMP_ 2002 (library.has_portrait_2002):
    // Dialog (0x4363d1 -> 0x4e92a8 -> 0x4e9338) caches the portrait directly
    // without calling the factory, reading a skeleton, or advancing CRT RNG.
    let (mut fx, file, record) = visitors();
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .has_portrait_bitmap = true;
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
    let before = fx.world.crt_rng;
    assert_eq!(fx.run_with(LAMP, 4096, record, &[], 1000), Ok(true));
    assert!(fx.world.portraits.contains(&WIDGET));
    assert_eq!(fx.world.crt_rng, before);
    assert!(fx.world.skeleton_files.is_empty());

    // 2. Writable template acquires BMP_ 2002 via UI Effect earlier in the
    // session; cloning it with Make New Character (0x42629e) copies that
    // resource into User%05d.iff, so UI Effect on the clone hits 0x4e9338
    // without creating a Sim or advancing CRT RNG.
    const TEMPLATE_PERSON: u32 = 0x7fd9_6b54;
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
    let tpl_rec = fx.world.neighbors.slots.len() as i16;
    fx.program
        .people
        .profiles
        .get_mut(&TEMPLATE_PERSON)
        .unwrap()
        .tones = vec!["lgt".into()];
    fx.program.wardrobe = Some(Wardrobe {
        suits: ["b001mafit_01".to_string(), "c001ma_bob".to_string()].into(),
        textures: [
            "b001mafitlgt_01.bmp".to_string(),
            "c001malgt_bob.bmp".to_string(),
        ]
        .into(),
        errors: BTreeMap::new(),
    });
    // First persist BMP_ 2002 on the writable template via UI Effect.
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, tpl_rec, 1),
        Ok(true)
    );
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Resource(tpl_file))
    );
    // Now clone the template via Make New Character.
    fx.add(
        LAMP_FILE,
        4098,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
            assign(scope::LOCALS, 1, scope::LITERAL, 27, 2),
            assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
            ins(19, 4, RETURN_FALSE, [0, 1, 2, 0, 0, 0, 0, 0]),
            assign(scope::GLOBAL, 34, scope::STACK_OBJECT, 0, RETURN_TRUE),
        ]),
    );
    assert_eq!(fx.run_with(LAMP, 4098, 0, &[], 1000), Ok(true));
    let clone_rec = fx.world.globals[34];
    let clone_guid = fx.world.neighbors.get(clone_rec).unwrap().guid;
    let clone_file = fx.world.characters.made[&clone_guid].file;
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Made(clone_file))
    );
    let crt_before_clone_ui = fx.world.crt_rng;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, clone_rec, 1),
        Ok(true)
    );
    assert!(fx.world.portraits.contains(&clone_guid));
    assert_eq!(
        fx.world.crt_rng, crt_before_clone_ui,
        "clone should load inherited BMP_ 2002 at 0x4e9338 without rendering"
    );
}

#[test]
fn live_appearance_and_runtime_indexed_skeleton_load_order() {
    // 1. Runtime-made override whose template had portrait_head removed still
    // resolves HEAD from the template's retained Profile::head + head_suits.
    let (mut fx, _, record) = visitors();
    let mut rig = fx.program.people.skeletons.remove(&WIDGET).unwrap();
    rig.name = "adult".to_owned();
    rig.file = "override.bcf".to_owned();
    fx.program
        .people
        .skeletons_by_name
        .insert("adult".to_owned(), rig);
    fx.program.people.portrait_head.remove(&WIDGET);
    fx.program.people.profiles.get_mut(&WIDGET).unwrap().head =
        Some("c001_head,HEAD-HEAD=texture".into());
    fx.program.people.head_suits.insert("c001_head".to_owned());
    let guid = 0x5555_999au32;
    fx.world.characters.made.insert(
        guid,
        sims_sim::characters::Character {
            template: WIDGET,
            file: 100,
            strings: [(sims_sim::characters::SKELETON, "adult".to_owned())].into(),
        },
    );
    fx.world.neighbors.get_mut(record).unwrap().guid = guid;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
        Ok(true)
    );
    assert!(fx.world.portraits.contains(&guid));
    assert!(
        fx.world
            .generated_portrait_2002
            .contains(&PortraitFile::Made(100))
    );

    // 2. Live on-lot appearance (`0x4e7801` -> `0x58a178` -> `0x4e810d`):
    // - A HEAD accessory makes a non-HEAD Sim renderable.
    // - Changing outfit to swim (suit 2, row 31 non-HEAD) removes an everyday
    //   body-only HEAD binding; changing to formal (suit 4, row 30 HEAD) adds one.
    // - A skeleton with non-empty bones lacking "HEAD" cannot render (`0x595699`).
    let (mut fx, _, _) = visitors();
    fx.words(SIM_FILE)[102] = 0; // clear synthetic Fixture dummy flags
    fx.world.neighbors.slots.push(Some(Neighbor {
        guid: SIM_GUID,
        id: 2,
        data: vec![0; 88],
        relationships: Vec::new(),
        friends: 0,
        revision: 0,
        stale: false,
    }));
    let sim_rec = fx.world.neighbors.slots.len() as i16;
    fx.program.people.portrait_head.insert(SIM_GUID, false);
    fx.program.people.head_suits.insert("acc_hat".into());
    fx.program
        .people
        .head_suits
        .insert("b_everyday_head".into());
    fx.program.people.suits.insert("s_swim_nohead".into());
    fx.program.people.head_suits.insert("f_formal_head".into());
    // Put on HEAD accessory -> renders!
    fx.world
        .objects
        .get_mut(&SIM)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .accessories
        .push(sims_formats::saved_objects::Accessory {
            name: "acc_hat".into(),
            binding: String::new(),
        });
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_rec, 1),
        Ok(true)
    );
    assert!(fx.world.portraits.contains(&SIM_GUID));

    // Take off accessory, configure everyday body=HEAD, head=NONE, swim=nohead, formal=HEAD.
    fx.world.portraits.remove(&SIM_GUID);
    fx.world.generated_portrait_2002.clear();
    fx.world
        .objects
        .get_mut(&SIM)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .accessories
        .clear();
    fx.program.people.profiles.insert(
        SIM_GUID,
        sims_content::Profile {
            body: Some("b_everyday_head,BODY=x".into()),
            head: Some("NONE".into()),
            suit_rows: BTreeMap::from([
                (31, "s_swim_nohead,BODY=x".into()),
                (30, "f_formal_head,BODY=x".into()),
            ]),
            ..Default::default()
        },
    );
    // Swim suit (data[8] = 2) replaces body with s_swim_nohead -> does NOT render!
    {
        let p = fx
            .world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        p.data[65] = 0; // adult human (clear synthetic Fixture dummy gender bits)
        p.data[58] = 27;
        p.data[8] = 2;
    }
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_rec, 1),
        Ok(true)
    );
    assert!(!fx.world.portraits.contains(&SIM_GUID));

    // Formal suit (data[8] = 4) wears f_formal_head -> DOES render!
    fx.world
        .objects
        .get_mut(&SIM)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[8] = 4;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_rec, 1),
        Ok(true)
    );
    assert!(fx.world.portraits.contains(&SIM_GUID));

    // Party costume (data[8] = 10, luau via `0x45f452`) dresses via the
    // generated costume table without trapping.
    fx.world.portraits.remove(&SIM_GUID);
    fx.world.generated_portrait_2002.clear();
    fx.world
        .objects
        .get_mut(&SIM)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[8] = 10;
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_rec, 1),
        Ok(true)
    );
    assert!(
        fx.world.portraits.contains(&SIM_GUID),
        "luau costume falls back to everyday_head_body when no ct-luau suit is registered"
    );

    // Skeleton lacking a "HEAD" bone cannot render (`0x595699`).
    fx.world.portraits.remove(&SIM_GUID);
    fx.world.generated_portrait_2002.clear();
    fx.world
        .objects
        .get_mut(&SIM)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[8] = 0;
    fx.program.people.skeletons.insert(
        SIM_GUID,
        sims_content::Skeleton {
            name: "nohead".into(),
            file: "nohead.bcf".into(),
            place: 0,
            scales: vec![vec![1]],
            bones: vec![sims_content::Bone {
                name: "ROOT".into(),
                parent: None,
                translation: [0; 3],
                rotation: [0, 0, 0, 1.0f32.to_bits()],
                blends: false,
            }],
        },
    );
    assert_eq!(
        run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_rec, 1),
        Ok(true)
    );
    assert!(!fx.world.portraits.contains(&SIM_GUID));

    // 3. Runtime-ordered lazy `.ndx` skeleton loading (`0x5a0384`, `0x59f19f`):
    // alpha.bcf defines [alpha(scale 2), beta(scale 3)], beta.bcf defines [beta(scale 4)].
    // Creating alpha before beta fills beta from alpha.bcf (different file + CRT draws);
    // creating beta first loads beta.bcf.
    let setup_indexed = || {
        let (mut fx, _, _) = visitors();
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        fx.program.people.skeletons.remove(&WIDGET);
        let mk_skel =
            |name: &str, file: &str, place: usize, scales: Vec<Vec<i32>>| sims_content::Skeleton {
                name: name.into(),
                file: file.into(),
                place,
                scales,
                bones: Vec::new(),
            };
        fx.program
            .people
            .indexed_skeletons
            .insert("alpha".into(), Ok("alpha.bcf".into()));
        fx.program
            .people
            .indexed_skeletons
            .insert("beta".into(), Ok("beta.bcf".into()));
        fx.program.people.indexed_catalogs.insert(
            "alpha.bcf".into(),
            (
                vec![
                    mk_skel("alpha", "alpha.bcf", 0, vec![vec![2], vec![3]]),
                    mk_skel("beta", "alpha.bcf", 1, vec![vec![2], vec![3]]),
                ],
                None,
            ),
        );
        fx.program.people.indexed_catalogs.insert(
            "beta.bcf".into(),
            (vec![mk_skel("beta", "beta.bcf", 0, vec![vec![4]])], None),
        );
        for (guid, file, skel_name) in [
            (0x5555_a001u32, 101, "alpha"),
            (0x5555_a002u32, 102, "beta"),
        ] {
            fx.world.characters.made.insert(
                guid,
                sims_sim::characters::Character {
                    template: WIDGET,
                    file,
                    strings: [(sims_sim::characters::SKELETON, skel_name.to_owned())].into(),
                },
            );
            fx.world.neighbors.slots.push(Some(Neighbor {
                guid,
                id: file as i16,
                data: vec![12; 88],
                relationships: Vec::new(),
                friends: 0,
                revision: 1,
                stale: false,
            }));
        }
        fx
    };
    // Order A: alpha then beta -> beta uses alpha.bcf (never reads beta.bcf!).
    let mut fx_a = setup_indexed();
    let interp_a = sims_vm::Interpreter::new(&fx_a.program, &fx_a.catalog, fx_a.stream);
    interp_a
        .create_person(&mut fx_a.world, 0x5555_a001)
        .unwrap();
    interp_a
        .create_person(&mut fx_a.world, 0x5555_a002)
        .unwrap();
    assert!(fx_a.world.skeleton_files.contains("alpha.bcf"));
    assert!(!fx_a.world.skeleton_files.contains("beta.bcf"));
    assert_eq!(
        fx_a.world.loaded_indexed_skeletons.get("beta"),
        Some(&Some("alpha.bcf".to_owned()))
    );
    // Order B: beta then alpha -> beta uses beta.bcf!
    let mut fx_b = setup_indexed();
    let interp_b = sims_vm::Interpreter::new(&fx_b.program, &fx_b.catalog, fx_b.stream);
    interp_b
        .create_person(&mut fx_b.world, 0x5555_a002)
        .unwrap();
    interp_b
        .create_person(&mut fx_b.world, 0x5555_a001)
        .unwrap();
    assert!(fx_b.world.skeleton_files.contains("beta.bcf"));
    assert!(fx_b.world.skeleton_files.contains("alpha.bcf"));
    assert_eq!(
        fx_b.world.loaded_indexed_skeletons.get("beta"),
        Some(&Some("beta.bcf".to_owned()))
    );
}

#[test]
fn person_data_invalidation_catalog_dialog_indexed_suits_live_outfits_and_ordered_wardrobe() {
    use sims_content::{Profile, Skeleton};
    const MY_PERSON_DATA: u8 = 18;
    const TEMPLATE_PERSON: u32 = 0x7fd9_6b54;

    // 1. P1: Writing My Person Data words 68, 74, or 84 (0x4538fd..0x4539c7 ->
    // 0x45810c -> 0x4eb0eb) clears the running object's cached portrait and
    // marks it dirty so Dialog/UI Effect rebuilds BEFORE checking BMP_ 2002.
    for word_idx in [68i16, 74, 84] {
        let (mut fx, _, _) = visitors();
        fx.world.portraits.insert(SIM_GUID);
        fx.add(
            SIM_FILE,
            4098,
            tree(vec![assign(
                MY_PERSON_DATA,
                word_idx,
                scope::LITERAL,
                1,
                RETURN_TRUE,
            )]),
        );
        assert_eq!(fx.run_with(SIM, 4098, 0, &[], 100), Ok(true));
        assert!(!fx.world.portraits.contains(&SIM_GUID));
        assert!(fx.world.dirty_portraits.contains(&SIM_GUID));
    }

    // 2. P2 #1: Catalog types whose skeletons are `.ndx`-indexed honor runtime
    // call order (even when `people.skeletons` holds per-GUID snapshots), and a
    // failed indexed skeleton lookup records `None` (`0x5a389b`) so a later
    // sibling catalog load cannot overwrite it.
    let mk_skel = |name: &str, file: &str, place: usize, scales: Vec<Vec<i32>>| Skeleton {
        name: name.to_owned(),
        file: file.to_owned(),
        place,
        scales,
        bones: Vec::new(),
    };
    let setup_catalog_indexed = || {
        let (mut fx, _, _) = visitors();
        for (guid, skel_name, rec_id) in [
            (0x7777_0001u32, "alpha", 41i16),
            (0x7777_0002u32, "beta", 42i16),
            (0x7777_0003u32, "missing_first", 43i16),
        ] {
            let f = fx.add_type(guid, 2, 4096, 4097);
            fx.add(
                f,
                4096,
                tree(vec![assign(
                    scope::MY_DATA,
                    4,
                    scope::LITERAL,
                    1,
                    RETURN_TRUE,
                )]),
            );
            fx.add(
                f,
                4097,
                tree(vec![
                    assign(scope::PARAMETERS, 0, scope::LITERAL, 10, 1),
                    ins(0, 0, 0, [0; 8]),
                ]),
            );
            fx.program
                .people
                .skeleton_names
                .insert(guid, skel_name.to_owned());
            fx.program.people.profiles.insert(guid, Profile::default());
            fx.world.neighbors.slots.push(Some(Neighbor {
                guid,
                id: rec_id,
                data: vec![12; 88],
                relationships: Vec::new(),
                friends: 0,
                revision: 1,
                stale: false,
            }));
        }
        // Populate per-GUID `people.skeletons` as `People::load_each` does,
        // while keeping `.ndx` entries unloaded in `indexed_skeletons`.
        fx.program.people.skeletons.insert(
            0x7777_0001,
            mk_skel("alpha", "alpha.bcf", 0, vec![vec![2], vec![3]]),
        );
        fx.program
            .people
            .skeletons
            .insert(0x7777_0002, mk_skel("beta", "beta.bcf", 0, vec![vec![4]]));
        fx.program
            .people
            .indexed_skeletons
            .insert("alpha".into(), Ok("alpha.bcf".into()));
        fx.program
            .people
            .indexed_skeletons
            .insert("beta".into(), Ok("beta.bcf".into()));
        fx.program.people.indexed_skeletons.insert(
            "missing_first".into(),
            Err("0x5a3810: missing_first.bcf is missing".into()),
        );
        fx.program.people.indexed_catalogs.insert(
            "alpha.bcf".into(),
            (
                vec![
                    mk_skel("alpha", "alpha.bcf", 0, vec![vec![2], vec![3], vec![5]]),
                    mk_skel("beta", "alpha.bcf", 1, vec![vec![2], vec![3], vec![5]]),
                    mk_skel(
                        "missing_first",
                        "alpha.bcf",
                        2,
                        vec![vec![2], vec![3], vec![5]],
                    ),
                ],
                None,
            ),
        );
        fx.program.people.indexed_catalogs.insert(
            "beta.bcf".into(),
            (vec![mk_skel("beta", "beta.bcf", 0, vec![vec![4]])], None),
        );
        fx
    };
    let mut fx_cat = setup_catalog_indexed();
    let interp = sims_vm::Interpreter::new(&fx_cat.program, &fx_cat.catalog, fx_cat.stream);
    // Querying `missing_first` fails and records `None` (`0x5a389b`).
    assert!(
        interp
            .create_person(&mut fx_cat.world, 0x7777_0003)
            .is_err()
    );
    assert_eq!(
        fx_cat.world.loaded_indexed_skeletons.get("missing_first"),
        Some(&None)
    );
    // Creating `alpha` fills `beta` from `alpha.bcf` (NOT `beta.bcf`!) and
    // leaves `missing_first` as `None`.
    interp
        .create_person(&mut fx_cat.world, 0x7777_0001)
        .unwrap();
    interp
        .create_person(&mut fx_cat.world, 0x7777_0002)
        .unwrap();
    assert!(!fx_cat.world.skeleton_files.contains("beta.bcf"));
    assert_eq!(
        fx_cat.world.loaded_indexed_skeletons.get("beta"),
        Some(&Some("alpha.bcf".to_owned()))
    );
    assert_eq!(
        fx_cat.world.loaded_indexed_skeletons.get("missing_first"),
        Some(&None)
    );
    assert!(
        interp
            .create_person(&mut fx_cat.world, 0x7777_0003)
            .is_err()
    );

    // 3. P2 #2: Catalog Dialog without BMP_ 2002 skips read-only off-lot Sim,
    // does NOT cache failed HEAD render, writes BMP_ 2002 on clean render, and
    // honors save flag 0 when dirty.
    {
        let (mut fx, file, record) = visitors();
        fx.program.libraries.get_mut(&file).unwrap().writable = false;
        let before_rng = fx.world.crt_rng.state();
        fx.add(
            LAMP_FILE,
            4098,
            tree(vec![
                assign(scope::STACK_OBJECT, 0, scope::LITERAL, record, 1),
                ins(36, RETURN_TRUE, RETURN_FALSE, [0, 0, 0, 0, 0, 0, 0, 5]),
            ]),
        );
        assert_eq!(fx.run_with(LAMP, 4098, 0, &[], 100), Ok(true));
        assert_eq!(fx.world.crt_rng.state(), before_rng);
        assert!(!fx.world.portraits.contains(&WIDGET));
    }
    {
        let (mut fx, file, record) = visitors();
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        fx.program.people.portrait_head.insert(WIDGET, false);
        fx.add(
            LAMP_FILE,
            4098,
            tree(vec![
                assign(scope::STACK_OBJECT, 0, scope::LITERAL, record, 1),
                ins(36, RETURN_TRUE, RETURN_FALSE, [0, 0, 0, 0, 0, 0, 0, 5]),
            ]),
        );
        assert_eq!(fx.run_with(LAMP, 4098, 0, &[], 100), Ok(true));
        assert!(
            !fx.world.portraits.contains(&WIDGET),
            "catalog Dialog must not cache portrait when HEAD binding is absent"
        );
        assert!(
            !fx.world
                .generated_portrait_2002
                .contains(&PortraitFile::Resource(file))
        );
        // With HEAD binding and dirty flag set: 0x4e92f2 caches portrait with
        // save flag 0 (no BMP_ 2002 writeback).
        fx.program.people.portrait_head.insert(WIDGET, true);
        fx.world.dirty_portraits.insert(WIDGET);
        assert_eq!(fx.run_with(LAMP, 4098, 0, &[], 100), Ok(true));
        assert!(fx.world.portraits.contains(&WIDGET));
        assert!(!fx.world.dirty_portraits.contains(&WIDGET));
        assert!(
            !fx.world
                .generated_portrait_2002
                .contains(&PortraitFile::Resource(file)),
            "dirty catalog Dialog rebuild uses save flag 0 (0x4e92f2)"
        );
        // Clean uncached catalog Dialog writes BMP_ 2002 via 0x4e938c.
        fx.world.portraits.remove(&WIDGET);
        assert_eq!(fx.run_with(LAMP, 4098, 0, &[], 100), Ok(true));
        assert!(fx.world.portraits.contains(&WIDGET));
        assert!(
            fx.world
                .generated_portrait_2002
                .contains(&PortraitFile::Resource(file)),
            "clean catalog Dialog fallback must persist BMP_ 2002 (0x4e938c)"
        );
    }

    // 4. P2 #4: Indexed `.ndx` appearance catalog sibling filling (`0x5a044d`
    // -> `0x5a38da` -> `0x59f35f`): loading `a_suit` first fills `b_suit` from
    // `a.bcf` (with HEAD = true), whereas loading `b_suit` first loads `b.bcf`
    // (with HEAD = false).
    let setup_indexed_suits = || {
        let (mut fx, _, record) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        fx.program.people.portrait_head.remove(&WIDGET);
        fx.program.people.portrait_head.remove(&SIM_GUID);
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("a_suit,BODY=a".into()),
                head: None,
                ..Default::default()
            },
        );
        fx.program.people.profiles.insert(
            WIDGET,
            Profile {
                body: Some("b_suit,BODY=b".into()),
                head: None,
                ..Default::default()
            },
        );
        fx.program
            .people
            .indexed_suits
            .insert("a_suit".into(), Ok("a.bcf".into()));
        fx.program
            .people
            .indexed_suits
            .insert("b_suit".into(), Ok("b.bcf".into()));
        fx.program.people.indexed_suit_catalogs.insert(
            "a.bcf".into(),
            (vec![("a_suit".into(), true), ("b_suit".into(), true)], None),
        );
        fx.program
            .people
            .indexed_suit_catalogs
            .insert("b.bcf".into(), (vec![("b_suit".into(), false)], None));
        (fx, sim_record, record)
    };
    {
        let (mut fx, sim_record, record) = setup_indexed_suits();
        // Render SIM (a_suit) first -> fills b_suit = true from a.bcf!
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(fx.world.portraits.contains(&SIM_GUID));
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
            Ok(true)
        );
        assert!(
            fx.world.portraits.contains(&WIDGET),
            "a.bcf must fill sibling b_suit with HEAD=true when a_suit loads first"
        );
    }
    {
        let (mut fx, _, record) = setup_indexed_suits();
        // Render WIDGET (b_suit) first -> loads b.bcf (HEAD=false)!
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
            Ok(true)
        );
        assert!(
            !fx.world.portraits.contains(&WIDGET),
            "b.bcf must supply HEAD=false when b_suit loads before a_suit"
        );
    }

    // 5. P2 #5: Live outfit resolution — formalwear checks STR# 200 row 13 age
    // (`0x58a6ff`, not `person.data[58]`), pets mutate `person.data[8] = 0`
    // (`0x58a2e5`), and out-of-range suits (-1, 61) dress everyday outfit 0.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.program
            .people
            .head_suits
            .insert("adult_formal_head".into());
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("plain_body,BODY=b".into()),
                head: None,
                // Body string age = 27 (adult), while live `person.data[58]` = 9!
                age: Some("27".into()),
                suit_rows: BTreeMap::from([
                    (27, "child_formal_nohead,BODY=c".into()),
                    (30, "adult_formal_head,BODY=a".into()),
                ]),
                ..Default::default()
            },
        );
        let person = fx
            .world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        person.data[65] = 0; // adult human
        person.data[58] = 9;
        person.data[8] = 4; // formalwear
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            fx.world.portraits.contains(&SIM_GUID),
            "formalwear must check STR# 200 row 13 age (27 -> row 30), not person.data[58]"
        );

        // Out-of-range suits (-1 and 61) fall back to everyday outfit 0 without trapping.
        fx.program.people.head_suits.insert("plain_body".into());
        for bad_suit in [-1i16, 61] {
            fx.world.portraits.remove(&SIM_GUID);
            fx.world.generated_portrait_2002.clear();
            fx.world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
                .data[8] = bad_suit;
            assert_eq!(
                run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
                Ok(true)
            );
            assert!(fx.world.portraits.contains(&SIM_GUID));
        }

        // Pet (gender word & 6 != 0, def_flags & 8 == 0) resets live `person.data[8] = 0`.
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        let person = fx
            .world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        person.data[65] = 2; // dogmale
        person.data[8] = 4;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.objects[&SIM].person.as_ref().unwrap().data[8],
            0,
            "0x58a2e5 must mutate live pet person.data[8] to 0"
        );
    }

    // 6. P2 #6 + P2 #7 + P3: `program.ordered_wardrobe` (`Some(Ok(...))` uses
    // exact native order, ignores legacy `wardrobe.errors`, caps complete
    // `BODY=<tex>` / `HEAD-HEAD=<tex>` at 63 bytes, and runs negative-age
    // index-0 fallback; `Some(Err(...))` fails loudly instead of falling back).
    {
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
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("b999fafit_miss,BODY=b999fafitlgt_miss".into()),
                head: Some("c999fa_miss,HEAD-HEAD=c999falgt_miss".into()),
                gender: Some("female".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                ..Default::default()
            },
        );
        // `Some(Err(...))` must fail loudly even when legacy `wardrobe` is valid.
        fx.program.wardrobe = Some(Wardrobe {
            suits: ["b001fafit_01".into(), "c001fa_01".into()].into(),
            textures: ["b001fafitlgt_01.bmp".into(), "c001falgt_01.bmp".into()].into(),
            errors: BTreeMap::new(),
        });
        fx.program.ordered_wardrobe = Some(Err("simulated ordered wardrobe failure".into()));
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
        assert!(fx.run_with(SIM, 4098, 0, &[], 1000).is_err());

        // `Some(Ok(ordered))` succeeds even when legacy `wardrobe.errors` is
        // non-empty, caps the complete descriptor at 63 chars, and falls back
        // to index 0 (`c002fa_first`) on negative-age miss!
        let long_stem = format!("b001fafitlgt_{}", "q".repeat(60));
        let ordered = sims_content::ImportWardrobe::from_ordered_names(
            vec![
                b"b001fafit_01".to_vec(),
                b"c002fa_first".to_vec(),
                b"c001fa_second".to_vec(),
            ],
            vec![
                long_stem.into_bytes(),
                b"c002falgt_first".to_vec(),
                b"c001falgt_second".to_vec(),
            ],
        );
        let (_, picked_body_row) = ordered.pick(false, true, false, "lgt", 0).unwrap();
        let (_, picked_desc) = picked_body_row.split_once(',').unwrap();
        assert_eq!(picked_desc.chars().count(), 63);
        assert!(picked_desc.starts_with("BODY="));

        fx.program
            .wardrobe
            .as_mut()
            .unwrap()
            .errors
            .insert("legacy.cmx".into(), "ignored".into());
        fx.program.ordered_wardrobe = Some(Ok(ordered));
        fx.program.people.head_suits.insert("c002fa_first".into());
        assert_eq!(fx.run_with(SIM, 4098, 0, &[], 1000), Ok(true));
        let copy_record = fx.world.globals[36];
        let copy_guid = fx.world.neighbors.get(copy_record).unwrap().guid;
        let copy_rows = &fx.world.made_outfits[&copy_guid];
        assert_eq!(copy_rows.get(&1), Some(&picked_body_row));
        assert_eq!(
            copy_rows.get(&2).map(String::as_str),
            Some("c002fa_first,HEAD-HEAD=c002falgt_first")
        );
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, copy_record, 1),
            Ok(true)
        );
        assert!(
            fx.world.portraits.contains(&copy_guid),
            "negative-age miss must fall back to index 0 (`c002fa_first`) when both lists are non-empty"
        );
    }
}

#[test]
fn round_16_unified_bcf_lazy_loads_dress_time_accessories_pet_reset_formal_extras_and_dialog_refusal()
 {
    use sims_content::{Bone, Profile, Skeleton};
    const MY_PERSON_DATA: u8 = 18;
    const STACK_OBJECT_PERSON_DATA: u8 = 19;
    const TEMPLATE_PERSON: u32 = 0x7fd9_6b54;
    const TEMPLATE_DOG: u32 = 0x4a70_df92;
    let mk_skel =
        |name: &str, file: &str, place: usize, scales: Vec<Vec<i32>>, bones: Vec<Bone>| Skeleton {
            name: name.to_owned(),
            file: file.to_owned(),
            place,
            scales,
            bones,
        };

    // 1. P2 #1 & #2 & #3: Unified `.bcf` lazy load populates BOTH skeletons
    // AND appearances; failed decode marks the appearance attempted (`0x5a3965`)
    // so later catalogs cannot overwrite it; Change Suit (opcode 6) resolves
    // `.ndx` accessories at dress time (`0x58d903`); and a HEAD-binding head
    // row does NOT skip resolving the body suit.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[65] = 0;

        // Mixed catalog `mixed.bcf` defines skeleton `shared_rig` (scale 7) AND
        // appearance `shared_acc` (HEAD=true) AND `sib_body` (HEAD=true).
        // `broken.bcf` stops decoding with an error before defining `broken_suit`.
        fx.program
            .people
            .indexed_skeletons
            .insert("shared_rig".into(), Ok("other_rig.bcf".into()));
        fx.program
            .people
            .indexed_suits
            .insert("shared_acc".into(), Ok("mixed.bcf".into()));
        fx.program
            .people
            .indexed_suits
            .insert("sib_body".into(), Ok("other_body.bcf".into()));
        fx.program
            .people
            .indexed_suits
            .insert("broken_suit".into(), Ok("broken.bcf".into()));
        fx.program.people.indexed_catalogs.insert(
            "mixed.bcf".into(),
            (
                vec![mk_skel("shared_rig", "mixed.bcf", 0, vec![vec![7]], vec![])],
                None,
            ),
        );
        fx.program.people.indexed_catalogs.insert(
            "other_rig.bcf".into(),
            (
                vec![mk_skel(
                    "shared_rig",
                    "other_rig.bcf",
                    0,
                    vec![vec![9]],
                    vec![],
                )],
                None,
            ),
        );
        fx.program.people.indexed_suit_catalogs.insert(
            "mixed.bcf".into(),
            (
                vec![
                    ("shared_acc".into(), true),
                    ("sib_body".into(), true),
                    ("broken_suit".into(), true),
                ],
                None,
            ),
        );
        fx.program.people.indexed_suit_catalogs.insert(
            "other_body.bcf".into(),
            (vec![("sib_body".into(), false)], None),
        );
        fx.program
            .people
            .indexed_suit_catalogs
            .insert("broken.bcf".into(), (Vec::new(), Some("corrupt".into())));
        fx.program
            .people
            .suit_errors
            .insert("broken.bcf".into(), "corrupt".into());

        // Attempt `broken_suit` via Change Suit -> traps on decode error AND
        // records `loaded_indexed_suits["broken_suit"] = None` (`0x5a3965`).
        fx.program
            .libraries
            .get_mut(&SIM_FILE)
            .unwrap()
            .strings
            .insert(304, vec!["broken_suit".into(), "shared_acc".into()]);
        fx.add(
            SIM_FILE,
            4098,
            tree(vec![ins(
                6,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 2, 0, 0, 0, 0, 0, 0],
            )]),
        );
        assert!(fx.run_with(SIM, 4098, 0, &[], 100).is_err());
        assert_eq!(
            fx.world.loaded_indexed_suits.get("broken_suit"),
            Some(&None)
        );

        // Now dress `shared_acc` via Change Suit (opcode 6): loads `mixed.bcf`
        // at dress time, populating BOTH `loaded_indexed_skeletons["shared_rig"]`
        // AND `loaded_indexed_suits["sib_body"]`, while leaving `broken_suit`
        // as `None` (`0x59f34c`)!
        fx.add(
            SIM_FILE,
            4099,
            tree(vec![ins(
                6,
                RETURN_TRUE,
                RETURN_FALSE,
                [1, 2, 0, 0, 0, 0, 0, 0],
            )]),
        );
        assert_eq!(fx.run_with(SIM, 4099, 0, &[], 100), Ok(true));
        assert_eq!(
            fx.world.loaded_indexed_skeletons.get("shared_rig"),
            Some(&Some("mixed.bcf".to_owned()))
        );
        assert_eq!(
            fx.world.loaded_indexed_suits.get("sib_body"),
            Some(&Some(true))
        );
        assert_eq!(
            fx.world.loaded_indexed_suits.get("broken_suit"),
            Some(&None),
            "0x5a3965 failed appearance must not be overwritten by later mixed.bcf"
        );

        // Non-short-circuiting outfit lookup: a Sim with an eager HEAD row
        // STILL resolves its indexed body suit `body_trigger` during portrait
        // rendering, populating `body_trigger.bcf`'s sibling `body_sib`.
        fx.program.people.eager_suits.insert("eager_head".into());
        fx.program.people.head_suits.insert("eager_head".into());
        fx.program
            .people
            .indexed_suits
            .insert("body_trigger".into(), Ok("body_trigger.bcf".into()));
        fx.program.people.indexed_suit_catalogs.insert(
            "body_trigger.bcf".into(),
            (
                vec![("body_trigger".into(), false), ("body_sib".into(), true)],
                None,
            ),
        );
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("body_trigger,BODY=b".into()),
                head: Some("eager_head,HEAD-HEAD=h".into()),
                ..Default::default()
            },
        );
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 0;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.loaded_indexed_suits.get("body_sib"),
            Some(&Some(true)),
            "HEAD-binding head row must not skip resolving the outfit body suit"
        );
    }

    // 2. P2 #4: Pet `data[8] = 0` mutation runs BEFORE skeleton HEAD bone check,
    // and definitions with `def_flags & 8 != 0` and negative suit do NOT reset
    // `person.data[8]` to 0.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.program.people.skeletons.insert(
            SIM_GUID,
            mk_skel(
                "nohead_skel",
                "nohead.bcf",
                0,
                vec![vec![]],
                vec![Bone {
                    name: "PELVIS".into(),
                    parent: None,
                    translation: [0; 3],
                    rotation: [0, 0, 0, 1.0f32.to_bits()],
                    blends: false,
                }],
            ),
        );
        let person = fx
            .world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        person.data[65] = 2; // dogmale
        person.data[8] = 4;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(!fx.world.portraits.contains(&SIM_GUID));
        assert_eq!(
            fx.world.objects[&SIM].person.as_ref().unwrap().data[8],
            0,
            "0x4e7801 -> 0x58a2e5 must clear pet data[8] even when skeleton lacks HEAD bone"
        );

        // With def_flags & 8 set and negative suit: bypasses 0x58a2e5.
        fx.words(SIM_FILE)[102] = 8;
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = -2;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.objects[&SIM].person.as_ref().unwrap().data[8],
            -2,
            "0x58a1e4 -> 0x58a1f8 must NOT reset data[8] when def_flags & 8 != 0 and suit <= 0"
        );
    }

    // 3. P2 #5: Adult formalwear with empty row 30 (`0x58a73e`) binds rows 28 &
    // 29 (`0x58d742`/`0x58d759`) ONLY when BOTH have non-empty suit names.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.program.people.head_suits.insert("extra28_head".into());
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("plain_body,BODY=b".into()),
                head: None,
                age: Some("27".into()),
                suit_rows: BTreeMap::from([
                    (27, "formal27_nohead,BODY=f".into()),
                    (28, "extra28_head,HEAD-HEAD=e".into()),
                    (29, "extra29_nohead,BODY=e".into()),
                ]),
                ..Default::default()
            },
        );
        let person = fx
            .world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        person.data[65] = 0;
        person.data[8] = 4;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            fx.world.portraits.contains(&SIM_GUID),
            "0x58a73e -> 0x58d742 must bind row 28 when row 30 is empty and both rows 28 & 29 are non-empty"
        );

        // Removing row 29 on a freshly dressed Sim makes `0x58d72e` skip BOTH rows 28 and 29!
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.world.person_outfits.clear();
        fx.program
            .people
            .profiles
            .get_mut(&SIM_GUID)
            .unwrap()
            .suit_rows
            .remove(&29);
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            !fx.world.portraits.contains(&SIM_GUID),
            "0x58d72e must skip row 28 when row 29 is empty"
        );
    }

    // 4. P2 #6: Catalog Dialog without BMP_ 2002 handles refused pet (`0x4e969d`)
    // nonfatally instead of trapping.
    {
        let (mut fx, _, record) = visitors();
        fx.program.expansions = 0; // no Unleashed / Makin' Magic
        fx.program
            .people
            .skeleton_names
            .insert(WIDGET, "dog".into());
        fx.add(
            LAMP_FILE,
            4098,
            tree(vec![
                assign(scope::STACK_OBJECT, 0, scope::LITERAL, record, 1),
                ins(36, RETURN_TRUE, RETURN_FALSE, [0, 0, 0, 0, 0, 0, 0, 5]),
            ]),
        );
        assert_eq!(fx.run_with(LAMP, 4098, 0, &[], 100), Ok(true));
        assert!(!fx.world.portraits.contains(&WIDGET));
    }

    // 5. P2 #7: Pet creation (`0x40a5e2`) overwrites rows 30..=34 with "NONE",
    // and negative-age empty-list (`0x40ac57`) clears rows 15..=18 to "".
    {
        let (mut fx, _, _) = visitors();
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        fx.program.expansions = 0x20;
        let pet_file = fx.add_type(TEMPLATE_DOG, 2, 4096, 4097);
        fx.add(
            pet_file,
            4096,
            tree(vec![
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, RETURN_TRUE),
            ]),
        );
        fx.add(
            pet_file,
            4097,
            tree(vec![
                assign(scope::PARAMETERS, 0, scope::LITERAL, 10, 1),
                ins(0, 0, 0, [0; 8]),
            ]),
        );
        equip(&mut fx, TEMPLATE_DOG, "dog", 12);
        fx.program
            .people
            .profiles
            .get_mut(&TEMPLATE_DOG)
            .unwrap()
            .tones = vec!["lgt".into()];
        fx.program.wardrobe = Some(Wardrobe {
            suits: ["b001dog_01".into()].into(),
            textures: ["b001dog_01.bmp".into()].into(),
            errors: BTreeMap::new(),
        });
        fx.add(
            SIM_FILE,
            4098,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
                assign(scope::LOCALS, 1, scope::LITERAL, 27, 2),
                assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
                ins(19, 4, RETURN_FALSE, [0, 1, 2, 1, 0, 0, 0, 0]),
                assign(scope::GLOBAL, 36, scope::STACK_OBJECT, 0, RETURN_TRUE),
            ]),
        );
        assert_eq!(fx.run_with(SIM, 4098, 0, &[], 1000), Ok(true));
        let pet_guid = fx.world.neighbors.get(fx.world.globals[36]).unwrap().guid;
        let pet_rows = &fx.world.made_outfits[&pet_guid];
        for r in [2usize, 30, 31, 32, 33, 34] {
            assert_eq!(pet_rows.get(&r).map(String::as_str), Some("NONE"));
        }

        // Negative-age empty-list branch (`0x40ac57`): clears rows 1, 2, 15..=18.
        let tpl_file = fx.add_type(TEMPLATE_PERSON, 2, 4096, 4097);
        fx.add(
            tpl_file,
            4096,
            tree(vec![assign(
                scope::MY_DATA,
                4,
                scope::LITERAL,
                1,
                RETURN_TRUE,
            )]),
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
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("b999fafit_miss,BODY=x".into()),
                head: Some("c999fa_miss,HEAD-HEAD=y".into()),
                gender: Some("female".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                ..Default::default()
            },
        );
        fx.add(
            SIM_FILE,
            4099,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
                assign(scope::LOCALS, 1, scope::LITERAL, -1, 2),
                assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
                ins(19, 4, RETURN_FALSE, [0, 1, 2, 0, 0, 0, 0, 0]),
                assign(scope::GLOBAL, 36, scope::STACK_OBJECT, 0, RETURN_TRUE),
            ]),
        );
        assert_eq!(fx.run_with(SIM, 4099, 0, &[], 1000), Ok(true));
        let copy_guid = fx.world.neighbors.get(fx.world.globals[36]).unwrap().guid;
        let copy_rows = &fx.world.made_outfits[&copy_guid];
        for r in [1usize, 2, 17, 18] {
            assert_eq!(copy_rows.get(&r).map(String::as_str), Some(""));
        }
        // 0x40af51 -> 0x40af5b subsequently rebuilds rows 15, 16, 27.
        for r in [15usize, 16, 27] {
            assert_eq!(copy_rows.get(&r).map(String::as_str), Some(",BODY="));
        }
    }

    // 6. P3 #1: Writing stack-object person data from a non-person (`LAMP`)
    // must NOT insert `LAMP_GUID` into `dirty_portraits` (`0x4eb0eb` exits on
    // portrait-table miss).
    {
        let (mut fx, _, _) = visitors();
        fx.add(
            LAMP_FILE,
            4098,
            tree(vec![
                assign(scope::STACK_OBJECT, 0, scope::LITERAL, SIM as i16, 1),
                assign(STACK_OBJECT_PERSON_DATA, 68, scope::LITERAL, 1, RETURN_TRUE),
            ]),
        );
        assert_eq!(fx.run_with(LAMP, 4098, 0, &[], 100), Ok(true));
        assert!(
            !fx.world.dirty_portraits.contains(&LAMP_GUID),
            "0x4eb0eb must ignore non-person GUIDs not in the portrait table"
        );
        // Writing My Person Data[68] from a person (`SIM`) DOES dirty `SIM_GUID`.
        fx.add(
            SIM_FILE,
            4098,
            tree(vec![assign(
                MY_PERSON_DATA,
                68,
                scope::LITERAL,
                1,
                RETURN_TRUE,
            )]),
        );
        assert_eq!(fx.run_with(SIM, 4098, 0, &[], 100), Ok(true));
        assert!(fx.world.dirty_portraits.contains(&SIM_GUID));
    }
}

#[test]
fn round_17_appearance_bcf_rng_missing_outfit_retention_hand_extras_dirty_clear_and_household_pet()
{
    use sims_content::{Profile, Skeleton};
    let mk_skel = |name: &str, file: &str, place: usize, scales: Vec<Vec<i32>>| Skeleton {
        name: name.to_owned(),
        file: file.to_owned(),
        place,
        scales,
        bones: Vec::new(),
    };

    // 1. P2 #1: Appearance-triggered `.bcf` load draws CRT RNG for every
    // skeleton in that `.bcf` (`0x59f107` -> `0x598af7`) and records its file
    // in `world.skeleton_files` so later person construction draws only its own
    // copy (`0x598da3`).
    {
        let (mut fx, _, _) = visitors();
        fx.program
            .people
            .indexed_suits
            .insert("rng_acc".into(), Ok("rng_mixed.bcf".into()));
        fx.program.people.indexed_catalogs.insert(
            "rng_mixed.bcf".into(),
            (
                vec![mk_skel("rng_skel", "rng_mixed.bcf", 0, vec![vec![5]])],
                None,
            ),
        );
        fx.program.people.indexed_suit_catalogs.insert(
            "rng_mixed.bcf".into(),
            (vec![("rng_acc".into(), true)], None),
        );
        fx.program
            .libraries
            .get_mut(&SIM_FILE)
            .unwrap()
            .strings
            .insert(304, vec!["rng_acc".into()]);
        fx.add(
            SIM_FILE,
            4098,
            tree(vec![ins(
                6,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 2, 0, 0, 0, 0, 0, 0],
            )]),
        );
        let before_rng = fx.world.crt_rng.state();
        assert_eq!(fx.run_with(SIM, 4098, 0, &[], 100), Ok(true));
        assert!(
            fx.world.skeleton_files.contains("rng_mixed.bcf"),
            "0x59f107 must mark the mixed `.bcf` read when opened by an appearance"
        );
        assert_ne!(
            fx.world.crt_rng.state(),
            before_rng,
            "0x59f107 -> 0x598af7 must advance CRT RNG for the `.bcf`'s skeletons"
        );
    }

    // 2. P2 #3: Failed outfit lookup (`0x583576`) keeps the existing everyday
    // body attachment; NPC flag 0x08 with suit <= 0 (`0x58a1f8`) skips new
    // `.ndx` loads.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.program
            .people
            .eager_suits
            .insert("everyday_head_body".into());
        fx.program
            .people
            .head_suits
            .insert("everyday_head_body".into());
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("everyday_head_body,BODY=b".into()),
                head: None,
                suit_rows: BTreeMap::from([(16, "missing_swim_suit,BODY=s".into())]),
                ..Default::default()
            },
        );
        // First dress everyday (suit 0) so `everyday_head_body` is actually
        // bound in `person_outfits`, then switch to `missing_swim_suit` (suit 2):
        // 0x583576 miss must keep `everyday_head_body` attached.
        {
            let person = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            person.data[65] = 0;
            person.data[8] = 0;
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 2; // swim suit -> missing_swim_suit misses at 0x583576
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            fx.world.portraits.contains(&SIM_GUID),
            "0x583576 miss must keep everyday_head_body attached"
        );

        // NPC flag 0x08 with suit <= 0 must NOT trigger lazy `.ndx` loads.
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.words(SIM_FILE)[102] = 8;
        fx.program
            .people
            .indexed_suits
            .insert("lazy_npc_body".into(), Ok("lazy_npc.bcf".into()));
        fx.program.people.indexed_suit_catalogs.insert(
            "lazy_npc.bcf".into(),
            (vec![("lazy_npc_body".into(), true)], None),
        );
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("lazy_npc_body,BODY=b".into()),
                head: None,
                ..Default::default()
            },
        );
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 0;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            !fx.world.loaded_indexed_suits.contains_key("lazy_npc_body"),
            "0x58a1f8 must not trigger `.ndx` lookup when def_flags & 8 != 0 and suit <= 0"
        );
    }

    // 3. P2 #4: Extra pieces from person data words 22 & 23 (`0x58a453..0x58a675`)
    // bind even on everyday suit 0, and generated workwear/pajamas trap when
    // `.ndx` suits exist so sibling loads cannot be skipped.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.program.people.head_suits.insert("hand_left_head".into());
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("plain_body,BODY=b".into()),
                head: None,
                suit_rows: BTreeMap::from([
                    (19, "hand_left_head,HAND=l".into()),
                    (20, "hand_right_plain,HAND=r".into()),
                ]),
                ..Default::default()
            },
        );
        let person = fx
            .world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        person.data[65] = 0;
        person.data[8] = 0;
        person.data[22] = 1; // row 19
        person.data[23] = 1; // row 20
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            fx.world.portraits.contains(&SIM_GUID),
            "0x58a453..0x58a675 must bind words 22/23 hand rows even on everyday suit 0"
        );

        // Workwear (suit 3, `0x45ee5f`) resolves cleanly even when unrelated
        // `.ndx` suits exist, without trapping.
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.program
            .people
            .indexed_suits
            .insert("some_ndx".into(), Ok("x.bcf".into()));
        fx.program
            .people
            .eager_suits
            .insert("hand_left_head".into());
        fx.program.people.profiles.get_mut(&SIM_GUID).unwrap().head =
            Some("hand_left_head,HEAD-HEAD=h".into());
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 3;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(fx.world.portraits.contains(&SIM_GUID));
    }

    // 4. P2 #5: Dressing an unbound head slot (`0x58d4e1` -> `0x4eb0cc` ->
    // `0x4e897e`) clears `dirty_portraits` even when rendering fails, so a
    // second off-lot query runs only one factory attempt (`0x4e938c`) instead
    // of two (`0x4e92f2` + `0x4e938c`).
    {
        let (mut fx, _, record) = visitors();
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        fx.program.people.portrait_head.insert(WIDGET, false);
        fx.world.dirty_portraits.insert(WIDGET);
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1),
            Ok(true)
        );
        assert!(!fx.world.portraits.contains(&WIDGET));
        assert!(
            !fx.world.dirty_portraits.contains(&WIDGET),
            "0x58d4e1 -> 0x4eb0cc must clear dirty_portraits even when render fails"
        );
    }

    // 5. P2 #6: Initial dressing in `person_factory` does NOT erase indexed
    // decode uncertainty: a suit whose catalog stopped decoding with an error
    // still traps on portrait query instead of silently returning Missing.
    {
        let (mut fx, _, record) = visitors();
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        fx.program.people.portrait_head.remove(&WIDGET);
        fx.program.people.profiles.insert(
            WIDGET,
            Profile {
                body: Some("corrupt_indexed_suit,BODY=b".into()),
                head: None,
                ..Default::default()
            },
        );
        fx.program
            .people
            .indexed_suits
            .insert("corrupt_indexed_suit".into(), Ok("corrupt.bcf".into()));
        fx.program
            .people
            .indexed_suit_catalogs
            .insert("corrupt.bcf".into(), (Vec::new(), Some("corrupt".into())));
        fx.program
            .people
            .suit_errors
            .insert("corrupt.bcf".into(), "corrupt".into());
        assert!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1).is_err(),
            "decode uncertainty must survive initial dressing in person_factory"
        );
    }

    // 6. P2 #7: Catalog Dialog checks disallowed pet (`0x440d63` -> `0x4e969d`)
    // BEFORE the household guard so an off-lot household pet with a disallowed
    // expansion mask continues nonfatally instead of trapping.
    {
        let (mut fx, _, record) = visitors();
        fx.program.expansions = 0;
        fx.program
            .people
            .skeleton_names
            .insert(WIDGET, "kat".into());
        fx.world.globals[9] = 109;
        fx.world.families[0].members.push(WIDGET);
        fx.add(
            LAMP_FILE,
            4098,
            tree(vec![
                assign(scope::STACK_OBJECT, 0, scope::LITERAL, record, 1),
                ins(36, RETURN_TRUE, RETURN_FALSE, [0, 0, 0, 0, 0, 0, 0, 5]),
            ]),
        );
        assert_eq!(fx.run_with(LAMP, 4098, 0, &[], 100), Ok(true));
        assert!(!fx.world.portraits.contains(&WIDGET));
    }

    // 7. P2 #2: Saved-house object loading (`0x58838b -> 0x5a073b -> 0x5a0384`)
    // resolves each saved person's skeleton through runtime `.ndx` order,
    // populating sibling skeletons from the first person's `.bcf` so a later
    // saved person binds that sibling instead of opening its own `.bcf`.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
        let alpha_skel = sims_content::Skeleton {
            name: "alpha".into(),
            file: "alpha.bcf".into(),
            place: 0,
            scales: vec![vec![1], vec![1, 1]],
            bones: Vec::new(),
        };
        let beta_from_alpha = sims_content::Skeleton {
            name: "beta".into(),
            file: "alpha.bcf".into(),
            place: 1,
            scales: vec![vec![1], vec![1, 1]],
            bones: Vec::new(),
        };
        let beta_from_beta = sims_content::Skeleton {
            name: "beta".into(),
            file: "beta.bcf".into(),
            place: 0,
            scales: vec![vec![1, 1, 1, 1, 1]],
            bones: Vec::new(),
        };
        fx.program
            .people
            .skeleton_names
            .insert(SIM_GUID, "alpha".into());
        fx.program
            .people
            .skeleton_names
            .insert(WIDGET, "beta".into());
        fx.program
            .people
            .indexed_skeletons
            .insert("alpha".into(), Ok("alpha.bcf".into()));
        fx.program
            .people
            .indexed_skeletons
            .insert("beta".into(), Ok("beta.bcf".into()));
        fx.program.people.indexed_catalogs.insert(
            "alpha.bcf".into(),
            (vec![alpha_skel, beta_from_alpha], None),
        );
        fx.program
            .people
            .indexed_catalogs
            .insert("beta.bcf".into(), (vec![beta_from_beta], None));

        let mut world = sims_sim::World::empty();
        world.neighbors.slots.push(Some(Neighbor {
            guid: SIM_GUID,
            id: 1,
            data: vec![20; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 1,
            stale: false,
        }));
        world.neighbors.slots.push(Some(Neighbor {
            guid: WIDGET,
            id: 2,
            data: vec![20; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 1,
            stale: false,
        }));
        let mk_inst = |id: u16, guid: u32| sims_formats::saved_objects::Instance {
            id,
            type_id: 0,
            guid,
            footprint: [0; 4],
            position: [0; 3],
            unknown_short: 0,
            attributes: Vec::new(),
            temps: vec![0; 8],
            data: vec![0; 72],
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: vec![sims_formats::saved_objects::Slot {
                unknown: 0,
                object: 0,
            }],
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: Some(person()),
            registry_word: Some(0),
            consumed_bits: 0,
            record: Vec::new(),
        };
        let saved = sims_formats::saved_objects::Objects {
            version: 0x47,
            order: vec![1, 2],
            types_by_id: BTreeMap::new(),
            instances: BTreeMap::from([(1, mk_inst(1, SIM_GUID)), (2, mk_inst(2, WIDGET))]),
            tail: Vec::new(),
        };
        let read = fx
            .interpreter()
            .read_objects(&mut world, &saved, &fx.program.people)
            .expect("saved-house read must succeed");
        assert_eq!(read.objects, vec![1, 2]);
        assert_eq!(
            world.loaded_indexed_skeletons.get("beta"),
            Some(&Some("alpha.bcf".into())),
            "first saved person's alpha.bcf must claim sibling beta"
        );
        assert!(
            !world.skeleton_files.contains("beta.bcf"),
            "second saved person must reuse alpha.bcf's beta instead of opening beta.bcf"
        );
        // Draws (32 per wiggle scale): alpha.bcf file skeletons (1 + 2 = 3) +
        // person 1 own alpha (1) + person 2 own beta from alpha.bcf (2) = 6 * 32 = 192.
        assert_eq!(read.draws, 192);
    }
}

#[test]
fn round_18_reopen_bcf_rng_missing_file_saved_read_frame_draw_live_slots_head_dirty_costumes_and_none()
 {
    use sims_content::Profile;

    // 1. P2 #1: Reopening a lazy `.bcf` via a second unresolved `.ndx` entry
    // draws every skeleton in that `.bcf` AGAIN (`0x59e70a -> 0x59f107`),
    // including duplicate skeleton entries.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        let dup_a = sims_content::Skeleton {
            name: "dup".into(),
            file: "shared.bcf".into(),
            place: 0,
            scales: vec![vec![1], vec![1]],
            bones: Vec::new(),
        };
        let dup_b = sims_content::Skeleton {
            name: "dup".into(),
            file: "shared.bcf".into(),
            place: 1,
            scales: vec![vec![1], vec![1]],
            bones: Vec::new(),
        };
        fx.program
            .people
            .indexed_catalogs
            .insert("shared.bcf".into(), (vec![dup_a, dup_b], None));
        fx.program
            .people
            .indexed_suit_catalogs
            .insert("shared.bcf".into(), (Vec::new(), None));
        fx.program
            .people
            .indexed_suits
            .insert("miss_one".into(), Ok("shared.bcf".into()));
        fx.program
            .people
            .indexed_suits
            .insert("miss_two".into(), Ok("shared.bcf".into()));
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("miss_one,BODY=a".into()),
                head: Some("miss_two,HEAD-HEAD=b".into()),
                ..Default::default()
            },
        );
        let p = fx
            .world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        p.data[65] = 0;
        p.data[8] = 0;
        let mut expected_rng = fx.world.crt_rng;
        // Two opens of `shared.bcf`, each drawing 2 skeletons * 32 = 64 draws -> 128 total.
        for _ in 0..128 {
            let _ = expected_rng.next_u15();
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.crt_rng, expected_rng,
            "reopening shared.bcf for miss_two must draw both duplicate skeletons again"
        );
    }

    // 2. P2 #2: Indexed appearance pointing to a missing file (`0x5a392f ->
    // 0x5a3965`) is a definite miss (retaining previous outfit) even when an
    // unrelated catalog populated `suit_errors`.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.program.people.eager_suits.insert("everyday_head".into());
        fx.program.people.head_suits.insert("everyday_head".into());
        fx.program
            .people
            .indexed_suits
            .insert("missing_file_swim".into(), Err("missing file".into()));
        fx.program
            .people
            .suit_errors
            .insert("unrelated.bcf".into(), "unrelated corrupt catalog".into());
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("everyday_head,BODY=e".into()),
                head: None,
                suit_rows: BTreeMap::from([(16, "missing_file_swim,BODY=s".into())]),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 0;
            p.data[8] = 0;
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 2;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            fx.world.portraits.contains(&SIM_GUID),
            "missing indexed catalog file must be a definite miss, retaining everyday_head"
        );
    }

    // 3. P2 #3: Saved-house read binds `appearance[0..10]` and `accessories`
    // in pass 2 (after construction), triggers lazy `.ndx` `.bcf` RNG draws,
    // and marks portrait dirty (`0x591b53 -> 0x4eb0eb`) for non-empty saved head.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
        let saved_skel = sims_content::Skeleton {
            name: "adult".into(),
            file: "adult.bcf".into(),
            place: 0,
            scales: vec![vec![1]],
            bones: Vec::new(),
        };
        let app_skel = sims_content::Skeleton {
            name: "acc_skel".into(),
            file: "saved_app.bcf".into(),
            place: 0,
            scales: vec![vec![1]],
            bones: Vec::new(),
        };
        fx.program
            .people
            .skeletons_by_name
            .insert("adult".into(), saved_skel);
        fx.program
            .people
            .skeleton_names
            .insert(SIM_GUID, "adult".into());
        fx.program
            .people
            .indexed_catalogs
            .insert("saved_app.bcf".into(), (vec![app_skel], None));
        fx.program.people.indexed_suit_catalogs.insert(
            "saved_app.bcf".into(),
            (vec![("saved_body".into(), true)], None),
        );
        fx.program
            .people
            .indexed_suits
            .insert("saved_body".into(), Ok("saved_app.bcf".into()));
        fx.program.people.eager_suits.insert("saved_head".into());
        fx.program.people.head_suits.insert("saved_head".into());

        let mut p = person();
        p.appearance[0] = "saved_body".into();
        p.appearance[1] = "BODY=x".into();
        p.appearance[8] = "saved_head".into();
        p.appearance[9] = "HEAD-HEAD=h".into();
        let inst = sims_formats::saved_objects::Instance {
            id: 1,
            type_id: 0,
            guid: SIM_GUID,
            footprint: [0; 4],
            position: [0; 3],
            unknown_short: 0,
            attributes: Vec::new(),
            temps: vec![0; 8],
            data: vec![0; 72],
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: vec![sims_formats::saved_objects::Slot {
                unknown: 0,
                object: 0,
            }],
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: Some(p),
            registry_word: Some(0),
            consumed_bits: 0,
            record: Vec::new(),
        };
        let saved = sims_formats::saved_objects::Objects {
            version: 0x47,
            order: vec![1],
            types_by_id: BTreeMap::new(),
            instances: BTreeMap::from([(1, inst)]),
            tail: Vec::new(),
        };
        let mut world = sims_sim::World::empty();
        world.neighbors.slots.push(Some(Neighbor {
            guid: SIM_GUID,
            id: 1,
            data: vec![20; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 1,
            stale: false,
        }));
        let read = fx
            .interpreter()
            .read_objects(&mut world, &saved, &fx.program.people)
            .expect("saved read");
        assert!(world.loaded_indexed_suits.contains_key("saved_body"));
        assert!(world.dirty_portraits.contains(&SIM_GUID));
        // adult.bcf file (32) + own adult (32) + saved_app.bcf skeleton (32) = 96.
        assert_eq!(read.draws, 96);
        assert_eq!(
            world.person_outfits[&1]
                .head
                .as_ref()
                .map(|b| b.suit.as_str()),
            Some("saved_head")
        );
    }

    // 4. P2 #5 & #6 & P3 #1:
    // - `0x583576` retains a PREVIOUS CHANGED OUTFIT (everyday non-HEAD ->
    //   formal HEAD -> missing swim keeps formal HEAD).
    // - NPC `def_flags & 8` with `raw_suit <= 0` does NOT count an unattached
    //   globally registered suit as worn.
    // - Changing a bound head descriptor runs `0x58d518 -> 0x4eb0eb` (marks
    //   dirty & clears cache).
    // - Literal `"none"` appearance is NOT skipped when registered (`0x583569`).
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.program.people.eager_suits.insert("plain_ev".into());
        fx.program.people.eager_suits.insert("formal_head".into());
        fx.program.people.head_suits.insert("formal_head".into());
        fx.program.people.eager_suits.insert("head_a".into());
        fx.program.people.eager_suits.insert("head_b".into());
        fx.program.people.eager_suits.insert("none".into());
        fx.program.people.head_suits.insert("none".into());
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("plain_ev,BODY=e".into()),
                head: Some("head_a,HEAD-HEAD=a".into()),
                age: Some("27".into()),
                suit_rows: BTreeMap::from([
                    (30, "formal_head,BODY=f".into()),
                    (16, "missing_swim,BODY=s".into()),
                ]),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 0;
            p.data[8] = 4; // dress into formal_head first
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(fx.world.portraits.contains(&SIM_GUID));

        // Now switch to missing swim (suit 2): `formal_head` must stay attached!
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 2;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            fx.world.portraits.contains(&SIM_GUID),
            "0x583576 must retain previous changed outfit (formal_head), not fall back to plain_ev"
        );

        // P2 #6: Change head descriptor to `head_b` (which has NO HEAD binding)
        // and draw a frame (`0x590cb8 -> 0x58a178(0)`): `0x58d518 -> 0x4eb0eb`
        // must invalidate `portraits` and set `dirty_portraits`!
        fx.program.people.profiles.get_mut(&SIM_GUID).unwrap().head =
            Some("head_b,HEAD-HEAD=b".into());
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 0; // trigger dress on non-forced frame draw
        fx.world.objects.get_mut(&SIM).unwrap().position = [32, 32, 0];
        fx.world.size = 16;
        fx.world.animators.insert(
            SIM,
            sims_sim::animator::Animator {
                profile_stale: false,
                ..sims_sim::animator::LOADED
            },
        );
        sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream)
            .draw_frame(&mut fx.world, 0)
            .expect("draw_frame");
        assert!(
            !fx.world.portraits.contains(&SIM_GUID) && fx.world.dirty_portraits.contains(&SIM_GUID),
            "0x590cb8 -> 0x58d518 -> 0x4eb0eb must clear portrait and mark dirty on changed head"
        );

        // P3 #1: Literal `"none"` appearance registered in `head_suits` binds!
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.dirty_portraits.remove(&SIM_GUID);
        fx.program.people.profiles.get_mut(&SIM_GUID).unwrap().head =
            Some("none,HEAD-HEAD=n".into());
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            fx.world.portraits.contains(&SIM_GUID),
            "0x583569 must pass literal \"none\" to 0x5a044d instead of skipping it"
        );

        // P2 #5 NPC: `def_flags & 8` with `raw_suit <= 0` and empty slots must
        // NOT count a globally registered profile suit as worn.
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 8;
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
        fx.program.people.eager_suits.insert("global_head".into());
        fx.program.people.head_suits.insert("global_head".into());
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("global_head,BODY=g".into()),
                head: None,
                ..Default::default()
            },
        );
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 0;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            !fx.world.portraits.contains(&SIM_GUID),
            "0x58a1f8 NPC with raw_suit <= 0 and no bound slots must not count unattached global_head"
        );
    }

    // 5. P2 #7: Generated matched pajamas (suit 5, `0x45f034`), skeleton (suit 6,
    // `0x45f15a`), and Makin' Magic costume (suit 57 donkey head) bind cleanly.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.program.people.eager_suits.insert("pajamaffit_01".into());
        fx.program.people.head_suits.insert("pajamaffit_01".into());
        fx.program
            .people
            .eager_suits
            .insert("cmagicmafat_donkey_01".into());
        fx.program
            .people
            .head_suits
            .insert("cmagicmafat_donkey_01".into());
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("b001fafit_01,BODY=b".into()),
                head: None,
                gender: Some("female".into()),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 1;
            p.data[58] = 27;
            p.data[8] = 5; // matched pajamas (empty row 32 -> 0x45f034)
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(fx.world.portraits.contains(&SIM_GUID));

        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.world.person_outfits.clear();
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 57; // donkey head
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(fx.world.portraits.contains(&SIM_GUID));
    }

    // 6. P3 #2: Shared placement error from UI Effect labels
    // "UI Effect's portrait" instead of "a dialog's portrait".
    {
        let (mut fx, file, record) = visitors();
        // Two rooms so `data[40] = 0x400` marks a different room depending on
        // which tile the missing `view_centre` selects.
        fx.world.rooms.rooms.insert(2, Default::default());
        for a in 3..6 {
            for b in 1..5 {
                fx.world.rooms.map[0][a * 64 + b] = 2;
            }
        }
        fx.add(
            file,
            4096,
            tree(vec![
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
                assign(scope::MY_DATA, 40, scope::LITERAL, 0x400, RETURN_TRUE),
            ]),
        );
        let err = run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("UI Effect's portrait") && !err.contains("a dialog's portrait"),
            "expected UI Effect label in placement diagnostic, got: {err}"
        );
    }
}

#[test]
fn round_19_global34_stale_refresh_mummy_hashtable_textures_clothing_workwear_npc_and_exact_saved()
{
    use sims_content::{Profile, SuitTable};
    use sims_formats::career::Level;
    use sims_formats::saved_objects::Accessory;

    // 1. P2 #1: Global 34 traps on a fresh object (`[object + 0x128] == NULL`),
    // but succeeds returning `globals[34]` after `Random Number` (`0x451e8d`)
    // populates `scratch_ptr_set`.
    {
        let (mut fx, _, record) = visitors();
        fx.world.globals[34] = record;
        assert!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::GLOBAL, 34, 1).is_err(),
            "fresh object must trap on Global 34"
        );
        // Run Random Number (op 8): dest = temp 0 (scope 8, idx 0), range = literal 1 (scope 7, idx 1).
        fx.add(
            LAMP_FILE,
            4099,
            tree(vec![
                ins(8, 1, RETURN_FALSE, [0, 0, 8, 0, 1, 0, 7, 0]),
                ins(
                    34,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [1, scope::GLOBAL, 34, 0, 1, 0, 0, 0],
                ),
            ]),
        );
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        fx.program.people.portrait_head.insert(WIDGET, true);
        assert_eq!(fx.run_with(LAMP, 4099, 0, &[], 100), Ok(true));
        assert!(fx.world.scratch_ptr_set.contains(&LAMP));
        assert!(fx.world.portraits.contains(&WIDGET));
    }

    // 2. P2 #2: Stale appearance refresh (`0x5888d3`) detaches slots, dresses
    // BEFORE reloading profile words (observing pre-reload data[65] & suit),
    // clears `person.accessories` + `outfit.accessories`, and resets
    // `last_dressed[0..2]` to `-1`.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
        fx.program.people.eager_suits.insert("pre_body".into());
        fx.program
            .people
            .indexed_suits
            .insert("pre_suit_lazy".into(), Ok("pre.bcf".into()));
        fx.program.people.indexed_suit_catalogs.insert(
            "pre.bcf".into(),
            (vec![("pre_suit_lazy".into(), true)], None),
        );
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("pre_body,BODY=b".into()),
                head: None,
                gender: Some("female".into()),
                age: Some("27".into()),
                suit_rows: BTreeMap::from([(16, "pre_suit_lazy,BODY=s".into())]),
                ..Default::default()
            },
        );
        let p = fx
            .world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        p.data[65] = 0; // pre-reload: male; profile says female (1)
        p.data[8] = 2; // swim -> binds pre_suit_lazy during 0x588930
        p.data[22] = 7;
        p.data[23] = 9;
        p.accessories.push(Accessory {
            name: "stale_acc".into(),
            binding: String::new(),
        });
        fx.world.objects.get_mut(&SIM).unwrap().position = [32, 32, 0];
        fx.world.size = 16;
        // Leave animator at LOADED (`profile_stale: true`) and call draw_frame:
        // Step 1 runs 0x5888d3 (clears accessories, dresses pre_suit_lazy, reloads
        // gender=1/hands=0, resets last_dressed[0..2]=-1); step 2 runs 0x58a178(0).
        sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream)
            .draw_frame(&mut fx.world, 0)
            .expect("draw_frame");
        let p = fx.world.objects[&SIM].person.as_ref().unwrap();
        assert_eq!(p.data[65], 1);
        assert_eq!(p.data[22], 0);
        assert_eq!(p.data[23], 0);
        assert!(p.accessories.is_empty());
        assert!(fx.world.person_outfits[&SIM].accessories.is_empty());
        assert!(fx.world.loaded_indexed_suits.contains_key("pre_suit_lazy"));
    }

    // 3. P2 #3: Mummy suits 55 & 56 clear live `data[22]` & `data[23]`, and
    // variant 3 binds `nohandl`/`nohandr` BEFORE `0x40e5ce`.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.program.people.eager_suits.insert("nohandl".into());
        fx.program.people.eager_suits.insert("nohandr".into());
        fx.program.people.head_suits.insert("nohandl".into());
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("b001fafit_01,BODY=b".into()),
                head: None,
                gender: Some("female".into()),
                age: Some("27".into()),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 1;
            p.data[58] = 27;
            p.data[8] = 55;
            p.data[21] = 3; // mummy variant 3 -> nohandl / nohandr
            p.data[22] = 4;
            p.data[23] = 5;
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        let p = fx.world.objects[&SIM].person.as_ref().unwrap();
        assert_eq!(p.data[22], 0);
        assert_eq!(p.data[23], 0);
        let outfit = &fx.world.person_outfits[&SIM];
        assert_eq!(
            outfit
                .extra_left
                .as_ref()
                .map(|b| (b.suit.as_str(), b.texture.as_str())),
            Some(("nohandl", "nohand"))
        );
        assert_eq!(
            outfit
                .extra_right
                .as_ref()
                .map(|b| (b.suit.as_str(), b.texture.as_str())),
            Some(("nohandr", "nohand"))
        );
        assert!(fx.world.portraits.contains(&SIM_GUID));
    }

    // 4. P2 #4, #5, #6:
    // - `SuitTable` (`0x409255`) walk order chooses hash-bucket winner (not
    //   alphabetical) and caches on first call (`suit_scan_cache`);
    // - `0x40e674` fuzzy texture matching formats `"BODY={best}"` with
    //   `0x40b75a` suffix tiebreak and returns `""` on miss;
    // - `$g` hand template (`0x40e856`, suit 39 Blue Genie) prepends `"HAND="`;
    // - `0x409315` initializes rows 30..=34 into `World::made_outfits` (and
    //   child `"ADDED"` on rows 30 & 34).
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        // Two equal-score suits for `Magic$gAFit_BlueGenie` (suit 39 = Blue Genie).
        // Insert both into `SuitTable` so we can verify `SuitTable::scan_names` order wins!
        let mut table = SuitTable::default();
        for name in [
            "magicfafit_bluegenie_a",
            "magicfafit_bluegenie_z",
            "f100fafit_01",
            "s100fafit_01",
            "l100fafit_01",
            "w100fafit_01",
            "h501fafit_01",
        ] {
            table.insert_name(name.into());
        }
        let first_genie = table
            .scan_names()
            .into_iter()
            .find(|s| s.starts_with("magicfafit_bluegenie_"))
            .unwrap();
        fx.program.people.suit_table = table;
        for name in [
            "magicfafit_bluegenie_a",
            "magicfafit_bluegenie_z",
            "hmlo",
            "hmro",
            "f100fafit_01",
            "s100fafit_01",
            "l100fafit_01",
            "w100fafit_01",
            "h501fafit_01",
        ] {
            fx.program.people.eager_suits.insert(name.into());
        }
        fx.program.people.head_suits.insert(first_genie.clone());
        // Add two matching textures differing by suffix `_suf` (`0x40b75a`).
        fx.program.people.textures = vec![
            "bmagicfafit_bluegenie_other".into(),
            "bmagicfafit_bluegenie_suf".into(),
        ];
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                // `BODY=` (5) + 12 chars (`b001fafitlgt`) + `_suf` -> base_tex[17..] == "_suf"
                body: Some("b001fafit_01,BODY=b001fafitlgt_suf".into()),
                head: None,
                gender: Some("female".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                tones: vec!["".into(), "lgt".into()],
                suit_rows: BTreeMap::from([
                    (19, "hmlo,HAND=old".into()),
                    (20, "hmro,HAND=old".into()),
                ]),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 1;
            p.data[58] = 27;
            p.data[22] = 1; // point hand gesture -> row 19, `$h` -> 'P'
            p.data[23] = 1; // row 20 so 0x58d72e binds both hands
            p.data[8] = 39; // Blue Genie (uses $g in hand template -> "HAND=HfAP_BlueGenie")
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        let outfit = &fx.world.person_outfits[&SIM];
        assert_eq!(
            outfit.body.as_ref().map(|b| b.suit.as_str()),
            Some(first_genie.as_str()),
            "0x40e5ce must follow native cTSHashMap scan order"
        );
        assert_eq!(
            outfit.body.as_ref().map(|b| b.texture.as_str()),
            Some("BODY=bmagicfafit_bluegenie_suf"),
            "0x40e674 must match texture with 0x40b75a suffix tiebreak and BODY= prefix"
        );
        assert_eq!(
            outfit
                .extra_left
                .as_ref()
                .map(|b| (b.suit.as_str(), b.texture.as_str())),
            Some(("hmlo", "HAND=HfAP_BlueGenie")),
            "0x40e856 must prepend HAND= when expanding $g hand template"
        );
        assert!(fx.world.suit_scan_cache.is_some());
        let rows = &fx.world.made_outfits[&SIM_GUID];
        assert!(rows.get(&30).is_some_and(|s| s.starts_with("f100fafit_01")));
        assert!(rows.get(&31).is_some_and(|s| s.starts_with("s100fafit_01")));
        assert!(rows.get(&32).is_some_and(|s| s.starts_with("l100fafit_01")));
        assert!(rows.get(&33).is_some_and(|s| s.starts_with("w100fafit_01")));
        assert!(rows.get(&34).is_some_and(|s| s.starts_with("h501fafit_01")));
    }

    // 5. P2 #7: Workwear (`0x45ee5f`) picks `strings[2]` vs `strings[1]` from
    // live `data[65] & 1` (ignoring profile gender) and caches by `(job_type, job_level)`.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        let mut table = SuitTable::default();
        table.insert_name("work_male_suit".into());
        table.insert_name("work_female_suit".into());
        fx.program.people.suit_table = table;
        fx.program
            .people
            .eager_suits
            .insert("work_male_suit".into());
        fx.program
            .people
            .eager_suits
            .insert("work_female_suit".into());
        fx.program
            .people
            .head_suits
            .insert("work_female_suit".into());
        fx.program.careers = Some(vec![sims_vm::Career {
            id: 1,
            levels: vec![
                Level {
                    values: [0; 21],
                    strings: [
                        "L0".into(),
                        "work_male_suit".into(),
                        "work_female_suit".into(),
                        String::new(),
                        String::new(),
                    ],
                },
                Level {
                    values: [0; 21],
                    strings: [
                        "L1".into(),
                        "work_male_suit".into(),
                        "work_female_suit".into(),
                        String::new(),
                        String::new(),
                    ],
                },
            ],
        }]);
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("b001mafit_01,BODY=b".into()),
                head: None,
                gender: Some("male".into()), // profile says male!
                age: Some("27".into()),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 1; // live word 65 says female -> picks strings[2] ("work_female_suit")
            p.data[58] = 27;
            p.data[56] = 1; // job_type
            p.data[57] = 0; // job_level
            p.data[8] = 3; // workwear
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.person_outfits[&SIM]
                .body
                .as_ref()
                .map(|b| b.suit.as_str()),
            Some("work_female_suit")
        );
        // Flip live gender back to 0 while keeping same (job_type=1, job_level=0):
        // cache hit MUST reuse `work_female_suit`!
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[65] = 0;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.person_outfits[&SIM]
                .body
                .as_ref()
                .map(|b| b.suit.as_str()),
            Some("work_female_suit"),
            "0x45ee5f must reuse cached workwear when (job_type, job_level) is unchanged"
        );
        // Changing job_level to 1 invalidates the cache and picks male `strings[1]`.
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[57] = 1;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.person_outfits[&SIM]
                .body
                .as_ref()
                .map(|b| b.suit.as_str()),
            Some("work_male_suit")
        );
    }

    // 6. P2 #8: Positive NPC suit (`def_flags & 8`, `raw_suit > 0`) repeat guard
    // (`0x58a224`) skips rebinding when `last_dressed[0] == raw_suit` even when
    // `force == true`.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 8;
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
        fx.program.people.eager_suits.insert("npc_first".into());
        fx.program.people.head_suits.insert("npc_first".into());
        fx.program.people.eager_suits.insert("npc_second".into());
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                suit_rows: BTreeMap::from([(15, "npc_first,BODY=1".into())]),
                ..Default::default()
            },
        );
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 15;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(fx.world.portraits.contains(&SIM_GUID));
        // Now change row 15 to `npc_second` (no HEAD) while keeping `data[8] == 15`:
        // `0x58a224` must skip rebinding even on forced render!
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.program
            .people
            .profiles
            .get_mut(&SIM_GUID)
            .unwrap()
            .suit_rows
            .insert(15, "npc_second,BODY=2".into());
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.person_outfits[&SIM]
                .body
                .as_ref()
                .map(|b| b.suit.as_str()),
            Some("npc_first"),
            "0x58a224 must skip rebinding repeat positive NPC suit even when forced"
        );
    }

    // 7. P2 #9 & #10: Saved-house Pass 2 (`0x591af3`, `0x591bf3`) uses
    // byte-exact lookup, and an accessory whose bind missed does NOT count as
    // worn for portrait rendering even if registered later.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
        let skel = sims_content::Skeleton {
            name: "adult".into(),
            file: "adult.bcf".into(),
            place: 0,
            scales: vec![vec![1]],
            bones: Vec::new(),
        };
        fx.program
            .people
            .skeletons_by_name
            .insert("adult".into(), skel);
        fx.program
            .people
            .skeleton_names
            .insert(SIM_GUID, "adult".into());
        fx.program.people.eager_suits.insert("saved_head".into());
        fx.program.people.head_suits.insert("saved_head".into());
        let mut p = person();
        p.appearance[8] = "Saved_Head".into(); // mixed case must NOT match "saved_head"
        p.accessories.push(Accessory {
            name: "late_head_acc".into(),
            binding: String::new(),
        });
        let inst = sims_formats::saved_objects::Instance {
            id: 1,
            type_id: 0,
            guid: SIM_GUID,
            footprint: [0; 4],
            position: [0; 3],
            unknown_short: 0,
            attributes: Vec::new(),
            temps: vec![0; 8],
            data: vec![0; 72],
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: vec![sims_formats::saved_objects::Slot {
                unknown: 0,
                object: 0,
            }],
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: Some(p),
            registry_word: Some(0),
            consumed_bits: 0,
            record: Vec::new(),
        };
        let saved = sims_formats::saved_objects::Objects {
            version: 0x47,
            order: vec![1],
            types_by_id: BTreeMap::new(),
            instances: BTreeMap::from([(1, inst)]),
            tail: Vec::new(),
        };
        let mut world = sims_sim::World::empty();
        world.neighbors.slots.push(Some(Neighbor {
            guid: SIM_GUID,
            id: 1,
            data: vec![20; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 1,
            stale: false,
        }));
        fx.interpreter()
            .read_objects(&mut world, &saved, &fx.program.people)
            .expect("saved read");
        assert!(
            world.person_outfits[&1].head.is_none(),
            "0x591af3 must compare saved appearance names byte-exact"
        );
        assert!(
            world.person_outfits[&1].accessories.is_empty(),
            "0x591bf3 must not mark an unresolved saved accessory as bound"
        );
    }

    // 8. P2 #11: Raw row presence check before descriptor parsing (`0x58a733`):
    // non-empty row 30 `",BODY=x"` takes the row-30 branch (empty suit ->
    // `0x58d494` fallback to row 1), NOT falling back to row 27.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.program.people.eager_suits.insert("row1_body".into());
        fx.program.people.eager_suits.insert("row27_head".into());
        fx.program.people.head_suits.insert("row27_head".into());
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("row1_body,BODY=1".into()),
                head: None,
                age: Some("27".into()),
                suit_rows: BTreeMap::from([
                    (27, "row27_head,BODY=27".into()),
                    (30, ",BODY=x".into()), // raw row 30 is non-empty, parsed suit is ""
                ]),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 0;
            p.data[8] = 4; // formal
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.person_outfits[&SIM]
                .body
                .as_ref()
                .map(|b| b.suit.as_str()),
            Some("row1_body"),
            "non-empty raw row 30 must take row-30 branch and fall back to row 1 (0x58d494), not row 27"
        );
        assert!(!fx.world.portraits.contains(&SIM_GUID));
    }

    // 9. P3 #1: Shared Dialog portrait path (`dialog.rs:403`) labels
    // "a dialog's portrait" instead of "UI Effect's portrait".
    {
        let (mut fx, file, record) = visitors();
        // Two rooms so `data[40] = 0x400` marks a different room depending on
        // which tile the missing `view_centre` selects.
        fx.world.rooms.rooms.insert(2, Default::default());
        for a in 3..6 {
            for b in 1..5 {
                fx.world.rooms.map[0][a * 64 + b] = 2;
            }
        }
        fx.add(
            file,
            4096,
            tree(vec![
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
                assign(scope::MY_DATA, 40, scope::LITERAL, 0x400, RETURN_TRUE),
            ]),
        );
        fx.add(
            LAMP_FILE,
            4098,
            tree(vec![
                assign(scope::STACK_OBJECT, 0, scope::LITERAL, record, 1),
                ins(36, RETURN_TRUE, RETURN_FALSE, [0, 0, 0, 0, 0, 0, 0, 5]),
            ]),
        );
        let err = fx
            .run_with(LAMP, 4098, 0, &[], 100)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("a dialog's portrait") && !err.contains("UI Effect's portrait"),
            "expected dialog label on shared portrait diagnostic, got: {err}"
        );
    }
}

#[test]
fn round_20_global34_grown_saved_shared_scan_empty_retry_mnc_init_leading_g_workwear_case_and_cp1252()
 {
    use sims_content::{ImportWardrobe, Profile, SuitTable};
    use sims_formats::career::Level;
    use sims_formats::saved_objects::Accessory;

    // 1. P2 #1: Set To Next (`0x44eb9d`, ebp-0x18) and Expression (`0x452c85`,
    // ebp-0x04) overwrite `scratch_ptr_set` so UI Effect Global 34 traps even
    // after Random Number previously set a safe slot.
    {
        let (mut fx, _, record) = visitors();
        fx.world.globals[34] = record;
        fx.add(
            LAMP_FILE,
            4099,
            tree(vec![
                ins(8, 1, RETURN_FALSE, [0, 0, 8, 0, 1, 0, 7, 0]),
                ins(31, 2, 2, [0, 0, 0, 0, 0x83, 8, 0, 0]),
                ins(
                    34,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [1, scope::GLOBAL, 34, 0, 1, 0, 0, 0],
                ),
            ]),
        );
        assert!(
            fx.run_with(LAMP, 4099, 0, &[], 100).is_err(),
            "Set To Next must make subsequent UI Effect Global 34 trap"
        );
        assert!(!fx.world.scratch_ptr_set.contains(&LAMP));

        fx.add(
            LAMP_FILE,
            4100,
            tree(vec![
                assign(scope::TEMPS, 0, scope::LITERAL, 1, 1),
                ins(
                    34,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [1, scope::GLOBAL, 34, 0, 1, 0, 0, 0],
                ),
            ]),
        );
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        fx.program.people.portrait_head.insert(WIDGET, true);
        assert_eq!(
            fx.run_with(LAMP, 4100, 0, &[], 100),
            Ok(true),
            "Expression (`0x452c85`) targets saved EBP restored at `0x452273`, so Global 34 succeeds"
        );
        assert!(fx.world.portraits.contains(&WIDGET));
    }

    // 2. P2 #2: Saved-house loading skips saved appearances & accessories for
    // `GROWN` types (`0x4400ef` -> `0x45cf79` -> `0x591ae2`/`0x591bdc`),
    // opening no indexed `.bcf`, drawing no extra CRT RNG, and not dirtying portraits.
    {
        const GROWN_GUID: u32 = 0x3C08_965F;
        let (mut fx, _, _) = visitors();
        let grown_file = fx.add_type(GROWN_GUID, 2, 4096, 4097);
        fx.words(grown_file)[102] = 0;
        let skel = sims_content::Skeleton {
            name: "adult".into(),
            file: "adult.bcf".into(),
            place: 0,
            scales: vec![vec![1]],
            bones: Vec::new(),
        };
        let app_skel = sims_content::Skeleton {
            name: "lazy_s".into(),
            file: "grown_lazy.bcf".into(),
            place: 0,
            scales: vec![vec![1]],
            bones: Vec::new(),
        };
        fx.program
            .people
            .skeletons_by_name
            .insert("adult".into(), skel);
        fx.program
            .people
            .skeleton_names
            .insert(GROWN_GUID, "adult".into());
        fx.program
            .people
            .indexed_catalogs
            .insert("grown_lazy.bcf".into(), (vec![app_skel], None));
        fx.program.people.indexed_suit_catalogs.insert(
            "grown_lazy.bcf".into(),
            (vec![("grown_body".into(), true)], None),
        );
        fx.program
            .people
            .indexed_suits
            .insert("grown_body".into(), Ok("grown_lazy.bcf".into()));
        fx.program.people.eager_suits.insert("grown_head".into());
        fx.program.people.head_suits.insert("grown_head".into());
        let mut p = person();
        p.appearance[0] = "grown_body".into();
        p.appearance[8] = "grown_head".into();
        p.accessories.push(Accessory {
            name: "grown_head".into(),
            binding: String::new(),
        });
        let inst = sims_formats::saved_objects::Instance {
            id: 1,
            type_id: 0,
            guid: GROWN_GUID,
            footprint: [0; 4],
            position: [0; 3],
            unknown_short: 0,
            attributes: Vec::new(),
            temps: vec![0; 8],
            data: vec![0; 72],
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: vec![sims_formats::saved_objects::Slot {
                unknown: 0,
                object: 0,
            }],
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: Some(p),
            registry_word: Some(0),
            consumed_bits: 0,
            record: Vec::new(),
        };
        let saved = sims_formats::saved_objects::Objects {
            version: 0x47,
            order: vec![1],
            types_by_id: BTreeMap::new(),
            instances: BTreeMap::from([(1, inst)]),
            tail: Vec::new(),
        };
        let mut world = sims_sim::World::empty();
        world.neighbors.slots.push(Some(Neighbor {
            guid: GROWN_GUID,
            id: 1,
            data: vec![20; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 1,
            stale: false,
        }));
        let read = fx
            .interpreter()
            .read_objects(&mut world, &saved, &fx.program.people)
            .expect("saved read");
        assert_eq!(read.draws, 64);
        assert!(!world.loaded_indexed_suits.contains_key("grown_body"));
        assert!(world.person_outfits[&1].body.is_none());
        assert!(world.person_outfits[&1].head.is_none());
        assert!(world.person_outfits[&1].accessories.is_empty());
        // 0x591acc/0x591ad7 deserialize head strings before `GROWN` skips `0x583541`,
        // so `0x591b01..0x591b53` still dirties the portrait when head is non-empty!
        assert!(world.dirty_portraits.contains(&GROWN_GUID));
    }

    // 3. P2 #3 & #5: Make New Character shares the lazy `0x409255` suit scan
    // with generated dressing (seeing `.bcf` siblings opened before first scan)
    // AND runs `0x409315` (`0x44afc0`) before editing the clone to initialize
    // derived clothing rows (30..=34).
    {
        const TEMPLATE_PERSON: u32 = 0x7fd9_6b54;
        let (mut fx, _, _) = visitors();
        let tpl_file = fx.add_type(TEMPLATE_PERSON, 2, 4096, 4097);
        fx.world.neighbors.slots.push(Some(Neighbor {
            guid: TEMPLATE_PERSON,
            id: 2,
            data: vec![0; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 0,
            stale: false,
        }));
        fx.add(
            tpl_file,
            4096,
            tree(vec![assign(
                scope::MY_DATA,
                4,
                scope::LITERAL,
                1,
                RETURN_TRUE,
            )]),
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
        fx.program.people.profiles.insert(
            TEMPLATE_PERSON,
            Profile {
                body: Some("b001mafit_01,BODY=b001mafitlgt_01".into()),
                head: Some("c001ma_01,HEAD-HEAD=c001malgt_01".into()),
                gender: Some("male".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                tones: vec!["lgt".into()],
                ..Default::default()
            },
        );
        let mut table = SuitTable::default();
        for name in [
            "c001ma_01",
            "f100mafit_01",
            "s100mafit_01",
            "l100mafit_01",
            "w100mafit_01",
            "h501mafit_01",
        ] {
            table.insert_name(name.into());
        }
        fx.program.people.suit_table = table;
        fx.program.people.textures = vec![
            "b002mafitlgt_late".into(),
            "c001malgt_01".into(),
            "f100mafitlgt_01".into(),
            "s100mafitlgt_01".into(),
            "l100mafitlgt_01".into(),
            "w100mafitlgt_01".into(),
            "h501mafitlgt_01".into(),
        ];
        fx.program.ordered_wardrobe = Some(Ok(ImportWardrobe::from_ordered_names(
            vec![b"c001ma_01".to_vec()],
            vec![b"b002mafitlgt_late".to_vec(), b"c001malgt_01".to_vec()],
        )));
        fx.world.runtime_added_suits.push("b002mafit_late".into());
        fx.add(
            SIM_FILE,
            4098,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
                assign(scope::LOCALS, 1, scope::LITERAL, 27, 2),
                assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
                ins(19, 4, RETURN_FALSE, [0, 1, 2, 0, 0, 0, 0, 0]),
                assign(scope::GLOBAL, 36, scope::STACK_OBJECT, 0, RETURN_TRUE),
            ]),
        );
        assert_eq!(fx.run_with(SIM, 4098, 0, &[], 1000), Ok(true));
        let made_guid = fx.world.neighbors.get(fx.world.globals[36]).unwrap().guid;
        let rows = &fx.world.made_outfits[&made_guid];
        assert!(
            rows.get(&1)
                .is_some_and(|s| s.starts_with("b002mafit_late,")),
            "Make New Character must share the live 0x409255 scan including runtime_added_suits, got: {rows:?}"
        );
        assert!(
            rows.get(&30)
                .is_some_and(|s| s.starts_with("f100mafit_01,BODY=f100mafitlgt_01")),
            "0x44afc0 -> 0x409315 must initialize derived formal row 30 on clone, got: {rows:?}"
        );
        assert!(rows.get(&33).is_some_and(|s| s.starts_with("w100mafit_01")));
    }

    // 4. P2 #4: Empty suit scan (only filtered `suit-` entries) is NOT cached
    // permanently (`0x40922f`), so a later `.bcf` load adding a real costume is found.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        let mut table = SuitTable::default();
        table.insert_name(b"suit-filtered".to_vec());
        fx.program.people.suit_table = table;
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("b001fafit_01,BODY=b".into()),
                head: None,
                gender: Some("female".into()),
                age: Some("27".into()),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 1;
            p.data[58] = 27;
            p.data[8] = 39;
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            fx.world.suit_scan_cache.is_none(),
            "0x40922f must not cache an empty scan permanently"
        );
        fx.world
            .runtime_added_suits
            .push("magicfafit_bluegenie".into());
        fx.program
            .people
            .eager_suits
            .insert("magicfafit_bluegenie".into());
        fx.program
            .people
            .head_suits
            .insert("magicfafit_bluegenie".into());
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.person_outfits[&SIM]
                .body
                .as_ref()
                .map(|b| b.suit.as_str()),
            Some("magicfafit_bluegenie")
        );
        assert!(fx.world.portraits.contains(&SIM_GUID));
    }

    // 5. P2 #6 & #7 & P3 #1..#4:
    // - Leading `$g` (`$gwork`) rejects `uwork` on male profile (`0x4726c5 -> 0x4727f1`);
    // - Workwear literal secondary suit (`CARR strings[4]`, `"HEAD_HAT"`) is byte-exact;
    // - Empty texture template matches `base_tex[17..]` (`"BODY=fresh"`);
    // - Generated body texture descriptor is capped at 63 bytes (`0x40e760`);
    // - `parse_row_descriptor` caps raw row at 255 bytes (`0x5874fa`);
    // - Windows-1252 scoring picks `f100mafit_€` (11 bytes) over `f100mafit_ab` (12 bytes).
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        let mut table = SuitTable::default();
        for name in ["uwork", "tuwork", "f100mafit_\u{20ac}", "f100mafit_ab"] {
            table.insert_name(sims_formats::far::encode_name(name).unwrap());
        }
        fx.program.people.suit_table = table;
        for name in ["uwork", "tuwork", "head_hat"] {
            fx.program.people.eager_suits.insert(name.into());
            fx.program.people.head_suits.insert(name.into());
        }
        let long_tex = format!("t{}", "x".repeat(62));
        fx.program.people.textures = vec!["fresh".into(), long_tex];
        fx.program.careers = Some(vec![sims_vm::Career {
            id: 1,
            levels: vec![
                Level {
                    values: [0; 21],
                    strings: [
                        "L0".into(),
                        "$gwork".into(),
                        String::new(),
                        String::new(),
                        "HEAD_HAT".into(),
                    ],
                },
                Level {
                    values: [0; 21],
                    strings: [
                        "L1".into(),
                        "t$gwork".into(),
                        String::new(),
                        "t".into(),
                        String::new(),
                    ],
                },
            ],
        }]);
        let long_row1 = format!("plain,BODY={}", "x".repeat(300));
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("b001mafit_01,BODY=b001mafitlgtfresh".into()),
                head: None,
                gender: Some("male".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                suit_rows: BTreeMap::from([(15, long_row1)]),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 0;
            p.data[58] = 27;
            p.data[56] = 1;
            p.data[57] = 0;
            p.data[8] = 3;
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            !fx.world.portraits.contains(&SIM_GUID),
            "leading $gwork must reject uwork and HEAD_HAT must not fold to head_hat"
        );
        assert!(fx.world.person_outfits[&SIM].body_alt.is_none());
        assert_eq!(
            fx.world.person_outfits[&SIM]
                .workwear_cache
                .as_ref()
                .map(|c| c.body_tex.as_str()),
            Some("BODY=fresh"),
            "0x40e6dc must still match base-texture suffix when template is empty"
        );
        assert!(
            fx.world.made_outfits[&SIM_GUID]
                .get(&30)
                .is_some_and(|s| s.starts_with("f100mafit_\u{20ac},BODY=")),
            "0x472a25 must score by Windows-1252 byte length, got: {:?}",
            fx.world.made_outfits[&SIM_GUID].get(&30)
        );

        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[57] = 1;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        let body = fx.world.person_outfits[&SIM].body.as_ref().unwrap();
        assert_eq!(body.suit, "tuwork");
        assert_eq!(body.texture.len(), 63);
        assert!(body.texture.starts_with("BODY=tx"));
        assert!(fx.world.portraits.contains(&SIM_GUID));

        fx.program.people.eager_suits.insert("plain".into());
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 1;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        let body = fx.world.person_outfits[&SIM].body.as_ref().unwrap();
        assert_eq!(body.suit, "plain");
        assert_eq!(body.texture.len(), 249);
    }
}

#[test]
fn round_21_old_rel_mnc_commit_empty_suit_freeze_pet_0x1f_and_p3_limits() {
    use sims_content::{ImportWardrobe, Profile, SuitTable};
    use sims_formats::career::Level;

    // 1. P2 #2: Old Relationship (opcode 24, `0x44d593`) does NOT populate
    // `[object + 0x128]`, so subsequent UI Effect Global 34 still traps.
    {
        let (mut fx, _, record) = visitors();
        fx.world.globals[34] = record;
        // Opcode 24 (Old Relationship): read (0), col 0, param 0, dir 0, flags 0 -> UI Effect Global 34
        fx.add(
            LAMP_FILE,
            4099,
            tree(vec![
                ins(24, 1, 1, [0, 0, 0, 0, 0, 0, 0, 0]),
                ins(
                    34,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [1, scope::GLOBAL, 34, 0, 1, 0, 0, 0],
                ),
            ]),
        );
        assert!(
            fx.run_with(LAMP, 4099, 0, &[0], 100).is_err(),
            "Old Relationship (opcode 24) must leave scratch_ptr_set unset so Global 34 traps"
        );
        assert!(!fx.world.scratch_ptr_set.contains(&LAMP));
    }

    // 2. P2 #4: Make New Character final commit (`0x44b185` -> `0x40a588` ->
    // `0x40af5b`) regenerates rows 30..=34 and rows 15, 16, 27 for the clone's
    // NEW gender/age/skin rather than keeping the template's original male rows.
    {
        const TEMPLATE_PERSON: u32 = 0x7fd9_6b54;
        let (mut fx, _, _) = visitors();
        let tpl_file = fx.add_type(TEMPLATE_PERSON, 2, 4096, 4097);
        fx.world.neighbors.slots.push(Some(Neighbor {
            guid: TEMPLATE_PERSON,
            id: 2,
            data: vec![0; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 0,
            stale: false,
        }));
        fx.add(
            tpl_file,
            4096,
            tree(vec![assign(
                scope::MY_DATA,
                4,
                scope::LITERAL,
                1,
                RETURN_TRUE,
            )]),
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
        fx.program.people.profiles.insert(
            TEMPLATE_PERSON,
            Profile {
                body: Some("b001mafit_01,BODY=b001mafitlgt_01".into()),
                head: Some("c001ma_01,HEAD-HEAD=c001malgt_01".into()),
                gender: Some("male".into()), // template is male!
                age: Some("27".into()),
                skin: Some("lgt".into()),
                tones: vec!["lgt".into()],
                ..Default::default()
            },
        );
        let mut table = SuitTable::default();
        for name in [
            "b001fafit_01",
            "c001fa_01",
            "f100mafit_01",
            "s100mafit_01",
            "l100mafit_01",
            "w100mafit_01",
            "h501mafit_01",
            "f100fafit_01",
            "s100fafit_01",
            "l100fafit_01",
            "w100fafit_01",
            "h501fafit_01",
            "nffit_01",
            "fffit_01",
        ] {
            table.insert_name(name.into());
        }
        fx.program.people.suit_table = table;
        fx.program.people.textures = vec![
            "b001fafitlgt_01".into(),
            "c001falgt_01".into(),
            "f100mafitlgt_01".into(),
            "s100mafitlgt_01".into(),
            "l100mafitlgt_01".into(),
            "w100mafitlgt_01".into(),
            "h501mafitlgt_01".into(),
            "f100fafitlgt_01".into(),
            "s100fafitlgt_01".into(),
            "l100fafitlgt_01".into(),
            "w100fafitlgt_01".into(),
            "h501fafitlgt_01".into(),
            "nffitlgt_01".into(),
            "uffitlgt_01".into(),
            "fffitlgt_01".into(),
        ];
        fx.program.ordered_wardrobe = Some(Ok(ImportWardrobe::from_ordered_names(
            vec![],
            fx.program
                .people
                .textures
                .iter()
                .map(|s| s.as_bytes().to_vec())
                .collect(),
        )));
        // Create FEMALE adult (sex=1, age=27, skin=0) from MALE template:
        fx.add(
            SIM_FILE,
            4098,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
                assign(scope::LOCALS, 1, scope::LITERAL, 27, 2),
                assign(scope::LOCALS, 2, scope::LITERAL, 1, 3),
                ins(19, 4, RETURN_FALSE, [0, 1, 2, 0, 0, 0, 0, 0]),
                assign(scope::GLOBAL, 36, scope::STACK_OBJECT, 0, RETURN_TRUE),
            ]),
        );
        assert_eq!(fx.run_with(SIM, 4098, 0, &[], 1000), Ok(true));
        let made_guid = fx.world.neighbors.get(fx.world.globals[36]).unwrap().guid;
        let rows = &fx.world.made_outfits[&made_guid];
        assert!(
            rows.get(&30)
                .is_some_and(|s| s.starts_with("f100fafit_01,BODY=f100fafitlgt_01")),
            "0x40a588 must regenerate formal row 30 for new female clone, got: {rows:?}"
        );
        assert!(
            rows.get(&33)
                .is_some_and(|s| s.starts_with("w100fafit_01,BODY=w100fafitlgt_01")),
            "0x40a588 must regenerate winter row 33 for new female clone, got: {rows:?}"
        );
        assert!(
            rows.get(&15)
                .is_some_and(|s| s.starts_with("nffit_01,BODY=nffitlgt_01")),
            "0x40af5b must rebuild naked row 15 for new female clone, got: {rows:?}"
        );
    }

    // 3. P2 #5: Empty workwear suit template still invokes `0x40922f` at `0x40e60e`,
    // freezing `world.suit_scan_cache` so later `.bcf` registrations are excluded.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        let mut table = SuitTable::default();
        table.insert_name(b"existing_suit".to_vec());
        fx.program.people.suit_table = table;
        fx.program.careers = Some(vec![sims_vm::Career {
            id: 1,
            levels: vec![Level {
                values: [0; 21],
                strings: [
                    "L0".into(),
                    String::new(), // empty workwear suit template!
                    String::new(),
                    String::new(),
                    String::new(),
                ],
            }],
        }]);
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: None, // empty row 1 so init_clothing_rows returns early
                head: None,
                gender: Some("male".into()),
                age: Some("27".into()),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 0;
            p.data[58] = 27;
            p.data[56] = 1;
            p.data[57] = 0;
            p.data[8] = 3;
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            fx.world.suit_scan_cache.is_some(),
            "0x40e60e must call 0x40922f and freeze suit_scan_cache even on empty template"
        );
    }

    // 4. P2 #6: Pet creation (`0x44b126` -> `0x4e8c5d(index, 0x1f, 1)`) renders
    // bit 0x01 (persisting BMP_ 2002), then runs additional forced dresses that
    // clear `world.portraits` via `0x58d4e1 -> 0x4eb0cc -> 0x4e897e` when the
    // pet's head slot is empty (body-mounted HEAD mesh).
    {
        const TEMPLATE_DOG: u32 = 0x4a70_df92;
        let (mut fx, _, _) = visitors();
        fx.world.rooms.rooms.get_mut(&1).unwrap().occupied = true;
        fx.program.expansions = 0x20;
        let pet_file = fx.add_type(TEMPLATE_DOG, 2, 4096, 4097);
        fx.world.neighbors.slots.push(Some(Neighbor {
            guid: TEMPLATE_DOG,
            id: 2,
            data: vec![0; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 0,
            stale: false,
        }));
        fx.add(
            pet_file,
            4096,
            tree(vec![
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, RETURN_TRUE),
            ]),
        );
        fx.add(
            pet_file,
            4097,
            tree(vec![
                assign(scope::PARAMETERS, 0, scope::LITERAL, 10, 1),
                ins(0, 0, 0, [0; 8]),
            ]),
        );
        equip(&mut fx, TEMPLATE_DOG, "dog", 12);
        fx.program
            .people
            .profiles
            .get_mut(&TEMPLATE_DOG)
            .unwrap()
            .tones = vec!["lgt".into()];
        fx.program.people.eager_suits.insert("b001dog_01".into());
        fx.program.people.head_suits.insert("b001dog_01".into());
        fx.program.wardrobe = Some(Wardrobe {
            suits: ["b001dog_01".into()].into(),
            textures: ["b001dog_01.bmp".into()].into(),
            errors: BTreeMap::new(),
        });
        fx.add(
            SIM_FILE,
            4098,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
                assign(scope::LOCALS, 1, scope::LITERAL, 27, 2),
                assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
                ins(19, 4, RETURN_FALSE, [0, 1, 2, 1, 0, 0, 0, 0]),
                assign(scope::GLOBAL, 36, scope::STACK_OBJECT, 0, RETURN_TRUE),
            ]),
        );
        assert_eq!(fx.run_with(SIM, 4098, 0, &[], 1000), Ok(true));
        let pet_guid = fx.world.neighbors.get(fx.world.globals[36]).unwrap().guid;
        let pet_file_num = fx.world.characters.made[&pet_guid].file;
        assert!(
            fx.world
                .generated_portrait_2002
                .contains(&sims_sim::PortraitFile::Made(pet_file_num)),
            "bit 0x01 must still persist BMP_ 2002 for body-mounted HEAD pet"
        );
        assert!(
            !fx.world.portraits.contains(&pet_guid),
            "later passes in 0x4e8c5d(index, 0x1f, 1) must clear cache via 0x58d4e1 -> 0x4e897e when head slot is empty"
        );
    }

    // 5. P3 #2, #3, #4, #5, #6, #7, #8:
    // - 63-byte base_tex cap before `[17..]` suffix (`0x472ed2`);
    // - 255-byte head texture destination (`0x58c7c9`);
    // - 63-byte CARR secondary suit cap (`0x45ef75`);
    // - Windows-1252 byte offset `6..9` for body_type (`b00émafat` -> `fat`);
    // - Windows-1252 bytes `0x81/0x8d/0x8f/0x90/0x9d` stay 1 byte;
    // - 15-byte age buffer cap (`"               18"` -> child in `0x40cbdd`);
    // - Decoded zero-bone skeleton (`scales: vec![vec![]], bones: vec![]`) returns false!
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        let long_sec = format!("{}_tail", "s".repeat(63));
        let sec_63 = "s".repeat(63);
        let long_head_tex = format!("cmagicmafat_donkey_{}", "y".repeat(44)); // 63 bytes -> HEAD-HEAD= + 63 = 73 bytes
        let mut table = SuitTable::default();
        for name in [
            "cmagicmafat_donkey",
            "f100mafat_01",
            "s100mcchd_01",
            "w100mcchd_01",
            "mwork_fat",
            "mwork_\u{0081}",
        ] {
            table.insert_name(name.chars().map(|c| c as u8).collect::<Vec<u8>>());
        }
        fx.program.people.suit_table = table;
        fx.program.people.eager_suits.insert(sec_63.clone());
        fx.program.people.eager_suits.insert("mwork_fat".into());
        fx.program
            .people
            .eager_suits
            .insert("cmagicmafat_donkey".into());
        fx.program
            .people
            .head_suits
            .insert("cmagicmafat_donkey".into());
        fx.program.people.textures = vec![
            format!("t{}", "z".repeat(46)), // matches 63-byte capped suffix (46 bytes)
            long_head_tex.clone(),
        ];
        fx.program.careers = Some(vec![sims_vm::Career {
            id: 1,
            levels: vec![Level {
                values: [0; 21],
                strings: [
                    "L0".into(),
                    "mwork_$b".into(),
                    String::new(),
                    "t".into(),
                    long_sec,
                ],
            }],
        }]);
        // Body suit `b00émafat`: UTF-8 offset 6..9 would be `afa`, cp1252 offset 6..9 is `fat`!
        // Age `"               18"` (15 spaces + "18"): 15-byte cap at `0x40cbdd` -> child (`"ADDED"` on row 30)!
        // Base tex: 17-byte prefix + 46 `'z'` + `"extra"`: 63-byte cap drops `"extra"` so `t` + 46 `'z'` matches!
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some(format!(
                    "b00\u{00e9}mafat,BODY=b001mafatlgt{}extra",
                    "z".repeat(46)
                )),
                head: None,
                gender: Some("male".into()),
                age: Some("               18".into()),
                skin: Some("lgt".into()),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 0;
            p.data[58] = 27;
            p.data[56] = 1;
            p.data[57] = 0;
            p.data[8] = 3; // workwear
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        let outfit = &fx.world.person_outfits[&SIM];
        assert_eq!(
            outfit.body.as_ref().map(|b| b.suit.as_str()),
            Some("mwork_fat"),
            "cp1252 offset 6..9 of b00émafat must be fat"
        );
        assert_eq!(
            outfit.body_alt.as_ref().map(|b| b.suit.as_str()),
            Some(sec_63.as_str()),
            "0x45ef75 must cap CARR secondary suit at 63 bytes"
        );
        assert_eq!(
            outfit.body.as_ref().map(|b| b.texture.as_str()),
            Some(format!("BODY=t{}", "z".repeat(46)).as_str()),
            "0x472ed2 must cap base_tex to 63 bytes before slicing [17..]"
        );
        assert_eq!(
            fx.world.made_outfits[&SIM_GUID]
                .get(&30)
                .map(String::as_str),
            Some("ADDED"),
            "0x40cbdd 15-byte age cap must treat 15 spaces + 18 as child"
        );

        // Suit 57 (Donkey Head) -> head texture keeps full 73-byte `HEAD-HEAD=` + 63-byte stem (`0x58c7c9`).
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 57;
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.person_outfits[&SIM]
                .head
                .as_ref()
                .map(|b| b.texture.len()),
            Some("HEAD-HEAD=".len() + long_head_tex.len()),
            "0x58c7c9 head texture must allow up to 255 bytes (not 63)"
        );
        assert!(fx.world.portraits.contains(&SIM_GUID));

        // Decoded zero-bone skeleton (`scales: vec![vec![]], bones: vec![]`) must NOT render portrait!
        fx.world.portraits.remove(&SIM_GUID);
        fx.world.generated_portrait_2002.clear();
        fx.program.people.skeletons.insert(
            SIM_GUID,
            sims_content::Skeleton {
                name: "empty_rig".into(),
                file: "empty.bcf".into(),
                place: 0,
                scales: vec![vec![]],
                bones: Vec::new(),
            },
        );
        fx.program
            .people
            .skeleton_names
            .insert(SIM_GUID, "empty_rig".into());
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert!(
            !fx.world.portraits.contains(&SIM_GUID),
            "decoded zero-bone skeleton has no HEAD bone and must fail portrait render"
        );
    }
}

#[test]
fn round_22_clothing_commit_pet_fallback_placed_callback_and_global_34() {
    use sims_content::{Profile, SuitTable};

    let add_template = |fx: &mut Fixture, guid: u32, skel: &str| {
        let tpl_file = fx.add_type(guid, 2, 4096, 4097);
        let id = fx.world.neighbors.slots.len() as i16 + 1;
        fx.world.neighbors.slots.push(Some(Neighbor {
            guid,
            id,
            data: vec![0; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 0,
            stale: false,
        }));
        fx.add(
            tpl_file,
            4096,
            tree(vec![assign(
                scope::MY_DATA,
                4,
                scope::LITERAL,
                1,
                RETURN_TRUE,
            )]),
        );
        fx.add(
            tpl_file,
            4097,
            tree(vec![
                assign(scope::PARAMETERS, 0, scope::LITERAL, 10, 1),
                ins(0, 0, 0, [0; 8]),
            ]),
        );
        equip(fx, guid, skel, 12);
    };

    let run_make_char = |fx: &mut Fixture,
                         caller: u16,
                         file: usize,
                         skin: i16,
                         age: i16,
                         sex: i16,
                         template: u8| {
        let mut t = tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, skin, 1),
            assign(scope::LOCALS, 1, scope::LITERAL, age, 2),
            assign(scope::LOCALS, 2, scope::LITERAL, sex, 3),
            ins(
                19,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 1, 2, template, 0, 0, 0, 0],
            ),
        ]);
        t.locals = 3;
        fx.add(file, 4096, t);
        fx.run_with(caller, 4096, 0, &[], 1000)
    };

    // 1. P2 #1: Child clothing fallback checks formalwear list too (`0x40acd4..0x40acf9`),
    // clearing rows 30..=32 to `""` when formalwear is empty (as on the stock install).
    {
        let (mut fx, _, _) = visitors();
        const TEMPLATE_PERSON: u32 = 0x7FD9_6B54;
        add_template(&mut fx, TEMPLATE_PERSON, "child");
        fx.program.people.profiles.insert(
            TEMPLATE_PERSON,
            Profile {
                body: Some("b001mcchd_01,BODY=b001mcchdlgt_01".into()),
                head: Some("c001mc_01,HEAD-HEAD=c001mclgt_01".into()),
                gender: Some("male".into()),
                age: Some("9".into()),
                skin: Some("lgt".into()),
                tones: vec!["lgt".into()],
                suit_rows: BTreeMap::from([
                    // Row 30 already populated so `0x409315` doesn't regenerate row 32;
                    // row 32 missing -> triggers `0x40acd2` three-list check!
                    (30, "ADDED".into()),
                    (31, "s100mcchd_01,BODY=s100mcchdlgt_01".into()),
                ]),
                ..Default::default()
            },
        );
        let mut table = SuitTable::default();
        for name in [
            "b001mcchd_01",
            "c001mc_01",
            "s100mcchd_01",
            "l100mcchd_01",
            // Intentionally NO child formalwear `f...mc...`!
        ] {
            table.insert_name(name.into());
        }
        fx.program.people.suit_table = table;
        fx.program.people.textures = vec![
            "b001mcchdlgt_01".into(),
            "c001mclgt_01".into(),
            "s100mcchdlgt_01".into(),
            "l100mcchdlgt_01".into(),
        ];
        fx.program.wardrobe = Some(Wardrobe {
            suits: vec!["b001mcchd_01".into(), "c001mc_01".into()]
                .into_iter()
                .collect(),
            textures: vec!["b001mcchdlgt_01.bmp".into(), "c001mclgt_01.bmp".into()]
                .into_iter()
                .collect(),
            errors: Default::default(),
        });
        assert_eq!(
            run_make_char(&mut fx, LAMP, LAMP_FILE, 0, 9, 0, 0),
            Ok(true)
        );
        let made_guid = *fx.world.characters.made.keys().next().unwrap();
        let outfits = &fx.world.made_outfits[&made_guid];
        assert_eq!(outfits.get(&30).map(String::as_str), Some(""));
        assert_eq!(outfits.get(&31).map(String::as_str), Some(""));
        assert_eq!(outfits.get(&32).map(String::as_str), Some(""));
    }

    // 2. P2 #2 + P2 #5 + P3 #1: Body-type validation (`0x40a757..0x40ab7b`),
    // hand-texture miss writing `""` (`0x40b5e3..0x40b657`), and separate 63-byte
    // suit/texture caps in `build_list`/`find_in_list` (`0x472e7e..0x472f41`).
    {
        let (mut fx, _, _) = visitors();
        const TEMPLATE_PERSON: u32 = 0x7FD9_6B54;
        add_template(&mut fx, TEMPLATE_PERSON, "adult");
        // 40-byte suit + 40-byte `BODY=...` -> 81-byte valid formal row (exceeds 63 bytes total!).
        let long_formal_suit = format!("f100mafat_{}", "a".repeat(30));
        let long_formal_tex = format!("f100mafatlgt_{}", "b".repeat(22));
        let long_formal_row = format!("{long_formal_suit},BODY={long_formal_tex}");
        assert!(long_formal_row.len() > 63);
        fx.program.people.profiles.insert(
            TEMPLATE_PERSON,
            Profile {
                body: Some("b001mafat_01,BODY=b001mafatlgt_01".into()),
                head: Some("c001ma_01,HEAD-HEAD=c001malgt_01".into()),
                gender: Some("male".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                tones: vec!["lgt".into()],
                suit_rows: BTreeMap::from([
                    (30, long_formal_row.clone()), // matches fat body -> MUST be kept intact (> 63 bytes)!
                    (31, "s100mafit_01,BODY=s100mafitlgt_01".into()), // fit != fat -> MUST be invalidated!
                    (32, "l100mafit_01,BODY=l100mafitlgt_01".into()), // fit != fat -> MUST be invalidated!
                    (33, "w100mafit_01,BODY=w100mafitlgt_01".into()), // fit != fat -> MUST be invalidated!
                    (34, "h500mafit_01,BODY=h500mafitlgt_01".into()), // fit != fat -> MUST be invalidated!
                    (17, "HMLO,HAND=stale_hand_tex".into()), // hand miss -> MUST be cleared to ""!
                ]),
                ..Default::default()
            },
        );
        let mut table = SuitTable::default();
        for name in [
            "b001mafat_01",
            "c001ma_01",
            long_formal_suit.as_str(),
            "s100mafit_01",
            "s100mafat_01",
            "l100mafit_01",
            "l100mafat_01",
            "w100mafit_01",
            "w100mafat_01",
            "h500mafit_01",
            "h500mafat_01",
        ] {
            table.insert_name(name.into());
        }
        fx.program.people.suit_table = table;
        fx.program.people.textures = vec![
            "b001mafatlgt_01".into(),
            "c001malgt_01".into(),
            long_formal_tex.clone(),
            "s100mafitlgt_01".into(),
            "s100mafatlgt_01".into(),
            "l100mafitlgt_01".into(),
            "l100mafatlgt_01".into(),
            "w100mafitlgt_01".into(),
            "w100mafatlgt_01".into(),
            "h500mafitlgt_01".into(),
            "h500mafatlgt_01".into(),
            // Intentionally NO hand textures (`H...`)!
        ];
        fx.program.wardrobe = Some(Wardrobe {
            suits: vec!["b001mafat_01".into(), "c001ma_01".into()]
                .into_iter()
                .collect(),
            textures: vec!["b001mafatlgt_01.bmp".into(), "c001malgt_01.bmp".into()]
                .into_iter()
                .collect(),
            errors: Default::default(),
        });
        assert_eq!(
            run_make_char(&mut fx, LAMP, LAMP_FILE, 0, 27, 0, 0),
            Ok(true)
        );
        let made_guid = *fx.world.characters.made.keys().next().unwrap();
        let outfits = &fx.world.made_outfits[&made_guid];
        // Row 30 matched fat & survived > 63 bytes without truncation!
        assert_eq!(
            outfits.get(&30).map(String::as_str),
            Some(long_formal_row.as_str())
        );
        // Winter (33) and hf (34) mismatched fat -> regenerated with `fat`!
        assert_eq!(
            outfits.get(&33).map(String::as_str),
            Some("w100mafat_01,BODY=w100mafatlgt_01"),
            "mismatched fit winter row must be invalidated and regenerated for fat"
        );
        assert_eq!(
            outfits.get(&34).map(String::as_str),
            Some("h500mafat_01,BODY=h500mafatlgt_01"),
            "mismatched fit high-fashion row must be invalidated and regenerated for fat"
        );
        // Hand miss cleared inherited stale row 17 to ""!
        assert_eq!(
            outfits.get(&17).map(String::as_str),
            Some(""),
            "0x40b5e3..0x40b657 must clear stale hand row on texture miss"
        );
    }

    // 3. P2 #3: Invalid pet picker index rechecks inherited body (`0x40a5a1`);
    // on a miss, `0x40ac57..0x40acac` clears rows 1, 2, 15..=18 and does NOT render BMP_ 2002.
    {
        let (mut fx, _, _) = visitors();
        const TEMPLATE_DOG: u32 = 0x4A70_DF92;
        add_template(&mut fx, TEMPLATE_DOG, "dog");
        fx.program.people.profiles.insert(
            TEMPLATE_DOG,
            Profile {
                body: Some("unlisted_dog_suit,BODY=unlisted_dog_tex".into()),
                gender: Some("dog".into()),
                tones: vec!["lgt".into()],
                ..Default::default()
            },
        );
        fx.program
            .people
            .eager_suits
            .insert("unlisted_dog_suit".into());
        fx.program
            .people
            .head_suits
            .insert("unlisted_dog_suit".into());
        fx.program.people.eager_suits.insert("b001dog_01".into());
        fx.program.people.head_suits.insert("b001dog_01".into());
        fx.program.wardrobe = Some(Wardrobe {
            suits: vec!["b001dog_01".into()].into_iter().collect(),
            textures: vec!["b001dog_01.bmp".into()].into_iter().collect(),
            errors: Default::default(),
        });
        // Set out-of-range pet picker index (99 >= 1).
        fx.world.dialogs.declined = 99;
        assert_eq!(
            run_make_char(&mut fx, LAMP, LAMP_FILE, 0, 27, 0, 1),
            Ok(true)
        );
        let made_guid = *fx.world.characters.made.keys().next().unwrap();
        let outfits = &fx.world.made_outfits[&made_guid];
        for row in [1usize, 2, 15, 16, 17, 18] {
            assert_eq!(
                outfits.get(&row).map(String::as_str),
                Some(""),
                "pet body miss must clear row {row} to empty string"
            );
        }
        assert_eq!(outfits.get(&30).map(String::as_str), Some("NONE"));
        assert!(
            !fx.world.portraits.contains(&made_guid),
            "unlisted inherited pet body must NOT render or cache portrait"
        );
        assert!(
            fx.world.generated_portrait_2002.is_empty(),
            "unlisted inherited pet body must NOT write BMP_ 2002"
        );
    }

    // 4. P2 #4: Negative-age body fallback invalidates inherited clothing rows
    // (`0x40abf9..0x40ac34`) so `0x40b7ed` regenerates them instead of keeping
    // inherited clothing that happened to match the list.
    {
        let (mut fx, _, _) = visitors();
        const TEMPLATE_PERSON: u32 = 0x7FD9_6B54;
        add_template(&mut fx, TEMPLATE_PERSON, "adult");
        fx.program.people.profiles.insert(
            TEMPLATE_PERSON,
            Profile {
                body: Some("b001mafit_01,BODY=b001mafitlgt_01".into()),
                head: Some("c001ma_01,HEAD-HEAD=c001malgt_01".into()),
                gender: Some("male".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                tones: vec!["lgt".into()],
                suit_rows: BTreeMap::from([
                    // Listed in formal_list (`f200mafit_01`), NOT the `f100` generated default!
                    (30, "f200mafit_01,BODY=f200mafitlgt_01".into()),
                    (31, "s100mafit_01,BODY=s100mafitlgt_01".into()),
                    (32, "l100mafit_01,BODY=l100mafitlgt_01".into()),
                ]),
                ..Default::default()
            },
        );
        // Running person (SIM) has an UNLISTED body so negative-age copy falls back to index 0!
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("b999mafit_unlisted,BODY=b999mafitlgt_unlisted".into()),
                head: Some("c001ma_01,HEAD-HEAD=c001malgt_01".into()),
                gender: Some("male".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                ..Default::default()
            },
        );
        let mut table = SuitTable::default();
        for name in [
            "b001mafit_01",
            "c001ma_01",
            "f200mafit_01",
            "f100mafit_01",
            "s100mafit_01",
            "l100mafit_01",
        ] {
            table.insert_name(name.into());
        }
        fx.program.people.suit_table = table;
        fx.program.people.textures = vec![
            "b001mafitlgt_01".into(),
            "c001malgt_01".into(),
            "f200mafitlgt_01".into(),
            "f100mafitlgt_01".into(),
            "s100mafitlgt_01".into(),
            "l100mafitlgt_01".into(),
        ];
        fx.program.wardrobe = Some(Wardrobe {
            suits: vec!["b001mafit_01".into(), "c001ma_01".into()]
                .into_iter()
                .collect(),
            textures: vec!["b001mafitlgt_01.bmp".into(), "c001malgt_01.bmp".into()]
                .into_iter()
                .collect(),
            errors: Default::default(),
        });
        assert_eq!(run_make_char(&mut fx, SIM, SIM_FILE, 0, -1, 0, 0), Ok(true));
        let made_guid = *fx.world.characters.made.keys().next().unwrap();
        let outfits = &fx.world.made_outfits[&made_guid];
        // Because body fell back to index 0, `f_idx/s_idx/l_idx` were all invalidated -> `f100` regenerated!
        assert_eq!(
            outfits.get(&30).map(String::as_str),
            Some("f100mafit_01,BODY=f100mafitlgt_01"),
            "negative-age body fallback must invalidate inherited formal row and regenerate f100"
        );
    }

    // 5. P2 #6: Valid off-lot portrait placement executes `function::PLACED` (function 9)
    // instead of trapping (`0x4e96fe` -> `0x433580`), both with explicit `view_centre` and
    // when invariant across all reachable tiles with `view_centre == None`.
    {
        for centre in [Some([2i8, 2i8]), None] {
            let (mut fx, file, record) = visitors();
            fx.add(
                file,
                4200,
                tree(vec![assign(
                    scope::GLOBAL,
                    36,
                    scope::LITERAL,
                    77,
                    RETURN_TRUE,
                )]),
            );
            let library = fx.program.libraries.get_mut(&file).unwrap();
            for table in library.callbacks.values_mut() {
                table.resize(
                    27,
                    sims_formats::execution::ObjectFunction {
                        condition: 0,
                        action: 0,
                    },
                );
                table[9].action = 4200; // function::PLACED
            }
            let [lo, hi] = record.to_le_bytes();
            fx.add(
                LAMP_FILE,
                4096,
                tree(vec![ins(
                    UI_EFFECT,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [1, scope::LITERAL, lo, hi, 1, 0, 0, 0],
                )]),
            );
            let mut interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
            interpreter.view_centre = centre;
            assert_eq!(
                interpreter.run_tree(&mut fx.world, LAMP, 4096, 0, &[], 1000),
                Ok(true),
                "UI Effect with Placed callback must succeed for view_centre={centre:?}"
            );
            assert_eq!(fx.world.globals[36], 77, "Placed callback must have run");
            assert!(fx.world.portraits.contains(&WIDGET));
        }
    }

    // 6. P3 #2: Short 7-byte body-type suffix (`b100mas` -> `"s"`, `0x40e59f -> 0x46f775`).
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
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
        fx.program.people.suit_table.insert_name("mwork_s".into());
        fx.program.people.eager_suits.insert("mwork_s".into());
        fx.program.people.head_suits.insert("mwork_s".into());
        fx.program.careers = Some(vec![sims_vm::Career {
            id: 1,
            levels: vec![sims_formats::career::Level {
                values: [0; 21],
                strings: [
                    "L0".into(),
                    "mwork_$b".into(),
                    String::new(),
                    String::new(),
                    String::new(),
                ],
            }],
        }]);
        fx.program.people.profiles.insert(
            SIM_GUID,
            Profile {
                body: Some("b100mas,BODY=b100maslgt".into()),
                gender: Some("male".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                ..Default::default()
            },
        );
        {
            let p = fx
                .world
                .objects
                .get_mut(&SIM)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            p.data[65] = 0;
            p.data[58] = 27;
            p.data[56] = 1;
            p.data[57] = 0;
            p.data[8] = 3; // workwear -> expands `mwork_$b` to `mwork_s`
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_record, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.person_outfits[&SIM]
                .body
                .as_ref()
                .map(|b| b.suit.as_str()),
            Some("mwork_s"),
            "7-byte suit b100mas must supply 's' for $b"
        );
    }

    // 7. P3 #3: Fallback `Wardrobe` in Make New Character freezes `suit_scan_cache`.
    {
        let (mut fx, _, _) = visitors();
        const TEMPLATE_PERSON: u32 = 0x7FD9_6B54;
        add_template(&mut fx, TEMPLATE_PERSON, "adult");
        fx.program.people.profiles.insert(
            TEMPLATE_PERSON,
            Profile {
                tones: vec!["lgt".into()],
                ..Default::default()
            },
        );
        fx.program.people.eager_suits.insert("b001mafit_01".into());
        fx.program.wardrobe = Some(Wardrobe {
            suits: vec!["b001mafit_01".into(), "c001ma_01".into()]
                .into_iter()
                .collect(),
            textures: vec!["b001mafitlgt_01.bmp".into(), "c001malgt_01.bmp".into()]
                .into_iter()
                .collect(),
            errors: Default::default(),
        });
        assert!(fx.world.suit_scan_cache.is_none());
        assert_eq!(
            run_make_char(&mut fx, LAMP, LAMP_FILE, 0, 27, 0, 0),
            Ok(true)
        );
        assert!(
            fx.world.suit_scan_cache.is_some(),
            "fallback Wardrobe Make New Character must freeze suit_scan_cache"
        );
    }

    // 8. P3 #4: Expression reading Global 34 on the RHS of a fresh object traps
    // (`0x452c76 -> 0x4541db` before `0x452c85` initializes `[object + 0x128]`).
    {
        let (mut fx, _, _) = visitors();
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![assign(
                scope::TEMPS,
                0,
                scope::GLOBAL,
                34,
                RETURN_TRUE,
            )]),
        );
        let err = fx
            .run_with(LAMP, 4096, 0, &[], 1000)
            .expect_err("fresh object reading global 34 on Expression RHS must trap");
        assert!(
            matches!(err.trap, Trap::Unsupported(_)),
            "expected Unsupported trap, got {err}"
        );
    }
}

#[test]
fn round_23_dialog_placed_large_lot_strcmp_and_set_to_next_expression() {
    use sims_content::{Profile, SuitTable};

    // 1. P2 #1: Dialog off-lot portrait executes `function::PLACED` (`0x4363d1` ->
    // `0x4e96fe` -> `0x45e4e9` -> `0x433580`), both with explicit `view_centre`
    // and when invariant across all reachable tiles with `view_centre == None`.
    for centre in [Some([2i8, 2i8]), None] {
        let (mut fx, file, record) = visitors();
        fx.add(
            file,
            4200,
            tree(vec![assign(
                scope::GLOBAL,
                36,
                scope::LITERAL,
                88,
                RETURN_TRUE,
            )]),
        );
        let library = fx.program.libraries.get_mut(&file).unwrap();
        for table in library.callbacks.values_mut() {
            table.resize(
                27,
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: 0,
                },
            );
            table[9].action = 4200; // function::PLACED
        }
        // Dialog (opcode 36): icon mode 2 (neighbor in stack object), wait=0.
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                36,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 0, 0, 0, 0, 0, 0, 1 | (2 << 1)],
            )]),
        );
        let mut interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        interpreter.view_centre = centre;
        assert_eq!(
            interpreter.run_tree(&mut fx.world, LAMP, 4096, record, &[], 1000),
            Ok(true),
            "Dialog with Placed callback must succeed for view_centre={centre:?}"
        );
        assert_eq!(fx.world.globals[36], 88);
        assert!(fx.world.portraits.contains(&WIDGET));
    }

    // 2. P2 #2: UI Effect on a 19x19 lot (289 interior tiles > 256) with
    // `function::PLACED` and `view_centre == None` succeeds instead of trapping
    // on an artificial candidate cap.
    {
        let (mut fx, file, record) = visitors();
        fx.world.size = 19;
        fx.add(
            file,
            4200,
            tree(vec![assign(
                scope::GLOBAL,
                36,
                scope::LITERAL,
                91,
                RETURN_TRUE,
            )]),
        );
        let library = fx.program.libraries.get_mut(&file).unwrap();
        for table in library.callbacks.values_mut() {
            table.resize(
                27,
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: 0,
                },
            );
            table[9].action = 4200;
        }
        let [lo, hi] = record.to_le_bytes();
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                UI_EFFECT,
                RETURN_TRUE,
                RETURN_FALSE,
                [1, scope::LITERAL, lo, hi, 1, 0, 0, 0],
            )]),
        );
        let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        assert_eq!(
            interpreter.run_tree(&mut fx.world, LAMP, 4096, 0, &[], 1000),
            Ok(true),
            "UI Effect with 289 invariant candidate tiles and view_centre=None must succeed"
        );
        assert_eq!(fx.world.globals[36], 91);
        assert!(fx.world.portraits.contains(&WIDGET));
    }

    // 3. P3 #1: Clothing body-type validation uses case-sensitive bytewise `strcmp`
    // (`0x40a91b -> 0x634d50`), so an inherited formal row with `FIT` fails against
    // a lowercase `fit` everyday body and enters fallback.
    {
        let (mut fx, _, _) = visitors();
        const TEMPLATE_PERSON: u32 = 0x7FD9_6B54;
        let tpl_file = fx.add_type(TEMPLATE_PERSON, 2, 4096, 4097);
        let id = fx.world.neighbors.slots.len() as i16 + 1;
        fx.world.neighbors.slots.push(Some(Neighbor {
            guid: TEMPLATE_PERSON,
            id,
            data: vec![0; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 0,
            stale: false,
        }));
        fx.add(
            tpl_file,
            4096,
            tree(vec![assign(
                scope::MY_DATA,
                4,
                scope::LITERAL,
                1,
                RETURN_TRUE,
            )]),
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
        fx.program.people.profiles.insert(
            TEMPLATE_PERSON,
            Profile {
                body: Some("b001mafit_01,BODY=b001mafitlgt_01".into()),
                head: Some("c001ma_01,HEAD-HEAD=c001malgt_01".into()),
                gender: Some("male".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                tones: vec!["lgt".into()],
                suit_rows: BTreeMap::from([
                    // Uppercase `FIT` matches wardrobe case-insensitively, but fails
                    // bytewise `strcmp` against lowercase `fit` at `0x40a91b..0x40a968`!
                    (30, "f200maFIT_01,BODY=f200maFITlgt_01".into()),
                    (31, "s200maFIT_01,BODY=s200maFITlgt_01".into()),
                    (32, "l200maFIT_01,BODY=l200maFITlgt_01".into()),
                ]),
                ..Default::default()
            },
        );
        let mut table = SuitTable::default();
        for name in [
            "b001mafit_01",
            "c001ma_01",
            "f100mafit_01",
            "f200maFIT_01",
            "s100mafit_01",
            "s200maFIT_01",
            "l100mafit_01",
            "l200maFIT_01",
        ] {
            table.insert_name(name.into());
        }
        fx.program.people.suit_table = table;
        fx.program.people.textures = vec![
            "b001mafitlgt_01".into(),
            "c001malgt_01".into(),
            "f100mafitlgt_01".into(),
            "f200maFITlgt_01".into(),
            "s100mafitlgt_01".into(),
            "s200maFITlgt_01".into(),
            "l100mafitlgt_01".into(),
            "l200maFITlgt_01".into(),
        ];
        fx.program.wardrobe = Some(Wardrobe {
            suits: vec!["b001mafit_01".into(), "c001ma_01".into()]
                .into_iter()
                .collect(),
            textures: vec!["b001mafitlgt_01.bmp".into(), "c001malgt_01.bmp".into()]
                .into_iter()
                .collect(),
            errors: Default::default(),
        });
        let mut t = tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
            assign(scope::LOCALS, 1, scope::LITERAL, 27, 2),
            assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
            ins(19, RETURN_TRUE, RETURN_FALSE, [0, 1, 2, 0, 0, 0, 0, 0]),
        ]);
        t.locals = 3;
        fx.add(LAMP_FILE, 4096, t);
        assert_eq!(fx.run_with(LAMP, 4096, 0, &[], 1000), Ok(true));
        let made_guid = *fx.world.characters.made.keys().next().unwrap();
        let outfits = &fx.world.made_outfits[&made_guid];
        // Because `FIT` != `fit` in `strcmp`, all three indices were invalidated to None
        // and `0x40b7ed` regenerated `f100`/`s100`/`l100`!
        assert_eq!(
            outfits.get(&30).map(String::as_str),
            Some("f100mafit_01,BODY=f100mafitlgt_01"),
            "case-sensitive strcmp at 0x40a91b must reject FIT against fit and regenerate f100"
        );
        assert_eq!(
            outfits.get(&31).map(String::as_str),
            Some("s100mafit_01,BODY=s100mafitlgt_01"),
            "case-sensitive strcmp at 0x40a93c must reject FIT against fit and regenerate s100"
        );
        assert_eq!(
            outfits.get(&32).map(String::as_str),
            Some("l100mafit_01,BODY=l100mafitlgt_01"),
            "case-sensitive strcmp at 0x40a95d must reject FIT against fit and regenerate l100"
        );
    }

    // 4. P3 #2: `Set To Next` followed by `Expression` reading Global 34 on the RHS
    // succeeds (`0x44eb9d` points inside `Expression`'s 0x58-byte local frame),
    // and `0x452c85` subsequently makes Global 34 safe for `UI Effect` too, whereas
    // `Set To Next` directly before `UI Effect` still traps.
    {
        let (mut fx, _, record) = visitors();
        fx.world.globals[34] = record;
        // Node 0: Set To Next (op 31, mode 0 = all objects, dest = stack object).
        // Node 1: Expression `temp[0] = global[34]` (succeeds!).
        // Node 2: UI Effect mode 1 reading global[34] (succeeds after Expression!).
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![
                ins(31, 1, 1, [0; 8]),
                assign(scope::TEMPS, 0, scope::GLOBAL, 34, 2),
                ins(
                    UI_EFFECT,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [1, scope::GLOBAL, 34, 0, 1, 0, 0, 0],
                ),
            ]),
        );
        assert_eq!(fx.run_with(LAMP, 4096, 0, &[], 1000), Ok(true));
        assert_eq!(fx.world.objects[&LAMP].temps[0], record);
        assert!(fx.world.portraits.contains(&WIDGET));
        assert!(fx.world.scratch_ptr_set.contains(&LAMP));
        assert!(!fx.world.scratch_ptr_set_to_next.contains(&LAMP));
    }
}

#[test]
fn round_24_search_callback_invariance_expr_g34_redirect_and_formal_skip() {
    use sims_content::Profile;

    // 1. P2: Portrait search with a state-changing overlap callback (`0x433a1a` /
    // `0x433a35`) succeeds with `view_centre == None` in BOTH UI Effect and Dialog
    // when every interior camera start finds the same tile, runs the callback,
    // and leaves identical post-removal state.
    {
        for use_dialog in [false, true] {
            let (mut fx, file, record) = visitors();
            // Single room everywhere; block all interior tiles EXCEPT (2, 2) with
            // immovable obstacles, and put a neighbor obstacle beside (2, 2) at
            // (2, 3) so every interior start lands on (2, 2) and triggers function 5
            // on the person (`0x433a1a`), which writes `global[36] = 66` and returns false.
            fx.world.rooms.rooms.clear();
            fx.world.rooms.rooms.insert(
                1,
                sims_sim::rooms::Room {
                    outdoors: true,
                    ..Default::default()
                },
            );
            fx.world.rooms.map[0].fill(1);
            const BLOCKER: u32 = 0x7788;
            let bfile = fx.add_type(BLOCKER, 4, 4096, 4097);
            fx.add(
                bfile,
                4096,
                tree(vec![assign(
                    scope::MY_DATA,
                    4,
                    scope::LITERAL,
                    1,
                    RETURN_TRUE,
                )]),
            );
            fx.add(
                bfile,
                4097,
                tree(vec![ins(0, RETURN_TRUE, RETURN_TRUE, [0; 8])]),
            );
            // Put blockers on all 15 interior tiles except (2, 2).
            for x in 1..5 {
                for y in 1..5 {
                    if (x, y) == (2, 2) {
                        continue;
                    }
                    let bid = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream)
                        .create(&mut fx.world, BLOCKER)
                        .unwrap();
                    let obj = fx.world.objects.get_mut(&bid).unwrap();
                    obj.position = [x * 16 + 8, y * 16 + 8, 1];
                    obj.data[29] = 1;
                    obj.data[4] = 1;
                    let idx = (x as usize) * 64 + (y as usize);
                    fx.world.arrays.get_mut(&3).unwrap().data[idx * 2..idx * 2 + 2]
                        .copy_from_slice(&(bid as i16).to_le_bytes());
                }
            }
            // Person's function 5 writes global 36 = 66 and returns false (cannot overlap).
            fx.add(
                file,
                4200,
                tree(vec![assign(
                    scope::GLOBAL,
                    36,
                    scope::LITERAL,
                    66,
                    RETURN_FALSE,
                )]),
            );
            let library = fx.program.libraries.get_mut(&file).unwrap();
            for table in library.callbacks.values_mut() {
                table.resize(
                    27,
                    sims_formats::execution::ObjectFunction {
                        condition: 0,
                        action: 0,
                    },
                );
                table[5].action = 4200;
            }
            let ran = if use_dialog {
                fx.add(
                    LAMP_FILE,
                    4096,
                    tree(vec![ins(
                        36,
                        RETURN_TRUE,
                        RETURN_FALSE,
                        [0, 0, 0, 0, 0, 0, 0, 1 | (2 << 1)],
                    )]),
                );
                fx.run_with(LAMP, 4096, record, &[], 1000)
            } else {
                run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1)
            };
            assert_eq!(
                ran,
                Ok(true),
                "invariant search callback must succeed with view_centre=None (dialog={use_dialog})"
            );
            assert_eq!(fx.world.globals[36], 66);
            assert!(fx.world.portraits.contains(&WIDGET));
        }
    }

    // 2. P3 #1: After an Expression at the same dispatch depth (`0x452c85`),
    // `My Motive[1] = Global[34]` writes Global 34 back to itself (`0x4541db` ->
    // `0x452e8b`) instead of changing the motive!
    {
        let (mut fx, _, _) = visitors();
        fx.world.globals[34] = 37;
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .motives[1] = -40.0;
        // Node 0: Expression `temp[0] = 0` (sets `[edi + 0x128] = &[ebp - 0x04]`).
        // Node 1: Expression `My Motive[1] = Global[34]` (redirects write to Global[34]!).
        fx.add(
            SIM_FILE,
            4096,
            tree(vec![
                assign(scope::TEMPS, 0, scope::LITERAL, 0, 1),
                assign(14, 1, scope::GLOBAL, 34, RETURN_TRUE),
            ]),
        );
        assert_eq!(fx.run_with(SIM, 4096, 0, &[], 1000), Ok(true));
        let motive = fx.world.objects[&SIM].person.as_ref().unwrap().motives[1];
        assert_eq!(
            motive, -40.0,
            "after Expression, My Motive[1] = Global[34] must leave motive at -40"
        );
        assert_eq!(fx.world.globals[34], 37);
    }

    // 3. P3 #2: In `0x40af5b` (`0x40b2f5..0x40b310`), once a naked suit has matched
    // during the `scan` walk, formal-suit matching is skipped for the rest of the
    // scan, leaving row 27's suit field empty when the formal suit appears later.
    {
        let (mut fx, _, _) = visitors();
        const TEMPLATE_PERSON: u32 = 0x7FD9_6B54;
        let tpl_file = fx.add_type(TEMPLATE_PERSON, 2, 4096, 4097);
        let id = fx.world.neighbors.slots.len() as i16 + 1;
        fx.world.neighbors.slots.push(Some(Neighbor {
            guid: TEMPLATE_PERSON,
            id,
            data: vec![0; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 0,
            stale: false,
        }));
        fx.add(
            tpl_file,
            4096,
            tree(vec![assign(
                scope::MY_DATA,
                4,
                scope::LITERAL,
                1,
                RETURN_TRUE,
            )]),
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
        fx.program.people.profiles.insert(
            TEMPLATE_PERSON,
            Profile {
                body: Some("b001mafit_01,BODY=b001mafitlgt_01".into()),
                head: Some("c001ma_01,HEAD-HEAD=c001malgt_01".into()),
                gender: Some("male".into()),
                age: Some("27".into()),
                skin: Some("lgt".into()),
                tones: vec!["lgt".into()],
                ..Default::default()
            },
        );
        // Pre-seed `suit_scan_cache` with `nmfit_01` BEFORE `fmfit_02` so we
        // deterministically test `0x40b2f5..0x40b310` skipping formal after naked!
        fx.world.suit_scan_cache = Some(vec![
            "b001mafit_01".into(),
            "c001ma_01".into(),
            "nmfit_01".into(),
            "fmfit_02".into(),
        ]);
        fx.program.people.textures = vec![
            "b001mafitlgt_01".into(),
            "c001malgt_01".into(),
            "nmfitlgt_01".into(),
            "fmfitlgt_02".into(),
        ];
        fx.program.wardrobe = Some(Wardrobe {
            suits: vec!["b001mafit_01".into(), "c001ma_01".into()]
                .into_iter()
                .collect(),
            textures: vec!["b001mafitlgt_01.bmp".into(), "c001malgt_01.bmp".into()]
                .into_iter()
                .collect(),
            errors: Default::default(),
        });
        let mut t = tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
            assign(scope::LOCALS, 1, scope::LITERAL, 27, 2),
            assign(scope::LOCALS, 2, scope::LITERAL, 0, 3),
            ins(19, RETURN_TRUE, RETURN_FALSE, [0, 1, 2, 0, 0, 0, 0, 0]),
        ]);
        t.locals = 3;
        fx.add(LAMP_FILE, 4096, t);
        assert_eq!(fx.run_with(LAMP, 4096, 0, &[], 1000), Ok(true));
        let made_guid = *fx.world.characters.made.keys().next().unwrap();
        let outfits = &fx.world.made_outfits[&made_guid];
        assert_eq!(
            outfits.get(&15).map(String::as_str),
            Some("nmfit_01,BODY=nmfitlgt_01")
        );
        assert_eq!(
            outfits.get(&27).map(String::as_str),
            Some(",BODY=fmfitlgt_02"),
            "0x40b2f5..0x40b310 must skip formal suit matching once naked suit matched earlier in scan"
        );
    }
}

#[test]
fn round_25_mixed_camera_starts_neighbor_clamp_and_arg_slot() {
    use sims_content::Profile;
    // 1. P2 #1 (`placement.rs:1379`): when some camera starts find no tile
    // (`0x4e9706`) and others place the temporary Sim (`0x4e96fe`), both UI
    // Effect and Dialog must compare the post-removal outcomes before trapping.
    // When the temporary Sim cannot render (missing HEAD binding), every start
    // produces identical state, so both succeed with `view_centre = None`.
    for use_dialog in [false, true] {
        let (mut fx, _, record) = visitors();
        // Put LAMP on (1,1) in isolated room 99 so start (1,1) finds no tile
        // while all other interior starts find a tile.
        fx.world.rooms.map[0][64 + 1] = 99;
        fx.world.objects.get_mut(&LAMP).unwrap().position = [1, 1, 1];
        fx.world.objects.get_mut(&LAMP).unwrap().data[29] = 99;
        fx.world.arrays.get_mut(&3).unwrap().data[(64 + 1) * 2..(64 + 1) * 2 + 2]
            .copy_from_slice(&(LAMP as i16).to_le_bytes());
        // Give WIDGET a profile whose head suit does not exist so rendering
        // returns false on placed starts, making placed (0x4e96fe) and unput
        // (0x4e9706) outcomes identical after removal.
        equip(&mut fx, WIDGET, "adult", 12);
        fx.program.people.profiles.insert(
            WIDGET,
            Profile {
                body: Some("missing_body,BODY=missing_body".into()),
                head: Some("missing_head,HEAD-HEAD=missing_head".into()),
                ..Default::default()
            },
        );
        let ran = if use_dialog {
            fx.add(
                LAMP_FILE,
                4096,
                tree(vec![ins(
                    36,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [0, 0, 0, 0, 0, 0, 0, 1 | (2 << 1)],
                )]),
            );
            fx.run_with(LAMP, 4096, record, &[], 1000)
        } else {
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1)
        };
        assert_eq!(
            ran,
            Ok(true),
            "mixed placed/no-tile starts with identical post-removal state must succeed (dialog={use_dialog})"
        );
        assert!(!fx.world.portraits.contains(&WIDGET));
    }

    // 2. P3 #1 (`expression.rs:74`): scope 24 (`Neighbor`) clears `[ebp - 0x04]`
    // via `0x453399 -> 0x425b8e`, so after a prior Expression, `Neighbor[0] +=
    // Global[34]` raises native error `0x13` instead of mutating Global 34.
    {
        let (mut fx, _, record) = visitors();
        fx.world.globals[34] = 37;
        let mut add_neighbor_g34 = [0u8; 8];
        add_neighbor_g34[0..2].copy_from_slice(&0i16.to_le_bytes()); // Neighbor[0]
        add_neighbor_g34[2..4].copy_from_slice(&34i16.to_le_bytes()); // Global[34]
        add_neighbor_g34[5] = 3; // +=
        add_neighbor_g34[6] = 24; // Neighbor scope
        add_neighbor_g34[7] = scope::GLOBAL;
        fx.add(
            SIM_FILE,
            4096,
            tree(vec![
                assign(scope::TEMPS, 0, scope::LITERAL, 0, 1),
                ins(2, RETURN_TRUE, RETURN_FALSE, add_neighbor_g34),
            ]),
        );
        let err = fx.run_with(SIM, 4096, record, &[], 1000).unwrap_err();
        assert!(
            matches!(err.trap, sims_vm::Trap::Native { code: 0x13, .. }),
            "Neighbor[0] += Global[34] after Expression must raise native 0x13, got {err}"
        );
        assert_eq!(fx.world.globals[34], 37);
    }

    // 3. P3 #2 (`expression.rs:84`): after a prior Expression, `My Motive[1] +=
    // Global[34]` redirects the write to Global 34 (`37 -> 74`) AND still runs
    // the native `[-100.0, 100.0]` motive clamp at `0x452d22..0x452d4d`.
    {
        let (mut fx, _, _) = visitors();
        fx.world.globals[34] = 37;
        let sim_obj = fx.world.objects.get_mut(&SIM).unwrap();
        sim_obj.data[11] = SIM as i16;
        sim_obj.person.as_mut().unwrap().motives[1] = 125.0;
        let mut add_motive_g34 = [0u8; 8];
        add_motive_g34[0..2].copy_from_slice(&1i16.to_le_bytes()); // Motive[1]
        add_motive_g34[2..4].copy_from_slice(&34i16.to_le_bytes()); // Global[34]
        add_motive_g34[5] = 3; // +=
        add_motive_g34[6] = 14; // My Motive
        add_motive_g34[7] = scope::GLOBAL;
        fx.add(
            SIM_FILE,
            4096,
            tree(vec![
                assign(scope::TEMPS, 0, scope::LITERAL, 0, 1),
                ins(2, RETURN_TRUE, RETURN_FALSE, add_motive_g34),
            ]),
        );
        assert_eq!(fx.run_with(SIM, 4096, 0, &[], 1000), Ok(true));
        assert_eq!(fx.world.globals[34], 74);
        let motive = fx.world.objects[&SIM].person.as_ref().unwrap().motives[1];
        assert_eq!(
            motive, 100.0,
            "redirected motive write must still clamp motive to 100.0 (0x452d22..0x452d4d)"
        );
    }

    // 4. P3 #3 (`expression.rs:53`): after Random Number (`0x451e8d`) or Get
    // Direction To (`0x45202b`), a same-depth Expression comparison reading
    // Global 34 on the RHS overwrites `[ebp + 0x08]` (`write` flag) before
    // `0x452c7b`, so comparing against `My Person Data[68]` dirties the
    // running Sim's cached portrait (`0x4539c7`), and comparing against
    // `My Motive[1]` clamps the motive (`0x452d01..0x452d4d`).
    for use_direction in [false, true] {
        let (mut fx, _, _) = visitors();
        fx.world.globals[34] = 0;
        fx.world.portraits.insert(SIM_GUID);
        fx.world.dirty_portraits.remove(&SIM_GUID);
        let sim_obj = fx.world.objects.get_mut(&SIM).unwrap();
        sim_obj.data[11] = SIM as i16;
        sim_obj.person.as_mut().unwrap().motives[1] = 125.0;
        let setup_ins = if use_direction {
            // Get Direction To (opcode 12): dest temp[0], from my ID (flags 0).
            let mut op = [0u8; 8];
            op[0..2].copy_from_slice(&0i16.to_le_bytes());
            op[2..4].copy_from_slice(&(scope::TEMPS as i16).to_le_bytes());
            ins(12, 1, RETURN_FALSE, op)
        } else {
            // Random Number (opcode 8): dest temp[0], range literal 10.
            let mut op = [0u8; 8];
            op[0..2].copy_from_slice(&0i16.to_le_bytes());
            op[2..4].copy_from_slice(&(scope::TEMPS as i16).to_le_bytes());
            op[4..6].copy_from_slice(&10i16.to_le_bytes());
            op[6..8].copy_from_slice(&(scope::LITERAL as i16).to_le_bytes());
            ins(8, 1, RETURN_FALSE, op)
        };
        // Node 1: `My Person Data[68] == Global[34]` (comparison -> dirties portrait!)
        let mut cmp_pd68 = [0u8; 8];
        cmp_pd68[0..2].copy_from_slice(&68i16.to_le_bytes());
        cmp_pd68[2..4].copy_from_slice(&34i16.to_le_bytes());
        cmp_pd68[5] = 2; // ==
        cmp_pd68[6] = 18; // My Person Data
        cmp_pd68[7] = scope::GLOBAL;
        fx.add(
            SIM_FILE,
            4096,
            tree(vec![
                setup_ins.clone(),
                ins(2, RETURN_TRUE, RETURN_TRUE, cmp_pd68),
            ]),
        );
        assert_eq!(fx.run_with(SIM, 4096, LAMP as i16, &[], 1000), Ok(true));
        assert!(
            !fx.world.portraits.contains(&SIM_GUID) && fx.world.dirty_portraits.contains(&SIM_GUID),
            "Global 34 comparison after {} must dirty SIM portrait at 0x4539c7",
            if use_direction {
                "Get Direction To"
            } else {
                "Random Number"
            }
        );

        // Also check motive clamp on comparison after Random Number / Get Direction To.
        let mut cmp_motive = [0u8; 8];
        cmp_motive[0..2].copy_from_slice(&1i16.to_le_bytes());
        cmp_motive[2..4].copy_from_slice(&34i16.to_le_bytes());
        cmp_motive[5] = 2; // ==
        cmp_motive[6] = 14; // My Motive
        cmp_motive[7] = scope::GLOBAL;
        fx.add(
            SIM_FILE,
            4097,
            tree(vec![
                setup_ins,
                ins(2, RETURN_TRUE, RETURN_TRUE, cmp_motive),
            ]),
        );
        assert_eq!(fx.run_with(SIM, 4097, LAMP as i16, &[], 1000), Ok(true));
        let motive = fx.world.objects[&SIM].person.as_ref().unwrap().motives[1];
        assert_eq!(
            motive,
            100.0,
            "Global 34 motive comparison after {} must clamp motive at 0x452d01..0x452d4d",
            if use_direction {
                "Get Direction To"
            } else {
                "Random Number"
            }
        );
    }
}

#[test]
fn round_26_dirty_reset_invalidation_mixed_starts() {
    // P2 (`ui_effect.rs:719`, `dialog.rs:610`): on a dirty portrait (`save_bitmap
    // = false`), a placed temporary Sim can render/cache the portrait and then
    // immediately invalidate it again during removal `Reset` (writing My Person
    // Data[68] via `0x4539c7`), while an unplaced start (`0x4e9706`) skips
    // rendering and leaves the portrait dirty too. Both paths produce identical
    // post-removal `(LotSnapshot, Cells)` before `0x4e9338` loads embedded
    // `BMP_` 2002, so BOTH UI Effect and Dialog must succeed with `view_centre =
    // None` instead of trapping on the discarded render bool.
    for use_dialog in [false, true] {
        let (mut fx, file, record) = visitors();
        // Put LAMP on (1,1) in isolated room 99 so start (1,1) finds no tile
        // while other interior starts find a tile.
        fx.world.rooms.map[0][64 + 1] = 99;
        fx.world.objects.get_mut(&LAMP).unwrap().position = [1, 1, 1];
        fx.world.objects.get_mut(&LAMP).unwrap().data[29] = 99;
        fx.world.arrays.get_mut(&3).unwrap().data[(64 + 1) * 2..(64 + 1) * 2 + 2]
            .copy_from_slice(&(LAMP as i16).to_le_bytes());
        // Embedded BMP_ 2002 + dirty portrait slot.
        fx.program
            .libraries
            .get_mut(&file)
            .unwrap()
            .has_portrait_bitmap = true;
        fx.world.dirty_portraits.insert(WIDGET);
        // Removal callback (`BEFORE_REMOVAL`, function 5 -> Wait: use `Reset` /
        // function 5 `BEFORE_REMOVAL` which runs during `remove_now` on BOTH
        // placed and unplaced paths!) writing My Person Data[68] = 0 so placed
        // render's cached portrait is immediately dirtied again on removal.
        fx.add(
            file,
            4200,
            tree(vec![assign(18, 68, scope::LITERAL, 0, RETURN_TRUE)]),
        );
        let library = fx.program.libraries.get_mut(&file).unwrap();
        for table in library.callbacks.values_mut() {
            table.resize(
                27,
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: 0,
                },
            );
            // Function 3 (`RESET`) runs during `remove_now` on both placed and
            // unplaced paths, writing My Person Data[68] to re-dirty the slot.
            table[3].action = 4200;
        }
        let ran = if use_dialog {
            fx.add(
                LAMP_FILE,
                4096,
                tree(vec![ins(
                    36,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [0, 0, 0, 0, 0, 0, 0, 1 | (2 << 1)],
                )]),
            );
            fx.run_with(LAMP, 4096, record, &[], 1000)
        } else {
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1)
        };
        assert_eq!(
            ran,
            Ok(true),
            "dirty mixed starts with identical post-removal state must succeed (dialog={use_dialog})"
        );
        assert!(fx.world.portraits.contains(&WIDGET));
        assert!(!fx.world.dirty_portraits.contains(&WIDGET));
    }
}

#[test]
fn round_27_held_object_callback_and_grouped_type2() {
    // 1. P2 #1 (`placement.rs:1417`): when a temporary Sim's Init places a held
    // object into one of its slots (`0x433517 -> 0x433612 -> 0x4337d8`) whose
    // `PLACED` callback depends on the tile/room, omitting `view_centre` must
    // NOT take the O(1) `FastPlaced` shortcut — it must detect the tile
    // dependence and fail loudly instead of silently skipping the held object's
    // callback.
    for use_dialog in [false, true] {
        let (mut fx, file, record) = visitors();
        fx.world.globals[19] = 0; // Live mode so Grab succeeds.
        // Place LAMP at (2, 2) facing east (+X -> tile (3, 2)) as a SEAT
        // (function 26) so placing WIDGET on (3, 2) sets bit 0x100 on any
        // held prop and dispatches `PLACED` at `0x4337d8`, whereas placing
        // WIDGET on (3, 3) does not.
        fx.lamp_mut().position = [(2 << 4) | 8, (2 << 4) | 8, 1];
        fx.lamp_mut().data[1] = 2; // facing +X (east)
        fx.world.arrays.get_mut(&3).unwrap().data[(2 * 64 + 2) * 2..(2 * 64 + 2) * 2 + 2]
            .copy_from_slice(&(LAMP as i16).to_le_bytes());
        let lamp_lib = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
        for table in lamp_lib.callbacks.values_mut() {
            table.resize(
                27,
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: 0,
                },
            );
            table[26].action = 4096; // SEAT
        }
        // Define a carryable prop (`data[4] = 0x21`) with `SEAT_CHANGED` (25)
        // and `PLACED` (5) incrementing `Global[36] += 1`.
        const PROP_GUID: u32 = 0x5500_0027;
        let prop_file = fx.add_type(PROP_GUID, 4, 4096, 4097);
        fx.add(
            prop_file,
            4096,
            tree(vec![assign(
                scope::MY_DATA,
                4,
                scope::LITERAL,
                0x21,
                RETURN_TRUE,
            )]),
        );
        fx.add(
            prop_file,
            4097,
            tree(vec![
                assign(scope::PARAMETERS, 0, scope::LITERAL, 10, 1),
                ins(0, 0, 0, [0; 8]),
            ]),
        );
        let mut inc_g36 = [0u8; 8];
        inc_g36[0..2].copy_from_slice(&36i16.to_le_bytes());
        inc_g36[2..4].copy_from_slice(&1i16.to_le_bytes());
        inc_g36[5] = 3; // +=
        inc_g36[6] = scope::GLOBAL;
        inc_g36[7] = scope::LITERAL;
        fx.add(
            prop_file,
            4200,
            tree(vec![ins(2, RETURN_TRUE, RETURN_FALSE, inc_g36)]),
        );
        let prop_lib = fx.program.libraries.get_mut(&prop_file).unwrap();
        for table in prop_lib.callbacks.values_mut() {
            table.resize(
                27,
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: 0,
                },
            );
            table[9].action = 4200; // PLACED
            table[25].action = 4200; // SEAT_CHANGED
        }
        // Provide a carry clip (STR# 150 index 0x26) so holding PROP_GUID in
        // hand slot 0 succeeds during `put_person` without hitting 0x59db2f.
        let mut names = vec![String::new(); 0x27];
        names[0x26] = "test-carry".into();
        fx.program
            .libraries
            .get_mut(&file)
            .unwrap()
            .strings
            .insert(150, names);
        fx.program.animations.insert(
            "test.far",
            sims_content::Clip {
                name: "test-carry".into(),
                skill: "test-carry-skill".into(),
                duration: 100.0f32.to_bits(),
                distance: 0,
                moving: false,
                frames: 4,
                motions: 1,
                tracks: Vec::new(),
                channels: Vec::new(),
                samples: [0, 0],
            },
        );
        fx.program.animations.insert_samples("test-carry-skill");
        // WIDGET's Init sets its normal placement flags (data[4] = 1, data[42] = 2),
        // places itself on tile (1, 1) via `Find Location For` (opcode 16, mode 0),
        // and creates PROP_GUID into hand slot 0 (`Create New Object Instance`,
        // opcode 42, mode 1) so `put_person` walks PROP_GUID at `0x433517 ->
        // 0x433612 -> 0x4337d8`.
        let mut create_in_slot = [0u8; 8];
        create_in_slot[0..4].copy_from_slice(&PROP_GUID.to_le_bytes());
        create_in_slot[4] = 1; // in my container slot 0
        // Skip re-creating PROP_GUID if construction runs Init a second time
        // while slot 0 already holds it.
        let mut slot0_empty = [0u8; 8];
        slot0_empty[0..2].copy_from_slice(&0i16.to_le_bytes());
        slot0_empty[2..4].copy_from_slice(&0i16.to_le_bytes());
        slot0_empty[5] = 2; // ==
        slot0_empty[6] = 20; // My Slot
        slot0_empty[7] = scope::LITERAL;
        fx.add(
            file,
            4096,
            tree(vec![
                ins(2, 1, RETURN_TRUE, slot0_empty),
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 2),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, 3),
                assign(scope::STACK_OBJECT, 0, scope::MY_DATA, 11, 4),
                ins(16, 5, RETURN_FALSE, [0; 8]),
                ins(42, RETURN_TRUE, RETURN_FALSE, create_in_slot),
            ]),
        );
        let ran = if use_dialog {
            fx.add(
                LAMP_FILE,
                4096,
                tree(vec![ins(
                    36,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [0, 0, 0, 0, 0, 0, 0, 1 | (2 << 1)],
                )]),
            );
            fx.run_with(LAMP, 4096, record, &[], 1000)
        } else {
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1)
        };
        let err = ran.unwrap_err().to_string();
        assert!(
            err.contains("0x4e96dd") || err.contains("view_centre"),
            "held object PLACED callback depending on room must fail loudly without view_centre (dialog={use_dialog}): {err}"
        );
    }

    // 2. P2 #2 (`ui_effect.rs:574`, `dialog.rs:492`): a grouped definition
    // (`OBJD[GROUP] != 0`) with `OBJD[TYPE] == 2` enters the group factory at
    // `0x440b6f`, creates the non-person group, fails the `cXPerson` cast at
    // `0x4e96b7`, and leaves the group registered at `0x4e96c3` without
    // trapping (in both UI Effect and Dialog).
    for use_dialog in [false, true] {
        let (mut fx, _, _) = visitors();
        const MASTER_GUID: u32 = 0x5500_0028;
        const LEAD_GUID: u32 = 0x5500_0029;
        let group_file = fx.add_type(MASTER_GUID, 2, 4096, 4097);
        // Mark MASTER_GUID as a group master (GROUP = 1, SUB_INDEX = -1).
        let master_words = fx.words(group_file);
        master_words[10] = 1; // GROUP
        master_words[11] = -1; // SUB_INDEX (master)
        let master_idx = fx.catalog.by_guid[&MASTER_GUID];
        fx.catalog.objects[master_idx].master = 1;
        // Add lead tile in the same resource file (GROUP = 1, SUB_INDEX = 0, LEAD = 1).
        let lead_chunk = 129u16;
        let mut lead_words = [0i16; 108];
        lead_words[0] = 138;
        lead_words[9] = 4; // normal object tile
        lead_words[10] = 1; // GROUP = 1
        lead_words[11] = 0; // SUB_INDEX = (0, 0)
        lead_words[14] = LEAD_GUID as i16;
        lead_words[15] = (LEAD_GUID >> 16) as i16;
        lead_words[0x7e / 2] = 1; // LEAD = 1
        let lead_idx = fx.catalog.objects.len();
        fx.catalog.objects.push(CatalogObject {
            guid: LEAD_GUID,
            resource: group_file,
            chunk: lead_chunk,
            kind: 4,
            master: 1,
        });
        fx.catalog.by_guid.insert(LEAD_GUID, lead_idx);
        let lib = fx.program.libraries.get_mut(&group_file).unwrap();
        lib.definition_order.push(lead_chunk);
        lib.definitions.insert(lead_chunk, lead_words);
        lib.callbacks.insert(
            lead_chunk,
            vec![
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: 4096,
                },
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: 4097,
                },
            ],
        );
        // Lead Init sets Global[36] = 88 to prove group factory ran.
        fx.add(
            group_file,
            4096,
            tree(vec![assign(
                scope::GLOBAL,
                36,
                scope::LITERAL,
                88,
                RETURN_TRUE,
            )]),
        );
        fx.add(
            group_file,
            4097,
            tree(vec![
                assign(scope::PARAMETERS, 0, scope::LITERAL, 10, 1),
                ins(0, 0, 0, [0; 8]),
            ]),
        );
        let rec_id = fx.world.neighbors.slots.len() as i16 + 1;
        fx.world.neighbors.slots.push(Some(Neighbor {
            guid: MASTER_GUID,
            id: rec_id,
            data: vec![0; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 0,
            stale: false,
        }));
        let ran = if use_dialog {
            fx.add(
                LAMP_FILE,
                4096,
                tree(vec![ins(
                    36,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [0, 0, 0, 0, 0, 0, 0, 1 | (2 << 1)],
                )]),
            );
            fx.run_with(LAMP, 4096, rec_id, &[], 1000)
        } else {
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, rec_id, 1)
        };
        assert_eq!(
            ran,
            Ok(true),
            "grouped type-2 definition must construct via group factory and return cleanly (dialog={use_dialog})"
        );
        assert_eq!(fx.world.globals[36], 88);
        assert!(
            fx.world.objects.values().any(|o| o.guid == LEAD_GUID),
            "created group lead must stay registered after failed cXPerson cast (dialog={use_dialog})"
        );
        assert!(!fx.world.portraits.contains(&MASTER_GUID));
    }
}

#[test]
fn round_28_init_placed_arrival_and_mesh_required_bones() {
    // 1. P2 #1 (`placement.rs:1472`): when a temporary Sim's Init already
    // places the Sim on-lot (`was_outside = false` at `0x433366`), portrait
    // placement must NOT take `FastPlaced` and re-invoke `placed_from_outside`
    // (`0x4335b9..0x4335cc`). Setting person word 77 > 0 after Init places the
    // Sim must succeed identically with `view_centre = None` and `Some([2, 2])`
    // in both UI Effect and Dialog.
    for use_dialog in [false, true] {
        for view_centre in [None, Some([2i8, 2i8])] {
            let (mut fx, file, record) = visitors();
            fx.add(
                file,
                4096,
                tree(vec![
                    assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                    assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
                    assign(scope::STACK_OBJECT, 0, scope::MY_DATA, 11, 3),
                    ins(16, 4, RETURN_FALSE, [0; 8]),
                    assign(
                        18, // My Person Data
                        77,
                        scope::LITERAL,
                        LAMP as i16,
                        RETURN_TRUE,
                    ),
                ]),
            );
            let [lo, hi] = record.to_le_bytes();
            let ran = if use_dialog {
                fx.add(
                    LAMP_FILE,
                    4096,
                    tree(vec![ins(
                        36,
                        RETURN_TRUE,
                        RETURN_FALSE,
                        [0, 0, 0, 0, 0, 0, 0, 1 | (2 << 1)],
                    )]),
                );
                let mut interpreter =
                    sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
                interpreter.view_centre = view_centre;
                interpreter.run_tree(&mut fx.world, LAMP, 4096, record, &[], 1000)
            } else {
                fx.add(
                    LAMP_FILE,
                    4096,
                    tree(vec![ins(
                        UI_EFFECT,
                        RETURN_TRUE,
                        RETURN_FALSE,
                        [1, scope::LITERAL, lo, hi, 1, 0, 0, 0],
                    )]),
                );
                let mut interpreter =
                    sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
                interpreter.view_centre = view_centre;
                interpreter.run_tree(&mut fx.world, LAMP, 4096, 0, &[], 1000)
            };
            assert_eq!(
                ran,
                Ok(true),
                "Init-placed temporary Sim with person word 77 must not re-fire arrival \
                 (dialog={use_dialog}, view_centre={view_centre:?})"
            );
        }
    }

    // 2. P2 #2 (`people.rs:1136`, `ui_effect.rs:179`): a `"HEAD"` binding whose
    // mesh requires a bone absent from the Sim's skeleton fails to bind at
    // `0x599f8a..0x59a01c` and does not render/cache/write `BMP_ 2002`. Once
    // the skeleton defines that bone, rendering succeeds.
    let bone = |name: &str, parent: Option<usize>| sims_content::Bone {
        name: name.into(),
        parent,
        translation: [0; 3],
        rotation: [0, 0, 0, 1.0f32.to_bits()],
        blends: false,
    };
    for has_extra_bone in [false, true] {
        let (mut fx, file, record) = visitors();
        let mut bones = vec![bone("PELVIS", None), bone("HEAD", Some(0))];
        if has_extra_bone {
            bones.push(bone("NECK_EXTRA", Some(1)));
        }
        let scales = vec![0i32; bones.len()];
        fx.program.people.skeletons.insert(
            WIDGET,
            sims_content::Skeleton {
                name: "adult".into(),
                file: "adult.bcf".into(),
                place: 0,
                scales: vec![scales],
                bones,
            },
        );
        fx.program.people.profiles.insert(
            WIDGET,
            sims_content::Profile {
                head: Some("extra_bone_head,tex".into()),
                ..Default::default()
            },
        );
        fx.program.people.suits.insert("extra_bone_head".into());
        fx.program
            .people
            .head_suits
            .insert("extra_bone_head".into());
        fx.program.people.head_mesh_bones.insert(
            "extra_bone_head".into(),
            vec![vec!["HEAD".into(), "NECK_EXTRA".into()]],
        );

        let [lo, hi] = record.to_le_bytes();
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                UI_EFFECT,
                RETURN_TRUE,
                RETURN_FALSE,
                [1, scope::LITERAL, lo, hi, 1, 0, 0, 0],
            )]),
        );
        let mut interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        interpreter.view_centre = Some([2i8, 2i8]);
        let ran = interpreter.run_tree(&mut fx.world, LAMP, 4096, 0, &[], 1000);
        assert_eq!(ran, Ok(true));
        assert_eq!(
            fx.world.portraits.contains(&WIDGET),
            has_extra_bone,
            "portrait cache must match whether skeleton defines all required mesh bones \
             (has_extra_bone={has_extra_bone})"
        );
        assert_eq!(
            fx.world
                .generated_portrait_2002
                .contains(&PortraitFile::Resource(file)),
            has_extra_bone,
            "BMP_ 2002 writeback must match whether skeleton defines all required mesh bones \
             (has_extra_bone={has_extra_bone})"
        );
    }
}

#[test]
fn round_29_ui_effect_events_catalog_bones_and_three_passes() {
    use sims_vm::UiEffectEvent;
    use std::cell::RefCell;

    // 1. P1: UI Effect modes 0, 1, 2 emit `UiEffectEvent` with operand 4 bit 0
    // (`0x476485`, `0x47690d` gated by `0x4768e8`, `0x4769c3`).
    {
        let (mut fx, _, record) = visitors();
        let [lo, hi] = record.to_le_bytes();
        let events = RefCell::new(Vec::new());
        // Mode 0 (highlight = 1): emits Button + falls through to Mode 1 Portrait.
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                UI_EFFECT,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, scope::LITERAL, lo, hi, 1, 0, 0, 0],
            )]),
        );
        let mut interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        interpreter.ui_effects = Some(&events);
        assert_eq!(
            interpreter.run_tree(&mut fx.world, LAMP, 4096, 0, &[], 1000),
            Ok(true)
        );
        assert_eq!(
            *events.borrow(),
            vec![
                UiEffectEvent::Button {
                    id: record,
                    highlight: true
                },
                UiEffectEvent::Portrait {
                    guid: WIDGET,
                    highlight: true
                },
            ]
        );

        // Mode 1 (highlight = 0) and Mode 2 (highlight = 1, then 0):
        events.borrow_mut().clear();
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![
                ins(
                    UI_EFFECT,
                    1,
                    RETURN_FALSE,
                    [1, scope::LITERAL, lo, hi, 0, 0, 0, 0],
                ),
                ins(
                    UI_EFFECT,
                    2,
                    RETURN_FALSE,
                    [2, scope::LITERAL, lo, hi, 1, 0, 0, 0],
                ),
                ins(
                    UI_EFFECT,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [2, scope::LITERAL, lo, hi, 0, 0, 0, 0],
                ),
            ]),
        );
        let mut interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        interpreter.ui_effects = Some(&events);
        assert_eq!(
            interpreter.run_tree(&mut fx.world, LAMP, 4096, 0, &[], 1000),
            Ok(true)
        );
        assert_eq!(
            *events.borrow(),
            vec![
                UiEffectEvent::Portrait {
                    guid: WIDGET,
                    highlight: false
                },
                UiEffectEvent::Relationship {
                    guid: WIDGET,
                    highlight: true
                },
                UiEffectEvent::Relationship {
                    guid: WIDGET,
                    highlight: false
                },
            ]
        );

        // Mode 1 when 0x4e92a8 fails (no HEAD binding): must NOT emit Portrait.
        let (mut fx2, _, rec2) = visitors();
        fx2.program.people.portrait_head.insert(WIDGET, false);
        let [lo2, hi2] = rec2.to_le_bytes();
        let events2 = RefCell::new(Vec::new());
        fx2.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                UI_EFFECT,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, scope::LITERAL, lo2, hi2, 1, 0, 0, 0],
            )]),
        );
        let mut interp2 = sims_vm::Interpreter::new(&fx2.program, &fx2.catalog, fx2.stream);
        interp2.ui_effects = Some(&events2);
        assert_eq!(
            interp2.run_tree(&mut fx2.world, LAMP, 4096, 0, &[], 1000),
            Ok(true)
        );
        assert_eq!(
            *events2.borrow(),
            vec![UiEffectEvent::Button {
                id: rec2,
                highlight: true
            }]
        );
    }

    // 2. P2 #2: Lazy suits preserve the required mesh bones from whichever
    // `.bcf` catalog loaded first at runtime (`0x59f34c..0x59f35f`), rather
    // than overwriting or conflating them globally across catalogs.
    let bone = |name: &str, parent: Option<usize>| sims_content::Bone {
        name: name.into(),
        parent,
        translation: [0; 3],
        rotation: [0, 0, 0, 1.0f32.to_bits()],
        blends: false,
    };
    for load_a_first in [false, true] {
        let (mut fx, _, widget_rec) = visitors();
        fx.words(SIM_FILE)[102] = 0;
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 0;
        let sim_rec = fx.world.neighbors.slots.len() as i16 + 1;
        fx.world.neighbors.slots.push(Some(Neighbor {
            guid: SIM_GUID,
            id: sim_rec,
            data: vec![0; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 0,
            stale: false,
        }));
        let bones = vec![bone("PELVIS", None), bone("HEAD", Some(0))];
        let scales = vec![0i32; bones.len()];
        fx.program.people.skeletons.insert(
            SIM_GUID,
            sims_content::Skeleton {
                name: "adult".into(),
                file: "adult.bcf".into(),
                place: 0,
                scales: vec![scales],
                bones,
            },
        );
        fx.program
            .people
            .indexed_suits
            .insert("trigger_a".into(), Ok("GameData/Skins/a.bcf".into()));
        fx.program
            .people
            .indexed_suits
            .insert("dup_head".into(), Ok("GameData/Skins/b.bcf".into()));
        fx.program.people.indexed_suit_catalogs.insert(
            "GameData/Skins/a.bcf".into(),
            (
                vec![("trigger_a".into(), false), ("dup_head".into(), true)],
                None,
            ),
        );
        fx.program.people.indexed_suit_head_bones.insert(
            "GameData/Skins/a.bcf".into(),
            BTreeMap::from([("dup_head".into(), vec![vec!["HEAD".into()]])]),
        );
        fx.program.people.indexed_suit_catalogs.insert(
            "GameData/Skins/b.bcf".into(),
            (vec![("dup_head".into(), true)], None),
        );
        fx.program.people.indexed_suit_head_bones.insert(
            "GameData/Skins/b.bcf".into(),
            BTreeMap::from([(
                "dup_head".into(),
                vec![vec!["HEAD".into(), "MISSING_BONE".into()]],
            )]),
        );
        fx.program.people.profiles.insert(
            WIDGET,
            sims_content::Profile {
                body: Some("trigger_a,tex".into()),
                ..Default::default()
            },
        );
        fx.program.people.profiles.insert(
            SIM_GUID,
            sims_content::Profile {
                head: Some("dup_head,tex".into()),
                ..Default::default()
            },
        );
        if load_a_first {
            // Trigger a.bcf to load at runtime via WIDGET's body before SIM
            // resolves dup_head (matching 0x59f34c..0x59f35f first-load wins).
            assert_eq!(
                run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, widget_rec, 1),
                Ok(true)
            );
            assert_eq!(
                fx.world.loaded_indexed_suit_bones.get("dup_head"),
                Some(&vec![vec!["HEAD".to_owned()]])
            );
        }
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_rec, 1),
            Ok(true)
        );
        assert_eq!(
            fx.world.portraits.contains(&SIM_GUID),
            load_a_first,
            "duplicate lazy suit must use first-loaded catalog's mesh bones \
             (load_a_first={load_a_first})"
        );
    }

    // 3. P2 #3 (`0x4e733b..0x4e7411`): portrait generation runs 3 forced
    // dressing passes. If pass 0 misses `sib_head` (not in `indexed_suits`)
    // and then binds `lazy_body` which loads `body.bcf` registering `sib_head`
    // as a sibling, pass 0 fails (`[ebp - 0xd] = 0`, so the first call does
    // not cache the portrait), while pass 1 attaches `sib_head` into
    // `person_outfits[SIM].head` so a second call succeeds on all 3 passes.
    {
        let (mut fx, _, _) = visitors();
        fx.words(SIM_FILE)[102] = 0;
        fx.world
            .objects
            .get_mut(&SIM)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[8] = 0;
        let sim_rec = fx.world.neighbors.slots.len() as i16 + 1;
        fx.world.neighbors.slots.push(Some(Neighbor {
            guid: SIM_GUID,
            id: sim_rec,
            data: vec![0; 88],
            relationships: Vec::new(),
            friends: 0,
            revision: 0,
            stale: false,
        }));
        fx.program
            .people
            .indexed_suits
            .insert("lazy_body".into(), Ok("GameData/Skins/body.bcf".into()));
        fx.program.people.indexed_suit_catalogs.insert(
            "GameData/Skins/body.bcf".into(),
            (
                vec![("lazy_body".into(), false), ("sib_head".into(), true)],
                None,
            ),
        );
        fx.program.people.profiles.insert(
            SIM_GUID,
            sims_content::Profile {
                body: Some("lazy_body,tex".into()),
                head: Some("sib_head,tex".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_rec, 1),
            Ok(true)
        );
        assert!(
            !fx.world.portraits.contains(&SIM_GUID),
            "pass 0 missed sib_head so 0x4e73a5 cleared [ebp - 0xd] on first call"
        );
        assert_eq!(
            fx.world
                .person_outfits
                .get(&SIM)
                .and_then(|o| o.head.as_ref())
                .map(|b| b.suit.as_str()),
            Some("sib_head"),
            "pass 1 (0x4e733b..0x4e7411) must attach sibling head loaded by pass 0's body"
        );
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, sim_rec, 1),
            Ok(true)
        );
        assert!(
            fx.world.portraits.contains(&SIM_GUID),
            "second call must succeed across all 3 passes now that sib_head is bound"
        );
    }
}
