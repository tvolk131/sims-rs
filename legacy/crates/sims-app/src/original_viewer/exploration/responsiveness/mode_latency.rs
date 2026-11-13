//! Measure individual production updates, not hashing, cold restore or GPU readback.
//! Paused mode changes still execute every ordinary engine service.
use super::*;

fn update(app: &mut Application, message: Message, rows: &mut Vec<Value>) -> Result<()> {
    ensure!(app.viewer.frame_profile.is_none());
    let label = format!("{message:?}");
    app.viewer.frame_profile = Some(FrameProfile::default());
    let start = Instant::now();
    let _ = app.viewer.update(message);
    let elapsed = start.elapsed().as_secs_f64() * 1000.;
    let stages = app.viewer.frame_profile.take().context("profile absent")?;
    rows.push(json!({"message":label,"ms":elapsed,"stages":stages,
        "pending":app.viewer.presentation_pending()}));
    app.check_errors()
}

fn finish(app: &mut Application, rows: &mut Vec<Value>) -> Result<()> {
    let now = app.epoch + Duration::from_millis(app.millis);
    for _ in 0..Limits::default().service_polls {
        if !app.viewer.presentation_pending() {
            return app.check_errors();
        }
        update(app, Message::Frame(now), rows)?;
    }
    anyhow::bail!("mode transition exceeded service poll limit")
}

#[test]
#[ignore = "private Mashuga checkpoint; per-update latency, actual Iced controls and cold restore"]
fn mashuga_mode_update_latency_and_state() -> Result<()> {
    run_worker(campaign)
}

fn campaign() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    select(&mut app, 196)?;
    let mut ui = Input::new()?;
    let initial = app.observe()?;
    write_json(&out.join("initial.json"), &app.capture(false)?)?;
    let mut operations = vec![];
    for (index, name) in ["Buy", "Live", "Buy", "Live"].into_iter().enumerate() {
        let (_, message) = sequences::control(&mut ui, &app, name)?;
        let before = app.observe()?;
        let mut rows = vec![];
        update(&mut app, message, &mut rows)?;
        if app.viewer.presentation_pending() {
            let now = app.epoch + Duration::from_millis(app.millis);
            update(&mut app, Message::Frame(now), &mut rows)?;
        }
        let busy = app.observe()?;
        // Replacement-only UI can respond between slices; gameplay input must
        // not reenter the synchronous original caller during that interval.
        if app.viewer.presentation_pending() {
            ensure!(!app.viewer.options_open);
            app.dispatch(Message::Options)?;
            ensure!(app.viewer.options_open);
            app.dispatch(Message::Options)?;
            ensure!(!app.viewer.options_open && app.observe()? == busy);
            app.dispatch(if name == "Buy" {
                Message::Live
            } else {
                Message::Buy
            })?;
            ensure!(
                app.observe()? == busy,
                "mode caller accepted reentrant input"
            );
            if index == 1 {
                ui.save(&app.viewer, &out.join("pending-Live.png"))?;
                ensure!(app.observe()? == busy, "pending redraw changed state");
            }
        }
        // Capture a real suspended owner before completing its remaining work.
        // Cold restoration and full-state comparisons are outside timed spans.
        let pending = app.viewer.presentation_pending();
        let checkpoint = app.checkpoint()?;
        let mut cold = app.restore(&checkpoint)?;
        ensure!(
            app.observe()? == cold.observe()?,
            "pending mode restore differs"
        );
        finish(&mut app, &mut rows)?;
        let mut cold_rows = vec![];
        finish(&mut cold, &mut cold_rows)?;
        ensure!(
            app.observe()? == cold.observe()?,
            "mode continuation differs"
        );
        let after = app.observe()?;
        ensure!(
            after.tick == before.tick && after.rng == before.rng,
            "paused mode change advanced tick/RNG"
        );
        let observed_mode = app
            .session()?
            .host
            .presentation
            .transport
            .as_ref()
            .context("transport absent")?
            .controls
            .mode;
        ensure!(observed_mode == if name == "Buy" { 1 } else { 2 });
        write_json(
            &out.join(format!("{index}-{name}-state.json")),
            &app.capture(false)?,
        )?;
        // A passive frame must neither run the VM nor mutate any saved owner.
        if index >= 2 {
            ui.save(&app.viewer, &out.join(format!("{index}-{name}.png")))?;
            ensure!(app.observe()? == after, "passive mode render changed state");
        }
        eprintln!(
            "MODE_LATENCY {}",
            json!({"mode":name,"updates":rows.len(),
            "max_update_ms":rows.iter().filter_map(|r|r["ms"].as_f64()).fold(0.,f64::max)})
        );
        operations.push(json!({"mode":name,"before":before,"after":after,
            "pending_checkpoint":pending,"cold_equal":true,
            "replacement_ui_between_slices":true,"updates":rows}));
        write_json(&out.join("operations.json"), &operations)?;
    }
    ensure!(app.observe()?.tick == initial.tick && app.observe()?.rng == initial.rng);
    write_json(
        &out.join("result.json"),
        &json!({"passed":true,"initial":initial,
        "final":app.observe()?,"operations":operations,"completed_ticks":0,
        "scope":"Per-update headless timings; exclude hashing, restore, widget discovery and GPU readback. No window FPS or native gameplay parity claim."}),
    )
}

#[test]
#[ignore = "private Mashuga content; isolate the synchronous controller GUID-discovery dependency"]
fn mashuga_controller_dependency_cost() -> Result<()> {
    run_worker(controller_dependency)
}

fn controller_dependency() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    let before = app.observe()?;
    let session = app.viewer.session.as_mut().context("session absent")?;
    let index = session.index().clone();
    let definition = session
        .host
        .presentation
        .move_tool
        .as_ref()
        .context("move catalog absent")?
        .history
        .named_definition(b"comeandsee")
        .map_err(anyhow::Error::msg)?
        .context("controller definition absent")?;
    let mut rows = vec![];
    for repetition in 0..3 {
        let program_before = engine::digest_serialized(&session.program)?;
        let start = Instant::now();
        sims_content::assets::load_object_guids(&index, &mut session.program, &[definition.guid])?;
        let lookup_ms = start.elapsed().as_secs_f64() * 1000.;
        let program_after = engine::digest_serialized(&session.program)?;
        let start = Instant::now();
        session
            .simulation
            .architecture
            .as_mut()
            .context("architecture absent")?
            .bind_footprints(&session.program)
            .map_err(anyhow::Error::msg)?;
        let footprints_ms = start.elapsed().as_secs_f64() * 1000.;
        rows.push(json!({"repetition":repetition,"lookup_ms":lookup_ms,
            "footprints_ms":footprints_ms,"program_before":program_before,
            "program_after":program_after}));
        eprintln!("CONTROLLER_DEPENDENCY {}", rows.last().unwrap());
    }
    let after = app.observe()?;
    ensure!(before.tick == after.tick && before.rng == after.rng);
    write_json(
        &out.join("result.json"),
        &json!({"passed":true,"definition":definition,
        "before":before,"after":after,"measurements":rows,
        "scope":"Direct existing dependency functions on the real checkpoint; timings exclude restore and hashing. No whole-transition or native parity claim.",
        "completed_ticks":0}),
    )
}

#[test]
#[ignore = "private Mashuga checkpoint; bounded ordinary mode loop for external CPU sampling"]
fn mashuga_mode_hot_loop_profile() -> Result<()> {
    run_worker(hot_loop)
}

fn hot_loop() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    select(&mut app, 196)?;
    let before = app.observe()?;
    let mut operations = vec![];
    eprintln!("MODE_HOT_LOOP_START");
    for repetition in 0..8 {
        for name in ["Buy", "Live"] {
            let mut rows = vec![];
            update(
                &mut app,
                if name == "Buy" {
                    Message::Buy
                } else {
                    Message::Live
                },
                &mut rows,
            )?;
            finish(&mut app, &mut rows)?;
            operations.push(json!({"repetition":repetition,"mode":name,"updates":rows}));
        }
    }
    eprintln!("MODE_HOT_LOOP_END");
    let after = app.observe()?;
    ensure!(before.tick == after.tick && before.rng == after.rng);
    write_json(
        &out.join("result.json"),
        &json!({"passed":true,"before":before,
        "after":after,"operations":operations,"completed_ticks":0,
        "scope":"Direct ordinary handlers for external CPU attribution; sampled timings are not a speedup benchmark. No GUI, native execution or player-save access."}),
    )
}
