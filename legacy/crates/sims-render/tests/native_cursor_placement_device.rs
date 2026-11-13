//! Continue the original-house cursor snapshots through native placement.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};

#[test]
#[ignore = "private original-house cursor checkpoints; release, headless only"]
fn original_house_cursor_placement_and_cold_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("placement test panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_CURSOR_PLACEMENT_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut reports = vec![];
    for lot in ["goth", "mashuga"] {
        let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
            format!("cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"),
        ))?)?;
        let camera: sims_render::camera_device::Saved =
            serde_json::from_value(wrapper["camera_device"].clone())?;
        for grouped in [false, true] {
            let saved: Saved = serde_json::from_slice(&std::fs::read(root.join(format!(
                "cache/original-cursor-live-frames/{lot}-{grouped}-session.json"
            )))?)?;
            let mut finals = vec![];
            let mut runs = vec![];
            for resumed in [false, true] {
                eprintln!("{lot}, grouped={grouped}, resumed={resumed}");
                let mut session = Session::restore(&index, saved.clone())?;
                let tick = session.simulation.vm.tick;
                let budget = session.simulation.vm.world.budget.clone();
                let f = session
                    .simulation
                    .cursor_factory
                    .as_ref()
                    .context("cursor factory absent")?;
                let input = f.placement().context("factory placement input absent")?;
                let handles = f.created().to_vec();
                session
                    .simulation
                    .begin_cursor_factory_placement(&session.program, false)
                    .map_err(anyhow::Error::msg)?;
                let mut cold = 0;
                let mut instructions = 0;
                let mut slices = 0;
                for n in 0..10000 {
                    let step = session
                        .simulation
                        .advance_cursor_factory(&session.program, if resumed { 1 } else { 1000 })
                        .map_err(anyhow::Error::msg)?;
                    instructions += step.trace.len();
                    slices += 1;
                    if session.simulation.pending_removal_graphic().is_some() {
                        session.service_removal_graphic(
                            session.host.presentation.camera.context("camera absent")?,
                        )?;
                    }
                    session.service_cursor_appearance()?;
                    let f = session.simulation.cursor_factory.as_ref().unwrap();
                    ensure!(
                        f.placement_request().is_none(),
                        "unbound cursor service: {:?}",
                        f.placement_request()
                    );
                    let focus = f.pending_focus().is_some();
                    let moved = handles.iter().any(|h| {
                        let id = session
                            .simulation
                            .vm
                            .world
                            .scheduler_storage
                            .as_ref()
                            .unwrap()
                            .object_for_handle(*h)
                            .unwrap();
                        let c = session.simulation.vm.world.objects[&id]
                            .cursor
                            .as_ref()
                            .unwrap();
                        let underlying = session
                            .simulation
                            .vm
                            .world
                            .scheduler_storage
                            .as_ref()
                            .unwrap()
                            .object_for_handle(c.underlying.unwrap())
                            .unwrap();
                        session.simulation.vm.world.positions[&underlying].x >= 0
                            && session.simulation.vm.world.positions[&id].x < 0
                    });
                    if resumed
                        && cold == 0
                        && (moved
                            || session
                                .simulation
                                .vm
                                .threads
                                .values()
                                .any(|t| t.callback.is_some()))
                    {
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
                    if focus {
                        break;
                    }
                    ensure!(n < 9999, "cursor placement stalled");
                }
                ensure!(
                    session
                        .simulation
                        .cursor_factory
                        .as_ref()
                        .unwrap()
                        .pending_focus()
                        == Some(input.cursor)
                );
                let mut positions = vec![];
                for h in &handles {
                    let w = &session.simulation.vm.world;
                    let id = w
                        .scheduler_storage
                        .as_ref()
                        .unwrap()
                        .object_for_handle(*h)
                        .unwrap();
                    let o = &w.objects[&id];
                    let c = o.cursor.as_ref().unwrap();
                    let underlying = w
                        .scheduler_storage
                        .as_ref()
                        .unwrap()
                        .object_for_handle(c.underlying.unwrap())
                        .unwrap();
                    let q = w.positions[&underlying];
                    ensure!(
                        q.x >= 0 && c.position == [q.x, q.y] && c.level == q.level && c.valid == 1,
                        "furniture/cursor differs: lot={lot}, grouped={grouped}, cursor={c:?}, physical={q:?}, diagnostics={:?}",
                        session.simulation.native_lot.as_ref().unwrap().diagnostics
                    );
                    ensure!(o.native_flags.unwrap() & 0x80 != 0);
                    positions.push(serde_json::json!({"cursor":id,"furniture":underlying,"point":[q.x,q.y,q.level]}));
                }
                ensure!(
                    session.simulation.vm.tick == tick
                        && session.simulation.vm.world.budget == budget
                );
                let mut device = Device::restore(&session, camera.clone())?;
                device.refresh(&session)?;
                let pixels = software::render(device.scene());
                if resumed {
                    ensure!(cold == 1, "no in-flight cold restoration");
                    pixels.save(out.join(format!("{lot}-{grouped}-placed.png")))?;
                    let snapshot = session.save();
                    std::fs::write(
                        out.join(format!("{lot}-{grouped}-session.json")),
                        serde_json::to_vec(&snapshot)?,
                    )?;
                    let camera = device.save(&session)?;
                    session = Session::restore(
                        &index,
                        serde_json::from_slice(&serde_json::to_vec(&snapshot)?)?,
                    )?;
                    ensure!(
                        serde_json::to_value(session.save())? == serde_json::to_value(snapshot)?
                    );
                    device = Device::restore(&session, camera)?;
                    ensure!(software::render(device.scene()) == pixels);
                    cold += 1;
                }
                finals.push((serde_json::to_value(session.save())?, pixels));
                runs.push(serde_json::json!({"resumed":resumed,"instructions":instructions,"slices":slices,"full_cold_restores":cold,"positions":positions,"tick":tick}));
            }
            ensure!(
                finals[0] == finals[1],
                "direct/resumed state or pixels differ"
            );
            reports.push(serde_json::json!({"lot":lot,"multipart":grouped,"runs":runs,"direct_resume_equal":true,"focus_and_e2_pending":true}));
        }
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"native_execution":false,"cases":reports,"scope":"Cursor and represented furniture placement through flag80; focus/E2 remain pending. Original bytecode runs if a placement callback is invoked. No normal-play tick frontier claim."}),
        )?,
    )?;
    Ok(())
}
