use super::*;
use clap::Parser;
use serde_json::json;

pub(super) fn drain(v: &mut Viewer, inputs: &mut Vec<serde_json::Value>) -> Result<()> {
    // Complete this frame's synchronous work without advancing the test's
    // wall clock on every helper poll. Timed frames are supplied below.
    let now = v.last.unwrap_or_else(Instant::now);
    for _ in 0..10000 {
        ensure!(v.error.is_none(), "{:?}", v.error);
        if !v.presentation_pending() {
            v.refresh()?;
            return Ok(());
        }
        if let Some(s) = &v.session {
            if let Some(id) = s.simulation.vm.world.dialogs.modal {
                if !v.presentation_children_pending() {
                    let record = s
                        .simulation
                        .vm
                        .world
                        .dialogs
                        .records
                        .iter()
                        .find(|r| r.id == id)
                        .context("modal record missing")?;
                    let notice = record.origin.bhav == 4119
                        && [
                            "HDHelpSystem.iff",
                            "HelpSystemMagic.iff",
                            "NeighborhoodHelpSystem.iff",
                            "HelpSystemStudioTown.iff",
                            "VacationHelpSystem.iff",
                        ]
                        .iter()
                        .any(|name| record.origin.source.ends_with(name))
                        && record.buttons.len() == 1
                        && record.buttons[0].result == 2;
                    let help = record.origin.bhav == 4119
                        && record.origin.source.ends_with("::HelpSystem.iff")
                        && record
                            .body
                            .ends_with("Would you like the help system activated?")
                        && record.buttons.len() == 2;
                    let response = if notice {
                        Some(2)
                    } else if help {
                        record
                            .buttons
                            .iter()
                            .find(|b| b.label.eq_ignore_ascii_case("no"))
                            .map(|b| b.result)
                    } else {
                        None
                    }
                    .with_context(|| format!("unexpected transport test dialog: {record:?}"))?;
                    ensure!(inputs.len() < 6, "startup input budget exceeded");
                    inputs.push(json!({"id":id,"tick":s.simulation.vm.tick,"source":record.origin.source,"bhav":record.origin.bhav,"result":response}));
                    // An explicit authored player response through the actual
                    // Iced handler; the production application never auto-OKs.
                    let _ = v.update(Message::Dialog(id, response));
                    continue;
                }
            }
        }
        let _ = v.update(Message::Frame(now));
    }
    let s = v.session.as_ref().unwrap();
    anyhow::bail!(
        "transport UI did not finish: status={:?}, transport={}, children={}, clock={:?}",
        v.status,
        s.transport_pending(),
        v.presentation_children_pending(),
        s.host.presentation.clock
    )
}

#[test]
#[ignore = "private Goth assets and headless Iced rendering; no GUI launch"]
fn iced_native_transport_polls_while_paused_and_restores_mid_pause() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_TRANSPORT_ICED_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace Iced transport evidence")?;
    let save = output.join("mid-pause.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let mut inputs = vec![];
    ensure!(v.native_transport() && v.session.as_ref().unwrap().transport_pending());
    let start = v.session.as_ref().unwrap().simulation.vm.tick;
    // The real control request is saved before it changes the lot pause word.
    v.save()?;
    let pending = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    v.load(&save)?;
    ensure!(pending == serde_json::to_vec(&v.session.as_ref().unwrap().save())?);
    drain(&mut v, &mut inputs)?;
    ensure!(
        v.status.is_empty(),
        "completed pause retained a busy status"
    );
    ensure!(v.device.as_ref().unwrap().transport_pause() == Some(1));
    ensure!(v.session.as_ref().unwrap().simulation.vm.world.globals[&17] == 1);
    let people = v.session.as_ref().unwrap().simulation.characters.clone();
    let now = Instant::now();
    v.last = Some(now);
    v.clock_origin = Some((
        now,
        v.session
            .as_ref()
            .unwrap()
            .host
            .presentation
            .clock
            .as_ref()
            .unwrap()
            .now,
    ));
    for n in 1..=3 {
        let _ = v.update(Message::Frame(
            now + std::time::Duration::from_millis(80 * n),
        ));
        ensure!(v.error.is_none(), "{:?}", v.error);
        drain(&mut v, &mut inputs)?;
    }
    ensure!(v.session.as_ref().unwrap().simulation.vm.tick == start);
    ensure!(v.session.as_ref().unwrap().simulation.characters == people);
    ensure!(v.accumulator < 0.32);
    let before = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("paused.png"), v.view())?;
    ensure!(before == serde_json::to_vec(&v.session.as_ref().unwrap().save())?);
    let _ = v.update(Message::Toggle);
    drain(&mut v, &mut inputs)?;
    ensure!(v.playing && v.device.as_ref().unwrap().transport_pause() == Some(0));
    ensure!(v.session.as_ref().unwrap().simulation.vm.world.globals[&17] == 0);
    let now = Instant::now();
    v.last = Some(now);
    v.clock_origin = Some((
        now,
        v.session
            .as_ref()
            .unwrap()
            .host
            .presentation
            .clock
            .as_ref()
            .unwrap()
            .now,
    ));
    let _ = v.update(Message::Frame(now + std::time::Duration::from_millis(45)));
    drain(&mut v, &mut inputs)?;
    ensure!(v.session.as_ref().unwrap().simulation.vm.tick > start);
    let before = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("playing.png"), v.view())?;
    ensure!(before == serde_json::to_vec(&v.session.as_ref().unwrap().save())?);
    let _ = v.update(Message::Toggle);
    ensure!(v.session.as_ref().unwrap().transport_pending());
    v.save()?;
    let pending = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    v.load(&save)?;
    ensure!(pending == serde_json::to_vec(&v.session.as_ref().unwrap().save())?);
    drain(&mut v, &mut inputs)?;
    ensure!(v.device.as_ref().unwrap().transport_pause() == Some(1));
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "tick_before":start,"tick_after":v.session.as_ref().unwrap().simulation.vm.tick,
            "paused_frames":3,"paused_people_unchanged":true,"mid_pause_cold_restore":true,
            "headless_render_does_not_mutate_session":true,"original_scheduler_required":true,
            "explicit_startup_dialog_inputs":inputs,
            "cadence":"Original elapsed-frame caller bound; complete dirty frame remains separate."
        }))?,
    )?;
    Ok(())
}
