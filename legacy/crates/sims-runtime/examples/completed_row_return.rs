//! Finish the computed first fast row and return through its actual scan caller.
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
    let previous: serde_json::Value = serde_json::from_slice(&std::fs::read(
        base.join("fixtures/private/goth-ground-renderer-20261031-first/joined-final.json"),
    )?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(previous["cold_cuts"][22]["blob"].clone())?;
    ensure!(
        input.sha256 == "5528fec16ca8312d7f9e7c577062fecbf5578e2f3c386bc21dfc5e3d655b6963",
        "computed scan input differs"
    );
    let store = sims_fixtures::Store::open(
        base.join("fixtures/private/goth-ground-renderer-20261031-first/renderer-cold-first.store"),
    )?;
    let (mut parent, mut scene, view, vm, mut profiler): Saved = store.get_value(&input)?;
    parent.bind_world_row_tail(&std::fs::read(base.join(
        "fixtures/private/goth-ground-renderer-20261031-first/ground-source-first.json",
    ))?)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path = base.join("fixtures/private/goth-row-return-20261031-first/return-cold-first.store");
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let native=std::fs::read(base.join("fixtures/private/goth-ground-renderer-witness-20261031-first/world-classification-trace.jsonl.gz"))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "ec2a525944d011cc7cd4ecd462771f2942945427132d923f69eb71e68ee02ead",
        "row witness differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
    let all = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let pcs = [0x4af1c5u32, 0x4b0147, 0x4b0155, 0x4ad680];
    let rows: Vec<_> = all
        .iter()
        .filter(|r| {
            r["kind"] == "window_tile_scan" && pcs.contains(&(r["pc"].as_u64().unwrap() as u32))
        })
        .collect();
    ensure!(rows.len() == 4, "row native prefix coverage differs");
    let mut cuts = vec![];
    for (index, row) in rows.iter().enumerate() {
        let pc = row["pc"].as_u64().unwrap() as u32;
        if pc == 0x4ad680 {
            parent.return_world_row()?;
        }
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
        if pc == 0x4ad680 {
            parent.compare_world_tiles(row)?;
        }
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
    // Reuse the returned row's mutable renderer/cache owners for the next
    // callback. This is a computed frontier, not an additional native boundary.
    let last_blob: sims_fixtures::BlobRef =
        serde_json::from_value(cuts.last().unwrap()["blob"].clone())?;
    let (mut cold, mut cold_scene, cold_view, cold_vm, mut cold_profiler): Saved =
        store.get_value(&last_blob)?;
    let owners_before = serde_json::to_value(parent.world_paint().unwrap().row_prefix().unwrap())?;
    for _ in 0..24 {
        if parent.world_paint().unwrap().tile_row().is_some() {
            break;
        }
        ensure!(
            parent.advance_world_paint(&mut scene, &mut profiler)?
                == cold.advance_world_paint(&mut cold_scene, &mut cold_profiler)?,
            "next scan cold progress differs"
        );
    }
    let next_request = parent
        .world_paint()
        .unwrap()
        .tile_row()
        .context("next scan row absent")?
        .clone();
    let row_source = std::fs::read(
        base.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"),
    )?;
    parent.bind_world_row_prefix(&row_source)?;
    cold.bind_world_row_prefix(&row_source)?;
    let owners_after = serde_json::to_value(parent.world_paint().unwrap().row_prefix().unwrap())?;
    ensure!(
        owners_before["placement_source"] == owners_after["placement_source"]
            && owners_before["renderer"] == owners_after["renderer"],
        "next row reset persistent renderer/cache owners"
    );
    let mut steps = 0;
    for _ in 0..256 {
        let warm = parent.advance_world_row_prefix(&vm)?;
        ensure!(
            warm == cold.advance_world_row_prefix(&cold_vm)? && parent == cold,
            "next row cold continuation differs"
        );
        if !warm {
            break;
        }
        steps += 1;
    }
    ensure!(
        parent == cold && scene == cold_scene && view == cold_view && profiler == cold_profiler,
        "next row retained callers differ"
    );
    ensure!(
        unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
        "next row changed VM/room/RNG/profiler state"
    );
    let frontier = parent
        .world_paint()
        .unwrap()
        .row_prefix()
        .unwrap()
        .pending();
    let next_blob = store.put_value(&(&parent, &scene, &view, &vm, &profiler))?;
    let report = serde_json::json!({"schema":1,"input":input,"native_row_return_boundaries":4,"cold_cuts":cuts,"next":frontier,"computed_next_request":next_request,"computed_next_steps":steps,"computed_next_blob":next_blob,"persistent_owners_retained":true,"retained_vm_and_profiler_unchanged":true,"native_parity_credit":0,"source_admission":false,"expected_state_injected":false,"qualification":"Computed complete first fast-row return, local-only point/counter/screen stores and caller acknowledgement. Native VM/RNG unchanged. Next row and full Draw remain pending; no strict tick credit."});
    if let Some(path) = std::env::var_os("SIMS_ROW_RETURN_REPORT") {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{}", report);
    Ok(())
}
