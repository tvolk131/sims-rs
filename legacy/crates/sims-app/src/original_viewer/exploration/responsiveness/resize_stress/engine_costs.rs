//! Bounded engine-service attribution after first-use font/catalog work.
//! Actual Iced events and their ordinary-handler reference remain unchanged.
use super::*;

#[test]
#[ignore = "native-owned paused Newbie; private per-service instrumentation; no timing ceilings"]
fn native_owned_warm_live_engine_services() -> Result<()> {
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
        run.hud()?;
        // Real layout and handlers warm their own resources. The warmup is
        // explicitly journaled rather than treated as a cold/warm speedup.
        for (index, size) in [[800, 600], [1000, 740], [1320, 900]]
            .into_iter()
            .enumerate()
        {
            run.events("deliberate size change", vec![Event::Resize(size)], true)?;
            for repetition in 0..3 {
                let prefix = format!("size{index}-{size:?}/repeat{repetition}");
                let resident = run.app.viewer.resident;
                control(
                    &mut run,
                    Control::Resident(resident),
                    &format!("{prefix}/resident"),
                )?;
                for (control_id, expected_mode, label) in
                    [(Control::Buy, 1, "Buy"), (Control::Live, 2, "Live")]
                {
                    let name = format!("{prefix}/{label}");
                    eprintln!(
                        "UI_ENGINE_OPERATION {}",
                        json!({"name":name,"warmup":index==0 && repetition==0})
                    );
                    control(&mut run, control_id, &name)?;
                    eprintln!("UI_ENGINE_OPERATION_END {}", json!({"name":name}));
                    ensure!(
                        mode(&run.app)? == expected_mode,
                        "mode command did not finish"
                    );
                }
            }
        }
        safe(&run.app)?;
        run.frame("warm-Live-final-frame")?;
        run.checkpoint("warm-Live-final-safe")?;
        let final_state = full(&run.app)?;
        ensure!(final_state.tick == run.initial.tick && final_state.rng == run.initial.rng);
        write_json(
            &run.out.join("result.json"),
            &json!({
                "passed":true,"sizes":3,"repetitions_each":3,"mode_pairs":9,
                "batches":run.steps.len(),"initial":run.initial,"final":final_state,
                "completed_ticks":0,"ordinary_handler_reference":true,"cold_equal":true,
                "scope":"Observed device services plus ordered app spans. Synchronous children stay in handler costs; no original parity or controlled speedup claim",
                "warmup":"First complete pair is explicitly labeled; process/OS caches are not forcibly reset"
            }),
        )?;
        Ok(())
    })();
    if let Err(error) = &result {
        run.journal()?;
        write_json(
            &run.out.join("first-failure.json"),
            &json!({
                "error":format!("{error:#}"),"actual":full(&run.app)?,
                "reference":full(&run.reference)?,"completed_batches":run.steps.len()
            }),
        )?;
        std::fs::write(run.out.join("failure.json"), run.app.checkpoint()?)?;
        std::fs::write(
            run.out.join("failure-reference.json"),
            run.reference.checkpoint()?,
        )?;
    }
    result
}
