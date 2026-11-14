//! The load steps that run scripts: the indicators, the lot resize and the
//! lot-wide controllers.

use crate::factory::{WIDGET, widgets};
use crate::support::*;
use sims_formats::lot::Array;
use sims_vm::{INDICATORS, Interpreter, Reconciled, Trap};

/// Adds a type with word 42 (the global word) set as given, whose Main
/// sleeps and which has no Init.
fn add(fx: &mut Fixture, guid: u32, kind: i16, global: i16) {
    let file = fx.add_type(guid, kind, 0, 4097);
    fx.words(file)[42] = global;
    fx.add(
        file,
        4097,
        tree(vec![ins(256, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])]),
    );
}

/// A bare lot, with the catalog holding two controllers (0xA1 of type 7,
/// 0xA2 of type 12) between types that are not controllers.
fn lot() -> Fixture {
    let (mut fx, _) = widgets();
    fx.world.objects.clear();
    fx.world.order.clear();
    add(&mut fx, 0xA0, 7, 0); // no global word
    add(&mut fx, 0xA1, 7, 1);
    add(&mut fx, 0xA3, 4, 1); // not type 7 or 12
    add(&mut fx, 0xA2, 12, -1);
    fx
}

/// A reconciliation that created these IDs and removed nothing.
fn created(ids: &[u16]) -> Reconciled {
    Reconciled {
        created: ids.to_vec(),
        removed: Vec::new(),
    }
}

fn guids(fx: &Fixture) -> Vec<u32> {
    fx.world
        .order
        .iter()
        .map(|id| fx.world.objects[id].guid)
        .collect()
}

#[test]
fn missing_controllers_are_created_in_catalog_order() {
    let mut fx = lot();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(
        interpreter.reconcile_controllers(&mut fx.world),
        Ok(created(&[1, 2]))
    );
    // The registration list has the newest first.
    assert_eq!(guids(&fx), [0xA2, 0xA1]);
    // Each ran its first turn.
    assert!(fx.world.objects.values().all(|o| o.countdown() == Some(19)));
    // Nothing is missing now.
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(
        interpreter.reconcile_controllers(&mut fx.world),
        Ok(created(&[]))
    );
    assert_eq!(fx.world.objects.len(), 2);
}

#[test]
fn an_existing_controller_is_kept() {
    let mut fx = lot();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let existing = interpreter.create(&mut fx.world, 0xA2).unwrap();
    assert_eq!(
        interpreter.reconcile_controllers(&mut fx.world),
        Ok(created(&[2]))
    );
    assert_eq!(guids(&fx), [0xA1, 0xA2]);
    assert_eq!(existing, 1);
}

/// The Reset tree the tests give controller 0xA1: it adds 1 to global 36
/// and stores the object's ID in global 34.
const RESET: u16 = 4300;

fn give_reset(fx: &mut Fixture, guid: u32, body: Vec<sims_formats::behavior::Instruction>) {
    let file = fx.catalog.object(guid).unwrap().resource;
    let table = fx
        .program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap();
    table.resize(
        4,
        sims_formats::execution::ObjectFunction {
            condition: 0,
            action: 0,
        },
    );
    table[3].action = RESET;
    fx.add(file, RESET, tree(body));
}

fn counting_reset() -> Vec<sims_formats::behavior::Instruction> {
    vec![
        assign(scope::GLOBAL, 34, scope::MY_DATA, 11, 1),
        ins(
            2,
            RETURN_TRUE,
            RETURN_TRUE,
            expr(scope::GLOBAL, 36, op::ADD, scope::LITERAL, 1),
        ),
    ]
}

#[test]
fn later_duplicates_are_removed_and_the_first_is_kept() {
    let mut fx = lot();
    give_reset(&mut fx, 0xA1, counting_reset());
    (fx.world.globals[34], fx.world.globals[36]) = (0, 0);
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    // Three of 0xA1 around one of 0xA2. The registration list has the
    // newest first: 4 (A1), 3 (A2), 2 (A1), 1 (A1).
    for guid in [0xA1, 0xA1, 0xA2, 0xA1] {
        interpreter.create(&mut fx.world, guid).unwrap();
    }
    fx.world.removals = vec![2];
    let before = fx.world.clone();
    assert_eq!(
        interpreter.reconcile_controllers(&mut fx.world),
        Ok(Reconciled {
            created: vec![],
            removed: vec![2, 1],
        })
    );
    // The first in the registration list stays, and so does the other
    // controller. Each removed duplicate ran its Reset tree, 1 last.
    assert_eq!(fx.world.order, [4, 3]);
    assert_eq!((fx.world.globals[34], fx.world.globals[36]), (1, 2));
    for id in [3, 4] {
        assert_eq!(fx.world.objects[&id].data, before.objects[&id].data);
        assert_eq!(fx.world.objects[&id].stack.len(), 2, "still asleep");
    }
    // The removal queue holds bare IDs and is not this step's to edit.
    assert_eq!(fx.world.removals, [2]);
    // A second pass finds nothing to do.
    let settled = fx.world.clone();
    assert_eq!(
        interpreter.reconcile_controllers(&mut fx.world),
        Ok(created(&[]))
    );
    assert_eq!(fx.world.encode(), settled.encode());
}

#[test]
fn a_duplicate_is_removed_before_the_next_controller_is_created() {
    let mut fx = lot();
    give_reset(&mut fx, 0xA1, counting_reset());
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    for _ in 0..2 {
        interpreter.create(&mut fx.world, 0xA1).unwrap();
    }
    // 0xA1 comes before 0xA2 in the catalog, so its duplicate (ID 1) goes
    // first and the new 0xA2 takes the freed ID.
    assert_eq!(
        interpreter.reconcile_controllers(&mut fx.world),
        Ok(Reconciled {
            created: vec![1],
            removed: vec![1],
        })
    );
    assert_eq!(guids(&fx), [0xA2, 0xA1]);
    assert_eq!(fx.world.order, [1, 2]);
}

#[test]
fn objects_that_are_not_controllers_are_never_duplicates() {
    let mut fx = lot();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    // 0xA0 has no global word and 0xA3 is not of type 7 or 12.
    for guid in [0xA0, 0xA0, 0xA3, 0xA3, WIDGET, WIDGET] {
        interpreter.create(&mut fx.world, guid).unwrap();
    }
    assert_eq!(
        interpreter.reconcile_controllers(&mut fx.world),
        Ok(created(&[7, 8]))
    );
    assert_eq!(fx.world.objects.len(), 8);
}

#[test]
fn duplicates_are_removed_on_a_lot_with_people_who_do_not_refer_to_them() {
    let mut fx = lot();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    for _ in 0..2 {
        interpreter.create(&mut fx.world, 0xA1).unwrap();
    }
    fx.add_sim(9);
    let sim = fx.world.objects[&9].clone();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let done = interpreter.reconcile_controllers(&mut fx.world).unwrap();
    assert_eq!((done.removed, done.created), (vec![1], vec![1]));
    // The person is as it was.
    let mut lot = fx.world.clone();
    lot.objects.insert(9, sim);
    assert_eq!(lot.encode(), fx.world.encode());
}

#[test]
fn a_duplicate_a_person_refers_to_fails_and_changes_nothing() {
    type Refer = fn(&mut sims_formats::saved_objects::Person);
    let cases: [Refer; 6] = [
        |p| p.active.target = 1,
        |p| p.active.icon = 1,
        |p| p.last.target = 1,
        |p| {
            let mut queued = p.last.clone();
            queued.target = 1;
            p.queue.push(queued);
        },
        |p| {
            let mut queued = p.last.clone();
            queued.icon = 1;
            p.queue.push(queued);
        },
        |p| {
            p.object_uses.push(sims_formats::saved_objects::ObjectUse {
                target: 1,
                stack_depth: 2,
                unknown: 0,
            });
        },
    ];
    for refer in cases {
        let mut fx = lot();
        give_reset(&mut fx, 0xA1, counting_reset());
        let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        for _ in 0..2 {
            interpreter.create(&mut fx.world, 0xA1).unwrap();
        }
        refer(fx.add_sim(9).person.as_mut().unwrap());
        let before = fx.world.clone();
        let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        let fault = interpreter
            .reconcile_controllers(&mut fx.world)
            .unwrap_err();
        assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
        assert!(fault.to_string().contains("person 9 refers"), "{fault}");
        assert_eq!(fx.world.encode(), before.encode(), "nothing ran");
        // A reference to the instance that is kept is no obstacle.
        let person = fx.world.objects.get_mut(&9).unwrap().person.as_mut();
        *person.unwrap() = person_referring_to(2);
        let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        let done = interpreter.reconcile_controllers(&mut fx.world).unwrap();
        assert_eq!(done.removed, [1]);
    }
}

/// A person whose running action targets `id`.
fn person_referring_to(id: i16) -> sims_formats::saved_objects::Person {
    let mut person = person();
    person.active.target = id;
    person
}

#[test]
fn every_catalog_definition_must_be_linked() {
    let mut fx = lot();
    let file = fx.catalog.object(0xA3).unwrap().resource;
    fx.program.libraries.remove(&file);
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter
        .reconcile_controllers(&mut fx.world)
        .unwrap_err();
    assert!(matches!(fault.trap, Trap::Invalid(_)), "{fault}");
}

fn resize(fx: &mut Fixture) -> Result<Vec<u16>, sims_vm::Fault> {
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.resize(&mut fx.world).map(|done| {
        assert_eq!(done.removed, []);
        done.created
    })
}

#[test]
fn shrinking_a_bare_lot_creates_the_controllers() {
    let mut fx = lot();
    fx.world.globals[23] = 52;
    fx.world.globals[27] = 204;
    assert_eq!(resize(&mut fx), Ok(vec![1, 2]));
    assert_eq!(fx.world.size, 52);
    // The room rebuild publishes the indoor area: none on a bare lot.
    assert_eq!(fx.world.globals[27], 0);
    // The same size again changes nothing.
    fx.world.globals[27] = 9;
    assert_eq!(resize(&mut fx), Ok(vec![]));
    assert_eq!((fx.world.size, fx.world.globals[27]), (52, 9));
}

#[test]
fn a_lot_that_keeps_its_size_gets_no_controllers() {
    // Global 23 at zero asks for the full 64 tiles, which the lot has.
    for requested in [0, 64, 3, -1] {
        let mut fx = lot();
        fx.world.globals[23] = requested;
        assert_eq!(resize(&mut fx), Ok(vec![]));
        assert_eq!(fx.world.size, 64);
        assert!(fx.world.objects.is_empty());
        assert_eq!(fx.world.globals[27], 127);
    }
}

#[test]
fn resizes_that_are_not_ported_fail_and_change_nothing() {
    let unsupported = |prepare: fn(&mut Fixture)| {
        let mut fx = lot();
        fx.world.globals[23] = 52;
        prepare(&mut fx);
        let before = fx.world.hash();
        let fault = resize(&mut fx).unwrap_err();
        assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
        assert_eq!(fx.world.hash(), before);
        fault.to_string()
    };
    assert!(unsupported(|fx| fx.world.size = 40).contains("growing"));
    let text = unsupported(|fx| {
        let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
        let id = interpreter.create(&mut fx.world, WIDGET).unwrap();
        fx.world.objects.get_mut(&id).unwrap().position = [24, 40, 1];
    });
    assert!(text.contains("objects on it"), "{text}");
    let text = unsupported(|fx| {
        let array = Array {
            width: 64,
            height: 64,
            stride: 1,
            encoding: 1,
            data: vec![0; 64 * 64],
        };
        fx.world.arrays.insert(1, array);
    });
    assert!(text.contains("arrays"), "{text}");
    // Saved globals the context was never applied to.
    assert!(unsupported(|fx| fx.world.globals.truncate(32)).contains("context"));
}

#[test]
fn the_indicators_are_created_and_hidden() {
    let (mut fx, _) = widgets();
    for guid in INDICATORS {
        add(&mut fx, guid, 4, 0);
    }
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.create_indicators(&mut fx.world), Ok([3, 4]));
    for (id, guid) in [(3, INDICATORS[0]), (4, INDICATORS[1])] {
        let indicator = &fx.world.objects[&id];
        assert_eq!(indicator.guid, guid);
        assert_eq!(indicator.data[8], 0x4000);
        assert_eq!(indicator.countdown(), Some(19));
    }
    // An install without them cannot make them.
    let (mut fx, _) = widgets();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert!(interpreter.create_indicators(&mut fx.world).is_err());
}

/// A saved type-table entry.
fn saved(id: u16, guid: u32, init: u32, main: u32) -> sims_formats::lot::ObjectType {
    sims_formats::lot::ObjectType {
        guid,
        init_signature: init,
        main_signature: main,
        id,
        kind: 4,
        name: String::new(),
        extra: None,
    }
}

/// The widget type's linked Init and Main signatures, with the trees'
/// revisions set to 3 and 5.
fn signed() -> (Fixture, u32, u32) {
    let (mut fx, file) = widgets();
    fx.world.objects.clear();
    fx.world.order.clear();
    let library = fx.program.libraries.get_mut(&file).unwrap();
    library.behaviors.get_mut(&4096).unwrap().revision = 3;
    library.behaviors.get_mut(&4097).unwrap().revision = 5;
    let init = fx.program.signature(file, 4096);
    let main = fx.program.signature(file, 4097);
    assert_ne!(init, main);
    (fx, init, main)
}

#[test]
fn a_type_is_changed_when_either_signature_differs() {
    let (fx, init, main) = signed();
    let interpreter = fx.interpreter();
    let changed = |init, main| interpreter.type_changed(&saved(9, WIDGET, init, main));
    assert_eq!(changed(init, main), Some(false));
    assert_eq!(changed(init + 1, main), Some(true));
    assert_eq!(changed(init, main + 1), Some(true));
    // A GUID with no definition has no answer.
    assert_eq!(
        interpreter.type_changed(&saved(9, 0xdead, init, main)),
        None
    );
}

#[test]
fn reading_the_type_table_replaces_it_and_marks_the_changed_types() {
    let (mut fx, init, main) = signed();
    // The lamp's saved signatures cannot match: its file has no such trees
    // and the save says otherwise.
    let table = sims_formats::lot::ObjectTypes {
        version: 2,
        entries: vec![
            saved(40, WIDGET, init, main),
            saved(41, LAMP_GUID, 77, 78),
            saved(42, 0xdead, 1, 2),
        ],
    };
    // Flags from an earlier load: one the table clears, one it does not
    // list and so leaves alone.
    fx.world.changed_types = [WIDGET, SIM_GUID].into();
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(
        interpreter.read_types(&mut world, &table),
        Ok(vec![LAMP_GUID])
    );
    assert_eq!(world.changed_types, [LAMP_GUID, SIM_GUID].into());
    // The table holds the saved IDs of the types that have a definition.
    assert_eq!(
        world.types,
        std::collections::BTreeMap::from([(40, WIDGET), (41, LAMP_GUID)])
    );
    // A new object's frames use the saved ID.
    assert_eq!(interpreter.create(&mut world, WIDGET), Ok(1));
    assert_eq!(world.objects[&1].stack[0].code_owner_type, 40);
    // With an object on the lot the table cannot be replaced.
    let fault = interpreter.read_types(&mut world, &table).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
}

#[test]
fn two_saved_types_for_one_definition_keep_the_later_id() {
    let (fx, init, main) = signed();
    let table = sims_formats::lot::ObjectTypes {
        version: 2,
        entries: vec![
            saved(40, WIDGET, init + 1, main),
            saved(44, WIDGET, init, main),
        ],
    };
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    // The later record's comparison is the one that stands.
    assert_eq!(interpreter.read_types(&mut world, &table), Ok(vec![WIDGET]));
    assert!(world.changed_types.is_empty());
    assert_eq!(
        world.types,
        std::collections::BTreeMap::from([(44, WIDGET)])
    );
}
