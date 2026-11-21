//! Load step 6e: saved objects are constructed, then read.

use crate::support::*;
use sims_content::{Clip, Event, People, Skeleton, Track};
use sims_core::CrtRng;
use sims_core::F32;
use sims_formats::saved_objects::{Frame, Instance, Multipart, Objects, Slot};
use sims_sim::animator::{Base, LOADED};
use sims_sim::neighbors::{Neighbor, Neighbors};
use sims_vm::{Fault, Interpreter, SavedRead, Status, Trap, Update};
use std::collections::BTreeMap;

const CHAIR: u32 = 0x5001;
const ADULT: u32 = 0x5002;
const CHILD: u32 = 0x5003;
const STOOL: u32 = 0x5004;

/// A cleared lot whose catalog has a chair (12 attributes, 3 dynamic
/// sprites), two person types, and a stool whose definition asks for
/// nothing. Both person types' scripts changed since the save, so load
/// step 13 resets their people.
fn lot() -> Fixture {
    let mut fx = Fixture::new();
    fx.world.objects.clear();
    fx.world.order.clear();
    fx.world.people.clear();
    let chair = fx.add_type(CHAIR, 4, 0, 4097);
    fx.words(chair)[58] = 12;
    fx.words(chair)[65] = 3;
    fx.add_type(ADULT, 2, 0, 4097);
    fx.add_type(CHILD, 2, 0, 4097);
    fx.add_type(STOOL, 4, 0, 4097);
    fx.world.neighbors = records(11);
    fx.world.changed_types = [ADULT, CHILD].into();
    fx
}

/// The neighborhood's records of the two person types, every person data
/// word `interest`.
fn records(interest: i16) -> Neighbors {
    let record = |id, guid| Neighbor {
        id,
        guid,
        revision: 5,
        data: vec![interest; 256],
        relationships: Vec::new(),
        friends: 0,
        stale: true,
    };
    Neighbors {
        slots: vec![Some(record(31, ADULT)), Some(record(32, CHILD))],
        ..Neighbors::default()
    }
}

fn saved(id: u16, guid: u32) -> Instance {
    let object = object(id, guid);
    let mut data = object.data;
    data[11] = id as i16;
    Instance {
        id,
        type_id: 7,
        guid,
        footprint: [16, 16, 32, 32],
        position: [24, 40, 1],
        unknown_short: 1,
        attributes: vec![3, -4],
        temps: (1..=8).collect(),
        data,
        obsolete_word: 0,
        stack_capacity_bytes: 512,
        stack: Vec::new(),
        relationship_flag: -1,
        relationships: Vec::new(),
        slots: vec![Slot {
            unknown: 0,
            object: 0,
        }],
        dynamic_sprites: vec![9],
        multipart: None,
        portal: None,
        person: None,
        registry_word: Some(0x0002_0005),
        consumed_bits: 0,
        record: Vec::new(),
    }
}

fn file(instances: Vec<Instance>) -> Objects {
    Objects {
        version: 62,
        order: instances.iter().map(|i| i.id).collect(),
        types_by_id: BTreeMap::new(),
        instances: instances.into_iter().map(|i| (i.id, i)).collect(),
        tail: Vec::new(),
    }
}

fn read(fx: &mut Fixture, saved: &Objects, people: &People) -> Result<SavedRead, Fault> {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.read_objects(&mut fx.world, saved, people)
}

fn trap(result: Result<SavedRead, Fault>) -> (u16, Trap) {
    let fault = result.unwrap_err();
    (fault.object, fault.trap)
}

#[test]
fn an_inherited_tree_bit_survives_a_saved_sleep_until_dispatch() {
    let mut fx = lot();
    fx.world.globals.fill(0);
    let owner = fx.catalog.object(CHAIR).unwrap().resource;
    fx.add(
        owner,
        4096,
        tree(vec![
            ins(280, 1, RETURN_FALSE, [0; 8]),
            assign(scope::TEMPS, 0, scope::LITERAL, 42, RETURN_TRUE),
        ]),
    );
    fx.add(GLOBAL, 280, sleeper());

    let id = 9;
    let mut instance = saved(id, CHAIR);
    fx.world.types.insert(instance.type_id, CHAIR);
    instance.registry_word = Some(0x0001_0001);
    let caller = Frame {
        tree: 4096,
        node: 0,
        parameters: Vec::new(),
        locals: Vec::new(),
        stack_object: 0,
        primitive_state: 0,
        code_owner_type: instance.type_id as i16,
    };
    instance.stack = vec![
        caller.clone(),
        Frame {
            tree: 280,
            parameters: vec![20_000],
            ..caller
        },
    ];
    let saved = file(vec![instance]);
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter
        .read_objects_with_frame_tree_bits(
            &mut fx.world,
            &saved,
            &People::default(),
            &[(id, 1)].into(),
        )
        .unwrap();
    assert_eq!(saved.instances[&id].stack[1].tree, 280);
    assert_eq!(fx.world.objects[&id].stack[0].tree, 4096);
    assert_eq!(fx.world.objects[&id].stack[1].tree, 0x8118);

    let mut world = sims_sim::World::decode(&fx.world.encode()).unwrap();
    let mut update = Update::default();
    interpreter
        .scheduler_pass(&mut world, &crate::tick::tuning(), &mut update)
        .unwrap();
    assert_eq!(update.counted, [id]);
    assert_eq!(world.objects[&id].stack[1].tree, 0x8118);

    // Returning through a flagged caller masks its tree only for lookup.
    let object = world.objects.get_mut(&id).unwrap();
    object.stack[0].tree |= Frame::TREE_FLAG;
    assert_eq!(interpreter.step(&mut world, id), Ok(Status::Running));
    assert_eq!(world.objects[&id].stack.len(), 1);
    assert_eq!(world.objects[&id].stack[0].tree, 0x9000);
    assert_eq!(world.objects[&id].stack[0].node, 1);
    // The popped Sleep leaves the cleared tree word in the arena.
    assert_eq!(world.frame_storage[&id].words[8], Some(280));
    assert_eq!(interpreter.step(&mut world, id), Ok(Status::Returned(true)));
    assert_eq!(world.objects[&id].temps[0], 42);
    assert_eq!(world.frame_storage[&id].words[0], Some(4096));
}

#[test]
fn a_saved_object_gets_the_constructors_storage_and_its_saved_fields() {
    let mut fx = lot();
    // What the earlier load steps left: an instance list, a queued
    // removal, the random streams.
    fx.world.instances = BTreeMap::from([(CHAIR, Vec::new())]);
    fx.world.removals = vec![42];
    let before = fx.world.clone();
    let mut bare = saved(4, STOOL);
    bare.registry_word = None;
    let objects = file(vec![saved(9, CHAIR), bare, saved(6, CHAIR)]);
    let done = read(&mut fx, &objects, &People::default()).unwrap();
    assert_eq!(
        done,
        SavedRead {
            objects: vec![9, 4, 6],
            draws: 0
        }
    );
    let world = &fx.world;
    // The file's order, not ID order.
    assert_eq!(world.order, [9, 4, 6]);
    assert_eq!(world.objects.keys().copied().collect::<Vec<_>>(), [4, 6, 9]);
    let chair = &world.objects[&9];
    // As many attributes as the definition asks for, the saved ones first.
    assert_eq!(chair.attributes, [3, -4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(chair.dynamic_sprites, [9, 0, 0]);
    assert_eq!(chair.temps, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(chair.data, objects.instances[&9].data);
    assert_eq!(chair.position, [24, 40, 1]);
    assert_eq!(chair.registry_word, Some(0x0002_0005));
    assert!(!chair.last_result);
    // The stool's definition asks for no attributes, so it has 8. Its file
    // has no slots, so it has the one for its tile.
    let stool = &world.objects[&4];
    assert_eq!(stool.attributes, [3, -4, 0, 0, 0, 0, 0, 0]);
    assert_eq!(stool.dynamic_sprites, [9]);
    assert_eq!(stool.slots.len(), 1);
    // A record without a scheduler word keeps the one registration gave.
    assert_eq!((stool.countdown(), stool.flags()), (Some(-1), 0));
    // A type with an instance list gets its objects, in the file's order.
    assert_eq!(world.instances, BTreeMap::from([(CHAIR, vec![9, 6])]));
    assert!(world.next_parts.is_empty());
    // Nothing else moves.
    assert_eq!(world.removals, before.removals);
    assert_eq!(world.globals, before.globals);
    assert_eq!(world.crt_rng, before.crt_rng);
    assert_eq!(world.types, before.types);
}

#[test]
fn a_read_that_cannot_be_done_changes_nothing() {
    let mut fx = lot();
    let before = fx.world.clone();
    let mut unknown = saved(6, 0x9999);
    unknown.id = 6;
    let (object, cause) = trap(read(
        &mut fx,
        &file(vec![saved(9, CHAIR), unknown]),
        &People::default(),
    ));
    assert_eq!(object, 6);
    assert!(matches!(cause, Trap::Invalid(_)), "{cause:?}");
    // More saved attributes than the constructor allocates.
    let mut long = saved(9, CHAIR);
    long.attributes = vec![1; 13];
    let (_, cause) = trap(read(&mut fx, &file(vec![long]), &People::default()));
    assert!(matches!(cause, Trap::Unsupported(_)), "{cause:?}");
    // An object with graphics of its own.
    fx.words(4)[102] = 1;
    let (_, cause) = trap(read(
        &mut fx,
        &file(vec![saved(9, CHAIR)]),
        &People::default(),
    ));
    assert!(matches!(cause, Trap::Unsupported(_)), "{cause:?}");
    fx.words(4)[102] = 0;
    assert_eq!(fx.world.encode(), before.encode());
    // A lot that still holds objects.
    read(&mut fx, &file(vec![saved(9, CHAIR)]), &People::default()).unwrap();
    let held = fx.world.clone();
    let (object, cause) = trap(read(
        &mut fx,
        &file(vec![saved(4, CHAIR)]),
        &People::default(),
    ));
    assert_eq!(object, 9);
    assert!(matches!(cause, Trap::Unsupported(_)), "{cause:?}");
    assert_eq!(fx.world.encode(), held.encode());
}

/// Four types may keep more saved attributes than their constructor
/// allocates (0x43fe49, 0x434345): the array grows to the saved count.
/// The last two only with Unleashed's bit in the expansions (0x43fe9d).
#[test]
fn the_types_the_house_read_names_grow_their_attributes() {
    const FIRST: u32 = 0x3C08_965F;
    const THIRD: u32 = 0x4A70_DF92;
    let mut fx = lot();
    fx.add_type(FIRST, 4, 0, 4097);
    fx.add_type(THIRD, 4, 0, 4097);
    let long = |id, guid| {
        let mut object = saved(id, guid);
        object.attributes = (1..=11).collect();
        object
    };
    fx.world.globals[20] = 0x20;
    read(
        &mut fx,
        &file(vec![long(5, FIRST), long(6, THIRD)]),
        &People::default(),
    )
    .unwrap();
    for id in [5, 6] {
        assert_eq!(
            fx.world.objects[&id].attributes,
            (1..=11).collect::<Vec<i16>>()
        );
    }
    // Without the bit only the first two grow.
    let mut fx = lot();
    fx.add_type(FIRST, 4, 0, 4097);
    fx.add_type(THIRD, 4, 0, 4097);
    fx.world.globals[20] = 0xdf;
    let before = fx.world.encode();
    let (object, cause) = trap(read(
        &mut fx,
        &file(vec![long(5, FIRST), long(6, THIRD)]),
        &People::default(),
    ));
    assert_eq!(object, 6);
    assert!(
        matches!(&cause, Trap::Unsupported(m) if m.contains("0x4A70DF92")),
        "{cause:?}"
    );
    assert_eq!(fx.world.encode(), before);
}

fn tile(id: u16, lead: i16) -> Instance {
    let mut tile = saved(id, CHAIR);
    tile.multipart = Some(Multipart {
        offset: [0; 3],
        group: [0; 3],
        parent: lead,
    });
    tile
}

#[test]
fn tiles_join_their_leads_list_in_the_files_order() {
    let mut fx = lot();
    // Two groups, their tiles interleaved, one tile before its lead.
    let objects = file(vec![
        tile(5, 7),
        tile(9, 0),
        tile(4, 9),
        tile(7, 0),
        tile(6, 9),
        tile(3, 9),
        saved(8, CHAIR),
    ]);
    read(&mut fx, &objects, &People::default()).unwrap();
    assert_eq!(
        fx.world.next_parts,
        BTreeMap::from([(9, 4), (4, 6), (6, 3), (7, 5)])
    );
    // A tile whose lead is missing, or is not a multi-tile object, or is
    // itself.
    for (id, lead) in [(4, 20), (4, 8), (4, 4)] {
        let mut fx = lot();
        let objects = file(vec![tile(id, lead), saved(8, CHAIR)]);
        let (object, cause) = trap(read(&mut fx, &objects, &People::default()));
        assert_eq!(object, id);
        assert!(matches!(cause, Trap::Invalid(_)), "{cause:?}");
        assert!(fx.world.objects.is_empty());
    }
}

fn sim(id: u16, guid: u32) -> Instance {
    let mut sim = saved(id, guid);
    let mut person = person();
    // A save from before ObjM version 0x47 holds 80 words.
    person.data.truncate(80);
    sim.person = Some(person);
    sim
}

/// Two skeleton files: one with two skeletons, of which adults use the
/// second, and one with the child's.
fn people() -> People {
    People {
        suits: Default::default(),
        suit_errors: Default::default(),
        skeletons_by_name: BTreeMap::new(),
        skeleton_names: BTreeMap::new(),
        profiles: BTreeMap::new(),
        head_suits: Default::default(),
        portrait_head: BTreeMap::new(),
        skeletons: BTreeMap::from([
            (
                ADULT,
                Skeleton {
                    name: "adult".into(),
                    file: "a.bcf".into(),
                    place: 1,
                    // A scale of 1 costs a bone 32 draws, any other 48.
                    scales: vec![vec![1], vec![1, 7, 0]],
                    bones: Vec::new(),
                },
            ),
            (
                CHILD,
                Skeleton {
                    name: "child".into(),
                    file: "c.bcf".into(),
                    place: 0,
                    scales: vec![vec![5]],
                    bones: Vec::new(),
                },
            ),
        ]),
    }
}

fn advanced(mut stream: CrtRng, draws: u32) -> CrtRng {
    for _ in 0..draws {
        stream.next_u15();
    }
    stream
}

#[test]
fn constructing_people_draws_for_their_skeletons() {
    let mut fx = lot();
    fx.world.crt_rng = CrtRng::from_state(1234);
    let start = fx.world.crt_rng;
    let objects = file(vec![
        sim(3, ADULT),
        saved(9, CHAIR),
        sim(2, CHILD),
        sim(5, ADULT),
    ]);
    // Interests of 11 or more in the records: no defaults are made.
    let done = read(&mut fx, &objects, &people()).unwrap();
    // The first adult reads a.bcf (32 + 128) and builds its own copy
    // (128). The child reads c.bcf (48) and builds its copy (48). The
    // second adult only builds its copy (128).
    assert_eq!(done.draws, 160 + 128 + 48 + 48 + 128);
    assert_eq!(fx.world.crt_rng, advanced(start, done.draws));
    assert_eq!(
        fx.world.skeleton_files.iter().collect::<Vec<_>>(),
        ["a.bcf", "c.bcf"]
    );
    let adult = &fx.world.objects[&3];
    // A tile slot and five of a person's own.
    assert_eq!(adult.slots.len(), 6);
    // The saved 80 words, then zeros up to 256.
    let data = &adult.person.as_ref().unwrap().data;
    assert_eq!(data.len(), 256);
    assert_eq!(data[..80], person().data[..80]);
    assert!(data[80..].iter().all(|&word| word == 0));
    assert_eq!(adult.attributes.len(), 8);

    // A game that has read a.bcf already does not read it again.
    let mut fx = lot();
    fx.world.skeleton_files.insert("a.bcf".into());
    let done = read(&mut fx, &file(vec![sim(3, ADULT)]), &people()).unwrap();
    assert_eq!(done.draws, 128);
}

/// The kat skeleton file is read before the first load, under whatever
/// spelling the install gives it, so the first person of its skeleton
/// builds only its own copy.
#[test]
fn the_kat_file_read_before_the_first_load_is_matched_without_case() {
    let kat = "ExpansionShared/ExpansionShared.FAR/Skeletons\\KAT_skeleton.cmx.bcf";
    // Without a type whose skeleton comes from it, nothing is listed.
    assert!(sims_vm::read_before_the_first_load(&people()).is_empty());
    // Here the adults' skeleton is the one file's only one.
    let mut cats = people();
    let skeleton = cats.skeletons.get_mut(&ADULT).unwrap();
    skeleton.file = kat.into();
    skeleton.place = 0;
    skeleton.scales = vec![vec![5]];
    let listed = sims_vm::read_before_the_first_load(&cats);
    assert_eq!(listed.iter().collect::<Vec<_>>(), [kat]);
    // Read: its own copy only, one bone at 48 draws. Not read: the file's
    // skeleton too.
    for (files, draws) in [(listed, 48), (Default::default(), 96)] {
        let mut fx = lot();
        fx.world.skeleton_files = files;
        let done = read(&mut fx, &file(vec![sim(3, ADULT)]), &cats).unwrap();
        assert_eq!(done.draws, draws);
        assert!(fx.world.skeleton_files.contains(kat));
    }
}

#[test]
fn a_person_with_low_interests_gets_defaults_that_only_cost_draws() {
    // Ten interests and five personality words: each draws a category
    // until one has room, then a value. So 30 draws at least.
    for interest in [0, 10] {
        let mut fx = lot();
        fx.world.neighbors = records(interest);
        fx.world.crt_rng = CrtRng::from_state(99);
        let start = fx.world.crt_rng;
        let done = read(&mut fx, &file(vec![sim(3, ADULT)]), &people()).unwrap();
        let defaults = done.draws - 160 - 128;
        assert!((30..200).contains(&defaults), "{defaults}");
        assert_eq!(fx.world.crt_rng, advanced(start, done.draws));
        // The saved person data replaces the defaults.
        let data = &fx.world.objects[&3].person.as_ref().unwrap().data;
        assert_eq!(data[..80], person().data[..80]);
    }
    // The stream decides how many. The two counts were worked out apart
    // from this code, by a script written from the old prototype's
    // `default_traits`.
    for (state, draws) in [(99, DEFAULTS_FROM_99), (1, DEFAULTS_FROM_1)] {
        let mut fx = lot();
        fx.world.neighbors = records(0);
        fx.world.crt_rng = CrtRng::from_state(state);
        let done = read(&mut fx, &file(vec![sim(3, ADULT)]), &people()).unwrap();
        assert_eq!(done.draws - 288, draws, "from state {state}");
    }
    // A record without person data leaves the interests at zero.
    let mut fx = lot();
    fx.world.neighbors = records(500);
    let record = fx.world.neighbors.slots[0].as_mut().unwrap();
    record.revision = 0;
    record.data.fill(0);
    let done = read(&mut fx, &file(vec![sim(3, ADULT)]), &people()).unwrap();
    assert!(done.draws >= 288 + 30);
}
const DEFAULTS_FROM_99: u32 = 35;
const DEFAULTS_FROM_1: u32 = 42;

#[test]
fn people_the_read_cannot_construct_fail_loudly() {
    // No skeleton for the type.
    let mut fx = lot();
    let before = fx.world.clone();
    let mut missing = people();
    missing.skeletons.remove(&CHILD);
    let objects = file(vec![sim(3, ADULT), sim(2, CHILD)]);
    let (object, cause) = trap(read(&mut fx, &objects, &missing));
    assert_eq!(object, 2);
    assert!(matches!(cause, Trap::Unsupported(_)), "{cause:?}");
    // No neighborhood record: the original makes one.
    fx.world.neighbors.slots[0] = None;
    let (object, cause) = trap(read(&mut fx, &objects, &people()));
    assert_eq!(object, 3);
    assert!(matches!(cause, Trap::Unsupported(_)), "{cause:?}");
    // A record too short to hold the interests.
    fx.world.neighbors = records(11);
    fx.world.neighbors.slots[0]
        .as_mut()
        .unwrap()
        .data
        .truncate(50);
    let (_, cause) = trap(read(&mut fx, &objects, &people()));
    assert!(matches!(cause, Trap::Invalid(_)), "{cause:?}");
    fx.world.neighbors = records(11);
    // The save says person and the definition does not, or the reverse.
    let (_, cause) = trap(read(&mut fx, &file(vec![sim(3, CHAIR)]), &people()));
    assert!(matches!(cause, Trap::Invalid(_)), "{cause:?}");
    let (_, cause) = trap(read(&mut fx, &file(vec![saved(3, ADULT)]), &people()));
    assert!(matches!(cause, Trap::Invalid(_)), "{cause:?}");
    // The stream and the files read are as they were.
    assert_eq!(fx.world.encode(), before.encode());
}

/// A clip of 100 ms with one bound motion, and a motion with these events
/// (time, and whether it has a property).
fn clip(name: &str, events: &[(i32, bool)]) -> Clip {
    let tracks = if events.is_empty() {
        Vec::new()
    } else {
        vec![Track {
            bone: "ROOT".into(),
            events: events
                .iter()
                .map(|&(time, any)| Event {
                    time,
                    properties: if any {
                        vec![("xevt".into(), "1".into())]
                    } else {
                        Vec::new()
                    },
                })
                .collect(),
        }]
    };
    Clip {
        name: name.into(),
        skill: "loaded-skill".into(),
        duration: 0x42c8_0000,
        distance: 0,
        moving: false,
        frames: 4,
        motions: 1,
        tracks,
        channels: Vec::new(),
        samples: [0, 0],
    }
}

/// The lot, with the child's type unchanged since the save, so the load
/// does not reset the child (0x459bf6), at tick 88444, with a foreground
/// clip whose one event comes at 80 ms and a base clip without events
/// that moves ROOT.
fn unreset_lot() -> Fixture {
    let mut fx = lot();
    fx.world.changed_types.remove(&CHILD);
    fx.world.ticks = 88444;
    fx.program
        .animations
        .insert("test.far", clip("fg-clip", &[(80, true)]));
    // The base clip moves ROOT and only turns PELVIS.
    let mut base = clip("base-clip", &[]);
    base.channels = vec![
        sims_content::Channel {
            bone: "ROOT".into(),
            translation: Some(0),
            rotation: Some(0),
        },
        sims_content::Channel {
            bone: "PELVIS".into(),
            translation: None,
            rotation: Some(1),
        },
    ];
    fx.program.animations.insert("test.far", base);
    fx.program.animations.insert_samples("loaded-skill");
    fx
}

/// A child saved with the foreground and base layers at these phases (in
/// thousandths), a routing state, engaged and 7 events seen.
fn playing_child(foreground: i32, base: i32, routing_state: i32) -> Instance {
    let mut child = sim(2, CHILD);
    let person = child.person.as_mut().unwrap();
    person.animations = [
        format!("fg-clip;1;1000;{foreground};1000;1;0;0"),
        String::new(),
        format!("base-clip;-10;1000;{base};1000;1;0;0"),
    ];
    person.routing_state = routing_state;
    person.engaged = 1;
    person.animation_events = 7;
    child
}

#[test]
fn a_person_the_load_does_not_reset_plays_on_from_its_saved_layers() {
    // Its animator keeps what the restore read (0x5918ba): the layers at
    // the saved phases, updated once at the load's clock with no time
    // gone by (0x5992b3), the events seen (+0xa4), whether it plays
    // (+0xa0), and its routing state (+0x190) when that is 0 to 3.
    let bits = |f: F32| f.to_bits();
    for (saved, state) in [(0, 0), (2, 2), (4, 3), (-1, 3)] {
        let mut fx = unreset_lot();
        let objects = file(vec![sim(3, ADULT), playing_child(500, 250, saved)]);
        read(&mut fx, &objects, &people()).unwrap();
        let clock = LOADED.clock_at(88444);
        assert_ne!(clock, 0);
        let a = &fx.world.animators[&2];
        assert_eq!(a.state, state, "saved {saved}");
        assert!(a.playing);
        assert_eq!(a.seen, 7);
        // The reset sets +0x1a0; the constructor's 0 stays.
        assert!(!a.promote);
        assert_eq!((a.clock, a.skin_clock, a.head_clock), (clock, clock, clock));
        assert!(!a.skin_dirty);
        let fg = a.foreground.as_ref().unwrap();
        assert_eq!((fg.current, fg.previous), (clock, clock));
        assert_eq!(bits(fg.phase), 0.5f32.to_bits());
        // The event at 80 ms is still to come.
        assert_eq!((fg.event_time, fg.previous_event_time), (50, 50));
        assert_eq!((fg.cursors.clone(), fg.mode, fg.priority), (vec![0], 1, 1));
        assert!(a.carry.is_none());
        // The base layer waits in the base slot; the constructor's posture
        // is -1, so the first base update clears it.
        assert_eq!(a.base, Base::Loaded);
        let base = &fx.world.base_layers[&2];
        assert_eq!((base.current, base.event_time), (clock, 25));
        assert_eq!(bits(base.phase), 0.25f32.to_bits());
        // Placed on the person's tile, its destination there too.
        assert_eq!(a.destination, a.position);
        // The skin's update at load posed it with the base layer, which
        // moved ROOT (0x5992b3, 0x598226): the bone counts as moved since
        // the skin was made.
        assert_eq!(
            fx.world.translated_bones.get(&2),
            Some(&std::collections::BTreeSet::from(["ROOT".to_owned()]))
        );
        assert!(!fx.world.translated_bones.contains_key(&3));
        // Nothing is held for a skin the load did not reset.
        assert!(!fx.world.held_poses.contains_key(&2));
        // The adult, whose type changed, is reset as before.
        assert!(fx.world.animators[&3].promote);
    }
}

#[test]
fn a_saved_layer_the_restore_cannot_play_fails_loudly() {
    // A foreground layer saved past its event: the update at load reaches
    // it before the person's data words are read.
    let mut fx = unreset_lot();
    let before = fx.world.clone();
    let objects = file(vec![sim(3, ADULT), playing_child(900, 250, 3)]);
    let (object, cause) = trap(read(&mut fx, &objects, &people()));
    assert_eq!(object, 2);
    assert!(
        matches!(&cause, Trap::Unsupported(text) if text.contains("0x5992b3")),
        "{cause:?}"
    );
    assert_eq!(fx.world.encode(), before.encode());

    // A base clip with timed events.
    let mut fx = unreset_lot();
    let mut child = playing_child(500, 250, 3);
    child.person.as_mut().unwrap().animations[2] = "fg-clip;-10;1000;0;1000;1;0;0".into();
    let (object, cause) = trap(read(&mut fx, &file(vec![child]), &people()));
    assert_eq!(object, 2);
    assert!(
        matches!(&cause, Trap::Unsupported(text) if text.contains("timed events")),
        "{cause:?}"
    );

    // A clip our content lacks.
    let mut fx = unreset_lot();
    let mut child = playing_child(500, 250, 3);
    child.person.as_mut().unwrap().animations[0] = "no-such-clip;1;1000;0;1000;1;0;0".into();
    let (_, cause) = trap(read(&mut fx, &file(vec![child]), &people()));
    assert!(
        matches!(&cause, Trap::Unsupported(text) if text.contains("not found")),
        "{cause:?}"
    );
}

#[test]
fn the_load_restores_the_head_as_saved_and_relaxes_its_turn_after_tick_0() {
    // The saved heading, target heading, turn and target point (0x5918ba).
    // The target point is kept: a measurement whose target is gone reads
    // it (0x5906bc). The constructor zeroed the head clock and set the
    // animator's clock to the clock at the load's tick count, so the
    // head's step relaxes the turn unless that is 0 (0x590788).
    let bits = |f: sims_core::F32| f.to_bits();
    for (ticks, relaxed) in [(0, false), (88444, true)] {
        let mut fx = lot();
        fx.world.ticks = ticks;
        let mut adult = sim(3, ADULT);
        adult.person.as_mut().unwrap().floats = [1.0, 2.0, 1.0, 70.0, 16.0, 80.0, 0.0, 0.0, 0.0];
        read(&mut fx, &file(vec![adult]), &people()).unwrap();
        let head = fx.world.animators[&3].head;
        let mut turn = sims_sim::head::Head {
            turn: sims_core::F32::from_f32(1.0),
            ..sims_sim::head::Head::LOADED
        };
        if relaxed {
            turn.relax();
        }
        assert_ne!(turn.turn.to_bits(), 0);
        assert_eq!(
            (
                bits(head.heading),
                bits(head.target_heading),
                bits(head.turn)
            ),
            (1.0f32.to_bits(), 2.0f32.to_bits(), bits(turn.turn)),
            "tick {ticks}"
        );
        assert_eq!(
            head.target.map(|t| t.map(bits)),
            Some([70.0f32, 16.0, 80.0].map(f32::to_bits))
        );
    }
}

#[test]
fn a_person_saved_on_a_route_gets_as_many_empty_contexts_as_it_counted() {
    // 0x460443: the save keeps only the count; each context is a copy of
    // one the default constructor makes (0x4815f1, 0x46dd48, 0x481693).
    let mut fx = lot();
    let mut walking = sim(3, ADULT);
    walking.person.as_mut().unwrap().routing_frames = 2;
    let objects = file(vec![walking, sim(5, ADULT)]);
    read(&mut fx, &objects, &people()).unwrap();
    let contexts = &fx.world.routing.routes[&3].contexts;
    assert_eq!(contexts.len(), 2);
    assert_eq!(contexts[0], contexts[1]);
    let c = &contexts[0];
    assert!(c.candidates.is_empty());
    assert_eq!((c.target, c.person, c.chosen, c.best), (0, 0, -1, -1));
    assert_eq!(
        (c.slot.number, c.slot.snap_target, c.slot.facing),
        (-1, -1, -2)
    );
    assert_eq!((c.slot.gradient, c.slot.flags), (0x3e40_0000, 0));
    assert_eq!(
        (c.byte_78, c.limit, c.saved, c.word_94),
        (true, 0x30, [0; 2], 0)
    );
    // A person that counted none has none.
    assert!(!fx.world.routing.routes.contains_key(&5));

    // A negative count fails, and the lot is unchanged.
    let mut fx = lot();
    let before = fx.world.clone();
    let mut odd = sim(3, ADULT);
    odd.person.as_mut().unwrap().routing_frames = -1;
    let (object, cause) = trap(read(&mut fx, &file(vec![odd]), &people()));
    assert_eq!(object, 3);
    assert!(
        matches!(&cause, Trap::Unsupported(text) if text.contains("0x460443")),
        "{cause:?}"
    );
    assert_eq!(fx.world.encode(), before.encode());
}
