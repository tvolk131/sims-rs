//! Actual resident floor sprite from the computed first sprite-call cut.
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
    let base = std::env::var_os("SIMS_FLOOR_SPRITE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let out = base.join("fixtures/private/goth-floor-raster-20261031-first");
    let old = out.clone();
    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(old.join("joined-first.json"))?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(j["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone())?;
    ensure!(
        input.sha256 == "883ccc97940296947e03bde6933f55dcc670f41a48d21ed13c41ce1b91c7caa7",
        "floor baseline differs"
    );
    let input_path = std::env::var_os("SIMS_FLOOR_SPRITE_INPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| old.join("floor-cold-first.store"));
    let store = sims_fixtures::Store::open(input_path)?;
    let (mut parent, scene, view, vm, profiler): Saved = store.get_value(&input)?;
    let native=std::fs::read(base.join("fixtures/private/goth-floor-sprite-witness-20261031-first/world-classification-trace.jsonl.gz"))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "6c4389c0b7e500870b959a864025557389fb6d9e731c961f774a60fb4b65e39a",
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
        .filter(|r| r["kind"] == "window_tile_scan" && r["trace_stop"].as_u64().unwrap() >= 334)
        .collect::<Vec<_>>();
    ensure!(
        rows.len() == 7
            && rows[0]["pc"] == 0x4c811fu32
            && rows.last().unwrap()["pc"] == 0x4b0000u32,
        "floor cut selection differs"
    );
    parent.compare_world_row_prefix(rows[0])?;
    parent.bind_world_row_floor_sprite(&std::fs::read(out.join("sprite-source-third.json"))?)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path = std::env::var_os("SIMS_FLOOR_SPRITE_OUTPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| out.join("sprite-cold-first.store"));
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
            eprintln!("SPRITE_MATCH {pc:x}");
            if matches!(pc, 0x4c811f | 0x4c8e0d | 0x4c8152 | 0x49f628 | 0x4b0000) {
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
            checks.len() == 7
                && unchanged
                    == sims_fixtures::digest(&serde_json::to_vec(&(
                        &scene, &view, &vm, &profiler
                    ))?),
            "floor changed VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"baseline_aligned":true,"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"steps":steps,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":unchanged==sims_fixtures::digest(&serde_json::to_vec(&(&scene,&view,&vm,&profiler))?),"next":parent.world_paint().unwrap().row_prefix().unwrap().pending(),"source_admission":false,"native_parity_credit":0,"scope":"Actual first resident floor sprite through its return. Native resource, destination aliases, palette/depth arguments, final planes and returned bounds compared; caller remains pending. No complete row35/Draw or Source/Session or completed tick admission."});
    let path = std::env::var_os("SIMS_FLOOR_SPRITE_REPORT").context("report output absent")?;
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
