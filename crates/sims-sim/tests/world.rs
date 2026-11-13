use sims_content::{Catalog, House, Index, Install, Neighborhood};
use sims_core::{CrtRng, GameRng};
use sims_formats::lot::Array;
use sims_formats::saved_objects::{Frame, Slot};
use sims_sim::{Budget, MagicHistory, Object, Snapshots, TypeAttributes, World};
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
    }
}

#[test]
fn the_hash_is_the_same_on_every_platform() {
    // Pinned: CI runs this on x86_64 Linux and aarch64 macOS. If the
    // encoding changes on purpose, bump snapshot::FORMAT and update this.
    assert_eq!(world().hash_hex(), PINNED);
}
const PINNED: &str = "339502b3b17dce9fb7e1f9ed32e5368cad8371ce2a218bfab2d2e26081bc8155";

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
}

fn instance(id: u16, guid: u32) -> sims_formats::saved_objects::Instance {
    let object = object(id, guid);
    sims_formats::saved_objects::Instance {
        id,
        type_id: 7,
        guid,
        footprint: object.footprint,
        position: object.position,
        unknown_short: object.unknown_short,
        attributes: object.attributes,
        temps: object.temps,
        data: object.data,
        obsolete_word: 0,
        stack_capacity_bytes: 512,
        stack: object.stack,
        relationship_flag: -1,
        relationships: Vec::new(),
        slots: object.slots,
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: None,
        person: None,
        registry_word: object.registry_word,
        consumed_bits: 0,
        record: Vec::new(),
    }
}

#[test]
fn saved_objects_are_read_onto_a_cleared_lot_in_file_order() {
    let definition = |guid, resource| sims_content::Object {
        guid,
        resource,
        chunk: 128,
        kind: 4,
        master: 0,
    };
    let catalog = Catalog {
        global: 0,
        objects: vec![definition(0x1111, 1), definition(0x2222, 2)],
        by_guid: BTreeMap::from([(0x1111, 0), (0x2222, 1)]),
        semi_globals: BTreeMap::new(),
        diagnostics: Vec::new(),
    };
    let saved = sims_formats::saved_objects::Objects {
        version: 62,
        order: vec![9, 4, 6],
        types_by_id: BTreeMap::new(),
        instances: BTreeMap::from([
            (4, instance(4, 0x1111)),
            (6, instance(6, 0x2222)),
            (9, instance(9, 0x1111)),
        ]),
        tail: Vec::new(),
    };
    // A cleared lot keeps what the earlier load steps left: here an empty
    // instance list for one type, a queued removal and the globals.
    let mut lot = world();
    lot.objects.clear();
    lot.order.clear();
    lot.instances = BTreeMap::from([(0x1111, Vec::new())]);
    let before = lot.clone();
    lot.read_objects(&saved, &catalog).unwrap();
    assert_eq!(lot.order, [9, 4, 6]);
    assert_eq!(lot.objects.keys().copied().collect::<Vec<_>>(), [4, 6, 9]);
    assert_eq!(lot.objects[&6].guid, 0x2222);
    assert_eq!(lot.objects[&6].stack[0].tree, 4096);
    assert!(!lot.objects[&6].last_result);
    // A type with an instance list gets its objects appended, in file
    // order. A type no script has asked about gets no list.
    assert_eq!(lot.instances, BTreeMap::from([(0x1111, vec![9, 4])]));
    assert_eq!(lot.removals, before.removals);
    assert_eq!(lot.globals, before.globals);
    assert_eq!(lot.global_rng, before.global_rng);
    assert_eq!(lot.types, before.types);

    // A lot that still holds objects is refused, unchanged.
    let held = lot.clone();
    assert!(matches!(
        lot.read_objects(&saved, &catalog),
        Err(sims_sim::Error::NotCleared)
    ));
    assert_eq!(lot.encode(), held.encode());
    // So is an object with no definition.
    let mut lot = before.clone();
    let mut unknown = saved.clone();
    unknown.instances.insert(6, instance(6, 0x9999));
    assert!(matches!(
        lot.read_objects(&unknown, &catalog),
        Err(sims_sim::Error::MissingDefinition {
            object: 6,
            guid: 0x9999
        })
    ));
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
