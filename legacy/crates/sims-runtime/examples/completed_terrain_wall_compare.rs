//! Computed continuation from the qualified first raster through wall reply.
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
    let base = std::env::var_os("SIMS_TERRAIN_WALL_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let out = base.join("fixtures/private/goth-terrain-walls-20261031-first");
    let prior = base.join("fixtures/private/goth-first-ground-raster-20261031-first");
    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(prior.join("joined-final.json"))?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(j["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone())?;
    ensure!(
        input.sha256 == "aee0553d008c79899c41858585aaf3447a13d5702705690f07d4e6e0711d9d8d",
        "wall input differs"
    );
    let old = sims_fixtures::Store::open(prior.join("raster-cold-first.store"))?;
    let (mut parent, mut scene, view, vm, mut profiler): Saved = old.get_value(&input)?;
    let raw = std::fs::read(base.join("fixtures/private/goth-terrain-wall-return-witness-20261031-first/world-classification-trace.jsonl.gz"))?;
    ensure!(
        sims_fixtures::digest(&raw)
            == "55a66da70d1050a1e522e65bcc8058ed70f854be324f998c00ff63a3c981b380",
        "wall native trace differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(raw.as_slice()).read_to_string(&mut text)?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let selected = rows
        .iter()
        .filter(|r| r["kind"] == "window_tile_scan" && r["trace_stop"].as_u64().unwrap() >= 251)
        .collect::<Vec<_>>();
    ensure!(
        selected.len() == 24
            && selected[0]["pc"] == 0x4c4622u32
            && selected.last().unwrap()["pc"] == 0x49d14bu32,
        "wall native cuts differ"
    );
    let source = std::fs::read(
        base.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"),
    )?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path = std::env::var_os("SIMS_TERRAIN_WALL_COLD_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| out.join("wall-cold-first.store"));
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let mut cuts = vec![];
    let mut checks = vec![];
    let mut steps = 0;
    let mut returned = 30;
    let mut current = 0;
    let result = (|| -> Result<()> {
        for native in selected {
            let pc = native["pc"].as_u64().unwrap() as u32;
            current = pc;
            if matches!(pc, 0x4c4622 | 0x4ad680 | 0x49d142 | 0x467690 | 0x49d14b) {
                steps += parent.advance_world_row_prefix_to(&vm, native, 1_000_000)?;
                if pc == 0x4ad680 {
                    ensure!(
                        parent
                            .world_paint()
                            .unwrap()
                            .row_prefix()
                            .unwrap()
                            .complete(),
                        "row return not reached"
                    );
                    parent.return_world_row()?;
                    returned += 1;
                    parent.compare_world_tiles(native)?;
                    let r =
                        serde_json::to_value(parent.world_paint().unwrap().row_prefix().unwrap())?;
                    let bytes = sims_runtime::original_session::completed_animations::bytes(
                        native["row_frame"]["raw"]
                            .as_str()
                            .context("native return locals absent")?,
                    )?;
                    let word = |i| i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
                    ensure!(
                        serde_json::json!(
                            bytes[0x214..0x217]
                                .iter()
                                .map(|v| *v as i8)
                                .collect::<Vec<_>>()
                        ) == r["tail"]["point"]
                            && r["tail"]["index"] == word(0x174)
                            && r["tail"]["screen"] == serde_json::json!([word(0x238), word(0x23c)]),
                        "native row return locals differ"
                    );
                }
                parent.compare_world_row_prefix(native)?;
            } else {
                for _ in 0..64 {
                    if parent.world_paint().unwrap().at_tile_cut(native) {
                        break;
                    }
                    ensure!(
                        parent.advance_world_paint(&mut scene, &mut profiler)?,
                        "scan stalled at{pc:x}"
                    );
                }
                parent.compare_world_tiles(native)?;
                if pc == 0x4aee4c {
                    parent.bind_world_row_prefix(&source)?;
                }
            }
            checks.push(serde_json::json!({"pc":pc,"trace_stop":native["trace_stop"],"rows_returned":returned}));
            eprintln!("WALL_MATCH {pc:x} rows{returned}");
            if pc == 0x4c4622
                || pc == 0x4ad680 && matches!(returned, 31 | 34)
                || matches!(pc, 0x49d142 | 0x467690 | 0x49d14b)
            {
                let blob = store.put_value(&(&parent, &scene, &view, &vm, &profiler))?;
                let (mut cold, cs, cv, cvm, cp): Saved = store.get_value(&blob)?;
                let mut warm = parent.clone();
                let mut n = 0;
                for _ in 0..16 {
                    let a = warm.advance_world_row_prefix(&vm)?;
                    let b = cold.advance_world_row_prefix(&cvm)?;
                    ensure!(a == b && warm == cold, "wall cold continuation differs");
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
                    "wall cold retained owners differ"
                );
                cuts.push(serde_json::json!({"pc":pc,"rows_returned":returned,"blob":blob,"resumed_microsteps":n}));
            }
        }
        ensure!(
            returned == 34 && checks.len() == 24,
            "wall continuation incomplete"
        );
        ensure!(
            unchanged
                == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
            "wall continuation changed VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"native_trace_sha256":sims_fixtures::digest(&raw),"baseline_aligned":!checks.is_empty(),"comparison_boundaries":checks,"cold_cuts":cuts,"steps":steps,"computed_row_returns":returned-30,"total_original_row_returns":returned,"next":parent.world_paint().unwrap().row_prefix().unwrap().pending(),"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene,&view,&vm,&profiler))?),"source_admission":false,"native_parity_credit":0,"scope":"First raster return through wall query return only. Actual planes, particles, renderer bytes, context, caller locals and wall result compared; VM/RNG unchanged. No completed Draw or Source/Session or completed gameplay tick admission."});
    let output = std::env::var_os("SIMS_TERRAIN_WALL_REPORT").context("output path absent")?;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"boundaries":checks.len(),"steps":steps,"rows_returned":returned,"first_divergence":report["first_divergence"]})
    );
    result
}
