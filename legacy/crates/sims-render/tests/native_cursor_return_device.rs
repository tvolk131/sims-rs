//! Finish4215e6 using the real session's shared E2 registry and viewer.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};

#[test]
#[ignore = "private original-house placement checkpoints; release, headless only"]
fn original_house_cursor_focus_notification_return_and_cold_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("cursor return test panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_CURSOR_RETURN_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut cases = vec![];
    for lot in ["goth", "mashuga"] {
        let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
            format!("cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"),
        ))?)?;
        let camera: sims_render::camera_device::Saved =
            serde_json::from_value(wrapper["camera_device"].clone())?;
        for grouped in [false, true] {
            let saved: Saved = serde_json::from_slice(&std::fs::read(root.join(format!(
                "cache/original-cursor-placement-frames/{lot}-{grouped}-session.json"
            )))?)?;
            let mut finals = vec![];
            let mut runs = vec![];
            for resumed in [false, true] {
                eprintln!("{lot}, grouped={grouped}, resumed={resumed}");
                let mut session = Session::restore(&index, saved.clone())?;
                let h = session
                    .simulation
                    .cursor_factory
                    .as_ref()
                    .context("cursor factory absent")?
                    .pending_focus()
                    .context("cursor focus absent")?;
                let id = session
                    .simulation
                    .vm
                    .world
                    .scheduler_storage
                    .as_ref()
                    .unwrap()
                    .object_for_handle(h)
                    .unwrap();
                let underlying = session.simulation.vm.world.objects[&id]
                    .cursor
                    .as_ref()
                    .unwrap()
                    .underlying;
                // Retain the actual saved registry and fields. These house
                // checkpoints have seven listeners, without an Interface;
                // its separate gates are covered by native_house_notifications.
                let ui = session
                    .host
                    .presentation
                    .notifications
                    .as_ref()
                    .context("registry absent")?
                    .dispatch
                    .fields
                    .ui
                    .clone();
                let mut expected_sim = session.simulation.clone();
                expected_sim.cursor_factory = None;
                let registry = session
                    .host
                    .presentation
                    .notifications
                    .as_ref()
                    .unwrap()
                    .dispatch
                    .registry
                    .clone();
                ensure!(
                    registry
                        .nodes
                        .values()
                        .all(|n| n.handler != sims_vm::commander::house::Receiver::Interface)
                );
                let before_pixels =
                    software::render(Device::restore(&session, camera.clone())?.scene());
                let mut calls = 0;
                let mut cold = 0;
                let mut dispatch_steps = 0;
                let returned = loop {
                    calls += 1;
                    ensure!(calls < 100, "cursor return stalled");
                    let started = session
                        .simulation
                        .cursor_factory
                        .as_ref()
                        .and_then(|f| f.pending_created())
                        .is_some_and(|(_, b)| b);
                    let result = session.service_cursor_factory_return(
                        session
                            .host
                            .presentation
                            .camera
                            .context("native camera absent")?,
                        if resumed { 1 } else { 1000 },
                    )?;
                    if started {
                        dispatch_steps += 1;
                    }
                    if calls == 1 {
                        ensure!(session.host.presentation.object_focus == Some(h));
                        ensure!(
                            session
                                .host
                                .presentation
                                .notifications
                                .as_ref()
                                .unwrap()
                                .dispatch
                                .fields
                                .ui
                                .dirty
                                == ui.dirty,
                            "focus must precede E2"
                        );
                        ensure!(
                            session
                                .simulation
                                .cursor_factory
                                .as_ref()
                                .unwrap()
                                .pending_created()
                                == Some((h, false))
                        );
                    }
                    if resumed && cold == 0 && dispatch_steps == 2 {
                        ensure!(session.house_notification_pending());
                        // Reject a live owner with missing focus or a different
                        // E2 payload, without changing the valid continuation.
                        let n = session.host.presentation.notifications.as_ref().unwrap();
                        let mut invalid_view = session.host.presentation.clone();
                        invalid_view.object_focus = None;
                        ensure!(n.validate(&invalid_view, &session.simulation).is_err());
                        let mut invalid = n.clone();
                        invalid.dispatch = sims_vm::commander::house::dispatch::Dispatch::begin(
                            registry.clone(),
                            invalid.dispatch.fields.clone(),
                            0xe2,
                            0,
                        )
                        .map_err(anyhow::Error::msg)?;
                        ensure!(
                            invalid
                                .validate(&session.host.presentation, &session.simulation)
                                .is_err()
                        );
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
                    if let Some(object) = result {
                        break object;
                    }
                };
                ensure!(returned == id && session.host.presentation.object_focus == Some(h));
                ensure!(
                    session.simulation == expected_sim,
                    "E2 on a cursor should not mutate household simulation"
                );
                let n = session.host.presentation.notifications.as_ref().unwrap();
                ensure!(
                    n.complete()
                        && n.dispatch.registry == registry
                        && n.dispatch.root_message() == Some((0xe2, id as u32))
                );
                ensure!(n.dispatch.fields.ui == ui);
                let device = Device::restore(&session, camera.clone())?;
                let pixels = software::render(device.scene());
                ensure!(
                    pixels == before_pixels,
                    "focus/notification unexpectedly changed the software scene"
                );
                if resumed {
                    ensure!(cold == 1);
                    pixels.save(out.join(format!("{lot}-{grouped}-returned.png")))?;
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
                            == pixels
                    );
                    cold += 1;
                }
                finals.push((serde_json::to_value(session.save())?, pixels));
                runs.push(serde_json::json!({"resumed":resumed,"calls":calls,"full_cold_restores":cold,"returned":returned,"underlying_handle":underlying,"tick":session.simulation.vm.tick,"registered_listeners":registry.nodes.len(),"interface_registered":false,"ui_fields_unchanged":true}));
            }
            ensure!(
                finals[0] == finals[1],
                "direct/resumed cursor return differs"
            );
            cases.push(serde_json::json!({"lot":lot,"multipart":grouped,"runs":runs,"direct_resume_equal":true,"factory_complete":true}));
        }
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"native_execution":false,"cases":cases,"scope":"Real4215e6 application tail: view focus, shared E2 listeners, native cursor ID return. No Buy frontend, drop, rotation or normal-tick frontier claim."}),
        )?,
    )?;
    Ok(())
}
