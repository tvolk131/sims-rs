//! Actual ordinary storage composition. No tick or complete Host admission.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_runtime::original_session::{completed_ordinary, completed_skin};
use std::{fs, path::Path};
fn read(path: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 6,
        "usage: completed_ordinary_live ORDINARY PROFILES SKIN_PROOF BINDINGS IMMUTABLE_ROOT NEW_DIR"
    );
    let source = read(&args[0])?;
    let profiles = read(&args[1])?;
    ensure!(
        profiles["provenance"]["ordinary_sha256"] == sims_fixtures::digest(&fs::read(&args[0])?),
        "ordinary cache/source file hash differs"
    );
    let proof: completed_skin::Input = serde_json::from_value(read(&args[2])?)?;
    let bindings = read(&args[3])?;
    let mut index = sims_content::Index::scan(Path::new(&args[4]))?;
    index.resources.retain(|r| {
        !r.locator
            .split('/')
            .any(|c| c.to_ascii_lowercase().starts_with("userdata"))
    });
    let imported = completed_ordinary::import(
        &source["ordinary"],
        &serde_json::from_value(source["definitions"].clone())?,
        &serde_json::from_value(source["lifetimes"].clone())?,
        &index,
        "new-unpublished-ordinary-live-storage",
    )?;
    for profile in profiles["profiles"].as_array().context("profiles absent")? {
        let name = format!(
            "native-provider:{:08x}",
            profile["provider"].as_u64().context("provider absent")?
        );
        let candidates: Vec<_> = bindings["candidate_module_bindings"]
            .as_array()
            .context("module bindings absent")?
            .iter()
            .filter(|row| row["native_source"] == name)
            .collect();
        ensure!(
            candidates.len() == 1,
            "ordinary provider candidate missing/duplicated"
        );
        let locator = candidates[0]["ordered_component_locators"][0]
            .as_str()
            .context("primary locator absent")?;
        let mut alias = index.resources[index.find(locator)?].clone();
        ensure!(
            sims_fixtures::digest(&alias.read()?) == candidates[0]["primary_content_sha256"],
            "ordinary file candidate hash differs"
        );
        alias.locator = name;
        index.resources.push(alias);
    }
    let before = serde_json::to_vec(&imported)?;
    let live = completed_ordinary::live::import(
        imported,
        &serde_json::from_value(source["definitions"].clone())?,
        &profiles,
        &proof,
        &index,
    )?;
    ensure!(
        serde_json::to_vec(&live.ordinary)? == before,
        "ordinary storage assembly changed source owners"
    );
    let out = Path::new(&args[5]);
    fs::create_dir(out)?;
    let store = sims_fixtures::Store::create(out.join("store"))?;
    let reference = store.put_value(&live)?;
    let cold: completed_ordinary::live::ImportedLive = store.get_value(&reference)?;
    ensure!(
        cold == live,
        "ordinary live named MessagePack restore differs"
    );
    let registry = sims_content::graphic_loading::Registry::restore(&index, cold.registry.clone())?;
    ensure!(
        registry.snapshot() == live.registry,
        "ordinary live registry restore differs"
    );
    let report = json!({"schema":1,"status":"ordinary_live_storage_assembled","objects":registry.graphics.keys().collect::<Vec<_>>(),
        "current_profiles":profiles["profiles"].as_array().unwrap().len(),"source_unchanged":true,"cold_storage_equal":true,
        "constructors_executed":0,"ticks_executed":0,"RNG_draws":0,"native_parity_ticks":0,
        "complete_native_host_admitted":false,"remaining":live.remaining,"saved":reference});
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"objects":report["objects"],"cold_storage_equal":true,"complete_native_host_admitted":false,"native_parity_ticks":0})
    );
    Ok(())
}
