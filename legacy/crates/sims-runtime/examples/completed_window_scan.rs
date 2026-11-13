//! Replay a Rust-computed background cut and retain the first original tile scan.
//! The input is a retained Rust cursor, never an injected native expected state.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{
    io::{Read, Write},
    path::PathBuf,
};
fn main() -> Result<()> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path=std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(||base.join("fixtures/private/goth-window-background-20261031-first/background-computed-cut-first.json"));
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        manifest["schema"] == 1
            && manifest["native_background_index"] == 67
            && manifest["native_parity_credit"] == 0,
        "computed-cut profile differs"
    );
    let root = base.join(
        manifest["store"]
            .as_str()
            .context("computed-cut store absent")?,
    );
    let store = sims_fixtures::Store::open(&root)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(manifest["input"].clone())?;

    let (mut parent, mut scene, view, vm, mut profiler): (
        State,
        WorldScene,
        WorldComposition,
        sims_sim::Simulation,
        Buffer,
    ) = store.get_value(&input)?;
    let before = serde_json::to_vec(&vm)?;
    let previous_profiler = profiler.clone();
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(base.join("fixtures/private/goth-window-background-witness-20261031-third/world-classification-trace.jsonl.gz"))?).read_to_string(&mut text)?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let pcs = [0x4acf4e, 0x4b3e06, 0x4acfb5, 0x4ad0a9, 0x4ad0ee, 0x4ad19e];
    let rows: Vec<_> = rows
        .iter()
        .filter(|r| {
            r["kind"] == "background_offset"
                || r["kind"] == "world_drawing_setup"
                    && pcs.contains(&(r["pc"].as_u64().unwrap() as u32))
        })
        .collect();
    ensure!(
        parent.world_paint().unwrap().at_background_cut(rows[67]),
        "input is not the original computed cut67"
    );
    for (index, row) in rows.into_iter().enumerate().skip(67) {
        for _ in 0..2048 {
            if parent.world_paint().unwrap().at_background_cut(row) {
                break;
            }
            ensure!(
                parent.advance_world_paint(&mut scene, &mut profiler)?,
                "stalled at{index}"
            );
        }
        parent
            .compare_world_background(row, &scene)
            .with_context(|| {
                format!(
                    "first mismatch at native background index{index}, PC{:x}",
                    row["pc"].as_u64().unwrap()
                )
            })?;
        if index < 537 {
            ensure!(
                parent.advance_world_paint(&mut scene, &mut profiler)?,
                "stalled after{index}"
            );
        }
    }
    ensure!(
        before == serde_json::to_vec(&vm)? && previous_profiler == profiler,
        "retained VM/profiler changed"
    );
    parent.bind_world_tiles(&std::fs::read(base.join(
        "fixtures/private/goth-window-dispatch-20261031-first/scan-source-first.json",
    ))?)?;
    let native=std::fs::read(base.join("fixtures/private/goth-window-scan-witness-20261031-first/world-classification-trace.jsonl.gz"))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "06f9073eb61a0e64d97839f4ef2360d646924e020d7983458321ce145e007106",
        "original scan witness differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let rows: Vec<_> = rows
        .iter()
        .filter(|r| {
            r["kind"] == "window_tile_scan"
                || r["kind"] == "world_drawing_setup"
                    && [0x4ad19eu32, 0x4ad4bd].contains(&(r["pc"].as_u64().unwrap() as u32))
        })
        .collect();
    ensure!(rows.len() == 6, "scan witness coverage differs");
    let cold_path = base.join("fixtures/private/goth-window-dispatch-20261031-first/scan-cold-first.store");
    let store = if cold_path.exists() {
        sims_fixtures::Store::open(cold_path)?
    } else {
        sims_fixtures::Store::create(cold_path)?
    };
    let mut cuts = vec![];
    for (index, row) in rows.into_iter().enumerate() {
        for _ in 0..128 {
            if parent.world_paint().unwrap().at_tile_cut(row) {
                break;
            }
            ensure!(
                parent.advance_world_paint(&mut scene, &mut profiler)?,
                "scan stalled at{index}"
            );
        }
        parent.compare_world_tiles(row)?;
        let cut = store.put_value(&(&parent, &scene, &view, &vm, &profiler))?;
        let (mut cold_parent, mut cold_scene, cold_view, cold_vm, mut cold_profiler): (
            State,
            WorldScene,
            WorldComposition,
            sims_sim::Simulation,
            Buffer,
        ) = store.get_value(&cut)?;
        if index < 5 {
            ensure!(
                parent.advance_world_paint(&mut scene, &mut profiler)?
                    && cold_parent.advance_world_paint(&mut cold_scene, &mut cold_profiler)?,
                "scan cold progress differs"
            );
        } else {
            ensure!(
                !parent.advance_world_paint(&mut scene, &mut profiler)?
                    && !cold_parent.advance_world_paint(&mut cold_scene, &mut cold_profiler)?,
                "unimplemented pixel row was acknowledged"
            );
        }
        ensure!(
            parent == cold_parent
                && scene == cold_scene
                && view == cold_view
                && profiler == cold_profiler
                && serde_json::to_vec(&vm)? == serde_json::to_vec(&cold_vm)?,
            "scan cold state differs"
        );
        cuts.push(cut);
    }
    ensure!(
        before == serde_json::to_vec(&vm)? && previous_profiler == profiler,
        "scan VM/profiler changed"
    );
    let report = serde_json::json!({"schema":1,"computed_input":input,"background_boundaries_replayed":471,"native_scan_boundaries":6,"cold_cuts":cuts,"next":parent.world_paint().unwrap().pending(),"simulation_and_profiler_unchanged":true,"expected_state_injected":false,"native_parity_credit":0,"source_admission":false,"qualification":"Rust-computed C8 midpoint continuation to first real row callback only; no complete row, paint or new strict ticks."});
    if let Some(path) = std::env::var_os("SIMS_WINDOW_SCAN_REPORT") {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{}", report);
    Ok(())
}
