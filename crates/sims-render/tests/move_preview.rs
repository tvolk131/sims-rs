//!486f5a composed with real factories, orientation, ObjectUndoable and hover.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::Session;
use std::collections::BTreeSet;

#[test]
#[ignore = "private stock Goth Buy checkpoint; release/headless"]
fn original_buy_preview_creates_orients_and_captures_undo_across_cold_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("Move preview panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_MOVE_PREVIEW_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let source: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-buy-product-frames-1/session.json"),
    )?)?;
    let mut reports = vec![];
    for name in ["chair", "sofa"] {
        let mut finals = vec![];
        let mut runs = vec![];
        for resumed in [false, true] {
            eprintln!("Move preview {name}, resumed={resumed}");
            let mut s =
                Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
            let mut d =
                Device::restore(&s, serde_json::from_value(source["camera_device"].clone())?)?;
            if name == "sofa" {
                use sims_vm::interface::buy::input::Event;
                let c = d.panel_widgets().unwrap().catalog().unwrap();
                ensure!(d.begin_buy_navigation(&mut s, c.revision(), Event::Category(2))?);
                finish_selection(&mut s, &mut d)?;
                let c = d.panel_widgets().unwrap().catalog().unwrap();
                let sofa = c
                    .view()
                    .unwrap()
                    .products
                    .iter()
                    .copied()
                    .find(|&i| {
                        let p = c.product(i).unwrap();
                        p.source.source.ends_with("Sofas.iff") && p.fields.subindex == -1
                    })
                    .context("original sofa absent from visible page")?;
                ensure!(d.begin_buy_product(&mut s, c.revision(), sofa)?);
                finish_selection(&mut s, &mut d)?;
            }
            let tick = s.simulation.vm.tick;
            let funds = s.simulation.vm.world.budget.clone();
            let tool = s
                .host
                .presentation
                .move_tool
                .as_ref()
                .context("Move absent")?;
            let selected = tool.tool.tool.definition.context("definition absent")?;
            let definition = tool.history.definitions[&selected].clone();
            let direction = sims_vm::cursor::orientation::camera_facing(
                s.retained_camera_input()?.rotation as i32,
            );
            let mut cold = BTreeSet::new();
            let mut slices = 0;
            s.begin_move_tool_preview()?;
            for n in 0..20000 {
                slices += 1;
                ensure!(s.begin_clock_frame().is_err());
                match d.advance_move_tool(&mut s, if resumed { 1 } else { 256 }) {
                    Ok(true) => break,
                    Ok(false) => (),
                    Err(e) => {
                        std::fs::write(
                            out.join(format!("{name}-error.json")),
                            serde_json::to_vec(
                                &json!({"session":s.save(),"camera_device":d.save(&s)?}),
                            )?,
                        )?;
                        return Err(e).with_context(|| format!("{name} slice{n}"));
                    }
                }
                let p = serde_json::to_value(s.host.presentation.move_tool.as_ref().unwrap())?;
                let phase = p["call"]["preview"]["phase"]
                    .as_str()
                    .context("preview phase absent")?;
                let boundary = if s.house_notification_pending() {
                    format!("{phase}-notice")
                } else {
                    phase.to_owned()
                };
                let save_here = if name == "chair" {
                    matches!(boundary.as_str(), "Purchase-notice" | "Orient" | "Offsets")
                } else {
                    matches!(boundary.as_str(), "Orient" | "Hover")
                };
                if resumed && save_here && cold.insert(boundary.clone()) {
                    let saved = json!({"session":s.save(),"camera_device":d.save(&s)?});
                    std::fs::write(
                        out.join(format!("{name}-{boundary}.json")),
                        serde_json::to_vec(&saved)?,
                    )?;
                    s = Session::restore(
                        &index,
                        serde_json::from_value(saved["session"].clone())?,
                    )?;
                    d = Device::restore(
                        &s,
                        serde_json::from_value(saved["camera_device"].clone())?,
                    )?;
                    ensure!(
                        json!({"session":s.save(),"camera_device":d.save(&s)?}) == saved,
                        "cold restore changed{boundary}"
                    );
                }
                ensure!(n < 19999, "Move preview stalled");
            }
            ensure!(!s.move_tool_pending() && !s.purchase_cursor_pending());
            ensure!(s.simulation.vm.tick == tick && s.simulation.vm.world.budget == funds);
            let tool = s.host.presentation.move_tool.as_ref().unwrap();
            ensure!(tool.tool.tool.definition == Some(selected));
            let world = &s.simulation.vm.world;
            let registry = world.scheduler_storage.as_ref().unwrap();
            let cursor = registry
                .object_for_handle(tool.tool.held.context("held cursor absent")?)
                .context("held cursor lifetime absent")?;
            let underlying = registry
                .object_for_handle(
                    world.objects[&cursor]
                        .cursor
                        .as_ref()
                        .unwrap()
                        .underlying
                        .context("held furniture absent")?,
                )
                .unwrap();
            let cost = i32::from(
                sims_vm::construction::definition_words(&s.program, &definition)
                    .map_err(anyhow::Error::msg)?[18],
            );
            ensure!(tool.tool.value90 == cost as u32 && tool.tool.hover == underlying);
            ensure!(
                world.objects[&underlying]
                    .native_flags
                    .context("hover flags absent")?
                    & 1
                    != 0
            );
            let payload = tool.tool.payload.context("undo command absent")?;
            let command = tool
                .history
                .objects
                .get(payload)
                .map_err(anyhow::Error::msg)?;
            ensure!(
                command.kind == 0 && command.cost == Some(cost) && command.budget == Some(7000)
            );
            ensure!(
                tool.history
                    .store
                    .command(payload)
                    .map_err(anyhow::Error::msg)?
                    .references
                    == 1
            );
            ensure!(
                tool.history
                    .references
                    .object(&command.snapshots[0])
                    .map_err(anyhow::Error::msg)?
                    .is_some()
            );
            ensure!(
                tool.history
                    .store
                    .vector(sims_sim::edit_history::List::Undo(tool.tool.clipboard))
                    .map_err(anyhow::Error::msg)?
                    .len
                    == 0
            );
            let mut parts = vec![];
            for id in sims_vm::multitile::chain(world, cursor).map_err(anyhow::Error::msg)? {
                let c = world.objects[&id].cursor.as_ref().unwrap();
                let o = registry.object_for_handle(c.underlying.unwrap()).unwrap();
                ensure!(c.purchase == 1 && i32::from(world.objects[&o].data[&1]) == direction);
                parts.push(json!({"cursor":id,"furniture":o,"direction":world.objects[&o].data[&1],"position":world.positions[&o]}));
            }
            ensure!(parts.len() == if name == "chair" { 1 } else { 2 });
            if resumed {
                ensure!(cold.len() == if name == "chair" { 3 } else { 2 });
            }
            ensure!(d.tool_cursor() == Some(0));
            d.refresh(&s)?;
            let pixels = software::render(d.scene());
            let mut saved = source.clone();
            saved["session"] = serde_json::to_value(s.save())?;
            saved["camera_device"] = serde_json::to_value(d.save(&s)?)?;
            if resumed {
                pixels.save(out.join(format!("{name}.png")))?;
                std::fs::write(
                    out.join(format!("{name}-session.json")),
                    serde_json::to_vec(&saved)?,
                )?;
            }
            runs.push(json!({"resumed":resumed,"cold_boundaries":cold,"slices":slices,"parts":parts,"cost":cost,"payload":payload,"tick":tick,"budget_unchanged":true}));
            finals.push((saved, pixels));
        }
        ensure!(
            finals[0] == finals[1],
            "{name} differs across execution budgets and cold restores"
        );
        reports.push(json!({"name":name,"runs":runs,"full_state_and_pixels_equal":true}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"cases":reports,"native_execution":false,"actual_iced_input":false,"move_tool_bound":true,"placement_commit":false}),
        )?,
    )?;
    Ok(())
}

fn finish_selection(s: &mut Session, d: &mut Device) -> Result<()> {
    for n in 0..10000 {
        if d.advance_tool_selection(s, 256)? {
            break;
        }
        ensure!(n < 9999, "catalog callback did not return");
    }
    s.republish_interface()?;
    for n in 0..10000 {
        if matches!(
            d.advance_publication(s, 256)?,
            sims_runtime::original_session::publication::Progress::Complete
        ) {
            return Ok(());
        }
        ensure!(n < 9999, "catalog publication did not return");
    }
    unreachable!()
}
