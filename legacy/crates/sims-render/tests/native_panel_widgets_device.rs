//! Real Goth 50a8ec -> ViewControl -> pause setters, preserving the full VM.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{
    camera_device::{
        Device,
        panels::{Boundary, Owners},
    },
    software,
};
use sims_runtime::original_session::{Session, publication::Progress};
use std::{collections::BTreeSet, path::Path};

#[test]
#[ignore = "private Goth checkpoint; release, 32MiB stack, headless"]
fn goth_panel_widgets_resume_native_callbacks_without_advancing_the_household() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("panel widgets panicked")
}
fn cold(index: &Index, s: &mut Session, d: &mut Device) -> Result<()> {
    let saved = s.save();
    let device = d.save(s)?;
    let pixels = software::render(d.scene());
    *s = Session::restore(index, serde_json::from_slice(&serde_json::to_vec(&saved)?)?)?;
    *d = Device::restore(s, serde_json::from_slice(&serde_json::to_vec(&device)?)?)?;
    ensure!(serde_json::to_value(&saved)? == serde_json::to_value(s.save())?);
    ensure!(device == d.save(s)? && pixels == software::render(d.scene()));
    Ok(())
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_PANEL_WIDGET_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let wrapper: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-person-publication-frames-2/session.json"),
    )?)?;
    let mut results = vec![];
    let mut reports = vec![];
    for resumed in [false, true] {
        eprintln!("Goth panel widgets resumed={resumed}");
        let mut s = Session::restore(&index, serde_json::from_value(wrapper["session"].clone())?)?;
        let mut d = Device::restore(
            &s,
            serde_json::from_value(wrapper["camera_device"].clone())?,
        )?;
        ensure!(d.panel_widgets().is_none());
        let expected = s.simulation.clone();
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
        let owners = Owners {
            panel: 10000,
            state: 10001,
            controls: 10002,
            lot: 10003,
            manager: 10004,
            panels: std::array::from_fn(|i| 10010 + i as u32),
            widgets: std::array::from_fn(|i| 10100 + i as u32),
            pause: 10200,
        };
        d.bind_panel_widgets(&mut s, owners.clone(), 1)?;
        let mut restores = BTreeSet::new();
        for n in 0..100 {
            let p = d.advance_publication(&mut s, if resumed { 1 } else { 100 })?;
            let widget = serde_json::to_value(d.save(&s)?)?;
            let child = &widget["panel_widgets"]["call"]["button"];
            let label =
                if child["pending"] == "Invalidate" && child["setter"].get("Auxiliary").is_some() {
                    Some("auxiliary invalidation before field write")
                } else if child["pending"] == "Changed" {
                    Some("selected field before style callback")
                } else if matches!(
                    d.panel_widgets().unwrap().boundary(),
                    Some(Boundary::Visibility(_))
                ) {
                    Some("panel visibility frontier")
                } else {
                    None
                };
            if resumed {
                if let Some(label) = label.filter(|l| restores.insert(*l)) {
                    eprintln!("cold restore: {label}");
                    let before = serde_json::to_value(s.save())?;
                    ensure!(s.step().is_err());
                    ensure!(d.bind_panel_widgets(&mut s, owners.clone(), 0).is_err());
                    ensure!(before == serde_json::to_value(s.save())?);
                    if label == "selected field before style callback" {
                        for pointer in [
                            "/panel_widgets",
                            "/panel_widgets/call/control",
                            "/panel_widgets/call/buttons",
                        ] {
                            let mut bad = widget.clone();
                            *bad.pointer_mut(pointer).unwrap() = Value::Null;
                            ensure!(
                                Device::restore(&s, serde_json::from_value(bad)?).is_err(),
                                "accepted removed {pointer}"
                            );
                        }
                        let mut bad = before.clone();
                        bad["view"]["publication"]["panels"] = Value::Null;
                        ensure!(Session::restore(&index, serde_json::from_value(bad)?).is_err());
                    }
                    cold(&index, &mut s, &mut d)?;
                }
            }
            if p == Progress::Panels(Some(10000)) {
                ensure!(matches!(
                    d.panel_widgets().unwrap().boundary(),
                    Some(Boundary::Visibility(_))
                ));
                break;
            }
            ensure!(p == Progress::Pending && n < 99);
        }
        ensure!(s.simulation == expected);
        ensure!(s.host.presentation.transport.as_ref().unwrap().controls == cp);
        ensure!(d.mode_window().unwrap().timers() == &timers);
        ensure!(software::render(d.scene()) == pixels);
        ensure!(s.selected_portrait_index()? == 23 && s.publication_pending());
        ensure!(d.panel_widgets().unwrap().pause().selected);
        ensure!(d.panel_widgets().unwrap().pause().auxiliary == 1);
        let result = json!({"session":s.save(),"camera_device":d.save(&s)?});
        for _ in 0..3 {
            ensure!(d.advance_publication(&mut s, 100)? == Progress::Panels(Some(10000)));
            ensure!(result == json!({"session":s.save(),"camera_device":d.save(&s)?}));
        }
        reports.push(json!({"resumed":resumed,"cold_restores":restores,
            "tick":s.simulation.vm.tick,"simulation_unchanged":true,"dirty_unchanged":true,
            "shared_timers_unchanged":true,"selected_person":21,"cache_index":23,
            "pause_selected":true,"pause_auxiliary":1,"frontier":format!("{:?}",d.panel_widgets().unwrap().boundary())}));
        if resumed {
            ensure!(restores.len() == 3);
            std::fs::write(out.join("session.json"), serde_json::to_vec(&result)?)?;
            software::render(d.scene()).save(out.join("goth.png"))?;
        }
        results.push(result);
    }
    ensure!(
        results[0] == results[1],
        "budget/cold restore changed panel publication"
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "scope":"Actual Goth panel/pause-widget prefix; panel visibility virtual callback remains pending",
            "equal":true,"runs":reports
        }))?,
    )?;
    Ok(())
}
