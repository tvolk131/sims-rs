//! Actual cursor rotation in imported houses, with complete application saves.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};

#[test]
#[ignore = "private original-house cursor-return checkpoints; release/headless"]
fn original_house_cursor_rotations_and_cold_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("rotation test panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_CURSOR_ROTATION_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut cases = vec![];
    for lot in ["goth", "mashuga"] {
        let camera_wrapper: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(format!(
                "cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"
            )))?)?;
        let camera: sims_render::camera_device::Saved =
            serde_json::from_value(camera_wrapper["camera_device"].clone())?;
        for grouped in [false, true] {
            let saved: Saved = serde_json::from_slice(&std::fs::read(root.join(format!(
                "cache/original-cursor-return-frames-2/{lot}-{grouped}-session.json"
            )))?)?;
            let mut finals = vec![];
            let mut runs = vec![];
            for resumed in [false, true] {
                eprintln!("{lot}, grouped={grouped}, resumed={resumed}");
                let mut session = Session::restore(&index, saved.clone())?;
                let h = session
                    .host
                    .presentation
                    .object_focus
                    .context("cursor focus absent")?;
                let object = session
                    .simulation
                    .vm
                    .world
                    .scheduler_storage
                    .as_ref()
                    .unwrap()
                    .object_for_handle(h)
                    .unwrap();
                let tick = session.simulation.vm.tick;
                let funds = session.simulation.vm.world.budget.clone();
                let before_positions = session.simulation.vm.world.positions.clone();
                let mut cold = 0;
                let mut instructions = 0;
                let mut poses = vec![];
                let mut pixels = vec![];
                for notches in [1, 0, -1] {
                    session
                        .simulation
                        .begin_cursor_rotation(&session.program, object, notches, false)
                        .map_err(anyhow::Error::msg)?;
                    for n in 0..20000 {
                        let step = session
                            .simulation
                            .advance_cursor_rotation(
                                &session.program,
                                if resumed { 1 } else { 1000 },
                            )
                            .map_err(anyhow::Error::msg)?;
                        instructions += step.trace.len();
                        if session.simulation.pending_removal_graphic().is_some() {
                            session.service_removal_graphic(
                                session.host.presentation.camera.context("camera absent")?,
                            )?;
                        }
                        session.service_cursor_appearance()?;
                        let complete = session
                            .simulation
                            .cursor_rotation
                            .as_ref()
                            .unwrap()
                            .complete();
                        if resumed && cold == 0 && n == 5 {
                            ensure!(!complete, "cold checkpoint must retain a live rotation");
                            let snapshot = session.save();
                            session = Session::restore(
                                &index,
                                serde_json::from_slice(&serde_json::to_vec(&snapshot)?)?,
                            )?;
                            ensure!(
                                serde_json::to_value(session.save())?
                                    == serde_json::to_value(snapshot)?
                            );
                            cold += 1;
                        }
                        if complete {
                            break;
                        }
                        ensure!(n < 19999, "rotation stalled");
                    }
                    session
                        .simulation
                        .finish_cursor_rotation(&session.program)
                        .map_err(anyhow::Error::msg)?;
                    ensure!(session.host.presentation.object_focus == Some(h));
                    let image =
                        software::render(Device::restore(&session, camera.clone())?.scene());
                    if resumed {
                        image.save(out.join(format!("{lot}-{grouped}-notches-{notches}.png")))?;
                    }
                    pixels.push(image);
                    let chain = sims_vm::multitile::chain(&session.simulation.vm.world, object)
                        .map_err(anyhow::Error::msg)?;
                    let mut parts = vec![];
                    for id in chain {
                        let o = &session.simulation.vm.world.objects[&id];
                        let c = o.cursor.as_ref().unwrap();
                        let target = session
                            .simulation
                            .vm
                            .world
                            .scheduler_storage
                            .as_ref()
                            .unwrap()
                            .object_for_handle(c.underlying.unwrap())
                            .unwrap();
                        parts.push(serde_json::json!({"cursor":id,"furniture":target,"direction":session.simulation.vm.world.objects[&target].data[&1],"position":session.simulation.vm.world.positions[&target],"offset":o.multitile.as_ref().unwrap().offset,"valid":c.valid}));
                    }
                    poses.push(serde_json::json!({"notches":notches,"parts":parts}));
                }
                ensure!(
                    session.simulation.vm.tick == tick
                        && session.simulation.vm.world.budget == funds
                );
                ensure!(
                    session.simulation.vm.world.positions == before_positions,
                    "inverse rotation did not restore original points"
                );
                if resumed {
                    ensure!(cold == 1);
                    let snapshot = session.save();
                    std::fs::write(
                        out.join(format!("{lot}-{grouped}-session.json")),
                        serde_json::to_vec(&snapshot)?,
                    )?;
                    session = Session::restore(
                        &index,
                        serde_json::from_slice(&serde_json::to_vec(&snapshot)?)?,
                    )?;
                    ensure!(
                        serde_json::to_value(session.save())? == serde_json::to_value(snapshot)?
                    );
                    ensure!(
                        software::render(Device::restore(&session, camera.clone())?.scene())
                            == *pixels.last().unwrap()
                    );
                    cold += 1;
                }
                runs.push(serde_json::json!({"resumed":resumed,"full_cold_restores":cold,"instructions":instructions,"poses":poses,"tick":tick}));
                finals.push((serde_json::to_value(session.save())?, pixels));
            }
            ensure!(finals[0] == finals[1], "direct/resumed rotation differs");
            cases.push(serde_json::json!({"lot":lot,"multipart":grouped,"runs":runs,"direct_resume_equal":true,"inverse_positions_equal":true}));
        }
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"cases":cases,"native_execution":false,"scope":"Native cursor notch calls 1,0,-1 in four original-house furniture cases. Full application cold resume and software pixels; no Buy frontend/drop/normal tick advancement claim."}),
        )?,
    )?;
    Ok(())
}
