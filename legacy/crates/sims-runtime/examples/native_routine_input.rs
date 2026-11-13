//! Describe a named private BHAV from a source-bound immutable installation.
use anyhow::{Context, Result, ensure};
use sims_fixtures::Store;
use sims_runtime::conformance::native_household::Prepared;
use std::path::Path;
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 6,
        "usage: native_routine_input STORE CASE IMMUTABLE SOURCE ROUTINE AFTER_TICK"
    );
    let store = Store::open(&a[0])?;
    let case = store.load_case(&a[1])?;
    let prepared = Prepared::load(
        &store,
        &case,
        Path::new(&a[2]),
        "UserData/Houses/House05.iff",
    )?
    .context("qualified baseline unavailable")?;
    let input = prepared.routine_input(a[5].parse()?, &a[3], a[4].parse()?)?;
    println!("{}", serde_json::to_string_pretty(&vec![input])?);
    Ok(())
}
