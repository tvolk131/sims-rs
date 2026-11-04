//!Real button-down/up, budget DA, repeat placement and cancellation.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{
    Session,
    move_tool::{
        button::{Input, Kind},
        motion,
    },
};
use std::collections::BTreeSet;
#[test]
#[ignore = "private stock house; release/headless"]
fn original_buy_buttons_charge_repeat_rotate_cancel_and_cold_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("button test panicked")
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
    let out = root.join(std::env::var("SIMS_MOVE_BUTTON_OUTPUT")?);
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
            eprintln!("Buy buttons {name} resumed={resumed}");
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
            let product = d
                .panel_widgets()
                .unwrap()
                .catalog()
                .unwrap()
                .view()
                .unwrap()
                .products
                .iter()
                .copied()
                .find(|&i| d.panel_widgets().unwrap().catalog().unwrap().selected(i))
                .context("selected product absent")?;
            let tick = s.simulation.vm.tick;
            let original_budget = s.simulation.vm.world.budget.clone().unwrap();
            s.begin_move_tool_viewport_enter()?;
            finish_move(&mut s, &mut d)?;
            let cost = s.host.presentation.move_tool.as_ref().unwrap().tool.value90 as i32;
            let automatic_repeat = s
                .host
                .presentation
                .move_tool
                .as_ref()
                .unwrap()
                .tool
                .tool
                .flags[1]
                != 0;
            let mut cold = BTreeSet::new();
            let mut states = vec![];
            let mut paid = vec![];
            for purchase in 0..2 {
                let pointer = motion::Input {
                    position: if purchase == 0 {
                        [376, 488]
                    } else {
                        [424, 520]
                    },
                    screen: [200 + purchase * 30, 300 + purchase * 30],
                    picked: 0,
                };
                s.begin_move_tool_motion(pointer)?;
                finish_move(&mut s, &mut d)?;
                let furniture: Vec<_> = s
                    .simulation
                    .vm
                    .world
                    .objects
                    .values()
                    .filter_map(|o| o.cursor.as_ref()?.underlying)
                    .collect();
                let mut up = pointer;
                for kind in [Kind::Down, Kind::Up] {
                    if kind == Kind::Up && purchase == 1 {
                        up.screen = [pointer.screen[0] + 20, pointer.screen[1] + 20];
                        s.begin_move_tool_motion(up)?;
                        finish_move(&mut s, &mut d)?;
                        let desired = (2 - s
                            .host
                            .presentation
                            .presenter
                            .as_ref()
                            .unwrap()
                            .view
                            .display_rotation as i32
                            * 2)
                            & 7;
                        let id = s
                            .simulation
                            .vm
                            .world
                            .scheduler_storage
                            .as_ref()
                            .unwrap()
                            .object_for_handle(furniture[0])
                            .unwrap();
                        ensure!(
                            i32::from(s.simulation.vm.world.objects[&id].data[&1]) == desired,
                            "drag did not orient furniture"
                        );
                    }
                    s.begin_move_button(Input {
                        kind,
                        pointer: if kind == Kind::Down { pointer } else { up },
                        shift: purchase == 0,
                    })?;
                    let mut trace = vec![];
                    let mut last = None;
                    for n in 0..50000 {
                        ensure!(s.begin_clock_frame().is_err());
                        let b = serde_json::to_value(
                            s.host.presentation.move_button.as_ref().unwrap(),
                        )?;
                        if last.as_ref() != Some(&b) {
                            trace.push(b.clone());
                            last = Some(b.clone());
                        }
                        let result = d.advance_move_button(&mut s, if resumed { 1 } else { 256 });
                        if let Err(e) = result {
                            std::fs::write(
                                out.join(format!("{name}-error.json")),
                                serde_json::to_vec(
                                    &json!({"session":s.save(),"camera_device":d.save(&s)?}),
                                )?,
                            )?;
                            return Err(e).with_context(|| {
                                format!("{name} purchase{purchase} {kind:?} slice{n}")
                            });
                        }
                        if result.unwrap() {
                            break;
                        }
                        let b = serde_json::to_value(
                            s.host.presentation.move_button.as_ref().unwrap(),
                        )?;
                        let boundary = if s.simulation.cursor_release.is_some() {
                            Some("release")
                        } else if b["child"].get("Budget").is_some() {
                            Some("budget")
                        } else if b["child"] == "Selector" {
                            Some("selector")
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
                            ensure!(
                                saved == json!({"session":s.save(),"camera_device":d.save(&s)?}),
                                "cold state changed"
                            );
                        }
                        ensure!(n < 49999, "button stalled");
                    }
                    states.push(json!({"purchase":purchase,"kind":kind,"trace":trace,"tool":s.host.presentation.move_tool,"budget":s.simulation.vm.world.budget}));
                }
                let budget = s.simulation.vm.world.budget.as_ref().unwrap();
                ensure!(
                    budget.balance == original_budget.balance - (i32::from(purchase) + 1) * cost,
                    "purchase did not charge once"
                );
                ensure!(
                    budget.expenses[7]
                        == original_budget.expenses[7] + (i32::from(purchase) + 1) * cost
                );
                for h in furniture {
                    let id = s
                        .simulation
                        .vm
                        .world
                        .scheduler_storage
                        .as_ref()
                        .unwrap()
                        .object_for_handle(h)
                        .unwrap();
                    ensure!(s.simulation.vm.world.positions[&id].x > 0);
                    paid.push(id);
                }
                let m = s.host.presentation.move_tool.as_ref().unwrap();
                ensure!(
                    m.tool.held.is_some() == (purchase == 0 || automatic_repeat),
                    "repeat placement continuation differs"
                );
                ensure!(m.history.budgets.0.values().all(|b| b.pending == 0));
            }
            if !automatic_repeat {
                let buy = &s.host.presentation.transport.as_ref().unwrap().controls.buy;
                ensure!(buy.selected_object == 0 && buy.selection_aux == u32::MAX);
                ensure!(
                    !d.panel_widgets()
                        .unwrap()
                        .catalog()
                        .unwrap()
                        .selected(product)
                );
                //A single-placement product clears selection; select it again
                //to exercise cancellation. Auto-repeat already owns a preview.
                let c = d.panel_widgets().unwrap().catalog().unwrap();
                ensure!(d.begin_buy_product(&mut s, c.revision(), product)?);
                finish_selection(&mut s, &mut d)?;
                s.begin_move_tool_viewport_enter()?;
                finish_move(&mut s, &mut d)?;
            } else {
                ensure!(
                    d.panel_widgets()
                        .unwrap()
                        .catalog()
                        .unwrap()
                        .selected(product)
                );
            }
            let unpaid: Vec<_> = s
                .simulation
                .vm
                .world
                .objects
                .values()
                .filter_map(|o| o.cursor.as_ref()?.underlying)
                .map(|h| {
                    s.simulation
                        .vm
                        .world
                        .scheduler_storage
                        .as_ref()
                        .unwrap()
                        .object_for_handle(h)
                        .unwrap()
                })
                .collect();
            s.begin_move_button(Input {
                kind: Kind::Cancel,
                pointer: motion::Input {
                    position: [-16, -16],
                    screen: [0, 0],
                    picked: 0,
                },
                shift: false,
            })?;
            for n in 0..50000 {
                if d.advance_move_button(&mut s, 256)? {
                    break;
                }
                ensure!(n < 49999);
            }
            ensure!(
                paid.iter()
                    .all(|id| s.simulation.vm.world.objects.contains_key(id))
                    && unpaid
                        .iter()
                        .all(|id| !s.simulation.vm.world.objects.contains_key(id))
            );
            ensure!(
                s.simulation
                    .vm
                    .world
                    .objects
                    .values()
                    .all(|o| o.cursor.is_none())
            );
            ensure!(
                s.simulation.vm.world.budget.as_ref().unwrap().balance
                    == original_budget.balance - 2 * cost
                    && s.simulation.vm.tick == tick
            );
            let saved = json!({"session":s.save(),"camera_device":d.save(&s)?});
            let pixels = software::render(d.scene());
            if resumed {
                ensure!(
                    cold.len() == if automatic_repeat { 2 } else { 3 },
                    "missing cold boundary: {cold:?}"
                );
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
            reports.push(json!({"name":name,"resumed":resumed,"automatic_repeat":automatic_repeat,"full_cold_restores":cold.len(),"states":states,"paid":paid,"cost":cost,"tick":tick,"funds":s.simulation.vm.world.budget.as_ref().unwrap().balance,"cancelled":unpaid}));
        }
        ensure!(
            finals[0] == finals[1],
            "button state/pixels differ across budgets and cold restores"
        );
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"cases":reports,"full_state_and_pixels_equal":true,"actual_iced_input":false}),
        )?,
    )?;
    Ok(())
}
