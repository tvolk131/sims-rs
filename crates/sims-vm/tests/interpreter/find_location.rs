//! Find Location For (opcode 16, 0x44b3c8) and Create New Object Instance
//! mode 8 (0x44e0c8), which both put an object on the lot after the place
//! test (0x433311).
//!
//! The lot is the six-tile one of the room tests: room 0 outside, room 1
//! the four tiles (2, 2) to (3, 3) inside walls. Its tiles have no floor,
//! so the objects here may stand without one (placement flag 2, data word
//! 42).

use crate::factory::{WIDGET, widgets};
use crate::rooms::{array, build};
use crate::support::*;
use sims_vm::Trap;

/// The opcodes under test.
const FIND_LOCATION: u16 = 16;
const CREATE: u16 = 42;

/// A widget's Init: it may be placed on level 1 (data word 4 bit 0) and
/// stand on a tile with no floor (data word 42 flag 2).
fn placeable() -> (Fixture, usize) {
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

fn widget(fx: &mut Fixture) -> u16 {
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

fn find(fx: &mut Fixture, local: i16, target: u16, flags: u8) -> Result<bool, sims_vm::Fault> {
    run(
        fx,
        local,
        target as i16,
        FIND_LOCATION,
        [0, 0, flags, 0, 0, 0, 0, 0],
    )
}

fn create(
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
fn other_find_location_modes_fail_loudly() {
    let (mut fx, _) = placeable();
    let id = widget(&mut fx);
    for mode in 1..=5 {
        let result = run(
            &mut fx,
            0,
            id as i16,
            FIND_LOCATION,
            [mode, 0, 0, 0, 0, 0, 0, 0],
        );
        assert!(matches!(trap(result), Trap::Unsupported(m) if m.contains("not ported")));
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
fn mode_8_with_other_flags_or_a_person_fails_loudly() {
    let (mut fx, _) = placeable();
    fx.lamp_mut().position = [40, 40, 1];
    let result = create(&mut fx, 0, 0x04, SIM as i16);
    assert!(matches!(trap(result), Trap::Unsupported(_)));
    let (mut fx, file) = placeable();
    fx.lamp_mut().position = [40, 40, 1];
    fx.words(file)[9] = 2;
    let result = create(&mut fx, 0, 0, SIM as i16);
    assert!(matches!(trap(result), Trap::Unsupported(m) if m.contains("person")));
}
