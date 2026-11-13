use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::Session;
use sims_sim::{
    Snapshot, Step, StepStop,
    portrait_person::{Progress, Service},
};
#[path = "support/portrait.rs"]
mod support;
use support::{capture, catalog, supply};

#[test]
#[ignore = "requires original assets and post-death Mashuga diagnostic capture"]
fn original_temporary_person_places_or_destroys_and_restores_at_service_boundaries() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut catalog = catalog(&index)?;
    for (label, projected, success) in [
        ("placed", [344, 232], true),
        ("no-location", [520, 408], false),
    ] {
        let mut session = capture(&index, &root)?;
        ensure!(session.simulation.vm.tick == 215742);
        ensure!(
            session
                .simulation
                .vm
                .world
                .objects
                .values()
                .all(|o| o.definition.as_ref().is_none_or(|d| d.guid != 1480716884))
        );
        let original_visit = session.simulation.visit;
        let original_visited = session.simulation.visited.clone();
        catalog
            .populate_portraits(
                &mut session.simulation.vm.world.image_cache,
                true,
                false,
                &mut |_, _| panic!("capture unexpectedly has retained portrait images"),
            )
            .map_err(anyhow::Error::msg)?;
        let slot = session
            .simulation
            .vm
            .world
            .image_cache
            .entries
            .iter()
            .position(|e| e.guid == 1480716884)
            .context("portrait absent")? as u32;
        let before_empty = session.simulation.clone();
        session
            .simulation
            .begin_temporary_portrait_person(u32::MAX)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            session
                .simulation
                .take_temporary_portrait_person()
                .map_err(anyhow::Error::msg)?
                .is_none()
        );
        session
            .simulation
            .begin_temporary_portrait_person(slot)
            .map_err(anyhow::Error::msg)?;
        session
            .simulation
            .supply_portrait_person_resource(&session.program, None)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            session
                .simulation
                .take_temporary_portrait_person()
                .map_err(anyhow::Error::msg)?
                .is_none()
        );
        ensure!(
            session.simulation == before_empty,
            "missing portrait/resource changed the lot"
        );
        session
            .simulation
            .begin_temporary_portrait_person(slot)
            .map_err(anyhow::Error::msg)?;
        ensure!(session.simulation.take_temporary_portrait_person().is_err());
        let mut control = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
        )?;
        let before = session.simulation.clone();
        ensure!(
            session
                .simulation
                .run_current_tick(&session.program, 100)
                .unwrap_err()
                .contains("synchronous owner")
        );
        ensure!(
            session
                .simulation
                .finish_registry_pass()
                .unwrap_err()
                .contains("native registry pass has not returned")
        );
        ensure!(session.simulation == before);
        let mut boundaries = std::collections::BTreeSet::new();
        let mut restores = 0;
        let mut services = 0;
        let mut instructions = 0;
        let mut result = None;
        let mut camera_calls = 0;
        for slice in 0..20000 {
            session.host.update_contexts(&mut session.simulation)?;
            let mut remaining = 1;
            let mut step = Step {
                stop: StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            let progress = session
                .simulation
                .drive_temporary_portrait_person(&session.program, &mut remaining, &mut step)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("{label} slice {slice}"))?;
            instructions += step.trace.len();
            let owner = session.simulation.portrait_person.as_ref().unwrap();
            let boundary = format!(
                "{:?}/factory={:?}/deleting={}/physical={}",
                owner.request(),
                owner
                    .created()
                    .and_then(|id| session.simulation.person_factories.get(&id))
                    .and_then(|f| f.factory.pending_request()),
                !session.simulation.deletion.is_empty(),
                session
                    .simulation
                    .native_lot
                    .as_ref()
                    .unwrap()
                    .removal
                    .is_some()
            );
            let first = boundaries.insert(boundary);
            match progress {
                Progress::Service(service) => {
                    camera_calls += usize::from(service == Service::CameraOrigin);
                    supply(&mut session, &mut catalog, service, projected)?;
                }
                Progress::Complete(id) => result = Some(id),
                Progress::Yielded => {}
            }
            services += session
                .host
                .service(&mut session.simulation, &mut session.program)?;
            ensure!(
                session.simulation.vm.tick == 215742
                    && session.simulation.visit == original_visit
                    && session.simulation.visited == original_visited
            );
            if first || result.is_some() {
                let simulation: Snapshot = serde_json::from_slice(&serde_json::to_vec(
                    &Snapshot::capture(&session.program, &session.simulation),
                )?)?;
                let simulation = simulation
                    .restore(&session.program)
                    .map_err(anyhow::Error::msg)
                    .with_context(|| format!("{label} restore slice {slice}"))?;
                ensure!(simulation == session.simulation);
                let assets = session.host.snapshot();
                let host = sims_content::character_loading::live::Host::restore(
                    &index,
                    session.host.presentation.clone(),
                    serde_json::from_slice(&serde_json::to_vec(&assets)?)?,
                )?;
                ensure!(host.snapshot() == assets);
                session.simulation = simulation;
                session.host = host;
                restores += 1;
            }
            if result.is_some() {
                break;
            }
        }
        let returned = result.context("temporary caller did not complete")?;
        let mut control_result = None;
        for _ in 0..1000 {
            control.host.update_contexts(&mut control.simulation)?;
            let mut remaining = 100_000;
            let mut step = Step {
                stop: StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            let progress = control
                .simulation
                .drive_temporary_portrait_person(&control.program, &mut remaining, &mut step)
                .map_err(anyhow::Error::msg)?;
            match progress {
                Progress::Service(service) => {
                    supply(&mut control, &mut catalog, service, projected)?
                }
                Progress::Complete(id) => control_result = Some(id),
                Progress::Yielded => {}
            }
            control
                .host
                .service(&mut control.simulation, &mut control.program)?;
            if control_result.is_some() {
                break;
            }
        }
        ensure!(
            control_result == Some(returned)
                && control.simulation == session.simulation
                && control.host.snapshot() == session.host.snapshot(),
            "budget/restoration changed {label} outcome"
        );
        let id = session
            .simulation
            .portrait_person
            .as_ref()
            .unwrap()
            .created()
            .context("person not constructed")?;
        ensure!(camera_calls == 1 && returned == success.then_some(id));
        let position = session.simulation.vm.world.positions.get(&id).copied();
        if success {
            ensure!(
                session.simulation.characters.contains_key(&id)
                    && session.host.characters.contains_key(&id)
            );
            ensure!(!session.simulation.unplaced.contains(&id));
            let position = position.context("placed person has no position")?;
            ensure!(position.level == 1 && position.x >= 0 && position.y >= 0);
        } else {
            ensure!(
                !session.simulation.vm.world.objects.contains_key(&id)
                    && !session.simulation.characters.contains_key(&id)
                    && !session.host.characters.contains_key(&id)
                    && !session.simulation.retired_characters.contains_key(&id)
                    && !session.simulation.person_factories.contains_key(&id)
                    && session.simulation.deletion.is_empty()
            );
        }
        ensure!(
            session
                .simulation
                .take_temporary_portrait_person()
                .map_err(anyhow::Error::msg)?
                == returned
        );
        let restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
        )?;
        ensure!(
            restored.simulation == session.simulation
                && restored.host.snapshot() == session.host.snapshot()
        );
        let report = serde_json::json!({"label":label,"tick":215742,"object":id,"returned":returned,
            "projected":projected,"position":position,"services":services,"instructions":instructions,
            "restores":restores,"boundaries":boundaries,"camera_calls":camera_calls,"fresh_program_restore_identical":true,
            "large_budget_without_intermediate_restores_identical":true,
            "scope":"Native temporary-person caller with real catalog, construction, one-pass search, placement and failed-search destruction. Camera projection is explicit; portrait rendering and live dialog remain unconnected."});
        std::fs::write(
            root.join(format!("cache/original-temporary-person-{label}.json")),
            serde_json::to_vec_pretty(&report)?,
        )?;
        std::fs::write(
            root.join(format!(
                "cache/original-temporary-person-{label}-session.json"
            )),
            serde_json::to_vec(&session.save())?,
        )?;
        println!("{report}");
    }
    Ok(())
}
