//! Real stock-home text publication through the replacement frontend.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{
    camera_device::{Device, panels::Boundary},
    software,
};
use sims_runtime::original_session::{Session, publication::Progress};
use sims_vm::interface::{
    panel_dispatch::{Operation, Request},
    view_control::Widget,
};
use std::path::Path;

#[test]
#[ignore = "private Goth checkpoint; release/headless"]
fn original_goth_publishes_funds_and_clock_with_deterministic_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("panel text check panicked")
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join(std::env::var("SIMS_PANEL_TEXT_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")) && !output.exists());
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let input: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-panel-presentation-frames-2/session.json"),
    )?)?;
    let mut runs = vec![];
    let mut report = vec![];
    for resumed in [false, true] {
        let mut s = Session::restore(&index, serde_json::from_value(input["session"].clone())?)?;
        let mut d = Device::restore(&s, serde_json::from_value(input["camera_device"].clone())?)?;
        let vm = s.simulation.clone();
        let cp = s
            .host
            .presentation
            .transport
            .as_ref()
            .unwrap()
            .controls
            .clone();
        let pixels = software::render(d.scene());
        let expected = s.panel_text()?;
        // The native getter's allowance/cap, including debt. Test writes are
        // restored before publication and no game tick is executed.
        let budget = s.simulation.vm.world.budget.as_ref().unwrap().balance;
        let lot_kind = s.simulation.vm.world.globals[&32];
        for (amount, text) in [
            (i32::MIN, "-$2,147,483,648"),
            (-1, "-$1"),
            (0, "$0"),
            (1234567, "$1,234,567"),
            (i32::MAX, "$9,999,999"),
        ] {
            s.simulation.vm.world.budget.as_mut().unwrap().balance = amount;
            ensure!(s.panel_text()?.funds == text.as_bytes());
        }
        s.simulation.vm.world.globals.insert(32, 1);
        ensure!(s.panel_text()?.funds == b"$999,999");
        s.simulation.vm.world.globals.insert(32, lot_kind);
        s.simulation.vm.world.budget.as_mut().unwrap().balance = budget;
        ensure!(s.simulation == vm);
        d.bind_panel_text(&s)?;
        ensure!(d.bind_panel_text(&s).is_err());
        let fires = s.simulation.vm.world.fires.clone();
        for registered in [vec![], vec![21], vec![21, 21]] {
            let blocked = !registered.is_empty();
            s.simulation.vm.world.fires.registered = registered;
            ensure!(s.panel_access()?.buy_blocked == blocked);
        }
        s.simulation.vm.world.fires = fires;
        for kind in [-1, 0, 1, 2] {
            s.simulation.vm.world.globals.insert(32, kind);
            let access = s.panel_access()?;
            ensure!(access.live_blocked == (kind == 1) && access.labels_hidden == (kind == 1));
        }
        s.simulation.vm.world.globals.insert(32, lot_kind);
        ensure!(s.simulation == vm);
        d.bind_panel_access(&s)?;
        ensure!(d.bind_panel_access(&s).is_err());
        let mut restored = false;
        for n in 0..300 {
            let progress = d.advance_publication(&mut s, if resumed { 1 } else { 256 })?;
            let text = d.panel_widgets().unwrap().texts().unwrap();
            if resumed && !restored && !text.funds().is_empty() && text.clock().is_empty() {
                let save = s.save();
                let device = d.save(&s)?;
                s = Session::restore(&index, serde_json::from_slice(&serde_json::to_vec(&save)?)?)?;
                d = Device::restore(&s, serde_json::from_slice(&serde_json::to_vec(&device)?)?)?;
                ensure!(device == d.save(&s)?);
                restored = true;
            }
            if progress == Progress::Panels(Some(10000)) {
                break;
            }
            ensure!(progress == Progress::Pending && n < 299);
        }
        let widgets = d.panel_widgets().unwrap();
        let text = widgets.texts().unwrap();
        ensure!(text.funds() == sims_formats::strings::decode(&expected.funds, expected.language));
        ensure!(text.clock() == sims_formats::strings::decode(&expected.clock, expected.language));
        ensure!(
            widgets.boundary()
                == Some(Boundary::Dispatcher(Request {
                    operation: Operation::HideBuild,
                    child: Some(10012),
                    state: None,
                })),
            "unexpected frontier: {:?}",
            widgets.boundary()
        );
        ensure!(widgets.presentation().unwrap().collapsed() == Some(true));
        ensure!(widgets.presentation().unwrap().active().is_none());
        ensure!(d.panel_state() == Some(1));
        ensure!(widgets.button(Widget::Buy).enabled && widgets.button(Widget::Build).enabled);
        ensure!(widgets.button(Widget::Live).enabled && widgets.button(Widget::Camera).enabled);
        ensure!(
            widgets.access().unwrap().funds_visible() && widgets.access().unwrap().label_visible()
        );
        ensure!(s.simulation == vm && software::render(d.scene()) == pixels);
        ensure!(s.host.presentation.transport.as_ref().unwrap().controls == cp);
        ensure!(!resumed || restored);
        report.push(
            json!({"funds":text.funds(), "clock":text.clock(), "tick":s.simulation.vm.tick,
            "cold_restore":restored,"simulation_unchanged":true,"cpstate_unchanged":true,
            "frontier":format!("{:?}",widgets.boundary())}),
        );
        let result = json!({"session":s.save(),"camera_device":d.save(&s)?});
        ensure!(d.advance_publication(&mut s, 256)? == Progress::Panels(Some(10000)));
        ensure!(result == json!({"session":s.save(),"camera_device":d.save(&s)?}));
        if resumed {
            std::fs::write(output.join("session.json"), serde_json::to_vec(&result)?)?;
            pixels.save(output.join("goth.png"))?;
        }
        runs.push(result);
    }
    ensure!(runs[0] == runs[1], "publication differs after restore");
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "equal":true,"runs":report,"new_gameplay_tick":false,
            "scope":"actual funds/clock, collapsed panel and availability publication; Build/Buy children remain pending"
        }))?,
    )?;
    Ok(())
}
