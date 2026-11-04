use anyhow::{Result, ensure};
use sims_content::Index;
use sims_render::{original_house::OriginalHouse, portrait::cache::Images, software};
use sims_runtime::original_session::{Progress, Saved, Session};

#[test]
#[ignore = "requires the ordinary Iced death-dialog regression save and original assets; CPU rendering"]
fn household_with_owned_dialog_assets_continues_and_restores() -> Result<()> {
    use sims_render::dialog::client::{Client, Inputs};
    fn step(session: &mut Session, client: &mut Client) -> Result<Progress> {
        if client.pending() && !client.advance(session, 256)?.0 {
            return Ok(Progress::DialogConstruction(0));
        }
        let progress = session.step()?;
        if let Progress::DialogConstruction(thread) = progress {
            let camera = OriginalHouse::camera(session, 1000, 740, 0, 0.55, 1);
            client.begin(session, thread, Inputs::camera(camera, 64)?)?;
        }
        client.collect(session)?;
        Ok(progress)
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/dialog-client-iced-save.json"),
    )?)?;
    let mut session = Session::restore(&index, serde_json::from_value(saved["session"].clone())?)?;
    let mut restored = Session::restore(&index, session.save())?;
    let mut client = Client::restore(
        &session,
        serde_json::from_value(saved["dialog_client"].clone())?,
    )?;
    let mut other = Client::restore(&restored, client.save(&session)?)?;
    ensure!(session.simulation.vm.tick == 215772);
    let start = session.simulation.vm.tick;
    let limit = 20_000;
    let mut boundary = None;
    let mut construction_slices = 0;
    let mut restores = 0;
    let started = std::time::Instant::now();
    for slice in 0..100_000 {
        let actual = step(&mut session, &mut client).map_err(|e| format!("{e:#}"));
        let expected = step(&mut restored, &mut other).map_err(|e| format!("{e:#}"));
        ensure!(
            actual == expected,
            "restored outcome differs at slice {slice}"
        );
        let elapsed = session.simulation.vm.tick - start;
        let stop = actual.is_err() || actual == Ok(Progress::Dialog);
        if stop || elapsed % 500 == 0 {
            ensure!(
                serde_json::to_value(session.save())? == serde_json::to_value(restored.save())?
                    && serde_json::to_value(client.save(&session)?)?
                        == serde_json::to_value(other.save(&restored)?)?,
                "restored VM/assets/dialog owner differ at +{elapsed}"
            );
            println!(
                "owned dialog household tick {}: {actual:?}",
                session.simulation.vm.tick
            );
        }
        if stop {
            boundary = Some(format!("{actual:?}"));
            break;
        }
        if matches!(actual, Ok(Progress::DialogConstruction(_))) {
            construction_slices += 1;
        } else if [500, 10_000].contains(&elapsed) {
            let checkpoint = other.save(&restored)?;
            restored = Session::restore(
                &index,
                serde_json::from_slice(&serde_json::to_vec(&restored.save())?)?,
            )?;
            other = Client::restore(
                &restored,
                serde_json::from_slice(&serde_json::to_vec(&checkpoint)?)?,
            )?;
            restores += 1;
        }
        if elapsed == limit {
            break;
        }
    }
    ensure!(
        boundary.is_some() || session.simulation.vm.tick - start == limit,
        "slice limit exceeded"
    );
    ensure!(serde_json::to_value(session.save())? == serde_json::to_value(restored.save())?);
    ensure!(
        serde_json::to_value(client.save(&session)?)?
            == serde_json::to_value(other.save(&restored)?)?
    );
    let seconds = started.elapsed().as_secs_f64();
    let before = serde_json::to_value(session.save())?;
    for floor in [1, 2] {
        let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.55, floor);
        let image = software::render(&OriginalHouse::default().scene(&session, camera)?);
        ensure!(image == software::render(&OriginalHouse::default().scene(&restored, camera)?));
        image.save(root.join(format!(
            "cache/dialog-client-continuation-floor-{floor}.png"
        )))?;
    }
    ensure!(
        serde_json::to_value(session.save())? == before,
        "render changed household"
    );
    let report = serde_json::json!({
        "scope":"Explicit empty-neighborhood/InferredRectangles diagnostic, continuing the ordinary Iced regression after its explicit player OK. Original input files read-only.",
        "start_tick":start,"tick":session.simulation.vm.tick,"ticks_from_import":session.simulation.vm.tick-178126,
        "additional_ticks":session.simulation.vm.tick-start,"limit":limit,"boundary":boundary,
        "construction_slices":construction_slices,"restores":restores,
        "restored_vm_assets_dialog_owner_identical":true,"two_headless_views_identical":true,"two_run_seconds":seconds,
    });
    println!("{report}");
    std::fs::write(
        root.join("cache/dialog-client-continuation.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    std::fs::write(
        root.join("cache/dialog-client-continuation-save.json"),
        serde_json::to_vec(&serde_json::json!({
            "session":session.save(),"dialog_client":client.save(&session)?,"floor":1,"zoom":0.55,"resident":195,"selected":null
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires source dialog-construction regression outputs; original assets; CPU rendering"]
fn household_continues_after_explicit_death_dialog_response() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let saved: Saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-death-dialog-false-continued.json"),
    )?)?;
    let mut session = Session::restore(&index, saved.clone())?;
    let mut restored = Session::restore(&index, saved)?;
    let mut images: Images = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-death-dialog-false-pixels.json"),
    )?)?;
    images.collect(&session.simulation.vm.world.image_cache)?;
    let start = session.simulation.vm.tick;
    let mut boundary = None;
    let started = std::time::Instant::now();
    for tick in 0..5000 {
        let actual = session.step().map_err(|e| format!("{e:#}"));
        let expected = restored.step().map_err(|e| format!("{e:#}"));
        ensure!(
            actual == expected,
            "restored household outcome differs at +{tick}"
        );
        images.collect(&session.simulation.vm.world.image_cache)?;
        if (tick + 1) % 500 == 0 || actual != Ok(Progress::Tick) {
            ensure!(
                session.simulation == restored.simulation
                    && session.host.snapshot() == restored.host.snapshot(),
                "full restored household differs at +{tick}"
            );
            println!(
                "post-dialog tick {}: {actual:?}",
                session.simulation.vm.tick
            );
        }
        if actual != Ok(Progress::Tick) {
            boundary = Some(format!("{actual:?}"));
            break;
        }
        if tick == 499 {
            restored = Session::restore(
                &index,
                serde_json::from_slice(&serde_json::to_vec(&restored.save())?)?,
            )?;
        }
    }
    ensure!(
        session.simulation == restored.simulation
            && session.host.snapshot() == restored.host.snapshot()
    );
    let seconds = started.elapsed().as_secs_f64();
    let mut renderer = OriginalHouse::default();
    let mut restored_renderer = OriginalHouse::default();
    let before = session.simulation.clone();
    let before_assets = session.host.snapshot();
    for floor in [1, 2] {
        let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.55, floor);
        let image = software::render(&renderer.scene(&session, camera)?);
        ensure!(image == software::render(&restored_renderer.scene(&restored, camera)?));
        image.save(root.join(format!("cache/original-after-death-floor-{floor}.png")))?;
    }
    ensure!(
        session.simulation == before && session.host.snapshot() == before_assets,
        "visual check advanced simulation"
    );
    let report = serde_json::json!({
        "scope":"Explicit empty-neighborhood and InferredRectangles diagnostic, continued from the original death dialog after a test player OK command. Not normal imported-house progress.",
        "start_tick":start,"tick":session.simulation.vm.tick,"ticks_from_import":session.simulation.vm.tick-178126,
        "additional_ticks":session.simulation.vm.tick-start,"boundary":boundary,"limit":5000,
        "restored_simulation_and_assets_identical":true,"two_headless_views_identical":true,"two_run_seconds":seconds,
    });
    println!("{report}");
    std::fs::write(
        root.join("cache/original-after-death-progress.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    std::fs::write(
        root.join("cache/original-after-death-session.json"),
        serde_json::to_vec(&session.save())?,
    )?;
    std::fs::write(
        root.join("cache/original-after-death-pixels.json"),
        serde_json::to_vec(&images)?,
    )?;
    Ok(())
}
