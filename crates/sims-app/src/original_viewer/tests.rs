use super::*;
use clap::Parser;

#[test]
#[ignore = "retained v86 Goth 110000 checkpoint; explicit SIMS_REUSED_RESIDENT_OUTPUT"]
fn reused_resident_id_keeps_live_object_name_and_rejects_stale_controls() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = repo.join("game-data/cabinet-extracted");
    let index = Index::scan(&root)?;
    let checkpoint = repo.join("cache/goth-v86-after-bella-94528/final.json");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&checkpoint)?)?;
    let session = Session::restore(&index, serde_json::from_value(value["session"].clone())?)?;
    let before = serde_json::to_value(session.save())?;
    let object = &session.simulation.vm.world.objects[&18];
    ensure!(
        object
            .definition
            .as_ref()
            .unwrap()
            .source
            .ends_with("::Ash.iff")
    );
    ensure!(live_resident(&session, 18).is_none());
    ensure!(live_resident(&session, 21).unwrap().name == "Mortimer");
    ensure!(live_resident(&session, 16).unwrap().name == "Cassandra");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
    ]);
    let (mut viewer, _) = Viewer::new(args);
    viewer.session = Some(session);
    viewer.resident = 18; // Stale frontend selection only, never VM state.
    viewer.selected = Some(18);
    let label = viewer.label(18);
    ensure!(label != "Bella");
    ensure!(viewer.label(21) == "Mortimer" && viewer.label(16) == "Cassandra");
    viewer.menu();
    ensure!(viewer.choices.is_empty() && viewer.menu_error.is_none());
    for event in [
        Message::Resident(18),
        Message::Queue(18, 0),
        Message::Cancel(1),
    ] {
        let _ = viewer.update(event);
        ensure!(viewer.error.is_none());
        ensure!(serde_json::to_value(viewer.session.as_ref().unwrap().save())? == before);
    }
    let path = std::env::var("SIMS_REUSED_RESIDENT_OUTPUT")?;
    ensure!(
        Path::new(&path).starts_with("cache")
            && Path::new(&path)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
    );
    let file = std::fs::File::create_new(repo.join(path))?;
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "checkpoint":checkpoint,"tick":viewer.session.as_ref().unwrap().simulation.vm.tick,
            "reused_object_id":18,"current_label":label,"historical_resident":"Bella",
            "live_resident_names_preserved":true,"stale_frontend_commands_ignored":3,
            "complete_session_unchanged":true,
            "scope":"Actual original-house aftermath; frontend identity and event filtering only. No VM state, scheduler, resource or save-schema changes."
        }),
    )?;
    Ok(())
}

fn trace_startup_presentation(viewer: &Viewer, stage: &str) {
    if let Some(session) = viewer.session.as_ref() {
        let vm = &session.simulation.vm;
        eprintln!(
            "{}",
            serde_json::json!({
                "stage":stage,"tick":vm.tick,"playing":viewer.playing,
                "globals": ([11,17,19].map(|index|(index,vm.world.globals.get(&index)))),
                "modal":vm.world.dialogs.modal.and_then(|id|vm.world.dialogs.records.iter().find(|record|record.id==id)),
                "clock":session.host.presentation.clock,
                "clock_pending":session.clock_pending(),
                "children_pending":viewer.presentation_children_pending(),
                "client_pending":viewer.client.as_ref().is_some_and(|client|client.pending()),
                "publication_pending":session.publication_pending(),
                "status":viewer.status,"error":viewer.error,
            })
        );
    }
}

fn drain_presentation(viewer: &mut Viewer) -> Result<()> {
    for _ in 0..10000 {
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
        if !viewer.presentation_pending() {
            return Ok(());
        }
        let _ = viewer.update(Message::Frame(Instant::now()));
    }
    trace_startup_presentation(viewer, "presentation_drain_exhausted");
    anyhow::bail!("Iced presentation did not finish")
}

#[test]
#[ignore = "original three-house installations and actual native loading"]
fn fresh_households_prepare_thumbnail_catalog_without_gameplay_mutation() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../game-data/cabinet-extracted")
        .canonicalize()?;
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let args = Args::parse_from([
            "sims-room",
            "--original",
            source,
            "--inferred-routing",
            "--paused",
            "--root",
            root.to_str().unwrap(),
        ]);
        let ready = Arc::new(Ready::load(&args)?);
        let (mut viewer, _) = Viewer::new(args);
        viewer.install(ready)?;
        ensure!(
            viewer.thumbnails.is_some(),
            "{source}: startup catalog absent"
        );
        let before = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
        viewer.thumbnails = None;
        viewer.prepare_thumbnail_catalog()?;
        ensure!(viewer.thumbnails.is_some());
        viewer.prepare_thumbnail_catalog()?;
        ensure!(
            before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?,
            "{source}: thumbnail catalog preparation changed gameplay state"
        );
    }
    Ok(())
}

#[test]
#[ignore = "original three-house fresh startup, actual Iced controls and offscreen frames"]
fn default_original_startup_completes_native_loading_for_three_homes() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_DEFAULT_LOADING_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace default loading evidence")?;
    let mut reports = vec![];
    for (name, source, floor, resident) in [
        ("newbie", "UserData/Houses/House07.iff", "1", "1"),
        ("goth", "UserData/Houses/House05.iff", "2", "1"),
        ("mashuga", "UserData2/Houses/House09.iff", "1", "2"),
    ] {
        let dir = output.join(name);
        std::fs::create_dir(&dir)?;
        let save = dir.join("session.json");
        let args = Args::parse_from([
            "sims-room",
            "--original",
            source,
            "--inferred-routing",
            "--paused",
            "--root",
            root.to_str().unwrap(),
            "--save",
            save.to_str().unwrap(),
            "--floor",
            floor,
            "--resident",
            resident,
        ]);
        let ready = Arc::new(Ready::load(&args)?);
        let start = ready
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .tick;
        let (mut viewer, _) = Viewer::new(args);
        viewer.install(ready)?;
        drain_presentation(&mut viewer)?;
        let s = viewer.session.as_ref().unwrap();
        ensure!(s.options.native_postload && s.options.native_load_completion);
        ensure!(
            !s.postload_pending()
                && !s.load_completion_pending()
                && !s.host.presentation.awaiting_house_open
        );
        ensure!(s.simulation.vm.tick == start, "startup consumed lot ticks");
        ensure!(s.simulation.vm.world.globals.get(&19) == Some(&0));
        ensure!(
            s.host
                .presentation
                .control_panel
                .as_ref()
                .is_some_and(|p| p.value() == 1)
        );
        ensure!(viewer.device.as_ref().unwrap().panel_state() == Some(1));
        let buttons = viewer
            .client
            .as_ref()
            .unwrap()
            .buttons()
            .context("fresh resident buttons absent")?;
        ensure!(buttons.initialized && buttons.activated.is_none());
        ensure!(
            buttons.items.len() == s.house.household.as_ref().unwrap().residents.len()
                && buttons.items.iter().all(|b| b.image.is_some())
        );
        let needs = viewer
            .client
            .as_ref()
            .unwrap()
            .live_panel()
            .context("fresh Needs widget absent")?;
        let displayed = needs.person().context("fresh displayed person absent")?;
        ensure!(needs.selected() && displayed.object == viewer.resident);
        let motives = s.simulation.vm.world.objects[&displayed.object]
            .motives
            .unwrap();
        for (i, (_, slot)) in sims_render::dialog::client::live_panel::MOTIVES
            .iter()
            .enumerate()
        {
            ensure!(displayed.values[i] == motives[*slot]);
        }
        ensure!(
            s.host
                .presentation
                .interface_update
                .as_ref()
                .is_some_and(|u| !u.pending())
        );
        ensure!(
            s.simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?
                == Some(viewer.resident)
        );
        ensure!(viewer.floor == floor.parse::<i32>()?);
        let initial = serde_json::to_vec(&s.save())?;
        render_headless_element(&root, &dir.join("opened.png"), viewer.view())?;
        ensure!(initial == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
        let mut modal = false;
        for _ in 0..64 {
            let _ = viewer.update(Message::Step);
            trace_startup_presentation(&viewer, "fresh_start_after_step");
            drain_presentation(&mut viewer)?;
            ensure!(viewer.error.is_none(), "{name}: {:?}", viewer.error);
            if viewer
                .session
                .as_ref()
                .unwrap()
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .is_some()
            {
                modal = true;
                break;
            }
        }
        let s = viewer.session.as_ref().unwrap();
        let tick = s.simulation.vm.tick;
        if name == "newbie" {
            ensure!(modal && tick == start + 1);
            let dialogs = &s.simulation.vm.world.dialogs;
            ensure!(
                dialogs
                    .records
                    .iter()
                    .any(|r| Some(r.id) == dialogs.modal && r.bitmap_icon == Some(5018))
            );
        } else {
            ensure!(tick == start + 64, "{name} did not run 64 ordinary ticks");
        }
        viewer.save()?;
        let before = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
        let pixels = sims_render::software::render(viewer.scene.as_ref().unwrap());
        render_headless_element(&root, &dir.join("played.png"), viewer.view())?;
        ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
        viewer.load(&save)?;
        viewer.refresh()?;
        ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
        ensure!(pixels == sims_render::software::render(viewer.scene.as_ref().unwrap()));
        let report = serde_json::json!({"house":name,"source":source,"start_tick":start,"tick":tick,
            "modal":modal,"floor":viewer.floor,"selected":viewer.resident,
            "native_postload_and_completion":true,"full_session_and_pixels_restore":true,
            "ordered_live_mode_publication":true,"script_mode":0,
            "control_panel_bound":true,"panel_state":1,
            "resident_buttons_bound":true,"resident_images":viewer.client.as_ref().unwrap().buttons(),
            "headless_render_preserves_session":true});
        std::fs::write(dir.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
        reports.push(report);
        eprintln!(
            "default Iced {name}: startup complete, +{} ticks, restored state/pixels match",
            tick - start
        );
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "houses":reports,"scope":"Default fresh startup without --load; original files/native postload/completion, explicit replacement listener and rendering profile. No dialog answers, coordinate overrides or original suspended-stack claim."
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private original houses; record bounded fresh Live-startup play frontiers without dialog or coordinate overrides"]
fn fresh_live_households_record_original_play_frontiers() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_FRESH_FRONTIER_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace fresh play evidence")?;
    let mut reports = vec![];
    for (name, source) in [
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ] {
        let dir = output.join(name);
        std::fs::create_dir(&dir)?;
        let save = dir.join("session.json");
        let args = Args::parse_from([
            "sims-room",
            "--original",
            source,
            "--inferred-routing",
            "--paused",
            "--root",
            root.to_str().unwrap(),
            "--save",
            save.to_str().unwrap(),
        ]);
        let ready = Arc::new(Ready::load(&args)?);
        let (mut viewer, _) = Viewer::new(args);
        viewer.install(ready)?;
        drain_presentation(&mut viewer)?;
        let start = viewer.session.as_ref().unwrap().simulation.vm.tick;
        let mut error = None;
        let mut modal = false;
        let mut ticks = 0;
        for _ in 0..200000 {
            let result = (|| -> Result<()> {
                if viewer.presentation_pending() {
                    viewer.step()?;
                } else if viewer.poll_application_ui()? {
                    if viewer.step()? {
                        ticks += 1;
                    }
                }
                Ok(())
            })();
            if let Err(e) = result {
                error = Some(format!("{e:#}"));
                break;
            }
            if viewer
                .session
                .as_ref()
                .unwrap()
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .is_some()
            {
                modal = true;
                break;
            }
            if ticks >= 10000 {
                break;
            }
        }
        ensure!(
            ticks > 64,
            "{name} could not exceed startup prefix: {error:?}"
        );
        if let Some(e) = &error {
            viewer.fail(e.clone());
        }
        // Keep raw simulation evidence even if a partially constructed frame
        // needs more work before the renderer can display this boundary.
        std::fs::write(
            dir.join("raw-session.json"),
            serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?,
        )?;
        viewer.save()?;
        let before = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
        let pixels =
            sims_render::software::render(viewer.scene.as_ref().context("frontier scene absent")?);
        render_headless_element(&root, &dir.join("frontier.png"), viewer.view())?;
        ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
        viewer.load(&save)?;
        viewer.refresh()?;
        ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
        ensure!(pixels == sims_render::software::render(viewer.scene.as_ref().unwrap()));
        let advanced = viewer.session.as_ref().unwrap().simulation.vm.tick - start;
        let stop = if error.is_some() {
            "unsupported_service"
        } else if modal {
            "unanswered_modal"
        } else if ticks >= 10000 {
            "tick_budget"
        } else {
            "slice_budget"
        };
        let report = serde_json::json!({"house":name,"source":source,"start_tick":start,
            "tick":viewer.session.as_ref().unwrap().simulation.vm.tick,"ticks":advanced,
            "completed_tick_calls":ticks,"stop":stop,"error":error,"modal":modal,
            "budget_ticks":10000,"full_session_and_pixels_restore":true,"render_preserves_session":true,
            "scope":"Fresh native postload/completion and Live/panel binding; one original UI poll before each ordinary tick, real synchronous frontend services, inferred rectangle routing. No empty nearby-coordinate inputs, automatic dialog answers or fabricated service replies. This records the reached frontier, not multi-day completion."});
        std::fs::write(dir.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
        eprintln!("fresh {name}: +{advanced} ({ticks} completed ticks), {stop}, error={error:?}");
        reports.push(report);
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"houses":reports}))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "fresh original Newbie device checkpoint; actual Iced tutorial controls and headless frames"]
fn fresh_original_tutorial_camera_lessons_use_iced_controls() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let source = repo.join(
        std::env::var("SIMS_FRESH_ICED_SOURCE")
            .unwrap_or_else(|_| "cache/newbie-actual-device-prefix/welcome.json".into()),
    );
    let output = repo.join(std::env::var("SIMS_FRESH_ICED_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace tutorial control evidence")?;
    let save = output.join("session.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
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
    let start = viewer.session.as_ref().unwrap().simulation.vm.tick;
    let script_mode = viewer.session.as_ref().unwrap().simulation.vm.world.globals[&19];
    let owner = viewer
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .ui_owner
        .owner
        .unwrap();
    let before = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("welcome.png"), viewer.view())?;
    ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    let dialogs = &viewer.session.as_ref().unwrap().simulation.vm.world.dialogs;
    let welcome = dialogs
        .records
        .iter()
        .find(|r| Some(r.id) == dialogs.modal)
        .unwrap();
    ensure!(welcome.bitmap_icon == Some(5018) && welcome.buttons.len() == 1);
    let _ = viewer.update(Message::Dialog(welcome.id, 2));
    let mut actions = vec![];
    let mut restored = false;
    let mut scrolls = std::collections::BTreeSet::new();
    let mut reached = false;
    for _ in 0..1000 {
        drain_presentation(&mut viewer)?;
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
        let s = viewer.session.as_ref().unwrap();
        let prompt = s
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .rev()
            .find(|r| r.origin.bhav == 4116)
            .map(|r| r.origin.instruction);
        if prompt == Some(11) {
            reached = true;
            break;
        }
        let waiting = s.simulation.vm.world.objects[&owner].attributes[2];
        let zoom = viewer.device.as_ref().unwrap().controls().zoom;
        let target = match prompt {
            Some(1) => Some(2),
            Some(4) => Some(3),
            _ => None,
        };
        if let Some(target) = target.filter(|n| *n != zoom) {
            if viewer.tutorial_highlight == Some(2013) {
                actions.push(serde_json::json!({"prompt":prompt,"kind":"zoom","before":zoom,"target":target}));
                let _ = viewer.update(if zoom < target {
                    Message::ZoomIn
                } else {
                    Message::ZoomOut
                });
                if !restored {
                    ensure!(viewer.session.as_ref().unwrap().camera_command_pending());
                    viewer.save()?;
                    let bytes = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
                    let pixels =
                        sims_render::software::render(viewer.device.as_ref().unwrap().scene());
                    viewer.load(&save)?;
                    viewer.refresh()?;
                    ensure!(bytes == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
                    ensure!(
                        pixels
                            == sims_render::software::render(
                                viewer.device.as_ref().unwrap().scene()
                            )
                    );
                    restored = true;
                }
                drain_presentation(&mut viewer)?;
            }
        }
        if let Some(instruction @ (9 | 10)) = prompt {
            if waiting == 7 && scrolls.insert(instruction) {
                ensure!(viewer.poll_application_ui()?);
                let before = viewer
                    .session
                    .as_ref()
                    .unwrap()
                    .retained_camera_input()?
                    .center;
                if instruction == 9 {
                    let now = Instant::now();
                    viewer.last = Some(now);
                    let _ = viewer.update(Message::Pan([1, 0]));
                    let _ =
                        viewer.update(Message::Frame(now + std::time::Duration::from_millis(20)));
                    let _ = viewer.update(Message::Pan([0, 0]));
                } else {
                    let _ = viewer.update(Message::Scroll([-32, 0]));
                }
                drain_presentation(&mut viewer)?;
                ensure!(viewer.poll_application_ui()?);
                let after = viewer
                    .session
                    .as_ref()
                    .unwrap()
                    .retained_camera_input()?
                    .center;
                ensure!(before != after, "actual camera did not scroll");
                actions.push(serde_json::json!({"prompt":instruction,"kind":"scroll","before":before,"after":after}));
            }
        }
        let _ = viewer.update(Message::Step);
    }
    ensure!(
        reached && restored && scrolls.len() == 2,
        "fresh tutorial navigation incomplete: {actions:?}; {:?}",
        viewer.error
    );
    viewer.save()?;
    let original = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("rotation-lesson.png"), viewer.view())?;
    ensure!(original == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(original == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "start_tick":start,"tick":viewer.session.as_ref().unwrap().simulation.vm.tick,"actions":actions,
            "source_checkpoint":source,"script_mode":script_mode,
            "restored_camera_command_and_pixels":restored,"final_full_session_restore":true,
            "render_preserves_session":true,"reached_original_rotation_lesson":true,
            "scope":"Fresh original Newbie welcome; one explicit welcome OK, ordinary Iced zoom, edge-pan and keyboard-scroll messages. No tutorial attribute edits or synthetic wait events. Headless frames only."
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "fresh original tutorial checkpoint; Iced rotation, original wait callback and headless frame"]
fn original_tutorial_rotation_preserves_unbound_live_mode_request() -> Result<()> {
    tutorial_rotation_boundary(
        "cache/newbie-fresh-iced-controls/session.json",
        3,
        Some((2030, 4104, 3)),
    )
}

#[test]
#[ignore = "new Live startup Newbie camera checkpoint; original tool-state observation"]
fn live_tutorial_rotation_preserves_original_tool_wait() -> Result<()> {
    tutorial_rotation_boundary("cache/newbie-live-iced-controls/session.json", 0, None)
}

#[test]
#[ignore = "fresh original panel-bound Newbie checkpoint; real Live button, pending setter restore and headless frames"]
fn original_live_button_expands_panel_and_completes_navigation() -> Result<()> {
    use sims_runtime::original_session::panel::Progress as PanelProgress;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let source = repo.join("cache/newbie-panel-iced-controls/session.json");
    let output = repo.join(std::env::var("SIMS_PANEL_ICED_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace panel evidence")?;
    let save = output.join("session.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
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
    let owner = viewer
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .ui_owner
        .owner
        .unwrap();
    let mut rotation_wait = false;
    for _ in 0..100 {
        drain_presentation(&mut viewer)?;
        if viewer.session.as_ref().unwrap().simulation.vm.world.objects[&owner].attributes[2] == 6 {
            rotation_wait = true;
            break;
        }
        let _ = viewer.update(Message::Step);
    }
    ensure!(rotation_wait);
    ensure!(viewer.poll_application_ui()?);
    let _ = viewer.update(Message::Rotate);
    drain_presentation(&mut viewer)?;
    ensure!(viewer.poll_application_ui()?);
    let mut ready = false;
    for _ in 0..500 {
        let _ = viewer.update(Message::Step);
        drain_presentation(&mut viewer)?;
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
        let s = viewer.session.as_ref().unwrap();
        if viewer.tutorial_highlight == Some(2010)
            && s.simulation
                .vm
                .world
                .dialogs
                .records
                .iter()
                .any(|r| r.origin.bhav == 4116 && r.origin.instruction == 20)
        {
            ready = true;
            break;
        }
    }
    ensure!(ready, "original expand-panel lesson not reached");
    ensure!(viewer.device.as_ref().unwrap().panel_state() == Some(1));
    let before = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("collapsed-lesson.png"), viewer.view())?;
    ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    let simulation = viewer.session.as_ref().unwrap().save().simulation;
    let click_tick = viewer.session.as_ref().unwrap().simulation.vm.tick;
    let _ = viewer.update(Message::Live);
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    ensure!(viewer.session.as_ref().unwrap().control_panel_pending());
    // First service is the disabled-audio whoosh; no state publication yet.
    ensure!(
        !viewer
            .device
            .as_mut()
            .unwrap()
            .advance_panel(viewer.session.as_mut().unwrap())?
    );
    let s = viewer.session.as_mut().unwrap();
    ensure!(s.host.presentation.control_panel.as_ref().unwrap().value() == 1);
    ensure!(s.advance_panel_update()? == PanelProgress::Publish(0));
    ensure!(s.host.presentation.control_panel.as_ref().unwrap().value() == 0);
    ensure!(viewer.device.as_ref().unwrap().panel_state() == Some(1));
    ensure!(s.save().simulation == simulation);
    // Field changed, actual widget not yet published: this exact split saves.
    viewer.save()?;
    std::fs::copy(&save, output.join("pending-publication.json"))?;
    let pending = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    let pending_device = viewer
        .device
        .as_ref()
        .unwrap()
        .save(viewer.session.as_ref().unwrap())?;
    let pixels = sims_render::software::render(viewer.device.as_ref().unwrap().scene());
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(pending == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    ensure!(
        pending_device
            == viewer
                .device
                .as_ref()
                .unwrap()
                .save(viewer.session.as_ref().unwrap())?
    );
    ensure!(pixels == sims_render::software::render(viewer.device.as_ref().unwrap().scene()));
    ensure!(viewer.device.as_ref().unwrap().panel_state() == Some(1));
    drain_presentation(&mut viewer)?;
    ensure!(viewer.device.as_ref().unwrap().panel_state() == Some(0));
    ensure!(viewer.session.as_ref().unwrap().save().simulation == simulation);
    let before = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("expanded-lesson.png"), viewer.view())?;
    ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    let mut completed = false;
    for _ in 0..500 {
        let _ = viewer.update(Message::Step);
        drain_presentation(&mut viewer)?;
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
        let dialogs = &viewer.session.as_ref().unwrap().simulation.vm.world.dialogs;
        if dialogs.records.iter().any(|r| {
            Some(r.id) == dialogs.modal && r.origin.bhav == 4097 && r.origin.instruction == 3
        }) {
            completed = true;
            break;
        }
    }
    ensure!(
        completed,
        "navigation did not return to the original General tutorial"
    );
    viewer.save()?;
    let before = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    let pixels = sims_render::software::render(viewer.scene.as_ref().unwrap());
    render_headless_element(
        &root,
        &output.join("navigation-complete.png"),
        viewer.view(),
    )?;
    ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    ensure!(pixels == sims_render::software::render(viewer.scene.as_ref().unwrap()));
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "click_tick":click_tick,"tick":viewer.session.as_ref().unwrap().simulation.vm.tick,
            "original_navigation_returned":true,"next_modal_bhav":4097,"next_modal_instruction":3,
            "unpublished_widget_and_native_field_restore":true,"panel_toggle_preserves_entire_simulation":true,
            "final_session_and_pixels_restore":true,"headless_render_preserves_session":true,
            "scope":"Real rotation and Live-button input from fresh panel-bound original Newbie. Original UI wait observes actual retained CPState. No scripted attribute edits or synthetic got-wait events; next modal remains unanswered."
        }))?,
    )?;
    Ok(())
}

fn tutorial_rotation_boundary(
    source: &str,
    script_mode: i16,
    control_request: Option<(i16, u16, usize)>,
) -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let source = repo.join(source);
    let output = repo.join(std::env::var("SIMS_TUTORIAL_ROTATION_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace tutorial rotation evidence")?;
    let save = output.join("session.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
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
    ensure!(viewer.session.as_ref().unwrap().simulation.vm.world.globals[&19] == script_mode);
    let start = viewer.session.as_ref().unwrap().simulation.vm.tick;
    let owner = viewer
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .ui_owner
        .owner
        .unwrap();
    let mut waiting = false;
    for _ in 0..100 {
        drain_presentation(&mut viewer)?;
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
        if viewer.session.as_ref().unwrap().simulation.vm.world.objects[&owner].attributes[2] == 6 {
            waiting = true;
            break;
        }
        let _ = viewer.update(Message::Step);
    }
    ensure!(waiting, "original rotation wait not reached");
    ensure!(viewer.poll_application_ui()?);
    let tick = viewer.session.as_ref().unwrap().simulation.vm.tick;
    let before = viewer
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .camera_rotation;
    let _ = viewer.update(Message::Rotate);
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    ensure!(viewer.session.as_ref().unwrap().camera_command_pending());
    viewer.save()?;
    let bytes = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    let pixels = sims_render::software::render(viewer.device.as_ref().unwrap().scene());
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(bytes == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    ensure!(pixels == sims_render::software::render(viewer.device.as_ref().unwrap().scene()));
    drain_presentation(&mut viewer)?;
    let s = viewer.session.as_ref().unwrap();
    let after = s.simulation.vm.world.camera_rotation;
    ensure!(after != before && s.simulation.vm.tick == tick);
    ensure!(viewer.poll_application_ui()?);
    let expected = match control_request {
        Some((control, _, _)) => {
            format!("original UI control {control} is not bound in this viewer")
        }
        None => "original UI wait requires its widget owner: PanelExpanded".into(),
    };
    let mut reached = false;
    for _ in 0..500 {
        let _ = viewer.update(Message::Step);
        if viewer.error.is_none() {
            // A synchronous presentation slice can reach this unsupported
            // request after the initial Step. Preserve it without replying.
            let result = drain_presentation(&mut viewer);
            if viewer.error.is_none() {
                result?;
            }
        }
        if let Some(error) = &viewer.error {
            ensure!(*error == expected, "unexpected tutorial boundary: {error}");
            reached = true;
            break;
        }
    }
    ensure!(reached, "original tutorial boundary not reached");
    let s = viewer.session.as_mut().unwrap();
    if let Some((control, bhav, instruction)) = control_request {
        let thread = s
            .simulation
            .vm
            .threads
            .values()
            .find(|t| {
                matches!(&t.status, sims_vm::Status::AwaitingUiEffect(c)
            if c.request().value == control && c.request().enabled)
            })
            .context("unacknowledged Live Mode highlight absent")?;
        ensure!(
            thread
                .stack
                .last()
                .is_some_and(|f| f.location.bhav == bhav && f.location.instruction == instruction)
        );
    } else {
        ensure!(s.ui_wait_pending());
        let before = serde_json::to_vec(&s.save())?;
        ensure!(matches!(
            s.advance_ui_wait(1)?,
            sims_runtime::original_session::ui_wait::Progress::Observe(
                sims_vm::ui_owner::wait::Condition::PanelExpanded
            )
        ));
        ensure!(before == serde_json::to_vec(&s.save())?);
    }
    viewer.save()?;
    let bytes = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    let pixels = sims_render::software::render(viewer.scene.as_ref().unwrap());
    let frame = if control_request.is_some() {
        "unbound-live-mode.png"
    } else {
        "needs-tool-owner.png"
    };
    render_headless_element(&root, &output.join(frame), viewer.view())?;
    ensure!(bytes == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(bytes == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    ensure!(pixels == sims_render::software::render(viewer.scene.as_ref().unwrap()));
    ensure!(viewer.step().unwrap_err().to_string() == expected);
    ensure!(bytes == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "start_tick":start,"tick":viewer.session.as_ref().unwrap().simulation.vm.tick,
            "source_checkpoint":source,"script_mode":script_mode,
            "rotation_before":before,"rotation_after":after,"unbound_control":control_request.map(|r| r.0),
            "pending_bhav":control_request.map(|r| r.1),"pending_instruction":control_request.map(|r| r.2),
            "pending_observation":if control_request.is_none() {Some("ToolIdle")} else {None},"error":expected,
            "mid_rotation_session_and_pixels_restore":true,"final_full_session_restore":true,
            "render_preserves_session":true,"restored_request_repeats_without_state_change":true,
            "scope":"One actual Iced Rotate command after the original rotation watch is active. The original resource's next unbound control or observation remains unacknowledged. No script/global edits, synthetic wait events or dialog answers. Mode switching and control-panel expansion remain unsupported."
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private tutorial checkpoint; real Iced scrolling messages and callback save/resume"]
fn original_viewer_scroll_wait_callbacks_restore_and_reach_rotation_lesson() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let source = repo.join("cache/native-camera-device-iced-save.json");
    let save = repo.join("cache/native-ui-wait-iced-save.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
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
    let (mut viewer, _) = Viewer::new(args.clone());
    viewer.install(ready)?;
    let owner = viewer
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .ui_owner
        .owner
        .unwrap();
    for _ in 0..100 {
        if viewer.session.as_ref().unwrap().simulation.vm.world.objects[&owner].attributes[2] == 7 {
            break;
        }
        let _ = viewer.update(Message::Step);
        drain_presentation(&mut viewer)?;
    }
    let s = viewer.session.as_mut().unwrap();
    ensure!(s.simulation.vm.world.objects[&owner].attributes[2] == 7);
    s.begin_ui_wait(None)?;
    for _ in 0..100 {
        s.advance_ui_wait(1)?;
        if s.simulation.ui_wait_callback.is_some() {
            break;
        }
    }
    ensure!(s.simulation.ui_wait_callback.is_some());
    viewer.save()?;
    let mut args2 = args;
    args2.load = Some(save.clone());
    let ready2 = Arc::new(Ready::load(&args2)?);
    let (mut resumed, _) = Viewer::new(args2);
    resumed.install(ready2)?;
    drain_presentation(&mut viewer)?;
    drain_presentation(&mut resumed)?;
    let now = Instant::now();
    let clock = |v: &Viewer| {
        let vm = &v.session.as_ref().unwrap().simulation.vm;
        (vm.tick, vm.rng_state, vm.lot_rng_state, vm.crt_rng_state)
    };
    let before = clock(&viewer);
    let origin = viewer
        .session
        .as_ref()
        .unwrap()
        .retained_camera_input()?
        .center;
    for v in [&mut viewer, &mut resumed] {
        // Same message sent by the viewport edge; no original script fields are patched.
        v.last = Some(now);
        let _ = v.update(Message::Pan([1, 0]));
        let _ = v.update(Message::Frame(now + std::time::Duration::from_millis(20)));
        let _ = v.update(Message::Pan([0, 0]));
        drain_presentation(v)?;
        ensure!(v.error.is_none(), "{:?}", v.error);
        ensure!(v.session.as_ref().unwrap().simulation.vm.world.objects[&owner].attributes[2] == 0);
        ensure!(
            clock(v) == before,
            "scroll and callback advanced lot time/RNG"
        );
        ensure!(
            v.session.as_ref().unwrap().retained_camera_input()?.center != origin,
            "edge scroll did not move camera"
        );
    }
    let mut keyboard = false;
    let mut rotation = false;
    for _ in 0..500 {
        let records = &viewer
            .session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .dialogs
            .records;
        if records
            .iter()
            .any(|r| r.origin.bhav == 4116 && r.origin.instruction == 11)
        {
            rotation = true;
            break;
        }
        let keyboard_prompt = records
            .iter()
            .any(|r| r.origin.bhav == 4116 && r.origin.instruction == 10);
        if keyboard_prompt
            && !keyboard
            && viewer.session.as_ref().unwrap().simulation.vm.world.objects[&owner].attributes[2]
                == 7
        {
            // Capture the native baseline before an actual arrow-key message.
            for v in [&mut viewer, &mut resumed] {
                ensure!(v.poll_application_ui()?);
                let _ = v.update(Message::Scroll([-32, 0]));
                drain_presentation(v)?;
            }
            keyboard = true;
        }
        let _ = viewer.update(Message::Step);
        let _ = resumed.update(Message::Step);
        drain_presentation(&mut viewer)?;
        drain_presentation(&mut resumed)?;
    }
    ensure!(
        keyboard && rotation,
        "original scroll lessons did not finish"
    );
    ensure!(
        serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?
            == serde_json::to_vec(&resumed.session.as_ref().unwrap().save())?,
        "Iced callback resume changed session"
    );
    ensure!(
        sims_render::software::render(viewer.scene.as_ref().unwrap())
            == sims_render::software::render(resumed.scene.as_ref().unwrap())
    );
    viewer.save()?;
    println!(
        "Iced: edge and arrow scrolling reach original rotation lesson; query callback restore, Session and pixels equal"
    );
    Ok(())
}

#[test]
#[ignore = "requires original household assets; actual Iced camera messages and saved surfaces"]
fn original_viewer_native_camera_messages_restore_mid_command() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let save = repo.join("cache/native-camera-iced-controls-save.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut viewer, _) = Viewer::new(args.clone());
    viewer.install(ready)?;
    let clock = |v: &Viewer| {
        let vm = &v.session.as_ref().unwrap().simulation.vm;
        (vm.tick, vm.rng_state, vm.lot_rng_state, vm.crt_rng_state)
    };
    let before = clock(&viewer);
    let _ = viewer.update(Message::Rotate);
    ensure!(viewer.presentation_pending());
    // Save inside a person refresh, not just before the command starts.
    let mut captured = false;
    for _ in 0..30000 {
        let s = viewer.session.as_mut().unwrap();
        if s.host
            .presentation
            .camera_command
            .as_ref()
            .and_then(|c| c.pending_person())
            .is_some()
        {
            captured = true;
            break;
        }
        ensure!(
            !viewer.device.as_mut().unwrap().advance(s, 1)?,
            "rotation finished before its person refresh"
        );
    }
    ensure!(captured, "no suspended person refresh");
    viewer.save()?;
    let frozen = sims_render::software::render(viewer.scene.as_ref().unwrap());
    let mut args2 = args;
    args2.load = Some(save.clone());
    let ready2 = Arc::new(Ready::load(&args2)?);
    let (mut resumed, _) = Viewer::new(args2);
    resumed.install(ready2)?;
    ensure!(frozen == sims_render::software::render(resumed.scene.as_ref().unwrap()));
    ensure!(
        serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?
            == serde_json::to_vec(&resumed.session.as_ref().unwrap().save())?
    );
    drain_presentation(&mut viewer)?;
    drain_presentation(&mut resumed)?;
    for message in [
        Message::Zoom(1.0),
        Message::Floor(2),
        Message::Zoom(0.25),
        Message::RotateBack,
        Message::Floor(1),
        Message::ZoomIn,
        Message::ZoomOut,
    ] {
        let _ = viewer.update(message.clone());
        let _ = resumed.update(message);
        drain_presentation(&mut viewer)?;
        drain_presentation(&mut resumed)?;
    }
    ensure!(
        clock(&viewer) == before && clock(&resumed) == before,
        "camera controls consumed simulation time or RNG"
    );
    ensure!(
        serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?
            == serde_json::to_vec(&resumed.session.as_ref().unwrap().save())?
    );
    ensure!(
        viewer
            .device
            .as_ref()
            .unwrap()
            .save(viewer.session.as_ref().unwrap())?
            == resumed
                .device
                .as_ref()
                .unwrap()
                .save(resumed.session.as_ref().unwrap())?
    );
    ensure!(
        sims_render::software::render(viewer.scene.as_ref().unwrap())
            == sims_render::software::render(resumed.scene.as_ref().unwrap())
    );
    viewer.save()?;
    println!(
        "Iced: eight camera commands, mid-person-refresh restore, equal Session/device/pixels; no tick/RNG consumed"
    );
    Ok(())
}

#[test]
#[ignore = "requires original household assets; headless controls and saved pixels"]
fn original_viewer_controls_queue_cancel_and_restore_the_same_scene() -> Result<()> {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
    let save = std::env::temp_dir().join(format!("sims-original-ui-{}.json", std::process::id()));
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
    let ready = Arc::new(Ready::load(&args)?);
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(ready)?;
    let resident = viewer.resident;
    let _ = viewer.update(Message::Resident(resident));
    drain_presentation(&mut viewer)?;
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    let before = viewer.session.as_ref().unwrap().simulation.clone();
    let targets: Vec<_> = before
        .vm
        .world
        .objects
        .iter()
        .filter_map(|(&id, o)| {
            o.definition
                .as_ref()
                .filter(|d| d.source.to_ascii_lowercase().ends_with("diningchairs2.iff"))
                .map(|_| id)
        })
        .collect();
    let target = *targets.first().context("original chair absent")?;
    let _ = viewer.update(Message::Select(Some(target)));
    super::object_tests::menu(&mut viewer)?;
    let session = viewer.session.as_ref().unwrap();
    let input = session.host.presentation.object_input.as_ref().unwrap();
    let index = input
        .actions()
        .iter()
        .position(|a| {
            session
                .simulation
                .vm
                .action_menu(&session.program, a.target)
                .unwrap()
                .iter()
                .any(|e| e.id == a.menu_id && e.label == "Sit")
        })
        .context("no available original chair interaction")?;
    ensure!(
        session.simulation.vm.tick == before.vm.tick,
        "menu advanced household clock"
    );
    ensure!(input.actions()[index].priority == 50);
    let _ = viewer.update(Message::ObjectChoice(index));
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    let id = viewer
        .session
        .as_ref()
        .unwrap()
        .simulation
        .pending_interactions(resident)
        .unwrap()
        .back()
        .context("queued action absent")?
        .id;
    let _ = viewer.update(Message::Cancel(id.into()));
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    ensure!(
        !viewer
            .session
            .as_ref()
            .unwrap()
            .simulation
            .pending_interactions(resident)
            .unwrap()
            .iter()
            .any(|a| a.id == id)
    );
    for _ in 0..30 {
        viewer.step()?;
    }
    viewer.refresh()?;
    let saved = viewer.session.as_ref().unwrap().save();
    let frame = sims_render::software::render(viewer.scene.as_ref().unwrap());
    viewer.save()?;
    for _ in 0..5 {
        viewer.step()?;
    }
    viewer.load(&save)?;
    viewer.refresh()?;
    let restored = viewer.session.as_ref().unwrap();
    ensure!(
        restored.simulation == saved.simulation.state && restored.host.snapshot() == saved.assets,
        "UI load changed session"
    );
    ensure!(
        frame == sims_render::software::render(viewer.scene.as_ref().unwrap()),
        "UI load changed pixels"
    );
    let before = restored.simulation.vm.tick;
    let _ = viewer.update(Message::Floor(2));
    drain_presentation(&mut viewer)?;
    ensure!(
        viewer.session.as_ref().unwrap().simulation.vm.tick == before,
        "floor view advanced simulation time"
    );
    let _ = viewer.update(Message::Floor(1));
    drain_presentation(&mut viewer)?;
    let _ = viewer.update(Message::Select(Some(target)));
    super::object_tests::menu(&mut viewer)?;
    let _ = viewer.update(Message::ObjectChoice(index));
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    let action = viewer
        .session
        .as_ref()
        .unwrap()
        .simulation
        .pending_interactions(resident)
        .unwrap()
        .back()
        .unwrap()
        .id;
    let mut dispatched = false;
    let mut seated = false;
    let mut saved_action = false;
    for _ in 0..1200 {
        viewer.step()?;
        let s = &viewer.session.as_ref().unwrap().simulation;
        let a = &s.vm.world.actions.as_ref().unwrap().actors[&resident];
        dispatched |= a.active.as_ref().is_some_and(|a| a.id == action);
        if dispatched
            && s.vm.world.objects[&resident]
                .container
                .is_some_and(|(id, _)| id == target)
        {
            seated = true;
            break;
        }
        if dispatched && !saved_action {
            viewer.refresh()?;
            let before = viewer.session.as_ref().unwrap().save();
            let pixels = sims_render::software::render(viewer.scene.as_ref().unwrap());
            viewer.save()?;
            viewer.load(&save)?;
            viewer.refresh()?;
            let restored = viewer.session.as_ref().unwrap();
            ensure!(
                restored.simulation == before.simulation.state
                    && restored.host.snapshot() == before.assets,
                "active native action changed on UI resume"
            );
            ensure!(
                pixels == sims_render::software::render(viewer.scene.as_ref().unwrap()),
                "active native action pixels changed on resume"
            );
            saved_action = true;
        }
    }
    ensure!(
        dispatched && seated && saved_action,
        "player Sit did not dispatch and reach the original chair"
    );
    viewer.refresh()?;
    println!(
        "Mashuga: native player Sit dispatched and entered chair #{target} at tick {}",
        viewer.session.as_ref().unwrap().simulation.vm.tick
    );
    let _ = viewer.update(Message::Cancel(action.into()));
    ensure!(
        viewer.error.is_none(),
        "active cancellation failed: {:?}",
        viewer.error
    );
    let mut ended = false;
    for _ in 0..400 {
        viewer.step()?;
        let a = &viewer
            .session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .actions
            .as_ref()
            .unwrap()
            .actors[&resident];
        if a.active.as_ref().is_none_or(|a| a.id != action)
            && !a.queue.pending.iter().any(|a| a.id == action)
        {
            ended = true;
            break;
        }
    }
    ensure!(ended, "cancelled native Sit did not end");
    println!(
        "Mashuga: pending cancellation, active-action save/resume and active cancellation passed"
    );
    std::fs::remove_file(save)?;
    Ok(())
}
