//! Compare the shared asset caches against actual retained source ownership.
//! No cache population, native constructor, random draw or simulation runs.
use super::super::{completed_assets, completed_lot};
use super::{Context, Result, Saved, Value, ensure, json, same};
use sims_content::Index;
#[cfg(test)]
mod tests;

pub struct Inputs<'a> {
    pub prototypes: &'a [u8],
    pub global: &'a [u8],
    pub clothing: &'a [u8],
    pub cache_proof: &'a Value,
    pub clothing_proof: &'a Value,
    pub physical: &'a Value,
}
pub fn evaluate(saved: &Saved, index: &Index, input: Inputs<'_>) -> Result<Value> {
    let Inputs {
        prototypes,
        global,
        clothing,
        cache_proof,
        clothing_proof,
        physical,
    } = input;
    ensure!(
        [88448, 88449].contains(&saved.simulation.state.vm.tick)
            && physical["tick"] == saved.simulation.state.vm.tick,
        "asset source cut differs"
    );
    const SOURCE: &str = "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e";
    const COVERAGE: &str = "e308ce89276b8d775a1820c160e495f7fde01491eea1a5b7621513ce8d18e393";
    ensure!(
        cache_proof["schema"] == 1
            && cache_proof["status"] == "native_shared_asset_caches_unchanged_qualified"
            && cache_proof["before_tick"] == 88448
            && cache_proof["after_tick"] == 88449
            && cache_proof["source_report_sha256"] == SOURCE
            && cache_proof["coverage_payload_sha256"] == COVERAGE
            && cache_proof["prototype_source_sha256"] == sims_fixtures::digest(prototypes)
            && cache_proof["global_source_sha256"] == sims_fixtures::digest(global)
            && cache_proof["target_executed"] == false,
        "shared cache source qualification differs"
    );
    let fields = cache_proof["fields"]
        .as_array()
        .context("native cache equality fields absent")?;
    ensure!(
        fields.len() == 1573
            && fields.iter().all(|r| r["before_after_equal"] == true)
            && fields
                .iter()
                .map(|r| r["written_bytes"].as_u64().unwrap_or(u64::MAX))
                .try_fold(0u64, |a, b| a.checked_add(b))
                == Some(8)
            && cache_proof["live_headers_compared_separately"]
                .as_array()
                .is_some_and(|r| r.len() == 6),
        "shared cache equality or explicit live-header boundary differs"
    );
    ensure!(
        clothing_proof["schema"] == 1
            && clothing_proof["status"] == "native_clothing_cache_unchanged_qualified"
            && clothing_proof["before_tick"] == 88448
            && clothing_proof["after_tick"] == 88449
            && clothing_proof["source_report_sha256"] == SOURCE
            && clothing_proof["coverage_payload_sha256"] == COVERAGE
            && clothing_proof["before_source_sha256"] == sims_fixtures::digest(clothing)
            && clothing_proof["target_executed"] == false,
        "clothing source qualification differs"
    );
    let cloth_fields = clothing_proof["fields"]
        .as_array()
        .context("native clothing fields absent")?;
    ensure!(
        cloth_fields.len() == 4 && cloth_fields.iter().all(|r| r["written_bytes"] == 0),
        "original clothing cache changed"
    );
    let prototype_source: Value = serde_json::from_slice(prototypes)?;
    let global_source: Value = serde_json::from_slice(global)?;
    let clothing_source: Value = serde_json::from_slice(clothing)?;
    let native = crate::conformance::completed_prototypes::import_parts(&prototype_source)?;
    native.skeletons.validate(index)?;
    let cloth = completed_assets::clothing::import(&clothing_source)?;
    let global_sources: Vec<_> = index
        .resources
        .iter()
        .filter(|r| r.name == "Global.iff" && r.archived && r.offset == 308710 && r.size == 148833)
        .collect();
    ensure!(
        global_sources.len() == 1,
        "immutable Global provider ambiguous or absent"
    );
    let provider =
        completed_assets::global::import(&global_source, index, &global_sources[0].locator)?;
    let lot = completed_lot::import(&physical["lot"])?;
    let floors = sims_content::character_loading::floors::from_builtin_coverings(
        index,
        lot.lot
            .floors
            .iter()
            .flat_map(|f| f.source.coverings.iter().copied()),
    )?;
    let settings = native
        .characters
        .residents
        .values()
        .next()
        .context("native initializer settings absent")?
        .controls
        .settings;
    ensure!(
        native
            .characters
            .residents
            .values()
            .all(|r| r.controls.settings == settings),
        "native initializer settings disagree"
    );
    let namespace = format!(
        "{}/Neighborhood.iff",
        saved
            .managed
            .as_ref()
            .context("owned mutable namespace absent")?
            .owned
            .directory
    );
    let mut rows = vec![];
    same(
        "skeletons/full_source_catalogs_and_retained_noise",
        &Some(native.skeletons),
        &saved.assets.skeletons,
        &mut rows,
    )?;
    same(
        "clothing/all_original_body_and_texture_names",
        &cloth,
        &saved.assets.clothing,
        &mut rows,
    )?;
    same(
        "autonomy_resources/full_original_global_cache_and_stream",
        &Some(provider),
        &saved.assets.autonomy_resources,
        &mut rows,
    )?;
    same(
        "floors/source_coverings_and_immutable_resource_versions",
        &floors,
        &saved.assets.floors,
        &mut rows,
    )?;
    same(
        "initializer_settings/original_process_inputs",
        &settings,
        &saved.assets.initializer_settings,
        &mut rows,
    )?;
    same(
        "neighborhood_source/owned_namespace_alias",
        &namespace,
        &saved.assets.neighborhood_source,
        &mut rows,
    )?;
    let differences = rows.iter().filter(|r| r["matched"] == false).count();
    Ok(
        json!({"schema":1,"tick":saved.simulation.state.vm.tick,"comparisons":rows,"difference_count":differences,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"Complete source-bound shared skeleton/noise, clothing, Global provider cache and stream, floor binding, initializer settings and namespace alias. Live residents, models, mesh registry, mutable metadata and complete Session admission require their separate fixed gates."}),
    )
}
