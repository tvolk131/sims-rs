use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::portrait::{
    cache::{self, Images, Settings},
    images::{self, Raster},
    resize::{Format, Image},
};
use sims_runtime::original_session::{Saved, Session};
use sims_vm::image_cache::{
    Entry, State,
    regeneration::{Regeneration, Request, Response},
};
use std::collections::BTreeMap;

fn raster(width: usize, height: usize, value: u16) -> Raster {
    Raster {
        format: Format::Rgb565,
        key: None,
        image: Image {
            width,
            height,
            pixels: vec![value; width * height],
            alpha: None,
        },
    }
}

#[test]
fn appearance_invalidation_during_render_reclaims_pixels_and_preserves_new_owners() -> Result<()> {
    struct Host {
        cache: State,
        draws: usize,
    }
    impl cache::Host for Host {
        fn cache(&self) -> &State {
            &self.cache
        }
        fn cache_mut(&mut self) -> &mut State {
            &mut self.cache
        }
        fn draw(
            &mut self,
            _: u32,
            _: Format,
            request: sims_content::dialog::portrait::generators::Render,
        ) -> Result<sims_content::dialog::portrait::generators::Rendered<Image>> {
            if self.draws == 0 {
                self.cache.invalidate(42).map_err(anyhow::Error::msg)?;
            }
            self.draws += 1;
            Ok(sims_content::dialog::portrait::generators::Rendered {
                success: true,
                image: Some(
                    raster(
                        request.target[0] as usize,
                        request.target[1] as usize,
                        0x1234,
                    )
                    .image,
                ),
            })
        }
    }
    let mut host = Host {
        cache: State {
            entries: vec![Entry::new(42)],
            ..Default::default()
        },
        draws: 0,
    };
    let mut images = Images::default();
    let old = images.insert(&mut host.cache, raster(3, 2, 0x4321))?;
    host.cache
        .store_image(0, 0, Some(old), &mut |e, s| images.event(e, s))
        .map_err(anyhow::Error::msg)?;
    images.release(&mut host.cache, &mut Some(old))?;
    let mut operation = Regeneration::new(0, 1, 0, None, None, 0xf81f);
    let settings = Settings {
        layout: [2, 2, 0, 0, 2, 2],
        format: Format::Rgb565,
        key: 0xf81f,
        templates: BTreeMap::new(),
    };
    ensure!(
        images.advance(&mut host.cache, &mut operation)? == Some(Request::Resource { guid: 42 })
    );
    images.respond(&mut host.cache, &mut operation, Response::Resource(Some(7)))?;
    ensure!(images.advance(&mut host.cache, &mut operation)? == Some(Request::Find { index: 0 }));
    images.respond(&mut host.cache, &mut operation, Response::Person(Some(1)))?;
    ensure!(images.service(&mut operation, &settings, &mut host)?);
    ensure!(images.advance(&mut host.cache, &mut operation)?.is_none());
    ensure!(operation.returned() == Some(true) && host.draws == 3);
    ensure!(host.cache.refs[old as usize - 1] == 0 && images.get(old).is_err());
    let new = host.cache.entries[0].images[0].unwrap();
    ensure!(host.cache.refs[new as usize - 1] == 1 && !host.cache.entries[0].dirty[0]);
    ensure!(images.get(new)?.image.pixels == vec![0x1234; 12]);
    // Normal simulation invalidation has no renderer borrow. Reclamation must
    // preserve the resulting VM state, while removing only destroyed pixels.
    host.cache.invalidate(42).map_err(anyhow::Error::msg)?;
    let metadata = host.cache.clone();
    images.collect(&host.cache)?;
    ensure!(host.cache == metadata && images.get(new).is_err());
    Ok(())
}

#[test]
#[ignore = "requires original household portrait session and UI assets; CPU rendering"]
fn original_portrait_cache_regeneration_restores_midway_with_identical_pixels_and_vm() -> Result<()>
{
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let saved: Saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-portrait-prepared-session.json"),
    )?)?;
    let mut templates = BTreeMap::new();
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
    let mut outputs = vec![];
    for restoring in [false, true] {
        let mut session = Session::restore(&index, saved.clone())?;
        session
            .host
            .prepare_portrait(&mut session.simulation, &session.program, 196)?;
        let before = session.simulation.clone();
        let mut images = Images::default();
        let state = &mut session.simulation.vm.world.image_cache;
        let slot = state
            .entries
            .iter()
            .position(|e| e.guid == 1480716884)
            .context("Frankie cache entry absent")? as u32;
        ensure!(state.refs.is_empty());
        let background = images.insert(state, settings.templates[&4600].clone())?;
        let thumbnail = images.insert(state, settings.templates[&108].clone())?;
        let mut operation = Regeneration::new(
            slot,
            31,
            0,
            Some(background),
            Some(thumbnail),
            u32::from(settings.key),
        );
        let mut requests = vec![];
        let mut restored_session = false;
        loop {
            let Some(request) =
                images.advance(&mut session.simulation.vm.world.image_cache, &mut operation)?
            else {
                break;
            };
            requests.push(request);
            if restoring {
                (images, operation) =
                    serde_json::from_slice(&serde_json::to_vec(&(&images, &operation))?)?;
                // The thumbnail request is midway through regeneration, after
                // faces/composite and the five-color strip have been stored.
                if matches!(request, Request::Render { slot: 3, .. }) {
                    session = Session::restore(
                        &index,
                        serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
                    )?;
                    restored_session = true;
                }
            }
            let old_image_count = session.simulation.vm.world.image_cache.refs.len();
            let state = &mut session.simulation.vm.world.image_cache;
            match request {
                Request::Resource { guid } => {
                    ensure!(guid == 1480716884);
                    // Resource/person lookup was validated independently. This
                    // integration starts with the already constructed resident.
                    images.respond(state, &mut operation, Response::Resource(Some(739)))?;
                }
                Request::Find { index } => {
                    ensure!(index == slot);
                    images.respond(state, &mut operation, Response::Person(Some(196)))?;
                }
                _ => {
                    ensure!(
                        images.service(
                            &mut operation,
                            &settings,
                            &mut cache::Session {
                                session: &mut session,
                                view: sims_sim::pose::portrait::view::Input {
                                    viewport: [0, 0, 800, 600],
                                    center: [344, 232],
                                    zoom: 1,
                                    rotation: 0
                                },
                                source_key: None,
                            }
                        )?,
                        "unexpected unresolved service {request:?}"
                    );
                }
            }
            if matches!(request, Request::Render { slot: 3, .. }) {
                ensure!(
                    session.simulation.vm.world.image_cache.refs.len() == old_image_count,
                    "thumbnail replaced its existing handle"
                );
            }
        }
        ensure!(operation.returned() == Some(true) && restored_session == restoring);
        let state = &mut session.simulation.vm.world.image_cache;
        images.validate(state)?;
        let handles = state.entries[slot as usize].images;
        ensure!(handles.iter().all(Option::is_some));
        ensure!(state.entries[slot as usize].dirty == [false; 7]);
        let mut gallery = image::RgbaImage::from_pixel(440, 560, image::Rgba([24, 32, 37, 255]));
        for (slot, handle) in handles.into_iter().enumerate() {
            let handle = handle.unwrap();
            ensure!(state.refs[handle as usize - 1] == 1);
            let rgba = images.get(handle)?.rgba()?;
            let (x, y) = if slot == 1 {
                (240, 8)
            } else {
                (8, 8 + 80 * (slot - usize::from(slot > 1)) as i64)
            };
            image::imageops::overlay(&mut gallery, &rgba, x, y);
        }
        if restoring {
            gallery.save(root.join("cache/original-regenerated-portrait-cache.png"))?;
        }
        // Cache generation is allowed to update image metadata, not the tick,
        // scheduler, residents, RNG streams or any other simulation state.
        let mut expected = before;
        expected.vm.world.image_cache = state.clone();
        ensure!(session.simulation == expected);
        outputs.push((session.save(), images, operation, requests));
    }
    ensure!(outputs[0].0.simulation == outputs[1].0.simulation);
    ensure!(outputs[0].0.assets == outputs[1].0.assets);
    ensure!(
        outputs[0].1 == outputs[1].1
            && outputs[0].2 == outputs[1].2
            && outputs[0].3 == outputs[1].3
    );
    let mut cleaned = vec![];
    for (run, (saved, mut pixels, _, _)) in outputs.iter().cloned().enumerate() {
        let mut session = Session::restore(&index, saved)?;
        let visit = session.simulation.visit;
        let visited = session.simulation.visited.clone();
        let tick = session.simulation.vm.tick;
        let before = session.simulation.clone();
        ensure!(
            session
                .simulation
                .begin_synchronous_removal(&session.program, -1)
                .is_err()
        );
        ensure!(session.simulation == before);
        session
            .simulation
            .begin_synchronous_removal(&session.program, 196)
            .map_err(anyhow::Error::msg)?;
        let frozen = session.simulation.clone();
        ensure!(session.simulation.take_synchronous_removal().is_err());
        ensure!(
            session
                .simulation
                .run_current_tick(&session.program, 100)
                .is_err()
        );
        ensure!(session.simulation.finish_registry_pass().is_err());
        ensure!(session.simulation == frozen);
        let mut complete = false;
        let mut trace = vec![];
        let mut events = vec![];
        let mut restores = 0;
        let mut boundaries = std::collections::BTreeSet::new();
        for slice in 0..1000 {
            session.host.update_contexts(&mut session.simulation)?;
            let mut remaining = if run == 0 { 100_000 } else { 1 };
            let mut step = sims_sim::Step {
                stop: sims_sim::StepStop::Waiting,
                trace: vec![],
                events: vec![],
            };
            complete = session
                .simulation
                .drive_synchronous_removal(&session.program, &mut remaining, &mut step)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("portrait cleanup run {run} slice {slice}"))?;
            let signature = format!(
                "{:?}/deletion={}/retired={}/physical={}",
                step.trace.last(),
                session.simulation.deletion.len(),
                session.simulation.retired_characters.len(),
                session
                    .simulation
                    .native_lot
                    .as_ref()
                    .unwrap()
                    .removal
                    .is_some()
            );
            let first = boundaries.insert(signature);
            trace.extend(step.trace);
            events.extend(step.events);
            session
                .host
                .service(&mut session.simulation, &mut session.program)?;
            pixels.collect(&session.simulation.vm.world.image_cache)?;
            ensure!(
                session.simulation.vm.tick == tick
                    && session.simulation.visit == visit
                    && session.simulation.visited == visited
            );
            if run == 1 && (first || complete) {
                let sim: sims_sim::Snapshot = serde_json::from_slice(&serde_json::to_vec(
                    &sims_sim::Snapshot::capture(&session.program, &session.simulation),
                )?)?;
                let sim = sim
                    .restore(&session.program)
                    .map_err(anyhow::Error::msg)
                    .with_context(|| format!("portrait cleanup restore slice {slice}"))?;
                ensure!(sim == session.simulation);
                session.simulation = sim;
                let assets = session.host.snapshot();
                session.host = sims_content::character_loading::live::Host::restore(
                    &index,
                    session.host.presentation.clone(),
                    serde_json::from_slice(&serde_json::to_vec(&assets)?)?,
                )?;
                ensure!(session.host.snapshot() == assets);
                let restored: Images = serde_json::from_slice(&serde_json::to_vec(&pixels)?)?;
                ensure!(restored == pixels);
                restores += 1;
            }
            if complete {
                break;
            }
        }
        ensure!(complete, "portrait cleanup did not complete");
        ensure!(
            session
                .simulation
                .take_synchronous_removal()
                .map_err(anyhow::Error::msg)?
                == 196
        );
        ensure!(
            !session.simulation.vm.world.objects.contains_key(&196)
                && !session.simulation.characters.contains_key(&196)
                && !session.simulation.retired_characters.contains_key(&196)
                && !session.simulation.person_factories.contains_key(&196)
                && !session.host.characters.contains_key(&196)
                && session.simulation.deletion.is_empty()
        );
        // Original cache ownership outlives the temporary person it pictured.
        let entry = session
            .simulation
            .vm
            .world
            .image_cache
            .entries
            .iter()
            .find(|e| e.guid == 1480716884)
            .unwrap();
        ensure!(entry.images.iter().all(Option::is_some) && entry.dirty == [false; 7]);
        pixels.validate(&session.simulation.vm.world.image_cache)?;
        let restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
        )?;
        ensure!(
            restored.simulation == session.simulation
                && restored.host.snapshot() == session.host.snapshot()
        );
        println!(
            "portrait cleanup run {run}: {} instructions, {restores} restores, all 7 cache images retained",
            trace.len()
        );
        cleaned.push((session.save(), pixels, trace, events, restores));
    }
    ensure!(
        cleaned[0].0.simulation == cleaned[1].0.simulation
            && cleaned[0].0.assets == cleaned[1].0.assets
    );
    ensure!(
        cleaned[0].1 == cleaned[1].1
            && cleaned[0].2 == cleaned[1].2
            && cleaned[0].3 == cleaned[1].3
    );
    std::fs::write(
        root.join("cache/original-generated-portrait-cleanup-session.json"),
        serde_json::to_vec(&(&cleaned[1].0, &cleaned[1].1))?,
    )?;
    std::fs::write(
        root.join("cache/original-generated-portrait-cleanup.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "tick":cleaned[1].0.simulation.state.vm.tick,"person":196,"instructions":cleaned[1].2.len(),"restores":cleaned[1].4,
            "large_and_single_instruction_budgets_identical":true,"traces_events_assets_pixels_and_vm_identical":true,
            "all_seven_cache_images_survive_person_removal":true,"source_session":"cache/original-portrait-prepared-session.json"
        }))?,
    )?;
    std::fs::write(
        root.join("cache/original-regenerated-portrait-cache.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "requests":outputs[0].3,"restored_midway":true,"pixels_and_vm_identical":true,
            "slots":7,"tick":outputs[0].0.simulation.state.vm.tick,
            "scope":"Actual existing prepared resident and live image cache; resource/find responses supplied; temporary lifecycle, persistence and live urn dialog remain"
        }))?,
    )?;
    Ok(())
}
