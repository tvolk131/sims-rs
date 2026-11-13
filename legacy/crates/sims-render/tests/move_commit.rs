//!Actual486ada with stock furniture, live cursor callbacks and undo history.
//!This tests the commit child; the mouse-up affordability/accounting wrapper
//!is deliberately not replaced by a synthetic charge.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Session, move_tool::motion::Input};
use sims_sim::{edit_history, object_catalog::move_tool::cleanup};
use std::collections::BTreeSet;

#[test]
#[ignore = "private stock house assets; release/headless"]
fn original_furniture_commit_keeps_objects_and_undo_history_across_cold_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("commit test panicked")
}
fn finish_move(s: &mut Session, d: &mut Device) -> Result<()> {
    for _ in 0..30000 {
        if d.advance_move_tool(s, 256)? {
            return Ok(());
        }
    }
    anyhow::bail!("Move callback stalled")
}
fn finish_selection(s: &mut Session, d: &mut Device) -> Result<()> {
    for n in 0..10000 {
        if d.advance_tool_selection(s, 256)? {
            break;
        }
        ensure!(n < 9999);
    }
    s.republish_interface()?;
    for _ in 0..10000 {
        if matches!(
            d.advance_publication(s, 256)?,
            sims_runtime::original_session::publication::Progress::Complete
        ) {
            return Ok(());
        }
    }
    anyhow::bail!("catalog publication stalled")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_MOVE_COMMIT_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let source: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-buy-product-frames-1/session.json"),
    )?)?;
    let mut reports = vec![];
    for name in ["chair", "sofa"] {
        let mut finals = vec![];
        for resumed in [false, true] {
            eprintln!("Commit {name}, resumed={resumed}");
            let mut s =
                Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
            let mut d =
                Device::restore(&s, serde_json::from_value(source["camera_device"].clone())?)?;
            if name == "sofa" {
                let c = d.panel_widgets().unwrap().catalog().unwrap();
                ensure!(d.begin_buy_navigation(
                    &mut s,
                    c.revision(),
                    sims_vm::interface::buy::input::Event::Category(2)
                )?);
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
                    .context("sofa absent")?;
                ensure!(d.begin_buy_product(&mut s, c.revision(), sofa)?);
                finish_selection(&mut s, &mut d)?;
            }
            let tick = s.simulation.vm.tick;
            let funds = s.simulation.vm.world.budget.clone();
            let initial_objects: BTreeSet<_> =
                s.simulation.vm.world.objects.keys().copied().collect();
            s.begin_move_tool_viewport_enter()?;
            finish_move(&mut s, &mut d)?;
            let m = s.host.presentation.move_tool.as_ref().unwrap();
            ensure!(
                m.history
                    .budgets
                    .0
                    .values()
                    .all(|b| b.category == 7 && b.pending == 0)
            );
            //The real release gate must reject an invalid preview without
            //consuming its command, removing furniture or charging funds.
            s.begin_move_tool_motion(Input {
                picked: 0,
                position: [456, 264],
                screen: [200, 300],
            })?;
            finish_move(&mut s, &mut d)?;
            let before = serde_json::to_value(&s.simulation)?;
            let old_tool = s.host.presentation.move_tool.clone();
            s.begin_move_tool_commit()?;
            finish_move(&mut s, &mut d)?;
            ensure!(
                serde_json::to_value(&s.simulation)? == before
                    && s.host.presentation.move_tool == old_tool,
                "rejected commit changed state"
            );
            s.begin_move_tool_motion(Input {
                picked: 0,
                position: [376, 488],
                screen: [230, 300],
            })?;
            finish_move(&mut s, &mut d)?;
            let m = s.host.presentation.move_tool.as_ref().unwrap();
            let payload = m.tool.payload.unwrap();
            let held = m.tool.held.unwrap();
            let clipboard = m.tool.clipboard;
            let parts: Vec<_> = s
                .simulation
                .vm
                .world
                .objects
                .iter()
                .filter_map(|(&id, o)| {
                    o.cursor.as_ref()?.underlying.map(|h| {
                        (
                            id,
                            s.simulation
                                .vm
                                .world
                                .scheduler_storage
                                .as_ref()
                                .unwrap()
                                .object_for_handle(h)
                                .unwrap(),
                        )
                    })
                })
                .collect();
            ensure!(parts.len() == if name == "chair" { 1 } else { 2 });
            let destinations: Vec<_> = parts
                .iter()
                .map(|(_, f)| (*f, s.simulation.vm.world.positions[f]))
                .collect();
            //The sofa's original Init also creates an HDSI expansion controller.
            //Keep every real non-cursor identity, not merely the visible parts.
            let retained_objects: BTreeSet<_> = s
                .simulation
                .vm
                .world
                .objects
                .iter()
                .filter_map(|(&id, o)| o.cursor.is_none().then_some(id))
                .collect();
            let created_objects: Vec<_> = retained_objects.difference(&initial_objects)
                .map(|&id| json!({"id":id,"definition":s.simulation.vm.world.objects[&id].definition})).collect();
            s.begin_move_tool_commit()?;
            let mut cold = BTreeSet::new();
            let mut calls = vec![];
            let mut last = None;
            for n in 0..30000 {
                ensure!(s.begin_clock_frame().is_err());
                let m = serde_json::to_value(s.host.presentation.move_tool.as_ref().unwrap())?;
                let signature = json!({"commit":m["call"]["commit"],"dispatch":m["call"]["dispatch"],"child":m["call"]["child"]});
                if last.as_ref() != Some(&signature) {
                    calls.push(signature.clone());
                    last = Some(signature);
                }
                let result = d.advance_move_tool(&mut s, if resumed { 1 } else { 256 });
                if let Err(e) = result {
                    std::fs::write(
                        out.join(format!("{name}-error.json")),
                        serde_json::to_vec(
                            &json!({"session":s.save(),"camera_device":d.save(&s)?}),
                        )?,
                    )?;
                    return Err(e).with_context(|| format!("{name} slice {n}"));
                }
                if result.unwrap() {
                    break;
                }
                let boundary = if s.simulation.cursor_release.is_some() {
                    Some("release")
                } else if s.house_notification_pending() {
                    Some("notification")
                } else {
                    None
                };
                if resumed
                    && let Some(boundary) = boundary
                    && cold.insert(boundary)
                {
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
                    ensure!(saved == json!({"session":s.save(),"camera_device":d.save(&s)?}));
                }
                ensure!(n < 29999, "commit stalled");
            }
            let m = s.host.presentation.move_tool.as_ref().unwrap();
            ensure!(m.tool.held == Some(held) && m.tool.payload.is_none());
            ensure!(
                m.history
                    .store
                    .read(
                        m.history
                            .store
                            .begin(edit_history::List::Undo(clipboard))
                            .map_err(anyhow::Error::msg)?
                    )
                    .map_err(anyhow::Error::msg)?
                    == payload
            );
            for (cursor, furniture) in &parts {
                ensure!(
                    s.simulation.vm.world.objects[cursor]
                        .cursor
                        .as_ref()
                        .unwrap()
                        .underlying
                        .is_none()
                );
                ensure!(s.simulation.vm.world.objects.contains_key(furniture));
            }
            //Native mouse-up next invokes Reset. It must now delete only the
            //empty cursor; later deactivation must not undo the committed item.
            s.begin_move_tool_cleanup(cleanup::Kind::Reset)?;
            finish_move(&mut s, &mut d)?;
            s.begin_move_tool_cleanup(cleanup::Kind::Deactivate)?;
            finish_move(&mut s, &mut d)?;
            ensure!(
                s.simulation
                    .vm
                    .world
                    .objects
                    .keys()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    == retained_objects
            );
            for (f, position) in &destinations {
                ensure!(s.simulation.vm.world.positions.get(f) == Some(position));
            }
            for (c, _) in &parts {
                ensure!(!s.simulation.vm.world.objects.contains_key(c));
            }
            ensure!(s.simulation.vm.tick == tick && s.simulation.vm.world.budget == funds);
            let m = s.host.presentation.move_tool.as_ref().unwrap();
            ensure!(m.tool.held.is_none() && m.tool.payload.is_none());
            ensure!(
                m.history
                    .store
                    .command(payload)
                    .map_err(anyhow::Error::msg)?
                    .references
                    == 1
            );
            ensure!(
                m.history
                    .store
                    .read(
                        m.history
                            .store
                            .begin(edit_history::List::Undo(clipboard))
                            .map_err(anyhow::Error::msg)?
                    )
                    .map_err(anyhow::Error::msg)?
                    == payload
            );
            let saved = json!({"session":s.save(),"camera_device":d.save(&s)?});
            let pixels = software::render(d.scene());
            if resumed {
                ensure!(cold.contains("release") && cold.contains("notification"));
                s = Session::restore(&index, serde_json::from_value(saved["session"].clone())?)?;
                d = Device::restore(&s, serde_json::from_value(saved["camera_device"].clone())?)?;
                ensure!(
                    saved == json!({"session":s.save(),"camera_device":d.save(&s)?})
                        && pixels == software::render(d.scene())
                );
                cold.insert("complete");
                std::fs::write(
                    out.join(format!("{name}-session.json")),
                    serde_json::to_vec(&saved)?,
                )?;
                image::save_buffer(
                    out.join(format!("{name}.png")),
                    &pixels,
                    d.scene().width,
                    d.scene().height,
                    image::ColorType::Rgba8,
                )?;
            }
            finals.push((saved, pixels));
            reports.push(json!({"name":name,"resumed":resumed,"calls":calls,"full_cold_restores":cold.len(),"parts":parts,
                "tick":tick,"created_objects":created_objects,"funds_unchanged":true,"rejection_unchanged":true,"history_retains_commit":true,"reset_keeps_furniture":true}));
        }
        ensure!(
            finals[0] == finals[1],
            "{name} full state/pixels differ after restore or execution budget"
        );
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({"cases":reports,"full_state_and_pixels_equal":true,
        "actual_iced_input":false,"commit_child":true,"mouse_up_accounting":false}))?,
    )?;
    Ok(())
}
