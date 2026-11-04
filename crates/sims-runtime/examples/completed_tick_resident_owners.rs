use anyhow::{Result, ensure};
use serde_json::Value;
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_epoch::residents};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 6,
        "usage: completed_tick_resident_owners STORE REFS FIELD NATIVE_PARTS CALLER_PROOF NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&a[1])?)?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let report = residents::evaluate(
        &saved,
        &serde_json::from_slice(&fs::read(&a[3])?)?,
        &serde_json::from_slice(&fs::read(&a[4])?)?,
    )?;
    fs::create_dir(Path::new(&a[5]))?;
    fs::write(
        Path::new(&a[5]).join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "Resident inventory3x15; latch comparisons {}, differences {}",
        report["comparisons"].as_array().unwrap().len(),
        report["difference_count"]
    );
    ensure!(
        report["difference_count"] == 0,
        "Resident completed caller storage differs"
    );
    Ok(())
}
