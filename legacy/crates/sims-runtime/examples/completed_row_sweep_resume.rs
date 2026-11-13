//! Resume the captured complete tuples into the next callback and row step.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{io::Write, path::PathBuf};
type Saved = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
);
fn next(
    p: &mut State,
    s: &mut WorldScene,
    b: &mut Buffer,
    vm: &sims_sim::Simulation,
    source: &[u8],
) -> Result<usize> {
    let mut steps = 0;
    for _ in 0..64 {
        if p.world_paint().unwrap().tile_row().is_some() {
            break;
        }
        ensure!(p.advance_world_paint(s, b)?, "resume scan stalled");
        steps += 1;
    }
    p.bind_world_row_prefix(source)?;
    ensure!(
        p.advance_world_row_prefix(vm)?,
        "resume row did not advance"
    );
    Ok(steps + 1)
}
fn main() -> Result<()> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = base.join("fixtures/private/goth-row-sweep-20261031-first");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("joined-first.json"))?)?;
    let store = sims_fixtures::Store::open(root.join("joined-cold-first.store"))?;
    let source = std::fs::read(
        base.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"),
    )?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(report["input"].clone())?;
    let previous = sims_fixtures::Store::open(
        base.join("fixtures/private/goth-terrain-clip-20261031-first/clip-cold-first.store"),
    )?;
    let (parent, _, _, _, _): Saved = previous.get_value(&input)?;
    let native = std::fs::read(base.join(
        "fixtures/private/goth-row-hash-witness-20261031-first/world-classification-trace.jsonl.gz",
    ))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "beaca0aac37b02b81c959d3e476d8ea857580d18eff589e91e2da2c5730fdcc7",
        "alignment trace differs"
    );
    let mut text = String::new();
    std::io::Read::read_to_string(
        &mut flate2::read::GzDecoder::new(native.as_slice()),
        &mut text,
    )?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let original = rows
        .iter()
        .filter(|r| r["kind"] == "window_tile_scan" && r["pc"] == 0x4ad680u32)
        .nth(2)
        .context("native alignment cut absent")?;
    parent.compare_world_row_prefix(original)?;
    parent.compare_world_tiles(original)?;
    let mut checks = vec![];
    for cut in report["cold_cuts"]
        .as_array()
        .context("sweep cuts absent")?
        .iter()
        .filter(|c| c["pc"] == 0x4ad680u32)
    {
        let reference: sims_fixtures::BlobRef = serde_json::from_value(cut["blob"].clone())?;
        let (mut warm, mut ws, wv, wvm, mut wp): Saved = store.get_value(&reference)?;
        let round = store.put_value(&(&warm, &ws, &wv, &wvm, &wp))?;
        let (mut cold, mut cs, cv, cvm, mut cp): Saved = store.get_value(&round)?;
        ensure!(
            reference == round,
            "stored warm tuple changed on reencoding"
        );
        let before = warm.clone();
        let a = next(&mut warm, &mut ws, &mut wp, &wvm, &source)?;
        let b = next(&mut cold, &mut cs, &mut cp, &cvm, &source)?;
        ensure!(
            a == b
                && a > 0
                && warm != before
                && warm == cold
                && ws == cs
                && wv == cv
                && wp == cp
                && serde_json::to_vec(&wvm)? == serde_json::to_vec(&cvm)?,
            "cold continuation differs"
        );
        checks.push(serde_json::json!({"row":cut["row"],"input":reference,"advance_steps":a,"state_changed":true,"complete_tuple_equal":true}));
    }
    ensure!(checks.len() == 4, "four return cuts expected");
    let result = serde_json::json!({"schema":1,"baseline_aligned":true,"checks":checks,"native_parity_credit":0,"source_admission":false,"scope":"Stored warm identity checked against the live tuple by the joined comparison; these independent restores advance the scan and enter the next row, comparing all retained state, scene, VM/RNG and profiler."});
    let path =
        std::env::var_os("SIMS_ROW_SWEEP_RESUME_REPORT").context("resume output path required")?;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?
        .write_all(&serde_json::to_vec_pretty(&result)?)?;
    println!("four nontrivial row resumes passed");
    Ok(())
}
