//! Authored wrapper controls and an opt-in actual stored Rust continuation.
use super::*;
use sims_fixtures::{
    Boundary, Reference, ReferenceKind,
    trace::{RecordKind, TraceOptions, TraceRecordMeta, TraceWriter},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "sims-session-conformance-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn authored(store: &Store) -> Result<CaseManifest> {
    let seed = store.put_value(&serde_json::json!({"authored":true}))?;
    let mut case = CaseManifest::minimal_import(
        "session-protocol",
        "Authored wrapper control, not a Session checkpoint",
        Artifact::self_contained(seed.clone(), ArtifactFormat::MessagePack, 1),
        vec![],
    );
    let boundary = Boundary::Phase {
        name: "authored".into(),
        tick: Some(1),
    };
    let mut writer = TraceWriter::new(store, TraceOptions::default())?;
    writer.append(
        &serde_json::json!({"authored":true}),
        TraceRecordMeta {
            kind: RecordKind::Observation,
            boundary: boundary.clone(),
            source: None,
        },
    )?;
    let trace = writer.finish()?;
    let trace_blob = store.put_value(&trace)?;
    case.artifacts.push(Artifact::declared(
        trace_blob.clone(),
        ArtifactFormat::TraceIndex,
        1,
        trace.blob_refs(),
    ));
    case.references.push(Reference {
        label: "start".into(),
        kind: ReferenceKind::Resumable,
        artifact: Artifact::self_contained(seed, ArtifactFormat::MessagePack, 1),
        resume_contract: Some(SessionDriver::RESUME_CONTRACT.into()),
        boundary,
    });
    let request = Request {
        schema: 1,
        case_id: case.id.clone(),
        initial_reference: "start".into(),
        observation_contract: SessionDriver::OBSERVATION_CONTRACT.into(),
        expected_trace: trace_blob,
        max_observations: 1,
        max_polls: 1,
        max_completed_ticks: 0,
    };
    let blob = store.put_value(&request)?;
    case.artifacts.push(Artifact::self_contained(
        blob,
        ArtifactFormat::Other(REQUEST.into()),
        1,
    ));
    Ok(case)
}
#[test]
fn missing_ambiguous_and_unsupported_requests_do_not_load_assets_or_mutate_source() -> Result<()> {
    for mode in 0..3 {
        let scratch = Scratch::new()?;
        let store = Store::create(scratch.0.join("source"))?;
        let mut case = authored(&store)?;
        match mode {
            0 => {
                case.artifacts.pop();
            }
            1 => case.artifacts.push(case.artifacts.last().unwrap().clone()),
            _ => case.artifacts.last_mut().unwrap().schema = 2,
        }
        store.publish_case(&case)?;
        let before = fs::read(store.root().join("cases/session-protocol/manifest.json"))?;
        let result = run(
            &store,
            &case,
            Path::new("/assets-must-not-be-loaded"),
            &scratch.0.join("worker"),
        )?;
        assert_eq!(result.status, Status::Unsupported);
        assert_eq!(result.scope, Scope::RustContinuation);
        assert_eq!(result.completed_ticks, 0);
        assert_eq!(result.observations, 0);
        assert_eq!(
            fs::read(store.root().join("cases/session-protocol/manifest.json"))?,
            before
        );
        assert!(scratch.0.join("worker/session-preflight.json").is_file());
        store.verify_all()?;
    }
    Ok(())
}
#[test]
fn observations_and_unsupported_inputs_reject_with_exact_preflight_receipts() -> Result<()> {
    for mode in 0..3 {
        let scratch = Scratch::new()?;
        let store = Store::create(scratch.0.join("source"))?;
        let mut case = authored(&store)?;
        match mode {
            0 => {
                case.references[0].kind = ReferenceKind::Observed;
                case.references[0].resume_contract = None;
            }
            1 => case.schedule.push(ScheduleStep::Input {
                input: InputEvent {
                    name: "automatic_ok".into(),
                    payload: serde_json::Value::Null,
                },
            }),
            _ => case.schedule.push(ScheduleStep::Input {
                input: InputEvent {
                    name: "managed_cold_restore".into(),
                    payload: serde_json::json!({"guess":true}),
                },
            }),
        }
        store.publish_case(&case)?;
        let result = run(
            &store,
            &case,
            Path::new("/assets-must-not-be-loaded"),
            &scratch.0.join("worker"),
        )?;
        assert_eq!(result.status, Status::Unsupported);
        assert_eq!(result.completed_ticks, 0);
        let receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(scratch.0.join("worker/session-preflight.json"))?)?;
        assert_eq!(receipt["runtime_executed"], false);
        if mode != 0 {
            assert_eq!(receipt["first_unknown_input"]["schedule_step"], 0);
            let ScheduleStep::Input { input } = &case.schedule[0] else {
                unreachable!()
            };
            assert_eq!(
                receipt["first_unknown_input"]["input"],
                serde_json::to_value(input)?
            );
        }
    }
    Ok(())
}
#[test]
fn output_inside_source_store_is_rejected_before_creation() -> Result<()> {
    let scratch = Scratch::new()?;
    let store = Store::create(scratch.0.join("source"))?;
    let mut case = authored(&store)?;
    case.artifacts.pop();
    store.publish_case(&case)?;
    let output = store.root().join("must-not-be-created");
    assert!(run(&store, &case, Path::new("/absent-assets"), &output).is_err());
    assert!(!output.exists());
    assert_eq!(store.list_cases()?, [case.id]);
    Ok(())
}
#[test]
fn stopped_report_has_a_verified_independent_declared_closure() -> Result<()> {
    let scratch = Scratch::new()?;
    let store = Store::create(scratch.0.join("source"))?;
    let case = authored(&store)?;
    store.publish_case(&case)?;
    let request = request(&store, &case)?.unwrap();
    let report = Report {
        schema: 1,
        request,
        case_source: case.source.blob.clone(),
        initial_source: case.references[0].artifact.blob.clone(),
        outcome: Outcome::Incomplete,
        compared_observations: 0,
        matched_completed_ticks: 0,
        completed_ticks: 0,
        polls: 0,
        schedule_step: None,
        boundary: case.references[0].boundary.clone(),
        difference: None,
        stop: Some("AUTHORED report-retention control, no backend instantiated".into()),
        actual_at_stop: None,
        qualification: "Authored output protocol, not a live backend or parity result".into(),
    };
    let output = scratch.0.join("worker");
    let paths = preserve(&store, &case, &output, &report, None)?;
    assert!(
        paths
            .iter()
            .all(|p| p.is_relative() && output.join(p).is_file())
    );
    let copied = Store::open(output.join(EVIDENCE))?;
    assert_eq!(copied.load_case(&case.id)?, case);
    let derived = copied.load_case("session-adapter-result")?;
    assert_eq!(derived.source.dependencies.len(), case.blob_refs().len());
    let cold: Report = copied.get_value(&derived.source.blob)?;
    assert_eq!(cold.stop, report.stop);
    assert_eq!(cold.outcome, Outcome::Incomplete);
    assert!(derived.references.is_empty());
    copied.verify_all()?;
    store.verify_all()?;
    Ok(())
}
#[test]
#[ignore = "private actual stored Mashuga managed runtime agreement; no native or GUI execution"]
fn actual_declared_mashuga_managed_case_runs_through_adapter_and_cold_input() -> Result<()> {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(|| -> Result<()> {
        let scratch = Scratch::new()?;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize()?;
        let source = Store::open(root.join("fixtures/private/runtime-replay-20261025-first"))?;
        let case = source.load_case("mashuga-managed-runtime-v1")?;
        ensure!(case.schedule.iter().any(|s| matches!(s, ScheduleStep::Input { input } if input.name == "managed_cold_restore")), "actual case lacks cold-restore input");
        let before = fs::read(source.root().join("cases").join(&case.id).join("manifest.json"))?;
        let result = run(&source, &case, &root.join("cache/managed-independent-20261022-third/immutable-assets"), &scratch.0.join("worker"))?;
        assert_eq!(result.status, Status::Passed, "{}", result.detail.as_deref().unwrap_or("no detail"));
        assert_eq!(result.scope, Scope::RustContinuation); assert!(result.difference.is_none());
        assert_eq!(result.completed_ticks, result.matched_completed_ticks);
        assert!(result.completed_ticks > 0); assert!(result.observations > result.completed_ticks);
        let output = Store::open(scratch.0.join("worker").join(EVIDENCE))?;
        let derived = output.load_case("session-adapter-result")?;
        let report: Report = output.get_value(&derived.source.blob)?;
        assert_eq!(report.outcome, Outcome::Agreement);
        assert!(derived.artifacts.is_empty()); assert!(derived.references.is_empty());
        assert_eq!(fs::read(source.root().join("cases").join(&case.id).join("manifest.json"))?, before);
        output.verify_all()?; source.verify_all()?;
        println!("actual conformance Session: status={:?} scope={:?} completed={} matched={} observations={} source_unchanged=true original_parity=0", result.status, result.scope, result.completed_ticks, result.matched_completed_ticks, result.observations);
        // Authored poisoned expectation: preserve the actual failed backend
        // after its first observation, with zero inputs/ticks executed. This is
        // an intentionally wrong Rust expectation, never observed evidence.
        let poisoned_path = scratch.0.join("poisoned-source");
        source.export_case(&case.id, &poisoned_path)?;
        let poisoned_store = Store::open(&poisoned_path)?;
        let mut poisoned = case.clone(); poisoned.id = "authored-poisoned-session-expectation".into();
        poisoned.provenance.origin = Origin::Authored;
        poisoned.notes.push("Negative control: only first expected retained digest is deliberately wrong; no native evidence/credit.".into());
        let mut request = request(&poisoned_store, &case)?.unwrap(); request.case_id = poisoned.id.clone();
        let expected: sims_fixtures::trace::TraceIndex = poisoned_store.get_value(&request.expected_trace)?;
        let mut writer = TraceWriter::new(&poisoned_store, TraceOptions::default())?;
        let mut first = true;
        expected.reconstruct(&poisoned_store, |meta, value| {
            let mut value = value.clone();
            if first { value["retained_session_sha256"] = serde_json::json!("AUTHORED deliberately incorrect digest"); first = false; }
            writer.append(&value, meta.clone())?; Ok(())
        })?;
        let trace = writer.finish()?; let trace_blob = poisoned_store.put_value(&trace)?;
        request.expected_trace = trace_blob.clone();
        let requested = poisoned_store.put_value(&request)?;
        for artifact in &mut poisoned.artifacts {
            if artifact.format == ArtifactFormat::TraceIndex {
                *artifact = Artifact::declared(trace_blob.clone(), ArtifactFormat::TraceIndex, 1, trace.blob_refs());
            } else if artifact.format == ArtifactFormat::Other(REQUEST.into()) {
                let mut dependencies = trace.blob_refs(); dependencies.extend([trace_blob.clone(), poisoned.references[0].artifact.blob.clone()]);
                *artifact = Artifact::declared(requested.clone(), ArtifactFormat::Other(REQUEST.into()), 1, dependencies);
            }
        }
        poisoned_store.publish_case(&poisoned)?;
        let failure_directory = scratch.0.join("failure-worker");
        let failed = run(&poisoned_store, &poisoned, &root.join("cache/managed-independent-20261022-third/immutable-assets"), &failure_directory)?;
        assert_eq!(failed.status, Status::Diverged); assert_eq!(failed.scope, Scope::RustContinuation);
        assert_eq!(failed.completed_ticks, 0); assert_eq!(failed.observations, 0);
        assert_eq!(failed.difference.as_ref().unwrap().path, "/retained_session_sha256");
        let failed_store = Store::open(failure_directory.join(EVIDENCE))?;
        let failed_case = failed_store.load_case("session-adapter-result")?;
        let failed_report: Report = failed_store.get_value(&failed_case.source.blob)?;
        let checkpoint: Checkpoint = failed_store.get_value(&failed_case.artifacts[0].blob)?;
        let saved = match checkpoint { Checkpoint::Managed { saved } => saved, Checkpoint::Research { saved, managed_capture_error } => {
            assert!(!managed_capture_error.is_empty()); saved
        } };
        let actual = failed_report.actual_at_stop.context("failed backend observation missing")?;
        assert_eq!(actual["tick"], saved.simulation.state.vm.tick);
        assert_eq!(actual["rng"], serde_json::json!({"global":saved.simulation.state.vm.rng_state,
            "lot":saved.simulation.state.vm.lot_rng_state,"crt":saved.simulation.state.vm.crt_rng_state}));
        assert!(failed_case.references.is_empty()); failed_store.verify_all()?;
        assert_eq!(fs::read(source.root().join("cases").join(&case.id).join("manifest.json"))?, before);
        println!("actual failed-backend control: diverged at first observation, completed=0, full checkpoint retained, source unchanged, original_parity=0");
        Ok(())
    })?.join().expect("Session conformance actual test panicked")
}
