//! Field coverage of the retained asset host; no constructors or loads run.
use super::{Result, Saved, Value, fact, json, obligation};
pub fn evaluate(saved: &Saved, effects: &super::effects::InactiveEffects) -> Result<Value> {
    let sims_content::character_loading::live::Snapshot {
        floors,
        neighborhood_source,
        initializer_settings,
        fresh,
        reloads,
        dependent_jobs,
        accessories,
        retirements,
        notification_jobs,
        notifications,
        characters,
        mesh_pool,
        clothing,
        skeletons,
        ordinary,
        autonomy_resources,
    } = &saved.assets;
    let mut rows = vec![];
    macro_rules! empty { ($($f:ident),+ $(,)?)=>{$(
        fact(stringify!($f),"unpublished_replacement_service",$f.is_empty(),&mut rows);
    )+}; }
    empty!(
        fresh,
        reloads,
        dependent_jobs,
        accessories,
        retirements,
        notification_jobs
    );
    fact(
        "skeletons",
        "required_native_prototype_cache",
        skeletons.is_some(),
        &mut rows,
    );
    fact(
        "autonomy_resources",
        "required_native_resource_owner",
        autonomy_resources.is_some(),
        &mut rows,
    );
    // This optional software font/surface renderer is distinct from the
    // captured Commander listeners in View.notifications. A missing renderer
    // never proves native presenter absence or acknowledges a reached call.
    fact(
        "notifications",
        "optional_software_renderer_with_qualified_inactive_consumers",
        effects.require_current_inputs(saved).is_ok(),
        &mut rows,
    );
    rows.last_mut().unwrap()["software_renderer_bound"] = notifications.is_some().into();
    rows.last_mut().unwrap()["native_inactive_proof_required"] = true.into();
    // Presence/shape alone cannot qualify these native data. These explicit
    // obligations prevent a wrapper-epoch pass from becoming an admission.
    let _ = (
        floors,
        initializer_settings,
        characters,
        mesh_pool,
        clothing,
        ordinary,
    );
    for (name, comparison) in [
        ("floors", "source_host/floor_context_and_lifetimes"),
        (
            "initializer_settings",
            "characters/original_animator_initialization_inputs",
        ),
        ("characters", "characters/complete_assets_and_bindings"),
        ("mesh_pool", "characters/complete_shared_mesh_pool"),
        ("clothing", "source_host/actual_clothing_catalog"),
        ("ordinary", "characters/complete_ordinary_graphic_registry"),
    ] {
        obligation(name, comparison, &mut rows);
    }
    fact(
        "neighborhood_source",
        "portable_owned_namespace",
        !neighborhood_source.is_empty(),
        &mut rows,
    );
    rows.last_mut().unwrap()["source_version_binding_required_separately"] = true.into();
    anyhow::ensure!(rows.len() == 16, "asset field coverage inventory changed");
    let differences = rows
        .iter()
        .filter(|r| r.get("matched") == Some(&Value::Bool(false)))
        .count();
    Ok(
        json!({"schema":1,"comparisons":rows,"difference_count":differences,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "effect_source":effects.source(),
        "scope":"All16 asset snapshot fields classified. Native caches/lifetimes retain their source obligations. Optional software renderer is unused only under independently regenerated native consumer proof; reached requests still require semantic services."}),
    )
}
