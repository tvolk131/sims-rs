//! Continue the actual Goth undo checkpoints through native placement and
//! deactivation. Original snapshots and source files remain read-only.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session, move_tool};
use std::collections::BTreeSet;

#[test]
#[ignore = "private original-house checkpoints; release/headless"]
fn original_house_undo_places_furniture_and_finishes_deactivation_across_cold_saves() -> Result<()>
{
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("undo placement panicked")
}
fn cold(index: &Index, s: &mut Session) -> Result<()> {
    let before = s.save();
    *s = Session::restore(
        index,
        serde_json::from_slice(&serde_json::to_vec(&before)?)?,
    )?;
    ensure!(serde_json::to_value(s.save())? == serde_json::to_value(before)?);
    Ok(())
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_UNDO_PLACEMENT_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-ui-timer-dispatch-iced/goth/resumed.json"),
    )?)?;
    let camera: sims_render::camera_device::Saved =
        serde_json::from_value(wrapper["camera_device"].clone())?;
    let mut cases = vec![];
    for grouped in [false, true] {
        let saved: Saved = serde_json::from_slice(&std::fs::read(root.join(format!(
            "cache/original-move-undo-frames-1/goth-{grouped}-session.json"
        )))?)?;
        let mut finals = vec![];
        let mut runs = vec![];
        for resumed in [false, true] {
            eprintln!("undo placement grouped={grouped} resumed={resumed}");
            let mut s = Session::restore(&index, saved.clone())?;
            //These older checkpoints stopped before the application budget
            //service. Supply its fixture owner explicitly; loading a real
            //house must never silently invent an application owner.
            let budgets = &mut s
                .host
                .presentation
                .move_tool
                .as_mut()
                .unwrap()
                .history
                .budgets
                .0;
            ensure!(budgets.is_empty());
            budgets.insert(
                8001,
                sims_sim::edit_history::object::budget::State {
                    pending: 0,
                    category: 0,
                },
            );
            let target = if grouped { 173 } else { 198 };
            let snapshot = s
                .host
                .presentation
                .move_tool
                .as_ref()
                .context("move tool absent")?
                .history
                .objects
                .get(7001)
                .map_err(anyhow::Error::msg)?
                .snapshots[0]
                .clone();
            let tick = s.simulation.vm.tick;
            let rng = s.simulation.vm.rng_state;
            let funds = s.simulation.vm.world.budget.clone();
            let mut device = Device::restore(&s, camera.clone())?;
            let mut restored = BTreeSet::new();
            let mut instructions = 0;
            let mut notices = vec![];
            let mut iterations = 0;
            loop {
                iterations += 1;
                ensure!(iterations < 10000, "deactivation stalled");
                let a = s.advance_move_tool(if resumed { 1 } else { 1000 })?;
                instructions += a.instructions;
                if resumed {
                    let mut boundary = None;
                    if let Some(e) = &s.simulation.object_placement {
                        boundary = Some(format!("{:?}", e.caller().service));
                        if s.simulation
                            .vm
                            .threads
                            .values()
                            .any(|t| t.callback.is_some())
                        {
                            boundary = Some("placement callback".into());
                        }
                        if !restored.contains(boundary.as_ref().unwrap()) {
                            let mut broken = serde_json::to_value(s.save())?;
                            broken["view"]["move_tool"]["call"]["child"]["History"]["placement"] =
                                serde_json::Value::Null;
                            ensure!(
                                Session::restore(&index, serde_json::from_value(broken)?).is_err(),
                                "orphan placement accepted"
                            );
                        }
                    } else if s.host.presentation.object_budget.is_some() {
                        boundary = Some("budget".into());
                    } else if s
                        .host
                        .presentation
                        .move_tool
                        .as_ref()
                        .is_some_and(|m| m.complete())
                    {
                        boundary = Some("complete".into());
                    }
                    if let Some(b) = boundary.filter(|b| !restored.contains(b)) {
                        eprintln!("cold restore at {b}");
                        cold(&index, &mut s)?;
                        restored.insert(b);
                    }
                }
                match a.progress {
                    move_tool::Progress::Pending => (),
                    move_tool::Progress::Complete => break,
                    move_tool::Progress::Service(move_tool::Service::Notification(r)) => {
                        notices.push(serde_json::to_value(r)?);
                        device.advance_notification(&mut s, 1)?;
                    }
                    other => anyhow::bail!("unbound undo continuation: {other:?}"),
                }
            }
            ensure!(
                s.simulation.object_placement.is_none()
                    && s.host.presentation.object_budget.is_none()
            );
            let q = &s.simulation.vm.world.positions[&target];
            ensure!(
                [q.x, q.y] == snapshot.position && q.level == snapshot.level.unwrap(),
                "undo failed to restore original furniture"
            );
            ensure!(
                s.simulation.vm.tick == tick
                    && s.simulation.vm.rng_state == rng
                    && s.simulation.vm.world.budget == funds
            );
            let tool = s.host.presentation.move_tool.as_ref().unwrap();
            ensure!(tool.tool.held.is_none() && tool.tool.payload.is_none() && tool.complete());
            s.finish_move_tool_cleanup()?;
            ensure!(!s.player_input_pending());
            let image = software::render(Device::restore(&s, camera.clone())?.scene());
            if resumed {
                ensure!(
                    restored.iter().any(|b| b.contains("Check"))
                        && restored.iter().any(|b| b.contains("Place("))
                        && restored.contains("budget")
                );
                cold(&index, &mut s)?;
                ensure!(software::render(Device::restore(&s, camera.clone())?.scene()) == image);
                image.save(out.join(format!("goth-{grouped}.png")))?;
                std::fs::write(
                    out.join(format!("goth-{grouped}-session.json")),
                    serde_json::to_vec(&s.save())?,
                )?;
            }
            runs.push(serde_json::json!({"resumed":resumed,"restored_boundaries":restored,"full_cold_restores":restored.len()+usize::from(resumed),"instructions":instructions,"tick":tick,"iterations":iterations,"notices":notices}));
            finals.push((serde_json::to_value(s.save())?, image, notices));
        }
        ensure!(
            finals[0] == finals[1],
            "live undo differs across cold resume"
        );
        cases.push(serde_json::json!({"grouped":grouped,"runs":runs,"equal":true}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"cases":cases,"native_execution":false,"scope":"Actual Goth chair/sofa undo, live placement, callbacks, budget and complete deactivation; no normal tick advancement claimed."}),
        )?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private original-house checkpoint; release/headless"]
fn original_history_budget_waits_for_interface_and_cold_resumes_its_exact_caller() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run_budget)?
        .join()
        .expect("history budget panicked")
}
fn run_budget() -> Result<()> {
    use sims_vm::commander::house::Receiver;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_UNDO_PLACEMENT_BUDGET_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let saved: Saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-move-undo-frames-1/goth-false-session.json"),
    )?)?;
    let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-ui-timer-dispatch-iced/goth/resumed.json"),
    )?)?;
    let camera: sims_render::camera_device::Saved =
        serde_json::from_value(wrapper["camera_device"].clone())?;
    let mut finals = vec![];
    let mut runs = vec![];
    for resumed in [false, true] {
        eprintln!("history budget resumed={resumed}");
        let mut s = Session::restore(&index, saved.clone())?;
        s.host
            .presentation
            .move_tool
            .as_mut()
            .unwrap()
            .history
            .budgets
            .0
            .insert(
                8001,
                sims_sim::edit_history::object::budget::State {
                    pending: 0,
                    category: 0,
                },
            );
        //The old checkpoint predates Interface registration. Append the real
        //replacement UI through its registry operation, retaining all existing
        //listener order; this is explicit fixture setup, not a load default.
        let context = &s
            .host
            .presentation
            .notifications
            .as_ref()
            .context("listeners absent")?
            .dispatch;
        let mut registry = context.registry.clone();
        ensure!(
            !registry
                .nodes
                .values()
                .any(|n| n.handler == Receiver::Interface)
        );
        let identity = registry
            .nodes
            .keys()
            .copied()
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .context("listener overflow")?;
        registry.construct(identity).map_err(anyhow::Error::msg)?;
        registry
            .bind(identity, Receiver::Interface)
            .map_err(anyhow::Error::msg)?;
        let mut fields = context.fields.clone();
        fields.ui.mask_owner_present = true;
        ensure!(
            fields.ui.gated == 0
                && fields.ui.creation_gated == 0
                && s.host.presentation.transport.is_some()
        );
        s.bind_house_listeners(registry, fields)?;
        let mut device = Device::restore(&s, camera.clone())?;
        let initial = s.simulation.clone();
        let mut notices = vec![];
        let mut restored = false;
        let mut complete = false;
        for _ in 0..1000 {
            let a = s.advance_move_tool(if resumed { 1 } else { 1000 })?;
            match a.progress {
                move_tool::Progress::Pending => (),
                move_tool::Progress::Complete => {
                    complete = true;
                    break;
                }
                move_tool::Progress::Service(move_tool::Service::Notification(r)) => {
                    let budget_owned = s.host.presentation.object_budget.is_some();
                    if budget_owned && resumed && !restored {
                        let mut bad = serde_json::to_value(s.save())?;
                        bad["view"]["move_tool"]["call"]["child"]["History"]["budget"] =
                            serde_json::Value::Null;
                        ensure!(
                            Session::restore(&index, serde_json::from_value(bad)?).is_err(),
                            "orphan budget accepted"
                        );
                        let device_saved = device.save(&s)?;
                        cold(&index, &mut s)?;
                        device = Device::restore(&s, device_saved)?;
                        restored = true;
                    }
                    notices.push(serde_json::json!({"budget_owned":budget_owned,"request":r}));
                    device.advance_notification(&mut s, 1)?;
                }
                other => anyhow::bail!("unexpected budget continuation: {other:?}"),
            }
        }
        ensure!(complete && (!resumed || restored));
        ensure!(
            notices.iter().any(|n| n["budget_owned"] == true),
            "UI budget service was not exercised"
        );
        ensure!(
            s.simulation.vm.tick == initial.vm.tick
                && s.simulation.vm.rng_state == initial.vm.rng_state
                && s.simulation.vm.world.budget == initial.vm.world.budget
        );
        s.finish_move_tool_cleanup()?;
        ensure!(!s.player_input_pending() && !s.house_notification_pending());
        let frozen_before_publication = serde_json::to_value(device.save(&s)?)?
            .get("frozen_pixels")
            .is_some();
        //A suspended device restores its last published pixels. Complete the
        //normal presentation step on both paths before comparing devices.
        device.refresh(&s)?;
        std::fs::write(
            out.join(format!("goth-{resumed}-session.json")),
            serde_json::to_vec(&s.save())?,
        )?;
        std::fs::write(
            out.join(format!("goth-{resumed}-device.json")),
            serde_json::to_vec(&device.save(&s)?)?,
        )?;
        runs.push(serde_json::json!({"resumed":resumed,"full_cold_restores":usize::from(restored),"notices":notices,"frozen_before_publication":frozen_before_publication}));
        finals.push((
            serde_json::to_value(s.save())?,
            serde_json::to_value(device.save(&s)?)?,
            software::render(device.scene()),
        ));
    }
    ensure!(
        finals[0].0 == finals[1].0,
        "history/UI budget session differs across cold resume"
    );
    ensure!(
        finals[0].1 == finals[1].1,
        "history/UI budget device differs across cold resume"
    );
    ensure!(
        finals[0].2 == finals[1].2,
        "history/UI budget pixels differ across cold resume"
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"equal":true,"runs":runs,"native_execution":false,"scope":"Exact nested history budget caller with actual Interface registration and headless UI replies, including full Session/Device cold restore while its reply is pending."}),
        )?,
    )?;
    Ok(())
}
