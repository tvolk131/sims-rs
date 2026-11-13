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
