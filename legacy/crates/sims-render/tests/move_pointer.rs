//!Real stock-house Move pointer work, including pending engine children.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Session, move_tool::motion::Input};
use std::collections::BTreeSet;

#[test]
#[ignore = "private stock house assets; release/headless"]
fn original_furniture_pointer_moves_rejects_and_resumes_without_ticks_or_spending() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("pointer test panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_MOVE_POINTER_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut reports = vec![];
    for name in ["chair", "sofa"] {
        let source: Value = serde_json::from_slice(&std::fs::read(root.join(format!(
            "cache/original-move-preview-frames-3/{name}-session.json"
        )))?)?;
        let mut finals = vec![];
        for resumed in [false, true] {
            eprintln!("Pointer {name}, resumed={resumed}");
            let mut s =
                Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
            let mut d =
                Device::restore(&s, serde_json::from_value(source["camera_device"].clone())?)?;
            let tick = s.simulation.vm.tick;
            let funds = s.simulation.vm.world.budget.clone();
            s.begin_move_tool_viewport_enter()?;
            for n in 0..20000 {
                if d.advance_move_tool(&mut s, 256)? {
                    break;
                }
                ensure!(n < 19999);
            }
            ensure!(
                s.host
                    .presentation
                    .move_tool
                    .as_ref()
                    .unwrap()
                    .tool
                    .byte(0x97)
                    == 1
            );
            let count = s.simulation.vm.world.objects.len();
            let mut cold = BTreeSet::new();
            let mut states = vec![];
            //Open lawn, an occupied furnishing tile, off-lot, then open lawn.
            for (number, position) in [[360, 488], [456, 264], [-16, -16], [376, 488]]
                .into_iter()
                .enumerate()
            {
                s.begin_move_tool_motion(Input {
                    picked: 0,
                    position,
                    screen: [200 + number as i16 * 30, 300],
                })?;
                let mut calls = vec![];
                let mut last = None;
                for n in 0..30000 {
                    ensure!(s.begin_clock_frame().is_err());
                    let movement =
                        serde_json::to_value(s.host.presentation.move_tool.as_ref().unwrap())?;
                    let signature = json!({"phase":movement["call"]["motion"]["phase"],"point":movement["call"]["motion"]["point"],"rotation":movement["call"]["motion"]["rotation"]});
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
                        return Err(e)
                            .with_context(|| format!("{name} position{position:?} slice{n}"));
                    }
                    if result.unwrap() {
                        break;
                    }
                    if resumed
                        && number == 0
                        && s.simulation
                            .cursor_rotation
                            .as_ref()
                            .is_some_and(|r| r.point().is_some())
                        && cold.is_empty()
                    {
                        let saved = json!({"session":s.save(),"camera_device":d.save(&s)?});
                        std::fs::write(
                            out.join(format!("{name}-point-pending.json")),
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
                        cold.insert("point");
                    }
                    ensure!(n < 29999, "pointer stalled");
                }
                ensure!(
                    s.simulation.vm.tick == tick
                        && s.simulation.vm.world.budget == funds
                        && s.simulation.vm.world.objects.len() == count
                );
                let m = s.host.presentation.move_tool.as_ref().unwrap();
                let held = m.tool.held.unwrap();
                let id = s
                    .simulation
                    .vm
                    .world
                    .scheduler_storage
                    .as_ref()
                    .unwrap()
                    .object_for_handle(held)
                    .unwrap();
                let c = s.simulation.vm.world.objects[&id].cursor.as_ref().unwrap();
                let state = json!({"position":position,"cursor":c,"physical":s.simulation.vm.world.positions[&id],"tool":m.tool,"fields":serde_json::to_value(m)?["preview_fields"],"error":s.simulation.native_lot.as_ref().unwrap().diagnostics.error});
                eprintln!(
                    "{name} position{position:?} valid={}, physical={:?}",
                    c.valid, s.simulation.vm.world.positions[&id]
                );
                if number == 0 || number == 3 {
                    ensure!(
                        c.valid != 0 && m.tool.byte(0x95) == 1,
                        "open lawn preview rejected: {state}"
                    );
                }
                if number == 1 {
                    ensure!(
                        c.valid == 0 && m.tool.byte(0x95) == 0,
                        "occupied fridge tile accepted: {state}"
                    );
                }
                states.push(json!({"state":state,"calls":calls}));
                if resumed {
                    let pixels = software::render(d.scene());
                    image::save_buffer(
                        out.join(format!("{name}-{number}.png")),
                        &pixels,
                        d.scene().width,
                        d.scene().height,
                        image::ColorType::Rgba8,
                    )?;
                }
            }
            if resumed {
                ensure!(cold.len() == 1);
            }
            let saved = json!({"session":s.save(),"camera_device":d.save(&s)?});
            if resumed {
                std::fs::write(
                    out.join(format!("{name}-session.json")),
                    serde_json::to_vec(&saved)?,
                )?;
            }
            finals.push((saved, software::render(d.scene())));
            reports.push(json!({"name":name,"resumed":resumed,"states":states,"full_cold_restores":cold.len(),"funds_unchanged":true,"tick":tick,"objects_unchanged":true}));
        }
        ensure!(
            finals[0] == finals[1],
            "pointer execution budgets/cold restore changed {name}"
        );
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"cases":reports,"full_state_and_pixels_equal":true,"purchase_committed":false}),
        )?,
    )?;
    Ok(())
}
