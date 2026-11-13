//! The lot resize at the start of a load, the clear that follows it and
//! the type table read, on a real install when `SIMS_INSTALL` names one.
//! Game data is never committed, so CI skips this. The mechanics have
//! synthetic tests in `tests/interpreter`.

use sims_content::{Catalog, House, Index, Install, Neighborhood};
use sims_sim::World;
use sims_sim::load::Context;
use sims_vm::{Interpreter, Program, Stream};

/// The Goth house's lot when the resize returns, built the way `sims boot`
/// builds it.
fn resized(root: &std::path::Path) -> (World, Vec<u16>) {
    let mut result = None;
    with_goth(root, |_, _, world, created| {
        result = Some((world.clone(), created.to_vec()))
    });
    result.unwrap()
}

/// Runs `then` with the Goth house, an interpreter over the whole catalog,
/// the lot as the resize leaves it, and the IDs the resize created.
fn with_goth(root: &std::path::Path, then: impl FnOnce(&House, &Interpreter, &mut World, &[u16])) {
    let install = Install::detect(root).unwrap();
    let neighborhood = Neighborhood::open(&install.file("UserData").unwrap()).unwrap();
    let mut index = Index::build(&install).unwrap();
    index
        .add_characters(&neighborhood.characters().unwrap().unwrap())
        .unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let house = House::load(&neighborhood.houses().unwrap()[&5], &catalog).unwrap();
    let context = Context::COMPLETE_COLLECTION;
    let mut program =
        Program::link(&index, &catalog, catalog.objects.iter().map(|o| o.guid)).unwrap();
    program.zoning = neighborhood.zoning(context.expansions).unwrap();
    let interpreter = Interpreter::new(&program, &catalog, Stream::Global);
    let mut world = World::empty();
    world.type_attributes = program.type_attributes(&catalog).unwrap();
    world.read_type_attributes(neighborhood.type_attributes.as_ref().unwrap());
    assert_eq!(interpreter.create_indicators(&mut world), Ok([1, 2]));
    world.read_simulator(&house.simulator);
    world.apply_context(context).unwrap();
    let created = interpreter.resize(&mut world).unwrap();
    then(&house, &interpreter, &mut world, &created);
}

/// The original shrinks the lot it is holding from 64 tiles to the Goth
/// house's 52 before it reads the house's objects, and that creates all
/// 141 lot-wide controllers on a lot that holds only the two indicators.
///
/// A capture of the original at the moment the resize returns, taken by
/// the old prototype's harness, was compared with this state outside the
/// repository; the capture is private. What was equal: all 141
/// controllers (GUID, the 72 data words, temps, attributes, last result,
/// every frame's tree, node, stack object, parameters and locals, the
/// registry word and the position), the 64 globals, both game random
/// streams, the tick, the lot size, the registration order, the objects
/// waiting for removal, the scheduler cursor, the funds, and the instance
/// lists.
///
/// What was not equal belongs to the game that was running, not to the
/// house or the install: the two indicators' creation stamp and how far
/// into their sleep they were, the C runtime stream, and six type
/// attribute words. With those copied from the capture's state at the
/// start of the resize, the two indicators matched as well. The capture
/// has nothing to compare `World`'s other fields with.
///
/// The digest is of our own state, as built here. Snapshot format 4 added
/// two fields at the end of the encoding (the indicators' IDs and the
/// changed types). Without those three bytes the encoding still hashes to
/// the digest pinned under format 3, the one that comparison was made on.
#[test]
fn the_goth_house_resize_creates_the_controllers() {
    const PINNED: &str = "06925c3997de50179b8bfe0dd5cdb751b8e0f449ce0280ffcc6269c4e4e7e72e";
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let (world, created) = resized(std::path::Path::new(&root));
    assert_eq!(created, (3..=143).collect::<Vec<u16>>());
    assert_eq!(world.order, (1..=143).rev().collect::<Vec<u16>>());
    assert_eq!((world.size, world.ticks), (52, 88444));
    assert_eq!(world.removals.len(), 2);
    assert_eq!(world.indicators, [1, 2]);
    // Every controller is asleep in a tree, counted down by the scheduler
    // or by itself, and marked ready.
    for id in &created {
        let object = &world.objects[id];
        assert!(object.stack.len() >= 2, "object {id}");
        assert_eq!(object.flags() & 1, 1, "object {id}");
    }
    assert_eq!(world.hash_hex(), PINNED);
    // The run is deterministic.
    let (again, _) = resized(std::path::Path::new(&root));
    assert_eq!(again.encode(), world.encode());
}

/// Load step 5: the original then removes every object on the lot, the
/// two indicators first and then the head of the registration list until
/// it is empty. Eighteen of the 141 controllers have a Reset tree, which
/// runs as each is deleted.
///
/// A capture of the original at the moment the clear returns, taken by
/// the old prototype's harness, was compared with this state outside the
/// repository; the capture is private. What was equal: no objects and an
/// empty registration list, the tick, the lot size, the 64 globals, both
/// game random streams, the two IDs still queued for removal, the
/// scheduler cursor, the funds, the indicator references (both 0), the one
/// instance list (kept, and empty), and the type attribute records' GUIDs,
/// order and lengths.
///
/// What was not equal belongs to the game that was running: the C runtime
/// stream, and the same six type attribute words as when the resize
/// returns. With those copied from the capture into the lot before the
/// resize, they were equal too, so no Reset tree changes either. The
/// capture has nothing to compare `World`'s other fields with, the
/// leftover type table among them.
///
/// The digest is of our own state, as built here.
#[test]
fn the_goth_house_clear_removes_every_object() {
    const PINNED: &str = "ee03713bb263459d92b0359743f94c5c6c582951ae3eda24a2b066c0729ef906";
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    with_goth(std::path::Path::new(&root), |_, interpreter, world, _| {
        world.clear_family().unwrap();
        let before = world.clone();
        let removed = interpreter.clear(world).unwrap();
        let mut expected = vec![1, 2];
        expected.extend((3..=143).rev());
        assert_eq!(removed, expected);
        assert!(world.objects.is_empty() && world.order.is_empty());
        assert_eq!(world.indicators, [0, 0]);
        assert_eq!(world.removals, before.removals);
        assert_eq!(world.globals, before.globals);
        assert_eq!(world.type_attributes, before.type_attributes);
        assert_eq!(
            (world.global_rng, world.lot_rng, world.crt_rng),
            (before.global_rng, before.lot_rng, before.crt_rng)
        );
        assert_eq!((world.size, world.ticks), (52, 88444));
        assert!(world.instances.values().all(Vec::is_empty));
        assert_eq!(world.hash_hex(), PINNED);
        // The original clears the lot again before it reads the saved
        // objects. On an empty lot that does nothing.
        assert_eq!(interpreter.clear(world), Ok(vec![]));
        assert_eq!(world.hash_hex(), PINNED);
    });
}

/// The type table read marks 77 of the Goth house's 113 saved types as
/// changed since the save.
///
/// Only three of those were compared with the original. A capture taken
/// after the original had read this table shows, for the three Goths'
/// own types (saved IDs 111 to 113), an Init signature of 20, the changed
/// flag set, and a Main signature never worked out. The first two are
/// asserted here; the third is why [`Interpreter::type_changed`] does not
/// need Main when Init differs. Nothing was compared for the other 110
/// types, the rebuilt table, or the lot arrays read before it, and the
/// load does not yet do the floor and wall renumbering that comes between
/// the arrays and this table.
#[test]
fn the_goth_house_type_table_marks_the_changed_types() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    with_goth(
        std::path::Path::new(&root),
        |house, interpreter, world, _| {
            world.clear_family().unwrap();
            interpreter.clear(world).unwrap();
            world.read_arrays(&house.arrays).unwrap();
            assert_eq!(world.arrays.len(), 14);
            let before = world.clone();
            let changed = interpreter.read_types(world, &house.types).unwrap();
            assert_eq!((changed.len(), house.types.entries.len()), (77, 113));
            assert_eq!(world.changed_types.len(), 77);
            assert_eq!(world.types.len(), 113);
            for entry in &house.types.entries {
                assert_eq!(world.types[&entry.id], entry.guid);
                if (111..=113).contains(&entry.id) {
                    let definition = interpreter.catalog.object(entry.guid).unwrap();
                    let init = interpreter
                        .program
                        .callback(definition, sims_vm::Callback::Init);
                    assert_eq!(interpreter.program.signature(definition.resource, init), 20);
                    assert!(world.changed_types.contains(&entry.guid));
                }
            }
            // Nothing else on the lot moves.
            world.types = before.types.clone();
            world.changed_types.clear();
            assert_eq!(world.encode(), before.encode());
        },
    );
}
