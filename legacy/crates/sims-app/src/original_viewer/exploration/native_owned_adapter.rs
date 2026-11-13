//! Private native-owned import adapter. Never reruns house startup or invents
//! missing process owners. Frontend binding is a separate, observed input.
use super::*;
use clap::Parser;

fn differences(before: &Value, after: &Value, path: &str, out: &mut Vec<Value>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let path = format!("{path}/{key}");
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => differences(a, b, &path, out),
                    (a, b) => out.push(json!({"path":path,"before":a,"after":b,
                        "before_present":a.is_some(),"after_present":b.is_some()})),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                differences(a, b, &format!("{path}/{i}"), out);
            }
        }
        _ => out.push(json!({"path":path,"before":before,"after":after})),
    }
}
fn state(session: &Session) -> Result<Value> {
    Ok(serde_json::to_value(session.save())?)
}
fn fingerprint(value: &Value) -> Result<String> {
    Ok(engine::digest(&serde_json::to_vec(value)?))
}
fn owners(s: &Session) -> Value {
    let vm = &s.simulation.vm;
    json!({"tick":vm.tick,"rng":[vm.rng_state,vm.lot_rng_state,vm.crt_rng_state],
        "lot_rng_scope":vm.lot_rng_scope,"map_size":vm.world.map_size,
        "physical_stride":s.simulation.architecture.as_ref().and_then(|a|a.original.as_ref()).map(|a|a.size),
        "registry_order":vm.world.registry_order,"routing":s.options.routing,
        "clock":s.host.presentation.clock,"transport":s.host.presentation.transport,
        "selected":s.host.presentation.selected,
        "native_selected":s.simulation.native_selected_person()})
}
fn preserve_binding_failure(
    s: &Session,
    device: &CameraDevice,
    client: &DialogClient,
    output: &Path,
    stage: &str,
    error: &anyhow::Error,
) -> Result<()> {
    write_json(
        &output.join("binding-failure.json"),
        &json!({"stage":stage,
        "error":format!("{error:#}"),"owners":owners(s),"strict_native_parity_ticks":0}),
    )?;
    match s.save_managed() {
        Ok(saved) => write_json(&output.join("binding-failure-managed-session.json"), &saved)?,
        Err(error) => {
            write_json(
                &output.join("binding-failure-research-session.json"),
                &s.save(),
            )?;
            write_json(
                &output.join("binding-failure-managed-capture-error.json"),
                &format!("{error:#}"),
            )?;
        }
    }
    match device.save(s) {
        Ok(saved) => write_json(&output.join("binding-failure-device.json"), &saved)?,
        Err(error) => write_json(
            &output.join("binding-failure-device-capture-error.json"),
            &format!("{error:#}"),
        )?,
    }
    match client.save(s) {
        Ok(saved) => write_json(&output.join("binding-failure-client.json"), &saved)?,
        Err(error) => write_json(
            &output.join("binding-failure-client-capture-error.json"),
            &format!("{error:#}"),
        )?,
    }
    Ok(())
}
#[test]
fn difference_observer_records_added_owners_and_nested_vm_changes() {
    let a = json!({"vm":{"rng":[1,2,3]},"owner":null});
    let b = json!({"vm":{"rng":[1,7,3]},"owner":{"serial":4}});
    let mut rows = vec![];
    differences(&a, &b, "", &mut rows);
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .any(|r| r["path"] == "/vm/rng/1" && r["before"] == 2 && r["after"] == 7)
    );
    assert!(rows.iter().any(|r| r["path"] == "/owner"));
    rows.clear();
    differences(&a, &a, "", &mut rows);
    assert!(rows.is_empty());
    differences(&json!({}), &json!({"owner":null}), "", &mut rows);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["before_present"], false);
    assert_eq!(rows[0]["after_present"], true);
}

#[test]
#[ignore = "private actual retained session/device/client; no GUI, guest or starting-save reload"]
fn actual_native_owned_session_adapts_without_startup_or_owner_reconstruction() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("native-owned adapter panicked")
}
fn run() -> Result<()> {
    let repo = PathBuf::from(std::env::var("SIMS_NATIVE_ADAPTER_REPO")?).canonicalize()?;
    let input = repo
        .join(std::env::var("SIMS_NATIVE_ADAPTER_INPUT")?)
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_NATIVE_ADAPTER_OUTPUT")?);
    ensure!(
        input.starts_with(repo.join("cache")) && output.starts_with(repo.join("cache")),
        "adapter inputs/output must remain private research cache"
    );
    std::fs::create_dir(&output)?;
    let result = adapt(&repo, &input, &output);
    if let Err(error) = &result {
        write_json(
            &output.join("first-boundary-failure.json"),
            &json!({"error":format!("{error:#}"),"input":input,
                "original_engine_executed":false,"strict_native_parity_ticks":0}),
        )?;
    }
    result
}
fn adapt(repo: &Path, input: &Path, output: &Path) -> Result<()> {
    let session_bytes = std::fs::read(input.join("opened-managed-session.json"))?;
    let client_bytes = std::fs::read(input.join("opened-client.json"))?;
    let device_bytes = std::fs::read(input.join("opened-device.json"))?;
    let original: Saved = serde_json::from_slice(&session_bytes)?;
    ensure!(
        original.version == 2
            && original.options.routing == sims_sim::lot_route::path::Policy::Strict,
        "requires actual managed strict input"
    );
    let root = repo.join("cache/native-parity-guest-20261023-second/assets-immutable");
    let index = Index::scan(&root)?;
    let mut session = Session::restore(&index, original.clone())?;
    // The old opened fixture has no definition provenance. Restoration must
    // retain its exact raw-word program, including after a real managed encode/
    // decode. A forged profile never authorizes rewriting its saved hash.
    ensure!(original.normalized_object_definitions.is_empty());
    ensure!(
        session
            .program
            .modules
            .values()
            .all(|module| !module.normalized_object_definitions)
    );
    let recaptured = session.save_managed()?;
    let encoded = serde_json::to_vec(&recaptured)?;
    let decoded: Saved = serde_json::from_slice(&encoded)?;
    ensure!(decoded.normalized_object_definitions.is_empty());
    ensure!(
        !serde_json::to_value(&decoded)?
            .as_object()
            .unwrap()
            .contains_key("normalized_object_definitions")
    );
    let roundtrip = Session::restore(&index, decoded)?;
    ensure!(
        state(&roundtrip)? == state(&session)?,
        "old managed profile silently upgraded"
    );
    drop(roundtrip);
    let mut bad = serde_json::to_value(&recaptured)?;
    bad["normalized_object_definitions"] = json!(["missing.iff", "missing.iff"]);
    let duplicate_error = serde_json::from_value::<Saved>(bad.clone())
        .unwrap_err()
        .to_string();
    ensure!(duplicate_error.contains("duplicate normalized definition module"));
    bad["normalized_object_definitions"] = json!(["missing.iff"]);
    let unknown = Session::restore(&index, serde_json::from_value(bad)?)
        .err()
        .context("unknown definition profile accepted")?;
    ensure!(format!("{unknown:#}").contains("normalized definition module absent"));
    let changed = session.program.modules.iter().find_map(|(name, module)| {
        module
            .object_definition_words
            .values()
            .any(|words| words.len() != 108 || matches!(words.first(), Some(136 | 137)))
            .then_some(name.clone())
    });
    // This opened Newbie fixture currently has only complete version138
    // definitions. Preserve that fact rather than claiming a changing-profile
    // negative; the later real Cursors128 Buy preview supplies that boundary.
    let mismatch = if let Some(source) = &changed {
        let mut forged = recaptured;
        forged.normalized_object_definitions.insert(source.clone());
        let error = Session::restore(&index, forged)
            .err()
            .context("forged normalized profile bypassed retained program fingerprint")?;
        ensure!(
            error.to_string() == "room snapshot version/content mismatch",
            "forged profile rejected outside retained fingerprint guard: {error:#}"
        );
        Some(format!("{error:#}"))
    } else {
        None
    };
    write_json(
        &output.join("historical-definition-profile-proof.json"),
        &json!({
            "old_profile_absent":true,"managed_encode_decode_exact":true,
            "unknown_profile_error":format!("{unknown:#}"),"duplicate_profile_error":duplicate_error,
            "forged_profile_source":changed,"forged_profile_error":mismatch,
            "baseline_changed_profile_negative_available":changed.is_some(),
            "changed_profile_negative_scope":"portable extent cases and actual mixed Cursors128 Buy preview",
            "saved_program_hash_unchanged":true,"strict_native_parity_ticks":0
        }),
    )?;
    ensure!(
        session.simulation.vm.world.map_size.is_some()
            && session.simulation.vm.world.registry_order.is_some(),
        "actual native map/registry owner absent"
    );
    let mut expected = serde_json::to_value(original)?;
    expected["version"] = json!(1);
    expected.as_object_mut().unwrap().remove("managed");
    let restored = state(&session)?;
    let mut diff = vec![];
    differences(&expected, &restored, "", &mut diff);
    write_json(&output.join("restored-session-differences.json"), &diff)?;
    ensure!(diff.is_empty(), "managed restore changed retained session");
    drop(expected);
    let original_client: sims_render::dialog::client::Saved =
        serde_json::from_slice(&client_bytes)?;
    let original_device: sims_render::camera_device::Saved = serde_json::from_slice(&device_bytes)?;
    let mut client = DialogClient::restore_managed(&session, original_client.clone())?;
    let mut device = CameraDevice::restore(&session, original_device.clone())?;
    ensure!(
        serde_json::to_value(client.save_managed(&session)?)?
            == serde_json::to_value(original_client)?,
        "actual client changed on restore"
    );
    ensure!(
        serde_json::to_value(device.save(&session)?)? == serde_json::to_value(original_device)?,
        "actual device changed on restore"
    );
    ensure!(
        state(&session)? == restored,
        "frontend restore changed session"
    );
    write_json(
        &output.join("restore-proof.json"),
        &json!({
        "session_sha256":engine::digest(&session_bytes),"client_sha256":engine::digest(&client_bytes),
        "device_sha256":engine::digest(&device_bytes),"restored_state_sha256":fingerprint(&restored)?,
        "owners":owners(&session),"session_equal":true,"client_equal":true,"device_equal":true,
        "starting_save_loaded":false,"original_engine_executed":false,"strict_native_parity_ticks":0}),
    )?;

    // These are ordinary fresh replacement frontend bindings, not native state
    // reconstructed from policy metadata. Preserve all their actual changes.
    ensure!(
        session.host.presentation.clock.is_none(),
        "elapsed clock already bound"
    );
    let before_timing = serde_json::to_value(
        session
            .host
            .presentation
            .transport
            .as_ref()
            .context("retained transport absent")?
            .timing,
    )?;
    if let Err(error) = session.bind_clock(0) {
        preserve_binding_failure(&session, &device, &client, output, "clock-bind", &error)?;
        return Err(error.context("ordinary clock binding failed"));
    }
    let bound_clock = session
        .host
        .presentation
        .clock
        .as_ref()
        .context("clock binding absent")?;
    ensure!(
        serde_json::to_value(bound_clock.fields.timing)? == before_timing,
        "fresh clock did not transfer actual transport timing"
    );
    ensure!(
        session
            .host
            .presentation
            .transport
            .as_ref()
            .unwrap()
            .timing
            .is_none(),
        "transport retained duplicate timing owner"
    );
    let after_clock = state(&session)?;
    ensure!(
        restored["simulation"] == after_clock["simulation"]
            && restored["assets"] == after_clock["assets"],
        "clock binding changed simulation/assets"
    );
    let mut clock_changes = vec![];
    differences(&restored, &after_clock, "", &mut clock_changes);
    write_json(
        &output.join("clock-binding-differences.json"),
        &clock_changes,
    )?;
    if let Err(error) = play::bind(&mut session, &mut device, &mut client) {
        preserve_binding_failure(&session, &device, &client, output, "play-bind", &error)?;
        return Err(error.context("ordinary play binding failed"));
    }
    let after_binding = state(&session)?;
    let mut changes = vec![];
    differences(&after_clock, &after_binding, "", &mut changes);
    write_json(&output.join("play-binding-differences.json"), &changes)?;
    write_json(
        &output.join("frontend-binding-proof.json"),
        &json!({
        "before_state_sha256":fingerprint(&restored)?,"after_clock_sha256":fingerprint(&after_clock)?,
        "after_play_sha256":fingerprint(&after_binding)?,"owners":owners(&session),
        "play_changed_vm":after_clock["simulation"]["state"]["vm"]!=after_binding["simulation"]["state"]["vm"],
        "starting_save_loaded":false,"strict_native_parity_ticks":0}),
    )?;
    ensure!(
        after_clock["simulation"]["state"]["vm"]["tick"]
            == after_binding["simulation"]["state"]["vm"]["tick"],
        "frontend binding advanced gameplay"
    );
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
        "--root",
        root.to_str().unwrap(),
        "--save",
        output.join("unused-player-save.json").to_str().unwrap(),
    ]);
    // Discard the unpolled loading task: only the already-restored objects below
    // enter this Viewer. Ready::load/install/selection are never executed.
    let (mut viewer, loading_task) = Viewer::new(args);
    drop(loading_task);
    viewer.display_size = [device.scene().width, device.scene().height];
    viewer.floor = device.controls().floor;
    viewer.zoom = (1u32 << device.controls().zoom) as f32 / 8.;
    viewer.resident = session
        .host
        .presentation
        .selected
        .context("retained viewer selected person absent")?;
    ensure!(
        session.house.household.as_ref().is_some_and(|h| h
            .residents
            .iter()
            .any(|r| r.object as i16 == viewer.resident)),
        "retained viewer selection is not a resident"
    );
    viewer.playing = session
        .host
        .presentation
        .transport
        .as_ref()
        .unwrap()
        .controls
        .paused
        == 0;
    viewer.scene = Some(device.scene().clone());
    viewer.icons = Some(Arc::new(dialog_view::Icons::load(&index)?));
    viewer.session = Some(session);
    viewer.client = Some(client);
    viewer.device = Some(device);
    let epoch = Instant::now();
    viewer.last = Some(epoch);
    viewer.clock_origin = Some((
        epoch,
        viewer
            .session
            .as_ref()
            .unwrap()
            .host
            .presentation
            .clock
            .as_ref()
            .unwrap()
            .now,
    ));
    let mut app = Application {
        viewer,
        epoch,
        millis: 0,
        paths: Default::default(),
        progress_burst_ticks: 0,
        camera_pointer: Default::default(),
    };
    ensure!(
        state(app.session()?)? == after_binding,
        "constructing Viewer changed retained session"
    );
    drop(restored);
    drop(after_clock);
    drop(after_binding);
    let checkpoint = app.checkpoint()?;
    // Use the immutable install index, not this live Session's owned namespace.
    let cold = Application::restore_checkpoint(
        &app.viewer.args,
        &index,
        app.viewer.icons.as_ref().unwrap(),
        epoch,
        &checkpoint,
    )?;
    ensure!(
        app.observe()? == cold.observe()?,
        "adapted application cold observation differs"
    );
    ensure!(
        app.checkpoint()? == cold.checkpoint()?,
        "adapted full cold checkpoint differs"
    );
    std::fs::write(output.join("attached-before-input.json"), checkpoint)?;
    // The actual native pause command and real display event are ordinary input;
    // neither is silently applied while asserting restore neutrality.
    let inputs = (|| -> Result<()> {
        app.execute(&Command::Playing { value: false }, &Limits::default())?;
        app.execute(
            &Command::Resize {
                width: 1320,
                height: 900,
            },
            &Limits::default(),
        )?;
        app.drain(&Limits::default())
    })();
    if let Err(error) = inputs {
        match app.checkpoint() {
            Ok(bytes) => std::fs::write(output.join("failed-input-checkpoint.json"), bytes)?,
            Err(capture) => write_json(
                &output.join("failed-input-checkpoint-error.json"),
                &format!("{capture:#}"),
            )?,
        }
        preserve_binding_failure(
            app.session()?,
            app.viewer.device.as_ref().unwrap(),
            app.viewer.client.as_ref().unwrap(),
            output,
            "pause-resize",
            &error,
        )?;
        return Err(error.context("ordinary pause/resize failed"));
    }
    let safe = app.checkpoint()?;
    // Preserve the real returned boundary even if a following readiness/cold
    // assertion discovers a modal or another unsupported frontend owner.
    std::fs::write(output.join("after-input-checkpoint.json"), &safe)?;
    let cold = Application::restore_checkpoint(
        &app.viewer.args,
        &index,
        app.viewer.icons.as_ref().unwrap(),
        epoch,
        &safe,
    )?;
    ensure!(
        app.observe()? == cold.observe()? && safe == cold.checkpoint()?,
        "safe paused frontend cold restore differs"
    );
    ensure!(
        !app.viewer.playing
            && !app.viewer.presentation_pending()
            && app.session()?.simulation.vm.world.dialogs.modal.is_none(),
        "ordinary frontend input not ready"
    );
    ensure!(
        app.session()?.options.routing == sims_sim::lot_route::path::Policy::Strict
            && app.session()?.simulation.vm.world.map_size.is_some()
            && app.session()?.simulation.vm.world.registry_order.is_some(),
        "native owners/policy lost through input"
    );
    std::fs::hard_link(
        output.join("after-input-checkpoint.json"),
        output.join("ready-checkpoint.json"),
    )?;
    write_json(
        &output.join("ready-proof.json"),
        &json!({"owners":owners(app.session()?),
        "observation":app.observe()?,"cold_equal":true,"ordinary_pause_resize":true,
        "strict_native_parity_ticks":0,"scope":"fresh native-owned state; frontend binding/internal continuation, not an original-input parity claim"}),
    )?;
    Ok(())
}
