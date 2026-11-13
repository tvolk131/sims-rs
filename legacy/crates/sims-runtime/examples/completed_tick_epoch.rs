//! Field coverage and source-derived equality at the retained boundary.
use anyhow::{Result, ensure};
use serde_json::Value;
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_epoch};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 7,
        "usage: completed_tick_epoch STORE REFS FIELD FRONTEND REENTRY EFFECT_PROOF NEW_DIR"
    );
    let read = |p: &str| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(p)?)?) };
    let refs = read(&a[1])?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let source = read(&a[3])?;
    let closure = read(&a[4])?;
    let view = completed_epoch::view(&saved, &source, &closure["source_closure"])?;
    let simulation = completed_epoch::simulation::evaluate(&saved)?;
    let effects = completed_epoch::effects::InactiveEffects::import(read(&a[5])?)?;
    let assets = completed_epoch::assets::evaluate(&saved, &effects)?;
    let world = completed_epoch::world::evaluate(&saved)?;
    let options = completed_epoch::options::evaluate(&saved)?;
    let vm = completed_epoch::vm::evaluate(&saved)?;
    let saved_root = completed_epoch::saved::evaluate(&saved)?;
    let difference_count = [
        &view,
        &simulation,
        &assets,
        &world,
        &options,
        &vm,
        &saved_root,
    ]
    .iter()
    .map(|v| v["difference_count"].as_u64().unwrap())
    .sum::<u64>();
    let report = serde_json::json!({"schema":1,"difference_count":difference_count,
        "view":view,"simulation":simulation,"assets":assets,"world":world,"options":options,"vm":vm,"saved":saved_root,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"Completed wrapper field coverage and explicit source obligations; never substitutes for full native state equality/admission."});
    let out = Path::new(&a[6]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    ensure!(
        report["difference_count"] == 0,
        "completed View epoch differs"
    );
    Ok(())
}
