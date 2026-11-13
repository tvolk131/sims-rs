//!Real Goth saved Live->Buy call, through native room/drawing/camera children.
//!Stops at the unbound current-tool callback; never clears the dirty word early.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{
    Session,
    publication::{Context as Bindings, Progress},
};
use sims_vm::interface::world_publication::Request;
use std::{collections::BTreeSet, path::Path};

#[test]
#[ignore = "private original Goth checkpoint; release/headless"]
fn buy_publication_runs_original_prefix_and_resumes_at_real_child_boundaries() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("publication panicked")
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
    let out = root.join(std::env::var("SIMS_BUY_PUBLICATION_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let walls: i32 = std::env::var("SIMS_BUY_PUBLICATION_WALLS")
        .unwrap_or_else(|_| "1".into())
        .parse()?;
    ensure!(
        matches!(walls, 1 | 2),
        "unsupported explicit wall test profile"
    );
    let wrapper: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-live-buy-mode-frames-2/session.json"),
    )?)?;
    let mut reports = vec![];
    let mut finals = vec![];
    for resumed in [false, true] {
        eprintln!("Buy publication resumed={resumed}");
        let mut s = Session::restore(&index, serde_json::from_value(wrapper["session"].clone())?)?;
        let mut d = Device::restore(
            &s,
            serde_json::from_value(wrapper["camera_device"].clone())?,
        )?;
        d.bind_hidden_roof(&mut s)?;
        let mut expected_simulation = s.simulation.clone();
        expected_simulation.vm.world.globals.insert(19, 2);
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
        let bindings = Bindings {
            panel_owner: Some(10000),
            closing: 0,
            walls,
            tool_gate: 0,
            current_tool: Some(6000),
            selected_person_index: 0,
        };
        s.begin_interface_publication(bindings.clone(), 0)?;
        let mut restored = BTreeSet::new();
        let mut camera_calls = BTreeSet::new();
        for n in 0..2000 {
            // Leave the real service pending before the device handles it.
            // A second advance repeats the pending request, without replaying
            // its preceding writes or running the callback prematurely.
            let staged = if resumed {
                Some(s.advance_interface_publication(1)?)
            } else {
                None
            };
            let p = s.host.presentation.publication.as_ref().unwrap();
            let request = p.request();
            if let Some(camera) = p.camera_request() {
                camera_calls.insert(format!("{camera:?}"));
            }
            let wall_service = s
                .host
                .presentation
                .camera_command
                .as_ref()
                .and_then(|c| c.pending_camera());
            let label = if matches!(
                wall_service,
                Some(sims_sim::lot_presenter::camera::Service::InvalidateTile(_))
            ) {
                Some("cutaway mask tile")
            } else if request == Some(Request::WallsDown(1))
                && matches!(
                    wall_service,
                    Some(sims_sim::lot_presenter::camera::Service::Floor(
                        sims_sim::lot_presenter::lifecycle::Request::InvalidateView
                    ))
                )
            {
                Some("walls invalidation")
            } else if p.drawing_written().is_some() {
                Some("drawing byte before83 return")
            } else if p.camera_owner().is_some() {
                Some("direct camera child")
            } else if staged == Some(Progress::World(Request::SelectTool(Some(7000)))) {
                Some("tool frontier")
            } else {
                None
            };
            if resumed {
                if let Some(label) = label.filter(|l| restored.insert(l.to_string())) {
                    let before = serde_json::to_value(s.save())?;
                    ensure!(s.step().is_err());
                    ensure!(s.begin_interface_publication(bindings.clone(), 0).is_err());
                    ensure!(
                        s.acknowledge_publication_audio(Request::SelectTool(Some(7000)))
                            .is_err()
                    );
                    ensure!(before == serde_json::to_value(s.save())?);
                    if label == "drawing byte before83 return" {
                        let mut bad = serde_json::to_value(d.save(&s)?)?;
                        bad["drawing_mode"] = json!(0);
                        ensure!(Device::restore(&s, serde_json::from_value(bad)?).is_err());
                    }
                    if label == "direct camera child" {
                        let mut bad = serde_json::to_value(s.save())?;
                        bad["view"]["camera_command"] = Value::Null;
                        ensure!(Session::restore(&index, serde_json::from_value(bad)?).is_err());
                    }
                    eprintln!("restoring {label}: {request:?}");
                    cold(&index, &mut s, &mut d)?;
                }
            }
            let progress = d.advance_publication(&mut s, if resumed { 1 } else { 1000 })?;
            if let Some(camera) = s
                .host
                .presentation
                .publication
                .as_ref()
                .unwrap()
                .camera_request()
            {
                camera_calls.insert(format!("{camera:?}"));
            }
            if progress == Progress::World(Request::SelectTool(Some(7000)))
                && (!resumed || restored.contains("tool frontier"))
            {
                break;
            }
            ensure!(
                matches!(
                    progress,
                    Progress::Pending | Progress::World(Request::SelectTool(Some(7000)))
                ),
                "unexpected service {progress:?}"
            );
            ensure!(n < 1999, "publication stalled");
        }
        ensure!(
            camera_calls.len() == 6,
            "direct camera calls missing: {camera_calls:?}"
        );
        ensure!(
            s.simulation == expected_simulation,
            "publication changed unrelated simulation state"
        );
        ensure!(
            s.host.presentation.camera_ui == ui,
            "direct camera replayed UI setters"
        );
        ensure!(
            s.host.presentation.transport.as_ref().unwrap().controls == controls,
            "dirty flags replaced before world/panels returned"
        );
        ensure!(s.publication_pending() && s.mode_publication_pending());
        ensure!(software::render(d.scene()) == pixels);
        let before = serde_json::to_value(s.save())?;
        for _ in 0..3 {
            ensure!(
                d.advance_publication(&mut s, 1000)?
                    == Progress::World(Request::SelectTool(Some(7000)))
            );
        }
        ensure!(before == serde_json::to_value(s.save())?);
        if resumed {
            ensure!(
                restored.contains("tool frontier")
                    && restored.contains("direct camera child")
                    && restored.contains("drawing byte before83 return")
            );
            if walls == 2 {
                ensure!(
                    restored.contains("cutaway mask tile")
                        && restored.contains("walls invalidation")
                );
            }
            software::render(d.scene()).save(out.join("goth.png"))?;
            std::fs::write(
                out.join("session.json"),
                serde_json::to_vec(&json!({"session":s.save(),"camera_device":d.save(&s)?}))?,
            )?;
        }
        reports.push(
            json!({"resumed":resumed,"restored":restored,"camera_calls":camera_calls,
            "frontier":"SelectTool(7000)","walls":walls,"lot_mode":2,"dirty_unchanged":true,
            "unrelated_simulation_unchanged":true,"pixels_unchanged":true}),
        );
        finals.push(json!({"session":s.save(),"device":d.save(&s)?}));
    }
    ensure!(
        finals[0] == finals[1],
        "budget/cold restore changed publication"
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "scope":"Real Goth4ce390/4ce44f through room, drawing83, camera/walls/cutaway/hidden-roof and global19; current tool and panels still pending",
            "runs":reports,"equal":true
        }))?,
    )?;
    Ok(())
}
