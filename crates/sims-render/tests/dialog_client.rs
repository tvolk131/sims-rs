use anyhow::{Result, ensure};
use sims_content::Index;
use sims_render::{
    Camera,
    dialog::client::{Client, Inputs},
    original_house::OriginalHouse,
};
use sims_runtime::original_session::{Progress, Session};
#[path = "../../sims-content/tests/support/portrait.rs"]
#[allow(dead_code)]
mod support;

#[test]
fn displayed_camera_drives_ground_search_and_frozen_native_view() -> Result<()> {
    for rotation in 0..4 {
        for level in [1, 2] {
            for zoom in [0.25, 0.55, 1.] {
                let camera = Camera {
                    width: 1000,
                    height: 740,
                    rotation,
                    level,
                    zoom,
                    center: glam::Vec3::new(25., if level == 1 { 0. } else { 3. }, 30.),
                };
                let inputs = Inputs::camera(camera, 64)?;
                let physical = inputs.search.origin(64, inputs.projected);
                let point = glam::Vec3::new(physical[1] as f32 / 16., 0., physical[0] as f32 / 16.);
                let screen = camera.screen(point);
                ensure!(
                    (screen.x - 500.).abs() < 5. && (screen.y - 370.).abs() < 5.,
                    "rot={rotation} floor={level} zoom={zoom}: {screen:?}"
                );
                let native = inputs
                    .view
                    .build()
                    .map_err(anyhow::Error::msg)?
                    .screen
                    .point([point.z * 3., 0., point.x * 3.])
                    .map_err(anyhow::Error::msg)?;
                ensure!(
                    (native[0] - 500.).abs() < 5. && (native[1] - 370.).abs() < 5.,
                    "native center {native:?}"
                );
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires original post-death Mashuga assets; real client owner and CPU visuals"]
fn original_dialog_client_owns_camera_resources_and_mid_callback_saves() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = if let Ok(path) = std::env::var("SIMS_DIALOG_CLIENT_OUTPUT") {
        let output = root.join(path);
        ensure!(output.starts_with(root.join("cache")));
        std::fs::create_dir(&output)?;
        output
    } else {
        root.join("cache")
    };
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut outcomes = vec![];
    for restoring in [false, true] {
        let mut session = support::capture(&index, &root)?;
        ensure!(
            matches!(&session.simulation.vm.threads[&439].status,sims_vm::Status::Faulted(f) if f.message=="Dialog explicit icon selection unsupported")
        );
        session.simulation.vm.threads.get_mut(&439).unwrap().status = sims_vm::Status::Ready;
        let mut client = Client::new(&mut session)?;
        let cache = &mut session.simulation.vm.world.image_cache;
        cache
            .entries
            .iter_mut()
            .find(|e| e.guid == 1480716884)
            .unwrap()
            .dirty[1] = true;
        ensure!(session.step()? == Progress::DialogConstruction(439));
        let camera = OriginalHouse::camera(&session, 1000, 740, 0, 0.55, 1);
        let inputs = Inputs::camera(camera, 64)?;
        client.begin(&session, 439, inputs)?;
        let before_visit = session.simulation.visit;
        let before_visited = session.simulation.visited.clone();
        let mut trace = vec![];
        let mut events = vec![];
        let mut boundaries = std::collections::BTreeSet::new();
        let mut restores = 0;
        let mut done = false;
        for _ in 0..20000 {
            let (finished, step) =
                client.advance(&mut session, if restoring { 1 } else { 100_000 })?;
            trace.extend(step.trace);
            events.extend(step.events);
            ensure!(
                session.simulation.vm.tick == 215742
                    && session.simulation.visit == before_visit
                    && session.simulation.visited == before_visited
            );
            let boundary = format!(
                "{}/{:?}/{}/{}",
                finished,
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
            if restoring && boundaries.insert(boundary) {
                let saved = client.save(&session)?;
                let before = serde_json::to_value(&saved)?;
                session = Session::restore(
                    &index,
                    serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
                )?;
                client = Client::restore(
                    &session,
                    serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                )?;
                ensure!(
                    serde_json::to_value(client.save(&session)?)? == before,
                    "client checkpoint changed on restore"
                );
                restores += 1;
            }
            if finished {
                done = true;
                break;
            }
        }
        ensure!(done && !session.simulation.vm.world.objects.contains_key(&196));
        let record = session.simulation.vm.world.dialogs.records[0].clone();
        ensure!(record.body.contains("Frankie") && record.portrait.is_some());
        ensure!(session.step()? == Progress::Dialog);
        let saved = client.save(&session)?;
        outcomes.push((
            serde_json::to_value(session.save())?,
            serde_json::to_value(saved)?,
            trace,
            events,
        ));
        if restoring {
            let mut bad = outcomes.last().unwrap().1.clone();
            bad["background"] = serde_json::json!(9999999);
            ensure!(Client::restore(&session, serde_json::from_value(bad)?).is_err());
            std::fs::write(
                output.join("dialog-client-modal.json"),
                serde_json::to_vec(
                    &serde_json::json!({"session":session.save(),"dialog_client":client.save(&session)?,"floor":1,"zoom":0.55,"resident":195,"selected":null}),
                )?,
            )?;
            let mut pixels = client
                .images()
                .get(record.portrait.unwrap().handle)?
                .rgba()?;
            image::imageops::crop(&mut pixels, 0, 0, 45, 45)
                .to_image()
                .save(output.join("dialog-client-portrait.png"))?;
            std::fs::write(
                output.join("dialog-client-source.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"inputs":inputs,"instructions":outcomes.last().unwrap().2.len(),"restores":restores,"same_state_after_restore":true}),
                )?,
            )?;
        }
        println!(
            "client restore={restoring}: {} instructions, {restores} restores; camera={inputs:?}",
            outcomes.last().unwrap().2.len()
        );
    }
    ensure!(
        outcomes[0] == outcomes[1],
        "client owner diverged across execution budgets/restores"
    );
    Ok(())
}
