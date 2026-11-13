//! Actual pause command, house callbacks, character refresh and cold resume.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Options, Progress, Session, completion};
use std::{collections::BTreeSet, path::Path};

fn finish(s: &mut Session, d: &mut Device, budget: usize) -> Result<()> {
    for _ in 0..10000 {
        if d.advance_transport(s, budget)? {
            return Ok(());
        }
    }
    anyhow::bail!("transport budget exceeded")
}

#[test]
#[ignore = "private Newbie pause/dialog diagnostic; no original instructions executed"]
fn newbie_paused_dialog_character_change_audit() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join(std::env::var("SIMS_TRANSPORT_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")));
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut s = Session::load(
        &index,
        "UserData/Houses/House07.iff",
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
        0,
        completion::Config {
            controller_suppressed: 0,
            mode: 0,
        },
    )?;
    s.begin_pause(true)?;
    finish(&mut s, &mut d, 256)?;
    std::fs::write(output.join("before.json"), serde_json::to_vec(&s.save())?)?;
    let progress = s.step()?;
    std::fs::write(output.join("after.json"), serde_json::to_vec(&s.save())?)?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"progress":format!("{progress:?}"),"tick":s.simulation.vm.tick,"pause_word":s.simulation.vm.world.globals[&17]}),
        )?,
    )?;
    println!(
        "Newbie paused boundary: {progress:?} at{} pause{}",
        s.simulation.vm.tick, s.simulation.vm.world.globals[&17]
    );
    Ok(())
}

#[test]
#[ignore = "private original houses; ordered pause services, cold restore and software frames"]
fn original_homes_preserve_native_pause_boundaries_and_resume() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join(std::env::var("SIMS_TRANSPORT_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace transport evidence")?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    for (name, source) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ] {
        let dir = output.join(name);
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
            0,
            completion::Config {
                controller_suppressed: 0,
                mode: 0,
            },
        )?;
        let start = s.simulation.vm.tick;
        println!("{name}: opened native scheduler at{start}");
        ensure!(s.host.presentation.transport.is_some() && d.transport_pause() == Some(0));
        let initial = s.save();
        let initial_device = d.save(&s)?;
        s.begin_pause(true)?;
        // Stop before the device acknowledges the window request; a complete
        // device advance would publish and acknowledge it in the same call.
        ensure!(matches!(
            s.advance_transport(1)?,
            sims_runtime::original_session::transport::Progress::Service(
                sims_runtime::original_session::transport::Service::WindowPause(1)
            )
        ));
        let mut checkpoints = BTreeSet::new();
        for _ in 0..10000 {
            if !s.transport_pending() {
                break;
            }
            // Save representative boundaries, including a real suspended
            // person refresh and the original C2 dispatcher, once each.
            let state = serde_json::to_value(s.host.presentation.transport.as_ref().unwrap())?;
            let call = &state["call"];
            let key = if call["broadcast"] == true {
                "broadcast"
            } else if !call["people"].is_null() {
                "people"
            } else if !call["publication"].is_null() {
                "publication"
            } else if !call["setter"]["pending"].is_null() {
                "window"
            } else {
                "start"
            };
            if checkpoints.insert(key.to_owned()) {
                let saved = s.save();
                let device = d.save(&s)?;
                let before = serde_json::to_vec(&saved)?;
                ensure!(s.step().is_err());
                ensure!(s.begin_pause(false).is_err());
                ensure!(
                    before == serde_json::to_vec(&s.save())?,
                    "blocked calls changed state"
                );
                let mut restored = Session::restore(&index, saved)?;
                let mut restored_device = Device::restore(&restored, device)?;
                ensure!(before == serde_json::to_vec(&restored.save())?);
                let a = d.advance_transport(&mut s, 1)?;
                let b = restored_device.advance_transport(&mut restored, 1)?;
                ensure!(
                    a == b
                        && serde_json::to_vec(&s.save())? == serde_json::to_vec(&restored.save())?,
                    "{name}: restore differs at{key}"
                );
                ensure!(d.save(&s)? == restored_device.save(&restored)?);
                println!("{name}: restored{key}");
            } else {
                d.advance_transport(&mut s, 1)?;
            }
        }
        ensure!(!s.transport_pending(), "{name}: pause did not complete");
        ensure!(
            checkpoints.contains("people")
                && checkpoints.contains("broadcast")
                && checkpoints.contains("window")
        );
        ensure!(s.simulation.vm.tick == start && s.simulation.vm.world.globals[&17] == 1);
        ensure!(d.transport_pause() == Some(1));
        // Same call with larger budgets has identical state and pixels.
        let mut fast = Session::restore(&index, initial)?;
        let mut fast_device = Device::restore(&fast, initial_device)?;
        fast.begin_pause(true)?;
        finish(&mut fast, &mut fast_device, 256)?;
        ensure!(serde_json::to_vec(&s.save())? == serde_json::to_vec(&fast.save())?);
        ensure!(software::render(d.scene()) == software::render(fast_device.scene()));
        let people = s.simulation.characters.clone();
        let person_threads: Vec<_> = people
            .keys()
            .map(|id| {
                let thread = s.simulation.vm.object_threads[id];
                (thread, s.simulation.vm.threads[&thread].clone())
            })
            .collect();
        let clock = s.simulation.household.as_ref().unwrap().clock.clone();
        let mut partial_passes = 0;
        let mut boundary = None;
        for _ in 0..3 {
            match s.step()? {
                Progress::PartialPass => partial_passes += 1,
                Progress::Dialog if name == "newbie" => {
                    // The original tutorial controller is eligible while
                    // paused. Preserve its prompt instead of inventing an
                    // answer or requiring all paused work to be inert.
                    boundary = Some("original tutorial dialog while paused");
                    break;
                }
                other => anyhow::bail!("{name}: unexpected paused boundary {other:?}"),
            }
        }
        ensure!(
            s.simulation.vm.tick == start
                && s.simulation.household.as_ref().unwrap().clock == clock
        );
        for (thread, before) in &person_threads {
            ensure!(
                &s.simulation.vm.threads[thread] == before,
                "paused ordinary person thread ran"
            );
        }
        if boundary.is_none() {
            ensure!(s.simulation.characters == people);
        } else {
            // Original Tutorial setup4098 finds/removes Betty before general
            // tutorial4097 calls4117 (motives90/50) and shows its welcome box.
            // Those controller writes are allowed while personal threads and
            // the clock are paused; freezing the entire world would be wrong.
            ensure!(s.simulation.characters[&105] == people[&105]);
            let position = &s.simulation.vm.world.positions[&137];
            ensure!(position.x == -16 && position.y == -16);
            ensure!(!s.simulation.bodies.contains_key(&137));
            for id in [105, 137] {
                let m = s.simulation.vm.world.objects[&id]
                    .motives
                    .context("tutorial motives absent")?;
                for slot in [5, 6, 7, 8, 9, 14, 15] {
                    ensure!(m[slot] == 90f32.to_bits());
                }
                ensure!(m[3] == 50f32.to_bits());
            }
            let dialog = s
                .simulation
                .vm
                .world
                .dialogs
                .records
                .last()
                .context("tutorial dialog absent")?;
            ensure!(
                dialog.caller == 322
                    && dialog.origin.bhav == 4097
                    && dialog.origin.instruction == 2
            );
        }
        d.refresh(&s)?;
        software::render(d.scene()).save(dir.join("paused.png"))?;
        if let Some(boundary) = boundary {
            let saved = s.save();
            let restored = Session::restore(&index, saved.clone())?;
            ensure!(serde_json::to_vec(&saved)? == serde_json::to_vec(&restored.save())?);
            ensure!(restored.simulation.vm.world.dialogs.modal.is_some());
            std::fs::write(dir.join("paused-dialog.json"), serde_json::to_vec(&saved)?)?;
            std::fs::write(
                dir.join("report.json"),
                serde_json::to_vec_pretty(&json!({
                    "source":source,"tick_before":start,"tick_after":s.simulation.vm.tick,
                    "restored_stages":checkpoints,"partial_passes":partial_passes,"boundary":boundary,
                    "ordinary_person_threads_unchanged":true,"paused_clock_unchanged":true,
                    "tutorial_removes_betty_and_sets_motives_from_original_bhavs":true,"small_large_budgets_match":true,
                    "dialog_cold_restore":true,"dialog_automatically_answered":false,
                    "scope":"Original paused controller reached its real tutorial prompt. Resume/timer reset are exercised by the other two homes, not by bypassing this modal."
                }))?,
            )?;
            println!(
                "{name}: original tutorial dialog retained at paused tick{start}; no automatic reply"
            );
            continue;
        }
        s.host.presentation.transport.as_mut().unwrap().timing =
            Some(sims_vm::scheduler::transport::Timing {
                last: 77,
                elapsed: 123,
            });
        s.begin_pause(false)?;
        let mut saw_broadcast = false;
        for _ in 0..10000 {
            if !s.transport_pending() {
                break;
            }
            let state = serde_json::to_value(s.host.presentation.transport.as_ref().unwrap())?;
            if !saw_broadcast && state["call"]["broadcast"] == true {
                saw_broadcast = true;
                ensure!(s.simulation.vm.world.globals[&17] == 0);
                ensure!(state["timing"] == json!({"last":77,"elapsed":123}));
                s = Session::restore(&index, s.save())?;
                d = Device::restore(&s, d.save(&s)?)?;
            }
            d.advance_transport(&mut s, 1)?;
        }
        ensure!(saw_broadcast && !s.transport_pending());
        ensure!(s.host.presentation.transport.as_ref().unwrap().timing == Some(Default::default()));
        ensure!(d.transport_pause() == Some(0));
        ensure!(s.step()? == Progress::Tick);
        ensure!(s.simulation.vm.tick == start + 1);
        d.refresh(&s)?;
        software::render(d.scene()).save(dir.join("resumed.png"))?;
        std::fs::write(
            dir.join("report.json"),
            serde_json::to_vec_pretty(&json!({
                "source":source,"tick_before":start,"tick_after":s.simulation.vm.tick,"restored_stages":checkpoints,
                "partial_passes":3,"paused_people_and_clock_unchanged":true,"resume_resets_after_c2":true,
                "small_large_budgets_match":true,"scope":"Fresh real household, actual person refresh and C2 dispatch. Replacement cadence; full native interface frame and speed changes remain separate."
            }))?,
        )?;
        println!("{name}: pause+3partial passes+resume+1tick passed");
    }
    Ok(())
}
