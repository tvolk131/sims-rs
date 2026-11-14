//! Load step 10: every object gets its room, then every object is told
//! that the rooms changed.

use crate::factory::{WIDGET, widgets};
use crate::support::*;
use sims_formats::execution::ObjectFunction;
use sims_formats::lot::Array;
use sims_vm::{Interpreter, Roomed, Trap};

/// The widget's function 7 tree.
const ROOMS_CHANGED: u16 = 4200;

fn array(stride: usize, fill: u8) -> Array {
    Array {
        width: 64,
        height: 64,
        stride,
        encoding: 0,
        data: vec![fill; 64 * 64 * stride],
    }
}

/// Gives the lot six flat tiles a side, with walls around the tiles 2 and
/// 3 in both directions, and works out its rooms: 0 outside the walls, 1
/// inside, 1024 upstairs.
fn build(fx: &mut Fixture) {
    let world = &mut fx.world;
    world.size = 6;
    world.arrays.insert(0, array(4, 0));
    for (floors, walls, flags) in [(1, 2, 8), (101, 102, 108)] {
        world.arrays.insert(floors, array(2, 0));
        world.arrays.insert(walls, array(14, 0));
        world.arrays.insert(flags, array(1, 0));
    }
    let walls = &mut world.arrays.get_mut(&2).unwrap().data;
    let mut wall = |x: usize, y: usize, bit: u8, style: usize| {
        let cell = (x * 64 + y) * 14;
        walls[cell] |= bit;
        walls[cell + 2 + style * 2] = 1;
    };
    for n in 2..4 {
        // West walls with their mirrored east walls, then north and south.
        wall(2, n, 1, 0);
        wall(1, n, 4, 0);
        wall(4, n, 1, 0);
        wall(3, n, 4, 0);
        wall(n, 2, 2, 1);
        wall(n, 1, 8, 1);
        wall(n, 4, 2, 1);
        wall(n, 3, 8, 1);
    }
    world.rebuild_rooms().unwrap();
    assert_eq!(world.rooms.rooms.len(), 3);
}

/// Makes `tree` the function 7 of the one type in `file`.
fn on_rooms_changed(fx: &mut Fixture, file: usize, tree: u16) {
    let callbacks = fx
        .program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap();
    callbacks.resize(
        8,
        ObjectFunction {
            condition: 0,
            action: 0,
        },
    );
    callbacks[7].action = tree;
}

fn assign_rooms(fx: &mut Fixture) -> Result<Roomed, sims_vm::Fault> {
    Interpreter::new(&fx.program, &fx.catalog, fx.stream).assign_rooms(&mut fx.world)
}

#[test]
fn every_object_gets_the_room_it_stands_in() {
    let mut fx = Fixture::new();
    build(&mut fx);
    // The lamp is inside the walls, on level 0, which becomes level 1.
    // The Sim is upstairs. A third object is off the lot.
    fx.lamp_mut().position = [2 * 16 + 8, 3 * 16 + 8, 0];
    fx.sim_mut().position = [4 * 16, 4 * 16, 2];
    fx.add_object(9).position = [-16, -16, 1];
    let before = fx.world.clone();
    assert_eq!(
        assign_rooms(&mut fx),
        Ok(Roomed {
            moved: vec![(LAMP, 58, 1), (SIM, 58, 1024), (9, 58, -5)],
            asked: vec![],
        })
    );
    assert_eq!(fx.lamp().data[29], 1);
    assert_eq!(fx.lamp().position, [2 * 16 + 8, 3 * 16 + 8, 1]);
    assert_eq!(fx.sim().data[29], 1024);
    assert_eq!(fx.world.objects[&9].data[29], -5);
    // Nothing else changes, and a second pass changes nothing at all.
    let mut expected = before;
    for (id, room) in [(LAMP, 1), (SIM, 1024), (9, -5)] {
        expected.objects.get_mut(&id).unwrap().data[29] = room;
    }
    expected.objects.get_mut(&LAMP).unwrap().position[2] = 1;
    assert_eq!(fx.world.encode(), expected.encode());
    assert_eq!(assign_rooms(&mut fx), Ok(Roomed::default()));
    assert_eq!(fx.world.encode(), expected.encode());
}

#[test]
fn every_object_is_told_after_all_have_their_rooms() {
    let (mut fx, file) = widgets();
    build(&mut fx);
    on_rooms_changed(&mut fx, file, ROOMS_CHANGED);
    // The tree counts the calls in the lamp's attribute 0, notes the
    // count and the lamp's room in its own attributes, and notes its first
    // argument.
    fx.add(
        file,
        ROOMS_CHANGED,
        tree(vec![
            assign(scope::STACK_OBJECT, 0, scope::LITERAL, LAMP as i16, 1),
            ins(
                2,
                2,
                2,
                expr(scope::STACK_ATTRIBUTES, 0, op::ADD, scope::LITERAL, 1),
            ),
            assign(scope::MY_ATTRIBUTES, 0, scope::STACK_ATTRIBUTES, 0, 3),
            assign(scope::MY_ATTRIBUTES, 1, scope::STACK_DATA, 29, 4),
            assign(scope::MY_ATTRIBUTES, 2, scope::PARAMETERS, 0, RETURN_FALSE),
        ]),
    );
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.create(&mut fx.world, WIDGET), Ok(3));
    assert_eq!(interpreter.create(&mut fx.world, WIDGET), Ok(4));
    // Registration order: 4, 3, the lamp, the Sim. The lamp is registered
    // after both widgets and stands inside the walls.
    assert_eq!(fx.world.order, [4, 3, LAMP, SIM]);
    fx.lamp_mut().position = [3 * 16, 3 * 16, 1];
    fx.lamp_mut().attributes[0] = 0;
    fx.world.objects.get_mut(&3).unwrap().temps[0] = 77;
    let depth = fx.world.objects[&3].stack.len();
    let done = assign_rooms(&mut fx).unwrap();
    // Only the widgets have a function 7. The lamp and the Sim have a
    // table too short to name one.
    assert_eq!(done.asked, [4, 3]);
    // The widgets are off the lot and already had no room.
    assert_eq!(done.moved, [(LAMP, 58, 1), (SIM, 58, -5)]);
    let (first, second) = (&fx.world.objects[&4], &fx.world.objects[&3]);
    assert_eq!(first.attributes[..3], [1, 1, 0]);
    // The tree's arguments are the object's temps.
    assert_eq!(second.attributes[..3], [2, 1, 77]);
    // The tree ran on top of the object's thread and left it as it was,
    // but for the last result.
    assert_eq!(second.stack.len(), depth);
    assert!(!second.last_result);
    assert_eq!(second.countdown(), Some(19));
}

#[test]
fn a_function_7_that_names_no_tree_runs_nothing() {
    let (mut fx, file) = widgets();
    build(&mut fx);
    on_rooms_changed(&mut fx, file, ROOMS_CHANGED);
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.create(&mut fx.world, WIDGET), Ok(3));
    let before = format!("{:?}", fx.world.objects[&3]);
    let done = assign_rooms(&mut fx).unwrap();
    assert_eq!(done.asked, []);
    assert_eq!(format!("{:?}", fx.world.objects[&3]), before);
}

#[test]
fn a_tree_that_faults_stops_the_step() {
    let (mut fx, file) = widgets();
    build(&mut fx);
    on_rooms_changed(&mut fx, file, ROOMS_CHANGED);
    // A Sleep cannot wait inside a callback.
    fx.add(
        file,
        ROOMS_CHANGED,
        tree(vec![ins(256, RETURN_TRUE, ERROR, [5, 0, 0, 0, 0, 0, 0, 0])]),
    );
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.create(&mut fx.world, WIDGET), Ok(3));
    let fault = assign_rooms(&mut fx).unwrap_err();
    assert_eq!((fault.object, fault.trap.code()), (3, Some(0x44f)));
}

#[test]
fn rooms_must_be_worked_out_first() {
    let mut fx = Fixture::new();
    fx.lamp_mut().position = [40, 40, 1];
    let fault = assign_rooms(&mut fx).unwrap_err();
    assert_eq!(fault.object, LAMP);
    assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
    // An object on a level the lot does not have.
    build(&mut fx);
    fx.lamp_mut().position = [40, 40, 3];
    let fault = assign_rooms(&mut fx).unwrap_err();
    assert!(fault.to_string().contains("level 3"), "{fault}");
}
