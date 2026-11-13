//!Actual Goth/catalog through4cea5d Live->Buy, before4ce44f publication.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{
    camera_device::{Device, mode::Window},
    software,
};
use sims_runtime::original_session::{
    Saved, Session,
    tool_selection::{Receiver, State, mode},
};
use sims_vm::interface::{mode_change::Request, tool::Fields};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[test]
#[ignore = "private original household; release/headless"]
fn original_house_live_to_buy_owns_tool_window_callbacks_and_defers_simulator_publication()
-> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("mode transition panicked")
}
fn cold(index: &Index, s: &mut Session, d: &mut Device) -> Result<()> {
    let saved = s.save();
    let device = d.save(s)?;
    let pixels = software::render(d.scene());
    *s = Session::restore(index, serde_json::from_slice(&serde_json::to_vec(&saved)?)?)?;
    *d = Device::restore(s, serde_json::from_slice(&serde_json::to_vec(&device)?)?)?;
    ensure!(serde_json::to_value(saved)? == serde_json::to_value(s.save())?);
    ensure!(device == d.save(s)? && pixels == software::render(d.scene()));
    Ok(())
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_LIVE_BUY_MODE_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let seed: Saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-live-move-activation-frames-1/session.json"),
    )?)?;
    let wrapper: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-ui-timer-dispatch-iced/goth/resumed.json"),
    )?)?;
    let mut reports = vec![];
    let mut finals = vec![];
    for resumed in [false, true] {
        eprintln!("live Buy mode resumed={resumed}");
        let mut s = Session::restore(&index, seed.clone())?;
        let mut slots = [None; 12];
        slots[6] = Some(6000);
        slots[7] = Some(7000);
        let mut tools = State::new(
            slots,
            BTreeMap::from([
                (6000, Receiver::Picker { mode: Some(0) }),
                (7000, Receiver::Move),
            ]),
            Fields {
                active: Some(6000),
                argument: 0,
            },
        );
        tools.bind_mode_context(mode::Context::new(8000))?;
        s.bind_tool_selection(tools)?;
        let original = s.simulation.clone();
        let original_controls = s
            .host
            .presentation
            .transport
            .as_ref()
            .context("CPState absent")?
            .controls
            .clone();
        ensure!(original_controls.mode == 2);
        let mut d = Device::restore(
            &s,
            serde_json::from_value(wrapper["camera_device"].clone())?,
        )?;
        d.bind_mode_window(
            &s,
            Window::new(9000, Some(9001), 80, original_controls.paused, 0xfffffff0),
        )?;
        let pixels = software::render(d.scene());
        let mut restored = BTreeSet::new();
        let mut events = vec![];
        s.begin_mode_change(1, 0)?;
        for n in 0..4000 {
            let before = s.save();
            ensure!(s.step().is_err());
            ensure!(serde_json::to_value(before)? == serde_json::to_value(s.save())?);
            let old_mode = s
                .host
                .presentation
                .transport
                .as_ref()
                .unwrap()
                .controls
                .mode;
            let finished = d.advance_tool_selection(&mut s, if resumed { 1 } else { 1000 })?;
            let state = s.host.presentation.tool_selection.as_ref().unwrap();
            let value = serde_json::to_value(state)?;
            let label = if !value["call"]["child"].is_null() && old_mode == 1 {
                Some("Buy activation")
            } else if d.mode_window().unwrap().pending() {
                Some("window timer")
            } else if state.mode_request() == Some(&Request::ResetBuySelection) {
                Some("catalog reset")
            } else if old_mode == 2
                && s.host
                    .presentation
                    .transport
                    .as_ref()
                    .unwrap()
                    .controls
                    .mode
                    == 1
            {
                Some("mode published")
            } else {
                None
            };
            if resumed {
                if let Some(label) = label.filter(|label| restored.insert(label.to_string())) {
                    if label == "window timer" {
                        let serial = s
                            .host
                            .presentation
                            .tool_selection
                            .as_ref()
                            .unwrap()
                            .mode_window_owner()
                            .context("window parent token absent")?;
                        let before = serde_json::to_value(s.save())?;
                        ensure!(s.claim_mode_window_pause(1, 1).is_err());
                        ensure!(
                            s.acknowledge_mode_window_pause(serial.wrapping_add(1), 1, 1)
                                .is_err()
                        );
                        ensure!(before == serde_json::to_value(s.save())?);
                        for corruption in ["missing call", "missing backend", "wrong token"] {
                            let mut device = serde_json::to_value(d.save(&s)?)?;
                            match corruption {
                                "missing call" => device["mode_window"]["call"] = Value::Null,
                                "missing backend" => device["mode_window"] = Value::Null,
                                _ => {
                                    device["mode_window"]["call"]["serial"] =
                                        json!(serial.wrapping_add(1))
                                }
                            }
                            ensure!(
                                Device::restore(&s, serde_json::from_value(device)?).is_err(),
                                "{corruption} was accepted"
                            );
                        }
                    }
                    eprintln!("restoring {label}");
                    cold(&index, &mut s, &mut d)?;
                }
            }
            if let Some(r) = s
                .host
                .presentation
                .tool_selection
                .as_ref()
                .unwrap()
                .mode_request()
            {
                events.push(format!("{r:?}"));
            }
            if finished {
                break;
            }
            ensure!(n < 3999, "mode transition stalled");
        }
        let controls = &s.host.presentation.transport.as_ref().unwrap().controls;
        let window = d.mode_window().unwrap();
        ensure!(controls.mode == 1 && controls.buy.catalog == 1 && controls.buy.subcatalog == 0);
        ensure!(
            controls.buy.selected_object == 0
                && controls.buy.selected_table == 0
                && controls.buy.selection_aux == 0
        );
        ensure!(
            controls.buy.dirty & 0xfff7ffbf == 0xfff7ffbf
                && controls.paused == original_controls.paused
        );
        ensure!(
            s.host
                .presentation
                .presenter
                .as_ref()
                .unwrap()
                .camera
                .as_ref()
                .unwrap()
                .settings
                .application_mode
                == 1
        );
        ensure!(
            window.fields().paused == 1
                && window.fields().timer_active == 1
                && window.indicator_visible()
        );
        ensure!(window.timers().entries.len() == 1 && window.timers().entries[0].deadline == 64);
        ensure!(s.simulation == original && pixels == software::render(d.scene()));
        ensure!(s.mode_publication_pending() && !s.tool_selection_pending());
        let before = serde_json::to_value(s.save())?;
        ensure!(s.step().is_err() && s.begin_mode_change(2, 0).is_err());
        ensure!(before == serde_json::to_value(s.save())?);
        if resumed {
            ensure!(restored.contains("Buy activation") && restored.contains("window timer"));
            cold(&index, &mut s, &mut d)?;
            restored.insert("transition returned".into());
            software::render(d.scene()).save(out.join("goth.png"))?;
            std::fs::write(
                out.join("session.json"),
                serde_json::to_vec(&json!({"session":s.save(),"camera_device":d.save(&s)?}))?,
            )?;
        }
        reports.push(json!({"resumed":resumed,"restored":restored,"events":events,
            "simulation_unchanged":true,"pixels_unchanged":true,"mode":1,"publication_still_pending":true}));
        finals.push((
            serde_json::to_value(s.save())?,
            d.save(&s)?,
            software::render(d.scene()),
        ));
    }
    ensure!(
        finals[0] == finals[1],
        "mode transition differs across budgets/restores"
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({"equal":true,"runs":reports,
        "scope":"Actual4cea5d Live->Buy composition.4ce44f Buy publication, catalog widget population and purchases remain pending; normal simulation blocked until publication."}))?,
    )?;
    Ok(())
}
