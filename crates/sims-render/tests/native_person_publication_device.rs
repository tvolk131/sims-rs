//!Real Goth current-tool ->4e989b ->disabled music ->panel callback.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Session, publication::Progress};
use sims_vm::interface::world_publication::Request;
use std::{collections::BTreeSet, path::Path};

#[test]
#[ignore = "private original Goth checkpoint; release/headless"]
fn goth_publication_keeps_selected_sim_and_resumes_original_person_child() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("person publication panicked")
}
#[test]
#[ignore = "private historical and completed Goth checkpoints; release"]
fn historical_person_index_is_not_rewritten_and_bound_cpstate_cannot_disappear() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| -> Result<()> {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
            let old: Value = serde_json::from_slice(&std::fs::read(
                root.join("cache/original-current-tool-frames-2/session.json"),
            )?)?;
            let mut s = Session::restore(&index, serde_json::from_value(old["session"].clone())?)?;
            let before = serde_json::to_value(s.save())?;
            ensure!(
                s.host.presentation.publication.as_ref().unwrap().request()
                    == Some(Request::SelectPersonIndex(0))
            );
            ensure!(s.host.presentation.person_selection_override.is_none());
            ensure!(s.bind_person_publication(0).is_err());
            ensure!(before == serde_json::to_value(s.save())?);
            let done: Value = serde_json::from_slice(&std::fs::read(
                root.join("cache/original-person-publication-frames-2/session.json"),
            )?)?;
            let s = Session::restore(&index, serde_json::from_value(done["session"].clone())?)?;
            ensure!(s.selected_portrait_index()? == 23);
            let mut bad = serde_json::to_value(s.save())?;
            bad["view"]["transport"]["controls"]["selected_person_index"] = Value::Null;
            ensure!(Session::restore(&index, serde_json::from_value(bad)?).is_err());
            Ok(())
        })?
        .join()
        .expect("historical person publication panicked")
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
    let out = root.join(std::env::var("SIMS_PERSON_PUBLICATION_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let wrapper: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-wall-publication-frames-1/session.json"),
    )?)?;
    let mut reports = vec![];
    let mut finals = vec![];
    for resumed in [false, true] {
        eprintln!("Goth person publication resumed={resumed}");
        let mut s = Session::restore(&index, serde_json::from_value(wrapper["session"].clone())?)?;
        let mut d = Device::restore(
            &s,
            serde_json::from_value(wrapper["camera_device"].clone())?,
        )?;
        ensure!(
            s.host
                .presentation
                .transport
                .as_ref()
                .unwrap()
                .controls
                .selected_person_index
                .is_none()
        );
        ensure!(s.host.presentation.person_selection_override.is_none());
        ensure!(s.selected_portrait_index()? == 23);
        ensure!(
            s.simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?
                == Some(21)
        );
        s.bind_person_publication(0)?;
        s.bind_current_tools(Some(6000))?;
        let controls = s
            .host
            .presentation
            .transport
            .as_ref()
            .unwrap()
            .controls
            .clone();
        ensure!(controls.selected_person_index == Some(23));
        let pixels = software::render(d.scene());
        let ui = s.host.presentation.camera_ui;
        let mut expected = s.simulation.clone();
        let guid = expected.vm.world.objects[&21]
            .definition
            .as_ref()
            .unwrap()
            .guid;
        ensure!(
            !expected
                .vm
                .world
                .person
                .as_ref()
                .unwrap()
                .resource_instances
                .contains_key(&guid)
        );
        expected
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .resource_instances
            .insert(guid, vec![21]);
        let mut restored = BTreeSet::new();
        if resumed {
            cold(&index, &mut s, &mut d)?;
            restored.insert("CPState index binding");
        }
        for n in 0..1000 {
            let progress = d.advance_publication(&mut s, if resumed { 1 } else { 1000 })?;
            let label = if s
                .simulation
                .native_selection
                .as_ref()
                .is_some_and(|e| e.publication_owner().is_some())
            {
                Some("original person selection child")
            } else if s.host.presentation.publication.as_ref().unwrap().request()
                == Some(Request::SelectPersonIndex(23))
            {
                Some("captured portrait index")
            } else if progress == Progress::Panels(Some(10000)) {
                Some("panel frontier")
            } else {
                None
            };
            if resumed {
                if let Some(label) = label.filter(|l| restored.insert(l)) {
                    eprintln!("restoring {label}");
                    let before = serde_json::to_value(s.save())?;
                    ensure!(s.step().is_err());
                    ensure!(s.bind_person_publication(0).is_err());
                    ensure!(s.begin_selection(None).is_err());
                    ensure!(before == serde_json::to_value(s.save())?);
                    if label == "original person selection child" {
                        for tamper in 0..4 {
                            let mut bad = before.clone();
                            match tamper {
                                0 => bad["view"]["publication"] = Value::Null,
                                1 => {
                                    bad["simulation"]["state"]["native_selection"]["publication"] =
                                        Value::Null
                                }
                                2 => {
                                    bad["simulation"]["state"]["native_selection"]["publication"] =
                                        Value::Null;
                                    bad["view"]["publication"]["person"] = Value::Null;
                                }
                                _ => bad["view"]["publication"]["person"]["index"] = json!(0),
                            }
                            ensure!(
                                Session::restore(&index, serde_json::from_value(bad)?).is_err(),
                                "accepted detached selection child {tamper}"
                            );
                        }
                    }
                    cold(&index, &mut s, &mut d)?;
                }
            }
            if progress == Progress::Panels(Some(10000)) {
                break;
            }
            ensure!(
                progress == Progress::Pending,
                "unexpected callback {progress:?}"
            );
            ensure!(n < 999, "person publication stalled");
        }
        ensure!(
            s.simulation == expected,
            "selection changed more than native resource-instance cache"
        );
        ensure!(s.selected_portrait_index()? == 23 && s.host.presentation.selected == Some(21));
        ensure!(s.host.presentation.transport.as_ref().unwrap().controls == controls);
        ensure!(s.host.presentation.camera_ui == ui && software::render(d.scene()) == pixels);
        ensure!(
            !s.simulation.selection_work_pending()
                && !s.house_notification_pending()
                && s.publication_pending()
        );
        let before = serde_json::to_value(s.save())?;
        for _ in 0..3 {
            ensure!(d.advance_publication(&mut s, 1000)? == Progress::Panels(Some(10000)));
        }
        ensure!(before == serde_json::to_value(s.save())?);
        if resumed {
            ensure!(
                restored.contains("original person selection child")
                    && restored.contains("panel frontier")
            );
            software::render(d.scene()).save(out.join("goth.png"))?;
            std::fs::write(
                out.join("session.json"),
                serde_json::to_vec(&json!({"session":s.save(),"camera_device":d.save(&s)?}))?,
            )?;
        }
        reports.push(json!({"resumed":resumed,"restored":restored,"selected":21,"cache_index":23,
            "frontier":"Panels(10000)","only_simulation_change":"native445857 lazy resource-instance vector",
            "dirty_unchanged":true,"pixels_unchanged":true}));
        finals.push(json!({"session":s.save(),"camera_device":d.save(&s)?}));
    }
    ensure!(
        finals[0] == finals[1],
        "budget/restore changed person publication"
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "scope":"Original4e989b selected-person publication with native445857 cache and445449 child; disabled music gates; actual panel callback pending",
            "runs":reports,"equal":true
        }))?,
    )?;
    Ok(())
}
