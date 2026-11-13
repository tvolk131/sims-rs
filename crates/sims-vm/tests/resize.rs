//! The lot resize at the start of a load, the clear that follows it, the
//! lot arrays, the type table and the saved objects, on a real install
//! when `SIMS_INSTALL` names one.
//! Game data is never committed, so CI skips this. The mechanics have
//! synthetic tests in `tests/interpreter`.

use sims_content::{Catalog, House, Index, Install, Neighborhood, People};
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
    with_goth_people(root, |house, interpreter, world, created, _| {
        then(house, interpreter, world, created)
    });
}

/// [`with_goth`], and what constructing the house's people needs.
fn with_goth_people(
    root: &std::path::Path,
    then: impl FnOnce(&House, &Interpreter, &mut World, &[u16], &People),
) {
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
    let guids = house.objects.instances.values().map(|o| o.guid);
    let people = People::load(&install, &index, &catalog, &neighborhood, guids).unwrap();
    then(&house, &interpreter, &mut world, &created, &people);
}

/// The SHA-256 of `bytes`, as lowercase hex.
fn digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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
/// The digest is of our own state, as built here. Snapshot format 5 added
/// two fields at the end of the encoding (the multi-tile links and the
/// skeleton files read), both empty here. Without those two bytes the
/// encoding still hashes to the digest pinned under format 4, which in
/// turn was the format 3 state that comparison was made on plus three
/// bytes.
#[test]
fn the_goth_house_resize_creates_the_controllers() {
    const PINNED: &str = "9e5cab3b2caf4e1915faf58b9a3ef5fe192495b44cfb09420b9396368947bd15";
    const FORMAT_4: &str = "06925c3997de50179b8bfe0dd5cdb751b8e0f449ce0280ffcc6269c4e4e7e72e";
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
    let encoded = world.encode();
    let (old, new) = encoded.split_at(encoded.len() - 2);
    assert_eq!((digest(old).as_str(), new), (FORMAT_4, &[0, 0][..]));
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
/// The digest is of our own state, as built here. It is the state pinned
/// under snapshot format 4, the one that comparison was made on, plus the
/// two bytes of format 5's new fields, which are empty here.
#[test]
fn the_goth_house_clear_removes_every_object() {
    const PINNED: &str = "b97f264759c4f6be2b3a356bb7a55bbdb662f2b0fdd80f1e87f613522362cae0";
    const FORMAT_4: &str = "ee03713bb263459d92b0359743f94c5c6c582951ae3eda24a2b066c0729ef906";
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
        let encoded = world.encode();
        let (old, new) = encoded.split_at(encoded.len() - 2);
        assert_eq!((digest(old).as_str(), new), (FORMAT_4, &[0, 0][..]));
        // The original clears the lot again before it reads the saved
        // objects. On an empty lot that does nothing.
        assert_eq!(interpreter.clear(world), Ok(vec![]));
        assert_eq!(world.hash_hex(), PINNED);
    });
}

/// Load steps 6a and 6b: the lot arrays are read, and the saved floor and
/// wall IDs renumbered. The Goth house saves its floors as bytes and its
/// walls as 8 bytes a cell, which are widened to the lot's 2 and 14.
///
/// A capture of the original taken when it enters the function that reads
/// the type table and the objects (0x43fc02), so after both steps, was
/// compared with this state outside the repository; the capture is
/// private. It holds the cells inside the lot's 52 tiles of the terrain
/// heights, both levels' floors, walls, object grid and flags, the pool
/// and the water. All were equal to arrays 0, 1, 2, 3, 8, 101, 102, 103,
/// 108, 9 and 10 here. So the renumbering changes nothing in this house,
/// as it does here: both maps are empty and no cell holds an ID it covers.
///
/// Not compared, because the capture does not hold them: arrays 4, 6 and
/// 7 (ground and grass), and every cell outside the 52 tiles. The digest
/// is of our own arrays.
#[test]
fn the_goth_house_arrays_are_read_and_nothing_is_renumbered() {
    const PINNED: &str = "023361585d1b6644d33928f33039ef73c33e347e3c01c81cc3a8b0004ddcb1f3";
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    with_goth(
        std::path::Path::new(&root),
        |house, interpreter, world, _| {
            world.clear_family().unwrap();
            interpreter.clear(world).unwrap();
            assert_eq!(house.version(), Some(62));
            world.read_arrays(&house.arrays, 62).unwrap();
            assert_eq!(world.arrays.len(), 14);
            for (id, stride) in [(1, 2), (101, 2), (2, 14), (102, 14)] {
                let (saved, read) = (&house.arrays[&id], &world.arrays[&id]);
                assert_eq!(read.stride, stride);
                // A cell inside the lot's size holds the saved bytes,
                // widened; one outside holds nothing.
                for (x, y) in [(0, 0), (26, 28), (51, 51), (52, 0), (63, 63)] {
                    let (saved, read) = (saved.cell(x, y).unwrap(), read.cell(x, y).unwrap());
                    // A floor byte becomes a word. A wall keeps its first
                    // two bytes and its other six become words.
                    let keep = if stride == 14 { 2 } else { 0 };
                    let mut wide = saved[..keep].to_vec();
                    wide.extend(saved[keep..].iter().flat_map(|&byte| [byte, 0]));
                    if x < 52 && y < 52 {
                        assert_eq!(read, wide, "array {id}");
                    } else {
                        assert!(read.iter().all(|&byte| byte == 0), "array {id}");
                    }
                }
            }
            // Both maps are in the file, and empty.
            assert_eq!(house.floor_map.as_deref(), Some(&[][..]));
            assert_eq!(house.wall_map.as_deref(), Some(&[][..]));
            let before = world.arrays.clone();
            let changed = world
                .renumber_surfaces(house.floor_map.as_deref(), house.wall_map.as_deref())
                .unwrap();
            assert_eq!(changed, 0);
            assert_eq!(world.arrays, before);
            // Each array's ID and cell size as two bytes each, then its
            // cells.
            let mut arrays = Vec::new();
            for (id, array) in &world.arrays {
                arrays.extend(id.to_le_bytes());
                arrays.extend((array.stride as u16).to_le_bytes());
                arrays.extend(&array.data);
            }
            assert_eq!(digest(&arrays), PINNED);
        },
    );
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
/// types or the rebuilt table.
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
            world.read_arrays(&house.arrays, 62).unwrap();
            world.renumber_surfaces(Some(&[]), Some(&[])).unwrap();
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

/// Load steps 6d and 6e: the lot is cleared again, which does nothing, and
/// the house's 245 saved objects are constructed and read.
///
/// A capture of the original taken when its object read (0x449c5e)
/// returns, by the old prototype's harness, was compared with this state
/// outside the repository; the capture is private. What was equal, for
/// all 245 objects: the ID and GUID, the 72 data words, the temps, the
/// attributes (their number too, which is the constructor's and for one
/// object more than the save holds), the scheduler word, the last result,
/// every frame's node, stack object, arguments, locals and saved primitive
/// state, the relationships, the number of slots, and the position; and
/// for the three people the 256 words of person data. Also equal: the
/// registration order, the tick, the lot size, the 64 globals, both game
/// random streams, the two IDs queued for removal, the scheduler cursor,
/// the funds, the indicator references, the one instance list, and the
/// arrays as in the test above. The multi-tile links were compared with
/// another capture, of the same house four ticks after its load: all 142
/// tiles name the same next tile.
///
/// One thing differed and is not explained: in one frame of object 19 the
/// capture's tree ID has bit 15 set (0x8118) where the save, and this
/// state, have 0x118. Object 17 has the same type and thread and no such
/// bit.
///
/// The C runtime stream and six type attribute words belong to the game
/// that was running, as in the tests above. With those copied from the
/// capture's state at the start of the load, the stream after the read was
/// equal to the capture's, so the people's constructors draw as many
/// numbers here as in the original, and the type attributes were equal.
/// That fixes how many draws there are, not what each is used for.
///
/// Not compared, because the capture does not hold it or `World` does
/// not: footprints, the short after the position, the frame arena size,
/// slot contents, dynamic sprite flags, each person's motives, queue and
/// animations, object flags, and [`World::skeleton_files`].
///
/// The digest is of our own state, as built here.
#[test]
fn the_goth_house_objects_are_constructed_and_read() {
    const PINNED: &str = "1034c34e3d93c040f31b0a56f9593e7e0ec7d8d319706b0afb591e8ac6f1531f";
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    with_goth_people(
        std::path::Path::new(&root),
        |house, interpreter, world, _, people| {
            world.clear_family().unwrap();
            interpreter.clear(world).unwrap();
            world.read_arrays(&house.arrays, 62).unwrap();
            world
                .renumber_surfaces(house.floor_map.as_deref(), house.wall_map.as_deref())
                .unwrap();
            interpreter.read_types(world, &house.types).unwrap();
            assert_eq!(interpreter.clear(world), Ok(vec![]));
            let before = world.clone();
            let read = interpreter
                .read_objects(world, &house.objects, people)
                .unwrap();
            assert_eq!(read.objects, house.objects.order);
            assert_eq!(world.order, house.objects.order);
            assert_eq!(world.objects.len(), 245);
            // Two adults and a child: both skeleton files are read, of 29
            // bones each at 48 draws a bone, and three copies built. The
            // rest are the three people's default interests.
            assert_eq!(world.skeleton_files.len(), 2);
            assert_eq!(read.draws, 5 * 29 * 48 + 114);
            let mut stream = before.crt_rng;
            for _ in 0..read.draws {
                stream.next_u15();
            }
            assert_eq!(world.crt_rng, stream);
            let mut padded = 0;
            for (id, object) in &world.objects {
                let saved = &house.objects.instances[id];
                assert_eq!(object.data, saved.data);
                assert_eq!(object.registry_word, saved.registry_word);
                assert!(object.attributes.len() >= 8);
                assert!(object.attributes.starts_with(&saved.attributes));
                padded += usize::from(object.attributes.len() > saved.attributes.len());
                if let Some(person) = &object.person {
                    assert_eq!((person.data.len(), object.slots.len()), (256, 6));
                }
            }
            assert_eq!(padded, 1);
            // 142 tiles in 52 groups.
            assert_eq!(world.next_parts.len(), 90);
            // The one instance list is of a type the house has none of.
            assert!(world.instances.values().all(Vec::is_empty));
            // Nothing else moves.
            assert_eq!(world.globals, before.globals);
            assert_eq!(world.removals, before.removals);
            assert_eq!(world.arrays, before.arrays);
            assert_eq!(
                (world.global_rng, world.lot_rng),
                (before.global_rng, before.lot_rng)
            );
            assert_eq!(world.hash_hex(), PINNED);
        },
    );
}
