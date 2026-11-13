//! Real stock-house Buy camera transitions with held furniture and cold resume.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{
    Session,
    camera_command::{Change, Progress},
};
use sims_sim::lot_presenter::{camera, transition};
use std::{collections::BTreeSet, path::Path};

#[test]
#[ignore = "private stock household; release/headless"]
fn buy_camera_preserves_held_furniture_and_resumes_native_callback() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("Buy camera test panicked")
}
fn snapshot(s: &Session, d: &Device) -> Result<Value> {
    Ok(json!({"session":s.save(),"camera_device":d.save(s)?}))
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_BUY_CAMERA_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let source: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-move-pickup-iced-4/held-cancel.json"),
    )?)?;
    let mut finals = vec![];
    let mut reports = vec![];
    for resumed in [false, true] {
        eprintln!("Buy camera resumed={resumed}");
        let mut s = Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
        let mut d = Device::restore(&s, serde_json::from_value(source["camera_device"].clone())?)?;
        let tick = s.simulation.vm.tick;
        let budget = s.simulation.vm.world.budget.clone();
        let original_move = s.host.presentation.move_tool.as_ref().unwrap().clone();
        ensure!(original_move.tool.held.is_some() && original_move.tool.tool.position != [0, 0]);
        let mut expected_move = original_move;
        expected_move.tool.tool.position = [0, 0];
        let initial_rotation = d.controls().rotation;
        let mut cold = BTreeSet::new();
        let mut frames = vec![];
        let mut events = vec![];
        for (step, change) in [
            Change::Rotation((initial_rotation + 1) & 3),
            Change::Rotation(initial_rotation),
            Change::ZoomIn,
            Change::ZoomOut,
            Change::Floor(2),
            Change::Floor(1),
        ]
        .into_iter()
        .enumerate()
        {
            d.begin(&mut s, change)?;
            let mut done = false;
            for slice in 0..50000 {
                ensure!(
                    s.begin_clock_frame().is_err(),
                    "camera allowed clock execution"
                );
                let progress = s.advance_camera_command(s.retained_camera_input()?, 1)?;
                let boundary = match &progress {
                    Progress::Camera(camera::Service::Transition(
                        transition::Request::RefreshUi,
                    )) => Some("definition-graphics"),
                    Progress::RefreshControls => Some("selected-tool"),
                    _ => None,
                };
                if let Some(name) = boundary {
                    events.push(json!({"step":step,"boundary":name}));
                    if resumed && cold.insert(name.to_owned()) {
                        let before = snapshot(&s, &d)?;
                        std::fs::write(
                            out.join(format!("{name}.json")),
                            serde_json::to_vec(&before)?,
                        )?;
                        s = Session::restore(
                            &index,
                            serde_json::from_value(before["session"].clone())?,
                        )?;
                        d = Device::restore(
                            &s,
                            serde_json::from_value(before["camera_device"].clone())?,
                        )?;
                        ensure!(before == snapshot(&s, &d)?, "cold restore changed {name}");
                    }
                }
                if progress == Progress::Pending {
                    continue;
                }
                match d.advance(&mut s, if resumed { 1 } else { 256 }) {
                    Ok(true) => {
                        done = true;
                        break;
                    }
                    Ok(false) => (),
                    Err(error) => {
                        std::fs::write(
                            out.join(format!("error-{resumed}-{step}.json")),
                            serde_json::to_vec(&snapshot(&s, &d)?)?,
                        )?;
                        return Err(error).with_context(|| format!("{change:?} slice{slice}"));
                    }
                }
            }
            ensure!(done, "camera stalled {change:?}");
            ensure!(s.simulation.vm.tick == tick && s.simulation.vm.world.budget == budget);
            ensure!(
                s.host.presentation.move_tool.as_ref() == Some(&expected_move),
                "camera changed held furniture beyond grab offset"
            );
            let pixels = software::render(&d.scene());
            if !resumed && [0, 4, 5].contains(&step) {
                pixels.save(out.join(format!("view-{step}.png")))?;
            }
            frames.push(pixels);
        }
        let final_state = snapshot(&s, &d)?;
        if resumed {
            ensure!(cold.len() == 2, "missed camera restore boundaries");
            std::fs::write(out.join("session.json"), serde_json::to_vec(&final_state)?)?;
        }
        reports.push(json!({"resumed":resumed,"cold_restores":cold,"events":events,"tick":tick,"held_preserved":true}));
        finals.push((final_state, frames));
    }
    ensure!(
        finals[0] == finals[1],
        "budget/resume changed complete state or pixels"
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"cases":reports,"full_state_and_pixels_equal":true,"actual_iced_input":false}),
        )?,
    )?;
    Ok(())
}
