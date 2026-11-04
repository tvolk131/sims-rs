use anyhow::{Result, ensure};
use sims_fixtures::Store;
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 4,
        "usage: package_native_replay SOURCE_EXPORT RESULT_STORE TARGET_STORE CASE_ID"
    );
    let result = sims_runtime::conformance::native_household::package::build(
        &Store::open(&a[0])?,
        &Store::open(&a[1])?,
        &Store::open(&a[2])?,
        &a[3],
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
