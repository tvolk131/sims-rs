use super::*;
use std::{fs, path::Path};
fn fixture() -> Result<Value> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    Ok(serde_json::from_slice(&fs::read(root.join(
        "fixtures/private/goth-lot-owners-20261028-first/lot-fifth.json",
    ))?)?)
}
#[test]
#[ignore = "actual private original completed lot"]
fn actual_complete_geometry_and_physical_world_remain_unchanged() -> Result<()> {
    let source = fixture()?;
    let original = serde_json::to_vec(&source)?;
    let imported = import(&source)?;
    ensure!(
        !imported.complete_native_lot_admitted
            && imported.lot.size == 64
            && imported.lot.active_size == Some(52),
        "geometry extent/admission changed"
    );
    ensure!(
        serde_json::to_vec(&source)? == original,
        "geometry source changed"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/private/goth-world-composition-20261028-first/executed-seventh");
    let refs: Value = serde_json::from_slice(&fs::read(root.join("model-refs.json"))?)?;
    let reference = serde_json::from_value(refs["vm"].clone())?;
    let snapshot: sims_vm::Snapshot =
        sims_fixtures::Store::open(root.join("model-store"))?.get_value(&reference)?;
    let before = serde_json::to_vec(&snapshot)?;
    let physical = physical::import(&imported, &snapshot.state.world)?;
    ensure!(
        physical.placement.links.next.len() == 344
            && physical.selected_person == Some(21)
            && physical.indicators.primary == Some(383)
            && physical.indicators.secondary == Some(384)
            && physical.membership.0.values().all(|r| *r == 0),
        "source physical owners differ"
    );
    ensure!(
        serde_json::to_vec(&snapshot)? == before,
        "physical import changed VM/RNG/scheduler state"
    );
    let mut changed = snapshot.state.world.clone();
    changed.objects.get_mut(&16).unwrap().native_flags = Some(0);
    ensure!(
        physical::import(&imported, &changed).is_err(),
        "physical source/World mismatch admitted"
    );
    Ok(())
}
#[test]
#[ignore = "actual private original completed lot"]
fn incomplete_or_changed_geometry_is_not_reconstructed() -> Result<()> {
    let source = fixture()?;
    let mut changed = source.clone();
    let text = changed["arrays"]["primary1"]["data_raw"].as_str().unwrap();
    changed["arrays"]["primary1"]["data_raw"] = format!(
        "{}{}",
        &text[..text.len() - 2],
        if text.ends_with("ff") { "00" } else { "ff" }
    )
    .into();
    ensure!(
        import(&changed).is_err(),
        "unobserved inactive tail admitted"
    );
    let mut changed = source.clone();
    changed["rooms"][0]["tiles_raw"] = "".into();
    ensure!(
        import(&changed).is_err(),
        "omitted ordered room tiles rebuilt"
    );
    let mut changed = source.clone();
    changed["half_nodes"].as_array_mut().unwrap().pop();
    ensure!(
        import(&changed).is_err(),
        "omitted native half room admitted"
    );
    let mut changed = source.clone();
    changed["provenance"]["image_sha256"] = "different epoch".into();
    ensure!(import(&changed).is_err(), "unqualified epoch admitted");
    Ok(())
}

#[test]
#[ignore = "actual private original completed room caches"]
fn actual_room_caches_are_retained_without_recount_or_lighting_reset() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let geometry = import(&fixture()?)?;
    let source: Value = serde_json::from_slice(&fs::read(
        root.join("fixtures/private/goth-lot-owners-20261028-first/environment-first.json"),
    )?)?;
    let model =
        root.join("fixtures/private/goth-world-composition-20261028-first/executed-seventh");
    let refs: Value = serde_json::from_slice(&fs::read(model.join("model-refs.json"))?)?;
    let reference = serde_json::from_value(refs["vm"].clone())?;
    let snapshot: sims_vm::Snapshot =
        sims_fixtures::Store::open(model.join("model-store"))?.get_value(&reference)?;
    let before = serde_json::to_vec(&snapshot)?;
    let imported = environment::import(&geometry, &source, &snapshot.state.world)?;
    ensure!(
        imported.manager.rooms.len() == 11
            && imported.manager.rooms[&0].totals.people == 4
            && imported.manager.light_maps[0]
                .cells
                .iter()
                .filter(|c| **c != [127; 4])
                .count()
                == 31,
        "actual room history lost"
    );
    ensure!(
        serde_json::to_vec(&snapshot)? == before,
        "room import changed VM/RNG/scheduler"
    );
    let mut changed = source.clone();
    changed["light_maps"][0]["data_raw"] = "7f".repeat(16384).into();
    ensure!(
        environment::import(&geometry, &changed, &snapshot.state.world).is_err(),
        "native local light map reset admitted"
    );
    let mut changed = source.clone();
    changed["rooms"].as_array_mut().unwrap().pop();
    ensure!(
        environment::import(&geometry, &changed, &snapshot.state.world).is_err(),
        "missing source room admitted"
    );
    let mut changed = snapshot.state.world.clone();
    changed.globals.remove(&27);
    ensure!(
        environment::import(&geometry, &source, &changed).is_err(),
        "missing native area publication guessed"
    );
    Ok(())
}

#[test]
#[ignore = "actual private original callback tables and Program join"]
fn actual_full_callback_tables_bind_architecture_without_unknown_portal_fallback() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let geometry = import(&fixture()?)?;
    let input = serde_json::from_slice(&fs::read(root.join(
        "fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json",
    ))?)?;
    let parts = crate::conformance::completed_reentry::import_parts(input)?;
    let systems: Value = serde_json::from_slice(&fs::read(
        root.join("fixtures/private/goth-world-composition-20261028-first/systems-seventh.json"),
    )?)?;
    let (mut program, _) =
        super::super::completed_world::program::join(parts.program, &systems["body_profiles"])?;
    let model =
        root.join("fixtures/private/goth-world-composition-20261028-first/executed-seventh");
    let refs: Value = serde_json::from_slice(&fs::read(model.join("model-refs.json"))?)?;
    let reference = serde_json::from_value(refs["vm"].clone())?;
    let snapshot: sims_vm::Snapshot =
        sims_fixtures::Store::open(model.join("model-store"))?.get_value(&reference)?;
    ensure!(
        program.fingerprint() == snapshot.program_hash,
        "observed Program join changed fingerprint"
    );
    let before = serde_json::to_vec(&snapshot)?;
    let architecture = architecture(&geometry, &program, &snapshot.state.world)?;
    ensure!(
        architecture.original.as_ref() == Some(&geometry.lot)
            && architecture.footprints.values().any(|f| f.portal),
        "original architecture/portals lost"
    );
    ensure!(
        serde_json::to_vec(&snapshot)? == before,
        "architecture binding changed VM"
    );
    let definition = snapshot.state.world.objects[&16]
        .definition
        .as_ref()
        .unwrap();
    program
        .modules
        .get_mut(&definition.source)
        .unwrap()
        .object_functions
        .remove(&definition.object);
    ensure!(
        super::architecture(&geometry, &program, &snapshot.state.world).is_err(),
        "unknown native callback directory became no portal"
    );
    Ok(())
}
