//! Bounded headless gameplay trace from a real original save. A trace produced
//! here is not a parity result until compared with aligned original boundaries.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Options, Progress, Session, completion};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};
#[path = "support/family_window.rs"]
mod family_window;
#[path = "support/opening.rs"]
mod opening;

/// Complete retained owner export only. Never call producer/tuning getters here.
fn retained_process(world: &sims_vm::World) -> Value {
    json!({"schema":1,"fields":sims_vm::autonomy_constants::ORDER,
        "autonomy_constants":world.autonomy_constants})
}

pub(crate) fn state(s: &Session) -> Value {
    let vm = &s.simulation.vm;
    let placement = s.simulation.native_lot.as_ref().map(|lot| &lot.placement);
    // Read retained owners directly. Missing historical owners stay null;
    // this observer must not reconstruct or initialize simulation state.
    let retained = json!({
        "schema":1,
        "resource_globals":vm.world.resource_storage.records,
        "active_size":vm.world.map_size.as_ref().map(|m|m.active_size),
        "registry_spatial_order":vm.world.registry_order,
        "placement":placement,
        "registry":vm.world.scheduler_storage.as_ref().map(|storage|json!({
            "ids":(1..=storage.len()).map(|i|if storage.contains(i as i16){i as i16}else{0}).collect::<Vec<_>>(),
            "words":(1..=storage.len()).map(|i|storage.word(&vm.world,i as i16).ok()).collect::<Vec<_>>(),
            "object_order":vm.world.person.as_ref().map(|p|&p.object_order),
            "pending_removals":s.simulation.removals.iter().map(|p|p.target).collect::<Vec<_>>()
        })),
        "objects":vm.world.objects.iter().map(|(&id,o)|json!({
            "id":id,"definition":o.definition,
            "tree_error":o.native_tree_error,"native_flags":o.native_flags,
            "keep_dynamic":o.native_keep_dynamic,
            "slots":o.slots,"motive_update":o.motive_update,"motive_changes":o.motive_changes
        })).collect::<Vec<_>>()
    });
    json!({
        "tick":vm.tick,"global_rng":vm.rng_state,"lot_rng":vm.lot_rng_state,
        "crt_rng":vm.crt_rng_state,"lot_rng_scope":vm.lot_rng_scope,"globals":vm.world.globals,
        "actions":vm.world.actions,"positions":vm.world.positions,
        "scheduler_cursor":s.simulation.native_scheduler.as_ref().map(|s|s.cursor),
        "budget":vm.world.budget,
        "retained_owners":retained,
        "retained_process":retained_process(&vm.world),
        "retained_family_window":family_window::retained(s.host.presentation.family_window.as_ref()),
        // Export the actual Rust neighborhood owners for the enriched native
        // comparison. This is evidence, not a reconstructed native record:
        // absent family/cache/history fields must remain explicitly unproven.
        "retained_neighborhood":{
            "schema":1,
            "words":vm.world.neighborhood,
            "person_world":vm.world.person,
            "magic_history":vm.world.magic_history
        },
        "objects":vm.world.objects.iter().map(|(&id,o)| {
            let thread=vm.object_threads.get(&id).map(|id| &vm.threads[id]);
            json!({"id":id,"guid":o.definition.as_ref().map(|d|d.guid),
                "data":o.data,"temps":o.temps,"attributes":o.attributes,
                "registry_word":o.idle.as_ref().map(|idle| idle.registry_word()),
                "relationships":{"rows":o.relationships,
                    "order":o.relationship_keys().ok().map(|ids|ids.into_owned())},
                "person":o.person,"motives":o.motives,"motive_changes":o.motive_changes,
                "motive_update":o.motive_update,
                "thread":thread})
        }).collect::<Vec<_>>()
    })
}
fn write(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let mut file = BufWriter::new(File::create_new(path)?);
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    Ok(())
}
fn cold_restore(index: &Index, session: &Session, managed: bool) -> Result<Session> {
    if managed {
        // Exercise the actual storage encoding as well as retained restoration.
        let encoded = serde_json::to_vec(&session.save_managed()?)?;
        Session::restore(index, serde_json::from_slice(&encoded)?)
    } else {
        Session::restore(index, session.save())
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ObservedEvent {
    NormalFrame {
        frame: u64,
    },
    UiPoll {
        serial: u64,
        resource: Option<i32>,
    },
    FamilyWindowUpdate {
        serial: u64,
        observation: String,
        raw_argument: u32,
    },
}
struct ObservedWork<'a> {
    next_frame: u64,
    family: Option<family_window::Replay>,
    cold_index: &'a Index,
    managed: bool,
    out: &'a Path,
}

/// Explicit application work observed between completed updates or inside a
/// modal continuation. This is not passive rendering and does not invent a
/// once-per-tick cadence. The original frame can update retained gameplay
/// caches; only the scheduler clock must remain unchanged here.
fn apply_observed_events(
    session: &mut Session,
    resumed: &mut Session,
    events: &Value,
    work: &mut ObservedWork<'_>,
) -> Result<()> {
    let events: Vec<ObservedEvent> = serde_json::from_value(events.clone())?;
    for event in events {
        let tick = session.simulation.vm.tick;
        match event {
            ObservedEvent::FamilyWindowUpdate {
                serial,
                observation,
                raw_argument,
            } => {
                let callback = family_window::Event {
                    serial,
                    observation,
                    raw_argument,
                };
                work.family
                    .as_mut()
                    .context("family callback event has no explicit binding/provenance")?
                    .apply(
                        session,
                        resumed,
                        &callback,
                        |s| cold_restore(work.cold_index, s, work.managed),
                        work.out,
                    )?;
            }
            ObservedEvent::NormalFrame { frame } => {
                ensure!(
                    frame == work.next_frame,
                    "missing or reordered observed frame"
                );
                session.prepare_view_frame()?;
                resumed.prepare_view_frame()?;
                work.next_frame += 1;
            }
            ObservedEvent::UiPoll { serial, resource } => {
                ensure!(serial != 0, "observed UI poll has no call identity");
                opening::poll_resource(session, resource)?;
                opening::poll_resource(resumed, resource)?;
            }
        }
        ensure!(
            session.simulation.vm.tick == tick && resumed.simulation.vm.tick == tick,
            "observed application work advanced a gameplay tick"
        );
        ensure!(
            serde_json::to_value(session.save())? == serde_json::to_value(resumed.save())?,
            "observed application work differs after cold restore"
        );
    }
    Ok(())
}
fn main() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("parity trace panicked"))?
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        matches!(args.len(), 4 | 5 | 6 | 7),
        "usage: newbie_parity CONTENT_ROOT SOURCE NEW_OUTPUT_DIR CRT_SEED [RECORDED_INPUTS_JSON] [PROCESS_SKELETON_CACHE_JSON] [RETAINED_STARTUP_REPORT_JSON]"
    );
    let out = Path::new(&args[2]);
    ensure!(
        out.parent()
            .context("output parent")?
            .canonicalize()?
            .starts_with(Path::new("cache").canonicalize()?),
        "private output required"
    );
    std::fs::create_dir(out).context("refusing to replace evidence")?;
    let index = Index::scan(Path::new(&args[0]))?;
    let seed = args[3].parse()?;
    let inputs: Vec<Value> = if let Some(path) = args.get(4) {
        serde_json::from_reader::<_, Value>(File::open(path)?)?["inputs"]
            .as_array()
            .context("recorded input list")?
            .clone()
    } else {
        vec![]
    };
    let mut input_index = 0;
    // Keep historical research reactivation explicit and available. Managed
    // restoration links retained state without re-running starting-house load.
    let managed_restores = if let Some(path) = args.get(4) {
        let data: Value = serde_json::from_reader(File::open(path)?)?;
        match data.get("cold_restore_format") {
            None => false,
            Some(Value::String(value)) if value == "research_v1" => false,
            Some(Value::String(value)) if value == "managed_v2" => true,
            _ => anyhow::bail!("cold_restore_format must be research_v1 or managed_v2"),
        }
    } else {
        false
    };
    let restore_content_root: Option<String> = args
        .get(4)
        .map(|path| -> Result<_> {
            let data: Value = serde_json::from_reader(File::open(path)?)?;
            data.get("restore_content_root")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(Into::into)
        })
        .transpose()?
        .flatten();
    ensure!(
        restore_content_root.is_none() || managed_restores,
        "separate immutable restore content requires managed_v2"
    );
    let restore_index = restore_content_root
        .as_ref()
        .map(|root| Index::scan(Path::new(root)))
        .transpose()?;
    if let Some(assets) = &restore_index {
        ensure!(
            assets
                .resources
                .iter()
                .all(|r| !r.locator.to_ascii_lowercase().starts_with("userdata")),
            "separate restore content contains original save namespaces"
        );
    }
    let cold_index = restore_index.as_ref().unwrap_or(&index);
    // Keep the historical 32-update diagnostic default. Longer captured runs
    // must explicitly request their bound in the preserved input schedule.
    let updates = args
        .get(4)
        .map(|path| -> Result<_> {
            let data: Value = serde_json::from_reader(File::open(path)?)?;
            data.get("completed_updates")
                .map(|n| {
                    n.as_u64()
                        .context("completed_updates must be an unsigned integer")
                })
                .transpose()
        })
        .transpose()?
        .flatten()
        .unwrap_or(32);
    ensure!(updates <= 4096, "completed_updates must be at most 4096");
    // An observed application poll schedule replaces this diagnostic's usual
    // once-per-update cadence. Empty intervals mean no poll, not a null poll.
    let ui_schedule: Option<Vec<Value>> = args
        .get(4)
        .map(|path| -> Result<_> {
            let data: Value = serde_json::from_reader(File::open(path)?)?;
            data.get("ui_polls_before_updates")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(Into::into)
        })
        .transpose()?
        .flatten();
    let mut ui_schedule_index = 0;
    let application_schedule: Option<Vec<Value>> = args
        .get(4)
        .map(|path| -> Result<_> {
            let data: Value = serde_json::from_reader(File::open(path)?)?;
            data.get("application_events_before_updates")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(Into::into)
        })
        .transpose()?
        .flatten();
    ensure!(
        application_schedule.is_none() || ui_schedule.is_none(),
        "choose ordered application events or legacy UI-poll schedule"
    );
    let mut application_schedule_index = 0;
    // These are explicit callback-body observations only. No callback is
    // inferred when absent, and retained incoming process/provider state is
    // still consumed by the existing loader without a fresh bootstrap repair.
    let callback_input: Option<Value> = args
        .get(4)
        .map(|path| -> Result<_> { Ok(serde_json::from_reader(File::open(path)?)?) })
        .transpose()?;
    let family =
        family_window::Replay::load(callback_input.as_ref().and_then(|v| v.get("family_window")))?;
    let startup_family_events = family_window::startup_events(
        callback_input
            .as_ref()
            .and_then(|v| v.get("startup_family_window_events_before_first_poll")),
        family.is_some(),
    )?;
    let mut observed_work = ObservedWork {
        next_frame: 0,
        family,
        cold_index,
        managed: managed_restores,
        out,
    };
    let application_context = args
        .get(4)
        .map(|path| -> Result<_> {
            let data: Value = serde_json::from_reader(File::open(path)?)?;
            data.get("application_context")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(Into::into)
        })
        .transpose()?
        .flatten();
    let mut report = json!({"scope":"Bounded Rust trace; original comparison remains separate",
        "crt_seed":seed,"source":args[1],"matched_original_ticks":0,"requested_rust_updates":updates,"applied_inputs":[],
        "cold_service_restores":[],"applied_ui_update_polls":[],"applied_application_events":[],
        "cold_restore_format":if managed_restores {"managed_v2"} else {"research_v1"},
        "restore_content_root":restore_content_root,
        "encoded_cold_restores":managed_restores});
    let resources = args
        .get(5)
        .map(|path| -> Result<_> { Ok(serde_json::from_reader(File::open(path)?)?) })
        .transpose()?
        .unwrap_or_default();
    let incoming: Option<sims_content::house_startup::Incoming> = args
        .get(6)
        .map(|path| -> Result<_> {
            let report: Value = serde_json::from_reader(File::open(path)?)?;
            Ok(serde_json::from_value(report["incoming"].clone())?)
        })
        .transpose()?;
    // An explicit experiment may continue beyond a strict routing boundary.
    // The default remains strict; record the policy alongside every report so
    // a conditional path experiment cannot be mistaken for verified routing.
    let routing: sims_sim::lot_route::path::Policy = args
        .get(4)
        .map(|path| -> Result<_> {
            let data: Value = serde_json::from_reader(File::open(path)?)?;
            Ok(data
                .get("diagnostic_routing")
                .cloned()
                .map(serde_json::from_value)
                .transpose()?
                .unwrap_or_default())
        })
        .transpose()?
        .unwrap_or_default();
    report["routing_policy"] = serde_json::to_value(routing)?;
    if observed_work.family.is_some() {
        report["callback_load_input"] = json!({
            "retained_incoming":incoming.is_some(),
            "retained_process":incoming.as_ref().and_then(|i|i.vm.world.autonomy_constants.as_ref()),
            "fresh_named_process_profile_applies":incoming.is_none(),
            "retained_provider_supplied_by_this_harness":false,
            "scope":"callback body; no claim that an older unknown incoming can replay the fresh original prefix"
        });
        write(
            &out.join("declared-callback-input.json"),
            &json!({"load":report["callback_load_input"],"family_window":callback_input.as_ref().unwrap()["family_window"],"startup_events":startup_family_events}),
        )?;
    }
    let mut session = Session::load_with_startup(
        &index,
        &args[1],
        Options {
            application_context,

            native_postload: true,
            native_load_completion: true,
            crt_seed: seed,
            routing,
            ..Default::default()
        },
        true,
        resources,
        incoming,
    )?;
    report["loaded_tick"] = json!(session.simulation.vm.tick);
    write(&out.join("loaded-state.json"), &state(&session))?;
    let result = (|| -> Result<()> {
        let mut device = Device::open_for_household(
            &mut session,
            800,
            600,
            0,
            completion::Config {
                controller_suppressed: 0,
                mode: 0,
            },
        )?;
        let mut client = opening::bind(&mut session, &mut device)?;
        device.publish_live_interface(&mut session)?;
        client.bind_live_panel(&session)?;
        if let Some(family) = observed_work.family.as_ref() {
            family.bind(&mut session)?;
        }
        for event in &startup_family_events {
            let mut cold = cold_restore(cold_index, &session, managed_restores)?;
            observed_work
                .family
                .as_mut()
                .context("startup callback unbound")?
                .apply(
                    &mut session,
                    &mut cold,
                    event,
                    |s| cold_restore(cold_index, s, managed_restores),
                    out,
                )?;
        }
        // Existing replacement-client startup poll/frame remain diagnostic.
        // The source callback proves it preceded the next poll entry, not the
        // complete widget-input protocol or universal graphics cadence.
        report["startup_poll_frame_scope"] = json!(
            "existing replacement-client diagnostic; callback source does not qualify full prefix"
        );
        opening::poll(&mut session, &client)?;
        session.prepare_view_frame()?;
        device.refresh(&session)?;
        report["startup_completed"] = json!(true);
        write(&out.join("opened-state.json"), &state(&session))?;
        software::render(device.scene()).save(out.join("opened.png"))?;
        let saved = session.save();
        write(&out.join("opened-session.json"), &saved)?;
        let saved_client = client.save(&session)?;
        write(&out.join("opened-client.json"), &saved_client)?;
        //Retain the actual opened device. An independently restored Session
        //already owns transport/panel state, so Device::new cannot reconstruct
        //this binding without inventing the missing frontend recipe.
        write(&out.join("opened-device.json"), &device.save(&session)?)?;
        ensure!(
            serde_json::to_value(&saved)? == serde_json::to_value(session.save())?,
            "capturing opened frontend owners changed retained simulation state"
        );
        if managed_restores {
            write(
                &out.join("opened-managed-session.json"),
                &session.save_managed()?,
            )?;
        }
        if restore_index.is_some() {
            let error = Session::restore(cold_index, session.save())
                .err()
                .context("research restore unexpectedly succeeded without starting saves")?;
            report["research_restore_without_starting_saves_rejected"] =
                json!(format!("{error:#}"));
        }
        let mut resumed = cold_restore(cold_index, &session, managed_restores)?;
        let resumed_client = sims_render::dialog::client::Client::restore(&resumed, saved_client)?;
        ensure!(
            serde_json::to_value(session.save())? == serde_json::to_value(resumed.save())?,
            "restored startup differs"
        );
        report["startup_restore_equal"] = json!(true);
        let mut trace = BufWriter::new(File::create_new(out.join("ticks.jsonl"))?);
        let start = session.simulation.vm.tick;
        for i in 0..=updates {
            let current = state(&session);
            serde_json::to_writer(&mut trace, &current)?;
            writeln!(trace)?;
            trace.flush()?;
            ensure!(
                serde_json::to_value(session.save())? == serde_json::to_value(resumed.save())?,
                "save/resume diverged at update {i}"
            );
            // Reload at every completed boundary too: running route frames
            // need to survive a cold restore even without a modal service.
            resumed = cold_restore(cold_index, &session, managed_restores)?;
            ensure!(
                serde_json::to_value(session.save())? == serde_json::to_value(resumed.save())?,
                "cold tick restore differs at update {i}"
            );
            report["cold_tick_restores"] = json!(i + 1);
            report["rust_updates"] = json!(i);
            report["last_tick"] = json!(session.simulation.vm.tick);
            if i == updates {
                break;
            }
            if let Some(schedule) = &application_schedule {
                while let Some(observation) = schedule.get(application_schedule_index) {
                    let tick = observation["after_completed_tick"]
                        .as_u64()
                        .context("recorded application-event tick")?;
                    ensure!(
                        tick >= session.simulation.vm.tick,
                        "missed application events"
                    );
                    if tick != session.simulation.vm.tick {
                        break;
                    }
                    apply_observed_events(
                        &mut session,
                        &mut resumed,
                        &observation["events"],
                        &mut observed_work,
                    )?;
                    report["applied_application_events"]
                        .as_array_mut()
                        .unwrap()
                        .push(observation.clone());
                    application_schedule_index += 1;
                }
            } else if let Some(schedule) = &ui_schedule {
                while let Some(observation) = schedule.get(ui_schedule_index) {
                    let tick = observation["after_completed_tick"]
                        .as_u64()
                        .context("recorded UI poll tick")?;
                    ensure!(
                        tick >= session.simulation.vm.tick,
                        "missed recorded UI poll"
                    );
                    if tick != session.simulation.vm.tick {
                        break;
                    }
                    for resource in observation["resources"]
                        .as_array()
                        .context("UI poll resources")?
                    {
                        let resource: Option<i32> = serde_json::from_value(resource.clone())?;
                        opening::poll_resource(&mut session, resource)?;
                        opening::poll_resource(&mut resumed, resource)?;
                        ensure!(
                            serde_json::to_value(session.save())?
                                == serde_json::to_value(resumed.save())?,
                            "recorded between-update UI poll restore differs"
                        );
                    }
                    report["applied_ui_update_polls"]
                        .as_array_mut()
                        .unwrap()
                        .push(observation.clone());
                    ui_schedule_index += 1;
                }
            } else if i != 0 {
                opening::poll(&mut session, &client)?;
                opening::poll(&mut resumed, &resumed_client)?;
            }
            loop {
                let step = session.step()?;
                let resumed_step = resumed.step()?;
                ensure!(
                    format!("{step:?}") == format!("{resumed_step:?}"),
                    "restored progress differs"
                );
                ensure!(
                    serde_json::to_value(session.save())? == serde_json::to_value(resumed.save())?,
                    "save/resume diverged inside update {i}"
                );
                if matches!(step, Progress::Tick) {
                    break;
                }
                // Reload the full managed session at each externally yielded
                // service, then compare its continuation on the next step.
                resumed = cold_restore(cold_index, &session, managed_restores)?;
                ensure!(
                    serde_json::to_value(session.save())? == serde_json::to_value(resumed.save())?,
                    "cold service restore differs"
                );
                report["cold_service_restores"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({
                        "tick":session.simulation.vm.tick,"progress":format!("{step:?}"),
                        "lot_scope_active":session.simulation.vm.lot_rng_scope.is_some()
                    }));
                if matches!(step, Progress::Dialog) && input_index < inputs.len() {
                    let input = &inputs[input_index];
                    ensure!(
                        input["after_completed_tick"] == session.simulation.vm.tick,
                        "recorded dialog input occurred at a different tick"
                    );
                    let dialogs = &session.simulation.vm.world.dialogs;
                    let id = dialogs.modal.context("no modal at recorded input")?;
                    let dialog = dialogs
                        .records
                        .iter()
                        .find(|r| r.id == id)
                        .context("modal record")?;
                    let expected = input["text_contains"]
                        .as_str()
                        .context("recorded dialog text")?;
                    ensure!(
                        dialog.body.contains(expected),
                        "different dialog at recorded input: {}",
                        dialog.body
                    );
                    let choice = input["choice"].as_str().context("recorded button label")?;
                    let result = dialog
                        .buttons
                        .iter()
                        .find(|b| b.label == choice)
                        .context("recorded button absent")?
                        .result;
                    report["applied_inputs"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({
                        "tick":session.simulation.vm.tick,"id":id,"origin":dialog.origin,
                        "body":dialog.body,"choice":choice,"result":result}));
                    ensure!(
                        input.get("result").is_none_or(|v| *v == json!(result)),
                        "recorded dialog result differs from offered button"
                    );
                    if let Some(events) = input.get("application_events_before_answer") {
                        ensure!(
                            input.get("ui_polls_before_answer").is_none(),
                            "dialog has two application-event schedules"
                        );
                        apply_observed_events(
                            &mut session,
                            &mut resumed,
                            events,
                            &mut observed_work,
                        )?;
                        report["applied_inputs"]
                            .as_array_mut()
                            .unwrap()
                            .last_mut()
                            .unwrap()["application_events_before_answer"] = events.clone();
                    } else if let Some(polls) = input.get("ui_polls_before_answer") {
                        // Explicit application observations, with their source
                        // and timing recorded by the input file. Older input
                        // files also use this for labeled conditional probes.
                        for resource in polls.as_array().context("UI poll inputs")? {
                            let resource: Option<i32> = serde_json::from_value(resource.clone())?;
                            opening::poll_resource(&mut session, resource)?;
                            opening::poll_resource(&mut resumed, resource)?;
                            ensure!(
                                serde_json::to_value(session.save())?
                                    == serde_json::to_value(resumed.save())?,
                                "recorded UI poll restore differs"
                            );
                        }
                        report["applied_inputs"]
                            .as_array_mut()
                            .unwrap()
                            .last_mut()
                            .unwrap()["ui_polls_before_answer"] = polls.clone();
                    }
                    session
                        .simulation
                        .vm
                        .answer_dialog(id, result)
                        .map_err(anyhow::Error::msg)?;
                    resumed
                        .simulation
                        .vm
                        .answer_dialog(id, result)
                        .map_err(anyhow::Error::msg)?;
                    if let Some(events) = input.get("application_events_after_answer") {
                        apply_observed_events(
                            &mut session,
                            &mut resumed,
                            events,
                            &mut observed_work,
                        )?;
                        report["applied_inputs"]
                            .as_array_mut()
                            .unwrap()
                            .last_mut()
                            .unwrap()["application_events_after_answer"] = events.clone();
                    }
                    input_index += 1;
                    continue;
                }
                anyhow::bail!("original input or host work required: {step:?}");
            }
            ensure!(
                session.simulation.vm.tick == start + i + 1,
                "unexpected tick increment"
            );
        }
        ensure!(input_index == inputs.len(), "unconsumed dialog inputs");
        ensure!(
            ui_schedule
                .as_ref()
                .is_none_or(|s| ui_schedule_index == s.len()),
            "unconsumed UI poll observations"
        );
        ensure!(
            application_schedule
                .as_ref()
                .is_none_or(|s| application_schedule_index == s.len()),
            "unconsumed application-event observations"
        );
        ensure!(
            observed_work.family.as_ref().is_none_or(|f| f.complete()),
            "unconsumed declared callback observations"
        );
        report["observed_normal_frames"] = json!(observed_work.next_frame);
        device.refresh(&session)?;
        software::render(device.scene()).save(out.join("final.png"))?;
        Ok(())
    })();
    report["family_window_callbacks"] = json!(observed_work.family.as_ref().map(|f| &f.journal));
    if let Err(e) = result {
        report["boundary"] = json!(format!("{e:#}"));
    }
    write(&out.join("final-session.json"), &session.save())?;
    write(&out.join("final-state.json"), &state(&session))?;
    if managed_restores {
        write(
            &out.join("final-managed-session.json"),
            &session.save_managed()?,
        )?;
    }
    if session.simulation.vm.lot_rng_scope.is_some() {
        // A strict unsupported-service stop still owns its suspended RNG scope.
        // Preserve that stop above; this one-step retry is a separate restore
        // diagnostic and contributes no rows to the gameplay trace.
        let mut cold = cold_restore(cold_index, &session, managed_restores)?;
        ensure!(
            serde_json::to_value(session.save())? == serde_json::to_value(cold.save())?,
            "cold suspended-scope restore differs"
        );
        let outcome = |r: Result<Progress>| match r {
            Ok(p) => format!("{p:?}"),
            Err(e) => format!("{e:#}"),
        };
        let original = outcome(session.step());
        let restored = outcome(cold.step());
        ensure!(
            original == restored,
            "cold suspended-scope progress differs"
        );
        ensure!(
            serde_json::to_value(session.save())? == serde_json::to_value(cold.save())?,
            "cold suspended-scope continuation differs"
        );
        report["cold_suspended_scope_retry"] = json!({
            "equal":true,"outcome":original,"tick":session.simulation.vm.tick,
            "lot_scope_active":session.simulation.vm.lot_rng_scope.is_some()
        });
    }
    write(&out.join("report.json"), &report)?;
    println!("{report}");
    Ok(())
}

#[cfg(test)]
mod process_export_tests {
    use super::*;
    use sims_vm::autonomy_constants::{Configuration, Owner, Process};
    #[test]
    fn missing_and_partial_retained_process_stay_unknown_and_state_neutral() {
        let mut world = sims_vm::World::default();
        let before = serde_json::to_vec(&world).unwrap();
        let missing = retained_process(&world);
        assert!(missing["autonomy_constants"].is_null());
        assert_eq!(before, serde_json::to_vec(&world).unwrap());
        world.autonomy_constants = Some(Owner {
            input_evidence: None,
            observation: None,
            process: Process {
                loaded: Some(7),
                words: [None; 12],
            },
            configuration: Configuration::default(),
        });
        let before = serde_json::to_vec(&world).unwrap();
        let partial = retained_process(&world);
        assert_eq!(partial["autonomy_constants"]["process"]["loaded"], 7);
        assert!(
            partial["autonomy_constants"]["process"]["words"]
                .as_array()
                .unwrap()
                .iter()
                .all(Value::is_null)
        );
        assert_eq!(before, serde_json::to_vec(&world).unwrap());
    }
    #[test]
    fn complete_raw_bits_order_loaded_and_configuration_are_exported_without_projection() {
        let words = [
            0,
            0x80000000,
            0x7fc00001,
            0x7fc00002,
            u32::MAX,
            4,
            50,
            0x3f800001,
            1,
            2,
            3,
            5,
        ];
        let mut world = sims_vm::World::default();
        world.autonomy_constants = Some(Owner {
            input_evidence: None,
            observation: None,
            process: Process {
                loaded: Some(255),
                words: words.map(Some),
            },
            configuration: Configuration::default(),
        });
        let before = serde_json::to_vec(&world).unwrap();
        let row = retained_process(&world);
        assert_eq!(
            row["fields"],
            serde_json::to_value(sims_vm::autonomy_constants::ORDER).unwrap()
        );
        assert_eq!(row["autonomy_constants"]["process"]["words"], json!(words));
        assert_eq!(row["autonomy_constants"]["process"]["loaded"], 255);
        assert_eq!(
            row["autonomy_constants"],
            serde_json::to_value(&world.autonomy_constants).unwrap()
        );
        assert_eq!(before, serde_json::to_vec(&world).unwrap());
    }
}
