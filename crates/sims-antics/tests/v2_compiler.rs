//! Comprehensive CI-safe tests for the `.antics` v2 high-level compiler,
//! `.air` (Antics IR) split, `sims-antics expand` CLI, formal safety verifier,
//! and live `sims-vm` execution of v2 prototypes (Camp Cot, Retro TV, Floor Lamp,
//! Frost Fridge + Counter + Stove + Meal Plate).
//!
//! Zero commercial assets required.

use sims_antics::{
    CompiledObject, PackResolverConfig, compile_air, compile_v2_object_file, compile_v2_pack_dir,
    compile_v2_source, expand_v2_to_air, verify_object_safety, verify_pack_safety,
};
use sims_content::Catalog;
use sims_core::GameRng;
use sims_formats::behavior::{Behavior, InteractionTable};
use sims_formats::execution::{ObjectFunction, ObjectFunctions, Slots, constants};
use sims_formats::iff::Iff;
use sims_formats::lot::Array as LotArray;
use sims_formats::saved_objects::{
    Frame, Interaction as SavedInteraction, Person, Slot as SavedSlot,
};
use sims_formats::strings::StringTable;
use sims_sim::rooms::Room;
use sims_sim::{Object, World};
use sims_vm::{DEFINITION_WORDS, Interpreter, Library, Program, SoundEvent, Status, Stream};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const DIALOG_DONE: u8 = 5;
const SIM_GUID: u32 = 0x7F00_00FF;
const MOTIVE_ENERGY: usize = 5;
const MOTIVE_COMFORT: usize = 6;
const MOTIVE_HUNGER: usize = 7;
const MOTIVE_ROOM: usize = 13;
const MOTIVE_FUN: usize = 15;
const SKILL_COOKING: usize = 10;

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

struct CleanupDir(PathBuf);

impl CleanupDir {
    fn new(label: &str) -> Self {
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "sims-antics-v2-{label}-{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for CleanupDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_sims-antics")
}

// Collected immediately into a sorted `BTreeSet` for deterministic comparison.
#[allow(clippy::disallowed_methods)]
fn list_dir(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// Minimal authentic `sims-vm` harness for live execution of v2 CompiledObjects
// ---------------------------------------------------------------------------

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
                lib.strings.insert(
                    chunk.id,
                    table.entries.into_iter().map(|e| e.text).collect(),
                );
            }
            b"CTSS" => {
                let table = StringTable::parse(data).unwrap();
                lib.catalog_strings.insert(
                    chunk.id,
                    table
                        .entries
                        .into_iter()
                        .map(|e| (e.language, e.text))
                        .collect(),
                );
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
    motives[MOTIVE_ROOM] = -10.0;
    motives[MOTIVE_FUN] = -25.0;

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
    person_data[58] = 25;

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

struct V2VmFixture {
    program: Program,
    catalog: Catalog,
    world: World,
    sim_id: u16,
    ttas_by_guid: BTreeMap<u32, Vec<String>>,
}

impl V2VmFixture {
    fn from_objects(objects: &[&CompiledObject], starting_funds: i32) -> Self {
        Self::from_objects_with_libs(objects, starting_funds, None, None)
    }

    /// Like [`Self::from_objects`], but also compiles synthetic `.air` into the global library
    /// (BHAV 256..=4095, BCON 256..) and an optional semi-global library (BHAV/BCON 8192..)
    /// shared by every object.
    fn from_objects_with_libs(
        objects: &[&CompiledObject],
        starting_funds: i32,
        global_air: Option<&str>,
        semi_global_air: Option<&str>,
    ) -> Self {
        const GLOBAL_RES: usize = 0;
        fn air_library(src: &str) -> Library {
            let module = compile_air(src).expect("compile synthetic library .air");
            let mut lib = Library::default();
            for (&id, bhav) in &module.behaviors {
                let raw = sims_antics::encode::encode_bhav(bhav).expect("encode_bhav");
                lib.behaviors.insert(id, Behavior::parse(&raw).unwrap());
                lib.names.insert(id, bhav.label.clone());
            }
            for (&id, bcon) in &module.constants {
                lib.constants.insert(id, bcon.values.clone());
            }
            lib
        }

        let mut catalog = Catalog {
            global: GLOBAL_RES,
            objects: Vec::new(),
            by_guid: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
        let global_lib = global_air.map_or_else(Library::default, air_library);
        let mut libraries = BTreeMap::from([(GLOBAL_RES, global_lib)]);
        let semi_res = objects.len() + 2;
        let semi_link = semi_global_air.map(|src| {
            libraries.insert(semi_res, air_library(src));
            semi_res
        });
        let mut semi_globals = BTreeMap::new();
        let mut ttas_by_guid = BTreeMap::new();

        for (idx, obj) in objects.iter().enumerate() {
            let res_id = idx + 1;
            let iff = Iff::parse(obj.iff_bytes.clone()).expect("IFF parse");
            assert!(iff.map_error().is_none(), "IFF rsmp error in {}", obj.name);
            if let Some(ttas) = iff.get(b"TTAs", 128) {
                let tbl = StringTable::parse(iff.data(ttas)).expect("TTAs parse");
                let names: Vec<String> = tbl.entries.into_iter().map(|e| e.text).collect();
                ttas_by_guid.insert(obj.guid, names);
            }
            let lib = load_iff_into_vm(&iff, res_id, &mut catalog);
            libraries.insert(res_id, lib);
            semi_globals.insert(res_id, semi_link);
        }

        let sim_res = objects.len() + 1;
        let sim_module = compile_air(
            r#"
tree 4097 "SimMain" (args: 1) {
    while 0 == 0 {
        param[0] := 1;
        idle_for_input(param[0], 1);
    }
    return true;
}
"#,
        )
        .expect("compile SimMain");
        let mut sim_lib = Library::default();
        for (&id, bhav) in &sim_module.behaviors {
            let raw = sims_antics::encode::encode_bhav(bhav).expect("encode_bhav");
            sim_lib.behaviors.insert(id, Behavior::parse(&raw).unwrap());
        }
        let mut sim_words = [0i16; DEFINITION_WORDS];
        sim_words[9] = 2; // person
        sim_words[14] = SIM_GUID as u16 as i16;
        sim_words[15] = (SIM_GUID >> 16) as i16;
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
        let sim_idx = catalog.objects.len();
        catalog.objects.push(sims_content::Object {
            guid: SIM_GUID,
            resource: sim_res,
            chunk: 128,
            kind: 2,
            master: 0,
        });
        catalog.by_guid.insert(SIM_GUID, sim_idx);

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
        world.globals[1] = 1;
        world.globals[7] = 1;
        world.globals[8] = 2000;
        world.globals[10] = 1;
        world.globals[14] = (starting_funds / 10000) as i16;
        world.globals[13] = (starting_funds % 10000) as i16;
        world.budget.funds = starting_funds;
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
            ttas_by_guid,
        }
    }

    fn spawn(&mut self, guid: u32, tx: i32, ty: i32) -> u16 {
        let vm = Interpreter::new(&self.program, &self.catalog, Stream::Lot);
        let id = vm
            .create(&mut self.world, guid)
            .expect("vm.create should succeed");
        let px = tx * 16 + 8;
        let py = ty * 16 + 8;
        let obj = self.world.objects.get_mut(&id).unwrap();
        obj.position = [px, py, 1];
        obj.footprint = [px - 8, py - 8, px + 8, py + 8];
        obj.data[1] = 0;
        obj.data[29] = 1;
        let idx = (tx as usize * 64 + ty as usize) * 2;
        let grid = &mut self.world.arrays.get_mut(&3).unwrap().data;
        grid[idx..idx + 2].copy_from_slice(&(id as i16).to_le_bytes());
        id
    }

    fn pie_menu_labels(&mut self, target_id: u16) -> Vec<String> {
        let vm = Interpreter::new(&self.program, &self.catalog, Stream::Lot);
        let entries = vm
            .pie_menu(&mut self.world, self.sim_id, target_id)
            .expect("pie_menu should not trap");
        let guid = self.world.objects[&target_id].guid;
        let names = self.ttas_by_guid.get(&guid);
        entries
            .into_iter()
            .map(|e| {
                e.name.unwrap_or_else(|| {
                    names
                        .and_then(|v| v.get(e.action.table_index as usize))
                        .cloned()
                        .unwrap_or_else(|| format!("entry_{}", e.action.table_index))
                })
            })
            .collect()
    }

    fn run_interaction(
        &mut self,
        target_id: u16,
        label: &str,
        cancel_on_yield: Option<usize>,
    ) -> Vec<SoundEvent> {
        let sounds = RefCell::new(Vec::new());
        {
            let mut vm = Interpreter::new(&self.program, &self.catalog, Stream::Lot);
            vm.sounds = Some(&sounds);
            let entries = vm
                .pie_menu(&mut self.world, self.sim_id, target_id)
                .expect("pie_menu");
            let guid = self.world.objects[&target_id].guid;
            let names = self.ttas_by_guid.get(&guid);
            let chosen = entries
                .into_iter()
                .find(|e| {
                    let ename = e.name.clone().unwrap_or_else(|| {
                        names
                            .and_then(|v| v.get(e.action.table_index as usize))
                            .cloned()
                            .unwrap_or_default()
                    });
                    ename == label
                })
                .unwrap_or_else(|| {
                    panic!("interaction '{label}' not offered on object {target_id}")
                });
            let uid = vm
                .pick(&mut self.world, self.sim_id, chosen.action)
                .expect("pick")
                .expect("queue accepted");
            assert!(uid > 0);

            let mut active_yields = 0usize;
            let mut saw_started = false;
            let mut cancelled = false;
            for _ in 0..80 {
                self.world.objects.get_mut(&self.sim_id).unwrap().data[25] = 1;
                let status = vm
                    .turn(&mut self.world, self.sim_id, 4_000)
                    .expect("VM trap during interaction turn");
                let depth = self.world.objects[&self.sim_id].stack.len();
                if depth > 1 {
                    saw_started = true;
                    if matches!(status, Status::Yielded) {
                        active_yields += 1;
                        if cancel_on_yield == Some(active_yields) && !cancelled {
                            assert!(
                                vm.cancel(&mut self.world, self.sim_id, uid)
                                    .expect("vm.cancel"),
                                "cancel should return true"
                            );
                            cancelled = true;
                        }
                    }
                } else if saw_started {
                    break;
                } else if matches!(status, Status::Yielded) {
                    let person = self.world.objects[&self.sim_id].person.as_ref().unwrap();
                    if person.queue.is_empty()
                        && (person.active.uid == uid || person.last.uid == uid)
                    {
                        break;
                    }
                }
            }
            if let Some(person) = self
                .world
                .objects
                .get_mut(&self.sim_id)
                .unwrap()
                .person
                .as_mut()
            {
                person.active.uid = 0;
                person.active.flags = 0;
                person.data[33] = 0;
                person.object_uses.clear();
            }
            self.world
                .dialogs
                .records
                .retain(|r| r.status != DIALOG_DONE);
        }
        sounds.into_inner()
    }

    fn run_interaction_with_dialog(&mut self, target_id: u16, label: &str, answer_status: u8) {
        let vm = Interpreter::new(&self.program, &self.catalog, Stream::Lot);
        let entries = vm
            .pie_menu(&mut self.world, self.sim_id, target_id)
            .expect("pie_menu");
        let guid = self.world.objects[&target_id].guid;
        let names = self.ttas_by_guid.get(&guid);
        let chosen = entries
            .into_iter()
            .find(|e| {
                let ename = e.name.clone().unwrap_or_else(|| {
                    names
                        .and_then(|v| v.get(e.action.table_index as usize))
                        .cloned()
                        .unwrap_or_default()
                });
                ename == label
            })
            .unwrap_or_else(|| panic!("interaction '{label}' not offered on object {target_id}"));
        let uid = vm
            .pick(&mut self.world, self.sim_id, chosen.action)
            .expect("pick")
            .expect("queue accepted");
        assert!(uid > 0);
        let mut saw_started = false;
        for _ in 0..80 {
            self.world.objects.get_mut(&self.sim_id).unwrap().data[25] = 1;
            let status = vm
                .turn(&mut self.world, self.sim_id, 4_000)
                .expect("VM trap during dialog interaction");
            if let Some(rec) = self
                .world
                .dialogs
                .records
                .iter_mut()
                .rev()
                .find(|r| r.caller == self.sim_id && r.status != DIALOG_DONE)
            {
                rec.status = answer_status;
            }
            let depth = self.world.objects[&self.sim_id].stack.len();
            if depth > 1 {
                saw_started = true;
            } else if saw_started {
                break;
            } else if matches!(status, Status::Yielded) {
                let person = self.world.objects[&self.sim_id].person.as_ref().unwrap();
                if person.queue.is_empty() && (person.active.uid == uid || person.last.uid == uid) {
                    break;
                }
            }
        }
        if let Some(person) = self
            .world
            .objects
            .get_mut(&self.sim_id)
            .unwrap()
            .person
            .as_mut()
        {
            person.active.uid = 0;
            person.active.flags = 0;
            person.data[33] = 0;
            person.object_uses.clear();
        }
        self.world
            .dialogs
            .records
            .retain(|r| r.status != DIALOG_DONE);
    }

    fn eval_guard_tree(&mut self, target_id: u16, tree: u16, param0: i16) -> bool {
        let sim_id = self.sim_id;
        let target_guid = self.world.objects[&target_id].guid;
        let code_owner_type = self
            .world
            .types
            .iter()
            .find_map(|(&ty_id, &guid)| (guid == target_guid).then_some(ty_id as i16))
            .expect("target object type registered");
        let saved_stack = self.world.objects[&sim_id].stack.clone();
        let saved_storage = self.world.frame_storage.remove(&sim_id);
        self.world.objects.get_mut(&sim_id).unwrap().stack = vec![Frame {
            stack_object: target_id as i16,
            tree,
            node: 0,
            parameters: vec![param0, 0, 0, 0],
            locals: vec![0; 16],
            primitive_state: 0,
            code_owner_type,
        }];
        let vm = Interpreter::new(&self.program, &self.catalog, Stream::Lot);
        let status = vm
            .turn(&mut self.world, sim_id, 4_000)
            .expect("guard tree must not trap");
        assert!(
            matches!(status, Status::Returned(_)),
            "guard tree must return synchronously, got {status:?}"
        );
        let res = matches!(status, Status::Returned(true));
        self.world.objects.get_mut(&sim_id).unwrap().stack = saved_stack;
        self.world.frame_storage.remove(&sim_id);
        if let Some(storage) = saved_storage {
            self.world.frame_storage.insert(sim_id, storage);
        }
        res
    }

    /// Runs `tree` on the Sim with `target_id` as the stack object and `param0`, returning the
    /// tree's true/false exit or the VM fault (e.g. `Trap::ErrorBranch` from `return error;`).
    fn try_run_tree(
        &mut self,
        target_id: u16,
        tree: u16,
        param0: i16,
    ) -> Result<bool, sims_vm::Fault> {
        let sim_id = self.sim_id;
        let target_guid = self.world.objects[&target_id].guid;
        let code_owner_type = self
            .world
            .types
            .iter()
            .find_map(|(&ty_id, &guid)| (guid == target_guid).then_some(ty_id as i16))
            .expect("target object type registered");
        let saved_stack = self.world.objects[&sim_id].stack.clone();
        let saved_storage = self.world.frame_storage.remove(&sim_id);
        self.world.objects.get_mut(&sim_id).unwrap().stack = vec![Frame {
            stack_object: target_id as i16,
            tree,
            node: 0,
            parameters: vec![param0, 0, 0, 0],
            locals: vec![0; 16],
            primitive_state: 0,
            code_owner_type,
        }];
        let vm = Interpreter::new(&self.program, &self.catalog, Stream::Lot);
        let status = vm.turn(&mut self.world, sim_id, 4_000);
        self.world.objects.get_mut(&sim_id).unwrap().stack = saved_stack;
        self.world.frame_storage.remove(&sim_id);
        if let Some(storage) = saved_storage {
            self.world.frame_storage.insert(sim_id, storage);
        }
        let status = status?;
        assert!(
            matches!(status, Status::Returned(_)),
            "tree must return synchronously, got {status:?}"
        );
        Ok(matches!(status, Status::Returned(true)))
    }

    fn sim_motive(&self, m: usize) -> f32 {
        self.world.objects[&self.sim_id]
            .person
            .as_ref()
            .unwrap()
            .motives[m]
    }

    fn sim_skill(&self, s: usize) -> i16 {
        self.world.objects[&self.sim_id]
            .person
            .as_ref()
            .unwrap()
            .data[s]
    }

    fn obj_attr(&self, id: u16, idx: usize) -> i16 {
        self.world.objects[&id].attributes[idx]
    }

    fn obj_data(&self, id: u16, idx: usize) -> i16 {
        self.world.objects[&id].data[idx]
    }
}

// ===========================================================================
// 1. Lexer, Parser & Semantic Span Error Tests
// ===========================================================================

#[test]
fn v2_span_error_diagnostics() {
    let empty_cfg = PackResolverConfig::default();

    // 1a. Invalid hex color
    let err = compile_v2_source(
        "bad_color.antics",
        r#"
object "Bad Color" {
    guid: 0x7F10_0001;
    sprite: Sprite::procedural("cot", primary: #12345, accent: #AABBCC, trim: #112233);
}
"#,
        &empty_cfg,
    )
    .expect_err("5-digit hex color must fail");
    assert!(err.span.is_some(), "expected span on error: {err}");
    assert!(err.message.contains("invalid hex color"), "{err}");

    // 1b. Nested `state { ... }` block rejected explicitly
    let err = compile_v2_source(
        "nested_state.antics",
        r#"
object "Nested State" {
    guid: 0x7F10_0002;
    state {
        occupied: bool = false;
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("state { ... } wrapper must be rejected");
    assert!(err.span.is_some(), "{err}");
    assert!(err.message.contains("flat top-level"), "{err}");

    // 1c. Missing required template parameter
    let err = compile_v2_source(
        "missing_param.antics",
        r#"
template BedBase {
    param energy_step: i16;
    param cycles: i16 = 3;
}
object "Incomplete Cot" from BedBase {
    guid: 0x7F10_0003;
}
"#,
        &empty_cfg,
    )
    .expect_err("missing required param must fail");
    assert!(err.span.is_some(), "{err}");
    assert!(
        err.message
            .contains("missing required parameter 'energy_step'"),
        "{err}"
    );

    // 1d. Unknown parameter assignment
    let err = compile_v2_source(
        "unknown_param.antics",
        r#"
template BedBase {
    param cycles: i16 = 3;
}
object "Bad Param Cot" from BedBase {
    guid: 0x7F10_0004;
    bogus_param: 42;
}
"#,
        &empty_cfg,
    )
    .expect_err("unknown param must fail");
    assert!(err.span.is_some(), "{err}");
    assert!(
        err.message.contains("unknown parameter 'bogus_param'"),
        "{err}"
    );

    // 1e. Duplicate state declaration
    let err = compile_v2_source(
        "dup_state.antics",
        r#"
object "Dup State" {
    guid: 0x7F10_0005;
    state occupied: bool = false;
    state occupied: bool = false;
}
"#,
        &empty_cfg,
    )
    .expect_err("duplicate state must fail");
    assert!(err.span.is_some(), "{err}");
    assert!(err.message.contains("duplicate state"), "{err}");

    // 1f. Context safety: `sim` referenced in `init`
    let err = compile_v2_source(
        "sim_in_init.antics",
        r#"
object "Bad Init" {
    guid: 0x7F10_0006;
    init {
        sim.motive.energy += 10;
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("sim in init must fail");
    assert!(err.span.is_some(), "{err}");
    assert!(err.message.contains("'sim' is not available"), "{err}");

    // 1g. Context safety: `self.motive.energy` (Object vs Sim mismatch)
    let err = compile_v2_source(
        "self_motive.antics",
        r#"
object "Self Motive" {
    guid: 0x7F10_0007;
    interaction "Oops" {
        run(sim) {
            self.motive.energy += 10;
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("self.motive must fail");
    assert!(err.span.is_some(), "{err}");
    assert!(
        err.message.contains("'self' is an Object, not a Sim"),
        "{err}"
    );

    // 1h. Context safety: `sim.occupied` (Sim vs Object state mismatch)
    let err = compile_v2_source(
        "sim_state.antics",
        r#"
object "Sim State" {
    guid: 0x7F10_0008;
    state occupied: bool = false;
    interaction "Oops" {
        guard(sim) => !sim.occupied,
        run(sim) { return true; }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("sim.occupied must fail");
    assert!(err.span.is_some(), "{err}");
    assert!(err.message.contains("use 'self.<state>'"), "{err}");

    // 1i. Private item import rejected
    let mut priv_cfg = PackResolverConfig::default();
    priv_cfg.virtual_modules.insert(
        "demo::helpers".to_owned(),
        r#"
fn secret_helper(sim: Sim) {
    sim.motive.fun += 5;
}
"#
        .to_owned(),
    );
    let err = compile_v2_source(
        "import_priv.antics",
        r#"
use demo::helpers::secret_helper;
object "Try Private" {
    guid: 0x7F10_0009;
    interaction "Run" {
        run(sim) {
            secret_helper(sim);
            return true;
        }
    }
}
"#,
        &priv_cfg,
    )
    .expect_err("importing private fn must fail");
    assert!(err.span.is_some(), "{err}");
    assert!(err.message.contains("is private"), "{err}");

    // 1j. Cyclic `use` import rejected
    let mut cycle_cfg = PackResolverConfig::default();
    cycle_cfg.virtual_modules.insert(
        "demo::a".to_owned(),
        "use demo::b::fb;\npub fn fa(sim: Sim) { fb(sim); }\n".to_owned(),
    );
    cycle_cfg.virtual_modules.insert(
        "demo::b".to_owned(),
        "use demo::a::fa;\npub fn fb(sim: Sim) { fa(sim); }\n".to_owned(),
    );
    let err = compile_v2_source(
        "cycle_use.antics",
        r#"
use demo::a::fa;
object "Cycle" {
    guid: 0x7F10_000A;
    interaction "Go" { run(sim) { fa(sim); return true; } }
}
"#,
        &cycle_cfg,
    )
    .expect_err("cyclic use import must fail");
    assert!(err.message.contains("cyclic module import"), "{err}");

    // 1k. Recursive helper function cycle rejected
    let err = compile_v2_source(
        "rec_fn.antics",
        r#"
fn ping(sim: Sim) { pong(sim); }
fn pong(sim: Sim) { ping(sim); }
object "Recursive" {
    guid: 0x7F10_000B;
    interaction "Go" {
        run(sim) {
            ping(sim);
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("recursive fn cycle must fail");
    assert!(
        err.message.contains("recursive function call cycle"),
        "{err}"
    );

    // 1l. `return` inside `defer` rejected
    let err = compile_v2_source(
        "defer_return.antics",
        r#"
object "Bad Defer" {
    guid: 0x7F10_000C;
    interaction "Go" {
        run(sim) {
            defer {
                return false;
            }
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("return inside defer must fail");
    assert!(
        err.message.contains("cannot exit out of a 'defer'"),
        "{err}"
    );
}

// ===========================================================================
// 2. Semantic Tests: LIFO `defer`, Short-Circuit `&&`/`||`/`!`, `&mut` Helpers,
//    `bool` Params, Extra Template Interactions, Multi-Tile & Multi-Graphic
// ===========================================================================

#[test]
fn lifo_defer_short_circuit_and_mut_ref_helpers_execute_exact_in_vm() {
    let mut cfg = PackResolverConfig::default();
    cfg.virtual_modules.insert(
        "test_pack::util".to_owned(),
        r#"
pub fn bump_and_report(counter: &mut i16, amount: i16, ok: bool) -> bool {
    *counter += amount;
    if !ok {
        return false;
    }
    return true;
}

pub fn mark_done(flag: &mut bool) {
    *flag = true;
}

pub fn clear_done(flag: &mut bool) {
    *flag = false;
}
"#
        .to_owned(),
    );

    let src = r#"
use test_pack::util::{bump_and_report, clear_done, mark_done};

object "Semantics Probe" {
    guid: 0x7F10_0100;
    price: 50;
    description: "Exercises LIFO defer, short-circuit &&/||, and &mut helpers.";
    sprite: Sprite::procedural("box", primary: #6688AA, accent: #99BBDD, trim: #334455);

    param enable_bonus: bool = true;
    param skip_branch: bool = false;

    state occupied: bool = false;
    state trace: i16 = 0;
    state side_effects: i16 = 0;
    state flag: bool = false;

    slot front {
        kind: routing,
        offset: [0.0, -0.75, 0.0],
        standing: true,
        sitting: false,
        ground: true,
        facing: north,
    }

    // LIFO defer test: pushes 1 (defer A), then 2 (defer B) -> executes B (*10+2), then A (*10+1) = 21!
    interaction "Test LIFO Defer" {
        guard(sim) => !self.occupied && enable_bonus && !skip_branch,
        run(sim) {
            with lock(self.occupied) {
                defer {
                    self.trace *= 10;
                    self.trace += 1;
                }
                defer {
                    self.trace *= 10;
                    self.trace += 2;
                }
                if skip_branch {
                    self.trace = 999;
                }
                return true;
            }
        }
    }

    // Short-circuit && / || test:
    // - `false && bump_and_report(&mut self.side_effects, 100, true)` MUST NOT run RHS!
    // - `true || bump_and_report(&mut self.side_effects, 100, true)` MUST NOT run RHS!
    // - `bump_and_report(&mut self.side_effects, 1, false) && bump_and_report(&mut self.side_effects, 50, true)`
    //   runs LHS (+1, returns false) and skips RHS!
    // - `bump_and_report(&mut self.side_effects, 2, true) || bump_and_report(&mut self.side_effects, 50, true)`
    //   runs LHS (+2, returns true) and skips RHS!
    // Total side_effects MUST equal 3!
    interaction "Test Short Circuit" {
        guard(sim) => !self.occupied,
        run(sim) {
            with lock(self.occupied) {
                if bump_and_report(&mut self.side_effects, 1, false)
                    && bump_and_report(&mut self.side_effects, 50, true)
                {
                    self.side_effects += 500;
                }
                if bump_and_report(&mut self.side_effects, 2, true)
                    || bump_and_report(&mut self.side_effects, 50, true)
                {
                    mark_done(&mut self.flag);
                }
            }
            return true;
        }
    }

    interaction "Reset Flag" {
        guard(sim) => !self.occupied && self.flag,
        run(sim) {
            clear_done(&mut self.flag);
            return true;
        }
    }
}
"#;

    let compiled = compile_v2_source("probe.antics", src, &cfg).expect("compile probe");
    let report = verify_object_safety(&compiled).expect("verify probe");
    assert!(report.is_ok(), "safety violations: {:?}", report.violations);

    let mut vm = V2VmFixture::from_objects(&[&compiled], 500);
    let obj_id = vm.spawn(0x7F10_0100, 4, 4);

    vm.run_interaction(obj_id, "Test LIFO Defer", None);
    assert_eq!(
        vm.obj_attr(obj_id, 0),
        0,
        "lock released after early return"
    );
    assert_eq!(
        vm.obj_attr(obj_id, 1),
        21,
        "defers must run in strict LIFO order (2 then 1)"
    );

    vm.run_interaction(obj_id, "Test Short Circuit", None);
    assert_eq!(vm.obj_attr(obj_id, 0), 0, "lock released");
    assert_eq!(
        vm.obj_attr(obj_id, 2),
        3,
        "short-circuit && and || must NOT evaluate skipped RHS"
    );
    assert_eq!(
        vm.obj_attr(obj_id, 3),
        1,
        "mark_done(&mut self.flag) must set flag to true"
    );
    vm.run_interaction(obj_id, "Reset Flag", None);
    assert_eq!(
        vm.obj_attr(obj_id, 3),
        0,
        "clear_done(&mut self.flag) must clear flag back to false"
    );
}

// ===========================================================================
// 3. End-to-End Prototypes: Camp Cot, Retro TV, Floor Lamp, Frost Fridge Chain
//    (Compiled .antics -> .air -> .iff, Verified 0 Violations, Executed in VM)
// ===========================================================================

const COMMON_HELPERS_V2: &str = r#"
pub fn restore_motive_loop(
    sim: Sim,
    motive: Motive,
    cycles: i16,
    ticks: i16,
    step: i16,
    cap: i16,
    completed: &mut i16,
) {
    repeat cycles cancel_on_abort {
        sleep(ticks);
        sim.motive[motive] += step;
        *completed += 1;
        if sim.motive[motive] >= cap {
            break;
        }
    }
}
"#;

const REST_TEMPLATE_V2: &str = r#"
use proto_pack::common::restore_motive_loop;

pub template RestFurniture {
    param nap_cycles: i16 = 3;
    param nap_ticks: i16 = 2;
    param energy_step: i16;
    param comfort_step: i16 = 5;
    param max_energy: i16 = 80;
    param energy_ad: i16 = 60;
    param comfort_ad: i16 = 20;

    state occupied: bool = false;
    state total_cycles: i16 = 0;

    sound settle = preset("soft_thud");

    slot side_entry {
        kind: routing,
        offset: [0.75, 0.0, 0.0],
        standing: true,
        sitting: false,
        ground: true,
        facing: east,
    }

    interaction "Take a Nap" {
        autonomy: 65,
        attenuation: 0.015,
        advertises: [
            energy +energy_ad (min: -100),
            comfort +comfort_ad (min: -100),
        ],
        guard(sim) => !self.occupied && sim.motive.energy < max_energy,
        run(sim) {
            with lock(self.occupied) {
                play_sound(settle, self);
                repeat nap_cycles cancel_on_abort {
                    sleep(nap_ticks);
                    sim.motive.energy += energy_step;
                    sim.motive.comfort += comfort_step;
                    self.total_cycles += 1;
                }
            }
            return true;
        }
    }
}
"#;

const CAMP_COT_V2: &str = r#"
use proto_pack::templates::RestFurniture;

object "Open Camp Cot V2" from RestFurniture {
    guid: 0x7F20_0002;
    price: 210;
    category: seating;
    description: "A v2 clean-room folding camp cot instantiated from RestFurniture.";
    sprite: Sprite::procedural("cot", primary: #587C60, accent: #E4E0D2, trim: #404844);

    energy_step: 20;

    // Extra interaction added directly on top of the template!
    interaction "Quick Power Nap" {
        autonomy: 40,
        attenuation: 0.02,
        advertises: [
            energy +25 (min: -100),
        ],
        guard(sim) => !self.occupied && sim.motive.energy < max_energy,
        run(sim) {
            with lock(self.occupied) {
                play_sound(settle, self);
                sleep(1);
                sim.motive.energy += energy_step;
                self.total_cycles += 1;
            }
            return true;
        }
    }
}
"#;

const RETRO_TV_V2: &str = r#"
use proto_pack::common::restore_motive_loop;

object "Open Retro Television V2" {
    guid: 0x7F20_0052;
    price: 380;
    category: electronics;
    description: "A v2 cabinet television with power state, helper watch loop, and cancel-safe lock.";
    sprite: Sprite::procedural("television", primary: #765438, accent: #364856, trim: #423022);

    param watch_ticks: i16 = 2;
    param max_cycles: i16 = 4;
    param fun_step: i16 = 24;
    param max_entry_fun: i16 = 95;

    state occupied: bool = false;
    state power: bool = false;
    state watch_cycles: i16 = 0;

    sound switch_click = preset("ui_click");
    sound jingle = preset("chime");

    slot viewer {
        kind: routing,
        offset: [0.0, -1.25, 0.0],
        standing: true,
        sitting: true,
        ground: true,
        facing: north,
    }

    interaction "Turn On" {
        autonomy: 30,
        attenuation: 0.02,
        guard(sim) => !self.power,
        run(sim) {
            self.power = true;
            play_sound(switch_click, self);
            return true;
        }
    }

    interaction "Turn Off" {
        autonomy: 20,
        attenuation: 0.02,
        guard(sim) => self.power && !self.occupied,
        run(sim) {
            self.power = false;
            play_sound(switch_click, self);
            return true;
        }
    }

    interaction "Watch TV" {
        autonomy: 60,
        attenuation: 0.02,
        advertises: [
            fun +75 (min: -100),
        ],
        guard(sim) => !self.occupied && sim.motive.fun < max_entry_fun,
        run(sim) {
            with lock(self.occupied) {
                if !self.power {
                    self.power = true;
                    play_sound(switch_click, self);
                }
                play_sound(jingle, self);
                restore_motive_loop(
                    sim,
                    Motive::Fun,
                    max_cycles,
                    watch_ticks,
                    fun_step,
                    100,
                    &mut self.watch_cycles,
                );
            }
            return true;
        }
    }
}
"#;

const FLOOR_LAMP_V2: &str = r#"
object "Open Floor Lamp V2" {
    guid: 0x7F20_0070;
    price: 110;
    category: lighting;
    description: "A v2 floor lamp with multi-graphic off/on states and Room score boost.";
    graphics: [
        Sprite::procedural("floor_lamp", primary: #988C74, accent: #B4A890, trim: #604E34),
        Sprite::procedural("floor_lamp", primary: #F2E0AE, accent: #FCF0CE, trim: #745E3E),
    ];

    param room_boost: i16 = 35;
    param enjoy_ticks: i16 = 1;

    state occupied: bool = false;
    state lit: bool = true;

    sound lamp_switch = preset("ui_click");

    slot front {
        kind: routing,
        offset: [0.0, -0.65, 0.0],
        standing: true,
        sitting: false,
        ground: true,
        facing: north,
    }

    init {
        set_graphic(1);
    }

    interaction "Turn On Light" {
        autonomy: 40,
        attenuation: 0.02,
        advertises: [ room +40 (min: -100) ],
        guard(sim) => !self.lit,
        run(sim) {
            self.lit = true;
            set_graphic(1);
            play_sound(lamp_switch, self);
            sim.motive.room += room_boost;
            return true;
        }
    }

    interaction "Turn Off Light" {
        autonomy: 15,
        attenuation: 0.02,
        guard(sim) => self.lit,
        run(sim) {
            self.lit = false;
            set_graphic(0);
            play_sound(lamp_switch, self);
            return true;
        }
    }

    interaction "Enjoy Warm Light" {
        autonomy: 30,
        attenuation: 0.02,
        advertises: [ room +45 (min: -100) ],
        guard(sim) => !self.occupied && self.lit,
        run(sim) {
            with lock(self.occupied) {
                sleep(enjoy_ticks);
                sim.motive.room += room_boost;
            }
            return true;
        }
    }
}
"#;

const OAK_COUNTER_V2: &str = r#"
object "Open Oak Counter V2" {
    guid: 0x7F20_0021;
    price: 260;
    category: surfaces;
    description: "A v2 kitchen prep counter advertising functional prep.";
    sprite: Sprite::procedural("counter", primary: #A67A4E, accent: #E8E4DA, trim: #583E2A);

    param prep_score: i16 = 50;
    param prep_nutrition_bonus: i16 = 10;

    state meals_prepped: i16 = 0;
    state occupied: bool = false;

    sound chop = preset("wood_creak");

    slot top_surface {
        kind: container,
        offset: [0.0, 0.0, 0.64],
        standing: false,
        sitting: false,
        ground: false,
        facing: north,
    }

    slot front {
        kind: routing,
        offset: [0.0, -0.75, 0.0],
        standing: true,
        sitting: false,
        ground: true,
        facing: north,
    }

    functional prep {
        score: prep_score,
        guard(sim) => !self.occupied,
        run(sim) {
            self.meals_prepped += 1;
            temp[3] = prep_nutrition_bonus;
            play_sound(chop, self);
            return true;
        }
    }
}
"#;

const HEARTH_STOVE_V2: &str = r#"
object "Open Hearth Stove V2" {
    guid: 0x7F20_0022;
    price: 490;
    category: appliances;
    description: "A v2 kitchen range scaling nutrition with Cooking skill and spawning Meal Plate.";
    sprite: Sprite::procedural("stove", primary: #DADEE4, accent: #949CA6, trim: #2E343C);

    param cook_score: i16 = 60;
    param cook_ticks: i16 = 2;
    param base_hunger_value: i16 = 30;
    param skill_divisor: i16 = 20;
    param max_nutrition: i16 = 95;

    state meals_cooked: i16 = 0;
    state occupied: bool = false;
    state force_fail: bool = false;

    sound sizzle = preset("sizzle");

    slot front {
        kind: routing,
        offset: [0.0, -0.75, 0.0],
        standing: true,
        sitting: false,
        ground: true,
        facing: north,
    }

    functional cook(cooking_skill: i16, sim_id: ObjectId, plate_slot: i16, prep_bonus: i16) {
        score: cook_score,
        guard(sim) => !self.occupied,
        run(sim) {
            if self.force_fail {
                return false;
            }
            play_sound(sizzle, self);
            sleep(cook_ticks);

            let mut nutrition = cooking_skill / skill_divisor + base_hunger_value + prep_bonus;
            if nutrition > max_nutrition {
                nutrition = max_nutrition;
            }

            let Some(plate) = create_object(0x7F20_0023, mode: stack_ahead, flags: 0x40) else {
                return false;
            };
            plate.attr[0] = nutrition;
            plate.attr[1] = 1;
            self.meals_cooked += 1;
            temp[2] = plate;
            return true;
        }
    }
}
"#;

const MEAL_PLATE_V2: &str = r#"
object "Open Meal Plate V2" {
    guid: 0x7F20_0023;
    price: 0;
    description: "A v2 cooked meal plate spawned on-lot by the stove.";
    sprite: Sprite::procedural("meal_plate", primary: #F2F0EA, accent: #C48044, trim: #58924C);

    param default_nutrition: i16 = 40;
    param bite_ticks: i16 = 1;
    param bites: i16 = 2;

    state nutrition: i16 = default_nutrition;
    state is_full: bool = true;

    sound eat_sfx = preset("chime");
    sound dispose_sfx = preset("ui_click");

    init {
        self.data[4] = 1;
        self.data[42] = 3;
    }

    slot front {
        kind: routing,
        offset: [0.0, -0.55, 0.0],
        standing: true,
        sitting: true,
        ground: true,
        facing: north,
    }

    interaction "Eat Meal" {
        autonomy: 40,
        attenuation: 0.02,
        advertises: [ hunger +70 (min: -100) ],
        guard(sim) => self.is_full,
        run(sim) {
            let per_bite = self.nutrition / bites;
            let remainder = self.nutrition % bites;
            repeat bites {
                sleep(bite_ticks);
                sim.motive.hunger += per_bite;
            }
            sim.motive.hunger += remainder;
            play_sound(eat_sfx, self);
            self.is_full = false;
            play_sound(dispose_sfx, self);
            destroy_self();
            return true;
        }
    }
}
"#;

const FROST_FRIDGE_V2: &str = r#"
object "Open Frost Fridge V2" {
    guid: 0x7F20_0020;
    price: 550;
    category: appliances;
    description: "A v2 fridge coordinating Counter + Stove with defer rollback and auto stack_obj safety.";
    sprite: Sprite::procedural("fridge", primary: #E4EAEE, accent: #B0BCC6, trim: #5C6670);

    param initial_stock: i16 = 5;
    param meal_cost: i16 = 15;
    param max_entry_hunger: i16 = 90;
    param cooking_xp: i16 = 20;

    state stock: i16 = initial_stock;
    state meals_cooked: i16 = 0;
    state last_plate: ObjectId = 0;

    sound door = preset("soft_thud");

    slot front {
        kind: routing,
        offset: [0.0, -0.75, 0.0],
        standing: true,
        sitting: false,
        ground: true,
        facing: north,
    }

    interaction "Cook Meal" {
        autonomy: 55,
        attenuation: 0.02,
        advertises: [
            hunger +75 (min: -100),
        ],
        guard(sim) => self.stock > 0 && sim.motive.hunger < max_entry_hunger && can_afford(meal_cost),
        run(sim) {
            if self.stock <= 0 {
                return false;
            }
            let Some(counter) = find_best_object(prep) else {
                return false;
            };
            let Some(stove) = find_best_object(cook) else {
                return false;
            };
            if !debit_funds(meal_cost) {
                return false;
            }
            self.stock -= 1;
            play_sound(door, self);

            let mut committed = false;
            defer {
                if !committed {
                    self.stock += 1;
                    credit_funds(meal_cost);
                }
            }

            if !counter.run_functional(prep, sim.skill.cooking, sim.id, 0, 0) {
                return false;
            }
            if !stove.run_functional(cook) {
                return false;
            }

            self.last_plate = temp[2];
            self.meals_cooked += 1;
            sim.skill.cooking += cooking_xp;
            committed = true;
            return true;
        }
    }
}
"#;

#[test]
fn prototypes_camp_cot_retro_tv_floor_lamp_and_frost_fridge_verify_and_run_in_vm() {
    let mut cfg = PackResolverConfig::default();
    cfg.virtual_modules.insert(
        "proto_pack::common".to_owned(),
        COMMON_HELPERS_V2.to_owned(),
    );
    cfg.virtual_modules.insert(
        "proto_pack::templates".to_owned(),
        REST_TEMPLATE_V2.to_owned(),
    );

    let cot = compile_v2_source("open-cot.antics", CAMP_COT_V2, &cfg).expect("compile cot");
    let tv = compile_v2_source("open-tv.antics", RETRO_TV_V2, &cfg).expect("compile tv");
    let lamp =
        compile_v2_source("open-floor-lamp.antics", FLOOR_LAMP_V2, &cfg).expect("compile lamp");
    let counter =
        compile_v2_source("open-counter.antics", OAK_COUNTER_V2, &cfg).expect("compile counter");
    let stove =
        compile_v2_source("open-stove.antics", HEARTH_STOVE_V2, &cfg).expect("compile stove");
    let plate =
        compile_v2_source("open-meal-plate.antics", MEAL_PLATE_V2, &cfg).expect("compile plate");
    let fridge =
        compile_v2_source("open-fridge.antics", FROST_FRIDGE_V2, &cfg).expect("compile fridge");

    // 1. Formal safety verification (0 violations across all 7 compiled v2 objects!)
    for obj in [&cot, &tv, &lamp, &counter, &stove, &plate, &fridge] {
        let rep = verify_object_safety(obj).expect("verify_object_safety");
        assert!(
            rep.is_ok(),
            "v2 object '{}' failed safety verification: {:?}",
            obj.name,
            rep.violations
        );
    }

    // Verify multi-graphic lamp OBJD.num_graphics == 2 and DGRP chunks 100 & 101 exist.
    let lamp_iff = Iff::parse(lamp.iff_bytes.clone()).expect("lamp iff");
    assert!(lamp_iff.get(b"DGRP", 100).is_some());
    assert!(lamp_iff.get(b"DGRP", 101).is_some());
    let objd_data = lamp_iff.data(lamp_iff.get(b"OBJD", 128).expect("lamp OBJD"));
    let num_graphics = u16::from_le_bytes([objd_data[8], objd_data[9]]);
    assert_eq!(num_graphics, 2, "Floor Lamp must declare 2 graphic states");

    // 2. Live `sims-vm` execution for Camp Cot (normal + mid-nap player cancel!)
    let mut vm =
        V2VmFixture::from_objects(&[&cot, &tv, &lamp, &counter, &stove, &plate, &fridge], 1000);
    let cot_id = vm.spawn(0x7F20_0002, 3, 3);
    let tv_id = vm.spawn(0x7F20_0052, 5, 3);
    let lamp_id = vm.spawn(0x7F20_0070, 7, 3);
    let fridge_id = vm.spawn(0x7F20_0020, 3, 6);
    let counter_id = vm.spawn(0x7F20_0021, 5, 6);
    let stove_id = vm.spawn(0x7F20_0022, 7, 6);

    // Camp Cot: full nap restores +60 Energy (+15 Comfort), releases lock.
    let cot_menu = vm.pie_menu_labels(cot_id);
    assert!(cot_menu.contains(&"Take a Nap".to_owned()));
    assert!(cot_menu.contains(&"Quick Power Nap".to_owned()));
    let energy_before = vm.sim_motive(MOTIVE_ENERGY);
    let sounds = vm.run_interaction(cot_id, "Take a Nap", None);
    assert!(!sounds.is_empty(), "Camp Cot should emit settle SFX");
    assert_eq!(vm.obj_attr(cot_id, 0), 0, "Camp Cot lock released");
    assert_eq!(vm.obj_attr(cot_id, 1), 3, "Camp Cot completed 3 cycles");
    assert!(
        vm.sim_motive(MOTIVE_ENERGY) >= energy_before + 59.0,
        "Energy should rise by ~60"
    );

    // Camp Cot: cancel_on_abort mid-nap releases occupancy lock immediately!
    vm.world
        .objects
        .get_mut(&vm.sim_id)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .motives[MOTIVE_ENERGY] = -60.0;
    let cycles_before_cancel = vm.obj_attr(cot_id, 1);
    vm.run_interaction(cot_id, "Take a Nap", Some(1));
    assert_eq!(
        vm.obj_attr(cot_id, 0),
        0,
        "cancel_on_abort must release Camp Cot lock"
    );
    assert_eq!(
        vm.obj_attr(cot_id, 1),
        cycles_before_cancel,
        "cancel_on_abort on first sleep yield must abort before completing cycle"
    );

    // 3. Retro TV: Watch TV auto-turns on power, runs imported helper `restore_motive_loop`, releases lock!
    assert_eq!(vm.obj_attr(tv_id, 1), 0, "TV starts powered off");
    let fun_before = vm.sim_motive(MOTIVE_FUN);
    vm.run_interaction(tv_id, "Watch TV", None);
    assert_eq!(vm.obj_attr(tv_id, 0), 0, "TV viewer lock released");
    assert_eq!(vm.obj_attr(tv_id, 1), 1, "TV power toggled on");
    assert!(
        vm.obj_attr(tv_id, 2) >= 3,
        "TV watch_cycles incremented via &mut"
    );
    assert!(vm.sim_motive(MOTIVE_FUN) > fun_before + 50.0);
    // Now Turn Off is offered in pie menu!
    assert!(vm.pie_menu_labels(tv_id).contains(&"Turn Off".to_owned()));
    vm.run_interaction(tv_id, "Turn Off", None);
    assert_eq!(vm.obj_attr(tv_id, 1), 0, "TV turned off");

    // 4. Floor Lamp: starts lit (graphic 1), Turn Off switches graphic to 0, Turn On restores 1 & boosts Room!
    assert_eq!(vm.obj_attr(lamp_id, 1), 1, "Lamp starts lit");
    assert_eq!(vm.obj_data(lamp_id, 0), 1, "Lamp init set graphic 1");
    vm.run_interaction(lamp_id, "Turn Off Light", None);
    assert_eq!(vm.obj_attr(lamp_id, 1), 0);
    assert_eq!(vm.obj_data(lamp_id, 0), 0, "Turn Off set graphic 0");
    let room_before = vm.sim_motive(MOTIVE_ROOM);
    vm.run_interaction(lamp_id, "Turn On Light", None);
    assert_eq!(vm.obj_attr(lamp_id, 1), 1);
    assert_eq!(vm.obj_data(lamp_id, 0), 1, "Turn On set graphic 1");
    assert!(vm.sim_motive(MOTIVE_ROOM) >= room_before + 34.0);

    // 5. Frost Fridge cooking chain — SUCCESS path!
    assert_eq!(
        vm.obj_attr(fridge_id, 0),
        5,
        "Fridge initialized stock from BCON param"
    );
    let funds_before = vm.world.budget.funds;
    vm.world
        .objects
        .get_mut(&vm.sim_id)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[SKILL_COOKING] = 200;
    vm.run_interaction(fridge_id, "Cook Meal", None);
    assert_eq!(
        vm.world.budget.funds,
        funds_before - 15,
        "Fridge debited $15"
    );
    assert_eq!(
        vm.obj_attr(fridge_id, 0),
        4,
        "Fridge stock decremented to 4"
    );
    assert_eq!(
        vm.obj_attr(fridge_id, 1),
        1,
        "Fridge recorded 1 meal cooked"
    );
    assert_eq!(vm.obj_attr(counter_id, 0), 1, "Counter prepped 1 meal");
    assert_eq!(vm.obj_attr(stove_id, 0), 1, "Stove cooked 1 meal");
    assert_eq!(
        vm.sim_skill(SKILL_COOKING),
        220,
        "Sim gained +20 Cooking XP"
    );
    let plate_id = vm.obj_attr(fridge_id, 2) as u16;
    assert!(plate_id > 0, "Fridge saved spawned Meal Plate ID");
    // Expected nutrition = 200 / 20 + 30 + 10 = 50!
    assert_eq!(
        vm.obj_attr(plate_id, 0),
        50,
        "Skill-scaled + counter-bonus nutrition"
    );

    // Eat the spawned Meal Plate -> restores Hunger and destroys itself!
    let hunger_before = vm.sim_motive(MOTIVE_HUNGER);
    vm.run_interaction(plate_id, "Eat Meal", None);
    assert!(vm.sim_motive(MOTIVE_HUNGER) >= hunger_before + 49.0);
    assert!(
        vm.world.removals.contains(&plate_id),
        "Meal Plate must schedule itself for removal via destroy_self()"
    );

    // 6. Frost Fridge cooking chain — FAILURE + `defer` ROLLBACK path!
    // Force the Stove to fail -> Fridge's `defer { if !committed { self.stock += 1; credit_funds(meal_cost); } }`
    // MUST refund BOTH the $15 AND the pantry stock!
    vm.world.objects.get_mut(&stove_id).unwrap().attributes[2] = 1; // force_fail = true
    let funds_pre_fail = vm.world.budget.funds;
    let stock_pre_fail = vm.obj_attr(fridge_id, 0);
    let meals_pre_fail = vm.obj_attr(fridge_id, 1);
    vm.world
        .objects
        .get_mut(&vm.sim_id)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .motives[MOTIVE_HUNGER] = -50.0;
    vm.run_interaction(fridge_id, "Cook Meal", None);
    assert_eq!(
        vm.world.budget.funds, funds_pre_fail,
        "defer rollback must refund meal_cost on Stove failure"
    );
    assert_eq!(
        vm.obj_attr(fridge_id, 0),
        stock_pre_fail,
        "defer rollback must restore Fridge stock on Stove failure"
    );
    assert_eq!(
        vm.obj_attr(fridge_id, 1),
        meals_pre_fail,
        "meals_cooked must not increment on failed cook"
    );
}

#[test]
fn multi_tile_v2_object_emits_master_and_tile_objds() {
    let src = r#"
object "Open Double Bed V2" {
    guid: 0x7F20_0012;
    price: 680;
    description: "A v2 two-tile double bed.";
    sprite: Sprite::procedural("double_bed", primary: #4C6C92, accent: #ECE8DC, trim: #5E3E2A);
    master_id: 1;
    tiles: [
        tile {
            name: "Open Double Bed V2 (Left Lead)",
            guid: 0x7F20_0010,
            objd_id: 129,
            offset: [0, 0],
            lead: true,
            interaction_group: -1,
            dgrp_id: 101,
            shape: "double_bed_left",
        },
        tile {
            name: "Open Double Bed V2 (Right Slave)",
            guid: 0x7F20_0011,
            objd_id: 130,
            offset: [1, 0],
            lead: false,
            interaction_group: 1,
            dgrp_id: 102,
            shape: "double_bed_right",
        },
    ];

    state occupancy_mask: i16 = 0;

    interaction "Sleep Left" {
        guard(sim) => !(self.occupancy_mask flag_test 1),
        run(sim) {
            self.occupancy_mask flag_set 1;
            for part in self.parts() {
                part.attr[0] = self.occupancy_mask;
            }
            sleep(1);
            self.occupancy_mask flag_clear 1;
            for part in self.parts() {
                part.attr[0] = self.occupancy_mask;
            }
            return true;
        }
    }
}
"#;
    let obj = compile_v2_source("double-bed.antics", src, &PackResolverConfig::default())
        .expect("compile multi-tile v2");
    assert_eq!(obj.tile_guids, vec![0x7F20_0010, 0x7F20_0011]);
    let report = verify_object_safety(&obj).expect("verify multi-tile");
    assert!(report.is_ok(), "violations: {:?}", report.violations);
    let iff = Iff::parse(obj.iff_bytes).expect("parse iff");
    assert!(iff.get(b"OBJD", 128).is_some());
    assert!(iff.get(b"OBJD", 129).is_some());
    assert!(iff.get(b"OBJD", 130).is_some());
    assert!(iff.get(b"DGRP", 101).is_some());
    assert!(iff.get(b"DGRP", 102).is_some());
}

// ===========================================================================
// 4. On-Disk V2 Pack & CLI Tests (`expand`, `check`, `verify`, `equiv`, `build`, `lift`)
// ===========================================================================

#[test]
fn cli_expand_check_verify_equiv_build_and_lift_air_labels() {
    let tmp = CleanupDir::new("cli-v2");
    let pack_dir = tmp.path().join("proto-pack");
    std::fs::create_dir_all(pack_dir.join("objects")).expect("mkdir objects");

    std::fs::write(
        pack_dir.join("pack.toml"),
        r#"
[pack]
id = "proto_pack"
name = "Proto V2 Pack"
version = "0.1.0"
archive = "proto-pack.far"
objects = ["open-cot", "open-tv", "open-floor-lamp"]
"#,
    )
    .expect("write pack.toml");
    std::fs::write(pack_dir.join("common.antics"), COMMON_HELPERS_V2).expect("write common");
    std::fs::write(pack_dir.join("templates.antics"), REST_TEMPLATE_V2).expect("write templates");
    let cot_path = pack_dir.join("objects/open-cot.antics");
    std::fs::write(&cot_path, CAMP_COT_V2).expect("write open-cot.antics");
    std::fs::write(pack_dir.join("objects/open-tv.antics"), RETRO_TV_V2).expect("write open-tv");
    std::fs::write(
        pack_dir.join("objects/open-floor-lamp.antics"),
        FLOOR_LAMP_V2,
    )
    .expect("write open-floor-lamp");

    // Library API: `compile_v2_pack_dir`, `compile_v2_object_file`, `expand_v2_to_air`
    let compiled_pack = compile_v2_pack_dir(&pack_dir).expect("compile_v2_pack_dir");
    assert_eq!(compiled_pack.objects.len(), 3);
    let pack_report = verify_pack_safety(&compiled_pack).expect("verify_pack_safety");
    assert!(pack_report.is_ok(), "violations: {:?}", pack_report.objects);

    let cot_obj =
        compile_v2_object_file(&cot_path, Some(&pack_dir)).expect("compile_v2_object_file");
    let expanded_air = expand_v2_to_air(&cot_path, Some(&pack_dir)).expect("expand_v2_to_air");
    assert!(expanded_air.contains("bcon 4096"), "{expanded_air}");
    assert!(
        expanded_air.contains("my.person.priority == 0"),
        "{expanded_air}"
    );
    assert!(
        expanded_air.contains("stack.attr[0] := 1;"),
        "{expanded_air}"
    );
    assert!(
        expanded_air.contains("stack.attr[0] := 0;"),
        "{expanded_air}"
    );
    // Lowered `.air` must compile cleanly through `compile_air`!
    let air_mod = compile_air(&expanded_air).expect("expanded .air must compile via compile_air");
    assert_eq!(air_mod.behaviors.len(), cot_obj.behaviors.len());

    // 1. CLI `sims-antics expand` is strictly stdout-only (no files created).
    let cwd = tmp.path().join("cwd");
    std::fs::create_dir_all(&cwd).expect("mkdir cwd");
    let before_cwd = list_dir(&cwd);
    let expand_out = Command::new(cli_bin())
        .current_dir(&cwd)
        .args(["expand"])
        .arg(&cot_path)
        .args(["--pack"])
        .arg(&pack_dir)
        .output()
        .expect("run expand");
    assert!(
        expand_out.status.success(),
        "expand stderr: {}",
        String::from_utf8_lossy(&expand_out.stderr)
    );
    assert_eq!(list_dir(&cwd), before_cwd, "expand must write zero files");
    let stdout_air = String::from_utf8(expand_out.stdout).expect("utf8 stdout");
    assert_eq!(stdout_air, expanded_air);

    // 2. CLI `check` & `verify` on both single v2 file and full v2 pack directory.
    let check_file = Command::new(cli_bin())
        .args(["check"])
        .arg(&cot_path)
        .output()
        .expect("run check file");
    assert!(
        check_file.status.success(),
        "check file stderr: {}",
        String::from_utf8_lossy(&check_file.stderr)
    );
    let check_pack = Command::new(cli_bin())
        .args(["check"])
        .arg(&pack_dir)
        .output()
        .expect("run check pack");
    assert!(
        check_pack.status.success(),
        "check pack stderr: {}",
        String::from_utf8_lossy(&check_pack.stderr)
    );

    let verify_file = Command::new(cli_bin())
        .args(["verify"])
        .arg(&cot_path)
        .output()
        .expect("run verify file");
    assert!(
        verify_file.status.success(),
        "verify file stderr: {}",
        String::from_utf8_lossy(&verify_file.stderr)
    );
    let verify_pack = Command::new(cli_bin())
        .args(["verify"])
        .arg(&pack_dir)
        .output()
        .expect("run verify pack");
    assert!(
        verify_pack.status.success(),
        "verify pack stderr: {}",
        String::from_utf8_lossy(&verify_pack.stderr)
    );

    // 3. CLI `build` on v2 pack + `equiv` on v2 `.antics` vs built `.iff` + `lift` `.air` label!
    let out_dir = tmp.path().join("built");
    let build_out = Command::new(cli_bin())
        .args(["build"])
        .arg(&pack_dir)
        .args(["--out"])
        .arg(&out_dir)
        .output()
        .expect("run build");
    assert!(
        build_out.status.success(),
        "build stderr: {}",
        String::from_utf8_lossy(&build_out.stderr)
    );
    let built_cot_iff = out_dir.join("objects/open_cot.iff");
    assert!(built_cot_iff.is_file());

    let equiv_out = Command::new(cli_bin())
        .args(["equiv"])
        .arg(&cot_path)
        .arg(&built_cot_iff)
        .args(["--tree", "4099", "--pack"])
        .arg(&pack_dir)
        .output()
        .expect("run equiv");
    assert!(
        equiv_out.status.success(),
        "equiv stderr: {}",
        String::from_utf8_lossy(&equiv_out.stderr)
    );
    assert!(String::from_utf8_lossy(&equiv_out.stdout).contains("EQUIVALENT"));

    let lift_out = Command::new(cli_bin())
        .args(["lift"])
        .arg(&built_cot_iff)
        .args(["--tree", "4099"])
        .output()
        .expect("run lift");
    assert!(
        lift_out.status.success(),
        "lift stderr: {}",
        String::from_utf8_lossy(&lift_out.stderr)
    );
    let lift_stdout = String::from_utf8_lossy(&lift_out.stdout);
    assert!(
        lift_stdout.contains(".air (Antics IR)"),
        "lift must label output as .air (Antics IR): {lift_stdout}"
    );
}

// ===========================================================================
// 5. Sol Round 1 Regression Tests (Lowering Correctness, Safety & CLI)
// ===========================================================================

#[test]
fn sol_r1_lowering_correctness_in_vm() {
    let mut cfg = PackResolverConfig::default();
    cfg.virtual_modules.insert(
        "r1_pack::mods".to_owned(),
        r#"
fn private_tag(out: &mut i16) {
    *out = 100;
}

pub fn call_module_private(out: &mut i16) {
    private_tag(out);
}

pub fn nested_sleep_helper(sim: Sim, inner_lock: &mut bool, after_sleep: &mut i16) {
    with lock(*inner_lock) {
        repeat 2 {
            sleep(2);
            *after_sleep += 1;
            sim.motive.fun += 1;
        }
    }
}

pub fn bump_and_one(counter: &mut i16, delta: i16) -> i16 {
    *counter += delta;
    return 1;
}

pub fn mutate_handle_and_attr(handle: &mut ObjectId, attr_ref: &mut i16) {
    *handle = 0;
    *attr_ref = 42;
}
"#
        .to_owned(),
    );
    cfg.virtual_modules.insert(
        "r1_pack::tpl".to_owned(),
        r#"
pub template InitBase {
    state init_trace: i16 = 0;
    init {
        let x = 1;
        defer {
            self.init_trace += 10;
        }
    }
}
"#
        .to_owned(),
    );

    let target_src = r#"
object "R1 Target" {
    guid: 0x7F30_0002;
    price: 10;
    description: "Target object for foreign attr & functional header tests.";
    sprite: Sprite::procedural("box", primary: #888888, accent: #AAAAAA, trim: #444444);
    state val: i16 = 5;
    state remaining: i16 = 2;

    functional prep {
        score: 50,
        run(sim) {
            if self.remaining <= 0 {
                return false;
            }
            self.remaining -= 1;
            return true;
        }
    }
}
"#;

    let probe_src = r#"
use r1_pack::mods::{bump_and_one, call_module_private, mutate_handle_and_attr, nested_sleep_helper};
use r1_pack::tpl::InitBase;

object "R1 Probe" from InitBase {
    guid: 0x7F30_0001;
    price: 100;
    description: "Exercises Sol R1 lowering fixes in live sims-vm.";
    sprite: Sprite::procedural("box", primary: #446688, accent: #6688AA, trim: #223344);

    // Local helper with the SAME name as `r1_pack::mods::private_tag` (P1-8)!
    fn private_tag(out: &mut i16) {
        *out = 999;
    }

    state outer_lock: bool = false;   // attr[1]
    state inner_lock: bool = false;   // attr[2]
    state after_sleep: i16 = 0;       // attr[3]
    state while_count: i16 = 0;       // attr[4]
    state src_val: i16 = 77;          // attr[5]
    state arith_res: i16 = 0;         // attr[6]
    state mod_tag: i16 = 0;           // attr[7]
    state defer_ran: i16 = 0;         // attr[8]
    state func_loops: i16 = 0;        // attr[9]
    state counter: i16 = 10;          // attr[10]

    // Mixed SLOT table: container first, then two routing slots (P1-9).
    slot top {
        kind: container,
        offset: [0.0, 0.0, 0.5],
        standing: false,
        sitting: false,
        ground: false,
        facing: north,
    }
    slot route_a {
        kind: routing,
        offset: [0.0, -0.75, 0.0],
        standing: true,
        sitting: false,
        ground: true,
        facing: north,
    }
    slot route_b {
        kind: routing,
        offset: [0.75, 0.0, 0.0],
        standing: true,
        sitting: false,
        ground: true,
        facing: east,
    }

    // Object init has its own `let x` and top-level `defer` (P1-6).
    init {
        let x = 2;
        defer {
            self.init_trace += 1;
        }
    }

    // P1-1: computed while condition `n + 1 < 4` must re-evaluate every iteration (runs 3 times).
    // P1-2: foreign write `target.attr[0] = self.src_val` must snapshot self.src_val BEFORE swapping stack_obj!
    // P1-3: `while target.run_functional(prep)` must preserve stack_obj for `self.func_loops += 1`!
    // P1-7: `self.counter + bump_and_one(&mut self.counter, 5)` must read LHS (10) before RHS mutates counter to 15!
    // P1-8: `call_module_private(&mut self.mod_tag)` must call module's private_tag (100), not local (999)!
    // P2-1: `mutate_handle_and_attr(&mut alias, &mut alias.attr[0])` must write to original target even after `*handle = 0`!
    interaction "Run Core Probes" {
        run(sim) {
            let mut n = 0;
            while n + 1 < 4 {
                n += 1;
                self.while_count += 1;
            }

            let Some(target) = find_best_object(prep) else {
                return false;
            };
            target.attr[0] = self.src_val;
            while target.run_functional(prep) {
                self.func_loops += 1;
            }

            self.arith_res = self.counter + bump_and_one(&mut self.counter, 5);
            call_module_private(&mut self.mod_tag);

            // Re-test P2-1 using a separate local handle that gets zeroed inside helper.
            let mut alias: ObjectId = target;
            mutate_handle_and_attr(&mut alias, &mut alias.attr[1]);
            return true;
        }
    }

    // P1-4 & P1-5: cancel_on_abort inside nested helper + sleeping defer!
    interaction "Cancel Nested Helper" {
        guard(sim) => !self.outer_lock && !self.inner_lock,
        run(sim) {
            repeat 3 cancel_on_abort {
                with lock(self.outer_lock) {
                    defer {
                        sleep(1);
                        self.defer_ran += 1;
                    }
                    nested_sleep_helper(sim, &mut self.inner_lock, &mut self.after_sleep);
                }
            }
            return true;
        }
    }

    // P1-9 check via expanded `.air`: `route_to(route_b)` must emit routing ordinal 1 (not slot index 2).
    interaction "Route B" {
        run(sim) {
            route_to(route_b);
            return true;
        }
    }
}
"#;

    let target = compile_v2_source("r1-target.antics", target_src, &cfg).expect("compile target");
    let probe = compile_v2_source("r1-probe.antics", probe_src, &cfg).expect("compile probe");
    // Verify P1-9 routing ordinal in lowered `.air`
    let probe_air =
        sims_antics::v2::expand_v2_source_to_objects("r1-probe.antics", probe_src, &cfg)
            .expect("expand probe")
            .into_iter()
            .next()
            .unwrap()
            .air_source;
    assert!(
        probe_air.contains("route_to(1)"),
        "route_b must lower to routing-only ordinal 1:\n{probe_air}"
    );
    assert!(
        !probe_air.contains("route_to(2)"),
        "container slot must not shift routing ordinal:\n{probe_air}"
    );
    let rep = verify_object_safety(&probe).expect("verify probe");
    assert!(rep.is_ok(), "safety violations: {:?}", rep.violations);

    let mut vm = V2VmFixture::from_objects(&[&target, &probe], 500);
    let target_id = vm.spawn(0x7F30_0002, 3, 3);
    let probe_id = vm.spawn(0x7F30_0001, 5, 3);

    // P1-6: template init defer (+10) + object init defer (+1) = 11!
    assert_eq!(
        vm.obj_attr(probe_id, 0),
        11,
        "init top-level defers must run for both template and object"
    );

    // Run Core Probes
    vm.run_interaction(probe_id, "Run Core Probes", None);
    assert_eq!(
        vm.obj_attr(probe_id, 4),
        3,
        "P1-1: while n + 1 < 4 must execute exactly 3 iterations"
    );
    assert_eq!(
        vm.obj_attr(target_id, 0),
        77,
        "P1-2: foreign write must copy self.src_val (77), not target's own attr[5]"
    );
    assert_eq!(
        vm.obj_attr(probe_id, 9),
        2,
        "P1-3: while target.run_functional(prep) must preserve stack_obj for self.func_loops"
    );
    assert_eq!(
        vm.obj_attr(probe_id, 6),
        11,
        "P1-7: self.counter (10) + bump_and_true (1) must snapshot LHS before RHS mutation"
    );
    assert_eq!(
        vm.obj_attr(probe_id, 10),
        15,
        "P1-7: counter mutated to 15 by RHS"
    );
    assert_eq!(
        vm.obj_attr(probe_id, 7),
        100,
        "P1-8: imported pub fn must resolve its own module's private_tag (100), not object-local (999)"
    );
    assert_eq!(
        vm.obj_attr(target_id, 1),
        42,
        "P2-1: &mut foreign.attr[1] must stay bound to original target after handle reassignment"
    );

    // Cancel Nested Helper on yield 1 (inside `nested_sleep_helper` -> `repeat 2` -> `sleep(2)`):
    vm.run_interaction(probe_id, "Cancel Nested Helper", Some(1));
    assert_eq!(
        vm.obj_attr(probe_id, 1),
        0,
        "P1-4: outer_lock must be released on cancel inside helper"
    );
    assert_eq!(
        vm.obj_attr(probe_id, 2),
        0,
        "P1-4: inner_lock inside helper must be released on cancel"
    );
    assert_eq!(
        vm.obj_attr(probe_id, 3),
        0,
        "P1-4: code after sleep inside helper must not run after cancel"
    );
    assert_eq!(
        vm.obj_attr(probe_id, 8),
        1,
        "P1-5: sleeping defer must run once on cancel without compiler stack overflow"
    );
}

#[test]
fn sol_r1_safety_validation_and_cli_regressions() {
    let empty_cfg = PackResolverConfig::default();

    // P1-9: container slot rejected in route_to
    let err = compile_v2_source(
        "bad_slot.antics",
        r#"
object "Bad Slot" {
    guid: 0x7F31_0001;
    slot surface {
        kind: container,
        offset: [0.0, 0.0, 0.5],
        standing: false,
        sitting: false,
        ground: false,
        facing: north,
    }
    interaction "Go" { run(sim) { route_to(surface); return true; } }
}
"#,
        &empty_cfg,
    )
    .expect_err("container slot must be rejected in route_to");
    assert!(err.message.contains("not a routing slot"), "{err}");

    // P2-2: bool assigned to ObjectId / i16 indexed as foreign object / foreign attr in with lock
    let err = compile_v2_source(
        "bad_oid.antics",
        r#"
object "Bad Oid" {
    guid: 0x7F31_0002;
    interaction "Go" { run(sim) { let x: ObjectId = true; return true; } }
}
"#,
        &empty_cfg,
    )
    .expect_err("bool->ObjectId must be rejected");
    assert!(err.message.contains("from bool expression"), "{err}");

    let err = compile_v2_source(
        "bad_idx.antics",
        r#"
object "Bad Idx" {
    guid: 0x7F31_0003;
    interaction "Go" { run(sim) { let n: i16 = 5; n.attr[0] = 1; return true; } }
}
"#,
        &empty_cfg,
    )
    .expect_err("indexing i16 as foreign object must be rejected");
    assert!(err.message.contains("expected ObjectId"), "{err}");

    let err = compile_v2_source(
        "bad_lock.antics",
        r#"
object "Bad Lock" {
    guid: 0x7F31_0004;
    interaction "Go" {
        run(sim) {
            let Some(other) = find_best_object(prep) else { return false; };
            with lock(other.attr[0]) { sleep(1); }
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("with lock on foreign i16 attr must be rejected");
    assert!(err.message.contains("bool state"), "{err}");

    // P2-3: missing return in `-> bool` helper
    let err = compile_v2_source(
        "missing_ret.antics",
        r#"
fn maybe_ok(flag: bool) -> bool {
    if flag {
        return true;
    }
}
object "Missing Ret" {
    guid: 0x7F31_0005;
    interaction "Go" { run(sim) { return maybe_ok(true); } }
}
"#,
        &empty_cfg,
    )
    .expect_err("missing return in -> bool helper must be rejected");
    assert!(
        err.message
            .contains("does not return on all control-flow paths"),
        "{err}"
    );

    // P2-4: `self.parts()` inside `init` uses `my.id` (not `stack_obj`)
    let init_parts_src = r#"
object "Init Parts" {
    guid: 0x7F31_0006;
    state x: i16 = 0;
    init {
        for p in self.parts() {
            p.attr[0] = 1;
        }
    }
}
"#;
    let init_parts_air = sims_antics::v2::expand_v2_source_to_objects(
        "init_parts.antics",
        init_parts_src,
        &empty_cfg,
    )
    .expect("self.parts() in init must expand")
    .into_iter()
    .next()
    .unwrap()
    .air_source;
    assert!(init_parts_air.contains("my.id"), "{init_parts_air}");
    compile_v2_source("init_parts.antics", init_parts_src, &empty_cfg)
        .expect("self.parts() in init must compile");

    // P2-5: re-entrant `with lock` rejected
    let err = compile_v2_source(
        "reentrant_lock.antics",
        r#"
object "Reentrant Lock" {
    guid: 0x7F31_0007;
    state occupied: bool = false;
    interaction "Go" {
        run(sim) {
            with lock(self.occupied) {
                with lock(self.occupied) {
                    sleep(1);
                }
            }
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("re-entrant with lock must be rejected");
    assert!(err.message.contains("re-entrant"), "{err}");

    // P2-6: `raw` control-flow escape under `with lock` and `raw` inside `defer` rejected
    let err = compile_v2_source(
        "raw_escape.antics",
        r#"
object "Raw Escape" {
    guid: 0x7F31_0008;
    state occupied: bool = false;
    interaction "Go" {
        run(sim) {
            with lock(self.occupied) {
                raw { return true; }
            }
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("raw return under with lock must be rejected");
    assert!(err.message.contains("cannot contain 'return'"), "{err}");

    let err = compile_v2_source(
        "raw_defer.antics",
        r#"
object "Raw Defer" {
    guid: 0x7F31_0009;
    interaction "Go" {
        run(sim) {
            defer {
                raw { my.attr[0] := 1; }
            }
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("raw inside defer must be rejected");
    assert!(
        err.message.contains("not allowed inside a 'defer'"),
        "{err}"
    );

    // P2-7: `raw` block with `#` and `/* } */` comments containing unmatched braces compiles cleanly!
    compile_v2_source(
        "raw_comments.antics",
        r#"
object "Raw Comments" {
    guid: 0x7F31_000A;
    state a: i16 = 0;
    init {
        raw {
            # comment with } brace
            /* block comment with { and } */
            my.attr[0] := 7;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect("raw block with .air comments containing braces must compile");

    // P2-8: deeply nested expression rejected cleanly without stack overflow
    let deep_parens = format!(
        "object \"Deep\" {{ guid: 0x7F31_000B; interaction \"G\" {{ run(sim) {{ let x = {}1{}; return true; }} }} }}",
        "(".repeat(120),
        ")".repeat(120)
    );
    let err = compile_v2_source("deep.antics", &deep_parens, &empty_cfg)
        .expect_err("excessive nesting must fail cleanly");
    assert!(err.message.contains("nesting depth"), "{err}");

    // P2-11: multi-graphic + multi-tile implicit DGRP IDs must not collide
    let multi_gfx_tile = compile_v2_source(
        "multi_gfx_tile.antics",
        r#"
object "Multi Gfx Tile" {
    guid: 0x7F31_000C;
    master_id: 1;
    graphics: [
        Sprite::procedural("floor_lamp", primary: #988C74, accent: #B4A890, trim: #604E34),
        Sprite::procedural("floor_lamp", primary: #F2E0AE, accent: #FCF0CE, trim: #745E3E),
    ];
    tiles: [
        tile { name: "Left", guid: 0x7F31_000D, objd_id: 129, offset: [0, 0], lead: true, interaction_group: -1, shape: "cot" },
        tile { name: "Right", guid: 0x7F31_000E, objd_id: 130, offset: [1, 0], lead: false, interaction_group: 1, shape: "cot" },
    ];
}
"#,
        &empty_cfg,
    )
    .expect("multi-graphic + multi-tile must compile without DGRP collision");
    let iff = Iff::parse(multi_gfx_tile.iff_bytes).expect("parse iff");
    assert!(iff.get(b"DGRP", 100).is_some());
    assert!(iff.get(b"DGRP", 101).is_some());
    assert!(iff.get(b"DGRP", 102).is_some());
    assert!(iff.get(b"DGRP", 103).is_some());

    // P2-12: invalid sound WAV filename rejected
    let err = compile_v2_source(
        "bad_wav.antics",
        r#"
object "Bad Wav" {
    guid: 0x7F31_000F;
    sound evil = preset("ui_click", wav: "../escape");
}
"#,
        &empty_cfg,
    )
    .expect_err("path traversal in sound wav must fail");
    assert!(err.message.contains("sound wav name"), "{err}");

    // P3-1: control character inside string literal escapes cleanly for `.air`
    compile_v2_source(
        "ctrl_str.antics",
        "object \"Ctrl\" {\n    guid: 0x7F31_0010;\n    strings 300 {\n        msg = \"hi\x07there\",\n    }\n}\n",
        &empty_cfg,
    )
    .expect("control char in string must escape cleanly");

    // P1-10: legacy `object.toml` referencing `scripts = ["object.antics"]` still takes legacy path!
    let tmp = CleanupDir::new("r1-cli");
    let legacy_pack = tmp.path().join("legacy-pack");
    std::fs::create_dir_all(legacy_pack.join("objects/demo")).expect("mkdir legacy");
    std::fs::write(
        legacy_pack.join("pack.toml"),
        "[pack]\nid = \"legacy\"\nname = \"Legacy\"\nversion = \"0.1.0\"\narchive = \"legacy.far\"\nobjects = [\"demo\"]\n",
    )
    .expect("write legacy pack.toml");
    std::fs::write(
        legacy_pack.join("objects/demo/object.toml"),
        "[object]\nname = \"Legacy Demo\"\nguid = 0x7F31_0020\nscripts = [\"object.antics\"]\ninit = 4096\nmain = 4097\n",
    )
    .expect("write object.toml");
    std::fs::write(
        legacy_pack.join("objects/demo/object.antics"),
        "tree 4096 \"Init\" { return true; }\ntree 4097 \"Main\" { return true; }\n",
    )
    .expect("write legacy .air script named object.antics");
    let compiled_legacy = sims_antics::compile_pack_dir(&legacy_pack)
        .expect("legacy object.toml must take precedence over object.antics");
    assert_eq!(compiled_legacy.objects.len(), 1);

    // P2-10: bare filename `sims-antics expand demo.antics` from inside its directory succeeds!
    let bare_dir = tmp.path().join("bare");
    std::fs::create_dir_all(&bare_dir).expect("mkdir bare");
    std::fs::write(
        bare_dir.join("demo.antics"),
        "object \"Bare Demo\" { guid: 0x7F31_0021; }\n",
    )
    .expect("write bare demo");
    let bare_out = Command::new(cli_bin())
        .current_dir(&bare_dir)
        .args(["expand", "demo.antics"])
        .output()
        .expect("run expand bare");
    assert!(
        bare_out.status.success(),
        "bare filename expand failed: {}",
        String::from_utf8_lossy(&bare_out.stderr)
    );

    // P3-2: `sims-antics lift --raw` includes `.air (Antics IR)` header!
    let raw_iff_path = tmp.path().join("legacy_demo.iff");
    std::fs::write(&raw_iff_path, &compiled_legacy.objects[0].object.iff_bytes).expect("write iff");
    let lift_raw = Command::new(cli_bin())
        .args(["lift"])
        .arg(&raw_iff_path)
        .args(["--tree", "4096", "--raw"])
        .output()
        .expect("run lift --raw");
    assert!(lift_raw.status.success());
    let lift_raw_out = String::from_utf8_lossy(&lift_raw.stdout);
    assert!(
        lift_raw_out.contains(".air (Antics IR)"),
        "lift --raw must identify output as .air (Antics IR): {lift_raw_out}"
    );

    // P1-11: absolute dependency path and symlinked ancestor directory rejected!
    let dep_pack = tmp.path().join("dep-pack");
    std::fs::create_dir_all(dep_pack.join("objects")).expect("mkdir dep-pack");
    std::fs::write(
        dep_pack.join("pack.toml"),
        format!(
            "[pack]\nid = \"dep_pack\"\nname = \"Dep\"\nversion = \"0.1.0\"\narchive = \"dep.far\"\nobjects = [\"o\"]\n\n[dependencies]\nevil = \"{}\"\n",
            tmp.path().display()
        ),
    )
    .expect("write dep pack.toml");
    std::fs::write(
        dep_pack.join("objects/o.antics"),
        "object \"O\" { guid: 0x7F31_0022; }\n",
    )
    .expect("write o.antics");
    let err = compile_v2_pack_dir(&dep_pack).expect_err("absolute dependency path must fail");
    assert!(err.message.contains("must be a relative path"), "{err}");

    #[cfg(unix)]
    {
        let sym_pack = tmp.path().join("sym-pack");
        std::fs::create_dir_all(sym_pack.join("real_mod")).expect("mkdir real_mod");
        std::fs::write(sym_pack.join("real_mod/h.antics"), "pub fn noop() {}\n")
            .expect("write h.antics");
        std::os::unix::fs::symlink(sym_pack.join("real_mod"), sym_pack.join("link_mod"))
            .expect("symlink");
        std::fs::write(
            sym_pack.join("pack.toml"),
            "[pack]\nid = \"sym_pack\"\nname = \"Sym\"\nversion = \"0.1.0\"\narchive = \"sym.far\"\nobjects = []\n",
        )
        .expect("write sym pack.toml");
        std::fs::write(
            sym_pack.join("main.antics"),
            "use sym_pack::link_mod::h::noop;\nobject \"S\" { guid: 0x7F31_0023; }\n",
        )
        .expect("write main.antics");
        let err = compile_v2_object_file(&sym_pack.join("main.antics"), Some(&sym_pack))
            .expect_err("symlinked ancestor directory must be rejected");
        assert!(err.message.contains("symlink"), "{err}");
    }
}

#[test]
fn sol_r2_regressions() {
    let empty_cfg = PackResolverConfig::default();

    // R2 P1-1: void helper containing `break` called inside caller's `repeat ... cancel_on_abort` must be rejected!
    let err = compile_v2_source(
        "r2_p1_1.antics",
        r#"
fn stop() {
    break;
}
object "Break Escape" {
    guid: 0x7F32_0001;
    state count: i16 = 0;
    interaction "Go" {
        run(sim) {
            repeat 2 cancel_on_abort {
                defer { self.count += 1; }
                stop();
            }
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("break inside helper with no helper-local loop must fail");
    assert!(err.message.contains("'break' outside of a loop"), "{err}");

    // R2 P1-2: re-entrant `with lock(self.busy)` inside `defer` registered under `with lock(self.busy)` must fail!
    let err = compile_v2_source(
        "r2_p1_2.antics",
        r#"
object "Defer Relock" {
    guid: 0x7F32_0002;
    state busy: bool = false;
    interaction "Go" {
        run(sim) {
            with lock(self.busy) {
                defer {
                    with lock(self.busy) {}
                    sleep(1);
                }
            }
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("re-entrant lock inside defer while outer lock is still held must fail");
    assert!(err.message.contains("re-entrant"), "{err}");

    // Non-overlapping `defer { with lock(self.busy) {} }` registered OUTSIDE `with lock(self.busy)` is allowed!
    compile_v2_source(
        "r2_p1_2_ok.antics",
        r#"
object "Defer Relock Ok" {
    guid: 0x7F32_0003;
    state busy: bool = false;
    interaction "Go" {
        run(sim) {
            defer {
                with lock(self.busy) {}
            }
            with lock(self.busy) {
                sleep(1);
            }
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect("defer outside lock runs after unlock and may reacquire lock");

    // R2 P1-3: `raw` block with explicit branch targets (`-> true: true, false: true`) or unbranched `call` under `with lock` must fail!
    let err = compile_v2_source(
        "r2_p1_3_branch.antics",
        r#"
object "Raw Branch" {
    guid: 0x7F32_0004;
    state busy: bool = false;
    interaction "Go" {
        run(sim) {
            with lock(self.busy) {
                raw { random(temp[0], 1) -> true: true, false: true; }
            }
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("raw branch exiting tree under with lock must fail");
    assert!(err.message.contains("cannot branch"), "{err}");

    let err = compile_v2_source(
        "r2_p1_3_call.antics",
        r#"
object "Raw Call" {
    guid: 0x7F32_0005;
    state busy: bool = false;
    interaction "Go" {
        run(sim) {
            with lock(self.busy) {
                raw { call 4096(); }
            }
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect_err("raw unbranched call under with lock must fail");
    assert!(err.message.contains("unbranched subroutine call"), "{err}");

    // R2 P1-4: same-module `fn tag()` + `fn bridge() { tag(); }` + object-local `fn tag() { bridge(); }` must NOT report false recursion!
    let same_mod_obj = compile_v2_source(
        "r2_p1_4.antics",
        r#"
fn tag(out: &mut i16) {
    *out = 55;
}
fn bridge(out: &mut i16) {
    tag(out);
}
object "Same Mod Non Recursive" {
    guid: 0x7F32_0006;
    state res: i16 = 0;
    fn tag(out: &mut i16) {
        bridge(out);
    }
    interaction "Go" {
        run(sim) {
            tag(&mut self.res);
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect("distinct local::tag -> mod::bridge -> mod::tag must not be flagged recursive");
    let mut vm = V2VmFixture::from_objects(&[&same_mod_obj], 100);
    let oid = vm.spawn(0x7F32_0006, 3, 3);
    vm.run_interaction(oid, "Go", None);
    assert_eq!(vm.obj_attr(oid, 0), 55);

    // R2 P2-1: unbounded postfix chain rejected cleanly!
    let deep_postfix = format!(
        "object \"Deep Postfix\" {{ guid: 0x7F32_0007; interaction \"G\" {{ run(sim) {{ let x = self{}; return true; }} }} }}",
        ".a".repeat(120)
    );
    let err = compile_v2_source("deep_postfix.antics", &deep_postfix, &empty_cfg)
        .expect_err("deep postfix chain must be rejected");
    assert!(err.message.contains("nesting depth"), "{err}");

    // R2 P2-2: valid uses of `raw` as identifier (`while raw`, `let mut raw`, `template raw`)!
    compile_v2_source(
        "r2_p2_2.antics",
        r#"
template raw {
    state ok: bool = false;
}
object "Raw Ident" from raw {
    guid: 0x7F32_0008;
    interaction "Go" {
        run(sim) {
            let mut raw = true;
            while raw {
                raw = false;
                break;
            }
            return true;
        }
    }
}
"#,
        &empty_cfg,
    )
    .expect("using 'raw' as variable and template identifier must succeed");

    // R2 P2-3: `Object` and `ObjectId` helper params reject plain `i16` locals!
    let err = compile_v2_source(
        "r2_p2_3_obj.antics",
        r#"
fn write_obj(o: Object) { o.attr[0] = 1; }
object "Bad Obj Arg" {
    guid: 0x7F32_0009;
    interaction "Go" { run(sim) { let n: i16 = 5; write_obj(n); return true; } }
}
"#,
        &empty_cfg,
    )
    .expect_err("i16 must not be accepted as Object parameter");
    assert!(err.message.contains("expects 'self' or ObjectId"), "{err}");

    let err = compile_v2_source(
        "r2_p2_3_oid.antics",
        r#"
fn accept_id(id: ObjectId) { let x = id; }
object "Bad Oid Arg" {
    guid: 0x7F32_000A;
    interaction "Go" { run(sim) { let n: i16 = 5; accept_id(n); return true; } }
}
"#,
        &empty_cfg,
    )
    .expect_err("i16 must not be accepted as ObjectId parameter");
    assert!(err.message.contains("expects ObjectId"), "{err}");

    // R2 P2-5: implicit tile DGRP ID skips explicit tile DGRP ID `103` without colliding!
    let mixed_dgrp = compile_v2_source(
        "r2_p2_5.antics",
        r#"
object "Mixed Tile DGRP" {
    guid: 0x7F32_000B;
    master_id: 1;
    graphics: [
        Sprite::procedural("floor_lamp", primary: #988C74, accent: #B4A890, trim: #604E34),
        Sprite::procedural("floor_lamp", primary: #F2E0AE, accent: #FCF0CE, trim: #745E3E),
    ];
    tiles: [
        tile { name: "Explicit", guid: 0x7F32_000C, objd_id: 129, offset: [0, 0], lead: true, interaction_group: -1, dgrp_id: 103, shape: "cot" },
        tile { name: "Implicit", guid: 0x7F32_000D, objd_id: 130, offset: [1, 0], lead: false, interaction_group: 1, shape: "cot" },
    ];
}
"#,
        &empty_cfg,
    )
    .expect("implicit tile dgrp_id must skip reserved explicit id 103");
    let iff = Iff::parse(mixed_dgrp.iff_bytes).expect("parse iff");
    assert!(iff.get(b"DGRP", 100).is_some());
    assert!(iff.get(b"DGRP", 101).is_some());
    assert!(iff.get(b"DGRP", 102).is_some());
    assert!(iff.get(b"DGRP", 103).is_some());

    // R2 P2-4 & R2 P1-5: CLI `check` & `equiv` with `--pack` on `.air` input, plus symlinked alias to pack root!
    let tmp = CleanupDir::new("r2-cli");
    let pack = tmp.path().join("pack");
    std::fs::create_dir_all(pack.join("sub")).expect("mkdir sub");
    std::fs::write(
        pack.join("pack.toml"),
        "[pack]\nid = \"r2_pack\"\nname = \"R2\"\nversion = \"0.1.0\"\narchive = \"r2.far\"\nobjects = []\n",
    )
    .expect("write pack.toml");
    let air_file = pack.join("sub/raw.air");
    std::fs::write(&air_file, "tree 4096 \"Init\" { return true; }\n").expect("write raw.air");
    let v2_file = pack.join("sub/demo.antics");
    std::fs::write(&v2_file, "object \"Demo\" { guid: 0x7F32_0010; }\n").expect("write demo");
    let demo_obj = compile_v2_object_file(&v2_file, Some(&pack)).expect("compile demo");
    let demo_iff = tmp.path().join("demo.iff");
    std::fs::write(&demo_iff, &demo_obj.iff_bytes).expect("write demo.iff");

    // R2 P2-4: `check` and `equiv` with `--pack` still support `.air` files!
    let check_air = Command::new(cli_bin())
        .args(["check"])
        .arg(&air_file)
        .args(["--pack"])
        .arg(&pack)
        .output()
        .expect("check air with --pack");
    assert!(
        check_air.status.success(),
        "check .air --pack failed: {}",
        String::from_utf8_lossy(&check_air.stderr)
    );
    let equiv_air = Command::new(cli_bin())
        .args(["equiv"])
        .arg(&air_file)
        .arg(&demo_iff)
        .args(["--tree", "4096", "--pack"])
        .arg(&pack)
        .output()
        .expect("equiv air with --pack");
    assert!(
        equiv_air.status.success(),
        "equiv .air --pack failed: {}",
        String::from_utf8_lossy(&equiv_air.stderr)
    );

    // R2 P1-5: alternate source path through symlink alias `/tmp/alias -> /tmp/pack` must be rejected!
    #[cfg(unix)]
    {
        let alias = tmp.path().join("alias");
        std::os::unix::fs::symlink(&pack, &alias).expect("symlink alias -> pack");
        let aliased_src = alias.join("sub/demo.antics");
        let err = compile_v2_object_file(&aliased_src, Some(&pack))
            .expect_err("aliased source path through symlinked pack root must be rejected");
        assert!(err.message.contains("symlink"), "{err}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_system_symlink_paths_work_while_user_symlinks_remain_rejected() {
    struct TmpCleanup(PathBuf);
    impl Drop for TmpCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    // Explicitly use uncanonicalized `/tmp/...` (where `/tmp -> /private/tmp` on macOS).
    let root = PathBuf::from(format!(
        "/tmp/sims-antics-macos-syslink-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pack/objects")).expect("mkdir /tmp pack");
    let _guard = TmpCleanup(root.clone());

    let pack_dir = root.join("pack");
    std::fs::write(
        pack_dir.join("pack.toml"),
        "[pack]\nid = \"mac_pack\"\nname = \"Mac Pack\"\nversion = \"0.1.0\"\narchive = \"mac.far\"\nobjects = [\"demo\"]\n",
    )
    .expect("write pack.toml");
    let obj_path = pack_dir.join("objects/demo.antics");
    std::fs::write(
        &obj_path,
        "object \"Mac Demo\" {\n    guid: 0x7F33_0001;\n    state busy: bool = false;\n    interaction \"Ping\" { guard(sim) => !self.busy, run(sim) { with lock(self.busy) { sleep(1); } return true; } }\n}\n",
    )
    .expect("write demo.antics");

    for subcmd in ["check", "expand", "verify"] {
        let out = Command::new(cli_bin())
            .args([subcmd])
            .arg(&obj_path)
            .output()
            .expect("run cli subcommand");
        assert!(
            out.status.success(),
            "sims-antics {subcmd} failed on /tmp path: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let out_dir = root.join("built");
    let build_out = Command::new(cli_bin())
        .args(["build"])
        .arg(&pack_dir)
        .args(["--out"])
        .arg(&out_dir)
        .output()
        .expect("run cli build");
    assert!(
        build_out.status.success(),
        "sims-antics build failed on /tmp path: {}",
        String::from_utf8_lossy(&build_out.stderr)
    );

    // Real user symlink under `/tmp/...` must STILL be rejected!
    let alias = root.join("alias");
    std::os::unix::fs::symlink(&pack_dir, &alias).expect("symlink alias");
    let aliased_file = alias.join("objects/demo.antics");
    let err = compile_v2_object_file(&aliased_file, Some(&pack_dir))
        .expect_err("user symlink under /tmp must still be rejected");
    assert!(err.message.contains("symlink"), "{err}");
}

// ===========================================================================
// 6. On-Disk `open-pack/` v2 Migration, Zero-`object.toml`, & 20/20 Expand Gate
// ===========================================================================

#[test]
fn on_disk_open_pack_is_pure_v2_expands_all_20_objects_and_passes_safety() {
    let pack_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("open-pack");

    // 1. Verify ZERO `object.toml` files remain anywhere under `open-pack/`.
    #[allow(clippy::disallowed_methods)]
    fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("read_dir")
            .map(|e| e.expect("entry").path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk_files(&p, out);
            } else if p.is_file() {
                out.push(p);
            }
        }
    }
    let mut all_files = Vec::new();
    walk_files(&pack_dir, &mut all_files);
    for file in &all_files {
        assert_ne!(
            file.file_name().and_then(|s| s.to_str()),
            Some("object.toml"),
            "retired object.toml found in open-pack: {}",
            file.display()
        );
        if file.extension().and_then(|s| s.to_str()) == Some("antics") {
            let src = std::fs::read_to_string(file).expect("read .antics file");
            assert!(
                !src.contains("raw {"),
                "migrated open-pack v2 file must not use raw {{ ... }} escape hatch: {}",
                file.display()
            );
            assert!(
                !src.contains("@id(") && !src.contains("@guard_id("),
                "migrated open-pack v2 file should not pin raw @id/@guard_id: {}",
                file.display()
            );
            assert!(
                !src.contains("attr[") && !src.contains("data[") && !src.contains("temp["),
                "migrated open-pack v2 file must not use raw attr[]/data[]/temp[] indexing: {}",
                file.display()
            );
        }
    }

    // 2. Both `compile_v2_pack_dir` and `compile_pack_dir` produce bit-identical output
    //    and pass `verify_pack_safety` with 0 violations across all 20 objects.
    let v2_pack = compile_v2_pack_dir(&pack_dir).expect("compile_v2_pack_dir(open-pack)");
    let std_pack = sims_antics::compile_pack_dir(&pack_dir).expect("compile_pack_dir(open-pack)");
    assert_eq!(v2_pack.objects.len(), 20);
    assert_eq!(v2_pack.far_bytes, std_pack.far_bytes);
    assert_eq!(v2_pack.sounds_wav, std_pack.sounds_wav);

    let safety = verify_pack_safety(&v2_pack).expect("verify_pack_safety");
    assert!(
        safety.is_ok(),
        "open-pack v2 safety violations: {:?}",
        safety.objects
    );

    // 3. Every one of the 20 objects expands cleanly via both `expand_v2_to_air` and
    //    `sims-antics expand <file> --pack open-pack` into valid `.air` that passes `compile_air`.
    for entry in &v2_pack.objects {
        let obj_path = pack_dir
            .join("objects")
            .join(format!("{}.antics", entry.slug));
        assert!(
            obj_path.is_file(),
            "expected flat v2 object file {}",
            obj_path.display()
        );
        let expanded = expand_v2_to_air(&obj_path, Some(&pack_dir))
            .unwrap_or_else(|e| panic!("expand_v2_to_air failed for {}: {e}", entry.slug));
        let air_mod = compile_air(&expanded)
            .unwrap_or_else(|e| panic!("compile_air failed for {}: {e}", entry.slug));
        assert_eq!(
            air_mod.behaviors.len(),
            entry.object.behaviors.len(),
            "expanded .air BHAV count mismatch for {}",
            entry.slug
        );

        let cli_out = Command::new(cli_bin())
            .args(["expand"])
            .arg(&obj_path)
            .args(["--pack"])
            .arg(&pack_dir)
            .output()
            .expect("run sims-antics expand");
        assert!(
            cli_out.status.success(),
            "sims-antics expand failed for {}: {}",
            entry.slug,
            String::from_utf8_lossy(&cli_out.stderr)
        );
        assert_eq!(
            String::from_utf8(cli_out.stdout).expect("utf8 expand stdout"),
            expanded,
            "CLI expand output must match expand_v2_to_air for {}",
            entry.slug
        );
    }

    // 4. Verify explicit legacy metadata expectations (GUID, CTSS name, SLOT kinds, FWAV names)
    //    across all 20 objects, plus open-floor-lamp default graphic 0 = lit / graphic 1 = off.
    type ExpectedMeta = (
        &'static str,
        u32,
        &'static str,
        &'static [u16],
        &'static [&'static str],
    );
    let expected_meta: &[ExpectedMeta] = &[
        (
            "open-bench",
            0x7F00_0001,
            "Open Pine Bench",
            &[3],
            &["open_bench_creak"],
        ),
        (
            "open-cot",
            0x7F00_0002,
            "Open Camp Cot",
            &[3],
            &["open_cot_settle"],
        ),
        (
            "open-basin",
            0x7F00_0003,
            "Open Ceramic Basin",
            &[3],
            &["open_basin_flush"],
        ),
        (
            "open-double-bed",
            0x7F00_0012,
            "Open Double Bed",
            &[3, 3],
            &["open_double_bed_settle", "open_double_bed_wake"],
        ),
        (
            "open-fridge",
            0x7F00_0020,
            "Open Frost Fridge",
            &[3],
            &["open_fridge_door"],
        ),
        (
            "open-counter",
            0x7F00_0021,
            "Open Oak Counter",
            &[0, 3],
            &["open_counter_chop"],
        ),
        (
            "open-stove",
            0x7F00_0022,
            "Open Hearth Stove",
            &[3],
            &["open_stove_sizzle"],
        ),
        (
            "open-meal-plate",
            0x7F00_0023,
            "Open Meal Plate",
            &[3],
            &["open_plate_eat", "open_plate_dispose"],
        ),
        (
            "open-wall-phone",
            0x7F00_0030,
            "Open Wall Phone",
            &[3],
            &["open_phone_ring", "open_phone_dial", "open_phone_confirm"],
        ),
        (
            "open-shower",
            0x7F00_0040,
            "Open Privacy Shower",
            &[3],
            &[
                "open_shower_spray",
                "open_shower_wrench",
                "open_shower_fixed",
            ],
        ),
        (
            "open-toilet",
            0x7F00_0050,
            "Open Porcelain Toilet",
            &[3],
            &["open_toilet_flush", "open_toilet_plunge"],
        ),
        (
            "open-sofa",
            0x7F00_0051,
            "Open Plush Sofa",
            &[3],
            &["open_sofa_creak", "open_sofa_chime"],
        ),
        (
            "open-tv",
            0x7F00_0052,
            "Open Retro Television",
            &[3],
            &["open_tv_click", "open_tv_jingle"],
        ),
        (
            "open-bookshelf",
            0x7F00_0053,
            "Open Timber Bookshelf",
            &[3],
            &["open_book_turn", "open_skill_chime"],
        ),
        (
            "open-dining-table",
            0x7F00_0060,
            "Open Dining Table",
            &[0, 3],
            &["open_table_tap"],
        ),
        (
            "open-dining-chair",
            0x7F00_0061,
            "Open Dining Chair",
            &[3],
            &["open_chair_creak"],
        ),
        (
            "open-floor-lamp",
            0x7F00_0070,
            "Open Floor Lamp",
            &[3],
            &["open_lamp_switch"],
        ),
        (
            "open-trash-can",
            0x7F00_0071,
            "Open Pedal Trash Can",
            &[3],
            &["open_trash_lid"],
        ),
        (
            "open-front-door",
            0x7F00_0080,
            "Open Oak Front Door",
            &[3],
            &["open_door_knock", "open_door_bell"],
        ),
        (
            "open-mailbox",
            0x7F00_0090,
            "Open Street Mailbox",
            &[3],
            &["open_mailbox_latch", "open_bills_paid"],
        ),
    ];
    assert_eq!(expected_meta.len(), v2_pack.objects.len());
    for &(slug, guid, name, slot_kinds, wav_names) in expected_meta {
        let entry = v2_pack
            .objects
            .iter()
            .find(|o| o.slug == slug)
            .unwrap_or_else(|| panic!("missing compiled object '{slug}'"));
        assert_eq!(entry.object.guid, guid, "GUID mismatch for {slug}");
        assert_eq!(entry.object.name, name, "name mismatch for {slug}");
        let iff = Iff::parse(entry.object.iff_bytes.clone()).expect("parse iff");
        let slots = sims_formats::execution::Slots::parse(
            iff.data(iff.get(b"SLOT", 128).expect("SLOT #128")),
        )
        .expect("parse SLOT");
        let actual_kinds: Vec<u16> = slots.entries.iter().map(|s| s.kind).collect();
        assert_eq!(actual_kinds, slot_kinds, "SLOT kinds mismatch for {slug}");
        let mut actual_fwavs: Vec<String> = iff
            .all(b"FWAV")
            .map(|c| {
                let d = iff.data(c);
                let end = d.iter().position(|&b| b == 0).unwrap_or(d.len());
                d[..end].iter().map(|&b| char::from(b)).collect()
            })
            .collect();
        actual_fwavs.sort();
        let mut expected_fwavs: Vec<String> = wav_names.iter().map(|s| (*s).to_owned()).collect();
        expected_fwavs.sort();
        assert_eq!(
            actual_fwavs, expected_fwavs,
            "FWAV names mismatch for {slug}"
        );
    }

    // 5. Verify Floor Lamp boots with graphic 0 (lit, data[0] == 0 for v0.3.0 save hash parity)
    //    and switches to graphic 1 when turned off, back to 0 when turned on.
    let lamp = &v2_pack
        .objects
        .iter()
        .find(|o| o.slug == "open-floor-lamp")
        .unwrap()
        .object;
    let cot = &v2_pack
        .objects
        .iter()
        .find(|o| o.slug == "open-cot")
        .unwrap()
        .object;
    let mut vm = V2VmFixture::from_objects(&[lamp, cot], 500);
    let lamp_id = vm.spawn(0x7F00_0070, 4, 4);
    assert_eq!(vm.obj_attr(lamp_id, 1), 1, "lamp starts powered on");
    assert_eq!(
        vm.obj_data(lamp_id, 0),
        0,
        "lamp must boot with graphic 0 (lit) to preserve v0.3.0 initial world state"
    );
    let room_before = vm.sim_motive(13);
    vm.run_interaction(lamp_id, "Turn Off Light", None);
    assert_eq!(vm.obj_attr(lamp_id, 1), 0);
    assert_eq!(
        vm.obj_data(lamp_id, 0),
        1,
        "Turn Off Light switches to off graphic 1"
    );
    vm.run_interaction(lamp_id, "Turn On Light", None);
    assert_eq!(vm.obj_attr(lamp_id, 1), 1);
    assert_eq!(
        vm.obj_data(lamp_id, 0),
        0,
        "Turn On Light switches back to lit graphic 0"
    );
    assert!(
        vm.sim_motive(13) > room_before + 30.0,
        "Turn On Light hook must boost Sim room motive"
    );

    // 6. Verify 2-Sim contention on `open-cot`: while Sim 1 naps, Sim 2 cannot claim the cot!
    let cot_id = vm.spawn(0x7F00_0002, 6, 4);
    let sim2_id = 10u16;
    vm.world.order.push(sim2_id);
    vm.world.people.push(sim2_id);
    vm.world.objects.insert(sim2_id, make_sim_object(sim2_id));

    let sim1_id = vm.sim_id;
    let interp = Interpreter::new(&vm.program, &vm.catalog, Stream::Lot);
    let entries1 = interp.pie_menu(&mut vm.world, sim1_id, cot_id).unwrap();
    assert_eq!(
        entries1.len(),
        1,
        "Sim 1 sees Take a Nap when cot is vacant"
    );
    let uid1 = interp
        .pick(&mut vm.world, sim1_id, entries1[0].action.clone())
        .unwrap()
        .unwrap();
    vm.world.objects.get_mut(&sim1_id).unwrap().data[25] = 1;
    let st = interp.turn(&mut vm.world, sim1_id, 4_000).unwrap();
    assert!(matches!(st, Status::Yielded));
    assert_eq!(
        vm.world.objects[&cot_id].attributes[0], 1,
        "cot locked by Sim 1"
    );
    // While Sim 1 occupies the cot, Sim 2's pie menu must hide "Take a Nap"!
    let entries2_busy = interp.pie_menu(&mut vm.world, sim2_id, cot_id).unwrap();
    assert!(
        entries2_busy.is_empty(),
        "Sim 2 must not be offered Take a Nap while Sim 1 occupies open-cot"
    );
    // Cancel Sim 1 -> lock releases -> Sim 2 is offered "Take a Nap"!
    assert!(interp.cancel(&mut vm.world, sim1_id, uid1).unwrap());
    for _ in 0..16 {
        vm.world.objects.get_mut(&sim1_id).unwrap().data[25] = 1;
        let _ = interp.turn(&mut vm.world, sim1_id, 4_000).unwrap();
        if vm.world.objects[&sim1_id].stack.len() == 1 {
            break;
        }
    }
    assert_eq!(
        vm.world.objects[&cot_id].attributes[0], 0,
        "cot lock released"
    );
    let entries2_free = interp.pie_menu(&mut vm.world, sim2_id, cot_id).unwrap();
    assert_eq!(
        entries2_free.len(),
        1,
        "Sim 2 sees Take a Nap again once Sim 1 vacates open-cot"
    );
}

#[test]
fn const_arg_helper_inlining_folds_immutable_constants_and_spills_mutated_params() {
    let src = r#"
fn set_flag(flag: &mut bool, enabled: bool) {
    *flag = enabled;
}

fn mutate_copy(val: i16, out: &mut i16) {
    val += 5;
    *out = val;
}

object "Const Fold Probe" {
    guid: 0x7F34_0001;
    state power: bool = false;
    state total: i16 = 0;

    interaction "Power On" {
        guard(sim) => !self.power,
        run(sim) {
            set_flag(&mut self.power, true);
            mutate_copy(10, &mut self.total);
            return true;
        }
    }

    interaction "Power Off" {
        guard(sim) => self.power,
        run(sim) {
            set_flag(&mut self.power, false);
            return true;
        }
    }
}
"#;
    let expanded = sims_antics::v2::expand_v2_source_to_objects(
        "probe.antics",
        src,
        &PackResolverConfig::default(),
    )
    .expect("expand probe");
    let air = &expanded[0].air_source;
    // Immutable `enabled: bool` literal folds directly to `stack.attr[0] := 1;` / `:= 0;`
    // (no `__arg_enabled` local spilled), so persistent toggle passes verify_object_safety!
    assert!(
        air.contains("stack.attr[0] := 1;") && air.contains("stack.attr[0] := 0;"),
        "expected constant-folded bool writes:\n{air}"
    );
    assert!(
        !air.contains("__arg_enabled"),
        "immutable bool literal arg must not spill a dummy local:\n{air}"
    );
    // Mutated `val: i16` parameter MUST still spill to a fresh local before `+= 5`!
    assert!(
        air.contains("__arg_val"),
        "mutated i16 param must spill to a local:\n{air}"
    );

    let compiled = compile_v2_source("probe.antics", src, &PackResolverConfig::default())
        .expect("compile probe");
    let report = verify_object_safety(&compiled).expect("verify probe");
    assert!(
        report.is_ok(),
        "persistent toggle via const-folded helper must pass safety: {:?}",
        report.violations
    );

    let mut vm = V2VmFixture::from_objects(&[&compiled], 100);
    let obj_id = vm.spawn(0x7F34_0001, 4, 4);
    vm.run_interaction(obj_id, "Power On", None);
    assert_eq!(vm.obj_attr(obj_id, 0), 1);
    assert_eq!(vm.obj_attr(obj_id, 1), 15);
    vm.run_interaction(obj_id, "Power Off", None);
    assert_eq!(vm.obj_attr(obj_id, 0), 0);

    // Sol R1 P2-1a: `param self: i16 = 7` must NOT shadow `self` ObjectId when calling a helper!
    let self_shadow_src = r#"
fn store_id(v: i16, out: &mut i16) {
    *out = v;
}

object "Self Shadow Probe" {
    guid: 0x7F34_0002;
    param self: i16 = 7;
    state captured: i16 = 0;

    interaction "Capture" {
        run(sim) {
            store_id(self, &mut self.captured);
            return true;
        }
    }
}
"#;
    let self_expanded = sims_antics::v2::expand_v2_source_to_objects(
        "self_shadow.antics",
        self_shadow_src,
        &PackResolverConfig::default(),
    )
    .expect("expand self shadow");
    assert!(
        !self_expanded[0].air_source.contains("bcon.self"),
        "`self` passed to read-only i16 helper must resolve to object id, never `bcon.self`:\n{}",
        self_expanded[0].air_source
    );
    assert!(
        self_expanded[0].air_source.contains("stack_obj"),
        "expected stack_obj for `self`:\n{}",
        self_expanded[0].air_source
    );

    // Sol R1 P2-1b: `param index: i16 = 0` shadowed by helper param `index: i16` in `self.attr[index]`!
    // Constant arg `1` must write `attr[1]`, NOT `attr[0]`, and a runtime local must be rejected!
    let idx_shadow_ok = r#"
fn write_at(index: i16, val: i16) {
    self.attr[index] = val;
}

object "Index Shadow Ok" {
    guid: 0x7F34_0003;
    param index: i16 = 0;
    state a0: i16 = 0;
    state a1: i16 = 0;

    interaction "Write One" {
        run(sim) {
            write_at(1, 42);
            return true;
        }
    }
}
"#;
    let idx_ok_obj = compile_v2_source(
        "idx_shadow_ok.antics",
        idx_shadow_ok,
        &PackResolverConfig::default(),
    )
    .expect("compile idx_shadow_ok");
    let mut vm2 = V2VmFixture::from_objects(&[&idx_ok_obj], 100);
    let idx_id = vm2.spawn(0x7F34_0003, 4, 4);
    vm2.run_interaction(idx_id, "Write One", None);
    assert_eq!(vm2.obj_attr(idx_id, 0), 0, "attr[0] must remain untouched");
    assert_eq!(
        vm2.obj_attr(idx_id, 1),
        42,
        "helper param `index = 1` must shadow `param index = 0` and write attr[1]"
    );

    let idx_shadow_runtime_err = r#"
object "Index Shadow Runtime" {
    guid: 0x7F34_0004;
    param index: i16 = 0;
    state a0: i16 = 0;

    interaction "Bad Dynamic Index" {
        run(sim) {
            let index = self.a0;
            self.attr[index] = 9;
            return true;
        }
    }
}
"#;
    let err = compile_v2_source(
        "idx_shadow_err.antics",
        idx_shadow_runtime_err,
        &PackResolverConfig::default(),
    )
    .expect_err(
        "runtime local `index` shadowing `param index` must be rejected in `self.attr[index]`",
    );
    assert!(err.message.contains("non-constant variable"), "{err}");

    // Sol R1 P2-2: Calling a helper with literal `false` must NOT suppress diagnostics in the untaken `if` branch!
    let dead_branch_err_src = r#"
fn maybe_call(enabled: bool) {
    if enabled {
        missing_helper();
    }
}

object "Dead Branch Diagnostic" {
    guid: 0x7F34_0005;

    interaction "Go" {
        run(sim) {
            maybe_call(false);
            return true;
        }
    }
}
"#;
    let err = compile_v2_source(
        "dead_branch.antics",
        dead_branch_err_src,
        &PackResolverConfig::default(),
    )
    .expect_err("invalid helper body called with literal false must still report unknown function");
    assert!(
        err.message.contains("unknown function 'missing_helper'"),
        "{err}"
    );

    // Sol R2 P2-1: Helpers can forward constant bool args into built-in compile-time options (`inform:`, `change_icon:`).
    let fwd_opt_src = r#"
fn show_notice(is_inform: bool) {
    dialog(kind: ok, title: title, message: msg, inform: is_inform);
}

object "Forward Option Probe" {
    guid: 0x7F34_0006;
    strings 301 "ProbeStr" {
        title = "Notice",
        msg = "Hello",
    }

    interaction "Notify" {
        run(sim) {
            show_notice(true);
            return true;
        }
    }
}
"#;
    let fwd_expanded = sims_antics::v2::expand_v2_source_to_objects(
        "fwd_opt.antics",
        fwd_opt_src,
        &PackResolverConfig::default(),
    )
    .expect("helper must forward const bool arg to dialog inform: option");
    assert!(
        fwd_expanded[0].air_source.contains("inform: true"),
        "{}",
        fwd_expanded[0].air_source
    );

    // Sol R2 P2-2: Renamed Sim binding (`run(actor)`) allows `param sim: i16 = 0` in `self.attr[sim]`.
    let renamed_sim_src = r#"
object "Renamed Sim Param" {
    guid: 0x7F34_0007;
    param sim: i16 = 1;
    state a0: i16 = 0;
    state a1: i16 = 0;

    interaction "Write" {
        run(actor) {
            self.attr[sim] = 77;
            return true;
        }
    }
}
"#;
    let renamed_obj = compile_v2_source(
        "renamed_sim.antics",
        renamed_sim_src,
        &PackResolverConfig::default(),
    )
    .expect("`param sim: i16 = 1` must resolve in `self.attr[sim]` when Sim is bound as `actor`");
    let mut vm3 = V2VmFixture::from_objects(&[&renamed_obj], 100);
    let rid = vm3.spawn(0x7F34_0007, 4, 4);
    vm3.run_interaction(rid, "Write", None);
    assert_eq!(vm3.obj_attr(rid, 0), 0);
    assert_eq!(vm3.obj_attr(rid, 1), 77);
}

// ===========================================================================
// 7. `.antics` v2 Tiers 1 + 2 Ergonomics & Language Extensions
// ===========================================================================

#[test]
fn tiers_1_2_compact_slots_defaults_and_autosized_attributes() {
    let cfg = PackResolverConfig::default();
    let src = r#"
object "Compact Defaults Probe" {
    guid: 0x7F40_0001;
    price: 150;
    category: misc;
    description: "Tests compact slot syntax, interaction defaults, auto-sized attributes, and play_sound default target.";
    sprite: Sprite::procedural("cot", primary: #556677, accent: #8899AA, trim: #223344);

    state uses: i16 = 0;
    state flag: bool = false;

    sound ding = preset("chime", wav: "compact_ding");

    slot front = routing([0.0, -0.75, 0.0]);
    slot side = routing([0.5, 0.25, 0.0], facing: east, sitting: true, standing: false, ground: false, min_dist: 16, max_dist: 48, optimal_dist: 32);
    slot top = container([0.0, 0.0, 0.64]);

    interaction "Ping" {
        autonomy: 60,
        advertises: [hunger +75, energy +40 (min: -50)],
        run(sim) {
            self.uses += 1;
            self.flag = true;
            play_sound(ding);
            sim.motive.hunger += 20;
        }
    }
}
"#;
    let expanded = sims_antics::v2::expand_v2_source_to_objects("compact.antics", src, &cfg)
        .expect("expand compact defaults");
    assert_eq!(expanded.len(), 1);
    let obj_exp = &expanded[0];
    // Auto-sized attributes defaults to state_count.max(8) = 8.
    assert_eq!(obj_exp.manifest.attributes, 8);
    // Interaction defaults: attenuation == 0.02, advertises min defaults to -100 while explicit -50 is preserved.
    assert_eq!(obj_exp.manifest.interactions.len(), 1);
    let inter = &obj_exp.manifest.interactions[0];
    assert!((inter.attenuation - 0.02).abs() < 1e-6);
    assert_eq!(inter.motives.len(), 2);
    assert_eq!(inter.motives[0].delta, 75);
    assert_eq!(inter.motives[0].minimum, -100);
    assert_eq!(inter.motives[1].delta, 40);
    assert_eq!(inter.motives[1].minimum, -50);
    // Compact slots produce exact SlotSpec fields.
    assert_eq!(obj_exp.manifest.slots.len(), 3);
    assert_eq!(obj_exp.manifest.slots[0].kind, 3);
    assert_eq!(obj_exp.manifest.slots[0].offset, [0.0, -0.75, 0.0]);
    assert_eq!(
        (
            obj_exp.manifest.slots[0].standing,
            obj_exp.manifest.slots[0].sitting,
            obj_exp.manifest.slots[0].ground,
            obj_exp.manifest.slots[0].facing
        ),
        (1, 0, 1, 0)
    );
    assert_eq!(obj_exp.manifest.slots[1].kind, 3);
    assert_eq!(obj_exp.manifest.slots[1].offset, [0.5, 0.25, 0.0]);
    assert_eq!(
        (
            obj_exp.manifest.slots[1].standing,
            obj_exp.manifest.slots[1].sitting,
            obj_exp.manifest.slots[1].ground,
            obj_exp.manifest.slots[1].facing
        ),
        (0, 1, 0, 2)
    );
    assert_eq!(obj_exp.manifest.slots[1].proximity, [16, 48, 32]);
    assert_eq!(obj_exp.manifest.slots[2].kind, 0);
    assert_eq!(obj_exp.manifest.slots[2].offset, [0.0, 0.0, 0.64]);
    assert_eq!(
        (
            obj_exp.manifest.slots[2].standing,
            obj_exp.manifest.slots[2].sitting,
            obj_exp.manifest.slots[2].ground,
            obj_exp.manifest.slots[2].height
        ),
        (0, 0, 0, 5)
    );
    // play_sound(ding) defaults to stack_obj in interaction context.
    assert!(
        obj_exp.air_source.contains("play_sound(1, stack_obj);"),
        "{}",
        obj_exp.air_source
    );

    let compiled = compile_v2_source("compact.antics", src, &cfg).expect("compile compact");
    let report = verify_object_safety(&compiled).expect("verify compact");
    assert!(report.is_ok(), "{:?}", report.violations);

    let mut vm = V2VmFixture::from_objects(&[&compiled], 100);
    let oid = vm.spawn(0x7F40_0001, 4, 4);
    let hunger_before = vm.sim_motive(7);
    let sounds = vm.run_interaction(oid, "Ping", None);
    assert_eq!(vm.obj_attr(oid, 0), 1);
    assert_eq!(vm.obj_attr(oid, 1), 1);
    assert!((vm.sim_motive(7) - (hunger_before + 20.0)).abs() < 1e-4);
    let sound_names: Vec<&str> = sounds
        .iter()
        .filter_map(|e| match e {
            SoundEvent::Play { name, .. } => Some(name.as_str()),
            SoundEvent::Stop { .. } => None,
        })
        .collect();
    assert_eq!(sound_names, vec!["compact_ding"]);

    // Auto-sized attributes grows above 8 when > 8 states are declared, and respects explicit attributes.
    let ten_states = r#"
object "Ten States" {
    guid: 0x7F40_0002;
    state s0: i16 = 0; state s1: i16 = 0; state s2: i16 = 0; state s3: i16 = 0; state s4: i16 = 0;
    state s5: i16 = 0; state s6: i16 = 0; state s7: i16 = 0; state s8: i16 = 0; state s9: i16 = 0;
}
"#;
    let exp_ten = sims_antics::v2::expand_v2_source_to_objects("ten.antics", ten_states, &cfg)
        .expect("expand ten states");
    assert_eq!(exp_ten[0].manifest.attributes, 10);

    let explicit_four = r#"
object "Explicit Four" {
    guid: 0x7F40_0003;
    attributes: 4;
    state s0: i16 = 0;
}
"#;
    let exp_four = sims_antics::v2::expand_v2_source_to_objects("four.antics", explicit_four, &cfg)
        .expect("expand explicit four");
    assert_eq!(exp_four[0].manifest.attributes, 4);
}

#[test]
fn tiers_1_2_declarative_lock_clause_guard_and_unlock_on_all_paths() {
    let cfg = PackResolverConfig::default();
    let src = r#"
object "Declarative Lock Probe" {
    guid: 0x7F40_0010;
    state occupied: bool = false;
    state completed: i16 = 0;
    state fail_early: bool = false;

    // 1. No explicit guard -> auto-generates `!self.occupied`
    interaction "Lock Only" {
        lock: self.occupied,
        run(sim) {
            if self.fail_early {
                return false;
            }
            repeat 3 cancel_on_abort {
                sleep(2);
                self.completed += 1;
            }
        }
    }

    // 2. Expression guard -> conjoins `!self.occupied && sim.motive.energy < 80`
    interaction "Lock And Guard" {
        lock: self.occupied,
        guard(sim) => sim.motive.energy < 80,
        run(sim) {
            self.completed += 10;
        }
    }

    // 3. Redundant leading `!self.occupied` in guard -> deduplicated cleanly
    interaction "Lock Redundant Guard" {
        lock: self.occupied,
        guard(sim) => !self.occupied && sim.motive.energy < 80,
        run(sim) {
            self.completed += 100;
        }
    }

    // 4. Block guard -> prepends `if self.occupied { return false; }`
    interaction "Lock Block Guard" {
        lock: self.occupied,
        guard(sim) {
            if sim.motive.energy >= 80 {
                return false;
            }
            return true;
        }
        run(sim) {
            self.completed += 1000;
        }
    }
}
"#;
    let expanded = sims_antics::v2::expand_v2_source_to_objects("lock_probe.antics", src, &cfg)
        .expect("expand lock probe");
    // Verify redundant guard does not emit duplicate `stack.attr[0] != 0` checks inside `Can_LockRedundantGuard`.
    let air = &expanded[0].air_source;
    let redundant_tree = air
        .split("tree ")
        .find(|s| s.contains("Can_LockRedundantGuard"))
        .expect("Can_LockRedundantGuard tree");
    assert_eq!(
        redundant_tree.matches("stack.attr[0] != 0").count(),
        1,
        "redundant leading !self.occupied must be deduplicated:\n{redundant_tree}"
    );

    let compiled = compile_v2_source("lock_probe.antics", src, &cfg).expect("compile lock probe");
    let report = verify_object_safety(&compiled).expect("verify lock probe");
    assert!(
        report.is_ok(),
        "declarative lock interactions must pass safety: {:?}",
        report.violations
    );

    let mut vm = V2VmFixture::from_objects(&[&compiled], 100);
    let oid = vm.spawn(0x7F40_0010, 4, 4);
    let sim1_id = vm.sim_id;
    let sim2_id = 10u16;
    vm.world.order.push(sim2_id);
    vm.world.people.push(sim2_id);
    vm.world.objects.insert(sim2_id, make_sim_object(sim2_id));

    // Early `return false;` releases lock!
    vm.world.objects.get_mut(&oid).unwrap().attributes[2] = 1; // fail_early = true
    vm.run_interaction(oid, "Lock Only", None);
    assert_eq!(vm.obj_attr(oid, 0), 0, "early return false must unlock");
    assert_eq!(vm.obj_attr(oid, 1), 0);

    // Normal execution with 2-Sim contention & cancel releases lock!
    vm.world.objects.get_mut(&oid).unwrap().attributes[2] = 0; // fail_early = false
    let interp = Interpreter::new(&vm.program, &vm.catalog, Stream::Lot);
    let entries1 = interp.pie_menu(&mut vm.world, sim1_id, oid).unwrap();
    assert_eq!(entries1.len(), 4, "all 4 interactions offered when vacant");
    let uid1 = interp
        .pick(&mut vm.world, sim1_id, entries1[0].action.clone())
        .unwrap()
        .unwrap();
    vm.world.objects.get_mut(&sim1_id).unwrap().data[25] = 1;
    let st = interp.turn(&mut vm.world, sim1_id, 4_000).unwrap();
    assert!(matches!(st, Status::Yielded));
    assert_eq!(
        vm.world.objects[&oid].attributes[0], 1,
        "locked while running"
    );

    // While Sim 1 holds `self.occupied`, Sim 2 sees ZERO of the 4 interactions (including block-guarded)!
    let entries2_busy = interp.pie_menu(&mut vm.world, sim2_id, oid).unwrap();
    assert!(
        entries2_busy.is_empty(),
        "all 4 lock: interactions must hide while occupied, got {}",
        entries2_busy.len()
    );

    // Cancel Sim 1 -> lock releases -> Sim 2 sees all 4 again!
    assert!(interp.cancel(&mut vm.world, sim1_id, uid1).unwrap());
    for _ in 0..16 {
        vm.world.objects.get_mut(&sim1_id).unwrap().data[25] = 1;
        let _ = interp.turn(&mut vm.world, sim1_id, 4_000).unwrap();
        if vm.world.objects[&sim1_id].stack.len() == 1 {
            break;
        }
    }
    assert_eq!(
        vm.world.objects[&oid].attributes[0], 0,
        "cancel must unlock"
    );
    let entries2_free = interp.pie_menu(&mut vm.world, sim2_id, oid).unwrap();
    assert_eq!(entries2_free.len(), 4);

    // Block guard & expression guard still respect `energy < 80` when vacant.
    vm.world
        .objects
        .get_mut(&sim2_id)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .motives[5] = 90.0;
    let entries2_energetic = interp.pie_menu(&mut vm.world, sim2_id, oid).unwrap();
    assert_eq!(
        entries2_energetic.len(),
        1,
        "only 'Lock Only' should pass when energy >= 80"
    );
}

#[test]
fn tiers_1_2_template_interactions_hook_overrides_and_minimal_params() {
    let mut cfg = PackResolverConfig::default();
    cfg.virtual_modules.insert(
        "test_pack::tpl".to_owned(),
        r#"
pub fn add_bonus(sim: Sim, amount: i16) {
    sim.motive.room += amount;
}

pub template SmartGadget {
    param initial_power: bool = false;
    param boost: i16 = 10;
    param unused_default: i16 = 99;

    state occupied: bool = false;
    state power: bool = initial_power;
    state hook_calls: i16 = 0;

    sound click = preset("ui_click");
    slot front = routing([0.0, -0.75, 0.0]);

    fn on_power_changed(sim: Sim, on: bool) {}

    interaction "Power On" {
        autonomy: 30,
        guard(sim) => !self.power,
        run(sim) {
            self.power = true;
            play_sound(click);
            on_power_changed(sim, true);
        }
    }

    interaction "Power Off" {
        autonomy: 20,
        guard(sim) => self.power && !self.occupied,
        run(sim) {
            self.power = false;
            play_sound(click);
            on_power_changed(sim, false);
        }
    }

    interaction "Inspect" {
        autonomy: 10,
        lock: self.occupied,
        run(sim) {
            add_bonus(sim, boost);
        }
    }
}
"#
        .to_owned(),
    );

    let obj_src = r#"
use test_pack::tpl::SmartGadget;

fn record_hook(counter: &mut i16) {
    *counter += 1;
}

object "Custom Lamp" from SmartGadget {
    guid: 0x7F40_0020;
    // Minimal overrides: omit `initial_power` (defaults false) and `unused_default` (defaults 99)!
    boost: 25;

    sound click = preset("chime", wav: "custom_lamp_click");

    // Override inherited hook (calling local module helper + reading overridden param `boost`)!
    fn on_power_changed(sim: Sim, on: bool) {
        record_hook(&mut self.hook_calls);
        if on {
            set_graphic(1);
            sim.motive.room += boost;
        } else {
            set_graphic(0);
        }
    }

    // Override inherited "Inspect" interaction in-place!
    interaction "Inspect" {
        autonomy: 15,
        lock: self.occupied,
        run(sim) {
            self.hook_calls += 10;
        }
    }

    // Add a new object-specific interaction!
    interaction "Pulse" {
        autonomy: 25,
        lock: self.occupied,
        guard(sim) => self.power,
        run(sim) {
            sim.motive.fun += 30;
        }
    }
}
"#;
    let compiled = compile_v2_source("custom_lamp.antics", obj_src, &cfg).expect("compile");
    let report = verify_object_safety(&compiled).expect("verify");
    assert!(report.is_ok(), "{:?}", report.violations);

    let mut vm = V2VmFixture::from_objects(&[&compiled], 100);
    let oid = vm.spawn(0x7F40_0020, 4, 4);
    assert_eq!(
        vm.obj_attr(oid, 1),
        0,
        "inherited default initial_power=false"
    );
    let room0 = vm.sim_motive(13);
    let sounds = vm.run_interaction(oid, "Power On", None);
    let sound_names: Vec<&str> = sounds
        .iter()
        .filter_map(|e| match e {
            SoundEvent::Play { name, .. } => Some(name.as_str()),
            SoundEvent::Stop { .. } => None,
        })
        .collect();
    assert_eq!(sound_names, vec!["custom_lamp_click"]);
    assert_eq!(vm.obj_attr(oid, 1), 1);
    assert_eq!(vm.obj_attr(oid, 2), 1, "on_power_changed hook ran");
    assert_eq!(vm.obj_data(oid, 0), 1, "graphic updated by hook");
    assert!((vm.sim_motive(13) - (room0 + 25.0)).abs() < 1e-4);

    // Overridden "Inspect" adds 10 instead of template body.
    vm.run_interaction(oid, "Inspect", None);
    assert_eq!(vm.obj_attr(oid, 2), 11);

    // Added "Pulse" works when powered on.
    let fun0 = vm.sim_motive(15);
    vm.run_interaction(oid, "Pulse", None);
    assert!((vm.sim_motive(15) - (fun0 + 30.0)).abs() < 1e-4);

    // Inherited "Power Off" runs hook with on=false.
    vm.run_interaction(oid, "Power Off", None);
    assert_eq!(vm.obj_attr(oid, 1), 0);
    assert_eq!(vm.obj_attr(oid, 2), 12);
    assert_eq!(vm.obj_data(oid, 0), 0);
}

#[test]
fn tiers_1_2_value_returning_i16_named_and_default_parameters() {
    let cfg = PackResolverConfig::default();
    let src = r#"
pub fn add_scaled(base: i16, step: i16 = 10, mult: i16 = 2) -> i16 {
    return base + step * mult;
}

pub fn count_cycles(limit: i16, ticks: i16 = 2) -> i16 {
    let mut done: i16 = 0;
    repeat limit cancel_on_abort {
        sleep(ticks);
        done += 1;
    }
    return done;
}

pub fn return_before_defer(val: &mut i16) -> i16 {
    *val = 42;
    defer {
        *val = 99;
    }
    return *val;
}

object "Fn I16 And Named Args Probe" {
    guid: 0x7F40_0030;
    param default_step: i16 = 7;

    state r0: i16 = 0;
    state r1: i16 = 0;
    state r2: i16 = 0;
    state r3: i16 = 0;
    state r4: i16 = 0;
    state r5: i16 = 0;
    state r6: i16 = 0;
    state defer_target: i16 = 0;

    // Local helper whose default references object param `default_step` (must not be hijacked by caller local).
    fn use_param_default(x: i16, inc: i16 = default_step) -> i16 {
        return x + inc;
    }

    interaction "Test Math" {
        run(sim) {
            // Positional + defaults: 5 + 10 * 2 = 25
            self.r0 = add_scaled(5);
            // Named out-of-order: 5 + 4 * 3 = 17
            self.r1 = add_scaled(5, mult: 3, step: 4);
            // Partial named + default step=10: 5 + 10 * 4 = 45
            self.r2 = add_scaled(5, mult: 4);
            // All named out-of-order + default mult=2: 1 + 7 * 2 = 15
            self.r3 = add_scaled(step: 7, base: 1);
            // Discarded i16 return value as statement + expression composition
            add_scaled(100);
            if add_scaled(5) > 20 {
                self.r4 = add_scaled(1, step: 2, mult: 3) + add_scaled(2, mult: 1);
            }
            // Hygiene: caller local named `default_step` must NOT hijack callee's default param `default_step = 7`!
            let default_step: i16 = 500;
            self.r5 = use_param_default(10) + (default_step - 500);
            // Return expression evaluated BEFORE defer mutates `*val`!
            self.r6 = return_before_defer(&mut self.defer_target);
        }
    }

    interaction "Test Cancel Count" {
        run(sim) {
            self.r0 = count_cycles(4, ticks: 2);
        }
    }
}
"#;
    let compiled = compile_v2_source("fn_i16.antics", src, &cfg).expect("compile fn_i16");
    let report = verify_object_safety(&compiled).expect("verify fn_i16");
    assert!(report.is_ok(), "{:?}", report.violations);

    let mut vm = V2VmFixture::from_objects(&[&compiled], 100);
    let oid = vm.spawn(0x7F40_0030, 4, 4);
    vm.run_interaction(oid, "Test Math", None);
    assert_eq!(vm.obj_attr(oid, 0), 25);
    assert_eq!(vm.obj_attr(oid, 1), 17);
    assert_eq!(vm.obj_attr(oid, 2), 45);
    assert_eq!(vm.obj_attr(oid, 3), 15);
    assert_eq!(vm.obj_attr(oid, 4), 19); // (1 + 2*3) + (2 + 10*1) = 7 + 12 = 19
    assert_eq!(
        vm.obj_attr(oid, 5),
        17,
        "default param hygiene: 10 + 7 = 17"
    );
    assert_eq!(vm.obj_attr(oid, 6), 42, "return captured before defer ran");
    assert_eq!(vm.obj_attr(oid, 7), 99, "defer ran after capturing return");

    // Full run returns 4 cycles; canceling after 1 yield returns 0 completed cycles (aborted during first sleep).
    vm.run_interaction(oid, "Test Cancel Count", None);
    assert_eq!(vm.obj_attr(oid, 0), 4);
    vm.run_interaction(oid, "Test Cancel Count", Some(1));
    assert_eq!(vm.obj_attr(oid, 0), 0);
}

#[test]
fn tiers_1_2_dynamic_skill_indexing_all_six_skills() {
    let cfg = PackResolverConfig::default();
    let src = r#"
pub fn train_skill(sim: Sim, sk: Skill, gain: i16 = 25, cap: i16 = 1000) -> i16 {
    if sim.skill[sk] < cap {
        sim.skill[sk] += gain;
    }
    return sim.skill[sk];
}

object "Skill Trainer" {
    guid: 0x7F40_0040;
    state last_level: i16 = 0;

    interaction "Train All" {
        run(sim) {
            train_skill(sim, Skill::Cooking, gain: 10);
            train_skill(sim, Skill::Charisma, gain: 20);
            train_skill(sim, Skill::Mechanical, gain: 30);
            train_skill(sim, Skill::Creativity, gain: 40);
            train_skill(sim, Skill::Body, gain: 50);
            self.last_level = train_skill(sim, Skill::Logic, gain: 60);
        }
    }
}
"#;
    let compiled = compile_v2_source("skills.antics", src, &cfg).expect("compile skills");
    let mut vm = V2VmFixture::from_objects(&[&compiled], 100);
    let oid = vm.spawn(0x7F40_0040, 4, 4);
    vm.run_interaction(oid, "Train All", None);
    assert_eq!(vm.sim_skill(10), 10, "Cooking = person.data[10]");
    assert_eq!(vm.sim_skill(11), 20, "Charisma = person.data[11]");
    assert_eq!(vm.sim_skill(12), 30, "Mechanical = person.data[12]");
    assert_eq!(vm.sim_skill(15), 40, "Creativity = person.data[15]");
    assert_eq!(vm.sim_skill(17), 50, "Body = person.data[17]");
    assert_eq!(vm.sim_skill(18), 60, "Logic = person.data[18]");
    assert_eq!(vm.obj_attr(oid, 0), 60, "returned new Logic skill level");
}

#[test]
fn tiers_1_2_error_diagnostics() {
    let cfg = PackResolverConfig::default();

    let cases: &[(&str, &str, &str)] = &[
        (
            "lock_non_bool",
            r#"
object "Bad Lock Type" {
    guid: 0x7F41_0001;
    state count: i16 = 0;
    interaction "Go" { lock: self.count, run(sim) { sleep(1); } }
}
"#,
            "requires a bool state",
        ),
        (
            "lock_unknown",
            r#"
object "Bad Lock Field" {
    guid: 0x7F41_0002;
    interaction "Go" { lock: self.missing, run(sim) { sleep(1); } }
}
"#,
            "unknown state 'missing'",
        ),
        (
            "lock_not_self",
            r#"
object "Bad Lock Base" {
    guid: 0x7F41_0003;
    interaction "Go" { lock: sim.occupied, run(sim) { sleep(1); } }
}
"#,
            "must target a bool state on 'self'",
        ),
        (
            "lock_reentrant",
            r#"
object "Double Lock" {
    guid: 0x7F41_0004;
    state busy: bool = false;
    interaction "Go" {
        lock: self.busy,
        run(sim) {
            with lock(self.busy) { sleep(1); }
        }
    }
}
"#,
            "re-entrant",
        ),
        (
            "dup_named_arg",
            r#"
fn f(a: i16, b: i16 = 1) -> i16 { return a + b; }
object "Dup Named" {
    guid: 0x7F41_0005;
    interaction "Go" { run(sim) { let x = f(1, a: 2); } }
}
"#,
            "duplicate argument for parameter 'a'",
        ),
        (
            "pos_after_named",
            r#"
fn f(a: i16, b: i16) -> i16 { return a + b; }
object "Pos After Named" {
    guid: 0x7F41_0006;
    interaction "Go" { run(sim) { let x = f(a: 1, 2); } }
}
"#,
            "positional argument cannot follow a named argument",
        ),
        (
            "unknown_named_arg",
            r#"
fn f(a: i16) -> i16 { return a; }
object "Unknown Named" {
    guid: 0x7F41_0007;
    interaction "Go" { run(sim) { let x = f(bogus: 1); } }
}
"#,
            "has no parameter named 'bogus'",
        ),
        (
            "missing_required_arg",
            r#"
fn f(a: i16, b: i16 = 2) -> i16 { return a + b; }
object "Missing Req" {
    guid: 0x7F41_0008;
    interaction "Go" { run(sim) { let x = f(b: 5); } }
}
"#,
            "missing required argument 'a'",
        ),
        (
            "req_after_default",
            r#"
fn f(a: i16 = 1, b: i16) -> i16 { return a + b; }
object "Req After Default" { guid: 0x7F41_0009; }
"#,
            "cannot follow a parameter with a default value",
        ),
        (
            "mutref_default",
            r#"
fn f(x: &mut i16 = 0) {}
object "MutRef Default" { guid: 0x7F41_000A; }
"#,
            "cannot have a default value",
        ),
        (
            "functional_default",
            r#"
object "Func Default" {
    guid: 0x7F41_000B;
    functional prep(a: i16 = 1) { run(sim) {} }
}
"#,
            "functional parameter 'a' cannot have a default value",
        ),
        (
            "i16_fn_returns_bool",
            r#"
fn bad() -> i16 { return true; }
object "Wrong Ret" {
    guid: 0x7F41_000C;
    interaction "Go" { run(sim) { let x = bad(); } }
}
"#,
            "returning 'i16' cannot return 'bool'",
        ),
        (
            "i16_fn_missing_return",
            r#"
fn bad(c: bool) -> i16 {
    if c { return 1; }
}
object "Missing Ret" {
    guid: 0x7F41_000D;
    interaction "Go" { run(sim) { let x = bad(true); } }
}
"#,
            "does not return on all control-flow paths",
        ),
        (
            "i16_fn_used_as_bool",
            r#"
fn num() -> i16 { return 1; }
object "I16 As Bool" {
    guid: 0x7F41_000E;
    interaction "Go" { run(sim) { if num() { sleep(1); } } }
}
"#,
            "returns 'i16' and cannot be used directly as a bool condition",
        ),
        (
            "unknown_skill",
            r#"
fn train(sim: Sim, sk: Skill) { sim.skill[sk] += 10; }
object "Bad Skill" {
    guid: 0x7F41_000F;
    interaction "Go" { run(sim) { train(sim, Skill::Alchemy); } }
}
"#,
            "unknown Skill::Alchemy",
        ),
        (
            "shadowed_skill_ident",
            r#"
object "Shadow Skill" {
    guid: 0x7F41_0010;
    interaction "Go" {
        run(sim) {
            let cooking: i16 = 5;
            sim.skill[cooking] += 10;
        }
    }
}
"#,
            "variable 'cooking' is not a Skill",
        ),
        (
            "unknown_slot_ctor",
            r#"
object "Bad Slot Ctor" {
    guid: 0x7F41_0011;
    slot front = magic([0.0, 0.0, 0.0]);
}
"#,
            "unknown slot constructor 'magic'",
        ),
    ];

    for &(name, src, expected_substr) in cases {
        let err = compile_v2_source(name, src, &cfg)
            .expect_err(&format!("case '{name}' must fail to compile"));
        assert!(
            err.message.contains(expected_substr),
            "case '{name}' expected error containing {expected_substr:?}, got: {err}"
        );
    }
}

#[test]
fn tiers_1_2_sol_r1_regressions() {
    let cfg = PackResolverConfig::default();

    // P1-4: Recursive default parameter triggers cycle detection (not 256-expansion budget),
    // while finite explicit nesting `f(f(1))` succeeds!
    let rec_def_err = compile_v2_source(
        "rec_default.antics",
        r#"
fn f(x: i16 = f()) -> i16 { return x; }
object "Rec Def" {
    guid: 0x7F42_0001;
    interaction "Go" { run(sim) { let v = f(); } }
}
"#,
        &cfg,
    )
    .expect_err("recursive default expression must hit cycle detector");
    assert!(
        rec_def_err
            .message
            .contains("recursive function call cycle detected"),
        "{rec_def_err}"
    );

    let finite_nest_obj = compile_v2_source(
        "finite_nest.antics",
        r#"
fn inc(x: i16 = 0) -> i16 { return x + 1; }
object "Finite Nest" {
    guid: 0x7F42_0002;
    state out: i16 = 0;
    interaction "Go" { run(sim) { self.out = inc(inc(inc(1))); } }
}
"#,
        &cfg,
    )
    .expect("finite explicit nesting inc(inc(inc(1))) must compile");
    let mut vm_nest = V2VmFixture::from_objects(&[&finite_nest_obj], 100);
    let nid = vm_nest.spawn(0x7F42_0002, 4, 4);
    vm_nest.run_interaction(nid, "Go", None);
    assert_eq!(vm_nest.obj_attr(nid, 0), 4);

    // P1-5: Default expression must NOT capture caller's `run(actor)` Sim binding!
    let sim_leak_err = compile_v2_source(
        "sim_leak_default.antics",
        r#"
fn f(x: i16 = actor.priority) -> i16 { return x; }
object "Sim Leak" {
    guid: 0x7F42_0003;
    interaction "Go" { run(actor) { let v = f(); } }
}
"#,
        &cfg,
    )
    .expect_err("default parameter must not capture caller's run(actor) binding");
    assert!(
        sim_leak_err.message.contains("unknown")
            || sim_leak_err
                .message
                .contains("not a valid assignment target"),
        "{sim_leak_err}"
    );

    // P1-6: Arithmetic and unary negation reject boolean operands (both in returns and defaults)!
    for (name, src, expected) in [
        (
            "bool_add",
            r#"
fn bad() -> i16 { return true + 1; }
object "Bool Add" {
    guid: 0x7F42_0004;
    interaction "Go" { run(sim) { let x = bad(); } }
}
"#,
            "cannot be applied to 'bool' operand",
        ),
        (
            "bool_neg",
            r#"
fn bad() -> i16 { return -true; }
object "Bool Neg" {
    guid: 0x7F42_0005;
    interaction "Go" { run(sim) { let x = bad(); } }
}
"#,
            "cannot apply unary '-' to 'bool'",
        ),
        (
            "bool_arith_default",
            r#"
fn bad(x: i16 = true + 1) -> i16 { return x; }
object "Bool Default" {
    guid: 0x7F42_0006;
    interaction "Go" { run(sim) { let x = bad(); } }
}
"#,
            "cannot be applied to 'bool' operand",
        ),
    ] {
        let err = compile_v2_source(name, src, &cfg).expect_err(&format!("{name} must fail"));
        assert!(err.message.contains(expected), "{name}: {err}");
    }

    // P1-7: Calling a value-returning helper (and helper-local loop break) inside `defer` works,
    // while `return`/`break` directly escaping `defer` is STILL rejected!
    let defer_call_obj = compile_v2_source(
        "defer_call.antics",
        r#"
fn compute_cleanup() -> i16 {
    let mut n: i16 = 0;
    repeat 5 {
        n += 1;
        if n == 3 {
            break;
        }
    }
    return n * 10;
}
object "Defer Helper" {
    guid: 0x7F42_0007;
    state out: i16 = 0;
    interaction "Go" {
        run(sim) {
            defer {
                self.out = compute_cleanup();
            }
        }
    }
}
"#,
        &cfg,
    )
    .expect("calling value-returning helper with internal break inside defer must compile");
    let mut vm_def = V2VmFixture::from_objects(&[&defer_call_obj], 100);
    let did = vm_def.spawn(0x7F42_0007, 4, 4);
    vm_def.run_interaction(did, "Go", None);
    assert_eq!(vm_def.obj_attr(did, 0), 30);

    let defer_escape_fn_err = compile_v2_source(
        "defer_escape_fn.antics",
        r#"
fn bad() -> i16 {
    defer { return 1; }
    return 2;
}
object "Defer Escape Fn" {
    guid: 0x7F42_0008;
    interaction "Go" { run(sim) { let x = bad(); } }
}
"#,
        &cfg,
    )
    .expect_err("return inside defer attempting to escape helper must still fail");
    assert!(
        defer_escape_fn_err
            .message
            .contains("'return' cannot exit out of a 'defer' block"),
        "{defer_escape_fn_err}"
    );

    let defer_escape_loop_err = compile_v2_source(
        "defer_escape_loop.antics",
        r#"
object "Defer Escape Loop" {
    guid: 0x7F42_0009;
    interaction "Go" {
        run(sim) {
            repeat 2 {
                defer { break; }
            }
        }
    }
}
"#,
        &cfg,
    )
    .expect_err("break inside defer targeting outer loop must fail");
    assert!(
        defer_escape_loop_err
            .message
            .contains("'break' cannot exit out of a 'defer' block"),
        "{defer_escape_loop_err}"
    );

    // P2-1: Declaring state conflicting with built-in object property (`room`, `id`, etc.) is rejected!
    let state_room_err = compile_v2_source(
        "state_room.antics",
        r#"
object "State Room" {
    guid: 0x7F42_000A;
    state room: bool = false;
    interaction "Go" { lock: self.room, run(sim) { sleep(1); } }
}
"#,
        &cfg,
    )
    .expect_err("state name conflicting with built-in self.room must be rejected");
    assert!(
        state_room_err
            .message
            .contains("conflicts with built-in object property 'self.room'"),
        "{state_room_err}"
    );

    // P2-2: Object `param` shadowing bare skill/motive name fails cleanly, while `Skill::...`/`Motive::...` works!
    let param_skill_err = compile_v2_source(
        "param_skill_shadow.antics",
        r#"
object "Param Skill Shadow" {
    guid: 0x7F42_000B;
    param cooking: i16 = 5;
    interaction "Go" { run(sim) { sim.skill[cooking] += 10; } }
}
"#,
        &cfg,
    )
    .expect_err("bare param cooking in sim.skill[cooking] must fail");
    assert!(
        param_skill_err
            .message
            .contains("parameter 'cooking' is not a Skill"),
        "{param_skill_err}"
    );

    let param_motive_err = compile_v2_source(
        "param_motive_shadow.antics",
        r#"
object "Param Motive Shadow" {
    guid: 0x7F42_000C;
    param energy: i16 = 5;
    interaction "Go" { run(sim) { sim.motive[energy] += 10; } }
}
"#,
        &cfg,
    )
    .expect_err("bare param energy in sim.motive[energy] must fail");
    assert!(
        param_motive_err
            .message
            .contains("parameter 'energy' is not a Motive"),
        "{param_motive_err}"
    );

    let explicit_enum_ok = compile_v2_source(
        "explicit_enum_shadow.antics",
        r#"
object "Explicit Enum Shadow" {
    guid: 0x7F42_000D;
    param cooking: i16 = 5;
    param energy: i16 = 7;
    interaction "Go" {
        run(sim) {
            sim.skill[Skill::Cooking] += cooking;
            sim.motive[Motive::Energy] += energy;
        }
    }
}
"#,
        &cfg,
    )
    .expect("Skill::Cooking and Motive::Energy must work even when params share those names");
    let mut vm_enum = V2VmFixture::from_objects(&[&explicit_enum_ok], 100);
    let eid = vm_enum.spawn(0x7F42_000D, 4, 4);
    let e0 = vm_enum.sim_motive(5);
    vm_enum.run_interaction(eid, "Go", None);
    assert_eq!(vm_enum.sim_skill(10), 5);
    assert!((vm_enum.sim_motive(5) - (e0 + 7.0)).abs() < 1e-4);

    // P2-3a: Side-effecting named-argument call-site evaluation order AND omitted default
    // that iterates foreign objects (restoring `stack_obj` back to `self`).
    let peer_obj = compile_v2_source(
        "peer.antics",
        r#"
object "Peer Marker" {
    guid: 0x7F42_000E;
    state value: i16 = 13;
}
"#,
        &cfg,
    )
    .expect("compile peer");
    let order_and_foreign_obj = compile_v2_source(
        "order_and_foreign.antics",
        r#"
fn push_digit(log: &mut i16, d: i16) -> i16 {
    *log = *log * 10 + d;
    return d;
}

fn sum_peers() -> i16 {
    let mut total: i16 = 0;
    for p in objects_by_guid(0x7F42_000E) {
        total += p.attr[0];
    }
    return total;
}

fn combine(a: i16, b: i16, bonus: i16 = sum_peers()) -> i16 {
    return a * 100 + b * 10 + bonus;
}

object "Order And Foreign Default" {
    guid: 0x7F42_000F;
    state log: i16 = 0;
    state result: i16 = 0;
    state after_self_write: i16 = 0;

    interaction "Run" {
        run(sim) {
            // `b` is listed BEFORE `a` at the call site, so `log` must record `2` then `1` (= 21)!
            // Omitted `bonus` calls `sum_peers()` which clobbers `stack_obj`, then restores `__self_obj`!
            self.result = combine(b: push_digit(&mut self.log, 2), a: push_digit(&mut self.log, 1));
            self.after_self_write = 77;
        }
    }
}
"#,
        &cfg,
    )
    .expect("compile order_and_foreign");
    let mut vm_ord = V2VmFixture::from_objects(&[&peer_obj, &order_and_foreign_obj], 100);
    let _peer1 = vm_ord.spawn(0x7F42_000E, 2, 2);
    let _peer2 = vm_ord.spawn(0x7F42_000E, 2, 3);
    let ord_id = vm_ord.spawn(0x7F42_000F, 4, 4);
    vm_ord.run_interaction(ord_id, "Run", None);
    assert_eq!(
        vm_ord.obj_attr(ord_id, 0),
        21,
        "named args must evaluate in call-site order (b then a => 21)"
    );
    assert_eq!(
        vm_ord.obj_attr(ord_id, 1),
        100 + 20 + 26,
        "combine(a=1, b=2, bonus=2*13=26) == 146"
    );
    assert_eq!(
        vm_ord.obj_attr(ord_id, 2),
        77,
        "stack_obj must be restored to self after foreign-object default evaluation"
    );

    // P2-3b: `lock:` + nested `with lock(self.inner)` + `defer` + `continue` & `break`!
    let nested_lock_obj = compile_v2_source(
        "nested_lock_loop.antics",
        r#"
object "Nested Lock Loop" {
    guid: 0x7F42_0010;
    state outer_lock: bool = false;
    state inner_lock: bool = false;
    state defer_runs: i16 = 0;
    state outer_held_after_inner: i16 = 0;
    state inner_free_after_iter: i16 = 0;

    interaction "Loop Exits" {
        lock: self.outer_lock,
        run(sim) {
            let mut i: i16 = 0;
            while i < 4 {
                i += 1;
                with lock(self.inner_lock) {
                    defer {
                        self.defer_runs += 1;
                    }
                    if i == 1 {
                        continue;
                    }
                    if i == 2 {
                        break;
                    }
                }
            }
            // After loop exits via continue (iter 1) and break (iter 2):
            // - inner_lock must be 0, outer_lock must STILL be 1, defer_runs must be exactly 2!
            if self.outer_lock {
                self.outer_held_after_inner = 1;
            }
            if !self.inner_lock {
                self.inner_free_after_iter = 1;
            }
        }
    }
}
"#,
        &cfg,
    )
    .expect("compile nested_lock_loop");
    let nested_report = verify_object_safety(&nested_lock_obj).expect("verify nested_lock_loop");
    assert!(nested_report.is_ok(), "{:?}", nested_report.violations);
    let mut vm_nl = V2VmFixture::from_objects(&[&nested_lock_obj], 100);
    let nl_id = vm_nl.spawn(0x7F42_0010, 4, 4);
    vm_nl.run_interaction(nl_id, "Loop Exits", None);
    assert_eq!(vm_nl.obj_attr(nl_id, 0), 0, "outer lock released on exit");
    assert_eq!(vm_nl.obj_attr(nl_id, 1), 0, "inner lock released");
    assert_eq!(
        vm_nl.obj_attr(nl_id, 2),
        2,
        "defer ran exactly once on continue + once on break"
    );
    assert_eq!(
        vm_nl.obj_attr(nl_id, 3),
        1,
        "outer lock remained held across loop continue/break"
    );
    assert_eq!(
        vm_nl.obj_attr(nl_id, 4),
        1,
        "inner lock was released before post-loop code"
    );

    // P1-1, P1-2, P1-3: Verify on-disk `open-pack/` metadata & per-cycle counter parity!
    let pack_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("open-pack");
    let pack = compile_v2_pack_dir(&pack_dir).expect("compile open-pack");
    for slug in ["open-basin", "open-bench", "open-cot"] {
        let obj = &pack.objects.iter().find(|o| o.slug == slug).unwrap().object;
        let iff = Iff::parse(obj.iff_bytes.clone()).expect("parse iff");
        let objd_data = iff.data(iff.get(b"OBJD", 128).expect("OBJD #128"));
        let num_attrs = i16::from_le_bytes([objd_data[58 * 2], objd_data[58 * 2 + 1]]);
        assert_eq!(
            num_attrs, 1,
            "{slug} OBJD num_attributes must remain 1 (no metadata drift)"
        );
    }

    // P1-1: Verify `open-floor-lamp` preserves Turn On Light (autonomy 40, room +40) and Turn Off Light (autonomy 15, available even while occupied).
    let lamp = &pack
        .objects
        .iter()
        .find(|o| o.slug == "open-floor-lamp")
        .unwrap()
        .object;
    let lamp_iff = Iff::parse(lamp.iff_bytes.clone()).expect("parse lamp iff");
    let ttab =
        InteractionTable::parse(lamp_iff.data(lamp_iff.get(b"TTAB", 128).expect("TTAB #128")))
            .expect("parse lamp TTAB");
    assert_eq!(ttab.interactions[0].autonomy, 40, "Turn On Light autonomy");
    assert_eq!(
        ttab.interactions[0].motives[13].delta, 40,
        "Turn On Light advertises room +40"
    );
    assert_eq!(ttab.interactions[1].autonomy, 15, "Turn Off Light autonomy");
    let mut vm_lamp = V2VmFixture::from_objects(&[lamp], 100);
    let lamp_id = vm_lamp.spawn(0x7F00_0070, 4, 4);
    vm_lamp.world.objects.get_mut(&lamp_id).unwrap().attributes[0] = 1; // occupied = true
    assert!(
        vm_lamp
            .pie_menu_labels(lamp_id)
            .contains(&"Turn Off Light".to_owned()),
        "Turn Off Light must remain available even while another Sim is enjoying the light"
    );

    // P1-2: Verify `open-tv` increments `watch_cycles` (attr[2]) live on EACH cycle (not only at loop end)!
    let tv = &pack
        .objects
        .iter()
        .find(|o| o.slug == "open-tv")
        .unwrap()
        .object;
    let mut vm_tv = V2VmFixture::from_objects(&[tv], 100);
    let tv_id = vm_tv.spawn(0x7F00_0052, 4, 4);
    let sim_id = vm_tv.sim_id;
    let interp = Interpreter::new(&vm_tv.program, &vm_tv.catalog, Stream::Lot);
    let watch_entry = interp
        .pie_menu(&mut vm_tv.world, sim_id, tv_id)
        .unwrap()
        .into_iter()
        .find(|e| e.action.table_index == 2)
        .expect("Watch TV offered");
    let _ = interp
        .pick(&mut vm_tv.world, sim_id, watch_entry.action)
        .unwrap();
    // Each cycle sleeps 2 ticks; step 3 yields -> cycle 1 finished, cycle 2 mid-sleep!
    for _ in 0..3 {
        vm_tv.world.objects.get_mut(&sim_id).unwrap().data[25] = 1;
        let st = interp.turn(&mut vm_tv.world, sim_id, 4_000).unwrap();
        assert!(matches!(st, Status::Yielded));
    }
    assert_eq!(
        vm_tv.obj_attr(tv_id, 2),
        1,
        "open-tv watch_cycles must increment per cycle (visible during cycle 2)"
    );

    // Sol R2 P1: Omitted default calling a helper whose `Object` parameter is named `sim`
    // (`sim.attr[0]`) must still trigger `__self_obj` save/restore so subsequent `self` writes
    // target `self` rather than the foreign peer object!
    let r2_peer = compile_v2_source(
        "r2_peer.antics",
        r#"
object "R2 Peer" {
    guid: 0x7F42_0011;
    state val: i16 = 37;
    state sentinel: i16 = 0;
}
"#,
        &cfg,
    )
    .expect("compile r2_peer");
    let r2_reader = compile_v2_source(
        "r2_reader.antics",
        r#"
fn read_peer(sim: Object) -> i16 {
    return sim.attr[0];
}

object "R2 Default Reader" {
    guid: 0x7F42_0012;
    state peer: ObjectId = 0;
    state result: i16 = 0;

    fn sample(value: i16 = read_peer(self.peer)) -> i16 {
        return value;
    }

    interaction "Sample" {
        run(actor) {
            self.result = sample();
        }
    }
}
"#,
        &cfg,
    )
    .expect("compile r2_reader");
    let mut vm_r2 = V2VmFixture::from_objects(&[&r2_peer, &r2_reader], 100);
    let peer_id = vm_r2.spawn(0x7F42_0011, 2, 2);
    let reader_id = vm_r2.spawn(0x7F42_0012, 4, 4);
    vm_r2.world.objects.get_mut(&reader_id).unwrap().attributes[0] = peer_id as i16;
    vm_r2.run_interaction(reader_id, "Sample", None);
    assert_eq!(
        vm_r2.obj_attr(reader_id, 1),
        37,
        "self.result on reader must receive 37 after foreign read_peer(sim: Object) default"
    );
    assert_eq!(
        vm_r2.obj_attr(peer_id, 1),
        0,
        "peer.sentinel must remain untouched (stack_obj restored before writing self.result)"
    );
}

#[test]
fn tier3_match_dialog_and_auto_interned_strings_in_vm() {
    let cfg = PackResolverConfig::default();
    let obj = compile_v2_source(
        "dialog_match_test.antics",
        r#"
object "Dialog Tester" {
    guid: 0x7F50_0001;
    state choice: i16 = 0;

    strings 301 "ExistingDialogs" {
        existing_title = "Dialog Tester",
    }

    interaction "Three Way" {
        run(sim) {
            match dialog(
                kind: yes_no_cancel,
                title: "Dialog Tester",
                message: "Pick an option",
                buttons: ("Alpha", "Beta", "Gamma"),
                icon_mode: 1,
            ) {
                Button1 => { self.choice = 11; }
                Button2 => { self.choice = 22; }
                Button3 => { self.choice = 33; }
            }
        }
    }

    interaction "Two Way" {
        run(sim) {
            match dialog(
                kind: yes_no,
                title: "Dialog Tester",
                message: "Confirm action?",
                buttons: ("Confirm", "Abort"),
                icon_mode: 1,
            ) {
                Yes => { self.choice = 44; }
                No => { self.choice = 55; }
            }
        }
    }
}
"#,
        &cfg,
    )
    .expect("compile dialog tester");

    // Verify STR# 301 preserved explicit entry AND deduplicated "Dialog Tester" across all dialogs!
    let iff = Iff::parse(obj.iff_bytes.clone()).expect("parse iff");
    let str301 =
        StringTable::parse(iff.data(iff.get(b"STR#", 301).expect("STR# 301"))).expect("parse 301");
    let texts: Vec<&str> = str301.entries.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(
        texts,
        vec![
            "Dialog Tester",
            "Pick an option",
            "Alpha",
            "Beta",
            "Gamma",
            "Confirm action?",
            "Confirm",
            "Abort",
        ],
        "STR# 301 must preserve explicit entries first and deduplicate repeated inline strings"
    );

    let report = verify_object_safety(&obj).expect("verify safety");
    assert!(report.is_ok(), "{:?}", report.violations);

    // Execute all 5 button paths live in sims-vm!
    let mut vm = V2VmFixture::from_objects(&[&obj], 100);
    let oid = vm.spawn(0x7F50_0001, 4, 4);

    for (label, answer_status, expected) in [
        ("Three Way", 2u8, 11i16),
        ("Three Way", 3u8, 22i16),
        ("Three Way", 4u8, 33i16),
        ("Two Way", 2u8, 44i16),
        ("Two Way", 3u8, 55i16),
    ] {
        vm.run_interaction_with_dialog(oid, label, answer_status);
        assert_eq!(
            vm.obj_attr(oid, 0),
            expected,
            "{label} with dialog answer {answer_status} must set choice={expected}"
        );
    }
}

#[test]
fn tier3_enums_and_bitflags_with_1_based_bits_and_module_imports_in_vm() {
    let mut cfg = PackResolverConfig::default();
    cfg.virtual_modules.insert(
        "crate::shared::types".to_owned(),
        r#"
pub enum Mode {
    Idle,
    Active = 5,
    Done,
}

pub bitflags Channels {
    Left = 1,
    Right = 2,
    Center = 3,
    Sub = 8,
}

pub fn enable_channel(ch: &mut Channels, bit: Channels) {
    ch.set(bit);
}

pub fn toggle_channel(ch: &mut Channels, bit: Channels) -> bool {
    if ch.contains(bit) {
        ch.clear(bit);
        return false;
    }
    ch.set(bit);
    return true;
}

pub fn next_mode(m: Mode) -> Mode {
    match m {
        Mode::Idle => { return Mode::Active; }
        Mode::Active => { return Mode::Done; }
        _ => { return Mode::Idle; }
    }
}
"#
        .to_owned(),
    );

    let src = r#"
use crate::shared::types::{Mode, Channels, enable_channel, toggle_channel, next_mode};

object "Typed Device" {
    guid: 0x7F50_0002;

    enum LocalStatus {
        Ok = 1,
        Warn = 2,
    }

    state mode: Mode = Mode::Idle;
    state channels: Channels = Channels::Left | Channels::Center;
    state status: LocalStatus = LocalStatus::Ok;
    state runtime_check: i16 = 0;

    interaction "Exercise Types" {
        run(sim) {
            // Advance enum twice: Idle(0) -> Active(5) -> Done(6)
            self.mode = next_mode(self.mode);
            if self.mode == Mode::Active {
                self.mode = next_mode(self.mode);
            }
            if self.mode == Mode::Done {
                self.status = LocalStatus::Warn;
            }

            // Bitflags 1-based bits: starts with Left(bit1=1) | Center(bit3=4) = 5.
            // Clear Left (bit1 -> 4), set Sub (bit8=128 -> 132), enable Right via &mut state (bit2=2 -> 134).
            self.channels.clear(Channels::Left);
            self.channels.set(Channels::Sub);
            enable_channel(&mut self.channels, Channels::Right);

            // Toggle on a local `&mut Channels` exercises both set and clear branches cleanly.
            let mut scratch: Channels = Channels::Center;
            let first = toggle_channel(&mut scratch, Channels::Right);
            let second = toggle_channel(&mut scratch, Channels::Right);

            // Runtime mask variable + multi-bit contains / clear!
            let mask: Channels = Channels::Center | Channels::Sub;
            if first
                && !second
                && self.channels.contains(mask)
                && self.channels.contains(Channels::Right)
                && !self.channels.is_empty()
            {
                self.runtime_check = 1;
            }
        }
    }
}
"#;

    let expanded = sims_antics::v2::expand_v2_source_to_objects("typed_device.antics", src, &cfg)
        .expect("expand typed_device");
    let air = &expanded[0].air_source;
    // Verify single-bit constant flag ops emit compact 1-based SimAntics flag ops!
    assert!(
        air.contains("flag_clear 1") && air.contains("flag_set 8") && air.contains("flag_test 2"),
        "expected compact 1-based flag_set/flag_clear/flag_test in .air:\n{air}"
    );

    let obj = compile_v2_source("typed_device.antics", src, &cfg).expect("compile typed_device");
    let report = verify_object_safety(&obj).expect("verify");
    assert!(report.is_ok(), "{:?}", report.violations);

    let mut vm = V2VmFixture::from_objects(&[&obj], 100);
    let oid = vm.spawn(0x7F50_0002, 4, 4);
    // Initial states: mode=0, channels=1|4=5, status=1
    assert_eq!(vm.obj_attr(oid, 0), 0);
    assert_eq!(vm.obj_attr(oid, 1), 5);
    assert_eq!(vm.obj_attr(oid, 2), 1);

    vm.run_interaction(oid, "Exercise Types", None);
    assert_eq!(vm.obj_attr(oid, 0), 6, "Mode::Done == 6");
    assert_eq!(
        vm.obj_attr(oid, 1),
        (1 << 2) | (1 << 7) | (1 << 1),
        "Center(4) | Sub(128) | Right(2) == 134"
    );
    assert_eq!(vm.obj_attr(oid, 2), 2, "LocalStatus::Warn == 2");
    assert_eq!(
        vm.obj_attr(oid, 3),
        1,
        "multi-bit .contains(mask) succeeded"
    );
}

#[test]
fn tier3_named_placement_properties_and_typed_cross_object_refs_in_vm() {
    let mut cfg = PackResolverConfig::default();
    let widget_src = r#"
object "Open Widget" {
    guid: 0x7F50_0010;
    state charge: i16 = 10;
    state ready: bool = false;

    init {
        self.allowed_heights = Height::Floor | Height::Table;
        self.placement = Placement::Floor | Placement::Terrain;
    }

    functional prep(boost: i16) -> i16 {
        score: 40,
        run(sim) {
            self.charge += boost;
            self.ready = true;
            return self.charge;
        }
    }
}
"#;
    cfg.virtual_modules.insert(
        "crate::objects::open_widget".to_owned(),
        widget_src.to_owned(),
    );

    let hub_src = r#"
use crate::objects::open_widget::OpenWidget;

object "Open Hub" {
    guid: 0x7F50_0011;
    state last_charge: i16 = 0;
    state spawned_id: ObjectId = 0;

    interaction "Orchestrate" {
        run(sim) {
            for w in objects_of::<OpenWidget>() {
                w.charge += 5;
            }
            let Some(found) = find_best_object::<OpenWidget>(prep) else {
                return false;
            };
            let Some(total) = found.run_functional(prep, 7) else {
                return false;
            };
            self.last_charge = total;

            let Some(spawned) = create_object::<OpenWidget>(mode: stack_ahead, flags: 0x40) else {
                return false;
            };
            spawned.charge = total;
            spawned.ready = true;
            self.spawned_id = spawned;
        }
    }
}
"#;

    let widget = compile_v2_source("open_widget.antics", widget_src, &cfg).expect("compile widget");
    let hub = compile_v2_source("open_hub.antics", hub_src, &cfg).expect("compile hub");

    let mut vm = V2VmFixture::from_objects(&[&widget, &hub], 100);
    let wid = vm.spawn(0x7F50_0010, 3, 4);
    let hid = vm.spawn(0x7F50_0011, 5, 5);

    // Verify named object-data properties initialized data[4] and data[42]!
    assert_eq!(
        vm.world.objects[&wid].data[4], 5,
        "Height::Floor(1) | Height::Table(4) == 5"
    );
    assert_eq!(
        vm.world.objects[&wid].data[42], 3,
        "Placement::Floor(1) | Placement::Terrain(2) == 3"
    );

    vm.run_interaction(hid, "Orchestrate", None);
    // 10 (init) + 5 (objects_of loop) + 7 (prep boost) = 22!
    assert_eq!(vm.obj_attr(wid, 0), 22);
    assert_eq!(vm.obj_attr(wid, 1), 1, "ready == true");
    assert_eq!(
        vm.obj_attr(hid, 0),
        22,
        "hub.last_charge received typed prep return"
    );

    let spawned_id = vm.obj_attr(hid, 1) as u16;
    assert!(spawned_id > 0 && spawned_id != wid);
    assert_eq!(vm.obj_attr(spawned_id, 0), 22, "spawned.charge == 22");
    assert_eq!(vm.obj_attr(spawned_id, 1), 1, "spawned.ready == true");
}

#[test]
fn tier3_guard_is_autonomous_runtime_vm() {
    let cfg = PackResolverConfig::default();
    let src = r#"
object "Autonomy Gate" {
    guid: 0x7F50_0020;
    state occupied: bool = false;
    state uses: i16 = 0;

    interaction "Smart Nap" {
        autonomy: 50,
        lock: self.occupied,
        guard(sim, is_autonomous) => !is_autonomous || sim.motive.energy < 20,
        run(sim) {
            self.uses += 1;
        }
    }

    interaction "Manual Only" {
        autonomy: 50,
        guard(sim, is_auto) {
            if is_auto {
                return false;
            }
            return true;
        }
        run(sim) {
            self.uses += 10;
        }
    }
}
"#;
    let expanded = sims_antics::v2::expand_v2_source_to_objects("autonomy_gate.antics", src, &cfg)
        .expect("expand");
    assert!(
        expanded[0].air_source.contains("(args: 1)"),
        "guard(sim, is_autonomous) must emit (args: 1):\n{}",
        expanded[0].air_source
    );

    let obj = compile_v2_source("autonomy_gate.antics", src, &cfg).expect("compile");
    let iff = Iff::parse(obj.iff_bytes.clone()).expect("parse iff");
    let ttab = InteractionTable::parse(iff.data(iff.get(b"TTAB", 128).expect("TTAB"))).unwrap();
    let smart_nap_guard = ttab.interactions[0].test;
    let manual_only_guard = ttab.interactions[1].test;
    assert!(smart_nap_guard != 0 && manual_only_guard != 0);

    let mut vm = V2VmFixture::from_objects(&[&obj], 100);
    let oid = vm.spawn(0x7F50_0020, 4, 4);
    let sim_id = vm.sim_id;

    // Set energy = 50 (above autonomous threshold of 20).
    vm.world
        .objects
        .get_mut(&sim_id)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .motives[MOTIVE_ENERGY] = 50.0;

    // 1. Player pie menu (`param[0] == 0`): BOTH interactions must be offered!
    let labels = vm.pie_menu_labels(oid);
    assert_eq!(labels, vec!["Smart Nap", "Manual Only"]);

    // 2. Direct VM guard invocation with autonomous probe `param[0] == 1`:
    //    - Smart Nap rejected while energy == 50, Manual Only always rejected autonomously!
    assert!(
        !vm.eval_guard_tree(oid, smart_nap_guard, 1),
        "autonomous probe must fail when energy == 50"
    );
    assert!(
        !vm.eval_guard_tree(oid, manual_only_guard, 1),
        "Manual Only must reject autonomous probe"
    );
    assert!(
        vm.eval_guard_tree(oid, smart_nap_guard, 0),
        "user probe must pass for Smart Nap"
    );
    assert!(
        vm.eval_guard_tree(oid, manual_only_guard, 0),
        "user probe must pass for Manual Only"
    );

    // 3. Lower energy to 10 (< 20): autonomous probe for Smart Nap now passes!
    vm.world
        .objects
        .get_mut(&sim_id)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .motives[MOTIVE_ENERGY] = 10.0;
    assert!(
        vm.eval_guard_tree(oid, smart_nap_guard, 1),
        "autonomous probe must pass when energy == 10"
    );

    // 4. Lock held (`occupied = true`) must reject BOTH user and autonomous probes!
    vm.world.objects.get_mut(&oid).unwrap().attributes[0] = 1;
    assert!(!vm.eval_guard_tree(oid, smart_nap_guard, 1));
    assert!(!vm.eval_guard_tree(oid, smart_nap_guard, 0));
    assert!(!vm.pie_menu_labels(oid).contains(&"Smart Nap".to_owned()));
}

#[test]
fn tier3_lifecycle_hooks_load_reset_cleanup_objf_slots() {
    let cfg = PackResolverConfig::default();
    let src = r#"
template LifecycleBase {
    state base_loads: i16 = 0;

    load {
        self.base_loads += 1;
    }
}

object "Lifecycle Widget" from LifecycleBase {
    guid: 0x7F50_0030;
    state obj_loads: i16 = 0;
    state resets: i16 = 0;
    state cleanups: i16 = 0;

    load {
        self.obj_loads += 10;
    }

    reset {
        self.resets += 1;
    }

    cleanup {
        self.cleanups += 1;
    }
}
"#;
    let obj = compile_v2_source("lifecycle_widget.antics", src, &cfg).expect("compile lifecycle");
    let report = verify_object_safety(&obj).expect("verify");
    assert!(report.is_ok(), "{:?}", report.violations);

    let iff = Iff::parse(obj.iff_bytes.clone()).expect("parse iff");
    let objf = ObjectFunctions::parse(iff.data(iff.get(b"OBJf", 128).expect("OBJf"))).unwrap();
    assert!(objf.entries.len() >= 5, "OBJf must have slots 0..=4");
    assert!(objf.entries[0].action >= 4096, "OBJf[0] Init");
    assert!(objf.entries[1].action >= 4096, "OBJf[1] Main");
    assert!(objf.entries[2].action >= 4096, "OBJf[2] Load");
    assert!(objf.entries[3].action >= 4096, "OBJf[3] Reset");
    assert!(objf.entries[4].action >= 4096, "OBJf[4] Cleanup");

    // Execute Load (OBJf[2] via real vm.visit), Reset (OBJf[3] via real vm.reset),
    // and Cleanup (OBJf[4] via vm.run_tree) live in sims-vm!
    let mut vm = V2VmFixture::from_objects(&[&obj], 100);
    let oid = vm.spawn(0x7F50_0030, 4, 4);
    let interp = Interpreter::new(&vm.program, &vm.catalog, Stream::Lot);

    interp
        .visit(&mut vm.world, oid, &Default::default())
        .expect("vm.visit should run OBJf[2] Load");
    assert_eq!(vm.obj_attr(oid, 0), 1, "template load ran via vm.visit");
    assert_eq!(vm.obj_attr(oid, 1), 10, "object load ran via vm.visit");

    interp
        .reset(&mut vm.world, oid, false)
        .expect("vm.reset should run OBJf[3] Reset");
    assert_eq!(vm.obj_attr(oid, 2), 1, "reset hook ran via vm.reset");

    assert!(
        interp
            .run_tree(&mut vm.world, oid, objf.entries[4].action, 0, &[], 4_000)
            .unwrap()
    );
    assert_eq!(vm.obj_attr(oid, 3), 1, "cleanup hook ran");
}

#[test]
fn tier3_compound_arithmetic_compact_lowering_and_aliasing_in_vm() {
    let cfg = PackResolverConfig::default();
    let src = r#"
fn calc_chain(a: i16) -> i16 {
    return (a * 24) / 5 + 1;
}

object "Math Lab" {
    guid: 0x7F50_0040;
    state out_chain: i16 = 0;
    state out_self_update: i16 = 10;
    state out_alias_rhs: i16 = 7;
    state out_alias_mul: i16 = 4;
    state out_neg: i16 = 13;
    state out_nested: i16 = 0;
    state out_cond: i16 = 0;

    interaction "Compute" {
        run(sim) {
            let a: i16 = 10;
            // Compact in-place chain: zero scratch locals!
            let chain_let = (a * 24) / 5 + 1;
            self.out_chain = (a * 24) / 5 + 1;

            // Self-updating chain where target is leftmost operand only: 10*24/5+1 = 49
            self.out_self_update = (self.out_self_update * 24) / 5 + 1;

            // Aliasing RHS: (10 + 1) + 7 = 18 (must NOT overwrite out_alias_rhs before reading it!)
            self.out_alias_rhs = (a + 1) + self.out_alias_rhs;

            // Compound aliasing RHS: (4 + 1) * (4 + 2) = 30
            self.out_alias_mul = (self.out_alias_mul + 1) * (self.out_alias_mul + 2);

            // Unary negation aliasing target: -13
            self.out_neg = -self.out_neg;

            // Nested binary + bitwise: (10 + 2) * (9 - 4) + ((10 & 6) | (5 ^ 3)) = 12*5 + (2 | 6) = 66
            let b: i16 = 2;
            let c: i16 = 9;
            let d: i16 = 4;
            self.out_nested = (a + b) * (c - d) + ((a & 6) | (5 ^ 3));

            // Condition + helper return using compound arithmetic
            if chain_let == 49 && calc_chain(10) == (a * 24) / 5 + 1 {
                self.out_cond = 1;
            }

            // Motive assignment must NOT clamp intermediate `* 24` before `/ 5`!
            // 10 * 24 = 240 (would clamp to 100 if done in-place on motive!), 240 / 5 = 48!
            sim.motive.energy = (a * 24) / 5;
        }
    }
}
"#;
    let expanded =
        sims_antics::v2::expand_v2_source_to_objects("math_lab.antics", src, &cfg).expect("expand");
    let air = &expanded[0].air_source;
    // Verify compact in-place lowering for `let chain_let = (a * 24) / 5 + 1;` and `self.out_chain = (a * 24) / 5 + 1;`!
    assert!(
        air.contains("stack.attr[0] := __v_a_0;\n    stack.attr[0] *= 24;\n    stack.attr[0] /= 5;\n    stack.attr[0] += 1;"),
        "expected zero-scratch in-place chain lowering into stack.attr[0]:\n{air}"
    );
    assert!(
        air.contains("stack.attr[1] *= 24;\n    stack.attr[1] /= 5;\n    stack.attr[1] += 1;"),
        "expected self-updating chain without redundant self-copy on stack.attr[1]:\n{air}"
    );

    let obj = compile_v2_source("math_lab.antics", src, &cfg).expect("compile math_lab");
    let mut vm = V2VmFixture::from_objects(&[&obj], 100);
    let oid = vm.spawn(0x7F50_0040, 4, 4);
    vm.run_interaction(oid, "Compute", None);

    assert_eq!(vm.obj_attr(oid, 0), 49, "out_chain");
    assert_eq!(vm.obj_attr(oid, 1), 49, "out_self_update");
    assert_eq!(vm.obj_attr(oid, 2), 18, "out_alias_rhs");
    assert_eq!(vm.obj_attr(oid, 3), 30, "out_alias_mul");
    assert_eq!(vm.obj_attr(oid, 4), -13, "out_neg");
    assert_eq!(vm.obj_attr(oid, 5), 66, "out_nested");
    assert_eq!(vm.obj_attr(oid, 6), 1, "out_cond");
    assert!(
        (vm.sim_motive(MOTIVE_ENERGY) - 48.0).abs() < 1e-4,
        "motive energy must be 48.0 (not prematurely clamped to 100/5=20), got {}",
        vm.sim_motive(MOTIVE_ENERGY)
    );
}

#[test]
fn tier3_compile_time_error_diagnostics() {
    let mut cfg = PackResolverConfig::default();
    cfg.virtual_modules.insert(
        "crate::objects::peer".to_owned(),
        r#"
object "Peer Box" {
    guid: 0x7F50_0099;
    state count: i16 = 0;
}
"#
        .to_owned(),
    );

    let cases: &[(&str, &str, &str)] = &[
        (
            "mismatched_enum_assign",
            r#"
enum A { X = 1 }
enum B { Y = 1 }
object "E1" {
    guid: 0x7F51_0001;
    state a: A = A::X;
    interaction "Go" { run(sim) { self.a = B::Y; } }
}
"#,
            "cannot assign",
        ),
        (
            "enum_assign_raw_i16",
            r#"
enum A { X = 1 }
object "E2" {
    guid: 0x7F51_0002;
    state a: A = A::X;
    interaction "Go" { run(sim) { self.a = 1; } }
}
"#,
            "cannot assign",
        ),
        (
            "mismatched_bitflags_set",
            r#"
bitflags F1 { A = 1 }
bitflags F2 { B = 1 }
object "B1" {
    guid: 0x7F51_0003;
    state f: F1 = F1::None;
    interaction "Go" { run(sim) { self.f.set(F2::B); } }
}
"#,
            "type mismatch",
        ),
        (
            "bitflags_out_of_range_zero",
            r#"
bitflags Bad { Zero = 0 }
object "B2" { guid: 0x7F51_0004; }
"#,
            "1..=16",
        ),
        (
            "unknown_cross_object_field",
            r#"
use crate::objects::peer::PeerBox;
object "X1" {
    guid: 0x7F51_0005;
    interaction "Go" {
        run(sim) {
            for p in objects_of::<PeerBox>() {
                p.missing_field = 1;
            }
        }
    }
}
"#,
            "no state field 'missing_field'",
        ),
        (
            "invalid_match_dialog_arm_for_yes_no",
            r#"
object "D1" {
    guid: 0x7F51_0006;
    interaction "Go" {
        run(sim) {
            match dialog(kind: yes_no, title: "T", message: "M") {
                Button1 => {}
                Button2 => {}
                Button3 => {}
            }
        }
    }
}
"#,
            "invalid for 2-button 'yes_no'",
        ),
        (
            "non_exhaustive_match_dialog",
            r#"
object "D2" {
    guid: 0x7F51_0007;
    interaction "Go" {
        run(sim) {
            match dialog(kind: yes_no_cancel, title: "T", message: "M") {
                Button1 => {}
                Button2 => {}
            }
        }
    }
}
"#,
            "non-exhaustive 'match dialog'",
        ),
        (
            "functional_guard_rejects_is_autonomous",
            r#"
object "G1" {
    guid: 0x7F51_0008;
    functional prep {
        guard(sim, is_auto) => !is_auto,
        run(sim) {}
    }
}
"#,
            "functional guards do not support",
        ),
        (
            "typed_functional_missing_return",
            r#"
object "F1" {
    guid: 0x7F51_0009;
    functional prep -> i16 {
        run(sim) {
            let x = 1;
        }
    }
}
"#,
            "must return a value on all control-flow paths",
        ),
    ];

    for (name, src, expected_substr) in cases {
        let err = compile_v2_source(&format!("{name}.antics"), src, &cfg)
            .expect_err(&format!("expected compile error for {name}"));
        assert!(
            err.message.contains(expected_substr),
            "case '{name}': expected error containing {expected_substr:?}, got: {}",
            err.message
        );
    }
}

#[test]
fn tier3_sol_r1_p0_find_best_object_type_filtering_and_fallback_in_vm() {
    let mut cfg = PackResolverConfig::default();
    let typed_counter_src = r#"
object "Typed Counter" {
    guid: 0x7F52_0001;
    state preps: i16 = 0;

    functional prep -> i16 {
        score: 40,
        run(sim) {
            self.preps += 1;
            return 42;
        }
    }
}
"#;
    let foreign_counter_src = r#"
object "Foreign High Score Counter" {
    guid: 0x7F52_0002;
    state foreign_hits: i16 = 0;

    functional prep -> i16 {
        score: 200,
        run(sim) {
            self.foreign_hits += 1;
            return 999;
        }
    }
}
"#;
    cfg.virtual_modules.insert(
        "crate::objects::typed_counter".to_owned(),
        typed_counter_src.to_owned(),
    );
    let caller_src = r#"
use crate::objects::typed_counter::TypedCounter;

object "Typed Caller" {
    guid: 0x7F52_0003;
    state got_bonus: i16 = 0;
    state missed: i16 = 0;

    interaction "Use Typed Counter" {
        run(sim) {
            let Some(c) = find_best_object::<TypedCounter>(prep) else {
                self.missed += 1;
                return false;
            };
            let Some(bonus) = c.run_functional(prep) else {
                return false;
            };
            self.got_bonus = bonus;
        }
    }
}
"#;
    let typed_counter =
        compile_v2_source("typed_counter.antics", typed_counter_src, &cfg).expect("compile");
    let foreign_counter =
        compile_v2_source("foreign_counter.antics", foreign_counter_src, &cfg).expect("compile");
    let caller = compile_v2_source("typed_caller.antics", caller_src, &cfg).expect("compile");

    // Case 1: Higher-scoring foreign object (score 200) + lower-scoring TypedCounter (score 40):
    // find_best_object::<TypedCounter>(prep) MUST ignore the foreign object and still find TypedCounter!
    let mut vm = V2VmFixture::from_objects(&[&typed_counter, &foreign_counter, &caller], 100);
    let foreign_id = vm.spawn(0x7F52_0002, 4, 4);
    let typed_id = vm.spawn(0x7F52_0001, 5, 4);
    let caller_id = vm.spawn(0x7F52_0003, 6, 4);

    vm.run_interaction(caller_id, "Use Typed Counter", None);
    assert_eq!(
        vm.obj_attr(caller_id, 0),
        42,
        "must call TypedCounter and get 42, not foreign 999"
    );
    assert_eq!(vm.obj_attr(caller_id, 1), 0, "must not miss TypedCounter");
    assert_eq!(vm.obj_attr(typed_id, 0), 1, "TypedCounter must be invoked");
    assert_eq!(
        vm.obj_attr(foreign_id, 0),
        0,
        "foreign object must NOT be invoked as TypedCounter"
    );

    // Case 2: Only foreign object on the lot -> must take the `else` branch cleanly!
    let mut vm2 = V2VmFixture::from_objects(&[&typed_counter, &foreign_counter, &caller], 100);
    let foreign2 = vm2.spawn(0x7F52_0002, 4, 4);
    let caller2 = vm2.spawn(0x7F52_0003, 6, 4);
    vm2.run_interaction(caller2, "Use Typed Counter", None);
    assert_eq!(vm2.obj_attr(caller2, 0), 0);
    assert_eq!(
        vm2.obj_attr(caller2, 1),
        1,
        "must take else branch when only foreign prep object exists"
    );
    assert_eq!(vm2.obj_attr(foreign2, 0), 0);
}

#[test]
fn tier3_sol_r1_p0_aliasing_fault_safety_and_is_empty_loop_in_vm() {
    let mut cfg = PackResolverConfig::default();
    let peer_src = r#"
pub bitflags Flags {
    A = 1,
    B = 2,
}

object "Flag Peer" {
    guid: 0x7F52_0010;
    state flags: Flags = Flags::None;
}
"#;
    cfg.virtual_modules
        .insert("crate::objects::flag_peer".to_owned(), peer_src.to_owned());

    let host_src = r#"
use crate::objects::flag_peer::{Flags, FlagPeer};

object "Alias Host" {
    guid: 0x7F52_0011;
    state n: i16 = 7;
    state divisor: i16 = 0;

    fn fill(f: &mut Flags) {
        while f.is_empty() {
            f.set(Flags::A);
        }
    }

    init {
        self.prep_score = 7;
    }

    load {
        for p in objects_of::<Self>() {
            self.n = 2 + p.n;
        }
    }

    reset {
        self.n = (10 + 2) / self.divisor;
    }

    interaction "Test Aliasing And Loop" {
        run(sim) {
            // Canonical cell alias: self.prep_score == self.data[31] (7 -> 2 + 7 = 9, NOT 4!)
            self.prep_score = 2 + self.data[31];
            // `&mut peer.flags` inside `while f.is_empty()` MUST re-read inside the loop!
            for peer in objects_of::<FlagPeer>() {
                fill(&mut peer.flags);
            }
        }
    }
}
"#;
    let peer = compile_v2_source("flag_peer.antics", peer_src, &cfg).expect("compile peer");
    let host = compile_v2_source("alias_host.antics", host_src, &cfg).expect("compile host");

    let mut vm = V2VmFixture::from_objects(&[&peer, &host], 100);
    let peer_id = vm.spawn(0x7F52_0010, 4, 4);
    let host_id = vm.spawn(0x7F52_0011, 5, 4);

    // 1. Trigger `load` (OBJf 2): `self.n = 2 + p.n` when `p == self` must yield 9 (not 4)!
    {
        let interp = Interpreter::new(&vm.program, &vm.catalog, Stream::Lot);
        interp
            .visit(&mut vm.world, host_id, &Default::default())
            .expect("load");
    }
    assert_eq!(
        vm.obj_attr(host_id, 0),
        9,
        "load self.n = 2 + p.n must not clobber self.n before reading p.n"
    );

    // 2. Run interaction: `self.prep_score = 2 + self.data[31]` -> 9, and `fill(&mut peer.flags)` terminates!
    vm.run_interaction(host_id, "Test Aliasing And Loop", None);
    assert_eq!(
        vm.world.objects[&host_id].data[31], 9,
        "self.prep_score = 2 + self.data[31] must equal 9 (not 4)"
    );
    assert_eq!(
        vm.obj_attr(peer_id, 0),
        1,
        "while f.is_empty() on &mut peer.flags must terminate with Flags::A set"
    );

    // 3. Fault atomicity: `self.n = (10 + 2) / self.divisor` with divisor == 0 traps WITHOUT overwriting `self.n` to 12!
    let interp = Interpreter::new(&vm.program, &vm.catalog, Stream::Lot);
    let fault = interp.reset(&mut vm.world, host_id, false);
    assert!(fault.is_err(), "division by zero must fault in VM");
    assert_eq!(
        vm.obj_attr(host_id, 0),
        9,
        "failed division must NOT leave partial write 12 in self.n"
    );
}

#[test]
fn tier3_sol_r1_p1_dialog_flags_temp_preservation_and_255_cap_in_vm() {
    let cfg = PackResolverConfig::default();
    let src = r#"
object "Dialog Edge" {
    guid: 0x7F52_0020;
    state saved_in_arm: i16 = 0;
    state saved_in_defer: i16 = 0;
    state flags_dispatch: i16 = 0;
    state loop_defers: i16 = 0;

    interaction "Implicit Temp0" {
        run(sim) {
            temp[0] = 77;
            defer {
                self.saved_in_defer = temp[0];
            }
            match dialog(
                kind: yes_no_cancel,
                title: "Pick",
                message: "Choose",
                buttons: ("One", "Two", "Three"),
            ) {
                Button1 => { self.saved_in_arm = temp[0]; }
                Button2 => { self.saved_in_arm = temp[0]; }
                Button3 => { self.saved_in_arm = temp[0]; }
            }
        }
    }

    interaction "Explicit Flags Temp1" {
        run(sim) {
            // flags: 0x10 encodes temp slot 1 in bits 4..6!
            match dialog(
                kind: yes_no_cancel,
                flags: 0x10,
                title: "Pick",
                message: "Flags",
                buttons: ("A", "B", "C"),
            ) {
                Button1 => { self.flags_dispatch = 1; }
                Button2 => { self.flags_dispatch = 2; }
                Button3 => { self.flags_dispatch = 3; }
            }
        }
    }

    interaction "Loop Defer Break" {
        run(sim) {
            repeat 3 {
                defer { self.loop_defers += 1; }
                match dialog(
                    kind: yes_no,
                    title: "Loop",
                    message: "Break?",
                    buttons: ("Stop", "More"),
                ) {
                    Yes => { break; }
                    No => {}
                }
            }
        }
    }
}
"#;
    let obj = compile_v2_source("dialog_edge.antics", src, &cfg).expect("compile");
    let mut vm = V2VmFixture::from_objects(&[&obj], 100);
    let oid = vm.spawn(0x7F52_0020, 4, 4);

    // Implicit 3-way match preserves temp[0] == 77 across ALL 3 answers and inside `defer`!
    for answer in [2u8, 3u8, 4u8] {
        vm.run_interaction_with_dialog(oid, "Implicit Temp0", answer);
        assert_eq!(
            vm.obj_attr(oid, 0),
            77,
            "temp[0] must remain 77 inside arm for answer {answer}"
        );
        assert_eq!(
            vm.obj_attr(oid, 1),
            77,
            "temp[0] must remain 77 inside defer for answer {answer}"
        );
    }

    // Explicit `flags: 0x10` (temp[1]) dispatches Button2 and Button3 accurately!
    vm.run_interaction_with_dialog(oid, "Explicit Flags Temp1", 3);
    assert_eq!(vm.obj_attr(oid, 2), 2);
    vm.run_interaction_with_dialog(oid, "Explicit Flags Temp1", 4);
    assert_eq!(vm.obj_attr(oid, 2), 3);

    // `match dialog` arm with `defer` + `break` inside a loop runs defer cleanly and exits after 1 iteration!
    vm.run_interaction_with_dialog(oid, "Loop Defer Break", 2);
    assert_eq!(vm.obj_attr(oid, 3), 1);

    // 255 explicit entries (including empty placeholder + numeric ref) succeed; 256th auto-interned fails!
    let mut s255 = String::from("object \"StrCap\" {\n    guid: 0x7F52_0021;\n    strings 301 {\n");
    s255.push_str("        s0: \"\",\n");
    for i in 1..255 {
        s255.push_str(&format!("        s{i}: \"entry_{i}\",\n"));
    }
    s255.push_str("    }\n    interaction \"Ok\" {\n        run(sim) {\n");
    // Referencing existing text deduplicates cleanly & numeric ref 0 works at 255 entries!
    s255.push_str("            dialog(kind: ok, title: 0, message: \"entry_1\", inform: true);\n");
    s255.push_str("        }\n    }\n}\n");
    compile_v2_source("str255_ok.antics", &s255, &cfg).expect("255 entries + dedup must succeed");

    let s256 = s255.replacen(
        "message: \"entry_1\"",
        "message: \"brand_new_256th_string\"",
        1,
    );
    let err =
        compile_v2_source("str256_fail.antics", &s256, &cfg).expect_err("256th entry must fail");
    assert!(err.message.contains("255 entries"), "got: {}", err.message);
}

#[test]
fn tier3_sol_r1_p1_enums_bit16_shadowing_multitile_and_lifecycle_cancel_in_vm() {
    let mut cfg = PackResolverConfig::default();
    cfg.virtual_modules.insert(
        "crate::shared::mod_types".to_owned(),
        r#"
pub enum Mode {
    Global = 10,
}

pub enum Sign {
    Neg = -1,
    Pos = 1,
}

pub fn sign_code(s: Sign) -> i16 {
    // Exhaustive enum match WITHOUT `_` must satisfy all-paths-return analysis!
    match s {
        Sign::Neg => { return -10; }
        Sign::Pos => { return 10; }
    }
}
"#
        .to_owned(),
    );
    let multi_src = r#"
object "Dual Tile" {
    guid: 0x7F52_0030;
    master_id: 1;
    tiles: [
        tile { name: "Dual Lead", guid: 0x7F52_0031, objd_id: 129, offset: [0, 0], lead: true },
        tile { name: "Dual Slave", guid: 0x7F52_0032, objd_id: 130, offset: [1, 0], lead: false },
    ];
    state pinged: i16 = 0;
    state peer: Self;
}
"#;
    cfg.virtual_modules
        .insert("crate::objects::dual_tile".to_owned(), multi_src.to_owned());

    let test_src = r#"
use crate::shared::mod_types::{Mode, Sign, sign_code};
use crate::objects::dual_tile::DualTile;

template OrderedLoadBase {
    state step: i16 = 0;
    load {
        self.step = 10;
    }
}

object "Full Coverage" from OrderedLoadBase {
    guid: 0x7F52_0033;

    // Local bitflags shadows module-level `enum Mode` consistently everywhere!
    bitflags Mode {
        Local = 1,
        Top = 16,
    }

    state mode: Mode = Mode::None;
    state sign_val: i16 = 0;
    state load_saw_base: i16 = 0;
    state cancel_sim_seen: i16 = 0;
    state cancel_peer_val: i16 = 0;

    load {
        // Must observe template `load` having already written `self.step = 10`!
        self.load_saw_base = self.step + 5;
    }

    cleanup {
        // Read foreign object via `objects_of::<DualTile>()` AND verify incoming `stack_obj`
        // (cancelling Sim ID!) is preserved right after!
        for bed in objects_of::<DualTile>() {
            bed.pinged += 1;
            self.cancel_peer_val = bed.pinged;
        }
        raw { my.attr[4] := stack_obj; }
    }

    interaction "Run All" {
        run(sim) {
            let local_m: Mode = Mode::Local | Mode::Top;
            self.mode.set(Mode::Top);
            if self.mode.contains(Mode::Top) {
                self.mode.set(Mode::Local);
            }
            if self.mode == local_m {
                self.mode.clear(Mode::Top);
            }
            self.sign_val = sign_code(Sign::Neg);
            for bed in objects_of::<DualTile>() {
                bed.pinged += 7;
            }
        }
    }

    interaction "Slow Action" {
        run(sim) {
            repeat 5 {
                sleep(2);
            }
        }
    }
}
"#;
    let multi_obj = compile_v2_source("dual_tile.antics", multi_src, &cfg).expect("compile dual");
    let full_obj = compile_v2_source("full_cov.antics", test_src, &cfg).expect("compile full");

    let mut vm = V2VmFixture::from_objects(&[&multi_obj, &full_obj], 100);
    let dual_lead = vm.spawn(0x7F52_0030, 3, 3);
    let full_id = vm.spawn(0x7F52_0033, 5, 5);

    // 1. `load` template-before-object ordering:
    {
        let interp = Interpreter::new(&vm.program, &vm.catalog, Stream::Lot);
        interp
            .visit(&mut vm.world, full_id, &Default::default())
            .expect("load");
    }
    assert_eq!(vm.obj_attr(full_id, 0), 10);
    assert_eq!(
        vm.obj_attr(full_id, 3),
        15,
        "object load must observe template load write (10 + 5 = 15)"
    );

    // 2. Bit-16 (`i16::MIN` / `-32768`), local shadowing of module `enum Mode`, exhaustive enum match,
    //    and `objects_of::<DualTile>()` hitting the spawned lead tile!
    vm.run_interaction(full_id, "Run All", None);
    assert_eq!(vm.obj_attr(full_id, 1), 1, "Top cleared, Local(1) remains");
    assert_eq!(vm.obj_attr(full_id, 2), -10, "sign_code(Sign::Neg) == -10");
    assert_eq!(
        vm.obj_attr(dual_lead, 0),
        7,
        "objects_of::<DualTile>() must find spawned lead tile"
    );

    // 3. Real VM queued-action cancellation (`pick` then `cancel`) drives OBJf slot 4 (`cleanup`)
    //    with the cancelling Sim ID in `stack_obj`, and `cleanup` preserves `stack_obj` across
    //    `objects_of::<DualTile>()`!
    {
        let interp = Interpreter::new(&vm.program, &vm.catalog, Stream::Lot);
        let entries = interp
            .pie_menu(&mut vm.world, vm.sim_id, full_id)
            .expect("pie_menu");
        let slow = entries
            .into_iter()
            .find(|e| e.action.table_index == 1)
            .expect("Slow Action menu entry");
        let uid = interp
            .pick(&mut vm.world, vm.sim_id, slow.action)
            .expect("pick")
            .expect("queued");
        assert!(
            interp
                .cancel(&mut vm.world, vm.sim_id, uid)
                .expect("cancel"),
            "cancelling queued action must return true and invoke OBJf[4] Cleanup"
        );
    }
    assert_eq!(
        vm.obj_attr(full_id, 4),
        vm.sim_id as i16,
        "cleanup must preserve cancelling Sim ID in stack_obj after foreign object access"
    );
    assert_eq!(
        vm.obj_attr(full_id, 5),
        8,
        "cleanup must have incremented DualTile.pinged from 7 -> 8"
    );
}

#[test]
fn tier3_sol_r1_diagnostics_p0_p1_p2() {
    let mut cfg = PackResolverConfig::default();
    cfg.virtual_modules.insert(
        "crate::objects::widget".to_owned(),
        r#"
object "Widget" {
    guid: 0x7F52_0040;
    state x: i16 = 0;
    functional prep -> i16 { run(sim) { return 1; } }
}
"#
        .to_owned(),
    );
    cfg.virtual_modules.insert(
        "crate::objects::cyc_a".to_owned(),
        r#"
use crate::objects::cyc_b::CycB;
object "CycA" { guid: 0x7F52_0041; state b: CycB = 0; }
"#
        .to_owned(),
    );
    cfg.virtual_modules.insert(
        "crate::objects::cyc_b".to_owned(),
        r#"
use crate::objects::cyc_a::CycA;
object "CycB" { guid: 0x7F52_0042; state a: CycA = 0; }
"#
        .to_owned(),
    );

    let cases: &[(&str, &str, &str)] = &[
        (
            "create_object_typed_from_record_rejected",
            r#"
use crate::objects::widget::Widget;
object "T1" {
    guid: 0x7F52_0050;
    interaction "Go" {
        run(sim) {
            let Some(w) = create_object::<Widget>(flags: 0x04) else { return false; };
            w.x = 1;
        }
    }
}
"#,
            "FROM_RECORD",
        ),
        (
            "untyped_run_functional_nominal_rejected",
            r#"
use crate::objects::widget::Widget;
object "T2" {
    guid: 0x7F52_0051;
    state raw_id: ObjectId = 0;
    interaction "Go" {
        run(sim) {
            let Some(w) = self.raw_id.run_functional::<Widget>(prep) else { return false; };
            w.x = 1;
        }
    }
}
"#,
            "cannot safely manufacture nominal type",
        ),
        (
            "match_dialog_raw_inform_flag_rejected",
            r#"
object "T3" {
    guid: 0x7F52_0052;
    interaction "Go" {
        run(sim) {
            match dialog(kind: yes_no, flags: 1, title: "T", message: "M") {
                Yes => {}
                No => {}
            }
        }
    }
}
"#,
            "inform bit",
        ),
        (
            "dialog_buttons_arity_mismatch_rejected",
            r#"
object "T4" {
    guid: 0x7F52_0053;
    interaction "Go" {
        run(sim) {
            dialog(kind: yes_no, title: "T", message: "M", buttons: ("A", "B", "C"));
        }
    }
}
"#,
            "requires exactly 2",
        ),
        (
            "match_dialog_arm_after_wildcard_rejected",
            r#"
object "T5" {
    guid: 0x7F52_0054;
    interaction "Go" {
        run(sim) {
            match dialog(kind: yes_no, title: "T", message: "M") {
                _ => {}
                Yes => {}
            }
        }
    }
}
"#,
            "unreachable arm after '_'",
        ),
        (
            "enum_state_missing_init_without_zero_rejected",
            r#"
object "T6" {
    guid: 0x7F52_0055;
    enum NonZero { A = -1, B = 1 }
    state e: NonZero;
}
"#,
            "requires an explicit initializer",
        ),
        (
            "enum_match_non_exhaustive_rejected",
            r#"
object "T7" {
    guid: 0x7F52_0056;
    enum NonZero { A = -1, B = 1 }
    state e: NonZero = NonZero::A;
    interaction "Go" {
        run(sim) {
            match self.e {
                NonZero::A => {}
            }
        }
    }
}
"#,
            "non-exhaustive match on enum",
        ),
        (
            "duplicate_enum_discriminant_rejected",
            r#"
object "T8" {
    guid: 0x7F52_0057;
    enum Dup { A = 1, B = 1 }
}
"#,
            "duplicate discriminant value 1",
        ),
        (
            "duplicate_bitflag_bit_rejected",
            r#"
object "T9" {
    guid: 0x7F52_0058;
    bitflags Dup { A = 16, B = 16 }
}
"#,
            "duplicate bit index 16",
        ),
        (
            "reserved_builtin_height_rejected",
            r#"
object "T10" {
    guid: 0x7F52_0059;
    bitflags Height { Custom = 1 }
}
"#,
            "reserved built-in type name",
        ),
        (
            "enum_autoincrement_overflow_rejected",
            r#"
object "T11" {
    guid: 0x7F52_005A;
    enum Over { Max = 32767, Boom }
}
"#,
            "overflows i16",
        ),
        (
            "mutual_object_import_cycle_rejected",
            r#"
use crate::objects::cyc_a::CycA;
object "T12" {
    guid: 0x7F52_005B;
    state a: CycA = 0;
}
"#,
            "cyclic",
        ),
    ];

    for (name, src, expected_substr) in cases {
        let err = compile_v2_source(&format!("{name}.antics"), src, &cfg)
            .expect_err(&format!("expected compile error for {name}"));
        assert!(
            err.message
                .to_lowercase()
                .contains(&expected_substr.to_lowercase()),
            "case '{name}': expected error containing {expected_substr:?}, got: {}",
            err.message
        );
    }
}

#[test]
fn tier3_sol_r2_regressions() {
    let mut cfg = PackResolverConfig::default();

    // 1. P1-1: Guard-rejected (`occupied = true`) and zero-score `TypedCounter` MUST NOT be bound
    //    by `find_best_object::<TypedCounter>(prep)` fallback when a foreign higher-scoring counter wins!
    let guarded_counter_src = r#"
object "Guarded Counter" {
    guid: 0x7F53_0001;
    state occupied: bool = false;
    state hits: i16 = 0;

    functional prep -> i16 {
        score: 40,
        guard(sim) => !self.occupied,
        run(sim) {
            self.hits += 1;
            return 55;
        }
    }
}
"#;
    let foreign_src = r#"
object "Foreign Counter R2" {
    guid: 0x7F53_0002;
    functional prep -> i16 {
        score: 200,
        run(sim) { return 999; }
    }
}
"#;
    cfg.virtual_modules.insert(
        "crate::objects::guarded_counter".to_owned(),
        guarded_counter_src.to_owned(),
    );
    let caller_src = r#"
use crate::objects::guarded_counter::GuardedCounter;

object "Guarded Caller" {
    guid: 0x7F53_0003;
    state bonus: i16 = 0;
    state missed: i16 = 0;

    interaction "Find Guarded" {
        run(sim) {
            let Some(c) = find_best_object::<GuardedCounter>(prep) else {
                self.missed += 1;
                return false;
            };
            let Some(b) = c.run_functional(prep) else { return false; };
            self.bonus = b;
        }
    }
}
"#;
    let gc = compile_v2_source("guarded_counter.antics", guarded_counter_src, &cfg).unwrap();
    let fc = compile_v2_source("foreign_r2.antics", foreign_src, &cfg).unwrap();
    let caller = compile_v2_source("guarded_caller.antics", caller_src, &cfg).unwrap();

    let mut vm = V2VmFixture::from_objects(&[&gc, &fc, &caller], 100);
    let _foreign_id = vm.spawn(0x7F53_0002, 4, 4);
    let gc_id = vm.spawn(0x7F53_0001, 5, 4);
    let caller_id = vm.spawn(0x7F53_0003, 6, 4);

    // Mark GuardedCounter occupied: fallback MUST skip it and take `else`!
    vm.world.objects.get_mut(&gc_id).unwrap().attributes[0] = 1;
    vm.run_interaction(caller_id, "Find Guarded", None);
    assert_eq!(
        vm.obj_attr(caller_id, 1),
        1,
        "occupied GuardedCounter must be rejected by fallback"
    );
    assert_eq!(vm.obj_attr(gc_id, 1), 0);

    // Unoccupy it but set prep_score == 0: fallback MUST also skip zero-score candidates!
    vm.world.objects.get_mut(&gc_id).unwrap().attributes[0] = 0;
    vm.world.objects.get_mut(&gc_id).unwrap().data[31] = 0;
    vm.run_interaction(caller_id, "Find Guarded", None);
    assert_eq!(
        vm.obj_attr(caller_id, 1),
        2,
        "zero-score GuardedCounter must be rejected by fallback"
    );

    // Restore positive score -> now fallback cleanly binds GuardedCounter!
    vm.world.objects.get_mut(&gc_id).unwrap().data[31] = 40;
    vm.run_interaction(caller_id, "Find Guarded", None);
    assert_eq!(vm.obj_attr(caller_id, 0), 55);
    assert_eq!(vm.obj_attr(gc_id, 1), 1);

    // 2. P1-2: Multi-tile implicit lead selection picks (0, 0) origin tile even when listed second!
    let rev_multi_src = r#"
object "Reversed Multi" {
    guid: 0x7F53_0010;
    master_id: 1;
    tiles: [
        tile { name: "Slave First", guid: 0x7F53_0011, objd_id: 129, offset: [1, 0] },
        tile { name: "Origin Second", guid: 0x7F53_0012, objd_id: 130, offset: [0, 0] },
    ];
    state pinged: i16 = 0;
}
"#;
    cfg.virtual_modules.insert(
        "crate::objects::reversed_multi".to_owned(),
        rev_multi_src.to_owned(),
    );
    let pinger_src = r#"
use crate::objects::reversed_multi::ReversedMulti;

object "Origin Pinger" {
    guid: 0x7F53_0013;
    interaction "Ping" {
        run(sim) {
            for m in objects_of::<ReversedMulti>() {
                m.pinged += 1;
            }
        }
    }
}
"#;
    let pinger_exp =
        sims_antics::v2::expand_v2_source_to_objects("pinger.antics", pinger_src, &cfg).unwrap();
    assert!(
        pinger_exp[0].air_source.contains("0x7f530012"),
        "objects_of::<ReversedMulti>() must enumerate origin tile 0x7f530012, not first-listed slave:\n{}",
        pinger_exp[0].air_source
    );

    // 3. P1-3: Side-effecting object data (`self.data[1]` facing) stages through scratch local!
    let facing_src = r#"
object "Safe Data Write" {
    guid: 0x7F53_0020;
    interaction "Set Facing" {
        run(sim) {
            self.data[1] = 2 + 2 - 3;
        }
    }
}
"#;
    let facing_exp =
        sims_antics::v2::expand_v2_source_to_objects("facing.antics", facing_src, &cfg).unwrap();
    assert!(
        !facing_exp[0].air_source.contains("stack.data[1] := 2;"),
        "self.data[1] compound arithmetic must stage in scratch local before writing facing:\n{}",
        facing_exp[0].air_source
    );

    // 4. P2-2: Module helper using `objects_of::<GuardedCounter>()` ignores caller-local `enum GuardedCounter`!
    cfg.virtual_modules.insert(
        "crate::shared::counter_util".to_owned(),
        r#"
use crate::objects::guarded_counter::GuardedCounter;

pub fn count_all() -> i16 {
    let mut n: i16 = 0;
    for c in objects_of::<GuardedCounter>() {
        n += 1;
    }
    return n;
}
"#
        .to_owned(),
    );
    let shadow_caller_src = r#"
use crate::shared::counter_util::count_all;

object "Shadow Caller" {
    guid: 0x7F53_0021;
    enum GuardedCounter { LocalOnly = 1 }
    state total: i16 = 0;
    interaction "Count" {
        run(sim) {
            self.total = count_all();
        }
    }
}
"#;
    compile_v2_source("shadow_caller.antics", shadow_caller_src, &cfg)
        .expect("module helper objects_of::<T> must ignore caller-local enum with same name");

    // 5. P2-1, P2-3, P2-4 diagnostics:
    cfg.virtual_modules.insert(
        "crate::shared::some_enum".to_owned(),
        "pub enum SomeMode { A = 0 }\n".to_owned(),
    );
    let diag_cases: &[(&str, &str, &str)] = &[
        (
            "run_functional_generic_checked_before_annotation",
            r#"
use crate::objects::guarded_counter::GuardedCounter;
object "D1" {
    guid: 0x7F53_0030;
    interaction "Go" {
        run(sim) {
            let Some(c) = find_best_object::<GuardedCounter>(prep) else { return false; };
            let Some(x: i16) = c.run_functional::<bool>(prep) else { return false; };
            let y = x;
        }
    }
}
"#,
            "run_functional::<bool> does not match",
        ),
        (
            "import_alias_reserved_builtin_rejected",
            r#"
use crate::shared::some_enum::SomeMode as Height;
object "D2" { guid: 0x7F53_0031; }
"#,
            "reserved built-in type name",
        ),
        (
            "object_named_self_rejected",
            r#"
object "Self" { guid: 0x7F53_0032; }
"#,
            "reserved built-in type name",
        ),
        (
            "enum_named_object_rejected",
            r#"
object "D3" {
    guid: 0x7F53_0033;
    enum Object { A = 0 }
}
"#,
            "reserved built-in type name",
        ),
        (
            "dialog_text_entry_two_buttons_rejected",
            r#"
object "D4" {
    guid: 0x7F53_0034;
    interaction "Go" {
        run(sim) {
            dialog(kind: text_entry, title: "T", message: "M", buttons: ("Save", "Cancel"));
        }
    }
}
"#,
            "'text_entry' dialog requires exactly 1",
        ),
        (
            "dialog_icon_two_buttons_rejected",
            r#"
object "D5" {
    guid: 0x7F53_0035;
    interaction "Go" {
        run(sim) {
            dialog(kind: icon, title: "T", message: "M", buttons: ("A", "B"));
        }
    }
}
"#,
            "'icon' dialog requires exactly 1",
        ),
    ];
    for (name, src, expected) in diag_cases {
        let err = compile_v2_source(&format!("{name}.antics"), src, &cfg)
            .expect_err(&format!("expected compile error for {name}"));
        assert!(
            err.message.contains(expected),
            "case '{name}': expected {expected:?}, got: {}",
            err.message
        );
    }
}

// ---------------------------------------------------------------------------
// PR C: Maxis interop (`extern mod` / `extern bcon`), sparse states, named BCON
// tables, and standalone `tree` declarations. All fixtures are synthetic.
// ---------------------------------------------------------------------------

const INTEROP_LAMP_V2: &str = r#"
extern mod light_globals {
    @id(256) fn add(a: i16, b: i16) -> i16 @ temp[0];
    @id(257) fn is_dark(level: i16 @ temp[4]) -> bool;
    @id(258) fn try_light(power: i16) -> Result;
}

extern mod lamp_semi {
    @id(8192) fn doubled(x: i16) -> Result<i16 @ temp[1]>;
}

extern bcon light_consts {
    table_height_flag @ 16521;
    by_chunk @ 257[3];
}

object "Interop Lamp" {
    guid: 0x7F30_0001;
    state quality: i16 = 3;
    state glow @ 9: i16;
    state sum_lit: i16;
    state sum_tmp: i16;
    state dark: bool;
    state flag: i16;
    state doubled: i16;
    state scored: i16;
    state fridge_hits: i16;

    bcon 4097 "LampTuning" {
        cheap = 10,
        bright = 25,
    }

    @id(4143) tree "CT - Is Fridge" {
        return true;
    }

    tree score "Quality Score" -> i16 @ temp[0] {
        if self.quality > 0 {
            return (self.quality * 24) / 5 + 1;
        } else {
            return 0;
        }
    }

    tree bump "Bump Hits" (sim: Sim, n: i16) {
        self.fridge_hits += n;
    }

    init {
        self.sum_lit = light_globals::add(2, LampTuning.cheap);
        let q = self.quality;
        self.sum_tmp = light_globals::add(q, lamp_tuning.bright);
        if light_globals::is_dark(level: 4) {
            self.dark = true;
        }
        self.flag = light_consts.table_height_flag;
        self.doubled = lamp_semi::doubled(self.quality)?;
        light_globals::try_light(1)?;
        self.scored = score();
        if ct_is_fridge() {
            self.glow = 5;
        }
    }

    interaction "Bump" {
        run(sim) {
            bump(sim, 2);
        }
    }
}
"#;

/// Synthetic stand-ins for Maxis global trees/constants (BHAV 256..=258, BCON 257).
const INTEROP_GLOBAL_AIR: &str = r#"
bcon 257 "LightConsts" = [0, 0, 0, 33, 0, 0, 0, 0, 0, 77];

tree 256 "Add" (args: 2) {
    temp[0] := param[0];
    temp[0] += param[1];
    return true;
}

tree 257 "Is Dark" {
    if temp[4] > 3 {
        return true;
    } else {
        return false;
    }
}

tree 258 "Try Light" (args: 1) {
    if param[0] > 0 {
        return true;
    } else {
        return false;
    }
}
"#;

/// Synthetic semi-global library (BHAV 8192).
const INTEROP_SEMI_AIR: &str = r#"
tree 8192 "Doubled" (args: 1) {
    temp[1] := param[0];
    temp[1] *= 2;
    return true;
}
"#;

const INTEROP_PROBE_V2: &str = r#"
object "Interop Probe" {
    guid: 0x7F30_0002;
    state quality: i16 = 4;
    state flag: i16;
    state got: i16;

    extern mod g {
        @id(258) fn try_light(power: i16) -> Result;
    }
    extern mod s {
        @id(8192) fn doubled(x: i16) -> Result<i16 @ temp[1]>;
    }

    @id(4200) tree probe "Probe" (sim: Sim, p: i16) -> Result {
        g::try_light(p)?;
        self.flag = 1;
        let d = s::doubled(self.quality)?;
        self.got = d + p;
    }
}
"#;

fn find_call(
    bhav: &sims_antics::ir::BhavChunk,
    opcode: u16,
) -> &sims_formats::behavior::Instruction {
    bhav.instructions
        .iter()
        .find(|i| i.opcode == opcode)
        .unwrap_or_else(|| panic!("no call to {opcode} in '{}'", bhav.label))
}

#[test]
fn maxis_interop_extern_calls_bcon_tables_sparse_states_and_trees_lower_exactly() {
    let cfg = PackResolverConfig::default();
    let expanded =
        sims_antics::v2::expand_v2_source_to_objects("lamp.antics", INTEROP_LAMP_V2, &cfg)
            .expect("expand interop lamp");
    let air = &expanded[0].air_source;

    // Sparse state: `glow @ 9` keeps its exact slot, auto states fill 0..=7, attributes = 10,
    // and the uninitialized explicit slot is NOT zeroed by init.
    assert!(
        air.contains("attr[7] = fridge_hits, attr[9] = glow"),
        "attribute map:\n{air}"
    );
    assert_eq!(expanded[0].manifest.attributes, 10);
    assert!(!air.contains("my.attr[9] := 0;"), "{air}");
    assert!(
        air.contains("my.attr[7] := 0;"),
        "auto states still zero:\n{air}"
    );
    assert!(air.contains("my.attr[9] := 5;"), "{air}");

    // Named private BCON table (list form, numeric operands) + extern bcon (no chunk).
    assert!(
        air.contains("bcon 4097 \"LampTuning\" = [10, 25];"),
        "{air}"
    );
    assert!(
        air.contains("temp[1] := bcon[129];"),
        "lamp_tuning.bright:\n{air}"
    );
    assert!(air.contains("my.attr[4] := bcon[16521];"), "{air}");
    assert!(
        !air.contains("bcon 257"),
        "extern bcon must not emit a chunk:\n{air}"
    );

    // extern mod: literal call (const args incl. a private BCON value), temps call, temp
    // binding, `?` on Result / Result<T @ temp[1]>, and direct return capture.
    for needle in [
        "if call 256(2, 10) {\n    }\n    my.attr[1] := temp[0];",
        "temp[0] := my.attr[0];\n    temp[1] := bcon[129];\n    if call 256(temps) {\n    }\n    my.attr[2] := temp[0];",
        "temp[4] := 4;\n    if call 257(temps) {\n        my.attr[3] := 1;\n    }",
        "temp[0] := my.attr[0];\n    if !call 8192(temps) {\n        return error;\n    }\n    my.attr[5] := temp[1];",
        "if !call 258(1) {\n        return error;\n    }",
        "if call 4098(temps) {\n    }\n    my.attr[6] := temp[0];",
        "if call 4143(temps) {\n        my.attr[9] := 5;\n    }",
    ] {
        assert!(air.contains(needle), "missing:\n{needle}\n---\n{air}");
    }

    // Standalone trees: pinned id, derived ident, real (non-inlined) calls, Sim tree params.
    assert!(
        air.contains("tree 4143 \"CT - Is Fridge\" {\n    return true;\n}"),
        "{air}"
    );
    assert!(
        air.contains(
            "tree 4098 \"Quality Score\" {\n    if my.attr[0] > 0 {\n        temp[0] := my.attr[0];\n        temp[0] *= 24;\n        temp[0] /= 5;\n        temp[0] += 1;\n    } else {\n        temp[0] := 0;\n    }\n    return true;\n}"
        ),
        "{air}"
    );
    assert!(
        air.contains("tree 4099 \"Bump Hits\" (args: 1) {\n    stack.attr[7] += param[0];"),
        "{air}"
    );
    assert!(air.contains("if call 4099(2) {\n    }"), "{air}");

    // Bytecode shapes.
    let obj = compile_v2_source("lamp.antics", INTEROP_LAMP_V2, &cfg).expect("compile lamp");
    let init = &obj.behaviors[&4096];
    let lit = find_call(init, 256);
    assert_eq!(
        lit.operands,
        [2, 0, 10, 0, 0, 0, 0, 0],
        "literal call operands"
    );
    assert_eq!(
        lit.on_true, lit.on_false,
        "ignored result: both exits continue"
    );
    let semi = find_call(init, 8192);
    assert_eq!(semi.operands, [0xFF; 8], "call(temps) operands");
    assert_eq!(
        semi.on_false, 0xFD,
        "'?' failure exits through the error branch"
    );
    let res = find_call(init, 258);
    assert_eq!(res.operands, [1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(res.on_false, 0xFD);
    let ct = find_call(init, 4143);
    assert_ne!(ct.on_true, ct.on_false, "condition call branches");
    // `(self.quality * 24) / 5 + 1` in place into temp[0]: compare + 4 arith + else = 6 nodes.
    assert_eq!(obj.behaviors[&4098].instructions.len(), 6);
    assert_eq!(obj.behaviors[&4143].instructions.len(), 1);
    assert_eq!(obj.behaviors[&4099].args, 1);

    let iff = Iff::parse(obj.iff_bytes.clone()).expect("IFF parse");
    let bcon = iff.get(b"BCON", 4097).expect("BCON 4097 emitted");
    assert_eq!(constants(iff.data(bcon)).unwrap(), vec![10, 25]);
    assert!(iff.get(b"BCON", 257).is_none());

    // Live VM: synthetic global + semi-global libraries.
    let probe = compile_v2_source("probe.antics", INTEROP_PROBE_V2, &cfg).expect("compile probe");
    let mut vm = V2VmFixture::from_objects_with_libs(
        &[&obj, &probe],
        5000,
        Some(INTEROP_GLOBAL_AIR),
        Some(INTEROP_SEMI_AIR),
    );
    let lamp = vm.spawn(0x7F30_0001, 4, 4);
    assert_eq!(vm.obj_attr(lamp, 0), 3, "quality");
    assert_eq!(vm.obj_attr(lamp, 1), 12, "add(2, LampTuning.cheap)");
    assert_eq!(
        vm.obj_attr(lamp, 2),
        28,
        "add(q, lamp_tuning.bright) via call(temps)"
    );
    assert_eq!(vm.obj_attr(lamp, 3), 1, "is_dark(level: 4) via temp[4]");
    assert_eq!(
        vm.obj_attr(lamp, 4),
        77,
        "bcon[16521] = global BCON 257 entry 9"
    );
    assert_eq!(vm.obj_attr(lamp, 5), 6, "semi-global doubled(3)?");
    assert_eq!(vm.obj_attr(lamp, 6), 15, "score() = (3 * 24) / 5 + 1");
    assert_eq!(vm.obj_attr(lamp, 9), 5, "CT marker tree as condition");
    vm.run_interaction(lamp, "Bump", None);
    assert_eq!(
        vm.obj_attr(lamp, 7),
        2,
        "Sim-context tree called from interaction"
    );

    // `?` propagation: failure exits through the error branch before touching state.
    let p = vm.spawn(0x7F30_0002, 6, 4);
    let err = vm
        .try_run_tree(p, 4200, 0)
        .expect_err("try_light(0)? must exit via error");
    assert!(format!("{err:?}").contains("ErrorBranch"), "{err:?}");
    assert_eq!(vm.obj_attr(p, 1), 0);
    assert!(vm.try_run_tree(p, 4200, 5).expect("success path"));
    assert_eq!(vm.obj_attr(p, 1), 1);
    assert_eq!(vm.obj_attr(p, 2), 13, "doubled(4)? + p");
}

#[test]
fn maxis_interop_states_trees_and_destroy_flags_lower_compactly() {
    let cfg = PackResolverConfig::default();
    let src = r#"
object "Sparse Holes" {
    guid: 0x7F30_0010;
    state a: i16;
    state b @ 0: i16;
    state c: i16 = 0;
    state d @ 2: bool = true;
    tree sum "Sum" (x: i16, y: i16 @ temp[5]) -> i16 @ temp[0] {
        return x + y;
    }
    main {
        self.a = sum(self.c, y: 3);
        self.c = sum(7, y: self.a);
        destroy_self(no_yield: true, skip_resets: true);
    }
}
"#;
    let expanded = sims_antics::v2::expand_v2_source_to_objects("holes.antics", src, &cfg)
        .expect("expand holes");
    let air = &expanded[0].air_source;
    // Explicit slots are reserved first; auto states take the lowest free slots in order.
    assert!(
        air.contains("// Attributes: attr[0] = b, attr[1] = a, attr[2] = d, attr[3] = c"),
        "{air}"
    );
    assert_eq!(expanded[0].manifest.attributes, 8);
    assert!(
        air.contains("my.attr[1] := 0;"),
        "auto state zeroed:\n{air}"
    );
    assert!(
        air.contains("my.attr[2] := 1;"),
        "explicit slot with initializer:\n{air}"
    );
    assert!(
        !air.contains("my.attr[0] := 0;"),
        "explicit uninitialized slot untouched:\n{air}"
    );
    // Temp-bound params are snapshotted at entry; positional params read `param[i]`.
    assert!(
        air.contains("tree 4098 \"Sum\" (args: 1) {\n    let __p_y_0 = temp[5];"),
        "{air}"
    );
    // Mixed signature with a non-constant positional arg: positional -> temp[0], bound -> temp[5].
    assert!(
        air.contains("temp[0] := my.attr[3];\n    temp[5] := 3;\n    if call 4098(temps) {"),
        "{air}"
    );
    // Constant positional arg stays a literal call operand; the bound arg is still written.
    assert!(
        air.contains("temp[5] := my.attr[1];\n    if call 4098(7) {"),
        "{air}"
    );
    assert!(air.contains("remove_object(me, flags: 3);"), "{air}");
}

const INTEROP_REVIEW_V2: &str = r#"
object "Interop Review" {
    guid: 0x7F30_0030;
    state n: i16 = 1;
    state seen: i16;
    state r: i16;
    state h: i16;
    state cleaned: i16;
    state got: i16;
    state nv: i16;
    state cleaned2: i16;

    tree take "Take" (a: i16, b: i16) -> i16 @ temp[4] {
        return b;
    }
    tree change "Change" -> i16 @ temp[4] {
        self.n = 5;
        return 0;
    }
    tree chk "Check" {
        let error = false;
        return error;
    }
    fn h(error: bool) -> bool {
        return error;
    }

    @id(4210) tree read "Read" (sim: Sim, x: i16) -> Result<i16 @ temp[0]> {
        if x > 0 {
            return x;
        }
        return None;
    }
    @id(4211) tree use_read "Use Read" (sim: Sim, x: i16) -> Result {
        defer {
            self.cleaned += 1;
        }
        let v = read(sim, x)?;
        self.got = v;
    }

    // `cond?;` always takes the error exit, even with a local named `error` in scope.
    @id(4212) tree try_err "Try Err" (sim: Sim) {
        let error = true;
        defer {
            self.cleaned2 += 1;
        }
        (self.n > 100)?;
        return false;
    }
    tree none_val "None Val" -> Result<i16 @ temp[0]> {
        let None = 7;
        return None;
    }

    init {
        let v = none_val()?;
        self.nv = v;
        // Named arguments evaluate left to right in source order.
        self.seen = take(b: self.n, a: change());
        if chk() {
            self.r = 1;
        } else {
            self.r = 2;
        }
        if h(false) {
            self.h = 1;
        } else {
            self.h = 2;
        }
    }
}
"#;

#[test]
fn maxis_interop_review_regressions_run_live() {
    let cfg = PackResolverConfig::default();
    let obj = compile_v2_source("review.antics", INTEROP_REVIEW_V2, &cfg).expect("compile review");
    let mut vm = V2VmFixture::from_objects(&[&obj], 5000);
    let o = vm.spawn(0x7F30_0030, 4, 4);
    assert_eq!(vm.obj_attr(o, 0), 5, "change() ran");
    assert_eq!(vm.obj_attr(o, 1), 1, "b: self.n read before a: change()");
    assert_eq!(
        vm.obj_attr(o, 2),
        2,
        "'return error;' with a local 'error' returns it"
    );
    assert_eq!(
        vm.obj_attr(o, 3),
        2,
        "'return error;' with an 'error' param returns it"
    );

    // `return None;` in a `Result<T>` tree takes the false exit, so the caller's `?` runs the
    // caller's own `defer` before exiting through the error branch.
    let err = vm
        .try_run_tree(o, 4211, 0)
        .expect_err("read(0)? must exit via error");
    assert!(format!("{err:?}").contains("ErrorBranch"), "{err:?}");
    assert_eq!(vm.obj_attr(o, 4), 1, "caller defer ran on '?' failure");
    assert_eq!(vm.obj_attr(o, 5), 0);
    assert!(vm.try_run_tree(o, 4211, 7).expect("success path"));
    assert_eq!(vm.obj_attr(o, 4), 2);
    assert_eq!(vm.obj_attr(o, 5), 7);
    assert_eq!(
        vm.obj_attr(o, 6),
        7,
        "a local named 'None' is returned as a value"
    );
    let err = vm
        .try_run_tree(o, 4212, 0)
        .expect_err("'cond?' with a local 'error' must still exit via error");
    assert!(format!("{err:?}").contains("ErrorBranch"), "{err:?}");
    assert_eq!(vm.obj_attr(o, 7), 1, "defer ran before the error exit");

    // Rebinding the Sim name turns `actor.count` into a foreign read that swaps stack_obj, so the
    // tree must save its own object first and write through the saved handle.
    let src = r#"
object "Rebind" {
    guid: 0x7F30_0031;
    state count: i16;
    tree copy "Copy" (actor: Sim, other: Self) {
        let actor = other;
        let n = actor.count;
        self.count = n + 1;
    }
    extern bcon ext { first @ 257[0]; second @ 257[1]; }
    tree take "Take" (a: i16 @ temp[0], b: i16 @ temp[1]) {
    }
    main {
        take(b: ext.second, a: 11);
    }
}
"#;
    let expanded = sims_antics::v2::expand_v2_source_to_objects("rebind.antics", src, &cfg)
        .expect("expand rebind");
    let air = &expanded[0].air_source;
    assert!(air.contains("let __self_obj = stack_obj;"), "{air}");
    // Runtime BCON arguments are read (and may fault) before any temp is staged.
    let first_read = air.find("= bcon[").expect("runtime bcon snapshot");
    let first_stage = air.find("temp[0] := 11;").expect("staged literal");
    assert!(first_read < first_stage, "{air}");
}

#[test]
fn maxis_interop_negative_diagnostics() {
    let cfg = PackResolverConfig::default();
    let expect_err = |name: &str, src: &str, needle: &str| {
        let err = compile_v2_source(name, src, &cfg).expect_err(&format!("{name} must fail"));
        assert!(err.span.is_some(), "{name}: expected a span: {err}");
        assert!(
            err.message.contains(needle),
            "{name}: expected '{needle}' in: {err}"
        );
    };
    let obj = |members: &str| format!("object \"Neg\" {{\n    guid: 0x7F30_0020;\n{members}\n}}\n");

    expect_err(
        "dup_temp.antics",
        &obj("    extern mod m { @id(256) fn f(a: i16 @ temp[2], b: i16 @ temp[2]); }"),
        "duplicate '@ temp[2]' binding",
    );
    expect_err(
        "temp_range.antics",
        &obj("    extern mod m { @id(256) fn f(a: i16 @ temp[8]); }"),
        "temp slot 8 is out of range",
    );
    expect_err(
        "slot_collide.antics",
        &obj("    state a @ 3: i16;\n    state b @ 3: i16;"),
        "overlaps state 'a'",
    );
    expect_err(
        "bcon_id.antics",
        &obj("    bcon 300 \"Bad\" { a = 1 }"),
        "must be a private BCON chunk id in 4096..=4159",
    );
    expect_err(
        "bhav_id.antics",
        &obj("    extern mod m { @id(4100) fn f(); }"),
        "must be a global (256..=4095) or semi-global (8192..) tree",
    );
    expect_err(
        "pos_temp.antics",
        &obj(
            "    extern mod m { @id(256) fn f(a: i16, b: i16 @ temp[0]); }\n    init { m::f(1, b: 2); }",
        ),
        "collides with positional parameter #0",
    );
    expect_err(
        "unused_result.antics",
        &obj("    extern mod m { @id(256) fn f() -> Result; }\n    init { m::f(); }"),
        "unused Result",
    );
    expect_err(
        "try_value.antics",
        &obj("    extern mod m { @id(256) fn f() -> i16 @ temp[0]; }\n    init { m::f()?; }"),
        "'?' needs a call returning 'bool' or 'Result'",
    );
    expect_err(
        "bcon_write.antics",
        &obj("    bcon 4097 \"T\" { a = 1 }\n    init { T.a = 2; }"),
        "read-only",
    );
    expect_err(
        "sim_tree_ctx.antics",
        &obj("    tree t (sim: Sim) { return true; }\n    init { t(); }"),
        "Sim-on-object context",
    );
    expect_err(
        "tree_paths.antics",
        &obj("    state q: i16;\n    tree t -> i16 @ temp[0] { if self.q > 0 { return 1; } }"),
        "must return on all control-flow paths",
    );
    expect_err(
        "try_defer.antics",
        &obj("    extern mod m { @id(256) fn f() -> Result; }\n    init { defer { m::f()?; } }"),
        "cannot exit out of a 'defer' block",
    );
    expect_err(
        "bcon_addr.antics",
        &obj("    extern bcon c { x @ 30000; }"),
        "out of range",
    );
    expect_err(
        "tree_builtin.antics",
        &obj("    tree sleep { return true; }"),
        "shadows the built-in",
    );
    expect_err(
        "bcon_4096.antics",
        &obj("    param power: i16 = 3;\n    bcon 4096 \"Mine\" { a = 1 }"),
        "reserved for this object's i16 params",
    );
    expect_err(
        "sparse_enum.antics",
        &format!(
            "enum Mode {{ Off = 1, On = 2 }}\n{}",
            obj("    state mode @ 9: Mode;")
        ),
        "requires an explicit initializer",
    );
    expect_err(
        "const_try_defer.antics",
        &obj("    init { defer { true?; } }"),
        "cannot exit out of a 'defer' block",
    );
    expect_err(
        "many_params.antics",
        &obj(&format!(
            "    tree t ({}) {{ }}",
            (0..256)
                .map(|i| format!("p{i}: i16"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        "positional parameters",
    );

    // An inherited tree may not shadow a helper fn of the deriving object's module.
    let mut cfg = PackResolverConfig::default();
    cfg.virtual_modules.insert(
        "test_pack::base".to_owned(),
        "pub template Base {\n    tree check { return false; }\n}\n".to_owned(),
    );
    let src = "use test_pack::base::Base;\nfn check() -> bool { return true; }\nobject \"Derived\" from Base {\n    guid: 0x7F30_0021;\n    state x: i16;\n    init { if check() { self.x = 1; } }\n}\n";
    let err = compile_v2_source("derived.antics", src, &cfg).expect_err("shadowed helper");
    assert!(
        err.message
            .contains("collides with helper function 'check'"),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// PR D: readability, zero-`raw` builtins, and proof-friendly lowering
// ---------------------------------------------------------------------------

const READABILITY_LIVE_V2: &str = r#"
const BASE = 3;

bitflags Marks {
    Low = 1,
    High = 14,
}

object "Counting Frame" {
    guid: 0x7F40_0001;
    const STEP = 2;
    const OTHER_KIND = 0x7F40_0999;
    state fwd: i16;
    state rev: i16;
    state incl: i16;
    state dyn_sum: i16;
    state banded: i16;
    state looped: i16;
    state uninit: i16;
    state tail: i16;
    state is_me: i16;
    state not_other: i16;
    state age: i16;
    state persons: i16;
    state objs: i16;
    state flagged: i16;
    state rnd: i16;
    state hand_empty: i16;
    state limit: i16 = 4;
    state seen_self: i16;

    fn band(x: i16) -> i16 {
        let r = 0;
        match x {
            -5..0 => { r = 1; }
            0..=9 => { r = 2; }
            10..20 => { r = 3; }
            _ => { r = 4; }
        }
        return r;
    }

    fn above(x: i16, lim: i16) -> bool {
        let d = x - lim;
        return d > 0;
    }

    init {
        for i in 0..5 {
            self.fwd += i;
        }
        for i in (0..5).rev() {
            self.rev = self.rev * 2 + i;
        }
        for i in 1..=STEP {
            self.incl += i;
        }
        let n = self.limit;
        for i in BASE..n {
            self.dyn_sum += i;
        }
        let a = band(-3);
        let b = band(0);
        let c = band(15);
        let d = band(20);
        self.banded = a * 1000 + b * 100 + c * 10 + d;
        let k = 0;
        loop {
            k += 1;
            if k == 7 {
                break;
            }
        }
        self.looped = k;
        let u: i16;
        u = 9;
        self.uninit = u;
        if above(self.limit, 3) {
            self.tail = 1;
        }
        if self.is_type(0x7F40_0001) {
            self.is_me = 1;
        }
        if !self.is_type(OTHER_KIND) {
            self.not_other = 1;
        }
        for p in lot.sims() {
            self.persons += 1;
        }
        for o in lot.objects() {
            self.objs += 1;
        }
        self.flags.set(Marks::High);
        if self.flags.contains(Marks::High) {
            self.flagged = 1;
        }
        self.flags.clear(Marks::High);
        if self.flags.contains(Marks::High) {
            self.flagged = 2;
        }
        self.rnd = 5;
        random_into(self.rnd, 1);
    }

    @id(4300) tree probe "Probe" (sim: Sim) {
        self.age = sim.age;
        if sim.person[58] == 25 {
            self.age += 100;
        }
        if sim.hand == 0 {
            self.hand_empty = 1;
        }
        let s = stack_obj;
        if s.is_type(0x7F40_0001) {
            self.seen_self = 1;
        }
    }
}
"#;

#[test]
fn readability_loops_ranges_consts_fields_and_builtins_run_live() {
    let cfg = PackResolverConfig::default();
    let obj = compile_v2_source("counting.antics", READABILITY_LIVE_V2, &cfg)
        .expect("compile counting frame");
    let mut vm = V2VmFixture::from_objects(&[&obj], 5000);
    let o = vm.spawn(0x7F40_0001, 4, 4);
    let attr = |vm: &V2VmFixture, i: usize| vm.obj_attr(o, i);
    assert_eq!(attr(&vm, 0), 10, "for i in 0..5 sums 0..=4");
    assert_eq!(
        attr(&vm, 1),
        98,
        "(0..5).rev() visits 4, 3, 2, 1, 0 in order"
    );
    assert_eq!(attr(&vm, 2), 3, "1..=STEP with an object const bound");
    assert_eq!(
        attr(&vm, 3),
        3,
        "BASE..n with a module const and a runtime bound"
    );
    assert_eq!(
        attr(&vm, 4),
        1234,
        "range match arms (negative, inclusive, exclusive, _)"
    );
    assert_eq!(attr(&vm, 5), 7, "loop {{ }} with break");
    assert_eq!(attr(&vm, 6), 9, "let u: i16; then assign");
    assert_eq!(attr(&vm, 7), 1, "tail-return bool helper as a condition");
    assert_eq!(attr(&vm, 8), 1, "self.is_type(own GUID)");
    assert_eq!(attr(&vm, 9), 1, "!self.is_type(GUID const)");
    assert_eq!(attr(&vm, 11), 1, "lot.sims() visits the one Sim");
    assert_eq!(
        attr(&vm, 12),
        2,
        "lot.objects() visits the Sim and this object"
    );
    assert_eq!(
        attr(&vm, 13),
        1,
        "self.flags set/contains/clear with any bitflags"
    );
    assert_eq!(attr(&vm, 14), 0, "random_into(x, 1) writes 0");
    assert_eq!(attr(&vm, 16), 4);

    assert!(vm.try_run_tree(o, 4300, 0).expect("probe runs"));
    assert_eq!(attr(&vm, 10), 125, "sim.age and sim.person[58]");
    assert_eq!(attr(&vm, 15), 1, "sim.hand is empty");
    assert_eq!(attr(&vm, 17), 1, "stack_obj read + ObjectId.is_type");

    // Proof-friendly shapes: counted loops are one node per iteration, `loop` has no
    // condition node, and tail-return bool helpers inline without a wrapper loop.
    let expanded =
        sims_antics::v2::expand_v2_source_to_objects("counting.antics", READABILITY_LIVE_V2, &cfg)
            .expect("expand counting frame");
    let air = &expanded[0].air_source;
    for needle in [
        // The four `for i` loops share one local (backend coalescing of one source name).
        "let __v_i_0 = -1;\n    while __v_i_0 inc_lt 5 {",
        "__v_i_0 := 5;\n    while __v_i_0 dec_gt -1 {",
        "__v_i_0 := 0;\n    while __v_i_0 inc_lt 3 {",
        "let __range_4 = my.attr[16];\n    __v_i_0 := 2;\n    while __v_i_0 inc_lt __range_4 {",
        // Single-pass helper wrappers unwrap; dead flags go and copies forward.
        "let __v_a_6 = 1;\n    let __v_b_10 = 2;\n    let __v_c_14 = 3;\n    let __v_d_18 = 4;",
        "while true {\n        __v_k_24 += 1;",
        "my.attr[6] := 9;",
        "__v_d_18 -= 3;\n    if __v_d_18 > 0 {\n        my.attr[7] := 1;",
        "if !test_object_type(0x7f400999, object: my.id) {",
        // Loop bodies that cannot read the stack object step `stack_obj` itself.
        "stack_obj := 0;\n    while set_to_next(person, dest: stack_obj) {",
        "stack_obj := 0;\n    while set_to_next(all, dest: stack_obj) {",
        "my.data[8] flag_set 14;",
        "random(my.attr[14], 1);",
        "stack.attr[10] := my.person[58];",
        "if my.slot[0] == 0 {",
    ] {
        assert!(air.contains(needle), "missing:\n{needle}\n---\n{air}");
    }
    assert!(
        !air.contains("__ret_above"),
        "tail helper must not use a wrapper:\n{air}"
    );
    assert!(
        !air.contains("let __v_i_1") && !air.contains("__fn_ret"),
        "{air}"
    );
}

const READABILITY_SHAPE_V2: &str = r#"
enum Mode {
    Quiet = 0,
    Loud = 1,
}

object "Shape Probe" {
    guid: 0x7F40_0010;
    const KIND = 0x7F40_0011;
    const TABLE = 301;
    state n: i16;
    state got: i16;

    fn pick(mode: Mode) {
        if mode == Mode::Loud {
            self.n = 1;
        } else {
            self.n = 2;
        }
    }

    @id(4096) tree pinned "Pinned" {
        return true;
    }
    tree helper "Helper" {
        return true;
    }
    @frame(args: 3, locals: 2, format: 0x8002, revision: 5)
    tree framed "Framed" (a: i16) {
        self.n = a;
    }

    @id(4200) tree builtins "Builtins" (sim: Sim) {
        let icon: i16 = 0;
        let ev: i16;
        push_interaction(menu: 2, object: 1, priority: 50, flags: 4, icon: icon);
        run_tree_by_name(table: TABLE, index: 3, mode: 1, global: true);
        go_to_relative(location: -1, facing: 2, options: 1);
        manage_inventory(op: 1, kind: 2, flags: 3, guid: KIND);
        refresh(target: self, mode: 2);
        action_label(table: TABLE, index: 4);
        show_string(table: TABLE, index: 5);
        set_headline(kind: 1, index: 2, duration: -1, options: 0, stack_obj: true, local: icon);
        special_effect(duration: 10, kind: 1, size: 2, flags: 0, str_index: 3);
        breakpoint();
        breakpoint(self.n);
        if !go_to_relative(location: 0, facing: 1) {
            return false;
        }
        animate(clip: 7, table: 0, flags: 0x10, events: 1);
        animate(clip: 8, table: 0, events: 2) |event| {
            self.got = event;
        }
        animate(clip: 9, table: 0, events: 1, local: ev) |e| {
            if e == 3 {
                break;
            }
        }
        if animate(clip: 10, table: 0) {
            self.n = 4;
        }
        grab();
        drop();
        notify_out_of_idle();
        if find_best_interaction() {
            gosub_found_action();
        }
        if create_object(KIND, mode: 0) {
            self.n = 5;
        }
        create_object(KIND, mode: 0);
        pick(Mode::Loud);
        advert.delta[2] = 10;
        advert.personality[3] = -5;
        advert.minimum[Motive::Energy] = 1;
        sim.person[40] = 3;
        refresh(target: sim, mode: 0);
        random_into(self.n, global[3]);
    }

    init {
        helper();
        if !helper() {
            self.n = 9;
        }
        framed(4);
    }

    @id(4201) tree raw_safe "Raw Safe" (sim: Sim) {
        raw { my.attr[0] := 1; call 256(temps); }
        self.n = 1;
    }
    @id(4202) tree raw_clobber "Raw Clobber" (sim: Sim) {
        raw { set_to_next(all); }
        self.n = 1;
    }
    @id(4203) tree manual_stack "Manual Stack" (sim: Sim) {
        let keep = stack_obj;
        stack_obj = sim;
        stack_obj = keep;
        self.n = 2;
    }
}
"#;

#[test]
fn readability_builtins_animate_frame_and_raw_lower_exactly() {
    let cfg = PackResolverConfig::default();
    // Everything must also encode: the `.air` text below is accepted by the `.air` compiler.
    compile_v2_source("shape.antics", READABILITY_SHAPE_V2, &cfg).expect("compile shape probe");
    let expanded =
        sims_antics::v2::expand_v2_source_to_objects("shape.antics", READABILITY_SHAPE_V2, &cfg)
            .expect("expand shape probe");
    let air = &expanded[0].air_source;
    for needle in [
        // Pinned private IDs are reserved before init/main take the lowest free ones.
        "tree 4097 \"Init\" {",
        "tree 4098 \"Main\" {",
        "tree 4096 \"Pinned\" {",
        // Argless calls pass the caller's temps ([0xFF; 8]).
        "if call 4099(temps) {\n    }\n    if !call 4099(temps) {",
        "tree 4100 \"Framed\" (args: 3, locals: 2, format: 0x8002, revision: 5) {",
        // `icon:` sets flag 0x01 (the icon is read from the named local).
        "push_interaction(menu: 2, object: 1, priority: 50, flags: 5, icon: __v_icon_0);",
        "run_tree_by_name(table: 301, index: 3, mode: 1, global: true);",
        "go_to_relative(location: -1, facing: 2, options: 1);",
        "manage_inventory(op: 1, kind: 2, flags: 3, guid: 0x7f400011);",
        // In a Sim-on-object tree `me` is the Sim; the object is the stack object.
        "refresh(target: stack_obj, mode: 2);",
        "refresh(target: me, mode: 0);",
        // A global range would receive the draw, so it is copied first.
        "= global[3];\n    random(stack.attr[0], __",
        "action_string(table: 301, index: 4);",
        "show_string(table: 301, index: 5);",
        "set_headline(kind: 1, index: 2, duration: -1, options: 0, stack_obj: true, local: __v_icon_0);",
        "special_effect(duration: 10, kind: 1, size: 2, flags: 0, str_index: 3);",
        "breakpoint();\n    breakpoint(stack.attr[0]);",
        "if !go_to_relative(location: 0, facing: 1) {\n        return false;",
        "while !animate(clip: 7, table: 0, flags: 0x10, events: 1) {\n    }",
        "while !animate(clip: 8, table: 0, flags: 0x00, events: 2) {\n        stack.attr[1] := param[0];",
        "while !animate(clip: 9, table: 0, flags: 0x20, events: 1, local: __v_ev_1) {\n        if __v_ev_1 == 3 {\n            break;",
        "if animate(clip: 10, table: 0, flags: 0x00, events: 0) {",
        // `notify_out_of_idle()` pads its operand with 0xFF like the original.
        "op 0x0004 (0, 0, 0, 0, 0, 0, 0, 0);\n    op 0x0005 (0, 0, 0, 0, 0, 0, 0, 0);\n    notify_idle() pad 0xff;",
        // `find_best_interaction()` leaves its pick in stack_obj for `gosub_found_action()`.
        "if op 0x0003 (0, 0, 0, 0, 0, 0, 0, 0) {\n        op 0x001e (0, 0, 0, 0, 0, 0, 0, 0);\n    }",
        // `create_object` puts back the stack object it found, on both edges.
        "= stack_obj;\n    if create_object(0x7f400011, mode: 0, flags: 0x00) {",
        ":= stack_obj;\n    create_object(0x7f400011, mode: 0, flags: 0x00);\n    stack_obj := __cond_2;",
        // `pick(Mode::Loud)`: the literal enum compare folds away.
        ";\n    stack.attr[0] := 1;\n    scope(12, 2) := 10;",
        "scope(28, 3) := -5;\n    scope(29, 5) := 1;",
        "my.person[40] := 3;",
        // A raw block that only writes attributes and calls a BHAV keeps no dead self save.
        "tree 4201 \"Raw Safe\" {\n    my.attr[0] := 1; call 256(temps);\n    stack.attr[0] := 1;",
        "tree 4202 \"Raw Clobber\" {\n    let __self_obj = stack_obj;",
        // Switching away and straight back is dropped by the backend pass, with the saves.
        "tree 4203 \"Manual Stack\" {\n    stack.attr[0] := 2;\n    return true;",
    ] {
        assert!(air.contains(needle), "missing:\n{needle}\n---\n{air}");
    }
    assert!(
        !air.contains("1 == 1"),
        "no always-true condition nodes:\n{air}"
    );
    assert!(
        !air.contains("Mode"),
        "folded compare leaves no enum test:\n{air}"
    );

    // Existing helpers and trees keep their names: a user `fn drop` / `tree grab` wins over
    // the new builtins of the same name.
    let src = r#"
object "Shadow" {
    guid: 0x7F40_0012;
    state n: i16;
    fn drop(x: i16) {
        self.n = x;
    }
    tree grab "Grab" {
        return true;
    }
    init {
        drop(3);
        grab();
    }
}
"#;
    let expanded = sims_antics::v2::expand_v2_source_to_objects("shadow.antics", src, &cfg)
        .expect("expand shadow");
    let air = &expanded[0].air_source;
    assert!(air.contains("my.attr[0] := 3;"), "{air}");
    assert!(air.contains("(temps)"), "{air}");
    assert!(!air.contains("op 0x"), "{air}");
}

#[test]
fn readability_negative_diagnostics() {
    let cfg = PackResolverConfig::default();
    let expect_err = |name: &str, src: &str, needle: &str| {
        let err = compile_v2_source(name, src, &cfg).expect_err(&format!("{name} must fail"));
        assert!(err.span.is_some(), "{name}: expected a span: {err}");
        assert!(
            err.message.contains(needle),
            "{name}: expected '{needle}' in: {err}"
        );
    };
    let obj = |members: &str| {
        format!("object \"Neg\" {{\n    guid: 0x7F40_0020;\n    state n: i16;\n{members}\n}}\n")
    };
    let init = |body: &str| obj(&format!("    init {{\n{body}\n    }}"));
    let sim_tree = |body: &str| obj(&format!("    tree t (sim: Sim) {{\n{body}\n    }}"));

    // Loops and ranges.
    expect_err(
        "range_mut.antics",
        &init("for i in 0..3 { i = 2; }"),
        "range loop variable 'i' is read-only",
    );
    expect_err(
        "range_empty.antics",
        &init("match self.n { 3..3 => { } _ => { } }"),
        "empty range pattern",
    );
    expect_err(
        "range_overlap.antics",
        &init("match self.n { 0..=4 => { } 3..9 => { } _ => { } }"),
        "overlaps an earlier match arm",
    );
    expect_err(
        "range_value_overlap.antics",
        &init("match self.n { 2 => { } 0..5 => { } _ => { } }"),
        "overlaps an earlier match arm",
    );
    expect_err(
        "let_uninit_untyped.antics",
        &init("let x;"),
        "needs a type ('let x: i16;')",
    );

    // Consts.
    expect_err(
        "dup_const.antics",
        &obj("    const A = 1;\n    const A = 2;"),
        "duplicate const",
    );
    expect_err(
        "guid_const_value.antics",
        &obj("    const KIND = 0x7F40_0021;\n    init { self.n = KIND; }"),
        "does not fit in i16; use it where a GUID is expected",
    );

    // @frame.
    expect_err(
        "frame_args.antics",
        &obj("    @frame(args: 1)\n    tree t (a: i16, b: i16) { }"),
        "is below its 2 positional parameter(s)",
    );
    expect_err(
        "frame_format.antics",
        &obj("    @frame(format: 0x8009)\n    tree t { }"),
        "not a supported BHAV format",
    );
    expect_err(
        "frame_on_fn.antics",
        &obj("    @frame(args: 1)\n    fn f() { }"),
        "only allowed on a standalone 'tree'",
    );

    // Builtins.
    expect_err(
        "builtin_positional.antics",
        &sim_tree("show_string(301, 2);"),
        "show_string() only takes named arguments",
    );
    expect_err(
        "builtin_unknown_key.antics",
        &sim_tree("show_string(table: 301, line: 2);"),
        "unknown show_string() argument 'line'",
    );
    expect_err(
        "builtin_dup_key.antics",
        &sim_tree("show_string(table: 301, table: 302);"),
        "duplicate show_string() argument 'table'",
    );
    expect_err(
        "builtin_range.antics",
        &sim_tree("action_label(table: 301, index: 300);"),
        "action_label index 300 out of range 0..=255",
    );
    expect_err(
        "builtin_runtime.antics",
        &sim_tree("action_label(table: self.n, index: 1);"),
        "action_label table must be a compile-time constant",
    );
    expect_err(
        "builtin_local.antics",
        &sim_tree("push_interaction(menu: 1, icon: self.n);"),
        "push_interaction(icon:) must name an i16 local, an object-handle local",
    );
    expect_err(
        "animate_local.antics",
        &sim_tree("animate(clip: 1, local: 4);"),
        "animate(local:) must name an i16 local variable",
    );
    expect_err(
        "op_args.antics",
        &sim_tree("grab(1);"),
        "grab() takes no arguments",
    );
    expect_err(
        "random_into_target.antics",
        &sim_tree("random_into(3, 4);"),
        "not a valid assignment target",
    );
    expect_err(
        "is_type_on_i16.antics",
        &sim_tree("if self.n.is_type(0x7F40_0020) { }"),
        "'.is_type(...)' needs an object handle",
    );
    expect_err(
        "is_type_runtime_guid.antics",
        &sim_tree("if self.is_type(self.n) { }"),
        "expected a GUID literal (0x...) or a const",
    );
    expect_err(
        "event_readonly.antics",
        &sim_tree("animate(clip: 1, events: 1) |e| { e = 2; }"),
        "the animation event 'e' is read-only",
    );
    expect_err(
        "event_local_readonly.antics",
        &sim_tree("let x: i16;\nanimate(clip: 1, local: x) |e| { e = 2; }"),
        "the animation event 'e' is read-only",
    );
    expect_err(
        "animate_flag_local.antics",
        &sim_tree("animate(clip: 1, flags: 0x20);"),
        "store events in a local; name it with 'local:'",
    );
    expect_err(
        "animate_shadowed_handler.antics",
        &obj("    fn animate(x: i16) { self.n = x; }\n    init { animate(1) |e| { } }"),
        "a user function named 'animate' shadows it",
    );
    expect_err(
        "push_object_local.antics",
        &sim_tree("push_interaction(menu: 1, flags: 2);"),
        "reads the object from a local slot",
    );
    expect_err(
        "push_icon_flag.antics",
        &sim_tree("push_interaction(menu: 1, flags: 1);"),
        "name it with 'icon:' instead",
    );
    expect_err(
        "effect_local.antics",
        &sim_tree("special_effect(duration: 1, flags: 0x20);"),
        "reads the duration from a local slot",
    );
    expect_err(
        "builtin_guid_const.antics",
        &obj(
            "    const KIND = 0x7F40_0021;\n    tree t (sim: Sim) { action_label(table: KIND, index: 1); }",
        ),
        "const 'KIND' (0x7f400021) does not fit in i16",
    );
    expect_err(
        "range_random_into.antics",
        &init("for i in 0..3 { random_into(i, 2); }"),
        "range loop variable 'i' is read-only",
    );
    expect_err(
        "range_animate_local.antics",
        &sim_tree("for i in 0..3 { animate(clip: 1, local: i); }"),
        "range loop variable 'i' is read-only",
    );
    expect_err(
        "range_nested_bound_borrow.antics",
        &obj(
            "    fn bump(x: &mut i16) -> i16 { *x += 1; return 2; }\n    init { for i in 0..3 { for j in bump(&mut i)..4 { } } }",
        ),
        "range loop variable 'i' is read-only",
    );
    expect_err(
        "frame_format_args.antics",
        &obj("    @frame(format: 0x8000)\n    tree t (a: i16, b: i16, c: i16, d: i16, e: i16) { }"),
        "allows at most 4 arguments, not 5",
    );
    expect_err(
        "frame_format_pinned_args.antics",
        &obj("    @frame(args: 6, format: 0x8001)\n    tree t { }"),
        "allows at most 4 arguments, not 6",
    );

    // Folded conditions still check what they fold away.
    expect_err(
        "fold_if_dead.antics",
        &init("if true { } else { missing(); }"),
        "unknown function 'missing'",
    );
    expect_err(
        "fold_while_dead.antics",
        &init("while 1 == 2 { missing(); }"),
        "unknown function 'missing'",
    );
    expect_err(
        "fold_enum_order.antics",
        &format!(
            "enum E {{ A = 0, B = 1 }}\n{}",
            obj("    init { if E::A < E::B { self.n = 1; } }")
        ),
        "ordering comparison is not supported",
    );

    // Fields.
    expect_err(
        "advert_range.antics",
        &sim_tree("advert.minimum[16] = 1;"),
        "motive index 16 out of 0..16 range",
    );
    expect_err(
        "sim_field.antics",
        &sim_tree("sim.shoes = 1;"),
        "cannot assign to 'sim.shoes'",
    );
    expect_err(
        "sim_hand_write.antics",
        &sim_tree("sim.hand = 0;"),
        "'sim.hand' is read-only",
    );
    expect_err(
        "sim_slot_write.antics",
        &sim_tree("sim.slot[2] = 0;"),
        "'sim.slot[i]' is read-only",
    );
    expect_err(
        "flags_typed.antics",
        &format!(
            "bitflags M {{ A = 1 }}\nbitflags K {{ B = 1 }}\n{}",
            obj("    state m: M;\n    init { self.m.set(K::B); }")
        ),
        "type mismatch",
    );
}

const RANGE_EDGES_V2: &str = r#"
object "Edge Frame" {
    guid: 0x7F40_0030;
    state e1: i16;
    state e2: i16;
    state e3: i16;
    state e4: i16;
    state e5: i16;
    state e6: i16;
    state e7: i16;
    state e8: i16;
    state e9: i16;
    state e10: i16;
    state e11: i16;
    state e12: i16;
    state top: i16 = 32767;
    state bottom: i16 = -32768;
    state one: i16 = 1;
    state five: i16 = 5;
    state four: i16 = 4;

    init {
        for i in 32765..=32767 {
            self.e1 += 1;
        }
        for i in (-32768..-32766).rev() {
            self.e2 = self.e2 * 10 + i + 32767 + 2;
        }
        for i in (0..-32768).rev() {
            self.e3 += 1;
        }
        let hi = self.top;
        for i in 32760..=hi {
            self.e4 += 1;
        }
        let lo = self.bottom;
        for i in (lo..-32765).rev() {
            self.e5 += 1;
        }
        let hi2 = self.five;
        for i in (0..=hi2).rev() {
            if i == 3 {
                continue;
            }
            self.e6 += 1;
        }
        for i in -32768..-32765 {
            self.e7 += 1;
        }
        for i in self.one..3 {
            self.e8 += i;
        }
        for i in (1 + 1)..4 {
            self.e9 += i;
        }
        for i in self.five..=self.four {
            self.e10 += 1;
        }
        for i in 0..=hi {
            if i == 2 {
                break;
            }
            self.e11 += 1;
        }
        while 1 == 1 {
            self.e12 += 1;
            if self.e12 == 2 {
                break;
            }
        }
        while 1 == 2 {
            self.e12 = 99;
        }
    }
}
"#;

#[test]
fn range_edges_and_folded_loops_run_live() {
    let cfg = PackResolverConfig::default();
    let obj = compile_v2_source("edges.antics", RANGE_EDGES_V2, &cfg).expect("compile edge frame");
    let mut vm = V2VmFixture::from_objects(&[&obj], 5000);
    let o = vm.spawn(0x7F40_0030, 4, 4);
    let attr = |i: usize| vm.obj_attr(o, i);
    assert_eq!(attr(0), 3, "32765..=32767 reaches the i16 maximum");
    assert_eq!(
        attr(1),
        21,
        "(-32768..-32766).rev() visits -32767, then -32768"
    );
    assert_eq!(attr(2), 0, "(0..-32768).rev() is empty");
    assert_eq!(attr(3), 8, "32760..=hi with a run-time hi of 32767");
    assert_eq!(
        attr(4),
        3,
        "(lo..-32765).rev() with a run-time lo of -32768"
    );
    assert_eq!(attr(5), 5, "continue in an edge-safe reversed range");
    assert_eq!(attr(6), 3, "-32768..-32765 starts at the i16 minimum");
    assert_eq!(attr(7), 3, "self.one..3 parses as a range");
    assert_eq!(attr(8), 5, "(1 + 1)..4 parses as a range, not .rev()");
    assert_eq!(attr(9), 0, "an empty run-time inclusive range");
    assert_eq!(attr(10), 2, "break in an edge-safe range");
    assert_eq!(
        attr(11),
        2,
        "while 1 == 1 is a loop; while 1 == 2 never runs"
    );

    let expanded =
        sims_antics::v2::expand_v2_source_to_objects("edges.antics", RANGE_EDGES_V2, &cfg)
            .expect("expand edge frame");
    let air = &expanded[0].air_source;
    // A literal range away from the edges keeps the one-node form; the exclusive start at
    // the minimum wraps (32767 + 1 == -32768).
    assert!(air.contains("= 32767;\n    while __v_i_"), "{air}");
    assert!(
        !air.contains("99"),
        "a constant-false loop emits nothing:\n{air}"
    );
    assert!(!air.contains("1 == 1") && !air.contains("1 == 2"), "{air}");
}

#[test]
fn stack_obj_review_cases_keep_the_self_save() {
    let cfg = PackResolverConfig::default();
    let src = r#"
object "Stack Cases" {
    guid: 0x7F40_0031;
    state n: i16;

    fn set_obj(x: &mut ObjectId) {
        *x = 0;
    }

    @id(4300) tree raw_step "Raw Step" (sim: Sim) {
        raw { if stack_obj inc_lt 3 { } }
        self.n = 1;
    }
    @id(4301) tree borrow "Borrow" (sim: Sim) {
        set_obj(&mut stack_obj);
        self.n = 2;
    }
    @id(4302) tree cancel_defer "Cancel Defer" (sim: Sim) {
        defer {
            self.n = 7;
        }
        while true cancel_on_abort {
            sleep(1);
        }
    }
}
"#;
    compile_v2_source("stack.antics", src, &cfg).expect("compile stack cases");
    let expanded = sims_antics::v2::expand_v2_source_to_objects("stack.antics", src, &cfg)
        .expect("expand stack cases");
    let air = &expanded[0].air_source;
    for tree in [
        "\"Raw Step\" {\n    let __self_obj = stack_obj;",
        // The user's own `stack_obj` write stands; the self save is unread and dropped.
        "\"Borrow\" {\n    stack_obj := 0;\n    stack.attr[0] := 2;",
    ] {
        assert!(air.contains(tree), "missing {tree}:\n{air}");
    }
    // A cancelled loop falls through to the deferred block and the tree's own return.
    let cancel = &air[air.find("\"Cancel Defer\"").expect("cancel tree")..];
    let after_loop = &cancel[cancel.find("idle(1);").expect("idle")..];
    assert!(after_loop.contains("stack.attr[0] := 7;"), "{cancel}");
    assert!(after_loop.contains("return true;"), "{cancel}");

    // User functions named like builtins keep winning in statement position too.
    let src = r#"
object "Shadow Two" {
    guid: 0x7F40_0032;
    state n: i16;
    fn animate(x: i16) {
        self.n = x;
    }
    fn create_object(x: i16) {
        self.n = x + 10;
    }
    init {
        animate(3);
        create_object(4);
    }
}
"#;
    let expanded = sims_antics::v2::expand_v2_source_to_objects("shadow2.antics", src, &cfg)
        .expect("expand shadow two");
    let air = &expanded[0].air_source;
    assert!(air.contains("my.attr[0] := 3;"), "{air}");
    assert!(!air.contains("animate("), "{air}");
    assert!(!air.contains("create_object("), "{air}");
}

#[test]
fn shadowed_builtins_folded_checks_and_local_slots() {
    let cfg = PackResolverConfig::default();
    let obj = |members: &str| {
        format!(
            "object \"Round Two\" {{\n    guid: 0x7F40_0040;\n    state n: i16;\n{members}\n}}\n"
        )
    };
    let expand = |name: &str, src: &str| {
        compile_v2_source(name, src, &cfg).unwrap_or_else(|e| panic!("{name}: {e}"));
        sims_antics::v2::expand_v2_source_to_objects(name, src, &cfg)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .remove(0)
            .air_source
    };
    let expect_err = |name: &str, src: &str, needle: &str| {
        let err = compile_v2_source(name, src, &cfg).expect_err(&format!("{name} must fail"));
        assert!(
            err.message.contains(needle),
            "{name}: expected '{needle}' in: {err}"
        );
    };

    // A user `fn animate` that writes stack_obj is followed by the clobber analysis.
    let air = expand(
        "shadow_clobber.antics",
        &obj(
            "    fn animate() { stack_obj = 0; }\n    tree t (sim: Sim) { animate(); self.n = 1; }",
        ),
    );
    // The write stands (the self save it forced is unread, so the backend pass drops it).
    assert!(
        air.contains("stack_obj := 0;\n    stack.attr[0] := 1;"),
        "{air}"
    );

    // User functions win in value position and are not treated as local-writing builtins.
    let air = expand(
        "shadow_value.antics",
        &obj(
            "    fn create_object(x: i16) -> i16 { return x; }\n    init { let n: i16 = create_object(4); self.n = n; }",
        ),
    );
    assert!(!air.contains("create_object("), "{air}");
    expand(
        "shadow_local.antics",
        &obj(
            "    fn animate(local: i16) { self.n = local; }\n    init { for i in 0..3 { animate(local: i); } }",
        ),
    );

    // Builtin iterator names are only iterators in their call syntax.
    expand(
        "iter_name.antics",
        &obj(
            "    init { let objects_by_guid: i16 = 1; for i in objects_by_guid..3 { self.n += i; } }",
        ),
    );

    // Discarded code does not shift later generated names.
    let air = expand(
        "dead_names.antics",
        &obj(
            "    init { while false { let dead = 1; } let live = 1; self.n = live; self.n += live; }",
        ),
    );
    assert!(air.contains("let __v_live_0 = 1;"), "{air}");

    // Modes that read a local name it.
    let air = expand(
        "create_local.antics",
        &obj(
            "    tree t (sim: Sim) { let o: i16 = 0; create_object(0x7F40_0041, mode: local_pos, local: o); }",
        ),
    );
    assert!(
        air.contains("mode: local_pos, flags: 0x00, local: __v_o_0);"),
        "{air}"
    );
    expect_err(
        "create_local_missing.antics",
        &obj("    tree t (sim: Sim) { create_object(0x7F40_0041, mode: local_pos); }"),
        "reads a local; name it with 'local:'",
    );
    expect_err(
        "create_local_unused.antics",
        &obj(
            "    tree t (sim: Sim) { let o: i16 = 0; create_object(0x7F40_0041, mode: here, local: o); }",
        ),
        "does not read 'local:'",
    );
    expect_err(
        "animate_enum_local.antics",
        &format!(
            "enum Ev {{ Z = 0 }}\n{}",
            obj("    tree t (sim: Sim) { let e: Ev = Ev::Z; animate(clip: 1, local: e); }")
        ),
        "animate(local:) must name an i16 local variable",
    );

    // Folded conditions and bodies are still fully checked.
    expect_err(
        "dead_raw.antics",
        &obj("    init { while false { raw { nonexistent; } } }"),
        "invalid .air syntax in raw block",
    );
    expect_err(
        "dead_operand.antics",
        &obj("    init { while false && missing() { } }"),
        "unsupported boolean call expression",
    );
    expect_err(
        "dead_if_operand.antics",
        &obj("    init { if false && missing() { } }"),
        "unsupported boolean call expression",
    );
    expect_err(
        "rev_missing_end.antics",
        &obj("    init { for i in (1..).rev() { } }"),
        "expected expression",
    );
}

// ===========================================================================
// PR E: zero-raw gaps, expression `match`, multi-variant IFF, backend pass
// ===========================================================================

/// The BHAV `tree`'s local count in a compiled object.
fn bhav_locals(obj: &CompiledObject, tree: u16) -> u16 {
    let iff = Iff::parse(obj.iff_bytes.clone()).expect("IFF parse");
    let chunk = iff.get(b"BHAV", tree).expect("BHAV present");
    Behavior::parse(iff.data(chunk)).expect("BHAV parse").locals
}

const PR_E_MATCH_V2: &str = r#"
enum Mode { Off, Low, High }

object "Match Probe" {
    guid: 0x7F40_0110;
    const DEFAULT = 2;
    state mode: Mode;
    state r0: i16;
    state r1: i16;
    state r2: i16;
    state r3: i16;
    state r4: i16;
    state r5: i16;
    state r6: i16;

    fn pick(mode: Mode) -> i16 {
        return match mode { Mode::Off => 0, Mode::Low => 10, Mode::High => 20 };
    }

    fn band(x: i16) -> i16 {
        return match x { -32768..=-1 => -1, 0 => 0, 1..=9 => 1, _ => 2 };
    }

    init {
        self.mode = Mode::Low;
        self.r0 = pick(Mode::High);
        match DEFAULT {
            1 => { self.r1 = 100; }
            2 => { self.r1 = 200; }
            _ => { self.r1 = 300; }
        }
    }

    @id(4300) tree probe "Probe" (sim: Sim, n: i16) {
        self.r2 = band(n);
        self.r3 = match self.mode { Mode::Off => n, Mode::Low => n * 2, Mode::High => n + 100 };
        let big = match n { 0 => self.r3 > 3, _ => n > 5 };
        self.r4 = 0;
        if big {
            self.r4 = 1;
        }
        self.r5 = pick(self.mode);
        self.r6 = 0;
        for i in 0..3 {
            self.r6 += i;
        }
        for i in 0..n {
            self.r6 += 1;
        }
    }
}
"#;

#[test]
fn pr_e_expression_match_and_coalescing_run_live() {
    let cfg = PackResolverConfig::default();
    let obj = compile_v2_source("match.antics", PR_E_MATCH_V2, &cfg).expect("compile match probe");
    let mut vm = V2VmFixture::from_objects(&[&obj], 5000);
    let o = vm.spawn(0x7F40_0110, 4, 4);
    assert_eq!(
        vm.obj_attr(o, 1),
        20,
        "pick(Mode::High) folds to the High arm"
    );
    assert_eq!(
        vm.obj_attr(o, 2),
        200,
        "a const scrutinee selects one statement arm"
    );
    for (n, band, r3, big, r6) in [
        (0, 0, 0, 0, 3),
        (7, 1, 14, 1, 10),
        (-4, -1, -8, 0, 3),
        (12, 2, 24, 1, 15),
    ] {
        assert!(vm.try_run_tree(o, 4300, n).expect("probe runs"), "n = {n}");
        assert_eq!(vm.obj_attr(o, 3), band, "band({n}) by range arms");
        assert_eq!(
            vm.obj_attr(o, 4),
            r3,
            "enum scrutinee, arithmetic arms (n = {n})"
        );
        assert_eq!(vm.obj_attr(o, 5), big, "bool-valued match (n = {n})");
        assert_eq!(vm.obj_attr(o, 6), 10, "pick(self.mode) at runtime");
        assert_eq!(vm.obj_attr(o, 7), r6, "two `for i` loops (n = {n})");
    }

    let air = &sims_antics::v2::expand_v2_source_to_objects("match.antics", PR_E_MATCH_V2, &cfg)
        .expect("expand match probe")[0]
        .air_source;
    // Operand scrutinees are tested in place; constant ones fold.
    assert!(!air.contains("__match"), "{air}");
    assert!(!air.contains("== 2 {"), "the DEFAULT match folds:\n{air}");
    assert!(air.contains("my.attr[1] := 20;"), "{air}");
    assert!(air.contains("my.attr[2] := 200;"), "{air}");
    // The two `i` bindings share a local, and compiler temps share slots with each other.
    assert!(
        !air.contains("let __v_i_") || air.matches("let __v_i_").count() == 1,
        "{air}"
    );
    assert!(bhav_locals(&obj, 4300) <= 4, "{air}");
}

const PR_E_GAPS_V2: &str = r#"
object "Gap Probe" {
    guid: 0x7F40_0120;
    state ages: i16;
    state visitors: i16;
    state kinds: i16;
    state slot0: i16;
    state flag2: i16;
    state counter: i16;
    state twice: i16;
    state seen: i16;

    interaction "Poke" {
        run(sim) {
            return true;
        }
    }

    @id(4300) tree people "People" (sim: Sim) {
        self.ages = 0;
        self.visitors = 0;
        self.kinds = 0;
        for p in lot.sims() {
            self.ages += p.age;
            if p.is_visitor {
                self.visitors += 1;
            }
            self.kinds += p.kind + p.person[12];
            p.person[40] = 7;
        }
    }

    @id(4301) tree sprites "Sprites" (sim: Sim) {
        set_dynamic_sprite(index: 2, value: 1);
        self.sprite_flags[3] = 1;
        self.flag2 = self.sprite_flags[2] + self.sprite_flags[3] * 10;
        self.sprite_flags[3] = 0;
    }

    @id(4304) tree slots "Slots" (sim: Sim) {
        if self.slot[0] == 0 {
            self.slot0 = 1;
        }
    }

    @id(4302) tree pusher "Pusher" (sim: Sim) {
        let target = stack_obj;
        stack_obj = sim;
        target.push_interaction(menu: 0, priority: 2);
    }

    @id(4303) tree counters "Counters" (sim: Sim, n: i16) {
        self.counter = 0;
        while inc_lt(&mut self.counter, 4) {
        }
        param[0] += 5;
        self.twice = n;
        if dec_gt(&mut self.counter, 3) {
            self.seen = 1;
        }
    }
}
"#;

#[test]
fn pr_e_zero_raw_gaps_run_live() {
    let cfg = PackResolverConfig::default();
    let mut obj = compile_v2_source("gaps.antics", PR_E_GAPS_V2, &cfg).expect("compile gaps");
    assert!(
        !PR_E_GAPS_V2.contains("raw"),
        "the probe needs no raw blocks"
    );
    // v2 declares no dynamic sprites; give the definition four so scope 27 has flags to set.
    {
        let iff = Iff::parse(obj.iff_bytes.clone()).expect("IFF parse");
        let objd = iff.get(b"OBJD", 128).expect("OBJD");
        let at = objd.payload.start + 65 * 2;
        obj.iff_bytes[at..at + 2].copy_from_slice(&4i16.to_le_bytes());
    }
    let mut vm = V2VmFixture::from_objects(&[&obj], 5000);
    let o = vm.spawn(0x7F40_0120, 4, 4);
    let sim = vm.sim_id;
    {
        let person = vm
            .world
            .objects
            .get_mut(&sim)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        person.data[32] = 1;
        person.data[65] = 3;
        person.data[12] = 4;
    }

    // Other Sims' person data from a `lot.sims()` loop, read and written.
    assert!(vm.try_run_tree(o, 4300, 0).expect("people runs"));
    assert_eq!(vm.obj_attr(o, 0), 25, "p.age");
    assert_eq!(vm.obj_attr(o, 1), 1, "p.is_visitor");
    assert_eq!(vm.obj_attr(o, 2), 7, "p.kind + p.person[12]");
    assert_eq!(vm.sim_skill(40), 7, "p.person[40] = 7");

    // Dynamic sprite flags (scope 27) and the object's own slot.
    assert!(vm.try_run_tree(o, 4301, 0).expect("sprites runs"));
    assert_eq!(vm.obj_attr(o, 4), 11, "sprite flags 2 and 3 read back");
    assert_eq!(vm.world.objects[&o].dynamic_sprites, vec![0, 0, 1, 0]);

    // `push_interaction` from a local: the object is pinned into an explicit local.
    assert!(vm.try_run_tree(o, 4302, 0).expect("pusher runs"));
    let queue = &vm.world.objects[&sim].person.as_ref().unwrap().queue;
    assert_eq!(queue.len(), 1, "one action queued");
    assert_eq!(
        queue[0].target, o as i16,
        "the local's object is the target"
    );

    // inc_lt / dec_gt and an assignable param.
    assert!(vm.try_run_tree(o, 4303, 2).expect("counters runs"));
    assert_eq!(
        vm.obj_attr(o, 5),
        3,
        "inc_lt to 4, then dec_gt 3 decrements once"
    );
    assert_eq!(
        vm.obj_attr(o, 6),
        7,
        "param[0] += 5 is seen through the named param"
    );
    assert_eq!(vm.obj_attr(o, 7), 0, "dec_gt(4 -> 3, 3) is false");

    let air = &sims_antics::v2::expand_v2_source_to_objects("gaps.antics", PR_E_GAPS_V2, &cfg)
        .expect("expand gaps")[0]
        .air_source;
    for needle in [
        "stack_obj := __v_p_0;\n        let __fdata_1 = stack.person[58];\n        stack_obj := __self_obj;",
        ":= stack.person[32];",
        ":= stack.person[65];",
        "stack_obj := __v_p_0;\n        stack.person[40] := 7;",
        "temp[0] := 2;\n    let __fdata_1 = scope(27, 0);\n    temp[0] := 3;\n    let __arith_2 = scope(27, 0);",
        "temp[0] := 2;\n    scope(27, 0) := 1;",
        // The pinned handle lives in `local[0]` itself.
        "local[0] := stack_obj;\n    stack_obj := my.id;",
        "push_interaction(menu: 0, object: 0, priority: 2, flags: 2);",
        "while stack.attr[5] inc_lt 4 {",
        "param[0] += 5;",
        "if stack.attr[5] dec_gt 3 {",
        "if stack.slot[0] == 0 {",
    ] {
        assert!(air.contains(needle), "missing:\n{needle}\n---\n{air}");
    }
}

const PR_E_VARIANTS_V2: &str = r#"
object "Lamp Line" {
    guid: 0x7F40_0130;
    price: 300;
    state level: i16;
    state kind: i16;

    init {
        self.level = 5;
        self.kind = 1;
    }

    tree init_budget "Init Budget" {
        self.level = 2;
        self.kind = 2;
    }

    variant "Budget Lamp" {
        guid: 0x7F40_0131,
        price: 120,
        description: "A cheaper lamp.",
        init: init_budget,
    }
    variant "Same Lamp" { guid: 0x7F40_0132 }
}
"#;

#[test]
fn pr_e_multi_variant_iff_runs_live() {
    let cfg = PackResolverConfig::default();
    let obj = compile_v2_source("variants.antics", PR_E_VARIANTS_V2, &cfg).expect("compile");
    let iff = Iff::parse(obj.iff_bytes.clone()).expect("IFF parse");
    assert!(iff.map_error().is_none(), "rsmp is rebuilt");
    let word = |id: u16, w: usize| {
        let data = iff.data(iff.get(b"OBJD", id).expect("OBJD"));
        u16::from_le_bytes([data[w * 2], data[w * 2 + 1]])
    };
    let guid = |id: u16| u32::from(word(id, 14)) | (u32::from(word(id, 15)) << 16);
    assert_eq!(guid(128), 0x7F40_0130);
    assert_eq!(guid(129), 0x7F40_0131);
    assert_eq!(guid(130), 0x7F40_0132);
    assert_eq!(
        (word(128, 18), word(129, 18), word(130, 18)),
        (300, 120, 300)
    );
    assert_eq!(
        word(129, 43),
        4098,
        "variant init names the standalone tree"
    );
    assert_eq!(word(130, 43), word(128, 43), "no override shares the init");
    assert_eq!(word(129, 41), word(128, 41) + 1, "own catalog strings");
    let ctss = StringTable::parse(iff.data(iff.get(b"CTSS", word(129, 41)).expect("CTSS")))
        .expect("CTSS parse");
    assert_eq!(ctss.entries[0].text, "Budget Lamp");
    assert_eq!(ctss.entries[1].text, "A cheaper lamp.");
    // One BHAV pool.
    assert_eq!(iff.chunks.iter().filter(|c| &c.tag == b"BHAV").count(), 3);

    let mut vm = V2VmFixture::from_objects(&[&obj], 5000);
    let base = vm.spawn(0x7F40_0130, 4, 4);
    let budget = vm.spawn(0x7F40_0131, 6, 4);
    let same = vm.spawn(0x7F40_0132, 8, 4);
    assert_eq!((vm.obj_attr(base, 0), vm.obj_attr(base, 1)), (5, 1));
    assert_eq!((vm.obj_attr(budget, 0), vm.obj_attr(budget, 1)), (2, 2));
    assert_eq!((vm.obj_attr(same, 0), vm.obj_attr(same, 1)), (5, 1));

    // A source without variants is unchanged.
    let plain = PR_E_VARIANTS_V2
        .split("    variant \"Budget Lamp\"")
        .next()
        .unwrap()
        .to_owned()
        + "}\n";
    let plain = compile_v2_source("plain.antics", &plain, &cfg).expect("compile plain");
    let plain_iff = Iff::parse(plain.iff_bytes.clone()).expect("IFF parse");
    assert_eq!(
        plain_iff
            .chunks
            .iter()
            .filter(|c| &c.tag == b"OBJD")
            .count(),
        1
    );
}

#[test]
fn pr_e_helper_arguments_skip_copies_for_plain_locals() {
    let cfg = PackResolverConfig::default();
    let src = r#"
object "Arg Probe" {
    guid: 0x7F40_0140;
    state a: i16;
    state b: i16;

    fn add(x: i16, y: i16) -> i16 {
        return x + y;
    }
    fn bump(x: i16, r: &mut i16) {
        *r += x;
    }

    init {
        let v = 3;
        let w = 4;
        self.a = add(v, w);
        let k = 5;
        bump(k, &mut k);
        self.b = k;
    }
}
"#;
    let obj = compile_v2_source("args.antics", src, &cfg).expect("compile args");
    let mut vm = V2VmFixture::from_objects(&[&obj], 5000);
    let o = vm.spawn(0x7F40_0140, 4, 4);
    assert_eq!(vm.obj_attr(o, 0), 7);
    assert_eq!(
        vm.obj_attr(o, 1),
        10,
        "the borrowed local is copied before the call"
    );
    let air = &sims_antics::v2::expand_v2_source_to_objects("args.antics", src, &cfg)
        .expect("expand args")[0]
        .air_source;
    assert!(
        !air.contains("__arg_x") || air.matches("__arg_x").count() <= 2,
        "{air}"
    );
    assert!(
        !air.contains("__arg_y"),
        "plain locals are passed in place:\n{air}"
    );
}

#[test]
fn pr_e_negative_diagnostics() {
    let cfg = PackResolverConfig::default();
    let obj = |members: &str| {
        format!(
            "enum Mode {{ Off, Low, High }}\nobject \"Pr E\" {{\n    guid: 0x7F40_0150;\n    state n: i16;\n    state m: Mode;\n{members}\n}}\n"
        )
    };
    let expect_err = |name: &str, src: &str, needle: &str| {
        let err = compile_v2_source(name, src, &cfg).expect_err(&format!("{name} must fail"));
        assert!(
            err.message.contains(needle),
            "{name}: expected '{needle}' in: {err}"
        );
    };
    let init = |body: &str| obj(&format!("    init {{ {body} }}"));
    let sim_tree = |body: &str| obj(&format!("    tree t (sim: Sim) {{ {body} }}"));

    // Expression match.
    expect_err(
        "match_i16_open.antics",
        &init("self.n = match self.n { 0 => 1, 1 => 2 };"),
        "exhaustive",
    );
    expect_err(
        "match_enum_open.antics",
        &init("self.n = match self.m { Mode::Off => 1, Mode::Low => 2 };"),
        "exhaustive",
    );
    expect_err(
        "match_types.antics",
        &init("self.n = match self.n { 0 => 1, _ => true };"),
        "different types",
    );
    expect_err(
        "match_block.antics",
        &init("self.n = match self.n { 0 => { 1 }, _ => 2 };"),
        "not a block",
    );
    expect_err(
        "match_empty.antics",
        &init("self.n = match self.n { };"),
        "at least one arm",
    );

    // Read-only slots and rooms; typed handles have no person data.
    expect_err(
        "slot_write.antics",
        &sim_tree("self.slot[0] = 1;"),
        "read-only",
    );
    expect_err(
        "room_write.antics",
        &sim_tree("for p in lot.sims() { p.room = 1; }"),
        "read-only",
    );
    expect_err(
        "bad_handle_field.antics",
        &sim_tree("for p in lot.sims() { self.n = p.nonsense; }"),
        "unknown field 'p.nonsense'",
    );

    // push_interaction from a handle sets the local flag itself.
    expect_err(
        "push_flag.antics",
        &sim_tree("let o = stack_obj; o.push_interaction(menu: 0, flags: 2);"),
        "implied by an object handle",
    );
    // param[n] needs a frame that has it.
    expect_err("param_init.antics", &init("param[0] = 1;"), "param");
    expect_err(
        "param_range.antics",
        &obj("    tree t (sim: Sim, a: i16) { param[1] = 1; }"),
        "param",
    );
    // inc_lt takes a mutable i16 place.
    expect_err(
        "inc_lt_value.antics",
        &init("if inc_lt(3, 4) { }"),
        "'&mut x'",
    );

    // Variants.
    expect_err(
        "variant_no_guid.antics",
        &obj("    variant \"V\" { price: 10 }"),
        "guid",
    );
    expect_err(
        "variant_dup_guid.antics",
        &obj("    variant \"V\" { guid: 0x7F40_0150 }"),
        "reuses GUID",
    );
    expect_err(
        "variant_dup_name.antics",
        &obj("    variant \"V\" { guid: 0x7F40_0151 }\n    variant \"V\" { guid: 0x7F40_0152 }"),
        "duplicate variant 'V'",
    );
    expect_err(
        "variant_unknown_tree.antics",
        &obj("    variant \"V\" { guid: 0x7F40_0151, init: nowhere }"),
        "nowhere",
    );
    let multi_tile = r#"
object "Two Tiles" {
    guid: 0x7F40_0160;
    master_id: 1;
    tiles: [
        tile { name: "A", guid: 0x7F40_0161, objd_id: 129, offset: [0, 0], lead: true, interaction_group: -1, shape: "cot" },
        tile { name: "B", guid: 0x7F40_0162, objd_id: 130, offset: [1, 0], lead: false, interaction_group: 1, shape: "cot" },
    ];
    variant "V" { guid: 0x7F40_0163 }
}
"#;
    expect_err("variant_tiles.antics", multi_tile, "multi-tile");
    expect_err(
        "variant_dup_field.antics",
        &obj("    variant \"V\" { guid: 0x7F40_0151, guid: 0x7F40_0152 }"),
        "duplicate variant field 'guid'",
    );
}

/// Review fixes: one-arm matches still read the scrutinee, `&mut` sprite cells fix their index
/// at the call, and functionals see the four-word frame.
#[test]
fn pr_e_review_fixes_lower_soundly() {
    let src = r#"
fn bump(flag: &mut i16, i: &mut i16) {
    *i = 0;
    *flag = 1;
}

object "Pr E Review" {
    guid: 0x7F40_0170;
    state n: i16;
    state k: i16;

    @id(4400) tree one_arm "One Arm" (sim: Sim) {
        self.n = match sim.person[12] { _ => 4 };
    }

    @id(4401) tree sprite_ref "Sprite Ref" (sim: Sim) {
        let i = self.k;
        bump(&mut self.sprite_flags[i], &mut i);
        self.n = i;
    }

    @id(4402) tree pinned "Pinned" (sim: Sim) {
        let o = stack_obj;
        stack_obj = sim;
        o.push_interaction(menu: 0, priority: 2);
    }

    functional prep(a: i16) {
        score: 10,
        run(sim) {
            self.n = param[3];
        }
    }
}
"#;
    let expanded = sims_antics::v2::expand_v2_source_to_objects(
        "review.antics",
        src,
        &PackResolverConfig::default(),
    )
    .expect("expand review probe");
    let air = &expanded[0].air_source;
    let tree = |label: &str| {
        let start = air
            .find(&format!("\"{label}\""))
            .unwrap_or_else(|| panic!("tree {label}:\n{air}"));
        let end = air[start..].find("\n}\n").map_or(air.len(), |e| start + e);
        &air[start..end]
    };
    // The wildcard-only match still reads its scrutinee once (a person cell read can fault).
    assert!(tree("One Arm").contains("= my.person[12];"), "{air}");
    // The reference keeps the index the call saw; the helper's `*i = 0` does not move it.
    let sprite = tree("Sprite Ref");
    let select = sprite
        .lines()
        .find(|l| l.trim_start().starts_with("temp[0] := "))
        .unwrap_or_else(|| panic!("{sprite}"));
    assert!(
        select.contains("__ref_") && !select.contains("__v_i_"),
        "{sprite}"
    );
    assert!(
        sprite.find("= stack.attr[1];").expect("snapshot")
            < sprite.find("__v_i_0 := 0;").expect("helper write"),
        "the index snapshot is taken before the helper runs:\n{sprite}"
    );
    assert!(tree("Pinned").contains("local[0] := stack_obj;"), "{air}");
    // `param[3]` is inside a functional's frame, and its header declares the four words.
    assert!(air.contains("param[3]"), "{air}");
    assert!(air.contains("(args: 4)"), "{air}");
    compile_v2_source("review.antics", src, &PackResolverConfig::default())
        .expect("the functional's header covers param[3]");

    // A helper argument bound to a tunable template param is a BCON read, not a constant: the
    // match inside the helper stays a run-time test.
    let tuned = r#"
fn pick(value: i16) -> i16 {
    return match value { 3 => 31, _ => 17 };
}

template Tuned {
    param cycles: i16 = 3;
    state n: i16;
    init {
        self.n = pick(cycles);
    }
}

object "Pr E Tuned" from Tuned {
    guid: 0x7F40_0171;
}
"#;
    let expanded = sims_antics::v2::expand_v2_source_to_objects(
        "tuned.antics",
        tuned,
        &PackResolverConfig::default(),
    )
    .expect("expand tuned probe");
    let air = &expanded[0].air_source;
    assert!(air.contains(":= 31;") && air.contains(":= 17;"), "{air}");

    // Trees whose bytes matter beyond `.air` semantics skip the backend pass: a raw block
    // (its literal calls may read the next instruction), or a literal call to a wide frame.
    let layout = r#"
object "Pr E Layout" {
    guid: 0x7F40_0172;
    state n: i16;

    @id(4410) @frame(args: 6, format: 0x8002) tree wide "Wide" (a: i16) {
        self.n = param[5] + a;
    }

    @id(4411) tree with_raw "With Raw" () {
        let unused = 5;
        raw { my.attr[0] := 1; }
    }

    @id(4412) tree caller "Caller" () {
        wide(1);
        let unused = 5;
    }
}
"#;
    let expanded = sims_antics::v2::expand_v2_source_to_objects(
        "layout.antics",
        layout,
        &PackResolverConfig::default(),
    )
    .expect("expand layout probe");
    let air = &expanded[0].air_source;
    let tree = |label: &str| {
        let start = air
            .find(&format!("\"{label}\""))
            .unwrap_or_else(|| panic!("tree {label}:\n{air}"));
        let end = air[start..].find("\n}\n").map_or(air.len(), |e| start + e);
        air[start..end].to_owned()
    };
    assert!(tree("With Raw").contains(" = 5;"), "{air}");
    assert!(tree("Caller").contains("call 4410(1)"), "{air}");
    assert!(tree("Caller").contains(" = 5;"), "{air}");
}

#[test]
fn pr_e_review_negative_diagnostics() {
    let cfg = PackResolverConfig::default();
    let obj = |members: &str| {
        format!(
            "object \"Pr E Review\" {{\n    guid: 0x7F40_0180;\n    state n: i16;\n{members}\n}}\n"
        )
    };
    let expect_err = |name: &str, src: &str, needle: &str| {
        let err = compile_v2_source(name, src, &cfg).expect_err(&format!("{name} must fail"));
        assert!(
            err.message.contains(needle),
            "{name}: expected '{needle}' in: {err}"
        );
    };
    // Arms must have the same type, not merely assignable ones.
    expect_err(
        "match_handle_i16.antics",
        &obj("    tree t (sim: Sim) { let x = match self.n { 0 => 1, _ => stack_obj }; }"),
        "different types",
    );
    // The pinned push slot and raw .air cannot share a tree, in either order.
    expect_err(
        "pin_then_raw.antics",
        &obj(
            "    tree t (sim: Sim) { let o = stack_obj; o.push_interaction(menu: 0); raw { my.attr[0] := 1; } }",
        ),
        "cannot share a tree",
    );
    expect_err(
        "raw_then_pin.antics",
        &obj(
            "    tree t (sim: Sim) { raw { my.attr[0] := 1; } let o = stack_obj; o.push_interaction(menu: 0); }",
        ),
        "cannot share a tree",
    );
    // Variant trees run as the object's own init/main.
    expect_err(
        "variant_sim_tree.antics",
        &obj("    tree t (sim: Sim) { }\n    variant \"V\" { guid: 0x7F40_0181, init: t }"),
        "object-self tree with no parameters",
    );
    expect_err(
        "variant_primary_name.antics",
        &obj("    variant \"Pr E Review\" { guid: 0x7F40_0181 }"),
        "primary object's name",
    );
    // A variant tree may not read more argument words than the VM's four.
    expect_err(
        "variant_wide_frame.antics",
        &obj(
            "    @frame(args: 5, format: 0x8002)\n    tree v () { }\n    variant \"V\" { guid: 0x7F40_0181, main: v }",
        ),
        "object-self tree with no parameters",
    );
    // A raw block in folded-away code emits nothing, so it does not conflict with the pin.
    compile_v2_source(
        "pin_discarded_raw.antics",
        &obj("    tree t (sim: Sim) { match 0 { 1 => { raw { my.attr[0] := 1; } } _ => { } } let o = stack_obj; o.push_interaction(menu: 0); }"),
        &cfg,
    )
    .expect("discarded raw and a pinned push may share a tree");
    // Nested match scrutinees are bounded like every other nesting.
    let deep = format!(
        "{}self.n{}",
        "match ".repeat(200),
        " { _ => 1 }".repeat(200)
    );
    expect_err(
        "match_deep.antics",
        &obj(&format!("    init {{ self.n = {deep}; }}")),
        "nesting depth",
    );
}

const PR_F_SUGAR_V2: &str = r#"
enum Tone { Off, Low, Mid, High }

object "Sugar Probe" {
    guid: 0x7F40_0210;
    const FLAG = 1;
    state r0: i16;
    state r1: i16;
    state r2: i16;
    state r3: i16;
    state r4: i16;
    state r5: i16;
    state r6: i16;
    state r7: i16;

    @id(4300) tree probe "Probe" (sim: Sim, n: i16) {
        self.r0 = if n > 3 { 7 } else if n == 0 { 1 } else { 2 };
        let wide = if n > 3 || n == -1 { n } else { 4 };
        self.r1 = wide;
        self.r2 = if n > 0 && n < 5 { 1 } else { 0 };
        self.r3 = match n { 0 | 2 | 5 => 1, 3..=4 | 9 => 2, _ => 3 };
        match n {
            1 | 6 => { self.r4 = 10; }
            7 => { self.r4 = 20; }
            _ => { self.r4 = 30; }
        }
        let tone = if n > 4 { Tone::High } else { Tone::Low };
        self.r5 = match tone { Tone::Off | Tone::Low => 1, Tone::Mid | Tone::High => 2 };
        self.r6 = 0;
        for i in 0..4 {
            match i {
                1 | 3 => { self.r6 += 1; }
                2 => { break; }
                _ => { }
            }
        }
        self.r7 = if FLAG == 1 { 5 } else { 6 };
    }
}
"#;

#[test]
fn pr_f_if_expressions_and_or_patterns_run_live() {
    let cfg = PackResolverConfig::default();
    let obj = compile_v2_source("sugar.antics", PR_F_SUGAR_V2, &cfg).expect("compile sugar");
    let mut vm = V2VmFixture::from_objects(&[&obj], 5000);
    let o = vm.spawn(0x7F40_0210, 4, 4);
    // (n, r0, r1, r2, r3, r4, r5)
    for (n, r0, r1, r2, r3, r4, r5) in [
        (0, 1, 4, 0, 1, 30, 1),
        (1, 2, 4, 1, 3, 10, 1),
        (2, 2, 4, 1, 1, 30, 1),
        (4, 7, 4, 1, 2, 30, 1),
        (5, 7, 5, 0, 1, 30, 2),
        (6, 7, 6, 0, 3, 10, 2),
        (7, 7, 7, 0, 3, 20, 2),
        (9, 7, 9, 0, 2, 30, 2),
        (-1, 2, -1, 0, 3, 30, 1),
    ] {
        assert!(vm.try_run_tree(o, 4300, n).expect("probe runs"), "n = {n}");
        assert_eq!(vm.obj_attr(o, 0), r0, "else-if chain (n = {n})");
        assert_eq!(vm.obj_attr(o, 1), r1, "short-circuit || value (n = {n})");
        assert_eq!(vm.obj_attr(o, 2), r2, "short-circuit && value (n = {n})");
        assert_eq!(vm.obj_attr(o, 3), r3, "OR of values and ranges (n = {n})");
        assert_eq!(vm.obj_attr(o, 4), r4, "statement OR arm (n = {n})");
        assert_eq!(vm.obj_attr(o, 5), r5, "enum OR arms (n = {n})");
        assert_eq!(vm.obj_attr(o, 6), 1, "OR arm beside a break arm (n = {n})");
        assert_eq!(vm.obj_attr(o, 7), 5, "a constant condition folds (n = {n})");
    }

    let air = &sims_antics::v2::expand_v2_source_to_objects("sugar.antics", PR_F_SUGAR_V2, &cfg)
        .expect("expand sugar")[0]
        .air_source;
    for needle in [
        // `let x = if ..` writes the binding; `||` / `&&` test in place, in order.
        "let __v_wide_",
        "if !param[0] > 3 {\n            if !param[0] == -1 {",
        // Single-compare OR arms test in a single-pass loop; the arm beside `break` uses a flag.
        "if !param[0] == 1 {\n            if !param[0] == 6 {",
        " != 0 {\n            stack.attr[6] += 1;\n        } else if __v_i_7 == 2 {",
        "stack.attr[7] := 5;",
    ] {
        assert!(air.contains(needle), "missing:\n{needle}\n---\n{air}");
    }
    assert!(!air.contains("__cond"), "{air}");
    assert!(!air.contains("== 1 {\n        stack.attr[7]"), "{air}");
}

const PR_F_SHELF_V2: &str = r#"
object "Shelf F" {
    guid: 0x7F40_0221;
    price: 10;
    state remaining: i16 = 2;

    functional prep {
        score: 50,
        run(sim) {
            if self.remaining <= 0 {
                return false;
            }
            self.remaining -= 1;
            return true;
        }
    }
}
"#;

const PR_F_GAPS_V2: &str = r#"
object "Gap Probe F" {
    guid: 0x7F40_0220;
    sound tick = preset("ui_click", id: 0, wav: "probe_tick");
    sound tock = preset("ui_click", id: 7, wav: "probe_tick");
    sound bell = preset("chime");
    state shelves: i16;
    state vk: i16;
    state ran: i16;
    state steps: i16;
    state banner: i16;
    state spare: i16;

    interaction "Poke" {
        run(sim) {
            play_sound(tock);
            return true;
        }
    }

    init {
        self.shelves = 0;
        for o in lot.objects() {
            if o.is_type(0x7F40_0221) && o.attr[0] == 2 {
                self.shelves += 1;
            }
        }
    }

    @id(4300) tree people "People" (sim: Sim) {
        self.vk = sim.visitor_kind;
        if sim.visitor_kind == 3 {
            self.vk += 10;
        }
    }

    @id(4301) tree shelf "Shelf" (sim: Sim) {
        let Some(t) = find_best_object(prep) else {
            self.ran = -1;
            return false;
        };
        if t.run_functional(prep) {
            self.ran = 1;
        } else {
            self.ran = 2;
        }
    }

    @id(4302) tree stepper "Stepper" (sim: Sim) {
        self.steps = 0;
        stack_obj.attr[5] = 0;
        while inc_lt(&mut stack_obj.attr[5], 3) {
            self.steps += 1;
        }
    }

    @id(4303) tree pusher "Pusher" (sim: Sim) {
        let o = stack_obj;
        stack_obj = sim;
        push_interaction(object: o, menu: 0, priority: 2, icon: o);
    }

    @id(4304) tree banner "Banner" (sim: Sim) {
        advert.delta[2] = 10;
        self.banner = 1;
    }

    @id(4305) @frame(locals: 0, kind: 3)
    tree waves "Waves" (sim: Sim) {
        animate(clip: 3, event_local: 21);
        return true;
    }
}
"#;

#[test]
fn pr_f_zero_raw_and_decompiler_gaps_run_live() {
    let cfg = PackResolverConfig::default();
    let shelf = compile_v2_source("shelf.antics", PR_F_SHELF_V2, &cfg).expect("compile shelf");
    let obj = compile_v2_source("gaps_f.antics", PR_F_GAPS_V2, &cfg).expect("compile gaps");
    assert!(
        !PR_F_GAPS_V2.contains("raw"),
        "the probe needs no raw blocks"
    );

    // Sound id 0, and two ids sharing one sound name.
    let iff = Iff::parse(obj.iff_bytes.clone()).expect("IFF parse");
    let mut fwavs: Vec<(u16, String)> = iff
        .all(b"FWAV")
        .map(|c| {
            let d = iff.data(c);
            let end = d.iter().position(|&b| b == 0).unwrap_or(d.len());
            (c.id, d[..end].iter().map(|&b| char::from(b)).collect())
        })
        .collect();
    fwavs.sort();
    assert_eq!(fwavs.len(), 3, "{fwavs:?}");
    assert_eq!(fwavs[0], (0, "probe_tick".to_owned()), "{fwavs:?}");
    assert!(fwavs.contains(&(7, "probe_tick".to_owned())), "{fwavs:?}");
    assert!(obj.sounds_wav.contains_key("probe_tick"));
    assert_eq!(obj.sounds_wav.len(), 2, "{:?}", obj.sounds_wav.keys());
    assert!(obj.sounds_wav.keys().all(|k| !k.contains("shared")));

    // `@frame(kind:)` and `animate(event_local:)` outside a zero-local frame.
    let waves =
        Behavior::parse(iff.data(iff.get(b"BHAV", 4305).expect("BHAV"))).expect("BHAV parse");
    assert_eq!((waves.kind, waves.locals), (3, 0));
    // The cursor loop in `init` needs no locals.
    assert_eq!(bhav_locals(&obj, 4096), 0);

    let mut vm = V2VmFixture::from_objects(&[&shelf, &obj], 5000);
    let s = vm.spawn(0x7F40_0221, 6, 6);
    let o = vm.spawn(0x7F40_0220, 4, 4);
    assert_eq!(
        vm.obj_attr(o, 0),
        1,
        "lot.objects() with stack_obj as the cursor"
    );
    let sim = vm.sim_id;
    vm.world
        .objects
        .get_mut(&sim)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[32] = 3;

    assert!(vm.try_run_tree(o, 4300, 0).expect("people runs"));
    assert_eq!(vm.obj_attr(o, 1), 13, "sim.visitor_kind is an i16");

    for expected in [1, 1, 2] {
        vm.try_run_tree(o, 4301, 0).expect("shelf runs");
        assert_eq!(
            vm.obj_attr(o, 2),
            expected,
            "run_functional branches directly"
        );
    }
    assert_eq!(vm.obj_attr(s, 0), 0, "the functional ran twice");

    assert!(vm.try_run_tree(o, 4302, 0).expect("stepper runs"));
    assert_eq!(vm.obj_attr(o, 3), 2, "inc_lt on a stack_obj cell");
    assert_eq!(vm.obj_attr(o, 5), 3);

    assert!(vm.try_run_tree(o, 4303, 0).expect("pusher runs"));
    let queue = &vm.world.objects[&sim].person.as_ref().unwrap().queue;
    assert_eq!(queue.len(), 1, "one action queued");
    assert_eq!(queue[0].target, o as i16, "the pinned handle is the target");
    assert_eq!(queue[0].icon, o as i16, "and the icon");

    assert!(vm.try_run_tree(o, 4304, 0).expect("banner runs"));
    assert_eq!(vm.obj_attr(o, 4), 1);

    let air = &sims_antics::v2::expand_v2_source_to_objects("gaps_f.antics", PR_F_GAPS_V2, &cfg)
        .expect("expand gaps")[0]
        .air_source;
    let tree = |label: &str| {
        let start = air
            .find(&format!("\"{label}\""))
            .unwrap_or_else(|| panic!("tree {label}:\n{air}"));
        let end = air[start..].find("\n}\n").map_or(air.len(), |e| start + e);
        &air[start..end]
    };
    for (label, needle) in [
        (
            "Init",
            "stack_obj := 0;\n    while set_to_next(all, dest: stack_obj) {",
        ),
        ("People", "stack.attr[1] := my.person[32];"),
        (
            "Shelf",
            "if run_functional_tree(prep) {\n        stack_obj := __self_obj;",
        ),
        ("Stepper", "while stack.attr[5] inc_lt 3 {"),
        ("Pusher", "local[0] := stack_obj;\n    stack_obj := my.id;"),
        ("Pusher", "flags: 3, icon: 0);"),
        ("Banner", "scope(12, 2) := 10;\n    stack.attr[4] := 1;"),
        ("Waves", "(locals: 0, kind: 3)"),
        (
            "Waves",
            "op 0x002c (0x03, 0x00, 0x15, 0x00, 0x00, 0x20, 0x00, 0x00)",
        ),
    ] {
        assert!(
            tree(label).contains(needle),
            "{label} missing:\n{needle}\n---\n{air}"
        );
    }
    assert!(!tree("Banner").contains("stack_obj"), "{air}");
    assert!(!tree("Init").contains("__saved_stack_obj"), "{air}");
    assert!(!air.contains("__cond"), "{air}");
}

#[test]
fn pr_f_negative_diagnostics() {
    let cfg = PackResolverConfig::default();
    let obj = |members: &str| {
        format!(
            "enum Tone {{ Off, Low, High }}\nobject \"Pr F\" {{\n    guid: 0x7F40_0230;\n    state n: i16;\n    state t: Tone;\n{members}\n}}\n"
        )
    };
    let expect_err = |name: &str, src: &str, needle: &str| {
        let err = compile_v2_source(name, src, &cfg).expect_err(&format!("{name} must fail"));
        assert!(
            err.message.contains(needle),
            "{name}: expected '{needle}' in: {err}"
        );
    };
    let init = |body: &str| obj(&format!("    init {{ {body} }}"));
    let sim_tree = |body: &str| obj(&format!("    tree t (sim: Sim) {{ {body} }}"));

    // `stack_obj` is never a binding.
    for (name, src) in [
        ("bind_let.antics", sim_tree("let stack_obj = 1;")),
        (
            "bind_for.antics",
            sim_tree("for stack_obj in lot.sims() { }"),
        ),
        (
            "bind_param.antics",
            obj("    tree t (sim: Sim, stack_obj: i16) { }"),
        ),
    ] {
        expect_err(name, &src, "'stack_obj' is the frame's stack object");
    }

    // `if` expressions.
    expect_err(
        "if_no_else.antics",
        &init("self.n = if self.n > 0 { 1 };"),
        "needs an 'else' branch",
    );
    expect_err(
        "if_types.antics",
        &init("self.n = if self.n > 0 { 1 } else { true };"),
        "'if' expression branches have different types",
    );
    expect_err(
        "if_folded_types.antics",
        &init("self.n = if 1 == 1 { 1 } else { true };"),
        "'if' expression branches have different types",
    );
    expect_err(
        "if_block.antics",
        &init("self.n = if self.n > 0 { self.n = 2; 1 } else { 0 };"),
        "not a block of statements",
    );

    // OR-patterns.
    expect_err(
        "or_wildcard.antics",
        &init("match self.n { 0 | _ => { } }"),
        "'_' cannot be an alternative",
    );
    expect_err(
        "or_ident.antics",
        &init("match self.n { 0 | other => { } _ => { } }"),
        "unexpected identifier pattern 'other'",
    );
    expect_err(
        "or_duplicate.antics",
        &init("match self.n { 0 | 1 => { } 1 => { } _ => { } }"),
        "duplicate match arm for value 1",
    );
    expect_err(
        "or_self_overlap.antics",
        &init("match self.n { 1..=3 | 2 => { } _ => { } }"),
        "duplicate match arm for value 2",
    );
    expect_err(
        "or_enum_dup.antics",
        &init("match self.t { Tone::Off | Tone::Off => { } _ => { } }"),
        "duplicate match arm for 'Tone::Off'",
    );
    expect_err(
        "or_enum_open.antics",
        &init("self.n = match self.t { Tone::Off | Tone::Low => 1 };"),
        "missing variant 'High'",
    );
    expect_err(
        "or_i16_open.antics",
        &init("self.n = match self.n { 0 | 1 => 1, 2..=9 => 2 };"),
        "non-exhaustive",
    );
    expect_err(
        "or_type.antics",
        &init("match self.t { Tone::Off | 1 => { } _ => { } }"),
        "integer pattern 1 cannot match",
    );
    expect_err(
        "or_dialog_dup.antics",
        &sim_tree("match dialog(message: \"Go?\") { Yes | 0 => { } No => { } }"),
        "duplicate arm for button 1",
    );
    compile_v2_source(
        "or_dialog.antics",
        &sim_tree("match dialog(message: \"Go?\") { Yes | No => { self.n = 1; } }"),
        &cfg,
    )
    .expect("dialog buttons may share an arm");

    // `animate(event_local:)` sits outside a declared frame.
    expect_err(
        "event_local_no_frame.antics",
        &sim_tree("animate(clip: 3, event_local: 5);"),
        "add @frame(locals: K)",
    );
    expect_err(
        "event_local_inside.antics",
        &obj("    @frame(locals: 4)\n    tree t (sim: Sim) { animate(clip: 3, event_local: 2); }"),
        "is inside the frame",
    );
    expect_err(
        "event_local_needed.antics",
        &obj(
            "    @frame(locals: 1)\n    tree t (sim: Sim) { let a = self.n; let b = a + 1; self.n = b; animate(clip: 3, event_local: 1); }",
        ),
        "must lie outside the frame",
    );
    expect_err(
        "event_local_both.antics",
        &obj(
            "    @frame(locals: 1)\n    tree t (sim: Sim) { let e = 0; animate(clip: 3, local: e, event_local: 4); }",
        ),
        "not both",
    );
    expect_err(
        "event_local_body.antics",
        &obj(
            "    @frame(locals: 0)\n    tree t (sim: Sim) { animate(clip: 3, event_local: 4) |ev| { } }",
        ),
        "takes no '|event|' body",
    );
    expect_err(
        "frame_kind.antics",
        &obj("    @frame(kind: 256)\n    tree t (sim: Sim) { }"),
        "@frame kind must fit in a byte",
    );

    // Shared sound names.
    expect_err(
        "sound_no_id.antics",
        &obj(
            "    sound a = preset(\"ui_click\", wav: \"same\");\n    sound b = preset(\"ui_click\", wav: \"same\");",
        ),
        "give it an explicit 'id:'",
    );
    expect_err(
        "sound_preset.antics",
        &obj(
            "    sound a = preset(\"ui_click\", id: 1, wav: \"same\");\n    sound b = preset(\"chime\", id: 2, wav: \"same\");",
        ),
        "has a different preset",
    );
    expect_err(
        "sound_reserved.antics",
        &obj(
            "    sound a = preset(\"ui_click\", id: 1, wav: \"same\");\n    sound b = preset(\"ui_click\", id: 2, wav: \"same\");\n    sound c = preset(\"chime\", id: 3, wav: \"v2-shared-sound-2\");",
        ),
        "reserved for shared sound names",
    );

    // `icon:` from a handle other than `object:` must be a plain local.
    expect_err(
        "icon_foreign.antics",
        &sim_tree("let o = stack_obj; push_interaction(object: o, menu: 0, icon: sim.hand);"),
        "push_interaction(icon:) must name",
    );
}

/// Review regressions for PR F's optimizer and cursor/push/animate lowering: each probe is
/// expanded and its `.air` checked, and each compiles.
#[test]
fn pr_f_review_regressions_lower_soundly() {
    let cfg = PackResolverConfig::default();
    let obj = |members: &str| {
        format!(
            "object \"Pr F review\" {{\n    guid: 0x7F40_0240;\n    state n: i16;\n{members}\n}}\n"
        )
    };
    let tree_of = |name: &str, src: &str, id: u16| -> String {
        compile_v2_source(name, src, &cfg).unwrap_or_else(|e| panic!("{name} must compile: {e}"));
        let expanded = sims_antics::v2::expand_v2_source_to_objects(name, src, &cfg)
            .unwrap_or_else(|e| panic!("{name} must expand: {e}"));
        let air = &expanded[0].air_source;
        let start = air
            .find(&format!("tree {id} "))
            .unwrap_or_else(|| panic!("{name}: no tree {id} in:\n{air}"));
        let end = air[start..].find("\n}\n").map_or(air.len(), |e| start + e);
        air[start..end].to_owned()
    };

    // A raw block with numeric branch targets keeps the entry save, so the targets still land.
    let t = tree_of(
        "raw_targets.antics",
        &obj(
            "    @id(4300) tree t (sim: Sim) {\n        raw {\n            stack_obj := my.id;\n            random(temp[0], 1) -> true: 3, false: 3;\n            stack.attr[0] := 1;\n            return true;\n        }\n    }",
        ),
        4300,
    );
    assert!(t.contains("let __self_obj = stack_obj;"), "{t}");

    // A body that writes sprite flags, or shadows the Sim, never uses `stack_obj` as the cursor.
    let t = tree_of(
        "cursor_sprite.antics",
        &obj("    init { for o in lot.objects() { self.sprite_flags[0] = 1; } }"),
        4096,
    );
    assert!(!t.contains("dest: stack_obj"), "{t}");
    let t = tree_of(
        "cursor_shadow.antics",
        &obj(
            "    @id(4300) tree t (sim: Sim) { for o in lot.objects() { let sim = o; sim.attr[0] = 1; } }",
        ),
        4300,
    );
    assert!(!t.contains("dest: stack_obj"), "{t}");

    // Scope 27 reads the stack object, so the switch before a sprite store stays.
    let t = tree_of(
        "sprite_scope.antics",
        &obj(
            "    @id(4300) tree t (sim: Sim) { stack_obj = sim; temp[0] = 0; stack_obj.sprite_flags[0] = 1; return true; }",
        ),
        4300,
    );
    assert!(t.contains("stack_obj := my.id;"), "{t}");

    // A pinned push of a local read before any write keeps the copy into the pin slot.
    let t = tree_of(
        "pin_uninit.antics",
        &obj(
            "    @id(4300) tree t (sim: Sim) { let o: ObjectId; push_interaction(object: o, menu: 0, priority: 2, icon: o); }",
        ),
        4300,
    );
    assert!(t.contains("local[0] := "), "{t}");

    // The entry save is not forwarded into a local-slot operand.
    let t = tree_of(
        "icon_forward.antics",
        &obj(
            "    @id(4300) tree t (sim: Sim) { push_interaction(object: 0, menu: 0, priority: 2, icon: self); stack_obj = sim; return true; }",
        ),
        4300,
    );
    assert!(!t.contains("icon: stack_obj"), "{t}");

    // A comment naming a local does not count toward the frame.
    tree_of(
        "frame_comment.antics",
        &obj(
            "    @id(4300) @frame(locals: 0) tree t () {\n        raw { /* local[0] is unused */ }\n        animate(clip: 3, event_local: 1);\n        return true;\n    }",
        ),
        4300,
    );

    // `icon: self` in an object's own tree is materialized from `my.id`.
    let t = tree_of(
        "icon_self_object.antics",
        &obj(
            "    @id(4300) tree t () { let o = stack_obj; push_interaction(object: o, menu: 0, priority: 2, icon: self); }",
        ),
        4300,
    );
    assert!(t.contains("= my.id;"), "{t}");

    // `animate(event_local:)` and `sleep` in one tree is a v2 diagnostic, not a backend error.
    let err = compile_v2_source(
        "event_local_sleep.antics",
        &obj(
            "    @id(4300) @frame(locals: 0) tree t (sim: Sim) { animate(clip: 3, event_local: 1); sleep(1); return true; }",
        ),
        &cfg,
    )
    .expect_err("event_local with sleep must fail");
    assert!(
        err.message
            .contains("a tree cannot mix raw ops with sleep()"),
        "{err}"
    );
    // Folded-away code takes no part in that check.
    for body in [
        "if false { sleep(1); } animate(clip: 3, event_local: 1); return true;",
        "if false { animate(clip: 3, event_local: 1); } sleep(1); return true;",
    ] {
        tree_of(
            "event_local_dead.antics",
            &obj(&format!(
                "    @id(4300) @frame(locals: 0) tree t (sim: Sim) {{ {body} }}"
            )),
            4300,
        );
    }
    // A raw `local x;` declaration counts toward the frame.
    let err = compile_v2_source(
        "event_local_decl.antics",
        &obj(
            "    @id(4300) @frame(locals: 0) tree t () { raw { local scratch; } animate(clip: 3, event_local: 0); return true; }",
        ),
        &cfg,
    )
    .expect_err("a declared local at the event slot must fail");
    assert!(err.message.contains("must lie outside the frame"), "{err}");
}
