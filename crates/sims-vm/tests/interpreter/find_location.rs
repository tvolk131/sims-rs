//! Find Location For (opcode 16, 0x44b3c8) and Create New Object Instance
//! mode 8 (0x44e0c8), which both put an object on the lot after the place
//! test (0x433311).
//!
//! The lot is the six-tile one of the room tests: room 0 outside, room 1
//! the four tiles (2, 2) to (3, 3) inside walls. Its tiles have no floor,
//! so the objects here may stand without one (placement flag 2, data word
//! 42).

use crate::factory::{WIDGET, widgets};
use crate::person_factory::equip;
use crate::rooms::{array, build};
use crate::support::*;
use sims_core::GameRng;
use sims_vm::Trap;

/// The opcodes under test.
const FIND_LOCATION: u16 = 16;
const CREATE: u16 = 42;

/// A widget's Init: it may be placed on level 1 (data word 4 bit 0) and
/// stand on a tile with no floor (data word 42 flag 2).
pub fn placeable() -> (Fixture, usize) {
    let (mut fx, file) = widgets();
    fx.add(
        file,
        4096,
        tree(vec![
            assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
            assign(scope::MY_DATA, 42, scope::LITERAL, 2, RETURN_TRUE),
        ]),
    );
    build(&mut fx);
    fx.world.arrays.insert(3, array(2, 0));
    fx.world.arrays.insert(103, array(2, 0));
    // No pool and no water anywhere.
    fx.world.arrays.insert(9, array(1, 0xff));
    fx.world.arrays.insert(10, array(1, 0xff));
    // The lot has its indicator objects, so an object coming onto it from
    // outside makes none.
    fx.world.indicators = [LAMP, SIM];
    (fx, file)
}

pub fn widget(fx: &mut Fixture) -> u16 {
    sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream)
        .create(&mut fx.world, WIDGET)
        .unwrap()
}

/// Runs the lamp's tree 4096: local 0 is set to `local`, then the
/// instruction runs with the given stack object.
fn run(
    fx: &mut Fixture,
    local: i16,
    stack_object: i16,
    opcode: u16,
    operands: [u8; 8],
) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, local, 1),
            ins(opcode, RETURN_TRUE, RETURN_FALSE, operands),
        ]),
    );
    fx.run_with(LAMP, 4096, stack_object, &[], 1000)
}

pub fn find(fx: &mut Fixture, local: i16, target: u16, flags: u8) -> Result<bool, sims_vm::Fault> {
    run(
        fx,
        local,
        target as i16,
        FIND_LOCATION,
        [0, 0, flags, 0, 0, 0, 0, 0],
    )
}

pub fn create(
    fx: &mut Fixture,
    local: u8,
    flags: u8,
    stack_object: i16,
) -> Result<bool, sims_vm::Fault> {
    let [g0, g1, g2, g3] = WIDGET.to_le_bytes();
    run(
        fx,
        LAMP as i16,
        stack_object,
        CREATE,
        [g0, g1, g2, g3, 8, flags, local, 0],
    )
}

#[test]
fn objects_go_on_the_start_tile_then_spiral_out() {
    let (mut fx, _) = placeable();
    let (first, second) = (widget(&mut fx), widget(&mut fx));
    assert_eq!(fx.world.objects[&first].position, [-16, -16, 1]);
    // With no flag the search starts on tile (1, 1).
    assert_eq!(find(&mut fx, 0, first, 0), Ok(true));
    assert_eq!(fx.world.objects[&first].position, [24, 24, 1]);
    // That tile is taken now. The spiral goes to (0, 1) and (0, 2),
    // outside the border, then to (1, 2).
    assert_eq!(find(&mut fx, 0, second, 0), Ok(true));
    assert_eq!(fx.world.objects[&second].position, [24, 40, 1]);
}

#[test]
fn a_search_from_an_object_stays_in_its_room() {
    let (mut fx, _) = placeable();
    fx.lamp_mut().position = [40, 40, 1];
    let (first, second) = (widget(&mut fx), widget(&mut fx));
    assert_eq!(find(&mut fx, LAMP as i16, first, 1), Ok(true));
    assert_eq!(fx.world.objects[&first].position, [40, 40, 1]);
    // (1, 2) and (1, 3) are free but in room 0; (2, 3) is in room 1.
    assert_eq!(find(&mut fx, LAMP as i16, second, 1), Ok(true));
    assert_eq!(fx.world.objects[&second].position, [40, 56, 1]);
}

#[test]
fn a_tile_with_another_object_is_allowed_with_flag_2() {
    let (mut fx, _) = placeable();
    let (first, second) = (widget(&mut fx), widget(&mut fx));
    assert_eq!(find(&mut fx, 0, first, 0), Ok(true));
    // The grid word no longer has to be empty, but the two would still
    // collide on (1, 1), so the second goes on to (1, 2) all the same.
    assert_eq!(find(&mut fx, 0, second, 2), Ok(true));
    assert_eq!(fx.world.objects[&second].position, [24, 40, 1]);
}

#[test]
fn an_object_that_fits_nowhere_takes_the_false_branch() {
    let (mut fx, _) = placeable();
    let id = widget(&mut fx);
    // Data word 4 without bit 0: the place test refuses every tile.
    fx.world.objects.get_mut(&id).unwrap().data[4] = 0;
    assert_eq!(find(&mut fx, 0, id, 0), Ok(false));
    assert_eq!(fx.world.objects[&id].position, [-16, -16, 1]);
}

#[test]
fn find_location_needs_its_objects() {
    let (mut fx, _) = placeable();
    let id = widget(&mut fx);
    assert_eq!(code(find(&mut fx, 0, 77, 0)), Some(0x15));
    assert_eq!(code(find(&mut fx, 77, id, 1)), Some(0x17));
}

#[test]
fn mode_1_takes_the_object_off_the_lot() {
    let (mut fx, _) = placeable();
    let id = widget(&mut fx);
    assert_eq!(find(&mut fx, 0, id, 0), Ok(true));
    assert_eq!(fx.world.objects[&id].position, [24, 24, 1]);
    // Its virtual function 0x50 (0x44b55f): off the lot, and the true
    // branch.
    let mode_1 =
        |fx: &mut Fixture, target: i16| run(fx, 0, target, FIND_LOCATION, [1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(mode_1(&mut fx, id as i16), Ok(true));
    assert_eq!(fx.world.objects[&id].position, [-16, -16, 1]);
    // The tile is free again.
    let other = widget(&mut fx);
    assert_eq!(find(&mut fx, 0, other, 0), Ok(true));
    assert_eq!(fx.world.objects[&other].position, [24, 24, 1]);
    // A tile of a group: `group_factory`.
}

#[test]
fn other_find_location_modes_fail_loudly() {
    let (mut fx, _) = placeable();
    let id = widget(&mut fx);
    // Mode 2, in front of the object.
    let result = run(
        &mut fx,
        0,
        id as i16,
        FIND_LOCATION,
        [2, 0, 0, 0, 0, 0, 0, 0],
    );
    assert!(matches!(trap(result), Trap::Unsupported(m) if m.contains("not ported")));
    // Modes 3 and 4 need an object to search from (flag bit 0): native
    // error 0x3d (0x44b676).
    for mode in [3, 4] {
        let result = run(
            &mut fx,
            0,
            id as i16,
            FIND_LOCATION,
            [mode, 0, 0, 0, 0, 0, 0, 0],
        );
        assert_eq!(trap(result).code(), Some(0x3d), "mode {mode}");
    }
}

#[test]
fn modes_3_and_4_spiral_when_the_line_direction_is_minus_1() {
    // The search record keeps the raw direction, and -1 there is the
    // spiral (0x434ba9): mode 3 from a facing of -1, and mode 4 from -3
    // (turned by 2, remainder 8). The start tile is taken, so the spiral
    // finds the free tile (2, 3); a line of direction -1 would never step
    // and would give up.
    for (mode, facing) in [(3, -1), (4, -3)] {
        let (mut fx, _) = placeable();
        fx.lamp_mut().position = [40, 40, 1];
        fx.lamp_mut().data[1] = facing;
        let (first, other) = (widget(&mut fx), widget(&mut fx));
        for (id, expected) in [(first, [40, 40, 1]), (other, [40, 56, 1])] {
            let result = run(
                &mut fx,
                LAMP as i16,
                id as i16,
                FIND_LOCATION,
                [mode, 0, 1, 0, 0, 0, 0, 0],
            );
            assert_eq!(result, Ok(true), "mode {mode}");
            assert_eq!(fx.world.objects[&id].position, expected, "mode {mode}");
        }
    }
}

#[test]
fn modes_3_and_4_search_along_a_line_by_the_objects_facing() {
    // The lamp stands on tile (2, 2), facing 0. Mode 3 searches along its
    // facing: the second tile coordinate steps 0, 1, -1 (0x434be9). Mode
    // 4 turns it by 2: the first coordinate steps.
    for (mode, second) in [(3, [40, 56, 1]), (4, [56, 40, 1])] {
        let (mut fx, _) = placeable();
        fx.lamp_mut().position = [40, 40, 1];
        fx.lamp_mut().data[1] = 0;
        let (first, other) = (widget(&mut fx), widget(&mut fx));
        for (id, expected) in [(first, [40, 40, 1]), (other, second)] {
            let result = run(
                &mut fx,
                LAMP as i16,
                id as i16,
                FIND_LOCATION,
                [mode, 0, 1, 0, 0, 0, 0, 0],
            );
            assert_eq!(result, Ok(true), "mode {mode}");
            assert_eq!(fx.world.objects[&id].position, expected, "mode {mode}");
        }
    }
}

#[test]
fn mode_8_creates_the_object_where_the_local_object_stands() {
    let (mut fx, _) = placeable();
    fx.lamp_mut().position = [40, 40, 1];
    let before = fx.world.objects.len();
    assert_eq!(create(&mut fx, 0, 0, SIM as i16), Ok(true));
    assert_eq!(fx.world.objects.len(), before + 1);
    let created = fx.world.order[0];
    assert_eq!(fx.world.objects[&created].guid, WIDGET);
    assert_eq!(fx.world.objects[&created].position, [40, 40, 1]);
}

#[test]
fn mode_8_removes_an_object_that_does_not_fit() {
    let (mut fx, _) = placeable();
    fx.lamp_mut().position = [40, 40, 1];
    let blocker = widget(&mut fx);
    assert_eq!(find(&mut fx, LAMP as i16, blocker, 1), Ok(true));
    let before: Vec<u16> = fx.world.objects.keys().copied().collect();
    // Both are solid and would overlap: the new one is made, refused and
    // removed again.
    assert_eq!(create(&mut fx, 0, 0, SIM as i16), Ok(false));
    assert_eq!(fx.world.objects.keys().copied().collect::<Vec<_>>(), before);
}

#[test]
fn mode_8_off_the_lot_makes_nothing() {
    let (mut fx, _) = placeable();
    fx.lamp_mut().position = [-16, -16, 1];
    let before = fx.world.objects.len();
    assert_eq!(create(&mut fx, 0, 0, SIM as i16), Ok(false));
    assert_eq!(fx.world.objects.len(), before);
}

#[test]
fn mode_8_needs_a_local_naming_an_object() {
    let (mut fx, _) = placeable();
    // The tree has four locals.
    assert_eq!(code(create(&mut fx, 9, 0, SIM as i16)), Some(8));
    let before = fx.world.objects.len();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            CREATE,
            RETURN_TRUE,
            RETURN_FALSE,
            [0x33, 0x33, 0, 0, 8, 0, 1, 0],
        )]),
    );
    // Local 1 is 0.
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[], 1000), Ok(false));
    assert_eq!(fx.world.objects.len(), before);
}

#[test]
fn flag_0x20_turns_the_new_object_only_when_it_already_faces_right() {
    let (mut fx, _) = placeable();
    fx.lamp_mut().position = [40, 40, 1];
    // The stack object faces 1; with the lowest bit cleared that is the
    // new object's facing 0, so nothing turns.
    fx.sim_mut().data[1] = 1;
    assert_eq!(create(&mut fx, 0, 0x20, SIM as i16), Ok(true));
    // Facing 3 asks for 2: turning is not ported.
    let (mut fx, _) = placeable();
    fx.lamp_mut().position = [40, 40, 1];
    fx.sim_mut().data[1] = 3;
    let result = create(&mut fx, 0, 0x20, SIM as i16);
    assert!(matches!(trap(result), Trap::Unsupported(m) if m.contains("0x433f75")));
}

#[test]
fn mode_8_with_other_flags_or_a_person_it_cannot_make_fails_loudly() {
    let (mut fx, _) = placeable();
    fx.lamp_mut().position = [40, 40, 1];
    let result = create(&mut fx, 0, 0x02, SIM as i16);
    assert!(matches!(trap(result), Trap::Unsupported(_)));
    // A person type with no skeleton supplied.
    let (mut fx, file) = placeable();
    fx.lamp_mut().position = [40, 40, 1];
    fx.words(file)[9] = 2;
    let result = create(&mut fx, 0, 0, SIM as i16);
    assert!(matches!(trap(result), Trap::Unsupported(m) if m.contains("skeleton")));
}

/// The widget as a person type, [`equip`]ped, with records whose words
/// are all 12 (bits 0 and 1 clear).
pub fn person_widget() -> Fixture {
    let (mut fx, file) = placeable();
    fx.words(file)[9] = 2;
    equip(&mut fx, WIDGET, "widget", 12);
    fx.lamp_mut().position = [40, 40, 1];
    fx
}

#[test]
fn mode_8_makes_a_person_that_takes_the_stack_objects_facing_whole() {
    let mut fx = person_widget();
    let before: Vec<u16> = fx.world.objects.keys().copied().collect();
    // The stack object faces 3. An object would turn to 2 (not ported); a
    // person takes 3 as it is.
    fx.sim_mut().data[1] = 3;
    assert_eq!(create(&mut fx, 0, 0x20, SIM as i16), Ok(true));
    let id = *fx
        .world
        .objects
        .keys()
        .find(|id| !before.contains(id))
        .unwrap();
    let person = &fx.world.objects[&id];
    assert!(person.person.is_some());
    assert_eq!(person.data[1], 3);
    assert_eq!(person.position, [40, 40, 1]);
    // The record's interests.
    assert_eq!(person.person.as_ref().unwrap().data[46..54], [12; 8]);
}

#[test]
fn a_person_is_kept_off_a_tile_by_a_solid_object_without_flag_0x10() {
    let mut fx = person_widget();
    let before: Vec<u16> = fx.world.objects.keys().copied().collect();
    assert_eq!(create(&mut fx, 0, 0, SIM as i16), Ok(true));
    let first = *fx
        .world
        .objects
        .keys()
        .find(|id| !before.contains(id))
        .unwrap();
    // The first person stands on the tile, solid (data word 8 flag 4
    // clear) and without flag 0x10: it is in the second's way. The place
    // test refuses, the new person is removed at once (0x44123f) and the
    // false branch is taken.
    assert_eq!(fx.world.objects[&first].data[8] & 0x14, 0);
    let placed = fx.world.clone();
    let refused = |fx: &mut Fixture| {
        assert_eq!(create(fx, 0, 0, SIM as i16), Ok(false));
        let ids: Vec<u16> = fx.world.objects.keys().copied().collect();
        assert_eq!(ids, placed.objects.keys().copied().collect::<Vec<_>>());
        assert_eq!(fx.world.people, placed.people);
        assert_eq!(fx.world.order, placed.order);
        assert!(fx.world.animators.keys().all(|id| ids.contains(id)));
    };
    refused(&mut fx);
    // With flag 0x10 it is not in the way.
    fx.world = placed.clone();
    fx.world.objects.get_mut(&first).unwrap().data[8] |= 0x10;
    assert_eq!(create(&mut fx, 0, 0, SIM as i16), Ok(true));
    // Not solid (flag 4), it is in the way only with flag 2.
    fx.world = placed.clone();
    fx.world.objects.get_mut(&first).unwrap().data[8] = 4;
    assert_eq!(create(&mut fx, 0, 0, SIM as i16), Ok(true));
    fx.world = placed.clone();
    fx.world.objects.get_mut(&first).unwrap().data[8] = 4 | 2;
    refused(&mut fx);
}

#[test]
fn what_placing_a_new_person_does_not_port_fails_loudly() {
    // Person word 68, 74 or 84 (from the record) sends the original
    // through 0x45810c. Person word 81 of 10 or more, once the person is
    // on the lot, has the interface move a pooled object to it (0x447a82).
    for (word, value, address) in [
        (68, 1, "0x45810c"),
        (74, 2, "0x45810c"),
        (84, 1, "0x45810c"),
        (81, 10, "0x447a82"),
    ] {
        let mut fx = person_widget();
        let record = fx
            .world
            .neighbors
            .slots
            .last_mut()
            .unwrap()
            .as_mut()
            .unwrap();
        record.revision = 10;
        record.data[word] = value;
        let result = create(&mut fx, 0, 0, SIM as i16);
        assert!(
            matches!(trap(result), Trap::Unsupported(ref m) if m.contains(address)),
            "word {word}"
        );
    }
}

#[test]
fn mode_1_on_a_person_fails_loudly() {
    // Taking a person off the lot other than to remove it refreshes its
    // animator if it was on screen (0x58da12), which is not kept.
    let mut fx = person_widget();
    let before: Vec<u16> = fx.world.objects.keys().copied().collect();
    assert_eq!(create(&mut fx, 0, 0, SIM as i16), Ok(true));
    let id = *fx
        .world
        .objects
        .keys()
        .find(|id| !before.contains(id))
        .unwrap();
    let result = run(
        &mut fx,
        0,
        id as i16,
        FIND_LOCATION,
        [1, 0, 0, 0, 0, 0, 0, 0],
    );
    assert!(matches!(trap(result), Trap::Unsupported(m) if m.contains("0x58da12")));
    assert_eq!(fx.world.objects[&id].position, [40, 40, 1]);
}

/// Find Location For in mode 5, with the view turned to `rotation`.
fn find_random(
    fx: &mut Fixture,
    local: i16,
    target: u16,
    flags: u8,
    rotation: u8,
) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, local, 1),
            ins(
                FIND_LOCATION,
                RETURN_TRUE,
                RETURN_FALSE,
                [5, 0, flags, 0, 0, 0, 0, 0],
            ),
        ]),
    );
    let mut interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.view_rotation = rotation;
    interpreter.run_tree(&mut fx.world, LAMP, 4096, target as i16, &[], 1000)
}

/// The random start as 0x434921 works it out before the view's turn:
/// tile offsets of a draw modulo 10 less 5, the first draw to the second
/// coordinate, added up until the tile, cut to a byte, is inside the
/// border.
fn walked(rng: &mut GameRng, size: i32, [mut x, mut y]: [i32; 2]) -> [i32; 2] {
    let inside = |n: i32| (1..=size - 2).contains(&i32::from((n >> 4) as i8));
    loop {
        let first = (rng.next_u32() % 10) as i32 - 5;
        let second = (rng.next_u32() % 10) as i32 - 5;
        y += first * 16 + 8;
        x += second * 16 + 8;
        if inside(x) && inside(y) {
            return [x, y];
        }
    }
}

#[test]
fn mode_5_searches_from_a_random_walk_away_from_the_start() {
    // From state 8 the first two draws move the start (16, 16) by tiles
    // (1, 1) to (40, 40), inside the border; from state 2 the walk takes
    // 84 draws and ends on tile (2, 1). The search then takes the start
    // tile, which is free.
    for (seed, draws, tile) in [(8, 2, [2, 2]), (2, 84, [2, 1])] {
        let (mut fx, _) = placeable();
        let id = widget(&mut fx);
        fx.world.lot_rng = GameRng::from_state(seed);
        let mut expected = fx.world.lot_rng;
        let start = walked(&mut expected, i32::from(fx.world.size), [16, 16]);
        assert_eq!([start[0] >> 4, start[1] >> 4], tile, "state {seed}");
        let mut counted = GameRng::from_state(seed);
        for _ in 0..draws {
            counted.next_u32();
        }
        assert_eq!(counted, expected, "state {seed}");
        assert_eq!(find_random(&mut fx, 0, id, 0, 0), Ok(true));
        assert_eq!(fx.world.lot_rng, expected, "state {seed}");
        let middle = tile.map(|t| (t << 4) | 8);
        assert_eq!(fx.world.objects[&id].position, [middle[0], middle[1], 1]);
    }
}

#[test]
fn mode_5_draws_a_new_start_for_the_search_anywhere() {
    // Room 1, tiles (2, 2) to (3, 3), is full, so the first search, which
    // starts on (2, 2) and keeps to its room, finds nothing. The search
    // anywhere walks again from (16, 16).
    let (mut fx, _) = placeable();
    fx.lamp_mut().position = [40, 40, 1];
    for _ in 0..4 {
        let id = widget(&mut fx);
        assert_eq!(find(&mut fx, LAMP as i16, id, 1), Ok(true));
        let [x, y, _] = fx.world.objects[&id].position;
        assert!((2..=3).contains(&(x >> 4)) && (2..=3).contains(&(y >> 4)));
    }
    let id = widget(&mut fx);
    fx.world.lot_rng = GameRng::from_state(8);
    let mut expected = fx.world.lot_rng;
    let size = i32::from(fx.world.size);
    assert_eq!(walked(&mut expected, size, [16, 16]), [40, 40]);
    let second = walked(&mut expected, size, [16, 16]);
    assert_eq!(second.map(|n| n >> 4), [3, 2]);
    assert_eq!(find_random(&mut fx, 0, id, 0, 0), Ok(true));
    assert_eq!(fx.world.lot_rng, expected);
    // (3, 2) is in the full room too; the spiral goes on through (2, 2),
    // (2, 3) and (3, 3) to (4, 3).
    assert_eq!(fx.world.objects[&id].position, [72, 56, 1]);
}

#[test]
fn mode_5_from_exactly_1_1_draws_a_point_modulo_the_lots_size() {
    // A start of (1, 1) (the lamp's own place, flag 1) is replaced by two
    // draws modulo 6, read as a point: tile 0, clamped to tile 1.
    let (mut fx, _) = placeable();
    fx.lamp_mut().position = [1, 1, 1];
    let id = widget(&mut fx);
    let mut expected = fx.world.lot_rng;
    expected.next_u32();
    expected.next_u32();
    assert_eq!(find_random(&mut fx, LAMP as i16, id, 1, 0), Ok(true));
    assert_eq!(fx.world.lot_rng, expected);
    assert_eq!(fx.world.objects[&id].position, [24, 24, 1]);
}

#[test]
fn mode_5_turns_its_start_by_the_inverse_of_the_view() {
    // From state 8 the walk ends at (40, 40). Turned half round within
    // 1024 it is (983, 983), tile 61, clamped to tile 4. A quarter turn
    // (view 1, turned by 3) gives (1023 - 40, 40): tiles (4, 2).
    for (rotation, tile) in [(0, [2, 2]), (2, [4, 4]), (1, [4, 2]), (3, [2, 4])] {
        let (mut fx, _) = placeable();
        let id = widget(&mut fx);
        fx.world.lot_rng = GameRng::from_state(8);
        assert_eq!(find_random(&mut fx, 0, id, 0, rotation), Ok(true));
        let middle = tile.map(|t| (t << 4) | 8);
        assert_eq!(
            fx.world.objects[&id].position,
            [middle[0], middle[1], 1],
            "view {rotation}"
        );
    }
}
