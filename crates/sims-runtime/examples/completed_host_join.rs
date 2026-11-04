//! Restore the retained asset host with owned current files and authenticated
//! property/camera services. This is not complete native Session admission.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_content::{Index, character_loading::live, owned_resources::Neighborhood};
use sims_fixtures::Store;
use sims_runtime::original_session::{
    completed_assets::Joined,
    completed_presentation::{self, Bindings},
};
use std::{collections::BTreeMap, fs, path::Path};
#[derive(Debug, Serialize, Deserialize)]
struct Saved {
    simulation: sims_sim::Snapshot,
    assets: live::Snapshot,
    presentation: Bindings,
    owned: Neighborhood,
}
fn value(root: &Path, c: &Value, k: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(
        root.join(c[k].as_str().context("host input absent")?),
    )?)?)
}
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 4,
        "usage: completed_host_join SOURCE_ROOT CONFIG IMMUTABLE_ROOT NEW_DIR"
    );
    let root = Path::new(&a[0]);
    let config: Value = serde_json::from_slice(&fs::read(&a[1])?)?;
    let joined: Joined = Store::open(root.join(config["assets_store"].as_str().unwrap()))?
        .get_value(&serde_json::from_value(
            value(root, &config, "assets_refs")?["assets"].clone(),
        )?)?;
    let owned: Neighborhood = Store::open(root.join(config["owned_store"].as_str().unwrap()))?
        .get_value(&serde_json::from_value(
            value(root, &config, "owned_refs")?["neighborhood"].clone(),
        )?)?;
    owned.validate()?;
    let presentation_source = value(root, &config, "presentation")?;
    let residents = value(root, &config, "residents")?;
    let presentation = completed_presentation::import(&presentation_source, &residents)?;
    let mut immutable = Index::scan(Path::new(&a[2]))?;
    immutable.resources.retain(|r| {
        !r.locator
            .split('/')
            .any(|p| p.to_ascii_lowercase().starts_with("userdata"))
    });
    let mut index = owned.mount(&immutable)?;
    let bindings = value(root, &config, "bindings")?;
    let qualified = value(root, &config, "qualification")?;
    for b in bindings["candidate_module_bindings"]
        .as_array()
        .context("provider bindings absent")?
    {
        let locator = b["ordered_component_locators"][0].as_str().unwrap();
        if locator.to_ascii_lowercase().starts_with("userdata/") {
            let q: Vec<_> = qualified["qualified_private_module_bindings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|q| q["native_source"] == b["native_source"])
                .collect();
            ensure!(
                q.len() == 1
                    && q[0]["complete_native_source_version_qualified"] == true
                    && q[0]["primary_content_sha256"] == b["primary_content_sha256"],
                "private provider source epoch unqualified"
            );
        }
        let mut alias = index.resources[index.find(locator)?].clone();
        ensure!(
            sims_fixtures::digest(&alias.read()?) == b["primary_content_sha256"],
            "provider whole-file hash differs"
        );
        alias.locator = b["native_source"].as_str().unwrap().into();
        index.resources.push(alias);
    }
    let snapshot = live::Snapshot {
        floors: joined.floors,
        neighborhood_source: format!("{}/Neighborhood.iff", owned.directory),
        initializer_settings: joined.initializer_settings,
        // New unpublished Rust delivery ownership, not a declaration that an
        // original process-global callback/event/window queue never existed.
        fresh: BTreeMap::new(),
        reloads: BTreeMap::new(),
        accessories: BTreeMap::new(),
        retirements: BTreeMap::new(),
        notification_jobs: BTreeMap::new(),
        notifications: None,
        characters: joined.characters,
        mesh_pool: joined.mesh_pool,
        clothing: joined.clothing,
        skeletons: Some(joined.skeletons),
        ordinary: joined.ordinary,
        autonomy_resources: Some(joined.autonomy_resources),
    };
    let before_assets = postcard::to_allocvec(&snapshot)?;
    let mut simulation = joined.simulation;
    let before_sim = postcard::to_allocvec(&simulation.state)?;
    let mut host = live::Host::restore(&index, presentation.clone(), snapshot)?;
    ensure!(
        postcard::to_allocvec(&host.snapshot())? == before_assets,
        "host restore changed supplied asset state"
    );
    host.update_contexts(&mut simulation.state)?;
    ensure!(
        postcard::to_allocvec(&simulation.state)? == before_sim,
        "source-bound context validation changed simulation"
    );
    let mut visibility = BTreeMap::new();
    for (&id, r) in &simulation.state.characters {
        let model = &host.characters[&id].model;
        let actual = presentation
            .visibility
            .visible(r, model, presentation.viewport)?;
        ensure!(
            actual == r.character.visible,
            "source-bound visibility differs from current native actor {id}: {actual} vs {}",
            r.character.visible
        );
        visibility.insert(id, actual);
    }
    let out = Path::new(&a[3]);
    fs::create_dir(out)?;
    let store = Store::create(out.join("store"))?;
    let saved = Saved {
        simulation,
        assets: host.snapshot(),
        presentation,
        owned,
    };
    let blob = store.put_value(&saved)?;
    let cold: Saved = store.get_value(&blob)?;
    ensure!(
        postcard::to_allocvec(&saved)? == postcard::to_allocvec(&cold)?,
        "cold host owners differ"
    );
    let mut coldhost = live::Host::restore(&index, cold.presentation, cold.assets)?;
    let mut coldsim = cold.simulation;
    coldhost.update_contexts(&mut coldsim.state)?;
    ensure!(
        postcard::to_allocvec(&coldsim.state)? == before_sim
            && postcard::to_allocvec(&coldhost.snapshot())? == before_assets,
        "cold host binding changed retained owners"
    );
    fs::write(
        out.join("refs.json"),
        serde_json::to_vec_pretty(&json!({"host":blob}))?,
    )?;
    fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"schema":1,"tick":88448,"native_parity_ticks":0,"executed_ticks":0,"asset_host_restore_validated":true,"source_bound_contexts_unchanged":true,"current_native_visibility":visibility,"cold_restore_identical":true,
        "complete_native_host_admitted":false,"complete_native_Session_admitted":false,"original_starting_save_files_opened":false,"presentation":saved.presentation,
        "scope":"current asset host, current owned files, property/visibility services; other frontend operations reject unbound services",
        "new_delivery_epoch":"empty Rust fresh/reload/accessory/retirement/notification jobs; software text presenter unbound",
        "remaining":["complete logical Session frontend and metadata owners","original ordered execution/yield trace","strict original tick and cold-save comparison"]}),
        )?,
    )?;
    Ok(())
}
