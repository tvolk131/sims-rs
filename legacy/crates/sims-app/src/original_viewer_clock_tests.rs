use super::*;
use clap::Parser;
use serde_json::json;
use sha2::{Digest, Sha256};
use sims_vm::interface::speed::{FAST, NORMAL, ULTRA};

fn drain(v: &mut Viewer) -> Result<()> {
    for _ in 0..10000 {
        ensure!(v.error.is_none(), "{:?}", v.error);
        if !v.presentation_pending()
            || (v
                .session
                .as_ref()
                .unwrap()
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .is_some()
                && !v.presentation_children_pending())
        {
            return v.refresh();
        }
        v.step()?;
        v.refresh()?;
    }
    anyhow::bail!("elapsed Iced caller did not return")
}
fn state(v: &Viewer) -> Result<Vec<u8>> {
    let s = v.session.as_ref().unwrap();
    Ok(serde_json::to_vec(&json!({"session":s.save(),
        "device":v.device.as_ref().unwrap().save(s)?,
        "client":v.client.as_ref().unwrap().save(s)?}))?)
}
fn restored_state(v: &Viewer, expected: &[u8], dir: &Path, name: &str) -> Result<()> {
    let actual = state(v)?;
    if actual != expected {
        std::fs::write(dir.join(format!("{name}-expected.json")), expected)?;
        std::fs::write(dir.join(format!("{name}-actual.json")), actual)?;
        anyhow::bail!("{name}: restored state differs; saved diagnostic pair");
    }
    Ok(())
}

// Authored test-player inputs, never production defaults. Do not dismiss an
// arbitrary modal merely because its first button looks like an acknowledgement.
pub(super) fn startup_dialog_response(record: &sims_vm::dialog::Record) -> Result<u32> {
    if record.source == record.origin.source
        && record.origin.source.ends_with("::HelpSystem.iff")
        && record.origin.bhav == 4119
        && record.origin.instruction == 10
        && record.title == "Help System"
        && format!("{:x}", Sha256::digest(record.body.as_bytes()))
            == "52f35949be4524be16205e81c259ff3e338cc6e87ff98c641387cfa7191e766f"
        && record.buttons.len() == 2
        && record.buttons[0].label == "Yes"
        && record.buttons[0].result == 2
        && record.buttons[1].label == "No"
        && record.buttons[1].result == 3
    {
        return Ok(3);
    }
    ensure!(
        record.origin.bhav == 4119
            && record.source == record.origin.source
            && [
                "::HDHelpSystem.iff",
                "::HelpSystemMagic.iff",
                "::NeighborhoodHelpSystem.iff",
                "::HelpSystemStudioTown.iff",
                "::VacationHelpSystem.iff",
            ]
            .iter()
            .any(|name| record.origin.source.ends_with(name))
            && record.buttons.len() == 1
            && record.buttons[0].result == 2,
        "unplanned original dialog: {record:?}"
    );
    Ok(2)
}

fn answer_startup_dialogs(
    v: &mut Viewer,
    dir: &Path,
    prefix: &str,
    paused: bool,
) -> Result<Vec<serde_json::Value>> {
    let mut inputs = vec![];
    for _ in 0..10 {
        let s = v.session.as_ref().unwrap();
        let Some(id) = s.simulation.vm.world.dialogs.modal else {
            return Ok(inputs);
        };
        let record = s
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .find(|r| r.id == id)
            .context("queued modal record absent")?
            .clone();
        let result = startup_dialog_response(&record)?;
        let tick = s.simulation.vm.tick;
        if inputs.is_empty() {
            let path = dir.join(format!("{prefix}-pending.json"));
            let previous_save = v.args.save.clone();
            v.args.save = path.clone();
            v.save()?;
            v.args.save = previous_save;
            let expected = state(v)?;
            v.load(&path)?;
            restored_state(v, &expected, dir, prefix)?;
        }
        inputs.push(json!({"tick":tick,"record":record,"result":result}));
        let _ = v.update(Message::Dialog(id, result));
        ensure!(v.error.is_none(), "{:?}", v.error);
        drain(v)?;
        if paused {
            ensure!(v.session.as_ref().unwrap().simulation.vm.tick == tick);
        }
    }
    anyhow::bail!("expansion notice input budget exceeded")
}

#[test]
#[ignore = "private three-home original elapsed clock and headless Iced frames; no GUI launch"]
fn iced_original_clock_controls_restore_callbacks_and_answer_owned_modal() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let out = repo.join(std::env::var("SIMS_CLOCK_ICED_OUTPUT")?);
    ensure!(out.starts_with(repo.join("cache")));
    std::fs::create_dir(&out)?;
    for (name, source) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ] {
        if std::env::var("SIMS_CLOCK_ICED_HOUSE").is_ok_and(|house| house != name) {
            continue;
        }
        let dir = out.join(name);
        std::fs::create_dir(&dir)?;
        let save = dir.join("pending.json");
        let args = Args::parse_from([
            "sims-room",
            "--original",
            source,
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
        ensure!(v.native_clock());
        drain(&mut v)?;
        let start = v.session.as_ref().unwrap().simulation.vm.tick;
        println!("{name}: Iced native clock at{start}");
        let now = Instant::now();
        let _ = v.update(Message::Frame(now));
        drain(&mut v)?;
        ensure!(v.session.as_ref().unwrap().simulation.vm.tick == start);
        // Preserve the first elapsed boundary, including an unexpected modal,
        // before testing its expected outcome. A modal keeps its clock parent
        // suspended legitimately; diagnostics must not discard that evidence.
        v.args.save = dir.join("initial-paused-frame.json");
        v.save()?;
        v.args.save = save.clone();
        let s = v.session.as_ref().unwrap();
        std::fs::write(
            dir.join("initial-paused-frame-summary.json"),
            serde_json::to_vec_pretty(&json!({
                "tick":s.simulation.vm.tick,
                "clock_pending":s.clock_pending(),
                "modal":s.simulation.vm.world.dialogs.modal,
                "records":s.simulation.vm.world.dialogs.records,
            }))?,
        )?;
        if name == "newbie" {
            let s = v.session.as_ref().unwrap();
            ensure!(s.clock_pending());
            let id = s
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .context("original welcome absent")?;
            let record = s
                .simulation
                .vm
                .world
                .dialogs
                .records
                .iter()
                .find(|r| r.id == id)
                .unwrap()
                .clone();
            let before = state(&v)?;
            render_headless_element(&root, &dir.join("pending.png"), v.view())?;
            ensure!(before == state(&v)?);
            v.save()?;
            // Save refreshes the actual camera surface and its generation.
            let before = state(&v)?;
            v.load(&save)?;
            restored_state(&v, &before, &dir, "modal")?;
            ensure!(v.clock_origin.is_none());
            // Only an explicit test player response returns the modal child.
            let _ = v.update(Message::Dialog(id, record.buttons[0].result));
            ensure!(v.error.is_none(), "{:?}", v.error);
            ensure!(
                v.session
                    .as_ref()
                    .unwrap()
                    .simulation
                    .vm
                    .world
                    .dialogs
                    .modal
                    .is_none()
            );
            drain(&mut v)?;
            ensure!(!v.session.as_ref().unwrap().clock_pending());
            std::fs::write(
                dir.join("report.json"),
                serde_json::to_vec_pretty(&json!({
                    "tick_before":start,"tick_after":v.session.as_ref().unwrap().simulation.vm.tick,
                    "modal_clock_parent_restored":true,"explicit_dialog_input_accepted":true,
                    "headless_render_preserves_state":true,"original_boundary_unanswered_in_saved_file":true
                }))?,
            )?;
            println!("{name}: pending modal cold restore and explicit Iced answer passed");
            continue;
        }
        let initial_notice_inputs = answer_startup_dialogs(&mut v, &dir, "initial-notices", true)?;
        ensure!(!v.session.as_ref().unwrap().clock_pending());
        v.args.save = save.clone();
        // Save the real UI speed command inside its actual SetSpeed C2 child.
        let _ = v.update(Message::NativeSpeed(FAST));
        ensure!(v.error.is_none(), "{:?}", v.error);
        let mut boundary = false;
        for _ in 0..10000 {
            let s = v.session.as_mut().unwrap();
            let call = serde_json::to_value(s.host.presentation.transport.as_ref().unwrap())?;
            if !call["call"]["speed"].is_null() && call["call"]["broadcast"] == true {
                ensure!(
                    call["call"]["publication"].get("world").is_some(),
                    "fresh transport did not use shared world publication"
                );
                boundary = true;
                break;
            }
            ensure!(
                s.transport_pending(),
                "speed command skipped its C2 boundary"
            );
            v.device.as_mut().unwrap().advance_transport(s, 1)?;
        }
        ensure!(boundary);
        v.save()?;
        std::fs::copy(&save, dir.join("speed-callback.json"))?;
        let before = state(&v)?;
        drain(&mut v)?;
        let completed = state(&v)?;
        v.load(&save)?;
        restored_state(&v, &before, &dir, "speed-pending")?;
        ensure!(v.clock_origin.is_none());
        drain(&mut v)?;
        restored_state(&v, &completed, &dir, "speed-completed")?;
        let mut speeds = vec![];
        let mut live_notice_inputs = vec![];
        let mut now = now;
        for speed in [NORMAL, FAST, ULTRA] {
            let _ = v.update(Message::NativeSpeed(speed));
            drain(&mut v)?;
            let s = v.session.as_ref().unwrap();
            ensure!(s.simulation.vm.world.globals[&16] == speed as i16);
            ensure!(s.simulation.vm.world.globals[&17] == 0);
            let timers = s
                .host
                .presentation
                .transport
                .as_ref()
                .unwrap()
                .controls
                .speed_timers
                .unwrap();
            ensure!(
                [timers.first.interval, timers.second.interval]
                    == sims_vm::interface::speed::intervals(speed)
            );
            let tick = s.simulation.vm.tick;
            for frame in 0..4 {
                now += std::time::Duration::from_millis(42);
                let _ = v.update(Message::Frame(now));
                drain(&mut v)?;
                let replies = answer_startup_dialogs(
                    &mut v,
                    &dir,
                    &format!("speed-{speed}-frame-{frame}"),
                    false,
                )?;
                if !replies.is_empty() {
                    live_notice_inputs.extend(replies);
                    let _ = v.update(Message::NativeSpeed(speed));
                    drain(&mut v)?;
                }
                ensure!(
                    v.session
                        .as_ref()
                        .unwrap()
                        .simulation
                        .vm
                        .world
                        .dialogs
                        .modal
                        .is_none()
                );
            }
            let after = v.session.as_ref().unwrap().simulation.vm.tick;
            ensure!(after > tick, "{name}: speed{speed} did not advance");
            speeds.push(json!({"speed":speed,"ticks":after-tick}));
        }
        let _ = v.update(Message::Toggle);
        drain(&mut v)?;
        let paused = v.session.as_ref().unwrap().simulation.vm.tick;
        let people = v.session.as_ref().unwrap().simulation.characters.clone();
        for _ in 0..3 {
            now += std::time::Duration::from_millis(42);
            let _ = v.update(Message::Frame(now));
            drain(&mut v)?;
        }
        ensure!(v.session.as_ref().unwrap().simulation.vm.tick == paused);
        ensure!(v.session.as_ref().unwrap().simulation.characters == people);
        let before = state(&v)?;
        render_headless_element(&root, &dir.join("paused.png"), v.view())?;
        ensure!(before == state(&v)?);
        v.save()?;
        let before = state(&v)?;
        v.load(&save)?;
        restored_state(&v, &before, &dir, "paused")?;
        let pending_notices: Vec<_> = v
            .session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .map(|r| r.title.clone())
            .collect();
        let paused_notice_inputs = answer_startup_dialogs(&mut v, &dir, "paused-notices", true)?;
        ensure!(
            v.session
                .as_ref()
                .unwrap()
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .is_none()
        );
        ensure!(!v.presentation_pending());
        // After those explicit responses, further paused frames must actually
        // complete, not merely wait behind a modal with the clock frozen.
        let people = v.session.as_ref().unwrap().simulation.characters.clone();
        for _ in 0..3 {
            now += std::time::Duration::from_millis(42);
            let _ = v.update(Message::Frame(now));
            drain(&mut v)?;
            ensure!(!v.presentation_pending());
        }
        ensure!(v.session.as_ref().unwrap().simulation.vm.tick == paused);
        ensure!(v.session.as_ref().unwrap().simulation.characters == people);
        let _ = v.update(Message::NativeSpeed(NORMAL));
        drain(&mut v)?;
        for _ in 0..4 {
            now += std::time::Duration::from_millis(42);
            let _ = v.update(Message::Frame(now));
            drain(&mut v)?;
        }
        let resumed = v.session.as_ref().unwrap().simulation.vm.tick;
        ensure!(resumed > paused && !v.presentation_pending());
        let before = state(&v)?;
        render_headless_element(&root, &dir.join("resumed.png"), v.view())?;
        ensure!(before == state(&v)?);
        let resumed_save = dir.join("resumed.json");
        v.args.save = resumed_save.clone();
        v.save()?;
        let before = state(&v)?;
        v.load(&resumed_save)?;
        restored_state(&v, &before, &dir, "resumed")?;
        let mut legacy_restored = false;
        let mut legacy_rejection = None;
        if name == "goth" {
            // Retained evidence from before the shared world caller was bound.
            // Loading it must not migrate/replay the already-suspended caller.
            let legacy = repo.join("cache/original-iced-clock-notices/goth/speed-callback.json");
            let original = std::fs::read(&legacy)?;
            let current = state(&v)?;
            match v.load(&legacy) {
                Ok(()) => {
                    let transport = serde_json::to_value(
                        v.session
                            .as_ref()
                            .unwrap()
                            .host
                            .presentation
                            .transport
                            .as_ref()
                            .unwrap(),
                    )?;
                    ensure!(transport["call"]["publication"].get("world").is_none());
                    ensure!(
                        transport["call"]["publication"]["pending"]
                            .get("SetSpeed")
                            .is_some()
                    );
                    let before = state(&v)?;
                    drain(&mut v)?;
                    let completed = state(&v)?;
                    v.load(&legacy)?;
                    restored_state(&v, &before, &dir, "legacy-speed-pending")?;
                    drain(&mut v)?;
                    restored_state(&v, &completed, &dir, "legacy-speed-completed")?;
                    ensure!(std::fs::read(&legacy)? == original);
                    std::fs::write(dir.join("legacy-speed-source.json"), original)?;
                    legacy_restored = true;
                }
                Err(error) => {
                    // This historical v1 fixture does not link to matching
                    // current program content. Keep the guard; its rejection
                    // is not a failure to restore the current managed save.
                    let error = format!("{error:#}");
                    ensure!(
                        error == "room snapshot version/content mismatch",
                        "unexpected historical checkpoint error: {error}"
                    );
                    restored_state(&v, &current, &dir, "legacy-rejection-preserves-current")?;
                    ensure!(std::fs::read(&legacy)? == original);
                    std::fs::write(
                        dir.join("legacy-rejection.json"),
                        serde_json::to_vec_pretty(&json!({
                            "source":legacy,"error":error,"current_session_preserved":true,
                            "original_file_unchanged":true,"legacy_compatibility_claimed":false
                        }))?,
                    )?;
                    legacy_rejection = Some(error);
                }
            }
        }
        std::fs::write(
            dir.join("report.json"),
            serde_json::to_vec_pretty(&json!({
                "tick_before":start,"tick_before_pause":paused,"tick_after":resumed,
                "speeds":speeds,"paused_frames_before_dialog_responses":3,
                "pending_notices":pending_notices,"initial_notice_inputs":initial_notice_inputs,
                "paused_notice_inputs":paused_notice_inputs,"live_notice_inputs":live_notice_inputs,
                "completed_paused_frames_after_responses":3,"resumed_ticks":resumed-paused,
                "speed_callback_cold_restore":true,"paused_people_unchanged":true,
                "final_cold_restore":true,"headless_render_preserves_state":true,
                "shared_world_transport":true,"legacy_speed_callback_cold_restore":legacy_restored,
                "historical_checkpoint_rejection":legacy_rejection
            }))?,
        )?;
        println!(
            "{name}: speeds, queued notices, completed paused frames and resumed cold saves passed; +{} ticks",
            resumed - start
        );
    }
    Ok(())
}
