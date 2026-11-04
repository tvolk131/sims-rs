//! Source-owned cTool bases; native object padding is not a logical field.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_tool_payloads, completed_tools};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 8,
        "usage: completed_tick_tool_payloads STORE REFS FIELD TOOLS_JSON THREADS_JSON REENTRY_JSON PAYLOAD_JSON NEW_DIR"
    );
    let read = |s: &str| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(s)?)?) };
    let refs = read(&a[1])?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let tools = read(&a[3])?;
    let threads = read(&a[4])?;
    let reentry = read(&a[5])?;
    let source = read(&a[6])?;
    ensure!(
        source["tick"] == saved.simulation.state.vm.tick,
        "tool base cut differs"
    );
    let state = completed_tools::import_retained_inputs(
        &tools,
        &reentry["source_closure"],
        &threads,
        &saved.simulation.state.vm.world,
    )?;
    let native = completed_tool_payloads::import(&source, &tools, &state)?;
    let native = serde_json::to_value(
        native
            .slots()
            .iter()
            .map(|s| s.base.as_ref())
            .collect::<Vec<_>>(),
    )?;
    let rust = serde_json::to_value(saved.view.retained_tool_inputs.as_ref().map(|s| {
        s.slots()
            .iter()
            .map(|s| s.base.as_ref())
            .collect::<Vec<_>>()
    }))?;
    let matched = native == rust;
    let report = json!({"schema":1,"tick":saved.simulation.state.vm.tick,"difference_count":usize::from(!matched),
        "comparisons":[{"field":"all12_actual_cTool_bases","matched":matched,"native":native,"rust":rust}],
        "vm_executed":false,"native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"Actual cTool base scalar words and tile-overlay byte. Derived payload coverage remains a separate owner audit; no undefined heap bytes become logical state."});
    let out = Path::new(&a[7]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    ensure!(matched, "actual stock-tool bases differ");
    Ok(())
}
