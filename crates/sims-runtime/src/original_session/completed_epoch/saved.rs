//! Saved root coverage. Content/provenance fields require independent Program
//! reconstruction and source version checks; their mere presence is not parity.
use super::{Result, Saved, Value, fact, json, obligation};
pub fn evaluate(saved: &Saved) -> Result<Value> {
    let Saved {
        version,
        source,
        source_hashes,
        options,
        modules,
        normalized_object_definitions,
        animations,
        headline_catalog,
        simulation,
        assets,
        view,
        managed,
    } = saved;
    let mut rows = vec![];
    fact(
        "version",
        "managed_replacement_wire_schema",
        *version == 2,
        &mut rows,
    );
    fact(
        "source",
        "portable_source_identity",
        !source.is_empty(),
        &mut rows,
    );
    rows.last_mut().unwrap()["source_version_binding_required_separately"] = true.into();
    let _ = (
        source_hashes,
        options,
        modules,
        normalized_object_definitions,
        animations,
        headline_catalog,
        simulation,
        assets,
        view,
    );
    for (field, comparison) in [
        (
            "source_hashes",
            "source_file_epoch_qualification_and_owned_current_versions",
        ),
        ("options", "complete_Options_contract"),
        (
            "modules",
            "independent_Program_reconstruction_and_provider_aliases",
        ),
        (
            "normalized_object_definitions",
            "original_normalized_resident_definition_provenance",
        ),
        (
            "animations",
            "actual_clip_cache_source_closure_and_independent_Program",
        ),
        (
            "headline_catalog",
            "source_catalog_and_restore_Program_fingerprint",
        ),
        (
            "simulation",
            "complete_Simulation_VM_native_state_and_program_binding",
        ),
        (
            "assets",
            "complete_asset_native_owner_and_delivery_contract",
        ),
        ("view", "complete_View_native_owner_and_delivery_contract"),
    ] {
        obligation(field, comparison, &mut rows);
    }
    fact(
        "managed",
        "owned_current_mutable_source_namespace",
        managed.is_some(),
        &mut rows,
    );
    rows.last_mut().unwrap()["source_versions_and_no_starting_save_fallback_required_separately"] =
        true.into();
    anyhow::ensure!(rows.len() == 12, "Saved field coverage inventory changed");
    let differences = rows
        .iter()
        .filter(|r| r.get("matched") == Some(&Value::Bool(false)))
        .count();
    Ok(
        json!({"schema":1,"comparisons":rows,"difference_count":differences,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"All12 Saved root fields. Native/source equality obligations and cold Program/asset/source reconstruction are not replaced by shape checks."}),
    )
}
