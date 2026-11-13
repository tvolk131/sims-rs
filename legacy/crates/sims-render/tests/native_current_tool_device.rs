//!Continue the real saved Goth4ce44f call through current-tool publication.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Session, publication::Progress};
use sims_vm::interface::{current_tool::Request, world_publication::Request as World};
use std::{collections::BTreeSet, path::Path};

#[test]
#[ignore = "private Goth publication checkpoint; release/headless"]
fn goth_current_tool_executes_real_callbacks_and_resumes_without_advancing_simulation() -> Result<()>
{
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("tool publication panicked")
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
    let out = root.join(std::env::var("SIMS_CURRENT_TOOL_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let wrapper: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-wall-publication-frames-1/session.json"),
    )?)?;
    let mut reports = vec![];
    let mut finals = vec![];
    for resumed in [false, true] {
        eprintln!("Goth current tool resumed={resumed}");
        let mut s = Session::restore(&index, serde_json::from_value(wrapper["session"].clone())?)?;
        let mut d = Device::restore(
            &s,
            serde_json::from_value(wrapper["camera_device"].clone())?,
        )?;
        ensure!(
            s.host.presentation.publication.as_ref().unwrap().request()
                == Some(World::SelectTool(Some(7000)))
        );
        let expected_simulation = s.simulation.clone();
        let ui = s.host.presentation.camera_ui;
        let controls = s
            .host
            .presentation
            .transport
            .as_ref()
            .unwrap()
            .controls
            .clone();
        let pixels = software::render(d.scene());
        s.bind_current_tools(Some(6000))?;
        let mut restored = BTreeSet::new();
        for n in 0..1000 {
            let progress = d.advance_publication(&mut s, if resumed { 1 } else { 1000 })?;
            let parent = s.host.presentation.publication.as_ref().unwrap().request();
            let tool = s.host.presentation.current_tool.as_ref().unwrap();
            let label = if parent == Some(World::SelectTool(Some(7000)))
                && tool.request() == Some(Request::Enter(7000))
                && tool.move_text_loaded()
                && !s.move_tool_pending()
            {
                Some("resource and current global before cursor")
            } else if tool.request() == Some(Request::Enter(7000)) && s.move_tool_pending() {
                Some("Move cursor child")
            } else if matches!(parent, Some(World::ActivateTool { .. })) && s.move_tool_pending() {
                Some("Move activation child")
            } else if progress == Progress::World(World::SelectPersonIndex(0)) {
                Some("selection frontier")
            } else {
                None
            };
            if resumed {
                if let Some(label) = label.filter(|l| restored.insert(l.to_string())) {
                    let before = serde_json::to_value(s.save())?;
                    ensure!(s.step().is_err());
                    ensure!(s.bind_current_tools(Some(6000)).is_err());
                    ensure!(before == serde_json::to_value(s.save())?);
                    if label != "selection frontier" {
                        let mut bad = before.clone();
                        bad["view"]["current_tool"] = Value::Null;
                        ensure!(Session::restore(&index, serde_json::from_value(bad)?).is_err());
                    }
                    if label == "Move cursor child" {
                        let mut bad = before.clone();
                        bad["view"]["move_tool"]["call"]["current"] = Value::Null;
                        ensure!(Session::restore(&index, serde_json::from_value(bad)?).is_err());
                        let mut bad = before;
                        bad["view"]["move_tool"]["call"]["current"] = Value::Null;
                        bad["view"]["current_tool"]["call"]["child"] = Value::Null;
                        ensure!(Session::restore(&index, serde_json::from_value(bad)?).is_err());
                    }
                    eprintln!("restoring {label}");
                    cold(&index, &mut s, &mut d)?;
                }
            }
            if progress == Progress::World(World::SelectPersonIndex(0)) {
                break;
            }
            ensure!(
                progress == Progress::Pending,
                "unexpected callback {progress:?}"
            );
            ensure!(n < 999, "current tool stalled");
        }
        ensure!(
            s.simulation == expected_simulation,
            "tool publication changed simulation"
        );
        ensure!(s.host.presentation.camera_ui == ui);
        ensure!(s.host.presentation.transport.as_ref().unwrap().controls == controls);
        ensure!(s.host.presentation.current_tool.as_ref().unwrap().current() == Some(7000));
        ensure!(
            s.host
                .presentation
                .current_tool
                .as_ref()
                .unwrap()
                .move_text_loaded()
        );
        ensure!(!s.move_tool_pending() && !s.tool_selection_pending() && s.publication_pending());
        ensure!(software::render(d.scene()) == pixels);
        let before = serde_json::to_value(s.save())?;
        for _ in 0..3 {
            ensure!(
                d.advance_publication(&mut s, 1000)?
                    == Progress::World(World::SelectPersonIndex(0))
            );
        }
        ensure!(before == serde_json::to_value(s.save())?);
        if resumed {
            ensure!(restored.len() == 4, "missing tool boundaries: {restored:?}");
            software::render(d.scene()).save(out.join("goth.png"))?;
            std::fs::write(
                out.join("session.json"),
                serde_json::to_vec(&json!({"session":s.save(),"camera_device":d.save(&s)?}))?,
            )?;
        }
        reports.push(json!({"resumed":resumed,"restored":restored,"current":7000,
            "frontier":"SelectPersonIndex(0)","original_move_text_loaded":true,
            "simulation_unchanged":true,"dirty_unchanged":true,"pixels_unchanged":true}));
        finals.push(json!({"session":s.save(),"camera_device":d.save(&s)?}));
    }
    ensure!(
        finals[0] == finals[1],
        "budget/cold restore changed current-tool publication"
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "scope":"Actual Goth476a35 old picker reset, Move UIText137 initialization/cursor tail, viewer listeners and4ce44f Move activation; selected-person index, music and panels remain pending",
            "runs":reports,"equal":true
        }))?,
    )?;
    Ok(())
}
