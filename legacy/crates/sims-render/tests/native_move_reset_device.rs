//!Live487105/487191 over original-house cursor instances; supplied application
//!tool/history context is a regression fixture, not imported suspended Windows UI.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session, move_tool};
use sims_sim::{
    edit_history::{self, object, snapshot},
    object_catalog::move_tool::cleanup,
};

#[test]
#[ignore = "private original-house checkpoints; release/headless"]
fn original_house_move_reset_and_cold_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("move reset test panicked")
}

#[test]
#[ignore = "private original-house checkpoints; release/headless"]
fn original_house_reset_releases_tail_payload_without_advancing_time() -> Result<()> {
    std::thread::Builder::new().stack_size(32 * 1024 * 1024).spawn(|| -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
        let saved: Saved = serde_json::from_slice(&std::fs::read(root.join(
            "cache/original-cursor-destruction-frames-1/goth-false-session.json"))?)?;
        let mut finals = vec![];
        let mut reports = vec![];
        for budget in [1, 1000] {
            let mut s = Session::restore(&index, saved.clone())?;
            let initial = s.simulation.clone();
            let mut history = move_tool::History::new(1);
            history.store.construct_manager(5001).map_err(anyhow::Error::msg)?;
            object::construction::construct(object::construction::Input {
                command: 7002, script_id: 0, kind: 1, cost: 0, budget: 8001,
            }, &mut history.store, &mut history.objects, &mut history.references,
                &mut snapshot::live::View {
                    world: &mut s.simulation.vm.world, program: &s.program,
                    links: &s.simulation.native_lot.as_ref().unwrap().placement.links,
                    definition: &|_| Err("null capture must not read a definition".into()),
                }, &mut || Err("null capture must not allocate children".into())).map_err(anyhow::Error::msg)?;
            history.store.acquire(7002).map_err(anyhow::Error::msg)?;
            let mut tool = cleanup::State::constructor_fields(5001);
            tool.payload = Some(7002);
            s.bind_move_tool(move_tool::State::new(tool, cleanup::Inputs {
                registry: 5002, window: 0, key_function: 0, control_state: 0,
            }, history))?;
            s.begin_move_tool_cleanup(cleanup::Kind::Reset)?;
            let mut retired_restore = false;
            for n in 0..1000 {
                let step = s.advance_move_tool(budget)?;
                ensure!(step.instructions == 0);
                let t = s.host.presentation.move_tool.as_ref().unwrap();
                if budget == 1 && t.tool.payload == Some(7002) && !t.history.store.commands.contains_key(&7002) && !retired_restore {
                    let before = s.save();
                    //A retired payload is legal only with the retained exact
                    //Release continuation; arbitrary missing storage is rejected.
                    let mut bad = serde_json::to_value(&before)?;
                    let move_state = &mut bad["view"]["move_tool"];
                    ensure!(!move_state.is_null(), "saved move host path changed");
                    move_state["call"]["retired_payload"] = serde_json::Value::Null;
                    if !move_state["call"]["child"]["History"].is_null() {
                        move_state["call"]["child"]["History"]["retired"] = serde_json::Value::Null;
                    }
                    ensure!(Session::restore(&index, serde_json::from_value(bad)?).is_err());
                    restore(&index, &mut s)?;
                    retired_restore = true;
                }
                match step.progress {
                    move_tool::Progress::Complete => break,
                    move_tool::Progress::Pending => (),
                    other => anyhow::bail!("empty payload requires no fabricated services: {other:?}"),
                }
                ensure!(n < 999);
            }
            ensure!(budget != 1 || retired_restore);
            s.finish_move_tool_cleanup()?;
            ensure!(s.simulation == initial, "tail reset changed simulation state");
            reports.push(serde_json::json!({"budget":budget,"retired_payload_cold_restore":retired_restore,"simulation_unchanged":true}));
            finals.push(serde_json::to_value(s.save())?);
        }
        ensure!(finals[0] == finals[1]);
        let output = root.join(std::env::var("SIMS_MOVE_RESET_TAIL_REPORT")?);
        ensure!(output.starts_with(root.join("cache")) && !output.exists());
        std::fs::write(output, serde_json::to_vec_pretty(&reports)?)?;
        Ok(())
    })?.join().expect("tail reset test panicked")
}
fn restore(index: &Index, s: &mut Session) -> Result<()> {
    let saved = s.save();
    *s = Session::restore(index, serde_json::from_slice(&serde_json::to_vec(&saved)?)?)?;
    ensure!(serde_json::to_value(s.save())? == serde_json::to_value(saved)?);
    Ok(())
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_MOVE_RESET_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut cases = vec![];
    for lot in ["goth", "mashuga"] {
        let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
            format!("cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"),
        ))?)?;
        let camera: sims_render::camera_device::Saved =
            serde_json::from_value(wrapper["camera_device"].clone())?;
        for grouped in [false, true] {
            let has_payload = grouped || lot == "mashuga";
            let flag = u8::from(lot == "mashuga");
            let kind = if lot == "mashuga" && grouped {
                cleanup::Kind::Deactivate
            } else {
                cleanup::Kind::Reset
            };
            let saved: Saved = serde_json::from_slice(&std::fs::read(root.join(format!(
                "cache/original-cursor-return-frames-2/{lot}-{grouped}-session.json"
            )))?)?;
            let mut finals = vec![];
            let mut runs = vec![];
            for resumed in [false, true] {
                eprintln!("move reset: {lot} grouped={grouped} resumed={resumed}");
                let mut s = Session::restore(&index, saved.clone())?;
                let focus = s
                    .host
                    .presentation
                    .object_focus
                    .context("held cursor absent")?;
                let cursor = s
                    .simulation
                    .vm
                    .world
                    .scheduler_storage
                    .as_ref()
                    .unwrap()
                    .object_for_handle(focus)
                    .unwrap();
                let chain = sims_vm::multitile::chain(&s.simulation.vm.world, cursor)
                    .map_err(anyhow::Error::msg)?;
                let furniture = s.simulation.vm.world.objects[&cursor]
                    .cursor
                    .as_ref()
                    .unwrap()
                    .underlying
                    .unwrap();
                let target = s
                    .simulation
                    .vm
                    .world
                    .scheduler_storage
                    .as_ref()
                    .unwrap()
                    .object_for_handle(furniture)
                    .unwrap();
                let tick = s.simulation.vm.tick;
                let rng = s.simulation.vm.rng_state;
                let funds = s.simulation.vm.world.budget.clone();
                let mut history = move_tool::History::new(1);
                history
                    .store
                    .construct_manager(5001)
                    .map_err(anyhow::Error::msg)?;
                if has_payload {
                    //Native capture normalizes a secondary sofa part to its
                    //master. Retain identities for every actual live definition,
                    //including occupied slots; don't assume the clicked part's.
                    for d in s
                        .simulation
                        .vm
                        .world
                        .objects
                        .values()
                        .filter_map(|o| o.definition.as_ref())
                    {
                        if !history.definitions.values().any(|old| old == d) {
                            history
                                .definitions
                                .insert(history.definitions.len() as u32 + 1, d.clone());
                        }
                    }
                    let definition = |v: &sims_vm::Definition| {
                        history
                            .definitions
                            .iter()
                            .find_map(|(&id, d)| (v == d).then_some(id))
                            .ok_or("fixture definition was not retained".into())
                    };
                    let mut next_command = 7002;
                    object::construction::construct(
                        object::construction::Input {
                            command: 7001,
                            script_id: target,
                            kind: 1,
                            cost: 123,
                            budget: 8001,
                        },
                        &mut history.store,
                        &mut history.objects,
                        &mut history.references,
                        &mut snapshot::live::View {
                            world: &mut s.simulation.vm.world,
                            program: &s.program,
                            links: &s.simulation.native_lot.as_ref().unwrap().placement.links,
                            definition: &definition,
                        },
                        &mut || {
                            let id = next_command;
                            next_command += 1;
                            Ok(Some(id))
                        },
                    )
                    .map_err(anyhow::Error::msg)?;
                    history.store.acquire(7001).map_err(anyhow::Error::msg)?; //tool owner
                    if flag != 0 {
                        //An existing UndoManager reference proves Release is
                        //not an unconditional destructor. This is supplied context.
                        history.store.acquire(7001).map_err(anyhow::Error::msg)?;
                        history
                            .store
                            .append(edit_history::List::Undo(5001), 7001)
                            .map_err(anyhow::Error::msg)?;
                    }
                }
                let context = &s
                    .host
                    .presentation
                    .notifications
                    .as_ref()
                    .context("listeners absent")?
                    .dispatch;
                let mut registry = context.registry.clone();
                if !registry
                    .nodes
                    .values()
                    .any(|n| n.handler == sims_vm::commander::house::Receiver::Interface)
                {
                    let key = registry.nodes.keys().max().copied().unwrap_or(0) + 1;
                    registry.construct(key).map_err(anyhow::Error::msg)?;
                    registry
                        .bind(key, sims_vm::commander::house::Receiver::Interface)
                        .map_err(anyhow::Error::msg)?;
                }
                s.bind_house_listeners(registry, context.fields.clone())?;
                let mut device = Device::restore(&s, camera.clone())?;
                let mut tool = cleanup::State::constructor_fields(5001);
                tool.held = Some(focus);
                tool.payload = has_payload.then_some(7001);
                tool.tool.flags[0] = flag;
                tool.hover = target;
                //Original hover flags are updated through the live flag setter.
                let flags = s.simulation.vm.world.objects[&target].native_flags.unwrap();
                s.simulation
                    .update_native_low_flags(&s.program, target, u32::from((flags as u16) | 8))
                    .map_err(anyhow::Error::msg)?;
                s.bind_move_tool(move_tool::State::new(
                    tool,
                    cleanup::Inputs {
                        registry: 5002,
                        window: 5003,
                        key_function: 5004,
                        control_state: 5005,
                    },
                    history,
                ))?;
                s.begin_move_tool_cleanup(kind)?;
                ensure!(s.player_input_pending());
                ensure!(s.begin_move_tool_cleanup(kind).is_err());
                ensure!(
                    s.acknowledge_move_tool_input(
                        cleanup::Request::DeleteObject {
                            registry: 5002,
                            id: cursor
                        },
                        cleanup::Reply::Done
                    )
                    .is_err()
                );
                let mut restores = 0;
                let mut retired_restore = false;
                let mut history_restore = false;
                let mut detach_restore = false;
                let mut instructions = 0;
                let mut services = vec![];
                for n in 0..50000 {
                    let advance = s.advance_move_tool(if resumed { 1 } else { 1000 })?;
                    instructions += advance.instructions;
                    let held_retired = s
                        .host
                        .presentation
                        .move_tool
                        .as_ref()
                        .unwrap()
                        .retained_deleted_held()
                        .is_some();
                    let history_pending = matches!(
                        advance.progress,
                        move_tool::Progress::Service(move_tool::Service::Notification(_))
                    );
                    let detach_pending = s.simulation.cursor_rotation.is_some();
                    if resumed
                        && (n == 1
                            || held_retired && !retired_restore
                            || history_pending && !history_restore
                            || detach_pending && !detach_restore)
                    {
                        restore(&index, &mut s)?;
                        restores += 1;
                        retired_restore |= held_retired;
                        history_restore |= history_pending;
                        detach_restore |= detach_pending;
                    }
                    match advance.progress {
                        move_tool::Progress::Complete => break,
                        move_tool::Progress::Pending => (),
                        move_tool::Progress::Service(move_tool::Service::Input(r)) => {
                            services.push(serde_json::to_value(r)?);
                            let reply = match r {
                                cleanup::Request::Key { .. } => cleanup::Reply::Key(0),
                                cleanup::Request::SetCursor { value: 10, .. } => {
                                    cleanup::Reply::Done
                                }
                                _ => anyhow::bail!("unexpected input service {r:?}"),
                            };
                            s.acknowledge_move_tool_input(r, reply)?;
                        }
                        move_tool::Progress::Service(move_tool::Service::Notification(ref r)) => {
                            services.push(serde_json::to_value(r)?);
                            device.advance_notification(&mut s, 1)?;
                        }
                        move_tool::Progress::Service(r) => {
                            anyhow::bail!("unexpected unbound reset service {r:?}")
                        }
                    }
                    ensure!(n < 49999, "move reset stalled");
                }
                ensure!(s.host.presentation.move_tool.as_ref().unwrap().complete());
                if resumed {
                    ensure!(retired_restore, "missing post44123f save boundary");
                }
                s.finish_move_tool_cleanup()?;
                ensure!(!s.move_tool_pending());
                let tool = s.host.presentation.move_tool.as_ref().unwrap();
                ensure!(
                    tool.tool.held.is_none() && tool.tool.payload.is_none() && tool.tool.hover == 0
                );
                if has_payload {
                    ensure!(
                        tool.history
                            .store
                            .command(7001)
                            .map_err(anyhow::Error::msg)?
                            .references
                            == 1
                    );
                    ensure!(
                        tool.history
                            .objects
                            .get(7001)
                            .map_err(anyhow::Error::msg)?
                            .kind
                            == if flag == 0 { 3 } else { 1 }
                    );
                }
                ensure!(
                    s.simulation.vm.tick == tick
                        && s.simulation.vm.rng_state == rng
                        && s.simulation.vm.world.budget == funds
                );
                ensure!(s.host.presentation.object_focus.is_none());
                ensure!(
                    chain
                        .iter()
                        .all(|id| !s.simulation.vm.world.objects.contains_key(id))
                );
                ensure!(s.simulation.vm.world.objects.contains_key(&target));
                let image = software::render(Device::restore(&s, camera.clone())?.scene());
                if resumed {
                    std::fs::write(
                        out.join(format!("{lot}-{grouped}-session.json")),
                        serde_json::to_vec(&s.save())?,
                    )?;
                    restore(&index, &mut s)?;
                    restores += 1;
                    ensure!(
                        software::render(Device::restore(&s, camera.clone())?.scene()) == image
                    );
                    image.save(out.join(format!("{lot}-{grouped}.png")))?;
                }
                runs.push(serde_json::json!({"resumed":resumed,"cold_restores":restores,"retired_held_restore":retired_restore,"history_restore":history_restore,"detach_restore":detach_restore,"instructions":instructions,"tick":tick,"services":services}));
                finals.push((serde_json::to_value(s.save())?, image, services));
            }
            ensure!(
                finals[0] == finals[1],
                "uninterrupted/cold-resumed reset differs"
            );
            cases.push(serde_json::json!({"lot":lot,"grouped":grouped,"has_payload":has_payload,"flag75":flag,"kind":kind,"runs":runs,"equal":true}));
        }
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"cases":cases,"original_execution":false,
        "scope":"Live reset and flag75 deactivation; original-house cursors, original object-command constructor, supplied tool/history owners, actual EE controls service. Does not claim original suspended UI import, complete Buy frontend, undo snapshot placement, or normal tick advancement."}),
        )?,
    )?;
    Ok(())
}
