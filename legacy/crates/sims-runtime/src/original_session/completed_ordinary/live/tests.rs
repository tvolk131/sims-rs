use super::*;
use std::{fs, path::Path};

fn read(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn fixture() -> Result<(
    Imported,
    BTreeMap<ObjectId, Definition>,
    Value,
    super::super::super::completed_skin::Input,
    Index,
)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = read(
        root.join("fixtures/private/goth-ordinary-host-20261028-second/ordinary-input-second.json"),
    )?;
    let profiles =
        read(root.join("fixtures/private/goth-live-ordinary-20261028-first/profiles-first.json"))?;
    let proof = serde_json::from_value(read(
        root.join("fixtures/private/goth-character-assembly-20261027-second/skin-proof-first.json"),
    )?)?;
    let bindings = read(root.join(
        "fixtures/private/goth-stream-owners-20261028-first/resident-BHAV-comparison-fourth.json",
    ))?;
    let mut index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    index.resources.retain(|r| {
        !r.locator
            .split('/')
            .any(|c| c.to_ascii_lowercase().starts_with("userdata"))
    });
    let definitions = serde_json::from_value(source["definitions"].clone())?;
    let imported = super::super::import(
        &source["ordinary"],
        &definitions,
        &serde_json::from_value(source["lifetimes"].clone())?,
        &index,
        "ordinary-test-unpublished",
    )?;
    for p in rows(&profiles, "profiles")? {
        let name = format!("native-provider:{:08x}", word(p, "provider")?);
        let row = rows(&bindings, "candidate_module_bindings")?
            .iter()
            .find(|b| b["native_source"] == name)
            .context("candidate absent")?;
        let source = row["ordered_component_locators"][0]
            .as_str()
            .context("locator absent")?;
        let mut alias = index.resources[index.find(source)?].clone();
        alias.locator = name;
        index.resources.push(alias);
    }
    Ok((imported, definitions, profiles, proof, index))
}

#[test]
#[ignore = "actual private original ROOT graphs and asset files"]
fn actual_nonempty_arrow_and_empty_date_arrow_survive_live_storage_restore() -> Result<()> {
    let (original, definitions, profiles, proof, index) = fixture()?;
    let source = serde_json::to_vec(&original)?;
    let imported = import(original, &definitions, &profiles, &proof, &index)?;
    ensure!(
        serde_json::to_vec(&imported.ordinary)? == source,
        "source changed"
    );
    let live = graphic_loading::Registry::restore(&index, imported.registry.clone())?;
    ensure!(
        live.graphics.keys().copied().collect::<Vec<_>>() == [383, 384],
        "ordinary actor set differs"
    );
    ensure!(
        live.retained.is_empty(),
        "generic live owners were put through the empty-graph path"
    );
    let arrow = &live.graphics[&383].ready;
    let date = &live.graphics[&384].ready;
    ensure!(
        arrow
            .geometry
            .vertices
            .values()
            .map(|v| v.vertices.len())
            .sum::<usize>()
            == 65,
        "published native vertices lost"
    );
    ensure!(
        arrow.avatar.snapshot().records.entries[0].binding.is_some()
            && date.avatar.snapshot().records.entries[0].binding.is_none(),
        "actual nonempty/empty graph distinction lost"
    );
    ensure!(
        arrow.frame.now == 4422350
            && serde_json::to_value(&arrow.bound.skeleton.skin.animator)?["last_clock"] == 4422350,
        "native clocks changed"
    );
    ensure!(
        arrow.appearance.marker
            && arrow.appearance.kind == 1
            && !arrow.appearance.ordinary_material,
        "native arrow marker flags lost"
    );
    ensure!(
        live.snapshot() == imported.registry && !imported.complete_native_host_admitted,
        "storage/admission scope differs"
    );
    Ok(())
}

#[test]
#[ignore = "synthetic negative changes to private actual source"]
fn changed_cache_strings_code_epoch_and_missing_owner_are_rejected() -> Result<()> {
    let (original, definitions, profiles, proof, index) = fixture()?;
    let mut changed = profiles.clone();
    changed["profiles"][0]["strings"][1] = json!("invented");
    ensure!(
        import(original.clone(), &definitions, &changed, &proof, &index).is_err(),
        "invented cached strings accepted"
    );
    let mut changed = profiles.clone();
    changed["code"][0]["raw"] = json!("00");
    ensure!(
        import(original.clone(), &definitions, &changed, &proof, &index).is_err(),
        "changed loader accepted"
    );
    let mut changed = profiles.clone();
    changed["provenance"]["image_sha256"] = json!("other");
    ensure!(
        import(original.clone(), &definitions, &changed, &proof, &index).is_err(),
        "foreign epoch accepted"
    );
    let mut changed = profiles.clone();
    changed["profiles"].as_array_mut().unwrap().pop();
    ensure!(
        import(original, &definitions, &changed, &proof, &index).is_err(),
        "missing date cache accepted"
    );
    Ok(())
}
