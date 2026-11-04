use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, ValueEnum};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_content::Index;
use sims_runtime::original_session::{Options, Progress, Session};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

mod household_performance_support;
use household_performance_support::exact_tick::{Capture, SOURCE};
use household_performance_support::{calendar, latency, state_summary};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Mode {
    Timed,
    Verify,
    Profile,
    Trace,
}

#[derive(Parser)]
struct Args {
    output: PathBuf,
    #[arg(value_enum)]
    mode: Mode,
    ticks: u64,
    start: Option<PathBuf>,
    #[arg(long, default_value = "UserData2/Houses/House09.iff")]
    source: String,
    #[arg(long, default_value_t = 120.)]
    wall_seconds: f64,
    #[arg(long)]
    profile_step_tick: Option<u64>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn answer_startup_dialog(session: &mut Session) -> Result<Value> {
    let record = session
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .find(|record| Some(record.id) == session.simulation.vm.world.dialogs.modal)
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
    let button = record
        .buttons
        .iter()
        .find(|button| notice || (help && button.label.eq_ignore_ascii_case("no")))
        .with_context(|| {
            format!(
                "unexpected modal at {}: {record:?}",
                session.simulation.vm.tick
            )
        })?;
    let response = json!({"id":record.id,"source":record.origin.source,
        "bhav":record.origin.bhav,"label":button.label,"result":button.result});
    session
        .simulation
        .vm
        .answer_dialog(record.id, button.result)
        .map_err(anyhow::Error::msg)?;
    Ok(response)
}

fn observation(
    session: &Session,
    outcome: &str,
    trace: Option<&[sims_vm::Trace]>,
) -> Result<Value> {
    Ok(json!({"tick":session.simulation.vm.tick,"outcome":outcome,
        "state_sha256":digest(&serde_json::to_vec(&session.save())?),
        "trace_sha256":trace.map(serde_json::to_vec).transpose()?.as_deref().map(digest),
        "rng":session.simulation.vm.rng_state,"lot_rng":session.simulation.vm.lot_rng_state,
        "crt_rng":session.simulation.vm.crt_rng_state}))
}

fn advance(
    session: &mut Session,
    trace: Option<&mut Vec<sims_vm::Trace>>,
) -> Result<(String, f64, Option<Value>)> {
    let progress = match trace {
        Some(trace) => session.step_with_trace(trace)?,
        None => session.step()?,
    };
    finish_advance(session, progress)
}

fn finish_advance(
    session: &mut Session,
    progress: Progress,
) -> Result<(String, f64, Option<Value>)> {
    if progress == Progress::Dialog {
        let start = Instant::now();
        let response = answer_startup_dialog(session)?;
        return Ok((
            format!("Dialog({},{})", response["id"], response["result"]),
            start.elapsed().as_secs_f64(),
            Some(response),
        ));
    }
    ensure!(
        matches!(progress, Progress::Tick | Progress::PartialPass),
        "unsupported boundary at {}: {progress:?}",
        session.simulation.vm.tick
    );
    Ok((format!("{progress:?}"), 0., None))
}

fn run() -> Result<()> {
    let args = Args::parse();
    ensure!((2..=10_000).contains(&args.ticks), "ticks must be 2..10000");
    ensure!(
        args.wall_seconds.is_finite() && (0.1..=900.).contains(&args.wall_seconds),
        "wall-seconds must be 0.1..900"
    );
    ensure!(
        args.mode != Mode::Trace || args.ticks <= 64,
        "trace is limited to 64 ticks"
    );
    if let Some(tick) = args.profile_step_tick {
        ensure!(
            tick == 178131
                && args.mode == Mode::Profile
                && args.ticks == 6
                && args.start.is_none()
                && args.source == SOURCE,
            "profile-step-tick requires fresh Mashuga profile 6 at tick 178131"
        );
    }
    fs::create_dir(&args.output)?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
    let setup = Instant::now();
    let index = Index::scan(&root)?;
    let index_seconds = setup.elapsed().as_secs_f64();
    let start = Instant::now();
    let loaded = if let Some(path) = &args.start {
        Session::restore(&index, serde_json::from_slice(&fs::read(path)?)?)
    } else {
        Session::load(
            &index,
            &args.source,
            Options {
                routing: sims_sim::lot_route::path::Policy::InferredRectangles,
                ..Default::default()
            },
        )
    };
    let load_seconds = start.elapsed().as_secs_f64();
    let mut session = match loaded {
        Ok(session) => session,
        Err(error) => {
            fs::write(
                args.output.join("report.json"),
                serde_json::to_vec_pretty(&json!({
                    "success":false,"stage":"load_or_restore","first_error":format!("{error:#}"),
                    "source":args.source,"checkpoint_input":args.start,"index_seconds":index_seconds,
                    "load_seconds":load_seconds,"build_profile":if cfg!(debug_assertions){"debug"}else{"release"},
                    "rendering_included":false,"full_application_startup":false
                }))?,
            )?;
            return Err(error);
        }
    };
    let loaded_state = state_summary(&session);
    if args.start.is_none() {
        *session
            .simulation
            .vm
            .world
            .globals
            .get_mut(&19)
            .context("mode global absent")? = 0;
    }
    let initial = serde_json::to_vec(&session.save())?;
    fs::write(args.output.join("initial.json"), &initial)?;
    let initial_state = state_summary(&session);
    let mut capture = args
        .profile_step_tick
        .map(|_| Capture::new(&args.output, &session))
        .transpose()?;
    let verify = matches!(args.mode, Mode::Verify | Mode::Trace);
    let tracing = args.mode == Mode::Trace;
    if args.mode == Mode::Profile {
        fs::write(args.output.join("ready"), std::process::id().to_string())?;
        println!("{}", json!({"profile_ready_pid":std::process::id()}));
        std::io::stdout().flush()?;
        let wait = Instant::now();
        while !args.output.join("go").exists() {
            ensure!(
                wait.elapsed() < Duration::from_secs(60),
                "profiler did not signal start"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let start_tick = session.simulation.vm.tick;
    let end_tick = start_tick
        .checked_add(args.ticks)
        .context("tick overflow")?;
    let mut outcomes = BTreeMap::<String, u64>::new();
    let mut trace_counts = BTreeMap::<String, u64>::new();
    let mut rows = vec![];
    let mut timings = vec![];
    let mut durations = vec![];
    let mut tick_durations = vec![];
    let mut checkpoint = None;
    let mut simulation_seconds = 0.;
    let mut dialog_response_seconds = 0.;
    let mut first_error = None;
    let wall = Instant::now();
    while session.simulation.vm.tick < end_tick {
        if wall.elapsed().as_secs_f64() >= args.wall_seconds
            || timings.len() >= args.ticks as usize * 100
        {
            first_error = Some("active boundary wall/step limit reached".to_string());
            break;
        }
        let before = session.simulation.vm.tick;
        let mut trace = Vec::new();
        let target = if let Some(capture) = &mut capture {
            match capture.prepare(&session, &timings) {
                Ok(target) => target,
                Err(error) => {
                    capture.fail(format!("target preparation: {error:#}"));
                    break;
                }
            }
        } else {
            false
        };
        let (result, seconds) = if target {
            let (progress, seconds) = capture.as_mut().unwrap().step(&mut session);
            let start = Instant::now();
            let result = progress.and_then(|progress| finish_advance(&mut session, progress));
            (result, seconds + start.elapsed().as_secs_f64())
        } else {
            let start = Instant::now();
            let result = advance(&mut session, tracing.then_some(&mut trace));
            (result, start.elapsed().as_secs_f64())
        };
        let (outcome, response_seconds, response) = match result {
            Ok(result) => result,
            Err(error) => {
                first_error = Some(format!("{error:#}"));
                ("Error".to_string(), 0., None)
            }
        };
        let step_seconds = seconds - response_seconds;
        simulation_seconds += step_seconds;
        dialog_response_seconds += response_seconds;
        durations.push(step_seconds * 1000.);
        if outcome == "Tick" {
            tick_durations.push(step_seconds * 1000.);
        }
        *outcomes.entry(outcome.clone()).or_default() += 1;
        timings.push(
            json!({"tick_before":before,"tick_after":session.simulation.vm.tick,
            "outcome":outcome,"step_ms":step_seconds*1000.,"response_ms":response_seconds*1000.,
            "response":response,"trace_instructions":if tracing {Some(trace.len())}else{None}}),
        );
        if let Some(capture) = &mut capture {
            capture.flush(&timings, &first_error);
            if capture.error.is_some() {
                break;
            }
        }
        for instruction in &trace {
            *trace_counts
                .entry(format!(
                    "{}:{}:{:#06x}",
                    instruction.location.source, instruction.location.bhav, instruction.opcode
                ))
                .or_default() += 1;
        }
        if first_error.is_some() {
            break;
        }
        if verify {
            rows.push(observation(
                &session,
                &outcome,
                tracing.then_some(trace.as_slice()),
            )?);
            if checkpoint.is_none() && session.simulation.vm.tick >= start_tick + args.ticks / 2 {
                checkpoint = Some((rows.len(), serde_json::to_vec(&session.save())?));
            }
        }
    }
    let active_wall_seconds = wall.elapsed().as_secs_f64();
    if let Some(capture) = &mut capture {
        let result = capture.finish(&timings, &first_error);
        if let Err(error) = result {
            if let Some(original) = &first_error {
                eprintln!("secondary capture failure: {error:#}");
                bail!("{original}");
            }
            return Err(error);
        }
        if let Some(original) = &first_error {
            bail!("{original}");
        }
    }
    let final_bytes = serde_json::to_vec(&session.save())?;
    fs::write(args.output.join("final.json"), &final_bytes)?;
    let mut resume_steps = 0;
    let mut restore_seconds = None;
    let mut restore_equal = None;
    if let Some((offset, bytes)) = checkpoint {
        fs::write(args.output.join("checkpoint.json"), &bytes)?;
        let check = (|| -> Result<()> {
            let start = Instant::now();
            let mut resumed = Session::restore(&index, serde_json::from_slice(&bytes)?)?;
            restore_seconds = Some(start.elapsed().as_secs_f64());
            restore_equal = Some(serde_json::to_vec(&resumed.save())? == bytes);
            ensure!(restore_equal == Some(true), "cold restore changed state");
            for row in &rows[offset..] {
                let mut trace = Vec::new();
                let (outcome, _, _) = advance(&mut resumed, tracing.then_some(&mut trace))?;
                ensure!(
                    observation(&resumed, &outcome, tracing.then_some(trace.as_slice()))? == *row,
                    "resumed boundary {} differs",
                    offset + resume_steps
                );
                resume_steps += 1;
            }
            ensure!(
                serde_json::to_vec(&resumed.save())? == final_bytes,
                "resumed final state differs"
            );
            Ok(())
        })();
        if let Err(error) = check {
            first_error.get_or_insert_with(|| format!("continuation: {error:#}"));
        }
    }
    if verify {
        fs::write(
            args.output.join("boundaries.json"),
            serde_json::to_vec(&rows)?,
        )?;
    }
    fs::write(
        args.output.join("timings.json"),
        serde_json::to_vec(&timings)?,
    )?;
    let completed_ticks = session.simulation.vm.tick - start_tick;
    let mut trace_counts: Vec<_> = trace_counts.into_iter().collect();
    trace_counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let report = json!({"schema":2,"scheduler":"original","mode":format!("{:?}",args.mode),
        "build_profile":if cfg!(debug_assertions){"debug"}else{"release"},
        "success":first_error.is_none() && completed_ticks == args.ticks,"first_error":first_error,
        "source":session.house.source,"checkpoint_input":args.start,"loaded_state":loaded_state,
        "initial_state":initial_state,"final_state":state_summary(&session),
        "start_tick":start_tick,"requested_ticks":args.ticks,"completed_ticks":completed_ticks,
        "steps":timings.len(),"outcomes":outcomes,"index_seconds":index_seconds,
        "load_seconds":load_seconds,"simulation_seconds":simulation_seconds,
        "dialog_response_seconds":dialog_response_seconds,"active_wall_seconds":active_wall_seconds,
        "step_latency_ms":latency(&durations),"tick_latency_ms":latency(&tick_durations),
        "ticks_per_second":if !verify && simulation_seconds>0. {Some(completed_ticks as f64/simulation_seconds)}else{None},
        "resume_steps":resume_steps,"restore_seconds":restore_seconds,"restore_equal":restore_equal,
        "initial_sha256":digest(&initial),"final_sha256":digest(&final_bytes),
        "calendar_advanced_seconds":calendar(&session).zip(initial_state["calendar_seconds"].as_i64()).map(|(end,start)|end-start),
        "trace_top_owners_bhav_opcode":trace_counts.into_iter().take(20).collect::<Vec<_>>(),
        "routing":"explicit inferred rectangles","rendering_included":false,
        "original_execution":false,"full_application_startup":false,
        "benchmark_input":if args.start.is_some(){"isolated checkpoint; no mode writes"}else{"fresh Session::load; explicit post-load PublishLiveMode global19=0; not GUI startup"},
        "timing_scope":"Session::step including nested services; excludes response, hashing, IO, restoration; verification/trace are not throughput runs",
        "wall_limit_scope":"checked between Session::step calls; cannot interrupt a single synchronous step"});
    fs::write(
        args.output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string(&report)?);
    if let Some(error) = first_error {
        bail!("{error}");
    }
    Ok(())
}

fn main() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("performance workload panicked")
}
