//! Actual category input owns its native tool reset and survives a cold resume.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Session, publication::Progress};
use sims_vm::interface::buy::input::Event;

#[test]
#[ignore = "private Goth Buy checkpoint; release/headless"]
fn original_buy_navigation_runs_real_tool_children_and_resumes_mid_input() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("Buy input panicked")
}
fn publish(s: &mut Session, d: &mut Device, context: Value) -> Result<()> {
    s.begin_interface_publication(serde_json::from_value(context)?, 0)?;
    for _ in 0..100 {
        match d.advance_publication(s, 256)? {
            Progress::Complete => return Ok(()),
            Progress::Pending => (),
            other => anyhow::bail!("catalog publication child: {other:?}"),
        }
    }
    anyhow::bail!("catalog publication stalled")
}
fn finish_input(s: &mut Session, d: &mut Device) -> Result<()> {
    for _ in 0..1000 {
        if d.advance_tool_selection(s, 1)? {
            return Ok(());
        }
    }
    anyhow::bail!("catalog tool input stalled")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join(std::env::var("SIMS_BUY_INPUT_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")) && !output.exists());
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let input: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-buy-panel-frames-2/session.json"),
    )?)?;
    let context = input["session"]["view"]["publication"]["context"].clone();
    let mut s = Session::restore(&index, serde_json::from_value(input["session"].clone())?)?;
    let mut d = Device::restore(&s, serde_json::from_value(input["camera_device"].clone())?)?;
    let simulation = s.simulation.clone();
    let pixels = software::render(d.scene());
    let revision = d.panel_widgets().unwrap().catalog().unwrap().revision();
    ensure!(
        !d.begin_buy_navigation(&mut s, revision, Event::Category(1))?,
        "hidden catalog accepted input"
    );
    d.begin_panel_expansion(&mut s, 0)?;
    for _ in 0..10 {
        if d.advance_panel(&mut s)? {
            break;
        }
    }
    ensure!(!s.control_panel_pending());
    ensure!(d.begin_buy_navigation(&mut s, revision, Event::Category(1))?);
    ensure!(!d.advance_tool_selection(&mut s, 1)?);
    ensure!(s.tool_selection_pending());
    let state = serde_json::to_value(s.host.presentation.tool_selection.as_ref().unwrap())?;
    ensure!(state["call"]["frames"].as_array().unwrap().len() == 2);
    let mid = json!({"session":s.save(),"camera_device":d.save(&s)?});
    let mut outcomes = vec![];
    for resumed in [false, true] {
        if resumed {
            s = Session::restore(&index, serde_json::from_value(mid["session"].clone())?)?;
            d = Device::restore(&s, serde_json::from_value(mid["camera_device"].clone())?)?;
            ensure!(mid == json!({"session":s.save(),"camera_device":d.save(&s)?}));
        }
        ensure!(
            s.begin_clock_frame().is_err(),
            "clock overtook catalog input"
        );
        finish_input(&mut s, &mut d)?;
        publish(&mut s, &mut d, context.clone())?;
        let catalog = d.panel_widgets().unwrap().catalog().unwrap();
        ensure!(catalog.view().unwrap().heading == "Surfaces");
        ensure!(catalog.view().unwrap().categories[0].selected);
        ensure!(!catalog.view().unwrap().products.is_empty());
        ensure!(s.simulation == simulation && software::render(d.scene()) == pixels);
        outcomes.push(json!({"session":s.save(),"camera_device":d.save(&s)?}));
    }
    ensure!(
        outcomes[0] == outcomes[1],
        "input differs after cold restore"
    );
    std::fs::write(
        output.join("surfaces.json"),
        serde_json::to_vec(&outcomes[1])?,
    )?;
    let stale = revision;
    let mut steps = vec![];
    for event in [
        Event::Next,
        Event::Previous,
        Event::Category(7),
        Event::Back,
        Event::Category(0),
    ] {
        let revision = d.panel_widgets().unwrap().catalog().unwrap().revision();
        let before = serde_json::to_value(s.save())?;
        ensure!(!d.begin_buy_navigation(&mut s, stale, event)?);
        ensure!(before == serde_json::to_value(s.save())?);
        let accepted = d.begin_buy_navigation(&mut s, revision, event)?;
        if accepted {
            finish_input(&mut s, &mut d)?;
            publish(&mut s, &mut d, context.clone())?;
        }
        steps.push(json!({"event":event,"accepted":accepted,"view":d.panel_widgets().unwrap().catalog().unwrap().view()}));
        ensure!(s.simulation == simulation && software::render(d.scene()) == pixels);
    }
    ensure!(steps.iter().all(|v| v["accepted"] == true));
    std::fs::write(
        output.join("session.json"),
        serde_json::to_vec(&json!({"session":s.save(),"camera_device":d.save(&s)?}))?,
    )?;
    pixels.save(output.join("goth.png"))?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "tick":s.simulation.vm.tick,"cold_restore_equal":true,"real_tool_callbacks":true,
            "whole_simulation_unchanged":true,"headless_pixels_equal":true,"blocked_clock_and_stale_input":true,
            "steps":steps,"purchasing":false,
        }))?,
    )?;
    Ok(())
}
