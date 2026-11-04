use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::portrait::{
    cache::{self, Images, Settings},
    resize::Format,
    retrieval::Progress,
    speech::Retrieval,
};
use sims_runtime::original_session::Session;

#[path = "../../sims-content/tests/support/portrait.rs"]
#[allow(dead_code)]
mod support;

#[test]
#[ignore = "requires original assets and a current-program Mashuga checkpoint; headless CPU images"]
fn original_saved_speech_images_preserve_gameplay_and_restore() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input = std::env::var("SIMS_SPEECH_CAPTURE")
        .unwrap_or_else(|_| "cache/mashuga-v86-normal/final.json".into());
    let output =
        std::env::var("SIMS_SPEECH_OUTPUT").context("exclusive output directory required")?;
    let output = root.join(output);
    std::fs::create_dir(&output)?;
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(input))?)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut reports = vec![];
    for format in [Format::Rgb555, Format::Rgb565] {
        for zoom in [2, 3] {
            let mut pair = vec![];
            for restoring in [false, true] {
                let mut session =
                    Session::restore(&index, serde_json::from_value(data["session"].clone())?)?;
                let mut catalog = support::catalog(&index)?;
                // Keep the checkpoint's dialog template owners/pixels, too.
                // Starting a second presenter must not drop their references.
                let mut pixels: Images =
                    serde_json::from_value(data["dialog_client"]["images"].clone())?;
                let cache = &mut session.simulation.vm.world.image_cache;
                catalog
                    .populate_portraits(cache, true, false, &mut |e, s| pixels.event(e, s))
                    .map_err(anyhow::Error::msg)?;
                let entry = cache
                    .entries
                    .iter()
                    .position(|e| e.guid == 1480716884)
                    .context("Frankie cache entry absent")?;
                let slot = if zoom == 2 { 4 } else { 5 };
                ensure!(
                    cache.entries[entry].images[slot].is_none()
                        && !cache.entries[entry].dirty[slot]
                );
                let settings = Settings {
                    layout: [45, 45, 5, 2, 35, 41],
                    format,
                    key: format.encode([255, 0, 255]),
                    templates: Default::default(),
                };
                let tick = session.simulation.vm.tick;
                let before = session.simulation.clone();
                let mut operation = Retrieval::new(
                    tick,
                    entry as u32,
                    zoom,
                    None,
                    false,
                    u32::from(settings.key),
                );
                let mut step = sims_sim::Step {
                    stop: sims_sim::StepStop::Waiting,
                    trace: vec![],
                    events: vec![],
                };
                let mut call = |operation: &mut Retrieval,
                                session: &mut Session,
                                images: &mut Images,
                                remaining: &mut usize| {
                    operation.advance(
                        &mut cache::Session {
                            session,
                            view: sims_sim::pose::portrait::view::Input {
                                viewport: [0, 0, 800, 600],
                                center: [344, 232],
                                zoom: 1,
                                rotation: 0,
                            },
                            source_key: None,
                        },
                        &mut catalog,
                        images,
                        &settings,
                        remaining,
                        &mut step,
                    )
                };
                if restoring {
                    ensure!(
                        call(&mut operation, &mut session, &mut pixels, &mut 0)?
                            == Progress::Yielded
                    );
                    (operation, pixels) =
                        serde_json::from_slice(&serde_json::to_vec(&(&operation, &pixels))?)?;
                    session = Session::restore(&index, session.save())?;
                }
                let mut remaining = 100;
                ensure!(
                    call(&mut operation, &mut session, &mut pixels, &mut remaining)?
                        == Progress::Complete(true)
                );
                ensure!(remaining == 100);
                operation.validate(&session, &pixels)?;
                let (_, image) = operation.finish()?;
                let image = image.context("saved speech bitmap absent")?;
                let frame = pixels.get(image)?.rgba()?;

                // A denied dirty read must retain the transferred old wrapper,
                // even though that wrapper is nonempty and cached pixels exist.
                session.simulation.vm.world.image_cache.entries[entry].dirty[slot] = true;
                let dirty_before = session.save();
                let mut denied = Retrieval::new(
                    tick,
                    entry as u32,
                    zoom,
                    Some(image),
                    false,
                    u32::from(settings.key),
                );
                ensure!(
                    call(&mut denied, &mut session, &mut pixels, &mut remaining)?
                        == Progress::Complete(false)
                );
                let (success, mut owner) = denied.finish()?;
                ensure!(!success && owner == Some(image));
                ensure!(
                    serde_json::to_value(session.save())? == serde_json::to_value(dirty_before)?
                );
                pixels.release(&mut session.simulation.vm.world.image_cache, &mut owner)?;
                ensure!(step.trace.is_empty() && step.events.is_empty());
                let mut after = session.simulation.clone();
                after.vm.world.image_cache = before.vm.world.image_cache.clone();
                ensure!(
                    after == before,
                    "speech retrieval changed gameplay outside its image cache"
                );
                let stem = format!("frankie-{format:?}-zoom{zoom}");
                if restoring {
                    frame.save(output.join(format!("{stem}.png")))?;
                }
                pair.push((serde_json::to_value(session.save())?, pixels, frame));
            }
            ensure!(pair[0] == pair[1], "speech restore changed state or pixels");
            reports.push(serde_json::json!({"format":format,"zoom":zoom,"restored_state_and_pixels_match":true,
                "gameplay_unchanged":true,"dimensions":pair[0].2.dimensions()}));
        }
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires original assets/current Mashuga checkpoint; CPU speech regeneration"]
fn original_dirty_speech_regenerates_both_sizes() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input = std::env::var("SIMS_SPEECH_CAPTURE")
        .unwrap_or_else(|_| "cache/mashuga-v86-normal/final.json".into());
    let output = root.join(std::env::var("SIMS_SPEECH_OUTPUT")?);
    std::fs::create_dir(&output)?;
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(input))?)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let format = Format::Rgb565;
    let mut templates = std::collections::BTreeMap::new();
    for resource in [9000, 9001] {
        templates.insert(
            resource,
            sims_render::portrait::images::template(&index, resource, format)?
                .context("speech template absent")?,
        );
    }
    let settings = Settings {
        layout: [45, 45, 5, 2, 35, 41],
        format,
        key: format.encode([255, 0, 255]),
        templates,
    };
    let mut pair = vec![];
    for restoring in [false, true] {
        let mut session =
            Session::restore(&index, serde_json::from_value(data["session"].clone())?)?;
        let mut catalog = support::catalog(&index)?;
        let mut pixels: Images = serde_json::from_value(data["dialog_client"]["images"].clone())?;
        let cache = &mut session.simulation.vm.world.image_cache;
        catalog
            .populate_portraits(cache, true, false, &mut |e, s| pixels.event(e, s))
            .map_err(anyhow::Error::msg)?;
        let entry = cache
            .entries
            .iter()
            .position(|e| e.guid == 1480716884)
            .context("Frankie cache entry absent")?;
        cache.entries[entry].dirty[4] = true;
        cache.entries[entry].dirty[5] = true;
        let initial_images = cache.entries[entry].images;
        let tick = session.simulation.vm.tick;
        let before_visit = session.simulation.visit;
        let before_visited = session.simulation.visited.clone();
        let before_rng = (
            session.simulation.vm.rng_state,
            session.simulation.vm.crt_rng_state,
            session.simulation.vm.lot_rng_state,
        );
        let mut operation =
            Retrieval::new(tick, entry as u32, 2, None, true, u32::from(settings.key));
        if restoring {
            (operation, pixels) =
                serde_json::from_slice(&serde_json::to_vec(&(&operation, &pixels))?)?;
            session = Session::restore(&index, session.save())?;
        }
        let mut step = sims_sim::Step {
            stop: sims_sim::StepStop::Waiting,
            trace: vec![],
            events: vec![],
        };
        let mut remaining = 100;
        let progress = operation.advance(
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
            progress == Progress::Complete(true),
            "speech regeneration incomplete: {progress:?}"
        );
        operation.validate(&session, &pixels)?;
        let (_, mut owner) = operation.finish()?;
        let cache = &mut session.simulation.vm.world.image_cache;
        ensure!(owner == cache.entries[entry].images[4]);
        let mut frames = vec![];
        for slot in [4, 5] {
            ensure!(!cache.entries[entry].dirty[slot]);
            let image =
                cache.entries[entry].images[slot].context("regenerated speech image absent")?;
            let frame = pixels.get(image)?.rgba()?;
            if restoring {
                frame.save(output.join(format!("frankie-generated-slot{slot}.png")))?;
            }
            frames.push(frame);
        }
        for slot in [0, 1, 2, 3, 6] {
            ensure!(cache.entries[entry].images[slot] == initial_images[slot]);
        }
        pixels.release(cache, &mut owner)?;
        ensure!(
            session.simulation.vm.tick == tick
                && session.simulation.visit == before_visit
                && session.simulation.visited == before_visited
        );
        ensure!(
            before_rng
                == (
                    session.simulation.vm.rng_state,
                    session.simulation.vm.crt_rng_state,
                    session.simulation.vm.lot_rng_state
                )
        );
        ensure!(remaining == 100 && step.trace.is_empty() && step.events.is_empty());
        pair.push((serde_json::to_value(session.save())?, pixels, frames));
    }
    ensure!(
        pair[0] == pair[1],
        "regenerated speech state/pixels differ after restoration"
    );
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
        "restored_state_and_pixels_match":true,"clock_and_rng_unchanged":true,
        "native_mask":4,"only_image_slots":[4,5],"native_pixel_equivalence_claimed":false,
        "scope":"Existing live Frankie, dirty speech images, no persistence; CPU backend"}))?,
    )?;
    Ok(())
}
