//! Continue from the compared diagonal-floor endpoint to the next unsupported
//! child. Diagnostic advancement never grants original gameplay parity.
use anyhow::{ensure, Result};
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
    let root = base.join("fixtures/private/goth-diagonal-floor-20261031-first");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("joined-first.json"))?)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(
        report["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone(),
    )?;
    let store = sims_fixtures::Store::open(root.join("cold-first.store"))?;
    let (mut p, mut scene, view, vm, mut profiler): Saved = store.get_value(&input)?;
    let original = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let source = std::fs::read(
        base.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"),
    )?;
    let stop = serde_json::json!({"pc":0x4b0147});
    let mut rows = Vec::new();
    let mut steps = 0;
    let mut error = None;
    for _ in 0..64 {
        if p.world_paint()
            .unwrap()
            .row_prefix()
            .is_some_and(|r| r.acknowledged())
        {
            for _ in 0..512 {
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
        match p.advance_world_row_prefix_to(&vm, &stop, 1_000_000) {
            Ok(n) => steps += n,
            Err(e) => error = Some(format!("{e:#}")),
        }
        let row = p.world_paint().unwrap().row_prefix().unwrap();
        let done = row.complete();
        rows.push(
            serde_json::json!({"request":row.request(),"complete":done,"pending":row.pending()}),
        );
        if !done || error.is_some() {
            break;
        }
        p.return_world_row()?;
    }
    ensure!(
        original == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
        "diagnostic changed VM/RNG/enclosing owners"
    );
    let cursor = serde_json::to_value(p.world_paint().unwrap().row_prefix())?;
    let result = serde_json::json!({"schema":1,"input":input,"steps":steps,"rows":rows,"error":error,"cursor":cursor,"VM_and_RNG_unchanged":true,"source_admission":false,"native_parity_credit":0,"qualification":"Rust-only diagnostic after the compared row55 endpoint. Later rows and frontiers are not original evidence."});
    let path = std::env::var_os("SIMS_DIAG_DIAGNOSTIC_REPORT").unwrap();
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?
        .write_all(&serde_json::to_vec_pretty(&result)?)?;
    println!(
        "{}",
        serde_json::json!({"rows":rows.len(),"steps":steps,"error":error})
    );
    Ok(())
}
