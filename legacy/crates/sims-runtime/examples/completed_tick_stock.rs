//! Exact initialized stock owners; separate from full Session admission.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{
    Saved, completed_stock, completed_tool_payloads, completed_tools,
};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 10,
        "usage: completed_tick_stock STORE REFS FIELD TOOLS THREADS REENTRY PAYLOADS CATALOGS UITEXT NEW_DIR"
    );
    let read = |p: &str| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(p)?)?) };
    let refs = read(&a[1])?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let tools = read(&a[3])?;
    let frames = read(&a[4])?;
    let reentry = read(&a[5])?;
    let payload = read(&a[6])?;
    let catalogs = read(&a[7])?;
    ensure!(
        payload["tick"] == saved.simulation.state.vm.tick,
        "stock cut differs from Session"
    );
    let native = completed_tools::import_retained_inputs(
        &tools,
        &reentry["source_closure"],
        &frames,
        &saved.simulation.state.vm.world,
    )?;
    let native = completed_tool_payloads::import(&payload, &tools, &native)?;
    let native = completed_stock::import(&payload, &catalogs, &tools, &native, &fs::read(&a[8])?)?;
    let native = serde_json::to_value(&native)?;
    let rust = serde_json::to_value(&saved.view.retained_tool_inputs)?;
    let matched = native == rust;
    let report = json!({"schema":1,"difference_count":usize::from(!matched),"comparisons":[{
        "field":"source_initialized_stock_configuration_and_shared_catalogs","matched":matched,"native":native,"rust":rust}],
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"Defined source fields, qualified empty previews/history, actual shared reason caches and ordered Move categories. Uninitialized heap bytes and allocation capacity are excluded. This does not admit the full Session."});
    let out = Path::new(&a[9]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    ensure!(matched, "initialized stock owners differ");
    Ok(())
}
