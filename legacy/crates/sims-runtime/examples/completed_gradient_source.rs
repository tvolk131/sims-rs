//! Verify retained B gradient ownership and its typed cold serialization.
//! Does not enable or admit the unverified live gradient caller.
use anyhow::{Result, ensure};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::scheduler::clock::profiler::Buffer;
type Saved = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
);
fn main() -> Result<()> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let parent = root.join("fixtures/private/goth-resident-floors-complete-20261031-first");
    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(parent.join("joined-final.json"))?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(j["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone())?;
    ensure!(
        input.sha256 == "de1b1eacc85b0fb6864b12a8696b81072244522659ab7c26b05e46719617188b",
        "gradient input differs"
    );
    let store = sims_fixtures::Store::open(parent.join("cold-first.store"))?;
    let (mut p, scene, view, vm, profiler): Saved = store.get_value(&input)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let out = root.join("fixtures/private/goth-floor-gradient-20261031-first");
    p.bind_world_row_floor_sprite(&std::fs::read(out.join("caller-source-first.json"))?)?;
    let saved = (p, scene, view, vm, profiler);
    let store = sims_fixtures::Store::create(out.join("source-cold.store"))?;
    let blob = store.put_value(&saved)?;
    let mut cold: Saved = store.get_value(&blob)?;
    ensure!(
        serde_json::to_vec(&saved)? == serde_json::to_vec(&cold)?,
        "gradient cold ownership differs"
    );
    let mut warm = saved.0.clone();
    ensure!(
        !warm.advance_world_row_prefix(&saved.3)?
            && !cold.0.advance_world_row_prefix(&cold.3)?
            && warm == cold.0,
        "complete input row unexpectedly advanced after gradient binding"
    );
    ensure!(
        unchanged
            == sims_fixtures::digest(&serde_json::to_vec(&(
                &saved.1, &saved.2, &saved.3, &saved.4
            ))?),
        "gradient binding changed enclosing owners"
    );
    let report = serde_json::json!({"schema":1,"success":true,"input":input,"cold":blob,"retained_B_inputs_roundtrip_equal":true,"VM_and_RNG_unchanged":true,"live_gradient_enabled":false,"source_admission":false,"native_parity_credit":0});
    std::fs::write(
        out.join("source-cold-first.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
