//! Continue the computed first row using its retained Rust room/light state.
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
    let previous: serde_json::Value = serde_json::from_slice(&std::fs::read(
        base.join("fixtures/private/goth-window-dispatch-20261031-first/joined-first.json"),
    )?)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(previous["cold_cuts"][5].clone())?;
    ensure!(
        input.sha256 == "8140222eb7869d6ecb4509df6f0bf76cc6995b99e0099c3d8ef4238895e4a885",
        "computed scan input differs"
    );
    let store = sims_fixtures::Store::open(
        base.join("fixtures/private/goth-window-dispatch-20261031-first/scan-cold-first.store"),
    )?;
    let (mut parent, scene, view, vm, profiler): Saved = store.get_value(&input)?;
    parent.bind_world_row_prefix(&std::fs::read(
        base.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"),
    )?)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path = base.join("fixtures/private/goth-row-context-20261031-first/row-cold-first.store");
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let native=std::fs::read(base.join("fixtures/private/goth-row-context-witness-20261031-first/world-classification-trace.jsonl.gz"))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "e7981c23161063f8fc674d3365deb4dd99751933a0a65b1df526729ef618f37f",
        "row witness differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
    let all = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let pcs = [
        0x4aee4cu32,
        0x4aeefb,
        0x4aef89,
        0x4aef90,
        0x4aef95,
        0x4af029,
    ];
    let rows: Vec<_> = all
        .iter()
        .filter(|r| {
            r["kind"] == "window_tile_scan" && pcs.contains(&(r["pc"].as_u64().unwrap() as u32))
        })
        .collect();
    ensure!(rows.len() == 6, "row native prefix coverage differs");
    let mut cuts = vec![];
    for (index, row) in rows.iter().enumerate() {
        let pc = row["pc"].as_u64().unwrap() as u32;
        for _ in 0..8 {
            if parent
                .world_paint()
                .unwrap()
                .row_prefix()
                .unwrap()
                .at_native_cut(pc)
            {
                break;
            }
            ensure!(
                parent.advance_world_row_prefix(&vm)?,
                "row stalled at{pc:x}"
            );
        }
        ensure!(
            parent
                .world_paint()
                .unwrap()
                .row_prefix()
                .unwrap()
                .at_native_cut(pc),
            "row cursor differs at{pc:x}"
        );
        parent.compare_world_row_prefix(row)?;
        let cut = store.put_value(&(&parent, &scene, &view, &vm, &profiler))?;
        let (mut cold, cold_scene, cold_view, cold_vm, cold_profiler): Saved =
            store.get_value(&cut)?;
        let mut warm = parent.clone();
        ensure!(
            warm.advance_world_row_prefix(&vm)? == cold.advance_world_row_prefix(&cold_vm)?,
            "row cold progress differs"
        );
        ensure!(
            warm == cold
                && scene == cold_scene
                && view == cold_view
                && serde_json::to_vec(&vm)? == serde_json::to_vec(&cold_vm)?
                && profiler == cold_profiler,
            "row cold owners differ"
        );
        cuts.push(serde_json::json!({"index":index,"pc":pc,"blob":cut}));
    }
    ensure!(
        unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
        "row changed VM/room/RNG/profiler state"
    );
    let report = serde_json::json!({"schema":1,"input":input,"native_prefix_boundaries":6,"cold_cuts":cuts,"next":parent.world_paint().unwrap().row_prefix().unwrap().pending(),"retained_vm_and_profiler_unchanged":true,"native_parity_credit":0,"source_admission":false,"expected_state_injected":false,"qualification":"Computed Rust room-byte query, both context-light stores and terrain corners only. Placement/depth, pixel row and complete draw remain pending; no strict tick credit."});
    if let Some(path) = std::env::var_os("SIMS_ROW_CONTEXT_REPORT") {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{}", report);
    Ok(())
}
