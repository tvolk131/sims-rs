use sims_content::{Catalog, House, Index, Install, Neighborhood};
use sims_core::{CrtRng, GameRng};
use sims_formats::lot::Array;
use sims_formats::saved_objects::{Frame, Slot};
use sims_sim::rooms::{Half, Room, Rooms};
use sims_sim::{Budget, FrameStorage, MagicHistory, Object, Snapshots, TypeAttributes, World};
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
                },
            )]),
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
    }
}

#[test]
fn the_hash_is_the_same_on_every_platform() {
    // Pinned: CI runs this on x86_64 Linux and aarch64 macOS. If the
    // encoding changes on purpose, bump snapshot::FORMAT and update this.
    assert_eq!(world().hash_hex(), PINNED);
}
const PINNED: &str = "e5eef2ff7049b4a63eaaccd0f2fb732bca6ad86fbd39006c538004bd19e013c2";

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
