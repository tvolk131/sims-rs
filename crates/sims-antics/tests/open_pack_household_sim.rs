//! End-to-end 20-object clean-room standalone `open-pack/` verification &
//! 8-motive household `sims-vm` simulation test suite.
//!
//! 1. Compiles the complete standalone `open-pack/` (20 objects, `.far`, WAV bank,
//!    4 `.flr` floors, 4 `.wll` walls, `House01.iff`, `Neighborhood.iff`).
//! 2. Runs `verify_pack_safety` across ALL 20 objects asserting **100% pass, 0 violations**.
//! 3. Validates every compiled `.iff`, `.flr`, `.wll`, `.far`, `House01.iff`, and
//!    `Neighborhood.iff` through `sims_formats`.
//! 4. Boots an authentic `sims-vm` household world and exercises ALL 8 motives
//!    (Hunger, Comfort, Hygiene, Bladder, Energy, Fun, Social, Room), Cooking &
//!    Mechanical skill study, `alter_budget` bill payment, and mid-loop native
//!    `vm.cancel` cleanup with **0 VM traps**!

use sims_antics::{compile_standalone_pack_dir, verify_pack_safety};
use sims_content::Catalog;
use sims_core::GameRng;
use sims_formats::behavior::{Behavior, InteractionTable};
use sims_formats::execution::{ObjectFunctions, Slots, constants};
use sims_formats::far;
use sims_formats::iff::Iff;
use sims_formats::lot::Array as LotArray;
use sims_formats::neighborhood::{Family, Neighborhood, Neighbors};
use sims_formats::saved_objects::{
    Frame, Interaction as SavedInteraction, Person, Slot as SavedSlot,
};
use sims_formats::sprite::{DrawGroup, LegacySpriteSet, Palette, SpriteSet, render};
use sims_formats::strings::StringTable;
use sims_sim::rooms::Room;
use sims_sim::{Object, World};
use sims_vm::{DEFINITION_WORDS, Interpreter, Library, Program, SoundEvent, Status, Stream};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const SIM_GUID: u32 = 0x7F00_0101;
const SIM2_GUID: u32 = 0x7F00_0102;
const TOILET_GUID: u32 = 0x7F00_0050;
const SOFA_GUID: u32 = 0x7F00_0051;
const TV_GUID: u32 = 0x7F00_0052;
const BOOKSHELF_GUID: u32 = 0x7F00_0053;
const DINING_TABLE_GUID: u32 = 0x7F00_0060;
const DINING_CHAIR_GUID: u32 = 0x7F00_0061;
const FLOOR_LAMP_GUID: u32 = 0x7F00_0070;
const TRASH_CAN_GUID: u32 = 0x7F00_0071;
const FRONT_DOOR_GUID: u32 = 0x7F00_0080;
const MAILBOX_GUID: u32 = 0x7F00_0090;
const FRIDGE_GUID: u32 = 0x7F00_0020;
const COUNTER_GUID: u32 = 0x7F00_0021;
const STOVE_GUID: u32 = 0x7F00_0022;
const PLATE_GUID: u32 = 0x7F00_0023;
const SHOWER_GUID: u32 = 0x7F00_0040;
const COT_GUID: u32 = 0x7F00_0002;

const MOTIVE_ENERGY: usize = 5;
const MOTIVE_COMFORT: usize = 6;
const MOTIVE_HUNGER: usize = 7;
const MOTIVE_HYGIENE: usize = 8;
const MOTIVE_BLADDER: usize = 9;
const MOTIVE_ROOM: usize = 13;
const MOTIVE_SOCIAL: usize = 14;
const MOTIVE_FUN: usize = 15;

const SKILL_COOKING: usize = 10;
const SKILL_MECHANICAL: usize = 12;

fn open_pack_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("open-pack")
}

fn load_iff_into_vm(iff: &Iff, resource_id: usize, catalog: &mut Catalog) -> Library {
    let mut lib = Library::default();
    for chunk in &iff.chunks {
        let data = iff.data(chunk);
        match &chunk.tag {
            b"BHAV" => {
                lib.behaviors
                    .insert(chunk.id, Behavior::parse(data).unwrap());
                lib.names.insert(chunk.id, chunk.label.clone());
            }
            b"BCON" => {
                lib.constants.insert(chunk.id, constants(data).unwrap());
            }
            b"OBJf" => {
                lib.callbacks
                    .insert(chunk.id, ObjectFunctions::parse(data).unwrap().entries);
            }
            b"OBJD" => {
                let mut words = [0i16; DEFINITION_WORDS];
                for (i, pair) in data
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .take(DEFINITION_WORDS)
                    .enumerate()
                {
                    words[i] = i16::from_le_bytes(*pair);
                }
                let guid = u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16);
                let kind = words[9] as u16;
                let master = words[10] as u16;
                lib.definitions.insert(chunk.id, words);
                lib.definition_order.push(chunk.id);
                let idx = catalog.objects.len();
                catalog.objects.push(sims_content::Object {
                    guid,
                    resource: resource_id,
                    chunk: chunk.id,
                    kind,
                    master,
                });
                catalog.by_guid.insert(guid, idx);
            }
            b"STR#" => {
                let table = StringTable::parse(data).unwrap();
                let entries: Vec<String> = table.entries.into_iter().map(|e| e.text).collect();
                lib.strings.insert(chunk.id, entries);
            }
            b"CTSS" => {
                let table = StringTable::parse(data).unwrap();
                let entries: Vec<(u8, String)> = table
                    .entries
                    .into_iter()
                    .map(|e| (e.language, e.text))
                    .collect();
                lib.catalog_strings.insert(chunk.id, entries);
            }
            b"FWAV" => {
                let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
                let name: String = data[..end].iter().map(|&b| char::from(b)).collect();
                lib.sounds.insert(chunk.id, name);
            }
            b"TTAB" => {
                lib.interactions
                    .insert(chunk.id, InteractionTable::parse(data).ok());
            }
            b"SLOT" => {
                let slots = Slots::parse(data).unwrap().entries;
                let containers = slots.iter().filter(|s| s.kind == 0).count();
                lib.containers.insert(chunk.id, containers);
                lib.slots.insert(chunk.id, slots);
            }
            _ => {}
        }
    }
    lib
}

fn make_lot_array(stride: usize, fill: u8) -> LotArray {
    LotArray {
        width: 64,
        height: 64,
        stride,
        encoding: 0,
        data: vec![fill; 64 * 64 * stride],
    }
}

fn make_sim_object(id: u16, guid: u32) -> Object {
    let mut motives = [0.0f32; 16];
    motives[MOTIVE_ENERGY] = -30.0;
    motives[MOTIVE_COMFORT] = -35.0;
    motives[MOTIVE_HUNGER] = -55.0;
    motives[MOTIVE_HYGIENE] = -40.0;
    motives[MOTIVE_BLADDER] = -60.0;
    motives[MOTIVE_ROOM] = -25.0;
    motives[MOTIVE_SOCIAL] = -30.0;
    motives[MOTIVE_FUN] = -40.0;

    let idle = SavedInteraction {
        join: 0,
        uid: 0,
        caller: 0,
        target: 0,
        icon: 0,
        table_index: -1,
        args: [0; 4],
        priority: 0,
        action: 0,
        attenuation: 0.0,
        flags: 0,
    };

    let mut data = vec![0i16; 72];
    data[11] = id as i16;
    data[14] = -1;
    data[29] = 1;
    data[67] = 5;

    let mut person_data = vec![0i16; 256];
    person_data[SKILL_COOKING] = 200;
    person_data[SKILL_MECHANICAL] = 150;
    person_data[58] = 25;

    Object {
        id,
        guid,
        footprint: [0; 4],
        position: [8 * 16 + 8, 8 * 16 + 8, 1],
        unknown_short: 0,
        attributes: vec![0; 16],
        temps: vec![0; 8],
        data,
        obsolete_word: 0,
        stack_capacity_bytes: 512,
        stack: vec![Frame {
            stack_object: 0,
            tree: 4097,
            node: 0,
            parameters: vec![1, 0, 0, 0],
            locals: Vec::new(),
            primitive_state: 0,
            code_owner_type: 1,
        }],
        relationship_flag: -1,
        relationships: Vec::new(),
        slots: vec![
            SavedSlot {
                unknown: 0,
                object: 0,
            };
            6
        ],
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: None,
        person: Some(Person {
            animation_events: 0,
            engaged: 0,
            appearance: Default::default(),
            accessories: Vec::new(),
            animations: Default::default(),
            routing_state: 0,
            floats: [0.0; 9],
            previous_motives: motives,
            motives,
            data: person_data,
            routing_frames: 0,
            active: idle.clone(),
            last: idle,
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        }),
        registry_word: Some(0xffff),
        last_result: false,
        removal_skips_resets: false,
        selected: false,
    }
}

fn place_on_tile(world: &mut World, obj_id: u16, tx: i32, ty: i32) {
    let px = tx * 16 + 8;
    let py = ty * 16 + 8;
    let obj = world.objects.get_mut(&obj_id).unwrap();
    obj.position = [px, py, 1];
    obj.footprint = [px - 8, py - 8, px + 8, py + 8];
    obj.data[1] = 0;
    obj.data[29] = 1;
    let idx = (tx as usize * 64 + ty as usize) * 2;
    let grid = &mut world.arrays.get_mut(&3).unwrap().data;
    grid[idx..idx + 2].copy_from_slice(&(obj_id as i16).to_le_bytes());
}

fn run_pie_interaction_by_name(
    vm: &mut Interpreter<'_>,
    world: &mut World,
    ttas_by_guid: &BTreeMap<u32, Vec<String>>,
    sim_id: u16,
    target: u16,
    label: &str,
    max_turns: usize,
) -> usize {
    let entries = vm.pie_menu(world, sim_id, target).expect("pie_menu failed");
    let target_guid = world.objects[&target].guid;
    let names = &ttas_by_guid[&target_guid];
    let chosen = entries
        .into_iter()
        .find(|e| {
            names
                .get(e.action.table_index as usize)
                .is_some_and(|s| s == label)
        })
        .unwrap_or_else(|| {
            panic!("pie_menu on object {target} missing '{label}'");
        });
    let uid = vm
        .pick(world, sim_id, chosen.action)
        .expect("pick failed")
        .expect("action queue rejected picked interaction");
    assert!(uid > 0);

    let mut active_yields = 0usize;
    let mut saw_started = false;
    for _ in 0..max_turns {
        world.objects.get_mut(&sim_id).unwrap().data[25] = 1;
        let status = vm
            .turn(world, sim_id, 4_000)
            .expect("VM trap during Sim interaction turn");
        let stack_depth = world.objects[&sim_id].stack.len();
        if stack_depth > 1 {
            saw_started = true;
            if matches!(status, Status::Yielded) {
                active_yields += 1;
            }
        } else if saw_started {
            if let Some(person) = world.objects.get_mut(&sim_id).unwrap().person.as_mut() {
                person.active.uid = 0;
                person.active.flags = 0;
                person.data[33] = 0;
                person.object_uses.clear();
            }
            return active_yields;
        } else if matches!(status, Status::Yielded) {
            let person = world.objects[&sim_id].person.as_ref().unwrap();
            if person.queue.is_empty() && (person.active.uid == uid || person.last.uid == uid) {
                if let Some(person) = world.objects.get_mut(&sim_id).unwrap().person.as_mut() {
                    person.active.uid = 0;
                    person.active.flags = 0;
                    person.data[33] = 0;
                    person.object_uses.clear();
                }
                return active_yields;
            }
        }
    }
    panic!("interaction '{label}' did not complete within {max_turns} turns");
}

#[test]
fn standalone_open_pack_builds_verifies_safety_and_validates_all_formats() {
    let standalone =
        compile_standalone_pack_dir(&open_pack_dir()).expect("compile_standalone_pack_dir failed");

    // 1. Complete 20-object pack + procedural surfaces + starter house + 2-Sim household.
    assert_eq!(standalone.pack.objects.len(), 20);
    assert_eq!(standalone.floors.len(), 4);
    assert_eq!(standalone.walls.len(), 4);
    assert_eq!(standalone.starter_sims.len(), 2);
    assert!(standalone.placements.len() >= 18);

    // 2. Formal safety prover: 100% pass, 0 violations across all 20 objects!
    let report = verify_pack_safety(&standalone.pack).expect("verify_pack_safety failed");
    assert!(
        report.is_ok(),
        "pack safety violations: {:?}",
        report
            .objects
            .iter()
            .flat_map(|o| &o.violations)
            .collect::<Vec<_>>()
    );
    assert!(
        report.total_locks_verified() >= 15,
        "expected >=15 lock invariants verified across open-pack, got {}",
        report.total_locks_verified()
    );

    // 3. Validate FAR!byAZ archive and every object IFF + sprites via sims_formats.
    let manifest_off = far::manifest_offset(
        &standalone.pack.far_bytes[..16],
        standalone.pack.far_bytes.len() as u64,
    )
    .unwrap();
    let dir_len = far::directory_len(manifest_off, standalone.pack.far_bytes.len() as u64).unwrap();
    let far_entries = far::directory(
        &standalone.pack.far_bytes[manifest_off as usize..manifest_off as usize + dir_len],
        standalone.pack.far_bytes.len() as u64,
    )
    .expect("FAR directory parse failed");
    assert_eq!(far_entries.len(), 20);
    for entry in &standalone.pack.objects {
        let iff = Iff::parse(entry.object.iff_bytes.clone()).unwrap();
        let dgrp_chunk = iff.all(b"DGRP").next().expect("missing DGRP");
        let dgrp = DrawGroup::parse(iff.data(dgrp_chunk)).unwrap();
        assert_eq!(dgrp.views.len(), 12);
        let preview = render(&iff, &dgrp.views[0]).unwrap();
        assert!(preview.width > 0 && preview.height > 0);
    }

    // 4. Validate every procedural .flr floor IFF via sims_formats.
    for (name, bytes) in &standalone.floors {
        let iff = Iff::parse(bytes.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let st = StringTable::parse(iff.data(iff.get(b"STR#", 0).unwrap())).unwrap();
        assert_eq!(st.entries.len(), 3, "{name} STR# 0");
        assert_eq!(iff.all(b"SPR2").count(), 3, "{name} 3 zoom SPR2s");
        for id in [1u16, 257, 513] {
            let chunk = iff.get(b"SPR2", id).unwrap();
            let raw = iff.data(chunk);
            let frame = SpriteSet::parse(raw).unwrap().decode(raw, 0, &iff).unwrap();
            assert!(frame.width > 0 && frame.height > 0);
        }
    }

    // 5. Validate every procedural .wll wall IFF via sims_formats.
    for (name, bytes) in &standalone.walls {
        let iff = Iff::parse(bytes.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let st = StringTable::parse(iff.data(iff.get(b"STR#", 0).unwrap())).unwrap();
        assert_eq!(st.entries.len(), 3, "{name} STR# 0");
        assert_eq!(iff.all(b"SPR#").count(), 3, "{name} 3 zoom SPR#s");
        let palt = Palette::parse(iff.data(iff.get(b"PALT", 1).unwrap())).unwrap();
        for id in [1u16, 257, 513] {
            let chunk = iff.get(b"SPR#", id).unwrap();
            let raw = iff.data(chunk);
            let set = LegacySpriteSet::parse(raw).unwrap();
            assert_eq!(set.frames.len(), 4, "{name} SPR# {id} must have 4 frames");
            for f in 0..4 {
                let frame = set.decode_with_palette(raw, f, &palt).unwrap();
                assert!(frame.width > 0 && frame.height > 0);
            }
        }
    }

    // 6. Validate House01.iff through the REAL `sims_content::House::parse_bytes` loader
    //    (using a real Catalog built strictly from `standalone.pack.objects` +
    //    `standalone.characters`), `World::read_arrays`, and `World::rebuild_rooms`!
    assert_eq!(standalone.characters.len(), 2);
    let mut real_catalog = Catalog {
        global: 0,
        objects: Vec::new(),
        by_guid: BTreeMap::new(),
        semi_globals: BTreeMap::new(),
        diagnostics: Vec::new(),
    };
    let mut res_idx = 1usize;
    for entry in &standalone.pack.objects {
        let iff = Iff::parse(entry.object.iff_bytes.clone()).unwrap();
        let _ = load_iff_into_vm(&iff, res_idx, &mut real_catalog);
        res_idx += 1;
    }
    for char_bytes in standalone.characters.values() {
        let iff = Iff::parse(char_bytes.clone()).unwrap();
        let _ = load_iff_into_vm(&iff, res_idx, &mut real_catalog);
        res_idx += 1;
    }

    let parsed_house = sims_content::House::parse_bytes(
        standalone.house_iff.clone(),
        std::path::Path::new("House01.iff"),
        &real_catalog,
    )
    .expect("sims_content::House::parse_bytes failed on generated House01.iff");
    assert_eq!(parsed_house.view.as_ref().unwrap().roof, "open_shingle");
    assert_eq!(
        parsed_house.model.as_ref().unwrap().name,
        "Open Starter Cottage"
    );
    assert_eq!(parsed_house.simulator.funds, 20_000);
    assert_eq!(parsed_house.simulator.globals.len(), 64);
    assert_eq!(parsed_house.floor_map.as_ref().unwrap().len(), 4);
    assert_eq!(parsed_house.wall_map.as_ref().unwrap().len(), 4);
    assert_eq!(parsed_house.arrays.len(), 14);
    assert_eq!(
        parsed_house.arrays[&0].stride, 4,
        "height array stride must be 4"
    );
    assert!(
        parsed_house.arrays[&9].data.iter().all(|&b| b == 0xff),
        "pool array #9 must be dry (0xff)"
    );
    assert!(
        parsed_house.arrays[&10].data.iter().all(|&b| b == 0xff),
        "water array #10 must be dry (0xff)"
    );

    // Multi-tile double bed has 2 tile instances (lead 0x7F00_0010 + slave 0x7F00_0011 with parent).
    let expected_instances = standalone.placements.len() + 1 + standalone.starter_sims.len();
    assert_eq!(parsed_house.objects.instances.len(), expected_instances);
    let bed_lead = parsed_house
        .objects
        .instances
        .values()
        .find(|i| i.guid == 0x7F00_0010)
        .expect("missing double bed lead tile");
    let bed_slave = parsed_house
        .objects
        .instances
        .values()
        .find(|i| i.guid == 0x7F00_0011)
        .expect("missing double bed slave tile");
    assert_eq!(bed_lead.multipart.as_ref().unwrap().parent, 0);
    assert_eq!(
        bed_slave.multipart.as_ref().unwrap().parent,
        bed_lead.id as i16
    );
    assert_eq!(
        World::tile(bed_slave.position),
        [
            World::tile(bed_lead.position)[0] + 1,
            World::tile(bed_lead.position)[1],
            1
        ]
    );

    let saved_persons: Vec<_> = parsed_house
        .objects
        .instances
        .values()
        .filter(|inst| inst.person.is_some())
        .collect();
    assert_eq!(
        saved_persons.len(),
        2,
        "expected 2 saved starter Sims in House01.iff"
    );
    for sim_inst in &saved_persons {
        assert_eq!(
            sim_inst.footprint[2] - sim_inst.footprint[0],
            6,
            "standing Sim footprint must be 6 subtile units wide"
        );
        assert_eq!(
            sim_inst.footprint[3] - sim_inst.footprint[1],
            6,
            "standing Sim footprint must be 6 subtile units wide"
        );
    }

    // Pass House01.iff arrays into real `World::read_arrays` and `World::rebuild_rooms`
    // and verify traversable doorway connectivity across Living Room, Kitchen, Bedroom,
    // Bathroom, and Front Porch!
    let mut loaded_world = World::empty();
    loaded_world.size = 64;
    let house_ver = parsed_house.model.as_ref().unwrap().version;
    loaded_world
        .read_arrays(&parsed_house.arrays, house_ver)
        .expect("World::read_arrays failed on House01.iff");
    loaded_world
        .rebuild_rooms()
        .expect("World::rebuild_rooms failed on House01.iff");
    assert!(
        !loaded_world.rooms.rooms.is_empty(),
        "rebuild_rooms produced no rooms"
    );

    // BFS tile walkability on ground-level walls (Arry #2 in loaded_world): verify all 5 room
    // centers are mutually reachable through the punched [[doorway]] openings!
    let wall_data = &loaded_world.arrays[&2].data;
    let can_step = |x0: usize, y0: usize, dx: isize, dy: isize| -> bool {
        let nx = x0 as isize + dx;
        let ny = y0 as isize + dy;
        if !(0..64).contains(&nx) || !(0..64).contains(&ny) {
            return false;
        }
        let (nx, ny) = (nx as usize, ny as usize);
        match (dx, dy) {
            (1, 0) => wall_data[(nx * 64 + ny) * 14] & 1 == 0,
            (-1, 0) => wall_data[(x0 * 64 + y0) * 14] & 1 == 0,
            (0, 1) => wall_data[(nx * 64 + ny) * 14] & 2 == 0,
            (0, -1) => wall_data[(x0 * 64 + y0) * 14] & 2 == 0,
            _ => false,
        }
    };
    let mut seen_tiles = BTreeSet::new();
    let mut q = std::collections::VecDeque::new();
    seen_tiles.insert((26usize, 27usize)); // Alex Sunvale in Living Room
    q.push_back((26usize, 27usize));
    while let Some((x, y)) = q.pop_front() {
        for (dx, dy) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
            if can_step(x, y, dx, dy) {
                let next = ((x as isize + dx) as usize, (y as isize + dy) as usize);
                if seen_tiles.insert(next) {
                    q.push_back(next);
                }
            }
        }
    }
    for (room_name, tile) in [
        ("Living Room", (26usize, 27usize)),
        ("Kitchen", (32, 27)),
        ("Bedroom", (25, 21)),
        ("Bathroom", (32, 21)),
        ("Front Porch", (26, 32)),
    ] {
        assert!(
            seen_tiles.contains(&tile),
            "{room_name} tile {tile:?} is unreachable from Living Room!"
        );
    }

    // Verify subtile ObjM positions match Arry #3 tile grid (`World::tile(pos)`).
    let grid_arr = &parsed_house.arrays[&3];
    for inst in parsed_house.objects.instances.values() {
        let [tx, ty, level] = World::tile(inst.position);
        assert!((0..64).contains(&tx) && (0..64).contains(&ty) && level == 1);
        let at = (tx as usize * 64 + ty as usize) * 2;
        let head = i16::from_le_bytes([grid_arr.data[at], grid_arr.data[at + 1]]) as u16;
        assert!(head > 0, "Arry #3 tile ({tx}, {ty}) missing head object");
    }

    // 7. Validate Neighborhood.iff through sims_formats::neighborhood.
    let nbhd_iff =
        Iff::parse(standalone.neighborhood_iff.clone()).expect("Neighborhood.iff parse failed");
    let ngbh = Neighborhood::parse(nbhd_iff.data(nbhd_iff.get(b"NGBH", 1).unwrap())).unwrap();
    assert_eq!(ngbh.version, 73);
    let nbrs = Neighbors::parse(nbhd_iff.data(nbhd_iff.get(b"NBRS", 1).unwrap())).unwrap();
    assert_eq!(nbrs.entries.len(), 2);
    let fami = Family::parse(nbhd_iff.data(nbhd_iff.get(b"FAMI", 1).unwrap())).unwrap();
    assert_eq!(fami.budget, 20_000);
    assert_eq!(fami.members, vec![0x7F00_0101, 0x7F00_0102]);

    // 8. Exercise REAL saved-House + saved-Sim startup from `parsed_house` + `Neighborhood.iff`
    //    + `standalone.pack.objects` + `standalone.characters` via `World::from_house` ->
    //    `read_neighbors` -> `read_types` -> `assign_rooms` -> `visit_objects`, proving both
    //    saved Sims boot organically onto `Main` (#4097) from `open_sim_01.iff`/`open_sim_02.iff`
    //    and interact cleanly with a saved object in `House01.iff` with 0 traps!
    let mut boot_catalog = Catalog {
        global: 0,
        objects: Vec::new(),
        by_guid: BTreeMap::new(),
        semi_globals: BTreeMap::new(),
        diagnostics: Vec::new(),
    };
    let mut boot_libs = BTreeMap::new();
    let mut boot_semi = BTreeMap::new();
    let mut boot_ttas: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    boot_libs.insert(0usize, Library::default());
    let mut boot_res = 1usize;
    for entry in &standalone.pack.objects {
        let iff = Iff::parse(entry.object.iff_bytes.clone()).unwrap();
        if let Some(ttas_chunk) = iff.get(b"TTAs", 128) {
            let st = StringTable::parse(iff.data(ttas_chunk)).unwrap();
            let labels: Vec<String> = st.entries.into_iter().map(|e| e.text).collect();
            boot_ttas.insert(entry.object.guid, labels.clone());
            for &tg in &entry.object.tile_guids {
                boot_ttas.insert(tg, labels.clone());
            }
        }
        let lib = load_iff_into_vm(&iff, boot_res, &mut boot_catalog);
        boot_libs.insert(boot_res, lib);
        boot_semi.insert(boot_res, None);
        boot_res += 1;
    }
    for char_bytes in standalone.characters.values() {
        let iff = Iff::parse(char_bytes.clone()).unwrap();
        let lib = load_iff_into_vm(&iff, boot_res, &mut boot_catalog);
        boot_libs.insert(boot_res, lib);
        boot_semi.insert(boot_res, None);
        boot_res += 1;
    }
    let boot_program = Program {
        global: 0,
        libraries: boot_libs,
        semi_globals: boot_semi,
        careers: None,
        career_texts_end: true,
        labels_hold_no_token: true,
        zoning: BTreeMap::new(),
        animations: Default::default(),
        people: Default::default(),
        icon_frames: None,
        wardrobe: None,
        ordered_wardrobe: None,
        character_files: Default::default(),
        expansions: 0,
        characters: Default::default(),
    };

    let mut booted_world = World::empty();
    booted_world.size = 64;
    booted_world.indicators = [u16::MAX, u16::MAX];
    booted_world.read_simulator(&parsed_house.simulator);
    if let Some(model) = &parsed_house.model {
        booted_world.read_house_model(model);
    }
    booted_world
        .read_arrays(&parsed_house.arrays, house_ver)
        .expect("read_arrays failed");
    booted_world.rebuild_rooms().expect("rebuild_rooms failed");
    let mut families_map = BTreeMap::new();
    families_map.insert(1u16, fami);
    let content_nbhd = sims_content::Neighborhood {
        directory: PathBuf::from("."),
        families: families_map,
        type_attributes: None,
        neighbors: nbrs.entries,
        inventories: ngbh.inventories,
        data: ngbh.data,
        options: Some(ngbh.options),
    };
    let registered: Vec<sims_sim::neighbors::Registered> = boot_catalog
        .objects
        .iter()
        .map(|o| sims_sim::neighbors::Registered {
            guid: o.guid,
            kind: o.kind,
            resolves: boot_catalog.object(o.guid).is_some_and(|resolved| {
                resolved.resource == o.resource && resolved.chunk == o.chunk
            }),
        })
        .collect();
    booted_world
        .read_neighbors(&content_nbhd, &registered)
        .expect("read_neighbors failed");
    let mut people_profiles = sims_content::People::default();
    let open_skeleton = sims_content::Skeleton {
        name: "open-adult".to_owned(),
        file: "open-pack/skeleton".to_owned(),
        place: 0,
        scales: vec![vec![1]],
        bones: vec![sims_content::Bone {
            name: "ROOT".to_owned(),
            parent: None,
            translation: [0; 3],
            rotation: [0, 0, 0, 1.0f32.to_bits()],
            blends: true,
        }],
    };
    for sim_guid in [SIM_GUID, SIM2_GUID] {
        people_profiles
            .skeletons
            .insert(sim_guid, open_skeleton.clone());
        people_profiles
            .profiles
            .insert(sim_guid, sims_content::Profile::default());
    }

    let mut boot_vm = Interpreter::new(&boot_program, &boot_catalog, Stream::Lot);
    let _ = boot_vm
        .read_types(&mut booted_world, &parsed_house.types)
        .expect("read_types failed");
    let _ = boot_vm
        .read_objects(&mut booted_world, &parsed_house.objects, &people_profiles)
        .expect("read_objects failed");
    let _ = boot_vm
        .assign_rooms(&mut booted_world)
        .expect("assign_rooms failed");
    let visited = boot_vm.visit_objects(&mut booted_world, &people_profiles);
    assert!(
        visited.stopped.is_none(),
        "visit_objects failed on saved House01.iff: {:?}",
        visited.stopped
    );
    assert_eq!(booted_world.people.len(), 2);
    for &saved_sim_id in &booted_world.people.clone() {
        let sim_obj = &booted_world.objects[&saved_sim_id];
        assert_eq!(
            sim_obj.stack.len(),
            1,
            "saved Sim {saved_sim_id} must boot organically onto its Main stack frame"
        );
        assert_eq!(sim_obj.stack[0].tree, 4097);
        let status = boot_vm
            .turn(&mut booted_world, saved_sim_id, 4_000)
            .expect("saved Sim Main turn trapped");
        assert!(matches!(status, Status::Yielded));
    }

    // Run a pie-menu interaction from a booted saved Sim onto the saved Open Bookshelf in House01.iff!
    let saved_sim_id = booted_world.people[0];
    let saved_bookshelf_id = booted_world
        .objects
        .values()
        .find(|o| o.guid == BOOKSHELF_GUID)
        .expect("House01.iff missing saved Open Bookshelf")
        .id;
    let before_fun = booted_world.objects[&saved_sim_id]
        .person
        .as_ref()
        .unwrap()
        .motives[MOTIVE_FUN];
    run_pie_interaction_by_name(
        &mut boot_vm,
        &mut booted_world,
        &boot_ttas,
        saved_sim_id,
        saved_bookshelf_id,
        "Read Novel",
        80,
    );
    assert!(
        booted_world.objects[&saved_sim_id]
            .person
            .as_ref()
            .unwrap()
            .motives[MOTIVE_FUN]
            > before_fun,
        "saved Sim interacting with saved Open Bookshelf in House01.iff must gain Fun"
    );
}

#[test]
fn household_sim_exercises_all_eight_motives_skills_bills_and_cancel_zero_traps() {
    let standalone = compile_standalone_pack_dir(&open_pack_dir()).unwrap();
    const GLOBAL_RES: usize = 0;
    let mut catalog = Catalog {
        global: GLOBAL_RES,
        objects: Vec::new(),
        by_guid: BTreeMap::new(),
        semi_globals: BTreeMap::new(),
        diagnostics: Vec::new(),
    };
    let mut libraries = BTreeMap::new();
    let mut semi_globals = BTreeMap::new();
    let mut ttas_by_guid: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    libraries.insert(GLOBAL_RES, Library::default());

    for (idx, entry) in standalone.pack.objects.iter().enumerate() {
        let res_id = idx + 1;
        let iff = Iff::parse(entry.object.iff_bytes.clone()).unwrap();
        if let Some(ttas_chunk) = iff.get(b"TTAs", 128) {
            let st = StringTable::parse(iff.data(ttas_chunk)).unwrap();
            let labels: Vec<String> = st.entries.into_iter().map(|e| e.text).collect();
            ttas_by_guid.insert(entry.object.guid, labels.clone());
            for &tg in &entry.object.tile_guids {
                ttas_by_guid.insert(tg, labels.clone());
            }
        }
        let lib = load_iff_into_vm(&iff, res_id, &mut catalog);
        libraries.insert(res_id, lib);
        semi_globals.insert(res_id, None);
    }

    // Load BOTH real compiled starter Sim character `.iff`s from `standalone.characters`.
    for (next_res, char_bytes) in
        (standalone.pack.objects.len() + 1..).zip(standalone.characters.values())
    {
        let iff = Iff::parse(char_bytes.clone()).unwrap();
        let lib = load_iff_into_vm(&iff, next_res, &mut catalog);
        libraries.insert(next_res, lib);
        semi_globals.insert(next_res, None);
    }

    let program = Program {
        global: GLOBAL_RES,
        libraries,
        semi_globals,
        careers: None,
        career_texts_end: true,
        labels_hold_no_token: true,
        zoning: BTreeMap::new(),
        animations: Default::default(),
        people: Default::default(),
        icon_frames: None,
        wardrobe: None,
        ordered_wardrobe: None,
        character_files: Default::default(),
        expansions: 0,
        characters: Default::default(),
    };

    let mut world = World::empty();
    world.size = 16;
    world.globals = vec![0; 64];
    world.globals[1] = 1;
    world.globals[7] = 1;
    world.globals[8] = 2000;
    world.globals[10] = 1;
    world.globals[13] = 1000;
    world.globals[14] = 0;
    world.budget.funds = 1000;
    world.budget.days = vec![Some([0; 8])];
    world.lot_rng = GameRng::from_state(11);
    world.global_rng = GameRng::from_state(22);
    world.arrays.insert(0, make_lot_array(4, 0));
    for (floors, walls, flags) in [(1, 2, 8), (101, 102, 108)] {
        world.arrays.insert(floors, make_lot_array(2, 0));
        world.arrays.insert(walls, make_lot_array(14, 0));
        world.arrays.insert(flags, make_lot_array(1, 0));
    }
    world.arrays.insert(3, make_lot_array(2, 0));
    world.arrays.insert(103, make_lot_array(2, 0));
    world.arrays.insert(9, make_lot_array(1, 0xff));
    world.arrays.insert(10, make_lot_array(1, 0xff));
    world.rooms.map = [vec![1; 64 * 64], vec![1024; 64 * 64]];
    world.rooms.rooms.insert(
        1,
        Room {
            outdoors: false,
            ..Room::default()
        },
    );

    let sim_id = 1u16;
    let sim2_id = 2u16;
    world.order.push(sim_id);
    world.order.push(sim2_id);
    world.people.push(sim_id);
    world.people.push(sim2_id);
    world.types.insert(1, SIM_GUID);
    world.types.insert(2, SIM2_GUID);
    world
        .objects
        .insert(sim_id, make_sim_object(sim_id, SIM_GUID));
    world
        .objects
        .insert(sim2_id, make_sim_object(sim2_id, SIM2_GUID));
    world.indicators = [u16::MAX, u16::MAX];

    let sound_log: RefCell<Vec<SoundEvent>> = RefCell::new(Vec::new());
    let mut vm = Interpreter::new(&program, &catalog, Stream::Lot);
    vm.sounds = Some(&sound_log);

    let spawn = |vm: &mut Interpreter<'_>, world: &mut World, guid: u32, tx: i32, ty: i32| -> u16 {
        let id = vm.create(world, guid).expect("vm.create failed");
        place_on_tile(world, id, tx, ty);
        id
    };

    let toilet = spawn(&mut vm, &mut world, TOILET_GUID, 2, 2);
    let shower = spawn(&mut vm, &mut world, SHOWER_GUID, 3, 2);
    let cot = spawn(&mut vm, &mut world, COT_GUID, 4, 2);
    let sofa = spawn(&mut vm, &mut world, SOFA_GUID, 5, 2);
    let tv = spawn(&mut vm, &mut world, TV_GUID, 6, 2);
    let bookshelf = spawn(&mut vm, &mut world, BOOKSHELF_GUID, 7, 2);
    let dining_table = spawn(&mut vm, &mut world, DINING_TABLE_GUID, 2, 4);
    let dining_chair = spawn(&mut vm, &mut world, DINING_CHAIR_GUID, 3, 4);
    let floor_lamp = spawn(&mut vm, &mut world, FLOOR_LAMP_GUID, 4, 4);
    let trash_can = spawn(&mut vm, &mut world, TRASH_CAN_GUID, 5, 4);
    let front_door = spawn(&mut vm, &mut world, FRONT_DOOR_GUID, 6, 4);
    let mailbox = spawn(&mut vm, &mut world, MAILBOX_GUID, 7, 4);
    let fridge = spawn(&mut vm, &mut world, FRIDGE_GUID, 2, 6);
    let _counter = spawn(&mut vm, &mut world, COUNTER_GUID, 3, 6);
    let _stove = spawn(&mut vm, &mut world, STOVE_GUID, 4, 6);

    // 1. Bladder -> Open Toilet
    let before_bladder = world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_BLADDER];
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        toilet,
        "Use Toilet",
        60,
    );
    assert!(
        world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_BLADDER]
            > before_bladder + 40.0
    );
    assert_eq!(world.objects[&toilet].attributes[0], 0);

    // 2. Hygiene -> Open Shower
    let before_hygiene = world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_HYGIENE];
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        shower,
        "Take Shower",
        80,
    );
    assert!(
        world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_HYGIENE]
            > before_hygiene + 50.0
    );
    assert_eq!(world.objects[&shower].attributes[0], 0);

    // 3. Energy -> Open Cot
    let before_energy = world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_ENERGY];
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        cot,
        "Take a Nap",
        80,
    );
    assert!(
        world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_ENERGY]
            > before_energy + 40.0
    );

    // 4. Comfort -> Open Sofa & Dining Chair
    let before_comfort = world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_COMFORT];
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        sofa,
        "Relax",
        80,
    );
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        dining_chair,
        "Sit",
        80,
    );
    assert!(
        world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_COMFORT]
            > before_comfort + 40.0
    );
    assert_eq!(world.objects[&sofa].attributes[0], 0);
    assert_eq!(world.objects[&dining_chair].attributes[0], 0);

    // 5. Fun + Power Toggle -> Open TV
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        tv,
        "Turn On",
        20,
    );
    assert_eq!(world.objects[&tv].attributes[1], 1);
    let before_fun = world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_FUN];
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        tv,
        "Watch TV",
        80,
    );
    assert!(
        world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_FUN] > before_fun + 40.0
    );
    assert_eq!(world.objects[&tv].attributes[0], 0);
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        tv,
        "Turn Off",
        20,
    );
    assert_eq!(world.objects[&tv].attributes[1], 0);

    // 6. Skill Study (Cooking + Mechanical) -> Open Bookshelf
    let before_cook = world.objects[&sim_id].person.as_ref().unwrap().data[SKILL_COOKING];
    let before_mech = world.objects[&sim_id].person.as_ref().unwrap().data[SKILL_MECHANICAL];
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        bookshelf,
        "Study Cooking",
        80,
    );
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        bookshelf,
        "Study Mechanical",
        80,
    );
    assert!(world.objects[&sim_id].person.as_ref().unwrap().data[SKILL_COOKING] > before_cook);
    assert!(world.objects[&sim_id].person.as_ref().unwrap().data[SKILL_MECHANICAL] > before_mech);
    assert_eq!(world.objects[&bookshelf].attributes[0], 0);

    // 7. Social -> Open Front Door + Sofa Chat
    let before_social = world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_SOCIAL];
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        front_door,
        "Knock and Welcome",
        40,
    );
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        sofa,
        "Chat on Sofa",
        60,
    );
    assert!(
        world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_SOCIAL]
            > before_social + 50.0
    );
    assert_eq!(world.objects[&front_door].attributes[1], 1);

    // 8. Room -> Open Floor Lamp + Dining Table + Trash Can
    let before_room = world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_ROOM];
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        floor_lamp,
        "Enjoy Warm Light",
        40,
    );
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        dining_table,
        "Set Table",
        40,
    );
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        trash_can,
        "Empty Trash",
        40,
    );
    assert!(
        world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_ROOM] > before_room + 50.0
    );
    assert_eq!(world.objects[&trash_can].attributes[1], 0);

    // 9. Bills via alter_budget -> Open Mailbox
    let before_funds = world.budget.funds;
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        mailbox,
        "Pay Bills",
        40,
    );
    assert_eq!(world.budget.funds, before_funds - 75);
    assert_eq!(world.objects[&mailbox].attributes[1], 0);
    assert_eq!(world.objects[&mailbox].attributes[2], 1);

    // 10. Hunger -> Cooking chain (Fridge -> Counter -> Stove -> Meal Plate)
    let before_hunger = world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_HUNGER];
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        fridge,
        "Cook Meal",
        100,
    );
    let plate_id = world.objects[&fridge].attributes[2] as u16;
    assert_eq!(world.objects[&plate_id].guid, PLATE_GUID);
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim_id,
        plate_id,
        "Eat Meal",
        80,
    );
    assert!(
        world.objects[&sim_id].person.as_ref().unwrap().motives[MOTIVE_HUNGER]
            > before_hunger + 25.0
    );

    // 11. Mid-loop native `vm.cancel` cleanup check on Open Bookshelf
    let entries = vm.pie_menu(&mut world, sim_id, bookshelf).unwrap();
    let book_names = &ttas_by_guid[&BOOKSHELF_GUID];
    let read_entry = entries
        .into_iter()
        .find(|e| {
            book_names
                .get(e.action.table_index as usize)
                .is_some_and(|s| s == "Read Novel")
        })
        .unwrap();
    let uid = vm
        .pick(&mut world, sim_id, read_entry.action)
        .unwrap()
        .unwrap();
    world.objects.get_mut(&sim_id).unwrap().data[25] = 1;
    vm.turn(&mut world, sim_id, 4_000).unwrap();
    assert_eq!(
        world.objects[&bookshelf].attributes[0], 1,
        "lock held mid-loop"
    );
    assert!(vm.cancel(&mut world, sim_id, uid).unwrap());
    for _ in 0..20 {
        world.objects.get_mut(&sim_id).unwrap().data[25] = 1;
        vm.turn(&mut world, sim_id, 4_000).unwrap();
        if world.objects[&sim_id].stack.len() == 1 {
            break;
        }
    }
    assert_eq!(
        world.objects[&bookshelf].attributes[0], 0,
        "mid-loop vm.cancel must cleanly release bookshelf occupancy lock"
    );

    // Exercise second starter Sim (Sam Sunvale, 0x7F00_0102) on sofa & bookshelf too!
    let sim2_before_fun = world.objects[&sim2_id].person.as_ref().unwrap().motives[MOTIVE_FUN];
    run_pie_interaction_by_name(
        &mut vm,
        &mut world,
        &ttas_by_guid,
        sim2_id,
        bookshelf,
        "Read Novel",
        60,
    );
    assert!(world.objects[&sim2_id].person.as_ref().unwrap().motives[MOTIVE_FUN] > sim2_before_fun);

    let played_count = sound_log
        .borrow()
        .iter()
        .filter(|e| matches!(e, SoundEvent::Play { .. }))
        .count();
    assert!(
        played_count >= 10,
        "expected >=10 sound events across household, got {played_count}"
    );
}

#[test]
fn cli_verify_build_and_equiv_subcommands_work_end_to_end() {
    use std::process::Command;

    struct CleanupDir(PathBuf);
    impl Drop for CleanupDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let bin = env!("CARGO_BIN_EXE_sims-antics");
    let pack = open_pack_dir();

    // 1. `sims-antics verify <pack_dir>` succeeds with 20 objects and 0 violations.
    let verify_out = Command::new(bin)
        .arg("verify")
        .arg(&pack)
        .output()
        .expect("failed to execute sims-antics verify");
    assert!(
        verify_out.status.success(),
        "sims-antics verify failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&verify_out.stdout),
        String::from_utf8_lossy(&verify_out.stderr),
    );
    let verify_stdout = String::from_utf8_lossy(&verify_out.stdout);
    assert!(
        verify_stdout.contains("VERIFIED:"),
        "stdout: {verify_stdout}"
    );
    assert!(
        verify_stdout.contains("20 objects"),
        "stdout: {verify_stdout}"
    );
    assert!(
        verify_stdout.contains("0 violations"),
        "stdout: {verify_stdout}"
    );

    // 2. `sims-antics build <pack_dir> --out <dir>` writes the full standalone world.
    let tmp_root =
        std::env::temp_dir().join(format!("sims-antics-cli-standalone-{}", std::process::id()));
    let _cleanup = CleanupDir(tmp_root.clone());
    let _ = std::fs::remove_dir_all(&tmp_root);
    let out_dir = tmp_root.join("out");

    let build_out = Command::new(bin)
        .arg("build")
        .arg(&pack)
        .arg("--out")
        .arg(&out_dir)
        .output()
        .expect("failed to execute sims-antics build");
    assert!(
        build_out.status.success(),
        "sims-antics build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build_out.stdout),
        String::from_utf8_lossy(&build_out.stderr),
    );
    assert!(out_dir.join("open-pack.far").is_file());
    assert!(out_dir.join("objects").join("open_toilet.iff").is_file());
    assert!(out_dir.join("floors").join("open_oak_plank.flr").is_file());
    assert!(
        out_dir
            .join("walls")
            .join("open_cream_plaster.wll")
            .is_file()
    );
    assert!(out_dir.join("characters").join("open_sim_01.iff").is_file());
    assert!(out_dir.join("characters").join("open_sim_02.iff").is_file());
    assert!(out_dir.join("houses").join("House01.iff").is_file());
    assert!(
        out_dir
            .join("neighborhood")
            .join("Neighborhood.iff")
            .is_file()
    );

    // 3. `sims-antics equiv <a.antics> <b.iff> --tree 4099 --pack <pack>` succeeds on matching v2 script.
    let toilet_antics = pack.join("objects").join("open-toilet.antics");
    let toilet_iff = out_dir.join("objects").join("open_toilet.iff");
    let equiv_ok = Command::new(bin)
        .arg("equiv")
        .arg(&toilet_antics)
        .arg(&toilet_iff)
        .arg("--tree")
        .arg("4099")
        .arg("--pack")
        .arg(&pack)
        .output()
        .expect("failed to execute sims-antics equiv");
    assert!(
        equiv_ok.status.success(),
        "sims-antics equiv expected success:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&equiv_ok.stdout),
        String::from_utf8_lossy(&equiv_ok.stderr),
    );
    assert!(
        String::from_utf8_lossy(&equiv_ok.stdout).contains("EQUIVALENT:"),
        "stdout: {}",
        String::from_utf8_lossy(&equiv_ok.stdout)
    );

    // 4. `sims-antics equiv` fails with COUNTEREXAMPLE when v2 script is subtly mutated.
    let mut_pack = tmp_root.join("mut_pack");
    std::fs::create_dir_all(mut_pack.join("common")).unwrap();
    std::fs::create_dir_all(mut_pack.join("templates")).unwrap();
    std::fs::create_dir_all(mut_pack.join("objects")).unwrap();
    std::fs::copy(pack.join("pack.toml"), mut_pack.join("pack.toml")).unwrap();
    std::fs::copy(
        pack.join("common/helpers.antics"),
        mut_pack.join("common/helpers.antics"),
    )
    .unwrap();
    std::fs::copy(
        pack.join("templates/plumbing.antics"),
        mut_pack.join("templates/plumbing.antics"),
    )
    .unwrap();
    let orig_src = std::fs::read_to_string(&toilet_antics).unwrap();
    let mutated_src = orig_src.replace("play_sound(flush);", "play_sound(plunge);");
    assert_ne!(orig_src, mutated_src);
    let mutated_path = mut_pack.join("objects/open-toilet.antics");
    std::fs::write(&mutated_path, mutated_src).unwrap();

    let equiv_fail = Command::new(bin)
        .arg("equiv")
        .arg(&mutated_path)
        .arg(&toilet_iff)
        .arg("--tree")
        .arg("4099")
        .arg("--pack")
        .arg(&mut_pack)
        .output()
        .expect("failed to execute sims-antics equiv (mutated)");
    assert!(
        !equiv_fail.status.success(),
        "expected non-zero exit for mutated script"
    );
    assert!(
        String::from_utf8_lossy(&equiv_fail.stderr).contains("COUNTEREXAMPLE"),
        "stderr: {}",
        String::from_utf8_lossy(&equiv_fail.stderr)
    );
}
