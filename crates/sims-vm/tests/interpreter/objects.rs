//! Load step 6e: saved objects are constructed, then read.

use crate::support::*;
use sims_content::{People, Skeleton};
use sims_core::CrtRng;
use sims_formats::saved_objects::{Instance, Multipart, Objects, Slot};
use sims_sim::neighbors::{Neighbor, Neighbors};
use sims_vm::{Fault, SavedRead, Trap};
use std::collections::BTreeMap;

const CHAIR: u32 = 0x5001;
const ADULT: u32 = 0x5002;
const CHILD: u32 = 0x5003;
const STOOL: u32 = 0x5004;

/// A cleared lot whose catalog has a chair (12 attributes, 3 dynamic
/// sprites), two person types, and a stool whose definition asks for
/// nothing.
fn lot() -> Fixture {
    let mut fx = Fixture::new();
    fx.world.objects.clear();
    fx.world.order.clear();
    let chair = fx.add_type(CHAIR, 4, 0, 4097);
    fx.words(chair)[58] = 12;
    fx.words(chair)[65] = 3;
    fx.add_type(ADULT, 2, 0, 4097);
    fx.add_type(CHILD, 2, 0, 4097);
    fx.add_type(STOOL, 4, 0, 4097);
    fx.world.neighbors = records(11);
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
        profiles: BTreeMap::new(),
        skeletons: BTreeMap::from([
            (
                ADULT,
                Skeleton {
                    name: "adult".into(),
                    file: "a.bcf".into(),
                    place: 1,
                    // A scale of 1 costs a bone 32 draws, any other 48.
                    scales: vec![vec![1], vec![1, 7, 0]],
                },
            ),
            (
                CHILD,
                Skeleton {
                    name: "child".into(),
                    file: "c.bcf".into(),
                    place: 0,
                    scales: vec![vec![5]],
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
