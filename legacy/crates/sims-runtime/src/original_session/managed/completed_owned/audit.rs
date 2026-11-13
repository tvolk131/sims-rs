//! Compare current owned bytes and independently parsed metadata; never use an
//! installed starting file or rebuild the live world from historical metadata.
use super::*;
use crate::original_session::completed_epoch::same;
use serde_json::{Value, json};

pub(in crate::original_session) fn evaluate(
    session: &Session,
    qualification: &Value,
) -> Result<Value> {
    let before_vm = postcard::to_allocvec(&session.simulation.vm)?;
    let saved = session.save_managed()?;
    let content = saved
        .managed
        .as_ref()
        .context("current managed source namespace absent")?;
    ensure!(
        saved.version == 2 && content.schema == 2,
        "source audit requires the complete owned schema2"
    );
    ensure!(
        qualification["schema"] == 1
            && qualification["qualification_uses_new_guest_bytes"] == false
            && qualification["prior_private_candidate_hashes_matched"] == 3
            && qualification["provenance"]["image"]["compressed_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f",
        "original kernel-backed mutable file qualification differs"
    );
    let native = qualification["files"]
        .as_array()
        .context("native current file cohort absent")?;
    ensure!(native.len() == 5, "original mutable source cohort differs");
    content.owned.validate()?;
    ensure!(
        content.owned.entries.len() == native.len(),
        "owned namespace loses/adds native current sources"
    );
    let mut native_hashes = BTreeMap::new();
    for row in native {
        let locator = row["locator"]
            .as_str()
            .context("native current locator absent")?;
        let hash = row["sha256"]
            .as_str()
            .context("native current source version absent")?;
        let e = content
            .owned
            .entries
            .iter()
            .find(|e| e.locator == locator)
            .context("native qualified file not independently owned")?;
        ensure!(
            row["bytes"] == e.bytes.len() && hash == e.sha256 && digest(&e.bytes) == hash,
            "current owned bytes differ from the original kernel-backed file: {locator}"
        );
        ensure!(
            native_hashes
                .insert(locator.to_owned(), hash.to_owned())
                .is_none(),
            "duplicate native current file"
        );
    }
    let metadata = decode(&content.house)?;
    // Strip native aliases before this independent metadata parser. They are
    // views of canonical resources, never a second set of definition files.
    let mut immutable_index = (*session.index).clone();
    immutable_index.resources.retain(|r| {
        !completed_baseline::mutable_locator(&r.locator)
            && !r.locator.starts_with("native-provider:")
    });
    completed_baseline::immutable_index(&immutable_index)?;
    let canonical = content.owned.mount(&immutable_index)?;
    let parsed = House::load(&canonical, &metadata.house.source)?;
    let expected_catalog = Catalog::from_program(&session.program);
    let current_modules = modules(&session.program);
    let current_required = required(&metadata.house, &content.catalog, &metadata.modules)?;
    let expected_immutable = immutable(&current_required);
    validate(
        &session.index,
        &metadata.house,
        &content.catalog,
        &metadata.modules,
        &metadata.immutable,
        &saved.modules,
    )?;
    validate_modules(&session.program, &metadata.modules)?;
    completed_baseline::resources(
        &session.index,
        &metadata.house.source,
        &metadata.house.neighborhood_source,
        &session.program,
    )?;
    let aliases = session
        .host
        .presentation
        .resource_aliases
        .as_ref()
        .context("retained native provider aliases absent")?;
    for (name, target) in aliases {
        let alias = &session.index.resources[session.index.find(name)?];
        let primary = &session.index.resources[session.index.find(target)?];
        ensure!(
            alias.name == *name
                && alias.locator == *name
                && primary.locator.eq_ignore_ascii_case(target)
                && alias.size == primary.size
                && alias.offset == primary.offset
                && alias.archived == primary.archived
                && alias.owned.is_some() == primary.owned.is_some()
                && digest(&alias.read()?) == digest(&primary.read()?),
            "current native provider alias diverges from its versioned canonical resource: {name}"
        );
        ensure!(
            !completed_baseline::mutable_locator(target) || alias.owned.is_some(),
            "native private provider can fall back to an installed file"
        );
    }
    let mut rows = vec![];
    same(
        "owned/full_native_kernel_file_versions",
        &native_hashes,
        &content.owned.hashes(),
        &mut rows,
    )?;
    same(
        "metadata/full_independent_current_owned_House_parse",
        &parsed,
        &metadata.house,
        &mut rows,
    )?;
    same(
        "metadata/full_in_memory_House_owner",
        &metadata.house,
        &session.house,
        &mut rows,
    )?;
    same(
        "source/House_identity",
        &metadata.house.source,
        &saved.source,
        &mut rows,
    )?;
    same(
        "source/all_historical_metadata_hash_receipts",
        &metadata.house.sources,
        &saved.source_hashes,
        &mut rows,
    )?;
    same(
        "program/all_retained_module_content_versions",
        &current_modules,
        &metadata.modules,
        &mut rows,
    )?;
    same(
        "program/complete_managed_catalog",
        &expected_catalog,
        &content.catalog,
        &mut rows,
    )?;
    same(
        "program/all_required_immutable_current_versions",
        &expected_immutable,
        &metadata.immutable,
        &mut rows,
    )?;
    same(
        "program/fingerprint",
        &session.program.fingerprint(),
        &saved.simulation.program_hash,
        &mut rows,
    )?;
    let modules: BTreeSet<_> = session.program.modules.keys().cloned().collect();
    same(
        "program/exact_declared_module_set",
        &modules,
        &saved.modules,
        &mut rows,
    )?;
    let normalized: BTreeSet<_> = session
        .program
        .modules
        .iter()
        .filter(|(_, m)| m.normalized_object_definitions)
        .map(|(s, _)| s.clone())
        .collect();
    same(
        "program/normalized_resident_definition_provenance",
        &normalized,
        &saved.normalized_object_definitions,
        &mut rows,
    )?;
    let animations: BTreeSet<_> = session.program.animations.keys().cloned().collect();
    same(
        "program/exact_retained_animation_set",
        &animations,
        &saved.animations,
        &mut rows,
    )?;
    let headline = session
        .program
        .modules
        .get(&session.program.globals)
        .is_some_and(|m| m.headline_catalog.is_some());
    same(
        "program/headline_catalog_presence",
        &headline,
        &saved.headline_catalog,
        &mut rows,
    )?;
    same(
        "namespace/asset_House_alias",
        &metadata.house.neighborhood_source,
        &saved.assets.neighborhood_source,
        &mut rows,
    )?;
    ensure!(
        postcard::to_allocvec(&session.simulation.vm)? == before_vm,
        "source metadata audit changed VM/RNG"
    );
    let differences = rows.iter().filter(|r| r["matched"] == false).count();
    Ok(
        json!({"schema":1,"tick":session.simulation.vm.tick,"comparisons":rows,"difference_count":differences,"comparison_count":rows.len(),
        "native_source_files":native.len(),"versioned_native_provider_aliases":aliases.len(),"source_qualification_sha256":sims_fixtures::digest(&serde_json::to_vec(qualification)?),
        "complete_native_Session_admitted":false,"native_parity_ticks":0,"starting_save_files_opened":false,"live_world_reconstructed_from_metadata":false,
        "scope":"All five original kernel-qualified current owned files; complete historical House metadata from a fresh owned-only parser, actual in-memory House and complete source/module/catalog/animation/version owners. Supplied Program is independently rebuilt before this audit; native active World is never reconstructed from House metadata."}),
    )
}
