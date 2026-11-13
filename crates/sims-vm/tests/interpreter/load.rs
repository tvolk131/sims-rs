//! The load steps that run scripts: the indicators, the lot resize and the
//! lot-wide controllers.

use crate::factory::{WIDGET, widgets};
use crate::support::*;
use sims_formats::lot::Array;
use sims_vm::{INDICATORS, Interpreter, Trap};

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
        Ok(vec![1, 2])
    );
    // The registration list has the newest first.
    assert_eq!(guids(&fx), [0xA2, 0xA1]);
    // Each ran its first turn.
    assert!(fx.world.objects.values().all(|o| o.countdown() == Some(19)));
    // Nothing is missing now.
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.reconcile_controllers(&mut fx.world), Ok(vec![]));
    assert_eq!(fx.world.objects.len(), 2);
}

#[test]
fn an_existing_controller_is_kept() {
    let mut fx = lot();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let existing = interpreter.create(&mut fx.world, 0xA2).unwrap();
    assert_eq!(
        interpreter.reconcile_controllers(&mut fx.world),
        Ok(vec![2])
    );
    assert_eq!(guids(&fx), [0xA1, 0xA2]);
    assert_eq!(existing, 1);
}

#[test]
fn duplicates_need_deletion_which_is_not_ported() {
    let mut fx = lot();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.create(&mut fx.world, 0xA1).unwrap();
    interpreter.create(&mut fx.world, 0xA1).unwrap();
    let fault = interpreter
        .reconcile_controllers(&mut fx.world)
        .unwrap_err();
    assert!(fault.to_string().contains("duplicates"), "{fault}");
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
    interpreter.resize(&mut fx.world)
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
