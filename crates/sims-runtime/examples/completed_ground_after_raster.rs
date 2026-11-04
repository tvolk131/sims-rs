//! Diagnostic scan after the first verified raster; no expected results injected.
use anyhow::{Result, ensure};
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
fn main() -> Result<()> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = base.join("fixtures/private/goth-first-ground-raster-20261031-first");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("joined-final.json"))?)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(
        report["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone(),
    )?;
    let store = sims_fixtures::Store::open(root.join("raster-cold-first.store"))?;
    let (mut p, mut scene, view, vm, mut profiler): Saved = store.get_value(&input)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let source = std::fs::read(
        base.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"),
    )?;
    let stop = serde_json::json!({"pc":0x4b0147});
    let mut rows = vec![];
    let mut steps = 0;
    let mut error = None;
    for _ in 0..64 {
        match p.advance_world_row_prefix_to(&vm, &stop, 1000000) {
            Ok(n) => steps += n,
            Err(e) => {
                error = Some(format!("{e:#}"));
            }
        }
        let done = p.world_paint().unwrap().row_prefix().unwrap().complete();
        rows.push(serde_json::json!({"request":p.world_paint().unwrap().row_prefix().unwrap().request(),"complete":done,"pending":p.world_paint().unwrap().row_prefix().unwrap().pending()}));
        if !done || error.is_some() {
            break;
        }
        p.return_world_row()?;
        for _ in 0..64 {
            if p.world_paint().unwrap().tile_row().is_some() {
                break;
            }
            if !p.advance_world_paint(&mut scene, &mut profiler)? {
                break;
            }
        }
        if p.world_paint().unwrap().tile_row().is_none() {
            break;
        }
        p.bind_world_row_prefix(&source)?;
    }
    ensure!(
        unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
        "diagnostic changed VM/RNG/scene/profiler"
    );
    let result = serde_json::json!({"schema":1,"input":input,"steps":steps,"rows":rows,"error":error,"pending":p.world_paint().unwrap().row_prefix().unwrap().pending(),"VM_and_RNG_unchanged":true,"native_parity_credit":0,"source_admission":false,"qualification":"Authored stop marker controls diagnostic advancement only. Results beyond the first raster have not been compared with the original."});
    let path = std::env::var_os("SIMS_GROUND_AFTER_RASTER_REPORT").unwrap();
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?
        .write_all(&serde_json::to_vec_pretty(&result)?)?;
    println!("{result}");
    Ok(())
}
