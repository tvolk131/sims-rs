//! Real household elapsed-frame execution, shared pause counters, and cold saves.
use anyhow::{Result, ensure};
use serde_json::json;
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Options, Progress, Session, clock, completion};
use std::path::Path;

fn transport(s: &mut Session, d: &mut Device) -> Result<()> {
    for _ in 0..10000 {
        if d.advance_transport(s, 256)? {
            return Ok(());
        }
    }
    anyhow::bail!("transport budget exceeded")
}
fn frame(s: &mut Session, budget: usize) -> Result<(u32, Option<Progress>)> {
    for _ in 0..10000 {
        match s.advance_clock(budget)? {
            clock::Progress::Complete { updates } => return Ok((updates, None)),
            clock::Progress::Pending
            | clock::Progress::Household(Progress::Tick | Progress::PartialPass) => (),
            clock::Progress::Household(other) => return Ok((0, Some(other))),
            clock::Progress::PollInput => s.acknowledge_clock_input(false)?,
        }
    }
    anyhow::bail!("elapsed frame budget exceeded")
}

#[test]
#[ignore = "private original homes; native elapsed caller integration and headless frames"]
fn original_homes_run_elapsed_frames_and_restore_pending_callers() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_CLOCK_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")));
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    for (name, source) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ] {
        let dir = out.join(name);
        std::fs::create_dir(&dir)?;
        let mut s = Session::load(
            &index,
            source,
            Options {
                native_postload: true,
                native_load_completion: true,
                routing: sims_sim::lot_route::path::Policy::InferredRectangles,
                ..Default::default()
            },
        )?;
        let mut d = Device::open(
            &mut s,
            1000,
            740,
            100,
            completion::Config {
                controller_suppressed: 0,
                mode: 0,
            },
        )?;
        let start = s.simulation.vm.tick;
        // Existing transport-only saves keep their timing profile.
        let legacy = s.save();
        let legacy_device = d.save(&s)?;
        let legacy_json = serde_json::to_vec(&legacy)?;
        let restored = Session::restore(&index, legacy)?;
        ensure!(restored.host.presentation.clock.is_none());
        ensure!(legacy_json == serde_json::to_vec(&restored.save())?);
        Device::restore(&restored, legacy_device)?;
        s.bind_clock(100)?;
        ensure!(
            s.host
                .presentation
                .transport
                .as_ref()
                .unwrap()
                .timing
                .is_none()
        );
        let period = s.host.presentation.clock.as_ref().unwrap().fields.period as u32;
        println!("{name}: clock bound at{start}, period{period}ms");
        s.begin_clock_frame()?;
        ensure!(s.advance_clock(1)? == clock::Progress::Pending);
        let saved = s.save();
        let device = d.save(&s)?;
        let before = serde_json::to_vec(&saved)?;
        ensure!(
            s.step().is_err() && s.begin_pause(true).is_err() && s.begin_clock_frame().is_err()
        );
        ensure!(before == serde_json::to_vec(&s.save())?);
        let mut cold = Session::restore(&index, saved)?;
        let _cold_device = Device::restore(&cold, device)?;
        let a = frame(&mut s, 1)?;
        let b = frame(&mut cold, 256)?;
        ensure!(
            a == b && serde_json::to_vec(&s.save())? == serde_json::to_vec(&cold.save())?,
            "{name}: frame budget/restore mismatch"
        );
        if let Some(boundary) = a.1 {
            ensure!(
                name == "newbie"
                    && matches!(
                        boundary,
                        Progress::Dialog | Progress::DialogConstruction(_) | Progress::UiEffect(_)
                    ),
                "{name}: unexpected boundary {boundary:?}"
            );
            ensure!(s.clock_pending() && s.simulation.vm.tick == start);
            let saved = s.save();
            let restored = Session::restore(&index, saved.clone())?;
            ensure!(serde_json::to_vec(&saved)? == serde_json::to_vec(&restored.save())?);
            d.refresh(&s)?;
            software::render(d.scene()).save(dir.join("pending.png"))?;
            std::fs::write(dir.join("session.json"), serde_json::to_vec(&saved)?)?;
            std::fs::write(
                dir.join("report.json"),
                serde_json::to_vec_pretty(
                    &json!({"tick_before":start,"tick_after":s.simulation.vm.tick,
                "boundary":format!("{boundary:?}"),"original_boundary_unanswered":true,"pending_frame_cold_restore":true,"small_large_budgets_match":true,"legacy_profile_preserved":true}),
                )?,
            )?;
            println!("{name}: retained original {boundary:?}; pending frame cold restore passed");
            continue;
        }
        ensure!(a.0 == 1 && s.simulation.vm.tick == start + 1);
        // Pause the same elapsed owner, consume real partial passes, then resume.
        s.begin_pause(true)?;
        transport(&mut s, &mut d)?;
        let paused = s.simulation.vm.tick;
        for n in 1..=3 {
            s.set_clock_time(100 + period * n)?;
            s.begin_clock_frame()?;
            let r = frame(&mut s, 256)?;
            ensure!(r == (1, None));
        }
        ensure!(s.simulation.vm.tick == paused);
        s.begin_pause(false)?;
        transport(&mut s, &mut d)?;
        ensure!(s.host.presentation.clock.as_ref().unwrap().fields.timing == Default::default());
        // Resume resets the lot counters, not the timer's independent credit.
        // At the same application time there is still no timer token to spend.
        s.begin_clock_frame()?;
        let first = frame(&mut s, 256)?;
        ensure!(first == (0, None));
        let mut full_updates = 1;
        for n in 4..=67 {
            s.set_clock_time(100 + period * n)?;
            s.begin_clock_frame()?;
            let r = frame(&mut s, 256)?;
            ensure!(r.1.is_none(), "{name}: frame boundary {:?}", r.1);
            full_updates += r.0;
        }
        ensure!(s.simulation.vm.tick == start + u64::from(full_updates));
        // This headless caller advanced the lot without a viewer frame. Bring
        // its surface to the current tick before checkpointing a UI callback.
        d.refresh(&s)?;
        // Exercise4ce44f's actual SetSpeed child on the same clock owner.
        // These are explicit CPState inputs; the4cee89 UI setter is separate.
        let old_speed = s
            .host
            .presentation
            .transport
            .as_ref()
            .unwrap()
            .controls
            .speed;
        let timer = s.host.presentation.clock.as_ref().unwrap().fields.timer;
        s.host
            .presentation
            .transport
            .as_mut()
            .unwrap()
            .controls
            .speed = 777;
        s.begin_pause(true)?;
        let mut restored_speed_notice = false;
        for _ in 0..10000 {
            if !s.transport_pending() {
                break;
            }
            let state = serde_json::to_value(s.host.presentation.transport.as_ref().unwrap())?;
            if !restored_speed_notice
                && !state["call"]["speed"].is_null()
                && state["call"]["broadcast"] == true
            {
                ensure!(s.simulation.vm.world.globals[&16] == 777);
                let fields = &s.host.presentation.clock.as_ref().unwrap().fields;
                ensure!(fields.period == sims_vm::scheduler::clock::period(777));
                ensure!(
                    (fields.timer.last, fields.timer.elapsed, fields.timer.limit)
                        == (timer.last, timer.elapsed, timer.limit)
                );
                let saved = s.save();
                let device = d.save(&s)?;
                let mut cold = Session::restore(&index, saved)?;
                let mut cold_device = Device::restore(&cold, device)?;
                transport(&mut s, &mut d)?;
                transport(&mut cold, &mut cold_device)?;
                ensure!(serde_json::to_vec(&s.save())? == serde_json::to_vec(&cold.save())?);
                ensure!(d.save(&s)? == cold_device.save(&cold)?);
                restored_speed_notice = true;
            } else {
                d.advance_transport(&mut s, 1)?;
            }
        }
        ensure!(restored_speed_notice && !s.transport_pending());
        s.host
            .presentation
            .transport
            .as_mut()
            .unwrap()
            .controls
            .speed = old_speed;
        s.begin_pause(false)?;
        transport(&mut s, &mut d)?;
        ensure!(s.simulation.vm.world.globals[&16] == old_speed as i16);
        ensure!(s.simulation.vm.tick == start + u64::from(full_updates));
        d.refresh(&s)?;
        let saved = s.save();
        let before = serde_json::to_vec(&saved)?;
        software::render(d.scene()).save(dir.join("running.png"))?;
        ensure!(before == serde_json::to_vec(&s.save())?);
        let restored = Session::restore(&index, saved.clone())?;
        ensure!(before == serde_json::to_vec(&restored.save())?);
        Device::restore(&restored, d.save(&s)?)?;
        std::fs::write(dir.join("session.json"), before)?;
        std::fs::write(
            dir.join("report.json"),
            serde_json::to_vec_pretty(
                &json!({"tick_before":start,"tick_after":s.simulation.vm.tick,
            "full_updates":full_updates,"paused_passes":3,"resume_shared_timing_reset":true,"small_large_budgets_match":true,
            "pending_frame_cold_restore":true,"speed_notice_cold_restore":true,"speed_uses_shared_clock_owner":true,
            "final_cold_restore":true,"legacy_profile_preserved":true,"headless_render_preserves_state":true}),
            )?,
        )?;
        println!(
            "{name}: {full_updates} full updates, 3paused passes, restore/headless checks passed"
        );
    }
    Ok(())
}
