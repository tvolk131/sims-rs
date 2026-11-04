use anyhow::{Result, ensure};
use serde_json::Value;
use sims_content::Index;
use sims_fixtures::Store;
use sims_runtime::original_session::{
    Saved,
    completed_epoch::source_assets::{self, Inputs},
};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 11,
        "usage: completed_tick_source_assets STORE REFS FIELD PROTOTYPES GLOBAL CLOTHING CACHE_PROOF CLOTHING_PROOF PHYSICAL IMMUTABLE NEW_DIR"
    );
    let read = |p: &str| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(p)?)?) };
    let refs = read(&a[1])?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let index = Index::scan(Path::new(&a[9]))?;
    ensure!(
        index.resources.iter().all(|r| !r
            .locator
            .split('/')
            .any(|s| s.to_ascii_lowercase().starts_with("userdata"))),
        "mutable starting files present in immutable source tree"
    );
    let report = source_assets::evaluate(
        &saved,
        &index,
        Inputs {
            prototypes: &fs::read(&a[3])?,
            global: &fs::read(&a[4])?,
            clothing: &fs::read(&a[5])?,
            cache_proof: &read(&a[6])?,
            clothing_proof: &read(&a[7])?,
            physical: &read(&a[8])?,
        },
    )?;
    let out = Path::new(&a[10]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    ensure!(
        report["difference_count"] == 0,
        "retained native asset cache or source provenance differs"
    );
    Ok(())
}
