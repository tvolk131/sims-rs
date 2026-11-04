//! Third row through its first interior terrain preparation return.
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
    let root = base.join("fixtures/private/goth-terrain-interior-20261031-first");
    let previous: serde_json::Value = serde_json::from_slice(&std::fs::read(
        base.join("fixtures/private/goth-row-return-20261031-first/second-row-final.json"),
    )?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(previous["cold_cuts"][5]["blob"].clone())?;
    ensure!(
        input.sha256 == "83776a4ecaf53aec7c5ceb70fccc0825b72e32745a4f657d5c42e400a9c2a1e2",
        "computed first return input differs"
    );
    let store = sims_fixtures::Store::open(
        base.join("fixtures/private/goth-row-return-20261031-first/second-row-cold-first.store"),
    )?;
    let (mut parent, mut scene, view, vm, mut profiler): Saved = store.get_value(&input)?;
    parent.bind_world_row_terrain(&std::fs::read(root.join("terrain-source-first.json"))?)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path = root.join("preparation-cold-first.store");
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let native = std::fs::read(base.join(
        "fixtures/private/goth-interior-terrain-witness-20261031-first/world-classification-trace.jsonl.gz",
    ))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "842c6c2dcf392b77dc49721f608cad9f962fab30929b246478ef85ee6293e5e1",
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
        .enumerate()
        .filter(|(_, r)| r["kind"] == "window_tile_scan" && r["pc"] == 0x4ad680u32)
        .nth(1)
        .context("second native row return absent")?
        .0;
    let mut cuts = vec![];
    let mut compared = 0;
    let mut bound = false;
    let mut interior = false;
    let mut requests = vec![];
    for row in rows[first_return + 1..]
        .iter()
        .filter(|r| r["kind"] == "window_tile_scan")
    {
        let pc = row["pc"].as_u64().unwrap() as u32;
        if pc == 0x48d346 {
            continue;
        }
        if pc == 0x49cf02 {
            interior = true;
        }
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
                    .at_native_observation(row)?
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
        if matches!(
            pc,
            0x4aee4c | 0x49cf02 | 0x49cfb9 | 0x49d0da | 0x49d237 | 0x4c4388
        ) {
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
        if interior && pc == 0x4c4388 {
            break;
        }
    }
    ensure!(interior, "interior delegate was not reached");
    ensure!(
        unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
        "second row changed VM/RNG/room/profiler"
    );
    let report = serde_json::json!({"schema":1,"input":input,"native_terrain_preparation_boundaries":compared,"cold_cuts":cuts,"requests":requests,"next":parent.world_paint().unwrap().row_prefix().unwrap().pending(),"persistent_owners_retained":true,"retained_vm_and_profiler_unchanged":true,"native_parity_credit":0,"source_admission":false,"expected_state_injected":false,"qualification":"Computed third-row prefix and complete first interior terrain preparation, using B map/variation inputs and live covering/pool queries. Selected cold tuple restores. Wrapper raster work and full Draw/Source/Session remain pending; no strict-tick credit."});
    if let Some(path) = std::env::var_os("SIMS_TERRAIN_PREPARATION_REPORT") {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{report}");
    Ok(())
}
