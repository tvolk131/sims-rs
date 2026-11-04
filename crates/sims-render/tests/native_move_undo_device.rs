//! Original furniture, real ordered registration and saved snapshot pickup.
//! Stop at the owned placement child, before it runs; the continuation test
//! completes this child and deactivation.
use anyhow::{Context, Result, ensure};
use sims_content::{Index, object_catalog::Catalog};
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session, move_tool};
use sims_sim::{
    edit_history::{object, snapshot},
    object_catalog::move_tool::cleanup,
};
use std::sync::Arc;

#[test]
#[ignore = "private original-house checkpoints; release/headless"]
fn original_house_deactivation_reaches_snapshot_placement_after_cold_pickup() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("undo test panicked")
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
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_MOVE_UNDO_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Arc::new(Index::scan(&root.join("game-data/cabinet-extracted"))?);
    let mut catalog = Catalog::household(
        index.clone(),
        "UserData/Neighborhood.iff",
        Default::default(),
    )?;
    let mut template = move_tool::History::new(1);
    template.retain_catalog(&mut catalog)?;
    template
        .store
        .construct_manager(5001)
        .map_err(anyhow::Error::msg)?;
    template.budgets.0.insert(
        8001,
        object::budget::State {
            pending: 0,
            category: 0,
        },
    );
    let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-ui-timer-dispatch-iced/goth/resumed.json"),
    )?)?;
    let camera: sims_render::camera_device::Saved =
        serde_json::from_value(wrapper["camera_device"].clone())?;
    let mut cases = vec![];
    for grouped in [false, true] {
        let saved: Saved = serde_json::from_slice(&std::fs::read(root.join(format!(
            "cache/original-cursor-return-frames-2/goth-{grouped}-session.json"
        )))?)?;
        let mut finals = vec![];
        let mut runs = vec![];
        for resumed in [false, true] {
            eprintln!("undo pickup grouped={grouped} resumed={resumed}");
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
            let mut h = template.clone();
            let definitions = h.definitions.clone();
            let definition = |d: &sims_vm::Definition| {
                let mut candidates = definitions
                    .iter()
                    .filter_map(|(&id, v)| (v == d).then_some(id));
                let id = candidates.next().ok_or("original definition absent")?;
                if candidates.next().is_some() {
                    return Err("ambiguous original definition".into());
                }
                Ok(id)
            };
            let mut next = 7002;
            object::construction::construct(
                object::construction::Input {
                    command: 7001,
                    script_id: target,
                    kind: 1,
                    cost: 123,
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
                    let id = next;
                    next += 1;
                    Ok(Some(id))
                },
            )
            .map_err(anyhow::Error::msg)?;
            h.store.acquire(7001).map_err(anyhow::Error::msg)?;
            let snapshot = h.objects.get(7001).map_err(anyhow::Error::msg)?.snapshots[0].clone();
            let original = serde_json::to_value(&s.simulation.vm.world.positions)?;
            let tick = s.simulation.vm.tick;
            let rng = s.simulation.vm.rng_state;
            let funds = s.simulation.vm.world.budget.clone();
            let mut tool = cleanup::State::constructor_fields(5001);
            tool.held = Some(focus);
            tool.payload = Some(7001);
            tool.hover = target;
            s.bind_move_tool(move_tool::State::new(
                tool,
                cleanup::Inputs {
                    registry: 5002,
                    window: 0,
                    key_function: 0,
                    control_state: 0,
                },
                h,
            ))?;
            s.begin_move_tool_cleanup(cleanup::Kind::Deactivate)?;
            let mut device = Device::restore(&s, camera.clone())?;
            let mut restores = 0;
            let mut pickup_restore = false;
            let mut callback_restore = false;
            let mut instructions = 0;
            let mut notices = vec![];
            let pending = loop {
                let a = s.advance_move_tool(1)?;
                instructions += a.instructions;
                ensure!(s.player_input_pending());
                let pickup = s.simulation.object_pickup.is_some();
                let callback = pickup
                    && s.simulation
                        .vm
                        .threads
                        .values()
                        .any(|t| t.callback.is_some());
                if resumed && (pickup && !pickup_restore || callback && !callback_restore) {
                    if !pickup_restore {
                        let mut broken = serde_json::to_value(s.save())?;
                        broken["view"]["move_tool"]["call"]["child"]["History"]["pickup"] =
                            serde_json::Value::Null;
                        ensure!(
                            Session::restore(&index, serde_json::from_value(broken)?).is_err(),
                            "orphan pickup was accepted"
                        );
                    }
                    cold(&index, &mut s)?;
                    restores += 1;
                    pickup_restore |= pickup;
                    callback_restore |= callback;
                }
                if let Some(child) = &s.simulation.object_placement {
                    if let object::driver::Service::Restore(
                        snapshot::lifecycle::Request::Placement(
                            snapshot::placement::Request::Check(p),
                        ),
                    ) = child.caller().service
                    {
                        break p;
                    }
                }
                match a.progress {
                    move_tool::Progress::Pending => (),
                    move_tool::Progress::Service(move_tool::Service::Notification(r)) => {
                        notices.push(serde_json::to_value(r)?);
                        device.advance_notification(&mut s, 1)?;
                    }
                    move_tool::Progress::Service(move_tool::Service::Object(
                        object::driver::Service::Restore(snapshot::lifecycle::Request::Placement(
                            snapshot::placement::Request::Check(p),
                        )),
                    )) => break p,
                    other => anyhow::bail!("unexpected undo boundary: {other:?}"),
                }
                ensure!(notices.len() < 100, "notification stalled");
            };
            ensure!(!resumed || pickup_restore, "never saved a live pickup");
            ensure!(s.simulation.object_pickup.is_none());
            ensure!(
                pending.position == snapshot.position
                    && pending.level == snapshot.level.unwrap()
                    && pending.slot == snapshot.slot.unwrap()
            );
            ensure!(
                s.simulation.vm.tick == tick
                    && s.simulation.vm.rng_state == rng
                    && s.simulation.vm.world.budget == funds
            );
            ensure!(
                serde_json::to_value(&s.simulation.vm.world.positions)? != original,
                "snapshot removal never moved the furniture off lot"
            );
            ensure!(s.host.presentation.object_focus.is_none());
            ensure!(!s.host.presentation.move_tool.as_ref().unwrap().complete());
            ensure!(s.finish_move_tool_cleanup().is_err());
            let image = software::render(Device::restore(&s, camera.clone())?.scene());
            if resumed {
                cold(&index, &mut s)?;
                restores += 1;
                ensure!(software::render(Device::restore(&s, camera.clone())?.scene()) == image);
                image.save(out.join(format!("goth-{grouped}.png")))?;
                std::fs::write(
                    out.join(format!("goth-{grouped}-session.json")),
                    serde_json::to_vec(&s.save())?,
                )?;
            }
            runs.push(serde_json::json!({"resumed":resumed,"full_cold_restores":restores,"pickup_restore":pickup_restore,"callback_restore":callback_restore,"instructions":instructions,"tick":tick,"pending_check":pending,"notices":notices}));
            finals.push((serde_json::to_value(s.save())?, image, notices));
        }
        ensure!(
            finals[0] == finals[1],
            "snapshot pickup diverged across cold resume"
        );
        cases.push(serde_json::json!({"grouped":grouped,"runs":runs,"equal":true}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"cases":cases,"native_execution":false,"scope":"Actual ordered startup catalog, native object-command constructor, live Deactivate/Undo and virtual pickup; stops at the owned original snapshot placement check before execution. No complete deactivation or normal tick advancement claimed."}),
        )?,
    )?;
    Ok(())
}
