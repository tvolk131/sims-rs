use super::*;
use clap::Parser;

#[test]
#[ignore = "original Newbie load-completion checkpoint and offscreen Iced rendering"]
fn original_viewer_preserves_and_answers_tutorial_dialog() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let index = Index::scan(&root)?;
    let checkpoint = repo.join("cache/native-tutorial-session.json");
    let saved: Saved = serde_json::from_slice(&std::fs::read(checkpoint)?)?;
    let source = saved.source.clone();
    let mut session = Session::restore(&index, saved)?;
    let client = DialogClient::new(&mut session)?;
    let resident = session.house.household.as_ref().unwrap().residents[0].object as i16;
    let save = repo.join("cache/native-ui-effect-iced-save.json");
    atomic_save(
        &save,
        &serde_json::to_vec(&serde_json::json!({
            "session":session.save(),"dialog_client":client.save(&session)?,
            "floor":1,"zoom":0.55,"resident":resident,"selected":null,
        }))?,
    )?;
    let args = Args::parse_from([
        "sims-room",
        "--original",
        &source,
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
        "--load",
        save.to_str().unwrap(),
    ]);
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(Arc::new(Ready {
        restore_index: None,
        session: Mutex::new(Some(session)),
        client: Mutex::new(Some(client)),
        device: Mutex::new(None),
        icons: Arc::new(dialog_view::Icons::load(&index)?),
    }))?;
    let s = viewer.session.as_ref().unwrap();
    let id = s
        .simulation
        .vm
        .world
        .dialogs
        .modal
        .context("tutorial modal absent")?;
    let record = s
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .find(|r| r.id == id)
        .unwrap();
    ensure!(record.operands[5] == 4 && record.buttons.len() == 1);
    ensure!(
        record
            .presentation
            .is_some_and(|p| p.hide_title && !p.large)
    );
    ensure!(record.bitmap_icon == Some(5018));
    ensure!(sims_content::dialog::private_bitmap(&index, &record.source, 5018)?.is_some());
    let mode = record.operands[5];
    let body = record.body.clone();
    let tick = s.simulation.vm.tick;
    viewer.save()?;
    let state = std::fs::read(&save)?;
    let modal = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(serde_json::to_vec(&viewer.session.as_ref().unwrap().save())? == modal);
    let _ = viewer.update(Message::DialogKey(65));
    for _ in 0..3 {
        let _ = viewer.update(Message::Step);
    }
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    ensure!(
        serde_json::to_vec(&viewer.session.as_ref().unwrap().save())? == modal,
        "unanswered tutorial changed VM state"
    );
    let image = repo.join("cache/native-ui-effect-welcome-iced.png");
    render_headless_element(&root, &image, viewer.view())?;
    ensure!(
        serde_json::to_vec(&viewer.session.as_ref().unwrap().save())? == modal,
        "headless rendering delivered a response"
    );
    let mut answered = None;
    for key in [None, Some(13), Some(27), Some(32)] {
        viewer.load(&save)?;
        viewer.refresh()?;
        let _ = viewer.update(key.map_or(Message::Dialog(id, 2), Message::DialogKey));
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
        ensure!(
            viewer
                .session
                .as_ref()
                .unwrap()
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .is_none()
        );
        let current = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
        if let Some(expected) = &answered {
            ensure!(&current == expected);
        } else {
            answered = Some(current.clone());
        }
        let _ = viewer.update(Message::DialogKey(13));
        ensure!(
            serde_json::to_vec(&viewer.session.as_ref().unwrap().save())? == current,
            "duplicate key changed the answered dialog"
        );
    }
    // Capture the real pending call before the presentation host runs.
    let mut ticks = 0;
    let thread = loop {
        ensure!(ticks < 3, "tutorial did not request its UI effect");
        match viewer.session.as_mut().unwrap().step()? {
            Progress::Tick => ticks += 1,
            Progress::UiEffect(thread) => break thread,
            other => anyhow::bail!("unexpected tutorial progress: {other:?}"),
        }
    };
    let pending = viewer.session.as_ref().unwrap().save();
    let pending_bytes = serde_json::to_vec(&pending)?;
    let sims_vm::Status::AwaitingUiEffect(call) =
        &viewer.session.as_ref().unwrap().simulation.vm.threads[&thread].status
    else {
        unreachable!()
    };
    let request = call.request();
    ensure!(
        request.kind == sims_vm::ui_effect::Kind::Control
            && request.value == 2013
            && request.enabled
    );
    let pending_path = repo.join("cache/native-ui-effect-pending-iced-save.json");
    viewer.args.save = pending_path.clone();
    viewer.save()?;
    viewer.load(&pending_path)?;
    viewer.refresh()?;
    let _ = viewer.update(Message::Rotate);
    let _ = viewer.update(Message::ZoomIn);
    ensure!(serde_json::to_vec(&viewer.session.as_ref().unwrap().save())? == pending_bytes);
    ensure!(viewer.tutorial_highlight.is_none());
    // Exercise the second saved boundary on a separate branch, leaving
    // the live viewer to perform the first highlight itself.
    let mut branch = Session::restore(&index, pending)?;
    branch
        .simulation
        .vm
        .complete_ui_effect(&branch.program, thread, request)
        .map_err(anyhow::Error::msg)?;
    let sims_vm::Status::AwaitingUiEffect(second) = &branch.simulation.vm.threads[&thread].status
    else {
        unreachable!()
    };
    let second = second.request();
    ensure!(second.kind == sims_vm::ui_effect::Kind::Portrait && second.value == 2013);
    ensure!(
        sims_vm::ui_effect::neighbor_guid(&branch.simulation.vm.world, second.value)
            .map_err(anyhow::Error::msg)?
            .is_none()
    );
    let second_bytes = serde_json::to_vec(&branch.save())?;
    branch = Session::restore(&index, branch.save())?;
    ensure!(serde_json::to_vec(&branch.save())? == second_bytes);
    ensure!(branch.simulation.vm.advance_tick().is_err());
    ensure!(
        branch
            .simulation
            .vm
            .complete_ui_effect(&branch.program, thread, request)
            .is_err()
    );
    ensure!(serde_json::to_vec(&branch.save())? == second_bytes);

    // No wait-event input is fabricated: the original script remains at
    // its navigation prompt while ordinary ticks continue.
    let mut boundary = serde_json::Value::Null;
    for _ in 0..99 {
        match viewer.step() {
            Ok(true) => ticks += 1,
            Ok(false) => {
                boundary = serde_json::json!({"status":viewer.status,
                    "modal":viewer.session.as_ref().unwrap().simulation.vm.world.dialogs.modal});
                break;
            }
            Err(error) => {
                boundary = serde_json::json!({"error":format!("{error:#}")});
                break;
            }
        }
    }
    ensure!(
        ticks == 100 && boundary.is_null(),
        "{ticks} ticks: {boundary}"
    );
    ensure!(viewer.tutorial_highlight == Some(2013));
    let records = &viewer
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .dialogs
        .records;
    ensure!(records.len() == 1 && !records[0].modal() && records[0].status == 1);
    ensure!(records[0].bitmap_icon == Some(5019) && records[0].body.contains("zoom-in button"));
    let navigation = records[0].clone();
    viewer.refresh()?;
    let highlighted_image = repo.join("cache/native-ui-effect-iced.png");
    render_headless_element(&root, &highlighted_image, viewer.view())?;
    let continued = viewer.session.as_ref().unwrap().save();
    let restored = Session::restore(&index, continued.clone())?;
    ensure!(serde_json::to_vec(&restored.save())? == serde_json::to_vec(&continued)?);
    let navigation_save = repo.join("cache/native-ui-effect-navigation-iced-save.json");
    viewer.args.save = navigation_save.clone();
    viewer.save()?;
    viewer.tutorial_highlight = None;
    viewer.load(&navigation_save)?;
    viewer.refresh()?;
    ensure!(viewer.tutorial_highlight == Some(2013));
    let restored_image = repo.join("cache/native-ui-effect-restored-iced.png");
    render_headless_element(&root, &restored_image, viewer.view())?;
    ensure!(
        std::fs::read(&restored_image)? == std::fs::read(&highlighted_image)?,
        "restored tutorial frame differs"
    );
    let before_zoom = viewer.zoom;
    let _ = viewer.update(Message::ZoomIn);
    ensure!(viewer.error.is_none() && viewer.zoom > before_zoom);
    ensure!(
        viewer
            .session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .dialogs
            .records
            == vec![navigation.clone()]
    );
    ensure!(
        std::fs::read(&save)? == state,
        "response test overwrote its unanswered checkpoint"
    );
    std::fs::write(
        repo.join("cache/native-ui-effect-iced-report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"source":source,"mode":mode,
            "body":body,"tick":tick,"explicit_responses":["OK", "Enter", "Escape", "Space"],
            "continued_ticks":ticks,"boundary":boundary,"image":image,
            "continued_tick":restored.simulation.vm.tick,"highlight":viewer.tutorial_highlight,"highlighted_image":highlighted_image,
            "pending_call":request,"pending_save":pending_path,"navigation_save":navigation_save,"navigation":navigation,
            "restored_image":restored_image,"equal_restored_pixels":true,"zoom_control_changes_view":true,"wait_event_delivered":false}))?,
    )?;
    println!(
        "Original tutorial: preserved modal/save, four equivalent explicit responses; {ticks} further ticks; boundary={boundary}"
    );
    Ok(())
}

#[test]
#[ignore = "current-program brush continuation checkpoint; headless Metal rendering"]
fn original_viewer_restores_shared_brush_texture() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let source = repo.join("cache/mashuga-texture-continuation/brush-loaded.json");
    let save = repo.join("cache/mashuga-texture-continuation/iced-save.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData2/Houses/House09.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        source.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(ready)?;
    let brush = "ExpansionShared/ExpansionShared.far::Accessories\\brush.bmp";
    let brush_hash = "7d1d5b9e7bbbc6d1746b08ba8fd3f833f7fdf3473e9f6f234b815549d0979001";
    let mut images = vec![];
    for floor in [1, 2] {
        let _ = viewer.update(Message::Floor(floor));
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
        let s = viewer.session.as_ref().unwrap();
        ensure!(s.host.characters.values().any(|a| {
            a.avatar
                .avatar
                .sources
                .get(brush)
                .is_some_and(|h| h == brush_hash)
        }));
        let before = serde_json::to_value(s.save())?;
        let pixels = sims_render::software::render(viewer.scene.as_ref().unwrap());
        viewer.save()?;
        viewer.load(&save)?;
        viewer.refresh()?;
        ensure!(serde_json::to_value(viewer.session.as_ref().unwrap().save())? == before);
        ensure!(sims_render::software::render(viewer.scene.as_ref().unwrap()) == pixels);
        let image = repo.join(format!(
            "cache/mashuga-texture-continuation/iced-floor-{floor}.png"
        ));
        render_headless_element(&root, &image, viewer.view())?;
        ensure!(serde_json::to_value(viewer.session.as_ref().unwrap().save())? == before);
        images.push(image);
    }
    std::fs::write(
        repo.join("cache/mashuga-texture-continuation/iced-report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source":source,"images":images,"source_sha256":brush_hash,
            "restored_session_and_pixels_equal":true,"render_does_not_advance":true,
            "scope":"Current v82 diagnostic, explicit empty coordinates and inferred routing; legacy camera profile."
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "fresh current-program Mashuga replay checkpoint; headless Metal rendering"]
fn original_viewer_fresh_replay_death_dialog_survives_save_and_input() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let source = repo.join("cache/mashuga-fresh-v82/modal-6.json");
    let save = repo.join("cache/mashuga-fresh-v82/iced-save.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData2/Houses/House09.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        source.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(ready)?;
    let session = viewer.session.as_ref().unwrap();
    let tick = session.simulation.vm.tick;
    let id = session
        .simulation
        .vm
        .world
        .dialogs
        .modal
        .context("modal absent")?;
    let record = session
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .find(|r| r.id == id)
        .context("modal record absent")?;
    ensure!(
        record.title == "Rest In Peace"
            && record.body.contains("Frankie")
            && record.portrait.is_some()
    );
    let before = serde_json::to_value(session.save())?;
    let resources = serde_json::to_value(viewer.client.as_ref().unwrap().save(session)?)?;
    let pixels = sims_render::software::render(viewer.scene.as_ref().unwrap());
    let _ = viewer.update(Message::Step);
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    ensure!(
        serde_json::to_value(viewer.session.as_ref().unwrap().save())? == before,
        "unanswered modal advanced"
    );
    viewer.save()?;
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(serde_json::to_value(viewer.session.as_ref().unwrap().save())? == before);
    ensure!(
        serde_json::to_value(
            viewer
                .client
                .as_ref()
                .unwrap()
                .save(viewer.session.as_ref().unwrap())?
        )? == resources
    );
    ensure!(sims_render::software::render(viewer.scene.as_ref().unwrap()) == pixels);
    let image = repo.join("cache/mashuga-fresh-v82/iced-death-modal.png");
    render_headless_element(&root, &image, viewer.view())?;
    ensure!(
        serde_json::to_value(viewer.session.as_ref().unwrap().save())? == before,
        "render delivered input"
    );
    let _ = viewer.update(Message::Dialog(id, 2));
    let _ = viewer.update(Message::Resident(195));
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    ensure!(viewer.session.as_ref().unwrap().simulation.vm.world.globals[&3] == 195);
    for _ in 0..30 {
        let _ = viewer.update(Message::Step);
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    }
    ensure!(viewer.session.as_ref().unwrap().simulation.vm.tick == tick + 30);
    ensure!(
        viewer
            .session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .dialogs
            .modal
            .is_none()
    );
    viewer.save()?;
    let after = serde_json::to_value(viewer.session.as_ref().unwrap().save())?;
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(serde_json::to_value(viewer.session.as_ref().unwrap().save())? == after);
    std::fs::write(
        repo.join("cache/mashuga-fresh-v82/iced-report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source":source,"start_tick":tick,"final_tick":tick+30,"modal":id,
            "modal_restore_equal":true,"client_resources_equal":true,"pixels_equal":true,
        "render_did_not_deliver_input":true,"explicit_response":2,"explicit_resident_selection":195,"continued_restore_equal":true,"image":image,
            "scope":"Current-program fresh diagnostic; explicitly supplied empty nearby-house input and inferred routing; legacy camera profile. No normal-load or native full-application claim.",
        }))?,
    )?;
    println!(
        "Fresh Mashuga Iced modal: current-program save/load, retained portrait, explicit OK and 30 further ticks; {}",
        image.display()
    );
    Ok(())
}

#[test]
#[ignore = "original Mashuga assets, live Iced controls and offscreen GPU rendering"]
fn original_viewer_constructs_saves_restores_and_answers_death_dialog() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let index = Index::scan(&root)?;
    let mut session = portrait_support::capture(&index, &repo)?;
    ensure!(
        matches!(&session.simulation.vm.threads[&439].status,sims_vm::Status::Faulted(f) if f.message=="Dialog explicit icon selection unsupported")
    );
    session.simulation.vm.threads.get_mut(&439).unwrap().status = sims_vm::Status::Ready;
    let client = DialogClient::new(&mut session)?;
    session
        .simulation
        .vm
        .world
        .image_cache
        .entries
        .iter_mut()
        .find(|e| e.guid == 1480716884)
        .unwrap()
        .dirty[1] = true;
    let save = repo.join("cache/dialog-client-iced-save.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData2/Houses/House09.iff",
        "--inferred-routing",
        "--paused",
        "--resident",
        "2",
        "--root",
        root.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(Arc::new(Ready {
        restore_index: None,
        session: Mutex::new(Some(session)),
        client: Mutex::new(Some(client)),
        device: Mutex::new(None),
        icons: Arc::new(dialog_view::Icons::load(&index)?),
    }))?;
    ensure!(viewer.session.as_ref().unwrap().simulation.vm.world.globals[&3] == 195);
    let _ = viewer.update(Message::Step);
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    ensure!(
        viewer.client.as_ref().unwrap().pending(),
        "regeneration must suspend at the UI instruction budget"
    );
    let frozen = sims_render::software::render(viewer.scene.as_ref().unwrap());
    let before = viewer.session.as_ref().unwrap().simulation.clone();
    let _ = viewer.update(Message::Rotate);
    let _ = viewer.update(Message::Queue(268, 0));
    ensure!(
        viewer.session.as_ref().unwrap().simulation == before,
        "game command interrupted synchronous dialog construction"
    );
    viewer.save()?;
    let saved = serde_json::to_value(viewer.session.as_ref().unwrap().save())?;
    let resources = serde_json::to_value(
        viewer
            .client
            .as_ref()
            .unwrap()
            .save(viewer.session.as_ref().unwrap())?,
    )?;
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(serde_json::to_value(viewer.session.as_ref().unwrap().save())? == saved);
    ensure!(
        serde_json::to_value(
            viewer
                .client
                .as_ref()
                .unwrap()
                .save(viewer.session.as_ref().unwrap())?
        )? == resources
    );
    ensure!(
        sims_render::software::render(viewer.scene.as_ref().unwrap()) == frozen,
        "pending save lost its stable lot view"
    );
    for _ in 0..100 {
        let _ = viewer.update(Message::Step);
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
        if !viewer.client.as_ref().unwrap().pending() {
            break;
        }
    }
    ensure!(!viewer.client.as_ref().unwrap().pending());
    let s = viewer.session.as_ref().unwrap();
    ensure!(s.simulation.vm.tick == 215742 && !s.simulation.vm.world.objects.contains_key(&196));
    let id = s
        .simulation
        .vm
        .world
        .dialogs
        .modal
        .context("death dialog not displayed")?;
    ensure!(
        s.simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .any(|r| r.id == id && r.portrait.is_some() && r.body.contains("Frankie"))
    );
    viewer.save()?;
    let modal = serde_json::to_value(viewer.session.as_ref().unwrap().save())?;
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(serde_json::to_value(viewer.session.as_ref().unwrap().save())? == modal);
    let image = repo.join("cache/dialog-client-iced-household.png");
    render_headless_element(&root, &image, viewer.view())?;
    ensure!(
        serde_json::to_value(viewer.session.as_ref().unwrap().save())? == modal,
        "headless rendering delivered input"
    );
    let _ = viewer.update(Message::Dialog(id, 2));
    for _ in 0..30 {
        let _ = viewer.update(Message::Step);
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    }
    ensure!(viewer.session.as_ref().unwrap().simulation.vm.tick == 215772);
    ensure!(
        viewer
            .session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .dialogs
            .records
            .is_empty()
    );
    viewer.save()?;
    let final_state = serde_json::to_value(viewer.session.as_ref().unwrap().save())?;
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(serde_json::to_value(viewer.session.as_ref().unwrap().save())? == final_state);
    println!(
        "Iced death dialog: saved/restored during native construction, at modal prompt and after explicit OK; 30 further ticks; headless frame {}",
        image.display()
    );
    Ok(())
}
