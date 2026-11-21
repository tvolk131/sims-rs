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
/// The widget's Main tree, and the global tree that sleeps (`factory`).
const MAIN: u16 = 4097;
const SLEEP: u16 = 256;

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
pub fn container() -> (Fixture, usize) {
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
fn mode_4_into_a_person_uses_the_slot_it_carries_in() {
    // The Sim, on the lamp's tile in live mode, has the tile slot and the
    // five a person adds (0x45962b); its slot 0 is at height 6.
    let person = |fx: &mut Fixture| {
        fx.world.globals[19] = 0;
        let sim = fx.sim_mut();
        sim.position = [40, 40, 1];
        sim.data[29] = 1;
        for held in &mut sim.slots {
            held.object = 0;
        }
        let tile = sim.slots[0].clone();
        sim.slots.resize(6, tile);
    };
    // The widget's Init lets it go at height 1 only, so the slot refuses
    // it and it is removed.
    let (mut fx, _) = container();
    person(&mut fx);
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 4, 0, SIM as i16), Ok(false));
    assert_eq!(ids(&fx), before);
    // With bit 5 of data word 4 too, it goes in.
    let (mut fx, file) = container();
    person(&mut fx);
    fx.add(
        file,
        4096,
        tree(vec![
            assign(scope::MY_DATA, 4, scope::LITERAL, 0x21, 1),
            assign(scope::MY_DATA, 42, scope::LITERAL, 2, RETURN_TRUE),
        ]),
    );
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 4, 0, SIM as i16), Ok(true));
    let id = new_object(&fx, &before);
    let held: Vec<i16> = fx.sim().slots[1..].iter().map(|s| s.object).collect();
    assert_eq!(held, [id as i16, 0, 0, 0, 0]);
    assert_eq!(fx.world.objects[&id].data[2], SIM as i16);
}

#[test]
fn mode_2_puts_the_new_object_in_the_running_objects_first_container_slot() {
    // 0x44e22f: the running object (the lamp) is the container, with no
    // stack object needed; the room filter stays at -5 (0x44e13c), so a
    // container whose room word names another room still takes it.
    for room in [1, 0] {
        let (mut fx, _) = container();
        fx.lamp_mut().data[29] = room;
        let before = ids(&fx);
        assert_eq!(create(&mut fx, 2, 0, 0), Ok(true), "room {room}");
        let id = new_object(&fx, &before);
        let widget = &fx.world.objects[&id];
        assert_eq!(widget.position, [40, 40, 1]);
        assert_eq!((widget.data[2], widget.data[3]), (LAMP as i16, 0));
        let held: Vec<i16> = fx.lamp().slots[1..].iter().map(|s| s.object).collect();
        assert_eq!(held, [id as i16, 0]);
    }
    // Off the lot, nothing is made.
    let (mut fx, _) = container();
    fx.lamp_mut().position = [-16, -16, 1];
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 2, 0, 0), Ok(false));
    assert_eq!(ids(&fx), before);
}

#[test]
fn mode_5_puts_the_new_object_where_the_running_object_stands() {
    // 0x44e3e9: the running object's position and level, then as mode 8:
    // on the floor, in no container.
    let (mut fx, _) = container();
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 5, 0, 0), Ok(true));
    let id = new_object(&fx, &before);
    let widget = &fx.world.objects[&id];
    assert_eq!(widget.position, [40, 40, 1]);
    assert_eq!(widget.data[2], 0);
    assert!(fx.lamp().slots[1..].iter().all(|s| s.object == 0));
    // Off the lot, nothing is made.
    let (mut fx, _) = container();
    fx.lamp_mut().position = [-16, -16, 1];
    assert_eq!(create(&mut fx, 5, 0, 0), Ok(false));
    assert_eq!(ids(&fx), before);
}

#[test]
fn mode_0_puts_the_new_object_on_the_tile_ahead_of_me() {
    // 0x44e1e6: my position moves one tile in my facing (0x42228a): word
    // 0 by [0, 1, 1, 1, 0, -1, -1, -1], word 1 by [-1, -1, 0, 1, 1, 1, 0,
    // -1]. Unlike mode 9, a diagonal is not my own tile, and no wall is
    // tested: facing 0's tile (2, 1) is walled off from the lamp.
    for (facing, place) in [
        (0, [40, 24]),
        (1, [56, 24]),
        (2, [56, 40]),
        (3, [56, 56]),
        (4, [40, 56]),
        (6, [24, 40]),
        (8, [40, 24]),
    ] {
        let (mut fx, _) = container();
        fx.lamp_mut().data[1] = facing;
        let before = ids(&fx);
        assert_eq!(create(&mut fx, 0, 0, 0), Ok(true), "{facing}");
        let id = new_object(&fx, &before);
        let widget = &fx.world.objects[&id];
        assert_eq!(widget.position, [place[0], place[1], 1], "{facing}");
        assert_eq!(widget.data[2], 0);
    }
    // Off the lot, nothing is made: tile (1, -1).
    let (mut fx, _) = container();
    fx.lamp_mut().position = [24, 8, 1];
    fx.lamp_mut().data[1] = 0;
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 0, 0, 0), Ok(false));
    assert_eq!(ids(&fx), before);
}

#[test]
fn mode_3_puts_the_new_object_on_the_tile_ahead_of_the_stack_object_in_its_room() {
    // 0x44e34c: the stack object's position moves one tile in its facing
    // (0x42228a), on its level. Its room (data word 29) is the filter:
    // tiles (3, 2) and (2, 3) are in the lamp's room 1, tile (1, 2) is
    // not (0x44e582).
    for (facing, place) in [(2, Some([56, 40])), (4, Some([40, 56])), (6, None)] {
        let (mut fx, _) = container();
        fx.lamp_mut().data[1] = facing;
        let before = ids(&fx);
        let made = create(&mut fx, 3, 0, LAMP as i16);
        assert_eq!(made, Ok(place.is_some()), "{facing}");
        match place {
            Some(place) => {
                let id = new_object(&fx, &before);
                let widget = &fx.world.objects[&id];
                assert_eq!(widget.position, [place[0], place[1], 1], "{facing}");
                assert_eq!(widget.data[2], 0);
            }
            None => assert_eq!(ids(&fx), before),
        }
    }
    // Tile (1, 2) takes it for a stack object in room 0, or in room -5,
    // which skips the test.
    for room in [0, -5] {
        let (mut fx, _) = container();
        fx.lamp_mut().data[1] = 6;
        fx.lamp_mut().data[29] = room;
        let before = ids(&fx);
        assert_eq!(create(&mut fx, 3, 0, LAMP as i16), Ok(true), "{room}");
        let id = new_object(&fx, &before);
        assert_eq!(fx.world.objects[&id].position, [24, 40, 1], "{room}");
    }
    // A stack object that names no object makes nothing.
    for stack_object in [0, -1, 99] {
        let (mut fx, _) = container();
        let before = ids(&fx);
        assert_eq!(create(&mut fx, 3, 0, stack_object), Ok(false));
        assert_eq!(ids(&fx), before);
    }
    // The running object's own facing and place play no part: the stack
    // object is the Sim, on tile (2, 2) facing 2, in room 1.
    let (mut fx, _) = container();
    fx.lamp_mut().data[1] = 6;
    let sim = fx.sim_mut();
    sim.position = [40, 40, 1];
    sim.data[1] = 2;
    sim.data[29] = 1;
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 3, 0, SIM as i16), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].position, [56, 40, 1]);
}

#[test]
fn flag_0x8_makes_nothing_on_a_tile_where_anything_stands() {
    // 0x44e51e: a free tile takes the new object; once it stands there,
    // a second one is refused before it is made.
    let (mut fx, _) = container();
    fx.lamp_mut().data[1] = 2;
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 0, 0x8, 0), Ok(true));
    let made = ids(&fx);
    assert_eq!(made.len(), before.len() + 1);
    assert_eq!(create(&mut fx, 0, 0x8, 0), Ok(false));
    assert_eq!(ids(&fx), made);
}

/// Runs the lamp's tree 4096: local 0 is set to `turn`, then Create New
/// Object Instance of the widget in mode 9, its turn from local 0.
fn create_in_front(fx: &mut Fixture, turn: i16) -> Result<bool, sims_vm::Fault> {
    let [g0, g1, g2, g3] = WIDGET.to_le_bytes();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, turn, 1),
            ins(
                CREATE,
                RETURN_TRUE,
                RETURN_FALSE,
                [g0, g1, g2, g3, 9, 0, 0, 0],
            ),
        ]),
    );
    fx.run_with(LAMP, 4096, 0, &[], 1000)
}

#[test]
fn mode_9_puts_the_new_object_on_the_tile_in_front_of_me() {
    // 0x44e157: the lamp on tile (2, 2) faces `facing`, turned by local
    // 0. Directions 0, 2, 4 and 6 are one tile along position word 1
    // back, word 0 on, word 1 on and word 0 back; an odd one is my own
    // tile. Each place is open towards the lamp.
    for (facing, turn, place) in [
        (2, 0, [56, 40]),
        (0, 2, [56, 40]),
        (0, -6, [56, 40]),
        (0, 4, [40, 56]),
        (1, 0, [40, 40]),
        (4, 3, [40, 40]),
    ] {
        let (mut fx, _) = container();
        fx.lamp_mut().data[1] = facing;
        let before = ids(&fx);
        assert_eq!(create_in_front(&mut fx, turn), Ok(true), "{facing} {turn}");
        let id = new_object(&fx, &before);
        let widget = &fx.world.objects[&id];
        assert_eq!(widget.position, [place[0], place[1], 1], "{facing} {turn}");
        assert_eq!(widget.data[2], 0);
    }
}

#[test]
fn mode_9_makes_nothing_on_a_tile_walled_off_from_me() {
    // Facing 0, the tile is (2, 1), whose wall segment 8 closes direction
    // 4, the way back to me (0x4359ea). Facing 6, it is (1, 2), whose
    // segment 4 closes direction 2.
    for facing in [0, 6] {
        let (mut fx, _) = container();
        fx.lamp_mut().data[1] = facing;
        let before = ids(&fx);
        assert_eq!(create_in_front(&mut fx, 0), Ok(false), "{facing}");
        assert_eq!(ids(&fx), before, "{facing}");
    }
    // Off the lot, nothing is made either: tile (1, 0) is on the border.
    let (mut fx, _) = container();
    fx.lamp_mut().position = [24, 8, 1];
    fx.lamp_mut().data[1] = 0;
    let before = ids(&fx);
    assert_eq!(create_in_front(&mut fx, 0), Ok(false));
    assert_eq!(ids(&fx), before);
}

#[test]
fn flag_0x10_puts_my_temp_0_in_the_new_objects_bottom_frame() {
    // 0x44e8e1 to 0x44e90c: without 0x2, the running object's temp 0
    // (+0x3a) goes to the first data word of the new object's bottom
    // frame (+0x10): its first argument. Nothing else changes.
    for mode in [2, 4, 5, 6] {
        let (mut fx, _) = container();
        fx.lamp_mut().temps[0] = 42;
        let before = ids(&fx);
        assert_eq!(
            create(&mut fx, mode, 0x10, LAMP as i16),
            Ok(true),
            "mode {mode}"
        );
        let id = new_object(&fx, &before);
        let (mut plain, _) = container();
        plain.lamp_mut().temps[0] = 42;
        assert_eq!(create(&mut plain, mode, 0, LAMP as i16), Ok(true));
        let bottom = &plain.world.objects[&id].stack[0];
        assert!(!bottom.parameters.is_empty(), "mode {mode}");
        assert_ne!(bottom.parameters[0], 42, "mode {mode}");
        plain.world.objects.get_mut(&id).unwrap().stack[0].parameters[0] = 42;
        assert_eq!(fx.world.hash(), plain.world.hash(), "mode {mode}");
    }
}

#[test]
fn flag_0x10_with_no_argument_writes_the_first_local_or_fails_loudly() {
    // The frame's data is its arguments, then its locals (0x47518f): a
    // bottom frame with no argument has its first local written, and one
    // with neither fails before anything past it is written.
    let run = |locals: u16| {
        let (mut fx, file) = container();
        fx.add(
            file,
            MAIN,
            shaped(
                0x8002,
                0,
                0,
                locals,
                vec![ins(SLEEP, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])],
            ),
        );
        fx.lamp_mut().temps[0] = 42;
        let before = ids(&fx);
        let result = create(&mut fx, 6, 0x10, LAMP as i16);
        (fx, before, result)
    };
    let (fx, before, result) = run(2);
    assert_eq!(result, Ok(true));
    let bottom = &fx.world.objects[&new_object(&fx, &before)].stack[0];
    assert_eq!(bottom.tree, MAIN);
    assert!(bottom.parameters.is_empty());
    assert_eq!(bottom.locals, [42, 0]);
    let (_, _, result) = run(0);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x44e90c")),);
}

/// Gives the lamp `count` empty container slots at its centre, height 1.
fn five_slot_container() -> (Fixture, usize) {
    let (mut fx, file) = container();
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .slots
        .insert(SLOTS, vec![container_slot(); 5]);
    let lamp = fx.lamp_mut();
    let tile = lamp.slots[0].clone();
    lamp.slots.resize(6, tile);
    for held in &mut lamp.slots[1..] {
        held.object = 0;
    }
    (fx, file)
}

#[test]
fn mode_1_puts_the_new_object_in_my_slot_0_like_mode_2() {
    // 0x44e215: mode 1 sets container = me and leaves slot at its initial 0,
    // with room filter -5.
    for room in [1, 0] {
        let (mut fx, _) = container();
        fx.lamp_mut().data[29] = room;
        let before = ids(&fx);
        assert_eq!(create(&mut fx, 1, 0, 0), Ok(true), "room {room}");
        let id = new_object(&fx, &before);
        let widget = &fx.world.objects[&id];
        assert_eq!(widget.position, [40, 40, 1]);
        assert_eq!((widget.data[2], widget.data[3]), (LAMP as i16, 0));
        assert_eq!(fx.lamp().slots[1].object, id as i16);
    }
}

#[test]
fn modes_10_and_11_put_the_new_object_in_my_slots_2_and_3() {
    // 0x44e24d (mode 10 -> slot 2) and 0x44e26e (mode 11 -> slot 3), keeping
    // room filter -5.
    for (mode, slot) in [(10u8, 2i16), (11, 3)] {
        let (mut fx, _) = five_slot_container();
        fx.lamp_mut().data[29] = 0; // mismatches tile room 1, still ignored
        let before = ids(&fx);
        assert_eq!(create(&mut fx, mode, 0, 0), Ok(true), "mode {mode}");
        let id = new_object(&fx, &before);
        let widget = &fx.world.objects[&id];
        assert_eq!(widget.position, [40, 40, 1]);
        assert_eq!((widget.data[2], widget.data[3]), (LAMP as i16, slot));
        assert_eq!(fx.lamp().slots[slot as usize + 1].object, id as i16);
    }
}

#[test]
fn modes_12_13_and_14_put_the_new_object_in_stack_objects_slots_2_3_and_4() {
    // 0x44e28f (mode 12 -> slot 2), 0x44e2ce (mode 13 -> slot 3), 0x44e30d
    // (mode 14 -> slot 4), filtered to the stack object's room (0x44e3d9).
    for (mode, slot) in [(12u8, 2i16), (13, 3), (14, 4)] {
        let (mut fx, _) = five_slot_container();
        let before = ids(&fx);
        assert_eq!(
            create(&mut fx, mode, 0, LAMP as i16),
            Ok(true),
            "mode {mode}"
        );
        let id = new_object(&fx, &before);
        let widget = &fx.world.objects[&id];
        assert_eq!(widget.position, [40, 40, 1]);
        assert_eq!((widget.data[2], widget.data[3]), (LAMP as i16, slot));
        assert_eq!(fx.lamp().slots[slot as usize + 1].object, id as i16);

        // Room mismatch refuses before creating.
        let (mut fx, _) = five_slot_container();
        fx.lamp_mut().data[29] = 0;
        let before = ids(&fx);
        assert_eq!(
            create(&mut fx, mode, 0, LAMP as i16),
            Ok(false),
            "mode {mode} room mismatch"
        );
        assert_eq!(ids(&fx), before);

        // Missing stack object returns false.
        let (mut fx, _) = five_slot_container();
        let before = ids(&fx);
        assert_eq!(create(&mut fx, mode, 0, 77), Ok(false), "mode {mode}");
        assert_eq!(ids(&fx), before);
    }
}

#[test]
fn mode_7_puts_the_new_object_where_parameter_0_stands() {
    // 0x44e3fd: reads parameter 0 (native error 8 if frame has 0 params),
    // ignores operand 6 (local), returns false if parameter 0 names no object.
    let [g0, g1, g2, g3] = WIDGET.to_le_bytes();
    let run_mode_7 = |fx: &mut Fixture, arg_count: u8, args: &[i16]| {
        fx.add(
            LAMP_FILE,
            4096,
            shaped(
                0x8002,
                0,
                arg_count,
                0,
                vec![ins(
                    CREATE,
                    RETURN_TRUE,
                    RETURN_FALSE,
                    [g0, g1, g2, g3, 7, 0, 99, 0],
                )],
            ),
        );
        fx.run_with(LAMP, 4096, 0, args, 1000)
    };

    // Valid object in parameter 0: placed at its position and level.
    let (mut fx, _) = container();
    fx.sim_mut().position = [56, 40, 1];
    let before = ids(&fx);
    assert_eq!(run_mode_7(&mut fx, 1, &[SIM as i16]), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].position, [56, 40, 1]);

    // Missing or zero object ID in parameter 0 returns false, creates nothing.
    for bad in [0i16, -1, 77] {
        let (mut fx, _) = container();
        let before = ids(&fx);
        assert_eq!(run_mode_7(&mut fx, 1, &[bad]), Ok(false));
        assert_eq!(ids(&fx), before);
    }

    // Frame with 0 parameters raises native error 8.
    let (mut fx, _) = container();
    assert_eq!(code(run_mode_7(&mut fx, 0, &[])), Some(8));
}

#[test]
fn modes_above_14_fall_through_to_origin_and_return_false() {
    // 0x44e147: unsigned compare > 14 jumps to 0x44e4c5 with pos = [0, 0]
    // and place_on_lot = true, failing the off-lot check without creating.
    for mode in [15u8, 16, 128, 255] {
        let (mut fx, _) = container();
        let before = ids(&fx);
        assert_eq!(
            create(&mut fx, mode, 0, LAMP as i16),
            Ok(false),
            "mode {mode}"
        );
        assert_eq!(ids(&fx), before, "mode {mode}");
    }
}

#[test]
fn flag_0x40_skips_room_and_wall_checks() {
    // 0x44e564..0x44e577: flag 0x40 clears both the room filter and the
    // wall-check facing.
    // 1. Mode 3 ahead into another room succeeds with 0x40.
    let (mut fx, _) = container();
    fx.lamp_mut().data[1] = 6; // tile (1, 2) is in room 0, lamp is in room 1
    assert_eq!(create(&mut fx, 3, 0, LAMP as i16), Ok(false));
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 3, 0x40, LAMP as i16), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].position, [24, 40, 1]);

    // 2. Mode 4 container whose room mismatches its tile succeeds with 0x40.
    let (mut fx, _) = container();
    fx.lamp_mut().data[29] = 0; // tile (2, 2) is in room 1
    assert_eq!(create(&mut fx, 4, 0, LAMP as i16), Ok(false));
    assert_eq!(create(&mut fx, 4, 0x40, LAMP as i16), Ok(true));

    // 3. Mode 9 onto a tile walled off from me succeeds with 0x40.
    let (mut fx, _) = container();
    fx.lamp_mut().data[1] = 0; // tile (2, 1) is walled off from (2, 2)
    let [g0, g1, g2, g3] = WIDGET.to_le_bytes();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, 0, 1),
            ins(
                CREATE,
                RETURN_TRUE,
                RETURN_FALSE,
                [g0, g1, g2, g3, 9, 0x40, 0, 0],
            ),
        ]),
    );
    let before = ids(&fx);
    assert_eq!(fx.run_with(LAMP, 4096, 0, &[], 1000), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].position, [40, 24, 1]);
}

#[test]
fn flag_0x02_passes_me_and_caller_stack_object_to_new_bottom_frame() {
    // 0x44e87c..0x44e8df: flag 0x02 writes self.me to bottom frame data[0]
    // and copies the caller's stack_object (before it becomes `created`) into
    // bottom.stack_object.
    let (mut fx, _) = container();
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 6, 0x02, SIM as i16), Ok(true));
    let id = new_object(&fx, &before);
    let bottom = &fx.world.objects[&id].stack[0];
    assert_eq!(bottom.parameters[0], LAMP as i16);
    assert_eq!(bottom.stack_object, SIM as i16);

    // With 0 parameters and >= 1 local, writes locals[0] and stack_object.
    let (mut fx, file) = container();
    fx.add(
        file,
        MAIN,
        shaped(
            0x8002,
            0,
            0,
            2,
            vec![ins(SLEEP, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])],
        ),
    );
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 6, 0x02, SIM as i16), Ok(true));
    let id = new_object(&fx, &before);
    let bottom = &fx.world.objects[&id].stack[0];
    assert_eq!(bottom.locals, [LAMP as i16, 0]);
    assert_eq!(bottom.stack_object, SIM as i16);

    // With 0 parameters and 0 locals, fails loudly at 0x44e8d1.
    let (mut fx, file) = container();
    fx.add(
        file,
        MAIN,
        shaped(
            0x8002,
            0,
            0,
            0,
            vec![ins(SLEEP, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])],
        ),
    );
    let result = create(&mut fx, 6, 0x02, SIM as i16);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x44e8d1")));

    // Combined with flag 0x10: raises native error 0x36 (0x44e88c) AFTER
    // creation/placement, but NOT when placement fails early.
    let (mut fx, _) = container();
    assert_eq!(
        code(create(&mut fx, 6, 0x02 | 0x10, SIM as i16)),
        Some(0x36)
    );

    let (mut fx, _) = container();
    fx.lamp_mut().position = [-16, -16, 1];
    assert_eq!(create(&mut fx, 5, 0x02 | 0x10, SIM as i16), Ok(false));
}

#[test]
fn container_and_off_lot_modes_honor_flags_0x08_0x20_and_ignore_0x80() {
    // 0x44e51e: flag 0x08 checks anything_on_tile in every mode (passes off
    // the lot for mode 6, blocks container mode 4 when the tile is occupied).
    let (mut fx, _) = container();
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 6, 0x08 | 0x80, 0), Ok(true));
    assert_eq!(ids(&fx).len(), before.len() + 1);

    let (mut fx, _) = container();
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 5, 0, 0), Ok(true)); // put a widget on (2, 2)
    let after_first = ids(&fx);
    assert_eq!(after_first.len(), before.len() + 1);
    assert_eq!(create(&mut fx, 4, 0x08, LAMP as i16), Ok(false));
    assert_eq!(ids(&fx), after_first);

    // 0x44e640: flag 0x20 turns the new object in container modes too.
    let (mut fx, file) = container();
    fx.add(
        file,
        4096,
        tree(vec![
            assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
            assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
            assign(scope::MY_DATA, 23, scope::LITERAL, 2, RETURN_TRUE),
        ]),
    );
    fx.lamp_mut().data[1] = 4;
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 4, 0x20 | 0x80, LAMP as i16), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].data[1], 4);
    assert_eq!(fx.world.objects[&id].data[2], LAMP as i16);
}

#[test]
fn mode_4_with_person_or_multitile_group_follows_common_tail() {
    // A person created in a container slot goes through make_person -> fits_in
    // -> put (or remove_now if refused).
    let (mut fx, file) = container();
    fx.words(file)[9] = 2;
    crate::person_factory::equip(&mut fx, WIDGET, "widget", 12);
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 4, 0, LAMP as i16), Ok(true));
    let id = new_object(&fx, &before);
    assert!(fx.world.objects[&id].person.is_some());
    assert_eq!(fx.world.objects[&id].data[2], LAMP as i16);
    assert_eq!(fx.lamp().slots[1].object, id as i16);

    // A person without a skeleton still fails cleanly before placement.
    let (mut fx, file) = container();
    fx.words(file)[9] = 2;
    let before = ids(&fx);
    let result = create(&mut fx, 4, 0, LAMP as i16);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("skeleton")));
    assert_eq!(ids(&fx), before);
}

#[test]
fn mode_6_with_flag_0x20_keeps_a_new_object_that_already_faces_right() {
    // The stack object faces 1: the new object is to face 0 (bit 0
    // cleared), as it does when made.
    let (mut fx, _) = container();
    fx.lamp_mut().data[1] = 1;
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 6, 0x20, LAMP as i16), Ok(true));
    let id = new_object(&fx, &before);
    let object = &fx.world.objects[&id];
    assert_eq!((object.position, object.data[1]), ([-16, -16, 1], 0));
    // With no stack object nothing turns.
    let (mut fx, _) = container();
    assert_eq!(create(&mut fx, 6, 0x21, 0), Ok(true));
    // The stack object faces 2, but the widget's turn step (data word 23)
    // is 0, so its turn (0x433f75) changes nothing, four times over.
    let (mut fx, _) = container();
    fx.lamp_mut().data[1] = 2;
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 6, 0x20, LAMP as i16), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].data[1], 0);
}

#[test]
fn mode_6_with_flag_0x02_and_empty_stack_skips_frame_write() {
    // 0x44e8b2: when the created object has no frames, flag 0x02 skips the
    // bottom-frame write and still sets the caller's stack_object.
    let (mut fx, file) = container();
    // Main returns immediately so the new object's stack is empty after its
    // first turn!
    fx.add(
        file,
        MAIN,
        tree(vec![ins(
            2,
            RETURN_TRUE,
            RETURN_TRUE,
            expr(scope::LITERAL, 0, op::EQUAL, scope::LITERAL, 0),
        )]),
    );
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 6, 0x02, SIM as i16), Ok(true));
    let id = new_object(&fx, &before);
    assert!(fx.world.objects[&id].stack.is_empty());
}

#[test]
fn flag_0x4_with_a_stack_object_that_names_no_record_makes_nothing() {
    // Flag 0x4 takes the type from the neighbor record the stack object
    // names (0x44e0e6), not from the GUID: the lamp's ID names none here.
    for mode in [4, 6] {
        let (mut fx, _) = container();
        let before = ids(&fx);
        assert_eq!(create(&mut fx, mode, 0x4, LAMP as i16), Ok(false));
        assert_eq!(ids(&fx), before);
    }
}
