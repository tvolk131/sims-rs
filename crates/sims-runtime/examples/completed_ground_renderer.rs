//! Continue computed placement through the original first ground delegate.
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
        base.join("fixtures/private/goth-row-placement-20261031-first/joined-final.json"),
    )?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(previous["cold_cuts"][21]["blob"].clone())?;
    ensure!(
        input.sha256 == "1efe31af8ab652021b4ad9e6c798ae375780dc439eaee46640d73eb66e615b23",
        "computed scan input differs"
    );
    let store = sims_fixtures::Store::open(
        base.join("fixtures/private/goth-row-placement-20261031-first/placement-cold-first.store"),
    )?;
    let (mut parent, scene, view, vm, profiler): Saved = store.get_value(&input)?;
    parent.bind_world_row_renderer(&std::fs::read(
        base.join("fixtures/private/goth-ground-renderer-20261031-first/ground-source-first.json"),
    )?)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path =
        base.join("fixtures/private/goth-ground-renderer-20261031-first/renderer-cold-first.store");
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
    let pcs = [
        0x4af199u32,
        0x4af1c0,
        0x4c42f0,
        0x4c4319,
        0x4c434d,
        0x4c4363,
        0x49ce09,
        0x49ce29,
        0x4a48eb,
        0x49ce2e,
        0x49ce42,
        0x49ce7c,
        0x49cea4,
        0x49dfbd,
        0x49cea9,
        0x49cec1,
        0x49cecc,
        0x49d237,
        0x4c4366,
        0x4c437e,
        0x4c4388,
        0x4c4622,
        0x4af1c5,
    ];
    let rows: Vec<_> = all
        .iter()
        .filter(|r| {
            r["kind"] == "window_tile_scan" && pcs.contains(&(r["pc"].as_u64().unwrap() as u32))
        })
        .collect();
    ensure!(rows.len() == 23, "row native prefix coverage differs");
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
    let report = serde_json::json!({"schema":1,"input":input,"native_renderer_boundaries":23,"cold_cuts":cuts,"next":parent.world_paint().unwrap().row_prefix().unwrap().pending(),"retained_vm_and_profiler_unchanged":true,"native_parity_credit":0,"source_admission":false,"expected_state_injected":false,"qualification":"Computed ground wrapper and secondary common/cell/depth/flag/position/reference writes through the original edge-tile return. Interior rasterization, changed-zoom RNG lifecycle, remaining row work and full Draw remain pending; no strict tick credit."});
    if let Some(path) = std::env::var_os("SIMS_GROUND_RENDERER_REPORT") {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{}", report);
    Ok(())
}
