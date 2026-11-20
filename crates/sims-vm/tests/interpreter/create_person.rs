//! Create New Object Instance (0x44e0c8) making a person off the lot in
//! mode 6, flag 0x4 (the type of a neighbor record, and a visitor's "init
//! visitor" tree), and Remove Object Instance taking a person off the lot
//! for another object (0x58da12 worked out on a copy).

use crate::factory::WIDGET;
use crate::find_location::{create as create_at_local, placeable};
use crate::person_factory::equip;
use crate::support::*;
use sims_vm::Trap;

const CREATE: u16 = 42;
const REMOVE: u16 = 18;
const FROM_RECORD: u8 = 0x4;
/// The widget's tree named "init visitor".
const INIT_VISITOR: u16 = 4099;
/// The global the "init visitor" tree writes its stack object to.
const SEEN: usize = 36;

/// Find Location For's lot, with the widget a person type
/// [`equip`]ped with records whose words are all 12. The lot is not
/// residential, so a new person is a visitor (person word 32 is 1). The
/// widget's file names tree 4099 "init visitor": it writes its stack
/// object to global 36. Gives the widget's file and the ID by which a
/// script names its record (the record's slot plus one).
fn visitors() -> (Fixture, usize, i16) {
    let (mut fx, file) = placeable();
    fx.words(file)[9] = 2;
    equip(&mut fx, WIDGET, "widget", 12);
    fx.lamp_mut().position = [40, 40, 1];
    fx.add(
        file,
        INIT_VISITOR,
        tree(vec![assign(
            scope::GLOBAL,
            SEEN as i16,
            scope::STACK_OBJECT,
            0,
            RETURN_TRUE,
        )]),
    );
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .names
        .insert(INIT_VISITOR, "init visitor".to_owned());
    fx.world.globals[SEEN] = 0;
    let record = fx.world.neighbors.slots.len() as i16;
    (fx, file, record)
}

/// Runs the lamp's tree 4096: Create New Object Instance of type `guid`
/// in mode `mode` with `flags`, with the stack object `stack_object`.
fn create(
    fx: &mut Fixture,
    guid: u32,
    mode: u8,
    flags: u8,
    stack_object: i16,
) -> Result<bool, sims_vm::Fault> {
    let [g0, g1, g2, g3] = guid.to_le_bytes();
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

fn ids(fx: &Fixture) -> Vec<u16> {
    fx.world.objects.keys().copied().collect()
}

fn new_object(fx: &Fixture, before: &[u16]) -> u16 {
    let made: Vec<u16> = ids(fx)
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect();
    assert_eq!(made.len(), 1, "{made:?}");
    made[0]
}

#[test]
fn flag_0x4_makes_the_type_of_the_record_the_stack_object_names() {
    // The GUID operand (0, which has no definition) is not read: the type
    // is the record's. Mode 6 leaves the person off the lot, with its
    // record's interests.
    let (mut fx, _, record) = visitors();
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 0, 6, FROM_RECORD, record), Ok(true));
    let id = new_object(&fx, &before);
    let object = &fx.world.objects[&id];
    assert_eq!(object.guid, WIDGET);
    assert_eq!(object.position, [-16, -16, 1]);
    let person = object.person.as_ref().unwrap();
    assert_eq!(person.data[46..54], [12; 8]);
    assert!(fx.world.people.contains(&id));
    // A stack object that names no record takes the false branch before
    // anything is made: past the slots, 0, or negative.
    for stack_object in [record + 1, 0, -1] {
        let (mut fx, _, _) = visitors();
        let before = ids(&fx);
        assert_eq!(
            create(&mut fx, 0, 6, FROM_RECORD, stack_object),
            Ok(false),
            "{stack_object}"
        );
        assert_eq!(ids(&fx), before);
    }
    // Without the flag the GUID operand is read: 0 has no definition.
    let (mut fx, _, record) = visitors();
    assert_eq!(create(&mut fx, 0, 6, 0, record), Ok(false));
}

#[test]
fn flag_0x4_runs_a_new_visitors_init_visitor_tree_at_once() {
    // The tree runs to its end on the new person's thread, with the person
    // as its stack object, before the new person becomes the creator's
    // stack object.
    let (mut fx, _, record) = visitors();
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 0, 6, FROM_RECORD, record), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].person.as_ref().unwrap().data[32], 1);
    assert_eq!(fx.world.globals[SEEN], id as i16);
    let stack = &fx.world.objects[&id].stack;
    assert!(stack.iter().all(|frame| frame.tree != INIT_VISITOR));
    // Mode 8 runs it too, once the person is placed.
    let (mut fx, _, record) = visitors();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, LAMP as i16, 1),
            ins(CREATE, RETURN_TRUE, RETURN_FALSE, [0, 0, 0, 0, 8, 4, 0, 0]),
        ]),
    );
    let before = ids(&fx);
    assert_eq!(fx.run_with(LAMP, 4096, record, &[], 1000), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].position, [40, 40, 1]);
    assert_eq!(fx.world.globals[SEEN], id as i16);
    // Not without the flag.
    let (mut fx, _, _) = visitors();
    assert_eq!(create(&mut fx, WIDGET, 6, 0, LAMP as i16), Ok(true));
    assert_eq!(fx.world.globals[SEEN], 0);
    // On a residential lot too: the factory's signal 0xe2 works out
    // again who is a visitor (0x42a2b4), and a person whose type is not
    // one of the selected family's members becomes one.
    let (mut fx, _, record) = visitors();
    let lot = fx.world.globals[10];
    fx.program.zoning.insert(lot, 1);
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 0, 6, FROM_RECORD, record), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].person.as_ref().unwrap().data[32], 1);
    assert_eq!(fx.world.globals[SEEN], id as i16);
    // Not for a person that is not a visitor: one of the family's
    // members, on a residential lot.
    let (mut fx, _, record) = visitors();
    let lot = fx.world.globals[10];
    fx.program.zoning.insert(lot, 1);
    fx.world.families[0].members.push(WIDGET);
    let before = ids(&fx);
    assert_eq!(create(&mut fx, 0, 6, FROM_RECORD, record), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].person.as_ref().unwrap().data[32], 0);
    assert_eq!(fx.world.globals[SEEN], 0);
    // A file with no tree of that name runs nothing.
    let (mut fx, file, record) = visitors();
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .names
        .remove(&INIT_VISITOR);
    assert_eq!(create(&mut fx, 0, 6, FROM_RECORD, record), Ok(true));
    assert_eq!(fx.world.globals[SEEN], 0);
}

#[test]
fn a_person_made_with_person_word_68_74_or_84_set_is_made_as_any_other() {
    // The person test (0x44e6b6 to 0x44e727) sends such a person through
    // 0x45810c, which touches only the screen; the port leaves it out.
    for word in [68, 74, 84] {
        let (mut fx, _, record) = visitors();
        let before = ids(&fx);
        let slot = fx.world.neighbors.slots.last_mut().unwrap();
        let slot = slot.as_mut().unwrap();
        slot.revision = 10;
        slot.data[word] = 3;
        assert_eq!(
            create(&mut fx, 0, 6, FROM_RECORD, record),
            Ok(true),
            "word {word}"
        );
        let id = new_object(&fx, &before);
        assert_eq!(fx.world.objects[&id].person.as_ref().unwrap().data[word], 3);
    }
}

#[test]
fn what_mode_6_making_a_person_does_not_port_fails_loudly() {
    // An "init visitor" tree that would wait cannot (native error 0x44f in
    // the original's synchronous run).
    let (mut fx, file, record) = visitors();
    fx.add(
        file,
        INIT_VISITOR,
        tree(vec![ins(
            0,
            RETURN_TRUE,
            RETURN_FALSE,
            [10, 0, 0, 0, 0, 0, 0, 0],
        )]),
    );
    let result = create(&mut fx, 0, 6, FROM_RECORD, record);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("init visitor")));
}

/// A visitor made by mode 8 on the lamp's tile, the lamp's tree 4200,
/// which removes its stack object, and the widget's file.
fn placed_visitor() -> (Fixture, u16, usize) {
    let (mut fx, file, _) = visitors();
    let before = ids(&fx);
    assert_eq!(create_at_local(&mut fx, 0, 0, SIM as i16), Ok(true));
    let id = new_object(&fx, &before);
    assert_eq!(fx.world.objects[&id].position, [40, 40, 1]);
    fx.add(
        LAMP_FILE,
        4200,
        tree(vec![ins(
            REMOVE,
            RETURN_TRUE,
            RETURN_FALSE,
            [1, 0, 0, 0, 0, 0, 0, 0],
        )]),
    );
    (fx, id, file)
}

#[test]
fn another_object_takes_a_person_off_and_nothing_reads_its_animator_after() {
    // The take-off's animator refresh (0x58da12), made only if the house
    // view drew the person, is worked out on a copy: it writes nothing but
    // the animator, so the person is queued.
    let (mut fx, id, file) = placed_visitor();
    assert_eq!(fx.run_with(LAMP, 4200, id as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.removals, [id]);
    assert_eq!(fx.world.objects[&id].position, [-16, -16, 1]);
    // Its animator may differ from the original's until the deletion, so
    // the person may not tick, nor run a primitive that reads an
    // animator. One that reads none (an expression) runs.
    let queued = fx.world.clone();
    let tuning = crate::tick::tuning();
    let ticks = fx.world.ticks as i32;
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter
        .tick_person(&mut fx.world, id, ticks, &tuning)
        .unwrap_err();
    assert!(
        matches!(fault.trap, Trap::Unsupported(ref m) if m.contains("queued for removal")),
        "{fault}"
    );
    fx.world = queued.clone();
    fx.add(
        file,
        4201,
        tree(vec![ins(44, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    let result = fx.run_with(id, 4201, 0, &[], 1000);
    let t = trap(result);
    assert!(
        matches!(t, Trap::Unsupported(ref m) if m.contains("queued for removal")),
        "{t}"
    );
    fx.world = queued;
    fx.add(
        file,
        4202,
        tree(vec![assign(
            scope::TEMPS,
            0,
            scope::LITERAL,
            7,
            RETURN_TRUE,
        )]),
    );
    assert_eq!(fx.run_with(id, 4202, 0, &[], 1000), Ok(true));
}

#[test]
fn another_object_taking_off_a_person_whose_hand_holds_something_fails_loudly() {
    let (mut fx, id, _) = placed_visitor();
    // The Sim, holding nothing itself, is in the person's hand.
    for held in &mut fx.sim_mut().slots {
        held.object = 0;
    }
    fx.world.objects.get_mut(&id).unwrap().slots[1].object = SIM as i16;
    let result = fx.run_with(LAMP, 4200, id as i16, &[], 1000);
    let t = trap(result);
    assert!(
        matches!(t, Trap::Unsupported(ref m) if m.contains("hand")),
        "{t}"
    );
}

#[test]
fn a_person_another_took_off_ticks_with_its_refresh_pending() {
    // The lamp (ID 1) removes the visitor (ID 3) in the scheduler's pass,
    // then sleeps. The visitor's tick comes after in the same pass: its
    // animator's refresh at the take-off, worked out then, waits for its
    // animator's tick, where the refresh then the tick must reach what the
    // tick alone does. It is deleted at the end of the update.
    let (mut fx, id, file) = placed_visitor();
    assert!(id > LAMP);
    fx.world.globals = vec![0; 64];
    fx.add(
        LAMP_FILE,
        4203,
        tree(vec![
            ins(REMOVE, 1, 1, [1, 0, 0, 0, 0, 0, 0, 0]),
            ins(0, RETURN_TRUE, RETURN_FALSE, [0; 8]),
        ]),
    );
    fx.thread(LAMP, 4203, &[5, 0, 0, 0]);
    fx.lamp_mut().stack[0].stack_object = id as i16;
    for object in fx.world.objects.values_mut() {
        // Ready, with no countdown (-1), or not ready.
        object.registry_word = Some(if object.id == LAMP || object.id == id {
            0x0001_ffff
        } else {
            0xffff
        });
    }
    let pass = |fx: &mut Fixture| {
        let tuning = crate::tick::tuning();
        let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        let mut update = sims_vm::Update::default();
        let result = interpreter.scheduler_pass(&mut fx.world, &tuning, &mut update);
        (result, update)
    };
    let ready = fx.world.clone();
    let (result, update) = pass(&mut fx);
    assert_eq!(result, Ok(true));
    assert_eq!(update.ticked, [LAMP, id]);
    assert_eq!(fx.world.removals, [id]);
    // Its thread may not run a primitive that reads an animator before
    // its animator's tick (Animate Sim, opcode 44, here).
    fx.world = ready;
    fx.add(
        file,
        4204,
        tree(vec![ins(44, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    fx.thread(id, 4204, &[0; 4]);
    let (result, _) = pass(&mut fx);
    let error = result.unwrap_err().to_string();
    assert!(error.contains("left the lot and before its"), "{error}");
}

#[test]
fn a_person_removing_itself_and_going_on_settles_its_refresh_at_its_animators_tick() {
    // The visitor (ID 3) removes itself in its own tick with flag bit 0,
    // so its tree goes on. Its animator's refresh at the take-off is
    // pending for its animator's tick later in the same tick, as for a
    // person that takes itself off: until then its thread may run only
    // what reads no animator.
    let (mut fx, id, file) = placed_visitor();
    fx.world.globals = vec![0; 64];
    for object in fx.world.objects.values_mut() {
        // Only the visitor is ready, with no countdown (-1).
        object.registry_word = Some(if object.id == id { 0x0001_ffff } else { 0xffff });
    }
    let pass = |fx: &mut Fixture| {
        let tuning = crate::tick::tuning();
        let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        let mut update = sims_vm::Update::default();
        let result = interpreter.scheduler_pass(&mut fx.world, &tuning, &mut update);
        (result, update)
    };
    let ready = fx.world.clone();
    // Then it sleeps, which reads no animator: its animator's tick
    // settles the refresh.
    fx.add(
        file,
        4205,
        tree(vec![
            ins(REMOVE, 1, 1, [0, 0, 1, 0, 0, 0, 0, 0]),
            ins(0, RETURN_TRUE, RETURN_FALSE, [0; 8]),
        ]),
    );
    fx.thread(id, 4205, &[5, 0, 0, 0]);
    let (result, update) = pass(&mut fx);
    assert_eq!(result, Ok(true));
    assert_eq!(update.ticked, [id]);
    assert_eq!(fx.world.removals, [id]);
    assert_eq!(fx.world.objects[&id].position, [-16, -16, 1]);
    // Then Animate Sim (opcode 44), which reads an animator: it fails on the
    // pending refresh.
    fx.world = ready;
    fx.add(
        file,
        4206,
        tree(vec![
            ins(REMOVE, 1, 1, [0, 0, 1, 0, 0, 0, 0, 0]),
            ins(44, RETURN_TRUE, RETURN_FALSE, [0; 8]),
        ]),
    );
    fx.thread(id, 4206, &[0; 4]);
    let (result, _) = pass(&mut fx);
    let error = result.unwrap_err().to_string();
    assert!(error.contains("left the lot and before its"), "{error}");
}

/// A record of the widget's type whose person data is all `value`.
fn widget_record(id: i16, value: i16) -> sims_sim::neighbors::Neighbor {
    sims_sim::neighbors::Neighbor {
        id,
        guid: WIDGET,
        revision: 3,
        data: vec![value; 256],
        relationships: Vec::new(),
        friends: 0,
        stale: false,
    }
}

#[test]
fn a_person_removed_is_saved_into_the_record_its_word_31_names() {
    // 0x451bee to 0x451c03, 0x425a69: the record word 31 names (by place
    // plus one), not the first of the type, gets revision 9 (0x423e04) and
    // the words the table at 0x423827 lists for revision 9.
    let (mut fx, id, _) = placed_visitor();
    let person = fx
        .world
        .objects
        .get_mut(&id)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    person.data = (0..256).map(|n| 2000 + n).collect();
    person.data[31] = 2;
    fx.world.neighbors.slots = vec![Some(widget_record(40, -1)), Some(widget_record(41, -2))];
    let before = fx.world.neighbors.clone();
    assert_eq!(fx.run_with(LAMP, 4200, id as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.removals, [id]);
    assert_eq!(fx.world.neighbors.get(1), before.get(1));
    let record = fx.world.neighbors.get(2).unwrap();
    assert_eq!(record.revision, 9);
    let saved = [
        2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 26, 46, 47, 48, 49, 50, 51,
        52, 53, 54, 55, 56, 57, 58, 60, 61, 63, 65, 67, 68, 69, 74, 75, 76, 80, 81, 82,
    ];
    for word in 0..256 {
        let expected = if saved.contains(&word) {
            2000 + word as i16
        } else {
            -2
        };
        assert_eq!(record.data[word], expected, "word {word}");
    }
    // The record's own ID is kept.
    assert_eq!(record.id, 41);
}

#[test]
fn a_person_whose_word_31_names_no_record_is_removed_without_a_save() {
    let (mut fx, id, _) = placed_visitor();
    let person = fx
        .world
        .objects
        .get_mut(&id)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    person.data[31] = 3;
    fx.world.neighbors.slots = vec![Some(widget_record(40, -1)), Some(widget_record(41, -2))];
    let before = fx.world.neighbors.clone();
    assert_eq!(fx.run_with(LAMP, 4200, id as i16, &[], 1000), Ok(true));
    assert_eq!(fx.world.removals, [id]);
    assert_eq!(fx.world.neighbors, before);
}

#[test]
fn go_to_routing_slot_reads_no_animator_before_its_route() {
    // The visitor removes itself in its own tick and goes on, its
    // animator's refresh pending. Go To Routing Slot (opcode 45) tests its
    // stack object's place (0x40743b, 0x432595) and names its slot
    // (native error 0x25 at 0x45ace4) before its route reads the
    // animator.
    let (mut fx, id, file) = placed_visitor();
    fx.world.globals = vec![0; 64];
    for object in fx.world.objects.values_mut() {
        object.registry_word = Some(if object.id == id { 0x0001_ffff } else { 0xffff });
    }
    let ready = fx.world.clone();
    // Each case: the stack object, the slot's source, and what the pass
    // gives (Ok, or a part of its error).
    for (n, (stack_object, source, expected)) in [
        // Itself, off the lot now: false, and the tree goes on.
        (id as i16, 1u8, None),
        // The lamp, on the lot, with source 3, which names no slot.
        (LAMP as i16, 3, Some("0x25")),
    ]
    .into_iter()
    .enumerate()
    {
        fx.world = ready.clone();
        let tree_id = 4207 + n as u16;
        fx.add(
            file,
            tree_id,
            tree(vec![
                ins(REMOVE, 1, 1, [0, 0, 1, 0, 0, 0, 0, 0]),
                ins(45, RETURN_TRUE, 2, [0, 0, source, 0, 0, 0, 0, 0]),
                ins(0, RETURN_TRUE, RETURN_FALSE, [0; 8]),
            ]),
        );
        // Then it sleeps (parameter 0), which reads no animator.
        fx.thread(id, tree_id, &[5, 0, 0, 0]);
        fx.world.objects.get_mut(&id).unwrap().stack[0].stack_object = stack_object;
        let tuning = crate::tick::tuning();
        let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        let mut update = sims_vm::Update::default();
        let result = interpreter.scheduler_pass(&mut fx.world, &tuning, &mut update);
        match expected {
            None => assert_eq!(result, Ok(true), "case {n}"),
            Some(part) => {
                let error = result.unwrap_err().to_string();
                assert!(error.contains(part), "case {n}: {error}");
                assert!(!error.contains("left the lot"), "case {n}: {error}");
            }
        }
    }
}

/// The visitor removes itself in its own tick and goes on, its
/// animator's refresh pending, to Idle For Input with an action queued on
/// the lamp that interrupts. The action has no tree, so it is cancelled
/// (0x45dcc8).
fn cancel_with_the_refresh_pending(
    fx: &mut Fixture,
    id: u16,
    file: usize,
) -> Result<bool, sims_vm::TickError> {
    fx.world.globals = vec![0; 64];
    for object in fx.world.objects.values_mut() {
        object.registry_word = Some(if object.id == id { 0x0001_ffff } else { 0xffff });
    }
    fx.add(
        file,
        4209,
        tree(vec![
            ins(REMOVE, 1, 1, [0, 0, 1, 0, 0, 0, 0, 0]),
            ins(17, RETURN_TRUE, RETURN_FALSE, [0, 0, 1, 0, 0, 0, 0, 0]),
        ]),
    );
    fx.thread(id, 4209, &[3, 0, 0, 0]);
    let person = fx
        .world
        .objects
        .get_mut(&id)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    person.data[71] = 0;
    person.data[33] = 7;
    let mut action = person.active.clone();
    (
        action.uid,
        action.target,
        action.action,
        action.priority,
        action.flags,
    ) = (0, LAMP as i16, 0, 50, 0);
    person.queue = vec![action];
    let tuning = crate::tick::tuning();
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let mut update = sims_vm::Update::default();
    interpreter.scheduler_pass(&mut fx.world, &tuning, &mut update)
}

#[test]
fn idle_for_input_cancels_a_queued_action_with_the_refresh_pending() {
    // Nothing of the cancellation reads an animator, and the pass goes
    // on.
    let (mut fx, id, file) = placed_visitor();
    assert_eq!(cancel_with_the_refresh_pending(&mut fx, id, file), Ok(true));
    assert_eq!(fx.world.removals, [id]);
    assert!(
        fx.world.objects[&id]
            .person
            .as_ref()
            .unwrap()
            .queue
            .is_empty()
    );
}

#[test]
fn a_cancellation_callback_may_not_remove_the_person_with_the_refresh_pending() {
    // The lamp's cancellation callback (function 4), run on the lamp with
    // the visitor as its stack object, removes the visitor: a removal on
    // another thread while the refresh is pending is refused, as any
    // other primitive there is.
    let (mut fx, id, file) = placed_visitor();
    let library = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
    let table = library.callbacks.get_mut(&128).unwrap();
    table.resize(
        5,
        sims_formats::execution::ObjectFunction {
            condition: 0,
            action: 0,
        },
    );
    table[4].action = 4200;
    let error = cancel_with_the_refresh_pending(&mut fx, id, file).unwrap_err();
    let sims_vm::TickError::Fault(fault) = error else {
        panic!("{error}");
    };
    assert!(
        matches!(fault.trap, Trap::Unsupported(ref m)
            if m.contains("opcode 18") && m.contains(&format!("thread of object {LAMP}"))),
        "{fault}"
    );
}
