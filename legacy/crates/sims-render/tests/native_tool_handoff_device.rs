//!Original catalog/house through live interface and concrete tool callbacks.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{
    Saved, Session,
    tool_selection::{self, Receiver},
};
use sims_vm::interface::tool::Fields;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[test]
#[ignore = "private original-house checkpoint; release/headless"]
fn original_house_interface_tool_handoff_runs_both_activations_and_restores_owned_callbacks()
-> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("tool handoff panicked")
}
fn cold(index: &Index, s: &mut Session, d: &mut Device) -> Result<()> {
    let saved = s.save();
    let surface = d.save(s)?;
    let pixels = software::render(d.scene());
    *s = Session::restore(index, serde_json::from_slice(&serde_json::to_vec(&saved)?)?)?;
    *d = Device::restore(s, serde_json::from_slice(&serde_json::to_vec(&surface)?)?)?;
    ensure!(serde_json::to_value(saved)? == serde_json::to_value(s.save())?);
    ensure!(surface == d.save(s)? && pixels == software::render(d.scene()));
    Ok(())
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_TOOL_HANDOFF_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let seed: Saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-live-move-activation-frames-1/session.json"),
    )?)?;
    let wrapper: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-ui-timer-dispatch-iced/goth/resumed.json"),
    )?)?;
    let mut camera = wrapper["camera_device"].clone();
    camera["drawing_mode"] = json!(5);
    let mut finals = vec![];
    let mut runs = vec![];
    for resumed in [false, true] {
        eprintln!("interface tool handoff resumed={resumed}");
        let mut s = Session::restore(&index, seed.clone())?;
        let h = &s.host.presentation.move_tool.as_ref().unwrap().history;
        let (&part, _) = h
            .definitions
            .iter()
            .find(|(_, d)| d.guid == 0xd5ca2d74)
            .context("bench part absent")?;
        let (&master, _) = h
            .definitions
            .iter()
            .find(|(_, d)| d.guid == 0x7cb11019)
            .context("bench master absent")?;
        ensure!(h.definitions.values().all(|d| d.guid != 0xdeadbeef));
        let mut slots = [None; 12];
        slots[6] = Some(6000);
        slots[7] = Some(7000);
        let mut state = tool_selection::State::new(
            slots,
            BTreeMap::from([
                (6000, Receiver::Picker { mode: Some(9) }),
                (7000, Receiver::Move),
            ]),
            Fields {
                active: Some(6000),
                argument: 42,
            },
        );
        state.retain_catalog_item(9001, part)?;
        s.bind_tool_selection(state)?;
        let initial = s.simulation.clone();
        let mut d = Device::restore(&s, serde_json::from_value(camera.clone())?)?;
        let initial_pixels = software::render(d.scene());
        let mut restores = BTreeSet::new();
        let mut observed = vec![];
        let mut serials = BTreeSet::new();
        let mut result_fields = vec![];
        for operation in 0..9 {
            match operation {
                0 => s.begin_indexed_tool(7, (-20000_i32) as u32)?,
                1 => s.begin_tool_replacement(Some(7000), part - 1)?,
                2 => s.begin_buy_definition(0xd5ca2d74)?,
                3 => {
                    s.host
                        .presentation
                        .transport
                        .as_mut()
                        .unwrap()
                        .controls
                        .buy
                        .selected_object = 9001;
                    s.begin_buy_tool(3)?;
                }
                4 => s.begin_buy_definition(0xdeadbeef)?,
                5 => s.begin_buy_tool(0)?,
                6 => s.begin_buy_tool(1)?,
                7 => s.begin_indexed_tool(6, 0x10001)?,
                8 => s.begin_tool_replacement(None, 0)?,
                _ => unreachable!(),
            }
            let start = serde_json::to_value(s.save())?;
            ensure!(
                s.step().is_err()
                    && s.begin_indexed_tool(7, 0).is_err()
                    && s.begin_move_tool_activation(0).is_err()
            );
            ensure!(serde_json::to_value(s.save())? == start);
            for n in 0..1000 {
                let progress = s.advance_tool_selection(if resumed { 1 } else { 1000 })?;
                let value =
                    serde_json::to_value(s.host.presentation.tool_selection.as_ref().unwrap())?;
                let child = &value["call"]["child"];
                if resumed && !child.is_null() {
                    let serial = child["serial"].as_u64().unwrap();
                    if serials.insert(serial) {
                        observed.push(
                            json!({"operation":operation,"child":child,"fields":value["fields"]}),
                        );
                    }
                    let label = if operation == 0 {
                        Some(format!("indexed callback{serial}"))
                    } else if operation == 2 && child["operation"]["Cleanup"] == "Reset" {
                        Some("definition reset".into())
                    } else {
                        None
                    };
                    if let Some(label) = label.filter(|label| restores.insert(label.clone())) {
                        if operation == 0 && serial == 1 {
                            let mut orphan = serde_json::to_value(s.save())?;
                            orphan["view"]["tool_selection"] = Value::Null;
                            ensure!(
                                Session::restore(&index, serde_json::from_value(orphan)?).is_err()
                            );
                            let mut mismatch = serde_json::to_value(s.save())?;
                            mismatch["view"]["tool_selection"]["call"]["child"]["serial"] =
                                json!(999);
                            ensure!(
                                Session::restore(&index, serde_json::from_value(mismatch)?)
                                    .is_err()
                            );
                        }
                        eprintln!("restoring {label}");
                        cold(&index, &mut s, &mut d)?;
                    }
                }
                match progress {
                    tool_selection::Progress::Complete => {
                        s.finish_tool_selection()?;
                        d.refresh(&s)?;
                        break;
                    }
                    tool_selection::Progress::Pending => (),
                    tool_selection::Progress::Service(tool_selection::Service::DrawingMode(0)) => {
                        let before = serde_json::to_value(s.save())?;
                        ensure!(s.acknowledge_tool_drawing(1).is_err());
                        ensure!(serde_json::to_value(s.save())? == before);
                        if resumed
                            && operation == 0
                            && restores.insert("picker deactivation".into())
                        {
                            cold(&index, &mut s, &mut d)?;
                        }
                        ensure!(!d.advance_tool_selection(&mut s, 1)?);
                        ensure!(s.acknowledge_tool_drawing(0).is_err());
                        ensure!(serde_json::to_value(d.save(&s)?)?["drawing_mode"] == 0);
                    }
                    other => anyhow::bail!("unbound interface callback: {other:?}"),
                }
                ensure!(n < 999, "tool handoff stalled");
            }
            let t = s.host.presentation.tool_selection.as_ref().unwrap();
            let m = &s.host.presentation.move_tool.as_ref().unwrap().tool.tool;
            match operation {
                0 => ensure!(
                    t.fields().active == Some(7000)
                        && t.fields().argument == (-20000_i32) as u32
                        && m.mode == 3
                ),
                1 => ensure!(
                    t.fields().active == Some(7000)
                        && t.fields().argument == part - 1
                        && m.definition == Some(part)
                ),
                2 | 3 => ensure!(m.definition == Some(master) && t.fields().argument == part - 1),
                4 | 5 => ensure!(
                    t.fields().active == Some(7000)
                        && t.fields().argument == u32::MAX
                        && m.definition.is_none()
                ),
                6 | 8 => ensure!(t.fields().active.is_none() && t.fields().argument == 0),
                7 => ensure!(
                    t.fields().active == Some(6000)
                        && t.fields().argument == 0x10001
                        && t.receivers()[&6000] == Receiver::Picker { mode: Some(2) }
                ),
                _ => unreachable!(),
            }
            ensure!(s.simulation == initial && software::render(d.scene()) == initial_pixels);
            result_fields.push(json!({"interface":t.fields(),"move":m}));
        }
        if resumed {
            ensure!(
                restores.contains("indexed callback1")
                    && restores.contains("indexed callback2")
                    && restores.contains("definition reset")
                    && restores.contains("picker deactivation")
            );
            let first = &observed[0];
            let second = &observed[1];
            ensure!(first["fields"]["active"] == 6000 && second["fields"]["active"] == 7000);
            cold(&index, &mut s, &mut d)?;
            restores.insert("complete".into());
            initial_pixels.save(out.join("goth.png"))?;
            std::fs::write(
                out.join("session.json"),
                serde_json::to_vec(&json!({"session":s.save(),"camera_device":d.save(&s)?}))?,
            )?;
        }
        runs.push(json!({"resumed":resumed,"restored":restores,"callbacks":observed,"results":result_fields,"operations":9,"simulation_unchanged":true,"pixels_unchanged":true}));
        finals.push((
            serde_json::to_value(s.save())?,
            d.save(&s)?,
            software::render(d.scene()),
        ));
    }
    ensure!(
        finals[0] == finals[1],
        "tool handoff differs across budgets/restores"
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({"runs":runs,"equal":true,
        "scope":"Actual Goth/catalog through interface tool selection, concrete callbacks and backend drawing byte. Explicit application bindings; full mode change, purchasing and tick progression remain separate."}))?,
    )?;
    Ok(())
}
