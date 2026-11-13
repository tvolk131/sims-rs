//! Join current shared asset owners; no complete Host/Session or tick admission.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{
    completed_assets, completed_ordinary, completed_simulation, completed_world,
};
use std::{fs, path::Path};
fn read(root: &Path, config: &Value, key: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(root.join(
        config[key].as_str().context("input path absent")?,
    ))?)?)
}
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 4,
        "usage: completed_assets_join SOURCE_ROOT CONFIG IMMUTABLE_ROOT NEW_DIR"
    );
    let root = Path::new(&a[0]);
    let config: Value = serde_json::from_slice(&fs::read(&a[1])?)?;
    let refs = read(root, &config, "simulation_refs")?;
    let simulation: completed_simulation::compose::Composed =
        Store::open(root.join(config["simulation_store"].as_str().unwrap()))?
            .get_value(&serde_json::from_value(refs["simulation"].clone())?)?;
    let refs = read(root, &config, "ordinary_report")?;
    let ordinary: completed_ordinary::live::ImportedLive =
        Store::open(root.join(config["ordinary_store"].as_str().unwrap()))?
            .get_value(&serde_json::from_value(refs["saved"].clone())?)?;
    let mut index = sims_content::Index::scan(Path::new(&a[2]))?;
    index.resources.retain(|r| {
        !r.locator
            .split('/')
            .any(|c| c.to_ascii_lowercase().starts_with("userdata"))
    });
    let bindings = read(root, &config, "bindings")?;
    for profile in ordinary.profile_sources["profiles"]
        .as_array()
        .context("ordinary profiles absent")?
    {
        let name = format!(
            "native-provider:{:08x}",
            profile["provider"].as_u64().unwrap()
        );
        let candidates: Vec<_> = bindings["candidate_module_bindings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["native_source"] == name)
            .collect();
        ensure!(
            candidates.len() == 1,
            "ordinary immutable provider binding ambiguous"
        );
        let b = candidates[0];
        let locator = b["ordered_component_locators"][0].as_str().unwrap();
        let mut alias = index.resources[index.find(locator)?].clone();
        ensure!(
            sims_fixtures::digest(&alias.read()?) == b["primary_content_sha256"],
            "ordinary immutable file hash differs"
        );
        alias.locator = name;
        index.resources.push(alias);
    }
    let simconfig = read(root, &config, "simulation_config")?;
    let parts = sims_runtime::conformance::completed_reentry::import_parts(read(
        root, &simconfig, "reentry",
    )?)?;
    let (mut program, _) = completed_world::program::join(
        parts.program,
        &read(root, &simconfig, "systems")?["body_profiles"],
    )?;
    let appearances = read(root, &config, "appearances")?;
    program.animations=sims_runtime::conformance::completed_animations::import_parts(
        &appearances["prototypes_input"]["resident_input"]["skin_input"]["skeleton_input"]["animation_input"])? .assets;
    ensure!(
        program.fingerprint() == simulation.snapshot.program_hash,
        "shared asset Program differs"
    );
    let global = read(root, &config, "global_provider")?;
    let global_name = format!(
        "native-provider:{:08x}",
        global["provider"]["pointer"].as_u64().unwrap()
    );
    let globals: Vec<_> = bindings["candidate_module_bindings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["native_source"] == global_name)
        .collect();
    ensure!(globals.len() == 1, "Global source binding ambiguous");
    let global_locator = globals[0]["ordered_component_locators"][0]
        .as_str()
        .unwrap();
    let global_locator = index.resources[index.find(global_locator)?]
        .locator
        .as_str();
    let joined = completed_assets::import(completed_assets::Input {
        appearances: &appearances,
        ordinary: &ordinary,
        simulation: &simulation,
        program: &program,
        index: &index,
        clothing: &read(root, &config, "clothing")?,
        global_provider: &global,
        global_locator,
    })?;
    joined.clothing.validate(&index)?;
    let out = Path::new(&a[3]);
    fs::create_dir(out)?;
    let store = Store::create(out.join("store"))?;
    let saved = store.put_value(&joined)?;
    let cold: completed_assets::Joined = store.get_value(&saved)?;
    ensure!(cold == joined, "joined asset owner cold restore differs");
    let report = json!({"schema":1,"tick":88448,"native_parity_ticks":0,"executed_ticks":0,
        "characters":joined.characters.len(),"ordinary":joined.ordinary.graphics.len(),
        "shared_meshes":joined.mesh_pool.meshes.len(),"clothing_loaded":joined.clothing.loaded(),
        "floor_bindings":joined.floors.keys().collect::<Vec<_>>(),
        "source_skeletons":joined.simulation.state.characters.values().filter(|c|c.source.is_some()).count(),
        "cold_restore_identical":true,"complete_native_host_admitted":false,"session_admitted":false,
        "source":joined.source});
    fs::write(
        out.join("refs.json"),
        serde_json::to_vec_pretty(&json!({"assets":saved}))?,
    )?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    fs::write(
        out.join("empty-store-directories.json"),
        serde_json::to_vec_pretty(&json!(["cases", "pins", "runs", "tmp"]))?,
    )?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
