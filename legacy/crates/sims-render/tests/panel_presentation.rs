//! Replacement panel effects over a real household, without legacy surfaces.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{
    camera_device::{
        Device,
        panels::{Boundary, Panel},
    },
    software,
};
use sims_runtime::original_session::{Session, publication::Progress};
use sims_vm::interface::view_control::{Query, Request, Widget};
use std::path::Path;

#[test]
#[ignore = "private original Goth checkpoint; release/headless"]
fn original_goth_switches_visible_panel_and_controls_without_changing_the_simulation() -> Result<()>
{
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("panel presentation panicked")
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_PANEL_PRESENTATION_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let input: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-panel-widgets-frames-2/session.json"),
    )?)?;
    let mut results = Vec::new();
    for resumed in [false, true] {
        let mut s = Session::restore(&index, serde_json::from_value(input["session"].clone())?)?;
        let mut d = Device::restore(&s, serde_json::from_value(input["camera_device"].clone())?)?;
        check_borrowed_queries(&mut s)?;
        let simulation = s.simulation.clone();
        let cp = s
            .host
            .presentation
            .transport
            .as_ref()
            .unwrap()
            .controls
            .clone();
        let timers = d.mode_window().unwrap().timers().clone();
        let pixels = software::render(d.scene());
        d.bind_panel_presentation(&s, Panel::People)?;
        ensure!(d.bind_panel_presentation(&s, Panel::Buy).is_err());
        let mut restored = false;
        for _ in 0..200 {
            let progress = d.advance_publication(&mut s, if resumed { 1 } else { 256 })?;
            let widgets = d.panel_widgets().unwrap();
            if resumed && !restored && widgets.presentation().unwrap().visible(Panel::Buy) {
                let saved = s.save();
                let device = d.save(&s)?;
                s = Session::restore(
                    &index,
                    serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                )?;
                d = Device::restore(&s, serde_json::from_slice(&serde_json::to_vec(&device)?)?)?;
                ensure!(device == d.save(&s)?);
                ensure!(serde_json::to_value(saved)? == serde_json::to_value(s.save())?);
                restored = true;
            }
            if progress == Progress::Panels(Some(10000)) {
                break;
            }
            ensure!(progress == Progress::Pending);
        }
        let widgets = d.panel_widgets().unwrap();
        ensure!(widgets.presentation().unwrap().active() == Some(Panel::Buy));
        ensure!(!widgets.presentation().unwrap().visible(Panel::People));
        ensure!(widgets.button(Widget::Buy).selected && !widgets.button(Widget::Live).selected);
        ensure!(widgets.button(Widget::FloorOne).selected);
        ensure!(widgets.button(Widget::FloorTwo).enabled);
        ensure!(!widgets.button(Widget::FloorTwo).selected);
        ensure!(widgets.button(Widget::ZoomIn).enabled);
        ensure!(!widgets.button(Widget::ZoomOut).enabled);
        // Text and later catalog work have not been acknowledged.
        ensure!(
            widgets.boundary()
                == Some(Boundary::Control(Request::Get {
                    owner: 10001,
                    query: Query::Status,
                }))
        );
        ensure!(s.simulation == simulation && software::render(d.scene()) == pixels);
        ensure!(s.host.presentation.transport.as_ref().unwrap().controls == cp);
        ensure!(d.mode_window().unwrap().timers() == &timers);
        ensure!(s.step().is_err());
        ensure!(!resumed || restored);
        let result = json!({"session":s.save(),"camera_device":d.save(&s)?});
        ensure!(d.advance_publication(&mut s, 256)? == Progress::Panels(Some(10000)));
        ensure!(result == json!({"session":s.save(),"camera_device":d.save(&s)?}));
        if resumed {
            std::fs::write(out.join("session.json"), serde_json::to_vec(&result)?)?;
            software::render(d.scene()).save(out.join("goth.png"))?;
        }
        results.push(result);
    }
    ensure!(
        results[0] == results[1],
        "budget/restore changed replacement panel effects"
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "equal":true,"cases":2,"full_cold_restores":1,"tick":88525,
            "visible_panel":"Buy","people_input_visible":false,
            "simulation_unchanged":true,"cpstate_unchanged":true,"timers_unchanged":true,
            "frontier":"ViewControl Status getter; catalog contents and mode publication remain pending",
            "new_gameplay_tick":false,
        }))?,
    )?;
    Ok(())
}

fn check_borrowed_queries(s: &mut Session) -> Result<()> {
    let before = serde_json::to_value(s.save())?;
    let saved_ui = s.host.presentation.camera_ui.unwrap();
    let globals = s.simulation.vm.world.globals.clone();
    ensure!(s.panel_number(Query::Floor)? == Some(1));
    ensure!(s.panel_number(Query::UpperFloorAvailable)? == Some(1));
    ensure!(s.panel_number(Query::BuildFloorAvailable)? == Some(1));
    ensure!(s.panel_number(Query::Status)?.is_none());
    ensure!(s.panel_number(Query::BuyBlocked)?.is_none());
    // The adapter must use CPState's requested floor, not the already rendered
    // scene. These temporary test writes also cover signed native comparisons.
    for (floor, walls) in [(-1, 1), (0, 1), (1, 7), (2, 7), (3, 1)] {
        s.host.presentation.camera_ui.as_mut().unwrap().floor = floor;
        ensure!(s.panel_number(Query::Floor)? == Some(floor as u32));
        ensure!(s.panel_number(Query::AvailableWalls)? == Some(walls));
    }
    for lot in [5, 99] {
        s.simulation.vm.world.globals.insert(10, lot);
        for zoom in [-1, 0, 1, 2, 3, 4] {
            s.host.presentation.camera_ui.as_mut().unwrap().zoom = zoom as u32;
            ensure!(s.panel_number(Query::ZoomIn)? == Some(u32::from(zoom < 3)));
            ensure!(
                s.panel_number(Query::ZoomOut)?
                    == Some(u32::from(zoom > if lot == 99 { 2 } else { 1 }))
            );
        }
    }
    for value in [i16::MIN, -1, 0, 760, i16::MAX] {
        for global in [16, 17, 18] {
            s.simulation.vm.world.globals.insert(global, value);
        }
        ensure!(s.panel_number(Query::Speed)? == Some(value as i32 as u32));
        ensure!(s.panel_number(Query::SavedSpeed)? == Some(value as i32 as u32));
        ensure!(s.panel_number(Query::SavedSpeedActive)? == Some(u32::from(value >= 0)));
        ensure!(s.panel_number(Query::LotPaused)? == Some(u32::from(value != 0)));
    }
    s.host.presentation.camera_ui = Some(saved_ui);
    s.simulation.vm.world.globals = globals;
    ensure!(before == serde_json::to_value(s.save())?);
    Ok(())
}
