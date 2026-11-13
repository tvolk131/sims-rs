use anyhow::{Context, Result, ensure};
use sims_content::{Index, object_catalog::Catalog, object_resources::Overlay};
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};
use sims_sim::edit_history::snapshot::{References, live::View};
use sims_sim::edit_history::{
    Store,
    command::{self, Method},
    manager,
    object::{Objects, budget, construction, driver},
};
use std::{cell::RefCell, sync::Arc};

#[path = "support/object_command_live.rs"]
mod object_command_live;
use object_command_live::CaptureOnly as Live;

#[test]
#[ignore = "private original Goth/Mashuga houses; real capture, catalogue identity and headless pixels"]
fn original_house_object_commands_capture_execute_and_destroy_without_vm_drift() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_OBJECT_COMMANDS_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Arc::new(Index::scan(&root.join("game-data/cabinet-extracted"))?);
    let mut reports = vec![];
    for lot in ["goth", "mashuga"] {
        let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
            format!("cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"),
        ))?)?;
        let saved: Saved = serde_json::from_value(wrapper["session"].clone())?;
        let neighborhood = format!(
            "{}/Neighborhood.iff",
            saved
                .source
                .split('/')
                .next()
                .context("house source directory absent")?
        );
        let catalog = RefCell::new(Catalog::household(
            index.clone(),
            &neighborhood,
            Overlay::default(),
        )?);
        let catalog_identity = catalog.borrow().identity().map_err(anyhow::Error::msg)?;
        let resolve = |d: &sims_vm::Definition| catalog.borrow_mut().live_dialog_resource(d);
        let mut session = Session::restore(&index, saved)?;
        let mut device = Device::restore(
            &session,
            serde_json::from_value(wrapper["camera_device"].clone())?,
        )?;
        let pixels = software::render(device.scene());
        let before = session.simulation.clone();
        let mut expected = before.clone();
        let mut targets = vec![];
        for multipart in [false, true] {
            let (&id, _) = before
                .vm
                .world
                .objects
                .iter()
                .filter(|(id, o)| {
                    o.person.is_none()
                        && o.definition.is_some()
                        && o.data.contains_key(&1)
                        && before
                            .native_lot
                            .as_ref()
                            .unwrap()
                            .placement
                            .links
                            .capture_raw(&before.vm.world, **id)
                            .is_ok_and(|d| d.on_lot)
                        && if multipart {
                            o.multitile.as_ref().is_some_and(|m| m.head.is_some())
                                && sims_vm::multitile::dynamic(
                                    &before.vm.world,
                                    &session.program,
                                    **id,
                                ) == Ok(false)
                        } else {
                            o.multitile.is_none()
                        }
                })
                .max_by_key(|(_, o)| o.slots.iter().filter(|s| s.is_some()).count())
                .context("required on-lot snapshot target absent")?;
            targets.push((multipart, id));
        }
        let mut refs = References::new(1);
        let mut store = Store::default();
        let mut objects = Objects::default();
        let mut budgets = budget::Budgets::default();
        let mut next = 30000;
        let mut roots = vec![];
        let mut rows = vec![];
        for (n, (multipart, id)) in targets.into_iter().enumerate() {
            let root_command = 10000 + n as u32;
            roots.push(root_command);
            let sim = &mut session.simulation;
            let mut view = View {
                world: &mut sim.vm.world,
                program: &session.program,
                links: &sim.native_lot.as_ref().unwrap().placement.links,
                definition: &resolve,
            };
            construction::construct(
                construction::Input {
                    command: root_command,
                    script_id: id,
                    kind: 2,
                    cost: 0,
                    budget: 900,
                },
                &mut store,
                &mut objects,
                &mut refs,
                &mut view,
                &mut || {
                    next += 1;
                    Ok(Some(next))
                },
            )
            .map_err(anyhow::Error::msg)?;
            rows.push(serde_json::json!({"multipart":multipart,"requested":id,"root_command":root_command,"initial":objects.get(root_command).map_err(anyhow::Error::msg)?.snapshots[0]}));
        }
        let command_count = objects.0.len();
        for reference in refs.records.values() {
            let id = expected
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .unwrap()
                .object_for_handle(reference.object.context("captured object missing")?)
                .context("captured lifetime missing")?;
            expected
                .native_lot
                .as_ref()
                .unwrap()
                .placement
                .links
                .capture(&mut expected.vm.world, id)
                .map_err(anyhow::Error::msg)?;
        }
        ensure!(
            session.simulation == expected,
            "constructor drift beyond native physical capture"
        );
        for &root_command in &roots {
            let mut execution = driver::Execution::new(command::Request {
                command: root_command,
                method: Method::Execute,
            })
            .map_err(anyhow::Error::msg)?;
            loop {
                let sim = &mut session.simulation;
                let mut view = Live(View {
                    world: &mut sim.vm.world,
                    program: &session.program,
                    links: &sim.native_lot.as_ref().unwrap().placement.links,
                    definition: &resolve,
                });
                match execution
                    .advance(
                        &mut store,
                        &mut objects,
                        &mut refs,
                        &mut budgets,
                        &mut view,
                        1,
                    )
                    .map_err(anyhow::Error::msg)?
                {
                    driver::Step::Budget => {
                        (execution, store, objects, refs, budgets) = serde_json::from_slice(
                            &serde_json::to_vec(&(execution, store, objects, refs, budgets))?,
                        )?;
                    }
                    driver::Step::Complete(value) => {
                        ensure!(value == 0);
                        break;
                    }
                    service => anyhow::bail!("unexpected execute service: {service:?}"),
                }
            }
        }
        ensure!(
            session.simulation == expected
                && refs
                    .records
                    .values()
                    .all(|r| r.count >= 2 && r.count % 2 == 0)
        );
        let captures = objects
            .0
            .values()
            .filter(|o| o.snapshots[0].active != 0)
            .count()
            * 2;
        let bytes = serde_json::to_vec(&(
            session.save(),
            device.save(&session)?,
            &store,
            &objects,
            &refs,
            &budgets,
        ))?;
        let (saved, saved_device, cold_store, cold_objects, cold_refs, cold_budgets): (
            Saved,
            _,
            Store,
            Objects,
            References,
            budget::Budgets,
        ) = serde_json::from_slice(&bytes)?;
        session = Session::restore(&index, saved)?;
        device = Device::restore(&session, saved_device)?;
        store = cold_store;
        objects = cold_objects;
        refs = cold_refs;
        budgets = cold_budgets;
        ensure!(session.simulation == expected && software::render(device.scene()) == pixels);
        for &root_command in &roots {
            store.acquire(root_command).map_err(anyhow::Error::msg)?;
            let mut history = sims_sim::edit_history::tool::Call::new(
                sims_sim::object_catalog::move_tool::cleanup::Request::ReleasePayload(root_command),
            )
            .map_err(anyhow::Error::msg)?
            .context("history release absent")?;
            let request = command::Request {
                command: root_command,
                method: Method::Destroy,
            };
            ensure!(
                history.advance(&mut store).map_err(anyhow::Error::msg)?
                    == Some(manager::Service::Virtual(request))
            );
            let mut execution = driver::Execution::new(request).map_err(anyhow::Error::msg)?;
            loop {
                let sim = &mut session.simulation;
                let mut view = Live(View {
                    world: &mut sim.vm.world,
                    program: &session.program,
                    links: &sim.native_lot.as_ref().unwrap().placement.links,
                    definition: &resolve,
                });
                match execution
                    .advance(
                        &mut store,
                        &mut objects,
                        &mut refs,
                        &mut budgets,
                        &mut view,
                        1,
                    )
                    .map_err(anyhow::Error::msg)?
                {
                    driver::Step::Budget => {
                        (execution, store, objects, refs, budgets) = serde_json::from_slice(
                            &serde_json::to_vec(&(execution, store, objects, refs, budgets))?,
                        )?;
                    }
                    driver::Step::Complete(value) => {
                        history
                            .acknowledge(manager::Service::Virtual(request), value)
                            .map_err(anyhow::Error::msg)?;
                        break;
                    }
                    service => anyhow::bail!("unexpected destructor service: {service:?}"),
                }
            }
            ensure!(
                history
                    .advance(&mut store)
                    .map_err(anyhow::Error::msg)?
                    .is_none()
                    && history.complete()
            );
        }
        store.validate().map_err(anyhow::Error::msg)?;
        refs.validate().map_err(anyhow::Error::msg)?;
        ensure!(store.commands.is_empty() && objects.0.is_empty() && refs.records.is_empty());
        ensure!(
            session.simulation == expected,
            "object command changed VM beyond capture normalization"
        );
        device.refresh(&session)?;
        let after = software::render(device.scene());
        ensure!(after == pixels);
        ensure!(catalog.borrow().identity().map_err(anyhow::Error::msg)? == catalog_identity);
        after.save(out.join(format!("{lot}.png")))?;
        reports.push(serde_json::json!({"lot":lot,"tick":session.simulation.vm.tick,"targets":rows,"command_count":command_count,"captures":captures,
            "constructor_and_recursive_virtuals_executed":true,"history_release_executed":true,"catalog_unchanged":true,"vm_matches_native_capture_only":true,
            "cold_restore_pixels_equal":true,"pixels_unchanged":true,"all_command_storage_released":true}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"native_execution":false,"cases":reports,
        "scope":"Construct ObjectUndoable from actual Goth/Mashuga ordinary and multipart receivers, execute original-shaped recursive captures, cold restore, and destroy through history release. No object movement/purchase, full Buy frontend, or original pixels claimed."}),
        )?,
    )?;
    Ok(())
}
