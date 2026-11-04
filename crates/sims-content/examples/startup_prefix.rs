//! Bounded controller-prefix diagnostic from explicit captured incoming storage.
//! Does not discard temporary objects or certify a playable imported household.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::{Index, object_catalog::Catalog, saves::House};
use sims_vm::{Definition, Object, Vm};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    sync::Arc,
};
fn signed(v: &Value) -> i16 {
    v.as_u64().unwrap() as u16 as i16
}
fn bytes(v: &Value) -> Vec<u8> {
    let s = v.as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|n| u8::from_str_radix(&s[n..n + 2], 16).unwrap())
        .collect()
}
fn state(vm: &Vm) -> Value {
    json!({"tick":vm.tick,"global_rng":vm.rng_state,"lot_rng":vm.lot_rng_state,
        "crt_rng":vm.crt_rng_state,"map_size":vm.world.map_size,
        "action_serial":vm.world.actions.as_ref().map(|a| a.serial),
        "object_order":vm.world.person.as_ref().map(|p| &p.object_order),
        "resource_instances":vm.world.person.as_ref().map(|p| &p.resource_instances),
        "globals":vm.world.globals,"resource_storage":vm.world.resource_storage,
        "objects":vm.world.objects.iter().map(|(&id,o)| json!({"id":id,"guid":o.definition.as_ref().unwrap().guid,
            "data":o.data,"temps":o.temps,"attributes":o.attributes,
            "thread":vm.object_threads.get(&id).map(|id| &vm.threads[id])})).collect::<Vec<_>>()})
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        (3..=5).contains(&args.len()),
        "usage: startup_prefix CONTENT_ROOT NATIVE_PHASE_TRACE NEW_PRIVATE_REPORT [SEPARATE_RETAINED_CONTEXT] [HOUSE_SOURCE]"
    );
    let out = Path::new(&args[2]);
    ensure!(
        out.parent()
            .context("output parent")?
            .canonicalize()?
            .starts_with(Path::new("cache").canonicalize()?),
        "private output required"
    );
    let file = File::create_new(out)?;
    let rows = BufReader::new(File::open(&args[1])?)
        .lines()
        .map(|l| -> Result<Value> { Ok(serde_json::from_str(&l?)?) })
        .collect::<Result<Vec<_>>>()?;
    let input = rows
        .iter()
        .find(|r| r["kind"] == "resize_entry")
        .context("resize entry absent")?;
    let separate_context: Option<Value> = args
        .get(3)
        .map(|path| -> Result<_> { Ok(serde_json::from_reader(File::open(path)?)?) })
        .transpose()?;
    let context = input
        .get("context")
        .or_else(|| separate_context.as_ref().and_then(|r| r.get("context")));
    let index = Index::scan(Path::new(&args[0]))?;
    let source = args
        .get(4)
        .map(String::as_str)
        .unwrap_or("UserData/Houses/House07.iff");
    let house = House::load(&index, source)?;
    ensure!(
        input["globals"][10] == house.number
            && input["objects"].as_array().context("objects")?.len() == 2,
        "requires explicit two-indicator load input for the selected house"
    );
    let mut catalog = Catalog::household(
        Arc::new(index.clone()),
        &house.neighborhood_source,
        Default::default(),
    )?;
    let controllers = catalog.global_controllers(&index)?;
    let mut program = house.link_program(&index)?;
    sims_content::catalog::link_controllers(&index, &mut program, &controllers)?;
    program.resource_storage = Some(catalog.resource_storage(&index)?);
    let guids = input["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            let b = bytes(&o["definition"]);
            u32::from_le_bytes(b[28..32].try_into().unwrap())
        })
        .collect::<Vec<_>>();
    sims_content::assets::load_object_guids(&index, &mut program, &guids)?;
    let mut world = house.prepare_neighborhood(&program)?;
    if let Some(flags) = context.and_then(|c| c.get("catalog_flags")) {
        // Retain winning GUID entries in registration order, like43ac10.
        for resource in flags.as_array().context("catalog flag rows")? {
            let guid = resource["guid"].as_u64().context("catalog GUID")? as u32;
            if resource["flags"].as_u64().context("catalog flags")? & 2 != 0 {
                world.changed_resources.insert(guid);
            } else {
                world.changed_resources.remove(&guid);
            }
        }
    }
    world.globals = input["globals"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(n, v)| (n as i16, signed(v)))
        .collect();
    let people = world.person.as_mut().unwrap();
    people.object_order.clear();
    people.person_order.clear();
    people.tile_chains.clear();
    people.resource_instances.clear();
    people.free_will = world.globals[&30] != 0;
    // The residential-lot query runs inside controller Main during creation,
    // before saved object reads. Supply the explicit installed zoning input.
    let zoning = sims_content::zoning::Zoning::load(
        &index,
        "GameData_ALL_Recursive/MasterLotZoning.iff",
        world.globals[&20] as u16,
    )?;
    people.lot_zoning = zoning.query_table();
    if let Some(context) = context {
        ensure!(
            context["zoning"] == json!(zoning.rows),
            "captured zoning differs from explicit master input"
        );
        // Earlier diagnostic captures called this pointer "travel" before its
        // family-record identity was recovered. Preserve those raw captures.
        let family = context.get("selected_family").unwrap_or(&context["travel"]);
        let storage = context
            .get("selected_family_storage")
            .unwrap_or(&context["travel_storage"]);
        world.magic_history = Some(if family.as_u64() == Some(0) {
            sims_vm::magic_history::Context::Absent
        } else {
            let history = bytes(storage);
            let registry = u32::from_le_bytes(history[0x128..0x12c].try_into().unwrap());
            let simulator = bytes(&input["simulator_storage"]);
            ensure!(
                registry == 0
                    || registry == u32::from_le_bytes(simulator[0xb0..0xb4].try_into().unwrap()),
                "selected family points to a foreign registry"
            );
            sims_vm::magic_history::Context::Retained {
                enabled: history[0x14c] != 0,
                family: i32::from_le_bytes(history[0x10c..0x110].try_into().unwrap()),
                registry_present: registry != 0,
                values: std::array::from_fn(|i| {
                    i16::from_le_bytes(history[0x14e + i * 2..0x150 + i * 2].try_into().unwrap())
                }),
            }
        });
    } else {
        // The captured return address establishes the428041 inner-load path,
        // whose earlier42814b store selects the freshly constructed family0.
        // This is supported by the native house-nearby-context and family-magic
        // probes; a different caller requires its actual incoming context.
        let entry = rows
            .iter()
            .find(|r| r["kind"] == "house_entry")
            .context("house entry absent")?;
        let stack = bytes(&entry["stack"]);
        ensure!(
            u32::from_le_bytes(stack[..4].try_into().unwrap()) == 0x4281df,
            "unknown inner-house caller requires explicit selected-family context"
        );
        world.magic_history = Some(sims_vm::magic_history::Context::constructed(0, true));
    }
    let refresh = context
        .and_then(|c| c.get("refresh"))
        .context("incoming native view/refresh fields absent; capture them explicitly")?;
    world.refresh = Some(sims_vm::refresh::RefreshState {
        visible_level: refresh["visible_level"]
            .as_i64()
            .context("captured visible level")?
            .try_into()?,
        render_clock: refresh["render_clock"]
            .as_u64()
            .context("captured render clock")?
            .try_into()?,
        ..Default::default()
    });
    // SIMI balance and the eight current-day statistics are read before this
    // boundary. Keep the same saved inputs as Loaded::load_simulator_context.
    let mut expenses = [0; 9];
    expenses[..8].copy_from_slice(
        &house
            .simulator
            .budget_days
            .first()
            .copied()
            .flatten()
            .unwrap_or([0; 8]),
    );
    world.budget = Some(sims_vm::budget::State {
        balance: house
            .simulator
            .funds
            .wrapping_add(house.simulator.pending_funds),
        expenses,
        notifications: vec![],
    });
    for (row, &guid) in input["objects"].as_array().unwrap().iter().zip(&guids) {
        let id = signed(&row["id"]);
        let mut definitions = program.modules.iter().flat_map(|(source, m)| {
            m.object_definition_words
                .iter()
                .filter_map(move |(&chunk, w)| {
                    (w.len() > 20 && (w[14] as u16 as u32 | (w[15] as u16 as u32) << 16) == guid)
                        .then(|| Definition {
                            source: source.clone(),
                            object: chunk,
                            guid,
                            slots: w[20] as u16,
                        })
                })
        });
        let definition = definitions
            .next()
            .context("incoming indicator definition")?;
        ensure!(
            definitions.next().is_none(),
            "ambiguous incoming definition"
        );
        let mut o = Object::construct(
            definition,
            &program,
            usize::try_from(signed(&row["data"][67]))?,
        )
        .map_err(anyhow::Error::msg)?;
        o.data = row["data"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(n, v)| (n as i16, signed(v)))
            .collect();
        o.attributes = row["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .map(signed)
            .collect();
        o.temps = row["temps"]
            .as_array()
            .unwrap()
            .iter()
            .map(signed)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        ensure!(
            o.data[&2] == 0 && o.data[&26] == 0,
            "incoming contained indicator unsupported"
        );
        let raw = bytes(&row["base_storage"]);
        o.native_flags = Some(u32::from_le_bytes(raw[0x114..0x118].try_into().unwrap()));
        let position = sims_vm::autonomy::Placement {
            x: i32::from_le_bytes(raw[0xf4..0xf8].try_into().unwrap()),
            y: i32::from_le_bytes(raw[0xf8..0xfc].try_into().unwrap()),
            level: i32::from(raw[0xfe] as i8),
        };
        ensure!(
            position.x < 0 && position.y < 0,
            "incoming placed indicator unsupported"
        );
        world
            .register_scheduler_object(id, &mut o)
            .map_err(anyhow::Error::msg)?;
        o.idle = Some(sims_vm::idle::IdleState::from_registry_word(
            row["registry_word"].as_u64().unwrap() as u32,
        ));
        world.person.as_mut().unwrap().object_order.push(id);
        world
            .refresh
            .as_mut()
            .unwrap()
            .draw
            .insert(id, sims_vm::refresh::DrawState::constructed(position.level));
        world.positions.insert(id, position);
        world.objects.insert(id, o);
    }
    world.actions = Some(sims_vm::action::WorldState {
        serial: input["action_serial"].as_u64().unwrap() as u32,
        ..Default::default()
    });
    if let Some(records) = context
        .and_then(|c| c.get("resource_globals"))
        .and_then(Value::as_array)
    {
        ensure!(
            records.len() == world.resource_storage.records.len(),
            "captured resource-global directory differs"
        );
        for (row, record) in records.iter().zip(&mut world.resource_storage.records) {
            ensure!(
                row["guid"].as_u64() == Some(record.guid as u64),
                "resource-global order differs"
            );
            let values = row["values"].as_array().context("resource words")?;
            ensure!(
                values.len() == record.values.len(),
                "resource-global allocation differs"
            );
            record.values = values.iter().map(signed).collect();
        }
    }
    if let Some(context) = context {
        if let Some(order) = context.get("object_order").and_then(Value::as_array) {
            world.person.as_mut().unwrap().object_order = order.iter().map(signed).collect();
        }
        if let Some(cache) = context.get("resource_instances").and_then(Value::as_array) {
            world.person.as_mut().unwrap().resource_instances = cache
                .iter()
                .map(|r| {
                    (
                        r["guid"].as_u64().unwrap() as u32,
                        r["objects"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(signed)
                            .collect(),
                    )
                })
                .collect();
        }
    }
    let mut e = sims_content::save_storage::Execution::from_retained(Vm::new(
        world,
        input["global_rng"].as_u64().unwrap(),
    ));
    e.vm.tick = input["tick"].as_u64().unwrap();
    e.vm.crt_rng_state = Some(input["crt"]["state"].as_u64().unwrap() as u32);
    e.vm.lot_rng_state = Some(input["lot_rng"].as_u64().unwrap() as u32);
    for row in input["objects"].as_array().unwrap() {
        let id = signed(&row["id"]);
        let raw = bytes(&row["base_storage"]);
        let start = u32::from_le_bytes(raw[8..12].try_into().unwrap());
        let end = u32::from_le_bytes(raw[12..16].try_into().unwrap());
        let owner = e.vm.world.objects[&id]
            .definition
            .as_ref()
            .unwrap()
            .source
            .clone();
        let frames = row["frames"]
            .as_array()
            .context("frames")?
            .iter()
            .map(|f| sims_vm::loading::SavedFrame {
                owner: owner.clone(),
                tree: f["tree"].as_u64().unwrap() as u16,
                node: f["node"].as_u64().unwrap() as u16,
                stack_object: signed(&f["stack_object"]),
                args: f["args"].as_array().unwrap().iter().map(signed).collect(),
                locals: f["locals"].as_array().unwrap().iter().map(signed).collect(),
                primitive_state: f["continuation"].as_u64().unwrap() as u32,
            })
            .collect::<Vec<_>>();
        let thread =
            e.vm.restore_object_stack(&program, id, &frames, (end - start) as usize, false)
                .map_err(anyhow::Error::msg)?;
        e.vm.threads.get_mut(&thread).unwrap().last_result =
            row["last_result"].as_u64().unwrap() != 0;
        e.vm.world.objects.get_mut(&id).unwrap().idle =
            Some(sims_vm::idle::IdleState::from_registry_word(
                row["registry_word"].as_u64().unwrap() as u32,
            ));
    }
    // Ordinary incoming threads remain suspended throughout all load callbacks.
    let before = state(&e.vm);
    let requested = match signed(&input["globals"][23]) {
        0 => 64,
        n => i32::from(n),
    };
    let resized = requested > 3 && requested != input["size"].as_i64().unwrap() as i32;
    if let Some(geometry) = context.and_then(|c| c.get("geometry")) {
        let geometry_context = context.unwrap();
        let size = geometry["size"].as_u64().context("geometry size")? as usize;
        //The capture's geometry size reads native world+10, not an inferred
        //global23 or the later saved-array dimensions.
        e.vm.world.map_size = Some(sims_vm::world_map::State {
            active_size: size as i32,
        });
        let words = |v: &Value| {
            bytes(v)
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
                .collect::<Vec<_>>()
        };
        let floors = geometry["floors"]
            .as_array()
            .context("floors")?
            .iter()
            .map(|f| sims_sim::lot_geometry::FloorSource {
                coverings: words(&f["coverings"]),
                object_heads: words(&f["heads"]),
                flags: bytes(&f["flags"]),
                walls: bytes(&f["walls"])
                    .chunks_exact(14)
                    .map(|w| sims_formats::lot::WallTile {
                        segments: w[0],
                        flags: w[1],
                        styles: std::array::from_fn(|i| {
                            u16::from_le_bytes(w[2 + i * 2..4 + i * 2].try_into().unwrap())
                        }),
                        patterns: std::array::from_fn(|i| {
                            u16::from_le_bytes(w[6 + i * 2..8 + i * 2].try_into().unwrap())
                        }),
                    })
                    .collect(),
            })
            .collect();
        let lot = sims_sim::lot_geometry::Lot::rebuild(
            size,
            bytes(&geometry["heights"])
                .chunks_exact(4)
                .map(|v| v.try_into().unwrap())
                .collect(),
            bytes(&geometry["pool"]),
            bytes(&geometry["water"]),
            floors,
        )
        .map_err(anyhow::Error::msg)?;
        let id = |v: &Value| {
            let n = signed(v);
            (n != 0).then_some(n)
        };
        let incoming = sims_content::house_startup::Incoming {
            vm: e.vm,
            lot,
            indicators: sims_vm::load_indicators::Indicators {
                primary: id(&geometry_context["indicators"][0]),
                secondary: id(&geometry_context["indicators"][1]),
            },
            pools: sims_vm::indicator_pools::Pools {
                active: geometry_context["active_indicators"]
                    .as_array()
                    .context("active pool")?
                    .iter()
                    .map(id)
                    .collect(),
                spare: geometry_context["spare_indicators"]
                    .as_array()
                    .context("spare pool")?
                    .iter()
                    .map(id)
                    .collect(),
            },
            room_settings: sims_sim::lot_environment::manager::Settings {
                daylight_multiplier: 1.,
                light_range: 12.,
                rooms_enabled: true,
                models: std::array::from_fn(|i| Some(i as u8)),
            },
        };
        let mut report = json!({"scope":"Retained pre-read owner from explicit capture; host gaps stop before saved import.","input_trace":args[1],"source":house.source,"before":before,"incoming":incoming,"phases":[]});
        let result = incoming
            .prepare_read(
                &house,
                &program,
                &controllers,
                |phase, vm, removals| {
                    let mut observed = state(vm);
                    observed["pending_removals"] =
                        json!(removals.iter().map(|r| r.target).collect::<Vec<_>>());
                    report["phases"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"phase":phase,"state":observed}));
                    if phase == "resize_return" {
                        report["after"] = state(vm);
                    }
                    Ok(())
                },
                &mut |sim, _| {
                    Err(format!(
                        "startup asset service: graphic={:?}, retired={:?}",
                        sim.pending_removal_graphic(),
                        sim.retired_graphics
                    ))
                },
            )
            .and_then(|vm| house.import_saved_execution(&program, vm));
        match result {
            Ok(e) => {
                report["imported"] = state(&e.vm);
                report["imported_vm"] = serde_json::to_value(&e.vm)?;
            }
            Err(error) => report["boundary"] = json!(format!("{error:#}")),
        }
        serde_json::to_writer(file, &report)?;
        println!(
            "{}",
            json!({"phases": report["phases"].as_array().unwrap().len(),"boundary":report["boundary"]})
        );
        return Ok(());
    }
    let outcome = if resized {
        e.reconcile_global_controllers(&program, &controllers)
            .map(|r| json!(r))
    } else {
        Ok(json!({"created":[]}))
    };
    let mut report = json!({"scope":"Explicit incoming storage and installed zoning; controller reconciliation only. Incoming animator/threads and resize room work are outside this diagnostic. No teardown or saved-object handoff is substituted. A separate retained-context capture is an explicit cross-boundary input, not proof of earlier history.","input_trace":args[1],"source":house.source,"separate_retained_context":args.get(3),"magic_history":e.vm.world.magic_history,"zoning":zoning,"resized":resized,"before":before,"after":state(&e.vm)});
    match outcome {
        Ok(r) => report["reconciliation"] = r,
        Err(error) => report["boundary"] = json!(format!("{error:#}")),
    };
    serde_json::to_writer(file, &report)?;
    println!(
        "{}",
        json!({"resized":resized,"global_rng":e.vm.rng_state,"objects":e.vm.world.objects.len(),"boundary":report["boundary"]})
    );
    Ok(())
}
