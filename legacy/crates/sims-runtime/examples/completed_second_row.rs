//! Actual second callback and both tile iterations, preserving shared owners.
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
    let root = base.join("fixtures/private/goth-row-return-20261031-first");
    let previous: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("joined-next-final.json"))?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(previous["cold_cuts"][3]["blob"].clone())?;
    ensure!(
        input.sha256 == "20f2c7c697a8c3c28095e5bcf60260d2bbbc9d7926c4adfad95863914eea85fc",
        "computed first return input differs"
    );
    let store = sims_fixtures::Store::open(root.join("return-cold-first.store"))?;
    let (mut parent, mut scene, view, vm, mut profiler): Saved = store.get_value(&input)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path = root.join("second-row-cold-first.store");
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let native = std::fs::read(base.join(
        "fixtures/private/goth-two-row-witness-20261031-second/world-classification-trace.jsonl.gz",
    ))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "1794778f6bd2baed57e6f0cbad9cf883ce78f55949ca449459933c780e0d4a6e",
        "two-row witness differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let first_return = rows
        .iter()
        .position(|r| r["kind"] == "window_tile_scan" && r["pc"] == 0x4ad680u32)
        .context("first native row return absent")?;
    let mut cuts = vec![];
    let mut compared = 0;
    let mut bound = false;
    let mut requests = vec![];
    for row in rows[first_return + 1..]
        .iter()
        .filter(|r| r["kind"] == "window_tile_scan")
    {
        let pc = row["pc"].as_u64().unwrap() as u32;
        if matches!(pc, 0x4ad6b2 | 0x4ad548 | 0x4ad55b | 0x4aee4c) {
            for _ in 0..32 {
                if parent.world_paint().unwrap().at_tile_cut(row) {
                    break;
                }
                ensure!(
                    parent.advance_world_paint(&mut scene, &mut profiler)?,
                    "second scan stalled at{pc:x}"
                );
            }
            parent.compare_world_tiles(row)?;
            if pc == 0x4aee4c {
                let before =
                    serde_json::to_value(parent.world_paint().unwrap().row_prefix().unwrap())?;
                requests.push(parent.world_paint().unwrap().tile_row().unwrap().clone());
                parent.bind_world_row_prefix(&std::fs::read(base.join(
                    "fixtures/private/goth-row-context-20261031-first/row-source-first.json",
                ))?)?;
                let after =
                    serde_json::to_value(parent.world_paint().unwrap().row_prefix().unwrap())?;
                ensure!(
                    before["placement_source"] == after["placement_source"]
                        && before["renderer"] == after["renderer"],
                    "second row reset its shared owners"
                );
                bound = true;
            }
        } else {
            ensure!(bound, "second row not bound");
            if pc == 0x4ad680 {
                parent.return_world_row()?;
            }
            for _ in 0..64 {
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
                    "second row stalled at{pc:x}"
                );
            }
            parent.compare_world_row_prefix(row)?;
            if pc == 0x4ad680 {
                parent.compare_world_tiles(row)?;
            }
        }
        compared += 1;
        // Save at the callback, both renderer entries, both loop tails, and return.
        if matches!(pc, 0x4aee4c | 0x49ce09 | 0x4b0111 | 0x4ad680) {
            let blob = store.put_value(&(&parent, &scene, &view, &vm, &profiler))?;
            let (mut cold, cold_scene, cold_view, cold_vm, cold_profiler): Saved =
                store.get_value(&blob)?;
            let mut warm = parent.clone();
            ensure!(
                warm.advance_world_row_prefix(&vm)? == cold.advance_world_row_prefix(&cold_vm)?
                    && warm == cold
                    && scene == cold_scene
                    && view == cold_view
                    && profiler == cold_profiler
                    && serde_json::to_vec(&vm)? == serde_json::to_vec(&cold_vm)?,
                "second row cold owner differs"
            );
            cuts.push(serde_json::json!({"index":compared-1,"pc":pc,"blob":blob}));
        }
    }
    ensure!(
        parent
            .world_paint()
            .unwrap()
            .row_prefix()
            .unwrap()
            .acknowledged(),
        "second row failed to return"
    );
    ensure!(
        unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
        "second row changed VM/RNG/room/profiler"
    );
    let report = serde_json::json!({"schema":1,"input":input,"native_second_row_boundaries":compared,"cold_cuts":cuts,"requests":requests,"persistent_owners_retained":true,"retained_vm_and_profiler_unchanged":true,"rows_returned":2,"native_parity_credit":0,"source_admission":false,"expected_state_injected":false,"qualification":"Original second fast-row callback and both tile iterations, through the actual caller return. Selected complete tuple cold restores. No complete Draw/Source/Session or additional completed-tick admission."});
    if let Some(path) = std::env::var_os("SIMS_SECOND_ROW_REPORT") {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{report}");
    Ok(())
}
