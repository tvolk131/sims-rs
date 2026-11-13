//! Continue the computed light/terrain row through shared placement and depth.
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
        base.join("fixtures/private/goth-row-context-20261031-first/joined-final.json"),
    )?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(previous["cold_cuts"][5]["blob"].clone())?;
    ensure!(
        input.sha256 == "cc3e16d8db3bfce62b39aeeb1984107d3d2aec320a662bbad89ab71528ee38ec",
        "computed scan input differs"
    );
    let store = sims_fixtures::Store::open(
        base.join("fixtures/private/goth-row-context-20261031-first/row-cold-first.store"),
    )?;
    let (mut parent, scene, view, vm, profiler): Saved = store.get_value(&input)?;
    parent.bind_world_row_placement(&std::fs::read(base.join(
        "fixtures/private/goth-row-placement-20261031-first/placement-source-first.json",
    ))?)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path =
        base.join("fixtures/private/goth-row-placement-20261031-first/placement-cold-first.store");
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let native=std::fs::read(base.join("fixtures/private/goth-row-placement-witness-20261031-first/world-classification-trace.jsonl.gz"))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "a53ef6105a311490e3b5ba485b5a6d7556a7f8d4d40ed4153ad31d760b17c7b5",
        "row witness differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
    let all = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let pcs = [
        0x4af029u32,
        0x4af194,
        0x4b9613,
        0x4b9633,
        0x4b9658,
        0x4b9660,
        0x4b97ea,
        0x4b9881,
        0x4b9883,
        0x4b9924,
        0x4b9935,
        0x454f29,
        0x454f4f,
        0x4b993a,
        0x4b997b,
        0x4c9ceb,
        0x4c9d72,
        0x4c9d9a,
        0x4c9e31,
        0x4b9980,
        0x4b9665,
        0x4af199,
    ];
    let rows: Vec<_> = all
        .iter()
        .filter(|r| {
            r["kind"] == "window_tile_scan" && pcs.contains(&(r["pc"].as_u64().unwrap() as u32))
        })
        .collect();
    ensure!(rows.len() == 22, "row native prefix coverage differs");
    let mut cuts = vec![];
    for (index, row) in rows.iter().enumerate() {
        let pc = row["pc"].as_u64().unwrap() as u32;
        for _ in 0..32 {
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
    let report = serde_json::json!({"schema":1,"input":input,"native_placement_boundaries":22,"cold_cuts":cuts,"next":parent.world_paint().unwrap().row_prefix().unwrap().pending(),"retained_vm_and_profiler_unchanged":true,"native_parity_credit":0,"source_admission":false,"expected_state_injected":false,"qualification":"Computed Rust ground selection, placement/projection and all shared depth writes through the original helper return. Rasterization, remaining row work and full Draw remain pending; no strict tick credit."});
    if let Some(path) = std::env::var_os("SIMS_ROW_PLACEMENT_REPORT") {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{}", report);
    Ok(())
}
