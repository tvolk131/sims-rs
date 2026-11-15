//! Create New Object Instance (opcode 42, 0x44e0c8) mode 4, which puts the
//! new object into the stack object's first container slot, and flag 0x1,
//! which makes nothing where an object of the same type already stands.
//!
//! The lot is Find Location For's: the six-tile one of the room tests,
//! room 1 the four tiles (2, 2) to (3, 3), with no floors, where the
//! widget may stand without one. The lamp, on tile (2, 2) in room 1, is
//! the container: it has two container slots at its centre, at height 1,
//! both empty.

use crate::factory::WIDGET;
use crate::find_location::{create as create_at_local, placeable};
use crate::support::*;
use sims_formats::execution::Slot;
use sims_vm::Trap;

const CREATE: u16 = 42;
/// The lamp's SLOT chunk.
const SLOTS: u16 = 300;
const NOT_BESIDE_ITS_TYPE: u8 = 0x1;

/// A container slot at the object's centre, at height 1.
fn container_slot() -> Slot {
    Slot {
        kind: 0,
        offset: [0.0; 3],
        standing: 0,
        sitting: 0,
        ground: 0,
        flags: 0,
        snap_target: -1,
        proximity: [16; 3],
        max_size: 0,
        unknown: 0,
        gradient: 0.1875,
        height: 0,
        facing: -2,
        resolution: 16,
    }
}

/// Find Location For's lot and widget, with the lamp a container on tile
/// (2, 2) in room 1, and the widget's file.
fn container() -> (Fixture, usize) {
    let (mut fx, file) = placeable();
    fx.words(LAMP_FILE)[20] = SLOTS as i16;
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .slots
        .insert(SLOTS, vec![container_slot(), container_slot()]);
    let lamp = fx.lamp_mut();
    lamp.position = [40, 40, 1];
    lamp.data[29] = 1;
    // The fixture's lamp holds objects in its script slots; empty them.
    for held in &mut lamp.slots[1..] {
        held.object = 0;
    }
    (fx, file)
}

/// Runs the lamp's tree 4096, whose one instruction is Create New Object
/// Instance of the widget in mode `mode` with `flags`, with the stack
/// object `stack_object`.
fn create(
    fx: &mut Fixture,
    mode: u8,
    flags: u8,
    stack_object: i16,
) -> Result<bool, sims_vm::Fault> {
    let [g0, g1, g2, g3] = WIDGET.to_le_bytes();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            CREATE,
            RETURN_TRUE,
            RETURN_FALSE,
            [g0, g1, g2, g3, mode, flags, 0, 0],
        )]),
    );
    fx.run_with(LAMP, 4096, stack_object, &[], 1000)
}

/// The objects there are, in ID order.
fn ids(fx: &Fixture) -> Vec<u16> {
    fx.world.objects.keys().copied().collect()
}

/// The object made since `before` was listed.
fn new_object(fx: &Fixture, before: &[u16]) -> u16 {
    let made: Vec<u16> = ids(fx)
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect();
    assert_eq!(made.len(), 1, "{made:?}");
    made[0]
}

#[test]
fn mode_4_puts_the_new_object_in_the_stack_objects_first_container_slot() {
    let (mut fx, _) = container();
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 4, 0, LAMP as i16), Ok(true));
    let id = new_object(&fx, &before);
    let widget = &fx.world.objects[&id];
    assert_eq!(widget.guid, WIDGET);
    // At the lamp's position, in its script slot 0 (saved slot 1), which
    // data words 2 and 3 name; the link words name the lamp and slot 1.
    assert_eq!(widget.position, [40, 40, 1]);
    assert_eq!((widget.data[2], widget.data[3]), (LAMP as i16, 0));
    assert_eq!((widget.data[14], widget.data[26]), (1, LAMP as i16));
    let held: Vec<i16> = fx.lamp().slots[1..].iter().map(|s| s.object).collect();
    assert_eq!(held, [id as i16, 0]);
}

#[test]
fn mode_4_makes_the_new_object_the_stack_object() {
    let (mut fx, _) = container();
    let before = ids(&fx);
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            ins(CREATE, 1, RETURN_FALSE, {
                let [g0, g1, g2, g3] = WIDGET.to_le_bytes();
                [g0, g1, g2, g3, 4, 0, 0, 0]
            }),
            // The stack object's data word 2 is its container.
            assign(scope::MY_DATA, 0, scope::STACK_DATA, 2, RETURN_TRUE),
        ]),
    );
    assert_eq!(fx.run_with(LAMP, 4096, LAMP as i16, &[], 1000), Ok(true));
    new_object(&fx, &before);
    assert_eq!(fx.lamp().data[0], LAMP as i16);
}

#[test]
fn mode_4_without_a_stack_object_makes_nothing() {
    let (mut fx, _) = container();
    let before = ids(&fx);
    for stack_object in [0, -1, 77] {
        assert_eq!(create(&mut fx, 4, 0, stack_object), Ok(false));
        assert_eq!(ids(&fx), before);
    }
}

#[test]
fn mode_4_makes_nothing_off_the_lot_or_in_another_room() {
    let (mut fx, _) = container();
    let before = ids(&fx);
    fx.lamp_mut().position = [-16, -16, 1];
    assert_eq!(create(&mut fx, 4, 0, LAMP as i16), Ok(false));
    assert_eq!(ids(&fx), before);
    // Tile (2, 2) is in room 1, not 0.
    let (mut fx, _) = container();
    fx.lamp_mut().data[29] = 0;
    assert_eq!(create(&mut fx, 4, 0, LAMP as i16), Ok(false));
    assert_eq!(ids(&fx), before);
    // Tile (1, 1) is in room 0.
    let (mut fx, _) = container();
    fx.lamp_mut().position = [24, 24, 1];
    assert_eq!(create(&mut fx, 4, 0, LAMP as i16), Ok(false));
    fx.lamp_mut().data[29] = 0;
    assert_eq!(create(&mut fx, 4, 0, LAMP as i16), Ok(true));
    // A container in room -5 skips the test.
    let (mut fx, _) = container();
    fx.lamp_mut().data[29] = -5;
    assert_eq!(create(&mut fx, 4, 0, LAMP as i16), Ok(true));
}

#[test]
fn mode_4_removes_an_object_the_slot_refuses() {
    let refusals: [fn(&mut Fixture); 3] = [
        // The slot holds another object.
        |fx| fx.lamp_mut().slots[1].object = SIM as i16,
        // The lamp takes less than the widget weighs (data word 27, 0).
        |fx| fx.lamp_mut().data[28] = -1,
        // The lamp is not solid (data word 8 flag 4).
        |fx| fx.lamp_mut().data[8] = 4,
    ];
    for (case, refuse) in refusals.into_iter().enumerate() {
        let (mut fx, _) = container();
        refuse(&mut fx);
        let before = ids(&fx);
        let slots: Vec<i16> = fx.lamp().slots.iter().map(|s| s.object).collect();
        assert_eq!(create(&mut fx, 4, 0, LAMP as i16), Ok(false), "case {case}");
        assert_eq!(ids(&fx), before, "case {case}");
        let after: Vec<i16> = fx.lamp().slots.iter().map(|s| s.object).collect();
        assert_eq!(after, slots, "case {case}");
    }
}

#[test]
fn flag_0x1_makes_nothing_where_an_object_of_the_same_type_stands() {
    // A widget made by mode 8 stands on the lamp's tile.
    let (mut fx, _) = container();
    let before = ids(&fx);
    assert_eq!(create_at_local(&mut fx, 0, 0, SIM as i16), Ok(true));
    let standing = new_object(&fx, &before);
    let placed = fx.world.clone();
    let before = ids(&fx);
    // Mode 4 with flag 0x1 makes nothing; without it, the widget goes into
    // the lamp.
    assert_eq!(
        create(&mut fx, 4, NOT_BESIDE_ITS_TYPE, LAMP as i16),
        Ok(false)
    );
    assert_eq!(ids(&fx), before);
    assert_eq!(create(&mut fx, 4, 0, LAMP as i16), Ok(true));
    // An object of another type on the tile is no matter.
    fx.world = placed.clone();
    fx.world.objects.get_mut(&standing).unwrap().guid = LAMP_GUID;
    assert_eq!(
        create(&mut fx, 4, NOT_BESIDE_ITS_TYPE, LAMP as i16),
        Ok(true)
    );
    // Mode 8 tests the same tile. The standing widget is not solid, so
    // without the flag a second one fits beside it.
    fx.world = placed.clone();
    fx.world.objects.get_mut(&standing).unwrap().data[8] = 4;
    let before = ids(&fx);
    assert_eq!(
        create_at_local(&mut fx, 0, NOT_BESIDE_ITS_TYPE, SIM as i16),
        Ok(false)
    );
    assert_eq!(ids(&fx), before);
    assert_eq!(create_at_local(&mut fx, 0, 0, SIM as i16), Ok(true));
    // Mode 6 leaves its object off the lot's arrays, where nothing is
    // found.
    fx.world = placed;
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 6, NOT_BESIDE_ITS_TYPE, 0), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].position, [-16, -16, 1]);
}

#[test]
fn flag_0x1_finds_an_object_held_by_one_on_the_tile() {
    // An object of another type, not solid, stands on the lamp's tile, and
    // a widget waits off the lot.
    let (mut fx, _) = container();
    let before = ids(&fx);
    assert_eq!(create_at_local(&mut fx, 0, 0, SIM as i16), Ok(true));
    let standing = new_object(&fx, &before);
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 6, 0, 0), Ok(true));
    let held = new_object(&fx, &before);
    let object = fx.world.objects.get_mut(&standing).unwrap();
    object.guid = LAMP_GUID;
    object.data[8] = 4;
    let placed = fx.world.clone();
    // The widget goes into the standing object's slot 1: the walk of the
    // tile reaches it.
    let object = fx.world.objects.get_mut(&standing).unwrap();
    object.slots.resize(2, object.slots[0].clone());
    object.slots[1].object = held as i16;
    let widget = fx.world.objects.get_mut(&held).unwrap();
    widget.data[14] = 1;
    widget.data[26] = standing as i16;
    let before = ids(&fx);
    assert_eq!(
        create_at_local(&mut fx, 0, NOT_BESIDE_ITS_TYPE, SIM as i16),
        Ok(false)
    );
    assert_eq!(ids(&fx), before);
    // Held by nothing on the tile, it is not found.
    fx.world = placed;
    assert_eq!(
        create_at_local(&mut fx, 0, NOT_BESIDE_ITS_TYPE, SIM as i16),
        Ok(true)
    );
}

#[test]
fn what_mode_4_does_not_port_fails_loudly() {
    // Flags other than 0x1.
    for flags in [0x2, 0x4, 0x8, 0x10, 0x20, 0x40, 0x21] {
        let (mut fx, _) = container();
        let result = create(&mut fx, 4, flags, LAMP as i16);
        assert!(
            matches!(trap(result), Trap::Unsupported(ref m) if m.contains("mode 4")),
            "flags {flags:#x}"
        );
    }
    // A person type, and a type that belongs to a group: nothing is made.
    for (word, value, text) in [(9, 2, "making a person"), (10, 1, "group")] {
        let (mut fx, file) = container();
        fx.words(file)[word] = value;
        let before = ids(&fx);
        let result = create(&mut fx, 4, 0, LAMP as i16);
        assert!(
            matches!(trap(result), Trap::Unsupported(ref m) if m.contains(text)),
            "word {word}"
        );
        assert_eq!(ids(&fx), before, "word {word}");
    }
    // A person as the container: its slots are not ported.
    let (mut fx, _) = container();
    fx.sim_mut().position = [40, 40, 1];
    fx.sim_mut().data[29] = 1;
    let before = ids(&fx);
    let result = create(&mut fx, 4, 0, SIM as i16);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x459630")));
    assert_eq!(ids(&fx), before);
    // The modes not ported.
    for mode in [0, 1, 2, 3, 5, 7, 9, 15] {
        let (mut fx, _) = container();
        let result = create(&mut fx, mode, 0, LAMP as i16);
        assert!(
            matches!(trap(result), Trap::Unsupported(ref m) if m.contains(&format!("mode {mode} "))),
            "mode {mode}"
        );
    }
}
