//! Remove Object Instance: queueing, yielding and the cases that need the
//! lot.

use crate::factory::{WIDGET, widgets};
use crate::support::*;
use sims_vm::{Interpreter, Status, Trap};

const BUDGET: usize = 100;

/// Remove Object Instance, then set attribute 3 to 1 on either branch but
/// return what the primitive said.
fn remover(mode: i16, flags: u8) -> sims_formats::behavior::Behavior {
    let [low, high] = mode.to_le_bytes();
    tree(vec![
        ins(18, 1, 2, [low, high, flags, 0, 0, 0, 0, 0]),
        assign(scope::MY_ATTRIBUTES, 3, scope::LITERAL, 1, RETURN_TRUE),
        assign(scope::MY_ATTRIBUTES, 3, scope::LITERAL, 1, RETURN_FALSE),
    ])
}

/// A fixture with two widgets (3 and 4) the factory made, off the lot.
fn two() -> (Fixture, usize) {
    let (mut fx, file) = widgets();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.create(&mut fx.world, WIDGET), Ok(3));
    assert_eq!(interpreter.create(&mut fx.world, WIDGET), Ok(4));
    (fx, file)
}

#[test]
fn an_object_that_removes_itself_yields_and_stays_registered() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(0, 0));
    fx.thread(3, 4200, &[0; 4]);
    let widget = fx.world.objects.get_mut(&3).unwrap();
    (widget.data[8], widget.data[29]) = (0x0101, 6);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(interpreter.turn(&mut world, 3, BUDGET), Ok(Status::Yielded));
    assert_eq!(world.removals, [3]);
    let widget = &world.objects[&3];
    // Detached: no container, no slot, no room, and flag 0x100 cleared.
    assert_eq!(
        (widget.data[2], widget.data[3], widget.data[14]),
        (0, -1, -1)
    );
    assert_eq!(
        (widget.data[26], widget.data[29], widget.data[8]),
        (0, -5, 1)
    );
    assert!(!widget.removal_skips_resets);
    // Its tree went no further, and the object is still on the lot.
    assert_eq!(widget.attributes[3], 0);
    assert_eq!(widget.stack[0].node, 0);
    assert!(world.order.contains(&3));
    // Running it again queues nothing new.
    assert_eq!(interpreter.turn(&mut world, 3, BUDGET), Ok(Status::Yielded));
    assert_eq!(world.removals, [3]);
}

#[test]
fn flag_bit_0_lets_the_tree_go_on() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(0, 1));
    fx.thread(3, 4200, &[0; 4]);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(
        interpreter.turn(&mut world, 3, BUDGET),
        Ok(Status::Returned(true))
    );
    assert_eq!(world.removals, [3]);
    assert_eq!(world.objects[&3].attributes[3], 1);
}

#[test]
fn flag_bit_1_is_recorded_on_the_object() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(0, 3));
    fx.thread(3, 4200, &[0; 4]);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    interpreter.turn(&mut world, 3, BUDGET).unwrap();
    assert!(world.objects[&3].removal_skips_resets);
    assert!(!world.objects[&4].removal_skips_resets);
}

#[test]
fn a_synchronous_tree_cannot_yield_and_takes_the_true_branch() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(0, 0));
    assert_eq!(fx.run_with(3, 4200, 0, &[], BUDGET), Ok(true));
    assert_eq!(fx.world.removals, [3]);
}

#[test]
fn another_object_is_removed_without_yielding() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(1, 0));
    fx.thread(3, 4200, &[0; 4]);
    fx.world.objects.get_mut(&3).unwrap().stack[0].stack_object = 4;
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(
        interpreter.turn(&mut world, 3, BUDGET),
        Ok(Status::Returned(true))
    );
    assert_eq!(world.removals, [4]);
    // Requests keep their order.
    fx.world = world;
    fx.add(file, 4200, remover(0, 1));
    fx.thread(3, 4200, &[0; 4]);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    interpreter.turn(&mut world, 3, BUDGET).unwrap();
    assert_eq!(world.removals, [4, 3]);
}

#[test]
fn a_missing_object_is_the_false_branch() {
    // Mode 2 names object 0, which never exists; stack object 40 is gone.
    for (mode, stack_object) in [(2, 4), (1, 40), (1, 0), (1, -3)] {
        let (mut fx, file) = two();
        fx.add(file, 4200, remover(mode, 0));
        assert_eq!(fx.run_with(3, 4200, stack_object, &[], BUDGET), Ok(false));
        assert!(fx.world.removals.is_empty());
    }
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(3, 0));
    assert_eq!(code(fx.run_with(3, 4200, 0, &[], BUDGET)), Some(9));
}

#[test]
fn objects_that_need_the_lot_to_leave_it_fail_loudly() {
    let unsupported = |prepare: fn(&mut Fixture, usize)| {
        let (mut fx, file) = two();
        fx.add(file, 4200, remover(1, 0));
        prepare(&mut fx, file);
        let fault = fx.run_with(3, 4200, 4, &[], BUDGET).unwrap_err();
        assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
        assert!(fx.world.removals.is_empty());
    };
    // Standing on the lot.
    unsupported(|fx, _| fx.world.objects.get_mut(&4).unwrap().position = [24, 24, 1]);
    // In a container.
    unsupported(|fx, _| fx.world.objects.get_mut(&4).unwrap().data[2] = 3);
    // Holding something.
    unsupported(|fx, _| {
        let widget = fx.world.objects.get_mut(&4).unwrap();
        widget.slots.push(sims_formats::saved_objects::Slot {
            unknown: 0,
            object: 3,
        });
    });
    // With a callback for leaving the lot.
    unsupported(|fx, file| {
        let callbacks = fx.program.libraries.get_mut(&file).unwrap();
        let table = callbacks.callbacks.get_mut(&128).unwrap();
        table.resize(
            11,
            sims_formats::execution::ObjectFunction {
                condition: 0,
                action: 0,
            },
        );
        table[10].action = 4300;
    });
    // A person: the fixture's Sim, removed by the lamp.
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, remover(1, 0));
    assert!(matches!(trap(fx.run()), Trap::Unsupported(_)));
}
