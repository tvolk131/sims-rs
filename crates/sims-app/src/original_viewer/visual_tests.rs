use super::*;
use clap::Parser;

#[test]
#[ignore = "private v86 Newbie checkpoint, immutable-only installation and offscreen Metal adapter"]
fn managed_player_save_cold_loads_with_no_original_save_files() -> Result<()> {
    managed_iced_restore("cache/visual-corrections-20261022/iced-complete/queued-resume.json")
}

#[test]
#[ignore = "private v86 Newbie Buy checkpoint, immutable-only installation and offscreen Metal adapter"]
fn managed_buy_save_cold_loads_tool_catalog_and_history_without_original_saves() -> Result<()> {
    managed_iced_restore("cache/newbie-play-20261021-10/catalog.json")
}

fn managed_iced_restore(checkpoint: &str) -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_MANAGED_ICED_OUTPUT")?);
    let assets = repo.join(std::env::var("SIMS_MANAGED_ASSETS")?);
    ensure!(output.starts_with(repo.join("cache")) && assets.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    ensure!(!assets.join("UserData").exists() && !assets.join("UserData2").exists());
    let checkpoint = repo.join(checkpoint);
    let save = output.join("managed.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        checkpoint.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    ensure!(
        ready.session.lock().unwrap().is_none(),
        "cold load activated original house"
    );
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(ready)?;
    viewer.save()?;
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&save)?)?;
    ensure!(value["session"]["version"] == 2);
    let session = viewer.session.as_ref().unwrap();
    let before = serde_json::to_vec(&session.save())?;
    let client = serde_json::to_vec(&viewer.client.as_ref().unwrap().save(session)?)?;
    let device = viewer.device.as_ref().unwrap().save(session)?;
    let tick = session.simulation.vm.tick;
    let mode = session
        .host
        .presentation
        .presenter
        .as_ref()
        .unwrap()
        .camera
        .as_ref()
        .unwrap()
        .settings
        .application_mode;
    let old_pixels = sims_render::software::render(viewer.scene.as_ref().unwrap());
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        assets.to_str().unwrap(),
        "--load",
        save.to_str().unwrap(),
        "--save",
        output.join("cold.json").to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    ensure!(
        ready.session.lock().unwrap().is_none(),
        "cold load activated original house"
    );
    let (mut cold, _) = Viewer::new(args);
    cold.install(ready)?;
    let session = cold.session.as_ref().unwrap();
    ensure!(
        before == serde_json::to_vec(&session.save())?,
        "cold Iced restore changed complete session"
    );
    ensure!(
        client == serde_json::to_vec(&cold.client.as_ref().unwrap().save(session)?)?,
        "cold Iced client differs"
    );
    ensure!(
        old_pixels == sims_render::software::render(cold.scene.as_ref().unwrap()),
        "cold Iced scene differs"
    );
    // install() draws once. Inspect the exact device owner before the next
    // draw too, rather than removing its legitimate frame-generation counters.
    cold.load(&save)?;
    ensure!(
        device
            == cold
                .device
                .as_ref()
                .unwrap()
                .save(cold.session.as_ref().unwrap())?,
        "cold device/catalog/history differs before rendering"
    );
    cold.refresh()?;
    let frozen = retained(&cold)?;
    render_headless_element(&assets, &output.join("cold.png"), cold.view())?;
    ensure!(
        frozen == retained(&cold)?,
        "headless drawing mutated retained state"
    );
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "checkpoint":checkpoint,"mode":mode,"tick":tick,"original_save_namespaces_physically_absent":true,"cold_starting_activation":false,
            "full_session_equal":true,"client_equal":true,"device_catalog_history_equal":true,"software_pixels_equal":true,"drawing_state_unchanged":true,
            "original_parity_ticks_added":0,"continuation_ticks_added":0
        }))?,
    )?;
    Ok(())
}

fn retained(v: &Viewer) -> Result<Vec<u8>> {
    let s = v.session.as_ref().context("session absent")?;
    Ok(serde_json::to_vec(&(
        s.save(),
        v.client.as_ref().unwrap().save(s)?,
        v.device.as_ref().unwrap().save(s)?,
    ))?)
}

#[test]
#[ignore = "private original household assets and offscreen Metal adapter"]
fn native_hud_and_ground_render_without_mutating_retained_state() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_VISUAL_PARITY_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("visual evidence output must be fresh")?;
    let save = output.join("state.json");
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
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    for _ in 0..10000 {
        if !v.session.as_ref().unwrap().transport_pending() {
            break;
        }
        v.step()?;
    }
    ensure!(!v.session.as_ref().unwrap().transport_pending());
    v.refresh()?;
    let before = retained(&v)?;
    render_headless_element(&root, &output.join("collapsed.png"), v.view())?;
    ensure!(
        before == retained(&v)?,
        "drawing changed retained engine/device/client state"
    );
    let _ = v.update(Message::Options);
    render_headless_element(&root, &output.join("options.png"), v.view())?;
    ensure!(
        before == retained(&v)?,
        "application menu changed native state"
    );
    let _ = v.update(Message::Options);
    // Use the original retained expansion continuation, never edit panel flags.
    let _ = v.update(Message::ExpandPanel);
    let mut inputs = vec![];
    super::transport_tests::drain(&mut v, &mut inputs)?;
    ensure!(
        !v.device
            .as_ref()
            .is_some_and(|d| d.panel_state() == Some(1))
    );
    let expanded = retained(&v)?;
    render_headless_element(&root, &output.join("expanded.png"), v.view())?;
    ensure!(
        expanded == retained(&v)?,
        "expanded drawing changed native state"
    );
    // Existing full-session/device/client cold restore and software frame check.
    super::person_tests::checkpoint(&mut v, &output, "expanded-resume")?;
    // checkpoint() intentionally refreshes before saving and after restoring.
    // CameraDevice::prepare increments these two presentation counters once
    // per refresh. Its own pre/post-load comparison above includes them; all
    // remaining retained state must still match our pre-save state exactly.
    let before: serde_json::Value = serde_json::from_slice(&expanded)?;
    let mut after: serde_json::Value = serde_json::from_slice(&retained(&v)?)?;
    for key in ["generation", "published_generation"] {
        ensure!(
            after[2][key].as_u64() == before[2][key].as_u64().map(|n| n + 2),
            "unexpected checkpoint refresh count for {key}"
        );
        after[2][key] = before[2][key].clone();
    }
    ensure!(
        before == after,
        "checkpoint changed state beyond presentation refresh counters"
    );
    // Actual frontend resizes run the retained sizing caller, never the VM.
    // Exercise both aspect ratios and the original800x600 presentation size.
    let simulation = serde_json::to_vec(&v.session.as_ref().unwrap().save().simulation)?;
    for (width, height) in [(800, 600), (1000, 740), (1320, 900)] {
        let _ = v.update(Message::DisplaySize(Size::new(width as f32, height as f32)));
        ensure!(v.error.is_none(), "resize failed: {:?}", v.error);
        ensure!(
            simulation == serde_json::to_vec(&v.session.as_ref().unwrap().save().simulation)?,
            "resize advanced or changed simulation state"
        );
        let scene = v.scene.as_ref().context("resized scene absent")?;
        ensure!(
            [scene.width, scene.height] == [width, height - 100],
            "letterboxed active viewport"
        );
        let state = retained(&v)?;
        super::super::support::render_headless_element_at_size(
            &root,
            &output.join(format!("expanded-{width}x{height}.png")),
            v.view(),
            [width, height],
        )?;
        ensure!(state == retained(&v)?, "resized redraw mutated state");
        // Saved viewport owners roundtrip with the resized surface dimensions.
        let s = v.session.as_ref().unwrap();
        let device = CameraDevice::restore(s, v.device.as_ref().unwrap().save(s)?)?;
        ensure!(
            device.save(s)? == v.device.as_ref().unwrap().save(s)?,
            "resized device restore differs"
        );
    }
    // Queue through the original eligibility/action-input path from this
    // current startup, rather than an obsolete historical test save.
    let _ = v.update(Message::Select(Some(127)));
    ensure!(v.error.is_none(), "chair selection failed: {:?}", v.error);
    super::object_tests::menu(&mut v)?;
    let s = v.session.as_ref().unwrap();
    let input = s
        .host
        .presentation
        .object_input
        .as_ref()
        .context("chair input absent")?;
    let choice = input
        .actions()
        .iter()
        .position(|a| {
            s.simulation
                .vm
                .action_menu(&s.program, a.target)
                .is_ok_and(|entries| {
                    entries
                        .iter()
                        .any(|e| e.id == a.menu_id && e.label == "Sit")
                })
        })
        .context("chair Sit absent")?;
    let _ = v.update(Message::ObjectChoice(choice));
    super::transport_tests::drain(&mut v, &mut inputs)?;
    ensure!(v.error.is_none(), "queue input failed: {:?}", v.error);
    let s = v.session.as_ref().unwrap();
    let actor = &s.simulation.vm.world.actions.as_ref().unwrap().actors[&v.resident];
    let action = actor
        .queue
        .pending
        .front()
        .context("queued chair action absent")?;
    let owner = &s.simulation.vm.world.objects[&action.icon_owner];
    ensure!(
        v.thumbnails
            .as_ref()
            .and_then(|t| t.live_image(owner.definition.as_ref().unwrap()))
            .is_some(),
        "queue icon did not load"
    );
    let queued = retained(&v)?;
    render_headless_element(&root, &output.join("queued-chair.png"), v.view())?;
    ensure!(queued == retained(&v)?, "queue redraw mutated state");
    super::person_tests::checkpoint(&mut v, &output, "queued-resume")?;
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "render_state_unchanged": true, "menu_state_unchanged": true,
            "cold_restore_exact": true, "inputs": inputs,
            "tick": v.session.as_ref().unwrap().simulation.vm.tick,
        }))?,
    )?;
    Ok(())
}
