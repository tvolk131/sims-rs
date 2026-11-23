//! End-to-end on-disk `open-pack/` compilation and authentic `sims-vm` behavioral stress suite.
//!
//! Compiles the entire `open-pack/` source tree from disk into `open-pack.far`,
//! verifies every member `.iff` and procedural asset via `sims_formats`, and
//! exercises all 4 complex life-sim patterns through REAL `sims-vm` entry points
//! (`vm.pie_menu`, `vm.pick`, Sim `IdleForInput` execution with `me = Sim` and
//! `stack_obj = Furniture`, real `vm.cancel`, and real on-lot placement/removal):
//!   A. Multi-Tile Double Bed (`open-double-bed`: master `0x7F00_0012`, lead `0x7F00_0010`, slave `0x7F00_0011`)
//!   B. Multi-Object Cooking Chain (`open-fridge` -> `open-counter` -> `open-stove` -> on-lot `open-meal-plate`)
//!   C. Interactive Wall Phone (`open-wall-phone`: 2-way & 3-way `dialog` branching + FWAV SFX)
//!   D. Repairable Privacy Shower (`open-shower`: guard trees, wear->broken, Mechanic-scaled repair, `vm.cancel`)

use sims_antics::{CompiledPack, compile_antics, compile_object_from_sources, compile_pack_dir};
use sims_content::Catalog;
use sims_core::GameRng;
use sims_formats::behavior::{Behavior, InteractionTable, ObjectDefinition};
use sims_formats::execution::{ObjectFunction, ObjectFunctions, Slots, constants};
use sims_formats::far;
use sims_formats::iff::Iff;
use sims_formats::lot::Array as LotArray;
use sims_formats::saved_objects::{
    Frame, Interaction as SavedInteraction, Person, Slot as SavedSlot,
};
use sims_formats::sprite::{DrawGroup, Palette, SpriteSet, render};
use sims_formats::strings::StringTable;
use sims_sim::rooms::Room;
use sims_sim::{Object, World};
use sims_vm::{DEFINITION_WORDS, Interpreter, Library, Program, SoundEvent, Status, Stream};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

// Dialog answer status codes from `sims-vm/src/primitives/dialog.rs`.
const DIALOG_FIRST: u8 = 2;
const DIALOG_SECOND: u8 = 3;
const DIALOG_THIRD: u8 = 4;
const DIALOG_DONE: u8 = 5;

const BENCH_GUID: u32 = 0x7F00_0001;
const COT_GUID: u32 = 0x7F00_0002;
const BASIN_GUID: u32 = 0x7F00_0003;
const BED_LEAD_GUID: u32 = 0x7F00_0010;
const BED_SLAVE_GUID: u32 = 0x7F00_0011;
const BED_MASTER_GUID: u32 = 0x7F00_0012;
const FRIDGE_GUID: u32 = 0x7F00_0020;
const COUNTER_GUID: u32 = 0x7F00_0021;
const STOVE_GUID: u32 = 0x7F00_0022;
const PLATE_GUID: u32 = 0x7F00_0023;
const PHONE_GUID: u32 = 0x7F00_0030;
const SHOWER_GUID: u32 = 0x7F00_0040;
const SIM_GUID: u32 = 0x7F00_00FF;

const MOTIVE_ENERGY: usize = 5;
const MOTIVE_COMFORT: usize = 6;
const MOTIVE_HUNGER: usize = 7;
const MOTIVE_HYGIENE: usize = 8;
const MOTIVE_BLADDER: usize = 9;
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

/// Builds a `sims_vm::Library` and catalog entries from a compiled IFF file,
/// preserving exact OBJD words, OBJf callbacks, BCONs, STR# tables, FWAVs,
/// TTABs, and SLOTs.
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

fn make_sim_object(id: u16) -> Object {
    let mut motives = [0.0f32; 16];
    motives[MOTIVE_ENERGY] = -40.0;
    motives[MOTIVE_COMFORT] = -20.0;
    motives[MOTIVE_HUNGER] = -50.0;
    motives[MOTIVE_HYGIENE] = -30.0;
    motives[MOTIVE_BLADDER] = -60.0;
    motives[MOTIVE_SOCIAL] = -10.0;
    motives[MOTIVE_FUN] = 0.0;

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
    data[29] = 1; // room 1
    data[67] = 5;

    let mut person_data = vec![0i16; 256];
    person_data[58] = 25; // adult age

    Object {
        id,
        guid: SIM_GUID,
        footprint: [0; 4],
        position: [32, 32, 1],
        unknown_short: 0,
        attributes: vec![0; 8],
        temps: vec![0; 8],
        data,
        obsolete_word: 0,
        stack_capacity_bytes: 512,
        // Sim runs `SimMain` (tree 4097): a real IdleForInput(param[0], 1) dispatch loop!
        stack: vec![Frame {
            stack_object: 0,
            tree: 4097,
            node: 0,
            parameters: vec![0],
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
            previous_motives: [0.0; 16],
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

struct PackVmFixture {
    program: Program,
    catalog: Catalog,
    world: World,
    sim_id: u16,
}

impl PackVmFixture {
    fn from_pack(pack: &CompiledPack, starting_funds: i32) -> Self {
        const GLOBAL_RES: usize = 0;

        let mut catalog = Catalog {
            global: GLOBAL_RES,
            objects: Vec::new(),
            by_guid: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            diagnostics: Vec::new(),
        };

        let mut libraries = BTreeMap::from([(GLOBAL_RES, Library::default())]);
        let mut semi_globals = BTreeMap::new();

        // Parse every member directly back out of the packaged FAR archive!
        let manifest_off =
            far::manifest_offset(&pack.far_bytes[..16], pack.far_bytes.len() as u64).unwrap();
        let dir_len = far::directory_len(manifest_off, pack.far_bytes.len() as u64).unwrap();
        let dir = far::directory(
            &pack.far_bytes[manifest_off as usize..manifest_off as usize + dir_len],
            pack.far_bytes.len() as u64,
        )
        .unwrap();
        assert_eq!(dir.len(), pack.objects.len());

        for (idx, entry) in dir.iter().enumerate() {
            let res_id = idx + 1;
            assert!(entry.within_payload(manifest_off));
            let slice =
                &pack.far_bytes[entry.offset as usize..(entry.offset + entry.stored_size) as usize];
            let iff = Iff::parse(slice.to_vec()).expect("FAR member IFF failed to parse");
            assert!(
                iff.map_error().is_none(),
                "IFF rsmp error in {}",
                entry.name
            );
            let lib = load_iff_into_vm(&iff, res_id, &mut catalog);
            libraries.insert(res_id, lib);
            semi_globals.insert(res_id, None);
        }

        // Register synthetic test Sim person with a real `IdleForInput` Main loop.
        let sim_res = dir.len() + 1;
        let sim_module = compile_antics(
            r#"
tree 4097 "SimMain" (args: 1) {
    while 0 == 0 {
        param[0] := 1;
        idle_for_input(param[0], 1);
    }
}
"#,
        )
        .unwrap();
        let mut sim_lib = Library::default();
        for (&id, bhav) in &sim_module.behaviors {
            let raw = sims_antics::encode::encode_bhav(bhav).unwrap();
            sim_lib.behaviors.insert(id, Behavior::parse(&raw).unwrap());
        }
        let mut sim_words = [0i16; DEFINITION_WORDS];
        sim_words[9] = 2; // person
        sim_words[14] = SIM_GUID as u16 as i16;
        sim_words[15] = (SIM_GUID >> 16) as u16 as i16;
        sim_lib.definitions.insert(128, sim_words);
        sim_lib.definition_order.push(128);
        sim_lib.callbacks.insert(
            128,
            vec![
                ObjectFunction {
                    condition: 0,
                    action: 0,
                },
                ObjectFunction {
                    condition: 0,
                    action: 4097,
                },
            ],
        );
        libraries.insert(sim_res, sim_lib);
        semi_globals.insert(sim_res, None);
        let sim_cat_idx = catalog.objects.len();
        catalog.objects.push(sims_content::Object {
            guid: SIM_GUID,
            resource: sim_res,
            chunk: 128,
            kind: 2,
            master: 0,
        });
        catalog.by_guid.insert(SIM_GUID, sim_cat_idx);

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

        let sim_id = 1u16;
        let mut world = World::empty();
        world.size = 16;
        world.globals = vec![0; 64];
        world.globals[1] = 1; // day
        world.globals[7] = 1; // month
        world.globals[8] = 2000; // year
        world.globals[10] = 1; // lot number
        world.globals[14] = (starting_funds / 10000) as i16;
        world.globals[13] = (starting_funds % 10000) as i16;
        world.budget.funds = starting_funds;
        world.budget.days = vec![Some([0; 8])];
        world.lot_rng = GameRng::from_state(11);
        world.global_rng = GameRng::from_state(22);

        // Minimal flat 16x16 lot with indoor room 1 so on-lot placement (create_object
        // mode stack_ahead) and remove_object run real placement & take-off code!
        world.arrays.insert(0, make_lot_array(4, 0)); // heights
        for (floors, walls, flags) in [(1, 2, 8), (101, 102, 108)] {
            world.arrays.insert(floors, make_lot_array(2, 0));
            world.arrays.insert(walls, make_lot_array(14, 0));
            world.arrays.insert(flags, make_lot_array(1, 0));
        }
        world.arrays.insert(3, make_lot_array(2, 0)); // level 1 object grid
        world.arrays.insert(103, make_lot_array(2, 0)); // level 2 object grid
        world.arrays.insert(9, make_lot_array(1, 0xff)); // pool (dry)
        world.arrays.insert(10, make_lot_array(1, 0xff)); // water (dry)
        world.rooms.map = [vec![1; 64 * 64], vec![1024; 64 * 64]];
        world.rooms.rooms.insert(
            1,
            Room {
                outdoors: false,
                ..Room::default()
            },
        );

        world.order.push(sim_id);
        world.people.push(sim_id);
        world.types.insert(1, SIM_GUID);
        world.objects.insert(sim_id, make_sim_object(sim_id));
        world.indicators = [u16::MAX, u16::MAX];

        Self {
            program,
            catalog,
            world,
            sim_id,
        }
    }

    /// Places a created object onto a tile `(tx, ty)` on level 1 (updating its room,
    /// position, footprint, and the level-1 object grid so `create_object` and
    /// `remove_object` see a valid on-lot object).
    fn place_on_tile(world: &mut World, obj_id: u16, tx: i32, ty: i32, facing: i16) {
        let px = tx * 16 + 8;
        let py = ty * 16 + 8;
        let obj = world.objects.get_mut(&obj_id).unwrap();
        obj.position = [px, py, 1];
        obj.footprint = [px - 8, py - 8, px + 8, py + 8];
        obj.data[1] = facing;
        obj.data[29] = 1; // room 1
        let idx = (tx as usize * 64 + ty as usize) * 2;
        let grid = &mut world.arrays.get_mut(&3).unwrap().data;
        grid[idx..idx + 2].copy_from_slice(&(obj_id as i16).to_le_bytes());
    }

    /// Lists pie-menu entries offered on `target` for `sim_id`, returning `(menu_index, name)` pairs.
    fn pie_menu_names(
        vm: &mut Interpreter<'_>,
        world: &mut World,
        sim_id: u16,
        target: u16,
    ) -> Vec<(u32, String)> {
        let entries = vm
            .pie_menu(world, sim_id, target)
            .expect("pie_menu failed with VM trap");
        entries
            .into_iter()
            .map(|e| {
                let target_guid = world.objects[&(e.action.target as u16)].guid;
                let def = vm.catalog.object(target_guid).unwrap();
                let ttab = vm.program.libraries[&def.resource].interactions[&128]
                    .as_ref()
                    .unwrap();
                let name = e.name.unwrap_or_else(|| {
                    ttab.interactions
                        .iter()
                        .position(|i| i.string_index == e.action.table_index as u32)
                        .map(|idx| format!("entry_{idx}"))
                        .unwrap_or_default()
                });
                (e.action.table_index as u32, name)
            })
            .collect()
    }

    /// Picks a pie-menu interaction by `table_index` on `target` via real `vm.pie_menu`
    /// and `vm.pick`, then steps the Sim's `IdleForInput` thread until the interaction finishes.
    ///
    /// Invokes `on_yield(world, yields_while_active)` on each yield while the interaction is running.
    fn run_pie_interaction(
        vm: &mut Interpreter<'_>,
        world: &mut World,
        sim_id: u16,
        target: u16,
        table_index: u32,
        max_turns: usize,
        mut on_yield: impl FnMut(&mut World, usize),
    ) -> usize {
        let entries = vm.pie_menu(world, sim_id, target).expect("pie_menu failed");
        let chosen = entries
            .into_iter()
            .find(|e| e.action.table_index as u32 == table_index)
            .unwrap_or_else(|| {
                panic!("pie_menu on object {target} did not offer interaction index {table_index}")
            });
        let uid = vm
            .pick(world, sim_id, chosen.action)
            .expect("pick failed")
            .expect("action queue rejected picked interaction");
        assert!(uid > 0);

        let mut active_yields = 0usize;
        let mut saw_started = false;
        for _ in 0..max_turns {
            // Simulate normal scheduler lockout decay so Sleep/idle counts down each turn.
            world.objects.get_mut(&sim_id).unwrap().data[25] = 1;
            let status = vm
                .turn(world, sim_id, 4_000)
                .expect("VM trap during Sim interaction turn");
            let stack_depth = world.objects[&sim_id].stack.len();
            if stack_depth > 1 {
                saw_started = true;
                if matches!(status, Status::Yielded) {
                    active_yields += 1;
                    on_yield(world, active_yields);
                }
            } else if saw_started {
                // Interaction frame returned cleanly to SimMain's IdleForInput!
                // Clear Sim's active action & DONE dialogs as end-of-tick does.
                if let Some(person) = world.objects.get_mut(&sim_id).unwrap().person.as_mut() {
                    person.active.uid = 0;
                    person.active.flags = 0;
                    person.data[33] = 0;
                    person.object_uses.clear();
                }
                world.dialogs.records.retain(|r| r.status != DIALOG_DONE);
                return active_yields;
            } else if matches!(status, Status::Yielded) {
                // Non-yielding interactions start and finish within the very same turn,
                // moving the completed action into `person.last` and clearing `person.active`.
                let person = world.objects[&sim_id].person.as_ref().unwrap();
                if person.queue.is_empty() && (person.active.uid == uid || person.last.uid == uid) {
                    if let Some(person) = world.objects.get_mut(&sim_id).unwrap().person.as_mut() {
                        person.active.uid = 0;
                        person.active.flags = 0;
                        person.data[33] = 0;
                        person.object_uses.clear();
                    }
                    world.dialogs.records.retain(|r| r.status != DIALOG_DONE);
                    return active_yields;
                }
            }
        }
        panic!(
            "picked interaction {table_index} on object {target} did not complete within {max_turns} turns"
        );
    }
}

fn played_sounds(sounds: &RefCell<Vec<SoundEvent>>) -> Vec<String> {
    sounds
        .borrow()
        .iter()
        .filter_map(|e| match e {
            SoundEvent::Play { name, .. } => Some(name.clone()),
            SoundEvent::Stop { .. } => None,
        })
        .collect()
}

#[test]
fn open_pack_compiles_from_disk_and_all_chunks_and_sprites_validate_without_clipping() {
    let pack = compile_pack_dir(&open_pack_dir()).expect("open-pack compile failed");
    assert_eq!(pack.name, "Open Starter Pack");
    assert_eq!(pack.archive_name, "open-pack.far");
    assert_eq!(pack.objects.len(), 20);

    // Determinism check: compiling twice yields bit-identical FAR and WAV bytes.
    let pack2 = compile_pack_dir(&open_pack_dir()).expect("second compile failed");
    assert_eq!(pack.far_bytes, pack2.far_bytes);
    assert_eq!(pack.sounds_wav, pack2.sounds_wav);

    let manifest_off =
        far::manifest_offset(&pack.far_bytes[..16], pack.far_bytes.len() as u64).unwrap();
    let dir_len = far::directory_len(manifest_off, pack.far_bytes.len() as u64).unwrap();
    let dir = far::directory(
        &pack.far_bytes[manifest_off as usize..manifest_off as usize + dir_len],
        pack.far_bytes.len() as u64,
    )
    .unwrap();
    assert_eq!(dir.len(), 20);

    for (member, entry) in pack.objects.iter().zip(&dir) {
        assert_eq!(entry.name, member.iff_name);
        assert!(entry.within_payload(manifest_off));
        let slice =
            &pack.far_bytes[entry.offset as usize..(entry.offset + entry.stored_size) as usize];
        let iff = Iff::parse(slice.to_vec()).expect("IFF parse failed");
        assert!(
            iff.map_error().is_none(),
            "IFF rsmp error in {}",
            member.slug
        );

        // OBJD + OBJf for every definition, and every referenced DGRP exists!
        let objds: Vec<_> = iff.all(b"OBJD").collect();
        assert!(!objds.is_empty());
        for objd_chunk in objds {
            let parsed = ObjectDefinition::parse(iff.data(objd_chunk)).unwrap();
            assert!(parsed.guid >= 0x7F00_0001);
            let objf_chunk = iff
                .get(b"OBJf", objd_chunk.id)
                .expect("missing matching OBJf for OBJD");
            let objf = ObjectFunctions::parse(iff.data(objf_chunk)).unwrap();
            assert!(objf.entries.len() >= 2);
            if parsed.graphic != 0 {
                assert!(
                    iff.get(b"DGRP", parsed.graphic).is_some(),
                    "{} OBJD #{} references missing DGRP #{}",
                    member.slug,
                    objd_chunk.id,
                    parsed.graphic
                );
            }
        }

        // Verify double bed emits distinct left/right tile DGRPs (101 & 102) in addition to master (100).
        if member.slug == "open-double-bed" {
            assert!(iff.get(b"DGRP", 100).is_some());
            assert!(iff.get(b"DGRP", 101).is_some());
            assert!(iff.get(b"DGRP", 102).is_some());
        }

        // CTSS
        let ctss_chunk = iff.get(b"CTSS", 2000).expect("missing CTSS #2000");
        let ctss = StringTable::parse(iff.data(ctss_chunk)).unwrap();
        assert_eq!(ctss.get(0, 1), Some(member.object.name.as_str()));

        // TTAB + TTAs + SLOT
        let ttab_chunk = iff.get(b"TTAB", 128).expect("missing TTAB #128");
        let ttab = InteractionTable::parse(iff.data(ttab_chunk)).unwrap();
        let ttas_chunk = iff.get(b"TTAs", 128).expect("missing TTAs #128");
        let ttas = StringTable::parse(iff.data(ttas_chunk)).unwrap();
        assert_eq!(ttab.interactions.len(), ttas.entries.len());

        let slot_chunk = iff.get(b"SLOT", 128).expect("missing SLOT #128");
        let slots = Slots::parse(iff.data(slot_chunk)).unwrap();
        assert!(!slots.entries.is_empty());

        // Validate EVERY emitted PALT + SPR2 + DGRP across all 12 views (4 rotations * 3 zooms),
        // AND verify ZERO border-pixel clipping on every rendered sprite (including tall Fridge/Shower)!
        for dgrp_chunk in iff.all(b"DGRP") {
            let palt = Palette::parse(iff.data(iff.get(b"PALT", dgrp_chunk.id).unwrap())).unwrap();
            assert!(palt.0.len() >= 4);
            let spr2 =
                SpriteSet::parse(iff.data(iff.get(b"SPR2", dgrp_chunk.id).unwrap())).unwrap();
            assert_eq!(spr2.frames.len(), 12);
            let dgrp = DrawGroup::parse(iff.data(dgrp_chunk)).unwrap();
            assert_eq!(dgrp.views.len(), 12);
            for (v_idx, view) in dgrp.views.iter().enumerate() {
                let rendered = render(&iff, view).expect("DGRP view render failed");
                assert!(rendered.width > 0 && rendered.height > 0);
                // Top and bottom rows of the canvas must remain transparent (no vertical clipping).
                let w = rendered.width as usize;
                let h = rendered.height as usize;
                for x in 0..w {
                    let top_alpha = rendered.rgba[x * 4 + 3];
                    let bot_alpha = rendered.rgba[((h - 1) * w + x) * 4 + 3];
                    assert_eq!(
                        top_alpha, 0,
                        "{} DGRP #{} view {v_idx} clipped top row at x={x}",
                        member.slug, dgrp_chunk.id
                    );
                    assert_eq!(
                        bot_alpha, 0,
                        "{} DGRP #{} view {v_idx} clipped bottom row at x={x}",
                        member.slug, dgrp_chunk.id
                    );
                }
            }
        }

        for chunk in iff.all(b"BHAV") {
            Behavior::parse(iff.data(chunk)).expect("BHAV parse failed");
        }
        for chunk in iff.all(b"BCON") {
            constants(iff.data(chunk)).expect("BCON parse failed");
        }
        for chunk in iff.all(b"STR#") {
            StringTable::parse(iff.data(chunk)).expect("STR# parse failed");
        }
    }

    assert!(pack.sounds_wav.len() >= 12);
    for (name, wav) in &pack.sounds_wav {
        assert!(wav.len() > 44, "WAV '{name}' too small");
        assert_eq!(&wav[..4], b"RIFF", "WAV '{name}' missing RIFF");
        assert_eq!(&wav[8..12], b"WAVE", "WAV '{name}' missing WAVE");
    }
}

#[test]
fn vm_migrated_starter_objects_run_via_pie_menu_and_sim_queue() {
    let pack = compile_pack_dir(&open_pack_dir()).unwrap();
    let mut fx = PackVmFixture::from_pack(&pack, 500);
    let sounds = RefCell::new(Vec::new());
    let mut vm = Interpreter::new(&fx.program, &fx.catalog, Stream::Lot);
    vm.sounds = Some(&sounds);

    let bench = vm.create(&mut fx.world, BENCH_GUID).unwrap();
    let cot = vm.create(&mut fx.world, COT_GUID).unwrap();
    let basin = vm.create(&mut fx.world, BASIN_GUID).unwrap();
    let sim = fx.sim_id;

    // Bench: Sit and Rest (+40 comfort) via real pie_menu + pick + Sim execution!
    assert_eq!(
        PackVmFixture::pie_menu_names(&mut vm, &mut fx.world, sim, bench).len(),
        1
    );
    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, bench, 0, 32, |_, _| {});
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_COMFORT],
        20.0
    );

    // Cot: Take a Nap yields 6 turns (3 * idle(2)), +60 energy!
    let yields =
        PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, cot, 0, 32, |_, _| {});
    assert_eq!(yields, 6);
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_ENERGY],
        20.0
    );

    // Cot: Native player cancellation via `vm.cancel` exits early and clears occupancy!
    fx.world
        .objects
        .get_mut(&sim)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .motives[MOTIVE_ENERGY] = -40.0;
    let cot_entries = vm.pie_menu(&mut fx.world, sim, cot).unwrap();
    let cot_uid = vm
        .pick(&mut fx.world, sim, cot_entries[0].action.clone())
        .unwrap()
        .unwrap();
    let mut cot_cancelled = false;
    let mut cot_cancel_yields = 0usize;
    for _ in 0..32 {
        fx.world.objects.get_mut(&sim).unwrap().data[25] = 1;
        let status = vm.turn(&mut fx.world, sim, 4_000).unwrap();
        if fx.world.objects[&sim].stack.len() > 1 && matches!(status, Status::Yielded) {
            cot_cancel_yields += 1;
            if cot_cancel_yields == 1 {
                assert_eq!(fx.world.objects[&cot].attributes[0], 1);
                assert!(vm.cancel(&mut fx.world, sim, cot_uid).unwrap());
                cot_cancelled = true;
            }
        } else if cot_cancelled && fx.world.objects[&sim].stack.len() == 1 {
            break;
        }
    }
    assert!(cot_cancelled);
    assert_eq!(cot_cancel_yields, 2);
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_ENERGY],
        -40.0
    );
    assert_eq!(fx.world.objects[&cot].attributes[0], 0);
    if let Some(person) = fx.world.objects.get_mut(&sim).unwrap().person.as_mut() {
        person.active.uid = 0;
        person.active.flags = 0;
        person.data[33] = 0;
        person.object_uses.clear();
    }

    // Basin: Use Basin sets bladder = 100 and hygiene += 25!
    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, basin, 0, 32, |_, _| {});
    let p = fx.world.objects[&sim].person.as_ref().unwrap();
    assert_eq!(p.motives[MOTIVE_BLADDER], 100.0);
    assert_eq!(p.motives[MOTIVE_HYGIENE], -5.0);
    // Now bladder == 100, so pie_menu guard rejects Use Basin!
    assert!(PackVmFixture::pie_menu_names(&mut vm, &mut fx.world, sim, basin).is_empty());

    assert_eq!(
        played_sounds(&sounds),
        vec![
            "open_bench_creak",
            "open_cot_settle",
            "open_cot_settle",
            "open_basin_flush",
        ]
    );
}

#[test]
fn vm_pattern_a_multi_tile_double_bed_pie_menu_occupancy_and_native_vm_cancel() {
    let pack = compile_pack_dir(&open_pack_dir()).unwrap();
    let mut fx = PackVmFixture::from_pack(&pack, 500);
    let sounds = RefCell::new(Vec::new());
    let mut vm = Interpreter::new(&fx.program, &fx.catalog, Stream::Lot);
    vm.sounds = Some(&sounds);

    // Instantiate the multi-tile group via sims-vm's real group factory!
    let lead = vm
        .create(&mut fx.world, BED_MASTER_GUID)
        .expect("multi-tile double bed factory creation failed");
    assert_eq!(fx.world.objects[&lead].guid, BED_LEAD_GUID);

    let slave = *fx
        .world
        .next_parts
        .get(&lead)
        .expect("lead tile must link to slave tile via next_parts");
    assert_eq!(fx.world.objects[&slave].guid, BED_SLAVE_GUID);
    assert_eq!(
        fx.world.objects[&slave].multipart.as_ref().unwrap().parent,
        lead as i16
    );
    assert_eq!(
        fx.world.objects[&slave].multipart.as_ref().unwrap().group,
        [1, 0, 0]
    );

    let sim = fx.sim_id;

    // Clicking the SLAVE tile redirects pie_menu to the LEAD tile and offers both sides!
    let slave_menu = vm.pie_menu(&mut fx.world, sim, slave).unwrap();
    assert_eq!(slave_menu.len(), 2);
    assert_eq!(slave_menu[0].action.target as u16, lead);
    assert_eq!(slave_menu[1].action.target as u16, lead);

    // 1. Full sleep on Left Side (table_index 0) picked from the slave tile:
    // verify shared occupancy synced to BOTH lead and slave mid-sleep, and cleared on wake!
    let mut saw_synced_occupancy = false;
    let yields = PackVmFixture::run_pie_interaction(
        &mut vm,
        &mut fx.world,
        sim,
        slave,
        0,
        64,
        |world, y| {
            if y == 1 {
                assert_eq!(world.objects[&lead].attributes[0], 1);
                assert_eq!(world.objects[&slave].attributes[0], 1);
                saw_synced_occupancy = true;
            }
        },
    );
    assert!(saw_synced_occupancy);
    assert_eq!(yields, 10, "5 cycles * idle(2) = 10 yields");
    let p = fx.world.objects[&sim].person.as_ref().unwrap();
    assert_eq!(p.motives[MOTIVE_ENERGY], 80.0);
    assert_eq!(p.motives[MOTIVE_COMFORT], 30.0);
    assert_eq!(fx.world.objects[&lead].attributes[0], 0);
    assert_eq!(fx.world.objects[&slave].attributes[0], 0);
    assert_eq!(fx.world.objects[&lead].attributes[2], 5);

    // 2. Native player cancellation via `vm.cancel(&mut world, sim, active_uid)` on Right Side!
    fx.world
        .objects
        .get_mut(&sim)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .motives[MOTIVE_ENERGY] = 0.0;

    let entries = vm.pie_menu(&mut fx.world, sim, lead).unwrap();
    let right_side = entries
        .into_iter()
        .find(|e| e.action.table_index == 1)
        .expect("Sleep (Right Side) must be offered");
    let uid = vm
        .pick(&mut fx.world, sim, right_side.action)
        .unwrap()
        .unwrap();

    let mut cancelled = false;
    let mut yields2 = 0usize;
    for _ in 0..32 {
        fx.world.objects.get_mut(&sim).unwrap().data[25] = 1;
        let status = vm.turn(&mut fx.world, sim, 4_000).unwrap();
        if fx.world.objects[&sim].stack.len() > 1 && matches!(status, Status::Yielded) {
            yields2 += 1;
            if yields2 == 1 {
                assert_eq!(fx.world.objects[&lead].attributes[0], 2);
                assert_eq!(fx.world.objects[&slave].attributes[0], 2);
                assert!(
                    vm.cancel(&mut fx.world, sim, uid).unwrap(),
                    "vm.cancel must accept the active sleep interaction uid"
                );
                cancelled = true;
            }
        } else if cancelled && fx.world.objects[&sim].stack.len() == 1 {
            break;
        }
    }
    assert!(cancelled);
    assert_eq!(
        yields2, 2,
        "finishes active idle(2) then breaks on priority == 0"
    );
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_ENERGY],
        0.0
    );
    assert_eq!(fx.world.objects[&lead].attributes[0], 0);
    assert_eq!(fx.world.objects[&slave].attributes[0], 0);

    assert_eq!(
        played_sounds(&sounds),
        vec![
            "open_double_bed_settle",
            "open_double_bed_wake",
            "open_double_bed_settle",
            "open_double_bed_wake",
        ]
    );
}

#[test]
fn vm_pattern_b_multi_object_cooking_chain_on_lot_and_skill_scaling() {
    let pack = compile_pack_dir(&open_pack_dir()).unwrap();
    let mut fx = PackVmFixture::from_pack(&pack, 100);
    let sounds = RefCell::new(Vec::new());
    let mut vm = Interpreter::new(&fx.program, &fx.catalog, Stream::Lot);
    vm.sounds = Some(&sounds);

    let fridge = vm.create(&mut fx.world, FRIDGE_GUID).unwrap();
    let counter = vm.create(&mut fx.world, COUNTER_GUID).unwrap();
    let stove = vm.create(&mut fx.world, STOVE_GUID).unwrap();
    // Place appliances on real on-lot tiles inside room 1 (stove at (6, 6) facing North=0
    // so the tile ahead (6, 5) is free for on-lot Open Meal Plate creation!).
    PackVmFixture::place_on_tile(&mut fx.world, fridge, 4, 6, 0);
    PackVmFixture::place_on_tile(&mut fx.world, counter, 5, 6, 0);
    PackVmFixture::place_on_tile(&mut fx.world, stove, 6, 6, 0);
    let sim = fx.sim_id;

    // Verify pie_menu guard rejects when funds < $15, accepts at $100.
    fx.world.budget.funds = 10;
    assert!(PackVmFixture::pie_menu_names(&mut vm, &mut fx.world, sim, fridge).is_empty());
    fx.world.budget.funds = 100;
    assert_eq!(
        PackVmFixture::pie_menu_names(&mut vm, &mut fx.world, sim, fridge).len(),
        1
    );

    // Meal 1: Novice cook (Cooking skill = 0) -> base 30 + prep 10 + 0/20 = 40 nutrition!
    fx.world
        .objects
        .get_mut(&sim)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[SKILL_COOKING] = 0;
    let yields1 =
        PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, fridge, 0, 32, |_, _| {});
    assert_eq!(yields1, 2, "Stove CookOnStove yields idle(2)");
    assert_eq!(fx.world.budget.funds, 85); // 100 - 15
    assert_eq!(fx.world.objects[&fridge].attributes[0], 4);
    assert_eq!(fx.world.objects[&fridge].attributes[1], 1);
    assert_eq!(fx.world.objects[&counter].attributes[0], 1);
    assert_eq!(fx.world.objects[&stove].attributes[0], 1);
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().data[SKILL_COOKING],
        20
    );

    // Verify Open Meal Plate was placed ON THE LOT directly ahead of the stove at tile (6, 5)!
    let plate1 = fx.world.objects[&fridge].attributes[2] as u16;
    assert!(plate1 > 0);
    assert_eq!(fx.world.objects[&plate1].guid, PLATE_GUID);
    assert_eq!(
        sims_sim::World::tile(fx.world.objects[&plate1].position),
        [6, 5, 1],
        "Open Meal Plate must be placed on-lot at the tile ahead of the stove"
    );
    let novice_nutrition = fx.world.objects[&plate1].attributes[0];
    assert_eq!(novice_nutrition, 40);

    // Sim eats plate1 via real pie_menu + pick: hunger -50 -> -10, plate disposed & removed!
    let plate_menu = PackVmFixture::pie_menu_names(&mut vm, &mut fx.world, sim, plate1);
    assert_eq!(plate_menu.len(), 1);
    assert_eq!(plate_menu[0].0, 0); // "Eat Meal" offered, "Clean Up" blocked while full
    let eat_yields =
        PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, plate1, 0, 32, |_, _| {});
    assert_eq!(eat_yields, 2);
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_HUNGER],
        -10.0
    );
    assert_eq!(fx.world.objects[&plate1].attributes[1], 0);
    assert!(
        fx.world.removals.contains(&plate1),
        "EatMeal must dispose/remove the plate via remove_object(stack_obj)"
    );
    // Clear the removed plate's grid cell as end-of-tick removal does so tile (6, 5) is free again.
    let grid = &mut fx.world.arrays.get_mut(&3).unwrap().data;
    let idx = (6 * 64 + 5) * 2;
    assert_eq!(
        &grid[idx..idx + 2],
        &[0, 0],
        "take_off_to_remove cleared tile (6, 5)"
    );

    // Meal 2: Skilled chef (Cooking skill = 600) -> base 30 + prep 10 + 600/20 = 70 nutrition!
    fx.world
        .objects
        .get_mut(&sim)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[SKILL_COOKING] = 600;
    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, fridge, 0, 32, |_, _| {});
    let plate2 = fx.world.objects[&fridge].attributes[2] as u16;
    assert_ne!(plate2, plate1);
    assert_eq!(
        sims_sim::World::tile(fx.world.objects[&plate2].position),
        [6, 5, 1]
    );
    let chef_nutrition = fx.world.objects[&plate2].attributes[0];
    assert_eq!(chef_nutrition, 70);
    assert!(chef_nutrition > novice_nutrition);

    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, plate2, 0, 32, |_, _| {});
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_HUNGER],
        60.0
    );
    assert!(fx.world.removals.contains(&plate2));

    // Verify atomic refund if stove cannot place a plate (block tile (6, 5) with a bench!).
    let blocker = vm.create(&mut fx.world, BENCH_GUID).unwrap();
    PackVmFixture::place_on_tile(&mut fx.world, blocker, 6, 5, 0);
    let funds_before_fail = fx.world.budget.funds;
    let stock_before_fail = fx.world.objects[&fridge].attributes[0];
    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, fridge, 0, 32, |_, _| {});
    assert_eq!(
        fx.world.budget.funds, funds_before_fail,
        "failed on-lot plate placement must refund meal cost"
    );
    assert_eq!(
        fx.world.objects[&fridge].attributes[0], stock_before_fail,
        "failed on-lot plate placement must restore fridge stock"
    );

    let played = played_sounds(&sounds);
    assert!(played.contains(&"open_fridge_door".to_owned()));
    assert!(played.contains(&"open_counter_chop".to_owned()));
    assert!(played.contains(&"open_stove_sizzle".to_owned()));
    assert!(played.contains(&"open_plate_eat".to_owned()));
    assert!(played.contains(&"open_plate_dispose".to_owned()));
}

#[test]
fn vm_pattern_c_wall_phone_dialog_branching_and_fwav_events() {
    let pack = compile_pack_dir(&open_pack_dir()).unwrap();
    let mut fx = PackVmFixture::from_pack(&pack, 200);
    let sounds = RefCell::new(Vec::new());
    let mut vm = Interpreter::new(&fx.program, &fx.catalog, Stream::Lot);
    vm.sounds = Some(&sounds);

    let phone = vm.create(&mut fx.world, PHONE_GUID).unwrap();
    let fridge = vm.create(&mut fx.world, FRIDGE_GUID).unwrap();
    let sim = fx.sim_id;

    let phone_def = fx.catalog.object(PHONE_GUID).unwrap();
    let str_301 = &fx.program.libraries[&phone_def.resource].strings[&301];
    assert_eq!(str_301[0], "Open Wall Phone");
    assert!(str_301.len() >= 8);

    // 1. QuickOrderGroceries (table_index 1, 2-button Yes/Cancel) -> Accept (FIRST)
    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, phone, 1, 16, |world, y| {
        if y == 1 {
            let rec = world
                .dialogs
                .records
                .iter_mut()
                .rev()
                .find(|r| r.caller == sim)
                .expect("expected pending phone dialog owned by Sim");
            rec.status = DIALOG_FIRST;
        }
    });
    assert_eq!(fx.world.budget.funds, 160); // 200 - 40
    assert_eq!(fx.world.objects[&fridge].attributes[0], 9); // 5 + 4
    assert_eq!(fx.world.objects[&phone].attributes[1], 1);
    assert_eq!(fx.world.objects[&phone].attributes[0], 0);

    // 2. QuickOrderGroceries (table_index 1, 2-button Yes/Cancel) -> Cancel (SECOND)
    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, phone, 1, 16, |world, y| {
        if y == 1 {
            let rec = world
                .dialogs
                .records
                .iter_mut()
                .rev()
                .find(|r| r.caller == sim)
                .expect("expected pending phone dialog");
            rec.status = DIALOG_SECOND;
        }
    });
    assert_eq!(fx.world.budget.funds, 160);
    assert_eq!(fx.world.objects[&fridge].attributes[0], 9);
    assert_eq!(fx.world.objects[&phone].attributes[1], 4);
    assert_eq!(fx.world.objects[&phone].attributes[0], 0);

    // 3. UsePhone (table_index 0, 3-button menu) -> Button 1: Order Groceries (FIRST)
    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, phone, 0, 16, |world, y| {
        if y == 1 {
            let rec = world
                .dialogs
                .records
                .iter_mut()
                .rev()
                .find(|r| r.caller == sim)
                .expect("expected pending 3-button menu dialog");
            rec.status = DIALOG_FIRST;
        }
    });
    assert_eq!(fx.world.budget.funds, 120);
    assert_eq!(fx.world.objects[&fridge].attributes[0], 13);
    assert_eq!(fx.world.objects[&phone].attributes[1], 1);

    // 4. UsePhone (table_index 0, 3-button menu) -> Button 2: Chat with Neighbor (SECOND)
    let social_before = fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_SOCIAL];
    let fun_before = fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_FUN];
    let chat_yields = PackVmFixture::run_pie_interaction(
        &mut vm,
        &mut fx.world,
        sim,
        phone,
        0,
        24,
        |world, y| {
            if y == 1 {
                let rec = world
                    .dialogs
                    .records
                    .iter_mut()
                    .rev()
                    .find(|r| r.caller == sim)
                    .expect("expected pending 3-button menu dialog");
                rec.status = DIALOG_SECOND;
            }
        },
    );
    assert_eq!(chat_yields, 3, "1 dialog yield + idle(2) chat");
    assert_eq!(fx.world.objects[&phone].attributes[1], 2);
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_SOCIAL],
        social_before + 35.0
    );
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_FUN],
        fun_before + 15.0
    );

    // 5. UsePhone (table_index 0, 3-button menu) -> Button 3: Call for Info (THIRD)
    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, phone, 0, 16, |world, y| {
        if y == 1 {
            let rec = world
                .dialogs
                .records
                .iter_mut()
                .rev()
                .find(|r| r.caller == sim)
                .expect("expected pending 3-button menu dialog");
            rec.status = DIALOG_THIRD;
        }
    });
    assert_eq!(fx.world.objects[&phone].attributes[1], 3);
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_FUN],
        fun_before + 25.0
    );

    let played = played_sounds(&sounds);
    assert!(played.contains(&"open_phone_ring".to_owned()));
    assert!(played.contains(&"open_phone_dial".to_owned()));
    assert!(played.contains(&"open_phone_confirm".to_owned()));
}

#[test]
fn vm_pattern_d_repairable_privacy_shower_guards_wear_repair_and_native_cancel() {
    let pack = compile_pack_dir(&open_pack_dir()).unwrap();
    let mut fx = PackVmFixture::from_pack(&pack, 500);
    let sounds = RefCell::new(Vec::new());
    let mut vm = Interpreter::new(&fx.program, &fx.catalog, Stream::Lot);
    vm.sounds = Some(&sounds);

    let shower = vm.create(&mut fx.world, SHOWER_GUID).unwrap();
    let sim = fx.sim_id;

    // Initially working: pie_menu offers ONLY Take Shower (0), NOT Repair Shower (1)!
    let init_menu = PackVmFixture::pie_menu_names(&mut vm, &mut fx.world, sim, shower);
    assert_eq!(init_menu.len(), 1);
    assert_eq!(init_menu[0].0, 0);

    // Run 3 showers: verify privacy lock mid-shower, hygiene refill, and break at wear == 3!
    let mut saw_privacy_lock = false;
    for use_idx in 1..=3 {
        fx.world
            .objects
            .get_mut(&sim)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .motives[MOTIVE_HYGIENE] = 0.0;
        PackVmFixture::run_pie_interaction(
            &mut vm,
            &mut fx.world,
            sim,
            shower,
            0,
            32,
            |world, y| {
                if y == 1 {
                    assert_eq!(world.objects[&shower].attributes[0], 1);
                    saw_privacy_lock = true;
                }
            },
        );
        assert_eq!(fx.world.objects[&shower].attributes[0], 0);
        assert_eq!(fx.world.objects[&shower].attributes[1], use_idx);
        assert_eq!(
            fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_HYGIENE],
            100.0
        );
    }
    assert!(saw_privacy_lock);

    // After 3 uses, shower is BROKEN!
    assert_eq!(fx.world.objects[&shower].attributes[2], 1);
    fx.world
        .objects
        .get_mut(&sim)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .motives[MOTIVE_HYGIENE] = -20.0;

    // Broken state pie_menu: Take Shower (0) is blocked, ONLY Repair Shower (1) is offered!
    let broken_menu = PackVmFixture::pie_menu_names(&mut vm, &mut fx.world, sim, shower);
    assert_eq!(broken_menu.len(), 1);
    assert_eq!(broken_menu[0].0, 1);

    // Native mid-repair cancel via `vm.cancel(&mut world, sim, uid)`:
    let entries = vm.pie_menu(&mut fx.world, sim, shower).unwrap();
    let repair_uid = vm
        .pick(&mut fx.world, sim, entries[0].action.clone())
        .unwrap()
        .unwrap();
    let mut cancelled = false;
    let mut r_yields = 0usize;
    for _ in 0..32 {
        fx.world.objects.get_mut(&sim).unwrap().data[25] = 1;
        let status = vm.turn(&mut fx.world, sim, 4_000).unwrap();
        if fx.world.objects[&sim].stack.len() > 1 && matches!(status, Status::Yielded) {
            r_yields += 1;
            if r_yields == 1 {
                assert!(vm.cancel(&mut fx.world, sim, repair_uid).unwrap());
                cancelled = true;
            }
        } else if cancelled && fx.world.objects[&sim].stack.len() == 1 {
            if let Some(person) = fx.world.objects.get_mut(&sim).unwrap().person.as_mut() {
                person.active.uid = 0;
                person.active.flags = 0;
                person.data[33] = 0;
                person.object_uses.clear();
            }
            break;
        }
    }
    assert!(cancelled);
    assert_eq!(fx.world.objects[&shower].attributes[2], 1);
    assert_eq!(fx.world.objects[&shower].attributes[0], 0);

    // Full repair with low Mechanical skill (0 -> step 20 -> 5 cycles).
    fx.world
        .objects
        .get_mut(&sim)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[SKILL_MECHANICAL] = 0;
    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, shower, 1, 32, |_, _| {});
    let low_skill_cycles = fx.world.objects[&shower].attributes[4];
    assert_eq!(low_skill_cycles, 5);
    assert_eq!(fx.world.objects[&shower].attributes[2], 0);
    assert_eq!(fx.world.objects[&shower].attributes[1], 0);
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().data[SKILL_MECHANICAL],
        15
    );

    // Guards flip back: pie_menu offers ONLY Take Shower (0)!
    let fixed_menu = PackVmFixture::pie_menu_names(&mut vm, &mut fx.world, sim, shower);
    assert_eq!(fixed_menu.len(), 1);
    assert_eq!(fixed_menu[0].0, 0);

    // Compare repair speed with high Mechanical skill (600 -> step 50 -> 2 cycles).
    fx.world.objects.get_mut(&shower).unwrap().attributes[2] = 1;
    fx.world
        .objects
        .get_mut(&sim)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[SKILL_MECHANICAL] = 600;
    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, shower, 1, 32, |_, _| {});
    let high_skill_cycles = fx.world.objects[&shower].attributes[4];
    assert_eq!(high_skill_cycles, 2);
    assert!(high_skill_cycles < low_skill_cycles);

    // Shower succeeds again via pie_menu!
    PackVmFixture::run_pie_interaction(&mut vm, &mut fx.world, sim, shower, 0, 32, |_, _| {});
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_HYGIENE],
        100.0
    );

    let played = played_sounds(&sounds);
    assert!(played.contains(&"open_shower_spray".to_owned()));
    assert!(played.contains(&"open_shower_wrench".to_owned()));
    assert!(played.contains(&"open_shower_fixed".to_owned()));
}

#[test]
fn dsl_nested_continue_break_indirect_param0_and_validation_regressions() {
    // 1. Positive VM regression for nested while + inner continue + inner break + trailing statements,
    //    AND scope(22, 0) indirect param[0] preservation across idle(), AND trailing uninitialized `local`
    //    inside a labeled tree (Sol Round 1 P2 #7, P2 #8, P3 #2).
    let script = r#"
tree 4096 "NestedLoops" {
    let outer = 0;
    let acc = 0;
    while outer < 3 {
        let inner = 0;
        while inner < 5 {
            inner += 1;
            if inner == 2 {
                continue;
            }
            if inner == 4 {
                break;
            }
            acc += inner;
        }
        outer += 1;
    }
    my.attr[0] := acc;
    my.attr[1] := outer;
    return true;
}

tree 4097 "IndirectParam0AcrossIdle" (args: 1) {
    idle(2);
    my.attr[2] := scope(22, 0);
    return true;
}

tree 4098 "LabeledTrailingUninitializedLocal" {
    @start:
    play_sound(1);
    local scratch;
}
"#;
    let compiled = compile_antics(script).expect("regression script failed to compile");
    assert_eq!(compiled.behaviors[&4098].instructions.len(), 1);
    assert_eq!(compiled.behaviors[&4098].instructions[0].on_true, 0xFE);
    // Indirect scope(22, 0) + idle(2) must emit save/set/sleep/restore (4 instructions) + assign = 5.
    assert_eq!(compiled.behaviors[&4097].instructions.len(), 5);
    assert!(compiled.behaviors[&4097].locals >= 1);

    // Execute NestedLoops live in sims-vm: each of 3 outer iterations sums inner = 1 + 3 = 4 -> acc = 12, outer = 3!
    const TEST_GUID: u32 = 0x7F00_00EE;
    let mut lib = Library::default();
    for (&id, bhav) in &compiled.behaviors {
        let raw = sims_antics::encode::encode_bhav(bhav).unwrap();
        lib.behaviors.insert(id, Behavior::parse(&raw).unwrap());
    }
    let mut words = [0i16; DEFINITION_WORDS];
    words[9] = 4;
    words[14] = TEST_GUID as u16 as i16;
    words[15] = (TEST_GUID >> 16) as u16 as i16;
    lib.definitions.insert(128, words);
    lib.definition_order.push(128);

    let program = Program {
        global: 0,
        libraries: BTreeMap::from([(0, Library::default()), (1, lib)]),
        semi_globals: BTreeMap::from([(1, None)]),
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
    let catalog = Catalog {
        global: 0,
        objects: vec![sims_content::Object {
            guid: TEST_GUID,
            resource: 1,
            chunk: 128,
            kind: 4,
            master: 0,
        }],
        by_guid: BTreeMap::from([(TEST_GUID, 0)]),
        semi_globals: BTreeMap::new(),
        diagnostics: Vec::new(),
    };
    let mut world = World::empty();
    world.types.insert(1, TEST_GUID);
    world.order.push(1);
    let mut obj = make_sim_object(1);
    obj.guid = TEST_GUID;
    obj.person = None;
    obj.attributes = vec![0, 0, 0, 77, 0, 0, 0, 0];
    world.objects.insert(1, obj);

    let vm = Interpreter::new(&program, &catalog, Stream::Lot);
    assert!(vm.run_tree(&mut world, 1, 4096, 1, &[], 500).unwrap());
    assert_eq!(world.objects[&1].attributes[0], 12);
    assert_eq!(world.objects[&1].attributes[1], 3);

    // Step IndirectParam0AcrossIdle with param[0] = 3 -> must preserve 3 across idle(2) and read stack.attr[3] == 77!
    world.frame_storage.remove(&1);
    world.objects.get_mut(&1).unwrap().stack = vec![Frame {
        stack_object: 1,
        tree: 4097,
        node: 0,
        parameters: vec![3],
        locals: vec![0],
        primitive_state: 0,
        code_owner_type: 1,
    }];
    world.objects.get_mut(&1).unwrap().data[25] = 1;
    let mut returned = None;
    for _ in 0..8 {
        if let Status::Returned(v) = vm.turn(&mut world, 1, 500).unwrap() {
            returned = Some(v);
            break;
        }
    }
    assert_eq!(returned, Some(true));
    assert_eq!(world.objects[&1].attributes[2], 77);

    // 2. Negative validation regressions (Sol Round 1 P1 #2, P2 #3-#6).
    assert!(compile_antics(r#"tree 4096 "BadBreak" { break; }"#).is_err());
    assert!(compile_antics(r#"tree 4096 "BadCont" { continue; }"#).is_err());
    assert!(
        compile_antics(r#"tree 4096 "DupLet" { let x = 1; let x = 2; return true; }"#).is_err()
    );
    assert!(compile_antics(r#"tree 4096 "Undeclared" { x := 1; return true; }"#).is_err());
    assert!(
        compile_antics(
            r#"tree 4096 "MissingStr" { dialog(kind: ok, message: str.nope); return true; }"#
        )
        .is_err()
    );
    // Reject Windows drive-prefixed filenames (`C:evil.antics`), duplicate callbacks, reserved callback 0/1,
    // unemitted tile dgrp_id/ttab_id, and non-private BCON IDs in private objects.
    assert!(
        sims_antics::manifest::ObjectManifest::parse(
            r#"
[object]
name = "BadDrive"
guid = 0x7F00_0091
scripts = ["C:evil.antics"]
"#
        )
        .is_err()
    );
    assert!(
        sims_antics::manifest::ObjectManifest::parse(
            r#"
[object]
name = "BadCb"
guid = 0x7F00_0092
[[callback]]
index = 0
action = "Init"
"#
        )
        .is_err()
    );
    assert!(
        compile_object_from_sources(
            r#"
[object]
name = "BadGlobalBcon"
guid = 0x7F00_0093
"#,
            &[(
                "bad.antics",
                r#"
bcon 256 "GlobalLeak" { v = 1 }
tree 4096 "Init" { my.attr[0] := bcon.v; return true; }
"#
            )]
        )
        .is_err()
    );
    // Unreachable private BCON IDs outside 4096..=4159 (e.g. 5000) are rejected in both scripts and manifest.
    assert!(
        compile_object_from_sources(
            r#"
[object]
name = "BadHighBcon"
guid = 0x7F00_0096
[[constant]]
id = 5000
label = "OutOfRange"
values = [1]
"#,
            &[]
        )
        .is_err()
    );
    assert!(
        compile_object_from_sources(
            r#"
[object]
name = "BadHighScriptBcon"
guid = 0x7F00_0097
"#,
            &[(
                "bad_high.antics",
                r#"
bcon 5000 "OutOfRange" { v = 1 }
tree 4096 "Init" { return true; }
"#
            )]
        )
        .is_err()
    );
    // Order-independent tile DGRP sharing: tile 0 may reference dgrp_id 102 defined by tile 1.
    let forward_tile = compile_object_from_sources(
        r#"
[object]
name = "ForwardSharedTile"
guid = 0x7F00_0098
master_id = 2
[[tile]]
guid = 0x7F00_009A
lead = true
sub_index = [0, 0]
dgrp_id = 102
[[tile]]
guid = 0x7F00_009B
sub_index = [1, 0]
dgrp_id = 102
shape = "double_bed_right"
"#,
        &[],
    )
    .expect("forward-ordered tile DGRP reference should succeed");
    let forward_iff = Iff::parse(forward_tile.iff_bytes).unwrap();
    assert!(forward_iff.get(b"DGRP", 102).is_some());
    assert!(
        compile_object_from_sources(
            r#"
[object]
name = "BadTileRef"
guid = 0x7F00_0094
master_id = 1
[[tile]]
guid = 0x7F00_0095
lead = true
sub_index = [0, 0]
dgrp_id = 999
"#,
            &[]
        )
        .is_err()
    );
    assert!(
        sims_antics::manifest::ObjectManifest::parse(
            r#"
[object]
name = "Bad"
guid = 0x7F00_0099
[[interaction]]
name = "Conflict"
action = "A"
test = "T"
guard = "G"
"#
        )
        .is_err()
    );
}

#[test]
fn cli_check_validates_on_disk_open_pack() {
    let bin = env!("CARGO_BIN_EXE_sims-antics");
    let output = std::process::Command::new(bin)
        .arg("check")
        .arg(open_pack_dir())
        .output()
        .expect("failed to execute sims-antics CLI");
    assert!(
        output.status.success(),
        "sims-antics check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Open Starter Pack"));
    assert!(stdout.contains("20 objects"));
}

#[test]
fn pack_compiler_rejects_normalized_filename_collisions_and_symlink_escapes() {
    let unique = format!("sims-antics-sec-test-{}", std::process::id());
    let root = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&root);

    // 1. Normalized filename collision: `foo-bar` vs `foo_bar` both map to `foo_bar.iff`.
    let collide_pack = root.join("collide-pack");
    std::fs::create_dir_all(collide_pack.join("objects/foo-bar")).unwrap();
    std::fs::create_dir_all(collide_pack.join("objects/foo_bar")).unwrap();
    std::fs::write(
        collide_pack.join("pack.toml"),
        "[pack]\nname = \"Collide\"\narchive = \"collide.far\"\nobjects = [\"foo-bar\", \"foo_bar\"]\n",
    )
    .unwrap();
    std::fs::write(
        collide_pack.join("objects/foo-bar/object.toml"),
        "[object]\nname = \"A\"\nguid = 0x7F00_1001\n",
    )
    .unwrap();
    std::fs::write(
        collide_pack.join("objects/foo-bar/behavior.antics"),
        "tree 4096 \"Init\" { return true; }\n",
    )
    .unwrap();
    std::fs::write(
        collide_pack.join("objects/foo_bar/object.toml"),
        "[object]\nname = \"B\"\nguid = 0x7F00_1002\n",
    )
    .unwrap();
    std::fs::write(
        collide_pack.join("objects/foo_bar/behavior.antics"),
        "tree 4096 \"Init\" { return true; }\n",
    )
    .unwrap();
    let err =
        compile_pack_dir(&collide_pack).expect_err("expected normalized filename collision error");
    assert!(err.to_string().contains("foo_bar.iff"));

    // 2. Unix symlink escape in pack input and CLI build output.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = root.join("outside.antics");
        std::fs::write(&outside, "tree 4096 \"Init\" { return true; }\n").unwrap();

        let sym_pack = root.join("sym-pack");
        std::fs::create_dir_all(sym_pack.join("objects/esc")).unwrap();
        std::fs::write(
            sym_pack.join("pack.toml"),
            "[pack]\nname = \"Sym\"\narchive = \"sym.far\"\nobjects = [\"esc\"]\n",
        )
        .unwrap();
        std::fs::write(
            sym_pack.join("objects/esc/object.toml"),
            "[object]\nname = \"Esc\"\nguid = 0x7F00_1003\nscripts = [\"behavior.antics\"]\n",
        )
        .unwrap();
        symlink(&outside, sym_pack.join("objects/esc/behavior.antics")).unwrap();
        assert!(
            compile_pack_dir(&sym_pack).is_err(),
            "expected symlinked script outside pack root to be rejected"
        );

        let outside_dir = root.join("outside-obj-dir");
        std::fs::create_dir_all(&outside_dir).unwrap();
        std::fs::write(
            outside_dir.join("object.toml"),
            "[object]\nname = \"EscDir\"\nguid = 0x7F00_1004\n",
        )
        .unwrap();
        let sym_dir_pack = root.join("sym-dir-pack");
        std::fs::create_dir_all(sym_dir_pack.join("objects")).unwrap();
        std::fs::write(
            sym_dir_pack.join("pack.toml"),
            "[pack]\nname = \"SymDir\"\narchive = \"sym_dir.far\"\nobjects = [\"esc\"]\n",
        )
        .unwrap();
        symlink(&outside_dir, sym_dir_pack.join("objects/esc")).unwrap();
        assert!(
            compile_pack_dir(&sym_dir_pack).is_err(),
            "expected symlinked object directory outside pack root to be rejected"
        );

        let out_dir = root.join("build-out");
        let outside_far = root.join("outside.far");
        std::fs::create_dir_all(&out_dir).unwrap();
        std::fs::write(&outside_far, b"sentinel").unwrap();
        symlink(&outside_far, out_dir.join("open-pack.far")).unwrap();
        let bin = env!("CARGO_BIN_EXE_sims-antics");
        let output = std::process::Command::new(bin)
            .args(["build"])
            .arg(open_pack_dir())
            .args(["--out"])
            .arg(&out_dir)
            .output()
            .expect("failed to execute sims-antics CLI");
        assert!(
            !output.status.success(),
            "CLI build should reject pre-existing output symlink"
        );
        assert_eq!(std::fs::read(&outside_far).unwrap(), b"sentinel");

        // Symlinked ancestor directory under `--out` must also be rejected!
        let outside_target_dir = root.join("outside-target-dir");
        std::fs::create_dir_all(&outside_target_dir).unwrap();
        let sym_ancestor = root.join("sym-ancestor");
        symlink(&outside_target_dir, &sym_ancestor).unwrap();
        let nested_out = sym_ancestor.join("nested-out");
        let output_ancestor = std::process::Command::new(bin)
            .args(["build"])
            .arg(open_pack_dir())
            .args(["--out"])
            .arg(&nested_out)
            .output()
            .expect("failed to execute sims-antics CLI");
        assert!(
            !output_ancestor.status.success(),
            "CLI build should reject symlinked ancestor directory under --out"
        );
        assert!(!outside_target_dir.join("nested-out").exists());

        // Normal clean `--out` directory succeeds and writes open-pack.far, objects/, sounds/.
        let clean_out = root.join("clean-out/sub");
        let output_ok = std::process::Command::new(bin)
            .args(["build"])
            .arg(open_pack_dir())
            .args(["--out"])
            .arg(&clean_out)
            .output()
            .expect("failed to execute sims-antics CLI");
        assert!(
            output_ok.status.success(),
            "clean CLI build failed: {}",
            String::from_utf8_lossy(&output_ok.stderr)
        );
        assert!(clean_out.join("open-pack.far").is_file());
        assert!(clean_out.join("objects/open_double_bed.iff").is_file());
    }

    let _ = std::fs::remove_dir_all(&root);
}
