//! Original-house cursor destruction, including nested furniture deletion.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};

#[test]
#[ignore = "private original-house checkpoints; release/headless"]
fn original_house_cursor_destruction_and_cold_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("destruction test panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_CURSOR_DESTRUCTION_OUTPUT")?);
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
            let discard = if lot == "goth" { grouped } else { !grouped };
            let saved: Saved = serde_json::from_slice(&std::fs::read(root.join(format!(
                "cache/original-cursor-return-frames-2/{lot}-{grouped}-session.json"
            )))?)?;
            let mut finals = vec![];
            let mut runs = vec![];
            for resumed in [false, true] {
                eprintln!("{lot}, grouped={grouped}, resumed={resumed}");
                let mut session = Session::restore(&index, saved.clone())?;
                let focus = session
                    .host
                    .presentation
                    .object_focus
                    .context("cursor focus absent")?;
                let cursor = session
                    .simulation
                    .vm
                    .world
                    .scheduler_storage
                    .as_ref()
                    .unwrap()
                    .object_for_handle(focus)
                    .unwrap();
                let chain = sims_vm::multitile::chain(&session.simulation.vm.world, cursor)
                    .map_err(anyhow::Error::msg)?;
                let targets = chain
                    .iter()
                    .map(|id| {
                        let h = session.simulation.vm.world.objects[id]
                            .cursor
                            .as_ref()
                            .unwrap()
                            .underlying
                            .unwrap();
                        let id = session
                            .simulation
                            .vm
                            .world
                            .scheduler_storage
                            .as_ref()
                            .unwrap()
                            .object_for_handle(h)
                            .unwrap();
                        (id, session.simulation.vm.world.positions[&id])
                    })
                    .collect::<Vec<_>>();
                let tick = session.simulation.vm.tick;
                let funds = session.simulation.vm.world.budget.clone();
                let rng = session.simulation.vm.rng_state;
                //42191c is the actual retain/forget prelude: it clears focus and
                //held references without manufacturing a focus clear in440eeb.
                if !discard {
                    session
                        .simulation
                        .begin_cursor_cleanup(
                            &session.program,
                            cursor,
                            sims_sim::lot_cursor::cleanup::Kind::Forget,
                        )
                        .map_err(anyhow::Error::msg)?;
                    session
                        .simulation
                        .advance_cursor_cleanup(&session.program, 1000)
                        .map_err(anyhow::Error::msg)?;
                    session.service_cursor_cleanup_focus()?;
                    session
                        .simulation
                        .finish_cursor_cleanup(&session.program)
                        .map_err(anyhow::Error::msg)?;
                } else {
                    //Exercise440eeb's failed-release branch explicitly.
                    for id in &chain {
                        session
                            .simulation
                            .vm
                            .world
                            .objects
                            .get_mut(id)
                            .unwrap()
                            .cursor
                            .as_mut()
                            .unwrap()
                            .valid = 0;
                    }
                }
                session
                    .simulation
                    .begin_synchronous_removal(&session.program, cursor)
                    .map_err(anyhow::Error::msg)?;
                let mut restores = 0;
                let mut instructions = 0;
                let mut callback_restore = false;
                let mut retired_restore = false;
                let mut focus_restore = false;
                for n in 0..30000 {
                    let mut remaining = if resumed { 1 } else { 1000 };
                    let mut step = sims_sim::Step {
                        stop: sims_sim::StepStop::Budget,
                        trace: vec![],
                        events: vec![],
                    };
                    let complete = session
                        .simulation
                        .drive_synchronous_removal(&session.program, &mut remaining, &mut step)
                        .map_err(anyhow::Error::msg)?;
                    instructions += step.trace.len();
                    if session.simulation.pending_removal_graphic().is_some() {
                        session.service_removal_graphic(
                            session.host.presentation.camera.context("camera absent")?,
                        )?;
                    }
                    session.service_cursor_appearance()?;
                    session.service_selection_assets(
                        session.host.presentation.camera.context("camera absent")?,
                        1000,
                    )?;
                    session
                        .host
                        .service(&mut session.simulation, &mut session.program)?;
                    session.service_cursor_release_animator()?;
                    if session.simulation.pending_cursor_cleanup_focus() {
                        session.service_cursor_cleanup_focus()?;
                    }
                    let callback = session
                        .simulation
                        .vm
                        .threads
                        .values()
                        .any(|t| t.callback.is_some());
                    let dangling = session.simulation.vm.world.objects.values().any(|o| {
                        o.cursor
                            .as_ref()
                            .and_then(|c| c.underlying)
                            .is_some_and(|h| {
                                session
                                    .simulation
                                    .vm
                                    .world
                                    .scheduler_storage
                                    .as_ref()
                                    .unwrap()
                                    .object_for_handle(h)
                                    .is_none()
                            })
                    });
                    let cleared_focus = session.simulation.cursor_cleanup_focus_cleared();
                    if resumed
                        && (n == 1
                            || callback && !callback_restore
                            || dangling && !retired_restore
                            || cleared_focus && !focus_restore)
                    {
                        ensure!(!complete, "checkpoint must retain a live destruction");
                        let snapshot = session.save();
                        session = Session::restore(
                            &index,
                            serde_json::from_slice(&serde_json::to_vec(&snapshot)?)?,
                        )?;
                        ensure!(
                            serde_json::to_value(session.save())?
                                == serde_json::to_value(snapshot)?
                        );
                        restores += 1;
                        callback_restore |= callback;
                        retired_restore |= dangling;
                        focus_restore |= cleared_focus;
                    }
                    if complete {
                        break;
                    }
                    ensure!(n < 29999, "destruction stalled");
                }
                session
                    .simulation
                    .take_synchronous_removal()
                    .map_err(anyhow::Error::msg)?;
                ensure!(
                    session.simulation.vm.tick == tick
                        && session.simulation.vm.world.budget == funds
                        && session.simulation.vm.rng_state == rng
                );
                ensure!(session.host.presentation.object_focus.is_none());
                for id in &chain {
                    ensure!(!session.simulation.vm.world.objects.contains_key(id));
                }
                for (id, point) in &targets {
                    if discard {
                        ensure!(!session.simulation.vm.world.objects.contains_key(id));
                    } else {
                        ensure!(session.simulation.vm.world.positions[id] == *point);
                    }
                }
                if resumed && discard {
                    ensure!(
                        retired_restore,
                        "discard must cold-resume with deleted furniture references"
                    );
                }
                let image = software::render(Device::restore(&session, camera.clone())?.scene());
                if resumed {
                    image.save(out.join(format!("{lot}-{grouped}.png")))?;
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
                            == image
                    );
                    restores += 1;
                }
                runs.push(serde_json::json!({"resumed":resumed,"full_cold_restores":restores,"callback_restore":callback_restore,"retired_reference_restore":retired_restore,"focus_restore":focus_restore,"instructions":instructions,"tick":tick,"targets":targets}));
                finals.push((serde_json::to_value(session.save())?, image));
            }
            ensure!(finals[0] == finals[1], "direct/resumed destruction differs");
            cases.push(serde_json::json!({"lot":lot,"multipart":grouped,"discard":discard,"runs":runs,"direct_resume_equal":true}));
        }
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"cases":cases,"native_execution":false,"scope":"Four original-house cursor destruction cases through44123f/440eeb. Two use42191c first to preserve furniture and clear focus; two force failed release to exercise nested42186b furniture destruction. Full application save/resume and software pixels. Direct successful-release focus ownership, Buy frontend and normal ticks remain separate."}),
        )?,
    )?;
    Ok(())
}
