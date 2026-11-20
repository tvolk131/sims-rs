//! Deleting objects and clearing the lot: the last reset, the unlinking,
//! the order, and the cases that fail loudly.

use crate::factory::{WIDGET, widgets};
use crate::support::*;
use sims_formats::execution::ObjectFunction;
use sims_formats::saved_objects::{Relationship, Slot};
use sims_sim::neighbors::{Neighbor, Row};
use sims_vm::{Interpreter, SoundEvent, Trap};
use std::cell::RefCell;

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
    fx.world.people.clear();
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
fn deletion_erases_incoming_object_relationships_including_a_persons() {
    let (mut fx, _) = lot(3, false);
    fx.add_sim(9);
    let row = |target| Relationship {
        present: 1,
        target,
        values: vec![target, 5],
    };
    fx.world.objects.get_mut(&1).unwrap().relationships = vec![row(9), row(2), row(3)];
    fx.world.objects.get_mut(&2).unwrap().relationships = vec![row(1)];
    fx.world.objects.get_mut(&9).unwrap().relationships = vec![row(2)];
    fx.world.neighbors.slots = vec![Some(Neighbor {
        id: 1,
        guid: SIM_GUID,
        revision: 0,
        data: Vec::new(),
        relationships: vec![Row {
            target: 2,
            values: vec![42],
        }],
        friends: 0,
        stale: false,
    })];
    let mut world = fx.world.clone();
    fx.interpreter().delete(&mut world, 2).unwrap();
    let rows: Vec<_> = world.objects[&1]
        .relationships
        .iter()
        .map(|row| (row.target, row.values.as_slice()))
        .collect();
    assert_eq!(rows, [(9, &[9, 5][..]), (3, &[3, 5][..])]);
    assert!(world.objects[&9].relationships.is_empty());
    assert_eq!(world.neighbors, fx.world.neighbors);
}

#[test]
fn an_object_without_a_reset_tree_is_deleted_without_running_anything() {
    let (mut fx, _) = lot(2, false);
    fx.world.instances.insert(WIDGET, vec![1, 2]);
    // Removal takes an object out of the person vector too.
    fx.world.people = vec![1, 2];
    let globals = fx.world.globals.clone();
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(interpreter.remove_now(&mut world, 1), Ok(()));
    assert_eq!(world.globals, globals);
    assert_eq!(world.order, [2]);
    assert_eq!(world.people, [2]);
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
    // A person is not deleted this way at all.
    let (mut fx, _) = lot(1, true);
    fx.add_sim(9);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    let fault = interpreter.delete(&mut world, 9).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
}

#[test]
fn people_let_go_of_a_deleted_object_and_others_are_left_alone() {
    let (mut fx, _) = lot(2, true);
    fx.add_sim(9);
    // The person's actions name the other widget and nothing.
    let person = fx
        .world
        .objects
        .get_mut(&9)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    person.active.target = 2;
    person.last.icon = 2;
    let sim = fx.world.objects[&9].clone();
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(interpreter.delete(&mut world, 1), Ok(()));
    assert_eq!(world.globals[36], 1, "its Reset ran");
    assert_eq!(world.order, [2, 9]);
    assert_eq!(format!("{:?}", world.objects[&9]), format!("{sim:?}"));
    // Deleting the widget the person does refer to: the person lets go of
    // it before its Reset runs. The last action shows it as its icon,
    // which is cleared; then the running action ends and becomes the last.
    world
        .objects
        .get_mut(&9)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[73] = 0;
    assert_eq!(interpreter.delete(&mut world, 2), Ok(()));
    let person = world.objects[&9].person.as_ref().unwrap();
    assert_eq!((person.active.target, person.active.table_index), (0, -1));
    assert_eq!((person.last.target, person.last.icon), (2, 0));
    assert_eq!(world.order, [9]);
}

#[test]
fn a_reset_stops_the_objects_sounds_and_a_deletion_stops_them_twice() {
    let (fx, _) = lot(2, true);
    let sink = RefCell::new(Vec::new());
    let mut interpreter = fx.interpreter();
    interpreter.sounds = Some(&sink);
    let mut world = fx.world.clone();
    assert_eq!(interpreter.reset(&mut world, 1, false), Ok(()));
    assert_eq!(sink.take(), [SoundEvent::Stop { object: 1 }]);
    // The deletion's own reset stops them (0x4320b4), and so does its
    // destructor (0x4323c9).
    assert_eq!(interpreter.delete(&mut world, 2), Ok(()));
    let stop = SoundEvent::Stop { object: 2 };
    assert_eq!(sink.take(), [stop.clone(), stop]);
    // Without a sink nothing is noted, and the lot ends the same.
    let mut quiet = fx.world.clone();
    let plain = fx.interpreter();
    assert_eq!(plain.reset(&mut quiet, 1, false), Ok(()));
    assert_eq!(plain.delete(&mut quiet, 2), Ok(()));
    assert_eq!(quiet.hash(), world.hash());
}

#[test]
fn a_person_lets_go_of_queued_actions_on_a_deleted_object_telling_each_target() {
    // The widget's function 4 (cancelled) notes its stack object in global
    // 34 and counts in global 36.
    let (mut fx, file) = lot(2, false);
    callback(&mut fx, file, 4, 4301);
    fx.add(
        file,
        4301,
        tree(vec![
            assign(scope::GLOBAL, 34, scope::STACK_OBJECT, 0, 1),
            ins(
                2,
                RETURN_TRUE,
                RETURN_TRUE,
                expr(scope::GLOBAL, 36, op::ADD, scope::LITERAL, 1),
            ),
        ]),
    );
    fx.add_sim(9);
    let person = fx
        .world
        .objects
        .get_mut(&9)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    let action = |target: i16, icon: i16, uid: i32| {
        let mut action = person.last.clone();
        (action.target, action.icon, action.uid) = (target, icon, uid);
        action
    };
    // On widget 2; on widget 1, kept; on widget 1 showing widget 2's icon;
    // on nothing showing widget 2's icon.
    person.queue = vec![
        action(2, 2, 1),
        action(1, 1, 2),
        action(1, 2, 3),
        action(0, 2, 4),
    ];
    let asked = RefCell::new(Vec::new());
    let mut interpreter = fx.interpreter();
    interpreter.asked = Some(&asked);
    let mut world = fx.world.clone();
    assert_eq!(interpreter.delete(&mut world, 2), Ok(()));
    let queue = &world.objects[&9].person.as_ref().unwrap().queue;
    let uids: Vec<i32> = queue.iter().map(|a| a.uid).collect();
    assert_eq!(uids, [2]);
    // Widget 2 is told of its own action, then widget 1 of the one showing
    // widget 2; the action on nothing tells no one.
    let cancels: Vec<(u16, usize)> = asked
        .take()
        .into_iter()
        .filter(|a| a.function == 4)
        .map(|a| (a.object, a.function))
        .collect();
    assert_eq!(cancels, [(2, 4), (1, 4)]);
    assert_eq!((world.globals[34], world.globals[36]), (9, 2));
}

#[test]
fn only_the_lots_people_are_told() {
    // A person object that is not on the lot's list of people keeps its
    // action on the deleted widget.
    let (mut fx, _) = lot(1, false);
    fx.add_sim(9);
    fx.world.people.clear();
    let person = fx
        .world
        .objects
        .get_mut(&9)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    person.last.target = 1;
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(interpreter.delete(&mut world, 1), Ok(()));
    assert_eq!(world.objects[&9].person.as_ref().unwrap().last.target, 1);
}

#[test]
fn a_failed_cancel_keeps_its_place_and_the_queue_its_ring() {
    // Widget 2's function 4 (cancelled) queues widget 1's menu 3 on the
    // person eight times, filling the queue, then runs a tree that does
    // not exist.
    let (mut fx, file) = lot(2, false);
    let library = fx.program.libraries.get_mut(&file).unwrap();
    library.definitions.get_mut(&128).unwrap()[7] = 1007;
    library.interactions.insert(
        1007,
        Some(sims_formats::behavior::InteractionTable {
            version: 7,
            interactions: vec![sims_formats::behavior::Interaction {
                action: 4302,
                test: 0,
                string_index: 3,
                flags: 0,
                autonomy: 50,
                attenuation_code: 0,
                attenuation: 0.0,
                joining_index: -1,
                motives: Vec::new(),
                flags2: 0,
            }],
        }),
    );
    callback(&mut fx, file, 4, 4301);
    let mut nodes = vec![assign(scope::LOCALS, 0, scope::LITERAL, 1, 1)];
    for node in 1..=8u8 {
        nodes.push(ins(13, node + 1, ERROR, [3, 0, 1, 2, 0, 0, 0, 0]));
    }
    nodes.push(ins(0x1fff, RETURN_TRUE, RETURN_TRUE, [0; 8]));
    fx.add(file, 4301, tree(nodes));
    fx.add_sim(9);
    let person = fx
        .world
        .objects
        .get_mut(&9)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    person.data[32] = 0;
    let action = |target: i16, uid: i32| {
        let mut action = person.last.clone();
        (action.target, action.icon, action.uid) = (target, target, uid);
        action
    };
    // One kept on widget 1, then one on widget 2.
    person.queue = vec![action(1, 1), action(2, 2)];
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    let fault = interpreter.delete(&mut world, 2).unwrap_err();
    // The fault names the cancel's tree, not the person.
    assert_eq!(fault.object, 2, "{fault}");
    assert_eq!(fault.at.map(|at| at.tree), Some(4301), "{fault}");
    // The kept action goes back in front, and the queue holds 8.
    let queue = &world.objects[&9].person.as_ref().unwrap().queue;
    assert_eq!(queue.len(), 8, "{fault}");
    assert_eq!(queue[0].uid, 1);
}
