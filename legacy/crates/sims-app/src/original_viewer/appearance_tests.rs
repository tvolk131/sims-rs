//! Original Mashuga pool checkpoint. Tests explicitly enter resource
//! preparation; plain refresh, Iced view and software draw remain observers.
use super::*;
use clap::Parser;

fn body(session: &Session, id: i16) -> (&str, &str) {
    let records = &session.host.characters[&id].avatar.records;
    let entry = &records.entries[records.body[0]];
    (&entry.name, &entry.texture)
}

#[test]
#[ignore = "actual Mashuga pool save; SIMS_DRAW_APPEARANCE_OUTPUT requires a fresh cache directory"]
fn native_draw_preparation_refreshes_swimwear_without_advancing_the_simulation() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let checkpoint = repo.join("cache/sale-20261022-seventh/mashuga/sold-returned-live.json");
    let output = repo.join(std::env::var("SIMS_DRAW_APPEARANCE_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to overwrite appearance evidence")?;
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData2/Houses/House09.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        checkpoint.to_str().unwrap(),
        "--save",
        output.join("restored.json").to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(ready)?;
    let start = viewer.session.as_ref().unwrap().save();
    ensure!(
        start.view.person_draw.is_none(),
        "old managed snapshot fabricated a draw owner"
    );
    ensure!(start.simulation.state.vm.tick == 180174);
    ensure!(
        start.simulation.state.vm.world.objects[&196]
            .person
            .as_ref()
            .unwrap()[&8]
            == 2
    );
    ensure!(
        body(viewer.session.as_ref().unwrap(), 196) == ("b004mafat_01", "BODY=b004mafatlgt_lizrd")
    );
    ensure!(!start.simulation.state.characters[&196].profile_dirty);
    let scene = viewer.device.as_ref().unwrap().scene();
    let before_pixels = sims_render::software::render(scene);
    image::save_buffer(
        output.join("before.png"),
        &before_pixels,
        scene.width,
        scene.height,
        image::ColorType::Rgba8,
    )?;
    let before = serde_json::to_vec(&start)?;
    viewer.refresh()?;
    render_headless_element(&root, &output.join("before-iced.png"), viewer.view())?;
    ensure!(
        before == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?,
        "drawing changed pending appearance"
    );

    // Respect an existing animator owner, even with outfit2 and stale bindings.
    let index = Index::scan(&root)?;
    let mut busy = Session::restore(&index, start.clone())?;
    let object = busy.simulation.vm.world.objects[&196].clone();
    let update_tick = busy.simulation.characters[&196].evaluated_tick + 1;
    busy.simulation
        .characters
        .get_mut(&196)
        .unwrap()
        .begin_ordinary_update(&object, update_tick, true, 0)
        .map_err(anyhow::Error::msg)?;
    let busy_before = serde_json::to_vec(&busy.save())?;
    ensure!(!busy.person_draw_preparation_ready());
    ensure!(busy.begin_person_draw_preparation(vec![196]).is_err());
    ensure!(
        busy.host
            .prepare_draw_appearance(&mut busy.simulation, &busy.program, 196)
            .is_err()
    );
    ensure!(busy_before == serde_json::to_vec(&busy.save())?);

    // Missing assets and unsupported wardrobe branches remain actual failures.
    let mut failure_messages = vec![];
    for missing in [false, true] {
        let mut failed = Session::restore(&index, start.clone())?;
        if missing {
            let d = failed.simulation.vm.world.objects[&196]
                .definition
                .clone()
                .unwrap();
            failed
                .simulation
                .vm
                .world
                .body_profiles
                .get_mut(&d)
                .map_err(anyhow::Error::msg)?[16] =
                "nmfat_01,BODY=missing-draw-regression-texture".into();
        } else {
            failed
                .simulation
                .vm
                .world
                .objects
                .get_mut(&196)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
                .insert(8, 6);
        }
        failed.begin_person_draw_preparation(vec![196])?;
        let pending = serde_json::to_vec(&failed.save())?;
        eprintln!(
            "appearance failure injection: missing_texture={missing}, outfit={}, cache={:?}",
            failed.simulation.vm.world.objects[&196]
                .person
                .as_ref()
                .unwrap()[&8],
            failed.host.characters[&196]
                .appearance_owner
                .selection
                .cached
        );
        let error = failed.advance_person_draw_preparation(1).unwrap_err();
        eprintln!("expected failure: {error:#}");
        ensure!(
            pending == serde_json::to_vec(&failed.save())?,
            "failed appearance partially committed"
        );
        failure_messages.push(format!("{error:#}"));
    }
    // Native+d8 is a separate full-reload branch; consuming it also performs
    // its normal profile/record lifecycle before the unconditional selector.
    let mut dirty = Session::restore(&index, start.clone())?;
    dirty
        .simulation
        .characters
        .get_mut(&196)
        .unwrap()
        .profile_dirty = true;
    dirty.begin_person_draw_preparation(vec![196])?;
    while !dirty.advance_person_draw_preparation(1)? {}
    ensure!(!dirty.simulation.characters[&196].profile_dirty);
    ensure!(body(&dirty, 196) == ("nmfat_01", "BODY=umfatlgt_lguard01"));
    ensure!(
        (
            dirty.simulation.vm.tick,
            dirty.simulation.vm.rng_state,
            dirty.simulation.vm.lot_rng_state,
            dirty.simulation.vm.crt_rng_state
        ) == (
            start.simulation.state.vm.tick,
            start.simulation.state.vm.rng_state,
            start.simulation.state.vm.lot_rng_state,
            start.simulation.state.vm.crt_rng_state
        )
    );
    let mut membership = Session::restore(&index, start.clone())?;
    let presenter = membership.host.presentation.presenter.as_mut().unwrap();
    presenter.lists.ordinary.retain(|id| *id != 196);
    presenter.lists.special.push(196);
    ensure!(
        membership.person_draw_objects()? == vec![196, 195],
        "special-list person omitted"
    );

    // Keep the existing portrait caller's force1 semantics on a current
    // managed checkpoint (the older portrait fixture predates native_scheduler).
    let mut portrait = Session::restore(&index, start.clone())?;
    portrait
        .host
        .characters
        .get_mut(&196)
        .unwrap()
        .appearance_owner
        .selection
        .cached = [2, 0, 0, 0];
    portrait
        .host
        .characters
        .get_mut(&196)
        .unwrap()
        .appearance_owner
        .selection
        .tint = [1, 2, 3];
    let portrait_failed = serde_json::to_vec(&portrait.save())?;
    ensure!(
        portrait
            .host
            .prepare_portrait(&mut portrait.simulation, &portrait.program, 196)
            .is_err(),
        "forced portrait silently accepted unsupported tint on matching caches"
    );
    ensure!(portrait_failed == serde_json::to_vec(&portrait.save())?);
    portrait
        .host
        .characters
        .get_mut(&196)
        .unwrap()
        .appearance_owner
        .selection
        .tint = [0; 3];
    let head = portrait
        .host
        .prepare_portrait(&mut portrait.simulation, &portrait.program, 196)?;
    ensure!(!head.is_empty());
    ensure!(body(&portrait, 196) == ("nmfat_01", "BODY=umfatlgt_lguard01"));

    // A real pending cursor survives cold restoration and blocks simulation.
    let mut resumed = Session::restore(&index, start.clone())?;
    ensure!(resumed.person_draw_objects()? == vec![196, 195]);
    resumed.begin_person_draw_preparation(resumed.person_draw_objects()?)?;
    ensure!(!resumed.advance_person_draw_preparation(1)?);
    let pending = resumed.save();
    std::fs::write(
        output.join("pending-person.json"),
        serde_json::to_vec(&pending)?,
    )?;
    resumed = Session::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&pending)?)?,
    )?;
    ensure!(serde_json::to_vec(&resumed.save())? == serde_json::to_vec(&pending)?);
    let pending_before = serde_json::to_vec(&resumed.save())?;
    ensure!(resumed.step().is_err());
    ensure!(pending_before == serde_json::to_vec(&resumed.save())?);
    let mut invalid = serde_json::to_value(&pending)?;
    invalid["view"]["person_draw"]["tick"] = serde_json::json!(180173);
    ensure!(Session::restore(&index, serde_json::from_value(invalid)?).is_err());
    // Cold restore the actual frontend bundle at a partial draw boundary.
    // Playing/native-clock input must not overtake this retained caller.
    let mut pending_bundle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&checkpoint)?)?;
    pending_bundle["session"] = serde_json::to_value(&pending)?;
    let pending_path = output.join("pending-viewer.json");
    std::fs::write(&pending_path, serde_json::to_vec(&pending_bundle)?)?;
    let pending_args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData2/Houses/House09.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        pending_path.to_str().unwrap(),
        "--save",
        output.join("pending-unused.json").to_str().unwrap(),
    ]);
    let pending_ready = Arc::new(Ready::load(&pending_args)?);
    let (mut pending_viewer, _) = Viewer::new(pending_args);
    pending_viewer.install(pending_ready)?;
    pending_viewer.playing = true; // Replacement frontend policy only.
    let held_vm = pending_viewer
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .clone();
    let held_clock = pending_viewer
        .session
        .as_ref()
        .unwrap()
        .host
        .presentation
        .clock
        .clone();
    let _ = pending_viewer.update(Message::Frame(Instant::now()));
    ensure!(pending_viewer.error.is_none(), "{:?}", pending_viewer.error);
    ensure!(
        !pending_viewer
            .session
            .as_ref()
            .unwrap()
            .person_draw_preparation_pending()
    );
    ensure!(pending_viewer.session.as_ref().unwrap().simulation.vm == held_vm);
    ensure!(
        pending_viewer
            .session
            .as_ref()
            .unwrap()
            .host
            .presentation
            .clock
            == held_clock
    );
    ensure!(
        pending_viewer.last.is_none(),
        "elapsed application frame overtook preparation"
    );
    while !resumed.advance_person_draw_preparation(1)? {}
    ensure!(body(&resumed, 196) == ("nmfat_01", "BODY=umfatlgt_lguard01"));

    // The paused-idle Frame early return must run the same explicit stage.
    let _ = viewer.update(Message::Frame(Instant::now()));
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    let session = viewer.session.as_ref().unwrap();
    ensure!(body(session, 196) == ("nmfat_01", "BODY=umfatlgt_lguard01"));
    ensure!(
        session.simulation.vm == start.simulation.state.vm,
        "appearance changed VM execution/RNG/action state"
    );
    let mut expected = start.simulation.state.clone();
    expected.characters = session.simulation.characters.clone();
    ensure!(
        session.simulation == expected,
        "appearance changed non-character simulation state"
    );
    for (&id, resident) in &session.simulation.characters {
        let before_resident = &start.simulation.state.characters[&id];
        let mut expected_resident = before_resident.clone();
        // 599427/599469 attachment lifetime invalidates mesh generations;
        // 58a178 writes the shared placement+d9 dirty flag. Nothing else may
        // change, including every animation clock/continuation/last-update.
        expected_resident.controls.geometry_generation = resident.controls.geometry_generation;
        expected_resident.character.placement.dirty = resident.character.placement.dirty;
        ensure!(
            *resident == expected_resident,
            "appearance progressed resident state beyond binding invalidation"
        );
        ensure!(
            resident
                .character
                .displayed_pose()
                .map_err(anyhow::Error::msg)?
                == before_resident
                    .character
                    .displayed_pose()
                    .map_err(anyhow::Error::msg)?
        );
    }
    ensure!(
        session.host.snapshot() == resumed.host.snapshot(),
        "warm and cold-boundary preparation differ"
    );
    let prepared = session.save();
    let prepared_bytes = serde_json::to_vec(&prepared)?;
    viewer.refresh()?;
    render_headless_element(&root, &output.join("after-iced.png"), viewer.view())?;
    ensure!(prepared_bytes == serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?);
    let scene = viewer.device.as_ref().unwrap().scene();
    let after_pixels = sims_render::software::render(scene);
    image::save_buffer(
        output.join("after.png"),
        &after_pixels,
        scene.width,
        scene.height,
        image::ColorType::Rgba8,
    )?;
    ensure!(
        before_pixels != after_pixels,
        "appearance did not change visible pixels"
    );
    let mut cold = Session::restore(&index, serde_json::from_slice(&prepared_bytes)?)?;
    let cold_before = serde_json::to_vec(&cold.save())?;
    cold.begin_person_draw_preparation(vec![195, 196])?;
    while !cold.advance_person_draw_preparation(1)? {}
    ensure!(
        cold_before == serde_json::to_vec(&cold.save())?,
        "cached selection replayed binding callbacks"
    );
    let mut renderer = OriginalHouse::default();
    let camera = OriginalHouse::native_camera(&cold)?;
    let cold_scene = renderer.scene(&cold, camera)?;
    ensure!(
        after_pixels == sims_render::software::render(&cold_scene),
        "cold-restored pixels differ"
    );
    std::fs::write(output.join("prepared-session.json"), &prepared_bytes)?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "checkpoint":checkpoint,"tick":session_tick(&viewer),"object":196,"outfit":2,
            "before_body":["b004mafat_01","BODY=b004mafatlgt_lizrd"],
            "after_body":["nmfat_01","BODY=umfatlgt_lguard01"],
            "source":"590ca9 clears+d8 onlyifdirty;590cb8 always58a178(force0); outfit2 emptyentry32 fallbackentry17",
            "vm_rng_actions_unchanged":true,"non_character_simulation_unchanged":true,
            "resident_unchanged_except":["controls.geometry_generation","character.placement.dirty"],
        "all_animation_clocks_continuations_evaluated_ticks_unchanged":true,
        "controls_and_pose_unchanged_except_binding_generation":true,
        "playing_viewer_cold_cursor_returns_before_ui_clock":true,"pending_owner_cold_restore":true,
            "pure_draw_complete_state_unchanged":true,"cached_preparation_complete_state_unchanged":true,
            "warm_cold_pixels_equal":true,"failure_messages":failure_messages,
            "forced_portrait_matching_cache_and_transaction_unchanged":true,
        "native_gameplay_pixel_parity":false,"scope":"Original component semantics and captured Rust state; no native guest run or source save mutation"
        }))?,
    )?;
    Ok(())
}

fn session_tick(viewer: &Viewer) -> u64 {
    viewer.session.as_ref().unwrap().simulation.vm.tick
}
