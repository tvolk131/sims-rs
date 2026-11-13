//!Private original-house cursor construction, frozen before+54 placement.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};
use sims_vm::{Definition, construction::ObjectDate};

#[test]
#[ignore = "private original homes and Cursors.iff; release, headless only"]
fn original_house_cursor_pickup_reset_and_cold_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("cursor test panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_CURSOR_FACTORY_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let cursor_program = sims_content::vm::load_program(&index, "Cursors.iff", "Global.iff")?;
    let cursor_source = index.resources[index.find("Cursors.iff")?].locator.clone();
    let words = &cursor_program.modules[&cursor_source].object_definition_words[&128];
    let cursor = Definition {
        source: cursor_source,
        object: 128,
        guid: words[14] as u16 as u32 | ((words[15] as u16 as u32) << 16),
        slots: words[20] as u16,
    };
    let mut reports = vec![];
    for lot in ["goth", "mashuga"] {
        let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
            format!("cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"),
        ))?)?;
        let saved: Saved = serde_json::from_value(wrapper["session"].clone())?;
        let mut initial = Session::restore(&index, saved)?;
        for (name, m) in &cursor_program.modules {
            if let Some(old) = initial.program.modules.get(name) {
                ensure!(old.content_hash == m.content_hash);
            } else {
                initial.program.modules.insert(name.clone(), m.clone());
            }
        }
        initial
            .simulation
            .architecture
            .as_mut()
            .unwrap()
            .bind_footprints(&initial.program)
            .map_err(anyhow::Error::msg)?;
        let baseline = initial.save();
        let camera = serde_json::from_value(wrapper["camera_device"].clone())?;
        let device = Device::restore(&initial, camera)?;
        let before = software::render(device.scene());
        before.save(out.join(format!("{lot}-before.png")))?;
        let camera = device.save(&initial)?;
        for grouped in [false, true] {
            let (&selected, _) = initial
                .simulation
                .vm
                .world
                .objects
                .iter()
                .find(|(id, o)| {
                    o.person.is_none()
                        && o.definition.as_ref().is_some_and(|d| {
                            d.source
                                .contains(if grouped { "Sofas" } else { "DiningChairs" })
                        })
                        && o.slots.iter().all(Option::is_none)
                        && if grouped {
                            o.multitile.as_ref().is_some_and(|m| m.head.is_some())
                        } else {
                            o.multitile.is_none()
                        }
                        && initial
                            .simulation
                            .native_lot
                            .as_ref()
                            .unwrap()
                            .placement
                            .links
                            .capture_raw(&initial.simulation.vm.world, **id)
                            .is_ok_and(|d| d.on_lot)
                })
                .context("real household furniture absent")?;
            let position = initial.simulation.vm.world.positions[&selected];
            let parts = if grouped {
                sims_vm::multitile::chain(&initial.simulation.vm.world, selected)
                    .map_err(anyhow::Error::msg)?
            } else {
                vec![selected]
            };
            let mut finals = vec![];
            let mut counts = vec![];
            for resumed in [false, true] {
                eprintln!("{lot} #{selected} grouped={grouped} restored={resumed}");
                let mut session = Session::restore(&index, baseline.clone())?;
                let c = &session
                    .simulation
                    .household
                    .as_ref()
                    .context("calendar absent")?
                    .clock;
                let date = ObjectDate {
                    year: c.year,
                    month: c.month,
                    day: c.day,
                };
                ensure!(
                    session
                        .simulation
                        .begin_cursor_factory(
                            &session.program,
                            Some(cursor.clone()),
                            selected,
                            position.level,
                            false,
                            date
                        )
                        .map_err(anyhow::Error::msg)?,
                    "original pickup permission denied"
                );
                let mut cold = 0;
                let mut saw_constructor = false;
                let mut saw_update = false;
                let mut instructions = 0;
                for n in 0..5000 {
                    let step = session
                        .simulation
                        .advance_cursor_factory(&session.program, if resumed { 1 } else { 1000 })
                        .map_err(anyhow::Error::msg)?;
                    instructions += step.trace.len();
                    if session.simulation.pending_removal_graphic().is_some() {
                        session.service_removal_graphic(
                            session.host.presentation.camera.context("camera absent")?,
                        )?;
                    }
                    let constructor = session
                        .simulation
                        .vm
                        .world
                        .objects
                        .values()
                        .any(|o| o.cursor.as_ref().is_some_and(|c| !c.postconstructed));
                    let update = session.simulation.vm.scheduled_object_visit.is_some();
                    if resumed && (constructor && !saw_constructor || update && !saw_update) {
                        let saved = session.save();
                        session = Session::restore(
                            &index,
                            serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                        )?;
                        ensure!(
                            serde_json::to_value(session.save())? == serde_json::to_value(saved)?
                        );
                        saw_constructor |= constructor;
                        saw_update |= update;
                        cold += 1;
                    }
                    if session
                        .simulation
                        .cursor_factory
                        .as_ref()
                        .unwrap()
                        .placement()
                        .is_some()
                    {
                        break;
                    }
                    ensure!(n < 4999, "cursor factory did not reach placement");
                }
                let f = session.simulation.cursor_factory.as_ref().unwrap();
                let pending = f.placement().unwrap();
                ensure!(
                    pending.position == [position.x, position.y] && pending.level == position.level
                );
                ensure!(f.created().len() == parts.len());
                ensure!(session.simulation.vm.tick == initial.simulation.vm.tick);
                for &part in &parts {
                    ensure!(session.simulation.vm.world.positions[&part].x == -16);
                }
                let mut device = Device::restore(&session, camera.clone())?;
                device.refresh(&session)?;
                let pixels = software::render(device.scene());
                if resumed {
                    pixels.save(out.join(format!("{lot}-{grouped}-awaiting-placement.png")))?;
                    let saved = session.save();
                    std::fs::write(
                        out.join(format!("{lot}-{grouped}-session.json")),
                        serde_json::to_vec(&saved)?,
                    )?;
                    let camera = device.save(&session)?;
                    session = Session::restore(
                        &index,
                        serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                    )?;
                    device = Device::restore(&session, camera)?;
                    ensure!(software::render(device.scene()) == pixels);
                    cold += 1;
                    ensure!(saw_constructor && (!grouped || saw_update));
                }
                finals.push((serde_json::to_value(session.save())?, pixels));
                counts.push(serde_json::json!({"resumed":resumed,"full_cold_restores":cold,"instructions":instructions}));
            }
            ensure!(
                finals[0] == finals[1],
                "direct and resumed state/pixels differ"
            );
            reports.push(serde_json::json!({"lot":lot,"selected":selected,"multipart":grouped,"parts":parts,"tick":initial.simulation.vm.tick,"runs":counts,"direct_resume_equal":true,"placement_still_pending":true}));
        }
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"native_execution":false,"original_bytecode":true,"cases":reports,"scope":"Live cursor construction/pickup/Reset in original households, frozen before cursor placement/focus/E2. No normal-play frontier or full Buy UI claim."}),
        )?,
    )?;
    Ok(())
}
