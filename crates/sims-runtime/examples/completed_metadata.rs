//! Parse historical metadata from the CURRENT owned resource cohort, never
//! from an installed starting save. This does not create a VM/Simulation,
//! replay load callbacks, or use saved metadata as live state.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::{Index, owned_resources::Neighborhood, saves::House};
use sims_fixtures::Store;
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 3,
        "usage: completed_metadata OWNED_DIR IMMUTABLE_ROOT NEW_DIR"
    );
    let owned_dir = Path::new(&a[0]);
    let refs: Value = serde_json::from_slice(&fs::read(owned_dir.join("refs.json"))?)?;
    let owned: Neighborhood = Store::open(owned_dir.join("store"))?
        .get_value(&serde_json::from_value(refs["neighborhood"].clone())?)?;
    owned.validate()?;
    let mut immutable = Index::scan(Path::new(&a[1]))?;
    immutable.resources.retain(|r| {
        !r.locator
            .split('/')
            .any(|c| c.to_ascii_lowercase().starts_with("userdata"))
    });
    let index = owned.mount(&immutable)?;
    let source = format!("{}/Houses/House05.iff", owned.directory);
    // All mutable resources in the mounted namespace are owned bytes. The
    // normal data parser may read hash-recorded immutable definition files.
    // It cannot reach a different installed mutable namespace.
    for r in &index.resources {
        if r.locator.to_ascii_lowercase().starts_with("userdata") {
            ensure!(r.owned.is_some(), "metadata mutable source is not owned");
        }
    }
    let house = House::load(&index, &source)?;
    ensure!(
        house.source == source
            && house.neighborhood_source == format!("{}/Neighborhood.iff", owned.directory),
        "current metadata identity differs"
    );
    for (locator, hash) in owned.hashes() {
        if let Some(used) = house.sources.get(&locator) {
            ensure!(used == &hash, "metadata/current owned file differs");
        }
    }
    let out = Path::new(&a[2]);
    fs::create_dir(out)?;
    let store = Store::create(out.join("store"))?;
    let blob = store.put_value(&house)?;
    let cold: House = store.get_value(&blob)?;
    ensure!(
        postcard::to_allocvec(&house)? == postcard::to_allocvec(&cold)?,
        "historical metadata cold restore differs"
    );
    fs::write(
        out.join("refs.json"),
        serde_json::to_vec_pretty(&json!({"house_metadata":blob}))?,
    )?;
    fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"schema":1,"source":house.source,"neighborhood":house.neighborhood_source,"number":house.number,"sources":house.sources,"owned_sources":owned.hashes(),"historical_metadata_only":true,"live_simulation_reconstructed":false,"vm_constructed_or_executed":false,"installed_starting_save_files_opened":false,"native_parity_ticks":0,"complete_native_Session_admitted":false}),
        )?,
    )?;
    Ok(())
}
