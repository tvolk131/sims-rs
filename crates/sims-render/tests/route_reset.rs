use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{
    dialog::client::{Client, Inputs},
    original_house::OriginalHouse,
    software,
};
use sims_runtime::original_session::{Progress, Session};

fn restore(index: &Index, session: &mut Session, client: &mut Client) -> Result<()> {
    let saved = session.save();
    let owned = client.save(session)?;
    let resumed = Session::restore(index, serde_json::from_slice(&serde_json::to_vec(&saved)?)?)?;
    let other = Client::restore(
        &resumed,
        serde_json::from_slice(&serde_json::to_vec(&owned)?)?,
    )?;
    ensure!(serde_json::to_value(resumed.save())? == serde_json::to_value(saved)?);
    ensure!(serde_json::to_value(other.save(&resumed)?)? == serde_json::to_value(owned)?);
    *session = resumed;
    *client = other;
    Ok(())
}

fn restore_slice(index: &Index, session: &mut Session, client: &mut Client) -> Result<()> {
    // Keep the already loaded immutable program during these tightly spaced
    // checkpoints. The later continuation also exercises full Session restore.
    let saved = session.save();
    let owned = client.save(session)?;
    session.simulation =
        serde_json::from_slice::<sims_sim::Snapshot>(&serde_json::to_vec(&saved.simulation)?)?
            .restore(&session.program)
            .map_err(anyhow::Error::msg)?;
    session.host = sims_content::character_loading::live::Host::restore(
        index,
        saved.view.clone(),
        serde_json::from_slice(&serde_json::to_vec(&saved.assets)?)?,
    )?;
    *client = Client::restore(
        session,
        serde_json::from_slice(&serde_json::to_vec(&owned)?)?,
    )?;
    ensure!(serde_json::to_value(session.save())? == serde_json::to_value(saved)?);
    ensure!(serde_json::to_value(client.save(session)?)? == serde_json::to_value(owned)?);
    Ok(())
}

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

#[test]
#[ignore = "requires original assets and the diagnostic tick-226227 Iced capture; CPU rendering"]
fn routed_person_reset_restores_and_continues_the_original_household() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/dialog-client-continuation-save.json"),
    )?)?;
    let mut session = Session::restore(&index, serde_json::from_value(saved["session"].clone())?)?;
    let mut client = Client::restore(
        &session,
        serde_json::from_value(saved["dialog_client"].clone())?,
    )?;
    let mut other_session = Session::restore(&index, session.save())?;
    let mut other_client = Client::restore(&other_session, client.save(&session)?)?;
    let start = session.simulation.vm.tick;
    ensure!(start == 226227);
    ensure!(session.simulation.characters[&195].native_walk.is_some());
    ensure!(session.simulation.route_control.contains_key(&6088));
    let path = session.simulation.native_lot.as_ref().unwrap().route_paths[&195].clone();
    let mut reset_restores = 0;
    let mut appearance_with_retained_route = false;
    let started = std::time::Instant::now();
    // Slice the actual Burn/Reset at each exposed BHAV/asset boundary. Compare
    // its completed tick to an unsliced run from the identical source save.
    for slice in 0..256 {
        session.host.update_contexts(&mut session.simulation)?;
        session
            .simulation
            .run_current_tick(&session.program, 1)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("Reset slice {slice}"))?;
        if !session
            .simulation
            .pending_reset_character_services()
            .is_empty()
        {
            ensure!(
                session.simulation.route_control.contains_key(&6088),
                "route destroyed before ReloadAppearance"
            );
            ensure!(session.simulation.characters[&195].native_walk.is_none());
            if !appearance_with_retained_route {
                let mut broken = serde_json::to_value(sims_sim::Snapshot::capture(
                    &session.program,
                    &session.simulation,
                ))?;
                let frames = broken["state"]["native_burn"]["reset"]["stack"]
                    .as_array_mut()
                    .context("Reset frames absent")?;
                let owner = frames
                    .iter_mut()
                    .find(|f| f["Person"]["reset"]["route"]["id"] == 6088)
                    .context("retained route owner absent")?;
                owner["Person"]["reset"]["route"] = serde_json::Value::Null;
                let error = serde_json::from_value::<sims_sim::Snapshot>(broken)?
                    .restore(&session.program)
                    .unwrap_err();
                ensure!(
                    error.contains("route controller without continuation"),
                    "wrong missing-owner failure: {error}"
                );
            }
            appearance_with_retained_route = true;
        }
        restore_slice(&index, &mut session, &mut client)?;
        reset_restores += 1;
        println!("restored route Reset boundary {reset_restores}");
        session
            .host
            .service(&mut session.simulation, &mut session.program)?;
        let resetting = session
            .simulation
            .native_burn
            .as_ref()
            .and_then(|e| e.resetting_person());
        if resetting.is_none() {
            ensure!(!session.simulation.route_control.contains_key(&6088));
            ensure!(
                session.simulation.native_lot.as_ref().unwrap().route_paths[&195] == path,
                "native Reset erased retained path storage"
            );
            break;
        }
    }
    ensure!(appearance_with_retained_route);
    ensure!(
        session
            .simulation
            .native_burn
            .as_ref()
            .and_then(|e| e.resetting_person())
            .is_none(),
        "Reset slice budget exceeded"
    );
    ensure!(
        session.simulation.vm.tick == start,
        "reset advanced the lot clock"
    );
    println!("route Reset completed at tick {start}, {reset_restores} restored boundaries");
    let mut boundary = None;
    let mut restores = 0;
    let limit = 10_000;
    for slice in 0..100_000 {
        let actual = step(&mut session, &mut client).map_err(|e| format!("{e:#}"));
        let expected = step(&mut other_session, &mut other_client).map_err(|e| format!("{e:#}"));
        ensure!(
            actual == expected,
            "different partition outcome at slice {slice}: {actual:?} / {expected:?}"
        );
        let elapsed = session.simulation.vm.tick - start;
        let stop = actual.is_err() || actual == Ok(Progress::Dialog);
        if slice == 0 || stop || elapsed % 500 == 0 {
            ensure!(
                serde_json::to_value(session.save())?
                    == serde_json::to_value(other_session.save())?,
                "VM/assets differ at +{elapsed}"
            );
            ensure!(
                serde_json::to_value(client.save(&session)?)?
                    == serde_json::to_value(other_client.save(&other_session)?)?,
                "dialog resources differ at +{elapsed}"
            );
            println!(
                "post-route-reset tick {}: {actual:?}",
                session.simulation.vm.tick
            );
        }
        if stop {
            boundary = Some(format!("{actual:?}"));
            break;
        }
        if [500, 5000].contains(&elapsed) && actual == Ok(Progress::Tick) {
            restore(&index, &mut session, &mut client)?;
            restores += 1;
        }
        if elapsed >= limit {
            break;
        }
    }
    ensure!(
        session.simulation.vm.tick > start,
        "did not advance past original Reset frontier"
    );
    ensure!(boundary.is_some() || session.simulation.vm.tick - start == limit);
    let before = serde_json::to_value(session.save())?;
    for floor in [1, 2] {
        let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.55, floor);
        let image = software::render(&OriginalHouse::default().scene(&session, camera)?);
        ensure!(
            image == software::render(&OriginalHouse::default().scene(&other_session, camera)?)
        );
        image.save(root.join(format!("cache/route-reset-floor-{floor}.png")))?;
    }
    ensure!(
        serde_json::to_value(session.save())? == before,
        "render mutated household"
    );
    let report = serde_json::json!({
        "scope":"Explicit empty-neighborhood/InferredRectangles diagnostic with prior player selection and death-dialog OK. Original inputs read-only; normal Mashuga frontier unchanged.",
        "start_tick":start,"tick":session.simulation.vm.tick,"ticks_from_import":session.simulation.vm.tick-178126,
        "additional_ticks":session.simulation.vm.tick-start,"limit":limit,"boundary":boundary,
        "reset_restores":reset_restores,"continuation_restores":restores,
        "appearance_retained_route_context":appearance_with_retained_route,"reset_retained_path":true,
        "sliced_reset_and_unsliced_tick_identical":true,"restored_vm_assets_client_identical":true,
        "two_headless_views_identical":true,"two_run_seconds":started.elapsed().as_secs_f64(),
    });
    println!("{report}");
    std::fs::write(
        root.join("cache/route-reset-continuation.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    std::fs::write(
        root.join("cache/route-reset-continuation-save.json"),
        serde_json::to_vec(&serde_json::json!({
            "session":session.save(),"dialog_client":client.save(&session)?,"floor":1,"zoom":0.55,"resident":195,"selected":null
        }))?,
    )?;
    Ok(())
}
