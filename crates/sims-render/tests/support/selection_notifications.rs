//! Real imported residents, with selection/deletion retaining E4 listener calls.
//! UI requests have explicit test sinks; no automatic native startup is claimed.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_runtime::original_session::{
    Options, Session,
    notifications::{Progress, Reply, Service, State},
};
use sims_sim::{lot_selection::NotificationOwner, pose::portrait::view::Input};
use sims_vm::commander::house::Request;

fn drain(
    s: &mut Session,
    index: &Index,
    camera: Input,
    budget: usize,
    resume: bool,
    expected: Option<NotificationOwner>,
) -> Result<(Vec<Value>, usize)> {
    let mut services = vec![];
    let mut restores = 0;
    for _ in 0..1000 {
        let owner = s
            .host
            .presentation
            .notifications
            .as_ref()
            .context("notification absent")?;
        let decoded: State = serde_json::from_slice(&serde_json::to_vec(owner)?)?;
        ensure!(&decoded == owner);
        decoded.validate(&s.host.presentation, &s.simulation)?;
        match s.advance_house_notification(camera, budget)? {
            Progress::Pending => (),
            Progress::Complete(1) => {
                ensure!(s.simulation.pending_selection_notification().is_none());
                return Ok((services, restores));
            }
            Progress::Complete(other) => anyhow::bail!("unexpected E4 result {other}"),
            Progress::Service(ref service @ Service::Listener(ref call))
                if matches!(
                    call.request,
                    Request::UiFlags { bits: 8 } | Request::UiRefresh
                ) =>
            {
                if let Some(expected) = expected {
                    let (parent, started) = s
                        .simulation
                        .pending_selection_notification()
                        .context("selection parent absent")?;
                    ensure!(started && parent.owner == expected);
                    if let NotificationOwner::Deletion { object, .. } = expected {
                        ensure!(
                            s.simulation.vm.world.objects.contains_key(&object),
                            "deletion overtook E4"
                        );
                        ensure!(parent.payload == 0);
                    }
                    if services.is_empty() {
                        let saved = s.save();
                        let bytes = serde_json::to_vec(&saved)?;
                        ensure!(s.step().is_err() && s.begin_selection(None).is_err());
                        ensure!(s.advance_selection(camera, budget).is_err());
                        let mut wrong = parent;
                        wrong.payload = wrong.payload.wrapping_add(1);
                        ensure!(s.simulation.complete_selection_notification(wrong).is_err());
                        let mut wrong_owner = parent;
                        wrong_owner.owner = match parent.owner {
                            NotificationOwner::Selection { target } => {
                                NotificationOwner::Selection {
                                    target: if target.is_some() { None } else { Some(1) },
                                }
                            }
                            NotificationOwner::Deletion { object, depth } => {
                                NotificationOwner::Deletion {
                                    object,
                                    depth: depth + 1,
                                }
                            }
                        };
                        ensure!(
                            s.simulation
                                .complete_selection_notification(wrong_owner)
                                .is_err()
                        );
                        ensure!(
                            serde_json::to_vec(&s.save())? == bytes,
                            "rejected calls changed state"
                        );
                        let mut corrupt = serde_json::to_value(
                            s.host.presentation.notifications.as_ref().unwrap(),
                        )?;
                        corrupt["selection"]["payload"] = json!(wrong.payload);
                        ensure!(
                            serde_json::from_value::<State>(corrupt)?
                                .validate(&s.host.presentation, &s.simulation)
                                .is_err()
                        );
                        if resume {
                            *s = Session::restore(index, serde_json::from_slice(&bytes)?)?;
                            ensure!(serde_json::to_vec(&s.save())? == bytes);
                            restores += 1;
                        }
                    }
                }
                let before = serde_json::to_vec(&s.save())?;
                ensure!(
                    s.advance_house_notification(camera, budget)?
                        == Progress::Service(service.clone())
                );
                ensure!(
                    serde_json::to_vec(&s.save())? == before,
                    "pending listener repeated effects"
                );
                services.push(json!({"service":service,"globals":[s.simulation.vm.world.globals[&2],s.simulation.vm.world.globals[&3]]}));
                s.acknowledge_house_notification(service, Reply::Done)?;
                ensure!(
                    s.acknowledge_house_notification(service, Reply::Done)
                        .is_err()
                );
            }
            other => anyhow::bail!("unexpected selection listener {other:?}"),
        }
    }
    anyhow::bail!("E4 failed to complete")
}

pub fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let camera = Input {
        viewport: [0, 0, 1000, 740],
        center: [500, 370],
        zoom: 2,
        rotation: 0,
    };
    let mut reports = vec![];
    let mut gallery = image::RgbaImage::new(2880, 1440);
    for (column, (name, source)) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut initial = Session::load(&index, source, Options::default())?;
        let residents = &initial.house.household.as_ref().unwrap().residents;
        let (a, b) = (residents[0].object as i16, residents[1].object as i16);
        initial.begin_selection(Some(a))?;
        while !initial.advance_selection(camera, 10000)? {}
        let (mut view, inputs, _order, retained) =
            super::camera_session::context(&initial, camera, 1)?;
        // The helper seeds stale room IDs to exercise F6's clear. E4 instead
        // starts here with a supplied host view that has no hovered rooms yet.
        view.floor.history.clear();
        let fields = super::notification_session::fields(view.floor.visibility.include_visible);
        initial.configure_registry_house_view(view, inputs, retained)?;
        initial.begin_house_notification(
            super::notification_session::registry()?,
            fields,
            0xe4,
            a as u32,
        )?;
        drain(&mut initial, &index, camera, 10000, false, None)?;
        // A pre-binding session snapshot keeps its explicit Globals-only host
        // profile. Restoring it must not silently enable future notifications.
        let mut legacy = initial.save();
        legacy.simulation.state.native_selection_broadcasts = false;
        ensure!(
            !Session::restore(&index, legacy)?
                .simulation
                .native_selection_broadcasts
        );
        let initial = initial.save();
        let mut completed = vec![];
        let mut runs = vec![];
        for budget in [1, 10000] {
            let mut s = Session::restore(&index, initial.clone())?;
            let clock_rng = (
                s.simulation.vm.tick,
                s.simulation.vm.rng_state,
                s.simulation.vm.lot_rng_state,
                s.simulation.vm.crt_rng_state,
            );
            let mut calls = vec![];
            let mut restores = 0;
            for (op, target) in [Some(b), Some(b), None, Some(a)].into_iter().enumerate() {
                s.begin_selection(target)?;
                let mut done = false;
                let mut broadcasts = 0;
                for _ in 0..2000 {
                    if s.advance_selection(camera, budget)? {
                        done = true;
                        break;
                    }
                    if s.house_notification_pending() {
                        let (events, count) = drain(
                            &mut s,
                            &index,
                            camera,
                            budget,
                            budget == 1,
                            Some(NotificationOwner::Selection { target }),
                        )?;
                        calls.push(json!({"operation":op,"events":events}));
                        restores += count;
                        broadcasts += 1;
                    }
                }
                ensure!(
                    done && broadcasts == usize::from(op != 1),
                    "selection broadcast count differs"
                );
                ensure!(
                    s.simulation
                        .native_selected_person()
                        .map_err(anyhow::Error::msg)?
                        == target
                );
                ensure!(s.simulation.vm.world.globals[&3] == target.unwrap_or(0));
            }
            if name == "newbie" {
                s.simulation
                    .begin_synchronous_removal(&s.program, a)
                    .map_err(anyhow::Error::msg)?;
                let mut done = false;
                let mut broadcasts = 0;
                for _ in 0..2000 {
                    let mut step = sims_sim::Step {
                        stop: sims_sim::StepStop::Budget,
                        trace: vec![],
                        events: vec![],
                    };
                    let mut remaining = budget;
                    let finished = s
                        .simulation
                        .drive_synchronous_removal(&s.program, &mut remaining, &mut step)
                        .map_err(anyhow::Error::msg)?;
                    if matches!(
                        s.simulation.pending_selection_notification(),
                        Some((_, false))
                    ) {
                        // Exercise the ordinary session handoff as well as the
                        // explicit lower-level deletion driver used by this fixture.
                        ensure!(
                            s.step()?
                                == sims_runtime::original_session::Progress::HouseNotification
                        );
                    } else {
                        s.service_selection_assets(camera, budget)?;
                    }
                    if s.house_notification_pending() {
                        let (events, count) = drain(
                            &mut s,
                            &index,
                            camera,
                            budget,
                            budget == 1,
                            Some(NotificationOwner::Deletion {
                                object: a,
                                depth: 1,
                            }),
                        )?;
                        calls.push(json!({"operation":"delete","events":events}));
                        restores += count;
                        broadcasts += 1;
                    }
                    s.host.service(&mut s.simulation, &mut s.program)?;
                    if finished {
                        done = true;
                        break;
                    }
                }
                ensure!(done && broadcasts == 1, "deletion E4 failed to finish");
                ensure!(
                    s.simulation
                        .take_synchronous_removal()
                        .map_err(anyhow::Error::msg)?
                        == a
                );
                ensure!(!s.simulation.vm.world.objects.contains_key(&a));
                ensure!(
                    s.simulation
                        .native_selected_person()
                        .map_err(anyhow::Error::msg)?
                        .is_none()
                );
                ensure!(s.simulation.vm.world.globals[&3] == 0);
            }
            ensure!(
                clock_rng
                    == (
                        s.simulation.vm.tick,
                        s.simulation.vm.rng_state,
                        s.simulation.vm.lot_rng_state,
                        s.simulation.vm.crt_rng_state
                    ),
                "selection/deletion changed time or RNG"
            );
            ensure!(
                restores
                    == if budget == 1 {
                        if name == "newbie" { 4 } else { 3 }
                    } else {
                        0
                    }
            );
            runs.push(json!({"budget":budget,"calls":calls,"restores":restores,"tick_rng_unchanged":true}));
            completed.push(s.save());
        }
        ensure!(
            runs[0]["calls"] == runs[1]["calls"],
            "listener order differs by budget"
        );
        ensure!(
            serde_json::to_vec(&completed[0])? == serde_json::to_vec(&completed[1])?,
            "saved selection states differ"
        );
        let mut a = Session::restore(&index, completed.remove(0))?;
        let mut b = Session::restore(&index, completed.remove(0))?;
        // Unmodified households must continue identically. The deliberate
        // deletion case is a callback fixture, not normal household progress.
        if name != "newbie" {
            for _ in 0..10 {
                ensure!(a.step()? == sims_runtime::original_session::Progress::Tick);
                ensure!(b.step()? == sims_runtime::original_session::Progress::Tick);
            }
            ensure!(serde_json::to_vec(&a.save())? == serde_json::to_vec(&b.save())?);
        }
        let mut ra = sims_render::original_house::OriginalHouse::default();
        let mut rb = sims_render::original_house::OriginalHouse::default();
        for rotation in 0..2 {
            let c = sims_render::original_house::OriginalHouse::camera(
                &a,
                960,
                720,
                rotation,
                0.55,
                if name == "goth" && rotation == 1 {
                    2
                } else {
                    1
                },
            );
            let pixels = sims_render::software::render(&ra.scene(&a, c)?);
            ensure!(pixels == sims_render::software::render(&rb.scene(&b, c)?));
            image::imageops::replace(
                &mut gallery,
                &pixels,
                column as i64 * 960,
                i64::from(rotation) * 720,
            );
        }
        reports.push(json!({"house":name,"runs":runs,"restored_pixels_equal":true,"continuation_ticks":if name=="newbie" {0} else {10}}));
    }
    gallery.save(root.join("cache/original-selection-notification-gallery.png"))?;
    std::fs::write(
        root.join("cache/original-selection-notification-session.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}
