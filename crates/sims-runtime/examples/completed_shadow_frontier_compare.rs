//! An unsupported shadow child must survive cold restore without acknowledgement.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{
    State, WorldClassification, WorldComposition, WorldScene,
};
use sims_vm::scheduler::clock::profiler::Buffer;
type Saved = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
    sims_vm::interface::state::State,
    WorldClassification,
);
fn main() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join("fixtures/private/goth-visible-sprite-20261101-first");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("verified-span.json"))?)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(report["frontier"].clone())?;
    let store = sims_fixtures::Store::open(out.join("verified-span.store"))?;
    let mut state: Saved = store.get_value(&input)?;
    let mut cold: Saved = store.get_value(&input)?;
    let before = serde_json::to_vec(&state)?;
    let pending = format!(
        "{:?}",
        state
            .0
            .world_paint()
            .and_then(|p| p.row_prefix())
            .and_then(|r| r.pending())
    );
    ensure!(
        pending == "Some(Ordinary(ObjectShadow))",
        "shadow boundary differs"
    );
    ensure!(
        !state.0.advance_world_row_objects(&state.3, &mut state.6)?
            && !cold.0.advance_world_row_objects(&cold.3, &mut cold.6)?,
        "unknown shadow was acknowledged"
    );
    ensure!(
        before == serde_json::to_vec(&state)? && before == serde_json::to_vec(&cold)?,
        "pending shadow modified retained state"
    );
    let encoded = serde_json::to_vec_pretty(
        &serde_json::json!({"schema":1,"success":true,"input":input,"pending":pending,"cold_state_and_service_result_equal":true,"all_state_and_VM_RNG_unchanged":true,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0}),
    )?;
    std::fs::write(out.join("shadow-cold.json"), encoded).context("write shadow gate")?;
    println!("{{\"success\":true}}");
    Ok(())
}
