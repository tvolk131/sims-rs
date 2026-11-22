//! End-to-end on-disk `open-pack/` compilation and `sims-vm` behavioral stress suite.
//!
//! Compiles the entire `open-pack/` source tree from disk into `open-pack.far`,
//! verifies every member `.iff` and procedural asset via `sims_formats`, and
//! exercises all 4 complex life-sim patterns live inside `sims-vm` with zero traps:
//!   A. Multi-Tile Double Bed (`open-double-bed`: master `0x7F00_0012`, lead `0x7F00_0010`, slave `0x7F00_0011`)
//!   B. Multi-Object Cooking Chain (`open-fridge` -> `open-counter` -> `open-stove` -> `open-meal-plate`)
//!   C. Interactive Wall Phone (`open-wall-phone`: 2-way & 3-way `dialog` branching + FWAV SFX)
//!   D. Repairable Privacy Shower (`open-shower`: guard trees, wear->broken, Mechanic-scaled repair, cancel reset)

use sims_antics::{CompiledPack, compile_antics, compile_pack_dir};
use sims_content::Catalog;
use sims_core::GameRng;
use sims_formats::behavior::{Behavior, InteractionTable, ObjectDefinition};
use sims_formats::execution::{ObjectFunction, ObjectFunctions, Slots, constants};
use sims_formats::far;
use sims_formats::iff::Iff;
use sims_formats::saved_objects::{
    Frame, Interaction as SavedInteraction, Person, Slot as SavedSlot,
};
use sims_formats::sprite::{DrawGroup, Palette, SpriteSet, render};
use sims_formats::strings::StringTable;
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

    Object {
        id,
        guid: SIM_GUID,
        footprint: [0; 4],
        position: [32, 32, 1],
        unknown_short: 0,
        attributes: vec![0; 8],
        temps: vec![0; 8],
        data: vec![0; 72],
        obsolete_word: 0,
        stack_capacity_bytes: 512,
        stack: Vec::new(),
        relationship_flag: -1,
        relationships: Vec::new(),
        slots: vec![SavedSlot {
            unknown: 0,
            object: 0,
        }],
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
            data: vec![0; 256],
            routing_frames: 0,
            active: idle.clone(),
            last: idle,
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        }),
        registry_word: None,
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

        // Register synthetic test Sim person definition.
        let sim_res = dir.len() + 1;
        let mut sim_lib = Library::default();
        let mut sim_words = [0i16; DEFINITION_WORDS];
        sim_words[9] = 2; // person
        sim_words[14] = SIM_GUID as u16 as i16;
        sim_words[15] = (SIM_GUID >> 16) as u16 as i16;
        sim_lib.definitions.insert(128, sim_words);
        sim_lib.definition_order.push(128);
        sim_lib.callbacks.insert(
            128,
            vec![ObjectFunction {
                condition: 0,
                action: 0,
            }],
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
        world.globals = vec![0; 64];
        world.globals[1] = 1; // day
        world.globals[7] = 1; // month
        world.globals[8] = 2000; // year
        world.globals[14] = (starting_funds / 10000) as i16;
        world.globals[13] = (starting_funds % 10000) as i16;
        world.budget.funds = starting_funds;
        world.budget.days = vec![Some([0; 8])];
        world.lot_rng = GameRng::from_state(11);
        world.global_rng = GameRng::from_state(22);
        world.order.push(sim_id);
        world.people.push(sim_id);
        world.types.insert(1, SIM_GUID);
        world.objects.insert(sim_id, make_sim_object(sim_id));

        Self {
            program,
            catalog,
            world,
            sim_id,
        }
    }

    /// Starts an asynchronous interaction tree on `obj_id` with `stack_obj`, stepping
    /// turns (invoking `on_yield` after each `Status::Yielded`) until it returns.
    fn step_tree(
        vm: &mut Interpreter<'_>,
        world: &mut World,
        obj_id: u16,
        tree_id: u16,
        stack_obj: i16,
        max_turns: usize,
        mut on_yield: impl FnMut(&mut World, usize),
    ) -> (bool, usize) {
        let guid = world.objects[&obj_id].guid;
        let code_owner_type = world
            .types
            .iter()
            .find_map(|(&tid, &g)| (g == guid).then_some(tid))
            .expect("object GUID missing from world.types");
        let (args_count, locals_count) = {
            let def = vm.catalog.object(guid).unwrap();
            let bhav = &vm.program.libraries[&def.resource].behaviors[&tree_id];
            (usize::from(bhav.args), usize::from(bhav.locals))
        };
        world.frame_storage.remove(&obj_id);
        world.objects.get_mut(&obj_id).unwrap().stack = vec![Frame {
            stack_object: stack_obj,
            tree: tree_id,
            node: 0,
            parameters: vec![0; args_count],
            locals: vec![0; locals_count],
            primitive_state: 0,
            code_owner_type: code_owner_type as i16,
        }];
        world.objects.get_mut(&obj_id).unwrap().data[25] = 1;

        let mut yields = 0usize;
        for _ in 0..max_turns {
            match vm
                .turn(world, obj_id, 2_000)
                .expect("VM trap during step_tree")
            {
                Status::Yielded => {
                    yields += 1;
                    on_yield(world, yields);
                }
                Status::Returned(result) => return (result, yields),
                Status::Running => {}
            }
        }
        panic!("tree {tree_id} on object {obj_id} did not return within {max_turns} turns");
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
fn open_pack_compiles_from_disk_and_all_chunks_and_sprites_validate() {
    let pack = compile_pack_dir(&open_pack_dir()).expect("open-pack compile failed");
    assert_eq!(pack.name, "Open Starter Pack");
    assert_eq!(pack.archive_name, "open-pack.far");
    assert_eq!(pack.objects.len(), 10);

    // Determinism check: compiling twice yields bit-identical FAR and WAV bytes.
    let pack2 = compile_pack_dir(&open_pack_dir()).expect("second compile failed");
    assert_eq!(pack.far_bytes, pack2.far_bytes);
    assert_eq!(pack.sounds_wav, pack2.sounds_wav);

    // Verify packaged FAR directory and parse every chunk of every member IFF.
    let manifest_off =
        far::manifest_offset(&pack.far_bytes[..16], pack.far_bytes.len() as u64).unwrap();
    let dir_len = far::directory_len(manifest_off, pack.far_bytes.len() as u64).unwrap();
    let dir = far::directory(
        &pack.far_bytes[manifest_off as usize..manifest_off as usize + dir_len],
        pack.far_bytes.len() as u64,
    )
    .unwrap();
    assert_eq!(dir.len(), 10);

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

        // OBJD + OBJf for every definition
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

        // PALT + SPR2 + DGRP across all 12 views (4 rotations * 3 zooms)
        let palt = Palette::parse(iff.data(iff.get(b"PALT", 100).unwrap())).unwrap();
        assert!(palt.0.len() >= 4);
        let spr2 = SpriteSet::parse(iff.data(iff.get(b"SPR2", 100).unwrap())).unwrap();
        assert_eq!(spr2.frames.len(), 12);
        let dgrp = DrawGroup::parse(iff.data(iff.get(b"DGRP", 100).unwrap())).unwrap();
        assert_eq!(dgrp.views.len(), 12);
        for view in &dgrp.views {
            let rendered = render(&iff, view).expect("DGRP view render failed");
            assert!(rendered.width > 0 && rendered.height > 0);
        }

        // Every BHAV, BCON, and STR# parses cleanly
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

    // Verify all synthesized WAV sound effects are valid RIFF/WAVE PCM files.
    assert!(pack.sounds_wav.len() >= 12);
    for (name, wav) in &pack.sounds_wav {
        assert!(wav.len() > 44, "WAV '{name}' too small");
        assert_eq!(&wav[..4], b"RIFF", "WAV '{name}' missing RIFF");
        assert_eq!(&wav[8..12], b"WAVE", "WAV '{name}' missing WAVE");
    }
}

#[test]
fn vm_migrated_starter_objects_run_cleanly_from_disk_pack() {
    let pack = compile_pack_dir(&open_pack_dir()).unwrap();
    let mut fx = PackVmFixture::from_pack(&pack, 500);
    let sounds = RefCell::new(Vec::new());
    let mut vm = Interpreter::new(&fx.program, &fx.catalog, Stream::Lot);
    vm.sounds = Some(&sounds);

    let bench = vm.create(&mut fx.world, BENCH_GUID).unwrap();
    let cot = vm.create(&mut fx.world, COT_GUID).unwrap();
    let basin = vm.create(&mut fx.world, BASIN_GUID).unwrap();
    let sim = fx.sim_id;

    // Bench: SitAndRest (+40 comfort)
    assert!(
        vm.run_tree(&mut fx.world, bench, 4098, sim as i16, &[], 500)
            .unwrap()
    );
    assert!(
        vm.run_tree(&mut fx.world, bench, 4099, sim as i16, &[], 500)
            .unwrap()
    );
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_COMFORT],
        20.0
    );

    // Cot: TakeNap yields 6 turns (3 * idle(2)), +60 energy
    let (ok, yields) =
        PackVmFixture::step_tree(&mut vm, &mut fx.world, cot, 4099, sim as i16, 32, |_, _| {});
    assert!(ok);
    assert_eq!(yields, 6);
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_ENERGY],
        20.0
    );

    // Basin: UseBasin sets bladder=100 and hygiene += 25
    assert!(
        vm.run_tree(&mut fx.world, basin, 4099, sim as i16, &[], 500)
            .unwrap()
    );
    let p = fx.world.objects[&sim].person.as_ref().unwrap();
    assert_eq!(p.motives[MOTIVE_BLADDER], 100.0);
    assert_eq!(p.motives[MOTIVE_HYGIENE], -5.0);

    assert_eq!(
        played_sounds(&sounds),
        vec!["open_bench_creak", "open_cot_settle", "open_basin_flush"]
    );
}

#[test]
fn vm_pattern_a_multi_tile_double_bed_sleep_occupancy_and_mid_cancel() {
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

    // Verify slave tile was created and linked to lead tile.
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

    // Verify both left and right routing slots exist in the bed's SLOT chunk.
    let bed_def = fx.catalog.object(BED_LEAD_GUID).unwrap();
    assert_eq!(fx.program.libraries[&bed_def.resource].slots[&128].len(), 2);

    let sim = fx.sim_id;
    // Start with tired Sim: energy = -40, comfort = -20.
    assert!(
        vm.run_tree(&mut fx.world, lead, 4099, sim as i16, &[], 500)
            .unwrap()
    ); // CanSleepLeft
    assert!(
        vm.run_tree(&mut fx.world, lead, 4100, sim as i16, &[], 500)
            .unwrap()
    ); // CanSleepRight

    // 1. Normal full sleep on Left Side (4102): verify shared occupancy synced to BOTH tiles mid-sleep!
    let mut saw_synced_occupancy = false;
    let (ok, yields) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        lead,
        4102,
        sim as i16,
        64,
        |world, y| {
            if y == 1 {
                // Both lead and slave must reflect left-side occupancy (bit 0 = 1)!
                assert_eq!(world.objects[&lead].attributes[0], 1);
                assert_eq!(world.objects[&slave].attributes[0], 1);
                saw_synced_occupancy = true;
            }
        },
    );
    assert!(ok);
    assert!(saw_synced_occupancy);
    assert_eq!(yields, 10, "5 cycles * idle(2) = 10 yields");

    // Energy gained: -40 + 5*24 = +80, Comfort gained: -20 + 5*10 = +30.
    let p = fx.world.objects[&sim].person.as_ref().unwrap();
    assert_eq!(p.motives[MOTIVE_ENERGY], 80.0);
    assert_eq!(p.motives[MOTIVE_COMFORT], 30.0);
    // Wake cleanup must clear occupancy on BOTH lead and slave tiles!
    assert_eq!(fx.world.objects[&lead].attributes[0], 0);
    assert_eq!(fx.world.objects[&slave].attributes[0], 0);
    assert_eq!(fx.world.objects[&lead].attributes[2], 5);

    // 2. Mid-sleep cancel on Right Side (4103): lower energy to 0, cancel after 1 yield!
    fx.world
        .objects
        .get_mut(&sim)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .motives[MOTIVE_ENERGY] = 0.0;
    let mut saw_right_occ = false;
    let (ok2, yields2) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        lead,
        4103,
        sim as i16,
        32,
        |world, y| {
            if y == 1 {
                assert_eq!(world.objects[&lead].attributes[0], 2);
                assert_eq!(world.objects[&slave].attributes[0], 2);
                saw_right_occ = true;
                // Trigger mid-sleep cancel!
                world.objects.get_mut(&lead).unwrap().attributes[1] = 1;
            }
        },
    );
    assert!(ok2);
    assert!(saw_right_occ);
    assert_eq!(
        yields2, 2,
        "finishes active idle(2) then exits cleanly on cancel"
    );
    // Cancel before motive add on cycle 1 means energy stayed 0 and BOTH tiles released cleanly!
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_ENERGY],
        0.0
    );
    assert_eq!(fx.world.objects[&lead].attributes[0], 0);
    assert_eq!(fx.world.objects[&slave].attributes[0], 0);
    assert_eq!(fx.world.objects[&lead].attributes[1], 0);

    // Both settle and wake sounds fired for both sleep sessions.
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
fn vm_pattern_b_multi_object_cooking_chain_and_skill_scaling() {
    let pack = compile_pack_dir(&open_pack_dir()).unwrap();
    let mut fx = PackVmFixture::from_pack(&pack, 100);
    let sounds = RefCell::new(Vec::new());
    let mut vm = Interpreter::new(&fx.program, &fx.catalog, Stream::Lot);
    vm.sounds = Some(&sounds);

    let fridge = vm.create(&mut fx.world, FRIDGE_GUID).unwrap();
    let counter = vm.create(&mut fx.world, COUNTER_GUID).unwrap();
    let stove = vm.create(&mut fx.world, STOVE_GUID).unwrap();
    let sim = fx.sim_id;

    // Verify Init populated stock & functional scores cleanly.
    assert_eq!(fx.world.objects[&fridge].attributes[0], 5);
    assert_eq!(fx.world.objects[&counter].data[31], 50);
    assert_eq!(fx.world.objects[&stove].data[32], 60);

    // CanCookMeal rejects when household funds < $15, accepts at $100.
    fx.world.budget.funds = 10;
    assert!(
        !vm.run_tree(&mut fx.world, fridge, 4098, sim as i16, &[], 500)
            .unwrap()
    );
    fx.world.budget.funds = 100;
    assert!(
        vm.run_tree(&mut fx.world, fridge, 4098, sim as i16, &[], 500)
            .unwrap()
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
    let (cooked1, yields1) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        fridge,
        4099,
        sim as i16,
        32,
        |_, _| {},
    );
    assert!(cooked1);
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

    let plate1 = fx.world.objects[&fridge].attributes[2] as u16;
    assert!(plate1 > 0);
    assert_eq!(fx.world.objects[&plate1].guid, PLATE_GUID);
    let novice_nutrition = fx.world.objects[&plate1].attributes[0];
    assert_eq!(novice_nutrition, 40);
    assert_eq!(fx.world.objects[&plate1].attributes[1], 1); // full plate

    // Sim eats plate1: hunger -50 -> -10, plate marked empty and queued for removal!
    assert!(
        vm.run_tree(&mut fx.world, plate1, 4098, sim as i16, &[], 500)
            .unwrap()
    ); // CanEat
    assert!(
        !vm.run_tree(&mut fx.world, plate1, 4099, sim as i16, &[], 500)
            .unwrap()
    ); // CanClean
    let (ate1, eat_yields) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        plate1,
        4101,
        sim as i16,
        32,
        |_, _| {},
    );
    assert!(ate1);
    assert_eq!(eat_yields, 2);
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_HUNGER],
        -10.0
    );
    assert_eq!(fx.world.objects[&plate1].attributes[1], 0);
    assert!(
        !vm.run_tree(&mut fx.world, plate1, 4098, sim as i16, &[], 500)
            .unwrap()
    ); // CanEat false
    assert!(
        vm.run_tree(&mut fx.world, plate1, 4099, sim as i16, &[], 500)
            .unwrap()
    ); // CanClean true
    assert!(
        fx.world.removals.contains(&plate1),
        "EatMeal must dispose/remove the plate via remove_object"
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
    let (cooked2, _) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        fridge,
        4099,
        sim as i16,
        32,
        |_, _| {},
    );
    assert!(cooked2);
    let plate2 = fx.world.objects[&fridge].attributes[2] as u16;
    assert_ne!(plate2, plate1);
    let chef_nutrition = fx.world.objects[&plate2].attributes[0];
    assert_eq!(chef_nutrition, 70);
    assert!(chef_nutrition > novice_nutrition);

    let (ate2, _) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        plate2,
        4101,
        sim as i16,
        32,
        |_, _| {},
    );
    assert!(ate2);
    // Hunger: -10 + 70 = +60!
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_HUNGER],
        60.0
    );
    assert!(fx.world.removals.contains(&plate2));

    // Verify all cooking-chain FWAV sound events fired across Fridge, Counter, Stove, and Plate!
    assert_eq!(
        played_sounds(&sounds),
        vec![
            "open_fridge_door",
            "open_counter_chop",
            "open_stove_sizzle",
            "open_plate_eat",
            "open_plate_dispose",
            "open_fridge_door",
            "open_counter_chop",
            "open_stove_sizzle",
            "open_plate_eat",
            "open_plate_dispose",
        ]
    );
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

    // Verify STR# 301 dialog string table loaded cleanly.
    let phone_def = fx.catalog.object(PHONE_GUID).unwrap();
    let str_301 = &fx.program.libraries[&phone_def.resource].strings[&301];
    assert_eq!(str_301[0], "Open Wall Phone");
    assert!(str_301.len() >= 8);

    // 1. QuickOrderGroceries (2-button Yes/Cancel) -> Accept (FIRST)
    let (ok_yes, _) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        phone,
        4102,
        sim as i16,
        16,
        |world, y| {
            if y == 1 {
                let rec = world
                    .dialogs
                    .records
                    .iter_mut()
                    .rev()
                    .find(|r| r.caller == phone)
                    .expect("expected pending phone dialog");
                rec.status = DIALOG_FIRST;
            }
        },
    );
    assert!(ok_yes);
    assert_eq!(fx.world.budget.funds, 160); // 200 - 40
    assert_eq!(fx.world.objects[&fridge].attributes[0], 9); // 5 + 4
    assert_eq!(fx.world.objects[&phone].attributes[1], 1); // groceries ordered
    assert_eq!(fx.world.objects[&phone].attributes[0], 0); // lock released
    // Simulate end-of-tick dialog registry cleanup (removes DONE records).
    fx.world.dialogs.records.retain(|r| r.status != DIALOG_DONE);

    // 2. QuickOrderGroceries (2-button Yes/Cancel) -> Cancel (SECOND)
    let (ok_no, _) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        phone,
        4102,
        sim as i16,
        16,
        |world, y| {
            if y == 1 {
                let rec = world
                    .dialogs
                    .records
                    .iter_mut()
                    .rev()
                    .find(|r| r.caller == phone)
                    .expect("expected pending phone dialog");
                rec.status = DIALOG_SECOND;
            }
        },
    );
    assert!(ok_no);
    assert_eq!(fx.world.budget.funds, 160); // unchanged
    assert_eq!(fx.world.objects[&fridge].attributes[0], 9); // unchanged
    assert_eq!(fx.world.objects[&phone].attributes[1], 4); // canceled
    assert_eq!(fx.world.objects[&phone].attributes[0], 0);
    fx.world.dialogs.records.retain(|r| r.status != DIALOG_DONE);

    // 3. UsePhone (3-button menu) -> Button 1: Order Groceries (FIRST)
    let (ok_menu_groceries, _) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        phone,
        4101,
        sim as i16,
        16,
        |world, y| {
            if y == 1 {
                let rec = world
                    .dialogs
                    .records
                    .iter_mut()
                    .rev()
                    .find(|r| r.caller == phone)
                    .expect("expected pending 3-button menu dialog");
                rec.status = DIALOG_FIRST;
            }
        },
    );
    assert!(ok_menu_groceries);
    assert_eq!(fx.world.budget.funds, 120); // 160 - 40
    assert_eq!(fx.world.objects[&fridge].attributes[0], 13); // 9 + 4
    assert_eq!(fx.world.objects[&phone].attributes[1], 1);
    fx.world.dialogs.records.retain(|r| r.status != DIALOG_DONE);

    // 4. UsePhone (3-button menu) -> Button 2: Chat with Neighbor (SECOND, temp[0] == 0)
    let social_before = fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_SOCIAL];
    let fun_before = fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_FUN];
    let (ok_chat, chat_yields) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        phone,
        4101,
        sim as i16,
        24,
        |world, y| {
            if y == 1 {
                let rec = world
                    .dialogs
                    .records
                    .iter_mut()
                    .rev()
                    .find(|r| r.caller == phone)
                    .expect("expected pending 3-button menu dialog");
                rec.status = DIALOG_SECOND;
            }
        },
    );
    assert!(ok_chat);
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
    fx.world.dialogs.records.retain(|r| r.status != DIALOG_DONE);

    // 5. UsePhone (3-button menu) -> Button 3: Call for Info (THIRD, temp[0] == 1)
    let records_before = fx.world.dialogs.records.len();
    let (ok_info, _) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        phone,
        4101,
        sim as i16,
        16,
        |world, y| {
            if y == 1 {
                let rec = world
                    .dialogs
                    .records
                    .iter_mut()
                    .rev()
                    .find(|r| r.caller == phone)
                    .expect("expected pending 3-button menu dialog");
                rec.status = DIALOG_THIRD;
            }
        },
    );
    assert!(ok_info);
    assert_eq!(fx.world.objects[&phone].attributes[1], 3);
    assert_eq!(
        fx.world.dialogs.records.len(),
        records_before + 2,
        "menu dialog + non-blocking info bulletin dialog"
    );
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().motives[MOTIVE_FUN],
        fun_before + 25.0
    );

    // 6. Verify ring, dial, and confirm FWAV sound events fired cleanly!
    let played = played_sounds(&sounds);
    assert!(played.contains(&"open_phone_ring".to_owned()));
    assert!(played.contains(&"open_phone_dial".to_owned()));
    assert!(played.contains(&"open_phone_confirm".to_owned()));
}

#[test]
fn vm_pattern_d_repairable_privacy_shower_guards_wear_repair_and_cancel() {
    let pack = compile_pack_dir(&open_pack_dir()).unwrap();
    let mut fx = PackVmFixture::from_pack(&pack, 500);
    let sounds = RefCell::new(Vec::new());
    let mut vm = Interpreter::new(&fx.program, &fx.catalog, Stream::Lot);
    vm.sounds = Some(&sounds);

    let shower = vm.create(&mut fx.world, SHOWER_GUID).unwrap();
    let sim = fx.sim_id;

    // Initially working: Use guard accepts, Repair guard rejects.
    assert!(
        vm.run_tree(&mut fx.world, shower, 4098, sim as i16, &[], 500)
            .unwrap()
    );
    assert!(
        !vm.run_tree(&mut fx.world, shower, 4099, sim as i16, &[], 500)
            .unwrap()
    );

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
        let (ok, _) = PackVmFixture::step_tree(
            &mut vm,
            &mut fx.world,
            shower,
            4100,
            sim as i16,
            32,
            |world, y| {
                if y == 1 {
                    assert_eq!(world.objects[&shower].attributes[0], 1);
                    saw_privacy_lock = true;
                }
            },
        );
        assert!(ok);
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

    // Broken state guards: CanTakeShower MUST reject, CanRepairShower MUST accept!
    assert!(
        !vm.run_tree(&mut fx.world, shower, 4098, sim as i16, &[], 500)
            .unwrap(),
        "Use guard must block when shower is broken"
    );
    assert!(
        vm.run_tree(&mut fx.world, shower, 4099, sim as i16, &[], 500)
            .unwrap(),
        "Repair guard must enable when shower is broken"
    );

    // Mid-repair cancel: cancels cleanly, leaves shower broken, resets occupancy & cancel flags!
    let (ok_cancel, _) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        shower,
        4101,
        sim as i16,
        32,
        |world, y| {
            if y == 1 {
                world.objects.get_mut(&shower).unwrap().attributes[3] = 1;
            }
        },
    );
    assert!(ok_cancel);
    assert_eq!(
        fx.world.objects[&shower].attributes[2], 1,
        "canceled repair leaves shower broken"
    );
    assert_eq!(
        fx.world.objects[&shower].attributes[0], 0,
        "cancel clears occupancy lock"
    );
    assert_eq!(
        fx.world.objects[&shower].attributes[3], 0,
        "cancel clears cancel flag"
    );

    // Full repair with low Mechanical skill (0 -> step 20 -> 5 cycles).
    fx.world
        .objects
        .get_mut(&sim)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[SKILL_MECHANICAL] = 0;
    let (repaired_low, _) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        shower,
        4101,
        sim as i16,
        32,
        |_, _| {},
    );
    assert!(repaired_low);
    let low_skill_cycles = fx.world.objects[&shower].attributes[4];
    assert_eq!(low_skill_cycles, 5);
    assert_eq!(fx.world.objects[&shower].attributes[2], 0); // fixed!
    assert_eq!(fx.world.objects[&shower].attributes[1], 0); // wear reset!
    assert_eq!(
        fx.world.objects[&sim].person.as_ref().unwrap().data[SKILL_MECHANICAL],
        15
    );

    // Guards flip back: Use succeeds again, Repair rejects!
    assert!(
        vm.run_tree(&mut fx.world, shower, 4098, sim as i16, &[], 500)
            .unwrap()
    );
    assert!(
        !vm.run_tree(&mut fx.world, shower, 4099, sim as i16, &[], 500)
            .unwrap()
    );

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
    let (repaired_high, _) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        shower,
        4101,
        sim as i16,
        32,
        |_, _| {},
    );
    assert!(repaired_high);
    let high_skill_cycles = fx.world.objects[&shower].attributes[4];
    assert_eq!(high_skill_cycles, 2);
    assert!(high_skill_cycles < low_skill_cycles);

    // Shower is usable again and completes cleanly!
    let (used_again, _) = PackVmFixture::step_tree(
        &mut vm,
        &mut fx.world,
        shower,
        4100,
        sim as i16,
        32,
        |_, _| {},
    );
    assert!(used_again);
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
fn dsl_rejects_invalid_break_continue_duplicate_locals_and_conflicting_guards() {
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
    assert!(stdout.contains("10 objects"));
}
