//! Resume a Rust-computed background cut for fast first-divergence diagnostics.
//! The input is a retained Rust cursor, never an injected native expected state.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{io::Read, path::PathBuf};
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
    let _ = view;
    println!(
        "{}",
        serde_json::json!({"computed_input":input,"native_boundaries":471,"next":parent.world_paint().unwrap().pending(),"native_parity_credit":0})
    );
    Ok(())
}
