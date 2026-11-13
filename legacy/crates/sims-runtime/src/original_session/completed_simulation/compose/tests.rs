use super::*;
use std::{fs, path::Path};
struct Fixture {
    vm: Snapshot,
    program: Program,
    owners: Imported,
    lot: completed_lot::Imported,
    environment: Value,
    residents: Value,
    boundary: completed_skin::Input,
}
fn fixture() -> Result<Fixture> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let config: Value = serde_json::from_slice(&fs::read(
        root.join("fixtures/private/goth-simulation-composition-20261028-first/input.json"),
    )?)?;
    let read = |key: &str| -> Result<Value> {
        Ok(serde_json::from_slice(&fs::read(
            root.join(config[key].as_str().unwrap()),
        )?)?)
    };
    let refs = read("vm_refs")?;
    let vm: Snapshot = sims_fixtures::Store::open(root.join(config["vm_store"].as_str().unwrap()))?
        .get_value(&serde_json::from_value(refs["vm"].clone())?)?;
    let owners = super::super::import(&read("owners")?, &vm)?;
    let vm = owners.with_score_owners(vm)?;
    let parts = crate::conformance::completed_reentry::import_parts(read("reentry")?)?;
    let (program, _) = crate::original_session::completed_world::program::join(
        parts.program,
        &read("systems")?["body_profiles"],
    )?;
    let residents = read("residents")?;
    let (program, vm, _) = with_current_clips(program, vm, &residents)?;
    Ok(Fixture {
        vm,
        program,
        owners,
        lot: completed_lot::import(&read("lot")?)?,
        environment: read("environment")?,
        residents,
        boundary: serde_json::from_value(read("boundary")?)?,
    })
}
fn compose(f: &Fixture) -> Result<Composed> {
    import(Input {
        vm: f.vm.clone(),
        program: &f.program,
        owners: &f.owners,
        lot: &f.lot,
        environment: &f.environment,
        resident_source: &f.residents,
        boundary: &f.boundary,
    })
}
#[test]
#[ignore = "actual private completed original Simulation composition"]
fn actual_full_simulation_restores_with_live_caches_and_no_fcns_or_slot_resources() -> Result<()> {
    let f = fixture()?;
    ensure!(
        f.program.modules.values().all(|m| m.slots.is_empty()),
        "test unexpectedly has SLOT resources"
    );
    let c = compose(&f)?;
    let s = &c.snapshot.state;
    ensure!(
        s.vm.tick == 88448 && s.vm.world.objects.len() == 344 && s.characters.len() == 3,
        "current Simulation coverage differs"
    );
    ensure!(
        s.vm.world.objects.values().all(|o| o
            .native_slot_geometry
            .as_ref()
            .is_some_and(|g| g.len() == o.slots.len())),
        "missing current slot owner"
    );
    ensure!(
        s.bodies.len() == 236 && s.unplaced.len() == 108 && !c.complete_native_simulation_admitted,
        "projection or admission differs"
    );
    let slots = sims_sim::lot_acceptance::contained::slots(&s.vm.world, &Program::default(), 200)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        slots.len() == 5 && slots[0].offset == [0., 0., 4.] && slots[1].offset == [0., -4., 4.],
        "retained geometry was replaced by a resource profile"
    );
    Ok(())
}
#[test]
#[ignore = "actual private completed original Simulation composition"]
fn full_validator_requires_current_clips_and_valid_cache_owners() -> Result<()> {
    let f = fixture()?;
    let c = compose(&f)?;
    let mut missing = f.program.clone();
    missing.animations.clear();
    let mut s = c.snapshot.clone();
    s.program_hash = missing.fingerprint();
    ensure!(
        s.restore(&missing)
            .unwrap_err()
            .contains("saved native character clip absent"),
        "clip absence hidden"
    );
    let mut s = c.snapshot.clone();
    s.state.household.as_mut().unwrap().tuning.rates[0].bladder_asleep = f32::NAN;
    ensure!(
        s.restore(&f.program)
            .unwrap_err()
            .contains("nonfinite retained motive coefficient"),
        "invalid cache accepted"
    );
    let mut s = c.snapshot;
    s.state
        .vm
        .world
        .objects
        .get_mut(&200)
        .unwrap()
        .native_slot_geometry
        .as_mut()
        .unwrap()
        .pop();
    ensure!(
        s.restore(&f.program)
            .unwrap_err()
            .contains("retained containment geometry/allocation mismatch"),
        "invalid slot owner accepted"
    );
    Ok(())
}
#[test]
#[ignore = "actual private completed original Simulation composition"]
fn contradictory_person_epoch_and_existing_slot_cache_are_rejected() -> Result<()> {
    let mut f = fixture()?;
    let text = f.residents["resident_input"]["actors"][0]["person_raw"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut raw = crate::original_session::completed_animations::bytes(&text)?;
    raw[0x40] ^= 1;
    f.residents["resident_input"]["actors"][0]["person_raw"] = raw
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .into();
    ensure!(
        compose(&f)
            .unwrap_err()
            .to_string()
            .contains("resident physical/animator epoch differs"),
        "mixed person source accepted"
    );
    let mut f = fixture()?;
    f.vm.state
        .world
        .objects
        .get_mut(&200)
        .unwrap()
        .native_slot_geometry = Some(vec![]);
    ensure!(
        compose(&f)
            .unwrap_err()
            .to_string()
            .contains("contradictory retained slot geometry"),
        "conflicting current geometry replaced"
    );
    Ok(())
}
