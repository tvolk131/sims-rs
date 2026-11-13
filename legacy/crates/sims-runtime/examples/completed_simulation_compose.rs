//! Compose and cold-validate current native owners without a tick or game load.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{completed_lot, completed_simulation, completed_world};
use std::{fs, path::Path};

fn read(root: &Path, config: &Value, name: &str) -> Result<Value> {
    let relative = Path::new(config[name].as_str().context("input path absent")?);
    ensure!(
        !relative.is_absolute()
            && relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "input must be a root-relative path"
    );
    Ok(serde_json::from_slice(&fs::read(root.join(relative))?)?)
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 3,
        "usage: completed_simulation_compose SOURCE_ROOT CONFIG NEW_OUTPUT_DIR"
    );
    let root = Path::new(&args[0]);
    let config: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let refs = read(root, &config, "vm_refs")?;
    let source_store =
        Store::open(root.join(config["vm_store"].as_str().context("VM store absent")?))?;
    let original: sims_vm::Snapshot =
        source_store.get_value(&serde_json::from_value(refs["vm"].clone())?)?;
    let owner_source = read(root, &config, "owners")?;
    let owners = completed_simulation::import(&owner_source, &original)?;
    let vm = owners.with_score_owners(original)?;
    let parts = sims_runtime::conformance::completed_reentry::import_parts(read(
        root, &config, "reentry",
    )?)?;
    let systems = read(root, &config, "systems")?;
    ensure!(
        systems["provenance"]["image_sha256"] == owner_source["provenance"]["image_sha256"]
            && systems["provenance"]["observation_sha256"]
                == owner_source["provenance"]["observation_sha256"],
        "Simulation Program epoch differs"
    );
    let (program, _) = completed_world::program::join(parts.program, &systems["body_profiles"])?;
    let resident_source = read(root, &config, "residents")?;
    let (program, vm, clip_receipt) =
        completed_simulation::compose::with_current_clips(program, vm, &resident_source)?;
    let lot = completed_lot::import(&read(root, &config, "lot")?)?;
    let boundary = serde_json::from_value(read(root, &config, "boundary")?)?;
    let composed = completed_simulation::compose::import(completed_simulation::compose::Input {
        vm,
        program: &program,
        owners: &owners,
        lot: &lot,
        environment: &read(root, &config, "environment")?,
        resident_source: &resident_source,
        boundary: &boundary,
    })?;
    let out = Path::new(&args[2]);
    fs::create_dir(out)?;
    let store = Store::create(out.join("store"))?;
    let reference = store.put_value(&composed)?;
    let cold: completed_simulation::compose::Composed = store.get_value(&reference)?;
    ensure!(
        cold == composed,
        "complete Simulation storage cold restore differs"
    );
    let restored = cold
        .snapshot
        .clone()
        .restore(&program)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        restored == composed.snapshot.state,
        "complete Simulation restore differs"
    );
    fs::write(
        out.join("refs.json"),
        serde_json::to_vec_pretty(&json!({"simulation":reference}))?,
    )?;
    fs::write(
        out.join("empty-store-directories.json"),
        serde_json::to_vec_pretty(&json!(["cases", "pins", "runs", "tmp"]))?,
    )?;
    let report = json!({"schema":1,"tick":restored.vm.tick,"executed_ticks":0,"native_parity_ticks":0,
        "snapshot_version":cold.snapshot.version,"objects":restored.vm.world.objects.len(),
        "characters":restored.characters.keys().collect::<Vec<_>>(),
        "ordinary_graphics":restored.retained_ordinary_graphics,
        "bodies":restored.bodies.len(),"unplaced":restored.unplaced.len(),
        "route_vectors":restored.native_lot.as_ref().unwrap().route_paths,
        "cold_restore_identical":true,"complete_native_simulation_admitted":false,
        "session_admitted":false,"clip_join":clip_receipt,"source":cold.source});
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
