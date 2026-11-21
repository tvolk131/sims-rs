//! UI Effect (opcode 34, 0x44acff): interface highlight control for the
//! Tutorial and Help System, including mode 0's fallthrough into mode 1
//! (`0x47689c` → `0x4e92a8` portrait build) and mode 2's read-only
//! relationship highlight (`0x476984`). Synthetic; no game data.

use crate::create_person::visitors;
use crate::factory::WIDGET;
use crate::support::*;
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
    // and on both non-person (LAMP) and person (SIM) callers.
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

    // A neighbor record whose GUID is not a person type (or has no definition)
    // is not in the portrait table (0x4e9753 returns -1) -> no-op, true.
    let (mut fx, _, _) = visitors();
    fx.world.neighbors.slots.push(Some(Neighbor {
        guid: 0xDEAD_BEEF,
        id: 50,
        data: vec![0; 88],
        relationships: Vec::new(),
        friends: 0,
        revision: 0,
        stale: false,
    }));
    let unknown_record = fx.world.neighbors.slots.len() as i16;
    assert_eq!(
        run_ui_effect(
            &mut fx,
            LAMP,
            LAMP_FILE,
            1,
            scope::LITERAL,
            unknown_record,
            1
        ),
        Ok(true)
    );
    assert!(fx.world.portraits.is_empty());
}

#[test]
fn mode_1_uses_on_lot_person_and_guards_household_or_runtime_made_characters() {
    // When a person of that type is already on the lot, no temporary person is
    // constructed (skeleton_files stays empty), and the portrait is marked.
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

    // An off-lot member of the active household fails loudly (same 0x4e9652
    // guard as Dialog's portrait).
    let (mut fx, _, record) = visitors();
    fx.world.globals[9] = 4;
    fx.world.families.push(Family {
        id: 4,
        house: 5,
        members: vec![WIDGET],
        friends: 0,
        stale: false,
    });
    let result = run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x4e9652")));

    // A runtime-made character (world.characters.made) fails loudly.
    let (mut fx, _, record) = visitors();
    fx.world.characters.made.insert(
        WIDGET,
        sims_sim::characters::Character {
            template: WIDGET,
            file: 0,
            strings: Default::default(),
        },
    );
    let result = run_ui_effect(&mut fx, LAMP, LAMP_FILE, 1, scope::LITERAL, record, 1);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x6f2698")));
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
fn modes_0_to_2_validate_variable_reads_while_modes_3_and_up_skip_them() {
    // Modes 0, 1, and 2 call 0x453141, raising native error 9 on an invalid
    // scope (> 38) and native error 8 on a missing local.
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
    }

    // Modes 3 and up (0x44ad1d jne 0x44ad83) skip 0x453141 completely and
    // return true even with an invalid scope.
    for mode in [3u8, 4, 128, 255] {
        let mut fx = Fixture::new();
        assert_eq!(
            run_ui_effect(&mut fx, LAMP, LAMP_FILE, mode, 99, 0, 1),
            Ok(true),
            "mode {mode}"
        );
    }
}
