//! Fresh original-house import followed by production player input handlers.
use super::*;
use clap::Parser;

#[path = "startup_play/host_journey.rs"]
mod host_journey;

#[path = "startup_play/alpha_discovery.rs"]
mod alpha_discovery;

#[path = "startup_play/alpha_activities.rs"]
mod alpha_activities;

#[path = "startup_play/alpha_tutorial.rs"]
mod alpha_tutorial;

#[derive(Debug, PartialEq, Eq)]
enum NextInput {
    Dialog(u64),
    Poll,
    Play,
    Frame,
}

fn startup_response(record: &sims_vm::dialog::Record) -> Result<u32> {
    if record.source == "GameData_ALL_Recursive/Objects/Objects.far::Tutorial.iff"
        && record.source == record.origin.source
        && record.origin.bhav == 4097
        && record.origin.instruction == 2
        && record.operands == [0, 18, 3, 0, 0, 4, 0, 6]
        && record.title == "Tutorial"
        && engine::digest(record.body.as_bytes())
            == "b3a70a44690fe52bc72ac4fdb009329186cbc3b52cf5543652a3053e5e0cef81"
        && record.buttons.len() == 1
        && record.buttons[0].label == "OK"
        && record.buttons[0].result == 2
        && record.status == 1
        && record.modal()
    {
        return Ok(2);
    }
    let result = super::super::clock_tests::startup_dialog_response(record)?;
    if result == 3 {
        return Ok(result);
    }
    let expected = [
        (
            "::HelpSystemMagic.iff",
            3,
            "Magic Town",
            "8adc30cd1de26fbbbf34b0e9a2c619161c31903c1c2e2fe5c5b4bf750b798588",
        ),
        (
            "::HDHelpSystem.iff",
            34,
            " ",
            "74458af6ac718e72b3a2ab6598af7e3bea98e2d04da12a23a4c8794850231d2f",
        ),
        (
            "::NeighborhoodHelpSystem.iff",
            3,
            "Old Town Shuttle",
            "018722c9a7b117b343d7815d4ce26980a01f01a1f544a691e0b1da621c0365cb",
        ),
        (
            "::HelpSystemStudioTown.iff",
            3,
            "Studio Town",
            "2fbd7eaa7fe4848c76c02c21954e6f8c8df1f623401370a70d3daa0f2f1ce63a",
        ),
        (
            "::VacationHelpSystem.iff",
            3,
            "Feeling Down?",
            "5ef4d7566d4762c9e0f57717f8a20f650cdbbced96b82ebcf93899c863b62d4d",
        ),
    ];
    ensure!(
        record.buttons[0].label == "OK"
            && expected
                .iter()
                .any(
                    |(source, instruction, title, hash)| record.source.ends_with(source)
                        && record.origin.instruction == *instruction
                        && record.title == *title
                        && engine::digest(record.body.as_bytes()) == *hash
                ),
        "unplanned original startup acknowledgement: {record:?}"
    );
    Ok(result)
}

fn next_input(
    error: Option<&str>,
    modal: Option<u64>,
    pending: bool,
    playing: bool,
    play_requested: bool,
    answered_dialog: bool,
) -> Result<NextInput> {
    ensure!(error.is_none(), "{}", error.unwrap_or_default());
    if let Some(id) = modal {
        return Ok(NextInput::Dialog(id));
    }
    if pending {
        return Ok(NextInput::Poll);
    }
    if playing {
        return Ok(NextInput::Frame);
    }
    if !play_requested || answered_dialog {
        return Ok(NextInput::Play);
    }
    anyhow::bail!("uncommanded pause without a modal; not a VM-fault diagnosis")
}

#[test]
#[ignore = "private original assets; fresh SIMS_FRESH_PLAY_OUTPUT, bounded production player path"]
fn fresh_original_household_load_and_play() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("fresh household worker panicked")
}

fn calendar(app: &Application) -> Result<i64> {
    let clock = &app
        .session()?
        .simulation
        .household
        .as_ref()
        .context("fresh household calendar absent")?
        .clock;
    Ok(i64::from(clock.elapsed_days) * 86400
        + i64::from(clock.hour) * 3600
        + i64::from(clock.minute) * 60
        + i64::from(clock.second))
}

fn bounded(app: &Application, wall: Instant, limits: &Limits, frames: u32) -> Result<()> {
    app.check_errors()?;
    ensure!(
        wall.elapsed().as_secs_f64() < limits.wall_seconds,
        "fresh-play wall limit"
    );
    ensure!(frames < 100_000, "fresh-play frame limit");
    Ok(())
}

fn execute(
    app: &mut Application,
    command: Command,
    limits: &Limits,
    commands: &mut Vec<Command>,
) -> Result<()> {
    ensure!(commands.len() < limits.commands, "fresh-play command limit");
    commands.push(command.clone());
    app.execute(&command, limits)
}

struct Evidence {
    commands: Vec<Command>,
    events: Vec<Value>,
    samples: Vec<Value>,
    frames: u32,
    dialogs: usize,
}

fn play_for(
    app: &mut Application,
    seconds: i64,
    phase: &str,
    wall: Instant,
    limits: &Limits,
    evidence: &mut Evidence,
) -> Result<()> {
    let start_tick = app.session()?.simulation.vm.tick;
    let start_calendar = calendar(app)?;
    let mut play_requested = false;
    let mut answered_dialog = false;
    loop {
        bounded(app, wall, limits, evidence.frames)?;
        let tick = app.session()?.simulation.vm.tick;
        ensure!(tick >= start_tick, "fresh-play tick moved backwards");
        ensure!(
            tick - start_tick < limits.completed_ticks,
            "fresh-play tick limit"
        );
        let modal = app.session()?.simulation.vm.world.dialogs.modal;
        let input = next_input(
            app.viewer
                .error
                .as_deref()
                .or(app.viewer.menu_error.as_deref()),
            modal,
            app.viewer.presentation_pending(),
            app.viewer.playing,
            play_requested,
            answered_dialog,
        )?;
        if modal.is_none() && calendar(app)? - start_calendar >= seconds {
            ensure!(
                tick > start_tick,
                "calendar target reached without completed ticks"
            );
            evidence
                .events
                .push(json!({"kind":"progress_target","phase":phase,
                "start_tick":start_tick,"tick":tick,"game_seconds":calendar(app)?-start_calendar}));
            return Ok(());
        }
        match input {
            NextInput::Poll => {
                execute(app, Command::Poll, limits, &mut evidence.commands)?;
            }
            NextInput::Dialog(id) => {
                ensure!(evidence.dialogs < 16, "fresh-play dialog input limit");
                let record = app
                    .session()?
                    .simulation
                    .vm
                    .world
                    .dialogs
                    .records
                    .iter()
                    .find(|record| record.id == id)
                    .context("modal record absent")?
                    .clone();
                evidence
                    .events
                    .push(json!({"kind":"modal","phase":phase,"tick":tick,
                    "playing":app.viewer.playing,"record":record,"response_status":"not_sent"}));
                let result = startup_response(&record)?;
                ensure!(
                    record.status == 1 && record.modal(),
                    "startup modal is not answerable"
                );
                let command = Command::Dialog {
                    id,
                    result,
                    body_sha256: engine::digest(record.body.as_bytes()),
                };
                let event = evidence.events.last_mut().unwrap();
                event["response"] = json!(result);
                event["choice"] = json!(
                    record
                        .buttons
                        .iter()
                        .find(|button| button.result == result)
                        .context("startup response button absent")?
                        .label
                );
                event["response_status"] = json!("attempted");
                execute(app, command, limits, &mut evidence.commands)?;
                evidence.events.last_mut().unwrap()["response_status"] = json!("handler_returned");
                evidence.dialogs += 1;
                answered_dialog = true;
            }
            NextInput::Play => {
                evidence.events.push(json!({"kind":"play","phase":phase,"tick":tick,
                    "reason":if answered_dialog {"resume_after_recorded_dialog"} else {"explicit_player_play"}}));
                execute(
                    app,
                    Command::Playing { value: true },
                    limits,
                    &mut evidence.commands,
                )?;
                play_requested = true;
                answered_dialog = false;
            }
            NextInput::Frame => {
                evidence.frames += 1;
                execute(
                    app,
                    Command::Advance { millis: 40 },
                    limits,
                    &mut evidence.commands,
                )?;
                if evidence.frames == 1 || evidence.frames % 256 == 0 {
                    let row = json!({"phase":phase,"frames":evidence.frames,
                        "tick":app.session()?.simulation.vm.tick,"calendar":calendar(app)?,
                        "playing":app.viewer.playing,"modal":app.session()?.simulation.vm.world.dialogs.modal,
                        "wall_seconds":wall.elapsed().as_secs_f64()});
                    println!("{row}");
                    evidence.samples.push(row);
                }
            }
        }
    }
}

fn continuation(app: &Application) -> Result<String> {
    let session = app.session()?;
    engine::digest_serialized(&json!({"session":session.save(),
        "client":app.viewer.client.as_ref().context("dialog client absent")?.save(session)?,
        "device":app.viewer.device.as_ref().map(|device| device.save(session)).transpose()?}))
}

fn action_ids(app: &Application, actor: i16) -> Result<Vec<u32>> {
    let actions = app
        .session()?
        .simulation
        .vm
        .world
        .actions
        .as_ref()
        .and_then(|actions| actions.actors.get(&actor))
        .context("resident action queue absent")?;
    Ok(actions
        .active
        .iter()
        .chain(actions.queue.pending.iter())
        .map(|action| action.id)
        .collect())
}

fn host_startup(
    app: &mut Application,
    wall: Instant,
    limits: &Limits,
    evidence: &mut Evidence,
    operations: &mut Value,
) -> Result<()> {
    use super::super::worker_host::{Event, Host};
    let start_tick = app.session()?.simulation.vm.tick;
    let placeholder = Viewer::new(app.viewer.args.clone()).0;
    let mut host = Host::from_viewer_for_test(std::mem::replace(&mut app.viewer, placeholder));
    let mut frames = 0;
    let mut play_requested = false;
    let mut answered_dialog = false;
    operations["host_startup"] = json!({"status":"attempted","start_tick":start_tick,
        "load_boundary":"fresh Ready::load/Viewer::install before Host owner construction",
        "per_frame_audit_snapshots":false});
    let result = (|| -> Result<()> {
        for _ in 0..512 {
            ensure!(
                wall.elapsed().as_secs_f64() < limits.wall_seconds,
                "fresh Host wall limit"
            );
            let publication = host.publication_for_test().clone();
            ensure!(
                !publication.modal || publication.dialog.is_some(),
                "Host modal record absent"
            );
            let input = next_input(
                publication.error.as_deref(),
                publication
                    .modal
                    .then(|| publication.dialog.as_ref().map(|record| record.id))
                    .flatten(),
                publication.pending,
                publication.playing,
                play_requested,
                answered_dialog,
            )?;
            if frames >= 32 && !publication.pending && !publication.modal && publication.playing {
                return Ok(());
            }
            let event = match input {
                NextInput::Poll => Event::Frame(app.epoch + Duration::from_millis(app.millis)),
                NextInput::Dialog(id) => {
                    ensure!(evidence.dialogs < 16, "fresh Host dialog input limit");
                    let record = publication
                        .dialog
                        .as_ref()
                        .context("Host modal record absent")?;
                    ensure!(
                        record.status == 1 && record.modal(),
                        "Host modal is not answerable"
                    );
                    evidence.events.push(
                        json!({"kind":"host_modal","record":record,"response_status":"not_sent"}),
                    );
                    let result = startup_response(record)?;
                    let event = evidence.events.last_mut().unwrap();
                    event["response"] = json!(result);
                    event["choice"] = json!(
                        record
                            .buttons
                            .iter()
                            .find(|button| button.result == result)
                            .context("Host startup response button absent")?
                            .label
                    );
                    event["response_status"] = json!("attempted");
                    answered_dialog = true;
                    evidence.dialogs += 1;
                    host.ui_event_for_test(&publication, Message::Dialog(id, result))
                }
                NextInput::Play => {
                    evidence
                        .events
                        .push(json!({"kind":"host_play","after_recorded_dialog":answered_dialog}));
                    answered_dialog = false;
                    play_requested = true;
                    host.ui_event_for_test(&publication, Message::Toggle)
                }
                NextInput::Frame => {
                    frames += 1;
                    evidence.frames += 1;
                    app.millis += 40;
                    Event::Frame(app.epoch + Duration::from_millis(app.millis))
                }
            };
            let _ = host.update(event);
            ensure!(
                host.settle_for_test()?.is_empty(),
                "fresh Host produced unhandled native task effects"
            );
            if evidence
                .events
                .last()
                .is_some_and(|event| event["kind"] == "host_modal")
            {
                evidence.events.last_mut().unwrap()["response_status"] = json!("handler_returned");
            }
        }
        anyhow::bail!("fresh Host command limit")
    })();
    app.viewer = host.recover_for_test();
    app.sample_paths();
    let final_tick = app.session()?.simulation.vm.tick;
    operations["host_startup"]["frames"] = json!(frames);
    operations["host_startup"]["final_tick"] = json!(final_tick);
    result?;
    app.check_errors()?;
    ensure!(
        final_tick > start_tick,
        "fresh Host completed no simulation ticks"
    );
    operations["host_startup"]["status"] = json!("passed");
    Ok(())
}

fn player_operations(
    app: &mut Application,
    limits: &Limits,
    evidence: &mut Evidence,
    operations: &mut Value,
) -> Result<()> {
    execute(
        app,
        Command::Playing { value: false },
        limits,
        &mut evidence.commands,
    )?;
    let residents = app.residents()?;
    ensure!(
        !residents.is_empty(),
        "fresh household has no selectable residents"
    );
    operations["resident_selection"] = json!({"status":"attempted","selected":[]});
    for identity in &residents {
        execute(
            app,
            Command::Resident {
                identity: identity.clone(),
            },
            limits,
            &mut evidence.commands,
        )?;
        ensure!(
            app.viewer.resident == identity.object,
            "resident selection not installed"
        );
        ensure!(
            Milestone::Selected {
                actor: identity.clone()
            }
            .reached(app, 0)?,
            "native selected resident differs"
        );
        operations["resident_selection"]["selected"]
            .as_array_mut()
            .unwrap()
            .push(json!(identity));
    }
    operations["resident_selection"]["status"] = json!("passed");

    let has_buy_owner = app.session()?.host.presentation.tool_selection.is_some();
    let buy_disabled = app
        .viewer
        .device
        .as_ref()
        .and_then(|device| device.panel_widgets())
        .is_some_and(|widgets| {
            !widgets
                .button(sims_vm::interface::view_control::Widget::Buy)
                .enabled
        });
    if !has_buy_owner || buy_disabled {
        operations["buy_live"] = json!({"status":"not_tested_unsupported_by_current_fresh_gui",
            "tool_owner_present":has_buy_owner,"button_disabled":buy_disabled,
            "optional_buy_preview_enabled":app.viewer.args.buy_preview});
    } else {
        operations["buy_live"] = json!({"status":"attempted"});
        execute(app, Command::Buy, limits, &mut evidence.commands)?;
        ensure!(
            Milestone::Mode { value: 1 }.reached(app, 0)?,
            "Buy mode did not become active"
        );
        operations["buy_live"]["buy_reached"] = json!(true);
        execute(app, Command::Live, limits, &mut evidence.commands)?;
        ensure!(
            Milestone::Mode { value: 2 }.reached(app, 0)?,
            "Live mode did not become active"
        );
        operations["buy_live"]["status"] = json!("passed");
    }

    let session = app.session()?;
    let target = app.drawn_targets()?.into_iter().find(|identity| {
        session
            .simulation
            .vm
            .world
            .objects
            .get(&identity.object)
            .and_then(|object| object.definition.as_ref())
            .is_some_and(|definition| definition.source.to_ascii_lowercase().contains("chair"))
    });
    operations["interaction_queue_cancel"] = json!({"status":"unsupported_in_bounded_smoke",
        "reason":"no drawn chair with an offered Sit action inspected"});
    if let Some(target) = target {
        operations["interaction_queue_cancel"] = json!({"status":"attempted","target":target});
        execute(
            app,
            Command::Object { identity: target },
            limits,
            &mut evidence.commands,
        )?;
        let actions = app
            .menu()
            .context("chair interaction menu absent")?
            .actions();
        operations["interaction_queue_cancel"]["offered"] = json!(actions);
        let choice = actions
            .iter()
            .enumerate()
            .find(|(_, action)| {
                action.flags & 16 == 0
                    && action
                        .label
                        .as_deref()
                        .is_some_and(|label| ["Sit", "Sit Here"].contains(&label))
            })
            .map(|(index, action)| (index, action.clone()));
        if let Some((index, expected)) = choice {
            let actor = residents.last().unwrap();
            let before = action_ids(app, actor.object)?;
            execute(
                app,
                Command::Choice { index, expected },
                limits,
                &mut evidence.commands,
            )?;
            let queued: Vec<_> = action_ids(app, actor.object)?
                .into_iter()
                .filter(|serial| !before.contains(serial))
                .collect();
            operations["interaction_queue_cancel"]["new_serials"] = json!(queued);
            ensure!(
                !queued.is_empty(),
                "offered Sit choice did not produce an observable queued action"
            );
            for serial in &queued {
                execute(
                    app,
                    Command::CancelAction {
                        actor: actor.clone(),
                        serial: *serial,
                    },
                    limits,
                    &mut evidence.commands,
                )?;
            }
            operations["interaction_queue_cancel"]["status"] =
                json!("queue_observed_cancellation_requested");
            operations["interaction_queue_cancel"]["cancellation_completion_verified"] =
                json!(false);
        } else {
            execute(app, Command::CloseMenu, limits, &mut evidence.commands)?;
            operations["interaction_queue_cancel"]["status"] = json!("unsupported_offered_labels");
        }
    }

    Ok(())
}

fn save_reload(
    app: &mut Application,
    limits: &Limits,
    evidence: &mut Evidence,
    operations: &mut Value,
) -> Result<()> {
    execute(
        app,
        Command::Playing { value: false },
        limits,
        &mut evidence.commands,
    )?;
    operations["save_reload"] = json!({"status":"attempted","path":app.viewer.args.save});
    evidence
        .events
        .push(json!({"kind":"production_message","message":"Save"}));
    app.dispatch(Message::Save)?;
    let expected = continuation(app)?;
    let before = app.observe()?;
    ensure!(
        app.viewer.args.save.is_file(),
        "production Save handler produced no file"
    );
    evidence
        .events
        .push(json!({"kind":"production_message","message":"Load","input_checkpoint":false}));
    app.dispatch(Message::Load)?;
    let actual = continuation(app)?;
    let after = app.observe()?;
    operations["save_reload"] = json!({"status":"compared","expected_state_sha256":expected,
        "actual_state_sha256":actual,"before_tick":before.tick,"after_tick":after.tick,
        "before_rng":before.rng,"after_rng":after.rng,"path":app.viewer.args.save,
        "input_checkpoint_restored":false,"restored_own_fresh_play_save":true});
    ensure!(
        expected == actual,
        "managed save/reload session/client/device continuation differs"
    );
    ensure!(
        before.tick == after.tick && before.rng == after.rng,
        "managed save/reload tick or RNG differs"
    );
    operations["save_reload"]["status"] = json!("passed");
    app.drain(limits)
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let relative = PathBuf::from(std::env::var("SIMS_FRESH_PLAY_OUTPUT")?);
    ensure!(
        relative.starts_with("cache")
            && relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "fresh-play output must stay in cache"
    );
    let out = root.join(relative);
    std::fs::create_dir(&out).context("refusing to overwrite fresh-play evidence")?;
    let source = std::env::var("SIMS_FRESH_PLAY_SOURCE")
        .unwrap_or_else(|_| "UserData/Houses/House05.iff".into());
    let minutes = std::env::var("SIMS_FRESH_PLAY_MINUTES")
        .unwrap_or_else(|_| "2".into())
        .parse::<i64>()?;
    let wall_seconds = std::env::var("SIMS_FRESH_PLAY_WALL_SECONDS")
        .unwrap_or_else(|_| "300".into())
        .parse::<f64>()?;
    ensure!(
        (1..=1440).contains(&minutes),
        "fresh-play calendar budget differs"
    );
    ensure!(
        (1.0..=900.0).contains(&wall_seconds),
        "fresh-play wall budget differs"
    );
    let limits = Limits {
        commands: 100_000,
        completed_ticks: 50_000,
        service_polls: 10_000,
        wall_seconds,
    };
    let mut args = Args::parse_from([
        "sims-room",
        "--original",
        &source,
        "--root",
        root.join("game-data/cabinet-extracted").to_str().unwrap(),
        "--inferred-routing",
        "--floor",
        "1",
        "--paused",
        "--save",
        out.join("player-save.json").to_str().unwrap(),
    ]);
    args.buy_preview = match std::env::var("SIMS_FRESH_PLAY_BUY_PREVIEW").as_deref() {
        Ok("1") => true,
        Ok("0") | Err(std::env::VarError::NotPresent) => false,
        _ => anyhow::bail!("SIMS_FRESH_PLAY_BUY_PREVIEW must be 0 or 1"),
    };
    args.neighborhood_policy = match std::env::var("SIMS_FRESH_PLAY_NEIGHBORHOOD_POLICY") {
        Ok(policy) => {
            ensure!(
                policy == super::super::neighborhood_usability::POLICY,
                "unsupported fresh-play neighborhood policy: {policy}"
            );
            Some(policy)
        }
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let requested_configuration = json!({
        "buy_preview":args.buy_preview,
        "neighborhood_policy":args.neighborhood_policy,
        "inferred_routing":args.inferred_routing,
        "routing_flag":"--inferred-routing",
    });
    ensure!(
        args.load.is_none() && args.at_tick == 0,
        "fresh-play cannot restore or fast-forward its input"
    );
    let wall = Instant::now();
    let mut app = None;
    let mut start_tick = None;
    let mut start_calendar = None;
    let mut household = None;
    let mut load_seconds = None;
    let mut play_succeeded = false;
    let mut post_reload_play_succeeded = false;
    let mut stage = "fresh_load";
    let mut evidence = Evidence {
        commands: vec![],
        events: vec![],
        samples: vec![],
        frames: 0,
        dialogs: 0,
    };
    let mut operations = json!({"resident_selection":{"status":"not_reached"},
        "interaction_queue_cancel":{"status":"not_reached"},"buy_live":{"status":"not_reached"},
        "save_reload":{"status":"not_reached"},"host_startup":{"status":"not_requested"}});
    let result = (|| -> Result<()> {
        let ready = Arc::new(Ready::load(&args)?);
        app = Some(Application::install_ready(
            &args,
            ready,
            Instant::now(),
            &limits,
        )?);
        load_seconds = Some(wall.elapsed().as_secs_f64());
        let app = app.as_mut().unwrap();
        app.check_errors()?;
        household = app
            .session()?
            .house
            .household
            .as_ref()
            .map(|family| json!({"name":family.name,"residents":family.residents}));
        start_tick = Some(app.session()?.simulation.vm.tick);
        start_calendar = Some(calendar(app)?);
        if std::env::var("SIMS_FRESH_PLAY_HOST").as_deref() == Ok("1") {
            stage = "fresh_host_startup";
            host_startup(app, wall, &limits, &mut evidence, &mut operations)?;
        }
        stage = "fresh_play";
        play_for(app, minutes * 60, stage, wall, &limits, &mut evidence)?;
        play_succeeded = true;
        stage = "managed_save_reload";
        bounded(app, wall, &limits, evidence.frames)?;
        save_reload(app, &limits, &mut evidence, &mut operations)?;
        stage = "play_after_own_save_reload";
        play_for(app, 60, stage, wall, &limits, &mut evidence)?;
        post_reload_play_succeeded = true;
        stage = "player_operations";
        bounded(app, wall, &limits, evidence.frames)?;
        player_operations(app, &limits, &mut evidence, &mut operations)?;
        Ok(())
    })();
    let current_tick = app
        .as_ref()
        .and_then(|app| app.session().ok())
        .map(|session| session.simulation.vm.tick);
    let current_calendar = app.as_ref().and_then(|app| calendar(app).ok());
    let application_fault = app
        .as_ref()
        .is_some_and(|app| app.viewer.error.is_some() || app.viewer.menu_error.is_some());
    let stop_kind = result.as_ref().err().map(|error| {
        let message = error.to_string();
        if application_fault {
            "application_fault"
        } else if message.contains("uncommanded pause") {
            "uncommanded_pause_not_diagnosed_as_VM_fault"
        } else if message.contains("unplanned original") {
            "unsupported_dialog"
        } else if message.contains("limit") {
            "harness_budget"
        } else {
            "load_or_player_path_check_failed"
        }
    });
    let failure_snapshot_error = if result.is_err() {
        app.as_ref()
            .map(|app| {
                app.checkpoint().and_then(|bytes| {
                    std::fs::write(out.join("first-failure.json"), bytes)?;
                    Ok(())
                })
            })
            .transpose()
            .err()
            .map(|error| format!("{error:#}"))
    } else {
        None
    };
    write_json(
        &out.join("commands.json"),
        &serde_json::to_value(&evidence.commands)?,
    )?;
    write_json(
        &out.join("report.json"),
        &json!({"success":result.is_ok(),"source":source,
        "household":household,"build_profile":if cfg!(debug_assertions) {"debug"} else {"release"},
        "load_kind":"fresh original household via Ready::load and Viewer::install",
        "restored_input":false,"injected_VM_state":false,"authored_BHAV_entry":false,
        "routing":"explicit --inferred-routing (unverified; not the Finder/default policy)","target_game_minutes":minutes,
        "requested_configuration":requested_configuration,
        "retained_configuration":app.as_ref().map(|app| &app.viewer.application),
        "start_tick":start_tick,"final_tick":current_tick,"start_calendar":start_calendar,
        "final_calendar":current_calendar,"frames":evidence.frames,"load_seconds":load_seconds,
        "play_succeeded":play_succeeded,"post_reload_play_succeeded":post_reload_play_succeeded,
        "operations":operations,"stage":stage,"stop_kind":stop_kind,"events":evidence.events,"samples":evidence.samples,
        "limits":{"commands":limits.commands,"completed_ticks_per_phase":limits.completed_ticks,
            "service_polls_per_command":limits.service_polls,"frames":100_000,"dialogs":16,
            "wall_seconds":wall_seconds,"wall_limit":"cooperative between handlers; not a native-frame wall cap"},
        "playing":app.as_ref().map(|app| app.viewer.playing),
        "application_error":app.as_ref().and_then(|app| app.viewer.error.as_deref()),
        "menu_error":app.as_ref().and_then(|app| app.viewer.menu_error.as_deref()),
        "modal":app.as_ref().and_then(|app| app.session().ok()).and_then(|session| session.simulation.vm.world.dialogs.modal),
        "observed_tabloid_stack":app.as_ref().is_some_and(|app| app.paths.iter().any(|path| path.contains("Tabloid.iff:"))),
        "wall_seconds":wall.elapsed().as_secs_f64(),"first_error":result.as_ref().err().map(|error| format!("{error:#}")),
        "failure_snapshot_error":failure_snapshot_error,"strict_original_parity_ticks":0,
        "full_player_path_qualified":false,
        "checks":"engine/player-path phases bypass Host scheduling; optional host_startup is explicitly scoped; real loading and production handlers; unsupported operations receive no coverage credit; no window, GPU, responsiveness, full-milestone or parity claim"}),
    )?;
    result
}

#[test]
fn fresh_play_distinguishes_modal_pause_uncommanded_pause_and_fault() -> Result<()> {
    assert_eq!(
        next_input(None, Some(14), true, false, true, false)?,
        NextInput::Dialog(14)
    );
    assert_eq!(
        next_input(None, None, false, false, false, false)?,
        NextInput::Play
    );
    assert_eq!(
        next_input(None, None, false, false, true, true)?,
        NextInput::Play
    );
    assert_eq!(
        next_input(None, None, false, true, true, false)?,
        NextInput::Frame
    );
    assert_eq!(
        next_input(None, None, true, false, true, true)?,
        NextInput::Poll
    );
    assert!(
        next_input(None, None, false, false, true, false)
            .unwrap_err()
            .to_string()
            .contains("not a VM-fault diagnosis")
    );
    assert_eq!(
        next_input(
            Some("retained VM failure"),
            Some(14),
            true,
            false,
            true,
            true
        )
        .unwrap_err()
        .to_string(),
        "retained VM failure"
    );
    Ok(())
}





#[test]
fn exploration_cli_accepts_fresh_household_without_a_checkpoint() {
    let args = Args::try_parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--exploration-plan",
        "plan.json",
        "--exploration-output",
        "cache/fresh-play",
    ])
    .expect("fresh original exploration must not require --load");
    assert!(args.load.is_none());
    assert_eq!(args.at_tick, 0);
    assert!(args.exploration_plan.is_some());
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
#[path = "startup_play/private_tests.rs"]
#[rustfmt::skip]
mod private_tests;
