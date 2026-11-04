//! Physical native owners at an observed boundary. Reads are checked against
//! that boundary; the decoder validates borrowed World coordinates and slots.
use super::*;
use crate::original_session::completed_lot;
use sims_fixtures::readset::Readset;

pub fn compare(state: &Value, memory: &Readset<'_>, sim: &sims_sim::Simulation) -> Result<Value> {
    ensure!(
        state["tick"] == sim.vm.tick && state["readset"]["sha256"] == memory.source_sha256(),
        "physical comparison source epoch differs"
    );
    let lot = completed_lot::import_captured(&state["domains"]["lot"], memory)?;
    let native = completed_lot::physical::import(&lot, &sim.vm.world)?;
    let actual = sim
        .native_lot
        .as_ref()
        .context("Rust physical owners absent")?;
    let mut checks = 0;
    let mut differences = 0;
    let mut first = Vec::new();
    for (path, expected, actual) in [
        (
            "/physical/links",
            json!(native.placement.links),
            json!(actual.placement.links),
        ),
        (
            "/physical/bounds",
            json!(native.placement.bounds),
            json!(actual.placement.bounds),
        ),
        (
            "/physical/flags",
            json!(native.placement.flags),
            json!(actual.placement.flags),
        ),
        (
            "/physical/room_membership",
            json!(native.membership),
            json!(actual.membership),
        ),
        (
            "/physical/indicators",
            json!(native.indicators),
            json!(actual.indicators),
        ),
        (
            "/physical/indicator_pools",
            json!(native.pools),
            json!(sim.native_indicator_pools),
        ),
    ] {
        super::pose::differences(
            path,
            &expected,
            &actual,
            &mut checks,
            &mut differences,
            &mut first,
        );
    }
    Ok(
        json!({"schema":1,"kind":"observed_physical_owners_diagnostic","native_parity_credit":0,
        "checks":checks,"differences":differences,"first_differences":first,
        "world_validated_objects":sim.vm.world.objects.len(),
        "qualification":"Actual tile heads, next links, physical bounds/flags, cached rooms and indicator owners. Native decoder also validates World positions, slots and native flags. Pending observers, diagnostics, route owners and complete Session are separate."}),
    )
}
