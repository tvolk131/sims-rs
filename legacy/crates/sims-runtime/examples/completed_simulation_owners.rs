//! Retained native Simulation inputs; no complete Simulation/Session admission.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_runtime::original_session::completed_simulation;
use std::{fs, path::Path};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 4 || args.len() == 6 || args.len() == 7,
        "usage: completed_simulation_owners OWNERS VM_STORE VM_REFS NEW_OUTPUT_DIR [REENTRY SYSTEMS [BOUNDARY_PROOF]]"
    );
    let input = serde_json::from_slice(&fs::read(&args[0])?)?;
    let refs: Value = serde_json::from_slice(&fs::read(&args[2])?)?;
    let reference = serde_json::from_value(refs["vm"].clone())?;
    let original: sims_vm::Snapshot =
        sims_fixtures::Store::open(&args[1])?.get_value(&reference)?;
    let before = serde_json::to_vec(&original)?;
    let imported = completed_simulation::import(&input, &original)?;
    ensure!(
        serde_json::to_vec(&original)? == before,
        "retained Simulation-input import mutated VM/RNG"
    );
    let out = Path::new(&args[3]);
    fs::create_dir(out)?;
    let store = sims_fixtures::Store::create(out.join("store"))?;
    fs::write(
        out.join("empty-store-directories.json"),
        serde_json::to_vec_pretty(&json!(["cases", "pins", "runs", "tmp"]))?,
    )?;
    let saved = store.put_value(&imported)?;
    let restored: completed_simulation::Imported = store.get_value(&saved)?;
    let vm = store.put_value(&original)?;
    let cold: sims_vm::Snapshot = store.get_value(&vm)?;
    ensure!(
        restored == imported && cold == original,
        "retained Simulation inputs/VM cold restore differs"
    );
    let linked = imported.with_score_owners(original.clone())?;
    let mut stripped = linked.clone();
    for (&id, object) in &mut stripped.state.world.objects {
        object.native_motive_scores = original.state.world.objects[&id]
            .native_motive_scores
            .clone();
    }
    ensure!(
        stripped == original,
        "native score join altered other VM state"
    );
    let linked_saved = store.put_value(&linked)?;
    let linked_cold: sims_vm::Snapshot = store.get_value(&linked_saved)?;
    ensure!(
        linked_cold == linked,
        "native score owner VM cold restore differs"
    );
    let household_saved = if args.len() == 7 {
        let proof = serde_json::from_slice(&fs::read(&args[6])?)?;
        let household = imported.household(&linked, &proof)?;
        let saved = store.put_value(&household)?;
        let restored: completed_simulation::HouseholdJoin = store.get_value(&saved)?;
        ensure!(
            restored == household,
            "source household wrapper cold restore differs"
        );
        Some(saved)
    } else {
        None
    };
    if args.len() >= 6 {
        let reentry = serde_json::from_slice(&fs::read(&args[4])?)?;
        let parts = sims_runtime::conformance::completed_reentry::import_parts(reentry)?;
        let systems: Value = serde_json::from_slice(&fs::read(&args[5])?)?;
        ensure!(
            systems["provenance"]["image_sha256"] == input["provenance"]["image_sha256"]
                && systems["provenance"]["observation_sha256"]
                    == input["provenance"]["observation_sha256"],
            "score VM Program source epoch differs"
        );
        let (program, _) = sims_runtime::original_session::completed_world::program::join(
            parts.program,
            &systems["body_profiles"],
        )?;
        let vm = linked_cold.restore(&program).map_err(anyhow::Error::msg)?;
        ensure!(
            vm == linked.state,
            "validated native score VM restore changed state"
        );
    }
    fs::write(
        out.join("refs.json"),
        serde_json::to_vec_pretty(
            &json!({"owners":saved,"vm":vm,"vm_with_scores":linked_saved,"household":household_saved}),
        )?,
    )?;
    let report = json!({"tick":imported.clock.ticks,"clock":imported.clock,"statistics":imported.statistics,"valuation":imported.valuation,
        "placement_diagnostics":imported.diagnostics,"character_settings":imported.character,"scheduler_cursor":imported.scheduler_cursor,
        "fp_control":imported.fp_control,"rate_profiles":imported.rates.len(),"mood_curves":[imported.adult_mood.len(),imported.child_mood.len()],
        "personal_score_curves":imported.person_scores.iter().map(|(&id,c)|(id,c.len())).collect::<std::collections::BTreeMap<_,_>>(),
        "vm_unchanged":true,"cold_equal":true,"native_parity_ticks":0,"complete_native_simulation_admitted":false});
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"tick":imported.clock.ticks,"cold_equal":true,"vm_unchanged":true,"native_parity_ticks":0,"complete_native_simulation_admitted":false})
    );
    Ok(())
}
