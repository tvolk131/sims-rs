//! Continue the verified row55 return through the descending diagonal floor.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{
    io::{Read, Write},
    path::PathBuf,
};
type Saved = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
);
fn cold_check(
    store: &sims_fixtures::Store,
    p: &State,
    scene: &WorldScene,
    view: &WorldComposition,
    vm: &sims_sim::Simulation,
    profiler: &Buffer,
    label: &str,
) -> Result<(State, serde_json::Value)> {
    let blob = store.put_value(&(p, scene, view, vm, profiler))?;
    let (mut cold, cs, cv, cvm, cp): Saved = store.get_value(&blob)?;
    let restored = cold.clone();
    let mut warm = p.clone();
    let mut n = 0;
    for _ in 0..16 {
        let a = warm.advance_world_row_prefix(vm)?;
        let b = cold.advance_world_row_prefix(&cvm)?;
        ensure!(
            a == b && warm == cold,
            "clipped warm/cold continuation differs at{label}"
        );
        if !a {
            break;
        }
        n += 1;
    }
    ensure!(
        *scene == cs
            && *view == cv
            && *profiler == cp
            && serde_json::to_vec(vm)? == serde_json::to_vec(&cvm)?,
        "clipped cold owners differ"
    );
    Ok((
        restored,
        serde_json::json!({"label":label,"blob":blob,"resumed_microsteps":n}),
    ))
}
fn cold_scan_check(
    store: &sims_fixtures::Store,
    p: &State,
    scene: &WorldScene,
    view: &WorldComposition,
    vm: &sims_sim::Simulation,
    profiler: &Buffer,
    label: &str,
) -> Result<(State, serde_json::Value)> {
    let blob = store.put_value(&(p, scene, view, vm, profiler))?;
    let (mut cold, mut cs, cv, cvm, mut cp): Saved = store.get_value(&blob)?;
    let restored = cold.clone();
    let (mut warm, mut ws, mut wp) = (p.clone(), scene.clone(), profiler.clone());
    let mut n = 0;
    for _ in 0..16 {
        if warm.world_paint().unwrap().tile_row().is_some() {
            break;
        }
        let a = warm.advance_world_paint(&mut ws, &mut wp)?;
        let b = cold.advance_world_paint(&mut cs, &mut cp)?;
        ensure!(
            a == b && warm == cold && ws == cs && wp == cp,
            "scan warm/cold continuation differs at{label}"
        );
        if !a {
            break;
        }
        n += 1;
    }
    ensure!(
        *view == cv && serde_json::to_vec(vm)? == serde_json::to_vec(&cvm)?,
        "scan cold immutable owners differ"
    );
    Ok((
        restored,
        serde_json::json!({"label":label,"blob":blob,"resumed_microsteps":n}),
    ))
}
fn main() -> Result<()> {
    let base = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let out = base.join("fixtures/private/goth-descending-floor-20261101-first");
    let old = base.join("fixtures/private/goth-diagonal-floor-20261031-first");
    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(old.join("joined-first.json"))?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(j["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone())?;
    ensure!(
        input.sha256 == "cb2b1ad4f11bea6584fbd1f56e8d9e57fdab5c75646abcdfb4c231ed41a23b24",
        "clipped input differs"
    );
    let input_store = std::env::var_os("SIMS_DIAG_INPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| old.join("cold-first.store"));
    let store = sims_fixtures::Store::open(input_store)?;
    let (mut p, mut scene, view, vm, mut profiler): Saved = store.get_value(&input)?;
    let view_source = std::fs::read(
        base.join("fixtures/private/goth-floor-raster-20261031-first/floor-source-second.json"),
    )?;
    p.bind_world_row_floor_sprite(&view_source)?;
    if let Some(path) = std::env::var_os("SIMS_DIAG_SOURCE") {
        p.bind_world_row_split(&std::fs::read(path)?)?;
    }
    let native_path = std::env::var_os("SIMS_DIAG_NATIVE").map(PathBuf::from).unwrap_or_else(|| base.join("fixtures/private/goth-descending-floor-witness-20261101-first/world-classification-trace.jsonl.gz"));
    let native = std::fs::read(native_path)?;
    let expected: serde_json::Value = serde_json::from_slice(&std::fs::read(
        std::env::var_os("SIMS_DIAG_PINS")
            .map(PathBuf::from)
            .unwrap_or_else(|| out.join("native-pins.json")),
    )?)?;
    ensure!(
        sims_fixtures::digest(&native) == expected["trace_sha256"],
        "clipped trace differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let selected = rows
        .iter()
        .filter(|r| {
            r["kind"] == "window_tile_scan"
                && r["trace_stop"].as_u64().unwrap()
                    >= expected["first_trace_stop"].as_u64().unwrap()
                && matches!(
                    r["pc"].as_u64().unwrap(),
                    0x4ad702
                        | 0x4ad715
                        | 0x4ad88f
                        | 0x467482
                        | 0x4674e4
                        | 0x467515
                        | 0x46754e
                        | 0x4671f8
                        | 0x467574
                        | 0x46767f
                        | 0x4aef90
                        | 0x4aee4c
                        | 0x4ad6b2
                        | 0x4ad548
                        | 0x4ad55b
                        | 0x4ad680
                        | 0x49d142
                        | 0x467690
                        | 0x49d14b
                        | 0x4af453
                        | 0x4aed23
                        | 0x4af479
                        | 0x4af485
                        | 0x4af4da
                        | 0x4af4e7
                        | 0x4afae4
                        | 0x4aff8d
                        | 0x4c6383
                        | 0x4affb1
                        | 0x4c811f
                        | 0x4c8e0d
                        | 0x4c8f52
                        | 0x4c8152
                        | 0x49f42d
                        | 0x49f628
                        | 0x49f949
                        | 0x4b954a
                        | 0x4c82c9
                        | 0x4b0000
                )
        })
        .collect::<Vec<_>>();
    ensure!(
        selected.first().context("clipped native cuts absent")?["pc"] == 0x4ad88fu32
            && selected.len() > 1,
        "clipped cut range differs"
    );
    let row_source = std::fs::read(
        base.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"),
    )?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path = std::env::var_os("SIMS_DIAG_OUTPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| out.join("cold-first.store"));
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let mut checks = vec![];
    let mut split_inputs = Vec::new();
    let mut cuts = vec![];
    let mut steps = 0;
    let mut returned = 55;
    let mut wall_cuts = std::collections::BTreeSet::new();
    let mut skipped_scan_tails = Vec::new();
    let mut current = 0;
    let result = (|| -> Result<()> {
        for (ordinal, r) in selected.into_iter().enumerate() {
            let pc = r["pc"].as_u64().unwrap() as u32;
            current = pc;
            //4ad88f also follows a vertically clipped row without a callback.
            let scan_only = pc == 0x4ad88f
                && ordinal != 0
                && p.world_paint()
                    .unwrap()
                    .row_prefix()
                    .is_some_and(|v| v.acknowledged());
            if scan_only {
                for _ in 0..64 {
                    if p.world_paint().unwrap().at_tile_cut(r) {
                        break;
                    }
                    ensure!(
                        p.advance_world_paint(&mut scene, &mut profiler)?,
                        "clipped-only scan stalled"
                    );
                }
                p.compare_world_tiles(r)?;
                // Compare the unchanged planes/context owned by the last
                // completed child, without reading deallocated child locals.
                p.compare_world_row_prefix(r)?;
                returned += 1;
                skipped_scan_tails.push(r["trace_stop"].clone());
            } else if matches!(
                pc,
                0x4aee4c | 0x4ad6b2 | 0x4ad548 | 0x4ad55b | 0x4ad702 | 0x4ad715
            ) {
                for _ in 0..64 {
                    if p.world_paint().unwrap().at_tile_cut(r) {
                        break;
                    }
                    ensure!(
                        p.advance_world_paint(&mut scene, &mut profiler)?,
                        "clipped scan stalled"
                    );
                }
                p.compare_world_tiles(r)?;
                if pc == 0x4aee4c {
                    p.bind_world_row_prefix(&row_source)?;
                }
            } else {
                steps += p.advance_world_row_prefix_to(&vm, r, 1000000)?;
                if matches!(pc, 0x4ad680 | 0x4ad88f) && ordinal != 0 {
                    ensure!(
                        p.world_paint().unwrap().row_prefix().unwrap().complete(),
                        "clipped row return not reached"
                    );
                    p.return_world_row()?;
                    returned += 1;
                    p.compare_world_tiles(r)?;
                    if pc == 0x4ad680 || r["row_frame"]["raw"].is_string() {
                        let state =
                            serde_json::to_value(p.world_paint().unwrap().row_prefix().unwrap())?;
                        let bytes = sims_runtime::original_session::completed_animations::bytes(
                            r["row_frame"]["raw"]
                                .as_str()
                                .context("clipped return locals absent")?,
                        )?;
                        let word = |i| i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
                        ensure!(
                            serde_json::json!(
                                bytes[0x214..0x217]
                                    .iter()
                                    .map(|v| *v as i8)
                                    .collect::<Vec<_>>()
                            ) == state["tail"]["point"]
                                && state["tail"]["index"] == word(0x174)
                                && state["tail"]["screen"]
                                    == serde_json::json!([word(0x238), word(0x23c)]),
                            "clipped return locals differ"
                        );
                    }
                    // The earlier captures omit deallocated child stack bytes
                    // at the second-pass caller return. Never synthesize them.
                }
                p.compare_world_row_prefix(r)?;
            }
            if pc == 0x4aef90 {
                split_inputs.push(
                    p.world_paint()
                        .unwrap()
                        .row_prefix()
                        .unwrap()
                        .split_inputs(&vm)?,
                );
            }
            checks.push(
                serde_json::json!({"pc":pc,"trace_stop":r["trace_stop"],"rows_returned":returned,"dead_child_return_locals_observed": !matches!(pc,0x4ad680|0x4ad88f) || r["row_frame"]["raw"].is_string()}),
            );
            eprintln!("RESIDENT_MATCH {pc:x} rows{returned}");
            let wall = matches!(pc, 0x467690 | 0x4af4da | 0x4af4e7 | 0x4afae4)
                && r.get("floor_wall_result").is_some()
                || matches!(pc, 0x4af4e7 | 0x4afae4);
            // Sample each first wall phase, an intervening row, and the endpoint.
            let wall = wall
                || matches!(pc, 0x4b954a | 0x4c82c9)
                || pc == 0x467690
                    && r["stack"]["raw"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("daf44a00"));
            let last = r["trace_stop"] == expected["last_trace_stop"];
            if p.world_paint()
                .unwrap()
                .row_prefix()
                .unwrap()
                .diagonal_suspended()
                && matches!(pc, 0x4671f8 | 0x4c6383 | 0x4c8152)
                || matches!(pc, 0x4674e4 | 0x46754e | 0x467574 | 0x46767f)
                || wall && wall_cuts.insert(pc)
                || matches!(pc, 0x4ad680 | 0x4ad88f)
                || last
            {
                let check = if scan_only {
                    cold_scan_check
                } else {
                    cold_check
                };
                let (restored, cut) = check(
                    &store,
                    &p,
                    &scene,
                    &view,
                    &vm,
                    &profiler,
                    &format!("{pc:x}-rows{returned}"),
                )?;
                cuts.push(cut);
                p = restored;
            }
        }
        ensure!(
            !wall_cuts.is_empty()
                && unchanged
                    == sims_fixtures::digest(&serde_json::to_vec(&(
                        &scene, &view, &vm, &profiler
                    ))?),
            "clipped changed VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let mut report = serde_json::json!({"schema":1,"success":result.is_ok(),"split_inputs":split_inputs,"input":input,"additional_B_view_source_sha256":sims_fixtures::digest(&view_source),"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"steps":steps,"rows_returned":returned,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene,&view,&vm,&profiler))?),"source_admission":false,"native_parity_credit":0,"scope":"Actual computed row55 caller return through the descending diagonal floor row return; no completed gameplay-tick admission. Deallocated second-pass child stack locals, unconsumed expanded-wall words and x87 exception status are not supplied or admitted. Shared contexts, consumed wall segment words, resource calls, returned bounds and full color/depth buffers compared. Consumed wall flags and diagonal covering words derive from the retained lot; unused expanded-record bytes are not fabricated or admitted. All nine resident resources and both palette tables are imported from B, with live default/cached palette inputs and result checks; palette rebuild/conversion remains unsupported. No completed WorldView drawing or Source/Session or completed-tick admission."});
    if !skipped_scan_tails.is_empty() {
        report["scan_tail_only_observations"] = serde_json::json!(skipped_scan_tails);
        report["actual_row_callbacks_returned"] =
            serde_json::json!(returned - skipped_scan_tails.len());
        report["rows_returned_is_legacy_scan_tail_counter"] = serde_json::json!(true);
    }
    let output = std::env::var_os("SIMS_DIAG_REPORT").context("clipped report output absent")?;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"boundaries":checks.len(),"rows_returned":returned,"steps":steps,"first_divergence":report["first_divergence"]})
    );
    result
}
