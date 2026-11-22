//! End-to-end clean-room starter pack test:
//! Compiles 3 100% original objects written in high-level `.antics` + `object.toml`
//! (Open Pine Bench, Open Camp Cot, Open Ceramic Basin), packages them into a
//! `.far` archive, verifies every chunk parses via `sims_formats`, AND executes
//! their compiled `BHAV` trees live inside `sims-vm` with zero VM traps!

use sims_antics::compile::{CompiledObject, compile_object_from_sources, package_far};
use sims_content::Catalog;
use sims_core::GameRng;
use sims_formats::behavior::{Behavior, InteractionTable, ObjectDefinition};
use sims_formats::execution::{ObjectFunction, ObjectFunctions, Slots};
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

const BENCH_GUID: u32 = 0x7F00_0001;
const COT_GUID: u32 = 0x7F00_0002;
const BASIN_GUID: u32 = 0x7F00_0003;
const SIM_GUID: u32 = 0x7F00_00FF;

// Motive indices from sims-sim/src/motives.rs
const MOTIVE_ENERGY: usize = 5;
const MOTIVE_COMFORT: usize = 6;
const MOTIVE_HYGIENE: usize = 8;
const MOTIVE_BLADDER: usize = 9;

const BENCH_TOML: &str = r#"
[object]
name = "Open Pine Bench"
description = "A clean-room pine wood bench that restores Comfort."
guid = 0x7F00_0001
price = 120
init = "Init"
main = "Main"
scripts = ["bench.antics"]

[sprite]
shape = "bench"
primary_rgb = [156, 106, 62]
accent_rgb = [206, 162, 110]
trim_rgb = [76, 52, 32]

[[interaction]]
name = "Sit and Rest"
action = "SitAndRest"
test = "CanSit"
autonomy = 55
attenuation = 0.02
motives = [
    { motive = "comfort", min = -100, delta = 45, personality = 0 },
]

[[slot]]
kind = 3
offset = [0.0, -0.75, 0.0]
standing = 1
sitting = 1
ground = 1
facing = 0

[[sound]]
id = 1
name = "open_bench_creak"
preset = "wood_creak"
"#;

const BENCH_ANTICS: &str = r#"
tree 4096 "Init" {
    my.attr[0] := 0;
    return true;
}

tree 4097 "Main" {
    return true;
}

tree 4098 "CanSit" {
    if my.attr[0] != 0 {
        return false;
    }
    if stack.motive.comfort >= 95 {
        return false;
    }
    return true;
}

interaction 4099 "SitAndRest" (locals: 1) {
    // Mark bench occupied
    my.attr[0] := 1;
    play_sound(1);
    local[0] := 0;
    while local[0] < 4 {
        stack.motive.comfort += 10;
        local[0] += 1;
    }
    // Release bench
    my.attr[0] := 0;
    return true;
}
"#;

const COT_TOML: &str = r#"
[object]
name = "Open Camp Cot"
description = "A clean-room folding camp cot that restores Energy over multiple ticks."
guid = 0x7F00_0002
price = 210
init = "Init"
main = "Main"
scripts = ["cot.antics"]

[sprite]
shape = "cot"
primary_rgb = [88, 124, 96]
accent_rgb = [228, 224, 210]
trim_rgb = [64, 72, 68]

[[interaction]]
name = "Take a Nap"
action = "TakeNap"
test = "CanSleep"
autonomy = 65
attenuation = 0.015
motives = [
    { motive = "energy", min = -100, delta = 60, personality = 0 },
    { motive = "comfort", min = -100, delta = 20, personality = 0 },
]

[[slot]]
kind = 3
offset = [0.75, 0.0, 0.0]
standing = 1
sitting = 0
ground = 1
facing = 2

[[sound]]
id = 1
name = "open_cot_settle"
preset = "soft_thud"
"#;

const COT_ANTICS: &str = r#"
tree 4096 "Init" {
    my.attr[0] := 0;
    return true;
}

tree 4097 "Main" {
    return true;
}

tree 4098 "CanSleep" {
    if stack.motive.energy < 80 {
        return true;
    } else {
        return false;
    }
}

interaction 4099 "TakeNap" (args: 1, locals: 1) {
    my.attr[0] := 1;
    play_sound(1);
    local[0] := 0;
    while local[0] < 3 {
        idle(2);
        stack.motive.energy += 20;
        stack.motive.comfort += 5;
        local[0] += 1;
    }
    my.attr[0] := 0;
    return true;
}
"#;

const BASIN_TOML: &str = r#"
[object]
name = "Open Ceramic Basin"
description = "A clean-room porcelain fixture that restores Bladder and Hygiene."
guid = 0x7F00_0003
price = 280
init = "Init"
main = "Main"
scripts = ["basin.antics"]

[sprite]
shape = "basin"
primary_rgb = [232, 236, 240]
accent_rgb = [208, 224, 238]
trim_rgb = [148, 158, 168]

[[interaction]]
name = "Use Basin"
action = "UseBasin"
test = "CanUse"
autonomy = 70
attenuation = 0.025
motives = [
    { motive = "bladder", min = -100, delta = 80, personality = 0 },
    { motive = "hygiene", min = -100, delta = 25, personality = 0 },
]

[[slot]]
kind = 3
offset = [0.0, -0.65, 0.0]
standing = 1
sitting = 0
ground = 1
facing = 0

[[sound]]
id = 1
name = "open_basin_flush"
preset = "water_flush"
"#;

const BASIN_ANTICS: &str = r#"
tree 4096 "Init" {
    my.attr[0] := 0;
    return true;
}

tree 4097 "Main" {
    return true;
}

tree 4098 "CanUse" {
    if stack.motive.bladder >= 90 {
        return false;
    }
    return true;
}

tree 4100 "WashHands" {
    stack.motive.hygiene += 25;
    play_sound(1);
    return true;
}

interaction 4099 "UseBasin" {
    my.attr[0] += 1;
    stack.motive.bladder := 100;
    call WashHands();
    return true;
}
"#;

fn verify_compiled_iff(
    obj: &CompiledObject,
    expected_guid: u32,
    expected_interaction: &str,
) -> Iff {
    let iff = Iff::parse(obj.iff_bytes.clone()).expect("compiled IFF failed to parse");
    assert!(
        iff.map_error().is_none(),
        "IFF rsmp map error for {}",
        obj.name
    );

    // OBJD
    let objd_chunk = iff.get(b"OBJD", 128).expect("missing OBJD #128");
    let objd = ObjectDefinition::parse(iff.data(objd_chunk)).expect("OBJD parse failed");
    assert_eq!(objd.guid, expected_guid);

    // OBJf
    let objf_chunk = iff.get(b"OBJf", 128).expect("missing OBJf #128");
    let objf = ObjectFunctions::parse(iff.data(objf_chunk)).expect("OBJf parse failed");
    assert!(objf.entries.len() >= 2);

    // CTSS
    let ctss_chunk = iff.get(b"CTSS", 2000).expect("missing CTSS #2000");
    let ctss = StringTable::parse(iff.data(ctss_chunk)).expect("CTSS parse failed");
    assert_eq!(ctss.get(0, 1), Some(obj.name.as_str()));

    // TTAB & TTAs
    let ttab_chunk = iff.get(b"TTAB", 128).expect("missing TTAB #128");
    let ttab = InteractionTable::parse(iff.data(ttab_chunk)).expect("TTAB parse failed");
    assert_eq!(ttab.interactions.len(), 1);
    let ttas_chunk = iff.get(b"TTAs", 128).expect("missing TTAs #128");
    let ttas = StringTable::parse(iff.data(ttas_chunk)).expect("TTAs parse failed");
    assert_eq!(ttas.get(0, 1), Some(expected_interaction));

    // SLOT
    let slot_chunk = iff.get(b"SLOT", 128).expect("missing SLOT #128");
    let slots = Slots::parse(iff.data(slot_chunk)).expect("SLOT parse failed");
    assert_eq!(slots.entries.len(), 1);
    assert!(slots.routing(0).is_some());

    // PALT + SPR2 + DGRP + full preview render
    let palt_chunk = iff.get(b"PALT", 100).expect("missing PALT #100");
    let palt = Palette::parse(iff.data(palt_chunk)).expect("PALT parse failed");
    assert!(!palt.0.is_empty());
    let spr2_chunk = iff.get(b"SPR2", 100).expect("missing SPR2 #100");
    let spr2 = SpriteSet::parse(iff.data(spr2_chunk)).expect("SPR2 parse failed");
    assert_eq!(spr2.frames.len(), 12);
    let dgrp_chunk = iff.get(b"DGRP", 100).expect("missing DGRP #100");
    let dgrp = DrawGroup::parse(iff.data(dgrp_chunk)).expect("DGRP parse failed");
    assert_eq!(dgrp.views.len(), 12);
    let preview = render(&iff, &dgrp.views[0]).expect("DGRP render failed");
    assert!(preview.width > 0 && preview.height > 0);

    // Every BHAV parses cleanly via sims_formats::behavior::Behavior
    for chunk in iff.all(b"BHAV") {
        Behavior::parse(iff.data(chunk)).expect("BHAV parse failed");
    }

    // Synthesized WAV is valid RIFF/WAVE
    assert!(!obj.sounds_wav.is_empty());
    for wav in obj.sounds_wav.values() {
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    iff
}

fn library_from_iff(iff: &Iff, guid: u32, kind: i16) -> Library {
    let mut lib = Library::default();
    for chunk in &iff.chunks {
        let data = iff.data(chunk);
        match &chunk.tag {
            b"BHAV" => {
                lib.behaviors
                    .insert(chunk.id, Behavior::parse(data).unwrap());
                lib.names.insert(chunk.id, chunk.label.clone());
            }
            b"OBJf" => {
                lib.callbacks
                    .insert(chunk.id, ObjectFunctions::parse(data).unwrap().entries);
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
                lib.slots
                    .insert(chunk.id, Slots::parse(data).unwrap().entries);
            }
            _ => {}
        }
    }
    let mut words = [0i16; DEFINITION_WORDS];
    words[9] = kind;
    words[14] = guid as u16 as i16;
    words[15] = (guid >> 16) as u16 as i16;
    lib.definitions.insert(128, words);
    lib.definition_order.push(128);
    lib
}

fn make_object(id: u16, guid: u32, is_person: bool) -> Object {
    let person = is_person.then(|| {
        let mut motives = [0.0f32; 16];
        motives[MOTIVE_ENERGY] = -40.0;
        motives[MOTIVE_COMFORT] = -20.0;
        motives[MOTIVE_HYGIENE] = 10.0;
        motives[MOTIVE_BLADDER] = -60.0;
        Person {
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
            active: SavedInteraction {
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
            },
            last: SavedInteraction {
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
            },
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        }
    });
    Object {
        id,
        guid,
        footprint: [0; 4],
        position: [0; 3],
        unknown_short: 0,
        attributes: vec![0; 8],
        temps: vec![0; 8],
        data: vec![0; 72],
        obsolete_word: 0,
        stack_capacity_bytes: 0,
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
        person,
        registry_word: None,
        last_result: false,
        removal_skips_resets: false,
        selected: false,
    }
}

#[test]
fn starter_objects_compile_package_and_execute_live_in_sims_vm() {
    // 1. Compile all 3 clean-room objects from high-level .antics + object.toml
    let bench = compile_object_from_sources(BENCH_TOML, &[("bench.antics", BENCH_ANTICS)])
        .expect("bench compile failed");
    let cot = compile_object_from_sources(COT_TOML, &[("cot.antics", COT_ANTICS)])
        .expect("cot compile failed");
    let basin = compile_object_from_sources(BASIN_TOML, &[("basin.antics", BASIN_ANTICS)])
        .expect("basin compile failed");

    let bench_iff = verify_compiled_iff(&bench, BENCH_GUID, "Sit and Rest");
    let cot_iff = verify_compiled_iff(&cot, COT_GUID, "Take a Nap");
    let basin_iff = verify_compiled_iff(&basin, BASIN_GUID, "Use Basin");

    // 2. Package all 3 objects into one FAR archive and verify via sims_formats::far
    let far_bytes = package_far(&[
        ("open_bench.iff", &bench.iff_bytes),
        ("open_cot.iff", &cot.iff_bytes),
        ("open_basin.iff", &basin.iff_bytes),
    ])
    .expect("FAR packaging failed");
    let manifest_off = far::manifest_offset(&far_bytes[..16], far_bytes.len() as u64).unwrap();
    let dir_len = far::directory_len(manifest_off, far_bytes.len() as u64).unwrap();
    let dir = far::directory(
        &far_bytes[manifest_off as usize..manifest_off as usize + dir_len],
        far_bytes.len() as u64,
    )
    .unwrap();
    assert_eq!(dir.len(), 3);
    for entry in &dir {
        assert!(entry.within_payload(manifest_off));
        let slice = &far_bytes[entry.offset as usize..(entry.offset + entry.stored_size) as usize];
        let parsed = Iff::parse(slice.to_vec()).expect("IFF member inside FAR failed to parse");
        assert!(parsed.map_error().is_none());
    }

    // 3. Build a clean-room sims-vm Program + Catalog + World and run every compiled BHAV!
    const GLOBAL_RES: usize = 0;
    const BENCH_RES: usize = 1;
    const COT_RES: usize = 2;
    const BASIN_RES: usize = 3;
    const SIM_RES: usize = 4;

    const BENCH_OBJ: u16 = 1;
    const COT_OBJ: u16 = 2;
    const BASIN_OBJ: u16 = 3;
    const SIM_OBJ: u16 = 4;

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

    let program = Program {
        global: GLOBAL_RES,
        libraries: BTreeMap::from([
            (GLOBAL_RES, Library::default()),
            (BENCH_RES, library_from_iff(&bench_iff, BENCH_GUID, 4)),
            (COT_RES, library_from_iff(&cot_iff, COT_GUID, 4)),
            (BASIN_RES, library_from_iff(&basin_iff, BASIN_GUID, 4)),
            (SIM_RES, sim_lib),
        ]),
        semi_globals: BTreeMap::from([
            (BENCH_RES, None),
            (COT_RES, None),
            (BASIN_RES, None),
            (SIM_RES, None),
        ]),
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
        global: GLOBAL_RES,
        objects: vec![
            sims_content::Object {
                guid: BENCH_GUID,
                resource: BENCH_RES,
                chunk: 128,
                kind: 4,
                master: 0,
            },
            sims_content::Object {
                guid: COT_GUID,
                resource: COT_RES,
                chunk: 128,
                kind: 4,
                master: 0,
            },
            sims_content::Object {
                guid: BASIN_GUID,
                resource: BASIN_RES,
                chunk: 128,
                kind: 4,
                master: 0,
            },
            sims_content::Object {
                guid: SIM_GUID,
                resource: SIM_RES,
                chunk: 128,
                kind: 2,
                master: 0,
            },
        ],
        by_guid: BTreeMap::from([
            (BENCH_GUID, 0),
            (COT_GUID, 1),
            (BASIN_GUID, 2),
            (SIM_GUID, 3),
        ]),
        semi_globals: BTreeMap::new(),
        diagnostics: Vec::new(),
    };

    let mut world = World {
        globals: vec![0; 64],
        lot_rng: GameRng::from_state(1),
        global_rng: GameRng::from_state(2),
        order: vec![BENCH_OBJ, COT_OBJ, BASIN_OBJ, SIM_OBJ],
        people: vec![SIM_OBJ],
        types: BTreeMap::from([
            (1, BENCH_GUID),
            (2, COT_GUID),
            (3, BASIN_GUID),
            (4, SIM_GUID),
        ]),
        objects: BTreeMap::from([
            (BENCH_OBJ, make_object(BENCH_OBJ, BENCH_GUID, false)),
            (COT_OBJ, make_object(COT_OBJ, COT_GUID, false)),
            (BASIN_OBJ, make_object(BASIN_OBJ, BASIN_GUID, false)),
            (SIM_OBJ, make_object(SIM_OBJ, SIM_GUID, true)),
        ])
        .into(),
        ..World::empty()
    };

    let sounds = RefCell::new(Vec::new());
    let mut vm = Interpreter::new(&program, &catalog, Stream::Lot);
    vm.sounds = Some(&sounds);

    // --- Test 1: Open Pine Bench (Init + Guard + Loop + Comfort + Sound) ---
    assert!(
        vm.run_tree(&mut world, BENCH_OBJ, 4096, 0, &[], 500)
            .unwrap()
    );
    assert!(
        vm.run_tree(&mut world, BENCH_OBJ, 4098, SIM_OBJ as i16, &[], 500)
            .unwrap(),
        "CanSit should allow sitting when comfort is -20"
    );
    assert!(
        vm.run_tree(&mut world, BENCH_OBJ, 4099, SIM_OBJ as i16, &[], 500)
            .unwrap(),
        "SitAndRest should complete without VM traps"
    );
    // Initial comfort -20 + 4 * 10 = +20
    let comfort = world.objects[&SIM_OBJ].person.as_ref().unwrap().motives[MOTIVE_COMFORT];
    assert_eq!(comfort, 20.0);
    assert_eq!(world.objects[&BENCH_OBJ].attributes[0], 0);

    // Raise comfort to 96 and verify CanSit guard returns false
    world
        .objects
        .get_mut(&SIM_OBJ)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .motives[MOTIVE_COMFORT] = 96.0;
    assert!(
        !vm.run_tree(&mut world, BENCH_OBJ, 4098, SIM_OBJ as i16, &[], 500)
            .unwrap(),
        "CanSit guard must reject when comfort >= 95"
    );

    // --- Test 2: Open Camp Cot (Guard + Multi-Tick idle(2) Loop + Energy & Comfort) ---
    assert!(vm.run_tree(&mut world, COT_OBJ, 4096, 0, &[], 500).unwrap());
    assert!(
        vm.run_tree(&mut world, COT_OBJ, 4098, SIM_OBJ as i16, &[], 500)
            .unwrap(),
        "CanSleep should return true when energy is -40"
    );
    // Start TakeNap as an asynchronous thread so Sleep/idle(2) yields across ticks!
    world.frame_storage.remove(&COT_OBJ);
    world.objects.get_mut(&COT_OBJ).unwrap().stack = vec![Frame {
        stack_object: SIM_OBJ as i16,
        tree: 4099,
        node: 0,
        parameters: vec![0],
        locals: vec![0],
        primitive_state: 0,
        code_owner_type: 2, // COT_GUID
    }];
    // Lockout word 25 = 1 so Sleep counts down per turn cleanly in standalone turn stepping
    world.objects.get_mut(&COT_OBJ).unwrap().data[25] = 1;
    let mut yields = 0usize;
    let mut returned = None;
    for _ in 0..32 {
        match vm.turn(&mut world, COT_OBJ, 500).unwrap() {
            Status::Yielded => yields += 1,
            Status::Returned(val) => {
                returned = Some(val);
                break;
            }
            Status::Running => {}
        }
    }
    assert_eq!(returned, Some(true), "TakeNap must finish and return true");
    assert_eq!(
        yields, 6,
        "3 loop iterations * idle(2) should yield 6 turns"
    );
    // Energy started at -40, + 3 * 20 = +20
    let energy = world.objects[&SIM_OBJ].person.as_ref().unwrap().motives[MOTIVE_ENERGY];
    assert_eq!(energy, 20.0);
    assert_eq!(world.objects[&COT_OBJ].attributes[0], 0);

    // --- Test 3: Open Ceramic Basin (Guard + Subroutine Call + Bladder & Hygiene + Sound) ---
    assert!(
        vm.run_tree(&mut world, BASIN_OBJ, 4096, 0, &[], 500)
            .unwrap()
    );
    assert!(
        vm.run_tree(&mut world, BASIN_OBJ, 4098, SIM_OBJ as i16, &[], 500)
            .unwrap(),
        "CanUse should return true when bladder is -60"
    );
    assert!(
        vm.run_tree(&mut world, BASIN_OBJ, 4099, SIM_OBJ as i16, &[], 500)
            .unwrap(),
        "UseBasin (with WashHands subroutine call) should succeed"
    );
    let sim_person = world.objects[&SIM_OBJ].person.as_ref().unwrap();
    assert_eq!(sim_person.motives[MOTIVE_BLADDER], 100.0);
    assert_eq!(sim_person.motives[MOTIVE_HYGIENE], 35.0); // 10 + 25
    assert_eq!(world.objects[&BASIN_OBJ].attributes[0], 1); // usage counter incremented
    assert!(
        !vm.run_tree(&mut world, BASIN_OBJ, 4098, SIM_OBJ as i16, &[], 500)
            .unwrap(),
        "CanUse guard must reject when bladder == 100"
    );

    // Verify all 3 FWAV sound events fired cleanly during VM execution!
    let played: Vec<String> = sounds
        .borrow()
        .iter()
        .filter_map(|e| match e {
            SoundEvent::Play { name, .. } => Some(name.clone()),
            SoundEvent::Stop { .. } => None,
        })
        .collect();
    assert_eq!(
        played,
        vec!["open_bench_creak", "open_cot_settle", "open_basin_flush"]
    );
}
