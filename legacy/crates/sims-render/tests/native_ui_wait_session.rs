use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, dialog::client::Client, software};
use sims_runtime::original_session::{Progress, Saved, Session, camera_command::Change, ui_wait};

fn poll(
    s: &mut Session,
    index: &Index,
    budget: usize,
    restore: bool,
    captured: &mut u8,
) -> Result<()> {
    s.begin_ui_wait(None)?;
    for _ in 0..10000 {
        let progress = s.advance_ui_wait(budget)?;
        let bit = s
            .simulation
            .ui_wait_callback
            .as_ref()
            .map_or(0, |c| if c.query { 1 } else { 2 });
        if restore && bit != 0 && *captured & bit == 0 {
            let saved = serde_json::to_vec(&s.save())?;
            ensure!(s.step().is_err());
            ensure!(s.simulation.run_current_tick(&s.program, 1).is_err());
            ensure!(s.simulation.finish_registry_pass().is_err());
            let mut invalid = s.save().simulation;
            invalid.state.ui_wait_callback.as_mut().unwrap().query ^= true;
            ensure!(
                invalid.restore(&s.program).is_err(),
                "mismatched named callback accepted"
            );
            ensure!(
                serde_json::to_vec(&s.save())? == saved,
                "rejected tick changed wait callback"
            );
            *s = Session::restore(index, serde_json::from_slice(&saved)?)?;
            ensure!(serde_json::to_vec(&s.save())? == saved);
            // Also restore after the named VM returns but before its caller
            // consumes the result. Advancing the poll must not execute Main.
            let thread = s.simulation.ui_wait_callback.as_ref().unwrap().thread;
            for _ in 0..100 {
                if s.simulation.vm.threads[&thread].callback.is_none() {
                    break;
                }
                s.simulation
                    .vm
                    .run_slice(&s.program, thread, 1)
                    .map_err(anyhow::Error::msg)?;
            }
            ensure!(s.simulation.vm.threads[&thread].callback.is_none());
            let saved = serde_json::to_vec(&s.save())?;
            *s = Session::restore(index, serde_json::from_slice(&saved)?)?;
            ensure!(serde_json::to_vec(&s.save())? == saved);
            *captured |= bit;
        }
        match progress {
            ui_wait::Progress::Complete => return Ok(()),
            ui_wait::Progress::Pending => (),
            p => anyhow::bail!("unexpected wait observation {p:?}"),
        }
    }
    anyhow::bail!("UI poll did not return")
}
#[test]
#[ignore = "private original tutorial checkpoint and actual camera/character assets"]
fn original_scroll_wait_uses_live_attempt_counter_and_resumes_named_callbacks() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/native-camera-device-iced-save.json"),
    )?)?;
    let mut outcomes = vec![];
    let mut pixels = vec![];
    for budget in [1, 10000] {
        let saved: Saved = serde_json::from_value(wrapper["session"].clone())?;
        let mut s = Session::restore(&index, saved)?;
        let mut device = Device::restore(
            &s,
            serde_json::from_value(wrapper["camera_device"].clone())?,
        )?;
        let mut client = Client::restore(
            &s,
            serde_json::from_value(wrapper["dialog_client"].clone())?,
        )?;
        let owner = s
            .simulation
            .vm
            .world
            .ui_owner
            .owner
            .context("tutorial UI owner absent")?;
        let mut captured = 0;
        let start = s.simulation.vm.tick;
        let first_dialog = s.simulation.vm.world.dialogs.records.last().unwrap().id;
        // Reach the original wait using normal script execution, with no input.
        for _ in 0..100 {
            if s.simulation.vm.world.objects[&owner].attributes[2] == 7 {
                break;
            }
            poll(&mut s, &index, budget, budget == 1, &mut captured)?;
            match s.step()? {
                Progress::Tick => (),
                p => anyhow::bail!("before scroll wait: {p:?}"),
            }
        }
        ensure!(
            s.simulation.vm.world.objects[&owner].attributes[2] == 7,
            "original scroll wait absent"
        );
        poll(&mut s, &index, budget, budget == 1, &mut captured)?;
        let baseline = s.host.presentation.ui_wait.as_ref().unwrap().baseline();
        let attempts = s
            .host
            .presentation
            .presenter
            .as_ref()
            .unwrap()
            .camera
            .as_ref()
            .unwrap()
            .settings
            .attempts;
        ensure!(baseline == attempts as i32);
        // Repeated unchanged observations must not acknowledge the wait.
        poll(&mut s, &index, budget, budget == 1, &mut captured)?;
        ensure!(s.simulation.vm.world.objects[&owner].attributes[2] == 7);
        let before = (
            s.simulation.vm.tick,
            s.simulation.vm.rng_state,
            s.simulation.vm.lot_rng_state,
            s.simulation.vm.crt_rng_state,
        );
        let position = s
            .retained_camera_input()?
            .center
            .map(|n| n.wrapping_add(48));
        device.begin(&mut s, Change::Scroll(position))?;
        for _ in 0..10000 {
            if device.advance(&mut s, budget)? {
                break;
            }
        }
        ensure!(!s.camera_command_pending());
        let attempts = s
            .host
            .presentation
            .presenter
            .as_ref()
            .unwrap()
            .camera
            .as_ref()
            .unwrap()
            .settings
            .attempts;
        ensure!(attempts == baseline.wrapping_add(1) as u32);
        poll(&mut s, &index, budget, budget == 1, &mut captured)?;
        ensure!(
            s.simulation.vm.world.objects[&owner].attributes[2] == 0,
            "got-wait-event did not clear its own request"
        );
        ensure!(s.host.presentation.ui_wait.as_ref().unwrap().baseline() == -1);
        ensure!(
            before
                == (
                    s.simulation.vm.tick,
                    s.simulation.vm.rng_state,
                    s.simulation.vm.lot_rng_state,
                    s.simulation.vm.crt_rng_state
                )
        );
        ensure!(
            budget != 1 || captured == 3,
            "no saved callback boundary exercised"
        );
        // Complete the tutorial's requested second scroll through the same
        // native call, only after its own script has registered another wait.
        let mut requests = 1;
        let mut armed = false;
        let mut keyboard_prompt = false;
        let mut rotation_controls = false;
        for _ in 0..500 {
            if client.pending() {
                client.advance(&mut s, budget.min(256))?;
                continue;
            }
            client.collect(&s)?;
            keyboard_prompt |= s.simulation.vm.world.dialogs.records.iter().any(|r| {
                r.id > first_dialog && r.origin.bhav == 4116 && r.origin.instruction == 10
            });
            poll(&mut s, &index, budget, false, &mut captured)?;
            if keyboard_prompt
                && s.simulation.vm.world.objects[&owner].attributes[2] == 7
                && requests < 2
            {
                if armed {
                    let p = s.retained_camera_input()?.center;
                    device.begin(&mut s, Change::Scroll([p[0] - 64, p[1]]))?;
                    for _ in 0..10000 {
                        if device.advance(&mut s, budget)? {
                            break;
                        }
                    }
                    requests += 1;
                    armed = false;
                } else {
                    armed = true;
                }
            }
            match s.step()? {
                Progress::Tick => (),
                Progress::DialogConstruction(thread) => {
                    let input = s.retained_camera_input()?;
                    let camera = sims_render::Camera::from_native(input, 1)?;
                    client.begin(
                        &mut s,
                        thread,
                        sims_render::dialog::client::Inputs::camera(camera, 64)?,
                    )?;
                }
                Progress::UiEffect(thread) => {
                    let t = &s.simulation.vm.threads[&thread];
                    let f = t.stack.last().unwrap();
                    ensure!(
                        requests == 2 && f.location.bhav == 4116 && f.location.instruction == 12,
                        "unexpected UI continuation {f:?}"
                    );
                    rotation_controls = true;
                    break;
                }
                p => anyhow::bail!("after scroll: {p:?}"),
            }
        }
        ensure!(
            keyboard_prompt && rotation_controls,
            "scroll lessons did not reach rotation controls"
        );
        device.refresh(&s)?;
        outcomes.push(serde_json::to_vec(&s.save())?);
        pixels.push(software::render(device.scene()));
        println!(
            "budget {budget}: {} ticks, {requests} explicit scroll attempts, keyboard prompt {keyboard_prompt}, rotation controls {rotation_controls}",
            s.simulation.vm.tick - start
        );
        if budget == 1 {
            std::fs::write(
                root.join("cache/native-ui-wait-session-report.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"ticks":s.simulation.vm.tick-start,"scrolls":requests,"keyboard_prompt":keyboard_prompt,"rotation_controls":rotation_controls,"callback_restored":captured}),
                )?,
            )?;
        }
    }
    ensure!(
        outcomes[0] == outcomes[1],
        "UI poll budget/restore changed session state"
    );
    ensure!(pixels[0] == pixels[1]);
    pixels[0].save(root.join("cache/native-ui-wait-session.png"))?;
    Ok(())
}
