//! Actual CPState/Picker/table inputs compared through retained portable owners.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_tools};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 7,
        "usage: completed_tick_tool_inputs STORE REFS FIELD TOOLS_JSON THREADS_JSON REENTRY_JSON NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&a[1])?)?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let source: Value = serde_json::from_slice(&fs::read(&a[3])?)?;
    let threads: Value = serde_json::from_slice(&fs::read(&a[4])?)?;
    let reentry: Value = serde_json::from_slice(&fs::read(&a[5])?)?;
    ensure!(
        source["tick"] == saved.simulation.state.vm.tick,
        "retained tool input cut differs"
    );
    let native = completed_tools::import_retained_inputs(
        &source,
        &reentry["source_closure"],
        &threads,
        &saved.simulation.state.vm.world,
    )?;
    let mut rows = vec![];
    let mut same = |name: &str, native: Value, rust: Value| {
        let matched = native == rust;
        rows.push(json!({"field":name,"matched":matched,"native":native,"rust":rust}));
    };
    let rust = saved.view.retained_tool_inputs.as_ref();
    same(
        "CPState_selected_tool_and_argument",
        serde_json::to_value(native.selected())?,
        serde_json::to_value(rust.map(|s| s.selected()))?,
    );
    same(
        "Picker_mode_cursor_and_original_cached_actor",
        serde_json::to_value(native.picker())?,
        serde_json::to_value(rust.map(|s| s.picker()))?,
    );
    same(
        "all12_stock_tool_handles_and_classes",
        serde_json::to_value(
            native
                .slots()
                .iter()
                .map(|s| json!({"handle":s.handle,"class":s.class}))
                .collect::<Vec<_>>(),
        )?,
        serde_json::to_value(rust.map(|s| {
            s.slots()
                .iter()
                .map(|s| json!({"handle":s.handle,"class":s.class}))
                .collect::<Vec<_>>()
        }))?,
    );
    let count = rows.iter().filter(|r| r["matched"] == false).count();
    let report = json!({"schema":1,"tick":saved.simulation.state.vm.tick,"comparisons":rows,"difference_count":count,"vm_executed":false,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"Actual source CPState/Picker/registry fields. Cached actor maps through original registry; unsupported receiver internals remain outside this projection."});
    let out = Path::new(&a[6]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    ensure!(count == 0, "native retained tool inputs differ");
    Ok(())
}
