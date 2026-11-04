//! Display-scale coverage is independent of logical window-size matrices.
//! GPU images are internal regression evidence, not original-game goldens.
use super::super::super::super::headless_input::physical_size;
use super::*;

/// The pinned iced_winit conversion divides the physical f64 cursor by its
/// scale, then narrows to f32. No real OS event delivery is claimed here.
fn cursor(point: [f32; 2], scale: f32) -> Result<([f64; 2], [f32; 2])> {
    ensure!(scale.is_finite() && scale > 0.);
    ensure!(point.iter().all(|v| v.is_finite()));
    let physical = point.map(|v| f64::from(v) * f64::from(scale));
    let logical = physical.map(|v| (v / f64::from(scale)) as f32);
    Ok((physical, logical))
}

#[test]
fn display_scale_dimensions_and_logical_pointer_domain() -> Result<()> {
    for (logical, scale, expected) in [
        ([800, 600], 1., [800, 600]),
        ([800, 600], 1.5, [1200, 900]),
        ([800, 600], 2., [1600, 1200]),
        ([801, 601], 1.5, [1202, 902]),
    ] {
        let size = physical_size(logical, scale)?;
        ensure!([size.width, size.height] == expected);
        for point in [[0., 0.], [99.25, 470.5], [799.75, 599.5]] {
            let (physical, returned) = cursor(point, scale)?;
            ensure!(returned == point);
            ensure!(physical == point.map(|v| f64::from(v) * f64::from(scale)));
        }
    }
    for scale in [0., -1., f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        ensure!(physical_size([800, 600], scale).is_err());
        ensure!(cursor([1., 2.], scale).is_err());
    }
    ensure!(physical_size([0, 600], 1.).is_err());
    ensure!(physical_size([800, 0], 1.).is_err());
    ensure!(physical_size([u32::MAX, 1], 2.).is_err());
    ensure!(physical_size([1, 1], f32::MIN_POSITIVE).is_err());
    ensure!(cursor([f32::NAN, 0.], 1.).is_err());
    Ok(())
}

fn scaled_click(run: &mut Run, control: Control, scale: f32) -> Result<()> {
    let point = run.point(control)?;
    let (physical, logical) = cursor(point, scale)?;
    run.rows.push(json!({"display_scale":scale,"control":control,
        "physical_cursor":physical,"logical_cursor":logical,
        "window_conversion":"test-driver f64 divide/narrow matching pinned iced_winit; actual UserInterface dispatch follows"}));
    run.step(Step {
        label: format!("scale {scale}: intentional {control:?} logical click"),
        events: vec![Event::moved(logical), Event::Down, Event::Up],
        intent: Intent::Control(control),
        flush: true,
    })
}

fn frame(run: &mut Run, stage: &str, scale: f32) -> Result<()> {
    run.flush()?;
    let before = full(&run.app)?;
    let viewport = run.ui.viewport_bounds(&run.app.viewer)?;
    let owners = run.ui.application_batch(&run.app.viewer, &[]).owners;
    let physical = physical_size(run.app.viewer.display_size, scale)?;
    let (pixels, costs) = run.ui.profiled_pixels_at_scale(&run.app.viewer, scale)?;
    ensure!(
        pixels.len() as u64 == u64::from(physical.width) * u64::from(physical.height) * 4,
        "scaled readback dimensions differ"
    );
    ensure!(
        full(&run.app)? == before
            && run.ui.viewport_bounds(&run.app.viewer)? == viewport
            && run.ui.application_batch(&run.app.viewer, &[]).owners == owners,
        "scaled passive draw changed retained state or logical widget geometry"
    );
    let filename = format!("{stage}-scale-{scale}.png");
    image::save_buffer(
        run.out.join(&filename),
        &pixels,
        physical.width,
        physical.height,
        image::ColorType::Rgba8,
    )?;
    run.rows.push(json!({"dpi_frame":stage,"scale":scale,
        "logical_size":run.app.viewer.display_size,"physical_size":[physical.width,physical.height],
        "pixels_sha256":engine::digest(&pixels),"png":filename,"costs":costs,
        "viewport":viewport,"widget_owners":owners,"before":before,"after":full(&run.app)?,
        "passive_state_neutral":true,"timing_scope":"diagnostic only; GPU execution/readback inseparable"}));
    run.journal()
}

fn cold_equal(run: &mut Run, scale: f32) -> Result<()> {
    let before = full(&run.app)?;
    let checkpoint = run.app.checkpoint()?;
    let restored = cold(&run.app, &checkpoint)?;
    ensure!(
        full(&restored)? == before && full(&run.app)? == before,
        "display-scale sequence changed complete managed cold restoration"
    );
    run.rows.push(
        json!({"dpi_safe_cold":scale,"warm":before,"cold":full(&restored)?,
        "checkpoint_sha256":engine::digest(&checkpoint),"checkpoint_bytes":checkpoint.len(),
        "independently_scanned_index":true,"checkpoint_persisted":false}),
    );
    run.journal()
}

#[test]
#[ignore = "private verified native-owned Newbie; actual offscreen DPI rendering and logical Iced input"]
fn native_owned_dpi_hud_object_popup_catalog_and_cold() -> Result<()> {
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
        run.events(
            "logical display size",
            vec![Event::Resize([800, 600])],
            true,
        )?;
        for scale in [1., 1.5, 2.] {
            let resident = run.app.viewer.resident;
            scaled_click(&mut run, Control::Resident(resident), scale)?;
            run.hud()?;
            let viewport = run.ui.viewport_bounds(&run.app.viewer)?;
            frame(&mut run, "live-hud", scale)?;
            scaled_click(&mut run, Control::Options, scale)?;
            ensure!(run.app.viewer.options_open);
            scaled_click(&mut run, Control::Options, scale)?;
            ensure!(!run.app.viewer.options_open);

            let (object, point, pixel) = run.object_point()?;
            let (physical, logical) = cursor(point, scale)?;
            run.rows.push(json!({"object":object,"display_scale":scale,
                "physical_cursor":physical,"logical_cursor":logical,"scene_pixel":pixel}));
            run.step(Step {
                label: format!("scale {scale}: actual rendered object {object}"),
                events: vec![Event::moved(logical), Event::Down, Event::Up],
                intent: Intent::Object {
                    object,
                    pixel: pixel.map(f32::to_bits),
                },
                flush: true,
            })?;
            ensure!(
                run.app
                    .menu()
                    .is_some_and(|m| m.target() == object && !m.actions().is_empty())
            );
            ensure!(
                run.ui.viewport_bounds(&run.app.viewer)? == viewport,
                "popup resized viewport"
            );
            frame(&mut run, "object-popup", scale)?;
            scaled_click(&mut run, Control::ObjectCancel, scale)?;
            ensure!(run.app.menu().is_none());
            ensure!(
                run.ui.viewport_bounds(&run.app.viewer)? == viewport,
                "popup dismissal resized viewport"
            );

            scaled_click(&mut run, Control::Buy, scale)?;
            ensure!(mode(&run.app)? == 1);
            run.scroll_if_available()?;
            frame(&mut run, "buy-catalog", scale)?;
            scaled_click(&mut run, Control::Live, scale)?;
            ensure!(mode(&run.app)? == 2);
            safe(&run.app)?;
            cold_equal(&mut run, scale)?;
        }
        let final_state = full(&run.app)?;
        ensure!(final_state.tick == run.initial.tick && final_state.rng == run.initial.rng);
        write_json(
            &run.out.join("result.json"),
            &json!({"passed":true,"scales":[1.,1.5,2.],
            "logical_size":[800,600],"physical_sizes":[[800,600],[1200,900],[1600,1200]],
            "rendered_states":["live-hud","object-popup","buy-catalog"],"frames":9,
            "actual_iced_batches":run.steps.len(),"initial":run.initial,"final":final_state,
            "completed_ticks":0,"safe_independent_cold_checks":3,
            "scope":"internal renderer/input invariants; no original pixels, native OS delivery or tick-parity credit",
            "timings":"diagnostic during concurrent correctness work; no thresholds or speed claims"}),
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
