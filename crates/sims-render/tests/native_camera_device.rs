use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, original_house::OriginalHouse, software};
use sims_runtime::original_session::{
    Saved, Session,
    camera_command::{Change, Progress},
};
use sims_sim::{
    lot_presenter::{camera, camera_command::Ui, transition},
    pose::portrait::view::Input,
};
use std::collections::BTreeSet;

#[test]
#[ignore = "requires private original assets and tutorial checkpoint"]
fn actual_camera_device_preserves_callbacks_and_restores_published_surfaces() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/native-ui-effect-navigation-iced-save.json"),
    )?)?;
    let seed: Saved = serde_json::from_value(wrapper["session"].clone())?;
    let mut seed = Session::restore(&index, seed)?;
    // These are the replacement frontend's actual property/clock inputs. The
    // historical diagnostic checkpoint used synthetic device scales/motion.
    let c = OriginalHouse::camera(&seed, 1000, 740, 0, 0.5, 1);
    let input = sims_render::dialog::client::Inputs::camera(c, 64)?.view;
    let p = seed
        .host
        .presentation
        .presenter
        .as_ref()
        .context("tutorial view absent")?;
    let mut view = p.view.clone();
    view.origin = input.center;
    view.pending = [0, 0];
    let mut retained = p.camera.clone().unwrap();
    let settings = &mut retained.settings;
    settings.clock = 0;
    settings.moving = 0;
    settings.motion.active = 0;
    settings.motion.x = [0; 4];
    settings.motion.y = [0; 4];
    settings.ui_present = false;
    settings.renderer_ready = true;
    settings.graphics_enabled = true;
    for zoom in 1..=3 {
        settings.depth_scales[zoom as usize] = Input { zoom, ..input }
            .build()
            .map_err(anyhow::Error::msg)?
            .far
            .to_bits();
    }
    let inputs = p.inputs.clone();
    match p.order.clone() {
        Some(order) => seed.configure_house_view(view, inputs, order, retained)?,
        None => seed.configure_registry_house_view(view, inputs, retained)?,
    }
    seed.bind_camera_ui(Ui {
        flags: 0,
        zoom: 2,
        rotation: 0,
        floor: 1,
    })?;
    let initial = seed.save();
    let mut completed = vec![];
    let mut reports = vec![];
    let mut frames = vec![];
    let mut tutorial_seeds = vec![];
    for budget in [1, 10000] {
        let mut s = Session::restore(&index, initial.clone())?;
        let mut device = Device::new(&s)?;
        let before = (
            s.simulation.vm.tick,
            s.simulation.vm.rng_state,
            s.simulation.vm.lot_rng_state,
            s.simulation.vm.crt_rng_state,
        );
        let mut restored = BTreeSet::new();
        let mut trace = vec![];
        let mut pixels = vec![];
        for change in [
            Change::Zoom(3),
            Change::Zoom(1),
            Change::Rotation(1),
            Change::Rotation(0),
            Change::Floor(2),
            Change::Floor(1),
            Change::Zoom(2),
        ] {
            device.begin(&mut s, change)?;
            let mut done = false;
            for _ in 0..30000 {
                let p = &s.host.presentation.presenter.as_ref().unwrap().view;
                let input = Input {
                    center: p.origin,
                    zoom: p.zoom,
                    rotation: p.display_rotation,
                    viewport: [0, 0, 1000, 740],
                };
                let progress = s.advance_camera_command(input, budget)?;
                if progress == Progress::Complete {
                    done = true;
                    break;
                }
                if progress == Progress::Pending {
                    if budget == 1
                        && s.host
                            .presentation
                            .camera_command
                            .as_ref()
                            .and_then(|c| c.pending_person())
                            .is_some()
                        && restored.insert("people")
                    {
                        let old = software::render(device.scene());
                        let a = serde_json::to_vec(&s.save())?;
                        let b = serde_json::to_vec(&device.save(&s)?)?;
                        ensure!(s.step().is_err());
                        s = Session::restore(&index, serde_json::from_slice(&a)?)?;
                        device = Device::restore(&s, serde_json::from_slice(&b)?)?;
                        ensure!(serde_json::to_vec(&s.save())? == a);
                        ensure!(software::render(device.scene()) == old);
                    }
                    continue;
                }
                let key = match &progress {
                    Progress::Camera(camera::Service::Transition(
                        transition::Request::DepthTable { .. },
                    )) => "depth",
                    Progress::Camera(camera::Service::Transition(
                        transition::Request::FlushSurface { .. },
                    )) => "flush",
                    Progress::Notification(_) => "notification",
                    Progress::RefreshControls => "controls",
                    _ => "other",
                };
                trace.push(format!("{progress:?}"));
                if budget == 1 && key != "other" && restored.insert(key) {
                    let published = software::render(device.scene());
                    let a = serde_json::to_vec(&s.save())?;
                    let b = serde_json::to_vec(&device.save(&s)?)?;
                    ensure!(s.step().is_err());
                    ensure!(
                        serde_json::to_vec(&s.save())? == a,
                        "rejected tick changed command state"
                    );
                    s = Session::restore(&index, serde_json::from_slice(&a)?)?;
                    device = Device::restore(&s, serde_json::from_slice(&b)?)?;
                    ensure!(serde_json::to_vec(&s.save())? == a);
                    ensure!(serde_json::to_vec(&device.save(&s)?)? == b);
                    ensure!(
                        software::render(device.scene()) == published,
                        "restore published an unfinished camera frame"
                    );
                }
                let old_global = s.simulation.vm.world.globals[&2];
                let old_publication = device.published_generation();
                let old_vertices = if key == "depth" {
                    Some(
                        device
                            .scene()
                            .vertices
                            .iter()
                            .map(|v| v.position)
                            .collect::<Vec<_>>(),
                    )
                } else {
                    None
                };
                device.advance(&mut s, budget)?;
                if let Some(old) = old_vertices {
                    ensure!(device.generation() > old_publication);
                    ensure!(device.published_generation() == old_publication);
                    ensure!(
                        device
                            .scene()
                            .vertices
                            .iter()
                            .map(|v| v.position)
                            .collect::<Vec<_>>()
                            == old
                    );
                }
                if key == "notification" {
                    ensure!(
                        s.simulation.vm.world.globals[&2] == old_global,
                        "Viewer completion bypassed Globals recipient"
                    );
                }
            }
            ensure!(done && !s.camera_command_pending());
            let before_render = serde_json::to_vec(&s.save())?;
            pixels.push(software::render(device.scene()));
            ensure!(serde_json::to_vec(&s.save())? == before_render);
            if matches!(change, Change::Zoom(3)) {
                tutorial_seeds.push((s.save(), device.save(&s)?));
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
        ensure!(device.controls().floor == 1 && device.controls().zoom == 2);
        ensure!(device.published_generation() == device.generation());
        if budget == 1 {
            ensure!(restored.len() == 5, "restores: {restored:?}");
        }
        completed.push((
            serde_json::to_vec(&s.save())?,
            serde_json::to_vec(&device.save(&s)?)?,
        ));
        frames.push(pixels);
        reports.push(serde_json::json!({"budget":budget,"restored":restored,"trace":trace,"generation":device.generation(),"ticks_rng_unchanged":true}));
    }
    ensure!(
        completed[0] == completed[1],
        "device budget/restore changed state"
    );
    ensure!(reports[0]["trace"] == reports[1]["trace"]);
    ensure!(frames[0] == frames[1], "restored device frames differ");
    ensure!(
        frames[0][0] != frames[0][1],
        "zoom did not change the real scene"
    );
    let mut gallery = image::RgbaImage::new(3000, 2220);
    for (i, frame) in frames[0].iter().enumerate() {
        image::imageops::replace(
            &mut gallery,
            frame,
            ((i % 3) * 1000) as i64,
            ((i / 3) * 740) as i64,
        );
    }
    gallery.save(root.join("cache/native-camera-device-gallery.png"))?;
    // The original script must observe F9 through Globals and advance itself.
    // The test owns the already-existing UI-effect/Dialog adapters; it never
    // writes tutorial variables, generates wait events or answers a prompt.
    let mut tutorial_results = vec![];
    let mut tutorial_reports = vec![];
    for (i, (saved, device_saved)) in tutorial_seeds.into_iter().enumerate() {
        let mut s = Session::restore(&index, saved)?;
        let mut device = Device::restore(&s, device_saved)?;
        let mut client = sims_render::dialog::client::Client::restore(
            &s,
            serde_json::from_value(wrapper["dialog_client"].clone())?,
        )?;
        let start = s.simulation.vm.tick;
        let mut highlight = Some(2013);
        let mut reached = false;
        for _ in 0..2000 {
            if client.pending() {
                client.advance(&mut s, if i == 0 { 1 } else { 256 })?;
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
                        sims_vm::ui_effect::Kind::Control => {
                            ensure!(
                                request.value == 2013 && !request.enabled,
                                "unexpected tutorial control {request:?}"
                            );
                            highlight = None;
                        }
                        _ => ensure!(
                            sims_vm::ui_effect::neighbor_guid(
                                &s.simulation.vm.world,
                                request.value
                            )
                            .map_err(anyhow::Error::msg)?
                            .is_none(),
                            "nonempty UI neighbor service"
                        ),
                    }
                    s.simulation
                        .vm
                        .complete_ui_effect(&s.program, thread, request)
                        .map_err(anyhow::Error::msg)?;
                }
                sims_runtime::original_session::Progress::DialogConstruction(thread) => {
                    client.begin(
                        &s,
                        thread,
                        sims_render::dialog::client::Inputs::camera(
                            OriginalHouse::native_camera(&s)?,
                            64,
                        )?,
                    )?;
                }
                other => anyhow::bail!("camera tutorial stopped at {other:?}"),
            }
        }
        ensure!(
            reached && highlight.is_none(),
            "original tutorial did not observe the zoom event"
        );
        device.refresh(&s)?;
        let pixels = software::render(device.scene());
        if i == 0 {
            pixels.save(root.join("cache/native-camera-device-tutorial.png"))?;
        } else {
            ensure!(
                pixels
                    == image::open(root.join("cache/native-camera-device-tutorial.png"))?
                        .to_rgba8()
            );
        }
        let mut output = wrapper.clone();
        output["session"] = serde_json::to_value(s.save())?;
        output["camera_device"] = serde_json::to_value(device.save(&s)?)?;
        output["dialog_client"] = serde_json::to_value(client.save(&s)?)?;
        output["floor"] = serde_json::json!(1);
        output["zoom"] = serde_json::json!(1.0);
        output["tutorial_highlight"] = serde_json::json!(highlight);
        tutorial_results.push(serde_json::to_vec(&output)?);
        tutorial_reports.push(serde_json::json!({"tick":s.simulation.vm.tick,"ticks":s.simulation.vm.tick-start,"next_bitmap":5020}));
    }
    ensure!(
        tutorial_results[0] == tutorial_results[1],
        "restored tutorial continuation differs"
    );
    std::fs::write(
        root.join("cache/native-camera-device-iced-save.json"),
        &tutorial_results[0],
    )?;
    std::fs::write(
        root.join("cache/native-camera-device-tutorial-report.json"),
        serde_json::to_vec_pretty(&tutorial_reports)?,
    )?;
    std::fs::write(
        root.join("cache/native-camera-device-report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    println!(
        "Seven device-owned camera changes, five full restores, equal published pixels and unchanged tick/RNG"
    );
    Ok(())
}
