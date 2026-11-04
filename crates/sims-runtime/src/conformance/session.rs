//! Production Session replay, with immutable input and worker-owned evidence.
use super::{AdapterResult, Scope, Status};
#[cfg(test)]
use crate::fixture_replay::Driver;
use crate::fixture_replay::{Outcome, Prepared, Report, Request, SessionDriver};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_fixtures::{
    Artifact, ArtifactFormat, CaseManifest, Coverage, InputEvent, Origin, ScheduleStep, Store,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const REQUEST: &str = "sims-runtime/replay-request-v1+msgpack";
const REPORT: &str = "sims-runtime/replay-report-v1+msgpack";
const CHECKPOINT: &str = "sims-runtime/replay-backend-diagnostic-v1+msgpack";
const EVIDENCE: &str = "session-evidence";

fn request(store: &Store, case: &CaseManifest) -> Result<Option<Request>> {
    let requests: Vec<_> = case
        .artifacts
        .iter()
        .filter(|a| a.format == ArtifactFormat::Other(REQUEST.into()))
        .collect();
    // Count unsupported-schema siblings too: do not silently select one input
    // from an ambiguous set of declarations.
    if requests.len() != 1 || requests[0].schema != 1 {
        return Ok(None);
    }
    let request: Request = store
        .get_value(&requests[0].blob)
        .context("decode declared Session replay request")?;
    ensure!(
        request.case_id == case.id,
        "Session replay request case identity differs"
    );
    Ok(Some(request))
}
fn unsupported(detail: impl Into<String>) -> AdapterResult {
    AdapterResult::stopped(Status::Unsupported, Scope::RustContinuation, detail)
}
#[derive(Serialize)]
struct UnknownInput<'a> {
    schedule_step: usize,
    input: &'a InputEvent,
}
#[derive(Serialize)]
struct Preflight<'a> {
    schema: u32,
    case_id: &'a str,
    request: Option<&'a Request>,
    first_unknown_input: Option<UnknownInput<'a>>,
    reason: &'a str,
    runtime_executed: bool,
}
fn directory(store: &Store, output: &Path) -> Result<PathBuf> {
    let source = store.root().canonicalize()?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure!(
        !parent
            .canonicalize()
            .context("Session worker output parent must exist")?
            .starts_with(&source),
        "Session worker output is inside source fixture store"
    );
    if output.exists() {
        ensure!(
            !output.canonicalize()?.starts_with(&source),
            "Session worker output aliases source fixture store"
        );
    } else {
        fs::create_dir(output)?;
    }
    output.canonicalize().map_err(Into::into)
}
fn json_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut json = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    json.write_all(&serde_json::to_vec_pretty(value)?)?;
    json.write_all(b"\n")?;
    json.sync_all()?;
    Ok(())
}
fn preflight(
    store: &Store,
    case: &CaseManifest,
    output: &Path,
    request: Option<&Request>,
    unknown: Option<UnknownInput<'_>>,
    reason: String,
) -> Result<AdapterResult> {
    let output = directory(store, output)?;
    json_new(
        &output.join("session-preflight.json"),
        &Preflight {
            schema: 1,
            case_id: &case.id,
            request,
            first_unknown_input: unknown,
            reason: &reason,
            runtime_executed: false,
        },
    )?;
    let mut result = unsupported(reason);
    result
        .artifacts
        .push(PathBuf::from("session-preflight.json"));
    Ok(result)
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Checkpoint {
    Managed {
        saved: crate::Saved,
    },
    /// Exact failed backend even when the managed-save guard cannot capture
    /// this host boundary. This is diagnostic, never a resumable reference.
    Research {
        saved: crate::Saved,
        managed_capture_error: String,
    },
}
fn preserve(
    store: &Store,
    case: &CaseManifest,
    output: &Path,
    report: &Report,
    checkpoint: Option<Checkpoint>,
) -> Result<Vec<PathBuf>> {
    // Validate existing ancestry before creating anything, including when a
    // caller accidentally points its worker directory into the source store.
    let output = directory(store, output)?;
    let destination = output.join(EVIDENCE);
    // export_case reserves a new directory atomically, verifies every blob,
    // and copies rather than hardlinking or modifying the source store.
    store.export_case(&case.id, &destination)?;
    let evidence = Store::open(&destination)?;
    let report_blob = evidence.put_value(report)?;
    let report_artifact = Artifact::declared(
        report_blob,
        ArtifactFormat::Other(REPORT.into()),
        1,
        case.blob_refs(),
    );
    let id = if case.id == "session-adapter-result" {
        "session-adapter-result-derived"
    } else {
        "session-adapter-result"
    };
    let mut derived = CaseManifest::minimal_import(id, "Compiled Session replay result; Rust continuation only",
        report_artifact, vec![report.qualification.clone(),
            "No original-engine parity or household-goal credit; source case and its complete declared closure are retained unchanged.".into()]);
    derived.provenance.origin = Origin::Authored;
    derived.provenance.source_locator = Some(format!("{}#{}", store.root().display(), case.id));
    derived.coverage = Coverage::Component { scope: "Rust runtime continuation under declared replay contract".into(),
        limitations: vec!["Not original-engine gameplay parity. Failed backend diagnostics are not promoted to resumable references.".into()] };
    if let Some(checkpoint) = checkpoint {
        let blob = evidence.put_value(&checkpoint)?;
        derived.artifacts.push(Artifact::declared(
            blob,
            ArtifactFormat::Other(CHECKPOINT.into()),
            1,
            case.blob_refs(),
        ));
    }
    evidence.publish_case(&derived)?;
    evidence.verify_all()?;
    json_new(&output.join("session-replay-report.json"), report)?;
    Ok(vec![
        PathBuf::from("session-replay-report.json"),
        PathBuf::from(EVIDENCE)
            .join("cases")
            .join(id)
            .join("manifest.json"),
    ])
}

/// Run only the case's explicit replay request and schedule. A Source locator,
/// observation or coverage claim never supplies an invented starting state.
pub fn run(
    store: &Store,
    case: &CaseManifest,
    immutable_content: &Path,
    output: &Path,
) -> Result<AdapterResult> {
    let stored = store.load_case(&case.id)?;
    ensure!(
        stored == *case,
        "Session adapter case differs from verified stored manifest"
    );
    let Some(request) = request(store, case)? else {
        return preflight(
            store,
            case,
            output,
            None,
            None,
            "case has no unique supported replay-request-v1+msgpack artifact; runtime not executed"
                .into(),
        );
    };
    for (schedule_step, step) in case.schedule.iter().enumerate() {
        if let ScheduleStep::Input { input } = step
            && (input.name != "managed_cold_restore" || !input.payload.is_null())
        {
            return preflight(
                store,
                case,
                output,
                Some(&request),
                Some(UnknownInput {
                    schedule_step,
                    input,
                }),
                format!(
                    "unsupported Session input at schedule step {schedule_step}: {}; runtime not executed",
                    input.name
                ),
            );
        }
    }
    let prepared = match Prepared::load::<SessionDriver>(store, request.clone()) {
        Ok(value) => value,
        Err(error) => {
            return preflight(
                store,
                case,
                output,
                Some(&request),
                None,
                format!("declared Session replay is unsupported: {error:#}; runtime not executed"),
            );
        }
    };
    let index = sims_content::Index::scan(immutable_content)?;
    ensure!(
        !index
            .resources
            .iter()
            .any(|r| r
                .locator
                .split_once('/')
                .is_some_and(|(directory, _)| directory
                    .to_ascii_lowercase()
                    .strip_prefix("userdata")
                    .is_some_and(|tail| tail.bytes().all(|b| b.is_ascii_digit())))),
        "Session replay content Index contains a mutable starting-save namespace"
    );
    let execution = prepared.run(|reference| SessionDriver::restore(&index, store, reference))?;
    let checkpoint = if execution.report.outcome != Outcome::Agreement {
        execution
            .driver
            .as_ref()
            .map(|driver| match driver.session.save_managed() {
                Ok(saved) => Checkpoint::Managed { saved },
                Err(error) => Checkpoint::Research {
                    saved: driver.session.save(),
                    managed_capture_error: format!("{error:#}"),
                },
            })
    } else {
        None
    };
    let artifacts = preserve(store, case, output, &execution.report, checkpoint)?;
    let report = execution.report;
    Ok(AdapterResult {
        status: match report.outcome {
            Outcome::Agreement => Status::Passed,
            Outcome::Diverged => Status::Diverged,
            Outcome::Incomplete => Status::Incomplete,
        },
        scope: Scope::RustContinuation,
        contract: report.request.observation_contract.clone(),
        observations: report.compared_observations as u64,
        completed_ticks: report.completed_ticks,
        matched_completed_ticks: report.matched_completed_ticks,
        difference: report.difference,
        detail: Some(match report.stop {
            Some(stop) => format!("{stop}; {}", report.qualification),
            None => report.qualification,
        }),
        artifacts,
    })
}

#[cfg(test)]
mod tests;
