use anyhow::{Context, Result, ensure};
use sims_content::{Index, object_resources::Overlay};
use sims_render::{
    dialog::{Pending, Progress},
    portrait::{
        cache::{self, Images, Settings},
        images,
        resize::Format,
    },
};
use sims_runtime::original_session::{Progress as SessionProgress, Session};
#[path = "../../sims-content/tests/support/portrait.rs"]
mod support;

#[test]
#[ignore = "requires original Mashuga capture and assets; headless CPU rendering"]
fn original_death_dialog_constructs_once_and_resumes_household() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let seed = Overlay::copy(&index, [index.find("UserData2/Characters/User00000.iff")?])
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
    for dirty in [false, true] {
        let mut results = vec![];
        for restoring in [false, true] {
            let mut session = support::capture(&index, &root)?;
            // Retry only the documented unsupported primitive at its unchanged
            // frame/instruction. No completed dialog or future state is invented.
            let thread = 439;
            let t = session
                .simulation
                .vm
                .threads
                .get_mut(&thread)
                .context("urn thread absent")?;
            ensure!(
                matches!(&t.status,sims_vm::Status::Faulted(f) if f.message=="Dialog explicit icon selection unsupported")
            );
            ensure!(
                t.stack.last().unwrap().location.bhav == 4106
                    && t.stack.last().unwrap().location.instruction == 1
                    && t.stack.last().unwrap().stack_object == 48
            );
            t.status = sims_vm::Status::Ready;
            let mut catalog = support::catalog_with_overlay(&index, seed.clone())?;
            let mut pixels = Images::default();
            let cache = &mut session.simulation.vm.world.image_cache;
            catalog
                .populate_portraits(cache, true, false, &mut |e, s| pixels.event(e, s))
                .map_err(anyhow::Error::msg)?;
            let slot = cache
                .entries
                .iter()
                .position(|e| e.guid == 1480716884)
                .unwrap();
            cache.entries[slot].dirty[1] = dirty;
            let background = pixels.insert(cache, settings.templates[&4600].clone())?;
            let thumbnail = pixels.insert(cache, settings.templates[&108].clone())?;
            ensure!(session.step()? == SessionProgress::DialogConstruction(thread));
            let before_visit = session.simulation.visit;
            let before_visited = session.simulation.visited.clone();
            let mut pending = Pending::begin(
                &session,
                &mut catalog,
                thread,
                Some(background),
                Some(thumbnail),
                u32::from(settings.key),
            )?;
            ensure!(
                pending.record().title == "Rest In Peace"
                    && pending.record().body
                        == "Deepest sympathy! Frankie has just died. Though the body is gone, the spirit will always remain.",
                "actual dialog: {:?}",
                pending.record()
            );
            ensure!(session.simulation.vm.world.dialogs.records.is_empty());
            let frozen = session.simulation.clone();
            ensure!(session.step()? == SessionProgress::DialogConstruction(thread));
            ensure!(session.simulation == frozen);
            let mut boundaries = std::collections::BTreeSet::new();
            let mut trace = vec![];
            let mut events = vec![];
            let mut services = 0;
            let mut restores = 0;
            let mut done = false;
            for _ in 0..20000 {
                session.host.update_contexts(&mut session.simulation)?;
                let mut budget = if restoring { 1 } else { 100_000 };
                let mut step = sims_sim::Step {
                    stop: sims_sim::StepStop::Waiting,
                    trace: vec![],
                    events: vec![],
                };
                let progress = pending.advance(
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
                    &mut budget,
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
                    Progress::Yielded => {}
                    Progress::Complete => done = true,
                }
                services += session
                    .host
                    .service(&mut session.simulation, &mut session.program)?;
                pixels.collect(&session.simulation.vm.world.image_cache)?;
                ensure!(
                    session.simulation.vm.tick == 215742
                        && session.simulation.visit == before_visit
                        && session.simulation.visited == before_visited
                );
                if !done {
                    pending.validate(&session, &pixels)?;
                    ensure!(session.simulation.vm.world.dialogs.records.is_empty());
                }
                if restoring && boundaries.insert(boundary) {
                    (pending, pixels) =
                        serde_json::from_slice(&serde_json::to_vec(&(&pending, &pixels))?)?;
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
                    if !done {
                        pending.validate(&session, &pixels)?;
                    }
                    restores += 1;
                }
                if done {
                    break;
                }
            }
            ensure!(done && !session.simulation.vm.world.objects.contains_key(&196));
            ensure!((trace.is_empty() && services == 0) == !dirty);
            ensure!(session.simulation.vm.world.dialogs.records.len() == 1);
            let record = session.simulation.vm.world.dialogs.records[0].clone();
            let image = record
                .portrait
                .as_ref()
                .context("death dialog portrait absent")?;
            if restoring {
                image::imageops::crop_imm(&pixels.get(image.handle)?.rgba()?, 0, 0, 45, 45)
                    .to_image()
                    .save(root.join(format!("cache/original-death-dialog-{dirty}.png")))?;
            }
            results.push((
                serde_json::to_value(session.save())?,
                pending,
                pixels,
                catalog.resources.overlay().map_err(anyhow::Error::msg)?,
                trace,
                events,
            ));
            println!(
                "death dialog dirty={dirty} restore={restoring}: {} instructions, {services} services, {restores} restores",
                results.last().unwrap().4.len()
            );
            if restoring {
                std::fs::write(
                    root.join(format!("cache/original-death-dialog-{dirty}-pixels.json")),
                    serde_json::to_vec(&results.last().unwrap().2)?,
                )?;
                std::fs::write(
                    root.join(format!("cache/original-death-dialog-{dirty}-state.json")),
                    serde_json::to_vec(&results.last().unwrap().0)?,
                )?;
                // This is an explicit test player command, not an engine default.
                ensure!(session.step()? == SessionProgress::Dialog);
                ensure!(session.simulation.vm.world.dialogs.modal == Some(record.id));
                session
                    .simulation
                    .vm
                    .answer_dialog(record.id, 2)
                    .map_err(anyhow::Error::msg)?;
                let mut outcomes = vec![];
                for _ in 0..100 {
                    let progress = session.step().map_err(|e| format!("{e:#}"));
                    let stop = progress != Ok(SessionProgress::Tick);
                    outcomes.push(format!("{progress:?}"));
                    if stop {
                        break;
                    }
                }
                println!(
                    "after dialog dirty={dirty}: tick={}, last={:?}",
                    session.simulation.vm.tick,
                    outcomes.last()
                );
                evidence.push(serde_json::json!({"dirty":dirty,"instructions":results.last().unwrap().4.len(),"services":services,"restores":restores,"record":record,"final_tick":session.simulation.vm.tick,"post_response":outcomes}));
                std::fs::write(
                    root.join(format!(
                        "cache/original-death-dialog-{dirty}-continued.json"
                    )),
                    serde_json::to_vec(&session.save())?,
                )?;
            }
        }
        ensure!(
            results[0] == results[1],
            "dialog construction differs after budget pauses/restores"
        );
    }
    seed.validate(&index).map_err(anyhow::Error::msg)?;
    std::fs::write(
        root.join("cache/original-death-dialog.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    Ok(())
}
