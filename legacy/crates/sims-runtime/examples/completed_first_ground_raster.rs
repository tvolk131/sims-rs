//! First original overlapping ground raster, from the computed row-sweep cut.
use anyhow::{Result, ensure};
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
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = base.join("fixtures/private/goth-first-ground-raster-20261031-first");
    let prior = base.join("fixtures/private/goth-row-sweep-20261031-first");
    let previous: serde_json::Value =
        serde_json::from_slice(&std::fs::read(prior.join("joined-first.json"))?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(previous["cold_cuts"][4]["blob"].clone())?;
    ensure!(
        input.sha256 == "bae2573ec90fe9f17aa2e57e16c83be87e5400d5dd7204a02164894a0955ca46",
        "raster input differs"
    );
    let previous = sims_fixtures::Store::open(prior.join("joined-cold-first.store"))?;
    let (mut parent, scene, view, vm, profiler): Saved = previous.get_value(&input)?;
    let native=std::fs::read(base.join("fixtures/private/goth-first-raster-witness-20261031-first/world-classification-trace.jsonl.gz"))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "121034b77a64738883fa941f62e0bd90ab056e4e5fa730fbc7ab73ae4cb89611",
        "raster native trace differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let rows = rows
        .iter()
        .filter(|r| r["kind"] == "window_tile_scan" && r["trace_stop"].as_u64().unwrap() >= 250)
        .collect::<Vec<_>>();
    ensure!(rows.len() == 15, "raster trace cuts differ");
    parent.compare_world_row_prefix(rows[0])?;
    parent.bind_world_row_raster(
        &std::fs::read(root.join("raster-source-second.json"))?,
        &std::fs::read(root.join("color-baseline-second.bin"))?,
        &std::fs::read(root.join("depth-baseline-second.bin"))?,
    )?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path = root.join("raster-cold-first.store");
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let mut checks = 0;
    let mut cuts = vec![];
    let mut steps = 0;
    for r in rows {
        let pc = r["pc"].as_u64().unwrap() as u32;
        if std::env::var_os("SIMS_FIRST_RASTER_SINGLE_STEP").is_some() {
            for _ in 0..16384 {
                if parent
                    .world_paint()
                    .unwrap()
                    .row_prefix()
                    .unwrap()
                    .at_native_observation(r)?
                {
                    break;
                }
                ensure!(
                    parent.advance_world_row_prefix(&vm)?,
                    "raster stalled at{pc:x}, {:?}",
                    parent
                        .world_paint()
                        .unwrap()
                        .row_prefix()
                        .unwrap()
                        .pending()
                );
                steps += 1;
            }
        } else {
            steps += parent.advance_world_row_prefix_to(&vm, r, 16384)?;
        }
        parent.compare_world_row_prefix(r)?;
        checks += 1;
        eprintln!("RASTER_MATCH {pc:x}");
        if matches!(
            pc,
            0x4c451c | 0x49d7d1 | 0x49d880 | 0x49da21 | 0x49d8fb | 0x49df77 | 0x4c4622
        ) {
            let blob = store.put_value(&(&parent, &scene, &view, &vm, &profiler))?;
            let (mut cold, cs, cv, cvm, cp): Saved = store.get_value(&blob)?;
            let mut warm = parent.clone();
            ensure!(
                warm.advance_world_row_prefix(&vm)? == cold.advance_world_row_prefix(&cvm)?
                    && warm == cold
                    && scene == cs
                    && view == cv
                    && profiler == cp
                    && serde_json::to_vec(&vm)? == serde_json::to_vec(&cvm)?,
                "raster cold restore differs"
            );
            cuts.push(serde_json::json!({"pc":pc,"blob":blob}));
        }
    }
    ensure!(
        unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
        "raster changed VM/RNG/scene/profiler"
    );
    let result = serde_json::json!({"schema":1,"input":input,"baseline_aligned":true,"advance_mode":if std::env::var_os("SIMS_FIRST_RASTER_SINGLE_STEP").is_some(){"single"}else{"batch"},"comparison_boundaries":checks,"steps":steps,"cold_cuts":cuts,"next":parent.world_paint().unwrap().row_prefix().unwrap().pending(),"VM_and_RNG_unchanged":true,"baseline_inputs_only":true,"source_admission":false,"native_parity_credit":0,"scope":"First clipped ground raster only. Particle arrays and renderer descriptors computed; complete original pixel buffers compared before/after. No complete row31/Draw or Source/Session admission."});
    if let Some(path) = std::env::var_os("SIMS_FIRST_GROUND_RASTER_REPORT") {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&result)?)?;
    }
    println!(
        "{}",
        serde_json::json!({"boundaries":checks,"steps":steps,"cold":cuts.len(),"next":result["next"]})
    );
    Ok(())
}
