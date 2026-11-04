//! Advance computed retained row owners until the next unsupported child.
//! Diagnostic only: no expected native state is supplied to the Rust advance.
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
fn main() -> Result<()> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let prior = base.join("fixtures/private/goth-terrain-clip-20261031-first");
    let out = base.join("fixtures/private/goth-row-sweep-20261031-first");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(prior.join("joined-final.json"))?)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(
        report["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone(),
    )?;
    let old = sims_fixtures::Store::open(prior.join("clip-cold-first.store"))?;
    let (mut parent, mut scene, view, vm, mut profiler): Saved = old.get_value(&input)?;
    parent.extend_world_row_depth(&std::fs::read(out.join("depth-source-first.json"))?)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let source = std::fs::read(
        base.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"),
    )?;
    let mut rows = vec![];
    let mut total = 0usize;
    let mut ended = false;
    let mut unavailable = None;
    for _ in 0..128 {
        let mut request = None;
        for _ in 0..64 {
            if let Some(r) = parent.world_paint().unwrap().tile_row() {
                request = Some(r.clone());
                break;
            }
            if !parent.advance_world_paint(&mut scene, &mut profiler)? {
                ended = true;
                break;
            }
        }
        if ended {
            break;
        }
        let request = request.context("scan request budget exhausted")?;
        parent.bind_world_row_prefix(&source)?;
        let mut finished = false;
        for _ in 0..16384 {
            total += 1;
            if parent
                .world_paint()
                .unwrap()
                .row_prefix()
                .unwrap()
                .complete()
            {
                parent.return_world_row()?;
                finished = true;
                break;
            }
            match parent.advance_world_row_prefix(&vm) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    unavailable = Some(format!("{e:#}"));
                    break;
                }
            }
        }
        let raw = serde_json::to_value(parent.world_paint().unwrap().row_prefix().unwrap())?;
        let row = serde_json::json!({"request":request,"complete":finished,"row":raw});
        eprintln!(
            "ROW count={} point={} complete={finished} pending={:?}",
            row["request"]["count"],
            row["request"]["point"],
            parent
                .world_paint()
                .unwrap()
                .row_prefix()
                .unwrap()
                .pending()
        );
        rows.push(row);
        if !finished {
            break;
        }
    }
    ensure!(
        unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
        "render-only sweep changed scene/VM/profiler"
    );
    let path = out.join("sweep-cold-first.store");
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let blob = store.put_value(&(&parent, &scene, &view, &vm, &profiler))?;
    let result = serde_json::json!({"schema":1,"input":input,"steps":total,"rows":rows,"scan_ended":ended,"unavailable":unavailable,"pending":parent.world_paint().unwrap().row_prefix().unwrap().pending(),"cold":blob,"VM_and_RNG_unchanged":true,"native_parity_credit":0,"source_admission":false,"expected_state_injected":false});
    if let Some(path) = std::env::var_os("SIMS_ROW_SWEEP_REPORT") {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&result)?)?;
    }
    println!(
        "{}",
        serde_json::json!({"steps":total,"rows":result["rows"].as_array().unwrap().len(),"pending":result["pending"],"cold":result["cold"]})
    );
    Ok(())
}
