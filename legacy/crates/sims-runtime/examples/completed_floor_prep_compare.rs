//! Actual floor preparation from the qualified Goth wall-return cut.
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
fn main() -> Result<()> {
    let base = std::env::var_os("SIMS_FLOOR_PREP_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let out = base.join("fixtures/private/goth-floor-raster-20261031-first");
    let old = base.join("fixtures/private/goth-terrain-walls-20261031-first");
    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(old.join("joined-final.json"))?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(j["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone())?;
    ensure!(
        input.sha256 == "0494068e979fc5b854412a7257a6a7c1900a9e27942f7d108f6cfb1b49894454",
        "floor baseline differs"
    );
    let input_path = std::env::var_os("SIMS_FLOOR_PREP_INPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| old.join("wall-cold-first.store"));
    let store = sims_fixtures::Store::open(input_path)?;
    let (mut parent, scene, view, vm, profiler): Saved = store.get_value(&input)?;
    let native=std::fs::read(base.join("fixtures/private/goth-floor-raster-witness-20261031-first/world-classification-trace.jsonl.gz"))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "fa5ec4fe1e71b418414a82758aa55e4e7a373f434ddfd14a5adcffb1bd9b7801",
        "floor trace differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let rows = rows
        .iter()
        .filter(|r| r["kind"] == "window_tile_scan" && r["trace_stop"].as_u64().unwrap() >= 324)
        .collect::<Vec<_>>();
    ensure!(
        rows.len() == 9
            && rows[0]["pc"] == 0x49d14bu32
            && rows.last().unwrap()["pc"] == 0x4c811fu32,
        "floor cut selection differs"
    );
    parent.compare_world_row_prefix(rows[0])?;
    parent.bind_world_row_floor(&std::fs::read(out.join("floor-source-second.json"))?)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path = std::env::var_os("SIMS_FLOOR_PREP_OUTPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| out.join("floor-cold-first.store"));
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let mut checks = vec![];
    let mut cuts = vec![];
    let mut steps = 0;
    let mut current = 0;
    let result = (|| -> Result<()> {
        for r in rows {
            let pc = r["pc"].as_u64().unwrap() as u32;
            current = pc;
            steps += parent.advance_world_row_prefix_to(&vm, r, 100000)?;
            parent.compare_world_row_prefix(r)?;
            checks.push(serde_json::json!({"pc":pc,"trace_stop":r["trace_stop"]}));
            eprintln!("FLOOR_MATCH {pc:x}");
            if matches!(pc, 0x4af453 | 0x4aed23 | 0x4af479 | 0x4affb1 | 0x4c811f) {
                let blob = store.put_value(&(&parent, &scene, &view, &vm, &profiler))?;
                let (mut cold, cs, cv, cvm, cp): Saved = store.get_value(&blob)?;
                let restored = cold.clone();
                let mut warm = parent.clone();
                let mut n = 0;
                for _ in 0..16 {
                    let a = warm.advance_world_row_prefix(&vm)?;
                    let b = cold.advance_world_row_prefix(&cvm)?;
                    ensure!(
                        a == b && warm == cold,
                        "floor warm/cold continuation differs"
                    );
                    if !a {
                        break;
                    }
                    n += 1;
                }
                ensure!(
                    scene == cs
                        && view == cv
                        && profiler == cp
                        && serde_json::to_vec(&vm)? == serde_json::to_vec(&cvm)?,
                    "floor cold owner tuple differs"
                );
                cuts.push(serde_json::json!({"pc":pc,"blob":blob,"resumed_microsteps":n}));
                // Subsequent actual boundaries execute from this decoded cut.
                parent = restored;
            }
        }
        ensure!(
            checks.len() == 9
                && unchanged
                    == sims_fixtures::digest(&serde_json::to_vec(&(
                        &scene, &view, &vm, &profiler
                    ))?),
            "floor changed VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"baseline_aligned":true,"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"steps":steps,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":unchanged==sims_fixtures::digest(&serde_json::to_vec(&(&scene,&view,&vm,&profiler))?),"next":parent.world_paint().unwrap().row_prefix().unwrap().pending(),"source_admission":false,"native_parity_credit":0,"scope":"Actual first floor preparation through sprite-call entry only. Original descriptor, brightness, catalog selection, dispatch arguments, shared context and complete raster buffers compared. Sprite drawing remains pending; no complete row35/Draw or Source/Session or completed tick admission."});
    let path = std::env::var_os("SIMS_FLOOR_PREP_REPORT").context("report output absent")?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"boundaries":checks.len(),"steps":steps,"first_divergence":report["first_divergence"]})
    );
    result
}
