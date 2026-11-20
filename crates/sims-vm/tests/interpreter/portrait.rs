//! A dialog's portrait (0x43601c, 0x436279, 0x4e92a8, 0x4e8c5d,
//! 0x4e9652): the Dialog primitive (opcode 36) has the interface build
//! the portrait of the person type its icon mode names, which makes a
//! person of that type and removes it at once when nobody of it is on the
//! lot (`Machine::dialog_portrait`). Synthetic; no game data.

use crate::create_person::visitors;
use crate::factory::WIDGET;
use crate::support::*;

/// Dialog (opcode 36).
const DIALOG: u16 = 36;

/// Runs, as the lamp's tree 4096 with stack object `stack_object`, a
/// one-button dialog that only informs (operand 7 bit 0) with icon mode
/// `mode` (operand 7 bits 1 to 3).
fn inform(fx: &mut Fixture, mode: u8, stack_object: i16) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            DIALOG,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 0, 0, 0, 0, 0, 1 | mode << 1],
        )]),
    );
    fx.run_with(LAMP, 4096, stack_object, &[], 1000)
}

/// The lot without its dialog records, portraits and frame storage: what
/// a dialog that builds no portrait leaves as it was.
fn rest(fx: &Fixture) -> [u8; 32] {
    let mut world = fx.world.clone();
    world.dialogs.records.clear();
    world.portraits.clear();
    hash_without_frames(&world)
}

/// [`rest`] after the same dialog run with icon mode 1, which shows no
/// portrait, on a copy of the lot.
fn as_with_no_portrait(fx: &Fixture, stack_object: i16) -> [u8; 32] {
    let mut copy = Fixture {
        program: fx.program.clone(),
        catalog: fx.catalog.clone(),
        world: fx.world.clone(),
        stream: fx.stream,
    };
    assert_eq!(inform(&mut copy, 1, stack_object), Ok(true));
    rest(&copy)
}

#[test]
fn a_neighbors_portrait_makes_a_person_of_its_type_and_removes_it_once() {
    // Mode 2: the type of the neighbor record the stack object names
    // (0x425afe, record +8). Nobody of it is on the lot, so a person is
    // made and removed at once (0x4e9652, 0x4e8fdb).
    let (mut fx, _, record) = visitors();
    let objects = fx.world.objects.clone();
    let before = as_with_no_portrait(&fx, record);
    assert_eq!(inform(&mut fx, 2, record), Ok(true));
    assert_eq!(fx.world.portraits, [WIDGET].into());
    assert_eq!(
        fx.world.objects.keys().collect::<Vec<_>>(),
        objects.keys().collect::<Vec<_>>()
    );
    assert!(
        fx.world.skeleton_files.contains("widget.bcf"),
        "the person made read its skeleton"
    );
    assert_ne!(rest(&fx), before);
    // The type has its image now: a second dialog builds nothing.
    fx.world.dialogs.records.clear();
    let after = rest(&fx);
    assert_eq!(inform(&mut fx, 2, record), Ok(true));
    assert_eq!(rest(&fx), after);
}

#[test]
fn a_portrait_of_a_type_someone_on_the_lot_has_makes_nobody() {
    // 0x4e8c5d draws the first person of the type on the lot (0x4e9608).
    // Mode 0: the stack object's own type.
    let mut fx = Fixture::new();
    fx.world.people = vec![SIM];
    let before = as_with_no_portrait(&fx, SIM as i16);
    assert_eq!(inform(&mut fx, 0, SIM as i16), Ok(true));
    assert_eq!(fx.world.portraits, [SIM_GUID].into());
    assert_eq!(rest(&fx), before);
}

#[test]
fn modes_with_no_person_build_no_portrait() {
    // Mode 0 on an object that is not a person, and a stack object that
    // names nothing; mode 2 with no record.
    let (fx, _, record) = visitors();
    for (mode, stack_object) in [(0, LAMP as i16), (0, 999), (2, record + 1)] {
        let mut fx = Fixture {
            program: fx.program.clone(),
            catalog: fx.catalog.clone(),
            world: fx.world.clone(),
            stream: fx.stream,
        };
        let before = as_with_no_portrait(&fx, stack_object);
        assert_eq!(inform(&mut fx, mode, stack_object), Ok(true), "{mode}");
        assert!(fx.world.portraits.is_empty());
        assert_eq!(rest(&fx), before, "{mode} {stack_object}");
    }
}

#[test]
fn a_portrait_the_port_cannot_tell_was_built_fails_loudly() {
    // A type of the household with nobody of it on the lot: the household
    // panel may have built it (not kept).
    let (mut fx, _, record) = visitors();
    fx.world.globals[9] = 7;
    fx.world.families.push(sims_sim::neighbors::Family {
        id: 7,
        house: 0,
        members: vec![WIDGET],
        friends: 0,
        stale: false,
    });
    let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
    assert!(
        fault.contains("0x4e9652") && fault.contains("household"),
        "{fault}"
    );
    assert!(fx.world.portraits.is_empty());
    // A type made while the game ran may not be in the interface's table.
    let (mut fx, _, record) = visitors();
    fx.world.characters.made.insert(
        WIDGET,
        sims_sim::characters::Character {
            template: WIDGET,
            file: 0,
            strings: Default::default(),
        },
    );
    let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
    assert!(fault.contains("0x6f2698"), "{fault}");
}

#[test]
fn a_portrait_whose_person_runs_code_when_put_down_fails_loudly() {
    // The person made is put on a tile (0x45e4e9, 0x433340), which asks
    // it its function 9 (0x433580), and its function 6 when the walls
    // beside the tile differ from its own (0x434fac, 0x435110). The tile
    // is not modeled.
    for (function, at) in [(9, "0x433580"), (6, "0x435110"), (26, "0x433612")] {
        let (mut fx, file, record) = visitors();
        let library = fx.program.libraries.get_mut(&file).unwrap();
        for table in library.callbacks.values_mut() {
            table.resize(
                27,
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: 0,
                },
            );
            table[function].action = 4200;
        }
        let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
        assert!(fault.contains(at) && fault.contains("tree 4200"), "{fault}");
        assert!(fx.world.portraits.is_empty());
    }
}

#[test]
fn a_portrait_whose_put_depends_on_its_tile_fails_loudly() {
    // Data word 40 bit 0x400 marks the room the person lands in as not
    // counted (0x433340).
    let (mut fx, file, record) = visitors();
    // The widget's Init (tree 4096, `crate::find_location::placeable`),
    // with the bit set.
    fx.add(
        file,
        4096,
        tree(vec![
            assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
            assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
            assign(scope::MY_DATA, 40, scope::LITERAL, 0x400, RETURN_TRUE),
        ]),
    );
    let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
    assert!(fault.contains("bit 0x400"), "{fault}");
    assert!(fx.world.portraits.is_empty());
    // A person of kind 0 or 1 marks the indoor room it lands in as
    // occupied (0x45e43d), which its removal leaves: it fails while an
    // indoor room that is not a pool is not marked. One already marked,
    // one outdoors and a pool are left as they are, and so is any room by
    // a pet (kind above 1). The person's words come from its neighbor
    // record (`crate::person_factory::equip`).
    for (kind, outdoors, pool, occupied, fails) in [
        (0, false, false, false, true),
        (1, false, false, false, true),
        (2, false, false, false, false),
        (0, false, false, true, false),
        (0, true, false, false, false),
        (0, false, true, false, false),
    ] {
        let (mut fx, _, record) = visitors();
        fx.world.rooms.rooms.clear();
        fx.world.neighbors.slots[record as usize - 1]
            .as_mut()
            .unwrap()
            .data[65] = kind;
        fx.world.rooms.rooms.insert(
            1,
            sims_sim::rooms::Room {
                outdoors,
                pool,
                occupied,
                ..Default::default()
            },
        );
        let ran = inform(&mut fx, 2, record);
        if fails {
            let fault = ran.unwrap_err().to_string();
            assert!(fault.contains("0x468802"), "{kind}: {fault}");
        } else {
            assert_eq!(ran, Ok(true), "{kind} {outdoors} {pool} {occupied}");
        }
    }
    // No tile takes it: the original removes it unput and builds again
    // (0x4e9706, 0x4e938c).
    let (mut fx, _, record) = visitors();
    fx.world.size = 2;
    let fault = inform(&mut fx, 2, record).unwrap_err().to_string();
    assert!(fault.contains("0x4e9706"), "{fault}");
}

#[test]
fn a_dialogs_text_can_show_the_stack_objects_neighbor_instead() {
    // Mode 0 shows first the type the formatter (0x44bf1a) found in the
    // record's strings, formatted in the order button (operand 3), title
    // (operand 6), message (operand 2): `$Neighbor` gives the stack
    // object's neighbor record's type, `$Object` the stack object's, and
    // the last found is kept. With neither, the stack object's (0x436308).
    let strings = ["Hello", "$Neighbor", "$Object", "$Object then $Neighbor"];
    for ((label, title, message), shown) in [
        ((0, 0, 1), SIM_GUID),
        ((0, 0, 2), WIDGET),
        ((2, 3, 0), SIM_GUID),
        ((3, 0, 2), WIDGET),
        ((0, 0, 4), WIDGET),
    ] {
        // The person (2) is the stack object, and neighbor record 2 is of
        // the widget's type, with nobody of it on the lot.
        let (mut fx, _, record) = visitors();
        let neighbor = fx.world.neighbors.slots[record as usize - 1].clone();
        let slots = &mut fx.world.neighbors.slots;
        slots.resize(slots.len().max(2), None);
        slots[usize::from(SIM) - 1] = neighbor;
        fx.world.people = vec![SIM];
        fx.program
            .libraries
            .get_mut(&LAMP_FILE)
            .unwrap()
            .strings
            .insert(301, strings.map(str::to_owned).to_vec());
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                DIALOG,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 0, message, label, 0, 0, title, 1],
            )]),
        );
        let ran = fx.run_with(LAMP, 4096, SIM as i16, &[], 1000);
        assert_eq!(ran, Ok(true), "{label} {title} {message}");
        assert_eq!(
            fx.world.portraits,
            [shown].into(),
            "{label} {title} {message}"
        );
    }
}

#[test]
fn a_neighbor_local_named_in_a_dialogs_text_is_not_its_neighbor() {
    // The formatter's `$NeighborLocal` pass (0x44c590 to 0x44c65a) comes
    // before the `$Neighbor` pass, which finds `$Neighbor` anywhere in the
    // text as it is left. A token whose local names a record of a person
    // type is replaced by that type's name, so `$Neighbor` is not found and
    // mode 0 shows the stack object's type. One whose local names no record,
    // or is past the tree's locals, stays, and `$Neighbor` is found in it.
    // A record of a type not in the person types' table ends the pass
    // (0x44c616), leaving the tokens after it too.
    let strings = [
        "Hi $NeighborLocal:0",
        "$NeighborLocal:9",
        "$NeighborLocal:1 $NeighborLocal:0",
        "$NeighborLocal:0 and $Neighbor",
        "$NeighborLocal:4294967297",
        "$NeighborLocal:4294967296",
    ];
    for (message, locals, shown) in [
        (1, [0, 0], Some(WIDGET)),
        (2, [-1, -1], Some(WIDGET)),
        (3, [-1, -2], Some(WIDGET)),
        (4, [-1, 0], Some(WIDGET)),
        (5, [-1, 0], Some(WIDGET)),
        (1, [-1, 0], None),
        (3, [-1, -1], None),
        (6, [-1, 0], None),
    ] {
        // The person (2) is the stack object, and neighbor record 2 is of
        // the widget's type, with nobody of it on the lot. Local `n` holds
        // `locals[n]`, where -1 is the widget's record and -2 a record of
        // the lamp's type, which is not a person.
        let (mut fx, _, record) = visitors();
        let neighbor = fx.world.neighbors.slots[record as usize - 1].clone();
        let mut lamp = neighbor.clone().unwrap();
        lamp.guid = LAMP_GUID;
        let slots = &mut fx.world.neighbors.slots;
        slots.resize(slots.len().max(2), None);
        slots[usize::from(SIM) - 1] = neighbor;
        slots.push(Some(lamp));
        let lamp_record = slots.len() as i16;
        fx.world.people = vec![SIM];
        fx.program
            .libraries
            .get_mut(&LAMP_FILE)
            .unwrap()
            .strings
            .insert(301, strings.map(str::to_owned).to_vec());
        let local = |value: i16| match value {
            -1 => record,
            -2 => lamp_record,
            v => v,
        };
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![
                assign(scope::LOCALS, 0, scope::LITERAL, local(locals[0]), 1),
                assign(scope::LOCALS, 1, scope::LITERAL, local(locals[1]), 2),
                ins(
                    DIALOG,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [0, 0, message, 0, 0, 0, 0, 1],
                ),
            ]),
        );
        let ran = fx.run_with(LAMP, 4096, SIM as i16, &[], 1000);
        match shown {
            Some(shown) => {
                assert_eq!(ran, Ok(true), "{message} {locals:?}");
                assert_eq!(fx.world.portraits, [shown].into(), "{message} {locals:?}");
            }
            None => {
                let fault = ran.unwrap_err().to_string();
                assert!(
                    fault.contains("0x4e893e") && fault.contains("0x44c65f"),
                    "{message} {locals:?}: {fault}"
                );
                assert!(fx.world.portraits.is_empty());
            }
        }
    }
}

#[test]
fn a_neighbor_local_the_original_left_unknown_fails_loudly() {
    // The pass reads the local (0x44c5f4) as any read of a local does: one
    // that starts as half of a pointer left in the frame storage fails.
    let (mut fx, _, record) = visitors();
    let neighbor = fx.world.neighbors.slots[record as usize - 1].clone();
    let slots = &mut fx.world.neighbors.slots;
    slots.resize(slots.len().max(2), None);
    slots[usize::from(SIM) - 1] = neighbor;
    fx.world.people = vec![SIM];
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .strings
        .insert(301, vec!["$NeighborLocal:1".to_owned()]);
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            DIALOG,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 1, 0, 0, 0, 0, 1],
        )]),
    );
    fx.world.frame_storage.insert(
        LAMP,
        sims_sim::FrameStorage {
            unknown: [(0, 1)].into(),
            ..Default::default()
        },
    );
    let fault = fx
        .run_with(LAMP, 4096, SIM as i16, &[], 1000)
        .unwrap_err()
        .to_string();
    assert!(
        fault.contains("0x44c5f4") && fault.contains("half of a pointer"),
        "{fault}"
    );
}
