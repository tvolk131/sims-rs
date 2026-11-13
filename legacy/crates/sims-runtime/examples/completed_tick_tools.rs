//! Compare retained tool/listener and picker owners with the actual source cut.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_tools};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 7,
        "usage: completed_tick_tools STORE REFS FIELD NATIVE_JSON REENTRY_JSON NEW_DIR EXPECTED_TICK"
    );
    let store = Store::open(&args[0])?;
    let refs: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let saved: Saved = store.get_value(&serde_json::from_value(refs[&args[2]].clone())?)?;
    let native: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    let reentry: Value = serde_json::from_slice(&fs::read(&args[4])?)?;
    ensure!(
        native["tick"] == saved.simulation.state.vm.tick
            && args[6].parse::<u64>()? == saved.simulation.state.vm.tick,
        "retained tool boundary differs"
    );
    let tool = completed_tools::import(&native, &reentry["source_closure"])?;
    let same =
        |name: &str, n: Value, r: Value| json!({"field":name,"matched":n==r,"native":n,"rust":r});
    let rows = vec![
        same(
            "current_tool_and_ordered_listeners",
            serde_json::to_value(Some(tool.current_tool))?,
            serde_json::to_value(&saved.view.current_tool)?,
        ),
        same(
            "picker_target",
            serde_json::to_value(tool.picker_target)?,
            serde_json::to_value(saved.view.picker_target)?,
        ),
        same(
            "picker_activation_word",
            json!(tool.picker_activation_word),
            serde_json::to_value(saved.view.picker_activation_word)?,
        ),
    ];
    let differences = rows.iter().filter(|r| r["matched"] == false).count();
    let report = json!({"schema":1,"tick":saved.simulation.state.vm.tick,"comparisons":rows,"difference_count":differences,
        "source":tool.source,"vm_executed":false,"native_parity_ticks":0,"complete_native_Session_admitted":false});
    let output = Path::new(&args[5]);
    fs::create_dir(output)?;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    ensure!(differences == 0, "retained tool fields differ");
    Ok(())
}
