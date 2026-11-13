//! Actual in-memory Session ownership and source-I/O policy. This is checked
//! after construction/independent cold restore, not inferred from Saved alone.
use super::super::{Session, completed_baseline, managed};
use super::{Result, Value, fact, json, obligation};
pub fn evaluate(session: &Session) -> Result<Value> {
    let Session {
        loaded_input_origin,
        index,
        managed_policy,
        house,
        program,
        simulation,
        host,
        options,
    } = session;
    let mut rows = vec![];
    // Runtime input observation is nonserialized diagnostics, not native equality.
    let _ = loaded_input_origin;
    rows.push(json!({"field":"loaded_input_origin",
        "kind":"nonserialized_process_local_observation_not_native_admission"}));
    let owned = index
        .resources
        .iter()
        .filter(|r| completed_baseline::mutable_locator(&r.locator))
        .all(|r| r.owned.is_some());
    fact(
        "index",
        "no_mutable_starting_file_fallback",
        owned,
        &mut rows,
    );
    fact(
        "managed_policy",
        "explicit_completed_owned_policy",
        *managed_policy == managed::Policy::CompletedOwnedSchema2,
        &mut rows,
    );
    let _ = (house, program, simulation, host, options);
    for (field, comparison) in [
        ("house", "source_bound_current_house_metadata_and_namespace"),
        (
            "program",
            "independent_exact_source_Program_and_provider_version_binding",
        ),
        (
            "simulation",
            "complete_simulation_VM_native_equality_and_delivery_contract",
        ),
        (
            "host",
            "complete_asset_and_View_native_equality_and_delivery_contract",
        ),
        ("options", "complete_Options_contract"),
    ] {
        obligation(field, comparison, &mut rows);
    }
    anyhow::ensure!(rows.len() == 8, "Session field coverage inventory changed");
    let differences = rows
        .iter()
        .filter(|r| r.get("matched") == Some(&Value::Bool(false)))
        .count();
    Ok(
        json!({"schema":1,"comparisons":rows,"difference_count":differences,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"All8 in-memory Session fields classified. Mutable resource fallback is forbidden; source/native equality and exact independent Program/asset restore remain separate mandatory obligations."}),
    )
}
