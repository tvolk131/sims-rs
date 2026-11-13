//! The lot resize at the start of a load, on a real install when
//! `SIMS_INSTALL` names one. Game data is never committed, so CI skips
//! this. The mechanics have synthetic tests in `tests/interpreter`.

use sims_content::{Catalog, House, Index, Install, Neighborhood};
use sims_sim::World;
use sims_sim::load::Context;
use sims_vm::{Interpreter, Program, Stream};

/// The Goth house's lot when the resize returns, built the way `sims boot`
/// builds it.
fn resized(root: &std::path::Path) -> (World, Vec<u16>) {
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
    (world, created)
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
/// The digest is of our own state, as built here.
#[test]
fn the_goth_house_resize_creates_the_controllers() {
    const PINNED: &str = "766a182095b3403a78c5e3f9b013f3d17b2276f603d05e688fa4118a9a519741";
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let (world, created) = resized(std::path::Path::new(&root));
    assert_eq!(created, (3..=143).collect::<Vec<u16>>());
    assert_eq!(world.order, (1..=143).rev().collect::<Vec<u16>>());
    assert_eq!((world.size, world.ticks), (52, 88444));
    assert_eq!(world.removals.len(), 2);
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
