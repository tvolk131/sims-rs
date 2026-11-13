//! Explicit captured process input through actual application restore/save.
use super::*;
use clap::Parser;
use serde_json::json;

fn retained(v: &Viewer) -> Result<Vec<u8>> {
    let s = v.session.as_ref().unwrap();
    Ok(serde_json::to_vec(
        &json!({"session":s.save_managed()?,"client":v.client.as_ref().unwrap().save_managed(s)?,
        "device":v.device.as_ref().unwrap().save(s)?}),
    )?)
}
#[test]
#[ignore = "private original Mashuga capture, managed checkpoint, immutable-only assets and offscreen Metal"]
fn captured_neighborhood_input_survives_cold_app_restore_without_logs_or_starting_saves()
-> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("neighborhood input test panicked")
}
fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_NEIGHBORHOOD_INPUT_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    ensure!(std::fs::read_dir(&root)?.all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("userdata")
    }));
    let input = repo.join("cache/controller-live-mashuga-20261022-sixth/live-session.json");
    let summary = output.join("summary.json");
    let log = output.join("observation.jsonl");
    std::fs::copy(
        repo.join("cache/gameplay-mashuga-neighborhood-summary-final.json"),
        &summary,
    )?;
    std::fs::copy(
        repo.join(
            "cache/research/sims-exe/parity-investigation-20261020/mashuga-house9-live.jsonl",
        ),
        &log,
    )?;
    let make = |file: &Path, capture: bool| -> Result<Viewer> {
        let mut args: Vec<String> = vec![
            "sims-room".into(),
            "--original".into(),
            "UserData2/Houses/House09.iff".into(),
            "--paused".into(),
            "--root".into(),
            root.to_str().unwrap().into(),
            "--load".into(),
            file.to_str().unwrap().into(),
            "--save".into(),
            output.join("bound.json").to_str().unwrap().into(),
        ];
        if capture {
            args.extend([
                "--neighborhood-observation".into(),
                log.to_str().unwrap().into(),
                "--neighborhood-summary".into(),
                summary.to_str().unwrap().into(),
            ]);
        }
        let args = Args::parse_from(args);
        let ready = Arc::new(Ready::load(&args)?);
        let (mut v, _) = Viewer::new(args);
        v.install(ready)?;
        Ok(v)
    };
    let control = make(&input, false)?;
    ensure!(
        control
            .session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .unwrap()
            .nearby_houses
            .is_none()
    );
    let mut bound = make(&input, true)?;
    let s = bound.session.as_ref().unwrap();
    let mut expected = control.session.as_ref().unwrap().simulation.clone();
    expected.vm.world.person.as_mut().unwrap().nearby_houses = Some(vec![]);
    ensure!(
        expected == s.simulation,
        "binding changed VM processing or RNG"
    );
    ensure!(s.host.presentation.neighborhood_input.is_some());
    let before = retained(&bound)?;
    let capture = s
        .host
        .presentation
        .neighborhood_input
        .as_ref()
        .unwrap()
        .clone();
    let mut conflicting = bound.args.clone();
    conflicting.neighborhood_policy = Some(neighborhood_usability::POLICY.into());
    ensure!(
        neighborhood_usability::bind_policy(bound.session.as_mut().unwrap(), &conflicting, false)
            .is_err(),
        "policy silently replaced retained capture"
    );
    ensure!(
        before == retained(&bound)?,
        "rejected policy request changed captured continuation"
    );
    ensure!(
        bound
            .session
            .as_mut()
            .unwrap()
            .bind_neighborhood_capture(capture)
            .is_err()
    );
    ensure!(
        before == retained(&bound)?,
        "duplicate input changed retained state"
    );
    bound.save()?;
    let gameplay_before = serde_json::to_vec(&bound.session.as_ref().unwrap().save_managed()?)?;
    std::fs::remove_file(&summary)?;
    std::fs::remove_file(&log)?;
    // Keep the same CLI paths: an owned binding must take precedence without
    // even reopening these now-missing import files.
    let cold = make(&output.join("bound.json"), true)?;
    // Viewer::install publishes one frame after restoring. Give the continuous
    // viewer that same presentation pass, rather than dropping generation fields
    // from equality. Device::refresh never runs a simulation slice.
    bound.refresh()?;
    ensure!(
        gameplay_before == serde_json::to_vec(&bound.session.as_ref().unwrap().save_managed()?)?,
        "matching presentation pass changed session/VM/RNG"
    );
    let before = retained(&bound)?;
    let actual = retained(&cold)?;
    if before != actual {
        std::fs::write(output.join("cold-expected.json"), &before)?;
        std::fs::write(output.join("cold-actual.json"), &actual)?;
        anyhow::bail!("cold owned input restore diverged; preserved both states");
    }
    render_headless_element(&root, &output.join("bound.png"), bound.view())?;
    render_headless_element(&root, &output.join("cold.png"), cold.view())?;
    ensure!(
        image::open(output.join("bound.png"))?.into_rgba8()
            == image::open(output.join("cold.png"))?.into_rgba8()
    );
    ensure!(
        before == retained(&bound)? && before == retained(&cold)?,
        "rendering changed retained input/state"
    );
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "tick":bound.session.as_ref().unwrap().simulation.vm.tick,"native_capture_lot":9,"neighborhood":2,
            "live_vector":[],"unknown_without_input":true,"only_vm_change_is_supplied_vector":true,
            "full_cold_retained_state_equal":true,"capture_files_removed_before_restore":true,"original_starting_save_namespace_absent":true,
            "headless_pixels_equal":true,"draw_preserves_state":true,"strict_original_parity_ticks_added":0
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private fresh original assets, existing immutable-only root and managed player save; source-only definition"]
fn installed_house_policy_fresh_binding_retained_restore_cancel_and_fault_recovery() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(installed_policy_run)?
        .join()
        .expect("installed-house policy integration test panicked")
}

fn installed_policy_run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let relative = std::path::PathBuf::from(std::env::var("SIMS_NEIGHBORHOOD_POLICY_OUTPUT")?);
    ensure!(
        relative.starts_with("cache")
            && relative
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
        "policy test output must stay in cache"
    );
    let output = repo.join(relative);
    std::fs::create_dir(&output).context("refusing to overwrite policy evidence")?;
    let root = repo.join("game-data/cabinet-extracted");
    let immutable_root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    ensure!(
        std::fs::read_dir(&immutable_root)?.all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("userdata")),
        "restore root unexpectedly contains starting UserData"
    );
    let source = std::env::var("SIMS_NEIGHBORHOOD_POLICY_SOURCE")
        .unwrap_or_else(|_| "UserData/Houses/House07.iff".into());
    let mut args = Args::try_parse_from([
        "sims-room",
        "--original",
        &source,
        "--root",
        root.to_str().unwrap(),
        "--paused",
        "--save",
        output.join("player.json").to_str().unwrap(),
    ])?;
    let control = Ready::load(&args)?;
    args.neighborhood_policy = Some(neighborhood_usability::POLICY.into());
    let ready = Arc::new(Ready::load(&args)?);
    {
        let control = control.session.lock().unwrap();
        let mut expected = control.as_ref().unwrap().simulation.clone();
        let owner = ready.session.lock().unwrap();
        let actual = owner.as_ref().unwrap();
        let replacement = actual
            .host
            .presentation
            .neighborhood_replacement
            .as_ref()
            .context("fresh replacement absent")?;
        expected
            .vm
            .world
            .person
            .as_mut()
            .context("person registry absent")?
            .nearby_houses = Some(replacement.nearby_houses().to_vec());
        ensure!(
            expected == actual.simulation,
            "fresh binding changed simulation/tick/RNG beyond supplied vector"
        );
        ensure!(
            actual.host.presentation.neighborhood_input.is_none(),
            "replacement mislabeled as captured process input"
        );
    }
    let (mut bound, task) = Viewer::new(args.clone());
    drop(task);
    bound.install(ready)?;
    ensure!(bound.application.provenance == neighborhood_usability::POLICY);
    let before = retained(&bound)?;
    let last = bound.last;
    drop(bound.update(Message::Options));
    drop(bound.update(Message::RequestApplicationChoice(
        neighborhood_usability::Choice::BuyPreview,
    )));
    let confirmation = bound
        .application_confirmation
        .context("confirmation absent")?;
    drop(bound.update(Message::Frame(Instant::now())));
    drop(bound.update(Message::CancelApplicationChoice(confirmation)));
    ensure!(
        retained(&bound)? == before && bound.last == last,
        "cancel changed complete retained state or clock input"
    );
    ensure!(!bound.args.buy_preview, "cancel staged configuration");
    bound.save()?;
    let mut cold_args = args;
    cold_args.root = immutable_root.clone();
    cold_args.load = Some(output.join("player.json"));
    cold_args.inferred_routing = true;
    let cold_ready = Arc::new(Ready::load(&cold_args)?);
    let (mut cold, task) = Viewer::new(cold_args);
    drop(task);
    cold.install(cold_ready)?;
    ensure!(
        cold.application.routing.as_deref() == Some("Strict"),
        "status used launch args rather than retained session routing"
    );
    ensure!(cold.application.provenance == neighborhood_usability::POLICY);
    bound.refresh()?;
    ensure!(
        retained(&bound)? == retained(&cold)?,
        "cold managed policy restore changed complete continuation"
    );
    let before = retained(&cold)?;
    cold.args.neighborhood_policy = None;
    cold.args.neighborhood_observation = Some(output.join("never-created-observation.jsonl"));
    cold.args.neighborhood_summary = Some(output.join("never-created-summary.json"));
    let error = cold
        .load(&output.join("player.json"))
        .expect_err("captured request must conflict with retained replacement");
    ensure!(format!("{error:#}").contains("conflicts with retained neighborhood replacement"));
    ensure!(
        retained(&cold)? == before,
        "rejected capture request changed retained state"
    );
    cold.args.neighborhood_observation = None;
    cold.args.neighborhood_summary = None;
    cold.args.neighborhood_policy = Some(neighborhood_usability::POLICY.into());
    cold.fail("synthetic application-error recovery guard, not an observed VM fault");
    for message in [Message::Toggle, Message::Step, Message::Save] {
        drop(cold.update(message));
        ensure!(
            cold.error.is_some() && retained(&cold)? == before,
            "faulted owner accepted gameplay or save work"
        );
    }
    drop(cold.update(Message::Load));
    ensure!(
        cold.error.is_none(),
        "pre-fault managed Load remained inaccessible: {:?}",
        cold.error
    );
    ensure!(cold.application.provenance == neighborhood_usability::POLICY);
    let player_save_before = std::fs::read(output.join("player.json"))?;
    {
        let session = cold.session.as_mut().unwrap();
        let thread = session
            .simulation
            .vm
            .threads
            .values_mut()
            .next()
            .context("fault-guard test requires an actual retained thread")?;
        thread.status = sims_vm::Status::Faulted(sims_vm::Fault {
            message: "synthetic fault-guard regression, not an observed gameplay fault".into(),
            stack: thread
                .stack
                .iter()
                .map(|frame| frame.location.clone())
                .collect(),
        });
    }
    let faulted_before = retained(&cold)?;
    ensure!(cold.error.is_none() && cold.menu_error.is_none());
    let error = cold
        .save()
        .expect_err("faulted VM must not overwrite player save even without app error");
    ensure!(format!("{error:#}").contains("faulted VM"));
    ensure!(
        std::fs::read(output.join("player.json"))? == player_save_before,
        "failed faulted Save changed player-save bytes"
    );
    ensure!(
        retained(&cold)? == faulted_before,
        "failed faulted Save changed session/client/device/tick/RNG"
    );
    drop(cold.update(Message::Load));
    ensure!(
        cold.error.is_none(),
        "could not restore last known-good save after synthetic fault"
    );
    std::fs::write(
        output.join("source-integration-report.json"),
        serde_json::to_vec_pretty(&json!({
            "requested_policy":neighborhood_usability::POLICY,"retained":cold.application,
            "immutable_only_restore":true,"capture_files_never_created":true,
            "cancel_complete_retained_state_equal":true,"synthetic_application_error_recovery_only":true,
            "synthetic_VM_fault_save_guard_tested":true,"failed_save_bytes_unchanged":true,
            "actual_VM_fault_recovery_tested":false,"gameplay_journey_tested":false,
            "full_player_path_qualified":false,"strict_original_parity_ticks":0,
        }))?,
    )?;
    Ok(())
}
