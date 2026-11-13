//! Replacement-client invariants, not evidence of original widget appearance
//! or a new native gameplay-parity claim. Every activation uses a retained
//! Iced Tree; serialized sibling branches check the actual application owners.
use super::*;

fn simulation_hash(app: &Application) -> Result<String> {
    Ok(engine::digest(&serde_json::to_vec(
        &app.session()?.simulation,
    )?))
}

pub(super) fn control(
    ui: &mut Input,
    app: &Application,
    expected: &str,
) -> Result<([f32; 2], Message)> {
    let before = app.observe()?;
    for point in ui.centers(&app.viewer) {
        let _ = ui.move_to(&app.viewer, point);
        // Layout nodes also include the viewport, whose press message is not
        // a control candidate. Discovery never dispatches those messages.
        let _ = ui.button(&app.viewer, true);
        let messages = ui.button(&app.viewer, false);
        if messages.len() == 1 && format!("{:?}", messages[0]) == expected {
            ensure!(app.observe()? == before, "control discovery changed state");
            return Ok((point, messages[0].clone()));
        }
    }
    anyhow::bail!("rendered {expected} control absent")
}

pub(super) fn activate(ui: &mut Input, app: &mut Application, expected: &str) -> Result<[f32; 2]> {
    let (point, _) = control(ui, app, expected)?;
    let messages = click_messages(ui, &app.viewer, point)?;
    ensure!(
        messages.len() == 1 && format!("{:?}", messages[0]) == expected,
        "control changed between discovery and activation: {messages:?}"
    );
    app.dispatch(messages[0].clone())?;
    app.drain(&Limits::default())?;
    Ok(point)
}

pub(super) fn passive_frame(ui: &mut Input, app: &Application) -> Result<()> {
    let before = app.observe()?;
    let _ = ui.pixels(&app.viewer)?;
    ensure!(app.observe()? == before, "passive Iced frame changed state");
    Ok(())
}

#[test]
#[ignore = "private managed Mashuga checkpoint; authentic catalog resources and headless Iced menu"]
fn authentic_roach_title_and_overlay_layout_are_state_neutral() -> Result<()> {
    run_worker(roach_title)
}

fn roach_title() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    select(&mut app, 196)?;
    let mut ui = Input::new()?;
    let definition = app.session()?.simulation.vm.world.objects[&140]
        .definition
        .clone()
        .context("roach definition absent")?;
    ensure!(
        definition.source == "ExpansionPack1/ExpansionPack.far::Roaches.iff"
            && definition.object == 16807
            && definition.guid == 1142270350,
        "fixture no longer has the captured roach definition"
    );
    let index = app.session()?.index();
    let iff =
        sims_formats::iff::Iff::parse(index.resources[index.find(&definition.source)?].read()?)?;
    let object = iff
        .get(b"OBJD", definition.object)
        .context("roach OBJD absent")?;
    let fields = sims_formats::behavior::ObjectDefinition::parse(iff.data(object))?;
    ensure!(fields.catalog == 2000 && iff.get(b"CATS", fields.catalog).is_none());
    let strings = sims_formats::strings::StringTable::parse(
        iff.data(
            iff.get(b"CTSS", fields.catalog)
                .context("roach CTSS absent")?,
        ),
    )?;
    let actual = strings.get(0, 1).context("roach catalog title absent")?;
    ensure!(actual == "SSRI Virtual Reality Set");
    let before_label = app.observe()?;
    ensure!(app.viewer.label(140) == actual);
    ensure!(
        app.observe()? == before_label,
        "label lookup changed retained state"
    );
    let viewport = ui.viewport_bounds(&app.viewer)?;
    let tick = app.observe()?.tick;
    app.execute(
        &Command::BeginObject {
            identity: Identity::get(app.session()?, 140)?,
        },
        &Limits::default(),
    )?;
    ensure!(app.menu().is_none() && app.session()?.object_input_pending());
    ensure!(ui.viewport_bounds(&app.viewer)? == viewport);
    passive_frame(&mut ui, &app)?;
    let bytes = app.checkpoint()?;
    std::fs::write(out.join("pending-roach-menu.json"), &bytes)?;
    let mut cold = app.restore(&bytes)?;
    ensure!(app.observe()? == cold.observe()?);
    app.drain(&Limits::default())?;
    cold.drain(&Limits::default())?;
    ensure!(app.observe()? == cold.observe()? && app.observe()?.tick == tick);
    ensure!(
        app.menu()
            .is_some_and(|m| m.target() == 140 && m.actions().iter().any(|a| a.menu_id == 2)),
        "original Spray action absent"
    );
    ensure!(app.viewer.label(140) == actual && cold.viewer.label(140) == actual);
    ensure!(ui.viewport_bounds(&app.viewer)? == viewport);
    let ready = app.observe()?;
    let ready_sim = simulation_hash(&app)?;
    let status = app.viewer.status.clone();
    app.viewer.status = "Long application status during an open interaction. ".repeat(20);
    app.viewer.error = Some("Synthetic presentation feedback overlay. ".repeat(20));
    let with_feedback = app.observe()?;
    ensure!(ui.viewport_bounds(&app.viewer)? == viewport);
    ui.save(&app.viewer, &out.join("roach-menu-feedback.png"))?;
    ensure!(app.observe()? == with_feedback && simulation_hash(&app)? == ready_sim);
    app.viewer.status = status;
    app.viewer.error = None;
    ensure!(app.observe()? == ready);
    ui.save(&app.viewer, &out.join("authentic-roach-menu.png"))?;
    activate(&mut ui, &mut app, "ObjectCancel")?;
    cold.dispatch(Message::ObjectCancel)?;
    cold.drain(&Limits::default())?;
    ensure!(app.observe()? == cold.observe()? && app.observe()?.tick == tick);
    write_json(
        &out.join("report.json"),
        &json!({"definition":definition,"catalog":fields.catalog,"authentic_catalog_title":actual,"catalog_lookup_correct":true,"native_popup_heading_rule":"unverified; custom replacement heading unchanged","viewport":viewport,"pending_cold_equal":true,"overlay_layout_and_render_state_neutral":true,"tick":tick,"strict_native_parity_ticks_added":0}),
    )?;
    Ok(())
}

#[test]
#[ignore = "private managed Mashuga checkpoint; real Iced input, pending collectors and headless GPU"]
fn pending_menu_resize_and_cancel_lifetimes() -> Result<()> {
    run_worker(pending_menu)
}

fn pending_menu() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    select(&mut app, 196)?;
    app.execute(
        &Command::Resize {
            width: 800,
            height: 600,
        },
        &Limits::default(),
    )?;
    let mut ui = Input::new()?;
    app.execute(
        &Command::BeginObject {
            identity: Identity::get(app.session()?, 140)?,
        },
        &Limits::default(),
    )?;
    let input = app
        .session()?
        .host
        .presentation
        .object_input
        .as_ref()
        .context("pending object input absent")?;
    ensure!(!input.ready() && !input.can_cancel());
    let pending = serde_json::to_value(input)?;
    let sim = simulation_hash(&app)?;
    let start = app.observe()?;
    let mut rows = vec![];

    // A previous widget event may arrive after the synchronous collector owns
    // input. Rejection must not execute a second operation or consume RNG.
    let other = app
        .residents()?
        .into_iter()
        .find(|r| r.object != 196)
        .context("second resident absent")?;
    for message in [
        Message::Resident(other.object),
        Message::Needs,
        Message::Buy,
        Message::ZoomIn,
        Message::ObjectChoice(0),
        Message::ObjectCancel,
        Message::DialogKey(27),
    ] {
        let before = app.observe()?;
        let name = format!("{message:?}");
        app.dispatch(message)?;
        ensure!(app.observe()? == before, "pending input accepted {name}");
        rows.push(json!({"blocked_while_pending":name}));
    }
    let bytes = app.checkpoint()?;
    std::fs::write(out.join("pending.json"), &bytes)?;
    let mut cold = app.restore(&bytes)?;
    ensure!(
        app.observe()? == cold.observe()?,
        "pending restore changed state"
    );

    // Dispatch Resize directly: draining here would complete the very caller
    // whose retention across layout changes this sequence is checking.
    for [width, height] in [[1000, 740], [1320, 900], [800, 600]] {
        for branch in [&mut app, &mut cold] {
            branch.dispatch(Message::DisplaySize(Size::new(width as f32, height as f32)))?;
            ensure!(
                simulation_hash(branch)? == sim,
                "resize ran simulation work"
            );
            ensure!(
                serde_json::to_value(branch.session()?.host.presentation.object_input.as_ref())?
                    == pending,
                "resize changed suspended object input"
            );
        }
        ensure!(app.observe()? == cold.observe()?, "pending resize diverged");
        passive_frame(&mut ui, &app)?;
        rows.push(json!({"pending_resize":[width,height],"cold_equal":true}));
    }
    app.drain(&Limits::default())?;
    cold.drain(&Limits::default())?;
    ensure!(
        app.observe()? == cold.observe()?,
        "collector continuation diverged"
    );
    ensure!(app.menu().is_some_and(|m| !m.actions().is_empty()));
    ensure!(app.observe()?.tick == start.tick);
    std::fs::write(out.join("ready-menu.json"), app.checkpoint()?)?;

    // Hold Cancel at its old location while the whole widget moves. Releasing
    // outside its new bounds must not activate it or click through to the lot.
    let (point, _) = control(&mut ui, &app, "ObjectCancel")?;
    let _ = ui.move_to(&app.viewer, point);
    ensure!(ui.button(&app.viewer, true).is_empty());
    for branch in [&mut app, &mut cold] {
        branch.dispatch(Message::DisplaySize(Size::new(1320., 900.)))?;
    }
    let before_release = app.observe()?;
    let released = ui.button(&app.viewer, false);
    ensure!(
        released.is_empty(),
        "resized pressed menu leaked activation: {released:?}"
    );
    ensure!(app.observe()? == before_release && app.menu().is_some());
    ensure!(app.observe()? == cold.observe()?);
    passive_frame(&mut ui, &app)?;
    ui.save(&app.viewer, &out.join("pressed-resized-menu.png"))?;
    activate(&mut ui, &mut app, "ObjectCancel")?;
    cold.dispatch(Message::ObjectCancel)?;
    cold.drain(&Limits::default())?;
    ensure!(app.menu().is_none() && app.observe()? == cold.observe()?);
    ensure!(app.observe()?.tick == start.tick);
    // Ended popup events cannot resurrect its action or cancel a later owner.
    for message in [Message::ObjectChoice(0), Message::ObjectCancel] {
        let before = app.observe()?;
        app.dispatch(message)?;
        ensure!(app.observe()? == before, "ended menu event changed state");
    }
    write_json(
        &out.join("report.json"),
        &json!({"cases":rows,"resized_press_cancelled":true,"stale_popup_messages_ignored":2,"pending_and_ready_cold_equal":true,"tick":start.tick,"strict_native_parity_ticks_added":0}),
    )?;
    Ok(())
}

#[test]
#[ignore = "private managed Mashuga checkpoint; actual resident/mode widgets with headless GPU"]
fn resident_mode_resize_and_cold_resume_sequence() -> Result<()> {
    run_worker(mode_sequence)
}

fn mode_sequence() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    select(&mut app, 196)?;
    let residents = app.residents()?;
    ensure!(
        residents.len() == 2,
        "sequence requires the two Mashuga residents"
    );
    let mut ui = Input::new()?;
    let tick = app.observe()?.tick;
    let mut rows = vec![];
    for (round, [width, height]) in [[800, 600], [1320, 900]].into_iter().enumerate() {
        app.execute(&Command::Resize { width, height }, &Limits::default())?;
        for resident in residents.iter().rev() {
            let expected = format!("Resident({})", resident.object);
            activate(&mut ui, &mut app, &expected)?;
            ensure!(app.viewer.resident == resident.object);
            let bytes = app.checkpoint()?;
            let mut cold = app.restore(&bytes)?;
            ensure!(
                app.observe()? == cold.observe()?,
                "selected resident restore diverged"
            );
            let mut cold_ui = Input::new()?;
            // Native selection and Needs publication may change VM-visible
            // fields. The requirement is equal ordinary processing, not that
            // a real control activation leaves the simulation unchanged.
            activate(&mut ui, &mut app, "Needs")?;
            activate(&mut cold_ui, &mut cold, "Needs")?;
            ensure!(
                app.observe()? == cold.observe()?,
                "restored Needs input diverged"
            );
            ensure!(app.observe()?.tick == tick);
            passive_frame(&mut ui, &app)?;
            rows.push(json!({"round":round,"size":[width,height],"resident":resident.object,"needs_cold_equal":true}));
        }
        let (_, stale_needs) = control(&mut ui, &app, "Needs")?;
        let (_, stale_resident) =
            control(&mut ui, &app, &format!("Resident({})", residents[1].object))?;
        activate(&mut ui, &mut app, "Buy")?;
        ensure!(
            !app.viewer.people_panel_visible(),
            "Buy did not hide People widgets"
        );
        for message in [stale_needs, stale_resident] {
            let before = app.observe()?;
            app.dispatch(message)?;
            ensure!(
                app.observe()? == before,
                "hidden People event changed state"
            );
        }
        let bytes = app.checkpoint()?;
        std::fs::write(out.join(format!("buy-{round}.json")), &bytes)?;
        let mut cold = app.restore(&bytes)?;
        ensure!(
            app.observe()? == cold.observe()?,
            "Buy cold restore diverged"
        );
        let mut cold_ui = Input::new()?;
        for branch in [&mut app, &mut cold] {
            branch.dispatch(Message::DisplaySize(Size::new(1000., 740.)))?;
        }
        ensure!(app.observe()? == cold.observe()?, "Buy resize diverged");
        passive_frame(&mut ui, &app)?;
        activate(&mut ui, &mut app, "Live")?;
        activate(&mut cold_ui, &mut cold, "Live")?;
        ensure!(
            app.observe()? == cold.observe()?,
            "cold Live return diverged"
        );
        ensure!(app.viewer.people_panel_visible() && app.observe()?.tick == tick);
        // No press from the removed mode widgets may survive the transition.
        ensure!(ui.button(&app.viewer, false).is_empty());
        passive_frame(&mut ui, &app)?;
        rows.push(json!({"round":round,"buy_resize_cold_live_equal":true,"stale_hidden_controls_ignored":2}));
    }
    ui.save(&app.viewer, &out.join("returned-live.png"))?;
    write_json(
        &out.join("report.json"),
        &json!({"cases":rows,"mode_round_trips":2,"resident_publications":4,"tick":tick,"strict_native_parity_ticks_added":0}),
    )?;
    Ok(())
}
