//! Original-house Buy furniture release, with complete application saves.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};

#[path = "support/cursor_checkpoint.rs"]
mod cursor_checkpoint;

#[test]
#[ignore = "private original-house checkpoints; release/headless"]
fn original_house_cursor_release_and_cold_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("release test panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_CURSOR_RELEASE_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut cases = vec![];
    for lot in ["goth", "mashuga"] {
        let camera = cursor_checkpoint::read_camera(&root.join(format!(
            "cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"
        )))?;
        for grouped in [false, true] {
            let saved: Saved = serde_json::from_slice(&std::fs::read(root.join(format!(
                "cache/original-cursor-return-frames-2/{lot}-{grouped}-session.json"
            )))?)?;
            let mut finals = vec![];
            let mut runs = vec![];
            for resumed in [false, true] {
                eprintln!("{lot}, grouped={grouped}, resumed={resumed}");
                let mut session = Session::restore(&index, saved.clone())?;
                let (focus, cursor) = cursor_checkpoint::focused_cursor(&session)?;
                let chain = sims_vm::multitile::chain(&session.simulation.vm.world, cursor)
                    .map_err(anyhow::Error::msg)?;
                let targets = chain
                    .iter()
                    .map(|cursor| {
                        let object = cursor_checkpoint::underlying_object(&session, *cursor);
                        (object, session.simulation.vm.world.positions[&object])
                    })
                    .collect::<Vec<_>>();
                let tick = session.simulation.vm.tick;
                let funds = session.simulation.vm.world.budget.clone();
                let rng = session.simulation.vm.rng_state;
                session
                    .simulation
                    .begin_cursor_release(&session.program, cursor)
                    .map_err(anyhow::Error::msg)?;
                let mut restores = 0;
                let mut instructions = 0;
                let mut callback_restore = false;
                for n in 0..30000 {
                    let step = session
                        .simulation
                        .advance_cursor_release(&session.program, if resumed { 1 } else { 1000 })
                        .map_err(anyhow::Error::msg)?;
                    instructions += step.trace.len();
                    if session.simulation.pending_removal_graphic().is_some() {
                        session.service_removal_graphic(
                            session.host.presentation.camera.context("camera absent")?,
                        )?;
                    }
                    session.service_cursor_release_animator()?;
                    let complete = session
                        .simulation
                        .cursor_release
                        .as_ref()
                        .unwrap()
                        .outcome()
                        .is_some();
                    let callback = session
                        .simulation
                        .vm
                        .threads
                        .values()
                        .any(|t| t.callback.is_some());
                    if resumed && (n == 5 || callback && !callback_restore) {
                        ensure!(!complete, "checkpoint must retain a live release");
                        let snapshot = session.save();
                        session = cursor_checkpoint::restore_json(&index, &snapshot)?;
                        ensure!(
                            serde_json::to_value(session.save())?
                                == serde_json::to_value(snapshot)?
                        );
                        restores += 1;
                        callback_restore |= callback;
                    }
                    if complete {
                        break;
                    }
                    ensure!(n < 29999, "release stalled");
                }
                ensure!(
                    session
                        .simulation
                        .finish_cursor_release(&session.program)
                        .map_err(anyhow::Error::msg)?
                );
                ensure!(
                    session.simulation.vm.tick == tick
                        && session.simulation.vm.world.budget == funds
                        && session.simulation.vm.rng_state == rng
                );
                ensure!(session.host.presentation.object_focus == Some(focus));
                for id in &chain {
                    ensure!(
                        session.simulation.vm.world.objects[id]
                            .cursor
                            .as_ref()
                            .unwrap()
                            .underlying
                            .is_none()
                    );
                    ensure!(session.simulation.vm.world.positions[id].x == -16);
                }
                for (id, point) in &targets {
                    ensure!(session.simulation.vm.world.positions[id] == *point);
                }
                let image = software::render(Device::restore(&session, camera.clone())?.scene());
                if resumed {
                    image.save(out.join(format!("{lot}-{grouped}.png")))?;
                    let snapshot = session.save();
                    std::fs::write(
                        out.join(format!("{lot}-{grouped}-session.json")),
                        serde_json::to_vec(&snapshot)?,
                    )?;
                    session = cursor_checkpoint::restore_json(&index, &snapshot)?;
                    ensure!(
                        serde_json::to_value(session.save())? == serde_json::to_value(snapshot)?
                    );
                    ensure!(
                        software::render(Device::restore(&session, camera.clone())?.scene())
                            == image
                    );
                    restores += 1;
                }
                runs.push(serde_json::json!({"resumed":resumed,"full_cold_restores":restores,"callback_restore":callback_restore,"instructions":instructions,"tick":tick,"targets":targets}));
                finals.push((serde_json::to_value(session.save())?, image));
            }
            ensure!(finals[0] == finals[1], "direct/resumed release differs");
            cases.push(serde_json::json!({"lot":lot,"multipart":grouped,"runs":runs,"direct_resume_equal":true}));
        }
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"cases":cases,"native_execution":false,"scope":"Four original-house furniture releases through421a0e and43562f/422e99. Full application save/resume and software pixels. Cursor destruction, purchase frontend and normal ticks are separate."}),
        )?,
    )?;
    Ok(())
}
