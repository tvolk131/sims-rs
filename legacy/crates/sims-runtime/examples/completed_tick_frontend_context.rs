//! Compare persistent native frontend inputs without starting publication work.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_frontend};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 7,
        "usage: completed_tick_frontend_context STORE REFS FIELD TOOLS_JSON APPLICATION_JSON FRONTEND_JSON NEW_DIR"
    );
    let read = |s: &str| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(s)?)?) };
    let refs = read(&a[1])?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let input = read(&a[5])?;
    ensure!(
        input["tick"] == saved.simulation.state.vm.tick,
        "frontend comparison cut differs"
    );
    let native = completed_frontend::import(&input, &read(&a[3])?, &read(&a[4])?)?;
    let mut rows = Vec::new();
    for (field, native, rust) in [
        (
            "persistent_native_publication_context",
            serde_json::to_value(&native.context)?,
            serde_json::to_value(&saved.view.frontend_context)?,
        ),
        (
            "CPState_camera_request_fields",
            serde_json::to_value(native.camera)?,
            serde_json::to_value(saved.view.camera_ui)?,
        ),
    ] {
        let matched = native == rust;
        rows.push(json!({"field":field,"matched":matched,"native":native,"rust":rust}));
    }
    let count = rows.iter().filter(|r| r["matched"] == false).count();
    let report = json!({"schema":1,"tick":saved.simulation.state.vm.tick,"comparisons":rows,"difference_count":count,
        "vm_executed":false,"native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"Persistent CPState/interface inputs. A publication continuation is not manufactured. Panel identity is opaque; no platform window tree enters equality."});
    let out = Path::new(&a[6]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    ensure!(count == 0, "native persistent frontend inputs differ");
    Ok(())
}
