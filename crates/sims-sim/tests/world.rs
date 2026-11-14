use sims_content::{Catalog, House, Index, Install, Neighborhood};
use sims_core::{CrtRng, F32, GameRng};
use sims_formats::lot::Array;
use sims_formats::saved_objects::{Frame, Slot};
use sims_sim::animator::{Animator, LOADED, Layer, Motion, UNSET};
use sims_sim::neighbors::{Inventory, Item, Neighbor, Neighbors, Row};
use sims_sim::rooms::{Half, Room, Rooms};
use sims_sim::route::{Candidate, Context, Route, RouteSlot, Routing};
use sims_sim::{
    Budget, Dialog, Dialogs, FrameStorage, HouseUsage, MagicHistory, Object, PersonTick, Snapshots,
    TypeAttributes, Usage, World,
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
    }
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
                }],
                path: vec![[24, 664], [76, 656], [408, 413]],
            },
        )]),
        slot_flags: BTreeMap::from([((2, 0), 0x4000)]),
        unknown_words: BTreeSet::from([(5, 37)]),
    }
}

/// An animator playing a clip backward, with an event queued. The posture,
/// base layer and destination are the values [`LOADED`] held before
/// snapshot format 12, so that the format 11 digests still apply.
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
    Animator {
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
        base: true,
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
const PINNED: &str = "f72290e87a9e96c2436bb657cdf83806bb38760885f45f9b0d6a53d1f12a19c3";

/// The encoding of `world` as snapshot format 14 wrote it. Format 15
/// added the last number given to a running action
/// ([`World::action_serial`]) at the end. This drops its bytes.
fn format_14_bytes(world: &World) -> Vec<u8> {
    let mut bytes = world.encode();
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
    tail.extend(postcard::to_allocvec(&world.routing).unwrap());
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
    let mut bytes = postcard::to_allocvec(animator).unwrap();
    assert_eq!(bytes.pop(), Some(u8::from(animator.walking)));
    assert_eq!(bytes.pop(), Some(u8::from(animator.undrawn)));
    let after = postcard::to_allocvec(&(
        animator.skin_clock,
        animator.skin_dirty,
        animator.profile_stale,
        &animator.motion,
    ))
    .unwrap();
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
            postcard::to_allocvec(&Motion::NONE).unwrap()
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
    world.dialogs = Dialogs::default();
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
