use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, original_house::OriginalHouse};
use sims_runtime::original_session::{Saved, Session};
use std::path::Path;

#[test]
#[ignore = "private original Newbie checkpoint and assets; software rendering only"]
fn original_ground_factory_placement_menu_queue_and_cancel_resume() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_GROUND_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace ground evidence")?;
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let input: serde_json::Value = serde_json::from_slice(&std::fs::read(
        repo.join("cache/newbie-person-button-click-cold/session.json"),
    )?)?;
    let initial: Saved = serde_json::from_value(input["session"].clone())?;
    let mut reports = vec![];
    let mut outcomes = vec![];
    for budget in [10000, 1] {
        let mut s = Session::restore(&index, initial.clone())?;
        let mut device =
            Device::restore(&s, serde_json::from_value(input["camera_device"].clone())?)?;
        let dialogs = &s.simulation.vm.world.dialogs;
        let dialog = dialogs
            .records
            .iter()
            .find(|r| r.status == 1 && r.origin.bhav == 4125 && r.origin.instruction == 3)
            .context("Go Here prompt absent")?;
        ensure!(dialogs.modal.is_none() && dialog.origin.bhav == 4125);
        // This is the original nonmodal instruction panel. Keep it visible:
        // the user's ground command, not an invented OK, advances the lesson.
        let actor = s
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            .unwrap();
        let at = s.simulation.vm.world.positions[&actor];
        let point = [at.x + 32, at.y];
        let old_destination = s.simulation.vm.world.objects[&139].clone();
        let before_ids: Vec<_> = s.simulation.vm.world.objects.keys().copied().collect();
        let start = s.simulation.vm.tick;
        let before_pixels = if budget == 1 {
            let camera = OriginalHouse::native_camera(&s)?;
            Some(sims_render::software::render(
                &OriginalHouse::default().scene(&s, camera)?,
            ))
        } else {
            None
        };
        s.begin_ground_input(point, at.level)?;
        let mut phases = std::collections::BTreeSet::new();
        let mut snapshots = 0;
        let mut slices = 0;
        loop {
            slices += 1;
            ensure!(slices < 10000, "ground continuation budget exceeded");
            let frozen = serde_json::to_vec(&s.save())?;
            ensure!(s.step().is_err());
            if slices == 1 {
                ensure!(s.begin_selection(Some(actor)).is_err());
                ensure!(s.begin_ui_wait(None).is_err());
                ensure!(s.begin_live_interface_update().is_err());
                ensure!(s.begin_panel_toggle(1).is_err());
                ensure!(s.begin_ui_portrait().is_err());
                ensure!(
                    s.begin_camera_change(
                        sims_runtime::original_session::camera_command::Change::Floor(at.level),
                        0
                    )
                    .is_err()
                );
            }
            ensure!(
                frozen == serde_json::to_vec(&s.save())?,
                "blocked tick changed session"
            );
            let state = s.host.presentation.ground_input.as_ref().unwrap();
            let mut phase = serde_json::to_value(state)?["phase"]
                .as_str()
                .unwrap()
                .to_owned();
            if matches!(
                s.simulation.pending_native_selection(),
                Some((
                    sims_sim::lot_selection::Request::CreationNotification(_),
                    true
                ))
            ) {
                phase = "FactoryNotification".into();
            }
            if phases.insert(phase.clone()) && budget == 1 {
                let saved = s.save();
                let file = output.join(format!("pending-{phase}.json"));
                std::fs::write(&file, serde_json::to_vec(&saved)?)?;
                s = Session::restore(&index, serde_json::from_slice(&std::fs::read(file)?)?)?;
                ensure!(serde_json::to_value(s.save())? == serde_json::to_value(&saved)?);
                snapshots += 1;
                println!("restored {phase} at tick {}", s.simulation.vm.tick);
            }
            if s.host.presentation.ground_input.as_ref().unwrap().ready() {
                break;
            }
            ensure!(
                !s.host
                    .presentation
                    .ground_input
                    .as_ref()
                    .unwrap()
                    .complete(),
                "ground placement/menu rejected"
            );
            if s.house_notification_pending() {
                device.advance_notification(&mut s, budget)?;
            } else {
                let camera = s.retained_camera_input()?;
                s.advance_ground_input(camera, budget)
                    .with_context(|| format!("phase {phase}, slice {slices}"))?;
            }
        }
        ensure!(s.simulation.vm.tick == start);
        let state = s.host.presentation.ground_input.as_ref().unwrap();
        let target = state.target().unwrap();
        ensure!(!before_ids.contains(&target) && target != 139);
        ensure!(s.simulation.vm.world.objects[&139] == old_destination);
        ensure!(s.simulation.vm.world.objects[&target].attributes[0] == 0);
        let choice = state
            .actions()
            .iter()
            .position(|a| a.menu_id == 2)
            .context("fresh Go Here unavailable")?;
        let actions = state.actions().to_vec();
        let menu = s.save();
        std::fs::write(
            output.join(format!("budget-{budget}-menu.json")),
            serde_json::to_vec(&menu)?,
        )?;
        if budget == 1 {
            let camera = OriginalHouse::native_camera(&s)?;
            let scene = OriginalHouse::default().scene(&s, camera)?;
            let pixels = sims_render::software::render(&scene);
            ensure!(
                Some(&pixels) == before_pixels.as_ref(),
                "hidden destination changed room pixels"
            );
            pixels.save(output.join("fresh-destination-room.png"))?;
        }
        let (serial, accepted) = s.choose_ground_action(choice)?;
        ensure!(accepted && !s.ground_input_pending());
        ensure!(s.simulation.vm.world.objects.contains_key(&target));
        let queued = &s.simulation.vm.world.actions.as_ref().unwrap().actors[&actor].queue;
        let queued_json = serde_json::to_value(queued)?;
        std::fs::write(
            output.join(format!("budget-{budget}-queued.json")),
            serde_json::to_vec(&s.save())?,
        )?;
        let restored = Session::restore(&index, s.save())?;
        ensure!(serde_json::to_value(restored.save())? == serde_json::to_value(s.save())?);
        let accepted_save = s.save();
        // Independent cancellation branch retains the native destructor owner.
        s = Session::restore(&index, menu.clone())?;
        s.cancel_ground_input()?;
        let pending_cancel = s.save();
        s = Session::restore(&index, pending_cancel.clone())?;
        ensure!(serde_json::to_value(s.save())? == serde_json::to_value(&pending_cancel)?);
        let mut cleanup_slices = 0;
        while !s
            .host
            .presentation
            .ground_input
            .as_ref()
            .unwrap()
            .complete()
        {
            cleanup_slices += 1;
            ensure!(cleanup_slices < 10000, "ground cleanup budget exceeded");
            if s.house_notification_pending() {
                device.advance_notification(&mut s, budget)?;
            } else {
                let camera = s.retained_camera_input()?;
                s.advance_ground_input(camera, budget)
                    .context("cancel cleanup")?;
            }
        }
        s.finish_ground_input()?;
        ensure!(!s.simulation.vm.world.objects.contains_key(&target));
        ensure!(s.simulation.vm.world.objects[&139] == old_destination);
        ensure!(s.simulation.vm.tick == start);
        let restored = Session::restore(&index, s.save())?;
        ensure!(serde_json::to_value(restored.save())? == serde_json::to_value(s.save())?);
        std::fs::write(
            output.join(format!("budget-{budget}-cancelled.json")),
            serde_json::to_vec(&s.save())?,
        )?;
        reports.push(
            serde_json::json!({"budget":budget,"point":point,"floor":at.level,"actor":actor,
            "target":target,"tick":start,"slices":slices,"snapshots":snapshots,"phases":phases,
            "actions":actions,"serial":serial,"queued":queued_json,"cleanup_slices":cleanup_slices,
            "original_destination_139_unchanged":true,"blocked_ticks_unchanged":true,"overlapping_ui_inputs_rejected":true,"hidden_helper_pixels_unchanged":budget == 1}),
        );
        outcomes.push(serde_json::to_value((menu, accepted_save, s.save()))?);
    }
    ensure!(
        outcomes[0] == outcomes[1],
        "instruction budget changed final session state"
    );
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "runs":reports,"slice_and_resume_results_identical":true,"native_execution_this_turn":false,
            "scope":"Original fresh helper factory/E2, same-room placement, live Go Here menu, native queue insertion and cancellation. No viewport click or subsequent household tick claimed."
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private original ground menu checkpoint; explicit negative fixture"]
fn empty_ground_menu_retains_unsupported_tile_fallback_and_can_cancel() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_GROUND_EMPTY_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace fallback evidence")?;
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let saved: Saved = serde_json::from_slice(&std::fs::read(
        repo.join("cache/newbie-ground-input-nonmodal/budget-1-menu.json"),
    )?)?;
    let mut s = Session::restore(&index, saved)?;
    let state = s.host.presentation.ground_input.as_ref().unwrap();
    let target = state.target().unwrap();
    let actor = state.actor();
    let tick = s.simulation.vm.tick;
    // Explicit negative fixture: a used helper makes original BHAV4109 reject.
    // This is not a naturally reached tutorial checkpoint or input command.
    // The historical saved destination139 and original files are untouched.
    s.simulation
        .vm
        .world
        .objects
        .get_mut(&target)
        .unwrap()
        .attributes[0] = 1;
    s.simulation
        .vm
        .begin_player_menu(&s.program, actor, target)
        .map_err(anyhow::Error::msg)?;
    let mut fixture = serde_json::to_value(s.save())?;
    fixture["view"]["ground_input"]["phase"] = serde_json::json!("Query");
    fixture["view"]["ground_input"]["actions"] = serde_json::json!([]);
    s = Session::restore(&index, serde_json::from_value(fixture)?)?;
    let camera = s.retained_camera_input()?;
    let error = s
        .advance_ground_input(camera, 1000)
        .unwrap_err()
        .to_string();
    ensure!(error.contains("same-tile fallback"), "{error}");
    ensure!(s.simulation.vm.player_menu.is_none());
    ensure!(s.simulation.vm.world.objects.contains_key(&target));
    let pending = s.save();
    std::fs::write(
        output.join("pending-fallback.json"),
        serde_json::to_vec(&pending)?,
    )?;
    s = Session::restore(&index, pending.clone())?;
    ensure!(serde_json::to_value(s.save())? == serde_json::to_value(&pending)?);
    ensure!(s.step().is_err());
    ensure!(s.advance_ground_input(camera, 1).is_err());
    ensure!(serde_json::to_value(s.save())? == serde_json::to_value(&pending)?);
    s.cancel_ground_input()?;
    for i in 0..1000 {
        ensure!(
            !s.house_notification_pending(),
            "negative fixture requires additional notification service"
        );
        s.advance_ground_input(camera, 1)?;
        if s.host
            .presentation
            .ground_input
            .as_ref()
            .unwrap()
            .complete()
        {
            break;
        }
        ensure!(i < 999);
    }
    s.finish_ground_input()?;
    ensure!(!s.simulation.vm.world.objects.contains_key(&target));
    ensure!(s.simulation.vm.tick == tick);
    let restored = Session::restore(&index, s.save())?;
    ensure!(serde_json::to_value(restored.save())? == serde_json::to_value(s.save())?);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "synthetic_negative_fixture":true,"original_bhav":4109,"used_helper":target,
            "boundary":error,"tick":tick,"empty_query_preserved_helper":true,
            "saved_boundary_restored":true,"repeated_blocked_calls_unchanged":true,
            "explicit_cancel_removed_helper":true,"native_execution_this_turn":false
        }))?,
    )?;
    Ok(())
}
