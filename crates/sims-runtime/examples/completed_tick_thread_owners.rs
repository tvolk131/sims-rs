use anyhow::{Result, ensure};
use serde_json::Value;
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_epoch::thread_owners};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 7,
        "usage: completed_tick_thread_owners STORE REFS FIELD REENTRY NATIVE_THREADS NATIVE_BYTECODE NEW_DIR"
    );
    let read = |p: &str| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(p)?)?) };
    let refs = read(&a[1])?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let report = thread_owners::evaluate(&saved, read(&a[3])?, &read(&a[4])?, &read(&a[5])?)?;
    let out = Path::new(&a[6]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    ensure!(
        report["difference_count"] == 0,
        "native thread ownership or replacement delivery differs"
    );
    Ok(())
}
