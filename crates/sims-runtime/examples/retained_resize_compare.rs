//! Compare actual retained startup output with a bounded observed native golden.
//! Only fields present in both declared schemas are compared; no VM patching.
mod retained_support;
use anyhow::{Context, Result, ensure};
use sims_fixtures::{BlobRef, Pin, Store};
use sims_vm::{Snapshot, Vm};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn semantic(vm: &Vm, removals: &[sims_vm::removal::Pending]) -> Result<serde_json::Value> {
    let mut objects = BTreeMap::new();
    let registry = vm
        .world
        .scheduler_storage
        .as_ref()
        .context("scheduler storage absent")?;
    for (&id, object) in &vm.world.objects {
        let thread = &vm.threads[vm.object_threads.get(&id).context("object thread absent")?];
        let frames: Vec<_> = thread
            .stack
            .iter()
            .map(|f| {
                serde_json::json!({
            "tree":f.location.bhav,"node":f.location.instruction,
            "stack_object":f.stack_object,"continuation":f.continuation,
            "args":f.args,"locals":f.locals})
            })
            .collect();
        let data = (0..72)
            .map(|i| object.data.get(&i).copied().context("object data absent"))
            .collect::<Result<Vec<_>>>()?;
        let idle = object.idle.as_ref().context("idle owner absent")?;
        objects.insert(
            id.to_string(),
            serde_json::json!({
            "guid":object.definition.as_ref().context("object definition absent")?.guid,
            "data":data,"temps":object.temps,"attributes":object.attributes,
            "last_result":thread.last_result,"frames":frames,
            "registry_word":registry.word(&vm.world,id).map_err(anyhow::Error::msg)?,
            "idle":{"countdown":idle.countdown,"flags":idle.flags},
            "position":vm.world.positions.get(&id).context("physical position absent")?}),
        );
    }
    let globals = (0..64)
        .map(|i| vm.world.globals.get(&i).copied().context("global absent"))
        .collect::<Result<Vec<_>>>()?;
    let person = vm.world.person.as_ref().context("registry absent")?;
    let resources: Vec<_> = person
        .resource_instances
        .iter()
        .map(|(guid, ids)| serde_json::json!({"guid":guid,"objects":ids}))
        .collect();
    let budget = vm.world.budget.as_ref().context("budget absent")?;
    Ok(serde_json::json!({"tick":vm.tick,"global_rng":vm.rng_state,
        "lot_rng":vm.lot_rng_state,"crt_rng":vm.crt_rng_state,
        "action_serial":vm.world.actions.as_ref().context("action state absent")?.serial,
        "globals":globals,"active_size":vm.world.map_size.context("active size absent")?.active_size,
        "objects":objects,"object_order":person.object_order,
        "capture_slot_order":vm.world.objects.keys().copied().collect::<Vec<_>>(),
        "pending_removals":removals.iter().map(|p|p.target).collect::<Vec<_>>(),
        "resource_instances":resources,"resource_globals":vm.world.resource_storage.records,
        "room_half_tiles":vm.world.room_half_tiles,"budget_balance":budget.balance,
        "budget_expenses":budget.expenses}))
}

/// The native return capture contains all active source cells and some inactive
/// cells. Its missing pages must remain unknown, never filled from entry pages.
fn backing_comparison(
    original: &Store,
    prefix: &serde_json::Value,
    golden_path: &std::ffi::OsStr,
    store: &Store,
) -> Result<(serde_json::Value, Vec<BlobRef>)> {
    type Cut = (
        sims_sim::Snapshot,
        sims_content::graphic_loading::Saved,
        BTreeMap<i16, u64>,
        Vec<serde_json::Value>,
    );
    let raw = fs::read(golden_path)?;
    let golden: serde_json::Value = serde_json::from_slice(&raw)?;
    ensure!(
        golden["schema"] == 1
            && golden["source"]["same_boundary_pages_only"] == true
            && golden["source"]["ranges"][0]["sha256"]
                == "e38aac41febccc34f0485c1955ab15cd70eb3a3915a538f546cda84058098acd"
            && golden["source"]["ranges"][1]["sha256"]
                == "ebb9e2ec916445319b3fde574869ce00283a6f343fb40fe07e16e8f70d97e211",
        "foreign backing golden"
    );
    let cuts: Vec<BlobRef> = serde_json::from_value(prefix["asset_service_checkpoints"].clone())?;
    ensure!(
        !cuts.is_empty(),
        "backing comparison needs clear service cuts"
    );
    let first: Cut = original.get_value(&cuts[0])?;
    let lot = first
        .0
        .state
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .context("retained lot backing absent")?;
    lot.validate().map_err(anyhow::Error::msg)?;
    // Clear services consume presentation state but do not read saved arrays.
    for blob in &cuts {
        let cut: Cut = original.get_value(blob)?;
        ensure!(
            cut.0
                .state
                .architecture
                .as_ref()
                .and_then(|a| a.original.as_ref())
                == Some(lot),
            "clear changed lot backing"
        );
    }
    let expected = &golden["backing"];
    let mut coverage = vec![];
    let mut cells = |name: &str,
                     values: serde_json::Value,
                     observed: &serde_json::Value|
     -> Result<serde_json::Value> {
        let values = values.as_array().context("Rust backing is not an array")?;
        let observed = observed.as_object().context("native cell map absent")?;
        ensure!(
            values.len() == lot.size * lot.size,
            "Rust backing extent differs"
        );
        for x in 0..lot.active_extent() {
            for y in 0..lot.active_extent() {
                ensure!(
                    observed.contains_key(&(x * lot.size + y).to_string()),
                    "native active cell absent:{name}:{x}:{y}"
                );
            }
        }
        let mut projected = serde_json::Map::new();
        for key in observed.keys() {
            let i: usize = key.parse()?;
            ensure!(key == &i.to_string(), "noncanonical native cell index");
            projected.insert(
                key.clone(),
                values
                    .get(i)
                    .context("native cell outside Rust backing")?
                    .clone(),
            );
        }
        coverage.push(
            serde_json::json!({"field":name,"observed_cells":observed.len(),
            "allocated_cells":values.len(),"all_active_cells_observed":true,
            "unobserved_cells":values.len()-observed.len()}),
        );
        Ok(projected.into())
    };
    let mut actual = serde_json::json!({"size":lot.size,"active_size":lot.active_extent()});
    actual["heights"] = cells(
        "heights",
        serde_json::to_value(&lot.heights)?,
        &expected["heights"],
    )?;
    actual["pool"] = cells("pool", serde_json::to_value(&lot.pool)?, &expected["pool"])?;
    actual["water"] = cells(
        "water",
        serde_json::to_value(&lot.water)?,
        &expected["water"],
    )?;
    let mut floors = vec![];
    ensure!(
        expected["floors"]
            .as_array()
            .is_some_and(|f| f.len() == lot.floors.len()),
        "native floor count differs"
    );
    for (i, floor) in lot.floors.iter().enumerate() {
        let native = &expected["floors"][i];
        let mut actual_floor = serde_json::Map::new();
        for (name, values) in [
            ("coverings", serde_json::to_value(&floor.source.coverings)?),
            ("walls", serde_json::to_value(&floor.source.walls)?),
            ("heads", serde_json::to_value(&floor.source.object_heads)?),
            // This is the live native flag array after room rebuilding, not
            // the saved input flags retained separately for validation.
            ("flags", serde_json::to_value(&floor.flags.flags)?),
        ] {
            actual_floor.insert(
                name.into(),
                cells(&format!("floor{i}.{name}"), values, &native[name])?,
            );
        }
        floors.push(serde_json::Value::Object(actual_floor));
    }
    actual["floors"] = floors.into();
    let difference = sims_runtime::fixture_replay::first_difference(expected, &actual);
    let blobs = vec![
        store.put_value(&golden)?,
        store.put_value(&actual)?,
        store.put_value(expected)?,
    ];
    Ok((
        serde_json::json!({"native_golden_sha256":sims_fixtures::digest(&raw),
        "native_golden":blobs[0],"actual":blobs[1],"expected":blobs[2],
        "coverage":coverage,"clear_service_cuts_unchanged":cuts.len(),
        "equal":difference.is_none(),"first_difference":difference,
        "unobserved_inactive_values_compared":false,"primary_room_values_compared":false}),
        blobs,
    ))
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 4 || args.len() == 5,
        "usage: retained_resize_compare PREFIX_OUTPUT REQUEST.json GOLDEN.json NEW_OUTPUT [BACKING_GOLDEN.json]"
    );
    let source = PathBuf::from(&args[0]);
    let report_raw = fs::read(source.join("report.json"))?;
    let prefix: serde_json::Value = serde_json::from_slice(&report_raw)?;
    let cleared = prefix["stop_boundary"] == "clear_return";
    let stop = if cleared {
        "clear_return"
    } else {
        "resize_return"
    };
    ensure!(
        prefix["resize_return_reached"] == true
            && prefix["result"] == format!("intentional research stop at {stop}")
            && (!cleared || prefix["clear_return_reached"] == true),
        "prefix did not reach declared stop"
    );
    let original = Store::open(source.join("store"))?;
    let program_ref: BlobRef = serde_json::from_value(prefix["linked_program"].clone())?;
    // Program is deliberately not deserializable executable input. Rebuild its
    // immutable modules with production loaders and compare the stored profile.
    let request_raw = fs::read(&args[1])?;
    ensure!(
        prefix["request_sha256"] == sims_fixtures::digest(&request_raw),
        "request changed"
    );
    let request: serde_json::Value = serde_json::from_slice(&request_raw)?;
    let vm_input: serde_json::Value = serde_json::from_slice(&fs::read(
        request["vm_input"].as_str().context("VM input")?,
    )?)?;
    let pins = std::ffi::OsStr::new(request["program_pins"].as_str().context("Program pins")?);
    let mut program = retained_support::load_program(pins, &vm_input["program"])?;
    let pins_value: serde_json::Value = serde_json::from_slice(&fs::read(pins)?)?;
    let index = sims_content::Index::scan(std::path::Path::new(
        pins_value["assets_root"].as_str().context("asset root")?,
    ))?;
    let plan_raw = fs::read(
        request["controller_plan"]
            .as_str()
            .context("controller plan")?,
    )?;
    ensure!(
        prefix["controller_plan_sha256"] == sims_fixtures::digest(&plan_raw),
        "controller plan changed"
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan_raw)?;
    let catalog = plan["controllers"]
        .as_array()
        .context("controllers")?
        .iter()
        .map(|c| {
            let label: Vec<u8> = serde_json::from_value(c["label_bytes"].clone())?;
            let words: Vec<u16> = serde_json::from_value(c["normalized_words"].clone())?;
            Ok(sims_content::catalog::Controller {
                guid: serde_json::from_value(c["guid"].clone())?,
                label: sims_formats::strings::decode(&label, 1),
                definition: sims_content::saves::DefinitionSource {
                    source: c["source"].as_str().context("controller source")?.into(),
                    sha256: c["source_sha256"]
                        .as_str()
                        .context("controller source hash")?
                        .into(),
                    chunk: serde_json::from_value(c["object"].clone())?,
                    kind: serde_json::from_value(c["kind"].clone())?,
                    multipart: *words.get(10).context("controller words")? != 0,
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    sims_content::catalog::link_controllers(&index, &mut program, &catalog)?;
    if let Some(hash) = request["career_source_sha256"].as_str() {
        let careers = sims_content::careers::load_with_expansions(&index, 0xff)?;
        ensure!(careers.content_hash == hash, "career source changed");
        program.careers = Some(careers);
    }
    original.verify_blob(&program_ref)?;
    let output = PathBuf::from(&args[3]);
    fs::create_dir(&output).context("comparison output must be new")?;
    let store = Store::create(output.join("store"))?;
    let rebuilt_program = store.put_value(&program)?;
    ensure!(
        rebuilt_program == program_ref,
        "rebuilt Program differs from stored profile"
    );
    let boundaries = prefix["checkpoints"]
        .as_array()
        .context("prefix checkpoints absent")?;
    let required = if cleared {
        vec![
            "resize_entry",
            "resize_return",
            "clear_entry",
            "clear_return",
        ]
    } else {
        vec!["resize_entry", "resize_return"]
    };
    ensure!(
        boundaries
            .iter()
            .map(|b| b["boundary"].as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            == required,
        "prefix boundary order differs"
    );
    let snapshot_ref: BlobRef =
        serde_json::from_value(boundaries.last().unwrap()["snapshot_and_removals"].clone())?;
    let (snapshot, removals): (Snapshot, Vec<sims_vm::removal::Pending>) =
        original.get_value(&snapshot_ref)?;
    let vm = snapshot
        .clone()
        .restore(&program)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        Snapshot::capture(&program, &vm) == snapshot,
        "serialized checkpoint restore differs"
    );
    let raw = fs::read(&args[2])?;
    let golden: serde_json::Value = serde_json::from_slice(&raw)?;
    ensure!(
        golden["schema"] == 1
            && if cleared {
                golden["source"]["state_and_same_boundary_page_ranges"]["clear_return"]["state"]["sha256"]
                    == "c3b9e25a85b735bf5d8de428ec45dfd0423d0f155bae70a1593479b5c853b251"
            } else {
                golden["source"]["return_row"]["sha256"]
                    == "e38aac41febccc34f0485c1955ab15cd70eb3a3915a538f546cda84058098acd"
            },
        "foreign native return golden"
    );
    let mut actual = semantic(&vm, &removals)?;
    let mut native = golden["semantic"].clone();
    if cleared {
        // The clear decoder records these source values in its room/registry
        // representation. Normalize names only; do not substitute Rust output.
        ensure!(
            native["objects"].as_object().is_some_and(|o| o.is_empty()),
            "native clear still has objects"
        );
        native["capture_slot_order"] = serde_json::json!([]);
        let mut halves = serde_json::Map::new();
        for (id, room) in native["refresh"]["rooms"]
            .as_object()
            .context("native rooms")?
        {
            halves.insert(id.clone(), room["half_tiles"].clone());
        }
        native["room_half_tiles"] = halves.into();
        let registry = vm
            .world
            .scheduler_storage
            .as_ref()
            .context("registry storage")?;
        actual["scheduler_pointer_logical_count"] = registry.len().into();
        actual["scheduler_word_logical_count"] =
            registry.word_extent().context("word extent")?.into();
        actual["registry_ui_disabled"] = vm.world.registry_ui_disabled.into();
        actual["spatial_order"] = serde_json::to_value(
            vm.world
                .registry_order
                .as_ref()
                .context("spatial registry")?,
        )?;
        let refresh = vm.world.refresh.as_ref().context("refresh state")?;
        let rooms: BTreeMap<_,_> = refresh.rooms.iter().map(|(id,r)|(id.to_string(),serde_json::json!({
            "category":r.category,"dirty":r.dirty,"outdoors":vm.world.room_outdoors.get(&(*id as i16)),
            "half_tiles":vm.world.room_half_tiles.get(&(*id as i16))}))).collect();
        actual["refresh"] = serde_json::json!({"visible_level":refresh.visible_level,
            "render_clock":refresh.render_clock,"dirty_categories":refresh.dirty_categories,"rooms":rooms});
    }
    let mut expected = serde_json::Map::new();
    for key in actual.as_object().unwrap().keys() {
        expected.insert(
            key.clone(),
            native
                .get(key)
                .with_context(|| format!("native field absent:{key}"))?
                .clone(),
        );
    }
    let expected = serde_json::Value::Object(expected);
    let difference = sims_runtime::fixture_replay::first_difference(&expected, &actual);
    let mut blobs = vec![
        rebuilt_program,
        store.put_value(&(snapshot.clone(), removals.clone()))?,
        store.put_value(&golden)?,
        store.put_value(&actual)?,
        store.put_value(&expected)?,
    ];
    let graphic_report = if let Some(value) = prefix.get("graphics_after") {
        let blob: BlobRef = serde_json::from_value(value.clone())?;
        let saved: sims_content::graphic_loading::Saved = original.get_value(&blob)?;
        let restored = sims_content::graphic_loading::Registry::restore(&index, saved.clone())?;
        restored.validate(&vm)?;
        ensure!(
            restored.snapshot() == saved,
            "final graphic registry restore differs"
        );
        if cleared {
            ensure!(
                restored.owned_objects().next().is_none()
                    && restored.pending_objects().next().is_none()
                    && !restored.updates_pending()
                    && restored.retirements.is_empty()
                    && restored.retained.survivors().len() == 2
                    && prefix["remaining_model_ids"]
                        .as_object()
                        .is_some_and(|m| m.is_empty())
                    && prefix["model_notifications"]
                        .as_array()
                        .is_some_and(|m| m.len() == 2),
                "clear retained a live graphic/model or lost survivor ownership"
            );
        }
        let saved_blob = store.put_value(&saved)?;
        blobs.push(saved_blob.clone());
        Some(serde_json::json!({"snapshot":saved_blob,"cold_equal":true,
            "live_objects":restored.owned_objects().count(),"skin_survivors":restored.retained.survivors().len(),
            "native_renderer_cache_equality_claimed":false}))
    } else {
        None
    };
    let backing = if let Some(path) = args.get(4) {
        let (report, references) = backing_comparison(&original, &prefix, path, &store)?;
        blobs.extend(references);
        Some(report)
    } else {
        None
    };
    store.pin(&Pin{schema:1,id:"retained-resize-comparison".into(),
        reason:"Bounded native resize-return comparison; full external ownership and gameplay remain unqualified".into(),blobs:blobs.clone()})?;
    let report = serde_json::json!({"schema":1,"source_report_sha256":sims_fixtures::digest(&report_raw),
        "native_golden_sha256":sims_fixtures::digest(&raw),"program":blobs[0],
        "checkpoint":blobs[1],"native_golden":blobs[2],"actual_semantic":blobs[3],"expected_semantic":blobs[4],
        "program_rebuilt_from_source_and_stored_profile_equal":true,
        "serialized_vm_and_removals_cold_equal":true,"objects":vm.world.objects.len(),
        "asset_registry":graphic_report,
        "lot_backing":backing,
        "frames":vm.threads.values().map(|t|t.stack.len()).sum::<usize>(),
        "fields_compared":actual.as_object().unwrap().keys().collect::<Vec<_>>(),
        "equal":difference.is_none(),"first_difference":difference,
        "stop_boundary":stop,
        "excluded":"native frame code-owner lifetime/arenas/caller-callee controls, external caches/renderer, complete loader continuation; post-clear word backing bytes unavailable; indicators/pools not exposed at this VM-only observer",
        "original_gameplay_ticks_added":0,"full_startup_qualified":false});
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string(&report)?);
    ensure!(
        difference.is_none(),
        "native resize-return semantic difference"
    );
    ensure!(
        report["lot_backing"].is_null() || report["lot_backing"]["equal"] == true,
        "native lot backing difference"
    );
    Ok(())
}
