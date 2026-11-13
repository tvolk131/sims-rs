//! Deleting objects and clearing the lot: the last reset, the unlinking,
//! the order, and the cases that fail loudly.

use crate::factory::{WIDGET, widgets};
use crate::support::*;
use sims_formats::execution::ObjectFunction;
use sims_formats::saved_objects::Slot;
use sims_vm::{Interpreter, Trap};

/// The widget's Reset tree.
const RESET: u16 = 4300;

/// Gives the widget type a callback table entry.
fn callback(fx: &mut Fixture, file: usize, index: usize, action: u16) {
    let library = fx.program.libraries.get_mut(&file).unwrap();
    let table = library.callbacks.get_mut(&128).unwrap();
    if table.len() <= index {
        table.resize(
            index + 1,
            ObjectFunction {
                condition: 0,
                action: 0,
            },
        );
    }
    table[index].action = action;
}

/// A lot that holds only widgets the factory made (IDs 1 to `count`). With
/// `reset`, the widget type has a Reset tree that stores the object's ID in
/// global 34 and adds 1 to global 36, the
/// two globals a script may write.
fn lot(count: u16, reset: bool) -> (Fixture, usize) {
    let (mut fx, file) = widgets();
    fx.world.objects.clear();
    fx.world.order.clear();
    (fx.world.globals[34], fx.world.globals[36]) = (0, 0);
    if reset {
        callback(&mut fx, file, 3, RESET);
        fx.add(
            file,
            RESET,
            tree(vec![
                assign(scope::GLOBAL, 34, scope::MY_DATA, 11, 1),
                ins(
                    2,
                    RETURN_TRUE,
                    RETURN_TRUE,
                    expr(scope::GLOBAL, 36, op::ADD, scope::LITERAL, 1),
                ),
            ]),
        );
    }
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    for id in 1..=count {
        assert_eq!(interpreter.create(&mut fx.world, WIDGET), Ok(id));
    }
    (fx, file)
}

#[test]
fn a_deleted_object_resets_one_last_time_and_is_unlinked() {
    let (mut fx, _) = lot(3, true);
    fx.world.instances.insert(WIDGET, vec![1, 2, 3]);
    fx.world.removals = vec![2];
    fx.world.event_owner = 2;
    let before = fx.world.clone();
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(interpreter.delete(&mut world, 2), Ok(()));
    // Its Reset tree ran.
    assert_eq!((world.globals[34], world.globals[36]), (2, 1));
    // It is gone from the registry and from its type's instance list.
    assert_eq!(world.objects.keys().copied().collect::<Vec<_>>(), [1, 3]);
    assert_eq!(world.order, [3, 1]);
    assert_eq!(world.instances[&WIDGET], [1, 3]);
    assert_eq!(world.event_owner, 0);
    // The removal queue holds bare IDs and is not this step's to edit.
    assert_eq!(world.removals, [2]);
    // Nothing else moved: the clock, the random streams, the other objects.
    assert_eq!(world.ticks, before.ticks);
    assert_eq!(
        (world.lot_rng, world.global_rng, world.crt_rng),
        (before.lot_rng, before.global_rng, before.crt_rng)
    );
    assert_eq!(world.objects[&1].data, before.objects[&1].data);
    // The freed ID is the next one the factory hands out.
    assert_eq!(interpreter.create(&mut world, WIDGET), Ok(2));
    assert_eq!(world.instances[&WIDGET], [1, 3, 2]);
}

#[test]
fn an_object_without_a_reset_tree_is_deleted_without_running_anything() {
    let (mut fx, _) = lot(2, false);
    fx.world.instances.insert(WIDGET, vec![1, 2]);
    let globals = fx.world.globals.clone();
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(interpreter.remove_now(&mut world, 1), Ok(()));
    assert_eq!(world.globals, globals);
    assert_eq!(world.order, [2]);
    // The instance list outlives its last instance.
    assert_eq!(interpreter.remove_now(&mut world, 2), Ok(()));
    assert_eq!(world.instances[&WIDGET], []);
    assert!(world.objects.is_empty());
    // A missing object is the original's error 10.
    let fault = interpreter.delete(&mut world, 2).unwrap_err();
    assert_eq!(fault.trap.code(), Some(10));
}

#[test]
fn a_reset_runs_the_tree_and_restarts_main() {
    let (mut fx, _) = lot(1, true);
    let widget = fx.world.objects.get_mut(&1).unwrap();
    assert_eq!(widget.countdown(), Some(19), "asleep in Main");
    (widget.data[8], widget.data[62]) = (0x0121, 7);
    let depth = widget.stack.len();
    assert!(depth > 1, "Main is inside the sleep tree");
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(interpreter.reset(&mut world, 1, false), Ok(()));
    let widget = &world.objects[&1];
    assert_eq!((world.globals[34], world.globals[36]), (1, 1));
    // Flag 0x20 and data word 62 are cleared; the thread is Main alone, at
    // its start; and the scheduler no longer counts the object down.
    assert_eq!((widget.data[8], widget.data[62]), (0x0101, 0));
    assert_eq!(widget.stack.len(), 1);
    assert_eq!((widget.stack[0].tree, widget.stack[0].node), (4097, 0));
    assert_eq!(widget.countdown(), Some(-1));
    assert_eq!(widget.flags(), fx.world.objects[&1].flags());
}

#[test]
fn a_reset_entry_whose_tree_is_missing_still_restarts_main() {
    let (mut fx, file) = lot(1, false);
    callback(&mut fx, file, 3, 4999);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(interpreter.reset(&mut world, 1, false), Ok(()));
    assert_eq!(world.objects[&1].stack.len(), 1);
    assert_eq!(world.objects[&1].countdown(), Some(-1));
}

#[test]
fn a_reset_without_a_tree_is_only_ported_for_a_deletion() {
    let (fx, _) = lot(1, false);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    let fault = interpreter.reset(&mut world, 1, false).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
    assert_eq!(world.encode(), fx.world.encode());
    // While deleting, the object leaves the lot and keeps its thread.
    let depth = world.objects[&1].stack.len();
    world.objects.get_mut(&1).unwrap().data[8] |= 0x100;
    assert_eq!(interpreter.reset(&mut world, 1, true), Ok(()));
    let widget = &world.objects[&1];
    assert_eq!(widget.stack.len(), depth);
    assert_eq!(widget.countdown(), Some(-1));
    assert_eq!((widget.data[8] & 0x100, widget.data[29]), (0, -5));
}

#[test]
fn a_reset_that_faults_leaves_the_object_registered() {
    let (mut fx, file) = lot(2, true);
    fx.add(file, RESET, tree(vec![ins(2, ERROR, ERROR, [0; 8])]));
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    let fault = interpreter.clear(&mut world).unwrap_err();
    assert_eq!(fault.object, 2, "the head of the list goes first");
    assert_eq!(world.order, [2, 1]);
}

#[test]
fn the_clear_takes_the_indicators_first_and_then_the_head_of_the_list() {
    let (mut fx, _) = lot(5, true);
    fx.world.indicators = [2, 4];
    fx.world.instances.insert(WIDGET, vec![1, 2, 3, 4, 5]);
    fx.world.removals = vec![3, 9];
    let before = fx.world.clone();
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(interpreter.clear(&mut world), Ok(vec![2, 4, 5, 3, 1]));
    // Every Reset tree ran, the last one on object 1.
    assert_eq!((world.globals[34], world.globals[36]), (1, 5));
    assert!(world.objects.is_empty() && world.order.is_empty());
    assert_eq!(world.indicators, [0, 0]);
    assert_eq!(world.instances[&WIDGET], []);
    // The queued removals wait for a later load step.
    assert_eq!(world.removals, [3, 9]);
    assert_eq!((world.ticks, world.size), (before.ticks, before.size));
    assert_eq!(world.global_rng, before.global_rng);
    // Clearing an empty lot does nothing.
    let cleared = world.clone();
    assert_eq!(interpreter.clear(&mut world), Ok(vec![]));
    assert_eq!(world.encode(), cleared.encode());
}

#[test]
fn a_reset_tree_that_removes_its_own_object_leaves_the_id_queued() {
    // Remove Object Instance on the object itself (mode 0).
    let (mut fx, file) = lot(2, true);
    fx.add(
        file,
        RESET,
        tree(vec![ins(18, RETURN_TRUE, RETURN_TRUE, [0; 8])]),
    );
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    // Each object's Reset removes the object itself. In a synchronous tree
    // that queues the ID and goes on, so the ID outlives the object.
    assert_eq!(interpreter.clear(&mut world), Ok(vec![2, 1]));
    assert_eq!(world.removals, [2, 1]);
    assert!(world.objects.is_empty());
}

#[test]
fn deletions_that_need_more_of_the_lot_fail_loudly() {
    let unsupported = |prepare: fn(&mut Fixture, usize)| {
        let (mut fx, file) = lot(2, true);
        prepare(&mut fx, file);
        let before = fx.world.clone();
        let interpreter = fx.interpreter();
        let mut world = fx.world.clone();
        let fault = interpreter.remove_now(&mut world, 1).unwrap_err();
        assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
        assert_eq!(fault.object, 1);
        assert_eq!(world.encode(), before.encode(), "nothing ran");
    };
    // Standing on the lot.
    unsupported(|fx, _| fx.world.objects.get_mut(&1).unwrap().position = [24, 24, 1]);
    // In a container.
    unsupported(|fx, _| fx.world.objects.get_mut(&1).unwrap().data[2] = 2);
    // Holding something.
    unsupported(|fx, _| {
        fx.world.objects.get_mut(&1).unwrap().slots.push(Slot {
            unknown: 0,
            object: 2,
        });
    });
    // With a callback for leaving the lot.
    unsupported(|fx, file| callback(fx, file, 26, 4400));
    // A person on the lot would have to drop their references to it.
    let (mut fx, _) = lot(1, true);
    fx.add_sim(9);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    let fault = interpreter.delete(&mut world, 1).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
    assert_eq!(world.globals[36], 0, "its Reset did not run");
    // A person is not deleted this way at all.
    let fault = interpreter.delete(&mut world, 9).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
}
