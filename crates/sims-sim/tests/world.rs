use sims_content::{Catalog, House, Index, Install, Neighborhood};
use sims_core::{CrtRng, F32, GameRng};
use sims_formats::lot::Array;
use sims_formats::saved_objects::{Frame, Slot};
use sims_sim::animator::{Animator, Base, Fade, HeldPose, LOADED, Layer, Motion, UNSET};
use sims_sim::characters::{Character, Characters};
use sims_sim::head::Head;
use sims_sim::neighbors::{Family, Inventory, Item, Neighbor, Neighbors, Row};
use sims_sim::rooms::{Half, Room, Rooms};
use sims_sim::route::{Candidate, Context, Route, RouteSlot, Routing};
use sims_sim::{
    Budget, Dialog, Dialogs, FrameStorage, Headline, HouseUsage, MagicHistory, Object, PersonTick,
    ReadOnlySnapshots, Snapshots, TypeAttributes, Usage, Whereabouts, World,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn object(id: u16, guid: u32) -> Object {
    Object {
        id,
        guid,
        footprint: [16, 16, 32, 32],
        position: [24, 24, 1],
        unknown_short: 1,
        attributes: vec![3, -4],
        temps: vec![0; 8],
        data: (0..72).collect(),
        obsolete_word: 0,
        stack_capacity_bytes: 512,
        stack: vec![Frame {
            stack_object: id as i16,
            tree: 4096,
            node: 3,
            parameters: vec![1, 2, 3, 4],
            locals: vec![-1],
            primitive_state: 0,
            code_owner_type: 7,
        }],
        relationship_flag: -1,
        relationships: Vec::new(),
        slots: vec![Slot {
            unknown: 0,
            object: 0,
        }],
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: Some(vec![0.5, -0.0, f32::from_bits(0x7fc0_0001)]),
        person: None,
        registry_word: Some(0x0002_0005),
        last_result: true,
        removal_skips_resets: false,
        selected: id == 5,
    }
}

fn world() -> World {
    World {
        ticks: 88444,
        globals: (0..32).map(|n| n * 3 - 40).collect(),
        scheduler_cursor: 0,
        lot_rng: GameRng::from_state(0x1234_5678),
        global_rng: GameRng::from_state(0x9abc_def0),
        crt_rng: CrtRng::from_state(0x0733_3a41),
        size: 52,
        budget: Budget {
            funds: 2515,
            pending_funds: 0,
            lot_value: 100,
            objects_value: 200,
            architecture_value: 300,
            days: vec![None, Some([1, 2, 3, 4, 5, 6, 7, 8]), None],
        },
        order: vec![5, 2],
        people: vec![5],
        types: BTreeMap::from([(7, 0x1111), (8, 0x2222)]),
        objects: BTreeMap::from([(2, object(2, 0x1111)), (5, object(5, 0x2222))]),
        arrays: BTreeMap::from([(
            1,
            Array {
                width: 2,
                height: 2,
                stride: 1,
                encoding: 8,
                data: vec![1, 2, 3, 4],
            },
        )]),
        removals: vec![5],
        type_attributes: vec![TypeAttributes {
            guid: 0x1111,
            values: vec![0, 1, -1],
        }],
        instances: BTreeMap::from([(0x2222, vec![5])]),
        event_owner: 2,
        magic_history: Some(MagicHistory {
            family: 5,
            values: [1, 2, 3, 4, 5, 6],
        }),
        indicators: [2, 0],
        changed_types: BTreeSet::from([0x2222]),
        next_parts: BTreeMap::from([(2, 5)]),
        skeleton_files: BTreeSet::from(["adult.bcf".to_owned()]),
        rooms: Rooms {
            map: [vec![0, -5, 1, -1], vec![1024]],
            halves: BTreeMap::from([(
                [1, 0, 1],
                [
                    Half {
                        triangle: 4,
                        room: 0,
                    },
                    Half {
                        triangle: 2,
                        room: -1,
                    },
                ],
            )]),
            rooms: BTreeMap::from([(
                1,
                Room {
                    tiles: vec![[0, 2, 1], [0, 2, 1]],
                    outdoors: false,
                    dirty: true,
                    half_tiles: 4,
                    covered_half_tiles: 2,
                    walls: 3,
                    covered_walls: 1,
                    objects: 7,
                    window_light: 50,
                    lamp_light: 25,
                    word_39: -2,
                    impact: -10,
                    good: 6,
                    category_4: 1,
                    category_5_to_7: 2,
                    lights: 3,
                    people: 1,
                    score: F32::from_f32(-12.5),
                    light: F32::from_f32(0.25),
                    pool: true,
                    occupied: true,
                },
            )]),
            pending: 2,
            outdoor_sum: F32::from_f32(3.0),
            outdoor_score: F32::from_f32(1.0),
        },
        fires: vec![7],
        frame_storage: BTreeMap::from([(
            2,
            FrameStorage {
                words: vec![Some(4096), Some(-1), None, Some(0)],
                offsets: vec![0],
                unknown: BTreeSet::from([(0, 1)]),
                restarts: 0,
            },
        )]),
        dialogs: Dialogs {
            records: vec![Dialog {
                caller: 5,
                operands: [0, 0, 0, 0, 0, 1, 0, 0x10],
                status: 2,
            }],
            declined: -1,
        },
        person_ticks: BTreeMap::from([(
            5,
            PersonTick {
                last_decay: 88440,
                room: 0,
            },
        )]),
        neighbors: Neighbors {
            slots: vec![
                None,
                Some(Neighbor {
                    id: 2,
                    guid: 0x2222,
                    revision: 9,
                    data: (0..256).map(|n| n - 100).collect(),
                    relationships: vec![Row {
                        target: -7,
                        values: vec![50, -3],
                    }],
                    friends: 1,
                    stale: true,
                }),
            ],
            inventories: vec![Inventory {
                neighbor: 2,
                items: vec![Item {
                    kind: 6,
                    guid: 0x4e56_2938,
                    count: 3,
                }],
            }],
            relationships_changed: true,
        },
        advertisement_record: Some(std::array::from_fn(|i| sims_sim::Advertisement {
            personality: i as i16,
            minimum: -(i as i16),
            delta: 100 + i as i16,
        })),
        families: vec![
            Family {
                id: 5,
                house: 5,
                members: vec![0x1234_5678, 0x9abc_def0],
                friends: 3,
                stale: false,
            },
            Family {
                id: 0,
                house: 0,
                members: Vec::new(),
                friends: -1,
                stale: true,
            },
        ],
        headlines: BTreeMap::from([(
            5,
            Headline {
                priority: 2,
                duration: -1,
            },
        )]),
        animators: BTreeMap::from([(5, animator())]),
        usage: HouseUsage {
            history: [
                Usage {
                    total: 1200,
                    routing: 40,
                },
                Usage::default(),
                Usage::default(),
                Usage {
                    total: -1,
                    routing: i32::MIN,
                },
                Usage::default(),
            ],
            today: Usage {
                total: 70000,
                routing: 3,
            },
        },
        routing: routing(),
        action_serial: 9,
        base_layers: BTreeMap::from([(5, base_layer())]),
        translated_bones: BTreeMap::from([(
            5,
            BTreeSet::from(["PELVIS".to_owned(), "ROOT".to_owned()]),
        )]),
        neighborhood_data: [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -6, 0],
        held_poses: BTreeMap::from([(
            6,
            HeldPose {
                layers: vec![base_layer()],
                position: [
                    F32::from_bits(0x4390_0000),
                    F32::from_bits(0x43e4_0000),
                    F32::from_bits(0x40c0_0000),
                ],
            },
        )]),
        stops_asked: BTreeSet::from([5]),
        characters: Characters {
            made: BTreeMap::from([(
                0x1234_0042,
                Character {
                    template: 0x7fd9_6b54,
                    file: 3,
                    strings: BTreeMap::from([(0, "child".into()), (14, "med".into())]),
                },
            )]),
            next_file: 4,
        },
        portraits: BTreeSet::from([0x1234_0042, 0x7fd9_6b54]),
    }
}

/// Person 5's base slot's layer: a posture clip that repeats, at the base
/// slot's priority, part way through.
fn base_layer() -> Layer {
    let mut layer = Layer::start(
        "a2o-standing-loop",
        F32::from_bits(0x4502_0000),
        64,
        true,
        3,
        false,
        4_422_100,
    );
    layer.mode = 1;
    layer.priority = -10;
    layer.phase = F32::from_bits(0x3f00_0000);
    layer
}

/// A route of person 5 to slot 0 of object 2, part way through.
fn routing() -> Routing {
    let slot = RouteSlot {
        offset: [0, 0xc0b6_6666, 0],
        number: 1,
        standing: 10,
        sitting: 0,
        ground: 0,
        flags: 0x800,
        snap_target: -1,
        proximity: [16, 16, 16],
        gradient: 0x3e40_0000,
        facing: -2,
        resolution: 16,
    };
    Routing {
        routes: BTreeMap::from([(
            5,
            Route {
                contexts: vec![Context {
                    candidates: vec![Candidate {
                        position: [408, 429],
                        score: 100,
                        word_c: 0,
                        word_e: None,
                    }],
                    slot,
                    target: 2,
                    person: 5,
                    chosen: 0,
                    best: 100,
                    word_58: 0,
                    time: 0,
                    saved: [24, 664],
                    word_68: 0,
                    start: [24, 664, 0],
                    byte_78: true,
                    objects: [0; 2],
                    byte_84: false,
                    result: 0,
                    word_8c: 0,
                    byte_8e: false,
                    word_94: 0,
                    limit: 8,
                    action: 4,
                }],
                path: vec![[24, 664], [76, 656], [408, 413]],
            },
        )]),
        slot_flags: BTreeMap::from([((2, 0), 0x4000)]),
        unknown_words: BTreeSet::from([(5, 37)]),
        slots: BTreeMap::from([(
            (7, 0),
            RouteSlot {
                flags: 0x2144,
                ..slot
            },
        )]),
    }
}

/// An animator playing a clip backward, with an event queued, its walk
/// timed to stop and its clip fading. The posture, base layer and
/// destination are the values [`LOADED`] held before snapshot format 12,
/// so that the format 11 digests still apply.
fn animator() -> Animator {
    let mut layer = Layer::start(
        "clip",
        F32::from_bits(0x4479_c000),
        30,
        true,
        0,
        true,
        4_422_300,
    );
    layer.phase = F32::from_bits(0x3d4d_0148);
    layer.event_time = 50;
    layer.interruptible = 0;
    layer.fade = Fade {
        start: 4_422_250,
        length: 274,
        from: F32::from_bits(0x3f80_0000),
        to: F32::from_bits(0x3e9a_6c1c),
    };
    let mut carry = Layer::start(
        "carry",
        F32::from_bits(0x447a_0000),
        12,
        true,
        1,
        false,
        4_422_250,
    );
    carry.mode = 1;
    carry.priority = 10;
    carry.cursors = vec![1];
    Animator {
        carry: Some(carry),
        carry_first: true,
        clock: 4_422_300,
        rate: F32::from_bits(0x3fb3_3333),
        state: 0,
        arrival: -1,
        position: [
            F32::from_bits(0x43dc_0000),
            F32::from_bits(0x43b4_0000),
            F32::ZERO,
        ],
        destination: [UNSET; 3],
        posture: 0,
        base: Base::Posture,
        events: vec![7, -2],
        foreground: Some(layer),
        playing: true,
        motion: Motion {
            segment: 1,
            progress: F32::from_bits(0x3c9c_0000),
            turn_from: [
                F32::from_bits(0x4426_0000),
                F32::from_bits(0x41c0_0000),
                F32::ZERO,
            ],
            turn_destination: [UNSET; 3],
            turn_wanted: F32::from_bits(0xc060_0000),
            turn_clip: F32::from_bits(0xc080_0000),
            step: [F32::from_bits(0x3f00_0000), F32::from_bits(0xbe80_0000)],
            near_end: 1,
            finishing: 0,
            timer: 4_437_050,
        },
        head: Head {
            heading: F32::from_bits(0x3e6d_fb3c),
            target_heading: F32::from_bits(0x3fa8_c336),
            turn: F32::from_bits(0xbe4c_cccd),
            target: Some([
                F32::from_bits(0x425a_4885),
                F32::from_bits(0x40f2_77fc),
                F32::from_bits(0x42d0_acac),
            ]),
        },
        ..LOADED
    }
}

#[test]
fn the_hash_is_the_same_on_every_platform() {
    // Pinned: CI runs this on x86_64 Linux and aarch64 macOS. If the
    // encoding changes on purpose, bump snapshot::FORMAT and update this.
    assert_eq!(world().hash_hex(), PINNED);
}
const PINNED: &str = "20fd811dcc1faebf0012fb041c894f499a0306c53d0cd7705f7530b18ea3b55f";

#[test]
fn without_the_portraits_the_encoding_is_format_30() {
    // Snapshot format 31 added the person types whose portrait the
    // interface has built at the end.
    assert_eq!(digest(&format_30_bytes(&world())), PINNED_30);
}
const PINNED_30: &str = "76bb018abadfef18d5ac5e7d47a751fd554b6d0a8bdef220d8fb05899c4f45c6";

#[test]
fn without_the_made_characters_the_encoding_is_format_29() {
    // Snapshot format 30 added the person types made while the game runs
    // at the end.
    assert_eq!(digest(&format_29_bytes(&world())), PINNED_29);
}
const PINNED_29: &str = "f1663444bf686193967fab0511a75eaad4599c03369d188260cc34f84ee1f9ae";

/// The encoding of `world` as snapshot format 30 wrote it: without the
/// person types whose portrait the interface has built
/// ([`sims_sim::World::portraits`]) that format 31 added at the end. The
/// older formats' encodings start from this one.
fn format_30_bytes(world: &World) -> Vec<u8> {
    let mut bytes = world.encode();
    let built = postcard::to_allocvec(&world.portraits).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - built.len()), built);
    bytes
}

/// The encoding of `world` as snapshot format 29 wrote it: without the
/// person types made while the game runs ([`World::characters`]) that
/// format 30 added at the end. The older formats' encodings start from
/// this one.
fn format_29_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_30_bytes(world);
    let made = postcard::to_allocvec(&world.characters).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - made.len()), made);
    bytes
}

#[test]
fn without_the_stops_asked_the_encoding_is_format_28() {
    // Snapshot format 29 added the people asked to stop their route at the
    // end.
    assert_eq!(digest(&format_28_bytes(&world())), PINNED_28);
}
const PINNED_28: &str = "1e854596699af21756dd277d321f3e0068d1f9159afa9b974e6c364ee47aa1ee";

/// The encoding of `world` as snapshot format 28 wrote it: without the
/// people asked to stop their route ([`World::stops_asked`]) that format
/// 29 added at the end either. It starts from [`format_29_bytes`], and the
/// older formats' encodings from this one.
fn format_28_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_29_bytes(world);
    let asked = postcard::to_allocvec(&world.stops_asked).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - asked.len()), asked);
    bytes
}

#[test]
fn without_the_held_poses_the_encoding_is_format_27() {
    // Snapshot format 28 added the poses the load left in the skins at
    // the end.
    assert_eq!(digest(&format_27_bytes(&world())), PINNED_27);
}
const PINNED_27: &str = "60918414b09f2283b3f219c0cf7023f405d55754046afe86b1594a0cb1237b8f";

#[test]
fn without_the_neighborhood_data_the_encoding_is_format_26() {
    // Snapshot format 27 added the neighborhood's data words at the end.
    assert_eq!(digest(&format_26_bytes(&world())), PINNED_26);
}
const PINNED_26: &str = "d550ed76e8971864d5c06fe3fbcb810ff283b751659697240ff30668ddedd13e";

/// The encoding of `world` as snapshot format 27 wrote it: without the
/// poses the load left in the skins ([`World::held_poses`]) that format 28
/// added at the end. It starts from [`format_28_bytes`], and the older
/// formats' encodings from this one.
fn format_27_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_28_bytes(world);
    let held = postcard::to_allocvec(&world.held_poses).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - held.len()), held);
    bytes
}

/// The encoding of `world` as snapshot format 26 wrote it: without the
/// neighborhood's data words ([`World::neighborhood_data`]) that format
/// 27 added at the end. It starts from [`format_27_bytes`], and the older
/// formats' encodings from this one.
fn format_26_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_27_bytes(world);
    let data = postcard::to_allocvec(&world.neighborhood_data).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - data.len()), data);
    bytes
}

#[test]
fn without_the_bones_moved_the_encoding_is_format_25() {
    // Snapshot format 26 added the bones the skins moved at the end.
    assert_eq!(digest(&format_25_bytes(&world())), PINNED_25);
}
const PINNED_25: &str = "f44771b305c9f7af1aaf760cb475f98f1a815bff357191a399bd14a79b35e85b";

/// The encoding of `world` as snapshot format 25 wrote it: without the
/// bones the skins moved ([`World::translated_bones`]) that format 26
/// added at the end. It starts from [`format_26_bytes`], and the older
/// formats' encodings from this one.
fn format_25_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_26_bytes(world);
    let moved = postcard::to_allocvec(&world.translated_bones).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - moved.len()), moved);
    bytes
}

#[test]
fn without_the_base_layers_the_encoding_is_format_24() {
    // Snapshot format 25 added the base slots' layers at the end.
    assert_eq!(digest(&format_24_bytes(&world())), PINNED_24);
}
const PINNED_24: &str = "992789f0d0ba45f0e3a203b4cdff77c9b180f5a1da7b99061e521ca5ae5b3d38";

/// The encoding of `world` as snapshot format 24 wrote it: without the
/// base slots' layers ([`World::base_layers`]) that format 25 added at
/// the end. It starts from [`format_25_bytes`], and the older formats'
/// encodings from this one.
fn format_24_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_25_bytes(world);
    let layers = postcard::to_allocvec(&world.base_layers).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - layers.len()), layers);
    bytes
}

#[test]
fn without_the_headlines_the_encoding_is_format_23() {
    // Snapshot format 24 added the headlines at the end.
    assert_eq!(digest(&format_23_bytes(&world())), PINNED_23);
}
const PINNED_23: &str = "d73a66741337b2527a28c856d5d94f1a91a6f7137a7ee199fcfc289bd21dcadb";

/// The encoding of `world` as snapshot format 23 wrote it: without the
/// headlines ([`World::headlines`]) that format 24 added at the end. It
/// starts from [`format_24_bytes`], and the older formats' encodings from
/// this one.
fn format_23_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_24_bytes(world);
    let headlines = postcard::to_allocvec(&world.headlines).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - headlines.len()), headlines);
    bytes
}

#[test]
fn without_the_families_the_encoding_is_format_22() {
    // Snapshot format 23 added the families at the end.
    assert_eq!(digest(&format_22_bytes(&world())), PINNED_22);
}
const PINNED_22: &str = "a4e74a05e62ae6eaff009cb2b32fe752b07cc739492dc1d964c585d53e0d2d65";

/// The encoding of `world` as snapshot format 22 wrote it: without the
/// families ([`World::families`]) that format 23 added at the end. It
/// starts from [`format_23_bytes`], and the older formats' encodings from
/// this one.
fn format_22_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_23_bytes(world);
    let families = postcard::to_allocvec(&world.families).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - families.len()), families);
    bytes
}

#[test]
fn without_the_advertisement_record_the_encoding_is_format_21() {
    // Snapshot format 22 added the advertisement record at the end.
    assert_eq!(digest(&format_21_bytes(&world())), PINNED_21);
}
const PINNED_21: &str = "23e9f53ef9c6129ef25795acb31d097cce39ddb0401b852212b4a4f96e49cdcd";

/// The encoding of `world` as snapshot format 21 wrote it: without the
/// advertisement record ([`World::advertisement_record`]) that format 22
/// added at the end. It starts from [`format_22_bytes`], and the older
/// formats' encodings from this one.
fn format_21_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_22_bytes(world);
    let record = postcard::to_allocvec(&world.advertisement_record).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - record.len()), record);
    bytes
}

#[test]
fn the_base_slot_encodes_as_the_flag_it_was_unless_a_script_fills_it() {
    // An empty slot and the posture's clip encode as false and true did,
    // so format 21 needs no new version for them.
    let bytes = |result: Result<Vec<u8>, postcard::Error>| result.unwrap();
    assert_eq!(
        bytes(postcard::to_allocvec(&Base::Empty)),
        bytes(postcard::to_allocvec(&false))
    );
    assert_eq!(
        bytes(postcard::to_allocvec(&Base::Posture)),
        bytes(postcard::to_allocvec(&true))
    );
    // A script's clip comes back as it went.
    let mut world = world();
    world.animators.get_mut(&5).unwrap().base = Base::Script {
        clip: "a2o-base".into(),
        reverse: true,
        start: 1234,
    };
    let back = World::decode(&world.encode()).unwrap();
    assert_eq!(back.animators, world.animators);
    assert_eq!(back.base_layers, world.base_layers);
    assert_eq!(back.hash(), world.hash());
}

#[test]
fn without_the_carry_layer_the_encoding_is_format_20() {
    // Snapshot format 21 added at the end of each animator its carry layer
    // and where that stands in the skin's list.
    assert_eq!(digest(&format_20_bytes(&world())), PINNED_20);
}
const PINNED_20: &str = "cde80db397840aa1122cd14801bcaf618d6476626e4aea7b29310e569134fdc5";

/// An animator's encoding under snapshot format 20: without the carry
/// layer ([`Animator::carry`]) and its place in the skin's list
/// ([`Animator::carry_first`]) that format 21 added at its end.
fn animator_20_bytes(animator: &Animator) -> Vec<u8> {
    let mut bytes = postcard::to_allocvec(animator).unwrap();
    let added = postcard::to_allocvec(&(&animator.carry, animator.carry_first)).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - added.len()), added);
    bytes
}

/// The encoding of `world` as snapshot format 20 wrote it: each animator
/// as [`animator_20_bytes`] writes it. It starts from
/// [`format_21_bytes`], and the older formats' encodings from this one.
fn format_20_bytes(world: &World) -> Vec<u8> {
    let blank = World {
        animators: BTreeMap::new(),
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

#[test]
fn without_the_person_vector_the_encoding_is_format_19() {
    // Snapshot format 20 added the person vector after the registration
    // order.
    assert_eq!(digest(&format_19_bytes(&world())), PINNED_19);
}
const PINNED_19: &str = "e4a24ddfd65d8ccb54cc45b7d6f6c49ef4f1b06413842308c1977c72e13cd064";

/// The encoding of `world` as snapshot format 19 wrote it: without the
/// person vector ([`World::people`]) that format 20 added after the
/// registration order. It starts from [`format_20_bytes`], and the older
/// formats' encodings from this one.
fn format_19_bytes(world: &World) -> Vec<u8> {
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

#[test]
fn without_the_action_and_the_slot_records_the_encoding_is_format_18() {
    // Snapshot format 19 added each route context's action asking a
    // person to move (+0x90) and the slot records objects hold.
    assert_eq!(digest(&format_18_bytes(&world())), PINNED_18);
}
const PINNED_18: &str = "52685edd0474b28f07deafcc89e3f8a34431542a20468adb23258ab344f0230b";

/// An animator's encoding under snapshot format 17: as
/// [`animator_20_bytes`] writes it, and without the head fields
/// ([`Animator::head`]) that format 18 added at its end.
fn animator_17_bytes(animator: &Animator) -> Vec<u8> {
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

/// A lot's encoding as snapshot format 18 wrote it. Format 19 added at
/// the end of each route context the number of the action that asks a
/// person to move out of the way, and at the end of the routing the slot
/// records objects hold in place of their type's. This writes the
/// routing again without them, so a state pinned under format 18 can be
/// shown to be the same state but for those fields. It starts from
/// [`format_19_bytes`], and the older formats' encodings from this one.
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

/// The encoding of `world` as snapshot format 17 wrote it. Format 18
/// added at the end of each animator the head's fields
/// ([`Animator::head`]). This writes the animators again without them.
fn format_17_bytes(world: &World) -> Vec<u8> {
    let blank = World {
        animators: BTreeMap::new(),
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

#[test]
fn without_the_head_the_encoding_is_format_17() {
    // Snapshot format 18 added the head's angles, turn and target.
    assert_eq!(digest(&format_17_bytes(&world())), PINNED_17);
}
const PINNED_17: &str = "85b353ea24114679b2ff1796bcce33b558e8c41f511c199752a0e5b22858f875";

/// `whole` with `part` replaced by `old`. `part` is found where `blank`
/// stands in `whole_blank`, the same encoding with that one field (and
/// nothing else) at its default value: the bytes before and after it are
/// the same in both.
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
    map: &BTreeMap<K, V>,
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
/// ([`Motion::timer`]) that format 17 added at its end.
fn motion_16_bytes(motion: &Motion) -> Vec<u8> {
    let mut bytes = postcard::to_allocvec(motion).unwrap();
    let added = postcard::to_allocvec(&motion.timer).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - added.len()), added);
    bytes
}

/// A motion's encoding under snapshot format 15: also without the last
/// step and the two end-of-path flags at its end.
fn motion_15_bytes(motion: &Motion) -> Vec<u8> {
    let mut bytes = motion_16_bytes(motion);
    let added = postcard::to_allocvec(&(motion.step, motion.near_end, motion.finishing)).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - added.len()), added);
    bytes
}

/// A foreground layer's encoding under snapshot format 16: without the
/// fade ([`Layer::fade`]) that format 17 added at its end.
fn layer_16_bytes(layer: &Option<Layer>) -> Vec<u8> {
    let mut bytes = postcard::to_allocvec(layer).unwrap();
    if let Some(layer) = layer {
        let added = postcard::to_allocvec(&layer.fade).unwrap();
        assert_eq!(bytes.split_off(bytes.len() - added.len()), added);
    }
    bytes
}

/// An animator's encoding with its motion written by `motion` and its
/// foreground layer by [`layer_16_bytes`].
fn animator_old_bytes(animator: &Animator, motion: fn(&Motion) -> Vec<u8>) -> Vec<u8> {
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

/// An animator's encoding under snapshot format 15: its motion without
/// the last step and the two end-of-path flags, which format 16 added at
/// the end of [`Motion`], and without format 17's fields.
fn animator_15_bytes(animator: &Animator) -> Vec<u8> {
    animator_old_bytes(animator, motion_15_bytes)
}

/// The encoding of `world` as snapshot format 16 wrote it. Format 17
/// added at the end of each animator's motion the timer of a walk that
/// slows to a stop ([`Motion::timer`]) and at the end of each foreground
/// layer its fade ([`Layer::fade`]). This writes the animators again
/// without those bytes.
fn format_16_bytes(world: &World) -> Vec<u8> {
    let blank = World {
        animators: BTreeMap::new(),
        ..world.clone()
    };
    splice(
        &format_18_bytes(world),
        &format_18_bytes(&blank),
        &map_bytes(&world.animators, animator_20_bytes),
        &[0],
        &map_bytes(&world.animators, |a| animator_old_bytes(a, motion_16_bytes)),
    )
}

#[test]
fn without_the_timer_and_fades_the_encoding_is_format_16() {
    // Snapshot format 17 added the motion's timer and the layer's fade.
    assert_eq!(digest(&format_16_bytes(&world())), PINNED_16);
}
const PINNED_16: &str = "794f72b969d9d0351f10ee7d8aca0b0177c29f5704bdb79e641c76ff2c8eaa6d";

/// The encoding of `world` with each animator as format 15 wrote it.
fn without_motion_steps(world: &World) -> Vec<u8> {
    let blank = World {
        animators: BTreeMap::new(),
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

/// The encoding of `world` as snapshot format 15 wrote it. Format 16
/// added two things: at the end of each room whether it knows a person is
/// in it ([`Room::occupied`]), and at the end of each animator's motion
/// its last step and two end-of-path flags ([`Motion::step`],
/// [`Motion::near_end`], [`Motion::finishing`]). This writes them again
/// without those bytes.
fn format_15_bytes(world: &World) -> Vec<u8> {
    let rooms = &world.rooms;
    let blank_rooms = Rooms {
        rooms: BTreeMap::new(),
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

#[test]
fn without_room_marks_or_last_steps_the_encoding_is_format_15() {
    // Snapshot format 16 added the room's mark that a person is in it and
    // the animator's last step and end-of-path flags.
    assert_eq!(digest(&format_15_bytes(&world())), PINNED_15);
}
const PINNED_15: &str = "f72290e87a9e96c2436bb657cdf83806bb38760885f45f9b0d6a53d1f12a19c3";

/// The encoding of `world` as snapshot format 14 wrote it. Format 15
/// added the last number given to a running action
/// ([`World::action_serial`]) at the end. This drops its bytes.
fn format_14_bytes(world: &World) -> Vec<u8> {
    let mut bytes = format_15_bytes(world);
    let serial = postcard::to_allocvec(&world.action_serial).unwrap();
    assert_eq!(bytes.split_off(bytes.len() - serial.len()), serial);
    bytes
}

#[test]
fn without_the_action_serial_the_encoding_is_format_14() {
    // Snapshot format 15 added the last number given to a running action.
    assert_eq!(digest(&format_14_bytes(&world())), PINNED_14);
}
const PINNED_14: &str = "0caafc6a30ff53da1bd3e8d59d36fc08216033a2c6c12304b89a32580939a1b5";

/// The encoding of `world` as snapshot format 13 wrote it. Format 14
/// added three fields: at the end of an animator whether the house view has
/// drawn its person yet ([`Animator::undrawn`]) and whether it walks
/// ([`Animator::walking`]), and at the end of a layer
/// where its clip's timed events stand ([`Layer::cursors`], one zero byte
/// when there are none). The animators come last but for the usage counts
/// and the routing. This writes them again without those bytes.
fn format_13_bytes(world: &World) -> Vec<u8> {
    let mut old = world.clone();
    old.animators.clear();
    let mut bytes = format_14_bytes(&old);
    let mut tail = postcard::to_allocvec(&world.usage).unwrap();
    tail.extend(routing_18_bytes(&world.routing));
    assert_eq!(bytes.split_off(bytes.len() - tail.len()), tail);
    assert_eq!(bytes.pop(), Some(0));
    bytes.extend(postcard::to_allocvec(&(world.animators.len() as u64)).unwrap());
    for (id, animator) in &world.animators {
        bytes.extend(postcard::to_allocvec(id).unwrap());
        bytes.extend(animator_13_bytes(animator));
    }
    bytes.extend(tail);
    bytes
}

/// An animator's encoding under snapshot format 13 ([`format_13_bytes`]):
/// without its last two bytes, `undrawn` and `walking`, and without its
/// layer's cursors,
/// which must be none. What follows the layer is the skin's clock and
/// flag, the appearance flag and the motion.
fn animator_13_bytes(animator: &Animator) -> Vec<u8> {
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
        assert!(layer.cursors.is_empty());
        assert_eq!(bytes.pop(), Some(0));
    }
    bytes.extend(after);
    bytes
}

#[test]
fn without_drawing_or_timed_events_the_encoding_is_format_13() {
    // Snapshot format 14 added the animator's flags that its person has
    // not been drawn and that it walks, and the layer's timed event
    // cursors.
    assert_eq!(digest(&format_13_bytes(&world())), PINNED_13);
}
const PINNED_13: &str = "67f6d55218dde3333e03d82ef2b1648f59e3b6ec3d67fb63477f96358039baec";

/// The encoding of `world` with no routing and its one animator with no
/// route ([`Motion::NONE`]), minus the bytes those encode to: what
/// snapshot format 12 encoded. Format 13 added the routing at the end of the world (three
/// zero bytes when empty) and the route's fields at the end of an
/// animator, the last thing encoded before the usage counts.
fn format_12_bytes(mut world: World) -> Vec<u8> {
    world.routing = Routing::default();
    assert!(world.animators.len() <= 1);
    let motion = match world.animators.values_mut().next() {
        Some(animator) => {
            animator.motion = Motion::NONE;
            motion_15_bytes(&Motion::NONE)
        }
        None => Vec::new(),
    };
    let usage = postcard::to_allocvec(&world.usage).unwrap();
    let mut bytes = format_13_bytes(&world);
    assert_eq!(bytes.split_off(bytes.len() - 3), [0; 3]);
    let usage_bytes = bytes.split_off(bytes.len() - usage.len());
    assert_eq!(usage_bytes, usage);
    assert_eq!(bytes.split_off(bytes.len() - motion.len()), motion);
    bytes.extend(usage);
    bytes
}

#[test]
fn without_routing_the_encoding_is_format_12() {
    // Snapshot format 13 added the lot's routing and the animator's
    // route fields.
    assert_eq!(digest(&format_12_bytes(world())), PINNED_12);
}
const PINNED_12: &str = "ec8aa330243910a992600b661de18c6fe08a438af7e4058ccd8c7ae67332780c";

/// The encoding of `world` with its usage counts all 0 and without what
/// format 13 added, minus the twelve zero bytes the counts encode to.
/// Snapshot format 12 added them at the end.
fn without_usage(mut world: World) -> Vec<u8> {
    world.usage = HouseUsage::default();
    let mut bytes = format_12_bytes(world);
    assert_eq!(bytes.split_off(bytes.len() - 12), [0; 12]);
    bytes
}

/// The encoding of `world`, which holds one animator, without the usage
/// counts and without the flag snapshot format 12 added at the end of an
/// animator: what snapshot format 11 encoded. The animator is the last
/// thing encoded before the usage counts, and the flag its last field.
fn format_11_bytes(world: &World) -> Vec<u8> {
    assert_eq!(world.animators.len(), 1);
    let stale = world.animators.values().next().unwrap().profile_stale;
    let mut bytes = without_usage(world.clone());
    assert_eq!(bytes.split_off(bytes.len() - 1), [u8::from(stale)]);
    bytes
}

#[test]
fn without_the_appearance_flag_the_encoding_is_format_11() {
    // Snapshot format 12 added the animator's flag that the person's
    // appearance is to be read again (+0xd8), and the house's usage
    // counts at the end.
    assert_eq!(digest(&format_11_bytes(&world())), PINNED_11);
}
const PINNED_11: &str = "db098963b435573f4282f5706b20ed599ef95c395951934dc0ef917d1c5aac1b";

/// The encoding of `world` without its animators or usage counts, minus
/// the bytes they encode to: what snapshot format 10 encoded.
fn format_10_bytes(mut world: World) -> Vec<u8> {
    world.animators.clear();
    let mut bytes = without_usage(world);
    assert_eq!(bytes.split_off(bytes.len() - 1), [0]);
    bytes
}

fn digest(bytes: &[u8]) -> String {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(bytes).into();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn without_animators_the_encoding_is_format_10_and_a_zero_byte() {
    // Snapshot format 11 added the persons' animators at the end.
    assert_eq!(digest(&format_10_bytes(world())), PINNED_10);
}
const PINNED_10: &str = "fd61775798ab1e3a152bc9875852ca2e51da1600fe802e90b5d711205c58fd68";

/// The digest of [`world`] without its neighbors or animators, minus the
/// bytes they encode to: what snapshot format 9 pinned.
fn format_9_digest(mut world: World) -> String {
    world.neighbors = Neighbors::default();
    let mut bytes = format_10_bytes(world);
    assert_eq!(bytes.split_off(bytes.len() - 3), [0; 3]);
    digest(&bytes)
}

#[test]
fn without_neighbors_the_encoding_is_format_9_and_three_zero_bytes() {
    // Snapshot format 10 added the neighborhood's records at the end.
    assert_eq!(format_9_digest(world()), PINNED_9);
}
const PINNED_9: &str = "2efd25eeadfef4e5f72cdc7f4fd89cf2889fdc57d892f8be3d4b78916c50d470";

#[test]
fn without_dialogs_or_person_ticks_the_encoding_is_format_8_and_three_zero_bytes() {
    // Snapshot format 9 added the dialog records and the persons' tick
    // state at the end. Emptied, they are three zero bytes, and the rest
    // hashes to the digest pinned under format 8.
    let mut world = world();
    world.dialogs = Dialogs {
        records: Vec::new(),
        declined: 0,
    };
    world.person_ticks.clear();
    world.neighbors = Neighbors::default();
    let mut bytes = format_10_bytes(world);
    assert_eq!(bytes.split_off(bytes.len() - 6), [0; 6]);
    assert_eq!(digest(&bytes), PINNED_8);
}
const PINNED_8: &str = "c30d0c3a346a5580c68f56b2e3101730b34a69357685be38a071c7b8f47250e0";

#[test]
fn encoding_round_trips_bit_for_bit() {
    let world = world();
    let bytes = world.encode();
    let decoded = World::decode(&bytes).unwrap();
    assert_eq!(decoded.encode(), bytes);
    // The NaN payload and negative zero survive.
    let portal = decoded.objects[&2].portal.as_ref().unwrap();
    assert_eq!(portal[1].to_bits(), 0x8000_0000);
    assert_eq!(portal[2].to_bits(), 0x7fc0_0001);
    assert!(World::decode(&bytes[..bytes.len() - 1]).is_err());
}

#[test]
fn any_field_change_changes_the_hash() {
    let base = world().hash();
    let mut changed = world();
    changed.people.push(2);
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.characters.next_file += 1;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.portraits.remove(&0x7fd9_6b54);
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.dialogs.records[0].status = 5;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.usage.sample(false);
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.dialogs.declined = 0;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.person_ticks.get_mut(&5).unwrap().room = -5;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.animators.get_mut(&5).unwrap().events.pop();
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    let animator = changed.animators.get_mut(&5).unwrap();
    animator.foreground.as_mut().unwrap().phase = F32::from_bits(0x3d4d_0149);
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.animators.get_mut(&5).unwrap().rate = F32::from_bits(0x3fb3_3334);
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.animators.get_mut(&5).unwrap().profile_stale = false;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.animators.get_mut(&5).unwrap().motion.segment = 2;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.animators.get_mut(&5).unwrap().undrawn = false;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.animators.get_mut(&5).unwrap().walking = true;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.animators.get_mut(&5).unwrap().head.target = None;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    let animator = changed.animators.get_mut(&5).unwrap();
    animator.foreground.as_mut().unwrap().cursors.push(0);
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.routing.routes.get_mut(&5).unwrap().path.pop();
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.routing.unknown_words.clear();
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.lot_rng.next_u32();
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.objects.get_mut(&5).unwrap().data[40] += 1;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.order.reverse();
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.types.insert(9, 0x3333);
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.next_parts.insert(5, 2);
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.skeleton_files.clear();
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.rooms.map[1][0] = 1025;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.rooms.rooms.get_mut(&1).unwrap().dirty = false;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.rooms.rooms.get_mut(&1).unwrap().light = F32::ZERO;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.rooms.pending = 0;
    assert_ne!(changed.hash(), base);
    let mut changed = world();
    changed.objects.get_mut(&5).unwrap().selected ^= true;
    assert_ne!(changed.hash(), base);
}

struct Temp(PathBuf);
impl Temp {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("sims-sim-{name}-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn snapshots_round_trip_through_redb() {
    let file = Temp::new("snapshots");
    let world = world();
    {
        let store = Snapshots::open(&file.0).unwrap();
        assert!(store.load("boot").unwrap().is_none());
        store.save("boot", &world).unwrap();
        let mut later = world.clone();
        later.ticks += 1;
        store.save("later", &later).unwrap();
    }
    let store = Snapshots::open(&file.0).unwrap();
    assert_eq!(
        store.load("boot").unwrap().unwrap().encode(),
        world.encode()
    );
    assert_eq!(store.load("later").unwrap().unwrap().ticks, world.ticks + 1);
    assert!(store.load("missing").unwrap().is_none());
}

#[test]
fn saved_lots_round_trip_through_redb_apart_from_the_snapshots() {
    let file = Temp::new("lots");
    let world = world();
    {
        let store = Snapshots::open(&file.0).unwrap();
        // No lot table yet: nothing is saved.
        assert!(store.load_lot("slot").unwrap().is_none());
        store.save("slot", &world).unwrap();
        store.save_lot("slot", b"first").unwrap();
        store.save_lot("slot", b"second").unwrap();
    }
    let store = Snapshots::open(&file.0).unwrap();
    // The later save replaced the earlier, and the snapshot of the same
    // name is its own.
    assert_eq!(store.load_lot("slot").unwrap().unwrap(), b"second");
    assert_eq!(
        store.load("slot").unwrap().unwrap().encode(),
        world.encode()
    );
    assert!(store.load_lot("missing").unwrap().is_none());
}

#[test]
fn snapshot_floor_rotations_round_trip_and_clear_on_plain_save() {
    let file = Temp::new("snapshot-floor-rot");
    let world = world();
    let mut overwritten = world.clone();
    overwritten.ticks += 7;
    {
        let store = Snapshots::open(&file.0).unwrap();
        assert!(store.load_with_floor_rotation("boot").unwrap().is_none());
        store.save_with_floor_rotation("boot", &world, 7).unwrap();
        store.save("plain", &world).unwrap();
    }
    {
        let ro = ReadOnlySnapshots::open(&file.0).unwrap();
        let (loaded, rot) = ro.load_with_floor_rotation("boot").unwrap().unwrap();
        assert_eq!(rot, Some(3));
        assert_eq!(loaded.encode(), world.encode());
        assert_eq!(ro.load("boot").unwrap().unwrap().encode(), world.encode());
        let (plain_world, plain_rot) = ro.load_with_floor_rotation("plain").unwrap().unwrap();
        assert_eq!(plain_rot, None);
        assert_eq!(plain_world.encode(), world.encode());
        assert!(ro.load_with_floor_rotation("missing").unwrap().is_none());
    }
    // Overwriting a named snapshot with plain save (the exact legacy record
    // format written by older writers) replaces the record and leaves no stale
    // floor rotation behind.
    {
        let store = Snapshots::open(&file.0).unwrap();
        store.save("boot", &overwritten).unwrap();
        let (loaded, rot) = store.load_with_floor_rotation("boot").unwrap().unwrap();
        assert_eq!(rot, None);
        assert_eq!(loaded.ticks, world.ticks + 7);
    }
}

fn person() -> sims_formats::saved_objects::Person {
    let interaction = sims_formats::saved_objects::Interaction {
        join: 0,
        uid: 0,
        caller: 0,
        target: 0,
        icon: 0,
        table_index: 0,
        args: [0; 4],
        priority: 0,
        action: 0,
        attenuation: 0.0,
        flags: 0,
    };
    sims_formats::saved_objects::Person {
        animation_events: 0,
        engaged: 0,
        appearance: Default::default(),
        accessories: Vec::new(),
        animations: Default::default(),
        routing_state: 0,
        floats: [0.0; 9],
        previous_motives: [0.0; 16],
        motives: [0.0; 16],
        data: vec![0; 256],
        routing_frames: 0,
        active: interaction.clone(),
        last: interaction,
        queue: Vec::new(),
        object_uses: Vec::new(),
        motive_deltas: Vec::new(),
    }
}

#[test]
fn an_actions_join_word_keeps_the_encoding_of_one_that_does_not_join() {
    use sims_formats::saved_objects::Interaction;
    let action = Interaction {
        join: 0,
        uid: 7,
        caller: 2,
        target: -3,
        icon: 4,
        table_index: 5,
        args: [1, -1, 6, 0],
        priority: 50,
        action: 0x1234,
        attenuation: 0.5,
        flags: 0x8000_000d,
    };
    // The fields as format 30 wrote them, the flags a 32-bit word.
    let old = postcard::to_allocvec(&(
        7i32,
        2i16,
        -3i16,
        4i16,
        5i32,
        [1i16, -1, 6, 0],
        50i32,
        0x1234u16,
        0.5f32,
        0x8000_000du32,
    ))
    .unwrap();
    assert_eq!(postcard::to_allocvec(&action).unwrap(), old);
    // The join word is kept, and only it changes the bytes.
    let joining = Interaction { join: 1, ..action };
    let bytes = postcard::to_allocvec(&joining).unwrap();
    assert_ne!(bytes, old);
    let read: Interaction = postcard::from_bytes(&bytes).unwrap();
    assert_eq!((read.join, read.flags, read.uid), (1, 0x8000_000d, 7));
    let read: Interaction = postcard::from_bytes(&old).unwrap();
    assert_eq!((read.join, read.flags), (0, 0x8000_000d));
}

#[test]
fn the_selected_sim_is_the_first_marked_person() {
    let mut world = world();
    // Object 5 is marked but is no person.
    assert_eq!(world.selected(), None);
    assert_eq!(world.selected_whereabouts(), None);
    for id in [2, 7, 9] {
        let mut sim = object(id, 0x3333);
        sim.person = Some(person());
        world.objects.insert(id, sim);
    }
    world.order = vec![5, 2, 9, 7];
    assert_eq!(world.selected(), None, "a person must be marked");
    for id in [7, 9] {
        let sim = world.objects.get_mut(&id).unwrap();
        sim.selected = true;
        sim.position = [-40, 600, 2];
        sim.data[29] = 1025;
    }
    // The registration order decides between two marked people.
    assert_eq!(world.selected(), Some(9));
    assert_eq!(
        world.selected_whereabouts(),
        Some(Whereabouts {
            id: 9,
            position: [-40, 600, 2],
            tile: [-3, 37, 2],
            level: 2,
            room: Some(1025),
        })
    );
    // An object without a room word has no room.
    world.objects.get_mut(&9).unwrap().data.truncate(29);
    assert_eq!(world.selected_whereabouts().unwrap().room, None);
    // Reading moves nothing.
    let hash = world.hash();
    world.selected_whereabouts();
    assert_eq!(world.hash(), hash);
}

#[test]
fn read_only_snapshots_never_write_the_file() {
    let file = Temp::new("read-only");
    assert!(ReadOnlySnapshots::open(&file.0).is_err());
    assert!(!file.0.exists(), "opening to read created the file");
    let world = world();
    Snapshots::open(&file.0)
        .unwrap()
        .save("boot", &world)
        .unwrap();
    let before = std::fs::read(&file.0).unwrap();
    {
        let store = ReadOnlySnapshots::open(&file.0).unwrap();
        assert_eq!(
            store.load("boot").unwrap().unwrap().encode(),
            world.encode()
        );
        assert!(store.load("missing").unwrap().is_none());
        // Two readers share the file.
        let other = ReadOnlySnapshots::open(&file.0).unwrap();
        assert!(other.load("boot").unwrap().is_some());
    }
    assert_eq!(std::fs::read(&file.0).unwrap(), before);
    // A program holding the file to write refuses the reader on platforms
    // that lock files, and the reader leaves nothing beside the file.
    let writer = Snapshots::open(&file.0).unwrap();
    if cfg!(any(target_os = "macos", target_os = "linux", windows)) {
        assert!(ReadOnlySnapshots::open(&file.0).is_err());
    }
    drop(writer);
    let directory = file.0.parent().unwrap();
    let name = file.0.file_name().unwrap().to_string_lossy().into_owned();
    // Only emptiness is checked, so the listing's order does not matter.
    #[allow(clippy::disallowed_methods)]
    let beside: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|other| other != &name && other.starts_with(&name))
        .collect();
    assert!(beside.is_empty(), "files beside the snapshot: {beside:?}");
}

/// Boots the target house from a real install when `SIMS_INSTALL` names
/// one. Game data is never committed, so CI skips this.
#[test]
fn boots_the_goth_house() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    let root = PathBuf::from(root);
    let install = Install::detect(&root).unwrap();
    let neighborhood = Neighborhood::open(&install.file("UserData").unwrap()).unwrap();
    let mut index = Index::build(&install).unwrap();
    let characters = neighborhood.characters().unwrap().unwrap();
    index.add_characters(&characters).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let house = House::load(&neighborhood.houses().unwrap()[&5], &catalog).unwrap();
    let world = World::from_house(&house, &catalog).unwrap();
    assert_eq!(world.objects.len(), 245);
    assert_eq!(world.ticks, 88444);
    let people = world
        .objects
        .values()
        .filter(|o| o.person.is_some())
        .count();
    assert_eq!(people, 3);
    let again = World::from_house(&house, &catalog).unwrap();
    assert_eq!(again.hash(), world.hash());
    let file = Temp::new("goth");
    let store = Snapshots::open(&file.0).unwrap();
    store.save("boot", &world).unwrap();
    assert_eq!(
        store.load("boot").unwrap().unwrap().encode(),
        world.encode()
    );
}

#[test]
fn the_house_model_gives_the_usage_counts() {
    // 0x416568 reads each pair's routing count (+4) before its total (+0).
    let model = sims_formats::lot::House {
        version: 0x49,
        name: String::new(),
        pairs: [[1, 2], [3, 4], [5, 6], [7, 8], [9, 10], [11, 12]],
        tail: Vec::new(),
    };
    let mut world = World::empty();
    world.read_house_model(&model);
    assert_eq!(
        world.usage.history.map(|u| (u.total, u.routing)),
        [(2, 1), (4, 3), (6, 5), (8, 7), (10, 9)]
    );
    assert_eq!(
        world.usage.today,
        Usage {
            total: 12,
            routing: 11
        }
    );
}

#[test]
fn the_end_of_the_object_read_sets_registry_byte_0x12() {
    // 0x43fc89, whatever load steps 8 and 9 found.
    let mut world = World::empty();
    assert!(!world.neighbors.relationships_changed);
    world.finish_object_read();
    assert!(world.neighbors.relationships_changed);
}
