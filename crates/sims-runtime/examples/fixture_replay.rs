//! Fixture-backed real Session continuation. The recording command deliberately
//! creates a Rust warm/cold diagnostic, never an original-engine reference.
use anyhow::{Result, bail, ensure};
use serde_json::json;
use sims_content::Index;
use sims_fixtures::{
    Artifact, ArtifactFormat, Boundary, CaseManifest, Compression, Coverage, DependencyMode,
    InputEvent, Origin, Reference, ReferenceKind, RunReport, ScheduleStep, Store,
    trace::{RecordKind, TraceOptions, TraceRecordMeta, TraceWriter},
};
use sims_runtime::fixture_replay::{Driver, Outcome, Poll, Prepared, Request, SessionDriver};
use std::{fs::File, io::Read, path::Path};

const REQUEST: &str = "sims-runtime/replay-request-v1+msgpack";

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("record-managed-diagnostic") if args.len() == 6 => {
            record(&args[1], &args[2], &args[3], &args[4], args[5].parse()?)
        }
        Some("run") if args.len() == 5 => run(&args[1], &args[2], &args[3], &args[4]),
        _ => bail!(
            "usage: fixture_replay record-managed-diagnostic STORE NEW_CASE RESEARCH_CHECKPOINT IMMUTABLE_CONTENT TICKS\n       fixture_replay run STORE CASE IMMUTABLE_CONTENT NEW_RUN"
        ),
    }
}

fn record(store_path: &str, id: &str, checkpoint: &str, content: &str, ticks: u64) -> Result<()> {
    ensure!(
        (2..=4096).contains(&ticks),
        "diagnostic tick range is 2..4096"
    );
    let store = if Path::new(store_path).exists() {
        Store::open(store_path)?
    } else {
        Store::create(store_path)?
    };
    ensure!(
        !store.list_cases()?.iter().any(|old| old == id),
        "case already exists"
    );
    let mut file = File::open(checkpoint)?;
    let before = file.metadata()?;
    ensure!(
        before.is_file() && before.len() <= 128 * 1024 * 1024,
        "research checkpoint size/type"
    );
    let mut bytes = Vec::new();
    file.by_ref()
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        before.len() == bytes.len() as u64
            && before.len() == file.metadata()?.len()
            && before.modified()? == file.metadata()?.modified()?,
        "research checkpoint changed during read"
    );
    let captured: serde_json::Value = serde_json::from_slice(&bytes)?;
    let saved: sims_runtime::Saved =
        serde_json::from_value(captured.get("session").unwrap_or(&captured).clone())?;
    ensure!(
        saved.version == 2 && saved.managed.is_some(),
        "owned managed checkpoint required"
    );
    let initial_tick = saved.simulation.state.vm.tick;
    let routing = format!("{:?}", saved.options.routing);
    let source = store.put_bytes(&bytes, Compression::Gzip)?;
    let checkpoint_blob = store.put_value(&saved)?;
    let boundary = Boundary::Phase {
        name: "managed_session_checkpoint".into(),
        tick: Some(initial_tick),
    };
    let initial = Reference {
        label: "managed-start".into(),
        kind: ReferenceKind::Resumable,
        artifact: Artifact::self_contained(checkpoint_blob.clone(), ArtifactFormat::MessagePack, 1),
        resume_contract: Some(SessionDriver::RESUME_CONTRACT.into()),
        boundary: boundary.clone(),
    };
    let index = Index::scan(Path::new(content))?;
    let mut warm = SessionDriver::restore(&index, &store, &initial)?;
    let mut writer = TraceWriter::new(&store, TraceOptions::default())?;
    writer.append(&warm.observe()?, meta(boundary))?;
    let halfway = ticks / 2;
    let mut completed = 0;
    let mut polls = 0;
    let mut stopped = None;
    while completed < ticks && polls < 100_000 {
        polls += 1;
        let before = warm.tick();
        match warm.poll() {
            Ok(Poll::CompletedTick) => {
                ensure!(
                    Some(warm.tick()) == before.checked_add(1),
                    "warm tick did not complete exactly once"
                );
                completed += 1;
                writer.append(
                    &warm.observe()?,
                    meta(Boundary::CompletedTick { tick: warm.tick() }),
                )?;
                if completed == halfway {
                    // The reference remains uninterrupted. The replay's cold
                    // branch must reproduce this exact warm state at the seam.
                    writer.append(
                        &warm.observe()?,
                        meta(Boundary::Event {
                            name: "managed_cold_restore".into(),
                            tick: Some(warm.tick()),
                        }),
                    )?;
                }
            }
            Ok(Poll::PartialPass) => {
                ensure!(warm.tick() == before, "warm partial pass advanced time")
            }
            Ok(Poll::Boundary(boundary)) => {
                ensure!(warm.tick() == before, "warm service advanced time");
                writer.append(&warm.observe()?, meta(boundary.clone()))?;
                stopped = Some(format!("explicit service required: {boundary:?}"));
                break;
            }
            Err(error) => {
                stopped = Some(format!("{error:#}"));
                break;
            }
        }
    }
    if completed < ticks && stopped.is_none() {
        stopped = Some("warm reference poll limit".into());
    }
    let trace = writer.finish()?;
    let trace_blob = store.put_value(&trace)?;
    let request = Request {
        schema: 1,
        case_id: id.into(),
        initial_reference: initial.label.clone(),
        observation_contract: SessionDriver::OBSERVATION_CONTRACT.into(),
        expected_trace: trace_blob.clone(),
        max_observations: ticks as usize + 4,
        max_polls: 100_000,
        max_completed_ticks: ticks,
    };
    let request_blob = store.put_value(&request)?;
    let final_snapshot = store.put_value(&warm.session.save())?;
    let mut case = CaseManifest::minimal_import(id, "Real managed Rust Session warm/cold continuation diagnostic",
        Artifact::self_contained(source, ArtifactFormat::Json, 1), vec![
            "Reference produced by the Rust runtime, not the original engine; no original parity credit.".into(),
            format!("Requested {ticks} completed ticks; warm recorded {completed}; routing {routing}."),
            format!("Warm stop: {stopped:?}. The input schedule is retained explicitly; no automatic dialog/service acknowledgment."),
            "Snapshot hashes cover complete retained Session bytes, including bit-exact float encoding; not a native-state projection.".into(),
        ]);
    case.provenance.origin = Origin::Imported;
    case.provenance.source_locator = Some(checkpoint.into());
    case.provenance.commands = vec![std::env::args().collect()];
    case.coverage = Coverage::Component { scope: "Rust runtime continuation and cold restore only".into(),
        limitations: vec!["Not original-engine comparison, strict-routing qualification or a full goal scenario.".into(),
            "Runtime-only adapter stops at external UI/client services; it never supplies guessed acknowledgments.".into()] };
    case.references.push(initial);
    case.artifacts.push(Artifact::declared(
        trace_blob.clone(),
        ArtifactFormat::TraceIndex,
        1,
        trace.blob_refs(),
    ));
    let mut dependencies = trace.blob_refs();
    dependencies.extend([trace_blob, checkpoint_blob]);
    case.artifacts.push(Artifact {
        blob: request_blob,
        format: ArtifactFormat::Other(REQUEST.into()),
        schema: 1,
        dependencies,
        dependency_mode: DependencyMode::Declared,
    });
    case.artifacts.push(Artifact::self_contained(
        final_snapshot,
        ArtifactFormat::MessagePack,
        1,
    ));
    case.schedule = vec![
        ScheduleStep::WaitCompletedTicks { count: halfway },
        ScheduleStep::Input {
            input: InputEvent {
                name: "managed_cold_restore".into(),
                payload: serde_json::Value::Null,
            },
        },
        ScheduleStep::WaitCompletedTicks {
            count: ticks - halfway,
        },
    ];
    store.publish_case(&case)?;
    println!(
        "{}",
        json!({"case_id":id,"warm_completed_ticks":completed,"warm_polls":polls,
        "warm_stop":stopped,"original_parity_ticks":0,"scope":"Rust runtime diagnostic"})
    );
    run(store_path, id, content, "cold-replay-first")
}

fn meta(boundary: Boundary) -> TraceRecordMeta {
    TraceRecordMeta {
        kind: RecordKind::Observation,
        boundary,
        source: None,
    }
}

fn run(store_path: &str, id: &str, content: &str, run_id: &str) -> Result<()> {
    let store = Store::open(store_path)?;
    let case = store.load_case(id)?;
    let requests: Vec<_> = case
        .artifacts
        .iter()
        .filter(|a| a.format == ArtifactFormat::Other(REQUEST.into()) && a.schema == 1)
        .collect();
    ensure!(
        requests.len() == 1,
        "case has no unique supported replay request"
    );
    let request: Request = store.get_value(&requests[0].blob)?;
    ensure!(request.case_id == case.id, "request case differs");
    let prepared = Prepared::load::<SessionDriver>(&store, request)?;
    let index = Index::scan(Path::new(content))?;
    let result = prepared.run(|reference| SessionDriver::restore(&index, &store, reference))?;
    let report = store.put_value(&result.report)?;
    let mut artifacts = vec![report.clone()];
    if result.report.outcome != Outcome::Agreement
        && let Some(driver) = &result.driver
    {
        // Preserve the actual failed backend as diagnostic state. It is not
        // promoted to a new qualified resumable reference.
        artifacts.push(store.put_value(&driver.session.save())?);
    }
    store.record_run(&RunReport { schema: 1, id: run_id.into(), case_id: id.into(),
        outcome: format!("{:?}", result.report.outcome),
        notes: vec!["Executed actual sims-runtime Session APIs; internal contract result is not original parity.".into()], artifacts })?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"run_id":run_id,"case_id":id,"report":report,
        "outcome":result.report.outcome,"compared_observations":result.report.compared_observations,
        "completed_ticks":result.report.completed_ticks,"matched_completed_ticks":result.report.matched_completed_ticks,
        "boundary":result.report.boundary,"difference":result.report.difference,"stop":result.report.stop,
        "original_parity_ticks":0}))?
    );
    ensure!(
        result.report.outcome == Outcome::Agreement,
        "fixture runtime comparison did not complete; evidence retained"
    );
    Ok(())
}
