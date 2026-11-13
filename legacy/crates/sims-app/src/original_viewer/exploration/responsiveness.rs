//! Opt-in real-household input and frame diagnostics. Timing never includes
//! state hashing/serialization, and readback is not presented as display FPS.
use super::super::headless_input::Input;
use super::*;
use clap::Parser;

#[path = "responsiveness/batched_focus.rs"]
mod batched_focus;
#[path = "responsiveness/catalog_identity.rs"]
mod catalog_identity;
#[path = "responsiveness/event_sequences.rs"]
mod event_sequences;
#[path = "responsiveness/handler_latency.rs"]
mod handler_latency;
#[path = "responsiveness/modal_lifetimes.rs"]
mod modal_lifetimes;
#[path = "responsiveness/mode_latency.rs"]
mod mode_latency;
#[path = "responsiveness/overlay_batches.rs"]
mod overlay_batches;
#[path = "responsiveness/presentation_latency.rs"]
mod presentation_latency;
#[path = "responsiveness/resize_stress.rs"]
mod resize_stress;
#[path = "responsiveness/sequences.rs"]
mod sequences;
#[path = "responsiveness/stale_intents.rs"]
mod stale_intents;
#[path = "responsiveness/titles.rs"]
mod titles;
#[path = "responsiveness/ui_publication.rs"]
mod ui_publication;

fn output() -> Result<PathBuf> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = repo.join(std::env::var("SIMS_RESPONSIVENESS_OUTPUT")?);
    ensure!(
        out.starts_with(repo.join("cache")),
        "private evidence must stay in cache"
    );
    std::fs::create_dir(&out)?;
    Ok(out)
}
fn load(out: &Path) -> Result<Application> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let source = std::env::var("SIMS_RESPONSIVENESS_SOURCE")
        .unwrap_or_else(|_| "cache/gameplay-gaps-reproducers-20261023/phone/initial.json".into());
    let source = repo.join(source);
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let save = out.join("unused-player-save.json");
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
    let ready = Ready::load(&args)?;
    let bytes = std::fs::read(&source)?;
    std::fs::write(out.join("input.json"), &bytes)?;
    let mut app = Application::restore_checkpoint(
        &args,
        ready
            .restore_index
            .as_ref()
            .context("restore index absent")?,
        &ready.icons,
        Instant::now(),
        &bytes,
    )?;
    app.execute(&Command::Playing { value: false }, &Limits::default())?;
    app.drain(&Limits::default())?;
    Ok(app)
}
fn run_worker(f: fn() -> Result<()>) -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)?
        .join()
        .expect("headless responsiveness test panicked")
}
fn select(app: &mut Application, id: i16) -> Result<()> {
    let identity = Identity::get(app.session()?, id)?;
    app.execute(&Command::Resident { identity }, &Limits::default())
}
fn click_messages(ui: &mut Input, v: &Viewer, point: [f32; 2]) -> Result<Vec<Message>> {
    // Hover may publish viewport motion; only a released button can activate
    // these controls. Retain the same Tree for down/up and subsequent redraws.
    let hover = ui.move_to(v, point);
    ensure!(
        !hover
            .iter()
            .any(|m| !matches!(m, Message::ViewportMotion { .. } | Message::ViewportLeave)),
        "hover activated a control: {hover:?}"
    );
    let press = ui.button(v, true);
    ensure!(
        press.is_empty(),
        "HUD activated on press or leaked to lot: {press:?}"
    );
    Ok(ui.button(v, false))
}
#[test]
#[ignore = "private independent Mashuga checkpoint; actual Iced widget events and headless GPU"]
fn hud_pointer_state_matrix() -> Result<()> {
    run_worker(hud_matrix)
}
fn hud_matrix() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    select(&mut app, 196)?;
    let mut ui = Input::new()?;
    let mut rows = vec![];
    for [width, height] in [[800, 600], [1000, 740], [1320, 900]] {
        app.execute(&Command::Resize { width, height }, &Limits::default())?;
        let before = app.observe()?;
        let pixels = ui.pixels(&app.viewer)?;
        ensure!(app.observe()? == before, "drawing changed retained state");
        ensure!(
            pixels == ui.pixels(&app.viewer)?,
            "idle frame changed pixels"
        );
        for (id, offset, expected) in [
            (2009, [193., 125.], "Options"),
            (2012, [9., 127.], "RotateBack"),
            (2011, [43., 127.], "Rotate"),
            (2008, [69., 20.], "Buy"),
        ] {
            let (w, h, _) = app
                .viewer
                .icons
                .as_ref()
                .unwrap()
                .hud(id)
                .context("HUD artwork missing")?;
            let point = [
                offset[0] + *w as f32 / 2.,
                height as f32 - 183. + offset[1] + *h as f32 / 2.,
            ];
            let messages = click_messages(&mut ui, &app.viewer, point)?;
            ensure!(
                messages.len() == 1 && format!("{:?}", messages[0]) == expected,
                "{width}x{height} {id}: {messages:?}"
            );
            let after = app.observe()?;
            ensure!(before == after, "undispatched pointer events changed game");
            // Leaving a pressed control for the viewport must cancel
            // activation. No lot pick may leak through on release.
            ui.move_to(&app.viewer, point);
            ensure!(ui.button(&app.viewer, true).is_empty());
            ui.move_to(&app.viewer, [width as f32 - 4., 4.]);
            let cancelled = ui.button(&app.viewer, false);
            ensure!(cancelled.is_empty(), "drag-out leaked input: {cancelled:?}");
            rows.push(json!({"size":[width,height],"icon":id,"point":point,"activation":expected,"drag_out_cancelled":true}));
        }
        // Disabled Build mode must neither activate nor pick through to the lot.
        let (w, h, _) = app.viewer.icons.as_ref().unwrap().hud(2003).unwrap();
        let point = [
            117. + *w as f32 / 2.,
            height as f32 - 183. + 45. + *h as f32 / 2.,
        ];
        ui.move_to(&app.viewer, point);
        ensure!(ui.button(&app.viewer, true).is_empty());
        ensure!(ui.button(&app.viewer, false).is_empty());
        ui.save(&app.viewer, &out.join(format!("hud-{width}.png")))?;
        // Dispatch the actual options button result and test a real open panel.
        let (w, h, _) = app.viewer.icons.as_ref().unwrap().hud(2009).unwrap();
        let point = [
            193. + *w as f32 / 2.,
            height as f32 - 183. + 125. + *h as f32 / 2.,
        ];
        for m in click_messages(&mut ui, &app.viewer, point)? {
            app.dispatch(m)?;
        }
        ensure!(app.viewer.options_open);
        ensure!(app.observe()?.tick == before.tick && app.observe()?.rng == before.rng);
        ui.save(&app.viewer, &out.join(format!("options-{width}.png")))?;
        for m in click_messages(&mut ui, &app.viewer, point)? {
            app.dispatch(m)?;
        }
        ensure!(!app.viewer.options_open);
        // Find rendered nodes, not a second copy of HUD coordinates. Dispatch
        // only the expected message, then wait for its ordinary native owners.
        for expected in [
            "Resident(196)",
            "Needs",
            "NativeSpeed(924)",
            "NativeSpeed(760)",
            "Buy",
            "Live",
        ] {
            let mut found = None;
            for point in ui.centers(&app.viewer) {
                ui.move_to(&app.viewer, point);
                let _ = ui.button(&app.viewer, true);
                let messages = ui.button(&app.viewer, false);
                if messages.len() == 1 && format!("{:?}", messages[0]) == expected {
                    found = Some((point, messages));
                    break;
                }
            }
            let (point, messages) = found.with_context(|| {
                format!("rendered {expected} control absent at {width}x{height}")
            })?;
            let tick = app.observe()?.tick;
            for m in messages {
                app.dispatch(m)?;
            }
            app.drain(&Limits::default())?;
            ensure!(
                app.observe()?.tick == tick,
                "paused HUD action advanced gameplay"
            );
            rows.push(json!({"size":[width,height],"activation":expected,"point":point,"dispatched":true}));
        }
    }
    select(&mut app, 196)?;
    app.execute(
        &Command::Object {
            identity: Identity::get(app.session()?, 140)?,
        },
        &Limits::default(),
    )?;
    if app.menu().is_none() {
        std::fs::write(out.join("missing-menu.json"), app.checkpoint()?)?;
        anyhow::bail!("roach menu did not open after mode changes; retained state preserved");
    }
    let menu_tick = app.observe()?.tick;
    app.execute(
        &Command::Resize {
            width: 1000,
            height: 740,
        },
        &Limits::default(),
    )?;
    ui.save(&app.viewer, &out.join("resized-menu.png"))?;
    let mut cancelled = false;
    for point in ui.centers(&app.viewer) {
        ui.move_to(&app.viewer, point);
        let _ = ui.button(&app.viewer, true);
        let messages = ui.button(&app.viewer, false);
        if messages.len() == 1 && matches!(messages[0], Message::ObjectCancel) {
            for message in messages {
                app.dispatch(message)?;
            }
            app.drain(&Limits::default())?;
            cancelled = true;
            break;
        }
    }
    ensure!(
        cancelled && app.menu().is_none(),
        "resized menu could not be cancelled through Iced"
    );
    ensure!(
        app.observe()?.tick == menu_tick,
        "menu resize advanced gameplay"
    );
    write_json(
        &out.join("report.json"),
        &json!({"cases":rows,"sizes":3,"render_state_neutral":true,"real_widget_tree":true,"resized_menu_cancelled":true}),
    )?;
    Ok(())
}

fn animations(app: &Application) -> Vec<String> {
    app.viewer.session.as_ref().unwrap().simulation.characters[&196]
        .character
        .skin
        .animator
        .layers()
        .iter()
        .map(|l| l.layer.name.clone())
        .collect()
}
fn percentile(values: &[f64], percent: usize) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[(sorted.len() - 1) * percent / 100]
}
fn stats(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!({"count":0});
    }
    json!({"count":values.len(),"median_ms":percentile(values,50),"p95_ms":percentile(values,95),"max_ms":values.iter().copied().fold(0.,f64::max)})
}
#[test]
#[ignore = "private independent Mashuga checkpoint; bounded real spray sequence and headless frame profile"]
fn spray_sequence_frame_profile() -> Result<()> {
    run_worker(spray_profile)
}
fn spray_profile() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    let mut ui = Input::new()?;
    select(&mut app, 196)?;
    let actor = Identity::get(app.session()?, 196)?;
    if app
        .session()?
        .simulation
        .vm
        .world
        .actions
        .as_ref()
        .is_some_and(|a| {
            a.actors
                .get(&196)
                .is_some_and(|a| a.active.is_some() || !a.queue.pending.is_empty())
        })
    {
        app.execute(&Command::CancelAll { actor }, &Limits::default())?;
    }
    app.execute(
        &Command::Object {
            identity: Identity::get(app.session()?, 140)?,
        },
        &Limits::default(),
    )?;
    let action = app
        .menu()
        .context("roach menu absent")?
        .actions()
        .first()
        .context("spray action absent")?
        .clone();
    ensure!(
        action.target == 140 && action.menu_id == 2,
        "unexpected roach action"
    );
    app.execute(
        &Command::Choice {
            index: 0,
            expected: action.clone(),
        },
        &Limits::default(),
    )?;
    app.execute(&Command::Playing { value: true }, &Limits::default())?;
    let start_tick = app.observe()?.tick;
    let mut rows = vec![];
    let mut spray_seen = false;
    let mut spray_end = false;
    let mut checkpoint = None;
    let mut reference = None;
    let mut previous_textures = None;
    let wall = Instant::now();
    for frame in 0..10000 {
        ensure!(wall.elapsed().as_secs() < 240, "spray workload wall limit");
        app.viewer.frame_profile = Some(FrameProfile::default());
        let begin = Instant::now();
        app.millis += 40;
        let _ = app.viewer.update(Message::Frame(
            app.epoch + Duration::from_millis(app.millis),
        ));
        let update_ms = begin.elapsed().as_secs_f64() * 1000.;
        let stages = app.viewer.frame_profile.take();
        app.check_errors()?;
        let names = animations(&app);
        let spraying = names
            .iter()
            .any(|n| n.to_ascii_lowercase().contains("spray"));
        let draw_ms = if spraying || frame % 16 == 0 {
            let start = Instant::now();
            let _ = ui.pixels(&app.viewer)?;
            Some(start.elapsed().as_secs_f64() * 1000.)
        } else {
            None
        };
        let s = app.session()?;
        let tick = s.simulation.vm.tick;
        let scene = app.viewer.scene.as_ref().unwrap();
        let textures = Arc::as_ptr(&scene.textures) as usize;
        rows.push(json!({"frame":frame,"tick":tick,"update_ms":update_ms,"stages":stages,"draw_readback_ms":draw_ms,"animations":names,"spraying":spraying,"vertices":scene.vertices.len(),"textures":scene.textures.len(),"texture_owner_changed":previous_textures != Some(textures),"censors":scene.censors.len(),"objects":s.simulation.vm.world.objects.len()}));
        previous_textures = Some(textures);
        if spraying && !spray_seen {
            spray_seen = true;
            checkpoint = Some(app.checkpoint()?);
            std::fs::write(out.join("spraying.json"), checkpoint.as_ref().unwrap())?;
            let frozen = app.observe()?;
            ui.save(&app.viewer, &out.join("spraying.png"))?;
            let mut fresh = Input::new()?;
            ensure!(
                ui.pixels(&app.viewer)? == fresh.pixels(&app.viewer)?,
                "cached spray frame differs from fresh GPU resources"
            );
            ensure!(app.observe()? == frozen, "spray render changed state");
        }
        if spray_seen && !spraying {
            spray_end = true;
            reference = Some(app.observe()?);
            break;
        }
        if frame % 250 == 0 {
            write_json(&out.join("progress.json"), &rows)?;
        }
        ensure!(tick - start_tick < 4000, "spray animation not reached");
    }
    write_json(&out.join("frames.json"), &rows)?;
    ensure!(spray_seen && spray_end, "spray start/end not observed");
    // Cold resume is an untimed correctness run through the same remaining
    // virtual frames. It must return to the exact original observed state.
    let mut cold = app.restore(checkpoint.as_ref().unwrap())?;
    let remaining = app.millis - cold.millis;
    for _ in 0..remaining / 40 {
        cold.millis += 40;
        cold.dispatch(Message::Frame(
            cold.epoch + Duration::from_millis(cold.millis),
        ))?;
    }
    ensure!(
        Some(cold.observe()?) == reference,
        "spray cold resume diverged"
    );
    let frozen = app.observe()?;
    let mut draw = vec![];
    for _ in 0..12 {
        let start = Instant::now();
        let _ = ui.pixels(&app.viewer)?;
        draw.push(start.elapsed().as_secs_f64() * 1000.);
    }
    ensure!(
        app.observe()? == frozen,
        "render benchmark changed retained state"
    );
    let spray: Vec<_> = rows
        .iter()
        .filter(|r| r["spraying"] == true)
        .map(|r| r["update_ms"].as_f64().unwrap())
        .collect();
    let other: Vec<_> = rows
        .iter()
        .filter(|r| r["spraying"] != true)
        .map(|r| r["update_ms"].as_f64().unwrap())
        .collect();
    write_json(
        &out.join("report.json"),
        &json!({"spray_update":stats(&spray),"other_update":stats(&other),"draw_readback":stats(&draw),"start_tick":start_tick,"end_tick":app.observe()?.tick,"cold_resume_equal":true,"action":action,"strict_native_parity_ticks_added":0}),
    )?;
    Ok(())
}
