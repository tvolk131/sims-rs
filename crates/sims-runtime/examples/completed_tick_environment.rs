//! Complete retained room/light manager and original geometry at an actual cut.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_lot};
use std::{fs, path::Path};
fn same<T: Serialize>(name: &str, a: &T, b: &T, rows: &mut Vec<Value>) -> Result<()> {
    let x = postcard::to_allocvec(a)?;
    let y = postcard::to_allocvec(b)?;
    let mut r = json!({"field":name,"matched":x==y,"native_sha256":sims_fixtures::digest(&x),"rust_sha256":sims_fixtures::digest(&y)});
    if x != y {
        r["native"] = serde_json::to_value(a)?;
        r["rust"] = serde_json::to_value(b)?;
    }
    rows.push(r);
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 6,
        "usage: completed_tick_environment STORE REFS FIELD PHYSICAL_JSON ROOM_JSON NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let saved: Saved =
        Store::open(&args[0])?.get_value(&serde_json::from_value(refs[&args[2]].clone())?)?;
    let physical: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    let room: Value = serde_json::from_slice(&fs::read(&args[4])?)?;
    let s = &saved.simulation.state;
    ensure!(
        physical["tick"] == s.vm.tick
            && room["tick"] == s.vm.tick
            && room["schema"] == 1
            && room["executes_target_code"] == false
            && room["environment"]["provenance"]["replay_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e",
        "room source boundary differs"
    );
    let before = postcard::to_allocvec(s)?;
    let lot = completed_lot::import(&physical["lot"])?;
    let expected = completed_lot::environment::import(&lot, &room["environment"], &s.vm.world)?;
    let actual = &s
        .room_environment
        .as_ref()
        .context("retained room/light manager absent")?
        .manager;
    let architecture = s
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .context("original retained geometry absent")?;
    let mut rows = Vec::new();
    same(
        "retained_room_light_manager_all_fields",
        &expected.manager,
        actual,
        &mut rows,
    )?;
    same(
        "original_lot_all_geometry_and_room_arrays",
        &lot.lot,
        architecture,
        &mut rows,
    )?;
    ensure!(
        postcard::to_allocvec(s)? == before,
        "environment projection mutated simulation/RNG"
    );
    let differences = rows.iter().filter(|r| r["matched"] != true).count();
    let report = json!({"schema":1,"tick":s.vm.tick,"comparisons":rows,"difference_count":differences,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,"simulation_unchanged":true,
        "normalization":"Scores.published_light is a replacement diagnostic derived by the qualified importer; native stored light/environment bits, dirty/occupancy state, models, vectors and both full published arrays are retained separately.",
        "scope":"Full retained manager and original geometry; complete Session/consumer admission remains separate"});
    let out = Path::new(&args[5]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"comparisons":rows.len(),"differences":differences})
    );
    ensure!(
        differences == 0,
        "retained room/light/geometry owners differ"
    );
    Ok(())
}
