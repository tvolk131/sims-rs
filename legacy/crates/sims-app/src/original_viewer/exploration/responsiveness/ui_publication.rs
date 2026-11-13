//! Real-checkpoint equivalence for the complete detached frontend.
//! No native guest, player save, synthetic engine input or benchmark claim.
use super::super::super::ui_publication::{Published, view::Frontend};
use super::*;

fn frontend(v: &Viewer) -> Frontend {
    Frontend {
        data: Arc::new(Published::capture(v)),
        diagnostics: v.diagnostics,
        options_open: v.options_open,
        widget_epoch: v.widget_epoch,
    }
}

fn compare(app: &Application, out: &Path, name: &str, rows: &mut Vec<Value>) -> Result<()> {
    let before = engine::digest_serialized(&app.capture(false)?)?;
    let state = frontend(&app.viewer);
    let mut original = Input::new()?;
    let mut published = Input::new()?;
    let size = app.viewer.display_size;
    let icons = app.viewer.icons.as_ref().context("icons absent")?;
    let (old_pixels, new_pixels) = icons.with_view_caches_held(|| -> Result<_> {
        let old = original.batch_pixels(&app.viewer)?;
        let new = published.element_pixels(state.view(), size, 1.)?.0;
        Ok((old, new))
    })?;
    if old_pixels != new_pixels {
        for (kind, pixels) in [("old", &old_pixels), ("published", &new_pixels)] {
            image::save_buffer(
                out.join(format!("{name}-{kind}-failure.png")),
                pixels,
                size[0],
                size[1],
                image::ColorType::Rgba8,
            )?;
        }
        let n = old_pixels
            .iter()
            .zip(&new_pixels)
            .filter(|(a, b)| a != b)
            .count();
        anyhow::bail!("{name}: detached frontend differs in {n} RGBA bytes");
    }
    image::save_buffer(
        out.join(format!("{name}.png")),
        &new_pixels,
        size[0],
        size[1],
        image::ColorType::Rgba8,
    )?;
    let points = original.batch_centers(&app.viewer);
    let mut event_count = 0;
    for point in &points {
        original.reset();
        published.reset();
        for event in [
            iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: iced::Point::new(point[0], point[1]),
            }),
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                iced::mouse::Button::Left,
            )),
        ] {
            let (old, new) = icons.with_view_caches_held(|| {
                (
                    original.application_batch(&app.viewer, std::slice::from_ref(&event)),
                    published.element_batch(state.view(), size, std::slice::from_ref(&event)),
                )
            });
            ensure!(
                format!("{:?}", old.messages) == format!("{:?}", new.messages),
                "{name}: message mismatch at {point:?}: {:?} vs {:?}",
                old.messages,
                new.messages
            );
            ensure!(
                old.statuses == new.statuses,
                "{name}: event status mismatch at {point:?}"
            );
            ensure!(
                old.owners == new.owners,
                "{name}: widget ownership mismatch at {point:?}"
            );
            event_count += 1;
        }
    }
    let after = engine::digest_serialized(&app.capture(false)?)?;
    ensure!(
        before == after,
        "{name}: capture/view/layout/events/draw changed complete app state"
    );
    rows.push(json!({"name":name,"display_size":size,"rgba_equal":true,
        "rgba_bytes":old_pixels.len(),"event_comparisons":event_count,"points":points.len(),
        "complete_state_unchanged":true,"producer_caches_held":true,
        "pending":app.viewer.presentation_pending(),"selected_page_products":state.catalog.as_ref().and_then(|c|c.view()).map(|v|v.products.len())}));
    write_json(&out.join("comparisons.json"), rows)?;
    Ok(())
}

#[test]
#[ignore = "private independent Mashuga checkpoint; complete immutable Iced frontend headless comparison"]
fn mashuga_complete_ui_publication_matches_original_frontend() -> Result<()> {
    run_worker(campaign)
}

fn campaign() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    select(&mut app, 196)?;
    let mut rows = vec![];
    compare(&app, &out, "live", &mut rows)?;
    app.dispatch(Message::Options)?;
    compare(&app, &out, "options", &mut rows)?;
    app.dispatch(Message::Options)?;
    app.dispatch(Message::Diagnostics)?;
    compare(&app, &out, "diagnostics", &mut rows)?;
    app.dispatch(Message::Diagnostics)?;
    app.dispatch(Message::Buy)?;
    app.drain(&Limits::default())?;
    compare(&app, &out, "buy-root", &mut rows)?;
    let catalog = app
        .viewer
        .device
        .as_ref()
        .and_then(|d| d.panel_widgets())
        .and_then(|w| w.catalog())
        .context("catalog absent")?;
    let index = catalog
        .view()
        .context("catalog view absent")?
        .categories
        .iter()
        .position(|c| c.visible && c.enabled)
        .context("available category absent")?;
    let revision = catalog.revision();
    app.dispatch(Message::BuyNavigate(
        revision,
        sims_vm::interface::buy::input::Event::Category(index as u8),
    ))?;
    app.drain(&Limits::default())?;
    compare(&app, &out, "buy-products", &mut rows)?;
    app.dispatch(Message::Live)?;
    ensure!(
        app.viewer.presentation_pending(),
        "real pending Live boundary absent"
    );
    compare(&app, &out, "pending-live", &mut rows)?;
    let checkpoint = app.checkpoint()?;
    let mut cold = app.restore(&checkpoint)?;
    compare(&cold, &out, "cold-pending-live", &mut rows)?;
    app.drain(&Limits::default())?;
    cold.drain(&Limits::default())?;
    ensure!(
        app.observe()? == cold.observe()?,
        "pending cold continuation diverges"
    );
    compare(&app, &out, "live-return", &mut rows)?;
    app.execute(
        &Command::Resize {
            width: 800,
            height: 600,
        },
        &Limits::default(),
    )?;
    compare(&app, &out, "live-800x600", &mut rows)?;
    write_json(
        &out.join("result.json"),
        &json!({"passed":true,"cases":rows,
        "strict_gameplay_parity_ticks":0,"window_latency_benchmark":false,
        "qualification":"Complete detached frontend render/input equality and passive state neutrality on one independent research checkpoint; no worker integration or gameplay parity claim."}),
    )
}

#[test]
#[ignore = "private Mashuga checkpoint; persistent production owner versus unchanged inline handlers and cold Save/Load"]
fn mashuga_worker_owner_preserves_all_mode_boundaries_and_managed_save_load() -> Result<()> {
    run_worker(owner_campaign)
}

fn owner_compare(
    reference: &Application,
    host: &super::super::super::worker_host::Host,
    rows: &mut Vec<Value>,
    label: &str,
) -> Result<()> {
    let expected = reference.capture(false)?;
    let actual = host.audit_for_test()?;
    if expected != actual {
        let keys: Vec<_> = ["session", "dialog_client", "camera_device", "application"]
            .into_iter()
            .filter(|k| expected[*k] != actual[*k])
            .collect();
        anyhow::bail!("{label}: worker owner differs in {keys:?}");
    }
    rows.push(json!({"label":label,"complete_state_equal":true,
        "pending":reference.viewer.presentation_pending(),"generation":host.current_generation_for_test(),
        "state_sha256":engine::digest_serialized(&expected)?}));
    Ok(())
}
fn owner_call(
    reference: &mut Application,
    host: &mut super::super::super::worker_host::Host,
    rows: &mut Vec<Value>,
    message: Message,
) -> Result<()> {
    use super::super::super::worker_host::Event;
    let label = format!("{message:?}");
    let inline = reference.viewer.update(message.clone());
    ensure!(
        inline.units() == 0,
        "test refuses unhandled inline task effects"
    );
    let task = host.update(Event::Widget {
        generation: host.current_generation_for_test(),
        message,
    });
    ensure!(
        task.units() <= 1,
        "test refuses unhandled shell task effects beyond the worker wake"
    );
    let effects = host.settle_for_test()?;
    ensure!(
        effects.is_empty(),
        "test refuses unhandled completed task effects"
    );
    owner_compare(reference, host, rows, &label)
}
fn owner_finish(
    reference: &mut Application,
    host: &mut super::super::super::worker_host::Host,
    rows: &mut Vec<Value>,
) -> Result<()> {
    let now = reference.epoch + Duration::from_millis(reference.millis);
    for _ in 0..Limits::default().service_polls {
        if !reference.viewer.presentation_pending() {
            return Ok(());
        }
        owner_call(reference, host, rows, Message::Frame(now))?;
    }
    anyhow::bail!("retained owner mode caller did not finish")
}
fn owner_campaign() -> Result<()> {
    use super::super::super::worker_host::Host;
    let out = output()?;
    let mut reference = load(&out)?;
    select(&mut reference, 196)?;
    let checkpoint = reference.checkpoint()?;
    let mut cold = reference.restore(&checkpoint)?;
    let mut host = Host::audited_for_test(cold.viewer, cold.epoch, cold.millis as u32);
    let mut rows = vec![];
    owner_compare(&reference, &host, &mut rows, "initial")?;
    for message in [Message::Buy, Message::Live, Message::Buy, Message::Live] {
        owner_call(&mut reference, &mut host, &mut rows, message.clone())?;
        if matches!(message, Message::Live) && reference.viewer.presentation_pending() {
            // Save at an actual suspended Live owner, then cold-restore it
            // through the unchanged managed Session/client/device adapters.
            let checkpoint = reference.checkpoint()?;
            cold.viewer = host.recover_for_test();
            ensure!(reference.capture(false)? == cold.capture(false)?);
            cold = reference.restore(&checkpoint)?;
            host = Host::audited_for_test(cold.viewer, cold.epoch, cold.millis as u32);
            owner_compare(&reference, &host, &mut rows, "managed-cold-pending-Live")?;
        }
        owner_finish(&mut reference, &mut host, &mut rows)?;
        owner_call(&mut reference, &mut host, &mut rows, Message::Options)?;
        owner_call(&mut reference, &mut host, &mut rows, Message::Options)?;
        write_json(&out.join("owner-comparisons.json"), &rows)?;
    }
    owner_call(&mut reference, &mut host, &mut rows, Message::Save)?;
    let save = reference.viewer.args.save.clone();
    ensure!(
        save.starts_with(&out),
        "research save escaped private output"
    );
    owner_call(&mut reference, &mut host, &mut rows, Message::Load)?;
    owner_finish(&mut reference, &mut host, &mut rows)?;
    cold.viewer = host.recover_for_test();
    ensure!(reference.capture(false)? == cold.capture(false)?);
    write_json(
        &out.join("owner-result.json"),
        &json!({"passed":true,"boundaries":rows,
        "pending_managed_cold_restore":true,"actual_private_player_save_load":true,
        "strict_original_parity_ticks":0,"scope":"Persistent worker versus unchanged actual application handlers; no native engine or latency/FPS claim."}),
    )
}

#[test]
#[ignore = "private Mashuga checkpoint; actual production Host render while its real owner is blocked, headless Metal"]
fn mashuga_production_host_renders_while_actual_owner_is_blocked() -> Result<()> {
    run_worker(blocked_host_campaign)
}
fn blocked_host_campaign() -> Result<()> {
    use super::super::super::worker_host::{Event, Host};
    let out = output()?;
    let mut reference = load(&out)?;
    select(&mut reference, 196)?;
    let before = engine::digest_serialized(&reference.capture(false)?)?;
    let checkpoint = reference.checkpoint()?;
    let mut cold = reference.restore(&checkpoint)?;
    let mut host = Host::from_viewer_for_test(cold.viewer);
    let initial = host.publication_for_test().clone();
    let (entered, resume) = host.gate_next_for_test();
    let now = reference.epoch + Duration::from_millis(reference.millis);
    let task = host.update(Event::Widget {
        generation: 0,
        message: Message::Frame(now),
    });
    ensure!(task.units() == 1, "missing single worker completion wake");
    entered.recv_timeout(Duration::from_secs(5))?;
    let root = reference.viewer.args.root.clone();
    let size = reference.viewer.display_size;
    let icons = reference
        .viewer
        .icons
        .as_ref()
        .context("icons absent")?
        .clone();
    let mut rows = vec![];
    for (name, toggle_options) in [
        ("busy-live", false),
        ("busy-options", true),
        ("busy-live-return", true),
    ] {
        if toggle_options {
            ensure!(reference.viewer.update(Message::Options).units() == 0);
            ensure!(
                host.update(Event::Widget {
                    generation: 0,
                    message: Message::Options
                })
                .units()
                    == 0
            );
        }
        for _ in 0..20 {
            ensure!(host.update(Event::Frame(now)).units() == 0);
        }
        let mut frame_handlers = vec![];
        let mut options_handlers = vec![];
        let mut views = vec![];
        for _ in 0..17 {
            let started = Instant::now();
            for _ in 0..128 {
                ensure!(host.update(Event::Frame(now)).units() == 0);
            }
            frame_handlers.push(started.elapsed().as_secs_f64() * 1e6 / 128.);
            for _ in 0..2 {
                let started = Instant::now();
                ensure!(host.update(Event::Widget {
                    generation: 0,
                    message: Message::Options,
                }).units() == 0);
                options_handlers.push(started.elapsed().as_secs_f64() * 1e6);
            }
            let started = Instant::now();
            for _ in 0..128 {
                std::hint::black_box(host.view());
            }
            views.push(started.elapsed().as_secs_f64() * 1e6 / 128.);
        }
        let expected = out.join(format!("{name}-inline.png"));
        let actual = out.join(format!("{name}-worker.png"));
        icons.with_view_caches_held(|| -> Result<()> {
            crate::support::render_headless_element_at_size(
                &root,
                &expected,
                reference.viewer.view(),
                size,
            )?;
            crate::support::render_headless_element_at_size(&root, &actual, host.view(), size)
        })?;
        ensure!(
            image::open(&expected)?.to_rgba8() == image::open(&actual)?.to_rgba8(),
            "{name}: actual Host pixels differ"
        );
        ensure!(
            Arc::ptr_eq(&initial, host.publication_for_test()),
            "painting consumed/changed owner publication"
        );
        rows.push(
            json!({"case":name,"rgba_equal":true,"owner_explicitly_blocked":true,
            "publication_unchanged":true,"producer_caches_held":true,
            "extra_shell_frames":20 + 17 * 128,"extra_options_toggles":34,
            "shell_frame_handler":presentation_latency::distribution(frame_handlers),
            "shell_options_handler":presentation_latency::distribution(options_handlers),
            "host_view_construct_and_drop":presentation_latency::distribution(views)}),
        );
    }
    ensure!(
        before == engine::digest_serialized(&reference.capture(false)?)?,
        "passive original frontend changed complete state"
    );
    resume.send(())?;
    ensure!(host.settle_for_test()?.is_empty(), "unhandled task effect");
    ensure!(reference.viewer.update(Message::Frame(now)).units() == 0);
    cold.viewer = host.recover_for_test();
    ensure!(
        reference.capture(false)? == cold.capture(false)?,
        "extra painting changed engine calls/state/RNG"
    );
    write_json(
        &out.join("blocked-host-result.json"),
        &json!({"passed":true,"cases":rows,
        "complete_state_equal_after_release":true,"strict_original_parity_ticks":0,
        "profile":if cfg!(debug_assertions){"debug"}else{"release"},
        "scope":"Actual production Host shell dispatch and immutable view construction while its exact Viewer is explicitly gated. Options/pixels remain equal; extra paints never create VM visits. No compositor/window latency, FPS, active-play or fresh-load claim."}),
    )
}

#[test]
#[ignore = "private Mashuga checkpoint; accepted production Save and Close, completion task only, no GUI or frame pulses"]
fn mashuga_accepted_save_closes_from_one_completion_without_frame_pulses() -> Result<()> {
    run_worker(no_pulse_save_campaign)
}
fn no_pulse_save_campaign() -> Result<()> {
    use super::super::super::worker_host::{Event, Host};
    use iced::futures::StreamExt;
    let out = output()?;
    let mut reference = load(&out)?;
    select(&mut reference, 196)?;
    let checkpoint = reference.checkpoint()?;
    let mut cold = reference.restore(&checkpoint)?;
    let mut host = Host::from_viewer_for_test(cold.viewer);
    ensure!(reference.viewer.update(Message::Save).units() == 0);
    let save = reference.viewer.args.save.clone();
    ensure!(save.starts_with(&out));
    let expected = std::fs::read(&save)?;
    std::fs::remove_file(&save)?; // This test's own output, never a player save.
    let (entered, resume) = host.gate_next_for_test();
    let task = host.update(Event::Widget {
        generation: 0,
        message: Message::Save,
    });
    ensure!(task.units() == 1, "missing single completion wake");
    entered.recv_timeout(Duration::from_secs(5))?;
    let id = window::Id::unique();
    ensure!(
        host.update(Event::Close(id)).units() == 0,
        "closed before accepted Save completed"
    );
    ensure!(!save.exists(), "Save ran before explicit release");
    resume.send(())?;
    // No more frame, window, input or polling calls: the actual Iced task is
    // the sole wake source, and the broker remains completion authority.
    let mut stream = iced_runtime::task::into_stream(task).context("wake stream absent")?;
    let event = iced::futures::executor::block_on(stream.next()).context("wake absent")?;
    ensure!(matches!(
        event,
        iced_runtime::Action::Output(Event::Completion)
    ));
    let close = host.update(Event::Completion);
    let mut stream = iced_runtime::task::into_stream(close).context("close stream absent")?;
    let action = iced::futures::executor::block_on(stream.next()).context("close absent")?;
    ensure!(
        matches!(action,iced_runtime::Action::Window(iced_runtime::window::Action::Close(actual)) if actual==id)
    );
    ensure!(
        std::fs::read(&save)? == expected,
        "accepted Save bytes differ"
    );
    cold.viewer = host.recover_for_test();
    ensure!(
        reference.capture(false)? == cold.capture(false)?,
        "Save/Close changed complete state/RNG/continuations"
    );
    write_json(
        &out.join("no-pulse-save-result.json"),
        &json!({"passed":true,
        "successful_private_save_bytes_equal":true,"complete_state_equal":true,
        "extra_window_input_or_frame_pulses":0,"completion_wakes":1,"close_effects":1,
        "save_sha256":engine::digest(&expected),"save_bytes":expected.len(),
        "strict_original_parity_ticks":0,"scope":"Actual production accepted Save and ordered Close driven solely by one completion notification; no GUI, player save or original engine used."}),
    )
}
