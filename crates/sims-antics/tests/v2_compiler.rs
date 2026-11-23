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
            semi_globals.insert(res_id, None);
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

pub fn bump_and_true(counter: &mut i16, delta: i16) -> bool {
    *counter += delta;
    return true;
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
use r1_pack::mods::{bump_and_true, call_module_private, mutate_handle_and_attr, nested_sleep_helper};
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
    // P1-7: `self.counter + bump_and_true(&mut self.counter, 5)` must read LHS (10) before RHS mutates counter to 15!
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

            self.arith_res = self.counter + bump_and_true(&mut self.counter, 5);
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
