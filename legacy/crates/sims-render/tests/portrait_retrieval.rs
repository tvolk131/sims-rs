use anyhow::{Context, Result, ensure};
use sims_content::{Index, object_resources::Overlay};
use sims_render::portrait::{
    cache::{self, Images, Settings},
    images,
    resize::Format,
    retrieval::{Progress, Retrieval},
};
use sims_runtime::original_session::Session;

#[path = "../../sims-content/tests/support/portrait.rs"]
mod support;

#[test]
#[ignore = "requires original post-death Mashuga capture and assets; CPU rendering"]
fn original_portrait_retrieval_loads_regenerates_persists_and_restores() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let original = Overlay::copy(&index, [index.find("UserData2/Characters/User00000.iff")?])
        .map_err(anyhow::Error::msg)?;
    let mut templates = std::collections::BTreeMap::new();
    for resource in [108, 4600] {
        templates.insert(
            resource,
            images::template(&index, resource, Format::Rgb565)?.unwrap(),
        );
    }
    let settings = Settings {
        layout: [45, 45, 5, 2, 35, 41],
        format: Format::Rgb565,
        key: 0xf81f,
        templates,
    };
    let mut evidence = vec![];
    for mode in ["saved", "dirty", "missing"] {
        let seed = if mode == "missing" {
            let mut catalog = support::catalog_with_overlay(&index, original.clone())?;
            let resource = catalog
                .portrait_resource(1480716884)
                .map_err(anyhow::Error::msg)?
                .unwrap();
            let file = catalog
                .portrait_file(resource)
                .map_err(anyhow::Error::msg)?;
            let handle = catalog
                .resources
                .by_id(file, u32::from_be_bytes(*b"BMP_"), 2002, 0)
                .map_err(anyhow::Error::msg)?;
            ensure!(handle != 0);
            catalog
                .resources
                .delete(file, handle)
                .map_err(anyhow::Error::msg)?;
            ensure!(catalog.resources.error(file).map_err(anyhow::Error::msg)? == 0);
            catalog.resources.flush(file).map_err(anyhow::Error::msg)?;
            catalog.resources.overlay().map_err(anyhow::Error::msg)?
        } else {
            original.clone()
        };
        let mut results = vec![];
        for restoring in [false, true] {
            let mut session = support::capture(&index, &root)?;
            let mut catalog = support::catalog_with_overlay(&index, seed.clone())?;
            let mut pixels = Images::default();
            let state = &mut session.simulation.vm.world.image_cache;
            catalog
                .populate_portraits(state, true, false, &mut |e, s| pixels.event(e, s))
                .map_err(anyhow::Error::msg)?;
            let slot = state
                .entries
                .iter()
                .position(|e| e.guid == 1480716884)
                .context("Frankie portrait entry absent")? as u32;
            state.entries[slot as usize].dirty[1] = mode == "dirty";
            let background = pixels.insert(state, settings.templates[&4600].clone())?;
            let thumbnail = pixels.insert(state, settings.templates[&108].clone())?;
            let before_visit = session.simulation.visit;
            let before_visited = session.simulation.visited.clone();
            let mut retrieval = Retrieval::new(
                session.simulation.vm.tick,
                slot,
                None,
                Some(background),
                Some(thumbnail),
                u32::from(settings.key),
            );
            let mut boundaries = std::collections::BTreeSet::new();
            let mut trace = vec![];
            let mut events = vec![];
            let mut services = 0;
            let mut restores = 0;
            let mut completed = false;
            for _ in 0..20000 {
                session.host.update_contexts(&mut session.simulation)?;
                let mut remaining = if restoring { 1 } else { 100_000 };
                let mut step = sims_sim::Step {
                    stop: sims_sim::StepStop::Waiting,
                    trace: vec![],
                    events: vec![],
                };
                let progress = retrieval.advance(
                    &mut cache::Session {
                        session: &mut session,
                        view: sims_sim::pose::portrait::view::Input {
                            viewport: [0, 0, 800, 600],
                            center: [344, 232],
                            zoom: 1,
                            rotation: 0,
                        },
                        source_key: None,
                    },
                    &mut catalog,
                    &mut pixels,
                    &settings,
                    &mut remaining,
                    &mut step,
                )?;
                trace.extend(step.trace);
                events.extend(step.events);
                let boundary = format!(
                    "{progress:?}/{:?}/{}/{}",
                    session
                        .simulation
                        .person_factories
                        .get(&196)
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
                match progress {
                    Progress::Temporary(service) => {
                        support::supply(&mut session, &mut catalog, service, [344, 232])?
                    }
                    Progress::Complete(success) => {
                        ensure!(success);
                        completed = true;
                    }
                    Progress::Yielded => {}
                }
                services += session
                    .host
                    .service(&mut session.simulation, &mut session.program)?;
                pixels.collect(&session.simulation.vm.world.image_cache)?;
                retrieval.validate(&session, &pixels)?;
                ensure!(
                    session.simulation.vm.tick == 215742
                        && session.simulation.visit == before_visit
                        && session.simulation.visited == before_visited
                );
                if restoring && boundaries.insert(boundary) {
                    (retrieval, pixels) =
                        serde_json::from_slice(&serde_json::to_vec(&(&retrieval, &pixels))?)?;
                    session = Session::restore(
                        &index,
                        serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
                    )?;
                    catalog = support::catalog_with_overlay(
                        &index,
                        serde_json::from_slice(&serde_json::to_vec(
                            &catalog.resources.overlay().map_err(anyhow::Error::msg)?,
                        )?)?,
                    )?;
                    retrieval.validate(&session, &pixels)?;
                    restores += 1;
                }
                if completed {
                    break;
                }
            }
            ensure!(completed && !session.simulation.vm.world.objects.contains_key(&196));
            ensure!(
                session.simulation.portrait_person.is_none()
                    && session.simulation.synchronous_removal.is_none()
            );
            ensure!((trace.is_empty() && services == 0) == (mode == "saved"));
            let saved = session.save();
            let output = retrieval
                .clone()
                .finish()?
                .1
                .context("portrait result absent")?;
            if restoring {
                pixels
                    .get(output)?
                    .rgba()?
                    .save(root.join(format!("cache/original-portrait-retrieval-{mode}.png")))?;
            }
            let overlay = catalog.resources.overlay().map_err(anyhow::Error::msg)?;
            ensure!((overlay == seed) == (mode != "missing"));
            // A subsequent read must borrow the cached atlas, execute no BHAV,
            // and balance the extra output owner when the dialog releases it.
            let before = session.simulation.clone();
            let mut cached = Retrieval::new(
                215742,
                slot,
                None,
                Some(background),
                Some(thumbnail),
                u32::from(settings.key),
            );
            let mut step = sims_sim::Step {
                stop: sims_sim::StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            let mut remaining = 100;
            let p = cached.advance(
                &mut cache::Session {
                    session: &mut session,
                    view: sims_sim::pose::portrait::view::Input {
                        viewport: [0, 0, 800, 600],
                        center: [344, 232],
                        zoom: 1,
                        rotation: 0,
                    },
                    source_key: None,
                },
                &mut catalog,
                &mut pixels,
                &settings,
                &mut remaining,
                &mut step,
            )?;
            ensure!(
                p == Progress::Complete(true)
                    && remaining == 100
                    && step.trace.is_empty()
                    && step.events.is_empty()
            );
            let (_, mut cached_output) = cached.finish()?;
            ensure!(cached_output == Some(output));
            pixels.release(
                &mut session.simulation.vm.world.image_cache,
                &mut cached_output,
            )?;
            ensure!(session.simulation == before);
            // A host that advances the clock cannot resume this invocation.
            session.simulation.vm.tick += 1;
            ensure!(retrieval.validate(&session, &pixels).is_err());
            session.simulation.vm.tick -= 1;
            println!(
                "retrieval {mode} restore={restoring}: {} instructions, {services} services, {restores} restores",
                trace.len()
            );
            results.push((
                serde_json::to_value(saved)?,
                retrieval,
                pixels,
                overlay,
                trace,
                events,
            ));
            if restoring {
                evidence.push(serde_json::json!({"mode":mode,"instructions":results[1].4.len(),"services":services,"restores":restores,"tick":215742,"image":format!("cache/original-portrait-retrieval-{mode}.png")}));
            }
        }
        ensure!(
            results[0] == results[1],
            "retrieval {mode} diverged across execution budgets/restores"
        );
    }
    original.validate(&index).map_err(anyhow::Error::msg)?;
    std::fs::write(
        root.join("cache/original-portrait-retrieval.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    Ok(())
}
