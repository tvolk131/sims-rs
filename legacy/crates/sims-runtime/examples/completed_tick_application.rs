//! Compare the retained4d1901 parent with its actual original suspended frame.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_application};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 5,
        "usage: completed_tick_application STORE REFS FIELD NATIVE_JSON NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&a[1])?)?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let input: Value = serde_json::from_slice(&fs::read(&a[3])?)?;
    ensure!(
        input["tick"] == saved.simulation.state.vm.tick,
        "application source boundary differs"
    );
    let source = completed_application::import(&input)?;
    let expected = source.runtime_parent(
        saved
            .view
            .clock
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("application child absent"))?,
        saved.simulation.state.vm.tick,
    )?;
    let native = serde_json::to_value(Some(expected))?;
    let rust = serde_json::to_value(&saved.view.application_parent)?;
    let matched = native == rust;
    let report = json!({"schema":1,"tick":saved.simulation.state.vm.tick,"difference_count":usize::from(!matched),
        "comparisons":[{"field":"retained_application_flags_and_elapsed_continuation","matched":matched,"native":native,"rust":rust}],
        "source":source.source,"vm_executed":false,"native_parity_ticks":0,"complete_native_Session_admitted":false});
    let output = Path::new(&a[4]);
    fs::create_dir(output)?;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    ensure!(matched, "retained original application parent differs");
    Ok(())
}
