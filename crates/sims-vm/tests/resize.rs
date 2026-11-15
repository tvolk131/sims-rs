//! The lot resize at the start of a load, the clear that follows it, the
//! lot arrays, the type table, the saved objects, the controllers among
//! them, the object grid, the rooms, the household, the object visits and
//! the end of the load, on a real install when `SIMS_INSTALL` names one.
//! Game data is never committed, so CI skips this. The mechanics have
//! synthetic tests in `tests/interpreter`.

use sims_content::{Animations, Catalog, House, Index, Install, Neighborhood, People};
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
    program.expansions = context.expansions;
    program.animations = Animations::load(&install, &index).unwrap();
    // Every person type's skeleton and appearance, for the people a script
    // makes, as `sims boot` loads them.
    program.people = People::load_each(
        &install,
        &index,
        &catalog,
        catalog
            .objects
            .iter()
            .filter(|o| o.kind == 2)
            .map(|o| o.guid),
    )
    .unwrap()
    .0;
    let interpreter = Interpreter::new(&program, &catalog, Stream::Global);
    let mut world = World::empty();
    world
        .read_neighbors(
            &neighborhood,
            &sims_sim::neighbors::registry(&index, &catalog),
        )
        .unwrap();
    world.drop_empty_families().unwrap();
    world.type_attributes = program.type_attributes(&catalog).unwrap();
    world.read_type_attributes(neighborhood.type_attributes.as_ref().unwrap());
    assert_eq!(interpreter.create_indicators(&mut world), Ok([1, 2]));
    world.read_simulator(&house.simulator);
    world.read_house_model(house.model.as_ref().unwrap());
    world.apply_context(context).unwrap();
    let resized = interpreter.resize(&mut world).unwrap();
    assert_eq!(resized.removed, []);
    let created = resized.created;
    let guids = house.objects.instances.values().map(|o| o.guid);
    let people = People::load(&install, &index, &catalog, guids).unwrap();
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

/// A lot's encoding as snapshot format 8 wrote it. Format 9 added two
/// fields at the end of the encoding: the dialog records with the
/// registry's word 0x100, and each person's tick state (the tick of the
/// last decay and the room last seen in). Before the first tick there is
/// no record, the word is 0 and no person has been ticked: an empty list,
/// a zero and an empty map, one zero byte each. This checks that the
/// encoding ends in those three bytes and drops them, so a state pinned
/// under format 8 can be shown to be the same state.
fn format_8_bytes(world: &World) -> Vec<u8> {
    let mut encoded = format_9_bytes(world);
    assert_eq!(encoded.split_off(encoded.len() - 3), [0; 3]);
    encoded
}

/// A lot's encoding as snapshot format 9 wrote it. Format 10 added the
/// neighborhood's records at the end of the encoding. This empties them,
/// checks that the encoding then ends in their three zero bytes (empty
/// lists of slots and inventories and a clear registry byte), and drops those, so a state
/// pinned under format 9 can be shown to be the same state but for the
/// records. The records are compared with the original apart from this.
fn format_9_bytes(world: &World) -> Vec<u8> {
    let mut old = world.clone();
    old.neighbors = sims_sim::neighbors::Neighbors::default();
    let mut encoded = format_10_bytes(&old);
    assert_eq!(encoded.split_off(encoded.len() - 3), [0; 3]);
    encoded
}

/// A lot's encoding as snapshot format 10 wrote it. Format 11 added the
/// persons' animators at the end of the encoding. This takes the format
/// 11 encoding without the animators, checks that it then ends in the
/// zero byte of an empty map, and drops it, so a state pinned under
/// format 10 can be shown to be the same state but for the animators.
fn format_10_bytes(world: &World) -> Vec<u8> {
    let mut old = world.clone();
    old.animators.clear();
    let mut encoded = without_usage(&old);
    assert_eq!(encoded.split_off(encoded.len() - 1), [0]);
    encoded
}

/// A lot's encoding as snapshot format 12 wrote it. Format 13 added two
/// things: the lot's routing at the end of the encoding, and at the end
/// of each animator where it is along a route ([`sims_sim::animator::Motion`]).
/// Before the first route there is neither: the routing encodes to three
/// zero bytes (two empty maps and an empty set) and each animator's
/// motion ([`sims_sim::animator::Motion::NONE`]) to ten bytes, the
/// segment's -1 (1 as postcard writes it) and nine zeros. This checks
/// both and drops them, so a state pinned under format 12 can be shown
/// to be the same state.
fn format_12_bytes(world: &World) -> Vec<u8> {
    assert_eq!(world.routing, sims_sim::route::Routing::default());
    let mut old = world.clone();
    old.animators.clear();
    let mut encoded = format_14_bytes(&old);
    assert_eq!(encoded.split_off(encoded.len() - 3), [0; 3]);
    let usage = postcard::to_allocvec(&world.usage).unwrap();
    assert_eq!(encoded.split_off(encoded.len() - usage.len()), usage);
    assert_eq!(encoded.pop(), Some(0));
    let count = world.animators.len() as u64;
    encoded.extend(postcard::to_allocvec(&count).unwrap());
    for (id, animator) in &world.animators {
        encoded.extend(postcard::to_allocvec(id).unwrap());
        encoded.extend(animator_12_bytes(animator));
    }
    encoded.extend(usage);
    encoded
}

/// `whole` with `part` replaced by `old`. `part` is found where `blank`
/// stands in `whole_blank`, the same encoding with that one field (and
/// nothing else) at its default value: the bytes before and after it are
/// the same in both, and it must be found in exactly one place.
fn splice(whole: &[u8], whole_blank: &[u8], part: &[u8], blank: &[u8], old: &[u8]) -> Vec<u8> {
    if part == old {
        return whole.to_vec();
    }
    let common = |a: &mut dyn Iterator<Item = (&u8, &u8)>| a.take_while(|(x, y)| x == y).count();
    let before = common(&mut whole.iter().zip(whole_blank));
    let after = common(&mut whole.iter().rev().zip(whole_blank.iter().rev()));
    let at: Vec<usize> = (0..=before.min(whole_blank.len() - blank.len()))
        .filter(|&p| {
            whole.get(p..p + part.len()) == Some(part)
                && whole_blank[p..p + blank.len()] == *blank
                && whole.len() - p - part.len() <= after
                && whole_blank.len() - p - blank.len() <= after
        })
        .collect();
    assert_eq!(at.len(), 1, "the field is not found in one place");
    let p = at[0];
    let mut bytes = whole[..p].to_vec();
    bytes.extend(old);
    bytes.extend(&whole[p + part.len()..]);
    bytes
}

/// A map's encoding with each value written by `value`.
fn map_bytes<K: serde::Serialize, V>(
    map: &std::collections::BTreeMap<K, V>,
    value: impl Fn(&V) -> Vec<u8>,
) -> Vec<u8> {
    let mut bytes = postcard::to_allocvec(&(map.len() as u64)).unwrap();
    for (key, item) in map {
        bytes.extend(postcard::to_allocvec(key).unwrap());
        bytes.extend(value(item));
    }
    bytes
}

/// A motion's encoding under snapshot format 16: without the timer
/// ([`sims_sim::animator::Motion::timer`]) that format 17 added at its end.
fn motion_16_bytes(motion: &sims_sim::animator::Motion) -> Vec<u8> {
    let mut bytes = postcard::to_allocvec(motion).unwrap();
    let added = postcard::to_allocvec(&motion.timer).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - added.len()), added);
    bytes
}

/// A motion's encoding under snapshot format 15: also without the last
/// step and the two end-of-path flags that format 16 added at its end.
fn motion_15_bytes(motion: &sims_sim::animator::Motion) -> Vec<u8> {
    let mut bytes = motion_16_bytes(motion);
    let added = postcard::to_allocvec(&(motion.step, motion.near_end, motion.finishing)).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - added.len()), added);
    bytes
}

/// A foreground layer's encoding under snapshot format 16: without the
/// fade ([`sims_sim::animator::Layer::fade`]) that format 17 added at its
/// end.
fn layer_16_bytes(layer: &Option<sims_sim::animator::Layer>) -> Vec<u8> {
    let mut bytes = postcard::to_allocvec(layer).unwrap();
    if let Some(layer) = layer {
        let added = postcard::to_allocvec(&layer.fade).unwrap();
        assert_eq!(bytes.split_off(bytes.len() - added.len()), added);
    }
    bytes
}

/// An animator's encoding under snapshot format 17: without the head
/// fields ([`sims_sim::animator::Animator::head`], animator +0x210 to
/// +0x224) that format 18 added at its end.
fn animator_17_bytes(animator: &sims_sim::animator::Animator) -> Vec<u8> {
    let mut bytes = animator_20_bytes(animator);
    let added = postcard::to_allocvec(&animator.head).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - added.len()), added);
    bytes
}

/// A route context's encoding under snapshot format 18: without the
/// number of the action asking a person to move
/// ([`sims_sim::route::Context::action`], context +0x90) that format 19
/// added at its end.
fn context_18_bytes(context: &sims_sim::route::Context) -> Vec<u8> {
    let mut bytes = postcard::to_allocvec(context).unwrap();
    let added = postcard::to_allocvec(&context.action).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - added.len()), added);
    bytes
}

/// The routing's encoding under snapshot format 18: each context as
/// [`context_18_bytes`] writes it, and without the slot records
/// ([`sims_sim::route::Routing::slots`]) that format 19 added at the end.
fn routing_18_bytes(routing: &sims_sim::route::Routing) -> Vec<u8> {
    let mut bytes = map_bytes(&routing.routes, |route| {
        let mut bytes = postcard::to_allocvec(&(route.contexts.len() as u64)).unwrap();
        for context in &route.contexts {
            bytes.extend(context_18_bytes(context));
        }
        bytes.extend(postcard::to_allocvec(&route.path).unwrap());
        bytes
    });
    bytes.extend(postcard::to_allocvec(&routing.slot_flags).unwrap());
    bytes.extend(postcard::to_allocvec(&routing.unknown_words).unwrap());
    bytes
}

/// The encoding of `world` as snapshot format 27 wrote it: without the
/// poses the load left in the skins ([`World::held_poses`]) that format 28
/// added at the end. The older formats' encodings start from this one.
fn format_27_bytes(world: &World) -> Vec<u8> {
    let mut bytes = world.encode();
    let held = postcard::to_allocvec(&world.held_poses).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - held.len()), held);
    bytes
}

/// A lot's encoding as snapshot format 26 wrote it: without the
/// neighborhood's data words ([`World::neighborhood_data`]) that format 27
/// added at the end. The older formats' encodings start from this one.
fn format_26_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_27_bytes(world);
    let data = postcard::to_allocvec(&world.neighborhood_data).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - data.len()), data);
    bytes
}

/// A lot's encoding as snapshot format 25 wrote it: without the record of
/// the bones the skins moved ([`World::translated_bones`]) that format 26
/// added at the end either. It starts from [`format_26_bytes`].
fn format_25_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_26_bytes(world);
    let moved = postcard::to_allocvec(&world.translated_bones).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - moved.len()), moved);
    bytes
}

/// A lot's encoding as snapshot format 24 wrote it: without the base
/// slots' layers ([`World::base_layers`]) that format 25 added at the end,
/// so a state pinned under format 24 can be shown to be the same state
/// but for those layers. It starts from [`format_25_bytes`], and the older
/// formats' encodings from this one.
fn format_24_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_25_bytes(world);
    let layers = postcard::to_allocvec(&world.base_layers).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - layers.len()), layers);
    bytes
}

/// A lot's encoding as snapshot format 23 wrote it: without the headlines
/// ([`World::headlines`]) that format 24 added at the end, so a state
/// pinned under format 23 can be shown to be the same state but for the
/// headlines. It starts from [`format_24_bytes`], and the older formats'
/// encodings from this one.
fn format_23_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_24_bytes(world);
    let headlines = postcard::to_allocvec(&world.headlines).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - headlines.len()), headlines);
    bytes
}

/// A lot's encoding as snapshot format 22 wrote it: without the families
/// ([`World::families`]) that format 23 added at the end, so a state
/// pinned under format 22 can be shown to be the same state but for the
/// families. It starts from [`format_23_bytes`], and the older formats'
/// encodings from this one.
fn format_22_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_23_bytes(world);
    let families = postcard::to_allocvec(&world.families).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - families.len()), families);
    bytes
}

/// A lot's encoding as snapshot format 21 wrote it: without the
/// advertisement record ([`World::advertisement_record`]) that format 22
/// added at the end, so a state pinned under format 21 can be shown to be
/// the same state but for the record. It starts from [`format_22_bytes`],
/// and the older formats' encodings from this one.
fn format_21_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_22_bytes(world);
    let record = postcard::to_allocvec(&world.advertisement_record).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - record.len()), record);
    bytes
}

/// An animator's encoding under snapshot format 20: without the carry
/// layer ([`sims_sim::animator::Animator::carry`]) and its place in the
/// skin's list ([`sims_sim::animator::Animator::carry_first`]) that
/// format 21 added at its end.
fn animator_20_bytes(animator: &sims_sim::animator::Animator) -> Vec<u8> {
    let mut bytes = postcard::to_allocvec(animator).unwrap();
    let added = postcard::to_allocvec(&(&animator.carry, animator.carry_first)).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - added.len()), added);
    bytes
}

/// A lot's encoding as snapshot format 20 wrote it: each animator as
/// [`animator_20_bytes`] writes it, so a state pinned under format 20 can
/// be shown to be the same state but for the carry layer. It starts from
/// [`format_21_bytes`], and the older formats' encodings from this one.
fn format_20_bytes(world: &World) -> Vec<u8> {
    let blank = World {
        animators: Default::default(),
        ..world.clone()
    };
    splice(
        &format_21_bytes(world),
        &format_21_bytes(&blank),
        &postcard::to_allocvec(&world.animators).unwrap(),
        &[0],
        &map_bytes(&world.animators, animator_20_bytes),
    )
}

/// A lot's encoding as snapshot format 19 wrote it. Format 20 added,
/// after the registration order, the person vector
/// ([`World::people`]). This writes the lot again without it, so a state
/// pinned under format 19 can be shown to be the same state but for that
/// field. It starts from [`format_20_bytes`], and the older formats'
/// encodings from this one.
fn format_19_bytes(world: &World) -> Vec<u8> {
    // A marker vector stands where the person vector does in the same
    // lot's encoding, so the field is found in one place.
    let marker = vec![0xbeef, 0xbeef, 0xbeef];
    let blank = World {
        people: marker.clone(),
        ..world.clone()
    };
    splice(
        &format_20_bytes(world),
        &format_20_bytes(&blank),
        &postcard::to_allocvec(&world.people).unwrap(),
        &postcard::to_allocvec(&marker).unwrap(),
        &[],
    )
}

/// A lot's encoding as snapshot format 18 wrote it. Format 19 added at
/// the end of each route context the number of the action that asks a
/// person to move out of the way, and at the end of the routing the slot
/// records objects hold in place of their type's. This writes the
/// routing again without them, so a state pinned under format 18 can be
/// shown to be the same state but for those fields. The older formats'
/// encodings start from this one.
fn format_18_bytes(world: &World) -> Vec<u8> {
    // The routing is found where a marker routing stands in the same
    // lot's encoding: an empty routing's bytes can stand in many places.
    let marker = sims_sim::route::Routing {
        unknown_words: [(0xbeef, 0xdead_beef)].into(),
        ..Default::default()
    };
    let blank = World {
        routing: marker.clone(),
        ..world.clone()
    };
    splice(
        &format_19_bytes(world),
        &format_19_bytes(&blank),
        &postcard::to_allocvec(&world.routing).unwrap(),
        &postcard::to_allocvec(&marker).unwrap(),
        &routing_18_bytes(&world.routing),
    )
}

/// A lot's encoding as snapshot format 17 wrote it. Format 18 added at
/// the end of each animator the head's angles, turn and target
/// ([`sims_sim::animator::Animator::head`]). This writes the animators
/// again without them, so a state pinned under format 17 can be shown to
/// be the same state but for those fields.
fn format_17_bytes(world: &World) -> Vec<u8> {
    let blank = World {
        animators: Default::default(),
        ..world.clone()
    };
    splice(
        &format_18_bytes(world),
        &format_18_bytes(&blank),
        &map_bytes(&world.animators, animator_20_bytes),
        &[0],
        &map_bytes(&world.animators, animator_17_bytes),
    )
}

/// An animator's encoding with the head left out ([`animator_17_bytes`]),
/// its motion written by `motion` and its foreground layer by
/// [`layer_16_bytes`].
fn animator_old_bytes(
    animator: &sims_sim::animator::Animator,
    motion: fn(&sims_sim::animator::Motion) -> Vec<u8>,
) -> Vec<u8> {
    use sims_sim::animator::{Animator, Motion};
    let with_motion = |a: &Animator| {
        let blank = Animator {
            motion: Motion::NONE,
            ..a.clone()
        };
        splice(
            &animator_17_bytes(a),
            &animator_17_bytes(&blank),
            &postcard::to_allocvec(&a.motion).unwrap(),
            &postcard::to_allocvec(&Motion::NONE).unwrap(),
            &motion(&a.motion),
        )
    };
    let blank = Animator {
        foreground: None,
        ..animator.clone()
    };
    splice(
        &with_motion(animator),
        &with_motion(&blank),
        &postcard::to_allocvec(&animator.foreground).unwrap(),
        &[0],
        &layer_16_bytes(&animator.foreground),
    )
}

/// An animator's encoding under snapshot format 16 ([`motion_16_bytes`],
/// [`layer_16_bytes`]).
fn animator_16_bytes(animator: &sims_sim::animator::Animator) -> Vec<u8> {
    animator_old_bytes(animator, motion_16_bytes)
}

/// An animator's encoding under snapshot format 15, with its motion as
/// [`motion_15_bytes`] writes it.
fn animator_15_bytes(animator: &sims_sim::animator::Animator) -> Vec<u8> {
    animator_old_bytes(animator, motion_15_bytes)
}

/// A lot's encoding as snapshot format 16 wrote it. Format 17 added two
/// things: at the end of each animator's motion the timer of a walk that
/// slows to a stop ([`sims_sim::animator::Motion::timer`], animator
/// +0x1c0), and at the end of each foreground layer its fade
/// ([`sims_sim::animator::Layer::fade`], layer +0x48 to +0x54). This
/// writes the animators again without those bytes, so a state pinned
/// under format 16 can be shown to be the same state but for those
/// fields.
fn format_16_bytes(world: &World) -> Vec<u8> {
    let blank = World {
        animators: Default::default(),
        ..world.clone()
    };
    splice(
        &format_18_bytes(world),
        &format_18_bytes(&blank),
        &map_bytes(&world.animators, animator_20_bytes),
        &[0],
        &map_bytes(&world.animators, animator_16_bytes),
    )
}

/// A lot's encoding with each animator as format 15 wrote it.
fn without_motion_steps(world: &World) -> Vec<u8> {
    let blank = World {
        animators: Default::default(),
        ..world.clone()
    };
    splice(
        &format_18_bytes(world),
        &format_18_bytes(&blank),
        &map_bytes(&world.animators, animator_20_bytes),
        &[0],
        &map_bytes(&world.animators, animator_15_bytes),
    )
}

/// A lot's encoding as snapshot format 15 wrote it. Format 16 added two
/// things: at the end of each room whether it knows a person is in it
/// ([`sims_sim::rooms::Room::occupied`]; the people's visits in the load
/// now set it), and at the end of each animator's motion its last step
/// and two end-of-path flags ([`sims_sim::animator::Motion`]). This
/// writes the rooms and the animators again without those bytes, so a
/// state pinned under format 15 can be shown to be the same state but for
/// those fields.
fn format_15_bytes(world: &World) -> Vec<u8> {
    use sims_sim::rooms::Rooms;
    let rooms = &world.rooms;
    let blank_rooms = Rooms {
        rooms: Default::default(),
        ..rooms.clone()
    };
    let old_rooms = splice(
        &postcard::to_allocvec(rooms).unwrap(),
        &postcard::to_allocvec(&blank_rooms).unwrap(),
        &postcard::to_allocvec(&rooms.rooms).unwrap(),
        &[0],
        &map_bytes(&rooms.rooms, |room| {
            let mut bytes = postcard::to_allocvec(room).unwrap();
            assert_eq!(bytes.pop(), Some(u8::from(room.occupied)));
            bytes
        }),
    );
    let blank = World {
        rooms: Rooms::default(),
        ..world.clone()
    };
    splice(
        &without_motion_steps(world),
        &without_motion_steps(&blank),
        &postcard::to_allocvec(rooms).unwrap(),
        &postcard::to_allocvec(&Rooms::default()).unwrap(),
        &old_rooms,
    )
}

/// A lot's encoding as snapshot format 14 wrote it. Format 15 added the
/// last number given to a running action ([`World::action_serial`]) at
/// the end. Before tick 233 no action has been given one, so it is 0 and
/// encodes to one zero byte; this checks that and drops it, so a state
/// pinned under format 14 can be shown to be the same state.
fn format_14_bytes(world: &World) -> Vec<u8> {
    assert_eq!(world.action_serial, 0);
    let mut encoded = format_15_bytes(world);
    assert_eq!(encoded.pop(), Some(0));
    encoded
}

/// A lot's encoding as snapshot format 13 wrote it. Format 14 added three
/// fields: at the end of each animator whether the house view has drawn
/// its person yet ([`sims_sim::animator::Animator::undrawn`]) and whether
/// it walks ([`sims_sim::animator::Animator::walking`]), and at the end of
/// each layer where its clip's timed events stand
/// ([`sims_sim::animator::Layer::cursors`]; one zero byte when there are
/// none). The animators come last but for the usage counts and the
/// routing. This writes them again without those bytes
/// ([`animator_13_bytes`]), so a state pinned under format 13 can be shown
/// to be the same state but for those fields.
fn format_13_bytes(world: &World) -> Vec<u8> {
    let mut old = world.clone();
    old.animators.clear();
    let mut encoded = format_14_bytes(&old);
    let mut tail = postcard::to_allocvec(&world.usage).unwrap();
    tail.extend(routing_18_bytes(&world.routing));
    assert_eq!(encoded.split_off(encoded.len() - tail.len()), tail);
    assert_eq!(encoded.pop(), Some(0));
    let count = world.animators.len() as u64;
    encoded.extend(postcard::to_allocvec(&count).unwrap());
    for (id, animator) in &world.animators {
        encoded.extend(postcard::to_allocvec(id).unwrap());
        encoded.extend(animator_13_bytes(animator));
    }
    encoded.extend(tail);
    encoded
}

/// An animator's encoding under snapshot format 13 ([`format_13_bytes`]):
/// without its last two bytes, `undrawn` and `walking`, and without its
/// foreground layer's cursors, the layer's last field. The layer is
/// followed by the skin's clock and flag, the appearance flag and the
/// motion.
fn animator_13_bytes(animator: &sims_sim::animator::Animator) -> Vec<u8> {
    let mut bytes = animator_15_bytes(animator);
    assert_eq!(bytes.pop(), Some(u8::from(animator.walking)));
    assert_eq!(bytes.pop(), Some(u8::from(animator.undrawn)));
    let mut after = postcard::to_allocvec(&(
        animator.skin_clock,
        animator.skin_dirty,
        animator.profile_stale,
    ))
    .unwrap();
    after.extend(motion_15_bytes(&animator.motion));
    assert_eq!(bytes.split_off(bytes.len() - after.len()), after);
    if let Some(layer) = &animator.foreground {
        let cursors = postcard::to_allocvec(&layer.cursors).unwrap();
        assert_eq!(bytes.split_off(bytes.len() - cursors.len()), cursors);
    }
    bytes.extend(after);
    bytes
}

/// The state as it was before the load kept each person's saved head
/// target (animator +0x21c, 0x5918ba): every target still the one the
/// load restored in `loaded` is cleared, as no measurement has written it,
/// and an animator then equal to [`sims_sim::animator::LOADED`] is dropped,
/// as the load did not keep one. A state pinned before that change can be
/// shown to be the same state but for those targets.
fn without_saved_targets(world: &World, loaded: &World) -> World {
    let mut old = world.clone();
    for (id, animator) in old.animators.iter_mut() {
        let saved = loaded.animators.get(id).and_then(|a| a.head.target);
        if saved.is_some() && animator.head.target == saved {
            animator.head.target = None;
        }
    }
    old.animators
        .retain(|_, animator| *animator != sims_sim::animator::LOADED);
    old
}

/// The lot without the rooms its people's visits noted. Each person's
/// visit now notes the room it stands in, as the original's does
/// (0x45e43d, called at 0x459c22): its tick state
/// ([`sims_sim::PersonTick`]) gets the room, and the room is marked as
/// knowing someone is in it. Before, a person's first tick did that. This
/// checks that each person's tick state is the constructed one with the
/// room its data word 29 names, and removes them, so a state pinned
/// before that change can be shown to be the same state but for those
/// fields (the rooms' marks are dropped by [`format_15_bytes`]).
fn without_room_notes(world: &World) -> World {
    use sims_sim::PersonTick;
    let mut old = world.clone();
    for (id, tick) in std::mem::take(&mut old.person_ticks) {
        let room = world.objects[&id].data[29];
        let expected = PersonTick {
            room,
            ..PersonTick::CONSTRUCTED
        };
        assert_eq!(tick, expected, "person {id}");
    }
    old
}

/// The lot with registry byte 0x12
/// ([`sims_sim::neighbors::Neighbors::relationships_changed`]) clear. The
/// end of the object read now sets it ([`World::finish_object_read`]), as
/// a hardware watchpoint on the original showed. Before, it stayed clear
/// until a script wrote a relationship, which on the Goth house is in
/// tick 2. The states pinned before that, at the load's end and after
/// tick 1, were pinned with it clear; this checks it is set and clears
/// it, so they can be shown to be the same state but for that byte.
fn with_read_flag_clear(world: &World) -> World {
    assert!(world.neighbors.relationships_changed);
    let mut old = world.clone();
    old.neighbors.relationships_changed = false;
    old
}

/// An animator's encoding under snapshot format 12: without its motion,
/// which must be that of no route ([`format_12_bytes`]).
fn animator_12_bytes(animator: &sims_sim::animator::Animator) -> Vec<u8> {
    use sims_sim::animator::Motion;
    assert_eq!(animator.motion, Motion::NONE);
    let motion = motion_15_bytes(&Motion::NONE);
    assert_eq!(motion, [1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let mut bytes = animator_13_bytes(animator);
    assert_eq!(bytes.split_off(bytes.len() - motion.len()), motion);
    bytes
}

/// A lot's encoding without the house's usage counts, which snapshot
/// format 12 added at its end: twelve numbers, one zero byte each once
/// they are cleared. This clears them, checks the twelve zero bytes and
/// drops them, from the format 12 encoding ([`format_12_bytes`]).
fn without_usage(world: &World) -> Vec<u8> {
    let mut old = world.clone();
    old.usage = sims_sim::HouseUsage::default();
    let mut encoded = format_12_bytes(&old);
    assert_eq!(encoded.split_off(encoded.len() - 12), [0; 12]);
    encoded
}

/// A lot's encoding as snapshot format 11 wrote it. Format 12 added two
/// things: the house's usage counts at the end of the encoding
/// ([`without_usage`]), and at the end of each animator its flag that the
/// person's appearance is to be read again. This writes the animators
/// again, each without its last byte, which is that flag. So a state
/// pinned under format 11 can be shown to be the same state but for those
/// fields.
fn format_11_bytes(world: &World) -> Vec<u8> {
    let mut old = world.clone();
    old.animators.clear();
    let mut encoded = without_usage(&old);
    assert_eq!(encoded.split_off(encoded.len() - 1), [0]);
    let count = world.animators.len() as u64;
    encoded.extend(postcard::to_allocvec(&count).unwrap());
    for (id, animator) in &world.animators {
        encoded.extend(postcard::to_allocvec(id).unwrap());
        let mut bytes = animator_12_bytes(animator);
        assert_eq!(bytes.pop(), Some(u8::from(animator.profile_stale)));
        encoded.extend(bytes);
    }
    encoded
}

/// Checks a state against the digest pinned for it under snapshot format
/// 8 ([`format_8_bytes`]) and under format 7 ([`format_7_bytes`]). Its
/// format 9 encoding is the format 8 one and three zero bytes.
///
/// Every pin in this file that holds a script's frames changed once with
/// format 10, for one reason: the original writes a return's target (0xfe
/// or 0xff) into the frame's node word before it pops the frame (0x474cea,
/// 0x474d1b), so the frame storage keeps that, not the node the frame
/// last ran. Before the pins were changed, each pinned state was dumped
/// under the old and the new interpreter and compared outside the
/// repository: everything but the frame storage was equal, the frame
/// storage's frame starts and unknown words were equal, and every word
/// that differed sits at a frame's node word (one past a multiple of
/// four) and now holds 0xfe or 0xff where it held a node of at most 250.
/// A later frame's local can land on such a word and start with it; the
/// capture of the Goth house's second tick shows one doing so.
fn pinned(world: &World, format_8: &str, format_7: &str) {
    assert_eq!(digest(&format_8_bytes(world)), format_8);
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
    const PINNED: &str = "819756c776e4f4517e5b43c2c81ecb448815c9bcaebee79209964b873526119b";
    const PINNED_7: &str = "041741b9964e5afb8d11370b056cadbc9fd1a573bfd0ab4dc219d194a5e6c411";

    const FORMAT_6: &str = "0bcd63b82873b11b2fdc9a5c2e964410b2ea2afa50bf2abe09ab456b092f3c92";
    const FORMAT_5: &str = "fc89c0a0ccde594eb06675a096f685d76a8dd69ec0dd18d2126dcdce7e7fa7ce";
    const FORMAT_4: &str = "72c502a09a31deb7fa65e1904a8e339265d239710cf1253b1f5f5a2d0062a229";
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
                .renumber_surfaces(
                    house.floor_map.as_deref(),
                    house.wall_map.as_deref(),
                    &Default::default(),
                )
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
            world
                .renumber_surfaces(Some(&[]), Some(&[]), &Default::default())
                .unwrap();
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
/// One thing differed: in one frame of object 19 the capture's tree ID
/// has bit 15 set (0x8118) where the save, and this state, have 0x118.
/// Object 17 has the same type and thread and no such bit. The frame
/// reader (0x474694) writes only the low 15 bits and keeps bit 15 from the
/// frame arena's memory, which is freshly allocated and not cleared
/// (0x4750ed), and every reader masks the bit off (0x4746c0, 0x4747ea,
/// 0x474f79), so it is left over from the heap and nothing reads it.
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
                .renumber_surfaces(
                    house.floor_map.as_deref(),
                    house.wall_map.as_deref(),
                    &Default::default(),
                )
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

/// Reading a person's interactions raises the action serial to the
/// largest number among them, compared signed (0x41f320): each person's
/// active, last and queued interactions. The Goth people's are all 0;
/// here some are given others.
#[test]
fn reading_the_people_raises_the_action_serial() {
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
                .renumber_surfaces(
                    house.floor_map.as_deref(),
                    house.wall_map.as_deref(),
                    &Default::default(),
                )
                .unwrap();
            interpreter.read_types(world, &house.types).unwrap();
            interpreter.clear(world).unwrap();
            let before = world.clone();
            let mut objects = house.objects.clone();
            let mut persons = objects
                .instances
                .values_mut()
                .filter_map(|i| i.person.as_mut());
            persons.next().unwrap().last.uid = 7;
            persons.next().unwrap().active.uid = -9;
            interpreter.read_objects(world, &objects, people).unwrap();
            assert_eq!(world.action_serial, 7);
            // A larger serial stays.
            *world = before.clone();
            world.action_serial = 8;
            interpreter.read_objects(world, &objects, people).unwrap();
            assert_eq!(world.action_serial, 8);
            // A queued interaction counts too.
            *world = before;
            let third = objects
                .instances
                .values_mut()
                .filter_map(|i| i.person.as_mut())
                .nth(2)
                .unwrap();
            let mut queued = third.active.clone();
            queued.uid = 12;
            third.queue.push(queued);
            interpreter.read_objects(world, &objects, people).unwrap();
            assert_eq!(world.action_serial, 12);
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
/// One thing differed, as after the object read: object 19's second frame
/// has tree ID 0x8118 in the trace and 0x118 here, a bit left over from the
/// heap that nothing reads (see the object read's test).
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
    const PINNED: &str = "bddc0670e658fd31dcb223e119126b69d2c2cc2d8859a285c827bbc68665373d";
    const PINNED_7: &str = "96194b11e2e07c4f2b3f438ec82999072e335676485a077e54f0820301ca2de1";

    const FORMAT_6: &str = "bc1dff5b1eb81b404163ceafe2d880abb2d796c53261c4224a04ec4048938cb8";
    const FORMAT_5: &str = "eb2602d277834de8cca39610d6d0e19425493d7973757ec009fde8bdbe23e02a";
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
                .renumber_surfaces(
                    house.floor_map.as_deref(),
                    house.wall_map.as_deref(),
                    &Default::default(),
                )
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
    const COUNTED: &str = "32e87085a0f2b6f4118ec0c252c2d60244fe59b147b1df0313076509c0e940d1";
    const COUNTED_7: &str = "97538352ea5c303dcbde81685a3c8b3bd769c4f7c76f5cbcaae261556dae1518";
    const SELECTED: &str = "df7beddf55ca287f181abd48574a8f72fb55a5f0d07c9ff8939e345b13175ac9";
    const SELECTED_7: &str = "b8926c8537b9d368b280b4cf630b16eb0961f7fc787d81bffcedd1f316e282e5";
    const COUNTED_6: &str = "50725dcfbbbd1b6aebcd72598e419cc8ae9e315546cd6f7f0d6a7cd3c97a7734";
    const SELECTED_6: &str = "961a16756c0f8b3d729e3d7a2d01a901a24fe1d9689f65d1a7f087ed1a6907bc";
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
            .renumber_surfaces(
                house.floor_map.as_deref(),
                house.wall_map.as_deref(),
                &Default::default(),
            )
            .unwrap();
        interpreter.read_types(world, &house.types).unwrap();
        interpreter.clear(world).unwrap();
        interpreter
            .read_objects(world, &house.objects, people)
            .unwrap();
        interpreter.reconcile_controllers(world).unwrap();
        world.repair_grid().unwrap();
        world.finish_object_read();
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
            .renumber_surfaces(
                house.floor_map.as_deref(),
                house.wall_map.as_deref(),
                &Default::default(),
            )
            .unwrap();
        interpreter.read_types(world, &house.types).unwrap();
        interpreter.clear(world).unwrap();
        interpreter
            .read_objects(world, &house.objects, people)
            .unwrap();
        interpreter.reconcile_controllers(world).unwrap();
        world.repair_grid().unwrap();
        world.finish_object_read();
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
/// Load step 15 on the Goth lot (`World::finish_selection`). Its people
/// are all of the family and the lot (house 5) is zoned residential, so
/// which types are
/// the neighborhood's characters does not matter, and the residents noted
/// before the selection are the people on the lot now.
fn finish_goth_selection(world: &mut World, family: &sims_formats::neighborhood::Family) {
    let residents = world.residents();
    world
        .finish_selection(&sims_sim::load::Household {
            house: 5,
            flags: family.flags,
            members: &family.members,
            characters: &std::collections::BTreeSet::new(),
            residents: &residents,
            zoning: &std::collections::BTreeMap::from([(5, 1)]),
        })
        .unwrap();
}

#[test]
fn the_goth_house_objects_are_all_visited() {
    const VISIT_201: &str = "680e6caf3382eff8f069b64920efdc6bd49de3c7458733f9df65076cdf22e02d";
    const VISIT_201_7: &str = "c8dee4827001dd91ab1a41482c0b1b0ce166a4bb005408d06269e5b1f55323d6";
    const VISITED: &str = "2558d1d97d6c006a495e185a86dda47d57891cb433bf40b909b1b6e891327521";
    const VISITED_7: &str = "d923a23687b3904c8427bc847433284509ac0becccf0f33b67cc6297bd99695c";
    const VISITED_16: &str = "4ebbd61a9def06bc235ed33db27fb6c62a2dcd2b549443fb6c0b89fe945403cd";
    const ROOMS: [(u16, i16); 3] = [(16, 0), (18, 0), (21, 0)];
    const OCCUPIED: [i16; 0] = [];
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
            [(0, 578), (2, 379), (3, 26), (6, 4), (9, 404), (10, 397)]
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
        // Each person noted the room it stands in.
        let rooms: Vec<(u16, i16)> = world
            .person_ticks
            .iter()
            .map(|(&id, tick)| (id, tick.room))
            .collect();
        assert_eq!(rooms, ROOMS);
        let occupied: Vec<i16> = world
            .rooms
            .rooms
            .iter()
            .filter(|(_, room)| room.occupied)
            .map(|(&id, _)| id)
            .collect();
        assert_eq!(occupied, OCCUPIED);
        let old = without_saved_targets(world, &before);
        assert_eq!(digest(&format_16_bytes(&old)), VISITED_16);
        pinned(&without_room_notes(&old), VISITED, VISITED_7);
        // Steps 14 and 15 change nothing on this lot, and neither does
        // asking for the indicators again in step 16.
        assert_eq!(interpreter.ensure_indicators(world), Ok(None));
        finish_goth_selection(world, &family);
        assert_eq!(interpreter.ensure_indicators(world), Ok(None));
        let old = without_saved_targets(world, &before);
        assert_eq!(digest(&format_16_bytes(&old)), VISITED_16);
        pinned(&without_room_notes(&old), VISITED, VISITED_7);
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
    // Pinned under snapshot format 8, the same state under format 9
    // ([`format_9_bytes`], without the neighborhood's records) and under
    // format 10 ([`format_10_bytes`], without the animators, of which the
    // load leaves none), and under formats 11, 12 and 13, the last three
    // with registry byte 0x12 clear ([`with_read_flag_clear`]), and
    // under format 14. All of those are without the rooms the people's
    // visits noted ([`without_room_notes`]); format 16 pins the state as
    // it is. See [`pinned`] for why the format 8 pin changed.
    const BASELINE: &str = "034d3b636d3529fa91826a5f5b759e4abba31d6c5be3b66732a1f1ef642ad9fb";
    const BASELINE_9: &str = "fe21f960b207ba0ced6a6a5dfe5628fcb101c12eb9571d49578ec65dfde459da";
    const BASELINE_10: &str = "29b4aec3debe353d77363e18cf3cbd263b30fe2fe4e1e03a5f309c08dd132168";
    const BASELINE_11: &str = "90c52a0d5be39db0556c0f8fc9ea41088cdb36b5d024094b8073eaf1d3bb918a";
    const BASELINE_12: &str = "9954409afc678bcf8d6815446a86eadeddb46bf5b2785e5461f7b06af38d1687";
    const BASELINE_13: &str = "f844587de2d116d798645aea70ea5ea5f91c49c398c93fc414c3ab31657be32a";
    const BASELINE_14: &str = "45642fa774010ff05a65fbc1d9b23a0edb1af70884a81215495d6380bd599bb3";
    const BASELINE_16: &str = "efdbb4764476cf9282889e94f52b91a337ddaf474a5b0cf8383bc636b4ec1e9d";
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
        // The people's animators as the load's read of the objects left
        // them, with their saved head targets.
        let loaded = world.clone();
        assert_eq!(interpreter.visit_objects(world, people).stopped, None);
        interpreter.ensure_indicators(world).unwrap();
        finish_goth_selection(world, &family);
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
        // Step 20. The Goth family's friend count is worked out: 0. In
        // the capture of the original at the first tick (whose friendship
        // threshold, 0x6c4398, is 50) the families are these, and only
        // the Goth family's count is up to date.
        assert_eq!(
            interpreter
                .program
                .autonomy_constants()
                .friendship_threshold,
            50
        );
        let goths = expected.families.iter_mut().find(|f| f.id == 5).unwrap();
        (goths.friends, goths.stale) = (0, false);
        let families: Vec<(i32, i32, usize, i32, bool)> = world
            .families
            .iter()
            .map(|f| (f.id, f.house, f.members.len(), f.friends, f.stale))
            .collect();
        let captured = [
            (4000, 0, 8),
            (6, 56, 5),
            (10, 76, 5),
            (12, 77, 4),
            (13, 69, 2),
            (11, 63, 4),
            (4, 0, 4),
            (3, 0, 2),
            (2, 0, 1),
            (5, 5, 3),
            (1, 7, 2),
            (6000, 0, 30),
            (5500, 0, 18),
            (5000, 0, 12),
            (3009, 0, 4),
            (3008, 0, 2),
            (3007, 0, 4),
            (3006, 0, 4),
            (3005, 0, 2),
            (3004, 0, 3),
            (3003, 0, 4),
            (3002, 0, 4),
            (3001, 0, 3),
            (3000, 0, 4),
            (2000, 0, 30),
            (0, 0, 0),
        ];
        let captured: Vec<(i32, i32, usize, i32, bool)> = captured
            .into_iter()
            .map(|(id, house, members)| (id, house, members, 0, id != 5))
            .collect();
        assert_eq!(families, captured);
        assert_eq!(world.encode(), expected.encode());
        // No tree ran in any of this, the clock stands and no random
        // stream moved.
        assert_eq!(asked.borrow().len(), 0);
        assert_eq!((world.ticks, world.scheduler_cursor), (88444, 0));
        assert_eq!(world.global_rng, before.global_rng);
        assert_eq!(world.lot_rng, before.lot_rng);
        assert_eq!(world.crt_rng, before.crt_rng);
        let untargeted = without_saved_targets(world, &loaded);
        assert_eq!(digest(&format_16_bytes(&untargeted)), BASELINE_16);
        let noted = without_room_notes(&untargeted);
        assert_eq!(digest(&format_8_bytes(&noted)), BASELINE);
        assert_eq!(digest(&format_9_bytes(&noted)), BASELINE_9);
        assert!(untargeted.animators.is_empty());
        let old = with_read_flag_clear(&noted);
        assert_eq!(digest(&format_10_bytes(&old)), BASELINE_10);
        assert_eq!(digest(&format_11_bytes(&old)), BASELINE_11);
        assert_eq!(digest(&format_12_bytes(&old)), BASELINE_12);
        assert_eq!(digest(&format_13_bytes(&old)), BASELINE_13);
        assert_eq!(digest(&format_14_bytes(&noted)), BASELINE_14);
    });
}

/// The first 261 ticks on the Goth house, and tick 262 up to where it
/// stops: the lot update (0x46d345) run from the state the load ends in,
/// with the house view's frames between updates as far as they write the
/// simulation's state (`frame.rs`). The frames come before tick 1, after
/// each of ticks 1 to 3, after tick 33 and after every 30th tick from
/// there (63, 93, ...). That schedule is an input, not something the
/// simulation decides (`sims boot --frames`, `docs/decisions.md`).
///
/// Four private captures of the original on the same install and house,
/// outside the repository (`docs/decisions.md`), each read the install's
/// own neighborhood file. Each holds one line per lot update in
/// `updates.jsonl` (the tick count, the lot's and the game's random
/// streams, the C runtime's random stream and the scheduler cursor) and
/// the full state after some updates:
///
/// - the first: after updates 1 to 100 and every tenth to 1000;
/// - the second: after updates 1 to 5, 30 to 35 and 225 to 240;
/// - the third: after updates 1 to 100 and every tenth to 300;
/// - the fourth: after every update from 1.
///
/// The last three also recorded when the house view drew (a hardware
/// watchpoint on its draw guard, 0x6f0f48). All three drew before update
/// 1, after updates 1, 2 and 3 (many times while a dialog waited; how many
/// times differs from run to run), after update 33 and after every 30th
/// update from there, which is the schedule used here.
///
/// Our state was compared with each capture's after every tick where the
/// capture holds one, field by field, floats bit for bit: the
/// `updates.jsonl` line and that the update advanced the tick count by
/// one; the tick count, the three random streams, the scheduler cursor,
/// the funds, all 64 globals, the action counter, the removal queue, the
/// registration order and the set of objects; the neighborhood's 463
/// records (ID, GUID, revision, data, relationships, friends, stale
/// mark), the inventories and registry byte 0x12; for each object its
/// GUID, 72 data words, temps, attributes, scheduler word, last result,
/// frames (tree, node, stack object, arguments, locals, saved primitive
/// state), relationships, slot count, position and its removal and
/// selected marks, and that no object of the capture holds a tree error;
/// for each person also the 256 person data words, the motives and
/// previous motives, the running and last actions, the queued actions and
/// the objects in use (object, stack depth, flag); the people's
/// animators, every field kept here that the capture's memory pages hold;
/// and the people's route contexts and paths where the capture saved
/// those pages. Results:
///
/// - The fourth capture: equal after every one of ticks 1 to 261.
/// - The second: equal after every tick it holds (1 to 5, 30 to 35, 225
///   to 240).
/// - The third: equal but for two fields, which also differ between the
///   original's own runs: global 12 after ticks 1 and 2 (17 there, 0 here
///   and in the second and fourth captures; the interface's wait check)
///   and one local of object 18's seventh frame after ticks 210 and 220
///   (16104 there, 0 here and in the fourth).
/// - The first: equal after every tick it holds to 230, and in the
///   `updates.jsonl` line to 232, but for global 12 after tick 1 (17
///   there). It does not choose in tick 233 and differs from there on;
///   see below.
///
/// Not compared: a frame local that we hold as unknown, because the
/// original left half of a pointer in the frame storage there; person
/// 259's word 37 from tick 14 on, which the original copies from memory
/// it never wrote (`sims_sim::route::Routing::unknown_words`); a route
/// context's word +0x74, which the original leaves as the heap held it
/// (0 here); and the context's door, which the original holds as a
/// pointer and we hold as the object's ID (they were compared by mapping
/// the pointer to its object). Also not compared, because no capture holds
/// them in a form read there: the rooms, the lot arrays, the dialog
/// records, the persons' tick state other than the animators, and the
/// house's usage counts.
///
/// In tick 6 object 382 makes a new person, 259 (Create New Object
/// Instance mode 8), who runs from tick 7 on. In tick 14 it goes to slot
/// 0 of object 51 (Go To Routing Slot) and turns on the spot (animator
/// state 4). The frame after tick 33 draws it for the first time, which
/// places its animator again, facing its object's direction word
/// (0x584895, in the house view's draw). In tick 47 it starts to walk
/// (state 9). In tick 24 person 18's clip ends (0x589c25) and it starts
/// the next; from tick 46 on, clips' timed events run (0x5912fd). In
/// tick 36 the tick count is a multiple of 40 and the update clears
/// registry byte 0x12 (0x442521). Every frame counts the rooms that wait
/// to be counted (0x4a8895, `draw_frame`): the frame after tick 2 counts
/// rooms 1 to 4, which a script marked in that tick.
///
/// In tick 233 person 21 runs Find Best Interaction (primitive 3), which
/// chooses menu 6 of object 132 and draws once from the lot's random
/// stream, and Gosub Found Action (primitive 30), which starts it. The
/// interaction's tree goes to the object (Go To Relative Position), which
/// is in another room: the routing between rooms (0x45c7c2, 0x46390f)
/// chooses door 51 and numbers the person and the doors (object data
/// word 6), and the person runs the door's trees, which walk it to the
/// door and through it. In tick 246 it stands at the door (state 3, arrival
/// 2; standing clears the walking flag, 0x58f117). In tick 255 it enters
/// room 4, which is dark and has no lamp: the room is marked as knowing
/// someone is in it (0x468802) and waits to be counted again. In tick 261
/// it stands at the end of that walk.
///
/// In tick 262 person 21's animator comes to the last point of its next
/// path with object data word 9 clear (0x58e8be): its foreground clip may
/// not be interrupted and has not finished, so it only marks the motion as
/// near its end and walks on. The walk's strides (animator +0x17c and
/// +0x178), which that code reads, are worked out by posing the person's
/// skeleton with the walk clip at the clip's two footfalls, as the
/// person's constructor does (0x58831d); they equal the stored values in
/// the fourth capture's updates 1, 300 and 600. In tick 263 it turns on the
/// spot. In tick 296, at the end of its next path and within a stride of
/// it, it sets a timer to stop (motion timer, 0x58e8be) and fades the walk
/// clip's weight; in tick 297 the timer stops it and it turns, and in
/// tick 321 it stands. In tick 322 it plays a clip from its own table
/// (Animate Sim, source 0).
///
/// In tick 334 person 16 looks towards person 259 (Look Towards, mode 0),
/// and its head follows the target (0x5907c5): the headings are measured
/// from the bones' places in the current poses (0x5904d2). In tick 335
/// person 18 starts a walk with clip 0x13, which carries it one clip's
/// distance straight ahead (animator state 6, 0x58e125). From tick 336 on
/// Find Best Interaction also scores the entry that joins person 18's
/// running interaction (0x41ee99). In tick 353 object 282 runs a tree on
/// person 259's own thread to its end (Run Tree By Name, mode 1). From
/// tick 356 on person 259 stands on sloped tiles, whose height comes from
/// the tile's corners (0x478dfe).
///
/// In tick 367 person 18's route to a slot finds no way, because person
/// 21 is in it. Step 6 of the route tries again, finds person 21 in the
/// way (0x45ba28) and asks it to move (0x45bf2e, 0x45d20b), and step 14
/// plays the asking tree. In tick 368 person 21 puts on a book
/// (Change Suit/Accessory, opcode 6). In tick 409 person 18's route
/// plays its waiting tree, which looks round the people in the person
/// vector, where person 259 comes last. Our state after each of ticks 1
/// to 416 equals the fourth capture (full state after every update), as
/// compared outside this test, but for one field: person 16's head target
/// height (+0x220) is one unit in the last place lower on some ticks from
/// 335 on (and its head turn a few units with it), for a reason that is
/// **unknown** (the bones' pages are not captured). Tick 417 stops
/// loudly: person 21 runs Find Best Object for Function (opcode 14),
/// which is not ported.
///
/// The first capture does not choose in tick 233: its one draw from the
/// lot's stream in ticks 233 and 234 comes in tick 234, and after tick
/// 240 person 21 runs tree 0x208a and has no running action. Why is
/// **unknown**; `docs/decisions.md` says what was ruled out. The other
/// three captures, and the original's code run in an emulator on the
/// first capture's state from update 47, choose in tick 233.
///
/// Object 15's two-button dialog in tick 2 is answered with 3, the answer
/// the captures' events show. People 18 and 21's clip in tick 3 is named
/// "-Rstand" in the script and found only by ignoring case; the original
/// plays the library's "-rstand" (`docs/decisions.md`). That the five
/// controllers' dialogs in tick 1 were answered with their only button is
/// what the captures show.
///
/// The digests suffixed with an older snapshot format were pinned under
/// it ([`format_11_bytes`] to [`format_14_bytes`]), before frames were
/// drawn and before the end of the object read set registry byte 0x12
/// ([`with_read_flag_clear`], for the state after tick 1); all still
/// hold. The states after ticks 5, 13, 33 and 232 were pinned before
/// under format 14 and are pinned again: since then the frames count the
/// rooms that wait, so from the frame after tick 2 on the rooms hold
/// other values (before, rooms 1 to 4 waited for ever). The rooms are in
/// no capture; everything the captures hold is equal before and after
/// that change. The states after ticks 1, 5, 13, 33, 232 and 261 were
/// pinned under format 16 ([`format_16_bytes`]), before the motion's timer
/// and the layer's fade were kept, and still hold; the state after tick
/// 261 is pinned again under format 17 (`TICK_261_17`). The states after
/// ticks 261 and 333 were pinned under format 17, before the head's fields
/// were kept, and still hold ([`format_17_bytes`]); the state after tick
/// 333 is pinned again under format 18 (`TICK_333_18`), and so are the
/// states after ticks 334 and 366; they still hold under format 19
/// ([`format_18_bytes`]), which pinned the states after ticks 367 and
/// 408. Those still hold under format 20 ([`format_19_bytes`]), which
/// pinned the state after tick 416; that still holds under format 21
/// ([`format_20_bytes`]), which pins the state after tick 417. The
/// digests are of our own state, as built here.
#[test]
fn the_goth_house_first_tick_matches_the_original() {
    const TICK_1_9: &str = "62fc28f5e7689d166791a620dbed066fb8a7d707ea79017fbadf0eed3da5c937";
    const TICK_1_10: &str = "bb55f7d5f656551708867b24779c2e0f019825459427858a27ad8c8874e7957b";
    const TICK_1_11: &str = "c2fa1ed11813b8ab67214f7177e1bb27a2e25ae2ecef94c3c1172071d5682400";
    const TICK_1_12: &str = "bc16bff8543bbd815f609600eb087157307e07b9ee0b592d3b55a64d2bc740f9";
    const TICK_1_13: &str = "f8fecfe693e419a642094715d879184b1d85268cbca22806ee31475b1aeb4dae";
    const TICK_1_14: &str = "0e022d8a9e16c9ea2dcea3654c0e85259f623678e68db62ccc807e17d99ebac7";
    const TICK_1: &str = "70a2c8fce9972dac5134eaf1f14295f6c4905318ed071224834684556354f338";
    const TICK_5: &str = "b1ba88aeef2c392bdb499dc61d64e9294d2f70cb3075e358c39cce054699fc3f";
    const TICK_13: &str = "ca6641d6917a3d13627fb3950989bc0bc9102782c973e0fd4447dfbf35b93eb5";
    const TICK_33: &str = "993d4a392c989284ac46b24598bea898621ce2361c106b38d2e5d39d398f67f3";
    const TICK_232: &str = "3e093fa4389e6ff31c8e9c976dfd03d7c1796b495d21b0e11bcb17d5d03111d7";
    const TICK_261: &str = "1b931d4492d3e02a8c72bcb658c55110e6b0f535cd20d99a937d09b6d62a420b";
    const TICK_261_17: &str = "2cc7b436d7f3e46f09b4801883c5d258d7d1e1e7411ba32e54eb1c8e52c419d4";
    const TICK_333_17: &str = "d82f96a31cf3dfd69c64b0b7ea14c0f943feb566ed2ecf7abdc8bcf845200916";
    const TICK_333_18: &str = "8c1e86596e70a9d320c14794e7483c4a8a9304c36236c0b1d4f3257f4d69af95";
    const TICK_334_18: &str = "85a315a4b435176ffe2e09ce08fb75880c792a0313743c94c21548f65653ef8b";
    const TICK_366_18: &str = "d9bcf685db4e2c1186715bf7a674c640bd8c4cbd6cbed374644c370b242667f0";
    const TICK_367_19: &str = "0a9a7c29774c3b5cee673ab0bc94f319a95b72fb9787e282ec9cbedc9975e82a";
    const TICK_408_19: &str = "7a9c943eba9536e011feafaf60b611e9d9accad6a0a50f8c13856dc72d4cb4d1";
    const TICK_417: &str = "e5b71ee022f181d8de6c5e72b9877002cfc7ba466e898e0105220e6287df6938";
    const TICK_436: &str = "5a60a15adc651b9d00329105701dd6c7841ab0fcbfb45b579a4e6cf46d8ecb6b";
    const TICK_466: &str = "7dd965a56f279614b46c16223faf1abad52290db1940fa293a082e559449b9cd";
    const TICK_583: &str = "95602638fb97031232502163ecded9078771357166b92166150eebfba73adb2d";
    const TICK_608: &str = "baef8063918e1a4bf5ba86408a3121db72ac29b0dfc9052afa5a63da07a9fddc";
    const TICK_871: &str = "9f87c3362a000543a2e3283da523261e51253e3a9c63184832bb10c62120a1f6";
    const TICK_877: &str = "a1a7d5d98711415cfe740d946f4ca561104f070ef18da034e5d4bbd16c9692fa";
    const TICK_900: &str = "cd9a69d332addf02fb32dcb811114d0dbc83392ba9ffb82b10aff4897fce6b5b";
    const TICK_991: &str = "5db7014c62c2a37753f8101f2be368e30f151429a4b1404e5e3a4b73c7e11fd1";
    const TICK_1000: &str = "43c2f13eee21f408a2f07965b6354a841751eb23ec4f92ba1c3b27947a375416";
    const TICK_416_20: &str = "37ff5756ac5cf26769da735eadb955be4f78f64381ea8b847963298b51f8320b";
    const USES_233: [(i16, i32); 2] = [(132, 5), (51, 8)];
    const TREES_233: [u16; 9] = [
        0x1000, 0x2001, 0x205b, 0x2029, 0x100e, 0x1014, 0x1012, 0x1009, 0x1010,
    ];
    const TREES_261: [u16; 9] = [
        0x1000, 0x2001, 0x205b, 0x2029, 0x100e, 0x1014, 0x1012, 0x1009, 0x1011,
    ];
    const POSITION_261: [i32; 3] = [409, 408, 1];
    let trees = |world: &World, id: u16| -> Vec<u16> {
        world.objects[&id]
            .stack
            .iter()
            .map(|frame| frame.tree)
            .collect()
    };
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = std::path::Path::new(&root);
    let install = Install::detect(root).unwrap();
    let neighborhood = Neighborhood::open(&install.file("UserData").unwrap()).unwrap();
    let family = neighborhood.family_in(5).unwrap().1.clone();
    selected(root, |interpreter, world, people| {
        assert_eq!(interpreter.visit_objects(world, people).stopped, None);
        interpreter.ensure_indicators(world).unwrap();
        finish_goth_selection(world, &family);
        interpreter.announce_house(world).unwrap();
        interpreter
            .select_resident(world, family.budget, &family.members)
            .unwrap();
        interpreter.go_live(world).unwrap();
        interpreter.before_frame(world).unwrap();
        let before = world.clone();
        // The house view draws a frame before the first lot update, which
        // draws the three people for the first time: each gets its
        // animator, placed on its tile (`frame.rs`).
        interpreter.draw_frame(world, 1).unwrap();
        let drawn: Vec<(u16, bool)> = world
            .animators
            .iter()
            .map(|(&id, a)| (id, a.undrawn))
            .collect();
        assert_eq!(drawn, [(16, false), (18, false), (21, false)]);
        let tuning = interpreter.program.motive_tuning().unwrap();
        let mut player = sims_vm::Answers::default();
        let update = interpreter.lot_update(world, &tuning, &mut player).unwrap();
        assert!(update.full);
        assert_eq!((update.counted.len(), update.ticked.len()), (267, 119));
        // The five controllers ask a one-button dialog and yield; the
        // registry shows each and it is answered.
        assert_eq!(
            update.answered,
            [(360, 2), (362, 2), (363, 2), (364, 2), (365, 2)]
        );
        // The seven IDs the load left queued, then two that the tick
        // queued.
        assert_eq!(
            update.removed,
            [355, 352, 350, 313, 312, 305, 292, 262, 351]
        );
        assert_eq!(world.ticks, before.ticks + 1);
        assert_eq!(world.globals[6], before.globals[6] + 2);
        assert_eq!(world.crt_rng, before.crt_rng);
        // Each person's motives decayed once.
        assert_eq!(world.person_ticks.len(), 3);
        assert!(
            world
                .person_ticks
                .values()
                .all(|t| t.last_decay == before.ticks as i32)
        );
        assert_eq!(digest(&format_9_bytes(world)), TICK_1_9);
        let old = with_read_flag_clear(world);
        assert_eq!(digest(&format_10_bytes(&old)), TICK_1_10);
        assert_eq!(digest(&format_11_bytes(&old)), TICK_1_11);
        assert_eq!(digest(&format_12_bytes(&old)), TICK_1_12);
        assert_eq!(digest(&format_13_bytes(&old)), TICK_1_13);
        assert_eq!(digest(&format_14_bytes(world)), TICK_1_14);
        assert_eq!(digest(&format_16_bytes(world)), TICK_1);
        // Ticks 2 to 5. Person 16 starts a clip with Animate Sim in tick 2
        // and yields; the capture shows that person at the same
        // instruction after ticks 2 to 5, with the same frames (the 254 in
        // its last local is a return's target left in the frame storage).
        // Controller 364 makes object 262 again and puts it down with Find
        // Location For. Object 15 asks a two-button dialog, answered with
        // 3 as the capture's events show. In tick 3 people 18 and 21 start
        // a clip and 35 objects are queued for removal.
        let mut player = sims_vm::Answers { queue: [3].into() };
        let mut shape = vec![];
        for tick in 2..=5 {
            let update = interpreter.lot_update(world, &tuning, &mut player).unwrap();
            assert!(update.full);
            if tick <= 3 {
                // Frames after ticks 1 to 3. Each counts the rooms that
                // wait to be counted, and changes nothing else.
                let mut drawn = world.clone();
                interpreter.draw_frame(world, 1).unwrap();
                let counted: Vec<i16> = drawn
                    .rooms
                    .rooms
                    .iter()
                    .filter(|&(id, room)| world.rooms.rooms[id] != *room)
                    .map(|(&id, _)| id)
                    .collect();
                let expected: &[i16] = if tick == 2 { &[1, 2, 3, 4] } else { &[] };
                assert_eq!(counted, expected);
                assert_eq!(world.rooms.pending, 0);
                drawn.rooms = world.rooms.clone();
                assert_eq!(world.encode(), drawn.encode());
            }
            shape.push((
                update.counted.len(),
                update.ticked.len(),
                update.answered.clone(),
                update.removed.len(),
            ));
        }
        assert_eq!(
            shape,
            [
                (357, 20, vec![(15, 3)], 0),
                (276, 103, vec![], 35),
                (340, 4, vec![], 0),
                (340, 4, vec![], 0),
            ]
        );
        assert_eq!(world.ticks, before.ticks + 5);
        assert_eq!(world.crt_rng, before.crt_rng);
        let frames: Vec<_> = world.objects[&16]
            .stack
            .iter()
            .map(|f| (f.tree, f.node))
            .collect();
        assert_eq!(
            frames,
            [
                (0x1000, 1),
                (0x2001, 4),
                (0x205b, 13),
                (0x206f, 0),
                (0x2025, 18)
            ]
        );
        assert_eq!(world.objects[&16].stack[4].locals, [0, 3, 8222, 254]);
        assert_eq!(digest(&format_16_bytes(world)), TICK_5);
        // Ticks 6 to 13. In tick 6 object 382 makes a person of type
        // 0x44A86A6C (Create New Object Instance mode 8), who gets the
        // lowest unused ID, 259, and is put down where the pedestrian
        // portal stands. (The load left an object with that ID; it was
        // removed in tick 3.)
        // From tick 7 on that person runs its own scripts.
        assert!(!world.objects.contains_key(&259));
        let mut shape = vec![];
        for tick in 6..=13 {
            let update = interpreter.lot_update(world, &tuning, &mut player).unwrap();
            assert!(update.full);
            if tick == 6 {
                assert_eq!(world.objects[&259].guid, 0x44A8_6A6C);
            }
            shape.push((
                update.counted.len(),
                update.ticked.len(),
                update.answered.clone(),
                update.removed.len(),
            ));
        }
        assert_eq!(
            shape,
            [
                (338, 6, vec![], 0),
                (341, 4, vec![], 0),
                (341, 4, vec![], 0),
                (341, 4, vec![], 0),
                (337, 8, vec![], 0),
                (340, 5, vec![], 0),
                (340, 5, vec![], 0),
                (339, 6, vec![], 0),
            ]
        );
        assert_eq!(world.ticks, before.ticks + 13);
        let people: Vec<u16> = world
            .objects
            .iter()
            .filter(|(_, o)| o.person.is_some())
            .map(|(&id, _)| id)
            .collect();
        assert_eq!(people, [16, 18, 21, 259]);
        let frames: Vec<_> = world.objects[&259]
            .stack
            .iter()
            .map(|f| (f.tree, f.node))
            .collect();
        assert_eq!(frames, [(0x1000, 9), (0x118, 0)]);
        assert_eq!(digest(&format_16_bytes(world)), TICK_13);
        // Ticks 14 to 33. In tick 14 person 259 runs a tree of object 51
        // (Run Tree By Name), which starts using it, and in that tree goes
        // to the object's routing slot 0 (Go To Routing Slot): it plans a
        // path of 11 points and its animator starts the route, then turns
        // on the spot (state 4) through ticks 15 to 33 while the thread
        // yields. In tick 24 person 18's clip ends and it starts another.
        let mut shape = vec![];
        for _ in 14..=33 {
            let update = interpreter.lot_update(world, &tuning, &mut player).unwrap();
            assert!(update.full);
            shape.push((update.counted.len(), update.ticked.len()));
        }
        // Ticks 14 to 33, in order.
        let mut expected = vec![(341, 4); 20];
        expected[8] = (340, 5);
        expected[9] = (339, 6);
        expected[16] = (339, 6);
        expected[17] = (332, 13);
        expected[18] = (339, 6);
        expected[19] = (338, 7);
        assert_eq!(shape, expected);
        assert_eq!(world.ticks, before.ticks + 33);
        let uses = &world.objects[&259].person.as_ref().unwrap().object_uses;
        let uses: Vec<_> = uses.iter().map(|u| (u.target, u.stack_depth)).collect();
        assert_eq!(uses, [(51, 4)]);
        assert_eq!(world.objects[&51].data[62], 1);
        let route = &world.routing.routes[&259];
        assert_eq!(route.path.len(), 11);
        assert_eq!(route.path[..3], [[24, 664], [76, 656], [204, 606]]);
        assert_eq!(route.path[10], [408, 413]);
        assert_eq!(route.contexts.len(), 1);
        assert_eq!(
            (route.contexts[0].target, route.contexts[0].chosen),
            (51, 0)
        );
        assert!(world.routing.unknown_words.contains(&(259, 37)));
        let animator = &world.animators[&259];
        assert_eq!((animator.state, animator.arrival), (4, 1));
        assert_eq!(animator.motion.segment, 1);
        assert_eq!(animator.motion.progress.to_bits(), 0x3de9_8f51);
        assert_eq!(animator.destination[2].to_bits(), 0x3e57_5200);
        assert!(animator.undrawn);
        assert_eq!(digest(&format_16_bytes(world)), TICK_33);
        // The frame after tick 33 draws person 259 for the first time,
        // which unsets its animator's position and places it again on its
        // tile, facing its direction word: 0, where the turn had it at
        // 0.127 (the destination's direction too).
        interpreter.draw_frame(world, 1).unwrap();
        let animator = &world.animators[&259];
        assert!(!animator.undrawn);
        assert_eq!(animator.position[2].to_bits(), 0);
        assert_eq!(animator.destination[2].to_bits(), 0);
        // Ticks 34 to 261, with a frame after every 30th tick from 33 (63,
        // 93, ...), as the captures recorded.
        for tick in 34..=261 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
            if (tick - 33) % 30 == 0 {
                interpreter.draw_frame(world, 1).unwrap();
            }
            if tick == 36 {
                // The tick count is a multiple of 40: the end of the update
                // clears registry byte 0x12 (0x442521).
                assert_eq!(world.ticks % 40, 0);
                assert!(!world.neighbors.relationships_changed);
            }
            if tick == 232 {
                assert_eq!(digest(&format_16_bytes(world)), TICK_232);
                assert_eq!(world.action_serial, 0);
            }
            if tick == 233 {
                // Person 21 runs Find Best Interaction, which chooses menu 6
                // of object 132 and draws once from the lot's stream, and
                // Gosub Found Action, which starts it. The interaction's
                // tree goes to the object, in another room: the routing
                // between rooms chooses door 51, and the person starts the
                // door's tree (0x1009, 0x1010), which walks to it.
                assert_eq!(world.lot_rng.state(), 2_459_469_755);
                assert_eq!(world.action_serial, 1);
                let person = world.objects[&21].person.as_ref().unwrap();
                let action = &person.active;
                assert_eq!(
                    (action.uid, action.caller, action.target, action.icon),
                    (1, 21, 132, 132)
                );
                assert_eq!(
                    (action.table_index, action.priority, action.action),
                    (6, 2, 0x100e)
                );
                assert_eq!(
                    (action.attenuation.to_bits(), action.flags),
                    (0x3f19_999a, 0)
                );
                let uses: Vec<_> = person
                    .object_uses
                    .iter()
                    .map(|u| (u.target, u.stack_depth))
                    .collect();
                assert_eq!(uses, USES_233);
                assert_eq!(world.objects[&132].data[62], 1);
                assert_eq!(trees(world, 21), TREES_233);
            }
            if tick == 246 {
                // Person 21 stands at the door: its animator is in state 3
                // with arrival 2, and no longer walks.
                let animator = &world.animators[&21];
                assert_eq!((animator.state, animator.arrival), (3, 2));
                assert!(!animator.walking);
            }
            if tick == 255 {
                // It has gone through the door into room 4, which is dark
                // and has no lamp: the room is marked as knowing someone is
                // in it and waits to be counted again.
                assert_eq!(world.person_ticks[&21].room, 4);
                assert!(world.rooms.rooms[&4].occupied);
                assert_eq!(world.rooms.pending, 1);
            }
        }
        assert_eq!(world.ticks, before.ticks + 261);
        assert_eq!(trees(world, 21), TREES_261);
        let animator = &world.animators[&21];
        assert_eq!((animator.state, animator.arrival), (3, 2));
        assert_eq!(world.objects[&21].position, POSITION_261);
        assert_eq!(digest(&format_16_bytes(world)), TICK_261);
        assert_eq!(digest(&format_17_bytes(world)), TICK_261_17);
        // The walk's strides, worked out by posing each person's skeleton
        // with its walk clip, equal what the original's constructor stored
        // (animator +0x17c and +0x178, the same in the capture's updates
        // 1, 300 and 600).
        let strides = |id| interpreter.strides(world, id).unwrap().map(|f| f.to_bits());
        assert_eq!(strides(16), [0x40d8_93f8, 0x40bc_ad30]);
        assert_eq!(strides(18), [0x412c_5b86, 0x412a_0544]);
        assert_eq!(strides(21), [0x412c_5b86, 0x412a_0544]);
        // Ticks 262 to 333, with frames after ticks 273, 303 and 333.
        for tick in 262..=333 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
            if (tick - 33) % 30 == 0 {
                interpreter.draw_frame(world, 1).unwrap();
            }
            let animator = &world.animators[&21];
            let motion = &animator.motion;
            let fields = (
                animator.state,
                animator.arrival,
                animator.selection,
                animator.walking,
                motion.near_end,
                motion.finishing,
                motion.timer,
            );
            match tick {
                // Person 21 walks the last point of its path with data word
                // 9 clear: the foreground clip may not be interrupted and
                // has not finished, so it walks on without a timer.
                262 => assert_eq!(fields, (9, 1, 20, true, 1, 1, 0)),
                // It stops at the end of the path and turns on the spot.
                263 => assert_eq!(fields, (4, 1, 5, false, 1, 1, 0)),
                // At the end of its next path, within a stride of it with
                // no foreground clip, it sets the timer to stop.
                296 => assert_eq!(fields, (9, 1, 20, true, 1, 1, 4_437_050)),
                // The timer stops it there, and it turns: three eighths or
                // less, the clip of a quarter turn.
                297 => assert_eq!(fields, (4, -1, 7, true, 1, 0, 0)),
                // Turning clears the walking flag.
                298 => assert_eq!(fields, (4, -1, 7, false, 1, 0, 0)),
                // It stands, then plays a clip from its own table (Animate
                // Sim, source 0).
                321 => assert_eq!(fields, (3, 2, -1, false, 1, 0, 0)),
                322 => assert_eq!(fields, (0, 0, -1, false, 1, 0, 0)),
                _ => {}
            }
        }
        assert_eq!(world.ticks, before.ticks + 333);
        let old = without_saved_targets(world, &before);
        assert_eq!(digest(&format_17_bytes(&old)), TICK_333_17);
        assert_eq!(digest(&format_18_bytes(&old)), TICK_333_18);
        // Tick 334: person 16 looks towards person 259 (Look Towards, mode
        // 0, sets its person data words 41 to 45), and its head then
        // follows it (0x590768, 0x5904d2): it measures the heading from
        // the top of its head to the point in front of it, and to the top
        // of person 259's head, from the bones of both in their current
        // poses. The head fields equal the capture's update 334.
        interpreter.lot_update(world, &tuning, &mut player).unwrap();
        assert_eq!(world.ticks, before.ticks + 334);
        let head = world.animators[&16].head;
        let bits = |f: sims_core::F32| f.to_bits();
        assert_eq!(
            (
                bits(head.heading),
                bits(head.target_heading),
                bits(head.turn)
            ),
            (0x3cfd_e622, 0x3fa8_e863, 0)
        );
        assert_eq!(
            head.target.map(|t| t.map(bits)),
            Some([0x425a_088a, 0x40f0_d678, 0x42d0_f97e])
        );
        // The others' heads are as the load left them: angles 0, and the
        // target the save held (the leftover words 0x00fdf000), which no
        // measurement has written since.
        for id in [18, 21, 259] {
            let head = world.animators[&id].head;
            let saved = before.animators.get(&id).and_then(|a| a.head.target);
            assert_eq!(head.target, saved, "person {id}");
            assert_eq!(
                sims_sim::head::Head {
                    target: None,
                    ..head
                },
                sims_sim::head::Head::LOADED
            );
        }
        assert_eq!(
            digest(&format_18_bytes(&without_saved_targets(world, &before))),
            TICK_334_18
        );
        // Ticks 335 to 366, with a frame after tick 363.
        for tick in 335..=366 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
            if (tick - 33) % 30 == 0 {
                interpreter.draw_frame(world, 1).unwrap();
            }
            let bits = |f: sims_core::F32| f.to_bits();
            match tick {
                // Person 18 starts its walk with clip 0x13 (state 6,
                // 0x58e125): its destination is one clip's distance ahead,
                // and at phase 0 it has not moved yet.
                335 => {
                    let a = &world.animators[&18];
                    assert_eq!((a.state, a.selection, a.walking), (6, 0x13, false));
                    assert_eq!(
                        a.position[..2].iter().map(|&f| bits(f)).collect::<Vec<_>>(),
                        [0x43dc_0000, 0x43cc_0000]
                    );
                    assert_eq!(bits(a.destination[0]), 0x43d7_ad0d);
                }
                // Person 16 looks at person 259 standing on a sloped
                // tile: the target's height comes from the tile's corners
                // (0x478dfe), as in the capture's update 357.
                357 => assert_eq!(
                    world.animators[&16].head.target.map(|t| t.map(bits)),
                    Some([0x4266_6ec9, 0x40f7_8106, 0x42cb_1aba])
                ),
                _ => {}
            }
        }
        assert_eq!(world.ticks, before.ticks + 366);
        assert_eq!(
            digest(&format_18_bytes(&without_saved_targets(world, &before))),
            TICK_366_18
        );
        // Tick 367: person 18's route to a slot finds no way, because
        // person 21 stands in it. Step 6 tries again, then finds person 21
        // in the way (0x45ba28) and asks it to move (0x45bf2e): a marker
        // (260) is made where person 21 stands and person 21 is given
        // action 4, to go to the marker's move slot. The slot search draws
        // 72 times from the lot's stream (0x483b02). Step 14 then plays
        // "route ask to move animation" with person 21 as the stack
        // object.
        interpreter.lot_update(world, &tuning, &mut player).unwrap();
        assert_eq!(world.ticks, before.ticks + 367);
        assert_eq!(world.lot_rng.state(), 689_947_733);
        assert_eq!(world.action_serial, 4);
        assert_eq!(world.objects[&260].guid, 0x7c4);
        let asked = &world.objects[&21].person.as_ref().unwrap().queue;
        assert_eq!(
            asked
                .iter()
                .map(|a| (a.uid, a.caller, a.target, a.table_index, a.priority))
                .collect::<Vec<_>>(),
            [(4, 21, 260, 3, 2)]
        );
        let context = world.routing.routes[&18].contexts.last().unwrap();
        assert_eq!((context.objects[1], context.action), (21, 4));
        assert!(world.routing.slots.contains_key(&(260, 0)));
        assert_eq!(
            digest(&format_19_bytes(&without_saved_targets(world, &before))),
            TICK_367_19
        );
        // Tick 368: person 21 takes a book from the bookcase (object 132),
        // whose tree puts on the accessory "suit-book101" (Change
        // Suit/Accessory, opcode 6). The capture shows one accessory on
        // person 21 from update 368 on.
        for tick in 368..=408 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
            if tick == 368 {
                let worn = &world.objects[&21].person.as_ref().unwrap().accessories;
                assert_eq!(
                    worn.iter()
                        .map(|a| (a.name.as_str(), a.binding.as_str()))
                        .collect::<Vec<_>>(),
                    [("suit-book101", "")]
                );
            }
        }
        assert_eq!(world.ticks, before.ticks + 408);
        assert_eq!(
            digest(&format_19_bytes(&without_saved_targets(world, &before))),
            TICK_408_19
        );
        // Person 259, made at run time, heads the registration list but
        // ends the person vector.
        assert_eq!(world.people, [21, 18, 16, 259]);
        assert_eq!(world.order[1], 259);
        // Tick 409: person 18's waiting tree looks round the people in the
        // vector (Set To Next mode 1), and looks at person 259.
        interpreter.lot_update(world, &tuning, &mut player).unwrap();
        assert_eq!(world.objects[&18].person.as_ref().unwrap().data[41], 259);
        for _ in 410..=416 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        assert_eq!(
            digest(&format_20_bytes(&without_saved_targets(world, &before))),
            TICK_416_20
        );
        // Tick 417: person 21's tree 0x1014 of the bookcase finds the best
        // object for function 8 (Find Best Object for Function, opcode
        // 14): object 174, the stack object the capture shows. The tree
        // then plays a clip on the carry slot (Animate Sim, flag 0x10),
        // which repeats there, and takes the true branch to node 7: Run
        // Functional Tree (opcode 20) runs object 174's action (tree 8215),
        // which runs trees 4105 and 4112 of object 52. The
        // capture shows the same frames and object uses after the tick.
        interpreter.lot_update(world, &tuning, &mut player).unwrap();
        assert_eq!(world.ticks, before.ticks + 417);
        let a = &world.animators[&21];
        let carry = a.carry.as_ref().unwrap();
        assert_eq!((carry.mode, carry.priority), (1, 10));
        assert_eq!((a.carry_mode, a.carry_selection), (6, -1));
        let person = &world.objects[&21];
        assert_eq!(
            person.stack[6..]
                .iter()
                .map(|f| (f.tree, f.node, f.stack_object))
                .collect::<Vec<_>>(),
            [(8215, 2, 174), (4105, 1, 52), (4112, 5, 52)]
        );
        assert_eq!(
            person
                .person
                .as_ref()
                .unwrap()
                .object_uses
                .iter()
                .map(|u| (u.target, u.stack_depth))
                .collect::<Vec<_>>(),
            [(132, 5), (174, 7), (52, 8)]
        );
        assert_eq!(
            digest(&format_21_bytes(&without_saved_targets(world, &before))),
            TICK_417
        );
        // Tick 418: person 21's tree 4113 of object 52 routes to the
        // door's slot with two people in the way, walked in the order of
        // the person vector (0x45bb04). The route goes through, and the
        // walk goes on to door 92, whose trees 4105 and 4112 run as in the
        // capture.
        interpreter.lot_update(world, &tuning, &mut player).unwrap();
        assert_eq!(
            world.objects[&21].stack[6..]
                .iter()
                .map(|f| (f.tree, f.node, f.stack_object))
                .collect::<Vec<_>>(),
            [(8215, 2, 174), (4105, 1, 92), (4112, 6, 92)]
        );
        for _ in 419..=436 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        assert_eq!(world.ticks, before.ticks + 436);
        assert_eq!(
            digest(&format_21_bytes(&without_saved_targets(world, &before))),
            TICK_436
        );
        // Tick 437: person 16's animator finds its next step blocked, and
        // its route ends blocked (+0x188 = 3, 0x58f682), as the capture
        // shows.
        interpreter.lot_update(world, &tuning, &mut player).unwrap();
        assert_eq!(world.animators[&16].arrival, 3);
        // Tick 438: the route stops short (step 10) and tries again (step
        // 6), finds no way and gives up (step 16) with the result the
        // planning left in the context, 13 (its refusals); the walk goes
        // on to door 87, as in the capture.
        interpreter.lot_update(world, &tuning, &mut player).unwrap();
        let person = &world.objects[&16];
        assert_eq!(person.person.as_ref().unwrap().data[62], 13);
        assert_eq!(
            person.stack[person.stack.len() - 2..]
                .iter()
                .map(|f| (f.tree, f.node, f.stack_object))
                .collect::<Vec<_>>(),
            [(4105, 1, 87), (4112, 5, 87)]
        );
        for _ in 439..=466 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        assert_eq!(world.ticks, before.ticks + 466);
        assert_eq!(
            digest(&format_21_bytes(&without_saved_targets(world, &before))),
            TICK_466
        );
        // Tick 467: person 21's tree 0x1017 looks for a door beside door
        // 93 with Get Direction To (opcode 12). The direction from 93 to
        // object 126, in the native order (+0xf4 first), is 0, not the 6
        // it looks for, so the scan runs out and tree 0x100a draws a
        // Random Number of 2 (0x451ed8): one draw from the lot's stream,
        // as the capture's stream shows.
        interpreter.lot_update(world, &tuning, &mut player).unwrap();
        assert_eq!(world.lot_rng.state(), 3_991_923_174);
        for _ in 468..=583 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        assert_eq!(world.ticks, before.ticks + 583);
        assert_eq!(
            digest(&format_21_bytes(&without_saved_targets(world, &before))),
            TICK_583
        );
        // Tick 584: person 18's tree 0x1011 runs Snap (opcode 46) to the
        // routing slot of object 207, one level up, and stands where 207
        // stands, as in the capture.
        interpreter.lot_update(world, &tuning, &mut player).unwrap();
        assert_eq!(world.objects[&18].position, [360, 376, 2]);
        assert_eq!(world.objects[&18].data[29], 1025);
        for _ in 585..=608 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        assert_eq!(world.ticks, before.ticks + 608);
        assert_eq!(
            digest(&format_21_bytes(&without_saved_targets(world, &before))),
            TICK_608
        );
        // Tick 609: object 260, on a tile of the lot, removes itself
        // (Remove Object Instance): it leaves the lot through the
        // placement and is queued.
        assert_eq!(world.objects[&260].position, [408, 408, 1]);
        interpreter.lot_update(world, &tuning, &mut player).unwrap();
        assert!(
            world
                .objects
                .get(&260)
                .is_none_or(|o| o.position == [-16, -16, 1]),
            "object 260 is still on the lot"
        );
        // Tick 672: person 21's first, unbounded search finds no path, and
        // In the Way asks person 16 to move, whose slot search keeps a
        // random choice of its places.
        for _ in 610..=690 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        // Tick 691: person 18 routes in a room upstairs (room 1025).
        for _ in 691..=752 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        // Tick 753: person 21 carries something, and its carry layer moves
        // no bone its head reads. Tick 798: Set Motive Change.
        for _ in 753..=871 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        assert_eq!(world.ticks, before.ticks + 871);
        assert_eq!(
            digest(&format_21_bytes(&without_saved_targets(world, &before))),
            TICK_871
        );
        // Tick 872: object 121, a trash can, clears its headline.
        for _ in 872..=877 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        assert_eq!(world.ticks, before.ticks + 877);
        assert_eq!(
            digest(&format_21_bytes(&without_saved_targets(world, &before))),
            TICK_877
        );
        // Tick 878: the hour turns to 7:00. The save already holds day as
        // the time of day (global 4 is 0 at 6:30), so nothing more happens:
        // no room is marked and the lawn, though global 36 lets it, does not
        // grow. The capture holds the same two globals.
        assert_eq!((world.globals[4], world.globals[36]), (0, 1));
        for _ in 878..=900 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        assert_eq!(world.ticks, before.ticks + 900);
        assert_eq!(
            digest(&format_21_bytes(&without_saved_targets(world, &before))),
            TICK_900
        );
        // Tick 901: object 8 reads job data. The capture drew a frame
        // after it, which counts the rooms waiting since earlier changes
        // (rooms 0, 1, 3 and 4 here); in tick 902 person 16's room motive
        // follows.
        interpreter.lot_update(world, &tuning, &mut player).unwrap();
        interpreter.draw_frame(world, 1).unwrap();
        for _ in 902..=991 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        assert_eq!(world.ticks, before.ticks + 991);
        assert_eq!(
            digest(&format_21_bytes(&without_saved_targets(world, &before))),
            TICK_991
        );
        // Tick 992: object 21 snaps into a slot of object 174: its
        // routing slot 0, the second entry of its SLOT chunk, which puts
        // it into the object's container slot. The fifth capture ends
        // after tick 1000.
        for _ in 992..=1000 {
            interpreter.lot_update(world, &tuning, &mut player).unwrap();
        }
        assert_eq!(world.ticks, before.ticks + 1000);
        assert_eq!(
            digest(&format_21_bytes(&without_saved_targets(world, &before))),
            TICK_1000
        );
    });
}
