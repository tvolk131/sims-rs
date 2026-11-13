//! The actual Iced runtime can update many events before application messages
//! run. Do not insert a Viewer update/view diff between focus loss and release.
use super::*;
use modal_lifetimes::pointer;

fn names(messages: &[Message]) -> Vec<String> {
    messages.iter().map(|m| format!("{m:?}")).collect()
}
fn apply(app: &mut Application, messages: Vec<Message>) -> Result<()> {
    for message in messages {
        app.dispatch(message)?;
    }
    Ok(())
}

#[test]
#[ignore = "private Car Pool modal; actual iced_runtime UserInterface event batches and headless GPU"]
fn actual_runtime_batched_focus_loss_cancels_held_modal_press() -> Result<()> {
    run_worker(modal_batches)
}
fn modal_batches() -> Result<()> {
    let out = output()?;
    let base = modal_lifetimes::load_car_pool(&out)?;
    let mut ui = Input::new()?;
    let mut rows = vec![];
    let mut failures = vec![];
    for regain in [false, true] {
        let mut app = base.fork()?;
        let point = modal_lifetimes::ok_point(&mut ui, &app)?;
        let before = app.observe()?;
        let viewport = ui.viewport_bounds(&app.viewer)?;
        let pressed = ui.application_batch(
            &app.viewer,
            &[
                iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                    position: iced::Point::new(point[0], point[1]),
                }),
                pointer(true),
            ],
        );
        ensure!(
            pressed.messages.iter().all(|m| matches!(
                m,
                Message::ViewportEnter | Message::ViewportLeave | Message::ViewportMotion { .. }
            )),
            "modal press activated: {:?}",
            pressed.messages
        );
        apply(&mut app, pressed.messages.clone())?;
        ensure!(
            app.observe()? == before,
            "passive cursor setup changed retained state/RNG"
        );
        let mut events = vec![iced::Event::Window(window::Event::Unfocused)];
        if regain {
            events.push(iced::Event::Window(window::Event::Focused));
        }
        events.push(pointer(false));
        let batch = ui.application_batch(&app.viewer, &events);
        ensure!(
            app.observe()? == before,
            "UserInterface widget processing mutated retained application state"
        );
        let had_activation = batch
            .messages
            .iter()
            .any(|m| !matches!(m, Message::FocusLost));
        let row = json!({"regain_focus_in_batch":regain,"events":events.iter().map(|e|format!("{e:?}")).collect::<Vec<_>>(),
            "statuses":batch.statuses.iter().map(|s|format!("{s:?}")).collect::<Vec<_>>(),
            "messages_before_any_viewer_update":names(&batch.messages),"before":before,"widget_owners_before":pressed.owners,"widget_owners_after":batch.owners});
        write_json(&out.join(format!("batch-before-apply-{regain}.json")), &row)?;
        apply(&mut app, batch.messages)?;
        let after = app.observe()?;
        let passed = !had_activation
            && after == before
            && batch.owners == pressed.owners
            && ui.viewport_bounds(&app.viewer)? == viewport;
        if !passed {
            failures.push(format!("regain-{regain}"));
            std::fs::write(
                out.join(format!("failure-{regain}.json")),
                app.checkpoint()?,
            )?;
        } else {
            let mut deliberate = vec![];
            if !regain {
                deliberate.push(iced::Event::Window(window::Event::Focused));
            }
            deliberate.extend([pointer(true), pointer(false)]);
            let fresh = ui.application_batch(&app.viewer, &deliberate);
            ensure!(
                matches!(fresh.messages.as_slice(), [Message::Dialog(149, 2)]),
                "fresh deliberate batch click failed: {:?}",
                fresh.messages
            );
            apply(&mut app, fresh.messages)?;
            app.drain(&Limits::default())?;
            ensure!(app.session()?.simulation.vm.world.dialogs.modal != Some(149));
        }
        rows.push(json!({"batch":row,"after_delayed_viewer_updates":after,"passed":passed,"fresh_deliberate_click_works":passed,"viewport":viewport}));
    }
    // A new press can begin after regaining focus but before the subscription
    // FocusLost message is applied. A delayed epoch fallback must not cancel
    // this newer press a second time when the next batch releases it.
    let mut app = base.fork()?;
    let point = modal_lifetimes::ok_point(&mut ui, &app)?;
    let before = app.observe()?;
    let pressed = ui.application_batch(
        &app.viewer,
        &[
            iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: iced::Point::new(point[0], point[1]),
            }),
            pointer(true),
        ],
    );
    apply(&mut app, pressed.messages)?;
    ensure!(app.observe()? == before);
    let batch = ui.application_batch(
        &app.viewer,
        &[
            iced::Event::Window(window::Event::Unfocused),
            pointer(false),
            iced::Event::Window(window::Event::Focused),
            pointer(true),
        ],
    );
    let events = names(&batch.messages);
    let no_old_activation = batch
        .messages
        .iter()
        .all(|m| matches!(m, Message::FocusLost));
    apply(&mut app, batch.messages)?;
    let next = ui.application_batch(&app.viewer, &[pointer(false)]);
    let newer = names(&next.messages);
    let passed = no_old_activation
        && app.observe()? == before
        && matches!(next.messages.as_slice(), [Message::Dialog(149, 2)])
        && next.owners == batch.owners;
    if !passed {
        failures.push("newer-held-press".into());
        std::fs::write(out.join("failure-newer-held-press.json"), app.checkpoint()?)?;
    } else {
        apply(&mut app, next.messages)?;
        app.drain(&Limits::default())?;
        ensure!(app.session()?.simulation.vm.world.dialogs.modal != Some(149));
    }
    rows.push(json!({"case":"newer press begins after Focused before delayed Viewer update",
        "focus_batch_messages":events,"next_release_messages":newer,"passed":passed,"before":before}));
    write_json(
        &out.join("report.json"),
        &json!({"cases":rows,"failures":failures,
        "actual_iced_runtime_update":true,"viewer_messages_delayed_until_entire_batch_finishes":true,
        "scope":"replacement-client batch event lifetime; zero original engine parity ticks"}),
    )?;
    ensure!(
        failures.is_empty(),
        "same-batch stale modal activations preserved: {failures:?}"
    );
    Ok(())
}

#[test]
#[ignore = "private Mashuga Buy catalogue; actual Iced batched focus, scroll and HUD events"]
fn actual_runtime_batched_buy_focus_preserves_scroll_and_input() -> Result<()> {
    run_worker(buy_batch)
}
fn buy_batch() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    let mut ui = Input::new()?;
    select(&mut app, 196)?;
    sequences::activate(&mut ui, &mut app, "Buy")?;
    let before = app.observe()?;
    let viewport = ui.viewport_bounds(&app.viewer)?;
    let initial = ui.application_batch(&app.viewer, &[]);
    let scroll = initial
        .owners
        .scrolls
        .iter()
        .find(|s| s.content[3] > s.bounds[3] + 1.)
        .context("Buy catalogue does not overflow")?;
    let point = iced::Point::new(
        scroll.bounds[0] + scroll.bounds[2] * 0.75,
        scroll.bounds[1] + scroll.bounds[3] * 0.5,
    );
    let moved = ui.application_batch(
        &app.viewer,
        &[
            iced::Event::Mouse(iced::mouse::Event::CursorMoved { position: point }),
            iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0., y: -3. },
            }),
        ],
    );
    apply(&mut app, moved.messages)?;
    ensure!(
        app.observe()? == before
            && moved.owners != initial.owners
            && moved.owners.scrolls.iter().any(|s| s.translation[1] > 0.)
    );
    let (point, _) = sequences::control(&mut ui, &app, "Options")?;
    let press = ui.application_batch(
        &app.viewer,
        &[
            iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: iced::Point::new(point[0], point[1]),
            }),
            pointer(true),
        ],
    );
    apply(&mut app, press.messages)?;
    ensure!(app.observe()? == before);
    let focused = ui.application_batch(
        &app.viewer,
        &[
            iced::Event::Window(window::Event::Unfocused),
            iced::Event::Window(window::Event::Focused),
            pointer(false),
        ],
    );
    write_json(
        &out.join("before-delayed-viewer-updates.json"),
        &json!({"initial":initial.owners,"scrolled":moved.owners,
        "after_focus_batch":focused.owners,"messages":names(&focused.messages)}),
    )?;
    let no_activation = focused
        .messages
        .iter()
        .all(|m| matches!(m, Message::FocusLost));
    apply(&mut app, focused.messages)?;
    ensure!(
        no_activation
            && app.observe()? == before
            && focused.owners == moved.owners
            && ui.viewport_bounds(&app.viewer)? == viewport,
        "batched focus reset scroll/geometry or activated stale HUD input"
    );
    let saved = app.capture(false)?;
    let fresh = ui.application_batch(&app.viewer, &[pointer(true), pointer(false)]);
    ensure!(matches!(fresh.messages.as_slice(), [Message::Options]));
    apply(&mut app, fresh.messages)?;
    let mut after = app.capture(false)?;
    after["application"]["options_open"] = saved["application"]["options_open"].clone();
    ensure!(
        app.viewer.options_open && after == saved,
        "fresh batch Options click changed retained owners unexpectedly"
    );
    write_json(
        &out.join("report.json"),
        &json!({"passed":true,"initial":initial.owners,"scrolled":moved.owners,
        "after_focus_batch":focused.owners,"viewport":viewport,"before":before,"fresh_click_changes_only_options_open":true,
        "scope":"actual Iced batch invariants, zero native engine parity ticks"}),
    )?;
    Ok(())
}
