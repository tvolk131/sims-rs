use super::*;

#[test]
#[ignore = "private actual original tutorial modal checkpoint; wrapper run-state fixtures only"]
fn native_frame_clock_modal_preserves_run_request_without_play_override() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(modal_run_request)?
        .join()
        .expect("native modal clock worker panicked")
}

fn modal_run_request() -> Result<()> {
    use clap::Parser;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let input = repo
        .join(std::env::var("SIMS_NATIVE_MODAL_CLOCK_INPUT")?)
        .canonicalize()?;
    ensure!(input.starts_with(repo.join("cache")));
    let root = repo.join("game-data/cabinet-extracted");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        input.to_str().unwrap(),
    ]);
    let ready = Ready::load(&args)?;
    let bytes = std::fs::read(&input)?;
    let tick_rng = |app: &Application| -> Result<(u64, [Option<u32>; 3])> {
        let vm = &app.session()?.simulation.vm;
        Ok((
            vm.tick,
            [Some(vm.rng_state), vm.lot_rng_state, vm.crt_rng_state],
        ))
    };
    for running in [true, false] {
        let mut app = Application::restore_checkpoint(
            &args,
            ready
                .restore_index
                .as_ref()
                .context("modal restore index absent")?,
            &ready.icons,
            Instant::now(),
            &bytes,
        )?;
        app.check_errors()?;
        let dialogs = &app.session()?.simulation.vm.world.dialogs;
        let modal = dialogs.modal.context("modal fixture has no actual modal")?;
        let record = dialogs
            .records
            .iter()
            .find(|record| record.id == modal)
            .context("modal record absent")?
            .clone();
        let encoded = serde_json::to_value(&record)?;
        ensure!(encoded["origin"]["bhav"] == 4135 && encoded["origin"]["instruction"] == 13);
        ensure!(encoded["buttons"] == json!([{"label":"OK","result":2}]));
        ensure!(app.session()?.simulation.vm.world.globals[&17] == 0);
        app.viewer.playing = running;
        let before = tick_rng(&app)?;
        for _ in 0..3 {
            app.execute(&Command::Advance { millis: 250 }, &Limits::default())?;
            ensure!(
                app.viewer.playing == running,
                "modal changed the wrapper run request"
            );
            ensure!(
                tick_rng(&app)? == before,
                "unanswered modal advanced native tick/RNG"
            );
            ensure!(app.session()?.simulation.vm.world.dialogs.modal == Some(modal));
        }
        app.execute(
            &Command::Dialog {
                id: modal,
                result: 2,
                body_sha256: engine::digest(record.body.as_bytes()),
            },
            &Limits::default(),
        )?;
        ensure!(app.viewer.playing == running);
        ensure!(app.session()?.simulation.vm.world.dialogs.modal.is_none());
        ensure!(app.session()?.simulation.vm.world.globals[&17] == 0);
        let after_answer = tick_rng(&app)?;
        for _ in 0..8 {
            app.execute(&Command::Advance { millis: 250 }, &Limits::default())?;
            ensure!(app.viewer.playing == running);
        }
        if running {
            ensure!(
                tick_rng(&app)?.0 > after_answer.0,
                "answered live modal left scheduling stopped"
            );
        } else {
            ensure!(
                tick_rng(&app)? == after_answer,
                "modal response resumed a stopped wrapper"
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "retained genuine Newbie Buy tutorial checkpoint and installed content; no window"]
fn native_frame_clock_buy_continues_elapsed_polling_without_play() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(buy_partial_polling)?
        .join()
        .expect("native Buy clock regression panicked")
}

fn buy_partial_polling() -> Result<()> {
    use clap::Parser;

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let input = repo.join(std::env::var("SIMS_NATIVE_BUY_CLOCK_INPUT")?);
    ensure!(input.canonicalize()?.starts_with(repo.join("cache")));
    let root = repo.join("game-data/cabinet-extracted");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
        "--paused",
        "--root",
        root.to_str().context("root path is not UTF-8")?,
        "--load",
        input.to_str().context("checkpoint path is not UTF-8")?,
    ]);
    let ready = Ready::load(&args)?;
    let mut app = Application::restore_checkpoint(
        &args,
        ready
            .restore_index
            .as_ref()
            .context("restore index absent")?,
        &ready.icons,
        Instant::now(),
        &std::fs::read(&input)?,
    )?;
    app.check_errors()?;
    ensure!(!app.viewer.playing);
    ensure!(app.session()?.simulation.vm.world.globals[&17] == 0);
    ensure!(app.session()?.simulation.vm.world.globals[&19] == 2);
    ensure!(app.session()?.simulation.vm.world.globals[&10] != 0);
    ensure!(app.viewer.native_partial_pass()?);
    ensure!(app.viewer.clock_origin.is_none());
    let before_tick = app.session()?.simulation.vm.tick;
    let before_clock = app
        .session()?
        .host
        .presentation
        .clock
        .as_ref()
        .context("native elapsed clock absent")?
        .now;
    let before_calendar = app
        .session()?
        .simulation
        .household
        .as_ref()
        .unwrap()
        .clock
        .clone();
    for _ in 0..4 {
        app.millis += 250;
        let _ = app.viewer.update(Message::Frame(
            app.epoch + Duration::from_millis(app.millis),
        ));
        app.drain(&Limits::default())?;
    }
    let session = app.session()?;
    ensure!(app.viewer.clock_origin.is_some());
    ensure!(session.host.presentation.clock.as_ref().unwrap().now != before_clock);
    ensure!(!app.viewer.playing);
    ensure!(session.simulation.vm.tick == before_tick);
    ensure!(session.simulation.household.as_ref().unwrap().clock == before_calendar);
    Ok(())
}

#[test]
#[ignore = "retained actual sleep Boost notification checkpoint and installed content; no window"]
fn native_frame_clock_sleep_boost_preserves_pending_owner_and_real_indicator_across_restore()
-> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(sleep_boost_notification)?
        .join()
        .expect("native sleep Boost regression panicked")
}

fn sleep_boost_notification() -> Result<()> {
    use clap::Parser;
    use sims_runtime::original_session::notifications;
    use sims_vm::commander::house::Request;

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let input = repo.join(std::env::var("SIMS_NATIVE_SPEED_OVERRIDE_INPUT")?);
    ensure!(input.canonicalize()?.starts_with(repo.join("cache")));
    let root = repo.join("game-data/cabinet-extracted");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData2/Houses/House09.iff",
        "--paused",
        "--root",
        root.to_str().context("root path is not UTF-8")?,
        "--load",
        input.to_str().context("checkpoint path is not UTF-8")?,
    ]);
    let ready = Ready::load(&args)?;
    let mut app = Application::restore_checkpoint(
        &args,
        ready
            .restore_index
            .as_ref()
            .context("restore index absent")?,
        &ready.icons,
        Instant::now(),
        &std::fs::read(&input)?,
    )?;
    let historical_error = app.viewer.error.clone();
    ensure!(
        historical_error.as_deref()
            == Some("original speed-override Windows panel service is not bound")
    );
    ensure!(app.viewer.menu_error.is_none());
    let session = app.session()?;
    let before_tick = session.simulation.vm.tick;
    let before_world = serde_json::to_vec(&session.simulation.vm.world)?;
    ensure!(before_tick == 188647);
    ensure!(
        session.simulation.vm.world.globals[&16] == 1000
            && session.simulation.vm.world.globals[&18] == 760
    );
    ensure!(session.clock_pending() && session.house_notification_pending());
    ensure!(
        session
            .host
            .presentation
            .notifications
            .as_ref()
            .unwrap()
            .dispatch
            .root_message()
            == Some((0x109, 1))
    );
    let device = app.viewer.device.as_mut().unwrap();
    device.advance_mode_clock(Duration::from_millis(123_456))?;
    let indicator_now = device.mode_window().unwrap().clock();
    let mut saw_service = false;
    let mut saw_pending_indicator = false;
    for operation in 0..128 {
        if let Some(notifications::Service::Listener(caller)) = app
            .session()?
            .host
            .presentation
            .notifications
            .as_ref()
            .and_then(|state| state.pending())
        {
            ensure!(caller.request == Request::SpeedOverride { active: true });
            saw_service = true;
            let before = app
                .session()?
                .host
                .presentation
                .notifications
                .as_ref()
                .unwrap()
                .clone();
            ensure!(matches!(
                app.viewer.session.as_mut().unwrap().advance_clock(1)?,
                sims_runtime::original_session::clock::Progress::Pending
            ));
            ensure!(
                app.session()?
                    .host
                    .presentation
                    .notifications
                    .as_ref()
                    .unwrap()
                    == &before
            );
        }
        let viewer = &mut app.viewer;
        let complete = viewer
            .device
            .as_mut()
            .context("sleep indicator device absent")?
            .advance_notification_with_client(
                viewer.session.as_mut().unwrap(),
                viewer
                    .client
                    .as_mut()
                    .context("sleep indicator client absent")?,
                1,
            )?;
        saw_pending_indicator |= app
            .viewer
            .device
            .as_ref()
            .unwrap()
            .panel_widgets()
            .unwrap()
            .button_callback_pending();
        let saved = app.checkpoint()?;
        app = app.restore(&saved)?;
        ensure!(app.viewer.error == historical_error && app.viewer.menu_error.is_none());
        ensure!(app.session()?.simulation.vm.tick == before_tick);
        ensure!(serde_json::to_vec(&app.session()?.simulation.vm.world)? == before_world);
        if complete {
            break;
        }
        ensure!(operation < 127, "sleep speed notification did not return");
    }
    ensure!(saw_service);
    ensure!(!app.session()?.house_notification_pending());
    let device = app.viewer.device.as_ref().unwrap();
    ensure!(device.panel_widgets().unwrap().button_phase() == 3);
    ensure!(
        device
            .mode_window()
            .unwrap()
            .timers()
            .entries
            .iter()
            .any(|entry| entry.owner == 10002
                && entry.interval == 500
                && entry.argument == 0
                && entry.deadline == indicator_now.wrapping_add(500))
    );
    ensure!(
        app.session()?
            .host
            .presentation
            .transport
            .as_ref()
            .unwrap()
            .controls
            .speed
            == 760
    );
    ensure!(app.session()?.clock_pending());
    let _ = app.viewer.session.as_mut().unwrap().advance_clock(1)?;
    let clock = serde_json::to_value(&app.session()?.host.presentation.clock)?;
    ensure!(clock["call"]["notice"].is_null());
    ensure!(app.session()?.simulation.vm.tick == before_tick);
    ensure!(serde_json::to_vec(&app.session()?.simulation.vm.world)? == before_world);
    println!(
        "native sleep Boost real indicator restored at tick {before_tick}; callback observed={saw_pending_indicator}"
    );
    Ok(())
}

#[test]
#[ignore = "private original household assets; no window"]
fn native_frame_clock_live_return_uses_native_pause_without_unpausing_or_same_mode_resume()
-> Result<()> {
    use clap::Parser;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
    for paused in [true, false] {
        let args = Args::parse_from([
            "sims-room",
            "--original",
            "UserData/Houses/House05.iff",
            "--paused",
            "--buy-preview",
            "--root",
            root.to_str().unwrap(),
        ]);
        let ready = Arc::new(Ready::load(&args)?);
        let (mut viewer, _) = Viewer::new(args);
        viewer.install(ready)?;
        let mut inputs = Vec::new();
        transport_tests::drain(&mut viewer, &mut inputs)?;
        if !paused {
            let _ = viewer.update(Message::NativeSpeed(sims_vm::interface::speed::NORMAL));
            transport_tests::drain(&mut viewer, &mut inputs)?;
        }
        let before_pause = viewer.session.as_ref().unwrap().simulation.vm.world.globals[&17];
        ensure!((before_pause != 0) == paused);
        if paused {
            let before_tick = viewer.session.as_ref().unwrap().simulation.vm.tick;
            let before_clock = viewer
                .device
                .as_ref()
                .unwrap()
                .mode_window()
                .unwrap()
                .clock();
            let now = viewer.last.context("paused frontend has no clock sample")?;
            let _ = viewer.update(Message::Frame(now + Duration::from_millis(1_001)));
            transport_tests::drain(&mut viewer, &mut inputs)?;
            ensure!(
                viewer
                    .device
                    .as_ref()
                    .unwrap()
                    .mode_window()
                    .unwrap()
                    .clock()
                    == before_clock.wrapping_add(1_001)
            );
            ensure!(viewer.session.as_ref().unwrap().simulation.vm.tick == before_tick);
        }
        let _ = viewer.update(Message::Buy);
        transport_tests::drain(&mut viewer, &mut inputs)?;
        ensure!(
            viewer
                .session
                .as_ref()
                .unwrap()
                .host
                .presentation
                .transport
                .as_ref()
                .unwrap()
                .controls
                .mode
                == 1
        );
        viewer.playing = false;
        viewer.accumulator = 0.25;
        let _ = viewer.update(Message::Live);
        ensure!(viewer.playing == !paused && viewer.accumulator == 0.);
        transport_tests::drain(&mut viewer, &mut inputs)?;
        let session = viewer.session.as_ref().unwrap();
        ensure!(
            session
                .host
                .presentation
                .transport
                .as_ref()
                .unwrap()
                .controls
                .mode
                == 2
        );
        ensure!(session.simulation.vm.world.globals[&17] == before_pause);
        ensure!(viewer.playing == !paused);
        viewer.playing = false;
        let _ = viewer.update(Message::Live);
        transport_tests::drain(&mut viewer, &mut inputs)?;
        ensure!(!viewer.playing);
        ensure!(viewer.session.as_ref().unwrap().simulation.vm.world.globals[&17] == before_pause);
    }
    Ok(())
}
