//! Live44123f and440b2d children of the original ObjectUndoable callers.
//! Explicit fixtures derived from the private Goth off-lot chair checkpoint.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session, move_tool};
use sims_sim::{
    edit_history::{object, snapshot},
    object_catalog::move_tool::cleanup,
};
use std::collections::BTreeSet;

#[test]
#[ignore = "private original-house checkpoints; release/headless"]
fn original_history_reference_release_deletes_offlot_furniture_across_cold_saves() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run_delete)?
        .join()
        .expect("history deletion panicked")
}
fn cold(index: &Index, s: &mut Session) -> Result<()> {
    let before = s.save();
    *s = Session::restore(
        index,
        serde_json::from_slice(&serde_json::to_vec(&before)?)?,
    )?;
    ensure!(serde_json::to_value(s.save())? == serde_json::to_value(before)?);
    Ok(())
}
fn run_delete() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_HISTORY_DELETE_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-ui-timer-dispatch-iced/goth/resumed.json"),
    )?)?;
    let camera: sims_render::camera_device::Saved =
        serde_json::from_value(wrapper["camera_device"].clone())?;
    let mut cases = vec![];
    for grouped in [false, true] {
        let seed: Saved = serde_json::from_slice(&std::fs::read(root.join(format!(
            "cache/original-move-undo-frames-1/goth-{grouped}-session.json"
        )))?)?;
        let mut finals = vec![];
        let mut runs = vec![];
        for resumed in [false, true] {
            eprintln!("history deletion grouped={grouped} resumed={resumed}");
            let mut s = Session::restore(&index, seed.clone())?;
            let old = s
                .host
                .presentation
                .move_tool
                .take()
                .context("seed history absent")?;
            let target = if grouped { 173 } else { 198 };
            let parts = if grouped {
                sims_vm::multitile::chain(&s.simulation.vm.world, target)
                    .map_err(anyhow::Error::msg)?
            } else {
                vec![target]
            };
            let handles: Vec<_> = parts
                .iter()
                .map(|id| {
                    s.simulation
                        .vm
                        .world
                        .scheduler_storage
                        .as_ref()
                        .unwrap()
                        .handle(*id)
                        .unwrap()
                })
                .collect();
            //Construct a fresh, sole-owned command against the existing off-lot
            //furniture. Reset releases this command without appending an undo.
            let mut h = old.history;
            h.store = Default::default();
            h.objects = Default::default();
            h.references = snapshot::References::new(1);
            h.budgets = Default::default();
            h.store
                .construct_manager(5001)
                .map_err(anyhow::Error::msg)?;
            let definitions = h.definitions.clone();
            let definition = |d: &sims_vm::Definition| {
                definitions
                    .iter()
                    .find_map(|(&id, v)| (v == d).then_some(id))
                    .ok_or_else(|| "definition absent".to_string())
            };
            let mut allocation = 7002;
            object::construction::construct(
                object::construction::Input {
                    command: 7001,
                    script_id: target,
                    kind: 1,
                    cost: 0,
                    budget: 8001,
                },
                &mut h.store,
                &mut h.objects,
                &mut h.references,
                &mut snapshot::live::View {
                    world: &mut s.simulation.vm.world,
                    program: &s.program,
                    links: &s.simulation.native_lot.as_ref().unwrap().placement.links,
                    definition: &definition,
                },
                &mut || {
                    let id = allocation;
                    allocation += 1;
                    Ok(Some(id))
                },
            )
            .map_err(anyhow::Error::msg)?;
            h.store.acquire(7001).map_err(anyhow::Error::msg)?;
            let mut tool = cleanup::State::constructor_fields(5001);
            tool.payload = Some(7001);
            s.bind_move_tool(move_tool::State::new(tool, old.inputs, h))?;
            s.begin_move_tool_cleanup(cleanup::Kind::Reset)?;
            let tick = s.simulation.vm.tick;
            let rng = s.simulation.vm.rng_state;
            let funds = s.simulation.vm.world.budget.clone();
            let mut device = Device::restore(&s, camera.clone())?;
            let mut boundaries = BTreeSet::new();
            let mut instructions = 0;
            let mut notices = vec![];
            let mut saw_unlinked = false;
            for iteration in 0..10000 {
                let a = s.advance_move_tool(if resumed { 1 } else { 1000 })?;
                instructions += a.instructions;
                if let Some(removal) = &s.simulation.synchronous_removal {
                    let h = &s.host.presentation.move_tool.as_ref().unwrap().history;
                    ensure!(
                        h.references.order.is_empty() && !h.references.records.is_empty(),
                        "reference allocation must survive deletion after catalogue unlink"
                    );
                    saw_unlinked = true;
                    let boundary = if removal.complete() {
                        "complete deletion"
                    } else if !s.simulation.deletion.is_empty() {
                        "destructor"
                    } else {
                        "physical removal"
                    };
                    if resumed && boundaries.insert(boundary) {
                        let mut broken = serde_json::to_value(s.save())?;
                        broken["view"]["move_tool"]["call"]["child"]["History"]["deletion"] =
                            serde_json::Value::Null;
                        ensure!(
                            Session::restore(&index, serde_json::from_value(broken)?).is_err(),
                            "orphan deletion accepted"
                        );
                        cold(&index, &mut s)?;
                    }
                }
                match a.progress {
                    move_tool::Progress::Pending => (),
                    move_tool::Progress::Complete => break,
                    move_tool::Progress::Service(move_tool::Service::Notification(r)) => {
                        notices.push(serde_json::to_value(r)?);
                        device.advance_notification(&mut s, 1)?;
                    }
                    other => anyhow::bail!("unbound history deletion: {other:?}"),
                }
                ensure!(iteration < 9999, "history deletion stalled");
            }
            if resumed {
                ensure!(saw_unlinked && !boundaries.is_empty());
            }
            ensure!(
                parts
                    .iter()
                    .all(|id| !s.simulation.vm.world.objects.contains_key(id))
            );
            ensure!(handles.iter().all(|h| {
                s.simulation
                    .vm
                    .world
                    .scheduler_storage
                    .as_ref()
                    .unwrap()
                    .retired_handle(*h)
            }));
            let h = &s.host.presentation.move_tool.as_ref().unwrap().history;
            ensure!(
                h.references.records.is_empty()
                    && h.references.order.is_empty()
                    && h.objects.0.is_empty()
                    && h.store.commands.is_empty()
            );
            ensure!(
                s.simulation.vm.tick == tick
                    && s.simulation.vm.rng_state == rng
                    && s.simulation.vm.world.budget == funds
            );
            s.finish_move_tool_cleanup()?;
            ensure!(!s.player_input_pending());
            device.refresh(&s)?;
            let image = software::render(device.scene());
            if resumed {
                cold(&index, &mut s)?;
                ensure!(software::render(Device::restore(&s, device.save(&s)?)?.scene()) == image);
                image.save(out.join(format!("goth-{grouped}.png")))?;
                std::fs::write(
                    out.join(format!("goth-{grouped}-session.json")),
                    serde_json::to_vec(&s.save())?,
                )?;
            }
            runs.push(serde_json::json!({"resumed":resumed,"boundaries":boundaries,"instructions":instructions,"notices":notices,"tick":tick}));
            finals.push((serde_json::to_value(s.save())?, image, notices));
        }
        ensure!(
            finals[0] == finals[1],
            "deletion differs across cold resume"
        );
        cases.push(serde_json::json!({"grouped":grouped,"runs":runs,"equal":true}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"cases":cases,"scope":"Original Goth furniture; explicit sole-owned off-lot reference fixtures; live44123f through actual destruction; no normal tick advancement claimed."}),
        )?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private original-house checkpoint; release/headless"]
fn original_history_missing_reference_uses_actual_factory_return_without_rebinding() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| run_factory(false))?
        .join()
        .expect("history factory panicked")
}
#[test]
#[ignore = "private original-house checkpoint; release/headless"]
fn original_history_missing_sofa_reference_runs_multipart_factory_across_cold_saves() -> Result<()>
{
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| run_factory(true))?
        .join()
        .expect("group history factory panicked")
}
fn run_factory(grouped: bool) -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_HISTORY_FACTORY_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
        format!("cache/original-move-undo-frames-1/goth-{grouped}-session.json"),
    ))?)?;
    //Explicit missing-reference fixture, at entry to48c053. The original
    //snapshot ID remains occupied, so440b2d must return a different free ID.
    //Neither the test nor the live bridge publishes a new reference pointer.
    let history = &mut fixture["view"]["move_tool"];
    history["history"]["references"]["records"]["1"]["object"] = serde_json::Value::Null;
    let frame = &mut history["call"]["child"]["History"]["body"]["frames"][0];
    frame["pending"] = serde_json::Value::Null;
    frame["child"]["work"]["Restore"] =
        serde_json::to_value(snapshot::lifecycle::Restore::default())?;
    let seed: Saved = serde_json::from_value(fixture)?;
    let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-ui-timer-dispatch-iced/goth/resumed.json"),
    )?)?;
    let camera: sims_render::camera_device::Saved =
        serde_json::from_value(wrapper["camera_device"].clone())?;
    let mut finals = vec![];
    let mut runs = vec![];
    for resumed in [false, true] {
        eprintln!("history factory grouped={grouped} resumed={resumed}");
        let mut s = Session::restore(&index, seed.clone())?;
        s.host
            .presentation
            .move_tool
            .as_mut()
            .unwrap()
            .history
            .budgets
            .0
            .insert(
                8001,
                object::budget::State {
                    pending: 0,
                    category: 0,
                },
            );
        let tick = s.simulation.vm.tick;
        let before_ids: BTreeSet<_> = s.simulation.vm.world.objects.keys().copied().collect();
        let expected_new = (1..=i16::MAX).find(|id| !before_ids.contains(id)).unwrap();
        let target = if grouped { 173 } else { 198 };
        ensure!(expected_new != target);
        let mut device = Device::restore(&s, camera.clone())?;
        let mut restored = BTreeSet::new();
        let mut notices = vec![];
        let mut saw_created = false;
        for iteration in 0..10000 {
            let a = s.advance_move_tool(if resumed { 1 } else { 1000 })?;
            if let Some(e) = &s.simulation.native_selection {
                ensure!(e.direct_factory().is_some());
                let nested_hdsi = s.simulation.vm.threads.values().any(|t| {
                    matches!(&t.status, sims_vm::Status::AwaitingCreation(r)
                        if r.operands[..4] == 0xfd4fb8e4_u32.to_le_bytes() && r.created.is_some())
                });
                let boundary = match s.simulation.pending_native_selection() {
                    Some((sims_sim::lot_selection::Request::FactoryGraphic(id), _)) => {
                        format!("graphics {id}")
                    }
                    Some((sims_sim::lot_selection::Request::CreationNotification(_), _)) => {
                        saw_created = true;
                        "created notification".to_string()
                    }
                    _ if nested_hdsi => "nested HDSI".to_string(),
                    _ if s.simulation.vm.world.objects.contains_key(&expected_new)
                        && !s.simulation.vm.object_threads.contains_key(&expected_new) =>
                    {
                        format!(
                            "registered {} parts",
                            s.simulation
                                .vm
                                .world
                                .objects
                                .keys()
                                .filter(|id| !before_ids.contains(id))
                                .count()
                        )
                    }
                    _ if s.simulation.vm.world.objects.contains_key(&expected_new) => {
                        "Init/Main".to_string()
                    }
                    _ => "factory entry".to_string(),
                };
                if resumed && restored.insert(boundary) {
                    let mut broken = serde_json::to_value(s.save())?;
                    broken["view"]["move_tool"]["call"]["child"]["History"]["factory"] =
                        serde_json::Value::Null;
                    ensure!(
                        Session::restore(&index, serde_json::from_value(broken)?).is_err(),
                        "orphan factory accepted"
                    );
                    cold(&index, &mut s)?;
                }
            }
            for r in s
                .host
                .presentation
                .move_tool
                .as_ref()
                .unwrap()
                .history
                .references
                .records
                .values()
            {
                ensure!(r.object.is_none(), "factory rebound the undo reference");
            }
            ensure!(
                s.simulation.object_placement.is_none(),
                "mismatched ID reached placement"
            );
            match a.progress {
                move_tool::Progress::Pending => (),
                move_tool::Progress::Complete => break,
                move_tool::Progress::Service(move_tool::Service::Notification(r)) => {
                    notices.push(serde_json::to_value(r)?);
                    device.advance_notification(&mut s, 1)?;
                }
                other => anyhow::bail!("unbound history factory: {other:?}"),
            }
            ensure!(iteration < 9999, "history factory stalled");
        }
        ensure!(s.simulation.vm.tick == tick);
        if resumed {
            ensure!(saw_created && restored.contains("created notification"));
        }
        let new_ids: Vec<_> = s
            .simulation
            .vm
            .world
            .objects
            .keys()
            .filter(|id| !before_ids.contains(id))
            .copied()
            .collect();
        std::fs::write(
            out.join(format!("factory-{resumed}-observed.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "new_objects":new_ids.iter().map(|id| (*id,&s.simulation.vm.world.objects[id].definition,&s.simulation.vm.world.objects[id].multitile)).collect::<Vec<_>>(),
                "expected_new":expected_new,
                "target":target,
                "references":s.host.presentation.move_tool.as_ref().unwrap().history.references,
            }))?,
        )?;
        ensure!(
            new_ids
                == (1..=i16::MAX)
                    .filter(|id| !before_ids.contains(id))
                    .take(if grouped { 3 } else { 1 })
                    .collect::<Vec<_>>(),
            "factory return was guessed or unexpected object removed: {new_ids:?}"
        );
        ensure!(
            s.simulation.vm.world.objects[&expected_new].definition
                == s.simulation.vm.world.objects[&target].definition
        );
        if grouped {
            //The original furniture Init calls Global400, instruction1:
            //Create Object FD4FB8E4 in mode6. That HDSI group-talk controller
            //is a separate object, not another static furniture part.
            let create = &s.program.modules[&s.program.globals].routines[&400]
                .body
                .instructions[1];
            ensure!(
                create.opcode == 0x2a && create.operands == [0xe4, 0xb8, 0x4f, 0xfd, 6, 0, 0, 0]
            );
            let controller = &s.simulation.vm.world.objects[&new_ids[2]];
            ensure!(
                controller
                    .definition
                    .as_ref()
                    .is_some_and(|d| d.guid == 0xfd4fb8e4)
                    && controller.multitile.is_none()
            );
            ensure!(
                sims_vm::multitile::chain(&s.simulation.vm.world, expected_new)
                    .map_err(anyhow::Error::msg)?
                    == new_ids[..2]
            );
            if resumed {
                ensure!(
                    restored.contains("registered 1 parts")
                        && restored.contains("registered 2 parts")
                        && restored.contains("nested HDSI")
                );
            }
        }
        s.finish_move_tool_cleanup()?;
        device.refresh(&s)?;
        let image = software::render(device.scene());
        if resumed {
            cold(&index, &mut s)?;
            ensure!(software::render(Device::restore(&s, device.save(&s)?)?.scene()) == image);
            image.save(out.join("goth.png"))?;
            std::fs::write(
                out.join("goth-session.json"),
                serde_json::to_vec(&s.save())?,
            )?;
        }
        runs.push(serde_json::json!({"resumed":resumed,"restored":restored,"actual_created_id":expected_new,"reference_id":target,"created_objects":new_ids,"notices":notices,"tick":tick}));
        finals.push((serde_json::to_value(s.save())?, image, notices));
    }
    ensure!(finals[0] == finals[1], "factory differs across cold resume");
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"grouped":grouped,"runs":runs,"equal":true,"scope":"Original chair/sofa factory scripts and E2 listeners under explicit missing-reference fixture; preserves failed ID comparison and unbound reference; no normal tick advancement claimed."}),
        )?,
    )?;
    Ok(())
}
