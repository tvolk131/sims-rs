//! Current-program Goth play from original files, with explicit diagnostic inputs.
//! Historical traces supply only reviewed player responses, never VM state.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{
    dialog::client::{Client, Inputs},
    original_house::OriginalHouse,
    software,
};
use sims_runtime::original_session::{Options, Progress, Session};
use std::{
    collections::VecDeque,
    path::{Component, Path},
};

fn step(session: &mut Session, client: &mut Client) -> Result<Progress> {
    if client.pending() && !client.advance(session, 256)?.0 {
        return Ok(Progress::DialogConstruction(0));
    }
    let progress = session.step()?;
    if let Progress::DialogConstruction(thread) = progress {
        let camera = OriginalHouse::camera(session, 1000, 740, 0, 0.55, 1);
        client.begin(session, thread, Inputs::camera(camera, 64)?)?;
    }
    client.collect(session)?;
    Ok(progress)
}

fn saved(session: &Session, client: &Client) -> Result<serde_json::Value> {
    let resident = session
        .house
        .household
        .as_ref()
        .context("household absent")?
        .residents
        .first()
        .context("resident absent")?
        .object as i16;
    Ok(serde_json::json!({
        "session":session.save(), "dialog_client":client.save(session)?,
        "floor":1, "zoom":0.55, "resident":resident, "selected":null,
    }))
}

fn restore(index: &Index, value: &serde_json::Value) -> Result<(Session, Client)> {
    let session = Session::restore(index, serde_json::from_value(value["session"].clone())?)?;
    let client = Client::restore(
        &session,
        serde_json::from_value(value["dialog_client"].clone())?,
    )?;
    ensure!(
        saved(&session, &client)? == *value,
        "full Goth checkpoint changed on restore"
    );
    Ok((session, client))
}

fn read_saved(path: &Path) -> Result<serde_json::Value> {
    let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    // Compare the actual saved types on both sides, not parsed decimal f64
    // approximations with to_value's widened in-memory f32 fields.
    value["session"] = serde_json::to_value(serde_json::from_value::<
        sims_runtime::original_session::Saved,
    >(value["session"].clone())?)?;
    value["dialog_client"] = serde_json::to_value(serde_json::from_value::<
        sims_render::dialog::client::Saved,
    >(value["dialog_client"].clone())?)?;
    Ok(value)
}

// Omit transient request IDs, portrait ownership and rendering fields. Match the
// original semantic notice and its available buttons, not only its title.
fn notice(record: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "title":record["title"],"body":record["body"],"source":record["source"],
        "origin":record["origin"],"operands":record["operands"],
        "caller":record["caller"],"buttons":record["buttons"],
    })
}

struct ReplyPlan {
    ordered: bool,
    remaining: VecDeque<(serde_json::Value, u32)>,
}

fn read_replies(root: &Path, path: &str) -> Result<VecDeque<(serde_json::Value, u32)>> {
    let path = Path::new(path);
    ensure!(
        path.starts_with("cache") && path.components().all(|c| matches!(c, Component::Normal(_)))
    );
    let records: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(root.join(path))?)?;
    ensure!(records.len() <= 16, "too many explicit Goth responses");
    records
        .iter()
        .map(|v| {
            let response = u32::try_from(v["response"].as_u64().context("response absent")?)?;
            ensure!(
                v["record"]["buttons"]
                    .as_array()
                    .context("buttons absent")?
                    .iter()
                    .any(|b| b["result"] == response),
                "recorded response is not a button"
            );
            Ok((notice(&v["record"]), response))
        })
        .collect()
}

#[test]
#[ignore = "original Goth checkpoint; explicit SIMS_GOTH_VIEW_INPUT/OUTPUT"]
fn original_goth_checkpoint_four_views_preserve_saved_state() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input = std::env::var("SIMS_GOTH_VIEW_INPUT")?;
    let output = std::env::var("SIMS_GOTH_VIEW_OUTPUT")?;
    for path in [&input, &output] {
        ensure!(
            Path::new(path).starts_with("cache")
                && Path::new(path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)))
        );
    }
    let output = root.join(output);
    ensure!(!output.exists(), "refusing to overwrite Goth view evidence");
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let before = read_saved(&root.join(&input))?;
    let (session, client) = restore(&index, &before)?;
    let (resumed, resumed_client) = restore(&index, &before)?;
    ensure!(session.house.source == "UserData/Houses/House05.iff");
    let mut renderer = OriginalHouse::default();
    let mut resumed_renderer = OriginalHouse::default();
    for rotation in 0..4 {
        // A diagnostic camera only: no camera command, animation refresh, or
        // simulation step is issued while inspecting the saved pose.
        let camera = OriginalHouse::camera(&session, 1400, 1100, rotation, 0.8, 2);
        let image = software::render(&renderer.scene(&session, camera)?);
        ensure!(image == software::render(&resumed_renderer.scene(&resumed, camera)?));
        image.save(output.join(format!("view-{rotation}.png")))?;
    }
    ensure!(saved(&session, &client)? == before);
    ensure!(saved(&resumed, &resumed_client)? == before);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "input":input,"tick":session.simulation.vm.tick,
            "rotations":4,"floor":2,"identical_pixels":true,"unchanged_state":true,
            "scope":"Read-only diagnostic views of saved native pose; no simulation or camera command."
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "original assets; explicit SIMS_GOTH_COORDINATES, SIMS_GOTH_OUTPUT and SIMS_GOTH_TICKS"]
fn original_goth_current_program_replay() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let profile = std::env::var("SIMS_GOTH_COORDINATES")?;
    ensure!(matches!(
        profile.as_str(),
        "unavailable" | "empty-diagnostic"
    ));
    let ticks = std::env::var("SIMS_GOTH_TICKS")?.parse::<u64>()?;
    ensure!((1..=60_000).contains(&ticks));
    let output = std::env::var("SIMS_GOTH_OUTPUT")?;
    let output = Path::new(&output);
    ensure!(
        output.starts_with("cache")
            && output
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
    );
    let output = root.join(output);
    ensure!(
        !output.exists(),
        "refusing to overwrite retained Goth evidence"
    );
    std::fs::create_dir(&output)?;

    let trace: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/native-goth-9907-group-deletion-prefix.json"),
    )?)?;
    ensure!(trace[0]["source"] == "UserData/Houses/House05.iff");
    let mut replies = trace[0]["dialogs"]
        .as_array()
        .context("historical input trace absent")?
        .iter()
        .map(|d| -> Result<_> {
            Ok((
                notice(&d["record"]),
                u32::try_from(d["response"].as_u64().context("response absent")?)?,
            ))
        })
        .collect::<Result<VecDeque<_>>>()?;
    ensure!(replies.len() == 5);
    drop(trace);
    if let Ok(path) = std::env::var("SIMS_GOTH_FRESH_RESPONSES") {
        // A new program starts from the original files. Only human-facing
        // recorded choices may be reused, never an older program's VM state.
        let expanded = read_replies(&root, &path)?;
        ensure!(
            expanded.iter().take(5).eq(replies.iter()),
            "fresh response plan changes the original five notices"
        );
        replies = expanded;
    }

    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut session = Session::load(
        &index,
        "UserData/Houses/House05.iff",
        Options {
            routing: sims_sim::lot_route::path::Policy::InferredRectangles,
            ..Default::default()
        },
    )?;
    ensure!(
        session
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .unwrap()
            .nearby_houses
            .is_none()
    );
    if profile == "empty-diagnostic" {
        // Explicit host-input experiment before any live tick. Never clear a
        // fault or retrofit this into a stopped normal-loading checkpoint.
        session
            .simulation
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .nearby_houses = Some(
            sims_vm::nearby_houses::order(&[], session.house.number.into())
                .map_err(anyhow::Error::msg)?,
        );
    }
    let client = Client::new(&mut session)?;
    replay(
        &index,
        session,
        client,
        &output,
        ticks,
        &profile,
        ReplyPlan {
            ordered: true,
            remaining: replies,
        },
    )
}

fn replay(
    index: &Index,
    mut session: Session,
    mut client: Client,
    output: &Path,
    ticks: u64,
    profile: &str,
    mut replies: ReplyPlan,
) -> Result<()> {
    let residents = session
        .house
        .household
        .as_ref()
        .context("Goth family absent")?
        .residents
        .clone();
    ensure!(residents.len() == 3 && session.house.floors.len() == 2);
    let before = saved(&session, &client)?;
    let (mut other, mut other_client) = restore(index, &before)?;
    let start = session.simulation.vm.tick;
    std::fs::write(output.join("start.json"), serde_json::to_vec(&before)?)?;
    std::fs::write(
        output.join("request.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source":session.house.source,"coordinate_profile":profile,"start_tick":start,
            "ticks":ticks,"supplied_replies":replies.remaining,
            "replies_must_follow_recorded_order":replies.ordered,"residents":residents,
            "source_hashes":session.house.sources,"original_files_read_only":true,
        }))?,
    )?;
    let mut last_checkpoint = start;
    let mut boundary = None;
    let mut inputs = vec![];
    let mut checkpoints = vec![];
    let mut audio_commands = 0usize;
    let capture_turns = std::env::var("SIMS_GOTH_CAPTURE_TURNS").as_deref() == Ok("1");
    let mut captured_turns = std::collections::BTreeSet::new();
    let mut turn_captures = vec![];
    for _ in 0..200_000 {
        ensure!(session.simulation.vm.sounds == other.simulation.vm.sounds);
        audio_commands += session.simulation.vm.sounds.len();
        session.simulation.vm.sounds.clear();
        other.simulation.vm.sounds.clear();
        let a = step(&mut session, &mut client).map_err(|e| format!("{e:#}"));
        let b = step(&mut other, &mut other_client).map_err(|e| format!("{e:#}"));
        ensure!(a == b, "Goth continuations diverged: {a:?} / {b:?}");
        if capture_turns {
            for (&actor, resident) in &session.simulation.characters {
                let Some(turn) = &resident.native_turn else {
                    continue;
                };
                if resident.controls.ordinary.mode != 4 || !captured_turns.insert(actor) {
                    continue;
                }
                let foreground = resident
                    .character
                    .skin
                    .animator
                    .slot(sims_sim::pose::animator::Slot::Foreground)
                    .context("turn foreground absent")?;
                let clip = &resident
                    .character
                    .skin
                    .animator
                    .layer(foreground)
                    .context("turn layer absent")?
                    .name;
                let checkpoint = saved(&session, &client)?;
                ensure!(checkpoint == saved(&other, &other_client)?);
                let name = format!("native-turn-{actor}.json");
                std::fs::write(output.join(&name), serde_json::to_vec(&checkpoint)?)?;
                let age = session.simulation.vm.world.objects[&actor]
                    .person
                    .as_ref()
                    .context("turn person absent")?[&58];
                turn_captures.push(serde_json::json!({"actor":actor,"age":age,
                    "tick":session.simulation.vm.tick,"clip":clip,"request":turn.request,
                    "path":turn.path,"checkpoint":name}));
                // Validate the complete original household inside the turn;
                // this diagnostic capture does not change either execution.
                restore(index, &checkpoint)?;
                std::fs::write(
                    output.join("native-turns.json"),
                    serde_json::to_vec_pretty(&turn_captures)?,
                )?;
                eprintln!("Goth native turn: actor {actor}, age {age}, {clip}");
            }
        }
        if a == Ok(Progress::Dialog) {
            let id = session
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .context("modal absent")?;
            let record = session
                .simulation
                .vm
                .world
                .dialogs
                .records
                .iter()
                .find(|r| r.id == id)
                .context("modal record absent")?
                .clone();
            let expected = notice(&serde_json::to_value(&record)?);
            let reply_index = replies
                .remaining
                .iter()
                .position(|(notice, _)| *notice == expected)
                .filter(|&i| !replies.ordered || i == 0);
            let response = reply_index.map(|i| replies.remaining[i].1);
            let at_dialog = saved(&session, &client)?;
            ensure!(at_dialog == saved(&other, &other_client)?);
            std::fs::write(
                output.join(format!("modal-{}.json", inputs.len())),
                serde_json::to_vec(&at_dialog)?,
            )?;
            let (resumed, resumed_client) = restore(index, &at_dialog)?;
            ensure!(step(&mut session, &mut client)? == Progress::Dialog);
            ensure!(
                saved(&session, &client)? == at_dialog,
                "unanswered Goth dialog advanced"
            );
            inputs.push(serde_json::json!({"tick":session.simulation.vm.tick,"record":record,"response":response}));
            std::fs::write(
                output.join("inputs.json"),
                serde_json::to_vec_pretty(&inputs)?,
            )?;
            eprintln!(
                "Goth +{}: {:?}, recorded response {response:?}",
                session.simulation.vm.tick - start,
                record.title
            );
            if let Some(response) = response {
                ensure!(record.buttons.iter().any(|b| b.result == response));
                (other, other_client) = (resumed, resumed_client);
                session
                    .simulation
                    .vm
                    .answer_dialog(id, response)
                    .map_err(anyhow::Error::msg)?;
                other
                    .simulation
                    .vm
                    .answer_dialog(id, response)
                    .map_err(anyhow::Error::msg)?;
                replies.remaining.remove(reply_index.unwrap());
                continue;
            }
            boundary = Some(format!("unanswered original modal: {}", record.title));
            break;
        }
        if !matches!(a, Ok(Progress::Tick | Progress::DialogConstruction(_))) {
            boundary = Some(format!("{a:?}"));
            break;
        }
        if session.simulation.vm.tick >= last_checkpoint + 2000 {
            let checkpoint = saved(&session, &client)?;
            ensure!(checkpoint == saved(&other, &other_client)?);
            std::fs::write(
                output.join("checkpoint-latest.json"),
                serde_json::to_vec(&checkpoint)?,
            )?;
            (other, other_client) = restore(index, &checkpoint)?;
            last_checkpoint = session.simulation.vm.tick;
            checkpoints.push(last_checkpoint);
            eprintln!(
                "Goth {profile} +{}: independent execution and full restore match",
                last_checkpoint - start
            );
        }
        if session.simulation.vm.tick >= start + ticks {
            break;
        }
    }
    let after = saved(&session, &client)?;
    ensure!(after == saved(&other, &other_client)?);
    std::fs::write(output.join("final.json"), serde_json::to_vec(&after)?)?;
    let restored = restore(index, &after);
    let restore_error = restored.as_ref().err().map(|e| format!("{e:#}"));
    let mut image_errors = vec![];
    for floor in [1, 2] {
        let result = (|| -> Result<()> {
            let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.55, floor);
            let pixels = software::render(&OriginalHouse::default().scene(&session, camera)?);
            if let Ok((other, _)) = &restored {
                ensure!(
                    pixels == software::render(&OriginalHouse::default().scene(other, camera)?)
                );
            }
            pixels.save(output.join(format!("floor-{floor}.png")))?;
            Ok(())
        })();
        if let Err(e) = result {
            image_errors.push(format!("floor{floor}: {e:#}"));
        }
    }
    ensure!(
        saved(&session, &client)? == after,
        "Goth render changed state"
    );
    let elapsed = session.simulation.vm.tick - start;
    let expected_missing_input = profile == "unavailable"
        && boundary
            .as_deref()
            .is_some_and(|b| b.contains("native nearby-house order unavailable"));
    let report = serde_json::json!({
        "source":session.house.source,"coordinate_profile":profile,"start_tick":start,
        "tick":session.simulation.vm.tick,"elapsed_ticks":elapsed,"requested_ticks":ticks,
        "boundary":boundary,"expected_missing_coordinate_input":expected_missing_input,
        "identical_continuation":true,"identical_restore":restored.is_ok(),"restore_error":restore_error,
        "image_errors":image_errors,"restored_pixels_compared":restored.is_ok() && image_errors.is_empty(),
        "checkpoints":checkpoints,"inputs":inputs,"unconsumed_historical_replies":replies.remaining,
        "silent_audio_commands":audio_commands,
        "native_turn_captures":turn_captures,
        "scope":"Original Goth files/current-program state. Inferred rectangle routing, legacy presentation and explicit recorded inputs. Empty coordinates only in separately named diagnostic; no fault clearing, retagging or demo state."
    });
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    eprintln!("Goth {profile} +{elapsed}: {boundary:?}; restore={restore_error:?}");
    ensure!(
        restore_error.is_none() && image_errors.is_empty(),
        "Goth restore/render failed"
    );
    ensure!(
        expected_missing_input || boundary.is_none() && elapsed == ticks,
        "new Goth boundary: {boundary:?}"
    );
    Ok(())
}

#[test]
#[ignore = "original Goth checkpoint and exact recorded replies; explicit SIMS_GOTH_RESUME_INPUT/RESPONSES/OUTPUT/TICKS"]
fn original_goth_checkpoint_continues_with_recorded_inputs() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input = std::env::var("SIMS_GOTH_RESUME_INPUT")?;
    let responses = std::env::var("SIMS_GOTH_RESUME_RESPONSES")?;
    let output = std::env::var("SIMS_GOTH_OUTPUT")?;
    for path in [&input, &responses, &output] {
        ensure!(
            Path::new(path).starts_with("cache")
                && Path::new(path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)))
        );
    }
    let ticks = std::env::var("SIMS_GOTH_TICKS")?.parse::<u64>()?;
    ensure!((1..=60_000).contains(&ticks));
    let output = root.join(output);
    ensure!(
        !output.exists(),
        "refusing to overwrite Goth continuation evidence"
    );
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let (session, client) = restore(&index, &read_saved(&root.join(&input))?)?;
    ensure!(session.house.source == "UserData/Houses/House05.iff");
    let profile = match session
        .simulation
        .vm
        .world
        .person
        .as_ref()
        .unwrap()
        .nearby_houses
        .as_deref()
    {
        None => "unavailable",
        Some([]) => "empty-diagnostic",
        Some(_) => anyhow::bail!("Goth checkpoint uses an unreviewed coordinate profile"),
    };
    let records: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(root.join(&responses))?)?;
    ensure!(records.len() <= 16, "too many explicit Goth responses");
    let replies = records
        .iter()
        .map(|v| -> Result<_> {
            let response = u32::try_from(v["response"].as_u64().context("response absent")?)?;
            ensure!(
                v["record"]["buttons"]
                    .as_array()
                    .context("buttons absent")?
                    .iter()
                    .any(|b| b["result"] == response),
                "recorded response is not a button"
            );
            Ok((notice(&v["record"]), response))
        })
        .collect::<Result<VecDeque<_>>>()?;
    std::fs::write(
        output.join("provenance.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"checkpoint":input,"responses":responses,"records":records,
            "coordinate_profile":"retained exactly from checkpoint","vm_edits":false}),
        )?,
    )?;
    replay(
        &index,
        session,
        client,
        &output,
        ticks,
        profile,
        ReplyPlan {
            ordered: false,
            remaining: replies,
        },
    )
}

#[test]
#[ignore = "retained v86 Goth fire episode; explicit SIMS_GOTH_FIRE_OUTPUT"]
fn original_goth_fire_episode_observer_preserves_complete_execution() -> Result<()> {
    use std::io::Write;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::env::var("SIMS_GOTH_FIRE_OUTPUT")?;
    ensure!(Path::new(&output).starts_with("cache"));
    ensure!(
        Path::new(&output)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    );
    let output = root.join(output);
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let input = "cache/goth-v86-after-school-bus-87306/start.json";
    let expected_path = "cache/goth-v86-after-school-bus-87306/final.json";
    let (mut session, mut client) = restore(&index, &read_saved(&root.join(input))?)?;
    let expected = read_saved(&root.join(expected_path))?;
    let mut replies = read_replies(&root, "cache/goth-v86-school-bus-87306-replies.json")?;
    let mut events = std::fs::File::create_new(output.join("events.jsonl"))?;
    let mut previous = serde_json::Value::Null;
    let mut first_fire = None;
    let mut first_burning = None;
    let mut event_count = 0;
    let start = session.simulation.vm.tick;
    for _ in 0..8000 {
        // Only borrow the live state. The observer has no VM/service callback,
        // runs no behavior tree, and makes no RNG or renderer calls.
        let vm = &session.simulation.vm;
        let residents = session
            .house
            .household
            .as_ref()
            .context("household absent")?;
        let signature = serde_json::json!({
            "fires":vm.world.fires.registered,
            "residents":residents.residents.iter().map(|r| {
                let id = r.object as i16;
                let object = vm.world.objects.get(&id);
                let thread = vm.object_threads.get(&id).and_then(|id| vm.threads.get(id));
                serde_json::json!({"id":id,"alive":object.is_some(),
                    "flags":object.and_then(|o| o.data.get(&8)),
                    "action":vm.world.actions.as_ref().and_then(|a| a.actors.get(&id)),
                    "death_stack":thread.map(|t| t.stack.iter().any(|f|
                        f.location.source.ends_with("::PersonGlobals.iff") && f.location.bhav == 8305)),
                })
            }).collect::<Vec<_>>()
        });
        if signature != previous {
            let objects = vm
                .world
                .objects
                .iter()
                .filter_map(|(&id, o)| {
                    let d = o.definition.as_ref()?;
                    (d.source.ends_with("::Fire.iff")
                        || d.source.ends_with("::Stoves.iff")
                        || d.source.ends_with("::Ash.iff")
                        || d.source.ends_with("::FireAlarm.iff"))
                    .then(|| {
                        serde_json::json!({"id":id,"definition":d,
                        "position":vm.world.positions.get(&id),"attributes":o.attributes})
                    })
                })
                .collect::<Vec<_>>();
            let details = residents.residents.iter().map(|r| {
                let id = r.object as i16;
                let object = vm.world.objects.get(&id);
                let thread = vm.object_threads.get(&id).and_then(|id| vm.threads.get(id));
                serde_json::json!({"id":id,"name":r.name,"position":vm.world.positions.get(&id),
                    "motives":object.map(|o| &o.motives),"person":object.and_then(|o| o.person.as_ref()),
                    "status":thread.map(|t| &t.status),
                    "stack":thread.map(|t| t.stack.iter().map(|f| serde_json::json!({
                        "location":f.location,"code_owner":f.code_owner,
                        "caller":f.caller,"callee":f.callee,"stack_object":f.stack_object,
                        "args":f.args,"locals":f.locals
                    })).collect::<Vec<_>>())})
            }).collect::<Vec<_>>();
            writeln!(
                events,
                "{}",
                serde_json::json!({"tick":vm.tick,
                "signature":signature,"residents":details,"objects":objects})
            )?;
            previous = signature;
            event_count += 1;
        }
        let fire = !vm.world.fires.registered.is_empty();
        let burning = vm
            .world
            .objects
            .get(&18)
            .and_then(|o| o.data.get(&8))
            .is_some_and(|flags| flags & 512 != 0);
        for (condition, captured, name) in [
            (fire, &mut first_fire, "first-fire.json"),
            (burning, &mut first_burning, "bella-burning.json"),
        ] {
            if condition && captured.is_none() {
                *captured = Some(session.simulation.vm.tick);
                let checkpoint = saved(&session, &client)?;
                restore(&index, &checkpoint)?;
                std::fs::write(output.join(name), serde_json::to_vec(&checkpoint)?)?;
                eprintln!("Goth fire observer {name}: {}", session.simulation.vm.tick);
            }
        }
        session.simulation.vm.sounds.clear();
        match step(&mut session, &mut client)? {
            Progress::Tick | Progress::DialogConstruction(_) => {}
            Progress::Dialog => {
                let dialogs = &session.simulation.vm.world.dialogs;
                let id = dialogs.modal.context("modal absent")?;
                let record = dialogs
                    .records
                    .iter()
                    .find(|r| r.id == id)
                    .context("record absent")?;
                if let Some((expected, response)) = replies.pop_front() {
                    ensure!(notice(&serde_json::to_value(record)?) == expected);
                    session
                        .simulation
                        .vm
                        .answer_dialog(id, response)
                        .map_err(anyhow::Error::msg)?;
                } else {
                    ensure!(record.title == "Rest In Peace");
                    break;
                }
            }
            other => anyhow::bail!("unexpected fire observer boundary: {other:?}"),
        }
    }
    ensure!(replies.is_empty());
    let after = saved(&session, &client)?;
    ensure!(
        after == expected,
        "read-only observation changed complete execution"
    );
    restore(&index, &after)?;
    ensure!(first_fire.is_some() && first_burning.is_some());
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "input":input,"expected":expected_path,"start_tick":start,"tick":session.simulation.vm.tick,
            "events":event_count,"first_fire":first_fire,"bella_burning":first_burning,
            "complete_state_matches_prior_execution":true,"full_restore":true,
            "scope":"Read-only tick-boundary observer of the existing inferred-routing, empty-coordinate diagnostic. Same recorded School Bus response; death modal remains unanswered. Does not establish native full-engine parity."
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "original Goth first-fire checkpoint; explicit SIMS_GOTH_FIRE_ELIGIBILITY_OUTPUT"]
fn original_fire_eligibility_distinguishes_player_from_autonomy() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::env::var("SIMS_GOTH_FIRE_ELIGIBILITY_OUTPUT")?;
    ensure!(Path::new(&output).starts_with("cache"));
    ensure!(
        Path::new(&output)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    );
    let mut output = std::fs::File::create_new(root.join(output))?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let before = read_saved(&root.join("cache/goth-v86-fire-observer/first-fire.json"))?;
    let (session, client) = restore(&index, &before)?;
    let target = session.simulation.vm.world.fires.registered[0];
    let definition = session.simulation.vm.world.objects[&target]
        .definition
        .as_ref()
        .unwrap();
    ensure!(definition.guid == 0x24c95f99);
    let mut cases = vec![];
    for (actor, adult) in [(18, true), (21, true), (16, false)] {
        for autonomous in [false, true] {
            // Execute the mounted original check tree in a disposable VM,
            // using the existing native eligibility argument convention.
            let mut vm = session.simulation.vm.clone();
            let thread = vm
                .spawn(
                    &session.program,
                    sims_vm::Start {
                        owner: &definition.source,
                        bhav: 4114,
                        caller: actor,
                        callee: target,
                        stack_object: target,
                        args: sims_vm::action::eligibility_arguments(autonomous, 0, 0, 4114),
                        check: true,
                    },
                )
                .map_err(anyhow::Error::msg)?;
            let result = vm
                .run_slice(&session.program, thread, 128)
                .map_err(anyhow::Error::msg)?;
            let expected = adult && !autonomous;
            ensure!(result.stop == sims_vm::Stop::Returned(expected));
            cases.push(serde_json::json!({"actor":actor,"adult":adult,
                "autonomous":autonomous,"eligible":expected,"trace":result.trace}));
        }
    }
    ensure!(saved(&session, &client)? == before);
    serde_json::to_writer_pretty(
        &mut output,
        &serde_json::json!({
            "tick":session.simulation.vm.tick,"fire":target,"cases":cases,"unchanged_live_state":true,
            "scope":"Original Fire.iff BHAV4114 and Global363 executed in Rust against the retained first-fire state. Confirms the original eligibility rule; not a new native-engine execution comparison or routing outcome."
        }),
    )?;
    Ok(())
}

#[test]
#[ignore = "original first-fire checkpoint; explicit SIMS_GOTH_EXTINGUISH_OUTPUT"]
fn original_goth_player_extinguishes_fire_with_save_resume() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::env::var("SIMS_GOTH_EXTINGUISH_OUTPUT")?;
    ensure!(Path::new(&output).starts_with("cache"));
    ensure!(
        Path::new(&output)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    );
    let output = root.join(output);
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let input = "cache/goth-v86-fire-observer/first-fire.json";
    let before = read_saved(&root.join(input))?;
    let (mut session, mut client) = restore(&index, &before)?;
    let actor = 18;
    let target = session.simulation.vm.world.fires.registered[0];
    let menu = session
        .simulation
        .vm
        .action_menu(&session.program, target)
        .map_err(anyhow::Error::msg)?
        .iter()
        .position(|e| e.action == 4107)
        .context("original Extinguish action absent")?;
    let choice = session
        .simulation
        .interaction_choice(&session.program, actor, target, menu)
        .map_err(anyhow::Error::msg)?;
    ensure!(choice.available && choice.label == "Extinguish");
    ensure!(saved(&session, &client)? == before);
    let start = session.simulation.vm.tick;
    let action = session
        .simulation
        .enqueue(&session.program, actor, target, menu)
        .map_err(anyhow::Error::msg)?;
    let queued = saved(&session, &client)?;
    std::fs::write(output.join("queued.json"), serde_json::to_vec(&queued)?)?;
    let (mut other, mut other_client) = restore(&index, &queued)?;
    let mut during = None;
    let mut boundary = None;
    let mut completed = false;
    for _ in 0..6000 {
        ensure!(session.simulation.vm.sounds == other.simulation.vm.sounds);
        session.simulation.vm.sounds.clear();
        other.simulation.vm.sounds.clear();
        let a = step(&mut session, &mut client).map_err(|e| format!("{e:#}"));
        let b = step(&mut other, &mut other_client).map_err(|e| format!("{e:#}"));
        ensure!(a == b, "player fire continuations diverged: {a:?} / {b:?}");
        if !matches!(a, Ok(Progress::Tick | Progress::DialogConstruction(_))) {
            boundary = Some(format!("{a:?}"));
            break;
        }
        let vm = &session.simulation.vm;
        let t = vm
            .object_threads
            .get(&actor)
            .and_then(|id| vm.threads.get(id));
        let in_extinguish = t.is_some_and(|t| {
            matches!(t.status, sims_vm::Status::AwaitingAnimation(_))
                && t.stack.iter().any(|f| {
                    f.caller == actor
                        && f.location.bhav == 4119
                        && f.location.source.ends_with("::Fire.iff")
                })
        });
        if during.is_none() && in_extinguish {
            let checkpoint = saved(&session, &client)?;
            ensure!(checkpoint == saved(&other, &other_client)?);
            std::fs::write(
                output.join("during-extinguish.json"),
                serde_json::to_vec(&checkpoint)?,
            )?;
            (other, other_client) = restore(&index, &checkpoint)?;
            during = Some(session.simulation.vm.tick);
            eprintln!("Goth player extinguish: restored original animation at {during:?}");
        }
        let vm = &session.simulation.vm;
        let actions = vm.world.actions.as_ref().unwrap().actors.get(&actor);
        if vm.world.fires.registered.is_empty()
            && actions.is_some_and(|a| {
                a.active.as_ref().is_none_or(|a| u64::from(a.id) != action)
                    && !a.queue.pending.iter().any(|a| u64::from(a.id) == action)
            })
        {
            completed = true;
            break;
        }
        if vm.tick >= start + 5000 {
            boundary = Some("5000-tick cap before Extinguish completion".into());
            break;
        }
    }
    let after = saved(&session, &client)?;
    ensure!(after == saved(&other, &other_client)?);
    std::fs::write(output.join("final.json"), serde_json::to_vec(&after)?)?;
    let (resumed, _) = restore(&index, &after)?;
    let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.55, 1);
    let image = software::render(&OriginalHouse::default().scene(&session, camera)?);
    ensure!(image == software::render(&OriginalHouse::default().scene(&resumed, camera)?));
    image.save(output.join("floor-1.png"))?;
    ensure!(saved(&session, &client)? == after);
    let alive = session
        .simulation
        .vm
        .world
        .objects
        .get(&actor)
        .and_then(|o| o.definition.as_ref())
        .is_some_and(|d| d.source == "UserData/Characters/User00010.iff");
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "input":input,"start_tick":start,"tick":session.simulation.vm.tick,"actor":actor,
            "target":target,"action":action,"menu_index":menu,"during_extinguish":during,
            "completed":completed,"bella_alive":alive,"boundary":boundary,
            "identical_execution":true,"identical_restore":true,"identical_pixels":true,
            "scope":"Separate player-command branch of the existing empty-coordinate, inferred-routing diagnostic. One original Extinguish command; no direct fire removal, motive edits or recorded modal replies."
        }))?,
    )?;
    ensure!(
        completed && alive && during.is_some() && boundary.is_none(),
        "player Extinguish incomplete: {boundary:?}"
    );
    Ok(())
}

#[test]
#[ignore = "original Goth start checkpoint; explicit SIMS_GOTH_PLAYER_INPUT/OUTPUT"]
fn original_goth_player_paints_upstairs_and_resumes_stairs() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input = std::env::var("SIMS_GOTH_PLAYER_INPUT")?;
    let output = std::env::var("SIMS_GOTH_PLAYER_OUTPUT")?;
    for path in [&input, &output] {
        ensure!(
            Path::new(path).starts_with("cache")
                && Path::new(path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)))
        );
    }
    let output = root.join(output);
    ensure!(
        !output.exists(),
        "refusing to overwrite Goth player evidence"
    );
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let before = read_saved(&root.join(&input))?;
    let (mut session, mut client) = restore(&index, &before)?;
    ensure!(session.house.source == "UserData/Houses/House05.iff");
    ensure!(
        session
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .unwrap()
            .nearby_houses
            .is_none(),
        "player check requires normal coordinate policy"
    );
    let actor = session
        .house
        .household
        .as_ref()
        .context("household absent")?
        .residents
        .iter()
        .find(|r| r.name == "Bella")
        .context("original Bella absent")?
        .object as i16;
    ensure!(session.simulation.vm.world.positions[&actor].level == 1);
    let mut choices = vec![];
    for (&id, object) in &session.simulation.vm.world.objects {
        if object
            .definition
            .as_ref()
            .is_some_and(|d| d.source.ends_with("::Easel.iff"))
            && session.simulation.vm.world.positions[&id].level == 2
        {
            for choice in session
                .simulation
                .interaction_menu(&session.program, actor, id)
                .map_err(anyhow::Error::msg)?
            {
                choices.push(serde_json::json!({"target":id,"index":choice.index,
                    "label":choice.label,"available":choice.available}));
            }
        }
    }
    std::fs::write(
        output.join("choices.json"),
        serde_json::to_vec_pretty(&choices)?,
    )?;
    ensure!(
        saved(&session, &client)? == before,
        "opening easel menu changed state"
    );
    let choice = choices
        .iter()
        .find(|c| c["available"] == true && c["label"] == "Paint")
        .context("original upstairs Paint unavailable")?;
    let target = choice["target"].as_i64().unwrap() as i16;
    let menu = choice["index"].as_u64().unwrap() as usize;
    let start = session.simulation.vm.tick;
    let action = session
        .simulation
        .enqueue(&session.program, actor, target, menu)
        .map_err(anyhow::Error::msg)?;
    let queued = saved(&session, &client)?;
    std::fs::write(output.join("queued.json"), serde_json::to_vec(&queued)?)?;
    let (mut other, mut other_client) = restore(&index, &queued)?;
    let mut stair_saved = false;
    let mut climb_saved = false;
    let mut upstairs = false;
    let mut entered_paint = false;
    let mut cancelled = false;
    let mut exited = false;
    let mut boundary = None;
    let mut transitions = vec![];
    let mut last_floor = 1;
    let mut audio_commands = 0usize;
    for _ in 0..20_000 {
        ensure!(session.simulation.vm.sounds == other.simulation.vm.sounds);
        audio_commands += session.simulation.vm.sounds.len();
        session.simulation.vm.sounds.clear();
        other.simulation.vm.sounds.clear();
        let a = step(&mut session, &mut client).map_err(|e| format!("{e:#}"));
        let b = step(&mut other, &mut other_client).map_err(|e| format!("{e:#}"));
        ensure!(
            a == b,
            "Goth directed continuations diverged: {a:?} / {b:?}"
        );
        if !matches!(a, Ok(Progress::Tick | Progress::DialogConstruction(_))) {
            boundary = Some(format!("{a:?}"));
            break;
        }
        let vm = &session.simulation.vm;
        let position = vm.world.positions[&actor];
        if position.level != last_floor {
            transitions.push(serde_json::json!({"tick":vm.tick,"position":position}));
            last_floor = position.level;
        }
        upstairs |= position.level == 2;
        let in_stair = vm.threads.values().any(|t| {
            t.stack
                .iter()
                .any(|f| f.caller == actor && f.location.source.ends_with("::stair.iff"))
        });
        if !stair_saved && in_stair {
            let at_stair = saved(&session, &client)?;
            ensure!(at_stair == saved(&other, &other_client)?);
            std::fs::write(output.join("stair.json"), serde_json::to_vec(&at_stair)?)?;
            (other, other_client) = restore(&index, &at_stair)?;
            stair_saved = true;
            eprintln!(
                "Goth player: saved during original stair behavior at +{}",
                session.simulation.vm.tick - start
            );
        }
        let mid_climb = session.simulation.characters[&actor]
            .character
            .skin
            .animator
            .layers()
            .iter()
            .any(|l| {
                l.layer.name == "a2o-stair-climb"
                    && !l.layer.clock.ended
                    && (0.3..0.7).contains(&l.layer.clock.phase)
            });
        if !climb_saved && in_stair && mid_climb {
            let at_climb = saved(&session, &client)?;
            ensure!(at_climb == saved(&other, &other_client)?);
            std::fs::write(
                output.join("mid-climb.json"),
                serde_json::to_vec(&at_climb)?,
            )?;
            (other, other_client) = restore(&index, &at_climb)?;
            climb_saved = true;
            eprintln!(
                "Goth player: saved during original climb animation at +{}",
                session.simulation.vm.tick - start
            );
        }
        let vm = &session.simulation.vm;
        let in_paint = vm.world.positions[&actor].level == 2
            && vm.world.actions.as_ref().unwrap().actors[&actor]
                .active
                .as_ref()
                .is_some_and(|a| u64::from(a.id) == action)
            && vm.threads.values().any(|t| {
                t.stack.iter().any(|f| {
                    f.caller == actor
                        && f.location.source.ends_with("::Easel.iff")
                        && f.location.bhav == 4104
                })
            }); // Original paint loop, after routing/begin.
        if !entered_paint && in_paint {
            entered_paint = true;
            let at_paint = saved(&session, &client)?;
            ensure!(at_paint == saved(&other, &other_client)?);
            std::fs::write(output.join("painting.json"), serde_json::to_vec(&at_paint)?)?;
            (other, other_client) = restore(&index, &at_paint)?;
            for s in [&mut session, &mut other] {
                s.simulation
                    .cancel_interaction(&s.program, actor, action)
                    .map_err(anyhow::Error::msg)?;
            }
            cancelled = true;
        }
        if cancelled {
            let vm = &session.simulation.vm;
            let a = &vm.world.actions.as_ref().unwrap().actors[&actor];
            if vm.world.objects[&actor].container.is_none()
                && a.active.as_ref().is_none_or(|a| u64::from(a.id) != action)
                && !a.queue.pending.iter().any(|a| u64::from(a.id) == action)
            {
                exited = true;
                break;
            }
        }
        if session.simulation.vm.tick >= start + 850 {
            boundary = Some("850-tick directed check reached before expected exit".into());
            break;
        }
    }
    let after = saved(&session, &client)?;
    ensure!(after == saved(&other, &other_client)?);
    std::fs::write(output.join("final.json"), serde_json::to_vec(&after)?)?;
    let (resumed, _) = restore(&index, &after)?;
    for floor in [1, 2] {
        let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.55, floor);
        let image = software::render(&OriginalHouse::default().scene(&session, camera)?);
        ensure!(image == software::render(&OriginalHouse::default().scene(&resumed, camera)?));
        image.save(output.join(format!("floor-{floor}.png")))?;
    }
    ensure!(
        saved(&session, &client)? == after,
        "directed rendering changed state"
    );
    let report = serde_json::json!({"source":session.house.source,"input":input,
        "start_tick":start,"tick":session.simulation.vm.tick,"actor":actor,"target":target,
        "menu_index":menu,"action":action,"stair_saved":stair_saved,"climb_saved":climb_saved,"upstairs":upstairs,
        "entered_paint":entered_paint,"cancelled":cancelled,"exited":exited,"boundary":boundary,
        "floor_transitions":transitions,"silent_audio_commands":audio_commands,
        "identical_continuation":true,"identical_restore":true,"identical_pixels":true,
        "scope":"Original Goth state; explicit player Paint and cancellation. Normal unavailable nearby coordinates, inferred rectangle routing, legacy presentation; no dialog responses or VM state edits."});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    eprintln!("Goth directed report: {report}");
    ensure!(
        boundary.is_none()
            && stair_saved
            && climb_saved
            && upstairs
            && entered_paint
            && cancelled
            && exited,
        "Goth upstairs player action incomplete: {boundary:?}"
    );
    Ok(())
}
