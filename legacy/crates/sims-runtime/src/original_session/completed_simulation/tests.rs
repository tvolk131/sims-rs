use super::*;
use std::{fs, path::Path};
fn fixture() -> Result<(Value, Snapshot)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input = serde_json::from_slice(&fs::read(
        root.join("fixtures/private/goth-simulation-owners-20261028-first/owners-third.json"),
    )?)?;
    let model =
        root.join("fixtures/private/goth-world-composition-20261028-first/executed-seventh");
    let refs: Value = serde_json::from_slice(&fs::read(model.join("model-refs.json"))?)?;
    let reference = serde_json::from_value(refs["vm"].clone())?;
    let snapshot = sims_fixtures::Store::open(model.join("model-store"))?.get_value(&reference)?;
    Ok((input, snapshot))
}
#[test]
#[ignore = "actual private completed original Simulation owners"]
fn actual_nondefault_clock_diagnostics_floor_vector_and_score_owners_are_retained() -> Result<()> {
    let (input, snapshot) = fixture()?;
    let before = serde_json::to_vec(&snapshot)?;
    let imported = import(&input, &snapshot)?;
    ensure!(
        imported.clock.ticks == 88448
            && imported.fp_control == 0x003f
            && imported.diagnostics.error == 17
            && imported.diagnostics.blocker.is_none(),
        "current source controls lost"
    );
    ensure!(
        imported.character.floor_vector == [0.5, 9.300511, 0.5]
            && imported.statistics.pairs[5] == [5935, 1829],
        "current source view/history lost"
    );
    ensure!(
        imported.rates.len() == 3
            && imported.adult_mood.len() == 7
            && imported.child_mood.len() == 7
            && imported.person_scores.len() == 3
            && imported.person_scores.values().all(|c| c.len() == 9),
        "current source motive owners lost"
    );
    ensure!(
        !imported.complete_native_simulation_admitted && serde_json::to_vec(&snapshot)? == before,
        "VM state/RNG or admission changed"
    );
    Ok(())
}
#[test]
#[ignore = "actual private completed original Simulation owners"]
fn unknown_or_contradictory_owners_are_not_defaulted() -> Result<()> {
    let (input, snapshot) = fixture()?;
    let mut changed = input.clone();
    changed["floor_vector_raw"] = "00".repeat(12).into();
    ensure!(
        import(&changed, &snapshot).is_err(),
        "floor vector default admitted"
    );
    let mut changed = input.clone();
    changed["diagnostics_raw"] = "00".repeat(8).into();
    ensure!(
        import(&changed, &snapshot).is_err(),
        "diagnostic reset admitted"
    );
    let mut changed = input.clone();
    changed["hardware_registers"] = input["hardware_registers"]
        .as_str()
        .unwrap()
        .replace("FCW=003f", "FCW=037f")
        .into();
    ensure!(
        import(&changed, &snapshot).is_err(),
        "wrong native FPU precision admitted"
    );
    let mut changed = input.clone();
    changed["people"].as_array_mut().unwrap().pop();
    ensure!(
        import(&changed, &snapshot).is_err(),
        "missing personal score owner admitted"
    );
    let mut changed = snapshot.clone();
    changed.state.world.globals.insert(0, 0);
    ensure!(
        import(&input, &changed).is_err(),
        "different World clock admitted"
    );
    Ok(())
}

#[test]
#[ignore = "actual private original completed source boundary and score owners"]
fn score_owner_join_changes_only_observed_cache_and_household_does_not_replay_history() -> Result<()>
{
    let (input, snapshot) = fixture()?;
    let imported = import(&input, &snapshot)?;
    let linked = imported.with_score_owners(snapshot.clone())?;
    let mut stripped = linked.clone();
    for (&id, o) in &mut stripped.state.world.objects {
        o.native_motive_scores = snapshot.state.world.objects[&id]
            .native_motive_scores
            .clone();
    }
    ensure!(
        stripped == snapshot,
        "score join changed VM queues, frames, clocks or RNG"
    );
    for (&id, curves) in &imported.person_scores {
        let current = sims_vm::motives::score_owner::current(
            &linked.state.world,
            &sims_vm::Program::default(),
            id,
        )
        .map_err(anyhow::Error::msg)?;
        ensure!(
            current == *curves,
            "query did not use the original per-person score cache"
        );
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let proof = serde_json::from_slice(&fs::read(
        root.join("fixtures/private/goth-character-assembly-20261027-second/skin-proof-first.json"),
    )?)?;
    let household = imported.household(&linked, &proof)?;
    ensure!(
        household.state.clock == imported.clock
            && household.state.house_statistics == Some(imported.statistics.clone())
            && household.state.valuation == Some(imported.valuation.clone())
            && household.state.people.len() == 3
            && household.state.clock_events.is_empty()
            && !household.native_subscriber_history_claimed,
        "household source/delivery ownership differs"
    );
    let mut changed = linked.clone();
    changed.state.threads.values_mut().next().unwrap().status = sims_vm::Status::Ready;
    ensure!(
        imported.household(&changed, &proof).is_err(),
        "active Rust work lost through wrapper reentry"
    );
    let mut changed = linked;
    changed
        .state
        .world
        .objects
        .get_mut(&16)
        .unwrap()
        .native_motive_scores
        .as_mut()
        .unwrap()
        .curves[0]
        .points[0][1] = 0;
    ensure!(
        imported.with_score_owners(changed).is_err(),
        "contradictory score owner was overwritten"
    );
    Ok(())
}

#[test]
#[ignore = "private actual retained Goth process-cache evidence"]
fn retained_terrain_scale_is_observed_not_reinitialized() -> Result<()> {
    let (_, snapshot) = fixture()?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input: Value=serde_json::from_slice(&fs::read(root.join("fixtures/private/goth-native-animation-rebase-20261028-first/simulation-owners-with-terrain-first.json"))?)?;
    let before = serde_json::to_vec(&snapshot)?;
    let imported = import(&input, &snapshot)?;
    ensure!(
        imported.character.terrain_scale.map(f32::to_bits) == Some(0x3e5105ec),
        "actual retained terrain scale lost"
    );
    for kind in 0..4 {
        let mut changed = input.clone();
        if kind == 0 {
            changed["terrain_scale_raw"] = "00000000ec05513e".into();
        } else {
            let r = changed["reads"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|r| r["address"] == if kind == 3 { 0x5868fa } else { 0x703f28 })
                .unwrap();
            let mut raw = super::super::completed_animations::bytes(r["raw"].as_str().unwrap())?;
            if kind == 1 {
                raw[0] = 0;
            } else if kind == 2 {
                raw[4..8].copy_from_slice(&f32::INFINITY.to_bits().to_le_bytes());
            } else {
                raw[0] ^= 1;
            }
            let hex = raw.iter().map(|b| format!("{b:02x}")).collect::<String>();
            r["raw"] = hex.clone().into();
            r["sha256"] = sims_fixtures::digest(&raw).into();
            if kind != 3 {
                changed["terrain_scale_raw"] = hex.into();
            }
        }
        ensure!(
            import(&changed, &snapshot).is_err(),
            "terrain/cache code mutation {kind} admitted"
        );
    }
    ensure!(
        serde_json::to_vec(&snapshot)? == before,
        "cache import mutated VM/RNG"
    );
    Ok(())
}

#[test]
#[ignore = "private actual retained Goth character/model transaction fixture"]
fn sampled_vertex_stamps_survive_failure_and_cold_save_without_double_delivery() -> Result<()> {
    std::thread::Builder::new()
        .name("retained-stamp-delivery".into())
        .stack_size(32 << 20)
        .spawn(sampled_vertex_delivery_on_owned_state)?
        .join()
        .expect("stamp delivery test panicked")
}
fn sampled_vertex_delivery_on_owned_state() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/private/goth-native-animation-rebase-20261028-first/session-terrain-first");
    let refs: Value = serde_json::from_slice(&fs::read(root.join("refs.json"))?)?;
    let saved: super::super::Saved = sims_fixtures::Store::open(root.join("store"))?
        .get_value(&serde_json::from_value(refs["session"].clone())?)?;
    let mut resident = saved.simulation.state.characters[&18].clone();
    let mut model = saved.assets.characters[&18].model.clone();
    // Stage the root's four source-observed channel writes at the host handoff.
    let mut wire = serde_json::to_value(&resident.character.skin.animator)?;
    wire["pending_vertex_updates"] = json!({"0":4});
    resident.character.skin.animator = serde_json::from_value(wire)?;
    let original = model.bone_generations[0];
    let temporary = std::env::temp_dir().join(format!(
        "sims-stamp-delivery-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let store = sims_fixtures::Store::create(&temporary)?;
    let before = store.put_value(&(model.clone(), resident.clone()))?;
    let failure: Result<()> =
        model.with_bound(&mut resident, |_| anyhow::bail!("authored host failure"));
    ensure!(
        failure.is_err() && store.put_value(&(model.clone(), resident.clone()))? == before,
        "failed delivery changed owners"
    );
    let (mut cold_model, mut cold_resident): (
        sims_content::character_model::State,
        sims_sim::character::Resident,
    ) = store.get_value(&before)?;
    ensure!(
        store.put_value(&(cold_model.clone(), cold_resident.clone()))? == before,
        "cold serialization lost staged delivery"
    );
    for (m, r) in [
        (&mut model, &mut resident),
        (&mut cold_model, &mut cold_resident),
    ] {
        m.with_bound(r, |b| {
            ensure!(
                b.skeleton.geometry.bone_generations[0] == original.wrapping_add(4),
                "vertex stamps not delivered"
            );
            Ok(())
        })?;
        m.with_bound(r, |b| {
            ensure!(
                b.skeleton.geometry.bone_generations[0] == original.wrapping_add(4),
                "vertex stamps delivered twice"
            );
            Ok(())
        })?;
    }
    ensure!(
        store.put_value(&(model, resident))? == store.put_value(&(cold_model, cold_resident))?,
        "cold and uninterrupted delivery diverged"
    );
    fs::remove_dir_all(temporary)?;
    Ok(())
}
