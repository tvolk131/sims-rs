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
fn deletion_erases_only_the_first_matching_row_and_passes_empty_rows_after_it() {
    let (mut fx, _) = lot(3, false);
    let row = |present, target| Relationship {
        present,
        target,
        values: vec![target],
    };
    // 0x464fa2 stops at the first row naming the deleted ID (0x464fbd),
    // so an empty row after it is never read and a second match stays.
    fx.world.objects.get_mut(&1).unwrap().relationships = vec![row(1, 2), row(0, 0), row(1, 2)];
    let mut world = fx.world.clone();
    fx.interpreter().delete(&mut world, 2).unwrap();
    let rows: Vec<_> = world.objects[&1]
        .relationships
        .iter()
        .map(|row| (row.present, row.target))
        .collect();
    assert_eq!(rows, [(0, 0), (1, 2)]);
}

#[test]
fn deletion_fails_on_an_empty_row_before_the_match() {
    let (mut fx, _) = lot(3, false);
    let row = |present, target| Relationship {
        present,
        target,
        values: vec![target],
    };
    // The original reads each row through its pointer (0x464fb1), so an
    // empty row before the match would fault there.
    fx.world.objects.get_mut(&1).unwrap().relationships = vec![row(0, 0), row(1, 2)];
    let mut world = fx.world.clone();
    let error = fx.interpreter().delete(&mut world, 2).unwrap_err();
    assert!(format!("{error:?}").contains("0x464fb1"), "{error:?}");
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
    assert_eq!(world.instances[&WIDGET], [0u16; 0]);
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
    assert_eq!(world.instances[&WIDGET], [0u16; 0]);
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

#[test]
fn deleting_an_object_that_a_person_is_routing_to_succeeds_without_trapping() {
    let (mut fx, _) = lot(2, false);
    fx.add_sim(9);
    // Give Sim 9 an active route context whose target is widget 2 (`0x440fff`
    // deletes widget 2 directly; `cXPerson::CheckGoal` / `go_to_slot.rs` at
    // `0x45ae8d` handles the missing target via LOST / LOST_UNDER_WAY).
    let ctx = sims_sim::route::Context {
        candidates: Vec::new(),
        slot: sims_sim::route::RouteSlot {
            offset: [0; 3],
            number: 1,
            standing: 0,
            sitting: 0,
            ground: 0,
            flags: 0,
            snap_target: 0,
            proximity: [0; 3],
            gradient: 0,
            facing: 0,
            resolution: 0,
        },
        target: 2,
        person: 9,
        chosen: -1,
        best: -1,
        word_58: 0,
        time: 0,
        saved: [0; 2],
        word_68: 0,
        start: [0; 3],
        byte_78: false,
        objects: [0; 2],
        byte_84: false,
        result: 0,
        word_8c: 0,
        byte_8e: false,
        word_94: 0,
        limit: 0x30,
        action: 0,
    };
    let route = sims_sim::route::Route {
        contexts: vec![ctx],
        ..Default::default()
    };
    fx.world.routing.routes.insert(9, route);

    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(interpreter.delete(&mut world, 2), Ok(()));
    assert!(!world.objects.contains_key(&2));
    assert!(world.objects.contains_key(&1));
    assert!(world.objects.contains_key(&9));
    // P1 regression: surviving route context is stamped with u16::MAX so even
    // if `create` immediately reuses freed ID 2, the route cannot target the
    // newly allocated object.
    assert_eq!(world.routing.routes[&9].contexts[0].target, u16::MAX);
    assert_eq!(interpreter.create(&mut world, WIDGET), Ok(2));
    assert_eq!(world.routing.routes[&9].contexts[0].target, u16::MAX);
}

fn place_test_person(fx: &mut Fixture, person_id: u16) {
    fx.add(SIM_FILE, 4096, sleeper());
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .callbacks
        .insert(
            128,
            vec![
                ObjectFunction {
                    condition: 0,
                    action: 0,
                },
                ObjectFunction {
                    condition: 0,
                    action: 4096,
                },
            ],
        );
    let position = [2 * 16 + 8, 3 * 16 + 8, 1];
    let room = fx.world.room_at(position).unwrap();
    let sim = fx.add_sim(person_id);
    sim.position = position;
    sim.data[29] = room;
    (sim.data[2], sim.data[3], sim.data[14], sim.data[26]) = (0, -1, -1, 0);
    sim.slots = vec![
        Slot {
            unknown: 0,
            object: 0,
        };
        6
    ];
    let person = sim.person.as_mut().unwrap();
    person.data[73] = 0;
    person.data[77] = 0; // no active date
    person.data[81] = 0; // not famous
    fx.program
        .people
        .profiles
        .insert(SIM_GUID, sims_content::Profile::default());
    let cell = ((2 * 64 + 3) * 2) as usize;
    fx.world.arrays.get_mut(&3).unwrap().data[cell..cell + 2]
        .copy_from_slice(&person_id.to_le_bytes());
}

#[test]
fn deleting_a_person_handles_head_slot_indicator_without_leaving_stale_indicator_id() {
    let (mut fx, _) = crate::visits::lot();
    let person_id = 50u16;
    place_test_person(&mut fx, person_id);

    let interpreter = fx.interpreter();

    // 1. Deleting a *selected* person deselects it (`0x445449` -> `deselect`),
    // taking `indicators[0]` off-lot before `0x4410e7` so the plumb bob
    // survives and remains registered in `world.indicators[0]`.
    {
        let mut world = fx.world.clone();
        let [first, _second] = interpreter.create_indicators(&mut world).unwrap();
        assert_eq!(interpreter.select(&mut world, Some(person_id)), Ok(()));
        assert_eq!(world.objects[&person_id].slots[2].object, first as i16);
        assert_eq!(interpreter.remove_now(&mut world, person_id), Ok(()));
        assert!(!world.objects.contains_key(&person_id));
        assert!(
            world.objects.contains_key(&first),
            "indicator {first} must survive selected-person deletion"
        );
        assert_eq!(world.indicators[0], first);
        assert_eq!(world.objects[&first].data[2], 0);
        assert_eq!(world.objects[&first].position, [-16, -16, 1]);
    }

    // 2. Deleting a *non-selected* person that still holds `first` in slot 2
    // follows native `0x4410e7` + `0x447d8b -> 0x448097`: `first` is deleted
    // as a held child and unregistered from `world.indicators[0]` (zeroed), so
    // subsequent selection/deselection never raises `missing object (0x0a)`.
    {
        let mut world = fx.world.clone();
        let [first, _second] = interpreter.create_indicators(&mut world).unwrap();
        world.objects.get_mut(&person_id).unwrap().selected = false;
        world.objects.get_mut(&person_id).unwrap().slots[2].object = first as i16;
        world.objects.get_mut(&first).unwrap().data[2] = person_id as i16;
        world.objects.get_mut(&first).unwrap().data[3] = 1;
        world.objects.get_mut(&first).unwrap().data[14] = 2;
        world.objects.get_mut(&first).unwrap().data[26] = person_id as i16;

        assert_eq!(interpreter.remove_now(&mut world, person_id), Ok(()));
        assert!(!world.objects.contains_key(&person_id));
        assert!(!world.objects.contains_key(&first));
        assert_eq!(
            world.indicators[0], 0,
            "0x447d8b -> 0x448097 must unregister deleted indicator {first}"
        );
        // Selecting / deselecting nobody now succeeds without 0x0a.
        assert_eq!(interpreter.select(&mut world, None), Ok(()));
    }

    // 3. Direct `remove_now` on an indicator unregisters `world.indicators`
    // BEFORE callbacks run (`0x447d8b -> 0x448097`), so a Reset callback on
    // the indicator already observes `world.indicators[0] == 0`.
    {
        let mut world = fx.world.clone();
        let [first, _second] = interpreter.create_indicators(&mut world).unwrap();
        assert_eq!(world.indicators[0], first);
        // If `remove_now` faults during Reset, `0x447d8b` has already run
        // before physical removal/Reset (`legacy/.../immediate.rs:38`), so
        // `world.indicators[0]` is already unregistered (0).
        let ind_guid = world.objects[&first].guid;
        let def = fx
            .catalog
            .objects
            .iter()
            .find(|o| o.guid == ind_guid)
            .unwrap()
            .clone();
        fx.add(def.resource, 4099, tree(vec![ins(2, ERROR, ERROR, [0; 8])]));
        fx.program
            .libraries
            .get_mut(&def.resource)
            .unwrap()
            .callbacks
            .insert(
                def.chunk,
                [0, 0, 0, 4099]
                    .map(|action| ObjectFunction {
                        condition: 0,
                        action,
                    })
                    .to_vec(),
            );
        assert!(fx.interpreter().remove_now(&mut world, first).is_err());
        assert_eq!(
            world.indicators[0], 0,
            "0x447d8b must unregister indicator before Reset/callbacks run"
        );
    }
}

#[test]
fn deleting_an_object_skips_stale_missing_targets_in_person_queues() {
    let (mut fx, _) = lot(2, false);
    fx.add_sim(9);
    let person = fx
        .world
        .objects
        .get_mut(&9)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    let mut stale_icon_match = person.last.clone();
    // Queued action whose icon is widget 2 (about to be deleted), while its
    // target 383 is already absent from `world.objects`.
    stale_icon_match.target = 383;
    stale_icon_match.icon = 2;
    stale_icon_match.uid = 10;
    let mut stale_direct = person.last.clone();
    stale_direct.target = 383;
    stale_direct.icon = 383;
    stale_direct.uid = 11;
    person.queue = vec![stale_icon_match, stale_direct];

    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    // `drop_references` -> `filter_queue` drops the action naming widget 2 as
    // icon without throwing `object 383: missing object (native error 0x0a)`.
    assert_eq!(interpreter.delete(&mut world, 2), Ok(()));
    assert!(!world.objects.contains_key(&2));
    let remaining_uids: Vec<i32> = world.objects[&9]
        .person
        .as_ref()
        .unwrap()
        .queue
        .iter()
        .map(|a| a.uid)
        .collect();
    assert_eq!(remaining_uids, [11]);

    // And deleting a person whose queue contains a stale missing target 383
    // (`delete_one` -> `reset_person_to_delete` -> `reset_marked`) skips 383
    // cleanly without throwing `native error 0x0a`.
    let (mut fx2, _) = crate::visits::lot();
    let person_id = 50u16;
    place_test_person(&mut fx2, person_id);
    let p2 = fx2
        .world
        .objects
        .get_mut(&person_id)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    let mut stale = p2.last.clone();
    stale.target = 383;
    stale.icon = 383;
    stale.uid = 12;
    p2.queue = vec![stale];

    let interp2 = fx2.interpreter();
    let mut world2 = fx2.world.clone();
    assert_eq!(interp2.remove_now(&mut world2, person_id), Ok(()));
    assert!(!world2.objects.contains_key(&person_id));
}

#[test]
fn a_deleted_person_leaves_every_indicator_queue_entry() {
    // 0x444962 queues a person for a star each time its fame is written,
    // and a removal (0x447d8b) erases only the first entry of each queue
    // (0x448663). The original's second entry would then name freed
    // memory; the port drops every entry as the person is deleted, so the
    // next frame's refresh (0x4a8b12 -> 0x4464db) does not fail.
    let (mut fx, _) = crate::visits::lot();
    let person_id = 50u16;
    place_test_person(&mut fx, person_id);
    fx.add(
        SIM_FILE,
        4097,
        tree(vec![assign(
            scope::MY_PERSON,
            81,
            scope::LITERAL,
            3,
            RETURN_TRUE,
        )]),
    );
    for _ in 0..2 {
        assert_eq!(fx.run_with(person_id, 4097, 0, &[], 100), Ok(true));
    }
    fx.world
        .objects
        .get_mut(&person_id)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[77] = 5;
    fx.world.indicator_queues.queue_date(person_id, 5);
    assert_eq!(fx.world.indicator_queues.stars, [person_id, person_id]);
    assert_eq!(fx.world.indicator_queues.dates, [person_id]);

    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    interpreter.create_indicators(&mut world).unwrap();
    assert_eq!(interpreter.remove_now(&mut world, person_id), Ok(()));
    assert!(world.indicator_queues.dates.is_empty());
    assert!(world.indicator_queues.stars.is_empty());
    assert!(world.indicator_queues.dirty, "the flag stays for the frame");
    assert_eq!(interpreter.draw_frame(&mut world, 1), Ok(()));
    assert_eq!(world.indicator_queues, Default::default());

    // Clearing the lot runs no interface removal (0x445f84 leaves the
    // queues as they are); the entries still go with the person.
    let mut world = fx.world.clone();
    world.order = world.objects.keys().copied().collect();
    let cleared = interpreter.clear(&mut world);
    assert!(cleared.is_ok(), "{cleared:?}");
    assert!(world.indicator_queues.dates.is_empty());
    assert!(world.indicator_queues.stars.is_empty());
}
