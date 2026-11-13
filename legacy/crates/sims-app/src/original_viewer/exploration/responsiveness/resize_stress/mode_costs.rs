//! Stage attribution only: the ordinary event/handler/reference path is shared
//! with resize_stress. No latency threshold or original parity claim.
use super::*;

#[path = "engine_costs.rs"]
mod engine_costs;

fn resources(app: &Application) -> Result<Value> {
    let scene = app.viewer.scene.as_ref().context("scene absent")?;
    let client = app.viewer.client.as_ref().context("client absent")?;
    let icons = app.viewer.icons.as_ref().context("icons absent")?;
    let residents: Vec<_> = client
        .buttons()
        .into_iter()
        .flat_map(|b| &b.items)
        .map(|b| {
            json!({"neighbor":b.neighbor,"native_image":b.image,
            "iced_image":icons.resident(b.neighbor).map(|h|format!("{:?}",h.id()))})
        })
        .collect();
    Ok(
        json!({"scene_size":[scene.width,scene.height],"vertices":scene.vertices.len(),
        "draws":scene.draws.len(),"texture_count":scene.textures.len(),
        "texture_owner_process_address":Arc::as_ptr(&scene.textures) as usize,
        "texture_rgba_bytes":scene.textures.iter().map(|t|t.rgba.len()).sum::<usize>(),
        "texture_depth_bytes":scene.textures.iter().map(|t|t.depth.len()).sum::<usize>(),
        "thumbnail_owner_present":app.viewer.thumbnails.is_some(),"residents":residents}),
    )
}
fn sampled<T>(run: &mut Run, name: &str, work: impl FnOnce(&mut Run) -> Result<T>) -> Result<T> {
    let before = full(&run.app)?;
    let resources_before = resources(&run.app)?;
    let first_row = run.rows.len();
    ensure!(run.app.viewer.frame_profile.is_none(), "nested profile");
    run.app.viewer.frame_profile = Some(FrameProfile::default());
    let result = work(run);
    let stages = run
        .app
        .viewer
        .frame_profile
        .take()
        .context("profile absent")?;
    let after = full(&run.app)?;
    run.rows.push(json!({"cost_operation":name,"first_journal_row":first_row,
        "last_journal_row":run.rows.len(),"stages":stages,"before":before,"after":after,
        "resources_before":resources_before,"resources_after":resources(&run.app)?,
        "error":result.as_ref().err().map(|e|format!("{e:#}")),
        "timing_scope":"Only existing cfg(test) stage spans; audit hashing/reference/cold restore excluded from spans"}));
    run.journal()?;
    ensure!(
        after.tick == before.tick && after.rng == before.rng,
        "presentation operation advanced tick/RNG: {name}"
    );
    result
}
fn control(run: &mut Run, control: Control, label: &str) -> Result<()> {
    let point = sampled(run, &format!("{label}: cursor/control discovery"), |r| {
        r.point(control)
    })?;
    sampled(run, label, |r| {
        r.step(Step {
            label: label.into(),
            events: vec![Event::moved(point), Event::Down, Event::Up],
            intent: Intent::Control(control),
            flush: true,
        })
    })
}
fn menu(run: &mut Run, label: &str) -> Result<()> {
    let (object, point, pixel) = run.object_point()?;
    sampled(run, label, |r| {
        r.step(Step {
            label: label.into(),
            events: vec![Event::moved(point), Event::Down, Event::Up],
            intent: Intent::Object {
                object,
                pixel: pixel.map(f32::to_bits),
            },
            flush: true,
        })
    })?;
    ensure!(
        run.app
            .menu()
            .is_some_and(|m| m.target() == object && !m.actions().is_empty()),
        "actual menu not ready"
    );
    Ok(())
}

#[test]
#[ignore = "real production Loading view then private native-owned HUD; offscreen layout attribution"]
fn loading_view_then_native_hud_layout_costs() -> Result<()> {
    run_worker(loading_layout)
}
fn loading_layout() -> Result<()> {
    let out = output()?;
    let app = load_native(&out)?;
    let before = full(&app)?;
    let mut ui = Input::new()?;
    // Actual production constructor/view; dropping the unpolled Task performs
    // no second house load. No native owner or simulation state is fabricated.
    let (loading, task) = Viewer::new(app.viewer.args.clone());
    drop(task);
    ensure!(loading.session.is_none());
    let (loading_batch, first_layout) = ui.profiled_batch(&loading, &[]);
    ensure!(loading_batch.messages.is_empty() && loading.session.is_none());
    ui.reset();
    let (loaded_batch, loaded_layout) = ui.profiled_batch(&app.viewer, &[]);
    ensure!(loaded_batch.messages.is_empty() && full(&app)? == before);
    let (repeat_batch, repeat_layout) = ui.profiled_batch(&app.viewer, &[]);
    ensure!(repeat_batch.messages.is_empty() && full(&app)? == before);
    write_json(
        &out.join("result.json"),
        &json!({"passed":true,"before":before,"after":full(&app)?,
        "cold_production_loading_view":first_layout,"loaded_hud_after_loading_view":loaded_layout,
        "repeated_loaded_hud":repeat_layout,"completed_ticks":0,
        "scope":"Actual Iced constructor/layout only; no app messages or original widget/pixel/tick parity claim",
        "limit":"Independent process/OS font state not forcibly reset; diagnostics without wall-time thresholds"}),
    )
}

#[test]
#[ignore = "verified private native-owned Newbie; actual Iced stage attribution, no portable timing ceilings"]
fn native_owned_first_use_and_repeated_mode_costs() -> Result<()> {
    run_worker(campaign)
}
fn campaign() -> Result<()> {
    let out = output()?;
    let app = load_native(&out)?;
    let initial = full(&app)?;
    let reference = app.fork()?;
    let mut run = Run {
        app,
        reference,
        ui: Input::new()?,
        out,
        steps: vec![],
        rows: vec![],
        pending: vec![],
        intended: vec![],
        initial,
    };
    let result = (|| -> Result<()> {
        sampled(
            &mut run,
            "initial actual Iced layout/native cursor/drain",
            |r| r.hud(),
        )?;
        run.frame("first-native-frame")?;
        for size in [[800, 600], [1000, 740], [1320, 900]] {
            sampled(&mut run, &format!("resize to {size:?}"), |r| {
                r.events("deliberate size change", vec![Event::Resize(size)], true)
            })?;
            for repetition in 0..2 {
                let prefix = format!("{size:?}/repeat{repetition}");
                let resident = run.app.viewer.resident;
                control(
                    &mut run,
                    Control::Resident(resident),
                    &format!("{prefix}/resident"),
                )?;
                menu(&mut run, &format!("{prefix}/menu-open"))?;
                if repetition == 0 {
                    run.frame(&format!("{}x{}-menu", size[0], size[1]))?;
                }
                control(
                    &mut run,
                    Control::ObjectCancel,
                    &format!("{prefix}/menu-cancel"),
                )?;
                ensure!(run.app.menu().is_none());
                control(&mut run, Control::Buy, &format!("{prefix}/Buy"))?;
                ensure!(mode(&run.app)? == 1);
                control(&mut run, Control::Live, &format!("{prefix}/Live"))?;
                ensure!(mode(&run.app)? == 2);
                run.frame(&format!("{}x{}-repeat{repetition}-Live", size[0], size[1]))?;
            }
        }
        safe(&run.app)?;
        run.checkpoint("final-safe")?;
        let final_state = full(&run.app)?;
        ensure!(final_state.tick == run.initial.tick && final_state.rng == run.initial.rng);
        write_json(
            &run.out.join("result.json"),
            &json!({"passed":true,
            "sizes":3,"repetitions_each":2,"batches":run.steps.len(),
            "initial":run.initial,"final":final_state,"completed_ticks":0,
            "ordinary_handler_reference":true,"cold_equal":true,
            "scope":"Scoped stage attribution; no original visual/tick parity or uncontrolled speedup claim",
            "first_vs_repeat":"Process/OS caches are not reset between sizes; first at each size is not a cold installation"}),
        )?;
        Ok(())
    })();
    if let Err(error) = &result {
        run.journal()?;
        write_json(
            &run.out.join("first-failure.json"),
            &json!({"error":format!("{error:#}"),
            "actual":full(&run.app)?,"reference":full(&run.reference)?,"completed_batches":run.steps.len()}),
        )?;
        std::fs::write(run.out.join("failure.json"), run.app.checkpoint()?)?;
        std::fs::write(
            run.out.join("failure-reference.json"),
            run.reference.checkpoint()?,
        )?;
    }
    result
}
