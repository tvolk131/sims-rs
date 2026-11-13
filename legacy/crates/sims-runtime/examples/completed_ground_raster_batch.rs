//! Batched row advancement compared to every saved single-step raster cut.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{
    io::{Read, Write},
    path::PathBuf,
    time::Instant,
};
type Saved = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
);
fn main() -> Result<()> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = base.join("fixtures/private/goth-first-ground-raster-20261031-first");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("joined-first.json"))?)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(report["input"].clone())?;
    let prior = sims_fixtures::Store::open(
        base.join("fixtures/private/goth-row-sweep-20261031-first/joined-cold-first.store"),
    )?;
    let (mut parent, scene, view, vm, profiler): Saved = prior.get_value(&input)?;
    parent.bind_world_row_raster(
        &std::fs::read(root.join("raster-source-second.json"))?,
        &std::fs::read(root.join("color-baseline-second.bin"))?,
        &std::fs::read(root.join("depth-baseline-second.bin"))?,
    )?;
    let encoded=std::fs::read(base.join("fixtures/private/goth-first-raster-witness-20261031-first/world-classification-trace.jsonl.gz"))?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(encoded.as_slice()).read_to_string(&mut text)?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let store = sims_fixtures::Store::open(root.join("raster-cold-first.store"))?;
    let started = Instant::now();
    let mut compared = 0;
    let mut snapshots = 0;
    let mut steps = 0;
    for r in rows
        .iter()
        .filter(|r| r["kind"] == "window_tile_scan" && r["trace_stop"].as_u64().unwrap() >= 250)
    {
        steps += parent.advance_world_row_prefix_to(&vm, r, 16384)?;
        parent.compare_world_row_prefix(r)?;
        compared += 1;
        if let Some(c) = report["cold_cuts"]
            .as_array()
            .context("single-step cuts absent")?
            .iter()
            .find(|c| c["pc"] == r["pc"])
        {
            let reference: sims_fixtures::BlobRef = serde_json::from_value(c["blob"].clone())?;
            let (p, s, v, m, b): Saved = store.get_value(&reference)?;
            ensure!(
                parent == p
                    && scene == s
                    && view == v
                    && profiler == b
                    && serde_json::to_vec(&vm)? == serde_json::to_vec(&m)?,
                "batch/single state differs at{}",
                r["pc"]
            );
            snapshots += 1;
        }
    }
    ensure!(
        compared == 15 && snapshots == 7 && steps == report["steps"].as_u64().unwrap() as usize,
        "batch coverage differs"
    );
    let result = serde_json::json!({"schema":1,"original_boundaries":compared,"single_step_snapshots_equal":snapshots,"microsteps":steps,"elapsed_ms":started.elapsed().as_millis(),"VM_RNG_yields_and_complete_tuple_equal":true,"native_parity_credit":0,"source_admission":false});
    let path = std::env::var_os("SIMS_RASTER_BATCH_REPORT").context("batch output required")?;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?
        .write_all(&serde_json::to_vec_pretty(&result)?)?;
    println!("{result}");
    Ok(())
}
