use anyhow::{Context, Result, ensure};
use sims_content::{Index, object_catalog::Catalog, object_resources::Overlay};
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};
use sims_sim::edit_history::snapshot::{
    self, CaptureView, References, Registry, lifecycle, live::View, placement,
};
use sims_sim::edit_history::{
    Store,
    command::{self, Method},
    object::{Objects, budget, construction, driver},
};
use std::{cell::RefCell, sync::Arc};

struct Live<'a>(View<'a>);
impl Registry for Live<'_> {
    fn lookup(&self, id: i16) -> Result<Option<u32>, String> {
        Registry::lookup(&self.0, id)
    }
    fn on_lot(&mut self, id: u32) -> Result<bool, String> {
        Registry::on_lot(&mut self.0, id)
    }
}
impl CaptureView for Live<'_> {
    fn normalized_master(&self, o: u32) -> Result<Option<u32>, String> {
        CaptureView::normalized_master(&self.0, o)
    }
    fn script_id(&self, o: u32) -> Result<i16, String> {
        CaptureView::script_id(&self.0, o)
    }
    fn capture(&mut self, o: u32) -> Result<snapshot::Position, String> {
        CaptureView::capture(&mut self.0, o)
    }
    fn definition(&self, o: u32) -> Result<u32, String> {
        CaptureView::definition(&self.0, o)
    }
    fn facing(&self, o: u32) -> Result<i16, String> {
        CaptureView::facing(&self.0, o)
    }
}
impl placement::View for Live<'_> {
    fn facing(&self, o: u32) -> Result<i16, String> {
        CaptureView::facing(&self.0, o)
    }
}
impl lifecycle::View for Live<'_> {
    fn manager(&self) -> Result<u32, String> {
        Err("capture/destruction check must not request a factory manager".into())
    }
}

#[test]
#[ignore = "private original houses, furniture callbacks, and headless rendering; run release"]
fn original_house_command_pickup_preserves_lifetimes_and_cold_resumes() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("pickup test panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_OBJECT_PICKUP_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Arc::new(Index::scan(&root.join("game-data/cabinet-extracted"))?);
    let mut reports = vec![];
    for lot in ["goth", "mashuga"] {
        eprintln!("{lot}: restoring real household");
        let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
            format!("cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"),
        ))?)?;
        let saved: Saved = serde_json::from_value(wrapper["session"].clone())?;
        let neighborhood = format!(
            "{}/Neighborhood.iff",
            saved.source.split('/').next().context("source absent")?
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
        let before = software::render(device.scene());
        before.save(out.join(format!("{lot}-before.png")))?;
        let tick = session.simulation.vm.tick;
        let registry = session.simulation.vm.world.scheduler_storage.clone();
        let people: Vec<_> = session
            .simulation
            .vm
            .world
            .objects
            .iter()
            .filter(|(_, o)| o.person.is_some())
            .map(|(&id, _)| (id, session.simulation.vm.world.positions[&id]))
            .collect();
        let mut targets = vec![];
        for grouped in [false, true] {
            let (&id, o) = session
                .simulation
                .vm
                .world
                .objects
                .iter()
                .find(|(id, o)| {
                    o.person.is_none()
                        && o.definition.as_ref().is_some_and(|d| {
                            d.source
                                .contains(if grouped { "Sofas" } else { "DiningChairs" })
                        })
                        && o.slots.iter().all(Option::is_none)
                        && if grouped {
                            o.multitile.as_ref().is_some_and(|m| m.head.is_some())
                        } else {
                            o.multitile.is_none()
                        }
                        && session
                            .simulation
                            .native_lot
                            .as_ref()
                            .unwrap()
                            .placement
                            .links
                            .capture_raw(&session.simulation.vm.world, **id)
                            .is_ok_and(|d| d.on_lot)
                })
                .context("on-lot furniture target absent")?;
            targets.push((grouped, id, o.definition.clone().unwrap()));
        }
        let mut rows = vec![];
        for (n, (grouped, id, definition)) in targets.into_iter().enumerate() {
            eprintln!("{lot}: capture furniture #{id}, grouped={grouped}");
            let mut refs = References::new(1);
            let mut store = Store::default();
            let mut objects = Objects::default();
            let mut budgets = budget::Budgets::default();
            budgets.0.insert(
                900,
                budget::State {
                    pending: 0,
                    category: 0,
                },
            );
            let root_command = 10000 + n as u32;
            let mut next = 30000;
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
                    kind: 0,
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
            let mut execution = driver::Execution::new(command::Request {
                command: root_command,
                method: Method::Undo,
            })
            .map_err(anyhow::Error::msg)?;
            let mut services = 0;
            let mut cold_count = 0;
            let mut traces = 0;
            let mut graphics = 0;
            let mut removed = vec![];
            let mut reached_end = false;
            for _ in 0..100 {
                let sim = &mut session.simulation;
                let mut view = Live(View {
                    world: &mut sim.vm.world,
                    program: &session.program,
                    links: &sim.native_lot.as_ref().unwrap().placement.links,
                    definition: &resolve,
                });
                let step = execution
                    .advance(
                        &mut store,
                        &mut objects,
                        &mut refs,
                        &mut budgets,
                        &mut view,
                        100,
                    )
                    .map_err(anyhow::Error::msg)?;
                match step {
                    driver::Step::Service(driver::Service::Remove(_)) => {
                        session
                            .simulation
                            .begin_object_command_pickup(&session.program, &execution)
                            .map_err(anyhow::Error::msg)?;
                        let target = session.simulation.object_pickup.as_ref().unwrap().object();
                        let normalized = session
                            .simulation
                            .vm
                            .world
                            .scheduler_storage
                            .as_ref()
                            .unwrap()
                            .object_for_handle(target)
                            .context("target lifetime absent")?;
                        let parts = if grouped {
                            sims_vm::multitile::chain(&session.simulation.vm.world, normalized)
                                .map_err(anyhow::Error::msg)?
                        } else {
                            vec![normalized]
                        };
                        for slice in 0..2000 {
                            ensure!(
                                session
                                    .simulation
                                    .run_current_tick(&session.program, 1)
                                    .is_err()
                            );
                            let step = session
                                .simulation
                                .advance_object_pickup(&session.program, 1)
                                .map_err(anyhow::Error::msg)?;
                            traces += step.trace.len();
                            if session.simulation.pending_removal_graphic().is_some() {
                                let camera =
                                    session.host.presentation.camera.context("camera absent")?;
                                session.service_removal_graphic(camera)?;
                                graphics += 1;
                            }
                            if cold_count == 0
                                && !session
                                    .simulation
                                    .object_pickup
                                    .as_ref()
                                    .unwrap()
                                    .complete()
                            {
                                eprintln!("{lot}: cold restore inside furniture pickup");
                                let camera = device.save(&session)?;
                                let bundle = serde_json::to_vec(&(
                                    session.save(),
                                    &store,
                                    &objects,
                                    &refs,
                                    &budgets,
                                    &execution,
                                ))?;
                                let (saved, s, o, r, b, e) =
                                    serde_json::from_slice::<(
                                        Saved,
                                        Store,
                                        Objects,
                                        References,
                                        budget::Budgets,
                                        driver::Execution,
                                    )>(&bundle)?;
                                session = Session::restore(&index, saved)?;
                                device = Device::restore(&session, camera)?;
                                (store, objects, refs, budgets, execution) = (s, o, r, b, e);
                                cold_count += 1;
                            }
                            if session
                                .simulation
                                .object_pickup
                                .as_ref()
                                .unwrap()
                                .complete()
                            {
                                break;
                            }
                            ensure!(slice < 1999, "pickup did not return");
                        }
                        let mut wrong = driver::Execution::new(command::Request {
                            command: root_command + 1,
                            method: Method::Undo,
                        })
                        .map_err(anyhow::Error::msg)?;
                        ensure!(
                            session
                                .simulation
                                .finish_object_command_pickup(&session.program, &mut wrong)
                                .is_err()
                        );
                        ensure!(
                            session
                                .simulation
                                .object_pickup
                                .as_ref()
                                .unwrap()
                                .complete()
                        );
                        ensure!(session.simulation.take_object_pickup().is_err());
                        session
                            .simulation
                            .finish_object_command_pickup(&session.program, &mut execution)
                            .map_err(anyhow::Error::msg)?;
                        for part in parts {
                            ensure!(session.simulation.vm.world.objects.contains_key(&part));
                            ensure!(
                                !session
                                    .simulation
                                    .native_lot
                                    .as_ref()
                                    .unwrap()
                                    .placement
                                    .links
                                    .capture_raw(&session.simulation.vm.world, part)
                                    .map_err(anyhow::Error::msg)?
                                    .on_lot
                            );
                            if !removed.contains(&part) {
                                removed.push(part);
                            }
                        }
                        services += 1;
                    }
                    driver::Step::Complete(_)
                    | driver::Step::Service(driver::Service::Budget(_)) => {
                        reached_end = true;
                        break;
                    }
                    other => anyhow::bail!("unexpected pickup command service: {other:?}"),
                }
            }
            ensure!(reached_end && services > 0 && !removed.is_empty());
            ensure!(session.simulation.vm.tick == tick);
            ensure!(session.simulation.vm.world.scheduler_storage == registry);
            ensure!(
                session.simulation.deletion.is_empty() && session.simulation.removals.is_empty()
            );
            for (person, at) in &people {
                ensure!(session.simulation.vm.world.positions[person] == *at);
            }
            device.refresh(&session)?;
            let pixels = software::render(device.scene());
            pixels.save(out.join(format!("{lot}-{n}-picked-up.png")))?;
            let camera = device.save(&session)?;
            eprintln!("{lot}: cold restore after physical removal");
            let saved = session.save();
            std::fs::write(
                out.join(format!("{lot}-{n}-session.json")),
                serde_json::to_vec(&saved)?,
            )?;
            session = Session::restore(
                &index,
                serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
            )?;
            device = Device::restore(&session, camera)?;
            ensure!(software::render(device.scene()) == pixels);
            cold_count += 1;
            rows.push(serde_json::json!({"requested":id,"definition":definition,"multipart":grouped,"removed":removed,"services":services,"vm_instructions":traces,"graphic_services":graphics,"full_cold_restores":cold_count,"pixels_changed_from_start":pixels!=before,"cold_restore_pixels_equal":true}));
        }
        ensure!(catalog.borrow().identity().map_err(anyhow::Error::msg)? == catalog_identity);
        reports.push(serde_json::json!({"lot":lot,"tick":tick,"cases":rows,"registry_lifetimes_preserved":true,"people_positions_unchanged":true,"catalog_unchanged":true}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"native_execution":false,"cases":reports,"scope":"Actual original furniture function12 and physical removal through ObjectUndoable; no cursor construction, drop, purchase, normal-play tick progress, or native pixel equivalence claimed."}),
        )?,
    )?;
    Ok(())
}
