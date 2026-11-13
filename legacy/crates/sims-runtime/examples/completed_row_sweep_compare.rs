//! Original row boundaries, computed from the closed third-return tuple.
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
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let prior = base.join("fixtures/private/goth-terrain-clip-20261031-first");
    let out = base.join("fixtures/private/goth-row-sweep-20261031-first");
    let old_report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(prior.join("joined-final.json"))?)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(
        old_report["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone(),
    )?;
    ensure!(
        input.sha256 == "ffdcb8dbd852866fd87c88ab281ca6ec03f7a0ac29c9a86aa03bbf4605d552ac",
        "third-return input differs"
    );
    let old = sims_fixtures::Store::open(prior.join("clip-cold-first.store"))?;
    let (mut parent, mut scene, view, vm, mut profiler): Saved = old.get_value(&input)?;
    let mut before = serde_json::to_value(parent.world_paint().unwrap().row_prefix().unwrap())?;
    parent.extend_world_row_depth(&std::fs::read(out.join("depth-source-first.json"))?)?;
    let mut after = serde_json::to_value(parent.world_paint().unwrap().row_prefix().unwrap())?;
    for v in [&mut before, &mut after] {
        let p = v["placement_source"].as_object_mut().unwrap();
        p.remove("rows");
        p.remove("wider_rows");
    }
    ensure!(
        before == after,
        "depth extension changed mutable owners/caches"
    );
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let source = std::fs::read(
        base.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"),
    )?;
    let native = std::fs::read(base.join(
        "fixtures/private/goth-row-hash-witness-20261031-first/world-classification-trace.jsonl.gz",
    ))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "beaca0aac37b02b81c959d3e476d8ea857580d18eff589e91e2da2c5730fdcc7",
        "native sweep differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let start = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r["kind"] == "window_tile_scan" && r["pc"] == 0x4ad680u32)
        .nth(2)
        .context("third original row return absent")?
        .0;
    let path = out.join("joined-cold-first.store");
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let mut cuts = vec![];
    let mut compared = 0;
    let mut returned = 3;
    let mut frontier = false;
    let mut steps = 0;
    for native in rows[start + 1..]
        .iter()
        .filter(|r| r["kind"] == "window_tile_scan")
    {
        let pc = native["pc"].as_u64().unwrap() as u32;
        if pc == 0x4ad680 || pc == 0x4c451c {
            for _ in 0..16384 {
                if pc == 0x4ad680
                    && parent
                        .world_paint()
                        .unwrap()
                        .row_prefix()
                        .unwrap()
                        .complete()
                {
                    break;
                }
                if pc == 0x4c451c
                    && parent
                        .world_paint()
                        .unwrap()
                        .row_prefix()
                        .unwrap()
                        .at_native_observation(native)?
                {
                    break;
                }
                steps += 1;
                ensure!(
                    parent.advance_world_row_prefix(&vm)?,
                    "row sweep stalled at{pc:x}, {:?}",
                    parent
                        .world_paint()
                        .unwrap()
                        .row_prefix()
                        .unwrap()
                        .pending()
                );
            }
            if pc == 0x4ad680 {
                parent.return_world_row()?;
                returned += 1;
                parent.compare_world_tiles(native)?;
                let r = serde_json::to_value(parent.world_paint().unwrap().row_prefix().unwrap())?;
                let text = native["row_frame"]["raw"]
                    .as_str()
                    .context("return row frame absent")?;
                let raw = text
                    .as_bytes()
                    .chunks_exact(2)
                    .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16))
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                let word = |i| i32::from_le_bytes(raw[i..i + 4].try_into().unwrap());
                ensure!(
                    serde_json::to_value(
                        raw[0x214..0x217]
                            .iter()
                            .map(|v| *v as i8)
                            .collect::<Vec<_>>()
                    )? == r["tail"]["point"]
                        && r["tail"]["index"] == word(0x174)
                        && r["tail"]["screen"] == serde_json::json!([word(0x238), word(0x23c)]),
                    "native row return locals differ at{returned}"
                );
                eprintln!("MATCHED_ROW {returned}");
            } else {
                frontier = true;
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
        compared += 1;
        if (pc == 0x4ad680 && [8, 16, 24, 30].contains(&returned)) || frontier {
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
                "sweep cold restore differs"
            );
            cuts.push(serde_json::json!({"row":returned,"pc":pc,"blob":blob}));
        }
        if frontier {
            break;
        }
    }
    ensure!(
        frontier && returned == 30,
        "first raster frontier was not reached"
    );
    ensure!(
        unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
        "render sweep changed VM/RNG/scene/profiler"
    );
    let report = serde_json::json!({"schema":1,"input":input,"comparison_boundaries":compared,"computed_row_returns":returned-3,"total_original_row_returns":returned,"cold_cuts":cuts,"steps":steps,"pending":parent.world_paint().unwrap().row_prefix().unwrap().pending(),"frontier_row":serde_json::to_value(parent.world_paint().unwrap().row_prefix().unwrap())?,"VM_and_RNG_unchanged":true,"mutable_caches_preserved_on_extension":true,"expected_state_injected":false,"source_admission":false,"native_parity_credit":0});
    if let Some(path) = std::env::var_os("SIMS_ROW_SWEEP_COMPARE_REPORT") {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&report)?)?;
    }
    println!(
        "{}",
        serde_json::json!({"boundaries":compared,"rows":returned,"cold":cuts.len(),"pending":report["pending"]})
    );
    Ok(())
}
