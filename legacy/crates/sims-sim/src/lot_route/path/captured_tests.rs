//! Private component projection at an actual wrapper entry. This intentionally
//! does not claim to import a gameplay checkpoint or validate a complete tick.
use super::*;
use crate::lot_route::{obstacle::Prefix, search};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_formats::{behavior::Behavior, far, iff::Iff};
use sims_vm::{
    Definition, Module, Object, Routine, Vm, World, autonomy::Placement, loading::SavedFrame,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek, SeekFrom},
    path::Path,
};

mod world;

fn bytes(v: &Value) -> Vec<u8> {
    let s = v.as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn word(b: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(b[offset..offset + 4].try_into().unwrap())
}
fn shorts(b: &[u8]) -> Vec<i16> {
    b.chunks_exact(2)
        .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
fn words(v: &Value) -> Vec<i16> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as i16)
        .collect()
}
fn resource(root: &Path, archive: &str, name: &str) -> Iff {
    let mut f = std::fs::File::open(root.join(archive)).unwrap();
    let len = f.metadata().unwrap().len();
    let mut header = [0; 16];
    f.read_exact(&mut header).unwrap();
    let offset = far::manifest_offset(&header, len).unwrap();
    f.seek(SeekFrom::Start(offset.into())).unwrap();
    let mut directory = vec![];
    f.read_to_end(&mut directory).unwrap();
    let entry = far::directory(&directory, len)
        .unwrap()
        .into_iter()
        .find(|e| e.name.eq_ignore_ascii_case(name))
        .unwrap();
    assert_eq!(entry.stored_size, entry.unpacked_size);
    f.seek(SeekFrom::Start(entry.offset.into())).unwrap();
    let mut data = vec![0; entry.stored_size as usize];
    f.read_exact(&mut data).unwrap();
    Iff::parse(data).unwrap()
}
fn routines(iff: &Iff) -> BTreeMap<u16, Routine> {
    iff.chunks
        .iter()
        .filter(|c| c.tag == *b"BHAV")
        .map(|c| {
            (
                c.id,
                Routine {
                    name: c.label.clone(),
                    body: Behavior::parse(iff.data(c)).unwrap(),
                },
            )
        })
        .collect()
}
fn projected(repo: &Path, state: &Value, captured: &Value) -> (Program, Simulation) {
    let assets = repo.join("game-data/cabinet-extracted/GameData_ALL_Recursive");
    let flamingo = resource(&assets, "Objects/Objects.far", "flamingo.iff");
    let global = resource(&assets, "Global/Global.far", "Global.iff");
    let mut p = Program {
        globals: "original-global".into(),
        ..Default::default()
    };
    p.modules.insert(
        p.globals.clone(),
        Module {
            source: p.globals.clone(),
            content_hash: format!("{:x}", Sha256::digest(&global.bytes)),
            routines: routines(&global),
            ..Default::default()
        },
    );
    let raw_objects = captured["objects"].as_array().unwrap();
    let resources = captured["resources"].as_array().unwrap();
    let mut world = World::default();
    world.globals = state["globals"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, v)| (i as i16, v.as_i64().unwrap() as i16))
        .collect();
    let mut links = crate::lot_links::Links {
        size: captured["floors"][0]["heads"]["width"].as_u64().unwrap() as usize,
        heads: captured["floors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| shorts(&bytes(&f["heads"]["bytes"])))
            .collect(),
        next: BTreeMap::new(),
    };
    let pointers: BTreeMap<_, _> = raw_objects
        .iter()
        .map(|o| {
            (
                o["pointer"].as_u64().unwrap() as u32,
                o["id"].as_i64().unwrap() as i16,
            )
        })
        .collect();
    let mut next = BTreeMap::new();
    let mut bounds = BTreeMap::new();
    for o in state["objects"].as_array().unwrap() {
        let id = o["id"].as_i64().unwrap() as i16;
        let raw = raw_objects.iter().find(|r| r["id"] == o["id"]).unwrap();
        let storage = bytes(&raw["storage"]);
        let definition = bytes(&o["definition"]);
        let pointer = word(&storage, 0x11c);
        let source = format!("captured-resource-{pointer:x}");
        let r = resources.iter().find(|r| r["pointer"] == pointer).unwrap();
        let entries = bytes(&r["functions"]["entries"]);
        let functions: Vec<_> = entries
            .chunks_exact(4)
            .map(|c| sims_formats::execution::ObjectFunction {
                action: u16::from_le_bytes(c[..2].try_into().unwrap()),
                condition: u16::from_le_bytes(c[2..].try_into().unwrap()),
            })
            .collect();
        let m = p.modules.entry(source.clone()).or_insert_with(|| Module {
            source: source.clone(),
            ..Default::default()
        });
        // Artificial module/definition keys only replace native pointers. The
        // complete definition words and callback table come from the capture.
        m.object_definition_words
            .insert(id as u16, shorts(&definition));
        m.object_functions.insert(id as u16, functions.clone());
        if functions.get(5).is_some_and(|f| f.action != 0) {
            assert_eq!(word(&definition, 28), 0x3659dca2);
            let def = flamingo.get(b"OBJD", 16807).unwrap();
            //The observer retains the first0xa4 definition bytes. Compare
            //every captured byte; the remaining IFF tail is not observed.
            assert_eq!(&flamingo.data(def)[..definition.len()], definition);
            m.content_hash = format!("{:x}", Sha256::digest(&flamingo.bytes));
            m.routines = routines(&flamingo);
            assert_eq!(functions[5].action, 4108);
        }
        let slots: Vec<i16> = serde_json::from_value(raw["slot_ids"].clone()).unwrap();
        links.next.insert(id, slots[0]);
        next.insert(id, pointers.get(&word(&storage, 0xe0)).copied());
        bounds.insert(
            id,
            [
                word(&storage, 0x100) as i32,
                word(&storage, 0x104) as i32,
                word(&storage, 0x108) as i32,
                word(&storage, 0x10c) as i32,
            ],
        );
        world.positions.insert(
            id,
            Placement {
                x: word(&storage, 0xf4) as i32,
                y: word(&storage, 0xf8) as i32,
                level: word(&storage, 0x110) as i32,
            },
        );
        world.objects.insert(
            id,
            Object {
                definition: Some(Definition {
                    source,
                    object: id as u16,
                    guid: word(&definition, 28),
                    slots: 0,
                }),
                data: o["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .map(|(n, v)| (n as i16, v.as_i64().unwrap() as i16))
                    .collect(),
                temps: words(&o["temps"]).try_into().unwrap(),
                attributes: words(&o["attributes"]),
                person: o.get("person").map(|p| {
                    p["data"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .enumerate()
                        .map(|(n, v)| (n as i16, v.as_i64().unwrap() as i16))
                        .collect()
                }),
                slots: slots[1..].iter().map(|&n| (n != 0).then_some(n)).collect(),
                ..Default::default()
            },
        );
    }
    let predecessors: BTreeSet<_> = next.values().flatten().copied().collect();
    let heads: Vec<_> = next
        .keys()
        .filter(|n| !predecessors.contains(n))
        .copied()
        .collect();
    assert_eq!(heads.len(), 1);
    let mut order = vec![];
    let mut current = Some(heads[0]);
    while let Some(id) = current {
        assert!(!order.contains(&id));
        order.push(id);
        current = next[&id];
    }
    assert_eq!(order.len(), world.objects.len());
    world.person = Some(sims_vm::person::WorldState {
        object_order: order,
        ..Default::default()
    });
    let original = repo.join("cache/native-route-wrapper-world-oracle-20261023-resume-first");
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(original.join("object-inputs.json")).unwrap())
            .unwrap();
    let architecture = world::project(&mut world, captured, &metadata, &p);
    let (_, mut s) = crate::lot_runtime::tests::fixture();
    s.vm = Vm::new(world, state["global_rng"].as_u64().unwrap());
    s.vm.lot_rng_state = Some(state["lot_rng"].as_u64().unwrap() as u32);
    s.vm.crt_rng_state = Some(state["crt"]["state"].as_u64().unwrap() as u32);
    s.vm.tick = state["tick"].as_u64().unwrap();
    s.bodies.clear(); //Not a character/animation or full world-load projection.
    s.architecture = Some(architecture);
    s.native_lot.as_mut().unwrap().placement.links = links;
    s.native_lot.as_mut().unwrap().placement.bounds = bounds;
    //Lighting is not projected; the route's architecture and retained room
    //policy/cache inputs are captured, with no environment refresh.
    let mut manager = crate::lot_environment::manager::Manager::new(
        s.architecture.as_ref().unwrap().original.as_ref().unwrap(),
        crate::lot_environment::Tuning::from_lookup(|_| None).unwrap(),
        crate::lot_environment::manager::Settings {
            daylight_multiplier: 1.0,
            light_range: 1.0,
            rooms_enabled: true,
            models: [None; 6],
        },
    )
    .unwrap();
    for (id, r) in &mut manager.rooms {
        let original = captured["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == *id)
            .unwrap();
        let raw = bytes(&original["storage"]);
        r.outdoors = raw[0x72] != 0;
        r.totals.half_tiles = word(&raw, 0x38) as i32;
        r.totals.objects = word(&raw, 0x40) as i32;
        r.route_rectangles = serde_json::from_value(original["rectangles"].clone()).unwrap();
    }
    s.room_environment = Some(crate::lot_environment::live::Live {
        manager,
        room_effects_consumed: 0,
        clock_events_consumed: 0,
    });
    for id in [156, 155] {
        let original = state["objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == id)
            .unwrap();
        let owner = s.vm.world.objects[&id]
            .definition
            .as_ref()
            .unwrap()
            .source
            .clone();
        let frames: Vec<_> = original["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| SavedFrame {
                owner: owner.clone(),
                tree: f["tree"].as_u64().unwrap() as u16,
                node: f["node"].as_u64().unwrap() as u16,
                stack_object: f["stack_object"].as_i64().unwrap() as i16,
                args: words(&f["args"]),
                locals: words(&f["locals"]),
                primitive_state: f["continuation"].as_u64().unwrap() as u32,
            })
            .collect();
        let thread =
            s.vm.restore_object_stack(&p, id, &frames, 4096, false)
                .unwrap();
        s.vm.threads.get_mut(&thread).unwrap().last_result = original["last_result"] == 1;
    }
    (p, s)
}

fn execution(s: &Simulation, entry: &Value, case: &Value) -> super::super::live::Execution {
    let actor = entry["context"]["actor"].as_i64().unwrap() as i16;
    let target = entry["context"]["target"].as_i64().unwrap() as i16;
    let raw = bytes(&entry["context"]["bytes"]);
    let values = case
        .get("candidates")
        .unwrap_or(&entry["context"]["candidates"]["values"]);
    let candidates: Vec<_> = values
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let packed = c[3].as_i64().unwrap() as u32;
            crate::routing::Candidate {
                position: [c[0].as_i64().unwrap() as i32, c[1].as_i64().unwrap() as i32],
                score: c[2].as_i64().unwrap() as i32,
                seat: packed as i16,
                flags: (packed >> 16) as u16,
            }
        })
        .collect();
    let pos = s.vm.world.positions[&actor];
    let target_pos = s.vm.world.positions[&target];
    //The stage under test begins after candidate/clearance production. Unused
    //scheduler/request fields are explicit laboratory values, not an import.
    serde_json::from_value(json!({"potential":if case["caller"] == true {Value::Null} else {json!({
            "ignored_person":if case["ignored"] == true {case["near_person"].clone()} else {Value::Null},
            "ignore_people":case["ignore_people"] == 1,"ignored_word":0,"blocked_object":0,"retained_result":word(&raw,0x88)})},
        "thread":0,"request":1,"caller":actor,"target":target,"target_position":[target_pos.x,target_pos.y],
        "target_facing":s.vm.world.objects[&target].data[&1],"descriptor_facing":word(&raw,0x40) as i32,
        "saved_facing":s.vm.world.objects[&actor].data[&1],"candidate_limit":48,"subset_rng":s.vm.rng_state,
        "retained_result":word(&raw,0x88),"departure":null,"prepared":true,"posture_checked":true,
        "point":null,"standing":null,"posture":null,"input":{"size":s.vm.world.map_size.unwrap().active_size,
            "position":[pos.x,pos.y],"target":[target_pos.x,target_pos.y],"level":pos.level,
            "room":s.vm.world.objects[&actor].data[&29],"flags":word(&raw,0x28),"direction":null,"ignored":null},
        "clearance":null,"blocked_object":0,"goal":null,"selected_seat":null,"floor":null,
        "candidates":{"candidates":candidates,"maximum":word(&raw,0x54),"result":0,"actor_facing":0},
        "obstacles":null,"policy":"InferredRectangles","path":null,"failed":false})).unwrap()
}

#[test]
#[ignore = "private original full-wrapper probe and fresh SIMS_ROUTE_RUNTIME_OUTPUT"]
fn runtime_path_cache_results_and_publication_match_original_wrapper() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let source = repo.join("cache/native-route-wrapper-20261023-fourth/world-01");
    let read = |p: &Path| -> Value { serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap() };
    let provenance = read(&source.join("report.json"));
    for (name, hash) in provenance["files"].as_object().unwrap() {
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(std::fs::read(source.join(name)).unwrap())
            ),
            hash.as_str().unwrap()
        );
    }
    let trace =
        std::fs::read(repo.join("cache/native-route-wrapper-20261023-fourth/trace.jsonl")).unwrap();
    let entry: Value = trace
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice::<Value>(l).unwrap())
        .find(|r| r["phase"] == "wrapper_entry")
        .unwrap();
    let oracle_path = repo.join(std::env::var("SIMS_ROUTE_WRAPPER_ORACLE").unwrap());
    let oracle = read(&oracle_path.join("report.json"));
    assert_eq!(oracle["source_hashes"], provenance["files"]);
    assert_eq!(
        oracle["trace_sha256"],
        format!("{:x}", Sha256::digest(&trace))
    );
    assert_eq!(oracle["original_execution"], true);
    assert_eq!(oracle["baseline_matches"], true);
    let (p, initial) = projected(
        &repo,
        &read(&source.join("state.json")),
        &read(&source.join("world.json")),
    );
    //A former strict-prefix stop has already entered object callbacks without
    //retaining entry inputs. Do not manufacture those inputs from later state.
    let mut legacy = initial.clone();
    let mut old = execution(&legacy, &entry, &json!({"caller":true}));
    old.policy = Policy::Strict;
    old.obstacles = Some(Prefix::begin(&legacy, old.caller).unwrap());
    let mut old_step = Step {
        stop: crate::StepStop::Budget,
        trace: vec![],
        events: vec![],
    };
    assert!(
        legacy
            .drive_native_obstacle_prefix(
                &p,
                old.obstacles.as_mut().unwrap(),
                &mut 1000,
                &mut old_step
            )
            .unwrap()
    );
    let retained = legacy.clone();
    assert!(
        legacy
            .drive_native_path_prefix(&p, &mut old, &mut 1000, &mut old_step)
            .unwrap_err()
            .contains("legacy partial route lacks pre-callback path inputs")
    );
    assert_eq!(legacy, retained);
    let output = repo.join(std::env::var("SIMS_ROUTE_RUNTIME_OUTPUT").unwrap());
    assert!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).unwrap();
    let mut results = vec![];
    for row in oracle["cases"].as_array().unwrap() {
        let c = &row["input"];
        let expected = &row["original"];
        //The runtime caller returns before this stage for an empty candidate
        //vector. Native empty-vector result1->7 remains separately recorded.
        if c.get("candidates")
            .is_some_and(|v| v.as_array().unwrap().is_empty())
        {
            continue;
        }
        for (policy, budget) in [
            (Policy::Strict, 0),
            (Policy::Strict, 1000),
            (Policy::InferredRectangles, 0),
            (Policy::InferredRectangles, 1000),
        ] {
            let mut s = initial.clone();
            let actor = entry["context"]["actor"].as_i64().unwrap() as i16;
            if let Some(n) = c["special"].as_i64() {
                *s.vm
                    .world
                    .objects
                    .get_mut(&actor)
                    .unwrap()
                    .person
                    .as_mut()
                    .unwrap()
                    .get_mut(&(n as i16))
                    .unwrap() |= 1;
            }
            if let Some(other) = c["near_person"].as_i64() {
                let other = other as i16;
                let a = s.vm.world.positions[&actor];
                s.vm.world.positions.insert(
                    other,
                    Placement {
                        x: a.x - 20,
                        y: a.y,
                        level: a.level,
                    },
                );
                let room = s.vm.world.objects[&actor].data[&29];
                s.vm.world
                    .objects
                    .get_mut(&other)
                    .unwrap()
                    .data
                    .insert(29, room);
                s.native_lot
                    .as_mut()
                    .unwrap()
                    .placement
                    .bounds
                    .insert(other, [a.x - 17, a.y + 3, a.x - 23, a.y - 3]);
            }
            let room = s.vm.world.objects[&actor].data[&29] as u16;
            if c["nonground"] == true {
                s.vm.world
                    .objects
                    .get_mut(&actor)
                    .unwrap()
                    .data
                    .insert(17, 1);
            }
            if let Some(half) = c["half_tiles"].as_i64() {
                s.room_environment
                    .as_mut()
                    .unwrap()
                    .manager
                    .rooms
                    .get_mut(&room)
                    .unwrap()
                    .totals
                    .half_tiles = half as i32;
            }
            if c["cached"] == true {
                s.room_environment
                    .as_mut()
                    .unwrap()
                    .manager
                    .rooms
                    .get_mut(&room)
                    .unwrap()
                    .route_rectangles =
                    serde_json::from_value(expected["room_cache"].clone()).unwrap();
                //A retained nonempty cache must not rebuild/read the lot.
                s.architecture.as_mut().unwrap().original = None;
            }
            s.native_lot
                .as_mut()
                .unwrap()
                .route_paths
                .insert(actor, vec![[-991, -992]]);
            let before = s.vm.clone();
            let mut e = execution(&s, &entry, c);
            e.policy = policy;
            s.native_lot.as_mut().unwrap().routing_policy = policy;
            let mut step = Step {
                stop: crate::StepStop::Budget,
                trace: vec![],
                events: vec![],
            };
            let mut remaining = budget;
            let mut yields = 0;
            loop {
                if s.drive_native_path_prefix(&p, &mut e, &mut remaining, &mut step)
                    .unwrap()
                {
                    break;
                }
                assert!(yields < 8);
                yields += 1;
                e.obstacles
                    .as_ref()
                    .unwrap()
                    .validate(&s, &p, actor)
                    .unwrap();
                s = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
                e = serde_json::from_slice(&serde_json::to_vec(&e).unwrap()).unwrap();
                remaining = 1;
            }
            let path = e.path.as_ref().unwrap();
            let actual = json!({"failed":e.failed,"result":e.potential_result().1,
                "path":path.points.as_ref().map(|p|p.iter().map(|v|[v.y,v.x]).collect::<Vec<_>>()).unwrap_or_default(),
                "dynamic":path.dynamic,"fixed":path.fixed,"retained_path":s.native_lot.as_ref().unwrap().route_paths[&actor],
                "entry_flags":s.vm.world.objects[&actor].person.as_ref().unwrap()[&37] as u16,
                "room_cache":s.room_environment.as_ref().unwrap().manager.rooms[&room].route_rectangles});
            let expected_path = if c["caller"] == true {
                actual["retained_path"].clone()
            } else {
                actual["path"].clone()
            };
            let matches = expected_path == expected["path"]
                && actual["result"] == expected["retained_result"]
                && actual["failed"] == (expected["returned"] == 0)
                && actual["room_cache"] == expected["room_cache"]
                && (c["caller"] != true || actual["entry_flags"] == expected["entry_flags"])
                && expected["searches"]
                    .as_array()
                    .unwrap()
                    .first()
                    .is_none_or(|search| {
                        actual["dynamic"] == search["dynamic"] && actual["fixed"] == search["fixed"]
                    });
            results.push(json!({"input":c,"policy":policy,"budget":budget,"yields":yields,"matches":matches,"actual":actual,"original":expected}));
            std::fs::write(output.join("report.json"),serde_json::to_vec_pretty(&json!({"source_hashes":provenance["files"],
                "scope":"Production runtime path stage with captured world; probe heap/callback/descriptor services remain explicit; not whole-tick parity.",
                "cases":results,"strict_original_parity_ticks_added":0})).unwrap()).unwrap();
            assert!(
                matches,
                "runtime path stage differs for {}; report preserved",
                c["name"]
            );
            assert_eq!(s.vm.rng_state, before.rng_state);
            assert_eq!(s.vm.lot_rng_state, before.lot_rng_state);
            assert_eq!(s.vm.crt_rng_state, before.crt_rng_state);
            assert_eq!(s.vm.tick, before.tick);
            if policy == Policy::Strict && c["caller"] == true {
                let retained = s.clone();
                assert_eq!(
                    s.install_original_path(&p, &e, &mut step).unwrap_err(),
                    "strict original routing requires the native character animator"
                );
                assert_eq!(
                    s, retained,
                    "strict routing entered tuned movement without an animator"
                );
            }
            if c["caller"] != true {
                assert_eq!(
                    s.native_lot.as_ref().unwrap().route_paths[&actor],
                    vec![[-991, -992]]
                );
            }
        }
    }
}

#[test]
#[ignore = "private original wrapper world, original IFF bytecode and fresh SIMS_ROUTE_DYNAMIC_OUTPUT"]
fn actual_dynamic_scan_and_original_callbacks_match_live_wrapper() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let source = repo.join("cache/native-route-wrapper-20261023-fourth/world-01");
    let trace =
        std::fs::read(repo.join("cache/native-route-wrapper-20261023-fourth/trace.jsonl")).unwrap();
    let rows: Vec<Value> = trace
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice(l).unwrap())
        .collect();
    let read = |name| -> Value {
        serde_json::from_slice(&std::fs::read(source.join(name)).unwrap()).unwrap()
    };
    let provenance = read("report.json");
    for (name, hash) in provenance["files"].as_object().unwrap() {
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(std::fs::read(source.join(name)).unwrap())
            ),
            hash.as_str().unwrap()
        );
    }
    let entry = rows.iter().find(|r| r["phase"] == "wrapper_entry").unwrap();
    assert_eq!(entry["world_inputs"]["files"], provenance["files"]);
    assert!(
        rows.iter()
            .any(|r| r["kind"] == "complete" && r["reason"] == "requested_wrapper_returns")
    );
    let expected: Vec<_> = rows
        .iter()
        .filter(|r| r["phase"] == "dynamic_before")
        .collect();
    let callback_entries: Vec<_> = rows
        .iter()
        .filter(|r| r["phase"] == "obstacle_callback_entry" && r["callback"]["routine"] != 0)
        .collect();
    assert_eq!(callback_entries.len(), 2);
    for entry in &callback_entries {
        let end = rows
            .iter()
            .find(|r| {
                r["phase"] == "obstacle_callback_return" && r["callback"] == entry["callback"]
            })
            .unwrap();
        assert_eq!(end["result"], 0);
        for field in ["global_rng", "lot_rng"] {
            assert_eq!(entry[field], end[field]);
        }
        assert_eq!(entry["state"]["crt"]["state"], end["state"]["crt"]["state"]);
        for (before, after) in entry["state"]["objects"]
            .as_array()
            .unwrap()
            .iter()
            .zip(end["state"]["objects"].as_array().unwrap())
        {
            assert_eq!(before["id"], after["id"]);
            for field in [
                "definition",
                "data",
                "temps",
                "attributes",
                "last_result",
                "frames",
                "relationships",
                "person",
            ] {
                assert_eq!(
                    before[field], after[field],
                    "original callback changed {} {field}",
                    before["id"]
                );
            }
        }
    }
    let (p, initial) = projected(&repo, &read("state.json"), &read("world.json"));
    let producer_path = repo.join(std::env::var("SIMS_ROUTE_WORLD_PRODUCER_REPORT").unwrap());
    let producer_bytes = std::fs::read(&producer_path).unwrap();
    let fixed_producer: Value = serde_json::from_slice(&producer_bytes).unwrap();
    assert_eq!(
        fixed_producer["sha256"],
        format!("{:x}", Sha256::digest(&trace))
    );
    let search_entries: Vec<_> = rows
        .iter()
        .filter(|r| r["phase"] == "search_entry")
        .collect();
    let search_results: Vec<_> = rows
        .iter()
        .filter(|r| r["phase"] == "search_result")
        .collect();
    assert_eq!(search_entries.len(), 2);
    assert_eq!(search_results.len(), 2);
    assert_eq!(fixed_producer["searches"].as_array().unwrap().len(), 2);
    let output = repo.join(std::env::var("SIMS_ROUTE_DYNAMIC_OUTPUT").unwrap());
    assert!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).unwrap();
    let mut reports = vec![];
    for limit in [1, 1000] {
        let mut s = initial.clone();
        let actor = entry["context"]["actor"].as_i64().unwrap() as i16;
        let before = s.vm.world.clone();
        let threads = s.vm.threads.clone();
        let mut input = Inputs::begin(&s, actor).unwrap();
        let mut prefix = Prefix::begin(&s, actor).unwrap();
        let mut step = Step {
            stop: crate::StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let mut actual = vec![];
        let mut yields = 0;
        let mut allowance = if limit == 1 { 0 } else { limit };
        loop {
            let mut budget = allowance;
            if !s
                .drive_native_obstacle_prefix(&p, &mut prefix, &mut budget, &mut step)
                .unwrap()
            {
                prefix.validate(&s, &p, actor).unwrap();
                let encoded = serde_json::to_vec(&s).unwrap();
                s = serde_json::from_slice(&encoded).unwrap();
                prefix = serde_json::from_slice(&serde_json::to_vec(&prefix).unwrap()).unwrap();
                input = serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
                yields += 1;
                allowance = limit;
                continue;
            }
            let Some(g) = &prefix.geometry else {
                break;
            };
            let row = expected.get(actual.len()).expect("extra dynamic obstacle");
            let matches = json!(g.object) == row["object"]
                && json!(g.kind) == row["object_kind"]
                && json!(g.rectangle) == row["rectangle"];
            actual.push(json!({"geometry":g,"original":row["rectangle"],"matches":matches}));
            input.retain_geometry(&s, g).unwrap();
            let after = rows
                .iter()
                .find(|r| r["phase"] == "dynamic_after" && r["object"] == g.object)
                .unwrap();
            assert_eq!(json!(input.dynamic), after["dynamic"]["values"]);
            prefix.consume_geometry(&s).unwrap();
            allowance = if limit == 1 { 0 } else { limit };
        }
        let callbacks: Vec<_> = step
            .trace
            .iter()
            .map(|t| {
                let object = initial
                    .vm
                    .object_threads
                    .iter()
                    .find(|(_, thread)| **thread == t.thread)
                    .unwrap()
                    .0;
                json!({"object":object,"tree":t.location.bhav,"instruction":t.location.instruction})
            })
            .collect();
        let matches = actual.len() == expected.len() && actual.iter().all(|r| r["matches"] == true);
        let raw_context = bytes(&entry["context"]["bytes"]);
        let candidates = crate::routing::CandidateSet {
            candidates: entry["context"]["candidates"]["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    let packed = c[3].as_i64().unwrap() as u32;
                    crate::routing::Candidate {
                        position: [c[0].as_i64().unwrap() as i32, c[1].as_i64().unwrap() as i32],
                        score: c[2].as_i64().unwrap() as i32,
                        seat: packed as i16,
                        flags: (packed >> 16) as u16,
                    }
                })
                .collect(),
            maximum: word(&raw_context, 0x54) as i32,
            result: 0,
            actor_facing: 0,
        };
        let (goals, indices) = search::goals(&candidates);
        let room = &s.room_environment.as_ref().unwrap().manager.rooms[&(input.room as u16)];
        let config = input.search_actor.unwrap().configure(
            room.totals.half_tiles,
            room.totals.objects,
            goals.len() as i32,
        );
        let fixed = &fixed_producer["searches"][0];
        assert_eq!(fixed["matches"], true);
        assert_eq!(fixed["room"], input.room);
        input.fixed = serde_json::from_value(fixed["actual"].clone()).unwrap();
        assert_eq!(json!(goals), search_entries[0]["goals"]["values"]);
        assert_eq!(json!(input.dynamic), search_entries[0]["dynamic"]["values"]);
        let result = config
            .run(input.start, &goals, &input.dynamic, &input.fixed)
            .unwrap();
        let passes: Vec<_> = result.preliminary.iter().chain(result.search.iter()).map(|r| json!({
            "goal":r.goal.map_or(-1,|n| n as i64),"path":r.points.iter().map(|p| [p.x,p.y]).collect::<Vec<_>>(),
            "rectangles":r.regions,"iterations":r.iterations})).collect();
        let expected_passes: Vec<_> = search_results
            .iter()
            .map(|r| {
                json!({"goal":r["goal"],"path":r["path"]["values"],
            "rectangles":r["rectangles"]["values"],"iterations":r["iterations"]})
            })
            .collect();
        let returned = rows
            .iter()
            .find(|r| r["phase"] == "wrapper_return")
            .unwrap();
        let final_search = result.search.as_ref().unwrap();
        assert_eq!(
            indices[final_search.goal.unwrap()],
            word(&bytes(&returned["context"]["bytes"]), 0x50) as usize
        );
        assert_eq!(
            json!(
                final_search
                    .points
                    .iter()
                    .map(|p| [p.y, p.x])
                    .collect::<Vec<_>>()
            ),
            returned["output"]["values"]
        );
        assert_eq!(passes, expected_passes);
        reports.push(json!({"budget":limit,"matches":matches,"obstacles":actual,"callbacks":callbacks,"diagnostic_yields":yields}));
        std::fs::write(output.join("report.json"),serde_json::to_vec_pretty(&json!({
            "scope":"Production dynamic scan and actual flamingo callbacks at captured wrapper entry; partial world projection, not full wrapper or tick parity.",
            "source_hashes":provenance["files"],"trace_sha256":format!("{:x}",Sha256::digest(&trace)),
            "bytecode_hashes":p.modules.iter().filter(|(_,m)| !m.content_hash.is_empty()).map(|(key,m)| (key.clone(),m.content_hash.clone())).collect::<BTreeMap<_,_>>(),
            "fixed_producer":producer_path,"fixed_producer_sha256":format!("{:x}",Sha256::digest(&producer_bytes)),
            "assembled_passes":passes,"assembled_policy":config,"assembled_candidate_indices":indices,
            "runs":reports,"strict_original_parity_ticks_added":0})).unwrap()).unwrap();
        assert!(matches, "dynamic scan differs; report preserved");
        assert_eq!(actual.len(), 71);
        assert_eq!(step.trace.len(), 2);
        assert_eq!(
            yields,
            if limit == 1 { 2 } else { 0 },
            "suspended-callback roundtrip must execute"
        );
        assert_eq!(
            callbacks
                .iter()
                .map(|r| json!([r["object"], r["tree"]]))
                .collect::<Vec<_>>(),
            callback_entries
                .iter()
                .map(|r| json!([r["callback"]["object"], r["callback"]["routine"]]))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            s.vm.world, before,
            "callbacks changed captured gameplay fields"
        );
        for (&id, t) in &threads {
            assert_eq!(s.vm.threads[&id].callback_result, Some(false));
            assert_eq!(s.vm.threads[&id].stack, t.stack);
            assert_eq!(s.vm.threads[&id].status, t.status);
            assert_eq!(s.vm.threads[&id].last_result, t.last_result);
        }
        assert_eq!(s.vm.rng_state, initial.vm.rng_state);
        assert_eq!(s.vm.lot_rng_state, initial.vm.lot_rng_state);
        assert_eq!(s.vm.crt_rng_state, initial.vm.crt_rng_state);
        assert_eq!(s.vm.tick, initial.vm.tick);
    }
}
