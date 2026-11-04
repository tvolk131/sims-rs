//! Original cleanup/factories/removal and F6, with explicit process/view inputs.
//! The headless host supplies a null native terrain-renderer pointer; reset and
//! import commands remain unsupported boundaries rather than acknowledged work.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::Index;
use sims_runtime::original_session::{
    Options, Session,
    completion::{Config, Progress},
    notifications::{Reply, Service},
};
use sims_sim::{lot_selection::Request as Selection, pose::portrait::view::Input};
use sims_vm::{commander::house::Request as Listener, load_completion::Request};
use std::collections::BTreeSet;

/// Rebuild the historical tutorial prefix from original files under the current
/// program. Keep the old checkpoints unchanged. The native terrain pointer is
/// explicitly absent and camera/device replies use the existing supplied host
/// profile; this does not claim a full original application startup.
pub fn fresh_newbie() -> Result<()> {
    use sims_render::{
        dialog::client::{Client, Inputs},
        original_house::OriginalHouse,
        software,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join(std::env::var("SIMS_FRESH_TUTORIAL_OUTPUT")?);
    ensure!(
        output.starts_with(root.join("cache")),
        "private cache output required"
    );
    std::fs::create_dir(&output).context("refusing to replace tutorial evidence")?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let source = "UserData/Houses/House07.iff";
    let mut s = Session::load(
        &index,
        source,
        Options {
            routing: sims_sim::lot_route::path::Policy::InferredRectangles,
            native_postload: true,
            native_load_completion: true,
            ..Default::default()
        },
    )?;
    let start = s.simulation.vm.tick;
    let resident = s
        .house
        .household
        .as_ref()
        .context("Newbie family absent")?
        .residents[0]
        .object;
    let camera = Input {
        viewport: [0, 0, 1000, 740],
        center: [500, 370],
        zoom: 2,
        rotation: 0,
    };
    let (mut view, inputs, _order, retained) = super::camera_session::context(&s, camera, 1)?;
    view.floor.history.clear();
    s.configure_registry_house_view(view, inputs, retained)?;
    let fields = super::notification_session::fields(
        s.host
            .presentation
            .presenter
            .as_ref()
            .unwrap()
            .view
            .floor
            .visibility
            .include_visible,
    );
    s.begin_postload(super::notification_session::registry()?, fields)?;
    let mut done = false;
    for _ in 0..30000 {
        if s.advance_postload(camera, 256)? {
            done = true;
            break;
        }
    }
    ensure!(done, "postload did not finish");
    let bytes = serde_json::to_vec(&s.save())?;
    s = Session::restore(&index, serde_json::from_slice(&bytes)?)?;
    ensure!(serde_json::to_vec(&s.save())? == bytes);
    s.begin_load_completion(Config {
        controller_suppressed: 0,
        mode: 0,
    })?;
    let mut services = vec![];
    let mut checkpoints = BTreeSet::new();
    done = false;
    for _ in 0..30000 {
        let progress = s.advance_load_completion(camera, 256)?;
        let key = match &progress {
            Progress::Environment => Some("environment"),
            Progress::Notification(Service::View(_)) => Some("view"),
            Progress::Notification(Service::Camera(_)) => Some("camera"),
            Progress::Notification(Service::Listener(_)) => Some("listener"),
            _ => None,
        };
        if key.is_some_and(|k| checkpoints.insert(k)) {
            let bytes = serde_json::to_vec(&s.save())?;
            s = Session::restore(&index, serde_json::from_slice(&bytes)?)?;
            ensure!(serde_json::to_vec(&s.save())? == bytes);
        }
        match progress {
            Progress::Pending => (),
            Progress::Complete => {
                done = true;
                break;
            }
            Progress::Environment => s.acknowledge_absent_terrain_renderer()?,
            Progress::Notification(service) => {
                let reply = match &service {
                    Service::View(_) => {
                        Reply::View(s.host.presentation.presenter.as_ref().unwrap().view.clone())
                    }
                    Service::Camera(request) => {
                        Reply::Camera(super::notification_session::device_reply(request)?)
                    }
                    Service::Listener(call)
                        if matches!(
                            call.request,
                            Listener::ViewerZoom { .. }
                                | Listener::ViewerRotation { .. }
                                | Listener::UiFlags { .. }
                                | Listener::UiRefresh
                                | Listener::PanelRefresh
                                | Listener::SharedInterface
                                | Listener::PanelBefore
                                | Listener::PanelAfter
                        ) =>
                    {
                        Reply::Done
                    }
                    other => anyhow::bail!("fresh tutorial load needs host service: {other:?}"),
                };
                services.push(serde_json::to_value(&service)?);
                s.acknowledge_house_notification(&service, reply)?;
            }
        }
    }
    ensure!(done && !s.load_completion_pending() && !s.host.presentation.awaiting_house_open);
    ensure!(
        s.simulation.vm.tick == start,
        "loading advanced the household clock"
    );
    let mut client = Client::new(&mut s)?;
    let mut reached = false;
    for _ in 0..1000 {
        if client.pending() {
            client.advance(&mut s, 256)?;
            continue;
        }
        match s.step()? {
            sims_runtime::original_session::Progress::Tick => (),
            sims_runtime::original_session::Progress::DialogConstruction(thread) => {
                let view = OriginalHouse::camera(&s, 1000, 740, 0, 0.55, 1);
                client.begin(&mut s, thread, Inputs::camera(view, 64)?)?;
            }
            sims_runtime::original_session::Progress::Dialog => {
                reached = true;
                break;
            }
            other => anyhow::bail!("unexpected fresh tutorial boundary: {other:?}"),
        }
        client.collect(&mut s)?;
    }
    ensure!(reached, "tutorial welcome not reached");
    let dialogs = &s.simulation.vm.world.dialogs;
    let modal = dialogs
        .records
        .iter()
        .find(|r| Some(r.id) == dialogs.modal)
        .context("welcome absent")?
        .clone();
    ensure!(
        modal.operands[5] & 127 == 4 && modal.buttons.len() == 1 && modal.bitmap_icon == Some(5018)
    );
    let owner = s
        .simulation
        .vm
        .world
        .ui_owner
        .owner
        .context("original tutorial owner absent")?;
    ensure!(
        s.simulation.vm.world.objects[&owner]
            .definition
            .as_ref()
            .unwrap()
            .guid
            == sims_vm::load_completion::CONTROLLER
    );
    let capture = |s: &Session, c: &Client| -> Result<serde_json::Value> {
        Ok(
            json!({"session":s.save(),"dialog_client":c.save(s)?,"floor":1,"zoom":0.55,"resident":resident,"selected":null}),
        )
    };
    let saved = capture(&s, &client)?;
    std::fs::write(output.join("welcome.json"), serde_json::to_vec(&saved)?)?;
    let restored = Session::restore(&index, serde_json::from_value(saved["session"].clone())?)?;
    let restored_client = Client::restore(
        &restored,
        serde_json::from_value(saved["dialog_client"].clone())?,
    )?;
    ensure!(saved == capture(&restored, &restored_client)?);
    ensure!(matches!(
        s.step()?,
        sims_runtime::original_session::Progress::Dialog
    ));
    ensure!(
        saved == capture(&s, &client)?,
        "unanswered welcome advanced"
    );
    let view = OriginalHouse::camera(&s, 1000, 740, 0, 0.55, 1);
    let pixels = software::render(&OriginalHouse::default().scene(&s, view)?);
    ensure!(pixels == software::render(&OriginalHouse::default().scene(&restored, view)?));
    pixels.save(output.join("floor-1.png"))?;
    ensure!(
        saved == capture(&s, &client)?,
        "render changed welcome state"
    );
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "source":source,"start_tick":start,"tick":s.simulation.vm.tick,"controller":owner,"modal":modal,
            "restored_full_session_and_client":true,"restored_pixels_match":true,"unanswered_dialog_preserved":true,
            "postload_restored":true,"completion_restore_checkpoints":checkpoints,"completion_services":services,
            "scope":"Fresh original Newbie files/current program; existing explicit camera/device reply profile and absent native terrain renderer. No retagging, tutorial attribute edits, injected UI events, empty neighborhood input, or claim of full original startup."
        }))?,
    )?;
    Ok(())
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
    let mut report = vec![];
    let mut gallery = image::RgbaImage::new(2880, 1440);
    for (column, (name, source)) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut initial = Session::load(
            &index,
            source,
            Options {
                native_postload: true,
                native_load_completion: true,
                ..Default::default()
            },
        )?;
        let level = if name == "goth" { 2 } else { 1 };
        initial
            .simulation
            .vm
            .world
            .refresh
            .as_mut()
            .unwrap()
            .visible_level = level;
        let (mut view, inputs, _order, retained) =
            super::camera_session::context(&initial, camera, level)?;
        view.floor.history.clear();
        initial.configure_registry_house_view(view, inputs, retained)?;
        let fields = super::notification_session::fields(
            initial
                .host
                .presentation
                .presenter
                .as_ref()
                .unwrap()
                .view
                .floor
                .visibility
                .include_visible,
        );
        initial.begin_postload(super::notification_session::registry()?, fields)?;
        while !initial.advance_postload(camera, 10000)? {}
        let initial = initial.save();
        for altered in [false, true] {
            if altered && name == "mashuga" {
                continue;
            }
            let mut seed = Session::restore(&index, initial.clone())?;
            let target = seed.house.household.as_ref().unwrap().residents[0].object as i16;
            let original_guid = seed.simulation.vm.world.objects[&target]
                .definition
                .as_ref()
                .unwrap()
                .guid;
            if altered {
                if name == "newbie" {
                    // An original post-load callback can leave a regular person
                    // without a family and associated with a different lot.
                    let p = seed
                        .simulation
                        .vm
                        .world
                        .objects
                        .get_mut(&target)
                        .unwrap()
                        .person
                        .as_mut()
                        .unwrap();
                    p.insert(61, 0);
                    p.insert(67, -1);
                } else {
                    // Membership changes after the pre-visit snapshot; visitor
                    // refresh must run before the second removal decision.
                    let guid = seed.simulation.vm.world.objects[&target]
                        .definition
                        .as_ref()
                        .unwrap()
                        .guid;
                    let people = seed.simulation.vm.world.person.as_mut().unwrap();
                    let members = people.families.get_mut(&people.controlled_family).unwrap();
                    members.retain(|id| people.neighbors[id].guid != guid);
                }
            }
            seed.begin_load_completion(Config {
                controller_suppressed: 0,
                mode: 0,
            })?;
            ensure!(
                seed.host
                    .presentation
                    .notifications
                    .as_ref()
                    .unwrap()
                    .dispatch
                    .is_idle()
            );
            let seed = seed.save();
            let mut finished = vec![];
            let mut runs = vec![];
            for budget in [1, 10000] {
                let mut s = Session::restore(&index, seed.clone())?;
                let before = (
                    s.simulation.vm.tick,
                    s.simulation.vm.rng_state,
                    s.simulation.vm.lot_rng_state,
                );
                let mut restores = BTreeSet::new();
                let mut calls = vec![];
                let mut last = None;
                let mut done = false;
                let mut services = vec![];
                for slice in 0..30000 {
                    let state = s.host.presentation.loading.as_ref().unwrap();
                    state.validate(&s)?;
                    let call = state.pending();
                    if call != last {
                        if let Some(c) = call {
                            calls.push(c);
                        }
                        last = call;
                    }
                    let checkpoint = match s.simulation.pending_native_selection() {
                        Some((Selection::FactoryGraphic(_), true)) => Some("factory_graphic"),
                        Some((Selection::CreationNotification(_), true)) => Some("factory_created"),
                        _ if call.is_some_and(|r| matches!(r, Request::Delete(_)))
                            && s.simulation.pending_removal_graphic().is_some() =>
                        {
                            Some("removal_graphic")
                        }
                        _ if !s
                            .simulation
                            .pending_deletion_character_services()
                            .is_empty() =>
                        {
                            Some("deletion_character")
                        }
                        _ if s.simulation.pending_selection_notification().is_some() => {
                            Some("selection_broadcast")
                        }
                        _ if call == Some(Request::Environment) => Some("environment"),
                        _ => None,
                    };
                    if budget == 1 && checkpoint.is_some_and(|key| restores.insert(key)) {
                        let bytes = serde_json::to_vec(&s.save())?;
                        ensure!(s.step().is_err() && s.begin_selection(None).is_err());
                        ensure!(
                            s.advance_selection(camera, 1).is_err()
                                && s.service_selection_assets(camera, 1).is_err()
                        );
                        ensure!(s.advance_view_refresh(camera, 1).is_err());
                        ensure!(serde_json::to_vec(&s.save())? == bytes);
                        s = Session::restore(&index, serde_json::from_slice(&bytes)?)
                            .with_context(|| {
                                format!(
                                    "{name} altered={altered} restore {checkpoint:?} slice {slice}"
                                )
                            })?;
                        ensure!(serde_json::to_vec(&s.save())? == bytes);
                        println!("{name} altered={altered}: restored {checkpoint:?}");
                    }
                    match s.advance_load_completion(camera, budget).with_context(|| {
                        format!(
                            "{name} altered={altered} budget={budget} slice={slice} call={call:?}"
                        )
                    })? {
                        Progress::Pending => (),
                        Progress::Complete => {
                            done = true;
                            break;
                        }
                        Progress::Environment => {
                            if budget == 1 && restores.insert("environment") {
                                let bytes = serde_json::to_vec(&s.save())?;
                                ensure!(s.step().is_err() && s.begin_selection(None).is_err());
                                ensure!(serde_json::to_vec(&s.save())? == bytes);
                                s = Session::restore(&index, serde_json::from_slice(&bytes)?)?;
                                ensure!(serde_json::to_vec(&s.save())? == bytes);
                                println!("{name} altered={altered}: restored environment");
                            }
                            s.acknowledge_absent_terrain_renderer()?;
                            ensure!(s.acknowledge_absent_terrain_renderer().is_err());
                        }
                        Progress::Notification(service) => {
                            let reply = match &service {
                                Service::View(_) => Reply::View(
                                    s.host.presentation.presenter.as_ref().unwrap().view.clone(),
                                ),
                                Service::Camera(request) => Reply::Camera(
                                    super::notification_session::device_reply(request)?,
                                ),
                                Service::Listener(call)
                                    if matches!(
                                        call.request,
                                        Listener::ViewerZoom { .. }
                                            | Listener::ViewerRotation { .. }
                                            | Listener::UiFlags { .. }
                                            | Listener::UiRefresh
                                            | Listener::PanelRefresh
                                            | Listener::SharedInterface
                                            | Listener::PanelBefore
                                            | Listener::PanelAfter
                                    ) =>
                                {
                                    Reply::Done
                                }
                                other => anyhow::bail!("native load work escaped: {other:?}"),
                            };
                            services.push(serde_json::to_value(&service)?);
                            s.acknowledge_house_notification(&service, reply)?;
                        }
                    }
                }
                ensure!(
                    done && !s.load_completion_pending()
                        && !s.host.presentation.awaiting_house_open
                );
                ensure!(
                    before
                        == (
                            s.simulation.vm.tick,
                            s.simulation.vm.rng_state,
                            s.simulation.vm.lot_rng_state
                        )
                );
                let retained = s.simulation.vm.world.objects.get(&target);
                if altered {
                    // The native factory reuses the first free object ID:
                    // Newbie's tutorial replaces the removed resident at that
                    // ID. Check identity, not whether the ID is occupied.
                    if let Some(replacement) = retained {
                        ensure!(name == "newbie" && replacement.person.is_none());
                        ensure!(
                            replacement.definition.as_ref().unwrap().guid
                                == sims_vm::load_completion::CONTROLLER
                        );
                    }
                } else {
                    ensure!(
                        retained.and_then(|o| o.definition.as_ref()).unwrap().guid == original_guid
                    );
                }
                if budget == 1 {
                    // Visitor refresh completes synchronously within a slice;
                    // only retained calls are observable in this log. Native
                    // ordering is compared separately at every callee boundary.
                    ensure!(calls.contains(&Request::HouseOpen));
                    if altered {
                        ensure!(
                            calls.contains(&Request::Delete(target)),
                            "cleanup failed to remove counterfactual"
                        );
                    }
                }
                let bytes = serde_json::to_vec(&s.save())?;
                ensure!(s.advance_load_completion(camera, budget)? == Progress::Complete);
                ensure!(serde_json::to_vec(&s.save())? == bytes);
                runs.push(json!({"budget":budget,"restores":restores.len(),"checkpoints":restores,"calls":calls,"services":services,"objects":s.simulation.vm.world.objects.len(),"crt_rng":s.simulation.vm.crt_rng_state}));
                finished.push(s.save());
            }
            ensure!(runs[0]["services"] == runs[1]["services"]);
            ensure!(
                serde_json::to_vec(&finished[0])? == serde_json::to_vec(&finished[1])?,
                "{name} cleanup state changes with budget"
            );
            let mut a = Session::restore(&index, finished.remove(0))?;
            let mut b = Session::restore(&index, finished.remove(0))?;
            let mut ticks = 0;
            let mut boundary = None;
            for _ in 0..10 {
                match (a.step(), b.step()) {
                    (
                        Ok(sims_runtime::original_session::Progress::Dialog),
                        Ok(sims_runtime::original_session::Progress::Dialog),
                    ) if name == "newbie" => {
                        let dialogs = &a.simulation.vm.world.dialogs;
                        let record = dialogs
                            .records
                            .iter()
                            .find(|r| Some(r.id) == dialogs.modal)
                            .context("tutorial modal absent")?;
                        ensure!(record.operands[5] & 127 == 4 && record.buttons.len() == 1);
                        ensure!(record.presentation.is_some_and(|p| p.hide_title));
                        ensure!(record.bitmap_icon == Some(5018));
                        boundary = Some(
                            json!({"kind":"tutorial_dialog","id":record.id,"mode":4,"tick":a.simulation.vm.tick}),
                        );
                        break;
                    }
                    (Ok(x), Ok(y)) => {
                        ensure!(x == sims_runtime::original_session::Progress::Tick && x == y);
                        ticks += 1;
                    }
                    (Err(x), Err(y)) if name == "newbie" && altered => {
                        let message = format!("{x:#}");
                        ensure!(message == format!("{y:#}"));
                        // Continuing this counterfactual now exposes a spatial
                        // service boundary that the former Dialog fault hid.
                        // Preserve the exact state rather than invent a body
                        // or conflate this ID with the removed resident's ID.
                        ensure!(message == "object #137 has no spatial body", "{message}");
                        let bytes = serde_json::to_vec(&a.save())?;
                        std::fs::write(
                            root.join("cache/native-tutorial-removal-boundary.json"),
                            bytes,
                        )?;
                        boundary = Some(json!({"message":message,"tick":a.simulation.vm.tick,
                            "checkpoint":"cache/native-tutorial-removal-boundary.json"}));
                        break;
                    }
                    (Err(x), Err(y)) if name == "goth" && altered => {
                        let message = format!("{x:#}");
                        ensure!(message == format!("{y:#}"));
                        // Removing a resident changes subsequent routing.
                        // Retain the protected-search boundary without
                        // supplying fabricated empty geometry.
                        ensure!(
                            message.starts_with("original-lot path construction required:")
                                && message.contains(
                                    "full routing wrapper configuration/publication remains UNVERIFIED"
                                )
                        );
                        boundary = Some(json!({"message":message,"tick":a.simulation.vm.tick}));
                        break;
                    }
                    (x, y) => {
                        anyhow::bail!("{name}: continuation differs or failed: {x:?} / {y:?}")
                    }
                }
            }
            ensure!(serde_json::to_vec(&a.save())? == serde_json::to_vec(&b.save())?);
            if name == "newbie" && !altered {
                let checkpoint = serde_json::to_vec(&a.save())?;
                std::fs::write(root.join("cache/native-tutorial-session.json"), &checkpoint)?;
                a = Session::restore(&index, serde_json::from_slice(&checkpoint)?)?;
                ensure!(serde_json::to_vec(&a.save())? == checkpoint);
            }
            let mut renderer = sims_render::original_house::OriginalHouse::default();
            let mut other = sims_render::original_house::OriginalHouse::default();
            for rotation in 0..2 {
                let floor = if name == "goth" && rotation == 1 {
                    2
                } else {
                    1
                };
                let camera = sims_render::original_house::OriginalHouse::camera(
                    &a, 960, 720, rotation, 0.55, floor,
                );
                let pixels = sims_render::software::render(&renderer.scene(&a, camera)?);
                ensure!(pixels == sims_render::software::render(&other.scene(&b, camera)?));
                if !altered {
                    image::imageops::replace(
                        &mut gallery,
                        &pixels,
                        column as i64 * 960,
                        i64::from(rotation) * 720,
                    );
                }
            }
            println!(
                "{name} altered={altered}: cleanup, {ticks} ticks and two CPU views match; boundary={boundary:?}"
            );
            report.push(json!({"house":name,"altered":altered,"removed":if altered {Some(target)} else {None},"runs":runs,"continued_ticks":ticks,"boundary":boundary,"pixel_views":2,"native_terrain_renderer":"absent explicit headless input"}));
        }
    }
    gallery.save(root.join("cache/original-load-completion-gallery.png"))?;
    std::fs::write(
        root.join("cache/native-load-completion-houses.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
