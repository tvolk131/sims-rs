//! Saved UI -> camera -> shared F9 on the original tutorial household.
//! Device/audio/tool replies are explicit test sinks, not a live renderer.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{
    Saved, Session,
    camera_command::{Progress, State},
    notifications,
};
use sims_sim::{
    lot_presenter::{camera, camera_command::Ui, transition},
    pose::portrait::view::Input,
};
use sims_vm::commander::house::Request;
use std::collections::BTreeSet;

pub fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/native-ui-effect-navigation-iced-save.json"),
    )?)?;
    let saved: Saved = serde_json::from_value(wrapper["session"].clone())?;
    let mut seed = Session::restore(&index, saved)?;
    let zoom = seed
        .host
        .presentation
        .presenter
        .as_ref()
        .context("tutorial view absent")?
        .view
        .zoom;
    ensure!(zoom == 2 && seed.simulation.vm.world.globals[&2] == 2);
    // The historical F6 checkpoint used a synthetic view near an empty corner.
    // Bind the test host's actual displayed house view before the command; no
    // simulated object, script variable or original save bytes are changed.
    let displayed = sims_render::original_house::OriginalHouse::camera(&seed, 1000, 740, 0, 0.5, 1);
    let input = sims_render::dialog::client::Inputs::camera(displayed, 64)?.view;
    let p = seed.host.presentation.presenter.as_ref().unwrap();
    let mut view = p.view.clone();
    view.origin = input.center;
    let inputs = p.inputs.clone();
    let camera = p.camera.clone().unwrap();
    match p.order.clone() {
        Some(order) => seed.configure_house_view(view, inputs, order, camera)?,
        None => seed.configure_registry_house_view(view, inputs, camera)?,
    }
    seed.bind_camera_ui(Ui {
        flags: 0x81,
        zoom,
        ..Default::default()
    })?;
    let original = seed.save();
    let mut runs = vec![];
    let mut finished = vec![];
    let mut zoomed = vec![];
    let mut images = std::collections::BTreeMap::new();
    for budget in [1, 10000] {
        let mut s = Session::restore(&index, original.clone())?;
        let mut renderer = sims_render::original_house::OriginalHouse::default();
        let before = (
            s.simulation.vm.tick,
            s.simulation.vm.rng_state,
            s.simulation.vm.lot_rng_state,
            s.simulation.vm.crt_rng_state,
        );
        let mut services = vec![];
        let mut restored = BTreeSet::new();
        for requested in [3, 1, 2] {
            let previous_global = s.simulation.vm.world.globals[&2];
            s.begin_camera_zoom(requested, 0)?;
            let mut done = false;
            for slice in 0..30000 {
                let owner = s.host.presentation.camera_command.as_ref().unwrap();
                owner.validate(&s)?;
                let decoded: State = serde_json::from_slice(&serde_json::to_vec(owner)?)?;
                ensure!(&decoded == owner);
                decoded.validate(&s)?;
                let view = &s.host.presentation.presenter.as_ref().unwrap().view;
                let input = Input {
                    viewport: [0, 0, 1000, 740],
                    center: view.origin,
                    zoom: view.zoom,
                    rotation: view.display_rotation,
                };
                let progress = s.advance_camera_command(input, budget)?;
                if progress == Progress::Complete {
                    done = true;
                    break;
                }
                if progress == Progress::Pending {
                    continue;
                }
                let key = match &progress {
                    Progress::Camera(camera::Service::Transition(
                        transition::Request::DepthTable { .. },
                    )) => "depth",
                    Progress::Notification(_) => "notification",
                    Progress::RefreshAudio => "audio",
                    _ => "camera",
                };
                if budget == 1 && restored.insert(key) {
                    let bytes = serde_json::to_vec(&s.save())?;
                    ensure!(
                        s.step().is_err()
                            && s.begin_selection(None).is_err()
                            && s.begin_indicator_refresh().is_err()
                    );
                    ensure!(
                        s.advance_selection(input, 1).is_err()
                            && s.service_selection_assets(input, 1).is_err()
                    );
                    ensure!(
                        s.service_removal_graphic(input).is_err()
                            && s.advance_view_refresh(input, 1).is_err()
                    );
                    ensure!(
                        s.advance_house_notification(input, 1).is_err()
                            && s.begin_camera_zoom(1, 0).is_err()
                    );
                    ensure!(
                        s.bind_camera_ui(Ui {
                            flags: 0,
                            zoom: 1,
                            ..Default::default()
                        })
                        .is_err()
                    );
                    ensure!(
                        s.advance_postload(input, 1).is_err()
                            && s.advance_load_completion(input, 1).is_err()
                    );
                    ensure!(
                        serde_json::to_vec(&s.save())? == bytes,
                        "rejected interleaving changed saved state"
                    );
                    s = Session::restore(&index, serde_json::from_slice(&bytes)?)
                        .with_context(|| format!("camera restore at {requested}/{slice}/{key}"))?;
                    ensure!(serde_json::to_vec(&s.save())? == bytes);
                    ensure!(s.advance_camera_command(input, budget)? == progress);
                    let mut corrupt: serde_json::Value = serde_json::from_slice(&bytes)?;
                    // Validation also rejects a cursor retained across VM time.
                    let state = &mut corrupt["view"]["camera_command"];
                    ensure!(state.is_object(), "saved command absent");
                    state["tick"] = serde_json::json!(before.0 + 1);
                    ensure!(Session::restore(&index, serde_json::from_value(corrupt)?).is_err());
                }
                services.push(serde_json::json!({"zoom":requested,"kind":format!("{progress:?}"),"global":s.simulation.vm.world.globals[&2]}));
                match progress {
                    Progress::Camera(service) => {
                        ensure!(
                            !matches!(
                                &service,
                                camera::Service::Transition(transition::Request::Broadcast { .. })
                            ),
                            "broadcast escaped shared dispatcher"
                        );
                        let reply = super::notification_session::device_reply(&service)?;
                        s.acknowledge_camera_command(&service, reply)?;
                        ensure!(
                            s.acknowledge_camera_command(&service, camera::Reply::Done)
                                .is_err(),
                            "duplicate camera reply accepted"
                        );
                    }
                    Progress::Notification(service) => {
                        ensure!(
                            matches!(&service, notifications::Service::Listener(call) if call.request == Request::ViewerZoom { value: requested })
                        );
                        ensure!(
                            s.simulation.vm.world.globals[&2] == previous_global,
                            "zoom global changed before Globals listener in retained registry order"
                        );
                        ensure!(
                            s.acknowledge_house_notification(&service, notifications::Reply::Done)
                                .is_err()
                        );
                        s.acknowledge_camera_notification(&service, notifications::Reply::Done)?;
                        ensure!(
                            s.acknowledge_camera_notification(&service, notifications::Reply::Done)
                                .is_err()
                        );
                    }
                    Progress::RefreshAudio => {
                        s.complete_camera_audio()?;
                        ensure!(s.complete_camera_audio().is_err());
                    }
                    _ => unreachable!(),
                }
            }
            ensure!(done && !s.camera_command_pending());
            ensure!(s.host.presentation.presenter.as_ref().unwrap().view.zoom == requested);
            ensure!(s.simulation.vm.world.globals[&2] == requested as i16);
            if requested == 3 {
                zoomed.push(s.save());
            }
            let before_render = serde_json::to_vec(&s.save())?;
            let camera = sims_render::original_house::OriginalHouse::native_camera(&s)?;
            let pixels = sims_render::software::render(&renderer.scene(&s, camera)?);
            ensure!(serde_json::to_vec(&s.save())? == before_render);
            if budget == 1 {
                image::save_buffer(
                    root.join(format!("cache/native-camera-command-zoom{requested}.png")),
                    &pixels,
                    camera.width,
                    camera.height,
                    image::ColorType::Rgba8,
                )?;
                images.insert(requested, pixels);
            } else {
                ensure!(
                    images[&requested] == pixels,
                    "restored camera frame differs"
                );
            }
        }
        ensure!(
            before
                == (
                    s.simulation.vm.tick,
                    s.simulation.vm.rng_state,
                    s.simulation.vm.lot_rng_state,
                    s.simulation.vm.crt_rng_state
                )
        );
        if budget == 1 {
            ensure!(restored.len() == 4, "missing nested restores {restored:?}");
        }
        runs.push(serde_json::json!({"budget":budget,"restored":restored,"services":services,"ticks_rng_unchanged":true}));
        finished.push(s.save());
    }
    ensure!(runs[0]["services"] == runs[1]["services"]);
    ensure!(
        serde_json::to_vec(&finished[0])? == serde_json::to_vec(&finished[1])?,
        "slice budget/restore changed result"
    );
    let mut a = Session::restore(&index, finished.remove(0))?;
    let mut b = Session::restore(&index, finished.remove(0))?;
    for _ in 0..10 {
        ensure!(a.step()? == sims_runtime::original_session::Progress::Tick);
        ensure!(b.step()? == sims_runtime::original_session::Progress::Tick);
    }
    ensure!(serde_json::to_vec(&a.save())? == serde_json::to_vec(&b.save())?);
    let mut tutorial_runs = vec![];
    let mut tutorial_finished = vec![];
    for (n, saved) in zoomed.into_iter().enumerate() {
        let mut s = Session::restore(&index, saved)?;
        let mut client = sims_render::dialog::client::Client::restore(
            &s,
            serde_json::from_value(wrapper["dialog_client"].clone())?,
        )?;
        let start_tick = s.simulation.vm.tick;
        let mut calls = vec![];
        let mut reached = false;
        for _ in 0..1000 {
            if client.pending() {
                client.advance(&mut s, if n == 0 { 1 } else { 256 })?;
                continue;
            }
            client.collect(&s)?;
            if s.simulation
                .vm
                .world
                .dialogs
                .records
                .iter()
                .any(|r| r.bitmap_icon == Some(5020))
            {
                reached = true;
                break;
            }
            match s.step()? {
                sims_runtime::original_session::Progress::Tick => (),
                sims_runtime::original_session::Progress::UiEffect(thread) => {
                    let sims_vm::Status::AwaitingUiEffect(call) =
                        &s.simulation.vm.threads[&thread].status
                    else {
                        unreachable!()
                    };
                    let request = call.request();
                    match request.kind {
                        sims_vm::ui_effect::Kind::Control => ensure!(
                            request.value == 2013 && !request.enabled,
                            "unexpected tutorial UI effect {request:?}"
                        ),
                        _ => ensure!(
                            sims_vm::ui_effect::neighbor_guid(
                                &s.simulation.vm.world,
                                request.value
                            )
                            .map_err(anyhow::Error::msg)?
                            .is_none()
                        ),
                    }
                    calls.push(request);
                    s.simulation
                        .vm
                        .complete_ui_effect(&s.program, thread, request)
                        .map_err(anyhow::Error::msg)?;
                }
                sims_runtime::original_session::Progress::DialogConstruction(thread) => {
                    let camera = sims_render::original_house::OriginalHouse::native_camera(&s)?;
                    client.begin(
                        &s,
                        thread,
                        sims_render::dialog::client::Inputs::camera(camera, 64)?,
                    )?;
                }
                other => anyhow::bail!("tutorial camera continuation stopped at {other:?}"),
            }
        }
        ensure!(reached, "original tutorial did not observe its zoom change");
        ensure!(
            !calls.is_empty(),
            "original tutorial did not clear its zoom highlight"
        );
        let record = s
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .find(|r| r.bitmap_icon == Some(5020))
            .unwrap()
            .clone();
        tutorial_runs.push(serde_json::json!({"ticks":s.simulation.vm.tick-start_tick,"ui_calls":calls,"next_prompt":record}));
        let result = serde_json::json!({"session":s.save(),"dialog_client":client.save(&s)?,"floor":1,"zoom":1.0,"resident":wrapper["resident"],"selected":null,"tutorial_highlight":null});
        tutorial_finished.push(serde_json::to_vec(&result)?);
    }
    ensure!(tutorial_runs[0] == tutorial_runs[1]);
    ensure!(
        tutorial_finished[0] == tutorial_finished[1],
        "restored tutorial continuation differs"
    );
    std::fs::write(
        root.join("cache/native-camera-command-tutorial-iced-save.json"),
        &tutorial_finished[0],
    )?;
    std::fs::write(
        root.join("cache/native-camera-command-session-report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"host_view_input":input,"runs":runs,"tutorial":tutorial_runs,"final_tick":a.simulation.vm.tick,"continued_ticks":10,"equal_restored_pixels":true,"device_audio_tool_replies":"explicit test sinks; live host not implemented"}),
        )?,
    )?;
    println!(
        "Original tutorial camera: three zoom transitions, four full nested restores, shared F9, unchanged clocks/RNG; 10 matching subsequent ticks"
    );
    Ok(())
}
