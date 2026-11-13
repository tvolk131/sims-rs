use super::*;
use clap::Parser;

pub(super) fn checkpoint(viewer: &mut Viewer, output: &Path, name: &str) -> Result<()> {
    viewer.args.save = output.join(format!("{name}.json"));
    viewer.save()?;
    let path = viewer.args.save.clone();
    let session = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    let client = serde_json::to_vec(
        &viewer
            .client
            .as_ref()
            .unwrap()
            .save(viewer.session.as_ref().unwrap())?,
    )?;
    let device = viewer
        .device
        .as_ref()
        .unwrap()
        .save(viewer.session.as_ref().unwrap())?;
    let pixels = sims_render::software::render(viewer.scene.as_ref().unwrap());
    viewer.load(&path)?;
    ensure!(session == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    ensure!(
        client
            == serde_json::to_vec(
                &viewer
                    .client
                    .as_ref()
                    .unwrap()
                    .save(viewer.session.as_ref().unwrap())?
            )?
    );
    let restored_device = viewer
        .device
        .as_ref()
        .unwrap()
        .save(viewer.session.as_ref().unwrap())?;
    if device != restored_device {
        let before = serde_json::to_value(&device)?;
        let after = serde_json::to_value(&restored_device)?;
        let changed: Vec<_> = before.as_object().unwrap().iter().filter_map(|(k,v)| {
            (after.get(k) != Some(v)).then(|| (k.clone(), if k == "frozen_pixels" {
                serde_json::json!({"before_length":v.as_array().map(Vec::len),"after_length":after[k].as_array().map(Vec::len),
                    "different":v.as_array().zip(after[k].as_array()).map(|(a,b)|a.iter().zip(b).filter(|(a,b)|a!=b).count())})
            } else { serde_json::json!({"before":v,"after":after[k]}) }))
        }).collect();
        std::fs::write(
            output.join(format!("{name}-device-diff.json")),
            serde_json::to_vec_pretty(&changed)?,
        )?;
        anyhow::bail!("device changed at {name}: {changed:?}");
    }
    viewer.refresh()?;
    ensure!(session == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    ensure!(pixels == sims_render::software::render(viewer.scene.as_ref().unwrap()));
    Ok(())
}

#[test]
#[ignore = "private original Newbie tutorial and headless Metal adapter"]
fn original_person_buttons_complete_original_portrait_lesson() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_PERSON_BUTTON_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace person-button evidence")?;
    let source = repo.join("cache/newbie-panel-completion/session.json");
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
    ensure!(viewer.client.as_ref().unwrap().buttons().is_none());
    let start = viewer.session.as_ref().unwrap().simulation.vm.tick;
    // Explicitly construct the new real widgets. Restoring an old profile above
    // must not silently add their image ownership or invent a prior click.
    let s = viewer.session.as_mut().unwrap();
    viewer.device.as_mut().unwrap().refresh(s)?;
    let inputs = portrait_inputs(s, viewer.zoom, viewer.floor, true)?;
    viewer
        .client
        .as_mut()
        .unwrap()
        .bind_resident_buttons(s, inputs)?;
    checkpoint(&mut viewer, &output, "constructing-buttons")?;
    let mut slices = 0;
    while viewer.client.as_ref().unwrap().ui_lookup_pending() {
        viewer.step()?;
        slices += 1;
        ensure!(slices < 10000, "resident construction budget exceeded");
    }
    viewer.refresh()?;
    ensure!(viewer.session.as_ref().unwrap().simulation.vm.tick == start);
    let buttons = viewer.client.as_ref().unwrap().buttons().unwrap();
    ensure!(buttons.initialized && buttons.activated.is_none());
    ensure!(buttons.items.len() == 2 && buttons.items.iter().all(|b| b.image.is_some()));
    checkpoint(&mut viewer, &output, "buttons-bound")?;
    render_headless_element(&root, &output.join("buttons-bound.png"), viewer.view())?;
    let d = &viewer.session.as_ref().unwrap().simulation.vm.world.dialogs;
    let intro = d
        .records
        .iter()
        .find(|r| Some(r.id) == d.modal)
        .context("intro absent")?;
    ensure!(intro.origin.bhav == 4097 && intro.origin.instruction == 3);
    let _ = viewer.update(Message::Dialog(intro.id, 2));
    let mut seen = Vec::new();
    for _ in 0..2000 {
        let _ = viewer.update(Message::Step);
        ensure!(viewer.error.is_none(), "person lesson: {:?}", viewer.error);
        let s = viewer.session.as_ref().unwrap();
        let d = &s.simulation.vm.world.dialogs;
        if let Some(record) = d.records.iter().find(|r| Some(r.id) == d.modal) {
            seen.push(serde_json::to_value(record)?);
            if record.origin.bhav == 4125 && record.origin.instruction == 1 {
                break;
            }
            anyhow::bail!("unexpected dialog: {:?}", record);
        }
        if s.simulation
            .vm
            .world
            .objects
            .get(&322)
            .is_some_and(|o| o.attributes.get(2) == Some(&5))
        {
            break;
        }
    }
    checkpoint(&mut viewer, &output, "portrait-prompt")?;
    render_headless_element(&root, &output.join("portrait-prompt.png"), viewer.view())?;
    std::fs::write(
        output.join("initial-report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "start":start,"tick":viewer.session.as_ref().unwrap().simulation.vm.tick,
            "dialogs":seen,"buttons":viewer.client.as_ref().unwrap().buttons(),"slices":slices,
            "owner":viewer.session.as_ref().unwrap().simulation.vm.world.objects.get(&322),
            "scope":"Explicit widget construction and original introduction only; portrait lesson remains pending."
        }))?,
    )?;
    ensure!(
        viewer
            .client
            .as_ref()
            .unwrap()
            .buttons()
            .unwrap()
            .highlighted(48)
    );
    Ok(())
}

#[test]
#[ignore = "private original Newbie checkpoint; preserve first person-control frontend boundary"]
fn original_person_lesson_preserves_unbound_portrait_request() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let source = repo.join("cache/newbie-panel-completion/session.json");
    let output = repo.join(std::env::var("SIMS_PERSON_BOUNDARY_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace person lesson evidence")?;
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
    let dialogs = &viewer.session.as_ref().unwrap().simulation.vm.world.dialogs;
    let intro = dialogs
        .records
        .iter()
        .find(|r| Some(r.id) == dialogs.modal)
        .context("original introduction absent")?;
    ensure!(intro.origin.bhav == 4097 && intro.origin.instruction == 3);
    ensure!(intro.buttons.len() == 1 && intro.buttons[0].result == 2);
    let _ = viewer.update(Message::Dialog(intro.id, 2));
    for _ in 0..1000 {
        if viewer.error.is_some() {
            break;
        }
        let _ = viewer.update(Message::Step);
    }
    let error = viewer
        .error
        .clone()
        .context("person lesson did not reach its boundary")?;
    let s = viewer.session.as_ref().unwrap();
    let pending = s
        .simulation
        .vm
        .threads
        .iter()
        .find_map(|(&id, t)| match &t.status {
            sims_vm::Status::AwaitingUiEffect(call) => Some((id, call.request())),
            _ => None,
        })
        .context("person lesson lost its original UI request")?;
    ensure!(pending.1.kind == sims_vm::ui_effect::Kind::Portrait);
    ensure!(pending.1.value == 48 && pending.1.enabled);
    viewer.save()?;
    let before = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    let pixels = sims_render::software::render(viewer.scene.as_ref().unwrap());
    render_headless_element(&root, &output.join("portrait-boundary.png"), viewer.view())?;
    ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    viewer.load(&save)?;
    viewer.refresh()?;
    ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    ensure!(pixels == sims_render::software::render(viewer.scene.as_ref().unwrap()));
    let retry = viewer.step().unwrap_err().to_string();
    ensure!(retry == error);
    ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source":source,"start_tick":start,"tick":viewer.session.as_ref().unwrap().simulation.vm.tick,
            "request":pending.1,"thread":pending.0,"error":error,
            "full_session_and_pixels_restore":true,"retry_preserves_request_and_state":true,
            "headless_render_preserves_session":true,
            "scope":"One explicit OK to original General tutorial4097 instruction3. No person-button event or portrait reply is fabricated. The next original UI request remains pending."
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private original portrait lesson checkpoint; actual Iced inputs and headless frames"]
fn original_person_button_click_reaches_original_go_here_prompt() -> Result<()> {
    use sims_runtime::original_session::ui_wait::Progress as WaitProgress;
    use sims_vm::ui_owner::wait::Condition;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let source = repo.join("cache/newbie-person-buttons-construction/portrait-prompt.json");
    let output = repo.join(std::env::var("SIMS_PERSON_CLICK_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace person-click evidence")?;
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
    let s = viewer.session.as_ref().unwrap();
    let start = s.simulation.vm.tick;
    let owner = s
        .simulation
        .vm
        .world
        .ui_owner
        .owner
        .context("tutorial owner absent")?;
    let bob = s
        .house
        .household
        .as_ref()
        .unwrap()
        .residents
        .iter()
        .find(|r| r.neighbor == 48)
        .unwrap()
        .object as i16;
    ensure!(viewer.resident == bob && s.simulation.vm.world.objects[&owner].attributes[2] == 5);
    ensure!(
        viewer
            .client
            .as_ref()
            .unwrap()
            .buttons()
            .unwrap()
            .activated
            .is_none()
    );
    // Merely loading with Bob selected must not satisfy a button-click watch.
    for _ in 0..3 {
        ensure!(viewer.poll_application_ui().context("no-click poll")?);
        ensure!(
            viewer.session.as_ref().unwrap().simulation.vm.world.objects[&owner].attributes[2] == 5
        );
    }
    let _ = viewer.update(Message::Live);
    for _ in 0..1000 {
        if !viewer.presentation_pending() {
            break;
        }
        let _ = viewer.update(Message::Step);
        ensure!(viewer.error.is_none(), "Live control: {:?}", viewer.error);
    }
    ensure!(!viewer.presentation_pending());
    ensure!(
        viewer.client.as_ref().unwrap().buttons().unwrap().activated
            == Some(ButtonKey::Control(2010))
    );
    ensure!(viewer.poll_application_ui().context("wrong-control poll")?);
    ensure!(
        viewer.session.as_ref().unwrap().simulation.vm.world.objects[&owner].attributes[2] == 5
    );
    // Use the actual resident button handler, even though Bob is already selected.
    let _ = viewer.update(Message::Resident(bob));
    ensure!(viewer.error.is_none(), "Bob click: {:?}", viewer.error);
    ensure!(
        viewer.client.as_ref().unwrap().buttons().unwrap().activated
            == Some(ButtonKey::Resident(48))
    );
    // The button also starts its real floor/camera command. Complete that
    // synchronous callback before the next application UI-wait poll.
    for _ in 0..1000 {
        if !viewer.presentation_pending() {
            break;
        }
        viewer.step()?;
    }
    ensure!(!viewer.presentation_pending());
    viewer.refresh()?;
    let s = viewer.session.as_mut().unwrap();
    let focused = viewer
        .client
        .as_ref()
        .unwrap()
        .buttons()
        .unwrap()
        .focused_resource();
    s.begin_ui_wait(focused)
        .context("begin actual portrait observation")?;
    let mut observed = false;
    for _ in 0..1000 {
        match s.advance_ui_wait(256)? {
            WaitProgress::Observe(Condition::Portrait(48)) => {
                observed = true;
                break;
            }
            WaitProgress::Pending => (),
            other => anyhow::bail!("lost portrait observation: {other:?}"),
        }
    }
    ensure!(observed && s.simulation.vm.world.globals.get(&12) == Some(&-1));
    let inputs = portrait_inputs(s, viewer.zoom, viewer.floor, true)?;
    viewer
        .client
        .as_mut()
        .unwrap()
        .begin_portrait_observation(s, 48, inputs)?;
    let before = serde_json::to_vec(&s.save())?;
    ensure!(s.step().is_err() && s.advance_ui_wait(1).is_err());
    ensure!(before == serde_json::to_vec(&s.save())?);
    checkpoint(&mut viewer, &output, "pending-observation")?;
    // Also open a completely new viewer. In-place restore can accidentally
    // hide missing widget reconstruction by retaining the old Iced handles.
    viewer.status.clear();
    let before = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("pending-before.png"), viewer.view())?;
    let mut args = viewer.args.clone();
    args.load = Some(output.join("pending-observation.json"));
    let ready = Arc::new(Ready::load(&args)?);
    let (mut fresh, _) = Viewer::new(args);
    fresh.install(ready)?;
    fresh.status.clear();
    render_headless_element(&root, &output.join("pending-reopened.png"), fresh.view())?;
    ensure!(before == serde_json::to_vec(&fresh.session.as_ref().unwrap().save())?);
    ensure!(
        image::open(output.join("pending-before.png"))?.to_rgba8()
            == image::open(output.join("pending-reopened.png"))?.to_rgba8(),
        "fresh viewer lost displayed widgets while portrait lookup was suspended"
    );
    viewer = fresh;
    let click_tick = viewer.session.as_ref().unwrap().simulation.vm.tick;
    for _ in 0..1000 {
        if !viewer.client.as_ref().unwrap().ui_lookup_pending() {
            break;
        }
        viewer.step()?;
    }
    ensure!(!viewer.client.as_ref().unwrap().ui_lookup_pending());
    ensure!(
        viewer
            .poll_application_ui()
            .context("finish actual portrait observation")?
    );
    let s = viewer.session.as_ref().unwrap();
    ensure!(s.simulation.vm.tick == click_tick);
    ensure!(s.simulation.vm.world.objects[&owner].attributes[2] == 0);
    let mut reached = false;
    for _ in 0..1000 {
        let _ = viewer.update(Message::Step);
        ensure!(
            viewer.error.is_none(),
            "after Bob click: {:?}",
            viewer.error
        );
        let s = viewer.session.as_ref().unwrap();
        if s.simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .any(|r| r.origin.bhav == 4125 && r.origin.instruction == 3)
        {
            reached = true;
            break;
        }
    }
    ensure!(reached, "original person lesson did not reach Go Here");
    ensure!(
        !viewer
            .client
            .as_ref()
            .unwrap()
            .buttons()
            .unwrap()
            .highlighted(48)
    );
    checkpoint(&mut viewer, &output, "session")?;
    let before = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("go-here-prompt.png"), viewer.view())?;
    ensure!(before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source":source,"start_tick":start,"tick":viewer.session.as_ref().unwrap().simulation.vm.tick,
            "bob":bob,"owner":owner,"click_tick":click_tick,
            "selected_person_does_not_satisfy_click":true,"other_control_does_not_satisfy_click":true,
            "actual_bob_button_completes_original_watch":true,"global12":-1,
            "pending_observation_session_client_device_pixels_restore":true,
        "fresh_viewer_pending_full_iced_frame_identical":true,
            "pending_observation_rejects_ticks_and_polling_without_mutation":true,
            "original_highlight_cleared":true,"final_session_client_device_pixels_restore":true,
            "headless_render_preserves_session":true,"next_bhav":4125,"next_instruction":3,
            "scope":"Actual Live/Bob inputs and original UI query/got callbacks. Go Here prompt is preserved; no ground interaction or further tutorial answer is synthesized."
        }))?,
    )?;
    Ok(())
}
