use super::{Args, Message, Options, Ready, Session, Viewer, player_timing};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use sims_sim::lot_route::path::Policy;
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

#[path = "strict_startup_probe/tutorial_entry.rs"]
mod tutorial_entry;

const NEWBIE_SOURCE: &str = "UserData/Houses/House07.iff";
const RECEIPT: &str = "receipt.private.json";
const HELP: &str = "Private, non-GUI diagnostic only. Set SIMS_STRICT_STARTUP_ROOT to an explicit absolute installation directory and SIMS_STRICT_STARTUP_OUTPUT to a fresh absolute directory below this checkout's ignored cache (its parent must exist). Run only original_viewer::strict_startup_probe::ordinary_strict_newbie_ready_load with --exact --ignored --nocapture --test-threads=1 under an external whole-process timeout with child cleanup. No internal loader/service budgets are changed. Outputs and failure chains are private; never upload or promote them as startup, native, performance or alpha acceptance.";
const FIRST_TICK_HELP: &str = "Private simulated/headless first-tick diagnostic, not native-window or performance evidence. Use the same explicit readonly parent-pinned root and fresh exclusive output as the Ready-only probe. Run only original_viewer::strict_startup_probe::ordinary_strict_newbie_first_tick with --exact --ignored --nocapture --test-threads=1 under a parent-owned external whole-process timeout with child cleanup. Production handler/service/VM budgets are unchanged. No modal responses, Save or Continue publication. All qualification remains false.";
const FIRST_TICK_MAX_FRAMES: u32 = 600;
const FIRST_TICK_FRAME_MILLIS: u64 = 16;

fn launcher_arguments(root: &Path, output: &Path) -> Vec<OsString> {
    vec![
        "sims-room".into(),
        "--original".into(),
        NEWBIE_SOURCE.into(),
        "--root".into(),
        root.as_os_str().to_owned(),
        "--save".into(),
        output.join("player-save.private.json").into_os_string(),
        "--paused".into(),
    ]
}

fn validate_arguments(args: &Args, root: &Path, output: &Path) -> Result<()> {
    ensure!(
        args.original.as_deref() == Some(NEWBIE_SOURCE)
            && args.root == root
            && args.save == output.join("player-save.private.json")
            && args.load.is_none()
            && args.paused
            && !args.inferred_routing
            && !args.buy_preview
            && args.neighborhood_policy.is_none()
            && args.neighborhood_observation.is_none()
            && args.neighborhood_summary.is_none()
            && args.neighborhood_sample == 0
            && args.at_tick == 0
            && args.screenshot.is_none()
            && args.exploration_plan.is_none()
            && args.window_session_output.is_none(),
        "probe arguments must match the ordinary paused Strict Newbie launcher without fallbacks"
    );
    ensure!(
        Options::default().routing == Policy::Strict,
        "default routing is no longer Strict"
    );
    Ok(())
}

fn reject_symlinks(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        ensure!(
            !fs::symlink_metadata(ancestor)?.file_type().is_symlink(),
            "private output has a symlink component: {}",
            ancestor.display()
        );
    }
    Ok(())
}

fn prepare_output(root: &Path, output: &Path, private_cache: &Path) -> Result<(PathBuf, PathBuf)> {
    ensure!(
        root.is_absolute() && output.is_absolute(),
        "root and output must be absolute; {HELP}"
    );
    ensure!(
        output
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_))),
        "output must not contain parent or current-directory components"
    );
    let root = root
        .canonicalize()
        .context("resolve explicit installation root")?;
    ensure!(root.is_dir(), "installation root is not a directory");
    let parent = output.parent().context("output has no parent")?;
    reject_symlinks(parent)?;
    let output = parent
        .canonicalize()?
        .join(output.file_name().context("output has no directory name")?);
    let private_cache = private_cache
        .canonicalize()
        .context("ignored private cache must already exist")?;
    ensure!(
        output.starts_with(&private_cache) && output != private_cache,
        "output must be below this checkout's ignored cache"
    );
    ensure!(
        !output.starts_with(&root) && !root.starts_with(&output),
        "output must not equal, contain, or be inside the installation root"
    );
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&output)
        .context("output must be fresh, exclusive, and not a symlink")?;
    Ok((root, output))
}

fn write_receipt(output: &Path, name: &str, receipt: &Value) -> Result<()> {
    reject_symlinks(output)?;
    let bytes = serde_json::to_vec_pretty(receipt)?;
    let pending = output.join(format!(".{name}.pending"));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&pending)
        .context("reserve private receipt staging file")?;
    file.write_all(&bytes)
        .context("write complete private receipt")?;
    file.write_all(b"\n")?;
    file.sync_all().context("sync complete private receipt")?;
    drop(file);
    fs::hard_link(&pending, output.join(name))
        .context("publish complete receipt without overwriting any existing path")?;
    if let Err(error) = fs::remove_file(&pending) {
        eprintln!("Complete private receipt published; staging cleanup failed: {error}");
    }
    Ok(())
}

fn session_metadata(session: &Session) -> Value {
    let world = &session.simulation.vm.world;
    let process = world.autonomy_constants.as_ref();
    let provider = session
        .host
        .autonomy_resources
        .as_ref()
        .map(|provider| provider.binding());
    json!({
        "source": session.house.source,
        "routing": session.options.routing,
        "initial_process_profile": session.options.initial_process_profile,
        "process_owner_present": process.is_some(),
        "process_profile": process.and_then(|owner| owner.configuration.profile),
        "process_source": process.and_then(|owner| owner.configuration.source.as_ref()),
        "process_input_evidence_present": process.is_some_and(|owner| owner.input_evidence.is_some()),
        "process_loaded_gate_known": process.is_some_and(|owner| owner.process.loaded.is_some()),
        "process_tuning_words_known": process.is_some_and(|owner| owner.process.words.iter().all(Option::is_some)),
        "process_provider_present": provider.is_some(),
        "process_provider_binding": provider,
        "process_provider_matches_owner": provider.as_ref().is_some_and(|binding| process.is_some_and(|owner| owner.configuration.source.as_ref() == Some(binding))),
        "retained_native_map_present": world.map_size.is_some(),
        "native_lot_present": session.simulation.native_lot.is_some(),
        "native_lot_routing": session.simulation.native_lot.as_ref().map(|lot| lot.routing_policy),
        "crt_stream_present": session.simulation.vm.crt_rng_state.is_some(),
        "buy_frontend_present": session.host.presentation.tool_selection.is_some(),
        "neighborhood_replacement_present": session.host.presentation.neighborhood_replacement.is_some(),
        "neighborhood_capture_present": session.host.presentation.neighborhood_input.is_some(),
        "production_options_match": session.options == Options {
            native_postload: true, native_load_completion: true, ..Options::default()
        }
    })
}

fn inspect_ready(ready: &Ready) -> Result<Value> {
    let session = ready
        .session
        .lock()
        .map_err(|_| anyhow::anyhow!("Ready session mutex poisoned"))?;
    let session = session
        .as_ref()
        .context("Ready returned without a fresh session")?;
    let metadata = session_metadata(session);
    ensure!(
        session.house.source.eq_ignore_ascii_case(NEWBIE_SOURCE),
        "Ready source is not the actual Newbie source"
    );
    ensure!(
        session.options.routing == Policy::Strict,
        "actual session routing is not Strict"
    );
    ensure!(
        session
            .simulation
            .native_lot
            .as_ref()
            .is_some_and(|lot| lot.routing_policy == Policy::Strict),
        "actual native lot routing is not Strict"
    );
    ensure!(
        metadata["production_options_match"] == true,
        "Ready changed ordinary production Options defaults"
    );
    ensure!(
        session.host.presentation.tool_selection.is_none(),
        "Buy preview was attached"
    );
    ensure!(
        session.host.presentation.neighborhood_replacement.is_none(),
        "neighborhood fallback was attached"
    );
    ensure!(
        session.host.presentation.neighborhood_input.is_none(),
        "unexpected neighborhood capture was attached"
    );
    ensure!(
        ready.restore_index.is_none(),
        "Ready entered managed restore instead of a fresh load"
    );
    ensure!(
        ready
            .client
            .lock()
            .map_err(|_| anyhow::anyhow!("Ready client mutex poisoned"))?
            .is_some(),
        "Ready dialog client absent"
    );
    ensure!(
        ready
            .device
            .lock()
            .map_err(|_| anyhow::anyhow!("Ready device mutex poisoned"))?
            .is_some(),
        "Ready camera device absent"
    );
    Ok(metadata)
}

fn failure(boundary: &str, error: &anyhow::Error) -> Value {
    json!({"boundary": boundary, "message": error.to_string(), "full_chain": format!("{error:#}"),
        "chain": error.chain().map(ToString::to_string).collect::<Vec<_>>()})
}

fn loader_record(timing: &Value) -> Option<&Value> {
    timing
        .get("rows")?
        .as_array()?
        .iter()
        .find(|row| row["scope"] == "loader_execution")?
        .get("loader")
}

fn frame_input_millis(frame: u32) -> Option<u64> {
    (frame < FIRST_TICK_MAX_FRAMES).then(|| u64::from(frame) * FIRST_TICK_FRAME_MILLIS)
}

fn viewer_observation(viewer: &Viewer, epoch: Instant) -> Value {
    let mut observation = json!({
        "tick": viewer.session.as_ref().map(|session| session.simulation.vm.tick),
        "playing": viewer.playing, "status": viewer.status,
        "error": viewer.error, "menu_error": viewer.menu_error,
        "resident": viewer.resident, "selected": viewer.selected,
        "last_input_ms": viewer.last.map(|now| now.saturating_duration_since(epoch).as_millis()),
        "clock_origin": viewer.clock_origin.map(|(now, offset)| json!({
            "input_ms": now.saturating_duration_since(epoch).as_millis(), "offset": offset
        })),
        "accumulator": viewer.accumulator,
        "native_transport": viewer.native_transport(), "native_clock": viewer.native_clock(),
        "presentation_pending": viewer.presentation_pending(),
        "presentation_children_pending": viewer.presentation_children_pending(),
        "dialog_client_pending": viewer.client.as_ref().is_some_and(|client| client.pending()),
        "camera_device_present": viewer.device.is_some(),
        "camera_frame_pending": viewer.device.as_ref().is_some_and(|device| device.normal_view_frame_pending()),
        "application_confirmation_present": viewer.application_confirmation.is_some(),
        "viewport_enter_requested": viewer.viewport_enter_requested,
        "queued_move_inputs": viewer.move_inputs.len()
    });
    if let Some(session) = &viewer.session {
        observation["session"] = session_metadata(session);
        observation["clock_now"] = json!(
            session
                .host
                .presentation
                .clock
                .as_ref()
                .map(|clock| clock.now)
        );
        observation["lot_clock_ticks"] = json!(
            session
                .simulation
                .household
                .as_ref()
                .map(|household| household.clock.ticks)
        );
        observation["native_selected_person"] = json!(
            session
                .simulation
                .native_lot
                .as_ref()
                .and_then(|lot| lot.selected_person)
        );
        observation["flags"] = json!({
            "modal": session.simulation.vm.world.dialogs.modal,
            "player_input_pending": session.player_input_pending(),
            "player_input_blocks_ticks": session.player_input_blocks_ticks(),
            "purchase_cursor_pending": session.purchase_cursor_pending(), "move_tool_pending": session.move_tool_pending(),
            "move_button_pending": session.move_button_pending(), "mode_publication_pending": session.mode_publication_pending(),
            "object_pickup_pending": session.simulation.object_pickup.is_some(),
            "object_placement_pending": session.simulation.object_placement.is_some(),
            "cursor_factory_pending": session.simulation.cursor_factory.is_some(),
            "running_menu": session.running_menu(), "menu_command_pending": session.menu_command_pending(),
            "clock_pending": session.clock_pending(), "transport_pending": session.transport_pending(),
            "selection_pending": session.simulation.selection_work_pending(),
            "house_notification_pending": session.house_notification_pending(),
            "ui_wait_pending": session.ui_wait_pending(), "ui_portrait_pending": session.ui_portrait_pending(),
            "normal_view_frame_pending": session.normal_view_frame_pending(),
            "person_draw_preparation_pending": session.person_draw_preparation_pending(),
            "publication_pending": session.publication_pending(), "tool_selection_pending": session.tool_selection_pending(),
            "camera_command_pending": session.camera_command_pending(),
            "interface_update_pending": session.interface_update_pending(), "control_panel_pending": session.control_panel_pending(),
            "partial_simulation_global_17": session.simulation.vm.world.globals.get(&17),
            "awaiting_ui_effect_threads": session.simulation.vm.threads.iter()
                .filter(|(_, thread)| matches!(thread.status, sims_vm::Status::AwaitingUiEffect(_)))
                .map(|(id, _)| *id).collect::<Vec<_>>()
        });
    }
    observation
}

fn retain_owner_capture(
    observation: &mut Value,
    failures: &mut Vec<Value>,
    name: &str,
    capture: Result<Value>,
) {
    match capture {
        Ok(value) => observation["owners"][name] = value,
        Err(error) => failures.push(failure(&format!("viewer_owner_capture.{name}"), &error)),
    }
}

fn capture_viewer_owners(viewer: &Viewer, observation: &mut Value, failures: &mut Vec<Value>) {
    let dialogs = viewer.session.as_ref().map_or(Ok(Value::Null), |session| {
        serde_json::to_value(&session.simulation.vm.world.dialogs)
    });
    capture_viewer_owners_with_dialogs(viewer, observation, failures, dialogs);
}

fn capture_viewer_owners_with_dialogs(
    viewer: &Viewer,
    observation: &mut Value,
    failures: &mut Vec<Value>,
    dialogs: serde_json::Result<Value>,
) {
    let Some(session) = &viewer.session else {
        return;
    };
    let presentation = &session.host.presentation;
    for (name, capture) in [
        ("dialogs", dialogs),
        ("clock", serde_json::to_value(&presentation.clock)),
        (
            "lot_clock",
            serde_json::to_value(
                session
                    .simulation
                    .household
                    .as_ref()
                    .map(|household| &household.clock),
            ),
        ),
        (
            "lot_clock_events",
            serde_json::to_value(
                session
                    .simulation
                    .household
                    .as_ref()
                    .map(|household| &household.clock_events),
            ),
        ),
        (
            "scheduler",
            serde_json::to_value(&session.simulation.native_scheduler),
        ),
        ("transport", serde_json::to_value(&presentation.transport)),
        (
            "selection",
            serde_json::to_value(&session.simulation.native_selection),
        ),
        (
            "house_notification",
            serde_json::to_value(&presentation.notifications),
        ),
        (
            "object_budget",
            serde_json::to_value(&presentation.object_budget),
        ),
        (
            "publication",
            serde_json::to_value(&presentation.publication),
        ),
        (
            "purchase_cursor",
            serde_json::to_value(&presentation.purchase_cursor),
        ),
        ("move_tool", serde_json::to_value(&presentation.move_tool)),
        (
            "move_button",
            serde_json::to_value(&presentation.move_button),
        ),
        (
            "tool_selection",
            serde_json::to_value(&presentation.tool_selection),
        ),
        (
            "object_pickup",
            serde_json::to_value(&session.simulation.object_pickup),
        ),
        (
            "object_placement",
            serde_json::to_value(&session.simulation.object_placement),
        ),
        (
            "cursor_factory",
            serde_json::to_value(&session.simulation.cursor_factory),
        ),
        (
            "object_input",
            serde_json::to_value(&presentation.object_input),
        ),
        (
            "ground_input",
            serde_json::to_value(&presentation.ground_input),
        ),
        ("live_menu", serde_json::to_value(&presentation.live_menu)),
        ("ui_wait", serde_json::to_value(&presentation.ui_wait)),
        (
            "control_panel",
            serde_json::to_value(&presentation.control_panel),
        ),
        (
            "interface_update",
            serde_json::to_value(&presentation.interface_update),
        ),
    ] {
        retain_owner_capture(observation, failures, name, capture.map_err(Into::into));
    }
}

fn first_tick_boundary(
    initial_tick: u64,
    observation: &Value,
    play_activated: bool,
    frames: u32,
) -> Option<&'static str> {
    if !observation["error"].is_null() {
        return Some("viewer_error");
    }
    if !observation["menu_error"].is_null() {
        return Some("viewer_menu_error");
    }
    let Some(tick) = observation["tick"].as_u64() else {
        return Some("viewer_session_absent");
    };
    if tick < initial_tick {
        return Some("vm_tick_regressed");
    }
    if tick > initial_tick {
        return Some(if !play_activated || frames == 0 {
            "tick_advanced_before_player_frame"
        } else if observation["session"]["native_lot_present"] != true
            || observation["session"]["native_lot_routing"] != "Strict"
        {
            "tick_advanced_without_strict_native_lot"
        } else {
            "first_native_tick"
        });
    }
    if !observation["flags"]["modal"].is_null() {
        return Some("modal_requires_player_input");
    }
    if observation["flags"]["running_menu"] == true {
        return Some("menu_requires_player_input");
    }
    if play_activated && observation["playing"] == false {
        return Some("playing_stopped_before_first_tick");
    }
    if frames >= FIRST_TICK_MAX_FRAMES {
        return Some(
            if observation["flags"]["house_notification_pending"] == true {
                "house_notification_pending_at_frame_limit"
            } else if observation["flags"]["player_input_pending"] == true {
                "player_input_pending_at_frame_limit"
            } else if observation["flags"]["selection_pending"] == true {
                "selection_pending_at_frame_limit"
            } else if observation["flags"]["clock_pending"] == true {
                "clock_pending_at_frame_limit"
            } else if observation["presentation_pending"] == true {
                "presentation_pending_at_frame_limit"
            } else {
                "no_first_tick_at_frame_limit"
            },
        );
    }
    None
}

fn drive_first_tick(
    args: &Args,
    ready: Ready,
    observation: &mut Value,
    capture_failures: &mut Vec<Value>,
) -> Result<&'static str> {
    drive_tick_probe(args, ready, observation, capture_failures, false)
}

fn drive_tick_probe(
    args: &Args,
    ready: Ready,
    observation: &mut Value,
    capture_failures: &mut Vec<Value>,
    tutorial_response: bool,
) -> Result<&'static str> {
    observation["active_boundary"] = json!("session_inspection");
    let initial_tick = {
        let guard = ready
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("Ready session mutex poisoned"))?;
        let session = guard
            .as_ref()
            .context("Ready returned without a fresh session")?;
        observation["ready_session"] = session_metadata(session);
        session.simulation.vm.tick
    };
    observation["initial_tick"] = json!(initial_tick);
    inspect_ready(&ready)?;
    let (mut viewer, loader_task) = Viewer::new(args.clone());
    drop(loader_task);
    let epoch = Instant::now();
    observation["active_boundary"] = json!("viewer_install");
    let outcome = (|| -> Result<&'static str> {
        viewer.install(Arc::new(ready))?;
        observation["after_install"] = viewer_observation(&viewer, epoch);
        ensure!(
            !viewer.playing,
            "ordinary paused launcher installed a playing Viewer"
        );
        let mut frames = 0;
        let mut play_requested = false;
        let mut play_activated = false;
        let mut response = tutorial_entry::OneResponse::default();
        loop {
            let state = viewer_observation(&viewer, epoch);
            let boundary = if tutorial_response {
                tutorial_entry::boundary(initial_tick, &state, play_activated, frames, &response)
            } else {
                first_tick_boundary(initial_tick, &state, play_activated, frames)
            };
            if let Some(boundary) = boundary {
                if tutorial_response && boundary == "modal_requires_player_input" {
                    if let Some(blocker) = tutorial_entry::respond_once(
                        &mut viewer,
                        epoch,
                        observation,
                        capture_failures,
                        &mut response,
                    ) {
                        return Ok(blocker);
                    }
                    continue;
                }
                if tutorial_response
                    && boundary == "playing_stopped_before_first_tick"
                    && response.acknowledged()
                    && frames < FIRST_TICK_MAX_FRAMES
                {
                    if !viewer.presentation_pending() {
                        if let Some(blocker) = tutorial_entry::resume_play(
                            &mut viewer,
                            epoch,
                            observation,
                            &mut response,
                        ) {
                            return Ok(blocker);
                        }
                        continue;
                    }
                } else {
                    return Ok(boundary);
                }
            }
            if !play_requested && !viewer.presentation_pending() {
                observation["active_boundary"] = json!("viewer_toggle");
                let _ = viewer.update(Message::Toggle);
                play_requested = true;
                play_activated = viewer.playing;
                observation["play_requested"] = json!(true);
                observation["play_activated"] = json!(play_activated);
                observation["after_play"] = viewer_observation(&viewer, epoch);
                if !play_activated && viewer.error.is_none() && viewer.menu_error.is_none() {
                    return Ok("play_input_not_activated");
                }
                continue;
            }
            observation["active_boundary"] = json!("viewer_frame");
            let millis = frame_input_millis(frames).context("bounded frame input exhausted")?;
            let tick_before = viewer
                .session
                .as_ref()
                .map(|session| session.simulation.vm.tick);
            let _ = viewer.update(Message::Frame(epoch + Duration::from_millis(millis)));
            frames += 1;
            observation["frames"] = json!(frames);
            observation["last_frame_input_ms"] = json!(millis);
            observation["frame_observations"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "frame": frames, "input_ms": millis, "tick_before": tick_before,
                    "after": viewer_observation(&viewer, epoch)
                }));
        }
    })();
    observation["final"] = viewer_observation(&viewer, epoch);
    if tutorial_response {
        tutorial_entry::capture_owners(&viewer, &mut observation["final"], capture_failures);
    } else {
        capture_viewer_owners(&viewer, &mut observation["final"], capture_failures);
    }
    outcome
}

fn finish_first_tick_probe(
    output: &Path,
    request: &Value,
    args: &Args,
    loaded: Result<Ready>,
    timing: Result<Value>,
) -> Result<()> {
    let ready_returned_ok = loaded.is_ok();
    let mut observation = json!({
        "initial_tick": null, "frames": 0, "last_frame_input_ms": null,
        "play_requested": false, "play_activated": false, "frame_observations": [],
        "active_boundary": "ready_load_inner"
    });
    let mut capture_failures = Vec::new();
    let outcome = loaded
        .and_then(|ready| drive_first_tick(args, ready, &mut observation, &mut capture_failures));
    finish_first_tick_observation(
        output,
        request,
        ready_returned_ok,
        observation,
        outcome,
        capture_failures,
        timing,
    )
}

fn finish_first_tick_observation(
    output: &Path,
    request: &Value,
    ready_returned_ok: bool,
    observation: Value,
    outcome: Result<&str>,
    capture_failures: Vec<Value>,
    timing: Result<Value>,
) -> Result<()> {
    finish_tick_observation(
        output,
        request,
        ready_returned_ok,
        observation,
        outcome,
        capture_failures,
        timing,
        false,
    )
}

fn finish_tick_observation(
    output: &Path,
    request: &Value,
    ready_returned_ok: bool,
    observation: Value,
    outcome: Result<&str>,
    capture_failures: Vec<Value>,
    timing: Result<Value>,
    tutorial_response: bool,
) -> Result<()> {
    let mut failures = Vec::new();
    match &outcome {
        Err(error) => failures.push(failure(
            observation["active_boundary"].as_str().unwrap(),
            error,
        )),
        Ok(boundary) if *boundary != "first_native_tick" => {
            let state = &observation["final"];
            let message = match *boundary {
                "viewer_error" => state["error"].clone(),
                "viewer_menu_error" => state["menu_error"].clone(),
                _ => json!(format!(
                    "First-tick diagnostic stopped at {boundary}; retained owner state is authoritative"
                )),
            };
            failures.push(json!({"boundary": boundary, "message": message,
                "chain": null, "full_chain": null,
                "error_representation": "stored Viewer string or diagnostic boundary; no recoverable anyhow chain"}));
        }
        _ => {}
    }
    if tutorial_response
        && matches!(outcome, Ok("first_native_tick"))
        && (observation["response_submitted"] != true
            || observation["response_acknowledged"] != true)
    {
        failures.push(json!({"boundary": "opening_response_not_acknowledged",
            "message": "A native tick alone does not prove acknowledgment of the offered opening response",
            "chain": null, "full_chain": null}));
    }
    failures.extend(capture_failures);
    if let Err(error) = &timing {
        failures.push(failure("loader_stage_export", error));
    }
    let loader = timing.as_ref().ok().and_then(loader_record);
    for failure in &mut failures {
        failure["loader_phase"] = loader
            .and_then(|value| value.get("phases")?.as_array()?.last()?.get("phase_id"))
            .cloned()
            .unwrap_or(Value::Null);
        failure["handler_boundary"] = observation["active_boundary"].clone();
        failure["frame"] = observation["frames"].clone();
        failure["input_ms"] = observation["last_frame_input_ms"].clone();
    }
    let first_tick_success = matches!(outcome, Ok("first_native_tick"));
    let success = first_tick_success && failures.is_empty();
    let tick_before = observation["initial_tick"].as_u64();
    let tick_after = observation["final"]["tick"].as_u64();
    let mut receipt = json!({
        "schema": "sims-strict-first-tick-probe-v1", "private_local_diagnostic": true,
        "request": request, "result_complete": true, "success": success,
        "ready_returned_ok": ready_returned_ok, "first_tick_success": first_tick_success,
        "tick_before": tick_before, "tick_after": tick_after,
        "vm_tick_advanced": tick_before.zip(tick_after).map(|(before, after)| after > before),
        "boundary": outcome.as_ref().ok().copied().or_else(|| observation["active_boundary"].as_str()), "observation": observation,
        "loader": loader, "timing": timing.as_ref().ok(), "first_failure": failures.first(), "failures": failures,
        "native_qualification": false, "startup_qualified": false,
        "performance_qualified": false, "alpha_qualified": false,
        "gui_started": false, "presentation_observed": false, "settlement_observed": false,
        "scope": "actual Ready installed in production Viewer; Toggle and bounded simulated/headless Frame inputs only; stop at first handler boundary observing advancement, error or blocker; a handler can perform multiple native passes",
        "help": FIRST_TICK_HELP
    });
    if tutorial_response {
        tutorial_entry::tag_receipt(&mut receipt);
        receipt["response_submitted"] = receipt["observation"]["response_submitted"].clone();
        receipt["response_acknowledged"] = receipt["observation"]["response_acknowledged"].clone();
    }
    write_receipt(output, RECEIPT, &receipt).with_context(|| {
        format!(
            "private first-tick receipt publication failed; retained diagnostic: {}",
            receipt["first_failure"]
        )
    })?;
    ensure!(
        success,
        "Strict first-tick diagnostic did not complete successfully; private receipt saved at {}: {}",
        output.join(RECEIPT).display(),
        receipt["first_failure"]
    );
    Ok(())
}

fn finish_probe(
    output: &Path,
    request: &Value,
    ready_returned_ok: bool,
    observation: Result<Value>,
    timing: Result<Value>,
) -> Result<()> {
    let mut failures = Vec::new();
    if let Err(error) = &observation {
        failures.push(failure(
            if ready_returned_ok {
                "session_inspection"
            } else {
                "ready_load_inner"
            },
            error,
        ));
    }
    if let Err(error) = &timing {
        failures.push(failure("loader_stage_export", error));
    }
    let timing_value = timing.as_ref().ok();
    let loader = timing_value.and_then(loader_record);
    if let Some(first) = failures.first_mut() {
        first["loader_phase"] = loader
            .and_then(|value| value.get("phases")?.as_array()?.last()?.get("phase_id"))
            .cloned()
            .unwrap_or(Value::Null);
    }
    let receipt = json!({
        "schema": "sims-strict-startup-probe-v1", "private_local_diagnostic": true,
        "request": request, "result_complete": true,
        "success": observation.is_ok() && timing.is_ok(), "ready_returned_ok": ready_returned_ok,
        "session": observation.as_ref().ok(), "loader": loader, "timing": timing_value,
        "first_failure": failures.first(), "failures": failures,
        "native_qualification": false, "startup_qualified": false,
        "performance_qualified": false, "alpha_qualified": false,
        "gui_started": false, "presentation_observed": false, "settlement_observed": false,
        "scope": "Ready::load_inner only; recorder parent is structural, not application settlement",
        "help": HELP
    });
    write_receipt(output, RECEIPT, &receipt).with_context(|| {
        format!(
            "private receipt could not be published; diagnostic failure: {}",
            failures
                .first()
                .map_or_else(|| "none".into(), |error| error["full_chain"].to_string())
        )
    })?;
    observation.with_context(|| {
        format!(
            "Strict startup diagnostic failed; private receipt saved at {}",
            output.join(RECEIPT).display()
        )
    })?;
    timing.context("loader stage export failed; private failure receipt saved")?;
    Ok(())
}

#[test]
#[ignore = "Private installed inputs required; set SIMS_STRICT_STARTUP_ROOT and fresh SIMS_STRICT_STARTUP_OUTPUT under ignored cache; external whole-process timeout and child cleanup required"]
fn ordinary_strict_newbie_ready_load() -> Result<()> {
    ordinary_probe(false)
}

#[test]
#[ignore = "Private parent-pinned readonly inputs required; fresh SIMS_STRICT_STARTUP_OUTPUT; external whole-process timeout and child cleanup; simulated/headless Frame inputs, not native performance"]
fn ordinary_strict_newbie_first_tick() -> Result<()> {
    ordinary_probe(true)
}

fn ordinary_probe(first_tick: bool) -> Result<()> {
    ordinary_probe_with_response(first_tick, false)
}

fn ordinary_probe_with_response(first_tick: bool, tutorial_response: bool) -> Result<()> {
    let help = if tutorial_response {
        tutorial_entry::HELP
    } else if first_tick {
        FIRST_TICK_HELP
    } else {
        HELP
    };
    let explicit_path = |name| -> Result<PathBuf> {
        let value =
            std::env::var_os(name).with_context(|| format!("{name} is required. {help}"))?;
        ensure!(!value.is_empty(), "{name} must not be empty. {help}");
        Ok(value.into())
    };
    let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache");
    let (root, output) = prepare_output(
        &explicit_path("SIMS_STRICT_STARTUP_ROOT")?,
        &explicit_path("SIMS_STRICT_STARTUP_OUTPUT")?,
        &cache,
    )?;
    let arguments = launcher_arguments(&root, &output);
    let args = Args::try_parse_from(&arguments)?;
    validate_arguments(&args, &root, &output)?;
    let arguments: Vec<_> = arguments
        .iter()
        .map(|argument| {
            argument
                .to_str()
                .context("argument identity requires UTF-8")
        })
        .collect::<Result<_>>()?;
    let mut request = json!({"arguments": arguments, "root": root, "output": output,
        "source": NEWBIE_SOURCE, "save": args.save, "paused": args.paused,
        "routing": "Strict", "buy_preview": args.buy_preview,
        "neighborhood_policy": args.neighborhood_policy, "options_overridden_by_probe": false,
        "bounds": "external whole-process timeout required; production loader/service budgets unchanged"});
    if first_tick {
        request["frame_driver"] = json!({
            "kind": "simulated/headless monotonic Instant offsets, not native-window wall time",
            "max_frames": FIRST_TICK_MAX_FRAMES, "first_input_ms": 0,
            "frame_interval_ms": FIRST_TICK_FRAME_MILLIS,
            "last_permitted_input_ms": frame_input_millis(FIRST_TICK_MAX_FRAMES - 1),
            "epoch_initialization": "Viewer::install leaves last/clock_origin unset; ordinary Frame handler establishes both without writing ticks or native clock fields",
            "play_input": "one ordinary Message::Toggle after paused presentation work returns; never answer or clear a modal/menu",
            "loader_task": "Viewer::new task dropped unpolled; install the actual single Ready result",
            "input_integrity_owner": "parent external readonly pin and source/build/input identities; probe does not provide native provenance",
            "publication": "no Save or Continue messages; receipts only"
        });
    }
    if tutorial_response {
        tutorial_entry::tag_request(&mut request);
    }
    let mut started = json!({
        "schema": if first_tick { "sims-strict-first-tick-probe-v1" } else { "sims-strict-startup-probe-v1" }, "request": request,
        "result_complete": false, "success": null, "private_local_diagnostic": true,
        "native_qualification": false, "startup_qualified": false,
        "performance_qualified": false, "alpha_qualified": false, "help": help
    });
    if tutorial_response {
        tutorial_entry::tag_receipt(&mut started);
    }
    write_receipt(&output, "started.private.json", &started)?;
    let recorder =
        player_timing::Recorder::enabled(true, 4).context("loader recorder unavailable")?;
    let parent = recorder.start(
        None,
        player_timing::Scope::ConstructionToSettled,
        player_timing::Operation::FreshLoad,
        player_timing::Role::PlayerPath,
        None,
        true,
    );
    let loaded = recorder
        .loader_probe(parent)
        .measure_staged(|stages| Ready::load_inner(&args, Some(stages)));
    let ready_returned_ok = loaded.is_ok();
    recorder.finish(parent, player_timing::Outcome::Censored, None, None);
    if tutorial_response {
        return tutorial_entry::finish_probe(
            &output,
            &request,
            &args,
            loaded,
            recorder.try_final_artifact(false, true),
        );
    }
    if first_tick {
        return finish_first_tick_probe(
            &output,
            &request,
            &args,
            loaded,
            recorder.try_final_artifact(false, true),
        );
    }
    let observation = loaded.and_then(|ready| inspect_ready(&ready));
    finish_probe(
        &output,
        &request,
        ready_returned_ok,
        observation,
        recorder.try_final_artifact(false, true),
    )
}

struct Scratch {
    base: PathBuf,
    root: PathBuf,
    cache: PathBuf,
}

impl Scratch {
    fn new() -> Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let base = std::env::temp_dir().canonicalize()?.join(format!(
            "sims-strict-startup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&base)?;
        let root = base.join("installation");
        let cache = base.join("cache");
        fs::create_dir(&root)?;
        fs::create_dir(&cache)?;
        Ok(Self { base, root, cache })
    }

    fn output(&self) -> Result<PathBuf> {
        Ok(prepare_output(&self.root, &self.cache.join("probe"), &self.cache)?.1)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.base) {
            eprintln!("synthetic probe fixture cleanup failed: {error}");
        }
    }
}

#[test]
fn launcher_policy_matches_ordinary_newbie_mapping() -> Result<()> {
    let root = Path::new("/synthetic installation");
    let output = Path::new("/synthetic cache/probe");
    let arguments = launcher_arguments(root, output);
    let mut args = Args::try_parse_from(&arguments)?;
    validate_arguments(&args, root, output)?;
    ensure!(
        include_str!("../../../../scripts/play_alpha.py")
            .contains(&format!("'newbie': '{NEWBIE_SOURCE}'")),
        "ordinary launcher Newbie mapping changed"
    );
    args.inferred_routing = true;
    assert!(validate_arguments(&args, root, output).is_err());
    args.inferred_routing = false;
    args.buy_preview = true;
    assert!(validate_arguments(&args, root, output).is_err());
    args.buy_preview = false;
    args.neighborhood_policy = Some("installed-houses-ascending-v1".into());
    assert!(validate_arguments(&args, root, output).is_err());
    Ok(())
}

#[test]
fn output_is_exclusive_private_and_separate_from_input() -> Result<()> {
    let scratch = Scratch::new()?;
    let output = scratch.output()?;
    assert!(prepare_output(&scratch.root, &output, &scratch.cache).is_err());
    assert!(prepare_output(&scratch.root, &scratch.root, &scratch.cache).is_err());
    assert!(prepare_output(&scratch.root, &scratch.root.join("probe"), &scratch.cache).is_err());
    assert!(
        prepare_output(
            &scratch.cache,
            &scratch.cache.join("inside"),
            &scratch.cache
        )
        .is_err()
    );
    assert!(prepare_output(&scratch.root, &scratch.base.join("outside"), &scratch.cache).is_err());
    assert!(prepare_output(&scratch.root, Path::new("relative-output"), &scratch.cache).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlink_outputs_and_receipt_collisions_do_not_touch_targets() -> Result<()> {
    use std::os::unix::fs::symlink;
    let scratch = Scratch::new()?;
    let alias = scratch.cache.join("alias");
    symlink(&scratch.root, &alias)?;
    assert!(prepare_output(&scratch.root, &alias, &scratch.cache).is_err());
    assert!(prepare_output(&scratch.root, &alias.join("child"), &scratch.cache).is_err());
    let output = scratch.output()?;
    let protected = scratch.root.join("synthetic-sentinel");
    fs::write(&protected, b"synthetic unchanged")?;
    symlink(&protected, output.join(RECEIPT))?;
    assert!(write_receipt(&output, RECEIPT, &json!({"success": true})).is_err());
    assert_eq!(fs::read(&protected)?, b"synthetic unchanged");
    Ok(())
}

#[test]
fn failed_publication_never_leaves_a_final_success_receipt() -> Result<()> {
    let scratch = Scratch::new()?;
    let output = scratch.output()?;
    fs::create_dir(output.join(format!(".{RECEIPT}.pending")))?;
    assert!(write_receipt(&output, RECEIPT, &json!({"success": true})).is_err());
    assert!(!output.join(RECEIPT).exists());
    Ok(())
}

#[test]
fn loader_failure_is_saved_with_full_chain_and_returned_as_error() -> Result<()> {
    let scratch = Scratch::new()?;
    let output = scratch.output()?;
    let failure = Err(anyhow::anyhow!("synthetic root cause")).context("synthetic loader boundary");
    assert!(finish_probe(&output, &json!({}), false, failure, Ok(json!({"rows": []}))).is_err());
    let receipt: Value = serde_json::from_slice(&fs::read(output.join(RECEIPT))?)?;
    assert_eq!(receipt["success"], false);
    assert_eq!(receipt["first_failure"]["boundary"], "ready_load_inner");
    assert_eq!(
        receipt["first_failure"]["chain"],
        json!(["synthetic loader boundary", "synthetic root cause"])
    );
    assert_eq!(receipt["result_complete"], true);
    assert!(write_receipt(&output, RECEIPT, &json!({"success": true})).is_err());
    let unchanged: Value = serde_json::from_slice(&fs::read(output.join(RECEIPT))?)?;
    assert_eq!(unchanged, receipt);
    Ok(())
}

#[test]
fn success_and_timing_failure_never_promote_qualification() -> Result<()> {
    for timing_failed in [false, true] {
        let scratch = Scratch::new()?;
        let output = scratch.output()?;
        let timing = if timing_failed {
            Err(anyhow::anyhow!("synthetic recorder failure"))
        } else {
            Ok(json!({"rows": []}))
        };
        let result = finish_probe(
            &output,
            &json!({}),
            true,
            Ok(json!({"synthetic": true})),
            timing,
        );
        assert_eq!(result.is_err(), timing_failed);
        let receipt: Value = serde_json::from_slice(&fs::read(output.join(RECEIPT))?)?;
        assert_eq!(receipt["success"], !timing_failed);
        for field in [
            "native_qualification",
            "startup_qualified",
            "performance_qualified",
            "alpha_qualified",
        ] {
            assert_eq!(receipt[field], false);
        }
    }
    Ok(())
}

#[test]
fn authored_frame_inputs_are_bounded_and_monotonic() {
    assert_eq!(frame_input_millis(0), Some(0));
    assert_eq!(frame_input_millis(1), Some(16));
    assert_eq!(frame_input_millis(FIRST_TICK_MAX_FRAMES - 1), Some(9584));
    assert_eq!(frame_input_millis(FIRST_TICK_MAX_FRAMES), None);
    assert_eq!(frame_input_millis(u32::MAX), None);
    for frame in 1..FIRST_TICK_MAX_FRAMES {
        assert_eq!(
            frame_input_millis(frame).unwrap() - frame_input_millis(frame - 1).unwrap(),
            FIRST_TICK_FRAME_MILLIS
        );
    }
}

#[test]
fn authored_boundaries_require_advancement_after_play_and_preserve_blockers() {
    let baseline = json!({
        "tick": 41, "error": null, "menu_error": null, "playing": true,
        "presentation_pending": false, "flags": {"modal": null, "running_menu": false},
        "session": {"native_lot_present": true, "native_lot_routing": "Strict"}
    });
    assert_eq!(first_tick_boundary(41, &baseline, true, 1), None);
    assert_eq!(
        first_tick_boundary(41, &baseline, true, FIRST_TICK_MAX_FRAMES),
        Some("no_first_tick_at_frame_limit")
    );
    for (field, expected) in [
        (
            "house_notification_pending",
            "house_notification_pending_at_frame_limit",
        ),
        (
            "player_input_pending",
            "player_input_pending_at_frame_limit",
        ),
        ("selection_pending", "selection_pending_at_frame_limit"),
        ("clock_pending", "clock_pending_at_frame_limit"),
    ] {
        let mut state = baseline.clone();
        state["flags"][field] = json!(true);
        assert_eq!(first_tick_boundary(41, &state, true, 1), None);
        assert_eq!(
            first_tick_boundary(41, &state, true, FIRST_TICK_MAX_FRAMES),
            Some(expected)
        );
    }
    let mut state = baseline.clone();
    state["flags"]["modal"] = json!(7);
    assert_eq!(
        first_tick_boundary(41, &state, true, 1),
        Some("modal_requires_player_input")
    );
    state = baseline.clone();
    state["flags"]["running_menu"] = json!(true);
    assert_eq!(
        first_tick_boundary(41, &state, true, 1),
        Some("menu_requires_player_input")
    );
    state = baseline.clone();
    state["tick"] = json!(40);
    assert_eq!(
        first_tick_boundary(41, &state, true, 1),
        Some("vm_tick_regressed")
    );
    state["tick"] = json!(42);
    assert_eq!(
        first_tick_boundary(41, &state, false, 1),
        Some("tick_advanced_before_player_frame")
    );
    assert_eq!(
        first_tick_boundary(41, &state, true, 0),
        Some("tick_advanced_before_player_frame")
    );
    assert_eq!(
        first_tick_boundary(41, &state, true, 1),
        Some("first_native_tick")
    );
    state["session"]["native_lot_routing"] = json!("InferredRectangles");
    assert_eq!(
        first_tick_boundary(41, &state, true, 1),
        Some("tick_advanced_without_strict_native_lot")
    );
    state["error"] = json!("authored Viewer failure: original rendered detail");
    assert_eq!(
        first_tick_boundary(41, &state, true, 1),
        Some("viewer_error")
    );
}

#[test]
fn authored_first_tick_receipts_retain_partial_state_and_never_qualify() -> Result<()> {
    for mode in [
        "first_native_tick",
        "viewer_error",
        "owner_failure",
        "timing_failure",
        "install_failure",
        "no_first_tick_at_frame_limit",
    ] {
        let scratch = Scratch::new()?;
        let output = scratch.output()?;
        let mut observation = json!({
            "active_boundary": "viewer_install", "initial_tick": 41, "frames": 1,
            "play_requested": true, "play_activated": true,
            "final": {"tick": 42, "error": null, "menu_error": null, "playing": true}
        });
        if mode == "viewer_error" {
            observation["final"]["error"] = json!("authored rendered error: exact inner string");
        }
        if mode == "no_first_tick_at_frame_limit" {
            observation["frames"] = json!(FIRST_TICK_MAX_FRAMES);
            observation["final"]["tick"] = json!(41);
        }
        let mut capture_failures = Vec::new();
        retain_owner_capture(
            &mut observation["final"],
            &mut capture_failures,
            "clock",
            Ok(json!({"now": 16})),
        );
        if mode == "owner_failure" {
            retain_owner_capture(
                &mut observation["final"],
                &mut capture_failures,
                "selection",
                Err(anyhow::anyhow!("authored owner export failure")),
            );
        }
        let outcome = match mode {
            "viewer_error" => Ok("viewer_error"),
            "no_first_tick_at_frame_limit" => Ok("no_first_tick_at_frame_limit"),
            "install_failure" => Err(anyhow::anyhow!("authored inner install failure"))
                .context("authored install boundary"),
            _ => Ok("first_native_tick"),
        };
        let timing = if mode == "timing_failure" {
            Err(anyhow::anyhow!("authored recorder failure"))
        } else {
            Ok(json!({"rows": []}))
        };
        let result = finish_first_tick_observation(
            &output,
            &json!({"authored_helper_data_only_not_runtime_proof": true}),
            true,
            observation,
            outcome,
            capture_failures,
            timing,
        );
        assert_eq!(result.is_ok(), mode == "first_native_tick");
        let receipt: Value = serde_json::from_slice(&fs::read(output.join(RECEIPT))?)?;
        assert_eq!(receipt["success"], mode == "first_native_tick");
        assert_eq!(
            receipt["first_tick_success"],
            !matches!(
                mode,
                "viewer_error" | "install_failure" | "no_first_tick_at_frame_limit"
            )
        );
        assert_eq!(receipt["result_complete"], true);
        assert_eq!(receipt["tick_before"], 41);
        assert_eq!(
            receipt["tick_after"],
            if mode == "no_first_tick_at_frame_limit" {
                41
            } else {
                42
            }
        );
        assert_eq!(
            receipt["observation"]["final"]["owners"]["clock"]["now"],
            16
        );
        if mode == "viewer_error" {
            assert_eq!(
                receipt["first_failure"]["message"],
                "authored rendered error: exact inner string"
            );
            assert_eq!(receipt["first_failure"]["chain"], Value::Null);
            assert_eq!(receipt["first_failure"]["full_chain"], Value::Null);
        }
        if mode == "install_failure" {
            assert_eq!(receipt["first_failure"]["boundary"], "viewer_install");
            assert_eq!(
                receipt["first_failure"]["chain"],
                json!([
                    "authored install boundary",
                    "authored inner install failure"
                ])
            );
        }
        if mode == "owner_failure" {
            assert_eq!(
                receipt["first_failure"]["boundary"],
                "viewer_owner_capture.selection"
            );
        }
        if mode == "timing_failure" {
            assert_eq!(receipt["first_failure"]["boundary"], "loader_stage_export");
        }
        for field in [
            "native_qualification",
            "startup_qualified",
            "performance_qualified",
            "alpha_qualified",
        ] {
            assert_eq!(receipt[field], false);
        }
        assert!(write_receipt(&output, RECEIPT, &json!({"success": true})).is_err());
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(output.join(RECEIPT))?)?,
            receipt
        );
    }
    Ok(())
}
