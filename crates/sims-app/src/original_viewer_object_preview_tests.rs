use super::*;
use clap::Parser;

#[test]
#[ignore = "private original Newbie checkpoint; headless Iced Metal rendering"]
fn original_motive_popup_completes_person_lesson_and_retains_buy_boundary() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_SHOWER_POPUP_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let source = repo.join("cache/newbie-shower-finish-first/session.json");
    let original = std::fs::read(&source)?;
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
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let s = v.session.as_ref().unwrap();
    let start = s.simulation.vm.tick;
    let actor = &s.simulation.vm.world.actions.as_ref().unwrap().actors[&105];
    ensure!(actor.active.is_none());
    ensure!(
        actor
            .previous
            .as_ref()
            .is_some_and(|a| a.id == 3 && a.target == 107)
    );
    ensure!(s.simulation.vm.world.objects[&107].data.get(&62) == Some(&0));
    let instruction = s
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .find(|r| r.status == 1 && r.origin.bhav == 4125 && r.origin.instruction == 18)
        .context("original needs-popup lesson absent")?;
    ensure!(
        instruction
            .body
            .starts_with("Click on the different Need indicator bars.")
    );
    let _ = v.update(Message::Dialog(instruction.id, 2));
    ensure!(v.error.is_none(), "{:?}", v.error);
    ensure!(
        v.client
            .as_ref()
            .unwrap()
            .live_panel()
            .unwrap()
            .popup()
            .is_none()
    );
    let before_click = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    let _ = v.update(Message::MotiveDetails(3));
    ensure!(v.error.is_none(), "{:?}", v.error);
    ensure!(v.client.as_ref().unwrap().live_panel().unwrap().popup() == Some(3));
    ensure!(
        before_click == serde_json::to_vec(&v.session.as_ref().unwrap().save())?,
        "opening the actual widget modified simulation state"
    );
    super::person_tests::checkpoint(&mut v, &output, "hygiene-popup")?;
    render_headless_element(&root, &output.join("hygiene-popup.png"), v.view())?;
    // Original4143 keeps rearming its20-tick counter while4764fe observes
    // the popup. It returns only after the player closes the explanation.
    for _ in 0..32 {
        let _ = v.update(Message::Step);
        ensure!(v.error.is_none(), "{:?}", v.error);
    }
    ensure!(
        v.session.as_ref().unwrap().simulation.vm.threads[&322]
            .stack
            .iter()
            .any(|f| f.location.bhav == 4143)
    );
    ensure!(v.client.as_ref().unwrap().live_panel().unwrap().popup() == Some(3));
    let before_close = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    let _ = v.update(Message::CloseMotiveDetails);
    ensure!(v.error.is_none(), "{:?}", v.error);
    ensure!(
        v.client
            .as_ref()
            .unwrap()
            .live_panel()
            .unwrap()
            .popup()
            .is_none()
    );
    ensure!(before_close == serde_json::to_vec(&v.session.as_ref().unwrap().save())?);
    super::person_tests::checkpoint(&mut v, &output, "popup-closed")?;
    let expected = "original UI control 2008 is not bound in this viewer";
    let mut stopped = false;
    for i in 0..2000 {
        let _ = v.update(Message::Step);
        if let Some(error) = &v.error {
            ensure!(error == expected, "unexpected next boundary: {error}");
            stopped = true;
            break;
        }
        let s = v.session.as_ref().unwrap();
        ensure!(
            s.simulation.vm.world.dialogs.modal.is_none(),
            "unseen modal instruction"
        );
        if i % 100 == 0 {
            println!("popup slice{i}: tick{}", s.simulation.vm.tick);
        }
    }
    if !stopped {
        v.args.save = output.join("unexpected-popup-wait.json");
        v.save()?;
    }
    ensure!(stopped, "original popup condition did not advance");
    let s = v.session.as_ref().unwrap();
    let pending = s
        .simulation
        .vm
        .threads
        .iter()
        .find_map(|(&id, t)| match &t.status {
            sims_vm::Status::AwaitingUiEffect(call) => Some((id, call.request())),
            _ => None,
        })
        .context("Buy highlight request was lost")?;
    ensure!(
        pending.1.kind == sims_vm::ui_effect::Kind::Control
            && pending.1.value == 2008
            && pending.1.enabled
    );
    let owner = &s.simulation.vm.threads[&pending.0];
    ensure!(
        owner
            .stack
            .iter()
            .any(|f| f.location.bhav == 4126 && f.location.instruction == 2)
    );
    ensure!(
        !owner
            .stack
            .iter()
            .any(|f| matches!(f.location.bhav, 4125 | 4143))
    );
    let retained = serde_json::to_vec(&s.save())?;
    let _ = v.update(Message::Step);
    ensure!(v.error.as_deref() == Some(expected));
    ensure!(retained == serde_json::to_vec(&v.session.as_ref().unwrap().save())?);
    super::person_tests::checkpoint(&mut v, &output, "buy-boundary")?;
    v.args.save = save;
    v.save()?;
    render_headless_element(&root, &output.join("buy-boundary.png"), v.view())?;
    ensure!(std::fs::read(&source)? == original);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "start": start, "tick": v.session.as_ref().unwrap().simulation.vm.tick,
            "popup": 3, "actual_widget_click": true, "click_preserves_session": true,
            "shower_finished": true, "original_person_control_lesson_complete": true,
        "full_client_session_device_restores": 3, "next_boundary": expected,
        "open_popup_holds_lesson": true, "original_popup_closed_before_continuation": true,
            "boundary_retries_without_mutation": true, "buy_highlight_retained": pending,
            "source_unchanged": true, "native_execution": false
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private original Newbie checkpoint; headless Iced Metal rendering"]
fn original_shower_finishes_and_restores_before_the_popup_lesson() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_SHOWER_FINISH_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let source = repo.join("cache/newbie-shower-interaction-checked/session.json");
    let original = std::fs::read(&source)?;
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
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let start = v.session.as_ref().unwrap().simulation.vm.tick;
    let hygiene = |v: &Viewer| {
        f32::from_bits(
            v.session.as_ref().unwrap().simulation.vm.world.objects[&105]
                .motives
                .unwrap()[8],
        )
    };
    let initial_hygiene = hygiene(&v);
    let s = v.session.as_ref().unwrap();
    let instruction = s
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .find(|r| r.origin.bhav == 4125 && r.origin.instruction == 17 && r.status == 1)
        .context("original hygiene explanation absent")?;
    ensure!(
        instruction.body
            == "Notice the Hygiene bar below. The green arrows show this need is improving."
    );
    let prior: Vec<_> = s
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .map(|r| r.id)
        .collect();
    let _ = v.update(Message::Dialog(instruction.id, 2));
    ensure!(v.error.is_none(), "{:?}", v.error);
    let mut improving_saved = false;
    let mut next = None;
    let mut error = None;
    let mut stop = "slice_budget";
    for i in 0..20000 {
        let _ = v.update(Message::Step);
        if let Some(e) = &v.error {
            error = Some(e.clone());
            stop = "unsupported_service";
            break;
        }
        if hygiene(&v) > initial_hygiene && !improving_saved {
            v.refresh()?;
            super::person_tests::checkpoint(&mut v, &output, "hygiene-improving")?;
            render_headless_element(&root, &output.join("hygiene-improving.png"), v.view())?;
            improving_saved = true;
        }
        let s = v.session.as_ref().unwrap();
        if let Some(r) = s
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .find(|r| r.status == 1 && !prior.contains(&r.id))
        {
            next = Some(r.clone());
            stop = "next_original_instruction";
            break;
        }
        if s.simulation.vm.tick >= start + 5000 {
            stop = "tick_budget";
            break;
        }
        if i % 250 == 0 {
            println!(
                "shower finish slice{i}: tick{} hygiene{}",
                s.simulation.vm.tick,
                hygiene(&v)
            );
        }
    }
    v.refresh()?;
    v.args.save = save;
    v.save()?;
    render_headless_element(&root, &output.join("next-instruction.png"), v.view())?;
    ensure!(std::fs::read(&source)? == original);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "start": start, "tick": v.session.as_ref().unwrap().simulation.vm.tick,
            "stop": stop, "error": error, "hygiene_before": initial_hygiene,
            "hygiene_after": hygiene(&v), "improving_restored": improving_saved,
            "next_dialog": next, "source_unchanged": true, "native_execution": false
        }))?,
    )?;
    ensure!(error.is_none(), "shower finish boundary: {error:?}");
    ensure!(improving_saved, "no observed hygiene gain");
    let next = next.context("tutorial did not advance")?;
    ensure!(
        next.origin.bhav == 4125 && next.origin.instruction == 18,
        "unexpected tutorial instruction: {:?}",
        next.origin
    );
    super::person_tests::checkpoint(&mut v, &output, "shower-finished")?;
    Ok(())
}

#[test]
#[ignore = "private original Newbie checkpoint; headless Iced Metal rendering"]
fn original_shower_player_command_advances_the_tutorial_and_restores() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_SHOWER_INTERACTION_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let source = repo.join("cache/newbie-object-preview-iced-checked/session.json");
    let original = std::fs::read(&source)?;
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
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    ensure!(v.resident == 105);
    let actor = v.resident;
    let start = v.session.as_ref().unwrap().simulation.vm.tick;
    let hygiene = |v: &Viewer| {
        f32::from_bits(
            v.session.as_ref().unwrap().simulation.vm.world.objects[&actor]
                .motives
                .unwrap()[8],
        )
    };
    let before_hygiene = hygiene(&v);
    let prior_dialogs: Vec<_> = v
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .map(|r| r.id)
        .collect();
    let _ = v.update(Message::Select(Some(107)));
    ensure!(v.error.is_none(), "{:?}", v.error);
    super::object_tests::menu(&mut v)?;
    let s = v.session.as_ref().unwrap();
    ensure!(
        s.simulation.vm.tick == start,
        "menu advanced household clock"
    );
    let input = s.host.presentation.object_input.as_ref().unwrap();
    let choice = input
        .actions()
        .iter()
        .position(|a| {
            s.simulation
                .vm
                .action_menu(&s.program, a.target)
                .unwrap()
                .iter()
                .any(|e| e.id == a.menu_id && e.label == "Take a Shower")
        })
        .context("original Take a Shower interaction absent")?;
    let selected = input.actions()[choice].clone();
    let _ = v.update(Message::ObjectChoice(choice));
    ensure!(v.error.is_none(), "{:?}", v.error);
    let action = v
        .session
        .as_ref()
        .unwrap()
        .simulation
        .pending_interactions(actor)
        .unwrap()
        .back()
        .context("queued shower absent")?
        .clone();
    ensure!(action.target == 107 && action.menu_id == selected.menu_id);
    super::person_tests::checkpoint(&mut v, &output, "shower-queued")?;
    render_headless_element(&root, &output.join("shower-queued.png"), v.view())?;
    let mut active_saved = false;
    let mut behavior_seen = false;
    let mut next_dialog = None;
    let mut error = None;
    let mut stop = "slice_budget";
    for i in 0..10000 {
        let _ = v.update(Message::Step);
        if let Some(e) = &v.error {
            error = Some(e.clone());
            stop = "unsupported_service";
            break;
        }
        let s = v.session.as_ref().unwrap();
        let active = s.simulation.vm.world.actions.as_ref().unwrap().actors[&actor]
            .active
            .as_ref()
            .is_some_and(|a| a.id == action.id);
        behavior_seen |= s.simulation.vm.threads.values().any(|t| {
            t.stack
                .iter()
                .any(|f| f.location.source.ends_with("ShowerC.iff"))
        });
        if active && !active_saved {
            v.refresh()?;
            super::person_tests::checkpoint(&mut v, &output, "shower-active")?;
            active_saved = true;
        }
        let s = v.session.as_ref().unwrap();
        if let Some(r) = s
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .find(|r| r.status == 1 && !prior_dialogs.contains(&r.id))
        {
            next_dialog = Some(r.clone());
            stop = "next_original_instruction";
            break;
        }
        if s.simulation.vm.tick >= start + 2000 {
            stop = "tick_budget";
            break;
        }
        if i % 100 == 0 {
            println!(
                "shower slice{i}: tick{} active{active} hygiene{}",
                s.simulation.vm.tick,
                hygiene(&v)
            );
        }
    }
    v.refresh()?;
    v.args.save = save;
    v.save()?;
    let frozen = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("shower-result.png"), v.view())?;
    ensure!(frozen == serde_json::to_vec(&v.session.as_ref().unwrap().save())?);
    ensure!(std::fs::read(&source)? == original);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "start": start, "tick": v.session.as_ref().unwrap().simulation.vm.tick,
            "actor": actor, "action": action, "stop": stop, "error": error,
            "hygiene_before": before_hygiene, "hygiene_after": hygiene(&v),
            "active_restored": active_saved, "original_shower_behavior_seen": behavior_seen,
            "next_dialog": next_dialog, "source_unchanged": true,
            "render_preserves_session": true, "native_execution": false
        }))?,
    )?;
    ensure!(error.is_none(), "shower continuation boundary: {error:?}");
    ensure!(active_saved, "shower did not become active");
    ensure!(
        next_dialog.is_some(),
        "tutorial did not reach its next instruction"
    );
    super::person_tests::checkpoint(&mut v, &output, "next-instruction")?;
    Ok(())
}

#[test]
#[ignore = "private original Newbie checkpoint; headless Iced Metal rendering"]
fn original_shower_preview_displays_restores_and_accepts_player_ok() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_SHOWER_PREVIEW_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let source = repo.join("cache/newbie-object-preview-checked/constructed.json");
    let original = std::fs::read(&source)?;
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
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let start = v.session.as_ref().unwrap().simulation.vm.tick;
    let id = v
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .find(|r| r.origin.bhav == 4125 && r.origin.instruction == 12)
        .context("shower dialog absent")?
        .id;
    for _ in 0..1000 {
        if v.session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .any(|r| r.id == id && r.status == 1)
        {
            break;
        }
        let _ = v.update(Message::Step);
        ensure!(v.error.is_none(), "{:?}", v.error);
    }
    ensure!(
        v.session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .any(|r| r.id == id && r.status == 1)
    );
    let record = v
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .find(|r| r.id == id)
        .unwrap()
        .clone();
    ensure!(record.portrait.as_ref().unwrap().rect == [0, 0, 120, 120]);
    super::person_tests::checkpoint(&mut v, &output, "shower-dialog")?;
    let frozen = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("shower-dialog.png"), v.view())?;
    ensure!(
        !record.modal(),
        "original shower instruction unexpectedly modal"
    );
    ensure!(
        frozen == serde_json::to_vec(&v.session.as_ref().unwrap().save())?,
        "rendering the instruction advanced simulation"
    );
    let _ = v.update(Message::Dialog(id, 2));
    ensure!(v.error.is_none(), "{:?}", v.error);
    super::person_tests::checkpoint(&mut v, &output, "shower-answered")?;
    // Continue through real front-end services; do not auto-answer any later
    // modal or substitute a player interaction. Report the actual next stop.
    let mut stop = "budget";
    for _ in 0..2000 {
        let _ = v.update(Message::Step);
        if v.error.is_some() {
            stop = "unsupported_service";
            break;
        }
        let session = v.session.as_ref().unwrap();
        if session.simulation.vm.world.dialogs.modal.is_some() {
            stop = "next_original_modal";
            break;
        }
        if session.simulation.vm.tick >= start + 64 {
            stop = "64_ticks";
            break;
        }
    }
    v.args.save = save;
    v.save()?;
    render_headless_element(
        &root,
        &output.join("after-shower-instruction.png"),
        v.view(),
    )?;
    ensure!(std::fs::read(&source)? == original);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "start":start,"tick":v.session.as_ref().unwrap().simulation.vm.tick,"stop":stop,"error":v.error,
            "record":record,"construction_published":true,"render_preserves_session":true,"original_instruction_nonmodal":true,
            "full_client_session_device_restore":true,"original_player_ok":true,
            "source_unchanged":true,"native_pixel_equivalence":false,"native_execution":false,
            "shower_interaction_completed":false
        }))?,
    )?;
    ensure!(v.error.is_none(), "continuation boundary: {:?}", v.error);
    Ok(())
}
