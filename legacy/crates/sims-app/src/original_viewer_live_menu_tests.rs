use super::transport_tests::drain;
use super::*;
use clap::Parser;
use serde_json::{Value, json};

fn state(v: &Viewer) -> Result<Vec<u8>> {
    let s = v.session.as_ref().unwrap();
    Ok(serde_json::to_vec(&json!({"session":s.save(),
        "client":v.client.as_ref().unwrap().save(s)?,
        "device":v.device.as_ref().unwrap().save(s)?}))?)
}
fn anchor(v: &mut Viewer) -> Instant {
    let now = Instant::now();
    v.last = Some(now);
    v.clock_origin = Some((
        now,
        v.session
            .as_ref()
            .unwrap()
            .host
            .presentation
            .clock
            .as_ref()
            .unwrap()
            .now,
    ));
    now
}
fn frame(v: &mut Viewer, inputs: &mut Vec<Value>) -> Result<()> {
    let now = anchor(v);
    let _ = v.update(Message::Frame(now + std::time::Duration::from_millis(50)));
    drain(v, inputs)
}
fn compare(v: &Viewer, expected: &[u8], out: &Path, label: &str) -> Result<()> {
    let actual = state(v)?;
    if actual != expected {
        std::fs::write(out.join(format!("{label}-expected.json")), expected)?;
        std::fs::write(out.join(format!("{label}-actual.json")), actual)?;
        anyhow::bail!("{label}: full restored state differs");
    }
    Ok(())
}

#[test]
#[ignore = "private independent Mashuga checkpoint; headless Iced actor-less clicks"]
fn mashuga_object_and_ground_clicks_without_selected_person_do_not_freeze() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(no_selected_person)?
        .join()
        .expect("actor-less click test panicked")
}
fn no_selected_person() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let out = repo.join(std::env::var("SIMS_NO_SELECTED_PERSON_OUTPUT")?);
    ensure!(out.starts_with(repo.join("cache")));
    std::fs::create_dir(&out)?;
    //Research checkpoint only; do not inspect/replace the player's save slot.
    let source = repo.join("cache/post-buy-mashuga-4096-observed-20261022-first/midpoint.json");
    let save = out.join("no-selected-person.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData2/Houses/House09.iff",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        source.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let mut inputs = vec![];
    drain(&mut v, &mut inputs)?;
    //The retained checkpoint is zoomed into the house. Use the actual camera
    //controls to expose the pool-corner lamps before testing their pixels.
    for _ in 0..2 {
        let _ = v.update(Message::ZoomOut);
        drain(&mut v, &mut inputs)?;
    }
    ensure!(
        v.session
            .as_ref()
            .unwrap()
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            .is_none()
    );
    v.playing = true;
    //The reported pool-corner object is the original garden lamp.
    let (lamp, pixel) = [131, 130]
        .into_iter()
        .find_map(|id| object_tests::object_pixel(&v, id).map(|pixel| (id, pixel)))
        .context("no garden-lamp pixel visible")?;
    ensure!(
        v.session.as_ref().unwrap().simulation.vm.world.objects[&lamp]
            .definition
            .as_ref()
            .unwrap()
            .source
            .ends_with("::lampgarden.iff")
    );
    let frozen = serde_json::to_vec(&v.session.as_ref().unwrap().simulation)?;
    object_tests::click(&mut v, lamp, pixel)?;
    ensure!(v.error.is_none() && v.playing && v.selected.is_none());
    ensure!(!v.session.as_ref().unwrap().player_input_pending());
    ensure!(frozen == serde_json::to_vec(&v.session.as_ref().unwrap().simulation)?);
    let (ground, _) = ground_tests::click_point(&v)?;
    let _ = v.update(Message::ViewportClick(ground));
    ensure!(v.error.is_none() && v.playing && v.selected.is_none());
    ensure!(!v.session.as_ref().unwrap().player_input_pending());
    ensure!(frozen == serde_json::to_vec(&v.session.as_ref().unwrap().simulation)?);
    //The ordinary no-person early return precedes object definitions/factories.
    //It must still reject corrupt registry data instead of hiding real errors.
    {
        let s = v.session.as_mut().unwrap();
        let before = serde_json::to_vec(&s.save())?;
        ensure!(!s.begin_object_input(lamp)?);
        ensure!(!s.begin_ground_input([440, 360], 1)?);
        ensure!(before == serde_json::to_vec(&s.save())?);
        let person = s.simulation.vm.world.person.as_ref().unwrap().person_order[0];
        let flags = s
            .simulation
            .vm
            .world
            .objects
            .get_mut(&person)
            .unwrap()
            .native_flags
            .take();
        ensure!(
            s.begin_object_input(lamp).unwrap_err().to_string() == "selection native flags absent"
        );
        s.simulation
            .vm
            .world
            .objects
            .get_mut(&person)
            .unwrap()
            .native_flags = flags;
    }
    v.save()?;
    let expected = state(&v)?;
    let before_render = state(&v)?;
    render_headless_element(&root, &out.join("no-selected-person.png"), v.view())?;
    ensure!(
        before_render == state(&v)?,
        "drawing changed retained state"
    );
    let start = v.session.as_ref().unwrap().simulation.vm.tick;
    let mut suffix = vec![];
    for _ in 0..8 {
        frame(&mut v, &mut inputs)?;
        suffix.push(state(&v)?);
    }
    ensure!(v.session.as_ref().unwrap().simulation.vm.tick > start);
    v.load(&save)?;
    compare(&v, &expected, &out, "loaded-no-person")?;
    v.playing = true;
    for (i, expected) in suffix.iter().enumerate() {
        frame(&mut v, &mut inputs)?;
        compare(&v, expected, &out, &format!("no-person-resumed-{i}"))?;
    }
    //A portrait click remains the explicit way to select the interaction actor.
    let _ = v.update(Message::Resident(v.resident));
    drain(&mut v, &mut inputs)?;
    ensure!(
        v.session
            .as_ref()
            .unwrap()
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            == Some(v.resident)
    );
    let _ = v.update(Message::Select(Some(187))); //Original Mashuga dining chair.
    drain(&mut v, &mut inputs)?;
    ensure!(v.session.as_ref().unwrap().running_menu());
    let menu = out.join("selected-chair-menu.json");
    v.args.save = menu.clone();
    v.save()?;
    let expected_menu = state(&v)?;
    v.load(&menu)?;
    compare(&v, &expected_menu, &out, "selected-chair-menu-restored")?;
    v.playing = true;
    let _ = v.update(Message::ObjectCancel);
    drain(&mut v, &mut inputs)?;
    ensure!(!v.session.as_ref().unwrap().player_input_pending());
    let (ground, _) = ground_tests::click_point(&v)?;
    let _ = v.update(Message::ViewportClick(ground));
    drain(&mut v, &mut inputs)?;
    ensure!(v.session.as_ref().unwrap().running_menu());
    let _ = v.update(Message::GroundCancel);
    drain(&mut v, &mut inputs)?;
    ensure!(!v.session.as_ref().unwrap().player_input_pending());
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "lamp":lamp,"initial_tick":start,"final_tick":v.session.as_ref().unwrap().simulation.vm.tick,
            "object_and_ground_clicks_preserve_simulation":true,"no_automatic_actor_selection":true,
            "clock_continues":true,"cold_restored_frames":suffix.len(),
            "render_preserves_retained_state":true,"malformed_registry_rejected":true,
            "explicit_portrait_selection_opens_object_and_ground_menus":true,
            "selected_chair_menu_cold_restore_and_cancel":true,"strict_original_parity_ticks_added":0,
            "source":source,"inputs":inputs
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private managed Mashuga checkpoint; actual Iced resize/edge events and headless Metal"]
fn mashuga_resize_with_open_ground_menu_keeps_simulation_running() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(resize_menu)?
        .join()
        .expect("resize/menu test panicked")
}
fn resize_menu() -> Result<()> {
    use widget::shader::Program as _;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let out = repo.join(std::env::var("SIMS_RESIZE_MENU_OUTPUT")?);
    ensure!(out.starts_with(repo.join("cache")));
    std::fs::create_dir(&out)?;
    // Immutable checkpoint used to launch the player's Mashuga slot. Never
    // read or replace their subsequently edited saves/mashuga.json.
    let source = repo.join("cache/post-buy-mashuga-4096-observed-20261022-first/midpoint.json");
    let save = out.join("open-menu.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData2/Houses/House09.iff",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        source.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    v.playing = true;
    let mut inputs = vec![];
    drain(&mut v, &mut inputs)?;
    let _ = v.update(Message::Resident(v.resident));
    drain(&mut v, &mut inputs)?;
    ensure!(
        v.session
            .as_ref()
            .unwrap()
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            == Some(v.resident),
        "resident click did not select person"
    );
    let (pixel, _) = ground_tests::click_point(&v)?;
    let _ = v.update(Message::ViewportClick(pixel));
    drain(&mut v, &mut inputs)?;
    ensure!(v.session.as_ref().unwrap().running_menu());
    let start = v.session.as_ref().unwrap().simulation.vm.tick;
    for size in [Size::new(960., 720.), Size::new(1512., 982.)] {
        let before = serde_json::to_vec(&v.session.as_ref().unwrap().simulation)?;
        let _ = v.update(Message::DisplaySize(size));
        ensure!(v.error.is_none(), "resize alone: {:?}", v.error);
        ensure!(
            before == serde_json::to_vec(&v.session.as_ref().unwrap().simulation)?,
            "resize processed simulation state"
        );
    }
    for message in [Message::Pan([1, 0]), Message::Scroll([32, 0])] {
        let before = state(&v)?;
        let _ = v.update(message);
        ensure!(v.error.is_none() && v.pan == [0, 0]);
        ensure!(
            before == state(&v)?,
            "blocked camera input changed retained state"
        );
    }
    let scene = v.scene.as_ref().unwrap().clone();
    let bounds = iced::Rectangle {
        x: 0.,
        y: 0.,
        width: scene.width as f32,
        height: scene.height as f32,
    };
    // During a native window resize the pointer can cross a viewport edge.
    // Exercise the actual shader event that emits continuous pan intent.
    let point = iced::Point::new(bounds.width - 1., bounds.height / 2.);
    let action = Viewport(scene)
        .update(
            &mut Default::default(),
            &iced::Event::Mouse(iced::mouse::Event::CursorMoved { position: point }),
            bounds,
            iced::mouse::Cursor::Available(point),
        )
        .context("edge event absent")?;
    let (message, _, _) = action.into_inner();
    let _ = v.update(message.context("edge message absent")?);
    ensure!(v.pan == [0, 0], "popup retained edge-scroll intent");
    let now = anchor(&mut v);
    let _ = v.update(Message::Frame(now + std::time::Duration::from_millis(50)));
    std::fs::write(
        out.join("edge-result.json"),
        serde_json::to_vec_pretty(&json!({
            "error":v.error,"playing":v.playing,"pan":v.pan,"start_tick":start,
            "tick":v.session.as_ref().unwrap().simulation.vm.tick,
            "menu_retained":v.session.as_ref().unwrap().running_menu(),
            "scope":"Rust Iced input ownership regression; zero original gameplay parity ticks"
        }))?,
    )?;
    ensure!(v.error.is_none(), "edge frame: {:?}", v.error);
    drain(&mut v, &mut inputs)?;
    let mut suffix = vec![];
    for i in 0..16 {
        frame(&mut v, &mut inputs)?;
        if i == 7 {
            v.save()?;
        }
        if i >= 8 {
            suffix.push(state(&v)?);
        }
    }
    ensure!(
        v.playing && v.session.as_ref().unwrap().simulation.vm.tick > start,
        "resized menu froze household ticks"
    );
    let before = state(&v)?;
    render_headless_element(&root, &out.join("resized-menu.png"), v.view())?;
    ensure!(before == state(&v)?, "headless drawing processed state");
    v.load(&save)?;
    v.playing = true;
    for (i, expected) in suffix.iter().enumerate() {
        frame(&mut v, &mut inputs)?;
        compare(
            &v,
            expected,
            &out,
            &format!("resized-menu-restored-frame-{i}"),
        )?;
    }
    // The old app latches this error outside the retained Session. Exercise
    // existing Save/Load handlers as a non-destructive recovery path: the
    // player can preserve unsaved household state before clearing that latch.
    v.fail("player input owns the live UI");
    let simulation = serde_json::to_vec(&v.session.as_ref().unwrap().simulation)?;
    let _ = v.update(Message::Save);
    ensure!(save.exists() && v.error.as_deref() == Some("player input owns the live UI"));
    let _ = v.update(Message::Load);
    ensure!(v.error.is_none(), "recovery load: {:?}", v.error);
    ensure!(
        simulation == serde_json::to_vec(&v.session.as_ref().unwrap().simulation)?,
        "recovery Save/Load processed simulation"
    );
    ensure!(v.session.as_ref().unwrap().running_menu());
    let _ = v.update(Message::GroundCancel);
    drain(&mut v, &mut inputs)?;
    ensure!(v.error.is_none() && !v.session.as_ref().unwrap().player_input_pending());
    let center = v.session.as_ref().unwrap().retained_camera_input()?.center;
    let tick = v.session.as_ref().unwrap().simulation.vm.tick;
    let _ = v.update(Message::Scroll([32, 0]));
    drain(&mut v, &mut inputs)?;
    ensure!(
        v.session.as_ref().unwrap().retained_camera_input()?.center != center,
        "camera scroll did not return after popup cancellation"
    );
    ensure!(
        v.session.as_ref().unwrap().simulation.vm.tick == tick,
        "camera scroll advanced the lot clock"
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "start_tick":start,"final_tick":v.session.as_ref().unwrap().simulation.vm.tick,
            "resize_preserves_simulation":true,"render_preserves_retained_state":true,
            "matching_restored_frames":suffix.len(),"cancel_returns_input":true,"inputs":inputs,
            "save_load_recovers_latched_error_without_simulation_changes":true,
        "camera_scroll_returns_after_cancel_without_ticks":true,
            "strict_original_parity_ticks":0
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private Goth household; actual Iced menu controls and headless frames"]
fn iced_live_menus_advance_pause_and_restore_pending_choices() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("live menu test panicked")
}
fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let out = repo.join(std::env::var("SIMS_LIVE_MENU_ICED_OUTPUT")?);
    ensure!(out.starts_with(repo.join("cache")));
    std::fs::create_dir(&out)?;
    let base = out.join("live-base.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--save",
        base.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let mut inputs = vec![];
    drain(&mut v, &mut inputs)?;
    // A paused household remains paused even though a popup retains input.
    let target = 198;
    let _ = v.update(Message::Select(Some(target)));
    drain(&mut v, &mut inputs)?;
    ensure!(
        v.session.as_ref().unwrap().running_menu(),
        "popup did not acquire its scope"
    );
    let tick = v.session.as_ref().unwrap().simulation.vm.tick;
    let people = v.session.as_ref().unwrap().simulation.characters.clone();
    for _ in 0..3 {
        frame(&mut v, &mut inputs)?;
    }
    ensure!(v.session.as_ref().unwrap().simulation.vm.tick == tick);
    ensure!(v.session.as_ref().unwrap().simulation.characters == people);
    let _ = v.update(Message::ObjectCancel);
    anchor(&mut v);
    drain(&mut v, &mut inputs)?;
    ensure!(!v.session.as_ref().unwrap().player_input_pending());
    let _ = v.update(Message::Toggle);
    drain(&mut v, &mut inputs)?;
    frame(&mut v, &mut inputs)?;
    v.args.save = base.clone();
    v.save()?;
    let mut reports = vec![];
    for (ground, cancel) in [(false, true), (false, false), (true, true), (true, false)] {
        let label = format!(
            "{}-{}",
            if ground { "ground" } else { "object" },
            if cancel { "cancel" } else { "choose" }
        );
        let dir = out.join(&label);
        std::fs::create_dir(&dir)?;
        v.load(&base)?;
        v.playing = true;
        v.refresh()?;
        if ground {
            let (pixel, _) = ground_tests::click_point(&v)?;
            let _ = v.update(Message::ViewportClick(pixel));
        } else {
            let _ = v.update(Message::Select(Some(target)));
        }
        drain(&mut v, &mut inputs)?;
        let s = v.session.as_ref().unwrap();
        ensure!(s.running_menu() && !s.player_input_blocks_ticks());
        let menu_target = if ground {
            s.host
                .presentation
                .ground_input
                .as_ref()
                .unwrap()
                .target()
                .unwrap()
        } else {
            s.host.presentation.object_input.as_ref().unwrap().target()
        };
        let start = s.simulation.vm.tick;
        let records = if ground {
            s.host
                .presentation
                .ground_input
                .as_ref()
                .unwrap()
                .actions()
                .to_vec()
        } else {
            s.host
                .presentation
                .object_input
                .as_ref()
                .unwrap()
                .actions()
                .to_vec()
        };
        {
            let s = v.session.as_mut().unwrap();
            let before = serde_json::to_vec(&s.save())?;
            ensure!(
                s.submit_menu_command(sims_runtime::original_session::live_menu::Command::Choose(
                    usize::MAX
                ))
                .is_err()
            );
            ensure!(before == serde_json::to_vec(&s.save())?);
            // Deliberately model ID reuse without running callbacks: a stale
            // pointer must not become a command for the same-definition heir.
            let world = s.simulation.vm.world.clone();
            s.simulation
                .vm
                .world
                .unregister_scheduler_object(menu_target)
                .map_err(anyhow::Error::msg)?;
            let mut replacement = s.simulation.vm.world.objects.remove(&menu_target).unwrap();
            s.simulation
                .vm
                .world
                .register_scheduler_object(menu_target, &mut replacement)
                .map_err(anyhow::Error::msg)?;
            s.simulation
                .vm
                .world
                .objects
                .insert(menu_target, replacement);
            let stale = serde_json::to_vec(&s.save())?;
            let error = s
                .submit_menu_command(sims_runtime::original_session::live_menu::Command::Choose(
                    0,
                ))
                .unwrap_err();
            ensure!(error.to_string().contains("lifetime ended"));
            ensure!(stale == serde_json::to_vec(&s.save())?);
            s.simulation.vm.world = world;
            ensure!(before == serde_json::to_vec(&s.save())?);
        }
        let checkpoint = dir.join("open-menu.json");
        v.args.save = checkpoint.clone();
        let before = state(&v)?;
        render_headless_element(&root, &dir.join("opened.png"), v.view())?;
        ensure!(state(&v)? == before);
        let mut expected = vec![];
        for i in 0..16 {
            frame(&mut v, &mut inputs)?;
            if i == 7 {
                v.save()?;
            }
            if i >= 8 {
                expected.push(state(&v)?);
            }
        }
        let after_tick = v.session.as_ref().unwrap().simulation.vm.tick;
        ensure!(after_tick > start, "open menu froze simulation");
        let current = if ground {
            v.session
                .as_ref()
                .unwrap()
                .host
                .presentation
                .ground_input
                .as_ref()
                .unwrap()
                .actions()
        } else {
            v.session
                .as_ref()
                .unwrap()
                .host
                .presentation
                .object_input
                .as_ref()
                .unwrap()
                .actions()
        };
        ensure!(
            current == records,
            "menu eligibility was recomputed while open"
        );
        let before = state(&v)?;
        render_headless_element(&root, &dir.join("advanced.png"), v.view())?;
        ensure!(state(&v)? == before);
        v.load(&checkpoint)?;
        v.playing = true;
        for (i, wanted) in expected.iter().enumerate() {
            frame(&mut v, &mut inputs)?;
            compare(&v, wanted, &dir, &format!("frame-{i}"))?;
        }
        // An actual Iced choice arrives while the elapsed caller is suspended.
        // It is saved as input, not injected into a half-completed VM pass.
        let s = v.session.as_mut().unwrap();
        let now = s
            .host
            .presentation
            .clock
            .as_ref()
            .unwrap()
            .now
            .wrapping_add(200);
        s.set_clock_time(now)?;
        s.begin_clock_frame()?;
        let simulation = serde_json::to_vec(&s.simulation)?;
        let _ = v.update(match (ground, cancel) {
            (true, true) => Message::GroundCancel,
            (true, false) => Message::GroundChoice(0),
            (false, true) => Message::ObjectCancel,
            (false, false) => Message::ObjectChoice(0),
        });
        ensure!(v.error.is_none(), "{:?}", v.error);
        ensure!(v.session.as_ref().unwrap().menu_command_pending());
        ensure!(simulation == serde_json::to_vec(&v.session.as_ref().unwrap().simulation)?);
        let pending = dir.join("pending-choice.json");
        v.args.save = pending.clone();
        v.save()?;
        anchor(&mut v);
        drain(&mut v, &mut inputs)?;
        let final_state = state(&v)?;
        let s = v.session.as_ref().unwrap();
        ensure!(!s.running_menu() && !s.player_input_pending());
        ensure!(
            s.host
                .presentation
                .clock
                .as_ref()
                .unwrap()
                .fields
                .poll_messages
                == 0
        );
        if ground && cancel {
            ensure!(!s.simulation.vm.world.objects.contains_key(&menu_target));
        }
        if !cancel {
            let actor = &s.simulation.vm.world.actions.as_ref().unwrap().actors[&v.resident];
            ensure!(
                actor
                    .queue
                    .pending
                    .iter()
                    .any(|a| a.target == menu_target && a.priority == 50),
                "chosen action was not accepted into the original player queue"
            );
            ensure!(s.simulation.vm.world.objects.contains_key(&menu_target));
        }
        if !ground {
            ensure!(s.simulation.vm.world.objects.contains_key(&target));
        }
        v.load(&pending)?;
        v.playing = true;
        anchor(&mut v);
        drain(&mut v, &mut inputs)?;
        compare(&v, &final_state, &dir, "pending-choice")?;
        let before = state(&v)?;
        render_headless_element(&root, &dir.join("returned.png"), v.view())?;
        ensure!(state(&v)? == before);
        reports.push(json!({"scenario":label,"start_tick":start,"after_open_frames_tick":after_tick,
            "target":menu_target,"matching_restored_frames":expected.len(),"pending_choice_restore":true,
            "unchanged_eligibility":true,"clock_scope_restored":true,"render_preserves_state":true,
            "player_queue_accepted":if cancel { None } else { Some(true) }}));
        println!("{label}: {start} -> {after_tick}; restore and return match");
    }
    // A menu input also waits for a synchronous UI child outside the elapsed
    // caller. Its notification cannot run inside the retained portrait scope.
    v.load(&base)?;
    let _ = v.update(Message::Select(Some(target)));
    drain(&mut v, &mut inputs)?;
    let s = v.session.as_mut().unwrap();
    s.begin_ui_portrait()?;
    s.submit_menu_command(sims_runtime::original_session::live_menu::Command::Cancel)?;
    let held = serde_json::to_vec(&s.save())?;
    ensure!(s.dispatch_menu_command()?.is_none());
    ensure!(held == serde_json::to_vec(&s.save())?);
    s.finish_ui_portrait()?;
    ensure!(matches!(
        s.dispatch_menu_command()?,
        Some(sims_runtime::original_session::live_menu::Outcome::Cancelled)
    ));
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
        "paused_menu_frames":3,"paused_people_unchanged":true,"inputs":inputs,"scenarios":reports,
            "native_registry_scheduler":true,"routing":"explicit inferred rectangles",
            "stale_lifetimes_rejected_without_mutation":true,"invalid_indices_rejected_without_mutation":true,
            "input_waits_for_non_clock_ui_child":true}))?,
    )?;
    Ok(())
}
