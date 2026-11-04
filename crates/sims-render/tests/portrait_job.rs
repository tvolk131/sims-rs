use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::portrait::{
    cache::{self, Images, Settings},
    images,
    job::{Job, Progress},
    resize::Format,
};
use sims_runtime::original_session::Session;
use sims_vm::image_cache::regeneration::{Regeneration, Request, Response};
#[path = "../../sims-content/tests/support/portrait.rs"]
mod support;

#[test]
#[ignore = "requires original post-death Mashuga capture and assets; CPU rendering"]
fn original_dead_neighbor_regeneration_creates_draws_and_removes_a_temporary_person() -> Result<()>
{
    run(0)
}

#[test]
#[ignore = "requires original post-death Mashuga capture and assets; CPU rendering"]
fn original_dead_neighbor_portrait_persists_to_owned_resource_and_reopens() -> Result<()> {
    run(1)
}

fn run(persist: i32) -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let overlay = sims_content::object_resources::Overlay::copy(
        &index,
        [index.find("UserData2/Characters/User00000.iff")?],
    )
    .map_err(anyhow::Error::msg)?;
    let mut catalog = support::catalog_with_overlay(&index, overlay.clone())?;
    let artifact = if persist == 0 {
        "original-portrait-job"
    } else {
        "original-portrait-persist-job"
    };
    let mut templates = std::collections::BTreeMap::new();
    for resource in [108, 4600, 4602, 9000, 9001] {
        templates.insert(
            resource,
            images::template(&index, resource, Format::Rgb565)?.unwrap(),
        );
    }
    let settings = Settings {
        layout: [45, 45, 5, 2, 35, 41],
        format: Format::Rgb565,
        key: Format::Rgb565.encode([255, 0, 255]),
        templates,
    };
    let mut results = vec![];
    for restore in [false, true] {
        if restore {
            // Both execution budgets start from the same original resource copy.
            catalog = support::catalog_with_overlay(&index, overlay.clone())?;
        }
        let mut session = support::capture(&index, &root)?;
        ensure!(
            session
                .simulation
                .vm
                .world
                .objects
                .values()
                .all(|o| o.definition.as_ref().is_none_or(|d| d.guid != 1480716884))
        );
        let before_visit = session.simulation.visit;
        let before_visited = session.simulation.visited.clone();
        let mut pixels = Images::default();
        let cache = &mut session.simulation.vm.world.image_cache;
        catalog
            .populate_portraits(cache, true, false, &mut |e, s| pixels.event(e, s))
            .map_err(anyhow::Error::msg)?;
        let slot = cache
            .entries
            .iter()
            .position(|e| e.guid == 1480716884)
            .context("Frankie portrait entry absent")? as u32;
        let background = pixels.insert(cache, settings.templates[&4600].clone())?;
        let thumbnail = pixels.insert(cache, settings.templates[&108].clone())?;
        // 4e92a8 uses mask1/persist0 for dirty portraits, mask1/persist1
        // after a failed saved-BMP load. Exercise each controller path explicitly.
        let mut job = Job::new(Regeneration::new(
            slot,
            1,
            persist,
            Some(background),
            Some(thumbnail),
            u32::from(settings.key),
        ));
        let mut done = false;
        let mut trace = vec![];
        let mut events = vec![];
        let mut boundaries = std::collections::BTreeSet::new();
        let mut restores = 0;
        let mut services = 0;
        for slice in 0..20000 {
            session.host.update_contexts(&mut session.simulation)?;
            let mut remaining = if restore { 1 } else { 100_000 };
            let mut step = sims_sim::Step {
                stop: sims_sim::StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            let progress = job.advance(
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
                &mut pixels,
                &settings,
                &mut remaining,
                &mut step,
            )?;
            let boundary = format!(
                "{progress:?}/factory={:?}/deleting={}/physical={}",
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
            let first = boundaries.insert(boundary);
            trace.extend(step.trace);
            events.extend(step.events);
            match progress {
                Progress::Service(request) => {
                    let response = match request {
                        Request::Resource { guid } => Response::Resource(
                            catalog
                                .portrait_resource(guid)
                                .map_err(anyhow::Error::msg)?,
                        ),
                        Request::Find { index } => {
                            ensure!(index == slot);
                            // No matching person exists in the native person registry.
                            let world = &session.simulation.vm.world;
                            ensure!(
                                world
                                    .person
                                    .as_ref()
                                    .unwrap()
                                    .person_order
                                    .iter()
                                    .all(|id| world.objects[id].definition.as_ref().unwrap().guid
                                        != 1480716884)
                            );
                            Response::Person(None)
                        }
                        Request::Eligible { resource } => {
                            ensure!(
                                catalog
                                    .portrait_definition(resource)
                                    .map_err(anyhow::Error::msg)?
                                    .guid
                                    == 1480716884
                            );
                            Response::Eligible(
                                catalog
                                    .portrait_writable(resource)
                                    .map_err(anyhow::Error::msg)?,
                            )
                        }
                        Request::Persist {
                            index: entry,
                            chunk,
                            image,
                        } => {
                            ensure!(persist == 1);
                            ensure!(
                                sims_render::portrait::persistence::persist(
                                    &mut session.simulation.vm.world.image_cache,
                                    &mut catalog,
                                    &pixels,
                                    entry,
                                    chunk,
                                    image,
                                )
                                .map_err(anyhow::Error::msg)?
                            );
                            Response::Done
                        }
                        Request::Flush { resource } => {
                            let file = catalog
                                .portrait_file(resource)
                                .map_err(anyhow::Error::msg)?;
                            catalog.resources.flush(file).map_err(anyhow::Error::msg)?;
                            ensure!(
                                catalog.resources.error(file).map_err(anyhow::Error::msg)? == 0
                            );
                            Response::Done
                        }
                        other => anyhow::bail!("unimplemented portrait host service {other:?}"),
                    };
                    job.respond(&mut session, &mut pixels, response)?;
                }
                Progress::Temporary(service) => {
                    support::supply(&mut session, &mut catalog, service, [344, 232])?
                }
                Progress::Complete(success) => {
                    ensure!(success);
                    done = true;
                }
                Progress::Yielded => {}
            }
            services += session
                .host
                .service(&mut session.simulation, &mut session.program)?;
            pixels.collect(&session.simulation.vm.world.image_cache)?;
            job.validate(&session, &pixels)?;
            ensure!(
                session.simulation.vm.tick == 215742
                    && session.simulation.visit == before_visit
                    && session.simulation.visited == before_visited
            );
            if restore && first {
                (job, pixels) = serde_json::from_slice(&serde_json::to_vec(&(&job, &pixels))?)?;
                let snapshot = sims_sim::Snapshot::capture(&session.program, &session.simulation);
                let snapshot: sims_sim::Snapshot =
                    serde_json::from_slice(&serde_json::to_vec(&snapshot)?)?;
                session.simulation = snapshot
                    .restore(&session.program)
                    .map_err(anyhow::Error::msg)
                    .with_context(|| format!("portrait job restore slice {slice}"))?;
                session.host = sims_content::character_loading::live::Host::restore(
                    &index,
                    session.host.presentation.clone(),
                    serde_json::from_slice(&serde_json::to_vec(&session.host.snapshot())?)?,
                )?;
                job.validate(&session, &pixels)?;
                let saved_overlay =
                    serde_json::to_vec(&catalog.resources.overlay().map_err(anyhow::Error::msg)?)?;
                catalog =
                    support::catalog_with_overlay(&index, serde_json::from_slice(&saved_overlay)?)?;
                restores += 1;
            }
            if done {
                break;
            }
        }
        ensure!(
            done && session.simulation.portrait_person.is_none()
                && session.simulation.synchronous_removal.is_none()
        );
        ensure!(
            !session.simulation.vm.world.objects.contains_key(&196)
                && !session.host.characters.contains_key(&196)
        );
        let cache = &session.simulation.vm.world.image_cache;
        let entry = &cache.entries[slot as usize];
        ensure!(entry.images[..2].iter().all(Option::is_some) && entry.dirty[..2] == [false; 2]);
        let rgba = pixels.get(entry.images[1].unwrap())?.rgba()?;
        if restore {
            rgba.save(root.join(format!("cache/{artifact}.png")))?;
        }
        if persist == 1 {
            let handle = catalog
                .portrait_resource(1480716884)
                .map_err(anyhow::Error::msg)?
                .unwrap();
            let expected = pixels.get(entry.images[0].unwrap())?.bmp()?;
            ensure!(
                catalog
                    .portrait_chunk(handle, *b"BMP_", 2002)
                    .map_err(anyhow::Error::msg)?
                    == Some(expected.clone())
            );
            let copy = catalog.resources.overlay().map_err(anyhow::Error::msg)?;
            let mut reopened = support::catalog_with_overlay(
                &index,
                serde_json::from_slice(&serde_json::to_vec(&copy)?)?,
            )?;
            ensure!(
                reopened
                    .portrait_resource(1480716884)
                    .map_err(anyhow::Error::msg)?
                    == Some(handle)
            );
            ensure!(
                reopened
                    .portrait_chunk(handle, *b"BMP_", 2002)
                    .map_err(anyhow::Error::msg)?
                    == Some(expected)
            );
        }
        let saved = session.save();
        let restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
        )?;
        ensure!(
            restored.simulation == session.simulation
                && restored.host.snapshot() == session.host.snapshot()
        );
        println!(
            "portrait job persist={persist} restore={restore}: {} instructions, {services} services, {restores} restores",
            trace.len()
        );
        results.push((
            saved,
            job,
            pixels,
            trace,
            events,
            restores,
            services,
            catalog.resources.overlay().map_err(anyhow::Error::msg)?,
        ));
    }
    ensure!(
        results[0].0.simulation == results[1].0.simulation
            && results[0].0.assets == results[1].0.assets
    );
    ensure!(
        results[0].1 == results[1].1
            && results[0].2 == results[1].2
            && results[0].3 == results[1].3
            && results[0].4 == results[1].4
            && results[0].7 == results[1].7
    );
    std::fs::write(
        root.join(format!("cache/{artifact}-session.json")),
        serde_json::to_vec(&(&results[1].0, &results[1].1, &results[1].2))?,
    )?;
    std::fs::write(
        root.join(format!("cache/{artifact}-overlay.json")),
        serde_json::to_vec(&catalog.resources.overlay().map_err(anyhow::Error::msg)?)?,
    )?;
    ensure!(
        (catalog.resources.overlay().map_err(anyhow::Error::msg)? == overlay) == (persist == 0)
    );
    overlay.validate(&index).map_err(anyhow::Error::msg)?;
    std::fs::write(
        root.join(format!("cache/{artifact}.json")),
        serde_json::to_vec_pretty(&serde_json::json!({
            "tick":215742,"neighbor":48,"temporary_person":196,"instructions":results[1].3.len(),"restores":results[1].5,"services":results[1].6,
            "large_and_single_instruction_budgets_identical":true,"headless_atlas":format!("cache/{artifact}.png"),"persist":persist,
            "scope":"Actual catalog/resource, native writable query, temporary factory/placement, mask1 regeneration and cleanup. Persist=1 executes native-equivalent BMP export/add/detach/flush into an owned character copy and reloads identical bytes after catalog reconstruction. Catalog/overlay restored with each pause. Camera origin explicit; live dialog not yet connected."
        }))?,
    )?;
    Ok(())
}
