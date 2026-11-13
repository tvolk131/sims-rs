use anyhow::{Result, ensure};
use serde_json::Value;
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_epoch::lifetimes};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 5,
        "usage: completed_tick_lifetimes STORE REFS FIELD LIFETIME_PROOF NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&a[1])?)?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let before = postcard::to_allocvec(&saved.simulation.state.vm)?;
    let report = lifetimes::evaluate(&saved, &serde_json::from_slice(&fs::read(&a[3])?)?)?;
    ensure!(
        before == postcard::to_allocvec(&saved.simulation.state.vm)?,
        "native lifetime observer changed VM/RNG"
    );
    fs::create_dir(Path::new(&a[4]))?;
    fs::write(
        Path::new(&a[4]).join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "native lifetime comparisons {}, differences {}",
        report["comparison_count"], report["difference_count"]
    );
    ensure!(
        report["difference_count"] == 0,
        "native lifetime owners differ"
    );
    Ok(())
}
