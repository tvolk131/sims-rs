//! The lot resize at the start of a load, the clear that follows it, the
//! lot arrays, the type table, the saved objects, the controllers among
//! them, the object grid, the rooms, the household, the object visits and
//! the end of the load, on a real install when `SIMS_INSTALL` names one.
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
    let resized = interpreter.resize(&mut world).unwrap();
    assert_eq!(resized.removed, []);
    let created = resized.created;
    let guids = house.objects.instances.values().map(|o| o.guid);
    let people = People::load(&install, &index, &catalog, &neighborhood, guids).unwrap();
    then(&house, &interpreter, &mut world, &created, &people);
}

/// A lot's encoding as snapshot format 7 wrote it. Format 8 added fields
/// in three places: each object's selected mark; each room's object
/// counts, light, score and pool mark; and the rooms' waiting levels and
/// outdoor sum and score. This writes every other field, in the order
/// format 7 had them, and leaves those out. So a state pinned under
/// format 7 can be shown to be the same state but for the new fields.
fn format_7_bytes(world: &World) -> Vec<u8> {
    use serde::Serialize;
    use sims_formats::saved_objects as saved;
    use std::collections::{BTreeMap, BTreeSet};
    #[derive(Serialize)]
    struct Object7<'a> {
        id: u16,
        guid: u32,
        footprint: [i32; 4],
        position: [i32; 3],
        unknown_short: i16,
        attributes: &'a Vec<i16>,
        temps: &'a Vec<i16>,
        data: &'a Vec<i16>,
        obsolete_word: i16,
        stack_capacity_bytes: i32,
        stack: &'a Vec<saved::Frame>,
        relationship_flag: i32,
        relationships: &'a Vec<saved::Relationship>,
        slots: &'a Vec<saved::Slot>,
        dynamic_sprites: &'a Vec<i16>,
        multipart: &'a Option<saved::Multipart>,
        portal: &'a Option<Vec<f32>>,
        person: &'a Option<saved::Person>,
        registry_word: Option<i32>,
        last_result: bool,
        removal_skips_resets: bool,
    }
    #[derive(Serialize)]
    struct Room7<'a> {
        tiles: &'a Vec<[u8; 3]>,
        outdoors: bool,
        dirty: bool,
        half_tiles: i32,
        covered_half_tiles: i32,
        walls: i32,
        covered_walls: i32,
    }
    #[derive(Serialize)]
    struct Rooms7<'a> {
        map: &'a [Vec<i16>; 2],
        halves: &'a BTreeMap<[u8; 3], [sims_sim::rooms::Half; 2]>,
        rooms: BTreeMap<i16, Room7<'a>>,
    }
    #[derive(Serialize)]
    struct World7<'a> {
        ticks: u32,
        globals: &'a Vec<i16>,
        scheduler_cursor: i16,
        lot_rng: &'a sims_core::GameRng,
        global_rng: &'a sims_core::GameRng,
        crt_rng: &'a sims_core::CrtRng,
        size: i16,
        budget: &'a sims_sim::Budget,
        order: &'a Vec<u16>,
        types: &'a BTreeMap<u16, u32>,
        objects: BTreeMap<u16, Object7<'a>>,
        arrays: &'a BTreeMap<u16, sims_formats::lot::Array>,
        removals: &'a Vec<u16>,
        type_attributes: &'a Vec<sims_sim::TypeAttributes>,
        instances: &'a BTreeMap<u32, Vec<u16>>,
        event_owner: u16,
        magic_history: &'a Option<sims_sim::MagicHistory>,
        indicators: [u16; 2],
        changed_types: &'a BTreeSet<u32>,
        next_parts: &'a BTreeMap<u16, u16>,
        skeleton_files: &'a BTreeSet<String>,
        rooms: Rooms7<'a>,
        fires: &'a Vec<u16>,
        frame_storage: &'a BTreeMap<u16, sims_sim::FrameStorage>,
    }
    let objects = world
        .objects
        .iter()
        .map(|(&id, o)| {
            let object = Object7 {
                id: o.id,
                guid: o.guid,
                footprint: o.footprint,
                position: o.position,
                unknown_short: o.unknown_short,
                attributes: &o.attributes,
                temps: &o.temps,
                data: &o.data,
                obsolete_word: o.obsolete_word,
                stack_capacity_bytes: o.stack_capacity_bytes,
                stack: &o.stack,
                relationship_flag: o.relationship_flag,
                relationships: &o.relationships,
                slots: &o.slots,
                dynamic_sprites: &o.dynamic_sprites,
                multipart: &o.multipart,
                portal: &o.portal,
                person: &o.person,
                registry_word: o.registry_word,
                last_result: o.last_result,
                removal_skips_resets: o.removal_skips_resets,
            };
            (id, object)
        })
        .collect();
    let rooms = world
        .rooms
        .rooms
        .iter()
        .map(|(&id, r)| {
            let room = Room7 {
                tiles: &r.tiles,
                outdoors: r.outdoors,
                dirty: r.dirty,
                half_tiles: r.half_tiles,
                covered_half_tiles: r.covered_half_tiles,
                walls: r.walls,
                covered_walls: r.covered_walls,
            };
            (id, room)
        })
        .collect();
    let old = World7 {
        ticks: world.ticks,
        globals: &world.globals,
        scheduler_cursor: world.scheduler_cursor,
        lot_rng: &world.lot_rng,
        global_rng: &world.global_rng,
        crt_rng: &world.crt_rng,
        size: world.size,
        budget: &world.budget,
        order: &world.order,
        types: &world.types,
        objects,
        arrays: &world.arrays,
        removals: &world.removals,
        type_attributes: &world.type_attributes,
        instances: &world.instances,
        event_owner: world.event_owner,
        magic_history: &world.magic_history,
        indicators: world.indicators,
        changed_types: &world.changed_types,
        next_parts: &world.next_parts,
        skeleton_files: &world.skeleton_files,
        rooms: Rooms7 {
            map: &world.rooms.map,
            halves: &world.rooms.halves,
            rooms,
        },
        fires: &world.fires,
        frame_storage: &world.frame_storage,
    };
    postcard::to_allocvec(&old).unwrap()
}

/// Checks a state against its digest under today's snapshot format and
/// against the digest pinned for it under format 7
/// ([`format_7_bytes`]).
fn pinned(world: &World, now: &str, format_7: &str) {
    assert_eq!(world.hash_hex(), now);
    assert_eq!(digest(&format_7_bytes(world)), format_7);
}

/// A lot's encoding as snapshot format 6 wrote it, for a state reached
/// before load step 13. Two things changed since:
///
/// - Format 7 added two fields at the end of the encoding: the list of
///   fires, and each object's frame storage. This empties both, checks
///   that the encoding then ends in their two zero bytes, and drops them.
///   The frame storage is not empty in these states: it holds what the
///   trees that ran left behind. Format 6 did not record that.
/// - The factory now draws 32 numbers from the C runtime's random stream
///   for an object with graphics of its own. The two indicators the lot
///   starts with are such objects, so the stream is 64 draws further on
///   before the house's people are constructed. Their constructors draw
///   until a drawn value fits, so from another state they draw another
///   number of times: 7,084 where they drew 7,074. Nothing they draw is
///   kept. This puts the stream back in the state it had under format 6:
///   its initial state 1 advanced by `draws` numbers.
///
/// So a state pinned under format 6 can be shown to be the same state but
/// for the frame storage and the stream.
fn format_6_bytes(world: &World, draws: u32) -> Vec<u8> {
    let mut old = world.clone();
    old.fires.clear();
    old.frame_storage.clear();
    old.crt_rng = sims_core::CrtRng::from_state(1);
    for _ in 0..draws {
        old.crt_rng.next_u15();
    }
    let mut encoded = format_7_bytes(&old);
    assert_eq!(encoded.split_off(encoded.len() - 2), [0; 2]);
    encoded
}

/// The digest of [`format_6_bytes`].
fn format_6(world: &World, draws: u32) -> String {
    digest(&format_6_bytes(world, draws))
}

/// How many numbers the C runtime's random stream had drawn, under
/// format 6, once the Goth house's people were constructed.
const DRAWS_6: u32 = 7074;

/// The digest of a lot's encoding as snapshot format 5 wrote it. Format 6
/// added the lot's rooms at the end of the encoding, and a lot whose rooms
/// have not been worked out encodes them as four zero bytes. This checks
/// that the format 6 encoding ends in those and digests the rest, so a
/// state pinned under format 5 can be shown to be the same state.
fn format_5(world: &World, draws: u32) -> String {
    let encoded = format_6_bytes(world, draws);
    let (old, new) = encoded.split_at(encoded.len() - 4);
    assert_eq!(new, [0; 4]);
    digest(old)
}

/// The lot with the ground level's wall cells outside the lot's 52 tiles
/// zeroed. That is what those cells held here when the digests of format
/// 5 were pinned. They now hold a bare lot's bytes
/// ([`sims_sim::load::BARE_GROUND_WALL`]), as the original's do. Used to
/// show that a state pinned earlier differs from today's in those cells
/// only.
fn with_zeroed_outer_walls(world: &World) -> World {
    let mut world = world.clone();
    let walls = &mut world.arrays.get_mut(&2).unwrap().data;
    for (cell, bytes) in walls.chunks_mut(14).enumerate() {
        if cell / 64 >= 52 || cell % 64 >= 52 {
            assert_eq!(bytes, sims_sim::load::BARE_GROUND_WALL);
            bytes.fill(0);
        }
    }
    world
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
/// The digests are of our own state, as built here. Snapshot format 8
/// added fields in the middle of the encoding; `format_7_bytes` writes the
/// state without them, and every state in this file that was pinned under
/// format 7 is checked against that digest too ([`pinned`]). Snapshot format 7 added
/// the frame storage and the list of fires, and the indicators the lot
/// starts with now draw from the C runtime stream; `format_6_bytes` says
/// how the encoding is shown to be the state pinned under format 6
/// otherwise. Snapshot format 6 added
/// the lot's rooms at the end of the encoding, which are empty here: four
/// zero bytes. Without them the encoding hashes to the digest pinned under
/// format 5. Format 5 had added two fields (the multi-tile links and the
/// skeleton files read), both empty here; without those two bytes as well
/// the encoding still hashes to the digest pinned under format 4, which in
/// turn was the format 3 state that comparison was made on plus three
/// bytes.
#[test]
fn the_goth_house_resize_creates_the_controllers() {
    const PINNED: &str = "a1b9ee0fce4ac7efc57ff249f932ad0eee2d609e545a639b0f1bd62b7b27dbf4";
    const PINNED_7: &str = "540a7a822c65b08918239a93d0accb9d46f26c5ab16400573e8d30e88f3cba90";

    const FORMAT_6: &str = "5da903b012eb85ced13111300ed5fd6f125ba083eb7517bac76e5c342eae2d91";
    const FORMAT_5: &str = "9e5cab3b2caf4e1915faf58b9a3ef5fe192495b44cfb09420b9396368947bd15";
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
    pinned(&world, PINNED, PINNED_7);
    assert_eq!(format_6(&world, 0), FORMAT_6);
    assert_eq!(format_5(&world, 0), FORMAT_5);
    let encoded = format_6_bytes(&world, 0);
    let (old, new) = encoded.split_at(encoded.len() - 6);
    assert_eq!((digest(old).as_str(), new), (FORMAT_4, &[0; 6][..]));
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
/// Snapshot format 7 changed the encoding again; `format_6_bytes` says how
/// this state is shown to be the one pinned under format 6.
///
/// The digest is of our own state, as built here. It is the state pinned
/// under snapshot format 4, the one that comparison was made on, plus the
/// two bytes of format 5's new fields and the four of format 6's, which
/// are all empty here.
#[test]
fn the_goth_house_clear_removes_every_object() {
    const PINNED: &str = "5893437d45ac7683f04682ff556b863ba1789440ba5f600ea6910505cba469cc";
    const PINNED_7: &str = "1c0600774ae59cb72770086809c94d54b65fd747e09fb5ddf79b7b62b675e8a6";

    const FORMAT_6: &str = "4f0b7439a1379c998d885eb10f2a12c0cb887c29d08524a6600d80da7c7486e7";
    const FORMAT_5: &str = "b97f264759c4f6be2b3a356bb7a55bbdb662f2b0fdd80f1e87f613522362cae0";
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
        pinned(world, PINNED, PINNED_7);
        assert_eq!(format_6(world, 0), FORMAT_6);
        assert_eq!(format_5(world, 0), FORMAT_5);
        let encoded = format_6_bytes(world, 0);
        let (old, new) = encoded.split_at(encoded.len() - 6);
        assert_eq!((digest(old).as_str(), new), (FORMAT_4, &[0; 6][..]));
        // The original clears the lot again before it reads the saved
        // objects. On an empty lot that does nothing.
        assert_eq!(interpreter.clear(world), Ok(vec![]));
        pinned(world, PINNED, PINNED_7);
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
/// Outside the 52 tiles that capture holds nothing. A later trace of the
/// same load saved pages of the game's memory at each stop. At the entry
/// of load step 10 (0x442659), which is after these steps and after no
/// step that writes these arrays, they hold every cell of both levels'
/// flags, of the ground level's object grid, of the pool and of the
/// water; the 688 to 1136 cells outside the 52 tiles of the heights, the
/// floors, the upper level's object grid and both levels' walls; and at
/// the end of the load every cell of both levels' walls. All of those
/// were equal to the arrays here. One correction came of it: the ground
/// level's walls outside the 52 tiles are not zero in the original. Each
/// cell there has the bytes of a bare lot's cell, which the held lot had
/// before the load and which is also what the house saved there.
///
/// Not compared, because neither holds them: arrays 4, 6 and 7 (ground
/// and grass). The digest is of our own arrays. With the ground level's
/// wall cells outside the 52 tiles zeroed, the arrays hash to the digest
/// pinned before that correction.
#[test]
fn the_goth_house_arrays_are_read_and_nothing_is_renumbered() {
    const PINNED: &str = "d21e8dd484bde265d6fa0bcc23e0da12f9aa3cc83910da49cc49fe5ac6cbc0dd";
    const ZEROED: &str = "023361585d1b6644d33928f33039ef73c33e347e3c01c81cc3a8b0004ddcb1f3";
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
                // widened; one outside holds what a bare lot holds.
                for (x, y) in [(0, 0), (26, 28), (51, 51), (52, 0), (63, 63)] {
                    let (saved, read) = (saved.cell(x, y).unwrap(), read.cell(x, y).unwrap());
                    // A floor byte becomes a word. A wall keeps its first
                    // two bytes and its other six become words.
                    let keep = if stride == 14 { 2 } else { 0 };
                    let mut wide = saved[..keep].to_vec();
                    wide.extend(saved[keep..].iter().flat_map(|&byte| [byte, 0]));
                    if x < 52 && y < 52 {
                        assert_eq!(read, wide, "array {id}");
                    } else if id == 2 {
                        // The held lot's own cell, which is also what
                        // the house saved there.
                        assert_eq!(read, sims_sim::load::BARE_GROUND_WALL);
                        assert_eq!(read, wide);
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
            let arrays = |world: &World| {
                let mut arrays = Vec::new();
                for (id, array) in &world.arrays {
                    arrays.extend(id.to_le_bytes());
                    arrays.extend((array.stride as u16).to_le_bytes());
                    arrays.extend(&array.data);
                }
                digest(&arrays)
            };
            assert_eq!(arrays(world), PINNED);
            assert_eq!(arrays(&with_zeroed_outer_walls(world)), ZEROED);
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
/// Snapshot format 7 changed the encoding again; `format_6_bytes` says how
/// this state is shown to be the one pinned under format 6.
///
/// The digest is of our own state, as built here. It differs in two ways
/// from the state pinned under snapshot format 5, the one that comparison
/// was made on. Format 6 added the lot's rooms at the end of the
/// encoding, which are empty here: four zero bytes. And the ground
/// level's wall cells outside the lot's 52 tiles now hold a bare lot's
/// bytes, as the original's do (see the arrays test above). With those
/// cells zeroed and without the four bytes, the encoding hashes to the
/// digest pinned under format 5.
#[test]
fn the_goth_house_objects_are_constructed_and_read() {
    const PINNED: &str = "8f448a4a8e3c7fbd295b5fe61808b392e6c16692d81efb65df9b5fbf3a7da267";
    const PINNED_7: &str = "d6b094744f72f7bd3dfbc76c30e500dbaff86350487bf6306d2c2c2c5f4467d3";

    const FORMAT_6: &str = "bda1f8eee5deabee63609ec1a2a8ea1e027161c7390b19a830bf7ed4c346c81e";
    const FORMAT_5: &str = "1034c34e3d93c040f31b0a56f9593e7e0ec7d8d319706b0afb591e8ac6f1531f";
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
            // rest are the three people's default interests, which draw
            // until a value fits: 124 times from the state the stream is
            // in here, 64 draws on from its initial state.
            assert_eq!(world.skeleton_files.len(), 2);
            assert_eq!(read.draws, 5 * 29 * 48 + 124);
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
            pinned(world, PINNED, PINNED_7);
            assert_eq!(format_6(world, DRAWS_6), FORMAT_6);
            assert_eq!(format_5(&with_zeroed_outer_walls(world), DRAWS_6), FORMAT_5);
        },
    );
}

/// Load steps 8 and 9: the lot-wide controllers are reconciled among the
/// 245 saved objects, which creates the 137 the house did not save, and
/// the ground level's object grid is checked, which clears nothing.
///
/// A trace of the original loading this house, taken by the old
/// prototype's harness, stops when the function that does both (0x43fc02)
/// returns. That stop was compared with this state outside the repository;
/// the trace is private. What was equal, for all 382 objects: the ID and
/// GUID, the 72 data words, the temps, the attributes and their number,
/// the scheduler word, the last result, every frame's node, stack object,
/// arguments, locals and saved primitive state, the relationships, the
/// number of slots, the position, the tile and level bytes the grid check
/// reads (they are the position divided by 16), and the mark of an object
/// that asked to be removed without resets; and for the three people the
/// 256 words of person data. Also equal: the registration order, the
/// tick, the lot size, the 64 globals, both game random streams, the four
/// IDs queued for removal, the scheduler cursor, the funds, the indicator
/// references, the one instance list, and inside the lot's 52 tiles the
/// terrain heights, both levels' floors, walls, object grid and flags, the
/// pool and the water. The trace's stop before these steps differs from
/// this one in the 137 new objects, the global random stream and the
/// removal queue, and in nothing else it holds: no saved object and no
/// grid cell changes there either.
///
/// The object ID in each slot was compared for the 216 objects whose slot
/// memory the trace holds at this stop or the one before, and was equal.
/// Those hold 30 of the 32 links between objects here.
///
/// One thing differed, as after the object read, and is still not
/// explained: object 19's second frame has tree ID 0x8118 in the trace and
/// 0x118 here.
///
/// The C runtime stream and six type attribute words belong to the game
/// that was running. With those copied from the trace's state at the
/// start of the load they were equal too.
///
/// Not compared with the original: removing a duplicate controller and
/// clearing a grid cell, neither of which this house needs; the slots of
/// the other 166 objects; every cell outside the 52 tiles; and what the
/// test above lists as not held.
///
/// Snapshot format 7 changed the encoding again; `format_6_bytes` says how
/// this state is shown to be the one pinned under format 6.
///
/// The digest is of our own state, as built here. As in the test above,
/// it is the state pinned under snapshot format 5, the one that comparison
/// was made on, but for the ground level's wall cells outside the lot's
/// 52 tiles and the four bytes of format 6's new field.
#[test]
fn the_goth_house_controllers_are_reconciled_and_the_grid_is_checked() {
    const PINNED: &str = "91d8845e2a929e3c9fd7b4b26f691b56ddf26e444d1f38e83ad1a27b6bf1b140";
    const PINNED_7: &str = "3724e0d6e307e13a398a6a8c47e7523e1fe4f7f83cb575c4ec6cb4a6aceabdd1";

    const FORMAT_6: &str = "2c000ad6962e840ece561db6b3a6ff572d8dd83815680c67a2a7eb861d3698b1";
    const FORMAT_5: &str = "d014a3ce407d420fc6b69248f9e23282b0fdc348bac79afc5670629b60ed46b1";
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
            interpreter.clear(world).unwrap();
            interpreter
                .read_objects(world, &house.objects, people)
                .unwrap();
            let before = world.clone();
            // Step 8.
            let done = interpreter.reconcile_controllers(world).unwrap();
            assert_eq!(done.removed, []);
            assert_eq!(done.created.len(), 137);
            // New objects take the lowest unused IDs, so the first four
            // fill the gaps the house left.
            assert_eq!(done.created[..5], [4, 15, 76, 77, 247]);
            assert_eq!(done.created[136], 382);
            // Each goes to the head of the registration list.
            let mut order: Vec<u16> = done.created.iter().rev().copied().collect();
            order.extend(&before.order);
            assert_eq!(world.order, order);
            // The four controllers the house saved are kept as they are,
            // and so is every other saved object.
            let controllers = interpreter
                .catalog
                .objects
                .iter()
                .filter(|d| {
                    let words = interpreter.program.definition(d.resource, d.chunk).unwrap();
                    matches!(words[9], 7 | 12) && words[42] != 0
                })
                .count();
            assert_eq!(controllers, 141);
            for (id, saved) in &before.objects {
                assert_eq!(format!("{:?}", world.objects[id]), format!("{saved:?}"));
            }
            // Two of the new controllers ask to be removed, as two did
            // after the resize. The two older requests are still queued,
            // and their IDs now name saved objects.
            assert_eq!(before.removals, [42, 44]);
            assert_eq!(world.removals, [42, 44, 278, 280]);
            for id in [42, 44, 278, 280] {
                assert_eq!(world.objects[&id].removal_skips_resets, id > 44);
            }
            assert_ne!(world.global_rng, before.global_rng);
            assert_eq!(
                (world.lot_rng, world.crt_rng),
                (before.lot_rng, before.crt_rng)
            );
            assert_eq!(world.globals, before.globals);
            assert_eq!(world.type_attributes, before.type_attributes);
            assert_eq!(world.arrays, before.arrays);
            // Step 9. The saved grid names objects that are elsewhere only
            // outside the lot's 52 tiles, where the original does not
            // look: scanning all 64 would clear eight cells.
            let reconciled = world.clone();
            assert_eq!(world.repair_grid(), Ok(vec![]));
            assert_eq!(world.encode(), reconciled.encode());
            let mut whole = world.clone();
            whole.size = 64;
            let outside = whole.repair_grid().unwrap();
            assert_eq!(outside.len(), 8);
            assert!(outside.iter().all(|r| r.tile[0] >= 52 || r.tile[1] >= 52));
            assert_eq!(world.objects.len(), 382);
            pinned(world, PINNED, PINNED_7);
            assert_eq!(format_6(world, DRAWS_6), FORMAT_6);
            assert_eq!(format_5(&with_zeroed_outer_walls(world), DRAWS_6), FORMAT_5);
        },
    );
}

/// The lot's rooms are worked out from its walls, and then load steps 10
/// to 12 run: every object is given its room and asked its function 7,
/// the rooms are counted, the removal queue is emptied, and the Goth
/// family is selected.
///
/// A trace of the original loading this house, taken by the old
/// prototype's harness, stops at the entry and the return of step 10
/// (0x442659), at the entry and the return of step 11 (0x428679) and when
/// step 12's funds setter (0x46d8dc) returns. Each stop was compared with
/// this state at the same moment, outside the repository; the trace is
/// private.
///
/// *The rooms.* Equal: the 11 room IDs; how many tiles each room lists;
/// the listed tiles, in order, of the 8 rooms whose tile memory the trace
/// holds (1, 2, 4, 1025 to 1029); the room of every tile of the upper
/// level in the 32 columns the trace holds at the entry of step 10; the
/// number of split tiles; whether each room is outdoors; and every cell
/// of both levels' tile flags, which the rebuild leaves as the house
/// saved them. The room of every tile of both levels,
/// all 64 columns, was compared with the trace's memory at the end of the
/// load and was equal; the trace's room list is the same there as at the
/// entry of step 10, so the maps are taken to be the same too. An empty
/// lot of 52 tiles partitions here into two rooms that list as many tiles
/// as the two rooms the trace has before the rebuild.
///
/// *Step 10.* The trace asks function 7 of all 382 objects in
/// registration order. The 62 whose thread it touches are the 62 that
/// have a function 7 tree here, in the same order. No data word changes
/// in the trace, and none here: the room worked out for each of the 382
/// objects is the room the house saved. Afterwards all the fields listed
/// for the test above are equal for all 382 objects, the last result
/// included, and so are the globals, the random streams and the arrays.
///
/// *The room count.* Equal for all 11 rooms, at the entry of step 11:
/// the twelve counts (half tiles, covered half tiles, objects, window
/// light, lamp light, the sum of data word 39, impact, good things, the
/// two categories, walls and covered walls), the lights and the people,
/// the score and the light bit for bit, whether the room is outdoors and
/// whether it is a pool, and the mark that it waits. Equal: the levels
/// that wait (none), the lot's outdoor sum and outdoor score bit for bit,
/// and global 27, the indoor area, which is 204. The trace has no stop
/// between the two room updates the original makes here, so the state
/// after the first of them is not compared.
///
/// *Steps 11 and 12.* Equal after each: the same fields, the removal
/// queue (empty), and after step 12 the globals (9 becomes 5 and 13
/// becomes 2542), the funds, the selected family's ID and its six magic
/// words.
///
/// What differed: object 19's second frame, as in the tests above. The C
/// runtime stream and six type attribute words belong to the game that
/// was running, and were equal once copied from the trace's start.
///
/// Not compared with the original, because the trace does not hold it:
/// the listed tiles of rooms 0, 3 and 1024; which room each triangle of a
/// split tile has; the room of an object on a split tile, which this
/// house has none of; room fields 2 and 3 as scripts read them, which no
/// tree reads here; and global 10 being written, since the house saved
/// its own number there. Not modeled, so not compared: which rooms know
/// they have people in them, and the lighting of the tiles.
///
/// Snapshot format 7 changed the encoding again; `format_6_bytes` says how
/// this state is shown to be the one pinned under format 6.
///
/// The digests are of our own state, as built here.
#[test]
fn the_goth_house_gets_its_rooms_and_its_household() {
    const COUNTED: &str = "c1c8a7ad538c67dd4f5dd23ce46e45f95992a05fcb2b7dba5c58d81ca8cb75c2";
    const COUNTED_7: &str = "c4c4bba18e2f07090e33ce5c823692498d3cfeac2b50d0652d2315f7ed040829";
    const SELECTED: &str = "3199667febb0a83088ee468c3f5d6713a037e03d0961547fdf41e7a20719b0a5";
    const SELECTED_7: &str = "09fcefc61a7bc468bee455e8e7e6ce0e551e9847bf6c6d67b3d773022140609e";
    const COUNTED_6: &str = "d994b4eb25b6bb9b8ec8202bb1ede1f33ae40513f9cc2745298d76675ab6d476";
    const SELECTED_6: &str = "6a9a7290da69b6e376c09a57295217df82ef28a8a7259eff97420d5c7c1e649d";
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = std::path::Path::new(&root);
    let install = Install::detect(root).unwrap();
    let neighborhood = Neighborhood::open(&install.file("UserData").unwrap()).unwrap();
    with_goth_people(root, |house, interpreter, world, _, people| {
        world.clear_family().unwrap();
        interpreter.clear(world).unwrap();
        world.read_arrays(&house.arrays, 62).unwrap();
        world
            .renumber_surfaces(house.floor_map.as_deref(), house.wall_map.as_deref())
            .unwrap();
        interpreter.read_types(world, &house.types).unwrap();
        interpreter.clear(world).unwrap();
        interpreter
            .read_objects(world, &house.objects, people)
            .unwrap();
        interpreter.reconcile_controllers(world).unwrap();
        world.repair_grid().unwrap();
        // The rooms. The rebuild changes no array: the flags it works
        // out are the flags the house saved, also outside the lot's size.
        let before = world.clone();
        world.rebuild_rooms().unwrap();
        assert_eq!(world.arrays, before.arrays);
        let rooms = &world.rooms;
        let ids: Vec<i16> = rooms.rooms.keys().copied().collect();
        assert_eq!(ids, [0, 1, 2, 3, 4, 1024, 1025, 1026, 1027, 1028, 1029]);
        let outdoors: Vec<i16> = ids
            .iter()
            .copied()
            .filter(|id| rooms.rooms[id].outdoors)
            .collect();
        // The three single triangles upstairs count as outdoors.
        assert_eq!(outdoors, [0, 1024, 1027, 1028, 1029]);
        assert_eq!(rooms.halves.len(), 7);
        assert!(rooms.rooms.values().all(|room| room.dirty));
        // Every tile inside the lot's size has a room or is split, and no
        // tile outside it has.
        for map in &rooms.map {
            for (cell, &room) in map.iter().enumerate() {
                let inside = cell / 64 < 52 && cell % 64 < 52;
                assert_eq!(room != sims_sim::rooms::UNSEEN, inside);
            }
        }
        let mut expected = before.clone();
        expected.rooms = world.rooms.clone();
        assert_eq!(world.encode(), expected.encode());
        // Step 10. Every room is the one the house saved.
        let roomed = world.clone();
        let done = interpreter.assign_rooms(world).unwrap();
        assert_eq!(done.moved, []);
        assert_eq!(done.asked.len(), 62);
        // The trees are those of doors and windows. Each returns true,
        // which is all it leaves behind.
        let mut expected = roomed.clone();
        for id in &done.asked {
            let object = expected.objects.get_mut(id).unwrap();
            assert!(!object.last_result);
            object.last_result = true;
        }
        // ... and what they left in the frame storage.
        expected.frame_storage = world.frame_storage.clone();
        assert_eq!(world.encode(), expected.encode());
        // The room count. The first update gives every room light, so
        // every room is announced as changed and marked again. The end of
        // the house read marks them all and counts what waits, which
        // changes no light.
        let asked = world.clone();
        // The global file's table gives the constants the original's code
        // falls back to.
        let tuning = interpreter.program.room_tuning();
        assert_eq!(tuning, sims_sim::rooms::Tuning::default());
        let table = &interpreter.program.libraries[&interpreter.program.global].tuning;
        assert!(table[&3].get("area clip to score").is_some());
        assert_eq!(interpreter.update_rooms(world, 0).unwrap(), ids);
        assert_eq!(world.globals[27], 204);
        assert!(world.rooms.rooms.values().all(|room| room.dirty));
        assert_eq!(world.rooms.pending, 0);
        let first = world.rooms.clone();
        world.mark_all_rooms();
        assert_eq!(interpreter.update_pending_rooms(world).unwrap(), ids);
        assert!(world.rooms.rooms.values().all(|room| !room.dirty));
        for (id, room) in &mut world.rooms.rooms.clone() {
            // The second count is the first again, but for the marks.
            room.dirty = true;
            assert_eq!(*room, first.rooms[id]);
            // By day every room is fully lit: the outdoor rooms by the
            // daylight and the indoor ones through their windows.
            assert_eq!(room.light, sims_core::F32::from_f32(1.0));
        }
        let mut expected = asked.clone();
        expected.globals[27] = 204;
        expected.rooms = world.rooms.clone();
        assert_eq!(world.encode(), expected.encode());
        pinned(world, COUNTED, COUNTED_7);
        assert_eq!(format_6(world, DRAWS_6), COUNTED_6);
        // Step 11.
        assert_eq!(world.discard_removals(), [42, 44, 278, 280]);
        // Step 12.
        let (id, family) = neighborhood.family_in(5).unwrap();
        let magic = family.magic_history().unwrap();
        assert_eq!((id, family.budget, magic), (5, 2542, Some([0; 6])));
        let mut expected = world.clone();
        world.select_family(5, 5, family.budget, magic).unwrap();
        assert_eq!((asked.globals[9], asked.globals[10]), (0, 5));
        assert_eq!((asked.globals[13], asked.budget.funds), (2515, 2515));
        (expected.globals[9], expected.globals[13]) = (5, 2542);
        expected.budget.funds = 2542;
        expected.magic_history = Some(sims_sim::MagicHistory {
            family: 5,
            values: [0; 6],
        });
        assert_eq!(world.encode(), expected.encode());
        pinned(world, SELECTED, SELECTED_7);
        assert_eq!(format_6(world, DRAWS_6), SELECTED_6);
    });
}

/// The lot after load step 12, built the way `sims boot` builds it.
fn selected(root: &std::path::Path, then: impl FnOnce(&Interpreter, &mut World, &People)) {
    let install = Install::detect(root).unwrap();
    let neighborhood = Neighborhood::open(&install.file("UserData").unwrap()).unwrap();
    with_goth_people(root, |house, interpreter, world, _, people| {
        world.clear_family().unwrap();
        interpreter.clear(world).unwrap();
        world.read_arrays(&house.arrays, 62).unwrap();
        world
            .renumber_surfaces(house.floor_map.as_deref(), house.wall_map.as_deref())
            .unwrap();
        interpreter.read_types(world, &house.types).unwrap();
        interpreter.clear(world).unwrap();
        interpreter
            .read_objects(world, &house.objects, people)
            .unwrap();
        interpreter.reconcile_controllers(world).unwrap();
        world.repair_grid().unwrap();
        world.rebuild_rooms().unwrap();
        interpreter.assign_rooms(world).unwrap();
        interpreter.update_rooms(world, 0).unwrap();
        world.mark_all_rooms();
        interpreter.update_pending_rooms(world).unwrap();
        world.discard_removals();
        let (id, family) = neighborhood.family_in(5).unwrap();
        let magic = family.magic_history().unwrap();
        world
            .select_family(id as i16, 5, family.budget, magic)
            .unwrap();
        then(interpreter, world, people);
    });
}

/// Load steps 13 to 15 on the Goth house, and the first half of step 16:
/// the original visits all 382 objects in registration order, asks for
/// the indicators, finishes the household selection and, when the house
/// is announced, asks for the indicators again. All of it runs here. The
/// test below takes the load from there to its end.
///
/// A trace of the original loading this house, taken by the old
/// prototype's harness, was compared with this run outside the
/// repository; the trace is private. It holds a full state at the step's
/// entry, at the entry and the return of the indicators' creation
/// (0x445e4a) in the middle of visit 140 and again after the last visit,
/// at the step's return, at the return of the lighting update (0x4c4648)
/// that ends the household selection, and at the return of the
/// indicators' creation called when the house is announced. For every
/// visit it holds each call of "run this object's function" (0x422d46).
///
/// Equal to the trace:
///
/// - For each of the 382 visits: the object visited, and the calls of
///   0x422d46 the visit makes, as object and function, in order (1,781
///   calls). The three people (visits 202 to 204) are asked functions 3
///   and 0 and nothing else.
/// - In the middle of visit 140, at the moment the visit creates the
///   indicators, and when that creation returns: see the list below, with
///   the exceptions the decision log gives.
/// - At the step's return, and at each of the three later stops: for all
///   386 objects the GUID, the 72 data words, temps, attributes,
///   scheduler word, last result, every frame's tree, node, stack object,
///   parameters, locals and saved primitive state, the relationships, the
///   number of slots, the position and the removal-without-resets mark;
///   for the three people the 256 person data words, the 16 motives and
///   the 16 previous motives bit for bit, the running and the last action,
///   the queued actions and the number of objects in use; the tick, the 64
///   globals, the three random streams, the funds, the removal queue, the
///   indicator references, the registration order, the instance lists,
///   which rooms wait to be counted, and the lot arrays inside the lot's
///   size (terrain, floors, walls, object grids, tile flags, pool,
///   water). The trace changes none of this between the step's return and
///   the last of those stops, and neither do steps 14 to 16 here.
///
/// What differed: object 19's second frame has tree 0x8118 in the trace
/// and 0x118 here and in the house file at the two stops of visit 140, as
/// in the tests above. Its visit is number 206. It makes the same calls
/// here as in the trace, and at the step's return the object is equal in
/// every field compared: the reset replaced its thread. Six type attribute
/// words belong to the game that was running, and were equal once copied
/// from the trace's start.
///
/// Not compared with the original, because the trace does not hold it:
/// the contents of the objects' slots and frame storage (the link words,
/// the object grids and the frames' locals, which mirror them, are
/// compared); a person's motive changes in progress, of which this house
/// has none; and the state between two visits after visit 140. Of a
/// person's reset the trace shows the restarted thread and what the Reset
/// and Init trees write. The words the reset clears or reloads already
/// hold those values in this house, so the trace shows them unchanged
/// and cannot show them being written. The same goes for the fields a
/// person takes from its neighborhood record, except words 46 to 55,
/// which do change, and to the trace's values.
///
/// The digests are of our own state, as built here. The first is the
/// state after visit 201, pinned when the step stopped there.
#[test]
fn the_goth_house_objects_are_all_visited() {
    const VISIT_201: &str = "fe7546148470609e9fd73a30dd2b0a79d306d1556e0bbb3b536098adf42f27c8";
    const VISIT_201_7: &str = "13ae49d59cfa5cc946e9b9522b2e7e8634ec060af08d3df31eadd566057d5717";
    const VISITED: &str = "2374a83ff7cad09db1a9252328948ba39d798f1ead256c6239d4c9429a295de8";
    const VISITED_7: &str = "3986e39715b51dfe7d82f23d7644fc654fe1634c6e1b3e88da7aa891f0e5d73c";
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = std::path::Path::new(&root);
    let install = Install::detect(root).unwrap();
    let neighborhood = Neighborhood::open(&install.file("UserData").unwrap()).unwrap();
    let flags = neighborhood.family_in(5).unwrap().1.flags;
    selected(root, |interpreter, world, people| {
        let asked = std::cell::RefCell::new(Vec::new());
        let mut interpreter = *interpreter;
        interpreter.asked = Some(&asked);
        let before = world.clone();
        // The first 201 visits, one at a time: the state pinned earlier.
        let mut early = world.clone();
        for &id in &before.order[..201] {
            interpreter.visit(&mut early, id, people).unwrap();
        }
        pinned(&early, VISIT_201, VISIT_201_7);
        asked.borrow_mut().clear();
        let done = interpreter.visit_objects(world, people);
        assert_eq!(done.stopped, None);
        assert_eq!(done.objects, before.order);
        assert_eq!(done.reset.len(), 184);
        // The calls of each function: Init, Load, Reset, walls changed,
        // placed and leaving.
        let mut calls = std::collections::BTreeMap::new();
        for call in asked.borrow().iter() {
            *calls.entry(call.function).or_insert(0) += 1;
        }
        let calls: Vec<(usize, i32)> = calls.into_iter().collect();
        assert_eq!(
            calls,
            [(0, 578), (2, 379), (3, 26), (6, 4), (9, 404), (10, 390)]
        );
        // The people are visits 202 to 204. Each is asked Reset and Init.
        assert_eq!(before.order[201..204], [21, 18, 16]);
        for id in [21, 18, 16] {
            let mine: Vec<usize> = asked
                .borrow()
                .iter()
                .filter(|call| call.object == id)
                .map(|call| call.function)
                .collect();
            assert_eq!(mine, [3, 0]);
            let person = world.objects[&id].person.as_ref().unwrap();
            let old = before.objects[&id].person.as_ref().unwrap();
            // Interests come from the neighborhood record.
            assert_ne!(person.data[46..54], old.data[46..54]);
            assert_eq!(person.data[..46], old.data[..46]);
            // The age, skin tone and gender read from the type's body
            // strings are the ones the house saved.
            assert_eq!(person.data[56..], old.data[56..]);
            assert!(people.profiles[&world.objects[&id].guid].tones.len() > 1);
            assert_eq!(world.objects[&id].stack.len(), 1);
        }
        // Visit 140 put the first rebuilt object back on the lot, which
        // made the two indicators. A script made two more objects. All
        // four went to the head of the registration list.
        assert_eq!(world.indicators, [383, 384]);
        assert_eq!(world.order[..4], [386, 385, 384, 383]);
        assert_eq!(world.order[4..], before.order[..]);
        assert_eq!(world.instances[&0xfd4f_b8e4], [385, 386]);
        // Seven controllers asked to be removed in their Load trees.
        assert_eq!(world.removals, [355, 352, 350, 313, 312, 305, 292]);
        // The magic controller was looked up for the selected family.
        assert_eq!(world.instances[&0xb6c9_0029], [376]);
        assert_eq!((before.globals[36], world.globals[36]), (0, 1));
        // Eight of the eleven rooms wait to be counted again.
        let waiting: Vec<i16> = world
            .rooms
            .rooms
            .iter()
            .filter(|(_, room)| room.dirty)
            .map(|(&id, _)| id)
            .collect();
        assert_eq!(waiting, [0, 1, 2, 3, 4, 1024, 1025, 1026]);
        // Nothing here advances the clock or touches the lot's stream.
        assert_eq!((world.ticks, world.lot_rng), (before.ticks, before.lot_rng));
        assert_ne!(world.global_rng, before.global_rng);
        pinned(world, VISITED, VISITED_7);
        // Steps 14 and 15 change nothing on this lot, and neither does
        // asking for the indicators again in step 16.
        assert_eq!(interpreter.ensure_indicators(world), Ok(None));
        world.finish_selection(flags).unwrap();
        assert_eq!(interpreter.ensure_indicators(world), Ok(None));
        pinned(world, VISITED, VISITED_7);
    });
}

/// Load steps 16 to 22 on the Goth house: the house is announced, the
/// funds are set again, a Sim is selected, the game goes to live mode,
/// the waiting rooms are counted and the interface's wait check runs.
/// The state this ends in is the one the original's first tick starts
/// from.
///
/// The trace of the original loading this house (see the test above) was
/// compared with this run outside the repository; the trace is private.
/// It stops at the entry and the return of the funds setter (0x46d8dc),
/// of the selection (0x445449, twice), of the mode change (0x4cea5d,
/// twice) and of the lot's part in it (0x416534), of the wait check
/// (0x4764fe), and at the entry of the first tick (0x46d345). It holds
/// every call of "run this object's function" (0x422d46) between them.
///
/// Equal to the trace, at the stop after each of our steps (the entry of
/// the funds setter, the return of the first selection, the return of
/// 0x416534, the entry of the second selection, the entry of the wait
/// check, and the entry of the first tick): everything the test above
/// lists for the end of step 13, and also each object's selected mark
/// and, for all 11 rooms, everything the rooms test lists for the room
/// count, with the levels that wait and the lot's outdoor sum and score.
/// Equal: the four callbacks asked while the Sim is selected, in order,
/// and that no other callback is asked between the end of step 13 and
/// the first tick. At the first tick the scheduler cursor is 0 in both.
///
/// What the trace cannot show, because the Goth house has only one case
/// of it: which Sim is selected when the family's first member is not on
/// the lot or may not be selected; a Sim with a date; and a lot whose
/// statistics set other bits of global 37. The parts of the statistics
/// pass that are not ported (global 22, the object and architecture
/// values) hold their saved values in the trace too, so it does not show
/// them being written. Six type attribute words belong to the game that
/// was running, and were equal once copied from the trace's start.
///
/// The digest is of our own state, as built here.
#[test]
fn the_goth_house_load_ends_where_the_first_tick_starts() {
    const BASELINE: &str = "13dd5b8ee042ed292266090ef1b7b650b2a6576531279c27db9af23e9c108648";
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = std::path::Path::new(&root);
    let install = Install::detect(root).unwrap();
    let neighborhood = Neighborhood::open(&install.file("UserData").unwrap()).unwrap();
    let family = neighborhood.family_in(5).unwrap().1.clone();
    selected(root, |interpreter, world, people| {
        let asked = std::cell::RefCell::new(Vec::new());
        let mut interpreter = *interpreter;
        assert_eq!(interpreter.visit_objects(world, people).stopped, None);
        interpreter.ensure_indicators(world).unwrap();
        world.finish_selection(family.flags).unwrap();
        // Step 16. Nobody is selected, so the saved selection in global 3
        // is cleared. Nothing else changes.
        let mut expected = world.clone();
        assert_eq!(world.globals[3], 21);
        assert_eq!(sims_vm::selected(world), None);
        assert_eq!(interpreter.announce_house(world), Ok(None));
        expected.globals[3] = 0;
        assert_eq!(world.encode(), expected.encode());
        // Steps 17 and 18. The funds are what they were. The family's
        // first member is person 21, who gets the mark and the first
        // indicator.
        interpreter.asked = Some(&asked);
        let sim = interpreter.select_resident(world, family.budget, &family.members);
        assert_eq!(sim, Ok(Some(21)));
        assert_eq!(sims_vm::selected(world), Some(21));
        let calls: Vec<(u16, usize)> = asked
            .borrow()
            .iter()
            .map(|call| (call.object, call.function))
            .collect();
        assert_eq!(calls, [(383, 10), (383, 10), (383, 9), (21, 9)]);
        let before = expected.clone();
        expected.globals[3] = 21;
        expected.objects.get_mut(&21).unwrap().selected = true;
        expected.instances.insert(world.objects[&21].guid, vec![21]);
        // The indicator sits in the Sim's slot 1, at the Sim's place and
        // in its room.
        let indicator = expected.objects.get_mut(&383).unwrap();
        (indicator.data[2], indicator.data[3]) = (21, 1);
        (indicator.data[14], indicator.data[26]) = (2, 21);
        indicator.data[29] = before.objects[&21].data[29];
        indicator.position = before.objects[&21].position;
        indicator.footprint = world.objects[&383].footprint;
        expected.objects.get_mut(&21).unwrap().slots[2].object = 383;
        expected.frame_storage = world.frame_storage.clone();
        assert_eq!(world.encode(), expected.encode());
        asked.borrow_mut().clear();
        // Step 19. Every room waits, global 37 says what the lot has,
        // and the mode is live.
        assert_eq!((world.globals[19], world.globals[37]), (3, 0));
        interpreter.go_live(world).unwrap();
        (expected.globals[19], expected.globals[37]) = (0, 1);
        for room in expected.rooms.rooms.values_mut() {
            room.dirty = true;
        }
        expected.rooms.pending = 3;
        assert_eq!(world.encode(), expected.encode());
        // Steps 21 and 22. All 11 rooms are counted. The six indoor rooms
        // have lost their window light since the count of step 10a (the
        // visits reset the windows), so their light changed and they are
        // marked again; no level waits.
        assert_eq!(world.globals[12], 2009);
        let counted = interpreter.before_frame(world).unwrap();
        assert_eq!(counted.len(), 11);
        let waiting: Vec<i16> = world
            .rooms
            .rooms
            .iter()
            .filter(|(_, room)| room.dirty)
            .map(|(&id, _)| id)
            .collect();
        assert_eq!(waiting, [1, 2, 3, 4, 1025, 1026]);
        assert_eq!(world.rooms.pending, 0);
        for id in waiting {
            let (now, then) = (&world.rooms.rooms[&id], &expected.rooms.rooms[&id]);
            assert_eq!((now.window_light, then.window_light > 0), (0, true));
            assert_ne!(now.light, then.light);
        }
        expected.rooms = world.rooms.clone();
        expected.globals[12] = 0;
        assert_eq!(world.encode(), expected.encode());
        // No tree ran in any of this, the clock stands and no random
        // stream moved.
        assert_eq!(asked.borrow().len(), 0);
        assert_eq!((world.ticks, world.scheduler_cursor), (88444, 0));
        assert_eq!(world.global_rng, before.global_rng);
        assert_eq!(world.lot_rng, before.lot_rng);
        assert_eq!(world.crt_rng, before.crt_rng);
        assert_eq!(world.hash_hex(), BASELINE);
    });
}
