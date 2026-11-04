//! Real popup/catalog widgets and their actual advanced tooltip overlays.
//! These are replacement-client lifetime invariants, not original UI parity.
use super::*;
use modal_lifetimes::pointer;

fn moved(point: [f32; 2]) -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position: iced::Point::new(point[0], point[1]),
    })
}
fn passive(message: &Message) -> bool {
    matches!(
        message,
        Message::FocusLost
            | Message::DisplaySize(_)
            | Message::ViewportEnter
            | Message::ViewportLeave
            | Message::ViewportMotion { .. }
            | Message::ViewportButton { down: false, .. }
    )
}
fn lifetime(message: &Message) -> bool {
    matches!(message, Message::FocusLost | Message::DisplaySize(_))
}
fn apply(app: &mut Application, messages: &[Message]) -> Result<()> {
    for message in messages {
        app.dispatch(message.clone())?;
    }
    Ok(())
}
fn runtime_control(
    ui: &mut Input,
    app: &Application,
    expected: &str,
) -> Result<([f32; 2], Message)> {
    let before = app.observe()?;
    for point in ui.batch_centers(&app.viewer) {
        let batch =
            ui.application_batch(&app.viewer, &[moved(point), pointer(true), pointer(false)]);
        let actual: Vec<_> = batch.messages.into_iter().filter(|m| !passive(m)).collect();
        if actual.len() == 1 && format!("{:?}", actual[0]) == expected {
            ensure!(
                app.observe()? == before,
                "runtime control discovery applied application input"
            );
            return Ok((point, actual[0].clone()));
        }
    }
    anyhow::bail!("runtime-cache {expected} control absent")
}
fn batch_record(
    events: &[iced::Event],
    batch: &super::super::super::headless_input::Batch,
) -> Value {
    json!({"events":events.iter().map(|e|format!("{e:?}")).collect::<Vec<_>>(),
        "statuses":batch.statuses.iter().map(|s|format!("{s:?}")).collect::<Vec<_>>(),
        "messages_before_app_updates":batch.messages.iter().map(|m|format!("{m:?}")).collect::<Vec<_>>(),
        "widget_owners":batch.owners})
}
fn hold(ui: &mut Input, app: &mut Application, expected: &str) -> Result<[f32; 2]> {
    let (point, _) = runtime_control(ui, app, expected)?;
    let before = app.observe()?;
    let batch = ui.application_batch(&app.viewer, &[moved(point), pointer(true)]);
    ensure!(
        batch.messages.iter().all(passive),
        "press activated {expected}: {:?}",
        batch.messages
    );
    apply(app, &batch.messages)?;
    ensure!(
        app.observe()? == before,
        "passive held press changed complete state/RNG"
    );
    Ok(point)
}
fn deliberate(ui: &mut Input, app: &mut Application, expected: &str) -> Result<()> {
    let (point, message) = runtime_control(ui, app, expected)?;
    let mut reference = app.fork()?;
    let batch = ui.application_batch(&app.viewer, &[moved(point), pointer(true), pointer(false)]);
    let actual: Vec<_> = batch.messages.iter().filter(|m| !passive(m)).collect();
    ensure!(
        actual.len() == 1 && format!("{:?}", actual[0]) == expected,
        "deliberate click did not emit exactly one {expected}: {:?}",
        batch.messages
    );
    apply(app, &batch.messages)?;
    reference.dispatch(message)?;
    app.drain(&Limits::default())?;
    reference.drain(&Limits::default())?;
    ensure!(
        app.observe()? == reference.observe()?,
        "actual click differs from one intentional handler call"
    );
    Ok(())
}
fn cold_equal(app: &Application) -> Result<()> {
    let cold = app.restore(&app.checkpoint()?)?;
    ensure!(
        app.observe()? == cold.observe()?,
        "cold restore changed complete state/RNG"
    );
    Ok(())
}

#[test]
#[ignore = "private Mashuga checkpoint; real popup/pending collectors and actual Iced event batches"]
fn actual_runtime_object_popup_focus_resize_and_pending_batches() -> Result<()> {
    run_worker(object_batches)
}
fn object_batches() -> Result<()> {
    let out = output()?;
    let mut base = load(&out)?;
    select(&mut base, 196)?;
    base.execute(
        &Command::Resize {
            width: 800,
            height: 600,
        },
        &Limits::default(),
    )?;
    let tick = base.observe()?.tick;
    base.execute(
        &Command::BeginObject {
            identity: Identity::get(base.session()?, 140)?,
        },
        &Limits::default(),
    )?;
    ensure!(base.session()?.object_input_pending() && base.menu().is_none());
    std::fs::write(out.join("pending.json"), base.checkpoint()?)?;
    let mut cold = base.restore(&base.checkpoint()?)?;
    let mut ui = Input::new()?;
    let events = [
        iced::Event::Window(window::Event::Unfocused),
        iced::Event::Window(window::Event::Focused),
        iced::Event::Window(window::Event::Resized(Size::new(1000., 740.))),
        pointer(false),
    ];
    let pending_before = base.observe()?;
    let batch = ui.application_batch(&base.viewer, &events);
    ensure!(base.observe()? == pending_before && batch.messages.iter().all(passive));
    let pending_row = batch_record(&events, &batch);
    apply(&mut base, &batch.messages)?;
    apply(
        &mut cold,
        &batch
            .messages
            .iter()
            .filter(|m| lifetime(m))
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    ensure!(base.observe()? == cold.observe()? && base.menu().is_none());
    base.drain(&Limits::default())?;
    cold.drain(&Limits::default())?;
    ensure!(
        base.observe()? == cold.observe()? && base.menu().is_some_and(|m| !m.actions().is_empty())
    );
    ensure!(base.observe()?.tick == tick);
    std::fs::write(out.join("ready.json"), base.checkpoint()?)?;
    let mut rows = vec![
        json!({"case":"pending collector delayed focus/resize/release","batch":pending_row,"cold_equal":true}),
    ];
    let mut failures = vec![];
    for name in [
        "focus-release",
        "new-press-before-delayed-focus-message",
        "resize-release",
        "new-press-after-resize",
    ] {
        let mut app = base.fork()?;
        let mut ui = Input::new()?;
        let point = hold(&mut ui, &mut app, "ObjectChoice(0)")?;
        let before = app.observe()?;
        let bytes = app.checkpoint()?;
        let mut reference = app.restore(&bytes)?;
        let viewport = ui.viewport_bounds(&app.viewer)?;
        let events = match name {
            "focus-release" => vec![
                iced::Event::Window(window::Event::Unfocused),
                iced::Event::Window(window::Event::Focused),
                pointer(false),
            ],
            "new-press-before-delayed-focus-message" => vec![
                iced::Event::Window(window::Event::Unfocused),
                pointer(false),
                iced::Event::Window(window::Event::Focused),
                pointer(true),
            ],
            "new-press-after-resize" => vec![
                iced::Event::Window(window::Event::Resized(Size::new(1320., 900.))),
                pointer(false),
                pointer(true),
            ],
            _ => vec![
                iced::Event::Window(window::Event::Resized(Size::new(1320., 900.))),
                pointer(false),
            ],
        };
        let batch = ui.application_batch(&app.viewer, &events);
        ensure!(
            app.observe()? == before,
            "widget batch executed application work"
        );
        let raw = batch_record(&events, &batch);
        write_json(&out.join(format!("{name}-before-apply.json")), &raw)?;
        let leaked = batch.messages.iter().any(|m| !passive(m));
        apply(
            &mut reference,
            &batch
                .messages
                .iter()
                .filter(|m| lifetime(m))
                .cloned()
                .collect::<Vec<_>>(),
        )?;
        let applied = apply(&mut app, &batch.messages).map_err(|e| e.to_string());
        let passed = !leaked && applied.is_ok() && app.observe()? == reference.observe()?;
        if !passed {
            failures.push(name);
            std::fs::write(out.join(format!("{name}-failure-before.json")), bytes)?;
            std::fs::write(
                out.join(format!("{name}-failure-after.json")),
                app.checkpoint()?,
            )?;
        } else {
            if name.starts_with("new-press") {
                let (next_point, _) = sequences::control(&mut ui, &app, "ObjectChoice(0)")?;
                let next = ui.application_batch(&app.viewer, &[moved(next_point), pointer(false)]);
                ensure!(
                    next.messages.iter().filter(|m| !passive(m)).count() == 1
                        && next
                            .messages
                            .iter()
                            .any(|m| matches!(m, Message::ObjectChoice(0))),
                    "delayed focus/size message cancelled a newer intentional popup press: {:?}",
                    next.messages
                );
                apply(&mut app, &next.messages)?;
                reference.dispatch(Message::ObjectChoice(0))?;
                app.drain(&Limits::default())?;
                reference.drain(&Limits::default())?;
                ensure!(app.observe()? == reference.observe()?);
            } else {
                ensure!(ui.viewport_bounds(&app.viewer)? == ui.viewport_bounds(&reference.viewer)?);
                deliberate(&mut ui, &mut app, "ObjectCancel")?;
                ensure!(app.menu().is_none());
            }
            ensure!(app.viewer.resident == 196 && app.observe()?.tick == tick);
            cold_equal(&app)?;
        }
        rows.push(
            json!({"case":name,"point":point,"before":before,"batch":raw,
            "apply_error":applied.err(),"passed":passed,"viewport_before":viewport,
            "viewport_after":ui.viewport_bounds(&app.viewer)?}),
        );
        if name == "focus-release" && passed {
            let before_frame = app.observe()?;
            ui.batch_save(&app.viewer, &out.join("closed-after-deliberate-cancel.png"))?;
            ensure!(app.observe()? == before_frame);
        }
    }
    write_json(
        &out.join("report.json"),
        &json!({"cases":rows,"failures":failures,
        "scope":"actual production stack/opaque object popup, actual runtime batches; no native parity credit"}),
    )?;
    ensure!(
        failures.is_empty(),
        "popup event lifetime failures preserved: {failures:?}"
    );
    Ok(())
}

#[test]
#[ignore = "private Mashuga checkpoint; real Buy controls/tooltips/scroll and actual Iced event batches"]
fn actual_runtime_catalog_product_tooltip_focus_resize_and_scroll_batches() -> Result<()> {
    run_worker(catalog_batches)
}
fn catalog_batches() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    select(&mut app, 196)?;
    let mut ui = Input::new()?;
    sequences::activate(&mut ui, &mut app, "Buy")?;
    let root_viewport = ui.viewport_bounds(&app.viewer)?;
    let tick = app.observe()?.tick;
    let revision = app
        .viewer
        .device
        .as_ref()
        .unwrap()
        .panel_widgets()
        .unwrap()
        .catalog()
        .unwrap()
        .revision();
    deliberate(
        &mut ui,
        &mut app,
        &format!("BuyNavigate({revision}, Category(1))"),
    )?;
    let cat = app
        .viewer
        .device
        .as_ref()
        .unwrap()
        .panel_widgets()
        .unwrap()
        .catalog()
        .unwrap();
    let view = cat.view().unwrap();
    ensure!(view.heading == "Surfaces" && !view.products.is_empty() && view.next);
    let revision = cat.revision();
    let product = view.products[0];
    let product_message = format!("BuyProduct({revision}, {product})");
    let (point, _) = sequences::control(&mut ui, &app, &product_message)?;
    let before = app.observe()?;
    let viewport = ui.viewport_bounds(&app.viewer)?;
    let neutral_source = app.checkpoint()?;
    // Preserve the initial test's world-hover witness separately. Entering the
    // lot in Buy mode is an actual native move-tool input, not a presentation-
    // only way to hide a tooltip; do not call any resulting state change neutral.
    let baseline = ui.application_batch(&app.viewer, &[moved([0., 0.])]);
    apply(&mut app, &baseline.messages)?;
    let world_hover = app.observe()?;
    let hovered = ui.application_batch(&app.viewer, &[moved(point)]);
    apply(&mut app, &hovered.messages)?;
    write_json(
        &out.join("world-hover-witness.json"),
        &json!({
        "before":before,"after_world_hover":world_hover,"after_return_to_product":app.observe()?,
        "world_messages":baseline.messages.iter().map(|m|format!("{m:?}")).collect::<Vec<_>>(),
        "product_messages":hovered.messages.iter().map(|m|format!("{m:?}")).collect::<Vec<_>>(),
        "before_category_viewport":root_viewport,"after_category_viewport":viewport,
        "after_world_hover_viewport":ui.viewport_bounds(&app.viewer)?,
        "scope":"diagnostic input witness; no state-neutrality or native parity claim"}),
    )?;
    std::fs::write(out.join("tooltip-world-hover-before.json"), &neutral_source)?;
    std::fs::write(
        out.join("tooltip-world-hover-after.json"),
        app.checkpoint()?,
    )?;
    app = app.restore(&neutral_source)?;
    ui = Input::new()?;
    ensure!(app.observe()? == before);
    let safe = [
        app.viewer.display_size[0] as f32 * 0.5,
        app.viewer.display_size[1] as f32 - 2.,
    ];
    let baseline = ui.application_batch(&app.viewer, &[moved(safe)]);
    apply(&mut app, &baseline.messages)?;
    ensure!(
        app.observe()? == before,
        "HUD background hover reached gameplay: {:?}",
        baseline.messages
    );
    let without_tooltip = ui.batch_pixels(&app.viewer)?;
    let hovered = ui.application_batch(&app.viewer, &[moved(point)]);
    apply(&mut app, &hovered.messages)?;
    let with_tooltip = ui.batch_pixels(&app.viewer)?;
    if without_tooltip == with_tooltip {
        std::fs::write(out.join("tooltip-failure.json"), app.checkpoint()?)?;
        ui.batch_save(&app.viewer, &out.join("tooltip-failure.png"))?;
    }
    write_json(
        &out.join("tooltip-neutrality-before-assert.json"),
        &json!({
            "before":before,"after":app.observe()?,"viewport_before":viewport,
            "viewport_after":ui.viewport_bounds(&app.viewer)?,"safe_background_point":safe,
            "safe_background_messages":baseline.messages.iter().map(|m|format!("{m:?}")).collect::<Vec<_>>(),
            "product_messages":hovered.messages.iter().map(|m|format!("{m:?}")).collect::<Vec<_>>()
        }),
    )?;
    let above_hud = (viewport[1] + viewport[3]) as usize;
    let tooltip_world_pixels = without_tooltip
        .chunks_exact(4)
        .zip(with_tooltip.chunks_exact(4))
        .enumerate()
        .filter(|(i, (a, b))| i / (app.viewer.display_size[0] as usize) < above_hud && a != b)
        .count();
    ensure!(
        tooltip_world_pixels > 0,
        "product hover changed no pixels above the HUD; tooltip absent"
    );
    ensure!(
        app.observe()? == before && ui.viewport_bounds(&app.viewer)? == viewport,
        "tooltip hover/draw mutated retained state or resized viewport"
    );
    ui.batch_save(&app.viewer, &out.join("actual-product-tooltip.png"))?;
    let scroll = hovered
        .owners
        .scrolls
        .iter()
        .find(|s| s.content[3] > s.bounds[3] + 1.)
        .context("actual Buy category list does not overflow")?;
    let scroll_point = [
        scroll.bounds[0] + scroll.bounds[2] * 0.75,
        scroll.bounds[1] + scroll.bounds[3] * 0.5,
    ];
    let scrolling = ui.application_batch(
        &app.viewer,
        &[
            moved(scroll_point),
            iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0., y: -3. },
            }),
        ],
    );
    apply(&mut app, &scrolling.messages)?;
    ensure!(
        scrolling
            .owners
            .scrolls
            .iter()
            .any(|s| s.translation[1] > 0.)
            && app.observe()? == before
    );
    let bytes = app.checkpoint()?;
    std::fs::write(out.join("catalog-ready.json"), &bytes)?;
    let mut rows = vec![];
    let mut failures = vec![];
    for name in ["product-focus-release", "product-resize-release"] {
        let mut branch = app.restore(&bytes)?;
        let mut input = Input::new()?;
        // Restore widget scroll through actual wheel input; widget state is
        // intentionally transient and is not fabricated in saved game owners.
        let scrolled = input.application_batch(
            &branch.viewer,
            &[
                moved(scroll_point),
                iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                    delta: iced::mouse::ScrollDelta::Lines { x: 0., y: -3. },
                }),
            ],
        );
        apply(&mut branch, &scrolled.messages)?;
        hold(&mut input, &mut branch, &product_message)?;
        let mut reference = branch.fork()?;
        let before = branch.observe()?;
        let events = if name == "product-focus-release" {
            vec![
                iced::Event::Window(window::Event::Unfocused),
                iced::Event::Window(window::Event::Focused),
                pointer(false),
            ]
        } else {
            vec![
                iced::Event::Window(window::Event::Resized(Size::new(1000., 740.))),
                pointer(false),
            ]
        };
        let batch = input.application_batch(&branch.viewer, &events);
        ensure!(branch.observe()? == before);
        let raw = batch_record(&events, &batch);
        write_json(&out.join(format!("{name}-before-apply.json")), &raw)?;
        let leaked = batch.messages.iter().any(|m| !passive(m));
        apply(
            &mut reference,
            &batch
                .messages
                .iter()
                .filter(|m| lifetime(m))
                .cloned()
                .collect::<Vec<_>>(),
        )?;
        let applied = apply(&mut branch, &batch.messages).map_err(|e| e.to_string());
        let passed = !leaked && applied.is_ok() && branch.observe()? == reference.observe()?;
        if !passed {
            failures.push(name);
            std::fs::write(out.join(format!("{name}-failure-before.json")), &bytes)?;
            std::fs::write(
                out.join(format!("{name}-failure-after.json")),
                branch.checkpoint()?,
            )?;
        } else {
            if name == "product-focus-release" {
                ensure!(
                    batch.owners == scrolled.owners,
                    "focus reset independent catalogue scrolling"
                );
            }
            let revision = branch
                .viewer
                .device
                .as_ref()
                .unwrap()
                .panel_widgets()
                .unwrap()
                .catalog()
                .unwrap()
                .revision();
            deliberate(
                &mut input,
                &mut branch,
                &format!("BuyNavigate({revision}, Next)"),
            )?;
            ensure!(branch.viewer.resident == 196 && branch.observe()?.tick == tick);
            cold_equal(&branch)?;
        }
        rows.push(json!({"case":name,"batch":raw,"before":before,"passed":passed,"apply_error":applied.err()}));
    }
    write_json(
        &out.join("report.json"),
        &json!({"cases":rows,"failures":failures,
        "actual_product_tooltip_drawn":true,"tooltip_without_sha256":engine::digest(&without_tooltip),
        "tooltip_changed_pixels_above_hud":tooltip_world_pixels,
        "tooltip_with_sha256":engine::digest(&with_tooltip),"viewport":viewport,
        "scroll_initial":hovered.owners,"scroll_moved":scrolling.owners,
        "scope":"real production catalog controls and advanced text tooltip; zero native parity credit"}),
    )?;
    ensure!(
        failures.is_empty(),
        "catalogue event lifetime failures preserved: {failures:?}"
    );
    Ok(())
}
